//! Character classes and the grammar's recurring regex pieces, matched as Oniguruma would:
//! an identifier, a qualified path with its backtracking order, a template's arguments, an
//! operator's name. A line is its bytes without the `\n`; where a regex could take the `\n`,
//! the line's end stands for it.

use crate::{is_space, skip_spaces};

/// The bytes of the UTF-8 character that starts with `c`.
fn char_len(c: u8) -> usize {
    match c {
        0xf0.. => 4,
        0xe0.. => 3,
        0xc0.. => 2,
        _ => 1,
    }
}

/// The character byte `i` of `b` is in.
#[cold]
fn wide(b: &[u8], i: usize) -> char {
    let s = (i.saturating_sub(3)..=i)
        .rev()
        .find(|&s| b[s] & 0xc0 != 0x80)
        .unwrap_or(i);
    let e = (s + char_len(b[s])).min(b.len());
    std::str::from_utf8(&b[s..e])
        .ok()
        .and_then(|t| t.chars().next())
        .unwrap_or('\u{fffd}')
}

/// ASCII's classes, by byte: `\w`, past ASCII (to decode).
const W: u8 = 1;
const WIDE: u8 = 2;
static CLASSES: [u8; 256] = {
    let mut t = [0; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        t[c] = if b >= 0x80 {
            WIDE
        } else if b.is_ascii_alphanumeric() || b == b'_' {
            W
        } else {
            0
        };
        c += 1;
    }
    t
};

/// `\w` at byte `i`.
#[inline]
pub fn is_word(b: &[u8], i: usize) -> bool {
    match b.get(i) {
        Some(&c) if c < 0x80 => CLASSES[usize::from(c)] & W != 0,
        Some(_) => {
            let c = wide(b, i);
            c.is_alphanumeric() || is_mark(c)
        }
        None => false,
    }
}

/// `\p{M}`, roughly: the combining marks Oniguruma's `\w` takes.
fn is_mark(c: char) -> bool {
    matches!(c, '\u{300}'..='\u{36f}' | '\u{1ab0}'..='\u{1aff}' | '\u{1dc0}'..='\u{1dff}'
        | '\u{20d0}'..='\u{20ff}' | '\u{fe20}'..='\u{fe2f}')
}

/// `\w` just before byte `i`.
#[inline]
pub fn word_before(b: &[u8], i: usize) -> bool {
    i > 0 && is_word(b, prev(b, i))
}

/// `\b` at `i`.
#[inline]
pub fn boundary(b: &[u8], i: usize) -> bool {
    word_before(b, i) != is_word(b, i)
}

/// `[\p{L}\p{N}_]` at `i`.
#[inline]
pub fn is_ident(b: &[u8], i: usize) -> bool {
    match b.get(i) {
        Some(&c) if c < 0x80 => c.is_ascii_alphanumeric() || c == b'_',
        Some(_) => {
            let c = wide(b, i);
            c.is_alphabetic() || c.is_numeric()
        }
        None => false,
    }
}

/// `[\p{L}_]` at `i`: where an identifier can start.
#[inline]
pub fn is_ident_start(b: &[u8], i: usize) -> bool {
    match b.get(i) {
        Some(&c) if c < 0x80 => c.is_ascii_alphabetic() || c == b'_',
        Some(_) => wide(b, i).is_alphabetic(),
        None => false,
    }
}

/// `\p{Lu}` at `i`.
#[inline]
pub fn is_upper(b: &[u8], i: usize) -> bool {
    match b.get(i) {
        Some(&c) if c < 0x80 => c.is_ascii_uppercase(),
        Some(_) => wide(b, i).is_uppercase(),
        None => false,
    }
}

/// `[\p{Lu}\p{Nd}_]` at `i`.
#[inline]
pub fn is_upper_rest(b: &[u8], i: usize) -> bool {
    match b.get(i) {
        Some(&c) if c < 0x80 => c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_',
        Some(_) => {
            let c = wide(b, i);
            c.is_uppercase() || c.is_numeric()
        }
        None => false,
    }
}

/// Where the run of `[\p{Lu}\p{Nd}_]` at `p` ends: ASCII's bytes at once, the rest by
/// character.
#[inline]
pub fn upper_rest_end(b: &[u8], mut p: usize) -> usize {
    while p < b.len() {
        let c = b[p];
        if c >= 0x80 {
            return run_of(b, p, is_upper_rest);
        }
        if !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_') {
            break;
        }
        p += 1;
    }
    p
}

/// The character after the one at `i`.
#[inline]
pub fn next(b: &[u8], i: usize) -> usize {
    match b.get(i) {
        Some(&c) => (i + char_len(c)).min(b.len()),
        None => i + 1,
    }
}

