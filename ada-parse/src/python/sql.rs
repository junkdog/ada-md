//! bat's `SQL`, embedded in a string that starts with a SQL keyword: its `main` (a list of
//! case-insensitive keyword patterns), its comments, strings and `create`'s name.

use super::*;

/// A piece of a keyword pattern: a word (ASCII case ignored), `\s+`, `\s`, an optional
/// group or alternatives, each tried in the regex's order with its backtracking.
#[derive(Clone, Copy)]
enum Re {
    W(&'static [u8]),
    Sp,
    Sp1,
    Opt(&'static [Re]),
    Or(&'static [&'static [Re]]),
}

use Re::{Opt, Or, Sp, Sp1, W};

/// The objects `create`, `drop` and `alter` take, as `drop`'s and `alter`'s spell them.
const OBJECTS: &[&[Re]] = &[
    &[W(b"aggregate")],
    &[W(b"conversion")],
    &[W(b"database")],
    &[W(b"domain")],
    &[W(b"function")],
    &[W(b"group")],
    &[W(b"index")],
    &[W(b"language")],
    &[W(b"operator class")],
    &[W(b"operator")],
    &[W(b"procedure")],
    &[W(b"rule")],
    &[W(b"schema")],
    &[W(b"sequence")],
    &[W(b"table")],
    &[W(b"tablespace")],
    &[W(b"trigger")],
    &[W(b"type")],
    &[W(b"user")],
    &[W(b"view")],
];

/// `create`'s, in extended mode: `operator class` is `operatorclass`, and each is followed
/// by `\b`.
const CREATE_OBJECTS: &[&[Re]] = &[
    &[W(b"aggregate")],
    &[W(b"conversion")],
    &[W(b"database")],
    &[W(b"domain")],
    &[W(b"function")],
    &[W(b"group")],
    &[Opt(&[W(b"unique"), Sp]), W(b"index")],
    &[W(b"language")],
    &[W(b"operatorclass")],
    &[W(b"operator")],
    &[W(b"procedure")],
    &[W(b"rule")],
    &[W(b"schema")],
    &[W(b"sequence")],
    &[W(b"table"), Opt(&[W(b"space")])],
    &[W(b"trigger")],
    &[W(b"type")],
    &[W(b"user")],
    &[W(b"view")],
];

const MODIFIERS: &[&[Re]] = &[
    &[Or(&[&[W(b"primary")], &[W(b"foreign")]]), Sp, W(b"key")],
    &[W(b"references")],
    &[W(b"on"), Sp1, W(b"delete"), Opt(&[Sp, W(b"cascade")])],
    &[W(b"on"), Sp1, W(b"update"), Opt(&[Sp, W(b"cascade")])],
    &[W(b"check")],
    &[W(b"constraint")],
    &[W(b"default")],
];

const DML: &[&[Re]] = &[
    &[
        W(b"select"),
        Opt(&[Sp, Or(&[&[W(b"distinct")], &[W(b"top")]])]),
    ],
    &[
        W(b"insert"),
        Opt(&[Sp, Opt(&[W(b"ignore"), Sp]), W(b"into")]),
    ],
    &[W(b"update")],
    &[W(b"delete")],
    &[W(b"truncate")],
    &[W(b"from")],
    &[W(b"set")],
    &[W(b"where")],
    &[W(b"group"), Sp, W(b"by")],
    &[W(b"with")],
    &[W(b"case")],
    &[W(b"when")],
    &[W(b"then")],
    &[W(b"else")],
    &[W(b"end")],
    &[W(b"union"), Opt(&[Sp, W(b"all")])],
    &[W(b"using")],
    &[W(b"order"), Sp, W(b"by")],
    &[W(b"limit")],
    &[Or(&[&[W(b"inner")], &[W(b"cross")]]), Sp, W(b"join")],
    &[W(b"join")],
    &[W(b"straight_join")],
    &[
        Or(&[&[W(b"left")], &[W(b"right")]]),
        Opt(&[Sp, W(b"outer")]),
        Sp,
        W(b"join"),
    ],
    &[
        W(b"natural"),
        Opt(&[
            Sp,
            Or(&[&[W(b"left")], &[W(b"right")]]),
            Opt(&[Sp, W(b"outer")]),
        ]),
        Sp,
        W(b"join"),
    ],
];

const LUW: &[&[Re]] = &[
    &[W(b"begin"), Opt(&[Sp, W(b"work")])],
    &[W(b"start"), Sp, W(b"transaction")],
    &[W(b"commit"), Opt(&[Sp, W(b"work")])],
    &[W(b"rollback"), Opt(&[Sp, W(b"work")])],
];

const AUTHORIZATION: &[&[Re]] = &[
    &[
        W(b"grant"),
        Opt(&[Sp1, W(b"with"), Sp1, W(b"grant"), Sp1, W(b"option")]),
    ],
    &[W(b"revoke")],
];

const COMMENT_OBJECTS: &[&[Re]] = &[
    &[W(b"table")],
    &[W(b"column")],
    &[W(b"aggregate")],
    &[W(b"constraint")],
    &[W(b"database")],
    &[W(b"domain")],
    &[W(b"function")],
    &[W(b"index")],
    &[W(b"operator")],
    &[W(b"rule")],
    &[W(b"schema")],
    &[W(b"sequence")],
    &[W(b"trigger")],
    &[W(b"type")],
    &[W(b"view")],
];

/// `bigint|…|text` of the types' first form, each then `\b`.
const TYPES: &[&[u8]] = &[
    b"bigint",
    b"bigserial",
    b"bit",
    b"boolean",
    b"box",
    b"bytea",
    b"cidr",
    b"circle",
    b"date",
    b"datetime",
    b"inet",
    b"int",
    b"integer",
    b"line",
    b"lseg",
    b"macaddr",
    b"money",
    b"ntext",
    b"oid",
    b"path",
    b"point",
    b"polygon",
    b"real",
    b"serial",
    b"smallint",
    b"sysdate",
    b"sysname",
    b"text",
];

impl Scan<'_, '_> {
    /// `\s` at `p`, the line's `\n` one: where it ends.
    fn sql_space(&self, p: usize) -> Option<usize> {
        if p == self.n {
            return Some(p + 1);
        }
        space_len(self.b, p).map(|k| p + k)
    }

