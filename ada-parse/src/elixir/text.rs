//! Elixir's text: strings, charlists, heredocs, sigils, quoted atoms and keys with their escapes
//! and `#{…}`s, the documentation attributes, and comments.

use super::*;
use crate::prim::find_in;

impl<K: Sink> Scan<'_, '_, K> {
    /// `"` at `p`: a heredoc (`"""`), a quoted key (`"…":`), or a string.
    #[inline(always)]
    pub(super) fn dquote(&mut self, p: usize) -> usize {
        self.quote(p, D_DQ)
    }

    /// `'` at `p`: a heredoc (`'''`), a quoted key (`'…':`), or a charlist.
    #[inline(always)]
    pub(super) fn squote(&mut self, p: usize) -> usize {
        self.quote(p, D_SQ)
    }

    #[inline(always)]
    fn quote(&mut self, p: usize, d: u8) -> usize {
        let b = self.b;
        let q = CLOSER[usize::from(d)];
        if !SOL
            && (matches!(self.top(), F_SQLCALL | F_RECALL) || self.callw.1 != usize::MAX)
            && let Some(r) = self.embedded_quote(p, d)
        {
            return r;
        }
        if b.get(p + 1) == Some(&q) && b.get(p + 2) == Some(&q) {
            return self.heredoc_open(p, p + 3, K_STR | HEREDOC | d);
        }
        let sk = K_STR | d;
        let m = self.k.mark(self.out);
        self.put_hot(p, p + 1, flavor(sk) + BEGIN);
        let e = self.str_body(p + 1, sk);
        if !SOL && b.get(e) == Some(&b':') && b[e - 1] == q && e > p + 1 && self.mode == M_CODE {
            // bat's `(["'])(?>\\.|(?!\1).)*\1:`: a keyword's key, quoted
            self.k.undo(self.out, m);
            let sk = K_KEY | d;
            self.put(p, p + 1, flavor(sk) + BEGIN);
            return self.str_body(p + 1, sk);
        }
        e
    }

    /// A `"` at `p` where a call may make its string SQL (a SQL call's arguments, or
    /// `execute "…"` without parentheses) or a regex (`Regex.compile(`'s first); `None` where it
    /// doesn't.
    #[cold]
    #[inline(never)]
    fn embedded_quote(&mut self, p: usize, d: u8) -> Option<usize> {
        let b = self.b;
        if d != D_DQ {
            return None;
        }
        let triple = b.get(p + 1) == Some(&b'"') && b.get(p + 2) == Some(&b'"');
        let (cs, ce) = self.callw;
        let bare = ce != usize::MAX
            && ce < p
            && &b[cs..ce] == b"execute"
            && b[ce..p].iter().all(|&c| c == b' ');
        let top = self.top();
        let sk = if top == F_SQLCALL || bare {
            self.ctx = sql::MAIN;
            K_SQL
        } else if top == F_RECALL && !triple {
            // the call's first string only
            self.set_top(F_PAREN);
            self.ctx = 0;
            K_REGEX_STR
        } else {
            return None;
        };
        self.callw = (0, usize::MAX);
        if triple {
            return Some(self.heredoc_open(p, p + 3, sk | HEREDOC | d));
        }
        self.put(p, p + 1, flavor(sk | d) + BEGIN);
        Some(self.str_body(p + 1, sk | d))
    }

    /// A heredoc's (or a heredoc sigil's) opening delimiter `p..s`: the text after it on the
    /// line bat's invalid, the body from the next line ([`M_STRING`]).
    #[inline(never)]
    pub(super) fn heredoc_open(&mut self, p: usize, s: usize, sk: u8) -> usize {
        let n = self.b.len();
        let text = flavor(sk);
        self.put(p, s, text + BEGIN);
        if s < n {
            self.put(s, n, text + BAD_OPEN);
        }
        self.mode = M_STRING;
        self.suspend(sk);
        n
    }

    /// A string suspended at a `#{` or a line's end: its kind kept on [`Scan::strs`].
    #[inline(always)]
    pub(super) fn suspend(&mut self, sk: u8) {
        self.strs = self.strs << 8 | u64::from(sk);
    }

