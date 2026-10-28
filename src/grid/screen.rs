//! Frames to the terminal. The diff against the last frame follows ratatui's `BufferDiff`, wide
//! graphemes and images included; the encoder writes each changed cell's cursor move, one SGR
//! sequence for what changed of its style, and its symbol, straight into one buffer. A list that
//! scrolled moves its rows in the terminal first, so only the rows that came in are drawn. Each
//! frame is one write and one synchronized update.

use std::io::{self, Write};
use std::ops::Range;

use crate::geom::{Rect, Size};
use crate::style::{Color, Kind, Modifier};

use super::{Cell, Grid, LONG};
use crate::kitty::{self, PLACEHOLDER as KITTY_PLACEHOLDER};

/// Styles that show on a blank cell: a blank with none of them shows only its background.
const VISIBLE_ON_BLANK: u16 = Modifier::REVERSED
    .union(Modifier::UNDERLINED)
    .union(Modifier::SLOW_BLINK)
    .union(Modifier::RAPID_BLINK)
    .union(Modifier::CROSSED_OUT)
    .bits();

/// The pen's style when a frame starts and after SGR 0: no modifier, the default colors.
const PLAIN: (u16, u32, u32) = (0, Color::RESET.bits(), Color::RESET.bits());

/// Synchronized output (DEC mode 2026): the terminal shows nothing of a frame until its end, so a
/// frame is never seen half drawn. Terminals without the mode ignore it.
const BEGIN_SYNC: &[u8] = b"\x1b[?2026h";
const END_SYNC: &[u8] = b"\x1b[?2026l";
/// Hidden after every frame that changes something, as ratatui does after every frame, and by
/// a screen's first; `b2 flood` counts frames by it.
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
    /// A frame written has hidden the cursor. Nothing else shows it while the screen lives: the
    /// editor's return makes a new one.
    hidden: bool,
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
            hidden: false,
        }
    }

    pub fn size(&self) -> Size {
        self.last.area.as_size()
    }

    /// What the terminal shows, as the frames written so far left it.
    pub fn shown(&self) -> &Grid {
        &self.last
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
    /// one synchronized update. A frame that changes nothing writes nothing once the cursor is
    /// hidden (TRM-01: each highlight result for a block off screen cost a write and a syscall).
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
        let changed = self.bytes.len() > BEGIN_SYNC.len();
        if !changed && self.hidden {
            std::mem::swap(&mut self.last, &mut self.next);
            return Ok(());
        }
        self.bytes.extend_from_slice(HIDE_CURSOR);
        // a frame that changes nothing needs no update around it
        let frame = if changed {
            self.bytes.extend_from_slice(END_SYNC);
            &self.bytes[..]
        } else {
            &self.bytes[BEGIN_SYNC.len()..]
        };
        self.out.write_all(frame)?;
        self.out.flush()?;
        self.hidden = true;
        std::mem::swap(&mut self.last, &mut self.next);
        Ok(())
    }

    /// Write the top `rows` rows of what the terminal shows as lines from the cursor, once ada
    /// has left the alternate screen, so that they stay on the screen and in the scrollback after
    /// it quits.
    pub fn keep(&mut self, rows: u16) -> io::Result<()> {
        self.bytes.clear();
        keep(&self.last, rows, &mut self.bytes);
        self.out.write_all(&self.bytes)?;
        self.out.flush()
    }
}

