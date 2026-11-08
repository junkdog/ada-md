//! C++'s contexts: each arm its context's patterns in the grammar's order, and the groups of
//! patterns they include (`expressions` and its parts, `identifiers`, `global`, `statements`,
//! `data-structures-body`).

use super::text::{
    at, boundary, eol_after, find_any, generic, id, id_loose, ident_end, is_ident, is_kw,
    is_upper_rest, is_word, next, non_space_at, non_word_at, operator, operator_name, paren_after,
    paren_or_eol, path, path_may_end, path_then, prev, punct_at, run_of, space_at, spaces_generic,
    starts, word_before, word_end,
};
use super::words::{Kw, c99_at};
use super::*;
use crate::{is_space, skip_spaces};

/// Which `#if` rules a context's preprocessor include: `-global`, `-statements` or
/// `-data-structures`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Rule {
    Global,
    Statements,
    Data,
}

/// The words `(?=\b(…)\b)` stops before in `global-type` and `data-structures-type` (with
/// `operator`), `global-maybe-function` and `data-structures-maybe-method` (without).
pub(super) fn is_stop_word(w: &[u8], with_operator: bool) -> bool {
    matches!(
        w,
        b"break"
            | b"case"
            | b"catch"
            | b"continue"
            | b"default"
            | b"do"
            | b"else"
            | b"for"
            | b"goto"
            | b"if"
            | b"_Pragma"
            | b"return"
            | b"switch"
            | b"throw"
            | b"try"
            | b"while"
            | b"and"
            | b"and_eq"
            | b"bitand"
            | b"bitor"
            | b"compl"
            | b"not"
            | b"not_eq"
            | b"or"
            | b"or_eq"
            | b"xor"
            | b"xor_eq"
            | b"noexcept"
            | b"const_cast"
            | b"dynamic_cast"
            | b"reinterpret_cast"
            | b"static_cast"
            | b"new"
            | b"delete"
            | b"typedef"
            | b"nullptr"
            | b"private"
            | b"protected"
            | b"public"
            | b"static_assert"
            | b"sizeof"
            | b"using"
            | b"typeid"
            | b"alignof"
            | b"alignas"
            | b"namespace"
            | b"template"
    ) || (with_operator && w == b"operator")
}

/// `(struct|union|enum\s+class|enum\s+struct|enum|class)` at `p`, without `\b`s: where each
/// alternative ends, in order.
pub(super) fn ds_keyword(b: &[u8], p: usize) -> [Option<usize>; 2] {
    let mut out = [None, None];
    // the first byte tells which word it may be (no call to compare the others)
    match at(b, p) {
        b's' | b'u' => {
            for (w, n) in [(&b"struct"[..], 6), (b"union", 5)] {
                if starts(b, p, w) {
                    out[0] = Some(p + n);
                    return out;
                }
            }
            return out;
        }
        b'e' | b'c' => {}
        _ => return out,
    }
    if starts(b, p, b"enum") {
        let s = skip_spaces(b, p + 4);
        if s > p + 4 && (starts(b, s, b"class") || starts(b, s, b"struct")) {
            out[0] = Some(s + if b[s] == b'c' { 5 } else { 6 });
            out[1] = Some(p + 4);
        } else {
            out[0] = Some(p + 4);
        }
        return out;
    }
    if starts(b, p, b"class") {
        out[0] = Some(p + 5);
    }
    out
}

/// C's `operators` at a byte `c`, `d` and `e` after it (`\n` past the line): how many bytes
/// the operator takes and its leaf (C's scopes added by the caller).
#[inline(always)]
pub(super) fn operator_at(c: u8, d: u8, e: u8) -> Option<(usize, u16)> {
    Some(match c {
        b'+' | b'-' if d == c => (2, ARITH),
        b'-' if d == b'>' => (2, ACCESSOR),
        b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^' if d == b'=' => (2, ASSIGN_AUG),
        b'>' | b'<' if d == c && e == b'=' => (3, ASSIGN_AUG),
        b'<' | b'>' | b'&' | b'|' if d == c => (2, ARITH),
        b'<' | b'>' | b'=' | b'!' if d == b'=' => (2, COMPARISON),
        b'<' | b'>' => (1, COMPARISON),
        b'+' | b'-' | b'/' | b'%' | b'|' | b'^' | b'~' | b'!' => (1, ARITH),
        b'*' | b'&' => (1, OPERATOR),
        b'=' => (1, ASSIGN),
        b'?' => (1, TERNARY),
        b':' if d != b':' => (1, TERNARY),
        b'.' if d == b'.' && e == b'.' => (3, VARIADIC),
        _ => return None,
    })
}

/// `\b(struct|union|enum\s+class|enum\s+struct|enum|class)\b` at `p`: where it ends.
fn ds_keyword_word(b: &[u8], p: usize) -> Option<usize> {
    if word_before(b, p) {
        return None;
    }
    ds_keyword(b, p)
        .into_iter()
        .flatten()
        .find(|&e| !is_word(b, e))
}

/// The forward declaration's lookahead after a name: `(\s+\b[\p{Lu}_][\p{Lu}\p{Nd}_]{2,}\b)*
/// \s*(:\s*((PATH|private|protected|public|,|\s|<[^;]*>)+)?;)` at `e`.
fn forward_decl_after(b: &[u8], e: usize) -> bool {
    // the macros' words, greedily; fewer of them can't help (the next is a word, not `:;`)
    let mut q = e;
    loop {
        let s = skip_spaces(b, q);
        if s == q
            || !upper_macro(b, s).is_some_and(|m| {
                q = m;
                true
            })
        {
            break;
        }
    }
    let s = skip_spaces(b, q);
    match at(b, s) {
        b';' => s < b.len(),
        b':' => {
            // `\s*` then the list, which takes spaces too; `;` must end it
            let mut i = s + 1;
            loop {
                let c = at(b, i);
                if i >= b.len() {
                    return false;
                }
                if c == b';' {
                    return true;
                }
                if is_space(c) || c == b',' {
                    i += 1;
                } else if c == b'<' {
                    // `<[^;]*>`: greedy to the last `>` before a `;`, backtracking to earlier
                    let stop = b[i..]
                        .iter()
                        .position(|&x| x == b';')
                        .map_or(b.len(), |k| i + k);
                    let Some(gt) = b[i..stop].iter().rposition(|&x| x == b'>') else {
                        return false;
                    };
                    i += gt + 1;
                } else if let Some(e) = path(b, i, |_, e| Some(e)) {
                    i = e;
                } else {
                    return false;
                }
            }
        }
        _ => false,
    }
}

/// Whether no pattern of context `c` (C++'s or C's) can start at a space past the line's start: those
/// whose arms try their line-start rules at 0 only, single bytes that aren't spaces, words, and
/// `expressions`, which starts no pattern at such a space.
pub(super) const fn spaces_inert(c: u8) -> bool {
    matches!(
        c,
        BLOCK
            | PAREN
            | BRACKETS
            | BRACE_EXPR
            | FN_BLOCK
            | FN_PARAMS_IN
            | METHOD_PARAMS_IN
            | FN_PARAM_DEFAULT
            | METHOD_PARAM_DEFAULT
            | ANGLE
            | CAST_ARGS
            | CASE
            | ENUM_BODY
            | FRIEND_BLOCK
            | NEGATED
            | TEMPLATE_ARGS
            // C's
            | C_PAREN
            | C_MACRO_CALL
            | C_BRACKETS
            | C_BLOCK
            | C_CASE
            | C_FN_BLOCK
            | C_NEGATED
            | C_STRUCT_BODY
            | C_ENUM_BODY
            | C_UNION_BODY
            | C_FN_PARAMS_IN
    )
}

/// Whether context `c` (C++'s or C's) takes a `,` past the line's start as `expressions` does, a
/// separator that leaves it open: [`spaces_inert`]'s but those that pop before a `,`.
pub(super) const fn comma_inert(c: u8) -> bool {
    spaces_inert(c) && !matches!(c, CASE | FN_PARAM_DEFAULT | METHOD_PARAM_DEFAULT)
}

/// Whether context `c` (C++'s or C's), past the line's start, gives a name or a digit to
/// `expressions` with `function-call` before any pattern of its own: its patterns before are a
/// closing bracket's and a branch directive's (at the line's start only).
pub(super) const fn expr_direct(c: u8) -> bool {
    matches!(
        c,
        BLOCK
            | PAREN
            | BRACKETS
            | BRACE_EXPR
            | ENUM_BODY
            | FRIEND_BLOCK
            | NEGATED
            // C's, to C's `expressions`
            | C_PAREN
            | C_MACRO_CALL
            | C_BRACKETS
            | C_BLOCK
            | C_CASE
            | C_NEGATED
    )
}

/// `\b[\p{Lu}_][\p{Lu}\p{Nd}_]{2,}\b` at `p`: where it ends.
fn upper_macro(b: &[u8], p: usize) -> Option<usize> {
    if word_before(b, p) || !(super::text::is_upper(b, p) || at(b, p) == b'_') {
        return None;
    }
    let e = super::text::upper_rest_end(b, next(b, p));
    (e >= p + 3 && !is_word(b, e)).then_some(e)
}

