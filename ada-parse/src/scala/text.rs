//! Scala's text: strings (`"…"`, `"""…"""`, interpolated `id"…"` and `id"""…"""`) with their
//! escapes, interpolations and formats, characters and symbols, comments, numbers, and the SQL
//! and regexes in strings ([`crate::sql`], [`crate::regex`]).

use super::*;
use crate::prim::find_in;

/// A string kind's flavor: bat's `string.quoted.*` scope for it ([`DQ`]…).
fn flavor(sk: u8) -> u16 {
    match sk {
        SK_DQ | SK_SQL => DQ,
        SK_IQ | SK_FQ | SK_TFQ | SK_SQLI => IQ,
        SK_RQ => RIQ,
        SK_TQ | SK_TSQL => TQ,
        _ => TIQ,
    }
}

/// Whether kind `sk` reads `$name` and `${…}`.
fn interpolates(sk: u8) -> bool {
    !matches!(sk, SK_DQ | SK_TQ | SK_SQL | SK_TSQL)
}

/// Whether kind `sk` reads `\` escapes (all of them; a `"""` string reads `\uXXXX` alone).
fn escapes(sk: u8) -> bool {
    matches!(sk, SK_DQ | SK_IQ | SK_FQ | SK_TFQ | SK_SQL | SK_SQLI)
}

/// Whether a `"""` string ends at `p`: the last three quotes of a run, as Scala reads them.
pub(super) fn triple_end(b: &[u8], p: usize) -> bool {
    b[p..].starts_with(b"\"\"\"") && b.get(p + 3) != Some(&b'"')
}

/// Where a `\` escape at `p` ends, bat's `\\[btnfr"'\\]|\\u[0-9a-fA-F]{4}|\\[0-7]{1,3}`; `p` for
/// none (a lone `\`).
fn escape_end(b: &[u8], p: usize) -> usize {
    match b.get(p + 1) {
        Some(b'b' | b't' | b'n' | b'f' | b'r' | b'"' | b'\'' | b'\\') => p + 2,
        Some(b'u') => {
            if b.get(p + 2..p + 6)
                .is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit))
            {
                p + 6
            } else {
                p
            }
        }
        Some(b'0'..=b'7') => {
            let mut e = p + 2;
            while e < p + 4 && b.get(e).is_some_and(|c| (b'0'..=b'7').contains(c)) {
                e += 1;
            }
            e
        }
        _ => p,
    }
}

/// Where `\uXXXX` at `p` ends, `p` for none: a `"""` string's one escape.
fn unicode_end(b: &[u8], p: usize) -> usize {
    if b.get(p + 1) == Some(&b'u')
        && b.get(p + 2..p + 6)
            .is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit))
    {
        p + 6
    } else {
        p
    }
}

/// Where an interpolation's name after `$` at `p` ends (`[A-Za-z_][A-Za-z0-9_]*`, letters past
/// ASCII); `p + 1` for none.
fn interpolation_end(b: &[u8], p: usize) -> usize {
    let s = p + 1;
    match b.get(s) {
        Some(&c) if c.is_ascii_alphabetic() || c == b'_' => {}
        Some(&c) if c >= 0x80 && is_word_starting(b, s) => {}
        _ => return s,
    }
    let mut e = s + 1;
    loop {
        while e < b.len() && (b[e].is_ascii_alphanumeric() || b[e] == b'_') {
            e += 1;
        }
        if e < b.len() && b[e] >= 0x80 && is_word(b, e) {
            e += char_len(b[e]);
            continue;
        }
        return e;
    }
}

