//! Dart's text: strings (escapes, interpolations, a regex's or SQL's text), comments (nested
//! blocks, dartdoc's references and code), numbers.

use super::{
    COMMENT, DOC, DOC_CODE, DOC_REF, EMPTY_COMMENT, ESCAPE, F_INTERP, LINE_COMMENT, M_CODE,
    M_COMMENT, M_DOC, M_STR, NUMBER, P_OP, P_VALUE, PARAM, RE_BASE, SOL, SQL_BASE, Scan,
    skip_blanks, str_leaf,
};
use crate::emit::Sink;
use crate::prim::{WORD, char_len, find_in};
use crate::regex;
use crate::sql;

// A string's kind (`sk`): its quote, raw or not, its embed's.
/// `"` (else `'`).
pub(super) const K_DQ: u8 = 1;
/// Three quotes.
pub(super) const K_TRIPLE: u8 = 2;
/// `r'…'`: no escape, no interpolation.
pub(super) const K_RAW: u8 = 4;
/// SQL's text.
pub(super) const E_SQL: u8 = 8;
/// A regex's text.
pub(super) const E_RE: u8 = 16;

// What a SQL call's argument is ([`Scan::sqls`]).
/// Not SQL.
pub(super) const S_DONE: u8 = 0;
/// SQL: the first argument, its strings each SQL (`'SELECT …' 'FROM …'`, `'…' + '…'`).
pub(super) const S_SQL: u8 = 1;

/// The quote of a string of kind `sk`.
#[inline(always)]
const fn quote_of(sk: u8) -> u8 {
    if sk & K_DQ != 0 { b'"' } else { b'\'' }
}

/// The interpolation's frame of a string of kind `sk`: its quote's bits and its embed's.
#[inline(always)]
const fn interp_frame(sk: u8) -> u8 {
    F_INTERP + (sk & 3) + ((sk >> 3) & 3) * 4
}

/// The kind of the string an interpolation's frame `f` resumes.
#[inline(always)]
pub(super) const fn frame_kind(f: u8) -> u8 {
    let v = f - F_INTERP;
    (v & 3) | ((v >> 2) & 3) << 3
}

/// Whether the string of kind `sk` closes at `q` (its quote there): its quote's length.
#[inline(always)]
fn closes(b: &[u8], q: usize, sk: u8) -> Option<usize> {
    if sk & K_TRIPLE == 0 {
        return Some(1);
    }
    let c = b[q];
    (b.get(q + 1) == Some(&c) && b.get(q + 2) == Some(&c)).then_some(3)
}

/// Whether `c` is a hex digit.
#[inline(always)]
const fn is_hex(c: u8) -> bool {
    c.is_ascii_hexdigit()
}

/// Where Dart's escape at `q` (a `\`) ends: `\xHH`, `\uHHHH`, `\u{H…}` whole, else the `\` and
/// the character after it (any other is that character); a malformed `\x` or `\u` its two
/// bytes.
pub(super) fn escape_end(b: &[u8], q: usize) -> usize {
    let n = b.len();
    let Some(&c) = b.get(q + 1) else {
        return q + 1;
    };
    let hex = |i: usize| b.get(i).is_some_and(|&c| is_hex(c));
    match c {
        b'x' if hex(q + 2) && hex(q + 3) => q + 4,
        b'u' if b.get(q + 2) == Some(&b'{') => {
            let mut e = q + 3;
            while e < n && e < q + 9 && is_hex(b[e]) {
                e += 1;
            }
            if e > q + 3 && b.get(e) == Some(&b'}') {
                e + 1
            } else {
                q + 2
            }
        }
        b'u' if (2..6).all(|i| hex(q + i)) => q + 6,
        _ => q + 1 + char_len(c).min(n - q - 1),
    }
}

/// Whether the `$` at `q` starts an interpolation: `${`, or a name (`[A-Za-z_]`, no `$`).
#[inline(always)]
fn interp_starts(b: &[u8], q: usize) -> bool {
    b.get(q + 1)
        .is_some_and(|&c| c == b'{' || c.is_ascii_alphabetic() || c == b'_')
}

