//! The regex pieces of bat's C# grammar that its patterns share: character classes, a name
//! (`{{name}}`, with its `@`-alternatives in their order), the generic and type-suffix pieces,
//! and the lookaheads built of them, each matched with its regex's backtracking order
//! (continuation-passing: a piece calls on with each end it can take, best first, until the
//! rest matches).
//!
//! Positions are bytes of the line; the line's `\n` is at [`Scan::n`], a space every class
//! but `.` and `\S` takes, and the line ends past it. Past ASCII, `\p{L}` is
//! `char::is_alphabetic`, `\w` `char::is_alphanumeric`, `\s` ASCII's.

use super::Scan;

/// `{{reserved}}`, in the grammar's order: `@in` comes before `@interface`.
pub(super) const RESERVED: [&[u8]; 61] = [
    b"abstract",
    b"as",
    b"base",
    b"break",
    b"case",
    b"catch",
    b"checked",
    b"class",
    b"const",
    b"continue",
    b"default",
    b"delegate",
    b"do",
    b"else",
    b"enum",
    b"event",
    b"explicit",
    b"extern",
    b"finally",
    b"fixed",
    b"for",
    b"foreach",
    b"goto",
    b"if",
    b"implicit",
    b"in",
    b"interface",
    b"internal",
    b"is",
    b"lock",
    b"nameof",
    b"namespace",
    b"new",
    b"null",
    b"operator",
    b"out",
    b"override",
    b"params",
    b"private",
    b"protected",
    b"public",
    b"readonly",
    b"ref",
    b"return",
    b"sealed",
    b"sizeof",
    b"stackalloc",
    b"static",
    b"string",
    b"struct",
    b"switch",
    b"this",
    b"throw",
    b"try",
    b"typeof",
    b"unchecked",
    b"unsafe",
    b"using",
    b"virtual",
    b"volatile",
    b"while",
];

/// `{{base_type}}`'s words, in the grammar's order.
pub(super) const BASE: [&[u8]; 16] = [
    b"bool", b"byte", b"sbyte", b"char", b"decimal", b"double", b"float", b"int", b"uint", b"long",
    b"ulong", b"short", b"ushort", b"object", b"string", b"void",
];

/// Where Unicode's decimal digits past ASCII start, ten in a row each (`\\d` is Unicode's in
/// syntect's regexes): Unicode 16's `Nd`.
const ND: [u32; 75] = [
    0x660, 0x6f0, 0x7c0, 0x966, 0x9e6, 0xa66, 0xae6, 0xb66, 0xbe6, 0xc66, 0xce6, 0xd66, 0xde6,
    0xe50, 0xed0, 0xf20, 0x1040, 0x1090, 0x17e0, 0x1810, 0x1946, 0x19d0, 0x1a80, 0x1a90, 0x1b50,
    0x1bb0, 0x1c40, 0x1c50, 0xa620, 0xa8d0, 0xa900, 0xa9d0, 0xa9f0, 0xaa50, 0xabf0, 0xff10,
    0x104a0, 0x10d30, 0x10d40, 0x11066, 0x110f0, 0x11136, 0x111d0, 0x112f0, 0x11450, 0x114d0,
    0x11650, 0x116c0, 0x116d0, 0x116da, 0x11730, 0x118e0, 0x11950, 0x11bf0, 0x11c50, 0x11d50,
    0x11da0, 0x11f50, 0x16130, 0x16a60, 0x16ac0, 0x16b50, 0x16d70, 0x1ccf0, 0x1d7ce, 0x1d7d8,
    0x1d7e2, 0x1d7ec, 0x1d7f6, 0x1e140, 0x1e2f0, 0x1e4f0, 0x1e5f1, 0x1e950, 0x1fbf0,
];

