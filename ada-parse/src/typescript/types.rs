//! Types: `type`'s patterns, and the frames that hold one (an annotation, a return type, `as`,
//! type arguments and parameters, a type object or tuple, a function type).

use super::*;

impl Scan<'_, '_, '_> {
    /// `type-annotation` at `:`.
    pub(super) fn type_annotation(&mut self) -> bool {
        let i = self.i;
        if self.c(i) != b':' {
            return false;
        }
        self.tok(i, i + 1, ANNOTATION);
        self.i = i + 1;
        let k = self.ws(i + 1);
        self.push(if k < self.n { TYPE_ANNOT } else { TYPE_ANNOT_EOL });
        true
    }

    /// `return-type`: `(?<=\))\s*(:)`.
    pub(super) fn return_type(&mut self) -> bool {
        let i = self.i;
        if i == 0 || self.b[i - 1] != b')' {
            return false;
        }
        let k = self.ws(i);
        if self.c(k) != b':' {
            return false;
        }
        self.tok(k, k + 1, ANNOTATION);
        self.i = k + 1;
        let m = self.ws(k + 1);
        self.push(if m < self.n { RETURN_TYPE } else { RETURN_TYPE_EOL });
        true
    }

    /// `arrow-return-type`: `(?<=\))\s*(:)` in an arrow's head.
    pub(super) fn arrow_return_type(&mut self) -> bool {
        let i = self.i;
        if i == 0 || self.b[i - 1] != b')' {
            return false;
        }
        let k = self.ws(i);
        if self.c(k) != b':' {
            return false;
        }
        self.tok(k, k + 1, ANNOTATION);
        self.i = k + 1;
        self.push(ARROW_RET);
        true
    }

    /// `type-function-return-type` at `=>`.
    pub(super) fn type_function_return(&mut self) -> bool {
        let i = self.i;
        if !(self.c(i) == b'=' && self.c(i + 1) == b'>') {
            return false;
        }
        self.tok(i, i + 2, ARROW);
        self.i = i + 2;
        let k = self.ws(i + 2);
        self.push(if k < self.n { TYPE_FN_RET } else { TYPE_FN_RET_EOL });
        true
    }

    /// `type-parameters` at `<`.
    pub(super) fn type_parameters(&mut self) -> bool {
        let i = self.i;
        if self.c(i) != b'<' {
            return false;
        }
        self.tok(i, i + 1, TP_BEGIN);
        self.i = i + 1;
        self.push(TYPE_PARAMS);
        true
    }

    /// `type-arguments` at `<`.
    pub(super) fn type_arguments(&mut self) -> bool {
        let i = self.i;
        if self.c(i) != b'<' {
            return false;
        }
        self.tok(i, i + 1, TP_BEGIN);
        self.i = i + 1;
        self.push(TYPE_ARGS);
        true
    }

    /// `(?<![:|&])` at `i`.
    pub(super) fn not_after_op(&self, i: usize) -> bool {
        i == 0 || !matches!(self.b[i - 1], b':' | b'|' | b'&')
    }

    /// `(?<=[\}>\]\)]|[_$[:alpha:]])\s*(?=\{)` at `i`.
    pub(super) fn before_brace(&self, i: usize) -> bool {
        i > 0
            && (matches!(self.b[i - 1], b'}' | b'>' | b']' | b')') || self.alnum_before(i) && !self.b[i - 1].is_ascii_digit())
            && self.c(self.ws(i)) == b'{'
    }

    /// `(?<=\S)(?=\s*$)` at `i`.
    pub(super) fn last_token_end(&self, i: usize) -> bool {
        i > 0 && !is_space(self.b[i - 1]) && self.ws(i) >= self.n
    }

    /// A frame of a type's turn.
    pub(super) fn type_step(&mut self, f: u8) -> bool {
        let i = self.i;
        let c = self.c(i);
        match f {
            TYPE_ANNOT => {
                if self.not_after_op(i)
                    && (i >= self.n || i == 0 || matches!(c, b',' | b')' | b';' | b'}' | b']') || (c == b'/' && self.c(i + 1) == b'/')
                        || (c == b'=' && self.c(i + 1) != b'>' && i + 1 <= self.n) || self.before_brace(i))
                    && self.pop_zero()
                {
                    return true;
                }
                self.type_()
            }
            TYPE_ANNOT_EOL => {
                if self.not_after_op(i)
                    && (matches!(c, b',' | b')' | b';' | b'}' | b']') || (c == b'/' && self.c(i + 1) == b'/')
                        || (c == b'=' && self.c(i + 1) != b'>') || self.blank_at(i) || self.last_token_end(i) || self.before_brace(i))
                    && self.pop_zero()
                {
                    return true;
                }
                self.type_()
            }
            PARAM_TYPE => {
                if (matches!(c, b',' | b')') || (c == b'=' && self.c(i + 1) != b'>')) && self.pop_zero() {
                    return true;
                }
                self.type_()
            }
            RETURN_TYPE => {
                if self.not_after_op(i)
                    && (i >= self.n || i == 0 || matches!(c, b'{' | b'}' | b';' | b',') || (c == b'/' && self.c(i + 1) == b'/'))
                    && self.pop_zero()
                {
                    return true;
                }
                self.return_type_core()
            }
            RETURN_TYPE_EOL => {
                if self.not_after_op(i)
                    && (matches!(c, b'{' | b'}' | b';' | b',') || (c == b'/' && self.c(i + 1) == b'/') || self.blank_at(i) || self.last_token_end(i))
                    && self.pop_zero()
                {
                    return true;
                }
                self.return_type_core()
            }
            ARROW_RET | MAYBE_ARROW_RET => {
                if ((c == b'=' && self.c(i + 1) == b'>') || c == b'{' || self.arrow_bol(i)) && self.pop_zero() {
                    return true;
                }
                // arrow-return-type-body: (?<=[:])(?=\s*\{)
                if i > 0 && self.b[i - 1] == b':' && self.c(self.ws(i)) == b'{' {
                    self.i = self.ws(i);
                    self.push(RET_OBJ);
                    return true;
                }
                self.type_predicate() || self.type_()
            }
            RET_OBJ | TYPE_OP_OBJ | TYPE_RET_OBJ => {
                if i > 0 && self.b[i - 1] == b'}' && self.pop_zero() {
                    return true;
                }
                if c == b'{' {
                    self.tok(i, i + 1, BLOCK_BRACE);
                    self.i = i + 1;
                    self.push(TYPE_OBJ);
                    return true;
                }
                false
            }
            TYPE_FN_RET => {
                let not_arrow = !(i >= 2 && &self.b[i - 2..i] == b"=>");
                if not_arrow
                    && (i == 0 || !matches!(self.b[i - 1], b'|' | b'&'))
                    && (matches!(c, b',' | b']' | b')' | b'{' | b'}' | b'=' | b';' | b'>' | b':' | b'?') || (c == b'/' && self.c(i + 1) == b'/') || i >= self.n)
                    && self.pop_zero()
                {
                    return true;
                }
                self.type_fn_return_core()
            }
            TYPE_FN_RET_EOL => {
                let not_arrow = !(i >= 2 && &self.b[i - 2..i] == b"=>");
                if not_arrow
                    && (i == 0 || !matches!(self.b[i - 1], b'|' | b'&'))
                    && (matches!(c, b',' | b']' | b')' | b'{' | b'}' | b'=' | b';' | b':' | b'?' | b'>') || (c == b'/' && self.c(i + 1) == b'/')
                        || self.blank_at(i) || self.last_token_end(i))
                    && self.pop_zero()
                {
                    return true;
                }
                self.type_fn_return_core()
            }
            TYPE_ARGS | TYPE_NAME_ARGS => {
                if c == b'>' {
                    self.tok(i, i + 1, TP_END);
                    self.i = i + 1;
                    return self.pop();
                }
                if let Some(e) = self.kw(i, b"_") {
                    return self.take(i, e, TYPE_OP);
                }
                self.type_() || (c == b',' && self.take(i, i + 1, COMMA))
            }
            TYPE_PARAMS => {
                if c == b'>' {
                    self.tok(i, i + 1, TP_END);
                    self.i = i + 1;
                    return self.pop();
                }
                if self.comment(false) {
                    return true;
                }
                if let Some(e) = self.kw(i, b"extends") {
                    return self.take(i, e, MODIFIER);
                }
                if self.type_() {
                    return true;
                }
                match c {
                    b',' => self.take(i, i + 1, COMMA),
                    b'=' if self.c(i + 1) != b'>' => self.take(i, i + 1, ASSIGN),
                    _ => false,
                }
            }
            TUPLE => {
                if c == b']' {
                    self.tok(i, i + 1, SQUARE);
                    self.i = i + 1;
                    return self.pop();
                }
                if c == b'.' && self.c(i + 1) == b'.' && self.c(i + 2) == b'.' {
                    return self.take(i, i + 3, REST);
                }
                if self.wb(i) {
                    let ne = self.name(i);
                    if ne > i {
                        let mut k = self.ws(ne);
                        let opt = if self.c(k) == b'?' {
                            let o = k;
                            k = self.ws(k + 1);
                            Some(o)
                        } else {
                            None
                        };
                        if self.c(k) == b':' {
                            self.tok(i, ne, LABEL);
                            if let Some(o) = opt {
                                self.tok(o, o + 1, OPTIONAL);
                            }
                            return self.take(k, k + 1, LABEL_SEP);
                        }
                    }
                }
                self.type_() || (c == b',' && self.take(i, i + 1, COMMA))
            }
            TYPE_OBJ => {
                if c == b'}' {
                    self.tok(i, i + 1, BLOCK_BRACE);
                    self.i = i + 1;
                    return self.pop();
                }
                self.type_object_member()
            }
            TYPE_SPREAD => {
                if (matches!(c, b'}' | b';' | b',') || i >= self.n || (i > 0 && self.b[i - 1] == b'}')) && self.pop_zero() {
                    return true;
                }
                self.type_()
            }
            TYPE_PAREN => {
                if c == b')' {
                    self.tok(i, i + 1, ROUND);
                    self.i = i + 1;
                    return self.pop();
                }
                if self.type_paren_param() {
                    return true;
                }
                if c == b':' && self.type_annotation() {
                    return true;
                }
                if c == b',' {
                    return self.take(i, i + 1, PARAM_SEP);
                }
                self.type_()
            }
            TYPE_FN | TYPE_CTOR => {
                if i > 0 && self.b[i - 1] == b')' && self.pop_zero() {
                    return true;
                }
                if c == b'(' {
                    self.tok(i, i + 1, PARAMS_BEGIN);
                    self.i = i + 1;
                    self.push(PARAMS);
                    return true;
                }
                false
            }
            TYPE_CTOR_TP => {
                if i > 0 && self.b[i - 1] == b'>' && self.pop_zero() {
                    return true;
                }
                self.comment(false) || (c == b'<' && self.type_parameters())
            }
            TYPE_COND => {
                if i > 0 && self.b[i - 1] == b':' && self.pop_zero() {
                    return true;
                }
                if c == b'?' {
                    self.tok(i, i + 1, TERNARY);
                    self.i = i + 1;
                    self.push(TYPE_COND_Q);
                    return true;
                }
                self.type_()
            }
            TYPE_COND_Q => {
                if c == b':' {
                    self.tok(i, i + 1, TERNARY);
                    self.i = i + 1;
                    return self.pop();
                }
                self.type_()
            }
            TYPE_OP_GAP => {
                if i < self.n && !is_space(c) && self.pop_zero() {
                    return true;
                }
                false
            }
            SUBST_TYPE => {
                if c == b'}' {
                    self.tok(i, i + 1, SUBST_END);
                    self.i = i + 1;
                    return self.pop();
                }
                self.type_()
            }
            TYPE_DECL | TYPE_DECL_VALUE | TYPE_DECL_INTRINSIC => {
                if (c == b'}' || c == b';' || self.blank_at(i) || self.decl_at_bol(i)) && self.pop_zero() {
                    return true;
                }
                if f != TYPE_DECL {
                    return self.type_();
                }
                if self.comment(false) {
                    return true;
                }
                if c == b'<' && self.type_parameters() {
                    return true;
                }
                if c == b'=' {
                    let k = self.ws(i + 1);
                    if let Some(e) = self.word_b(k, b"intrinsic")
                        && self.we(e)
                    {
                        self.tok(i, i + 1, ASSIGN);
                        self.tok(k, e, INTRINSIC);
                        self.i = e;
                        self.push(TYPE_DECL_INTRINSIC);
                        return true;
                    }
                    self.tok(i, i + 1, ASSIGN);
                    self.i = k.min(self.n);
                    self.push(TYPE_DECL_VALUE);
                    return true;
                }
                false
            }
            AS_F | OBJ_AS => {
                let ends = (f == AS_F && i == 0)
                    || (f == OBJ_AS && i == 0)
                    || matches!(c, b';' | b')' | b',' | b'}' | b']' | b':' | b'?' | b'-' | b'+' | b'>')
                    || i >= self.n
                    || self.rest(i).starts_with(b"||")
                    || self.rest(i).starts_with(b"&&")
                    || self.rest(i).starts_with(b"!==")
                    || (self.kw(i, b"as").is_some_and(|e| is_space(self.c(e))))
                    || (f == AS_F && is_space(c) && self.c(self.ws(i)) == b'<');
                if ends && self.pop_zero() {
                    return true;
                }
                self.type_()
            }
            INSTANCEOF_F => {
                let after_paren = i > 0 && self.b[i - 1] == b')';
                let ends = after_paren
                    || matches!(c, b';' | b')' | b',' | b'}' | b']' | b':' | b'?' | b'-' | b'+' | b'>')
                    || i >= self.n
                    || self.rest(i).starts_with(b"||")
                    || self.rest(i).starts_with(b"&&")
                    || self.rest(i).starts_with(b"!==")
                    || self.instanceof_again(i)
                    || self.function_ahead(i);
                if ends && self.pop_zero() {
                    return true;
                }
                self.type_()
            }
            CAST => {
                if c == b'>' {
                    self.tok(i, i + 1, ANGLE);
                    self.i = i + 1;
                    return self.pop();
                }
                self.type_()
            }
            INDEXER => {
                if c == b']' {
                    self.tok(i, i + 1, SQUARE);
                    let k = self.ws(i + 1);
                    if self.c(k) == b'?' {
                        self.tok(k, k + 1, OPTIONAL);
                        self.i = self.ws(k + 1).min(self.n);
                    } else {
                        self.i = k.min(self.n);
                    }
                    return self.pop();
                }
                if i >= self.n && self.pop_zero() {
                    return true;
                }
                c == b':' && self.type_annotation()
            }
            MAPPED => {
                if c == b']' {
                    self.tok(i, i + 1, SQUARE);
                    let mut k = i + 1;
                    if matches!(self.c(k), b'+' | b'-') {
                        self.tok(k, k + 1, TYPE_MODIFIER_OP);
                        k += 1;
                    }
                    let m = self.ws(k);
                    if self.c(m) == b'?' {
                        self.tok(m, m + 1, OPTIONAL);
                        k = self.ws(m + 1);
                    }
                    self.i = k.min(self.n);
                    return self.pop();
                }
                if i >= self.n && self.pop_zero() {
                    return true;
                }
                if let Some(e) = self.kw(i, b"as") {
                    let k = self.ws(e);
                    if k > e {
                        self.tok(i, e, AS);
                        self.i = k.min(self.n);
                        return true;
                    }
                }
                self.type_()
            }
            _ => false,
        }
    }

    /// `(([\&\~\^\|]\s*)?[_$[:alpha:]][_$[:alnum:]]*\s+instanceof«we»)` at `i`.
    pub(super) fn instanceof_again(&self, i: usize) -> bool {
        let mut j = i;
        if matches!(self.c(j), b'&' | b'~' | b'^' | b'|') {
            j = self.ws(j + 1);
        }
        let ne = self.name(j);
        if ne == j {
            return false;
        }
        let k = self.ws(ne);
        k > ne && self.rest(k).starts_with(b"instanceof") && self.we(k + 10)
    }

    /// `«wb»function((\s+name)|(\s*[\(]))` at `i`.
    pub(super) fn function_ahead(&self, i: usize) -> bool {
        if !(self.wb(i) && self.rest(i).starts_with(b"function")) {
            return false;
        }
        let e = i + 8;
        let k = self.ws(e);
        (k > e && self.name(k) > k) || self.c(k) == b'('
    }

    /// `^\s*(export|function|…)\s+` at `i`.
    pub(super) fn arrow_bol(&self, i: usize) -> bool {
        if i != 0 {
            return false;
        }
        let j = self.ws(0);
        [&b"export"[..], b"function", b"class", b"interface", b"let", b"var", b"const", b"import", b"enum", b"namespace", b"module", b"type", b"abstract", b"declare"]
            .iter()
            .any(|w| self.rest(j).starts_with(w) && is_space(self.c(j + w.len())))
    }

    /// `return-type-core`.
    pub(super) fn return_type_core(&mut self) -> bool {
        let i = self.i;
        if self.comment(false) {
            return true;
        }
        if i > 0 && matches!(self.b[i - 1], b':' | b'|' | b'&') && self.c(self.ws(i)) == b'{' {
            self.i = self.ws(i);
            self.push(RET_OBJ);
            return true;
        }
        self.type_predicate() || self.type_()
    }

    /// `type-function-return-type-core`.
    pub(super) fn type_fn_return_core(&mut self) -> bool {
        let i = self.i;
        if self.comment(false) {
            return true;
        }
        if i >= 2 && &self.b[i - 2..i] == b"=>" && self.c(self.ws(i)) == b'{' {
            self.i = self.ws(i);
            self.push(TYPE_RET_OBJ);
            return true;
        }
        self.type_predicate() || self.type_()
    }

    /// `type-predicate-operator`.
    pub(super) fn type_predicate(&mut self) -> bool {
        let i = self.i;
        if look::alpha(self.b, i) == 0 || !self.wb(i) {
            return false;
        }
        let mut j = i;
        let mut asserts = None;
        if let Some(e) = self.word_b(i, b"asserts") {
            let k = self.ws(e);
            if k > e {
                asserts = Some(e);
                j = k;
            }
        }
        // (?!asserts)(this|name)\s(is)«we»
        if self.word_b(j, b"asserts").is_none() || j == i && asserts.is_none() && self.word_b(j, b"asserts").is_none() {
            let ne = self.name(j);
            if ne > j && is_space(self.c(ne)) && self.rest(ne + 1).starts_with(b"is") && self.we(ne + 3) && !(asserts.is_none() && self.is_name(j, ne, b"asserts")) {
                if let Some(e) = asserts {
                    self.tok(i, e, ASSERTS);
                }
                let k = if self.is_name(j, ne, b"this") { THIS_PARAM } else { PARAM };
                self.tok(j, ne, k);
                return self.take(ne + 1, ne + 3, IS);
            }
        }
        if let Some(e) = asserts {
            // (asserts)\s+(?!is)(this|name)«we»
            if self.word_b(j, b"is").is_none() {
                let ne = self.name(j);
                if ne > j && self.we(ne) {
                    self.tok(i, e, ASSERTS);
                    let k = if self.is_name(j, ne, b"this") { THIS_PARAM } else { PARAM };
                    return self.take(j, ne, k);
                }
            }
        }
        if let Some(e) = self.kw(i, b"asserts") {
            return self.take(i, e, ASSERTS);
        }
        if let Some(e) = self.kw(i, b"is") {
            return self.take(i, e, IS);
        }
        false
    }

    /// `type` at the place.
    pub(super) fn type_(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        match c {
            b'/' => return self.comment(false),
            b'\'' | b'"' => return self.string(),
            b'`' => return self.template(true),
            b'0'..=b'9' | b'.' if self.numeric() => return true,
            b'<' => return self.type_parameters(),
            b'[' => {
                self.tok(i, i + 1, SQUARE);
                self.i = i + 1;
                self.push(TUPLE);
                return true;
            }
            b'{' => {
                self.tok(i, i + 1, BLOCK_BRACE);
                self.i = i + 1;
                self.push(TYPE_OBJ);
                return true;
            }
            _ => {}
        }
        if i == 0 && is_space(c) && self.comment(false) {
            return true;
        }
        let name_start = look::alpha(self.b, i) > 0;
        if name_start && self.template(true) {
            return true;
        }
        if name_start && self.wb(i) {
            let ne = self.name(i);
            let w = &self.b[i..ne];
            let we = self.we(ne);
            if we && matches!(w, b"string" | b"number" | b"bigint" | b"boolean" | b"symbol" | b"any" | b"void" | b"never" | b"unknown") {
                return self.take(i, ne, PRIMITIVE);
            }
            if we && matches!(w, b"this" | b"true" | b"false" | b"undefined" | b"null" | b"object") {
                return self.take(i, ne, BUILTIN_TYPE);
            }
            if w == b"extends" {
                let k = self.ws(ne);
                if k > ne {
                    self.tok(i, ne, MODIFIER);
                    self.i = k.min(self.n);
                    self.push(TYPE_COND);
                    return true;
                }
            }
            // type-operators' words
            if w == b"typeof" && we {
                self.tok(i, ne, TYPEOF);
                self.i = ne;
                self.push(TYPEOF_F);
                return true;
            }
            if w == b"keyof" && we {
                return self.take(i, ne, KEYOF);
            }
            if w == b"infer" {
                let k = self.ws(ne);
                if k > ne && look::alpha(self.b, k) > 0 {
                    return self.take(i, ne, INFER);
                }
            }
            if w == b"import" && self.c(self.ws(ne)) == b'(' {
                return self.take(i, ne, IMPORT_OP);
            }
        }
        match c {
            b'&' | b'|' => {
                let k = self.ws(i + 1);
                self.tok(i, i + 1, TYPE_OP);
                self.i = i + 1;
                if self.c(k) == b'{' {
                    self.push(TYPE_OP_OBJ);
                } else {
                    self.push(TYPE_OP_GAP);
                }
                return true;
            }
            b'?' | b':' => return self.take(i, i + 1, TERNARY),
            _ => {}
        }
        // type-fn-type-parameters
        if name_start && self.wb(i) {
            let mut k = i;
            let mut abs = None;
            if let Some(e) = self.word_b(i, b"abstract") {
                let m = self.ws(e);
                if m > e {
                    abs = Some(e);
                    k = m;
                }
            }
            if let Some(e) = self.word_b(k, b"new") {
                let m = self.ws(e);
                if self.c(m) == b'<' {
                    if let Some(a) = abs {
                        self.tok(i, a, MODIFIER);
                    }
                    self.tok(k, e, NEW_CONTROL);
                    self.i = e;
                    self.push(TYPE_CTOR_TP);
                    return true;
                }
                if self.c(m) == b'(' {
                    if let Some(a) = abs {
                        self.tok(i, a, MODIFIER);
                    }
                    self.tok(k, e, NEW_CONTROL);
                    self.i = m;
                    self.push(TYPE_CTOR);
                    return true;
                }
            }
        }
        if c == b'(' {
            // (?=[(]\s*(([)])|(\.\.\.)|([_$[:alnum:]]+\s*(([:,?=])|([)]\s*=>)))))
            let k = self.ws(i + 1);
            let fn_type = self.c(k) == b')' || self.rest(k).starts_with(b"...") || {
                let mut e = k;
                while look::alnum(self.b, e) > 0 {
                    e += look::alnum(self.b, e);
                }
                let f = self.ws(e);
                e > k && (matches!(self.c(f), b':' | b',' | b'?' | b'=') || (self.c(f) == b')' && {
                    let g = self.ws(f + 1);
                    self.c(g) == b'=' && self.c(g + 1) == b'>'
                }))
            };
            if fn_type {
                return self.push_zero(TYPE_FN);
            }
            self.tok(i, i + 1, ROUND);
            self.i = i + 1;
            self.push(TYPE_PAREN);
            return true;
        }
        if c == b'=' && self.c(i + 1) == b'>' {
            return self.type_function_return();
        }
        if name_start {
            if let Some(e) = self.kw(i, b"readonly") {
                self.tok(i, e, MODIFIER);
                self.i = self.ws(e).min(self.n);
                return true;
            }
            return self.type_name();
        }
        false
    }

    /// `type-name` at a name.
    pub(super) fn type_name(&mut self) -> bool {
        let i = self.i;
        let ne = self.name(i);
        if ne == i {
            return false;
        }
        let k = self.ws(ne);
        let dot = match self.c(k) {
            b'.' => Some((k + 1, ACCESSOR)),
            b'?' if self.c(k + 1) == b'.' && !self.c(self.ws(k + 2)).is_ascii_digit() => Some((k + 2, ACCESSOR_OPT)),
            _ => None,
        };
        if let Some((de, dk)) = dot {
            let m = self.ws(de);
            self.tok(i, ne, TYPE_MODULE);
            self.tok(k, de, dk);
            if self.c(m) == b'<' {
                self.tok(m, m + 1, TP_BEGIN);
                self.i = m + 1;
                self.push(TYPE_NAME_ARGS);
                return true;
            }
            self.i = de;
            return true;
        }
        if self.c(k) == b'<' {
            self.tok(i, ne, TYPE);
            self.tok(k, k + 1, TP_BEGIN);
            self.i = k + 1;
            self.push(TYPE_NAME_ARGS);
            return true;
        }
        self.take(i, ne, TYPE)
    }

    /// A type object's member: `type-object`'s patterns after its `}`.
    pub(super) fn type_object_member(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        if self.comment(false) {
            return true;
        }
        if self.method_declaration() || self.indexer() || self.mapped() || self.field() {
            return true;
        }
        if c == b':' && self.type_annotation() {
            return true;
        }
        if c == b'.' && self.c(i + 1) == b'.' && self.c(i + 2) == b'.' {
            self.tok(i, i + 3, SPREAD);
            self.i = i + 3;
            self.push(TYPE_SPREAD);
            return true;
        }
        match c {
            b',' => self.take(i, i + 1, COMMA),
            b';' => self.take(i, i + 1, TERMINATOR),
            _ => self.type_(),
        }
    }

    /// `indexer-mapped-type-declaration`: `(?:«wb»([+-])?(readonly)\s*)?\s*(\[)\s*(name)\s+(in)\s+`.
    pub(super) fn mapped(&mut self) -> bool {
        let i = self.i;
        let mut j = i;
        let mut sign = None;
        let mut ro = None;
        {
            let s = if matches!(self.c(i), b'+' | b'-') { i + 1 } else { i };
            if let Some(e) = self.word_b(s, b"readonly")
                && self.wb(i)
            {
                if s > i {
                    sign = Some(i);
                }
                ro = Some((s, e));
                j = e;
            }
        }
        let k = self.ws(j);
        if self.c(k) != b'[' {
            return false;
        }
        let m = self.ws(k + 1);
        let ne = self.name(m);
        if ne == m {
            return false;
        }
        let g = self.ws(ne);
        if g == ne || self.word_b(g, b"in").is_none() || !is_space(self.c(g + 2)) {
            return false;
        }
        if let Some(s) = sign {
            self.tok(s, s + 1, TYPE_MODIFIER_OP);
        }
        if let Some((s, e)) = ro {
            self.tok(s, e, MODIFIER);
        }
        self.tok(k, k + 1, SQUARE);
        self.tok(m, ne, TYPE);
        self.tok(g, g + 2, IN);
        self.i = self.ws(g + 2).min(self.n);
        self.push(MAPPED);
        true
    }

    /// A type paren's parameter: a name that is a function, or one before `:`.
    pub(super) fn type_paren_param(&mut self) -> bool {
        let i = self.i;
        if !self.wb(i) {
            return false;
        }
        let mods: &[&[u8]] = &[b"public", b"private", b"protected", b"readonly"];
        let mut j = i;
        let mut modifier = None;
        for m in mods {
            if self.rest(j).starts_with(m) && self.boundary(j + m.len()) {
                let k = self.ws(j + m.len());
                // the modifier only before a name (else it is the name)
                let r = if self.rest(k).starts_with(b"...") { self.ws(k + 3) } else { k };
                if k > j + m.len() && self.name(r) > r {
                    modifier = Some((j, j + m.len()));
                    j = k;
                    break;
                }
            }
        }
        let mut rest = None;
        if self.rest(j).starts_with(b"...") {
            rest = Some(j);
            j = self.ws(j + 3);
        }
        if j > 0 && matches!(self.b[j - 1], b'=' | b':') {
            return false;
        }
        if !self.wb(j) {
            return false;
        }
        let ne = self.name(j);
        if ne == j {
            return false;
        }
        let this = self.is_name(j, ne, b"this");
        let q = self.ws(ne);
        let opt = if self.c(q) == b'?' { Some(q) } else { None };
        let after = opt.map_or(q, |o| o + 1);
        let fun = self.we(ne) && look::fn_decl(self.b, after);
        let plain = self.c(after) == b':';
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
            self.i = q.min(self.n);
        }
        true
    }
}