    /// Where `\s*` at `p` ends, the line's `\n` taken.
    fn sql_spaces(&self, mut p: usize) -> usize {
        while let Some(e) = self.sql_space(p) {
            p = e;
        }
        p
    }

    /// Whether `\b` holds at `p`.
    fn boundary(&self, p: usize) -> bool {
        let b = &self.b[..self.n];
        let before = p > 0 && p <= self.n && is_word(b, p - 1);
        let after = p < self.n && is_word(b, p);
        before != after
    }

    /// Pattern `pat` at `p`, then `k`: the end `k` gives for the first way through.
    fn re(&self, pat: &[Re], p: usize, k: &dyn Fn(usize) -> Option<usize>) -> Option<usize> {
        let Some((first, rest)) = pat.split_first() else {
            return k(p);
        };
        match *first {
            W(w) => {
                if at_ci(&self.b[..self.n], p, w) {
                    self.re(rest, p + w.len(), k)
                } else {
                    None
                }
            }
            Sp => {
                let mut ends = Vec::new();
                let mut q = p;
                while let Some(e) = self.sql_space(q) {
                    ends.push(e);
                    q = e;
                }
                ends.into_iter().rev().find_map(|e| self.re(rest, e, k))
            }
            Sp1 => self.sql_space(p).and_then(|e| self.re(rest, e, k)),
            Opt(g) => self
                .re(g, p, &|q| self.re(rest, q, k))
                .or_else(|| self.re(rest, p, k)),
            Or(alts) => alts
                .iter()
                .find_map(|a| self.re(a, p, &|q| self.re(rest, q, k))),
        }
    }

    /// `\b(alternatives)\b` at `p`: where it ends.
    fn words(&self, p: usize, alts: &[&[Re]]) -> Option<usize> {
        if !self.boundary(p) {
            return None;
        }
        alts.iter()
            .find_map(|a| self.re(a, p, &|q| self.boundary(q).then_some(q)))
    }

    /// The word at `p`, `\b…\b`, if it is one of `ws` (ASCII case ignored).
    fn word_in(&self, p: usize, ws: &[&[u8]]) -> Option<usize> {
        let e = self.word(p)?;
        let w = &self.b[p..e];
        ws.iter().any(|x| w.eq_ignore_ascii_case(x)).then_some(e)
    }

