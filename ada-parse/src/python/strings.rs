//! Python's strings: the openers in the grammar's order, the string contexts, docstrings,
//! f-strings' replacements, placeholders, and the prototype each embedding string gives the
//! SQL or regex in it.

use super::*;

impl<K: Sink> Scan<'_, '_, K> {
    /// `{{sql_indicator}}` at `p`: `\s*(?:SELECT|INSERT|UPDATE|DELETE|CREATE|REPLACE|ALTER|WITH)\b`.
    #[inline]
    pub(super) fn sql_follows(&self, p: usize) -> bool {
        // most strings start with no space and no capital: the first byte turns them away
        let q = match self.b.get(p) {
            Some(b'S' | b'I' | b'U' | b'D' | b'C' | b'R' | b'A' | b'W') => p,
            Some(&c)
                if c < 0x80 && !matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r') =>
            {
                return false;
            }
            Some(_) => self.spaces(p),
            None => return false,
        };
        if !matches!(
            self.b.get(q),
            Some(b'S' | b'I' | b'U' | b'D' | b'C' | b'R' | b'A' | b'W')
        ) {
            return false;
        }
        let e = word_end(self.b, q);
        matches!(
            &self.b[q..e],
            b"SELECT"
                | b"INSERT"
                | b"UPDATE"
                | b"DELETE"
                | b"CREATE"
                | b"REPLACE"
                | b"ALTER"
                | b"WITH"
        )
    }

    /// `strings` at `p`: a string's prefix and quotes. The grammar's patterns (the blocks of
    /// `"` first, then `"`, then `'`'s, each prefix in its order) come down to the prefix
    /// before the first quote within three bytes, which one pattern of the quote's group
    /// takes whole.
    pub(super) fn strings(&mut self, p: usize) -> Option<usize> {
        let quote_at = |q: usize| q < self.b.len() && matches!(self.b[q], b'"' | b'\'');
        let q0 = if quote_at(p) {
            p
        } else if quote_at(p + 1) {
            p + 1
        } else if quote_at(p + 2) {
            p + 2
        } else {
            return None;
        };
        let quote = self.b[q0];
        let block = self.has(q0, &[quote; 3]);
        let q = u8::from(quote == b'\'') + if block { 2 } else { 0 };
        let pre = &self.b[p..q0];
        let lower = |c: u8| c | 0x20;
        let v = match pre {
            [] | [b'u' | b'U'] => {
                if block {
                    V_DETECT
                } else if self.sql_follows(q0 + 1) {
                    V_SQL
                } else {
                    V_PLAIN
                }
            }
            [b'R'] | [b'u' | b'U', b'R'] => V_RAWCAP_U,
            [b'b' | b'B', b'R'] | [b'R', b'b' | b'B'] => V_RAWCAP_B,
            [b'r'] | [b'u' | b'U', b'r'] => {
                if block {
                    V_RDETECT
                } else if self.sql_follows(q0 + 1) {
                    V_RSQL
                } else {
                    V_RRE
                }
            }
            [b'b' | b'B', b'r'] | [b'r', b'b' | b'B'] => V_BRRE,
            [b'f' | b'F', b'R'] | [b'R', b'f' | b'F'] => V_RF,
            [b'f' | b'F', b'r'] | [b'r', b'f' | b'F'] => V_RFRE,
            [c] if lower(*c) == b'f' => V_F,
            [c] if lower(*c) == b'b' => V_BYTES,
            _ => return None,
        };
        let c = STR + q * VARIANTS + v;
        let k = self.k(L::StringPrefix);
        self.put(p, q0, k);
        let k = self.base_of(c) | L::StringBegin as u16;
        let qlen = if block { 3 } else { 1 };
        self.tok_push(q0, q0 + qlen, k, &[c])
    }

