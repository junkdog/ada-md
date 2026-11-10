//! Here-documents, the part the scanners share (X-105): a label kept between lines and the test
//! of whether a line closes it. A primitive (X-017's shape), not a parser: the opener's syntax
//! and the body's text (bash's `$` and backticks, PHP's `{$…}`, Ruby's `#{}`, Terraform's `${}`)
//! are each host's own code. Counted delimiters (Lua's long brackets, raw strings) carry a count,
//! not a label, and aren't here.
//!
//! **The bit budget.** A [`Label`] is 28 bits: the word's length in 6 (0–62 exactly, 63 for 63
//! or more) and 22 bits of its FNV-1a hash. 28 bits are four 7-bit levels of a byte stack
//! (bash's, whose level bytes keep their top bit for "not a context"), or a quarter of a 64-bit
//! word less 4 bits. A line closes a label when its candidate has the label's length and hash:
//! the length is compared first, so a body line is hashed only when it is as long as the word
//! (the whole line was hashed before, X-105's floor probe: 13.9 → 2.7 ns a body line on bash's
//! census). Two words of one length collide at 2⁻²²; a long word's closer is hashed whole.
//!
//! **Several on a line.** Bodies follow their openers' order (`cat <<A <<B`: `A`'s lines, then
//! `B`'s). A host with a stack of its own keeps them there in that order (bash: the later
//! opener's levels go under the earlier's); a host without one keeps a [`Queue`] of
//! `N` labels, each beside 4 flag bits of the host's, 4 bytes an entry. A label past the `N`th is
//! dropped: its opener is still the host's opener, its body read as the host's text after the
//! bodies before it, and the bodies before it are unchanged.

/// The bits of a label's length; the largest length is "this or more".
const LEN_BITS: u32 = 6;
const LEN_MAX: u32 = (1 << LEN_BITS) - 1;
/// The bits of a label's hash.
const HASH_BITS: u32 = Label::BITS - LEN_BITS;

/// A here-document's label as a state keeps it: its length and a hash.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Label(u32);

/// What may stand on a closing line before the label.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lead {
    /// Nothing: the label starts the line (bash's `<<`).
    None,
    /// Oniguruma's `\s` on a line, any number: a blank, `\r`, a form feed, a vertical tab (bash's
    /// `<<-` as bat reads it; PHP 7.3's closer, Ruby's `<<~` and `<<-`, Terraform's `<<-`).
    Blanks,
}

/// What may follow the label on its closing line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tail {
    /// Blanks to the line's end, none or more ([`Close::end`] says where they start).
    Blanks,
    /// Anything that can't continue the label: not an ASCII word byte nor a byte past ASCII
    /// (PHP 7.3's `EOT;`, `EOT)`).
    #[allow(dead_code)] // PHP's closer (X-099)
    NonWord,
}

/// Where a closing line's label is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Close {
    pub start: usize,
    pub end: usize,
}

/// Oniguruma's `\s` on a line.
#[inline(always)]
fn is_blank(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | 0x0b | 0x0c)
}

/// A byte that continues a label for [`Tail::NonWord`].
#[inline(always)]
fn is_label_byte(c: u8) -> bool {
    c >= 0x80 || crate::prim::WORD[usize::from(c)]
}

/// FNV-1a.
#[inline(always)]
fn fnv(word: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &c in word {
        h = (h ^ u32::from(c)).wrapping_mul(0x0100_0193);
    }
    h
}

impl Label {
    /// The bits a label takes.
    pub const BITS: u32 = 28;

    /// The label of `word`.
    pub fn of(word: &[u8]) -> Label {
        let len = (word.len() as u32).min(LEN_MAX);
        Label(len | (fnv(word) & ((1 << HASH_BITS) - 1)) << LEN_BITS)
    }

    /// The label's [`Label::BITS`] bits.
    #[inline(always)]
    pub fn bits(self) -> u32 {
        self.0
    }

    /// A label from its bits (those past [`Label::BITS`] ignored).
    #[inline(always)]
    pub fn from_bits(bits: u32) -> Label {
        Label(bits & ((1 << Self::BITS) - 1))
    }