/// Rows of `grid` as lines from the cursor, which is where the shell's next line would go, so
/// nothing about where they land is known. Each line is its cells left to right, with the style
/// reset before the newline so that a scroll brings in a default blank row. Kitty images' cells
/// are text like the rest (the image goes to the terminal before). Other images' cells go as
/// blanks on their background; then, from the saved cursor below the last line, each image's
/// escape sequence is written up and across at its first cell.
fn keep(grid: &Grid, rows: u16, out: &mut Vec<u8>) {
    let w = usize::from(grid.area.width);
    let rows = rows.min(grid.area.height);
    let mut pen = Pen::new(grid.area);
    let mut images = Vec::new();
    for y in 0..usize::from(rows) {
        let mut x = 0;
        while x < w {
            let i = y * w + x;
            let c = &grid.cells[i];
            // a kitty image's cells are text, which takes its id from the foreground color
            let placeholder = c.forced() && grid.symbol_at(i).contains(KITTY_PLACEHOLDER);
            if c.is_image() && !placeholder {
                if c.forced() {
                    images.push(i);
                }
                pen.set((0, pen.fg, c.bg), out);
                out.push(b' ');
                x += 1;
                continue;
            }
            pen.set(pen.wants(c), out);
            symbol(grid, i, out);
            let width = usize::from(c.width()).max(1);
            x += width;
            // terminals disagree on how wide some graphemes are: the next cell goes where the
            // grid has it
            if width > 1 && x < w {
                out.extend_from_slice(b"\x1b[");
                number(out, x as u16 + 1);
                out.push(b'G');
            }
        }
        pen.set(PLAIN, out);
        out.extend_from_slice(b"\r\n");
    }
    if images.is_empty() {
        return;
    }
    out.extend_from_slice(b"\x1b7");
    let to = |i: usize, out: &mut Vec<u8>| {
        let (x, y) = (i % w, i / w);
        out.extend_from_slice(b"\x1b8\x1b[");
        number(out, rows - y as u16);
        out.push(b'A');
        if x > 0 {
            out.extend_from_slice(b"\x1b[");
            number(out, x as u16 + 1);
            out.push(b'G');
        }
    };
    for i in images {
        to(i, out);
        symbol(grid, i, out);
        // the image erased its area: what was drawn over parts of it goes over it again, from
        // the restored cursor's style, the default
        let Some(&(_, area)) = grid.covers.iter().rfind(|(first, _)| *first == i) else {
            continue;
        };
        for y in area.top()..area.bottom().min(rows) {
            // each row from the restored cursor, in its style, the default; then across it
            let mut at = None;
            let mut x = usize::from(area.x);
            while x < usize::from(area.right()) {
                let k = usize::from(y) * w + x;
                let c = &grid.cells[k];
                let width = usize::from(c.width()).max(1);
                if !c.skip() && k != i {
                    match at {
                        None => {
                            to(k, out);
                            (pen.modifier, pen.fg, pen.bg) = PLAIN;
                        }
                        Some(at) if at != x => {
                            out.extend_from_slice(b"\x1b[");
                            number(out, x as u16 + 1);
                            out.push(b'G');
                        }
                        Some(_) => {}
                    }
                    pen.set(pen.wants(c), out);
                    symbol(grid, k, out);
                    // after a wide grapheme, the next cell goes where the grid has it
                    at = Some(if width == 1 { x + 1 } else { usize::MAX });
                }
                x += width;
            }
        }
    }
    // an image's escape sequence may leave a style set, which the restore resets
    out.extend_from_slice(b"\x1b8");
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
    // most rows have no long symbol, and compare as plain bytes
    if a.plain(usize::from(ya - a.area.y)) {
        return bytes(x) == bytes(y);
    }
    if alike(x, y, |x, y| (x.bits() == y.bits()) & (x.len != LONG)) == w {
        return true;
    }
    (0..w).all(|k| same_cell(a, i + k, b, j + k))
}

/// The cells as their bytes, which compare in a few wide loads.
fn bytes(cells: &[Cell]) -> &[u8] {
    // SAFETY: a `Cell` is `repr(C)` integers with no padding, 16 bytes, so every byte of the
    // slice is initialized, and `u8` has no alignment to meet.
    unsafe { std::slice::from_raw_parts(cells.as_ptr().cast(), size_of_val(cells)) }
}

