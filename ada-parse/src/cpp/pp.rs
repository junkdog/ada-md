//! The preprocessor: `#define`, `#include` and the other directives, and the `#if` rules that
//! color a disabled branch as a comment, C++'s and C's (which differ in their scopes and in
//! little else).

use super::cxx::Rule;
use super::text::{at, id, is_word, path, path_then, run_of, starts, word_before};
use super::words::c99_at;
use super::*;
use crate::skip_spaces;

/// `^\s*#\s*` then a word in `b`, whose indentation ends at `q`: where the `#` is and the
/// word's bounds.
pub(super) fn directive(b: &[u8], q: usize) -> Option<(usize, usize, usize)> {
    if at(b, q) != b'#' || q >= b.len() {
        return None;
    }
    let s = skip_spaces(b, q + 1);
    let e = super::text::word_end(b, s);
    Some((q, s, e))
}

/// `[\p{Lu}_][\p{Lu}\p{Nd}_]{2,}` with `\b`s at `p`.
fn upper_word(b: &[u8], p: usize) -> Option<usize> {
    // ASCII around it, the common case, without decoding a character
    if let Some(&c) = b.get(p)
        && c < 0x80
        && (p == 0 || b[p - 1] < 0x80)
    {
        let w = |x: u8| x.is_ascii_alphanumeric() || x == b'_';
        if (p > 0 && w(b[p - 1])) || !(c.is_ascii_uppercase() || c == b'_') {
            return None;
        }
        let mut e = p + 1;
        while e < b.len() && (b[e].is_ascii_uppercase() || b[e].is_ascii_digit() || b[e] == b'_') {
            e += 1;
        }
        if e >= b.len() || b[e] < 0x80 {
            return (e >= p + 3 && !(e < b.len() && w(b[e]))).then_some(e);
        }
    }
    upper_word_wide(b, p)
}

/// [`upper_word`] where a character past ASCII is next to it or in it.
#[cold]
#[inline(never)]
fn upper_word_wide(b: &[u8], p: usize) -> Option<usize> {
    if word_before(b, p) || !(super::text::is_upper(b, p) || at(b, p) == b'_') {
        return None;
    }
    let e = super::text::upper_rest_end(b, super::text::next(b, p));
    (e >= p + 3 && !is_word(b, e)).then_some(e)
}

impl<K: Sink> Scan<'_, '_, K> {
    /// The `#if` rules of `preprocessor-global`, `-statements` or `-data-structures` (C++'s),
    /// at the line's start: `#if 1` (enabled), `#if 0` (disabled), any other `#if` but in data
    /// structures.
    pub(super) fn rules(&mut self, p: usize, rule: Rule) -> Option<usize> {
        self.rules_in(p, rule, false)
    }

    /// The same, C's (which has no global ones here).
    pub(super) fn c_rules(&mut self, p: usize, rule: Rule) -> Option<usize> {
        self.rules_in(p, rule, true)
    }

    fn rules_in(&mut self, p: usize, rule: Rule, cl: bool) -> Option<usize> {
        let b = self.b;
        if p != 0 {
            return None;
        }
        let (q, s, e) = self.dir?;
        let lang = if cl { C } else { 0 };
        if starts(b, q, b"#if") {
            let t = skip_spaces(b, q + 3);
            if t > q + 3 {
                let z = run_of(b, t, |b, i| b[i] == b'0');
                let enabled = at(b, z) == b'1' && z < b.len() && !is_word(b, z + 1);
                let disabled = at(b, t) == b'0' && !is_word(b, t + 1);
                if enabled || disabled {
                    let n = if enabled { z + 1 } else { t + 1 };
                    self.put(q, q + 3, IMPORT | PRE | lang);
                    self.put(q + 3, t, PRE | lang);
                    self.put(t, n, DEC | PRE | lang);
                    let c = match (enabled, rule, cl) {
                        (true, Rule::Global, false) => EN_G,
                        (true, Rule::Global, true) => C_EN_G,
                        (true, Rule::Statements, false) => EN_S,
                        (true, Rule::Data, false) => EN_D,
                        (true, Rule::Statements, true) => C_EN_S,
                        (true, Rule::Data, true) => C_EN_D,
                        (false, Rule::Global, false) => DIS_G,
                        (false, Rule::Global, true) => C_DIS_G,
                        (false, Rule::Statements, false) => DIS_S,
                        (false, Rule::Data, false) => DIS_D,
                        (false, Rule::Statements, true) => C_DIS_S,
                        (false, Rule::Data, true) => C_DIS_D,
                    };
                    self.push(n, c);
                    return Some(n);
                }
            }
        }
        if rule != Rule::Data && matches!(&b[s..e], b"if" | b"ifdef" | b"ifndef") {
            let c = match (rule, cl) {
                (Rule::Global, false) => OTHER_G,
                (Rule::Global, true) => C_OTHER_G,
                (_, false) => OTHER_S,
                (_, true) => C_OTHER_S,
            };
            self.push(0, c);
            return Some(self.token(q, e - q, IMPORT | lang));
        }
        None
    }

