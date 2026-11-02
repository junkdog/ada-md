//! `.env` files, in place of bat's `DotENV`: its 9 contexts on the [machine](crate::machine).
//! Quoted values run on across lines.
//!
//! The contexts keep syntect's ids (the order the probe's `dump DotENV` lists them in). Names'
//! `\x{7f}-\x{ff}` is any byte past ASCII here, and `\w` ASCII plus those.

use crate::machine::{
    Ctx, Grammar, It::I, It::P, Kinds, Line, M, Pat, atoms, empty, first, is_space, is_word, lit1,
    protos, slots,
};

pub(crate) const INTERPOLATION_0: u8 = 0;
pub(crate) const DOUBLE_0: u8 = 1;
pub(crate) const SINGLE_0: u8 = 2;
pub(crate) const MAIN_0: u8 = 3;
pub(crate) const START: u8 = 4;
const ESCAPES: u8 = 5;
const INTERPOLATION: u8 = 6;
const MAIN: u8 = 7;
const VARIABLE: u8 = 8;

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

const fn ctx(items: &'static [crate::machine::It]) -> Ctx {
    Ctx::new(items)
}

static CTXS: [Ctx; 9] = [
    // #anon_interpolation_0
    ctx(&[
        P(Pat::new(close_brace)
            .caps(&[(1, &[s::INTERPOLATED, s::TEMPLATE_END])])
            .pop()),
        P(Pat::new(maybe_variable).caps(&[
            (1, &[s::VARIABLE_MARK, s::VARIABLE_S]),
            (2, &[s::VARIABLE_S]),
        ])),
    ]),
    // #anon_main_0
    ctx(&[
        P(Pat::new(dquote).caps(&[(1, &[s::STR_END])]).pop()),
        I(INTERPOLATION),
        I(VARIABLE),
        I(ESCAPES),
    ])
    .meta(&[s::DOUBLE]),
    // #anon_main_1
    ctx(&[P(Pat::new(squote).caps(&[(1, &[s::STR_END])]).pop())]).meta(&[s::SINGLE]),
    // __main
    ctx(&[I(MAIN)]),
    // __start: its `source.env` is the base scope
    ctx(&[P(Pat::new(empty).push(&[MAIN_0]))]).no_proto(),
    // escape-characters
    ctx(&[P(Pat::new(escape).sc(&[s::ESCAPE]))]),
    // interpolation
    ctx(&[P(Pat::new(interpolation_open)
        .caps(&[(1, &[s::INTERPOLATED, s::TEMPLATE_BEGIN])])
        .push(&[INTERPOLATION_0]))]),
    // main: only ever included, so its `source.env` never applies
    ctx(&[
        P(Pat::new(comment)
            .sc(&[s::COMMENT])
            .caps(&[(1, &[s::COMMENT_MARK])])),
        P(Pat::new(dquote)
            .caps(&[(1, &[s::STR_BEGIN])])
            .push(&[DOUBLE_0])),
        P(Pat::new(squote)
            .caps(&[(1, &[s::STR_BEGIN])])
            .push(&[SINGLE_0])),
        P(Pat::new(assignment).sc(&[s::ASSIGNMENT])),
        P(Pat::new(name).sc(&[s::VARIABLE_S])),
        P(Pat::new(export).sc(&[s::KEYWORD])),
        P(Pat::new(constant).sc(&[s::CONSTANT])),
        P(Pat::new(number).sc(&[s::NUMERIC])),
    ]),
    // variable
    ctx(&[P(Pat::new(variable).caps(&[
        (1, &[s::VARIABLE_MARK, s::VARIABLE_S]),
        (2, &[s::VARIABLE_S]),
    ]))]),
];

static SLOTS: [u16; 9] = slots(&CTXS);
static PROTOS: [bool; 9] = protos(&CTXS, None);
static KINDS: Kinds = Kinds::new();

pub static GRAMMAR: Grammar = Grammar {
    ctxs: &CTXS,
    start: &[START],
    atoms: ATOMS,
    slots: &SLOTS,
    proto: None,
    protos: &PROTOS,
    kinds: &KINDS,
};