/// Whether `w` is a word of `{{reserved}}`.
pub(super) fn is_reserved(w: &[u8]) -> bool {
    matches!(
        w,
        b"abstract"
            | b"as"
            | b"base"
            | b"break"
            | b"case"
            | b"catch"
            | b"checked"
            | b"class"
            | b"const"
            | b"continue"
            | b"default"
            | b"delegate"
            | b"do"
            | b"else"
            | b"enum"
            | b"event"
            | b"explicit"
            | b"extern"
            | b"finally"
            | b"fixed"
            | b"for"
            | b"foreach"
            | b"goto"
            | b"if"
            | b"implicit"
            | b"in"
            | b"interface"
            | b"internal"
            | b"is"
            | b"lock"
            | b"nameof"
            | b"namespace"
            | b"new"
            | b"null"
            | b"operator"
            | b"out"
            | b"override"
            | b"params"
            | b"private"
            | b"protected"
            | b"public"
            | b"readonly"
            | b"ref"
            | b"return"
            | b"sealed"
            | b"sizeof"
            | b"stackalloc"
            | b"static"
            | b"string"
            | b"struct"
            | b"switch"
            | b"this"
            | b"throw"
            | b"try"
            | b"typeof"
            | b"unchecked"
            | b"unsafe"
            | b"using"
            | b"virtual"
            | b"volatile"
            | b"while"
    )
}

/// The bytes of the UTF-8 character that starts with `c`.
pub(super) fn char_len(c: u8) -> usize {
    match c {
        0xf0.. => 4,
        0xe0.. => 3,
        0xc0.. => 2,
        _ => 1,
    }
}

fn is_hex(c: u8) -> bool {
    c.is_ascii_hexdigit()
}

fn ascii_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// The bytes of the `\\s` character (Unicode's White_Space, as syntect's regexes take it) at
/// the start of `b`.
#[inline]
pub(super) fn space_len(b: &[u8]) -> Option<usize> {
    match *b {
        [c, ..] if c < 0x80 => crate::is_space(c).then_some(1),
        [0xc2, 0x85 | 0xa0, ..] => Some(2),
        [0xe1, 0x9a, 0x80, ..] => Some(3),
        [0xe2, 0x80, 0x80..=0x8a | 0xa8 | 0xa9 | 0xaf, ..] => Some(3),
        [0xe2, 0x81, 0x9f, ..] | [0xe3, 0x80, 0x80, ..] => Some(3),
        _ => None,
    }
}

impl Scan<'_, '_> {
    /// The byte at `i`: the line's, its `\n` at the end, 0 past it.
    #[inline]
    pub(super) fn ch(&self, i: usize) -> u8 {
        if i < self.n {
            self.b[i]
        } else if i == self.n {
            b'\n'
        } else {
            0
        }
    }

    /// Whether `i` is on the line or its `\n`.
    #[inline]
    pub(super) fn on(&self, i: usize) -> bool {
        i <= self.n
    }

    /// The character starting at byte `i` (not past ASCII's start bytes).
    fn char_at(&self, i: usize) -> Option<char> {
        let b = self.b;
        let e = (i + char_len(b[i])).min(self.n);
        std::str::from_utf8(&b[i..e]).ok()?.chars().next()
    }

    /// The character ending at byte `i`.
    fn char_before(&self, i: usize) -> Option<char> {
        let b = self.b;
        let s = (i.saturating_sub(4)..i)
            .rev()
            .find(|&s| b[s] & 0xc0 != 0x80)
            .unwrap_or(i - 1);
        std::str::from_utf8(&b[s..i]).ok()?.chars().next_back()
    }

    /// `\w` at the character starting at `i`.
    #[inline]
    pub(super) fn word_at(&self, i: usize) -> bool {
        if i >= self.n {
            return false;
        }
        let c = self.b[i];
        if c < 0x80 {
            ascii_word(c)
        } else {
            self.char_at(i).is_some_and(char::is_alphanumeric)
        }
    }

    /// `\w` at the character ending at `i`.
    #[inline]
    pub(super) fn word_before(&self, i: usize) -> bool {
        if i == 0 || i > self.n {
            return false;
        }
        let c = self.b[i - 1];
        if c < 0x80 {
            ascii_word(c)
        } else {
            self.char_before(i).is_some_and(char::is_alphanumeric)
        }
    }

    /// `\b` at `i`.
    #[inline]
    pub(super) fn bnd(&self, i: usize) -> bool {
        self.word_before(i) != self.word_at(i)
    }

    /// Whether the line has `s` at `i`.
    #[inline]
    pub(super) fn lit(&self, i: usize, s: &[u8]) -> bool {
        self.b.get(i..).is_some_and(|r| r.starts_with(s))
    }

