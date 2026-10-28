//! The line a matcher reads, and the scans matchers share. Each matcher finds its pattern's
//! leftmost match at or after a position, as Oniguruma's search would, in time linear in what it
//! reads: no backtracking.

use std::cell::{Cell, OnceCell, RefCell};

use anpa::core::parse;
use anpa::findbyte::{eq, find_byte};

/// Byte offsets `(start, end)` of a pattern's match, `c[0]`, and of its groups 1 to 5; a group
/// that took part in no match is [`NONE`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct M {
    pub c: [(u32, u32); 6],
}

pub const NONE: (u32, u32) = (u32::MAX, u32::MAX);

impl M {
    pub fn at(s: usize, e: usize) -> M {
        M {
            c: [(s as u32, e as u32), NONE, NONE, NONE, NONE, NONE],
        }
    }

    pub fn cap(mut self, i: usize, s: usize, e: usize) -> M {
        self.c[i] = (s as u32, e as u32);
        self
    }

    /// The match, ending at `e` instead.
    pub fn end(mut self, e: usize) -> M {
        self.c[0].1 = e as u32;
        self
    }

    pub fn s(&self) -> usize {
        self.c[0].0 as usize
    }

    pub fn e(&self) -> usize {
        self.c[0].1 as usize
    }
}

pub type R = Option<M>;

/// A line as the matchers see it: its text with a `\n` after it, as syntect parses lines, and
/// what a level hands its patterns.
pub struct Line<'a> {
    pub s: &'a str,
    pub b: &'a [u8],
    /// What a backreference stands for: the quote an embedded shell script started with, a
    /// here-document's end word.
    pub cap: RefCell<Vec<u8>>,
    /// The builtin a command's arguments belong to, as an index into the shell's table.
    pub arg: Cell<u16>,
    parens: OnceCell<Parens>,
    last_eq: OnceCell<Option<usize>>,
}

