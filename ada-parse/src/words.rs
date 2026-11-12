//! One word table type for every scanner (X-017): a list's words hashed at compile time, looked up
//! at a fixed cost. A word's id is its index in the list plus one (0 for none), so what a word
//! means stays beside the list, in an array indexed by the id, its element 0 the answer for none.
//!
//! - A first-byte and length filter ([`Words::get`]'s first test) turns away most names before
//!   the hash: a bit a length for each first byte (both cases' rows set in a `FOLD` table).
//! - The hash reads the first and last four bytes (or a shorter word's bytes) and the length, and
//!   multiplies once: no loop over the word, where FNV-1a's multiply a byte was serial.
//! - A slot holds the word's length, id and place in one blob of the words' text (Lisp's compact
//!   form, the length in the slot too), so a probe of another length loads nothing more; a hit
//!   is checked by [`crate::same`]: two loads a side up to 16 bytes, no `memcmp`.
//! - What a word is comes from the caller's array by id ([`Words::value`]), a miss answered
//!   before the id is read.
//! - `FOLD` tables ignore ASCII case: the hash and the compare fold eight bytes at a time; the
//!   blob keeps the words as listed.

/// A list's words by their hash, open addressed. `SLOTS` is a power of two, at least twice the
/// words; `N` is the list's length (under 2,048) and `TEXT` its words' bytes ([`text_len`], under
/// 32 KB). A word is under 64 bytes. A word listed twice (ASCII case aside, in a `FOLD` table)
/// keeps its first id.
pub struct Words<const SLOTS: usize, const N: usize, const TEXT: usize, const FOLD: bool = false> {
    /// A word: its length in the low 6 bits, its id in the next 11, its offset in
    /// [`Words::text`] in the top 15 ([`len`], [`id`], [`at`]); 0 for none. A miss on the length
    /// needs no other load, nor a shift.
    slots: [u32; SLOTS],
    text: [u8; TEXT],
    /// Which lengths (a bit each, modulo 32) the words starting with a byte have (in a `FOLD`
    /// table, with either case of an ASCII letter).
    first_len: [u32; 256],
}

/// Where a slot's id and offset start.
const ID: u32 = 6;
const AT: u32 = 17;

/// A slot's word's length.
#[inline(always)]
const fn len(s: u32) -> usize {
    (s & ((1 << ID) - 1)) as usize
}

/// A slot's word's id.
#[inline(always)]
const fn id(s: u32) -> u16 {
    ((s >> ID) & ((1 << (AT - ID)) - 1)) as u16
}

/// A slot's word's offset in its table's text.
#[inline(always)]
const fn at(s: u32) -> usize {
    (s >> AT) as usize
}

/// How many bytes the words of `list` hold: a [`Words`] table's `TEXT`.
pub const fn text_len(list: &[&[u8]]) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < list.len() {
        n += list[i].len();
        i += 1;
    }
    n
}

