//! Haskell's text (X-136): strings with their escapes and gaps, characters, line and block
//! comments, pragmas, quasi-quotes, and the SQL and regexes embedded in quasi-quotes and in the
//! strings of the calls that take them.

use super::*;
use crate::prim::find_any;

/// The bytes a string's patterns start on: its end and an escape. A quasi-quote's: `|`.
static YIELDS: [[bool; 256]; 2] = {
    let mut t = [[false; 256]; 2];
    t[0][b'"' as usize] = true;
    t[0][b'\\' as usize] = true;
    t[1][b'|' as usize] = true;
    t
};

/// A regex in a quasi-quote (`[re|…|]`, PCRE: Java's dialect), then in a string (POSIX's ERE,
/// regex-tdfa's and regex-compat's), the string cut at its end.
static RE_EMBEDS: [regex::Embed<'static>; 2] = [
    regex::Embed {
        dialect: regex::Dialect::Java,
        cooked: false,
        yields: &YIELDS[1],
        stop: |b, p, _| b.get(p..p + 2) == Some(b"|]"),
        arg: 0,
        escape: |_, p| p,
        top_only: false,
        continues: false,
        base: RE_Q,
        text: QUASI,
    },
    regex::Embed {
        dialect: regex::Dialect::Ere,
        cooked: true,
        yields: &YIELDS[0],
        stop: |b, p, _| b[p] == b'"',
        arg: 0,
        escape: haskell_escape,
        top_only: false,
        continues: false,
        base: RE_S,
        text: STRING,
    },
];

/// The ASCII control characters' names an escape may spell, longest first where one starts
/// another (`SOH` before `SO`).
static CONTROLS: [&[u8]; 34] = [
    b"NUL", b"SOH", b"STX", b"ETX", b"EOT", b"ENQ", b"ACK", b"BEL", b"BS", b"HT", b"LF", b"VT",
    b"FF", b"CR", b"SO", b"SI", b"DLE", b"DC1", b"DC2", b"DC3", b"DC4", b"NAK", b"SYN", b"ETB",
    b"CAN", b"EM", b"SUB", b"ESC", b"FS", b"GS", b"RS", b"US", b"SP", b"DEL",
];

// An escape's form ([`escape_at`]).
/// `\n`, `\"`, `\NUL`, `\&`.
const E_CHAR: u8 = 0;
/// `\o17`.
const E_OCTAL: u8 = 1;
/// `\x41`.
const E_HEX: u8 = 2;
/// `\65`.
const E_DECIMAL: u8 = 3;
/// `\^C`.
const E_CONTROL: u8 = 4;

/// The escape at `p`, a `\`: where it ends and its form ([`E_CHAR`]…), `None` where Haskell has
/// none there (a gap's `\` among them).
#[inline]
fn escape_at(b: &[u8], p: usize) -> Option<(usize, u8)> {
    let q = p + 1;
    let c = *b.get(q)?;
    let run = |mut e: usize, ok: fn(u8) -> bool| {
        while e < b.len() && ok(b[e]) {
            e += 1;
        }
        e
    };
    match c {
        b'a' | b'b' | b'f' | b'n' | b'r' | b't' | b'v' | b'\\' | b'"' | b'\'' | b'&' => {
            Some((q + 1, E_CHAR))
        }
        b'o' if b.get(q + 1).is_some_and(|c| matches!(c, b'0'..=b'7')) => {
            Some((run(q + 1, |c| matches!(c, b'0'..=b'7')), E_OCTAL))
        }
        b'x' if b.get(q + 1).is_some_and(u8::is_ascii_hexdigit) => {
            Some((run(q + 1, |c| c.is_ascii_hexdigit()), E_HEX))
        }
        b'0'..=b'9' => Some((run(q, |c| c.is_ascii_digit()), E_DECIMAL)),
        b'^' if b.get(q + 1).is_some_and(|c| {
            matches!(c, b'A'..=b'Z' | b'@' | b'[' | b']' | b'\\' | b'^' | b'_')
        }) =>
        {
            Some((q + 2, E_CONTROL))
        }
        b'A'..=b'Z' => CONTROLS
            .iter()
            .find(|w| b[q..].starts_with(w))
            .map(|w| (q + w.len(), E_CHAR)),
        _ => None,
    }
}

/// [`regex::Embed::escape`] for a Haskell string: where the escape at `p` ends, `p` for none.
fn haskell_escape(b: &[u8], p: usize) -> usize {
    escape_at(b, p).map_or(p, |(e, _)| e)
}

