//! C's contexts: its comments and strings (C++'s too), and its expression grammar, which C++
//! reaches through an upper-case macro call on a line of its own (`FOO(x, y)`): its
//! statements, blocks, data structures and function definitions behind them.

use super::cxx::Rule;
use super::text::{
    at, digits_plain, exponent, find_any, id, ident_end, is_ident, is_kw, is_word, next,
    paren_after, run_of, starts, word_before, word_end,
};
use super::words::{c_value, c99_at, is_c_control, is_c_modifier, is_c_type, support};
use super::*;
use crate::{is_space, skip_spaces};

/// `(?:[lL]{1,2}[uU]?|[uU][lL]{0,2})?\b` at `q`: where it ends, tried in a backtracking
/// engine's order.
fn int_suffix(b: &[u8], q: usize) -> Option<usize> {
    let l = |i: usize| matches!(at(b, i), b'l' | b'L') && i < b.len();
    let u = |i: usize| matches!(at(b, i), b'u' | b'U') && i < b.len();
    let ends = |e: usize| boundary_at(b, e);
    if l(q) {
        for n in [2, 1] {
            if n == 2 && !l(q + 1) {
                continue;
            }
            if u(q + n) && ends(q + n + 1) {
                return Some(q + n + 1);
            }
            if ends(q + n) {
                return Some(q + n);
            }
        }
    }
    if u(q) {
        for n in [2, 1, 0] {
            if (0..n).all(|k| l(q + 1 + k)) && ends(q + 1 + n) {
                return Some(q + 1 + n);
            }
        }
    }
    boundary_at(b, q).then_some(q)
}

fn boundary_at(b: &[u8], i: usize) -> bool {
    super::text::boundary(b, i)
}

