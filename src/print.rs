//! `--print`, and output to a pipe (ADR-15): the same [`Layout`], written
//! as SGR-styled text, so ada can stand in for glow in pipes and fzf preview panes.

use std::io::{self, Write};

use ratatui::style::{Color, Modifier, Style};

use crate::doc::Doc;
use crate::highlight::{Highlights, SyntaxTheme};
use crate::layout::{self, Layout, LayoutParams};
use crate::theme::Theme;
use crate::ui::Painter;

/// Render a document as ANSI text, one line per row. Code is highlighted first, synchronously,
/// because the output has to be complete (architecture §12).
///
/// With `align`, the output is shifted so that the row rendering that source line is output line
/// `align`. A pager or fzf preview that scrolls by source line, as it would for `bat`, then lands
/// on the right row.
pub fn render(doc: &Doc, theme: &Theme, params: LayoutParams, align: Option<u32>) -> String {
    let mut out = Vec::new();
    write(doc, theme, params, align, &mut out).expect("writing to a Vec");
    String::from_utf8(out).expect("UTF-8 in, UTF-8 out")
}

/// Output is written in pieces of about this size: building all of it first held up to twice
/// its size (6.3 MB in an 8 MB buffer for XL) at the peak (O-16).
const PIECE: usize = 64 * 1024;

/// [`render`] into `w`, a piece at a time.
fn write(
    doc: &Doc,
    theme: &Theme,
    params: LayoutParams,
    align: Option<u32>,
    w: &mut impl Write,
) -> io::Result<()> {
    let layout = layout::layout(doc, &theme.glyphs, params);
    let (skip, pad) = align.map_or((0, 0), |line| alignment(&layout, line));
    let highlights = Highlights::compute_all(&doc.code_blocks(), &SyntaxTheme::new(&theme.syntax));
    let painter = Painter {
        theme,
        layout: &layout,
        highlights: &highlights,
        code_x: 0,
        links: &[],
    };
    let mut out = String::with_capacity(PIECE * 2);
    out.extend(std::iter::repeat_n('\n', pad));
    let mut scratch = String::new();
    for row in layout.rows.iter().skip(skip) {
        let mut line = Line::default();
        painter.row(row, &mut |text, style| {
            line.span(text, Pen::of(style), &mut out, &mut scratch);
        });
        if line.pen != Pen::default() {
            out.push_str("\x1b[0m");
        }
        out.push('\n');
        if out.len() >= PIECE {
            w.write_all(out.as_bytes())?;
            out.clear();
        }
    }
    w.write_all(out.as_bytes())
}

/// One output line as it's written. A line starts with no style and ends with none, so a
/// pager or fzf can show it on its own; within it, each SGR sequence sends only what changes.
#[derive(Default)]
struct Line {
    pen: Pen,
    /// Blanks on the default background not written yet: at the end of the line they needn't be.
    spaces: usize,
}

impl Line {
    fn span(&mut self, text: &str, style: Pen, out: &mut String, scratch: &mut String) {
        let blank = text.bytes().all(|b| b == b' ') && !style.modifier.intersects(VISIBLE_ON_BLANK);
        if blank && style.bg.is_none() {
            self.spaces += text.len();
            return;
        }
        if self.spaces > 0 {
            // with this span's style, if that draws them
            let fits = style.bg.is_none() && !style.modifier.intersects(VISIBLE_ON_BLANK);
            self.blank(if fits { style } else { Pen::default() }, out, scratch);
            out.extend(std::iter::repeat_n(' ', self.spaces));
            self.spaces = 0;
        }
        if blank {
            self.blank(style, out, scratch);
        } else {
            self.set(style, out, scratch);
        }
        out.push_str(text);
    }

    /// For blanks, which show only their background: keep the pen if it draws them so.
    fn blank(&mut self, style: Pen, out: &mut String, scratch: &mut String) {
        if self.pen.bg != style.bg || self.pen.modifier.intersects(VISIBLE_ON_BLANK) {
            self.set(style, out, scratch);
        }
    }