impl<'a> Line<'a> {
    pub fn new(s: &'a str) -> Self {
        Self {
            s,
            b: s.as_bytes(),
            cap: RefCell::new(Vec::new()),
            arg: Cell::new(0),
            parens: OnceCell::new(),
            last_eq: OnceCell::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.b.len()
    }

    pub fn at(&self, i: usize) -> u8 {
        self.b.get(i).copied().unwrap_or(0)
    }

    /// The paren groups `just_eat` accepts, found once a line.
    pub fn parens(&self) -> &Parens {
        self.parens.get_or_init(|| Parens::new(self.b))
    }

    /// The line's last `=`.
    pub fn last_eq(&self) -> Option<usize> {
        *self
            .last_eq
            .get_or_init(|| self.b.iter().rposition(|&c| c == b'='))
    }

    /// Whether the character at `i` is a word character (`\w`).
    pub fn word_at(&self, i: usize) -> bool {
        match self.b.get(i) {
            None => false,
            Some(&c) if c < 0x80 => c.is_ascii_alphanumeric() || c == b'_',
            Some(_) => self.char_at(i).is_some_and(char::is_alphanumeric),
        }
    }

    /// Whether the character before `i` is a word character.
    pub fn word_before(&self, i: usize) -> bool {
        match i.checked_sub(1).map(|j| self.b[j]) {
            None => false,
            Some(c) if c < 0x80 => c.is_ascii_alphanumeric() || c == b'_',
            Some(_) => self.char_before(i).is_some_and(char::is_alphanumeric),
        }
    }

    /// The length of the alphabetic character at `i` (`[[:alpha:]]`), if it is one.
    pub fn alpha_at(&self, i: usize) -> Option<usize> {
        match self.b.get(i) {
            None => None,
            Some(&c) if c < 0x80 => c.is_ascii_alphabetic().then_some(1),
            Some(_) => self
                .char_at(i)
                .filter(|c| c.is_alphabetic())
                .map(char::len_utf8),
        }
    }

    /// The character starting at `i`, past ASCII: rare, so out of the matchers' way.
    #[cold]
    #[inline(never)]
    fn char_at(&self, i: usize) -> Option<char> {
        self.s.get(i..).and_then(|t| t.chars().next())
    }

    #[cold]
    #[inline(never)]
    fn char_before(&self, i: usize) -> Option<char> {
        self.s.get(..i).and_then(|t| t.chars().next_back())
    }

    /// The length of the character at `i`.
    pub fn char_len(&self, i: usize) -> usize {
        match self.at(i) {
            c if c < 0x80 => 1,
            c if c >= 0xf0 => 4,
            c if c >= 0xe0 => 3,
            _ => 2,
        }
    }
}

/// `\s`, in ASCII.
pub fn ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// The first `x` in `b[from..]`: anpa's scan, 8 bytes at a time.
///
/// Not inlined, like the scans below: a hundred matchers call them.
#[inline(never)]
pub fn find(b: &[u8], from: usize, x: u8) -> Option<usize> {
    let rest = b.get(from..)?;
    let found = parse(find_byte(eq(x), false), rest);
    found.result.map(|_| b.len() - found.state.len())
}

/// The first `x` or `y` in `b[from..]`.
#[inline(never)]
pub fn find2(b: &[u8], from: usize, x: u8, y: u8) -> Option<usize> {
    let rest = b.get(from..)?;
    let found = parse(find_byte(eq(x) | eq(y), false), rest);
    found.result.map(|_| b.len() - found.state.len())
}

/// The first `needle` in `b[from..]`.
#[inline(never)]
pub fn find_str(b: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    let mut at = from;
    while let Some(i) = find(b, at, needle[0]) {
        if b[i..].starts_with(needle) {
            return Some(i);
        }
        at = i + 1;
    }
    None
}

/// The first byte at or after `from` that isn't `\s`.
pub fn non_ws(b: &[u8], from: usize) -> Option<usize> {
    (from..b.len()).find(|&i| !ws(b[i]))
}

/// Where whitespace before `i` starts, but not before `lo`.
pub fn ws_back(b: &[u8], lo: usize, i: usize) -> usize {
    let mut q = i;
    while q > lo && ws(b[q - 1]) {
        q -= 1;
    }
    q
}

/// Where spaces (` `) before `i` start, but not before `lo`.
pub fn spaces_back(b: &[u8], lo: usize, i: usize) -> usize {
    let mut q = i;
    while q > lo && b[q - 1] == b' ' {
        q -= 1;
    }
    q
}

/// The end of the whitespace from `i`.
pub fn ws_end(b: &[u8], i: usize) -> usize {
    let mut j = i;
    while j < b.len() && ws(b[j]) {
        j += 1;
    }
    j
}

/// `$`: before the line's `\n`, or at its end.
pub fn eol_at(l: &Line, pos: usize) -> usize {
    let n = l.len();
    if pos < n {
        find(l.b, pos, b'\n').unwrap_or(n)
    } else {
        n
    }
}

/// The paren structure behind the Makefile grammar's `just_eat`: up to four levels of groups,
/// where each level allows only so many groups of so many shapes, and closes are optional
/// only by skipping a whole group. Every `(` gets its matching `)` and whether the group, as
/// level 1, is a shape `just_eat` takes; it is then matched whole or not at all, so a
/// lookahead built on it is decided by looking at runs of text between parens, linearly.
pub struct Parens {
    /// `(open, close, fits)` for each `(` that closes, by `open`.
    groups: Vec<(u32, u32, bool)>,
}

#[derive(Default, Clone, Copy)]
struct Shape {
    /// No groups inside.
    leaf: bool,
    /// Level 3 in level 2's first group: `()` or `(t(t)t)`.
    g3a: bool,
    /// Level 3 in level 2's second group: `(t)` or `(t(t)t)`.
    g3b: bool,
    g2a: bool,
    g2b: bool,
    g1: bool,
}

impl Parens {
    fn new(b: &[u8]) -> Self {
        struct Open {
            at: usize,
            kids: Vec<Shape>,
        }
        let mut stack: Vec<Open> = Vec::new();
        let mut groups = Vec::new();
        for (i, &c) in b.iter().enumerate() {
            match c {
                b'(' => stack.push(Open {
                    at: i,
                    kids: Vec::new(),
                }),
                b')' => {
                    let Some(open) = stack.pop() else {
                        continue;
                    };
                    let k = &open.kids;
                    let leaf = k.is_empty();
                    let one_leaf = k.len() == 1 && k[0].leaf;
                    let shape = Shape {
                        leaf,
                        g3a: (leaf && i == open.at + 1) || one_leaf,
                        g3b: leaf || one_leaf,
                        g2a: k.len() <= 2 && k.iter().all(|s| s.g3a),
                        g2b: k.len() <= 3 && k.iter().all(|s| s.g3b),
                        g1: match k.as_slice() {
                            [] => true,
                            [a] => a.g2a || a.g2b,
                            [a, b] => a.g2a && b.g2b,
                            _ => false,
                        },
                    };
                    groups.push((open.at as u32, i as u32, shape.g1));
                    if let Some(parent) = stack.last_mut() {
                        parent.kids.push(shape);
                    }
                }
                _ => {}
            }
        }
        groups.sort_unstable_by_key(|g| g.0);
        Self { groups }
    }

    /// Where the group opened at `open` closes, when it is one `just_eat` takes.
    pub fn fitting(&self, open: usize) -> Option<usize> {
        let i = self
            .groups
            .binary_search_by_key(&(open as u32), |g| g.0)
            .ok()?;
        let (_, close, fits) = self.groups[i];
        fits.then_some(close as usize)
    }
}
