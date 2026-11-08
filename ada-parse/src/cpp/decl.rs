//! C++'s declarations on frames (X-057): the top level's and a namespace's statements, a class's
//! body's line start, and the contexts bat's grammar reads a declaration's head through
//! (`global-modifier`, `global-type`, `global-maybe-function`, a function's name, its
//! parameters' `(`, what follows them, a declarator's name; a member's `data-structures-modifier`,
//! `-type` and `-maybe-method`, a method's name, its parameters' `(` and what follows them), a
//! template path's name (`GT_NAME`), and C's global declarations. The grammar decides each step
//! by a lookahead along the line (a path, then what follows it); here a turn reads the path once,
//! plain ASCII names joined by `::`, and takes the arm's step from what follows it: the same
//! push, set or pop, through the same helpers, so the output and the stack are the arms' byte for
//! byte. A path or a byte the frames don't read (a template's `<`, a spaced `::`, a destructor's
//! `~`, a name past ASCII, a keyword with a pattern of its own) is the arm's turn, at the same
//! place.

use super::cxx::ds_keyword;
use super::cxx::is_stop_word;
use super::text::{ascii_word_end, at, paren_after, paren_or_eol, starts};
use super::words::{
    Kw, c_value, is_c_control, is_c_modifier, is_c_type, lookup_inline as lookup, support,
};
use super::*;
use crate::clike::{CLASS, HIGH, WORD};
use crate::skip_spaces;

impl<K: Sink> Scan<'_, '_, K> {
    /// A plain path at `p`: ASCII names joined by `::` without blanks, none of them `template`
    /// or `operator`, the last not followed by a byte past ASCII or by a spaced `::`. Where it
    /// ends, how many names it has, and where its first ends; `None` for any other.
    #[inline(always)]
    pub(super) fn plain_path(&self, p: usize) -> Option<(usize, usize, usize)> {
        let b = self.b;
        let mut q = p;
        let mut n = 0;
        let mut first = 0;
        loop {
            let c = *b.get(q)?;
            if !(c.is_ascii_alphabetic() || c == b'_') {
                return None;
            }
            let e = ascii_word_end(b, q + 1);
            if e < b.len() && b[e] >= 0x80 {
                return None;
            }
            let w = &b[q..e];
            if w == b"template" || w == b"operator" {
                return None;
            }
            if n == 0 {
                first = e;
            }
            n += 1;
            if at(b, e) == b':' && at(b, e + 1) == b':' {
                q = e + 2;
                continue;
            }
            let s = skip_spaces(b, e);
            if s > e && starts(b, s, b"::") {
                return None;
            }
            return Some((e, n, first));
        }
    }

    /// The top level's line start, `MAIN` on top at 0 (its `#if` rules and `global`): a
    /// declaration's head, which `^\s*(?=\w)` hands to `global-modifier` (pushed at the line's
    /// first word, or entered at 0 without indentation); `None` for anything a pattern before it
    /// may take (a directive, an upper-case workaround's line, `extern`, and without indentation
    /// `template`, `namespace`, a keyword's or value's word, a path before `<`, a string's
    /// prefix).
    pub(super) fn global_start(&mut self) -> Option<usize> {
        let b = self.b;
        if self.dir.is_some() {
            return None;
        }
        let q = self.lead;
        let Some(&c) = b.get(q) else {
            // a blank line: the arm's miss passes its blanks
            return Some(self.miss(q));
        };
        if !(c.is_ascii_alphabetic() || c == b'_') {
            // past indentation, a byte no name starts: the arm misses to it (its line-start
            // patterns want a name or a directive); at 0 the arm's or the line's first turn
            return (q > 0 && CLASS[usize::from(c)] & (WORD | HIGH) == 0).then(|| self.miss(q));
        }
        let (e, _, first) = self.plain_path(q)?;
        let w = &b[q..first];
        if w == b"extern" {
            return None;
        }
        if (c.is_ascii_uppercase() || c == b'_') && self.upper_line() {
            return None;
        }
        if q == 0 {
            // `global`'s patterns before its `^\s*(?=\w)` try the word at 0
            if w == b"template" || w == b"namespace" {
                return None;
            }
            if !matches!(
                lookup(w),
                Kw::Other | Kw::C99 | Kw::Modifier | Kw::CxxType | Kw::CType | Kw::Support(_)
            ) {
                return None;
            }
            let s = skip_spaces(b, e);
            if s < b.len() && b[s] == b'<' || matches!(at(b, first), b'"' | b'\'') {
                return None;
            }
            return Some(self.enter(0, &[G_MODIFIER]));
        }
        self.push(q, G_MODIFIER);
        Some(q)
    }

