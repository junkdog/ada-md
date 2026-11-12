//! Python's strings: prefixes and quotes, escapes, printf, strftime and `str.format`
//! placeholders, f-strings' replacement fields and format specs, docstrings, and the SQL and
//! regexes in strings ([`crate::sql`], [`crate::regex`]'s Python dialect); which string is which
//! ([`Scan::decide`], [`re_valid`]).

use super::*;
use crate::prim::find_in;

/// Whether the quote at `p` is escaped: an odd run of `\` before it.
#[inline(always)]
fn escaped(b: &[u8], p: usize) -> bool {
    let mut q = p;
    while q > 0 && b[q - 1] == b'\\' {
        q -= 1;
    }
    (p - q) % 2 == 1
}

/// Whether a string of kind `sk` closes at `q`, a quote of its: one, or three for a triple.
#[inline(always)]
fn closes(b: &[u8], q: usize, sk: SK) -> bool {
    sk & Q_TRIPLE == 0 || (b.get(q + 1) == Some(&b[q]) && b.get(q + 2) == Some(&b[q]))
}

/// The quote byte of a string of kind `sk`.
#[inline(always)]
fn quote_of(sk: SK) -> u8 {
    if sk & 1 != 0 { b'"' } else { b'\'' }
}

/// Where Python's escape at `p` (a `\`, not before the line's end) ends in a cooked string;
/// `p` itself where Python has none there and keeps the `\` (`\d`).
pub(super) fn escape_end(b: &[u8], p: usize, bytes: bool) -> usize {
    let hex = |q: usize, k: usize| (q..q + k).all(|i| b.get(i).is_some_and(u8::is_ascii_hexdigit));
    let Some(&c) = b.get(p + 1) else {
        return p;
    };
    match c {
        b'\\' | b'\'' | b'"' | b'a' | b'b' | b'f' | b'n' | b'r' | b't' | b'v' => p + 2,
        b'x' if hex(p + 2, 2) => p + 4,
        b'0'..=b'7' => {
            let mut q = p + 2;
            while q < p + 4 && b.get(q).is_some_and(|c| matches!(c, b'0'..=b'7')) {
                q += 1;
            }
            q
        }
        b'u' if !bytes && hex(p + 2, 4) => p + 6,
        b'U' if !bytes && hex(p + 2, 8) => p + 10,
        // a character's name: letters, digits, spaces and `-` (so no search runs past the string's
        // quote)
        b'N' if !bytes && b.get(p + 2) == Some(&b'{') => {
            let mut q = p + 3;
            while q < b.len().min(p + 80)
                && (b[q].is_ascii_alphanumeric() || matches!(b[q], b' ' | b'-'))
            {
                q += 1;
            }
            if b.get(q) == Some(&b'}') { q + 1 } else { p }
        }
        _ => p,
    }
}

/// A regex's escape of the host's: Python's in a cooked string, none in a raw one.
fn re_escape_cooked(b: &[u8], p: usize) -> usize {
    escape_end(b, p, false)
}

fn re_escape_raw(_: &[u8], p: usize) -> usize {
    p
}

// [`regex::Embed::arg`] for a Python string: its quote, and these.
const ARG_TRIPLE: usize = 1 << 8;
const ARG_F: usize = 1 << 9;

/// A regex comment of a triple-quoted (verbose) pattern: `#` at a line's start or after a blank.
#[inline(always)]
fn comment_at(b: &[u8], p: usize) -> bool {
    b[p] == b'#' && (p == 0 || matches!(b[p - 1], b' ' | b'\t'))
}

/// Whether a pattern of the host's (but its escape) matches at `p` in a regex's string: its
/// closing quote, an f-string's `{` or `}`, a verbose comment.
fn re_stop(b: &[u8], p: usize, arg: usize) -> bool {
    let c = b[p];
    if c == arg as u8 {
        return !escaped(b, p)
            && (arg & ARG_TRIPLE == 0 || (b.get(p + 1) == Some(&c) && b.get(p + 2) == Some(&c)));
    }
    (arg & ARG_F != 0 && (c == b'{' || c == b'}')) || (arg & ARG_TRIPLE != 0 && comment_at(b, p))
}

/// The bytes a string's patterns start on, by the quote (`'`, `"`), whether it's cooked (`\`),
/// and whether `{`, `}`, `%` and `#` are its (an f-string's, a placeholder's, a verbose
/// pattern's comment): the line's end is always one.
static YIELDS: [[bool; 256]; 8] = {
    let mut t = [[false; 256]; 8];
    let mut i = 0;
    while i < 8 {
        t[i][if i & 1 != 0 { b'"' } else { b'\'' } as usize] = true;
        if i & 2 != 0 {
            t[i][b'\\' as usize] = true;
        }
        if i & 4 != 0 {
            t[i][b'{' as usize] = true;
            t[i][b'}' as usize] = true;
            t[i][b'%' as usize] = true;
            t[i][b'#' as usize] = true;
        }
        i += 1;
    }
    t
};

