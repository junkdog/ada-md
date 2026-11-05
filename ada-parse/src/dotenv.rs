//! `.env` files by a scanner on the line-format engine: bat's `DotENV` compiled by hand. A line
//! that starts with `#` is a comment; any other is `main`'s tokens (a name before its `=`,
//! `export`, a constant after the `=`, a number, a comment, the strings) from its first byte, as
//! bat's grammar reads them anywhere on the line. A `"` or `'` string runs on across lines, and
//! is all the state there is.
//!
//! `main` tests its patterns at a byte where one can start, in the grammar's order, where
//! syntect asks every pattern for its leftmost match; the bytes none can start at are passed in
//! one step ([`CLASS`]). Exact against syntect on bat's grammar (`tests/exact_lexers.rs`,
//! the `parse` fuzz target). Names' `\x{7f}-\x{ff}` is any byte past ASCII here, and `\w` ASCII
//! plus those.

use crate::line_format::{Cursor, Format, find, leads};
use crate::matcher::{Atom, Line, atoms, is_space};
use crate::prim::stops;
use crate::{Out, State};

atoms! {
    INTERPOLATED = "string.interpolated.env",
    TEMPLATE_END = "keyword.other.template.end.env",
    VARIABLE_MARK = "punctuation.definition.variable.env",
    VARIABLE_S = "variable.other.env",
    DOUBLE = "string.quoted.double.env",
    STR_END = "punctuation.definition.string.end",
    SINGLE = "string.quoted.single.env",
    ESCAPE = "constant.character.escape.env",
    TEMPLATE_BEGIN = "keyword.other.template.begin.env",
    COMMENT = "comment.line.number-sign.env",
    COMMENT_MARK = "punctuation.definition.comment.env",
    STR_BEGIN = "punctuation.definition.string.begin.env",
    ASSIGNMENT = "keyword.operator.assignment.env",
    KEYWORD = "keyword.other.env",
    CONSTANT = "constant.language.env",
    NUMERIC = "constant.numeric.env",
}

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

/// Each kind's scopes under `source.env`, by kind; kind 0 is none.
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

/// The context open at the scan's place, as the line leaves it: `main` (0, its text without a
/// scope), a `"` string, a `${…}` or `{…}` in one, a `'` string.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ctx {
    Main = 0,
    Double = 1,
    Interpolation = 2,
    Single = 3,
}

/// Lex `b`, a line without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    crate::line_format::line::<DotEnv>(b, state, out);
}

struct DotEnv;

impl Format for DotEnv {
    // `(#).*(?m:$)\n?`, the blanks before it `main`'s
    const LEAD: [u8; 256] = leads(b"#", b"");
    const COMMENT_TEXT: u16 = COMMENT;
    const COMMENT_MARK: u16 = COMMENT_MARK;
    const COMMENT_BLANKS: bool = false;

    #[inline(always)]
    fn resume(c: &mut Cursor, open: u64) -> u64 {
        let ctx = match open {
            1 => Ctx::Double,
            2 => Ctx::Interpolation,
            _ => Ctx::Single,
        };
        scan(c, 0, ctx)
    }

    /// `main` from the blank before `p`, where `\s?(export)` can start: none of `main`'s
    /// patterns starts at a blank that another blank follows.
    #[inline(always)]
    fn entry(c: &mut Cursor, p: usize) -> u64 {
        scan(c, p.saturating_sub(1), Ctx::Main)
    }
}

/// Each byte's class in `main`: a word's ([`WORD`]: `\w`, ASCII's word bytes and any byte past
/// ASCII, as Oniguruma's Unicode `\w` takes letters), one no pattern starts at ([`SKIP`]: all
/// but `\w`, `\s`, `=`, `#`, the quotes and `.`, a number's start after a word), or another.
static CLASS: [u8; 256] = {
    let keep = stops(b"=#\"'. \t\n\x0b\x0c\r");
    let mut t = [0; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        t[c] = if b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80 {
            WORD
        } else if keep[c] {
            0
        } else {
            SKIP
        };
        c += 1;
    }
    t
};
const WORD: u8 = 1;
const SKIP: u8 = 2;

/// `\w` at a byte ([`CLASS`]).
#[inline(always)]
fn is_word(c: u8) -> bool {
    CLASS[usize::from(c)] == WORD
}

