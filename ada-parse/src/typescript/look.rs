//! The grammar's lookaheads as scans of the line. bat's grammar copies a handful of regexes into
//! each other (a type's `<…>`, an arrow's parameters and return type, a value that is a
//! function), nesting brackets to three levels by copying them; each is one function here, with
//! the same bounded nesting. The line is read with its `\n`, as syntect reads it: byte `n` is
//! `\n`, and `$` holds at `n` and past it.

/// Byte `i` of `b`, `\n` at its end, 0 past it.
#[inline]
pub(super) fn at(b: &[u8], i: usize) -> u8 {
    if i < b.len() {
        b[i]
    } else if i == b.len() {
        b'\n'
    } else {
        0
    }
}

/// `$` at `i`.
#[inline]
fn eol(b: &[u8], i: usize) -> bool {
    i >= b.len()
}

/// Past `\s` from `i`, the line's `\n` included.
#[inline]
pub(super) fn ws(b: &[u8], mut i: usize) -> usize {
    while matches!(at(b, i), b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        i += 1;
    }
    i
}

/// The character at `i`, past ASCII.
#[cold]
fn wide(b: &[u8], i: usize) -> Option<char> {
    let e = (i + super::char_len(b[i])).min(b.len());
    std::str::from_utf8(&b[i..e]).ok()?.chars().next()
}

/// The character that ends just before `i`, past ASCII.
#[cold]
fn wide_before(b: &[u8], i: usize) -> Option<char> {
    let s = (i.saturating_sub(4)..i)
        .rev()
        .find(|&s| b[s] & 0xc0 != 0x80)?;
    wide(b, s)
}

/// [`CLASS`]: an ASCII name character, `[_$a-zA-Z0-9]`.
pub(super) const NAME: u8 = 1;
/// [`CLASS`]: an ASCII name's first character, `[_$a-zA-Z]`.
pub(super) const START: u8 = 2;
/// [`CLASS`]: a byte past ASCII, its character decoded to be tested (`wide`).
pub(super) const HIGH: u8 = 4;
/// [`CLASS`]: `.`.
pub(super) const DOT: u8 = 8;
/// [`CLASS`]: `\s`, a blank or a line's end.
pub(super) const SPACE: u8 = 16;

/// Each byte's classes: what the predicates test, one load each (a chain of compares before, and
/// the bytes past ASCII tested by a call the hot path saved registers for).
pub(super) static CLASS: [u8; 256] = {
    let mut t = [0; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        t[c] = match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' => NAME | START,
            b'0'..=b'9' => NAME,
            b'.' => DOT,
            b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c => SPACE,
            0x80.. => HIGH,
            _ => 0,
        };
        c += 1;
    }
    t
};

/// Byte `c`'s classes.
#[inline(always)]
pub(super) fn class(c: u8) -> u8 {
    CLASS[usize::from(c)]
}

/// `[_$[:alpha:]]` at `i`: its length, 0 if none.
#[inline]
pub(super) fn alpha(b: &[u8], i: usize) -> usize {
    match b.get(i) {
        Some(&c) if class(c) & START != 0 => 1,
        Some(&c) if c >= 0x80 => alpha_wide(b, i),
        _ => 0,
    }
}

/// [`alpha`] past ASCII.
#[cold]
#[inline(never)]
fn alpha_wide(b: &[u8], i: usize) -> usize {
    match wide(b, i) {
        Some(ch) if ch.is_alphabetic() => ch.len_utf8(),
        _ => 0,
    }
}

/// `[_$[:alnum:]]` at `i`: its length, 0 if none.
#[inline]
pub(super) fn alnum(b: &[u8], i: usize) -> usize {
    match b.get(i) {
        Some(&c) if class(c) & NAME != 0 => 1,
        Some(&c) if c >= 0x80 => alnum_wide(b, i),
        _ => 0,
    }
}

/// [`alnum`] past ASCII.
#[cold]
#[inline(never)]
pub(super) fn alnum_wide(b: &[u8], i: usize) -> usize {
    match wide(b, i) {
        Some(ch) if ch.is_alphanumeric() => ch.len_utf8(),
        _ => 0,
    }
}

