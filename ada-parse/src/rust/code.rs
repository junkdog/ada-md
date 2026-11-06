//! Rust's code on the C family's core ([`crate::clike`], X-056): the contexts made of bat's
//! `statements` (the top level, a block, parentheses, brackets, a macro's brackets, a closure's
//! body) and the two it pushes after an operator (`after-operator`, `try-closure`), run by a loop
//! of their own.
//!
//! A blank run is passed whole; a name is read once with the classes of its bytes
//! ([`clike::class_run`]) and looked up once among every word the frames ask about ([`WORDS`]:
//! the keywords by leaf, the primitive and `support.type` names, the item words, the macros whose
//! first argument is a format string, SQL's calls), and its shape (a constant's capitals, a call's
//! lowercase, a C type's) is told from those classes, not read again. Punctuation is one `match`
//! on its byte. The frames are the grammar's contexts, the same ids on the same stack, so what a
//! line leaves the next is what bat's patterns left; the headers, types, strings, comments and
//! attributes stay on them ([`Scan::at`]).

use super::*;
use crate::clike::{self, CLASS, DIGIT, HIGH, Ident, LOWER, SPACE, UPPER, WORD};
use crate::words::{Words, text_len};

/// A name's bytes: `\w`, every byte past ASCII among them.
const NAME: u8 = WORD | HIGH;

// What a word is, by group.
const G_KEYWORD: u8 = 1;
const G_PRIMITIVE: u8 = 2;
const G_SUPPORT: u8 = 3;
/// A word a frame asks for by its id.
const G_ASKED: u8 = 4;
/// A call SQL's may be ([`sql_call`]).
const G_SQL: u8 = 5;

const N: usize = 139;

/// Every word the frames tell apart, its group, and a keyword's leaf; its id is its index plus
/// one. The item words come first, so that an id up to [`W_IMPL`] is one.
const LIST: [(&[u8], u8, u16); N] = [
    (b"pub", G_KEYWORD, MODIFIER),
    (b"mod", G_KEYWORD, MODULE),
    (b"fn", G_KEYWORD, FN),
    (b"struct", G_KEYWORD, STRUCT),
    (b"type", G_KEYWORD, TYPE),
    (b"trait", G_KEYWORD, TRAIT),
    (b"enum", G_KEYWORD, ENUM),
    (b"impl", G_KEYWORD, IMPL),
    (b"return", G_KEYWORD, CONTROL),
    (b"macro_rules", G_ASKED, 0),
    (b"b", G_ASKED, 0),
    (b"br", G_ASKED, 0),
    (b"r", G_ASKED, 0),
    (b"self", G_ASKED, 0),
    (b"super", G_ASKED, 0),
    (b"format", G_ASKED, 0),
    (b"print", G_ASKED, 0),
    (b"println", G_ASKED, 0),
    (b"eprint", G_ASKED, 0),
    (b"eprintln", G_ASKED, 0),
    (b"write", G_ASKED, 0),
    (b"writeln", G_ASKED, 0),
    (b"as", G_KEYWORD, OPERATOR),
    (b"alignof", G_KEYWORD, INVALID),
    (b"abstract", G_KEYWORD, INVALID),
    (b"break", G_KEYWORD, CONTROL),
    (b"box", G_KEYWORD, OPERATOR),
    (b"become", G_KEYWORD, INVALID),
    (b"const", G_KEYWORD, STORAGE),
    (b"crate", G_KEYWORD, KEYWORD),
    (b"continue", G_KEYWORD, CONTROL),
    (b"do", G_KEYWORD, INVALID),
    (b"else", G_KEYWORD, CONTROL),
    (b"extern", G_KEYWORD, KEYWORD),
    (b"for", G_KEYWORD, CONTROL),
    (b"false", G_KEYWORD, LANGUAGE),
    (b"final", G_KEYWORD, INVALID),
    (b"if", G_KEYWORD, CONTROL),
    (b"in", G_KEYWORD, OPERATOR),
    (b"let", G_KEYWORD, STORAGE),
    (b"loop", G_KEYWORD, CONTROL),
    (b"mut", G_KEYWORD, MODIFIER),
    (b"move", G_KEYWORD, MODIFIER),
    (b"match", G_KEYWORD, CONTROL),
    (b"macro", G_KEYWORD, INVALID),
    (b"offsetof", G_KEYWORD, INVALID),
    (b"override", G_KEYWORD, INVALID),
    (b"priv", G_KEYWORD, INVALID),
    (b"proc", G_KEYWORD, INVALID),
    (b"pure", G_KEYWORD, INVALID),
    (b"ref", G_KEYWORD, MODIFIER),
    (b"static", G_KEYWORD, STORAGE),
    (b"sizeof", G_KEYWORD, INVALID),
    (b"true", G_KEYWORD, LANGUAGE),
    (b"typeof", G_KEYWORD, INVALID),
    (b"use", G_KEYWORD, KEYWORD),
    (b"unsafe", G_KEYWORD, MODIFIER),
    (b"unsized", G_KEYWORD, INVALID),
    (b"virtual", G_KEYWORD, INVALID),
    (b"where", G_KEYWORD, KEYWORD),
    (b"while", G_KEYWORD, CONTROL),
    (b"yield", G_KEYWORD, INVALID),
    (b"bool", G_PRIMITIVE, 0),
    (b"char", G_PRIMITIVE, 0),
    (b"f32", G_PRIMITIVE, 0),
    (b"f64", G_PRIMITIVE, 0),
    (b"i8", G_PRIMITIVE, 0),
    (b"i16", G_PRIMITIVE, 0),
    (b"i32", G_PRIMITIVE, 0),
    (b"i64", G_PRIMITIVE, 0),
    (b"i128", G_PRIMITIVE, 0),
    (b"isize", G_PRIMITIVE, 0),
    (b"str", G_PRIMITIVE, 0),
    (b"u8", G_PRIMITIVE, 0),
    (b"u16", G_PRIMITIVE, 0),
    (b"u32", G_PRIMITIVE, 0),
    (b"u64", G_PRIMITIVE, 0),
    (b"u128", G_PRIMITIVE, 0),
    (b"usize", G_PRIMITIVE, 0),
    (b"Self", G_PRIMITIVE, 0),
    (b"AsRef", G_SUPPORT, 0),
    (b"AsMut", G_SUPPORT, 0),
    (b"Box", G_SUPPORT, 0),
    (b"Copy", G_SUPPORT, 0),
    (b"Clone", G_SUPPORT, 0),
    (b"Drop", G_SUPPORT, 0),
    (b"Default", G_SUPPORT, 0),
    (b"DoubleEndedIterator", G_SUPPORT, 0),
    (b"Eq", G_SUPPORT, 0),
    (b"Extend", G_SUPPORT, 0),
    (b"ExactSizeIterator", G_SUPPORT, 0),
    (b"Err", G_SUPPORT, 0),
    (b"Fn", G_SUPPORT, 0),
    (b"FnMut", G_SUPPORT, 0),
    (b"FnOnce", G_SUPPORT, 0),
    (b"From", G_SUPPORT, 0),
    (b"Into", G_SUPPORT, 0),
    (b"Iterator", G_SUPPORT, 0),
    (b"IntoIterator", G_SUPPORT, 0),
    (b"None", G_SUPPORT, 0),
    (b"Ord", G_SUPPORT, 0),
    (b"Option", G_SUPPORT, 0),
    (b"Ok", G_SUPPORT, 0),
    (b"PartialEq", G_SUPPORT, 0),
    (b"PartialOrd", G_SUPPORT, 0),
    (b"Result", G_SUPPORT, 0),
    (b"Send", G_SUPPORT, 0),
    (b"Sized", G_SUPPORT, 0),
    (b"Sync", G_SUPPORT, 0),
    (b"Some", G_SUPPORT, 0),
    (b"SliceConcatExt", G_SUPPORT, 0),
    (b"String", G_SUPPORT, 0),
    (b"ToOwned", G_SUPPORT, 0),
    (b"ToString", G_SUPPORT, 0),
    (b"Vec", G_SUPPORT, 0),
    (b"query", G_SQL, 0),
    (b"query_as", G_SQL, 0),
    (b"query_scalar", G_SQL, 0),
    (b"query_with", G_SQL, 0),
    (b"query_as_with", G_SQL, 0),
    (b"query_scalar_with", G_SQL, 0),
    (b"query_row", G_SQL, 0),
    (b"query_one", G_SQL, 0),
    (b"query_opt", G_SQL, 0),
    (b"query_raw", G_SQL, 0),
    (b"query_typed", G_SQL, 0),
    (b"query_unchecked", G_SQL, 0),
    (b"query_scalar_unchecked", G_SQL, 0),
    (b"query_as_unchecked", G_SQL, 0),
    (b"sql_query", G_SQL, 0),
    (b"sql", G_SQL, 0),
    (b"simple_query", G_SQL, 0),
    (b"push_sql", G_SQL, 0),
    (b"prepare", G_SQL, 0),
    (b"prepare_cached", G_SQL, 0),
    (b"prepare_typed", G_SQL, 0),
    (b"execute", G_SQL, 0),
    (b"execute_batch", G_SQL, 0),
    (b"batch_execute", G_SQL, 0),
];