    /// The innermost suspended string's kind, taken off [`Scan::strs`].
    #[inline(always)]
    pub(super) fn resume(&mut self) -> u8 {
        let sk = (self.strs & 0xff) as u8;
        self.strs >>= 8;
        sk
    }

    /// A string's rest at a line's start ([`M_STRING`]): a heredoc closed by a line whose first
    /// non-blank is its delimiter (Elixir's; bat's closes at one anywhere on a line).
    #[cold]
    pub(super) fn string_rest(&mut self) -> usize {
        let sk = self.resume();
        self.mode = M_CODE;
        if sk & HEREDOC != 0 {
            let b = self.b;
            let n = b.len();
            let mut q = 0;
            while q < n && matches!(b[q], b' ' | b'\t') {
                q += 1;
            }
            let c = CLOSER[usize::from(sk & 7)];
            if b.get(q..q + 3) == Some(&[c, c, c][..]) {
                let text = flavor(sk);
                if q > 0 {
                    self.put(0, q, text + TEXT);
                }
                self.put(q, q + 3, text + END);
                self.ctx = 0;
                if is_sigil(sk) {
                    return self.modifiers(q + 3);
                }
                return q + 3;
            }
        }
        self.str_body(0, sk)
    }

    /// A string's text from `p` of kind `sk`, its escapes and `#{…}`s, to its end, a `#{` (code
    /// to its `}`, [`F_INTERP`]) or the line's end (the string goes on, [`M_STRING`]).
    #[inline(always)]
    pub(super) fn str_body(&mut self, mut p: usize, sk: u8) -> usize {
        if !SOL
            && matches!(
                sk & KIND,
                K_REGEX | K_REGEX_RAW | K_SQL | K_REGEX_STR | K_HEEX
            )
        {
            return match sk & KIND {
                K_SQL => self.sql_text(p, sk),
                K_HEEX => self.heex_text(p, sk),
                _ => self.regex_text(p, sk),
            };
        }
        let b = self.b;
        let n = b.len();
        let text = flavor(sk);
        // a heredoc ends only at a line's start
        let closer = if sk & HEREDOC != 0 {
            b'\\'
        } else {
            CLOSER[usize::from(sk & 7)]
        };
        let interp = interpolates(sk);
        loop {
            let q = if interp {
                find_in(b, p, n, [closer, b'\\', b'#'])
            } else {
                find_in(b, p, n, [closer, b'\\'])
            };
            if q >= n {
                if n > p {
                    self.put_hot(p, n, text + TEXT);
                }
                self.mode = M_STRING;
                self.suspend(sk);
                return n;
            }
            if q > p {
                self.put_hot(p, q, text + TEXT);
            }
            match b[q] {
                b'\\' => {
                    if interp {
                        p = self.escape(q, text);
                    } else {
                        // a raw sigil's `\` takes the character after it (Elixir's: an escaped
                        // delimiter doesn't end it; bat's raw sigils end there)
                        let e = b
                            .get(q + 1)
                            .map_or(q + 1, |&d| (q + 1 + char_len(d)).min(n));
                        self.put(q, e, text + TEXT);
                        p = e;
                    }
                }
                b'#' => {
                    if b.get(q + 1) == Some(&b'{') {
                        self.put(q, q + 2, text + INTERP);
                        self.suspend(sk);
                        self.push(F_INTERP);
                        return q + 2;
                    }
                    self.put_hot(q, q + 1, text + TEXT);
                    p = q + 1;
                }
                _ => return self.str_close(q, sk),
            }
        }
    }

    /// A string's closing delimiter at `q`: a quoted key's with its `:`, a sigil's with its
    /// modifiers after.
    #[inline(always)]
    pub(super) fn str_close(&mut self, q: usize, sk: u8) -> usize {
        let text = flavor(sk);
        if sk & KIND == K_KEY {
            self.put(q, q + 2, text + END);
            return q + 2;
        }
        self.put_hot(q, q + 1, text + END);
        if is_sigil(sk) {
            return self.modifiers(q + 1);
        }
        q + 1
    }

