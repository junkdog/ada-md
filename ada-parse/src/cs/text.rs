//! What isn't code: strings (regular, verbatim, interpolated, raw), characters, numbers,
//! comments, doc comments, preprocessor lines and attributes. Each runs in a loop of its own,
//! its spans written as it goes; a string's interpolation is a frame the code loop runs in
//! ([`F_INTERP`]…), whose `}` hands the string back.

use super::names::{blanks, name_end, name_start};
use super::words;
use super::*;
use crate::prim::char_len;

/// Where a regular string's or a character's escape at `p` (its `\`) ends: C#'s escapes, `\u`
/// with four hex digits, `\U` with eight, `\x` with one to four; another a `\` and a character.
pub(super) fn escape_end(b: &[u8], p: usize) -> usize {
    let hex = |q: usize| b.get(q).is_some_and(u8::is_ascii_hexdigit);
    match b.get(p + 1) {
        Some(b'u') if (p + 2..p + 6).all(hex) => p + 6,
        Some(b'U') if (p + 2..p + 10).all(hex) => p + 10,
        Some(b'x') if hex(p + 2) => (p + 3..p + 6).find(|&q| !hex(q)).unwrap_or(p + 6),
        Some(&c) => p + 1 + char_len(c),
        None => p + 1,
    }
}

/// A regular string's stops: its end, an escape, a brace.
static STR_STOPS: [bool; 256] = crate::prim::stops(b"\"\\{}");
/// A verbatim string's.
static VSTR_STOPS: [bool; 256] = crate::prim::stops(b"\"{}");

impl<K: Sink> Scan<'_, '_, K> {
    /// `"` at `p`: a raw string where three or more quotes open it, else a regular one.
    #[inline(always)]
    pub(super) fn quote(&mut self, p: usize, s: usize, dollars: u8) -> usize {
        let b = self.b;
        if b.get(p + 1) == Some(&b'"') && b.get(p + 2) == Some(&b'"') {
            return self.raw_open(s, p, dollars);
        }
        self.put_hot(s, p + 1, STR_BEGIN);
        if dollars == 0 && self.regex_at(s) {
            return self.regex_body(p + 1, false);
        }
        self.str_body(p + 1, dollars != 0)
    }

    /// Whether the string that starts at `s` is a regex, by the call it is an argument of.
    #[inline(always)]
    fn regex_at(&mut self, s: usize) -> bool {
        let b = self.b;
        let q = regex_calls::blank_back(b, s);
        q > 0
            && match b[q - 1] {
                b'(' => regex_calls::regex_first_argument(b, q),
                b',' => self
                    .rx
                    .get_or_insert_with(|| regex_calls::RegexCalls::find(b))
                    .has_comma(q - 1),
                _ => false,
            }
    }

    /// A regex's text in a string from `p` (`verbatim`: a verbatim one's) on [`crate::regex`]:
    /// the string's end and escapes the host's (a verbatim one's `""`), the rest the regex's; a
    /// verbatim one goes on past the line ([`M_VREGEX`]).
    pub(super) fn regex_body(&mut self, mut p: usize, verbatim: bool) -> usize {
        static YIELDS: [[bool; 256]; 2] = [regex::table(b"\"\\"), regex::table(b"\"")];
        let b = self.b;
        let n = b.len();
        let host = regex::Embed {
            dialect: regex::Dialect::DotNet,
            cooked: !verbatim,
            yields: &YIELDS[usize::from(verbatim)],
            stop: |b, p, _| b[p] == b'"',
            arg: 0,
            escape: regex_calls::escape_end_rx,
            top_only: false,
            continues: false,
            base: if verbatim { RE_VSTR } else { RE_STR },
            text: if verbatim { VSTR } else { STR },
        };
        while p < n {
            match b[p] {
                b'"' if verbatim && b.get(p + 1) == Some(&b'"') => {
                    self.put(p, p + 2, VSTR_ESCAPE);
                    p += 2;
                    continue;
                }
                b'"' => {
                    self.mode = M_CODE;
                    self.re = 0;
                    return self.str_end(p, if verbatim { VSTR_END } else { STR_END });
                }
                b'\\' if !verbatim && b.get(p + 1) != Some(&b'\\') => {
                    let e = regex_calls::escape_end_rx(b, p).max(p + 1).min(n);
                    self.put(p, e, STR_ESCAPE);
                    p = e;
                    continue;
                }
                _ => {}
            }
            self.k.close(self.out);
            let mut at = p;
            let r = regex::embedded(b, p, self.re, &host, &mut at, self.out);
            self.k.reopen(self.out, 16);
            match r {
                Some((e, st)) => {
                    self.re = st;
                    p = e.max(p + 1);
                }
                None => break,
            }
        }
        if verbatim {
            self.mode = M_VREGEX;
        } else {
            self.re = 0;
            self.prev = P_VALUE;
        }
        n
    }