// The ids of the words the frames ask for by name.
const W_PUB: u8 = id_of(b"pub");
const W_MOD: u8 = id_of(b"mod");
const W_FN: u8 = id_of(b"fn");
const W_STRUCT: u8 = id_of(b"struct");
const W_TYPE: u8 = id_of(b"type");
const W_TRAIT: u8 = id_of(b"trait");
const W_ENUM: u8 = id_of(b"enum");
const W_IMPL: u8 = id_of(b"impl");
const W_RETURN: u8 = id_of(b"return");
const W_MACRO_RULES: u8 = id_of(b"macro_rules");
const W_B: u8 = id_of(b"b");
const W_R: u8 = id_of(b"r");
const W_SELF: u8 = id_of(b"self");
const W_SUPER: u8 = id_of(b"super");
const W_FORMAT: u8 = id_of(b"format");
const W_EPRINTLN: u8 = id_of(b"eprintln");
const W_WRITE: u8 = id_of(b"write");
const W_WRITELN: u8 = id_of(b"writeln");
const W_REF: u8 = id_of(b"ref");
const W_MUT: u8 = id_of(b"mut");
const W_CONST: u8 = id_of(b"const");
const W_AS: u8 = id_of(b"as");
const _: () = assert!(W_IMPL == 8 && W_B + 2 == W_R && W_FORMAT + 4 == W_EPRINTLN);

/// A listed word's id, at compile time.
const fn id_of(w: &[u8]) -> u8 {
    let mut i = 0;
    while i < N {
        let x = LIST[i].0;
        if x.len() == w.len() {
            let mut k = 0;
            while k < w.len() && x[k] == w[k] {
                k += 1;
            }
            if k == w.len() {
                return i as u8 + 1;
            }
        }
        i += 1;
    }
    panic!("not listed")
}

const WORD_LIST: [&[u8]; N] = {
    let mut w: [&[u8]; N] = [b""; N];
    let mut i = 0;
    while i < N {
        w[i] = LIST[i].0;
        i += 1;
    }
    w
};
static WORDS: Words<512, N, { text_len(&WORD_LIST) }> = Words::new(&WORD_LIST);

/// Each id's group and leaf, the id's own index (0 for none).
static INFO: [(u8, u16); N + 1] = {
    let mut t = [(0, 0); N + 1];
    let mut i = 0;
    while i < N {
        t[i + 1] = (LIST[i].1, LIST[i].2);
        i += 1;
    }
    t
};

/// Whether the frames run `top`: the contexts made of `statements`, `after-operator`,
/// `try-closure`, a path's segments (`no-type-names`) and the type frames ([`TYPES`]).
#[inline(always)]
pub(super) fn is_code(top: u8) -> bool {
    top < 64 && (FRAMES >> top) & 1 != 0
}

/// [`is_code`]'s contexts, a bit each.
const FRAMES: u64 = {
    let mut m = 0;
    let mut c = MAIN;
    while c <= CLOSURE_BODY {
        m |= 1 << c;
        c += 1;
    }
    m | 1 << AFTER_OP | 1 << TRY_CLOSURE | 1 << NO_TYPE_NAMES | TYPES
};

