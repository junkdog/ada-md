//! TOML by a scanner (prototype, 2026-10-31): bat's `TOML` grammar compiled by hand. Each of
//! the grammar's contexts (their ids in `toml.rs`) is an arm of one `match`, and each of its
//! patterns a test of the bytes at the position, where syntect asks every pattern for its
//! leftmost match. Most contexts end in a pattern that matches anywhere (`.`,
//! `\w+|.`, the empty regex), so the leftmost match is the first pattern that matches right
//! there; only the strings and a `[[table]]` name search ahead.
//!
//! The stack holds syntect's context ids and packs into a [`State`]. Exact against syntect on
//! bat's grammar (`tests/exact_lexers.rs`, the `parse` fuzz target), as it was against the port
//! of syntect's machine it replaced. A span's kind is an outer meta scope (a key's, a table
//! name's) and a leaf, both fixed ([`scopes`]).

use crate::matcher::{Atom, DEPTH, Line, MAX_PUSH, is_blank, is_space, is_word};
use crate::toml::{
    ARRAY_SEP, ARRAY_SEP_0, ATOMS, BASIC_STRING_0, DATA_TYPES, Digit, INLINE_KEYVAL_SEP,
    INLINE_KEYVALS, INLINE_KEYVALS_0, INLINE_TABLE_0, KEY, KEY_SIMPLE, KEYVAL_SEP, MAIN_0,
    MAYBE_ARRAY_COMMENTS, MAYBE_COMMENT_EOL, MAYBE_DOT_KEY, MAYBE_DOT_TABLE, MAYBE_EMPTY_ARRAY,
    START, STRING_0, STRING_1, TABLE_ARRAY_NAME, TABLE_NAME, TABLE_NAME_SIMPLE, date, digits_from,
    fraction, int_part, is_bare, is_key_start, offset, s, sign, time, word_end, word_start,
};
use crate::{Out, State};

/// The meta scope a span sits in, by the context below it: none, a key's, a table name's, an
/// array of tables' name's. A kind is `outer << 6 | leaf`.
const OUTERS: [&[Atom]; 4] = [
    &[],
    &[s::META_KEY],
    &[s::META_TABLE],
    &[s::META_TABLE_ARRAY],
];
const NO_OUTER: u16 = 0;
const IN_KEY: u16 = 1;
const IN_TABLE: u16 = 2;
const IN_TABLE_ARRAY: u16 = 3;

/// The leaves: the scopes under the outer one.
mod leaf {
    pub const NONE: u16 = 0;
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
    pub const BAD_TABLE_END: u16 = 40;
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

/// Lex `b`, a line without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let l = Line { b };
    let mut scan = Scan {
        nl: b.len(),
        stack: [0; DEPTH],
        depth: 0,
        dropped: 0,
        exact: state.is_exact(),
        outer: NO_OUTER,
        out,
    };
    match state.levels() {
        [] => {
            scan.stack[0] = START;
            scan.depth = 1;
        }
        levels => {
            scan.stack[..levels.len()].copy_from_slice(levels);
            scan.depth = levels.len();
            // a table name left open (an unterminated quoted part) keeps its meta scope
            scan.outer = levels.iter().fold(NO_OUTER, |outer, &c| match c {
                KEY => IN_KEY,
                TABLE_NAME => IN_TABLE,
                TABLE_ARRAY_NAME => IN_TABLE_ARRAY,
                _ => outer,
            });
        }
    }
    scan.run(&l);
    static NO_WITHS: [u8; DEPTH] = [0; DEPTH];
    *state = State::pack(
        &scan.stack[..scan.depth],
        &NO_WITHS[..scan.depth],
        0,
        scan.exact,
    );
}

struct Scan<'b, 'c> {
    /// Where the `\n` is; positions run to `nl + 1`, past it.
    nl: usize,
    stack: [u8; DEPTH],
    depth: usize,
    /// Levels dropped from the bottom, past [`DEPTH`], still counted toward syntect's push limit.
    dropped: usize,
    exact: bool,
    outer: u16,
    out: &'b mut Out<'c>,
}

