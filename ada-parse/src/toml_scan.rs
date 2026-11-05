//! TOML by a scanner of ada's own (X-020, 2026-11-05): one pass over a line, a statement at a
//! time (a comment, a table's header, or a key, its `=` and its value), each value dispatched
//! on its first byte. It took the place of bat's grammar compiled by hand (2026-10-31), whose
//! stack of the grammar's contexts cost a dispatch and a push or pop a token. A kind is the
//! same outer meta scope (a key's, a table name's) × leaf as then ([`scopes`]), so the themes
//! color it as they did.
//!
//! Where it reads TOML as TOML 1.1 does and bat's grammar doesn't (syntect a guideline here,
//! `examples/parity.rs` reads how close it comes): blanks after a value are no error; an inline
//! table may run across lines and hold comments; a multi-line string closes at the last three
//! quotes of a run of up to five; `true` and `false` are whole words (`trueish` is a bad value);
//! `\e` and `\xHH` are escapes; an unterminated literal string is a string to the line's end, as
//! a basic one is; a table's header ends with its line. Bad keys and names have one rule each:
//! a key not followed by `=` leaves the rest of the line a bad key-value, a dot with no key
//! after it is a bad key, junk in a header runs to its `]`.
//!
//! The state between lines is one word: a multi-line string open, the arrays and inline tables
//! open (each level's kind, the innermost 48), and where in the innermost the line stopped.

use crate::matcher::{Atom, Line, is_word};
use crate::prim::find_any;
use crate::toml::{
    ATOMS, Digit, date, digits_from, fraction, int_part, offset, s, sign, time, word_end,
    word_start,
};
use crate::{Out, State};

/// The meta scope a span sits in: none, a key's, a table name's, an array of tables' name's.
/// A kind is `outer << 6 | leaf`.
const OUTERS: [&[Atom]; 4] = [
    &[],
    &[s::META_KEY],
    &[s::META_TABLE],
    &[s::META_TABLE_ARRAY],
];
const IN_KEY: u16 = 1 << 6;
const IN_TABLE: u16 = 2 << 6;
const IN_TABLE_ARRAY: u16 = 3 << 6;

/// The leaves: the scopes under the outer one.
mod leaf {
    pub const KEY: u16 = 1;
    pub const TABLE_NAME: u16 = 2;
    pub const KEY_DOT: u16 = 3;
    pub const TABLE_DOT: u16 = 4;
    pub const BAD_KEY: u16 = 5;
    pub const BAD_TABLE: u16 = 6;
    pub const BASIC: u16 = 7;
    pub const LITERAL: u16 = 12;
    pub const TRIPLE_BASIC: u16 = 15;
    pub const TRIPLE_LITERAL: u16 = 20;
    pub const COMMENT: u16 = 23;
    pub const COMMENT_MARK: u16 = 24;
    pub const DATETIME: u16 = 25;
    pub const FLOAT: u16 = 26;
    pub const FLOAT_SIGN: u16 = 27;
    pub const INTEGER: u16 = 28;
    pub const INTEGER_SIGN: u16 = 29;
    pub const BOOL: u16 = 30;
    pub const BAD_VALUE: u16 = 31;
    pub const KEY_VALUE: u16 = 32;
    pub const ARRAY_BEGIN: u16 = 33;
    pub const ARRAY_END: u16 = 34;
    pub const ARRAY_SEP: u16 = 35;
    pub const BAD_ARRAY: u16 = 36;
    pub const INLINE_BEGIN: u16 = 37;
    pub const INLINE_END: u16 = 38;
    pub const INLINE_SEP: u16 = 39;
    // 40, a bad inline table end, is no longer given (TOML 1.1 takes a trailing `,`)
    pub const TABLE_BEGIN: u16 = 41;
    pub const TABLE_END: u16 = 42;
    pub const TABLE_ARRAY_BEGIN: u16 = 43;
    pub const TABLE_ARRAY_END: u16 = 44;
    pub const BAD_KEYVAL: u16 = 45;
    pub const BAD_TRAILING: u16 = 46;
    pub const ILLEGAL: u16 = 47;
}

/// A string's leaves, from its text: its text, its opening quote, its closing one, an escape,
/// a bad escape (the last two only in basic strings).
const TEXT: u16 = 0;
const BEGIN: u16 = 1;
const END: u16 = 2;
const ESCAPE: u16 = 3;
const BAD_ESCAPE: u16 = 4;

