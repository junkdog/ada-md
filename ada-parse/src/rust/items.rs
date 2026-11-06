//! Rust's frames above the code's and the types' (X-064): the headers (a module's, a trait's and
//! an enum's, an impl's, a struct's, the `where` clauses), a closure's parameters and return
//! type, `macro_rules!`, the format macros' first arguments, a generic's constant expressions;
//! then attributes, strings and block comments, each read by a loop of its own. All of them run
//! in [`Scan::code_run`]'s loop, so a line has one dispatcher: only the regexes and the SQL that
//! strings hold keep a turn loop of their own ([`Scan::run`]).
//!
//! The header frames' text is plain and none of them runs in an attribute, so they emit through
//! [`Scan::tok`] and push and pop with [`Scan::push_code`] and [`Scan::pop_code`]. An attribute's
//! frames, a string's and a comment's color their text (`meta.annotation`, the string's, the
//! comment's), so they fill it as they go ([`Scan::put`]). A pattern that matches nothing at a
//! place passes a word whole or one byte, as bat's contexts did.
//!
//! Where these differ from bat's grammar, by design: a struct's `where` clause, and a trait's or
//! an enum's generics, supertraits and `where` clause, read as a function's are (bat leaves them
//! plain); a `macro_rules!` matcher's groups balanced (bat's matcher ends at the first `)`, its
//! `$(…)` at the first `)` inside), and a matcher in `[…]` or `{…}` read as one in `(…)`.

use super::*;

/// A matcher's frame's closer, by its id less [`MATCHER`]: the matcher's own (`(…)`, `[…]`,
/// `{…}`), a repetition's `$(…)`, then a plain group's in it.
const CLOSERS: [u8; 7] = *b")]}))]}";
const _: () = assert!(NEST_CURLY - MATCHER == 6 && MATCHER_GROUP - MATCHER == 3);

/// Whether `top` is a matcher's frame, where the prototype doesn't reach.
#[inline(always)]
fn is_matcher(top: u8) -> bool {
    (MATCHER..=NEST_CURLY).contains(&top)
}

impl Scan<'_, '_> {
    /// [`Scan::tok_close`] on the emitter out of line.
    fn close_far(&mut self, p: usize, leaf: u16) -> usize {
        self.tok_far(p, p + 1, leaf);
        self.pop_code();
        p + 1
    }

    /// [`Scan::open_generic`] on the emitter out of line.
    fn generic_far(&mut self, p: usize) -> usize {
        self.push_code(GENERIC_ARGS);
        self.tok_far(p, p + 1, GENERIC_BEGIN)
    }

    /// A frame above the type frames, `top`, at `p`, not the line's end (whose `c` is): a turn
    /// of its, or (for an attribute, a string, a comment) of its loop. Where the scan goes on.
    #[inline(never)]
    pub(super) fn item_turn(&mut self, p: usize, c: u8, top: u8) -> usize {
        match top {
            STRING..=RAW_BYTES => return self.string_run(p, top),
            BLOCK_COMMENT | DOC_COMMENT => return self.comment_run(p, top),
            ATTRIBUTE..=ATTR_CALL => return self.attr_run(p),
            _ => {}
        }
        let b = self.b;
        // blanks are the frame's text, but where a frame moves on at once
        if is_space(c) && !matches!(top, IMPL_GENERIC | STRUCT_GENERIC | IMPL_FOR) {
            return skip_spaces(b, p);
        }
        // the prototype
        if c == b'$'
            && !is_matcher(top)
            && let Some(e) = ident_end(b, p + 1)
        {
            self.tok_far(p, e, VARIABLE);
            if top == STRUCT_ID {
                // a macro's `struct $name`: the name, else the header never ends
                self.struct_named(e);
            }
            return e;
        }
        // a closer no header holds: a header misread (`struct enum` in a macro's word list) ends
        // there, and the frame below reads it (bat's let it pass, and the block never closed)
        let ends = match c {
            b'}' => matches!(top, HEAD | MOD_HEAD | FN_WHERE..=IMPL_DEF | IMPL_TRAIT..=STRUCT_BODY),
            // where a type's group can't be open (`for [T]`, `F: Fn(A)` are)
            b')' | b']' => matches!(top, HEAD | MOD_HEAD | STRUCT_ID | STRUCT_BODY),
            _ => false,
        };
        if ends {
            self.pop_code();
            return p;
        }
        match self.item_frame(p, c, top) {
            Some(q) => q,
            // nothing matches: a word passed whole, else a byte
            None if is_word(c) => word_end(b, p),
            None => p + 1,
        }
    }

