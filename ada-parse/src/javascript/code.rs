//! JavaScript's code frames on the C family's core (X-058): the contexts whose text is code
//! (the top level, a block or an object, parentheses, brackets, a template's substitution, a JSX
//! expression, a `switch`'s body and its `case`, a parameter's default, a `for`'s head) run by a
//! loop of their own ([`Scan::code_run`]): a blank run and the token after it in one turn, a name
//! read once and classified by what follows it. What the loop doesn't take it hands back to the
//! context's dispatch ([`Scan::dispatch`]), which tries Babel's patterns in order.

use super::*;

use crate::emit::Mark;
use crate::words::{Words, text_len};

// What a word is to the name turn ([`Scan::name_turn`]), beside its patterns' mask.
const C_NONE: u8 = 0;
/// `literal-keywords`' alone: its leaf.
const C_KW: u8 = 1;
/// `await`, `yield`: `literal-keywords`', with a generator's `*` after.
const C_AWAIT: u8 = 2;
/// `literal-module`'s alone.
const C_MODULE: u8 = 3;
/// `literal-language-constant`'s alone: its leaf.
const C_CONST: u8 = 4;
/// `literal-language-variable`'s alone: its leaf.
const C_VAR: u8 = 5;
/// `support-class`' or `support-other`'s: its leaf, 0 for a support object's (with a member).
const C_SUPPORT: u8 = 6;
/// `function`.
const C_FUNCTION: u8 = 7;
/// `for`, `switch`: `literal-for`'s and `literal-switch`'s in `core`, their leaf.
const C_FOR: u8 = 8;
/// `new`.
const C_NEW: u8 = 9;
/// `literal-operators`' words alone.
const C_OP: u8 = 10;

const SC: u16 = W_SUPPORT_CLASS;
const SO: u16 = W_SUPPORT_OTHER;

const N: usize = 138;