    /// `@` at `p`: a verbatim string (`@"`, `@$"`), or a verbatim name (`@class`).
    pub(super) fn at_sign(&mut self, p: usize) -> usize {
        let b = self.b;
        match b.get(p + 1) {
            Some(b'"') => {
                self.put(p, p + 2, VSTR_BEGIN);
                if self.regex_at(p) {
                    return self.regex_body(p + 2, true);
                }
                self.vstr_body(p + 2, false)
            }
            Some(b'$') if b.get(p + 2) == Some(&b'"') => {
                self.put(p, p + 3, VSTR_BEGIN);
                self.vstr_body(p + 3, true)
            }
            _ if name_start(b, p) => self.name(p),
            _ => p + 1,
        }
    }

    /// `$` at `p`: an interpolated string, regular (`$"`), verbatim (`$@"`) or raw (`$"""`,
    /// `$$"""`…).
    pub(super) fn dollar(&mut self, p: usize) -> usize {
        let b = self.b;
        let mut q = p;
        while b.get(q) == Some(&b'$') {
            q += 1;
        }
        let d = (q - p).min(255) as u8;
        match b.get(q) {
            Some(b'"') => self.quote(q, p, d),
            Some(b'@') if b.get(q + 1) == Some(&b'"') => {
                self.put(p, q + 2, VSTR_BEGIN);
                self.vstr_body(q + 2, true)
            }
            _ => q,
        }
    }

    /// A regular string's text from `p` (past its `"`, or an interpolation's `}`), where
    /// `interp` an interpolated one's: to its end, an interpolation's `{` (the hole's frame
    /// pushed, the code goes on in it), or the line's end (a regular string ends with its line).
    pub(super) fn str_body(&mut self, mut p: usize, interp: bool) -> usize {
        let b = self.b;
        let n = b.len();
        let mut t = p;
        while p < n {
            if !STR_STOPS[usize::from(b[p])] {
                p += 1;
                continue;
            }
            match b[p] {
                b'"' => {
                    self.put_hot(t, p, STR);
                    return self.str_end(p, STR_END);
                }
                b'\\' => {
                    self.put(t, p, STR);
                    let e = escape_end(b, p).min(n);
                    self.put(p, e, STR_ESCAPE);
                    p = e;
                    t = p;
                }
                c if b.get(p + 1) == Some(&c) => {
                    // `{{`, `}}`: an escape (bat's, in any string)
                    self.put(t, p, STR);
                    self.put(p, p + 2, STR_ESCAPE);
                    p += 2;
                    t = p;
                }
                b'{' if interp => {
                    self.put(t, p, STR);
                    self.put(p, p + 1, INTERP_BEGIN);
                    self.push(F_INTERP);
                    self.prev = P_EXPR;
                    return p + 1;
                }
                b'{' => match self.placeholder(p, t, false) {
                    Some(e) => {
                        p = e;
                        t = p;
                    }
                    None => p += 1,
                },
                _ => p += 1,
            }
        }
        self.put(t, n, STR);
        self.prev = P_VALUE;
        n
    }

    /// A string's closing `"` at `p` (`leaf`), a UTF-8 string's `u8` after it.
    #[inline(always)]
    fn str_end(&mut self, p: usize, leaf: u16) -> usize {
        let b = self.b;
        self.put_hot(p, p + 1, leaf);
        self.prev = P_VALUE;
        if b.get(p + 1..p + 3) == Some(b"u8")
            && !b.get(p + 3).is_some_and(|&c| NAME[usize::from(c)])
        {
            self.put(p + 1, p + 3, STR_SUFFIX);
            return p + 3;
        }
        p + 1
    }

