//! Frames to the terminal. The diff against the last frame follows ratatui's `BufferDiff`, wide
//! graphemes and images included; the encoder writes each changed cell's cursor move, one SGR
//! sequence for what changed of its style, and its symbol, straight into one buffer. A list that
//! scrolled moves its rows in the terminal first, so only the rows that came in are drawn. Each
//! frame is one write and one synchronized update.

use std::io::{self, Write};
use std::ops::Range;

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

/// Synchronized output (DEC mode 2026): the terminal shows nothing of a frame until its end, so a
/// frame is never seen half drawn. Terminals without the mode ignore it.
const BEGIN_SYNC: &[u8] = b"\x1b[?2026h";
const END_SYNC: &[u8] = b"\x1b[?2026l";
/// Hidden after every frame, as ratatui does; `b2 flood` counts frames by it.
const HIDE_CURSOR: &[u8] = b"\x1b[?25l";

/// The terminal, as the frames written to it left it.
pub struct Screen<W: Write> {
    out: W,
    /// What the terminal shows.
    last: Grid,
    next: Grid,
    bytes: Vec<u8>,
    /// The size changed: the next frame clears the terminal first.
    clear: bool,
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
            clear: false,
        }
    }

    pub fn size(&self) -> Size {
        self.last.area.as_size()
    }

    /// A new size: the next frame clears the terminal and draws every cell, in one synchronized
    /// update, so the terminal never shows the cleared screen.
    pub fn resize(&mut self, size: Size) {
        let area = Rect::new(0, 0, size.width, size.height);
        self.last.resize(area);
        self.next.resize(area);
        self.clear = true;
    }

    /// Paint a frame on a blank grid and write what differs from the last one, in one write and
    /// one synchronized update.
    pub fn draw(&mut self, paint: impl FnOnce(&mut Grid)) -> io::Result<()> {
        self.next.reset();
        paint(&mut self.next);
        self.bytes.clear();
        self.bytes.extend_from_slice(BEGIN_SYNC);
        if std::mem::take(&mut self.clear) {
            self.bytes.extend_from_slice(b"\x1b[2J");
        }
        if let Some(scroll) = Scroll::between(&self.last, &self.next) {
            scroll.encode(self.last.area.bottom(), &mut self.bytes);
            self.last.scroll_rows(scroll.rows, scroll.by);
        }
        encode(&self.last, &self.next, &mut self.bytes);
        self.bytes.extend_from_slice(HIDE_CURSOR);
        // a frame that changes nothing needs no update around it
        let frame = if self.bytes.len() > BEGIN_SYNC.len() + HIDE_CURSOR.len() {
            self.bytes.extend_from_slice(END_SYNC);
            &self.bytes[..]
        } else {
            &self.bytes[BEGIN_SYNC.len()..]
        };
        self.out.write_all(frame)?;
        self.out.flush()?;
        std::mem::swap(&mut self.last, &mut self.next);
        Ok(())
    }
}

/// Rows the terminal moves before a frame is drawn: `rows` up by `by` rows, or down when it's
/// negative.
struct Scroll {
    rows: Range<u16>,
    by: i32,
}

impl Scroll {
    /// The move that lets `next` keep the most of `last`'s rows, when the painter marked the same
    /// rows of both as one list from different rows, and more of them match moved than in place.
    /// Rows at either end that don't match moved, such as a table's sticky header, stay out.
    fn between(last: &Grid, next: &Grid) -> Option<Scroll> {
        let ((top, bottom, from), (t, b, to)) = (last.scrolled?, next.scrolled?);
        if (top, bottom) != (t, b) || last.area != next.area || last.images || next.images {
            return None;
        }
        let by = i64::from(to) - i64::from(from);
        if by == 0 || by.unsigned_abs() >= u64::from(bottom - top) {
            return None;
        }
        let by = by as i32;
        let moved = |y: u16| same_row(next, y, last, (i32::from(y) + by) as u16);
        // the rows of `next` whose row in `last` is in the region, trimmed to those that match
        let (mut lo, mut hi) = (top + (-by).max(0) as u16, bottom - by.max(0) as u16);
        while lo < hi && !moved(lo) {
            lo += 1;
        }
        while hi > lo && !moved(hi - 1) {
            hi -= 1;
        }
        let rows = (i32::from(lo) + by.min(0)) as u16..(i32::from(hi) + by.max(0)) as u16;
        let kept = (lo..hi).filter(|&y| moved(y)).count();
        let in_place = rows.clone().filter(|&y| same_row(next, y, last, y)).count();
        (kept > in_place).then_some(Scroll { rows, by })
    }

