//! Expressions: `expression`'s patterns in the grammar's order, tried by the byte at the place;
//! names by what is around them (a call, an object, a property, a function assigned).

use super::words;
use super::*;

/// `support-objects`' first pattern, a member `w` (its [`words::class`] `cl`) after a `.`: its
/// kind, `call` if a `(` follows it past blanks and `no_call` unless a `(` follows it or its type
/// parameters. (`PLAIN` for `Number`'s constants: the grammar's capture 4 is in its lookahead.)
fn member_support(w: &[u8], cl: u32, call: bool, no_call: bool) -> Option<u16> {
    Some(
        if matches!(w, b"constructor" | b"length" | b"prototype" | b"__proto__") && no_call {
            SUPPORT_PROP
        } else if matches!(
            w,
            b"EPSILON"
                | b"MAX_SAFE_INTEGER"
                | b"MAX_VALUE"
                | b"MIN_SAFE_INTEGER"
                | b"MIN_VALUE"
                | b"NEGATIVE_INFINITY"
                | b"POSITIVE_INFINITY"
        ) {
            PLAIN
        } else if cl & words::DOM_CONST != 0 && no_call {
            DOM_CONST
        } else if cl & words::DOM_PROP != 0 && no_call {
            DOM_PROP
        } else if call && cl & words::EVENT_HANDLER != 0 {
            EVENT_HANDLER
        } else if call && cl & words::BROWSER_FN != 0 {
            SUPPORT_FN
        } else if call && cl & words::DOM_FN != 0 {
            SUPPORT_FN_DOM
        } else if call
            && matches!(
                w,
                b"all" | b"catch" | b"finally" | b"race" | b"reject" | b"resolve" | b"then"
            )
        {
            PROMISE_FN
        } else {
            return None;
        },
    )
}

/// [`Scan::call_chain`]'s answer.
enum Chain {
    /// The call's tokens taken, its arguments' frame pushed.
    Call,
    /// No call of any target from the name.
    NoCall,
    /// Not shown either way.
    Unknown,
}

