//! ada's own frame, in place of ratatui's `Buffer`: a grid of small `Copy` cells that [`ui`] paints
//! and [`Screen`] diffs and writes to the terminal. Painting keeps ratatui's semantics (a style
//! patches the cell's, a wide grapheme blanks the cells it covers, graphemes with control
//! characters are dropped), so the frames are the same; what changes is the cost per cell.
//!
//! [`ui`]: crate::ui

mod screen;

pub use screen::Screen;

use std::ops::{Index, IndexMut, Range};

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// `Cell::len` when the symbol doesn't fit in `sym`, which then holds its index in `Grid::long`.
const LONG: u8 = u8::MAX;
/// `Cell::len` for a kitty image's placeholder: `sym` holds its row and column in the image, two
/// bytes each, and the encoder writes the diacritics that number them.
const PLACED: u8 = LONG - 1;
/// A placeholder's symbol, as it reads: kitty's placeholder character, without its diacritics.
const PLACEHOLDER: &str = "\u{10EEEE}";
/// `Cell::meta`: the low four bits are the width in columns.
const WIDTH: u8 = 0b1111;
/// The terminal leaves the cell alone: an image covers it.
const SKIP: u8 = 1 << 4;
/// The symbol is written as is and counts as `WIDTH` columns: an image's escape sequence.
const FORCED: u8 = 1 << 5;

/// One terminal cell: 16 bytes, `Copy`, compared as plain bytes. There is no underline color: an
/// underline takes the foreground's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
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

    /// The cell as one number, in the order of its bytes, so comparing two is one comparison.
    /// Equal numbers are the same cell unless the symbol is long: its index may point at
    /// different text in another grid.
    fn bits(&self) -> u128 {
        u128::from(u32::from_le_bytes(self.sym))
            | u128::from(self.len) << 32
            | u128::from(self.meta) << 40
            | u128::from(self.modifier) << 48
            | u128::from(self.fg) << 64
            | u128::from(self.bg) << 96
    }

    /// Columns the symbol covers.
    pub fn width(&self) -> u16 {
        u16::from(self.meta & WIDTH)
    }

    fn skip(&self) -> bool {
        self.meta & SKIP != 0
    }

    fn forced(&self) -> bool {
        self.meta & FORCED != 0
    }

    /// Part of an image: the terminal draws it, or leaves it alone, by the protocol's rules.
    pub fn is_image(&self) -> bool {
        self.meta & (SKIP | FORCED) != 0
    }

    /// A kitty placeholder's row and column in its image.
    fn placed(&self) -> Option<(u16, u16)> {
        let [r0, r1, c0, c1] = self.sym;
        (self.len == PLACED).then_some((u16::from_le_bytes([r0, r1]), u16::from_le_bytes([c0, c1])))
    }
}

/// A style packed for patching cells: each field is kept where `keep` has bits and set from `set`,
/// so patching a cell is three masks with no branch. A style's colors are packed once, not per cell.
#[derive(Clone, Copy)]
struct Patch {
    keep: (u16, u32, u32),
    set: (u16, u32, u32),
}

impl Patch {
    fn new(style: Style) -> Patch {
        let color = |c: Option<Color>| c.map_or((u32::MAX, 0), |c| (0, pack(c)));
        let (keep_fg, fg) = color(style.fg);
        let (keep_bg, bg) = color(style.bg);
        let sub = style.sub_modifier.bits();
        Patch {
            keep: (!sub, keep_fg, keep_bg),
            set: (style.add_modifier.bits() & !sub, fg, bg),
        }
    }

    /// Patch the style over `c`'s: only what the style sets changes.
    fn apply(self, c: &mut Cell) {
        c.modifier = c.modifier & self.keep.0 | self.set.0;
        c.fg = c.fg & self.keep.1 | self.set.1;
        c.bg = c.bg & self.keep.2 | self.set.2;
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
        fill(&mut self.cells, Cell::BLANK);
        self.long.clear();
        self.scrolled = None;
        self.images = false;
    }