/// Every word a pattern at a name's start looks for ([`word_mask`]): its patterns' mask, and
/// for the name turn its class and leaf.
const LIST: [(&[u8], u16, u8, u16); N] = [
    (b"Array", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"ArrayBuffer", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"Boolean", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"DataView", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"Date", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"Float32Array", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"Float64Array", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"Function", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"Int16Array", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"Int32Array", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"Int8Array", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"JSON", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"Map", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"Math", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"Number", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"Object", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"Promise", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"Proxy", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"Reflect", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"RegExp", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"Set", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"String", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"Symbol", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"System", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"TypeError", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"Uint16Array", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"Uint32Array", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"Uint8Array", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"Uint8ClampedArray", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"WeakMap", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"WeakSet", SC, C_SUPPORT, BUILTIN_CLASS),
    (b"Error", SC, C_SUPPORT, ERROR_CLASS),
    (b"EvalError", SC, C_SUPPORT, ERROR_CLASS),
    (b"RangeError", SC, C_SUPPORT, ERROR_CLASS),
    (b"ReferenceError", SC, C_SUPPORT, ERROR_CLASS),
    (b"SyntaxError", SC, C_SUPPORT, ERROR_CLASS),
    (b"URIError", SC, C_SUPPORT, ERROR_CLASS),
    (b"Buffer", SC, C_SUPPORT, NODE_CLASS),
    (b"Infinity", SC | W_NUMBER, C_SUPPORT, BUILTIN_CLASS),
    (b"NaN", SC | W_CONSTANT, C_SUPPORT, BUILTIN_CLASS),
    (b"constructor", SO | W_LANG_VAR, C_SUPPORT, BUILTIN_FN),
    (b"decodeURI", SO, C_SUPPORT, BUILTIN_FN),
    (b"decodeURIComponent", SO, C_SUPPORT, BUILTIN_FN),
    (b"encodeURI", SO, C_SUPPORT, BUILTIN_FN),
    (b"encodeURIComponent", SO, C_SUPPORT, BUILTIN_FN),
    (b"escape", SO, C_SUPPORT, BUILTIN_FN),
    (b"eval", SO, C_SUPPORT, BUILTIN_FN),
    (b"hasOwnProperty", SO, C_SUPPORT, BUILTIN_FN),
    (b"isFinite", SO, C_SUPPORT, BUILTIN_FN),
    (b"isNaN", SO, C_SUPPORT, BUILTIN_FN),
    (b"isPrototypeOf", SO, C_SUPPORT, BUILTIN_FN),
    (b"parseFloat", SO, C_SUPPORT, BUILTIN_FN),
    (b"parseInt", SO, C_SUPPORT, BUILTIN_FN),
    (b"propertyIsEnumerable", SO, C_SUPPORT, BUILTIN_FN),
    (b"toLocaleString", SO, C_SUPPORT, BUILTIN_FN),
    (b"toString", SO, C_SUPPORT, BUILTIN_FN),
    (b"unescape", SO, C_SUPPORT, BUILTIN_FN),
    (b"valueOf", SO, C_SUPPORT, BUILTIN_FN),
    (b"clearImmediate", SO, C_SUPPORT, NODE_FN),
    (b"clearInterval", SO, C_SUPPORT, NODE_FN),
    (b"clearTimeout", SO, C_SUPPORT, NODE_FN),
    (b"require", SO, C_SUPPORT, NODE_FN),
    (b"setImmediate", SO, C_SUPPORT, NODE_FN),
    (b"setInterval", SO, C_SUPPORT, NODE_FN),
    (b"setTimeout", SO, C_SUPPORT, NODE_FN),
    (b"document", SO, C_SUPPORT, DOM),
    (b"window", SO, C_SUPPORT, DOM),
    (b"global", SO, C_SUPPORT, NODE_OBJ),
    (b"GLOBAL", SO, C_SUPPORT, NODE_OBJ),
    (b"root", SO, C_SUPPORT, NODE_OBJ),
    (b"__dirname", SO, C_SUPPORT, NODE_OBJ),
    (b"__filename", SO, C_SUPPORT, NODE_OBJ),
    (b"console", SO, C_SUPPORT, 0),
    (b"process", SO, C_SUPPORT, 0),
    (b"exports", SO, C_SUPPORT, 0),
    (b"module", SO | W_KEYWORD, C_SUPPORT, 0),
    (b"const", W_KEYWORD, C_KW, STORAGE),
    (b"let", W_KEYWORD, C_KW, STORAGE),
    (b"var", W_KEYWORD, C_KW, STORAGE),
    (b"await", W_KEYWORD, C_AWAIT, FLOW_KW),
    (b"yield", W_KEYWORD, C_AWAIT, FLOW_KW),
    (b"return", W_KEYWORD, C_KW, FLOW_KW),
    (b"if", W_KEYWORD, C_KW, COND),
    (b"else", W_KEYWORD, C_KW, COND),
    (b"catch", W_KEYWORD, C_KW, TRY),
    (b"finally", W_KEYWORD, C_KW, TRY),
    (b"throw", W_KEYWORD, C_KW, TRY),
    (b"try", W_KEYWORD, C_KW, TRY),
    (b"break", W_KEYWORD, C_KW, LOOP),
    (b"continue", W_KEYWORD, C_KW, LOOP),
    (b"do", W_KEYWORD, C_KW, LOOP),
    (b"goto", W_KEYWORD, C_KW, LOOP),
    (b"while", W_KEYWORD, C_KW, LOOP),
    (b"case", W_KEYWORD, C_KW, LOOP),
    (b"enum", W_KEYWORD, C_KW, RESERVED),
    (b"public", W_KEYWORD, C_KW, RESERVED),
    (b"package", W_KEYWORD, C_KW, RESERVED),
    (b"private", W_KEYWORD, C_KW, RESERVED),
    (b"protected", W_KEYWORD, C_KW, RESERVED),
    (b"debugger", W_KEYWORD, C_KW, KW_OTHER),
    (b"default", W_KEYWORD | W_MODULE, C_KW, LOOP),
    (b"interface", W_KEYWORD | W_CLASS, C_NONE, 0),
    (b"import", W_MODULE, C_MODULE, MODULE),
    (b"export", W_MODULE, C_MODULE, MODULE),
    (b"from", W_MODULE, C_MODULE, MODULE),
    (b"as", W_MODULE, C_MODULE, MODULE),
    (b"class", W_CLASS, C_NONE, 0),
    (b"declare", W_FLOW, C_NONE, 0),
    (b"type", W_FLOW, C_NONE, 0),
    (b"true", W_CONSTANT, C_CONST, TRUE),
    (b"false", W_CONSTANT, C_CONST, FALSE),
    (b"null", W_CONSTANT, C_CONST, NULL),
    (b"undefined", W_CONSTANT, C_CONST, UNDEFINED),
    (b"arguments", W_LANG_VAR, C_VAR, ARGUMENTS),
    (b"super", W_LANG_VAR, C_VAR, SUPER),
    (b"this", W_LANG_VAR, C_VAR, THIS),
    (b"self", W_LANG_VAR, C_VAR, SELF),
    (b"__proto__", W_LANG_VAR, C_NONE, 0),
    (b"prototype", W_LANG_VAR, C_NONE, 0),
    (b"new", W_NEW | W_OPERATOR, C_NEW, NEW),
    (b"delete", W_OPERATOR, C_OP, OP),
    (b"instanceof", W_OPERATOR, C_OP, OP),
    (b"in", W_OPERATOR, C_OP, OP),
    (b"of", W_OPERATOR, C_OP, OP),
    (b"typeof", W_OPERATOR, C_OP, OP),
    (b"void", W_OPERATOR, C_OP, OP),
    (b"with", W_OPERATOR, C_OP, OP),
    (b"async", W_OTHER, C_NONE, 0),
    (b"function", W_OTHER, C_FUNCTION, 0),
    (b"static", W_OTHER, C_NONE, 0),
    (b"get", W_OTHER, C_NONE, 0),
    (b"set", W_OTHER, C_NONE, 0),
    (b"for", W_FOR_SWITCH, C_FOR, LOOP),
    (b"switch", W_FOR_SWITCH, C_FOR, SWITCH_KW),
    (b"styled", W_STYLED, C_NONE, 0),
    (b"injectGlobal", W_STYLED, C_NONE, 0),
    (b"keyframes", W_STYLED, C_NONE, 0),
    (b"gql", W_GQL, C_NONE, 0),
];

const WORD_LIST: [&[u8]; N] = {
    let mut w: [&[u8]; N] = [b""; N];
    let mut i = 0;
    while i < N {
        w[i] = LIST[i].0;
        i += 1;
    }
    w
};
#[cfg(test)]
pub(super) const LIST_WORDS: [&[u8]; N] = WORD_LIST;
pub(super) static WORDS: Words<512, N, { text_len(&WORD_LIST) }> = Words::new(&WORD_LIST);

