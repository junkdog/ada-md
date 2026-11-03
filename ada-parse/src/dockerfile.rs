//! Dockerfiles by a scanner: bat's `Dockerfile` compiled by hand. Every pattern but an
//! instruction's arguments' is anchored at a line's start, so a line is decided by its first
//! bytes: a comment, an instruction (its arguments then run to a line's end that isn't
//! continued by `\`), `FROM`, or text bat marks `invalid`. A string left open at a line's end
//! turns the next line `invalid`, as bat's does. bat's `Dockerfile (with bash)`, which colors
//! `RUN` as shell, is a syntax of its own that no file name selects, and stays on syntect.
//!
//! Exact against bat's grammar on syntect: the `parse` fuzz target holds every byte's scopes to
//! syntect's.

use crate::matcher::{Atom, atoms, is_space};
use crate::{Out, State};

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

const COMMENT_LEADING: u16 = 0;
const COMMENT: u16 = 1;
const COMMENT_MARK: u16 = 2;
const KEYWORD: u16 = 3;
const ONBUILD: u16 = 4;
const OPERATOR: u16 = 5;
const STAGE: u16 = 7;
const TAG: u16 = 8;
const INVALID: u16 = 9;
const ESCAPED: u16 = 10;
/// A double-quoted string's kinds; a single-quoted one's are the same, [`IN_SINGLE`] on.
const DOUBLE: u16 = 11;
const DOUBLE_BEGIN: u16 = 12;
const DOUBLE_END: u16 = 13;
const DOUBLE_ESCAPED: u16 = 14;
const IN_SINGLE: u16 = 4;

/// Each kind's scopes under `source.dockerfile`, by kind.
static KINDS: [&[Atom]; 19] = [
    &[s::COMMENT_LEADING],
    &[s::COMMENT_S],
    &[s::COMMENT_S, s::COMMENT_MARK],
    &[s::KEYWORD],
    &[s::KEYWORD, s::SPECIAL],
    &[s::OPERATOR],
    &[s::OPERATOR, s::SPECIAL],
    &[s::STAGE],
    &[s::TAG_DIGEST],
    &[s::INVALID_S],
    &[s::ESCAPED],
    &[s::DOUBLE],
    &[s::DOUBLE, s::STR_BEGIN],
    &[s::DOUBLE, s::STR_END],
    &[s::DOUBLE, s::ESCAPED],
    &[s::SINGLE],
    &[s::SINGLE, s::STR_BEGIN],
    &[s::SINGLE, s::STR_END],
    &[s::SINGLE, s::ESCAPED],
];

/// The scopes under `source.dockerfile` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    KINDS.get(usize::from(kind)).map_or_else(Vec::new, |k| {
        k.iter().map(|&a| ATOMS[usize::from(a)]).collect()
    })
}

/// The context open at the scan's place: the top of bat's stack.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ctx {
    /// `main`, before the first `FROM`: comments, `ARG` and `FROM` only.
    Main,
    /// `body`, which each `FROM` pushes and nothing pops.
    Body,
    /// `args`: an instruction's arguments, to a line's end that isn't continued.
    Args,
    /// A string in the arguments, `"` or `'`.
    Double,
    Single,
    /// `invalid`, where a string left open at a line's end set it: the next line that starts
    /// with something other than an ASCII capital is `invalid`.
    Invalid,
}

/// The levels bat's stack holds above `main` past which a line is no longer exact: syntect's
/// stack, which also holds `__start` and `__main`, takes no push past 100 levels, where this
/// scanner goes on.
const EXACT_DEPTH: u64 = 96;

/// Lex `b`, a line without its `\n`, from `state`: its word holds the open context and how many
/// levels bat's stack holds above `main`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let word = state.word();
    let mut ctx = match word & 0xf {
        1 => Ctx::Body,
        2 => Ctx::Args,
        3 => Ctx::Double,
        4 => Ctx::Single,
        5 => Ctx::Invalid,
        _ => Ctx::Main,
    };
    let mut depth = word >> 4;
    let blank = b.iter().all(|&c| is_space(c));
    // the patterns anchored at the line's start; past them only the arguments' and strings'
    // patterns can match
    let p = match ctx {
        Ctx::Main | Ctx::Body => {
            if comment(b, out) {
                return save(state, ctx, depth);
            }
            if ctx == Ctx::Body
                && let Some(e) = directive(b, out)
            {
                ctx = Ctx::Args;
                depth += 1;
                // a keyword whose `\s` is the line's `\n` took it: nothing pops the arguments
                if e > b.len() {
                    return save(state, ctx, depth);
                }
                e
            } else {
                if ctx == Ctx::Main && arg(b) {
                    out.push(0, 4.min(b.len()), KEYWORD);
                } else if ctx == Ctx::Body && invalid(b) {
                    out.push(0, b.len(), INVALID);
                } else if from(b, out) {
                    ctx = Ctx::Body;
                    depth += 1;
                }
                return save(state, ctx, depth);
            }
        }
        // `^\s*$` takes a blank line and its `\n`: the arguments go on
        Ctx::Args if blank || comment(b, out) => return save(state, ctx, depth),
        Ctx::Double | Ctx::Single if blank => {
            let kind = if ctx == Ctx::Double {
                DOUBLE
            } else {
                DOUBLE + IN_SINGLE
            };
            out.push(0, b.len(), kind);
            return save(state, ctx, depth);
        }
        Ctx::Args | Ctx::Double | Ctx::Single => 0,
        Ctx::Invalid => {
            if invalid(b) {
                out.push(0, b.len(), INVALID);
                ctx = Ctx::Body;
            }
            return save(state, ctx, depth);
        }
    };
    let (ctx, depth) = args(b, p, ctx, depth, out);
    save(state, ctx, depth);
}