impl<K: Sink> Scan<'_, '_, K> {
    /// The patterns of C++'s context `top` at `p`.
    pub(super) fn cxx_context(&mut self, top: u8, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        match top {
            MAIN => self.rules(p, Rule::Global).or_else(|| self.global(p, c)),
            G_TEMPLATE => self.template(p).or_else(|| self.any_set(p, &[G_MODIFIER])),
            G_COMMENT_AFTER => {
                let q = skip_spaces(b, p);
                if is_word(b, q) {
                    return Some(self.shift(q, &[G_MODIFIER]));
                }
                self.can_pop(p).then(|| self.leave(p))
            }
            G_EXTERN => self
                .comments(p, c)
                .or_else(|| self.strings(p, c))
                .or_else(|| (c == b'{').then(|| self.then(p, 1, &[G_EXTERN_BLOCK], BLOCK_BEGIN)))
                .or_else(|| self.any_set(p, &[G_MODIFIER])),
            G_EXTERN_BLOCK | G_EXTERN_BLOCK2 => {
                if top == G_EXTERN_BLOCK
                    && p == 0
                    && let Some(e) = self.ifdef_cplusplus()
                {
                    let q = self.lead;
                    let k = skip_spaces(b, q + 1) + 5;
                    self.put(0, q, PRE);
                    self.put(q, k, IMPORT | PRE);
                    self.put(k, e, PRE);
                    self.set(e, &[G_EXTERN_BLOCK2]);
                    return Some(e);
                }
                if c == b'}' {
                    return Some(self.close(p, 1, BLOCK_END));
                }
                self.rules(p, Rule::Global).or_else(|| self.global(p, c))
            }
            G_MODIFIER => self.global_modifier(p, c),
            G_TYPE => self.global_type(p, c, false),
            DS_TYPE => self.global_type(p, c, true),
            G_MAYBE_FN => self.maybe_function(p, c, false),
            DS_MAYBE_METHOD => self.maybe_function(p, c, true),
            G_OPERATOR => {
                if let Some(r) = self.identifier_path_generic(p, c) {
                    return Some(r);
                }
                let (s, q) = self.colons(p);
                if let Some(e) = operator(b, q, |e| paren_after(b, e).then_some(e)) {
                    self.put(p, s, DOUBLE_COLON);
                    return Some(self.token(q, e - q, ENTITY_FUNCTION));
                }
                paren_or_eol(b, p).then(|| self.shift(p, &[FN_PARAMS]))
            }
            IDENT_POP | INHERITED_NAME => self
                .identifiers(p)
                .or_else(|| self.can_pop(p).then(|| self.leave(p))),
            G_TYPE_STRUCT => self
                .identifiers(p)
                .or_else(|| Some(self.shift(p, &[G_MAYBE_FN]))),
            DS_TYPE_STRUCT => self
                .identifiers(p)
                .or_else(|| Some(self.shift(p, &[DS_MAYBE_METHOD]))),
            G_TYPE_CALL => self
                .function_call(p)
                .or_else(|| self.can_pop(p).then(|| self.leave(p))),
            NS_NAME => self.identifiers(p).or_else(|| Some(self.shift(p, &[NS]))),
            G_MOD_CTOR | G_MOD_DTOR | G_MOD_FN => self.modifier_name(top, p, c),
            G_FN_ID => {
                let (s, q) = self.colons(p);
                if let Some(e) = id(b, q)
                    && !starts(b, skip_spaces(b, e), b"::")
                {
                    self.put(p, s, DOUBLE_COLON);
                    return Some(self.token(q, e - q, ENTITY_FUNCTION));
                }
                self.identifiers(p).or_else(|| self.any_pop(p))
            }
            G_FN_ID_GENERIC => {
                if let Some(r) = self.identifier_path_generic(p, c) {
                    return Some(r);
                }
                let (s, q) = self.colons(p);
                if let Some(e) = id(b, q)
                    && self.angles_then(e, b'(')
                {
                    self.put(p, s, DOUBLE_COLON);
                    return Some(self.token(q, e - q, ENTITY_FUNCTION));
                }
                (c == b'(' && self.can_pop(p)).then(|| self.leave(p))
            }
            FN_PARAMS | METHOD_PARAMS => self.comments(p, c).or_else(|| {
                if c == b'(' {
                    let inner = if top == FN_PARAMS {
                        FN_PARAMS_IN
                    } else {
                        METHOD_PARAMS_IN
                    };
                    return Some(self.then(p, 1, &[inner], GROUP_BEGIN));
                }
                self.any_pop(p)
            }),
            FN_PARAMS_IN | METHOD_PARAMS_IN => {
                if c == b')' {
                    let next = if top == FN_PARAMS_IN {
                        FN_CONTINUE
                    } else {
                        METHOD_CONTINUE
                    };
                    return Some(self.then(p, 1, &[next], GROUP_END));
                }
                if is_kw(b, p, b"void") {
                    return Some(self.token(p, 4, STORAGE_TYPE));
                }
                if let Some(e) = id(b, p) {
                    let q = skip_spaces(b, e);
                    if q < b.len() && matches!(b[q], b'[' | b',' | b')' | b'=') {
                        return Some(self.token(p, e - p, PARAMETER));
                    }
                }
                if c == b'=' {
                    let d = if top == FN_PARAMS_IN {
                        FN_PARAM_DEFAULT
                    } else {
                        METHOD_PARAM_DEFAULT
                    };
                    self.put(p, p + 1, ASSIGN);
                    self.push(p + 1, d);
                    return Some(p + 1);
                }
                self.exprs(p, c, false, true).or_else(|| {
                    (top == FN_PARAMS_IN)
                        .then(|| self.line_continuation(p, c))
                        .flatten()
                })
            }
            FN_PARAM_DEFAULT | METHOD_PARAM_DEFAULT => {
                if (c == b',' || c == b')') && self.can_pop(p) {
                    return Some(self.leave(p));
                }
                self.exprs(p, c, false, true).or_else(|| {
                    (top == FN_PARAM_DEFAULT)
                        .then(|| self.line_continuation(p, c))
                        .flatten()
                })
            }
            FN_CONTINUE | METHOD_CONTINUE => self.fn_continue(top, p, c),
            FN_TRAILING => {
                if let Some(r) = self.comments(p, c) {
                    return Some(r);
                }
                match c {
                    b';' if self.can_pop(p) => return Some(self.leave(p)),
                    b'{' => return Some(self.then(p, 1, &[FN_BLOCK], BLOCK_BEGIN)),
                    _ => {}
                }
                self.function_specifiers(p)
                    .or_else(|| self.trailing_return_type(p, c))
            }
            FN_BODY => (c == b'{').then(|| self.then(p, 1, &[FN_BLOCK], BLOCK_BEGIN)),
            FN_BLOCK => {
                if c == b'}' {
                    return Some(self.close(p, 1, BLOCK_END));
                }
                if self.before_branch(p) && self.can_pop(p) {
                    return Some(self.leave(p));
                }
                if self.ds_ahead(p) {
                    return Some(self.enter(p, &[DS]));
                }
                self.statements(p, c)
            }
            TRAILING_PARENS => {
                if c == b')' {
                    return Some(self.close(p, 1, GROUP_END));
                }
                self.trailing_return_type(p, c)
            }
            NS => {
                if let Some(r) = self.comments(p, c) {
                    return Some(r);
                }
                match c {
                    b'=' => Some(self.token(p, 1, ALIAS)),
                    b';' if self.can_pop(p) => Some(self.leave(p)),
                    b'}' => Some(self.close(p, 1, BLOCK_END)),
                    b'{' => Some(self.open(p, 1, NS_BLOCK, BLOCK_BEGIN)),
                    _ => self.exprs(p, c, true, true),
                }
            }
            NS_BLOCK => {
                if c == b'}' && self.can_pop(p) {
                    return Some(self.leave(p));
                }
                self.rules(p, Rule::Global).or_else(|| self.global(p, c))
            }
            DS => self.data_structures(p, c),
            CLASS_DEF..=ENUM_DEF => self.ds_definition(top, p, c),
            CLASS_NAME..=ENUM_NAME => {
                let after = top - CLASS_NAME + CLASS_AFTER;
                if let Some(e) = id(b, p)
                    && !starts(b, skip_spaces(b, e), b"::")
                {
                    let leaf = [
                        CLASS_NAME_LEAF,
                        STRUCT_NAME_LEAF,
                        UNION_NAME_LEAF,
                        ENUM_NAME_LEAF,
                    ][usize::from(top - CLASS_NAME)];
                    return Some(self.then(p, e - p, &[after], leaf));
                }
                self.identifiers(p)
                    .or_else(|| punct_at(b, p).then(|| self.shift(p, &[after])))
            }
            CLASS_AFTER | STRUCT_AFTER | ENUM_AFTER => {
                if let Some(r) = self.common_begin(p, c) {
                    return Some(r);
                }
                // common-end
                if let Some(r) = self.angle_brackets(p, c) {
                    return Some(r);
                }
                if is_kw(b, p, b"final") {
                    return Some(self.token(p, 5, MODIFIER));
                }
                match c {
                    b':' => {
                        self.put(p, p + 1, SEPARATOR);
                        self.push(p + 1, INHERIT);
                        Some(p + 1)
                    }
                    b';' if self.can_pop(p) => Some(self.leave(p)),
                    b'{' => {
                        let body = top - CLASS_AFTER + CLASS_BODY;
                        Some(self.then(p, 1, &[body], BLOCK_BEGIN))
                    }
                    _ => None,
                }
            }
            UNION_AFTER => {
                if let Some(r) = self.common_begin(p, c) {
                    return Some(r);
                }
                if let Some(r) = self.angle_brackets(p, c) {
                    return Some(r);
                }
                match c {
                    b'{' => Some(self.then(p, 1, &[UNION_BODY], BLOCK_BEGIN)),
                    b';' if self.can_pop(p) => Some(self.leave(p)),
                    _ => None,
                }
            }
            CLASS_BODY | STRUCT_BODY | UNION_BODY => {
                if c == b'}' {
                    return Some(self.close(p, 1, BLOCK_END));
                }
                self.data_structures_body(p, c)
            }
            ENUM_BODY | FRIEND_BLOCK => {
                if c == b'}' {
                    return Some(self.close(p, 1, BLOCK_END));
                }
                self.statements(p, c)
            }
            INHERIT => {
                if let Some(r) = self
                    .comments(p, c)
                    .or_else(|| self.preprocessor_other(p))
                    .or_else(|| self.modifiers_parens(p))
                    .or_else(|| self.modifiers(p))
                {
                    return Some(r);
                }
                let w = self.word(p);
                if matches!(w, b"virtual" | b"private" | b"protected" | b"public") {
                    return Some(self.token(p, w.len(), MODIFIER));
                }
                if path(b, p, |_, e| Some(e)).is_some() {
                    return Some(self.enter(p, &[INHERITED_NAME]));
                }
                if let Some(r) = self.angle_brackets(p, c) {
                    return Some(r);
                }
                match c {
                    b',' => Some(self.token(p, 1, SEPARATOR)),
                    b'{' | b';' if self.can_pop(p) => Some(self.leave(p)),
                    _ => None,
                }
            }
            DS_TEMPLATE => self.template(p).or_else(|| self.any_set(p, &[DS_MODIFIER])),
            DS_MODIFIER => self.ds_modifier(p, c),
            FRIEND => {
                if let Some(r) = self.comments(p, c) {
                    return Some(r);
                }
                if let Some(e) = ds_keyword_word(b, p) {
                    return Some(self.then(p, e - p, &[FRIEND_DS], STORAGE_TYPE));
                }
                if non_space_at(b, p)
                    && b[p..]
                        .iter()
                        .position(|&x| x == b';')
                        .is_some_and(|k| k > 0)
                {
                    return Some(self.shift(p, &[FRIEND_DS]));
                }
                self.any_pop(p)
            }
            FRIEND_DS => match c {
                b';' if self.can_pop(p) => Some(self.leave(p)),
                b'{' => Some(self.then(p, 1, &[FRIEND_BLOCK], BLOCK_BEGIN)),
                _ => self.exprs(p, c, true, false),
            },
            STATIC_ASSERT => (c == b'(').then(|| self.then(p, 1, &[PAREN], GROUP_BEGIN)),
            DS_FN_ID => self.identifiers(p).or_else(|| self.any_pop(p)),
            DS_FN_ID_GENERIC => {
                if let Some(r) = self.angle_brackets(p, c) {
                    return Some(r);
                }
                if id(b, p).is_some() {
                    return Some(self.enter(p, &[DS_FN_ID_GENERIC_IN]));
                }
                (c == b'(' && self.can_pop(p)).then(|| self.leave(p))
            }
            DS_FN_ID_GENERIC_IN => self
                .identifiers(p)
                .or_else(|| (c == b'<' && self.can_pop(p)).then(|| self.leave(p))),
            MC_COLON => (c == b':').then(|| self.then(p, 1, &[MC_INIT], INIT_LIST)),
            MC_INIT => {
                if let Some(e) = id(b, p) {
                    self.put(p, e, MEMBER_VAR);
                    self.push(e, MC_MEMBER);
                    return Some(e);
                }
                if c == b'{' || c == b';' {
                    return Some(self.shift(p, &[METHOD_CONTINUE]));
                }
                self.exprs(p, c, true, true)
            }
            MC_MEMBER => match c {
                b'(' => Some(self.then(p, 1, &[PAREN], GROUP_BEGIN)),
                b'{' => Some(self.then(p, 1, &[BRACE_EXPR], GROUP_BEGIN)),
                _ => self.comments(p, c),
            },
            TEMPLATE => {
                if let Some(r) = self.comments(p, c) {
                    return Some(r);
                }
                if c == b'<' {
                    return Some(self.then(p, 1, &[TEMPLATE_ARGS], GENERIC_BEGIN));
                }
                self.any_set(p, &[TEMPLATE_AFTER])
            }
            TEMPLATE_ARGS => {
                if c == b'>' {
                    return Some(self.close(p, 1, GENERIC_END));
                }
                if starts(b, p, b"...") {
                    return Some(self.token(p, 3, VARIADIC));
                }
                if is_kw(b, p, b"typename") {
                    return Some(self.token(p, 8, STORAGE_TYPE));
                }
                if let Some(e) = ds_keyword_word(b, p) {
                    return Some(self.token(p, e - p, STORAGE_TYPE));
                }
                self.template(p).or_else(|| self.template_common(p, c))
            }
            TEMPLATE_AFTER => {
                if let Some(e) = ds_keyword_word(b, p) {
                    return Some(self.token(p, e - p, STORAGE_TYPE));
                }
                self.template_common(p, c)
            }
            TYPEDEF => {
                let e = id(b, p).unwrap_or(p);
                let s = skip_spaces(b, e);
                if at(b, s) == b';' && s < b.len() {
                    self.put(p, e, TYPEDEF_NAME);
                    if s == p && self.can_pop(p) {
                        return Some(self.leave(p));
                    }
                    self.pop(s);
                    return Some(s);
                }
                if is_kw(b, p, b"struct") {
                    let s = skip_spaces(b, p + 6);
                    if s > p + 6
                        && let Some(e) = id(b, s)
                    {
                        self.put(p, p + 6, STORAGE_TYPE);
                        return Some(e);
                    }
                }
                self.exprs(p, c, false, true)
            }
            CASE => match c {
                b')' | b';' | b',' if self.can_pop(p) => Some(self.leave(p)),
                b':' => Some(self.close(p, 1, SEPARATOR)),
                _ => self.exprs(p, c, true, true),
            },
            CAST => match c {
                b'>' => Some(self.close(p, 1, GENERIC_END)),
                b'<' => Some(self.open(p, 1, CAST_ARGS, GENERIC_BEGIN)),
                _ => None,
            },
            CAST_ARGS => {
                if c == b'>' && self.can_pop(p) {
                    return Some(self.leave(p));
                }
                self.exprs(p, c, false, false)
            }
            ANGLE => {
                if c == b'>' {
                    return Some(self.close(p, 1, GENERIC_END));
                }
                self.exprs(p, c, false, false)
            }
            BLOCK => {
                if self.before_branch(p) && self.can_pop(p) {
                    return Some(self.leave(p));
                }
                if c == b'}' {
                    return Some(self.close(p, 1, BLOCK_END));
                }
                self.statements(p, c)
            }
            BRACKETS => {
                if c == b']' {
                    return Some(self.close(p, 1, BRACKETS_END));
                }
                self.exprs(p, c, true, true)
            }
            PAREN => {
                if c == b')' {
                    return Some(self.close(p, 1, GROUP_END));
                }
                self.exprs(p, c, true, true)
            }
            BRACE_EXPR => {
                if c == b'}' {
                    return Some(self.close(p, 1, GROUP_END));
                }
                self.exprs(p, c, true, true)
            }
            ATTRIBUTE => {
                if c == b'(' {
                    return Some(self.open(p, 1, PAREN, GROUP_BEGIN));
                }
                if let Some(r) = self.strings(p, c) {
                    return Some(r);
                }
                (c == b')' && at(b, p + 1) == b')').then(|| self.close(p, 2, GROUP_END))
            }
            DECLSPEC => {
                if c == b')' {
                    return Some(self.close(p, 1, GROUP_END));
                }
                let w = self.word(p);
                if matches!(
                    w,
                    b"align" | b"allocate" | b"code_seg" | b"deprecated" | b"property" | b"uuid"
                ) {
                    let s = skip_spaces(b, p + w.len());
                    if at(b, s) == b'(' && s < b.len() {
                        self.put(p, p + w.len(), MODIFIER);
                        self.put(s, s + 1, GROUP_BEGIN);
                        self.push(s + 1, DECLSPEC_ARGS);
                        return Some(s + 1);
                    }
                }
                is_declspec_constant(w).then(|| self.token(p, w.len(), CONSTANT_OTHER))
            }
            DECLSPEC_ARGS => {
                if c == b')' {
                    return Some(self.close(p, 1, GROUP_END));
                }
                if let Some(r) = self.numbers(p, c).or_else(|| self.strings(p, c)) {
                    return Some(r);
                }
                let w = self.word(p);
                if matches!(w, b"get" | b"put") {
                    return Some(self.token(p, 3, PARAMETER));
                }
                match c {
                    b',' => Some(self.token(p, 1, SEPARATOR)),
                    b'=' => Some(self.token(p, 1, ASSIGN)),
                    _ => None,
                }
            }
            FC => {
                if let Some(e) = c99_at(b, p) {
                    return Some(self.token(p, e - p, C99 | C));
                }
                let (s, q) = self.colons(p);
                if let Some(e) = id(b, q) {
                    let d = skip_spaces(b, e);
                    if starts(b, d, b"::") {
                        let f = skip_spaces(b, d + 2);
                        self.put(p, s, DOUBLE_COLON);
                        self.put(d, d + 2, DOUBLE_COLON);
                        return Some(f);
                    }
                    self.put(p, s, ACCESSOR);
                    return Some(self.token(q, e - q, FUNCTION));
                }
                match c {
                    b'(' => Some(self.then(p, 1, &[PAREN], GROUP_BEGIN)),
                    b'{' => Some(self.then(p, 1, &[BRACE_EXPR], GROUP_BEGIN)),
                    _ => None,
                }
            }
            GT => {
                if is_kw(b, p, b"template") {
                    return Some(self.token(p, 8, TEMPLATE_KW));
                }
                let (s, q) = self.colons(p);
                if let Some(e) = id(b, q) {
                    let d = skip_spaces(b, e);
                    let open = at(b, d);
                    if d < b.len() && matches!(open, b'<' | b'(' | b'{') {
                        self.put(p, s, DOUBLE_COLON);
                        self.put(q, e, FUNCTION);
                        return Some(match open {
                            b'<' => self.open(d, 1, ANGLE, GENERIC_BEGIN),
                            b'(' => self.then(d, 1, &[PAREN], GROUP_BEGIN),
                            _ => self.then(d, 1, &[BRACE_EXPR], GROUP_BEGIN),
                        });
                    }
                }
                if let Some(r) = self.identifiers(p).or_else(|| self.angle_brackets(p, c)) {
                    return Some(r);
                }
                match c {
                    b'(' => Some(self.then(p, 1, &[PAREN], GROUP_BEGIN)),
                    b'{' => Some(self.then(p, 1, &[BRACE_EXPR], GROUP_BEGIN)),
                    _ => None,
                }
            }
            GT_NAME => self
                .identifiers(p)
                .or_else(|| (c == b'<').then(|| self.then(p, 1, &[ANGLE], GENERIC_BEGIN))),
            MEMBERS => self.members(p, c),
            MEMBERS_TEMPLATE => {
                if let Some(r) = self.comments(p, c) {
                    return Some(r);
                }
                if let Some(e) = id(b, p) {
                    return Some(self.then(p, e - p, &[MEMBERS_TEMPLATE_CALL], MEMBER_FUNCTION));
                }
                self.any_pop(p)
            }
            MEMBERS_TEMPLATE_CALL => {
                if c == b'(' {
                    return Some(self.then(p, 1, &[PAREN], GROUP_BEGIN));
                }
                self.comments(p, c)
                    .or_else(|| self.angle_brackets(p, c))
                    .or_else(|| self.any_pop(p))
            }
            MEMBERS_GENERIC => {
                (c == b'<').then(|| self.then(p, 1, &[MEMBERS_GENERIC_ARGS], GENERIC_BEGIN))
            }
            MEMBERS_GENERIC_ARGS => {
                if c == b'>' {
                    return Some(self.then(p, 1, &[MEMBERS_GENERIC_CALL], GENERIC_END));
                }
                self.exprs(p, c, true, true)
            }
            MEMBERS_GENERIC_CALL => {
                if let Some(r) = self.comments(p, c) {
                    return Some(r);
                }
                if c == b'(' {
                    return Some(self.then(p, 1, &[PAREN], GROUP_BEGIN));
                }
                self.any_pop(p)
            }
            NEGATED => {
                if c == b'{' {
                    return Some(self.close(p, 1, BLOCK_BEGIN));
                }
                if self.before_branch(p) && self.can_pop(p) {
                    return Some(self.leave(p));
                }
                self.statements(p, c)
            }
            NUM_HEX_FLOAT => {
                if matches!(c, b'p' | b'P')
                    && let Some(e) = super::text::exponent(b, p, *b"pP", super::text::digits_quoted)
                {
                    return Some(self.close(p, e - p, HEX_FLOAT));
                }
                if c == b'.' {
                    return Some(self.token(p, 1, HEX_FLOAT_DOT));
                }
                if p >= b.len() {
                    // `\H` takes the `\n`, and the context stays open
                    self.halt = true;
                    return Some(p);
                }
                (!c.is_ascii_hexdigit()).then(|| {
                    let e = next(b, p);
                    self.token(p, e - p, HEX_FLOAT_BAD)
                })
            }
            NUM_BIN | NUM_HEX | NUM_OCT | NUM_DEC => {
                let hex = top == NUM_HEX;
                // `(?:[a-zA-Z_][\p{L}\p{N}_]*|(?=[^\p{L}\p{N}_']))`, `[g-zG-Z_]` after hex
                let starts_suffix = if hex {
                    matches!(c, b'g'..=b'z' | b'G'..=b'Z' | b'_')
                } else {
                    c.is_ascii_alphabetic() || c == b'_'
                };
                if starts_suffix && p < b.len() {
                    let e = ident_end(b, p + 1);
                    let leaf = [BIN_SUFFIX, HEX_SUFFIX, OCT_SUFFIX, DEC_SUFFIX]
                        [usize::from(top - NUM_BIN)];
                    return Some(self.close(p, e - p, leaf));
                }
                if !is_ident(b, p) && c != b'\'' && self.can_pop(p) {
                    return Some(self.leave(p));
                }
                match top {
                    NUM_BIN if matches!(c, b'2'..=b'9') => Some(self.token(p, 1, BIN_BAD)),
                    NUM_OCT if matches!(c, b'8' | b'9') => Some(self.token(p, 1, OCT_BAD)),
                    _ => None,
                }
            }
            RAW_STRING => {
                let d = self.delim_len;
                if c == b')'
                    && b.get(p + 1..p + 1 + d) == Some(&self.delim[..d])
                    && at(b, p + 1 + d) == b'"'
                    && p + 1 + d < b.len()
                {
                    return Some(self.close(p, d + 2, DQ_END));
                }
                None
            }
            _ => None,
        }
    }