/// Where a format at `p` (`%`) ends, by bat's patterns; `p` for none.
fn format_end(b: &[u8], p: usize) -> usize {
    let at = |i: usize| b.get(i).copied().unwrap_or(0);
    let c = at(p + 1);
    if matches!(c, b'%' | b'n') {
        return p + 2;
    }
    // %\-?#?[bBhHsS], %\-?[cC], %\-?[tT][…]?
    let mut q = p + 1;
    if at(q) == b'-' {
        q += 1;
    }
    let r = if at(q) == b'#' { q + 1 } else { q };
    if matches!(at(r), b'b' | b'B' | b'h' | b'H' | b's' | b'S') {
        return r + 1;
    }
    if matches!(at(q), b'c' | b'C') {
        return q + 1;
    }
    if matches!(at(q), b't' | b'T') {
        return if b"HIklMSLNpzZsQBbhAaCYyjmdeRTrDFc".contains(&at(q + 1)) {
            q + 2
        } else {
            q + 1
        };
    }
    // %[\+\-# 0\(,]*[\.0-9]*[feEgGaA], %[\+\-# 0\(,]*[doxX]
    let mut q = p + 1;
    while matches!(at(q), b'+' | b'-' | b'#' | b' ' | b'0' | b'(' | b',') {
        q += 1;
    }
    if matches!(at(q), b'd' | b'o' | b'x' | b'X') {
        return q + 1;
    }
    while matches!(at(q), b'.' | b'0'..=b'9') {
        q += 1;
    }
    if matches!(at(q), b'f' | b'e' | b'E' | b'g' | b'G' | b'a' | b'A') {
        return q + 1;
    }
    p
}

/// Where the string SQL is in ends, from `p` inside it: its quote (a `"""` one's last three of
/// a run), past escapes and interpolations, or the line's end.
fn string_end(b: &[u8], mut p: usize, sk: u8) -> usize {
    let triple = is_triple(sk);
    while p < b.len() {
        match b[p] {
            b'\\' if escapes(sk) => p += 2,
            b'"' if !triple || triple_end(b, p) => return p,
            _ => p += 1,
        }
    }
    b.len()
}

/// The bytes a string's patterns start on: `"…"`'s, `"""…"""`'s, and an interpolated one's.
static YIELDS: [[bool; 256]; 4] = {
    let mut t = [[false; 256]; 4];
    t[0][b'"' as usize] = true;
    t[0][b'\\' as usize] = true;
    t[1][b'"' as usize] = true;
    t[2][b'"' as usize] = true;
    t[2][b'\\' as usize] = true;
    t[2][b'$' as usize] = true;
    t[3][b'"' as usize] = true;
    t[3][b'$' as usize] = true;
    t
};

/// A regex in a `"…"` string, then in a `"""…"""` one.
static RE_EMBEDS: [regex::Embed<'static>; 2] = [
    regex::Embed {
        dialect: regex::Dialect::Java,
        cooked: true,
        yields: &YIELDS[0],
        stop: |b, p, _| b[p] == b'"',
        arg: 0,
        escape: regex::java_escape,
        top_only: false,
        continues: false,
        base: RE_DQ,
        text: DQ,
    },
    regex::Embed {
        dialect: regex::Dialect::Java,
        cooked: false,
        yields: &YIELDS[1],
        stop: |b, p, _| triple_end(b, p),
        arg: 0,
        escape: regex::java_escape,
        top_only: false,
        continues: false,
        base: RE_TQ,
        text: TQ,
    },
];

/// Whether the string whose quote opens at `p` is a regex: right after `Regex(` or
/// `Pattern.compile(`. One before `.r` is found at its end ([`Scan::relex`]).
fn regex_string(b: &[u8], p: usize) -> bool {
    let q = crate::trim_end(b, 0, p);
    if q == 0 || b[q - 1] != b'(' {
        return false;
    }
    let w = crate::trim_end(b, 0, q - 1);
    let name: &[u8] = match w.checked_sub(1).map(|i| b[i]) {
        Some(b'x') => b"Regex",
        Some(b'e') => b"Pattern.compile",
        _ => return false,
    };
    w >= name.len()
        && b[w - name.len()..w].iter().zip(name).all(|(x, y)| x == y)
        && (w == name.len() || !is_word(b, w - name.len() - 1))
}