    /// A composite format's placeholder at `p` (its `{`), `{0}`, `{0,-5}`, `{0:N2}`, in a
    /// string that isn't interpolated (`verbatim`: a verbatim one's), the string's text from `t`
    /// before it: its tokens, where it ends.
    fn placeholder(&mut self, p: usize, t: usize, verbatim: bool) -> Option<usize> {
        let b = self.b;
        let mut q = p + 1;
        while b.get(q).is_some_and(u8::is_ascii_digit) {
            q += 1;
        }
        if q == p + 1 {
            return None;
        }
        let d = q;
        // `,` and an alignment, `:` and a format, to the `}`
        let mut align = None;
        let s = blanks(b, q);
        if b.get(s) == Some(&b',') {
            let a = blanks(b, s + 1);
            let mut e = a;
            if b.get(e) == Some(&b'-') {
                e += 1;
            }
            let ds = e;
            while b.get(e).is_some_and(u8::is_ascii_digit) {
                e += 1;
            }
            if e == ds {
                return None;
            }
            align = Some((s, a, e));
            q = blanks(b, e);
        }
        let colon = (b.get(q) == Some(&b':')).then_some(q);
        while let Some(&c) = b.get(q) {
            match c {
                b'}' => break,
                b'"' | b'{' | b'\\' => return None,
                _ if colon.is_none() && !matches!(c, b' ' | b'\t') => return None,
                _ => q += 1,
            }
        }
        if q >= b.len() {
            return None;
        }
        let (before, begin, num, text, end) = if verbatim {
            (VSTR, VSTR_PH_BEGIN, VSTR_PH_NUM, VSTR_PH, VSTR_PH_END)
        } else {
            (STR, STR_PH_BEGIN, STR_PH_NUM, STR_PH, STR_PH_END)
        };
        self.put(t, p, before);
        self.put(p, p + 1, begin);
        self.put(p + 1, d, num);
        let mut at = d;
        if let Some((s, a, e)) = align {
            let (sep, al) = if verbatim {
                (VSTR_PH_ALIGN_SEP, VSTR_PH_ALIGN)
            } else {
                (STR_PH_ALIGN_SEP, STR_PH_ALIGN)
            };
            self.put(at, s, text);
            self.put(s, s + 1, sep);
            self.put(s + 1, a, text);
            self.put(a, e, al);
            at = e;
        }
        if let Some(c) = colon {
            let (sep, fmt) = if verbatim {
                (VSTR_PH_FORMAT_SEP, VSTR_PH_FORMAT)
            } else {
                (STR_PH_FORMAT_SEP, STR_PH_FORMAT)
            };
            self.put(at, c, text);
            self.put(c, c + 1, sep);
            self.put(c + 1, q, fmt);
            at = q;
        }
        self.put(at, q, text);
        self.put(q, q + 1, end);
        Some(q + 1)
    }

    /// A verbatim string's text from `p`, where `interp` an interpolated one's: to its end, a
    /// hole's `{`, or the line's end, where it goes on ([`M_VSTR`], [`M_VISTR`]).
    pub(super) fn vstr_body(&mut self, mut p: usize, interp: bool) -> usize {
        let b = self.b;
        let n = b.len();
        let mut t = p;
        while p < n {
            if !VSTR_STOPS[usize::from(b[p])] {
                p += 1;
                continue;
            }
            match b[p] {
                b'"' if b.get(p + 1) == Some(&b'"') => {
                    self.put(t, p, VSTR);
                    self.put(p, p + 2, VSTR_ESCAPE);
                    p += 2;
                    t = p;
                }
                b'"' => {
                    self.put(t, p, VSTR);
                    self.mode = M_CODE;
                    return self.str_end(p, VSTR_END);
                }
                c if b.get(p + 1) == Some(&c) => {
                    self.put(t, p, VSTR);
                    self.put(p, p + 2, VSTR_ESCAPE);
                    p += 2;
                    t = p;
                }
                b'{' if interp => {
                    self.put(t, p, VSTR);
                    self.put(p, p + 1, INTERP_BEGIN);
                    self.push(F_VINTERP);
                    self.mode = M_CODE;
                    self.prev = P_EXPR;
                    return p + 1;
                }
                b'{' => match self.placeholder(p, t, true) {
                    Some(e) => {
                        p = e;
                        t = p;
                    }
                    None => p += 1,
                },
                _ => p += 1,
            }
        }
        self.put(t, n, VSTR);
        self.mode = if interp { M_VISTR } else { M_VSTR };
        n
    }

    /// A raw string at `q` (its first `"`), from `s` (its `$`s, `dollars` of them): its
    /// quotes, then a single line's text, or nothing more on a multi-line one's first line.
    fn raw_open(&mut self, s: usize, q: usize, dollars: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let mut e = q;
        while b.get(e) == Some(&b'"') {
            e += 1;
        }
        let k = (e - q).min(127) as u8;
        self.put(s, e, VSTR_BEGIN);
        self.rawd = dollars;
        if blanks(b, e) == n {
            // a multi-line one: its text from the next line
            self.rawq = k | 0x80;
            self.mode = M_RAW;
            self.prev = P_VALUE;
            return n;
        }
        self.rawq = k;
        self.raw_body(e)
    }