    /// Where the next pattern of C++'s context `top` can start, after none did at `p`.
    pub(super) fn cxx_advance(&self, top: u8, p: usize) -> usize {
        let b = self.b;
        let c = b[p];
        match top {
            RAW_STRING => find_any(b, p + 1, *b")"),
            PP_INC_DQ => find_any(b, p + 1, *b"\""),
            PP_INC_LT => find_any(b, p + 1, *b">"),
            IF_COMMENT | ELSE_COMMENT => b.len(),
            // a number's digits (and `'`s) start none of its context's patterns: past them whole
            NUM_HEX_FLOAT => run_of(b, next(b, p), |b, i| b[i].is_ascii_hexdigit()),
            NUM_HEX => run_of(b, next(b, p), |b, i| {
                b[i].is_ascii_hexdigit() || b[i] == b'\''
            }),
            NUM_DEC => run_of(b, next(b, p), |b, i| b[i].is_ascii_digit() || b[i] == b'\''),
            NUM_OCT => run_of(b, next(b, p), |b, i| matches!(b[i], b'0'..=b'7' | b'\'')),
            NUM_BIN => run_of(b, next(b, p), |b, i| matches!(b[i], b'0' | b'1' | b'\'')),
            G_OPERATOR | TYPEDEF => next(b, p),
            _ if is_space(c) => {
                if p == 0 {
                    self.lead
                } else {
                    skip_spaces(b, p)
                }
            }
            // a word that nothing took at its start: its middle can only start a string's
            // prefix (`1fL"x"`)
            _ if is_word(b, p) && !word_before(b, p) => {
                let e = word_end(b, p);
                b[p + 1..e]
                    .iter()
                    .position(|&x| matches!(x, b'L' | b'u' | b'U' | b'R'))
                    .map_or(e, |k| p + 1 + k)
            }
            _ => next(b, p),
        }
    }