// the matchers: each finds its regex's leftmost match at or after `from`

/// `\w` at `i`, the `\n` not.
pub(crate) fn word(l: &Line, i: usize) -> bool {
    l.is(i, |c| is_word(c) && c != b'\n')
}

/// `\b` at `i`.
pub(crate) fn boundary(l: &Line, i: usize) -> bool {
    (i > 0 && word(l, i - 1)) != word(l, i)
}

/// `[a-zA-Z_\x{7f}-\x{ff}]`
pub(crate) fn is_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x7f
}

/// `[a-zA-Z0-9_\x{7f}-\x{ff}]`
fn is_name(c: u8) -> bool {
    is_name_start(c) || c.is_ascii_digit()
}

/// `NAME*?\b` from a name's first character at `q`: the shortest run of name characters
/// that a `\b` follows.
pub(crate) fn lazy_name(l: &Line, q: usize) -> Option<usize> {
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

/// `(\$+)?NAME*?\b`, or with `dollar` `(\$+)NAME*?\b`
fn dollar_name(l: &Line, from: usize, dollar: bool) -> Option<M> {
    l.find(from, |p| {
        let q = l.run(p, |c| c == b'$');
        if (dollar && q == p) || !l.is(q, is_name_start) {
            return None;
        }
        let e = lazy_name(l, q)?;
        let m = M::new(p, e).g(2, q, e);
        Some(if q > p { m.g(1, p, q) } else { m })
    })
}

/// `(?x)(\$+)?([a-zA-Z_\x{7f}-\x{ff}][a-zA-Z0-9_\x{7f}-\x{ff}]*?\b)`
fn maybe_variable(l: &Line, from: usize) -> Option<M> {
    dollar_name(l, from, false)
}

/// `(?x)(\$+)([a-zA-Z_\x{7f}-\x{ff}][a-zA-Z0-9_\x{7f}-\x{ff}]*?\b)`
fn variable(l: &Line, from: usize) -> Option<M> {
    dollar_name(l, from, true)
}

/// `(\})`
fn close_brace(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'}').map(|m| m.g(1, m.s as usize, m.e as usize))
}

/// `(\")`
fn dquote(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'"').map(|m| m.g(1, m.s as usize, m.e as usize))
}

/// `(\')`
fn squote(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'\'').map(|m| m.g(1, m.s as usize, m.e as usize))
}

/// `\\[nrt\\\$\"\']`
fn escape(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, b'\\') {
        if l.is(k + 1, |c| b"nrt\\$\"'".contains(&c)) {
            return Some(M::new(k, k + 2));
        }
        at = k + 1;
    }
    None
}

/// `(\$\{|\{)`
fn interpolation_open(l: &Line, from: usize) -> Option<M> {
    let k = first(l, from, b'{')?;
    let s = if k > from && l.b[k - 1] == b'$' {
        k - 1
    } else {
        k
    };
    Some(M::new(s, k + 1).g(1, s, k + 1))
}

/// `(#).*(?m:$)\n?`
fn comment(l: &Line, from: usize) -> Option<M> {
    let k = first(l, from, b'#')?;
    Some(M::new(k, l.end()).g(1, k, k + 1))
}

/// `(?<=[\w])\s?=`
fn assignment(l: &Line, from: usize) -> Option<M> {
    l.find(from, |p| {
        if p == 0 || !word(l, p - 1) {
            return None;
        }
        if l.is(p, is_space) && l.at(p + 1) == Some(b'=') {
            return Some(M::new(p, p + 2));
        }
        (l.at(p) == Some(b'=')).then(|| M::new(p, p + 1))
    })
}