/// Whether `\` at `p` opens a gap: blanks after it to the line's end, or to another `\`.
/// Where the gap's blanks end (the line's end, or the closing `\`).
#[inline]
fn gap_at(b: &[u8], p: usize) -> Option<usize> {
    let q = blanks(b, p + 1);
    if q == p + 1 && q < b.len() {
        return None;
    }
    (q == b.len() || b[q] == b'\\').then_some(q)
}

/// Where a string's text from `p` ends: its `"`, a gap's `\` that runs to the line's end, or
/// the line's end.
fn string_end(b: &[u8], mut p: usize) -> usize {
    let n = b.len();
    while p < n {
        p = find_any(b, p, *b"\"\\");
        if p >= n || b[p] == b'"' {
            return p;
        }
        // as [`Scan::backslash`] reads it: an escape, a gap, or a `\` of the string's text
        if let Some((e, _)) = escape_at(b, p) {
            p = e;
            continue;
        }
        match gap_at(b, p) {
            Some(q) if q == n => return p,
            Some(q) => p = q + 1,
            None => p += 1,
        }
    }
    n
}

/// A quoter's name `w`: what its quasi-quote holds ([`Q_TEXT`]…), its qualifier dropped.
fn quoter(w: &[u8]) -> u8 {
    let name = w.rsplit(|&c| c == b'.').next().unwrap_or(w);
    match name {
        b"" | b"e" | b"d" | b"t" | b"p" if w == name => Q_TH,
        b"sql"
        | b"sqlQQ"
        | b"sqlExp"
        | b"resultlessStatement"
        | b"singletonStatement"
        | b"maybeStatement"
        | b"vectorStatement"
        | b"foldStatement"
        | b"rowsAffectedStatement"
        | b"uncheckedSql" => Q_SQL,
        b"re" | b"rex" | b"reBS" | b"reU" => Q_RE,
        _ => Q_TEXT,
    }
}

impl<K: Sink> Scan<'_, '_, K> {
    /// `--` at `p`: a line comment to the line's end.
    #[inline(always)]
    pub(super) fn line_comment(&mut self, p: usize) -> usize {
        let n = self.b.len();
        self.put(p, p + 2, COMMENT_MARK);
        self.put(p + 2, n, COMMENT);
        n
    }

    /// A block comment's text from `p`, [`Scan::nest`] deep: to its last `-}`, or the line's
    /// end. A `{-` nests (`{-#` too, bat's), a `-}` closes a level.
    pub(super) fn comment(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut at = p;
        let mut q = p;
        loop {
            q = find_any(b, q, *b"{-");
            if q + 1 >= n {
                break;
            }
            if b[q] == b'{' && b[q + 1] == b'-' {
                self.put(at, q, BLOCK);
                if b.get(q + 2) == Some(&b'#') {
                    self.put(q, q + 3, BLOCK);
                    (q, at) = (q + 3, q + 3);
                } else {
                    self.put(q, q + 2, BLOCK_BEGIN);
                    (q, at) = (q + 2, q + 2);
                }
                self.nest = self.nest.saturating_add(1);
            } else if b[q] == b'-' && b[q + 1] == b'}' {
                self.put(at, q, BLOCK);
                self.put(q, q + 2, BLOCK_END);
                (q, at) = (q + 2, q + 2);
                // a level past the 255 kept is lost: the comment ends sooner
                self.nest = self.nest.saturating_sub(1);
                if self.nest == 0 {
                    self.mode = M_CODE;
                    return q;
                }
            } else {
                q += 1;
            }
        }
        self.put(at, n, BLOCK);
        n
    }

    /// A pragma's text from `p`, to its `#-}` or the line's end: its words bat lists
    /// ([`W_PRAGMA`]) keywords, the rest the pragma's.
    pub(super) fn pragma(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut at = p;
        let mut q = p;
        while q < n {
            let c = b[q];
            if c == b'#' && b.get(q + 1..q + 3) == Some(b"-}") {
                self.put(at, q + 3, PRAGMA);
                self.mode = M_CODE;
                return q + 3;
            }
            if c.is_ascii_uppercase() && (q == 0 || !crate::prim::is_word(b, q - 1)) {
                let e = crate::prim::word_end(b, q);
                if info(&b[q..e]) & W_PRAGMA != 0 {
                    self.put(at, q, PRAGMA);
                    self.put(q, e, PRAGMA_KW);
                    at = e;
                }
                q = e;
                continue;
            }
            q += 1;
        }
        self.put(at, n, PRAGMA);
        n
    }

