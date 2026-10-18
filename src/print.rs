//! `--print`, and output to a pipe (ADR-15): the same [`Layout`], written
//! as SGR-styled text, so ada can stand in for glow in pipes and fzf preview panes.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use image::DynamicImage;
use ratatui::layout::Size;
use ratatui::style::{Color, Modifier, Style};

use crate::doc::{Doc, LinkId};
use crate::highlight::{Highlights, SyntaxTheme};
use crate::images::{self, Probe, Proto};
use crate::kitty;
use crate::layout::{self, ImageSlot, Layout, LayoutParams, Measure, RowKind};
use crate::links::{self, BuiltinResolver, Resolved, Resolver, Target};
use crate::pixels;
use crate::theme::{StyleKey, Theme};
use crate::ui::Painter;

/// How to print, besides the layout.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options<'a> {
    /// Shift the output so that the row rendering this source line is output line `align`. A
    /// pager or fzf preview that scrolls by source line, as it would for `bat`, then lands on
    /// the right row.
    pub align: Option<u32>,
    /// The most rows to write, after the ones `align` drops.
    pub rows: Option<usize>,
    /// Draw block images, which are otherwise `🖼 alt (path)` lines.
    pub images: Option<&'a Pictures>,
}

/// The images a printed document shows, and how to draw them (architecture §12).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pictures {
    /// Kitty or half-blocks.
    pub proto: Proto,
    /// The font's cell size in pixels.
    pub cell: Size,
    /// The image each link leads to, with its upright size in pixels, by [`LinkId`].
    pub files: Vec<Option<(PathBuf, (u32, u32))>>,
    /// Give kitty images the ids the previous run gave them, which replaces those ([`kitty::image_id`]).
    pub reuse_ids: bool,
}

impl Pictures {
    /// Resolve `doc`'s links as the file `source` holds them, and read the headers of the
    /// images they lead to.
    pub fn find(doc: &Doc, source: &Path, proto: Proto, cell: Size, reuse_ids: bool) -> Pictures {
        let mut probed: Vec<(PathBuf, Option<(u32, u32)>)> = Vec::new();
        let files = doc
            .links
            .iter()
            .map(|link| {
                let Target::Local { target, .. } = Target::of(link) else {
                    return None;
                };
                let Resolved::Found(path) = BuiltinResolver.resolve(source, &target) else {
                    return None;
                };
                if !links::is_image(&path) {
                    return None;
                }
                let px = match probed.iter().find(|(p, _)| *p == path) {
                    Some(&(_, px)) => px,
                    None => {
                        let px = match images::probe(&path) {
                            Probe::Image { w, h, .. } => Some((w, h)),
                            Probe::Failed(_) => None,
                        };
                        probed.push((path.clone(), px));
                        px
                    }
                };
                px.map(|px| (path, px))
            })
            .collect();
        Pictures {
            proto,
            cell,
            files,
            reuse_ids,
        }
    }
}

/// Render a document as ANSI text, one line per row. Code is highlighted first, synchronously,
/// because the output has to be complete (architecture §12).
pub fn render(doc: &Doc, theme: &Theme, params: LayoutParams, opts: Options) -> String {
    let mut out = Vec::new();
    write(doc, theme, params, opts, &mut out).expect("writing to a Vec");
    String::from_utf8_lossy(&out).into_owned()
}

/// Output is written in pieces of about this size: building all of it first held up to twice
/// its size (6.3 MB in an 8 MB buffer for XL) at the peak (O-16).
const PIECE: usize = 64 * 1024;