/// The arguments or a string in them, from `p` to the line's end: the context the line
/// leaves open, and the depth.
fn args(b: &[u8], mut p: usize, mut ctx: Ctx, mut depth: u64, out: &mut Out) -> (Ctx, u64) {
    loop {
        if ctx == Ctx::Args {
            let next = b[p..]
                .iter()
                .position(|&c| matches!(c, b'\\' | b'"' | b'\''));
            let Some(k) = next.map(|i| p + i) else {
                // `\n` pops the arguments
                return (Ctx::Body, depth - 1);
            };
            match b[k] {
                b'\\' => match escaped(b, k) {
                    Some(e) => {
                        out.push(k, e, ESCAPED);
                        p = e;
                    }
                    // `\\\s+$` takes the `\` and the `\n`: the arguments go on
                    None => return (ctx, depth),
                },
                q => {
                    let single = if q == b'"' { 0 } else { IN_SINGLE };
                    out.push(k, k + 1, DOUBLE_BEGIN + single);
                    p = k + 1;
                    ctx = if single == 0 {
                        Ctx::Double
                    } else {
                        Ctx::Single
                    };
                    depth += 1;
                }
            }
            continue;
        }
        let (quote, single) = match ctx {
            Ctx::Double => (b'"', 0),
            _ => (b'\'', IN_SINGLE),
        };
        let next = b[p..].iter().position(|&c| c == b'\\' || c == quote);
        let Some(k) = next.map(|i| p + i) else {
            // `\n` sets `invalid`
            out.push(p, b.len(), DOUBLE + single);
            return (Ctx::Invalid, depth);
        };
        out.push(p, k, DOUBLE + single);
        if b[k] == quote {
            out.push(k, k + 1, DOUBLE_END + single);
            p = k + 1;
            ctx = Ctx::Args;
            depth -= 1;
            continue;
        }
        match escaped(b, k) {
            Some(e) => {
                out.push(k, e, DOUBLE_ESCAPED + single);
                p = e;
            }
            // `\\\s+$`: the string goes on
            None => {
                out.push(k, b.len(), DOUBLE + single);
                return (ctx, depth);
            }
        }
    }
}

/// Keeps the context a line leaves open and the depth in `state`; a depth near syntect's limit
/// marks it inexact.
fn save(state: &mut State, ctx: Ctx, depth: u64) {
    if depth >= EXACT_DEPTH {
        state.inexact = true;
    }
    let ctx = match ctx {
        Ctx::Main => 0,
        Ctx::Body => 1,
        Ctx::Args => 2,
        Ctx::Double => 3,
        Ctx::Single => 4,
        Ctx::Invalid => 5,
    };
    state.set_word(depth.min(u64::MAX >> 4) << 4 | ctx);
}

/// `\\.` at `k`: where it ends; `None` for a `\` that ends the line, which `\\\s+$` takes.
fn escaped(b: &[u8], k: usize) -> Option<usize> {
    (k + 1 < b.len()).then(|| k + 1 + char_len(b, k + 1))
}

/// `^(\s*)((#).*(?m:$)\n?)`: the whole line, `\n` and all.
fn comment(b: &[u8], out: &mut Out) -> bool {
    let Some(q) = b.iter().position(|&c| !is_space(c)) else {
        return false;
    };
    if b[q] != b'#' {
        return false;
    }
    out.push(0, q, COMMENT_LEADING);
    out.push(q, q + 1, COMMENT_MARK);
    out.push(q + 1, b.len(), COMMENT);
    true
}

/// Whether `word` starts at `at`, ignoring ASCII case.
fn word_at(b: &[u8], at: usize, word: &[u8]) -> bool {
    b.get(at..at + word.len())
        .is_some_and(|w| w.eq_ignore_ascii_case(word))
}

/// Whether `\s` matches at `at`, the line's `\n` at its end.
fn space_at(b: &[u8], at: usize) -> bool {
    b.get(at).is_none_or(|&c| is_space(c)) && at <= b.len()
}

/// Where the spaces at `at` end.
fn spaces(b: &[u8], at: usize) -> usize {
    at + b[at.min(b.len())..]
        .iter()
        .take_while(|&&c| is_space(c))
        .count()
}