    /// `"` at `p`: a string, its text SQL's or a regex's where the call before it takes one.
    #[inline(always)]
    pub(super) fn dquote(&mut self, p: usize) -> usize {
        self.put_hot(p, p + 1, STRING_BEGIN);
        if !SOL && (self.sqlw != usize::MAX || self.rew != usize::MAX) {
            // a call's string is its first after it: past it, the call's place is gone
            let sql = self.sqlw != usize::MAX && self.sql_place(p) != 0;
            let re = self.rew != usize::MAX && blanks(self.b, self.rew) == p;
            self.sqlw = usize::MAX;
            self.rew = usize::MAX;
            if sql {
                self.ctx = sql::MAIN;
                return self.sql_text(p + 1);
            }
            if re {
                self.ctx = 0;
                return self.regex_text(p + 1);
            }
        }
        self.string(p + 1)
    }

    /// A string's text from `p`: escapes, a gap, to its `"` or the line's end.
    #[inline(always)]
    fn string(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut at = p;
        let mut q = p;
        loop {
            q = find_any(b, q, *b"\"\\");
            if q >= n {
                self.put(at, n, STRING);
                return n;
            }
            self.put(at, q, STRING);
            if b[q] == b'"' {
                self.put(q, q + 1, STRING_END);
                return q + 1;
            }
            match self.backslash(q, S_PLAIN) {
                Some(e) => (q, at) = (e, e),
                None => return n,
            }
        }
    }

    /// `\` at `q` in a string of kind `sk` ([`S_PLAIN`]…): an escape, a gap, or the string's
    /// text. Where the string goes on, `None` where the gap runs past the line.
    #[inline]
    fn backslash(&mut self, q: usize, sk: u8) -> Option<usize> {
        let b = self.b;
        if let Some((e, form)) = escape_at(b, q) {
            let kind = match form {
                E_CONTROL => ESCAPE_CONTROL,
                E_CHAR => ESCAPE,
                _ => ESCAPE_NUM,
            };
            self.put(q, e, kind);
            return Some(e);
        }
        match gap_at(b, q) {
            Some(g) if g == b.len() => {
                self.put(q, q + 1, ESCAPE);
                self.mode = M_GAP;
                self.gap = sk;
                None
            }
            Some(g) => {
                self.put(q, q + 1, ESCAPE);
                self.put(q + 1, g, STRING);
                self.put(g, g + 1, ESCAPE);
                Some(g + 1)
            }
            None => {
                self.put(q, q + 1, STRING);
                Some(q + 1)
            }
        }
    }

    /// A line in a string's gap ([`M_GAP`]) from `p`: blanks, then the gap's closing `\` and the
    /// string on; without one, the string ended at the gap.
    pub(super) fn gap_rest(&mut self, p: usize) -> usize {
        let b = self.b;
        let q = blanks(b, p);
        self.mode = M_CODE;
        if q >= b.len() || b[q] != b'\\' {
            self.ctx = 0;
            return p;
        }
        // the gap's indent is no more the string's than the line's is
        self.put(q, q + 1, ESCAPE);
        match self.gap {
            S_SQL => self.sql_text(q + 1),
            S_RE => self.regex_text(q + 1),
            _ => self.string(q + 1),
        }
    }

    /// `'` at `p`: a character (`'a'`, `'\n'`, `'λ'`), else a quote of Template Haskell's
    /// (`'name`, `''Type`) or a promoted constructor's (`'Just`), plain.
    #[inline(never)]
    pub(super) fn squote(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let q = p + 1;
        if q >= n {
            return q;
        }
        let (e, kind) = match b[q] {
            b'\\' => match escape_at(b, q) {
                Some((e, form)) => (
                    e,
                    match form {
                        E_CHAR => CHAR_ESCAPE,
                        E_HEX => CHAR_HEX,
                        E_CONTROL => CHAR_CONTROL,
                        _ => CHAR_OCTAL,
                    },
                ),
                None => return q,
            },
            b'\'' => return q,
            c if c < 0x20 => return q,
            c => ((q + char_len(c)).min(n), CHAR),
        };
        if b.get(e) != Some(&b'\'') {
            return q;
        }
        self.put(p, q, CHAR_BEGIN);
        self.put(q, e, kind);
        self.put(e, e + 1, CHAR_END);
        e + 1
    }

