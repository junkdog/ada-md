//! Elixir's embedded languages: regexes in `~r` and `~R` sigils (bat's) and in
//! `Regex.compile("…")` (ada's), PCRE through Erlang's `re` read in Java's dialect; SQL in Ecto's
//! `fragment(`, a migration's `execute`, and `query` on a repository, `Ecto.Adapters.SQL`,
//! Postgrex and MyXQL (ada's: bat's strings there are text).

use super::*;

/// The bytes a regex string's own patterns start on: by its closing delimiter (the low three
/// bits), a heredoc's (`#`, its comments) at 8; `#` too where it interpolates (from 9); a
/// cooked `"…"`'s (`Regex.compile`) at 18.
static RE_YIELDS: [[bool; 256]; 19] = {
    let mut t = [[false; 256]; 19];
    let mut d = 0;
    while d < 8 {
        t[d][CLOSER[d] as usize] = true;
        t[9 + d][CLOSER[d] as usize] = true;
        t[9 + d][b'#' as usize] = true;
        d += 1;
    }
    t[8][b'#' as usize] = true;
    t[17][b'#' as usize] = true;
    t[18][b'"' as usize] = true;
    t[18][b'#' as usize] = true;
    t[18][b'\\' as usize] = true;
    t
};

/// The bytes a SQL string's own patterns start on: a `"…"`'s, a heredoc's.
static SQL_YIELDS: [[bool; 256]; 2] = {
    let mut t = [[false; 256]; 2];
    t[0][b'"' as usize] = true;
    t[0][b'\\' as usize] = true;
    t[0][b'#' as usize] = true;
    t[1][b'\\' as usize] = true;
    t[1][b'#' as usize] = true;
    t
};

/// Whether the byte at `p` is escaped: an odd run of `\` before it.
fn escaped(b: &[u8], p: usize) -> bool {
    let mut s = p;
    while s > 0 && b[s - 1] == b'\\' {
        s -= 1;
    }
    (p - s) % 2 == 1
}

/// A raw regex's end: its closing delimiter (`arg`), not escaped (Elixir's tokenizer skips
/// `\` and the character after it).
fn stop_raw(b: &[u8], p: usize, arg: usize) -> bool {
    usize::from(b[p]) == arg && !escaped(b, p)
}

/// An interpolating regex's end or `#{`.
fn stop_interp(b: &[u8], p: usize, arg: usize) -> bool {
    (b[p] == b'#' && b.get(p + 1) == Some(&b'{')) || stop_raw(b, p, arg)
}

/// A heredoc regex's comment: `#` after a blank (bat's `(?<=\s)#`, an extended regex's).
fn comment_at(b: &[u8], p: usize) -> bool {
    b[p] == b'#' && p > 0 && matches!(b[p - 1], b' ' | b'\t')
}

/// A raw heredoc regex's comment.
fn stop_raw_heredoc(b: &[u8], p: usize, _: usize) -> bool {
    comment_at(b, p)
}

/// An interpolating heredoc regex's `#{` or comment.
fn stop_interp_heredoc(b: &[u8], p: usize, _: usize) -> bool {
    (b[p] == b'#' && b.get(p + 1) == Some(&b'{')) || comment_at(b, p)
}

/// A sigil's text has no escape of the host's: every `\` is the regex's.
fn no_escape(_: &[u8], p: usize) -> usize {
    p
}

/// A cooked `"…"`'s escape at `p` (Elixir's, as bat's `escaped_char` reads them), but `\\`,
/// the regex's `\`.
fn cooked_escape(b: &[u8], p: usize) -> usize {
    match b.get(p + 1) {
        None | Some(b'\\') => p,
        Some(&c) => (p + 1 + char_len(c)).min(b.len()),
    }
}

/// Where the `"…"` string whose text starts at `p` ends on the line (its closing quote, past
/// escapes and `#{…}`s), or the line's end.
fn string_end(b: &[u8], mut p: usize) -> usize {
    let n = b.len();
    while p < n {
        match b[p] {
            b'\\' => p += 2,
            b'"' => return p,
            b'#' if b.get(p + 1) == Some(&b'{') => match interp_end(b, p + 1) {
                Some(e) => p = e,
                None => return n,
            },
            _ => p += 1,
        }
    }
    n
}