/// Whether a name's character starts at `i`.
#[inline]
pub(super) fn alnum_at(b: &[u8], i: usize) -> bool {
    alnum(b, i) > 0
}

/// Whether a name's character ends just before `i`.
#[inline]
pub(super) fn alnum_before(b: &[u8], i: usize) -> bool {
    // `i == 0` wraps to a place past any line, as does one past the line's `\n`
    match b.get(i.wrapping_sub(1)) {
        Some(&c) if class(c) & NAME != 0 => true,
        Some(&c) if c >= 0x80 => alnum_wide_before(b, i),
        _ => false,
    }
}

/// [`alnum_before`] past ASCII.
#[cold]
#[inline(never)]
pub(super) fn alnum_wide_before(b: &[u8], i: usize) -> bool {
    wide_before(b, i).is_some_and(char::is_alphanumeric)
}

/// `\w` at `i`.
#[inline]
pub(super) fn is_word_at(b: &[u8], i: usize) -> bool {
    match at(b, i) {
        b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' => true,
        c if c >= 0x80 && i < b.len() => wide(b, i).is_some_and(char::is_alphanumeric),
        _ => false,
    }
}

/// `\w` just before `i`.
#[inline]
pub(super) fn is_word_before(b: &[u8], i: usize) -> bool {
    if i == 0 || i > b.len() {
        return false;
    }
    match b[i - 1] {
        b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' => true,
        c if c >= 0x80 => wide_before(b, i).is_some_and(char::is_alphanumeric),
        _ => false,
    }
}

/// The end of a name `[_$[:alpha:]][_$[:alnum:]]*` at `i`; `i` if none starts there. The first
/// byte is tested here, inlined, so that a place where no name starts costs a load and a test;
/// the rest is [`name_from`]'s, which saves no registers.
#[inline(always)]
pub(super) fn name(b: &[u8], i: usize) -> usize {
    match b.get(i) {
        Some(&c) if class(c) & (START | HIGH) != 0 => name_from(b, i, c),
        _ => i,
    }
}

/// [`name`] from `i`, where `c`, an ASCII name's first byte or one past ASCII, is: ASCII's names
/// to their end, anything else [`name_wide`]'s.
#[inline(never)]
fn name_from(b: &[u8], i: usize, c: u8) -> usize {
    if c < 0x80 {
        let j = ascii_name_end(b, i + 1);
        if j >= b.len() || b[j] < 0x80 {
            return j;
        }
    }
    name_wide(b, i)
}

/// [`name`] at `i` with characters past ASCII in it (or at `i`), read again from its start.
#[cold]
#[inline(never)]
fn name_wide(b: &[u8], i: usize) -> usize {
    let a = alpha(b, i);
    if a == 0 {
        return i;
    }
    let mut j = i + a;
    loop {
        j = ascii_name_end(b, j);
        if j >= b.len() || b[j] < 0x80 {
            return j;
        }
        let k = alnum(b, j);
        if k == 0 {
            return j;
        }
        j += k;
    }
}

/// Past ASCII's name characters (`[_$a-zA-Z0-9]`) from `j` (`j <= b.len()`): sixteen bytes a step
/// on x86-64 where the line has sixteen, the last sixteen read again where fewer are left past
/// `j`; a byte a step otherwise.
#[inline(always)]
fn ascii_name_end(b: &[u8], mut j: usize) -> usize {
    #[cfg(target_arch = "x86_64")]
    if b.len() >= 16 {
        while j < b.len() {
            let q = j.min(b.len() - 16);
            let s = j - q;
            // the bytes from `j` that aren't a name's, the line's end past them
            let stop = !(name_mask(b, q) >> s) & (0xffff >> s);
            if stop != 0 {
                return j + stop.trailing_zeros() as usize;
            }
            j = q + 16;
        }
        return j;
    }
    while j < b.len() && class(b[j]) & NAME != 0 {
        j += 1;
    }
    j
}

