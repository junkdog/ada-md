//! Which strings are regexes (ADR-40's phase 4, as the compiled scanner had it): the first
//! argument of `new Regex(` (`new System.Text.RegularExpressions.Regex(`), `[GeneratedRegex(`
//! and `[RegularExpression(`, looked back from the string; the second of `Regex.IsMatch(`,
//! `Regex.Replace(` and the like ([`REGEX_METHODS`]), found once a line ([`RegexCalls`]). A
//! regex's text is on [`crate::regex`]'s .NET dialect, the string's end and escapes the host's.

pub(crate) struct RegexCalls {
    commas: [u32; 8],
    len: u8,
}

/// The static methods of `Regex` that take the regex second, after the input.
const REGEX_METHODS: [&[u8]; 8] = [
    b"IsMatch",
    b"Match",
    b"Matches",
    b"Replace",
    b"Split",
    b"Count",
    b"EnumerateMatches",
    b"EnumerateSplits",
];

/// Where a string's escape at `p` (a `\\` not before another) ends: `{{escaped_char}}`, or the
/// `\\` alone.
pub(super) fn escape_end_rx(b: &[u8], p: usize) -> usize {
    let hex = |q: usize| b.get(q).is_some_and(u8::is_ascii_hexdigit);
    match b.get(p + 1) {
        Some(b'u') if (p + 2..p + 6).all(hex) => p + 6,
        Some(b'U') if (p + 2..p + 10).all(hex) => p + 10,
        Some(b'x') if hex(p + 2) => (p + 3..p + 6).find(|&q| !hex(q)).unwrap_or(p + 6),
        Some(b'0'..=b'9') => (p + 2..p + 4)
            .find(|&q| !b.get(q).is_some_and(u8::is_ascii_digit))
            .unwrap_or(p + 4),
        Some(b'a' | b'b' | b'f' | b'n' | b'r' | b't' | b'v' | b'"' | b'\'') => p + 2,
        _ => p + 1,
    }
}

/// Whether `c` can be in a name.
fn word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

/// Where the blanks before `p` start.
pub(super) fn blank_back(b: &[u8], p: usize) -> usize {
    let mut q = p;
    while q > 0 && matches!(b[q - 1], b' ' | b'\t') {
        q -= 1;
    }
    q
}

/// Whether `b[..w]` ends with the word `name`.
fn ends_word(b: &[u8], w: usize, name: &[u8]) -> bool {
    b[..w].ends_with(name) && (w == name.len() || !word_byte(b[w - name.len() - 1]))
}

impl RegexCalls {
    /// The line `b`'s.
    #[cold]
    pub(super) fn find(b: &[u8]) -> Self {
        let mut calls = RegexCalls {
            commas: [0; 8],
            len: 0,
        };
        let mut i = 0;
        while usize::from(calls.len) < calls.commas.len()
            && let Some(at) = b[i..].iter().position(|&c| c == b'R')
        {
            let s = i + at;
            i = s + 1;
            if !b[s..].starts_with(b"Regex.") {
                continue;
            }
            i = s + 6;
            if s > 0 && word_byte(b[s - 1]) {
                continue;
            }
            let mut e = i;
            while e < b.len() && word_byte(b[e]) {
                e += 1;
            }
            if !REGEX_METHODS.contains(&&b[i..e]) {
                continue;
            }
            let mut o = e;
            while o < b.len() && matches!(b[o], b' ' | b'\t') {
                o += 1;
            }
            if b.get(o) == Some(&b'(')
                && let Some(c) = first_argument(b, o + 1)
            {
                calls.commas[usize::from(calls.len)] = c as u32;
                calls.len += 1;
            }
        }
        calls
    }

    pub(super) fn has_comma(&self, c: usize) -> bool {
        self.commas[..usize::from(self.len)]
            .iter()
            .any(|&x| x as usize == c)
    }
}

/// The comma after a call's first argument, which starts at `s`: `None` if the call has one
/// argument, or its first runs past the line.
fn first_argument(b: &[u8], s: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = s;
    while i < b.len() {
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.checked_sub(1)?,
            b',' if depth == 0 => return Some(i),
            b'/' if b.get(i + 1) == Some(&b'/') => return None,
            b'"' => {
                // a verbatim string's `""`, any other's `\"`
                let verbatim = i > 0 && (b[i - 1] == b'@' || (i > 1 && b[i - 2] == b'@'));
                i += 1;
                loop {
                    match b.get(i)? {
                        b'"' if verbatim && b.get(i + 1) == Some(&b'"') => i += 1,
                        b'"' => break,
                        b'\\' if !verbatim => i += 1,
                        _ => {}
                    }
                    i += 1;
                }
            }
            b'\'' => {
                i += 1;
                loop {
                    match b.get(i)? {
                        b'\'' => break,
                        b'\\' => i += 1,
                        _ => {}
                    }
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Whether the string after `b[..q]`, its `(`, is a regex: the first argument of `new Regex(`
/// (`new System.Text.RegularExpressions.Regex(`), `[GeneratedRegex(` or
/// `[RegularExpression(`, looked back on its line. The second argument of `Regex.IsMatch(`,
/// `Regex.Replace(` and the like ([`REGEX_METHODS`]) is found by the line's [`RegexCalls`].
pub(super) fn regex_first_argument(b: &[u8], q: usize) -> bool {
    let w = blank_back(b, q - 1);
    // each name below ends with `x`, `n` or `e`: most calls are turned away by that byte
    if w == 0 || !matches!(b[w - 1], b'x' | b'n' | b'e') {
        return false;
    }
    if b[..w].ends_with(b"Regex") {
        let mut s = w - 5;
        if s > 0 && word_byte(b[s - 1]) {
            return ends_word(b, w, b"GeneratedRegex");
        }
        // the namespace before it, then `new`
        while s > 0 && (b[s - 1] == b'.' || word_byte(b[s - 1])) {
            s -= 1;
        }
        let t = blank_back(b, s);
        return t < s && ends_word(b, t, b"new");
    }
    ends_word(b, w, b"RegularExpression")
        || ends_word(b, w, b"GeneratedRegexAttribute")
        || ends_word(b, w, b"RegularExpressionAttribute")
}