    // ---- the groups of patterns

    /// `(?:(::)\s*)?` at `p`: where the `::` ends and where what follows the spaces starts.
    fn colons(&self, p: usize) -> (usize, usize) {
        if starts(self.b, p, b"::") {
            (p + 2, skip_spaces(self.b, p + 2))
        } else {
            (p, p)
        }
    }

    /// `identifiers`: `(?:(::)\s*)?\bID\b\s*(::)\s*`, `(?:(::)\s*)?\bID\b`.
    pub(super) fn identifiers(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let (s, q) = self.colons(p);
        let e = self.id_at(q)?;
        self.put(p, s, DOUBLE_COLON);
        let d = skip_spaces(b, e);
        if starts(b, d, b"::") {
            self.put(d, d + 2, DOUBLE_COLON);
            return Some(skip_spaces(b, d + 2));
        }
        self.fill(e);
        Some(e)
    }

    /// `identifier-path-generic`: angle brackets, `(?:(::)\s*)?\bID\b\s*(?=(<.*>)?\s*(::))\s*`.
    fn identifier_path_generic(&mut self, p: usize, c: u8) -> Option<usize> {
        if let Some(r) = self.angle_brackets(p, c) {
            return Some(r);
        }
        let b = self.b;
        let (s, q) = self.colons(p);
        let e = id(b, q)?;
        let d = skip_spaces(b, e);
        if !self.angles_then_colons(d) {
            return None;
        }
        self.put(p, s, DOUBLE_COLON);
        Some(d)
    }

    /// `(?=(<.*>)?\s*(::))` at `d`.
    fn angles_then_colons(&self, d: usize) -> bool {
        let b = self.b;
        if at(b, d) == b'<' && d < b.len() {
            // `.*` greedy to the last `>`, backtracking to earlier ones
            let mut k = b.len();
            while let Some(j) = b[d + 1..k].iter().rposition(|&x| x == b'>') {
                let j = d + 1 + j;
                if starts(b, skip_spaces(b, j + 1), b"::") {
                    return true;
                }
                k = j;
            }
        }
        starts(b, skip_spaces(b, d), b"::")
    }

    /// `(?=\s*(<.*>)?\s*X)` at `e`, `X` being `open`.
    fn angles_then(&self, e: usize, open: u8) -> bool {
        let b = self.b;
        let d = skip_spaces(b, e);
        let ok = |i: usize| {
            let s = skip_spaces(b, i);
            s < b.len() && b[s] == open
        };
        if at(b, d) == b'<' && d < b.len() {
            let mut k = b.len();
            while let Some(j) = b[d + 1..k].iter().rposition(|&x| x == b'>') {
                let j = d + 1 + j;
                if ok(j + 1) {
                    return true;
                }
                k = j;
            }
        }
        ok(d)
    }

    /// `angle-brackets`: `<(?!<)`.
    pub(super) fn angle_brackets(&mut self, p: usize, c: u8) -> Option<usize> {
        (c == b'<' && at(self.b, p + 1) != b'<' && p < self.b.len())
            .then(|| self.open(p, 1, ANGLE, GENERIC_BEGIN))
    }

    /// `template`: `\btemplate\b`, then `#anon_template_0`.
    fn template(&mut self, p: usize) -> Option<usize> {
        is_kw(self.b, p, b"template").then(|| {
            self.put(p, p + 8, TEMPLATE_KW);
            self.push(p + 8, TEMPLATE);
            p + 8
        })
    }

    /// `template-common`: `(?=[{};])` pops, then expressions.
    fn template_common(&mut self, p: usize, c: u8) -> Option<usize> {
        if matches!(c, b'{' | b'}' | b';') && p < self.b.len() && self.can_pop(p) {
            return Some(self.leave(p));
        }
        self.exprs(p, c, true, true)
    }

    /// C's `comments`, in C++.
    pub(super) fn comments(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        match c {
            b'/' if p < b.len() => {
                let d = at(b, p + 1);
                if p == 0 && starts(b, 0, b"/* =") && b.len() >= 8 && b.ends_with(b"= */") {
                    self.put(0, b.len(), BLOCK_TEXT | C);
                    self.halt = true;
                    return Some(b.len());
                }
                if d == b'*' {
                    return Some(self.open(p, 2, C_COMMENT, COMMENT_MARK | C));
                }
                if p == 0 && starts(b, 0, b"// =") && line_banner(b) {
                    self.put(0, b.len(), BANNER_LINE | C);
                    self.halt = true;
                    return Some(b.len());
                }
                (d == b'/').then(|| self.open(p, 2, C_LINE_COMMENT, LINE_MARK | C))
            }
            b'*' if at(b, p + 1) == b'/' && at(b, p + 2) != b'*' => {
                Some(self.token(p, 2, STRAY_COMMENT_END | C))
            }
            _ => None,
        }
    }

    /// `strings`: C++'s raw strings, then C's.
    pub(super) fn strings(&mut self, p: usize, c: u8) -> Option<usize> {
        if matches!(c, b'L' | b'u' | b'U' | b'R')
            && let Some(r) = self.raw_string(p)
        {
            return Some(r);
        }
        self.c_strings(p, c)
    }

    /// `((?:L|u8|u|U)?R)("([^\(\)\\ ]{0,16})\()` at `p`.
    fn raw_string(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let mut q = p;
        if starts(b, q, b"u8") {
            q += 2;
        } else if matches!(at(b, q), b'L' | b'u' | b'U') && at(b, q + 1) == b'R' {
            q += 1;
        }
        if at(b, q) != b'R' || at(b, q + 1) != b'"' {
            return None;
        }
        let s = q + 2;
        let e = b[s..]
            .iter()
            .position(|&x| matches!(x, b'(' | b')' | b'\\' | b' '))
            .map(|k| s + k)?;
        if b[e] != b'(' || e - s > DELIM || b[s..e].contains(&b'\n') {
            return None;
        }
        self.put(p, q + 1, DQ_PREFIX);
        self.put(q + 1, e + 1, DQ_BEGIN);
        self.delim[..e - s].copy_from_slice(&b[s..e]);
        self.delim_len = e - s;
        self.raw_may = true;
        self.push(e + 1, RAW_STRING);
        Some(e + 1)
    }

    /// `numbers`, C++'s.
    pub(super) fn numbers(&mut self, p: usize, c: u8) -> Option<usize> {
        use super::text::{digits_quoted, exponent};
        let b = self.b;
        if !(c.is_ascii_digit() || c == b'.') || p >= b.len() {
            return None;
        }
        let at_word = !word_before(b, p);
        // the decimal float
        let suffix = |q: usize| -> usize {
            if at(b, q).is_ascii_alphabetic() || at(b, q) == b'_' {
                ident_end(b, q + 1)
            } else {
                q
            }
        };
        let exp = |q: usize| exponent(b, q, *b"eE", digits_quoted);
        // a hex number: no decimal float starts with `0x`
        if c == b'0' && at_word && matches!(at(b, p + 1), b'x' | b'X') && p + 1 < b.len() {
            // digits, then a byte that ends the number and no float's: `NUM_HEX`'s text, and
            // its pop, which takes nothing (and can't loop where the scan hasn't been)
            let q = run_of(b, p + 2, |b, i| b[i].is_ascii_hexdigit() || b[i] == b'\'');
            if q > p + 2 && !is_ident(b, q) && at(b, q) != b'.' && self.depth < MAX {
                self.put(p, p + 2, HEX_BASE);
                self.put(p + 2, q, HEX);
                return Some(q);
            }
            if hex_float_ahead(b, p + 2, true) {
                return Some(self.open(p, 2, NUM_HEX_FLOAT, HEX_FLOAT_BASE));
            }
            return Some(self.open(p, 2, NUM_HEX, HEX_BASE));
        }
        if c.is_ascii_digit() && at_word {
            let d = digits_quoted(b, p).unwrap_or(p + 1);
            let mut dot = None;
            let mut end = None;
            if at(b, d) == b'.' && d < b.len() {
                let f = d + 1;
                if let Some(g) = digits_quoted(b, f) {
                    end = Some(exp(g).unwrap_or(g));
                } else if let Some(g) = exp(f) {
                    end = Some(g);
                } else if at(b, f) != b'.' || f >= b.len() {
                    end = Some(f);
                }
                if end.is_some() {
                    dot = Some(d);
                }
            }
            if end.is_none() {
                end = exp(d);
            }
            if let Some(e) = end {
                let x = suffix(e);
                let mut s = p;
                if let Some(t) = dot {
                    self.put(p, t, FLOAT);
                    self.put(t, t + 1, FLOAT_DOT);
                    s = t + 1;
                }
                self.put(s, e, FLOAT);
                self.put(e, x, FLOAT_SUFFIX);
                return Some(x);
            }
            if matches!(at(b, d), b'f' | b'F') && d < b.len() {
                self.put(p, d, FLOAT);
                return Some(self.token(d, 1, FLOAT_SUFFIX));
            }
        }
        if c == b'.'
            && let Some(g) = digits_quoted(b, p + 1)
        {
            let e = exp(g).unwrap_or(g);
            let x = suffix(e);
            self.put(p, p + 1, FLOAT_DOT);
            self.put(p + 1, e, FLOAT);
            self.put(e, x, FLOAT_SUFFIX);
            return Some(x);
        }
        if !c.is_ascii_digit() || !at_word {
            return None;
        }
        let x = at(b, p + 1);
        if c == b'0' && matches!(x, b'b' | b'B') && p + 1 < b.len() {
            return Some(self.open(p, 2, NUM_BIN, BIN_BASE));
        }
        if c == b'0' && (x.is_ascii_digit() || x == b'\'') && p + 1 < b.len() {
            return Some(self.open(p, 1, NUM_OCT, OCT_BASE));
        }
        let e = run_of(b, p, |b, i| b[i].is_ascii_digit());
        self.put(p, e, DEC);
        // `NUM_DEC` would pop at once, taking nothing
        if !is_ident(b, e) && at(b, e) != b'\'' && self.depth < MAX {
            return Some(e);
        }
        self.push(e, NUM_DEC);
        Some(e)
    }

    /// `expressions` and its variants: with `generic-type` or plain `angle-brackets`, with
    /// `function-call` or not.
    pub(super) fn exprs(&mut self, p: usize, c: u8, generic: bool, call: bool) -> Option<usize> {
        // no pattern of expressions starts with a space (or at the line's end) but the
        // preprocessor's, `^\s*…`, at the line's start
        if is_space(c) {
            return (p == 0 && p < self.b.len())
                .then(|| self.preprocessor_expressions())
                .flatten();
        }
        if let Some(r) = self.name(p, c, call) {
            return Some(r);
        }
        // past the line's start, a byte of punctuation that starts no comment, number, `<`,
        // `::` or string can only be one of `early-expressions-after-generic-type`'s, or `;`
        if p > 0
            && c.is_ascii_punctuation()
            && !matches!(c, b'/' | b'*' | b'.' | b'<' | b':' | b'"' | b'\'' | b'_')
        {
            return self
                .early_after(p, c)
                .or_else(|| (c == b';').then(|| self.token(p, 1, TERMINATOR)));
        }
        // a digit starts nothing of `early-expressions-before-generic-type` before `numbers`
        // (the line-start ones want a `#` or a name)
        if c.is_ascii_digit()
            && let Some(r) = self.numbers(p, c)
        {
            return Some(r);
        }
        if let Some(r) = self.early_before(p, c) {
            return Some(r);
        }
        if generic {
            if let Some(r) = self.generic_type(p) {
                return Some(r);
            }
        } else if let Some(r) = self.angle_brackets(p, c) {
            return Some(r);
        }
        if let Some(r) = self.early_after(p, c) {
            return Some(r);
        }
        if let Some(r) = self.late_before_call(p) {
            return Some(r);
        }
        if call && let Some(r) = self.function_call(p) {
            return Some(r);
        }
        if let Some(r) = self.identifiers(p) {
            return Some(r);
        }
        (c == b';' && p < self.b.len()).then(|| self.token(p, 1, TERMINATOR))
    }