    /// A class's body's line start, a byte other than `}` at 0 (`data-structures-body`): a
    /// member's head, which `^\s*(?=~?\w|::)` hands to `data-structures-modifier` (pushed at
    /// the indentation's end, or entered at 0 without indentation); blanks before anything
    /// else, which the arm's miss passes. `None` for a directive, an upper-case workaround's
    /// line, a name past ASCII, and without indentation the body's own words and a byte no
    /// member starts with (the turn the frame takes past the line's start).
    pub(super) fn ds_body_start(&mut self) -> Option<usize> {
        let b = self.b;
        if self.dir.is_some() {
            return None;
        }
        let q = self.lead;
        let Some(&c) = b.get(q) else {
            return Some(self.miss(q));
        };
        if c >= 0x80 || ((c.is_ascii_uppercase() || c == b'_') && self.upper_line()) {
            return None;
        }
        let t = if c == b'~' { q + 1 } else { q };
        let d = at(b, t);
        if d >= 0x80 && t < b.len() {
            return None;
        }
        let member = (t < b.len() && CLASS[usize::from(d)] & WORD != 0) || starts(b, q, b"::");
        if q == 0 {
            if c.is_ascii_alphabetic() || c == b'_' {
                let e = ascii_word_end(b, 1);
                if e < b.len() && b[e] >= 0x80 {
                    return None;
                }
                if matches!(
                    &b[..e],
                    b"template" | b"using" | b"typedef" | b"private" | b"protected" | b"public"
                ) {
                    return None;
                }
            }
            return member.then(|| self.enter(0, &[DS_MODIFIER]));
        }
        if member {
            self.push(q, DS_MODIFIER);
            return Some(q);
        }
        Some(self.miss(q))
    }