    /// A SQL context's own patterns at `p` (its string's prototype is tried before).
    pub(super) fn sql(&mut self, p: usize, c: u8) -> Option<usize> {
        match c {
            SQL_MAIN => self.sql_main(p),
            SQL_DASHES | SQL_HASH => (p == self.n).then(|| self.close(p, p + 1, 0)).flatten(),
            SQL_BLOCK => {
                if self.has(p, b"*/") {
                    let w = self.w();
                    return self.close(p, p + 2, w);
                }
                if p == 0 {
                    let q = self.spaces(0);
                    if self.is(q, b'*') && q < self.n && !self.is(q + 1, b'/') {
                        let w = self.w();
                        self.put(0, q, w);
                        return self.tok(q, q + 1, L::SqlCommentMark);
                    }
                }
                None
            }
            SQL_SINGLE | SQL_DOUBLE | SQL_BACKTICK => {
                let quote = match c {
                    SQL_SINGLE => b'\'',
                    SQL_DOUBLE => b'"',
                    _ => b'`',
                };
                if c != SQL_BACKTICK && self.has(p, &[quote, quote]) {
                    return self.tok(p, p + 2, L::SqlEscape);
                }
                if self.is(p, quote) && p < self.n {
                    let k = self.k(L::SqlStringEnd);
                    return self.close(p, p + 1, k);
                }
                if c == SQL_DOUBLE {
                    return self.sql_interpolation(p);
                }
                if self.is(p, b'\\')
                    && let Some(e) = self.any(p + 1)
                {
                    return self.tok(p, e, L::SqlEscape);
                }
                None
            }
            SQL_BRACKETS | SQL_MODR => {
                if self.is(p, b'}') && p < self.n {
                    let k = self.k(L::SqlStringEnd);
                    return self.close(p, p + 1, k);
                }
                self.sql_interpolation(p)
            }
            SQL_CREATE => self.sql_create(p),
            _ => None,
        }
    }

    /// `string_interpolation`: `(#\{)([^\}]*)(\})`.
    fn sql_interpolation(&mut self, p: usize) -> Option<usize> {
        if !self.has(p, b"#{") {
            return None;
        }
        let e = (p + 2..self.n).find(|&q| self.b[q] == b'}')?;
        self.tok(p, p + 2, L::SqlInterpolatedBegin);
        self.tok(p + 2, e, L::SqlInterpolated);
        self.tok(e, e + 1, L::SqlInterpolatedEnd)
    }

    /// `identifier_create`.
    fn sql_create(&mut self, p: usize) -> Option<usize> {
        // `{{end_identifier}}`: `(?=[ \t]*(?:[^\w'"`. \t]|$))`
        let end_ident = |s: &Self, q: usize| {
            let mut r = q;
            while matches!(s.ch(r), Some(b' ' | b'\t')) && r < s.n {
                r += 1;
            }
            s.eol(r) || !(is_word(s.b, r) || matches!(s.b[r], b'\'' | b'"' | b'`' | b'.'))
        };
        // a name: `\w+`, or one in quotes, whose text is the capture
        let name = |s: &Self, q: usize| -> Option<(usize, usize, usize)> {
            match s.ch(q)? {
                quote @ (b'\'' | b'"' | b'`') if q < s.n => {
                    let e = (q + 1..s.n).find(|&i| s.b[i] == quote)?;
                    (e > q + 1).then_some((q + 1, e, e + 1))
                }
                _ if q < s.n && is_word(s.b, q) => {
                    let e = word_end(s.b, q);
                    Some((q, e, e))
                }
                _ => None,
            }
        };
        if let Some((s, e, end)) = name(self, p) {
            if end_ident(self, end) {
                let w = self.w();
                self.put(p, s, w);
                let k = self.k(L::SqlName);
                self.put(s, e, k);
                return self.close(e, end, w);
            }
            let d = self.spaces(end);
            if self.is(d, b'.') && d < self.n {
                let w = self.w();
                self.put(p, d, w);
                return self.tok(d, d + 1, L::SqlDot);
            }
        }
        if end_ident(self, p) {
            return self.leave(p);
        }
        None
    }

