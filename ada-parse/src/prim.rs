//! The byte-level pieces the scanners share (X-017's first part, the parser audit's item 3): a
//! UTF-8 character's length and its decoding past ASCII, `\w`, `\s` and `\d` as the regex
//! grammars read them (past ASCII a letter or one of Unicode's decimal digits, Unicode's
//! spaces), a name's end with ASCII's run taken at once, byte sets as tables, a search for one
//! of a few bytes eight at a time, and a block's `\n`s 64 bytes a step (X-087). Each was a copy
//! in up to thirteen scanners; the scanners whose classes differ (a name's characters past ASCII,
//! `$` in a name) keep their own or pass theirs in. No behavior of their own: what each did
//! before, the digests hold.

/// ASCII's word bytes, `[A-Za-z0-9_]`.
pub static WORD: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 128 {
        t[c] = (c as u8).is_ascii_alphanumeric() || c == b'_' as usize;
        c += 1;
    }
    t
};

/// The bytes of the UTF-8 character that starts with `c`.
#[inline]
pub const fn char_len(c: u8) -> usize {
    match c {
        0xf0.. => 4,
        0xe0.. => 3,
        0xc0.. => 2,
        _ => 1,
    }
}

/// The character that starts at byte `i` of `b`, past ASCII.
#[cold]
pub fn wide_char(b: &[u8], i: usize) -> Option<char> {
    let e = (i + char_len(b[i])).min(b.len());
    std::str::from_utf8(&b[i..e]).ok()?.chars().next()
}

/// The character that byte `i` of `b` is in, past ASCII.
#[cold]
pub fn wide_char_at(b: &[u8], i: usize) -> Option<char> {
    let s = (i.saturating_sub(3)..=i)
        .rev()
        .find(|&s| b[s] & 0xc0 != 0x80)
        .unwrap_or(i);
    wide_char(b, s)
}

/// Where Unicode's decimal digits past ASCII start, ten in a row each (`\\d` is Unicode's in
/// syntect's regexes): Unicode 16's `Nd`.
pub const ND: [u32; 75] = [
    0x660, 0x6f0, 0x7c0, 0x966, 0x9e6, 0xa66, 0xae6, 0xb66, 0xbe6, 0xc66, 0xce6, 0xd66, 0xde6,
    0xe50, 0xed0, 0xf20, 0x1040, 0x1090, 0x17e0, 0x1810, 0x1946, 0x19d0, 0x1a80, 0x1a90, 0x1b50,
    0x1bb0, 0x1c40, 0x1c50, 0xa620, 0xa8d0, 0xa900, 0xa9d0, 0xa9f0, 0xaa50, 0xabf0, 0xff10,
    0x104a0, 0x10d30, 0x10d40, 0x11066, 0x110f0, 0x11136, 0x111d0, 0x112f0, 0x11450, 0x114d0,
    0x11650, 0x116c0, 0x116d0, 0x116da, 0x11730, 0x118e0, 0x11950, 0x11bf0, 0x11c50, 0x11d50,
    0x11da0, 0x11f50, 0x16130, 0x16a60, 0x16ac0, 0x16b50, 0x16d70, 0x1ccf0, 0x1d7ce, 0x1d7d8,
    0x1d7e2, 0x1d7ec, 0x1d7f6, 0x1e140, 0x1e2f0, 0x1e4f0, 0x1e5f1, 0x1e950, 0x1fbf0,
];

/// Whether `c` is one of Unicode's decimal digits, `\p{Nd}`.
pub fn is_nd(c: char) -> bool {
    let u = c as u32;
    c.is_ascii_digit() || ND.iter().any(|&s| u >= s && u - s < 10)
}

/// Whether the character that byte `i` of `b` is in is a word's, `\w` (and `[[:alnum:]_]`):
/// past ASCII, a letter or a decimal digit (`₁` is neither).
pub fn is_word(b: &[u8], i: usize) -> bool {
    match b.get(i) {
        Some(&c) if c < 0x80 => WORD[usize::from(c)],
        Some(_) => wide_char_at(b, i).is_some_and(|c| c.is_alphabetic() || is_nd(c)),
        None => false,
    }
}

/// [`is_word`] for the character that starts at byte `i`: a byte inside a character is none.
pub fn is_word_starting(b: &[u8], i: usize) -> bool {
    match b.get(i) {
        Some(&c) if c < 0x80 => WORD[usize::from(c)],
        Some(_) => wide_char(b, i).is_some_and(|c| c.is_alphabetic() || is_nd(c)),
        None => false,
    }
}

/// Where the run of `\w` ([`is_word`]) at `p` ends: ASCII's bytes on the table at once, a
/// character past ASCII decoded only where one is.
pub fn word_end(b: &[u8], mut p: usize) -> usize {
    loop {
        while p < b.len() && WORD[usize::from(b[p])] {
            p += 1;
        }
        if p < b.len() && b[p] >= 0x80 && is_word(b, p) {
            p += char_len(b[p]);
        } else {
            return p.min(b.len());
        }
    }
}