    /// `preprocessor-expressions` at the line's start: C's `incomplete-inc`, the macro's
    /// `#define`, C's `pragma-mark`, `preprocessor-other`.
    pub(super) fn preprocessor_expressions(&mut self) -> Option<usize> {
        self.pp_expressions(false)
    }

    pub(super) fn pp_expressions(&mut self, cl: bool) -> Option<usize> {
        let b = self.b;
        let lang = if cl { C } else { 0 };
        if let Some((q, s, e)) = self.dir {
            let w = &b[s..e];
            // `^\s*(#i(nc?)?)\b\s*`: no space after the `#`
            if s == q + 1 && matches!(w, b"i" | b"in" | b"inc") {
                let t = skip_spaces(b, e);
                self.put(0, t, PRE | C);
                return Some(t);
            }
            if w == b"define" {
                self.put(q, e, IMPORT_DEFINE | PRE | lang);
                self.push(e, if cl { C_PP_DEFINE } else { PP_DEFINE });
                return Some(e);
            }
            if let Some(r) = self.pragma_mark() {
                return Some(r);
            }
        }
        self.pp_other(cl)
    }

    /// C's `pragma-mark`: `^\s*((#\s*pragma\s+mark)\s+(.*))`.
    pub(super) fn pragma_mark(&mut self) -> Option<usize> {
        let b = self.b;
        let (q, s, e) = self.dir?;
        if &b[s..e] != b"pragma" {
            return None;
        }
        let m = skip_spaces(b, e);
        if m == e || !starts(b, m, b"mark") {
            return None;
        }
        // `\s+` after `mark`, the line's `\n` at its end too
        let t = skip_spaces(b, m + 4);
        if t == m + 4 && t < b.len() {
            return None;
        }
        self.put(q, m + 4, IMPORT_PRAGMA | PRE | C);
        self.put(m + 4, b.len(), PRE | C);
        self.halt = t == m + 4;
        Some(b.len())
    }

    /// `preprocessor-other`, C++'s.
    pub(super) fn preprocessor_other(&mut self, p: usize) -> Option<usize> {
        if p != 0 {
            return None;
        }
        self.pp_other(false)
    }

    pub(super) fn pp_other(&mut self, cl: bool) -> Option<usize> {
        let b = self.b;
        let lang = if cl { C } else { 0 };
        if let Some((q, s, e)) = self.dir {
            let w = &b[s..e];
            let push = match w {
                b"if" | b"ifdef" | b"ifndef" | b"elif" | b"else" | b"line" | b"pragma"
                | b"undef" => Some((IMPORT, pick(cl, PP_DIRECTIVE, C_PP_DIRECTIVE))),
                b"error" | b"warning" => {
                    Some((IMPORT_ERROR, pick(cl, PP_DIAGNOSTIC, C_PP_DIAGNOSTIC)))
                }
                b"include" | b"include_next" | b"import" => {
                    Some((IMPORT_INCLUDE, pick(cl, PP_INCLUDE, C_PP_INCLUDE)))
                }
                b"endif" => {
                    self.put(q, e, IMPORT | PRE | lang);
                    return Some(e);
                }
                _ => None,
            };
            if let Some((leaf, c)) = push {
                self.push(0, c);
                return Some(self.token(q, e - q, leaf | lang));
            }
        }
        if cl {
            self.c_practical_workarounds()
        } else {
            self.practical_workarounds()
        }
    }

    /// `preprocessor-practical-workarounds`, C++'s: its `uppercase-ident-lines`, then C's
    /// `uppercase-calls-without-semicolon`.
    pub(super) fn practical_workarounds(&mut self) -> Option<usize> {
        if !self.upper_line_start() {
            return None;
        }
        if self.upper_ident_line() {
            self.push(0, UPPER_IDENT);
            return Some(self.b.len());
        }
        self.uppercase_call()
    }