    /// The SQL's `main`.
    fn sql_main(&mut self, p: usize) -> Option<usize> {
        let w = self.w();
        // comments
        if self.has(p, b"--") || (self.is(p, b'#') && p < self.n) || self.has(p, b"/*") {
            let (ctx, l) = match self.b[p] {
                b'-' => (SQL_DASHES, 2),
                b'#' => (SQL_HASH, 1),
                _ => (SQL_BLOCK, 2),
            };
            let k = self.base_of(ctx) | L::SqlCommentMark as u16;
            return self.tok_push(p, p + l, k, &[ctx]);
        }
        // `create`
        if let Some(e) = self.kw_ci(p, b"create") {
            let with_replace = self.re(&[Sp, W(b"or"), Sp, W(b"replace")], e, &Some);
            for c1 in [with_replace, Some(e)].into_iter().flatten() {
                let s = self.sql_spaces(c1);
                if s == c1 {
                    continue;
                }
                let found = CREATE_OBJECTS
                    .iter()
                    .find_map(|a| self.re(a, s, &|q| self.boundary(q).then_some(q)));
                if let Some(o) = found {
                    let end = self.sql_spaces(o);
                    self.tok(p, c1, L::SqlCreate);
                    self.put(c1, s, w);
                    self.tok(s, o, L::SqlKeyword);
                    return self.tok_push(o, end, w, &[SQL_CREATE]);
                }
            }
        }
        // `\s*\b(drop)\s+(objects)`, `\s*(drop)\s+(table)\s+(\w+)(\s+cascade)?\b`,
        // `\s*\b(alter)\s+(objects)\s+`
        let s = self.sql_spaces(p);
        if s < self.n {
            if self.boundary(s)
                && let Some(d) = self.re(&[W(b"drop"), Sp], s, &Some)
                && let Some(o) = OBJECTS.iter().find_map(|a| self.re(a, d, &Some))
            {
                self.put(p, s, w);
                self.tok(s, s + 4, L::SqlCreate);
                self.put(s + 4, d, w);
                return self.tok(d, o, L::SqlKeyword);
            }
            if at_ci(&self.b[..self.n], s, b"drop")
                && let Some(t) = self.re(&[W(b"drop"), Sp], s, &Some)
                && at_ci(&self.b[..self.n], t, b"table")
                && let Some(nm) = self.re(&[W(b"table"), Sp], t, &Some)
                && nm < self.n
                && is_word(self.b, nm)
            {
                let ne = word_end(self.b, nm);
                let cascade = self.re(&[Sp, W(b"cascade")], ne, &|q| self.boundary(q).then_some(q));
                self.put(p, s, w);
                self.tok(s, s + 4, L::SqlCreate);
                self.put(s + 4, t, w);
                self.tok(t, t + 5, L::SqlTable);
                self.put(t + 5, nm, w);
                self.tok(nm, ne, L::SqlName);
                if let Some(e) = cascade {
                    return self.tok(ne, e, L::SqlCascade);
                }
                return Some(ne);
            }
            if self.boundary(s)
                && let Some(a) = self.re(&[W(b"alter"), Sp], s, &Some)
                && let Some((o, e)) = OBJECTS.iter().find_map(|x| {
                    let o = self.re(x, a, &|q| self.sql_space(q).map(|_| q))?;
                    Some((o, self.sql_spaces(o)))
                })
            {
                self.put(p, s, w);
                self.tok(s, s + 5, L::SqlCreate);
                self.put(s + 5, a, w);
                self.tok(a, o, L::SqlTable);
                self.put(o, e, w);
                return Some(e);
            }
        }
        if let Some(e) = self.sql_type(p) {
            return Some(e);
        }
        if let Some(e) = self.words(p, MODIFIERS) {
            return self.tok(p, e, L::SqlModifier);
        }
        // `\b\d+\b`
        if self.boundary(p)
            && digit(&self.b[..self.n], p).is_some()
            && let e = digits0(&self.b[..self.n], p)
            && self.boundary(e)
        {
            return self.tok(p, e, L::SqlNumber);
        }
        if let Some(e) = self.word_in(p, &[b"true", b"false"]) {
            return self.tok(p, e, L::SqlBoolean);
        }
        if let Some(e) = self.words(p, DML) {
            return self.tok(p, e, L::SqlDml);
        }
        if let Some(e) = self.sql_null(p) {
            return Some(e);
        }
        if let Some(e) = self.word_in(
            p,
            &[
                b"and", b"or", b"like", b"having", b"exists", b"between", b"in",
            ],
        ) {
            return self.tok(p, e, L::SqlLogical);
        }
        if let Some(e) = self.word_in(p, &[b"values"]) {
            return self.tok(p, e, L::SqlDml2);
        }
        if let Some(e) = self.words(p, LUW) {
            return self.tok(p, e, L::SqlLuw);
        }
        if let Some(e) = self.words(p, AUTHORIZATION) {
            return self.tok(p, e, L::SqlAuthorization);
        }
        if let Some(e) = self.comment_on(p) {
            return self.tok(p, e, L::SqlObjectComments);
        }
        if let Some(e) = self.word_in(p, &[b"as"]) {
            return self.tok(p, e, L::SqlAlias);
        }
        if let Some(e) = self.word_in(p, &[b"asc", b"desc"]) {
            return self.tok(p, e, L::SqlOrder);
        }
        let c = self.ch(p).filter(|_| p < self.n)?;
        match c {
            b'*' => return self.tok(p, p + 1, L::SqlStar),
            b'<' if self.has(p, b"<=>") => return self.tok(p, p + 3, L::SqlComparison),
            b'!' | b'<' | b'>' if self.is(p + 1, b'=') => {
                return self.tok(p, p + 2, L::SqlComparison);
            }
            b'=' => return self.tok(p, p + 1, L::SqlComparison),
            b'<' if self.is(p + 1, b'>') => return self.tok(p, p + 2, L::SqlComparison),
            b'<' | b'>' => return self.tok(p, p + 1, L::SqlComparison),
            b'-' | b'+' | b'/' => return self.tok(p, p + 1, L::SqlMath),
            b'|' if self.is(p + 1, b'|') => return self.tok(p, p + 2, L::SqlConcatenator),
            _ => {}
        }
        if let Some(e) = self.word(p) {
            let wd = &self.b[p..e];
            let scalar = [
                &b"current_date"[..],
                b"current_time",
                b"current_timestamp",
                b"current_user",
                b"session_user",
                b"system_user",
            ];
            if scalar.iter().any(|x| wd.eq_ignore_ascii_case(x)) {
                return self.tok(p, e, L::SqlScalar);
            }
            // `\b(AVG|COUNT|MIN|MAX|SUM)(?=\s*\()`, no `\b` after
            for f in [&b"avg"[..], b"count", b"min", b"max", b"sum"] {
                if at_ci(&self.b[..self.n], p, f) && self.paren_follows(p + f.len()) {
                    return self.tok(p, p + f.len(), L::SqlAggregate);
                }
            }
            let strings = [
                &b"concatenate"[..],
                b"convert",
                b"lower",
                b"substring",
                b"translate",
                b"trim",
                b"upper",
            ];
            if strings.iter().any(|x| wd.eq_ignore_ascii_case(x)) {
                return self.tok(p, e, L::SqlStringFunction);
            }
            // `\b(\w+?)\.(\w+)\b`
            if self.is(e, b'.') && e + 1 < self.n && is_word(self.b, e + 1) {
                let t = word_end(self.b, e + 1);
                self.tok(p, e, L::SqlDatabase);
                self.put(e, e + 1, w);
                return self.tok(e + 1, t, L::SqlTableName);
            }
        }
        match c {
            b'\'' | b'`' | b'"' => {
                let ctx = match c {
                    b'\'' => SQL_SINGLE,
                    b'`' => SQL_BACKTICK,
                    _ => SQL_DOUBLE,
                };
                let k = self.base_of(ctx) | L::SqlStringBegin as u16;
                self.tok_push(p, p + 1, k, &[ctx])
            }
            b'%' if self.is(p + 1, b'{') => {
                let k = self.base_of(SQL_BRACKETS) | L::SqlStringBegin as u16;
                self.tok_push(p, p + 2, k, &[SQL_BRACKETS])
            }
            b'%' if self.has(p, b"%r{") => {
                let k = self.base_of(SQL_MODR) | L::SqlStringBegin as u16;
                self.tok_push(p, p + 3, k, &[SQL_MODR])
            }
            b'(' if self.is(p + 1, b')') => {
                self.tok(p, p + 1, L::SqlScopeBegin);
                self.tok(p + 1, p + 2, L::SqlScopeEnd)
            }
            _ => None,
        }
    }