static LEAVES: [&[Atom]; 48] = [
    &[],
    &[s::KEY],
    &[s::TABLE_NAME],
    &[s::KEY_DOT],
    &[s::TABLE_DOT],
    &[s::BAD_KEY],
    &[s::BAD_TABLE],
    &[s::BASIC],
    &[s::BASIC, s::STR_BEGIN],
    &[s::BASIC, s::STR_END],
    &[s::BASIC, s::ESCAPE],
    &[s::BASIC, s::BAD_ESCAPE],
    &[s::LITERAL],
    &[s::LITERAL, s::STR_BEGIN],
    &[s::LITERAL, s::STR_END],
    &[s::TRIPLE_BASIC],
    &[s::TRIPLE_BASIC, s::STR_BEGIN],
    &[s::TRIPLE_BASIC, s::STR_END],
    &[s::TRIPLE_BASIC, s::ESCAPE],
    &[s::TRIPLE_BASIC, s::BAD_ESCAPE],
    &[s::TRIPLE_LITERAL],
    &[s::TRIPLE_LITERAL, s::STR_BEGIN],
    &[s::TRIPLE_LITERAL, s::STR_END],
    &[s::COMMENT],
    &[s::COMMENT, s::COMMENT_MARK],
    &[s::DATETIME],
    &[s::FLOAT],
    &[s::FLOAT, s::SIGN],
    &[s::INTEGER],
    &[s::INTEGER, s::SIGN],
    &[s::BOOL],
    &[s::BAD_VALUE],
    &[s::KEY_VALUE],
    &[s::ARRAY_BEGIN],
    &[s::ARRAY_END],
    &[s::ARRAY_SEP],
    &[s::BAD_ARRAY],
    &[s::INLINE_BEGIN],
    &[s::INLINE_END],
    &[s::INLINE_SEP],
    &[s::BAD_TABLE_END],
    &[s::TABLE_BEGIN],
    &[s::TABLE_END],
    &[s::TABLE_ARRAY_BEGIN],
    &[s::TABLE_ARRAY_END],
    &[s::BAD_KEYVAL],
    &[s::BAD_TRAILING],
    &[s::ILLEGAL],
];

/// The scopes under `source.toml` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    let outer = OUTERS.get(usize::from(kind >> 6)).copied().unwrap_or(&[]);
    let leaf = LEAVES.get(usize::from(kind & 63)).copied().unwrap_or(&[]);
    outer
        .iter()
        .chain(leaf)
        .map(|&a| ATOMS[usize::from(a)])
        .collect()
}

/// A byte's classes: a bare key's (`[A-Za-z0-9_-]`), what a key or a table name starts with
/// (those and the quotes), a blank (`[ \t]`).
const BARE: u8 = 1;
const KEY_START: u8 = 2;
const BLANK: u8 = 4;

static CLASS: [u8; 256] = {
    let mut t = [0; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'-' {
            t[c] = BARE | KEY_START;
        }
        c += 1;
    }
    t[b'"' as usize] = KEY_START;
    t[b'\'' as usize] = KEY_START;
    t[b' ' as usize] = BLANK;
    t[b'\t' as usize] = BLANK;
    t
};

#[inline(always)]
fn class(c: u8) -> u8 {
    CLASS[usize::from(c)]
}

/// Where the run of `[ \t]` from `p` ends.
#[inline(always)]
fn blanks(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && class(b[p]) & BLANK != 0 {
        p += 1;
    }
    p
}

/// Where the line stopped, in the innermost array or inline table, or at the top level. Only
/// the first six are kept between lines; at the top level a line starts a statement.
mod at {
    /// An array's value (after its `[` or a `,`).
    pub const ARRAY_VALUE: u8 = 0;
    /// After an array's value: its `,` or `]`.
    pub const ARRAY_SEP: u8 = 1;
    /// An inline table's first key, or its `}`.
    pub const INLINE_KEY: u8 = 2;
    /// An inline table's key after a `,`, or its `}` (TOML 1.1).
    pub const INLINE_NEXT: u8 = 3;
    /// After an inline table's key: its `=`.
    pub const INLINE_EQ: u8 = 4;
    /// After an inline table's value: its `,` or `}`.
    pub const INLINE_SEP: u8 = 5;
    /// A line's start at the top level.
    pub const LINE: u8 = 6;
    /// A value, after a `=`.
    pub const VALUE: u8 = 7;
    /// After a value at the top level: a comment, or the line's end.
    pub const TOP_AFTER: u8 = 8;
}