/// The type frames ([`Scan::type_frame`]): a function's head after its name, its parameters,
/// its return and return type; a generic's arguments; a struct's fields and a field's type; a
/// type's group, a function type and its return.
const TYPES: u64 = 1 << FN_GENERIC
    | 1 << FN_PARAMS
    | 1 << FN_RETURN
    | 1 << RETURN_TYPE
    | 1 << GENERIC
    | 1 << GENERIC_ARGS
    | 1 << GENERIC_NESTED
    | 1 << GENERIC_ARRAY
    | 1 << STRUCT_FIELDS
    | 1 << FIELD_TYPE
    | 1 << TYPE_GROUP
    | 1 << FN_TYPE
    | 1 << FN_TYPE_RETURN;
const _: () = assert!(NO_TYPE_NAMES < 64);

/// `[\p{Ll}_][\p{Ll}\p{Nd}_]*`, a whole name of these classes whose first byte is `first`.
#[inline(always)]
fn lower(first: u8, bits: u8) -> bool {
    matches!(first, b'a'..=b'z' | b'_') && bits & (UPPER | HIGH) == 0
}

/// A name read at the scan's place: where it ends, its bytes' classes, its word's id.
#[derive(Clone, Copy)]
struct NameAt {
    e: usize,
    bits: u8,
    id: u8,
}

impl Scan<'_, '_> {
    /// The name at `p`, whose first byte is a name's.
    #[inline(always)]
    fn name(&self, p: usize) -> NameAt {
        let b = self.b;
        let Ident { end, bits } = clike::class_run(b, p, NAME);
        let id = if bits & HIGH == 0 {
            WORDS.get(&b[p..end]) as u8
        } else {
            0
        };
        NameAt { e: end, bits, id }
    }

    /// Bytes `s..e` are `leaf` (not 0), in a frame, where the text is plain: the frames run
    /// outside attributes, so no text before it is filled and its kind is the leaf.
    #[inline(always)]
    fn tok(&mut self, s: usize, e: usize, leaf: u16) -> usize {
        self.merge(s, e, leaf);
        self.at = e;
        e
    }

    /// A closer at `p`, `leaf`, that ends the frame.
    #[inline(always)]
    fn tok_close(&mut self, p: usize, leaf: u16) -> usize {
        self.tok(p, p + 1, leaf);
        self.pop_code();
        p + 1
    }

    /// Frame `c` pushed from a frame: no text to fill before it, nothing to count, its text
    /// plain as the frame's.
    #[inline(always)]
    fn push_code(&mut self, c: u8) {
        if self.depth == MAX {
            crate::stack::drop_bottom(&mut self.stack, &mut self.depth, &mut self.exact);
            self.count();
        }
        self.stack[self.depth] = c;
        self.depth += 1;
        self.cur = c;
    }

    /// The frame on top popped: what is below a frame is a frame or a context of the code
    /// around it, never a string's or a comment's, so the text stays plain.
    #[inline(always)]
    fn pop_code(&mut self) {
        if self.depth > 1 {
            self.depth -= 1;
            self.cur = self.stack[self.depth - 1];
        }
    }

    /// Where bat's grammar pushes `after-operator` or `try-closure` (`c`) from a frame: the
    /// next token may start a closure (and after an operator, a generic), told by
    /// [`Scan::opened`] when it comes, without a context of its own.
    #[inline(always)]
    fn push_ahead_code(&mut self, _at: usize, c: u8) {
        self.opened = if c == AFTER_OP {
            OPENED_AFTER
        } else {
            OPENED_TRY
        };
    }

    /// `n` bytes at `p` are `leaf`, after an operator.
    #[inline(always)]
    fn after_op_code(&mut self, p: usize, n: usize, leaf: u16) -> usize {
        self.tok(p, p + n, leaf);
        self.push_ahead_code(p + n, AFTER_OP);
        p + n
    }

    /// A path's segment `p..e` before `::`, the word `id`: `self`, `super` or a module's name;
    /// the next segment's frame pushed after the `::`.
    fn path_code(&mut self, p: usize, e: usize, id: u8) -> usize {
        if self.b.get(e + 2) == Some(&b'<') {
            self.sql_name(p, e);
        }
        let leaf = match id {
            W_SELF => PATH_SELF,
            W_SUPER => PATH_SUPER,
            _ => PATH,
        };
        self.tok(p, e, leaf);
        self.tok(e, e + 2, PATH_ACCESSOR);
        self.push_code(NO_TYPE_NAMES);
        e + 2
    }

    /// The frames' run from `p`, while one is on top: where it stopped, at the line's end or
    /// at the start of a context on bat's patterns.
    pub(super) fn code_run(&mut self, mut p: usize) -> usize {
        let b = self.b;
        loop {
            let top = self.top();
            // a tuple struct's fields end at the first `)` in any context, `at`'s; in an
            // attribute, every kind is the attribute's
            if !is_code(top) || (self.tuple | self.annot) != 0 {
                return p;
            }
            if p >= b.len() {
                // `(?=[};)\]\n])`, and the empty match
                if top == CLOSURE_BODY || top == NO_TYPE_NAMES {
                    self.pop_code();
                    continue;
                }
                return b.len();
            }
            let c = b[p];
            if CLASS[usize::from(c)] & SPACE == 0 {
                // the next token's, a closure's start once
                self.last_opened = std::mem::take(&mut self.opened);
            }
            if top > CLOSURE_BODY {
                p = match top {
                    AFTER_OP | TRY_CLOSURE => self.after_op_frame(p, c, top),
                    NO_TYPE_NAMES => self.path_frame(p, c),
                    // no type frame matches on a blank
                    _ if CLASS[usize::from(c)] & SPACE != 0 => clike::skip_blanks(b, p),
                    _ => self.type_frame(p, c, top),
                };
                continue;
            }
            let k = CLASS[usize::from(c)];
            if k & SPACE != 0 {
                p = clike::skip_blanks(b, p);
            } else if k & NAME != 0 && k & DIGIT == 0 {
                p = self.code_word(p);
            } else {
                p = self.code_punct(p, c, top);
            }
        }
    }

