//! Java properties files, in place of bat's `Java Properties`: its 10 contexts on the
//! [machine](crate::machine). A line is a comment or a key and its value; a value (or key)
//! ending in `\` runs on into the next line.
//!
//! The contexts keep syntect's ids (the order the probe's `dump "Java Properties"` lists them
//! in).

use crate::machine::{
    Ctx, Grammar, It::I, It::P, Kinds, Line, M, Pat, atoms, empty, first, is_space, non_space,
    protos, slots,
};

const MAIN_0: u8 = 0;
const VALUE_0: u8 = 1;
const PROPERTY_0: u8 = 2;
const PROPERTY_1: u8 = 3;
const MAIN_1: u8 = 4;
const START: u8 = 5;
const BACKSLASH: u8 = 6;
const MAIN: u8 = 7;
const PROPERTY: u8 = 8;
const PROPERTY_VALUE: u8 = 9;

atoms! {
    PROPERTY_META = "meta.property.java-props",
    VALUE = "string.unquoted.java-props",
    SEPARATOR = "punctuation.separator.java-props",
    CONTINUATION = "punctuation.separator.continuation.java-props",
    UNICODE = "constant.character.escape.unicode.java-props",
    ESCAPE = "constant.character.escape.java-props",
    COMMENT = "comment.line.java-props",
    COMMENT_MARK = "punctuation.definition.comment.java-props",
    KEY = "entity.name.key.java-props",
}

const fn ctx(items: &'static [crate::machine::It]) -> Ctx {
    Ctx::new(items)
}

static CTXS: [Ctx; 10] = [
    // #anon_main_0
    ctx(&[P(Pat::new(empty).pop())]).meta(&[s::PROPERTY_META]),
    // #anon_property-value_0
    ctx(&[I(BACKSLASH)]).content(&[s::VALUE]),
    // #anon_property_0
    ctx(&[
        P(Pat::new(separator)
            .sc(&[s::SEPARATOR])
            .set(&[PROPERTY_VALUE])),
        I(PROPERTY_VALUE),
    ]),
    // #anon_property_1, the `with_prototype` of a key's end
    ctx(&[P(Pat::new(newline_ahead).pop())]).no_proto(),
    // __main
    ctx(&[I(MAIN)]),
    // __start: its `source.java-props` is the base scope
    ctx(&[P(Pat::new(empty).push(&[MAIN_1]))]).no_proto(),
    // backslash
    ctx(&[
        P(Pat::new(continuation).caps(&[(1, &[s::CONTINUATION])])),
        P(Pat::new(unicode).sc(&[s::UNICODE])),
        P(Pat::new(escape).sc(&[s::ESCAPE])),
    ]),
    // main: only ever included, so its `source.java-props` never applies
    ctx(&[
        P(Pat::new(comment)
            .sc(&[s::COMMENT])
            .caps(&[(1, &[s::COMMENT_MARK])])),
        P(Pat::new(non_space_ahead).push(&[MAIN_0, PROPERTY])),
    ]),
    // property
    ctx(&[
        I(BACKSLASH),
        P(Pat::new(key_end_ahead).set(&[PROPERTY_0]).with(PROPERTY_1)),
    ])
    .content(&[s::KEY]),
    // property-value
    ctx(&[P(Pat::new(non_space_ahead).set(&[VALUE_0]))]),
];

static SLOTS: [u16; 10] = slots(&CTXS);
static PROTOS: [bool; 10] = protos(&CTXS, None);
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

/// `[:=]`
fn separator(l: &Line, from: usize) -> Option<M> {
    let hay = l.b.get(from..)?;
    let i = from + hay.iter().position(|&c| c == b':' || c == b'=')?;
    Some(M::new(i, i + 1))
}

/// `(?=\n)`
fn newline_ahead(l: &Line, from: usize) -> Option<M> {
    (from <= l.nl()).then(|| M::new(l.nl(), l.nl()))
}

/// `(\\)\n`
fn continuation(l: &Line, from: usize) -> Option<M> {
    let k = l.nl().checked_sub(1)?;
    (k >= from && l.b[k] == b'\\').then(|| M::new(k, l.end()).g(1, k, k + 1))
}

/// `\\u[[:xdigit:]]{4}`
fn unicode(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(m) = l.lit(at, b"\\u") {
        let k = m.s as usize;
        if (k + 2..k + 6).all(|i| l.is(i, |c| c.is_ascii_hexdigit())) {
            return Some(M::new(k, k + 6));
        }
        at = k + 1;
    }
    None
}

/// `\\[^u]`, the `\n` among what follows
fn escape(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, b'\\') {
        match l.at(k + 1) {
            Some(b'u') | None => at = k + 1,
            Some(_) if k + 1 == l.nl() => return Some(M::new(k, l.end())),
            Some(_) => return Some(M::new(k, k + 1 + l.char_len(k + 1).unwrap_or(1))),
        }
    }
    None
}

/// `^\s*([#!]).*`
fn comment(l: &Line, from: usize) -> Option<M> {
    if from > 0 {
        return None;
    }
    let q = l.run(0, is_space);
    (q < l.nl() && matches!(l.b[q], b'#' | b'!')).then(|| M::new(0, l.nl()).g(1, q, q + 1))
}

/// `(?=\S)`
fn non_space_ahead(l: &Line, from: usize) -> Option<M> {
    non_space(l, from).map(|m| M::new(m.s as usize, m.s as usize))
}

/// `(?=[:=\s])`
fn key_end_ahead(l: &Line, from: usize) -> Option<M> {
    (from..=l.nl())
        .find(|&i| l.is(i, |c| c == b':' || c == b'=' || is_space(c)))
        .map(|i| M::new(i, i))
}