    /// C's.
    pub(super) fn c_practical_workarounds(&mut self) -> Option<usize> {
        if !self.upper_line_start() {
            return None;
        }
        if self.upper_ident_line() {
            self.push(0, C_UPPER_IDENT);
            return Some(self.b.len());
        }
        self.uppercase_call()
    }

    /// Whether the line's first byte past its indentation can start an upper-case word, as
    /// both workarounds need.
    fn upper_line_start(&self) -> bool {
        let b = self.b;
        let q = self.lead;
        q < b.len() && (b[q].is_ascii_uppercase() || b[q] == b'_' || b[q] >= 0x80)
    }

    /// `^(\s*\b[\p{Lu}_][\p{Lu}\p{Nd}_]{2,}\b)+\s*(?m:$)`.
    pub(super) fn upper_ident_line(&self) -> bool {
        let b = self.b;
        let mut q = self.lead;
        let mut words = 0;
        while let Some(e) = upper_word(b, q) {
            words += 1;
            q = skip_spaces(b, e);
        }
        words > 0 && q >= b.len()
    }

    /// [`Self::uppercase_call`]'s match: where its name ends and where its `(` is.
    pub(super) fn upper_call(&self) -> Option<(usize, usize)> {
        let b = self.b;
        let q = self.lead;
        let e = upper_word(b, q)?;
        let s = skip_spaces(b, e);
        if at(b, s) != b'(' || s >= b.len() {
            return None;
        }
        let close = b[s + 1..].iter().position(|&x| x == b')')? + s + 1;
        (skip_spaces(b, close + 1) >= b.len()).then_some((e, s))
    }

    /// `^\s*(\b[\p{Lu}_][\p{Lu}\p{Nd}_]{2,}\b)\s*(\()(?=[^)]*\)\s*(?m:$))`.
    fn uppercase_call(&mut self) -> Option<usize> {
        let (e, s) = self.upper_call()?;
        let q = self.lead;
        self.put(q, e, MACRO_FUNCTION | C);
        self.put(s, s + 1, GROUP_BEGIN | C);
        self.push(s + 1, C_MACRO_CALL);
        Some(s + 1)
    }