/// What a string's text is (plain, a docstring's, SQL, a regex) at a line's start or its open.
impl<K: Sink> Scan<'_, '_, K> {
    /// A string at `p0` (its prefix there, its quote at `q`).
    #[inline(always)]
    pub(super) fn string(&mut self, p0: usize, q: usize) -> usize {
        let b = self.b;
        let pre = if q > p0 {
            prefix(&b[p0..q]).unwrap_or(0)
        } else {
            0
        };
        let c = b[q];
        let triple = b.get(q + 1) == Some(&c) && b.get(q + 2) == Some(&c);
        let mut sk = u8::from(c == b'"') | if triple { Q_TRIPLE } else { 0 } | pre;
        let open = q + if triple { 3 } else { 1 };
        if !SOL {
            sk = with_content(sk, self.decide(p0, open, sk));
        }
        let t = text_scope(sk);
        if q > p0 {
            self.put(p0, q, STRING_PREFIX);
        }
        self.put_hot(q, open, sk_leaf(t, S_BEGIN));
        self.prev = P_VALUE;
        self.send = usize::MAX;
        self.sk = sk;
        self.instr = true;
        self.ctx = if content(sk) == C_SQL { sql::MAIN } else { 0 };
        self.text_from(open)
    }

    /// What the string opening at `p0` (its text from `open`), of kind `sk` so far, holds.
    #[inline(always)]
    fn decide(&mut self, p0: usize, open: usize, sk: SK) -> u8 {
        let b = self.b;
        let want = std::mem::replace(&mut self.want, WANT_NONE);
        if self.meta != 0 {
            // in a decorator, whose plain text is filled around the host's writes: no embeds
            return C_PLAIN;
        }
        if self.prev == P_START
            && self.depth == 0
            && sk & (FSTR | BYTES) == 0
            && b[..p0].iter().all(|&c| matches!(c, b' ' | b'\t' | 0x0c))
        {
            return C_DOC;
        }
        // a string right after a SQL or regex string: implicit concatenation, the same
        if self.str_c != 0
            && self.str_end <= p0
            && b[self.str_end..p0]
                .iter()
                .all(|&c| matches!(c, b' ' | b'\t'))
        {
            return self.str_c;
        }
        let top = self.top();
        if (top == F_REARG && self.prev == P_OPEN)
            || (want == WANT_RE
                && self.want_at <= p0
                && b[self.want_at..p0]
                    .iter()
                    .all(|&c| matches!(c, b' ' | b'\t')))
        {
            return C_RE;
        }
        if top == F_SQLARG && self.prev == P_OPEN {
            return match sql_start(b, open, true) {
                Some(true) => C_SQL,
                Some(false) => C_PLAIN,
                None if sk & Q_TRIPLE != 0 => C_DETECT_ANY,
                None => C_PLAIN,
            };
        }
        if sk & BYTES == 0
            && b.get(open)
                .is_none_or(|&c| c.is_ascii_uppercase() || c <= b' ')
        {
            match sql_start(b, open, false) {
                Some(true) => return C_SQL,
                None if sk & Q_TRIPLE != 0 => return C_DETECT_UP,
                _ => {}
            }
        }
        if sk & RAW != 0
            && let Some(e) = close_on_line(b, open, sk)
            && re_valid(&b[open..e], sk & FSTR != 0)
        {
            return C_RE;
        }
        C_PLAIN
    }

    /// An open string's rest at a line's start.
    #[cold]
    pub(super) fn string_rest(&mut self) -> usize {
        self.text_from(0)
    }

    /// The open string's text from `p` (of kind [`Scan::sk`]), by what it holds.
    #[inline(always)]
    pub(super) fn text_from(&mut self, p: usize) -> usize {
        match content(self.sk) {
            C_SQL => self.sql_text(p),
            C_RE => self.regex_text(p),
            C_DETECT_UP | C_DETECT_ANY => self.detect(p),
            _ => self.body(p),
        }
    }

    /// A string's close, `n` quotes at `q`, its text before it from `at` written.
    #[inline(always)]
    fn close(&mut self, at: usize, q: usize, n: usize) -> usize {
        let t = text_scope(self.sk);
        self.put_hot(at, q, sk_leaf(t, S_TEXT));
        self.put_hot(q, q + n, sk_leaf(t, S_END));
        self.instr = false;
        self.send = usize::MAX;
        self.str_end = q + n;
        self.str_c = match content(self.sk) {
            c @ (C_SQL | C_RE) => c,
            _ => 0,
        };
        q + n
    }