    /// Where the run of `\\w` at `i` ends (`i` where there is none), once a place.
    #[inline]
    pub(super) fn word_end(&self, i: usize) -> usize {
        let (at, end) = self.wcache.get();
        if at == i {
            return end;
        }
        let b = self.b;
        let mut e = i;
        while e < self.n {
            let c = b[e];
            if c < 0x80 {
                if !ascii_word(c) {
                    break;
                }
                e += 1;
            } else if self.word_at(e) {
                e += char_len(c).min(self.n - e);
            } else {
                break;
            }
        }
        self.wcache.set((i, e));
        e
    }

    /// The whole run of `\\w` at `i`.
    #[inline]
    pub(super) fn word(&self, i: usize) -> &[u8] {
        if i >= self.n {
            return &[];
        }
        let e = self.word_end(i);
        &self.b[i..e]
    }

    /// `\bw\b` at `i`.
    pub(super) fn kw(&self, i: usize, w: &[u8]) -> bool {
        self.word(i) == w && !self.word_before(i)
    }

    /// `w\b` at `i` (no `\b` before), `w` all word characters.
    #[inline]
    pub(super) fn kw_end(&self, i: usize, w: &[u8]) -> bool {
        self.word(i) == w
    }

    /// The first of `ws` that is a whole word at `i` (`\b(?:…)\b`): where it ends.
    pub(super) fn kw_any(&self, i: usize, ws: &[&[u8]]) -> Option<usize> {
        if self.word_before(i) {
            return None;
        }
        let w = self.word(i);
        if w.is_empty() {
            return None;
        }
        ws.contains(&w).then_some(i + w.len())
    }

    /// Past `\s*` at `i`, the `\n` included.
    #[inline]
    pub(super) fn sp(&self, i: usize) -> usize {
        let i = self.sp_line(i);
        if i == self.n { i + 1 } else { i }
    }

    /// Past `\s*` at `i` on the line (its `\n` not taken).
    #[inline]
    pub(super) fn sp_line(&self, mut i: usize) -> usize {
        while i < self.n {
            let c = self.b[i];
            if c < 0x80 {
                if !crate::is_space(c) {
                    break;
                }
                i += 1;
            } else {
                match space_len(&self.b[i..]) {
                    Some(l) => i += l,
                    None => break,
                }
            }
        }
        i
    }

    /// Past `\s+` at `i`, if a space is there.
    pub(super) fn sp1(&self, i: usize) -> Option<usize> {
        self.space(i).then(|| self.sp(i))
    }

    /// `\S` at `i`.
    #[inline]
    pub(super) fn nonspace(&self, i: usize) -> bool {
        i < self.n && space_len(&self.b[i..]).is_none()
    }

    /// `\s` at `i` (the `\n` too).
    pub(super) fn space(&self, i: usize) -> bool {
        i == self.n || (i < self.n && space_len(&self.b[i..]).is_some())
    }

    /// `$` at `i`.
    pub(super) fn eol(&self, i: usize) -> bool {
        i >= self.n
    }

    /// `\\u\h{4}` or `\\U\h{8}` at `i`: where it ends.
    fn unicode_escape(&self, i: usize) -> Option<usize> {
        let b = self.b;
        if b.get(i) != Some(&b'\\') {
            return None;
        }
        let n = match b.get(i + 1) {
            Some(b'u') => 4,
            Some(b'U') => 8,
            _ => return None,
        };
        let hex = b.get(i + 2..i + 2 + n)?;
        hex.iter().all(|&c| is_hex(c)).then_some(i + 2 + n)
    }

    /// `{{start_char}}` at `i`: where it ends.
    fn start_char(&self, i: usize) -> Option<usize> {
        let c = *self.b.get(i)?;
        if c.is_ascii_alphabetic() || c == b'_' {
            Some(i + 1)
        } else if c == b'\\' {
            self.unicode_escape(i)
        } else if c >= 0x80 {
            self.char_at(i)
                .filter(|ch| ch.is_alphabetic())
                .map(|ch| i + ch.len_utf8())
        } else {
            None
        }
    }

