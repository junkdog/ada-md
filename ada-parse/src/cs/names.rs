//! Names, words, types and the punctuation that opens and closes frames: what a name is by
//! where it stands ([`Scan::prev`], the frame, [`Scan::st`]), a declaration's type read once
//! more as one ([`Scan::ty`]), a group's writing taken back where it turns out a lambda's
//! parameters or a cast ([`Group`]).

use super::words::{self, *};
use super::*;
use crate::prim::{is_word_starting, word_end};

/// Whether `X122_SOL` builds the floor probe: every name a variable, every keyword its leaf, no
/// declaration, type or group read (`x122`'s floor).
const SOL: bool = option_env!("ADA_PARSE_X122_SOL").is_some();

/// Where the blanks at `p` end.
#[inline(always)]
pub(super) fn blanks(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && ACT[usize::from(b[p])] == B_SPACE {
        p += 1;
    }
    p
}

/// Whether a name starts at `p`: a letter, `_`, `@` before one, a letter past ASCII.
#[inline(always)]
pub(super) fn name_start(b: &[u8], p: usize) -> bool {
    match b.get(p) {
        Some(&c) if c < 0x80 => {
            ACT[usize::from(c)] == B_NAME
                || c == b'@' && b.get(p + 1).is_some_and(|&d| ACT[usize::from(d)] == B_NAME)
        }
        Some(_) => is_word_starting(b, p),
        None => false,
    }
}

/// Where the name at `p` ([`name_start`]) ends.
#[inline(always)]
pub(super) fn name_end(b: &[u8], p: usize) -> usize {
    let mut e = p + 1;
    while e < b.len() && NAME[usize::from(b[e])] {
        e += 1;
    }
    if e < b.len() && b[e] >= 0x80 {
        e = word_end(b, e);
    }
    e
}

/// Whether the byte at `p` follows a type's `?` or `*` (not a conditional's or a product's): a
/// blank, a closer, a list's `,`, another suffix, or the line's end.
#[inline(always)]
fn suffix_ok(b: &[u8], p: usize) -> bool {
    matches!(
        b.get(p),
        None | Some(b' ' | b'\t' | b'\r' | b'>' | b',' | b')' | b']' | b'[' | b'*' | b'?')
    )
}

/// Whether type arguments that close before `p` are a generic's: C#'s rule (§12.8.10), the
/// token after them one of `( ) ] } : ; , . ? == != | ^ && || & [`, a nested list's `>`, or
/// the line's end.
#[inline(always)]
fn generic_follows(b: &[u8], p: usize) -> bool {
    match b.get(p) {
        None => true,
        Some(b'(' | b')' | b']' | b'}' | b':' | b';' | b',' | b'.' | b'?' | b'|' | b'^' | b'&')
        | Some(b'[' | b'>') => true,
        Some(b'=' | b'!') => b.get(p + 1) == Some(&b'='),
        Some(b' ' | b'\t') => {
            let q = blanks(b, p);
            matches!(
                b.get(q),
                None | Some(b'(' | b')' | b']' | b'}' | b':' | b';' | b',' | b'.' | b'?' | b'{')
            ) || b
                .get(q..q + 2)
                .is_some_and(|s| s == b"==" || s == b"!=" || s == b"=>")
        }
        _ => false,
    }
}

/// Whether what follows a group's `)` at `q` makes it a cast (C#'s rule, §12.9.7): `~`, `!`, `(`,
/// a name, a literal, or a keyword but `as` and `is` (and the other words an operand can't
/// start with).
fn cast_follows(b: &[u8], q: usize) -> bool {
    match b.get(q) {
        Some(b'~' | b'(' | b'"' | b'\'' | b'$' | b'0'..=b'9') => true,
        Some(b'!') => b.get(q + 1) != Some(&b'='),
        Some(b'@') => true,
        Some(_) if name_start(b, q) => {
            let i = words::info(&b[q..name_end(b, q)]);
            i == 0
                || !matches!(
                    (i >> 8) as u8,
                    A_IS | A_PATTERN | A_WITH | A_WHEN | A_PARAM | A_QUERY | A_COND
                )
        }
        _ => false,
    }
}

impl<K: Sink> Scan<'_, '_, K> {
    /// A name at `p`: a word looked up once, a keyword's action, or a name by where it stands.
    #[inline(always)]
    pub(super) fn name(&mut self, p: usize) -> usize {
        let b = self.b;
        let e = name_end(b, p);
        let info = words::info(&b[p..e]);
        if SOL {
            self.put_hot(p, e, if info != 0 { info & 0xff } else { VAR_OTHER });
            self.prev = P_VALUE;
            return e;
        }
        if info != 0 {
            return self.word(p, e, info);
        }
        self.ident(p, e)
    }

    /// A name that is no keyword at `p..e`.
    #[inline(always)]
    fn ident(&mut self, p: usize, e: usize) -> usize {
        match self.prev {
            P_DOT => self.member(p, e),
            P_EXPR | P_VALUE => self.expr_name(p, e, true),
            _ => self.ident_slow(p, e, 0),
        }
    }

    /// A member's name after `.`: a call's or a field's (one color), a generic's.
    #[inline(always)]
    fn member(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        match b.get(e) {
            Some(b'(') => self.put_hot(p, e, VAR_FUNCTION),
            Some(b'<') => return self.generic_name(p, e),
            Some(b':') if self.case_constant(e) => self.put(p, e, CONSTANT),
            _ => self.put_hot(p, e, VAR_OTHER),
        }
        self.prev = P_VALUE;
        e
    }

    /// Whether a name before `:` at `e` is a `case` label's constant (bat's `constant.other`):
    /// in a `case`'s pattern, before its `when`, not `::`.
    #[inline(always)]
    fn case_constant(&self, e: usize) -> bool {
        self.st & (S_CASE | S_PAT) == S_CASE | S_PAT && self.b.get(e + 1) != Some(&b':')
    }

    /// A name in an expression: a variable's, a call's, a generic's, or (where `lambda`) a
    /// lambda's parameter before `=>`.
    #[inline(always)]
    fn expr_name(&mut self, p: usize, e: usize, lambda: bool) -> usize {
        let b = self.b;
        match b.get(e) {
            Some(b'(') => self.put_hot(p, e, VAR_FUNCTION),
            Some(b'<') => return self.generic_name(p, e),
            Some(b':') if self.case_constant(e) => self.put(p, e, CONSTANT),
            Some(b' ' | b'\t' | b'=') if lambda && self.prev != P_VALUE => {
                let q = blanks(b, e);
                if b.get(q..q + 2) == Some(b"=>") {
                    self.put(p, e, PARAMETER);
                    self.prev = P_LAMBDA;
                    return e;
                }
                self.put_hot(p, e, VAR_OTHER);
            }
            _ => self.put_hot(p, e, VAR_OTHER),
        }
        self.prev = P_VALUE;
        e
    }

    /// A name `p..e` before `<`: a generic call's or type's where its type arguments close
    /// before the bytes C# names ([`generic_follows`]), else a variable before `<`.
    #[inline(never)]
    fn generic_name(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        if let Some(ae) = self.ty_args::<false>(e, 1)
            && generic_follows(b, ae)
        {
            let leaf = if b.get(ae) == Some(&b'(') {
                VAR_FUNCTION
            } else {
                SUPPORT_TYPE
            };
            self.put(p, e, leaf);
            let ae = self.ty_args::<true>(e, 1).unwrap_or(ae);
            self.prev = P_VALUE;
            return ae;
        }
        self.put_hot(p, e, VAR_OTHER);
        self.prev = P_VALUE;
        e
    }