/// The first cell of `next` from `i` up to `len` that changed or doesn't take one column alone,
/// or `len`. A row of `next` with nothing but narrow, short symbols compares as plain bytes: a
/// cell of `last` that is any other kind can't equal one of them (TRM-04).
fn unchanged(next: &Grid, last: &Grid, mut i: usize, len: usize) -> usize {
    let w = usize::from(next.area.width);
    while i < len {
        let y = i / w;
        let end = ((y + 1) * w).min(len);
        let (x, p) = (&next.cells[i..end], &last.cells[i..end]);
        let n = if !next.plain(y) {
            alike(x, p, |c, p| {
                (c.bits() == p.bits()) & (c.meta == 1) & (c.len != LONG)
            })
        } else if bytes(x) == bytes(p) {
            x.len()
        } else {
            alike(x, p, |c, p| c.bits() == p.bits())
        };
        i += n;
        if n < x.len() {
            break;
        }
    }
    i
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

/// Where the row's run of blank cells on one background starts, and the background, which an
/// erase to the end of the line can draw: terminals erase with the current background (bce),
/// so a theme's background erases as the default does (TRM-02).
fn blank_tail(grid: &Grid, row: std::ops::Range<usize>) -> (usize, u32) {
    let cells = &grid.cells[row.clone()];
    let bg = cells.last().map_or(PLAIN.2, |c| c.bg);
    let blanks = cells
        .iter()
        .rev()
        .take_while(|c| blank(c) && c.bg == bg)
        .count();
    (row.end - blanks, bg)
}

/// How many cells from `i` up to `end` an erase on background `bg` can draw: blanks of `next`
/// on it, over narrow cells of `last`, changed or not.
fn blanks(next: &Grid, last: &Grid, i: usize, end: usize, bg: u32) -> usize {
    next.cells[i..end]
        .iter()
        .zip(&last.cells[i..end])
        .take_while(|(c, p)| blank(c) && c.bg == bg && p.width() <= 1)
        .count()
}

/// The cells of `next` that differ from `last`, as ratatui's `BufferDiff` picks them, encoded.
fn encode(last: &Grid, next: &Grid, out: &mut Vec<u8>) {
    let mut pen = Pen::new(next.area);
    let len = next.cells.len().min(last.cells.len());
    let w = usize::from(next.area.width).max(1);
    let mut tail = (usize::MAX, 0, 0);
    let mut i = 0;
    while i < len {
        // the cells that didn't change and take one column, most of them, in one sweep
        i = unchanged(next, last, i, len);
        if i == len {
            break;
        }
        let cur = &next.cells[i];
        let at = i;
        i += 1;
        if cur.skip() {
            continue;
        }
        let width = usize::from(cur.width());
        if cur.forced() {
            i += width.saturating_sub(1);
            match next.covers.iter().rfind(|(first, _)| *first == at) {
                // A sixel or iTerm2 image erases its whole area and draws over it, what was
                // drawn over parts of it since included (a popup): when the image is written,
                // so are they, after it, the same or not. And when any cell of its area changed,
                // such as a popup over it closing, the image is written again.
                Some(&(_, area)) => {
                    let cells = |area: Rect| {
                        (area.top()..area.bottom()).flat_map(move |y| {
                            let row = usize::from(y) * w;
                            row + usize::from(area.x)..row + usize::from(area.right())
                        })
                    };
                    if !same(next, last, at) || cells(area).any(|k| !same(next, last, k)) {
                        pen.cell(next, at, out);
                        let mut k = at + 1;
                        for k0 in cells(area).filter(|&k0| k0 > at) {
                            if k0 < k {
                                continue;
                            }
                            k = k0 + usize::from(next.cells[k0].width()).max(1);
                            if !next.cells[k0].skip() {
                                pen.cell(next, k0, out);
                            }
                        }
                    }
                }
                None if !same(next, last, at) => pen.cell(next, at, out),
                None => {}
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
            let (start, bg) = blank_tail(next, row * w..(row + 1) * w);
            tail = (row, start, bg);
        }
        let end = ((row + 1) * w).min(len);
        if at >= tail.1 && (at + 1..end).any(|k| !same(next, last, k)) {
            pen.erase_line(at, tail.2, out);
            i = end;
            continue;
        }
        // A grapheme written over part of a wide one, a narrow one over its first column or a
        // wide one over either, leaves the columns the wide one covered past it to the
        // terminal, which erases them by its own rules (tmux and alacritty to a default blank,
        // others with the pen, F-04): redraw those after it, even where the grid's cell is the
        // same, as a blank a wide grapheme covered is the same as any other. Each one redrawn
        // may be written over part of another.
        let reach = |k: usize, w: usize| {
            (k..(k + w).min(len))
                .map(|k| k + usize::from(last.cells[k].width()).max(1))
                .fold(k + w, usize::max)
                .min(len)
        };
        let span = (at + width.max(1)).min(len);
        let covered = reach(at, width.max(1));
        let trailing = (covered > span).then_some((span, covered));
        i = span;
        // A wide emoji with VS16 is drawn two columns wide by most terminals and one by some:
        // the column it covers goes first, so the emoji covers it in the first kind and leaves
        // it shown in the second. Drawn after the emoji, the column would erase it (F-04).
        if width > 1 && next.symbol_at(at).contains('\u{FE0F}') {
            for k in at + 1..span {
                if !next.cells[k].skip() && last.symbol_at(k) != next.symbol_at(k) {
                    pen.cell(next, k, out);
                }
            }
        }
        pen.cell(next, at, out);
        if trailing.is_none() && width == 1 {
            i = pen.run(next, last, i, end.min(tail.1), out);
        }
        if let Some((mut j, mut end)) = trailing {
            while j < end {
                let (k, w) = (j, usize::from(next.cells[j].width()).max(1));
                j += w;
                end = end.max(reach(k, w));
                if !next.cells[k].skip() {
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
            modifier: PLAIN.0,
            fg: PLAIN.1,
            bg: PLAIN.2,
        }
    }

    fn styled(&self) -> bool {
        self.style() != PLAIN
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
        // after anything but a narrow cell the next one is placed explicitly, as ratatui does. A
        // kitty placeholder is a narrow character in the terminals that draw them.
        let narrow = c.width() == 1 && (!c.forced() || c.placed().is_some());
        self.cursor = narrow.then_some((x + 1, y));
    }

    /// Write the changed cells from `i` up to `end` that follow a narrow cell just written, while
    /// they are narrow too: the cursor is already at each, so each is what changed of its style
    /// and its symbol, as [`Pen::cell`] writes it. A long run of blanks on one background is
    /// erased instead, with the cursor moved past it when the run goes on. Returns where the run
    /// stopped.
    fn run(
        &mut self,
        next: &Grid,
        last: &Grid,
        mut i: usize,
        end: usize,
        out: &mut Vec<u8>,
    ) -> usize {
        let start = i;
        // cells erased ahead of the cursor
        let mut erased = 0;
        while i < end {
            let (c, p) = (&next.cells[i], &last.cells[i]);
            // a wide cell, a long symbol, or the leftovers of a wide one go the long way
            if c.meta != 1 || c.len == LONG || p.width() > 1 || c.bits() == p.bits() {
                break;
            }
            if erased > 0 {
                forward_by(out, erased as u16);
                erased = 0;
            }
            if blank(c) {
                let n = blanks(next, last, i, end, c.bg);
                if n > 2 * (3 + digits(n as u16)) {
                    self.erase(n as u16, c.bg, out);
                    (i, erased) = (i + n, n);
                    continue;
                }
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
            *x += (i - start - erased) as u16;
        }
        i
    }

    /// Clear from cell `i` to the end of its row, to background `bg`.
    fn erase_line(&mut self, i: usize, bg: u32, out: &mut Vec<u8>) {
        self.goto_cup(self.at(i), out);
        self.set((self.modifier & !VISIBLE_ON_BLANK, self.fg, bg), out);
        out.extend_from_slice(b"\x1b[K");
    }

    /// Clear `n` cells from the cursor, to background `bg` (`ECH`), leaving the cursor where it
    /// is.
    fn erase(&mut self, n: u16, bg: u32, out: &mut Vec<u8>) {
        self.set((self.modifier & !VISIBLE_ON_BLANK, self.fg, bg), out);
        out.extend_from_slice(b"\x1b[");
        number(out, n);
        out.push(b'X');
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
    } else if let Some(at) = c.placed() {
        placeholder(grid, i, at, out);
    } else {
        out.extend_from_slice(grid.symbol_bytes(i));
    }
}

/// A kitty placeholder at `row` and `col` of its image, with the diacritics that number them
/// unless kitty can tell both from the cell to its left. Once the frame is written the terminal
/// shows the grid's cell there, and when that's the same image's previous column, the placeholder
/// alone is this one; anything else there, such as a popup's border, and it's numbered.
fn placeholder(grid: &Grid, i: usize, (row, col): (u16, u16), out: &mut Vec<u8>) {
    let follows = !i.is_multiple_of(usize::from(grid.area.width)) && col > 0 && {
        let left = &grid.cells[i - 1];
        left.fg == grid.cells[i].fg && left.placed() == Some((row, col - 1))
    };
    let chars = kitty::placeholder(row, col);
    let mut buf = [0; 4];
    for c in &chars[..if follows { 1 } else { 3 }] {
        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
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
/// background; the 16 named colors (indexes 0 to 15) go as `3x` and `9x`, as print writes them.
fn color(out: &mut Vec<u8>, base: u16, c: u32) {
    match Color::from_bits(c).kind() {
        Kind::Reset => number(out, base + 9),
        Kind::Rgb(r, g, b) => {
            number(out, base + 8);
            out.extend_from_slice(b";2;");
            number(out, r.into());
            out.push(b';');
            number(out, g.into());
            out.push(b';');
            number(out, b.into());
        }
        Kind::Indexed(n @ 0..8) => number(out, base + u16::from(n)),
        Kind::Indexed(n @ 8..16) => number(out, base + 60 + u16::from(n - 8)),
        Kind::Indexed(n) => {
            number(out, base + 8);
            out.extend_from_slice(b";5;");
            number(out, n.into());
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

    use crate::style::Style;

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
                    "G" => x = params.parse::<usize>().unwrap() - 1,
                    "K" | "X" => {
                        let row = y * usize::from(area.width);
                        let end = match kind {
                            "K" => usize::from(area.width),
                            _ => {
                                (x + params.parse::<usize>().unwrap()).min(usize::from(area.width))
                            }
                        };
                        for c in &mut cells[row + x..row + end] {
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
        let p: Vec<u8> = params
            .split(';')
            .map(|p| p.parse::<u8>().unwrap())
            .collect();
        let mut it = p.iter().copied();
        let color = |it: &mut dyn Iterator<Item = u8>| match it.next() {
            Some(2) => Color::rgb(it.next().unwrap(), it.next().unwrap(), it.next().unwrap()),
            _ => Color::indexed(it.next().unwrap()),
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
                30..=37 => style.fg(Color::indexed(n - 30)),
                90..=97 => style.fg(Color::indexed(n - 90 + 8)),
                40..=47 => style.bg(Color::indexed(n - 40)),
                100..=107 => style.bg(Color::indexed(n - 100 + 8)),
                38 => style.fg(color(&mut it)),
                39 => style.fg(Color::RESET),
                48 => style.bg(color(&mut it)),
                49 => style.bg(Color::RESET),
                n => panic!("unexpected SGR {n}"),
            };
        }
        style
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
                    st.fg.filter(|_| !blank),
                    st.bg,
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
            if !screen.out.is_empty() && screen.out != HIDE_CURSOR {
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
        let bg = Style::new().bg(Color::rgb(40, 40, 40)).fg(Color::GRAY);
        frames(&[
            &|g| {
                g.set_style(g.area(), bg);
                g.set_string(0, 0, "hello 日本", Style::new().bold());
                g.set_string(
                    1,
                    1,
                    "x",
                    Style::new().underlined().underline_color(Color::RED),
                );
            },
            &|g| {
                g.set_style(g.area(), bg);
                g.set_string(0, 0, "hel", Style::new().italic());
                g.set_string(6, 0, "ab", Style::new().reversed());
                g.set_string(0, 2, "→ two", Style::new().fg(Color::indexed(100)).dim());
            },
            &|g| {
                g.set_style(g.area(), bg);
                g.set_string(0, 2, "→ two", Style::new().fg(Color::indexed(100)));
            },
        ]);
    }

    /// Blanks on a theme's background are erased with it set, as on the default one (TRM-02):
    /// a row's tail to the end of the line, a long run inside it in place, the cursor moved past
    /// it when the row goes on.
    #[test]
    fn blanks_on_a_background_are_erased() {
        let theme = Style::new().bg(Color::rgb(40, 40, 40)).fg(Color::GRAY);
        let code = Style::new().bg(Color::rgb(60, 56, 54)).fg(Color::GREEN);
        let mut term = Term::new(Rect::new(0, 0, 40, 3));
        let page = |text: &'static str| {
            move |g: &mut Grid| {
                g.set_style(g.area(), theme);
                g.set_style(Rect::new(2, 1, 30, 1), code);
                g.set_string(2, 0, text, theme);
                g.set_string(3, 1, text, code);
                g.set_string(34, 1, "end", theme);
            }
        };
        term.draw(page("a paragraph's line"));
        term.draw(page("x"));
        let out = String::from_utf8_lossy(&term.screen.out).into_owned();
        assert!(out.contains("\x1b[K"), "the row's tail erased: {out:?}");
        assert!(out.contains("X\x1b["), "the code's padding erased: {out:?}");
        assert!(!out.contains("    "), "no run of spaces: {out:?}");
        // a run that ends the changed cells leaves the cursor where it erased from (above);
        // one that goes on moves it past the erased cells
        term.draw(page("a paragraph's line"));
        term.draw(|g| {
            page("x")(g);
            g.set_string(32, 1, "END", theme);
        });
        let out = String::from_utf8_lossy(&term.screen.out).into_owned();
        assert!(out.contains("26X\x1b[26C"), "erased, then past it: {out:?}");
    }

    /// The 16 named colors go as `3x`/`9x` and `4x`/`10x`, as print writes them, and other
    /// indexes as `38;5;n`.
    #[test]
    fn named_colors_go_by_their_own_codes() {
        let mut term = Term::new(Rect::new(0, 0, 12, 1));
        term.draw(|g| {
            g.set_string(0, 0, "a", Style::new().fg(Color::RED).bg(Color::BLACK));
            g.set_string(
                1,
                0,
                "b",
                Style::new().fg(Color::WHITE).bg(Color::LIGHT_BLUE),
            );
            g.set_string(2, 0, "c", Style::new().fg(Color::indexed(16)));
        });
        let out = String::from_utf8_lossy(&term.screen.out).into_owned();
        for sgr in ["\x1b[31;40ma", "\x1b[97;104mb", "\x1b[38;5;16;49mc"] {
            assert!(out.contains(sgr), "{sgr:?} in {out:?}");
        }
    }

    /// A frame that changes anything is one synchronized update, a resize's clear inside it; one
    /// that changes nothing writes nothing once the cursor is hidden, and only hides it before.
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
        assert_eq!(draw(&mut screen, ""), "\x1b[?25l");
        assert_eq!(draw(&mut screen, ""), "");
        assert_eq!(
            draw(&mut screen, "hi"),
            "\x1b[?2026h\x1b[1Hhi\x1b[?25l\x1b[?2026l"
        );
        assert_eq!(draw(&mut screen, "hi"), "");
        screen.resize(Size::new(8, 3));
        assert_eq!(
            draw(&mut screen, "hi"),
            "\x1b[?2026h\x1b[2J\x1b[1Hhi\x1b[?25l\x1b[?2026l"
        );
    }

    /// A narrow grapheme over a wide one redraws the column the wide one covered, even blank on
    /// blank: terminals erase it by their own rules, some with the pen just used (F-04).
    #[test]
    fn a_narrow_grapheme_over_a_wide_one_redraws_its_other_column() {
        let mut screen = Screen::new(Vec::new(), Size::new(10, 3));
        screen
            .draw(|g| g.set_string(0, 0, "日x", Style::new()))
            .unwrap();
        screen.out.clear();
        let border = Style::new().bg(Color::rgb(60, 56, 54));
        screen
            .draw(|g| {
                g.set_string(0, 0, "│", border);
                g.set_string(2, 0, "x", Style::new());
            })
            .unwrap();
        assert_eq!(
            text_of(&screen.out),
            "│ ",
            "{:?}",
            screen.out.escape_ascii()
        );
    }

    /// A wide grapheme over the first column of another wide one redraws the column the other
    /// one covered, even when the grid's cell there is unchanged: the terminal erased it (F-04).
    #[test]
    fn a_wide_grapheme_over_half_a_wide_one_redraws_its_other_column() {
        let mut screen = Screen::new(Vec::new(), Size::new(10, 3));
        let hit = Style::new().bg(Color::rgb(254, 128, 25));
        let paint = |g: &mut Grid, x| {
            g.set_string(x, 0, "日", Style::new());
            g.set_style(Rect::new(2, 0, 1, 1), hit);
        };
        screen.draw(|g| paint(g, 1)).unwrap();
        screen.out.clear();
        screen.draw(|g| paint(g, 0)).unwrap();
        assert_eq!(
            text_of(&screen.out),
            "日 ",
            "{:?}",
            screen.out.escape_ascii()
        );
    }

    /// A column redrawn after a narrow grapheme over a wide one can itself be a wide grapheme
    /// over half of the next wide one, whose other column is redrawn too (F-04).
    #[test]
    fn a_redrawn_column_over_half_a_wide_one_redraws_its_other_column() {
        let mut screen = Screen::new(Vec::new(), Size::new(10, 3));
        let hit = Style::new().bg(Color::rgb(254, 128, 25));
        let paint = |g: &mut Grid, s| {
            g.set_string(1, 0, s, Style::new());
            g.set_style(Rect::new(4, 0, 1, 1), hit);
        };
        screen.draw(|g| paint(g, "日日")).unwrap();
        screen.out.clear();
        screen.draw(|g| paint(g, "x本")).unwrap();
        assert_eq!(
            text_of(&screen.out),
            "x本 ",
            "{:?}",
            screen.out.escape_ascii()
        );
    }

    /// A VS16 emoji's covered column is drawn before the emoji, which a terminal drawing it two
    /// columns wide would erase if it came after (F-04).
    #[test]
    fn a_vs16_emoji_comes_after_the_column_it_covers() {
        let mut screen = Screen::new(Vec::new(), Size::new(10, 3));
        screen
            .draw(|g| g.set_string(0, 0, "ab", Style::new()))
            .unwrap();
        screen.out.clear();
        screen
            .draw(|g| g.set_string(0, 0, "❤\u{fe0f}", Style::new()))
            .unwrap();
        assert_eq!(
            text_of(&screen.out),
            " ❤\u{fe0f}",
            "{:?}",
            screen.out.escape_ascii()
        );
    }

    /// The text a frame wrote, without its escape sequences.
    fn text_of(out: &[u8]) -> String {
        let mut text = String::new();
        let mut chars = std::str::from_utf8(out).unwrap().chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                chars.by_ref().find(char::is_ascii_alphabetic);
            } else {
                text.push(c);
            }
        }
        text
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
            Style::new().fg(Color::rgb(200, 10, 10)),
            Style::new().bg(Color::rgb(40, 40, 40)),
            Style::new().fg(Color::BLUE).bg(Color::rgb(40, 40, 40)),
            Style::new().underlined(),
            Style::new().reversed(),
            Style::new().bold().italic(),
            Style::new().crossed_out().fg(Color::indexed(100)),
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
                let red = Style::new().fg(Color::rgb(200, 10, 10));
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
        use crate::app::{Cmd, Computed, Config, Dir, Msg, ScrollBy, Size, State, update};
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
                        doc: Computed(None),
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

    /// The rows kept on quit, written as lines over a blank terminal, show as the frame did: its
    /// cells, wide graphemes and styles, with no row or column given by number.
    #[test]
    fn kept_rows_replay_to_the_frame() {
        let area = Rect::new(0, 0, 10, 3);
        let mut term = Term::new(area);
        let bg = Style::new().bg(Color::rgb(40, 40, 40)).fg(Color::GRAY);
        term.draw(|g| {
            g.fill(bg);
            g.set_string(0, 0, "hello 日本", Style::new().bold());
            g.set_string(1, 1, "→ ok", Style::new().fg(Color::RED));
            g.set_string(0, 2, "status", Style::new().reversed());
        });
        term.screen.out.clear();
        term.screen.keep(2).unwrap();
        let out = std::mem::take(&mut term.screen.out);
        assert!(!out.contains(&b'H'), "{:?}", out.escape_ascii());
        let blank = (" ".to_owned(), style_of(&Cell::BLANK));
        let mut cells = vec![blank; area.area() as usize];
        replay(&mut cells, area, &mut (0..3), &out);
        let want: Vec<_> = (0..20)
            .map(|i| {
                let last = &term.screen.last;
                (last.symbol_at(i).to_owned(), style_of(&last.cells[i]))
            })
            .collect();
        assert_eq!(seen(&cells[..20]), seen(&want));
        assert!(
            cells[20..]
                .iter()
                .all(|(s, st)| s == " " && st.bg == Some(Color::RESET))
        );
    }

    /// An image's cells are kept as blanks, then its escape sequence goes over them, moved to
    /// from the saved cursor below the lines.
    #[test]
    fn kept_images_are_drawn_over_their_cells() {
        let mut screen = Screen::new(Vec::new(), Size::new(6, 3));
        screen
            .draw(|g| g.image(Rect::new(1, 1, 3, 1), "\x1bPq#0!3~\x1b\\"))
            .unwrap();
        screen.out.clear();
        screen.keep(3).unwrap();
        let out = String::from_utf8(screen.out).unwrap();
        assert!(
            out.ends_with("\x1b7\x1b8\x1b[2A\x1b[2G\x1bPq#0!3~\x1b\\\x1b8"),
            "{out:?}"
        );
        let lines: Vec<_> = out.split("\r\n").collect();
        assert_eq!(lines[1], "      ", "the image's cells as blanks");
    }

    /// A sixel or iTerm2 image erases its area and draws over it: what a popup drew over part of
    /// it is written after it whenever it's written, though unchanged, and the image is written
    /// again when the popup closes (fuzz round 8, 2026-10-21).
    #[test]
    fn what_is_drawn_over_an_image_goes_after_it() {
        let mut screen = Screen::new(Vec::new(), Size::new(6, 3));
        let draw = |screen: &mut Screen<Vec<u8>>, seq: &str, popup: bool| {
            screen.out.clear();
            screen
                .draw(|g| {
                    g.image(Rect::new(0, 1, 6, 2), seq);
                    if popup {
                        // a popup clears its cells first, which an image's skipped cells need
                        g[(2, 1)].reset();
                        g[(3, 1)].reset();
                        g.set_stringn(2, 1, "ab", 2, Style::new());
                    }
                })
                .unwrap();
            String::from_utf8(std::mem::take(&mut screen.out)).unwrap()
        };
        let after = |out: &str, seq: &str| out.find(seq).is_some_and(|i| out[i..].contains("ab"));
        let (a, b) = ("\x1bPq#0!6~\x1b\\", "\x1bPq#1!6~\x1b\\");
        let first = draw(&mut screen, a, true);
        assert!(after(&first, a), "{first:?}");
        // the image written again, a scroll's other rows say, and the popup the same
        assert!(after(&draw(&mut screen, b, true), b));
        // nothing changed: nothing written
        assert!(!draw(&mut screen, b, true).contains(b));
        // the popup closed: the image again, over where it was
        assert!(draw(&mut screen, b, false).contains(b));
        // and kept on quit: the image, then the popup over it
        draw(&mut screen, a, true);
        screen.keep(3).unwrap();
        let out = String::from_utf8(screen.out).unwrap();
        assert!(after(&out, a), "{out:?}");
        assert_eq!(out.matches(a).count(), 1, "{out:?}");
    }

    /// A kitty image's cells are text in its color, which names the image: they're kept in the
    /// lines, with nothing drawn over them after.
    #[test]
    fn kept_kitty_placeholders_are_text_in_their_color() {
        let mut screen = Screen::new(Vec::new(), Size::new(4, 2));
        let id = Color::rgb(1, 2, 3);
        screen
            .draw(|g| {
                g.placeholder((1, 0), (0, 0), id);
                g.placeholder((2, 0), (0, 1), id);
            })
            .unwrap();
        screen.out.clear();
        screen.keep(1).unwrap();
        let out = String::from_utf8(screen.out).unwrap();
        assert_eq!(
            out,
            " \x1b[38;2;1;2;3m\u{10EEEE}\u{305}\u{305}\u{10EEEE} \x1b[39m\r\n"
        );
    }

    /// A kitty placeholder is numbered unless the cell to its left is the same image's previous
    /// column in the same row, which kitty numbers it from: not after a popup's border over the
    /// image, a gap, another image, or at the start of a row. The cursor follows them.
    #[test]
    fn placeholders_are_numbered_where_kitty_cant_tell() {
        let mut screen = Screen::new(Vec::new(), Size::new(7, 2));
        let (a, b) = (Color::rgb(0, 0, 1), Color::rgb(0, 0, 2));
        screen
            .draw(|g| {
                for x in 0..7 {
                    g.placeholder((x, 0), (0, x), a);
                    g.placeholder((x, 1), (1, x), a);
                }
                // a popup's border, which keeps the image cell's width, and a gap
                g.set_string(2, 0, "│", Style::new());
                g.placeholder((5, 0), (0, 6), a);
                g.placeholder((6, 0), (0, 7), a);
                g.placeholder((6, 1), (0, 6), b);
            })
            .unwrap();
        let out = String::from_utf8(screen.out).unwrap();
        let p = |row: u16, col: u16| {
            let d = |n| crate::kitty::placeholder(0, n)[2];
            format!("\u{10EEEE}{}{}", d(row), d(col))
        };
        let q = "\u{10EEEE}";
        let rows = [
            format!("{}{q}│\x1b[1;4H{}{q}{}{q}", p(0, 0), p(0, 3), p(0, 6)),
            format!("{}{q}{q}{q}{q}{q}\x1b[38;2;0;0;2m{}", p(1, 0), p(0, 6)),
        ];
        let frame = format!("\x1b[1H\x1b[38;2;0;0;1m{}\r\n{}\x1b[0m", rows[0], rows[1]);
        assert!(out.contains(&frame), "{out:?}");
    }
}
