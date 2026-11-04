//! Comments, strings, templates, regex literals and JSDoc: the frames whose bytes are text.

use super::*;

impl Scan<'_, '_, '_> {
    /// `comment` at the place: `/** … */`, `/* … */`, `// …` (with the line's indentation before
    /// it as `punctuation.whitespace.comment.leading`).
    pub(super) fn comment(&mut self, consume_eol: bool) -> bool {
        let i = self.i;
        let mut j = i;
        if i == 0 {
            j = self.sp(0);
        }
        if self.c(j) != b'/' {
            return false;
        }
        match self.c(j + 1) {
            b'*' if j == i => {
                if self.c(j + 2) == b'*' && self.c(j + 3) != b'/' {
                    self.tok(j, j + 3, DOC_MARK);
                    self.i = j + 3;
                    self.push(DOC_F);
                    return true;
                }
                self.tok(j, j + 2, BLOCK_MARK);
                self.i = j + 2;
                self.push(BLOCK_F);
                // (?:\s*((@)internal)(?=\s|(\*/)))?
                let k = self.ws(j + 2);
                if self.has(k, b"@internal") {
                    let e = k + 9;
                    if is_space(self.c(e)) || (self.c(e) == b'*' && self.c(e + 1) == b'/') {
                        self.tok(k, k + 1, INTERNAL_MARK);
                        self.tok(k + 1, e, INTERNAL);
                        self.i = e;
                    }
                }
                true
            }
            b'/' => {
                if j > i {
                    self.tok(i, j, LEADING);
                }
                self.tok(j, j + 2, LINE_MARK);
                let mut e = j + 2;
                let k = self.ws(e);
                if self.has(k, b"@internal") {
                    let f = k + 9;
                    if f >= self.b.len() || is_space(self.c(f)) {
                        self.tok(e, k, LINE);
                        self.tok(k, k + 1, LINE_INTERNAL_MARK);
                        self.tok(k + 1, f, LINE_INTERNAL);
                        e = f;
                    }
                }
                self.tok(e, self.b.len(), LINE);
                self.i = if consume_eol {
                    self.b.len() + 1
                } else {
                    self.b.len()
                };
                true
            }
            _ => false,
        }
    }

    /// `string`: `'…'`, `"…"`, or a template (`template-call` first: a tag's `true` is the literal).
    pub(super) fn string(&mut self) -> bool {
        match self.c(self.i) {
            b'\'' => {
                let i = self.i;
                self.tok(i, i + 1, STR_S_BEGIN);
                self.i += 1;
                self.push(STR_S_F);
                if after_paren(self.b, i) && self.regexp_arg(i) {
                    self.push_regex(RE_STR_F);
                }
                true
            }
            b'"' => {
                let i = self.i;
                self.tok(i, i + 1, STR_D_BEGIN);
                self.i += 1;
                self.push(STR_D_F);
                if after_paren(self.b, i) && self.regexp_arg(i) {
                    self.push_regex(RE_STR_F);
                }
                true
            }
            _ => self.template_call() || self.template(false),
        }
    }

    /// `template`: `([_$[:alpha:]][_$[:alnum:]]*)?(`)`. In a type (`template-type`) its `${…}`
    /// holds a type.
    pub(super) fn template(&mut self, in_type: bool) -> bool {
        let i = self.i;
        if self.tick <= i {
            return false;
        }
        let e = self.name(i);
        if self.c(e) != b'`' {
            return false;
        }
        if e > i {
            self.tok(i, e, TPL_TAG);
        }
        self.tok(e, e + 1, TPL_BEGIN);
        self.i = e + 1;
        self.push(if in_type { TPL_TYPE } else { TPL_F });
        if !in_type && e == i && after_paren(self.b, i) && self.regexp_arg(i) {
            self.push_regex(RE_STR_F);
        }
        true
    }

    /// Whether the string or template opening at `i` is the first argument of `RegExp(`
    /// (`new RegExp(…)` too): a regex, in ECMAScript's dialect, as WebStorm colors it.
    #[inline(never)]
    pub(super) fn regexp_arg(&self, i: usize) -> bool {
        let b = self.b;
        let mut k = i;
        while k > 0 && is_space(b[k - 1]) {
            k -= 1;
        }
        if k == 0 || b[k - 1] != b'(' {
            return false;
        }
        k -= 1;
        while k > 0 && is_space(b[k - 1]) {
            k -= 1;
        }
        k >= 6
            && &b[k - 6..k] == b"RegExp"
            && (k == 6 || !(b[k - 7].is_ascii_alphanumeric() || matches!(b[k - 7], b'_' | b'$')))
    }

    /// Push a regex's frame, its state empty.
    pub(super) fn push_regex(&mut self, f: u8) {
        self.push(f);
        self.re = 0;
    }

    /// The regex from the place in host `host` ([`RE_HOSTS`]' order: a literal, a `'…'`, a
    /// `"…"`, a template), up to where the host's patterns can next match, before `lim`, or past
    /// the line.
    #[inline(never)]
    pub(super) fn regex_run(&mut self, host: usize, lim: usize) -> bool {
        static YIELDS: [[bool; 256]; 4] = [
            regex::table(b"/"),
            regex::table(b"'\\"),
            regex::table(b"\"\\"),
            regex::table(b"`\\$"),
        ];
        fn quote(_: &[u8], _: usize, _: usize) -> bool {
            true
        }
        fn tpl(b: &[u8], p: usize, _: usize) -> bool {
            b[p] == b'`' || b.get(p + 1) == Some(&b'{')
        }
        let embed = regex::Embed {
            dialect: regex::Dialect::Ecma,
            cooked: host != 0,
            yields: &YIELDS[host],
            stop: match host {
                0 => |_, _, _| false,
                3 => tpl,
                _ => quote,
            },
            arg: 0,
            escape: escape_at,
            top_only: host == 0,
            continues: host != 0,
            base: (RE_KINDS + host as u16 * regex::KINDS as u16) | self.wrap,
            text: RE_HOSTS[host] | self.wrap,
        };
        let b = &self.b[..lim];
        match regex::embedded(b, self.i, self.re, &embed, &mut self.done, self.out) {
            Some((q, st)) => {
                self.i = q;
                self.re = st;
                true
            }
            None => false,
        }
    }

    /// A regex string's turn ([`RE_STR_F`]): the host string's or template's patterns first
    /// (its end, an escape but `\\`, which is the regex's `\`, a `${`, a string's last character
    /// on its line), then the regex.
    #[inline(never)]
    pub(super) fn regex_string(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        let host = self.stack[self.depth - 2];
        if host == TPL_F {
            match c {
                b'`' => {
                    self.tok(i, i + 1, TPL_END);
                    self.i = i + 1;
                    return self.end_regex_string();
                }
                b'\\' if self.c(i + 1) != b'\\' => {
                    let e = self.escape_end(i);
                    self.tok(i, e, TPL_ESC);
                    self.i = e;
                    return true;
                }
                b'$' if self.c(i + 1) == b'{' => {
                    self.tok(i, i + 2, SUBST_BEGIN);
                    self.i = i + 2;
                    self.spill();
                    self.push(SUBST);
                    return true;
                }
                _ => {}
            }
            if i >= self.b.len() {
                self.i = self.b.len() + 1;
                return true;
            }
            return self.regex_run(3, self.b.len());
        }
        let (q, h, end, esc, bad) = if host == STR_S_F {
            (b'\'', 1, STR_S_END, STR_S_ESC, STR_S_BAD)
        } else {
            (b'"', 2, STR_D_END, STR_D_ESC, STR_D_BAD)
        };
        if c == q {
            self.tok(i, i + 1, end);
            self.i = i + 1;
            return self.end_regex_string();
        }
        if c == b'\\' && self.c(i + 1) != b'\\' {
            let e = self.escape_end(i);
            self.tok(i, e, esc);
            self.i = e;
            return true;
        }
        if i >= self.b.len() {
            self.i = self.b.len() + 1;
            return true;
        }
        // `((?:[^\\\n])$)`: the line's last character ends the string
        let mut last = self.b.len() - 1;
        while last > 0 && self.b[last] & 0xc0 == 0x80 {
            last -= 1;
        }
        if i >= last {
            self.tok(i, self.b.len(), bad);
            self.i = self.b.len();
            return self.end_regex_string();
        }
        // a `\` last is an escape's (a `\\`, the regex's `\`, and the string goes on) or the line's
        // continuation, never `[^\\\n]$`
        let lim = if self.b[last] == b'\\' {
            self.b.len()
        } else {
            last
        };
        self.regex_run(h, lim)
    }

    /// The end of a regex string: its frame and the string's.
    fn end_regex_string(&mut self) -> bool {
        self.re = 0;
        self.pop();
        self.pop()
    }

    /// Keep the top regex's state as frames above its own, where something is pushed above it
    /// or the line ends: [`RE_GROUP_F`] for each group open, then [`RE_SET_F`] for a class.
    #[inline(never)]
    pub(super) fn spill(&mut self) {
        let st = self.re;
        self.re = 0;
        for _ in 0..st & regex::DEPTH {
            self.push(RE_GROUP_F);
        }
        if st & regex::SET != 0 {
            self.push(RE_SET_F);
        }
    }

    /// Undo [`Scan::spill`], where its frames are the top again.
    #[inline(never)]
    pub(super) fn unspill(&mut self) {
        let mut st = 0;
        while self.depth > 1 && matches!(self.top(), RE_GROUP_F | RE_SET_F) {
            st = if self.top() == RE_SET_F {
                st | regex::SET
            } else {
                (st & regex::SET) | ((st & regex::DEPTH) + 1).min(regex::DEPTH)
            };
            self.depth -= 1;
        }
        self.re = if matches!(self.top(), REGEX_F | RE_STR_F) {
            st
        } else {
            0
        };
        self.refresh();
    }

    /// `string-character-escape` at the place, `\` there: its end.
    pub(super) fn escape_end(&self, i: usize) -> usize {
        escape_at(&self.b[..self.b.len()], i)
    }

    /// A frame of text's turn. Out of the scan's loop, where its arms would crowd the code's.
    #[inline(never)]
    pub(super) fn text_step(&mut self, f: u8) -> bool {
        let i = self.i;
        let c = self.c(i);
        match f {
            STR_S_F | STR_D_F | DIRECTIVE_STR_S_F | DIRECTIVE_STR_D_F => {
                let q = if matches!(f, STR_S_F | DIRECTIVE_STR_S_F) {
                    b'\''
                } else {
                    b'"'
                };
                let (end, esc, bad) = match f {
                    STR_S_F => (STR_S_END, STR_S_ESC, STR_S_BAD),
                    STR_D_F => (STR_D_END, STR_D_ESC, STR_D_BAD),
                    DIRECTIVE_STR_S_F => (DIRECTIVE_STR_S_END, DIRECTIVE_STR_S_ESC, STR_S_BAD),
                    _ => (DIRECTIVE_STR_D_END, DIRECTIVE_STR_D_ESC, STR_D_BAD),
                };
                // the next quote, backslash or the line's last character
                let mut j = i;
                while j < self.b.len() {
                    let d = self.b[j];
                    if d == q || d == b'\\' {
                        break;
                    }
                    let l = char_len(d);
                    if j + l >= self.b.len() {
                        break;
                    }
                    j += l;
                }
                if j >= self.b.len() {
                    self.i = self.b.len() + 1;
                    return true;
                }
                self.fill(j);
                let d = self.b[j];
                if d == q {
                    self.tok(j, j + 1, end);
                    self.i = j + 1;
                    return self.pop();
                }
                if d == b'\\' {
                    let e = self.escape_end(j);
                    self.tok(j, e, esc);
                    self.i = e;
                    return true;
                }
                // `((?:[^\\\n])$)`: the line's last character
                let e = j + char_len(d);
                self.tok(j, e, bad);
                self.i = e;
                self.pop()
            }
            TPL_F | TPL_TYPE => {
                let mut j = i;
                while j < self.b.len() && !matches!(self.b[j], b'`' | b'\\' | b'$') {
                    j += 1;
                }
                if j >= self.b.len() {
                    self.i = self.b.len() + 1;
                    return true;
                }
                match self.b[j] {
                    b'`' => {
                        self.tok(j, j + 1, TPL_END);
                        self.i = j + 1;
                        self.pop()
                    }
                    b'\\' => {
                        let e = self.escape_end(j);
                        self.tok(j, e, TPL_ESC);
                        self.i = e;
                        true
                    }
                    _ if self.c(j + 1) == b'{' => {
                        self.tok(j, j + 2, SUBST_BEGIN);
                        self.i = j + 2;
                        self.push(if f == TPL_F { SUBST } else { SUBST_TYPE });
                        true
                    }
                    _ => {
                        self.i = j + 1;
                        true
                    }
                }
            }
            BLOCK_F => match find2(self.b, i, b'*', b'/') {
                Some(j) => {
                    self.tok(j, j + 2, BLOCK_MARK);
                    self.i = j + 2;
                    self.pop()
                }
                None => {
                    self.i = self.b.len() + 1;
                    true
                }
            },
            DOC_F => {
                if c == b'*' && self.c(i + 1) == b'/' {
                    self.tok(i, i + 2, DOC_MARK);
                    self.i = i + 2;
                    return self.pop();
                }
                if c == b'@' || c == b'[' || c == b'{' {
                    return self.docblock();
                }
                // to the next place a pattern can start
                let mut j = i + 1;
                while j < self.b.len() && !matches!(self.b[j], b'*' | b'@' | b'[' | b'{') {
                    j += 1;
                }
                self.i = j;
                true
            }
            REGEX_F => {
                if c == b'/' && self.re == 0 {
                    let mut e = i + 1;
                    while matches!(
                        self.c(e),
                        b'd' | b'g' | b'i' | b'm' | b's' | b'u' | b'v' | b'y'
                    ) {
                        e += 1;
                    }
                    self.tok(i, i + 1, RE_END);
                    self.tok(i + 1, e, RE_FLAGS);
                    self.i = e;
                    return self.pop();
                }
                self.regex_run(0, self.b.len())
            }
            RE_STR_F => self.regex_string(),
            RE_GROUP_F | RE_SET_F => {
                self.unspill();
                true
            }
            LINE_EOL => {
                self.i = self.b.len() + 1;
                true
            }
            DOC_EXAMPLE => {
                if c == b'@' || (c == b'*' && self.c(i + 1) == b'/') {
                    return self.pop_zero();
                }
                // ^\s\*\s+
                if i == 0 && is_space(c) && self.c(1) == b'*' && is_space(self.c(2)) {
                    self.i = self.ws(2).min(self.b.len());
                    return true;
                }
                // \G(<)caption(>): its failed search is cached for the rest of the line
                if !self.g_caption && !self.has(i, b"<caption>") {
                    self.g_caption = true;
                }
                if !self.g_caption {
                    self.tok(i, i + 1, JD_CAPTION_ANGLE_BEGIN);
                    self.tok(i + 1, i + 8, JD_CAPTION_TAG);
                    self.tok(i + 8, i + 9, JD_CAPTION_ANGLE_END);
                    self.i = i + 9;
                    self.push(DOC_CAPTION);
                    return true;
                }
                if i < self.b.len() && !is_space(c) && c != b'@' && c != b'*' {
                    // [^\s@*](?:[^*]|\*[^/])*
                    let mut e = i + 1;
                    while e < self.b.len() {
                        if self.b[e] == b'*' {
                            if self.c(e + 1) == b'/' || e + 1 >= self.b.len() {
                                break;
                            }
                            e += 2;
                        } else {
                            e += 1;
                        }
                    }
                    self.tok(i, e.min(self.b.len()), JD_EXAMPLE_CODE);
                    self.i = e.min(self.b.len());
                    return true;
                }
                false
            }
            DOC_CAPTION => {
                if self.has(i, b"</caption>") {
                    self.tok(i, i + 2, JD_CAPTION_ANGLE_BEGIN);
                    self.tok(i + 2, i + 9, JD_CAPTION_TAG);
                    self.tok(i + 9, i + 10, JD_CAPTION_ANGLE_END);
                    self.i = i + 10;
                    return self.pop();
                }
                if c == b'*' && self.c(i + 1) == b'/' {
                    return self.pop_zero();
                }
                false
            }
            DOC_TYPE => {
                // ((}))\s*|(?=\*/)
                if c == b'}' {
                    // `((}))\s*`: the blanks are the pop's, outside the type's content scope
                    self.tok(i, i + 1, JD_TYPE_END);
                    self.i = i + 1;
                    self.pop();
                    self.i = self.ws(i + 1).min(self.b.len());
                    return true;
                }
                if c == b'*' && self.c(i + 1) == b'/' {
                    return self.pop_zero();
                }
                self.doc_brackets()
            }
            DOC_BRACKETS | DOC_BRACKETS_SQ => {
                let close = if f == DOC_BRACKETS { b'}' } else { b']' };
                if c == close {
                    self.i = i + 1;
                    return self.pop();
                }
                if c == b'*' && self.c(i + 1) == b'/' {
                    return self.pop_zero();
                }
                self.doc_brackets()
            }
            DOC_TYPEDEF | DOC_PARAM | DOC_TYPED => {
                // (?=\s|\*/|[^{}\[\]A-Za-z_$])
                if i >= self.b.len()
                    || is_space(c)
                    || (c == b'*' && self.c(i + 1) == b'/')
                    || !matches!(c, b'{' | b'}' | b'[' | b']' | b'A'..=b'Z' | b'a'..=b'z' | b'_' | b'$')
                {
                    return self.pop_zero();
                }
                if self.jsdoctype() {
                    return true;
                }
                match f {
                    DOC_TYPEDEF => {
                        // (?:[^@\s*/]|\*[^/])+
                        let e = doc_run(self.b, i, b"@/");
                        if e > i {
                            return self.take(i, e, JD_TYPE);
                        }
                        false
                    }
                    DOC_PARAM => {
                        if matches!(c, b'A'..=b'Z' | b'a'..=b'z' | b'_' | b'$') {
                            let mut e = i + 1;
                            while matches!(self.c(e), b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'$' | b'.' | b'[' | b']')
                            {
                                e += 1;
                            }
                            return self.take(i, e, JD_VAR);
                        }
                        if c == b'[' {
                            return self.doc_optional();
                        }
                        false
                    }
                    _ => false,
                }
            }
            DOC_STRING_S | DOC_STRING_D => {
                let q = if f == DOC_STRING_S { b'\'' } else { b'"' };
                if c == q {
                    self.tok(i, i + 1, JD_STR_END);
                    self.i = i + 1;
                    return self.pop();
                }
                if i >= self.b.len() || (c == b'*' && self.c(i + 1) == b'/') {
                    return self.pop_zero();
                }
                false
            }
            DOC_INLINE => {
                // }|(?=\*/)
                if c == b'}' {
                    self.tok(i, i + 1, JD_INLINE_END);
                    self.i = i + 1;
                    return self.pop();
                }
                if c == b'*' && self.c(i + 1) == b'/' {
                    return self.pop_zero();
                }
                self.inline_g()
            }
            DIRECTIVE_F => {
                if i >= self.b.len() {
                    return self.pop_zero();
                }
                if c == b'<' {
                    for w in [&b"reference"[..], b"amd-dependency", b"amd-module"] {
                        if self.has(i + 1, w) {
                            self.tok(i, i + 1, DIRECTIVE_TAG_MARK);
                            self.tok(i + 1, i + 1 + w.len(), DIRECTIVE_TAG);
                            self.i = i + 1 + w.len();
                            self.push(DIRECTIVE_TAG_F);
                            return true;
                        }
                    }
                }
                false
            }
            DIRECTIVE_TAG_F => {
                if c == b'/' && self.c(i + 1) == b'>' {
                    self.tok(i, i + 2, DIRECTIVE_TAG_MARK);
                    self.i = i + 2;
                    return self.pop();
                }
                for w in [&b"path"[..], b"types", b"no-default-lib", b"lib", b"name"] {
                    if self.has(i, w) {
                        return self.take(i, i + w.len(), DIRECTIVE_ATTR);
                    }
                }
                match c {
                    b'=' => self.take(i, i + 1, DIRECTIVE_EQ),
                    b'\'' => {
                        self.tok(i, i + 1, DIRECTIVE_STR_S_BEGIN);
                        self.i = i + 1;
                        self.push(DIRECTIVE_STR_S_F);
                        true
                    }
                    b'"' => {
                        self.tok(i, i + 1, DIRECTIVE_STR_D_BEGIN);
                        self.i = i + 1;
                        self.push(DIRECTIVE_STR_D_F);
                        true
                    }
                    _ => false,
                }
            }
            IF_REGEX => false,
            _ => false,
        }
    }

    /// `docblock`'s patterns at the place in a `/** … */`: `@`, `[` or `{` there.
    pub(super) fn docblock(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        if c == b'[' || c == b'{' {
            return self.inline_tags();
        }
        // a tag's name
        let s = i + 1;
        let mut e = s;
        while self.c(e).is_ascii_alphanumeric() || self.c(e) == b'_' || self.c(e) == b'$' {
            e += 1;
        }
        let tag = &self.b[s..e.min(self.b.len())];
        let after = self.ws(e);
        let gap = after > e;
        let at_word = |w: &[u8]| tag == w;
        let tag_tok = |me: &mut Self, e2: usize| {
            me.tok(i, i + 1, JD_TAG_AT);
            me.tok(i + 1, e2, JD_TAG);
        };
        // ((@)(?:access|api))\s+(private|protected|public)\b
        if (at_word(b"access") || at_word(b"api")) && gap {
            for w in [&b"private"[..], b"protected", b"public"] {
                if let Some(we) = self.word_b(after, w) {
                    tag_tok(self, e);
                    return self.take(after, we, JD_ACCESS);
                }
            }
        }
        // ((@)author)\s+([^@\s<>*/](?:[^@<>*/]|\*[^/])*)(?:\s*(<)([^>\s]+)(>))?
        if at_word(b"author")
            && gap
            && after < self.b.len()
            && !matches!(self.c(after), b'@' | b'<' | b'>' | b'*' | b'/')
        {
            let mut k = after + 1;
            while k < self.b.len() {
                match self.b[k] {
                    b'@' | b'<' | b'>' | b'/' => break,
                    b'*' if self.c(k + 1) == b'/' || k + 1 >= self.b.len() => break,
                    b'*' => k += 2,
                    _ => k += 1,
                }
            }
            let k = k.min(self.b.len());
            tag_tok(self, e);
            self.tok(after, k, JD_TYPE);
            self.i = k;
            let m = self.ws(k);
            if self.c(m) == b'<' {
                let mut q = m + 1;
                while q < self.b.len() && self.b[q] != b'>' && !is_space(self.b[q]) {
                    q += 1;
                }
                if q > m + 1 && self.c(q) == b'>' {
                    self.tok(m, m + 1, JD_ANGLE_BEGIN);
                    self.tok(m + 1, q, JD_EMAIL);
                    self.tok(q, q + 1, JD_ANGLE_END);
                    self.i = q + 1;
                }
            }
            return true;
        }
        // ((@)borrows)\s+(namepath)\s+(as)\s+(namepath)
        if at_word(b"borrows") && gap {
            let k = doc_run(self.b, after, b"@/");
            let m = self.ws(k);
            if k > after
                && m > k
                && let Some(ae) = self.word_b(m, b"as")
            {
                let q = self.ws(ae);
                let r = doc_run(self.b, q, b"@/");
                if q > ae && r > q {
                    tag_tok(self, e);
                    self.tok(after, k, JD_TYPE);
                    self.tok(m, ae, JD_AS);
                    return self.take(q, r, JD_TYPE);
                }
            }
        }
        if at_word(b"example") && gap {
            tag_tok(self, e);
            self.i = after.min(self.b.len());
            self.push(DOC_EXAMPLE);
            return true;
        }
        // @kind
        if at_word(b"kind") && gap {
            for w in [
                &b"class"[..],
                b"constant",
                b"event",
                b"external",
                b"file",
                b"function",
                b"member",
                b"mixin",
                b"module",
                b"namespace",
                b"typedef",
            ] {
                if let Some(we) = self.word_b(after, w) {
                    tag_tok(self, e);
                    return self.take(after, we, JD_SYMBOL);
                }
            }
        }
        // @template A, B
        if at_word(b"template")
            && gap
            && matches!(self.c(after), b'A'..=b'Z' | b'a'..=b'z' | b'_' | b'$')
        {
            let mut k = after;
            loop {
                k += 1;
                while matches!(self.c(k), b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'$' | b'.' | b'[' | b']')
                {
                    k += 1;
                }
                let m = self.ws(k);
                if self.c(m) == b',' {
                    let m2 = self.ws(m + 1);
                    if matches!(self.c(m2), b'A'..=b'Z' | b'a'..=b'z' | b'_' | b'$') {
                        k = m2;
                        continue;
                    }
                }
                break;
            }
            tag_tok(self, e);
            return self.take(after, k.min(self.b.len()), JD_VAR);
        }
        let param_like = [
            &b"arg"[..],
            b"argument",
            b"const",
            b"constant",
            b"member",
            b"namespace",
            b"param",
            b"var",
        ];
        if param_like.contains(&tag)
            && gap
            && matches!(self.c(after), b'A'..=b'Z' | b'a'..=b'z' | b'_' | b'$')
        {
            let mut k = after + 1;
            while matches!(self.c(k), b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'$' | b'.' | b'[' | b']')
            {
                k += 1;
            }
            tag_tok(self, e);
            return self.take(after, k.min(self.b.len()), JD_VAR);
        }
        if at_word(b"typedef") && gap && self.c(after) == b'{' {
            tag_tok(self, e);
            self.i = after;
            self.push(DOC_TYPEDEF);
            return true;
        }
        let param_typed = [
            &b"arg"[..],
            b"argument",
            b"const",
            b"constant",
            b"member",
            b"namespace",
            b"param",
            b"prop",
            b"property",
            b"var",
        ];
        if param_typed.contains(&tag) && gap && self.c(after) == b'{' {
            tag_tok(self, e);
            self.i = after;
            self.push(DOC_PARAM);
            return true;
        }
        let typed = [
            &b"define"[..],
            b"enum",
            b"exception",
            b"export",
            b"extends",
            b"lends",
            b"implements",
            b"modifies",
            b"namespace",
            b"private",
            b"protected",
            b"returns",
            b"return",
            b"suppress",
            b"this",
            b"throws",
            b"type",
            b"yields",
            b"yield",
        ];
        if typed.contains(&tag) && gap && self.c(after) == b'{' {
            tag_tok(self, e);
            self.i = after;
            self.push(DOC_TYPED);
            return true;
        }
        let named = [
            &b"alias"[..],
            b"augments",
            b"callback",
            b"constructs",
            b"emits",
            b"event",
            b"fires",
            b"exports",
            b"export",
            b"extends",
            b"external",
            b"function",
            b"func",
            b"host",
            b"lends",
            b"listens",
            b"interface",
            b"memberof",
            b"memberof!",
            b"method",
            b"module",
            b"mixes",
            b"mixin",
            b"name",
            b"requires",
            b"see",
            b"this",
            b"typedef",
            b"uses",
        ];
        let tag_x = if self.c(e) == b'!' && tag == b"memberof" {
            &self.b[s..e + 1]
        } else {
            tag
        };
        let e_x = s + tag_x.len();
        let after_x = self.ws(e_x);
        if named.contains(&tag_x) && after_x > e_x {
            // (?:[^{}@\s*]|\*[^/])+
            let k = doc_run(self.b, after_x, b"{}@");
            if k > after_x {
                if tag_x == b"see" {
                    // @see's own pattern comes first: a URL, or a namepath not before {@link
                } else {
                    tag_tok(self, e_x);
                    return self.take(after_x, k, JD_TYPE);
                }
            }
        }
        if at_word(b"see") && gap {
            let r = self.rest(after);
            if crate::starts(r, b"http://") || crate::starts(r, b"https://") {
                let k = doc_run(self.b, after, b"");
                tag_tok(self, e);
                return self.take(after, k, JD_LINK);
            }
            let k = doc_run(self.b, after, b"@/");
            // not before an inline tag, a `[description]` before it or not
            let inline = inline_tag_len(r).is_some()
                || (crate::starts(r, b"[") && {
                    let e = r.iter().position(|&c| c == b']');
                    e.is_some_and(|e| {
                        !r[1..e].contains(&b'[') && inline_tag_len(&r[e + 1..]).is_some()
                    })
                });
            if k > after && !inline {
                tag_tok(self, e);
                return self.take(after, k, JD_TYPE);
            }
        }
        // @default '…'
        if (at_word(b"default")
            || at_word(b"defaultvalue")
            || at_word(b"license")
            || at_word(b"version"))
            && gap
            && matches!(self.c(after), b'\'' | b'"')
        {
            tag_tok(self, e);
            self.tok(after, after + 1, JD_STR_BEGIN);
            self.i = after + 1;
            self.push(if self.c(after) == b'\'' {
                DOC_STRING_S
            } else {
                DOC_STRING_D
            });
            return true;
        }
        if [
            &b"default"[..],
            b"defaultvalue",
            b"license",
            b"tutorial",
            b"variation",
            b"version",
        ]
        .contains(&tag)
            && gap
        {
            let mut k = after;
            while k < self.b.len() && !is_space(self.b[k]) && self.b[k] != b'*' {
                k += 1;
            }
            if k > after {
                tag_tok(self, e);
                return self.take(after, k, JD_VAR);
            }
        }
        // (@)(?:abstract|…)\b: `$` is no word character, so `@return$s` is `@return` and `$s`
        let mut w = s;
        while self.c(w).is_ascii_alphanumeric() || self.c(w) == b'_' {
            w += 1;
        }
        if doc_tag(&self.b[s..w.min(self.b.len())]) && self.boundary(w) {
            tag_tok(self, w);
            self.i = w;
            return true;
        }
        // ((@)(?:[_$[:alpha:]][_$[:alnum:]]*))(?=\s+)
        let ne = self.name(s);
        if ne > s && is_space(self.c(ne)) {
            tag_tok(self, ne);
            self.i = ne;
            return true;
        }
        false
    }

    /// `inline-tags` at `[` or `{`.
    pub(super) fn inline_tags(&mut self) -> bool {
        let i = self.i;
        if self.c(i) == b'[' {
            // (\[)[^\]]+(\])(?={@(?:link|linkcode|linkplain|tutorial))
            let mut e = i + 1;
            while e < self.b.len() && self.b[e] != b']' {
                e += 1;
            }
            if e > i + 1 && e < self.b.len() && inline_tag_len(self.rest(e + 1)).is_some() {
                self.tok(i, i + 1, JD_DESC_BEGIN);
                self.tok(i + 1, e, JD_DESC);
                return self.take(e, e + 1, JD_DESC_END);
            }
            return false;
        }
        let Some(len) = inline_tag_len(self.rest(i)) else {
            return false;
        };
        let e = i + 2 + len;
        self.tok(i, i + 1, JD_INLINE_BEGIN);
        self.tok(i + 1, i + 2, JD_INLINE_TAG_AT);
        self.tok(i + 2, e, JD_INLINE_TAG);
        let k = self.ws(e).min(self.b.len());
        // the blanks after the tag are the push's, under the inline tag's scope
        self.i = e;
        self.push(DOC_INLINE);
        self.i = k;
        true
    }

    /// An inline tag's `\G` patterns at the place: a link or a description, then `|`. syntect
    /// caches a pattern's failed search for the rest of the line, and a `\G` pattern's search
    /// fails wherever it starts but at its match: so once one fails on a line, it doesn't match
    /// again on it ([`Scan::g_link`], [`Scan::g_desc`]).
    pub(super) fn inline_g(&mut self) -> bool {
        let k = self.i;
        let pipe = |me: &mut Self, j: usize| {
            me.i = j;
            if me.c(j) == b'|' {
                me.tok(j, j + 1, JD_INLINE_PIPE);
                me.i = j + 1;
            }
            true
        };
        if !self.g_link {
            let r = self.rest(k);
            let mut j = k;
            if crate::starts(r, b"http://") || crate::starts(r, b"https://") {
                while j < self.b.len() {
                    let d = self.b[j];
                    if d == b'|' || d == b'}' || is_space(d) {
                        break;
                    }
                    if d == b'*' {
                        if self.c(j + 1) == b'/' {
                            j += 2;
                            continue;
                        }
                        break;
                    }
                    j += 1;
                }
            }
            if j > k {
                self.tok(k, j, JD_INLINE_LINK);
                return pipe(self, j);
            }
            self.g_link = true;
        }
        if !self.g_desc {
            let mut j = k;
            while j < self.b.len() {
                let d = self.b[j];
                if matches!(d, b'{' | b'}' | b'@' | b'|') || is_space(d) {
                    break;
                }
                if d == b'*' {
                    if self.c(j + 1) != b'/' && j + 1 < self.b.len() {
                        j += 2;
                        continue;
                    }
                    break;
                }
                j += 1;
            }
            if j > k {
                self.tok(k, j, JD_INLINE_DESC);
                return pipe(self, j);
            }
            self.g_desc = true;
        }
        false
    }

    /// `jsdoctype` at `{`.
    ///
    /// syntect caches a pattern's failed search for the rest of the line, and the cache is the
    /// pattern's, shared by the three tag contexts that include `jsdoctype`: a `\G` pattern
    /// searched where it doesn't match (after a type's `}`, at the description) doesn't match
    /// again on the line, so a second `@type {T}` on it has no type (as [`Scan::inline_g`]).
    /// Reached only where the tag's pop didn't match, so both were searched.
    pub(super) fn jsdoctype(&mut self) -> bool {
        let i = self.i;
        if self.c(i) != b'{' {
            self.g_type_open = true;
            self.g_type_brace = true;
            return false;
        }
        if !self.g_type_open {
            if let Some(e) = self.jsdoctype_open(i) {
                return self.take(i, e, JD_TYPE_BAD);
            }
            self.g_type_open = true;
        }
        if self.g_type_brace {
            return false;
        }
        self.tok(i, i + 1, JD_TYPE_BEGIN);
        self.i = i + 1;
        self.push(DOC_TYPE);
        true
    }

    /// `\G{(?:[^}*]|\*[^/}])+$` at the `{` at `i`: the line's end if it matches.
    fn jsdoctype_open(&self, i: usize) -> Option<usize> {
        // \G{(?:[^}*]|\*[^/}])+$: a type left open at the line's end. syntect's line ends in `\n`,
        // which the class takes: a `{` or `*` last on the line is open too (`{\n`, `{x*\n`)
        let mut j = i + 1;
        while j < self.b.len() {
            match self.b[j] {
                b'}' => break,
                b'*' if j + 1 < self.b.len() && matches!(self.b[j + 1], b'/' | b'}') => break,
                b'*' => j += 2,
                _ => j += 1,
            }
        }
        (j >= self.b.len()).then_some(self.b.len())
    }

    /// `brackets`, in a JSDoc type.
    pub(super) fn doc_brackets(&mut self) -> bool {
        match self.c(self.i) {
            b'{' => {
                self.i += 1;
                self.push(DOC_BRACKETS);
                true
            }
            b'[' => {
                self.i += 1;
                self.push(DOC_BRACKETS_SQ);
                true
            }
            _ => false,
        }
    }

    /// A parameter's optional value, `[name=value]`, in a JSDoc tag.
    pub(super) fn doc_optional(&mut self) -> bool {
        let i = self.i;
        let mut j = self.ws(i + 1);
        let s = j;
        while self.c(j).is_ascii_alphanumeric() || matches!(self.c(j), b'_' | b'$') {
            j += 1;
        }
        if j == s {
            return false;
        }
        loop {
            let mut k = j;
            if self.has(k, b"[]") {
                k += 2;
            }
            if self.c(k) == b'.'
                && (self.c(k + 1).is_ascii_alphanumeric() || matches!(self.c(k + 1), b'_' | b'$'))
            {
                k += 1;
                while self.c(k).is_ascii_alphanumeric() || matches!(self.c(k), b'_' | b'$') {
                    k += 1;
                }
                j = k;
                continue;
            }
            break;
        }
        let mut eq = None;
        let k = self.ws(j);
        let mut end = k;
        if self.c(k) == b'=' {
            let v = self.ws(k + 1);
            let mut e = v;
            let mut depth = 0;
            while e < self.b.len() {
                let d = self.b[e];
                if d == b'*' && self.c(e + 1) == b'/' {
                    break;
                }
                if d == b'[' {
                    depth += 1;
                } else if d == b']' {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                e += 1;
            }
            let mut ve = e;
            while ve > v && is_space(self.b[ve - 1]) {
                ve -= 1;
            }
            eq = Some((k, v, ve));
            end = e;
        }
        let close = self.c(end) == b']';
        if !close && !(self.c(end) == b'*' && self.c(end + 1) == b'/') {
            return false;
        }
        self.tok(i, i + 1, JD_OPT_BEGIN);
        if let Some((k, v, ve)) = eq {
            self.tok(i + 1, k, JD_VAR);
            self.tok(k, k + 1, JD_OPT_EQ);
            self.tok(k + 1, v, JD_VAR);
            self.tok(v, ve, JD_OPT_VALUE);
            self.tok(ve, end, JD_VAR);
        } else {
            self.tok(i + 1, end, JD_VAR);
        }
        if close {
            self.tok(end, end + 1, JD_OPT_END);
            let bad = doc_run(self.b, end + 1, b"");
            if bad > end + 1 {
                self.tok(end + 1, bad, JD_OPT_BAD);
                self.i = bad;
                return true;
            }
            self.i = end + 1;
        } else {
            self.i = end;
        }
        true
    }
}

