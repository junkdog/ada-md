//! ada's own frame, in place of ratatui's `Buffer`: a grid of small `Copy` cells that [`ui`] paints
//! and [`Screen`] diffs and writes to the terminal. Painting keeps ratatui's semantics (a style
//! patches the cell's, a wide grapheme blanks the cells it covers, graphemes with control
//! characters are dropped), so the frames are the same; what changes is the cost per cell.
//!
//! [`ui`]: crate::ui

mod screen;

pub use screen::Screen;

use std::num::NonZeroU16;
use std::ops::{Index, IndexMut, Range};

use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// `Cell::len` when the symbol doesn't fit in `sym`, which then holds its index in `Grid::long`.
const LONG: u8 = u8::MAX;
/// `Cell::meta`: the low four bits are the width in columns.
const WIDTH: u8 = 0b1111;
/// The terminal leaves the cell alone: an image covers it.
const SKIP: u8 = 1 << 4;
/// The symbol is written as is and counts as `WIDTH` columns: an image's escape sequence.
const FORCED: u8 = 1 << 5;

/// One terminal cell: 16 bytes, `Copy`, compared as plain bytes. There is no underline color: an
/// underline takes the foreground's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    /// The grapheme's UTF-8, zero-padded, or the index of a longer one.
    sym: [u8; 4],
    len: u8,
    meta: u8,
    modifier: u16,
    fg: u32,
    bg: u32,
}

const _: () = assert!(size_of::<Cell>() == 16);

impl Cell {
    /// A space with no style, as ratatui's `Cell::EMPTY`.
    pub const BLANK: Cell = Cell {
        sym: [b' ', 0, 0, 0],
        len: 1,
        meta: 1,
        modifier: 0,
        fg: 0,
        bg: 0,
    };

    pub fn reset(&mut self) {
        *self = Cell::BLANK;
    }

    pub fn fg(&self) -> Color {
        unpack(self.fg)
    }

    pub fn bg(&self) -> Color {
        unpack(self.bg)
    }

    pub fn modifier(&self) -> Modifier {
        Modifier::from_bits_truncate(self.modifier)
    }

    fn width(&self) -> u16 {
        u16::from(self.meta & WIDTH)
    }

    fn skip(&self) -> bool {
        self.meta & SKIP != 0
    }

    fn forced(&self) -> bool {
        self.meta & FORCED != 0
    }

    /// Patch `style` over the cell's: only what `style` sets changes.
    fn set_style(&mut self, style: Style) {
        if let Some(c) = style.fg {
            self.fg = pack(c);
        }
        if let Some(c) = style.bg {
            self.bg = pack(c);
        }
        self.modifier = (self.modifier | style.add_modifier.bits()) & !style.sub_modifier.bits();
    }
}

/// The cells of `area`, row by row.
#[derive(Clone, Debug)]
pub struct Grid {
    area: Rect,
    cells: Vec<Cell>,
    /// Symbols too long for a cell: graphemes over four bytes and images' escape sequences.
    long: Vec<String>,
    /// The rows that show a scrolled list, and the list's row at the first of them.
    scrolled: Option<(u16, u16, u32)>,
    /// An image protocol wrote escape sequences into cells, which the terminal may not move
    /// with its rows.
    images: bool,
}

impl Grid {
    pub fn empty(area: Rect) -> Grid {
        Grid {
            area,
            cells: vec![Cell::BLANK; usize::from(area.width) * usize::from(area.height)],
            long: Vec::new(),
            scrolled: None,
            images: false,
        }
    }

    pub fn area(&self) -> Rect {
        self.area
    }

    /// Every cell blank.
    pub fn reset(&mut self) {
        self.cells.fill(Cell::BLANK);
        self.long.clear();
        self.scrolled = None;
        self.images = false;
    }

    /// Blank, at a new size.
    pub fn resize(&mut self, area: Rect) {
        self.area = area;
        self.cells.clear();
        self.cells.resize(
            usize::from(area.width) * usize::from(area.height),
            Cell::BLANK,
        );
        self.long.clear();
        self.scrolled = None;
        self.images = false;
    }