    /// A raw string's text from `p`: to its closing quotes, a hole's `{`s, or the line's end
    /// (a multi-line one's goes on; a single line's ends).
    pub(super) fn raw_body(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let k = usize::from(self.rawq & 0x7f).max(3);
        let d = usize::from(self.rawd);
        let t = p;
        while p < n {
            match b[p] {
                b'"' => {
                    let mut e = p;
                    while b.get(e) == Some(&b'"') {
                        e += 1;
                    }
                    if e - p >= k {
                        self.put(t, p, VSTR);
                        self.put(p, e, VSTR_END);
                        self.mode = M_CODE;
                        self.rawq = 0;
                        self.prev = P_VALUE;
                        return e;
                    }
                    p = e;
                }
                b'{' if d > 0 => {
                    let mut e = p;
                    while b.get(e) == Some(&b'{') {
                        e += 1;
                    }
                    if e - p >= d {
                        self.put(t, e - d, VSTR);
                        self.put(e - d, e, INTERP_BEGIN);
                        self.push(F_RINTERP);
                        self.mode = M_CODE;
                        self.prev = P_EXPR;
                        return e;
                    }
                    p = e;
                }
                _ => p += 1,
            }
        }
        self.put(t, n, VSTR);
        if self.rawq & 0x80 != 0 {
            self.mode = M_RAW;
        } else {
            self.rawq = 0;
            self.prev = P_VALUE;
        }
        n
    }

    /// `}` at `p` closing an interpolation's hole: the string after it.
    pub(super) fn interp_close(&mut self, p: usize) -> usize {
        let b = self.b;
        let f = self.top();
        self.pop();
        match f {
            F_INTERP => {
                self.put(p, p + 1, INTERP_END);
                self.str_body(p + 1, true)
            }
            F_VINTERP => {
                self.put(p, p + 1, INTERP_END);
                self.vstr_body(p + 1, true)
            }
            _ => {
                let d = usize::from(self.rawd).max(1);
                let mut e = p;
                while e < p + d && b.get(e) == Some(&b'}') {
                    e += 1;
                }
                self.put(p, e, INTERP_END);
                self.raw_body(e)
            }
        }
    }

    /// `:` at `p` in a hole: its format, to the hole's `}`.
    pub(super) fn format_spec(&mut self, p: usize) -> usize {
        let b = self.b;
        self.put(p, p + 1, FORMAT_SEP);
        let mut e = p + 1;
        while e < b.len() && b[e] != b'}' && b[e] != b'"' {
            e += 1;
        }
        self.put(p + 1, e, FORMAT);
        e
    }

    /// `,` at `p` in a hole: its alignment, `-?\d+`.
    pub(super) fn interp_align(&mut self, p: usize) -> usize {
        let b = self.b;
        self.put(p, p + 1, ALIGN_SEP);
        let s = blanks(b, p + 1);
        let mut e = s;
        if b.get(e) == Some(&b'-') {
            e += 1;
        }
        let d = e;
        while b.get(e).is_some_and(u8::is_ascii_digit) {
            e += 1;
        }
        if e > d {
            self.put(s, e, ALIGN);
        }
        self.prev = P_VALUE;
        e
    }

    /// A line that starts in a mode ([`Scan::mode`]): its rest, where the code goes on.
    pub(super) fn resume(&mut self) -> usize {
        match self.mode {
            M_COMMENT => self.block_comment(0),
            M_VSTR => self.vstr_body(0, false),
            M_VISTR => self.vstr_body(0, true),
            M_RAW => self.raw_body(0),
            M_VREGEX => self.regex_body(0, true),
            _ => {
                self.mode = M_CODE;
                0
            }
        }
    }

    /// `'` at `p`: a character, `'a'`, `'\n'`, `'A'`; a lone `'` plain.
    pub(super) fn char_lit(&mut self, p: usize) -> usize {
        let b = self.b;
        self.prev = P_VALUE;
        match b.get(p + 1) {
            Some(b'\\') => {
                let e = escape_end(b, p + 1).min(b.len());
                if b.get(e) == Some(&b'\'') {
                    self.put(p, p + 1, CHAR);
                    self.put(p + 1, e, CHAR_ESCAPE);
                    self.put(e, e + 1, CHAR);
                    return e + 1;
                }
                p + 1
            }
            Some(&c) => {
                let e = p + 1 + char_len(c);
                if b.get(e) == Some(&b'\'') {
                    self.put(p, e + 1, CHAR);
                    return e + 1;
                }
                p + 1
            }
            None => p + 1,
        }
    }

