//! Dockerfiles, in place of bat's `Dockerfile`: its 12 contexts on the
//! [machine](crate::machine). An instruction's arguments run on across lines ending in `\`
//! (and blank ones); a string left open at a line's end turns to `invalid`, as bat's does.
//! bat's `Dockerfile (with bash)`, which colors `RUN` as shell, is a syntax of its own that no
//! file name selects, and stays on syntect.
//!
//! The contexts keep syntect's ids (the order the probe's `dump Dockerfile` lists them in).

use crate::machine::{
    Ctx, Grammar, It::I, It::P, Kinds, Line, M, Pat, atoms, empty, first, is_space, lit1, protos,
    slots,
};

const MAIN_0: u8 = 0;
const START: u8 = 1;
const ARGS: u8 = 2;
const BODY: u8 = 3;
const COMMENTS: u8 = 4;
const DIRECTIVES: u8 = 5;
const DOUBLE_QUOTED: u8 = 6;
const ESCAPED_CHAR: u8 = 7;
const FROM: u8 = 8;
const INVALID: u8 = 9;
const MAIN: u8 = 10;
const SINGLE_QUOTED: u8 = 11;

atoms! {
    STR_BEGIN = "punctuation.definition.string.begin.dockerfile",
    COMMENT_LEADING = "punctuation.whitespace.comment.leading.dockerfile",
    COMMENT_S = "comment.dockerfile",
    COMMENT_MARK = "punctuation.definition.comment.dockerfile",
    KEYWORD = "keyword.control.dockerfile",
    SPECIAL = "keyword.other.special-method.dockerfile",
    STAGE = "variable.stage-name",
    OPERATOR = "keyword.operator.dockerfile",
    DOUBLE = "string.quoted.double.dockerfile",
    STR_END = "punctuation.definition.string.end.dockerfile",
    ESCAPED = "constant.character.escaped.dockerfile",
    TAG_DIGEST = "entity.name.enum.tag-digest",
    INVALID_S = "invalid",
    SINGLE = "string.quoted.single.dockerfile",
}

const fn ctx(items: &'static [crate::machine::It]) -> Ctx {
    Ctx::new(items)
}

static CTXS: [Ctx; 12] = [
    // __main
    ctx(&[I(MAIN)]),
    // __start: its `source.dockerfile` is the base scope
    ctx(&[P(Pat::new(empty).push(&[MAIN_0]))]).no_proto(),
    // args
    ctx(&[
        I(COMMENTS),
        I(ESCAPED_CHAR),
        P(Pat::new(blank_line)),
        P(Pat::new(continuation)),
        P(Pat::new(newline).pop()),
        P(Pat::new(dquote).sc(&[s::STR_BEGIN]).push(&[DOUBLE_QUOTED])),
        P(Pat::new(squote).sc(&[s::STR_BEGIN]).push(&[SINGLE_QUOTED])),
    ]),
    // body
    ctx(&[I(COMMENTS), I(DIRECTIVES), I(INVALID), I(FROM)]),
    // comments
    ctx(&[P(Pat::new(comment).caps(&[
        (1, &[s::COMMENT_LEADING]),
        (2, &[s::COMMENT_S]),
        (3, &[s::COMMENT_MARK]),
    ]))]),
    // directives
    ctx(&[
        P(Pat::new(instruction)
            .caps(&[(0, &[s::KEYWORD]), (1, &[s::SPECIAL])])
            .push(&[ARGS])),
        P(Pat::new(maintainer).sc(&[s::KEYWORD]).push(&[ARGS])),
        P(Pat::new(copy)
            .caps(&[(1, &[s::KEYWORD]), (2, &[s::SPECIAL]), (3, &[s::STAGE])])
            .push(&[ARGS])),
        P(Pat::new(command)
            .caps(&[(0, &[s::OPERATOR]), (1, &[s::SPECIAL])])
            .push(&[ARGS])),
    ]),
    // double_quote_string
    ctx(&[
        I(ESCAPED_CHAR),
        P(Pat::new(blank_line)),
        P(Pat::new(continuation)),
        P(Pat::new(newline).set(&[INVALID])),
        P(Pat::new(dquote).sc(&[s::STR_END]).pop()),
    ])
    .meta(&[s::DOUBLE]),
    // escaped-char
    ctx(&[P(Pat::new(escaped).sc(&[s::ESCAPED]))]),
    // from
    ctx(&[P(Pat::new(from)
        .caps(&[
            (1, &[s::KEYWORD]),
            (2, &[s::TAG_DIGEST]),
            (3, &[s::KEYWORD]),
            (4, &[s::STAGE]),
        ])
        .push(&[BODY]))]),
    // invalid
    ctx(&[P(Pat::new(invalid).sc(&[s::INVALID_S]).set(&[BODY]))]),
    // main: only ever included, so its `source.dockerfile` never applies
    ctx(&[I(COMMENTS), P(Pat::new(arg).sc(&[s::KEYWORD])), I(FROM)]),
    // single_quote_string
    ctx(&[
        I(ESCAPED_CHAR),
        P(Pat::new(blank_line)),
        P(Pat::new(continuation)),
        P(Pat::new(newline).set(&[INVALID])),
        P(Pat::new(squote).sc(&[s::STR_END]).pop()),
    ])
    .meta(&[s::SINGLE]),
];