    /// The patterns of the header frame `top` at `p`, applied: where the scan goes on, or `None`
    /// when none matches.
    fn item_frame(&mut self, p: usize, c: u8, top: u8) -> Option<usize> {
        let b = self.b;
        match top {
            HEAD | MOD_HEAD => self.head(top, p, c),
            CLOSURE_PARAMS => self.closure_params(p, c),
            PARAM_TUPLE => match c {
                b')' => Some(self.close_far(p, META_GROUP_END)),
                b',' => Some(self.tok_far(p, p + 1, SEPARATOR)),
                _ => {
                    let e = ident_end(b, p)?;
                    Some(self.tok_far(p, e, PARAMETER))
                }
            },
            PARAM_TUPLE_TYPE => match c {
                b')' => Some(self.close_far(p, META_GROUP_END)),
                b',' => Some(self.tok_far(p, p + 1, SEPARATOR)),
                _ => self.tai(p, c),
            },
            CLOSURE_PARAM_TYPE => {
                if c == b',' || c == b'|' {
                    self.pop_code();
                    return Some(p);
                }
                self.tai(p, c)
            }
            CLOSURE_RETURN => match c {
                b'-' if b.get(p + 1) == Some(&b'>') => {
                    self.tok_far(p, p + 2, SEPARATOR);
                    self.push_code(RETURN_TYPE);
                    Some(p + 2)
                }
                b'{' => Some(self.open_block(p, true)),
                _ => {
                    self.set_top(CLOSURE_BODY);
                    Some(p)
                }
            },
            FN_WHERE | IMPL_WHERE | STRUCT_WHERE => {
                if let Some(e) = self.comment(p) {
                    return Some(e);
                }
                if c == b'{' {
                    if top == STRUCT_WHERE {
                        self.set_top(STRUCT_FIELDS);
                        return Some(self.tok_far(p, p + 1, META_BLOCK_BEGIN));
                    }
                    return Some(self.open_block(p, top == IMPL_WHERE));
                }
                if self.is_word_at(p, b"for") || self.is_word_at(p, b"where") {
                    return Some(self.tok_far(p, word_end(b, p), KEYWORD));
                }
                if let Some(e) = self.tai(p, c) {
                    return Some(e);
                }
                match c {
                    b':' => Some(self.tok_far(p, p + 1, SEPARATOR)),
                    b';' if top != IMPL_WHERE => {
                        self.pop_code();
                        Some(p)
                    }
                    _ => None,
                }
            }
            IMPL_DEF => {
                if let Some(e) = self.comment(p) {
                    return Some(e);
                }
                self.set_top(IMPL_FOR);
                if c == b'<' {
                    self.push_code(IMPL_GENERIC);
                }
                Some(p)
            }
            IMPL_GENERIC | STRUCT_GENERIC => {
                if c == b'<' {
                    return Some(self.generic_far(p));
                }
                if top == IMPL_GENERIC {
                    self.pop_code();
                } else {
                    self.set_top(STRUCT_BODY);
                }
                Some(p)
            }
            IMPL_FOR => {
                let next = if impl_for_ahead(b, p) {
                    IMPL_TRAIT
                } else {
                    IMPL_IDENT
                };
                self.set_top(next);
                Some(p)
            }
            IMPL_TRAIT => {
                if let Some(e) = self.comment(p) {
                    return Some(e);
                }
                if self.is_word_at(p, b"for") {
                    self.tok_far(p, p + 3, KEYWORD);
                    self.set_top(IMPL_IDENT);
                    return Some(p + 3);
                }
                self.tai(p, c)
            }
            IMPL_IDENT => {
                if let Some(e) = self.comment(p) {
                    return Some(e);
                }
                match c {
                    b'{' => Some(self.open_block(p, true)),
                    b'<' => Some(self.generic_far(p)),
                    b'&' => Some(self.tok_far(p, p + 1, OPERATOR)),
                    b'\'' => self.lifetime(p),
                    _ if self.is_word_at(p, b"where") => {
                        self.set_top(IMPL_WHERE);
                        Some(p)
                    }
                    _ => {
                        let e = ident_end(b, p)?;
                        Some(self.tok_far(p, e, IMPL_NAME))
                    }
                }
            }
            STRUCT_ID => {
                let e = ident_end(b, p)?;
                self.tok_far(p, e, STRUCT_NAME);
                self.struct_named(e);
                Some(e)
            }
            STRUCT_BODY => {
                if let Some(e) = self.comment(p) {
                    return Some(e);
                }
                match c {
                    b'(' => {
                        self.set_top(TUPLE_FIELDS);
                        Some(self.tok_far(p, p + 1, META_GROUP_BEGIN))
                    }
                    b'{' => {
                        self.set_top(STRUCT_FIELDS);
                        Some(self.tok_far(p, p + 1, META_BLOCK_BEGIN))
                    }
                    b';' => {
                        self.pop_code();
                        Some(p)
                    }
                    // a struct's own `where`, as a function's (bat's leaves it plain)
                    b'w' if self.is_word_at(p, b"where") => {
                        self.set_top(STRUCT_WHERE);
                        Some(p)
                    }
                    _ => None,
                }
            }
            GENERIC_LEN => {
                if c == b']' {
                    return Some(self.close_far(p, GROUP_END));
                }
                if let Some(e) = self.const_expr(p, c) {
                    return Some(e);
                }
                self.pop_code();
                Some(p)
            }
            CONST_GROUP => {
                if c == b')' {
                    return Some(self.close_far(p, GROUP_END));
                }
                self.const_expr(p, c)
            }
            GENERIC_EXTERN => {
                if let Some(e) = self.string(p, false) {
                    return Some(e);
                }
                self.pop_code();
                Some(p)
            }
            MACRO_BLOCK => {
                let next = match c {
                    b'{' => MACRO_RULES,
                    b'[' => MACRO_RULES_BRACKET,
                    b'(' => MACRO_RULES_PAREN,
                    _ => {
                        self.pop_code();
                        return Some(p);
                    }
                };
                self.set_top(next);
                let leaf = if c == b'{' {
                    META_BLOCK_BEGIN
                } else {
                    META_GROUP_BEGIN
                };
                Some(self.tok_far(p, p + 1, leaf))
            }
            MACRO_RULES | MACRO_RULES_BRACKET | MACRO_RULES_PAREN => {
                let close = match top {
                    MACRO_RULES => b'}',
                    MACRO_RULES_BRACKET => b']',
                    _ => b')',
                };
                if c == close {
                    let leaf = if c == b'}' { BLOCK_END } else { META_GROUP_END };
                    return Some(self.close_far(p, leaf));
                }
                if let Some(e) = self.comment(p) {
                    return Some(e);
                }
                // a rule's matcher, in any of the three (bat's, in `(…)` only)
                let matcher = match c {
                    b'(' => MATCHER,
                    b'[' => MATCHER_SQUARE,
                    b'{' => MATCHER_CURLY,
                    _ => return None,
                };
                for f in [MACRO_TERM, MACRO_BODY, MATCH_OP, matcher] {
                    self.push_code(f);
                }
                Some(self.tok_far(p, p + 1, GROUP_BEGIN))
            }
            MATCHER..=NEST_CURLY => self.matcher(p, c, top),
            MATCH_OP => {
                if c == b'=' && b.get(p + 1) == Some(&b'>') {
                    self.tok_far(p, p + 2, OPERATOR);
                    self.pop_code();
                    return Some(p + 2);
                }
                self.pop_code();
                Some(p)
            }
            MACRO_BODY => {
                let (frame, leaf) = match c {
                    b'{' => (BLOCK, BLOCK_BEGIN),
                    b'(' => (GROUP, GROUP_BEGIN),
                    b'[' => (MACRO_BRACKET, GROUP_BEGIN),
                    _ => {
                        self.pop_code();
                        return Some(p);
                    }
                };
                self.set_top(frame);
                Some(self.tok_far(p, p + 1, leaf))
            }
            MACRO_TERM => {
                if c == b';' || c == b',' {
                    return Some(self.tok_far(p, p + 1, MACRO_TERMINATOR));
                }
                self.pop_code();
                Some(p)
            }
            FMT_FIRST => {
                if let Some(e) = self.comment(p) {
                    return Some(e);
                }
                if c == b'"' {
                    self.push(p, FMT_STRING);
                    return Some(self.token(p, 1, STR_BEGIN));
                }
                if c == b'r' {
                    let h = b[p + 1..].iter().take_while(|&&c| c == b'#').count();
                    if b.get(p + 1 + h) == Some(&b'"') {
                        self.push(p, FMT_RAW);
                        self.hashes = h;
                        self.put(p, p + 1, RAW_PREFIX);
                        self.put(p + 1, p + 1 + h, RAW_BEGIN);
                        self.put(p + 1 + h, p + 2 + h, RAW);
                        return Some(p + 2 + h);
                    }
                }
                self.set_top(GROUP);
                Some(p)
            }
            // WRITE_FIRST
            _ => {
                if let Some(e) = self.comment(p) {
                    return Some(e);
                }
                (c == b',').then(|| {
                    self.set_top(FMT_FIRST);
                    p + 1
                })
            }
        }
    }

