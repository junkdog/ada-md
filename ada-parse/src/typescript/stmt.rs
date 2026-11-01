//! Statements, declarations and class members, and the turn of every frame of code.

use super::*;

/// The words `^\s*(…)\b` that end a declaration at a line's start.
const DECL_WORDS: &[&[u8]] = &[
    b"abstract", b"async", b"class", b"const", b"declare", b"enum", b"export", b"function",
    b"import", b"interface", b"let", b"module", b"namespace", b"return", b"type", b"var",
];

impl Scan<'_, '_, '_> {
    /// `(?:^\s*(?:abstract|async|…|var)\b)` at `i`: only at the line's start.
    pub(super) fn decl_at_bol(&self, i: usize) -> bool {
        if i != 0 {
            return false;
        }
        let j = self.ws(0);
        DECL_WORDS.iter().any(|w| self.word_b(j, w).is_some())
    }

    /// `^\s*$` at `i`.
    pub(super) fn blank_at(&self, i: usize) -> bool {
        i == 0 && self.blank_line()
    }

    /// `(?=;|^\s*$|(?:^\s*(?:decl words)\b))`, `meta.function`'s end and others'.
    pub(super) fn decl_end(&self, i: usize) -> bool {
        self.c(i) == b';' || self.blank_at(i) || self.decl_at_bol(i)
    }

    /// `statements` at the place.
    pub(super) fn statements(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        if self.alpha_or_at(c) && (self.declaration() || self.control()) {
            return true;
        }
        if self.after_op_obj() {
            return true;
        }
        if c == b'{' {
            self.tok(i, i + 1, BLOCK_BRACE);
            self.i = i + 1;
            self.push(BLOCK);
            return true;
        }
        if self.label() {
            return true;
        }
        if self.expression() {
            return true;
        }
        if c == b';' {
            return self.take(i, i + 1, TERMINATOR);
        }
        false
    }

    /// Whether `c` can start a name or a decorator.
    pub(super) fn alpha_or_at(&self, c: u8) -> bool {
        c == b'@' || c.is_ascii_alphabetic() || c == b'_' || c == b'$' || c >= 0x80
    }

    /// `(?:(\bexport)\s+)?(?:(\bdeclare)\s+)?` at `i`: where each ends, and what follows.
    pub(super) fn prefix(&self, i: usize) -> (Option<usize>, Option<usize>, usize) {
        let mut j = i;
        let mut export = None;
        let mut declare = None;
        if self.rest(j).starts_with(b"export") && self.boundary(j) {
            let k = self.ws(j + 6);
            if k > j + 6 && k < self.n {
                export = Some(j + 6);
                j = k;
            }
        }
        if self.rest(j).starts_with(b"declare") && self.boundary(j) {
            let k = self.ws(j + 7);
            if k > j + 7 && k < self.n {
                declare = Some(j + 7);
                j = k;
            }
        }
        (export, declare, j)
    }

    /// The prefix's tokens, `export` and `declare`.
    pub(super) fn prefix_toks(&mut self, i: usize, export: Option<usize>, declare: Option<usize>, j: usize) {
        if let Some(e) = export {
            self.tok(i, e, EXPORT);
        }
        if let Some(d) = declare {
            self.tok(d - 7, d, MODIFIER);
        }
        let _ = j;
    }

    /// `declaration` at the place.
    pub(super) fn declaration(&mut self) -> bool {
        let i = self.i;
        if self.c(i) == b'@' {
            return self.decorator();
        }
        if !self.wb(i) {
            return false;
        }
        let (export, declare, j) = self.prefix(i);
        let w = self.name(j);
        let word = &self.b[j..w];
        let we = self.we(w);
        // var-expr
        if (word == b"var" || word == b"let") && we {
            return self.push_zero(VAR);
        }
        if word == b"const" && we {
            let k = self.ws(w);
            if !(k > w && self.word_b(k, b"enum").is_some()) {
                return self.push_zero(VAR_CONST);
            }
        }
        // function-declaration
        if let Some(r) = self.function_head(j, true) {
            self.prefix_toks(i, export, declare, j);
            return self.function_toks(r, FUNC);
        }
        // class-declaration, interface-declaration
        let (abs, k) = if word == b"abstract" {
            let k = self.ws(w);
            if k > w { (Some(w), k) } else { (None, j) }
        } else {
            (None, j)
        };
        for (kw, kind, frame) in [(&b"class"[..], CLASS_KW, CLASS), (b"interface", INTERFACE_KW, INTERFACE)] {
            if let Some(e) = self.word_b(k, kw)
                && (k == j || self.boundary(k))
                && (is_space(self.c(e)) || (self.c(e) == b'/' && matches!(self.c(e + 1), b'/' | b'*')))
            {
                self.prefix_toks(i, export, declare, j);
                if let Some(a) = abs {
                    self.tok(j, a, MODIFIER);
                }
                self.tok(k, e, kind);
                self.i = e;
                self.push(frame);
                return true;
            }
        }
        // enum-declaration: (const\s+)?enum\s+name
        {
            let (cst, k) = if word == b"const" {
                let k = self.ws(w);
                if k > w { (Some(w), k) } else { (None, j) }
            } else {
                (None, j)
            };
            if let Some(e) = self.word_b(k, b"enum") {
                let m = self.ws(e);
                let ne = self.name(m);
                if m > e && ne > m {
                    self.prefix_toks(i, export, declare, j);
                    if let Some(c) = cst {
                        self.tok(j, c, MODIFIER);
                    }
                    self.tok(k, e, ENUM_KW);
                    self.tok(m, ne, TYPE_ENUM);
                    self.i = ne;
                    self.push(ENUM);
                    return true;
                }
            }
        }
        // namespace-declaration
        if (word == b"namespace" || word == b"module") && self.boundary(w) {
            let k = self.ws(w);
            if k > w && (look::alpha(self.b, k) > 0 || matches!(self.c(k), b'"' | b'\'' | b'`')) {
                self.prefix_toks(i, export, declare, j);
                self.tok(j, w, NAMESPACE_KW);
                self.i = k;
                self.push(NAMESPACE);
                return true;
            }
        }
        // type-alias-declaration: \b(type)\b\s+(name)\s*
        if word == b"type" {
            let k = self.ws(w);
            let ne = self.name(k);
            if k > w && ne > k {
                self.prefix_toks(i, export, declare, j);
                self.tok(j, w, TYPE_KW);
                self.tok(k, ne, TYPE_ALIAS);
                self.i = self.ws(ne).min(self.n);
                self.push(TYPE_DECL);
                return true;
            }
        }
        if word == b"import" && self.boundary(w) {
            // (\s+(type))?
            let mut k = w;
            let mut ty = None;
            let t = self.ws(w);
            if t > w && let Some(te) = self.word_b(t, b"type") {
                ty = Some((t, te));
                k = te;
            }
            // import-equals-declaration
            let m = self.ws(k);
            let ne = self.name(m);
            if m > k && ne > m {
                let q = self.ws(ne);
                if self.c(q) == b'=' {
                    let r = self.ws(q + 1);
                    let req = self.word_b(r, b"require");
                    let p = req.map(|e| self.ws(e));
                    if let (Some(re), Some(p)) = (req, p)
                        && self.c(p) == b'('
                    {
                        self.prefix_toks(i, export, declare, j);
                        self.tok(j, w, IMPORT);
                        if let Some((t, te)) = ty {
                            self.tok(t, te, TYPE_CONTROL);
                        }
                        self.tok(m, ne, ALIAS);
                        self.tok(q, q + 1, ASSIGN);
                        self.tok(r, re, REQUIRE);
                        self.tok(p, p + 1, ROUND);
                        self.i = p + 1;
                        self.push(IMPORT_REQUIRE);
                        return true;
                    }
                    if req.is_none() {
                        self.prefix_toks(i, export, declare, j);
                        self.tok(j, w, IMPORT);
                        if let Some((t, te)) = ty {
                            self.tok(t, te, TYPE_CONTROL);
                        }
                        self.tok(m, ne, ALIAS);
                        self.tok(q, q + 1, ASSIGN);
                        self.i = r.min(self.n);
                        self.push(IMPORT_ALIAS);
                        return true;
                    }
                }
            }
            // import-declaration: (?:\s+(type)(?!\s+from))?(?!\s*[:\(])«we»
            let mut k = w;
            let mut ty = None;
            let t = self.ws(w);
            if t > w && let Some(te) = self.word_b(t, b"type") {
                let f = self.ws(te);
                if !(f > te && self.word_b(f, b"from").is_some()) {
                    ty = Some((t, te));
                    k = te;
                }
            }
            let q = self.ws(k);
            if !matches!(self.c(q), b':' | b'(') && self.we(k) {
                self.prefix_toks(i, export, declare, j);
                self.tok(j, w, IMPORT);
                if let Some((t, te)) = ty {
                    self.tok(t, te, TYPE_CONTROL);
                }
                self.i = k;
                self.push(IMPORT_F);
                // (?<=^import|[^\._$[:alnum:]]import)(?!\s*["'])
                if ty.is_none() && !matches!(self.c(self.ws(k)), b'"' | b'\'') {
                    self.push(IMPORT_CLAUSE);
                }
                return true;
            }
        }
        // export-declaration
        if self.rest(i).starts_with(b"export") && self.wb(i) && self.boundary(i + 6)
            && self.export_declaration() {
                return true;
            }
        // (declare|export) alone
        for kw in [&b"declare"[..], b"export"] {
            if let Some(e) = self.kw(i, kw) {
                return self.take(i, e, MODIFIER);
            }
        }
        false
    }

    /// `export-declaration`'s three, at `export`.
    pub(super) fn export_declaration(&mut self) -> bool {
        let i = self.i;
        let e = i + 6;
        // export as namespace name
        let a = self.ws(e);
        if a > e && let Some(ae) = self.word_b(a, b"as") {
            let n1 = self.ws(ae);
            if n1 > ae && let Some(ne) = self.word_b(n1, b"namespace") {
                let m = self.ws(ne);
                let me = self.name(m);
                if m > ne && me > m {
                    self.tok(i, e, EXPORT);
                    self.tok(a, ae, AS);
                    self.tok(n1, ne, NAMESPACE_KW);
                    return self.take(m, me, TYPE_MODULE);
                }
            }
        }
        // export(\s+type)?((\s*(=))|(\s+(default)(?=\s+)))
        let mut k = e;
        let mut ty = None;
        let t = self.ws(e);
        if t > e && let Some(te) = self.word_b(t, b"type") {
            ty = Some((t, te));
            k = te;
        }
        let q = self.ws(k);
        let eq = self.c(q) == b'=';
        let def = if q > k { self.word_b(q, b"default").filter(|&d| is_space(self.c(d))) } else { None };
        if eq || def.is_some() {
            self.tok(i, e, EXPORT);
            if let Some((t, te)) = ty {
                self.tok(t, te, TYPE_CONTROL);
            }
            if eq {
                self.tok(q, q + 1, ASSIGN);
                self.i = q + 1;
            } else {
                let d = def.unwrap_or(q);
                self.tok(q, d, DEFAULT);
                self.i = d;
            }
            self.push(EXPORT_DEFAULT);
            return true;
        }
        // export(\s+type)?\b(?!(\$)|(\s*:))((?=\s*[\{*])|((?=\s*name(\s|,))(?!\s*decl words\b)))
        for (k, ty) in [(k, ty), (e, None)] {
            if self.c(k) == b'$' || self.c(self.ws(k)) == b':' {
                continue;
            }
            let q = self.ws(k);
            let ok = matches!(self.c(q), b'{' | b'*') || {
                let ne = self.name(q);
                ne > q && (is_space(self.c(ne)) || self.c(ne) == b',') && !DECL_WORDS.iter().any(|w| self.word_b(q, w).is_some())
            };
            if ok {
                self.tok(i, e, EXPORT);
                if let Some((t, te)) = ty {
                    self.tok(t, te, TYPE_CONTROL);
                }
                self.i = k;
                self.push(EXPORT_F);
                return true;
            }
        }
        false
    }

    /// `decorator`: `«wb»\@`.
    pub(super) fn decorator(&mut self) -> bool {
        let i = self.i;
        if self.c(i) != b'@' || !self.wb(i) {
            return false;
        }
        self.tok(i, i + 1, DECORATOR);
        self.i = i + 1;
        self.push(DECORATOR_F);
        true
    }

    /// A function's head at `j`: `«wb»(async\s+)?(function\b)(\s*(\*))?((\s+|(?<=\*))name)?\s*`,
    /// `export` and `declare` before it in a declaration. The parts' places.
    pub(super) fn function_head(&self, j: usize, _decl: bool) -> Option<[usize; 7]> {
        let mut k = j;
        let mut asy = usize::MAX;
        if self.rest(k).starts_with(b"async") && self.boundary(k) {
            let m = self.ws(k + 5);
            if m > k + 5 {
                asy = k;
                k = m;
            }
        }
        if !self.rest(k).starts_with(b"function") || !self.boundary(k + 8) {
            return None;
        }
        if asy == usize::MAX && !self.wb(k) {
            return None;
        }
        let fe = k + 8;
        let mut star = usize::MAX;
        let mut m = fe;
        let s = self.ws(fe);
        if self.c(s) == b'*' {
            star = s;
            m = s + 1;
        }
        let mut name = (usize::MAX, usize::MAX);
        let g = self.ws(m);
        if g > m || star != usize::MAX {
            let ne = self.name(g);
            if ne > g {
                name = (g, ne);
                m = ne;
            }
        }
        let end = self.ws(m).min(self.n);
        Some([asy, k, star, name.0, name.1, end, fe])
    }

    /// A function's tokens from [`Scan::function_head`]'s places, and its frame pushed.
    pub(super) fn function_toks(&mut self, r: [usize; 7], frame: u8) -> bool {
        let [asy, k, star, ns, ne, end, fe] = r;
        if asy != usize::MAX {
            self.tok(asy, asy + 5, ASYNC);
        }
        self.tok(k, fe, FUNCTION_KW);
        if star != usize::MAX {
            self.tok(star, star + 1, ASTERISK);
        }
        if ns != usize::MAX {
            self.tok(ns, ne, ENTITY_FN);
        }
        self.i = end;
        self.push(frame);
        true
    }

    /// `control-statement` at the place.
    pub(super) fn control(&mut self) -> bool {
        let i = self.i;
        if !self.wb(i) {
            return false;
        }
        let w = self.name(i);
        if w == i || !self.we(w) {
            // `(break|continue|goto)\s+name` still needs the word whole
            if w == i {
                return false;
            }
        }
        let word = &self.b[i..w];
        let we = self.we(w);
        match word {
            b"switch" => {
                let k = self.ws(w);
                if self.c(k) == b'(' {
                    return self.push_zero(SWITCH_F);
                }
            }
            b"for" => {
                // for(?=((\s+|(\s*/*…*/\s*))await)?\s*(/*…*/\s*)?(\())
                let mut k = self.ws(w);
                let c2 = look::comments(self.b, k);
                if let Some(ae) = self.word_b(c2, b"await")
                    && c2 > w {
                        k = ae;
                    }
                let k = look::comments(self.b, self.ws(k));
                if self.c(k) == b'(' {
                    self.tok(i, w, LOOP);
                    self.i = w;
                    self.push(FOR);
                    return true;
                }
            }
            _ => {}
        }
        if !we {
            return false;
        }
        match word {
            b"catch" | b"finally" | b"throw" | b"try" => return self.take(i, w, TRY),
            b"break" | b"continue" | b"goto" => {
                let k = self.ws(w);
                let ne = self.name(k);
                if k > w && ne > k && self.we(ne) {
                    self.tok(i, w, LOOP);
                    return self.take(k, ne, LABEL);
                }
                return self.take(i, w, LOOP);
            }
            b"do" | b"while" => return self.take(i, w, LOOP),
            b"return" => {
                self.tok(i, w, FLOW);
                self.i = w;
                self.push(RETURN);
                return true;
            }
            b"case" | b"default" | b"switch" => return self.take(i, w, SWITCH),
            b"if" => {
                // (?=\bif\s*(«B(»)\s*(?!\{))
                let k = self.ws(w);
                if let Some(e) = look::bal(self.b, k, b'(', b')', 3)
                    && self.c(self.ws(e)) != b'{'
                {
                    return self.push_zero(IF);
                }
                return self.take(i, w, CONDITIONAL);
            }
            b"else" => return self.take(i, w, CONDITIONAL),
            b"with" => return self.take(i, w, WITH),
            b"package" => return self.take(i, w, CONTROL),
            b"debugger" => return self.take(i, w, DEBUGGER),
            _ => {}
        }
        false
    }

    /// `after-operator-block-as-object-literal`: `{` after an operator or `return` and such.
    pub(super) fn after_op_obj(&mut self) -> bool {
        let i = self.i;
        let k = self.ws(i);
        if self.c(k) != b'{' || !self.after_operator(i) {
            return false;
        }
        self.tok(k, k + 1, BLOCK_BRACE);
        self.i = k + 1;
        self.push(OBJ);
        true
    }

    /// `(?<!\+\+|--)(?<=[:=(,\[?+!>]|^await|[^\._$[:alnum:]]await|^return|…|&&|\|\||\*)` at `i`.
    pub(super) fn after_operator(&self, i: usize) -> bool {
        if i == 0 {
            return false;
        }
        let b = self.b;
        if i >= 2 && (&b[i - 2..i] == b"++" || &b[i - 2..i] == b"--") {
            return false;
        }
        if matches!(b[i - 1], b':' | b'=' | b'(' | b',' | b'[' | b'?' | b'+' | b'!' | b'>' | b'*') {
            return true;
        }
        if i >= 2 && (&b[i - 2..i] == b"&&" || &b[i - 2..i] == b"||") {
            return true;
        }
        self.word_ends_at(i, &[b"await", b"return", b"yield", b"throw", b"in", b"of", b"typeof"])
    }

    /// `(?<=^w|[^\._$[:alnum:]]w)` at `i` for one of `ws`.
    pub(super) fn word_ends_at(&self, i: usize, ws: &[&[u8]]) -> bool {
        ws.iter().any(|w| {
            i >= w.len() && &self.b[i - w.len()..i] == *w && {
                let s = i - w.len();
                s == 0 || (self.b[s - 1] != b'.' && !look::alnum_before(self.b, s))
            }
        })
    }

    /// `label`: `name\s*(:)`, a `{` after it a labelled block.
    pub(super) fn label(&mut self) -> bool {
        let i = self.i;
        let ne = self.name(i);
        if ne == i {
            return false;
        }
        let k = self.ws(ne);
        if self.c(k) != b':' {
            return false;
        }
        self.tok(i, ne, LABEL);
        self.tok(k, k + 1, LABEL_SEP);
        self.i = k + 1;
        if self.c(self.ws(k + 1)) == b'{' {
            self.push(LABEL_BLOCK);
        }
        true
    }

    /// `class-or-interface-body` at the place.
    pub(super) fn class_body(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        if self.comment(false) || self.decorator() {
            return true;
        }
        // (?<=:)\s*: after a `:` a frame above took (a ternary's), an expression
        if i > 0 && self.b[i - 1] == b':' {
            self.i = self.ws(i).min(self.n);
            if self.i == i {
                return self.push_zero(CLASS_COLON);
            }
            self.push(CLASS_COLON);
            return true;
        }
        if self.method_declaration() || self.indexer() || self.field() {
            return true;
        }
        if self.string() {
            return true;
        }
        if c == b':' && self.type_annotation() {
            return true;
        }
        if c == b'=' && self.initializer() {
            return true;
        }
        if self.alpha_or_at(c) && self.wb(i) {
            let w = self.name(i);
            if self.we(w) {
                let word = &self.b[i..w];
                if matches!(word, b"abstract" | b"declare" | b"override" | b"public" | b"protected" | b"private" | b"readonly" | b"static") {
                    return self.take(i, w, MODIFIER);
                }
                if matches!(word, b"get" | b"set") {
                    return self.take(i, w, PROPERTY_KW);
                }
                if word == b"async" {
                    return self.take(i, w, ASYNC);
                }
            }
        }
        if self.after_op_obj() {
            return true;
        }
        if c == b'{' {
            self.tok(i, i + 1, BLOCK_BRACE);
            self.i = i + 1;
            self.push(BLOCK);
            return true;
        }
        if self.expression() {
            return true;
        }
        match c {
            b',' => self.take(i, i + 1, COMMA),
            b';' => self.take(i, i + 1, TERMINATOR),
            _ => false,
        }
    }

    /// `(?:\b(override)\s+)?(?:\b(public|private|protected)\s+)?(?:\b(abstract)\s+)?(?:\b(async)\s+)?`
    /// at `i`: each one's place, and where the rest starts.
    pub(super) fn modifiers(&self, i: usize, with_access: bool) -> ([(usize, usize, u16); 4], usize) {
        let mut out = [(0, 0, PLAIN); 4];
        let mut j = i;
        let steps: [(&[&[u8]], u16); 4] = [
            (&[b"override"], MODIFIER),
            (&[b"public", b"private", b"protected"], MODIFIER),
            (&[b"abstract"], MODIFIER),
            (&[b"async"], ASYNC),
        ];
        for (n, (ws, kind)) in steps.iter().enumerate() {
            if !with_access && n < 3 {
                continue;
            }
            for w in ws.iter() {
                if self.rest(j).starts_with(w) && self.boundary(j) {
                    let e = j + w.len();
                    let k = self.ws(e);
                    if k > e && k < self.n + 1 {
                        out[n] = (j, e, *kind);
                        j = k;
                        break;
                    }
                }
            }
        }
        (out, j)
    }

    /// A method's or member's key at `i`: `«NUM»|(«name»)|«sq»|«dq»|«bq»|(\[([^\[\]]|\[[^\[\]]*\])+\])`: its end.
    pub(super) fn member_key(&self, i: usize) -> Option<usize> {
        let c = self.c(i);
        match c {
            b'\'' | b'"' | b'`' => look::quoted(self.b, i, c),
            b'[' => {
                // two levels, not empty
                let mut j = i + 1;
                loop {
                    match self.c(j) {
                        0 => return None,
                        b']' => return (j > i + 1).then_some(j + 1),
                        b'[' => {
                            let mut k = j + 1;
                            while !matches!(self.c(k), b']' | b'[' | 0) {
                                k += 1;
                            }
                            if self.c(k) != b']' {
                                return None;
                            }
                            j = k + 1;
                        }
                        _ => j += 1,
                    }
                }
            }
            b'0'..=b'9' | b'.' => self.number_end(i),
            _ => {
                let e = self.name(i);
                (e > i).then_some(e)
            }
        }
    }

    /// `method-declaration` at the place.
    pub(super) fn method_declaration(&mut self) -> bool {
        let i = self.i;
        if !self.wb(i) {
            return false;
        }
        let (mods, j) = self.modifiers(i, true);
        let toks = |me: &mut Self| {
            for (s, e, k) in mods {
                if e > s {
                    me.tok(s, e, k);
                }
            }
        };
        // constructor
        let j0 = self.ws(j);
        if let Some(e) = self.word_b(j0, b"constructor")
            && self.boundary(j0)
            && self.c(e) != b':'
            && self.we(e)
        {
            toks(self);
            self.tok(j0, e, STORAGE);
            self.i = e;
            self.push(METHOD);
            return true;
        }
        // new or * before (
        {
            let mut k = j;
            let mut new = None;
            let mut star = None;
            let n0 = self.ws(j);
            if let Some(e) = self.word_b(n0, b"new")
                && self.boundary(n0)
                && self.c(e) != b':'
                && self.we(e)
            {
                new = Some((n0, e));
                k = e;
            } else if self.c(j) == b'*' {
                star = Some(j);
                k = self.ws(j + 1);
            }
            let q = self.ws(k);
            let ok = match self.c(q) {
                b'(' => true,
                b'<' => look::tp(self.b, q).is_some_and(|e| self.c(e) == b'('),
                _ => false,
            };
            if ok {
                toks(self);
                if let Some((s, e)) = new {
                    self.tok(s, e, NEW);
                }
                if let Some(s) = star {
                    self.tok(s, s + 1, ASTERISK);
                }
                self.i = k;
                self.push_zero(METHOD);
                return true;
            }
        }
        // get/set, *, then a key and ( or <
        let mut k = j;
        let mut acc = None;
        for w in [&b"get"[..], b"set"] {
            if self.rest(k).starts_with(w) && self.boundary(k) && self.boundary(k + 3) {
                let m = self.ws(k + 3);
                if m > k + 3 {
                    acc = Some((k, k + 3));
                    k = m;
                    break;
                }
            }
        }
        let mut star = None;
        if self.c(k) == b'*' {
            star = Some(k);
            k = self.ws(k + 1);
        }
        if self.method_key_ahead(k) {
            toks(self);
            if let Some((s, e)) = acc {
                self.tok(s, e, PROPERTY_KW);
            }
            if let Some(s) = star {
                self.tok(s, s + 1, ASTERISK);
            }
            self.i = k;
            if k == i {
                self.push_zero(METHOD);
            } else {
                self.push(METHOD);
            }
            return true;
        }
        false
    }

    /// `(?=\s*(KEY)\s*(\??))\s*(«TP»)?[\(]`, a method's key ahead at `k`.
    pub(super) fn method_key_ahead(&self, k: usize) -> bool {
        let q = self.ws(k);
        let Some(e) = self.member_key(q) else { return false };
        let mut r = self.ws(e);
        if self.c(r) == b'?' {
            r = self.ws(r + 1);
        }
        match self.c(r) {
            b'(' => true,
            b'<' => look::tp(self.b, r).is_some_and(|t| self.c(t) == b'('),
            _ => false,
        }
    }

    /// `indexer-declaration`: `(?:«wb»(readonly)\s*)?\s*(\[)\s*(name)\s*(?=:)`.
    pub(super) fn indexer(&mut self) -> bool {
        let i = self.i;
        let mut j = i;
        let mut ro = None;
        if let Some(e) = self.kw(i, b"readonly") {
            ro = Some(e);
            j = e;
        }
        let k = self.ws(j);
        if self.c(k) != b'[' {
            return false;
        }
        let m = self.ws(k + 1);
        let ne = self.name(m);
        if ne == m || self.c(self.ws(ne)) != b':' {
            return false;
        }
        if let Some(e) = ro {
            self.tok(i, e, MODIFIER);
        }
        self.tok(k, k + 1, SQUARE);
        self.tok(m, ne, PARAM);
        self.i = ne;
        self.push(INDEXER);
        true
    }

    /// `field-declaration`: `(?<!\()(?:«wb»(readonly)\s+)?(?=\s*KEY\s*(?:(?:(\?)|(\!))\s*)?(=|:|;|,|\}|$))`.
    pub(super) fn field(&mut self) -> bool {
        let i = self.i;
        if i > 0 && self.b[i - 1] == b'(' {
            return false;
        }
        let mut j = i;
        let mut ro = None;
        if let Some(e) = self.kw(i, b"readonly") {
            let k = self.ws(e);
            if k > e {
                ro = Some(e);
                j = k;
            }
        }
        if !self.field_ahead(j, true) {
            return false;
        }
        if let Some(e) = ro {
            self.tok(i, e, MODIFIER);
        }
        self.i = j;
        if j == i {
            self.push_zero(FIELD);
        } else {
            self.push(FIELD);
        }
        true
    }

    /// `\s*(KEY)\s*(?:(?:(\?)|(\!))\s*)?(=|:|;|,|\}|$)` at `j` (the key may be `#name`); `}` only
    /// if `brace`.
    pub(super) fn field_ahead(&self, j: usize, brace: bool) -> bool {
        let q = self.ws(j);
        let q2 = if self.c(q) == b'#' { q + 1 } else { q };
        let e = if q2 > q {
            let e = self.name(q2);
            if e == q2 {
                return false;
            }
            e
        } else {
            match self.member_key(q) {
                Some(e) => e,
                None => return false,
            }
        };
        let mut r = self.ws(e);
        if matches!(self.c(r), b'?' | b'!') {
            r = self.ws(r + 1);
        }
        matches!(self.c(r), b'=' | b':' | b';' | b',') || (brace && self.c(r) == b'}') || r >= self.n
    }

    /// A field's turn.
    pub(super) fn field_step(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        // (?=\}|;|,|$|(^(?!\s*KEY…(=|:|;|,|$))))|(?<=\})
        if (matches!(c, b'}' | b';' | b',') || i >= self.n || (i == 0 && !self.field_ahead(0, false)) || (i > 0 && self.b[i - 1] == b'}'))
            && self.pop_zero() {
                return true;
            }
        if c == b'=' && self.initializer() {
            return true;
        }
        if c == b':' && self.type_annotation() {
            return true;
        }
        if self.string() || self.array_literal() || self.numeric() || self.comment(false) {
            return true;
        }
        // (\#?«name»)(?:(\?)|(\!))?(?=…a function)
        let s = if c == b'#' { i + 1 } else { i };
        let ne = self.name(s);
        if ne > s {
            let mut e = ne;
            let mut mark = None;
            if matches!(self.c(ne), b'?' | b'!') {
                mark = Some(ne);
                e = ne + 1;
            }
            if look::fn_decl(self.b, e, self.tsx) {
                self.tok(i, ne, ENTITY_FN);
                if let Some(m) = mark {
                    self.tok(m, m + 1, if self.b[m] == b'?' { OPTIONAL } else { DEFINITE });
                }
                self.i = e;
                return true;
            }
            return self.take(i, ne, OBJ_PROPERTY);
        }
        match c {
            b'?' => self.take(i, i + 1, OPTIONAL),
            b'!' => self.take(i, i + 1, DEFINITE),
            _ => false,
        }
    }

    /// `variable-initializer` at `=`.
    pub(super) fn initializer(&mut self) -> bool {
        let i = self.i;
        if self.c(i) != b'=' || self.c(i + 1) == b'=' {
            return false;
        }
        if i > 0 && matches!(self.b[i - 1], b'=' | b'!') {
            return false;
        }
        self.tok(i, i + 1, ASSIGN);
        self.i = i + 1;
        // (?=\s*\S)(?!\s*.*=>\s*$): something after it on the line, and not an arrow at its end
        let k = self.ws(i + 1);
        let tail_arrow = {
            let mut e = self.n;
            while e > i + 1 && is_space(self.b[e - 1]) {
                e -= 1;
            }
            e >= i + 3 && &self.b[e - 2..e] == b"=>"
        };
        if k < self.n && !tail_arrow {
            self.push(INIT);
        } else {
            self.push(INIT_EOL);
        }
        true
    }

    /// `(?=$|^|[,);}\]]|((?<![_$[:alnum:]])(?:(?<=\.\.\.)|(?<!\.))(of|in)\s+))` at `i`.
    pub(super) fn init_end(&self, i: usize) -> bool {
        i >= self.n || i == 0 || matches!(self.c(i), b',' | b')' | b';' | b'}' | b']') || self.of_in(i)
    }

    /// `«wb»(of|in)\s+` at `i`.
    pub(super) fn of_in(&self, i: usize) -> bool {
        for w in [&b"of"[..], b"in"] {
            if self.rest(i).starts_with(w) && self.wb(i) && is_space(self.c(i + 2)) {
                return true;
            }
        }
        false
    }

    /// The turn of a frame of code.
    pub(super) fn code_step(&mut self, f: u8) -> bool {
        let i = self.i;
        let c = self.c(i);
        match f {
            MAIN => {
                if i == 0 && self.c(0) == b'#' && self.c(1) == b'!' && self.exact_first_line() {
                    self.tok(0, 2, SHEBANG_MARK);
                    return self.take(2, self.n, SHEBANG);
                }
                if i == 0 && self.directive() {
                    return true;
                }
                self.statements() || self.comment(false)
            }
            BLOCK | CASE_BLOCK | LABEL_BLOCK => {
                if c == b'}' {
                    self.tok(i, i + 1, BLOCK_BRACE);
                    self.i = i + 1;
                    self.pop();
                    if f == LABEL_BLOCK {
                        return true;
                    }
                    return true;
                }
                if f == LABEL_BLOCK {
                    // (?<=\}) pops; its own `{` is a block
                    if c == b'{' {
                        self.tok(i, i + 1, BLOCK_BRACE);
                        self.i = i + 1;
                        self.set(BLOCK);
                        return true;
                    }
                    return false;
                }
                self.statements() || self.comment(false)
            }
            CALL_ARGS => {
                if c == b')' {
                    self.tok(i, i + 1, ROUND);
                    self.i = i + 1;
                    // the call goes on if another follows (`function-call`'s pop's lookahead)
                    if look::call_target(self.b, i + 1).is_some() || look::call_after(self.b, i + 1) {
                        self.set(CALL);
                        return true;
                    }
                    return self.pop();
                }
                self.expression()
            }
            PAREN | IF_COND | SWITCH_EXPR => {
                if c == b')' {
                    self.tok(i, i + 1, ROUND);
                    self.i = i + 1;
                    return self.pop();
                }
                self.expression()
            }
            ARRAY => {
                if c == b']' {
                    self.tok(i, i + 1, SQUARE);
                    self.i = i + 1;
                    return self.pop();
                }
                self.expression() || (c == b',' && self.take(i, i + 1, COMMA))
            }
            OBJ => {
                if c == b'}' {
                    self.tok(i, i + 1, BLOCK_BRACE);
                    self.i = i + 1;
                    return self.pop();
                }
                self.object_member()
            }
            OBJ_VAL | OBJ_SPREAD => {
                if matches!(c, b',' | b'}') && self.pop_zero() {
                    return true;
                }
                if f == OBJ_VAL && self.obj_value_arrow() {
                    return true;
                }
                if f == OBJ_VAL && self.maybe_arrow_return() {
                    return true;
                }
                self.expression()
            }
            OBJ_ASSIGN => {
                if (matches!(c, b',' | b'}') || i >= self.n || (c == b'/' && matches!(self.c(i + 1), b'/' | b'*'))) && self.pop_zero() {
                    return true;
                }
                self.expression()
            }
            OBJ_KEY_SQ => {
                if (c == b':' || (i > 0 && self.b[i - 1] == b']' && matches!(self.c(self.ws(i)), b'(' | b'<')))
                    && self.pop_zero() {
                        return true;
                    }
                self.comment(false) || self.array_literal()
            }
            OBJ_KEY_STR => {
                if (c == b':' || (i > 0 && matches!(self.b[i - 1], b'\'' | b'"' | b'`') && self.key_str_end(i)))
                    && self.pop_zero() {
                        return true;
                    }
                self.comment(false) || self.string()
            }
            OBJ_KEY_NUM => {
                let k = self.ws(i);
                if (c == b':' || matches!(self.c(k), b'(' | b'<' | b',' | b'}') || self.as_ahead(i))
                    && self.pop_zero() {
                        return true;
                    }
                self.comment(false) || self.numeric_key()
            }
            OBJ_KEY_METHOD | OBJ_METHOD | METHOD | FUNC | FUNC_EXPR => self.function_frame(f),
            METHOD_NAME => {
                if matches!(c, b'(' | b'<') && self.pop_zero() {
                    return true;
                }
                if self.string() || self.array_literal() || self.numeric() {
                    return true;
                }
                let ne = self.name(i);
                if ne > i {
                    return self.take(i, ne, ENTITY_FN);
                }
                c == b'?' && self.take(i, i + 1, OPTIONAL)
            }
            TERNARY_F => {
                let k = self.ws(i);
                if self.c(k) == b':' {
                    self.tok(k, k + 1, TERNARY);
                    self.i = k + 1;
                    return self.pop();
                }
                self.expression()
            }
            NEW_F => {
                if ((i > 0 && self.b[i - 1] == b')') || self.new_end(i))
                    && self.pop_zero() {
                        return true;
                    }
                self.expression()
            }
            RETURN => {
                if (matches!(c, b';' | b'}') || i >= self.n || self.blank_at(i) || self.decl_at_bol(i)) && self.pop_zero() {
                    return true;
                }
                self.expression()
            }
            VAR | VAR_CONST => self.var_step(f),
            VAR_MORE | VAR_CONST_MORE => {
                // (?<!,)(((?==|;|}|of/in|^\s*$))|((?<=\S)(?=\s*$)))
                let not_comma = i == 0 || self.b[i - 1] != b',';
                if not_comma
                    && (matches!(c, b'=' | b';' | b'}') || self.of_in(i) || self.blank_at(i)
                        || (i > 0 && !is_space(self.b[i - 1]) && self.ws(i) >= self.n))
                    && self.pop_zero()
                {
                    return true;
                }
                if self.comment(true) {
                    return true;
                }
                if self.destructuring(f == VAR_CONST_MORE) || self.var_single(f == VAR_CONST_MORE) {
                    return true;
                }
                c == b',' && self.take(i, i + 1, COMMA)
            }
            VAR_NAME => {
                // (?=$|^|[;,=}]|«wb»(of|in)\s+)
                if (i >= self.n || i == 0 || matches!(c, b';' | b',' | b'=' | b'}') || self.of_in(i)) && self.pop_zero() {
                    return true;
                }
                (c == b':' && self.type_annotation()) || self.string() || self.comment(false)
            }
            INIT => {
                if self.init_end(i) && self.pop_zero() {
                    return true;
                }
                self.expression()
            }
            INIT_EOL => {
                // (?=[,);}\]]|of/in)|(?=^\s*$)|(?<=\S)(?<!=)(?=\s*$)
                if (matches!(c, b',' | b')' | b';' | b'}' | b']') || self.of_in(i) || self.blank_at(i)
                    || (i > 0 && !is_space(self.b[i - 1]) && self.b[i - 1] != b'=' && self.ws(i) >= self.n))
                    && self.pop_zero()
                {
                    return true;
                }
                self.expression()
            }
            DESTRUCT | DESTRUCT_CONST | DESTRUCT_ARR | DESTRUCT_ARR_CONST => {
                // (?=$|^|[;,=}]|of/in\s+)
                if (i >= self.n || i == 0 || matches!(c, b';' | b',' | b'=' | b'}') || self.of_in(i)) && self.pop_zero() {
                    return true;
                }
                // the object's context takes only `{`, the array's only `[`
                let arr = matches!(f, DESTRUCT_ARR | DESTRUCT_ARR_CONST);
                let j = if self.rest(i).starts_with(b"...") { self.ws(i + 3) } else { i };
                (self.c(j) == if arr { b'[' } else { b'{' } && self.binding_pattern(matches!(f, DESTRUCT_CONST | DESTRUCT_ARR_CONST), false))
                    || (c == b':' && self.type_annotation())
                    || self.comment(false)
            }
            BIND_OBJ_F | BIND_OBJ_CONST | PBIND_OBJ => {
                if c == b'}' {
                    self.tok(i, i + 1, BIND_OBJ);
                    self.i = i + 1;
                    return self.pop();
                }
                self.binding_element_obj(f)
            }
            BIND_ARR_F | BIND_ARR_CONST | PBIND_ARR => {
                if c == b']' {
                    self.tok(i, i + 1, BIND_ARR);
                    self.i = i + 1;
                    return self.pop();
                }
                self.binding_element(f) || (c == b',' && self.take(i, i + 1, COMMA))
            }
            BIND_ELEM | BIND_ELEM_CONST | PBIND_ELEM => {
                if matches!(c, b',' | b'}') && self.pop_zero() {
                    return true;
                }
                if self.binding_key() {
                    return true;
                }
                let g = match f {
                    BIND_ELEM => BIND_ARR_F,
                    BIND_ELEM_CONST => BIND_ARR_CONST,
                    _ => PBIND_ARR,
                };
                self.binding_element(g)
            }
            BIND_KEY => {
                if c == b':' {
                    self.tok(i, i + 1, DESTRUCTURING);
                    self.i = i + 1;
                    return self.pop();
                }
                if self.string() || self.array_literal() || self.numeric() {
                    return true;
                }
                let ne = self.name(i);
                ne > i && self.take(i, ne, OBJ_PROPERTY)
            }
            SUBST => {
                if c == b'}' {
                    self.tok(i, i + 1, SUBST_END);
                    self.i = i + 1;
                    return self.pop();
                }
                self.expression()
            }
            DECORATOR_F => {
                if is_space(c) && self.pop_zero() {
                    return true;
                }
                self.expression()
            }
            TYPEOF_F => {
                // (?=[,);}\]=>:&|{\?]|$|;|^\s*$|decl at bol)
                if (matches!(c, b',' | b')' | b';' | b'}' | b']' | b'=' | b'>' | b':' | b'&' | b'|' | b'{' | b'?')
                    || i >= self.n || self.blank_at(i) || self.decl_at_bol(i))
                    && self.pop_zero()
                {
                    return true;
                }
                self.expression()
            }
            CALL_TARGET => {
                if self.call_target_end(i) && self.pop_zero() {
                    return true;
                }
                self.call_target_name()
            }
            CALL => self.call_step(),
            CALL_EOL => {
                // (?<=\>)(?!target(<\s*[\{\[\(]\s*$))
                if i > 0 && self.b[i - 1] == b'>' && self.call_eol_target(i).is_none() && self.pop_zero() {
                    return true;
                }
                if self.call_eol_target(i).is_some() {
                    return self.push_zero(CALL_EOL_TARGET);
                }
                if self.comment(false) {
                    return true;
                }
                match c {
                    b'?' if self.c(i + 1) == b'.' => self.take(i, i + 2, ACCESSOR_OPT),
                    b'!' => self.take(i, i + 1, DEFINITE),
                    b'<' => self.type_arguments(),
                    _ => false,
                }
            }
            CALL_EOL_TARGET => {
                if self.type_args_open_eol(i) && self.pop_zero() {
                    return true;
                }
                self.call_target_name()
            }
            TPL_CALL => {
                if c == b'`' && self.pop_zero() {
                    return true;
                }
                c == b'<' && self.type_arguments()
            }
            TPL_CALL_TARGET => {
                // (?=(«TA»\s*)?`)
                let at_args = c == b'<' && look::type_args(self.b, i).is_some_and(|e| self.c(self.ws(e)) == b'`');
                if (c == b'`' || at_args) && self.pop_zero() {
                    return true;
                }
                if self.literal() || self.support_objects() || self.object_identifiers() || self.accessor() {
                    return true;
                }
                let ne = self.name(i);
                ne > i && self.take(i, ne, TPL_TAG)
            }
            ARROW_HEAD => {
                // (?==>|\{|(^\s*(export|function|…)\s+))
                if ((c == b'=' && self.c(i + 1) == b'>') || c == b'{' || self.arrow_head_bol(i)) && self.pop_zero() {
                    return true;
                }
                if self.comment(false) {
                    return true;
                }
                if c == b'<' && self.type_parameters() {
                    return true;
                }
                if c == b'(' {
                    self.tok(i, i + 1, PARAMS_BEGIN);
                    self.i = i + 1;
                    self.push(PARAMS);
                    return true;
                }
                self.arrow_return_type() || self.maybe_arrow_return()
            }
            ARROW_F => {
                // ((?<=\}|\S)(?<!=>)|((?!\{)(?=\S)))(?!\/[\/\*])
                let prev_ok = i > 0 && (self.b[i - 1] == b'}' || !is_space(self.b[i - 1])) && !(i >= 2 && &self.b[i - 2..i] == b"=>");
                let next_ok = i < self.n && c != b'{' && !is_space(c);
                let not_comment = !(c == b'/' && matches!(self.c(i + 1), b'/' | b'*'));
                if (prev_ok || next_ok) && not_comment && self.pop_zero() {
                    return true;
                }
                if self.comment(true) {
                    return true;
                }
                if c == b'{' {
                    self.tok(i, i + 1, BLOCK_BRACE);
                    self.i = i + 1;
                    self.set(BLOCK); // the arrow ends with its block
                    return true;
                }
                self.expression()
            }
            MAYBE_ARROW => {
                if i > 0 && self.b[i - 1] == b')' && self.pop_zero() {
                    return true;
                }
                if c == b'<' && self.type_parameters() {
                    return true;
                }
                if c == b'(' {
                    self.tok(i, i + 1, ROUND);
                    self.i = i + 1;
                    self.push(ARROW_PARENS);
                    return true;
                }
                false
            }
            ARROW_PARENS | OBJ_ARROW_PARENS => {
                if c == b')' {
                    self.tok(i, i + 1, ROUND);
                    self.i = i + 1;
                    return self.pop();
                }
                self.inside_arrow_parens()
            }
            OBJ_ARROW_TP => {
                if i > 0 && self.b[i - 1] == b'>' && self.pop_zero() {
                    return true;
                }
                c == b'<' && self.type_parameters()
            }
            PARAMS => {
                if c == b')' {
                    self.tok(i, i + 1, PARAMS_END);
                    self.i = i + 1;
                    return self.pop();
                }
                self.parameters_body()
            }
            CLASS | INTERFACE => {
                if i > 0 && self.b[i - 1] == b'}' && self.pop_zero() {
                    return true;
                }
                if self.comment(false) || self.heritage() {
                    return true;
                }
                let ne = self.name(i);
                if ne > i {
                    return self.take(i, ne, if f == CLASS { TYPE_CLASS } else { TYPE_INTERFACE });
                }
                if c == b'<' && self.type_parameters() {
                    return true;
                }
                if c == b'{' {
                    self.tok(i, i + 1, BLOCK_BRACE);
                    self.i = i + 1;
                    self.set(CLASS_BODY); // the class ends with its body
                    return true;
                }
                false
            }
            CLASS_BODY => {
                if c == b'}' {
                    self.tok(i, i + 1, BLOCK_BRACE);
                    self.i = i + 1;
                    return self.pop();
                }
                self.class_body()
            }
            HERITAGE => {
                if c == b'{' && self.pop_zero() {
                    return true;
                }
                self.heritage_body()
            }
            CLASS_COLON => {
                // (?=\s|[;),}\]:\-\+]|;|^\s*$|(?:^\s*(?:decl words)\b))
                if (is_space(c) || matches!(c, b';' | b')' | b',' | b'}' | b']' | b':' | b'-' | b'+') || self.blank_at(i) || self.decl_at_bol(i))
                    && self.pop_zero()
                {
                    return true;
                }
                self.expression()
            }
            FIELD => self.field_step(),
            ENUM => {
                if i > 0 && self.b[i - 1] == b'}' && self.pop_zero() {
                    return true;
                }
                if self.comment(false) {
                    return true;
                }
                if c == b'{' {
                    self.tok(i, i + 1, BLOCK_BRACE);
                    self.i = i + 1;
                    self.set(ENUM_BODY);
                    return true;
                }
                false
            }
            ENUM_BODY => {
                if c == b'}' {
                    self.tok(i, i + 1, BLOCK_BRACE);
                    self.i = i + 1;
                    return self.pop();
                }
                if self.comment(false) {
                    return true;
                }
                let ne = self.name(i);
                if ne > i {
                    self.tok(i, ne, ENUM_MEMBER);
                    self.i = ne;
                    self.push(ENUM_MEMBER_KEY);
                    return true;
                }
                if matches!(c, b'\'' | b'"' | b'`' | b'[') && self.member_key(i).is_some() {
                    self.push_zero(ENUM_MEMBER_F);
                    return true;
                }
                c == b',' && self.take(i, i + 1, COMMA)
            }
            ENUM_MEMBER_KEY | ENUM_MEMBER_F => {
                if (matches!(c, b',' | b'}') || i >= self.n) && self.pop_zero() {
                    return true;
                }
                if self.comment(false) {
                    return true;
                }
                if f == ENUM_MEMBER_F && (self.string() || self.array_literal()) {
                    return true;
                }
                c == b'=' && self.initializer()
            }
            NAMESPACE => {
                if ((i > 0 && self.b[i - 1] == b'}') || self.decl_end(i)) && self.pop_zero() {
                    return true;
                }
                if self.comment(false) || self.string() {
                    return true;
                }
                let ne = self.name(i);
                if ne > i {
                    return self.take(i, ne, TYPE_MODULE);
                }
                if self.accessor() {
                    return true;
                }
                if c == b'{' {
                    self.tok(i, i + 1, BLOCK_BRACE);
                    self.i = i + 1;
                    self.set(BLOCK);
                    return true;
                }
                false
            }
            IMPORT_F => {
                // (?<!^import|[^\._$[:alnum:]]import)(?=;|$|^)
                if (c == b';' || i >= self.n || i == 0) && !self.word_ends_at(i, &[b"import"]) && self.pop_zero() {
                    return true;
                }
                if self.comment(true) || self.string() {
                    return true;
                }
                self.import_export_declaration()
            }
            IMPORT_CLAUSE => {
                if let Some(e) = self.word_b(i, b"from")
                    && self.boundary(i)
                {
                    self.tok(i, e, FROM);
                    self.i = e;
                    return self.pop();
                }
                self.import_export_declaration()
            }
            IMPORT_BLOCK => {
                if c == b'}' {
                    self.tok(i, i + 1, BLOCK_BRACE);
                    self.i = i + 1;
                    return self.pop();
                }
                self.import_export_clause()
            }
            EXPORT_F => {
                if (i >= self.n || c == b';' || self.blank_at(i) || self.decl_at_bol(i)) && self.pop_zero() {
                    return true;
                }
                self.import_export_declaration()
            }
            EXPORT_DEFAULT => {
                if (i >= self.n || c == b';' || self.blank_at(i) || self.decl_at_bol(i)) && self.pop_zero() {
                    return true;
                }
                self.interface_in_export() || self.expression()
            }
            IMPORT_REQUIRE => {
                if c == b')' {
                    self.tok(i, i + 1, ROUND);
                    self.i = i + 1;
                    return self.pop();
                }
                self.comment(false) || self.string()
            }
            IMPORT_ALIAS => {
                if (c == b';' || i >= self.n || i == 0) && self.pop_zero() {
                    return true;
                }
                if self.comment(true) {
                    return true;
                }
                let ne = self.name(i);
                if ne > i {
                    let k = self.ws(ne);
                    if self.c(k) == b'.' || (self.c(k) == b'?' && self.c(k + 1) == b'.' && !self.c(self.ws(k + 2)).is_ascii_digit()) {
                        self.tok(i, ne, TYPE_MODULE);
                        return if self.c(k) == b'.' {
                            self.take(k, k + 1, ACCESSOR)
                        } else {
                            self.take(k, k + 2, ACCESSOR_OPT)
                        };
                    }
                    return self.take(i, ne, READWRITE);
                }
                false
            }
            FOR => {
                if i > 0 && self.b[i - 1] == b')' && self.pop_zero() {
                    return true;
                }
                if self.comment(false) {
                    return true;
                }
                if let Some(e) = self.word_b(i, b"await") {
                    return self.take(i, e, LOOP);
                }
                if c == b'(' {
                    self.tok(i, i + 1, ROUND);
                    self.i = i + 1;
                    self.push(FOR_PARENS);
                    return true;
                }
                false
            }
            FOR_PARENS => {
                if c == b')' {
                    self.tok(i, i + 1, ROUND);
                    self.i = i + 1;
                    return self.pop();
                }
                if self.alpha_or_at(c) && self.wb(i) {
                    let (_, _, j) = self.prefix(i);
                    let w = self.name(j);
                    let word = &self.b[j..w];
                    if (word == b"var" || word == b"let") && self.we(w) {
                        return self.push_zero(VAR);
                    }
                    if word == b"const" && self.we(w) {
                        return self.push_zero(VAR_CONST);
                    }
                }
                self.expression() || (c == b';' && self.take(i, i + 1, TERMINATOR))
            }
            IF => {
                if (c == b';' || i >= self.n || c == b'}') && self.pop_zero() {
                    return true;
                }
                if self.comment(false) {
                    return true;
                }
                if let Some(e) = self.kw(i, b"if") {
                    let k = self.ws(e);
                    if self.c(k) == b'(' {
                        self.tok(i, e, CONDITIONAL);
                        self.tok(k, k + 1, ROUND);
                        self.i = k + 1;
                        self.push(IF_COND);
                        return true;
                    }
                }
                if self.if_regex() {
                    return true;
                }
                self.statements()
            }
            SWITCH_F => {
                if c == b'}' {
                    self.tok(i, i + 1, BLOCK_BRACE);
                    self.i = i + 1;
                    return self.pop();
                }
                if self.comment(false) {
                    return true;
                }
                if let Some(e) = self.kw(i, b"switch") {
                    let k = self.ws(e);
                    if self.c(k) == b'(' {
                        self.tok(i, e, SWITCH);
                        self.tok(k, k + 1, ROUND);
                        self.i = k + 1;
                        self.push(SWITCH_EXPR);
                        return true;
                    }
                }
                if c == b'{' {
                    self.tok(i, i + 1, BLOCK_BRACE);
                    self.i = i + 1;
                    self.set(SWITCH_BLOCK); // the switch ends with its block
                    return true;
                }
                false
            }
            SWITCH_BLOCK => {
                if c == b'}' {
                    self.tok(i, i + 1, BLOCK_BRACE);
                    self.i = i + 1;
                    return self.pop();
                }
                if self.wb(i) {
                    for (w, needs_colon) in [(&b"case"[..], false), (b"default", true)] {
                        if let Some(e) = self.word_b(i, w)
                            && self.we(e)
                            && (!needs_colon || self.c(e) == b':')
                        {
                            self.tok(i, e, SWITCH);
                            self.i = e;
                            self.push(CASE);
                            return true;
                        }
                    }
                }
                if c == b':' {
                    let k = self.ws(i + 1);
                    if self.c(k) == b'{' {
                        self.tok(i, i + 1, CASE_SEP);
                        self.tok(k, k + 1, BLOCK_BRACE);
                        self.i = k + 1;
                        self.push(CASE_BLOCK);
                        return true;
                    }
                    return self.take(i, i + 1, CASE_SEP);
                }
                self.statements()
            }
            CASE => {
                if c == b':' && self.pop_zero() {
                    return true;
                }
                self.expression()
            }
            YIELD_STAR => {
                if c == b'*' {
                    self.tok(i, i + 1, ASTERISK);
                    self.i = i + 1;
                    return self.pop();
                }
                self.comment(false)
            }
            _ => false,
        }
    }

    /// `^\s*(export|function|class|…)\s+` at `i`, an arrow's head's end.
    pub(super) fn arrow_head_bol(&self, i: usize) -> bool {
        if i != 0 {
            return false;
        }
        let j = self.ws(0);
        [&b"export"[..], b"function", b"class", b"interface", b"let", b"var", b"const", b"import", b"enum", b"namespace", b"module", b"type", b"abstract", b"declare"]
            .iter()
            .any(|w| self.rest(j).starts_with(w) && is_space(self.c(j + w.len())))
    }

    /// Whether the line is the block's first (a `#!` line counts only there).
    pub(super) fn exact_first_line(&self) -> bool {
        // `\A`: the state says nothing of the line's place; a `#!` line is taken anywhere at
        // the top level
        true
    }

    /// `directives`: `^(///)\s*(?=<(reference|amd-dependency|amd-module)(\s+(path|types|no-default-lib|lib|name)\s*=\s*(«sq»|«dq»|«bq»))+\s*/>\s*$)`.
    pub(super) fn directive(&mut self) -> bool {
        if !self.b.starts_with(b"///") {
            return false;
        }
        let j = self.ws(3);
        if self.c(j) != b'<' {
            return false;
        }
        let r = self.rest(j + 1);
        let w = [&b"reference"[..], b"amd-dependency", b"amd-module"].into_iter().find(|w| r.starts_with(w));
        let Some(w) = w else { return false };
        let mut k = j + 1 + w.len();
        let mut any = false;
        loop {
            let m = self.ws(k);
            if m == k {
                break;
            }
            let a = [&b"path"[..], b"types", b"no-default-lib", b"lib", b"name"].into_iter().find(|a| self.rest(m).starts_with(a));
            let Some(a) = a else { break };
            let q = self.ws(m + a.len());
            if self.c(q) != b'=' {
                break;
            }
            let s = self.ws(q + 1);
            let qc = self.c(s);
            if !matches!(qc, b'\'' | b'"' | b'`') {
                break;
            }
            let Some(e) = look::quoted(self.b, s, qc) else { break };
            k = e;
            any = true;
        }
        let m = self.ws(k);
        if !any || !(self.c(m) == b'/' && self.c(m + 1) == b'>') || self.ws(m + 2) < self.n {
            return false;
        }
        self.tok(0, 3, DIRECTIVE_MARK);
        self.i = 3;
        self.push(DIRECTIVE_F);
        self.i = j;
        true
    }

    /// The parts of a function or method frame: name, parameters, return type, body.
    pub(super) fn function_frame(&mut self, f: u8) -> bool {
        let i = self.i;
        let c = self.c(i);
        let after_brace = i > 0 && self.b[i - 1] == b'}';
        let ends = match f {
            FUNC => after_brace || self.decl_end(i),
            FUNC_EXPR => after_brace || c == b';',
            METHOD | OBJ_KEY_METHOD => after_brace || matches!(c, b'}' | b';' | b',') || (f == METHOD && i >= self.n),
            _ => after_brace || matches!(c, b'}' | b';' | b','),
        };
        if ends && self.pop_zero() {
            return true;
        }
        match f {
            FUNC | FUNC_EXPR => {
                let ne = self.name(i);
                if ne > i {
                    return self.take(i, ne, ENTITY_FN);
                }
                if f == FUNC_EXPR && self.comment(true) {
                    return true;
                }
            }
            METHOD => {
                if self.method_name() {
                    return true;
                }
            }
            OBJ_METHOD
                if self.method_name() => {
                    return true;
                }
            _ => {}
        }
        if self.function_body() {
            return true;
        }
        if f == OBJ_METHOD {
            // the second pattern: async/get/set/* before a key and `(`
            return self.obj_method_head(true);
        }
        false
    }

    /// `method-declaration-name`: a key ahead before `(` or `<`.
    pub(super) fn method_name(&mut self) -> bool {
        let i = self.i;
        let Some(e) = self.member_key(i) else { return false };
        let mut r = self.ws(e);
        if self.c(r) == b'?' {
            r = self.ws(r + 1);
        }
        if !matches!(self.c(r), b'(' | b'<') {
            return false;
        }
        self.push_zero(METHOD_NAME)
    }

    /// `function-body` at the place.
    pub(super) fn function_body(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        if self.comment(false) {
            return true;
        }
        match c {
            b'<' => self.type_parameters(),
            b'(' => {
                self.tok(i, i + 1, PARAMS_BEGIN);
                self.i = i + 1;
                self.push(PARAMS);
                true
            }
            b'{' => {
                self.tok(i, i + 1, BLOCK_BRACE);
                self.i = i + 1;
                self.set(BLOCK); // a function or method ends with its body
                true
            }
            b'*' => self.take(i, i + 1, ASTERISK),
            b'=' if self.c(i + 1) == b'>' => self.type_function_return(),
            _ => self.return_type(),
        }
    }

    /// `function-parameters-body` at the place.
    pub(super) fn parameters_body(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        if self.comment(false) || self.string() || self.decorator() {
            return true;
        }
        if self.destructuring_parameter() {
            return true;
        }
        if self.parameter_name() {
            return true;
        }
        match c {
            b':' => self.parameter_type(),
            b'=' => self.initializer(),
            b',' => self.take(i, i + 1, PARAM_SEP),
            _ => false,
        }
    }

    /// `parameter-type-annotation`.
    pub(super) fn parameter_type(&mut self) -> bool {
        let i = self.i;
        self.tok(i, i + 1, ANNOTATION);
        self.i = i + 1;
        self.push(PARAM_TYPE);
        true
    }

    /// `parameter-name` at the place.
    pub(super) fn parameter_name(&mut self) -> bool {
        let i = self.i;
        if !self.wb(i) {
            return self.rest_param_prefix_only();
        }
        let mods: &[&[u8]] = &[b"override", b"public", b"protected", b"private", b"readonly"];
        // a modifier before another
        for m in mods {
            if self.rest(i).starts_with(m) && self.boundary(i + m.len()) {
                let k = self.ws(i + m.len());
                if k > i + m.len() && mods.iter().any(|m2| self.rest(k).starts_with(m2) && is_space(self.c(k + m2.len()))) {
                    return self.take(i, i + m.len(), MODIFIER);
                }
            }
        }
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
        // (?<!=|:)«wb»(this|name)«we»\s*(\??)
        if j > 0 && matches!(self.b[j - 1], b'=' | b':') {
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

    /// `...` before a parameter's name that isn't at a word's start (none: the pattern needs it).
    pub(super) fn rest_param_prefix_only(&mut self) -> bool {
        false
    }

    /// `destructuring-parameter`: `(?<!=|:)\s*(?:(\.\.\.)\s*)?(\{|\[)`.
    pub(super) fn destructuring_parameter(&mut self) -> bool {
        let i = self.i;
        if i > 0 && matches!(self.b[i - 1], b'=' | b':') {
            return false;
        }
        let mut j = self.ws(i);
        let mut rest = None;
        if self.rest(j).starts_with(b"...") {
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

    /// `var-expr`'s turn.
    pub(super) fn var_step(&mut self, f: u8) -> bool {
        let i = self.i;
        let c = self.c(i);
        let cst = f == VAR_CONST;
        // the pop: not at the keyword again, and at `;`, `}`, of/in, a blank line, a declaration
        // at the line's start, or the line's end (not right after the keyword)
        let at_kw = {
            let (_, _, j) = self.prefix(i);
            let w = self.name(j);
            let word = &self.b[j..w];
            self.wb(i) && self.we(w) && if cst { word == b"const" } else { word == b"var" || word == b"let" }
        };
        if !at_kw {
            // (?<!…const)(?=\s*$): not right after the keyword, but syntect tries it past the
            // line's `\n` too, where it holds unless the keyword's own `\s*` took the `\n`
            let at_end = {
                let e = self.ws(i);
                e >= self.n && (!self.word_ends_at(i, if cst { &[b"const"] } else { &[b"let", b"var"] }) || !self.var_kw_eol)
            };
            if (matches!(c, b';' | b'}') || self.of_in(i) || self.blank_at(i) || self.decl_at_bol(i) || at_end) && self.pop_zero() {
                return true;
            }
        } else {
            let (export, declare, j) = self.prefix(i);
            let w = self.name(j);
            self.prefix_toks(i, export, declare, j);
            self.tok(j, w, STORAGE);
            self.i = self.ws(w).min(self.n).max(w);
            self.var_kw_eol = self.i >= self.n;
            if self.var_kw_eol {
                // `\s*` took the `\n`: its `(?=\S)` push holds the next lines' blanks
                self.push(GAP);
            }
            return true;
        }
        if self.destructuring(cst) || self.var_single(cst) {
            return true;
        }
        if c == b'=' && self.initializer() {
            return true;
        }
        if self.comment(false) {
            return true;
        }
        if c == b',' {
            let k = self.ws(i + 1);
            self.tok(i, i + 1, COMMA);
            self.i = i + 1;
            if k >= self.n || (self.c(k) == b'/' && self.c(k + 1) == b'/') {
                self.i = k.min(self.n);
                self.push(if cst { VAR_CONST_MORE } else { VAR_MORE });
            }
            return true;
        }
        false
    }

    /// `destructuring-variable`/`-const`: `(?<!=|:|^of|…of|^in|…in)\s*(?=\{|\[)`.
    pub(super) fn destructuring(&mut self, cst: bool) -> bool {
        let i = self.i;
        if i > 0 && matches!(self.b[i - 1], b'=' | b':') {
            return false;
        }
        if self.word_ends_at(i, &[b"of", b"in"]) {
            return false;
        }
        let k = self.ws(i);
        if !matches!(self.c(k), b'{' | b'[') {
            return false;
        }
        self.i = k.min(self.n);
        let f = match (self.c(k) == b'[', cst) {
            (false, false) => DESTRUCT,
            (false, true) => DESTRUCT_CONST,
            (true, false) => DESTRUCT_ARR,
            (true, true) => DESTRUCT_ARR_CONST,
        };
        if k == i {
            self.push_zero(f)
        } else {
            self.push(f);
            true
        }
    }

    /// `var-single-variable`/`-const` at a name.
    pub(super) fn var_single(&mut self, cst: bool) -> bool {
        let i = self.i;
        let ne = self.name(i);
        if ne == i {
            return false;
        }
        let mut e = ne;
        let mut bang = None;
        if !cst && self.c(ne) == b'!' {
            bang = Some(ne);
            e = ne + 1;
        }
        let fun = look::fn_decl(self.b, e, self.tsx);
        let kind = if fun {
            if cst { CONST_FN } else { ENTITY_FN }
        } else if cst {
            CONSTANT
        } else {
            let u = look::upper(self.b, i);
            if u == ne && !self.alnum_at(u) { CONSTANT } else { READWRITE }
        };
        self.tok(i, ne, kind);
        if let Some(b) = bang {
            self.tok(b, b + 1, DEFINITE);
        }
        self.i = e;
        self.push(VAR_NAME);
        true
    }

    /// `object-binding-pattern`/`array-binding-pattern` (or the const ones) at the place.
    pub(super) fn binding_pattern(&mut self, cst: bool, _param: bool) -> bool {
        let i = self.i;
        let mut j = i;
        let mut rest = None;
        if self.rest(i).starts_with(b"...") {
            rest = Some(i);
            j = self.ws(i + 3);
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
            self.push(if cst { BIND_OBJ_CONST } else { BIND_OBJ_F });
        } else {
            self.tok(j, j + 1, BIND_ARR);
            self.i = j + 1;
            self.push(if cst { BIND_ARR_CONST } else { BIND_ARR_F });
        }
        true
    }

    /// `object-binding-element` (and the const and parameter ones) at the place.
    pub(super) fn binding_element_obj(&mut self, f: u8) -> bool {
        let i = self.i;
        let c = self.c(i);
        if self.comment(false) {
            return true;
        }
        // a key and `:` ahead
        if let Some(e) = self.member_key(i)
            && self.c(self.ws(e)) == b':'
        {
            self.push_zero(match f {
                BIND_OBJ_F => BIND_ELEM,
                BIND_OBJ_CONST => BIND_ELEM_CONST,
                _ => PBIND_ELEM,
            });
            return true;
        }
        let cst = f == BIND_OBJ_CONST;
        if f == PBIND_OBJ {
            if self.c(i) == b'{' || (self.rest(i).starts_with(b"...") && self.c(self.ws(i + 3)) == b'{') {
                return self.pbinding_obj();
            }
        } else if self.binding_obj_only(cst) {
            return true;
        }
        if self.rest_name(f) {
            return true;
        }
        match c {
            b'=' => self.initializer(),
            b',' => self.take(i, i + 1, COMMA),
            _ => false,
        }
    }

    /// `object-binding-pattern` alone (not the array's).
    pub(super) fn binding_obj_only(&mut self, cst: bool) -> bool {
        let i = self.i;
        let j = if self.rest(i).starts_with(b"...") { self.ws(i + 3) } else { i };
        if self.c(j) != b'{' {
            return false;
        }
        self.binding_pattern(cst, false)
    }

    /// `parameter-object-binding-pattern`.
    pub(super) fn pbinding_obj(&mut self) -> bool {
        let i = self.i;
        let mut j = i;
        if self.rest(i).starts_with(b"...") {
            self.tok(i, i + 3, REST);
            j = self.ws(i + 3);
        }
        self.tok(j, j + 1, BIND_OBJ);
        self.i = j + 1;
        self.push(PBIND_OBJ);
        true
    }

    /// `destructuring-variable-rest`/`-const`/`destructuring-parameter-rest`: `(?:(\.\.\.)\s*)?(name)`.
    pub(super) fn rest_name(&mut self, f: u8) -> bool {
        let i = self.i;
        let mut j = i;
        let mut rest = None;
        if self.rest(i).starts_with(b"...") {
            rest = Some(i);
            j = self.ws(i + 3);
        }
        let ne = self.name(j);
        if ne == j {
            return false;
        }
        if let Some(r) = rest {
            self.tok(r, r + 3, REST);
        }
        let kind = match f {
            BIND_OBJ_CONST | BIND_ARR_CONST => CONSTANT,
            PBIND_OBJ | PBIND_ARR => PARAM,
            _ => READWRITE,
        };
        self.take(j, ne, kind)
    }

    /// `binding-element` (and the const and parameter ones).
    pub(super) fn binding_element(&mut self, f: u8) -> bool {
        let i = self.i;
        let c = self.c(i);
        if self.comment(false) || self.string() || self.numeric() || self.regex() {
            return true;
        }
        let cst = matches!(f, BIND_ARR_CONST | BIND_OBJ_CONST);
        if matches!(f, PBIND_ARR | PBIND_OBJ) {
            let j = if self.rest(i).starts_with(b"...") { self.ws(i + 3) } else { i };
            match self.c(j) {
                b'{' => return self.pbinding_obj(),
                b'[' => {
                    if j > i {
                        self.tok(i, i + 3, REST);
                    }
                    self.tok(j, j + 1, BIND_ARR);
                    self.i = j + 1;
                    self.push(PBIND_ARR);
                    return true;
                }
                _ => {}
            }
        } else if self.binding_pattern(cst, false) {
            return true;
        }
        if self.rest_name(f) {
            return true;
        }
        c == b'=' && self.initializer()
    }

    /// `object-binding-element-propertyName`: a key and `:` ahead.
    pub(super) fn binding_key(&mut self) -> bool {
        let i = self.i;
        if let Some(e) = self.member_key(i)
            && self.c(self.ws(e)) == b':'
        {
            return self.push_zero(BIND_KEY);
        }
        false
    }

    /// `class-or-interface-heritage`: `extends` or `implements`.
    pub(super) fn heritage(&mut self) -> bool {
        let i = self.i;
        for w in [&b"extends"[..], b"implements"] {
            if let Some(e) = self.kw(i, w) {
                self.tok(i, e, MODIFIER);
                self.i = e;
                self.push(HERITAGE);
                return true;
            }
        }
        false
    }

    /// The heritage's patterns.
    pub(super) fn heritage_body(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        if self.comment(false) || self.heritage() {
            return true;
        }
        if c == b'<' && self.type_parameters() {
            return true;
        }
        if self.expression_without_identifiers() {
            return true;
        }
        let ne = self.name(i);
        if ne > i {
            // (name)\s*(?:(\.)|(\?\.(?!\s*[[:digit:]])))(?=\s*name(\s*\??\.\s*name)*\s*)
            let k = self.ws(ne);
            let (acc, ae) = if self.c(k) == b'.' {
                (ACCESSOR, k + 1)
            } else if self.c(k) == b'?' && self.c(k + 1) == b'.' && !self.c(self.ws(k + 2)).is_ascii_digit() {
                (ACCESSOR_OPT, k + 2)
            } else {
                (PLAIN, k)
            };
            if acc != PLAIN {
                let m = self.ws(ae);
                if self.name(m) > m {
                    self.tok(i, ne, TYPE_MODULE);
                    return self.take(k, ae, acc);
                }
            }
            return self.take(i, ne, INHERITED);
        }
        self.expression_punctuations()
    }

    /// `import-export-declaration`.
    pub(super) fn import_export_declaration(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        if self.comment(false) || self.string() {
            return true;
        }
        if c == b'{' {
            self.tok(i, i + 1, BLOCK_BRACE);
            self.i = i + 1;
            self.push(IMPORT_BLOCK);
            return true;
        }
        if let Some(e) = self.word_b(i, b"from")
            && self.boundary(i)
        {
            return self.take(i, e, FROM);
        }
        self.import_export_clause()
    }

    /// `import-export-clause`.
    pub(super) fn import_export_clause(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        if self.comment(false) {
            return true;
        }
        // «wb»(?:(\bdefault)|(\*)|(\bname))\s+(as)\s+(?:(default«we»)|(name))
        if self.wb(i) {
            let (e, kind) = if let Some(e) = self.word_b(i, b"default").filter(|_| self.boundary(i)) {
                (e, DEFAULT)
            } else if c == b'*' {
                (i + 1, IMPORT_ALL)
            } else if look::is_word_at(self.b, i) {
                // `\b` before the name: not before `$`
                let ne = self.name(i);
                (ne, READWRITE)
            } else {
                (i, READWRITE)
            };
            if e > i {
                let a = self.ws(e);
                if a > e && let Some(ae) = self.word_b(a, b"as") {
                    let m = self.ws(ae);
                    if m > ae {
                        let (me, mk) = match self.word_b(m, b"default") {
                            Some(d) if self.we(d) => (d, DEFAULT),
                            _ => (self.name(m), ALIAS),
                        };
                        if me > m {
                            self.tok(i, e, kind);
                            self.tok(a, ae, AS);
                            return self.take(m, me, mk);
                        }
                    }
                }
            }
        }
        match c {
            b',' => return self.take(i, i + 1, COMMA),
            b'*' => return self.take(i, i + 1, IMPORT_ALL),
            _ => {}
        }
        if let Some(e) = self.word_b(i, b"default").filter(|_| self.boundary(i)) {
            return self.take(i, e, DEFAULT);
        }
        let ne = self.name(i);
        ne > i && self.take(i, ne, ALIAS)
    }

    /// `export default interface …`: `interface-declaration` inside `meta.export.default`.
    pub(super) fn interface_in_export(&mut self) -> bool {
        let i = self.i;
        if !self.wb(i) {
            return false;
        }
        let (export, declare, j) = self.prefix(i);
        let w = self.name(j);
        let (abs, k) = if self.is_name(j, w, b"abstract") {
            let k = self.ws(w);
            if k > w { (Some(w), k) } else { (None, j) }
        } else {
            (None, j)
        };
        if let Some(e) = self.word_b(k, b"interface")
            && (is_space(self.c(e)) || (self.c(e) == b'/' && matches!(self.c(e + 1), b'/' | b'*')))
        {
            self.prefix_toks(i, export, declare, j);
            if let Some(a) = abs {
                self.tok(j, a, MODIFIER);
            }
            self.tok(k, e, INTERFACE_KW);
            self.i = e;
            self.push(INTERFACE);
            return true;
        }
        false
    }

    /// The `if`'s regex right after its `)` (`if-statement`'s).
    pub(super) fn if_regex(&mut self) -> bool {
        false
    }
}