    /// `{{other_char}}` at `i`: where it ends.
    fn other_char(&self, i: usize) -> Option<usize> {
        let c = *self.b.get(i)?;
        if ascii_word(c) {
            Some(i + 1)
        } else if c == b'\\' {
            self.unicode_escape(i)
        } else if c >= 0x80 {
            self.char_at(i)
                .filter(|ch| ch.is_alphabetic())
                .map(|ch| i + ch.len_utf8())
        } else {
            None
        }
    }

    /// `{{name_normal}}` at `i`, each end it can take (best first) given to `f` until it says
    /// yes.
    fn name_normal(&self, i: usize, f: &mut dyn FnMut(usize) -> bool) -> bool {
        // the common case: ASCII to a byte that ends it
        if let Some(e) = self.ascii_name(i) {
            return f(e);
        }
        let Some(first) = self.start_char(i) else {
            return false;
        };
        // past ASCII or an escape: every element's end, then those `\b` holds at, longest first
        let mut ends = vec![first];
        let mut q = first;
        while let Some(r) = self.other_char(q) {
            q = r;
            ends.push(q);
        }
        for &end in ends.iter().rev() {
            if self.bnd(end) && f(end) {
                return true;
            }
        }
        false
    }

    /// A name of ASCII at `i` that a byte ends (no escape, nothing past ASCII): where it ends,
    /// once a place.
    #[inline]
    pub(super) fn ascii_name(&self, i: usize) -> Option<usize> {
        let (at, end) = self.ncache.get();
        if at == i {
            return end;
        }
        let b = self.b;
        let end = match b.get(i) {
            Some(&c) if c.is_ascii_alphabetic() || c == b'_' => {
                let mut e = i + 1;
                while e < self.n && ascii_word(b[e]) {
                    e += 1;
                }
                (e >= self.n || (b[e] < 0x80 && b[e] != b'\\')).then_some(e)
            }
            _ => None,
        };
        self.ncache.set((i, end));
        end
    }

    /// `{{name}}` at `i`, each end it can take (best first) given to `f` until it says yes.
    pub(super) fn names(&self, i: usize, f: &mut dyn FnMut(usize) -> bool) -> bool {
        if self.b.get(i) == Some(&b'@') {
            let j = i + 1;
            for r in &RESERVED {
                if self.lit(j, r) && f(j + r.len()) {
                    return true;
                }
            }
            for t in &BASE {
                if self.lit(j, t) && !self.word_at(j + t.len()) && f(j + t.len()) {
                    return true;
                }
            }
            if self.lit(j, b"var") && f(j + 3) {
                return true;
            }
            return self.name_normal(j, f);
        }
        self.name_normal(i, f)
    }

    /// The first end `{{name}}` takes at `i`.
    #[inline]
    pub(super) fn name(&self, i: usize) -> Option<usize> {
        if let Some(e) = self.ascii_name(i) {
            return Some(e);
        }
        let mut end = None;
        self.names(i, &mut |e| {
            end = Some(e);
            true
        });
        end
    }

    /// Whether a word of `{{reserved}}` starts at `i` (`(?={{reserved}})`, no `\\b`).
    pub(super) fn starts_reserved(&self, i: usize) -> bool {
        let w = self.word(i);
        let Some(&c) = w.first() else {
            return false;
        };
        // the list is sorted: the words with `c` first are a run of it
        let from = RESERVED.partition_point(|r| r[0] < c);
        RESERVED[from..]
            .iter()
            .take_while(|r| r[0] == c)
            .any(|r| w.starts_with(r))
    }

    /// `{{reserved}}\b` at `i`: where it ends.
    pub(super) fn reserved_word(&self, i: usize) -> Option<usize> {
        let w = self.word(i);
        is_reserved(w).then_some(i + w.len())
    }

    /// `{{base_type}}` at `i` (its `\b` after, none before): where it ends.
    pub(super) fn base_type(&self, i: usize) -> Option<usize> {
        let w = self.word(i);
        matches!(
            w,
            b"bool"
                | b"byte"
                | b"sbyte"
                | b"char"
                | b"decimal"
                | b"double"
                | b"float"
                | b"int"
                | b"uint"
                | b"long"
                | b"ulong"
                | b"short"
                | b"ushort"
                | b"object"
                | b"string"
                | b"void"
        )
        .then_some(i + w.len())
    }