/// A multi-line string open: none, a basic one, a literal one.
const NO_STRING: u64 = 0;
const BASIC_OPEN: u64 = 1;
const LITERAL_OPEN: u64 = 2;

/// The levels of arrays and inline tables whose kinds are kept, the innermost; past them, a
/// level is taken for an array.
const KINDS: u32 = 48;

/// Lex `b`, a line without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let w = state.word();
    let mut t = Scan {
        b,
        depth: (w >> 8) as u8,
        kinds: w >> 16,
        string: NO_STRING,
        stop: at::LINE,
        out,
    };
    let (p, at) = match w & 3 {
        NO_STRING if t.depth == 0 => (0, at::LINE),
        NO_STRING => (0, (w >> 2 & 7) as u8),
        open => match t.triple(0, open) {
            Some(e) => (e, t.after()),
            None => (b.len(), at::LINE),
        },
    };
    if t.string == NO_STRING {
        t.run(p, at);
    }
    let stop = match t.depth {
        0 => 0,
        _ => u64::from(t.stop),
    };
    state.set_word(t.string | stop << 2 | u64::from(t.depth) << 8 | t.kinds << 16);
}

struct Scan<'b, 'o, 'c> {
    b: &'b [u8],
    /// The arrays and inline tables open.
    depth: u8,
    /// Bit `i`: whether the `i`th level from the innermost is an inline table.
    kinds: u64,
    /// A multi-line string left open at the line's end.
    string: u64,
    /// Where the line stopped inside the innermost level.
    stop: u8,
    out: &'o mut Out<'c>,
}

