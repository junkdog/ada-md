//! Groovy's text: strings (`'…'`, `"…"`, `'''…'''`, `"""…"""`, slashy `/…/`, dollar-slashy
//! `$/…/$`) with their escapes, `$name`s and `${…}`s, and comments.

use super::*;
use crate::prim::find_in;

/// Whether `sk`'s strings go on past a line's end.
#[inline(always)]
fn spans_lines(sk: u8) -> bool {
    matches!(base(sk), SK_TDQ | SK_TSQ | SK_SLASHY | SK_DOLLAR)
}

/// Whether `sk`'s strings have `$name` and `${…}`.
#[inline(always)]
fn interpolates(sk: u8) -> bool {
    !matches!(base(sk), SK_SQ | SK_TSQ)
}

/// Where the string of kind `sk` that `p` is in ends on the line: its end mark, past escapes,
/// `$name`s and `${…}`s (as [`Scan::string`] reads them), or the line's end.
fn string_end(b: &[u8], mut p: usize, sk: u8) -> usize {
    let n = b.len();
    let f = base(sk);
    while p < n {
        match b[p] {
            b'\\' => {
                p = match f {
                    SK_SLASHY => {
                        if b.get(p + 1) == Some(&b'/') {
                            p + 2
                        } else {
                            p + 1
                        }
                    }
                    SK_DOLLAR => p + 1,
                    // the one reader of an escape's extent ([`Scan::backslash`]'s): a bad `\u`
                    // takes four characters, a quote among them
                    _ => escape(b, p).0,
                }
            }
            b'$' if interpolates(sk) => match b.get(p + 1) {
                Some(b'{') => p = template_end(b, p).unwrap_or(n),
                Some(b'$' | b'/') if f == SK_DOLLAR => p += 2,
                _ => p += 1,
            },
            b'"' if matches!(f, SK_DQ | SK_TDQ) => {
                if f == SK_DQ || b[p..].starts_with(b"\"\"\"") {
                    return p;
                }
                p += 1;
            }
            b'\'' if matches!(f, SK_SQ | SK_TSQ) => {
                if f == SK_SQ || b[p..].starts_with(b"\'\'\'") {
                    return p;
                }
                p += 1;
            }
            b'/' if f == SK_SLASHY => return p,
            b'/' if f == SK_DOLLAR && b.get(p + 1) == Some(&b'$') => return p,
            _ => p += 1,
        }
    }
    n
}

