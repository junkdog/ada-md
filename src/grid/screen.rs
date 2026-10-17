//! Frames to the terminal. The diff against the last frame follows ratatui's `BufferDiff`, wide
//! graphemes and images included; the encoder writes each changed cell's cursor move, one SGR
//! sequence for what changed of its style, and its symbol, straight into one buffer.

use std::io::{self, Write};

use ratatui::layout::{Rect, Size};
use ratatui::style::Modifier;

use super::{Cell, Grid, LONG};

/// Styles that show on a blank cell, so a wide grapheme's leftover columns need redrawing.
const VISIBLE_ON_BLANK: u16 = Modifier::REVERSED
    .union(Modifier::UNDERLINED)
    .union(Modifier::SLOW_BLINK)
    .union(Modifier::RAPID_BLINK)
    .union(Modifier::CROSSED_OUT)
    .bits();

/// The terminal, as the frames written to it left it.
pub struct Screen<W: Write> {
    out: W,
    /// What the terminal shows.
    last: Grid,
    next: Grid,
    bytes: Vec<u8>,
}

impl<W: Write> Screen<W> {
    /// A screen of `size` over a blank terminal, such as a new alternate screen.
    pub fn new(out: W, size: Size) -> Screen<W> {
        let area = Rect::new(0, 0, size.width, size.height);
        Screen {
            out,
            last: Grid::empty(area),
            next: Grid::empty(area),
            bytes: Vec::new(),
        }
    }

    pub fn size(&self) -> Size {
        self.last.area.as_size()
    }

    /// Clear the terminal for a new size, so the next frame draws every cell.
    pub fn resize(&mut self, size: Size) -> io::Result<()> {
        let area = Rect::new(0, 0, size.width, size.height);
        self.last.resize(area);
        self.next.resize(area);
        self.out.write_all(b"\x1b[2J")?;
        self.out.flush()
    }

    /// Paint a frame on a blank grid and write what differs from the last one, in one write.
    pub fn draw(&mut self, paint: impl FnOnce(&mut Grid)) -> io::Result<()> {
        self.next.reset();
        paint(&mut self.next);
        self.bytes.clear();
        encode(&self.last, &self.next, &mut self.bytes);
        self.out.write_all(&self.bytes)?;
        self.out.flush()?;
        std::mem::swap(&mut self.last, &mut self.next);
        Ok(())
    }
}

/// Whether cell `i` is the same in both grids.
fn same(a: &Grid, b: &Grid, i: usize) -> bool {
    let (x, y) = (&a.cells[i], &b.cells[i]);
    if x.len != LONG && y.len != LONG {
        return x == y;
    }
    let style = |c: &Cell| (c.meta, c.modifier, c.fg, c.bg, c.underline);
    x.len == y.len && style(x) == style(y) && a.symbol_at(i) == b.symbol_at(i)
}

/// The cells of `next` that differ from `last`, as ratatui's `BufferDiff` picks them, encoded.
fn encode(last: &Grid, next: &Grid, out: &mut Vec<u8>) {
    let mut pen = Pen::new(next.area);
    let len = next.cells.len().min(last.cells.len());
    let mut i = 0;
    while i < len {
        let (cur, prev) = (&next.cells[i], &last.cells[i]);
        let at = i;
        i += 1;
        if cur.skip() {
            continue;
        }
        let width = usize::from(cur.width());
        if cur.forced() {
            i += width.saturating_sub(1);
            if !same(next, last, at) {
                pen.cell(next, at, out);
            }
            continue;
        }
        if same(next, last, at) {
            i += width.saturating_sub(1);
            continue;
        }
        // A wide emoji with VS16 may be drawn one column wide, and a wide grapheme replaced by a
        // narrow one may leave its style on the columns it covered: redraw those after it.
        let mut trailing = None;
        if width > 1 && next.symbol_at(at).contains('\u{FE0F}') {
            trailing = Some((at + 1, (at + width).min(len), false));
        } else if width > 1 {
            i += width - 1;
        } else if usize::from(prev.width()) > width
            && (prev.bg != 0 || prev.modifier & VISIBLE_ON_BLANK != 0)
        {
            trailing = Some((at + 1, (at + usize::from(prev.width())).min(len), true));
        }
        pen.cell(next, at, out);
        if let Some((mut j, mut end, force)) = trailing {
            while j < end {
                let k = j;
                j += usize::from(next.cells[k].width()).max(1);
                end = end.max(j).min(len);
                if !next.cells[k].skip() && (force || last.symbol_at(k) != next.symbol_at(k)) {
                    pen.cell(next, k, out);
                }
            }
            i = end;
        }
    }
    if pen.styled() {
        out.extend_from_slice(b"\x1b[0m");
    }
    // as ratatui does after every frame; `b2 flood` counts frames by it
    out.extend_from_slice(b"\x1b[?25l");
}