    /// A declaration's frame's turn at `p` ([`G_MODIFIER`], [`G_TYPE`], [`G_MAYBE_FN`],
    /// [`G_FN_ID`], [`FN_PARAMS`], [`FN_CONTINUE`], [`IDENT_POP`]): the arm's one match, a miss
    /// (blanks the arm passes, [`Scan::miss`]), or `None` for the arm's turn. Never one
    /// that could be past syntect's guard against a loop (its matches that take nothing are
    /// left to [`Scan::run`] at the limit).
    #[inline(never)]
    pub(super) fn decl_turn(&mut self, p: usize, top: u8) -> Option<usize> {
        let b = self.b;
        if self.stuck >= STUCK {
            return None;
        }
        let Some(&c) = b.get(p) else {
            // the line's end: `(?=\s)` takes its `\n`, an identifier's frame pops; the others
            // match nothing there ([`super::code::ends_at_eol`])
            return match top {
                G_TYPE => Some(self.shift(p, &[G_MAYBE_FN])),
                DS_TYPE => Some(self.shift(p, &[DS_MAYBE_METHOD])),
                IDENT_POP => self.can_pop(p).then(|| self.leave(p)),
                _ => None,
            };
        };
        if p == 0 && c == b'/' {
            return None;
        }
        // a word's byte that starts no name (a digit, a name's inside where the arm's advance
        // stopped, a byte past ASCII) is the arm's
        if c >= 0x80
            || c.is_ascii_digit()
            || ((c.is_ascii_alphabetic() || c == b'_') && self.inside_word(p))
        {
            return None;
        }
        let name = c.is_ascii_alphabetic() || c == b'_';
        match top {
            G_MODIFIER => {
                if crate::is_space(c) {
                    return Some(self.miss(skip_spaces(b, p)));
                }
                if !name {
                    return None;
                }
                let (e, n, first) = self.plain_path(p)?;
                match lookup(&b[p..first]) {
                    Kw::Modifier => return Some(self.token(p, first - p, MODIFIER)),
                    Kw::Alignas | Kw::Attribute | Kw::Declspec => return None,
                    _ => {}
                }
                // a constructor's lookahead (`Foo::Foo(`, `Foo<T>::Foo(`) is the arm's, and a
                // string's prefix
                let s = skip_spaces(b, e);
                if (n >= 2 && paren_after(b, e))
                    || (at(b, s) == b'<' && s < b.len())
                    || matches!(at(b, first), b'"' | b'\'')
                {
                    return None;
                }
                Some(self.shift(p, &[G_TYPE]))
            }
            DS_MODIFIER => {
                if crate::is_space(c) {
                    return Some(self.miss(skip_spaces(b, p)));
                }
                // a comment, a destructor, a path from `::`: the arm's
                if matches!(c, b'/' | b'~' | b':') || (c == b'*' && at(b, p + 1) == b'/') {
                    return None;
                }
                if !name {
                    if c.is_ascii_digit() || c >= 0x80 {
                        return None;
                    }
                    return Some(self.shift(p, &[DS_TYPE]));
                }
                let e = ascii_word_end(b, p + 1);
                if e < b.len() && b[e] >= 0x80 {
                    return None;
                }
                let w = &b[p..e];
                if matches!(w, b"friend" | b"static_assert") {
                    return None;
                }
                match lookup(w) {
                    Kw::Modifier => return Some(self.token(p, e - p, MODIFIER)),
                    Kw::Alignas | Kw::Attribute | Kw::Declspec => return None,
                    _ => {}
                }
                // a macro's name before the member (`preprocessor-workaround-eat-macro-…`)
                if e - p >= 3
                    && (c.is_ascii_uppercase() || c == b'_')
                    && w.iter()
                        .all(|&x| x.is_ascii_uppercase() || x.is_ascii_digit() || x == b'_')
                {
                    return None;
                }
                let s = skip_spaces(b, e);
                match at(b, s) {
                    // a call's or a constructor's `(`
                    b'(' if s < b.len() => return None,
                    // a qualified destructor's or constructor's name
                    b':' if at(b, s + 1) == b':' => {
                        let t = skip_spaces(b, s + 2);
                        let d = at(b, t);
                        if d == b'~' || d >= 0x80 {
                            return None;
                        }
                        if (d.is_ascii_alphabetic() || d == b'_')
                            && paren_after(b, ascii_word_end(b, t + 1))
                        {
                            return None;
                        }
                    }
                    _ => {}
                }
                Some(self.shift(p, &[DS_TYPE]))
            }
            G_TYPE | DS_TYPE => {
                // `data-structures-type` differs in what follows it and in taking no call
                let ds = top == DS_TYPE;
                if crate::is_space(c) {
                    return Some(self.shift(p, &[if ds { DS_MAYBE_METHOD } else { G_MAYBE_FN }]));
                }
                match c {
                    b'*' if at(b, p + 1) == b'/' => return None,
                    b'*' | b'&' => return Some(self.token(p, 1, OPERATOR)),
                    b'<' => {
                        if matches!(at(b, p + 1), b'<' | b'=') {
                            return None;
                        }
                        return Some(self.open(p, 1, ANGLE, GENERIC_BEGIN));
                    }
                    b'/' | b':' => return None,
                    _ => {}
                }
                if !name {
                    if c.is_ascii_digit() || c >= 0x80 || !self.can_pop(p) {
                        return None;
                    }
                    return Some(self.leave(p));
                }
                let first = ascii_word_end(b, p + 1);
                if first < b.len() && b[first] >= 0x80 {
                    return None;
                }
                let w = &b[p..first];
                if is_stop_word(w, true) || ds_keyword(b, p).iter().any(Option::is_some) {
                    return None;
                }
                let kw = lookup(w);
                let leaf = match kw {
                    Kw::Cast | Kw::Decltype => return None,
                    Kw::CxxType => Some(STORAGE_TYPE),
                    Kw::CType => Some(STORAGE_TYPE | C),
                    Kw::Support(leaf) => Some(leaf),
                    _ => None,
                };
                if let Some(leaf) = leaf {
                    return Some(self.token(p, first - p, leaf));
                }
                if ds {
                    if let Kw::Value(leaf) = kw {
                        return Some(self.token(p, first - p, leaf));
                    }
                    return self.identifiers(p);
                }
                // a call: its name and group, where a `)` comes after its `(` and a byte
                let s = skip_spaces(b, first);
                if at(b, s) == b'(' && s < b.len() {
                    if b[s + 1..]
                        .iter()
                        .position(|&x| x == b')')
                        .is_some_and(|k| k > 0)
                    {
                        self.put(p, first, FUNCTION);
                        self.put(s, s + 1, GROUP_BEGIN);
                        self.push(s + 1, PAREN);
                        return Some(s + 1);
                    }
                    return None;
                }
                // a path before a call is `#anon_global-type_2`'s, the arm's
                let (e, _, _) = self.plain_path(p)?;
                if paren_after(b, e) {
                    return None;
                }
                if let Kw::Value(leaf) = kw {
                    return Some(self.token(p, first - p, leaf));
                }
                self.identifiers(p)
            }
            G_MAYBE_FN | DS_MAYBE_METHOD => {
                // `data-structures-maybe-method` differs in its operator, its macro's name, a
                // declarator at the line's end and a method's `(`
                let ds = top == DS_MAYBE_METHOD;
                if crate::is_space(c) {
                    return Some(self.miss(skip_spaces(b, p)));
                }
                match c {
                    b'*' if at(b, p + 1) == b'/' => return None,
                    b'*' | b'&' => return Some(self.token(p, 1, OPERATOR)),
                    b'<' => {
                        if matches!(at(b, p + 1), b'<' | b'=') {
                            return None;
                        }
                        return Some(self.open(p, 1, ANGLE, GENERIC_BEGIN));
                    }
                    b'/' | b':' => return None,
                    _ => {}
                }
                if !name {
                    if c.is_ascii_digit() || c >= 0x80 {
                        return None;
                    }
                    return self.any_pop(p);
                }
                let first = ascii_word_end(b, p + 1);
                if first < b.len() && b[first] >= 0x80 {
                    return None;
                }
                let w = &b[p..first];
                if is_stop_word(w, false) {
                    return None;
                }
                if matches!(
                    w,
                    b"const" | b"constexpr" | b"mutable" | b"typename" | b"volatile"
                ) {
                    return Some(self.token(p, first - p, MODIFIER));
                }
                let leaf = match lookup(w) {
                    Kw::Decltype | Kw::Alignas | Kw::Attribute | Kw::Declspec => return None,
                    Kw::CxxType => STORAGE_TYPE,
                    Kw::CType => STORAGE_TYPE | C,
                    Kw::Support(leaf) => leaf,
                    Kw::Modifier => MODIFIER,
                    _ => 0,
                };
                if leaf != 0 {
                    return Some(self.token(p, first - p, leaf));
                }
                if ds {
                    if starts(b, p, b"operator") {
                        return None;
                    }
                    return self.ds_maybe_method(p);
                }
                // `[\p{Lu}\p{Nd}_]+\s*$`: a macro's name ending the line, no scope
                let mut caps = p;
                while caps < b.len()
                    && (b[caps].is_ascii_uppercase() || b[caps].is_ascii_digit() || b[caps] == b'_')
                {
                    caps += 1;
                }
                if caps > p
                    && (caps >= b.len() || b[caps] < 0x80)
                    && skip_spaces(b, caps) >= b.len()
                {
                    return Some(skip_spaces(b, caps));
                }
                let (e, _, _) = self.plain_path(p)?;
                // a template's arguments after the path (a generic call, an operator after them)
                let s = skip_spaces(b, e);
                if at(b, s) == b'<' && s < b.len() {
                    return None;
                }
                // `(?=PATH([ \t]+|[*&])(?!\s*(<|::|\(|$)))`: a declarator's name
                let mut t = e;
                while t < b.len() && matches!(b[t], b' ' | b'\t') {
                    t += 1;
                }
                let after = if t > e {
                    Some(t)
                } else if matches!(at(b, e), b'*' | b'&') && e < b.len() {
                    Some(e + 1)
                } else {
                    None
                };
                if let Some(after) = after {
                    let s = skip_spaces(b, after);
                    if s < b.len() && !(b[s] == b'<' || b[s] == b'(' || starts(b, s, b"::")) {
                        return Some(self.enter(p, &[IDENT_POP]));
                    }
                }
                if paren_or_eol(b, e) {
                    return Some(self.shift(p, &[FN_PARAMS, G_FN_ID]));
                }
                self.any_pop(p)
            }
            G_FN_ID => {
                if !name {
                    // `::` is a path's, a blank the arm's miss
                    if c == b':' || crate::is_space(c) {
                        return None;
                    }
                    return self.any_pop(p);
                }
                let e = ascii_word_end(b, p + 1);
                if e < b.len() && b[e] >= 0x80 {
                    return None;
                }
                if starts(b, skip_spaces(b, e), b"::") {
                    return self.identifiers(p);
                }
                Some(self.token(p, e - p, ENTITY_FUNCTION))
            }
            FN_PARAMS => match c {
                b'(' => Some(self.then(p, 1, &[FN_PARAMS_IN], GROUP_BEGIN)),
                b'/' => None,
                _ => self.any_pop(p),
            },
            METHOD_PARAMS => match c {
                b'(' => Some(self.then(p, 1, &[METHOD_PARAMS_IN], GROUP_BEGIN)),
                b'/' => None,
                _ => self.any_pop(p),
            },
            FN_CONTINUE | METHOD_CONTINUE => {
                if crate::is_space(c) {
                    return Some(self.miss(skip_spaces(b, p)));
                }
                match c {
                    b';' => return self.can_pop(p).then(|| self.leave(p)),
                    b'{' => return Some(self.then(p, 1, &[FN_BLOCK], BLOCK_BEGIN)),
                    // a method's member initializers
                    b':' if top == METHOD_CONTINUE => return Some(self.shift(p, &[MC_COLON])),
                    b'=' => return Some(self.token(p, 1, ASSIGN)),
                    b'&' => return Some(self.token(p, 1, OPERATOR)),
                    b'/' | b'-' | b'0' => return None,
                    _ => {}
                }
                if name {
                    let e = ascii_word_end(b, p + 1);
                    if e < b.len() && b[e] >= 0x80 {
                        return None;
                    }
                    if matches!(
                        &b[p..e],
                        b"const" | b"final" | b"noexcept" | b"override" | b"default" | b"delete"
                    ) {
                        return Some(self.token(p, e - p, MODIFIER));
                    }
                }
                if c.is_ascii_digit() || c >= 0x80 {
                    return None;
                }
                self.any_pop(p)
            }
            IDENT_POP => {
                if c >= 0x80 || (name && at(b, ascii_word_end(b, p + 1)) >= 0x80) {
                    return None;
                }
                self.identifiers(p)
                    .or_else(|| self.can_pop(p).then(|| self.leave(p)))
            }
            GT_NAME => {
                // `identifiers`, then a `<` that sets the arguments
                if crate::is_space(c) {
                    return Some(self.miss(skip_spaces(b, p)));
                }
                if c == b'<' {
                    return Some(self.then(p, 1, &[ANGLE], GENERIC_BEGIN));
                }
                if c >= 0x80 || (name && at(b, ascii_word_end(b, p + 1)) >= 0x80) {
                    return None;
                }
                if name || c == b':' {
                    return self.identifiers(p);
                }
                None
            }
            DS_FN_ID => {
                if c >= 0x80 || (name && at(b, ascii_word_end(b, p + 1)) >= 0x80) {
                    return None;
                }
                self.identifiers(p).or_else(|| self.any_pop(p))
            }
            _ => None,
        }
    }