impl Scan<'_, '_> {
    /// Bytes `s..e` (never past the `\n`) are `leaf` in the current outer scope.
    fn put(&mut self, s: usize, e: usize, leaf: u16) {
        let kind = self.outer << 6 | leaf;
        let e = e.min(self.nl);
        if kind != 0 && e > s {
            self.out.push_merged(s, e, kind);
        }
    }

    fn top(&self) -> u8 {
        self.stack[self.depth - 1]
    }

    /// Whether syntect would push here (it doesn't past 100 levels; a `set` it always does).
    fn can_push(&self) -> bool {
        self.depth + self.dropped < MAX_PUSH
    }

    fn push(&mut self, refs: &[u8]) {
        for &r in refs {
            if self.depth == DEPTH {
                self.stack.copy_within(1.., 0);
                self.depth -= 1;
                self.dropped += 1;
                self.exact = false;
            }
            self.stack[self.depth] = r;
            self.depth += 1;
        }
    }

    fn set(&mut self, refs: &[u8]) {
        self.depth -= 1;
        self.push(refs);
    }

    fn pop(&mut self) {
        self.depth -= 1;
    }

    fn run(&mut self, l: &Line) {
        let mut p = 0;
        while self.depth > 0 {
            match self.step(l, p) {
                Some(q) => p = q,
                None => return,
            }
        }
    }