impl<const SLOTS: usize, const N: usize, const TEXT: usize, const FOLD: bool>
    Words<SLOTS, N, TEXT, FOLD>
{
    pub const fn new(list: &[&[u8]]) -> Self {
        assert!(SLOTS.is_power_of_two() && N * 2 <= SLOTS && N < 1 << (AT - ID));
        assert!(list.len() == N && text_len(list) == TEXT && TEXT < 1 << (32 - AT));
        let mut slots = [0u32; SLOTS];
        let mut text = [0u8; TEXT];
        let mut first_len = [0u32; 256];
        let mut o = 0;
        let mut i = 0;
        while i < N {
            let w = list[i];
            assert!(!w.is_empty() && w.len() < 1 << ID);
            let mut k = 0;
            while k < w.len() {
                text[o + k] = w[k];
                k += 1;
            }
            first_len[w[0] as usize] |= len_bit(w.len());
            if FOLD {
                first_len[w[0].to_ascii_lowercase() as usize] |= len_bit(w.len());
                first_len[w[0].to_ascii_uppercase() as usize] |= len_bit(w.len());
            }
            let mut h = hash::<SLOTS, FOLD>(w);
            loop {
                let s = slots[h];
                if s == 0 {
                    slots[h] = (o as u32) << AT | (i as u32 + 1) << ID | w.len() as u32;
                    break;
                }
                if const_same::<FOLD>(list[id(s) as usize - 1], w) {
                    break;
                }
                h = (h + 1) & (SLOTS - 1);
            }
            o += w.len();
            i += 1;
        }
        Words {
            slots,
            text,
            first_len,
        }
    }

    /// Word `w`'s slot, 0 for none.
    #[inline(always)]
    fn slot(&self, w: &[u8]) -> u32 {
        let n = w.len();
        match w.first() {
            Some(&c) if self.first_len[usize::from(c)] & len_bit(n) != 0 => {}
            _ => return 0,
        }
        let mut h = hash::<SLOTS, FOLD>(w);
        loop {
            let s = self.slots[h];
            if s == 0 {
                return 0;
            }
            if len(s) == n {
                // SAFETY: `new` wrote each slot's word, `len(s)` bytes, at `at(s)` in `text`
                let t = unsafe { self.text.get_unchecked(at(s)..at(s) + n) };
                if if FOLD {
                    fold_same(t, w)
                } else {
                    crate::same(t, w)
                } {
                    return s;
                }
            }
            h = (h + 1) & (SLOTS - 1);
        }
    }

    /// Word `w`'s id, 0 for none.
    #[inline(always)]
    pub fn get(&self, w: &[u8]) -> u16 {
        id(self.slot(w))
    }

    /// What word `w` is, from what each word is by id (`by_id`, element 0 the answer for none):
    /// without a bounds check, since no id passes `N`.
    #[inline(always)]
    pub fn value<T: Copy>(&self, w: &[u8], by_id: &[T]) -> T {
        // a static's length: the test folds away
        assert!(by_id.len() > N);
        let s = self.slot(w);
        // most words miss before the hash: their answer a constant, the slot's id not read
        if s == 0 {
            return by_id[0];
        }
        // SAFETY: an id is a list's index plus one, so at most `N`
        unsafe { *by_id.get_unchecked(usize::from(id(s))) }
    }

    /// [`Words::get`] at compile time, for a table built from another.
    pub const fn find(&self, w: &[u8]) -> u16 {
        if w.is_empty() {
            return 0;
        }
        let mut h = hash::<SLOTS, FOLD>(w);
        loop {
            let s = self.slots[h];
            if s == 0 {
                return 0;
            }
            if len(s) == w.len() {
                let (_, t) = self.text.split_at(at(s));
                if const_same::<FOLD>(t.split_at(w.len()).0, w) {
                    return id(s);
                }
            }
            h = (h + 1) & (SLOTS - 1);
        }
    }
}

/// A length's bit in `first_len`, modulo 32 (a shift's own count on x86): a longer word shares a
/// shorter one's bit, which only lets it on to the hash.
#[inline(always)]
const fn len_bit(n: usize) -> u32 {
    1 << (n & 31)
}

/// A hash of fixed cost: the first and last four bytes (or the bytes of a shorter word) and the
/// length, ASCII case folded in a `FOLD` table, multiplied, the top bits a slot.
#[inline(always)]
const fn hash<const SLOTS: usize, const FOLD: bool>(w: &[u8]) -> usize {
    let n = w.len();
    let x = if n >= 4 {
        let a = u32::from_le_bytes([w[0], w[1], w[2], w[3]]) as u64;
        let z = u32::from_le_bytes([w[n - 4], w[n - 3], w[n - 2], w[n - 1]]) as u64;
        a | z << 32
    } else if n > 0 {
        short(w)
    } else {
        0
    };
    let x = if FOLD { lower(x) } else { x };
    ((x ^ n as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) >> (64 - SLOTS.trailing_zeros())) as usize
}

/// A word of one to three bytes in a `u64` without a loop: its first, middle and last bytes
/// (each of a shorter word's bytes, the length telling them apart).
#[inline(always)]
pub(crate) const fn short(w: &[u8]) -> u64 {
    let n = w.len();
    w[0] as u64 | (w[n >> 1] as u64) << 8 | (w[n - 1] as u64) << 16
}

const ONES: u64 = 0x0101_0101_0101_0101;
const HIGH: u64 = 0x8080_8080_8080_8080;

