//! The C family's lexical core (X-015; the parser audit's item 1): what a tokenizer-first
//! scanner of a C-like language reads before it classifies, shared by the languages built on it
//! (Java first). A byte's class is one table lookup ([`CLASS`]); a name is read once, ASCII a
//! byte at a time on the table and past ASCII by characters in a cold path ([`ident_end`]),
//! with the classes its bytes had ([`Ident`]), so that a classifier tells a constant's
//! all-capitals from a type's capital without reading the name again; and a keyword is looked
//! up once in a table hashed at compile time ([`crate::words::Words`]). The languages differ in parameters,
//! not code: which characters past ASCII a name takes, whether `$` is one of its bytes.
pub use crate::prim::char_len;

/// A blank: `\s` as far as ASCII goes.
pub const SPACE: u8 = 1;
/// `\w` in ASCII: a letter, a digit, `_`.
pub const WORD: u8 = 2;
/// `$`, a name's byte where the language says so.
pub const DOLLAR: u8 = 4;
/// A byte past ASCII, which a name may take by its character.
pub const HIGH: u8 = 8;
pub const DIGIT: u8 = 16;
pub const UPPER: u8 = 32;
pub const LOWER: u8 = 64;

/// Each byte's classes.
pub static CLASS: [u8; 256] = {
    let mut t = [0; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        t[c] = if b >= 0x80 {
            HIGH
        } else if b.is_ascii_uppercase() {
            WORD | UPPER
        } else if b.is_ascii_lowercase() {
            WORD | LOWER
        } else if b.is_ascii_digit() {
            WORD | DIGIT
        } else if b == b'_' {
            WORD
        } else if b == b'$' {
            DOLLAR
        } else if matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r') {
            SPACE
        } else {
            0
        };
        c += 1;
    }
    t
};

/// The classes a name's bytes had, OR-ed: [`UPPER`], [`LOWER`] and [`DIGIT`] for its ASCII
/// letters and digits, [`DOLLAR`], and [`HIGH`] for a character past ASCII (whose case isn't
/// read: a classifier asks the character itself).
#[derive(Clone, Copy)]
pub struct Ident {
    pub end: usize,
    pub bits: u8,
}

/// Where the blanks at `p` end.
#[inline(always)]
pub fn skip_blanks(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && CLASS[usize::from(b[p])] & SPACE != 0 {
        p += 1;
    }
    p
}

/// The character at `i`, a byte past ASCII that starts one ('\u{fffd}' for a broken one), and
/// where it ends.
#[cold]
pub fn wide_at(b: &[u8], i: usize) -> (char, usize) {
    let e = (i + char_len(b[i])).min(b.len());
    let c = std::str::from_utf8(&b[i..e])
        .ok()
        .and_then(|t| t.chars().next())
        .unwrap_or('\u{fffd}');
    (c, e)
}

/// The name that runs from `p` (whose first byte the caller has taken), over ASCII's `\w`, `$`
/// where `DOLLAR` says so, and characters past ASCII that `wide` takes.
#[inline(always)]
pub fn ident_end<const DOLLAR_IN: bool>(b: &[u8], p: usize, wide: fn(char) -> bool) -> Ident {
    let mut i = p;
    let mut bits = 0;
    let take = if DOLLAR_IN { WORD | DOLLAR } else { WORD };
    while let Some(&c) = b.get(i) {
        let k = CLASS[usize::from(c)];
        if k & take != 0 {
            bits |= k;
            i += 1;
        } else if k & HIGH != 0 {
            return ident_wide::<DOLLAR_IN>(b, i, bits, wide);
        } else {
            break;
        }
    }
    Ident { end: i, bits }
}

/// The run from `p` of bytes whose classes meet `take`, with the classes they had: a name in a
/// language whose every byte past ASCII is a letter (Rust's `\w`: `WORD | HIGH`), read a byte at
/// a time on the table (X-054: no wider read gained).
#[inline(always)]
pub fn class_run(b: &[u8], p: usize, take: u8) -> Ident {
    let mut i = p;
    let mut bits = 0;
    while let Some(&c) = b.get(i) {
        let k = CLASS[usize::from(c)];
        if k & take == 0 {
            break;
        }
        bits |= k;
        i += 1;
    }
    Ident { end: i, bits }
}

#[cold]
#[inline(never)]
fn ident_wide<const DOLLAR_IN: bool>(
    b: &[u8],
    mut i: usize,
    mut bits: u8,
    wide: fn(char) -> bool,
) -> Ident {
    let take = if DOLLAR_IN { WORD | DOLLAR } else { WORD };
    while let Some(&c) = b.get(i) {
        let k = CLASS[usize::from(c)];
        if k & take != 0 {
            bits |= k;
            i += 1;
        } else if k & HIGH != 0 {
            let (ch, e) = wide_at(b, i);
            if !wide(ch) {
                break;
            }
            bits |= HIGH;
            i = e;
        } else {
            break;
        }
    }
    Ident { end: i, bits }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_read_with_their_classes() {
        let r = ident_end::<true>(b"Foo$bar1 x", 0, char::is_alphanumeric);
        assert_eq!(r.end, 8);
        assert_eq!(
            r.bits & (UPPER | LOWER | DIGIT | DOLLAR),
            UPPER | LOWER | DIGIT | DOLLAR
        );
        let r = ident_end::<false>("naïve(".as_bytes(), 0, char::is_alphanumeric);
        assert_eq!((r.end, r.bits & HIGH), (6, HIGH));
        assert_eq!(ident_end::<false>(b"a$b", 0, char::is_alphanumeric).end, 1);
        let r = class_run("u8é_X+".as_bytes(), 0, WORD | HIGH);
        assert_eq!(
            (r.end, r.bits & (UPPER | LOWER | DIGIT | HIGH)),
            (6, UPPER | LOWER | DIGIT | HIGH)
        );
    }
}