/// The line from `p` in `ctx`, the cursor where the spans so far end: what it leaves open.
/// One copy, for a line's entry and a line in a string alike.
#[inline(never)]
fn scan(c: &mut Cursor, mut p: usize, mut ctx: Ctx) -> u64 {
    let b = c.b;
    let l = Line { b };
    let nl = b.len();
    while p < nl {
        match ctx {
            Ctx::Main => {
                while p < nl && CLASS[usize::from(b[p])] == SKIP {
                    p += 1;
                }
                if p == nl {
                    break;
                }
                match main_at(&l, p, c) {
                    Step::To(e) => p = e,
                    Step::Open(e, open) => {
                        p = e;
                        ctx = open;
                    }
                }
            }
            Ctx::Double => {
                // to the next byte a pattern can start at
                p = find(b, p, *b"\"${\\");
                if p == nl {
                    break;
                }
                match b[p] {
                    b'"' => {
                        c.tok(Some(DOUBLE), p, p + 1, DOUBLE_END);
                        p += 1;
                        ctx = Ctx::Main;
                    }
                    b'{' => {
                        c.tok(Some(DOUBLE), p, p + 1, TEMPLATE_BEGIN);
                        p += 1;
                        ctx = Ctx::Interpolation;
                    }
                    b'$' if l.at(p + 1) == Some(b'{') => {
                        c.tok(Some(DOUBLE), p, p + 2, TEMPLATE_BEGIN);
                        p += 2;
                        ctx = Ctx::Interpolation;
                    }
                    b'$' => match variable_at(&l, p) {
                        Some((q, e)) => {
                            c.tok(Some(DOUBLE), p, q, VARIABLE_MARK);
                            c.tok(Some(DOUBLE), q, e, VARIABLE);
                            p = e;
                        }
                        None => p += 1,
                    },
                    // `\\[nrt\\\$\"\']`
                    _ if l.is(p + 1, |c| b"nrt\\$\"'".contains(&c)) => {
                        c.tok(Some(DOUBLE), p, p + 2, ESCAPE);
                        p += 2;
                    }
                    _ => p += 1,
                }
            }
            Ctx::Interpolation => {
                if b[p] == b'}' {
                    c.tok(Some(DOUBLE), p, p + 1, TEMPLATE_END);
                    p += 1;
                    ctx = Ctx::Double;
                    continue;
                }
                // `(\$+)?([a-zA-Z_\x{7f}-\x{ff}][a-zA-Z0-9_\x{7f}-\x{ff}]*?\b)`
                let q = l.run(p, |c| c == b'$');
                match l.is(q, is_name_start).then(|| lazy_name(&l, q)).flatten() {
                    Some(e) => {
                        if q > p {
                            c.tok(Some(DOUBLE), p, q, VARIABLE_MARK);
                        }
                        c.tok(Some(DOUBLE), q, e, VARIABLE);
                        p = e;
                    }
                    None => p += 1,
                }
            }
            Ctx::Single => {
                let q = find(b, p, *b"'");
                if q == nl {
                    p = nl;
                } else {
                    c.tok(Some(SINGLE), q, q + 1, SINGLE_END);
                    p = q + 1;
                    ctx = Ctx::Main;
                }
            }
        }
    }
    let text = match ctx {
        Ctx::Main => None,
        Ctx::Double | Ctx::Interpolation => Some(DOUBLE),
        Ctx::Single => Some(SINGLE),
    };
    c.text(text, nl);
    ctx as u64
}

/// What a step in `main` did: moved on to a place, or opened a string there.
enum Step {
    To(usize),
    Open(usize, Ctx),
}

/// `\w` at `i`, the `\n` not.
fn word(l: &Line, i: usize) -> bool {
    l.is(i, |c| is_word(c) && c != b'\n')
}

/// `\b` at `i`.
fn boundary(l: &Line, i: usize) -> bool {
    (i > 0 && word(l, i - 1)) != word(l, i)
}

/// `[a-zA-Z_\x{7f}-\x{ff}]`
fn is_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x7f
}

/// `[a-zA-Z0-9_\x{7f}-\x{ff}]`
fn is_name(c: u8) -> bool {
    is_name_start(c) || c.is_ascii_digit()
}

/// `NAME*?\b` from a name's first character at `q`: the shortest run of name characters
/// that a `\b` follows.
fn lazy_name(l: &Line, q: usize) -> Option<usize> {
    let mut e = q + 1;
    loop {
        if boundary(l, e) {
            return Some(e);
        }
        if !l.is(e, is_name) {
            return None;
        }
        e += 1;
    }
}

/// `(\$+)(NAME*?\b)` at `p`: where the name starts and ends.
fn variable_at(l: &Line, p: usize) -> Option<(usize, usize)> {
    let q = l.run(p, |c| c == b'$');
    if !l.is(q, is_name_start) {
        return None;
    }
    Some((q, lazy_name(l, q)?))
}

