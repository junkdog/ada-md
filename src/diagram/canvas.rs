//! The grid a diagram is drawn on. A cell holds text or line bits; lines drawn over each other
//! merge their bits, and the glyphs are picked at the end, so a frame crossing a lifeline
//! becomes `┼` and an arrow leaving one `├` without either knowing about the other.

use unicode_width::UnicodeWidthChar;

use crate::theme::StyleKey;

const UP: u8 = 1;
const DOWN: u8 = 1 << 1;
const UP_DOWN: u8 = UP | DOWN;
const LEFT: u8 = 1 << 2;
const RIGHT: u8 = 1 << 3;
const DASHED: u8 = 1 << 4;
const ROUND: u8 = 1 << 5;
/// Vertical lines drawn heavy: an active lifeline.
const HEAVY: u8 = 1 << 6;
/// Every line in the cell heavy: a flowchart's thick edge (`==>`).
const THICK: u8 = 1 << 7;

/// How a line is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pen {
    pub dashed: bool,
    /// Rounded corners.
    pub round: bool,
    pub thick: bool,
}

impl Pen {
    pub const SOLID: Pen = Pen {
        dashed: false,
        round: false,
        thick: false,
    };
    pub const DASHED: Pen = Pen {
        dashed: true,
        round: false,
        thick: false,
    };
    pub const ROUND: Pen = Pen {
        dashed: false,
        round: true,
        thick: false,
    };
    pub const THICK: Pen = Pen {
        dashed: false,
        round: false,
        thick: true,
    };