impl Scan<'_, '_> {
    /// The patterns of C's context `top` at `p`.
    pub(super) fn c_context(&mut self, top: u8, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        match top {
            C_COMMENT => {
                if c == b'*' && at(b, p + 1) == b'/' {
                    return Some(self.close(p, 2, COMMENT_MARK | C));
                }
                if p == 0 {
                    let q = self.lead;
                    if at(b, q) == b'*' && q < b.len() && at(b, q + 1) != b'/' {
                        return Some(self.token(q, 1, COMMENT_MARK | C));
                    }
                }
                None
            }
            C_LINE_COMMENT => {
                if c == b'\\' && p + 1 == b.len() {
                    self.put(p, p + 1, LINE_CONT | C);
                    self.halt = true;
                    return Some(b.len());
                }
                if p >= b.len() {
                    self.pop(p);
                    self.halt = true;
                    return Some(p);
                }
                None
            }
            C_PP_COMMENT => {
                if c == b'\\' && p + 1 == b.len() {
                    self.put(p, p + 1, BLOCK_CONT | C);
                    self.halt = true;
                    return Some(b.len());
                }
                (c == b'*' && at(b, p + 1) == b'/').then(|| self.close(p, 2, COMMENT_MARK | C))
            }
            C_PP_LINE_COMMENT => {
                if c == b'\\' && p + 1 == b.len() {
                    self.put(p, p + 1, LINE_CONT | C);
                    self.pop(b.len());
                    self.halt = true;
                    return Some(b.len());
                }
                (p >= b.len() && self.can_pop(p)).then(|| self.leave(p))
            }
            C_STRING | C_CHAR => {
                let (end, quote, cont, escape, bad) = if top == C_STRING {
                    (DQ_END, b'"', DQ_CONT, DQ_ESCAPE, DQ_BAD_ESCAPE)
                } else {
                    (SQ_END, b'\'', SQ_CONT, SQ_ESCAPE, SQ_BAD_ESCAPE)
                };
                if c == quote && p < b.len() {
                    return Some(self.close(p, 1, end | C));
                }
                if c == b'\\' && p < b.len() {
                    if p + 1 == b.len() {
                        self.put(p, p + 1, cont | C);
                        self.halt = true;
                        return Some(b.len());
                    }
                    if let Some(e) = escape_end(b, p) {
                        return Some(self.token(p, e - p, escape | C));
                    }
                    let e = next(b, p + 1);
                    return Some(self.token(p, e - p, bad | C));
                }
                if c == b'%'
                    && top == C_STRING
                    && let Some(e) = placeholder(b, p)
                {
                    return Some(self.token(p, e - p, DQ_PLACEHOLDER | C));
                }
                None
            }
            C_MACRO_CALL | C_PAREN => {
                if c == b')' && p < b.len() {
                    return Some(self.close(p, 1, GROUP_END | C));
                }
                self.c_exprs(p, c)
            }
            C_BRACKETS => {
                if c == b']' && p < b.len() {
                    return Some(self.close(p, 1, BRACKETS_END | C));
                }
                self.c_exprs(p, c)
            }
            C_BLOCK => {
                if self.before_branch(p) && self.can_pop(p) {
                    return Some(self.leave(p));
                }
                if c == b'}' && p < b.len() {
                    return Some(self.close(p, 1, BLOCK_END | C));
                }
                self.c_statements(p, c)
            }
            C_CASE => {
                if c == b':' && p < b.len() {
                    return Some(self.close(p, 1, SEPARATOR | C));
                }
                self.c_exprs(p, c)
            }
            C_TYPEDEF => {
                let e = id(b, p).unwrap_or(p);
                let s = skip_spaces(b, e);
                if at(b, s) == b';' && s < b.len() {
                    self.put(p, e, TYPEDEF_NAME | C);
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
                        self.put(p, p + 6, STORAGE_TYPE | C);
                        return Some(e);
                    }
                }
                self.c_exprs(p, c)
            }
            C_FC => {
                if let Some(e) = c99_at(b, p) {
                    return Some(self.token(p, e - p, C99 | C));
                }
                if let Some(e) = id(b, p) {
                    return Some(self.token(p, e - p, FUNCTION | C));
                }
                (c == b'(').then(|| self.then(p, 1, &[C_PAREN], GROUP_BEGIN | C))
            }
            C_ATTRIBUTE => {
                if c == b'(' {
                    return Some(self.open(p, 1, C_PAREN, GROUP_BEGIN | C));
                }
                if let Some(r) = self.c_strings(p, c) {
                    return Some(r);
                }
                (c == b')' && at(b, p + 1) == b')').then(|| self.close(p, 2, GROUP_END | C))
            }
            C_DECLSPEC => {
                if c == b')' && p < b.len() {
                    return Some(self.close(p, 1, GROUP_END | C));
                }
                let w = self.word(p);
                if matches!(
                    w,
                    b"align" | b"allocate" | b"code_seg" | b"deprecated" | b"property" | b"uuid"
                ) {
                    let s = skip_spaces(b, p + w.len());
                    if at(b, s) == b'(' && s < b.len() {
                        self.put(p, p + w.len(), MODIFIER | C);
                        self.put(s, s + 1, GROUP_BEGIN | C);
                        self.push(s + 1, C_DECLSPEC_ARGS);
                        return Some(s + 1);
                    }
                }
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
                .then(|| self.token(p, w.len(), CONSTANT_OTHER | C))
            }
            C_DECLSPEC_ARGS => {
                if c == b')' && p < b.len() {
                    return Some(self.close(p, 1, GROUP_END | C));
                }
                if let Some(r) = self.c_numbers(p, c).or_else(|| self.c_strings(p, c)) {
                    return Some(r);
                }
                let w = self.word(p);
                if matches!(w, b"get" | b"put") {
                    return Some(self.token(p, 3, PARAMETER | C));
                }
                match c {
                    b',' => Some(self.token(p, 1, SEPARATOR | C)),
                    b'=' => Some(self.token(p, 1, ASSIGN | C)),
                    _ => None,
                }
            }
            C_NUM_HEX_FLOAT => {
                if matches!(c, b'p' | b'P')
                    && let Some(e) = exponent(b, p, *b"pP", digits_plain)
                    && super::text::boundary(b, e)
                {
                    return Some(self.close(p, e - p, HEX_FLOAT | C));
                }
                if c == b'.' && p < b.len() {
                    return Some(self.token(p, 1, HEX_FLOAT_DOT | C));
                }
                if p >= b.len() {
                    self.halt = true;
                    return Some(p);
                }
                (!c.is_ascii_hexdigit()).then(|| {
                    let e = next(b, p);
                    self.token(p, e - p, HEX_FLOAT_BAD | C)
                })
            }
            C_NUM_HEX | C_NUM_OCT | C_NUM_DEC => {
                if top == C_NUM_OCT && matches!(c, b'8' | b'9') {
                    return Some(self.token(p, 1, OCT_BAD | C));
                }
                if let Some(e) = int_suffix(b, p) {
                    let leaf = [HEX_SUFFIX, OCT_SUFFIX, DEC_SUFFIX][usize::from(top - C_NUM_HEX)];
                    return Some(self.close(p, e - p, leaf | C));
                }
                let starts_bad = if top == C_NUM_HEX {
                    matches!(c, b'g'..=b'z' | b'G'..=b'Z' | b'_')
                } else {
                    c.is_ascii_alphabetic() || c == b'_'
                };
                (starts_bad && p < b.len()).then(|| {
                    let e = ident_end(b, p + 1);
                    let leaf = [HEX_BAD_SUFFIX, OCT_BAD_SUFFIX, DEC_BAD_SUFFIX]
                        [usize::from(top - C_NUM_HEX)];
                    self.close(p, e - p, leaf | C)
                })
            }
            C_NEGATED => {
                if c == b'{' && p < b.len() {
                    return Some(self.close(p, 1, BLOCK_BEGIN | C));
                }
                if self.before_branch(p) && self.can_pop(p) {
                    return Some(self.leave(p));
                }
                self.c_statements(p, c)
            }
            C_PP_DEFINE..=C_PP_INC_LT | C_EN_S..=C_PP_FC_ARGS | C_UPPER_IDENT | C_DISABLED => {
                self.pp_context(top, p, c)
            }
            C_FN_PARAMS => {
                if let Some(r) = self.c_comments(p, c) {
                    return Some(r);
                }
                if c == b'(' {
                    return Some(self.then(p, 1, &[C_FN_PARAMS_IN], GROUP_BEGIN | C));
                }
                self.any_pop(p)
            }
            C_FN_PARAMS_IN => {
                if c == b')' {
                    return Some(self.then(p, 1, &[C_FN_CONTINUE], GROUP_END | C));
                }
                if is_kw(b, p, b"void") {
                    return Some(self.token(p, 4, STORAGE_TYPE | C));
                }
                if let Some(e) = id(b, p) {
                    let q = skip_spaces(b, e);
                    if q < b.len() && matches!(b[q], b'[' | b',' | b')') {
                        return Some(self.token(p, e - p, PARAMETER | C));
                    }
                }
                self.c_exprs(p, c).or_else(|| self.line_continuation(p, c))
            }
            C_FN_CONTINUE => {
                if let Some(r) = self.c_comments(p, c) {
                    return Some(r);
                }
                if c == b';' && p < b.len() && self.can_pop(p) {
                    return Some(self.leave(p));
                }
                let w = self.word(p);
                if matches!(w, b"const" | b"final" | b"noexcept" | b"override") {
                    return Some(self.token(p, w.len(), MODIFIER | C));
                }
                if c == b'{' {
                    return Some(self.then(p, 1, &[C_FN_BLOCK], BLOCK_BEGIN | C));
                }
                self.any_pop(p)
            }
            C_FN_BODY => (c == b'{').then(|| self.then(p, 1, &[C_FN_BLOCK], BLOCK_BEGIN | C)),
            C_FN_BLOCK => {
                if c == b'}' && p < b.len() {
                    return Some(self.close(p, 1, BLOCK_END | C));
                }
                if self.before_branch(p) && self.can_pop(p) {
                    return Some(self.leave(p));
                }
                if c_ds_ahead(b, p) {
                    return Some(self.enter(p, &[C_DS]));
                }
                self.c_statements(p, c)
            }
            C_DS => {
                for (w, def) in [
                    (&b"struct"[..], C_STRUCT_DEF),
                    (b"enum", C_ENUM_DEF),
                    (b"union", C_UNION_DEF),
                ] {
                    if is_kw(b, p, w) {
                        let n = w.len();
                        // `(?=\s+\bID\b\s+\bID\b\s*[=;\[])`: a declaration, the keyword alone
                        let s = skip_spaces(b, p + n);
                        let declares = s > p + n
                            && id(b, s).is_some_and(|e1| {
                                let t = skip_spaces(b, e1);
                                t > e1
                                    && id(b, t).is_some_and(|e2| {
                                        let u = skip_spaces(b, e2);
                                        u < b.len() && matches!(b[u], b'=' | b';' | b'[')
                                    })
                            });
                        if declares {
                            return Some(self.token(p, n, STORAGE_TYPE | C));
                        }
                        return Some(self.then(p, n, &[def], STORAGE_TYPE | C));
                    }
                }
                self.any_pop(p)
            }
            C_STRUCT_DEF | C_ENUM_DEF | C_UNION_DEF => {
                let k = top - C_STRUCT_DEF;
                if let Some(r) = self.c_common_begin(p, c) {
                    return Some(r);
                }
                // `data-structures-definition-common-macro`
                let caps = run_of(b, p, super::text::is_upper_rest);
                if caps > p && !word_before(b, p) && !is_word(b, caps) {
                    let s = skip_spaces(b, caps);
                    if !(s >= b.len() || b[s] == b'{') {
                        self.fill(caps);
                        return Some(caps);
                    }
                }
                if let Some(e) = id(b, p) {
                    let s = skip_spaces(b, e);
                    if at(b, s) == b';' && s < b.len() {
                        let leaf = [STRUCT_FWD, ENUM_FWD, UNION_FWD][usize::from(k)];
                        return Some(self.token(p, e - p, leaf | C));
                    }
                    let leaf = [STRUCT_NAME_LEAF, ENUM_NAME_LEAF, UNION_NAME_LEAF][usize::from(k)];
                    return Some(self.then(p, e - p, &[C_STRUCT_AFTER + k], leaf | C));
                }
                if c == b'{' {
                    return Some(self.then(p, 1, &[C_STRUCT_BODY + k], BLOCK_BEGIN | C));
                }
                (c == b';' && p < b.len() && self.can_pop(p)).then(|| self.leave(p))
            }
            C_STRUCT_AFTER | C_ENUM_AFTER | C_UNION_AFTER => {
                let k = top - C_STRUCT_AFTER;
                if let Some(r) = self.c_common_begin(p, c) {
                    return Some(r);
                }
                if c == b';' && p < b.len() && self.can_pop(p) {
                    return Some(self.leave(p));
                }
                (c == b'{').then(|| self.then(p, 1, &[C_STRUCT_BODY + k], BLOCK_BEGIN | C))
            }
            C_STRUCT_BODY | C_ENUM_BODY | C_UNION_BODY => {
                if c == b'}' && p < b.len() {
                    return Some(self.close(p, 1, BLOCK_END | C));
                }
                self.c_data_structures_body(p, c)
            }
            _ => None,
        }
    }

    /// Where the next pattern of C's context `top` can start, after none did at `p`.
    pub(super) fn c_advance(&self, top: u8, p: usize) -> usize {
        let b = self.b;
        let c = b[p];
        match top {
            C_COMMENT => find_any(b, p + 1, *b"*"),
            C_PP_COMMENT => find_any(b, p + 1, *b"*\\"),
            C_LINE_COMMENT | C_PP_LINE_COMMENT => {
                let i = find_any(b, p + 1, *b"\\");
                if i + 1 >= b.len() {
                    i
                } else {
                    self.c_advance(top, i)
                }
            }
            C_STRING => find_any(b, p + 1, *b"\"\\%"),
            C_CHAR => find_any(b, p + 1, *b"'\\"),
            C_PP_INC_DQ => find_any(b, p + 1, *b"\""),
            C_PP_INC_LT => find_any(b, p + 1, *b">"),
            C_IF_COMMENT | C_ELSE_COMMENT | C_DISABLED => b.len(),
            C_NUM_HEX_FLOAT | C_NUM_HEX | C_NUM_OCT | C_NUM_DEC | C_TYPEDEF | C_UPPER_IDENT => {
                next(b, p)
            }
            _ if is_space(c) => {
                if p == 0 {
                    self.lead
                } else {
                    skip_spaces(b, p)
                }
            }
            _ if is_word(b, p) && !word_before(b, p) => {
                let e = word_end(b, p);
                b[p + 1..e]
                    .iter()
                    .position(|&x| matches!(x, b'L' | b'u' | b'U'))
                    .map_or(e, |k| p + 1 + k)
            }
            _ => next(b, p),
        }
    }

    /// C's `comments`.
    fn c_comments(&mut self, p: usize, c: u8) -> Option<usize> {
        self.comments(p, c)
    }

    /// C's `strings`.
    pub(super) fn c_strings(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        let q = match c {
            b'L' | b'U' => p + 1,
            b'u' if at(b, p + 1) == b'8' => p + 2,
            b'u' => p + 1,
            _ => p,
        };
        let quote = at(b, q);
        if !(quote == b'"' || quote == b'\'') || q >= b.len() {
            return None;
        }
        let (ctx, prefix, begin) = if quote == b'"' {
            (C_STRING, DQ_PREFIX, DQ_BEGIN)
        } else {
            (C_CHAR, SQ_PREFIX, SQ_BEGIN)
        };
        self.push(p, ctx);
        self.put(p, q, prefix | C);
        Some(self.token(q, 1, begin | C))
    }

    /// C's `numbers`.
    pub(super) fn c_numbers(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if !(c.is_ascii_digit() || c == b'.') || p >= b.len() {
            return None;
        }
        let exp = |q: usize| exponent(b, q, *b"eE", digits_plain);
        // `(?: ([fFlL])\b | ([a-zA-Z_][\p{L}\p{N}_]*)? )` at `q`: the suffix, valid or not
        let suffix = |q: usize| -> (usize, bool) {
            if matches!(at(b, q), b'f' | b'F' | b'l' | b'L')
                && q < b.len()
                && super::text::boundary(b, q + 1)
            {
                return (q + 1, true);
            }
            if (at(b, q).is_ascii_alphabetic() || at(b, q) == b'_') && q < b.len() {
                return (ident_end(b, q + 1), false);
            }
            (q, false)
        };
        let at_word = !word_before(b, p);
        if c.is_ascii_digit() && at_word {
            let d = digits_plain(b, p).unwrap_or(p + 1);
            let mut dot = None;
            let mut end = None;
            if at(b, d) == b'.' && d < b.len() {
                let f = d + 1;
                if let Some(g) = digits_plain(b, f) {
                    end = Some(exp(g).unwrap_or(g));
                } else if let Some(g) = exp(f) {
                    end = Some(g);
                } else if f < b.len() && b[f] != b'.' {
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
                let (x, valid) = suffix(e);
                let mut s = p;
                if let Some(t) = dot {
                    self.put(p, t, FLOAT | C);
                    self.put(t, t + 1, FLOAT_DOT | C);
                    s = t + 1;
                }
                self.put(s, e, FLOAT | C);
                self.put(
                    e,
                    x,
                    if valid {
                        FLOAT_SUFFIX
                    } else {
                        FLOAT_BAD_SUFFIX
                    } | C,
                );
                return Some(x);
            }
            if matches!(at(b, d), b'f' | b'F') && d < b.len() && super::text::boundary(b, d + 1) {
                self.put(p, d, FLOAT | C);
                return Some(self.token(d, 1, FLOAT_SUFFIX | C));
            }
        }
        if c == b'.'
            && let Some(g) = digits_plain(b, p + 1)
        {
            let e = exp(g).unwrap_or(g);
            let (x, valid) = suffix(e);
            self.put(p, p + 1, FLOAT_DOT | C);
            self.put(p + 1, e, FLOAT | C);
            self.put(
                e,
                x,
                if valid {
                    FLOAT_SUFFIX
                } else {
                    FLOAT_BAD_SUFFIX
                } | C,
            );
            return Some(x);
        }
        if !c.is_ascii_digit() || !at_word {
            return None;
        }
        let x = at(b, p + 1);
        if c == b'0' && matches!(x, b'x' | b'X') && p + 1 < b.len() {
            if super::cxx::hex_float_ahead(b, p + 2, false) {
                return Some(self.open(p, 2, C_NUM_HEX_FLOAT, HEX_FLOAT_BASE | C));
            }
            return Some(self.open(p, 2, C_NUM_HEX, HEX_BASE | C));
        }
        if c == b'0' && x.is_ascii_digit() && p + 1 < b.len() {
            return Some(self.open(p, 1, C_NUM_OCT, OCT_BASE | C));
        }
        let e = run_of(b, p, |b, i| b[i].is_ascii_digit());
        self.put(p, e, DEC | C);
        self.push(e, C_NUM_DEC);
        Some(e)
    }

    /// C's `statements`: its `#if` rules, `label`, expressions.
    pub(super) fn c_statements(&mut self, p: usize, c: u8) -> Option<usize> {
        if p == 0 {
            if let Some(r) = self.c_rules(0, Rule::Statements) {
                return Some(r);
            }
            if let Some(r) = self.label() {
                return Some(r);
            }
        }
        self.c_exprs(p, c)
    }

    /// C's `data-structures-body`.
    pub(super) fn c_data_structures_body(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if p == 0
            && let Some(r) = self.c_rules(0, Rule::Data)
        {
            return Some(r);
        }
        if starts(b, p, b"struct") || starts(b, p, b"union") || starts(b, p, b"enum") {
            return Some(self.enter(p, &[C_DS]));
        }
        self.c_exprs(p, c)
    }

    /// C's `data-structures-definition-common-begin`.
    fn c_common_begin(&mut self, p: usize, c: u8) -> Option<usize> {
        if let Some(r) = self.c_comments(p, c) {
            return Some(r);
        }
        let w = self.word(p);
        if (matches!(w, b"struct" | b"union" | b"enum") || is_c_control(w)) && self.can_pop(p) {
            return Some(self.leave(p));
        }
        self.c_modifiers_parens(p).or_else(|| self.c_modifiers(p))
    }

    /// C's `expressions`.
    pub(super) fn c_exprs(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if p >= b.len() {
            return None;
        }
        if p == 0
            && let Some(r) = self.pp_expressions(true)
        {
            return Some(r);
        }
        if let Some(r) = self.c_comments(p, c) {
            return Some(r);
        }
        if is_word(b, p) && !c.is_ascii_digit() {
            let w = self.word(p);
            let n = w.len();
            if n > 0 {
                match w {
                    b"case" | b"default" => return Some(self.open(p, n, C_CASE, CONTROL | C)),
                    b"typedef" => {
                        self.put(p, p + 7, STORAGE_TYPE | C);
                        self.push(p + 7, C_TYPEDEF);
                        return Some(p + 7);
                    }
                    b"sizeof" => {
                        let s = skip_spaces(b, p + 6);
                        if at(b, s) == b'(' && s < b.len() {
                            self.put(p, p + 6, WORD_OP | C);
                            self.put(s, s + 1, GROUP_BEGIN | C);
                            self.push(s + 1, C_PAREN);
                            return Some(s + 1);
                        }
                        return Some(self.token(p, 6, WORD_OP | C));
                    }
                    b"break" => return Some(self.token(p, n, BREAK_KW | C)),
                    b"continue" => return Some(self.token(p, n, CONTINUE_KW | C)),
                    b"goto" => return Some(self.token(p, n, GOTO_KW | C)),
                    b"return" => return Some(self.token(p, n, RETURN_KW | C)),
                    _ if is_c_control(w) => return Some(self.token(p, n, CONTROL | C)),
                    _ => {}
                }
            }
        }
        if let Some(r) = self.c_numbers(p, c) {
            return Some(r);
        }
        if let Some(r) = self.operators(p, c) {
            return Some(r);
        }
        if let Some(r) = self.c_strings(p, c) {
            return Some(r);
        }
        match c {
            b'(' => return Some(self.open(p, 1, C_PAREN, GROUP_BEGIN | C)),
            b'[' => return Some(self.open(p, 1, C_BRACKETS, BRACKETS_BEGIN | C)),
            b'{' => return Some(self.open(p, 1, C_BLOCK, BLOCK_BEGIN | C)),
            _ => {}
        }
        if is_word(b, p) {
            let w = self.word(p);
            if let Some(leaf) = c_value(w) {
                return Some(self.token(p, w.len(), leaf));
            }
        }
        if c == b'.' {
            // `access`
            if let Some(e) = id(b, p + 1)
                && !paren_after(b, e)
            {
                self.put(p, p + 1, ACCESSOR | C);
                return Some(self.token(p + 1, e - p - 1, MEMBER | C));
            }
            if at(b, p + 1) == b'.' {
                if at(b, p + 2) != b'.' {
                    return Some(self.token(p, 2, ILLEGAL_SYNTAX | C));
                }
            } else {
                return Some(self.token(p, 1, ACCESSOR | C));
            }
        }
        match c {
            b',' => return Some(self.token(p, 1, SEPARATOR | C)),
            b')' | b'}' => return Some(self.token(p, 1, STRAY_BRACKET | C)),
            _ => {}
        }
        // late
        if is_word(b, p) {
            if let Some(r) = self.c_modifiers_parens(p).or_else(|| self.c_modifiers(p)) {
                return Some(r);
            }
            let w = self.word(p);
            if is_c_type(w) {
                return Some(self.token(p, w.len(), STORAGE_TYPE | C));
            }
            if let Some(leaf) = support(w) {
                return Some(self.token(p, w.len(), leaf));
            }
            if let Some(e) = id(b, p)
                && paren_after(b, e)
            {
                return Some(self.enter(p, &[C_FC]));
            }
        }
        (c == b';').then(|| self.token(p, 1, TERMINATOR | C))
    }

    /// C's `modifiers-parens`.
    fn c_modifiers_parens(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let w = self.word(p);
        match w {
            b"__attribute__" => {
                let s = skip_spaces(b, p + 13);
                starts(b, s, b"((").then(|| {
                    self.put(p, p + 13, MODIFIER | C);
                    self.push(s, C_ATTRIBUTE);
                    self.token(s, 2, GROUP_BEGIN | C)
                })
            }
            b"__declspec" if at(b, p + 10) == b'(' && p + 10 < b.len() => {
                self.put(p, p + 10, MODIFIER | C);
                self.put(p + 10, p + 11, GROUP_BEGIN | C);
                self.push(p + 11, C_DECLSPEC);
                Some(p + 11)
            }
            _ => None,
        }
    }

    /// C's `modifiers`.
    fn c_modifiers(&mut self, p: usize) -> Option<usize> {
        let w = self.word(p);
        is_c_modifier(w).then(|| self.token(p, w.len(), MODIFIER | C))
    }
}

