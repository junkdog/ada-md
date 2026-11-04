//! A `using A.B.C;` line without the scan (X-025's seventh stage): at the top level or in a
//! namespace's block, the commonest of a file's first lines, where the scan's turns (`main`'s
//! words, `using`'s lookaheads for an alias and `static`, and `using_namespace`'s patterns at
//! each name, `.` and blank) cost several times the line's few spans. Each span is the one
//! the scan gives: `using`, each name a path, each `.` a separator, the `;` the end, and the
//! state is as it was (`using_namespace` pushed and popped). Anything else on the line (an
//! alias, `static`, `global::`, a name past ASCII, blanks inside the name, text after the
//! `;`), a context not named here, or a level whose bits would give the spans scopes of their
//! own, is the scan's.

use super::c::*;
use super::*;
use crate::clike::DIGIT;

/// `b`, a line whose leading blanks end at `i`, lexed here if it is a plain `using` of a
/// namespace in a context that takes one: its spans pushed as [`line`](super::line) would,
/// the state as it was. `false`, and nothing touched, when the line is the scan's.
#[inline(always)]
pub(super) fn line(b: &[u8], i: usize, state: &State, out: &mut Out) -> bool {
    b.get(i) == Some(&b'u') && take(b, i, state, out)
}

/// The blanks at `p`: where they end.
fn blanks(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && matches!(b[p], b' ' | b'\t') {
        p += 1;
    }
    p
}

/// [`line`] past its first byte's check.
#[inline(never)]
fn take(b: &[u8], i: usize, state: &State, out: &mut Out) -> bool {
    // `using` and its blanks (`\b(using)\s+`): the word whole, a blank after it
    if !(b.len() > i + 5 && &b[i..i + 5] == b"using" && matches!(b[i + 5], b' ' | b'\t')) {
        return false;
    }
    // where no level gives bits of its own (the save's flag, [`BITS`]), each span's kind is its
    // leaf; the top passes over the blanks before `using` and takes it with `main`'s patterns
    let d = usize::from(state.depth).min(KEPT);
    let top = match d {
        0 => MAIN,
        d if d <= HALF => state.levels[d - 1],
        d => state.withs[d - 1 - HALF],
    };
    if state.flags() & BITS != 0 || !matches!(top, MAIN | MAIN_BRACE | MAIN_BRACE_M) {
        return false;
    }
    // the name: ASCII names (none `static`, `using static`'s) with a `.` between, then `;` and
    // blanks to the line's end; the name's first `.` or `;` rules out `using`'s alias and
    // `using_namespace`'s `::`
    let s = blanks(b, i + 5);
    let mut p = s;
    let mut names = 0;
    loop {
        let e = p + b[p..]
            .iter()
            .position(|&c| CLASS[usize::from(c)] & WORD == 0)
            .unwrap_or(b.len() - p);
        if e == p || CLASS[usize::from(b[p])] & DIGIT != 0 || e == b.len() {
            return false;
        }
        if names == 0 && &b[p..e] == b"static" {
            return false;
        }
        names += 1;
        match b[e] {
            b'.' => p = e + 1,
            b';' if blanks(b, e + 1) == b.len() => break,
            _ => return false,
        }
    }
    let spans = &mut *out.0;
    let mut put = |s: usize, e: usize, kind: u16| {
        spans.push(Span {
            start: s as u32,
            end: e as u32,
            kind,
        });
    };
    put(i, i + 5, IMPORT);
    let mut p = s;
    for _ in 0..names {
        let e = p + b[p..]
            .iter()
            .position(|&c| CLASS[usize::from(c)] & WORD == 0)
            .unwrap_or(0);
        put(p, e, PATH);
        if b[e] == b'.' {
            put(e, e + 1, NS_SEP);
        } else {
            put(e, e + 1, TERMINATOR);
        }
        p = e + 1;
    }
    true
}