/// `([\w]+)(?=\s?\=)`: every start in a run ends where it does, so a run that fails is
/// skipped whole.
fn name(l: &Line, from: usize) -> Option<M> {
    let mut p = from;
    while p < l.nl() {
        if !word(l, p) {
            p += 1;
            continue;
        }
        let e = l.run(p, |c| is_word(c) && c != b'\n');
        let eq = |i: usize| l.at(i) == Some(b'=');
        if eq(e) || (l.is(e, is_space) && eq(e + 1)) {
            return Some(M::new(p, e).g(1, p, e));
        }
        p = e;
    }
    None
}

/// `(?i)\s?(export)`
fn export(l: &Line, from: usize) -> Option<M> {
    let at = |i: usize| {
        l.b.get(i..i + 6)
            .is_some_and(|w| w.eq_ignore_ascii_case(b"export"))
    };
    l.find(from, |p| {
        if l.is(p, is_space) && at(p + 1) {
            return Some(M::new(p, p + 7).g(1, p + 1, p + 7));
        }
        at(p).then(|| M::new(p, p + 6).g(1, p, p + 6))
    })
}

/// `(?i)(?<=\=)\s?(true|false|null)`
fn constant(l: &Line, from: usize) -> Option<M> {
    let word_at = |i: usize| {
        [&b"true"[..], b"false", b"null"]
            .iter()
            .find(|w| {
                l.b.get(i..i + w.len())
                    .is_some_and(|x| x.eq_ignore_ascii_case(w))
            })
            .map(|w| i + w.len())
    };
    l.find(from, |p| {
        if p == 0 || l.b.get(p - 1) != Some(&b'=') {
            return None;
        }
        if l.is(p, is_space)
            && let Some(e) = word_at(p + 1)
        {
            return Some(M::new(p, e).g(1, p + 1, e));
        }
        word_at(p).map(|e| M::new(p, e).g(1, p, e))
    })
}

/// `\b((0(x|X)[0-9a-fA-F]*)|(([0-9]+\.?[0-9]*)|(\.[0-9]+))((e|E)(\+|-)?[0-9]+)?)\b`, each
/// alternative's ends tried longest first until a `\b` follows one.
fn number(l: &Line, from: usize) -> Option<M> {
    let digit = |i: usize| l.is(i, |c| c.is_ascii_digit());
    let digits = |i: usize| l.run(i, |c| c.is_ascii_digit());
    // `((e|E)(\+|-)?[0-9]+)?` after `at`: its ends, longest first, then none
    let exponents = |at: usize| {
        let mut ends = Vec::new();
        if l.is(at, |c| c == b'e' || c == b'E') {
            for s in [
                at + 1 + usize::from(l.is(at + 1, |c| c == b'+' || c == b'-')),
                at + 1,
            ] {
                let d = digits(s);
                ends.extend((s + 1..=d).rev());
                if s == at + 1 {
                    break;
                }
            }
        }
        ends.push(at);
        ends
    };
    l.find(from, |p| {
        if !boundary(l, p) {
            return None;
        }
        if l.at(p) == Some(b'0') && l.is(p + 1, |c| c == b'x' || c == b'X') {
            let h = l.run(p + 2, |c| c.is_ascii_hexdigit());
            if let Some(e) = (p + 2..=h).rev().find(|&e| boundary(l, e)) {
                return Some(M::new(p, e));
            }
        }
        // `[0-9]+\.?[0-9]*`
        if digit(p) {
            for i in (p + 1..=digits(p)).rev() {
                let dots: &[usize] = if l.at(i) == Some(b'.') {
                    &[i + 1, i]
                } else {
                    &[i]
                };
                for &d in dots {
                    for j in (d..=digits(d)).rev() {
                        if let Some(e) = exponents(j).into_iter().find(|&e| boundary(l, e)) {
                            return Some(M::new(p, e));
                        }
                    }
                }
            }
        }
        // `\.[0-9]+`
        if l.at(p) == Some(b'.') && digit(p + 1) {
            for j in (p + 2..=digits(p + 1)).rev() {
                if let Some(e) = exponents(j).into_iter().find(|&e| boundary(l, e)) {
                    return Some(M::new(p, e));
                }
            }
        }
        None
    })
}
