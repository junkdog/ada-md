//! The type frames of TypeScript's own scanner ([`super::Scan`]): an annotation after its `:`,
//! a type after `as`, an arrow's return type, `type X = …`, and a type's `(`, `[`, `{` and `<`.
//! A type frame is left at the first byte no type takes at its level, which the frame below
//! then reads. Its few lookaheads ([`type_args_end`], [`arrow_return`], [`fn_type_params`],
//! [`index_signature`]) run only at the rare tokens whose meaning hangs on what follows: a `<`
//! right after a name, a `:` after a group's `)`, a `(` in a type, a `[` at a member's place.

use super::*;

impl<K: Sink> Scan<'_, '_, K> {
    /// The type frames' turns from `p` (`top` the first's): their tokens, until a frame below
    /// them is on top again, a text frame is, or the line ends.
    #[inline(never)]
    pub(super) fn type_turn(&mut self, mut p: usize, mut top: u8) -> usize {
        let n = self.b.len();
        loop {
            p = self.type_token(p, top);
            top = self.cur;
            if p >= n || !(F_TYPE..TEXT).contains(&top) {
                return p;
            }
        }
    }

    /// A type frame's turn at `p` (`top`): a type's token, or the frame left at a byte no type
    /// takes there.
    #[inline(always)]
    fn type_token(&mut self, p: usize, top: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let p = skip_blanks(b, p);
        if p >= n {
            return n;
        }
        let c = b[p];
        let nested = top >= F_T_ROUND;
        let leave = |s: &mut Self| {
            if top == F_T_SUBST {
                // a template literal type's substitution ends only at its `}`: left here, the
                // template's text would read again what its tokens wrote
                return p + char_len(b[p]);
            }
            s.pop();
            p
        };
        match ACT[usize::from(c)] {
            A_NAME | A_HIGH => self.type_name(p, top),
            A_QUOTE => {
                let q = self.string(p, c);
                self.prev = P_VALUE;
                q
            }
            A_DIGIT => self.number(p),
            A_TICK => self.template(p),
            A_SIGN => {
                if b.get(p + 1).is_some_and(u8::is_ascii_digit) {
                    self.tok_far(p, p + 1, ARITHMETIC);
                    return self.number(p + 1);
                }
                if matches!(b.get(p + 1), Some(b'?' | b'r')) && top == F_T_CURLY {
                    // a mapped type's `-?`, `+readonly`
                    return self.tok_far(p, p + 1, TYPE_MODIFIER_OP);
                }
                if nested { p + 1 } else { leave(self) }
            }
            A_PAREN => {
                if skip_blanks(b, p + 1) >= b.len() {
                    self.push(F_T_ROUND_LINE);
                    self.prev = P_OP;
                    return p + 1;
                }
                if fn_type_params(b, p + 1) {
                    return self.open_params(p, F_PARAMS);
                }
                self.push(F_T_ROUND);
                self.prev = P_OP;
                p + 1
            }
            A_BRACKET => {
                if top == F_T_CURLY && self.prev == P_KEY {
                    self.push(F_T_SQUARE);
                    self.prev = if index_signature(b, p + 1) {
                        P_INDEX
                    } else {
                        P_OP
                    };
                    return p + 1;
                }
                if top == F_TYPE_AS && self.prev == P_VALUE && b.get(p + 1) == Some(&b']') {
                    // `as T[]`: an array's type
                    return p + 2;
                }
                if top != F_TYPE_AS && self.prev == P_VALUE || self.prev != P_VALUE {
                    self.push(F_T_SQUARE);
                    self.prev = P_OP;
                    return p + 1;
                }
                leave(self)
            }
            A_CURLY => {
                if matches!(top, F_TYPE | F_TYPE_AS | F_TYPE_RET) && self.prev == P_VALUE {
                    // a body's `{`: the type ended
                    return leave(self);
                }
                self.push(F_T_CURLY);
                self.prev = P_KEY;
                self.tok_far(p, p + 1, BLOCK_BRACE)
            }
            A_LT => {
                self.push(F_T_ANGLE);
                self.prev = P_OP;
                self.tok_far(p, p + 1, TP_BEGIN)
            }
            A_OP => match c {
                b'>' if top == F_T_ARROW_TP_LINE && b.get(p + 1) == Some(&b'(') => {
                    self.pop();
                    self.tok_far(p, p + 1, TP_END);
                    self.push(F_PARAMS_PLAIN);
                    self.prev = P_OP;
                    p + 2
                }
                b'>' if matches!(
                    top,
                    F_T_ANGLE | F_T_TPARAMS | F_T_ARROW_TP | F_T_ARROW_TP_LINE | F_T_CAST
                ) =>
                {
                    self.pop();
                    if top != F_T_CAST {
                        self.tok(p, p + 1, TP_END);
                    }
                    self.prev = match top {
                        F_T_TPARAMS => P_KEYED,
                        F_T_ARROW_TP => P_ARROW_TP,
                        _ => P_VALUE,
                    };
                    p + 1
                }
                b'|' | b'&' if b.get(p + 1) != Some(&c) => {
                    self.prev = P_OP;
                    self.tok(p, p + 1, TYPE_OP)
                }
                _ => {
                    if nested {
                        p + 1
                    } else {
                        leave(self)
                    }
                }
            },
            A_CLOSE_CURLY if top == F_T_SUBST => {
                self.pop();
                self.tok_far(p, p + 1, SUBST_END);
                self.at = p + 1;
                p + 1
            }
            A_CLOSE_PAREN | A_CLOSE_BRACKET | A_CLOSE_CURLY => {
                let closes = match c {
                    b')' => F_T_ROUND,
                    b']' => F_T_SQUARE,
                    _ => F_T_CURLY,
                };
                if top != closes && !(c == b')' && top == F_T_ROUND_LINE) {
                    return leave(self);
                }
                self.pop();
                self.prev = P_VALUE;
                if c == b'}' {
                    return self.tok_far(p, p + 1, BLOCK_BRACE);
                }
                p + 1
            }
            A_COMMA if nested => {
                self.prev = if top == F_T_CURLY { P_KEY } else { P_OP };
                self.tok_far(p, p + 1, COMMA)
            }
            A_SEMI if top == F_T_CURLY => {
                self.prev = P_KEY;
                self.tok_far(p, p + 1, TERMINATOR)
            }
            A_COLON => {
                if matches!(top, F_TYPE_AS | F_TYPE_RET)
                    || (top == F_TYPE && self.under() == F_TERNARY)
                {
                    return leave(self);
                }
                self.prev = P_OP;
                let leaf = match top {
                    F_T_CURLY | F_T_SQUARE | F_T_ROUND_LINE => ANNOTATION,
                    _ => TERNARY,
                };
                self.tok(p, p + 1, leaf)
            }
            A_QUESTION => {
                if top == F_TYPE_AS || b.get(p + 1) == Some(&b'.') {
                    return leave(self);
                }
                let s = skip_blanks(b, p + 1);
                let leaf = if matches!(b.get(s), Some(b':' | b']' | b',' | b'(')) {
                    OPTIONAL
                } else {
                    TERNARY
                };
                self.prev = P_OP;
                self.tok_far(p, p + 1, leaf)
            }
            A_EQ => {
                if b.get(p + 1) == Some(&b'>') {
                    if top == F_TYPE_RET {
                        return leave(self);
                    }
                    self.prev = P_OP;
                    return self.tok_far(p, p + 2, ARROW);
                }
                if b.get(p + 1) != Some(&b'=')
                    && matches!(
                        top,
                        F_TYPE_DECL | F_T_ANGLE | F_T_TPARAMS | F_T_ARROW_TP | F_T_ARROW_TP_LINE
                    )
                {
                    self.prev = P_OP;
                    return self.tok_far(p, p + 1, ASSIGN);
                }
                leave(self)
            }
            A_DOT => {
                if b.get(p + 1) == Some(&b'.') && b.get(p + 2) == Some(&b'.') {
                    self.prev = P_OP;
                    return self.tok_far(p, p + 3, SPREAD);
                }
                if self.prev != P_TYPEOF {
                    self.prev = P_DOT;
                }
                self.tok_far(p, p + 1, ACCESSOR)
            }
            A_SLASH => match b.get(p + 1) {
                Some(b'/') => self.line_comment(p),
                Some(b'*') => self.block_comment(p),
                _ => leave(self),
            },
            A_STAR if nested => p + 1,
            _ => {
                if nested && !matches!(c, b';' | b',') {
                    p + char_len(c)
                } else {
                    leave(self)
                }
            }
        }
    }