    /// `data-structures-maybe-method`'s path at `p`, an ASCII name past its words: a
    /// declarator's name (`(?=PATH([ \t]+|[*&])(?!\s*(<|::|\())`, the line's end one too), a
    /// method's name before its `(`, else a pop; `None` for a path the frame doesn't read or a
    /// template's arguments after it.
    #[inline(always)]
    fn ds_maybe_method(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let (e, _, _) = self.plain_path(p)?;
        let s = skip_spaces(b, e);
        if at(b, s) == b'<' && s < b.len() {
            return None;
        }
        let mut t = e;
        while t < b.len() && matches!(b[t], b' ' | b'\t') {
            t += 1;
        }
        let after = if t > e {
            Some(t)
        } else if matches!(at(b, e), b'*' | b'&') && e < b.len() {
            Some(e + 1)
        } else {
            None
        };
        if let Some(after) = after {
            let s = skip_spaces(b, after);
            if s >= b.len() || !(b[s] == b'<' || b[s] == b'(' || starts(b, s, b"::")) {
                return Some(self.enter(p, &[IDENT_POP]));
            }
        }
        if paren_after(b, e) {
            return Some(self.shift(p, &[METHOD_PARAMS, DS_FN_ID]));
        }
        self.any_pop(p)
    }

    /// C's top level's line start, `C_MAIN` on top at 0 (its `#if` rules and `global`): a
    /// declaration's head, which `^\s*(?=\w+)` hands to `global-modifier` (pushed at the line's
    /// first word, or entered at 0 without indentation); blanks before a byte no name starts,
    /// which the arm's miss passes. `None` for anything a pattern before it may take (a
    /// directive, an upper-case workaround's line, and without indentation a keyword's or
    /// value's word, a string's prefix), and for a byte at 0 no name starts, whose turn is the
    /// one past the line's start.
    pub(super) fn c_global_start(&mut self) -> Option<usize> {
        let b = self.b;
        if self.dir.is_some() {
            return None;
        }
        let q = self.lead;
        let Some(&c) = b.get(q) else {
            return Some(self.miss(q));
        };
        if !(c.is_ascii_alphabetic() || c == b'_') {
            return (q > 0 && CLASS[usize::from(c)] & (WORD | HIGH) == 0).then(|| self.miss(q));
        }
        let e = ascii_word_end(b, q + 1);
        if e < b.len() && b[e] >= 0x80 {
            return None;
        }
        if (c.is_ascii_uppercase() || c == b'_') && self.upper_line() {
            return None;
        }
        if q == 0 {
            // C's `early-expressions` try the word at 0 first
            let w = &b[..e];
            if matches!(w, b"case" | b"default" | b"typedef" | b"sizeof")
                || is_c_control(w)
                || c_value(w).is_some()
                || matches!(at(b, e), b'"' | b'\'')
            {
                return None;
            }
            return Some(self.enter(0, &[C_G_MODIFIER]));
        }
        self.push(q, C_G_MODIFIER);
        Some(q)
    }