static SLOTS: [u16; 12] = slots(&CTXS);
static PROTOS: [bool; 12] = protos(&CTXS, None);
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

// the matchers: each finds its regex's leftmost match at or after `from`; `^` holds at the
// line's start and after its `\n`, where nothing these look for follows

/// `^\s*(?m:$)`: a blank line, its `\n` and all.
fn blank_line(l: &Line, from: usize) -> Option<M> {
    (from == 0 && l.run(0, is_space) == l.end()).then(|| M::new(0, l.end()))
}

/// `\\\s+(?m:$)`: a `\` with only spaces after it, the `\n` taken.
fn continuation(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, b'\\') {
        if l.run(k + 1, is_space) == l.end() {
            return Some(M::new(k, l.end()));
        }
        at = k + 1;
    }
    None
}

/// `\n`
fn newline(l: &Line, from: usize) -> Option<M> {
    (from <= l.nl()).then(|| M::new(l.nl(), l.end()))
}

/// `"`
fn dquote(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'"')
}

/// `'`
fn squote(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'\'')
}

/// `\\.`
fn escaped(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, b'\\') {
        if let Some(n) = l.char_len(k + 1) {
            return Some(M::new(k, k + 1 + n));
        }
        at = k + 1;
    }
    None
}

/// `^(\s*)((#).*(?m:$)\n?)`
fn comment(l: &Line, from: usize) -> Option<M> {
    if from > 0 {
        return None;
    }
    let q = l.run(0, is_space);
    (q < l.nl() && l.b[q] == b'#').then(|| {
        let m = M::new(0, l.end()).g(2, q, l.end()).g(3, q, q + 1);
        if q > 0 { m.g(1, 0, q) } else { m }
    })
}

/// Whether `word` starts at `at`, ignoring ASCII case.
fn word_at(l: &Line, at: usize, word: &[u8]) -> bool {
    l.b.get(at..at + word.len())
        .is_some_and(|w| w.eq_ignore_ascii_case(word))
}

/// `^\s*(?i:(onbuild)\s+)?(?i:<words>)\s`: the match's end, and where `onbuild` is.
fn keyword(l: &Line, words: &[&[u8]]) -> Option<(usize, Option<(usize, usize)>)> {
    let q = l.run(0, is_space);
    let after = |at: usize| {
        words
            .iter()
            .map(|w| at + w.len())
            .zip(words)
            .find(|&(e, w)| word_at(l, at, w) && l.is(e, is_space))
            .map(|(e, _)| e + 1)
    };
    if word_at(l, q, b"onbuild") {
        let k = l.run(q + 7, is_space);
        if k > q + 7
            && let Some(e) = after(k)
        {
            return Some((e, Some((q, q + 7))));
        }
    }
    after(q).map(|e| (e, None))
}

const INSTRUCTIONS: [&[u8]; 12] = [
    b"add",
    b"arg",
    b"env",
    b"expose",
    b"healthcheck",
    b"label",
    b"run",
    b"shell",
    b"stopsignal",
    b"user",
    b"volume",
    b"workdir",
];