    /// A name at `p` in type frame `top`.
    #[inline(always)]
    fn type_name(&mut self, p: usize, top: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let mut e = p;
        while e < n {
            let c = b[e];
            if c >= 0x80 {
                if !is_word(b, e) {
                    break;
                }
                e += char_len(c);
            } else if NAME[usize::from(c)] {
                e += 1;
            } else {
                break;
            }
        }
        if e == p {
            return self.leave_or_step(p, top);
        }
        let id = WORDS.get(&b[p..e]);
        let next = b.get(e).copied();
        let prev = self.prev;
        if ((top == F_T_CURLY && prev == P_KEY)
            || (top == F_T_SQUARE && matches!(prev, P_OP | P_INDEX)))
            && let Some(q) = self.type_member(p, e, top, id)
        {
            return q;
        }
        // a type predicate's parameter, at a return type's start (a function's, an arrow's, a
        // function type's): `x is T`
        if prev == P_ASSERTS
            || (prev == P_OP && next == Some(b' ') && has(b, skip_blanks(b, e), b"is "))
            || (top == F_T_ROUND_LINE && prev == P_OP && param_colon(b, e))
        {
            self.prev = P_VALUE;
            return self.tok_far(p, e, PARAM);
        }
        let leaf = match prev {
            P_ALIAS_NAME => TYPE_ALIAS,
            P_TYPEOF => return self.typeof_name(p, e, id),
            P_DOT => {
                if next == Some(b'.') {
                    TYPE_MODULE
                } else {
                    TYPE
                }
            }
            _ => {
                if id != 0 {
                    match type_word(id) {
                        Some((leaf, after)) => {
                            self.prev = after;
                            return self.tok(p, e, leaf);
                        }
                        None => {
                            if next == Some(b'.') {
                                TYPE_MODULE
                            } else {
                                TYPE
                            }
                        }
                    }
                } else if next == Some(b'.') {
                    TYPE_MODULE
                } else {
                    TYPE
                }
            }
        };
        self.prev = P_VALUE;
        self.tok(p, e, leaf);
        if next == Some(b'<') {
            return self.type_args(e);
        }
        e
    }

