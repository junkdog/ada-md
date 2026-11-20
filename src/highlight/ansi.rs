//! Text that keeps its own colors (FEAT-022): piped output colored by the program that wrote it,
//! `git log --graph --format='%C(…)…'`, `ls --color`, `grep --color`, a logger's. Its SGR
//! sequences (`ESC [ … m`) are read here as a lexer reads code, a line at a time, the pen they
//! set carried from line to line as a terminal carries it. Each style the text uses is one
//! entry of its block's palette, and a run's rule names its entry ([`palette_rule`]), past any
//! theme's rules; the text the document shows is the input without its escapes
//! ([`crate::stdin::strip`]), the runs placed on it as tabs expand it, so they agree with the
//! layout's bytes.
//!
//! A line shows as a terminal shows it (PGR-007): a `\r` in it, a backspace or a move along the
//! line (`ESC [ C`, `D`, `G`) writes over what's there, as a progress line's updates do, and
//! `ESC [ K` erases; a character struck over itself is bold and one struck over `_` underlined,
//! as `man` writes them without colors ([`crate::stdin::has_overstrike`]). A line whose
//! `ESC [ K` comes with a background set (delta's) fills its row with it ([`FILL`]).
//!
//! Whether piped text keeps its colors at all is [`keeps_own_colors`]: a diff gets ada's, its
//! lines in their file's language, unless its colors are the change itself (`--word-diff=color`).

use std::borrow::Cow;
use std::collections::HashMap;

use ada_parse::{Rule, diff_hunk};
use unicode_width::UnicodeWidthChar;

use super::OpenLine;
use crate::layout::TAB_STOP;
use crate::stdin::{Strip, has_overstrike, strip};
use crate::style::{Color, Kind, Modifier, Style};

/// The rule of palette entry 0; entry `i` is `PALETTE + i`. Past any theme's rules, below the
/// tints' ([`super::Tint`]).
const PALETTE: u16 = 0x8000;

/// The most styles a block's palette holds; text in a style past them shows plain. Past
/// [`EXACT`] styles, a new one's colors are taken to the nearest of 256, past [`NEAR`] to the
/// nearest of 16, so that truecolor art keeps its colors near enough (PGR-007).
const PALETTE_MAX: usize = 0x6000;
const EXACT: usize = 0x5000;
const NEAR: usize = 0x5c00;

/// The rule of a line's fill (PGR-007), the row's background past its text: the line's last two
/// runs, both empty, this rule's and then the fill's palette entry's, after the text's runs, so
/// that no run of its text takes the entry's run over.
pub(super) const FILL: u16 = Rule::UNSET - 6;

/// The style of `rule`, if it is one of `palette`'s entries.
pub fn palette_style(rule: Rule, palette: &[Style]) -> Option<Style> {
    let i = rule.0.checked_sub(PALETTE)?;
    palette.get(usize::from(i)).copied()
}

/// The rule of palette entry `i`.
fn palette_rule(i: usize) -> Rule {
    Rule(PALETTE + i as u16)
}

/// The palette's hasher: a style is a few small numbers, looked up at every pen change, so a
/// multiply and a rotate a number (FxHash's) beat SipHash, which was 5–6% of a colored log's
/// print (PGR-007). No defense against crafted collisions is needed: a palette holds at most
/// [`PALETTE_MAX`] styles.
#[derive(Clone, Copy, Debug, Default)]
struct Styles;

impl std::hash::BuildHasher for Styles {
    type Hasher = StyleHasher;

    fn build_hasher(&self) -> StyleHasher {
        StyleHasher(0)
    }
}

struct StyleHasher(u64);

impl StyleHasher {
    fn add(&mut self, n: u64) {
        const K: u64 = 0x51_7c_c1_b7_27_22_0a_95;
        self.0 = (self.0.rotate_left(5) ^ n).wrapping_mul(K);
    }
}

impl std::hash::Hasher for StyleHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.add(u64::from(b));
        }
    }

    fn write_u8(&mut self, n: u8) {
        self.add(u64::from(n));
    }

    fn write_u16(&mut self, n: u16) {
        self.add(u64::from(n));
    }

    fn write_u32(&mut self, n: u32) {
        self.add(u64::from(n));
    }

    fn write_u64(&mut self, n: u64) {
        self.add(n);
    }

    fn write_usize(&mut self, n: usize) {
        self.add(n as u64);
    }

    fn write_isize(&mut self, n: isize) {
        self.add(n as u64);
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// Reads a block's lines' SGR sequences into runs, carrying the pen from line to line.
#[derive(Debug, Default)]
pub(super) struct Ansi {
    /// What the sequences so far set: what a run takes over the code's text style.
    pen: Style,
    palette: Vec<Style>,
    index: HashMap<Style, u16, Styles>,
    /// The line being read, as it shows, and the rules of its pen's changes.
    shown: Shown,
    rules: Vec<(usize, Rule)>,
}

impl Ansi {
    pub(super) fn palette(&self) -> &[Style] {
        &self.palette
    }

    /// The rule of `style`: none for one that sets nothing, else its palette entry, or, once
    /// the palette is past [`EXACT`], a nearer one's.
    fn rule(&mut self, style: Style) -> Rule {
        if style == Style::new() {
            return Rule(Rule::NONE);
        }
        if let Some(&i) = self.index.get(&style) {
            return palette_rule(usize::from(i));
        }
        let mut style = style;
        for (limit, colors) in [(EXACT, None), (NEAR, Some(256)), (PALETTE_MAX, Some(16))] {
            if let Some(n) = colors {
                style = nearest(style, n);
                if let Some(&i) = self.index.get(&style) {
                    return palette_rule(usize::from(i));
                }
            }
            let i = self.palette.len();
            if i < limit {
                self.palette.push(style);
                self.index.insert(style, i as u16);
                return palette_rule(i);
            }
        }
        Rule(Rule::NONE)
    }

    /// Read `line`, a line of the block as [`super::Lines`] splits it, into `out`'s runs: on the
    /// line as it shows ([`shown`]) and without a `\r` at its end, as tabs expand it.
    pub(super) fn line(&mut self, line: &str, mut out: OpenLine<'_>) {
        let mut cur = self.rule(self.pen);
        self.shown.read(line, &mut self.pen, true);
        let mut shown = std::mem::take(&mut self.shown);
        if shown.plain.last() == Some(&b'\r') {
            shown.plain.pop();
        }
        let mut marks = std::mem::take(&mut self.rules);
        marks.clear();
        for &(at, style) in &shown.marks {
            marks.push((at, self.rule(style)));
        }
        let fill = shown.fill.map(|bg| self.rule(Style::new().bg(bg)));
        // whole characters only went: a sequence starts and ends on ASCII bytes
        let plain = std::str::from_utf8(&shown.plain).expect("a line without escapes is UTF-8");
        // where the expanded line is, in bytes and in columns (`expand_tabs`' stops)
        let (mut x, mut col) = (0u32, 0usize);
        let mut last = 0;
        let mut changes = marks.iter().peekable();
        let mut change = |x: u32, rule: Rule, cur: &mut Rule, out: &mut OpenLine<'_>| {
            if rule != *cur {
                if x > last {
                    out.push(x, *cur);
                    last = x;
                }
                *cur = rule;
            }
        };
        for (i, c) in plain.char_indices() {
            while let Some(&&(at, rule)) = changes.peek()
                && at <= i
            {
                change(x, rule, &mut cur, &mut out);
                changes.next();
            }
            let (bytes, cols) = if c == '\t' {
                let n = TAB_STOP - col % TAB_STOP;
                (n, n)
            } else if c.is_control() {
                (1, 1)
            } else {
                (c.len_utf8(), c.width().unwrap_or(0))
            };
            x += bytes as u32;
            col += cols;
        }
        for &(_, rule) in changes {
            change(x, rule, &mut cur, &mut out);
        }
        if cur.0 != Rule::NONE && x > last {
            out.push(x, cur);
        }
        if let Some(fill) = fill.filter(|f| f.0 != Rule::NONE) {
            out.push(x, Rule(FILL));
            out.push(x, fill);
        }
        out.end();
        self.shown = shown;
        self.rules = marks;
    }
}