    /// A name (or a type's word, `kw` its action) where the token before decides more than an
    /// expression's: a statement's start, a list's item, a type's, a declared name, an entity's
    /// name, a base, a label, a parameter, an arm's pattern.
    #[inline(never)]
    fn ident_slow(&mut self, p: usize, e: usize, kw: u8) -> usize {
        match self.prev {
            P_STMT => self.stmt_name(p, e, kw),
            P_LIST => self.list_name(p, e, kw),
            P_TYPE => self.type_name(p),
            P_NEW => {
                let te = self.ty(p);
                self.prev = P_VALUE;
                te
            }
            P_DECLARE => self.declared(p, e),
            P_ENTITY => self.entity_name(p, e),
            P_BASE => self.base_name(p),
            P_GOTO => {
                self.put(p, e, LABEL_REF);
                self.prev = P_VALUE;
                e
            }
            P_PARAM => self.param_name(p, e),
            P_ARM => self.arm_name(p, e, kw),
            _ => self.expr_word(p, e, kw, true),
        }
    }

    /// A name or a type's word in an expression: a type's word its leaf, a name as
    /// [`Scan::expr_name`].
    #[inline(always)]
    fn expr_word(&mut self, p: usize, e: usize, kw: u8, lambda: bool) -> usize {
        if kw == A_TYPE {
            self.put(p, e, STORAGE_TYPE);
            self.prev = P_VALUE;
            return e;
        }
        self.expr_name(p, e, lambda)
    }

    /// A type at `p` where one is read once its shape is known ([`Scan::ty_at`]): its tokens,
    /// or where it isn't one, its first name's.
    pub(super) fn ty(&mut self, p: usize) -> usize {
        // a plain name, nothing after it that goes on with a type: written at once
        let b = self.b;
        if name_start(b, p) {
            let e = name_end(b, p);
            if !matches!(b.get(e), Some(b'<' | b'.' | b'?' | b'*' | b'[' | b':')) {
                let i = words::info(&b[p..e]);
                let leaf = match (i >> 8) as u8 {
                    _ if i == 0 => SUPPORT_TYPE,
                    A_TYPE => STORAGE_TYPE,
                    A_VAR => VAR_KW,
                    A_CMOD | A_RECORD | A_ACCESSOR | A_QUERY | A_CONSTRAINT | A_NAMEVALUE
                    | A_AWAIT | A_WITH | A_PATTERN | A_WHERE | A_WHEN => SUPPORT_TYPE,
                    _ => i & 0xff,
                };
                self.put(p, e, leaf);
                return e;
            }
        }
        if self.ty_at::<false>(p, 0).is_some()
            && let Some(e) = self.ty_at::<true>(p, 0)
        {
            return e;
        }
        let b = self.b;
        if name_start(b, p) {
            let e = name_end(b, p);
            let i = words::info(&b[p..e]);
            let leaf = match (i >> 8) as u8 {
                _ if i == 0 => SUPPORT_TYPE,
                A_TYPE => STORAGE_TYPE,
                A_VAR => VAR_KW,
                _ => i & 0xff,
            };
            self.put(p, e, leaf);
            return e;
        }
        p + 1
    }

    /// A type at `p` whose shape [`Scan::ty_at`] has checked: written.
    #[inline(always)]
    fn ty_known(&mut self, p: usize) -> usize {
        self.ty_at::<true>(p, 0).unwrap_or(p + 1)
    }

    /// Where a declaration's type at `p` (its first name to `e`) ends, where a name it declares
    /// follows: the type's end. Looked for only where the byte after the name can go on with a
    /// type or start the declared name.
    #[inline(always)]
    fn decl_at(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        let q = blanks(b, e);
        match b.get(q) {
            Some(b'<' | b'.' | b'?' | b'[' | b'*') if q == e => {}
            Some(b':') if q == e && b.get(q + 1) == Some(&b':') => {}
            Some(_) if q > e && name_start(b, q) => {}
            _ => return None,
        }
        let te = self.ty_at::<false>(p, 0)?;
        let q = blanks(b, te);
        if q == te && NAME[usize::from(b[te - 1])] {
            return None;
        }
        if !name_start(b, q) {
            return None;
        }
        let we = name_end(b, q);
        if words::not_a_name(words::info(&b[q..we])) {
            return None;
        }
        Some(te)
    }

    /// A name at a statement's or a member's start.
    fn stmt_name(&mut self, p: usize, e: usize, kw: u8) -> usize {
        let b = self.b;
        let top = self.top();
        match top {
            F_TYPE => {
                let q = blanks(b, e);
                // a constructor's name: no type before it
                if kw == 0 && b.get(q) == Some(&b'(') {
                    self.put(p, e, CTOR);
                    self.prev = P_FUNC;
                    self.st = S_DECL;
                    return e;
                }
                if let Some(te) = self.ty_at::<false>(p, 0) {
                    let q = blanks(b, te);
                    if q > te && name_start(b, q) {
                        let delegate = self.st & S_KIND == S_DELEGATE;
                        let te = self.ty_known(p);
                        self.prev = P_DECLARE;
                        self.st = if delegate { S_DELEGATE } else { S_DECL };
                        return te;
                    }
                }
                self.expr_word(p, e, kw, true)
            }
            F_ENUM => {
                self.put(p, e, ENUM_MEMBER);
                self.prev = P_DECLARED;
                e
            }
            _ => {
                // a label
                if kw == 0
                    && b.get(e) == Some(&b':')
                    && b.get(e + 1) != Some(&b':')
                    && matches!(top, 0 | F_CODE | F_LAMBDA)
                {
                    self.put(p, e, LABEL);
                    self.put(e, e + 1, LABEL_SEP);
                    return e + 1;
                }
                if self.decl_at(p, e).is_some() {
                    let delegate = self.st & S_KIND == S_DELEGATE;
                    let te = self.ty_known(p);
                    self.prev = P_DECLARE;
                    self.st = if delegate { S_DELEGATE } else { S_DECL };
                    return te;
                }
                self.expr_word(p, e, kw, true)
            }
        }
    }

    /// A name at a list's item: a named argument's, a declaration's type, or an expression's.
    fn list_name(&mut self, p: usize, e: usize, kw: u8) -> usize {
        let b = self.b;
        let top = self.top();
        if kw == 0
            && b.get(e) == Some(&b':')
            && b.get(e + 1) != Some(&b':')
            && self.ternary == 0
            && matches!(top, F_ARGS | F_PAREN | F_INIT)
        {
            self.put(p, e, if top == F_ARGS { PARAMETER } else { VAR_OTHER });
            self.put(e, e + 1, ARG_VALUE_SEP);
            self.prev = P_EXPR;
            return e + 1;
        }
        // a list pattern's `var x` too: `Type name` is a declaration in brackets as in parentheses
        if matches!(top, F_ARGS | F_PAREN | F_COND | F_FOR | F_BRACKET)
            && let Some(_) = self.decl_at(p, e)
        {
            let te = self.ty_known(p);
            self.prev = P_DECLARE;
            if matches!(top, F_COND | F_FOR) {
                self.st = (self.st & !S_KIND) | S_DECL;
            }
            return te;
        }
        self.expr_word(p, e, kw, true)
    }

    /// A type's name where one is next (after `new`, `is`, `as`, `typeof(`…): the type, then a
    /// declared name where one follows (`is Foo f`).
    fn type_name(&mut self, p: usize) -> usize {
        let te = self.ty(p);
        let b = self.b;
        let q = blanks(b, te);
        self.prev = if q > te
            && name_start(b, q)
            && !words::not_a_name(words::info(&b[q..name_end(b, q)]))
            && self.st & S_PAT != 0
        {
            P_DECLARE
        } else {
            P_VALUE
        };
        te
    }