/// Of the sixteen bytes of `b` at `q` (`q + 16 <= b.len()`), a bit for each in `[_$a-zA-Z0-9]`.
/// SSE2, which every x86-64 has.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn name_mask(b: &[u8], q: usize) -> u32 {
    use std::arch::x86_64::*;
    assert!(q + 16 <= b.len());
    // SAFETY: SSE2 is part of x86-64; the load reads `b[q..q + 16]`, inside `b` (above), and
    // needs no alignment
    unsafe {
        let w = _mm_loadu_si128(b.as_ptr().add(q).cast());
        // `x` in `lo..=lo + n`: `x - lo + 0x80` below `n + 1 - 0x80`, signed (one add and one
        // compare, where an unsigned range takes a subtract, a min and a compare)
        let within = |x: __m128i, lo: u8, n: u8| {
            let d = _mm_add_epi8(x, _mm_set1_epi8(0x80u8.wrapping_sub(lo) as i8));
            _mm_cmplt_epi8(d, _mm_set1_epi8((n as i8).wrapping_sub(127)))
        };
        // `| 0x20` takes `A..=Z` to `a..=z` and no other byte there
        let letter = within(_mm_or_si128(w, _mm_set1_epi8(0x20)), b'a', 25);
        let digit = within(w, b'0', 9);
        let us = _mm_cmpeq_epi8(w, _mm_set1_epi8(b'_' as i8));
        let dollar = _mm_cmpeq_epi8(w, _mm_set1_epi8(b'$' as i8));
        let m = _mm_or_si128(_mm_or_si128(letter, digit), _mm_or_si128(us, dollar));
        _mm_movemask_epi8(m) as u32
    }
}

/// `[[:upper:]][_$[:digit:][:upper:]]*` at `i`, ASCII's: its end, `i` if none.
pub(super) fn upper(b: &[u8], i: usize) -> usize {
    if !at(b, i).is_ascii_uppercase() {
        return i;
    }
    let mut j = i + 1;
    while matches!(at(b, j), b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'$') {
        j += 1;
    }
    j
}

/// A bracket group `open … close` at `i`, nested up to `depth` levels with nothing else
/// counted: `(\{([^\{\}]|(\{([^\{\}]|\{[^\{\}]*\})*\}))*\})` and its kin. Its end.
pub(super) fn bal(b: &[u8], i: usize, open: u8, close: u8, depth: u8) -> Option<usize> {
    if at(b, i) != open || depth == 0 {
        return None;
    }
    // one pass with the level counted (the line's `\n` is neither bracket): an `open` at the
    // deepest level fails it, as the nested copies of the regex run out
    let mut level = 1;
    let mut j = i + 1;
    loop {
        // the next bracket of either kind, eight bytes a step
        j = crate::prim::find_swar(b, j, [open, close]);
        if j >= b.len() {
            return None;
        }
        if b[j] == close {
            level -= 1;
            if level == 0 {
                return Some(j + 1);
            }
        } else {
            if level == depth {
                return None;
            }
            level += 1;
        }
        j += 1;
    }
}

/// Whether `=>` starts at `i` or past it on the line: a test every arrow's lookahead needs to
/// pass, as each ends in `\s*=>`.
fn arrow_ahead(b: &[u8], i: usize) -> bool {
    let mut j = i + 1;
    loop {
        j = crate::prim::find_swar(b, j, *b">");
        if j >= b.len() {
            return false;
        }
        if b[j - 1] == b'=' {
            return true;
        }
        j += 1;
    }
}

/// `«B{»`, `«B(»` or `«B[»` at `i`.
fn bal3(b: &[u8], i: usize) -> Option<usize> {
    match at(b, i) {
        b'{' => bal(b, i, b'{', b'}', 3),
        b'(' => bal(b, i, b'(', b')', 3),
        b'[' => bal(b, i, b'[', b']', 3),
        _ => None,
    }
}

/// `«TP»` at `i`: `<…>` of type parameters, three levels deep, and the blanks after it.
pub(super) fn tp(b: &[u8], i: usize) -> Option<usize> {
    fn level(b: &[u8], i: usize, depth: u8) -> Option<usize> {
        // `<\s*(«alpha»|«B{»|«B(»|«B[»)([^=<>]|=[^<]|\<…)*\>`
        if at(b, i) != b'<' {
            return None;
        }
        let mut j = ws(b, i + 1);
        let a = alpha(b, j);
        j = if a > 0 { j + a } else { bal3(b, j)? };
        loop {
            match at(b, j) {
                0 => return None,
                b'>' => return Some(j + 1),
                b'<' if depth > 1 => j = level(b, j, depth - 1)?,
                b'<' => return None,
                b'=' => {
                    if matches!(at(b, j + 1), b'<' | 0) {
                        return None;
                    }
                    j += 2;
                }
                _ => j += 1,
            }
        }
    }
    level(b, i, 3).map(|j| ws(b, j))
}