    /// A name `p..e` (`id` its word) after a type's `typeof`, or after a `.` there: a value's
    /// as bat reads it (a support object's, an object's, a property's; `import(`'s keyword).
    #[inline(never)]
    fn typeof_name(&mut self, p: usize, e: usize, id: u16) -> usize {
        let b = self.b;
        let dotted = b.get(e) == Some(&b'.') && b.get(e + 1) != Some(&b'.');
        let leaf = if p > 0 && b[p - 1] == b'.' {
            if dotted { OBJECT_PROP } else { PROPERTY }
        } else if id == ID_IMPORT && b.get(e) == Some(&b'(') {
            IMPORT_OP
        } else {
            match INFO[usize::from(id)].leaf {
                l @ (NODE_OBJ | DOM_VAR) if id != 0 => l,
                _ if dotted => OBJECT,
                _ => READWRITE,
            }
        };
        self.prev = if dotted { P_TYPEOF } else { P_VALUE };
        self.tok_far(p, e, leaf)
    }

    /// A name `p..e` (`id` its word) at a member's place in an object type, or a tuple's
    /// label's or an index signature's parameter's: `None` where it is a type's name after all.
    #[inline(never)]
    fn type_member(&mut self, p: usize, e: usize, top: u8, id: u16) -> Option<usize> {
        let b = self.b;
        let prev = self.prev;
        // a member's name in an object type
        if top == F_T_CURLY && prev == P_KEY {
            let s = skip_blanks(b, e);
            if id == ID_READONLY
                && s > e
                && b.get(s).is_some_and(|&c| NAME[usize::from(c)] || c == b'[')
            {
                return Some(self.tok_far(p, e, MODIFIER));
            }
            if id == ID_NEW && matches!(b.get(s), Some(b'(' | b'<')) {
                self.prev = P_OP;
                return Some(self.tok_far(p, e, NEW));
            }
            let leaf = match b.get(s) {
                Some(b'(') => {
                    self.tok_far(p, e, ENTITY_FN);
                    self.prev = P_KEYED;
                    return Some(self.open_params(s, F_PARAMS));
                }
                Some(b'<') => {
                    self.tok_far(p, e, ENTITY_FN);
                    return Some(self.type_params(s));
                }
                Some(b'?') if b.get(skip_blanks(b, s + 1)) == Some(&b'(') => {
                    let t = skip_blanks(b, s + 1);
                    self.tok_far(p, e, ENTITY_FN);
                    self.tok_far(s, s + 1, OPTIONAL);
                    return Some(self.open_params(t, F_PARAMS));
                }
                _ => {
                    let t = if b.get(s) == Some(&b'?') { s + 1 } else { s };
                    if s == e && b.get(t) == Some(&b':') && fn_typed(b, t + 1) {
                        ENTITY_FN
                    } else {
                        OBJ_PROPERTY
                    }
                }
            };
            self.prev = P_VALUE;
            return Some(self.tok(p, e, leaf));
        }
        if top == F_T_SQUARE && matches!(prev, P_OP | P_INDEX) {
            // a named tuple member's label, an index signature's parameter
            let s = skip_blanks(b, e);
            let t = if b.get(s) == Some(&b'?') {
                skip_blanks(b, s + 1)
            } else {
                s
            };
            if b.get(t) == Some(&b':') {
                if prev == P_INDEX {
                    self.tok_far(p, e, PARAM);
                    self.prev = P_VALUE;
                    self.tok_far(t, t + 1, ANNOTATION);
                    self.prev = P_OP;
                    return Some(t + 1);
                }
                self.tok_far(p, e, LABEL);
                if t > s {
                    self.tok_far(s, s + 1, OPTIONAL);
                }
                self.tok_far(t, t + 1, LABEL_SEP);
                self.prev = P_OP;
                return Some(t + 1);
            }
        }
        None
    }

