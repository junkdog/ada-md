//! `.env` files by a scanner: bat's `DotENV` compiled by hand. Each of its 9 contexts that can be
//! open is a loop that tests its patterns at the scan's place, in the grammar's order, on the
//! byte there, where syntect asks every pattern for its leftmost match.
//!
//! The state holds syntect's context ids. Exact against syntect on bat's grammar
//! (`tests/exact_lexers.rs`, the `parse` fuzz target), as it was against the port it replaced.
//! A span's kind is one of a fixed set ([`scopes`]).

use crate::dotenv::{
    ATOMS, DOUBLE_0, INTERPOLATION_0, MAIN_0, SINGLE_0, START, boundary, is_name_start, lazy_name,
    s,
};
use crate::matcher::{Atom, Line, is_space, is_word};
use crate::{Out, State};

const NONE: u16 = 0;
const COMMENT: u16 = 1;
const COMMENT_MARK: u16 = 2;
const ASSIGNMENT: u16 = 3;
const NAME: u16 = 4;
const EXPORT: u16 = 5;
const CONSTANT: u16 = 6;
const NUMBER: u16 = 7;
const DOUBLE: u16 = 8;
const DOUBLE_BEGIN: u16 = 9;
const DOUBLE_END: u16 = 10;
const ESCAPE: u16 = 11;
const VARIABLE_MARK: u16 = 12;
const VARIABLE: u16 = 13;
const TEMPLATE_BEGIN: u16 = 14;
const TEMPLATE_END: u16 = 15;
const SINGLE: u16 = 16;
const SINGLE_BEGIN: u16 = 17;
const SINGLE_END: u16 = 18;

/// Each kind's scopes under `source.env`, by kind.
static KINDS: [&[Atom]; 19] = [
    &[],
    &[s::COMMENT],
    &[s::COMMENT, s::COMMENT_MARK],
    &[s::ASSIGNMENT],
    &[s::VARIABLE_S],
    &[s::KEYWORD],
    &[s::CONSTANT],
    &[s::NUMERIC],
    &[s::DOUBLE],
    &[s::DOUBLE, s::STR_BEGIN],
    &[s::DOUBLE, s::STR_END],
    &[s::DOUBLE, s::ESCAPE],
    &[s::DOUBLE, s::VARIABLE_MARK, s::VARIABLE_S],
    &[s::DOUBLE, s::VARIABLE_S],
    &[s::DOUBLE, s::INTERPOLATED, s::TEMPLATE_BEGIN],
    &[s::DOUBLE, s::INTERPOLATED, s::TEMPLATE_END],
    &[s::SINGLE],
    &[s::SINGLE, s::STR_BEGIN],
    &[s::SINGLE, s::STR_END],
];

/// The scopes under `source.env` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    KINDS.get(usize::from(kind)).map_or_else(Vec::new, |k| {
        k.iter().map(|&a| ATOMS[usize::from(a)]).collect()
    })
}

/// The context open at the scan's place: the top of syntect's stack.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ctx {
    /// `main`, under `__start`; its text has no scope.
    Main,
    /// A `"` string (`#anon_main_0`).
    Double,
    /// A `${…}` or `{…}` in one (`#anon_interpolation_0`).
    Interpolation,
    /// A `'` string (`#anon_main_1`).
    Single,
}

/// A line's spans: the text before a token gets the open context's kind.
struct Emit<'a, 'b> {
    out: &'a mut Out<'b>,
    /// Where the spans so far end.
    at: usize,
}

impl Emit<'_, '_> {
    /// The text up to `s` is `text`, then `s..e` is `kind`.
    #[inline]
    fn tok(&mut self, text: u16, s: usize, e: usize, kind: u16) {
        self.text(text, s);
        self.out.push_merged(s, e, kind);
        self.at = e;
    }

    #[inline]
    fn text(&mut self, kind: u16, to: usize) {
        if kind != NONE && to > self.at {
            self.out.push_merged(self.at, to, kind);
        }
        self.at = to;
    }
}