/// `«cm*»`: `(\/\*([^\*]|(\*[^\/]))*\*\/\s*)*` at `i`, its end.
pub(super) fn comments(b: &[u8], mut i: usize) -> usize {
    'outer: while at(b, i) == b'/' && at(b, i + 1) == b'*' {
        let mut j = i + 2;
        loop {
            match at(b, j) {
                0 => break 'outer,
                b'*' if at(b, j + 1) == b'/' => {
                    i = ws(b, j + 2);
                    continue 'outer;
                }
                b'*' => {
                    if at(b, j + 1) == 0 {
                        break 'outer;
                    }
                    j += 2;
                }
                _ => j += 1,
            }
        }
    }
    i
}

/// A quoted string `q([^q\\]|\\.)*q` at `i`, its end.
pub(super) fn quoted(b: &[u8], i: usize, q: u8) -> Option<usize> {
    if at(b, i) != q {
        return None;
    }
    let mut j = i + 1;
    loop {
        match at(b, j) {
            0 => return None,
            c if c == q => return Some(j + 1),
            b'\\' => {
                if at(b, j + 1) == 0 || at(b, j + 1) == b'\n' {
                    // `.` doesn't match the `\n`
                    return None;
                }
                j += 2;
            }
            _ => j += 1,
        }
    }
}

/// One of `«RT»`'s tokens at `i`, `[^<>\(\)\{\}]|\<([^<>]|\<([^<>]|\<[^<>]+\>)+\>)+\>|\([^\(\)]+\)|\{[^\{\}]+\}`: its end.
fn rt(b: &[u8], i: usize) -> Option<usize> {
    fn angle(b: &[u8], i: usize, depth: u8) -> Option<usize> {
        // `\<( [^<>] | \<…\> )+\>`, the innermost `\<[^<>]+\>`
        let mut j = i + 1;
        let mut any = false;
        loop {
            match at(b, j) {
                0 => return None,
                b'>' => return any.then_some(j + 1),
                b'<' if depth > 1 => j = angle(b, j, depth - 1)?,
                b'<' => return None,
                _ => j += 1,
            }
            any = true;
        }
    }
    fn flat(b: &[u8], i: usize, open: u8, close: u8) -> Option<usize> {
        let mut j = i + 1;
        loop {
            match at(b, j) {
                0 => return None,
                c if c == close => return (j > i + 1).then_some(j + 1),
                c if c == open => return None,
                _ => j += 1,
            }
        }
    }
    match at(b, i) {
        0 => None,
        b'<' => angle(b, i, 3),
        b'(' => flat(b, i, b'(', b')'),
        b'{' => flat(b, i, b'{', b'}'),
        b'>' | b')' | b'}' => None,
        c => Some(i + if c >= 0x80 { super::char_len(c) } else { 1 }),
    }
}

/// `«RT»+` from `i`, then `then` at one of its tokens' ends (any, as the regex backtracks).
fn rt_then(b: &[u8], mut i: usize, then: impl Fn(usize) -> bool) -> bool {
    while let Some(j) = rt(b, i) {
        if then(j) {
            return true;
        }
        i = j;
    }
    false
}

/// `\s*=>` at `i`.
fn arrow_at(b: &[u8], i: usize) -> bool {
    let j = ws(b, i);
    at(b, j) == b'=' && at(b, j + 1) == b'>'
}

/// Optional `«TP»` at `i`: where the next part starts, `None` if a `<` doesn't open one.
fn tp_opt(b: &[u8], i: usize) -> Option<usize> {
    if at(b, i) == b'<' { tp(b, i) } else { Some(i) }
}