    /// A byte no type name starts at `p` in type frame `top`: stepped over in a nested frame,
    /// else the frame left.
    fn leave_or_step(&mut self, p: usize, top: u8) -> usize {
        if top >= F_T_ROUND {
            return p + char_len(self.b[p]);
        }
        self.pop();
        p
    }

    /// Type arguments' `<` at `p` (after a name): their frame.
    #[inline(never)]
    pub(super) fn type_args(&mut self, p: usize) -> usize {
        self.push(F_T_ANGLE);
        self.prev = P_OP;
        self.tok(p, p + 1, TP_BEGIN)
    }

    /// Type parameters' `<` at `p` (a function's, a class's, a method's, a generic arrow's):
    /// their frame, whose `>` leaves [`P_KEYED`] for the parameters' `(`.
    #[inline(never)]
    pub(super) fn type_params(&mut self, p: usize) -> usize {
        self.push(F_T_TPARAMS);
        self.prev = P_OP;
        self.tok_far(p, p + 1, TP_BEGIN)
    }

    /// A `<` at `p` where an expression starts: a generic arrow's type parameters (one that
    /// ends its line after `(`, `=`, `,`, `return` or a key's `:` always), a cast (TypeScript)
    /// or JSX's tag (TSX); `None` for an operator.
    #[inline(never)]
    pub(super) fn angle_start(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if skip_blanks(b, p + 1) >= b.len() {
            // a `<` that ends its line where bat's possibly-arrow `(` would: a generic arrow's
            // type parameters (a cast's too, as bat's), its `(` plain; a key's value's as one
            // on its line
            if possibly_arrow(b, p) {
                return Some(self.generic_arrow(p, F_T_ARROW_TP_LINE));
            }
            if self.assign.value == p as u32 && self.assign.kind == A_KEY {
                return Some(self.generic_arrow(p, F_T_ARROW_TP));
            }
        }
        if self.tsx {
            let s = p + 1;
            if !b.get(s).is_some_and(|&c| NAME[usize::from(c)] || c == b'>') {
                return None;
            }
            // `<T,>(` and `<T extends …>(`: a generic arrow's
            let mut e = s;
            while e < b.len() && NAME[usize::from(b[e])] {
                e += 1;
            }
            let t = skip_blanks(b, e);
            if e > s
                && (b.get(t) == Some(&b',') || (t > e && has(b, t, b"extends ")))
                && type_args_end(b, p, true).is_some_and(|q| b.get(q) == Some(&b'('))
            {
                return Some(self.generic_arrow(p, F_T_ARROW_TP));
            }
            return Some(self.jsx_open(p));
        }
        let q = type_args_end(b, p, true)?;
        if b.get(q) == Some(&b'(') {
            return Some(self.generic_arrow(p, F_T_ARROW_TP));
        }
        // a cast: its `<` and `>` plain
        self.push(F_T_CAST);
        self.prev = P_OP;
        Some(p + 1)
    }