    /// The end of string `q` (`"`, `'`, `"""`, `'''`) at `p`, if it ends there.
    fn string_end(&self, p: usize, q: u8) -> Option<usize> {
        let quote = if q.is_multiple_of(2) { b'"' } else { b'\'' };
        if q >= 2 {
            self.has(p, &[quote; 3]).then_some(p + 3)
        } else {
            (self.is(p, quote) && p < self.b.len()).then_some(p + 1)
        }
    }

    /// A string's context.
    pub(super) fn string(&mut self, p: usize, c: u8) -> Option<usize> {
        let q = (c - STR) / VARIANTS;
        let v = (c - STR) % VARIANTS;
        let block = q >= 2;
        match v {
            V_RDETECT | V_DETECT => {
                let (sql, other) = if v == V_RDETECT {
                    (V_RSQL, V_RRE)
                } else {
                    (V_SQL, V_PLAIN)
                };
                if self.sql_follows(p) {
                    return self.shift(p, &[STR + q * VARIANTS + sql]);
                }
                if self.nonspace(p) {
                    return self.shift(p, &[STR + q * VARIANTS + other]);
                }
                return None;
            }
            _ => {}
        }
        if let Some(e) = self.string_end(p, q) {
            let l = if block && matches!(v, V_RF | V_RFRE | V_F) {
                L::StringBegin
            } else {
                L::StringEnd
            };
            let k = self.k(l);
            return self.then(p, e, k, &[AFTER]);
        }
        if block {
            match v {
                V_RAWCAP_U if q == 2 => self.escaped_unicode(p),
                V_RSQL | V_SQL => self.enter(p, &[SQL_MAIN]),
                V_RRE | V_BRRE | V_RFRE => self.enter(p, &[RE_MAIN]),
                V_RF => self.fcontent(p),
                V_F => self
                    .lcbs(p)
                    .or_else(|| self.fstring_escape(p))
                    .or_else(|| self.escaped_unicode(p))
                    .or_else(|| self.escaped_char(p))
                    .or_else(|| self.fcontent(p)),
                V_PLAIN => self
                    .lcbs(p)
                    .or_else(|| self.escaped_unicode(p))
                    .or_else(|| self.escaped_char(p))
                    .or_else(|| self.placeholder(p)),
                V_BYTES => self
                    .lcbs(p)
                    .or_else(|| self.escaped_char(p))
                    .or_else(|| self.placeholder(p)),
                _ => None,
            }
        } else {
            match v {
                V_RAWCAP_U | V_RAWCAP_B => self.lcs(p),
                V_RSQL | V_SQL => self.lcs(p).or_else(|| self.enter(p, &[SQL_MAIN])),
                V_RRE | V_BRRE | V_RFRE => self.lcs(p).or_else(|| self.enter(p, &[RE_MAIN])),
                V_RF => self.lcs(p).or_else(|| self.fcontent(p)),
                V_F => self
                    .fstring_escape(p)
                    .or_else(|| self.escaped_unicode(p))
                    .or_else(|| self.escaped_char(p))
                    .or_else(|| self.lcs(p))
                    .or_else(|| self.fcontent(p)),
                V_PLAIN => self
                    .escaped_unicode(p)
                    .or_else(|| self.escaped_char(p))
                    .or_else(|| self.lcs(p))
                    .or_else(|| self.placeholder(p)),
                V_BYTES => self
                    .escaped_char(p)
                    .or_else(|| self.lcs(p))
                    .or_else(|| self.placeholder(p)),
                _ => None,
            }
        }
    }