    /// `[` at `p` before a name or `|`: a quasi-quote, `[quoter|`, where one opens; else plain.
    #[inline(never)]
    pub(super) fn quasi_open(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut e = p + 1;
        while e < n && (NAME[usize::from(b[e])] || (b[e] == b'.' && e > p + 1)) {
            e += 1;
        }
        if b.get(e) != Some(&b'|') {
            return p + 1;
        }
        let w = &b[p + 1..e];
        let kind = if SOL { Q_TEXT } else { quoter(w) };
        if kind == Q_TEXT && !w.contains(&b'.') {
            // `[x|x <- xs]`: a list's comprehension, where no `|]` follows on the line
            let (close, arrow) = self.line_tail();
            if close <= e + 1 && arrow > e + 1 {
                return p + 1;
            }
        }
        if kind == Q_TH && !SOL {
            // a quote of Template Haskell's: code, to its `|]` ([`Scan::th_close`])
            let e = if b.get(e + 1) == Some(&b'|') {
                e + 1
            } else {
                e
            };
            self.put(p, e + 1, QUASI_BRACKET);
            self.th = self.th.saturating_add(1);
            return e + 1;
        }
        self.put(p, e + 1, QUASI_BRACKET);
        self.mode = M_QUASI;
        self.quasi = kind;
        self.ctx = 0;
        self.quasi_rest(e + 1)
    }

    /// `|]` or `||]` at `p`, its `]` at `e`: the innermost quote of Template Haskell's closes.
    #[cold]
    #[inline(never)]
    pub(super) fn th_close(&mut self, p: usize, e: usize) -> usize {
        self.put(p, e + 1, QUASI_BRACKET);
        self.th -= 1;
        e + 1
    }

    /// Where the line's last `|]` and last `<-` start, plus one (0 for none): read once a line,
    /// where a quoter's `[x|` asks.
    #[cold]
    fn line_tail(&mut self) -> (usize, usize) {
        if self.tail.0 == usize::MAX {
            let b = self.b;
            let last = |t: &[u8; 2]| b.windows(2).rposition(|w| w == t).map_or(0, |i| i + 1);
            self.tail = (last(b"|]"), last(b"<-"));
        }
        self.tail
    }

    /// A quasi-quote's body from `p`, to its first `|]` or the line's end.
    pub(super) fn quasi_rest(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut end = p;
        loop {
            end = find_any(b, end, *b"|");
            if end >= n || b.get(end + 1) == Some(&b']') {
                break;
            }
            end += 1;
        }
        let end = end.min(n);
        match self.quasi {
            Q_SQL if !SOL => self.sql_quasi(p, end),
            Q_RE if !SOL => self.regex_quasi(p, end),
            Q_TH if end == n => {
                // bat's `$(` a splice where the line doesn't close the quote
                let mut at = p;
                let mut q = p;
                while let Some(i) = b[q..n].windows(2).position(|t| t == b"$(") {
                    let s = q + i;
                    self.put(at, s, QUASI);
                    self.put(s, s + 2, SPLICE);
                    (q, at) = (s + 2, s + 2);
                }
                self.put(at, n, QUASI);
            }
            _ => self.put(p, end, QUASI),
        }
        if end < n {
            self.put(end, end + 2, QUASI_BRACKET);
            self.mode = M_CODE;
            self.ctx = 0;
            return end + 2;
        }
        n
    }

    /// Whether a SQL string may open at `p` after the line's SQL call ([`Scan::sqlw`]): blanks
    /// and at most one name (the connection) between. Where the call stands there
    /// ([`C_CALL`], [`C_ARG`]), 0 where it doesn't.
    pub(super) fn sql_place(&self, p: usize) -> u8 {
        let b = self.b;
        let mut place = if self.sqlw == self.base && self.sqlc != 0 {
            self.sqlc
        } else {
            C_CALL
        };
        let mut q = blanks(b, self.sqlw);
        if q < p && place == C_CALL && ACT[usize::from(b[q])] == A_LOWER {
            let e = name_end(b, q);
            if info(&b[q..e]) & 0xff != 0 {
                return 0;
            }
            place = C_ARG;
            q = blanks(b, e);
        }
        if q == p { place } else { 0 }
    }

    /// A SQL string's text from `p` ([`crate::sql`], in context [`Scan::ctx`]): the string's
    /// escapes first, SQL's between, the line cut at the string's end so no SQL token runs
    /// past it; a gap at the line's end carries it on.
    #[cold]
    #[inline(never)]
    fn sql_text(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let end = string_end(b, p);
        let line = &b[..end];
        let host = sql::Embed {
            yields: &YIELDS[0],
            base: SQL_S,
            text: STRING,
        };
        self.sql.is_end = usize::MAX;
        let mut q = p;
        while q < end {
            if b[q] == b'\\' {
                match self.backslash(q, S_SQL) {
                    Some(e) => q = e,
                    None => return n,
                }
                continue;
            }
            q = self.sql_turn(line, q, &host);
        }
        if end < n && b[end] == b'"' {
            self.put(end, end + 1, STRING_END);
            self.ctx = 0;
            return end + 1;
        }
        if end < n {
            // a gap at the line's end
            if self.backslash(end, S_SQL).is_none() {
                self.sql_line_end(b, &host);
            }
            return n;
        }
        self.ctx = 0;
        n
    }