    /// A sigil's modifiers from `p`: bat's `[a-zA-Z]+`, then `\w+` invalid.
    #[inline(never)]
    fn modifiers(&mut self, p: usize) -> usize {
        let b = self.b;
        let mut e = p;
        while e < b.len() && b[e].is_ascii_alphabetic() {
            e += 1;
        }
        self.put(p, e, MODIFIERS);
        let f = skip_word(b, e);
        self.put(e, f, BAD_MODIFIERS);
        f
    }

    /// A `\` at `q` in a string with escapes (`text` its flavor's first leaf): bat's
    /// `escaped_char`, `\x` with one or two hex digits (a character not a delimiter's taken as
    /// a bad one), `\u` with four or `{…}`, anything else one character.
    #[inline(never)]
    pub(super) fn escape(&mut self, q: usize, text: u16) -> usize {
        let b = self.b;
        let n = b.len();
        let Some(&c) = b.get(q + 1) else {
            // `\` at the line's end: bat's `\\.` doesn't take the line's end
            self.put(q, n, text + TEXT);
            return n;
        };
        let not_delim = |c: u8| !matches!(c, b'\'' | b'"' | b'/' | b')' | b']' | b'}' | b'>');
        match c {
            b'x' => {
                let mut e = q + 2;
                let mut parts = 0;
                while parts < 2 && e < n && not_delim(b[e]) {
                    e += char_len(b[e]).min(n - e);
                    parts += 1;
                }
                if parts == 0 {
                    self.put(q, q + 2, text + ESC_CHAR);
                    return q + 2;
                }
                self.put(q, q + 2, text + ESC_HEX);
                let mut i = q + 2;
                while i < e {
                    let k = char_len(b[i]).min(e - i);
                    let kind = if b[i].is_ascii_hexdigit() {
                        ESC_HEX
                    } else {
                        ESC_HEX_BAD
                    };
                    self.put(i, i + k, text + kind);
                    i += k;
                }
                e
            }
            b'u' => {
                // `\u\h{4}`, `\u{\h{1,6}}`
                if b.len() >= q + 6 && b[q + 2..q + 6].iter().all(u8::is_ascii_hexdigit) {
                    self.put(q, q + 6, text + ESC_UNI);
                    return q + 6;
                }
                if b.get(q + 2) == Some(&b'{') {
                    let mut e = q + 3;
                    while e < n && e < q + 9 && b[e].is_ascii_hexdigit() {
                        e += 1;
                    }
                    if e > q + 3 && b.get(e) == Some(&b'}') {
                        self.put(q, e + 1, text + ESC_UNI);
                        return e + 1;
                    }
                    if b.get(q + 3) == Some(&b'}') {
                        self.put(q, q + 4, text + ESC_UNI_BAD);
                        return q + 4;
                    }
                }
                // `\u{?[^'"/)\]}>]{1,6}}?`
                let mut e = q + 2;
                if b.get(e) == Some(&b'{') {
                    e += 1;
                }
                let mut k = 0;
                while k < 6 && e < n && not_delim(b[e]) {
                    e += char_len(b[e]).min(n - e);
                    k += 1;
                }
                if k == 0 {
                    self.put(q, q + 2, text + ESC_CHAR);
                    return q + 2;
                }
                if b.get(e) == Some(&b'}') {
                    e += 1;
                }
                self.put(q, e, text + ESC_UNI_BAD);
                e
            }
            _ => {
                let e = (q + 1 + char_len(c)).min(n);
                self.put(q, e, text + ESC_CHAR);
                e
            }
        }
    }

    /// The `}` at `p` that closes a string's `#{`: its string goes on, and a binary's segments
    /// after it.
    #[cold]
    #[inline(never)]
    pub(super) fn interp_close(&mut self, p: usize) -> usize {
        self.pop();
        self.put(p, p + 1, INTERP_END);
        let sk = self.resume();
        let sk = if sk == 0 { K_STR | D_DQ } else { sk };
        let r = self.str_body(p + 1, sk);
        if self.mode == M_CODE && matches!(self.top(), F_BINARY | F_BINTYPE) {
            return self.binary(r);
        }
        r
    }