    /// A generic arrow's `<` at `p`: an assigned name whose value it starts named a function's;
    /// the type parameters' frame (`frame`) pushed.
    fn generic_arrow(&mut self, p: usize, frame: u8) -> usize {
        if self.assign.value == p as u32 {
            self.function_named();
        } else if self.async_value(p) {
            let v = self.assign.value as usize;
            self.function_named();
            self.tok_far(v, v + 5, ASYNC);
        }
        self.push(frame);
        self.prev = P_OP;
        self.tok_far(p, p + 1, TP_BEGIN)
    }

    /// A name `p..e` before `<`: a generic call's (`f<T>(`, `` f<T>` ``) where the type
    /// arguments close before one, else a variable before an operator.
    #[inline(never)]
    pub(super) fn maybe_generic_call(&mut self, p: usize, e: usize, bits: u8) -> usize {
        let b = self.b;
        if generic_call(b, e) {
            self.tok_far(p, e, ENTITY_FN);
            return self.type_args(e);
        }
        self.tok_far(p, e, var_kind(b[p], bits))
    }
}

/// Whether the `<` at `e` after a name opens a generic call's type arguments: they close
/// before a `(` or a template, or open a bracket the line ends in (`useState<{`).
#[inline(never)]
pub(super) fn generic_call(b: &[u8], e: usize) -> bool {
    if type_args_end(b, e, false).is_some_and(|q| matches!(b.get(q), Some(b'(' | b'`'))) {
        return true;
    }
    let s = skip_blanks(b, e + 1);
    matches!(b.get(s), Some(b'{' | b'[' | b'(')) && skip_blanks(b, s + 1) >= b.len()
}

/// Whether a `<` at `p` right after a `)` opens type arguments that close before a `(`, as
/// bat reads a call's result called with them (`styled(X)<P>(…)`).
#[cold]
#[inline(never)]
pub(super) fn call_type_args(b: &[u8], p: usize) -> bool {
    type_args_end(b, p, false).is_some_and(|q| b.get(q) == Some(&b'('))
}