    /// `after-operator` or `try-closure` at `p`, not the line's end: a closure's `|`, a
    /// generic's `<` after an operator, or the empty match that pops it.
    fn after_op_frame(&mut self, p: usize, c: u8, top: u8) -> usize {
        let b = self.b;
        // the prototype
        if c == b'$'
            && let Some(e) = ident_end(b, p + 1)
        {
            self.tok(p, e, VARIABLE);
            return e;
        }
        let q = clike::skip_blanks(b, p);
        match b.get(q) {
            None => q,
            Some(b'|') => {
                self.set(p, CLOSURE);
                q
            }
            Some(b'<') if top == AFTER_OP => {
                self.set(q, GENERIC);
                q
            }
            Some(b'$') if q > p && ident_starts(b, q + 1) => q,
            Some(_) => {
                self.pop_code();
                q
            }
        }
    }

    /// `type-any-identifier`'s patterns for a name at `p`, at a word's start: a modifier, a
    /// function's type, a path's segment, a generic's name, a primitive type, `_`; else, for
    /// an identifier, its end, which the contexts that try `type-any-identifier` last would
    /// pass to.
    pub(super) fn type_word(&mut self, p: usize) -> Option<usize> {
        let n = self.name(p);
        self.type_name(p, n, true)
    }

    /// A function's parameter's name at `p`, at a word's start (`self`, `name:`), else
    /// `type-any-identifier`'s patterns for it.
    pub(super) fn param_word(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let n = self.name(p);
        let e = n.e;
        if n.id == W_SELF {
            return Some(self.token(p, e - p, PARAMETER));
        }
        if is_ident(&b[p..e]) {
            // `name\s*(:(?!:))`
            let s = skip_spaces(b, e);
            if b.get(s) == Some(&b':') && b.get(s + 1) != Some(&b':') {
                self.put(p, e, PARAMETER);
                return Some(self.token(s, 1, SEPARATOR));
            }
        }
        self.type_name(p, n, true)
    }

    /// A generic's argument's name at `p`, at a word's start: `type-any-identifier`'s
    /// patterns, then a `support.type` name or `as`; else plain to its end.
    pub(super) fn generic_word(&mut self, p: usize) -> usize {
        let n = self.name(p);
        if let Some(q) = self.type_name(p, n, false) {
            return q;
        }
        match INFO[usize::from(n.id)] {
            (G_SUPPORT, _) => self.token(p, n.e - p, SUPPORT_TYPE),
            _ if n.id == W_AS => self.token(p, 2, OPERATOR),
            _ => n.e,
        }
    }

    /// [`Scan::type_word`]'s patterns for the name `n` at `p`; a name none takes is `None`, or
    /// with `plain` its end where it is an identifier.
    fn type_name(&mut self, p: usize, n: NameAt, plain: bool) -> Option<usize> {
        let b = self.b;
        let (e, len) = (n.e, n.e - p);
        let next = b.get(e).copied();
        match n.id {
            W_REF | W_MUT | W_CONST => return Some(self.token(p, len, MODIFIER)),
            W_FN if next == Some(b'(') => {
                self.token(p, e - p, FN);
                self.push(e, FN_TYPE);
                return Some(self.token(e, 1, META_GROUP_BEGIN));
            }
            _ => {}
        }
        let first = b[p];
        let ident =
            CLASS[usize::from(first)] & (UPPER | LOWER | HIGH) != 0 || (first == b'_' && len >= 2);
        if next == Some(b':') && b.get(e + 1) == Some(&b':') {
            if is_type_name(&b[p..e]) {
                self.token(p, e - p, PATH_TYPE);
                return Some(self.token(e, 2, TYPE_ACCESSOR));
            }
            if ident {
                self.token(p, e - p, PATH);
                return Some(self.token(e, 2, PATH_ACCESSOR));
            }
        }
        let group = INFO[usize::from(n.id)].0;
        if next == Some(b'<') && ident {
            if group == G_SUPPORT {
                self.token(p, e - p, SUPPORT_TYPE);
            }
            self.push(e, GENERIC);
            return Some(e);
        }
        if group == G_PRIMITIVE {
            return Some(self.token(p, len, STORAGE));
        }
        if len == 1 && first == b'_' {
            return Some(self.token(p, 1, OPERATOR));
        }
        (plain && ident).then_some(e)
    }