/// `{@link`, `{@linkcode`, `{@linkplain` or `{@tutorial` at the start of `r`: the tag's length,
/// `link(?:code|plain)?|tutorial` with no `\b` after it (`{@linkn` is `link` and `n`).
fn inline_tag_len(r: &[u8]) -> Option<usize> {
    let r = r.strip_prefix(b"{@")?;
    if crate::starts(r, b"linkcode") {
        Some(8)
    } else if crate::starts(r, b"linkplain") {
        Some(9)
    } else if crate::starts(r, b"link") {
        Some(4)
    } else if crate::starts(r, b"tutorial") {
        Some(8)
    } else {
        None
    }
}

/// `(?:[^<stop>\s*]|\*[^/])+` from `i`, with the extra stops in `stop`: its end.
fn doc_run(b: &[u8], mut i: usize, stop: &[u8]) -> usize {
    while i < b.len() {
        let c = b[i];
        if is_space(c) || stop.contains(&c) {
            break;
        }
        if c == b'*' {
            if i + 1 < b.len() && b[i + 1] != b'/' && !is_space(b[i + 1]) {
                i += 2;
                continue;
            }
            break;
        }
        i += 1;
    }
    i
}

/// The tags of `docblock`'s list of plain ones.
fn doc_tag(t: &[u8]) -> bool {
    matches!(
        t,
        b"abstract"
            | b"access"
            | b"alias"
            | b"api"
            | b"arg"
            | b"argument"
            | b"async"
            | b"attribute"
            | b"augments"
            | b"author"
            | b"beta"
            | b"borrows"
            | b"bubbles"
            | b"callback"
            | b"chainable"
            | b"class"
            | b"classdesc"
            | b"code"
            | b"config"
            | b"const"
            | b"constant"
            | b"constructor"
            | b"constructs"
            | b"copyright"
            | b"default"
            | b"defaultvalue"
            | b"define"
            | b"deprecated"
            | b"desc"
            | b"description"
            | b"dict"
            | b"emits"
            | b"enum"
            | b"event"
            | b"example"
            | b"exception"
            | b"exports"
            | b"export"
            | b"extends"
            | b"extension"
            | b"extension_for"
            | b"extensionfor"
            | b"external"
            | b"externs"
            | b"file"
            | b"fileoverview"
            | b"final"
            | b"fires"
            | b"for"
            | b"func"
            | b"function"
            | b"generator"
            | b"global"
            | b"hideconstructor"
            | b"host"
            | b"ignore"
            | b"implements"
            | b"implicitCast"
            | b"inheritDoc"
            | b"inheritdoc"
            | b"inner"
            | b"instance"
            | b"interface"
            | b"internal"
            | b"kind"
            | b"lends"
            | b"license"
            | b"listens"
            | b"main"
            | b"member"
            | b"memberof"
            | b"method"
            | b"mixes"
            | b"mixin"
            | b"mixins"
            | b"modifies"
            | b"module"
            | b"name"
            | b"namespace"
            | b"noalias"
            | b"nocollapse"
            | b"nocompile"
            | b"nosideeffects"
            | b"override"
            | b"overview"
            | b"package"
            | b"param"
            | b"polymer"
            | b"polymerBehavior"
            | b"preserve"
            | b"private"
            | b"prop"
            | b"property"
            | b"protected"
            | b"public"
            | b"readOnly"
            | b"readonly"
            | b"record"
            | b"required"
            | b"requires"
            | b"returns"
            | b"return"
            | b"see"
            | b"since"
            | b"static"
            | b"struct"
            | b"submodule"
            | b"summary"
            | b"suppress"
            | b"template"
            | b"this"
            | b"throws"
            | b"todo"
            | b"tutorial"
            | b"type"
            | b"typedef"
            | b"unrestricted"
            | b"uses"
            | b"var"
            | b"variation"
            | b"version"
            | b"virtual"
            | b"writeOnce"
            | b"yields"
            | b"yield"
    )
}