    /// Change the pen to `to`: what differs, or a reset and all of `to`, whichever is shorter.
    /// Only a change that turns something off can be longer than the reset.
    fn set(&mut self, to: Pen, out: &mut String, scratch: &mut String) {
        if self.pen == to {
            return;
        }
        let start = out.len();
        out.push_str("\x1b[");
        if diff(self.pen, to, out) {
            scratch.clear();
            scratch.push('0');
            diff(Pen::default(), to, scratch);
            if scratch.len() < out.len() - start - 3 {
                out.truncate(start + 2);
                out.push_str(scratch);
            }
        }
        // the first parameter has no separator before it
        if out.as_bytes()[start + 2] == b';' {
            out.remove(start + 2);
        }
        out.push('m');
        self.pen = to;
    }
}

/// What shows of a style in print: six modifiers, the colors, and an RGB underline color.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Pen {
    modifier: Modifier,
    fg: Option<Color>,
    bg: Option<Color>,
    underline: Option<(u8, u8, u8)>,
}

const PRINTED: Modifier = Modifier::BOLD
    .union(Modifier::DIM)
    .union(Modifier::ITALIC)
    .union(Modifier::UNDERLINED)
    .union(Modifier::REVERSED)
    .union(Modifier::CROSSED_OUT);

/// Modifiers that show on a space.
const VISIBLE_ON_BLANK: Modifier = Modifier::UNDERLINED
    .union(Modifier::REVERSED)
    .union(Modifier::CROSSED_OUT);

impl Pen {
    fn of(style: Style) -> Pen {
        let modifier = style.add_modifier & PRINTED;
        let set = |c: Option<Color>| c.filter(|&c| c != Color::Reset);
        Pen {
            modifier,
            fg: set(style.fg),
            bg: set(style.bg),
            underline: match style.underline_color {
                Some(Color::Rgb(r, g, b)) if modifier.contains(Modifier::UNDERLINED) => {
                    Some((r, g, b))
                }
                _ => None,
            },
        }
    }
}

/// SGR parameters from `from` to `to`, each after a `;`. Returns whether any turns something off.
fn diff(from: Pen, to: Pen, out: &mut String) -> bool {
    let removed = from.modifier - to.modifier;
    let mut added = to.modifier - from.modifier;
    if removed.intersects(Modifier::BOLD | Modifier::DIM) {
        // 22 ends both bold and dim
        out.push_str(";22");
        added |= to.modifier & (Modifier::BOLD | Modifier::DIM);
    }
    for (flag, code) in [
        (Modifier::ITALIC, ";23"),
        (Modifier::UNDERLINED, ";24"),
        (Modifier::REVERSED, ";27"),
        (Modifier::CROSSED_OUT, ";29"),
    ] {
        if removed.contains(flag) {
            out.push_str(code);
        }
    }
    for (flag, code) in [
        (Modifier::BOLD, ";1"),
        (Modifier::DIM, ";2"),
        (Modifier::ITALIC, ";3"),
        (Modifier::UNDERLINED, ";4"),
        (Modifier::REVERSED, ";7"),
        (Modifier::CROSSED_OUT, ";9"),
    ] {
        if added.contains(flag) {
            out.push_str(code);
        }
    }
    let mut off = !removed.is_empty();
    if from.fg != to.fg {
        off |= to.fg.is_none();
        color(to.fg.unwrap_or(Color::Reset), 30, out);
    }
    if from.bg != to.bg {
        off |= to.bg.is_none();
        color(to.bg.unwrap_or(Color::Reset), 40, out);
    }
    if from.underline != to.underline {
        match to.underline {
            Some((r, g, b)) => {
                out.push_str(";58;2");
                rgb(r, g, b, out);
            }
            None => {
                off = true;
                out.push_str(";59");
            }
        }
    }
    off
}