    /// A type frame `top` at `p`, not a blank nor the line's end. Of ada's own design (X-056),
    /// in place of bat's contexts for types: a generic's arguments are one frame, opened with
    /// its `<` and closed by its `>` (bat's pushed `generic` then `generic-arguments`, and read a
    /// nested `>` as a begin); a byte no type holds ends a generic and is the frame below's
    /// (bat's marked it invalid, `if a<b {`'s `{`), `?Sized`'s `?` an operator; a return type
    /// ends at the token that ends it, not by a look-ahead at each blank.
    fn type_frame(&mut self, p: usize, c: u8, top: u8) -> usize {
        let b = self.b;
        let next = b.get(p + 1).copied();
        // the prototype, then comments
        if c == b'$'
            && let Some(e) = ident_end(b, p + 1)
        {
            return self.tok(p, e, VARIABLE);
        }
        if c == b'/'
            && matches!(next, Some(b'/' | b'*'))
            && let Some(e) = self.comment(p)
        {
            return e;
        }
        let word = CLASS[usize::from(c)] & NAME != 0;
        let starts = word && (p == 0 || CLASS[usize::from(b[p - 1])] & NAME == 0);
        match top {
            GENERIC => {
                // entered at its `<`
                if c == b'<' {
                    self.set_top(GENERIC_ARGS);
                    return self.tok(p, p + 1, GENERIC_BEGIN);
                }
                self.pop_code();
                p
            }
            GENERIC_ARGS | GENERIC_NESTED | GENERIC_ARRAY => self.generic_frame(p, c, top, starts),
            FN_PARAMS => match c {
                b')' => {
                    self.tok(p, p + 1, FN_PARAMS_END);
                    self.set_top(FN_RETURN);
                    p + 1
                }
                b'(' => {
                    self.push(p, PARAM_TUPLE);
                    self.tok(p, p + 1, META_GROUP_BEGIN)
                }
                b':' if let Some(o) = colon_paren(b, p) => {
                    self.tok(p, p + 1, SEPARATOR);
                    self.push(o, PARAM_TUPLE_TYPE);
                    self.tok(o, o + 1, META_GROUP_BEGIN)
                }
                _ if starts => self.param_word(p).unwrap_or_else(|| word_end(b, p)),
                _ if word => word_end(b, p),
                _ => self.type_tok(p, c).unwrap_or(p + 1),
            },
            FN_GENERIC => match c {
                b'<' => {
                    self.push_code(GENERIC_ARGS);
                    self.tok(p, p + 1, GENERIC_BEGIN)
                }
                b'(' => {
                    self.set_top(FN_PARAMS);
                    self.tok(p, p + 1, PARAMS_BEGIN)
                }
                _ => {
                    self.pop_code();
                    p
                }
            },
            FN_RETURN => match c {
                b'{' => self.open_block(p, false),
                b'-' if next == Some(b'>') => {
                    self.tok(p, p + 2, SEPARATOR);
                    self.push_code(RETURN_TYPE);
                    p + 2
                }
                b'w' if self.is_word_at(p, b"where") => {
                    self.set(p, FN_WHERE);
                    p
                }
                _ => {
                    self.pop_code();
                    p
                }
            },
            RETURN_TYPE | FN_TYPE_RETURN => {
                if top == FN_TYPE_RETURN {
                    if c == b'-' && next == Some(b'>') {
                        self.tok(p, p + 2, SEPARATOR);
                        self.set_top(RETURN_TYPE);
                        return p + 2;
                    }
                } else if c != b'{' && !(c == b'w' && self.is_word_at(p, b"where")) {
                    let q = if starts {
                        self.type_word(p)
                    } else if word {
                        Some(word_end(b, p))
                    } else {
                        self.type_tok(p, c)
                    };
                    if let Some(q) = q {
                        return q;
                    }
                }
                self.pop_code();
                p
            }
            STRUCT_FIELDS => match c {
                b'}' => self.tok_close(p, BLOCK_END),
                b'#' => self.attribute(p).unwrap_or(p + 1),
                _ if starts => self.field_word(p),
                _ if word => word_end(b, p),
                _ => p + 1,
            },
            FIELD_TYPE => match c {
                // the field's `,` plain, then the fields'
                b',' => {
                    self.pop_code();
                    p + 1
                }
                b'}' => {
                    self.pop_code();
                    p
                }
                b':' if next != Some(b':') => self.tok(p, p + 1, SEPARATOR_TYPE),
                _ => self.type_any(p, c, word, starts),
            },
            TYPE_GROUP => match c {
                b')' => self.tok_close(p, GROUP_END),
                _ => self.type_any(p, c, word, starts),
            },
            // FN_TYPE
            _ => match c {
                b')' => {
                    self.tok(p, p + 1, META_GROUP_END);
                    self.set_top(FN_TYPE_RETURN);
                    p + 1
                }
                _ => self.type_any(p, c, word, starts),
            },
        }
    }

    /// `type-any-identifier` at `p` where it is the frame's last pattern: what it takes, else
    /// plain to the next place (a word's end, the next byte).
    #[inline(always)]
    fn type_any(&mut self, p: usize, c: u8, word: bool, starts: bool) -> usize {
        if starts {
            return self.type_word(p).unwrap_or_else(|| word_end(self.b, p));
        }
        if word {
            return word_end(self.b, p);
        }
        self.type_tok(p, c).unwrap_or(p + 1)
    }

    /// `type-any-identifier`'s patterns that start on punctuation, at `p`: a reference's `&`,
    /// a raw pointer's `*const`/`*mut`, a lifetime, a path's `::`, a generic's arguments, a
    /// type's group.
    fn type_tok(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        match c {
            b'&' => Some(self.tok(p, p + 1, OPERATOR)),
            b'*' => {
                let n = if b[p + 1..].starts_with(b"mut") {
                    4
                } else if b[p + 1..].starts_with(b"const") {
                    6
                } else {
                    return None;
                };
                ends_word(b, p + n).then(|| self.tok(p, p + n, MODIFIER))
            }
            b'\'' => self.lifetime(p),
            b':' => (b.get(p + 1) == Some(&b':') && ident_starts(b, p + 2))
                .then(|| self.tok(p, p + 2, PATH_ACCESSOR)),
            b'<' => {
                self.push_code(GENERIC_ARGS);
                Some(self.tok(p, p + 1, GENERIC_BEGIN))
            }
            b'(' => {
                self.push_code(TYPE_GROUP);
                Some(self.tok(p, p + 1, GROUP_BEGIN))
            }
            _ => None,
        }
    }

    /// A generic's arguments (or an array's in them, `[T; N]`) at `p`.
    fn generic_frame(&mut self, p: usize, c: u8, top: u8, starts: bool) -> usize {
        let b = self.b;
        match c {
            b'>' if top != GENERIC_ARRAY => return self.tok_close(p, GENERIC_END),
            b']' if top == GENERIC_ARRAY => return self.tok_close(p, GROUP_END),
            b';' if top == GENERIC_ARRAY => {
                self.tok(p, p + 1, SEPARATOR);
                self.set(p + 1, GENERIC_LEN);
                return p + 1;
            }
            b'[' => {
                self.push_code(GENERIC_ARRAY);
                return self.tok(p, p + 1, GROUP_BEGIN);
            }
            b'-' if b.get(p + 1) == Some(&b'>') => return self.tok(p, p + 2, SEPARATOR_GENERIC),
            b'e' if starts && self.is_word_at(p, b"extern") => {
                self.tok(p, p + 6, KEYWORD);
                self.push(p + 6, GENERIC_EXTERN);
                return p + 6;
            }
            _ if starts => {
                // a macro in a type (`Token![,]`): its brackets hold code, closed back into
                // the arguments
                let e = word_end(b, p);
                if b.get(e) == Some(&b'!') {
                    let (frame, leaf) = match b.get(e + 1) {
                        Some(b'[') => (BRACKET, GROUP_BEGIN),
                        Some(b'(') => (GROUP, GROUP_BEGIN),
                        Some(b'{') => (BLOCK, BLOCK_BEGIN),
                        _ => return self.generic_word(p),
                    };
                    self.tok(p, e + 1, SUPPORT_MACRO);
                    self.push_code(frame);
                    return self.tok(e + 1, e + 2, leaf);
                }
                return self.generic_word(p);
            }
            // an attribute on a parameter (`#[may_dangle] T`), else a `quote!` interpolation's
            // `#`, plain
            b'#' => return self.attribute(p).unwrap_or(p + 1),
            b'+' | b'?' => return self.tok(p, p + 1, OPERATOR),
            // plain in the arguments: the rest of a word, `,`, `=` (an associated type's), `!`
            // (the never type), the brackets bat's grammar lets through
            b',' | b'=' | b'!' | b')' | b']' => return p + 1,
            _ if CLASS[usize::from(c)] & NAME != 0 => return word_end(b, p),
            _ => {
                if let Some(q) = self.type_tok(p, c) {
                    return q;
                }
                match c {
                    b':' => return self.tok(p, p + 1, SEPARATOR),
                    b'*' | b'\'' | b'$' => return p + 1,
                    _ => {}
                }
            }
        }
        // a byte no type holds: the generic ends, and the frame below reads it
        self.pop_code();
        p
    }