    /// A header's `statements-block` at `p`: comments and the body's braces; a trait's or an
    /// enum's generics, supertraits and `where` clause as a function's are.
    fn head(&mut self, top: u8, p: usize, c: u8) -> Option<usize> {
        if top == MOD_HEAD && c == b';' {
            self.tok_far(p, p + 1, TERMINATOR);
            self.set_top(AFTER_OP);
            return Some(p + 1);
        }
        if let Some(e) = self.comment(p) {
            return Some(e);
        }
        match c {
            b'{' => Some(self.open_block(p, true)),
            _ if top == MOD_HEAD => None,
            b'<' => Some(self.generic_far(p)),
            b':' => Some(self.tok_far(p, p + 1, SEPARATOR)),
            b'w' if self.is_word_at(p, b"where") => {
                self.set_top(IMPL_WHERE);
                Some(p)
            }
            _ => self.tai(p, c),
        }
    }

    /// A `macro_rules!` matcher's frame `top` at `p`: its closer, a repetition's `$(`, a
    /// metavariable, a plain group opened (so that its closer isn't the matcher's).
    fn matcher(&mut self, p: usize, c: u8, top: u8) -> Option<usize> {
        let b = self.b;
        let close = CLOSERS[usize::from(top - MATCHER)];
        if c == close {
            return Some(match top {
                MATCHER..=MATCHER_CURLY => self.close_far(p, GROUP_END),
                MATCHER_GROUP => {
                    self.tok_far(p, p + 1, GROUP_END);
                    // `(?:\s*[^*+]?\s*([*+]))?`
                    let j = skip_spaces(b, p + 1);
                    let rep = |k: usize| matches!(b.get(k), Some(b'*' | b'+'));
                    let op = if rep(j) {
                        Some(j)
                    } else if j < b.len() {
                        let k = skip_spaces(b, j + char_len(b[j]));
                        rep(k).then_some(k)
                    } else {
                        None
                    };
                    let end = match op {
                        Some(k) => self.tok_far(k, k + 1, OPERATOR),
                        None => p + 1,
                    };
                    self.pop_code();
                    end
                }
                // a plain group's closer, plain
                _ => {
                    self.pop_code();
                    p + 1
                }
            });
        }
        match c {
            b'$' if b.get(p + 1) == Some(&b'(') => {
                self.push_code(MATCHER_GROUP);
                self.tok_far(p, p + 1, REPEAT_BEGIN);
                Some(self.tok_far(p + 1, p + 2, GROUP_BEGIN))
            }
            b'$' => self.metavariable(p),
            // a literal string, so that a closer in it isn't the matcher's, and comments
            b'"' => self.string(p, false),
            b'/' => self.comment(p),
            b'(' | b'[' | b'{' => {
                let nest = match c {
                    b'(' => NEST_PAREN,
                    b'[' => NEST_SQUARE,
                    _ => NEST_CURLY,
                };
                self.push_code(nest);
                Some(p + 1)
            }
            _ => None,
        }
    }