/// `\\(?:\\|[abefnrtv\'"?]|[0-3][0-9]{0,2}|[4-7][0-9]?|x[a-fA-F0-9]+|u[a-fA-F0-9]{4}|
/// U[a-fA-F0-9]{8})` at `p`: where it ends.
fn escape_end(b: &[u8], p: usize) -> Option<usize> {
    let c = at(b, p + 1);
    let digits = |from: usize, max: usize| {
        let mut e = from;
        while e < b.len() && e - from < max && b[e].is_ascii_digit() {
            e += 1;
        }
        e
    };
    let hex = |from: usize| {
        let mut e = from;
        while e < b.len() && b[e].is_ascii_hexdigit() {
            e += 1;
        }
        e
    };
    let fixed = |from: usize, n: usize| {
        (b.len() >= from + n && b[from..from + n].iter().all(u8::is_ascii_hexdigit))
            .then_some(from + n)
    };
    if p + 1 >= b.len() {
        return None;
    }
    match c {
        b'\\' | b'a' | b'b' | b'e' | b'f' | b'n' | b'r' | b't' | b'v' | b'\'' | b'"' | b'?' => {
            Some(p + 2)
        }
        b'0'..=b'3' => Some(digits(p + 2, 2)),
        b'4'..=b'7' => Some(digits(p + 2, 1)),
        b'x' => {
            let e = hex(p + 2);
            (e > p + 2).then_some(e)
        }
        b'u' => fixed(p + 2, 4),
        b'U' => fixed(p + 2, 8),
        _ => None,
    }
}