    /// C's declaration frames' turn at `p` ([`C_G_MODIFIER`], [`C_G_TYPE`], [`C_G_MAYBE_FN`],
    /// [`C_FN_PARAMS`], [`C_FN_CONTINUE`]), as [`Scan::decl_turn`] has C++'s.
    #[inline(never)]
    pub(super) fn c_decl_turn(&mut self, p: usize, top: u8) -> Option<usize> {
        let b = self.b;
        if self.stuck >= STUCK {
            return None;
        }
        let Some(&c) = b.get(p) else {
            // the line's end: `(?=\s)` takes its `\n`; the others match nothing there
            return (top == C_G_TYPE).then(|| self.shift(p, &[C_G_MAYBE_FN]));
        };
        if c == b'/' || c >= 0x80 || c.is_ascii_digit() {
            return None;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            if self.inside_word(p) {
                return None;
            }
        } else if crate::is_space(c) {
            return match top {
                C_G_TYPE => Some(self.shift(p, &[C_G_MAYBE_FN])),
                C_FN_PARAMS => None,
                _ => Some(self.miss(skip_spaces(b, p))),
            };
        } else {
            return match (top, c) {
                (C_G_MODIFIER, _) => Some(self.shift(p, &[C_G_TYPE])),
                (C_G_TYPE | C_G_MAYBE_FN, b'*') if at(b, p + 1) == b'/' => None,
                (C_G_TYPE | C_G_MAYBE_FN, b'*') => Some(self.token(p, 1, OPERATOR | C)),
                (C_G_TYPE, _) => self.can_pop(p).then(|| self.leave(p)),
                (C_FN_PARAMS, b'(') => Some(self.then(p, 1, &[C_FN_PARAMS_IN], GROUP_BEGIN | C)),
                (C_FN_CONTINUE, b';') => self.can_pop(p).then(|| self.leave(p)),
                (C_FN_CONTINUE, b'{') => Some(self.then(p, 1, &[C_FN_BLOCK], BLOCK_BEGIN | C)),
                _ => self.any_pop(p),
            };
        }
        let e = ascii_word_end(b, p + 1);
        if e < b.len() && b[e] >= 0x80 {
            return None;
        }
        let w = &b[p..e];
        match top {
            C_G_MODIFIER => {
                if matches!(w, b"__attribute__" | b"__declspec") {
                    return None;
                }
                if is_c_modifier(w) {
                    return Some(self.token(p, e - p, MODIFIER | C));
                }
                Some(self.shift(p, &[C_G_TYPE]))
            }
            C_G_TYPE => {
                // `(struct|union|enum)\s+(?=…)` and the data structures: without a `\b` first
                if [&b"struct"[..], b"union", b"enum"]
                    .iter()
                    .any(|k| w.starts_with(k))
                {
                    return None;
                }
                if is_c_control(w) {
                    return None;
                }
                let s = skip_spaces(b, e);
                if at(b, s) == b'(' && s < b.len() {
                    if b[s + 1..]
                        .iter()
                        .position(|&x| x == b')')
                        .is_some_and(|k| k > 0)
                    {
                        self.put(p, e, FUNCTION | C);
                        self.put(s, s + 1, GROUP_BEGIN | C);
                        self.push(s + 1, C_PAREN);
                        return Some(s + 1);
                    }
                    return None;
                }
                if is_c_type(w) {
                    return Some(self.token(p, e - p, STORAGE_TYPE | C));
                }
                if let Some(leaf) = support(w) {
                    return Some(self.token(p, e - p, leaf));
                }
                self.fill(e);
                Some(e)
            }
            C_G_MAYBE_FN => {
                if is_c_type(w) {
                    return Some(self.token(p, e - p, STORAGE_TYPE | C));
                }
                if let Some(leaf) = support(w) {
                    return Some(self.token(p, e - p, leaf));
                }
                if matches!(w, b"__attribute__" | b"__declspec") {
                    return None;
                }
                if is_c_modifier(w) {
                    return Some(self.token(p, e - p, MODIFIER | C));
                }
                // `[\p{Lu}\p{Nd}_]+\s*$`: a macro's name ending the line
                let mut caps = p;
                while caps < b.len()
                    && (b[caps].is_ascii_uppercase() || b[caps].is_ascii_digit() || b[caps] == b'_')
                {
                    caps += 1;
                }
                if caps > p
                    && (caps >= b.len() || b[caps] < 0x80)
                    && skip_spaces(b, caps) >= b.len()
                {
                    let t = skip_spaces(b, caps);
                    self.fill(t);
                    return Some(t);
                }
                if paren_or_eol(b, e) {
                    return Some(self.then(p, e - p, &[C_FN_PARAMS], ENTITY_FUNCTION | C));
                }
                self.fill(e);
                Some(e)
            }
            C_FN_CONTINUE => {
                if matches!(w, b"const" | b"final" | b"noexcept" | b"override") {
                    return Some(self.token(p, e - p, MODIFIER | C));
                }
                self.any_pop(p)
            }
            _ => self.any_pop(p),
        }
    }

    /// A turn that matched nothing and passed to `q`, as the arm's miss and its advance do:
    /// [`Scan::stuck`] as it was ([`super::code::KEEP`]).
    #[inline(always)]
    pub(super) fn miss(&self, q: usize) -> usize {
        q | super::code::KEEP
    }
}