    /// `{{visibility}}` at `i`: `\b(?:public|private|protected|internal|protected\s+internal)\b`.
    pub(super) fn visibility(&self, i: usize) -> Option<usize> {
        if self.word_before(i) {
            return None;
        }
        for w in [&b"public"[..], b"private", b"protected", b"internal"] {
            if self.kw_end(i, w) {
                return Some(i + w.len());
            }
        }
        // `protected` followed by a word char fails above; `protected\s+internal` can't
        // match where `protected\b` didn't
        None
    }

    /// `\s*(<[^(={};<]*>)?\s*` at `i`: each end, best first, to `f`.
    pub(super) fn generic_decl(&self, i: usize, f: &mut dyn FnMut(usize) -> bool) -> bool {
        let s = self.sp(i);
        if self.ch(s) == b'<' {
            // the run to a stop byte, then each `>` in it, last first
            let mut r = s + 1;
            while self.on(r) && !matches!(self.ch(r), b'(' | b'=' | b'{' | b'}' | b';' | b'<') {
                r += 1;
            }
            let mut g = r;
            while g > s + 1 {
                g -= 1;
                if self.ch(g) == b'>' {
                    let e = self.sp(g + 1);
                    for e2 in (g + 1..=e).rev() {
                        if f(e2) {
                            return true;
                        }
                    }
                }
            }
        }
        (i..=s).rev().any(f)
    }

    /// `{{type_suffix}}`, `(?:\s*(?:\[,*\]|\*|\?)*)`, at `i`: each end, best first, to `f`.
    pub(super) fn type_suffix(&self, i: usize, f: &mut dyn FnMut(usize) -> bool) -> bool {
        let s = self.sp(i);
        // the elements' ends, greedy; tried longest first
        let mut ends = [0usize; 16];
        let mut k = 0;
        let mut q = s;
        while k < ends.len() {
            match self.ch(q) {
                b'*' | b'?' if q < self.n => q += 1,
                b'[' if q < self.n => {
                    let mut r = q + 1;
                    while r < self.n && self.b[r] == b',' {
                        r += 1;
                    }
                    if r < self.n && self.b[r] == b']' {
                        q = r + 1;
                    } else {
                        break;
                    }
                }
                _ => break,
            }
            ends[k] = q;
            k += 1;
        }
        if ends[..k].iter().rev().any(|&e| f(e)) || f(s) {
            return true;
        }
        (i..s).rev().any(f)
    }

    /// `{{namespaced_name}}`, `(?:(?:{{name}}{{generic_declaration}}\s*\.\s*)*{{name}}{{generic_declaration}})`,
    /// at `i`: each end, best first, to `f`.
    pub(super) fn namespaced(&self, i: usize, f: &mut dyn FnMut(usize) -> bool) -> bool {
        self.names(i, &mut |e| {
            self.generic_decl(e, &mut |g| {
                // another segment first (the `*` is greedy), then this one the last
                let s = self.sp(g);
                if self.ch(s) == b'.' && s < self.n {
                    let t = self.sp(s + 1);
                    for t2 in (s + 1..=t).rev() {
                        if self.namespaced(t2, f) {
                            return true;
                        }
                    }
                }
                f(g)
            })
        })
    }

    /// `{{type_suffix_capture}}`, `(\?)?((\[)(,*)(\]))?(?:\s*(\*))?`, at `i`, greedy: the `?`'s
    /// end, the brackets' (`[`, the commas' end, `]`'s end) and the `*`'s place.
    pub(super) fn type_suffix_capture(&self, i: usize) -> Suffix {
        let mut q = i;
        let question = (q < self.n && self.b[q] == b'?').then(|| {
            q += 1;
            q
        });
        let mut brackets = None;
        if q < self.n && self.b[q] == b'[' {
            let mut r = q + 1;
            while r < self.n && self.b[r] == b',' {
                r += 1;
            }
            if r < self.n && self.b[r] == b']' {
                brackets = Some((q, r));
                q = r + 1;
            }
        }
        let s = self.sp(q);
        let star = (s < self.n && self.b[s] == b'*').then_some(s);
        let end = star.map_or(q, |s| s + 1);
        Suffix {
            question,
            brackets,
            star,
            end,
        }
    }