/// A word's mask, class and leaf by its id ([`WORDS`]), 0 for none.
#[derive(Clone, Copy)]
pub(super) struct Info {
    pub(super) mask: u16,
    pub(super) class: u8,
    pub(super) leaf: u16,
}

pub(super) static INFO: [Info; N + 1] = {
    let mut t = [Info {
        mask: 0,
        class: C_NONE,
        leaf: 0,
    }; N + 1];
    let mut i = 0;
    while i < N {
        t[i + 1] = Info {
            mask: LIST[i].1,
            class: LIST[i].2,
            leaf: LIST[i].3,
        };
        i += 1;
    }
    t
};

/// What word `w` is: [`INFO`]'s entry for it.
#[inline(always)]
pub(super) fn info(w: &[u8]) -> Info {
    WORDS.value(w, &INFO)
}

/// A name's bytes past its first, ASCII's: `[$\w]`.
static NAME: [bool; 256] = {
    let mut t = WORD;
    t[b'$' as usize] = true;
    t
};

/// The bytes before a place inside a name (or a word past ASCII's): `[$\w]` and any byte past
/// ASCII, where no `\b` holds for the patterns at a name's start.
static IN_NAME: [bool; 256] = {
    let mut t = NAME;
    let mut c = 0x80;
    while c < 256 {
        t[c] = true;
        c += 1;
    }
    t
};