    /// The prototype string `host` gives the embedded context on top, at `p`.
    pub(super) fn proto(&mut self, p: usize, host: u8) -> Option<usize> {
        if host < STR {
            return None;
        }
        let q = (host - STR) / VARIANTS;
        let v = (host - STR) % VARIANTS;
        let block = q >= 2;
        let ends = if block {
            self.string_end(p, q).is_some()
        } else {
            self.string_end(p, q).is_some() || p == self.b.len()
        };
        if ends && let Some(e) = self.leave(p) {
            return Some(e);
        }
        if block {
            match v {
                V_RSQL => self
                    .escaped_unicode(p)
                    .or_else(|| if q == 3 { self.escaped_char(p) } else { None })
                    .or_else(|| self.placeholder(p)),
                V_SQL => self
                    .lcbs(p)
                    .or_else(|| self.escaped_unicode(p))
                    .or_else(|| self.escaped_char(p))
                    .or_else(|| self.placeholder(p)),
                V_RRE => self.escaped_unicode(p),
                V_RFRE => self.fcontent_re(p),
                _ => None,
            }
        } else {
            match v {
                V_RSQL => self.lcs(p).or_else(|| self.placeholder(p)),
                V_SQL => self
                    .escaped_unicode(p)
                    .or_else(|| self.escaped_char(p))
                    .or_else(|| self.lcs(p))
                    .or_else(|| self.placeholder(p)),
                V_RRE => self.lcs(p),
                // `"`'s is an `embed` with an escape, `'`'s a `with_prototype` taking `\`
                V_BRRE if q == 1 => self.lcs(p),
                V_RFRE => self.lcs(p).or_else(|| self.fcontent_re(p)),
                _ => None,
            }
        }
    }

    /// `line-continuation-inside-string`: `(\\)$\n?`, or the line's `\n` ending the string.
    fn lcs(&mut self, p: usize) -> Option<usize> {
        if p + 1 == self.b.len() && self.b[p] == b'\\' {
            self.tok(p, p + 1, L::Continuation);
            return Some(self.b.len() + 1);
        }
        if p == self.b.len() {
            return self.then(p, p + 1, 0, &[AFTER]);
        }
        None
    }

    /// `line-continuation-inside-block-string`: `\\$`.
    fn lcbs(&mut self, p: usize) -> Option<usize> {
        if p + 1 == self.b.len() && self.b[p] == b'\\' {
            return self.tok(p, p + 1, L::Continuation);
        }
        None
    }

    /// `escaped-char`.
    pub(super) fn escaped_char(&mut self, p: usize) -> Option<usize> {
        if !self.is(p, b'\\') || p >= self.b.len() {
            return None;
        }
        let hex = |q: usize| q < self.b.len() && self.b[q].is_ascii_hexdigit();
        let oct = |q: usize| q < self.b.len() && matches!(self.b[q], b'0'..=b'7');
        if self.is(p + 1, b'x') && hex(p + 2) && hex(p + 3) {
            return self.tok(p, p + 4, L::EscapeHex);
        }
        if oct(p + 1) {
            let e = (p + 1..p + 4).take_while(|&q| oct(q)).count() + p + 1;
            return self.tok(p, e, L::EscapeOctal);
        }
        if p + 1 < self.b.len() && b"\\\"'abfnrtv".contains(&self.b[p + 1]) {
            return self.tok(p, p + 2, L::Escape);
        }
        let e = self.any(p + 1)?;
        self.tok(p, e, L::EscapeDeprecated)
    }

    /// `escaped-unicode-char`.
    pub(super) fn escaped_unicode(&mut self, p: usize) -> Option<usize> {
        if !self.is(p, b'\\') || p + 1 >= self.b.len() {
            return None;
        }
        let hex = |from: usize, n: usize| {
            from + n <= self.b.len() && self.b[from..from + n].iter().all(u8::is_ascii_hexdigit)
        };
        match self.b[p + 1] {
            b'U' if hex(p + 2, 8) => self.tok(p, p + 10, L::Escape16),
            b'u' if hex(p + 2, 4) => self.tok(p, p + 6, L::Escape32),
            b'N' if self.is(p + 2, b'{') => {
                let e = (p + 3..self.b.len())
                    .find(|&q| !(self.b[q].is_ascii_alphabetic() || self.b[q] == b' '))
                    .unwrap_or(self.b.len());
                if e > p + 3 && self.is(e, b'}') && e < self.b.len() {
                    return self.tok(p, e + 1, L::EscapeName);
                }
                None
            }
            _ => None,
        }
    }