    /// The rows of `area` show a list from its row `top` on. When the next frame shows the same
    /// list from another row, [`Screen`] can move the rows in the terminal instead of drawing
    /// them again. Only a full-width area can scroll.
    pub fn set_scroll(&mut self, area: Rect, top: u32) {
        let area = self.area.intersection(area);
        if !area.is_empty() && area.width == self.area.width {
            self.scrolled = Some((area.top(), area.bottom(), top));
        }
    }

    /// Move `rows` up by `by` rows, or down when it's negative, as the terminal's delete or
    /// insert line does: the rows left behind are blank.
    fn scroll_rows(&mut self, rows: Range<u16>, by: i32) {
        let w = usize::from(self.area.width);
        let start = usize::from(rows.start - self.area.y) * w;
        let end = usize::from(rows.end - self.area.y) * w;
        let n = by.unsigned_abs() as usize * w;
        if by > 0 {
            self.cells.copy_within(start + n..end, start);
            self.cells[end - n..end].fill(Cell::BLANK);
        } else {
            self.cells.copy_within(start..end - n, start + n);
            self.cells[start..start + n].fill(Cell::BLANK);
        }
    }

    /// The index of cell `(x, y)`; panics outside the area, as ratatui's buffer does.
    fn index(&self, x: u16, y: u16) -> usize {
        let a = self.area;
        assert!(
            x >= a.left() && x < a.right() && y >= a.top() && y < a.bottom(),
            "({x}, {y}) is outside the grid's {a:?}"
        );
        usize::from(y - a.y) * usize::from(a.width) + usize::from(x - a.x)
    }

    pub fn symbol(&self, (x, y): (u16, u16)) -> &str {
        self.symbol_at(self.index(x, y))
    }

    fn symbol_at(&self, i: usize) -> &str {
        let c = &self.cells[i];
        if c.len == LONG {
            &self.long[u32::from_le_bytes(c.sym) as usize]
        } else {
            std::str::from_utf8(&c.sym[..usize::from(c.len)]).unwrap_or(" ")
        }
    }

    /// The symbol's bytes, for the encoder.
    fn symbol_bytes(&self, i: usize) -> &[u8] {
        let c = &self.cells[i];
        if c.len == LONG {
            self.long[u32::from_le_bytes(c.sym) as usize].as_bytes()
        } else {
            &c.sym[..usize::from(c.len)]
        }
    }

    /// Set cell `i`'s symbol, keeping its style. An image's flags stay, as ratatui's diff option
    /// does, and a forced width with them.
    fn put(&mut self, i: usize, symbol: &str, width: u16) {
        let long = self.long.len() as u32;
        let c = &mut self.cells[i];
        if symbol.len() <= 4 {
            c.sym = [0; 4];
            c.sym[..symbol.len()].copy_from_slice(symbol.as_bytes());
            c.len = symbol.len() as u8;
        } else {
            c.sym = long.to_le_bytes();
            c.len = LONG;
            self.long.push(symbol.to_owned());
        }
        if !c.forced() {
            c.meta = (c.meta & SKIP) | width.min(u16::from(WIDTH)) as u8;
        }
    }