impl<K: Sink> Scan<'_, '_, K> {
    /// A `"` at `p` after no name: a `"…"` or `"""…"""` string, its text SQL's or a regex's
    /// where the call or marker around it says so.
    #[inline(always)]
    pub(super) fn dquote(&mut self, p: usize) -> usize {
        let b = self.b;
        let triple = b.get(p + 1..p + 3) == Some(b"\"\"");
        let (n, sk) = if triple { (3, SK_TQ) } else { (1, SK_DQ) };
        self.put_hot(p, p + n, flavor(sk) + BEGIN);
        self.prev = P_VALUE;
        let s = p + n;
        if SOL || self.meta != 0 {
            return self.string_cold(s, sk);
        }
        if self.top() == F_SQLCALL || (p >= self.mark && self.marked_at(p)) {
            self.mark = usize::MAX;
            self.ctx = sql::MAIN;
            return self.sql_text(s, if triple { SK_TSQL } else { SK_SQL });
        }
        if regex_string(b, p) {
            self.ctx = 0;
            return self.regex_text(s, triple);
        }
        self.opened = Some((s, self.k.mark(self.out)));
        self.string(s, sk)
    }

    /// An interpolated string: the name `b[p..e]` (its interpolator) right before a `"` at `e`.
    #[inline(never)]
    pub(super) fn interpolated(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let triple = b.get(e + 1..e + 3) == Some(b"\"\"");
        let id = &b[p..e];
        let sk = match id {
            b"f" => {
                if triple {
                    SK_TFQ
                } else {
                    SK_FQ
                }
            }
            b"raw" if !triple => SK_RQ,
            b"sql" | b"sqlu" | b"fr" | b"fr0" if !SOL && self.meta == 0 => {
                if triple {
                    SK_TSQLI
                } else {
                    SK_SQLI
                }
            }
            _ => {
                if triple {
                    SK_TIQ
                } else {
                    SK_IQ
                }
            }
        };
        let f = flavor(sk);
        let n = if triple { 3 } else { 1 };
        self.put(p, e, f + FUNC);
        self.put(e, e + n, f + BEGIN);
        self.prev = P_VALUE;
        if matches!(sk, SK_SQLI | SK_TSQLI) {
            self.ctx = sql::MAIN;
            return self.sql_text(e + n, sk);
        }
        self.string_cold(e + n, sk)
    }

    /// Whether the SQL marker still holds at `p`; it's dropped if not.
    #[cold]
    fn marked_at(&mut self, p: usize) -> bool {
        let holds = sql::marker_holds(self.b, self.mark, p);
        if !holds {
            self.mark = usize::MAX;
        }
        holds
    }

    /// A string suspended at a `${` or a line's end: its kind kept on [`Scan::skinds`].
    #[inline(always)]
    fn suspend(&mut self, sk: u8) {
        self.skinds = self.skinds << 4 | u32::from(sk);
    }

    /// The kind of the string resumed after a `${`'s `}` or at a line's start.
    #[inline(always)]
    fn resume(&mut self) -> u8 {
        let sk = (self.skinds & 15) as u8;
        self.skinds >>= 4;
        sk
    }

    /// A `"""` string's rest at a line's start: by the mode, plain, SQL's or a regex's.
    #[cold]
    pub(super) fn triple_rest(&mut self) -> usize {
        match self.mode {
            M_TRIPLE_RE => self.regex_text(0, true),
            M_TRIPLE_SQL => {
                let sk = self.resume();
                self.sql_text(0, if sk == SK_TSQLI { SK_TSQLI } else { SK_TSQL })
            }
            _ => {
                let sk = self.resume();
                let sk = if is_triple(sk) && !matches!(sk, SK_TSQL | SK_TSQLI) {
                    sk
                } else {
                    SK_TQ
                };
                self.string_cold(0, sk)
            }
        }
    }

    /// [`Scan::string`] out of line, for the rarer places a string's text starts or goes on
    /// (an interpolated one, after a `${…}`, a `"""` one's next line): one copy for them.
    #[cold]
    #[inline(never)]
    pub(super) fn string_cold(&mut self, p: usize, sk: u8) -> usize {
        self.string(p, sk)
    }