    /// A declared name at `p..e`: a member's, a function's, a parameter's, a local's.
    fn declared(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let top = self.top();
        // an explicit interface's name before the member's, `IFoo.Bar`
        if top == F_TYPE && b.get(e) == Some(&b'.') && name_start(b, e + 1) {
            self.put(p, e, INHERITED);
            self.put(e, e + 1, DOT);
            return e + 1;
        }
        let q = blanks(b, e);
        let fun = matches!(b.get(q), Some(b'(' | b'<'));
        let leaf = match top {
            F_PARAMS | F_HPARAMS | F_LPARAMS | F_IPARAMS => PARAMETER,
            _ if self.st & S_KIND == S_DELEGATE => DELEGATE_NAME,
            F_TYPE if fun => FUNC,
            F_TYPE => MEMBER,
            0 | F_NS | F_CODE | F_LAMBDA if fun => FUNC,
            _ => VAR_OTHER,
        };
        self.put_hot(p, e, leaf);
        self.prev = if fun && leaf != PARAMETER && leaf != VAR_OTHER {
            P_FUNC
        } else {
            P_DECLARED
        };
        e
    }

    /// The name an entity's word (`class`…) names.
    fn entity_name(&mut self, p: usize, e: usize) -> usize {
        let leaf = match self.ent {
            E_STRUCT => STRUCT_NAME,
            E_INTERFACE => INTERFACE_NAME,
            E_ENUM => ENUM_NAME,
            _ => CLASS_NAME,
        };
        self.put(p, e, leaf);
        self.prev = P_HEAD;
        self.st = S_HEAD;
        self.pending = if self.ent == E_ENUM {
            PEND_ENUM
        } else {
            PEND_TYPE
        };
        e
    }

    /// A base's name in a base list or a constraint: its path's names inherited classes (plain
    /// to the themes, as bat's), its type arguments types.
    fn base_name(&mut self, p: usize) -> usize {
        let b = self.b;
        let mut q = p;
        loop {
            if !name_start(b, q) {
                break;
            }
            let we = name_end(b, q);
            let i = words::info(&b[q..we]);
            self.put(
                q,
                we,
                if (i >> 8) as u8 == A_TYPE {
                    STORAGE_TYPE
                } else {
                    INHERITED
                },
            );
            q = we;
            if b.get(q) == Some(&b'<') {
                match self.ty_args::<false>(q, 1) {
                    Some(_) => q = self.ty_args::<true>(q, 1).unwrap_or(q + 1),
                    None => break,
                }
            }
            if b.get(q) == Some(&b'.') && name_start(b, q + 1) {
                self.put(q, q + 1, DOT);
                q += 1;
                continue;
            }
            break;
        }
        self.prev = P_VALUE;
        q
    }

    /// A name at a parameter's start: a lambda's untyped parameter, or a parameter's type.
    fn param_name(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let q = blanks(b, e);
        if self.top() == F_LPARAMS && matches!(b.get(q), None | Some(b',' | b')')) {
            self.put(p, e, PARAMETER);
            self.prev = P_DECLARED;
            return e;
        }
        let te = self.ty(p);
        self.prev = P_DECLARE;
        te
    }

    /// A name at a switch arm's or a `case`'s pattern: a declaration's type, `_`, or a
    /// constant's.
    fn arm_name(&mut self, p: usize, e: usize, kw: u8) -> usize {
        if self.decl_at(p, e).is_some() {
            let te = self.ty_known(p);
            self.prev = P_DECLARE;
            return te;
        }
        if kw == 0 && crate::same(&self.b[p..e], b"_") {
            self.put(p, e, DISCARD);
            self.prev = P_VALUE;
            return e;
        }
        self.expr_word(p, e, kw, false)
    }

    /// A type at `p` (where `EMIT`, written): a tuple, or a path of names with their type
    /// arguments, then `?`, `*` and ranks. Where it ends, or `None` where none is there.
    pub(super) fn ty_at<const EMIT: bool>(&mut self, p: usize, depth: u8) -> Option<usize> {
        let b = self.b;
        if depth > 8 {
            return None;
        }
        let mut q;
        if b.get(p) == Some(&b'(') {
            // a tuple's type, two elements at least, each a type and a name
            if EMIT {
                self.put(p, p + 1, GROUP_BEGIN);
            }
            q = blanks(b, p + 1);
            let mut n = 0;
            loop {
                q = self.ty_at::<EMIT>(q, depth + 1)?;
                q = blanks(b, q);
                if name_start(b, q) {
                    let we = name_end(b, q);
                    if words::not_a_name(words::info(&b[q..we])) {
                        return None;
                    }
                    if EMIT {
                        self.put(q, we, VAR_OTHER);
                    }
                    q = blanks(b, we);
                }
                n += 1;
                match b.get(q) {
                    Some(b',') => {
                        if EMIT {
                            self.put(q, q + 1, TUPLE_SEP);
                        }
                        q = blanks(b, q + 1);
                    }
                    Some(b')') if n > 1 => {
                        if EMIT {
                            self.put(q, q + 1, GROUP_END);
                        }
                        q += 1;
                        break;
                    }
                    _ => return None,
                }
            }
        } else {
            q = p;
            let mut first = true;
            loop {
                if !name_start(b, q) {
                    return None;
                }
                let we = name_end(b, q);
                let i = words::info(&b[q..we]);
                let leaf = if i == 0 {
                    SUPPORT_TYPE
                } else {
                    match (i >> 8) as u8 {
                        A_TYPE => STORAGE_TYPE,
                        // `var`, `dynamic` as a declaration's own type; in type arguments a type's name
                        A_VAR if first && depth == 0 => VAR_KW,
                        A_VAR | A_CMOD | A_RECORD | A_ACCESSOR | A_QUERY | A_CONSTRAINT
                        | A_NAMEVALUE | A_AWAIT | A_WITH | A_PATTERN | A_WHERE | A_WHEN => {
                            SUPPORT_TYPE
                        }
                        A_GLOBAL if b.get(we..we + 2) == Some(b"::") => KW_OTHER,
                        _ => return None,
                    }
                };
                if EMIT {
                    self.put(q, we, leaf);
                }
                q = we;
                first = false;
                if b.get(q) == Some(&b'<') {
                    q = self.ty_args::<EMIT>(q, depth + 1)?;
                }
                match b.get(q) {
                    Some(b'.') if name_start(b, q + 1) => {
                        if EMIT {
                            self.put(q, q + 1, DOT_NS);
                        }
                        q += 1;
                    }
                    Some(b':') if b.get(q + 1) == Some(&b':') && name_start(b, q + 2) => {
                        if EMIT {
                            self.put(q, q + 2, DOUBLE_COLON);
                        }
                        q += 2;
                    }
                    _ => break,
                }
            }
        }
        // `?`, `*`, ranks
        loop {
            match b.get(q) {
                Some(b'?') if suffix_ok(b, q + 1) => {
                    if EMIT {
                        self.put(q, q + 1, NULLABLE);
                    }
                    q += 1;
                }
                Some(b'*') if suffix_ok(b, q + 1) => {
                    if EMIT {
                        self.put(q, q + 1, POINTER);
                    }
                    q += 1;
                }
                Some(b'[') => {
                    let mut r = blanks(b, q + 1);
                    while b.get(r) == Some(&b',') {
                        r = blanks(b, r + 1);
                    }
                    if b.get(r) != Some(&b']') {
                        break;
                    }
                    if EMIT {
                        self.put(q, q + 1, BRACKETS_BEGIN);
                        let mut c = q + 1;
                        while c < r {
                            if b[c] == b',' {
                                self.put(c, c + 1, LABEL_SEP);
                            }
                            c += 1;
                        }
                        self.put(r, r + 1, BRACKETS_END);
                    }
                    q = r + 1;
                }
                _ => break,
            }
        }
        Some(q)
    }