    /// Write `string` from `(x, y)`, within `max_width` columns and the grid, patching `style`
    /// over each cell. Returns where it ended. As ratatui's `Buffer::set_stringn`, but an ASCII
    /// character that no combining mark follows skips grapheme segmentation.
    pub fn set_stringn(
        &mut self,
        mut x: u16,
        y: u16,
        string: impl AsRef<str>,
        max_width: usize,
        style: Style,
    ) -> (u16, u16) {
        let s = string.as_ref();
        let max_width = u16::try_from(max_width).unwrap_or(u16::MAX);
        let mut remaining = self.area.right().saturating_sub(x).min(max_width);
        if remaining == 0 {
            return (x, y);
        }
        let row = self.index(self.area.x, y);
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            let (g, width) = if b.is_ascii() && bytes.get(i + 1).is_none_or(u8::is_ascii) {
                i += 1;
                if b.is_ascii_control() {
                    continue;
                }
                (&s[i - 1..i], 1)
            } else {
                let g = s[i..].graphemes(true).next().unwrap_or(&s[i..]);
                i += g.len();
                let width = cell_width(g);
                if width == 0 || g.contains(char::is_control) {
                    continue;
                }
                (g, width)
            };
            let Some(rest) = remaining.checked_sub(width) else {
                break;
            };
            remaining = rest;
            let at = row + usize::from(x - self.area.x);
            self.put(at, g, width);
            self.cells[at].set_style(style);
            // the cells a wide grapheme covers are blank
            for c in &mut self.cells[at + 1..at + usize::from(width)] {
                *c = Cell::BLANK;
            }
            x += width;
        }
        (x, y)
    }

    pub fn set_string(&mut self, x: u16, y: u16, string: impl AsRef<str>, style: Style) {
        self.set_stringn(x, y, string, usize::MAX, style);
    }

    /// Patch `style` over every cell of `area` within the grid.
    pub fn set_style(&mut self, area: Rect, style: Style) {
        let area = self.area.intersection(area);
        for y in area.top()..area.bottom() {
            let row = self.index(area.x, y);
            for c in &mut self.cells[row..row + usize::from(area.width)] {
                c.set_style(style);
            }
        }
    }

    /// Let ratatui code draw on `area` of the grid: the cells go out as a ratatui buffer, `draw`
    /// paints on it, and they come back with whatever it changed. For images, whose protocols
    /// write escape sequences into cells.
    pub fn lend(&mut self, area: Rect, draw: impl FnOnce(&mut Buffer)) {
        let area = self.area.intersection(area);
        if area.is_empty() {
            return;
        }
        let mut buf = Buffer::empty(area);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let i = self.index(x, y);
                let c = self.cells[i];
                let out = &mut buf[(x, y)];
                out.set_symbol(self.symbol_at(i));
                out.fg = c.fg();
                out.bg = c.bg();
                out.modifier = c.modifier();
                out.diff_option = match NonZeroU16::new(c.width()) {
                    _ if c.skip() => CellDiffOption::Skip,
                    Some(w) if c.forced() => CellDiffOption::ForcedWidth(w),
                    _ => CellDiffOption::None,
                };
            }
        }
        draw(&mut buf);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let i = self.index(x, y);
                let from = &buf[(x, y)];
                let symbol = from.symbol();
                let (flags, width) = match from.diff_option {
                    CellDiffOption::Skip => (SKIP, cell_width(symbol)),
                    CellDiffOption::ForcedWidth(w) => (FORCED, w.get()),
                    _ => (0, cell_width(symbol)),
                };
                self.images |= flags != 0;
                self.cells[i].meta = 0;
                self.put(i, symbol, width);
                let c = &mut self.cells[i];
                c.meta = flags | width.min(u16::from(WIDTH)) as u8;
                c.fg = pack(from.fg);
                c.bg = pack(from.bg);
                c.modifier = from.modifier.bits();
            }
        }
    }
}

impl Index<(u16, u16)> for Grid {
    type Output = Cell;

    fn index(&self, (x, y): (u16, u16)) -> &Cell {
        &self.cells[Grid::index(self, x, y)]
    }
}

impl IndexMut<(u16, u16)> for Grid {
    fn index_mut(&mut self, (x, y): (u16, u16)) -> &mut Cell {
        let i = Grid::index(self, x, y);
        &mut self.cells[i]
    }
}

/// Columns a grapheme takes, as ratatui counts them (halfwidth sound marks take one each).
fn cell_width(g: &str) -> u16 {
    let marks = g
        .chars()
        .filter(|c| matches!(c, '\u{FF9E}' | '\u{FF9F}'))
        .count();
    (g.width() + marks) as u16
}