    /// A string's text from `p` of kind `sk`, its escapes, interpolations and formats, to its
    /// end quote, a `${` (code to its `}`, [`F_TMPL`]) or the line's end: a `"…"` string ends
    /// there, a `"""…"""` one goes on ([`M_TRIPLE`]).
    #[inline(always)]
    pub(super) fn string(&mut self, mut p: usize, sk: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let text = flavor(sk);
        let triple = is_triple(sk);
        loop {
            let q = match sk {
                SK_DQ => find_in(b, p, n, *b"\"\\"),
                SK_TQ => find_in(b, p, n, *b"\"\\"),
                SK_IQ | SK_TFQ => find_in(b, p, n, *b"\"\\$%"),
                SK_FQ => find_in(b, p, n, *b"\"\\$%"),
                _ => find_in(b, p, n, *b"\"\\$"),
            };
            if q >= n {
                self.put_hot(p, n, text);
                if triple {
                    self.mode = M_TRIPLE;
                    self.suspend(sk);
                } else {
                    self.mode = M_CODE;
                }
                return n;
            }
            match b[q] {
                b'"' => {
                    if !triple || triple_end(b, q) {
                        self.put_hot(p, q, text);
                        return self.string_close(q, if triple { 3 } else { 1 }, sk);
                    }
                    self.put_hot(p, q + 1, text);
                    p = q + 1;
                }
                b'$' => {
                    self.put_hot(p, q, text);
                    match b.get(q + 1) {
                        Some(b'{') => {
                            self.put(q, q + 2, text + EXPR);
                            self.suspend(sk);
                            self.push(F_TMPL);
                            self.mode = M_CODE;
                            self.prev = P_START;
                            return q + 2;
                        }
                        Some(b'$') => {
                            self.put(q, q + 2, text + ESCAPE);
                            p = q + 2;
                        }
                        _ => {
                            let e = interpolation_end(b, q);
                            if e > q + 1 {
                                self.put(q, q + 1, text + VAR_MARK);
                                self.put(q + 1, e, text + VAR);
                            } else {
                                self.put(q, e, text);
                            }
                            p = e;
                        }
                    }
                }
                b'%' => {
                    let e = format_end(b, q);
                    if e > q && matches!(sk, SK_FQ | SK_TFQ) {
                        self.put_hot(p, q, text);
                        self.put(q, e, text + FORMAT);
                        p = e;
                    } else {
                        self.put_hot(p, q + 1, text);
                        p = q + 1;
                    }
                }
                _ => {
                    // `\`
                    let e = if escapes(sk) {
                        escape_end(b, q)
                    } else if triple {
                        unicode_end(b, q)
                    } else {
                        q
                    };
                    self.put_hot(p, q, text);
                    if e > q {
                        self.put(q, e, text + ESCAPE);
                        p = e;
                    } else if escapes(sk) {
                        self.put(q, q + 1, text + LONE);
                        p = q + 1;
                    } else {
                        self.put_hot(q, q + 1, text);
                        p = q + 1;
                    }
                }
            }
        }
    }

    /// A string's end quote, `n` bytes at `q`. Right before `.r`, the string opened on the line
    /// is a regex: its spans are taken back and it is lexed again as one.
    #[inline(always)]
    fn string_close(&mut self, q: usize, n: usize, sk: u8) -> usize {
        self.mode = M_CODE;
        let rest = &self.b[q + n..];
        if !SOL
            && rest.first() == Some(&b'.')
            && rest.get(1) == Some(&b'r')
            && !rest
                .get(2)
                .is_some_and(|&c| NAME[usize::from(c)] || c >= 0x80)
            && matches!(sk, SK_DQ | SK_TQ)
            && let Some((s, m)) = self.opened.take()
        {
            return self.relex(s, m);
        }
        self.put_hot(q, q + n, flavor(sk) + END);
        q + n
    }

    /// [`Scan::string_close`]'s regex: the string from `s`, its lexemes since `m`, again.
    #[cold]
    #[inline(never)]
    fn relex(&mut self, s: usize, m: Mark) -> usize {
        self.k.undo(self.out, m);
        let triple = s >= 3 && self.b[s - 3..s] == *b"\"\"\"";
        self.ctx = 0;
        self.regex_text(s, triple)
    }