/// The text `colored`, a document's in its own colors, shows (PGR-007): each line as a terminal
/// shows it (`Shown`), its escapes gone; the same lines as `colored`'s, but for a last line of
/// escapes alone, which shows as none (`lines_of`).
pub fn shown(colored: &str) -> Cow<'_, str> {
    let colored = lines_of(colored);
    if memchr::memchr3(0x1b, b'\r', 0x08, colored.as_bytes()).is_none() {
        return Cow::Borrowed(colored);
    }
    // stripped whole, at once; unless a line may write over itself, which a line shows only
    // when it has a backspace, text after a `\r`, an erase after one, or a move: then each line
    // as [`Shown`] reads it, cell by cell where it does
    let mut bytes = colored.as_bytes().to_vec();
    let mut strip = Strip::default();
    let kept = strip.bytes(&mut bytes);
    bytes.truncate(kept);
    let over = strip.moved()
        || memchr::memchr(0x08, &bytes).is_some()
        || memchr::memchr_iter(b'\r', &bytes)
            .any(|i| strip.erased() || bytes.get(i + 1).is_some_and(|&c| c != b'\n'));
    if !over {
        // whole characters only went: a sequence starts and ends on ASCII bytes
        return Cow::Owned(String::from_utf8(bytes).expect("stripped text is UTF-8"));
    }
    drop(bytes);
    let (mut out, mut line) = (String::with_capacity(colored.len()), Shown::default());
    let mut pen = Style::new();
    let mut rest = colored;
    loop {
        let (text, next) = match rest.find('\n') {
            Some(i) => (&rest[..i], Some(&rest[i + 1..])),
            None => (rest, None),
        };
        if memchr::memchr3(0x1b, b'\r', 0x08, text.as_bytes()).is_none() {
            out.push_str(text);
        } else {
            line.read(text, &mut pen, false);
            // whole characters only went, or were written whole
            out.push_str(std::str::from_utf8(&line.plain).expect("a shown line is UTF-8"));
        }
        let Some(next) = next else {
            break;
        };
        out.push('\n');
        rest = next;
    }
    Cow::Owned(out)
}

/// `colored` without a last line that shows nothing (escapes after the last newline, PGR-008):
/// the text [`shown`] has a line for each of its lines, and its highlighting the same.
pub(super) fn lines_of(colored: &str) -> &str {
    let tail = colored.rfind('\n').map_or(0, |nl| nl + 1);
    let last = &colored[tail..];
    if last.is_empty() || !has_escape_or_moves(last) {
        return colored;
    }
    let mut shown = Shown::default();
    shown.read(last, &mut Style::new(), false);
    match shown.plain.is_empty() {
        true => &colored[..tail],
        false => colored,
    }
}

fn has_escape_or_moves(line: &str) -> bool {
    memchr::memchr2(0x1b, 0x08, line.as_bytes()).is_some()
}

/// A line as a terminal shows it: its text without its escapes, where its pen changes in it,
/// and the background its last `ESC [ K` fills the rest of its row with. Most lines are their
/// text with the escapes dropped; one that moves back over itself (a `\r` with text after it,
/// a backspace, `ESC [ D`, `ESC [ G`, `ESC [ 1 K`, `ESC [ 2 K`) or on (`ESC [ C`) is written
/// out cell by cell ([`Shown::render`]).
#[derive(Debug, Default)]
struct Shown {
    plain: Vec<u8>,
    marks: Vec<(usize, Style)>,
    fill: Option<Color>,
    /// [`Shown::render`]'s screen line, its cells' text in `arena`.
    cells: Vec<Cell>,
    arena: Vec<u8>,
}

/// A column of a rendered line: its text, `arena[at..at + len]`, and style; the second column
/// of a wide character has none. A blank is one no text was written to, padding or erased.
#[derive(Clone, Copy, Debug)]
struct Cell {
    at: u32,
    len: u8,
    style: Style,
    blank: bool,
}