    /// Every cell blank in `style`: on a blank grid, the same as patching `style` over all of it,
    /// without reading a cell.
    pub fn fill(&mut self, style: Style) {
        let mut blank = Cell::BLANK;
        Patch::new(style).apply(&mut blank);
        fill(&mut self.cells, blank);
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
        } else if c.len == PLACED {
            PLACEHOLDER
        } else {
            std::str::from_utf8(&c.sym[..usize::from(c.len)]).unwrap_or(" ")
        }
    }

    /// The symbol's bytes, for the encoder.
    fn symbol_bytes(&self, i: usize) -> &[u8] {
        let c = &self.cells[i];
        if c.len == LONG {
            self.long[u32::from_le_bytes(c.sym) as usize].as_bytes()
        } else if c.len == PLACED {
            PLACEHOLDER.as_bytes()
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
        self.split_wide(row, x);
        let patch = Patch::new(style);
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            // A run of printable ASCII, one cell a byte; the last byte stays out when a non-ASCII
            // character follows it, which may be a combining mark that joins it.
            let mut run = bytes[i..]
                .iter()
                .take_while(|b| matches!(b, b' '..=b'~'))
                .count();
            if run > 0 && bytes.get(i + run).is_some_and(|b| !b.is_ascii()) {
                run -= 1;
            }
            if run > 0 {
                let n = run.min(usize::from(remaining));
                let at = row + usize::from(x - self.area.x);
                // `put` for one byte
                for (c, &b) in self.cells[at..at + n].iter_mut().zip(&bytes[i..i + n]) {
                    c.sym = [b, 0, 0, 0];
                    c.len = 1;
                    if !c.forced() {
                        c.meta = (c.meta & SKIP) | 1;
                    }
                    patch.apply(c);
                }
                // `n` is at most `remaining`, a u16
                (x, remaining) = (x + n as u16, remaining - n as u16);
                if n < run {
                    break;
                }
                i += n;
                continue;
            }
            let b = bytes[i];
            if b.is_ascii_control() {
                i += 1;
                continue;
            }
            let (g, width) = match lone(&s[i..]) {
                Some(len) => (&s[i..i + len], 1),
                None => {
                    let g = s[i..].graphemes(true).next().unwrap_or(&s[i..]);
                    (g, cell_width(g))
                }
            };
            i += g.len();
            if width == 0 || g.contains(char::is_control) {
                continue;
            }
            let Some(rest) = remaining.checked_sub(width) else {
                break;
            };
            remaining = rest;
            let at = row + usize::from(x - self.area.x);
            self.put(at, g, width);
            patch.apply(&mut self.cells[at]);
            // the cells a wide grapheme covers are blank
            for c in &mut self.cells[at + 1..at + usize::from(width)] {
                *c = Cell::BLANK;
            }
            x += width;
        }
        (x, y)
    }

    /// A write from column `x` of the row starting at `row` that lands inside a wide grapheme,
    /// such as a popup's border over CJK text: the grapheme can't show whole, so it becomes a
    /// blank with its style, as in a terminal. Kept, the encoder would write the grapheme and
    /// skip the columns it covers, and with them what was written there (F-04).
    fn split_wide(&mut self, row: usize, x: u16) {
        let x = x - self.area.x;
        for d in 1..=x.min(u16::from(WIDTH)) {
            let c = &mut self.cells[row + usize::from(x - d)];
            if c.forced() || c.skip() {
                return;
            }
            let width = c.width();
            if width > d {
                c.sym = [b' ', 0, 0, 0];
                c.len = 1;
                c.meta = 1;
                return;
            }
            // only blanks may be the columns of a wide grapheme further left
            if width > 1 || c.len != 1 || c.sym[0] != b' ' {
                return;
            }
        }
    }

    pub fn set_string(&mut self, x: u16, y: u16, string: impl AsRef<str>, style: Style) {
        self.set_stringn(x, y, string, usize::MAX, style);
    }

    /// Patch `style` over every cell of `area` within the grid.
    pub fn set_style(&mut self, area: Rect, style: Style) {
        let area = self.area.intersection(area);
        let patch = Patch::new(style);
        for y in area.top()..area.bottom() {
            let row = self.index(area.x, y);
            for c in &mut self.cells[row..row + usize::from(area.width)] {
                patch.apply(c);
            }
        }
    }

    /// Make cell `(x, y)` the one at `row` and `col` of the kitty image that `color` names: a
    /// placeholder one column wide, on the cell's background.
    pub fn placeholder(&mut self, (x, y): (u16, u16), (row, col): (u16, u16), color: Color) {
        let i = self.index(x, y);
        let ([r0, r1], [c0, c1]) = (row.to_le_bytes(), col.to_le_bytes());
        let c = &mut self.cells[i];
        c.sym = [r0, r1, c0, c1];
        c.len = PLACED;
        c.meta = FORCED | 1;
        c.fg = pack(color);
        self.images = true;
    }

    /// Make cell `(x, y)` an image's, which `symbol` draws as it is in one column, in the
    /// foreground `fg`: the first cell of a kitty image's first frame, which carries its
    /// transmission before its placeholder.
    pub fn image_cell(&mut self, (x, y): (u16, u16), symbol: &str, fg: Color) {
        let i = self.index(x, y);
        self.cells[i].meta = 0;
        self.put(i, symbol, 1);
        let c = &mut self.cells[i];
        c.meta = FORCED | 1;
        c.fg = pack(fg);
        self.images = true;
    }

    /// Cover `area` with an image that `seq` draws from its first cell: a sixel or iTerm2 image,
    /// whose escape sequence is written as is there, in the cell's style, and moves the cursor
    /// its own way. The terminal leaves the rest of the area alone, as the image covers it.
    pub fn image(&mut self, area: Rect, seq: &str) {
        let area = self.area.intersection(area);
        if area.is_empty() {
            return;
        }
        for y in area.top()..area.bottom() {
            let row = self.index(area.x, y);
            for c in &mut self.cells[row..row + usize::from(area.width)] {
                c.meta = c.meta & WIDTH | SKIP;
            }
        }
        let first = self.index(area.x, area.y);
        self.cells[first].meta = 0;
        self.put(first, seq, 1);
        self.cells[first].meta = FORCED | 1;
        self.images = true;
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

/// Set every cell to `cell` by copying the cells already set, doubling each time: `<[T]>::fill`
/// stores a cell's five fields one by one, and a copy moves many cells a store.
fn fill(cells: &mut [Cell], cell: Cell) {
    let Some(first) = cells.first_mut() else {
        return;
    };
    *first = cell;
    let mut done = 1;
    while done < cells.len() {
        let n = done.min(cells.len() - done);
        cells.copy_within(..n, done);
        done += n;
    }
}

/// Characters that are graphemes of their own next to ASCII or each other: Latin-1 but the soft
/// hyphen, dashes, bullets and quotes, arrows, box drawing and shapes. None of them is a mark, a
/// joiner, a prepended character, a control, Hangul or a regional indicator, the only kinds that
/// join a neighbor.
fn loner(c: char) -> bool {
    matches!(c,
        '\u{A0}'..='\u{AC}'
        | '\u{AE}'..='\u{FF}'
        | '\u{2010}'..='\u{2027}'
        | '\u{2030}'..='\u{205E}'
        | '\u{2190}'..='\u{21FF}'
        | '\u{2500}'..='\u{25FF}')
}

/// The length of the character `s` starts with, when it is a grapheme one column wide by
/// itself: a [`loner`] that ASCII, another loner or nothing follows. Most box drawing, dashes and
/// arrows skip grapheme segmentation this way.
fn lone(s: &str) -> Option<usize> {
    let mut chars = s.chars();
    let c = chars.next()?;
    let alone = loner(c) && chars.next().is_none_or(|d| d.is_ascii() || loner(d));
    (alone && c.width() == Some(1)).then(|| c.len_utf8())
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

    use ratatui::buffer::Buffer;

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

    /// Every character that can skip grapheme segmentation, before what may or may not join it.
    #[test]
    fn lone_characters_paint_as_on_a_ratatui_buffer() {
        let next = [
            "",
            "a",
            " ",
            "\t",
            "─",
            "é",
            "\u{301}",
            "\u{FE0F}",
            "\u{200D}",
            "\u{903}",
            "\u{AD}",
            "日",
            "\u{1F1E6}",
            "\u{1100}",
            "ﾞ",
        ];
        let loners = (0xA0..0x100)
            .chain(0x2010..0x2060)
            .chain(0x2190..0x2200)
            .chain(0x2500..0x2600)
            .filter_map(char::from_u32);
        for c in loners {
            for d in next {
                same_as_ratatui(|g| {
                    g.stringn(0, 0, &format!("{c}{d}{c}x"), 12, Style::new().red());
                    g.stringn(0, 1, &format!("ab{c}{d}"), 12, Style::new());
                });
            }
        }
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

    /// Writing into the right half of a wide grapheme blanks the grapheme, keeping its style,
    /// so what was written there shows (F-04).
    #[test]
    fn a_write_inside_a_wide_grapheme_blanks_it() {
        let mut grid = Grid::empty(Rect::new(0, 0, 6, 1));
        let red = Style::new().bg(Color::Red);
        grid.set_string(0, 0, "日本x", red);
        grid.set_string(3, 0, "\u{2502}", Style::new());
        let row: Vec<_> = (0..6).map(|x| grid.symbol((x, 0))).collect();
        assert_eq!(row, ["日", " ", " ", "\u{2502}", "x", " "]);
        assert_eq!((grid[(2, 0)].width(), grid[(2, 0)].bg()), (1, Color::Red));
        // a write that starts where one ends leaves it alone
        grid.set_string(2, 0, "ab", Style::new());
        assert_eq!(grid.symbol((0, 0)), "日");
    }
}