    /// A struct's field at `p`, a word's start: `pub` (and `pub(crate)`), a field's name before
    /// its `:`, else plain.
    fn field_word(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = self.name(p);
        let e = n.e;
        if n.id == W_PUB {
            self.tok(p, e, MODIFIER);
            if b[e..].starts_with(b"(crate)") {
                self.tok(e, e + 1, MODIFIER_BEGIN);
                self.tok(e + 1, e + 6, MODIFIER);
                return self.tok(e + 6, e + 7, MODIFIER_END);
            }
            return e;
        }
        // `name(?=\s*:)`
        if is_ident(&b[p..e]) && b.get(clike::skip_blanks(b, e)) == Some(&b':') {
            self.tok(p, e, MEMBER);
            self.push_code(FIELD_TYPE);
        }
        e
    }

    /// `no-type-names` at `p`, not the line's end: a path's next segment, or nothing (an empty
    /// match that ends it, which the frame below then reads).
    fn path_frame(&mut self, p: usize, c: u8) -> usize {
        let b = self.b;
        if CLASS[usize::from(c)] & NAME != 0 {
            if p == 0 || CLASS[usize::from(b[p - 1])] & NAME == 0 {
                let n = self.name(p);
                let (e, len) = (n.e, n.e - p);
                if len >= 2 && matches!(c, b'A'..=b'Z' | b'_') && n.bits & (LOWER | HIGH) == 0 {
                    return self.tok(p, p + len, CONSTANT);
                }
                if len >= 3
                    && lower(c, n.bits)
                    && (b[p..e].starts_with(b"c_") || b[p..e].ends_with(b"_t"))
                {
                    return self.tok(p, p + len, STORAGE);
                }
                let ident =
                    CLASS[usize::from(c)] & (UPPER | LOWER | HIGH) != 0 || (c == b'_' && len >= 2);
                if ident && b.get(e) == Some(&b':') && b.get(e + 1) == Some(&b':') {
                    return self.path_code(p, e, n.id);
                }
                match n.id {
                    W_SELF => return self.tok(p, p + len, SELF),
                    W_SUPER => return self.tok(p, p + len, KEYWORD),
                    _ if ident => {
                        if INFO[usize::from(n.id)].0 == G_SQL
                            && matches!(b.get(e), Some(b'(' | b'!'))
                        {
                            self.sql_name(p, e);
                        }
                        return e;
                    }
                    _ => {}
                }
            }
        } else {
            match c {
                b'/' => {
                    if let Some(e) = self.comment(p) {
                        return e;
                    }
                }
                // the prototype
                b'$' => {
                    if let Some(e) = ident_end(b, p + 1) {
                        self.tok(p, e, VARIABLE);
                        return e;
                    }
                }
                b':' if b.get(p + 1) == Some(&b':') && ident_starts(b, p + 2) => {
                    self.tok(p, p + 2, PATH_ACCESSOR);
                    self.push_code(NO_TYPE_NAMES);
                    return p + 2;
                }
                b'<' => {
                    self.push(p, GENERIC);
                    return p;
                }
                _ => {}
            }
        }
        self.pop_code();
        p
    }