    /// Type arguments at `p` (its `<`; where `EMIT`, written): types separated by `,`, or
    /// none (an unbound generic's `<>`, `<,>`). Where they end, past their `>`.
    pub(super) fn ty_args<const EMIT: bool>(&mut self, p: usize, depth: u8) -> Option<usize> {
        let b = self.b;
        if depth > 8 {
            return None;
        }
        if EMIT {
            self.put(p, p + 1, GENERIC_BEGIN);
        }
        let mut r = blanks(b, p + 1);
        if matches!(b.get(r), Some(b'>' | b',')) {
            while b.get(r) == Some(&b',') {
                if EMIT {
                    self.put(r, r + 1, TYPE_SEP);
                }
                r = blanks(b, r + 1);
            }
            if b.get(r) != Some(&b'>') {
                return None;
            }
            if EMIT {
                self.put(r, r + 1, GENERIC_END);
            }
            return Some(r + 1);
        }
        loop {
            r = self.ty_at::<EMIT>(r, depth)?;
            r = blanks(b, r);
            match b.get(r) {
                Some(b',') => {
                    if EMIT {
                        self.put(r, r + 1, TYPE_SEP);
                    }
                    r = blanks(b, r + 1);
                }
                Some(b'>') => {
                    if EMIT {
                        self.put(r, r + 1, GENERIC_END);
                    }
                    return Some(r + 1);
                }
                _ => return None,
            }
        }
    }

    /// A declaration's type parameters at `p` (its `<`): names, `in` and `out`, `,`, to `>`.
    fn type_params(&mut self, p: usize) -> usize {
        let b = self.b;
        let prev = self.prev;
        self.put(p, p + 1, GENERIC_BEGIN);
        let mut q = p + 1;
        while q < b.len() {
            q = blanks(b, q);
            match b.get(q) {
                Some(b'>') => {
                    self.put(q, q + 1, GENERIC_END);
                    self.prev = prev;
                    return q + 1;
                }
                Some(b',') => {
                    self.put(q, q + 1, TYPE_SEP);
                    q += 1;
                }
                Some(b'[') => {
                    q = self.attribute(q);
                    self.prev = prev;
                }
                Some(_) if name_start(b, q) => {
                    let e = name_end(b, q);
                    let w = &b[q..e];
                    let leaf = if crate::same(w, b"in") || crate::same(w, b"out") {
                        MODIFIER
                    } else {
                        SUPPORT_TYPE
                    };
                    self.put(q, e, leaf);
                    q = e;
                }
                _ => return q,
            }
        }
        q
    }