    /// A number at `p` (a digit, or `.` before one): an integer, a float, a hex or binary
    /// one, their suffixes.
    pub(super) fn number(&mut self, p: usize) -> usize {
        let b = self.b;
        let digits = |mut q: usize, hex: bool| {
            while let Some(&c) = b.get(q) {
                if c.is_ascii_digit() || c == b'_' || hex && c.is_ascii_hexdigit() {
                    q += 1;
                } else {
                    break;
                }
            }
            q
        };
        self.prev = P_VALUE;
        if b[p] == b'0' && matches!(b.get(p + 1), Some(b'x' | b'X' | b'b' | b'B')) {
            let hex = b[p + 1] | 0x20 == b'x';
            let e = digits(p + 2, hex);
            let mut s = e;
            while b
                .get(s)
                .is_some_and(|&c| matches!(c, b'u' | b'U' | b'l' | b'L'))
            {
                s += 1;
            }
            if hex {
                self.put(p, p + 2, HEX_BASE);
                self.put(p + 2, e, HEX);
                self.put(e, s, HEX_SUFFIX);
            } else {
                self.put(p, p + 2, BIN_BASE);
                self.put(p + 2, s, BIN);
            }
            return s;
        }
        let i = digits(p, false);
        let mut e = i;
        let mut float = false;
        // a fraction
        if b.get(e) == Some(&b'.') && b.get(e + 1).is_some_and(u8::is_ascii_digit) {
            float = true;
            e = digits(e + 1, false);
        }
        // an exponent
        if matches!(b.get(e), Some(b'e' | b'E')) {
            let mut q = e + 1;
            if matches!(b.get(q), Some(b'+' | b'-')) {
                q += 1;
            }
            if b.get(q).is_some_and(u8::is_ascii_digit) {
                float = true;
                e = digits(q, false);
            }
        }
        let mut s = e;
        let fsuf = matches!(b.get(e), Some(b'f' | b'F' | b'd' | b'D' | b'm' | b'M'));
        if fsuf {
            s = e + 1;
        } else {
            while b
                .get(s)
                .is_some_and(|&c| matches!(c, b'u' | b'U' | b'l' | b'L'))
            {
                s += 1;
            }
        }
        if float || fsuf {
            if i < e && b[i] == b'.' {
                self.put(p, i, FLOAT);
                self.put(i, i + 1, FLOAT_DOT);
                self.put(i + 1, e, FLOAT);
            } else {
                self.put(p, e, FLOAT);
            }
            self.put(e, s, FLOAT_SUFFIX);
        } else {
            self.put_hot(p, e, DEC);
            self.put(e, s, DEC_SUFFIX);
        }
        s
    }

    /// `//` at `p`: a comment to the line's end.
    #[inline(always)]
    pub(super) fn line_comment(&mut self, p: usize) -> usize {
        let n = self.b.len();
        self.put_hot(p, p + 2, LC_MARK);
        self.put_hot(p + 2, n, LC_TEXT);
        n
    }

    /// `/*` at `p`.
    pub(super) fn block_comment_open(&mut self, p: usize) -> usize {
        self.put(p, p + 2, BC_BEGIN);
        self.block_comment(p + 2)
    }

    /// A block comment's text from `p`: to its `*/`, or the line's end ([`M_COMMENT`]).
    pub(super) fn block_comment(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut q = p;
        while q + 1 < n {
            if b[q] == b'*' && b[q + 1] == b'/' {
                self.put(p, q, BC_TEXT);
                self.put(q, q + 2, BC_END);
                self.mode = M_CODE;
                return q + 2;
            }
            q += 1;
        }
        self.put(p, n, BC_TEXT);
        self.mode = M_COMMENT;
        n
    }

    /// A doc comment's line from `p` (its `///`): its text and XML tags.
    pub(super) fn doc_line(&mut self, p: usize) {
        let b = self.b;
        let n = b.len();
        self.put(p, p + 3, DOC_MARK);
        let mut q = p + 3;
        let mut t = q;
        loop {
            q = crate::prim::find_any(b, q, *b"<");
            if q >= n {
                break;
            }
            if let Some(e) = self.doc_tag(q, t) {
                q = e;
                t = q;
                continue;
            }
            q += 1;
        }
        self.put(t, n, DOC_TEXT);
    }