/// The sure shot arrow: `«TP»?[(]\s*«cm*»(([)]\s*:)|((\.\.\.\s*)?«name»\s*:))` at `i`.
pub(super) fn arrow_sure(b: &[u8], i: usize) -> bool {
    let Some(j) = tp_opt(b, i) else { return false };
    if at(b, j) != b'(' {
        return false;
    }
    let j = comments(b, ws(b, j + 1));
    if at(b, j) == b')' {
        return at(b, ws(b, j + 1)) == b':';
    }
    let j = if crate::has_at(b, j.min(b.len()), b"...") {
        ws(b, j + 3)
    } else {
        j
    };
    let e = name(b, j);
    e > j && at(b, ws(b, e)) == b':'
}

/// The arrow found on its line: `«TP»?\(\s*«cm*»((«alpha»|«B{»|«B[»|(\.\.\.\s*«alpha»))([^()'"`]|«B(»|«sq»|«dq»|«bq»)*)?\)(\s*:\s*«RT»+)?\s*=>` at `i`.
pub(super) fn arrow_line(b: &[u8], i: usize) -> bool {
    let Some(j) = tp_opt(b, i) else { return false };
    // the `=>` it ends in, past the `(`
    if at(b, j) != b'(' || !arrow_ahead(b, j) {
        return false;
    }
    let mut j = comments(b, ws(b, j + 1));
    if at(b, j) != b')' {
        // the first parameter's start
        let a = alpha(b, j);
        j = if a > 0 {
            j + a
        } else if matches!(at(b, j), b'{' | b'[') {
            match bal3(b, j) {
                Some(e) => e,
                None => return false,
            }
        } else if crate::has_at(b, j.min(b.len()), b"...") {
            let k = ws(b, j + 3);
            let a = alpha(b, k);
            if a == 0 {
                return false;
            }
            k + a
        } else {
            return false;
        };
        loop {
            match at(b, j) {
                0 | b')' => break,
                b'(' => match bal(b, j, b'(', b')', 3) {
                    Some(e) => j = e,
                    None => return false,
                },
                q @ (b'\'' | b'"' | b'`') => match quoted(b, j, q) {
                    Some(e) => j = e,
                    None => return false,
                },
                c => j += if c >= 0x80 { super::char_len(c) } else { 1 },
            }
        }
        if at(b, j) != b')' {
            return false;
        }
    }
    let j = j + 1;
    if arrow_at(b, j) {
        return true;
    }
    let k = ws(b, j);
    // `:\s*«RT»+`: a blank is a `«RT»` too, so the units start at once (`(a): => x`)
    at(b, k) == b':' && rt_then(b, k + 1, |e| arrow_at(b, e))
}

/// `(«B{»|«B[»)\s*((:\s*[{[]?$)|((\s*«RT»+\s*)?=\s*))` at `i`, a destructured parameter.
fn destructured(b: &[u8], i: usize) -> bool {
    let (open, close) = if at(b, i) == b'{' {
        (b'{', b'}')
    } else {
        (b'[', b']')
    };
    let Some(e) = bal(b, i, open, close, 3) else {
        return false;
    };
    let j = ws(b, e);
    if at(b, j) == b':' {
        let mut k = ws(b, j + 1);
        if at(b, k) == open {
            k += 1;
        }
        if eol(b, k) {
            return true;
        }
    }
    if at(b, e) == b'=' || at(b, j) == b'=' {
        return true;
    }
    rt_then(b, e, |k| at(b, k) == b'=' || at(b, ws(b, k)) == b'=')
}

/// `(([\{\[]\s*)?$)|destructured` at `j` (after a `(` and its blanks): parameters that go on
/// past the line, or a destructured one.
pub(super) fn open_tail(b: &[u8], j: usize) -> bool {
    if eol(b, j) {
        return true;
    }
    if matches!(at(b, j), b'{' | b'[') {
        if ws(b, j + 1) >= b.len() {
            return true;
        }
        return destructured(b, j);
    }
    false
}

