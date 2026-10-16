//! `--print`, and output to a pipe (ADR-15): the same [`Layout`], written
//! as SGR-styled text, so ada can stand in for glow in pipes and fzf preview panes.

use std::fmt::Write as _;
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
    let mut out = String::with_capacity(layout.text.len() * 2);
    out.extend(std::iter::repeat_n('\n', pad));
    for row in layout.rows.iter().skip(skip) {
        let mut current = None;
        painter.row(row, &mut |text, style| {
            if current != Some(style) {
                sgr(style, &mut out);
                current = Some(style);
            }
            out.push_str(text);
        });
        if current.is_some() {
            out.push_str("\x1b[0m");
        }
        out.push('\n');
    }
    out
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
    let out = render(doc, theme, params, align);
    let mut stdout = io::stdout().lock();
    match stdout
        .write_all(out.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result,
    }
}

fn sgr(style: Style, out: &mut String) {
    out.push_str("\x1b[0");
    let m = style.add_modifier;
    for (flag, code) in [
        (Modifier::BOLD, "1"),
        (Modifier::DIM, "2"),
        (Modifier::ITALIC, "3"),
        (Modifier::UNDERLINED, "4"),
        (Modifier::REVERSED, "7"),
        (Modifier::CROSSED_OUT, "9"),
    ] {
        if m.contains(flag) {
            out.push(';');
            out.push_str(code);
        }
    }
    if let Some(c) = style.fg {
        color(c, 30, out);
    }
    if let Some(c) = style.bg {
        color(c, 40, out);
    }
    if m.contains(Modifier::UNDERLINED)
        && let Some(Color::Rgb(r, g, b)) = style.underline_color
    {
        let _ = write!(out, ";58;2;{r};{g};{b}");
    }
    out.push('m');
}

fn color(c: Color, base: u8, out: &mut String) {
    let named = |n: u8| if n < 8 { base + n } else { base + 60 + n - 8 };
    let _ = match c {
        Color::Reset => Ok(()),
        Color::Rgb(r, g, b) => write!(out, ";{};2;{r};{g};{b}", base + 8),
        Color::Indexed(n) => write!(out, ";{};5;{n}", base + 8),
        Color::Black => write!(out, ";{}", named(0)),
        Color::Red => write!(out, ";{}", named(1)),
        Color::Green => write!(out, ";{}", named(2)),
        Color::Yellow => write!(out, ";{}", named(3)),
        Color::Blue => write!(out, ";{}", named(4)),
        Color::Magenta => write!(out, ";{}", named(5)),
        Color::Cyan => write!(out, ";{}", named(6)),
        Color::Gray => write!(out, ";{}", named(7)),
        Color::DarkGray => write!(out, ";{}", named(8)),
        Color::LightRed => write!(out, ";{}", named(9)),
        Color::LightGreen => write!(out, ";{}", named(10)),
        Color::LightYellow => write!(out, ";{}", named(11)),
        Color::LightBlue => write!(out, ";{}", named(12)),
        Color::LightMagenta => write!(out, ";{}", named(13)),
        Color::LightCyan => write!(out, ";{}", named(14)),
        Color::White => write!(out, ";{}", named(15)),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sgr_sequences() {
        let mut out = String::new();
        sgr(
            Style::new()
                .fg(Color::Rgb(1, 2, 3))
                .bg(Color::LightRed)
                .add_modifier(Modifier::BOLD | Modifier::ITALIC),
            &mut out,
        );
        assert_eq!(out, "\x1b[0;1;3;38;2;1;2;3;101m");
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
            panic!("strings have an rgb colour");
        };
        assert!(
            out.contains(&format!("38;2;{r};{g};{b};48;2;50;48;47m\"hi\"\x1b")),
            "{out:?}"
        );
    }
}