/// Whether `b` holds `word`, a lowercase word of letters, at `i` in any case: a letter's two
/// cases differ in bit 5 alone, and no other byte sets it to one of them.
#[inline(always)]
fn folded<const N: usize>(b: &[u8], i: usize, word: &[u8; N]) -> bool {
    b.get(i..i + N).is_some_and(|w| {
        let mut d = 0;
        for k in 0..N {
            d |= (w[k] | 0x20) ^ word[k];
        }
        d == 0
    })
}

/// `export`, in any case, at `i`.
#[inline(always)]
fn export_at(b: &[u8], i: usize) -> bool {
    folded(b, i, b"export")
}

/// `true|false|null`, in any case, at `i`: where it ends.
fn constant_word(b: &[u8], i: usize) -> Option<usize> {
    let ok = match b.get(i)? | 0x20 {
        b't' => folded(b, i, b"true").then_some(4),
        b'f' => folded(b, i, b"false").then_some(5),
        b'n' => folded(b, i, b"null").then_some(4),
        _ => None,
    };
    ok.map(|n| i + n)
}

/// `main`'s patterns at `p`, in the grammar's order (a comment, the strings, `=`, a name, `export`,
/// a constant, a number); past a word that is no name, to its end, as only `export` can start
/// inside one.
#[inline]
fn main_at(l: &Line, p: usize, o: &mut Cursor) -> Step {
    let b = l.b;
    let c = b[p];
    match c {
        // `(#).*(?m:$)\n?`
        b'#' => {
            o.tok(None, p, p + 1, COMMENT_MARK);
            o.tok(None, p + 1, b.len(), COMMENT);
            return Step::To(b.len());
        }
        b'"' => {
            o.tok(None, p, p + 1, DOUBLE_BEGIN);
            return Step::Open(p + 1, Ctx::Double);
        }
        b'\'' => {
            o.tok(None, p, p + 1, SINGLE_BEGIN);
            return Step::Open(p + 1, Ctx::Single);
        }
        _ => {}
    }
    // `(?<=[\w])\s?=`
    if p > 0 && is_word(b[p - 1]) {
        if c == b'=' {
            o.tok(None, p, p + 1, ASSIGNMENT);
            return Step::To(p + 1);
        }
        if is_space(c) && b.get(p + 1) == Some(&b'=') {
            o.tok(None, p, p + 2, ASSIGNMENT);
            return Step::To(p + 2);
        }
    }
    let after_eq = p > 0 && b[p - 1] == b'=';
    if is_word(c) {
        // `([\w]+)(?=\s?\=)`
        let mut e = p + 1;
        while e < b.len() && is_word(b[e]) {
            e += 1;
        }
        if b.get(e) == Some(&b'=') || (e < b.len() && is_space(b[e]) && b.get(e + 1) == Some(&b'='))
        {
            o.tok(None, p, e, NAME);
            return Step::To(e);
        }
        // `(?i)\s?(export)`
        if export_at(b, p) {
            o.tok(None, p, p + 6, EXPORT);
            return Step::To(p + 6);
        }
        // `(?i)(?<=\=)\s?(true|false|null)`
        if after_eq && let Some(end) = constant_word(b, p) {
            o.tok(None, p, end, CONSTANT);
            return Step::To(end);
        }
        // a number's first byte in a word is a digit
        if c.is_ascii_digit()
            && boundary(l, p)
            && let Some(end) = number_at(l, p)
        {
            o.tok(None, p, end, NUMBER);
            return Step::To(end);
        }
        // inside the word only `export` can start (no `\b`, no `=` before, no space), and only
        // where the word holds all six of its letters
        let mut q = p + 1;
        while q + 6 <= e {
            q = find(&b[..e], q, *b"eE");
            if q + 6 > e {
                break;
            }
            if export_at(b, q) {
                o.tok(None, q, q + 6, EXPORT);
                return Step::To(q + 6);
            }
            q += 1;
        }
        return Step::To(e);
    }
    if is_space(c) {
        if export_at(b, p + 1) {
            o.tok(None, p, p + 7, EXPORT);
            return Step::To(p + 7);
        }
        if after_eq && let Some(end) = constant_word(b, p + 1) {
            o.tok(None, p, end, CONSTANT);
            return Step::To(end);
        }
    } else if c == b'.'
        && boundary(l, p)
        && let Some(end) = number_at(l, p)
    {
        o.tok(None, p, end, NUMBER);
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