const BLANK: Cell = Cell {
    at: 0,
    len: 0,
    style: Style::new(),
    blank: true,
};

impl Shown {
    /// Read `line` from `pen`, which it leaves where the line does; its pen's changes, its
    /// styles and its fill too when `styled`, else its text alone, its SGR sequences unread.
    fn read(&mut self, line: &str, pen: &mut Style, styled: bool) {
        let start = *pen;
        if !self.strip(line.as_bytes(), pen, styled) {
            *pen = start;
            self.render(line, pen, styled);
        }
    }

    /// `line` without its escapes, false when it moves back over itself or on, which only
    /// [`Shown::render`] reads.
    fn strip(&mut self, b: &[u8], pen: &mut Style, styled: bool) -> bool {
        self.plain.clear();
        self.marks.clear();
        self.fill = None;
        let mut strip = Strip::default();
        let mut params = 0;
        // a `\r` with nothing after it yet: CRLF's
        let mut cr = false;
        let mut i = 0;
        while i < b.len() {
            // text up to the next escape, `\r` or backspace, at once: short runs, between a
            // colored log's escapes, which a search call a run cost more than it saved
            if strip.in_text() {
                let mut next = i;
                while next < b.len() && !matches!(b[next], 0x1b | b'\r' | 0x08) {
                    next += 1;
                }
                if next > i {
                    if cr {
                        return false;
                    }
                    self.plain.extend_from_slice(&b[i..next]);
                    i = next;
                    continue;
                }
            }
            let (at, c) = (i, b[i]);
            i += 1;
            let was = strip.in_csi();
            if strip.byte(c) {
                match c {
                    0x08 => return false,
                    b'\r' => cr = true,
                    _ if cr => return false,
                    _ => {}
                }
                self.plain.push(c);
            } else if !was && strip.in_csi() {
                params = at + 1;
            } else if was && !strip.in_csi() {
                let p = &b[params..at];
                match (c, private(p)) {
                    (b'm', _) => {
                        if styled {
                            sgr(p, pen);
                            self.marks.push((self.plain.len(), *pen));
                        }
                    }
                    (b'K', false) if !cr && param(p, 0) == 0 => self.fill = pen.bg,
                    (b'K' | b'C' | b'D' | b'G' | b'`' | b'a', false) => return false,
                    _ => {}
                }
            }
        }
        true
    }

    /// `line` written cell by cell, as a terminal writes it to a row.
    fn render(&mut self, line: &str, pen: &mut Style, styled: bool) {
        self.cells.clear();
        self.arena.clear();
        self.fill = None;
        let b = line.as_bytes();
        let mut strip = Strip::default();
        let (mut col, mut params) = (0usize, 0);
        // after a backspace: the next character strikes over the one there
        let mut over = false;
        let mut cr_last = false;
        for (i, c) in line.char_indices() {
            let first = b[i];
            let was = strip.in_csi();
            let mut kept = strip.byte(first);
            for &r in &b[i + 1..i + c.len_utf8()] {
                kept &= strip.byte(r);
            }
            if !kept {
                if !was && strip.in_csi() {
                    params = i + 1;
                } else if was && !strip.in_csi() && !private(&b[params..i]) {
                    let p = &b[params..i];
                    let n = param(p, 1).max(1) as usize;
                    match first {
                        b'm' if styled => sgr(p, pen),
                        b'K' => match param(p, 0) {
                            0 => {
                                self.cells.truncate(col);
                                self.fill = pen.bg;
                            }
                            1 => {
                                let end = (col + 1).min(self.cells.len());
                                self.cells[..end].fill(BLANK);
                            }
                            2 => {
                                self.cells.clear();
                                self.fill = pen.bg;
                            }
                            _ => {}
                        },
                        b'C' | b'a' => col += n,
                        b'D' => col = col.saturating_sub(n),
                        b'G' | b'`' => col = n - 1,
                        _ => {}
                    }
                    // a move ends a strike
                    over = false;
                }
                continue;
            }
            cr_last = c == '\r';
            match c {
                '\r' => col = 0,
                '\u{8}' => {
                    col = col.saturating_sub(1);
                    over = true;
                    continue;
                }
                '\t' => {
                    let to = col + TAB_STOP - col % TAB_STOP;
                    if self.cells.len() < to {
                        self.cells.resize(to, BLANK);
                    }
                    col = to;
                }
                c => {
                    let width = if c.is_control() {
                        1
                    } else {
                        c.width().unwrap_or(0)
                    };
                    if width == 0 {
                        // a mark joins the character before it
                        if let Some(cell) = col.checked_sub(1).and_then(|p| self.cells.get_mut(p))
                            && !cell.blank
                            && usize::from(cell.len) + c.len_utf8() <= usize::from(u8::MAX)
                            && cell.at as usize + usize::from(cell.len) == self.arena.len()
                        {
                            cell.len += c.len_utf8() as u8;
                            let mut buf = [0; 4];
                            self.arena
                                .extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                        }
                        continue;
                    }
                    self.put(col, c, width, *pen, over);
                    col += width;
                }
            }
            over = false;
        }
        // the cells to the last written one, then a `\r` the line ended on (CRLF's)
        let end = self
            .cells
            .iter()
            .rposition(|c| !c.blank)
            .map_or(0, |i| i + 1);
        self.plain.clear();
        self.marks.clear();
        let mut style = None;
        for cell in &self.cells[..end] {
            if cell.len == 0 && !cell.blank {
                // a wide character's second column
                continue;
            }
            if style != Some(cell.style) {
                style = Some(cell.style);
                self.marks.push((self.plain.len(), cell.style));
            }
            if cell.blank {
                self.plain.push(b' ');
            } else {
                let at = cell.at as usize;
                self.plain
                    .extend_from_slice(&self.arena[at..at + usize::from(cell.len)]);
            }
        }
        // the pen the line ends with, for what follows it
        if style != Some(*pen) {
            self.marks.push((self.plain.len(), *pen));
        }
        if cr_last {
            self.plain.push(b'\r');
        }
    }