/// `^\s*(?i:(onbuild)\s+)?(?i:<words>)\s`: the match's end, and where `onbuild` is.
fn keyword(b: &[u8], words: &[&[u8]]) -> Option<(usize, Option<usize>)> {
    let q = spaces(b, 0);
    let after = |at: usize| {
        words
            .iter()
            .find(|w| word_at(b, at, w) && space_at(b, at + w.len()))
            .map(|w| at + w.len() + 1)
    };
    if word_at(b, q, b"onbuild") {
        let k = spaces(b, q + 7);
        if k > q + 7
            && let Some(e) = after(k)
        {
            return Some((e, Some(q)));
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

/// `directives`, in the grammar's order: an instruction that takes arguments (`RUN`, `ENV`,
/// …), `MAINTAINER`, `COPY` and `CMD` or `ENTRYPOINT`, `ONBUILD` before any but `MAINTAINER`.
/// Where the match ends, its arguments' start: past the line's end when the keyword's `\s` is
/// its `\n`.
fn directive(b: &[u8], out: &mut Out) -> Option<usize> {
    // caps 0 the keyword, 1 `onbuild` within it
    let whole = |e: usize, onbuild: Option<usize>, kind: u16, out: &mut Out| {
        let end = e.min(b.len());
        match onbuild {
            Some(o) => {
                out.push(0, o, kind);
                out.push(o, o + 7, kind + 1);
                out.push(o + 7, end, kind);
            }
            None => out.push(0, end, kind),
        }
        e
    };
    if let Some((e, onbuild)) = keyword(b, &INSTRUCTIONS) {
        return Some(whole(e, onbuild, KEYWORD, out));
    }
    if let Some((e, None)) = keyword(b, &[b"maintainer"]) {
        return Some(whole(e, None, KEYWORD, out));
    }
    if let Some(e) = copy(b, out) {
        return Some(e);
    }
    keyword(b, &[b"cmd", b"entrypoint"]).map(|(e, onbuild)| whole(e, onbuild, OPERATOR, out))
}

/// `^\s*((?i:(onbuild)\s+)?(?i:copy))(?:\s+--from=(\S+))?\s`: caps 1 the keyword, 2 `onbuild`,
/// 3 the stage.
fn copy(b: &[u8], out: &mut Out) -> Option<usize> {
    let q = spaces(b, 0);
    let (kw_end, onbuild) = match word_at(b, q, b"onbuild") {
        true => {
            let k = spaces(b, q + 7);
            if k > q + 7 && word_at(b, k, b"copy") {
                (k + 4, true)
            } else if word_at(b, q, b"copy") {
                (q + 4, false)
            } else {
                return None;
            }
        }
        false if word_at(b, q, b"copy") => (q + 4, false),
        false => return None,
    };
    let keyword = |out: &mut Out| {
        if onbuild {
            out.push(q, q + 7, ONBUILD);
            out.push(q + 7, kw_end, KEYWORD);
        } else {
            out.push(q, kw_end, KEYWORD);
        }
    };
    let s = spaces(b, kw_end);
    if s > kw_end && b.get(s..s + 7) == Some(b"--from=") {
        let e = s + 7 + b[s + 7..].iter().take_while(|&&c| !is_space(c)).count();
        if e > s + 7 && space_at(b, e) {
            keyword(out);
            out.push(s + 7, e, STAGE);
            return Some(e + 1);
        }
    }
    space_at(b, kw_end).then(|| {
        keyword(out);
        kw_end + 1
    })
}

/// `^(?i:(from))\s+[^\s:@]+(?:[:@](\S+))?(?:\s+(?i:(as))\s+(\S+))?`: caps 1 `from`, 2 the tag
/// or digest, 3 `as`, 4 the stage.
fn from(b: &[u8], out: &mut Out) -> bool {
    if !word_at(b, 0, b"from") {
        return false;
    }
    let image = spaces(b, 4);
    let e = image
        + b[image..]
            .iter()
            .take_while(|&&c| !is_space(c) && c != b':' && c != b'@')
            .count();
    if image == 4 || e == image {
        return false;
    }
    out.push(0, 4, KEYWORD);
    let mut end = e;
    if b.get(e).is_some_and(|&c| c == b':' || c == b'@') {
        let t = e + 1 + b[e + 1..].iter().take_while(|&&c| !is_space(c)).count();
        if t > e + 1 {
            out.push(e + 1, t, TAG);
            end = t;
        }
    }
    let a = spaces(b, end);
    if a > end && word_at(b, a, b"as") {
        let n = spaces(b, a + 2);
        let ne = n + b[n..].iter().take_while(|&&c| !is_space(c)).count();
        if n > a + 2 && ne > n {
            out.push(a, a + 2, KEYWORD);
            out.push(n, ne, STAGE);
        }
    }
    true
}

/// `^[^A-Z\n](.*)(?m:$)`
fn invalid(b: &[u8]) -> bool {
    b.first().is_some_and(|c| !c.is_ascii_uppercase())
}

/// `^(?i:arg)\s`
fn arg(b: &[u8]) -> bool {
    word_at(b, 0, b"arg") && space_at(b, 3)
}

/// The length of the character at `i`, a valid UTF-8 line's.
fn char_len(b: &[u8], i: usize) -> usize {
    match b[i] {
        0..0x80 => 1,
        0xc0..0xe0 => 2,
        0xe0..0xf0 => 3,
        _ => 4,
    }
}