    /// An ASCII name past the line's start that no keyword list holds, in `expressions`, the
    /// common cases at once. With nothing after it that can make more of it (`<`, `(`, `{`,
    /// `::`, a string's quote), every pattern before `identifiers` misses it, and `identifiers`
    /// takes it alone. Followed by spaces and `(` or `{`, every pattern before `function-call`
    /// misses it, and `function-call`'s context (which colors nothing around) takes the name as
    /// a function, C99's or any, and the bracket, which sets a group or a braced expression:
    /// done here without the push that matches nothing (unless that push would be the one
    /// where the scan steps on). Where the scan goes on.
    pub(super) fn name(&mut self, p: usize, c: u8, call: bool) -> Option<usize> {
        let b = self.b;
        if p == 0 || !(c.is_ascii_alphabetic() || c == b'_') || word_before(b, p) {
            return None;
        }
        let e = super::text::ascii_word_end(b, p + 1);
        if e < b.len() && b[e] >= 0x80 {
            return None;
        }
        let q = skip_spaces(b, e);
        let open = if q < b.len() { b[q] } else { b'\n' };
        if open == b':' && at(b, q + 1) == b':' && call {
            return self.qualified_call(p, e, q);
        }
        if matches!(open, b'<' | b':' | b'"' | b'\'') {
            return None;
        }
        // `\w` is the name's bytes here: the whole word ends where it does
        self.word_at.set((p, e));
        // the words whose group takes them whatever follows (but `<`, `::`, a quote): those of
        // `keywords`, the values of `early-expressions-after-generic-type`, `modifiers` and
        // `types` in `late-expressions-before-function-call`
        let kw = self.kw(p);
        let leaf = match kw {
            Kw::Other | Kw::C99 => None,
            Kw::Flow(leaf) | Kw::Value(leaf) | Kw::Support(leaf) => Some(leaf),
            Kw::Control => Some(CONTROL),
            Kw::WordOp => Some(WORD_OP),
            Kw::Modifier => Some(MODIFIER),
            Kw::CxxType => Some(STORAGE_TYPE),
            Kw::CType => Some(STORAGE_TYPE | C),
            _ => return None,
        };
        if let Some(leaf) = leaf {
            return Some(self.token(p, e - p, leaf));
        }
        if !matches!(open, b'(' | b'{') {
            self.fill(e);
            return Some(e);
        }
        if !call || self.stuck >= STUCK || starts(b, p, b"template") {
            return None;
        }
        let leaf = if kw == Kw::C99 { C99 | C } else { FUNCTION };
        self.put(p, e, leaf);
        self.put(q, q + 1, GROUP_BEGIN);
        self.push(q + 1, if open == b'(' { PAREN } else { BRACE_EXPR });
        Some(q + 1)
    }

    /// A qualified call past the line's start in `expressions` with `function-call`, `a::b(`
    /// or `{`, its names ASCII, the `::` at `q` after the first name, `p..e`. Every pattern
    /// before `function-call` misses it (the first name is no keyword's, value's or support
    /// word's, no `<` follows a name, none is `template`), and `function-call`'s context, a
    /// dispatch a name, gives the `::`s, the last name as a function and the bracket, which
    /// sets a group or a braced expression: done here at once, unless a later name is C99's
    /// (that context takes one first) or the push would be the one where the scan steps on.
    /// Where the scan goes on.
    pub(super) fn qualified_call(&mut self, p: usize, e: usize, q: usize) -> Option<usize> {
        let b = self.b;
        if self.stuck >= STUCK || starts(b, p, b"template") {
            return None;
        }
        self.word_at.set((p, e));
        if self.kw(p) != Kw::Other {
            return None;
        }
        let mut colons = [0usize; 8];
        let mut n = 0;
        let mut d = q;
        let (s, t, r) = loop {
            if n == colons.len() {
                return None;
            }
            colons[n] = d;
            n += 1;
            let s = skip_spaces(b, d + 2);
            let c = at(b, s);
            if !(c.is_ascii_alphabetic() || c == b'_') {
                return None;
            }
            let t = super::text::ascii_word_end(b, s + 1);
            if t < b.len() && b[t] >= 0x80 {
                return None;
            }
            let w = &b[s..t];
            if w.starts_with(b"template") || super::words::is_c99(w) {
                return None;
            }
            let r = skip_spaces(b, t);
            if starts(b, r, b"::") {
                d = r;
                continue;
            }
            if r < b.len() && matches!(b[r], b'(' | b'{') {
                break (s, t, r);
            }
            return None;
        };
        for &d in &colons[..n] {
            self.put(d, d + 2, DOUBLE_COLON);
        }
        self.put(s, t, FUNCTION);
        self.put(r, r + 1, GROUP_BEGIN);
        self.push(r + 1, if b[r] == b'(' { PAREN } else { BRACE_EXPR });
        Some(r + 1)
    }

    /// `early-expressions-before-generic-type`.
    fn early_before(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if p >= b.len() {
            return None;
        }
        if p == 0
            && let Some(r) = self.preprocessor_expressions()
        {
            return Some(r);
        }
        if let Some(r) = self.comments(p, c) {
            return Some(r);
        }
        if is_word(b, p) && !c.is_ascii_digit() {
            let w = self.word(p);
            if !w.is_empty()
                && let Some(r) = self.keyword(p, w)
            {
                return Some(r);
            }
        }
        if let Some(r) = self.numbers(p, c) {
            return Some(r);
        }
        (c == b'<' && at(b, p + 1) == b'=').then(|| self.token(p, 2, COMPARISON | C))
    }

    /// The words of `early-expressions-before-generic-type`: `case-default`, `using-namespace`,
    /// `typedef`, `using-alias`, `keywords-angle-brackets`, `keywords-parens`, `keywords`.
    fn keyword(&mut self, p: usize, w: &[u8]) -> Option<usize> {
        let b = self.b;
        let n = w.len();
        match self.kw(p) {
            Kw::Case => Some(self.open(p, n, CASE, CONTROL)),
            Kw::Using => {
                if let Some(r) = self.using_namespace(p) {
                    return Some(r);
                }
                let s = skip_spaces(b, p + 5);
                if s > p + 5 {
                    if is_kw(b, s, b"typename") {
                        return Some(self.token(p, 5, CONTROL));
                    }
                    if let Some(e) = id(b, s) {
                        let d = skip_spaces(b, e);
                        if !(at(b, d) == b'<' || starts(b, d, b"::")) || d >= b.len() {
                            self.put(p, p + 5, CONTROL);
                            return Some(self.token(s, e - s, USING_NAME));
                        }
                    }
                }
                Some(self.token(p, 5, CONTROL))
            }
            Kw::Typedef => {
                self.put(p, p + 7, STORAGE_TYPE);
                self.push(p + 7, TYPEDEF);
                Some(p + 7)
            }
            Kw::Cast => {
                let e = skip_spaces(b, p + n);
                self.put(p, e, CAST_KW);
                self.push(e, CAST);
                Some(e)
            }
            Kw::Parens | Kw::Sizeof => {
                let s = skip_spaces(b, p + n);
                if at(b, s) == b'(' && s < b.len() {
                    self.put(p, p + n, WORD_OP);
                    self.put(s, s + 1, GROUP_BEGIN);
                    self.push(s + 1, PAREN);
                    return Some(s + 1);
                }
                (w == b"sizeof").then(|| self.token(p, n, WORD_OP | C))
            }
            Kw::Flow(leaf) => Some(self.token(p, n, leaf)),
            Kw::Control => Some(self.token(p, n, CONTROL)),
            Kw::Delete => {
                let s = skip_spaces(b, p + 6);
                let e = if starts(b, s, b"[]") { s + 2 } else { p + 6 };
                Some(self.token(p, e - p, CONTROL))
            }
            Kw::New if at(b, p + 3) != b']' || p + 3 >= b.len() => Some(self.token(p, 3, CONTROL)),
            Kw::WordOp => Some(self.token(p, n, WORD_OP)),
            _ => None,
        }
    }

    /// `using-namespace`: `\b(using)\s+(namespace)\b`, then `#anon_using-namespace_0`.
    fn using_namespace(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if !is_kw(b, p, b"using") {
            return None;
        }
        let s = skip_spaces(b, p + 5);
        if s == p + 5 || !starts(b, s, b"namespace") || is_word(b, s + 9) {
            return None;
        }
        self.put(p, p + 5, CONTROL);
        self.put(s, s + 9, CONTROL);
        self.push(s + 9, IDENT_POP);
        Some(s + 9)
    }

    /// `generic-type`'s lookaheads.
    pub(super) fn generic_type(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if starts(b, p, b"template") || !path_may_end(b, p, |c| c == b'<') {
            return None;
        }
        // the first lookahead wants a call after the arguments; the second takes any. Both
        // only push at `p`, so one walk of the path's ends tells them: the first if any end
        // has its arguments and a call, else the second if any has its arguments
        let mut args = false;
        let call = path(b, p, |_, e| {
            let g = spaces_generic(b, e)?;
            args = true;
            let s = skip_spaces(b, g);
            (s < b.len() && matches!(b[s], b'(' | b'{')).then_some(())
        })
        .is_some();
        if call {
            return Some(self.enter(p, &[GT]));
        }
        args.then(|| self.enter(p, &[GT_NAME]))
    }

    /// `early-expressions-after-generic-type`.
    fn early_after(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if p >= b.len() {
            return None;
        }
        if c == b'-' && at(b, p + 1) == b'>' {
            return Some(self.open(p, 2, MEMBERS, ARROW));
        }
        if let Some(r) = self.operators(p, c) {
            return Some(r);
        }
        if c == b'.' {
            if at(b, p + 1) == b'.' && at(b, p + 2) != b'.' {
                return Some(self.token(p, 2, ILLEGAL_SYNTAX | C));
            }
            return Some(self.open(p, 1, MEMBERS, DOT));
        }
        if let Some(r) = self.strings(p, c) {
            return Some(r);
        }
        match c {
            b'(' => return Some(self.open(p, 1, PAREN, GROUP_BEGIN)),
            b'[' => return Some(self.open(p, 1, BRACKETS, BRACKETS_BEGIN)),
            b'{' => return Some(self.open(p, 1, BLOCK, BLOCK_BEGIN)),
            b',' => return Some(self.token(p, 1, SEPARATOR)),
            b')' | b'}' => return Some(self.token(p, 1, STRAY_BRACKET)),
            _ => {}
        }
        if is_word(b, p) {
            let w = self.word(p);
            if let Kw::Value(leaf) = self.kw(p) {
                return Some(self.token(p, w.len(), leaf));
            }
        }
        None
    }

    /// C's `operators`.
    pub(super) fn operators(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        let (n, leaf) = operator_at(c, at(b, p + 1), at(b, p + 2))?;
        (p + n <= b.len()).then(|| self.token(p, n, leaf | C))
    }

    /// `late-expressions-before-function-call`: `operator`, `modifiers-parens`, `modifiers`,
    /// `types`.
    pub(super) fn late_before_call(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if !is_word(b, p) {
            return None;
        }
        if is_kw(b, p, b"operator") {
            return Some(self.token(p, 8, CONTROL));
        }
        self.modifiers_parens(p)
            .or_else(|| self.modifiers(p))
            .or_else(|| self.types(p))
    }