    /// Set the margins to the rows unless they end at the screen's, then delete or insert lines
    /// from the first row, and reset the margins. `DECSTBM`, `DL` and `IL` are VT100 and VT102,
    /// unlike `SU` and `SD`, which the Linux console lacks. The style is the default when a frame
    /// starts, so the blank rows come in on the default background, with bce or without.
    fn encode(&self, screen_bottom: u16, out: &mut Vec<u8>) {
        let margins = self.rows.end < screen_bottom;
        if margins {
            out.extend_from_slice(b"\x1b[");
            number(out, self.rows.start + 1);
            out.push(b';');
            number(out, self.rows.end);
            out.push(b'r');
        }
        out.extend_from_slice(b"\x1b[");
        number(out, self.rows.start + 1);
        out.extend_from_slice(b"H\x1b[");
        let n = self.by.unsigned_abs() as u16;
        if n > 1 {
            number(out, n);
        }
        out.push(if self.by > 0 { b'M' } else { b'L' });
        if margins {
            out.extend_from_slice(b"\x1b[r");
        }
    }
}

/// Whether cell `i` is the same in both grids.
fn same(a: &Grid, b: &Grid, i: usize) -> bool {
    same_cell(a, i, b, i)
}

/// Whether cell `i` of `a` is the same as cell `j` of `b`.
fn same_cell(a: &Grid, i: usize, b: &Grid, j: usize) -> bool {
    let (x, y) = (&a.cells[i], &b.cells[j]);
    if x.bits() == y.bits() && x.len != LONG {
        return true;
    }
    let style = |c: &Cell| (c.meta, c.modifier, c.fg, c.bg);
    x.len == LONG && y.len == LONG && style(x) == style(y) && a.symbol_at(i) == b.symbol_at(j)
}

/// Whether row `ya` of `a` is the same as row `yb` of `b`, grids of one width.
fn same_row(a: &Grid, ya: u16, b: &Grid, yb: u16) -> bool {
    let w = usize::from(a.area.width);
    let i = usize::from(ya - a.area.y) * w;
    let j = usize::from(yb - b.area.y) * w;
    let (x, y) = (&a.cells[i..i + w], &b.cells[j..j + w]);
    // most rows have no long symbol, and compare as plain numbers
    if alike(x, y, |x, y| (x.bits() == y.bits()) & (x.len != LONG)) == w {
        return true;
    }
    (0..w).all(|k| same_cell(a, i + k, b, j + k))
}

/// How many cells from the start of `a` and `b` are alike by `alike`, eight at a time while they
/// all are, so that the comparisons don't branch on every cell.
fn alike(a: &[Cell], b: &[Cell], alike: impl Fn(&Cell, &Cell) -> bool) -> usize {
    let mut n = 0;
    for (x, y) in a.as_chunks::<8>().0.iter().zip(b.as_chunks::<8>().0) {
        if !x.iter().zip(y).fold(true, |all, (x, y)| all & alike(x, y)) {
            break;
        }
        n += 8;
    }
    n + a[n..]
        .iter()
        .zip(&b[n..])
        .take_while(|(x, y)| alike(x, y))
        .count()
}

/// Whether a cell shows nothing but its background: a narrow space with no style that marks a
/// blank. Its foreground and other modifiers don't show, so the encoder needn't send them.
fn blank(c: &Cell) -> bool {
    c.len == 1 && c.sym[0] == b' ' && c.meta == 1 && c.modifier & VISIBLE_ON_BLANK == 0
}

/// Where the row's run of blank cells on the default background starts, which an erase to the
/// end of the line can draw.
fn blank_tail(grid: &Grid, row: std::ops::Range<usize>) -> usize {
    let cells = &grid.cells[row.clone()];
    let blanks = cells
        .iter()
        .rev()
        .take_while(|c| blank(c) && c.bg == 0)
        .count();
    row.end - blanks
}