    /// The line ends in the open string: a triple-quoted one goes on, and a `\`-continued one; else
    /// it ends there (Python's error).
    #[inline(always)]
    fn line_end(&mut self, at: usize, cont: bool) -> usize {
        let n = self.b.len();
        let t = text_scope(self.sk);
        self.put_hot(at, n, sk_leaf(t, S_TEXT));
        self.instr = self.sk & Q_TRIPLE != 0 || cont;
        n
    }

    /// An undecided string's text from `p`: its first text decides it SQL or plain.
    #[cold]
    #[inline(never)]
    fn detect(&mut self, p: usize) -> usize {
        let any = content(self.sk) == C_DETECT_ANY;
        match sql_start(self.b, p, any) {
            Some(true) => {
                self.sk = with_content(self.sk, C_SQL);
                self.ctx = sql::MAIN;
                self.sql_text(p)
            }
            Some(false) => {
                self.sk = with_content(self.sk, C_PLAIN);
                self.body(p)
            }
            // a blank line: still undecided, unless it closes
            None => self.body(p),
        }
    }

    /// A plain string's, a docstring's or an f-string's text from `p`, to its close, a field's
    /// `{` (the field's code then), or the line's end.
    #[inline(always)]
    fn body(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let sk = self.sk;
        let t = text_scope(sk);
        let quote = quote_of(sk);
        let f = sk & FSTR != 0;
        // placeholders in a plain string's text (not an f-string's, a docstring's, a raw one's:
        // LaTeX's `^{2}`)
        let ph = !SOL && !f && content(sk) != C_DOC && sk & RAW == 0;
        let raw = sk & RAW != 0;
        let mut at = p;
        loop {
            let q = if f || ph {
                find_in(b, p, n, [quote, b'\\', b'{', b'}', b'%'])
            } else {
                find_in(b, p, n, [quote, b'\\'])
            };
            if q >= n {
                return self.line_end(at, false);
            }
            let c = b[q];
            if c == quote {
                if closes(b, q, sk) {
                    return self.close(at, q, if sk & Q_TRIPLE != 0 { 3 } else { 1 });
                }
                p = q + 1;
                continue;
            }
            match c {
                b'\\' => {
                    if q + 1 == n {
                        // the line goes on in the string
                        self.put_hot(at, q, sk_leaf(t, S_TEXT));
                        self.put(q, n, sk_leaf(t, S_CONT));
                        self.instr = true;
                        return n;
                    }
                    if raw {
                        // the `\` and the character after it are the text (a quote doesn't end it)
                        p = q + 1 + char_len(b[q + 1]);
                        continue;
                    }
                    self.put_hot(at, q, sk_leaf(t, S_TEXT));
                    let e = self.escape(q, t);
                    (p, at) = (e, e);
                }
                b'{' if b.get(q + 1) == Some(&b'{') => {
                    self.put_hot(at, q, sk_leaf(t, S_TEXT));
                    self.put(q, q + 2, sk_leaf(t, S_ESC));
                    (p, at) = (q + 2, q + 2);
                }
                b'}' if b.get(q + 1) == Some(&b'}') => {
                    self.put_hot(at, q, sk_leaf(t, S_TEXT));
                    self.put(q, q + 2, sk_leaf(t, S_ESC));
                    (p, at) = (q + 2, q + 2);
                }
                b'{' if f => {
                    self.put_hot(at, q, sk_leaf(t, S_TEXT));
                    return self.field_open(q);
                }
                b'{' if ph => match self.format_ph(q, t) {
                    Some(e) => {
                        self.put_hot(at, q, sk_leaf(t, S_TEXT));
                        self.write_format_ph(q, e, t);
                        (p, at) = (e, e);
                    }
                    None => p = q + 1,
                },
                b'%' if ph => match percent_ph(b, q) {
                    Some((e, name)) => {
                        self.put_hot(at, q, sk_leaf(t, S_TEXT));
                        self.write_percent(q, e, name, t);
                        (p, at) = (e, e);
                    }
                    None => p = q + 1,
                },
                _ => p = q + 1,
            }
        }
    }

    /// A cooked string's escape at `q` (a `\` before more of the line), its kind written in text
    /// scope `t`: where it ends.
    #[inline(never)]
    fn escape(&mut self, q: usize, t: u16) -> usize {
        let b = self.b;
        let bytes = self.sk & BYTES != 0;
        let e = escape_end(b, q, bytes);
        let (e, l) = if e > q {
            let l = match b[q + 1] {
                b'x' => S_ESC_HEX,
                b'0'..=b'7' => S_ESC_OCT,
                b'u' => S_ESC_U16,
                b'U' => S_ESC_U32,
                b'N' => S_ESC_NAME,
                _ => S_ESC,
            };
            (e, l)
        } else {
            ((q + 1 + char_len(b[q + 1])).min(b.len()), S_ESC_BAD)
        };
        self.put(q, e, sk_leaf(t, l));
        e
    }