/// The terminal's cursor and current style while a frame is written. A frame starts with the
/// style reset (the last one ended with SGR 0) and the cursor unknown.
struct Pen {
    area: Rect,
    cursor: Option<(u16, u16)>,
    modifier: u16,
    fg: u32,
    bg: u32,
    underline: u32,
}

impl Pen {
    fn new(area: Rect) -> Pen {
        Pen {
            area,
            cursor: None,
            modifier: 0,
            fg: 0,
            bg: 0,
            underline: 0,
        }
    }

    fn styled(&self) -> bool {
        (self.modifier, self.fg, self.bg, self.underline) != (0, 0, 0, 0)
    }

    /// Write cell `i` of `grid`: move there unless the cursor is there, set what differs of its
    /// style, then its symbol.
    fn cell(&mut self, grid: &Grid, i: usize, out: &mut Vec<u8>) {
        let w = usize::from(self.area.width);
        let (x, y) = (self.area.x + (i % w) as u16, self.area.y + (i / w) as u16);
        let c = &grid.cells[i];
        if self.cursor != Some((x, y)) {
            out.extend_from_slice(b"\x1b[");
            number(out, y + 1);
            out.push(b';');
            number(out, x + 1);
            out.push(b'H');
        }
        let start = out.len();
        out.extend_from_slice(b"\x1b[");
        let params = out.len();
        if c.modifier != self.modifier {
            modifiers(out, self.modifier, c.modifier);
            self.modifier = c.modifier;
        }
        if c.fg != self.fg {
            color(out, 30, c.fg);
            self.fg = c.fg;
        }
        if c.bg != self.bg {
            color(out, 40, c.bg);
            self.bg = c.bg;
        }
        if c.underline != self.underline {
            color(out, 50, c.underline);
            self.underline = c.underline;
        }
        if out.len() == params {
            out.truncate(start);
        } else {
            // the last parameter's separator ends the sequence
            *out.last_mut().unwrap_or(&mut 0) = b'm';
        }
        out.extend_from_slice(grid.symbol_bytes(i));
        // Terminals disagree on how wide some graphemes are (emoji with modifiers, ZWJ sequences,
        // halfwidth sound marks), and an image's escape sequence moves the cursor its own way, so
        // after anything but a narrow cell the next one is placed explicitly, as ratatui does.
        self.cursor = (c.width() == 1 && !c.forced()).then_some((x + 1, y));
    }
}