    /// `\bw\b` at `p`, ASCII case ignored.
    fn kw_ci(&self, p: usize, w: &[u8]) -> Option<usize> {
        (self.boundary(p) && at_ci(&self.b[..self.n], p, w)).then_some(p + w.len())
    }

    /// `\b(?:(is)\s+)?(?:(not)\s+)?(null)\b`.
    fn sql_null(&mut self, p: usize) -> Option<usize> {
        if !self.boundary(p) {
            return None;
        }
        let tail = |s: &Self, q: usize| -> Option<(Option<usize>, usize)> {
            // `(?:(not)\s+)?(null)\b`
            if let Some(n) = s.re(&[W(b"not"), Sp], q, &Some)
                && let Some(e) = s.re(&[W(b"null")], n, &|e| s.boundary(e).then_some(e))
            {
                return Some((Some(n), e));
            }
            s.re(&[W(b"null")], q, &|e| s.boundary(e).then_some(e))
                .map(|e| (None, e))
        };
        let with_is = self
            .re(&[W(b"is"), Sp], p, &Some)
            .and_then(|q| tail(self, q).map(|t| (Some(q), t)));
        let (is, (not, e)) = with_is.or_else(|| tail(self, p).map(|t| (None, t)))?;
        let w = self.w();
        let mut at = p;
        if let Some(q) = is {
            self.tok(p, p + 2, L::SqlLogical);
            self.put(p + 2, q, w);
            at = q;
        }
        if let Some(n) = not {
            self.tok(at, at + 3, L::SqlLogical);
            self.put(at + 3, n, w);
            at = n;
        }
        self.tok(at, e, L::SqlNull)
    }