    fn flags(self) -> u8 {
        (if self.dashed { DASHED } else { 0 })
            | (if self.round { ROUND } else { 0 })
            | if self.thick { THICK } else { 0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cell {
    Empty,
    Text(char, StyleKey),
    /// The columns after the first of a wide character (two for most, three for a few, such as
    /// U+17D8).
    Wide,
    Lines(u8, StyleKey),
}

#[derive(Debug)]
pub struct Canvas {
    width: usize,
    cells: Vec<Cell>,
    ascii: bool,
}

impl Canvas {
    pub fn new(width: usize, ascii: bool) -> Self {
        Self {
            width: width.max(1),
            cells: Vec::new(),
            ascii,
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.cells.len() / self.width
    }

    pub fn cells(&self) -> usize {
        self.cells.len()
    }

    /// Make sure rows `..=row` exist.
    fn reach(&mut self, row: usize) {
        let need = (row + 1) * self.width;
        if self.cells.len() < need {
            self.cells.resize(need, Cell::Empty);
        }
    }

    /// Add blank rows up to `rows`.
    pub fn grow(&mut self, rows: usize) {
        if rows > 0 {
            self.reach(rows - 1);
        }
    }

    fn at(&mut self, row: usize, col: usize) -> Option<&mut Cell> {
        if col >= self.width {
            return None;
        }
        self.reach(row);
        self.cells.get_mut(row * self.width + col)
    }

    fn bits(&mut self, row: usize, col: usize, bits: u8, key: StyleKey) {
        if let Some(cell) = self.at(row, col) {
            *cell = match *cell {
                Cell::Lines(b, k) => Cell::Lines(b | bits, k),
                Cell::Empty => Cell::Lines(bits, key),
                text => text,
            };
        }
    }

    /// A horizontal line over columns `from..=to` of `row`.
    pub fn hline(&mut self, row: usize, from: usize, to: usize, pen: Pen, key: StyleKey) {
        let (a, b) = (from.min(to), from.max(to));
        for col in a..=b {
            let mut bits = pen.flags();
            if col > a {
                bits |= LEFT;
            }
            if col < b {
                bits |= RIGHT;
            }
            self.bits(row, col, bits, key);
        }
    }

    /// A vertical line over rows `from..=to` of `col`.
    pub fn vline(&mut self, col: usize, from: usize, to: usize, pen: Pen, key: StyleKey) {
        let (a, b) = (from.min(to), from.max(to));
        for row in a..=b {
            let mut bits = pen.flags();
            if row > a {
                bits |= UP;
            }
            if row < b {
                bits |= DOWN;
            }
            self.bits(row, col, bits, key);
        }
    }

    /// A box with corners at `(top, left)` and `(bottom, right)`.
    pub fn rect(
        &mut self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
        pen: Pen,
        key: StyleKey,
    ) {
        self.hline(top, left, right, pen, key);
        self.hline(bottom, left, right, pen, key);
        self.vline(left, top, bottom, pen, key);
        self.vline(right, top, bottom, pen, key);
    }

    /// Draw the lines in column `col` heavy over rows `from..=to`: an active lifeline.
    pub fn thicken(&mut self, col: usize, from: usize, to: usize) {
        for row in from..=to {
            if let Some(Cell::Lines(bits, _)) = self.at(row, col) {
                *bits |= HEAVY;
            }
        }
    }

    /// Join a line end at `(row, col)` to the line below it, as a `┬` on a box's bottom edge.
    pub fn join_down(&mut self, row: usize, col: usize, key: StyleKey) {
        self.bits(row, col, DOWN, key);
    }

    /// Extend the line at `(row, col)` one way, as a corner reaching up into a label above it:
    /// `(up, down, left, right)`. Nothing happens where there is no line.
    pub fn extend(&mut self, row: usize, col: usize, way: (bool, bool, bool, bool)) {
        let bits = (if way.0 { UP } else { 0 })
            | (if way.1 { DOWN } else { 0 })
            | (if way.2 { LEFT } else { 0 })
            | if way.3 { RIGHT } else { 0 };
        if let Some(Cell::Lines(b, _)) = self.at(row, col) {
            *b |= bits;
        }
    }

    /// Write `text` from `col`, over whatever is there; it stops at the right edge. Returns the
    /// column after it.
    pub fn text(&mut self, row: usize, col: usize, text: &str, key: StyleKey) -> usize {
        self.reach(row);
        let base = row * self.width;
        let mut col = col;
        for c in text.chars() {
            let w = c.width().unwrap_or(0);
            if w == 0 {
                continue;
            }
            if col + w > self.width {
                break;
            }
            for k in 0..w {
                self.clear(base, col + k);
            }
            self.cells[base + col] = Cell::Text(c, key);
            for k in 1..w {
                self.cells[base + col + k] = Cell::Wide;
            }
            col += w;
        }
        col
    }

    /// Empty the character the cell at `base + col` is part of, every column of it, when it is
    /// wider than one: writing over part of one must not leave the rest, or the row would come
    /// out wider than the canvas. `base` is the row's first cell, and the row exists.
    fn clear(&mut self, base: usize, col: usize) {
        let row = &mut self.cells[base..base + self.width];
        let mut start = col;
        while start > 0 && row[start] == Cell::Wide {
            start -= 1;
        }
        let wide = matches!(row[start], Cell::Text(c, _) if c.width().unwrap_or(0) > 1);
        if !wide && start == col {
            return;
        }
        row[start] = Cell::Empty;
        for cell in row[start + 1..]
            .iter_mut()
            .take_while(|c| **c == Cell::Wide)
        {
            *cell = Cell::Empty;
        }
    }

    /// `c` in every column of `from..to` on `row`, as [`text`](Self::text) would write it: a
    /// gantt bar.
    pub fn fill(&mut self, row: usize, from: usize, to: usize, c: char, key: StyleKey) {
        if c.width() != Some(1) {
            let mut buf = [0; 4];
            let s = c.encode_utf8(&mut buf);
            let mut col = from;
            while col < to {
                let next = self.text(row, col, s, key);
                if next == col {
                    break;
                }
                col = next;
            }
            return;
        }
        self.reach(row);
        let base = row * self.width;
        for col in from..to.min(self.width) {
            self.clear(base, col);
            self.cells[base + col] = Cell::Text(c, key);
        }
    }

    /// One glyph, as text.
    pub fn glyph(&mut self, row: usize, col: usize, c: char, key: StyleKey) {
        let mut buf = [0; 4];
        self.text(row, col, c.encode_utf8(&mut buf), key);
    }

    /// Row `row` as pieces of one style each, trailing blanks left out. `blank` styles the
    /// gaps.
    pub fn pieces(
        &self,
        row: usize,
        blank: StyleKey,
        emit: &mut impl FnMut(&str, usize, StyleKey),
    ) {
        let cells = &self.cells[row * self.width..(row + 1) * self.width];
        let end = cells
            .iter()
            .rposition(|c| *c != Cell::Empty)
            .map_or(0, |i| i + 1);
        let mut buf = String::new();
        let mut cols = 0;
        let mut key = blank;
        for cell in &cells[..end] {
            let (c, k) = match *cell {
                Cell::Empty => (' ', blank),
                Cell::Text(c, k) => (c, k),
                Cell::Wide => continue,
                Cell::Lines(bits, k) => (glyph(bits, self.ascii), k),
            };
            if k != key && !buf.is_empty() {
                emit(&buf, cols, key);
                buf.clear();
                cols = 0;
            }
            key = k;
            buf.push(c);
            cols += c.width().unwrap_or(1);
        }
        if !buf.is_empty() {
            emit(&buf, cols, key);
        }
    }

    /// Row `row` as plain text, for tests.
    pub fn row_text(&self, row: usize) -> String {
        let mut out = String::new();
        self.pieces(row, StyleKey::DiagramLine, &mut |t, _, _| out.push_str(t));
        out
    }
}

/// The glyph for a heavy vertical line with light lines joining it.
fn heavy(dirs: u8, ascii: bool) -> char {
    let (v, h) = (dirs & (UP | DOWN), dirs & (LEFT | RIGHT));
    if ascii {
        return if h == 0 { '#' } else { '+' };
    }
    match (v, h) {
        (_, 0) => '┃',
        (UP_DOWN, RIGHT) => '┝',
        (UP_DOWN, LEFT) => '┥',
        (UP_DOWN, _) => '┿',
        (DOWN, RIGHT) => '┎',
        (DOWN, LEFT) => '┒',
        (DOWN, _) => '┰',
        (_, RIGHT) => '┖',
        (_, LEFT) => '┚',
        _ => '┸',
    }
}

/// The glyph for a cell whose lines are all heavy.
fn thick(dirs: u8, ascii: bool) -> char {
    let vertical = dirs & !(UP | DOWN) == 0;
    let horizontal = dirs & !(LEFT | RIGHT) == 0;
    if ascii {
        return match () {
            _ if vertical => '|',
            _ if horizontal => '=',
            _ => '+',
        };
    }
    match dirs {
        _ if vertical => '┃',
        _ if horizontal => '━',
        d if d == DOWN | RIGHT => '┏',
        d if d == DOWN | LEFT => '┓',
        d if d == UP | RIGHT => '┗',
        d if d == UP | LEFT => '┛',
        d if d == UP | DOWN | RIGHT => '┣',
        d if d == UP | DOWN | LEFT => '┫',
        d if d == DOWN | LEFT | RIGHT => '┳',
        d if d == UP | LEFT | RIGHT => '┻',
        _ => '╋',
    }
}

/// The glyph for a cell's line bits.
fn glyph(bits: u8, ascii: bool) -> char {
    let dashed = bits & DASHED != 0;
    let round = bits & ROUND != 0;
    let dirs = bits & (UP | DOWN | LEFT | RIGHT);
    let vertical = dirs & !(UP | DOWN) == 0;
    let horizontal = dirs & !(LEFT | RIGHT) == 0;
    if bits & HEAVY != 0 && dirs & (UP | DOWN) != 0 {
        return heavy(dirs, ascii);
    }
    if bits & THICK != 0 && dirs != 0 {
        return thick(dirs, ascii);
    }
    if ascii {
        return match () {
            _ if dirs == 0 => ' ',
            _ if vertical => {
                if dashed {
                    ':'
                } else {
                    '|'
                }
            }
            _ if horizontal => {
                if dashed {
                    '.'
                } else {
                    '-'
                }
            }
            _ => '+',
        };
    }
    match dirs {
        0 => ' ',
        _ if vertical => {
            if dashed {
                '┆'
            } else {
                '│'
            }
        }
        _ if horizontal => {
            if dashed {
                '┄'
            } else {
                '─'
            }
        }
        d if d == DOWN | RIGHT => {
            if round {
                '╭'
            } else {
                '┌'
            }
        }
        d if d == DOWN | LEFT => {
            if round {
                '╮'
            } else {
                '┐'
            }
        }
        d if d == UP | RIGHT => {
            if round {
                '╰'
            } else {
                '└'
            }
        }
        d if d == UP | LEFT => {
            if round {
                '╯'
            } else {
                '┘'
            }
        }
        d if d == UP | DOWN | RIGHT => '├',
        d if d == UP | DOWN | LEFT => '┤',
        d if d == DOWN | LEFT | RIGHT => '┬',
        d if d == UP | LEFT | RIGHT => '┴',
        _ => '┼',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const K: StyleKey = StyleKey::DiagramLine;

    fn rows(c: &Canvas) -> Vec<String> {
        (0..c.height()).map(|r| c.row_text(r)).collect()
    }

    #[test]
    fn crossing_lines_join() {
        let mut c = Canvas::new(5, false);
        c.rect(0, 0, 2, 4, Pen::SOLID, K);
        c.vline(2, 0, 2, Pen::SOLID, K);
        c.hline(1, 0, 4, Pen::SOLID, K);
        assert_eq!(rows(&c), ["┌─┬─┐", "├─┼─┤", "└─┴─┘"]);
    }

    #[test]
    fn heavy_lifelines_take_light_joins() {
        let mut c = Canvas::new(3, false);
        c.vline(1, 0, 3, Pen::SOLID, K);
        c.hline(1, 1, 2, Pen::SOLID, K);
        c.hline(2, 0, 2, Pen::SOLID, K);
        c.thicken(1, 1, 3);
        assert_eq!(rows(&c), [" │", " ┝─", "─┿─", " ┃"]);
        let mut c = Canvas::new(1, true);
        c.vline(0, 0, 1, Pen::SOLID, K);
        c.thicken(0, 0, 1);
        assert_eq!(rows(&c), ["#", "#"]);
    }

    #[test]
    fn rounded_dashed_and_ascii() {
        let mut c = Canvas::new(4, false);
        c.rect(0, 0, 1, 3, Pen::ROUND, K);
        assert_eq!(rows(&c), ["╭──╮", "╰──╯"]);
        let mut c = Canvas::new(3, false);
        c.hline(0, 0, 2, Pen::DASHED, K);
        assert_eq!(rows(&c), ["┄┄┄"]);
        let mut c = Canvas::new(3, true);
        c.rect(0, 0, 1, 2, Pen::SOLID, K);
        assert_eq!(rows(&c), ["+-+", "+-+"]);
    }

    #[test]
    fn text_wins_and_wide_characters_take_two_cells() {
        let mut c = Canvas::new(6, false);
        c.hline(0, 0, 5, Pen::SOLID, K);
        assert_eq!(c.text(0, 1, "日x", StyleKey::DiagramLabel), 4);
        c.hline(0, 0, 5, Pen::SOLID, K);
        assert_eq!(rows(&c), ["─日x──"]);
        // past the edge is dropped
        assert_eq!(c.text(1, 4, "abc", K), 6);
        assert_eq!(c.row_text(1), "    ab");
    }

    #[test]
    fn writing_over_half_a_wide_character_clears_the_other_half() {
        let mut c = Canvas::new(4, false);
        c.text(0, 0, "日本", K);
        c.text(0, 1, "x", K);
        assert_eq!(c.row_text(0), " x本");
        c.text(0, 2, "y", K);
        assert_eq!(c.row_text(0), " xy");
        c.text(0, 0, "語", K);
        assert_eq!(c.row_text(0), "語y");
        // unicode-width counts a few characters as three columns
        let mut c = Canvas::new(5, false);
        assert_eq!(c.text(0, 0, "\u{17D8}ab", K), 5);
        c.text(0, 1, "x", K);
        assert_eq!(c.row_text(0), " x ab");
        // a fill over half of one clears the rest of it
        let mut c = Canvas::new(5, false);
        c.text(0, 1, "日", K);
        c.fill(0, 0, 2, '█', K);
        assert_eq!(c.row_text(0), "██");
        c.fill(0, 3, 9, '本', K);
        assert_eq!(c.row_text(0), "██ 本");
    }

    #[test]
    fn pieces_split_by_style_and_drop_trailing_blanks() {
        let mut c = Canvas::new(8, false);
        c.text(0, 0, "ab", StyleKey::DiagramLabel);
        c.hline(0, 3, 4, Pen::SOLID, K);
        let mut got = Vec::new();
        c.pieces(0, K, &mut |t, w, k| got.push((t.to_owned(), w, k)));
        assert_eq!(
            got,
            [
                ("ab".to_owned(), 2, StyleKey::DiagramLabel),
                (" ──".to_owned(), 3, K)
            ]
        );
    }
}