impl Scan<'_, '_, '_> {
    /// `expression` at the place.
    pub(super) fn expression(&mut self) -> bool {
        let c = self.c(self.i);
        // no pattern before the comma's can start at a `,`
        if c == b',' {
            return self.take(self.i, self.i + 1, COMMA);
        }
        // at a blank, no pattern matches unless `blank_led`'s bytes follow: the turns in frames
        // the run loop doesn't pass blanks in ([`super::quiet`]), each pattern walking the blanks
        // to fail (X-035)
        if is_space(c) && !self.blank_led(self.ws(self.i)) {
            return false;
        }
        if is_name_start(c)
            && let Some(r) = self.plain_name(false)
        {
            return r;
        }
        if matches!(c, b'.' | b'?')
            && let Some(r) = self.plain_member()
        {
            return r;
        }
        self.expression_without_identifiers()
            || self.identifiers()
            || self.expression_punctuations()
    }

    /// A name at the place that no pattern keys on ([`words::AT_NAME`]), classified once by the
    /// byte after it instead of through `expression`'s patterns, each of which would fail on it
    /// but `identifiers`' (and an arrow's parameter before `=>`): `None` where something else
    /// may follow it (a call, a property, a template, a label in `statements`), for the patterns
    /// in order.
    pub(super) fn plain_name(&mut self, stmt: bool) -> Option<bool> {
        let i = self.i;
        if self.tick > i || !self.wb(i) {
            return None;
        }
        let e = self.name(i);
        let w = &self.b[i..e];
        // `\b` falls before a `$` (`function$x` is `function` then `$x`), so a name with one is
        // left to the patterns
        if has_dollar(w) {
            return None;
        }
        let cl = words::class(w);
        if cl & (words::LITERAL | words::OP_WORD) != 0 {
            return self.keyword_name(i, e, cl, stmt);
        }
        if cl & words::AT_NAME != 0 {
            return None;
        }
        let k = self.ws(e);
        match self.c(k) {
            // `function-call` with its target and `(` in one step: the target's turn would find
            // only the name, and the call's only the `(`; the frames' depth as theirs would be
            b'(' if k == e && self.depth + 2 < MAX => {
                self.tok(i, e, ENTITY_FN);
                self.tok(e, e + 1, ROUND);
                self.i = e + 1;
                self.push(CALL_ARGS);
                Some(true)
            }
            // a call's target through dots, else `object-identifiers`
            b'.' => Some(
                match if k == e {
                    self.call_chain(i, e, READWRITE)
                } else {
                    Chain::Unknown
                } {
                    Chain::Call => true,
                    Chain::NoCall => self.plain_object(i, e),
                    Chain::Unknown => self.function_call() || self.plain_object(i, e),
                },
            ),
            b'(' | b'<' | b'?' | b'!' | b'`' => None,
            b':' if stmt => None,
            b'=' if self.c(k + 1) == b'>' => {
                // arrow-function's first: `name\s*(?==>)`
                self.tok(i, e, PARAM);
                self.i = k.min(self.b.len());
                Some(true)
            }
            b'=' if self.c(k + 1) != b'=' && look::fn_value(self.b, self.ws(k + 1), self.tsx) => {
                Some(self.take(i, e, ENTITY_FN))
            }
            _ => {
                let u = look::upper(self.b, i);
                Some(self.take(i, e, if u == e { CONSTANT } else { READWRITE }))
            }
        }
    }

    /// [`Scan::plain_name`] for a keyword `i..e` of `literal`'s or an operator word that takes
    /// only itself (`await`, `delete`, `void`, `new`): the patterns before them can take one only
    /// as an arrow's parameter or a label, or as a call's target.
    fn keyword_name(&mut self, i: usize, e: usize, cl: u32, stmt: bool) -> Option<bool> {
        let k = self.ws(e);
        let d = self.c(k);
        if (d == b'=' && self.c(k + 1) == b'>') || (d == b':' && stmt) {
            return None;
        }
        if cl & words::OP_WORD != 0 {
            if !self.we(e) {
                return None;
            }
            // `words::OP_WORD`'s four have four lengths
            return Some(match e - i {
                3 => {
                    self.tok(i, e, NEW);
                    self.i = e;
                    self.push(NEW_F);
                    true
                }
                5 => self.take(i, e, FLOW),
                6 => self.take(i, e, DELETE),
                _ => self.take(i, e, VOID),
            });
        }
        match d {
            b'(' | b'<' | b'?' | b'!' | b'`' => return None,
            b'.' if k == e && crate::same(&self.b[i..e], b"this") && self.boundary(e) => {
                match self.call_chain(i, e, THIS) {
                    Chain::Call => return Some(true),
                    Chain::NoCall => {}
                    Chain::Unknown => {
                        if self.function_call() {
                            return Some(true);
                        }
                    }
                }
            }
            b'.' if self.function_call() => return Some(true),
            _ => {}
        }
        // `words::LITERAL`'s eight, each by its length and last byte
        let kind = match (e - i, self.b[e - 1]) {
            (4, b's') => THIS,
            (5, b'r') => SUPER,
            (4, b'e') => TRUE,
            (5, b'e') => FALSE,
            (4, b'l') => NULL,
            (9, b'd') => UNDEFINED,
            (3, b'N') => NAN,
            _ => INFINITY,
        };
        let end_ok = if matches!(kind, THIS | SUPER) {
            self.boundary(e)
        } else {
            self.we(e)
        };
        if !end_ok {
            return None;
        }
        Some(self.take(i, e, kind))
    }

    /// `function-call` of a target `i..e` (a plain name, or `this`: `first` its kind) then plain
    /// members, `.name` each, and `(` right after the last, in one step: the target's turns
    /// would give the first its kind (as an object before a `.`), each middle one an object's
    /// property, the last a function, and the call's turn take the `(`. `None` for any other
    /// shape (a blank, `?.`, `#`, a `$`), left to the patterns, but where the members end before
    /// a byte no call can follow ([`Chain::NoCall`]).
    fn call_chain(&mut self, i: usize, e: usize, first: u16) -> Chain {
        let mut segs = [(0, 0); 8];
        let mut n = 0;
        let mut j = e;
        while self.c(j) == b'.' {
            let s = j + 1;
            if !is_name_start(self.c(s)) || n == segs.len() {
                return Chain::Unknown;
            }
            let ne = self.name(s);
            let w = &self.b[s..ne];
            if has_dollar(w) {
                return Chain::Unknown;
            }
            segs[n] = (s, ne);
            n += 1;
            j = ne;
        }
        if n == 0 {
            return Chain::Unknown;
        }
        if self.c(j) != b'(' || self.depth + 2 >= MAX {
            // `function-call`'s targets end only where these members do, at a `.` or here: a
            // call needs `(`, `<`, `?.` or `!` after one
            return if matches!(self.c(self.ws(j)), b'(' | b'<' | b'?' | b'!' | b'.') {
                Chain::Unknown
            } else {
                Chain::NoCall
            };
        }
        let kind = match first {
            THIS => THIS,
            _ if &self.b[segs[0].0..segs[0].1] == b"prototype" => SUPPORT_CLASS,
            _ if look::upper(self.b, i) == e => CONSTANT_OBJ,
            _ => OBJECT,
        };
        self.tok(i, e, kind);
        for (k, &(s, ne)) in segs[..n].iter().enumerate() {
            self.tok(s - 1, s, ACCESSOR);
            // `support-objects` first, at the `.`; the last one's name in its own turn after the
            // accessor (no pattern but the name's after a `.`), each other one's with its `.`
            let w = &self.b[s..ne];
            let last = k + 1 == n;
            let cl = words::class(w);
            let support = if cl & words::AT_DOT != 0 {
                member_support(w, cl, last, !last)
            } else {
                None
            };
            let kind = if let Some(kind) = support {
                kind
            } else if last {
                ENTITY_FN
            } else if look::upper(self.b, s) == ne {
                CONSTANT_OBJ_PROP
            } else {
                OBJECT_PROP
            };
            self.tok(s, ne, kind);
        }
        self.tok(j, j + 1, ROUND);
        self.i = j + 1;
        self.push(CALL_ARGS);
        Chain::Call
    }

    /// `identifiers` for a plain name `i..e` before a `.` that no call follows:
    /// `object-identifiers`' `prototype` and object, else the name.
    pub(super) fn plain_object(&mut self, i: usize, e: usize) -> bool {
        // name(?=\s*\??\.\s*prototype\b(?!\$))
        let mut k = self.ws(e);
        if self.c(k) == b'?' {
            k += 1;
        }
        if self.c(k) == b'.'
            && let Some(p) = self.word_b(self.ws(k + 1), b"prototype")
            && self.c(p) != b'$'
        {
            return self.take(i, e, SUPPORT_CLASS);
        }
        let u = look::upper(self.b, i);
        let kind = if self.dot_name_after(e) {
            if u == e { CONSTANT_OBJ } else { OBJECT }
        } else if u == e {
            CONSTANT
        } else {
            READWRITE
        };
        self.take(i, e, kind)
    }

    /// A `.` or `?.` and a member name at the place that no pattern keys on ([`words::AT_DOT`]),
    /// as [`Scan::plain_name`] does a name: `object-identifiers`' property before another
    /// `.`, `identifiers`' function or property; `None` before a call and the like.
    pub(super) fn plain_member(&mut self) -> Option<bool> {
        let i = self.i;
        if self.tick > i || (self.c(i) == b'.' && self.c(i + 1) == b'.') {
            return None;
        }
        let (de, dk) = self.dot(i)?;
        let s = self.ws(de);
        let s2 = if self.c(s) == b'#' { s + 1 } else { s };
        let ne = self.name(s2);
        if ne == s2 {
            return None;
        }
        let w = &self.b[s2..ne];
        if has_dollar(w) {
            return None;
        }
        let cl = words::class(w);
        let k = self.ws(ne);
        let kind = match self.c(k) {
            b'(' | b'<' | b'?' | b'!' | b'`' => return None,
            // `support-objects` (not after `#`), no call after it
            _ if s2 == s
                && cl & words::AT_DOT != 0
                && let Some(kind) = member_support(w, cl, false, true) =>
            {
                kind
            }
            b'.' if self.dot_name_after(ne) => {
                if look::upper(self.b, s2) == ne {
                    CONSTANT_OBJ_PROP
                } else {
                    OBJECT_PROP
                }
            }
            b'=' if s2 == s
                && self.c(k + 1) != b'='
                && look::fn_value(self.b, self.ws(k + 1), self.tsx) =>
            {
                ENTITY_FN
            }
            _ => {
                if look::upper(self.b, s2) == ne {
                    CONSTANT_PROP
                } else {
                    PROPERTY
                }
            }
        };
        self.tok(i, de, dk);
        Some(self.take(s, ne, kind))
    }

    /// `expressionPunctuations`: `,`, `.` and `?.`.
    pub(super) fn expression_punctuations(&mut self) -> bool {
        let i = self.i;
        match self.c(i) {
            b',' => self.take(i, i + 1, COMMA),
            _ => self.accessor(),
        }
    }

    /// `punctuation-accessor`: `(?:(\.)|(\?\.(?!\s*[[:digit:]])))`.
    pub(super) fn accessor(&mut self) -> bool {
        let i = self.i;
        match self.c(i) {
            b'.' => self.take(i, i + 1, ACCESSOR),
            b'?' if self.c(i + 1) == b'.' && !self.c(self.ws(i + 2)).is_ascii_digit() => {
                self.take(i, i + 2, ACCESSOR_OPT)
            }
            _ => false,
        }
    }

    /// `(\.)|(\?\.(?!\s*[[:digit:]]))` at `i`: its end and kind.
    pub(super) fn dot(&self, i: usize) -> Option<(usize, u16)> {
        match self.c(i) {
            b'.' if self.c(i + 1) != b'.' || self.c(i + 2) != b'.' => Some((i + 1, ACCESSOR)),
            b'.' => Some((i + 1, ACCESSOR)),
            b'?' if self.c(i + 1) == b'.' && !self.c(self.ws(i + 2)).is_ascii_digit() => {
                Some((i + 2, ACCESSOR_OPT))
            }
            _ => None,
        }
    }

    /// `expressionWithoutIdentifiers` at the place.
    pub(super) fn expression_without_identifiers(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        // TSX: jsx, first
        if self.tsx && self.jsx() {
            return true;
        }
        // string
        if matches!(c, b'\'' | b'"' | b'`') {
            return self.string();
        }
        let name_start = look::alpha(self.b, i) > 0;
        if (name_start || matches!(c, b'.' | b'?' | b'<') || is_space(c)) && self.template_call() {
            return true;
        }
        if name_start && self.template(false) {
            return true;
        }
        // regex
        if (c == b'/' || is_space(c)) && self.regex() {
            return true;
        }
        if c == b'/' && self.comment(false) {
            return true;
        }
        if i == 0 && is_space(c) && self.comment(false) {
            return true;
        }
        if name_start {
            // function-expression
            if let Some(r) = self.function_head(i, false) {
                return self.function_toks(r, FUNC_EXPR);
            }
            if self.class_expression() {
                return true;
            }
        }
        // arrow-function
        if self.arrow_function() {
            return true;
        }
        // paren-expression-possibly-arrow
        if self.possibly_arrow() {
            return true;
        }
        if (c == b':' || is_space(c)) && self.maybe_arrow_return() {
            return true;
        }
        // cast
        if !self.tsx && (c == b'<' || is_space(c)) && self.cast() {
            return true;
        }
        if c == b'?' && self.ternary() {
            return true;
        }
        if name_start {
            if let Some(e) = self.kw(i, b"new") {
                self.tok(i, e, NEW);
                self.i = e;
                self.push(NEW_F);
                return true;
            }
            if let Some(e) = self.kw(i, b"instanceof") {
                self.tok(i, e, INSTANCEOF);
                self.i = e;
                self.push(INSTANCEOF_F);
                return true;
            }
        }
        if c == b'{' {
            self.tok(i, i + 1, BLOCK_BRACE);
            self.i = i + 1;
            self.push(OBJ);
            return true;
        }
        if self.expression_operators() {
            return true;
        }
        if (name_start || matches!(c, b'.' | b'?') || (i > 0 && self.b[i - 1] == b')'))
            && self.function_call()
        {
            return true;
        }
        if self.literal() || self.support_objects() {
            return true;
        }
        if c == b'(' {
            self.tok(i, i + 1, ROUND);
            self.i = i + 1;
            self.push(PAREN);
            return true;
        }
        false
    }

    /// `class-expression`: `«wb»(?:(abstract)\s+)?(class)\b(?=\s+|[<{]|\/[\/*])`.
    pub(super) fn class_expression(&mut self) -> bool {
        let i = self.i;
        if !self.wb(i) {
            return false;
        }
        let mut k = i;
        let mut abs = None;
        if let Some(e) = self.word_b(i, b"abstract") {
            let m = self.ws(e);
            if m > e {
                abs = Some(e);
                k = m;
            }
        }
        let Some(e) = self.word_b(k, b"class") else {
            return false;
        };
        if !(k == i || self.boundary(k)) {
            return false;
        }
        let d = self.c(e);
        if !(is_space(d)
            || matches!(d, b'<' | b'{')
            || (d == b'/' && matches!(self.c(e + 1), b'/' | b'*')))
        {
            return false;
        }
        if let Some(a) = abs {
            self.tok(i, a, MODIFIER);
        }
        self.tok(k, e, CLASS_KW);
        self.i = e;
        self.push(CLASS);
        true
    }

    /// `arrow-function`'s three.
    pub(super) fn arrow_function(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        // =>
        if c == b'=' && self.c(i + 1) == b'>' {
            self.tok(i, i + 2, ARROW);
            self.i = i + 2;
            self.push(ARROW_F);
            return true;
        }
        // (?:«wb»(\basync)\s+)?(name)\s*(?==>)
        let mut j = i;
        let mut asy = None;
        if self.wb(i) && self.word_b(i, b"async").is_some() && self.boundary(i) {
            let k = self.ws(i + 5);
            if k > i + 5 {
                asy = Some(i);
                j = k;
            }
        }
        let mut ne = self.name(j);
        if asy.is_some()
            && !(ne > j && self.c(self.ws(ne)) == b'=' && self.c(self.ws(ne) + 1) == b'>')
        {
            // backtracking: no name after `async`, or none before `=>`: `async` is the name
            // (`async => x`)
            let e = self.name(i);
            let k = self.ws(e);
            if self.c(k) == b'=' && self.c(k + 1) == b'>' {
                asy = None;
                j = i;
                ne = e;
            }
        }
        if ne > j {
            let k = self.ws(ne);
            if self.c(k) == b'=' && self.c(k + 1) == b'>' {
                if let Some(a) = asy {
                    self.tok(a, a + 5, ASYNC);
                }
                self.tok(j, ne, PARAM);
                self.i = k.min(self.b.len());
                return true;
            }
            if asy.is_none() {
                // a name can't start the second pattern (but `async`)
                if !self.is_name(i, ne, b"async") {
                    return false;
                }
            }
        }
        // (?:«wb»(\basync))?((?<![})!\]])\s*(?=sure|line))
        let mut s = i;
        let mut asy = None;
        if self.wb(i) && self.has(i, b"async") && self.boundary(i + 5) {
            asy = Some(i);
            s = i + 5;
        }
        if s > 0 && matches!(self.b[s - 1], b'}' | b')' | b'!' | b']') {
            return false;
        }
        let k = self.ws(s);
        if !matches!(self.c(k), b'(' | b'<') {
            return false;
        }
        if !(look::arrow_sure(self.b, k)
            || (self.tsx && look::tp_extends(self.b, k))
            || look::arrow_line(self.b, k))
        {
            return false;
        }
        if let Some(a) = asy {
            self.tok(a, a + 5, ASYNC);
        }
        self.i = k.min(self.b.len());
        if self.i == i {
            self.push_zero(ARROW_HEAD)
        } else {
            self.push(ARROW_HEAD);
            true
        }
    }

    /// `paren-expression-possibly-arrow`'s two.
    pub(super) fn possibly_arrow(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        if !(matches!(c, b'(' | b'<' | b'a') || is_space(c)) || i == 0 {
            return false;
        }
        let p = self.b[i - 1];
        let first = matches!(p, b'(' | b'=' | b',');
        let second =
            first || (i >= 2 && &self.b[i - 2..i] == b"=>") || self.word_ends_at(i, &[b"return"]);
        if !second {
            return false;
        }
        let mut k = self.ws(i);
        let mut asy = None;
        if self.has(k, b"async") {
            asy = Some(k);
            k += 5;
        }
        let q = self.ws(k);
        let ok = (first && {
            // «TP»?\(\s*(OPEN)
            let t = if self.c(q) == b'<' {
                look::tp(self.b, q)
            } else {
                Some(q)
            };
            t.is_some_and(|t| self.c(t) == b'(' && look::open_tail(self.b, self.ws(t + 1)))
        }) || {
            // (((«TP»)?\()|(<))\s*$
            let t = if self.c(q) == b'<' {
                look::tp(self.b, q)
            } else {
                Some(q)
            };
            let a = t.is_some_and(|t| self.c(t) == b'(' && self.ws(t + 1) >= self.b.len());
            a || (self.c(q) == b'<' && self.ws(q + 1) >= self.b.len())
        };
        if !ok {
            return false;
        }
        if let Some(a) = asy {
            self.tok(a, a + 5, ASYNC);
            self.i = a + 5;
        } else {
            self.i = k.min(self.b.len());
        }
        if self.i == i {
            self.push_zero(MAYBE_ARROW)
        } else {
            self.push(MAYBE_ARROW);
            true
        }
    }

    /// `possibly-arrow-return-type`: `(?<=\)|^)\s*(:)(?=\s*«RT»+\s*=>)`.
    pub(super) fn maybe_arrow_return(&mut self) -> bool {
        let i = self.i;
        if !(i == 0 || self.b[i - 1] == b')') {
            return false;
        }
        let k = self.ws(i);
        if self.c(k) != b':' || !look::rt_arrow(self.b, k + 1) {
            return false;
        }
        self.tok(k, k + 1, ANNOTATION);
        self.i = k + 1;
        self.push(MAYBE_ARROW_RET);
        true
    }

    /// `cast`'s three.
    pub(super) fn cast(&mut self) -> bool {
        let i = self.i;
        let k = self.ws(i);
        if self.c(k) != b'<' {
            return false;
        }
        // \s*(<)\s*(const)\s*(>)
        let m = self.ws(k + 1);
        if let Some(e) = self.word_b(m, b"const") {
            let g = self.ws(e);
            if self.c(g) == b'>' {
                self.tok(k, k + 1, ANGLE);
                self.tok(m, e, MODIFIER);
                return self.take(g, g + 1, ANGLE);
            }
        }
        // the lookbehind's operators, then \s*(<)(?!<?\=)(?!\s*$): `(?<!\+\+|--)` holds for every
        // alternative, so its `[^_$[:alnum:]](?:\+\+|\-\-)` never does
        let before = i > 0 && {
            let b = self.b;
            let p = b[i - 1];
            !(i >= 2 && (&b[i - 2..i] == b"++" || &b[i - 2..i] == b"--"))
                && (self.word_ends_at(i, &[b"return", b"throw", b"yield", b"await", b"default"])
                    || matches!(
                        p,
                        b'=' | b'(' | b',' | b':' | b'>' | b'*' | b'?' | b'&' | b'|' | b'^'
                    )
                    || (i >= 2 && matches!(p, b'+' | b'-')))
        };
        let not_eq = !(self.c(k + 1) == b'=' || (self.c(k + 1) == b'<' && self.c(k + 2) == b'='));
        if before && not_eq && self.ws(k + 1) < self.b.len() {
            self.tok(k, k + 1, ANGLE);
            self.i = k + 1;
            self.push(CAST);
            return true;
        }
        // (?<=^)\s*(<)(?=name\s*>)
        if i == 0 {
            let e = self.name(k + 1);
            if e > k + 1 && self.c(self.ws(e)) == b'>' {
                self.tok(k, k + 1, ANGLE);
                self.i = k + 1;
                self.push(CAST);
                return true;
            }
        }
        false
    }

    /// `ternary-expression`: `(?!\?\.\s*[^[:digit:]])(\?)(?!\?)`.
    pub(super) fn ternary(&mut self) -> bool {
        let i = self.i;
        if self.c(i + 1) == b'?' {
            return false;
        }
        if self.c(i + 1) == b'.' {
            let k = self.ws(i + 2);
            if !self.c(k).is_ascii_digit() {
                return false;
            }
        }
        self.tok(i, i + 1, TERNARY);
        self.i = i + 1;
        self.push(TERNARY_F);
        true
    }

    /// `expression-operators` at the place.
    pub(super) fn expression_operators(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        if look::alpha(self.b, i) > 0 {
            if !self.wb(i) {
                return false;
            }
            let w = self.name(i);
            if !self.we(w) {
                return false;
            }
            let word = &self.b[i..w];
            return match word {
                b"await" => self.take(i, w, FLOW),
                b"yield" => {
                    // a comment then `*`
                    let k = self.ws(w);
                    if self.c(k) == b'/' && self.c(k + 1) == b'*' {
                        let e = look::comments(self.b, k);
                        if e > k && self.c(e) == b'*' {
                            self.tok(i, w, FLOW);
                            self.i = w;
                            self.push(YIELD_STAR);
                            return true;
                        }
                    }
                    self.tok(i, w, FLOW);
                    if self.c(k) == b'*' {
                        return self.take(k, k + 1, ASTERISK);
                    }
                    self.i = w;
                    true
                }
                b"delete" => self.take(i, w, DELETE),
                b"in" if self.c(w) != b'(' => self.take(i, w, IN),
                b"of" if self.c(w) != b'(' => self.take(i, w, OF),
                b"instanceof" => self.take(i, w, INSTANCEOF),
                b"new" => self.take(i, w, NEW),
                b"typeof" => {
                    self.tok(i, w, TYPEOF);
                    self.i = w;
                    self.push(TYPEOF_F);
                    true
                }
                b"void" => self.take(i, w, VOID),
                b"as" => {
                    let k = self.ws(w);
                    if k == w {
                        return false;
                    }
                    if let Some(e) = self.word_b(k, b"const") {
                        let m = self.ws(e);
                        if m >= self.b.len()
                            || matches!(self.c(m), b';' | b',' | b':' | b'}' | b')' | b']')
                        {
                            self.tok(i, w, AS);
                            return self.take(k, e, MODIFIER);
                        }
                    }
                    self.tok(i, w, AS);
                    self.i = k.min(self.b.len());
                    self.push(AS_F);
                    true
                }
                _ => false,
            };
        }
        let c1 = self.c(i + 1);
        let c2 = self.c(i + 2);
        let c3 = self.c(i + 3);
        let (len, kind) = match c {
            b'.' if c1 == b'.' && c2 == b'.' => (3, SPREAD),
            b'*' if c1 == b'=' => (2, COMPOUND),
            b'/' if c1 == b'=' && !(i > 0 && self.b[i - 1] == b'(') => (2, COMPOUND),
            b'%' | b'+' | b'-' if c1 == b'=' => (2, COMPOUND),
            b'&' | b'^' | b'|' if c1 == b'=' => (2, COMPOUND_BITWISE),
            b'<' if c1 == b'<' && c2 == b'=' => (3, COMPOUND_BITWISE),
            b'>' if c1 == b'>' && c2 == b'=' => (3, COMPOUND_BITWISE),
            b'>' if c1 == b'>' && c2 == b'>' && c3 == b'=' => (4, COMPOUND_BITWISE),
            b'<' if c1 == b'<' => (2, SHIFT),
            b'>' if c1 == b'>' && c2 == b'>' => (3, SHIFT),
            b'>' if c1 == b'>' => (2, SHIFT),
            b'=' if c1 == b'=' && c2 == b'=' => (3, COMPARISON),
            b'!' if c1 == b'=' && c2 == b'=' => (3, COMPARISON),
            b'=' | b'!' if c1 == b'=' => (2, COMPARISON),
            b'<' | b'>' if c1 == b'=' => (2, RELATIONAL),
            b'<' if c1 == b'>' => (2, RELATIONAL),
            b'<' | b'>' => (1, RELATIONAL),
            b'!' if self.alnum_before(i) && {
                let k = self.ws(i + 1);
                self.c(k) == b'/'
                    && (self.c(k + 1) == b'=' || !matches!(self.c(k + 1), b'/' | b'*'))
            } =>
            {
                let k = self.ws(i + 1);
                self.tok(i, i + 1, LOGICAL);
                return if self.c(k + 1) == b'=' {
                    self.take(k, k + 2, COMPOUND)
                } else {
                    self.take(k, k + 1, ARITHMETIC)
                };
            }
            b'!' => (1, LOGICAL),
            b'&' if c1 == b'&' => (2, LOGICAL),
            b'|' if c1 == b'|' => (2, LOGICAL),
            b'?' if c1 == b'?' => (2, LOGICAL),
            b'&' | b'~' | b'^' | b'|' => (1, BITWISE),
            b'=' => (1, ASSIGN),
            b'-' if c1 == b'-' => (2, DECREMENT),
            b'+' if c1 == b'+' => (2, INCREMENT),
            b'%' | b'*' | b'/' | b'-' | b'+' => (1, ARITHMETIC),
            _ => {
                // (?<=[_$[:alnum:])\]])\s*(?:(/=)|(?:(/)(?![/*]))), with comments between
                if is_space(c)
                    && i > 0
                    && (self.alnum_before(i) || matches!(self.b[i - 1], b')' | b']'))
                {
                    let k = self.ws(i);
                    let k2 = look::comments(self.b, k);
                    if self.c(k2) == b'/'
                        && (self.c(k2 + 1) == b'=' || !matches!(self.c(k2 + 1), b'/' | b'*'))
                    {
                        if k2 > k {
                            // comments between: their own tokens first
                            return false;
                        }
                        return if self.c(k + 1) == b'=' {
                            self.take(k, k + 2, COMPOUND)
                        } else {
                            self.take(k, k + 1, ARITHMETIC)
                        };
                    }
                }
                return false;
            }
        };
        self.take(i, i + len, kind)
    }

    /// `function-call` at the place: a target and a call after it.
    pub(super) fn function_call(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        let target = if c == b'.' || c == b'?' || look::alpha(self.b, i) > 0 {
            look::call_target(self.b, i)
        } else {
            None
        };
        if target.is_none() {
            // (?<=[\)])\s*…\(: a call of what a `)` closed
            let after_paren = i > 0 && self.b[i - 1] == b')' && look::call_after(self.b, i);
            if !after_paren {
                // the second: a target and `<` before `{`, `[` or `(` at the line's end
                if self.call_eol_target(i).is_some() {
                    self.push_zero(CALL_EOL);
                    return self.push_zero(CALL_EOL_TARGET);
                }
                return false;
            }
        }
        self.push_zero(CALL);
        if target.is_some() {
            self.push_zero(CALL_TARGET);
        }
        true
    }

    /// `template-call`'s two: a tag, through dots, and type arguments maybe, before a template.
    pub(super) fn template_call(&mut self) -> bool {
        let i = self.i;
        if self.tick <= i {
            return false;
        }
        // (?=((name\s*\??\.\s*)*|(\??\.\s*)?)(name)(«TA»)?`)
        let mut j = i;
        let mut chain = true;
        if look::alpha(self.b, i) == 0 {
            let mut k = i;
            if self.c(k) == b'?' {
                k += 1;
            }
            if self.c(k) == b'.' {
                j = self.ws(k + 1);
                // `(\??\.\s*)?` then one name: no more dots after it
                let ne = self.name(j);
                let after = if self.c(ne) == b'<' {
                    look::type_args(self.b, ne).map(|e| self.ws(e))
                } else {
                    Some(ne)
                };
                if !(ne > j && after.is_some_and(|a| self.c(a) == b'`')) {
                    return false;
                }
            } else {
                chain = false;
            }
        }
        let mut last = None;
        while chain && last.is_none() {
            let ne = self.name(j);
            if ne == j {
                break;
            }
            let after = if self.c(ne) == b'<' {
                look::type_args(self.b, ne).map(|e| self.ws(e))
            } else {
                Some(ne)
            };
            if after.is_some_and(|a| self.c(a) == b'`') {
                last = Some(ne);
                break;
            }
            let mut k = self.ws(ne);
            if self.c(k) == b'?' {
                k += 1;
            }
            if self.c(k) != b'.' {
                break;
            }
            j = self.ws(k + 1);
        }
        if last.is_some() {
            self.push_zero(TPL_CALL);
            return self.push_zero(TPL_CALL_TARGET);
        }
        // (name)?\s*(?=(«TA»)`)
        let ne = self.name(i);
        let k = self.ws(ne);
        if self.c(k) == b'<'
            && look::type_args(self.b, k).is_some_and(|e| self.c(self.ws(e)) == b'`')
        {
            self.push(TPL_CALL);
            if ne > i {
                self.tok(i, ne, TPL_TAG);
            }
            self.i = k;
            return true;
        }
        false
    }

    /// `(<\s*[\{\[\(]\s*$)` at `i`.
    pub(super) fn type_args_open_eol(&self, i: usize) -> bool {
        self.c(i) == b'<' && {
            let k = self.ws(i + 1);
            matches!(self.c(k), b'{' | b'[' | b'(') && self.ws(k + 1) >= self.b.len()
        }
    }

    /// A call's target from `i` with `(<\s*[\{\[\(]\s*$)` right after it: its end.
    pub(super) fn call_eol_target(&self, i: usize) -> Option<usize> {
        let mut j = i;
        let first = self.name(j);
        if first == j {
            let mut k = j;
            if self.c(k) == b'?' {
                k += 1;
            }
            if self.c(k) != b'.' {
                return None;
            }
            k = self.ws(k + 1);
            if self.c(k) == b'#' {
                k += 1;
            }
            let e = self.name(k);
            return (e > k && self.type_args_open_eol(e)).then_some(e);
        }
        j = first;
        let mut best = self.type_args_open_eol(j).then_some(j);
        loop {
            let mut k = self.ws(j);
            if self.c(k) == b'?' {
                k += 1;
            }
            if self.c(k) != b'.' {
                break;
            }
            k = self.ws(k + 1);
            if self.c(k) == b'#' {
                k += 1;
            }
            let e = self.name(k);
            if e == k {
                break;
            }
            j = e;
            if self.type_args_open_eol(j) {
                best = Some(j);
            }
        }
        best
    }

    /// Whether a call's target ends at `i`: `(?=\s*(?:(\?\.\s*)|(\!))?(«TA»\s*)?\()`.
    pub(super) fn call_target_end(&self, i: usize) -> bool {
        look::call_after(self.b, i)
    }

    /// `function-call-target`'s patterns: `support-function-call-identifiers`, then a name.
    pub(super) fn call_target_name(&mut self) -> bool {
        let i = self.i;
        if self.literal() || self.support_objects() || self.object_identifiers() || self.accessor()
        {
            return true;
        }
        if let Some(e) = self.kw(i, b"import") {
            let k = self.ws(e);
            if self.c(k) == b'(' {
                let q = self.ws(k + 1);
                if matches!(self.c(q), b'"' | b'\'' | b'`') {
                    return self.take(i, e, IMPORT_OP);
                }
            }
        }
        let s = if self.c(i) == b'#' { i + 1 } else { i };
        let ne = self.name(s);
        if ne > s {
            return self.take(i, ne, ENTITY_FN);
        }
        false
    }

    /// A call's turn after its target.
    pub(super) fn call_step(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        if i > 0 && self.b[i - 1] == b')' {
            let again = look::call_follows(self.b, i);
            if !again && self.pop_zero() {
                return true;
            }
        }
        // the inner push: another target (before the optionals, as the grammar orders them), its
        // lookahead only a name, not the call after it (`f<typeof o<d>S>(x)`: `S`, the type
        // arguments closed at `d>`)
        if self.name(i) > i
            || (matches!(c, b'.' | b'?') && self.dot(i).is_some() && {
                let s = self.ws(self.dot(i).map_or(i, |d| d.0));
                let s = if self.c(s) == b'#' { s + 1 } else { s };
                self.name(s) > s
            })
        {
            return self.push_zero(CALL_TARGET);
        }
        if self.comment(false) {
            return true;
        }
        match c {
            b'?' if self.c(i + 1) == b'.' => return self.take(i, i + 2, ACCESSOR_OPT),
            b'!' => return self.take(i, i + 1, DEFINITE),
            b'<' => return self.type_arguments(),
            b'(' => {
                self.tok(i, i + 1, ROUND);
                self.i = i + 1;
                self.set(CALL_ARGS);
                return true;
            }
            _ => {}
        }
        // the inner push: another target
        if look::call_target(self.b, i).is_some()
            || (matches!(c, b'.' | b'?') && self.dot(i).is_some())
        {
            return self.push_zero(CALL_TARGET);
        }
        false
    }

    /// `literal` at the place.
    pub(super) fn literal(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        if (c.is_ascii_digit() || c == b'.') && self.numeric() {
            return true;
        }
        if look::alpha(self.b, i) > 0 && self.wb(i) {
            let w = self.name(i);
            let word = &self.b[i..w];
            let we = self.we(w);
            let kind = match word {
                b"true" if we => TRUE,
                b"false" if we => FALSE,
                b"null" if we => NULL,
                b"undefined" if we => UNDEFINED,
                b"NaN" if we => NAN,
                b"Infinity" if we => INFINITY,
                b"this" if self.boundary(w) && self.c(w) != b'$' => THIS,
                b"super" if self.boundary(w) && self.c(w) != b'$' => SUPER,
                _ => PLAIN,
            };
            if kind != PLAIN {
                return self.take(i, w, kind);
            }
        }
        self.array_literal()
    }

    /// `array-literal`: `\s*(\[)`.
    pub(super) fn array_literal(&mut self) -> bool {
        let i = self.i;
        let k = self.ws(i);
        if self.c(k) != b'[' || k >= self.b.len() {
            return false;
        }
        self.tok(k, k + 1, SQUARE);
        self.i = k + 1;
        self.push(ARRAY);
        true
    }

    /// `numeric-literal` at the place.
    pub(super) fn numeric(&mut self) -> bool {
        let i = self.i;
        let Some((e, kind, big)) = self.number(i) else {
            return false;
        };
        if let Some(n) = big {
            self.tok(i, n, kind);
            let bk = match kind {
                HEX => HEX_BIG,
                BINARY => BINARY_BIG,
                OCTAL => OCTAL_BIG,
                _ => DECIMAL_BIG,
            };
            return self.take(n, e, bk);
        }
        self.take(i, e, kind)
    }

    /// A numeric key's turn: `numeric-literal` only.
    pub(super) fn numeric_key(&mut self) -> bool {
        self.numeric()
    }

    /// The end of a number at `i`, as `numeric-literal` reads it.
    pub(super) fn number_end(&self, i: usize) -> Option<usize> {
        self.number(i).map(|(e, _, _)| e)
    }

    /// A number at `i`: its end, kind, and where its `n` starts if a bigint.
    pub(super) fn number(&self, i: usize) -> Option<(usize, u16, Option<usize>)> {
        let b = self.b;
        let dollar_before = i > 0 && b[i - 1] == b'$';
        if dollar_before {
            return None;
        }
        let digits = |mut k: usize, ok: &dyn Fn(u8) -> bool| {
            while ok(self.c(k)) || self.c(k) == b'_' {
                k += 1;
            }
            k
        };
        let wb_start = self.boundary(i);
        // hex, binary, octal: \b(?<!\$)0(?:x|X)[0-9a-fA-F][0-9a-fA-F_]*(n)?\b(?!\$)
        if wb_start && self.c(i) == b'0' {
            let x = self.c(i + 1);
            // a digit's test, the kind, the first digit's place
            type Radix = (fn(u8) -> bool, u16, usize);
            let pick: Option<Radix> = match x {
                b'x' | b'X' => Some((|c| c.is_ascii_hexdigit(), HEX, i + 2)),
                b'b' | b'B' => Some((|c| c == b'0' || c == b'1', BINARY, i + 2)),
                b'o' | b'O' => Some((|c| (b'0'..=b'7').contains(&c), OCTAL, i + 2)),
                b'0'..=b'7' => Some((|c| (b'0'..=b'7').contains(&c), OCTAL, i + 1)),
                _ => None,
            };
            if let Some((ok, kind, s)) = pick
                && ok(self.c(s))
            {
                let e = digits(s + 1, &ok);
                let (e2, big) = if self.c(e) == b'n' {
                    (e + 1, Some(e))
                } else {
                    (e, None)
                };
                if self.boundary(e2) && self.c(e2) != b'$' {
                    return Some((e2, kind, big));
                }
            }
        }
        let dig = |c: u8| c.is_ascii_digit();
        let end_ok = |e: usize| self.c(e) != b'$';
        let with_n = |e: usize| -> (usize, Option<usize>) {
            if self.c(e) == b'n' {
                (e + 1, Some(e))
            } else {
                (e, None)
            }
        };
        let exp = |k: usize| -> Option<usize> {
            if !matches!(self.c(k), b'e' | b'E') {
                return None;
            }
            let mut m = k + 1;
            if matches!(self.c(m), b'+' | b'-') {
                m += 1;
            }
            if !dig(self.c(m)) {
                return None;
            }
            Some(digits(m + 1, &dig))
        };
        let c = self.c(i);
        if dig(c) && wb_start {
            let int = digits(i + 1, &dig);
            if self.c(int) == b'.' {
                let f = int + 1;
                // 1. 1.1E+3
                if dig(self.c(f)) {
                    let fe = digits(f + 1, &dig);
                    if let Some(x) = exp(fe) {
                        let (e, big) = with_n(x);
                        if self.boundary(e) && end_ok(e) {
                            return Some((e, DECIMAL, big));
                        }
                    }
                }
                // 2. 1.E+3
                if let Some(x) = exp(f) {
                    let (e, big) = with_n(x);
                    if self.boundary(e) && end_ok(e) {
                        return Some((e, DECIMAL, big));
                    }
                }
            }
            // 4. 1E+3
            if let Some(x) = exp(int) {
                let (e, big) = with_n(x);
                if self.boundary(e) && end_ok(e) {
                    return Some((e, DECIMAL, big));
                }
            }
            if self.c(int) == b'.' {
                let f = int + 1;
                // 5. 1.1
                if dig(self.c(f)) {
                    let fe = digits(f + 1, &dig);
                    let (e, big) = with_n(fe);
                    if self.boundary(e) && end_ok(e) {
                        return Some((e, DECIMAL, big));
                    }
                }
                // 6. 1. then \B
                let (e, big) = with_n(f);
                if !self.boundary(e) && end_ok(e) {
                    return Some((e, DECIMAL, big));
                }
            }
            // 8. 1 then \b(?!\.)
            let (e, big) = with_n(int);
            if self.boundary(e) && self.c(e) != b'.' && end_ok(e) {
                return Some((e, DECIMAL, big));
            }
            return None;
        }
        if c == b'.' && !wb_start && dig(self.c(i + 1)) {
            let fe = digits(i + 2, &dig);
            // 3. .1E+3
            if let Some(x) = exp(fe) {
                let (e, big) = with_n(x);
                if self.boundary(e) && end_ok(e) {
                    return Some((e, DECIMAL, big));
                }
            }
            // 7. .1
            let (e, big) = with_n(fe);
            if self.boundary(e) && end_ok(e) {
                return Some((e, DECIMAL, big));
            }
        }
        None
    }

    /// `regex`'s two at the place.
    pub(super) fn regex(&mut self) -> bool {
        let i = self.i;
        let k = self.ws(i);
        if self.c(k) != b'/' || matches!(self.c(k + 1), b'/' | b'*') {
            return false;
        }
        let b = self.b;
        // the first: (?<!\+\+|--|})(?<=[=(:,\[?+!]|^return|…|^case|…|=>|&&|\|\||\*\/)\s*(\/)
        let not_before = !(i >= 2 && (&b[i - 2..i] == b"++" || &b[i - 2..i] == b"--"))
            && !(i >= 1 && b[i - 1] == b'}');
        let lb1 = i > 0
            && not_before
            && (matches!(
                b[i - 1],
                b'=' | b'(' | b':' | b',' | b'[' | b'?' | b'+' | b'!'
            ) || self.word_ends_at(i, &[b"return", b"case"])
                || (i >= 2 && matches!(&b[i - 2..i], b"=>" | b"&&" | b"||" | b"*/")));
        if lb1 && self.regex_body_ahead(k + 1, true) {
            // the blanks before the `/` are in the match, under the regex's scope
            self.push(REGEX_F);
            self.tok(k, k + 1, RE_BEGIN);
            self.i = k + 1;
            return true;
        }
        // the second: ((?<![_$[:alnum:])\]]|\+\+|--|}|\*\/)|((?<=^return|…|^case|…))\s*)\/
        if k == i {
            let blocked = i > 0
                && (self.alnum_before(i)
                    || matches!(b[i - 1], b')' | b']' | b'}')
                    || (i >= 2 && matches!(&b[i - 2..i], b"++" | b"--" | b"*/")));
            if !blocked && self.regex_body_ahead(k + 1, false) {
                self.tok(k, k + 1, RE_BEGIN);
                self.i = k + 1;
                self.push(REGEX_F);
                return true;
            }
        }
        if self.word_ends_at(i, &[b"return", b"case"]) && self.regex_body_ahead(k + 1, false) {
            // the blanks after `return` are the regex's begin too
            self.tok(i, k + 1, RE_BEGIN);
            self.i = k + 1;
            self.push(REGEX_F);
            return true;
        }
        false
    }

    /// A regex's body and end ahead from `i` (after its `/`):
    /// first: `(?:[^\/\\\[\()]|\\.|\[([^\]\\]|\\.)+\]|\(([^\)\\]|\\.)+\))+\/(flags)`;
    /// second: `(?:[^\/\\\[]|\\.|\[([^\]\\]|\\.)*\])+\/(flags)`; then `([dgimsuvy]+|(?![\/\*])|(?=\/\*))(?!\s*[a-zA-Z0-9_$])` (ECMAScript's `v` too).
    pub(super) fn regex_body_ahead(&self, i: usize, first: bool) -> bool {
        let mut j = i;
        let mut any = false;
        loop {
            match self.c(j) {
                0 => return false,
                b'/' => break,
                b'\\' => {
                    if j + 1 > self.b.len() || self.c(j + 1) == b'\n' {
                        return false;
                    }
                    j += 2;
                }
                b'[' => {
                    let mut k = j + 1;
                    let mut inner = false;
                    loop {
                        match self.c(k) {
                            0 => return false,
                            b']' if inner || !first => break,
                            b']' => return false,
                            b'\\' => {
                                if self.c(k + 1) == b'\n' || self.c(k + 1) == 0 {
                                    return false;
                                }
                                k += 2;
                                inner = true;
                                continue;
                            }
                            _ => k += 1,
                        }
                        inner = true;
                    }
                    j = k + 1;
                }
                b'(' if first => {
                    let mut k = j + 1;
                    let mut inner = false;
                    loop {
                        match self.c(k) {
                            0 => return false,
                            b')' if inner => break,
                            b')' => return false,
                            b'\\' => {
                                if self.c(k + 1) == b'\n' || self.c(k + 1) == 0 {
                                    return false;
                                }
                                k += 2;
                            }
                            _ => k += 1,
                        }
                        inner = true;
                    }
                    j = k + 1;
                }
                b')' if first => return false,
                b'\n' => {
                    // `[^…]` takes the `\n`, but no `/` can follow
                    return false;
                }
                c => j += char_len(c),
            }
            any = true;
        }
        if !any {
            return false;
        }
        let mut e = j + 1;
        let fs = e;
        while matches!(
            self.c(e),
            b'd' | b'g' | b'i' | b'm' | b's' | b'u' | b'v' | b'y'
        ) {
            e += 1;
        }
        if e == fs
            && matches!(self.c(e), b'/' | b'*')
            && !(self.c(e) == b'/' && self.c(e + 1) == b'*')
        {
            return false;
        }
        let k = self.ws(e);
        !matches!(self.c(k), b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'$')
    }

    /// `support-objects` at the place.
    pub(super) fn support_objects(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        if let Some((de, dk)) = self.dot(i) {
            if c == b'.' && self.c(i + 1) == b'.' {
                return false;
            }
            let s = self.ws(de);
            let ne = self.name(s);
            if ne == s {
                return false;
            }
            let w = &self.b[s..ne];
            let no_call = |me: &Self| {
                // \b(?!\$|\s*(«TP»)?\()
                let k = me.ws(ne);
                let t = if me.c(k) == b'<' {
                    look::tp(me.b, k)
                } else {
                    Some(k)
                };
                !(t.is_some_and(|t| me.c(t) == b'('))
            };
            let call = self.c(self.ws(ne)) == b'(';
            let cl = words::class(w);
            if cl & words::AT_DOT == 0 {
                return false;
            }
            let Some(kind) = member_support(w, cl, call, no_call(self)) else {
                return false;
            };
            self.tok(i, de, dk);
            return self.take(s, ne, kind);
        }
        if look::alpha(self.b, i) == 0 || !self.wb(i) {
            return false;
        }
        let ne = self.name(i);
        let w = &self.b[i..ne];
        let bend = self.boundary(ne) && self.c(ne) != b'$';
        if !bend {
            return false;
        }
        let cl = words::class(w);
        if cl == 0 {
            return false;
        }
        if w == b"arguments" {
            return self.take(i, ne, ARGUMENTS);
        }
        if cl & words::BUILTIN_CLASS != 0 {
            return self.take(i, ne, BUILTIN_CLASS);
        }
        if w.ends_with(b"Error")
            && matches!(
                &w[..w.len() - 5],
                b"" | b"Eval"
                    | b"Internal"
                    | b"Range"
                    | b"Reference"
                    | b"Syntax"
                    | b"Type"
                    | b"URI"
            )
        {
            return self.take(i, ne, ERROR_CLASS);
        }
        if w == b"Promise" {
            return self.take(i, ne, PROMISE);
        }
        if cl & words::SUPPORT_FN != 0 && self.c(self.ws(ne)) == b'(' {
            return self.take(i, ne, SUPPORT_FN);
        }
        // (Math|console|JSON)(?:\s*(\.|\?\.)\s*(member))?\b(?!\$)
        let member = |me: &Self| -> Option<(usize, u16, usize, usize)> {
            let k = me.ws(ne);
            let (de, dk) = me.dot(k)?;
            if me.c(k) == b'.' && me.c(k + 1) == b'.' {
                return None;
            }
            let s = me.ws(de);
            let e = me.name(s);
            (e > s && me.boundary(e) && me.c(e) != b'$')
                .then_some((k, dk, de, s))
                .map(|(k, dk, _de, s)| (k, dk, s, e))
        };
        let with_member =
            |me: &mut Self, kind: u16, check: &dyn Fn(&[u8]) -> Option<u16>| -> bool {
                if let Some((k, dk, s, e)) = member(me)
                    && let Some(mk) = check(&me.b[s..e])
                {
                    me.tok(i, ne, kind);
                    let de = if dk == ACCESSOR { k + 1 } else { k + 2 };
                    me.tok(k, de, dk);
                    return me.take(s, e, mk);
                }
                me.take(i, ne, kind)
            };
        match w {
            b"Math" => {
                return with_member(self, MATH, &|m| {
                    if words::class(m) & words::MATH_FN != 0 {
                        Some(MATH_FN)
                    } else if words::class(m) & words::MATH_CONST != 0 {
                        Some(MATH_CONST)
                    } else {
                        None
                    }
                });
            }
            b"console" => {
                return with_member(self, CONSOLE, &|m| {
                    (words::class(m) & words::CONSOLE_FN != 0).then_some(CONSOLE_FN)
                });
            }
            b"JSON" => {
                return with_member(self, JSON, &|m| {
                    matches!(m, b"parse" | b"stringify").then_some(JSON_FN)
                });
            }
            b"import" | b"new" => {
                let (k1, want, kind) = if w == b"import" {
                    (IMPORT, &b"meta"[..], IMPORT_META)
                } else {
                    (NEW, &b"target"[..], NEW_TARGET)
                };
                if let Some((k, dk, s, e)) = member(self)
                    && &self.b[s..e] == want
                {
                    self.tok(i, ne, k1);
                    let de = if dk == ACCESSOR { k + 1 } else { k + 2 };
                    self.tok(k, de, dk);
                    return self.take(s, e, kind);
                }
                return false;
            }
            _ => {}
        }
        if cl & words::DOM_VAR != 0 {
            return self.take(i, ne, DOM_VAR);
        }
        if cl & words::DOM_CLASS != 0 {
            return self.take(i, ne, DOM_CLASS);
        }
        if cl & words::NODE_CLASS != 0 {
            return self.take(i, ne, NODE_CLASS);
        }
        if w == b"process" {
            // (process)(?:(\.|\?\.)(?:(props)|(fns)))?\b(?!\$): no blanks around the dot
            if let Some((de, dk)) = self.dot(ne)
                && !(self.c(ne) == b'.' && self.c(ne + 1) == b'.')
            {
                let e = self.name(de);
                let m = &self.b[de..e];
                if e > de && self.boundary(e) && self.c(e) != b'$' {
                    let mk = if words::class(m) & words::PROCESS_PROP != 0 {
                        Some(PROCESS_PROP)
                    } else if words::class(m) & words::PROCESS_FN != 0 {
                        Some(PROCESS_FN)
                    } else {
                        None
                    };
                    if let Some(mk) = mk {
                        self.tok(i, ne, PROCESS);
                        self.tok(ne, de, dk);
                        return self.take(de, e, mk);
                    }
                }
            }
            return self.take(i, ne, PROCESS);
        }
        if w == b"exports" {
            return self.take(i, ne, MODULE);
        }
        if w == b"module" {
            if let Some((de, dk)) = self.dot(ne)
                && !(self.c(ne) == b'.' && self.c(ne + 1) == b'.')
            {
                let e = self.name(de);
                if e > de
                    && matches!(
                        &self.b[de..e],
                        b"exports" | b"id" | b"filename" | b"loaded" | b"parent" | b"children"
                    )
                    && self.boundary(e)
                    && self.c(e) != b'$'
                {
                    self.tok(i, ne, MODULE);
                    self.tok(ne, de, dk);
                    return self.take(de, e, MODULE);
                }
            }
            return self.take(i, ne, MODULE);
        }
        if matches!(
            w,
            b"global" | b"GLOBAL" | b"root" | b"__dirname" | b"__filename"
        ) {
            return self.take(i, ne, NODE_OBJ);
        }
        false
    }

    /// `object-identifiers` at the place.
    pub(super) fn object_identifiers(&mut self) -> bool {
        let i = self.i;
        // a dot, then a name before another dot and name
        if let Some((de, dk)) = self.dot(i) {
            if self.c(i) == b'.' && self.c(i + 1) == b'.' {
                return false;
            }
            let s = self.ws(de);
            let s2 = if self.c(s) == b'#' { s + 1 } else { s };
            let ne = self.name(s2);
            if ne > s2 && self.dot_name_after(ne) {
                let u = look::upper(self.b, s2);
                let kind = if s2 == s && u == ne || s2 > s && u == ne {
                    CONSTANT_OBJ_PROP
                } else {
                    OBJECT_PROP
                };
                self.tok(i, de, dk);
                return self.take(s, ne, kind);
            }
            return false;
        }
        let ne = self.name(i);
        if ne == i {
            return false;
        }
        // name(?=\s*\??\.\s*prototype\b(?!\$))
        {
            let mut k = self.ws(ne);
            if self.c(k) == b'?' {
                k += 1;
            }
            if self.c(k) == b'.' {
                let m = self.ws(k + 1);
                if let Some(e) = self.word_b(m, b"prototype")
                    && self.c(e) != b'$'
                {
                    return self.take(i, ne, SUPPORT_CLASS);
                }
            }
        }
        if self.dot_name_after(ne) {
            let u = look::upper(self.b, i);
            return self.take(i, ne, if u == ne { CONSTANT_OBJ } else { OBJECT });
        }
        false
    }

    /// `(?=\s*\??\.\s*\#?«name»)` at `i`.
    pub(super) fn dot_name_after(&self, i: usize) -> bool {
        let mut k = self.ws(i);
        if self.c(k) == b'?' {
            k += 1;
        }
        if self.c(k) != b'.' {
            return false;
        }
        let mut m = self.ws(k + 1);
        if self.c(m) == b'#' {
            m += 1;
        }
        self.name(m) > m
    }

    /// `identifiers` at the place.
    pub(super) fn identifiers(&mut self) -> bool {
        if self.object_identifiers() {
            return true;
        }
        let i = self.i;
        // (?:(\.)|(\?\.(?!\s*[[:digit:]])))\s*)?(name)(?=\s*=\s*FNV)
        let (s, dot) = match self.dot(i) {
            Some((de, dk)) if !(self.c(i) == b'.' && self.c(i + 1) == b'.') => {
                (self.ws(de), Some((de, dk)))
            }
            _ => (i, None),
        };
        let ne = self.name(s);
        if ne > s {
            let k = self.ws(ne);
            if self.c(k) == b'='
                && self.c(k + 1) != b'='
                && look::fn_value(self.b, self.ws(k + 1), self.tsx)
            {
                if let Some((de, dk)) = dot {
                    self.tok(i, de, dk);
                }
                return self.take(s, ne, ENTITY_FN);
            }
        }
        if let Some((de, dk)) = dot {
            let s2 = if self.c(s) == b'#' { s + 1 } else { s };
            let ne = self.name(s2);
            if ne == s2 {
                return false;
            }
            let u = look::upper(self.b, s2);
            self.tok(i, de, dk);
            return self.take(s, ne, if u == ne { CONSTANT_PROP } else { PROPERTY });
        }
        if ne == i {
            return false;
        }
        let u = look::upper(self.b, i);
        self.take(i, ne, if u == ne { CONSTANT } else { READWRITE })
    }

    /// `object-member` at the place.
    pub(super) fn object_member(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        if self.comment(false) {
            return true;
        }
        if self.obj_method_head(false) {
            return true;
        }
        match c {
            b'[' => return self.push_zero(OBJ_KEY_SQ),
            b'\'' | b'"' | b'`' => return self.push_zero(OBJ_KEY_STR),
            _ => {}
        }
        if (c.is_ascii_digit() || c == b'.') && self.number_end(i).is_some() {
            return self.push_zero(OBJ_KEY_NUM);
        }
        // (?<=[\]\'\"\`])(?=\s*[\(\<])
        if i > 0
            && matches!(self.b[i - 1], b']' | b'\'' | b'"' | b'`')
            && matches!(self.c(self.ws(i)), b'(' | b'<')
        {
            return self.push_zero(OBJ_KEY_METHOD);
        }
        // (?![_$[:alpha:]])([[:digit:]]+)\s*(?=«cm*»:)
        if c.is_ascii_digit() {
            let mut e = i;
            while self.c(e).is_ascii_digit() {
                e += 1;
            }
            let k = look::comments(self.b, self.ws(e));
            if self.c(k) == b':' {
                return self.take(i, e, DECIMAL);
            }
        }
        let ne = self.name(i);
        if ne > i {
            let k = look::comments(self.b, self.ws(ne));
            if self.c(k) == b':' {
                // name\s*(?=«cm*»:(\s*«cm*»)*\s*FNV)
                let v = look::comments(self.b, self.ws(k + 1));
                if look::fn_value(self.b, self.ws(v), self.tsx) {
                    return self.take(i, ne, ENTITY_FN);
                }
                self.i = self.ws(ne).min(self.b.len());
                return true;
            }
        }
        if c == b'.' && self.c(i + 1) == b'.' && self.c(i + 2) == b'.' {
            self.tok(i, i + 3, SPREAD);
            self.i = i + 3;
            self.push(OBJ_SPREAD);
            return true;
        }
        if ne > i {
            // name\s*(?=,|\}|$|\/\/|\/\*)
            let k = self.ws(ne);
            if matches!(self.c(k), b',' | b'}')
                || k >= self.b.len()
                || (self.c(k) == b'/' && matches!(self.c(k + 1), b'/' | b'*'))
            {
                self.tok(i, ne, READWRITE);
                self.i = k.min(self.b.len());
                return true;
            }
        }
        if let Some(e) = self.kw(i, b"as") {
            let k = self.ws(e);
            if k > e {
                if let Some(ce) = self.word_b(k, b"const") {
                    let m = self.ws(ce);
                    if m >= self.b.len() || matches!(self.c(m), b',' | b'}') {
                        self.tok(i, e, AS);
                        return self.take(k, ce, MODIFIER);
                    }
                }
                self.tok(i, e, AS);
                self.i = k.min(self.b.len());
                self.push(OBJ_AS);
                return true;
            }
        }
        if ne > i {
            let k = self.ws(ne);
            if self.c(k) == b'=' {
                return self.push_zero(OBJ_ASSIGN);
            }
        }
        match c {
            b':' => {
                self.tok(i, i + 1, KEY_VALUE);
                self.i = i + 1;
                self.push(OBJ_VAL);
                true
            }
            b',' => self.take(i, i + 1, COMMA),
            _ => false,
        }
    }

    /// `object-literal-method-declaration`'s head: `«wb»(async\s+)?(get|set\s+)?(\*\s*)?` before a
    /// key and `(`. In the method's frame (`again`) it doesn't push.
    pub(super) fn obj_method_head(&mut self, again: bool) -> bool {
        let i = self.i;
        if !self.wb(i) {
            return false;
        }
        // (async\s+)?((get|set)\s+)?(\*\s*)? before a key and `(`, each optional group taken
        // where it matches and dropped when the key isn't found after it, the regex's
        // backtracking: `async (x)` is a method named `async`
        let word = |me: &Self, k: usize, w: &[u8]| {
            let e = k + w.len();
            (me.has(k, w) && me.boundary(k) && me.boundary(e) && me.ws(e) > e).then(|| me.ws(e))
        };
        let mut found = None;
        // without `a`, `g`, `s` or `*` at the place, only the bare key can match
        if !matches!(self.c(i), b'a' | b'g' | b's' | b'*') {
            if !self.method_key_ahead(i) {
                return false;
            }
            found = Some((None, None, None, i));
        }
        'search: for asy in [true, false] {
            if found.is_some() {
                break;
            }
            let k1 = match (asy, word(self, i, b"async")) {
                (true, Some(m)) => m,
                (true, None) => continue,
                (false, _) => i,
            };
            for acc in [true, false] {
                let k2 = match (
                    acc,
                    word(self, k1, b"get").or_else(|| word(self, k1, b"set")),
                ) {
                    (true, Some(m)) => m,
                    (true, None) => continue,
                    (false, _) => k1,
                };
                for star in [true, false] {
                    let k3 = match (star, self.c(k2) == b'*') {
                        (true, true) => self.ws(k2 + 1),
                        (true, false) => continue,
                        (false, _) => k2,
                    };
                    if self.method_key_ahead(k3) {
                        found = Some((asy.then_some(i), acc.then_some(k1), star.then_some(k2), k3));
                        break 'search;
                    }
                }
            }
        }
        let Some((asy, acc, star, k)) = found else {
            return false;
        };
        if let Some(a) = asy {
            self.tok(a, a + 5, ASYNC);
        }
        if let Some(a) = acc {
            self.tok(a, a + 3, PROPERTY_KW);
        }
        if let Some(s) = star {
            self.tok(s, s + 1, ASTERISK);
        }
        self.i = k.min(self.b.len());
        if again {
            if k == i {
                return self.method_name_here();
            }
            return true;
        }
        if k == i {
            self.push_zero(OBJ_METHOD)
        } else {
            self.push(OBJ_METHOD);
            true
        }
    }

    /// The method name pushed at the place, in an object's method.
    pub(super) fn method_name_here(&mut self) -> bool {
        self.push_zero(METHOD_NAME)
    }

    /// An object member's value that starts an arrow's parameters (`object-member`'s value's own).
    pub(super) fn obj_value_arrow(&mut self) -> bool {
        let i = self.i;
        let after_colon = i > 0 && self.b[i - 1] == b':';
        let after_angle = i > 0 && self.b[i - 1] == b'>';
        if !after_colon && !after_angle {
            return false;
        }
        let mut k = self.ws(i);
        let mut asy = None;
        if after_colon && self.has(k, b"async") {
            asy = Some(k);
            k = self.ws(k + 5);
        }
        if after_colon {
            // (?=\s*«TP»\(\s*OPEN)
            if self.c(k) == b'<'
                && let Some(t) = look::tp(self.b, k)
                && self.c(t) == b'('
                && look::open_tail(self.b, self.ws(t + 1))
            {
                if let Some(a) = asy {
                    self.tok(a, a + 5, ASYNC);
                    self.i = a + 5;
                }
                if self.i == i {
                    return self.push_zero(MAYBE_ARROW);
                }
                self.push(MAYBE_ARROW);
                return true;
            }
            // \s*(async)?\s*(\()(?=\s*OPEN)
            if self.c(k) == b'(' && look::open_tail(self.b, self.ws(k + 1)) {
                if let Some(a) = asy {
                    self.tok(a, a + 5, ASYNC);
                }
                self.tok(k, k + 1, ROUND);
                self.i = k + 1;
                self.push(OBJ_ARROW_PARENS);
                return true;
            }
            // \s*(async)?\s*(?=\<\s*$)
            if self.c(k) == b'<' && self.ws(k + 1) >= self.b.len() {
                if let Some(a) = asy {
                    self.tok(a, a + 5, ASYNC);
                }
                self.i = k;
                self.push(OBJ_ARROW_TP);
                return true;
            }
            return false;
        }
        // (?<=\>)\s*(\()(?=\s*OPEN)
        if self.c(k) == b'(' && look::open_tail(self.b, self.ws(k + 1)) {
            self.tok(k, k + 1, ROUND);
            self.i = k + 1;
            self.push(OBJ_ARROW_PARENS);
            return true;
        }
        false
    }

    /// `expression-inside-possibly-arrow-parens` at the place.
    pub(super) fn inside_arrow_parens(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        if self.expression_without_identifiers() {
            return true;
        }
        if self.comment(false) || self.string() || self.decorator_expr() {
            return true;
        }
        if self.destructuring_param_here() {
            return true;
        }
        if self.inside_param_name() {
            return true;
        }
        match c {
            b':' if self.type_annotation() => return true,
            b'=' if self.initializer() => return true,
            b',' => return self.take(i, i + 1, PARAM_SEP),
            _ => {}
        }
        self.identifiers() || self.expression_punctuations()
    }

    /// `decorator` in an expression's parameters.
    pub(super) fn decorator_expr(&mut self) -> bool {
        let i = self.i;
        if self.c(i) != b'@' || !self.wb(i) {
            return false;
        }
        self.tok(i, i + 1, DECORATOR);
        self.i = i + 1;
        self.push(DECORATOR_F);
        true
    }

    /// `destructuring-parameter` in an expression's parameters.
    pub(super) fn destructuring_param_here(&mut self) -> bool {
        let i = self.i;
        if i > 0 && matches!(self.b[i - 1], b'=' | b':') {
            return false;
        }
        let mut j = self.ws(i);
        let mut rest = None;
        if self.has(j, b"...") {
            rest = Some(j);
            j = self.ws(j + 3);
        }
        let c = self.c(j);
        if !matches!(c, b'{' | b'[') {
            return false;
        }
        if let Some(r) = rest {
            self.tok(r, r + 3, REST);
        }
        if c == b'{' {
            self.tok(j, j + 1, BIND_OBJ);
            self.i = j + 1;
            self.push(PBIND_OBJ);
        } else {
            self.tok(j, j + 1, BIND_ARR);
            self.i = j + 1;
            self.push(PBIND_ARR);
        }
        true
    }

    /// The parameter names of `expression-inside-possibly-arrow-parens`: a modifier before
    /// another, a name that is a function, a name before `:` or `,` or the line's end.
    pub(super) fn inside_param_name(&mut self) -> bool {
        let i = self.i;
        if !self.wb(i) {
            return false;
        }
        let mods: &[&[u8]] = &[
            b"override",
            b"public",
            b"protected",
            b"private",
            b"readonly",
        ];
        for m in mods {
            if self.has(i, m) && self.boundary(i + m.len()) {
                let k = self.ws(i + m.len());
                if k > i + m.len()
                    && mods
                        .iter()
                        .any(|m2| self.has(k, m2) && is_space(self.c(k + m2.len())))
                {
                    return self.take(i, i + m.len(), MODIFIER);
                }
            }
        }
        let mut j = i;
        let mut modifier = None;
        for m in mods {
            if self.has(j, m) && self.boundary(j + m.len()) {
                let k = self.ws(j + m.len());
                // the modifier only before a name (else it is the name)
                let r = if self.has(k, b"...") {
                    self.ws(k + 3)
                } else {
                    k
                };
                if k > j + m.len() && self.name(r) > r {
                    modifier = Some((j, j + m.len()));
                    j = k;
                    break;
                }
            }
        }
        let mut rest = None;
        if self.has(j, b"...") {
            rest = Some(j);
            j = self.ws(j + 3);
        }
        if j > 0 && matches!(self.c(j - 1), b'=' | b':') {
            return false;
        }
        if !self.wb(j) {
            return false;
        }
        let ne = self.name(j);
        if ne == j || !self.we(ne) {
            return false;
        }
        let this = self.is_name(j, ne, b"this");
        let q = self.ws(ne);
        let opt = if self.c(q) == b'?' { Some(q) } else { None };
        let after = opt.map_or(q, |o| o + 1);
        let fun = look::fn_decl(self.b, after, self.tsx);
        let plain = {
            let k = self.ws(after);
            matches!(self.c(k), b':' | b',') || after >= self.b.len()
        };
        if !fun && !plain {
            return false;
        }
        if let Some((s, e)) = modifier {
            self.tok(s, e, MODIFIER);
        }
        if let Some(r) = rest {
            self.tok(r, r + 3, REST);
        }
        let kind = match (fun, this) {
            (true, true) => THIS_FN,
            (true, false) => ENTITY_FN,
            (false, true) => THIS_PARAM,
            (false, false) => PARAM,
        };
        self.tok(j, ne, kind);
        if let Some(o) = opt {
            self.tok(o, o + 1, OPTIONAL);
            self.i = o + 1;
        } else {
            self.i = q.min(self.b.len());
        }
        true
    }

    /// Whether a numeric key's frame ends at `i` before `as`: `(\s+as\s+)`.
    pub(super) fn as_ahead(&self, i: usize) -> bool {
        let k = self.ws(i);
        k > i && self.word_b(k, b"as").is_some_and(|e| is_space(self.c(e)))
    }

    /// A quoted key's end after its quote: `(?=((\s*[\(\<,}])|(\s+(as)\s+)))`.
    pub(super) fn key_str_end(&self, i: usize) -> bool {
        matches!(self.c(self.ws(i)), b'(' | b'<' | b',' | b'}') || self.as_ahead(i)
    }

    /// `new-expr`'s end at `i`: `(?=[;),}\]:?\-\+\>]|\|\||\&\&|\!\=\=|$|(«wb»new«we»)|(«wb»function((\s+name)|(\s*[\(]))))`.
    pub(super) fn new_end(&self, i: usize) -> bool {
        let c = self.c(i);
        if matches!(
            c,
            b';' | b')' | b',' | b'}' | b']' | b':' | b'?' | b'-' | b'+' | b'>'
        ) || i >= self.b.len()
        {
            return true;
        }
        let r = self.rest(i);
        if crate::starts(r, b"||") || crate::starts(r, b"&&") || crate::starts(r, b"!==") {
            return true;
        }
        if self.kw(i, b"new").is_some() {
            return true;
        }
        if self.wb(i) && crate::starts(r, b"function") {
            let e = i + 8;
            let k = self.ws(e);
            if (k > e && self.name(k) > k) || self.c(k) == b'(' {
                return true;
            }
        }
        false
    }
}