    /// The label in four 7-bit pieces, low first, for a byte stack's levels.
    #[inline(always)]
    pub fn septets(self) -> [u8; 4] {
        std::array::from_fn(|i| ((self.0 >> (7 * i)) & 0x7f) as u8)
    }

    /// The label from [`Label::septets`]' pieces (each byte's top bit ignored).
    #[inline(always)]
    pub fn from_septets(s: [u8; 4]) -> Label {
        Label(
            s.iter()
                .enumerate()
                .fold(0, |l, (i, &c)| l | u32::from(c & 0x7f) << (7 * i)),
        )
    }

    /// Where `b`, a line without its `\n`, has the label as its closer, what may stand before
    /// and after it as `lead` and `tail` allow.
    #[inline(always)]
    pub fn close(self, b: &[u8], lead: Lead, tail: Tail) -> Option<Close> {
        let n = b.len();
        let mut start = 0;
        if lead == Lead::Blanks {
            while start < n && is_blank(b[start]) {
                start += 1;
            }
        }
        let want = self.0 & LEN_MAX;
        let end = match tail {
            Tail::Blanks => {
                let mut end = n;
                while end > start && is_blank(b[end - 1]) {
                    end -= 1;
                }
                if ((end - start) as u32).min(LEN_MAX) != want {
                    return None;
                }
                end
            }
            Tail::NonWord if want < LEN_MAX => {
                let end = start + want as usize;
                if end > n || b.get(end).is_some_and(|&c| is_label_byte(c)) {
                    return None;
                }
                end
            }
            Tail::NonWord => {
                let mut end = start;
                while end < n && is_label_byte(b[end]) {
                    end += 1;
                }
                if end - start < LEN_MAX as usize {
                    return None;
                }
                end
            }
        };
        (start < end && fnv(&b[start..end]) & ((1 << HASH_BITS) - 1) == self.0 >> LEN_BITS)
            .then_some(Close { start, end })
    }
}

/// Up to `N` labels waiting for their bodies, in their openers' order, each with 4 bits of the
/// host's beside it: for a host without a stack of its own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Queue<const N: usize> {
    items: [u32; N],
    len: u8,
}

impl<const N: usize> Default for Queue<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Queue<N> {
    /// The bytes a queue's labels take in a state.
    pub const BYTES: usize = 4 * N;

    pub const fn new() -> Self {
        const { assert!(N > 0 && N < 16) };
        Queue {
            items: [0; N],
            len: 0,
        }
    }