    /// `\s*\b(comment\s+on\s+(objects))\s+.*?\s+(is)\s+`: where it ends.
    fn comment_on(&self, p: usize) -> Option<usize> {
        let s = self.sql_spaces(p);
        if s >= self.n || !self.boundary(s) {
            return None;
        }
        let o = self.re(
            &[W(b"comment"), Sp, W(b"on"), Sp, Or(COMMENT_OBJECTS)],
            s,
            &Some,
        )?;
        // `\s+.*?\s+(is)\s+`: the first `\s+` greedy, the lazy `.*?` short of the `\n`
        let mut starts = Vec::new();
        let mut q = o;
        while let Some(e) = self.sql_space(q) {
            starts.push(e);
            q = e;
        }
        for &start in starts.iter().rev() {
            let mut q = start;
            loop {
                if let Some(e) = self.re(&[Sp, W(b"is"), Sp], q, &Some) {
                    return Some(e);
                }
                if q >= self.n {
                    break;
                }
                q = self.next(q);
            }
        }
        None
    }

    /// The types' pattern.
    fn sql_type(&mut self, p: usize) -> Option<usize> {
        if !self.boundary(p) {
            return None;
        }
        let b = &self.b[..self.n];
        let word = self.word(p);
        let w = self.w();
        // `\b(…|double\sprecision|…)\b`
        if let Some(e) = word {
            let wd = &b[p..e];
            if TYPES.iter().any(|x| wd.eq_ignore_ascii_case(x)) {
                return self.tok(p, e, L::SqlType);
            }
            if wd.eq_ignore_ascii_case(b"double")
                && let Some(e) = self.re(&[W(b"double"), Sp1, W(b"precision")], p, &|q| {
                    self.boundary(q).then_some(q)
                })
            {
                return self.tok(p, e, L::SqlType);
            }
        }
        let num = |q: usize| -> Option<usize> {
            if b.get(q) != Some(&b'(') {
                return None;
            }
            let d = digits0(b, q + 1);
            (d > q + 1 && b.get(d) == Some(&b')')).then_some(d)
        };
        // `\b(bit\svarying|character\s(?:varying)?|tinyint|var\schar|float|interval)\((\d+)\)`
        const SIZED: &[&[Re]] = &[
            &[W(b"bit"), Sp1, W(b"varying")],
            &[W(b"character"), Sp1, Opt(&[W(b"varying")])],
            &[W(b"tinyint")],
            &[W(b"var"), Sp1, W(b"char")],
            &[W(b"float")],
            &[W(b"interval")],
        ];
        if let Some(e) = SIZED
            .iter()
            .find_map(|a| self.re(a, p, &|q| num(q).map(|_| q)))
            && let Some(d) = num(e)
        {
            self.tok(p, e, L::SqlType);
            self.put(e, e + 1, w);
            self.tok(e + 1, d, L::SqlNumber);
            self.put(d, d + 1, w);
            return Some(d + 1);
        }
        let opt_num = |s: &mut Self, e: usize| -> usize {
            if let Some(d) = num(e) {
                s.put(e, e + 1, w);
                s.tok(e + 1, d, L::SqlNumber);
                s.put(d, d + 1, w);
                d + 1
            } else {
                e
            }
        };
        if let Some(e) = word {
            let wd = &b[p..e];
            // `\b(char|number|nvarchar|varbinary|varchar\d?)\b(?:\((\d+)\))?`
            let varchar =
                wd.len() == 8 && wd[..7].eq_ignore_ascii_case(b"varchar") && wd[7].is_ascii_digit();
            if [
                &b"char"[..],
                b"number",
                b"nvarchar",
                b"varbinary",
                b"varchar",
            ]
            .iter()
            .any(|x| wd.eq_ignore_ascii_case(x))
                || varchar
            {
                self.tok(p, e, L::SqlType);
                return Some(opt_num(self, e));
            }
            // `\b(numeric|decimal)\b(?:\((\d+),(\d+)\))?`
            if wd.eq_ignore_ascii_case(b"numeric") || wd.eq_ignore_ascii_case(b"decimal") {
                self.tok(p, e, L::SqlType);
                if b.get(e) == Some(&b'(') {
                    let d1 = digits0(b, e + 1);
                    if d1 > e + 1 && b.get(d1) == Some(&b',') {
                        let d2 = digits0(b, d1 + 1);
                        if d2 > d1 + 1 && b.get(d2) == Some(&b')') {
                            self.put(e, e + 1, w);
                            self.tok(e + 1, d1, L::SqlNumber);
                            self.put(d1, d1 + 1, w);
                            self.tok(d1 + 1, d2, L::SqlNumber);
                            self.put(d2, d2 + 1, w);
                            return Some(d2 + 1);
                        }
                    }
                }
                return Some(e);
            }
            // `\b(times?)\b(?:\((\d+)\))?(\swith(?:out)?\stime\szone\b)?`
            if wd.eq_ignore_ascii_case(b"time") || wd.eq_ignore_ascii_case(b"times") {
                self.tok(p, e, L::SqlType);
                let q = opt_num(self, e);
                let zone = self.re(
                    &[
                        Sp1,
                        W(b"with"),
                        Opt(&[W(b"out")]),
                        Sp1,
                        W(b"time"),
                        Sp1,
                        W(b"zone"),
                    ],
                    q,
                    &|r| self.boundary(r).then_some(r),
                );
                if let Some(z) = zone {
                    return self.tok(q, z, L::SqlType);
                }
                return Some(q);
            }
        }
        // `\b(timestamp)(?:(s|tz))?\b(?:\((\d+)\))?(\s(with|without)\stime\szone\b)?`
        if at_ci(b, p, b"timestamp") {
            let t = p + 9;
            let suffix = [&b"s"[..], b"tz"]
                .iter()
                .find(|x| at_ci(b, t, x) && self.boundary(t + x.len()))
                .map(|x| t + x.len());
            let e = match suffix {
                Some(e) => Some(e),
                None => self.boundary(t).then_some(t),
            };
            if let Some(e) = e {
                self.tok(p, t, L::SqlType);
                self.tok(t, e, L::SqlType);
                let q = opt_num(self, e);
                let zone = self.re(
                    &[
                        Sp1,
                        Or(&[&[W(b"with")], &[W(b"without")]]),
                        Sp1,
                        W(b"time"),
                        Sp1,
                        W(b"zone"),
                    ],
                    q,
                    &|r| self.boundary(r).then_some(r),
                );
                if let Some(z) = zone {
                    return self.tok(q, z, L::SqlType);
                }
                return Some(q);
            }
        }
        None
    }
}