    /// `modifiers-parens`.
    pub(super) fn modifiers_parens(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let w = self.word(p);
        match w {
            b"alignas" => {
                let s = skip_spaces(b, p + 7);
                (at(b, s) == b'(' && s < b.len()).then(|| {
                    self.put(p, p + 7, MODIFIER);
                    self.put(s, s + 1, GROUP_BEGIN);
                    self.push(s + 1, PAREN);
                    s + 1
                })
            }
            b"__attribute__" => {
                let s = skip_spaces(b, p + 13);
                starts(b, s, b"((").then(|| {
                    self.put(p, p + 13, MODIFIER);
                    self.push(s, ATTRIBUTE);
                    self.token(s, 2, GROUP_BEGIN)
                })
            }
            b"__declspec" if at(b, p + 10) == b'(' && p + 10 < b.len() => {
                self.put(p, p + 10, MODIFIER);
                self.put(p + 10, p + 11, GROUP_BEGIN);
                self.push(p + 11, DECLSPEC);
                Some(p + 11)
            }
            _ => None,
        }
    }

    /// `modifiers`: C++'s, then C's.
    pub(super) fn modifiers(&mut self, p: usize) -> Option<usize> {
        let w = self.word(p);
        (self.kw(p) == Kw::Modifier).then(|| self.token(p, w.len(), MODIFIER))
    }

    /// `types`: C++'s, `decltype(`, C's.
    pub(super) fn types(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let w = self.word(p);
        let n = w.len();
        match self.kw(p) {
            Kw::CxxType => Some(self.token(p, n, STORAGE_TYPE)),
            Kw::Decltype => {
                let s = skip_spaces(b, p + 8);
                (at(b, s) == b'(' && s < b.len()).then(|| {
                    self.put(p, p + 8, STORAGE_TYPE);
                    self.put(s, s + 1, GROUP_BEGIN);
                    self.push(s + 1, PAREN);
                    s + 1
                })
            }
            Kw::CType => Some(self.token(p, n, STORAGE_TYPE | C)),
            Kw::Support(leaf) => Some(self.token(p, n, leaf)),
            _ => None,
        }
    }

    /// `function-call`: `(?=PATH\s*(\(|\{))`.
    pub(super) fn function_call(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if !path_may_end(b, p, |c| c == b'(' || c == b'{') {
            return None;
        }
        path_then(b, p, |e| {
            let s = skip_spaces(b, e);
            s < b.len() && matches!(b[s], b'(' | b'{')
        })
        .then(|| self.enter(p, &[FC]))
    }

    /// `statements`: the `#if` rules, C's `label`, expressions.
    pub(super) fn statements(&mut self, p: usize, c: u8) -> Option<usize> {
        if p == 0 {
            // at a space, only the rules (a directive), `label` and the workarounds (a name
            // first) can start: none without a `#` or a name's first byte past the indentation
            if is_space(c)
                && self.dir.is_none()
                && !at(self.b, self.lead).is_ascii_alphabetic()
                && !matches!(at(self.b, self.lead), b'_' | 0x80..)
            {
                return None;
            }
            if let Some(r) = self.rules(0, Rule::Statements) {
                return Some(r);
            }
            if let Some(r) = self.label() {
                return Some(r);
            }
        }
        self.exprs(p, c, true, true)
    }

    /// C's `label`: `^\s*((?!default)\bID\b)(:)(?!:)`.
    pub(super) fn label(&mut self) -> Option<usize> {
        let b = self.b;
        let q = self.lead;
        if starts(b, q, b"default") {
            return None;
        }
        // a name's ASCII run ending on another ASCII byte than `:` is no label's
        let a = super::text::ascii_word_end(b, q);
        if a < b.len() && b[a] < 0x80 && b[a] != b':' {
            return None;
        }
        let e = self.id_at(q)?;
        if at(b, e) != b':' || at(b, e + 1) == b':' || e >= b.len() {
            return None;
        }
        self.put(q, e, LABEL | C);
        Some(self.token(e, 1, SEPARATOR | C))
    }

    /// `global`.
    pub(super) fn global(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if is_kw(b, p, b"template") {
            return Some(self.enter(p, &[G_TEMPLATE]));
        }
        if let Some(r) = self.using_namespace(p) {
            return Some(r);
        }
        if is_kw(b, p, b"namespace") {
            let s = skip_spaces(b, p + 9);
            if s > p + 9 && !(s < b.len() && matches!(b[s], b';' | b',')) {
                self.put(p, p + 9, CONTROL);
                self.push(s, NS_NAME);
                return Some(s);
            }
            if p + 9 == b.len() {
                // `\s+` takes the line's `\n`, and the lookahead after holds at its end
                self.put(p, p + 9, CONTROL);
                self.push(p + 9, NS_NAME);
                self.halt = true;
                return Some(p + 9);
            }
        }
        if is_word(b, p) {
            let w = self.word(p);
            if self.kw(p) == Kw::Cast {
                let e = skip_spaces(b, p + w.len());
                self.put(p, e, CAST_KW);
                self.push(e, CAST);
                return Some(e);
            }
        }
        if (is_word(b, p) || starts(b, p, b"::"))
            && path_then(b, p, |e| {
                let s = skip_spaces(b, e);
                at(b, s) == b'<' && s < b.len()
            })
        {
            return Some(self.enter(p, &[G_MODIFIER]));
        }
        if c == b'/' && at(b, p + 1) == b'*' {
            self.push(p, G_COMMENT_AFTER);
            return Some(self.open(p, 2, C_COMMENT, COMMENT_MARK | C));
        }
        if let Some(r) = self.early_expressions(p, c) {
            return Some(r);
        }
        if p == 0 {
            let q = self.lead;
            if is_kw(b, q, b"extern") {
                let s = skip_spaces(b, q + 6);
                if s > q + 6 && starts(b, s, b"\"C") {
                    let t = if starts(b, s + 2, b"++") {
                        s + 4
                    } else {
                        s + 2
                    };
                    if at(b, t) == b'"' && t < b.len() {
                        // the pattern's scope: its whole match, the indentation too
                        self.put(0, q + 6, MODIFIER);
                        self.push(q + 6, G_EXTERN);
                        return Some(q + 6);
                    }
                }
            }
            if is_word(b, q) {
                // `^\s*` took nothing without indentation: a push that matched nothing
                if q == 0 {
                    return Some(self.enter(0, &[G_MODIFIER]));
                }
                self.push(q, G_MODIFIER);
                return Some(q);
            }
        }
        self.late(p, c)
    }

    /// `early-expressions`.
    fn early_expressions(&mut self, p: usize, c: u8) -> Option<usize> {
        self.early_before(p, c)
            .or_else(|| self.generic_type(p))
            .or_else(|| self.early_after(p, c))
    }

    /// `late-expressions`.
    fn late(&mut self, p: usize, c: u8) -> Option<usize> {
        self.late_before_call(p)
            .or_else(|| self.function_call(p))
            .or_else(|| self.identifiers(p))
            .or_else(|| (c == b';' && p < self.b.len()).then(|| self.token(p, 1, TERMINATOR)))
    }

    /// `global-modifier`.
    fn global_modifier(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if let Some(r) = self
            .comments(p, c)
            .or_else(|| self.modifiers_parens(p))
            .or_else(|| self.modifiers(p))
        {
            return Some(r);
        }
        if is_word(b, p) || starts(b, p, b"::") {
            // `PATH\s*(?:GEN)?\s*::\s*` then what each lookahead wants
            let after = |want: &dyn Fn(usize) -> bool, spaced: bool| {
                path_then(b, p, |e| {
                    let mut ends = [Some(skip_spaces(b, e)), None];
                    if let Some(g) = spaces_generic(b, e) {
                        ends = [Some(g), Some(skip_spaces(b, e))];
                    }
                    ends.into_iter().flatten().any(|g| {
                        let s = if spaced { skip_spaces(b, g) } else { g };
                        starts(b, s, b"::") && want(skip_spaces(b, s + 2))
                    })
                })
            };
            if after(&|q| id(b, q).is_some_and(|e| paren_after(b, e)), true) {
                return Some(self.shift(p, &[G_MOD_CTOR]));
            }
            if after(
                &|q| at(b, q) == b'~' && id_loose(b, q + 1).is_some_and(|e| paren_or_eol(b, e)),
                true,
            ) {
                return Some(self.shift(p, &[G_MOD_DTOR]));
            }
            if after(&|q| q >= b.len(), false) {
                return Some(self.shift(p, &[G_MOD_FN]));
            }
        }
        if matches!(c, b'L' | b'u' | b'U' | b'R')
            && let Some(r) = self.raw_string(p)
        {
            return Some(r);
        }
        self.any_set(p, &[G_TYPE])
    }

    /// `#anon_global-modifier_0`, `_1`, `_2`.
    fn modifier_name(&mut self, top: u8, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if let Some(r) = self.identifier_path_generic(p, c) {
            return Some(r);
        }
        let (s, q) = self.colons(p);
        match top {
            G_MOD_CTOR => {
                if let Some(e) = id(b, q) {
                    self.put(p, s, DOUBLE_COLON);
                    return Some(self.token(q, e - q, CTOR));
                }
            }
            G_MOD_DTOR => {
                if at(b, q) == b'~'
                    && let Some(e) = id_loose(b, q + 1)
                {
                    self.put(p, s, DOUBLE_COLON);
                    return Some(self.token(q, e - q, DTOR));
                }
            }
            _ => {
                if s > p && eol_after(b, s) {
                    return Some(self.token(p, 2, DOUBLE_COLON));
                }
                let t = if at(b, q) == b'~' { q + 1 } else { q };
                let e = if t > q { id_loose(b, t) } else { id(b, t) };
                if let Some(e) = e
                    && paren_after(b, e)
                {
                    self.put(p, s, DOUBLE_COLON);
                    return Some(self.token(q, e - q, ENTITY_FUNCTION));
                }
            }
        }
        punct_at(b, p).then(|| self.shift(p, &[FN_PARAMS]))
    }

    /// `global-type` (or `data-structures-type`, which differs in what follows).
    fn global_type(&mut self, p: usize, c: u8, ds: bool) -> Option<usize> {
        let b = self.b;
        let maybe = if ds { DS_MAYBE_METHOD } else { G_MAYBE_FN };
        if let Some(r) = self.comments(p, c) {
            return Some(r);
        }
        if (c == b'*' || c == b'&') && p < b.len() {
            return Some(self.token(p, 1, OPERATOR));
        }
        if ds && is_kw(b, p, b"operator") {
            let s = skip_spaces(b, p + 8);
            if s > p + 8
                && let Some(e) = id(b, s)
                && paren_or_eol(b, e)
            {
                self.put(p, p + 8, CONTROL);
                self.put(s, e, ENTITY_FUNCTION);
                self.set(e, &[METHOD_PARAMS]);
                return Some(e);
            }
        }
        let w = self.word(p);
        if !w.is_empty() && is_stop_word(w, true) && self.can_pop(p) {
            return Some(self.leave(p));
        }
        if space_at(b, p) {
            return Some(self.shift(p, &[maybe]));
        }
        if let Some((k, e)) = self.struct_before_name(p) {
            let struct_ctx = if ds { DS_TYPE_STRUCT } else { G_TYPE_STRUCT };
            self.put(p, k, STORAGE_TYPE);
            self.set(e, &[struct_ctx]);
            return Some(e);
        }
        if ds_keyword_word(b, p).is_some() {
            return Some(self.shift(p, &[DS]));
        }
        if !w.is_empty() && self.kw(p) == Kw::Cast {
            let s = skip_spaces(b, p + w.len());
            if at(b, s) == b'<' && s < b.len() && self.can_pop(p) {
                return Some(self.leave(p));
            }
        }
        if c == b'<' && matches!(at(b, p + 1), b'<' | b'=') && self.can_pop(p) {
            return Some(self.leave(p));
        }
        if let Some(r) = self.angle_brackets(p, c).or_else(|| self.types(p)) {
            return Some(r);
        }
        if !ds {
            if let Some(e) = id(b, p) {
                let s = skip_spaces(b, e);
                if at(b, s) == b'(' && s < b.len() {
                    let close = b[s + 1..].iter().position(|&x| x == b')');
                    if close.is_some_and(|k| k > 0) {
                        self.put(p, e, FUNCTION);
                        self.put(s, s + 1, GROUP_BEGIN);
                        self.push(s + 1, PAREN);
                        return Some(s + 1);
                    }
                }
            }
            if (is_word(b, p) || starts(b, p, b"::")) && path_then(b, p, |e| paren_after(b, e)) {
                return Some(self.shift(p, &[G_TYPE_CALL]));
            }
        }
        if !w.is_empty()
            && let Kw::Value(leaf) = self.kw(p)
        {
            return Some(self.token(p, w.len(), leaf));
        }
        if let Some(r) = self.identifiers(p) {
            return Some(r);
        }
        if ds && (c == b'&' || c == b'*') {
            return Some(self.shift(p, &[maybe]));
        }
        (non_word_at(b, p) && self.can_pop(p)).then(|| self.leave(p))
    }