    /// A keyword at `p..e` (`info` its row): its action.
    #[inline(never)]
    fn word(&mut self, p: usize, e: usize, info: u16) -> usize {
        let b = self.b;
        let leaf = info & 0xff;
        let act = (info >> 8) as u8;
        let prev = self.prev;
        let top = self.top();
        match act {
            A_VALUE => {
                self.put(p, e, leaf);
                self.prev = P_VALUE;
                e
            }
            A_LITERAL => {
                self.put(p, e, LANGUAGE);
                self.prev = P_VALUE;
                e
            }
            A_TYPE => {
                if prev == P_DOT {
                    return self.member(p, e);
                }
                self.ident_slow(p, e, A_TYPE)
            }
            A_VAR => {
                match prev {
                    P_DOT => self.member(p, e),
                    // `var (a, b) = …`: a deconstruction
                    P_STMT | P_LIST if b.get(blanks(b, e)) == Some(&b'(') => {
                        self.put(p, e, VAR_KW);
                        self.prev = P_EXPR;
                        e
                    }
                    P_STMT | P_LIST | P_TYPE | P_PARAM | P_ARM => self.ident_slow(p, e, A_VAR),
                    _ => self.ident(p, e),
                }
            }
            A_MOD | A_STATIC | A_CONVERSION => {
                // `unsafe { … }`: a block
                if leaf == MODIFIER
                    && crate::same(&b[p..e], b"unsafe")
                    && b.get(blanks(b, e)) == Some(&b'{')
                {
                    self.put(p, e, KW_OTHER);
                    self.prev = P_STMT;
                    self.pending = PEND_CODE;
                    return e;
                }
                if prev == P_DOT {
                    return self.member(p, e);
                }
                let leaf = if matches!(top, F_PARAMS | F_HPARAMS | F_LPARAMS) && leaf == MODIFIER {
                    PARAM_MOD
                } else {
                    leaf
                };
                self.put(p, e, leaf);
                if !matches!(prev, P_LIST | P_EXPR | P_PARAM | P_ARROW) {
                    self.prev = P_STMT;
                }
                e
            }
            A_CMOD => {
                let q = blanks(b, e);
                let is_mod = matches!(prev, P_STMT | P_LIST | P_EXPR | P_PARAM | P_ARROW)
                    && q > e
                    && (name_start(b, q)
                        || b.get(q) == Some(&b'(') && crate::same(&b[p..e], b"async"));
                if !is_mod {
                    return self.ident(p, e);
                }
                self.put(
                    p,
                    e,
                    if matches!(top, F_PARAMS | F_HPARAMS | F_LPARAMS) {
                        PARAM_MOD
                    } else {
                        MODIFIER
                    },
                );
                if !matches!(prev, P_LIST | P_EXPR | P_PARAM | P_ARROW) {
                    self.prev = P_STMT;
                }
                e
            }
            A_ENTITY => {
                let ent = words::entity(leaf);
                match ent {
                    E_EVENT => {
                        self.put(p, e, MODIFIER);
                        self.prev = P_STMT;
                        e
                    }
                    E_NAMESPACE => self.namespace(p, e),
                    // `where T : class`, `struct`
                    _ if prev == P_BASE => {
                        self.put(p, e, STORAGE_TYPE);
                        self.prev = P_VALUE;
                        e
                    }
                    // `record class`, `record struct`
                    E_CLASS | E_STRUCT if prev == P_ENTITY && self.ent == E_RECORD => {
                        self.put(p, e, leaf);
                        e
                    }
                    _ if prev == P_DOT => self.member(p, e),
                    _ => {
                        self.put(p, e, leaf);
                        self.prev = P_ENTITY;
                        self.ent = ent;
                        e
                    }
                }
            }
            A_RECORD => {
                let q = blanks(b, e);
                if prev == P_STMT && q > e && name_start(b, q) {
                    self.put(p, e, CLASS_KW);
                    self.prev = P_ENTITY;
                    self.ent = E_RECORD;
                    return e;
                }
                self.ident(p, e)
            }
            A_DELEGATE => {
                if prev == P_STMT {
                    self.put(p, e, DELEGATE_KW);
                    self.st = S_DELEGATE;
                    return e;
                }
                self.put(p, e, KW_OTHER);
                self.prev = P_ANON;
                e
            }
            A_COND => {
                let w = &b[p..e];
                if crate::same(w, b"switch") && prev == P_VALUE {
                    self.put(p, e, KW_OTHER);
                    self.prev = P_SWITCH;
                    return e;
                }
                if prev == P_DOT {
                    return self.member(p, e);
                }
                self.put(p, e, leaf);
                self.prev = if crate::same(w, b"catch") {
                    P_CATCH
                } else {
                    P_COND
                };
                e
            }
            A_FOR => {
                self.put(p, e, leaf);
                self.prev = P_FOR;
                e
            }
            A_BLOCK => {
                if crate::same(&b[p..e], b"else") {
                    let q = blanks(b, e);
                    if b.get(q..q + 2) == Some(b"if")
                        && !b.get(q + 2).is_some_and(|&c| NAME[usize::from(c)])
                    {
                        self.put(p, q + 2, ELSEIF_KW);
                        self.prev = P_COND;
                        return q + 2;
                    }
                    self.put(p, q, ELSE_KW);
                    self.prev = P_STMT;
                    return q;
                }
                self.put(p, e, leaf);
                self.prev = P_STMT;
                e
            }
            A_CHECKED => {
                self.put(p, e, leaf);
                if b.get(blanks(b, e)) == Some(&b'{') {
                    self.prev = P_STMT;
                    self.pending = PEND_CODE;
                } else {
                    self.prev = P_EXPR;
                }
                e
            }
            A_EXPR | A_JUMP => {
                self.put(p, e, leaf);
                self.prev = P_EXPR;
                e
            }
            A_GOTO => {
                self.put(p, e, leaf);
                self.prev = P_GOTO;
                e
            }
            A_YIELD => {
                let q = blanks(b, e);
                if q > e && name_start(b, q) {
                    let we = name_end(b, q);
                    let w = &b[q..we];
                    if crate::same(w, b"return") || crate::same(w, b"break") {
                        self.put(
                            p,
                            we,
                            if crate::same(w, b"return") {
                                RETURN_KW
                            } else {
                                BREAK_KW
                            },
                        );
                        self.prev = P_EXPR;
                        return we;
                    }
                }
                self.ident(p, e)
            }
            A_CASE => {
                self.put(p, e, leaf);
                if prev == P_GOTO {
                    self.prev = P_EXPR;
                } else {
                    self.st |= S_CASE | S_PAT;
                    self.ternary = 0;
                    self.prev = P_ARM;
                }
                e
            }
            A_DEFAULT => {
                let q = blanks(b, e);
                match b.get(q) {
                    Some(b':') if b.get(q + 1) != Some(&b':') && prev != P_EXPR => {
                        self.put(p, e, CASE_KW);
                        self.put(q, q + 1, CASE_SEP);
                        self.prev = P_STMT;
                        q + 1
                    }
                    Some(b'(') => {
                        self.put(p, e, REFLECTION);
                        self.prev = P_TYPEOF;
                        e
                    }
                    _ => {
                        self.put(p, e, if prev == P_GOTO { CASE_KW } else { KW_OTHER });
                        self.prev = P_VALUE;
                        e
                    }
                }
            }
            A_WHEN => {
                if self.st & S_CASE != 0 || top == F_SWITCH {
                    self.put(p, e, WHEN_KW);
                    self.st &= !S_PAT;
                    self.prev = P_EXPR;
                    return e;
                }
                if prev == P_STMT && b.get(blanks(b, e)) == Some(&b'(') {
                    self.put(p, e, TRY_WHEN);
                    self.prev = P_COND;
                    return e;
                }
                self.ident(p, e)
            }
            A_THIS => {
                if prev == P_PARAM {
                    self.put(p, e, PARAM_MOD);
                    return e;
                }
                if prev == P_DECLARE && top == F_TYPE {
                    // an indexer's `this[`
                    self.put(p, e, VAR_LANGUAGE);
                    self.prev = P_FUNC;
                    return e;
                }
                self.put_hot(p, e, VAR_LANGUAGE);
                self.prev = P_VALUE;
                e
            }
            A_NAMEVALUE => {
                if matches!(prev, P_DECLARE | P_DOT | P_PARAM | P_TYPE | P_STMT | P_LIST) {
                    return self.ident(p, e);
                }
                self.put(p, e, VAR_LANGUAGE);
                self.prev = P_VALUE;
                e
            }
            A_NEW => {
                if prev == P_STMT && top == F_TYPE {
                    self.put(p, e, MODIFIER);
                    return e;
                }
                if prev == P_BASE && b.get(e..e + 2) == Some(b"()") {
                    self.put(p, e + 2, NEW_OP);
                    self.prev = P_VALUE;
                    return e + 2;
                }
                self.put(p, e, NEW_OP);
                self.prev = P_NEW;
                e
            }
            A_IS => {
                if prev == P_DOT {
                    return self.member(p, e);
                }
                self.put(p, e, leaf);
                self.prev = P_TYPE;
                if crate::same(&b[p..e], b"is") {
                    self.st |= S_PAT;
                }
                e
            }
            A_TYPEOF => {
                self.put(p, e, leaf);
                self.prev = P_TYPEOF;
                e
            }
            A_NAMEOF => {
                self.put(p, e, leaf);
                let q = blanks(b, e);
                // `nameof(Type.Member)`: a path read as a type's, as bat's; `nameof(local)` a variable
                self.prev = if b.get(q) == Some(&b'(')
                    && b.get(blanks(b, q + 1)).is_some_and(u8::is_ascii_uppercase)
                {
                    P_TYPEOF
                } else {
                    P_EXPR
                };
                e
            }
            A_AWAIT => {
                if matches!(prev, P_DECLARE | P_DOT) {
                    return self.ident(p, e);
                }
                self.put(p, e, leaf);
                if prev != P_STMT {
                    self.prev = P_EXPR;
                }
                e
            }
            A_STACKALLOC => {
                self.put(p, e, leaf);
                self.prev = P_NEW;
                e
            }
            A_ACCESSOR => {
                if top == F_ACCESSORS && prev == P_STMT {
                    self.put(p, e, leaf);
                    self.prev = P_ACCESSOR;
                    return e;
                }
                self.ident(p, e)
            }
            A_WHERE => {
                if self.st & S_QUERY != 0 && prev != P_DOT {
                    self.put(p, e, SQL);
                    self.prev = P_EXPR;
                    return e;
                }
                if matches!(self.st & S_KIND, S_HEAD | S_BASE | S_WHERE)
                    || matches!(prev, P_PARAMS_DONE | P_HEAD)
                {
                    self.put(p, e, MODIFIER);
                    self.st = (self.st & !S_KIND) | S_WHERE;
                    self.prev = P_TYPE;
                    return e;
                }
                self.ident(p, e)
            }
            A_CONSTRAINT => {
                if prev == P_BASE {
                    self.put(p, e, STORAGE_TYPE);
                    self.prev = P_VALUE;
                    return e;
                }
                self.ident(p, e)
            }
            A_QUERY => {
                if prev != P_DOT && prev != P_DECLARE {
                    if self.st & S_QUERY != 0 {
                        self.put(p, e, SQL);
                        self.prev = P_EXPR;
                        return e;
                    }
                    if crate::same(&b[p..e], b"from")
                        && matches!(prev, P_EXPR | P_LIST | P_ARROW)
                        && name_start(b, blanks(b, e))
                        && blanks(b, e) > e
                    {
                        self.put(p, e, SQL);
                        self.st |= S_QUERY;
                        self.prev = P_EXPR;
                        return e;
                    }
                }
                self.ident(p, e)
            }
            A_PARAM => {
                let w = &b[p..e];
                if crate::same(w, b"in") && self.st & S_QUERY != 0 {
                    self.put(p, e, SQL);
                    self.prev = P_EXPR;
                    return e;
                }
                if matches!(top, F_PARAMS | F_HPARAMS | F_LPARAMS | F_IPARAMS) {
                    self.put(p, e, PARAM_MOD);
                    self.prev = P_PARAM;
                    return e;
                }
                if crate::same(w, b"in") && top == F_COND {
                    self.put(p, e, IN_KW);
                    self.prev = P_EXPR;
                    return e;
                }
                if prev == P_LIST {
                    self.put(p, e, ARG_MOD);
                    return e;
                }
                self.put(p, e, MODIFIER);
                if !matches!(prev, P_STMT | P_EXPR | P_ARROW) {
                    self.prev = P_EXPR;
                }
                e
            }
            A_OPERATOR => {
                self.put(p, e, leaf);
                self.operator_decl(e)
            }
            A_PATTERN => {
                if self.st & S_PAT != 0 && prev != P_DOT && prev != P_DECLARE {
                    self.put(p, e, leaf);
                    self.prev = P_TYPE;
                    return e;
                }
                self.ident(p, e)
            }
            A_WITH => {
                if prev == P_VALUE {
                    self.put(p, e, leaf);
                    self.prev = P_EXPR;
                    self.pending = PEND_INIT;
                    return e;
                }
                self.ident(p, e)
            }
            A_USING => {
                let q = blanks(b, e);
                if prev == P_STMT && matches!(top, 0 | F_NS) {
                    let nw = if name_start(b, q) {
                        &b[q..name_end(b, q)]
                    } else {
                        b""
                    };
                    if b.get(q) != Some(&b'(') && nw != b"var" {
                        return self.using_directive(p, e);
                    }
                }
                self.put(p, e, leaf);
                self.prev = if b.get(q) == Some(&b'(') {
                    P_COND
                } else {
                    P_STMT
                };
                e
            }
            A_GLOBAL => {
                let q = blanks(b, e);
                if b.get(e..e + 2) == Some(b"::") {
                    self.put(p, e, leaf);
                    self.prev = P_VALUE;
                    return e;
                }
                if prev == P_STMT && b.get(q..q + 5) == Some(b"using") {
                    self.put(p, e, IMPORT);
                    return e;
                }
                self.ident(p, e)
            }
            _ => {
                self.put(p, e, leaf);
                self.prev = P_VALUE;
                e
            }
        }
    }