/// The character before `i`.
pub fn prev(b: &[u8], i: usize) -> usize {
    let mut j = i - 1;
    while j > 0 && b[j] & 0xc0 == 0x80 {
        j -= 1;
    }
    j
}

pub fn run_of(b: &[u8], mut p: usize, f: impl Fn(&[u8], usize) -> bool) -> usize {
    while p < b.len() && f(b, p) {
        p = next(b, p);
    }
    p
}

/// Where the run of `\w` at `p` ends.
pub fn word_end(b: &[u8], mut p: usize) -> usize {
    while let Some(&c) = b.get(p) {
        let k = CLASSES[usize::from(c)];
        if k & W != 0 {
            p += 1;
        } else if k & WIDE != 0 && is_word(b, p) {
            p = next(b, p);
        } else {
            break;
        }
    }
    p
}

/// Where the line's indentation ends: [`skip_spaces`] from 0, eight bytes at a time while
/// they're all ` `.
pub fn indent_end(b: &[u8]) -> usize {
    let mut p = 0;
    while let Some(chunk) = b.get(p..p + 8) {
        let x = u64::from_le_bytes(chunk.try_into().unwrap()) ^ 0x2020_2020_2020_2020;
        if x != 0 {
            p += (x.trailing_zeros() / 8) as usize;
            break;
        }
        p += 8;
    }
    skip_spaces(b, p)
}

/// Where the run of ASCII `\w` at `p` ends.
#[inline]
pub fn ascii_word_end(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && CLASSES[usize::from(b[p])] & W != 0 {
        p += 1;
    }
    p
}

/// Where the run of `[\p{L}\p{N}_]` at `p` ends.
pub fn ident_end(b: &[u8], mut p: usize) -> usize {
    while let Some(&c) = b.get(p) {
        let k = CLASSES[usize::from(c)];
        if k & W != 0 {
            p += 1;
        } else if k & WIDE != 0 && is_ident(b, p) {
            p = next(b, p);
        } else {
            break;
        }
    }
    p
}

/// `\b[\p{L}_][\p{L}\p{N}_]*\b` at `p`: where it ends.
#[inline]
pub fn id(b: &[u8], p: usize) -> Option<usize> {
    if !is_ident_start(b, p) || word_before(b, p) {
        return None;
    }
    let e = ident_end(b, next(b, p));
    (!is_word(b, e)).then_some(e)
}

/// `[\p{L}_][\p{L}\p{N}_]*\b` at `p`, without the `\b` before.
pub fn id_loose(b: &[u8], p: usize) -> Option<usize> {
    if !is_ident_start(b, p) {
        return None;
    }
    let e = ident_end(b, next(b, p));
    (!is_word(b, e)).then_some(e)
}

/// Whether the whole word at `p` is `w`: `\bw\b`.
pub fn is_kw(b: &[u8], p: usize, w: &[u8]) -> bool {
    b[p.min(b.len())..].starts_with(w) && !word_before(b, p) && !is_word(b, p + w.len())
}

/// Whether the text at `p` starts with `w` (no `\b`).
pub fn starts(b: &[u8], p: usize, w: &[u8]) -> bool {
    b.get(p..).is_some_and(|r| r.starts_with(w))
}

/// The byte at `i`, or `\n` at the line's end and past it.
#[inline]
pub fn at(b: &[u8], i: usize) -> u8 {
    b.get(i).copied().unwrap_or(b'\n')
}

/// `(?:::\s*)?(?:\bID\b\s*::\s*)*(?:template\s+)?\bID\b` at `p`: where each way it can match
/// ends, in the order a backtracking engine tries them (most `ID::` first, `template ` before
/// none). `f` is the rest of the regex at an end; the first end it takes is returned with
/// what it returned.
pub fn path<T>(b: &[u8], p: usize, mut f: impl FnMut(usize, usize) -> Option<T>) -> Option<T> {
    let mut s = p;
    if starts(b, p, b"::") {
        s = skip_spaces(b, p + 2);
    } else if !starts(b, p, b"template") {
        // the common case, one name: its end is the only one
        let e = id(b, p)?;
        if !starts(b, skip_spaces(b, e), b"::") {
            return f(p, e);
        }
    }
    // the segments' ends: after each `ID ::` and the spaces after
    let mut ends = [0usize; 32];
    let mut n = 0;
    ends[0] = s;
    let mut q = s;
    while let Some(e) = id(b, q) {
        let c = skip_spaces(b, e);
        if !starts(b, c, b"::") || n + 1 == ends.len() {
            break;
        }
        q = skip_spaces(b, c + 2);
        n += 1;
        ends[n] = q;
    }
    for k in (0..=n).rev() {
        let q = ends[k];
        if starts(b, q, b"template") {
            let t = skip_spaces(b, q + 8);
            if t > q + 8
                && let Some(e) = id(b, t)
                && let Some(r) = f(t, e)
            {
                return Some(r);
            }
        }
        if let Some(e) = id(b, q)
            && let Some(r) = f(q, e)
        {
            return Some(r);
        }
    }
    None
}