    /// One match of the top context at `p`: where the next starts, or `None` when nothing
    /// matches before the line ends.
    fn step(&mut self, l: &Line, p: usize) -> Option<usize> {
        let nl = self.nl;
        let at = |i: usize| l.at(i);
        let blanks = |i: usize| l.run(i, is_blank);
        match self.top() {
            START => {
                if !self.can_push() {
                    return None;
                }
                self.push(&[MAIN_0]);
                Some(p)
            }
            MAIN_0 => {
                if p == 0 {
                    let q = blanks(0);
                    if q > 0 {
                        return Some(q);
                    }
                }
                if let Some(q) = self.comment(l, p) {
                    return Some(q);
                }
                if let Some(e) = empty_table(l, p) {
                    self.put(p, e, leaf::BAD_TABLE);
                    return Some(e);
                }
                if at(p) == Some(b'[') && self.can_push() {
                    let (opened, ctx, outer, begin) = match at(p + 1) == Some(b'[') {
                        true => (2, TABLE_ARRAY_NAME, IN_TABLE_ARRAY, leaf::TABLE_ARRAY_BEGIN),
                        false => (1, TABLE_NAME, IN_TABLE, leaf::TABLE_BEGIN),
                    };
                    self.put(p, p + opened, begin);
                    self.push(&[MAYBE_COMMENT_EOL, ctx]);
                    self.outer = outer;
                    return Some(blanks(p + opened));
                }
                if l.is(p, is_key_start) && self.can_push() {
                    self.push(&[KEYVAL_SEP, KEY]);
                    self.outer = IN_KEY;
                    return Some(p);
                }
                if p < nl {
                    self.put(p, nl, leaf::ILLEGAL);
                    return Some(nl);
                }
                None
            }
            KEYVAL_SEP => {
                let q = blanks(p);
                if q < nl && at(q) == Some(b'=') {
                    self.put(q, q + 1, leaf::KEY_VALUE);
                    self.set(&[MAYBE_COMMENT_EOL, DATA_TYPES]);
                    return Some(blanks(q + 1));
                }
                self.pop();
                if p > nl || q == nl {
                    return Some(p.max(nl));
                }
                self.put(p, nl, leaf::BAD_KEYVAL);
                Some(nl)
            }
            KEY => {
                if l.is(p, is_key_start) && self.can_push() {
                    self.push(&[MAYBE_DOT_KEY, KEY_SIMPLE]);
                    return Some(p);
                }
                let q = blanks(p);
                // `(?=[ \t]*=)`, or `$` past the `\n`
                let e = if (q < nl && at(q) == Some(b'=')) || p > nl {
                    p
                } else if q == nl {
                    self.put(p, nl, leaf::NONE);
                    nl
                } else {
                    let e = p + l.char_len(p).unwrap_or(1);
                    self.put(p, e, leaf::BAD_KEY);
                    e
                };
                self.pop();
                self.outer = NO_OUTER;
                Some(e)
            }
            ctx @ (MAYBE_DOT_KEY | MAYBE_DOT_TABLE) => {
                let (dot, bad, close, simple) = match ctx {
                    MAYBE_DOT_KEY => (leaf::KEY_DOT, leaf::BAD_KEY, b'=', KEY_SIMPLE),
                    _ => (leaf::TABLE_DOT, leaf::BAD_TABLE, b']', TABLE_NAME_SIMPLE),
                };
                let q = blanks(p);
                if q < nl && at(q) == Some(b'.') && self.can_push() {
                    let r = blanks(q + 1);
                    if l.is(r, is_key_start) {
                        self.put(p, q, leaf::NONE);
                        self.put(q, q + 1, dot);
                        self.put(q + 1, r, leaf::NONE);
                        self.push(&[simple]);
                        return Some(r);
                    }
                }
                self.pop();
                if l.eol(q) || at(q) == Some(close) {
                    return Some(p);
                }
                let e = p + l.char_len(p).unwrap_or(1);
                self.put(p, e, bad);
                Some(e)
            }
            ctx @ (KEY_SIMPLE | TABLE_NAME_SIMPLE) => {
                let (name, bad) = match ctx {
                    KEY_SIMPLE => (leaf::KEY, leaf::BAD_KEY),
                    _ => (leaf::TABLE_NAME, leaf::BAD_TABLE),
                };
                match at(p) {
                    Some(c) if is_bare(c) => {
                        let e = l.run(p, is_bare);
                        self.put(p, e, name);
                        self.pop();
                        Some(e)
                    }
                    Some(b'"') => {
                        self.put(p, p + 1, leaf::BASIC + BEGIN);
                        self.set(&[BASIC_STRING_0]);
                        Some(p + 1)
                    }
                    _ => {
                        if let Some(e) = self.literal(l, p) {
                            self.pop();
                            return Some(e);
                        }
                        if p >= nl {
                            return None;
                        }
                        // a key ends at a space, a table name's bad byte is one character;
                        // either context starts on a key's first byte, never a space
                        let e = match ctx {
                            KEY_SIMPLE => l.run(p, |c| !is_space(c)),
                            _ => p + l.char_len(p).unwrap_or(1),
                        };
                        self.put(p, e, bad);
                        self.pop();
                        Some(e)
                    }
                }
            }
            BASIC_STRING_0 => {
                let (e, closed) =
                    self.escapes(l, p, leaf::BASIC, |b, k| (b[k] == b'"').then_some(k + 1));
                self.pop();
                // unterminated, its `\n` invalid: the line goes on past it
                Some(if closed { e } else { nl + 1 })
            }
            STRING_1 => {
                let (e, closed) = self.escapes(l, p, leaf::TRIPLE_BASIC, |b, k| {
                    b[k..].starts_with(b"\"\"\"").then_some(k + 3)
                });
                if !closed {
                    return None;
                }
                self.pop();
                Some(e)
            }
            STRING_0 => {
                let mut i = p;
                while let Some(k) = find(l.b, i, |c| c == b'\'') {
                    if l.b[k..].starts_with(b"'''") {
                        self.put(p, k, leaf::TRIPLE_LITERAL + TEXT);
                        self.put(k, k + 3, leaf::TRIPLE_LITERAL + END);
                        self.pop();
                        return Some(k + 3);
                    }
                    i = k + 1;
                }
                self.put(p, nl, leaf::TRIPLE_LITERAL + TEXT);
                None
            }
            TABLE_NAME => {
                let q = blanks(p);
                if q < nl && at(q) == Some(b']') {
                    self.outer = NO_OUTER;
                    self.put(q, q + 1, leaf::TABLE_END);
                    self.pop();
                    return Some(q + 1);
                }
                if l.is(p, is_key_start) && self.can_push() {
                    self.push(&[MAYBE_DOT_TABLE, TABLE_NAME_SIMPLE]);
                    return Some(p);
                }
                if p > nl {
                    return None;
                }
                self.junk(l, p)
            }
            TABLE_ARRAY_NAME => {
                // leftmost of `[ \t]*(\]\])`, a key's first byte, `[^\]]+\]?`; ties in that order
                let close = (p..nl.saturating_sub(1))
                    .find(|&k| l.b[k] == b']' && l.b[k + 1] == b']')
                    .map(|k| (back_blanks(l.b, p, k), k));
                let key = find(l.b, p, is_key_start).filter(|_| self.can_push());
                let junk = (p..=nl).find(|&i| at(i) != Some(b']'));
                let start = [close.map(|c| c.0), key, junk].into_iter().flatten().min();
                let Some(start) = start else {
                    self.put(p, nl, leaf::NONE);
                    return None;
                };
                self.put(p, start, leaf::NONE);
                if let Some((s, k)) = close
                    && s == start
                {
                    self.outer = NO_OUTER;
                    self.put(k, k + 2, leaf::TABLE_ARRAY_END);
                    self.pop();
                    return Some(k + 2);
                }
                if key == Some(start) {
                    self.push(&[MAYBE_DOT_TABLE, TABLE_NAME_SIMPLE]);
                    return Some(start);
                }
                self.junk(l, start)
            }
            MAYBE_COMMENT_EOL => {
                if let Some(q) = self.comment(l, p) {
                    return Some(q);
                }
                self.pop();
                if p >= nl {
                    return Some(p);
                }
                self.put(p, nl, leaf::BAD_TRAILING);
                Some(nl)
            }
            DATA_TYPES => self.value(l, p),
            MAYBE_ARRAY_COMMENTS => {
                if let Some(q) = self.ws_or_comment(l, p) {
                    return Some(q);
                }
                if p < nl {
                    self.pop();
                    return Some(p);
                }
                None
            }
            MAYBE_EMPTY_ARRAY => {
                if at(p) == Some(b']') && p < nl {
                    self.put(p, p + 1, leaf::ARRAY_END);
                    self.pop();
                    return Some(p + 1);
                }
                self.set(&[ARRAY_SEP, DATA_TYPES]);
                Some(p)
            }
            ctx @ (ARRAY_SEP | ARRAY_SEP_0) => {
                if let Some(q) = self.ws_or_comment(l, p) {
                    return Some(q);
                }
                if p >= nl {
                    return None;
                }
                match at(p) {
                    Some(b']') => {
                        self.put(p, p + 1, leaf::ARRAY_END);
                        self.pop();
                        Some(p + 1)
                    }
                    _ if ctx == ARRAY_SEP_0 => {
                        self.set(&[ARRAY_SEP, DATA_TYPES]);
                        Some(p)
                    }
                    Some(b',') => {
                        self.put(p, p + 1, leaf::ARRAY_SEP);
                        self.set(&[ARRAY_SEP_0]);
                        Some(p + 1)
                    }
                    _ => {
                        let e = word_or_char(l, p);
                        self.put(p, e, leaf::BAD_ARRAY);
                        Some(e)
                    }
                }
            }
            ctx @ (INLINE_TABLE_0 | INLINE_KEYVALS_0) => {
                let q = blanks(p);
                if q > p {
                    return Some(q);
                }
                if at(p) == Some(b'}') && p < nl {
                    let end = match ctx {
                        INLINE_TABLE_0 => leaf::INLINE_END,
                        _ => leaf::BAD_TABLE_END,
                    };
                    self.put(p, p + 1, end);
                    self.pop();
                    return Some(p + 1);
                }
                self.set(&[INLINE_KEYVAL_SEP, KEY]);
                self.outer = IN_KEY;
                Some(p)
            }
            INLINE_KEYVAL_SEP => {
                let q = blanks(p);
                if q < nl && at(q) == Some(b'=') {
                    self.put(q, q + 1, leaf::KEY_VALUE);
                    self.set(&[INLINE_KEYVALS, DATA_TYPES]);
                    return Some(blanks(q + 1));
                }
                if p < nl {
                    self.pop();
                    return Some(p);
                }
                None
            }
            INLINE_KEYVALS => {
                let q = blanks(p);
                if q > p {
                    return Some(q);
                }
                if p >= nl {
                    return None;
                }
                match at(p) {
                    Some(b'}') => {
                        self.put(p, p + 1, leaf::INLINE_END);
                        self.pop();
                        Some(p + 1)
                    }
                    Some(b',') => {
                        self.put(p, p + 1, leaf::INLINE_SEP);
                        self.set(&[INLINE_KEYVALS_0]);
                        Some(p + 1)
                    }
                    _ => {
                        self.pop();
                        Some(p)
                    }
                }
            }
            // the rest are only ever included
            _ => None,
        }
    }