    /// The `}` at `p` that closes a string's `${`: its string goes on.
    #[cold]
    #[inline(never)]
    pub(super) fn template_close(&mut self, p: usize) -> usize {
        self.pop();
        // an annotation opened inside the `${…}` ends with it
        self.annot_closed(p);
        let sk = self.resume();
        self.put(p, p + 1, flavor(sk) + EXPR);
        self.prev = P_VALUE;
        if matches!(sk, SK_SQLI | SK_TSQLI) {
            return self.sql_text(p + 1, sk);
        }
        self.string_cold(p + 1, sk)
    }

    /// A regex's string from `p` ([`crate::regex`], Java's dialect): the string's own patterns
    /// first (its end, an escape but `\\`, which is the regex's), the regex's between, to the
    /// string's end or the line's ([`M_TRIPLE_RE`] for a `"""` one).
    #[cold]
    #[inline(never)]
    fn regex_text(&mut self, p: usize, triple: bool) -> usize {
        let b = self.b;
        let n = b.len();
        let text = if triple { TQ } else { DQ };
        let embed = &RE_EMBEDS[usize::from(triple)];
        let mut at = p;
        let mut q = p;
        while q < n {
            let c = b[q];
            if c == b'"' && (!triple || triple_end(b, q)) {
                self.put(at, q, text);
                let k = if triple { 3 } else { 1 };
                self.put(q, q + k, text + END);
                self.mode = M_CODE;
                self.ctx = 0;
                return q + k;
            }
            if c == b'\\' && !triple && b.get(q + 1) != Some(&b'\\') {
                let e = escape_end(b, q);
                self.put(at, q, text);
                if e > q {
                    self.put(q, e, text + ESCAPE);
                    (q, at) = (e, e);
                } else {
                    self.put(q, q + 1, text + LONE);
                    (q, at) = (q + 1, q + 1);
                }
                continue;
            }
            self.k.close(self.out);
            let r = regex::embedded(b, q, self.ctx, embed, &mut at, self.out);
            self.k.reopen(self.out, 16);
            // the regex's spans went to the sink directly: an annotation's fill starts past them
            self.gap = self.gap.max(at);
            let Some((e, st)) = r else {
                break;
            };
            self.ctx = st;
            q = if e > q { e } else { q + char_len(c) };
        }
        self.put(at, n, text);
        if triple {
            self.mode = M_TRIPLE_RE;
        } else {
            self.mode = M_CODE;
            self.ctx = 0;
        }
        n
    }