    /// An f-string's field opens at `q`: its `{`, its frame; the code after it is the field's.
    #[inline(always)]
    fn field_open(&mut self, q: usize) -> usize {
        self.put(q, q + 1, INTERP_B);
        self.push(field(self.sk, false));
        self.instr = false;
        self.fctx = self.ctx;
        self.prev = P_OPEN;
        q + 1
    }

    /// `}` at `p` closes field frame `f`: its string goes on after it (or, for a field in a format
    /// spec, the outer field's spec).
    #[inline(never)]
    pub(super) fn field_close(&mut self, p: usize, f: u8) -> usize {
        self.put(p, p + 1, INTERP_E);
        self.pop();
        if f & 8 != 0 {
            let top = self.top();
            if top & F_FIELD != 0 {
                return self.spec_text(p + 1, top);
            }
            return p + 1;
        }
        self.sk = field_sk(f);
        self.instr = true;
        self.ctx = self.fctx;
        self.prev = P_VALUE;
        self.text_from(p + 1)
    }

    /// A field's `:` at `colon` (its frame `top`): the format spec from it.
    #[inline(never)]
    pub(super) fn spec(&mut self, _p: usize, top: u8, colon: usize) -> usize {
        self.spec_text(colon, top)
    }

    /// A field's format spec from `p` (its frame `top`): to a nested field's `{`, the field's `}`
    /// or the line's end.
    fn spec_text(&mut self, p: usize, top: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let q = find_in(b, p, n, *b"{}");
        self.put(p, q, FORMAT_SPEC);
        if q >= n {
            return n;
        }
        if b[q] == b'{' {
            self.put(q, q + 1, INTERP_B);
            self.push(top | 8);
            self.prev = P_OPEN;
            return q + 1;
        }
        self.field_close(q, top)
    }

    /// A `str.format` placeholder at `q` (`{`, not `{{`) in text scope `t`: where it ends, if the
    /// string's text has one there.
    #[inline(never)]
    fn format_ph(&self, q: usize, _t: u16) -> Option<usize> {
        let b = self.b;
        let quote = quote_of(self.sk);
        let n = b.len().min(q + LOOK);
        let mut r = q + 1;
        // the field's name: `[\w.\[\]]*`
        while r < n && (WORD[usize::from(b[r])] || matches!(b[r], b'.' | b'[' | b']')) {
            r += 1;
        }
        if r < n && b[r] == b'!' {
            if !matches!(b.get(r + 1), Some(b'a' | b'r' | b's')) {
                return None;
            }
            r += 2;
        }
        if r < n && b[r] == b':' {
            // a spec, one nested `{name}` in it at most a time
            let s0 = r + 1;
            r = s0;
            let mut nested = false;
            while r < n && b[r] != b'}' {
                match b[r] {
                    c if c == quote || c == b'\\' => return None,
                    b'{' => {
                        let mut s = r + 1;
                        while s < n && (WORD[usize::from(b[s])] || b[s] == b'.') {
                            s += 1;
                        }
                        if b.get(s) != Some(&b'}') {
                            return None;
                        }
                        nested = true;
                        r = s + 1;
                    }
                    _ => r += 1,
                }
            }
            if !nested && r <= n && !format_spec(&b[s0..r.min(n)]) {
                return None;
            }
        }
        (r < n && b[r] == b'}').then_some(r + 1)
    }

    /// [`Scan::format_ph`]'s placeholder `q..e` written: its braces, name, conversion and spec.
    #[inline(never)]
    fn write_format_ph(&mut self, q: usize, e: usize, t: u16) {
        let b = self.b;
        self.put(q, q + 1, sk_leaf(t, S_PH_B));
        let mut r = q + 1;
        while r < e - 1 && !matches!(b[r], b'!' | b':') {
            r += 1;
        }
        self.put(q + 1, r, sk_leaf(t, S_PH));
        if b[r] == b'!' {
            self.put(r, r + 2, sk_leaf(t, S_PH_CONV));
            r += 2;
        }
        if b[r] == b':' {
            let mut s = r;
            while s < e - 1 {
                if b[s] == b'{' {
                    self.put(r, s, sk_leaf(t, S_PH_SPEC));
                    let c = s + 1 + b[s + 1..e - 1].iter().position(|&c| c == b'}').unwrap_or(0);
                    self.put(s, s + 1, sk_leaf(t, S_PH_B));
                    self.put(s + 1, c, sk_leaf(t, S_PH));
                    self.put(c, c + 1, sk_leaf(t, S_PH_E));
                    s = c + 1;
                    r = s;
                } else {
                    s += 1;
                }
            }
            self.put(r, e - 1, sk_leaf(t, S_PH_SPEC));
        }
        self.put(e - 1, e, sk_leaf(t, S_PH_E));
    }