impl Scan<'_, '_, '_> {
    #[inline(always)]
    fn put(&mut self, s: usize, e: usize, kind: u16) {
        self.out.push(s, e, kind);
    }

    /// What follows a value: the innermost level's separator, or the top level's end.
    #[inline(always)]
    fn after(&self) -> u8 {
        match (self.depth, self.kinds & 1) {
            (0, _) => at::TOP_AFTER,
            (_, 0) => at::ARRAY_SEP,
            _ => at::INLINE_SEP,
        }
    }

    fn open(&mut self, inline: bool) {
        self.depth = self.depth.saturating_add(1);
        self.kinds = (self.kinds << 1 | u64::from(inline)) & ((1 << KINDS) - 1);
    }

    /// Closes the innermost level: what follows it.
    fn close(&mut self) -> u8 {
        self.depth -= 1;
        self.kinds >>= 1;
        self.after()
    }

    /// The line from `p`, where `at` says it is.
    #[inline(always)]
    fn run(&mut self, mut p: usize, mut at: u8) {
        let b = self.b;
        let nl = b.len();
        loop {
            match at {
                at::LINE => {
                    let q = blanks(b, p);
                    let Some(&c) = b.get(q) else { return };
                    if c == b'#' {
                        return self.comment(q);
                    }
                    if c == b'[' {
                        p = self.header(q);
                        at = at::TOP_AFTER;
                        continue;
                    }
                    if class(c) & KEY_START == 0 {
                        return self.put(q, nl, leaf::ILLEGAL);
                    }
                    let e = self.key(q, IN_KEY);
                    let r = blanks(b, e);
                    match b.get(r) {
                        Some(b'=') => {
                            self.put(r, r + 1, leaf::KEY_VALUE);
                            p = blanks(b, r + 1);
                            at = at::VALUE;
                        }
                        Some(_) => return self.put(r, nl, leaf::BAD_KEYVAL),
                        None => return,
                    }
                }
                at::VALUE => {
                    if p >= nl {
                        at = self.after();
                        continue;
                    }
                    (p, at) = self.value(p);
                    if self.string != NO_STRING {
                        return;
                    }
                }
                at::TOP_AFTER => {
                    let q = blanks(b, p);
                    match b.get(q) {
                        Some(b'#') => self.comment(q),
                        Some(_) => self.put(q, nl, leaf::BAD_TRAILING),
                        None => {}
                    }
                    return;
                }
                at::ARRAY_VALUE | at::ARRAY_SEP => {
                    p = blanks(b, p);
                    let Some(&c) = b.get(p) else {
                        self.stop = at;
                        return;
                    };
                    match c {
                        b'#' => {
                            self.stop = at;
                            return self.comment(p);
                        }
                        b']' => {
                            self.put(p, p + 1, leaf::ARRAY_END);
                            p += 1;
                            at = self.close();
                        }
                        b',' if at == at::ARRAY_SEP => {
                            self.put(p, p + 1, leaf::ARRAY_SEP);
                            p += 1;
                            at = at::ARRAY_VALUE;
                        }
                        _ if at == at::ARRAY_VALUE => {
                            (p, at) = self.value(p);
                            if self.string != NO_STRING {
                                return;
                            }
                        }
                        _ => {
                            let e = word_or_char(b, p);
                            self.out.push_merged(p, e, leaf::BAD_ARRAY);
                            p = e;
                        }
                    }
                }
                at::INLINE_KEY | at::INLINE_NEXT => {
                    p = blanks(b, p);
                    let Some(&c) = b.get(p) else {
                        self.stop = at;
                        return;
                    };
                    match c {
                        b'#' => {
                            self.stop = at;
                            return self.comment(p);
                        }
                        // TOML 1.1 lets a `,` end the last pair
                        b'}' => {
                            self.put(p, p + 1, leaf::INLINE_END);
                            p += 1;
                            at = self.close();
                        }
                        _ if class(c) & KEY_START != 0 => {
                            p = self.key(p, IN_KEY);
                            at = at::INLINE_EQ;
                        }
                        _ => {
                            let e = word_or_char(b, p);
                            self.out.push_merged(p, e, IN_KEY | leaf::BAD_KEY);
                            p = e;
                        }
                    }
                }
                at::INLINE_EQ => {
                    p = blanks(b, p);
                    match b.get(p) {
                        Some(b'=') => {
                            self.put(p, p + 1, leaf::KEY_VALUE);
                            p = blanks(b, p + 1);
                            at = at::VALUE;
                        }
                        Some(_) => at = self.close(),
                        None => {
                            self.stop = at;
                            return;
                        }
                    }
                }
                _ => {
                    // at::INLINE_SEP
                    p = blanks(b, p);
                    let Some(&c) = b.get(p) else {
                        self.stop = at;
                        return;
                    };
                    match c {
                        b'#' => {
                            self.stop = at;
                            return self.comment(p);
                        }
                        b'}' => {
                            self.put(p, p + 1, leaf::INLINE_END);
                            p += 1;
                            at = self.close();
                        }
                        b',' => {
                            self.put(p, p + 1, leaf::INLINE_SEP);
                            p += 1;
                            at = at::INLINE_NEXT;
                        }
                        _ => at = self.close(),
                    }
                }
            }
        }
    }

    /// A key at `p`, on a byte a key starts with, its parts and dots in `outer`: where it
    /// ends, the line's end if a quoted part is left open.
    fn key(&mut self, p: usize, outer: u16) -> usize {
        let (name, dot, bad) = match outer {
            IN_KEY => (leaf::KEY, leaf::KEY_DOT, leaf::BAD_KEY),
            _ => (leaf::TABLE_NAME, leaf::TABLE_DOT, leaf::BAD_TABLE),
        };
        let b = self.b;
        let mut p = p;
        loop {
            let e = match b[p] {
                b'"' => match self.basic(p, outer | leaf::BASIC) {
                    Some(e) => e,
                    None => return b.len(),
                },
                b'\'' => match self.literal(p, outer | leaf::LITERAL) {
                    Some(e) => e,
                    None => {
                        let rest = b[p + 1..].iter().take_while(|&&c| class(c) & BLANK == 0);
                        let e = p + 1 + rest.count();
                        self.put(p, e, outer | bad);
                        e
                    }
                },
                _ => {
                    let mut e = p + 1;
                    while e < b.len() && class(b[e]) & BARE != 0 {
                        e += 1;
                    }
                    self.put(p, e, outer | name);
                    e
                }
            };
            let q = blanks(b, e);
            if b.get(q) != Some(&b'.') {
                return e;
            }
            let r = blanks(b, q + 1);
            if !b.get(r).is_some_and(|&c| class(c) & KEY_START != 0) {
                self.put(e, q, outer);
                self.put(q, q + 1, outer | bad);
                return q + 1;
            }
            self.put(e, q, outer);
            self.put(q, q + 1, outer | dot);
            self.put(q + 1, r, outer);
            p = r;
        }
    }

    /// A table's header at `p`, its `[`: where it ends.
    fn header(&mut self, p: usize) -> usize {
        let b = self.b;
        let nl = b.len();
        let two = b.get(p + 1) == Some(&b'[');
        let n = 1 + usize::from(two);
        let q = blanks(b, p + n);
        // `[]`, `[ ]`, `[[]]`, `[[]`, `[]]`: no name
        if b.get(q) == Some(&b']') {
            let e = q + 1 + usize::from(b.get(q + 1) == Some(&b']'));
            self.put(p, e, leaf::BAD_TABLE);
            return e;
        }
        let (outer, begin, end) = match two {
            true => (
                IN_TABLE_ARRAY,
                leaf::TABLE_ARRAY_BEGIN,
                leaf::TABLE_ARRAY_END,
            ),
            false => (IN_TABLE, leaf::TABLE_BEGIN, leaf::TABLE_END),
        };
        self.put(p, p + n, begin);
        let mut e = q;
        if b.get(q).is_some_and(|&c| class(c) & KEY_START != 0) {
            e = self.key(q, outer);
        }
        let r = blanks(b, e);
        if r >= nl {
            return nl;
        }
        if b[r] == b']' && (!two || b.get(r + 1) == Some(&b']')) {
            self.put(r, r + n, end);
            return r + n;
        }
        // junk to the header's `]` (an array of tables' `]]`, or its first `]` without one)
        let close = |s: &[u8]| s.windows(2).position(|w| w == b"]]").map(|i| i + 2);
        let first = |s: &[u8]| s.iter().position(|&c| c == b']').map(|i| i + 1);
        let k = match two {
            true => close(&b[r..]).or_else(|| first(&b[r..])),
            false => first(&b[r..]),
        };
        let k = k.map_or(nl, |k| r + k);
        self.put(r, k, leaf::BAD_TABLE);
        k
    }

    /// A value at `p`, before the line's end and not a blank: where it ends and what follows.
    #[inline(always)]
    fn value(&mut self, p: usize) -> (usize, u8) {
        let b = self.b;
        let c = b[p];
        match c {
            b'"' => {
                if b[p..].starts_with(b"\"\"\"") {
                    self.put(p, p + 3, leaf::TRIPLE_BASIC + BEGIN);
                    return match self.triple(p + 3, BASIC_OPEN) {
                        Some(e) => (e, self.after()),
                        None => (b.len(), at::LINE),
                    };
                }
                let e = self.basic(p, leaf::BASIC).unwrap_or(b.len());
                return (e, self.after());
            }
            b'\'' => {
                if b[p..].starts_with(b"'''") {
                    self.put(p, p + 3, leaf::TRIPLE_LITERAL + BEGIN);
                    return match self.triple(p + 3, LITERAL_OPEN) {
                        Some(e) => (e, self.after()),
                        None => (b.len(), at::LINE),
                    };
                }
                let e = match self.literal(p, leaf::LITERAL) {
                    Some(e) => e,
                    None => {
                        self.put(p, p + 1, leaf::LITERAL + BEGIN);
                        self.put(p + 1, b.len(), leaf::LITERAL + TEXT);
                        b.len()
                    }
                };
                return (e, self.after());
            }
            b'[' => {
                self.put(p, p + 1, leaf::ARRAY_BEGIN);
                self.open(false);
                return (p + 1, at::ARRAY_VALUE);
            }
            b'{' => {
                self.put(p, p + 1, leaf::INLINE_BEGIN);
                self.open(true);
                return (p + 1, at::INLINE_KEY);
            }
            _ => {}
        }
        let l = Line { b };
        let e = match c {
            b't' | b'f' => {
                let word: &[u8] = if c == b't' { b"true" } else { b"false" };
                match b[p..].starts_with(word) && word_end(&l, p + word.len()) {
                    true => {
                        self.put(p, p + word.len(), leaf::BOOL);
                        Some(p + word.len())
                    }
                    false => None,
                }
            }
            b'0'..=b'9' | b'+' | b'-' | b'i' | b'n' => self.number(&l, p),
            _ => None,
        };
        let e = e.unwrap_or_else(|| {
            let e = word_or_char(b, p);
            self.put(p, e, leaf::BAD_VALUE);
            e
        });
        (e, self.after())
    }

    /// A date-time, a float or an integer at `p`: where it ends.
    fn number(&mut self, l: &Line, p: usize) -> Option<usize> {
        if l.b[p].is_ascii_digit()
            && let Some(e) = datetime(l, p)
        {
            self.put(p, e, leaf::DATETIME);
            return Some(e);
        }
        if let Some((q, e)) = float(l, p).or_else(|| inf_nan(l, p)) {
            self.put(p, q, leaf::FLOAT_SIGN);
            self.put(q, e, leaf::FLOAT);
            return Some(e);
        }
        let (q, e) = integer(l, p)?;
        self.put(p, q, leaf::INTEGER_SIGN);
        self.put(q, e, leaf::INTEGER);
        Some(e)
    }

    /// `'…'` at `p` as `base`'s leaves: where it ends, or none if it isn't closed.
    fn literal(&mut self, p: usize, base: u16) -> Option<usize> {
        let b = self.b;
        let j = p + 1 + b[p + 1..].iter().position(|&c| c == b'\'')?;
        self.put(p, p + 1, base + BEGIN);
        self.put(p + 1, j, base + TEXT);
        self.put(j, j + 1, base + END);
        Some(j + 1)
    }

    /// A basic string at `p`, its `"`, as `base`'s leaves, its escapes marked: where it ends, or
    /// none if it runs to the line's end unclosed.
    fn basic(&mut self, p: usize, base: u16) -> Option<usize> {
        self.put(p, p + 1, base + BEGIN);
        let b = self.b;
        let mut from = p + 1;
        let mut i = from;
        loop {
            let k = find_any(b, i, *b"\"\\");
            if k >= b.len() {
                self.put(from, b.len(), base + TEXT);
                return None;
            }
            if b[k] == b'"' {
                self.put(from, k, base + TEXT);
                self.put(k, k + 1, base + END);
                return Some(k + 1);
            }
            from = self.escape(k, from, base);
            i = from.max(k + 1);
        }
    }

    /// The escape at `k` (a `\`) in a basic string whose text so far starts at `from`: where
    /// the text goes on.
    fn escape(&mut self, k: usize, from: usize, base: u16) -> usize {
        let b = self.b;
        let n = escape(b, k);
        if n > 0 {
            self.put(from, k, base + TEXT);
            self.put(k, k + n, base + ESCAPE);
            return k + n;
        }
        if k + 1 >= b.len() {
            // a `\` ending the line: text (in a multi-line string, its line ending)
            return from;
        }
        let e = k + 1 + Line { b }.char_len(k + 1).unwrap_or(1);
        self.put(from, k, base + TEXT);
        self.put(k, e, base + BAD_ESCAPE);
        e
    }

    /// A multi-line string's text from `p`, `open` saying which: where it closes, or none if it
    /// goes on past the line (kept in [`Scan::string`]).
    fn triple(&mut self, p: usize, open: u64) -> Option<usize> {
        let b = self.b;
        let (quote, base) = match open {
            BASIC_OPEN => (b'"', leaf::TRIPLE_BASIC),
            _ => (b'\'', leaf::TRIPLE_LITERAL),
        };
        let mut from = p;
        let mut i = p;
        loop {
            let k = match open {
                BASIC_OPEN => find_any(b, i, *b"\"\\"),
                _ => find_any(b, i, *b"'"),
            };
            if k >= b.len() {
                self.put(from, b.len(), base + TEXT);
                self.string = open;
                return None;
            }
            if b[k] == b'\\' {
                from = self.escape(k, from, base);
                i = from.max(k + 1);
                continue;
            }
            let run = b[k..].iter().take_while(|&&c| c == quote).count();
            if run < 3 {
                i = k + run;
                continue;
            }
            // the last three of up to five close it
            let e = k + run.min(5);
            self.put(from, e - 3, base + TEXT);
            self.put(e - 3, e, base + END);
            return Some(e);
        }
    }

    /// A comment at `p`, its `#`, to the line's end.
    fn comment(&mut self, p: usize) {
        self.put(p, p + 1, leaf::COMMENT_MARK);
        self.put(p + 1, self.b.len(), leaf::COMMENT);
    }
}