    /// How many labels wait.
    #[inline(always)]
    pub fn len(&self) -> usize {
        usize::from(self.len)
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// `label` and its `flags` (the low 4 bits) queued after those waiting; `false` where `N`
    /// wait already, the label dropped.
    pub fn push(&mut self, label: Label, flags: u8) -> bool {
        if self.len() == N {
            return false;
        }
        self.items[self.len()] = label.0 | u32::from(flags & 0xf) << Label::BITS;
        self.len += 1;
        true
    }

    /// `label` and its `flags` queued before those waiting: a body that nests in the front's (an
    /// opener on a line of a body read as code, Ruby's RUBY bodies); `false` where `N` wait
    /// already, the label dropped.
    pub fn push_front(&mut self, label: Label, flags: u8) -> bool {
        if self.len() == N {
            return false;
        }
        let len = self.len();
        self.items.copy_within(0..len, 1);
        self.items[0] = label.0 | u32::from(flags & 0xf) << Label::BITS;
        self.len += 1;
        true
    }

    /// The label whose body is next, and its flags.
    #[inline(always)]
    pub fn front(&self) -> Option<(Label, u8)> {
        (self.len > 0).then(|| {
            let e = self.items[0];
            (Label::from_bits(e), (e >> Label::BITS) as u8)
        })
    }

    /// The front label's body done.
    pub fn pop(&mut self) {
        if self.len > 0 {
            self.items.copy_within(1.., 0);
            self.len -= 1;
            self.items[self.len()] = 0;
        }
    }

    /// The labels as [`Queue::BYTES`] bytes, those past the last zeros.
    pub fn save(&self, out: &mut [u8]) {
        for (i, e) in self.items.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&e.to_le_bytes());
        }
    }

    /// The queue of `len` labels (at most `N`) that [`Queue::save`] wrote to `bytes`.
    pub fn load(bytes: &[u8], len: usize) -> Self {
        let mut q = Self::new();
        q.len = len.min(N) as u8;
        for i in 0..q.len() {
            let mut w = [0; 4];
            w.copy_from_slice(&bytes[4 * i..4 * i + 4]);
            q.items[i] = u32::from_le_bytes(w);
        }
        q
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_closes_its_line() {
        let l = Label::of(b"EOF");
        let at = |b: &[u8], lead, tail| l.close(b, lead, tail).map(|c| (c.start, c.end));
        assert_eq!(at(b"EOF", Lead::None, Tail::Blanks), Some((0, 3)));
        assert_eq!(at(b"EOF \t", Lead::None, Tail::Blanks), Some((0, 3)));
        assert_eq!(at(b" EOF", Lead::None, Tail::Blanks), None);
        assert_eq!(at(b"\t\tEOF", Lead::Blanks, Tail::Blanks), Some((2, 5)));
        assert_eq!(at(b"EOFX", Lead::None, Tail::Blanks), None);
        assert_eq!(at(b"EOG", Lead::None, Tail::Blanks), None);
        assert_eq!(at(b"", Lead::None, Tail::Blanks), None);
        // PHP 7.3: the label, then anything that isn't a label's byte
        assert_eq!(at(b"    EOF;", Lead::Blanks, Tail::NonWord), Some((4, 7)));
        assert_eq!(at(b"EOF)", Lead::None, Tail::NonWord), Some((0, 3)));
        assert_eq!(at(b"EOF", Lead::None, Tail::NonWord), Some((0, 3)));
        assert_eq!(at(b"EOFS", Lead::None, Tail::NonWord), None);
        assert_eq!(at(b"EOF\xc3\xa9", Lead::None, Tail::NonWord), None);
        assert_eq!(at(b"EO", Lead::None, Tail::NonWord), None);
    }

    #[test]
    fn a_long_label_is_hashed_whole() {
        let long = [b'A'; 70];
        let l = Label::of(&long);
        assert_eq!(
            l.close(&long, Lead::None, Tail::Blanks).map(|c| c.end),
            Some(70)
        );
        assert!(l.close(&long[..69], Lead::None, Tail::Blanks).is_none());
        assert!(l.close(&long[..63], Lead::None, Tail::Blanks).is_none());
        let mut semi = long.to_vec();
        semi.push(b';');
        assert_eq!(
            l.close(&semi, Lead::None, Tail::NonWord).map(|c| c.end),
            Some(70)
        );
        assert!(l.close(&long[..64], Lead::None, Tail::NonWord).is_none());
        // 63 bytes is the saturated length too
        let l63 = Label::of(&long[..63]);
        assert!(l63.close(&long[..63], Lead::None, Tail::Blanks).is_some());
        assert!(l63.close(&long, Lead::None, Tail::Blanks).is_none());
    }

    #[test]
    fn a_label_survives_its_septets() {
        for w in [&b"EOF"[..], b"END_OF_TEXT", b"", &[b'x'; 100]] {
            let l = Label::of(w);
            assert_eq!(Label::from_septets(l.septets()), l);
            assert_eq!(Label::from_bits(l.bits()), l);
            assert!(l.septets().iter().all(|&s| s < 0x80));
        }
    }

    #[test]
    fn a_queue_keeps_its_openers_order_and_drops_past_its_cap() {
        let mut q = Queue::<2>::new();
        assert!(q.push(Label::of(b"A"), 1));
        assert!(q.push(Label::of(b"B"), 2));
        assert!(!q.push(Label::of(b"C"), 3));
        let mut bytes = [0; Queue::<2>::BYTES];
        q.save(&mut bytes);
        let mut q = Queue::<2>::load(&bytes, q.len());
        assert_eq!(q.front(), Some((Label::of(b"A"), 1)));
        q.pop();
        assert_eq!(q.front(), Some((Label::of(b"B"), 2)));
        q.pop();
        assert_eq!(q.front(), None);
        assert!(q.is_empty());
        q.pop();
        assert!(q.is_empty());
    }
}