/// The sixteen named colors, by their ANSI index.
const NAMED: [Color; 16] = [
    Color::Black,
    Color::Red,
    Color::Green,
    Color::Yellow,
    Color::Blue,
    Color::Magenta,
    Color::Cyan,
    Color::Gray,
    Color::DarkGray,
    Color::LightRed,
    Color::LightGreen,
    Color::LightYellow,
    Color::LightBlue,
    Color::LightMagenta,
    Color::LightCyan,
    Color::White,
];

/// A color in four bytes: the kind in the top byte (0 reset, 1 RGB, 2 indexed, 3 named), the
/// value below.
fn pack(c: Color) -> u32 {
    match c {
        Color::Reset => 0,
        Color::Rgb(r, g, b) => 1 << 24 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b),
        Color::Indexed(i) => 2 << 24 | u32::from(i),
        named => 3 << 24 | NAMED.iter().position(|&n| n == named).unwrap_or(0) as u32,
    }
}

fn unpack(c: u32) -> Color {
    let [kind, r, g, b] = c.to_be_bytes();
    match kind {
        1 => Color::Rgb(r, g, b),
        2 => Color::Indexed(b),
        3 => NAMED[usize::from(b & 15)],
        _ => Color::Reset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same calls on a ratatui buffer and a grid leave the same symbols and styles.
    fn same_as_ratatui(paint: impl Fn(&mut dyn Canvas)) {
        let area = Rect::new(0, 0, 12, 3);
        let mut buf = Buffer::empty(area);
        let mut grid = Grid::empty(area);
        paint(&mut buf);
        paint(&mut grid);
        for y in 0..3 {
            for x in 0..12 {
                let (b, g) = (&buf[(x, y)], &grid[(x, y)]);
                let at = format!("at ({x}, {y})");
                assert_eq!(b.symbol(), grid.symbol((x, y)), "{at}");
                assert_eq!(
                    (b.fg, b.bg, b.modifier),
                    (g.fg(), g.bg(), g.modifier()),
                    "{at}"
                );
            }
        }
    }

    trait Canvas {
        fn stringn(&mut self, x: u16, y: u16, s: &str, max: usize, style: Style) -> (u16, u16);
        fn style(&mut self, area: Rect, style: Style);
    }

    impl Canvas for Buffer {
        fn stringn(&mut self, x: u16, y: u16, s: &str, max: usize, style: Style) -> (u16, u16) {
            self.set_stringn(x, y, s, max, style)
        }
        fn style(&mut self, area: Rect, style: Style) {
            self.set_style(area, style);
        }
    }

    impl Canvas for Grid {
        fn stringn(&mut self, x: u16, y: u16, s: &str, max: usize, style: Style) -> (u16, u16) {
            self.set_stringn(x, y, s, max, style)
        }
        fn style(&mut self, area: Rect, style: Style) {
            self.set_style(area, style);
        }
    }

    #[test]
    fn strings_and_styles_paint_as_on_a_ratatui_buffer() {
        same_as_ratatui(|c| {
            c.style(Rect::new(0, 0, 12, 3), Style::new().bg(Color::Rgb(1, 2, 3)));
            let end = c.stringn(1, 0, "ab\tc\u{301}d", 20, Style::new().red().bold());
            assert_eq!(end, (5, 0));
            // wide graphemes, one that doesn't fit, and a zero-width joiner sequence
            c.stringn(0, 1, "日本語x", 5, Style::new().fg(Color::Indexed(3)));
            c.stringn(6, 1, "👩‍🔬e", 6, Style::new().underline_color(Color::Blue));
            c.stringn(0, 2, "ﾊﾞ…›│", 12, Style::new().not_bold().italic());
            c.style(Rect::new(2, 0, 3, 3), Style::new().reversed().not_italic());
        });
    }

    #[test]
    fn colors_survive_packing() {
        let colors =
            NAMED
                .into_iter()
                .chain([Color::Reset, Color::Indexed(200), Color::Rgb(9, 8, 7)]);
        for c in colors {
            assert_eq!(unpack(pack(c)), c);
        }
    }
}