/// Where an interpolated name from `p` ends: `[A-Za-z0-9_]`, Dart's identifier without `$`.
#[inline(always)]
fn interp_name_end(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && WORD[usize::from(b[p])] {
        p += 1;
    }
    p
}

/// Where the text of a string of kind `sk` from `p` ends on the line, past its escapes and
/// `$name`s: its closing quote's place (`true`), an interpolation's `${` (`false`), or the
/// line's end (`false`). Each piece between interpolations is read once.
fn string_end(b: &[u8], mut p: usize, sk: u8) -> (usize, bool) {
    let n = b.len();
    let qc = quote_of(sk);
    loop {
        let q = if sk & K_RAW != 0 {
            find_in(b, p, n, [qc])
        } else {
            find_in(b, p, n, [qc, b'\\', b'$'])
        };
        if q >= n {
            return (n, false);
        }
        match b[q] {
            b'\\' => p = escape_end(b, q),
            b'$' if b.get(q + 1) == Some(&b'{') => return (q, false),
            b'$' => p = q + 1,
            _ if closes(b, q, sk).is_some() => return (q, true),
            _ => p = q + 1,
        }
    }
}

/// The bytes a string's own patterns start on, by its quote and rawness: its quote, and a
/// cooked one's `\` and `$`.
static YIELDS: [[bool; 256]; 4] = {
    let mut t = [[false; 256]; 4];
    t[0][b'\'' as usize] = true;
    t[0][b'\\' as usize] = true;
    t[0][b'$' as usize] = true;
    t[1][b'"' as usize] = true;
    t[1][b'\\' as usize] = true;
    t[1][b'$' as usize] = true;
    t[2][b'\'' as usize] = true;
    t[3][b'"' as usize] = true;
    t
};

/// [`YIELDS`]' table for a string of kind `sk`.
#[inline(always)]
fn yields_of(sk: u8) -> &'static [bool; 256] {
    &YIELDS[usize::from((sk & K_DQ) | ((sk >> 1) & 2))]
}

/// Whether a regex's string of kind `arg` has a pattern of its own at `p` (a byte of its
/// [`YIELDS`]): its closing quote(s), a cooked one's interpolation.
fn re_stop(b: &[u8], p: usize, arg: usize) -> bool {
    let sk = arg as u8;
    let c = b[p];
    if c == quote_of(sk) {
        return closes(b, p, sk).is_some();
    }
    c == b'$' && sk & K_RAW == 0 && interp_starts(b, p)
}

macro_rules! re_embeds {
    ($($sk:literal),*) => {
        [$(regex::Embed {
            dialect: regex::Dialect::Ecma,
            cooked: $sk & K_RAW == 0,
            yields: &YIELDS[(($sk & K_DQ) | (($sk >> 1) & 2)) as usize],
            stop: re_stop,
            arg: $sk,
            escape: if $sk & K_RAW == 0 { escape_end } else { |_, p| p },
            top_only: false,
            continues: false,
            base: RE_BASE + $sk as u16 * regex::KINDS as u16,
            text: str_leaf($sk),
        }),*]
    };
}