    /// Write `c`, `width` columns wide, at column `col` in `pen`: over what's there, struck over
    /// it when `over` (bold over itself, underlined over or under `_`).
    fn put(&mut self, col: usize, c: char, width: usize, pen: Style, over: bool) {
        if self.cells.len() < col + width {
            self.cells.resize(col + width, BLANK);
        }
        let mut style = pen;
        let mut text = c;
        if over && width == 1 && !self.cells[col].blank && self.cells[col].len > 0 {
            let cell = self.cells[col];
            let at = cell.at as usize;
            let was = std::str::from_utf8(&self.arena[at..at + usize::from(cell.len)])
                .ok()
                .and_then(|s| s.chars().next());
            match was {
                Some(w) if w == c => style = cell.style.add_modifier(Modifier::BOLD),
                Some('_') => style = cell.style.add_modifier(Modifier::UNDERLINED),
                Some(w) if c == '_' => {
                    text = w;
                    style = cell.style.add_modifier(Modifier::UNDERLINED);
                }
                _ => {}
            }
        }
        // a wide character cut in half leaves a blank
        if col > 0 && self.cells[col].len == 0 && !self.cells[col].blank {
            self.cells[col - 1] = BLANK;
        }
        let after = col + width;
        if after < self.cells.len() && self.cells[after].len == 0 && !self.cells[after].blank {
            self.cells[after] = BLANK;
        }
        let at = self.arena.len() as u32;
        let mut buf = [0; 4];
        self.arena
            .extend_from_slice(text.encode_utf8(&mut buf).as_bytes());
        self.cells[col] = Cell {
            at,
            len: text.len_utf8() as u8,
            style,
            blank: false,
        };
        for cell in &mut self.cells[col + 1..after] {
            *cell = Cell {
                at,
                len: 0,
                style,
                blank: false,
            };
        }
    }
}

/// Whether a CSI sequence's parameters are a private one's (`<`, `=`, `>` or `?` first).
fn private(params: &[u8]) -> bool {
    params
        .first()
        .is_some_and(|c| matches!(c, b'<' | b'=' | b'>' | b'?'))
}

/// A CSI sequence's first parameter, `default` when it has none.
fn param(params: &[u8], default: u32) -> u32 {
    let first = params.split(|&c| c == b';').next().unwrap_or_default();
    if first.is_empty() || !first.iter().all(u8::is_ascii_digit) {
        return default;
    }
    num(first)
}

/// A parameter's number, as large as it gets past `u32`'s room (PGR-007: no digits dropped).
fn num(p: &[u8]) -> u32 {
    p.iter().fold(0u32, |n, &d| {
        n.saturating_mul(10)
            .saturating_add(u32::from(d.wrapping_sub(b'0')).min(9))
    })
}

/// `style` with its colors the nearest of the first `n` indexed ones, 256 or 16.
fn nearest(style: Style, n: u16) -> Style {
    let near = |c: Option<Color>| c.map(|c| nearest_color(c, n));
    Style {
        fg: near(style.fg),
        bg: near(style.bg),
        underline_color: near(style.underline_color),
        ..style
    }
}

/// `c`, an RGB or indexed color, as the nearest of the first `n` indexed colors (xterm's).
fn nearest_color(c: Color, n: u16) -> Color {
    let (r, g, b) = match c.kind() {
        Kind::Rgb(r, g, b) => (r, g, b),
        Kind::Indexed(i) if u16::from(i) < n => return c,
        Kind::Indexed(i) => xterm(i),
        Kind::Reset => return c,
    };
    let distance = |i: u8| {
        let (x, y, z) = xterm(i);
        let d = |a: u8, b: u8| (i32::from(a) - i32::from(b)).pow(2);
        d(r, x) + d(g, y) + d(b, z)
    };
    if n > 16 {
        // the cube's and the gray ramp's nearest, past the 16 a theme may change
        let step = |v: u8| -> u8 {
            match v {
                0..48 => 0,
                48..115 => 1,
                _ => ((v - 35) / 40).min(5),
            }
        };
        let cube = 16 + 36 * step(r) + 6 * step(g) + step(b);
        let level = ((u16::from(r) + u16::from(g) + u16::from(b)) / 3) as u8;
        let gray = 232 + (level.saturating_sub(3) / 10).min(23);
        return Color::indexed(if distance(gray) < distance(cube) {
            gray
        } else {
            cube
        });
    }
    Color::indexed((0..16).min_by_key(|&i| distance(i)).unwrap_or(0))
}