    /// A doc comment's tag at `q` (its `<`), the text from `t` before it: `</name>`, or
    /// `<name attr="value" …>` (`/>`). Where it ends, or `None` where it is no tag.
    fn doc_tag(&mut self, q: usize, t: usize) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        let tag_name = |s: usize| {
            let mut e = s;
            while e < n && (b[e].is_ascii_alphanumeric() || matches!(b[e], b'_' | b'-' | b':')) {
                e += 1;
            }
            (e > s && b[s].is_ascii_alphabetic()).then_some(e)
        };
        if b.get(q + 1) == Some(&b'/') {
            let e = tag_name(q + 2)?;
            let g = blanks(b, e);
            if b.get(g) != Some(&b'>') {
                return None;
            }
            self.put(t, q, DOC_TEXT);
            self.put(q, q + 2, DOC_TAG_BEGIN);
            self.put(q + 2, e, DOC_NAME_END);
            self.put(e, g, DOC);
            self.put(g, g + 1, DOC_TAG_END);
            return Some(g + 1);
        }
        let e = tag_name(q + 1)?;
        self.put(t, q, DOC_TEXT);
        self.put(q, q + 1, DOC_TAG_BEGIN);
        self.put(q + 1, e, DOC_NAME_BEGIN);
        let mut r = e;
        loop {
            let s = blanks(b, r);
            self.put(r, s, DOC);
            r = s;
            match b.get(r) {
                Some(b'>') => {
                    self.put(r, r + 1, DOC_TAG_END);
                    return Some(r + 1);
                }
                Some(b'/') if b.get(r + 1) == Some(&b'>') => {
                    self.put(r, r + 2, DOC_TAG_END);
                    return Some(r + 2);
                }
                Some(_) => {
                    let Some(ae) = tag_name(r) else {
                        return Some(r);
                    };
                    self.put(r, ae, DOC_ATTR);
                    let s = blanks(b, ae);
                    if b.get(s) != Some(&b'=') {
                        return Some(ae);
                    }
                    self.put(ae, s, DOC);
                    self.put(s, s + 1, DOC_EQ);
                    let v = blanks(b, s + 1);
                    self.put(s + 1, v, DOC);
                    match b.get(v) {
                        Some(&qc @ (b'"' | b'\'')) => {
                            let Some(k) = b[v + 1..].iter().position(|&c| c == qc) else {
                                self.put(v, n, DOC_STR);
                                return Some(n);
                            };
                            self.put(v, v + k + 2, DOC_STR);
                            r = v + k + 2;
                        }
                        _ => return Some(v),
                    }
                }
                None => return Some(n),
            }
        }
    }

    /// A preprocessor directive's line from `p` (its `#`): its word, its rest by the word, the
    /// gaps between `meta.preprocessor`'s text.
    pub(super) fn directive(&mut self, p: usize) {
        let b = self.b;
        let n = b.len();
        // its indentation the directive's (bat's `^\s*#`)
        let i = if b.starts_with(b"\xef\xbb\xbf") { 3 } else { 0 };
        self.put(i, p, PP_TEXT);
        self.put(p, p + 1, PP_HASH);
        let s = blanks(b, p + 1);
        let mut e = s;
        while e < n && b[e].is_ascii_alphabetic() {
            e += 1;
        }
        let w = &b[s..e];
        let leaf = match w {
            b"if" | b"elif" | b"else" | b"endif" | b"define" | b"undef" => PP_CONTROL,
            b"region" | b"endregion" | b"pragma" | b"line" | b"warning" | b"error"
            | b"nullable" => PP_OTHER,
            _ => PP_TEXT,
        };
        self.put(p + 1, e, leaf);
        let mut at = e;
        let mut q = e;
        // the rest, by the directive
        let section = matches!(w, b"region" | b"endregion");
        let message = matches!(w, b"warning" | b"error");
        if section || message {
            let r = blanks(b, q);
            if r < n {
                let (txt, end) = match b[r..].windows(2).position(|x| x == b"//") {
                    Some(k) if !section && !message => (r..r + k, r + k),
                    _ => (r..n, n),
                };
                self.put(at, txt.start, PP_TEXT);
                self.put(
                    txt.start,
                    txt.end,
                    if section { PP_SECTION } else { PP_MESSAGE },
                );
                at = end;
            }
            self.put(at, n, PP_TEXT);
            return;
        }
        let expr = matches!(w, b"if" | b"elif");
        while q < n {
            let c = b[q];
            let (s, e, kind) = match c {
                b'/' if b.get(q + 1) == Some(&b'/') => {
                    self.put(at, q, PP_TEXT);
                    self.put(q, q + 2, PP_LC_MARK);
                    self.put(q + 2, n, PP_LC_TEXT);
                    return;
                }
                b'"' => {
                    let e = b[q + 1..]
                        .iter()
                        .position(|&c| c == b'"')
                        .map_or(n, |k| q + k + 2);
                    self.put(at, q, PP_TEXT);
                    self.put(q, q + 1, PP_STR_BEGIN);
                    if e > q + 1 && e <= n && b[e - 1] == b'"' && e - 1 > q {
                        self.put(q + 1, e - 1, PP_STR);
                        self.put(e - 1, e, PP_STR_END);
                    } else {
                        self.put(q + 1, e, PP_STR);
                    }
                    at = e;
                    q = e;
                    continue;
                }
                b'&' | b'|' if b.get(q + 1) == Some(&c) => (q, q + 2, PP_LOGICAL),
                b'=' | b'!' if b.get(q + 1) == Some(&b'=') => (q, q + 2, PP_LOGICAL),
                b'!' => (q, q + 1, PP_LOGICAL),
                b'(' | b')' => (q, q + 1, PP_PARENS),
                b'0'..=b'9' if !expr => {
                    let mut e = q;
                    while e < n && b[e].is_ascii_alphanumeric() {
                        e += 1;
                    }
                    (q, e, PP_NUM)
                }
                _ if c.is_ascii_alphabetic() || c == b'_' => {
                    let mut e = q;
                    while e < n && (b[e].is_ascii_alphanumeric() || b[e] == b'_') {
                        e += 1;
                    }
                    let word = &b[q..e];
                    let kind = match w {
                        b"if" | b"elif"
                            if crate::same(word, b"true") || crate::same(word, b"false") =>
                        {
                            PP_LANGUAGE
                        }
                        b"if" | b"elif" => PP_FLAG,
                        b"define" | b"undef" => PP_CONSTANT,
                        b"pragma"
                            if matches!(
                                word,
                                b"warning" | b"disable" | b"restore" | b"checksum" | b"enable"
                            ) =>
                        {
                            PP_OTHER
                        }
                        b"pragma" => PP_WARNING,
                        b"nullable" | b"line" => PP_OTHER,
                        _ => PP_TEXT,
                    };
                    (q, e, kind)
                }
                _ => {
                    q += 1;
                    continue;
                }
            };
            if kind != PP_TEXT {
                self.put(at, s, PP_TEXT);
                self.put(s, e, kind);
                at = e;
            }
            q = e;
        }
        self.put(at, n, PP_TEXT);
    }

    /// An attribute's list at `p` (its `[`): its frame pushed, its items read.
    pub(super) fn attribute(&mut self, p: usize) -> usize {
        self.put(p, p + 1, ANN_BEGIN);
        self.push(F_ATTR);
        self.attr = 0;
        self.prev = P_STMT;
        self.attr_run(p + 1)
    }

    /// An attribute's list from `p` (its frame open, [`F_ATTR_ARGS`] where its arguments are):
    /// names, targets, arguments, to its `]`, the gaps `meta.annotation`'s text. Where the code
    /// goes on.
    pub(super) fn attr_run(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut at = p;
        // the next name an attribute's (after `[`, `,` in the list)
        let mut head = self.top() == F_ATTR && self.prev == P_STMT;
        // a `typeof(` or `nameof(` open: its name a type's
        let mut ty = false;
        // the token before was a named argument's name (`name:`)
        let mut named = false;
        macro_rules! tok {
            ($s:expr, $e:expr, $k:expr) => {{
                let (s, e) = ($s, $e);
                if s > at {
                    self.put(at, s, ANN);
                }
                self.put(s, e, $k);
                at = e;
            }};
        }
        while p < n {
            let c = b[p];
            let was_named = named;
            if !matches!(c, b' ' | b'\t' | b'\r') {
                named = false;
            }
            match c {
                b' ' | b'\t' | b'\r' => p += 1,
                b']' if self.top() == F_ATTR => {
                    if p > at {
                        self.put(at, p, ANN);
                    }
                    self.put(p, p + 1, ANN_END);
                    self.pop();
                    self.prev =
                        if matches!(self.top(), F_PARAMS | F_HPARAMS | F_LPARAMS | F_IPARAMS) {
                            P_PARAM
                        } else {
                            P_STMT
                        };
                    return p + 1;
                }
                b',' if self.top() == F_ATTR => {
                    tok!(p, p + 1, ANN_SEP);
                    head = true;
                    p += 1;
                }
                b',' => {
                    tok!(p, p + 1, ANN_ARG_SEP);
                    p += 1;
                }
                b'(' => {
                    if self.top() == F_ATTR {
                        self.push(F_ATTR_ARGS);
                        self.attr = 0;
                    }
                    self.attr = self.attr.saturating_add(1);
                    tok!(p, p + 1, ANN_GROUP_BEGIN);
                    p += 1;
                }
                b')' if self.top() == F_ATTR_ARGS => {
                    tok!(p, p + 1, ANN_GROUP_END);
                    self.attr = self.attr.saturating_sub(1);
                    ty = false;
                    if self.attr == 0 {
                        self.pop();
                    }
                    p += 1;
                }
                b'"' | b'@' | b'$' | b'\'' => {
                    if p > at {
                        self.put(at, p, ANN);
                    }
                    let mode = self.mode;
                    let depth = self.depth;
                    let e = match c {
                        b'"' => self.quote(p, p, 0),
                        b'@' => self.at_sign(p),
                        b'$' => self.dollar(p),
                        _ => self.char_lit(p),
                    };
                    // an interpolated string's hole, a verbatim one across lines: the code's
                    if self.depth != depth || self.mode != mode {
                        return e;
                    }
                    at = e.max(p + 1);
                    p = at;
                }
                b'0'..=b'9' => {
                    if p > at {
                        self.put(at, p, ANN);
                    }
                    p = self.number(p);
                    at = p;
                }
                b'/' if b.get(p + 1) == Some(&b'/') => {
                    if p > at {
                        self.put(at, p, ANN);
                    }
                    self.line_comment(p);
                    return n;
                }
                b'/' if b.get(p + 1) == Some(&b'*') => {
                    if p > at {
                        self.put(at, p, ANN);
                    }
                    p = self.block_comment_open(p);
                    at = p;
                    if self.mode != M_CODE {
                        return n;
                    }
                }
                b'.' => {
                    tok!(p, p + 1, ANN_DOT);
                    p += 1;
                }
                b':' if self.top() == F_ATTR_ARGS && b.get(p + 1) != Some(&b':') && was_named => {
                    tok!(p, p + 1, ANN_ARG_VALUE_SEP);
                    p += 1;
                }
                b'=' if b.get(p + 1) != Some(&b'=') => {
                    tok!(p, p + 1, ANN_ASSIGN);
                    p += 1;
                }
                b'<' if head || ty => {
                    tok!(p, p + 1, ANN_GENERIC_BEGIN);
                    p += 1;
                }
                b'>' if head || ty => {
                    tok!(p, p + 1, ANN_GENERIC_END);
                    p += 1;
                }
                b':' if b.get(p + 1) == Some(&b':') => {
                    tok!(p, p + 2, ANN_DOT);
                    p += 2;
                }
                _ if ACT[usize::from(c)] == B_OP || c == b'?' || c == b':' => {
                    tok!(p, p + 1, ANN_OPERATOR);
                    p += 1;
                }
                _ if name_start(b, p) => {
                    let e = name_end(b, p);
                    let i = words::info(&b[p..e]);
                    if self.top() == F_ATTR
                        && head
                        && b.get(e) == Some(&b':')
                        && b.get(e + 1) != Some(&b':')
                    {
                        // a target, `assembly:`, `return:`…
                        tok!(p, e, ANN_TARGET);
                        tok!(e, e + 1, ANN_TARGET_SEP);
                        p = e + 1;
                        continue;
                    }
                    let kind = if self.top() == F_ATTR {
                        match b.get(e) {
                            Some(b'.') => ANN_VAR,
                            Some(b':') if b.get(e + 1) == Some(&b':') => ANN,
                            _ => ANN_NAME,
                        }
                    } else if i == 0 && {
                        let q = blanks(b, e);
                        b.get(q) == Some(&b'=') && b.get(q + 1) != Some(&b'=')
                            || b.get(q) == Some(&b':') && b.get(q + 1) != Some(&b':')
                    } {
                        named = true;
                        ANN_PARAM
                    } else if i != 0 {
                        let act = (i >> 8) as u8;
                        if matches!(act, words::A_TYPEOF | words::A_NAMEOF) {
                            ty = true;
                        }
                        match act {
                            words::A_TYPE => STORAGE_TYPE,
                            words::A_LITERAL => LANGUAGE,
                            _ if i & 0xff == 0 => ANN_VAR,
                            _ => i & 0xff,
                        }
                    } else if ty {
                        ANN_TYPE
                    } else {
                        ANN_VAR
                    };
                    tok!(p, e, kind);
                    p = e;
                }
                _ => p += char_len(c),
            }
            if !matches!(c, b',' | b' ' | b'\t' | b'\r') && self.top() == F_ATTR {
                head = c == b'.' || (c == b'[');
            }
        }
        if n > at {
            self.put(at, n, ANN);
        }
        self.prev = P_STMT;
        n
    }
}