    /// A printf or strftime placeholder `q..e`, its mapping key's name `name`, in text scope `t`.
    #[inline(never)]
    fn write_percent(&mut self, q: usize, e: usize, name: Option<(usize, usize)>, t: u16) {
        match name {
            Some((s, c)) => {
                self.put(q, s, sk_leaf(t, S_PH));
                self.put(s, c, sk_leaf(t, S_PH_VAR));
                self.put(c, e, sk_leaf(t, S_PH));
            }
            None => self.put(q, e, sk_leaf(t, S_PH)),
        }
    }

    /// A regex's string text from `p` ([`crate::regex`], Python's dialect, in state
    /// [`Scan::ctx`]): the string's own patterns first (its end, an escape of Python's in a cooked
    /// one, an f-string's field and `{{`, a verbose pattern's comment), the regex's between.
    #[cold]
    #[inline(never)]
    fn regex_text(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let sk = self.sk;
        let t = text_scope(sk);
        let quote = quote_of(sk);
        let cooked = sk & RAW == 0;
        let f = sk & FSTR != 0;
        let triple = sk & Q_TRIPLE != 0;
        let embed = regex::Embed {
            dialect: regex::Dialect::Python,
            cooked,
            yields: &YIELDS[usize::from(sk & 1)
                | if cooked { 2 } else { 0 }
                | if f || triple { 4 } else { 0 }],
            stop: re_stop,
            arg: usize::from(quote)
                | if triple { ARG_TRIPLE } else { 0 }
                | if f { ARG_F } else { 0 },
            escape: if cooked {
                re_escape_cooked
            } else {
                re_escape_raw
            },
            top_only: false,
            continues: !triple,
            base: RE + t * 128,
            text: sk_leaf(t, S_TEXT),
        };
        let mut at = p;
        let mut q = p;
        while q < n {
            let c = b[q];
            if c == quote && !escaped(b, q) && closes(b, q, sk) {
                self.put(at, q, sk_leaf(t, S_TEXT));
                self.ctx = 0;
                return self.close(q, q, if triple { 3 } else { 1 });
            }
            if f && (c == b'{' || c == b'}') {
                if b.get(q + 1) == Some(&c) {
                    self.put(at, q, sk_leaf(t, S_TEXT));
                    self.put(q, q + 2, sk_leaf(t, S_ESC));
                    (q, at) = (q + 2, q + 2);
                    continue;
                }
                if c == b'{' {
                    self.put(at, q, sk_leaf(t, S_TEXT));
                    return self.field_open(q);
                }
            }
            if triple && comment_at(b, q) && self.ctx & regex::VERBATIM == 0 {
                // a verbose pattern's comment to the line's end
                self.put(at, q, sk_leaf(t, S_TEXT));
                self.put(q, q + 1, sk_leaf(t, S_RE_COMMENT_MARK));
                self.put(q + 1, n, sk_leaf(t, S_RE_COMMENT));
                self.instr = true;
                return n;
            }
            if c == b'\\' && q + 1 == n && !triple {
                self.put(at, q, sk_leaf(t, S_TEXT));
                self.put(q, n, sk_leaf(t, S_CONT));
                self.instr = true;
                return n;
            }
            if cooked && c == b'\\' && b.get(q + 1) != Some(&b'\\') {
                let e = escape_end(b, q, sk & BYTES != 0);
                if e > q {
                    self.put(at, q, sk_leaf(t, S_TEXT));
                    let e = self.escape(q, t);
                    (q, at) = (e, e);
                    continue;
                }
            }
            self.k.close(self.out);
            let r = regex::embedded(b, q, self.ctx, &embed, &mut at, self.out);
            self.k.reopen(self.out, 16);
            let Some((e, st)) = r else {
                break;
            };
            self.ctx = st;
            q = if e > q { e } else { q + char_len(c) };
        }
        self.put(at, n, sk_leaf(t, S_TEXT));
        self.instr = triple;
        if !triple {
            self.ctx = 0;
        }
        n
    }