/// SGR parameters from `from` to `to`, as ratatui's crossterm backend sends them, each followed
/// by `;`.
fn modifiers(out: &mut Vec<u8>, from: u16, to: u16) {
    let (from, to) = (
        Modifier::from_bits_truncate(from),
        Modifier::from_bits_truncate(to),
    );
    let mut sgr = |n: &[u8]| {
        out.extend_from_slice(n);
        out.push(b';');
    };
    let removed = from - to;
    if removed.contains(Modifier::REVERSED) {
        sgr(b"27");
    }
    let reset_intensity = removed.intersects(Modifier::BOLD | Modifier::DIM);
    if reset_intensity {
        sgr(b"22");
        if to.contains(Modifier::DIM) {
            sgr(b"2");
        }
        if to.contains(Modifier::BOLD) {
            sgr(b"1");
        }
    }
    for (m, n) in [
        (Modifier::ITALIC, &b"23"[..]),
        (Modifier::UNDERLINED, b"24"),
        (Modifier::CROSSED_OUT, b"29"),
        (Modifier::HIDDEN, b"28"),
    ] {
        if removed.contains(m) {
            sgr(n);
        }
    }
    if removed.intersects(Modifier::SLOW_BLINK | Modifier::RAPID_BLINK) {
        sgr(b"25");
    }
    let added = to - from;
    for (m, n) in [
        (Modifier::REVERSED, &b"7"[..]),
        (Modifier::BOLD, b"1"),
        (Modifier::ITALIC, b"3"),
        (Modifier::UNDERLINED, b"4"),
        (Modifier::DIM, b"2"),
        (Modifier::CROSSED_OUT, b"9"),
        (Modifier::HIDDEN, b"8"),
        (Modifier::SLOW_BLINK, b"5"),
        (Modifier::RAPID_BLINK, b"6"),
    ] {
        let intensity = m == Modifier::BOLD || m == Modifier::DIM;
        if added.contains(m) && !(intensity && reset_intensity) {
            sgr(n);
        }
    }
}

/// A color's SGR parameters, followed by `;`. `base` is 30 for the foreground, 40 for the
/// background and 50 for the underline; named colors go as their 256-color index, as crossterm
/// sends them.
fn color(out: &mut Vec<u8>, base: u16, c: u32) {
    let [kind, r, g, b] = c.to_be_bytes();
    match kind {
        0 => number(out, base + 9),
        1 => {
            number(out, base + 8);
            out.extend_from_slice(b";2;");
            number(out, r.into());
            out.push(b';');
            number(out, g.into());
            out.push(b';');
            number(out, b.into());
        }
        _ => {
            number(out, base + 8);
            out.extend_from_slice(b";5;");
            number(out, b.into());
        }
    }
    out.push(b';');
}