    /// SQL's turn at `q` on `line`: up to where the host's patterns may start. Where it stopped.
    #[inline(always)]
    fn sql_turn(&mut self, line: &[u8], q: usize, host: &sql::Embed) -> usize {
        let end = line.len();
        self.sql.at = q;
        self.k.close(self.out);
        let r = sql::embedded(line, q, self.ctx, host, &mut self.sql, self.out);
        self.k.reopen(self.out, 16);
        let Some((e, ctx)) = r else {
            if self.sql.at < end {
                self.put(self.sql.at, end, host.text);
            }
            return end;
        };
        self.ctx = ctx;
        let e = e.min(end);
        if self.sql.at < e {
            self.put(self.sql.at, e, host.text);
        }
        if e > q {
            e
        } else {
            // no progress: the character is the host's text
            let e = (q + char_len(line[q])).min(end);
            self.put(q, e, host.text);
            e
        }
    }

    /// SQL's turn at the line's end, where its line comments end.
    fn sql_line_end(&mut self, b: &[u8], host: &sql::Embed) {
        let n = b.len();
        self.sql.at = n;
        self.k.close(self.out);
        let r = sql::embedded(b, n, self.ctx, host, &mut self.sql, self.out);
        self.k.reopen(self.out, 16);
        if let Some((_, ctx)) = r {
            self.ctx = ctx;
        }
        if matches!(self.ctx, sql::DASHES | sql::HASHES) {
            self.ctx = sql::MAIN;
        }
    }

    /// A SQL quasi-quote's body from `p` to `end` (its `|]`, or the line's end).
    #[cold]
    #[inline(never)]
    fn sql_quasi(&mut self, p: usize, end: usize) {
        let b = self.b;
        let line = &b[..end];
        let host = sql::Embed {
            yields: &YIELDS[1],
            base: SQL_Q,
            text: QUASI,
        };
        self.sql.is_end = usize::MAX;
        let mut q = p;
        while q < end {
            q = self.sql_turn(line, q, &host);
        }
        if end == b.len() {
            self.sql_line_end(b, &host);
        }
    }

    /// A regex string's text from `p` (POSIX's ERE, in state [`Scan::ctx`]): the string's
    /// escapes and the regex's tokens, to the string's end.
    #[cold]
    #[inline(never)]
    fn regex_text(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let end = string_end(b, p);
        let line = &b[..end];
        let embed = &RE_EMBEDS[1];
        let mut at = p;
        let mut q = p;
        while q < end {
            let c = b[q];
            if c == b'\\' && b.get(q + 1) != Some(&b'\\') {
                // the string's escape (`\n`, `\"`), or a gap inside the line
                self.put(at, q, STRING);
                match self.backslash(q, S_RE) {
                    Some(e) => (q, at) = (e, e),
                    None => return n,
                }
                continue;
            }
            self.k.close(self.out);
            let r = regex::embedded(line, q, self.ctx, embed, &mut at, self.out);
            self.k.reopen(self.out, 16);
            let Some((e, st)) = r else {
                break;
            };
            self.ctx = st;
            q = if e > q { e } else { q + char_len(c) };
        }
        self.put(at, end, STRING);
        if end < n && b[end] == b'"' {
            self.put(end, end + 1, STRING_END);
            self.ctx = 0;
            return end + 1;
        }
        if end < n {
            // a gap at the line's end
            self.backslash(end, S_RE);
            return n;
        }
        self.ctx = 0;
        n
    }

    /// A regex quasi-quote's body from `p` to `end` (its `|]`, or the line's end).
    #[cold]
    #[inline(never)]
    fn regex_quasi(&mut self, p: usize, end: usize) {
        let b = self.b;
        let line = &b[..end];
        let embed = &RE_EMBEDS[0];
        let mut at = p;
        let mut q = p;
        while q < end {
            self.k.close(self.out);
            let r = regex::embedded(line, q, self.ctx, embed, &mut at, self.out);
            self.k.reopen(self.out, 16);
            let Some((e, st)) = r else {
                break;
            };
            self.ctx = st;
            q = if e > q { e } else { q + char_len(b[q]) };
        }
        self.put(at, end, QUASI);
    }
}