    /// `(struct|union|enum\s+class|enum\s+struct|enum|class)\s+(?=(?![\p{Lu}\p{Nd}_]+\b|
    /// __declspec|struct|union|enum\s+class|enum\s+struct|enum|class)PATH(\s+\bID\b\s*\(|
    /// \s*[*&]))` at `p`: where the keyword ends, and the spaces after it.
    fn struct_before_name(&self, p: usize) -> Option<(usize, usize)> {
        let b = self.b;
        for e in ds_keyword(b, p).into_iter().flatten() {
            let s = skip_spaces(b, e);
            if s == e {
                continue;
            }
            let caps = run_of(b, s, is_upper_rest);
            if (caps > s && !is_word(b, caps))
                || starts(b, s, b"__declspec")
                || ds_keyword(b, s)[0].is_some()
            {
                continue;
            }
            let ok = path_then(b, s, |e| {
                let t = skip_spaces(b, e);
                if t > e
                    && let Some(f) = id(b, t)
                    && paren_after(b, f)
                {
                    return true;
                }
                t < b.len() && matches!(b[t], b'*' | b'&')
            });
            if ok {
                return Some((e, s));
            }
        }
        None
    }

    /// `global-maybe-function` (or `data-structures-maybe-method`).
    fn maybe_function(&mut self, p: usize, c: u8, ds: bool) -> Option<usize> {
        let b = self.b;
        let (params, fn_id, fn_generic) = if ds {
            (METHOD_PARAMS, DS_FN_ID, DS_FN_ID_GENERIC)
        } else {
            (FN_PARAMS, G_FN_ID, G_FN_ID_GENERIC)
        };
        if let Some(r) = self.comments(p, c) {
            return Some(r);
        }
        if (c == b'*' || c == b'&') && p < b.len() {
            return Some(self.token(p, 1, OPERATOR));
        }
        let w = self.word(p);
        if !w.is_empty() && is_stop_word(w, false) && self.can_pop(p) {
            return Some(self.leave(p));
        }
        if matches!(
            w,
            b"const" | b"constexpr" | b"mutable" | b"typename" | b"volatile"
        ) {
            return Some(self.token(p, w.len(), MODIFIER));
        }
        if c == b'<' && matches!(at(b, p + 1), b'<' | b'=') && self.can_pop(p) {
            return Some(self.leave(p));
        }
        if let Some(r) = self
            .angle_brackets(p, c)
            .or_else(|| self.types(p))
            .or_else(|| self.modifiers_parens(p))
            .or_else(|| self.modifiers(p))
        {
            return Some(r);
        }
        if ds {
            if let Some(e) = operator(b, p, |e| paren_or_eol(b, e).then_some(e)) {
                self.put(p, e, ENTITY_FUNCTION);
                self.set(e, &[METHOD_PARAMS]);
                return Some(e);
            }
        } else {
            let caps = run_of(b, p, is_upper_rest);
            if caps > p && eol_after(b, caps) {
                return Some(skip_spaces(b, caps));
            }
            if self.operator_ahead(p) {
                return Some(self.shift(p, &[G_OPERATOR]));
            }
        }
        if !(is_word(b, p) || starts(b, p, b"::")) {
            return self.any_pop(p);
        }
        // `(?=PATH([ \t]+|[*&])(?!\s*(<|::|\(|$)))`, `$` only in global's
        let declares = path_then(b, p, |e| {
            let t = run_of(b, e, |b, i| matches!(b[i], b' ' | b'\t'));
            let after = if t > e {
                t
            } else if matches!(at(b, e), b'*' | b'&') && e < b.len() {
                e + 1
            } else {
                return false;
            };
            let s = skip_spaces(b, after);
            if s >= b.len() {
                return ds;
            }
            !(b[s] == b'<' || b[s] == b'(' || starts(b, s, b"::"))
        });
        if declares {
            return Some(self.enter(p, &[IDENT_POP]));
        }
        let call_end = |e: usize| {
            if ds {
                paren_after(b, e)
            } else {
                paren_or_eol(b, e)
            }
        };
        // `PATH GEN PATH?` then the call
        let generic_call = path_then(b, p, |e| {
            let Some(g) = (at(b, e) == b'<').then(|| generic(b, e)).flatten() else {
                return false;
            };
            call_end(g) || (!ds && path_then(b, g, call_end))
        });
        if generic_call {
            return Some(self.shift(p, &[params, fn_generic]));
        }
        if path_then(b, p, call_end) {
            return Some(self.shift(p, &[params, fn_id]));
        }
        if path_then(b, p, |e| {
            let s = skip_spaces(b, e);
            starts(b, s, b"::") && eol_after(b, s + 2)
        }) {
            return Some(self.shift(p, &[params, fn_id]));
        }
        self.any_pop(p)
    }

    /// `(?=((PATH\s*(?:GEN)?::\s*)?\boperator\s*OPS\s*(\(|$)))` at `p`.
    fn operator_ahead(&self, p: usize) -> bool {
        let b = self.b;
        let op_at = |q: usize| operator(b, q, |e| paren_or_eol(b, e).then_some(())).is_some();
        if (is_word(b, p) || starts(b, p, b"::"))
            && path_then(b, p, |e| {
                let s = skip_spaces(b, e);
                let ends = [spaces_generic(b, e), Some(s)];
                ends.into_iter()
                    .flatten()
                    .any(|g| starts(b, g, b"::") && op_at(skip_spaces(b, g + 2)))
            })
        {
            return true;
        }
        op_at(p)
    }

    /// `function-definition-continue` (or the method's).
    fn fn_continue(&mut self, top: u8, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if let Some(r) = self.comments(p, c) {
            return Some(r);
        }
        match c {
            b';' if p < b.len() && self.can_pop(p) => return Some(self.leave(p)),
            b'-' if at(b, p + 1) == b'>' => {
                return Some(self.then(p, 2, &[FN_TRAILING], SEPARATOR));
            }
            _ => {}
        }
        if let Some(r) = self.function_specifiers(p) {
            return Some(r);
        }
        match c {
            b'=' => return Some(self.token(p, 1, ASSIGN)),
            b'&' => return Some(self.token(p, 1, OPERATOR)),
            b'0' if !word_before(b, p) && !is_word(b, p + 1) => {
                return Some(self.token(p, 1, DEC));
            }
            _ => {}
        }
        let w = self.word(p);
        if matches!(w, b"default" | b"delete") {
            return Some(self.token(p, w.len(), MODIFIER));
        }
        if top == METHOD_CONTINUE && c == b':' && p < b.len() {
            return Some(self.shift(p, &[MC_COLON]));
        }
        if c == b'{' {
            return Some(self.then(p, 1, &[FN_BLOCK], BLOCK_BEGIN));
        }
        self.any_pop(p)
    }

    /// `function-specifiers`.
    fn function_specifiers(&mut self, p: usize) -> Option<usize> {
        let w = self.word(p);
        matches!(w, b"const" | b"final" | b"noexcept" | b"override")
            .then(|| self.token(p, w.len(), MODIFIER))
    }

    /// `function-trailing-return-type`.
    fn trailing_return_type(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if c == b'<' && matches!(at(b, p + 1), b'<' | b'=') && self.can_pop(p) {
            return Some(self.leave(p));
        }
        if let Some(r) = self
            .angle_brackets(p, c)
            .or_else(|| self.types(p))
            .or_else(|| self.modifiers_parens(p))
            .or_else(|| self.modifiers(p))
            .or_else(|| self.identifiers(p))
        {
            return Some(r);
        }
        match c {
            b'*' | b'&' if p < b.len() => Some(self.token(p, 1, OPERATOR)),
            b'(' => Some(self.open(p, 1, TRAILING_PARENS, GROUP_BEGIN)),
            _ => self.any_pop(p),
        }
    }

    /// `(?=^\s*#\s*(elif|else|endif)\b)` at `p`.
    pub(super) fn before_branch(&self, p: usize) -> bool {
        p == 0 && self.directive_is(&[b"elif", b"else", b"endif"])
    }

    /// Whether the line is `^\s*#\s*` and one of `words` then `\b`.
    pub(super) fn directive_is(&self, words: &[&[u8]]) -> bool {
        let b = self.b;
        let q = self.lead;
        if at(b, q) != b'#' || q >= b.len() {
            return false;
        }
        let s = skip_spaces(b, q + 1);
        words
            .iter()
            .any(|w| starts(b, s, w) && !is_word(b, s + w.len()))
    }

    /// `(?=(struct|union|enum\s+class|enum\s+struct|enum|class)([^(;]+(?m:$)|.*\{))` at `p`.
    pub(super) fn ds_ahead(&self, p: usize) -> bool {
        let b = self.b;
        ds_keyword(b, p).into_iter().flatten().any(|e| {
            let rest = &b[e.min(b.len())..];
            !rest.iter().any(|&x| x == b'(' || x == b';') || rest.contains(&b'{')
        })
    }

    /// `data-structures`.
    fn data_structures(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if is_kw(b, p, b"class") {
            return Some(self.then(p, 5, &[CLASS_DEF], STORAGE_TYPE));
        }
        if let Some(e) = ds_keyword_word(b, p) {
            // `(?=\s+PATH\s+PATH\s*[=;\[])` after the keyword, for any of its alternatives
            let declares = ds_keyword(b, p).into_iter().flatten().find(|&k| {
                let s = skip_spaces(b, k);
                s > k
                    && path_then(b, s, |e1| {
                        let t = skip_spaces(b, e1);
                        t > e1
                            && path_then(b, t, |e2| {
                                let u = skip_spaces(b, e2);
                                u < b.len() && matches!(b[u], b'=' | b';' | b'[')
                            })
                    })
            });
            if let Some(k) = declares {
                return Some(self.token(p, k - p, STORAGE_TYPE));
            }
            if is_kw(b, p, b"struct") {
                return Some(self.then(p, 6, &[STRUCT_DEF], STORAGE_TYPE));
            }
            if starts(b, p, b"enum") {
                return Some(self.then(p, e - p, &[ENUM_DEF], STORAGE_TYPE));
            }
            if is_kw(b, p, b"union") {
                return Some(self.then(p, 5, &[UNION_DEF], STORAGE_TYPE));
            }
        }
        let _ = c;
        self.any_pop(p)
    }