/// Lex `b`, a line without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let l = Line { b };
    let nl = b.len();
    let mut ctx = match state.levels().last() {
        Some(&DOUBLE_0) => Ctx::Double,
        Some(&INTERPOLATION_0) => Ctx::Interpolation,
        Some(&SINGLE_0) => Ctx::Single,
        _ => Ctx::Main,
    };
    let mut o = Emit { out, at: 0 };
    let mut p = 0;
    while p < nl {
        match ctx {
            Ctx::Main => match main_at(&l, p, &mut o) {
                Step::To(e) => p = e,
                Step::Open(e, c) => {
                    p = e;
                    ctx = c;
                }
            },
            Ctx::Double => {
                // to the next byte a pattern can start at
                while p < nl && !matches!(b[p], b'"' | b'$' | b'{' | b'\\') {
                    p += 1;
                }
                if p == nl {
                    break;
                }
                match b[p] {
                    b'"' => {
                        o.tok(DOUBLE, p, p + 1, DOUBLE_END);
                        p += 1;
                        ctx = Ctx::Main;
                    }
                    b'{' => {
                        o.tok(DOUBLE, p, p + 1, TEMPLATE_BEGIN);
                        p += 1;
                        ctx = Ctx::Interpolation;
                    }
                    b'$' if l.at(p + 1) == Some(b'{') => {
                        o.tok(DOUBLE, p, p + 2, TEMPLATE_BEGIN);
                        p += 2;
                        ctx = Ctx::Interpolation;
                    }
                    b'$' => match variable_at(&l, p) {
                        Some((q, e)) => {
                            o.tok(DOUBLE, p, q, VARIABLE_MARK);
                            o.tok(DOUBLE, q, e, VARIABLE);
                            p = e;
                        }
                        None => p += 1,
                    },
                    // `\\[nrt\\\$\"\']`
                    _ if l.is(p + 1, |c| b"nrt\\$\"'".contains(&c)) => {
                        o.tok(DOUBLE, p, p + 2, ESCAPE);
                        p += 2;
                    }
                    _ => p += 1,
                }
            }
            Ctx::Interpolation => {
                if b[p] == b'}' {
                    o.tok(DOUBLE, p, p + 1, TEMPLATE_END);
                    p += 1;
                    ctx = Ctx::Double;
                    continue;
                }
                // `(\$+)?([a-zA-Z_\x{7f}-\x{ff}][a-zA-Z0-9_\x{7f}-\x{ff}]*?\b)`
                let q = l.run(p, |c| c == b'$');
                match l.is(q, is_name_start).then(|| lazy_name(&l, q)).flatten() {
                    Some(e) => {
                        if q > p {
                            o.tok(DOUBLE, p, q, VARIABLE_MARK);
                        }
                        o.tok(DOUBLE, q, e, VARIABLE);
                        p = e;
                    }
                    None => p += 1,
                }
            }
            Ctx::Single => match b[p..].iter().position(|&c| c == b'\'') {
                Some(i) => {
                    o.tok(SINGLE, p + i, p + i + 1, SINGLE_END);
                    p += i + 1;
                    ctx = Ctx::Main;
                }
                None => p = nl,
            },
        }
    }
    let text = match ctx {
        Ctx::Main => NONE,
        Ctx::Double | Ctx::Interpolation => DOUBLE,
        Ctx::Single => SINGLE,
    };
    o.text(text, nl);
    let levels: &[u8] = match ctx {
        Ctx::Main => &[START, MAIN_0],
        Ctx::Double => &[START, MAIN_0, DOUBLE_0],
        Ctx::Interpolation => &[START, MAIN_0, DOUBLE_0, INTERPOLATION_0],
        Ctx::Single => &[START, MAIN_0, SINGLE_0],
    };
    *state = State::pack(levels, &[0; 4][..levels.len()], 0, state.is_exact());
}

/// What a step in `main` did: moved on to a place, or opened a string there.
enum Step {
    To(usize),
    Open(usize, Ctx),
}

/// `(\$+)(NAME*?\b)` at `p`: where the name starts and ends.
fn variable_at(l: &Line, p: usize) -> Option<(usize, usize)> {
    let q = l.run(p, |c| c == b'$');
    if !l.is(q, is_name_start) {
        return None;
    }
    Some((q, lazy_name(l, q)?))
}

/// `export`, in any case, at `i`.
fn export_at(b: &[u8], i: usize) -> bool {
    b.get(i..i + 6)
        .is_some_and(|w| w.eq_ignore_ascii_case(b"export"))
}

/// `true|false|null`, in any case, at `i`: where it ends.
fn constant_word(b: &[u8], i: usize) -> Option<usize> {
    let w = |n: usize, word: &[u8]| {
        b.get(i..i + n)
            .is_some_and(|x| x.eq_ignore_ascii_case(word))
            .then_some(i + n)
    };
    match b.get(i)? | 0x20 {
        b't' => w(4, b"true"),
        b'f' => w(5, b"false"),
        b'n' => w(4, b"null"),
        _ => None,
    }
}