    /// A SQL string's text from `p` of kind `sk` ([`crate::sql`], in context [`Scan::ctx`]): the
    /// string's own patterns first at each place they can start (its end, an escape, an
    /// interpolation), SQL's between, the line cut at the string's end so no SQL token runs past
    /// it; at the line's end a `"""` one goes on ([`M_TRIPLE_SQL`]).
    #[cold]
    #[inline(never)]
    fn sql_text(&mut self, p: usize, sk: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let triple = is_triple(sk);
        let interp = interpolates(sk);
        let text = flavor(sk);
        let base = match sk {
            SK_SQL => SQL_DQ,
            SK_TSQL => SQL_TQ,
            SK_SQLI => SQL_IQ,
            _ => SQL_TIQ,
        };
        let host = sql::Embed {
            yields: &YIELDS[usize::from(interp) * 2 + usize::from(triple)],
            base,
            text,
        };
        let end = string_end(b, p, sk);
        let line = &b[..end];
        self.sql.is_end = usize::MAX;
        let mut q = p;
        while q < end {
            let c = b[q];
            if c == b'$' && interp {
                match b.get(q + 1) {
                    Some(b'{') => {
                        self.put(q, q + 2, text + EXPR);
                        self.suspend(sk);
                        self.push(F_TMPL);
                        self.mode = M_CODE;
                        self.prev = P_START;
                        return q + 2;
                    }
                    Some(b'$') => {
                        self.put(q, q + 2, text + ESCAPE);
                        q += 2;
                        continue;
                    }
                    _ => {
                        let e = interpolation_end(b, q).min(end);
                        if e > q + 1 {
                            self.put(q, q + 1, text + VAR_MARK);
                            self.put(q + 1, e, text + VAR);
                        } else {
                            self.put(q, e, text);
                        }
                        q = e;
                        continue;
                    }
                }
            }
            if c == b'\\' && escapes(sk) {
                let e = escape_end(b, q).min(end);
                if e > q {
                    self.put(q, e, text + ESCAPE);
                    q = e;
                } else {
                    self.put(q, q + 1, text + LONE);
                    q += 1;
                }
                continue;
            }
            self.sql.at = q;
            self.k.close(self.out);
            let r = sql::embedded(line, q, self.ctx, &host, &mut self.sql, self.out);
            self.k.reopen(self.out, 16);
            // SQL's spans went to the sink directly: an annotation's fill starts past them
            self.gap = self.gap.max(self.sql.at);
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
        if end < n {
            let k = if triple { 3 } else { 1 };
            self.put(end, end + k, text + END);
            self.mode = M_CODE;
            self.ctx = 0;
            return end + k;
        }
        if triple {
            // SQL's turn at the line's end, where its line comments end
            self.sql.at = n;
            self.k.close(self.out);
            let r = sql::embedded(b, n, self.ctx, &host, &mut self.sql, self.out);
            self.k.reopen(self.out, 16);
            self.gap = self.gap.max(self.sql.at);
            if let Some((_, ctx)) = r {
                self.ctx = ctx;
            }
            if matches!(self.ctx, sql::DASHES | sql::HASHES) {
                self.ctx = sql::MAIN;
            }
            self.mode = M_TRIPLE_SQL;
            self.suspend(sk);
        } else {
            self.mode = M_CODE;
            self.ctx = 0;
        }
        n
    }

    /// A `(` right after a [`W_SQL`] word: a SQL call's ([`F_SQLCALL`]: Anorm's `SQL(`, the
    /// JVM's calls, Spark's `sql(`), else a parenthesis.
    #[cold]
    #[inline(never)]
    pub(super) fn sql_paren(&mut self) -> u8 {
        let b = self.b;
        let (s, w) = self.sqlw;
        let name = &b[s..w];
        if name == b"SQL" || sql::jvm_call(b, s, w) {
            F_SQLCALL
        } else {
            F_PAREN
        }
    }

    /// `'` at `p`: a character (`'a'`, `'\n'`), a symbol (`'sym`), or a quote.
    #[inline(never)]
    pub(super) fn squote(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        self.prev = P_VALUE;
        // a character: one escape or one character, then `'`
        let q = p + 1;
        let e = match b.get(q) {
            Some(b'\\') => {
                let e = escape_end(b, q);
                if e > q { e } else { usize::MAX }
            }
            Some(&c) if c != b'\n' => q + char_len(c),
            _ => usize::MAX,
        };
        if e < n && b[e] == b'\'' {
            self.put(p, e + 1, CHAR);
            return e + 1;
        }
        // a symbol: a name or an operator's run
        if let Some(&c) = b.get(q) {
            let e = if ACT[usize::from(c)] == A_NAME || (c >= 0x80 && is_word_starting(b, q)) {
                self.name_end(q)
            } else if OPCHAR[usize::from(c)] {
                let mut e = q;
                while e < n && OPCHAR[usize::from(b[e])] {
                    e += 1;
                }
                e
            } else {
                q
            };
            if e > q {
                self.put(p, e, SYMBOL);
                return e;
            }
        }
        self.prev = P_OP;
        q
    }

    /// `//` at `p`: a line comment to the line's end; `// language=SQL` marks the next string.
    #[inline(always)]
    pub(super) fn line_comment(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        self.put_hot(p, p + 2, LINE_MARK);
        self.put_hot(p + 2, n, LINE_COMMENT);
        if !SOL && sql::marker_comment(b, p + 2) {
            self.mark = n;
        }
        n
    }

    /// `/*` at `p`: a block comment, a documentation one at `/**`, `/**/` an empty one;
    /// `/* language=SQL */` marks the next string.
    #[inline(never)]
    pub(super) fn comment_open(&mut self, p: usize) -> usize {
        let b = self.b;
        if b[p..].starts_with(b"/**/") {
            self.put(p, p + 4, EMPTY_COMMENT);
            return p + 4;
        }
        if !SOL && sql::marker_comment(b, p + 2) {
            self.mark = b[p + 2..]
                .windows(2)
                .position(|w| w == b"*/")
                .map_or(b.len(), |i| p + 4 + i);
        }
        self.mode = M_COMMENT;
        if b.get(p + 2) == Some(&b'*') {
            self.put(p, p + 3, DOC_MARK);
            self.nest = 1 | NEST_DOC;
            return self.comment(p + 3);
        }
        self.put(p, p + 2, BLOCK_COMMENT);
        self.nest = 1;
        self.comment(p + 2)
    }

    /// A block comment's text from `p`, [`Scan::nest`] deep: Scala's comments nest. In a
    /// documentation comment, a line's leading `*`, `@tag`s and `{@link …}`. Where the code goes
    /// on, or the line's end ([`M_COMMENT`]).
    pub(super) fn comment(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let doc = self.nest & NEST_DOC != 0;
        if doc && p == 0 && self.nest & !NEST_DOC == 1 {
            // `^\s*(\*)(?!/)`
            let q = skip_spaces(b, 0);
            if b.get(q) == Some(&b'*') && b.get(q + 1) != Some(&b'/') {
                self.put(0, q, DOC);
                self.put(q, q + 1, DOC_MARK);
                p = q + 1;
            }
        }
        loop {
            let depth = self.nest & !NEST_DOC;
            let text = if !doc {
                BLOCK_COMMENT
            } else if depth == 1 {
                DOC
            } else {
                DOC_NESTED
            };
            let q = if doc && depth == 1 {
                find_in(b, p, n, *b"*/@{")
            } else {
                find_in(b, p, n, *b"*/")
            };
            if q >= n {
                self.put(p, n, text);
                return n;
            }
            match b[q] {
                b'*' if b.get(q + 1) == Some(&b'/') => {
                    self.put(p, q, text);
                    self.put(q, q + 2, if text == DOC { DOC_MARK } else { text });
                    let d = depth.saturating_sub(1);
                    self.nest = if d == 0 {
                        0
                    } else {
                        d | (self.nest & NEST_DOC)
                    };
                    if d == 0 {
                        self.mode = M_CODE;
                        return q + 2;
                    }
                    p = q + 2;
                }
                b'/' if b.get(q + 1) == Some(&b'*') => {
                    self.put(p, q, text);
                    let inner = if doc { DOC_NESTED } else { BLOCK_COMMENT };
                    self.put(q, q + 2, inner);
                    self.nest = depth.saturating_add(1).min(0x7f) | (self.nest & NEST_DOC);
                    p = q + 2;
                }
                b'@' => {
                    // `@\w+\s`
                    let mut e = q + 1;
                    while e < n && WORD_BYTE[usize::from(b[e])] {
                        e += 1;
                    }
                    // bat's `\s` after the tag matches the line's `\n` too
                    if e > q + 1 && (e == n || matches!(b[e], b' ' | b'\t' | b'\r' | 0x0b | 0x0c)) {
                        let t = (e + 1).min(n);
                        self.put(p, q, text);
                        self.put(q, t, DOC_TAG);
                        p = t;
                    } else {
                        self.put(p, q + 1, text);
                        p = q + 1;
                    }
                }
                b'{' => {
                    // `\{@link\s+[^\}]*\}`
                    let e = if b[q..].starts_with(b"{@link")
                        && b.get(q + 6).is_some_and(|&c| matches!(c, b' ' | b'\t'))
                    {
                        let c = find_in(b, q + 6, n, *b"}");
                        if c < n { c + 1 } else { q }
                    } else {
                        q
                    };
                    if e > q {
                        self.put(p, q, text);
                        self.put(q, e, DOC_LINK);
                        p = e;
                    } else {
                        self.put(p, q + 1, text);
                        p = q + 1;
                    }
                }
                _ => {
                    self.put(p, q + 1, text);
                    p = q + 1;
                }
            }
        }
    }

    /// A number at `p` (a digit, or `.` before one): bat's floats, hexadecimals and integers,
    /// with Scala's `_` between digits; the decimal point, the base's `0x` and a suffix their
    /// own leaves.
    pub(super) fn number_at(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let digits = |mut i: usize, ok: fn(u8) -> bool| {
            while i < n
                && (ok(b[i]) || (b[i] == b'_' && b.get(i + 1).is_some_and(|&c| ok(c) || c == b'_')))
            {
                i += 1;
            }
            i
        };
        let word_after = |i: usize| b.get(i).is_some_and(|&c| NAME[usize::from(c)] || c >= 0x80);
        if b[p] == b'0' && matches!(b.get(p + 1), Some(b'x' | b'X')) {
            let e = digits(p + 2, |c| c.is_ascii_hexdigit());
            if e > p + 2 {
                let s = if matches!(b.get(e), Some(b'l' | b'L')) {
                    e + 1
                } else {
                    e
                };
                if !word_after(s) {
                    self.put(p, p + 2, HEX_BASE);
                    self.put(p + 2, e, HEX);
                    if s > e {
                        self.put(e, s, HEX_SUFFIX);
                    }
                    return s;
                }
            }
        }
        // a float: `1.5`, `1e5`, `1.5e-3f`, `.5`, `2f`
        let int_end = if b[p] == b'.' {
            p
        } else {
            digits(p, |c| c.is_ascii_digit())
        };
        let mut e = int_end;
        let mut dot = None;
        let mut float = false;
        if b.get(e) == Some(&b'.') && b.get(e + 1).is_some_and(u8::is_ascii_digit) {
            dot = Some(e);
            e = digits(e + 1, |c| c.is_ascii_digit());
            float = true;
        }
        if matches!(b.get(e), Some(b'e' | b'E')) {
            let mut s = e + 1;
            if matches!(b.get(s), Some(b'+' | b'-')) {
                s += 1;
            }
            if b.get(s).is_some_and(u8::is_ascii_digit) {
                e = digits(s, |c| c.is_ascii_digit());
                float = true;
            }
        }
        let suffix = matches!(b.get(e), Some(b'd' | b'D' | b'f' | b'F'));
        if (float || suffix) && !word_after(e + usize::from(suffix)) {
            match dot {
                Some(d) => {
                    self.put(p, d, FLOAT);
                    self.put(d, d + 1, FLOAT_DOT);
                    self.put(d + 1, e, FLOAT);
                }
                None => self.put(p, e, FLOAT),
            }
            if suffix {
                self.put(e, e + 1, FLOAT_SUFFIX);
                return e + 1;
            }
            return e;
        }
        if b[p] == b'.' {
            self.put(p, p + 1, ACCESSOR);
            self.prev = P_DOT;
            return p + 1;
        }
        // an integer: `0`, `[1-9][0-9]*`, a suffix `L`
        let e = int_end;
        let s = if matches!(b.get(e), Some(b'l' | b'L')) {
            e + 1
        } else {
            e
        };
        if word_after(s) || (b[p] == b'0' && e > p + 1) {
            return word_end_after(b, s);
        }
        self.put_hot(p, e, INT);
        if s > e {
            self.put(e, s, INT_SUFFIX);
        }
        s
    }
}

/// ASCII's word bytes, `\w`.
static WORD_BYTE: [bool; 256] = crate::prim::WORD;

/// Where the name run on from a number at `s` ends (`1x`, `0123`): plain.
fn word_end_after(b: &[u8], s: usize) -> usize {
    let mut e = s;
    while e < b.len() && (NAME[usize::from(b[e])] || b[e] >= 0x80) {
        e += 1;
    }
    e
}