/// The kind of a name `p..e` that starts uppercase or with `_`, no `.` after it, read from its
/// bytes: a constant's capitals (`_*[A-Z][_$\dA-Z]*`), else `var` (X-058: every name in an
/// expression is one color in the themes, so `literal-variable`'s patterns aren't read again).
/// Out of line: inline at its three sites the loop grew dearer than the calls (X-058, C1).
#[inline(never)]
pub(super) fn upper_kind(b: &[u8], p: usize, e: usize, var: u16) -> u16 {
    let mut u = p;
    while u < e && b[u] == b'_' {
        u += 1;
    }
    if u < e
        && b[u].is_ascii_uppercase()
        && b[u + 1..e]
            .iter()
            .all(|&c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_' || c == b'$')
    {
        CONSTANT
    } else {
        var
    }
}

/// How many groups a line keeps pending at once ([`Scan::pending`]); past it, a group's `(` is
/// decided by its lookahead, as the dispatch does.
pub(super) const PENDING: usize = 8;

/// A group's `(` opened as plain, decided at its `)` (X-058, "arrows classified later"): a `{`
/// after a call's `)` makes it a method's head, a `=>` after a group's (or a call's) its arrow's
/// parameters. Then what was written since `undo` is taken back and the dispatch reads the place
/// `slow` again with its lookaheads: only real methods and arrows pay for them.
#[derive(Clone, Copy)]
pub(super) struct Pending {
    /// Where the writing stood before the group (a call's: before its name).
    undo: Mark,
    /// Where the spans ended then.
    at: u32,
    /// Where the dispatch reads again: the name (a call's, an assignment's), else the `(`.
    slow: u32,
    /// The stack's depth with the group open.
    depth: u16,
    /// A call's (`name(`), else a plain group's.
    call: bool,
}

/// [`Scan::prefix`]: an assignment's `(` at `at`, its name at `name`, the writing before it.
#[derive(Clone, Copy)]
pub(super) struct Prefix {
    at: usize,
    name: usize,
    undo: (Mark, usize),
}

/// Whether `c` is a code frame [`Scan::code_run`] runs.
#[inline(always)]
pub(super) fn is_fast(c: u8) -> bool {
    matches!(
        c,
        MAIN | CURLY
            | ROUND
            | SQUARE
            | SUBST
            | JSX_EXPR
            | SWITCH_BODY
            | CASE
            | PARAM_DEFAULT
            | FOR_IN
            | GQL_SUBST
            | STYLED_ARGS
    )
}

impl<K: Sink + Loop> Scan<'_, '_, K> {
    /// The code frames' run from `p`: where it stopped, at the line's end, where a frame it
    /// doesn't run is on top, or at a token it hands to the dispatch.
    pub(super) fn code_run(&mut self, mut p: usize) -> usize {
        let b = self.b;
        loop {
            let top = self.top();
            if p >= b.len() || !is_fast(top) {
                return p;
            }
            let mut c = b[p];
            let mut blank = false;
            if c == b' ' || c == b'\t' {
                // the blanks and the token after them in one turn: a blank opens and closes no
                // frame
                match self.blanks(p) {
                    Some(q) if q < b.len() => (p, c, blank) = (q, b[q], true),
                    Some(q) => return q,
                    None => return p,
                }
            }
            let q = match c {
                b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' => self.name_turn(p, top, blank),
                c if c < 0x80 => self.punct_turn(p, c, top),
                _ => None,
            };
            match q {
                Some(q) => p = q,
                // a group taken back hands its place to the dispatch ([`Scan::decide`])
                None if self.slow_at != usize::MAX => {
                    return std::mem::replace(&mut self.slow_at, usize::MAX);
                }
                None => return p,
            }
        }
    }

    /// A run of blanks at `p` where no pattern can start: where it ends; else `None`, for the
    /// dispatch (`\s*:`, a regex's or JSX's spaces, `gql`, a member's spaces after a `.`).
    #[inline(always)]
    fn blanks(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let q = p + b[p..]
            .iter()
            .take_while(|&&c| c == b' ' || c == b'\t')
            .count();
        let may = match b.get(q) {
            None => false,
            Some(b'\x0b' | b'\x0c' | b'\r' | b':' | b'`' | 0x80..) => true,
            Some(b'/') => !matches!(b.get(q + 1), Some(b'/' | b'*')),
            Some(b'g') => has(b, q, b"gql`"),
            Some(b'<') => ident_start(b.get(q + 1).copied()),
            _ => false,
        };
        (!may && (p == 0 || b[p - 1] != b'.')).then_some(q)
    }

    /// A name at `p` in code frame `top`, its first byte ASCII's: read once, and classified by
    /// what follows it as the dispatch's patterns would ([`Scan::plain_name`]'s, the keywords'
    /// alone); `None` for the dispatch, nothing written.
    #[inline(always)]
    fn name_turn(&mut self, p: usize, top: u8, blank: bool) -> Option<usize> {
        let b = self.b;
        // inside a word (after a digit's run or a letter past ASCII) only the dispatch's
        // patterns without a `\b` take it; never after blanks
        if !blank && p > 0 && IN_NAME[usize::from(b[p - 1])] {
            return None;
        }
        // `[$\w]+`, ASCII only; `we`, where its first `\w` run ends (the word the patterns key
        // on), then the rest of the name past a `$`
        let mut e = p;
        while e < b.len() && WORD[usize::from(b[e])] {
            e += 1;
        }
        let we = e;
        if e < b.len() && b[e] == b'$' {
            e += 1;
            while e < b.len() && NAME[usize::from(b[e])] {
                e += 1;
            }
        }
        if e < b.len() && b[e] >= 0x80 {
            return None;
        }
        let word = info(&b[p..we]);
        let mask = word.mask;
        let after_dot = p > 0 && b[p - 1] == b'.';
        match top {
            // `case`, `default`: the `case` context pushed
            SWITCH_BODY if !after_dot && matches!(&b[p..we], b"case" | b"default") => {
                return self.open_code(p, we, SWITCH_KW, CASE);
            }
            // `literal-keyword-storage`
            FOR_IN if !after_dot && matches!(&b[p..we], b"const" | b"let" | b"var") => {
                return self.tok_code(p, we, STORAGE);
            }
            _ => {}
        }
        // the byte after its spaces (`\s*`, the line's `\n` among them)
        let s = match b.get(e) {
            Some(&c) if c < 0x80 && !matches!(c, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r') => e,
            _ => spaces(b, e),
        };
        let next = at(b, s);
        let core = matches!(top, MAIN | CURLY | SWITCH_BODY | GQL_SUBST);
        let first = b[p];
        let upper = first.is_ascii_uppercase() || first == b'_';
        if mask != 0 {
            // a word of the patterns' lists: where a pattern before its own may take it, the
            // dispatch's
            if core && next == Some(b':') {
                return None;
            }
            match word.class {
                // `support-class` and `support-other` come first, after a `.` only `Buffer`
                C_SUPPORT if after_dot => {
                    return if word.leaf == NODE_CLASS {
                        self.tok_code(p, we, NODE_CLASS)
                    } else {
                        None
                    };
                }
                C_SUPPORT if word.leaf == 0 => return self.support_object(p, &b[p..we]),
                C_SUPPORT => return self.tok_code(p, we, word.leaf),
                // `literal-function`, before the patterns of the words after it
                C_FUNCTION => return self.function(p),
                C_FOR if core && !after_dot => {
                    let c = if word.leaf == LOOP { FOR } else { SWITCH };
                    return self.open_code(p, we, word.leaf, c);
                }
                _ => {}
            }
            if upper
                || after_dot
                || next == Some(b'=')
                || (b.get(e) == Some(&b'.') && has(b, e + 1, b"prototype"))
            {
                return None;
            }
            return match word.class {
                C_KW => self.tok_code(p, we, word.leaf),
                C_AWAIT => {
                    self.tok_code(p, we, FLOW_KW);
                    let t = spaces(b, we);
                    if b.get(t) == Some(&b'*') {
                        return self.tok_code(t, t + 1, ASTERISK);
                    }
                    Some(we)
                }
                C_MODULE | C_CONST | C_VAR => {
                    if next == Some(b'(') && self.body_follows(s) {
                        return None;
                    }
                    self.tok_code(p, we, word.leaf);
                    if word.class != C_MODULE && we == e && next == Some(b'.') {
                        return Some(self.after_object_code(e));
                    }
                    Some(we)
                }
                // `literal-constructor`, where no method, call or member's pattern comes first
                C_NEW if next != Some(b'(') && b.get(e) != Some(&b'.') => self.constructor(p),
                // `literal-operators`' words, likewise
                C_OP if !matches!(next, Some(b'(' | b'`')) && b.get(e) != Some(&b'.') => {
                    self.tok_code(p, we, OP)
                }
                _ => None,
            };
        }
        // `literal-variable`'s: a property after a `.`, else a variable; a name starting
        // uppercase or with `_` by its bytes (X-058: every name in an expression is one color in
        // the themes, so its shape is read here, not by `literal-variable`'s patterns again)
        let var = if after_dot { PROPERTY } else { READWRITE };
        // past a `.` or `[` the object's kind is `upper_member`'s, which reads `Foo.bar(` in one
        // turn and leaves a member a function is assigned to (`entity.name.class`) to the dispatch
        let var_at = |e| if upper { upper_kind(b, p, e, var) } else { var };
        match next {
            Some(b'=') => {
                let mut group = None;
                let assign = match at(b, s + 1) {
                    // `literal-arrow-function`'s parameter
                    Some(b'>') => return None,
                    Some(b'=') => false,
                    _ => {
                        // `literal-function`'s or `literal-arrow-function`'s name; an arrow's
                        // group decided at its `)`
                        let v = spaces(b, s + 1);
                        if b.get(v) == Some(&b'(') && self.npending < PENDING {
                            group = Some(v);
                        } else if self.function_value(v) {
                            return None;
                        }
                        true
                    }
                };
                let undo = (self.k.mark(self.out), self.at);
                let q = self.tok_code(p, e, var_at(e))?;
                if let Some(v) = group {
                    self.prefix = Some(Prefix {
                        at: v,
                        name: p,
                        undo,
                    });
                }
                // the `=` after blanks: `literal-operators`' assignment
                if q == e && assign && b[e..s].iter().all(|&c| c == b' ' || c == b'\t') {
                    return self.tok_code(s, s + 1, ASSIGN);
                }
                Some(q)
            }
            Some(b'(') => {
                let c = spaces(b, s + 1);
                if self.is(c, b')') {
                    // `()`: a method's key where a body follows, read here
                    if body_follows_in(b, Some(c + 1)) {
                        return None;
                    }
                    self.tok_code(p, e, FUNCTION);
                    return self.tok_code(s, c + 1, CALL_PARENS);
                }
                if self.npending == PENDING {
                    // `literal-method`'s key, then the call's `(` where no arrow follows
                    if self.body_follows(s) {
                        return None;
                    }
                    self.tok_code(p, e, FUNCTION);
                    if !self.arrow_follows(s) && self.can_push() {
                        return self.open_code(s, s + 1, ROUND_BEGIN, ROUND);
                    }
                    return Some(s);
                }
                let (undo, at) = (self.k.mark(self.out), self.at);
                self.tok_code(p, e, FUNCTION);
                if !self.can_push() {
                    return Some(s);
                }
                self.open_code(s, s + 1, ROUND_BEGIN, ROUND);
                self.defer(Pending {
                    undo,
                    at: at as u32,
                    slow: p as u32,
                    depth: self.depth as u16,
                    call: true,
                });
                Some(s + 1)
            }
            Some(b'.' | b'[') if upper => self.upper_member(p, e),
            Some(b'.' | b'[') => {
                if self.is(e, b'.') && has(b, e + 1, b"prototype") {
                    return None;
                }
                if after_dot {
                    self.tok_code(p, e, PROPERTY);
                    if s != e {
                        return Some(e);
                    }
                } else {
                    self.tok_code(p, s, OBJECT);
                }
                Some(self.after_object_code(s))
            }
            Some(b':') if core => {
                // the labels' functions and arrows, then `literal-labels`' key
                if self.function_value(spaces(b, s + 1)) {
                    return None;
                }
                let after_q = before(b, p) == Some(b'?')
                    || space_before(b, p).is_some_and(|s| s > 0 && b[s - 1] == b'?');
                if after_q || after_dot {
                    return self.tok_code(p, e, var_at(e));
                }
                self.tok_code(p, e, OBJ_KEY_LABEL);
                self.tok_code(e, s, OBJ_KEY);
                self.tok_code(s, s + 1, OBJ_KEY_SEP)
            }
            Some(b'`') => None,
            _ => self.tok_code(p, e, var_at(e)),
        }
    }

    /// Bytes `s..e` are `leaf`, in a code frame: its text is plain and the spans end at or before
    /// `s`, so the token goes straight to the sink. On the run sink one call (`merge_call`): the
    /// run copy inline at each place was 16.5 KB more for no time measured (X-058).
    #[inline(always)]
    fn tok_code(&mut self, s: usize, e: usize, leaf: u16) -> Option<usize> {
        debug_assert!(self.text == PLAIN && s >= self.at && e <= self.b.len());
        self.k.merge_call(&mut self.out, s, e, leaf);
        self.at = e;
        Some(e)
    }

    /// [`Scan::after_object`] in a code frame, where the dispatch goes straight to `expression`
    /// ([`Scan::straight`]): the `.` before a member, or a `[`, after a name ending at `s`.
    #[inline(always)]
    fn after_object_code(&mut self, s: usize) -> usize {
        match self.b.get(s) {
            Some(b'.') if self.dot_code(s) => {
                self.tok_code(s, s + 1, ACCESSOR);
                self.member = s + 1;
                s + 1
            }
            Some(b'[') if self.can_push() => {
                self.tok_code(s, s + 1, SQUARE_BEGIN);
                self.push(SQUARE);
                s + 1
            }
            _ => s,
        }
    }

    /// [`Scan::plain_dot`] at `d`, its member's name read once: whether the `.` can only be the
    /// accessor.
    #[inline(always)]
    fn dot_code(&self, d: usize) -> bool {
        let b = self.b;
        let m = d + 1;
        match b.get(m) {
            Some(b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$') => {}
            Some(&c) if c >= 0x80 => return self.plain_dot(d),
            _ => return false,
        }
        let mut e = m + 1;
        while e < b.len() && NAME[usize::from(b[e])] {
            e += 1;
        }
        if e < b.len() && b[e] >= 0x80 {
            return self.plain_dot(d);
        }
        if crate::same(&b[m..e], b"prototype") {
            return false;
        }
        let s = match b.get(e) {
            Some(&c) if c < 0x80 && !matches!(c, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r') => e,
            _ => spaces(b, e),
        };
        if at(b, s) != Some(b'=') {
            return true;
        }
        matches!(at(b, s + 1), Some(b'=' | b'>')) || !self.function_value(spaces(b, s + 1))
    }

    /// Punctuation, a digit or a quote at `p` (`c`) in code frame `top`: the frame's closer,
    /// then the dispatch's patterns for the byte where they're few; `None` for the dispatch,
    /// nothing written.
    #[inline(always)]
    fn punct_turn(&mut self, p: usize, c: u8, top: u8) -> Option<usize> {
        let b = self.b;
        let closer = match top {
            CURLY => (c == b'}').then_some(CURLY_END),
            ROUND => (c == b')').then_some(ROUND_END),
            SQUARE => (c == b']').then_some(SQUARE_END),
            SUBST => (c == b'}').then_some(SUBST_END),
            JSX_EXPR => (c == b'}').then_some(EMBED_END),
            GQL_SUBST => (c == b'}').then_some(PLAIN),
            STYLED_ARGS => (c == b')').then_some(GROUP_END),
            // a closer that leaves, taking nothing, for the context below's: the `switch`'s `}`, the
            // `for`'s `)`, `case`'s `:` the `switch` body's
            SWITCH_BODY | FOR_IN if c == if top == FOR_IN { b')' } else { b'}' } => {
                let (below, leaf) = if top == FOR_IN {
                    (FOR, ROUND_END)
                } else {
                    (SWITCH, CURLY_END)
                };
                if self.loop_at == (p, self.depth)
                    || self.depth < 3
                    || self.stack[self.depth - 2] != below
                {
                    return None;
                }
                self.pop();
                self.tok_code(p, p + 1, leaf);
                self.pop();
                return Some(p + 1);
            }
            CASE if c == b':' => {
                if self.loop_at == (p, self.depth)
                    || self.depth < 3
                    || self.stack[self.depth - 2] != SWITCH_BODY
                {
                    return None;
                }
                self.pop();
                return self.colon_code(p);
            }
            PARAM_DEFAULT if matches!(c, b',' | b')') => return None,
            _ => None,
        };
        if top == ROUND && c == b')' && self.npending > 0 && self.decide(p) {
            return None;
        }
        if let Some(leaf) = closer {
            if leaf == PLAIN {
                self.at = p + 1;
            } else {
                self.tok_code(p, p + 1, leaf);
            }
            self.pop();
            return Some(p + 1);
        }
        let core = matches!(top, MAIN | CURLY | SWITCH_BODY | GQL_SUBST);
        match c {
            // `literal-punctuation`'s, in `core` and a `for`'s head; elsewhere no pattern's
            b';' | b',' => {
                if core || top == FOR_IN {
                    let leaf = if c == b';' { TERMINATOR } else { COMMA };
                    self.tok_code(p, p + 1, leaf)
                } else {
                    Some(p + 1)
                }
            }
            b'(' => {
                let prefix = self.prefix.take().filter(|x| x.at == p);
                if self.npending == PENDING {
                    if self.arrow_follows(p) {
                        return None;
                    }
                    return self.open_code(p, p + 1, ROUND_BEGIN, ROUND);
                }
                let ((undo, at), slow) = match prefix {
                    Some(x) => (x.undo, x.name),
                    None => ((self.k.mark(self.out), self.at), p),
                };
                self.open_code(p, p + 1, ROUND_BEGIN, ROUND)?;
                self.defer(Pending {
                    undo,
                    at: at as u32,
                    slow: slow as u32,
                    depth: self.depth as u16,
                    call: false,
                });
                Some(p + 1)
            }
            b'[' => self.open_code(p, p + 1, SQUARE_BEGIN, SQUARE),
            b'{' if b.get(p + 1) != Some(&b'{') => self.open_code(p, p + 1, CURLY_BEGIN, CURLY),
            b'.' if self.dot_code(p) => self.tok_code(p, p + 1, ACCESSOR),
            b'"' | b'\''
                if self.depth + 2 < SYNTECT_DEPTH.min(MAX)
                    && !(after_paren(b, p) && self.regexp_arg(p)) =>
            {
                self.string_code(p, core)
            }
            b'"' | b'\'' => {
                if let Some(e) = quoted(b, p) {
                    let n = at(b, spaces(b, e));
                    if n == Some(b'(') || (core && n == Some(b':')) {
                        return None;
                    }
                }
                self.string(p)
            }
            b'`' if !(core && has(b, p, b"`#graphql")) => self.template_start(p),
            b'0'..=b'9' => {
                // a key before `:` in `core`, a method's before its `(`: digits alone read here
                if let Some(q) = self.number_code(p, core) {
                    return q;
                }
                let n = self.name_at(p);
                if core && n.next == Some(b':') {
                    return None;
                }
                let k = digits(b, p)?;
                if self.is(spaces(b, k), b'(') {
                    return None;
                }
                self.number(p)
            }
            b'/' if matches!(b.get(p + 1), Some(b'/' | b'*')) => self.comments(p),
            b'=' | b'|' | b'>' if p > 0 => self.op_code(p, c),
            b'!' | b'%' | b'&' | b'^' | b'~' | b'?' => self.op_code(p, c),
            b':' if !core => self.op_code(p, c),
            b':' => self.colon_code(p),
            b'<' if p > 0
                && !has(b, p, b"<!--")
                && !(ident_start(b.get(p + 1).copied()) && self.jsx_may(p)) =>
            {
                self.op_code(p, c)
            }
            b'/' if p > 0 && !self.regex_may(p) => self.op_code(p, c),
            b'.' if has(b, p, b"...") && (p == 0 || b[p - 1] != b'.') => {
                self.tok_code(p, p + 3, SPREAD)
            }
            b'+' | b'-' if !self.signed_number(p) && !has(b, p, b"-->") => self.op_code(p, c),
            b'+' => self.number(p).or_else(|| self.operators(p)),
            b'-' => self
                .number(p)
                .or_else(|| self.comments(p))
                .or_else(|| self.operators(p)),
            _ => None,
        }
    }

    /// Digits at `p` that are a whole word (no fraction, exponent, base or suffix after them):
    /// `Some` with the dispatch's answer, a number, or `Some(None)` for the dispatch (a key, a
    /// method's name); `None` where the slow read decides.
    #[inline(always)]
    fn number_code(&mut self, p: usize, core: bool) -> Option<Option<usize>> {
        let b = self.b;
        if p > 0 && (WORD[usize::from(b[p - 1])] || b[p - 1] >= 0x80) {
            return None;
        }
        let mut d = p + 1;
        while d < b.len() && b[d].is_ascii_digit() {
            d += 1;
        }
        let s = match b.get(d) {
            Some(&c) if c >= 0x80 || WORD[usize::from(c)] || c == b'$' || c == b'.' => return None,
            Some(b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r') | None => spaces(b, d),
            Some(_) => d,
        };
        let next = at(b, s);
        if next == Some(b'(') || (core && next == Some(b':')) {
            return Some(None);
        }
        Some(self.tok_code(p, d, NUMBER))
    }

    /// A string at `p`, not after `(` (where `RegExp(…)` would take it): one that closes on the
    /// line before any `\\` whole in this turn, a key's (`"k": v`, in `core`) with its `:`; else
    /// [`Scan::string`]'s, or `None` for the dispatch.
    #[inline(always)]
    fn string_code(&mut self, p: usize, core: bool) -> Option<usize> {
        let b = self.b;
        let q = b[p];
        let i = crate::prim::find_in(b, p + 1, b.len(), [q, b'\\']);
        if i >= b.len() || b[i] != q {
            // an escape or the line's end first: the string's own context
            return self.string_slow(p, core);
        }
        let s = spaces(b, i + 1);
        let next = at(b, s);
        if next == Some(b'(') {
            return None;
        }
        if core && next == Some(b':') {
            // `literal-labels`' key, unless after a `?` or a function or arrow follows
            if before(b, p) == Some(b'?')
                || space_before(b, p).is_some_and(|t| t > 0 && b[t - 1] == b'?')
                || self.function_value(spaces(b, s + 1))
            {
                return None;
            }
            // `literal-labels`' empty match pushed at `p`, then its string and its `:`
            self.loop_at = (p, self.depth + 1);
            self.str_code(p, i);
            return self.tok_code(s, s + 1, KEY_SEP);
        }
        self.str_code(p, i);
        Some(i + 1)
    }

    /// [`Scan::string`]'s answer at `p` where the string doesn't close before an escape.
    #[inline(never)]
    fn string_slow(&mut self, p: usize, core: bool) -> Option<usize> {
        if let Some(e) = quoted(self.b, p) {
            let n = at(self.b, spaces(self.b, e));
            if n == Some(b'(') || (core && n == Some(b':')) {
                return None;
            }
        }
        self.string(p)
    }

    /// A string `p..=i`, its quotes at both ends and none between.
    #[inline(always)]
    fn str_code(&mut self, p: usize, i: usize) {
        self.tok_code(p, p + 1, STR_BEGIN);
        self.tok_code(p + 1, i, STR);
        self.tok_code(i, i + 1, STR_END);
    }

    /// Bytes `p..e`, `leaf`, that open frame `c`; `None` at syntect's depth.
    #[inline(always)]
    fn open_code(&mut self, p: usize, e: usize, leaf: u16, c: u8) -> Option<usize> {
        if !self.can_push() {
            return None;
        }
        self.tok_code(p, e, leaf);
        self.push(c);
        Some(e)
    }

    /// Whether a number may start at the sign at `p` (`literal-number`'s `\B[-+]` before a digit
    /// or a `.`, or `Infinity`): where [`Scan::number`] is asked first.
    #[inline(always)]
    fn signed_number(&self, p: usize) -> bool {
        let b = self.b;
        matches!(b.get(p + 1), Some(b'0'..=b'9' | b'.' | b'I'))
    }

    /// `literal-operators`' punctuation at `p` (`c`), as one `match` on the bytes there.
    #[inline(always)]
    fn op_code(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        let at1 = b.get(p + 1).copied();
        let at2 = b.get(p + 2).copied();
        let at3 = b.get(p + 3).copied();
        let (len, leaf) = match c {
            b'!' if at1 != Some(b'=') => (1, LOGICAL),
            b'!' if at2 == Some(b'=') => (3, COMPARISON),
            b'!' => (2, COMPARISON),
            b'&' | b'|' if at1 == Some(c) => (2, LOGICAL),
            b'=' if at1 != Some(b'=') => (1, ASSIGN),
            b'=' if at2 == Some(b'=') => (3, COMPARISON),
            b'=' => (2, COMPARISON),
            b'%' | b'&' | b'*' | b'+' | b'-' | b'/' | b'^' | b'|' if at1 == Some(b'=') => {
                (2, AUGMENTED)
            }
            b'<' if at1 == Some(b'<') && at2 == Some(b'=') => (3, AUGMENTED),
            b'>' if at1 == Some(b'>') && at2 == Some(b'=') => (3, AUGMENTED),
            b'>' if at1 == Some(b'>') && at2 == Some(b'>') && at3 == Some(b'=') => (4, AUGMENTED),
            b'~' => (1, BITWISE),
            b'<' if at1 == Some(b'<') => (2, BITWISE),
            b'>' if at1 == Some(b'>') && at2 == Some(b'>') => (3, BITWISE),
            b'>' if at1 == Some(b'>') => (2, BITWISE),
            b'&' | b'^' | b'|' => (1, BITWISE),
            b'<' | b'>' if at1 == Some(b'=') => (2, RELATIONAL),
            b'<' | b'>' => (1, RELATIONAL),
            b'-' | b'+' if at1 == Some(c) => (2, ARITHMETIC),
            b'/' | b'%' | b'*' | b'+' | b'-' => (1, ARITHMETIC),
            b'?' | b':' => (1, TERNARY),
            _ => return self.operators(p),
        };
        self.tok_code(p, p + len, leaf)
    }

    /// `core`'s `:` at `p`: `literal-labels`' key separator (its empty match pushed, then the `:`
    /// that pops it), but after a `?`, where it is the ternary's.
    #[inline(always)]
    fn colon_code(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if before(b, p) == Some(b'?')
            || space_before(b, p).is_some_and(|t| t > 0 && b[t - 1] == b'?')
        {
            return self.tok_code(p, p + 1, TERNARY);
        }
        if !self.can_push() {
            return self.tok_code(p, p + 1, TERNARY);
        }
        if self.depth == MAX {
            return None;
        }
        self.loop_at = (p, self.depth + 1);
        self.tok_code(p, p + 1, KEY_SEP)
    }

    /// Whether `literal-jsx` may start at the `<` at `p` (not the line's start), by what precedes
    /// it.
    #[inline(always)]
    fn jsx_may(&self, p: usize) -> bool {
        let b = self.b;
        matches!(b[p - 1], b'(' | b'{' | b'[' | b',' | b'?' | b':' | b'=')
            || (p >= 2 && matches!(&b[p - 2..p], b"&&" | b"||" | b"=>"))
            || self.after_word(p, b"return")
            || self.after_word(p, b"default")
    }

    /// Whether `literal-regexp` may start at the `/` at `p` (not the line's start, nor a
    /// comment's): by what precedes it.
    #[inline(always)]
    fn regex_may(&self, p: usize) -> bool {
        let b = self.b;
        if p + 1 >= b.len() {
            return false;
        }
        matches!(
            b[p - 1],
            b'.' | b'('
                | b','
                | b'{'
                | b'}'
                | b'['
                | b';'
                | b'<'
                | b'>'
                | b'='
                | b'!'
                | b'+'
                | b'-'
                | b'*'
                | b'%'
                | b'&'
                | b'|'
                | b'^'
                | b'~'
                | b'?'
                | b':'
                | b'/'
        ) || BEFORE_REGEX.iter().any(|w| self.after_word(p, w))
    }

    /// A group pending on [`Scan::pending`].
    #[inline(always)]
    fn defer(&mut self, g: Pending) {
        self.pending[self.npending].write(g);
        self.npending += 1;
    }

    /// The `)` at `p` that closes the group on top: where it was pending, what follows decides
    /// it. A call's before `{` (or a comment, or a space past ASCII) is a method's head, a group's
    /// or call's before `=>` an arrow's parameters: what was written since its undo point taken
    /// back and its place handed to the dispatch ([`Scan::slow_at`], where the loop leaves):
    /// `true`. `false` for a plain group, its `)` the closer's.
    #[inline(always)]
    fn decide(&mut self, p: usize) -> bool {
        // groups left pending above this one (none but where the stack moved) are dropped
        while self.npending > 0 {
            // SAFETY: entries below the count are written
            let g = unsafe { self.pending[self.npending - 1].assume_init() };
            let depth = usize::from(g.depth);
            if depth > self.depth {
                self.npending -= 1;
                continue;
            }
            if depth < self.depth {
                return false;
            }
            self.npending -= 1;
            let b = self.b;
            let mut t = p + 1;
            while t < b.len() && matches!(b[t], b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r') {
                t += 1;
            }
            // a method's body may follow a comment (`body_follows_in`), an arrow's `=>` only
            // blanks; past ASCII a blank may be `\s`'s
            let comment = b.get(t) == Some(&b'/') && b.get(t + 1) == Some(&b'*');
            // a call's arrow (`f(a) => …`) read again from its name: the dispatch writes the call's
            // name as it was and leaves its `(` to the arrow's lookahead
            match b.get(t) {
                Some(b'{') if g.call => {}
                Some(b'/') if g.call && comment => {}
                Some(0x80..) => {}
                Some(b'=') if b.get(t + 1) == Some(&b'>') => {}
                _ => return false,
            }
            self.k.undo(self.out, g.undo);
            self.at = g.at as usize;
            self.pop();
            self.slow_at = g.slow as usize;
            return true;
        }
        false
    }
}