/// `\w+|.` at `p`.
fn word_or_char(b: &[u8], p: usize) -> usize {
    match is_word(b[p]) {
        true => p + b[p..].iter().take_while(|&&c| is_word(c)).count(),
        false => p + Line { b }.char_len(p).unwrap_or(1),
    }
}

/// The length of a valid escape at `k` (its `\`), or 0: `\b \t \n \f \r \" \\ \e`, `\xHH`,
/// `\uHHHH`, `\UHHHHHHHH`.
fn escape(b: &[u8], k: usize) -> usize {
    let hex = |at: usize, n: usize| {
        b.get(at..at + n)
            .is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit))
    };
    match b.get(k + 1) {
        Some(b'b' | b't' | b'n' | b'f' | b'r' | b'"' | b'\\' | b'e') => 2,
        Some(b'x') if hex(k + 2, 2) => 4,
        Some(b'u') if hex(k + 2, 4) => 6,
        Some(b'U') if hex(k + 2, 8) => 10,
        _ => 0,
    }
}

/// A date, a time, or both with an offset, at `p`.
fn datetime(l: &Line, p: usize) -> Option<usize> {
    if let Some(d) = date(l, p) {
        if l.is(d, |c| matches!(c, b't' | b'T' | b' '))
            && let Some(t) = time(l, d + 1)
        {
            return Some(offset(l, t).unwrap_or(t));
        }
        return Some(d);
    }
    time(l, p)
}