/// C's `string_placeholder` at `p` (a `%`): where it ends, by a backtracking search over its
/// parts in the regex's order.
fn placeholder(b: &[u8], p: usize) -> Option<usize> {
    let byte = |i: usize| b.get(i).copied();
    // `(-?\d+)` at `i`: each end, the sign first, the longest first
    let signed = |i: usize| -> Vec<usize> {
        let mut out = Vec::new();
        if byte(i) == Some(b'-') {
            let e = run_of(b, i + 1, |b, k| b[k].is_ascii_digit());
            out.extend((i + 2..=e).rev());
        }
        let e = run_of(b, i, |b, k| b[k].is_ascii_digit());
        out.extend((i + 1..=e).rev());
        out
    };
    // `((-?\d+)|\*(-?\d+\$)?)?` at `i`
    let width = |i: usize| -> Vec<usize> {
        let mut out = signed(i);
        if byte(i) == Some(b'*') {
            for e in signed(i + 1) {
                if byte(e) == Some(b'$') {
                    out.push(e + 1);
                }
            }
            out.push(i + 1);
        }
        out.push(i);
        out
    };
    let mut fields = Vec::new();
    let d = run_of(b, p + 1, |b, k| b[k].is_ascii_digit());
    if d > p + 1 && byte(d) == Some(b'$') {
        fields.push(d + 1);
    }
    fields.push(p + 1);
    for f in fields {
        let flags = run_of(b, f, |b, k| {
            matches!(b[k], b'#' | b'0' | b'-' | b' ' | b'+' | b'\'')
        });
        for g in (f..=flags).rev() {
            let mut seps = Vec::new();
            if matches!(byte(g), Some(b',' | b';' | b':' | b'_')) {
                seps.push(g + 1);
            }
            seps.push(g);
            for s in seps {
                for w in width(s) {
                    let mut precs = Vec::new();
                    if byte(w) == Some(b'.') {
                        precs.extend(width(w + 1));
                    }
                    precs.push(w);
                    for q in precs {
                        let mut lens = Vec::new();
                        for m in [
                            &b"hh"[..],
                            b"h",
                            b"ll",
                            b"l",
                            b"j",
                            b"t",
                            b"z",
                            b"q",
                            b"L",
                            b"vh",
                            b"vl",
                            b"v",
                            b"hv",
                            b"hl",
                        ] {
                            if starts(b, q, m) {
                                lens.push(q + m.len());
                            }
                        }
                        lens.push(q);
                        for l in lens {
                            if let Some(e) = conversion(b, l) {
                                return Some(e);
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// `(\[[^\]]+\]|[am]s|[diouxXDOUeEfFgGaACcSspn%])` at `i`.
fn conversion(b: &[u8], i: usize) -> Option<usize> {
    let c = *b.get(i)?;
    if c == b'['
        && let Some(k) = b[i + 1..].iter().position(|&x| x == b']')
        && k > 0
    {
        return Some(i + 2 + k);
    }
    if matches!(c, b'a' | b'm') && b.get(i + 1) == Some(&b's') {
        return Some(i + 2);
    }
    b"diouxXDOUeEfFgGaACcSspn%".contains(&c).then_some(i + 1)
}

/// `(?=(struct|union|enum)([^(;]+(?m:$)|.*\{))` at `p`.
fn c_ds_ahead(b: &[u8], p: usize) -> bool {
    [&b"struct"[..], b"union", b"enum"].iter().any(|w| {
        starts(b, p, w) && {
            let rest = &b[p + w.len()..];
            !rest.iter().any(|&x| x == b'(' || x == b';') || rest.contains(&b'{')
        }
    })
}

#[allow(dead_code)]
fn unused(b: &[u8], i: usize) -> bool {
    is_ident(b, i)
}