/// Eight bytes with their ASCII capitals lowercased (each byte apart: no carry crosses one).
#[inline(always)]
pub(crate) const fn lower(x: u64) -> u64 {
    let low = x & !HIGH;
    // a byte's top bit: its low seven bits at least `A`, and past `Z`
    let ge_a = low + ONES * (0x80 - b'A' as u64);
    let gt_z = low + ONES * (0x80 - b'Z' as u64 - 1);
    x | ((ge_a & !gt_z & !x & HIGH) >> 2)
}

/// Whether `a` and `b`, of one length, are equal with ASCII case ignored: up to 16 bytes, two
/// loads a side that overlap, folded eight bytes at a time.
#[inline(always)]
fn fold_same(a: &[u8], b: &[u8]) -> bool {
    fn at<const K: usize>(s: &[u8], i: usize) -> [u8; K] {
        s[i..i + K].try_into().unwrap()
    }
    fn two(s: &[u8], n: usize) -> u64 {
        u64::from(u32::from_le_bytes(at(s, 0))) | u64::from(u32::from_le_bytes(at(s, n - 4))) << 32
    }
    let n = a.len();
    debug_assert_eq!(n, b.len());
    match n {
        0 => true,
        1..4 => lower(short(a)) == lower(short(b)),
        4..8 => lower(two(a, n)) == lower(two(b, n)),
        8..=16 => {
            lower(u64::from_le_bytes(at(a, 0))) == lower(u64::from_le_bytes(at(b, 0)))
                && lower(u64::from_le_bytes(at(a, n - 8)))
                    == lower(u64::from_le_bytes(at(b, n - 8)))
        }
        _ => a.eq_ignore_ascii_case(b),
    }
}

/// [`crate::same`] or [`fold_same`] at compile time.
const fn const_same<const FOLD: bool>(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        let (x, y) = if FOLD {
            (a[i].to_ascii_lowercase(), b[i].to_ascii_lowercase())
        } else {
            (a[i], b[i])
        };
        if x != y {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: [&[u8]; 6] = [
        b"if",
        b"else",
        b"while",
        b"instanceof",
        b"if",
        b"a_rather_long_word_of_forty_bytes_or_so",
    ];
    static W: Words<16, 6, { text_len(&LIST) }, false> = Words::new(&LIST);
    static F: Words<16, 6, { text_len(&LIST) }, true> = Words::new(&LIST);

    #[test]
    fn the_table_finds_its_words() {
        assert_eq!(W.get(b"if"), 1);
        assert_eq!(W.get(b"else"), 2);
        assert_eq!(W.get(b"while"), 3);
        assert_eq!(W.get(b"instanceof"), 4);
        assert_eq!(W.get(b"a_rather_long_word_of_forty_bytes_or_so"), 6);
        for w in [
            &b"i"[..],
            b"iff",
            b"els",
            b"While",
            b"",
            b"\xc3\xa9",
            b"\xe9lse",
            b"a_rather_long_word_of_forty_bytes_or_sO",
        ] {
            assert_eq!(W.get(w), 0, "{}", String::from_utf8_lossy(w));
        }
    }

    #[test]
    fn a_folded_table_ignores_ascii_case() {
        assert_eq!(F.get(b"IF"), 1);
        assert_eq!(F.get(b"While"), 3);
        assert_eq!(F.get(b"InstanceOf"), 4);
        assert_eq!(F.get(b"A_RATHER_long_word_of_forty_bytes_or_so"), 6);
        for w in [&b"wHilE_"[..], b"\xc3\xa9", b"el@e", b"iF\0"] {
            assert_eq!(F.get(w), 0, "{}", String::from_utf8_lossy(w));
        }
    }

    /// `lower` against `to_ascii_lowercase` on every byte, in every place of a word.
    #[test]
    fn lower_folds_ascii_capitals_alone() {
        for c in 0..=255u8 {
            for k in 0..8 {
                let x = u64::from(c) << (8 * k) | 0x4040_4040_4040_4040 & !(0xff << (8 * k));
                let want = u64::from_le_bytes(x.to_le_bytes().map(|b| b.to_ascii_lowercase()));
                assert_eq!(lower(x), want, "{c:#x} at {k}");
            }
        }
        for (a, b) in [
            (&b"Ab"[..], &b"aB"[..]),
            (b"@[`{", b"`{@["),
            (b"_\x7f", b"\x7f_"),
        ] {
            assert_eq!(fold_same(a, b), a.eq_ignore_ascii_case(b));
        }
    }
}