/// Rows to drop from the top, and blank lines to add, so that the first row rendering `line`
/// becomes output line `line`.
fn alignment(layout: &Layout, line: u32) -> (usize, usize) {
    let Some(row) = layout.row_for_line(line) else {
        return (0, 0);
    };
    let (row, target) = (row as usize, line.saturating_sub(1) as usize);
    (row.saturating_sub(target), target.saturating_sub(row))
}

/// Print a document to stdout. A closed pipe (fzf moving on) is not an error.
pub fn run(doc: &Doc, theme: &Theme, params: LayoutParams, align: Option<u32>) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    match write(doc, theme, params, align, &mut stdout).and_then(|()| stdout.flush()) {
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result,
    }
}

fn color(c: Color, base: u8, out: &mut String) {
    out.push(';');
    let n = match c {
        Color::Reset => base + 9,
        Color::Rgb(r, g, b) => {
            number(base + 8, out);
            out.push_str(";2");
            return rgb(r, g, b, out);
        }
        Color::Indexed(n) => {
            number(base + 8, out);
            out.push_str(";5;");
            return number(n, out);
        }
        Color::Black => base,
        Color::Red => base + 1,
        Color::Green => base + 2,
        Color::Yellow => base + 3,
        Color::Blue => base + 4,
        Color::Magenta => base + 5,
        Color::Cyan => base + 6,
        Color::Gray => base + 7,
        Color::DarkGray => base + 60,
        Color::LightRed => base + 61,
        Color::LightGreen => base + 62,
        Color::LightYellow => base + 63,
        Color::LightBlue => base + 64,
        Color::LightMagenta => base + 65,
        Color::LightCyan => base + 66,
        Color::White => base + 67,
    };
    number(n, out);
}

/// `;r;g;b`.
fn rgb(r: u8, g: u8, b: u8, out: &mut String) {
    for n in [r, g, b] {
        out.push(';');
        number(n, out);
    }
}