    /// A closure's `|` at `p`, where a closure starts: its parameters' frame, then its return
    /// type's or body's.
    #[inline(always)]
    pub(super) fn open_closure(&mut self, p: usize) -> usize {
        self.push_code(CLOSURE_RETURN);
        self.push_code(CLOSURE_PARAMS);
        self.tok_far(p, p + 1, PARAMS_BEGIN)
    }

    /// A closure's parameters at `p`.
    fn closure_params(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        match c {
            b'}' | b';' | b')' | b']' => {
                self.pop_code();
                Some(p)
            }
            b',' => Some(self.tok_far(p, p + 1, SEPARATOR)),
            b'(' => {
                self.push_code(PARAM_TUPLE);
                Some(self.tok_far(p, p + 1, META_GROUP_BEGIN))
            }
            b':' => {
                let o = colon_paren(b, p)?;
                self.tok_far(p, p + 1, SEPARATOR);
                self.push_code(PARAM_TUPLE_TYPE);
                Some(self.tok_far(o, o + 1, META_GROUP_BEGIN))
            }
            b'|' => Some(self.close_far(p, PARAMS_END)),
            b'&' => Some(self.tok_far(p, p + 1, OPERATOR)),
            b'\'' => self.lifetime(p),
            _ if is_word(c) && starts_word(b, p) => {
                let e = word_end(b, p);
                let w = &b[p..e];
                if w == b"self" {
                    return Some(self.tok_far(p, e, PARAMETER));
                }
                if is_ident(w) {
                    // `name\s*(?:(:(?!:))|(?=\||,))`
                    let s = skip_spaces(b, e);
                    match b.get(s) {
                        Some(b':') if b.get(s + 1) != Some(&b':') => {
                            self.tok_far(p, e, PARAMETER);
                            self.tok_far(s, s + 1, SEPARATOR);
                            self.push_code(CLOSURE_PARAM_TYPE);
                            return Some(s + 1);
                        }
                        Some(b'|' | b',') => {
                            self.tok_far(p, e, PARAMETER);
                            self.push_code(CLOSURE_PARAM_TYPE);
                            return Some(s);
                        }
                        _ => {}
                    }
                }
                matches!(w, b"mut" | b"ref").then(|| self.tok_far(p, e, MODIFIER))
            }
            _ => None,
        }
    }