/// [`render`] into `w`, a piece at a time.
fn write(
    doc: &Doc,
    theme: &Theme,
    params: LayoutParams,
    opts: Options,
    w: &mut impl Write,
) -> io::Result<()> {
    let mut measure = Measure::new(doc, &theme.glyphs);
    for (id, file) in opts.images.map_or(&[][..], |p| &p.files).iter().enumerate() {
        measure.set_image(id as LinkId, file.as_ref().map(|f| f.1));
    }
    let layout = layout::layout_measured(doc, &measure, &theme.glyphs, params);
    drop(measure);
    let (skip, pad) = opts.align.map_or((0, 0), |line| alignment(&layout, line));
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
    let mut image: Option<(u32, Drawn)> = None;
    let rows = layout.rows.iter().enumerate().skip(skip);
    for (r, row) in rows.take(opts.rows.unwrap_or(usize::MAX)) {
        let mut line = Line::default();
        let mut emit = |text: &str, style| {
            line.span(text, Pen::of(style), &mut out, &mut scratch);
        };
        painter.number(r as u32, &mut emit);
        painter.row(row, &mut emit);
        if let (RowKind::Image { slot }, Some(pictures)) = (row.kind, opts.images)
            && let Some(at) = layout.images.get(slot as usize)
        {
            if image.as_ref().is_none_or(|(s, _)| *s != slot) {
                // a kitty image is sent before the first of its rows that shows
                w.write_all(out.as_bytes())?;
                out.clear();
                image = Some((slot, Drawn::new(pictures, at, slot, w)?));
            }
            if let Some((_, drawn)) = &image {
                let frame = Pen::of(theme.style(StyleKey::ImageFrame));
                drawn.row(r as u32 - at.row, frame, &mut line, &mut out, &mut scratch);
            }
        }
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

/// A block image, ready for its rows.
enum Drawn {
    /// Sent to the terminal as image `id`: each row is placeholders.
    Kitty { id: u32, cols: u16 },
    /// Two pixels a cell, the top one in the foreground of `▀`.
    Halfblocks {
        image: DynamicImage,
        cols: u16,
        rows: u16,
    },
    /// Why not, shown on its first row.
    Failed(String),
}

impl Drawn {
    /// Decode the image `at` shows, and for kitty send it.
    fn new(
        pictures: &Pictures,
        at: &ImageSlot,
        slot: u32,
        w: &mut impl Write,
    ) -> io::Result<Drawn> {
        let Some(Some((path, px))) = pictures.files.get(at.link as usize) else {
            return Ok(Drawn::Failed(String::new()));
        };
        let (cols, rows) = (at.size.width, at.size.height);
        Ok(match pictures.proto {
            Proto::Kitty => {
                let (cols, rows) = (cols.min(kitty::MAX_CELLS), rows.min(kitty::MAX_CELLS));
                let cell = pictures.cell;
                let max = (
                    u32::from(cols) * u32::from(cell.width),
                    u32::from(rows) * u32::from(cell.height),
                );
                let id = kitty::image_id(slot, pictures.reuse_ids);
                // a PNG small enough already goes as it is, undecoded: kitty decodes it
                let fits = px.0 <= max.0 && px.1 <= max.1;
                if fits && pixels::is_plain_png(path) {
                    kitty::transmit(w, id, &kitty::Payload::Png(path), cols, rows)?;
                    Drawn::Kitty { id, cols }
                } else {
                    match pixels::decode(path, max) {
                        Ok(image) => {
                            let payload = kitty::Payload::Pixels(&image);
                            kitty::transmit(w, id, &payload, cols, rows)?;
                            Drawn::Kitty { id, cols }
                        }
                        Err(why) => Drawn::Failed(why),
                    }
                }
            }
            _ => match pixels::decode_to(path, |_| (u32::from(cols), 2 * u32::from(rows))) {
                Ok(image) => Drawn::Halfblocks { image, cols, rows },
                Err(why) => Drawn::Failed(why),
            },
        })
    }

    /// Row `y` of the image.
    fn row(&self, y: u32, frame: Pen, line: &mut Line, out: &mut String, scratch: &mut String) {
        match self {
            Drawn::Kitty { id, cols } => {
                let (r, g, b) = kitty::id_color(*id);
                let mut cells = String::new();
                kitty::placeholders(y as u16, *cols, &mut cells);
                let pen = Pen {
                    fg: Some(Color::Rgb(r, g, b)),
                    ..Pen::default()
                };
                line.span(&cells, pen, out, scratch);
            }
            Drawn::Halfblocks { image, cols, rows } => {
                // the image is no larger than its own pixels: a small one repeats them
                let (w, h) = (image.width(), image.height());
                let at = |x: u32, y: u32| {
                    let (x, y) = (x * w / u32::from(*cols), y * h / (2 * u32::from(*rows)));
                    let p = &image.as_bytes()[(y * w + x) as usize * channels(image)..];
                    let alpha = if image.color().has_alpha() { p[3] } else { 255 };
                    (alpha >= 128).then_some(Color::Rgb(p[0], p[1], p[2]))
                };
                for x in 0..u32::from(*cols) {
                    let (top, bottom) = (at(x, 2 * y), at(x, 2 * y + 1));
                    let (text, fg, bg) = match (top, bottom) {
                        (None, None) => (" ", None, None),
                        (Some(t), b) => ("▀", Some(t), b),
                        (None, b) => ("▄", b, None),
                    };
                    line.span(
                        text,
                        Pen {
                            fg,
                            bg,
                            ..Pen::default()
                        },
                        out,
                        scratch,
                    );
                }
            }
            Drawn::Failed(why) if y == 0 && !why.is_empty() => line.span(why, frame, out, scratch),
            Drawn::Failed(_) => {}
        }
    }
}

fn channels(image: &DynamicImage) -> usize {
    usize::from(image.color().channel_count())
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
pub fn run(doc: &Doc, theme: &Theme, params: LayoutParams, opts: Options) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    match write(doc, theme, params, opts, &mut stdout).and_then(|()| stdout.flush()) {
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
            line_numbers: Default::default(),
            glyphs_gen: 0,
            images: None,
        };
        let out = render(&doc, &Theme::builtin(), params, Options::default());
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
            line_numbers: Default::default(),
            glyphs_gen: 0,
            images: None,
        };
        let plain = |text: &str, line| {
            let opts = Options {
                align: Some(line),
                ..Options::default()
            };
            let out = render(&crate::doc::parse(text), &Theme::builtin(), params, opts);
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
    fn line_numbers_lead_the_rows_that_start_a_line() {
        let doc = crate::doc::parse("# Title\n\nsome text\n");
        let params = LayoutParams {
            width: 40,
            max_width: None,
            table_mode: Default::default(),
            code_wrap: true,
            line_numbers: crate::layout::LineNumbers::All,
            glyphs_gen: 0,
            images: None,
        };
        let out = render(&doc, &Theme::builtin(), params, Options::default());
        let mut plain = String::new();
        let mut esc = false;
        for c in out.chars() {
            match (esc, c) {
                (_, '\x1b') => esc = true,
                (true, 'm') => esc = false,
                (true, _) => {}
                (false, c) => plain.push(c),
            }
        }
        let lines: Vec<&str> = plain.lines().collect();
        assert!(
            lines[0].starts_with("1   ") && lines[0].contains("Title"),
            "{lines:?}"
        );
        assert_eq!(lines[1], "");
        assert!(lines[2].starts_with("3   some text"), "{lines:?}");
    }

    /// `text` as a note in a fresh directory beside a `w`×`h` PNG named `pic.png`, printed with
    /// images at 10×20-pixel cells.
    fn with_image(text: &str, w: u32, h: u32, proto: Proto, rows: Option<usize>) -> String {
        // a directory of its own for each call, as tests run at once
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let png = crate::images::tests::png(&format!("print{n}"), w, h);
        let dir = png.parent().unwrap();
        std::fs::rename(&png, dir.join("pic.png")).unwrap();
        let note = dir.join("note.md");
        let doc = crate::doc::parse(text);
        let cell = Size::new(10, 20);
        let pictures = Pictures::find(&doc, &note, proto, cell, false);
        let params = LayoutParams {
            width: 40,
            max_width: None,
            table_mode: Default::default(),
            code_wrap: true,
            line_numbers: Default::default(),
            glyphs_gen: 0,
            images: Some(crate::layout::ImageFit {
                cell,
                max_rows: 6,
                screen_rows: 10,
            }),
        };
        let opts = Options {
            rows,
            images: Some(&pictures),
            ..Options::default()
        };
        let out = render(&doc, &Theme::builtin(), params, opts);
        std::fs::remove_dir_all(dir).unwrap();
        out
    }

    #[test]
    fn kitty_images_are_sent_once_then_drawn_with_placeholders() {
        // 60×40 px: 6×2 cells, small enough to go as the PNG it is
        let out = with_image(
            "# Pic\n\n![a cat](pic.png)\n\nafter\n",
            60,
            40,
            Proto::Kitty,
            None,
        );
        assert_eq!(out.matches("\x1b_G").count(), 1, "{out:?}");
        assert!(out.contains(",c=6,r=2,f=100,m=0;"), "{out:?}");
        let rows: Vec<&str> = out.lines().filter(|l| l.contains('\u{10EEEE}')).collect();
        assert_eq!(rows.len(), 2);
        for row in rows {
            assert_eq!(row.matches('\u{10EEEE}').count(), 6, "{row:?}");
        }
        assert!(out.contains("a cat") && out.contains("after"));
        // too large for its cells: decoded, shrunk and deflated
        let out = with_image("![big](pic.png)\n", 2000, 400, Proto::Kitty, None);
        assert!(out.contains(",c=36,r=4,f=24,o=z,s=360,v=72,"), "{out:?}");
    }

    #[test]
    fn half_block_images_are_cells_of_two_pixels() {
        let out = with_image("![a cat](pic.png)\n", 60, 40, Proto::Halfblocks, None);
        let rows: Vec<&str> = out.lines().filter(|l| l.contains('▀')).collect();
        assert_eq!(rows.len(), 2, "{out:?}");
        assert!(rows.iter().all(|r| r.matches('▀').count() == 6));
        assert!(!out.contains("\x1b_G"));
    }

    #[test]
    fn a_row_budget_stops_the_output() {
        let text = "one\n\ntwo\n\nthree\n";
        assert_eq!(
            with_image(text, 1, 1, Proto::Kitty, Some(3))
                .lines()
                .count(),
            3
        );
        // a missing image is its link line
        let out = with_image("![gone](nope.png)\n", 1, 1, Proto::Kitty, None);
        assert!(out.contains("nope.png") && !out.contains("\x1b_G"));
    }

    #[test]
    fn code_is_highlighted() {
        let doc = crate::doc::parse("```rust\nlet s = \"hi\";\n```\n");
        let params = LayoutParams {
            width: 40,
            max_width: None,
            table_mode: Default::default(),
            code_wrap: true,
            line_numbers: Default::default(),
            glyphs_gen: 0,
            images: None,
        };
        let theme = Theme::builtin();
        let out = render(&doc, &theme, params, Options::default());
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