    /// `data-structures-definition-common-begin`.
    fn common_begin(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if let Some(r) = self.comments(p, c) {
            return Some(r);
        }
        if ds_keyword_word(b, p).is_some() && self.can_pop(p) {
            return Some(self.leave(p));
        }
        let w = self.word(p);
        if matches!(
            w,
            b"break"
                | b"case"
                | b"catch"
                | b"continue"
                | b"default"
                | b"do"
                | b"else"
                | b"for"
                | b"goto"
                | b"if"
                | b"_Pragma"
                | b"return"
                | b"switch"
                | b"throw"
                | b"try"
                | b"while"
        ) && self.can_pop(p)
        {
            return Some(self.leave(p));
        }
        self.preprocessor_other(p)
            .or_else(|| self.modifiers_parens(p))
            .or_else(|| self.modifiers(p))
            .or_else(|| self.eat_macro(p))
    }

    /// `preprocessor-workaround-eat-macro-before-identifier`.
    fn eat_macro(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let e = upper_macro(b, p)?;
        let s = skip_spaces(b, e);
        if s == e {
            return None;
        }
        let t = if at(b, s) == b'~' { s + 1 } else { s };
        let ok = if t > s { id_loose(b, t) } else { id(b, t) };
        ok.map(|_| {
            self.fill(e);
            e
        })
    }

    /// A class's, a struct's, a union's or an enum's definition, before its name.
    fn ds_definition(&mut self, top: u8, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        let k = top - CLASS_DEF;
        if let Some(r) = self.common_begin(p, c) {
            return Some(r);
        }
        if let Some(e) = id(b, p)
            && forward_decl_after(b, e)
        {
            let leaf = [CLASS_FWD, STRUCT_FWD, UNION_FWD, ENUM_FWD][usize::from(k)];
            return Some(self.then(p, e - p, &[CLASS_AFTER + k], leaf));
        }
        if (is_word(b, p) || starts(b, p, b"::")) && path(b, p, |_, e| Some(e)).is_some() {
            return Some(self.shift(p, &[CLASS_NAME + k]));
        }
        let opens = if top == UNION_DEF {
            c == b'{'
        } else {
            c == b':' || c == b'{'
        };
        if opens && p < b.len() {
            return Some(self.shift(p, &[CLASS_AFTER + k]));
        }
        (c == b';' && p < b.len() && self.can_pop(p)).then(|| self.leave(p))
    }

    /// `data-structures-body`.
    pub(super) fn data_structures_body(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if p == 0
            && let Some(r) = self
                .rules(0, Rule::Data)
                .or_else(|| self.practical_workarounds())
        {
            return Some(r);
        }
        if is_kw(b, p, b"template") {
            return Some(self.enter(p, &[DS_TEMPLATE]));
        }
        if let Some(r) = self.using_namespace(p) {
            return Some(r);
        }
        if is_kw(b, p, b"typedef") {
            self.put(p, p + 7, STORAGE_TYPE);
            self.push(p + 7, TYPEDEF);
            return Some(p + 7);
        }
        if is_kw(b, p, b"using") {
            // using-alias
            let s = skip_spaces(b, p + 5);
            if s > p + 5 {
                if is_kw(b, s, b"typename") {
                    return Some(self.token(p, 5, CONTROL));
                }
                if let Some(e) = id(b, s) {
                    let d = skip_spaces(b, e);
                    if !(at(b, d) == b'<' || starts(b, d, b"::")) || d >= b.len() {
                        self.put(p, p + 5, CONTROL);
                        return Some(self.token(s, e - s, USING_NAME));
                    }
                }
            }
        }
        let w = self.word(p);
        if matches!(w, b"private" | b"protected" | b"public") {
            let s = skip_spaces(b, p + w.len());
            if at(b, s) == b':' && at(b, s + 1) != b':' && s < b.len() {
                self.put(p, p + w.len(), MODIFIER);
                return Some(self.token(s, 1, CLASS_SECTION));
            }
        }
        if p == 0 {
            let q = self.lead;
            let t = if at(b, q) == b'~' { q + 1 } else { q };
            if is_word(b, t) || starts(b, q, b"::") {
                // `^\s*` took nothing without indentation: a push that matched nothing
                if q == 0 {
                    return Some(self.enter(0, &[DS_MODIFIER]));
                }
                self.push(q, DS_MODIFIER);
                return Some(q);
            }
        }
        self.exprs(p, c, false, true)
    }

    /// `data-structures-modifier`.
    fn ds_modifier(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if is_kw(b, p, b"friend") {
            self.put(p, p + 6, MODIFIER);
            self.push(p + 6, FRIEND);
            return Some(p + 6);
        }
        if let Some(r) = self
            .comments(p, c)
            .or_else(|| self.modifiers_parens(p))
            .or_else(|| self.modifiers(p))
        {
            return Some(r);
        }
        if is_kw(b, p, b"static_assert") && paren_after(b, p + 13) {
            self.put(p, p + 13, WORD_OP);
            self.push(p + 13, STATIC_ASSERT);
            return Some(p + 13);
        }
        // a destructor: `(?:\bID\b\s*(::)\s*)?~\bID\b(?=\s*(\(|$))`
        {
            let mut t = p;
            let mut colons = None;
            if let Some(e) = id(b, p) {
                let d = skip_spaces(b, e);
                if starts(b, d, b"::") {
                    colons = Some(d);
                    t = skip_spaces(b, d + 2);
                }
            }
            for (t, colons) in [(t, colons), (p, None)] {
                if at(b, t) == b'~'
                    && let Some(e) = id_loose(b, t + 1)
                    && paren_or_eol(b, e)
                {
                    if let Some(d) = colons {
                        self.put(p, d, DTOR);
                        self.put(d, d + 2, DTOR_ACCESSOR);
                        self.put(d + 2, e, DTOR);
                    } else {
                        self.put(p, e, DTOR);
                    }
                    self.set(e, &[METHOD_PARAMS]);
                    return Some(e);
                }
                if colons.is_none() {
                    break;
                }
            }
        }
        if let Some(e) = id(b, p) {
            let s = skip_spaces(b, e);
            if at(b, s) == b'(' && s < b.len() {
                let t = skip_spaces(b, s + 1);
                if !starts(b, t, b"void")
                    && let Some(f) = id(b, t)
                {
                    let u = skip_spaces(b, f);
                    if u < b.len() && matches!(b[u], b')' | b',') {
                        self.put(p, e, FUNCTION);
                        self.put(s, s + 1, GROUP_BEGIN);
                        self.push(s + 1, PAREN);
                        return Some(s + 1);
                    }
                }
            }
        }
        if let Some(r) = self.eat_macro(p) {
            return Some(r);
        }
        if let Some(e) = id(b, p)
            && ds_keyword(b, p)[0].is_none()
            && !starts(b, p, b"template")
            && paren_after(b, e)
        {
            self.put(p, e, CTOR);
            self.set(e, &[METHOD_PARAMS]);
            return Some(e);
        }
        if let Some(e) = id(b, p) {
            let d = skip_spaces(b, e);
            if starts(b, d, b"::") {
                let s = skip_spaces(b, d + 2);
                if let Some(f) = id(b, s)
                    && paren_after(b, f)
                {
                    self.put(p, d, CTOR);
                    self.put(d, d + 2, CTOR_ACCESSOR);
                    self.put(d + 2, f, CTOR);
                    self.push(f, METHOD_PARAMS);
                    return Some(f);
                }
            }
        }
        self.any_set(p, &[DS_TYPE])
    }

    /// `members-after-accessor-junction`.
    fn members(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if let Some(r) = self.comments(p, c) {
            return Some(r);
        }
        if is_kw(b, p, b"template") {
            return Some(self.then(p, 8, &[MEMBERS_TEMPLATE], TEMPLATE_KW));
        }
        if let Some((e, s)) = operator(b, p, |e| {
            let s = skip_spaces(b, e);
            (at(b, s) == b'(' && s < b.len()).then_some((e, s))
        }) {
            self.put(p, e, MEMBER_FUNCTION);
            return Some(self.then(s, 1, &[PAREN], GROUP_BEGIN));
        }
        let t = if c == b'~' { p + 1 } else { p };
        let e = if t > p { id_loose(b, t) } else { id(b, t) };
        if let Some(e) = e {
            let s = skip_spaces(b, e);
            if at(b, s) == b'(' && s < b.len() {
                self.put(p, e, MEMBER_FUNCTION);
                return Some(self.then(s, 1, &[PAREN], GROUP_BEGIN));
            }
            if at(b, s) == b'<' && generic(b, s).is_some() {
                self.put(p, e, MEMBER_FUNCTION);
                self.set(s, &[MEMBERS_GENERIC]);
                return Some(s);
            }
        }
        // `id` at `p` is the one above, or none at a `~`
        if t == p
            && let Some(e) = e
        {
            let s = skip_spaces(b, e);
            if starts(b, s, b"::") {
                self.put(p, e, BASE_CLASS);
                return Some(self.then(s, 2, &[MEMBERS], DOUBLE_COLON));
            }
            return Some(self.close(p, e - p, MEMBER_VAR));
        }
        None
    }

    /// C's `preprocessor-line-continuation`, in C++.
    pub(super) fn line_continuation(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if c != b'\\' || p >= b.len() {
            return None;
        }
        if p + 1 == b.len() {
            self.put(p, p + 1, CONT | C);
            self.halt = true;
            return Some(b.len());
        }
        let s = skip_spaces(b, p + 1);
        (s > p + 1 && s == b.len()).then(|| {
            self.put(p + 1, s, SPACE_AFTER_CONT | C);
            s
        })
    }

    /// `^\s*(#\s*ifdef)\s*__cplusplus\s*` on the line: where it ends.
    fn ifdef_cplusplus(&self) -> Option<usize> {
        let b = self.b;
        let q = self.lead;
        if at(b, q) != b'#' || q >= b.len() {
            return None;
        }
        let s = skip_spaces(b, q + 1);
        if !starts(b, s, b"ifdef") {
            return None;
        }
        let t = skip_spaces(b, s + 5);
        starts(b, t, b"__cplusplus").then(|| skip_spaces(b, t + 11))
    }
}

/// `__declspec`'s constants.
fn is_declspec_constant(w: &[u8]) -> bool {
    matches!(
        w,
        b"appdomain"
            | b"deprecated"
            | b"dllimport"
            | b"dllexport"
            | b"jintrinsic"
            | b"naked"
            | b"noalias"
            | b"noinline"
            | b"noreturn"
            | b"nothrow"
            | b"novtable"
            | b"process"
            | b"restrict"
            | b"safebuffers"
            | b"selectany"
            | b"thread"
    )
}

/// `^// =(\s*.*?)\s*=\s*(?m:$)`: the line's last byte past its spaces is a `=` after the
/// first four.
fn line_banner(b: &[u8]) -> bool {
    let end = crate::trim_end(b, 0, b.len());
    end > 4 && b[end - 1] == b'='
}

/// `(?=[\p{L}\p{N}_'.]+?[pP])` at `q` (or `[\p{L}\p{N}.]` for C's): a `p` after one byte of
/// the class at least, all of them in it.
pub(super) fn hex_float_ahead(b: &[u8], q: usize, cxx: bool) -> bool {
    let mut i = q;
    while i < b.len() {
        let c = b[i];
        if i > q && matches!(c, b'p' | b'P') {
            return true;
        }
        let class = if cxx {
            is_ident(b, i) || c == b'.' || c == b'\''
        } else {
            (is_ident(b, i) && c != b'_') || c == b'.'
        };
        if !class {
            return false;
        }
        i = next(b, i);
    }
    false
}

#[allow(dead_code)]
fn unused(b: &[u8], p: usize) -> bool {
    boundary(b, p) || prev(b, p + 1) == p || operator_name(b, p, |_| Some(())).is_some()
}