    /// `namespace` at `p..e` and its name: the names, the dots between them.
    fn namespace(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        self.put(p, e, NAMESPACE_KW);
        let mut q = blanks(b, e);
        while name_start(b, q) {
            let we = name_end(b, q);
            self.put(q, we, NAMESPACE_NAME);
            q = we;
            if b.get(q) == Some(&b'.') && name_start(b, q + 1) {
                self.put(q, q + 1, NS_NAME_SEP);
                q += 1;
            } else {
                break;
            }
        }
        self.pending = PEND_NS;
        self.prev = P_HEAD;
        self.st = S_NONE;
        q
    }

    /// A `using` directive at `p..e`: `static`, an alias's `name =`, the namespace's path
    /// (plain, as bat's), its type arguments.
    fn using_directive(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        self.put(p, e, IMPORT);
        let mut q = blanks(b, e);
        if b.get(q..q + 6) == Some(b"static")
            && !b.get(q + 6).is_some_and(|&c| NAME[usize::from(c)])
        {
            self.put(q, q + 6, IMPORT);
            q = blanks(b, q + 6);
        }
        // an alias
        if name_start(b, q) {
            let we = name_end(b, q);
            let r = blanks(b, we);
            if b.get(r) == Some(&b'=') && b.get(r + 1) != Some(&b'=') {
                self.put(r, r + 1, ASSIGN);
                q = blanks(b, r + 1);
            }
        }
        while name_start(b, q) {
            let we = name_end(b, q);
            q = we;
            if b.get(q) == Some(&b'<') {
                match self.ty_args::<false>(q, 1) {
                    Some(_) => q = self.ty_args::<true>(q, 1).unwrap_or(q + 1),
                    None => break,
                }
            }
            match b.get(q) {
                Some(b'.') if name_start(b, q + 1) => {
                    self.put(q, q + 1, NS_SEP);
                    q += 1;
                }
                Some(b':') if b.get(q + 1) == Some(&b':') => {
                    self.put(q, q + 2, DOUBLE_COLON);
                    q += 2;
                }
                _ => break,
            }
        }
        // a type's rest (`?`, `[]`) after an alias's path
        self.prev = P_VALUE;
        q
    }

    /// The operator `operator` declares, from `e` (past the word): an operator's bytes, `true`
    /// or `false`, a conversion's type; its parameters' `(` next.
    fn operator_decl(&mut self, e: usize) -> usize {
        let b = self.b;
        let q = blanks(b, e);
        self.prev = P_FUNC;
        match b.get(q) {
            Some(&c) if c != b'(' && ACT[usize::from(c)] == B_OP => {
                let mut r = q + 1;
                while r < b.len() && ACT[usize::from(b[r])] == B_OP {
                    r += 1;
                }
                self.put(q, r, FUNC);
                r
            }
            Some(_) if name_start(b, q) => {
                let we = name_end(b, q);
                let w = &b[q..we];
                if crate::same(w, b"true") || crate::same(w, b"false") {
                    self.put(q, we, FUNC);
                    return we;
                }
                // `checked` before the operator (C# 11)
                if crate::same(w, b"checked") {
                    self.put(q, we, KW_OTHER);
                    return self.operator_decl(we);
                }
                self.ty(q)
            }
            _ => q,
        }
    }

    /// `(` at `p`: what it opens by the token before.
    #[inline(always)]
    pub(super) fn lparen(&mut self, p: usize) -> usize {
        match self.prev {
            P_VALUE | P_NEW => {
                self.push(F_ARGS);
                self.put_hot(p, p + 1, GROUP_BEGIN);
                self.prev = P_LIST;
                p + 1
            }
            _ => self.lparen_slow(p),
        }
    }

    #[inline(never)]
    fn lparen_slow(&mut self, p: usize) -> usize {
        let (frame, leaf, next) = match self.prev {
            P_COND => (F_COND, GROUP_BEGIN, P_LIST),
            P_CATCH => (F_COND, GROUP_BEGIN, P_TYPE),
            P_FOR => {
                self.st = S_NONE;
                (F_FOR, GROUP_BEGIN, P_LIST)
            }
            P_TYPEOF => (F_PAREN, GROUP_BEGIN, P_TYPE),
            P_FUNC => (F_PARAMS, PARAMS_BEGIN, P_PARAM),
            P_HEAD => (F_HPARAMS, PARAMS_BEGIN, P_PARAM),
            P_ANON => (F_LPARAMS, GROUP_BEGIN, P_PARAM),
            P_DOT | P_DECLARED | P_GOTO | P_ACCESSOR | P_PARAMS_DONE | P_LAMBDA => {
                (F_ARGS, GROUP_BEGIN, P_LIST)
            }
            _ => {
                // an expression's start: a group, a cast, a lambda's parameters, a tuple
                let mark = self.k.mark(self.out);
                self.push(F_PAREN);
                self.put(p, p + 1, GROUP_BEGIN);
                if usize::from(self.ng) == GROUPS {
                    self.groups.copy_within(1.., 0);
                    self.ng -= 1;
                }
                self.groups[usize::from(self.ng)] = MaybeUninit::new(Group {
                    mark,
                    open: p as u32,
                    depth: self.depth,
                });
                self.ng += 1;
                self.prev = P_LIST;
                return p + 1;
            }
        };
        self.push(frame);
        self.put(p, p + 1, leaf);
        self.prev = next;
        p + 1
    }

    /// The innermost marked group (one is: [`Scan::ng`] is past 0).
    #[inline(always)]
    fn group_top(&self) -> &Group {
        debug_assert!(self.ng > 0);
        // SAFETY: the first `ng` groups are set ([`Scan::lparen_slow`])
        unsafe { self.groups[usize::from(self.ng) - 1].assume_init_ref() }
    }

    /// `)` at `p`: the innermost parenthesis closed, by what it opened.
    #[inline(always)]
    pub(super) fn rparen(&mut self, p: usize) -> usize {
        match self.top() {
            F_ARGS => {
                self.pop();
                self.put_hot(p, p + 1, GROUP_END);
                self.prev = P_VALUE;
                p + 1
            }
            F_PAREN if self.ng == 0 || self.group_top().depth != self.depth => {
                self.pop();
                self.put_hot(p, p + 1, GROUP_END);
                self.prev = P_VALUE;
                p + 1
            }
            _ => self.rparen_slow(p),
        }
    }

