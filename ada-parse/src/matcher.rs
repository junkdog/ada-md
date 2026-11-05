//! What the scanners compiled from bat's grammars (TOML, XML, YAML, `.env`, Git, Dockerfile,
//! Java properties) match a line with: a regex's match and its groups ([`M`]), the line with
//! the `\n` syntect matches it with ([`Line`]), each grammar's scopes as small indices
//! ([`atoms!`]), and the character classes and searches their matchers share.

/// A scope, as an index into a grammar's `ATOMS`.
pub type Atom = u8;

/// A match: where it starts and ends, and where those of its groups that took part do, by
/// their number in the regex (up to [`GROUPS`] of them).
#[derive(Clone, Copy, Debug)]
pub struct M {
    pub s: u32,
    pub e: u32,
    n: u8,
    nums: [u8; GROUPS],
    spans: [(u32, u32); GROUPS],
}

/// The most groups a match keeps, after the whole match.
pub const GROUPS: usize = 8;

impl M {
    pub fn new(s: usize, e: usize) -> M {
        M {
            s: s as u32,
            e: e as u32,
            n: 0,
            nums: [0; GROUPS],
            spans: [(0, 0); GROUPS],
        }
    }

    /// Group `n` (from 1) spans `s..e`.
    pub fn g(mut self, n: usize, s: usize, e: usize) -> M {
        let i = self.nums[..usize::from(self.n)]
            .iter()
            .position(|&x| usize::from(x) == n)
            .unwrap_or_else(|| {
                self.n += 1;
                usize::from(self.n) - 1
            });
        self.nums[i] = n as u8;
        self.spans[i] = (s as u32, e as u32);
        self
    }

    /// The groups that took part, as `(number, start, end)`, in the order they were set.
    pub fn groups(&self) -> impl Iterator<Item = (u8, usize, usize)> + '_ {
        let n = usize::from(self.n);
        self.nums[..n]
            .iter()
            .zip(&self.spans[..n])
            .map(|(&g, &(s, e))| (g, s as usize, e as usize))
    }

    /// Where group `n` (from 1; 0 for the whole match) spans, if it took part.
    pub fn group(&self, n: u8) -> Option<(usize, usize)> {
        if n == 0 {
            return Some((self.s as usize, self.e as usize));
        }
        let i = self.nums[..usize::from(self.n)]
            .iter()
            .position(|&x| x == n)?;
        let (s, e) = self.spans[i];
        Some((s as usize, e as usize))
    }
}

/// A line as a regex sees it: its bytes, then a `\n` at [`Line::nl`]; positions run to
/// [`Line::end`], past the `\n`.
pub struct Line<'a> {
    pub b: &'a [u8],
}

impl Line<'_> {
    /// Where the `\n` is: the line's length.
    pub fn nl(&self) -> usize {
        self.b.len()
    }

    /// The end of the text matched, after the `\n`.
    pub fn end(&self) -> usize {
        self.b.len() + 1
    }

    /// The byte at `i`: the `\n` at [`Line::nl`], none at or after [`Line::end`].
    pub fn at(&self, i: usize) -> Option<u8> {
        match self.b.get(i) {
            Some(&c) => Some(c),
            None if i == self.b.len() => Some(b'\n'),
            None => None,
        }
    }

    /// Whether the byte at `i` is in `set`.
    pub fn is(&self, i: usize, set: impl Fn(u8) -> bool) -> bool {
        self.at(i).is_some_and(set)
    }

    /// Where a run of bytes in `set` from `i` ends.
    pub fn run(&self, mut i: usize, set: impl Fn(u8) -> bool) -> usize {
        while self.is(i, &set) {
            i += 1;
        }
        i
    }

    /// The length of the character at `i`, for `.` (a byte of a broken one counts alone);
    /// none at the `\n` or past it.
    pub fn char_len(&self, i: usize) -> Option<usize> {
        let &c = self.b.get(i)?;
        let n = match c {
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf7 => 4,
            _ => 1,
        };
        let whole = i + n <= self.b.len() && self.b[i + 1..i + n].iter().all(|&c| c & 0xc0 == 0x80);
        Some(if whole { n } else { 1 })
    }

    /// `$` in Oniguruma's multi-line sense: before the `\n`, or at the end.
    pub fn eol(&self, i: usize) -> bool {
        i == self.nl() || i == self.end()
    }

    /// The leftmost `lit` at or after `from`.
    pub fn lit(&self, from: usize, lit: &[u8]) -> Option<M> {
        let hay = self.b.get(from..)?;
        let i = hay.windows(lit.len()).position(|w| w == lit)?;
        Some(M::new(from + i, from + i + lit.len()))
    }
}

/// A grammar's scopes, each a constant in a module `s` naming its index in `ATOMS`.
macro_rules! atoms {
    ($($name:ident = $scope:literal,)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms, dead_code)]
        #[repr(u8)]
        enum Atoms {
            $($name,)*
        }

        /// The scopes, as indices into [`ATOMS`]; a grammar's scanner may color some of them
        /// no way.
        #[allow(dead_code)]
        pub(crate) mod s {
            $(pub const $name: crate::matcher::Atom = super::Atoms::$name as u8;)*
        }

        pub(crate) const ATOMS: &[&str] = &[$($scope,)*];
    };
}

pub(crate) use atoms;

/// The first `c` of the line at or after `from`.
pub fn first(l: &Line, from: usize, c: u8) -> Option<usize> {
    let hay = l.b.get(from..)?;
    hay.iter().position(|&x| x == c).map(|i| from + i)
}

/// `\w` as far as ASCII goes; any non-ASCII byte counts as one, as Oniguruma's Unicode `\w`
/// takes letters.
pub fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// `[ \t]`.
pub fn is_blank(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

/// `\s`.
pub fn is_space(b: u8) -> bool {
    crate::is_space(b)
}

/// The levels a [`crate::State`] keeps between lines.
pub const KEPT: usize = 14;