/// Indexed color `i`'s RGB, as xterm has it.
fn xterm(i: u8) -> (u8, u8, u8) {
    const BASE: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    match i {
        0..16 => BASE[usize::from(i)],
        16..232 => {
            let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            let i = i - 16;
            (level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        _ => {
            let v = 8 + 10 * (i - 232);
            (v, v, v)
        }
    }
}

/// What the SGR sequence of `params` (`ESC [ params m`) does to `pen`. A private sequence
/// (`<`, `=`, `>` or `?` first) or one with intermediates does nothing.
fn sgr(params: &[u8], pen: &mut Style) {
    if params
        .first()
        .is_some_and(|c| matches!(c, b'<' | b'=' | b'>' | b'?'))
        || params
            .iter()
            .any(|&c| !(c.is_ascii_digit() || c == b';' || c == b':'))
    {
        return;
    }
    // no allocation: a colored log has a sequence every few words
    let mut tokens = params.split(|&c| c == b';');
    while let Some(token) = tokens.next() {
        if token.contains(&b':') {
            // `38:2::r:g:b`, `38:5:n`, `4:3`: one code and its subparameters, the first six read
            let (mut sub, mut len) = ([0u32; 6], 0);
            for (k, p) in token.split(|&c| c == b':').enumerate() {
                if let Some(s) = sub.get_mut(k) {
                    *s = num(p);
                }
                len = k + 1;
            }
            let get = |k: usize| (k < len.min(sub.len())).then(|| sub[k]);
            match sub[0] {
                4 => set(pen, Modifier::UNDERLINED, get(1).is_none_or(|n| n != 0)),
                code @ (38 | 48 | 58) => {
                    let color = match get(1) {
                        Some(5) => get(2).map(|n| Color::indexed(byte(n))),
                        // with a color space id before the three, or without one
                        Some(2) if len >= 6 => Some(rgb(sub[3], sub[4], sub[5])),
                        Some(2) if len == 5 => Some(rgb(sub[2], sub[3], sub[4])),
                        _ => None,
                    };
                    if let Some(color) = color {
                        set_color(pen, code, Some(color));
                    }
                }
                code => apply(code, pen),
            }
            continue;
        }
        let code = num(token);
        match code {
            38 | 48 | 58 => {
                let mut arg = || tokens.next().map(num);
                match arg() {
                    Some(5) => {
                        if let Some(n) = arg() {
                            set_color(pen, code, Some(Color::indexed(byte(n))));
                        }
                    }
                    Some(2) => {
                        if let (Some(r), Some(g), Some(b)) = (arg(), arg(), arg()) {
                            set_color(pen, code, Some(rgb(r, g, b)));
                        }
                    }
                    _ => {}
                }
            }
            code => apply(code, pen),
        }
    }
}

/// One SGR code without parameters of its own.
fn apply(code: u32, pen: &mut Style) {
    match code {
        0 => *pen = Style::new(),
        1 => set(pen, Modifier::BOLD, true),
        2 => set(pen, Modifier::DIM, true),
        3 => set(pen, Modifier::ITALIC, true),
        4 | 21 => set(pen, Modifier::UNDERLINED, true),
        5 => set(pen, Modifier::SLOW_BLINK, true),
        6 => set(pen, Modifier::RAPID_BLINK, true),
        7 => set(pen, Modifier::REVERSED, true),
        8 => set(pen, Modifier::HIDDEN, true),
        9 => set(pen, Modifier::CROSSED_OUT, true),
        22 => set(pen, Modifier::BOLD | Modifier::DIM, false),
        23 => set(pen, Modifier::ITALIC, false),
        24 => set(pen, Modifier::UNDERLINED, false),
        25 => set(pen, Modifier::SLOW_BLINK | Modifier::RAPID_BLINK, false),
        27 => set(pen, Modifier::REVERSED, false),
        28 => set(pen, Modifier::HIDDEN, false),
        29 => set(pen, Modifier::CROSSED_OUT, false),
        30..=37 => pen.fg = Some(Color::indexed((code - 30) as u8)),
        39 => pen.fg = None,
        40..=47 => pen.bg = Some(Color::indexed((code - 40) as u8)),
        49 => pen.bg = None,
        59 => pen.underline_color = None,
        90..=97 => pen.fg = Some(Color::indexed((code - 90 + 8) as u8)),
        100..=107 => pen.bg = Some(Color::indexed((code - 100 + 8) as u8)),
        _ => {}
    }
}

fn set(pen: &mut Style, m: Modifier, on: bool) {
    pen.add_modifier = if on {
        pen.add_modifier | m
    } else {
        pen.add_modifier - m
    };
}

/// Code 38's, 48's or 58's color: the foreground, background or underline's.
fn set_color(pen: &mut Style, code: u32, color: Option<Color>) {
    match code {
        38 => pen.fg = color,
        48 => pen.bg = color,
        _ => pen.underline_color = color,
    }
}

fn byte(n: u32) -> u8 {
    n.min(255) as u8
}

fn rgb(r: u32, g: u32, b: u32) -> Color {
    Color::rgb(byte(r), byte(g), byte(b))
}

/// How much of piped text [`keeps_own_colors`] reads: as much as a streamed print types it on
/// (`print::stream`'s `FIRST`), so the viewer and `-p` agree. A word diff shows on its first hunk's
/// first line; a plain diff's lines were read to its end (+9% of `--color-only`'s time on a 38 MB
/// `git log -p` once every hunk line was weighed).
const READ: usize = 1 << 20;

/// Whether piped `text`, holding escapes, shows in its own colors rather than ada's: when it is
/// colored at all (an SGR sequence that sets something, not only resets) and isn't a diff
/// (`diff`, as its text without them reads) whose colors only say what its marks say. A diff's
/// colors say more when a hunk's line reads as a word diff's (`word_diff_line`): git's
/// `--word-diff=color` and `--color-words`, whose lines carry no mark, the words changed only
/// colored. A word diff whose changes are all whole indented lines, each in one color and led
/// by a space, is told by its hunk's count instead: its added and removed lines read as
/// context, more of them than the hunk has room for. Read as far as `READ`, to the last
/// whole line in it.
pub fn keeps_own_colors(text: &str, diff: bool) -> bool {
    let text = match text.get(..READ) {
        Some(head) => head.rfind('\n').map_or(head, |nl| &head[..=nl]),
        // shorter, or `READ` inside a character: a character past ASCII is no newline
        None if text.len() <= READ => text,
        None => {
            let head = &text.as_bytes()[..READ];
            memchr::memrchr(b'\n', head).map_or(text, |nl| &text[..=nl])
        }
    };
    if !colored(text) {
        return false;
    }
    if !diff {
        return true;
    }
    // the lines a hunk has left, the old file's and the new one's
    let (mut old, mut new) = (0u32, 0u32);
    for line in text.lines() {
        let plain = strip(line);
        if old == 0 && new == 0 {
            if let Some(hunk) = diff_hunk(&plain) {
                (old, new) = (hunk.old, hunk.new);
            }
            continue;
        }
        if word_diff_line(line, &plain) {
            return true;
        }
        match plain.as_bytes().first() {
            Some(b'+') => new = new.saturating_sub(1),
            Some(b'-') => old = old.saturating_sub(1),
            Some(b'\\') => {}
            // a context line where a side has none left: a word diff's whole-line change,
            // marked by its indentation's space and in one color, which a diff's count has
            // no room for
            _ if old == 0 || new == 0 => return true,
            _ => (old, new) = (old - 1, new - 1),
        }
    }
    false
}

/// Whether `text` has an SGR sequence that sets something, or a character struck over
/// ([`has_overstrike`]), `man`'s bold and underline without colors.
fn colored(text: &str) -> bool {
    sgr_sequences(text).any(|(_, params)| sets(params)) || has_overstrike(text.as_bytes())
}

/// Whether a hunk's `line` (`plain` without its escapes) reads as a word diff's: with no mark
/// first, which a diff's every line has (an empty one aside, as some tools trim them), or with
/// text that isn't whitespace in other colors than its first character's. A diff colors a line
/// whole, its mark too; only a whitespace error (`color.diff.whitespace`, on any line under
/// `diff.wsErrorHighlight=all`) sits on part of one, and only on whitespace. A word diff's
/// lines whose own text starts with `-` or `+` (a list's, a `--` comment's) are told by their
/// colors, the unchanged words in none.
fn word_diff_line(line: &str, plain: &str) -> bool {
    let bytes = plain.as_bytes();
    if bytes.first().is_some_and(|b| !b" +-\\".contains(b)) {
        return true;
    }
    let cuts = sgr_sequences(line)
        .map(|(at, params)| (at, Some(params)))
        .chain(std::iter::once((bytes.len(), None)));
    let (mut pen, mut first, mut from) = (Style::new(), None, 0);
    for (at, params) in cuts {
        let shown = &bytes[from.min(bytes.len())..at.min(bytes.len())];
        if !shown.is_empty() {
            match first {
                None => first = Some(pen),
                Some(f) if f != pen && !shown.iter().all(u8::is_ascii_whitespace) => return true,
                Some(_) => {}
            }
        }
        from = at;
        if let Some(params) = params {
            sgr(params, &mut pen);
        }
    }
    false
}

/// Whether an SGR sequence's parameters set something: any code but a reset or an "off".
fn sets(params: &[u8]) -> bool {
    let mut pen = Style::new();
    sgr(params, &mut pen);
    pen != Style::new()
}

/// `text`'s SGR sequences: how much text came before each one, and its parameters.
fn sgr_sequences(text: &str) -> impl Iterator<Item = (usize, &[u8])> {
    let b = text.as_bytes();
    let mut strip = Strip::default();
    let (mut shown, mut params) = (0, 0);
    let mut i = 0;
    std::iter::from_fn(move || {
        while i < b.len() {
            // text runs to the next escape, a newline in it starting the count again
            if strip.in_text() {
                let next = memchr::memchr(0x1b, &b[i..]).map_or(b.len(), |n| i + n);
                let run = &b[i..next];
                shown = match memchr::memrchr(b'\n', run) {
                    Some(nl) => run.len() - nl - 1,
                    None => shown + run.len(),
                };
                i = next;
                if i == b.len() {
                    break;
                }
            }
            let (at, c) = (i, b[i]);
            i += 1;
            let was = strip.in_csi();
            if strip.byte(c) {
                shown = if c == b'\n' { 0 } else { shown + 1 };
            } else if !was && strip.in_csi() {
                params = at + 1;
            } else if was && !strip.in_csi() && c == b'm' {
                return Some((shown, &b[params..at]));
            }
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::expand_tabs;

    /// Each line's text and its runs, read by a fresh [`Ansi`], as `(text, style)` pieces.
    fn read(text: &str) -> Vec<Vec<(String, Option<Style>)>> {
        let mut ansi = Ansi::default();
        let mut lines = super::super::LineRuns::default();
        let shown: Vec<String> = super::shown(text)
            .lines()
            .map(|l| expand_tabs(l).into_owned())
            .collect();
        for line in super::super::Lines::new(text) {
            ansi.line(line, lines.open_line());
        }
        shown
            .iter()
            .enumerate()
            .map(|(n, line)| {
                let mut start = 0;
                let mut out = Vec::new();
                for run in lines.line(n).unwrap() {
                    let end = run.end as usize;
                    assert!(end <= line.len() && start < end, "{run:?} on {line:?}");
                    out.push((
                        line[start..end].to_owned(),
                        palette_style(run.rule, &ansi.palette),
                    ));
                    start = end;
                }
                if start < line.len() {
                    out.push((line[start..].to_owned(), None));
                }
                out
            })
            .collect()
    }

    fn fg(c: Color) -> Option<Style> {
        Some(Style::new().fg(c))
    }

    #[test]
    fn a_log_lines_colors() {
        // `git lg`'s line: `%C(208)%h%Creset -%C(214)%d%Creset %s`
        let line = "* \x1b[38;5;208mabc1234\x1b[m -\x1b[38;5;214m (HEAD -> main)\x1b[m feat: x";
        assert_eq!(
            read(line),
            [vec![
                ("* ".into(), None),
                ("abc1234".into(), fg(Color::indexed(208))),
                (" -".into(), None),
                (" (HEAD -> main)".into(), fg(Color::indexed(214))),
                (" feat: x".into(), None),
            ]]
        );
    }

    #[test]
    fn the_pen_carries_across_lines_and_resets() {
        let text = "\x1b[1;31mred\nstill\x1b[22m dim\x1b[0m\nplain";
        let bold_red = Some(Style::new().fg(Color::RED).add_modifier(Modifier::BOLD));
        assert_eq!(
            read(text),
            [
                vec![("red".into(), bold_red)],
                vec![("still".into(), bold_red), (" dim".into(), fg(Color::RED))],
                vec![("plain".into(), None)],
            ]
        );
    }

    #[test]
    fn colors_of_every_form() {
        let mut pen = Style::new();
        for (params, want) in [
            (&b"31"[..], Style::new().fg(Color::RED)),
            (b"91;44", Style::new().fg(Color::LIGHT_RED).bg(Color::BLUE)),
            (b"38;2;1;2;3", Style::new().fg(Color::rgb(1, 2, 3))),
            (b"38:2::1:2:3", Style::new().fg(Color::rgb(1, 2, 3))),
            (b"38:2:1:2:3", Style::new().fg(Color::rgb(1, 2, 3))),
            (b"48;5;200", Style::new().bg(Color::indexed(200))),
            (
                b"1;4:3",
                Style::new().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            ),
            (b"7;27;3", Style::new().add_modifier(Modifier::ITALIC)),
            (b"31;39", Style::new()),
            (b"", Style::new()),
            (b"?25", Style::new()),
        ] {
            pen = Style::new();
            sgr(params, &mut pen);
            assert_eq!(pen, want, "{}", String::from_utf8_lossy(params));
        }
        // a short 38 takes what it has and no more
        sgr(b"38;5", &mut pen);
        assert_eq!(pen, Style::new());
    }

    #[test]
    fn runs_sit_on_the_expanded_line() {
        let text = "\ta\x1b[31mb\tc\x1b[mé\x1b[32m\u{1}x";
        assert_eq!(
            read(text),
            [vec![
                ("    a".into(), None),
                ("b  c".into(), fg(Color::RED)),
                ("é".into(), None),
                (" x".into(), fg(Color::GREEN)),
            ]]
        );
    }

    #[test]
    fn a_cr_before_the_newline_goes() {
        assert_eq!(
            read("\x1b[31mred\x1b[m\r\nnext\r\x1b[32m\n"),
            [
                vec![("red".into(), fg(Color::RED))],
                vec![("next".into(), None)]
            ]
        );
    }

    #[test]
    fn which_text_keeps_its_colors() {
        // `git lg`, `ls --color`: their own
        assert!(keeps_own_colors("* \x1b[33mabc\x1b[m msg\n", false));
        // only resets and erases: nothing to keep
        assert!(!keeps_own_colors("text\x1b[m\x1b[K\n", false));
        // a colored diff: ada's
        let diff = "\x1b[1mdiff --git a/x b/x\x1b[m\n\x1b[36m@@ -1,2 +1,2 @@\x1b[m fn\n \
                    ctx\n\x1b[31m-old\x1b[m\n\x1b[32m+new\x1b[41m \x1b[m\n";
        assert!(!keeps_own_colors(diff, true));
        // `--word-diff=color`: the words' colors are the change
        let words = "\x1b[1mdiff --git a/x b/x\x1b[m\n\x1b[36m@@ -1,2 +1,2 @@\x1b[m\n\
                     ctx\n    let x = \x1b[31m1;\x1b[m\x1b[32m42;\x1b[m\n";
        assert!(keeps_own_colors(words, true));
        // a line past its hunk (a `--stat` of the next commit) is no word diff
        let stat = "\x1b[36m@@ -1 +1 @@\x1b[m\n\x1b[31m-a\x1b[m\n\x1b[32m+b\x1b[m\n \
                    x.rs | 2 \x1b[32m+\x1b[m\x1b[31m-\x1b[m\n";
        assert!(!keeps_own_colors(stat, true));
    }

    /// `diff.wsErrorHighlight=all`: a context or removed line's trailing whitespace on a red
    /// background is a whitespace error, not a word diff's colors; git's own lines, as written.
    #[test]
    fn whitespace_errors_are_no_word_diff() {
        let diff = "\x1b[1mdiff --git a/a.rs b/a.rs\x1b[m\n\x1b[36m@@ -1,3 +1,3 @@\x1b[m\n \
                    \x1b[mfn a() {\x1b[m\x1b[41m   \x1b[m\n\
                    \x1b[31m-\x1b[m\x1b[31m    let x = 1;\x1b[m\x1b[41m\t\x1b[m\n\
                    \x1b[32m+\x1b[m\x1b[32m    let x = 2;\x1b[m\n \x1b[m}\x1b[m\n";
        assert!(!keeps_own_colors(diff, true));
    }

    /// A word diff whose changes are whole lines: none changes color inside, but its lines
    /// carry no mark. And one of a list's lines, `-` first, its words colored.
    #[test]
    fn word_diffs_of_whole_lines_and_lists() {
        let lines = "\x1b[1mdiff --git a/w b/w\x1b[m\n\x1b[36m@@ -1,2 +1,3 @@\x1b[m\n\
                     one\n\x1b[32mnew line here\x1b[m\n\x1b[31mgone\x1b[m\n";
        assert!(keeps_own_colors(lines, true));
        let list = "\x1b[1mdiff --git a/l.md b/l.md\x1b[m\n\x1b[36m@@ -1,2 +1,2 @@\x1b[m\n\
                    - apple \x1b[31mpie\x1b[m\x1b[32mtart\x1b[m\n- banana\n";
        assert!(keeps_own_colors(list, true));
        // trimmed empty context lines are no word diff's
        let trimmed = "\x1b[36m@@ -1,3 +1,3 @@\x1b[m\n\n\x1b[31m-a\x1b[m\n\x1b[32m+b\x1b[m\n x\n";
        assert!(!keeps_own_colors(trimmed, true));
    }

    /// PGR-001: `--color-words` of indented lines, removed or added whole: every line led by a
    /// space and in one color, told by the hunk's count (git's own lines, as written).
    #[test]
    fn word_diffs_of_whole_indented_lines() {
        let removed = "\x1b[1mdiff --git a/y.rs b/y.rs\x1b[m\n\x1b[36m@@ -1,2 +1 @@\x1b[m\n\
                       \x1b[31m    a();\x1b[m\n    b();\x1b[m\n";
        assert!(keeps_own_colors(removed, true));
        let added = "\x1b[36m@@ -1,2 +1,3 @@\x1b[m\n\x1b[32m    a();\x1b[m\n    b();\x1b[m\n    \
                     c();\x1b[m\n";
        assert!(keeps_own_colors(added, true));
        // the same lines in a plain diff, counted out
        let plain = "\x1b[36m@@ -1,2 +1 @@\x1b[m\n\x1b[31m-    a();\x1b[m\n     b();\x1b[m\n";
        assert!(!keeps_own_colors(plain, true));
    }

    /// PGR-007: a line as a terminal shows it, a progress line's last update, `man`'s struck
    /// characters bold and underlined, moves along the line applied.
    #[test]
    fn lines_show_as_a_terminal_shows_them() {
        let shown = |t: &str| super::shown(t).into_owned();
        assert_eq!(shown("10%\r50%\r100%\ndone\n"), "100%\ndone\n");
        assert_eq!(shown("abcdef\rXY\n"), "XYcdef\n");
        assert_eq!(shown("old text\r\x1b[Knew\n"), "new\n");
        assert_eq!(shown("crlf\r\n\x1b[31mred\x1b[m\r\n"), "crlf\r\nred\r\n");
        assert_eq!(shown("a\x1b[3Cb\n"), "a   b\n");
        assert_eq!(shown("abc\x1b[2Dx\n"), "axc\n");
        assert_eq!(shown("abc\x1b[1Gx\n"), "xbc\n");
        assert_eq!(shown("abcd\x1b[2D\x1b[1Kx\n"), "  xd\n");
        assert_eq!(shown("abc\x1b[2Kx\n"), "   x\n");
        // a wide character half written over leaves a blank
        assert_eq!(shown("日本\r\x1b[1Cx\n"), " x本\n");
        assert_eq!(shown("ab\rü\u{301}\n"), "ü\u{301}b\n");
        assert_eq!(shown("\t\x1b[2Dx\n"), "  x\n");

        let bold = Some(Style::new().add_modifier(Modifier::BOLD));
        let under = Some(Style::new().add_modifier(Modifier::UNDERLINED));
        assert_eq!(
            read("N\x08NA\x08AM\x08ME\x08E is _\x08u_\x08l\n"),
            [vec![
                ("NAME".into(), bold),
                (" is ".into(), None),
                ("ul".into(), under)
            ]]
        );
        assert_eq!(
            read("x\x1b[31m50%\r\x1b[32m100%\x1b[m\n"),
            [vec![("100%".into(), fg(Color::GREEN))]]
        );
        assert!(has_overstrike(b"N\x08NAME"));
        assert!(!has_overstrike(b"a\x08\x08b, \x08"));
        assert!(keeps_own_colors("N\x08NA\x08AM\x08ME\x08E\n", false));
    }

    /// PGR-007: an `ESC [ K` with a background set fills the row past the text (delta's
    /// lines); one without, or none, fills nothing.
    #[test]
    fn an_erase_with_a_background_fills_the_row() {
        let st = super::super::SyntaxTheme::new(&crate::theme::Theme::builtin().syntax);
        let text = "\x1b[48;5;22m+ x\x1b[K\x1b[m\nplain\x1b[K\n\x1b[41mred\x1b[m tail\x1b[K\n";
        let spans = super::super::highlight(1, super::super::ANSI, text, &st);
        assert_eq!(spans.fill(0), Some(Color::indexed(22)));
        assert_eq!((spans.fill(1), spans.fill(2)), (None, None));
        let bg = Some(Style::new().bg(Color::indexed(22)));
        let runs: Vec<_> = spans
            .line(0)
            .iter()
            .map(|r| (r.end, palette_style(r.rule, spans.palette())))
            .collect();
        assert_eq!(runs, [(3, bg)]);
        assert_eq!(super::shown(text), "+ x\nplain\nred tail\n");
    }

    /// PGR-007: past the palette's room for styles as they are, colors near enough; SGR
    /// numbers of any length.
    #[test]
    fn many_styles_and_long_numbers() {
        let mut ansi = Ansi::default();
        let rgb = |i: u32| Style::new().fg(Color::rgb((i >> 16) as u8, (i >> 8) as u8, i as u8));
        let rules: Vec<Rule> = (0..PALETTE_MAX as u32 + 5000)
            .map(|i| ansi.rule(rgb(i * 97)))
            .collect();
        assert!(rules.iter().all(|r| r.0 != Rule::NONE));
        assert!(ansi.palette.len() <= PALETTE_MAX);
        let late = palette_style(*rules.last().unwrap(), &ansi.palette).unwrap();
        assert!(matches!(late.fg.map(Color::kind), Some(Kind::Indexed(_))));
        assert_eq!(
            nearest_color(Color::rgb(255, 0, 0), 256),
            Color::indexed(196)
        );
        assert_eq!(
            nearest_color(Color::rgb(128, 128, 128), 256),
            Color::indexed(244)
        );
        assert_eq!(
            nearest_color(Color::rgb(250, 10, 10), 16),
            Color::indexed(9)
        );

        let mut pen = Style::new();
        sgr(b"38;2;0000000000255;0;99999999999", &mut pen);
        assert_eq!(pen, Style::new().fg(Color::rgb(255, 0, 255)));
    }

    /// PGR-008: escapes after the last newline make no line of their own, in the text nor in
    /// its highlighting.
    #[test]
    fn escapes_after_the_last_newline_make_no_line() {
        let st = super::super::SyntaxTheme::new(&crate::theme::Theme::builtin().syntax);
        for (text, lines) in [
            ("\x1b[31ma\n\x1b[m", 1),
            ("\x1b[m", 0),
            ("a\n\x1b[m\x1b[K", 1),
            ("a\n\x1b[mb", 2),
        ] {
            let spans = super::super::highlight(1, super::super::ANSI, text, &st);
            assert_eq!(spans.line_count(), lines, "{text:?}");
            let shown = super::shown(text);
            assert_eq!(
                crate::doc::code_lines(&shown).count() as u32,
                lines,
                "{text:?}"
            );
        }
    }

    /// Read as far as a streamed print types its input: a word diff's first line past it isn't.
    #[test]
    fn colors_are_read_as_far_as_a_stream_types() {
        let hunk = "\x1b[36m@@ -1 +1 @@\x1b[m\n\x1b[31m-a\x1b[m\n\x1b[32m+b\x1b[m\n";
        let words = "\x1b[36m@@ -1 +1 @@\x1b[m\nx \x1b[31mold\x1b[m\x1b[32mnew\x1b[m\n";
        let mut text = hunk.repeat(READ / hunk.len());
        text.push_str(words);
        assert!(text.len() > READ);
        assert!(!keeps_own_colors(&text, true));
        text.replace_range(..hunk.len(), words);
        assert!(keeps_own_colors(&text, true));
    }
}