    #[inline(never)]
    fn rparen_slow(&mut self, p: usize) -> usize {
        let top = self.top();
        match top {
            F_PAREN => {
                let g = *self.group_top();
                self.ng -= 1;
                self.pop();
                if let Some(q) = self.group_close(g, p) {
                    return q;
                }
                self.put(p, p + 1, GROUP_END);
                self.prev = P_VALUE;
            }
            F_COND | F_FOR => {
                self.pop();
                self.put(p, p + 1, GROUP_END);
                self.prev = P_STMT;
                self.st &= !(S_KIND | S_EXPR | S_PAT);
                self.ternary = 0;
            }
            F_PARAMS => {
                self.pop();
                self.put(p, p + 1, PARAMS_END);
                self.prev = P_PARAMS_DONE;
            }
            F_HPARAMS => {
                self.pop();
                self.put(p, p + 1, PARAMS_END);
                self.prev = P_HEAD;
            }
            F_LPARAMS => {
                self.pop();
                self.put(p, p + 1, GROUP_END);
                self.prev = P_LAMBDA;
            }
            _ => {
                // a `)` with none open: plain
                self.prev = P_VALUE;
            }
        }
        p + 1
    }

    /// A marked group `g` closed at `p` (its frame popped): read again as a lambda's
    /// parameters before `=>`, or as a cast's type before an operand. Where the scan goes on,
    /// or `None` where it is a group.
    fn group_close(&mut self, g: Group, p: usize) -> Option<usize> {
        let b = self.b;
        let q = blanks(b, p + 1);
        let open = g.open as usize;
        if b.get(q..q + 2) == Some(b"=>") {
            self.k.undo(self.out, g.mark);
            self.push(F_LPARAMS);
            self.put(open, open + 1, GROUP_BEGIN);
            self.prev = P_PARAM;
            return Some(open + 1);
        }
        if cast_follows(b, q) {
            let s = blanks(b, open + 1);
            if let Some(te) = self.ty_at::<false>(s, 0)
                && blanks(b, te) == p
            {
                self.k.undo(self.out, g.mark);
                self.put(open, open + 1, GROUP_BEGIN);
                let _ = self.ty_at::<true>(s, 0);
                self.put(p, p + 1, GROUP_END);
                self.prev = P_EXPR;
                return Some(p + 1);
            }
        }
        None
    }

    /// `{` at `p`: what it opens, by what a word said or the token before.
    pub(super) fn lbrace(&mut self, p: usize) -> usize {
        let f = match self.pending {
            PEND_NS => F_NS,
            PEND_TYPE => F_TYPE,
            PEND_ENUM => F_ENUM,
            PEND_CODE => F_CODE,
            PEND_LAMBDA => F_LAMBDA,
            PEND_INIT => F_INIT,
            PEND_SWITCH => F_SWITCH,
            _ => match self.prev {
                P_SWITCH => F_SWITCH,
                P_ARROW | P_LAMBDA | P_ANON => F_LAMBDA,
                P_DECLARED if self.top() == F_TYPE => F_ACCESSORS,
                P_STMT | P_PARAMS_DONE | P_ACCESSOR | P_CATCH | P_COND | P_HEAD => F_CODE,
                _ if matches!(self.st & S_KIND, S_WHERE | S_PREBODY) => F_CODE,
                _ => F_INIT,
            },
        };
        self.pending = PEND_NONE;
        self.push(f);
        match f {
            F_INIT => {
                self.put_hot(p, p + 1, BRACES_BEGIN);
                self.prev = P_LIST;
            }
            F_SWITCH => {
                self.put(p, p + 1, BLOCK_BEGIN);
                self.prev = P_ARM;
                self.st |= S_PAT;
            }
            _ => {
                self.put_hot(p, p + 1, BLOCK_BEGIN);
                self.prev = P_STMT;
                self.st = S_NONE;
                self.ternary = 0;
            }
        }
        p + 1
    }

    /// `}` at `p`: an interpolation's end, or the innermost brace's (the parentheses and
    /// brackets left open in it dropped).
    pub(super) fn rbrace(&mut self, p: usize) -> usize {
        while matches!(
            self.top(),
            F_ARGS
                | F_PAREN
                | F_COND
                | F_FOR
                | F_PARAMS
                | F_HPARAMS
                | F_LPARAMS
                | F_IPARAMS
                | F_BRACKET
                | F_ATTR
                | F_ATTR_ARGS
        ) {
            self.pop();
        }
        let f = self.top();
        if matches!(f, F_INTERP | F_VINTERP | F_RINTERP) {
            return self.interp_close(p);
        }
        self.pop();
        match f {
            F_INIT => {
                self.put_hot(p, p + 1, BRACES_END);
                self.prev = P_VALUE;
            }
            F_LAMBDA | F_SWITCH => {
                self.put(p, p + 1, BLOCK_END);
                self.prev = P_VALUE;
                self.st = S_EXPR;
            }
            _ => {
                self.put_hot(p, p + 1, BLOCK_END);
                self.prev = P_STMT;
                self.st = S_NONE;
                self.ternary = 0;
            }
        }
        p + 1
    }

    /// `[` at `p`: an attribute's list at a declaration's start, an indexer's parameters, else
    /// an index's, a collection's or a list pattern's brackets.
    pub(super) fn lbrack(&mut self, p: usize) -> usize {
        match self.prev {
            P_STMT | P_PARAM if !matches!(self.top(), F_INIT | F_ARGS | F_PAREN | F_BRACKET) => {
                self.attribute(p)
            }
            P_FUNC => {
                self.push(F_IPARAMS);
                self.put(p, p + 1, BRACKETS_BEGIN);
                self.prev = P_PARAM;
                p + 1
            }
            _ => {
                self.push(F_BRACKET);
                self.put_hot(p, p + 1, BRACKETS_BEGIN);
                self.prev = P_LIST;
                p + 1
            }
        }
    }

    /// `]` at `p`.
    pub(super) fn rbrack(&mut self, p: usize) -> usize {
        match self.top() {
            F_BRACKET => {
                self.pop();
                self.put_hot(p, p + 1, BRACKETS_END);
                self.prev = P_VALUE;
            }
            F_IPARAMS => {
                self.pop();
                self.put(p, p + 1, BRACKETS_END);
                self.prev = P_DECLARED;
            }
            _ => self.prev = P_VALUE,
        }
        p + 1
    }

    /// `;` at `p`: a statement's end (a `for`'s clause's), the frames it left open dropped.
    #[inline(always)]
    pub(super) fn semi(&mut self, p: usize) -> usize {
        self.put_hot(p, p + 1, TERMINATOR);
        if self.top() == F_FOR {
            self.prev = P_LIST;
            self.st = S_NONE;
            return p + 1;
        }
        while matches!(
            self.top(),
            F_ARGS
                | F_PAREN
                | F_COND
                | F_PARAMS
                | F_HPARAMS
                | F_LPARAMS
                | F_IPARAMS
                | F_BRACKET
                | F_ATTR
                | F_ATTR_ARGS
        ) {
            self.pop();
        }
        // no `;` in an initializer's braces: they were a block's
        if self.top() == F_INIT {
            self.pop();
            self.push(F_CODE);
        }
        self.prev = P_STMT;
        self.st = S_NONE;
        self.pending = PEND_NONE;
        self.ternary = 0;
        p + 1
    }

    /// `,` at `p`, by the frame and the statement it is in.
    pub(super) fn comma(&mut self, p: usize) -> usize {
        let (leaf, next) = match self.top() {
            F_ARGS | F_PAREN | F_COND | F_BRACKET => (ARG_SEP, P_LIST),
            F_PARAMS | F_HPARAMS | F_LPARAMS | F_IPARAMS => (PARAM_SEP, P_PARAM),
            F_ENUM => (ENUM_SEP, P_STMT),
            F_INIT => (ARRAY_SEP, P_LIST),
            F_SWITCH => {
                self.st |= S_PAT;
                (ARRAY_SEP, P_ARM)
            }
            F_INTERP | F_VINTERP | F_RINTERP => return self.interp_align(p),
            F_FOR if self.st & S_KIND == S_DECL => (VARS_SEP, P_DECLARE),
            F_FOR => (ARG_SEP, P_LIST),
            _ => match self.st & S_KIND {
                S_DECL | S_DELEGATE => {
                    self.st &= !S_EXPR;
                    (VARS_SEP, P_DECLARE)
                }
                S_BASE | S_WHERE | S_HEAD => (INHERITED_SEP, P_BASE),
                _ => (ARG_SEP, P_EXPR),
            },
        };
        self.put_hot(p, p + 1, leaf);
        self.prev = next;
        p + 1
    }