    /// The preprocessor's contexts, C++'s and C's.
    pub(super) fn pp_context(&mut self, top: u8, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        let cl = top >= C_FIRST;
        let lang = if cl { C } else { 0 };
        match top {
            PP_DEFINE | C_PP_DEFINE => {
                if let Some(r) = self.continuation(p, c) {
                    return Some(r);
                }
                if p >= b.len() {
                    return Some(self.line_end());
                }
                if let Some(r) = self.pp_comments(p, c) {
                    return Some(r);
                }
                // a set: the name keeps the define's meta content scope
                let e = id(b, p)?;
                if at(b, e) == b'(' && e < b.len() {
                    Some(self.then(
                        p,
                        e - p,
                        &[pick(cl, PP_DEFINE_OPEN, C_PP_DEFINE_OPEN)],
                        PP_FUNCTION | lang,
                    ))
                } else {
                    Some(self.then(
                        p,
                        e - p,
                        &[pick(cl, PP_MACRO, C_PP_MACRO)],
                        PP_CONSTANT | lang,
                    ))
                }
            }
            PP_DEFINE_OPEN | C_PP_DEFINE_OPEN => (c == b'(').then(|| {
                self.set(p, &[pick(cl, PP_PARAMS, C_PP_PARAMS)]);
                self.token(p, 1, GROUP_BEGIN | lang)
            }),
            PP_PARAMS | C_PP_PARAMS => {
                if let Some(e) = id(b, p) {
                    return Some(self.token(p, e - p, PARAMETER | lang));
                }
                match c {
                    b')' => {
                        return Some(self.then(
                            p,
                            1,
                            &[pick(cl, PP_MACRO, C_PP_MACRO)],
                            GROUP_END | lang,
                        ));
                    }
                    b',' => {
                        return Some(self.open_after(
                            p,
                            1,
                            pick(cl, PP_PARAM, C_PP_PARAM),
                            SEPARATOR | lang,
                        ));
                    }
                    _ => {}
                }
                if let Some(r) = self.continuation(p, c).or_else(|| self.pp_comments(p, c)) {
                    return Some(r);
                }
                if starts(b, p, b"...") {
                    return Some(self.token(p, 3, VARIADIC | lang));
                }
                if p >= b.len() {
                    // `(?m:$)\n` as `invalid.illegal.unexpected-end-of-line`, and the
                    // parameters go on
                    self.halt = true;
                    return Some(p);
                }
                None
            }
            PP_PARAM | C_PP_PARAM => {
                if let Some(e) = id(b, p) {
                    return Some(self.close(p, e - p, PARAMETER | lang));
                }
                if let Some(r) = self.continuation(p, c).or_else(|| self.pp_comments(p, c)) {
                    return Some(r);
                }
                if starts(b, p, b"...") {
                    return Some(self.token(p, 3, VARIADIC | lang));
                }
                if c == b')' && p < b.len() && self.can_pop(p) {
                    return Some(self.leave(p));
                }
                if p < b.len() && !crate::is_space(c) {
                    let e = run_of(b, p, |b, i| !crate::is_space(b[i]));
                    return Some(self.token(p, e - p, UNEXPECTED_CHAR | lang));
                }
                None
            }
            PP_MACRO | C_PP_MACRO => {
                if let Some(r) = self.continuation(p, c) {
                    return Some(r);
                }
                if p >= b.len() {
                    return Some(self.line_end());
                }
                if let Some(r) = self.pp_comments(p, c) {
                    return Some(r);
                }
                match c {
                    b'{' => return Some(self.token(p, 1, BLOCK_BEGIN | lang)),
                    b'}' => return Some(self.token(p, 1, BLOCK_END | lang)),
                    _ => {}
                }
                if cl {
                    return self.c_exprs(p, c);
                }
                if super::text::is_kw(b, p, b"namespace") {
                    let s = skip_spaces(b, p + 9);
                    if s > p + 9
                        && path_then(b, s, |e| {
                            let t = skip_spaces(b, e);
                            at(b, t) == b'{' && t < b.len()
                        })
                    {
                        self.put(p, p + 9, CONTROL);
                        self.push(s, PP_MACRO_NS);
                        return Some(s);
                    }
                }
                self.exprs(p, c, true, true)
            }
            PP_MACRO_NS => self.identifiers(p).or_else(|| self.any_pop(p)),
            PP_DIRECTIVE | C_PP_DIRECTIVE | PP_DIAGNOSTIC | C_PP_DIAGNOSTIC | PP_INCLUDE
            | C_PP_INCLUDE => {
                if let Some(r) = self.continuation(p, c) {
                    return Some(r);
                }
                if p >= b.len() {
                    return Some(self.line_end());
                }
                if let Some(r) = self.pp_comments(p, c) {
                    return Some(r);
                }
                match top {
                    PP_DIRECTIVE | C_PP_DIRECTIVE => super::text::is_kw(b, p, b"defined")
                        .then(|| self.token(p, 7, CONTROL | lang)),
                    PP_DIAGNOSTIC | C_PP_DIAGNOSTIC => {
                        let s = if cl {
                            self.c_strings(p, c)
                        } else {
                            self.strings(p, c)
                        };
                        s.or_else(|| {
                            (!crate::is_space(c)).then(|| {
                                let e = run_of(b, p, |b, i| !crate::is_space(b[i]));
                                self.token(p, e - p, UNQUOTED | lang)
                            })
                        })
                    }
                    _ => match c {
                        b'"' => Some(self.open(
                            p,
                            1,
                            pick(cl, PP_INC_DQ, C_PP_INC_DQ),
                            INC_DQ_BEGIN | lang,
                        )),
                        b'<' => Some(self.open(
                            p,
                            1,
                            pick(cl, PP_INC_LT, C_PP_INC_LT),
                            INC_LT_BEGIN | lang,
                        )),
                        _ => None,
                    },
                }
            }
            PP_INC_DQ | C_PP_INC_DQ => {
                (c == b'"' && p < b.len()).then(|| self.close(p, 1, INC_DQ_END | lang))
            }
            PP_INC_LT | C_PP_INC_LT => {
                (c == b'>' && p < b.len()).then(|| self.close(p, 1, INC_LT_END | lang))
            }
            EN_G | EN_S | EN_D | C_EN_G | C_EN_S | C_EN_D | DIS_G | DIS_S | DIS_D | C_DIS_G
            | C_DIS_S | C_DIS_D => {
                if p == 0
                    && let Some((q, s, e)) = self.dir
                {
                    let w = &b[s..e];
                    if w == b"endif" {
                        self.put(q, e, IMPORT | PRE | lang);
                        self.pop(e);
                        return Some(e);
                    }
                    if w == b"else" {
                        self.put(q, e, IMPORT_ELSE | PRE | lang);
                        let next = match top {
                            EN_G | EN_S | EN_D => ELSE_COMMENT,
                            C_EN_G | C_EN_S | C_EN_D => C_ELSE_COMMENT,
                            DIS_G => DIS_ELSE_G,
                            C_DIS_G => C_DIS_ELSE_G,
                            DIS_S => DIS_ELSE_S,
                            DIS_D => DIS_ELSE_D,
                            C_DIS_S => C_DIS_ELSE_S,
                            _ => C_DIS_ELSE_D,
                        };
                        self.push(e, next);
                        return Some(e);
                    }
                }
                let inner = match top {
                    EN_G => EN_IN_G,
                    EN_S => EN_IN_S,
                    EN_D => EN_IN_D,
                    C_EN_G => C_EN_IN_G,
                    C_EN_S => C_EN_IN_S,
                    C_EN_D => C_EN_IN_D,
                    DIS_G | DIS_S | DIS_D => IF_COMMENT,
                    _ => C_IF_COMMENT,
                };
                Some(self.enter(p, &[inner]))
            }
            EN_IN_G | EN_IN_S | EN_IN_D | DIS_ELSE_G | DIS_ELSE_S | DIS_ELSE_D | C_EN_IN_G
            | C_EN_IN_S | C_EN_IN_D | C_DIS_ELSE_G | C_DIS_ELSE_S | C_DIS_ELSE_D => {
                let dis = matches!(
                    top,
                    DIS_ELSE_G
                        | DIS_ELSE_S
                        | DIS_ELSE_D
                        | C_DIS_ELSE_G
                        | C_DIS_ELSE_S
                        | C_DIS_ELSE_D
                );
                let ends: &[&[u8]] = if dis {
                    &[b"endif"]
                } else {
                    &[b"else", b"endif"]
                };
                if p == 0 && self.directive_is(ends) && self.can_pop(0) {
                    return Some(self.leave(0));
                }
                if matches!(top, C_EN_IN_G | C_DIS_ELSE_G) {
                    if let Some(r) = self.c_rules(p, Rule::Global) {
                        return Some(r);
                    }
                    if c == b'}' {
                        return Some(self.open_after(p, 1, C_NEGATED, BLOCK_END | C));
                    }
                    return self.c_global(p, c);
                }
                if cl {
                    if c == b'}' {
                        return Some(self.open_after(p, 1, C_NEGATED, BLOCK_END | C));
                    }
                    return if matches!(top, C_EN_IN_D | C_DIS_ELSE_D) {
                        self.c_data_structures_body(p, c)
                    } else {
                        self.c_statements(p, c)
                    };
                }
                let global = matches!(top, EN_IN_G | DIS_ELSE_G);
                if global && let Some(r) = self.rules(p, Rule::Global) {
                    return Some(r);
                }
                if c == b'}' {
                    return Some(self.open_after(p, 1, NEGATED, BLOCK_END));
                }
                match top {
                    EN_IN_G | DIS_ELSE_G => self.global_in(p, c),
                    EN_IN_S | DIS_ELSE_S => self.statements(p, c),
                    _ => self.data_structures_body_in(p, c),
                }
            }
            ELSE_COMMENT | IF_COMMENT | C_ELSE_COMMENT | C_IF_COMMENT | C_DISABLED => {
                if p != 0 {
                    return None;
                }
                let (q, s, e) = self.dir?;
                let w = &b[s..e];
                match top {
                    ELSE_COMMENT | C_ELSE_COMMENT if w == b"endif" && self.can_pop(0) => {
                        return Some(self.leave(0));
                    }
                    IF_COMMENT | C_IF_COMMENT
                        if matches!(w, b"else" | b"endif") && self.can_pop(0) =>
                    {
                        return Some(self.leave(0));
                    }
                    C_DISABLED if w == b"endif" => {
                        self.put(q, e, PRE | C);
                        self.pop(e);
                        return Some(e);
                    }
                    _ => {}
                }
                if matches!(w, b"if" | b"ifdef" | b"ifndef") {
                    self.put(q, e, PRE | C);
                    self.push(e, C_DISABLED);
                    return Some(e);
                }
                self.pragma_mark()
            }
            OTHER_G | OTHER_S | C_OTHER_G | C_OTHER_S => {
                if let Some(r) = self.continuation(p, c).or_else(|| self.pp_comments(p, c)) {
                    return Some(r);
                }
                if super::text::is_kw(b, p, b"defined") {
                    return Some(self.token(p, 7, CONTROL | lang));
                }
                if p >= b.len() {
                    let next = match top {
                        OTHER_G => IF_G,
                        C_OTHER_G => C_IF_G,
                        OTHER_S => IF_S,
                        _ => C_IF_S,
                    };
                    self.set(p, &[next]);
                    self.halt = true;
                    return Some(p);
                }
                None
            }
            IF_G | IF_S | C_IF_G | C_IF_S => {
                if p == 0
                    && let Some((q, s, e)) = self.dir
                {
                    let w = &b[s..e];
                    if w == b"endif" {
                        self.put(q, e, IMPORT | PRE | lang);
                        self.pop(e);
                        return Some(e);
                    }
                    if matches!(w, b"elif" | b"else") {
                        let elif = match top {
                            IF_G => ELIF_G,
                            C_IF_G => C_ELIF_G,
                            IF_S => ELIF_S,
                            _ => C_ELIF_S,
                        };
                        return Some(self.enter(0, &[elif]));
                    }
                }
                if c == b'{' {
                    let block = match top {
                        IF_G => BLOCK_IF_G,
                        C_IF_G => C_BLOCK_IF_G,
                        IF_S => BLOCK_IF_S,
                        _ => C_BLOCK_IF_S,
                    };
                    return Some(self.then(p, 1, &[block], BLOCK_BEGIN | lang));
                }
                if top == IF_G {
                    if let Some(r) = self.rules(p, Rule::Global) {
                        return Some(r);
                    }
                    if c == b'}' {
                        return Some(self.open_after(p, 1, NEGATED, BLOCK_END));
                    }
                    return self.global_in(p, c);
                }
                if top == C_IF_G {
                    if let Some(r) = self.c_rules(p, Rule::Global) {
                        return Some(r);
                    }
                    if c == b'}' {
                        return Some(self.open_after(p, 1, C_NEGATED, BLOCK_END | C));
                    }
                    return self.c_global(p, c);
                }
                if self.pp_call_ahead(p, cl) {
                    return Some(self.shift(p, &[pick(cl, PP_FC, C_PP_FC)]));
                }
                if c == b'}' {
                    return Some(self.open_after(
                        p,
                        1,
                        pick(cl, NEGATED, C_NEGATED),
                        BLOCK_END | lang,
                    ));
                }
                if cl {
                    self.c_statements(p, c)
                } else {
                    self.statements(p, c)
                }
            }
            ELIF_G | ELIF_S | C_ELIF_G | C_ELIF_S => {
                if p == 0 && self.directive_is(&[b"endif"]) && self.can_pop(0) {
                    return Some(self.leave(0));
                }
                if top == ELIF_G
                    && let Some(r) = self.rules(p, Rule::Global)
                {
                    return Some(r);
                }
                if top == C_ELIF_G
                    && let Some(r) = self.c_rules(p, Rule::Global)
                {
                    return Some(r);
                }
                if c == b'}' {
                    return Some(self.open_after(
                        p,
                        1,
                        pick(cl, NEGATED, C_NEGATED),
                        BLOCK_END | lang,
                    ));
                }
                match top {
                    ELIF_G => self.global_in(p, c),
                    C_ELIF_G => self.c_global(p, c),
                    ELIF_S => self.statements(p, c),
                    _ => self.c_statements(p, c),
                }
            }
            BLOCK_IF_G | BLOCK_IF_S | C_BLOCK_IF_G | C_BLOCK_IF_S => {
                let (finish, elif, if_branch) = match top {
                    BLOCK_IF_G => (FINISH_G, ELIF_G, IF_G),
                    BLOCK_IF_S => (FINISH_S, ELIF_S, IF_S),
                    C_BLOCK_IF_G => (C_FINISH_G, C_ELIF_G, C_IF_G),
                    _ => (C_FINISH_S, C_ELIF_S, C_IF_S),
                };
                if p == 0
                    && let Some((q, s, e)) = self.dir
                {
                    let w = &b[s..e];
                    if w == b"endif" {
                        self.put(q, e, IMPORT | PRE | lang);
                        self.set(e, &[finish]);
                        return Some(e);
                    }
                    if matches!(w, b"elif" | b"else") {
                        return Some(self.enter(0, &[elif]));
                    }
                }
                if c == b'}' {
                    return Some(self.then(p, 1, &[if_branch], BLOCK_END | lang));
                }
                if cl {
                    self.c_statements(p, c)
                } else {
                    self.statements(p, c)
                }
            }
            FINISH_G | FINISH_S | C_FINISH_G | C_FINISH_S => {
                if p == 0
                    && let Some((q, s, e)) = self.dir
                    && matches!(&b[s..e], b"if" | b"ifdef" | b"ifndef")
                {
                    self.put(q, e, IMPORT | PRE | lang);
                    let next = match top {
                        FINISH_G => FINISH_IF_G,
                        C_FINISH_G => C_FINISH_IF_G,
                        FINISH_S => FINISH_IF_S,
                        _ => C_FINISH_IF_S,
                    };
                    self.set(e, &[next]);
                    return Some(e);
                }
                if c == b'}' {
                    return Some(self.close(p, 1, BLOCK_END | lang));
                }
                if cl {
                    self.c_statements(p, c)
                } else {
                    self.statements(p, c)
                }
            }
            FINISH_IF_G | FINISH_IF_S | C_FINISH_IF_G | C_FINISH_IF_S => {
                if p == 0
                    && let Some((q, s, e)) = self.dir
                    && &b[s..e] == b"endif"
                {
                    // C's capture is in `meta.preprocessor`, C++'s isn't
                    self.put(q, e, IMPORT | if cl { PRE | C } else { 0 });
                    self.pop(e);
                    return Some(e);
                }
                if c == b'}' {
                    let next = match top {
                        FINISH_IF_G => IF_G,
                        C_FINISH_IF_G => C_IF_G,
                        FINISH_IF_S => IF_S,
                        _ => C_IF_S,
                    };
                    return Some(self.then(p, 1, &[next], BLOCK_END | lang));
                }
                if cl {
                    self.c_statements(p, c)
                } else {
                    self.statements(p, c)
                }
            }
            PP_FC | C_PP_FC => {
                if let Some(e) = c99_at(b, p) {
                    return Some(self.token(p, e - p, C99 | C));
                }
                if !cl {
                    let (s, q) = if starts(b, p, b"::") {
                        (p + 2, skip_spaces(b, p + 2))
                    } else {
                        (p, p)
                    };
                    if let Some(e) = id(b, q) {
                        let d = skip_spaces(b, e);
                        self.put(p, s, FUNCTION_ACCESSOR);
                        self.put(s, q, FUNCTION);
                        self.put(q, e, FUNCTION);
                        if starts(b, d, b"::") {
                            let f = skip_spaces(b, d + 2);
                            self.put(e, d, FUNCTION);
                            self.put(d, d + 2, FUNCTION_ACCESSOR);
                            self.put(d + 2, f, FUNCTION);
                            return Some(f);
                        }
                        return Some(e);
                    }
                } else if let Some(e) = id(b, p) {
                    return Some(self.token(p, e - p, FUNCTION | C));
                }
                (c == b'(').then(|| {
                    self.then(
                        p,
                        1,
                        &[pick(cl, PP_FC_ARGS, C_PP_FC_ARGS)],
                        GROUP_BEGIN | lang,
                    )
                })
            }
            PP_FC_ARGS | C_PP_FC_ARGS => {
                if c == b')' {
                    return Some(self.then(p, 1, &[pick(cl, IF_S, C_IF_S)], GROUP_END | lang));
                }
                if p == 0
                    && let Some((q, s, e)) = self.dir
                {
                    let w = &b[s..e];
                    if matches!(w, b"elif" | b"else") {
                        self.put(q, e, IMPORT | PRE | lang);
                        self.set(e, &[pick(cl, IF_S, C_IF_S)]);
                        return Some(e);
                    }
                    if w == b"endif" {
                        self.put(q, e, IMPORT | PRE | lang);
                        self.set(e, &[pick(cl, PAREN, C_PAREN)]);
                        return Some(e);
                    }
                }
                if cl {
                    self.c_exprs(p, c)
                } else {
                    self.exprs(p, c, true, true)
                }
            }
            UPPER_IDENT | C_UPPER_IDENT => {
                if cl {
                    let q = skip_spaces(b, p);
                    if let Some(e) = id(b, q)
                        && super::text::paren_after(b, e)
                    {
                        self.put(q, e, ENTITY_FUNCTION | C);
                        self.set(e, &[C_FN_PARAMS]);
                        return Some(e);
                    }
                } else if super::text::is_word(b, p) || starts(b, p, b"::") {
                    let generic = path_then(b, p, |e| {
                        let Some(g) = (at(b, e) == b'<')
                            .then(|| super::text::generic(b, e))
                            .flatten()
                        else {
                            return false;
                        };
                        super::text::paren_after(b, g)
                            || path_then(b, g, |e| super::text::paren_after(b, e))
                    });
                    if generic {
                        return Some(self.shift(p, &[FN_PARAMS, G_FN_ID_GENERIC]));
                    }
                    if path_then(b, p, |e| super::text::paren_after(b, e)) {
                        return Some(self.shift(p, &[FN_PARAMS, G_FN_ID]));
                    }
                }
                (p == 0 && self.can_pop(0)).then(|| self.leave(0))
            }
            _ => None,
        }
    }