    /// `type-any-identifier` at `p` (whose byte is `c`) in a header: the type frames' patterns
    /// for a word at its start or for punctuation.
    fn tai(&mut self, p: usize, c: u8) -> Option<usize> {
        if is_word(c) {
            return if starts_word(self.b, p) {
                self.type_word(p)
            } else {
                None
            };
        }
        self.type_tok(p, c)
    }

    /// An attribute's frames from `p`, to the line's end or till the top is none of them (a
    /// string opened, the attribute closed). Its text is `meta.annotation`'s, filled as it goes.
    fn attr_run(&mut self, mut p: usize) -> usize {
        let b = self.b;
        loop {
            let top = self.top();
            if !(ATTRIBUTE..=ATTR_CALL).contains(&top) || p >= b.len() {
                return p;
            }
            let c = b[p];
            if is_space(c) {
                p = skip_spaces(b, p);
                continue;
            }
            // the prototype
            if c == b'$'
                && let Some(e) = ident_end(b, p + 1)
            {
                self.put(p, e, VARIABLE);
                p = e;
                continue;
            }
            let r = if top == ATTRIBUTE {
                match c {
                    b'=' => Some(self.token(p, 1, OPERATOR)),
                    b']' => Some(self.close(p, GROUP_END)),
                    _ => match ident_end(b, p) {
                        Some(e) => {
                            self.put(p, e, ANNOTATION);
                            if b.get(e) == Some(&b'(') {
                                self.push_code(ATTR_PARAMS);
                                Some(self.token(e, 1, ANNOTATION_BEGIN))
                            } else {
                                Some(e)
                            }
                        }
                        None => self.string(p, false),
                    },
                }
            } else {
                match c {
                    b')' => {
                        let leaf = if top == ATTR_PARAMS {
                            ANNOTATION_END
                        } else {
                            CALL_END
                        };
                        // the frame below is an attribute's too: the text stays its
                        self.put(p, p + 1, leaf);
                        self.pop_code();
                        Some(p + 1)
                    }
                    b',' => Some(self.token(p, 1, SEPARATOR)),
                    b'=' => Some(self.token(p, 1, OPERATOR)),
                    _ if is_word(c) => {
                        // the word read once: a call, a string's prefix, else plain to its end
                        let e = word_end(b, p);
                        let w = &b[p..e];
                        if is_ident(w) && b.get(e) == Some(&b'(') {
                            self.put(p, e, CALL);
                            self.push_code(ATTR_CALL);
                            Some(self.token(e, 1, CALL_BEGIN))
                        } else if matches!(w, b"b" | b"br" | b"r")
                            && let Some(q) = self.string(p, false)
                        {
                            Some(q)
                        } else {
                            Some(e)
                        }
                    }
                    _ => self.string(p, false),
                }
            };
            p = match r {
                Some(q) => q,
                None if is_word(c) => word_end(b, p),
                None => p + 1,
            };
        }
    }