/// A regex in a string of each kind (`sk & 7`).
static RE_EMBEDS: [regex::Embed<'static>; 8] = re_embeds!(0, 1, 2, 3, 4, 5, 6, 7);

/// Where a number at `p` (a digit, or the `.` before one) ends: `0x` hex, or decimal digits, a
/// fraction and an exponent, `_` between digits (Dart 3.6).
fn number_end(b: &[u8], p: usize) -> usize {
    let n = b.len();
    let digits = |from: usize, ok: fn(u8) -> bool| {
        let mut e = from;
        while e < n && (ok(b[e]) || b[e] == b'_') {
            e += 1;
        }
        while e > from && b[e - 1] == b'_' {
            e -= 1;
        }
        e
    };
    let digit = |i: usize| b.get(i).is_some_and(u8::is_ascii_digit);
    if b[p] == b'0'
        && matches!(b.get(p + 1), Some(b'x' | b'X'))
        && b.get(p + 2).is_some_and(|&c| is_hex(c))
    {
        return digits(p + 2, is_hex);
    }
    let mut e = digits(p, |c| c.is_ascii_digit());
    if b.get(e) == Some(&b'.') && digit(e + 1) {
        e = digits(e + 1, |c| c.is_ascii_digit());
    }
    if matches!(b.get(e), Some(b'e' | b'E')) {
        let mut q = e + 1;
        if matches!(b.get(q), Some(b'+' | b'-')) {
            q += 1;
        }
        if digit(q) {
            e = digits(q, |c| c.is_ascii_digit());
        }
    }
    e.max(p + 1)
}

impl<K: Sink> Scan<'_, '_, K> {
    /// A number at `p`.
    #[inline(always)]
    pub(super) fn number(&mut self, p: usize) -> usize {
        self.prev = P_VALUE;
        let e = number_end(self.b, p);
        self.put_hot(p, e, NUMBER);
        e
    }

    /// A string whose prefix starts at `s` (its `r`, or its quote) and whose quote is at `q`:
    /// its kind from its quotes, SQL's or a regex's by the call it's an argument of.
    #[inline(always)]
    pub(super) fn string(&mut self, s: usize, q: usize) -> usize {
        let b = self.b;
        let qc = b[q];
        let mut sk = if qc == b'"' { K_DQ } else { 0 };
        if s < q {
            sk |= K_RAW;
        }
        let mut e = q + 1;
        if b.get(q + 1) == Some(&qc) && b.get(q + 2) == Some(&qc) {
            sk |= K_TRIPLE;
            e = q + 3;
        }
        self.prev = P_VALUE;
        if !SOL && self.top() >= super::F_SQLCALL {
            sk |= self.embed_here();
        }
        self.put_hot(s, e, str_leaf(sk));
        self.body(sk, e)
    }

    /// The embed of a string opened where the scan stands: SQL in a SQL call's argument, a
    /// regex in `RegExp(`'s first; its context starts afresh.
    #[cold]
    fn embed_here(&mut self) -> u8 {
        match self.top() {
            super::F_SQLCALL if self.sqls == S_SQL => {
                self.ctx = sql::MAIN;
                E_SQL
            }
            super::F_RECALL => {
                self.ctx = 0;
                E_RE
            }
            _ => 0,
        }
    }

    /// A string's text of kind `sk` from `p`: to its close, an interpolation's `${` (the code
    /// loop goes on in it), or the line's end.
    #[inline(always)]
    pub(super) fn body(&mut self, sk: u8, p: usize) -> usize {
        if sk & (E_SQL | E_RE) != 0 {
            return self.embedded_body(sk, p);
        }
        if sk & K_RAW != 0 {
            self.raw_body(sk, p)
        } else {
            self.cooked_body(sk, p)
        }
    }

    /// A triple-quoted string's rest at a line's start ([`M_STR`]).
    #[cold]
    pub(super) fn string_rest(&mut self) -> usize {
        let sk = self.aux;
        self.mode = M_CODE;
        self.body(sk, 0)
    }

    /// A string of kind `sk` that the line ends in: a triple-quoted one goes on
    /// ([`M_STR`]), a `'…'` or `"…"` one ends there (Dart's, an error; bat runs it on).
    #[inline(always)]
    fn open_at_end(&mut self, sk: u8) -> usize {
        if sk & K_TRIPLE != 0 {
            self.mode = M_STR;
            self.aux = sk;
        } else {
            self.ctx = 0;
        }
        self.b.len()
    }

    /// An interpolation's `${` written up to `e`: its frame pushed, the code loop goes on.
    fn open_interp(&mut self, sk: u8, at: usize, e: usize) -> usize {
        self.put(at, e, str_leaf(sk));
        self.push(interp_frame(sk));
        self.inter = self.inter.saturating_add(1);
        self.ipar = str_leaf(sk) + PARAM;
        self.prev = P_OP;
        e
    }

    /// An interpolation's `}` at `p` (frame `f`, still open): its string goes on.
    #[cold]
    #[inline(never)]
    pub(super) fn interp_close(&mut self, p: usize, f: u8) -> usize {
        self.pop();
        self.inter = self.inter.saturating_sub(1);
        self.ipar = if self.inter != 0 { self.find_ipar() } else { 0 };
        let sk = frame_kind(f);
        self.put(p, p + 1, str_leaf(sk));
        self.prev = P_VALUE;
        self.body(sk, p + 1)
    }

    /// A cooked string's text from `p`: escapes, interpolations, to its quote(s).
    #[inline(always)]
    fn cooked_body(&mut self, sk: u8, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let text = str_leaf(sk);
        let qc = quote_of(sk);
        let mut at = p;
        let mut from = p;
        loop {
            let q = find_in(b, from, n, [qc, b'\\', b'$']);
            if q >= n {
                self.put_hot(at, n, text);
                return self.open_at_end(sk);
            }
            match b[q] {
                b'\\' => {
                    let e = escape_end(b, q);
                    self.put_hot(at, q, text);
                    self.put(q, e, text + ESCAPE);
                    (at, from) = (e, e);
                }
                b'$' => match b.get(q + 1) {
                    Some(b'{') => return self.open_interp(sk, at, q + 2),
                    Some(&c) if c.is_ascii_alphabetic() || c == b'_' => {
                        let e = interp_name_end(b, q + 2);
                        self.put_hot(at, q + 1, text);
                        self.put(q + 1, e, text + PARAM);
                        (at, from) = (e, e);
                    }
                    _ => from = q + 1,
                },
                _ => match closes(b, q, sk) {
                    Some(k) => {
                        self.put_hot(at, q + k, text);
                        return q + k;
                    }
                    None => from = q + 1,
                },
            }
        }
    }

    /// A raw string's text from `p` to its quote(s).
    #[inline(always)]
    fn raw_body(&mut self, sk: u8, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let text = str_leaf(sk);
        let qc = quote_of(sk);
        let mut from = p;
        loop {
            let q = find_in(b, from, n, [qc]);
            if q >= n {
                self.put_hot(p, n, text);
                return self.open_at_end(sk);
            }
            if let Some(k) = closes(b, q, sk) {
                self.put_hot(p, q + k, text);
                return q + k;
            }
            from = q + 1;
        }
    }

    /// A SQL or regex string's text from `p`.
    #[cold]
    #[inline(never)]
    fn embedded_body(&mut self, sk: u8, p: usize) -> usize {
        if sk & E_SQL != 0 {
            self.sql_text(sk, p)
        } else {
            self.regex_text(sk, p)
        }
    }

    /// A string's own token at `q` in a SQL or regex string of kind `sk` (an escape, an
    /// interpolation) written from `at`: where the text goes on, or `Err` where the code loop
    /// does (an interpolation's `${`); `None` where none is there.
    fn host_token(&mut self, sk: u8, at: usize, q: usize) -> Option<Result<usize, usize>> {
        let b = self.b;
        if sk & K_RAW != 0 {
            return None;
        }
        let text = str_leaf(sk);
        match b[q] {
            b'\\' if b.get(q + 1) != Some(&b'\\') || sk & E_SQL != 0 => {
                let e = escape_end(b, q);
                self.put(at, q, text);
                self.put(q, e, text + ESCAPE);
                Some(Ok(e))
            }
            b'$' if interp_starts(b, q) => {
                if b[q + 1] == b'{' {
                    return Some(Err(self.open_interp(sk, at, q + 2)));
                }
                let e = interp_name_end(b, q + 2);
                self.put(at, q + 1, text);
                self.put(q + 1, e, text + PARAM);
                Some(Ok(e))
            }
            _ => None,
        }
    }

    /// A regex's string from `p` ([`crate::regex`], ECMAScript's dialect): the string's own
    /// patterns first (its end, an escape but `\\`, which is the regex's, an interpolation), the
    /// regex's between, to the string's end or the line's.
    fn regex_text(&mut self, sk: u8, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let text = str_leaf(sk);
        let qc = quote_of(sk);
        let embed = &RE_EMBEDS[usize::from(sk & 7)];
        let mut at = p;
        let mut q = p;
        while q < n {
            let c = b[q];
            if c == qc
                && let Some(k) = closes(b, q, sk)
            {
                self.put(at, q + k, text);
                self.ctx = 0;
                return q + k;
            }
            if matches!(c, b'\\' | b'$') {
                match self.host_token(sk, at, q) {
                    Some(Ok(e)) => {
                        (q, at) = (e, e);
                        continue;
                    }
                    Some(Err(e)) => return e,
                    None => {}
                }
            }
            self.k.close(self.out);
            let r = regex::embedded(b, q, self.ctx, embed, &mut at, self.out);
            self.k.reopen(self.out, 16);
            let Some((e, st)) = r else {
                break;
            };
            self.ctx = st;
            q = if e > q { e } else { q + char_len(c) };
        }
        self.put(at, n, text);
        self.open_at_end(sk)
    }

    /// A SQL string's text from `p` ([`crate::sql`], in context [`Scan::ctx`]): the string's own
    /// patterns first at each place they can start (its end, an escape, an interpolation), SQL's
    /// between, the line cut at the string's end so no SQL token runs past it; at the line's end
    /// a triple-quoted one goes on.
    fn sql_text(&mut self, sk: u8, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let text = str_leaf(sk);
        let host = sql::Embed {
            yields: yields_of(sk),
            base: SQL_BASE + u16::from(sk & 7) * sql::KINDS as u16,
            text,
        };
        let (end, closed) = string_end(b, p, sk);
        let line = &b[..end];
        self.sql.is_end = usize::MAX;
        let mut q = p;
        while q < end {
            let c = b[q];
            if matches!(c, b'\\' | b'$') {
                match self.host_token(sk, q, q) {
                    Some(Ok(e)) => {
                        q = e.min(end).max(q + 1);
                        continue;
                    }
                    Some(Err(e)) => return e,
                    None => {}
                }
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
                self.put(self.sql.at, e, text);
            }
            if e > q {
                q = e;
            } else {
                // no progress: the character is the string's text
                let e = (q + char_len(c)).min(end);
                self.put(q, e, text);
                q = e;
            }
        }
        if closed {
            let k = closes(b, end, sk).unwrap_or(1);
            self.put(end, end + k, text);
            self.ctx = 0;
            return end + k;
        }
        if end < n {
            // an interpolation's `${`
            return self.open_interp(sk, end, end + 2);
        }
        if sk & K_TRIPLE != 0 {
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
        }
        self.open_at_end(sk)
    }

    /// `//` at `p`: a line comment, or `///`'s documentation.
    #[inline(always)]
    pub(super) fn line_comment(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        if b.get(p + 2) == Some(&b'/') {
            if SOL {
                self.put(p, n, DOC);
            } else {
                self.doc_line(p);
            }
            return n;
        }
        self.put(p, n, LINE_COMMENT);
        n
    }

    /// A `///` line from `p`: dartdoc's references and code in its text, its fenced code's lines
    /// ([`Scan::fence`]) code whole.
    #[cold]
    #[inline(never)]
    fn doc_line(&mut self, p: usize) {
        let b = self.b;
        let n = b.len();
        let t = skip_blanks(b, p + 3);
        if b[t..].starts_with(b"```") {
            self.fence = !self.fence;
            self.put(p, n, DOC);
            return;
        }
        if self.fence {
            self.put(p, p + 3, DOC);
            self.put(p + 3, n, DOC_CODE);
            return;
        }
        self.doc_text(p, p + 3, n);
    }

    /// A line's fenced code goes on only on a `///` line: any other ends it.
    #[cold]
    pub(super) fn fence_line(&mut self) {
        let b = self.b;
        let t = skip_blanks(b, 0);
        if !b[t..].starts_with(b"///") {
            self.fence = false;
        }
    }

    /// Documentation's text from `from` to `end` (written from `at`): `[…]` a reference and
    /// `` `…` `` code, each closed on the line, as dartdoc reads them.
    fn doc_text(&mut self, mut at: usize, from: usize, end: usize) {
        let b = self.b;
        let mut p = from;
        // a kind whose close the line lacks past here: its openers are text (no search again)
        let (mut no_ref, mut no_code) = (false, false);
        loop {
            let q = match (no_ref, no_code) {
                (false, false) => find_in(b, p, end, *b"[`"),
                (false, true) => find_in(b, p, end, *b"["),
                (true, false) => find_in(b, p, end, *b"`"),
                (true, true) => end,
            };
            if q >= end {
                break;
            }
            let (close, kind) = if b[q] == b'[' {
                (b']', DOC_REF)
            } else {
                (b'`', DOC_CODE)
            };
            let r = find_in(b, q + 1, end, [close]);
            if r >= end {
                if close == b']' {
                    no_ref = true;
                } else {
                    no_code = true;
                }
                p = q + 1;
                continue;
            }
            self.put(at, q, DOC);
            self.put(q, r + 1, kind);
            at = r + 1;
            p = r + 1;
        }
        self.put(at, end, DOC);
    }

    /// `/*` at `p`: a block comment, `/** … */` documentation, or `/**/`.
    #[inline(always)]
    pub(super) fn comment_open(&mut self, p: usize) -> usize {
        let b = self.b;
        if b.get(p + 2) == Some(&b'*') {
            if b.get(p + 3) == Some(&b'/') {
                self.put(p, p + 4, EMPTY_COMMENT);
                return p + 4;
            }
            self.mode = M_DOC;
            self.aux = 1;
            self.fence = false;
            self.put(p, p + 3, DOC);
            return self.comment(p + 3);
        }
        self.mode = M_COMMENT;
        self.aux = 1;
        self.put(p, p + 2, COMMENT);
        self.comment(p + 2)
    }

    /// A block comment's text from `p` ([`M_COMMENT`], [`M_DOC`], [`Scan::aux`] its depth): to
    /// the `*/` closing it (a `/*` in it nests, as Dart reads it), or the line's end.
    pub(super) fn comment(&mut self, mut p: usize) -> usize {
        if self.mode == M_DOC && !SOL {
            return self.doc_block(p);
        }
        let b = self.b;
        let n = b.len();
        let kind = if self.mode == M_DOC { DOC } else { COMMENT };
        let at = p;
        loop {
            let q = find_in(b, p, n, *b"*/");
            if q + 1 >= n {
                break;
            }
            if let Some(e) = self.comment_mark(q) {
                p = e;
                if self.mode == M_CODE {
                    self.put(at, e, kind);
                    return e;
                }
            } else {
                p = q + 1;
            }
        }
        self.put(at, n, kind);
        n
    }

    /// A `*/` or `/*` at `q` in a block comment: the depth less or more one (the mode code when
    /// it's closed); where it ends, or `None` for neither.
    #[inline(always)]
    fn comment_mark(&mut self, q: usize) -> Option<usize> {
        let b = self.b;
        match (b[q], b.get(q + 1)) {
            (b'*', Some(b'/')) => {
                self.aux = self.aux.saturating_sub(1);
                if self.aux == 0 {
                    self.mode = M_CODE;
                    self.fence = false;
                }
                Some(q + 2)
            }
            (b'/', Some(b'*')) => {
                self.aux = self.aux.saturating_add(1);
                Some(q + 2)
            }
            _ => None,
        }
    }

    /// A `/** … */` comment's text from `p`, as bat's dartdoc reads it: `[…]` references,
    /// `` `…` `` code, a ```` ``` ```` fence's lines code (to its fence or the comment's end; bat's
    /// runs past the comment), a line's code after `*` and five spaces, a line indented four
    /// spaces a reference.
    #[cold]
    #[inline(never)]
    fn doc_block(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut at = p;
        if p == 0 && !self.fence {
            // bat's `^ {4,}(?![ \*]).*`
            let t = b.iter().take_while(|&&c| c == b' ').count();
            if t >= 4 && t < n && b[t] != b'*' {
                let e = self.doc_end(t);
                self.put(0, e, DOC_REF);
                at = e;
                p = e;
            }
        }
        // the line's next `*/` past the last `[` or `` ` `` that asked (`usize::MAX`: not looked
        // for yet), and whether a `]` or a `` ` `` is missing before it: each read once a stretch
        let mut lim = usize::MAX;
        let (mut no_ref, mut no_code) = (false, false);
        while p < n && self.mode == M_DOC {
            if self.fence {
                // the fence's code, to its closing fence or the comment's end
                let q = find_in(b, p, n, *b"`*");
                if q >= n {
                    self.put(at, n, DOC_CODE);
                    return n;
                }
                if b[q..].starts_with(b"```") {
                    self.put(at, q, DOC_CODE);
                    self.put(q, q + 3, DOC);
                    self.fence = false;
                    (at, p) = (q + 3, q + 3);
                } else if b.get(q + 1) == Some(&b'/') && b[q] == b'*' {
                    self.put(at, q, DOC_CODE);
                    let e = self.comment_mark(q).unwrap_or(q + 2);
                    self.put(q, e, DOC);
                    (at, p) = (e, e);
                } else {
                    p = q + 1;
                }
                continue;
            }
            let q = find_in(b, p, n, *b"*/[`");
            if q >= n {
                break;
            }
            match b[q] {
                b'*' | b'/' => {
                    if let Some(e) = self.comment_mark(q) {
                        p = e;
                    } else if b[q] == b'*' && b[q + 1..].starts_with(b"     ") {
                        // bat's `\* ((    ).*)`: code to the line's (or the comment's) end
                        let e = self.doc_end(q + 2);
                        self.put(at, q + 2, DOC);
                        self.put(q + 2, e, DOC_CODE);
                        (at, p) = (e, e);
                    } else {
                        p = q + 1;
                    }
                }
                b'`' if b[q..].starts_with(b"```") => {
                    // a fence's opener: its line's rest the comment's
                    let e = self.doc_end(q);
                    self.put(at, e, DOC);
                    self.fence = true;
                    (at, p) = (e, e);
                    if e < n {
                        // the comment closes on the opener's line
                        self.fence = false;
                    }
                }
                c => {
                    if lim == usize::MAX || q >= lim {
                        lim = self.doc_end(q + 1);
                        (no_ref, no_code) = (false, false);
                    }
                    let close = if c == b'[' { b']' } else { b'`' };
                    if (c == b'[' && no_ref) || (c == b'`' && no_code) {
                        p = q + 1;
                        continue;
                    }
                    let r = find_in(b, q + 1, lim, [close]);
                    if r >= lim {
                        if c == b'[' {
                            no_ref = true;
                        } else {
                            no_code = true;
                        }
                        p = q + 1;
                        continue;
                    }
                    self.put(at, q, DOC);
                    self.put(q, r + 1, if c == b'[' { DOC_REF } else { DOC_CODE });
                    (at, p) = (r + 1, r + 1);
                }
            }
        }
        let e = if self.mode == M_DOC { n } else { p };
        self.put(at, e, DOC);
        e
    }

    /// Where a documentation comment's text from `p` ends on the line: its next `*/`, or the
    /// line's end.
    fn doc_end(&self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut q = p;
        loop {
            q = find_in(b, q, n, *b"*");
            if q + 1 >= n {
                return n;
            }
            if b[q + 1] == b'/' {
                return q;
            }
            q += 1;
        }
    }
}