/// `main`'s patterns at `p`, in the grammar's order (a comment, the strings, `=`, a name, `export`,
/// a constant, a number); past a word that is no name, to its end, as only `export` can start
/// inside one.
#[inline]
fn main_at(l: &Line, p: usize, o: &mut Emit) -> Step {
    let b = l.b;
    let c = b[p];
    match c {
        // `(#).*(?m:$)\n?`
        b'#' => {
            o.tok(NONE, p, p + 1, COMMENT_MARK);
            o.tok(NONE, p + 1, b.len(), COMMENT);
            return Step::To(b.len());
        }
        b'"' => {
            o.tok(NONE, p, p + 1, DOUBLE_BEGIN);
            return Step::Open(p + 1, Ctx::Double);
        }
        b'\'' => {
            o.tok(NONE, p, p + 1, SINGLE_BEGIN);
            return Step::Open(p + 1, Ctx::Single);
        }
        _ => {}
    }
    // `(?<=[\w])\s?=`
    if p > 0 && is_word(b[p - 1]) {
        if c == b'=' {
            o.tok(NONE, p, p + 1, ASSIGNMENT);
            return Step::To(p + 1);
        }
        if is_space(c) && b.get(p + 1) == Some(&b'=') {
            o.tok(NONE, p, p + 2, ASSIGNMENT);
            return Step::To(p + 2);
        }
    }
    let after_eq = p > 0 && b[p - 1] == b'=';
    if is_word(c) {
        // `([\w]+)(?=\s?\=)`
        let e = p + b[p..]
            .iter()
            .position(|&c| !is_word(c))
            .unwrap_or(b.len() - p);
        if b.get(e) == Some(&b'=') || (e < b.len() && is_space(b[e]) && b.get(e + 1) == Some(&b'='))
        {
            o.tok(NONE, p, e, NAME);
            return Step::To(e);
        }
        // `(?i)\s?(export)`
        if export_at(b, p) {
            o.tok(NONE, p, p + 6, EXPORT);
            return Step::To(p + 6);
        }
        // `(?i)(?<=\=)\s?(true|false|null)`
        if after_eq && let Some(end) = constant_word(b, p) {
            o.tok(NONE, p, end, CONSTANT);
            return Step::To(end);
        }
        if boundary(l, p)
            && let Some(end) = number_at(l, p)
        {
            o.tok(NONE, p, end, NUMBER);
            return Step::To(end);
        }
        // inside the word only `export` can start: no `\b`, no `=` before, no space
        for q in p + 1..e {
            if b[q] | 0x20 == b'e' && export_at(b, q) {
                o.tok(NONE, q, q + 6, EXPORT);
                return Step::To(q + 6);
            }
        }
        return Step::To(e);
    }
    if is_space(c) {
        if export_at(b, p + 1) {
            o.tok(NONE, p, p + 7, EXPORT);
            return Step::To(p + 7);
        }
        if after_eq && let Some(end) = constant_word(b, p + 1) {
            o.tok(NONE, p, end, CONSTANT);
            return Step::To(end);
        }
    } else if c == b'.'
        && boundary(l, p)
        && let Some(end) = number_at(l, p)
    {
        o.tok(NONE, p, end, NUMBER);
        return Step::To(end);
    }
    Step::To(p + 1)
}

/// `\b((0(x|X)[0-9a-fA-F]*)|(([0-9]+\.?[0-9]*)|(\.[0-9]+))((e|E)(\+|-)?[0-9]+)?)\b` at `p`, where
/// a `\b` holds: where it ends. The regex backtracks through every shorter run of digits, but a
/// `\b` can't hold between two digits, so only a run's ends are tried, in the regex's order.
fn number_at(l: &Line, p: usize) -> Option<usize> {
    let digits = |i: usize| l.run(i, |c| c.is_ascii_digit());
    // `((e|E)(\+|-)?[0-9]+)?\b` after `j`
    let exponent = |j: usize| {
        if l.is(j, |c| c == b'e' || c == b'E') {
            let s = j + 1 + usize::from(l.is(j + 1, |c| c == b'+' || c == b'-'));
            let d = digits(s);
            if d > s && boundary(l, d) {
                return Some(d);
            }
        }
        boundary(l, j).then_some(j)
    };
    if l.at(p) == Some(b'0') && l.is(p + 1, |c| c == b'x' || c == b'X') {
        let h = l.run(p + 2, |c| c.is_ascii_hexdigit());
        if boundary(l, h) {
            return Some(h);
        }
    }
    if l.is(p, |c| c.is_ascii_digit()) {
        let d0 = digits(p);
        if l.at(d0) == Some(b'.') {
            let d = digits(d0 + 1);
            if let Some(e) = exponent(d) {
                return Some(e);
            }
            if d > d0 + 1
                && let Some(e) = exponent(d0 + 1)
            {
                return Some(e);
            }
        }
        return exponent(d0);
    }
    if l.at(p) == Some(b'.') && l.is(p + 1, |c| c.is_ascii_digit()) {
        return exponent(digits(p + 1));
    }
    None
}