    /// `escaped-fstring-escape`: `(\\)(\{\{|\}\})`.
    fn fstring_escape(&mut self, p: usize) -> Option<usize> {
        if self.is(p, b'\\') && (self.has(p + 1, b"{{") || self.has(p + 1, b"}}")) {
            self.tok(p, p + 1, L::FEscapeDeprecated);
            return self.tok(p + 1, p + 3, L::FEscapeBraces);
        }
        None
    }

    /// `f-string-content`.
    fn fcontent(&mut self, p: usize) -> Option<usize> {
        match self.ch(p)? {
            b'{' if p < self.b.len() => {
                if self.is(p + 1, b'{') {
                    return self.tok(p, p + 2, L::Escape);
                }
                let q = self.spaces(p + 1);
                if self.is(q, b'}') && q < self.b.len() {
                    return self.tok(p, q + 1, L::EmptyExpression);
                }
                self.enter(p, &[FREPL])
            }
            b'}' if p < self.b.len() => {
                if self.is(p + 1, b'}') {
                    return self.tok(p, p + 2, L::Escape);
                }
                self.tok(p, p + 1, L::StrayBrace)
            }
            _ => None,
        }
    }

    /// `f-string-content-with-regex`.
    fn fcontent_re(&mut self, p: usize) -> Option<usize> {
        if self.is(p, b'\\') && (self.has(p + 1, b"{{") || self.has(p + 1, b"}}")) {
            self.tok(p, p + 1, L::FEscapeBackslash);
            return self.tok(p + 1, p + 3, L::FEscapeBraces);
        }
        match self.ch(p)? {
            b'{' if p < self.b.len() => {
                if self.is(p + 1, b'{') {
                    return self.tok(p, p + 2, L::Escape);
                }
                let q = self.spaces(p + 1);
                if self.is(q, b'}') && q < self.b.len() {
                    return self.tok(p, q + 1, L::EmptyExpression);
                }
                self.enter(p, &[FREPL_R])
            }
            b'}' if p < self.b.len() => {
                if self.is(p + 1, b'}') {
                    return self.tok(p, p + 2, L::Escape);
                }
                self.tok(p, p + 1, L::StrayBrace)
            }
            _ => None,
        }
    }

    /// `constant-placeholder`.
    pub(super) fn placeholder(&mut self, p: usize) -> Option<usize> {
        match self.ch(p)? {
            b'%' if p < self.b.len() => self.printf(p).or_else(|| self.strftime(p)),
            b'{' if self.has(p, b"{{") => self.tok(p, p + 2, L::Escape),
            b'}' if self.has(p, b"}}") => self.tok(p, p + 2, L::Escape),
            b'{' if p < self.b.len() => self.formatting(p),
            _ => None,
        }
    }