fn number(n: u8, out: &mut String) {
    if n >= 100 {
        out.push(char::from(b'0' + n / 100));
    }
    if n >= 10 {
        out.push(char::from(b'0' + n / 10 % 10));
    }
    out.push(char::from(b'0' + n % 10));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The SGR sequence that takes a line from `from` to `to`.
    fn sgr(from: Style, to: Style) -> String {
        let mut line = Line {
            pen: Pen::of(from),
            spaces: 0,
        };
        let mut out = String::new();
        line.set(Pen::of(to), &mut out, &mut String::new());
        out
    }

    #[test]
    fn sgr_sequences() {
        let rgb = Color::Rgb(1, 2, 3);
        let to = Style::new()
            .fg(rgb)
            .bg(Color::LightRed)
            .add_modifier(Modifier::BOLD | Modifier::ITALIC);
        assert_eq!(sgr(Style::new(), to), "\x1b[1;3;38;2;1;2;3;101m");
        // only what changes
        assert_eq!(sgr(to, to.fg(Color::Red)), "\x1b[31m");
        assert_eq!(sgr(to, to.remove_modifier(Modifier::BOLD)), "\x1b[22m");
        assert_eq!(sgr(to, to.bg(Color::Reset)), "\x1b[49m");
        assert_eq!(
            sgr(to.dim(), to.not_bold().not_italic().dim()),
            "\x1b[22;23;2m"
        );
        // or a reset, when that's shorter
        assert_eq!(sgr(to, Style::new()), "\x1b[0m");
        assert_eq!(sgr(to, Style::new().fg(rgb)), "\x1b[22;23;49m");
        assert_eq!(
            sgr(
                Style::new().underlined().underline_color(rgb),
                Style::new().underlined()
            ),
            "\x1b[59m"
        );
    }

    /// A line's spans, printed.
    fn line(spans: &[(&str, Style)]) -> String {
        let mut line = Line::default();
        let mut out = String::new();
        for &(text, style) in spans {
            line.span(text, Pen::of(style), &mut out, &mut String::new());
        }
        out
    }

    #[test]
    fn blanks_show_only_their_background() {
        let (a, b) = (Color::Rgb(1, 1, 1), Color::Rgb(2, 2, 2));
        let code = Style::new().bg(b);
        // a margin and a space between words take no style of their own
        assert_eq!(
            line(&[
                ("  ", Style::new().fg(a)),
                ("x", Style::new().fg(b)),
                (" ", Style::new().fg(a)),
                ("y", Style::new().fg(b)),
            ]),
            "  \x1b[38;2;2;2;2mx y"
        );
        // a blank on a background takes its own style, and the next span need not change it
        assert_eq!(
            line(&[("x", Style::new()), (" ", code.fg(a)), ("y", code.fg(a))]),
            "x\x1b[38;2;1;1;1;48;2;2;2;2m y"
        );
        // an underlined space shows
        assert_eq!(
            line(&[
                ("x", Style::new().underlined()),
                (" ", Style::new().fg(a)),
                ("y", Style::new().fg(a)),
            ]),
            "\x1b[4mx\x1b[0;38;2;1;1;1m y"
        );
        // blanks at the end of a line aren't written, but within it, on the default background
        assert_eq!(
            line(&[
                ("x", code),
                (" ", Style::new()),
                ("y", Style::new()),
                ("  ", Style::new())
            ]),
            "\x1b[48;2;2;2;2mx\x1b[0m y"
        );
    }

    #[test]
    fn renders_one_line_per_row() {
        let doc = crate::doc::parse("# Title\n\nsome *text*\n");
        let params = LayoutParams {
            width: 40,
            max_width: None,
            table_mode: Default::default(),
            code_wrap: true,
            glyphs_gen: 0,
            images: None,
        };
        let out = render(&doc, &Theme::builtin(), params, None);
        assert_eq!(out.lines().count(), 3);
        assert!(out.contains("Title"));
    }

    #[test]
    fn aligned_output_puts_the_line_on_its_own_row() {
        let params = LayoutParams {
            width: 30,
            max_width: None,
            table_mode: Default::default(),
            code_wrap: true,
            glyphs_gen: 0,
            images: None,
        };
        let plain = |text: &str, line| {
            let out = render(
                &crate::doc::parse(text),
                &Theme::builtin(),
                params,
                Some(line),
            );
            let mut lines: Vec<String> = Vec::new();
            for l in out.lines() {
                let mut s = String::new();
                let mut esc = false;
                for c in l.chars() {
                    match (esc, c) {
                        (_, '\x1b') => esc = true,
                        (true, 'm') => esc = false,
                        (true, _) => {}
                        (false, c) => s.push(c),
                    }
                }
                lines.push(s.trim().to_owned());
            }
            lines
        };
        // more rows than lines above the target: rows are dropped
        let wrapped = "a paragraph long enough to wrap over several rows at thirty\n\ntarget\n";
        assert_eq!(plain(wrapped, 3)[2], "target");
        // fewer rows than lines above it: blank lines are added
        let fence = "```\nx\n```\n\n\n\n\ntarget\n";
        let out = plain(fence, 8);
        assert_eq!(out[7], "target");
        // a line past the end leaves the output alone
        assert_eq!(plain("one\n", 50), ["one"]);
    }

    #[test]
    fn code_is_highlighted() {
        let doc = crate::doc::parse("```rust\nlet s = \"hi\";\n```\n");
        let params = LayoutParams {
            width: 40,
            max_width: None,
            table_mode: Default::default(),
            code_wrap: true,
            glyphs_gen: 0,
            images: None,
        };
        let theme = Theme::builtin();
        let out = render(&doc, &theme, params, None);
        let string = theme.syntax.iter().find(|(s, _)| s == "string").unwrap().1;
        let Some(Color::Rgb(r, g, b)) = string.fg else {
            panic!("strings have an rgb color");
        };
        assert!(
            out.contains("48;2;50;48;47m")
                && out.contains(&format!("\x1b[38;2;{r};{g};{b}m\"hi\"\x1b")),
            "{out:?}"
        );
    }
}