/// A float at `p`: where its sign ends, and it.
fn float(l: &Line, p: usize) -> Option<(usize, usize)> {
    if !word_start(l, p) {
        return None;
    }
    let q = sign(l, p);
    int_part(l, q, |r| fraction(l, r)).map(|e| (q, e))
}

/// `[+-]?(inf|nan)` as a word at `p`.
fn inf_nan(l: &Line, p: usize) -> Option<(usize, usize)> {
    if !word_start(l, p) {
        return None;
    }
    let q = sign(l, p);
    let word = l.b.get(q..q + 3)?;
    ((word == b"inf" || word == b"nan") && word_end(l, q + 3)).then_some((q, q + 3))
}

/// An integer at `p` (decimal with a sign, or hexadecimal, octal or binary): where its sign
/// ends, and it.
fn integer(l: &Line, p: usize) -> Option<(usize, usize)> {
    if l.at(p) == Some(b'0') {
        let radix: [(u8, Digit); 3] = [
            (b'x', |c| c.is_ascii_hexdigit()),
            (b'o', |c| (b'0'..=b'7').contains(&c)),
            (b'b', |c| c == b'0' || c == b'1'),
        ];
        for (n, (mark, d)) in radix.into_iter().enumerate() {
            if (n > 0 || word_start(l, p)) && l.at(p + 1) == Some(mark) && l.is(p + 2, d) {
                return Some((p, digits_from(l, p + 3, d)));
            }
        }
    }
    let q = sign(l, p);
    int_part(l, q, |r| word_end(l, r).then_some(r)).map(|e| (q, e))
}