    /// A string's text and patterns from `p` (`top` its frame), to the line's end or its
    /// close. Where the scan goes on.
    fn string_run(&mut self, mut p: usize, top: u8) -> usize {
        let b = self.b;
        let d = self.depth;
        while p < b.len() {
            let c = b[p];
            let r = match top {
                STRING | FMT_STRING | BYTE_STRING => match c {
                    b'"' => Some(self.close(p, STR_END)),
                    b'\\' => match escape(b, p, top != BYTE_STRING) {
                        Some(e) => Some(self.token(p, e - p, STR_ESCAPE)),
                        None => (top != BYTE_STRING && p + 1 == b.len())
                            .then(|| self.token(p, 1, STR_CONTINUATION)),
                    },
                    b'{' | b'}' if top == FMT_STRING => {
                        self.format_escape(p, c, STR_ESCAPE, STR_PLACEHOLDER)
                    }
                    _ => None,
                },
                _ => {
                    let h = self.hashes;
                    if c == b'"'
                        && b.len() > p + h
                        && b[p + 1..p + 1 + h].iter().all(|&c| c == b'#')
                    {
                        Some(self.close_n(p, 1 + h, RAW_END))
                    } else if top == FMT_RAW && (c == b'{' || c == b'}') {
                        self.format_escape(p, c, RAW_ESCAPE, RAW_PLACEHOLDER)
                    } else {
                        None
                    }
                }
            };
            match r {
                Some(q) => {
                    p = q;
                    if self.depth != d {
                        return p;
                    }
                }
                // its text, to the next byte a pattern starts on, sixteen bytes a step
                None => {
                    p = match top {
                        STRING | BYTE_STRING => prim::find_any(b, p + 1, *b"\"\\"),
                        FMT_STRING => prim::find_any(b, p + 1, *b"\"\\{}"),
                        RAW_STRING | RAW_BYTES => prim::find_any(b, p + 1, *b"\""),
                        _ => prim::find_any(b, p + 1, *b"\"{}"),
                    }
                }
            }
        }
        p
    }

    /// A block comment's text and patterns from `p` (`top` its frame), to the line's end, its
    /// close or a nested one's open. Where the scan goes on.
    fn comment_run(&mut self, mut p: usize, top: u8) -> usize {
        let b = self.b;
        let d = self.depth;
        while p < b.len() {
            let c = b[p];
            // the prototype
            if c == b'$'
                && let Some(e) = ident_end(b, p + 1)
            {
                let leaf = if top == DOC_COMMENT {
                    DOC_BLOCK + VAR
                } else {
                    BLOCK_TEXT + VAR
                };
                self.put(p, e, leaf);
                p = e;
                continue;
            }
            match self.in_comment(top, p, c) {
                Some(q) => {
                    p = q;
                    if self.depth != d {
                        return p;
                    }
                }
                None => p = prim::find_any(b, p + 1, *b"*/$"),
            }
        }
        p
    }
}