    /// `(?=(?!if|for|switch|while|decltype|sizeof|__declspec|__attribute__|typeid|alignof|
    /// alignas|static_assert)PATH\s*\()` at `p` (C's without the last four, and `\bID\b`
    /// for `PATH`).
    pub(super) fn pp_call_ahead(&self, p: usize, cl: bool) -> bool {
        let b = self.b;
        let mut stops: &[&[u8]] = &[
            b"if",
            b"for",
            b"switch",
            b"while",
            b"decltype",
            b"sizeof",
            b"__declspec",
            b"__attribute__",
            b"typeid",
            b"alignof",
            b"alignas",
            b"static_assert",
        ];
        if cl {
            stops = &stops[..8];
            if stops.iter().any(|w| starts(b, p, w)) {
                return false;
            }
            return id(b, p).is_some_and(|e| super::text::paren_after(b, e));
        }
        if stops.iter().any(|w| starts(b, p, w)) || !(is_word(b, p) || starts(b, p, b"::")) {
            return false;
        }
        path(b, p, |_, e| super::text::paren_after(b, e).then_some(())).is_some()
    }

    /// `global`, for the rules' branches.
    pub(super) fn global_in(&mut self, p: usize, c: u8) -> Option<usize> {
        self.global(p, c)
    }

    /// `data-structures-body`, for the rules' branches.
    fn data_structures_body_in(&mut self, p: usize, c: u8) -> Option<usize> {
        self.data_structures_body(p, c)
    }