/// Where a `#{` whose `{` is at `p` closes on the line (past its `}`: braces balanced, strings
/// inside skipped); `None` where it doesn't.
fn interp_end(b: &[u8], p: usize) -> Option<usize> {
    let mut depth = 0u32;
    let mut i = p;
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

/// Whether the call named `b[s..e]` (a [`W_SQL`] word, its `!` after) takes SQL: Ecto's
/// `fragment`, a migration's `execute`, `query` on a repository (`Repo`, `MyApp.Repo`,
/// `repo()`), `Ecto.Adapters.SQL` (`SQL`), Postgrex or MyXQL.
pub(super) fn sql_call(b: &[u8], s: usize, e: usize) -> bool {
    match &b[s..e] {
        b"fragment" | b"execute" => true,
        b"query" => {
            if s < 2 || b[s - 1] != b'.' {
                return false;
            }
            let d = s - 1;
            if b[..d].ends_with(b"repo()") {
                return true;
            }
            let mut r = d;
            while r > 0 && WORD[usize::from(b[r - 1])] {
                r -= 1;
            }
            let w = &b[r..d];
            w.ends_with(b"Repo") || matches!(w, b"SQL" | b"Postgrex" | b"MyXQL" | b"Exqlite")
        }
        _ => false,
    }
}

/// Whether the call named `b[s..e]` (a [`W_RE`] word) takes a regex first: `Regex.compile(`,
/// `Regex.compile!(`, `:re.compile(`.
pub(super) fn regex_call(b: &[u8], s: usize, e: usize) -> bool {
    &b[s..e] == b"compile" && (b[..s].ends_with(b"Regex.") || b[..s].ends_with(b":re."))
}

impl<K: Sink> Scan<'_, '_, K> {
    /// A regex's text from `p` of kind `sk` ([`crate::regex`], Java's dialect): the string's
    /// own patterns first (its end, a `#{`, a cooked string's escape but `\\`), the regex's
    /// between, to the string's end or the line's (the string goes on, its state in
    /// [`Scan::ctx`]).
    #[cold]
    #[inline(never)]
    pub(super) fn regex_text(&mut self, p: usize, sk: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let text = flavor(sk);
        let heredoc = sk & HEREDOC != 0;
        let interp = interpolates(sk);
        let cooked = sk & KIND == K_REGEX_STR;
        let closer = if heredoc {
            256
        } else {
            usize::from(CLOSER[usize::from(sk & 7)])
        };
        let yields = if cooked {
            18
        } else if heredoc {
            if interp { 17 } else { 8 }
        } else {
            usize::from(sk & 7) + if interp { 9 } else { 0 }
        };
        let fl = (text - STR_BASE) / STR_LEAVES;
        let embed = regex::Embed {
            dialect: regex::Dialect::Java,
            cooked,
            yields: &RE_YIELDS[yields],
            stop: match (heredoc, interp) {
                (true, true) => stop_interp_heredoc,
                (true, false) => stop_raw_heredoc,
                (false, true) => stop_interp,
                (false, false) => stop_raw,
            },
            arg: closer,
            escape: if cooked { cooked_escape } else { no_escape },
            top_only: false,
            continues: false,
            base: RE_BASE + fl * 128,
            text: text + TEXT,
        };
        let mut at = p;
        let mut q = p;
        while q < n {
            let c = b[q];
            if usize::from(c) == closer && !escaped(b, q) {
                self.put(at, q, text + TEXT);
                self.ctx = 0;
                return self.str_close(q, sk);
            }
            if interp && c == b'#' && b.get(q + 1) == Some(&b'{') {
                self.put(at, q, text + TEXT);
                self.put(q, q + 2, text + INTERP);
                self.suspend(sk);
                self.push(F_INTERP);
                return q + 2;
            }
            if heredoc && comment_at(b, q) && self.ctx & regex::VERBATIM == 0 {
                // an extended regex's comment to the line's end
                self.put(at, q, text + TEXT);
                self.put(q, n, text + RE_COMMENT);
                at = n;
                break;
            }
            if cooked && c == b'\\' && b.get(q + 1) != Some(&b'\\') {
                self.put(at, q, text + TEXT);
                let e = self.escape(q, text);
                (q, at) = (e, e);
                continue;
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
        self.put(at, n, text + TEXT);
        self.mode = M_STRING;
        self.suspend(sk);
        n
    }

    /// A SQL string's text from `p` of kind `sk` ([`crate::sql`], in context [`Scan::ctx`]): the
    /// string's own patterns first at each place they can start (its end, an escape, a `#{`:
    /// code to its `}`), SQL's between, the line cut at the string's end so no SQL token runs
    /// past it; at the line's end the string goes on.
    #[cold]
    #[inline(never)]
    pub(super) fn sql_text(&mut self, p: usize, sk: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let text = flavor(sk);
        let heredoc = sk & HEREDOC != 0;
        let fl = (text - STR_BASE) / STR_LEAVES;
        let host = sql::Embed {
            yields: &SQL_YIELDS[usize::from(heredoc)],
            base: SQL_BASE + fl * 128,
            text: text + TEXT,
        };
        let end = if heredoc { n } else { string_end(b, p) };
        let line = &b[..end];
        self.sql.is_end = usize::MAX;
        let mut q = p;
        while q < end {
            let c = b[q];
            if c == b'#' && b.get(q + 1) == Some(&b'{') {
                self.put(q, q + 2, text + INTERP);
                self.suspend(sk);
                self.push(F_INTERP);
                return q + 2;
            }
            if c == b'\\' {
                let e = self.escape(q, text);
                q = e.min(end).max(q + 1);
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
                self.put(self.sql.at, e, text + TEXT);
            }
            if e > q {
                q = e;
            } else {
                // no progress: the character is the string's text
                let e = (q + char_len(c)).min(end);
                self.put(q, e, text + TEXT);
                q = e;
            }
        }
        if end < n {
            self.ctx = 0;
            return self.str_close(end, sk);
        }
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
        self.mode = M_STRING;
        self.suspend(sk);
        n
    }
}