    /// `.` at `p`: a member's accessor, a range's `..`, a number's start.
    #[inline(always)]
    pub(super) fn dot(&mut self, p: usize) -> usize {
        let b = self.b;
        match b.get(p + 1) {
            Some(b'.') => {
                self.put(p, p + 2, OPERATOR);
                // a slice pattern's `.. var rest` in a list pattern: a declaration may follow
                self.prev = if self.top() == F_BRACKET {
                    P_LIST
                } else {
                    P_EXPR
                };
                p + 2
            }
            Some(b'0'..=b'9') if self.prev != P_VALUE => self.number(p),
            _ => {
                self.put_hot(p, p + 1, DOT);
                self.prev = P_DOT;
                p + 1
            }
        }
    }

    /// `:` at `p`, by the statement it is in.
    pub(super) fn colon(&mut self, p: usize) -> usize {
        let b = self.b;
        if b.get(p + 1) == Some(&b':') {
            self.put(p, p + 2, DOUBLE_COLON);
            self.prev = P_DOT;
            return p + 2;
        }
        let top = self.top();
        if matches!(top, F_INTERP | F_VINTERP | F_RINTERP) {
            return self.format_spec(p);
        }
        let (leaf, next) = if self.ternary > 0 {
            self.ternary -= 1;
            (TERNARY, P_EXPR)
        } else if self.st & S_CASE != 0 {
            self.st &= !(S_CASE | S_PAT);
            (CASE_SEP, P_STMT)
        } else {
            match self.st & S_KIND {
                S_HEAD if self.ent == E_ENUM => (ENUM_TYPE_SEP, P_TYPE),
                S_HEAD => {
                    self.st = (self.st & !S_KIND) | S_BASE;
                    (TYPE_SEP, P_BASE)
                }
                S_WHERE => (TYPE_SEP, P_BASE),
                _ if self.prev == P_PARAMS_DONE && top == F_TYPE => {
                    self.st = (self.st & !S_KIND) | S_PREBODY;
                    (PREBODY_SEP, P_EXPR)
                }
                _ => (LABEL_SEP, P_EXPR),
            }
        };
        self.put(p, p + 1, leaf);
        self.prev = next;
        p + 1
    }

    /// `?` at `p`: `??`, `??=`, `?.`, `?[`, or a conditional's.
    pub(super) fn quest(&mut self, p: usize) -> usize {
        let b = self.b;
        match b.get(p + 1) {
            Some(b'?') => {
                let e = if b.get(p + 2) == Some(&b'=') {
                    p + 3
                } else {
                    p + 2
                };
                self.put(p, e, OPERATOR);
                self.st &= !S_PAT;
                self.prev = P_EXPR;
                e
            }
            Some(b'.') if !b.get(p + 2).is_some_and(u8::is_ascii_digit) => {
                self.put(p, p + 2, NULL_DOT);
                self.prev = P_DOT;
                p + 2
            }
            Some(b'[') => {
                self.put(p, p + 1, NULL_DOT);
                self.prev = P_DOT;
                p + 1
            }
            _ => {
                self.put(p, p + 1, TERNARY);
                self.ternary = self.ternary.saturating_add(1);
                self.st &= !S_PAT;
                self.prev = P_EXPR;
                p + 1
            }
        }
    }

    /// An operator at `p`.
    #[inline(always)]
    pub(super) fn op(&mut self, p: usize) -> usize {
        let b = self.b;
        let c = b[p];
        let c1 = b.get(p + 1).copied().unwrap_or(0);
        let c2 = b.get(p + 2).copied().unwrap_or(0);
        let (e, leaf, next) = match c {
            b'=' => match c1 {
                b'=' => (p + 2, OPERATOR, P_EXPR),
                b'>' => return self.arrow(p),
                _ => return self.assign(p),
            },
            b'!' => match c1 {
                b'=' => (p + 2, OPERATOR, P_EXPR),
                _ if self.prev == P_VALUE => (p + 1, OPERATOR, P_VALUE),
                _ => (p + 1, OPERATOR, P_EXPR),
            },
            b'<' if matches!(self.prev, P_FUNC | P_HEAD) => return self.type_params(p),
            b'<' => match (c1, c2) {
                (b'<', b'=') => (p + 3, OPERATOR, P_EXPR),
                (b'<' | b'=', _) => (p + 2, OPERATOR, P_EXPR),
                _ => (p + 1, OPERATOR, P_EXPR),
            },
            b'>' => {
                let mut e = p + 1;
                while e < p + 3 && b.get(e) == Some(&b'>') {
                    e += 1;
                }
                if b.get(e) == Some(&b'=') {
                    e += 1;
                }
                (e, OPERATOR, P_EXPR)
            }
            b'+' | b'-' => match c1 {
                _ if c1 == c => {
                    let next = if self.prev == P_VALUE {
                        P_VALUE
                    } else {
                        P_EXPR
                    };
                    (p + 2, OPERATOR, next)
                }
                b'=' => (p + 2, OPERATOR, P_EXPR),
                b'>' if c == b'-' => (p + 2, ARROW, P_DOT),
                _ => (p + 1, OPERATOR, P_EXPR),
            },
            b'&' | b'|' => match c1 {
                _ if c1 == c => {
                    self.st &= !S_PAT;
                    (p + 2, OPERATOR, P_EXPR)
                }
                b'=' => (p + 2, OPERATOR, P_EXPR),
                _ => (p + 1, OPERATOR, P_EXPR),
            },
            b'~' if self.prev == P_STMT && self.top() == F_TYPE && name_start(b, p + 1) => {
                let e = name_end(b, p + 1);
                self.put(p, e, DTOR);
                self.prev = P_FUNC;
                return e;
            }
            _ => match c1 {
                b'=' => (p + 2, OPERATOR, P_EXPR),
                _ => (p + 1, OPERATOR, P_EXPR),
            },
        };
        self.put_hot(p, e, leaf);
        self.prev = next;
        e
    }

    /// `=` at `p`: an assignment's, a declarator's initializer's, a parameter's default value's.
    fn assign(&mut self, p: usize) -> usize {
        let leaf = if self.prev == P_DECLARED {
            match self.top() {
                F_PARAMS | F_HPARAMS | F_LPARAMS | F_IPARAMS => DEFAULT_VALUE,
                F_FOR => ASSIGN_LOOP,
                F_ENUM | F_INIT => ASSIGN,
                _ => ASSIGN_VAR,
            }
        } else {
            ASSIGN
        };
        self.put_hot(p, p + 1, leaf);
        self.prev = P_EXPR;
        self.st = (self.st | S_EXPR) & !S_PAT;
        p + 1
    }

    /// `=>` at `p`: a lambda's, a member's expression body's, a switch arm's.
    fn arrow(&mut self, p: usize) -> usize {
        let leaf = if self.prev == P_LAMBDA {
            LAMBDA
        } else if self.top() == F_SWITCH {
            OPERATOR
        } else if matches!(self.prev, P_PARAMS_DONE | P_DECLARED | P_ACCESSOR) {
            FUNC_ARROW
        } else {
            LAMBDA
        };
        self.put(p, p + 2, leaf);
        self.prev = P_ARROW;
        self.st = (self.st | S_EXPR) & !S_PAT;
        p + 2
    }
}