/// Where a `${` at `p` closes on the line (past its `}`: braces balanced, strings inside
/// skipped); `None` where it doesn't.
fn template_end(b: &[u8], p: usize) -> Option<usize> {
    let mut depth = 0u32;
    let mut i = p + 1;
    while i < b.len() {
        match b[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            q @ (b'"' | b'\'') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The host's escape at `p` (a `\`) in a regex's string: a quoted string's Groovy escapes but
/// `\\` (the regex's), a slashy string's `\/` and `\u`; `p` where the `\` is the regex's.
fn re_escape(b: &[u8], p: usize, sk: u8) -> usize {
    match (base(sk), b.get(p + 1)) {
        (SK_SLASHY, Some(b'/')) => p + 2,
        (_, Some(b'u')) => {
            let (e, ok) = unicode(b, p);
            if ok { e } else { p }
        }
        (SK_DQ | SK_SQ, Some(b'n' | b'r' | b't' | b'b' | b'f' | b'$' | b'\'' | b'"')) => p + 2,
        _ => p,
    }
}

/// Whether a `$` at `p` starts a `$name` or a `${`.
fn dollar_takes(b: &[u8], p: usize) -> bool {
    b.get(p + 1) == Some(&b'{') || ident_end(b, p + 1).is_some()
}

/// The bytes a regex string's own patterns start on, by flavor: `"…"`'s, `'…'`'s, slashy's,
/// dollar-slashy's.
static RE_YIELDS: [[bool; 256]; 4] = {
    let mut t = [[false; 256]; 4];
    t[0][b'"' as usize] = true;
    t[0][b'$' as usize] = true;
    t[0][b'\\' as usize] = true;
    t[1][b'\'' as usize] = true;
    t[1][b'\\' as usize] = true;
    t[2][b'/' as usize] = true;
    t[2][b'$' as usize] = true;
    t[2][b'\\' as usize] = true;
    t[3][b'/' as usize] = true;
    t[3][b'$' as usize] = true;
    t[3][b'\\' as usize] = true;
    t
};

/// A regex in a `"…"`, a `'…'`, a slashy and a dollar-slashy string.
static RE_EMBEDS: [regex::Embed<'static>; 4] = [
    regex::Embed {
        dialect: regex::Dialect::Java,
        cooked: true,
        yields: &RE_YIELDS[0],
        stop: |b, p, _| b[p] == b'"' || (b[p] == b'$' && dollar_takes(b, p)),
        arg: 0,
        escape: |b, p| re_escape(b, p, SK_DQ),
        top_only: false,
        continues: false,
        base: RE_BASE,
        text: STR_BASE,
    },
    regex::Embed {
        dialect: regex::Dialect::Java,
        cooked: true,
        yields: &RE_YIELDS[1],
        stop: |b, p, _| b[p] == b'\'',
        arg: 0,
        escape: |b, p| re_escape(b, p, SK_SQ),
        top_only: false,
        continues: false,
        base: RE_BASE + 128,
        text: STR_BASE + STR_LEAVES,
    },
    regex::Embed {
        dialect: regex::Dialect::Java,
        cooked: false,
        yields: &RE_YIELDS[2],
        stop: |b, p, _| b[p] == b'/' || (b[p] == b'$' && dollar_takes(b, p)),
        arg: 0,
        escape: |b, p| re_escape(b, p, SK_SLASHY),
        top_only: false,
        continues: false,
        base: RE_BASE + 4 * 128,
        text: STR_BASE + 4 * STR_LEAVES,
    },
    regex::Embed {
        dialect: regex::Dialect::Java,
        cooked: false,
        yields: &RE_YIELDS[3],
        stop: |b, p, _| {
            (b[p] == b'/' && b.get(p + 1) == Some(&b'$'))
                || (b[p] == b'$'
                    && (dollar_takes(b, p) || matches!(b.get(p + 1), Some(b'$' | b'/'))))
        },
        arg: 0,
        escape: |b, p| re_escape(b, p, SK_DOLLAR),
        top_only: false,
        continues: false,
        base: RE_BASE + 5 * 128,
        text: STR_BASE + 5 * STR_LEAVES,
    },
];

/// The bytes a SQL string's own patterns start on, by flavor: `"…"`, `'…'`, `"""…"""`, `'''…'''`.
static SQL_YIELDS: [[bool; 256]; 4] = {
    let mut t = [[false; 256]; 4];
    t[0][b'"' as usize] = true;
    t[0][b'$' as usize] = true;
    t[0][b'\\' as usize] = true;
    t[1][b'\'' as usize] = true;
    t[1][b'\\' as usize] = true;
    t[2][b'"' as usize] = true;
    t[2][b'$' as usize] = true;
    t[2][b'\\' as usize] = true;
    t[3][b'\'' as usize] = true;
    t[3][b'\\' as usize] = true;
    t
};

/// Whether a character that may start a name (bat's `[a-zA-Z_]` and its Unicode range) is at
/// `p`, and where the name that starts there ends (bat's identifier: `\w` after the first).
fn ident_end(b: &[u8], p: usize) -> Option<usize> {
    let c = *b.get(p)?;
    if c.is_ascii_alphabetic() || c == b'_' || (c >= 0x80 && is_word_starting(b, p)) {
        Some(word_end(b, p))
    } else {
        None
    }
}

/// Where a `\` escape at `p` ends and whether it's one Groovy has, in a string that has
/// escapes: `\n`, `\$`, `\\`… (bat's `\\[nrtbf$\\'"]`), `\uXXXX`; `\u` before anything else and
/// any other `\x` a bad one; `\` at the line's end continues the string (bat's escaped
/// end-of-line).
fn escape(b: &[u8], p: usize) -> (usize, bool) {
    match b.get(p + 1) {
        None => (p + 1, true),
        Some(b'n' | b'r' | b't' | b'b' | b'f' | b'$' | b'\\' | b'\'' | b'"') => (p + 2, true),
        Some(b'u') => unicode(b, p),
        Some(&c) => (p + 1 + char_len(c).min(b.len() - p - 1), false),
    }
}

/// `\u` at `p`: four hex digits an escape, else bat's `\\u(?!\h{4}).{4}` a bad one.
fn unicode(b: &[u8], p: usize) -> (usize, bool) {
    if b.len() >= p + 6 && b[p + 2..p + 6].iter().all(u8::is_ascii_hexdigit) {
        return (p + 6, true);
    }
    // four characters after `\u`, if the line has them
    let mut e = p + 2;
    for _ in 0..4 {
        if e >= b.len() {
            return (p + 2, false);
        }
        e += char_len(b[e]);
    }
    (e.min(b.len()), false)
}

impl<K: Sink> Scan<'_, '_, K> {
    /// A `"` at `p`: a `"…"` or `"""…"""` string.
    #[inline(always)]
    pub(super) fn dquote(&mut self, p: usize) -> usize {
        let triple = self.b.get(p + 1..p + 3) == Some(b"\"\"");
        let (n, sk) = if triple { (3, SK_TDQ) } else { (1, SK_DQ) };
        self.put_hot(p, p + n, flavor(sk) + BEGIN);
        if !SOL && (self.callw.1 != usize::MAX || self.prev == P_REGEX || self.top() == F_SQLCALL) {
            return self.embedded_string(p, p + n, sk);
        }
        self.prev = P_VALUE;
        self.string(p + n, sk)
    }

    /// A string opened at `p` (its text from `s`) of flavor `sk` where a call or an operator
    /// may make it SQL or a regex.
    #[cold]
    #[inline(never)]
    fn embedded_string(&mut self, p: usize, s: usize, sk: u8) -> usize {
        let b = self.b;
        let regex = self.prev == P_REGEX;
        self.prev = P_VALUE;
        let q = crate::trim_end(b, 0, p);
        let (cs, ce) = self.callw;
        let sql = self.top() == F_SQLCALL
            || (ce != usize::MAX && q == ce && q < p && sql_call(b, cs, ce));
        let re = regex
            || (ce != usize::MAX
                && q > 0
                && b[q - 1] == b'('
                && crate::trim_end(b, 0, q - 1) == ce
                && regex_call(b, cs, ce));
        let v = match (sk, sql, re) {
            (SK_DQ, true, _) => SK_DQ_SQL,
            (SK_SQ, true, _) => SK_SQ_SQL,
            (SK_TDQ, true, _) => SK_TDQ_SQL,
            (SK_TSQ, true, _) => SK_TSQ_SQL,
            (SK_DQ, _, true) => SK_DQ_RE,
            (SK_SQ, _, true) => SK_SQ_RE,
            (SK_SLASHY, _, true) => SK_SLASHY_RE,
            (SK_DOLLAR, _, true) => SK_DOLLAR_RE,
            _ => sk,
        };
        self.ctx = if sql { sql::MAIN } else { 0 };
        self.string_any(s, v)
    }

    /// A string's text from `p` of kind `sk`, plain, SQL's or a regex's.
    #[inline(always)]
    pub(super) fn string_any(&mut self, p: usize, sk: u8) -> usize {
        match sk {
            SK_DQ_SQL..=SK_TSQ_SQL => self.sql_text(p, sk),
            SK_DQ_RE..=SK_DOLLAR_RE => self.regex_text(p, sk),
            _ => self.string(p, sk),
        }
    }

    /// A `'` at `p`: a `'…'` or `'''…'''` string.
    #[inline(always)]
    pub(super) fn squote(&mut self, p: usize) -> usize {
        let triple = self.b.get(p + 1..p + 3) == Some(b"''");
        let (n, sk) = if triple { (3, SK_TSQ) } else { (1, SK_SQ) };
        self.put_hot(p, p + n, flavor(sk) + BEGIN);
        if !SOL && (self.callw.1 != usize::MAX || self.prev == P_REGEX || self.top() == F_SQLCALL) {
            return self.embedded_string(p, p + n, sk);
        }
        self.prev = P_VALUE;
        self.string(p + n, sk)
    }

    /// A `/` at `p` where an operand starts: a slashy string.
    #[inline(never)]
    pub(super) fn slashy(&mut self, p: usize) -> usize {
        self.put(p, p + 1, flavor(SK_SLASHY) + BEGIN);
        if !SOL && (self.callw.1 != usize::MAX || self.prev == P_REGEX) {
            return self.embedded_string(p, p + 1, SK_SLASHY);
        }
        self.prev = P_VALUE;
        self.string(p + 1, SK_SLASHY)
    }

    /// `$/` at `p`: a dollar-slashy string.
    #[inline(never)]
    pub(super) fn dollar_slashy(&mut self, p: usize) -> usize {
        self.put(p, p + 2, flavor(SK_DOLLAR) + BEGIN);
        if !SOL && (self.callw.1 != usize::MAX || self.prev == P_REGEX) {
            return self.embedded_string(p, p + 2, SK_DOLLAR);
        }
        self.prev = P_VALUE;
        self.string(p + 2, SK_DOLLAR)
    }

    /// A string's rest at a line's start ([`M_STRING`]).
    #[cold]
    pub(super) fn string_rest(&mut self) -> usize {
        let sk = self.resume();
        self.mode = M_CODE;
        self.refill(0);
        self.string_any(0, sk)
    }

    /// A string suspended at a `${` or a line's end: its kind kept on [`Scan::skinds`].
    #[inline(always)]
    pub(super) fn suspend(&mut self, sk: u8) {
        self.skinds = self.skinds << 4 | u32::from(sk);
    }

    /// The innermost suspended string's kind, taken off [`Scan::skinds`].
    #[inline(always)]
    fn resume(&mut self) -> u8 {
        let sk = (self.skinds & 15) as u8;
        self.skinds >>= 4;
        sk
    }

    /// A string's text from `p` of kind `sk`, its escapes, `$name`s and `${…}`s, to its end, a
    /// `${` (code to its `}`, [`F_TMPL`]) or the line's end: a `"…"` and a `'…'` string end
    /// there (but after a `\`), the others go on ([`M_STRING`]).
    #[inline(always)]
    pub(super) fn string(&mut self, mut p: usize, sk: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let text = flavor(sk);
        loop {
            let q = match sk {
                SK_DQ | SK_TDQ => find_in(b, p, n, *b"\"\\$"),
                SK_SQ | SK_TSQ => find_in(b, p, n, *b"'\\"),
                SK_SLASHY => find_in(b, p, n, *b"/\\$"),
                _ => find_in(b, p, n, *b"/\\$"),
            };
            if q >= n {
                if q > p {
                    self.put_hot(p, n, text);
                }
                if spans_lines(sk) {
                    self.mode = M_STRING;
                    self.suspend(sk);
                }
                return n;
            }
            let c = b[q];
            if q > p {
                self.put_hot(p, q, text);
            }
            match c {
                b'"' | b'\'' => {
                    if !matches!(sk, SK_TDQ | SK_TSQ) {
                        self.put_hot(q, q + 1, text + END);
                        return q + 1;
                    }
                    if b.get(q + 1) == Some(&c) && b.get(q + 2) == Some(&c) {
                        self.put_hot(q, q + 3, text + END);
                        return q + 3;
                    }
                    self.put_hot(q, q + 1, text);
                    p = q + 1;
                }
                b'/' => {
                    if sk == SK_SLASHY {
                        self.put(q, q + 1, text + END);
                        return q + 1;
                    }
                    if b.get(q + 1) == Some(&b'$') {
                        self.put(q, q + 2, text + END);
                        return q + 2;
                    }
                    self.put(q, q + 1, text);
                    p = q + 1;
                }
                b'\\' => {
                    p = self.backslash(q, sk);
                    if p > n {
                        // `\` at the line's end: the string goes on
                        self.mode = M_STRING;
                        self.suspend(sk);
                        return n;
                    }
                }
                _ => {
                    // `$`
                    match self.dollar_in(q, sk) {
                        Some(e) => p = e,
                        None => return q + 2,
                    }
                }
            }
        }
    }

    /// A `\` at `q` in a string of kind `sk`: an escape, a bad one, or text; past the line's
    /// end (`n + 1`) where it continues the string.
    #[inline(never)]
    fn backslash(&mut self, q: usize, sk: u8) -> usize {
        let sk = base(sk);
        let b = self.b;
        let n = b.len();
        let text = flavor(sk);
        if q + 1 == n {
            // bat's escaped end-of-line
            self.put(q, n, text + ESCAPE);
            return n + 1;
        }
        match sk {
            SK_SLASHY | SK_DOLLAR => {
                let e = match b[q + 1] {
                    b'/' if sk == SK_SLASHY => (q + 2, true),
                    b'u' => unicode(b, q),
                    _ => (q, true),
                };
                if e.0 == q {
                    self.put(q, q + 1, text);
                    return q + 1;
                }
                self.put(q, e.0, text + if e.1 { ESCAPE } else { BAD_ESCAPE });
                e.0
            }
            _ => {
                let (e, ok) = escape(b, q);
                self.put(q, e, text + if ok { ESCAPE } else { BAD_ESCAPE });
                e
            }
        }
    }

    /// A `$` at `q` in a string of kind `sk`: `$name` (and its `.name`s), a `${` (`None`: the
    /// code goes on after it), a dollar-slashy string's `$$` and `$/`, or a bad `$`.
    #[inline(never)]
    fn dollar_in(&mut self, q: usize, full: u8) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        let sk = base(full);
        let text = flavor(sk);
        if let Some(mut e) = ident_end(b, q + 1) {
            self.put(q, e, text + VAR);
            // bat's `\.(?=name)` and the name, again
            while b.get(e) == Some(&b'.')
                && let Some(f) = ident_end(b, e + 1)
            {
                self.put(e, e + 1, text + VAR_DOT);
                self.put(e + 1, f, text + VAR);
                e = f;
            }
            return Some(e);
        }
        match b.get(q + 1) {
            Some(b'{') => {
                self.put(q, q + 2, text + EMBED);
                self.suspend(full);
                self.push(F_TMPL);
                self.refill(q + 2);
                self.prev = P_START;
                None
            }
            Some(b'$' | b'/') if sk == SK_DOLLAR => {
                self.put(q, q + 2, text + ESCAPE);
                Some(q + 2)
            }
            _ if matches!(sk, SK_DQ | SK_TDQ) => {
                // bat's `\$(?=")` and `\$[^"]+`
                let e = if b.get(q + 1) == Some(&b'"') {
                    q + 1
                } else {
                    find_in(b, q + 1, n, *b"\"")
                };
                self.put(q, e, text + BAD_DOLLAR);
                Some(e)
            }
            _ => {
                self.put(q, q + 1, text);
                Some(q + 1)
            }
        }
    }

    /// The `}` at `p` that closes a string's `${`: its string goes on.
    #[cold]
    #[inline(never)]
    pub(super) fn template_close(&mut self, p: usize) -> usize {
        self.fill_to(p);
        self.pop();
        let sk = self.resume();
        let sk = if sk == 0 { SK_DQ } else { sk };
        self.refill(p);
        self.put(p, p + 1, flavor(sk) + EMBED);
        self.prev = P_VALUE;
        self.string_any(p + 1, sk)
    }

    /// A regex's string from `p` of kind `sk` ([`crate::regex`], Java's dialect): the string's
    /// own patterns first (its end, a `$name`, a `${`, an escape but a cooked `\\`), the regex's
    /// between, to the string's end or the line's (a slashy or dollar-slashy one goes on).
    #[cold]
    #[inline(never)]
    fn regex_text(&mut self, p: usize, sk: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let f = base(sk);
        let text = flavor(sk);
        let embed = &RE_EMBEDS[match f {
            SK_DQ => 0,
            SK_SQ => 1,
            SK_SLASHY => 2,
            _ => 3,
        }];
        let mut at = p;
        let mut q = p;
        while q < n {
            let c = b[q];
            let end = match (f, c) {
                (SK_DQ, b'"') | (SK_SQ, b'\'') | (SK_SLASHY, b'/') => 1,
                (SK_DOLLAR, b'/') if b.get(q + 1) == Some(&b'$') => 2,
                _ => 0,
            };
            if end > 0 {
                self.put(at, q, text);
                self.put(q, q + end, text + END);
                self.ctx = 0;
                return q + end;
            }
            if c == b'$' && interpolates(sk) {
                if dollar_takes(b, q)
                    || (f == SK_DOLLAR && matches!(b.get(q + 1), Some(b'$' | b'/')))
                {
                    self.put(at, q, text);
                    match self.dollar_in(q, sk) {
                        Some(e) => {
                            (q, at) = (e, e);
                            continue;
                        }
                        None => return q + 2,
                    }
                }
            } else if c == b'\\' {
                let e = re_escape(b, q, sk);
                if e > q {
                    self.put(at, q, text);
                    self.put(q, e, text + ESCAPE);
                    (q, at) = (e, e);
                    continue;
                }
                if q + 1 == n && matches!(f, SK_DQ | SK_SQ) {
                    // `\` at the line's end: the string goes on
                    self.put(at, q, text);
                    self.put(q, n, text + ESCAPE);
                    self.mode = M_STRING;
                    self.suspend(sk);
                    return n;
                }
            }
            self.k.close(self.out);
            let r = regex::embedded(b, q, self.ctx, embed, &mut at, self.out);
            self.k.reopen(self.out, 16);
            // the regex's spans went to the sink directly: a `${…}`'s fill starts past them
            self.gap = self.gap.max(at);
            let Some((e, st)) = r else {
                break;
            };
            self.ctx = st;
            q = if e > q { e } else { q + char_len(c) };
        }
        self.put(at, n, text);
        if spans_lines(sk) {
            self.mode = M_STRING;
            self.suspend(sk);
        } else {
            self.ctx = 0;
        }
        n
    }

    /// A SQL string's text from `p` of kind `sk` ([`crate::sql`], in context [`Scan::ctx`]): the
    /// string's own patterns first at each place they can start (its end, an escape, a `$name`,
    /// a `${`: a parameter, code to its `}`), SQL's between, the line cut at the string's end so
    /// no SQL token runs past it; at the line's end a triple-quoted one goes on.
    #[cold]
    #[inline(never)]
    fn sql_text(&mut self, p: usize, sk: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let f = base(sk);
        let text = flavor(sk);
        let host = sql::Embed {
            yields: &SQL_YIELDS[usize::from(f - 1)],
            base: SQL_BASE + u16::from(f - 1) * 128,
            text,
        };
        let end = string_end(b, p, sk);
        let line = &b[..end];
        self.sql.is_end = usize::MAX;
        let mut q = p;
        while q < end {
            let c = b[q];
            if c == b'$' && interpolates(sk) {
                if dollar_takes(b, q) {
                    match self.dollar_in(q, sk) {
                        Some(e) => {
                            q = e.min(end);
                            continue;
                        }
                        None => return q + 2,
                    }
                }
            } else if c == b'\\' {
                let e = self.backslash(q, sk);
                if e > n {
                    self.mode = M_STRING;
                    self.suspend(sk);
                    return n;
                }
                q = e.min(end).max(q + 1);
                continue;
            }
            self.sql.at = q;
            self.k.close(self.out);
            let r = sql::embedded(line, q, self.ctx, &host, &mut self.sql, self.out);
            self.k.reopen(self.out, 16);
            // SQL's spans went to the sink directly: a `${…}`'s fill starts past them
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
            let k = if matches!(f, SK_TDQ | SK_TSQ) { 3 } else { 1 };
            self.put(end, end + k, text + END);
            self.ctx = 0;
            return end + k;
        }
        if spans_lines(sk) {
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
            self.mode = M_STRING;
            self.suspend(sk);
        } else {
            self.ctx = 0;
        }
        n
    }

    /// `//` at `p`: a line comment to the line's end.
    #[inline(always)]
    pub(super) fn line_comment(&mut self, p: usize) -> usize {
        let n = self.b.len();
        self.put_hot(p, p + 2, LINE_MARK);
        if n > p + 2 {
            self.put_hot(p + 2, n, LINE_COMMENT);
        }
        n
    }

    /// `/*` at `p`: a block comment, `/**` a documentation one, `/**/` an empty one.
    #[inline(never)]
    pub(super) fn comment_open(&mut self, p: usize) -> usize {
        let b = self.b;
        if b.get(p + 2) == Some(&b'*') {
            if b.get(p + 3) == Some(&b'/') {
                self.put(p, p + 4, EMPTY_COMMENT);
                return p + 4;
            }
            self.put(p, p + 3, DOC_BEGIN);
            self.mode = M_DOC;
            if SOL {
                return self.comment(p + 3, true);
            }
            return self.doc(p + 3, true);
        }
        self.put(p, p + 2, BLOCK_MARK);
        self.mode = M_COMMENT;
        self.comment(p + 2, false)
    }

    /// A block comment's text from `p` (a documentation one's where `doc`), to its `*/` or the
    /// line's end.
    pub(super) fn comment(&mut self, p: usize, doc: bool) -> usize {
        let b = self.b;
        let n = b.len();
        let (text, end, mark) = if doc {
            (DOC, 2, DOC_END)
        } else {
            (BLOCK_COMMENT, 2, BLOCK_MARK)
        };
        let mut q = p;
        loop {
            q = find_in(b, q, n, *b"*");
            if q >= n {
                if n > p {
                    self.put(p, n, text);
                }
                return n;
            }
            if b.get(q + 1) == Some(&b'/') {
                if q > p {
                    self.put(p, q, text);
                }
                self.put(q, q + end, mark);
                self.mode = M_CODE;
                return q + end;
            }
            q += 1;
        }
    }
}