/// Whether the character at `i` can start a name, `[[:alpha:]_]`.
pub fn is_name_start(b: &[u8], i: usize) -> bool {
    match b.get(i) {
        Some(&c) if c < 0x80 => c.is_ascii_alphabetic() || c == b'_',
        Some(_) => wide_char(b, i).is_some_and(char::is_alphabetic),
        None => false,
    }
}

/// An ASCII space other than the line's `\n`.
pub fn is_blank(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r')
}

/// `\s` at `p`: how many bytes the space takes.
pub fn space_len(b: &[u8], p: usize) -> Option<usize> {
    let &c = b.get(p)?;
    if c < 0x80 {
        return matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r').then_some(1);
    }
    wide_space(b, p)
}

/// [`space_len`] without the line's `\n`.
pub fn blank_len(b: &[u8], p: usize) -> Option<usize> {
    let &c = b.get(p)?;
    if c < 0x80 {
        return is_blank(c).then_some(1);
    }
    wide_space(b, p)
}

/// A space past ASCII at `p` (one of Unicode's `White_Space` but the separators below `0x80`):
/// how many bytes it takes.
#[cold]
pub fn wide_space(b: &[u8], p: usize) -> Option<usize> {
    let c = wide_char(b, p)?;
    matches!(
        c,
        '\u{85}' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
    .then(|| c.len_utf8())
}

/// Where the spaces ([`space_len`]) at `p` end.
pub fn skip_spaces(b: &[u8], mut p: usize) -> usize {
    while let Some(k) = space_len(b, p) {
        p += k;
    }
    p
}

/// `\d` at `p`, Unicode's decimal digits: where it ends.
pub fn digit(b: &[u8], p: usize) -> Option<usize> {
    match *b.get(p)? {
        c if c.is_ascii_digit() => Some(p + 1),
        c if c < 0x80 => None,
        _ => wide_digit(b, p),
    }
}

/// A decimal digit past ASCII at `p` ([`is_nd`]): where it ends.
#[cold]
pub fn wide_digit(b: &[u8], p: usize) -> Option<usize> {
    let c = wide_char(b, p)?;
    is_nd(c).then(|| p + c.len_utf8())
}

/// A table of the bytes in `bytes`.
pub const fn stops(bytes: &[u8]) -> [bool; 256] {
    let mut t = [false; 256];
    let mut i = 0;
    while i < bytes.len() {
        t[bytes[i] as usize] = true;
        i += 1;
    }
    t
}

/// Where the first byte from `from` that is one of `set` is, or the end: [`find_in`] to the
/// line's end on x86-64, [`find_swar`] elsewhere.
pub fn find_any<const N: usize>(b: &[u8], from: usize, set: [u8; N]) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        find_in(b, from.min(b.len()), b.len(), set)
    }
    #[cfg(not(target_arch = "x86_64"))]
    find_swar(b, from, set)
}