    /// A name at `p` in a statement frame: an item's head, a closure's name, a keyword, a
    /// type, a call, a macro, a constant, a path.
    fn code_word(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = self.name(p);
        let e = n.e;
        // a name that starts inside a word matches nothing: plain to the word's end
        if p > 0 && CLASS[usize::from(b[p - 1])] & NAME != 0 {
            return e;
        }
        let first = b[p];
        let ident = CLASS[usize::from(first)] & (UPPER | LOWER | HIGH) != 0
            || (first == b'_' && e - p >= 2);
        let next = b.get(e).copied();
        let id = n.id;
        // the item's keyword, `pub`'s next word after blanks: where it is, and its id
        let (mut k, mut ke, mut item) = (p, e, if id <= W_IMPL { id } else { 0 });
        if id == W_PUB {
            item = 0;
            let s = clike::skip_blanks(b, e);
            if s > e && s < b.len() && CLASS[usize::from(b[s])] & NAME != 0 {
                let m = self.name(s);
                if m.id > W_PUB && m.id <= W_IMPL {
                    (k, ke, item) = (s, m.e, m.id);
                }
            }
        }
        // the items, in bat's order: a module, a closure's name, a function, a struct, a type,
        // a trait, an impl, an enum
        if item == W_MOD
            && let Some(q) = self.item_at(p, e, (k, ke), MODULE, MODULE_NAME, Some(MOD_HEAD))
        {
            return q;
        }
        if ident && matches!(next, Some(b' ' | b'\t' | b'=')) {
            // `name\s*=\s*(?=\|)`
            let s = skip_spaces(b, e);
            if b.get(s) == Some(&b'=') {
                let t = skip_spaces(b, s + 1);
                if b.get(t) == Some(&b'|') {
                    self.tok(p, e, FN_NAME);
                    self.tok(s, s + 1, OPERATOR);
                    self.push(t, CLOSURE);
                    return t;
                }
            }
        }
        if item > W_MOD {
            let q = match item {
                W_FN => self.item_at(p, e, (k, ke), FN, FN_NAME, Some(FN_GENERIC)),
                W_STRUCT => self.struct_at(p, e, (k, ke)),
                W_TYPE => self.item_at(p, e, (k, ke), TYPE, TYPE_NAME, None),
                W_TRAIT => self.item_at(p, e, (k, ke), TRAIT, TRAIT_NAME, Some(HEAD)),
                W_IMPL if id == W_IMPL => {
                    self.tok(p, e, IMPL);
                    self.push(e, IMPL_DEF);
                    Some(e)
                }
                W_ENUM => self.item_at(p, e, (k, ke), ENUM, ENUM_NAME, Some(HEAD)),
                _ => None,
            };
            if let Some(q) = q {
                return q;
            }
        }
        let (group, leaf) = INFO[usize::from(id)];
        if group == G_KEYWORD {
            self.put(p, e, leaf);
            if id == W_RETURN {
                self.push_ahead_code(e, AFTER_OP);
            }
            return e;
        }
        // `type`: a name before `<`, then the primitive types
        if next == Some(b'<') && ident {
            if group == G_SUPPORT {
                self.tok(p, e, SUPPORT_TYPE);
            }
            self.push(e, GENERIC);
            return e;
        }
        if group == G_PRIMITIVE {
            return self.tok(p, e, STORAGE);
        }
        if group == G_ASKED {
            match id {
                W_MACRO_RULES if next == Some(b'!') => {
                    let s = skip_spaces(b, e + 1);
                    if s > e + 1 && matches!(b.get(s), Some(&c) if is_letter(c) || c == b'_') {
                        let m = word_end(b, s);
                        self.tok(p, e + 1, SUPPORT_FN);
                        self.tok(s, m, MACRO_NAME);
                        self.push(m, MACRO_BLOCK);
                        return m;
                    }
                }
                W_B..=W_R => {
                    if let Some(q) = self.string(p, true) {
                        return q;
                    }
                }
                _ => {}
            }
        }
        if matches!(next, Some(b'(' | b'!')) && lower(first, n.bits) {
            if next == Some(b'(') {
                if group == G_SQL {
                    self.sql_name(p, e);
                }
                return self.tok(p, e, SUPPORT_FN);
            }
            let s = skip_spaces(b, e + 1);
            let open = b.get(s).copied();
            if open == Some(b'(') {
                if group == G_SQL {
                    self.sql_name(p, e);
                }
                let fmt = match id {
                    W_FORMAT..=W_EPRINTLN => FMT_FIRST,
                    W_WRITE | W_WRITELN => WRITE_FIRST,
                    _ => MAIN,
                };
                if fmt != MAIN {
                    self.tok(p, e + 1, SUPPORT_MACRO);
                    self.push(s, fmt);
                    return self.tok(s, s + 1, META_GROUP_BEGIN);
                }
            }
            if matches!(open, Some(b'(' | b'{' | b'[')) {
                return self.tok(p, e + 1, SUPPORT_MACRO);
            }
        }
        if group == G_SUPPORT {
            return self.tok(p, e, SUPPORT_TYPE);
        }
        // basic-identifiers
        let len = e - p;
        if len >= 2 && matches!(first, b'A'..=b'Z' | b'_') && n.bits & (LOWER | HIGH) == 0 {
            return self.tok(p, p + len, CONSTANT);
        }
        if len >= 3
            && lower(first, n.bits)
            && (b[p..e].starts_with(b"c_") || b[p..e].ends_with(b"_t"))
        {
            return self.tok(p, p + len, STORAGE);
        }
        if ident && next == Some(b':') && b.get(e + 1) == Some(&b':') {
            return self.path_code(p, e, id);
        }
        match id {
            W_SELF => self.tok(p, p + len, SELF),
            W_SUPER => self.tok(p, p + len, KEYWORD),
            _ if len == 1 && first == b'_' => self.tok(p, p + 1, OPERATOR),
            // no pattern matches inside a word: it is plain
            _ => e,
        }
    }

    /// `\b(?:(pub)\s+)?(kw)\s+(name)\b`, the word `p..e`, its keyword at `k..ke`: the keyword
    /// is `leaf`, the name `name`, and `then` is pushed after it.
    fn item_at(
        &mut self,
        p: usize,
        e: usize,
        (k, ke): (usize, usize),
        leaf: u16,
        name: u16,
        then: Option<u8>,
    ) -> Option<usize> {
        let b = self.b;
        let s = skip_spaces(b, ke);
        if s == ke {
            return None;
        }
        let n = ident_end(b, s)?;
        if k > p {
            self.tok(p, e, MODIFIER);
        }
        self.put(k, ke, leaf);
        self.put(s, n, name);
        if let Some(c) = then {
            self.push(n, c);
        }
        Some(n)
    }

    /// `\b(?:(pub)\s+)?(struct)\s+`, the `\n` among the spaces.
    fn struct_at(&mut self, p: usize, e: usize, (k, ke): (usize, usize)) -> Option<usize> {
        let b = self.b;
        let end = if ke == b.len() {
            ke
        } else if is_space(b[ke]) {
            skip_spaces(b, ke)
        } else {
            return None;
        };
        if k > p {
            self.tok(p, e, MODIFIER);
        }
        self.tok(k, ke, STRUCT);
        self.push(end, STRUCT_ID);
        Some(end)
    }