#[cfg(test)]
mod tests {
    use crate::Lang;
    use crate::tests::marked;

    fn kinds(lines: &[&'static str]) -> Vec<(&'static str, &'static str)> {
        marked(Lang::Toml, lines)
            .into_iter()
            .map(|(t, s)| (t, s.rsplit(' ').next().unwrap()))
            .collect()
    }

    #[test]
    fn keys_and_values() {
        assert_eq!(
            kinds(&["name = \"ada\" # why"]),
            [
                ("name", "entity.name.tag.toml"),
                ("=", "punctuation.definition.key-value.toml"),
                ("\"", "punctuation.definition.string.begin.toml"),
                ("ada", "string.quoted.double.basic.toml"),
                ("\"", "punctuation.definition.string.end.toml"),
                ("#", "punctuation.definition.comment.toml"),
                (" why", "comment.line.number-sign.toml"),
            ]
        );
    }

    #[test]
    fn arrays_run_across_lines() {
        let got = kinds(&["x = [", "  1, -2.5,", "]"]);
        assert!(
            got.contains(&("-", "keyword.operator.arithmetic.toml")),
            "{got:?}"
        );
        assert!(
            got.contains(&("1", "constant.numeric.integer.toml")),
            "{got:?}"
        );
        assert_eq!(
            got.last(),
            Some(&("]", "punctuation.definition.array.end.toml"))
        );
    }

    /// Where it reads TOML 1.1 and bat's grammar doesn't.
    #[test]
    fn toml_1_1_where_bat_differs() {
        // blanks after a value are no error
        assert_eq!(kinds(&["a = 1   "]).len(), 3);
        // an inline table across lines, with a comment and a trailing `,`
        let got = kinds(&["t = {", " a = 1, # c", "}"]);
        assert!(got.contains(&("a", "entity.name.tag.toml")), "{got:?}");
        assert!(
            got.contains(&(" c", "comment.line.number-sign.toml")),
            "{got:?}"
        );
        assert_eq!(
            got.last(),
            Some(&("}", "punctuation.definition.inline-table.end.toml"))
        );
        // a multi-line string closes at the last three quotes of up to five
        let got = kinds(&[r#"s = """a""""""#]);
        assert!(
            got.contains(&(r#"a"""#, "string.quoted.triple.basic.block.toml")),
            "{got:?}"
        );
        // whole words, the new escapes, an unterminated literal string
        assert_eq!(
            kinds(&["f = trueish"]).last(),
            Some(&("trueish", "invalid.illegal.value.toml"))
        );
        let got = kinds(&[r#"k = "\e\x41""#]);
        assert!(
            got.contains(&(r"\e", "constant.character.escape.toml"))
                && got.contains(&(r"\x41", "constant.character.escape.toml")),
            "{got:?}"
        );
        assert_eq!(
            kinds(&["y = 'also"]).last(),
            Some(&("also", "string.quoted.single.literal.toml"))
        );
    }

    #[test]
    fn bad_keys_and_headers() {
        assert_eq!(
            kinds(&["a b = 1"]),
            [
                ("a", "entity.name.tag.toml"),
                ("b = 1", "invalid.illegal.keyval.toml")
            ]
        );
        assert_eq!(
            kinds(&["[[a]b]]"]).last(),
            Some(&("]b]]", "invalid.illegal.table.toml"))
        );
        assert_eq!(kinds(&["[ ]"]), [("[ ]", "invalid.illegal.table.toml")]);
        // a header ends with its line, a quoted part left open
        assert_eq!(
            kinds(&["[\"a", "b = 1"]).last(),
            Some(&("1", "constant.numeric.integer.toml"))
        );
    }

    /// Arrays nested past the levels whose kinds are kept, and the odd blocks the scanner was
    /// once held to syntect on: the spans in order, and the top level back after them.
    #[test]
    fn deep_and_odd_input() {
        let deep = format!("a = {}1{}", "[".repeat(200), "]".repeat(200));
        let blocks = [
            deep.as_str(),
            "bad = \nbad2 = = 1\nkey\n= 1\n[]\n[[]]\n[a]b]\n[[a]b]]\n[[a]\nx = 1 trailing",
            "t = {\n a = 1,\n}\nu = { , }\nv = { a = 1, , b = 2 }\nw = {a=1}}",
            "x = \"unterminated\ny = 'also\nz = [1 2]\n\"k\\\"ey\" = 1\n\t  key\t=\t1\t",
            "s = \"\"\"\nline \\n \\u00e9 \\q\nend\"\"\" # c\nt = '''\nraw\n'''",
        ];
        for block in blocks {
            let mut lines: Vec<&'static str> = block.to_string().leak().split('\n').collect();
            lines.push("after = 1");
            let got = kinds(&lines);
            assert!(
                got.ends_with(&[
                    ("after", "entity.name.tag.toml"),
                    ("=", "punctuation.definition.key-value.toml"),
                    ("1", "constant.numeric.integer.toml"),
                ]),
                "{block:?}: {got:?}"
            );
        }
        // past the kinds kept, the levels below are taken for arrays: still in order
        kinds(&[format!("a = {}x = 1", "[{".repeat(70)).leak(), "b = 2"]);
    }
}