    /// A SQL string's text from `p` ([`crate::sql`], in context [`Scan::ctx`]): the string's own
    /// patterns first at each place they can start (its end, an escape, a placeholder, an
    /// f-string's field), SQL's between, the line cut at the string's end so no SQL token runs
    /// past it; at the line's end a triple-quoted one goes on.
    #[cold]
    #[inline(never)]
    fn sql_text(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let sk = self.sk;
        let t = text_scope(sk);
        let cooked = sk & RAW == 0;
        let f = sk & FSTR != 0;
        let triple = sk & Q_TRIPLE != 0;
        let host = sql::Embed {
            yields: &YIELDS[usize::from(sk & 1) | if cooked { 2 } else { 0 } | 4],
            base: SQL + t * 128,
            text: sk_leaf(t, S_TEXT),
        };
        // the string's end, once a line: its turns between fields ask again
        let end = if self.send == usize::MAX {
            let e = string_end(b, p, sk);
            self.send = e;
            e
        } else {
            self.send
        };
        let line = &b[..end];
        self.sql.is_end = usize::MAX;
        let mut q = p;
        while q < end {
            let c = b[q];
            match c {
                b'\\' if q + 1 == n => {
                    self.put(q, n, sk_leaf(t, S_CONT));
                    self.instr = true;
                    return n;
                }
                b'\\' if cooked => {
                    let e = self.escape(q, t).min(end);
                    q = e;
                    continue;
                }
                b'\\' => {
                    let e = (q + 1 + char_len(b[q + 1])).min(end);
                    self.put(q, e, sk_leaf(t, S_TEXT));
                    q = e;
                    continue;
                }
                b'{' | b'}' if b.get(q + 1) == Some(&c) => {
                    self.put(q, q + 2, sk_leaf(t, S_ESC));
                    q += 2;
                    continue;
                }
                b'{' if f => return self.field_open(q),
                b'{' => {
                    if let Some(e) = self.format_ph(q, t).filter(|&e| e <= end) {
                        self.write_format_ph(q, e, t);
                        q = e;
                        continue;
                    }
                }
                b'%' => {
                    if let Some((e, name)) = percent_ph(line, q) {
                        self.write_percent(q, e, name, t);
                        q = e;
                        continue;
                    }
                }
                _ => {}
            }
            self.sql.at = q;
            self.k.close(self.out);
            let r = sql::embedded(line, q, self.ctx, &host, &mut self.sql, self.out);
            self.k.reopen(self.out, 16);
            let Some((e, ctx)) = r else {
                break;
            };
            self.ctx = ctx;
            let e = e.min(end);
            if self.sql.at < e {
                self.put(self.sql.at, e, sk_leaf(t, S_TEXT));
            }
            if e > q {
                q = e;
            } else {
                // no progress: the character is the string's text
                let e = (q + char_len(c)).min(end);
                self.put(q, e, sk_leaf(t, S_TEXT));
                q = e;
            }
        }
        if end < n {
            self.ctx = 0;
            return self.close(end, end, if triple { 3 } else { 1 });
        }
        if triple {
            // SQL's turn at the line's end, where its line comments end
            self.sql.at = n;
            self.k.close(self.out);
            let r = sql::embedded(b, n, self.ctx, &host, &mut self.sql, self.out);
            self.k.reopen(self.out, 16);
            if let Some((_, ctx)) = r {
                self.ctx = ctx;
            }
            if matches!(self.ctx, sql::DASHES | sql::HASHES) {
                self.ctx = sql::MAIN;
            }
            self.instr = true;
        } else {
            self.instr = false;
            self.ctx = 0;
        }
        n
    }
}

/// Where the string of kind `sk` whose text is at `p` closes on the line (its quote), past
/// escapes and an f-string's fields; the line's end if it doesn't.
fn string_end(b: &[u8], mut p: usize, sk: SK) -> usize {
    let quote = quote_of(sk);
    let f = sk & FSTR != 0;
    let mut depth = 0u32;
    while p < b.len() {
        let c = b[p];
        if c == b'\\' {
            p += 2;
            continue;
        }
        if f {
            if c == b'{' {
                if depth == 0 && b.get(p + 1) == Some(&b'{') {
                    p += 2;
                    continue;
                }
                depth += 1;
            } else if c == b'}' && depth > 0 {
                depth -= 1;
            }
        }
        if depth == 0 && c == quote && closes(b, p, sk) {
            return p;
        }
        p += 1;
    }
    b.len()
}

/// Where the string of kind `sk` whose text starts at `p` closes on the line, if it does.
fn close_on_line(b: &[u8], p: usize, sk: SK) -> Option<usize> {
    let e = string_end(b, p, sk);
    (e < b.len()).then_some(e)
}