    /// The printf placeholder at `p`, a `%`.
    fn printf(&mut self, p: usize) -> Option<usize> {
        let b = &self.b[..self.b.len()];
        let mut q = p + 1;
        let mut key = None;
        // `( \( ({{identifier}}) \) )?`
        if b.get(q) == Some(&b'(')
            && let Some(e) = self.ident(q + 1)
            && b.get(e) == Some(&b')')
        {
            key = Some((q + 1, e));
            q = e + 1;
        }
        let opt = |q: &mut usize, ok: &dyn Fn(u8) -> bool| {
            if b.get(*q).is_some_and(|&c| ok(c)) {
                *q += 1;
            }
        };
        opt(&mut q, &|c| c == b'#');
        opt(&mut q, &|c| c == b'0');
        opt(&mut q, &|c| c == b'-');
        opt(&mut q, &|c| c == b' ');
        opt(&mut q, &|c| c == b'+' || c == b'-');
        // `(\d*|\*)`
        let w = digits0(b, q);
        q = if w > q {
            w
        } else if b.get(q) == Some(&b'*') {
            q + 1
        } else {
            q
        };
        // `(\. (\d*|\*))?`
        if b.get(q) == Some(&b'.') {
            let w = digits0(b, q + 1);
            q = if w > q + 1 {
                w
            } else if b.get(q + 1) == Some(&b'*') {
                q + 2
            } else {
                q + 1
            };
        }
        opt(&mut q, &|c| matches!(c, b'h' | b'l' | b'L'));
        if !b.get(q).is_some_and(|c| b"acdeEfFgGiorsuxX%".contains(c)) {
            return None;
        }
        let e = q + 1;
        if let Some((s, ke)) = key {
            self.tok(p, s, L::Placeholder);
            self.tok(s, ke, L::PlaceholderVariable);
            return self.tok(ke, e, L::Placeholder);
        }
        self.tok(p, e, L::Placeholder)
    }

    /// `{{strftime_spec}}` at `p`, a `%`.
    fn strftime(&mut self, p: usize) -> Option<usize> {
        let c = *self.b.get(p + 1).filter(|_| p + 1 < self.b.len())?;
        if b"aAwdbBGmyYHIpMSfzZjuUVWcxX%".contains(&c) {
            return self.tok(p, p + 2, L::Placeholder);
        }
        if c == b'-' && p + 2 < self.b.len() && b"dmHIMSj".contains(&self.b[p + 2]) {
            return self.tok(p, p + 3, L::Placeholder);
        }
        None
    }

    /// `{{format_spec}}` at `s`, then `}`: where the `}` is, the alignments tried in the
    /// regex's order.
    fn format_spec(&self, s: usize) -> Option<usize> {
        let b = &self.b[..self.b.len()];
        let align = |q: usize| matches!(b.get(q), Some(b'<' | b'>' | b'=' | b'^'));
        let rest = |mut q: usize| {
            if matches!(b.get(q), Some(b' ' | b'+' | b'-')) {
                q += 1;
            }
            if b.get(q) == Some(&b'#') {
                q += 1;
            }
            q = digits0(b, q);
            if b.get(q) == Some(&b',') {
                q += 1;
            }
            if b.get(q) == Some(&b'.')
                && let Some(e) = digit(b, q + 1)
            {
                q = digits0(b, e);
            }
            if b.get(q).is_some_and(|c| b"bcdeEfFgGnosxX%".contains(c)) {
                q += 1;
            }
            (b.get(q) == Some(&b'}')).then_some(q)
        };
        if let Some(c) = self.any(s)
            && align(c)
            && let Some(e) = rest(c + 1)
        {
            return Some(e);
        }
        if align(s)
            && let Some(e) = rest(s + 1)
        {
            return Some(e);
        }
        rest(s)
    }