/// A type word's leaf and the previous token it leaves, by id: `None` for a type's name.
#[inline(always)]
fn type_word(id: u16) -> Option<(u16, u8)> {
    Some(match id {
        ID_STRING | ID_NUMBER | ID_BOOLEAN | ID_ANY | ID_UNKNOWN | ID_NEVER | ID_VOID
        | ID_SYMBOL | ID_BIGINT => (PRIMITIVE, P_VALUE),
        ID_OBJECT | ID_UNDEFINED | ID_NULL => (BUILTIN_TYPE, P_VALUE),
        ID_TRUE | ID_FALSE => (BUILTIN_TYPE, P_VALUE),
        ID_THIS => (THIS, P_VALUE),
        ID_KEYOF => (KEYOF, P_OP),
        ID_TYPEOF => (TYPEOF, P_TYPEOF),
        ID_INFER => (INFER, P_INFER),
        ID_IS => (IS, P_OP),
        ID_ASSERTS => (ASSERTS, P_ASSERTS),
        ID_EXTENDS => (MODIFIER, P_OP),
        ID_READONLY | ID_UNIQUE | ID_ABSTRACT => (MODIFIER, P_OP),
        ID_IN => (IN, P_OP),
        ID_AS => (AS, P_OP),
        // a constructor type's (an object type's `new (` is a member's, `type_member`)
        ID_NEW => (NEW_CONTROL, P_OP),
        _ => return None,
    })
}

const ID_STRING: u16 = words::id_of(b"string");
const ID_NUMBER: u16 = words::id_of(b"number");
const ID_BOOLEAN: u16 = words::id_of(b"boolean");
const ID_ANY: u16 = words::id_of(b"any");
const ID_UNKNOWN: u16 = words::id_of(b"unknown");
const ID_NEVER: u16 = words::id_of(b"never");
const ID_VOID: u16 = words::id_of(b"void");
const ID_SYMBOL: u16 = words::id_of(b"symbol");
const ID_BIGINT: u16 = words::id_of(b"bigint");
const ID_OBJECT: u16 = words::id_of(b"object");
const ID_UNDEFINED: u16 = words::id_of(b"undefined");
const ID_NULL: u16 = words::id_of(b"null");
const ID_TRUE: u16 = words::id_of(b"true");
const ID_FALSE: u16 = words::id_of(b"false");
const ID_INFER: u16 = words::id_of(b"infer");
const ID_IS: u16 = words::id_of(b"is");
const ID_ASSERTS: u16 = words::id_of(b"asserts");
const ID_UNIQUE: u16 = words::id_of(b"unique");

