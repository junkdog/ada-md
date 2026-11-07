//! Java's code on frames of ada's own design (X-015), on the C family's core ([`crate::clike`]).
//!
//! The run loop reads a token at a time: a blank run is passed whole, a name is read once with
//! the classes of its bytes and looked up once among the keywords ([`WORDS`]), and the frame on
//! top of the stack classifies it, with what follows it on the line (`(`, `.`, `->`, `<`). A
//! type is read whole where it is (its package, its name, its arguments, its `[]`), so the
//! contexts bat's grammar keeps between a type's parts aren't pushed. The frames are the
//! structure that colors: a block, a class's body, a member's declaration, parentheses and
//! parameters, an annotation's arguments, an array's initializer.
//!
//! What the frames color is bat's `Java` grammar's, odd corners included, so that themes
//! written for it apply: a capital starts a type and all capitals a constant, `name(` is a
//! call, a class body's `name(` a method's name, a parameter's name its own scope, `@Override`'s
//! line end and the next line's indentation the annotation's. The declarations' headers (a
//! class's, a type parameter's, `throws`, imports, `module-info.java`) are rare and stay on
//! bat's patterns, from [`FRAMES`] up; a type's arguments that don't close on their line, or a
//! path a line's end cuts, continue on those too.

use super::*;
use crate::clike::{self, DIGIT, DOLLAR, HIGH, Ident, LOWER, UPPER};
use crate::words::{Words, text_len};
use clike::CLASS as BYTES;

// What a keyword is, by group.
const K_LANGUAGE: u8 = 1;
const K_SPECIAL: u8 = 2;
const K_NEW: u8 = 3;
const K_MODIFIER: u8 = 4;
const K_PRIMITIVE: u8 = 5;
const K_VOID: u8 = 6;
const K_VAR: u8 = 7;
const K_INSTANCEOF: u8 = 8;
const K_ILLEGAL: u8 = 9;
const K_CONTROL: u8 = 10;
/// `class`, `interface`, `enum`: a declaration ahead.
const K_CLASS: u8 = 11;
/// A word a frame asks for by its id.
const K_OTHER: u8 = 12;

const N: usize = 59;

/// Every word the frames tell apart, its group, and a control keyword's leaf; its id is its
/// index plus one.
static KEYWORDS: [(&[u8], u8, u16); N] = [
    (b"true", K_LANGUAGE, 0),
    (b"false", K_LANGUAGE, 0),
    (b"null", K_LANGUAGE, 0),
    (b"this", K_SPECIAL, 0),
    (b"super", K_SPECIAL, 0),
    (b"new", K_NEW, 0),
    (b"public", K_MODIFIER, 0),
    (b"private", K_MODIFIER, 0),
    (b"protected", K_MODIFIER, 0),
    (b"static", K_MODIFIER, 0),
    (b"final", K_MODIFIER, 0),
    (b"native", K_MODIFIER, 0),
    (b"synchronized", K_MODIFIER, 0),
    (b"strictfp", K_MODIFIER, 0),
    (b"abstract", K_MODIFIER, 0),
    (b"transient", K_MODIFIER, 0),
    (b"default", K_MODIFIER, 0),
    (b"volatile", K_MODIFIER, 0),
    (b"boolean", K_PRIMITIVE, 0),
    (b"byte", K_PRIMITIVE, 0),
    (b"char", K_PRIMITIVE, 0),
    (b"short", K_PRIMITIVE, 0),
    (b"int", K_PRIMITIVE, 0),
    (b"float", K_PRIMITIVE, 0),
    (b"long", K_PRIMITIVE, 0),
    (b"double", K_PRIMITIVE, 0),
    (b"void", K_VOID, 0),
    (b"var", K_VAR, 0),
    (b"instanceof", K_INSTANCEOF, 0),
    (b"goto", K_ILLEGAL, 0),
    (b"const", K_ILLEGAL, 0),
    (b"catch", K_CONTROL, CATCH_KW),
    (b"try", K_CONTROL, TRY_KW),
    (b"for", K_CONTROL, FOR_KW),
    (b"assert", K_CONTROL, ASSERT_KW),
    (b"finally", K_CONTROL, FINALLY_KW),
    (b"break", K_CONTROL, BREAK_KW),
    (b"continue", K_CONTROL, CONTINUE_KW),
    (b"return", K_CONTROL, RETURN_KW),
    (b"throw", K_CONTROL, THROW_KW),
    (b"if", K_CONTROL, IF_KW),
    (b"else", K_CONTROL, ELSE_KW),
    (b"switch", K_CONTROL, SWITCH_KW),
    (b"case", K_CONTROL, CASE_KW),
    (b"do", K_CONTROL, DO_KW),
    (b"while", K_CONTROL, WHILE_KW),
    (b"class", K_CLASS, 0),
    (b"interface", K_CLASS, 0),
    (b"enum", K_CLASS, 0),
    (b"package", K_OTHER, 0),
    (b"import", K_OTHER, 0),
    (b"module", K_OTHER, 0),
    (b"open", K_OTHER, 0),
    (b"throws", K_OTHER, 0),
    (b"extends", K_OTHER, 0),
    (b"implements", K_OTHER, 0),
    (b"UUID", K_OTHER, 0),
    (b"URL", K_OTHER, 0),
    (b"URI", K_OTHER, 0),
];

// The ids of the words the frames ask for by name.
const W_FINAL: u8 = 11;
const W_DEFAULT: u8 = 17;
const W_PACKAGE: u8 = 50;
const W_IMPORT: u8 = 51;
const W_MODULE: u8 = 52;
const W_OPEN: u8 = 53;
const W_THROWS: u8 = 54;
const W_UUID: u8 = 57;
const W_URI: u8 = 59;

const WORD_LIST: [&[u8]; N] = {
    let mut w: [&[u8]; N] = [b""; N];
    let mut i = 0;
    while i < N {
        w[i] = KEYWORDS[i].0;
        i += 1;
    }
    w
};
static WORDS: Words<128, N, { text_len(&WORD_LIST) }> = Words::new(&WORD_LIST);

/// A name read at the scan's place: where it starts and ends, its bytes' classes, its keyword.
#[derive(Clone, Copy)]
struct Name {
    p: usize,
    e: usize,
    bits: u8,
    id: u8,
}

impl Name {
    fn group(self) -> u8 {
        if self.id == 0 {
            0
        } else {
            KEYWORDS[usize::from(self.id - 1)].1
        }
    }

    fn leaf(self) -> u16 {
        KEYWORDS[usize::from(self.id - 1)].2
    }

    /// Whether it is ASCII letters, digits and `_` only.
    fn plain(self) -> bool {
        self.bits & (HIGH | DOLLAR) == 0
    }

    /// Whether a plain name is all capitals: `[_]*[A-Z][A-Z0-9_]*`, a constant's.
    fn capitals(self, b: &[u8]) -> bool {
        if self.bits & (LOWER | UPPER) != UPPER {
            return false;
        }
        let mut i = self.p;
        while b[i] == b'_' {
            i += 1;
        }
        b[i].is_ascii_uppercase()
    }
}