fn number(out: &mut Vec<u8>, n: u16) {
    let mut digits = [0u8; 5];
    let mut i = digits.len();
    let mut n = n;
    loop {
        i -= 1;
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    out.extend_from_slice(&digits[i..]);
}

#[cfg(test)]
mod tests {
    use super::*;

    use ratatui::style::{Color, Style};

    /// A frame's cells, read back from what the encoder wrote: replayed over the last frame's.
    fn replay(last: &Grid, bytes: &[u8]) -> Vec<(String, Style)> {
        let area = last.area;
        let mut cells: Vec<(String, Style)> = (0..last.cells.len())
            .map(|i| (last.symbol_at(i).to_owned(), style_of(&last.cells[i])))
            .collect();
        let (mut x, mut y, mut style) = (0usize, 0usize, Style::reset());
        let text = String::from_utf8_lossy(bytes);
        let mut rest = &*text;
        while !rest.is_empty() {
            if let Some(seq) = rest.strip_prefix("\x1b[") {
                let end = seq.find(|c: char| c.is_ascii_alphabetic()).unwrap();
                let (params, kind) = (&seq[..end], &seq[end..end + 1]);
                match kind {
                    "H" => {
                        let (r, c) = params.split_once(';').unwrap();
                        (y, x) = (
                            r.parse::<usize>().unwrap() - 1,
                            c.parse::<usize>().unwrap() - 1,
                        );
                    }
                    "m" => style = sgr(style, params),
                    _ => {}
                }
                rest = &seq[end + 1..];
                continue;
            }
            let g = unicode_segmentation::UnicodeSegmentation::graphemes(rest, true)
                .next()
                .unwrap();
            let at = y * usize::from(area.width) + x;
            let width = usize::from(super::super::cell_width(g).max(1));
            cells[at] = (g.to_owned(), style);
            for covered in &mut cells[at + 1..at + width] {
                *covered = (" ".to_owned(), style_of(&Cell::BLANK));
            }
            x += width;
            rest = &rest[g.len()..];
        }
        cells
    }

    fn style_of(c: &Cell) -> Style {
        Style::reset()
            .fg(c.fg())
            .bg(c.bg())
            .underline_color(super::super::unpack(c.underline))
            .add_modifier(c.modifier())
    }

    fn sgr(mut style: Style, params: &str) -> Style {
        let p: Vec<u8> = params.split(';').map(|p| p.parse().unwrap()).collect();
        let mut it = p.iter().copied();
        let color = |it: &mut dyn Iterator<Item = u8>| match it.next() {
            Some(2) => Color::Rgb(it.next().unwrap(), it.next().unwrap(), it.next().unwrap()),
            _ => Color::Indexed(it.next().unwrap()),
        };
        while let Some(n) = it.next() {
            let m = |m| style.add_modifier(m);
            let off = |m| style.remove_modifier(m);
            style = match n {
                0 => Style::reset(),
                1 => m(Modifier::BOLD),
                2 => m(Modifier::DIM),
                3 => m(Modifier::ITALIC),
                4 => m(Modifier::UNDERLINED),
                7 => m(Modifier::REVERSED),
                9 => m(Modifier::CROSSED_OUT),
                22 => off(Modifier::BOLD | Modifier::DIM),
                23 => off(Modifier::ITALIC),
                24 => off(Modifier::UNDERLINED),
                27 => off(Modifier::REVERSED),
                29 => off(Modifier::CROSSED_OUT),
                38 => style.fg(color(&mut it)),
                39 => style.fg(Color::Reset),
                48 => style.bg(color(&mut it)),
                49 => style.bg(Color::Reset),
                58 => style.underline_color(color(&mut it)),
                59 => style.underline_color(Color::Reset),
                n => panic!("unexpected SGR {n}"),
            };
        }
        style
    }

    /// Named colors as their index, as the terminal sees them.
    fn indexed(c: Color) -> Color {
        let packed = super::super::pack(c);
        match packed >> 24 {
            3 => Color::Indexed(packed as u8),
            _ => c,
        }
    }

    fn seen(cells: &[(String, Style)]) -> Vec<(String, Option<Color>, Option<Color>, Modifier)> {
        cells
            .iter()
            .map(|(s, st)| {
                (
                    s.clone(),
                    st.fg.map(indexed),
                    st.bg.map(indexed),
                    st.add_modifier,
                )
            })
            .collect()
    }

    /// Frames painted in turn: after each, what the encoder wrote, replayed over the last frame,
    /// is the frame.
    fn frames(paints: &[&dyn Fn(&mut Grid)]) {
        let area = Rect::new(0, 0, 10, 3);
        let mut screen = Screen::new(Vec::new(), area.as_size());
        for paint in paints {
            let last = screen.last.clone();
            screen.out.clear();
            screen.draw(|g| paint(g)).unwrap();
            let want: Vec<_> = (0..screen.last.cells.len())
                .map(|i| {
                    (
                        screen.last.symbol_at(i).to_owned(),
                        style_of(&screen.last.cells[i]),
                    )
                })
                .collect();
            assert_eq!(seen(&replay(&last, &screen.out)), seen(&want));
        }
    }

    #[test]
    fn the_encoded_frames_replay_to_the_frames() {
        let bg = Style::new().bg(Color::Rgb(40, 40, 40)).fg(Color::Gray);
        frames(&[
            &|g| {
                g.set_style(g.area(), bg);
                g.set_string(0, 0, "hello 日本", Style::new().bold());
                g.set_string(
                    1,
                    1,
                    "x",
                    Style::new().underlined().underline_color(Color::Red),
                );
            },
            &|g| {
                g.set_style(g.area(), bg);
                g.set_string(0, 0, "hel", Style::new().italic());
                g.set_string(6, 0, "ab", Style::new().reversed());
                g.set_string(0, 2, "→ two", Style::new().fg(Color::Indexed(100)).dim());
            },
            &|g| {
                g.set_style(g.area(), bg);
                g.set_string(0, 2, "→ two", Style::new().fg(Color::Indexed(100)));
            },
        ]);
    }
}