/// An instruction that takes arguments (`RUN`, `ENV`, …), `ONBUILD` before it or not.
fn instruction(l: &Line, from: usize) -> Option<M> {
    with_onbuild(l, from, &INSTRUCTIONS)
}

/// `^\s*(?i:(onbuild)\s+)?(?i:cmd|entrypoint)\s`
fn command(l: &Line, from: usize) -> Option<M> {
    with_onbuild(l, from, &[b"cmd", b"entrypoint"])
}

fn with_onbuild(l: &Line, from: usize, words: &[&[u8]]) -> Option<M> {
    if from > 0 {
        return None;
    }
    let (e, onbuild) = keyword(l, words)?;
    let m = M::new(0, e);
    Some(match onbuild {
        Some((s, e)) => m.g(1, s, e),
        None => m,
    })
}

/// `^\s*(?i:maintainer)\s`
fn maintainer(l: &Line, from: usize) -> Option<M> {
    if from > 0 {
        return None;
    }
    keyword(l, &[b"maintainer"])
        .filter(|&(_, o)| o.is_none())
        .map(|(e, _)| M::new(0, e))
}

/// `^\s*((?i:(onbuild)\s+)?(?i:copy))(?:\s+--from=(\S+))?\s`
fn copy(l: &Line, from: usize) -> Option<M> {
    if from > 0 {
        return None;
    }
    let q = l.run(0, is_space);
    let try_copy = |at: usize| word_at(l, at, b"copy").then_some(at + 4);
    let (start, kw_end, onbuild) = match word_at(l, q, b"onbuild") {
        true => {
            let k = l.run(q + 7, is_space);
            match try_copy(k).filter(|_| k > q + 7) {
                Some(e) => (q, e, Some((q, q + 7))),
                None => (q, try_copy(q)?, None),
            }
        }
        false => (q, try_copy(q)?, None),
    };
    let mut m = M::new(0, 0).g(1, start, kw_end);
    if let Some(o) = onbuild {
        m = m.g(2, o.0, o.1);
    }
    // `(?:\s+--from=(\S+))?`, then `\s`
    let s = l.run(kw_end, is_space);
    if s > kw_end && l.b.get(s..s + 7) == Some(b"--from=") {
        let e = l.run(s + 7, |c| !is_space(c));
        if e > s + 7 && l.is(e, is_space) {
            m = m.g(3, s + 7, e);
            m.e = e as u32 + 1;
            return Some(m);
        }
    }
    l.is(kw_end, is_space).then(|| {
        m.e = kw_end as u32 + 1;
        m
    })
}

/// `^(?i:(from))\s+[^\s:@]+(?:[:@](\S+))?(?:\s+(?i:(as))\s+(\S+))?`
fn from(l: &Line, from: usize) -> Option<M> {
    if from > 0 || !word_at(l, 0, b"from") {
        return None;
    }
    let image = l.run(4, is_space);
    let e = l.run(image, |c| !is_space(c) && c != b':' && c != b'@');
    if image == 4 || e == image {
        return None;
    }
    let mut m = M::new(0, e).g(1, 0, 4);
    let mut end = e;
    if l.is(e, |c| c == b':' || c == b'@') {
        let t = l.run(e + 1, |c| !is_space(c));
        if t > e + 1 {
            m = m.g(2, e + 1, t);
            end = t;
        }
    }
    let a = l.run(end, is_space);
    if a > end && word_at(l, a, b"as") {
        let n = l.run(a + 2, is_space);
        let ne = l.run(n, |c| !is_space(c));
        if n > a + 2 && ne > n {
            m = m.g(3, a, a + 2).g(4, n, ne);
            end = ne;
        }
    }
    m.e = end as u32;
    Some(m)
}

/// `^[^A-Z\n](.*)(?m:$)`
fn invalid(l: &Line, from: usize) -> Option<M> {
    (from == 0 && l.b.first().is_some_and(|c| !c.is_ascii_uppercase())).then(|| M::new(0, l.nl()))
}

/// `^(?i:arg)\s`
fn arg(l: &Line, from: usize) -> Option<M> {
    (from == 0 && word_at(l, 0, b"arg") && l.is(3, is_space)).then(|| M::new(0, 4))
}