/// Where type arguments opening at `p` (a `<`) close, past their `>`: `None` where a byte no
/// type holds comes first, a bracket closes below them, or the line ends. With `defaults`, type
/// parameters' (`<T = {}>`): a `=` is a default's.
pub(super) fn type_args_end(b: &[u8], p: usize, defaults: bool) -> Option<usize> {
    let mut angle = 0u32;
    let mut nest = 0u32;
    let mut i = p;
    while i < b.len() {
        let c = b[i];
        match c {
            b'<' => angle += 1,
            b'>' => {
                if nest != 0 {
                    return None;
                }
                angle -= 1;
                if angle == 0 {
                    return Some(i + 1);
                }
            }
            b'(' | b'[' | b'{' => nest += 1,
            b')' | b']' | b'}' => {
                if nest == 0 {
                    return None;
                }
                nest -= 1;
            }
            b'=' => {
                if b.get(i + 1) == Some(&b'>') {
                    i += 2;
                    continue;
                }
                if !defaults || b.get(i + 1) == Some(&b'=') {
                    return None;
                }
            }
            b'"' | b'\'' => {
                let q = c;
                i += 1;
                while i < b.len() && b[i] != q {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                if i >= b.len() {
                    return None;
                }
            }
            b';' if nest == 0 => return None,
            b'a'..=b'z'
            | b'A'..=b'Z'
            | b'0'..=b'9'
            | b'_'
            | b'$'
            | b' '
            | b'\t'
            | b','
            | b'.'
            | b'|'
            | b'&'
            | b'?'
            | b':'
            | b';'
            | b'-'
            | 0x80..=0xff => {}
            _ => return None,
        }
        i += 1;
    }
    None
}

/// Whether a type from `p` (past a group's `):`) runs to an arrow's `=>` on the line: the
/// group was an arrow's parameters, the type its return type.
pub(super) fn arrow_return(b: &[u8], p: usize) -> bool {
    let mut nest = 0u32;
    let mut i = p;
    while i < b.len() {
        let c = b[i];
        match c {
            b'(' | b'[' | b'{' | b'<' => nest += 1,
            b')' | b']' | b'}' => {
                if nest == 0 {
                    return false;
                }
                nest -= 1;
            }
            b'>' => {
                if nest == 0 {
                    return false;
                }
                nest -= 1;
            }
            b'=' => {
                if b.get(i + 1) == Some(&b'>') {
                    if nest == 0 {
                        return true;
                    }
                    i += 2;
                    continue;
                }
                return false;
            }
            b',' | b';' if nest == 0 => return false,
            b'"' | b'\'' => {
                let q = c;
                i += 1;
                while i < b.len() && b[i] != q {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'a'..=b'z'
            | b'A'..=b'Z'
            | b'0'..=b'9'
            | b'_'
            | b'$'
            | b' '
            | b'\t'
            | b','
            | b';'
            | b'.'
            | b'|'
            | b'&'
            | b'?'
            | b':'
            | b'-'
            | 0x80..=0xff => {}
            _ => return false,
        }
        i += 1;
    }
    false
}

/// Whether a name ending at `e` is a parameter's in a function type: `:` or `?:` after it.
#[cold]
#[inline(never)]
fn param_colon(b: &[u8], e: usize) -> bool {
    let s = skip_blanks(b, e);
    match b.get(s) {
        Some(b':') => true,
        Some(b'?') => b.get(skip_blanks(b, s + 1)) == Some(&b':'),
        _ => false,
    }
}

/// Whether an assignment's `=` from `p` on the line (not `=>` or `==`) is followed by what
/// could start a function value (`function`, `async`, `(`, `<`): [`annotated_fn`]'s gate, so
/// the common annotation without one isn't walked.
#[inline(always)]
fn fn_value_ahead(b: &[u8], p: usize) -> bool {
    let n = b.len();
    let mut i = p;
    loop {
        i = crate::prim::find_in(b, i, n, *b"=");
        if i >= n {
            return false;
        }
        if !matches!(b.get(i + 1), Some(b'>' | b'=')) {
            let v = skip_blanks(b, i + 1);
            if matches!(b.get(v), Some(b'f' | b'a' | b'(' | b'<')) {
                return true;
            }
        }
        i += 1;
    }
}

/// Whether a type's `(` before `p` opens a function type's parameters: `()` before `=>`, a
/// rest, a pattern, or a name before `:`, `?`, `,`, or `)` and `=>`; one that ends its line
/// possibly (bat's).
pub(super) fn fn_type_params(b: &[u8], p: usize) -> bool {
    let s = skip_blanks(b, p);
    match b.get(s) {
        None => true,
        Some(b')') => {
            let t = skip_blanks(b, s + 1);
            b.get(t) == Some(&b'=') && b.get(t + 1) == Some(&b'>')
        }
        Some(b'.') => b.get(s + 1) == Some(&b'.'),
        Some(b'{' | b'[') => true,
        Some(&c) if NAME[usize::from(c)] => {
            let mut e = s;
            while e < b.len() && NAME[usize::from(b[e])] {
                e += 1;
            }
            let t = skip_blanks(b, e);
            match b.get(t) {
                Some(b':' | b',') => true,
                Some(b'=') => b.get(t + 1) != Some(&b'>'),
                Some(b'?') => matches!(b.get(skip_blanks(b, t + 1)), Some(b':' | b',' | b')')),
                Some(b')') => {
                    let u = skip_blanks(b, t + 1);
                    b.get(u) == Some(&b'=') && b.get(u + 1) == Some(&b'>')
                }
                _ => false,
            }
        }
        _ => false,
    }
}

/// Whether a `[` before `p` at a member's place opens an index signature: a name before `:`.
pub(super) fn index_signature(b: &[u8], p: usize) -> bool {
    let s = skip_blanks(b, p);
    let mut e = s;
    while e < b.len() && NAME[usize::from(b[e])] {
        e += 1;
    }
    e > s && b.get(skip_blanks(b, e)) == Some(&b':')
}

/// Whether a type from `p` (past a member's or a parameter's `:`) is a function's: `(…) =>`,
/// `<T>(…) =>`, or bat's `Function`.
pub(super) fn fn_typed(b: &[u8], p: usize) -> bool {
    let s = skip_blanks(b, p);
    match b.get(s) {
        Some(b'(') => fn_type_params(b, s + 1),
        Some(b'<') => type_args_end(b, s, true).is_some_and(|q| b.get(q) == Some(&b'(')),
        Some(b'F') => {
            has(b, s, b"Function") && !b.get(s + 8).is_some_and(|&c| NAME[usize::from(c)])
        }
        _ => false,
    }
}

/// Whether a name from `p` is a type alias's: its `=` or `<` after it (`export type X =`).
pub(super) fn alias_ahead(b: &[u8], p: usize) -> bool {
    let mut e = p;
    while e < b.len() && NAME[usize::from(b[e])] {
        e += 1;
    }
    e > p && matches!(b.get(skip_blanks(b, e)), Some(b'=' | b'<'))
}

/// Whether a name and a `:` start at `p` (past blanks): `catch (e: unknown)`'s parameter.
pub(super) fn annotated_name(b: &[u8], p: usize) -> bool {
    let s = skip_blanks(b, p);
    let mut e = s;
    while e < b.len() && NAME[usize::from(b[e])] {
        e += 1;
    }
    e > s && b.get(skip_blanks(b, e)) == Some(&b':')
}

/// Whether an annotated declaration's type from `p` (past its `:`) is followed by `=` and a
/// function as the value: `function`, `async`, a `(` that ends the line or opens an arrow's
/// parameters, a generic arrow's `<` (bat's `var-single-variable` lookahead).
pub(super) fn annotated_fn(b: &[u8], p: usize) -> bool {
    if !fn_value_ahead(b, p) {
        return false;
    }
    // the type, to its `=` at depth 0
    let mut nest = 0u32;
    let mut i = p;
    while i < b.len() {
        match b[i] {
            b'(' | b'[' | b'{' | b'<' => nest += 1,
            b')' | b']' | b'}' => {
                if nest == 0 {
                    return false;
                }
                nest -= 1;
            }
            b'>' => {
                if i > 0 && b[i - 1] == b'=' {
                    // a function type's `=>`
                } else if nest == 0 {
                    return false;
                } else {
                    nest -= 1;
                }
            }
            b'=' if nest == 0 && !matches!(b.get(i + 1), Some(b'>' | b'=')) => break,
            b',' | b';' if nest == 0 => return false,
            _ => {}
        }
        i += 1;
    }
    if i >= b.len() {
        return false;
    }
    let v = skip_blanks(b, i + 1);
    let rest = &b[v.min(b.len())..];
    if rest.starts_with(b"function") || rest.starts_with(b"async ") || rest.starts_with(b"async(") {
        return true;
    }
    match rest.first() {
        Some(b'(') => {
            let s = skip_blanks(b, v + 1);
            if s >= b.len() || (matches!(b[s], b'{' | b'[') && skip_blanks(b, s + 1) >= b.len()) {
                return true;
            }
            // the group's `)` and an arrow after it
            let mut d = 0u32;
            let mut k = v;
            while k < b.len() {
                match b[k] {
                    b'(' => d += 1,
                    b')' => {
                        d -= 1;
                        if d == 0 {
                            let t = skip_blanks(b, k + 1);
                            return (b.get(t) == Some(&b'=') && b.get(t + 1) == Some(&b'>'))
                                || (b.get(t) == Some(&b':') && arrow_return(b, t + 1));
                        }
                    }
                    _ => {}
                }
                k += 1;
            }
            false
        }
        Some(b'<') => type_args_end(b, v, true).is_some_and(|q| b.get(q) == Some(&b'(')),
        _ => false,
    }
}