/// Whether `s` is a format spec of Python's mini-language
/// (`[[fill]align][sign][z][#][0][width][grouping][.precision][type]`), or one with a `%` in it
/// (a date's, `{:%Y-%m-%d}`).
fn format_spec(s: &[u8]) -> bool {
    if s.contains(&b'%') {
        return true;
    }
    let mut i = 0;
    let align = |c: u8| matches!(c, b'<' | b'>' | b'=' | b'^');
    if s.len() >= 2 && align(s[1]) {
        i = 2;
    } else if s.first().is_some_and(|&c| align(c)) {
        i = 1;
    }
    if matches!(s.get(i), Some(b'+' | b'-' | b' ')) {
        i += 1;
    }
    if s.get(i) == Some(&b'z') {
        i += 1;
    }
    if s.get(i) == Some(&b'#') {
        i += 1;
    }
    while s.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    if matches!(s.get(i), Some(b',' | b'_')) {
        i += 1;
    }
    if s.get(i) == Some(&b'.') {
        i += 1;
        while s.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
    }
    if s.get(i).is_some_and(|c| b"bcdeEfFgGnosxX%".contains(c)) {
        i += 1;
    }
    i == s.len()
}

/// Whether the text at `p` starts with a SQL statement's keyword: bat's (`SELECT`, `INSERT`,
/// `UPDATE`, `DELETE`, `CREATE`, `REPLACE`, `ALTER`, `WITH`, uppercase) or, `any`, those and the
/// other statements' in any case. The keyword needs a blank after it or the line's end (a lone
/// word, `"DELETE"`, isn't SQL); `None` where the line is blank from `p`.
pub(super) fn sql_start(b: &[u8], p: usize, any: bool) -> Option<bool> {
    let s = skip_blanks(b, p);
    if s >= b.len() {
        return None;
    }
    let mut e = s;
    while e < b.len() && b[e].is_ascii_alphabetic() {
        e += 1;
    }
    if e == s || !(e == b.len() || matches!(b[e], b' ' | b'\t' | b'\r' | b'\\')) {
        return Some(false);
    }
    // `\` a cooked string's `\n` after the keyword
    if e < b.len() && b[e] == b'\\' && !matches!(b.get(e + 1), Some(b'n' | b't' | b'r')) {
        return Some(false);
    }
    let w = &b[s..e];
    const BAT: [&[u8]; 8] = [
        b"SELECT", b"INSERT", b"UPDATE", b"DELETE", b"CREATE", b"REPLACE", b"ALTER", b"WITH",
    ];
    const MORE: [&[u8]; 14] = [
        b"DROP",
        b"TRUNCATE",
        b"PRAGMA",
        b"BEGIN",
        b"COMMIT",
        b"ROLLBACK",
        b"EXPLAIN",
        b"GRANT",
        b"REVOKE",
        b"VACUUM",
        b"ANALYZE",
        b"SET",
        b"SHOW",
        b"MERGE",
    ];
    if !any {
        return Some(BAT.contains(&w));
    }
    Some(
        BAT.iter()
            .chain(MORE.iter())
            .any(|k| w.eq_ignore_ascii_case(k)),
    )
}

/// A printf placeholder at `q` (`%`): `%(name)` (a mapping key), flags, a width, a precision, a
/// length and a conversion; or strftime's (`%Y`, `%-d`). Where it ends and its key's name.
#[inline(never)]
pub(super) fn percent_ph(b: &[u8], q: usize) -> Option<(usize, Option<(usize, usize)>)> {
    let n = b.len();
    let mut r = q + 1;
    let mut name = None;
    if b.get(r) == Some(&b'(') {
        let s = r + 1;
        let mut c = s;
        while c < n && WORD[usize::from(b[c])] {
            c += 1;
        }
        if c == s || b.get(c) != Some(&b')') {
            return None;
        }
        name = Some((s, c));
        r = c + 1;
    }
    let flags = r;
    while r < n && matches!(b[r], b'#' | b'0' | b'-' | b' ' | b'+') && r < flags + 5 {
        r += 1;
    }
    if b.get(r) == Some(&b'*') {
        r += 1;
    } else {
        while r < n && b[r].is_ascii_digit() {
            r += 1;
        }
    }
    if b.get(r) == Some(&b'.') {
        r += 1;
        if b.get(r) == Some(&b'*') {
            r += 1;
        } else {
            while r < n && b[r].is_ascii_digit() {
                r += 1;
            }
        }
    }
    if matches!(b.get(r), Some(b'h' | b'l' | b'L')) {
        r += 1;
    }
    if b.get(r).is_some_and(|c| b"acdeEfFgGiorsuxX%".contains(c)) {
        return Some((r + 1, name));
    }
    if name.is_some() {
        return None;
    }
    // strftime's: `%Y`, `%-d`
    match (b.get(q + 1), b.get(q + 2)) {
        (Some(c), _) if b"aAwdbBGmyYHIpMSfzZjuUVWcxX%".contains(c) => Some((q + 2, None)),
        (Some(b'-'), Some(c)) if b"dmHIMSj".contains(c) => Some((q + 3, None)),
        _ => None,
    }
}