    /// A value, `data-types`: the first of its patterns that matches at `p`.
    fn value(&mut self, l: &Line, p: usize) -> Option<usize> {
        let nl = self.nl;
        let b = l.b;
        let opened = |s: &mut Self, n: usize, kind: u16, ctx: &[u8]| {
            s.put(p, p + n, kind);
            s.set(ctx);
            Some(p + n)
        };
        match l.at(p) {
            Some(b'{') => return opened(self, 1, leaf::INLINE_BEGIN, &[INLINE_TABLE_0]),
            Some(b'[') => {
                let ctx = &[MAYBE_EMPTY_ARRAY, MAYBE_ARRAY_COMMENTS];
                return opened(self, 1, leaf::ARRAY_BEGIN, ctx);
            }
            Some(b'\'') if b[p..].starts_with(b"'''") => {
                return opened(self, 3, leaf::TRIPLE_LITERAL + BEGIN, &[STRING_0]);
            }
            Some(b'"') if b[p..].starts_with(b"\"\"\"") => {
                return opened(self, 3, leaf::TRIPLE_BASIC + BEGIN, &[STRING_1]);
            }
            Some(b'"') => return opened(self, 1, leaf::BASIC + BEGIN, &[BASIC_STRING_0]),
            _ => {}
        }
        let e = if let Some(e) = self.literal(l, p) {
            e
        } else if let Some(e) = datetime(l, p) {
            self.put(p, e, leaf::DATETIME);
            e
        } else if let Some((q, e)) = float(l, p).or_else(|| inf_nan(l, p)) {
            self.put(p, q, leaf::FLOAT_SIGN);
            self.put(q, e, leaf::FLOAT);
            e
        } else if let Some((q, e)) = integer(l, p) {
            self.put(p, q, leaf::INTEGER_SIGN);
            self.put(q, e, leaf::INTEGER);
            e
        } else if b[p.min(nl)..].starts_with(b"true") || b[p.min(nl)..].starts_with(b"false") {
            let e = p + if b[p] == b't' { 4 } else { 5 };
            self.put(p, e, leaf::BOOL);
            e
        } else if p > nl {
            p
        } else if l.run(p, is_blank) == nl {
            nl
        } else {
            let e = word_or_char(l, p);
            self.put(p, e, leaf::BAD_VALUE);
            e
        };
        self.pop();
        Some(e)
    }