    /// `formatting-syntax` at `p`, a `{`.
    fn formatting(&mut self, p: usize) -> Option<usize> {
        let b = &self.b[..self.b.len()];
        // the simple form: `(\{)(?:[\w.\[\]]+)?(![ars])?(:(?:{{format_spec}}|[^}%]*%.[^}]*))?(\})`
        let mut q = p + 1;
        while q < self.b.len() && (is_word(b, q) || matches!(b[q], b'.' | b'[' | b']')) {
            q = self.next(q);
        }
        let field = q;
        let conv = (b.get(q) == Some(&b'!') && matches!(b.get(q + 1), Some(b'a' | b'r' | b's')))
            .then_some(q);
        if conv.is_some() {
            q += 2;
        }
        let spec = if b.get(q) == Some(&b':') {
            self.format_spec(q + 1).or_else(|| {
                // `[^}%]*%.[^}]*`
                let r = (q + 1..self.b.len()).find(|&i| matches!(b[i], b'}' | b'%'))?;
                if b[r] != b'%' {
                    return None;
                }
                let d = self.any(r + 1)?;
                (d..self.b.len()).find(|&i| b[i] == b'}')
            })
        } else {
            None
        };
        let close = spec.unwrap_or(q);
        if b.get(close) == Some(&b'}') {
            self.tok(p, p + 1, L::PlaceholderBegin);
            self.tok(p + 1, field, L::Placeholder);
            if let Some(c) = conv {
                self.tok(c, c + 2, L::PlaceholderConversion);
            }
            if spec.is_some() {
                self.tok(q, close, L::PlaceholderSpec);
            }
            return self.tok(close, close + 1, L::PlaceholderEnd);
        }
        // the nested form: `\{(?=[^\}"']+\{[^"']*\})`
        let run = (p + 1..self.b.len())
            .find(|&i| matches!(b[i], b'}' | b'"' | b'\''))
            .unwrap_or(self.b.len());
        if let Some(j) = (p + 2..run).find(|&i| b[i] == b'{') {
            let quote = (j + 1..self.b.len())
                .find(|&i| matches!(b[i], b'"' | b'\''))
                .unwrap_or(self.b.len());
            if (j + 1..quote).any(|i| b[i] == b'}') {
                let k = self.base_of(PLACEHOLDER) | L::PhBegin as u16;
                return self.tok_push(p, p + 1, k, &[PLACEHOLDER]);
            }
        }
        None
    }

    /// The f-strings' replacements and the placeholders' nested form.
    pub(super) fn fstring(&mut self, p: usize, c: u8) -> Option<usize> {
        match c {
            FREPL | FREPL_R => match self.ch(p)? {
                b'}' if p < self.b.len() => {
                    let k = self.k(L::InterpolationEnd);
                    self.close(p, p + 1, k)
                }
                b'{' if p < self.b.len() => {
                    let k = self.k(L::InterpolationBegin);
                    let inner = if c == FREPL { FREPL_IN } else { FREPL_R_IN };
                    self.tok_push(p, p + 1, k, &[inner])
                }
                _ => None,
            },
            FREPL_IN | FREPL_R_IN => match self.ch(p)? {
                b'}' => self.leave(p),
                b'!' if matches!(self.ch(p + 1), Some(b'a' | b'r' | b's')) => {
                    self.tok(p, p + 2, L::Conversion)
                }
                b'=' if c == FREPL_IN => self.tok(p, p + 1, L::Debug),
                b':' => {
                    let k = self.base_of(FSPEC);
                    self.tok_push(p, p + 1, k, &[FSPEC])
                }
                _ => {
                    let expr = if c == FREPL_IN { FEXPR } else { FEXPR_R };
                    self.enter(p, &[expr])
                }
            },
            FSPEC => {
                if self.is(p, b'}') {
                    return self.leave(p);
                }
                self.fcontent(p)
            }
            FEXPR | FEXPR_R => {
                // `(?==?(![^=]|:|\}))`, `(?=![^=]|:|\})`
                let ends = |q: usize| match self.ch(q) {
                    Some(b'!') => self.ch(q + 1).is_some_and(|c| c != b'='),
                    Some(b':' | b'}') => true,
                    _ => false,
                };
                if ((c == FEXPR && self.is(p, b'=') && ends(p + 1)) || ends(p))
                    && let Some(e) = self.leave(p)
                {
                    return Some(e);
                }
                if self.is(p, b'\\') && p < self.b.len() {
                    return self.tok(p, p + 1, L::BackslashInFString);
                }
                self.inline_for(p).or_else(|| self.in_group(p))
            }
            PLACEHOLDER => match self.ch(p)? {
                b'}' if p < self.b.len() => {
                    let k = self.k(L::PhEnd);
                    self.close(p, p + 1, k)
                }
                b'!' if matches!(self.ch(p + 1), Some(b'a' | b'r' | b's')) => {
                    self.tok(p, p + 2, L::Conversion)
                }
                b':' => {
                    let k = self.base_of(PH_SPEC);
                    self.tok_push(p, p + 1, k, &[PH_SPEC])
                }
                _ => {
                    // `[\w.\[\]]+`, no scope of its own
                    let mut q = p;
                    while q < self.b.len()
                        && (is_word(self.b, q) || matches!(self.b[q], b'.' | b'[' | b']'))
                    {
                        q = self.next(q);
                    }
                    (q > p).then(|| {
                        let k = self.w();
                        self.put(p, q, k);
                        q
                    })
                }
            },
            PH_SPEC => {
                if self.is(p, b'}') {
                    return self.leave(p);
                }
                if self.is(p, b'{') && p < self.b.len() {
                    return self.formatting(p);
                }
                None
            }
            _ => None,
        }
    }

