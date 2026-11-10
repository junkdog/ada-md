//! Kotlin's text: strings (`"…"`, `"""…"""`) with their templates and escapes, characters,
//! comments, and the SQL and regexes in strings ([`crate::sql`], [`crate::regex`]).

use super::*;
use crate::prim::find_in;

/// Where a template at `p` (`$`) ends: `$name` (a name's start after `$`), or `${…}` to the `}`
/// that closes it (nested braces, strings and characters inside), within [`LOOK`] bytes on the
/// line; `None` for a `$` that is text.
pub(super) fn template_end(b: &[u8], p: usize) -> Option<usize> {
    match b.get(p + 1) {
        Some(&c) if c < 0x80 && (c.is_ascii_alphabetic() || c == b'_') => Some(word_end(b, p + 1)),
        Some(b'{') => {
            let end = b.len().min(p + LOOK);
            let mut depth = 1u32;
            let mut i = p + 2;
            if b.get(i) == Some(&b'}') {
                return None;
            }
            while i < end {
                match b[i] {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(i + 1);
                        }
                    }
                    b'"' => {
                        // a string inside: to its quote, past escapes
                        i += 1;
                        while i < end && b[i] != b'"' {
                            i += if b[i] == b'\\' { 2 } else { 1 };
                        }
                    }
                    b'\'' => {
                        i += 1;
                        while i < end && b[i] != b'\'' {
                            i += if b[i] == b'\\' { 2 } else { 1 };
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            None
        }
        Some(&c) if c >= 0x80 && is_word_starting(b, p + 1) => Some(word_end(b, p + 1)),
        _ => None,
    }
}

/// Whether a `"""` string ends at `p`: the last three quotes of a run, as Kotlin reads them.
pub(super) fn raw_end(b: &[u8], p: usize) -> bool {
    b[p..].starts_with(b"\"\"\"") && b.get(p + 3) != Some(&b'"')
}

/// Where a `\` escape at `p` in a `"…"` string or a character ends: `\uXXXX`, else the `\` and
/// the character after it.
fn escape_end(b: &[u8], p: usize) -> usize {
    regex::java_escape(b, p).min(b.len())
}

/// Where the string SQL is in ends, from `p` inside it: its quote (a raw string's last three of
/// a run), past escapes and templates, or the line's end.
fn string_end(b: &[u8], mut p: usize, raw: bool) -> usize {
    while p < b.len() {
        match b[p] {
            b'\\' if !raw => p += 2,
            b'$' => p = template_end(b, p).unwrap_or(p + 1),
            b'"' if !raw || raw_end(b, p) => return p,
            _ => p += 1,
        }
    }
    b.len()
}

/// The bytes a string's patterns start on: `"…"`'s, then `"""…"""`'s (no escape).
static YIELDS: [[bool; 256]; 2] = {
    let mut t = [[false; 256]; 2];
    t[0][b'"' as usize] = true;
    t[0][b'$' as usize] = true;
    t[0][b'\\' as usize] = true;
    t[1][b'"' as usize] = true;
    t[1][b'$' as usize] = true;
    t
};

/// A regex in a `"…"` string, then in a `"""…"""` one.
static RE_EMBEDS: [regex::Embed<'static>; 2] = [
    regex::Embed {
        dialect: regex::Dialect::Java,
        cooked: true,
        yields: &YIELDS[0],
        stop: |b, p, _| b[p] == b'"' || template_end(b, p).is_some(),
        arg: 0,
        escape: regex::java_escape,
        top_only: false,
        continues: false,
        base: RE_STRING,
        text: STRING_TEXT,
    },
    regex::Embed {
        dialect: regex::Dialect::Java,
        cooked: false,
        yields: &YIELDS[1],
        stop: |b, p, _| raw_end(b, p) || (b[p] == b'$' && template_end(b, p).is_some()),
        arg: 0,
        escape: regex::java_escape,
        top_only: false,
        continues: false,
        base: RE_RAW,
        text: RAW_TEXT,
    },
];

/// Whether the string whose quote opens at `p` is a regex: right after `Regex(`,
/// `Pattern.compile(` or `Pattern.matches(`. One right before `.toRegex(` or `.toPattern(` is
/// found at its end ([`Scan::relex`]).
fn regex_string(b: &[u8], p: usize) -> bool {
    let q = crate::trim_end(b, 0, p);
    if q == 0 || b[q - 1] != b'(' {
        return false;
    }
    let w = crate::trim_end(b, 0, q - 1);
    // the name's last byte first: most calls end in none of the three
    let name: &[u8] = match w.checked_sub(1).map(|i| b[i]) {
        Some(b'x') => b"Regex",
        Some(b'e') => b"Pattern.compile",
        Some(b's') => b"Pattern.matches",
        _ => return false,
    };
    b[..w].ends_with(name) && (w == name.len() || !is_word(b, w - name.len() - 1))
}

impl<K: Sink> Scan<'_, '_, K> {
    /// A `"` at `p`: a `"…"` or `"""…"""` string, its text SQL's or a regex's where the call,
    /// annotation or marker around it says so.
    #[inline(always)]
    pub(super) fn dquote(&mut self, p: usize) -> usize {
        let b = self.b;
        let raw = b.get(p + 1..p + 3) == Some(b"\"\"");
        let (n, text) = if raw { (3, RAW_TEXT) } else { (1, STRING_TEXT) };
        self.put_hot(p, p + n, text + BEGIN);
        self.prev = P_VALUE;
        let s = p + n;
        if SOL {
            return self.string(s, raw);
        }
        if matches!(self.top(), F_SQLCALL | F_SQLANNOT) || (p >= self.mark && self.marked_at(p)) {
            self.mark = usize::MAX;
            self.ctx = sql::MAIN;
            return self.sql_text(s, raw);
        }
        if regex_string(b, p) {
            self.ctx = 0;
            return self.regex_text(s, raw);
        }
        self.opened = Some((s, self.k.mark(self.out)));
        self.string(s, raw)
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

    /// A `"""` string's rest at a line's start: plain, SQL's or a regex's by the mode.
    #[cold]
    pub(super) fn raw_rest(&mut self) -> usize {
        match self.mode {
            M_RAW_SQL => self.sql_text(0, true),
            M_RAW_RE => self.regex_text(0, true),
            _ => self.string(0, true),
        }
    }

    /// A string's text from `p`, its templates and (in a `"…"` one) escapes, to its end quote
    /// or the line's end: a `"…"` string ends there, a `"""…"""` one goes on ([`M_RAW`]).
    #[inline(always)]
    pub(super) fn string(&mut self, mut p: usize, raw: bool) -> usize {
        let b = self.b;
        let n = b.len();
        let text = if raw { RAW_TEXT } else { STRING_TEXT };
        loop {
            let q = if raw {
                find_in(b, p, n, *b"\"$")
            } else {
                find_in(b, p, n, *b"\"$\\")
            };
            if q >= n {
                self.put_hot(p, n, text);
                self.mode = if raw { M_RAW } else { M_CODE };
                return n;
            }
            match b[q] {
                b'"' => {
                    if !raw || raw_end(b, q) {
                        self.put_hot(p, q, text);
                        return self.string_close(q, if raw { 3 } else { 1 }, text);
                    }
                    self.put_hot(p, q + 1, text);
                    p = q + 1;
                }
                b'$' => match template_end(b, q) {
                    Some(e) => {
                        self.put_hot(p, q, text);
                        self.put_hot(q, e, text + TEMPLATE);
                        p = e;
                    }
                    // a template left open at the line's end: code to its `}`
                    None if !SOL && b.get(q + 1) == Some(&b'{') => {
                        self.put_hot(p, q, text);
                        self.put(q, q + 2, text + TEMPLATE);
                        self.push(if raw { F_TMPL_RAW } else { F_TMPL_DQ });
                        self.mode = M_CODE;
                        self.prev = P_OP;
                        return q + 2;
                    }
                    None => {
                        self.put_hot(p, q + 1, text);
                        p = q + 1;
                    }
                },
                _ => {
                    let e = escape_end(b, q);
                    self.put_hot(p, q, text);
                    self.put(q, e, text + ESCAPE);
                    p = e;
                }
            }
        }
    }

    /// A string's end quote, `n` bytes at `q`. Right before `.toRegex(` or `.toPattern(`, the
    /// string opened on the line is a regex: its spans are taken back and it is lexed again as
    /// one.
    #[inline(always)]
    fn string_close(&mut self, q: usize, n: usize, text: u16) -> usize {
        self.mode = M_CODE;
        let rest = &self.b[q + n..];
        if !SOL
            && rest.first() == Some(&b'.')
            && (rest.starts_with(b".toRegex(") || rest.starts_with(b".toPattern("))
            && let Some((s, m)) = self.opened.take()
        {
            return self.relex(s, m);
        }
        self.put_hot(q, q + n, text + END);
        q + n
    }

    /// [`Scan::string_close`]'s regex: the string from `s`, its lexemes since `m`, again.
    #[cold]
    #[inline(never)]
    fn relex(&mut self, s: usize, m: Mark) -> usize {
        self.k.undo(self.out, m);
        let raw = s >= 3 && self.b[s - 3..s] == *b"\"\"\"";
        self.ctx = 0;
        self.regex_text(s, raw)
    }

    /// A regex's string from `p` ([`crate::regex`], Java's dialect): the string's own patterns
    /// first (its end, a template, an escape but `\\`, which is the regex's), the regex's
    /// between, to the string's end or the line's ([`M_RAW_RE`] for a `"""` one).
    #[cold]
    #[inline(never)]
    fn regex_text(&mut self, p: usize, raw: bool) -> usize {
        let b = self.b;
        let n = b.len();
        let text = if raw { RAW_TEXT } else { STRING_TEXT };
        let embed = &RE_EMBEDS[usize::from(raw)];
        let mut at = p;
        let mut q = p;
        while q < n {
            let c = b[q];
            if c == b'"' && (!raw || raw_end(b, q)) {
                self.put(at, q, text);
                let k = if raw { 3 } else { 1 };
                self.put(q, q + k, text + END);
                self.mode = M_CODE;
                self.ctx = 0;
                return q + k;
            }
            if c == b'$'
                && let Some(e) = template_end(b, q)
            {
                self.put(at, q, text);
                self.put(q, e, text + TEMPLATE);
                (q, at) = (e, e);
                continue;
            }
            if c == b'\\' && !raw && b.get(q + 1) != Some(&b'\\') {
                let e = escape_end(b, q);
                self.put(at, q, text);
                self.put(q, e, text + ESCAPE);
                (q, at) = (e, e);
                continue;
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
        if raw {
            self.mode = M_RAW_RE;
        } else {
            self.mode = M_CODE;
            self.ctx = 0;
        }
        n
    }

    /// A SQL string's text from `p` ([`crate::sql`], in context [`Scan::ctx`]): the string's own
    /// patterns first at each place they can start (its end, a template, an escape), SQL's
    /// between, the line cut at the string's end so no SQL token runs past it; at the line's end
    /// a `"""` one goes on ([`M_RAW_SQL`]).
    #[cold]
    #[inline(never)]
    fn sql_text(&mut self, p: usize, raw: bool) -> usize {
        let b = self.b;
        let n = b.len();
        let (text, base) = if raw {
            (RAW_TEXT, SQL_RAW)
        } else {
            (STRING_TEXT, SQL_STRING)
        };
        let host = sql::Embed {
            yields: &YIELDS[usize::from(raw)],
            base,
            text,
        };
        let end = string_end(b, p, raw);
        let line = &b[..end];
        self.sql.is_end = usize::MAX;
        let mut q = p;
        while q < end {
            let c = b[q];
            if c == b'$'
                && let Some(e) = template_end(b, q)
            {
                self.put(q, e, text + TEMPLATE);
                q = e;
                continue;
            }
            if c == b'\\' && !raw {
                let e = escape_end(b, q).min(end);
                self.put(q, e, text + ESCAPE);
                q = e;
                continue;
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
        if end < n {
            let k = if raw { 3 } else { 1 };
            self.put(end, end + k, text + END);
            self.mode = M_CODE;
            self.ctx = 0;
            return end + k;
        }
        if raw {
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
            self.mode = M_RAW_SQL;
        } else {
            self.mode = M_CODE;
            self.ctx = 0;
        }
        n
    }

    /// A character at `p` (`'`): to its closing `'` past escapes, or the line's end.
    pub(super) fn char_literal(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        self.put(p, p + 1, CHAR_TEXT + BEGIN);
        self.prev = P_VALUE;
        let mut q = p + 1;
        let mut at = q;
        while q < n {
            match b[q] {
                b'\'' => {
                    self.put(at, q, CHAR_TEXT);
                    self.put(q, q + 1, CHAR_TEXT + END);
                    return q + 1;
                }
                b'\\' => {
                    let e = escape_end(b, q);
                    self.put(at, q, CHAR_TEXT);
                    self.put(q, e, CHAR_TEXT + ESCAPE);
                    (q, at) = (e, e);
                }
                _ => q += 1,
            }
        }
        self.put(at, n, CHAR_TEXT);
        n
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

    /// `/*` at `p`: a block comment; `/* language=SQL */` marks the next string.
    #[inline(never)]
    pub(super) fn comment_open(&mut self, p: usize) -> usize {
        let b = self.b;
        if !SOL && sql::marker_comment(b, p + 2) {
            self.mark = b[p + 2..]
                .windows(2)
                .position(|w| w == b"*/")
                .map_or(b.len(), |i| p + 4 + i);
        }
        self.put(p, p + 2, BLOCK_MARK);
        self.nest = 1;
        self.mode = M_COMMENT;
        self.comment(p + 2)
    }

    /// A block comment's text from `p`, [`Scan::nest`] deep: Kotlin's comments nest. Where the
    /// code goes on, or the line's end ([`M_COMMENT`]).
    pub(super) fn comment(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        loop {
            let q = find_in(b, p, n, *b"*/");
            if q >= n {
                self.put(p, n, BLOCK_COMMENT);
                return n;
            }
            if b[q] == b'*' && b.get(q + 1) == Some(&b'/') {
                self.put(p, q, BLOCK_COMMENT);
                self.put(q, q + 2, BLOCK_MARK);
                self.nest = self.nest.saturating_sub(1);
                if self.nest == 0 {
                    self.mode = M_CODE;
                    return q + 2;
                }
                p = q + 2;
            } else if b[q] == b'/' && b.get(q + 1) == Some(&b'*') {
                self.put(p, q, BLOCK_COMMENT);
                self.put(q, q + 2, BLOCK_MARK);
                self.nest = self.nest.saturating_add(1);
                p = q + 2;
            } else {
                self.put(p, q + 1, BLOCK_COMMENT);
                p = q + 1;
            }
        }
    }
}