    /// `{{escaped_char}}` at `i`: where it ends.
    pub(super) fn escaped_char(&self, i: usize) -> Option<usize> {
        let b = self.b;
        if b.get(i) != Some(&b'\\') {
            return None;
        }
        let c = *b.get(i + 1)?;
        match c {
            b'a' | b'b' | b'f' | b'n' | b'r' | b't' | b'v' | b'"' | b'\'' | b'\\' => Some(i + 2),
            b'u' | b'U' => self.unicode_escape(i),
            b'x' => {
                let mut e = i + 2;
                while e < self.n && e < i + 6 && is_hex(b[e]) {
                    e += 1;
                }
                (e > i + 2).then_some(e)
            }
            b'0'..=b'9' => {
                let mut e = i + 1;
                while e < self.n && e < i + 4 && b[e].is_ascii_digit() {
                    e += 1;
                }
                Some(e)
            }
            _ => None,
        }
    }

    /// `{{escaped_char}}` at `i` followed by `'`: the escape's end, its backtracking tried.
    pub(super) fn escaped_char_then_quote(&self, i: usize) -> Option<usize> {
        let b = self.b;
        let e = self.escaped_char(i)?;
        if self.ch(e) == b'\'' && e < self.n {
            return Some(e);
        }
        // `\x` and `\digits` take fewer
        let min = match b.get(i + 1) {
            Some(b'x') => i + 3,
            Some(b'0'..=b'9') => i + 2,
            _ => return None,
        };
        (min..e)
            .rev()
            .find(|&e2| self.ch(e2) == b'\'' && e2 < self.n)
    }

    /// `{{dec_digits}}`, `[\d_]*\d`, at `i`: its ends, longest first.
    pub(super) fn dec_digits(&self, i: usize, f: &mut dyn FnMut(usize) -> bool) -> bool {
        // the run of digits and `_`, then back from its end to each digit's end
        let mut e = i;
        loop {
            if e < self.n && self.b[e] == b'_' {
                e += 1;
            } else if let Some(d) = self.digit(e) {
                e = d;
            } else {
                break;
            }
        }
        while e > i {
            if self.b[e - 1] == b'_' {
                e -= 1;
                continue;
            }
            if f(e) {
                return true;
            }
            // back past this digit (one past ASCII is several bytes)
            e -= 1;
            while e > i && self.b[e] & 0xc0 == 0x80 {
                e -= 1;
            }
        }
        false
    }

    /// `\d` at `i` (Unicode's decimal digits): where it ends.
    #[inline]
    pub(super) fn digit(&self, i: usize) -> Option<usize> {
        let c = *self.b.get(i)?;
        if c.is_ascii_digit() {
            return Some(i + 1);
        }
        if c < 0x80 {
            return None;
        }
        let ch = self.char_at(i)?;
        let u = ch as u32;
        ND.iter()
            .any(|&s| u >= s && u - s < 10)
            .then(|| i + ch.len_utf8())
    }

    /// `\d+` at `i`: where it ends, if it takes one.
    pub(super) fn digits(&self, i: usize) -> Option<usize> {
        let mut e = self.digit(i)?;
        while let Some(d) = self.digit(e) {
            e = d;
        }
        Some(e)
    }

    /// `\s+{{name}}` at `i`: each end of the name, best first, to `f`.
    pub(super) fn spaced_name(&self, i: usize, f: &mut dyn FnMut(usize) -> bool) -> bool {
        match self.sp1(i) {
            Some(s) => self.names(s, f),
            None => false,
        }
    }

    /// `{{namespaced_name}}{{type_suffix}}\s+{{name}}` at `i`, the rest tested by `f` at the
    /// end of the last name.
    pub(super) fn declaration(&self, i: usize, f: &mut dyn FnMut(usize) -> bool) -> bool {
        self.namespaced(i, &mut |e| {
            self.type_suffix(e, &mut |t| self.spaced_name(t, &mut |n| f(n)))
        })
    }
}

/// What `{{type_suffix_capture}}` took.
pub(super) struct Suffix {
    pub question: Option<usize>,
    /// The `[` and the `]`.
    pub brackets: Option<(usize, usize)>,
    pub star: Option<usize>,
    pub end: usize,
}