    /// `(')[^']*(')` at `p`.
    fn literal(&mut self, l: &Line, p: usize) -> Option<usize> {
        if l.at(p) != Some(b'\'') || p >= self.nl {
            return None;
        }
        let j = find(l.b, p + 1, |c| c == b'\'')?;
        self.put(p, p + 1, leaf::LITERAL + BEGIN);
        self.put(p + 1, j, leaf::LITERAL + TEXT);
        self.put(j, j + 1, leaf::LITERAL + END);
        Some(j + 1)
    }

    /// A basic string's text from `p`, its escapes marked, to where `close` takes a quote it
    /// meets: where that ends and true, or the line's end and false.
    fn escapes(
        &mut self,
        l: &Line,
        p: usize,
        base: u16,
        close: impl Fn(&[u8], usize) -> Option<usize>,
    ) -> (usize, bool) {
        let b = l.b;
        let mut from = p;
        let mut i = p;
        while let Some(k) = find(b, i, |c| c == b'"' || c == b'\\') {
            if b[k] == b'"' {
                if let Some(e) = close(b, k) {
                    self.put(from, k, base + TEXT);
                    self.put(k, e, base + END);
                    return (e, true);
                }
                i = k + 1;
                continue;
            }
            let n = escape(l, k);
            let e = match n {
                0 => l.char_len(k + 1).map(|c| (k + 1 + c, base + BAD_ESCAPE)),
                n => Some((k + n, base + ESCAPE)),
            };
            match e {
                Some((e, kind)) => {
                    self.put(from, k, base + TEXT);
                    self.put(k, e, kind);
                    from = e;
                    i = e;
                }
                None => i = k + 1,
            }
        }
        self.put(from, self.nl, base + TEXT);
        (self.nl, false)
    }