/// [`find_any`] eight bytes at a time (each byte's zero test: a borrow only ever marks bytes
/// above a true match, so the lowest mark is the first): where runs are short, as in
/// TypeScript's lookaheads, cheaper than [`find_in`]'s set-up (X-041).
pub fn find_swar<const N: usize>(b: &[u8], from: usize, set: [u8; N]) -> usize {
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

/// Where the first byte of `b[from..end]` that is one of `set` is, or `end` (`from` where
/// `from >= end`; `end` at most `b.len()`): sixteen bytes a step, the last step read again
/// from `end - 16` with the bytes before `from` masked off, so a run that ends near the line's
/// end is found in one load. SSE2, which every x86-64 has; [`find_swar`]'s eight elsewhere.
#[inline(always)]
pub fn find_in<const N: usize>(b: &[u8], from: usize, end: usize, set: [u8; N]) -> usize {
    const { assert!(N > 0) };
    let end = end.min(b.len());
    if from >= end {
        return from;
    }
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::*;
        // SAFETY: SSE2 is part of x86-64; every load reads `b[q..q + 16]` with `q + 16 <= end
        // <= b.len()` (each call below), and needs no alignment
        unsafe {
            let vs = set.map(|c| _mm_set1_epi8(c as i8));
            let hits = |q: usize| {
                let w = _mm_loadu_si128(b.as_ptr().add(q).cast());
                let mut m = _mm_cmpeq_epi8(w, vs[0]);
                for v in &vs[1..] {
                    m = _mm_or_si128(m, _mm_cmpeq_epi8(w, *v));
                }
                _mm_movemask_epi8(m) as u32
            };
            let mut i = from;
            while i + 16 <= end {
                let m = hits(i);
                if m != 0 {
                    return i + m.trailing_zeros() as usize;
                }
                i += 16;
            }
            if i == end {
                return end;
            }
            if end >= 16 {
                let q = end - 16;
                let m = hits(q) >> (i - q);
                return if m != 0 {
                    i + m.trailing_zeros() as usize
                } else {
                    end
                };
            }
            while i < end && !set.contains(&b[i]) {
                i += 1;
            }
            i
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    find_swar(&b[..end], from, set)
}

/// The `\n`s of `c`, a bit each, byte `i` at bit `i`, and how many: SSE2's four compares on
/// x86-64, the count summed from them (`psadbw`: x86-64's floor has no `popcnt`, and its
/// emulation is 14 instructions), eight bytes a word elsewhere.
#[inline(always)]
pub fn newlines64(c: &[u8; 64]) -> (u64, usize) {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::*;
        // SAFETY: SSE2 is part of x86-64; each load reads 16 of `c`'s 64 bytes, unaligned
        unsafe {
            let nl = _mm_set1_epi8(b'\n' as i8);
            let eq = |q: usize| _mm_cmpeq_epi8(_mm_loadu_si128(c.as_ptr().add(q).cast()), nl);
            let (a, b, d, e) = (eq(0), eq(16), eq(32), eq(48));
            let bits = |v| u64::from(_mm_movemask_epi8(v) as u16);
            let m = bits(a) | bits(b) << 16 | bits(d) << 32 | bits(e) << 48;
            // each `\n` a -1 in its lane, so the lanes' negated sums are the counts (4 at most)
            let zero = _mm_setzero_si128();
            let sum = _mm_sub_epi8(zero, _mm_add_epi8(_mm_add_epi8(a, b), _mm_add_epi8(d, e)));
            let s = _mm_sad_epu8(sum, zero);
            let n = _mm_cvtsi128_si32(s) + _mm_cvtsi128_si32(_mm_unpackhi_epi64(s, s));
            (m, n as usize)
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    newlines64_swar(c)
}

/// [`newlines64`] eight bytes a word, where x86-64's SSE2 isn't.
#[cfg(any(test, not(target_arch = "x86_64")))]
#[inline(always)]
fn newlines64_swar(c: &[u8; 64]) -> (u64, usize) {
    const LO: u64 = 0x0101_0101_0101_0101;
    const LOW7: u64 = 0x7f7f_7f7f_7f7f_7f7f;
    let mut m = 0;
    for (k, w) in c.as_chunks::<8>().0.iter().enumerate() {
        let v = u64::from_le_bytes(*w) ^ (LO * u64::from(b'\n'));
        // a byte's top bit set exactly where it's zero (no borrow between bytes)
        let z = !(((v & LOW7) + LOW7) | v | LOW7);
        // the eight top bits gathered into one byte, byte `i`'s at bit `i`
        m |= ((z >> 7).wrapping_mul(0x0102_0408_1020_4080) >> 56) << (8 * k);
    }
    (m, m.count_ones() as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newlines_a_bit_each() {
        let mut c = [b'a'; 64];
        assert_eq!(newlines64(&c), (0, 0));
        for i in [0, 1, 7, 8, 15, 16, 31, 32, 47, 48, 62, 63] {
            c[i] = b'\n';
        }
        c[5] = 0x8a;
        c[6] = 0x0b;
        c[9] = 0x09;
        let want = (0..64)
            .filter(|&i| c[i] == b'\n')
            .fold(0u64, |m, i| m | 1 << i);
        assert_eq!(newlines64(&c), (want, want.count_ones() as usize));
        assert_eq!(newlines64(&[b'\n'; 64]), (u64::MAX, 64));
        for c in [c, [b'\n'; 64], [0x8a; 64], [0x0a ^ 0x80; 64]] {
            assert_eq!(newlines64_swar(&c), newlines64(&c));
        }
        let mut c = [0u8; 64];
        for (i, x) in c.iter_mut().enumerate() {
            *x = (i as u8).wrapping_mul(37) ^ 0x0a;
        }
        assert_eq!(newlines64_swar(&c), newlines64(&c));
    }

    #[test]
    fn words_spaces_and_digits_past_ascii() {
        let b = "a_1é٣₁ x".as_bytes();
        assert_eq!(word_end(b, 0), 7, "é and ٣ are \\w, ₁ isn't");
        assert!(is_word(b, 4), "inside é");
        assert!(!is_word_starting(b, 4), "inside é");
        assert_eq!(digit(b, 5), Some(7));
        assert_eq!(digit(b, 7), None);
        assert_eq!(space_len("\u{3000}".as_bytes(), 0), Some(3));
        assert_eq!(space_len(b"\n", 0), Some(1));
        assert_eq!(blank_len(b"\n", 0), None);
        assert_eq!(find_any(b"0123456789abcdef;", 0, *b";#"), 16);
        assert_eq!(find_any(b"abc", 1, *b";"), 3);
        assert_eq!(find_any(b"abc", 5, *b";"), 3);
        assert_eq!(find_swar(b"0123456789abcdef;", 0, *b";#"), 16);
        assert_eq!(find_swar(b"abc", 5, *b";"), 3);
        let long = b"0123456789abcdef0123456789\"x\\";
        for from in 0..long.len() {
            for end in from..=long.len() {
                let want = (from..end)
                    .find(|&i| b"\"\\".contains(&long[i]))
                    .unwrap_or(end);
                assert_eq!(find_in(long, from, end, *b"\"\\"), want, "{from}..{end}");
            }
        }
        assert_eq!(find_in(b"abc", 3, 1, *b"a"), 3);
    }
}
