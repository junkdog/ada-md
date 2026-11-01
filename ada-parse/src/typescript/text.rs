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
                if self.rest(k).starts_with(b"@internal") {
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
                if self.rest(k).starts_with(b"@internal") {
                    let f = k + 9;
                    if f >= self.n || is_space(self.c(f)) {
                        self.tok(e, k, LINE);
                        self.tok(k, k + 1, LINE_INTERNAL_MARK);
                        self.tok(k + 1, f, LINE_INTERNAL);
                        e = f;
                    }
                }
                self.tok(e, self.n, LINE);
                self.i = if consume_eol { self.n + 1 } else { self.n };
                true
            }
            _ => false,
        }
    }

    /// `string`: `'…'`, `"…"`, or a template (`template-call` first: a tag's `true` is the literal).
    pub(super) fn string(&mut self) -> bool {
        match self.c(self.i) {
            b'\'' => {
                self.tok(self.i, self.i + 1, STR_S_BEGIN);
                self.i += 1;
                self.push(STR_S_F);
                true
            }
            b'"' => {
                self.tok(self.i, self.i + 1, STR_D_BEGIN);
                self.i += 1;
                self.push(STR_D_F);
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
        true
    }

    /// `string-character-escape` at the place, `\` there: its end.
    pub(super) fn escape_end(&self, i: usize) -> usize {
        let c = self.c(i + 1);
        let hex = |k: usize| self.c(k).is_ascii_hexdigit();
        let oct = |k: usize| matches!(self.c(k), b'0'..=b'7');
        match c {
            b'x' if hex(i + 2) && hex(i + 3) => i + 4,
            b'u' if (2..6).all(|k| hex(i + k)) => i + 6,
            b'u' if self.c(i + 2) == b'{' && hex(i + 3) => {
                let mut k = i + 3;
                while hex(k) {
                    k += 1;
                }
                if self.c(k) == b'}' { k + 1 } else { i + 2 }
            }
            b'0'..=b'2' => {
                let mut k = i + 2;
                while k < i + 4 && oct(k) {
                    k += 1;
                }
                k
            }
            b'3' => match self.c(i + 2) {
                b'0'..=b'6' => i + 3 + usize::from(oct(i + 3)),
                b'7' => i + 3 + usize::from(oct(i + 3)),
                _ => i + 2,
            },
            b'4'..=b'7' => i + 2 + usize::from(oct(i + 2)),
            b'\n' => i + 1,
            c => (i + 1 + char_len(c)).min(self.n),
        }
    }

    /// A frame of text's turn.
    pub(super) fn text_step(&mut self, f: u8) -> bool {
        let i = self.i;
        let c = self.c(i);
        match f {
            STR_S_F | STR_D_F | DIRECTIVE_STR_S_F | DIRECTIVE_STR_D_F => {
                let q = if matches!(f, STR_S_F | DIRECTIVE_STR_S_F) { b'\'' } else { b'"' };
                let (end, esc, bad) = match f {
                    STR_S_F => (STR_S_END, STR_S_ESC, STR_S_BAD),
                    STR_D_F => (STR_D_END, STR_D_ESC, STR_D_BAD),
                    DIRECTIVE_STR_S_F => (DIRECTIVE_STR_S_END, DIRECTIVE_STR_S_ESC, STR_S_BAD),
                    _ => (DIRECTIVE_STR_D_END, DIRECTIVE_STR_D_ESC, STR_D_BAD),
                };
                // the next quote, backslash or the line's last character
                let mut j = i;
                while j < self.n {
                    let d = self.b[j];
                    if d == q || d == b'\\' {
                        break;
                    }
                    let l = char_len(d);
                    if j + l >= self.n {
                        break;
                    }
                    j += l;
                }
                if j >= self.n {
                    self.i = self.n + 1;
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
                while j < self.n && !matches!(self.b[j], b'`' | b'\\' | b'$') {
                    j += 1;
                }
                if j >= self.n {
                    self.i = self.n + 1;
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
            BLOCK_F => {
                match find2(self.b, i, b'*', b'/') {
                    Some(j) => {
                        self.tok(j, j + 2, BLOCK_MARK);
                        self.i = j + 2;
                        self.pop()
                    }
                    None => {
                        self.i = self.n + 1;
                        true
                    }
                }
            }
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
                while j < self.n && !matches!(self.b[j], b'*' | b'@' | b'[' | b'{') {
                    j += 1;
                }
                self.i = j;
                true
            }
            REGEX_F | RE_GROUP_F | RE_ASSERT_F => {
                if f == REGEX_F && c == b'/' {
                    let mut e = i + 1;
                    while matches!(self.c(e), b'd' | b'g' | b'i' | b'm' | b's' | b'u' | b'y') {
                        e += 1;
                    }
                    self.tok(i, i + 1, RE_END);
                    self.tok(i + 1, e, RE_FLAGS);
                    self.i = e;
                    return self.pop();
                }
                if f != REGEX_F && c == b')' {
                    self.tok(i, i + 1, RE_GROUP);
                    self.i = i + 1;
                    return self.pop();
                }
                self.regexp()
            }
            RE_SET_F => {
                if c == b']' {
                    self.tok(i, i + 1, RE_SET_MARK);
                    self.i = i + 1;
                    return self.pop();
                }
                if i >= self.n {
                    return false;
                }
                // a range: (?:.|(\\num)|(\\c[A-Z])|(\\.))\-(?:[^\]\\]|…): the left side is any
                // one character first, a `\` too
                let plain = (i + char_len(c), None);
                let left = if self.c(plain.0) == b'-' && self.re_set_atom(plain.0 + 1, false).is_some() {
                    Some(plain)
                } else {
                    self.re_set_atom(i, true)
                };
                if let Some((e1, k1)) = left
                    && self.c(e1) == b'-'
                    && let Some((e2, k2)) = self.re_set_atom(e1 + 1, false)
                {
                    self.tok(i, e1, k1.unwrap_or(RE_RANGE));
                    self.tok(e1, e1 + 1, RE_RANGE);
                    self.tok(e1 + 1, e2, k2.unwrap_or(RE_RANGE));
                    self.i = e2;
                    return true;
                }
                self.re_char_class(true)
            }
            LINE_EOL => {
                self.i = self.n + 1;
                true
            }
            DOC_EXAMPLE => {
                if c == b'@' || (c == b'*' && self.c(i + 1) == b'/') {
                    return self.pop_zero();
                }
                // ^\s\*\s+
                if i == 0 && is_space(c) && self.c(1) == b'*' && is_space(self.c(2)) {
                    self.i = self.ws(2).min(self.n);
                    return true;
                }
                // \G(<)caption(>): its failed search is cached for the rest of the line
                if !self.g_caption && !self.rest(i).starts_with(b"<caption>") {
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
                if i < self.n && !is_space(c) && c != b'@' && c != b'*' {
                    // [^\s@*](?:[^*]|\*[^/])*
                    let mut e = i + 1;
                    while e < self.n {
                        if self.b[e] == b'*' {
                            if self.c(e + 1) == b'/' || e + 1 >= self.n {
                                break;
                            }
                            e += 2;
                        } else {
                            e += 1;
                        }
                    }
                    self.tok(i, e.min(self.n), JD_EXAMPLE_CODE);
                    self.i = e.min(self.n);
                    return true;
                }
                false
            }
            DOC_CAPTION => {
                if self.rest(i).starts_with(b"</caption>") {
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
                    self.i = self.ws(i + 1).min(self.n);
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
                if i >= self.n
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
                            while matches!(self.c(e), b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'$' | b'.' | b'[' | b']') {
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
                if i >= self.n || (c == b'*' && self.c(i + 1) == b'/') {
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
                if i >= self.n {
                    return self.pop_zero();
                }
                if c == b'<' {
                    for w in [&b"reference"[..], b"amd-dependency", b"amd-module"] {
                        if self.rest(i + 1).starts_with(w) {
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
                    if self.rest(i).starts_with(w) {
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

    /// `regexp`'s patterns at the place, in a regex's body or group.
    pub(super) fn regexp(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        match c {
            b'^' | b'$' => self.take(i, i + 1, RE_ANCHOR),
            b'\\' if matches!(self.c(i + 1), b'b' | b'B') => self.take(i, i + 2, RE_ANCHOR),
            b'\\' if matches!(self.c(i + 1), b'1'..=b'9') => {
                let mut e = i + 2;
                while self.c(e).is_ascii_digit() {
                    e += 1;
                }
                self.take(i, e, RE_BACKREF)
            }
            b'\\' if self.c(i + 1) == b'k' && self.c(i + 2) == b'<' => {
                let s = i + 3;
                let mut e = s;
                if matches!(self.c(e), b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$') {
                    e += 1;
                    while matches!(self.c(e), b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'$') {
                        e += 1;
                    }
                }
                if e > s && self.c(e) == b'>' {
                    self.tok(i, s, RE_BACKREF);
                    self.tok(s, e, RE_BACKREF_NAME);
                    return self.take(e, e + 1, RE_BACKREF);
                }
                self.re_char_class(false)
            }
            b'?' | b'+' | b'*' => self.take(i, i + 1, RE_QUANT),
            b'{' => {
                // \{(\d+,\d+|\d+,|,\d+|\d+)\}\??
                let mut j = i + 1;
                let d1 = j;
                while self.c(j).is_ascii_digit() {
                    j += 1;
                }
                let had1 = j > d1;
                let mut had2 = false;
                if self.c(j) == b',' {
                    j += 1;
                    let d2 = j;
                    while self.c(j).is_ascii_digit() {
                        j += 1;
                    }
                    had2 = j > d2;
                }
                if (had1 || had2) && self.c(j) == b'}' {
                    let mut e = j + 1;
                    if self.c(e) == b'?' {
                        e += 1;
                    }
                    return self.take(i, e, RE_QUANT);
                }
                false
            }
            b'|' => self.take(i, i + 1, RE_OR),
            b'(' => {
                let r = self.rest(i + 1);
                let a = if r.starts_with(b"?<=") {
                    Some((3, RE_LOOKBEHIND))
                } else if r.starts_with(b"?<!") {
                    Some((3, RE_NEG_LOOKBEHIND))
                } else if r.starts_with(b"?=") {
                    Some((2, RE_LOOKAHEAD))
                } else if r.starts_with(b"?!") {
                    Some((2, RE_NEG_LOOKAHEAD))
                } else {
                    None
                };
                if let Some((l, k)) = a {
                    self.tok(i, i + 1, RE_GROUP);
                    self.tok(i + 1, i + 1 + l, k);
                    self.i = i + 1 + l;
                    self.push(RE_ASSERT_F);
                    return true;
                }
                if r.starts_with(b"?:") {
                    self.tok(i, i + 1, RE_GROUP);
                    self.tok(i + 1, i + 3, RE_NO_CAPTURE);
                    self.i = i + 3;
                } else if r.starts_with(b"?<") {
                    let s = i + 3;
                    let mut e = s;
                    if matches!(self.c(e), b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$') {
                        e += 1;
                        while matches!(self.c(e), b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'$') {
                            e += 1;
                        }
                    }
                    if e > s && self.c(e) == b'>' {
                        self.tok(i, s, RE_GROUP);
                        self.tok(s, e, RE_GROUP_NAME);
                        self.tok(e, e + 1, RE_GROUP);
                        self.i = e + 1;
                    } else {
                        self.tok(i, i + 1, RE_GROUP);
                        self.i = i + 1;
                    }
                } else {
                    self.tok(i, i + 1, RE_GROUP);
                    self.i = i + 1;
                }
                self.push(RE_GROUP_F);
                true
            }
            b'[' => {
                self.tok(i, i + 1, RE_SET_MARK);
                let mut e = i + 1;
                if self.c(e) == b'^' {
                    self.tok(e, e + 1, RE_SET_NEG);
                    e += 1;
                }
                self.i = e;
                self.push(RE_SET_F);
                true
            }
            _ => self.re_char_class(false),
        }
    }

    /// `regex-character-class` at the place.
    pub(super) fn re_char_class(&mut self, in_set: bool) -> bool {
        let i = self.i;
        let c = self.c(i);
        let k = |a: u16, b: u16| if in_set { b } else { a };
        if c == b'.' {
            return self.take(i, i + 1, k(RE_CLASS, RE_SET_CLASS));
        }
        if c != b'\\' || i + 1 >= self.n {
            return false;
        }
        let d = self.c(i + 1);
        if matches!(d, b'w' | b'W' | b's' | b'S' | b'd' | b'D' | b't' | b'r' | b'n' | b'v' | b'f') {
            return self.take(i, i + 2, k(RE_CLASS, RE_SET_CLASS));
        }
        if let Some(e) = self.re_numeric(i) {
            return self.take(i, e, k(RE_NUM, RE_SET_NUM));
        }
        if d == b'c' && self.c(i + 2).is_ascii_uppercase() {
            return self.take(i, i + 3, k(RE_CTRL, RE_SET_CTRL));
        }
        let e = (i + 1 + char_len(d)).min(self.n);
        self.take(i, e, k(RE_ESC, RE_SET_ESC))
    }

    /// `\\([0-7]{3}|x[0-9A-Fa-f]{2}|u[0-9A-Fa-f]{4})` at `i`: its end.
    pub(super) fn re_numeric(&self, i: usize) -> Option<usize> {
        let oct = |k: usize| matches!(self.c(k), b'0'..=b'7');
        let hex = |k: usize| self.c(k).is_ascii_hexdigit();
        if (1..4).all(|k| oct(i + k)) || (self.c(i + 1) == b'x' && hex(i + 2) && hex(i + 3)) {
            Some(i + 4)
        } else if self.c(i + 1) == b'u' && (2..6).all(|k| hex(i + k)) {
            Some(i + 6)
        } else {
            None
        }
    }

    /// One side of a class's range at `i`: its end, and the kind of an escape (none for a
    /// plain character). The right side can't be `]` or `\` alone.
    pub(super) fn re_set_atom(&self, i: usize, left: bool) -> Option<(usize, Option<u16>)> {
        let c = self.c(i);
        if i >= self.n {
            return None;
        }
        if c == b'\\' {
            if let Some(e) = self.re_numeric(i) {
                return Some((e, Some(RE_RANGE_NUM)));
            }
            if self.c(i + 1) == b'c' && self.c(i + 2).is_ascii_uppercase() {
                return Some((i + 3, Some(RE_RANGE_CTRL)));
            }
            if i + 1 < self.n {
                return Some((i + 1 + char_len(self.c(i + 1)), Some(RE_RANGE_ESC)));
            }
            return if left { Some((i + 1, None)) } else { None };
        }
        if !left && c == b']' {
            return None;
        }
        Some((i + char_len(c), None))
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
        let tag = &self.b[s..e.min(self.n)];
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
        if at_word(b"author") && gap && after < self.n && !matches!(self.c(after), b'@' | b'<' | b'>' | b'*' | b'/') {
            let mut k = after + 1;
            while k < self.n {
                match self.b[k] {
                    b'@' | b'<' | b'>' | b'/' => break,
                    b'*' if self.c(k + 1) == b'/' || k + 1 >= self.n => break,
                    b'*' => k += 2,
                    _ => k += 1,
                }
            }
            let k = k.min(self.n);
            tag_tok(self, e);
            self.tok(after, k, JD_TYPE);
            self.i = k;
            let m = self.ws(k);
            if self.c(m) == b'<' {
                let mut q = m + 1;
                while q < self.n && self.b[q] != b'>' && !is_space(self.b[q]) {
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
            if k > after && m > k && let Some(ae) = self.word_b(m, b"as") {
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
            self.i = after.min(self.n);
            self.push(DOC_EXAMPLE);
            return true;
        }
        // @kind
        if at_word(b"kind") && gap {
            for w in [&b"class"[..], b"constant", b"event", b"external", b"file", b"function", b"member", b"mixin", b"module", b"namespace", b"typedef"] {
                if let Some(we) = self.word_b(after, w) {
                    tag_tok(self, e);
                    return self.take(after, we, JD_SYMBOL);
                }
            }
        }
        // @template A, B
        if at_word(b"template") && gap && matches!(self.c(after), b'A'..=b'Z' | b'a'..=b'z' | b'_' | b'$') {
            let mut k = after;
            loop {
                k += 1;
                while matches!(self.c(k), b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'$' | b'.' | b'[' | b']') {
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
            return self.take(after, k.min(self.n), JD_VAR);
        }
        let param_like = [&b"arg"[..], b"argument", b"const", b"constant", b"member", b"namespace", b"param", b"var"];
        if param_like.contains(&tag) && gap && matches!(self.c(after), b'A'..=b'Z' | b'a'..=b'z' | b'_' | b'$') {
            let mut k = after + 1;
            while matches!(self.c(k), b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'$' | b'.' | b'[' | b']') {
                k += 1;
            }
            tag_tok(self, e);
            return self.take(after, k.min(self.n), JD_VAR);
        }
        if at_word(b"typedef") && gap && self.c(after) == b'{' {
            tag_tok(self, e);
            self.i = after;
            self.push(DOC_TYPEDEF);
            return true;
        }
        let param_typed = [&b"arg"[..], b"argument", b"const", b"constant", b"member", b"namespace", b"param", b"prop", b"property", b"var"];
        if param_typed.contains(&tag) && gap && self.c(after) == b'{' {
            tag_tok(self, e);
            self.i = after;
            self.push(DOC_PARAM);
            return true;
        }
        let typed = [&b"define"[..], b"enum", b"exception", b"export", b"extends", b"lends", b"implements", b"modifies", b"namespace", b"private", b"protected", b"returns", b"return", b"suppress", b"this", b"throws", b"type", b"yields", b"yield"];
        if typed.contains(&tag) && gap && self.c(after) == b'{' {
            tag_tok(self, e);
            self.i = after;
            self.push(DOC_TYPED);
            return true;
        }
        let named = [&b"alias"[..], b"augments", b"callback", b"constructs", b"emits", b"event", b"fires", b"exports", b"export", b"extends", b"external", b"function", b"func", b"host", b"lends", b"listens", b"interface", b"memberof", b"memberof!", b"method", b"module", b"mixes", b"mixin", b"name", b"requires", b"see", b"this", b"typedef", b"uses"];
        let tag_x = if self.c(e) == b'!' && tag == b"memberof" { &self.b[s..e + 1] } else { tag };
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
            if r.starts_with(b"http://") || r.starts_with(b"https://") {
                let k = doc_run(self.b, after, b"");
                tag_tok(self, e);
                return self.take(after, k, JD_LINK);
            }
            let k = doc_run(self.b, after, b"@/");
            // not before an inline tag, a `[description]` before it or not
            let inline = inline_tag_len(r).is_some() || (r.starts_with(b"[") && {
                let e = r.iter().position(|&c| c == b']');
                e.is_some_and(|e| !r[1..e].contains(&b'[') && inline_tag_len(&r[e + 1..]).is_some())
            });
            if k > after && !inline {
                tag_tok(self, e);
                return self.take(after, k, JD_TYPE);
            }
        }
        // @default '…'
        if (at_word(b"default") || at_word(b"defaultvalue") || at_word(b"license") || at_word(b"version")) && gap && matches!(self.c(after), b'\'' | b'"') {
            tag_tok(self, e);
            self.tok(after, after + 1, JD_STR_BEGIN);
            self.i = after + 1;
            self.push(if self.c(after) == b'\'' { DOC_STRING_S } else { DOC_STRING_D });
            return true;
        }
        if [&b"default"[..], b"defaultvalue", b"license", b"tutorial", b"variation", b"version"].contains(&tag) && gap {
            let mut k = after;
            while k < self.n && !is_space(self.b[k]) && self.b[k] != b'*' {
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
        if doc_tag(&self.b[s..w.min(self.n)]) && self.boundary(w) {
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
            while e < self.n && self.b[e] != b']' {
                e += 1;
            }
            if e > i + 1 && e < self.n && inline_tag_len(self.rest(e + 1)).is_some() {
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
        let k = self.ws(e).min(self.n);
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
            if r.starts_with(b"http://") || r.starts_with(b"https://") {
                while j < self.n {
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
            while j < self.n {
                let d = self.b[j];
                if matches!(d, b'{' | b'}' | b'@' | b'|') || is_space(d) {
                    break;
                }
                if d == b'*' {
                    if self.c(j + 1) != b'/' && j + 1 < self.n {
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
        while j < self.n {
            match self.b[j] {
                b'}' => break,
                b'*' if j + 1 < self.n && matches!(self.b[j + 1], b'/' | b'}') => break,
                b'*' => j += 2,
                _ => j += 1,
            }
        }
        (j >= self.n).then_some(self.n)
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
            if self.rest(k).starts_with(b"[]") {
                k += 2;
            }
            if self.c(k) == b'.' && (self.c(k + 1).is_ascii_alphanumeric() || matches!(self.c(k + 1), b'_' | b'$')) {
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
            while e < self.n {
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
    if r.starts_with(b"linkcode") {
        Some(8)
    } else if r.starts_with(b"linkplain") {
        Some(9)
    } else if r.starts_with(b"link") {
        Some(4)
    } else if r.starts_with(b"tutorial") {
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
        b"abstract" | b"access" | b"alias" | b"api" | b"arg" | b"argument" | b"async"
            | b"attribute" | b"augments" | b"author" | b"beta" | b"borrows" | b"bubbles"
            | b"callback" | b"chainable" | b"class" | b"classdesc" | b"code" | b"config"
            | b"const" | b"constant" | b"constructor" | b"constructs" | b"copyright"
            | b"default" | b"defaultvalue" | b"define" | b"deprecated" | b"desc"
            | b"description" | b"dict" | b"emits" | b"enum" | b"event" | b"example"
            | b"exception" | b"exports" | b"export" | b"extends" | b"extension"
            | b"extension_for" | b"extensionfor" | b"external" | b"externs" | b"file"
            | b"fileoverview" | b"final" | b"fires" | b"for" | b"func" | b"function"
            | b"generator" | b"global" | b"hideconstructor" | b"host" | b"ignore"
            | b"implements" | b"implicitCast" | b"inheritDoc" | b"inheritdoc" | b"inner"
            | b"instance" | b"interface" | b"internal" | b"kind" | b"lends" | b"license"
            | b"listens" | b"main" | b"member" | b"memberof" | b"method" | b"mixes"
            | b"mixin" | b"mixins" | b"modifies" | b"module" | b"name" | b"namespace"
            | b"noalias" | b"nocollapse" | b"nocompile" | b"nosideeffects" | b"override"
            | b"overview" | b"package" | b"param" | b"polymer" | b"polymerBehavior"
            | b"preserve" | b"private" | b"prop" | b"property" | b"protected" | b"public"
            | b"readOnly" | b"readonly" | b"record" | b"required" | b"requires" | b"returns"
            | b"return" | b"see" | b"since" | b"static" | b"struct" | b"submodule"
            | b"summary" | b"suppress" | b"template" | b"this" | b"throws" | b"todo"
            | b"tutorial" | b"type" | b"typedef" | b"unrestricted" | b"uses" | b"var"
            | b"variation" | b"version" | b"virtual" | b"writeOnce" | b"yields" | b"yield"
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