    /// Punctuation, a digit or a `$` at `p` in a statement frame `top`: the frame's closers,
    /// then `statements`' patterns; a byte none starts on is plain.
    fn code_punct(&mut self, p: usize, c: u8, top: u8) -> usize {
        let b = self.b;
        let next = b.get(p + 1).copied();
        match c {
            b'}' if top == BLOCK => self.tok_close(p, BLOCK_END),
            b')' if top == GROUP => self.tok_close(p, META_GROUP_END),
            b']' if matches!(top, BRACKET | MACRO_BRACKET) => self.tok_close(p, GROUP_END),
            b';' if top == BRACKET => self.tok(p, p + 1, SEPARATOR),
            b'}' | b';' | b')' | b']' if top == CLOSURE_BODY => {
                self.pop_code();
                p
            }
            b'\'' => self.quote(p),
            b'"' => {
                self.push(p, STRING);
                let e = self.tok(p, p + 1, STR_BEGIN);
                self.regex_in(p, e);
                e
            }
            b'/' => match self.comment(p) {
                Some(e) => e,
                None => self.op_eq(p, ARITHMETIC),
            },
            b'#' => match self.attribute(p) {
                Some(e) => e,
                None => self.tok(p, p + 1, OPERATOR),
            },
            b'{' => {
                self.push_code(BLOCK);
                self.tok(p, p + 1, BLOCK_BEGIN);
                self.push_ahead_code(p + 1, TRY_CLOSURE);
                p + 1
            }
            b'(' => {
                if p == self.sql_paren {
                    self.push(p, PSQL + self.sql_call);
                    self.tok(p, p + 1, GROUP_BEGIN);
                    self.push_ahead(p + 1, TRY_CLOSURE);
                } else {
                    self.push_code(GROUP);
                    self.tok(p, p + 1, GROUP_BEGIN);
                    self.push_ahead_code(p + 1, TRY_CLOSURE);
                }
                p + 1
            }
            b'[' => {
                self.push_code(BRACKET);
                self.tok(p, p + 1, GROUP_BEGIN)
            }
            b'0'..=b'9' => {
                if p > 0 && CLASS[usize::from(b[p - 1])] & NAME != 0 {
                    // inside a word: plain to its end
                    return clike::class_run(b, p, NAME).end;
                }
                // a decimal's digits to a byte that is no number's: the integer alone
                let r = digits_end(b, p, |c| c.is_ascii_digit());
                match b.get(r) {
                    Some(&c) if c == b'.' || CLASS[usize::from(c)] & NAME != 0 => {}
                    _ => return self.tok(p, r, DECIMAL),
                }
                self.number(p, true)
                    .unwrap_or_else(|| clike::class_run(b, p, NAME).end)
            }
            b':' => {
                if next == Some(b':') && ident_starts(b, p + 2) {
                    self.tok(p, p + 2, PATH_ACCESSOR);
                    self.push_code(NO_TYPE_NAMES);
                    p + 2
                } else {
                    self.after_op_code(p, 1, SEPARATOR)
                }
            }
            b';' => self.after_op_code(p, 1, TERMINATOR),
            b',' => self.after_op_code(p, 1, SEPARATOR),
            b'-' if next == Some(b'>') => {
                self.tok(p, p + 2, SEPARATOR);
                self.push(p + 2, RETURN_TYPE);
                p + 2
            }
            b'=' => match next {
                Some(b'>') => self.after_op_code(p, 2, OPERATOR),
                Some(b'=') => self.tok(p, p + 2, COMPARISON),
                _ => self.after_op_code(p, 1, ASSIGNMENT),
            },
            b'*' => match self.star_keyword(p) {
                Some(e) => e,
                None => self.op_eq(p, ARITHMETIC),
            },
            b'-' | b'+' | b'%' => self.op_eq(p, ARITHMETIC),
            b'^' => self.op_eq(p, BITWISE),
            b'|' if self.last_opened != 0 => {
                self.push(p, CLOSURE);
                p
            }
            b'<' if self.last_opened == OPENED_AFTER => {
                self.push_code(GENERIC);
                p
            }
            b'&' | b'|' => {
                if next == Some(b'=') {
                    self.tok(p, p + 2, ASSIGNMENT)
                } else if next == Some(c) {
                    self.tok(p, p + 2, LOGICAL)
                } else {
                    self.tok(p, p + 1, BITWISE)
                }
            }
            b'!' => {
                if next == Some(b'=') {
                    self.tok(p, p + 2, COMPARISON)
                } else {
                    self.tok(p, p + 1, LOGICAL)
                }
            }
            b'<' | b'>' => {
                if next == Some(c) && b.get(p + 2) == Some(&b'=') {
                    self.tok(p, p + 3, ASSIGNMENT)
                } else if next == Some(b'=') {
                    self.tok(p, p + 2, COMPARISON)
                } else {
                    self.tok(p, p + 1, COMPARISON)
                }
            }
            b'.' => {
                if next == Some(b'.') {
                    let n = if b.get(p + 2) == Some(&b'.') { 3 } else { 2 };
                    self.tok(p, p + n, RANGE)
                } else {
                    self.tok(p, p + 1, DOT)
                }
            }
            b'$' => match ident_end(b, p + 1) {
                // the prototype
                Some(e) => {
                    self.tok(p, e, VARIABLE);
                    e
                }
                None => self.tok(p, p + 1, OPERATOR),
            },
            b'@' | b'~' | b'?' => self.tok(p, p + 1, OPERATOR),
            _ => p + 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ids the frames ask for by name are their words', and each word is listed once.
    #[test]
    fn the_ids_name_their_words() {
        for (id, w) in [
            (W_PUB, &b"pub"[..]),
            (W_MOD, b"mod"),
            (W_FN, b"fn"),
            (W_STRUCT, b"struct"),
            (W_TYPE, b"type"),
            (W_TRAIT, b"trait"),
            (W_ENUM, b"enum"),
            (W_IMPL, b"impl"),
            (W_RETURN, b"return"),
            (W_MACRO_RULES, b"macro_rules"),
            (W_B, b"b"),
            (W_R, b"r"),
            (W_SELF, b"self"),
            (W_SUPER, b"super"),
            (W_FORMAT, b"format"),
            (W_EPRINTLN, b"eprintln"),
            (W_WRITE, b"write"),
            (W_WRITELN, b"writeln"),
        ] {
            assert_eq!(
                WORDS.get(w),
                u16::from(id),
                "{}",
                String::from_utf8_lossy(w)
            );
        }
        assert_eq!(WORDS.get(b"br"), u16::from(W_B) + 1);
        for (i, (w, ..)) in LIST.iter().enumerate() {
            assert_eq!(
                usize::from(WORDS.get(w)),
                i + 1,
                "{}",
                String::from_utf8_lossy(w)
            );
        }
    }

    /// The table tells each word as `lookup` and `sql_call` do.
    #[test]
    fn the_table_agrees_with_lookup() {
        for (w, group, leaf) in LIST {
            let want = match lookup(w) {
                Name::Keyword(l) => (G_KEYWORD, l),
                Name::Support => (G_SUPPORT, 0),
                Name::Primitive => (G_PRIMITIVE, 0),
                Name::Other => {
                    let sql = sql_call(w, false).is_some() || sql_call(w, true).is_some();
                    (if sql { G_SQL } else { G_ASKED }, 0)
                }
            };
            assert_eq!((group, leaf), want, "{}", String::from_utf8_lossy(w));
        }
    }
}