/// `(<\s*$)|(«TP»?[\(]\s*OPEN)` at `i`: a value whose parameters go on past the line. TSX's has no
/// `«TP»?` (a `<` there may be an element).
fn open_params(b: &[u8], i: usize, tsx: bool) -> bool {
    if at(b, i) == b'<' && ws(b, i + 1) >= b.len() {
        return true;
    }
    let Some(j) = (if tsx { Some(i) } else { tp_opt(b, i) }) else {
        return false;
    };
    at(b, j) == b'(' && open_tail(b, ws(b, j + 1))
}

/// TSX's `[<]\s*«name»\s+extends\s*[^=>]` at `i`: type parameters, not an element.
pub(super) fn tp_extends(b: &[u8], i: usize) -> bool {
    if at(b, i) != b'<' {
        return false;
    }
    let j = ws(b, i + 1);
    let e = name(b, j);
    if e == j || ws(b, e) == e {
        return false;
    }
    let k = ws(b, e);
    crate::has_at(b, k.min(b.len()), b"extends") && !matches!(at(b, k + 7), b'=' | b'>' | 0)
}

/// `\s*«RT»+\s*=>` at `i`.
pub(super) fn rt_arrow(b: &[u8], i: usize) -> bool {
    // `\s*«RT»+\s*=>`: a blank is a `«RT»` too (`[^<>\(\)\{\}]`), so `: =>` matches
    rt_then(b, i, |e| arrow_at(b, e))
}

/// `FNV`, a value that is a function, at `i`:
/// `(async\s+)?((function\s*[(<*])|(function\s+)|(«name»\s*=>))|(async\s*)?(open|sure|line)`;
/// TSX's `(open|sure|extends|line)`.
pub(super) fn fn_value(b: &[u8], i: usize, tsx: bool) -> bool {
    let rest = &b[i.min(b.len())..];
    // every alternative starts with a name (`async`, `function`, one before `=>`), a `(` or a
    // `<`; a name other than `async` and `function` is one only before `=>`
    match at(b, i) {
        b'(' | b'<' => {}
        b'a' | b'f' if crate::starts(rest, b"async") || crate::starts(rest, b"function") => {}
        _ => {
            let e = name(b, i);
            return e > i && arrow_at(b, e);
        }
    }
    let after_async = |gap_needed: bool| -> Option<usize> {
        if crate::starts(rest, b"async") {
            let j = ws(b, i + 5);
            if gap_needed && j == i + 5 {
                return None;
            }
            Some(j)
        } else {
            None
        }
    };
    let first = |k: usize| -> bool {
        let r = &b[k.min(b.len())..];
        if crate::starts(r, b"function") {
            let j = ws(b, k + 8);
            if matches!(at(b, j), b'(' | b'<' | b'*') || j > k + 8 {
                return true;
            }
        }
        let e = name(b, k);
        e > k && arrow_at(b, e)
    };
    if first(i) || after_async(true).is_some_and(first) {
        return true;
    }
    let second = |k: usize| {
        open_params(b, k, tsx) || arrow_sure(b, k) || (tsx && tp_extends(b, k)) || arrow_line(b, k)
    };
    second(i) || after_async(false).is_some_and(second)
}

/// After a name being declared (`var-single-variable`'s, a parameter's, a field's): whether it
/// is a function, `(?=\s*(=\s*FNV)|(:\s*…)|…)`, from `i` (after the name and its `?` or `!`). The
/// lookahead's `\s*` is its first alternative's alone (the `|` are at its top level), so only `=`
/// may follow blanks; each `:` alternative needs the `:` at `i` (`a : () => void` is no function).
pub(super) fn fn_decl(b: &[u8], i: usize, tsx: bool) -> bool {
    let w = ws(b, i);
    if at(b, w) == b'=' {
        return fn_value(b, ws(b, w + 1), tsx);
    }
    at(b, i) == b':' && fn_colon(b, i, tsx, true)
}