/// Whether `t`, a raw string's text, is a valid pattern of Python's `re` as far as its escapes,
/// groups, classes and quantifiers go: every `\` before a character `re` knows an escape of
/// (an ASCII letter it doesn't is an error, `\U` needs eight hex digits), groups balanced, a class
/// closed, a quantifier after something to repeat.
///
/// In an rf-string (`f`) a field (`{…}`) is a character, and `{{` and `}}` are a brace.
pub(super) fn re_valid(t: &[u8], f: bool) -> bool {
    let n = t.len();
    let hex = |q: usize, k: usize| (q..q + k).all(|i| t.get(i).is_some_and(u8::is_ascii_hexdigit));
    let mut depth = 0u32;
    let mut q = 0;
    // something to repeat before (not a line's start, `(`, `|`), and a quantifier just read
    let mut atom = false;
    let mut quant = false;
    let mut class: Option<usize> = None;
    while q < n {
        let c = t[q];
        if c == b'\\' {
            let Some(&d) = t.get(q + 1) else {
                return false;
            };
            let in_class = class.is_some();
            let mut e = q + 2;
            match d {
                b'x' if !hex(q + 2, 2) => return false,
                b'x' => e = q + 4,
                b'u' if !hex(q + 2, 4) => return false,
                b'u' => e = q + 6,
                b'U' if !hex(q + 2, 8) => return false,
                b'U' => e = q + 10,
                b'N' => {
                    if t.get(q + 2) != Some(&b'{') {
                        return false;
                    }
                    match t[q + 2..].iter().position(|&c| c == b'}') {
                        Some(c) => e = q + 2 + c + 1,
                        None => return false,
                    }
                }
                b'a' | b'b' | b'f' | b'n' | b'r' | b't' | b'v' | b'd' | b'D' | b's' | b'S'
                | b'w' | b'W' => {}
                b'A' | b'B' | b'Z' | b'z' if !in_class => {}
                b'0'..=b'9' => {}
                c if c.is_ascii_alphabetic() => return false,
                _ => {}
            }
            q = e;
            atom = true;
            quant = false;
            continue;
        }
        if let Some(s) = class {
            if c == b']' && q > s {
                class = None;
                atom = true;
                quant = false;
            }
            q += 1;
            continue;
        }
        match c {
            b'[' => {
                let mut s = q + 1;
                if t.get(s) == Some(&b'^') {
                    s += 1;
                }
                // a `]` first is a character
                class = Some(s);
            }
            b'(' => {
                depth += 1;
                atom = false;
                quant = false;
                q += 1;
                // `(?…`: its own syntax, taken as a group's
                if t.get(q) == Some(&b'?') {
                    q += 1;
                }
                continue;
            }
            b')' => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
                atom = true;
                quant = false;
            }
            b'|' => {
                atom = false;
                quant = false;
            }
            b'*' | b'+' | b'?' => {
                if quant {
                    // a lazy or possessive quantifier's second byte, once
                    if c == b'*' {
                        return false;
                    }
                    quant = false;
                    atom = false;
                } else if !atom {
                    return false;
                } else {
                    quant = true;
                    atom = false;
                }
                q += 1;
                continue;
            }
            b'{' if f => {
                if t.get(q + 1) == Some(&b'{') {
                    // a brace, or a repeat's `{{n}}`
                    q += 2;
                    atom = true;
                    quant = false;
                    continue;
                }
                // a field: its value is a character to the regex
                let mut d = 0u32;
                while q < n {
                    match t[q] {
                        b'{' => d += 1,
                        b'}' => {
                            d -= 1;
                            if d == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    q += 1;
                }
                atom = true;
                quant = false;
            }
            b'}' if f && t.get(q + 1) == Some(&b'}') => {
                q += 2;
                if !atom && !quant {
                    atom = true;
                }
                continue;
            }
            b'{' => {
                // a repeat where it reads as one, else a character
                let mut r = q + 1;
                while r < n && (t[r].is_ascii_digit() || t[r] == b',') {
                    r += 1;
                }
                if r > q + 1 && t.get(r) == Some(&b'}') {
                    if !atom && !quant {
                        return false;
                    }
                    quant = true;
                    atom = false;
                    q = r + 1;
                    continue;
                }
                atom = true;
                quant = false;
            }
            b'^' | b'$' => {
                atom = false;
                quant = false;
            }
            _ => {
                atom = true;
                quant = false;
            }
        }
        q += 1;
    }
    depth == 0 && class.is_none()
}