/// Whether `PATH` at `p` is followed by what `f` takes at its end.
pub fn path_then(b: &[u8], p: usize, f: impl Fn(usize) -> bool) -> bool {
    path(b, p, |_, e| f(e).then_some(())).is_some()
}

/// `[^(){}&;*^%=<>-]` at `i`: a template argument's byte (the `\n` at the line's end too,
/// though nothing can follow it).
#[inline]
fn arg_byte(b: &[u8], i: usize) -> bool {
    b.get(i).is_some_and(|&c| {
        !matches!(
            c,
            b'(' | b')'
                | b'{'
                | b'}'
                | b'&'
                | b';'
                | b'*'
                | b'^'
                | b'%'
                | b'='
                | b'<'
                | b'>'
                | b'-'
        )
    })
}

fn args_run(b: &[u8], mut i: usize) -> usize {
    while arg_byte(b, i) {
        i = next(b, i);
    }
    i
}

/// `<(?:N+(?:<(?:N+(?:<N*>)?)?\s*>)?)?N*(?:\(N*\))?N*>` at `p`, `b[p]` being `<`, with `N`
/// the argument bytes: where it ends. Deterministic: `N` takes neither bracket.
pub fn generic(b: &[u8], p: usize) -> Option<usize> {
    let i = p + 1;
    let r1 = args_run(b, i);
    let mut q = r1;
    if r1 > i
        && at(b, r1) == b'<'
        && let Some(e) = nested(b, r1)
    {
        q = e;
    }
    q = args_run(b, q);
    if at(b, q) == b'(' {
        let r = args_run(b, q + 1);
        if at(b, r) == b')' {
            q = args_run(b, r + 1);
        }
    }
    (at(b, q) == b'>' && q < b.len()).then_some(q + 1)
}

/// `<(?:N+(?:<N*>)?)?\s*>` at `p`.
fn nested(b: &[u8], p: usize) -> Option<usize> {
    let j = p + 1;
    let r2 = args_run(b, j);
    let mut k = j;
    if r2 > j {
        k = r2;
        if at(b, r2) == b'<' {
            let r3 = args_run(b, r2 + 1);
            if at(b, r3) == b'>' && r3 < b.len() {
                k = r3 + 1;
            }
        }
    }
    let k = skip_spaces(b, k);
    (at(b, k) == b'>' && k < b.len()).then_some(k + 1)
}

/// `\s*` then `GENERIC` at `p`, if a `<` follows the spaces.
pub fn spaces_generic(b: &[u8], p: usize) -> Option<usize> {
    let q = skip_spaces(b, p);
    (at(b, q) == b'<').then(|| generic(b, q)).flatten()
}

/// The operators' names after `operator\s*`, at `q`:
/// `[-+*/%^&|~!=<>]|[-+*/%^&|=!<>]=|<<=?|>>=?|&&|\|\||\+\+|--|,|->\*?|\(\)|\[\]|""\s*\bID\b`,
/// each end in the alternation's order, given to `f` until it takes one.
pub fn operator_name<T>(b: &[u8], q: usize, mut f: impl FnMut(usize) -> Option<T>) -> Option<T> {
    let c = at(b, q);
    let d = at(b, q + 1);
    let has = |n: usize| q + n <= b.len();
    let mut try_end = |n: usize, ok: bool| if ok && has(n) { f(q + n) } else { None };
    if let Some(r) = try_end(1, b"-+*/%^&|~!=<>".contains(&c)) {
        return Some(r);
    }
    if let Some(r) = try_end(2, b"-+*/%^&|=!<>".contains(&c) && d == b'=') {
        return Some(r);
    }
    for (x, y) in [(b'<', b'<'), (b'>', b'>')] {
        if c == x && d == y {
            if let Some(r) = try_end(3, at(b, q + 2) == b'=') {
                return Some(r);
            }
            if let Some(r) = try_end(2, true) {
                return Some(r);
            }
        }
    }
    for pair in [b"&&", b"||", b"++", b"--"] {
        if let Some(r) = try_end(2, c == pair[0] && d == pair[1]) {
            return Some(r);
        }
    }
    if let Some(r) = try_end(1, c == b',') {
        return Some(r);
    }
    if c == b'-' && d == b'>' {
        if let Some(r) = try_end(3, at(b, q + 2) == b'*') {
            return Some(r);
        }
        if let Some(r) = try_end(2, true) {
            return Some(r);
        }
    }
    if let Some(r) = try_end(2, c == b'(' && d == b')') {
        return Some(r);
    }
    if let Some(r) = try_end(2, c == b'[' && d == b']') {
        return Some(r);
    }
    if c == b'"' && d == b'"' {
        let s = skip_spaces(b, q + 2);
        if let Some(e) = id(b, s) {
            return f(e);
        }
    }
    None
}