/// `(<) | ([(]\s*(([)])|(\.\.\.)|([_$[:alnum:]]+\s*(([:,?=])|([)]\s*=>)))))` at `j` (after a `:` and
/// blanks): a function type's start, the one `:` alternative a type's `( … )` lets blanks precede.
pub(super) fn fn_type_head(b: &[u8], j: usize) -> bool {
    if at(b, j) == b'<' {
        return true;
    }
    if at(b, j) == b'(' {
        let k = ws(b, j + 1);
        if at(b, k) == b')' || crate::has_at(b, k.min(b.len()), b"...") {
            return true;
        }
        let mut e = k;
        while alnum(b, e) > 0 {
            e += alnum(b, e);
        }
        if e > k {
            let f = ws(b, e);
            if matches!(at(b, f), b':' | b',' | b'?' | b'=')
                || (at(b, f) == b')' && arrow_at(b, f + 1))
            {
                return true;
            }
        }
    }
    false
}

/// The `:` alternatives of [`fn_decl`]'s lookahead, at the `:` at `i`; with `value`, the last
/// one too, `:\s*(…)+=\s*FNV`, which a type's `( … )` leaves out (no initializer there).
pub(super) fn fn_colon(b: &[u8], i: usize, tsx: bool, value: bool) -> bool {
    match at(b, i) {
        b':' => {
            let j = ws(b, i + 1);
            // a function type's head and open parameters start with `<` or `(`, `Function`
            // with its `F`: a type that starts with anything else is only the last alternative's
            match at(b, j) {
                b'<' | b'(' => {
                    if fn_type_head(b, j) || open_params(b, j, tsx) {
                        return true;
                    }
                }
                // :\s*«wb»Function«we»
                b'F' if crate::has_at(b, j.min(b.len()), b"Function")
                    && !alnum_at(b, j + 8)
                    && (at(b, j + 8) != b'.' || crate::has_at(b, j + 8, b"...")) =>
                {
                    return true;
                }
                _ => {}
            }
            if !value {
                return false;
            }
            // :\s*(=>|«B(»|(<[^<>]*>)|[^<>(),=])+=\s*FNV
            let mut k = j;
            let mut any = false;
            loop {
                // past the `[^<>(),=]` units, eight bytes a step: the line's end fails it (its
                // `\n` one more unit, and nothing after it)
                let f = crate::prim::find_swar(b, k, *b"=(<>),");
                any |= f > k;
                k = f;
                if k >= b.len() {
                    return false;
                }
                match b[k] {
                    b'=' if at(b, k + 1) == b'>' => k += 2,
                    b'=' => return any && fn_value(b, ws(b, k + 1), tsx),
                    b'(' => match bal(b, k, b'(', b')', 3) {
                        Some(e) => k = e,
                        None => return false,
                    },
                    b'<' => {
                        let e = crate::prim::find_swar(b, k + 1, *b"<>");
                        if at(b, e) != b'>' {
                            return false;
                        }
                        k = e + 1;
                    }
                    _ => return false,
                }
                any = true;
            }
        }
        _ => false,
    }
}

/// `START` of a call's type arguments: `((keyof|infer|typeof|readonly)\s+)|((«name»|«B{»|«B(»|«B[»|«sq»|«dq»|«bq»)(?=\s*([\<\>\,\.\[]|=>|&(?!&)|\|(?!\|))))`.
fn ta_start(b: &[u8], i: usize) -> Option<usize> {
    let r = &b[i.min(b.len())..];
    for w in [&b"keyof"[..], b"infer", b"typeof", b"readonly"] {
        if crate::starts(r, w) {
            let j = ws(b, i + w.len());
            if j > i + w.len() {
                return Some(j);
            }
        }
    }
    let e = name(b, i);
    let e = if e > i {
        e
    } else {
        match at(b, i) {
            b'{' | b'(' | b'[' => bal3(b, i)?,
            q @ (b'\'' | b'"' | b'`') => quoted(b, i, q)?,
            _ => return None,
        }
    };
    let j = ws(b, e);
    let ok = match at(b, j) {
        b'<' | b'>' | b',' | b'.' | b'[' => true,
        b'=' => at(b, j + 1) == b'>',
        b'&' => at(b, j + 1) != b'&',
        b'|' => at(b, j + 1) != b'|',
        _ => false,
    };
    ok.then_some(e)
}