/// `[\p{L}\p{N}]` past ASCII: an identifier's character.
fn wide_ident(c: char) -> bool {
    c.is_alphabetic() || c.is_numeric()
}

/// Whether `top` is one of the frames here, not a context on bat's patterns.
#[inline(always)]
pub(super) fn is_frame(top: u8) -> bool {
    top < FRAMES
}

/// A package path read on its line ([`Scan::path`]).
enum Path {
    /// To a type's name, which ends here.
    Typed(usize),
    /// To where it stopped, no type there.
    Stopped(usize),
    /// To the line's end.
    Cut,
}

impl<'b, K: Sink> Scan<'b, '_, K> {
    /// The name at `p`, whose first byte is an identifier's start.
    #[inline(always)]
    fn name(&self, p: usize) -> Name {
        let b = self.b;
        let Ident { end, bits } = clike::ident_end::<true>(b, next(b, p), wide_ident);
        let bits = bits | BYTES[usize::from(b[p])];
        let id = if b[p].is_ascii_lowercase() || b[p] == b'U' {
            WORDS.get(&b[p..end]) as u8
        } else {
            0
        };
        Name {
            p,
            e: end,
            bits,
            id,
        }
    }

    /// The frames' run from `p`, while a frame is on top: where it stopped, the line's end or
    /// the start of a context on bat's patterns.
    pub(super) fn code_run(&mut self, mut p: usize) -> usize {
        let b = self.b;
        if p == 0 && !self.enum_checked {
            self.enum_checked = true;
            if self.top() == ENUM_BODY && self.enum_line() {
                self.push(0, ENUM_CONSTANTS);
            }
        }
        loop {
            let top = self.top();
            if !is_frame(top) {
                return p;
            }
            if p < b.len() && BYTES[usize::from(b[p])] & clike::SPACE != 0 {
                let q = clike::skip_blanks(b, p);
                // `illegal-open-block`'s `\s?(?={)`: the last blank before `{` ends parentheses
                if b.get(q) == Some(&b'{')
                    && matches!(top, PARENS | PARENS_SQL | CTOR_ARGS | DECL_PARENS_IN)
                {
                    p = self.close(q - 1, 1, STRAY_TERMINATOR);
                    continue;
                }
                p = q;
            }
            if p >= b.len() {
                return b.len();
            }
            p = self.step(p, top);
        }
    }

    /// One token at `p`, not a blank, in frame `top`: where the scan goes on (the same place
    /// where the frame popped without taking it).
    #[inline(always)]
    fn step(&mut self, p: usize, top: u8) -> usize {
        let b = self.b;
        let c = b[p];
        // the prototype: comments, `goto` and `const`, `%>`
        match c {
            b'/' => {
                if let Some(r) = self.comment(p) {
                    return r;
                }
            }
            b'%' if b.get(p + 1) == Some(&b'>') && self.depth > 1 => {
                self.pop(p);
                return p;
            }
            _ => {}
        }
        let k = BYTES[usize::from(c)];
        if (k & (clike::WORD | DOLLAR) != 0 && k & DIGIT == 0)
            || (k & HIGH != 0 && is_ident_start(b, p))
        {
            let n = self.name(p);
            if n.group() == K_ILLEGAL {
                return self.token(p, n.e - p, ILLEGAL_KEYWORD);
            }
            return self.word_step(n, top);
        }
        self.punct_step(p, c, top)
    }

    fn word_step(&mut self, n: Name, top: u8) -> usize {
        match top {
            BLOCK | DECL_PARENS_IN => match n.group() {
                K_MODIFIER => self.token(n.p, n.e - n.p, MODIFIER),
                K_VAR => self.token(n.p, n.e - n.p, VAR),
                _ => self.code_word(n),
            },
            TOP => self.top_word(n),
            CLASS_BODY | ENUM_BODY | ANON_BODY => self.body_word(n),
            METHOD_PARAMS | LAMBDA_PARAM_LIST | CATCH_PARAMS => self.param_word(n),
            ENUM_CONSTANTS => self.enum_constant(n.p),
            ANNOT_ARGS | ANNOT_ARGS_SQL => {
                let b = self.b;
                let q = skip_spaces(b, n.e);
                if b.get(q) == Some(&b'=') {
                    self.put(n.p, n.e, PARAMETER);
                    self.put(q, q + 1, ASSIGN);
                    let value = if top == ANNOT_ARGS_SQL && sql::jvm_argument(&b[n.p..n.e]) {
                        ANNOT_VALUE_SQL
                    } else {
                        ANNOT_VALUE
                    };
                    self.push(q + 1, value);
                    return q + 1;
                }
                self.code_word(n)
            }
            STATIC_RHS => {
                if matches!(n.group(), K_MODIFIER | K_PRIMITIVE | K_VOID) {
                    self.pop(n.p);
                    return n.p;
                }
                self.code_word(n)
            }
            METHOD => self.method_word(n),
            MEMBER => {
                let to = if self.call_ahead(n.p) { METHOD } else { FIELD };
                self.stack[self.depth - 1] = to;
                n.p
            }
            FIELD => self.field_word(n),
            INSTANTIATION => self.instantiation(n),
            METHOD_REF => self.close(n.p, n.e - n.p, FUNCTION_REF),
            LAMBDA_ARROW => n.e,
            AFTER_NEW_TYPE | ARRAY_DEF | CTOR_AFTER | DECL_PARENS | CATCH | ANNOT_PARAMS
            | ANNOT_PARAMS_SQL | FIELD_NAMED | ENUM_CONSTANT => {
                self.pop(n.p);
                n.p
            }
            _ => self.code_word(n),
        }
    }

    fn punct_step(&mut self, p: usize, c: u8, top: u8) -> usize {
        let b = self.b;
        match top {
            TOP => match c {
                b'@' => {
                    if crate::has_at(b, p, b"@interface") && self.class_ahead(p) {
                        return self.enter(p, &[CLASS_TYPE]);
                    }
                    return self.annotation(p);
                }
                b'}' => return self.token(p, 1, STRAY_BRACE),
                _ => {}
            },
            BLOCK => match c {
                b'}' => return self.close(p, 1, BLOCK_END),
                b'@' => return self.annotation(p),
                b')' => return self.token(p, 1, STRAY_PAREN),
                _ => {}
            },
            CLASS_BODY | ANON_BODY | ENUM_BODY => return self.body_punct(p, c, top),
            PARENS | PARENS_SQL | CTOR_ARGS => match c {
                b')' => {
                    if top == CTOR_ARGS {
                        self.put(p, p + 1, PAREN_END);
                        return self.ctor_after(p + 1);
                    }
                    return self.close(p, 1, PAREN_END);
                }
                b';' => return self.close(p, 1, STRAY_TERMINATOR),
                b'{' => return self.pop_at(p),
                b',' if top == PARENS_SQL => {
                    self.stack[self.depth - 1] = PARENS;
                    return p;
                }
                b'"' if top == PARENS_SQL => return self.sql_string(p),
                _ => {}
            },
            DECL_PARENS_IN => match c {
                b')' => return self.close(p, 1, PAREN_END),
                b'{' => return self.pop_at(p),
                b'@' => return self.annotation(p),
                _ => {}
            },
            METHOD_PARAMS | LAMBDA_PARAM_LIST | CATCH_PARAMS => {
                return self.param_punct(p, c, top);
            }
            ENUM_CONSTANTS => {
                return match c {
                    b';' | b'}' => self.pop_at(p),
                    b',' => self.token(p, 1, COMMA),
                    b'0'..=b'9' => self.enum_constant(p),
                    _ => next(b, p),
                };
            }
            ENUM_CONSTANT => {
                return match c {
                    b'{' => self.open(p, 1, ENUM_BODY, BLOCK_BEGIN),
                    b'(' => self.open(p, 1, PARENS, PAREN_BEGIN),
                    _ => self.pop_at(p),
                };
            }
            CTOR_AFTER => {
                if c == b'{' {
                    return self.then(p, 1, &[ANON_BODY], BRACE_BEGIN);
                }
                return self.pop_at(p);
            }
            LAMBDA_ARROW => {
                if c == b'-' && b.get(p + 1) == Some(&b'>') {
                    return self.then(p, 2, &[LAMBDA_BODY], ARROW);
                }
                return next(b, p);
            }
            RHS => {
                if matches!(c, b';' | b')' | b'}' | b',') {
                    return self.pop_at(p);
                }
            }
            LAMBDA_BODY => {
                if matches!(c, b')' | b'}' | b';') {
                    return self.pop_at(p);
                }
            }
            ASSERT => match c {
                b';' => return self.pop_at(p),
                b':' => return self.token(p, 1, EXPR_SEP),
                _ => {}
            },
            ANNOT_DEFAULT => {
                if c == b';' {
                    return self.pop_at(p);
                }
            }
            ARRAY_INIT | ANNOT_ARRAY_INIT => match c {
                b'}' => return self.close(p, 1, BRACE_END),
                b'{' => return self.open(p, 1, ARRAY_INIT, BRACE_BEGIN),
                b'@' if top == ANNOT_ARRAY_INIT => return self.annotation(p),
                _ => {}
            },
            ARRAY_DEF_IN => {
                if c == b']' {
                    return self.then(p, 1, &[ARRAY_DEF], BRACKET_END);
                }
            }
            ARRAY_DEF => {
                return match c {
                    b'[' => self.then(p, 1, &[ARRAY_DEF_IN], BRACKET_BEGIN),
                    b'{' => self.then(p, 1, &[ARRAY_INIT], BRACE_BEGIN),
                    _ => self.pop_at(p),
                };
            }
            ANNOT_PARAMS | ANNOT_PARAMS_SQL => {
                if c == b'(' {
                    let to = if top == ANNOT_PARAMS {
                        ANNOT_ARGS
                    } else {
                        ANNOT_ARGS_SQL
                    };
                    return self.then(p, 1, &[to], PAREN_BEGIN);
                }
                return self.pop_at(p);
            }
            ANNOT_ARGS | ANNOT_ARGS_SQL => match c {
                b')' => return self.close(p, 1, PAREN_END),
                b'{' => return self.open(p, 1, ANNOT_ARRAY_INIT, BRACE_BEGIN),
                b'@' => return self.annotation(p),
                b'"' if top == ANNOT_ARGS_SQL => return self.sql_string(p),
                _ => {}
            },
            ANNOT_VALUE | ANNOT_VALUE_SQL => match c {
                b',' | b'}' | b')' => return self.pop_at(p),
                b'@' => return self.annotation(p),
                b'"' if top == ANNOT_VALUE_SQL => return self.sql_string(p),
                _ => {}
            },
            STATIC_RHS => match c {
                b',' | b';' => return self.pop_at(p),
                b')' => return self.token(p, 1, STRAY_PAREN),
                _ => {}
            },
            METHOD => {
                return match c {
                    b'(' => self.open(p, 1, METHOD_PARAMS, PAREN_BEGIN),
                    b'{' => self.then(p, 1, &[BLOCK], BLOCK_BEGIN),
                    _ => self.pop_at(p),
                };
            }
            MEMBER => {
                self.stack[self.depth - 1] = FIELD;
                return p;
            }
            FIELD => {
                if c == b',' {
                    return self.token(p, 1, COMMA);
                }
                return self.pop_at(p);
            }
            FIELD_NAMED => {
                if c == b'=' {
                    self.put(p, p + 1, ASSIGN);
                    self.stack[self.depth - 1] = STATIC_RHS;
                    return p + 1;
                }
                return match self.brackets(p) {
                    Some(e) => e,
                    None => self.pop_at(p),
                };
            }
            INSTANTIATION => return self.pop_at(p),
            AFTER_NEW_TYPE => return self.after_new(p),
            METHOD_REF => return self.pop_at(p),
            DECL_PARENS | CATCH => {
                if c == b'(' {
                    let to = if top == CATCH {
                        CATCH_PARAMS
                    } else {
                        DECL_PARENS_IN
                    };
                    return self.then(p, 1, &[to], PAREN_BEGIN);
                }
                return self.pop_at(p);
            }
            _ => {}
        }
        // `code`'s
        if c.is_ascii_digit() {
            return self.number_or_word(p);
        }
        match self.code_punct(p, c) {
            Some(r) => r,
            None => next(b, p),
        }
    }

    /// The top frame pops at `p`, which its parent then reads.
    fn pop_at(&mut self, p: usize) -> usize {
        self.pop(p);
        p
    }

    // code

    /// `main`'s words: its declarations, then modifiers, then `code`.
    fn top_word(&mut self, n: Name) -> usize {
        let p = n.p;
        match n.id {
            W_PACKAGE => {
                self.put(p, n.e, PACKAGE_KW);
                self.push(n.e, PACKAGE_NAME);
                return n.e;
            }
            W_IMPORT => return self.open(p, n.e - p, IMPORT_BODY, IMPORT_KW),
            W_OPEN | W_MODULE if self.module_ahead(p) => return self.enter(p, &[MODULE_KW]),
            _ => {}
        }
        match n.group() {
            K_CLASS => return self.enter(p, &[CLASS_TYPE]),
            K_MODIFIER => {
                if self.class_ahead(p) {
                    return self.enter(p, &[CLASS_TYPE]);
                }
                return self.token(p, n.e - p, MODIFIER);
            }
            _ => {}
        }
        self.code_word(n)
    }

    /// `code` at a name.
    fn code_word(&mut self, n: Name) -> usize {
        let b = self.b;
        let (p, e) = (n.p, n.e);
        let g = n.group();
        match g {
            K_LANGUAGE => return self.token(p, e - p, LANGUAGE),
            K_SPECIAL => return self.token(p, e - p, VAR_LANGUAGE),
            _ => {}
        }
        let q = skip_spaces(b, e);
        let after = b.get(q).copied();
        // lambdas, `name ->`
        if after == Some(b'-') && b.get(q + 1) == Some(&b'>') {
            self.put(p, e, PARAMETER);
            self.put(q, q + 2, ARROW);
            return self.lambda_body(q + 2);
        }
        match g {
            K_NEW => return self.open(p, e - p, INSTANTIATION, NEW),
            K_CONTROL => {
                let leaf = n.leaf();
                let frame = match leaf {
                    CATCH_KW => CATCH,
                    TRY_KW | FOR_KW => DECL_PARENS,
                    ASSERT_KW => ASSERT,
                    _ => return self.token(p, e - p, leaf),
                };
                if frame != ASSERT && after == Some(b'(') {
                    let to = if frame == CATCH {
                        CATCH_PARAMS
                    } else {
                        DECL_PARENS_IN
                    };
                    self.put(p, e, leaf);
                    return self.open(q, 1, to, PAREN_BEGIN);
                }
                return self.open(p, e - p, frame, leaf);
            }
            _ => {}
        }
        if n.id == W_DEFAULT {
            return self.token(p, e - p, DEFAULT_KW);
        }
        // method-invocations
        if after == Some(b'(') {
            self.put(p, e, FUNCTION_CALL);
            let parens = if sql::jvm_call(b, p, e) {
                PARENS_SQL
            } else {
                PARENS
            };
            return self.open(q, 1, parens, PAREN_BEGIN);
        }
        if !n.plain() {
            return self.code_word_wide(n);
        }
        let first = b[p];
        if first.is_ascii_lowercase() {
            // a plain name, a primitive, a package before its type, `instanceof`
            return match g {
                K_PRIMITIVE => {
                    self.put(p, e, PRIMITIVE);
                    self.array_mods(e)
                }
                K_INSTANCEOF => self.token(p, e - p, INSTANCEOF),
                _ if b.get(e) == Some(&b'.') && self.fqn_ahead(p) => self.code_path(p),
                _ => e,
            };
        }
        if first.is_ascii_uppercase() {
            // `UUID`, `URL`, `URI` and a single capital are types; all capitals a constant
            if (W_UUID..=W_URI).contains(&n.id) || e == p + 1 {
                self.put(p, e, CLASS);
                return self.code_type(e);
            }
            if n.bits & LOWER == 0 {
                return self.token(p, e - p, CONSTANT);
            }
            self.put(p, e, CLASS);
            return self.code_type(e);
        }
        // `_` first
        if n.capitals(b) {
            return self.token(p, e - p, CONSTANT);
        }
        if b.get(e) == Some(&b'.') && self.fqn_ahead(p) {
            return self.code_path(p);
        }
        e
    }

    /// A package path in code at `p`, which `fqn_ahead` found leads to a type.
    fn code_path(&mut self, p: usize) -> usize {
        match self.path(p) {
            Path::Typed(t) => self.code_type(t),
            Path::Stopped(q) => q,
            Path::Cut => self.b.len(),
        }
    }

    /// [`Scan::code_word`] for a name with `$` or a character past ASCII: bat's patterns,
    /// each by characters.
    #[cold]
    #[inline(never)]
    fn code_word_wide(&mut self, n: Name) -> usize {
        let b = self.b;
        let p = n.p;
        // the word of `\w` at `p`, which `\b` follows
        let w = if is_word(b, p) { word_end(b, p) } else { p };
        if w == p {
            // `$` first: nothing starts here but past it
            return next(b, p);
        }
        let plain = b[p].is_ascii_lowercase() && n.group() == 0 && n.e == w;
        if plain && b.get(w) != Some(&b'.') {
            return w;
        }
        if is_upper(b, p) && next(b, p) == w {
            self.put(p, w, CLASS);
            return self.code_type(w);
        }
        if (matches!(b[p], b'_' | b'$') || is_upper(b, p))
            && let Some(ue) = upper_word(b, p)
        {
            return self.token(p, ue - p, CONSTANT);
        }
        if b.get(n.e) == Some(&b'.') && self.fqn_ahead(p) {
            return self.code_path(p);
        }
        if let Some(te) = type_name(b, p)
            && let Some(te) = back_to(b, next(b, p), te, |_| true)
        {
            self.put(p, te, CLASS);
            return self.code_type(te);
        }
        w
    }

    /// A digit at `p` in code: a number, or the word it starts.
    fn number_or_word(&mut self, p: usize) -> usize {
        let b = self.b;
        if boundary(b, p)
            && let Some(r) = self.number(p)
        {
            return r;
        }
        // the digits, then what may start a name
        run_of(b, p, |b, i| !is_ident_start(b, i))
            .min(word_end(b, p))
            .max(p + 1)
    }

    /// `code`'s patterns at a byte that isn't a name's.
    fn code_punct(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        let next = b.get(p + 1).copied();
        let (n, leaf) = match c {
            b'.' => {
                // `(\.)(_*)\d…`
                if matches!(next, Some(b'0'..=b'9' | b'_'))
                    && let Some(r) = self.number(p)
                {
                    return Some(r);
                }
                self.put(p, p + 1, DOT);
                // `(\.)\s*(?=<)`
                let q = skip_spaces(b, p + 1);
                if b.get(q) == Some(&b'<') {
                    return Some(self.type_args(q, &[]).0);
                }
                // `(\.)(class\b)?`
                if crate::has_at(b, p + 1, b"class") && boundary(b, p + 6) {
                    return Some(self.token(p + 1, 5, VAR_LANGUAGE));
                }
                return Some(p + 1);
            }
            b'(' => {
                if self.arrow_after(p) {
                    return Some(self.open(p, 1, LAMBDA_PARAM_LIST, PAREN_BEGIN));
                }
                return Some(self.open(p, 1, PARENS, PAREN_BEGIN));
            }
            b'"' if self.mark <= p && self.marked(p) => return Some(self.sql_string(p)),
            b'"' if crate::has_at(b, p + 1, b"\"\"") => {
                return Some(self.open(p, 3, TEXT_BLOCK, TB_BEGIN));
            }
            b'"' if regex_string(b, p) => {
                self.re = 0;
                return Some(self.open(p, 1, STRING_RE, DQ_BEGIN));
            }
            b'"' => return Some(self.open(p, 1, STRING, DQ_BEGIN)),
            b'\'' => return Some(self.open(p, 1, CHAR, SQ_BEGIN)),
            b'{' => return Some(self.open(p, 1, BLOCK, BLOCK_BEGIN)),
            b'=' if next != Some(b'=') => return Some(self.open(p, 1, RHS, ASSIGN)),
            b'|' | b'&' | b'^' | b'*' | b'/' | b'+' | b'-' if next == Some(b'=') => {
                return Some(self.open(p, 2, RHS, ASSIGN));
            }
            b':' if next == Some(b':') => {
                self.put(p, p + 2, DOUBLE_COLON);
                let q = skip_spaces(b, p + 2);
                if q == b.len() {
                    return Some(self.open(p + 2, 0, METHOD_REF, PLAIN));
                }
                return Some(match ident(b, q) {
                    Some(e) => self.token(q, e - q, FUNCTION_REF),
                    None => p + 2,
                });
            }
            b'?' | b':' => (1, TERNARY),
            b'<' if next == Some(b'<') => (2, BITSHIFT),
            b'>' if next == Some(b'>') => {
                (if b.get(p + 2) == Some(&b'>') { 3 } else { 2 }, BITSHIFT)
            }
            b'=' | b'!' if next == Some(b'=') => (2, COMPARISON),
            b'<' if matches!(next, Some(b'=' | b'>')) => (2, COMPARISON),
            b'>' if next == Some(b'=') => (2, COMPARISON),
            b'<' | b'>' => (1, COMPARISON),
            b'-' | b'+' if next == Some(c) => (2, INCDEC),
            b'-' | b'+' | b'*' | b'/' | b'%' => (1, ARITH),
            b'!' => (1, LOGICAL),
            b'&' | b'|' if next == Some(c) => (2, LOGICAL),
            b'~' | b'^' | b'&' | b'|' => (1, BITWISE),
            b',' => (1, COMMA),
            b';' => (1, TERMINATOR),
            _ => return None,
        };
        Some(self.token(p, n, leaf))
    }

    /// After a lambda's `->`, ending at `p`: its body, to `)`, `}` or `;`.
    fn lambda_body(&mut self, p: usize) -> usize {
        self.push(p, LAMBDA_BODY);
        p
    }

    /// After a constructor's `)`, ending at `p`, with its arguments' frame on top: an
    /// anonymous class's body where `{` follows.
    fn ctor_after(&mut self, p: usize) -> usize {
        let b = self.b;
        let q = skip_spaces(b, p);
        match b.get(q) {
            Some(b'{') => self.then(q, 1, &[ANON_BODY], BRACE_BEGIN),
            None => {
                self.set(p, &[CTOR_AFTER]);
                b.len()
            }
            Some(_) => self.pop_at(p),
        }
    }

    // types

    /// After a type in code, ending at `e` (`after-type`): its arguments, `.` and what follows,
    /// `[]`.
    fn code_type(&mut self, mut e: usize) -> usize {
        let b = self.b;
        loop {
            let q = skip_spaces(b, e);
            match b.get(q) {
                Some(b'<') => {
                    let (r, done) = self.type_args(q, &[ARRAY_BRACKETS]);
                    return if done { self.array_mods(r) } else { r };
                }
                Some(b'.') if b.get(q + 1) != Some(&b'.') => {
                    self.put(q, q + 1, DOT);
                    let r = skip_spaces(b, q + 1);
                    let Some(&c) = b.get(r) else {
                        return q + 1;
                    };
                    if c == b'<' {
                        return self.type_args(r, &[]).0;
                    }
                    if c == b'c' && crate::has_at(b, r, b"class") && boundary(b, r + 5) {
                        return self.token(r, 5, VAR_LANGUAGE);
                    }
                    if let Some(u) = upper_word(b, r) {
                        return self.token(r, u - r, CONSTANT);
                    }
                    match type_name(b, r) {
                        Some(t) => {
                            self.put(r, t, CLASS);
                            e = t;
                        }
                        None => return q + 1,
                    }
                }
                Some(b'[') => return self.array_mods(e),
                _ => return e,
            }
        }
    }

    /// `array-brackets`, each `[\s*]` past `e`'s blanks: where they end, else `e`.
    fn array_mods(&mut self, e: usize) -> usize {
        let mut e = e;
        while let Some(r) = self.brackets(skip_spaces(self.b, e)) {
            e = r;
        }
        e
    }

    /// `\[\s*\]` at `p`, a `storage.modifier.array`: where it ends.
    fn brackets(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if b.get(p) != Some(&b'[') {
            return None;
        }
        let q = skip_spaces(b, p + 1);
        (b.get(q) == Some(&b']')).then(|| self.token(p, q + 1 - p, ARRAY_MOD))
    }

    /// A package path at `p` (`package` then `object-type-fqn`'s type): each lower-case name a
    /// package, each `.` a dot, across blanks, then the type's name a class.
    fn path(&mut self, p: usize) -> Path {
        let b = self.b;
        let mut q = p;
        loop {
            q = skip_spaces(b, q);
            if q == b.len() {
                return Path::Cut;
            }
            if let Some(e) = lower_word(b, q) {
                q = self.token(q, e - q, PACKAGE);
            } else if b[q] == b'.' {
                q = self.token(q, 1, DOT);
            } else if let Some(e) = type_name(b, q) {
                self.put(q, e, CLASS);
                return Path::Typed(e);
            } else {
                return Path::Stopped(q);
            }
        }
    }

    /// A type's arguments at `q`, a `<`, on its line where they close there; else the contexts
    /// of bat's grammar for them (`below`, then `generic-type-invocation`) and the rest left to
    /// those. Where the scan goes on, and whether they closed.
    fn type_args(&mut self, q: usize, below: &[u8]) -> (usize, bool) {
        let (m, at) = (self.k.mark(self.out), self.at);
        if let Some(r) = self.args_inline(q, 0) {
            return (r, true);
        }
        // undone: the lexemes since `q`
        self.k.undo(self.out, m);
        self.at = at;
        for &c in below {
            self.push(q, c);
        }
        self.push(q, GENERIC_INVOCATION);
        (q, false)
    }

    /// `generic-type-invocation` at `q`, a `<`, read on the line: where it ends past its `>`,
    /// or `None` where it doesn't close there or holds what only bat's contexts follow.
    fn args_inline(&mut self, q: usize, depth: usize) -> Option<usize> {
        let b = self.b;
        if depth > 8 {
            return None;
        }
        self.put(q, q + 1, GENERIC_BEGIN);
        let mut p = q + 1;
        loop {
            p = skip_spaces(b, p);
            let &c = b.get(p)?;
            match c {
                b'>' => return Some(self.token(p, 1, GENERIC_END)),
                b',' => p = self.token(p, 1, COMMA),
                b'?' => {
                    p = self.token(p, 1, WILDCARD);
                    p = self.bounds_inline(p, depth)?;
                }
                b';' | b'{' | b'}' | b'(' | b')' | b'/' | b'%' | b'"' | b'\'' => return None,
                _ if c >= 0x80 || c == b'$' || c == b'_' || c.is_ascii_alphabetic() => {
                    p = self.arg_type(p, depth)?;
                }
                _ => p = next(b, p),
            }
        }
    }

    /// `object-and-array-types` at `p` in a type's arguments: where it ends.
    fn arg_type(&mut self, p: usize, depth: usize) -> Option<usize> {
        let b = self.b;
        let n = self.name(p);
        if n.group() == K_ILLEGAL {
            return None;
        }
        if package_ahead(b, p) {
            let mut q = p;
            loop {
                q = skip_spaces(b, q);
                let &c = b.get(q)?;
                if let Some(e) = lower_word(b, q) {
                    q = self.token(q, e - q, PACKAGE);
                } else if c == b'.' {
                    q = self.token(q, 1, DOT);
                } else {
                    break;
                }
            }
            return match type_name(b, q) {
                Some(e) => {
                    self.put(q, e, CLASS);
                    self.obj_after_inline(e, depth)
                }
                None => Some(q),
            };
        }
        if n.group() == K_PRIMITIVE {
            let r = skip_spaces(b, n.e);
            if b.get(r) == Some(&b'[') {
                self.put(p, n.e, PRIMITIVE);
                return Some(self.array_mods(n.e));
            }
        }
        if boundary(b, p)
            && let Some(e) = type_name(b, p)
        {
            self.put(p, e, CLASS);
            return self.obj_after_inline(e, depth);
        }
        // no pattern takes the name
        Some(n.e.max(next(b, p)))
    }

    /// `after-object-and-array-types` from `e`, in a type's arguments.
    fn obj_after_inline(&mut self, mut e: usize, depth: usize) -> Option<usize> {
        let b = self.b;
        loop {
            let q = skip_spaces(b, e);
            match b.get(q) {
                Some(b'<') => {
                    let r = self.args_inline(q, depth + 1)?;
                    return Some(self.array_mods(r));
                }
                Some(b'.') if b.get(q + 1) != Some(&b'.') => {
                    self.put(q, q + 1, DOT);
                    let r = skip_spaces(b, q + 1);
                    match type_name(b, r) {
                        Some(t) => {
                            self.put(r, t, CLASS);
                            e = t;
                        }
                        None => return Some(q + 1),
                    }
                }
                Some(b'[') => return Some(self.array_mods(e)),
                None => return None,
                _ => return Some(e),
            }
        }
    }

    /// A wildcard's bounds from `p`, past its `?`: `extends` or `super` and their types, to the
    /// `,` that ends them (taken) or the `>` (left).
    fn bounds_inline(&mut self, mut p: usize, depth: usize) -> Option<usize> {
        let b = self.b;
        loop {
            p = skip_spaces(b, p);
            let &c = b.get(p)?;
            match c {
                b',' => return Some(self.token(p, 1, COMMA)),
                b'>' => return Some(p),
                b';' | b'{' | b'}' | b'(' | b')' | b'/' | b'%' | b'"' | b'\'' => return None,
                b'e' if crate::has_at(b, p, b"extends") && boundary(b, p + 7) => {
                    p = self.token(p, 7, EXTENDS_KW);
                    p = self.ref_inline(p, depth)?;
                    loop {
                        let q = skip_spaces(b, p);
                        if b.get(q) != Some(&b'&') {
                            break;
                        }
                        p = self.token(q, 1, BOUNDS_AND);
                        p = self.ref_inline(p, depth)?;
                    }
                }
                b's' if crate::has_at(b, p, b"super") && boundary(b, p + 5) => {
                    p = self.token(p, 5, SUPER_KW);
                    p = self.ref_inline(p, depth)?;
                }
                _ => p = next(b, p),
            }
        }
    }

    /// `object-type-reference` from `p`, in a wildcard's bounds: where it ends.
    fn ref_inline(&mut self, p: usize, depth: usize) -> Option<usize> {
        let b = self.b;
        let mut q = skip_spaces(b, p);
        b.get(q)?;
        if package_ahead(b, q) {
            loop {
                q = skip_spaces(b, q);
                let &c = b.get(q)?;
                if let Some(e) = lower_word(b, q) {
                    q = self.token(q, e - q, PACKAGE);
                } else if c == b'.' {
                    q = self.token(q, 1, DOT);
                } else {
                    break;
                }
            }
        }
        let Some(mut e) = type_name(b, q) else {
            return Some(q);
        };
        self.put(q, e, CLASS);
        loop {
            let r = skip_spaces(b, e);
            match b.get(r) {
                Some(b'<') => return self.args_inline(r, depth + 1),
                Some(b'.') => {
                    self.put(r, r + 1, DOT);
                    let s = skip_spaces(b, r + 1);
                    match type_name(b, s) {
                        Some(t) => {
                            self.put(s, t, CLASS);
                            e = t;
                        }
                        None => return Some(r + 1),
                    }
                }
                None => return None,
                _ => return Some(e),
            }
        }
    }

    /// After a type in a declaration, ending at `e` (`after-object-and-array-types`): its
    /// arguments, `.` and a type, `[]`. Where it ends, and whether it did there: where its
    /// arguments don't close on the line, bat's contexts take them, `below` under them.
    fn decl_after(&mut self, mut e: usize, below: Option<u8>) -> (usize, bool) {
        let b = self.b;
        loop {
            let q = skip_spaces(b, e);
            match b.get(q) {
                Some(b'<') => {
                    let (r, done) = match below {
                        Some(c) => self.type_args(q, &[c, ARRAY_BRACKETS]),
                        None => self.type_args(q, &[ARRAY_BRACKETS]),
                    };
                    return if done {
                        (self.array_mods(r), true)
                    } else {
                        (r, false)
                    };
                }
                Some(b'.') if b.get(q + 1) != Some(&b'.') => {
                    self.put(q, q + 1, DOT);
                    let r = skip_spaces(b, q + 1);
                    match type_name(b, r) {
                        Some(t) => {
                            self.put(r, t, CLASS);
                            e = t;
                        }
                        None => return (q + 1, true),
                    }
                }
                Some(b'[') => return (self.array_mods(e), true),
                _ => return (e, true),
            }
        }
    }

    // `new`

    /// `object-type-instantiation` at `n`, past `new`, with [`INSTANTIATION`] on top.
    fn instantiation(&mut self, n: Name) -> usize {
        let b = self.b;
        let p = n.p;
        if n.group() == K_PRIMITIVE {
            return self.then(p, n.e - p, &[ARRAY_DEF], PRIMITIVE);
        }
        if package_ahead(b, p) {
            return match self.path(p) {
                Path::Typed(t) => self.after_new(t),
                Path::Stopped(q) => self.after_new(q),
                Path::Cut => {
                    self.set(b.len(), &[AFTER_NEW_TYPE, TYPE_FQN]);
                    b.len()
                }
            };
        }
        match type_name(b, p) {
            Some(e) => {
                self.put(p, e, CLASS);
                self.after_new(e)
            }
            None => self.pop_at(p),
        }
    }

    /// `after-object-type-instantiation` past `e`'s blanks, with [`INSTANTIATION`] or
    /// [`AFTER_NEW_TYPE`] on top: what follows a type after `new`.
    fn after_new(&mut self, e: usize) -> usize {
        let b = self.b;
        let q = skip_spaces(b, e);
        let Some(&c) = b.get(q) else {
            self.set(e, &[AFTER_NEW_TYPE]);
            return b.len();
        };
        match c {
            b'[' => {
                self.set(q, &[ARRAY_DEF]);
                q
            }
            b'(' => self.then(q, 1, &[CTOR_ARGS], PAREN_BEGIN),
            b'<' if b.get(q + 1) == Some(&b'>') => {
                self.put(q, q + 2, DIAMOND);
                self.construction(q + 2, false)
            }
            b'<' => {
                self.set(q, &[AFTER_NEW_TYPE]);
                let (r, done) = self.type_args(q, &[]);
                if done {
                    return self.construction(r, true);
                }
                r
            }
            b'.' => {
                self.put(q, q + 1, DOT);
                let r = skip_spaces(b, q + 1);
                match type_name(b, r) {
                    Some(t) => {
                        self.put(r, t, CLASS);
                        self.after_new(t)
                    }
                    None => self.pop_at(q + 1),
                }
            }
            _ => self.pop_at(q),
        }
    }

    /// `object-type-instantiation-construction` past `p`: the constructor's `(`; past a type's
    /// arguments (`arrays`), or an array's `[`.
    fn construction(&mut self, p: usize, arrays: bool) -> usize {
        let b = self.b;
        let q = skip_spaces(b, p);
        match b.get(q) {
            Some(b'(') => self.then(q, 1, &[CTOR_ARGS], PAREN_BEGIN),
            Some(b'[') if arrays => {
                self.set(q, &[ARRAY_DEF]);
                q
            }
            None => {
                self.set(p, &[AFTER_NEW_TYPE]);
                b.len()
            }
            Some(_) => self.pop_at(q),
        }
    }

    // annotations

    /// `annotations` at `p`, an `@`: its name, and its arguments' `(` where it follows.
    fn annotation(&mut self, p: usize) -> usize {
        let b = self.b;
        self.push(p, ANNOT_PARAMS);
        self.put(p, p + 1, ANNOT_MARK);
        let mut q = skip_spaces(b, p + 1);
        // `annotation-type-reference`: a package, then the name
        if package_ahead(b, q) {
            loop {
                q = skip_spaces(b, q);
                let Some(&c) = b.get(q) else {
                    self.push(b.len(), ANNOT_PATH);
                    return b.len();
                };
                if let Some(e) = lower_word(b, q) {
                    q = self.token(q, e - q, ANNOTATION_PKG);
                } else if c == b'.' {
                    q = self.token(q, 1, DOT);
                } else {
                    break;
                }
            }
        }
        let Some(mut e) = type_name(b, q) else {
            return q.min(b.len());
        };
        loop {
            self.annotation_name(q, e);
            let r = skip_spaces(b, e);
            if b.get(r) != Some(&b'.') {
                break;
            }
            self.put(r, r + 1, DOT);
            q = skip_spaces(b, r + 1);
            match type_name(b, q) {
                Some(t) => e = t,
                None => return q.min(b.len()),
            }
        }
        // `annotation-parameters`
        let r = skip_spaces(b, e);
        match b.get(r) {
            None | Some(b'/' | b'%') => e,
            Some(b'(') => {
                let to = if self.top() == ANNOT_PARAMS {
                    ANNOT_ARGS
                } else {
                    ANNOT_ARGS_SQL
                };
                self.then(r, 1, &[to], PAREN_BEGIN)
            }
            Some(_) => self.pop_at(r),
        }
    }

    /// An annotation's name `b[p..e]`: `variable.annotation`, and SQL's annotations
    /// ([`ANNOT_PARAMS_SQL`]) and marker told apart, its parameters' frame the top or the one
    /// below it.
    pub(super) fn annotation_name(&mut self, p: usize, e: usize) {
        let b = self.b;
        self.put(p, e, ANNOTATION);
        let at = self.depth - 1 - usize::from(!is_frame(self.top()));
        if matches!(self.stack[at], ANNOT_PARAMS | ANNOT_PARAMS_SQL) {
            self.stack[at] = if sql::jvm_annotation(&b[p..e]) {
                ANNOT_PARAMS_SQL
            } else {
                ANNOT_PARAMS
            };
        }
        if &b[p..e] == b"Language"
            && let Some(end) = sql::language_annotation(b, e)
        {
            self.mark = end;
        }
    }

    // a class's body

    /// `class-body`'s words, in its patterns' order.
    fn body_word(&mut self, n: Name) -> usize {
        let b = self.b;
        let (p, e) = (n.p, n.e);
        let g = n.group();
        if g == K_CLASS || (g == K_MODIFIER && self.class_ahead(p)) {
            return self.enter(p, &[CLASS_TYPE]);
        }
        let q = skip_spaces(b, e);
        let after = b.get(q).copied();
        match g {
            K_VOID => {
                self.put(p, e, VOID);
                self.push(e, METHOD);
                return self.method_name(q).unwrap_or(e);
            }
            K_MODIFIER if !matches!(after, Some(b'(' | b'.')) => {
                return self.token(p, e - p, MODIFIER);
            }
            K_PRIMITIVE if !matches!(after, Some(b'(' | b'.')) => {
                self.put(p, e, PRIMITIVE);
                let r = self.array_mods(e);
                return self.member_name(r);
            }
            _ => {}
        }
        // `method`: a name before its `(`
        if after == Some(b'(') {
            self.push(p, METHOD);
            return self.method_name(p).unwrap_or(p);
        }
        // a type, after its package
        if after == Some(b'.') && package_ahead(b, p) {
            return match self.path(p) {
                Path::Typed(t) => self.member_type(t),
                Path::Stopped(q) => self.member_name(q),
                Path::Cut => {
                    self.push(b.len(), MEMBER);
                    self.push(b.len(), AFTER_OBJ_ARRAY);
                    self.push(b.len(), TYPE_FQN);
                    b.len()
                }
            };
        }
        if b[p].is_ascii_uppercase() || (b[p] >= 0x80 && is_upper(b, p)) {
            let t = if n.plain() {
                e
            } else {
                type_name(b, p).unwrap_or(e)
            };
            self.put(p, t, CLASS);
            return self.member_type(t);
        }
        match g {
            K_PRIMITIVE => {
                self.put(p, e, PRIMITIVE);
                let r = self.array_mods(e);
                self.member_name(r)
            }
            K_LANGUAGE => self.token(p, e - p, LANGUAGE),
            K_SPECIAL => self.token(p, e - p, VAR_LANGUAGE),
            K_MODIFIER => self.token(p, e - p, MODIFIER),
            _ => e,
        }
    }

    /// After a member's type's name, ending at `t`: the rest of its type, then its name.
    fn member_type(&mut self, t: usize) -> usize {
        match self.decl_after(t, Some(MEMBER)) {
            (r, true) => self.member_name(r),
            (r, false) => r,
        }
    }

    /// A member's name past its type (`field-or-method`), at the first byte past `r`'s blanks
    /// on the line: a method's name before its `(`, a field's; where the line ends there, the
    /// next line's ([`MEMBER`]).
    fn member_name(&mut self, r: usize) -> usize {
        let b = self.b;
        let q = skip_spaces(b, r);
        let Some(&c) = b.get(q) else {
            self.push(r, MEMBER);
            return b.len();
        };
        if matches!(c, b'/' | b'%') {
            self.push(r, MEMBER);
            return q;
        }
        if self.call_ahead(q) {
            self.push(q, METHOD);
            return self.method_name(q).unwrap_or(q);
        }
        self.push(q, FIELD);
        q
    }

    /// `method`'s first pattern at `r`, a method's name before its `(`.
    fn method_name(&mut self, r: usize) -> Option<usize> {
        let b = self.b;
        let &c = b.get(r)?;
        if matches!(c, b'/' | b'%') {
            return None;
        }
        let e = ident(b, r)?;
        if matches!(c, b'g' | b'c') && self.name(r).group() == K_ILLEGAL {
            return None;
        }
        if b.get(skip_spaces(b, e)) != Some(&b'(') {
            return None;
        }
        let leaf = if type_name(b, r) == Some(e) {
            ENTITY_CTOR
        } else {
            ENTITY_FUNCTION
        };
        Some(self.token(r, e - r, leaf))
    }

    /// `method`'s words: a name before `(`, `throws`, `default`.
    fn method_word(&mut self, n: Name) -> usize {
        let (p, e) = (n.p, n.e);
        if let Some(r) = self.method_name(p) {
            return r;
        }
        match n.id {
            W_THROWS => {
                self.put(p, e, THROWS_KW);
                self.push(e, THROWS_LIST);
                self.push(e, TYPE_REF);
                e
            }
            W_DEFAULT => self.open(p, e - p, ANNOT_DEFAULT, DEFAULT_DECL),
            _ => self.pop_at(p),
        }
    }

    /// `field`'s words: a field's name, or the next member's start.
    fn field_word(&mut self, n: Name) -> usize {
        let b = self.b;
        let p = n.p;
        if matches!(n.group(), K_MODIFIER | K_PRIMITIVE | K_VOID) {
            return self.pop_at(p);
        }
        let end = if n.plain() {
            let leaf = if n.capitals(b) {
                ENTITY_CONSTANT
            } else {
                PLAIN
            };
            self.token(p, n.e - p, leaf)
        } else if let Some(u) = upper_word(b, p) {
            self.token(p, u - p, ENTITY_CONSTANT)
        } else {
            match ident(b, p) {
                Some(e) => self.token(p, e - p, PLAIN),
                None => return self.pop_at(p),
            }
        };
        // `static-assignment` after the name
        let q = skip_spaces(b, end);
        match b.get(q) {
            Some(b'=') => {
                self.put(q, q + 1, ASSIGN);
                self.push(q + 1, STATIC_RHS);
                q + 1
            }
            Some(b'[') | None | Some(b'/' | b'%') => {
                self.push(end, FIELD_NAMED);
                end
            }
            Some(_) => end,
        }
    }

    fn body_punct(&mut self, p: usize, c: u8, top: u8) -> usize {
        let b = self.b;
        match c {
            b'}' => {
                let leaf = if top == ANON_BODY {
                    BRACE_END
                } else {
                    BLOCK_END
                };
                self.close(p, 1, leaf)
            }
            b'@' => {
                if crate::has_at(b, p, b"@interface") && self.class_ahead(p) {
                    return self.enter(p, &[CLASS_TYPE]);
                }
                self.annotation(p)
            }
            b'{' => self.open(p, 1, BLOCK, BLOCK_BEGIN),
            b',' => self.token(p, 1, COMMA),
            b';' => self.token(p, 1, TERMINATOR),
            b'<' => self.enter(p, &[GENERIC_DECL]),
            b'.' => self.number(p).unwrap_or(p + 1),
            b'0'..=b'9' => self.number_or_word(p),
            _ => next(b, p),
        }
    }

    // an enum's constants

    /// A constant's name at `p`, a word of `\w`: `constant.other.enum`.
    fn enum_constant(&mut self, p: usize) -> usize {
        let b = self.b;
        let e = word_end(b, p);
        if e == p {
            return next(b, p);
        }
        self.put(p, e, ENUM_NAME);
        self.push(e, ENUM_CONSTANT);
        e
    }

    // parameters

    fn param_word(&mut self, n: Name) -> usize {
        let b = self.b;
        let (p, e) = (n.p, n.e);
        if n.id == W_FINAL {
            return self.token(p, e - p, MODIFIER);
        }
        if n.group() == K_PRIMITIVE {
            self.put(p, e, PRIMITIVE);
            return self.array_mods(e);
        }
        if b[p].is_ascii_lowercase() && b.get(skip_spaces(b, e)) == Some(&b'.') {
            if package_ahead(b, p) {
                return match self.path(p) {
                    Path::Typed(t) => self.decl_after(t, None).0,
                    Path::Stopped(q) => q,
                    Path::Cut => self.open(b.len(), 0, OBJ_ARRAY_PATH, PLAIN),
                };
            }
        } else if b[p].is_ascii_uppercase() || (b[p] >= 0x80 && is_upper(b, p)) {
            let t = if n.plain() {
                e
            } else {
                type_name(b, p).unwrap_or(e)
            };
            self.put(p, t, CLASS);
            return self.decl_after(t, None).0;
        }
        let e = if n.plain() {
            e
        } else {
            match ident(b, p) {
                Some(e) => e,
                None => return next(b, p),
            }
        };
        self.put(p, e, PARAMETER);
        self.array_mods(e)
    }

    fn param_punct(&mut self, p: usize, c: u8, top: u8) -> usize {
        let b = self.b;
        match c {
            b')' if top == LAMBDA_PARAM_LIST => self.then(p, 1, &[LAMBDA_ARROW], PAREN_END),
            b')' => self.close(p, 1, PAREN_END),
            b'@' => self.annotation(p),
            b'.' if crate::has_at(b, p, b"...") => self.token(p, 3, VARIADIC),
            b',' => self.token(p, 1, COMMA),
            b'|' if top == CATCH_PARAMS => self.token(p, 1, BAR),
            _ if top == METHOD_PARAMS => self.close(p, char_len(c), MISSING_PARAM_END),
            _ => next(b, p),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ids_name_their_words() {
        let ids = [
            (W_FINAL, &b"final"[..]),
            (W_DEFAULT, b"default"),
            (W_PACKAGE, b"package"),
            (W_IMPORT, b"import"),
            (W_MODULE, b"module"),
            (W_OPEN, b"open"),
            (W_THROWS, b"throws"),
            (W_UUID, b"UUID"),
            (W_URI, b"URI"),
        ];
        for (id, w) in ids {
            assert_eq!(KEYWORDS[usize::from(id) - 1].0, w);
            assert_eq!(WORDS.get(w), u16::from(id));
        }
        for (i, (w, _, _)) in KEYWORDS.iter().enumerate() {
            assert_eq!(
                usize::from(WORDS.get(w)),
                i + 1,
                "{}",
                String::from_utf8_lossy(w)
            );
        }
    }
}