    /// `[ \t]*((#).*)` at `p`: the line's end.
    fn comment(&mut self, l: &Line, p: usize) -> Option<usize> {
        let q = l.run(p, is_blank);
        if q >= self.nl || l.b[q] != b'#' {
            return None;
        }
        self.put(p, q, leaf::NONE);
        self.put(q, q + 1, leaf::COMMENT_MARK);
        self.put(q + 1, self.nl, leaf::COMMENT);
        Some(self.nl)
    }

    /// `([ \t\n])*` when it takes something, else a comment, at `p`.
    fn ws_or_comment(&mut self, l: &Line, p: usize) -> Option<usize> {
        let q = l.run(p, |c| matches!(c, b' ' | b'\t' | b'\n'));
        if q > p {
            return Some(q);
        }
        self.comment(l, p)
    }

    /// A table name's `[^\]]+\]?` at `p`, the `\n` among what it takes; it ends the name.
    fn junk(&mut self, l: &Line, p: usize) -> Option<usize> {
        let q = l.run(p, |c| c != b']');
        let e = q + usize::from(l.at(q) == Some(b']'));
        self.outer = NO_OUTER;
        self.put(p, e, leaf::BAD_TABLE);
        self.pop();
        Some(e)
    }
}

/// The first byte at or after `from` in `set`.
fn find(b: &[u8], from: usize, set: impl Fn(u8) -> bool) -> Option<usize> {
    let hay = b.get(from..)?;
    hay.iter().position(|&c| set(c)).map(|i| from + i)
}

/// Where the `[ \t]` before `at` start, not before `from`.
fn back_blanks(b: &[u8], from: usize, mut at: usize) -> usize {
    while at > from && is_blank(b[at - 1]) {
        at -= 1;
    }
    at
}

/// `\w+|.` at `p`, before the `\n`.
fn word_or_char(l: &Line, p: usize) -> usize {
    match is_word(l.b[p]) {
        true => l.run(p, |c| is_word(c) && c != b'\n'),
        false => p + l.char_len(p).unwrap_or(1),
    }
}

/// `\[?\[[ \t]*\]\]?` at `p`.
fn empty_table(l: &Line, p: usize) -> Option<usize> {
    if l.at(p) != Some(b'[') {
        return None;
    }
    let close = |at: usize| {
        let q = l.run(at, is_blank);
        (l.at(q) == Some(b']')).then(|| q + 1 + usize::from(l.at(q + 1) == Some(b']')))
    };
    match l.at(p + 1) {
        Some(b'[') => close(p + 2).or_else(|| close(p + 1)),
        _ => close(p + 1),
    }
}

/// The length of `\\(?:[btnfr"\\]|u\h{4}|U\h{8})` at `k`, or 0.
fn escape(l: &Line, k: usize) -> usize {
    let hex = |at: usize, n: usize| (at..at + n).all(|i| l.is(i, |c| c.is_ascii_hexdigit()));
    match l.at(k + 1) {
        Some(b'b' | b't' | b'n' | b'f' | b'r' | b'"' | b'\\') => 2,
        Some(b'u') if hex(k + 2, 4) => 6,
        Some(b'U') if hex(k + 2, 8) => 10,
        _ => 0,
    }
}

/// The date-time regex at `p`.
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

/// The float regex at `p`: where its sign ends, and it.
fn float(l: &Line, p: usize) -> Option<(usize, usize)> {
    if !word_start(l, p) {
        return None;
    }
    let q = sign(l, p);
    int_part(l, q, |r| fraction(l, r)).map(|e| (q, e))
}

/// `(?<!\w)([+-])?(inf|nan)(?!\w)` at `p`.
fn inf_nan(l: &Line, p: usize) -> Option<(usize, usize)> {
    if !word_start(l, p) {
        return None;
    }
    let q = sign(l, p);
    let word = l.b.get(q..q + 3)?;
    ((word == b"inf" || word == b"nan") && word_end(l, q + 3)).then_some((q, q + 3))
}

/// The integer regex at `p` (`toml::integer`'s branches): where its sign ends, and it.
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