/// The cells of `next` that differ from `last`, as ratatui's `BufferDiff` picks them, encoded.
fn encode(last: &Grid, next: &Grid, out: &mut Vec<u8>) {
    let mut pen = Pen::new(next.area);
    let len = next.cells.len().min(last.cells.len());
    let w = usize::from(next.area.width).max(1);
    let mut tail = (usize::MAX, 0);
    let mut i = 0;
    while i < len {
        // the cells that didn't change and take one column, most of them, in one sweep
        i += alike(&next.cells[i..len], &last.cells[i..len], |c, p| {
            (c.bits() == p.bits()) & (c.meta == 1) & (c.len != LONG)
        });
        if i == len {
            break;
        }
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
        // The rest of the row is blank: erase it in one go, if more than this cell changed.
        let row = at / w;
        if tail.0 != row {
            tail = (row, blank_tail(next, row * w..(row + 1) * w));
        }
        let end = ((row + 1) * w).min(len);
        if at >= tail.1 && (at + 1..end).any(|k| !same(next, last, k)) {
            pen.erase_line(at, out);
            i = end;
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
        if trailing.is_none() && width == 1 {
            i = pen.run(next, last, i, end.min(tail.1), out);
        }
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
}

/// The terminal's cursor and current style while a frame is written. A frame starts with the
/// style reset (the last one ended with SGR 0) and the cursor unknown.
struct Pen {
    area: Rect,
    cursor: Option<(u16, u16)>,
    modifier: u16,
    fg: u32,
    bg: u32,
}

impl Pen {
    fn new(area: Rect) -> Pen {
        Pen {
            area,
            cursor: None,
            modifier: 0,
            fg: 0,
            bg: 0,
        }
    }

    fn styled(&self) -> bool {
        (self.modifier, self.fg, self.bg) != (0, 0, 0)
    }

    fn at(&self, i: usize) -> (u16, u16) {
        let w = usize::from(self.area.width);
        (self.area.x + (i % w) as u16, self.area.y + (i / w) as u16)
    }

    /// The style to write `c` with: the pen's, for a blank it can draw as it is, or else its
    /// own. A blank that needs a new background takes its own style too, since the cells after
    /// it are likely to share it.
    fn wants(&self, c: &Cell) -> (u16, u32, u32) {
        if blank(c) && self.modifier & VISIBLE_ON_BLANK == 0 && self.bg == c.bg {
            self.style()
        } else {
            (c.modifier, c.fg, c.bg)
        }
    }

    /// Write cell `i` of `grid`: move there unless the cursor is there, set what differs of its
    /// style, then its symbol.
    fn cell(&mut self, grid: &Grid, i: usize, out: &mut Vec<u8>) {
        let (x, y) = self.at(i);
        let c = &grid.cells[i];
        if self.cursor != Some((x, y)) {
            self.goto(grid, i, (x, y), out);
        }
        let wants = self.wants(c);
        if wants != self.style() {
            self.set(wants, out);
        }
        symbol(grid, i, out);
        // Terminals disagree on how wide some graphemes are (emoji with modifiers, ZWJ sequences,
        // halfwidth sound marks), and an image's escape sequence moves the cursor its own way, so
        // after anything but a narrow cell the next one is placed explicitly, as ratatui does.
        self.cursor = (c.width() == 1 && !c.forced()).then_some((x + 1, y));
    }

    /// Write the changed cells from `i` up to `end` that follow a narrow cell just written, while
    /// they are narrow too: the cursor is already at each, so each is what changed of its style
    /// and its symbol, as [`Pen::cell`] writes it. Returns where the run stopped.
    fn run(
        &mut self,
        next: &Grid,
        last: &Grid,
        mut i: usize,
        end: usize,
        out: &mut Vec<u8>,
    ) -> usize {
        let start = i;
        while i < end {
            let (c, p) = (&next.cells[i], &last.cells[i]);
            // a wide cell, a long symbol, or the leftovers of a wide one go the long way
            if c.meta != 1 || c.len == LONG || p.width() > 1 || c.bits() == p.bits() {
                break;
            }
            let wants = self.wants(c);
            if wants != self.style() {
                self.set(wants, out);
            }
            if c.len == 1 {
                out.push(c.sym[0]);
            } else {
                out.extend_from_slice(&c.sym[..usize::from(c.len)]);
            }
            i += 1;
        }
        if let Some((x, _)) = &mut self.cursor {
            *x += (i - start) as u16;
        }
        i
    }

    /// Clear from cell `i` to the end of its row, to the default background.
    fn erase_line(&mut self, i: usize, out: &mut Vec<u8>) {
        self.goto_cup(self.at(i), out);
        self.set((self.modifier & !VISIBLE_ON_BLANK, self.fg, 0), out);
        out.extend_from_slice(b"\x1b[K");
    }

    /// Move the cursor to cell `i` by the shortest way the pen knows: from where it is, by
    /// writing again the cells in between when they need no new style, or else forward, or to
    /// the start of the next row, or by row and column.
    fn goto(&mut self, grid: &Grid, i: usize, (x, y): (u16, u16), out: &mut Vec<u8>) {
        let cup = 3 + digits(y + 1) + if x == 0 { 0 } else { 1 + digits(x + 1) };
        match self.cursor {
            Some((cx, cy)) if cy == y && cx < x => {
                let n = x - cx;
                let forward = 3 + digits(n);
                let gap = i - usize::from(n)..i;
                let mut bytes = 0;
                let fits = grid.cells[gap.clone()].iter().all(|c| {
                    bytes += usize::from(c.len);
                    c.meta == 1
                        && c.len != LONG
                        && self.wants(c) == self.style()
                        && bytes <= cup.min(forward)
                });
                if fits {
                    for k in gap {
                        symbol(grid, k, out);
                    }
                } else if forward < cup {
                    forward_by(out, n);
                } else {
                    return self.goto_cup((x, y), out);
                }
            }
            Some((_, cy)) if y == cy + 1 && 2 + if x == 0 { 0 } else { 3 + digits(x) } < cup => {
                out.extend_from_slice(b"\r\n");
                if x > 0 {
                    forward_by(out, x);
                }
            }
            _ => return self.goto_cup((x, y), out),
        }
        self.cursor = Some((x, y));
    }

    fn goto_cup(&mut self, (x, y): (u16, u16), out: &mut Vec<u8>) {
        if self.cursor == Some((x, y)) {
            return;
        }
        out.extend_from_slice(b"\x1b[");
        number(out, y + 1);
        if x > 0 {
            out.push(b';');
            number(out, x + 1);
        }
        out.push(b'H');
        self.cursor = Some((x, y));
    }

    fn style(&self) -> (u16, u32, u32) {
        (self.modifier, self.fg, self.bg)
    }

    /// One SGR sequence for what differs of `(modifier, fg, bg)`, or none.
    fn set(&mut self, (modifier, fg, bg): (u16, u32, u32), out: &mut Vec<u8>) {
        let start = out.len();
        out.extend_from_slice(b"\x1b[");
        let params = out.len();
        if modifier != self.modifier {
            modifiers(out, self.modifier, modifier);
            self.modifier = modifier;
        }
        if fg != self.fg {
            color(out, 30, fg);
            self.fg = fg;
        }
        if bg != self.bg {
            color(out, 40, bg);
            self.bg = bg;
        }
        if out.len() == params {
            out.truncate(start);
        } else {
            // the last parameter's separator ends the sequence
            *out.last_mut().unwrap_or(&mut 0) = b'm';
        }
    }
}

/// Cell `i`'s symbol; most are one byte, which needn't go through a copy.
fn symbol(grid: &Grid, i: usize, out: &mut Vec<u8>) {
    let c = &grid.cells[i];
    if c.len == 1 {
        out.push(c.sym[0]);
    } else {
        out.extend_from_slice(grid.symbol_bytes(i));
    }
}

/// `CSI n C`: the cursor `n` columns right.
fn forward_by(out: &mut Vec<u8>, n: u16) {
    out.extend_from_slice(b"\x1b[");
    number(out, n);
    out.push(b'C');
}

fn digits(n: u16) -> usize {
    match n {
        0..10 => 1,
        10..100 => 2,
        100..1000 => 3,
        1000..10000 => 4,
        _ => 5,
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

/// A color's SGR parameters, followed by `;`. `base` is 30 for the foreground and 40 for the
/// background; named colors go as their 256-color index, as crossterm sends them.
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

    /// What the encoder wrote, replayed over the terminal's cells, within the scroll margins
    /// the terminal kept from the frame before.
    fn replay(cells: &mut [(String, Style)], area: Rect, margins: &mut Range<usize>, bytes: &[u8]) {
        let (mut x, mut y, mut style) = (0usize, 0usize, Style::reset());
        let w = usize::from(area.width);
        // `n` rows from row `y` to the bottom margin move up, or down when negative, and the
        // rows left behind are blank on the current background (bce)
        let shift = |cells: &mut [(String, Style)], y: usize, end: usize, n: isize, bg| {
            let rows = &mut cells[y * w..end * w];
            let k = n.unsigned_abs().min(end - y) * w;
            if n > 0 {
                rows.rotate_left(k);
            } else {
                rows.rotate_right(k);
            }
            let len = rows.len();
            let blank = if n > 0 { len - k..len } else { 0..k };
            for c in &mut rows[blank] {
                *c = (" ".to_owned(), Style::reset().bg(bg));
            }
        };
        let text = String::from_utf8_lossy(bytes);
        let mut rest = &*text;
        while !rest.is_empty() {
            if let Some(seq) = rest.strip_prefix("\x1b[") {
                let end = seq.find(|c: char| c.is_ascii_alphabetic()).unwrap();
                let (params, kind) = (&seq[..end], &seq[end..end + 1]);
                match kind {
                    "H" => {
                        let (r, c) = params.split_once(';').unwrap_or((params, "1"));
                        (y, x) = (
                            r.parse::<usize>().unwrap() - 1,
                            c.parse::<usize>().unwrap() - 1,
                        );
                    }
                    "C" => x += params.parse::<usize>().unwrap(),
                    "K" => {
                        let row = y * usize::from(area.width);
                        for c in &mut cells[row + x..row + usize::from(area.width)] {
                            *c = (" ".to_owned(), Style::reset().bg(style.bg.unwrap()));
                        }
                    }
                    "m" => style = sgr(style, params),
                    "r" => {
                        *margins = match params.split_once(';') {
                            Some((t, b)) => t.parse::<usize>().unwrap() - 1..b.parse().unwrap(),
                            None => 0..usize::from(area.height),
                        };
                        (x, y) = (0, 0);
                    }
                    "L" | "M" if margins.contains(&y) => {
                        let n = params.parse::<isize>().unwrap_or(1);
                        let n = if kind == "M" { n } else { -n };
                        shift(cells, y, margins.end, n, style.bg.unwrap());
                        x = 0;
                    }
                    _ => {}
                }
                rest = &seq[end + 1..];
                continue;
            }
            if let Some(after) = rest.strip_prefix('\r') {
                x = 0;
                rest = after;
                continue;
            }
            if let Some(after) = rest.strip_prefix('\n') {
                if y + 1 == margins.end {
                    shift(cells, margins.start, margins.end, 1, style.bg.unwrap());
                } else {
                    y += 1;
                }
                rest = after;
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
    }

    fn style_of(c: &Cell) -> Style {
        Style::reset()
            .fg(c.fg())
            .bg(c.bg())
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
                // a blank shows only its background
                let blank = s == " "
                    && !st
                        .add_modifier
                        .intersects(Modifier::from_bits_truncate(VISIBLE_ON_BLANK));
                (
                    s.clone(),
                    st.fg.map(indexed).filter(|_| !blank),
                    st.bg.map(indexed),
                    if blank {
                        Modifier::empty()
                    } else {
                        st.add_modifier
                    },
                )
            })
            .collect()
    }

    /// A terminal of `area` that the frames drawn on it are replayed on.
    struct Term {
        screen: Screen<Vec<u8>>,
        cells: Vec<(String, Style)>,
        margins: Range<usize>,
    }

    impl Term {
        fn new(area: Rect) -> Term {
            let blank = (" ".to_owned(), style_of(&Cell::BLANK));
            Term {
                screen: Screen::new(Vec::new(), area.as_size()),
                cells: vec![blank; area.area() as usize],
                margins: 0..usize::from(area.height),
            }
        }

        /// Draw a frame: what the encoder wrote, replayed, shows the frame. A frame that changes
        /// anything is one synchronized update, which `screen.out` is left holding the inside of.
        fn draw(&mut self, paint: impl FnOnce(&mut Grid)) {
            let screen = &mut self.screen;
            screen.out.clear();
            screen.draw(paint).unwrap();
            if screen.out != HIDE_CURSOR {
                let inner = screen
                    .out
                    .strip_prefix(BEGIN_SYNC)
                    .and_then(|o| o.strip_suffix(END_SYNC))
                    .expect("a frame in a synchronized update");
                assert!(inner.ends_with(HIDE_CURSOR), "the cursor hidden last");
                screen.out = inner.to_vec();
            }
            replay(
                &mut self.cells,
                screen.last.area,
                &mut self.margins,
                &screen.out,
            );
            let full = 0..usize::from(screen.last.area.height);
            assert_eq!(self.margins, full, "a frame leaves the margins reset");
            let want: Vec<_> = (0..screen.last.cells.len())
                .map(|i| {
                    (
                        screen.last.symbol_at(i).to_owned(),
                        style_of(&screen.last.cells[i]),
                    )
                })
                .collect();
            assert_eq!(seen(&self.cells), seen(&want));
        }
    }

    /// Frames painted in turn on a 10×3 terminal.
    fn frames(paints: &[&dyn Fn(&mut Grid)]) {
        let mut term = Term::new(Rect::new(0, 0, 10, 3));
        for paint in paints {
            term.draw(paint);
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

    /// A frame that changes anything is one synchronized update, a resize's clear inside it; one
    /// that changes nothing only hides the cursor.
    #[test]
    fn frames_are_synchronized_updates() {
        let mut screen = Screen::new(Vec::new(), Size::new(10, 3));
        let draw = |screen: &mut Screen<Vec<u8>>, text: &str| {
            screen.out.clear();
            screen
                .draw(|g| g.set_string(0, 0, text, Style::new()))
                .unwrap();
            String::from_utf8(screen.out.clone()).unwrap()
        };
        assert_eq!(
            draw(&mut screen, "hi"),
            "\x1b[?2026h\x1b[1Hhi\x1b[?25l\x1b[?2026l"
        );
        assert_eq!(draw(&mut screen, "hi"), "\x1b[?25l");
        screen.resize(Size::new(8, 3));
        assert_eq!(
            draw(&mut screen, "hi"),
            "\x1b[?2026h\x1b[2J\x1b[1Hhi\x1b[?25l\x1b[?2026l"
        );
    }

    /// Frames of random strings and styles, shifted a row or a column at a time as scrolling
    /// does, so the encoder's cursor moves, refills and erases all come up.
    #[test]
    fn random_frames_replay_to_the_frames() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let words = [
            "ab", "  ", "日本", "x y", "    ", "👍", "é", "wörd", " ", "ﾊﾞ",
        ];
        let styles = [
            Style::new(),
            Style::new().fg(Color::Rgb(200, 10, 10)),
            Style::new().bg(Color::Rgb(40, 40, 40)),
            Style::new().fg(Color::Blue).bg(Color::Rgb(40, 40, 40)),
            Style::new().underlined(),
            Style::new().reversed(),
            Style::new().bold().italic(),
            Style::new().crossed_out().fg(Color::Indexed(100)),
        ];
        let area = Rect::new(0, 0, 24, 8);
        let doc: Vec<Vec<(u16, &str, Style)>> = (0..40)
            .map(|_| {
                (0..next(5))
                    .map(|_| {
                        (
                            next(24) as u16,
                            words[next(words.len())],
                            styles[next(styles.len())],
                        )
                    })
                    .collect()
            })
            .collect();
        let mut term = Term::new(area);
        let (mut top, mut left, mut base, mut scrolled) = (0, 0, styles[0], 0);
        for frame in 0..300 {
            // a few rows up or down, and now and then a shift sideways or a new background
            top = (top + next(7)).saturating_sub(3).min(32);
            if next(4) == 0 {
                (left, base) = (next(3) as u16, styles[next(3)]);
            }
            term.draw(|g| {
                if frame % 7 != 0 {
                    g.set_style(area, base);
                }
                // rows 1 to 6 as one list, which the screen may scroll within margins
                g.set_scroll(Rect::new(0, 1, 24, 6), top as u32 + 1);
                for (y, row) in doc[top..top + 8].iter().enumerate() {
                    for &(x, word, style) in row {
                        g.set_string(x.saturating_sub(left), y as u16, word, style);
                    }
                }
            });
            scrolled += usize::from(term.screen.out.starts_with(b"\x1b[2;7r"));
        }
        assert!(scrolled > 50, "{scrolled} frames scrolled");
    }

    /// A list that scrolls moves its rows in the terminal: only the rows that came in are drawn.
    #[test]
    fn a_scrolled_list_moves_its_rows() {
        let list = |top: u32, header: bool, images: bool| {
            move |g: &mut Grid| {
                let rows = Rect::new(0, 0, 10, 5);
                g.set_scroll(rows, top);
                let red = Style::new().fg(Color::Rgb(200, 10, 10));
                for y in 0..5 {
                    g.set_string(0, y, format!("line {}", top + u32::from(y)), red);
                }
                if header {
                    g.set_string(0, 0, format!("{:<10}", "header"), Style::new().bold());
                }
                if g.area.height > 5 {
                    g.set_string(0, 5, format!("{top}%"), Style::new().reversed());
                }
                g.images = images;
            }
        };
        let mut term = Term::new(Rect::new(0, 0, 10, 6));
        let mut draw = |top, header, images| {
            term.draw(list(top, header, images));
            String::from_utf8(term.screen.out.clone()).unwrap()
        };
        draw(0, false, false);
        let down = draw(1, false, false);
        assert!(down.starts_with("\x1b[1;5r\x1b[1H\x1b[M\x1b[r"), "{down:?}");
        assert!(down.contains("line 5"), "{down:?}");
        assert!(
            !["line 2", "line 3", "line 4"]
                .iter()
                .any(|l| down.contains(l))
        );
        let up = draw(0, false, false);
        assert!(up.starts_with("\x1b[1;5r\x1b[1H\x1b[L\x1b[r"), "{up:?}");
        let three = draw(3, false, false);
        assert!(
            three.starts_with("\x1b[1;5r\x1b[1H\x1b[3M\x1b[r"),
            "{three:?}"
        );
        // a page down keeps no row, and images stay where they are
        for (top, images) in [(8, false), (9, true)] {
            let frame = draw(top, false, images);
            assert!(!frame.contains('r'), "{frame:?}");
        }
        // a sticky first row stays out of the margins
        draw(3, true, false);
        let sticky = draw(4, true, false);
        assert!(
            sticky.starts_with("\x1b[2;5r\x1b[2H\x1b[M\x1b[r"),
            "{sticky:?}"
        );
        assert!(!sticky.contains("header"), "{sticky:?}");

        // a list down to the screen's last row needs no margins
        let mut term = Term::new(Rect::new(0, 0, 10, 5));
        term.draw(list(0, false, false));
        term.draw(list(1, false, false));
        let out = String::from_utf8(term.screen.out.clone()).unwrap();
        assert!(out.starts_with("\x1b[1H\x1b[M\x1b[5H"), "{out:?}");
    }

    /// The test documents, scrolled and searched through the real view.
    #[test]
    fn scrolled_documents_replay_to_their_frames() {
        use crate::app::{Cmd, Config, Dir, Msg, ScrollBy, Size, State, update};
        let docs = [
            include_str!("../../tests/fixtures/basics.md"),
            include_str!("../../tests/fixtures/code.md"),
            include_str!("../../tests/fixtures/tables.md"),
            include_str!("../../tests/fixtures/wide.md"),
            include_str!("../../tests/fixtures/quotes.md"),
        ];
        for (w, h) in [(60, 20), (120, 40)] {
            for text in docs {
                let size = Size { w, h };
                let mut term = Term::new(Rect::new(0, 0, w, h));
                let mut state = State::new(crate::theme::Theme::builtin(), Config::default());
                update(&mut state, Msg::Init { size });
                let cmds = update(
                    &mut state,
                    Msg::DocLoaded {
                        path: "test.md".into(),
                        mtime: None,
                        text: text.into(),
                    },
                );
                term.draw(|g| crate::ui::view(&state, g));
                for cmd in cmds {
                    if let Cmd::Highlight(job) = cmd {
                        update(&mut state, job.run());
                    }
                }
                let mut moves = vec![ScrollBy::Rows(1); 6];
                moves.extend([
                    ScrollBy::Page(Dir::Next),
                    ScrollBy::HalfPage(Dir::Next),
                    ScrollBy::Rows(-1),
                    ScrollBy::Bottom,
                    ScrollBy::Page(Dir::Prev),
                    ScrollBy::Top,
                ]);
                for by in moves {
                    update(&mut state, Msg::Scroll { by });
                    term.draw(|g| crate::ui::view(&state, g));
                }
            }
        }
    }
}