    /// `docstrings` at the line's start: `^\s*(?=(?i)(ur|ru|u|r)?("""|'''))`.
    pub(super) fn docstring_start(&mut self, p: usize) -> Option<usize> {
        let q = self.spaces(p);
        let triple = |s: &Self, at: usize| s.has(at, b"\"\"\"") || s.has(at, b"'''");
        let pre = [&b"ur"[..], b"ru", b"u", b"r", b""]
            .iter()
            .any(|x| at_ci(self.b, q, x) && triple(self, q + x.len()));
        if !pre {
            return None;
        }
        let w = self.w();
        self.tok_push(p, q, w, &[DOC_START])
    }

    /// `docstrings`' context, before the quotes.
    pub(super) fn doc_start(&mut self, p: usize) -> Option<usize> {
        let quote_at = |s: &Self, at: usize| {
            if s.has(at, b"\"\"\"") {
                Some(false)
            } else if s.has(at, b"'''") {
                Some(true)
            } else {
                None
            }
        };
        // `(?i)(u)?("""|''')`, `(?i)(u?ru?)("""|''')`
        let plain = [1usize, 0]
            .into_iter()
            .filter(|&l| l == 0 || matches!(self.ch(p), Some(b'u' | b'U')))
            .find_map(|l| quote_at(self, p + l).map(|sq| (l, sq)));
        let (l, sq, raw) = if let Some((l, sq)) = plain {
            (l, sq, false)
        } else {
            let mut l = 0;
            if matches!(self.ch(p), Some(b'u' | b'U')) {
                l += 1;
            }
            if !matches!(self.ch(p + l), Some(b'r' | b'R')) {
                return None;
            }
            l += 1;
            let found = if matches!(self.ch(p + l), Some(b'u' | b'U'))
                && let Some(sq) = quote_at(self, p + l + 1)
            {
                Some((l + 1, sq))
            } else {
                quote_at(self, p + l).map(|sq| (l, sq))
            };
            let (l, sq) = found?;
            (l, sq, true)
        };
        let c = match (raw, sq) {
            (false, false) => DOC_DQ,
            (false, true) => DOC_SQ,
            (true, false) => DOC_RAW_DQ,
            (true, true) => DOC_RAW_SQ,
        };
        let host = self.base_of(c);
        self.put(p, p + l, host | L::StringPrefix as u16);
        self.then(p + l, p + l + 3, host | L::DocBegin as u16, &[c])
    }

    /// A docstring.
    pub(super) fn docstring(&mut self, p: usize, c: u8) -> Option<usize> {
        let end: &[u8] = if c == DOC_DQ || c == DOC_RAW_DQ {
            b"\"\"\""
        } else {
            b"'''"
        };
        if (c == DOC_DQ || c == DOC_SQ)
            && let Some(e) = self.escaped_unicode(p).or_else(|| self.escaped_char(p))
        {
            return Some(e);
        }
        if self.has(p, end) {
            let k = self.k(L::DocEnd);
            return self.close(p, p + 3, k);
        }
        None
    }
}