/// The first `a` `b` pair in `s` from `i`.
fn find2(s: &[u8], i: usize, a: u8, b: u8) -> Option<usize> {
    let mut j = i;
    while j + 1 < s.len() {
        if s[j] == a && s[j + 1] == b {
            return Some(j);
        }
        j += 1;
    }
    None
}

/// `string-character-escape` at `p` in line `b`, `\` there: its end (a `\` ending the line is
/// one, its `\n` the next line's continuation).
fn escape_at(b: &[u8], p: usize) -> usize {
    let n = b.len();
    let c = |k: usize| match k.cmp(&n) {
        std::cmp::Ordering::Less => b[k],
        std::cmp::Ordering::Equal => b'\n',
        std::cmp::Ordering::Greater => 0,
    };
    let hex = |k: usize| c(k).is_ascii_hexdigit();
    let oct = |k: usize| matches!(c(k), b'0'..=b'7');
    match c(p + 1) {
        b'x' if hex(p + 2) && hex(p + 3) => p + 4,
        b'u' if (2..6).all(|k| hex(p + k)) => p + 6,
        b'u' if c(p + 2) == b'{' && hex(p + 3) => {
            let mut k = p + 3;
            while hex(k) {
                k += 1;
            }
            if c(k) == b'}' { k + 1 } else { p + 2 }
        }
        b'0'..=b'2' => {
            let mut k = p + 2;
            while k < p + 4 && oct(k) {
                k += 1;
            }
            k
        }
        b'3' => match c(p + 2) {
            b'0'..=b'7' => p + 3 + usize::from(oct(p + 3)),
            _ => p + 2,
        },
        b'4'..=b'7' => p + 2 + usize::from(oct(p + 2)),
        b'\n' => p + 1,
        d => (p + 1 + char_len(d)).min(n),
    }
}

/// Whether a `(` is right before `p`, or one blank and a `(` (`RegExp( "…")`): where a string may
/// be `RegExp(`'s argument, tested inline before [`Scan::regexp_arg`] reads further back.
#[inline]
fn after_paren(b: &[u8], p: usize) -> bool {
    match p {
        0 => false,
        1 => b[0] == b'(',
        _ => b[p - 1] == b'(' || (b[p - 1] == b' ' && b[p - 2] == b'('),
    }
}