/// `\boperator\s*OPS` at `p`, each end given to `f` until it takes one.
pub fn operator<T>(b: &[u8], p: usize, f: impl FnMut(usize) -> Option<T>) -> Option<T> {
    if !(starts(b, p, b"operator") && !word_before(b, p)) {
        return None;
    }
    operator_name(b, skip_spaces(b, p + 8), f)
}

/// `\s*` then `(?m:$)`, or `(` after the spaces: the end of a line or a call's parentheses.
pub fn paren_or_eol(b: &[u8], i: usize) -> bool {
    let q = skip_spaces(b, i);
    q >= b.len() || b[q] == b'('
}

/// `\s*` then a `(` at `i`.
pub fn paren_after(b: &[u8], i: usize) -> bool {
    at(b, skip_spaces(b, i)) == b'(' && skip_spaces(b, i) < b.len()
}

/// Whether the rest of the line from `i` is spaces: `\s*(?m:$)`.
pub fn eol_after(b: &[u8], i: usize) -> bool {
    skip_spaces(b, i) >= b.len()
}

/// `\s` at `i`, the line's end (its `\n`) included.
pub fn space_at(b: &[u8], i: usize) -> bool {
    b.get(i).is_none_or(|&c| is_space(c))
}

/// `\S` at `i`.
pub fn non_space_at(b: &[u8], i: usize) -> bool {
    b.get(i).is_some_and(|&c| !is_space(c))
}

/// `[^\w\s]` at `i`.
pub fn punct_at(b: &[u8], i: usize) -> bool {
    non_space_at(b, i) && !is_word(b, i)
}

/// `\W` at `i`, the `\n` at the line's end included.
pub fn non_word_at(b: &[u8], i: usize) -> bool {
    !is_word(b, i)
}

/// Where the first byte from `from` that is one of `set` is, or the end: eight bytes at a time
/// (each byte's zero test: a borrow only ever marks bytes above a true match, so the lowest
/// mark is the first).
pub fn find_any<const N: usize>(b: &[u8], from: usize, set: [u8; N]) -> usize {
    const LO: u64 = 0x0101_0101_0101_0101;
    const HI: u64 = 0x8080_8080_8080_8080;
    let mut i = from;
    while let Some(chunk) = b.get(i..i + 8) {
        let w = u64::from_le_bytes(chunk.try_into().unwrap_or([0; 8]));
        let mut m = 0;
        for x in set {
            let v = w ^ (LO * u64::from(x));
            m |= v.wrapping_sub(LO) & !v & HI;
        }
        if m != 0 {
            return i + (m.trailing_zeros() / 8) as usize;
        }
        i += 8;
    }
    while i < b.len() && !set.contains(&b[i]) {
        i += 1;
    }
    i.min(b.len())
}

/// `(?:\d(?:[\d']*\d)?)` at `q`: where it ends (the longest, ending on a digit).
pub fn digits_quoted(b: &[u8], q: usize) -> Option<usize> {
    if !at(b, q).is_ascii_digit() {
        return None;
    }
    let mut e = q + 1;
    let mut last = e;
    while e < b.len() && (b[e].is_ascii_digit() || b[e] == b'\'') {
        e += 1;
        if b[e - 1].is_ascii_digit() {
            last = e;
        }
    }
    Some(last)
}

/// `[eE][-+]??D` at `q` with `D` from `digits`: where it ends. The sign's `??` is lazy: without
/// it first.
pub fn exponent(
    b: &[u8],
    q: usize,
    marks: [u8; 2],
    digits: fn(&[u8], usize) -> Option<usize>,
) -> Option<usize> {
    if !marks.contains(&at(b, q)) || q >= b.len() {
        return None;
    }
    digits(b, q + 1).or_else(|| {
        matches!(at(b, q + 1), b'-' | b'+')
            .then(|| digits(b, q + 2))
            .flatten()
    })
}

/// `\d+` at `q`: where it ends.
pub fn digits_plain(b: &[u8], q: usize) -> Option<usize> {
    let mut e = q;
    while e < b.len() && b[e].is_ascii_digit() {
        e += 1;
    }
    (e > q).then_some(e)
}