    /// `~` and a letter at `p`: a sigil (Elixir's: a lowercase letter, or uppercase letters and
    /// digits), its delimiter, its text and its modifiers.
    #[inline(never)]
    pub(super) fn sigil(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let letter = b[p + 1];
        let mut e = p + 2;
        if !SOL && letter.is_ascii_uppercase() {
            // ada's: `~HTML"…"` (bat's sigils are one letter)
            while e < n && (b[e].is_ascii_uppercase() || b[e].is_ascii_digit()) {
                e += 1;
            }
        }
        let Some(&d) = b.get(e) else {
            self.put(p, e, SIGIL_BAD);
            return e;
        };
        let kind = match letter {
            b'r' => K_REGEX,
            b'R' => K_REGEX_RAW,
            b'a'..=b'z' => K_LOWER,
            b'H' if !SOL && e == p + 2 => {
                self.heex_open();
                K_HEEX
            }
            _ => K_UPPER,
        };
        let triple = |q: u8| b.get(e + 1) == Some(&q) && b.get(e + 2) == Some(&q);
        let dk = match d {
            b'"' if triple(b'"') => {
                self.put(p, e, SIGIL);
                return self.heredoc_open(e, e + 3, kind | HEREDOC | D_DQ);
            }
            b'\'' if triple(b'\'') => {
                self.put(p, e, SIGIL);
                return self.heredoc_open(e, e + 3, kind | HEREDOC | D_SQ);
            }
            b'"' => D_DQ,
            b'\'' => D_SQ,
            b'/' => D_SLASH,
            b'|' => D_PIPE,
            b'(' => 4,
            b'[' => 5,
            b'{' => 6,
            b'<' => 7,
            _ => {
                let k = char_len(d).min(n - e);
                self.put(p, e, SIGIL);
                self.put(e, e + k, SIGIL_BAD_DELIM);
                return e + k;
            }
        };
        let sk = kind | dk;
        self.put(p, e, SIGIL);
        self.put(e, e + 1, flavor(sk) + BEGIN);
        self.str_body(e + 1, sk)
    }

    /// `@doc ` (`@moduledoc`, `@typedoc`) at `p`, its text from `s`: bat's documentation
    /// (`"""`, `'''`, `~s"""`, `~S"""`, `"…"`, `false`), all of it a comment's color; `None`
    /// where it's an attribute.
    #[inline(never)]
    pub(super) fn doc(&mut self, p: usize, s: usize) -> Option<usize> {
        let b = self.b;
        let rest = &b[s..];
        let (sk, open) = match rest {
            [b'"', b'"', b'"', ..] => (K_DOC | HEREDOC | D_DQ, 3),
            [b'\'', b'\'', b'\'', ..] => (K_DOC | HEREDOC | D_SQ, 3),
            [b'~', l, q, q2, q3, ..]
                if l.is_ascii_alphabetic() && q == q2 && q2 == q3 && matches!(*q, b'"' | b'\'') =>
            {
                let k = if l.is_ascii_lowercase() {
                    K_DOC
                } else {
                    K_DOC_RAW
                };
                (k | HEREDOC | if *q == b'"' { D_DQ } else { D_SQ }, 5)
            }
            [b'f', b'a', b'l', b's', b'e', ..] => {
                self.put(p, s + 5, DOC_FALSE);
                return Some(s + 5);
            }
            [b'"', ..] => (K_DOC | D_DQ, 1),
            _ => return None,
        };
        // the rest of a heredoc's opening line is the documentation's (bat's)
        self.put(p, s + open, flavor(sk) + TEXT);
        Some(self.str_body(s + open, sk))
    }

    /// `#` at `p`: a comment to the line's end, the blanks before it with it where it starts
    /// the line (bat's `(?:^[ \t]+)?(#)`), `##` there a section's.
    #[inline(always)]
    pub(super) fn hash(&mut self, p: usize) -> usize {
        let n = self.b.len();
        if p == self.lead {
            if self.b.get(p + 1) == Some(&b'#') {
                if p > 0 {
                    self.put(0, p, SECTION);
                }
                self.put(p, p + 2, SECTION_MARK);
                if n > p + 2 {
                    self.put(p + 2, n, SECTION);
                }
                return n;
            }
            if p > 0 {
                self.put_hot(0, p, COMMENT);
            }
        }
        self.put_hot(p, p + 1, COMMENT_MARK);
        if n > p + 1 {
            self.put_hot(p + 1, n, COMMENT);
        }
        n
    }
}