/// A call's type arguments, `<…>` three levels deep, at `i`: its end.
pub(super) fn type_args(b: &[u8], i: usize) -> Option<usize> {
    fn level(b: &[u8], i: usize, depth: u8) -> Option<usize> {
        if at(b, i) != b'<' {
            return None;
        }
        let mut j = ta_start(b, ws(b, i + 1))?;
        loop {
            match at(b, j) {
                0 => return None,
                b'>' if j > 0 && at(b, j - 1) == b'=' => j += 1,
                b'>' => return Some(j + 1),
                b'<' if depth > 1 => j = level(b, j, depth - 1)?,
                b'<' => return None,
                b'(' => j = bal(b, j, b'(', b')', 3)?,
                _ => j += 1,
            }
        }
    }
    level(b, i, 3)
}

/// What follows a call's target: `\s*(?:(\?\.\s*)|(\!))?(«TA»\s*)?\(` at `i`.
pub(super) fn call_after(b: &[u8], i: usize) -> bool {
    let mut j = ws(b, i);
    if at(b, j) == b'?' && at(b, j + 1) == b'.' {
        j = ws(b, j + 2);
    } else if at(b, j) == b'!' {
        j += 1;
    }
    if at(b, j) == b'<' {
        match type_args(b, j) {
            Some(e) => j = ws(b, e),
            None => return false,
        }
    }
    at(b, j) == b'('
}

/// Whether a call follows at `i`, a target with a call after it or a call at once:
/// `call_target(b, i).is_some() || call_after(b, i)`, the bytes that start neither turned away
/// first (after a call's `)`, mostly `;`, `)`, `,` or `.`).
#[inline]
pub(super) fn call_follows(b: &[u8], i: usize) -> bool {
    match at(b, i) {
        b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' | b'.' | b'?' | 0x80.. => {
            call_target(b, i).is_some() || call_after(b, i)
        }
        // a target starts with a name, `.` or `?`
        b'!' | b'<' | b'(' | b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c => call_after(b, i),
        _ => false,
    }
}

/// A call's target from `i`, `((«name»)(\s*\??\.\s*(\#?«name»))*)|(\??\.\s*\#?«name»)`, with
/// a call after it: the end of the target, the longest that works.
pub(super) fn call_target(b: &[u8], i: usize) -> Option<usize> {
    let mut ends: [usize; 16] = [0; 16];
    let mut n = 0;
    let mut j = i;
    let e = name(b, j);
    if e > j {
        j = e;
    } else {
        // \??\.\s*\#?«name»
        let mut k = j;
        if at(b, k) == b'?' {
            k += 1;
        }
        if at(b, k) != b'.' {
            return None;
        }
        k = ws(b, k + 1);
        if at(b, k) == b'#' {
            k += 1;
        }
        let e = name(b, k);
        if e == k {
            return None;
        }
        return call_after(b, e).then_some(e);
    }
    ends[0] = j;
    n += 1;
    loop {
        let mut k = ws(b, j);
        if at(b, k) == b'?' {
            k += 1;
        }
        if at(b, k) != b'.' {
            break;
        }
        k = ws(b, k + 1);
        if at(b, k) == b'#' {
            k += 1;
        }
        let e = name(b, k);
        if e == k {
            break;
        }
        j = e;
        if n < ends.len() {
            ends[n] = j;
            n += 1;
        } else {
            ends[n - 1] = j;
        }
    }
    ends[..n].iter().rev().copied().find(|&e| call_after(b, e))
}

#[cfg(test)]
mod tests {
    use super::{NAME, ascii_name_end, class};

    /// The sixteen-byte steps end where the byte loop does, at every start of every line from 0 to
    /// 40 bytes, each byte of 256 at each place in turn among name bytes.
    #[test]
    fn ascii_name_end_is_the_byte_loops() {
        let slow = |b: &[u8], mut j: usize| {
            while j < b.len() && class(b[j]) & NAME != 0 {
                j += 1;
            }
            j
        };
        for n in 0..=40 {
            for at in 0..n.max(1) {
                for c in 0..=255u8 {
                    let mut b: Vec<u8> = (0..n).map(|k| b"aZ09_$xQ"[k % 8]).collect();
                    if at < n {
                        b[at] = c;
                    }
                    for j in 0..=n {
                        assert_eq!(ascii_name_end(&b, j), slow(&b, j), "{b:?} from {j}");
                    }
                }
            }
        }
    }
}