    /// `n` bytes at `p`, `leaf`, then `c` pushed (whose meta content scope they don't carry).
    pub(super) fn open_after(&mut self, p: usize, n: usize, c: u8, leaf: u16) -> usize {
        self.put(p, p + n, leaf);
        self.push(p + n, c);
        p + n
    }

    /// C's `preprocessor-line-continuation`: `(\\)(?m:$)\n`, `\\(\s+?)(?m:$)`.
    pub(super) fn continuation(&mut self, p: usize, c: u8) -> Option<usize> {
        self.line_continuation(p, c)
    }

    /// C's `preprocessor-line-ending`: `(?m:$)\n`, a pop.
    pub(super) fn line_end(&mut self) -> usize {
        let n = self.b.len();
        self.pop(n);
        self.halt = true;
        n
    }

    /// C's `preprocessor-comments`.
    pub(super) fn pp_comments(&mut self, p: usize, c: u8) -> Option<usize> {
        if c != b'/' || p >= self.b.len() {
            return None;
        }
        match at(self.b, p + 1) {
            b'*' => Some(self.open(p, 2, C_PP_COMMENT, COMMENT_MARK | C)),
            b'/' => Some(self.open(p, 2, C_PP_LINE_COMMENT, LINE_MARK | C)),
            _ => None,
        }
    }
}

/// C++'s context or C's.
fn pick(cl: bool, cpp: u8, c: u8) -> u8 {
    if cl { c } else { cpp }
}
