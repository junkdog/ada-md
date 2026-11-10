//! The loop ([`Scan::code`]): a blank run passed whole, a name read once and looked up once
//! ([`Scan::word`]), punctuation one `match` ([`Scan::punct`]); numbers, strings, characters and
//! comments each a reader of its own. `PP` is a `#define`'s body ([`super::pp`]): its plain
//! text is [`PP_TEXT`], filled before each token, and its braces open no level.

use super::pp::skip_blanks;
use super::words::{
    A_ACCESS, A_C99, A_CASE, A_CAST, A_CLASS, A_DELETE, A_ENUM, A_LEAF, A_NAMESPACE, A_NS,
    A_OPERATOR, A_REGEX, A_STRUCT, A_TEMPLATE, A_TYPEDEF, A_TYPENAME, A_UNION, A_USING, ACT, WORDS,
};
use super::*;
use crate::prim;

impl<K: Sink> Scan<'_, '_, K> {
    /// The code from `p` to the line's end. At a namespace's or a class's level, as bat has
    /// it, a line's end ends a statement without a keyword that a macro's call ends
    /// (`DECLARE(x)`) or that has names (`TEST_END`), but not a path's `::`; and a definition's
    /// header with no `;` or `{` ([`P_DEFINED`]: a `:` next is its initializer list's).
    #[inline(always)]
    pub(super) fn code_line(&mut self, p: usize) {
        self.code::<false>(p);
        if self.paren == 0 && self.level() <= CLASS {
            self.line_end();
        }
    }

    #[inline(never)]
    fn line_end(&mut self) {
        let ends = match self.decl {
            // a path's `::` at the line's end goes on (`PB_DS_CLASS_C_DEC::`, the name next)
            // after a keyword (`static void __attribute__((destructor))`) the name is next
            D_NONE => {
                matches!(self.prev, P_CLOSE | P_OTHER)
                    && !self.kw
                    && !self.b.trim_ascii_end().ends_with(b"::")
            }
            D_DEFINED => {
                (self.prev, self.decl, self.kw) = (P_DEFINED, D_NONE, false);
                return;
            }
            _ => false,
        };
        if ends {
            (self.prev, self.decl, self.kw) = (P_STMT, D_NONE, false);
        }
    }

    /// The code from `p` to the line's end.
    #[inline(never)]
    pub(super) fn code<const PP: bool>(&mut self, mut p: usize) {
        let b = self.b;
        while p < b.len() {
            let c = b[p];
            let k = CC[usize::from(c)];
            if k & SP != 0 {
                p += 1;
                while p < b.len() && CC[usize::from(b[p])] & SP != 0 {
                    p += 1;
                }
                continue;
            }
            p = if k & NS0 != 0 {
                self.word::<PP>(p)
            } else {
                self.punct::<PP>(p, c)
            };
        }
    }

    /// The name at `p`, whose first byte is a name's: a keyword by its leaf, else what the token
    /// before and the byte after make it.
    #[inline(always)]
    fn word<const PP: bool>(&mut self, p: usize) -> usize {
        let b = self.b;
        let mut e = p + 1;
        let mut bits = CC[usize::from(b[p])];
        while e < b.len() {
            let k = CC[usize::from(b[e])];
            if k & NW == 0 {
                break;
            }
            bits |= k;
            e += 1;
        }
        let id = if bits & HI == 0 {
            WORDS.get(&b[p..e])
        } else {
            0
        };
        // SAFETY: an id is at most the table's words, the row's length less one
        let info = unsafe { *self.info.get_unchecked(usize::from(id)) };
        if self.prev == P_ENTITY || self.decl >= D_TYPEDEF {
            return self.word_in::<PP>(p, e, info);
        }
        self.word_on::<PP>(p, e, info)
    }

    /// The word `p..e`, `info` what it is ([`words::INFO`]), where no declaration's state rules
    /// it.
    #[inline(always)]
    fn word_on<const PP: bool>(&mut self, p: usize, e: usize, info: u16) -> usize {
        if info == 0 {
            return self.name::<PP>(p, e, false);
        }
        let leaf = info & ((1 << ACT) - 1);
        match info >> ACT {
            // a member named as a support type (`.Size`)
            A_LEAF if self.prev == P_ACCESSOR && (SYS_TYPE..=MAC_TYPE).contains(&leaf) => {
                return self.name::<PP>(p, e, false);
            }
            A_LEAF => {}
            // a regex engine's name: a plain name, its line's strings checked for a pattern
            A_C99 | A_REGEX => {
                let c99 = info >> ACT == A_C99;
                self.rx |= !c99;
                return self.name::<PP>(p, e, c99);
            }
            // `= default;`, `= delete;` after a definition's parameters
            A_CASE | A_DELETE if self.decl == D_DEFINED => {
                self.put::<PP>(p, e, MODIFIER);
                self.prev = P_OTHER;
                return e;
            }
            A_CASE => self.case = true,
            // `delete[]` and `delete []` one keyword, as bat has it
            A_DELETE => {
                let q = skip_blanks(self.b, e);
                if self.b.get(q..q + 2) == Some(b"[]") {
                    self.put::<PP>(p, q + 2, leaf);
                    (self.prev, self.kw) = (P_OTHER, self.kw | (self.paren == 0));
                    return q + 2;
                }
            }
            A_ACCESS => {
                self.put::<PP>(p, e, leaf);
                self.prev = P_ACCESS;
                return e;
            }
            // a template's parameters' `typename T`, `class U`: a type's word, no class
            A_TYPENAME | A_CLASS if self.in_tpl() => {
                self.put::<PP>(p, e, STORAGE_TYPE);
                (self.prev, self.kw) = (P_OTHER, true);
                return e;
            }
            A_TEMPLATE => {
                self.put::<PP>(p, e, leaf);
                self.kw |= self.paren == 0;
                // `a.template get<0>()` keeps the member
                if self.prev != P_ACCESSOR {
                    (self.prev, self.entity) = (P_ENTITY, E_TEMPLATE);
                }
                return e;
            }
            A_CAST => {
                self.put::<PP>(p, e, leaf);
                (self.prev, self.entity) = (P_ENTITY, E_CAST);
                return e;
            }
            A_CLASS | A_STRUCT | A_UNION | A_ENUM => {
                let act = info >> ACT;
                // `enum class` stays an enum's
                self.pending = if act == A_ENUM { PEND_INIT } else { PEND_CLASS };
                self.put::<PP>(p, e, leaf);
                self.kw |= self.paren == 0;
                // a `typedef`'s struct leaves its name plain, as bat does, and a friend's names
                // nothing (`friend class Other;`)
                if self.decl == D_TYPEDEF || act != A_ENUM && after_friend(self.b, p) {
                    self.prev = P_OTHER;
                } else {
                    self.prev = P_ENTITY;
                    self.entity = match act {
                        A_CLASS => E_CLASS,
                        A_STRUCT => E_STRUCT,
                        A_UNION => E_UNION,
                        _ => E_ENUM,
                    };
                }
                return e;
            }
            // C++'s `extern "C" {` a namespace's level; C's a block of code, as bat has it
            A_NS if !self.c => self.pending = PEND_NS,
            A_NAMESPACE => {
                self.pending = PEND_NS;
                self.put::<PP>(p, e, leaf);
                (self.prev, self.entity) = (P_ENTITY, E_NAMESPACE);
                return e;
            }
            A_USING => {
                self.put::<PP>(p, e, leaf);
                (self.prev, self.entity) = (P_ENTITY, E_USING);
                return e;
            }
            A_TYPEDEF => {
                self.put::<PP>(p, e, leaf);
                self.prev = P_OTHER;
                (self.decl, self.td) = (D_TYPEDEF, self.depth.saturating_add(1));
                return e;
            }
            A_OPERATOR if !PP && self.may_define() => return self.operator(p, e, leaf),
            _ => {}
        }
        self.put::<PP>(p, e, leaf);
        self.kw |= self.paren == 0;
        // a modifier alone is no type: `const FOO(x) *f(` calls `FOO`, as bat has it
        if leaf != MODIFIER || self.prev != P_STMT {
            self.prev = P_OTHER;
        }
        e
    }

    /// The word `p..e` where a declaration's state rules it: after a word that names what
    /// follows ([`P_ENTITY`]), in a definition's parameter list, in a `typedef`.
    #[inline(never)]
    fn word_in<const PP: bool>(&mut self, p: usize, e: usize, info: u16) -> usize {
        // a regex engine's name is a plain name here (`class RE2 {`, `namespace ctre {`)
        let info = if info >> ACT == A_REGEX {
            self.rx = true;
            0
        } else {
            info
        };
        let b = self.b;
        let q = skip_blanks(b, e);
        let next = next_byte(b, q);
        if self.prev == P_ENTITY {
            return self.entity_word::<PP>(p, e, info, q, next);
        }
        if self.decl == D_PARAMS {
            // the word before a `,`, `)`, `[` or `=` is the parameter, a type's word too, but
            // `void`
            if self.angle == 0
                && self.paren == 1
                && matches!(next, b',' | b')' | b'[' | b'=')
                && (info == 0 || &b[p..e] != b"void")
            {
                self.put::<PP>(p, e, PARAMETER);
                self.prev = P_OTHER;
                return e;
            }
        } else if next == b';' && self.paren == 0 && info == 0 {
            // a `typedef`'s name
            self.put::<PP>(p, e, TYPEDEF_NAME);
            self.prev = P_OTHER;
            return e;
        }
        self.word_on::<PP>(p, e, info)
    }

    /// The word `p..e` after one that names what follows ([`Scan::entity`]), `next` the byte at
    /// `q`, after its blanks: a namespace's name (and its path), an alias's before `=`, a
    /// class's, a struct's, a union's or an enum's before `{`, `:`, `;`, `<`, `final` or the
    /// line's end (a macro in upper case before the name passed over), `operator`'s word before
    /// `(`.
    fn entity_word<const PP: bool>(
        &mut self,
        p: usize,
        e: usize,
        info: u16,
        q: usize,
        next: u8,
    ) -> usize {
        let b = self.b;
        let kind = self.entity;
        let leaf = ENTITY_LEAF[usize::from(kind)];
        match kind {
            E_NAMESPACE if info == 0 => {
                // the prev stays, for a path's `::` and its next name
                self.put::<PP>(p, e, leaf);
                return e;
            }
            E_USING if info != 0 => {
                // `using namespace std;`: no namespace's name
                self.put::<PP>(p, e, info & ((1 << ACT) - 1));
                self.prev = P_OTHER;
                return e;
            }
            E_USING if next == b'=' && b.get(q + 1) != Some(&b'=') => {
                self.put::<PP>(p, e, leaf);
                self.prev = P_OTHER;
                return e;
            }
            E_OPERATOR if next == b'(' => {
                self.put::<PP>(p, e, leaf);
                self.prev = P_OTHER;
                (self.decl, self.angle) = (D_PARAMS, 0);
                return e;
            }
            E_CLASS..=E_ENUM if info == 0 => {
                let ends = match next {
                    b'{' | b';' | b'<' | b'\n' => true,
                    b':' => b.get(q + 1) != Some(&b':'),
                    b'/' => matches!(b.get(q + 1), Some(b'/' | b'*')),
                    _ => word_at(b, q, b"final"),
                };
                if ends {
                    self.put::<PP>(p, e, leaf);
                    self.prev = P_OTHER;
                    return e;
                }
                if CC[usize::from(next)] & NS0 != 0 && upper(&b[p..e]) {
                    // `class EXPORT Foo`: the macro plain, the name next
                    return e;
                }
            }
            E_ENUM if info >> ACT == A_CLASS => {
                // `enum class`, `enum struct`: one token with `enum`, its blanks too (bat's)
                let s = b[..p].trim_ascii_end().len();
                self.put::<PP>(s, e, info & ((1 << ACT) - 1));
                return e;
            }
            _ => {}
        }
        self.prev = P_OTHER;
        self.word_on::<PP>(p, e, info)
    }

    /// Whether the innermost open `<` is a template's parameters'.
    #[inline(always)]
    fn in_tpl(&self) -> bool {
        self.angle > 0
            && u32::from(self.tpl)
                .checked_shr(u32::from(self.angle) - 1)
                .is_some_and(|t| t & 1 != 0)
    }

    /// Whether a name before `(` here could be a definition's: a namespace's or a class's level,
    /// outside parentheses and template arguments, before any `=`.
    #[inline(always)]
    fn may_define(&self) -> bool {
        self.decl == D_NONE && self.paren == 0 && self.angle == 0 && self.level() <= CLASS
    }

    /// `operator` at `p..e` (`leaf` its keyword's) where a definition's name may be: with its
    /// symbol before `(` a function's name (`operator==`, `operator()`, `operator[]`); before a
    /// word, that word's ([`E_OPERATOR`]).
    #[cold]
    #[inline(never)]
    fn operator(&mut self, p: usize, e: usize, leaf: u16) -> usize {
        let b = self.b;
        let q = skip_blanks(b, e);
        let next = next_byte(b, q);
        let mut s = q;
        match (next, b.get(q + 1)) {
            _ if CC[usize::from(next)] & NS0 != 0 => {
                self.put::<false>(p, e, leaf);
                (self.prev, self.entity) = (P_ENTITY, E_OPERATOR);
                return e;
            }
            (b'(', Some(b')')) | (b'[', Some(b']')) => s += 2,
            _ => {
                while s < b.len() && b"+-*/%^&|~!=<>,".contains(&b[s]) {
                    s += 1;
                }
            }
        }
        self.prev = P_OTHER;
        if s > q && next_byte(b, skip_blanks(b, s)) == b'(' {
            self.put::<false>(p, s, FUNC_DEF);
            (self.decl, self.angle) = (D_PARAMS, 0);
            return s;
        }
        self.put::<false>(p, e, leaf);
        e
    }

    /// A name that isn't a keyword, `p..e` (`c99` one of C99's functions): a call before `(`, a
    /// member after `.` or `->`, a string's prefix before its quote, a label at a statement's
    /// start, else by its shape (`mFoo`, `kFoo`) or plain.
    #[inline(always)]
    fn name<const PP: bool>(&mut self, p: usize, e: usize, c99: bool) -> usize {
        let b = self.b;
        let prev = self.prev;
        self.prev = P_OTHER;
        let leaf = match b.get(e) {
            Some(b'(') => self.call_leaf::<PP>(p, e, prev, c99),
            Some(b'"' | b'\'') => return self.prefixed::<PP>(p, e),
            Some(b'<') if !self.c => return self.name_lt::<PP>(p, e, e, prev),
            // C++'s `name{…}`: a call's color, as bat has it (not a base class's before a body)
            Some(b'{') if !self.c && self.pending == NO_PENDING => FUNCTION,
            Some(b':') if !PP && prev == P_STMT && b.get(e + 1) != Some(&b':') => {
                self.put::<PP>(p, e, LABEL);
                self.put::<PP>(e, e + 1, SEPARATOR);
                self.prev = P_STMT;
                return e + 1;
            }
            Some(&c) if CC[usize::from(c)] & SP != 0 => {
                let mut q = e + 1;
                while q < b.len() && CC[usize::from(b[q])] & SP != 0 {
                    q += 1;
                }
                if b.get(q) == Some(&b'(') {
                    self.call_leaf::<PP>(p, e, prev, c99)
                } else if b.get(q) == Some(&b'<') && !self.c {
                    return self.name_lt::<PP>(p, e, q, prev);
                } else if b.get(q) == Some(&b'{') && !self.c && self.pending == NO_PENDING {
                    FUNCTION
                } else {
                    self.value_leaf(p, e, prev)
                }
            }
            _ => self.value_leaf(p, e, prev),
        };
        if leaf != PLAIN {
            self.put::<PP>(p, e, leaf);
        }
        e
    }

    /// A name `p..e` before `(`: a definition's (or a declaration's) name where a statement at a
    /// namespace's or a class's level has a type before it (`int f(`, `Foo::Foo(`), or at a
    /// class's member's start a constructor's (a name in upper case is a macro's call); else a
    /// call, a member's after an accessor, C99's for its functions.
    #[inline(always)]
    fn call_leaf<const PP: bool>(&mut self, p: usize, e: usize, prev: u8, c99: bool) -> u16 {
        if !PP
            && self.may_define()
            && (prev == P_OTHER
                || prev == P_CLOSE
                || prev == P_STMT && self.level() == CLASS && !(e - p > 1 && upper(&self.b[p..e])))
        {
            (self.decl, self.angle) = (D_PARAMS, 0);
            return FUNC_DEF;
        }
        if prev == P_ACCESSOR {
            MEMBER_FUNCTION
        } else if self.angle > 0 {
            // no call in template arguments (`UniquePtr<STACK_OF(X509)>`), as bat has it
            PLAIN
        } else if c99 {
            C99
        } else {
            FUNCTION
        }
    }

    /// A name `p..e` before `<` at `q` (C++): template arguments where a generic's shape
    /// follows ([`generic_end`]), and in a declaration (a namespace's or a class's level, before
    /// any `=`; a definition's parameters) or in template arguments any `<` but `<<` and `<=`.
    /// Before `(` or `{` after them a call in code, a member's after an accessor, a
    /// definition's name where one may be; else the name by its shape.
    #[cold]
    #[inline(never)]
    fn name_lt<const PP: bool>(&mut self, p: usize, e: usize, q: usize, prev: u8) -> usize {
        let b = self.b;
        let end = generic_end(b, q);
        if end.is_none() {
            let lenient = !matches!(b.get(q + 1), Some(b'<' | b'='))
                && (self.angle > 0
                    || !PP
                        && self.level() <= CLASS
                        && (self.decl == D_NONE && self.paren == 0 || self.decl == D_PARAMS));
            if !lenient {
                let leaf = self.value_leaf(p, e, prev);
                if leaf != PLAIN {
                    self.put::<PP>(p, e, leaf);
                }
                return e;
            }
        }
        let open = end.map_or(b.len(), |x| skip_blanks(b, x + 1));
        let after = next_byte(b, open);
        let leaf = if matches!(after, b'(' | b'{') {
            // not a function pointer's type: `UniquePtr<X509> (*certificate)();`
            if after == b'('
                && !PP
                && self.may_define()
                && matches!(prev, P_OTHER | P_CLOSE)
                && next_byte(b, skip_blanks(b, open + 1)) != b'*'
            {
                self.decl = D_PARAMS;
                FUNC_DEF
            } else if prev == P_ACCESSOR {
                MEMBER_FUNCTION
            } else if self.angle == 0 && self.level() > CLASS {
                FUNCTION
            } else {
                PLAIN
            }
        } else {
            self.value_leaf(p, e, prev)
        };
        if leaf != PLAIN {
            self.put::<PP>(p, e, leaf);
        }
        self.generic_open::<PP>(q, false)
    }

    /// A generic's `<` at `p` (a template's parameters' where `tpl`).
    #[inline(always)]
    fn generic_open<const PP: bool>(&mut self, p: usize, tpl: bool) -> usize {
        if tpl && self.angle < 3 {
            self.tpl |= 1 << self.angle;
        }
        self.angle = self.angle.saturating_add(1);
        self.put::<PP>(p, p + 1, GENERIC_BEGIN);
        self.prev = P_OTHER;
        p + 1
    }

    /// A generic's `>` at `p`.
    #[inline(always)]
    fn generic_close<const PP: bool>(&mut self, p: usize) -> usize {
        self.angle -= 1;
        if self.angle < 3 {
            self.tpl &= !(1 << self.angle);
        }
        self.put::<PP>(p, p + 1, GENERIC_END);
        self.prev = P_OTHER;
        p + 1
    }

    /// A name `p..e` that isn't called: a member after an accessor, else by its shape.
    #[inline(always)]
    fn value_leaf(&self, p: usize, e: usize, prev: u8) -> u16 {
        if prev == P_ACCESSOR {
            return MEMBER_VAR;
        }
        if prev == P_ARROW {
            return PLAIN;
        }
        let b = self.b;
        let first = b[p];
        if !matches!(first, b'f' | b'm' | b'g' | b's' | b'k') || e - p < 2 {
            return PLAIN;
        }
        let second = b[p + 1];
        let leaf = if CC[usize::from(second)] & UP != 0 {
            match first {
                b'f' | b'm' if !self.c => MEMBER_VAR,
                b'g' => GLOBAL_MAC,
                b's' => STATIC_MAC,
                b'k' => MAC_CONSTANT,
                _ => PLAIN,
            }
        } else if first == b'm' && second == b'_' && e - p > 2 && !self.c {
            MEMBER_VAR
        } else {
            PLAIN
        };
        if leaf != PLAIN && prev != P_STMT && self.declared(e) {
            return PLAIN;
        }
        leaf
    }

    /// Whether the name ending at `e` is the one a declaration at a namespace's or a class's
    /// level declares, as bat reads it: after a type, before blanks (or a `*`, a `&`) and
    /// something but `<`, `::`, `(` or the line's end (`const int kMax = 3;`): plain, not by its
    /// shape; in C before `[` too.
    #[cold]
    fn declared(&self, e: usize) -> bool {
        let b = self.b;
        if !(self.decl == D_NONE && self.paren == 0 && self.angle == 0 && self.level() <= CLASS) {
            return false;
        }
        let q = match b.get(e) {
            Some(&c) if CC[usize::from(c)] & SP != 0 => skip_blanks(b, e),
            Some(b'*' | b'&') => skip_blanks(b, e + 1),
            // C's array: `static const uint8_t kData[] = {…};` (bat's C++ colors it)
            Some(b'[') => return self.c,
            _ => return false,
        };
        match b.get(q) {
            None | Some(b'<' | b'(') => false,
            Some(b':') => b.get(q + 1) != Some(&b':'),
            _ => true,
        }
    }

    /// A name `p..e` before a quote: a string's or a character's prefix (`L`, `u`, `U`, `u8`,
    /// C++'s raw strings' `R`, `LR`…), else plain.
    #[cold]
    #[inline(never)]
    fn prefixed<const PP: bool>(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let w = &b[p..e];
        let plain = matches!(w, b"L" | b"u" | b"U" | b"u8");
        if b[e] == b'"' {
            if plain {
                self.put::<PP>(p, e, DQ_PREFIX);
                if let Some(d) = self.regex_here::<PP>(p) {
                    self.put::<PP>(e, e + 1, DQ_BEGIN);
                    return self.dq_regex(e + 1, d);
                }
                return self.dq::<PP>(e);
            }
            if !self.c
                && matches!(w, b"R" | b"LR" | b"uR" | b"UR" | b"u8R")
                && let Some(q) = self.raw::<PP>(p, e)
            {
                return q;
            }
        } else if plain {
            self.put::<PP>(p, e, SQ_PREFIX);
            return self.sq::<PP>(e);
        }
        let leaf = self.value_leaf(p, e, P_OTHER);
        if leaf != PLAIN {
            self.put::<PP>(p, e, leaf);
        }
        e
    }

    /// The token at `p` that isn't a blank nor a name: a number, a string, a comment, an
    /// operator, a bracket.
    #[inline(always)]
    fn punct<const PP: bool>(&mut self, p: usize, c: u8) -> usize {
        let b = self.b;
        let next = b.get(p + 1).copied().unwrap_or(0);
        let (n, leaf) = match c {
            b'0'..=b'9' => {
                self.prev = P_OTHER;
                return self.number::<PP>(p);
            }
            b'"' => {
                self.prev = P_OTHER;
                // a line with a regex engine's name: its strings checked for a pattern, out of
                // the loop's way
                if !PP && self.rx {
                    return self.dq_checked(p);
                }
                return self.dq::<PP>(p);
            }
            b'\'' => {
                self.prev = P_OTHER;
                return self.sq::<PP>(p);
            }
            b'/' => match next {
                b'/' => return self.line_comment::<PP>(p),
                b'*' => return self.block_comment::<PP>(p),
                b'=' => (2, ASSIGN_AUG),
                _ => (1, ARITH),
            },
            b'(' => {
                self.pending = NO_PENDING;
                self.paren = self.paren.saturating_add(1);
                (1, GROUP_BEGIN)
            }
            b')' => {
                self.put::<PP>(p, p + 1, GROUP_END);
                // a `struct` in parentheses names no `{`'s level
                (self.prev, self.pending) = (P_CLOSE, NO_PENDING);
                self.paren = self.paren.saturating_sub(1);
                if self.paren == 0 && matches!(self.decl, D_PARAMS | D_DEFAULT) {
                    self.decl = D_DEFINED;
                }
                return p + 1;
            }
            b';' => {
                self.put::<PP>(p, p + 1, TERMINATOR);
                (self.prev, self.pending, self.ternary, self.case) = (P_STMT, NO_PENDING, 0, false);
                if self.td > self.depth {
                    self.td = 0;
                }
                self.new_statement();
                return p + 1;
            }
            b',' => {
                if self.decl == D_DEFAULT && self.paren == 1 {
                    self.decl = D_PARAMS;
                }
                (1, SEPARATOR)
            }
            b'{' => {
                if !PP {
                    self.open_brace();
                }
                self.put::<PP>(p, p + 1, BLOCK_BEGIN);
                (self.prev, self.ternary) = (P_STMT, 0);
                self.new_statement();
                return p + 1;
            }
            b'}' => {
                if !PP {
                    if p == 0 && self.level() >= CODE {
                        self.resync();
                    } else {
                        self.close_brace();
                    }
                }
                self.put::<PP>(p, p + 1, BLOCK_END);
                (self.prev, self.ternary) = (P_STMT, 0);
                self.new_statement();
                return p + 1;
            }
            b'[' => (1, BRACKETS_BEGIN),
            b']' => (1, BRACKETS_END),
            b'.' => match next {
                b'0'..=b'9' => {
                    self.prev = P_OTHER;
                    return self.number::<PP>(p);
                }
                b'.' if b.get(p + 2) == Some(&b'.') => (3, VARIADIC),
                b'*' => (2, OPERATOR),
                _ => {
                    self.put::<PP>(p, p + 1, DOT);
                    self.prev = P_ACCESSOR;
                    return p + 1;
                }
            },
            b'-' => match next {
                // a trailing return type's, after a definition's parameters (bat's)
                b'>' if self.decl == D_DEFINED => (2, SEPARATOR),
                b'>' => {
                    let n = if b.get(p + 2) == Some(&b'*') { 3 } else { 2 };
                    self.put::<PP>(p, p + n, ARROW);
                    self.prev = if self.c { P_ARROW } else { P_ACCESSOR };
                    return p + n;
                }
                b'-' => (2, ARITH),
                b'=' => (2, ASSIGN_AUG),
                _ => (1, ARITH),
            },
            b'+' => match next {
                b'+' => (2, ARITH),
                b'=' => (2, ASSIGN_AUG),
                _ => (1, ARITH),
            },
            b'*' | b'%' | b'^' => match next {
                b'=' => (2, ASSIGN_AUG),
                _ if c == b'*' => (1, OPERATOR),
                _ => (1, ARITH),
            },
            b'&' | b'|' => match next {
                b'=' => (2, ASSIGN_AUG),
                _ if next == c => (2, OPERATOR),
                _ => (1, OPERATOR),
            },
            b'=' => match next {
                b'=' => (2, COMPARISON),
                _ => {
                    self.pending = NO_PENDING;
                    match self.decl {
                        D_NONE if self.paren == 0 && self.angle == 0 => self.decl = D_DONE,
                        D_PARAMS if self.angle == 0 => self.decl = D_DEFAULT,
                        _ => {}
                    }
                    (1, ASSIGN)
                }
            },
            b'!' => match next {
                b'=' => (2, COMPARISON),
                _ => (1, OPERATOR),
            },
            b'<' | b'>' => {
                if !self.c {
                    if c == b'>' && self.angle > 0 {
                        return self.generic_close::<PP>(p);
                    }
                    // `template <`, `static_cast<`
                    if c == b'<'
                        && self.prev == P_ENTITY
                        && matches!(self.entity, E_TEMPLATE | E_CAST)
                        && next != b'<'
                    {
                        return self.generic_open::<PP>(p, self.entity == E_TEMPLATE);
                    }
                }
                if next == c {
                    if b.get(p + 2) == Some(&b'=') {
                        (3, ASSIGN_AUG)
                    } else {
                        (2, ARITH)
                    }
                } else if next == b'=' {
                    if c == b'<' && b.get(p + 2) == Some(&b'>') {
                        (3, COMPARISON)
                    } else {
                        (2, COMPARISON)
                    }
                } else {
                    (1, COMPARISON)
                }
            }
            b'~' => {
                if !PP && self.may_define() {
                    return self.tilde::<PP>(p);
                }
                (1, OPERATOR)
            }
            b'?' => {
                self.ternary = self.ternary.saturating_add(1);
                (1, TERNARY)
            }
            b':' => return self.colon::<PP>(p, next),
            b'\\' if p + 1 == b.len() => (1, CONT),
            _ => {
                // a stray byte (`#`, `@`, a control character) is plain
                self.prev = P_OTHER;
                return p + 1;
            }
        };
        self.put::<PP>(p, p + n, leaf);
        self.prev = P_OTHER;
        p + n
    }

    /// A `~` at `p` where a definition's name may be: before a name and `(` a destructor's name,
    /// `~` and all; else the operator.
    #[cold]
    #[inline(never)]
    fn tilde<const PP: bool>(&mut self, p: usize) -> usize {
        let b = self.b;
        let mut e = p + 1;
        if e < b.len() && CC[usize::from(b[e])] & NS0 != 0 {
            while e < b.len() && CC[usize::from(b[e])] & NW != 0 {
                e += 1;
            }
            if next_byte(b, skip_blanks(b, e)) == b'(' {
                self.put::<PP>(p, e, FUNC_DEF);
                self.prev = P_OTHER;
                (self.decl, self.angle) = (D_PARAMS, 0);
                return e;
            }
        }
        self.put::<PP>(p, p + 1, OPERATOR);
        self.prev = P_OTHER;
        p + 1
    }

    /// A `:` at `p`, or `::`: by the token before, a ternary's, a `case`'s, an access label's,
    /// an initializer list's, a base list's; else as bat has a bit-field's or a range `for`'s,
    /// the ternary's operator.
    #[inline(always)]
    fn colon<const PP: bool>(&mut self, p: usize, next: u8) -> usize {
        if next == b':' {
            if self.prev == P_ENTITY && self.entity == E_NAMESPACE {
                // a namespace's path: `namespace a::b`
                self.put::<PP>(p, p + 2, NAMESPACE_SEP);
                return p + 2;
            }
            self.put::<PP>(p, p + 2, DOUBLE_COLON);
            self.prev = P_OTHER;
            return p + 2;
        }
        if self.decl <= D_DEFINED {
            // an initializer list's, a base list's, a bit-field's: no definition after it
            self.decl = D_DONE;
        }
        let (leaf, prev) = if self.ternary > 0 {
            self.ternary -= 1;
            (TERNARY, P_OTHER)
        } else if self.case {
            self.case = false;
            (SEPARATOR, P_STMT)
        } else {
            match self.prev {
                P_ACCESS => {
                    self.new_statement();
                    (CLASS_SECTION, P_STMT)
                }
                // C has no initializer lists: bat's C has an `asm`'s `:` the ternary's
                P_CLOSE | P_DEFINED if !self.c => (INIT_LIST, P_OTHER),
                _ if self.pending == PEND_CLASS || self.pending == PEND_INIT => {
                    (SEPARATOR, P_OTHER)
                }
                _ => (TERNARY, P_OTHER),
            }
        };
        self.put::<PP>(p, p + 1, leaf);
        self.prev = prev;
        p + 1
    }

    /// A number at `p` (a digit, or `.` before one): its base's prefix, digits, a float's `.`
    /// and exponent, a suffix.
    #[inline(never)]
    fn number<const PP: bool>(&mut self, p: usize) -> usize {
        let b = self.b;
        if b[p] == b'0' {
            match b.get(p + 1).map(|c| c | 0x20) {
                Some(b'x') => return self.hex::<PP>(p),
                Some(b'b') if matches!(b.get(p + 2), Some(b'0' | b'1')) => {
                    let e = digits(b, p + 2, |c| matches!(c, b'0' | b'1'));
                    self.put::<PP>(p, p + 2, BIN_BASE);
                    self.put::<PP>(p + 2, e, BIN);
                    return self.suffix::<PP>(e, BIN_SUFFIX, DEC_BAD_SUFFIX, false);
                }
                _ => {}
            }
        }
        let e = digits(b, p, |c| c.is_ascii_digit());
        if matches!(b.get(e), Some(b'.'))
            || matches!(b.get(e), Some(b'e' | b'E')) && exp_end(b, e).is_some()
        {
            return self.float::<PP>(p, e);
        }
        if b[p] == b'0' && e > p + 1 {
            self.put::<PP>(p, p + 1, OCT_BASE);
            self.put::<PP>(p + 1, e, OCT);
            return self.suffix::<PP>(e, OCT_SUFFIX, DEC_BAD_SUFFIX, false);
        }
        self.put::<PP>(p, e, DEC);
        self.suffix::<PP>(e, DEC_SUFFIX, DEC_BAD_SUFFIX, false)
    }

    /// A decimal float whose integer part is `p..e`: its `.`, its fraction, its exponent, a
    /// suffix.
    fn float<const PP: bool>(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        if e > p {
            self.put::<PP>(p, e, FLOAT);
        }
        let mut q = e;
        if b.get(q) == Some(&b'.') {
            self.put::<PP>(q, q + 1, FLOAT_DOT);
            let f = digits(b, q + 1, |c| c.is_ascii_digit());
            if f > q + 1 {
                self.put::<PP>(q + 1, f, FLOAT);
            }
            q = f;
        }
        if let Some(x) = exp_end(b, q) {
            self.put::<PP>(q, x, FLOAT);
            q = x;
        }
        self.suffix::<PP>(q, FLOAT_SUFFIX, FLOAT_BAD_SUFFIX, true)
    }

    /// A hexadecimal number at `p`, its `0x` there.
    fn hex<const PP: bool>(&mut self, p: usize) -> usize {
        let b = self.b;
        let e = digits(b, p + 2, |c| c.is_ascii_hexdigit());
        if matches!(b.get(e), Some(b'.' | b'p' | b'P')) {
            // a hexadecimal float
            self.put::<PP>(p, p + 2, HEX_FLOAT_BASE);
            if e > p + 2 {
                self.put::<PP>(p + 2, e, HEX_FLOAT);
            }
            let mut q = e;
            if b.get(q) == Some(&b'.') {
                self.put::<PP>(q, q + 1, HEX_FLOAT_DOT);
                let f = digits(b, q + 1, |c| c.is_ascii_hexdigit());
                if f > q + 1 {
                    self.put::<PP>(q + 1, f, HEX_FLOAT);
                }
                q = f;
            }
            if matches!(b.get(q), Some(b'p' | b'P')) {
                let mut x = q + 1;
                if matches!(b.get(x), Some(b'+' | b'-')) {
                    x += 1;
                }
                let d = digits(b, x, |c| c.is_ascii_digit());
                if d > x {
                    self.put::<PP>(q, d, HEX_FLOAT);
                    q = d;
                }
            }
            return self.suffix::<PP>(q, HEX_FLOAT_SUFFIX, FLOAT_BAD_SUFFIX, true);
        }
        self.put::<PP>(p, p + 2, HEX_BASE);
        if e > p + 2 {
            self.put::<PP>(p + 2, e, HEX);
        }
        self.suffix::<PP>(e, HEX_SUFFIX, HEX_BAD_SUFFIX, false)
    }

    /// A number's suffix at `e`, a name's bytes: `ok` where it is one (`u`, `ll`, `f`…), else
    /// `bad`.
    #[inline(always)]
    fn suffix<const PP: bool>(&mut self, e: usize, ok: u16, bad: u16, float: bool) -> usize {
        let b = self.b;
        let mut s = e;
        while s < b.len() && CC[usize::from(b[s])] & NW != 0 {
            s += 1;
        }
        if s > e {
            let w = &b[e..s];
            let good = if float {
                float_suffix(w)
            } else {
                int_suffix(w)
            };
            self.put::<PP>(e, s, if good { ok } else { bad });
        }
        s
    }

    /// A `"…"` string at `p`, its quote there.
    pub(super) fn dq<const PP: bool>(&mut self, p: usize) -> usize {
        self.put::<PP>(p, p + 1, DQ_BEGIN);
        self.dq_body::<PP>(p + 1)
    }

    /// A `"…"` string's text from `q`: escapes, `printf`'s placeholders, its end. A string
    /// whose line ends in `\` goes on on the next ([`M_DQ`]); one that doesn't ends at the
    /// line's end (ada's: bat's runs on into the lines after).
    #[inline(never)]
    pub(super) fn dq_body<const PP: bool>(&mut self, mut q: usize) -> usize {
        let b = self.b;
        let mut s = q;
        loop {
            q = prim::find_in(b, q, b.len(), *b"\"\\%");
            if q >= b.len() {
                if b.len() > s {
                    self.put::<PP>(s, b.len(), DQ);
                }
                return b.len();
            }
            match b[q] {
                b'"' => {
                    if q > s {
                        self.put::<PP>(s, q, DQ);
                    }
                    self.put::<PP>(q, q + 1, DQ_END);
                    return q + 1;
                }
                b'\\' => {
                    if q > s {
                        self.put::<PP>(s, q, DQ);
                    }
                    if q + 1 == b.len() {
                        self.put::<PP>(q, q + 1, DQ_CONT);
                        self.mode = M_DQ;
                        return b.len();
                    }
                    let e = escape_end(b, q);
                    self.put::<PP>(q, e, DQ_ESCAPE);
                    (s, q) = (e, e);
                }
                _ => match placeholder(b, q) {
                    Some(e) => {
                        if q > s {
                            self.put::<PP>(s, q, DQ);
                        }
                        self.put::<PP>(q, e, DQ_PLACEHOLDER);
                        (s, q) = (e, e);
                    }
                    None => q += 1,
                },
            }
        }
    }

    /// A character `'…'` at `p`, its quote there.
    pub(super) fn sq<const PP: bool>(&mut self, p: usize) -> usize {
        self.put::<PP>(p, p + 1, SQ_BEGIN);
        self.sq_body::<PP>(p + 1)
    }

    /// A character's text from `q`, as [`Scan::dq_body`]'s (no placeholders).
    #[inline(never)]
    pub(super) fn sq_body<const PP: bool>(&mut self, mut q: usize) -> usize {
        let b = self.b;
        let mut s = q;
        loop {
            q = prim::find_swar(b, q, *b"'\\");
            if q >= b.len() {
                if b.len() > s {
                    self.put::<PP>(s, b.len(), SQ);
                }
                return b.len();
            }
            if q > s {
                self.put::<PP>(s, q, SQ);
            }
            if b[q] == b'\'' {
                self.put::<PP>(q, q + 1, SQ_END);
                return q + 1;
            }
            if q + 1 == b.len() {
                self.put::<PP>(q, q + 1, SQ_CONT);
                self.mode = M_SQ;
                return b.len();
            }
            let e = escape_end(b, q);
            self.put::<PP>(q, e, SQ_ESCAPE);
            (s, q) = (e, e);
        }
    }

    /// A raw string at `p` (its prefix to `q`, its quote): `R"delim(…)delim"`, on to the lines
    /// after till its end ([`M_RAW`]); `None` where no `(` ends a delimiter.
    fn raw<const PP: bool>(&mut self, p: usize, q: usize) -> Option<usize> {
        let b = self.b;
        let mut d = q + 1;
        while d < b.len() && d - q <= 17 && !matches!(b[d], b'(' | b')' | b'\\' | b'"' | b' ') {
            d += 1;
        }
        if b.get(d) != Some(&b'(') {
            return None;
        }
        let n = d - q - 1;
        self.delim = [0; DELIM_KEPT];
        let kept = n.min(DELIM_KEPT);
        self.delim[..kept].copy_from_slice(&b[q + 1..q + 1 + kept]);
        self.aux = n as u8;
        self.put::<PP>(p, q, DQ_PREFIX);
        self.put::<PP>(q, d + 1, DQ_BEGIN);
        if let Some(dialect) = self.regex_here::<PP>(p) {
            self.delim[RE_DIALECT] = regex_calls::dialect_code(dialect);
            self.delim[RE_STATE] = 0;
            return Some(self.raw_regex(d + 1));
        }
        Some(self.raw_body::<PP>(d + 1))
    }

    /// A `"…"` string at `p` on a line with a regex engine's name ([`Scan::rx`]): a regex's
    /// where the call it is an argument of takes a pattern.
    #[cold]
    #[inline(never)]
    fn dq_checked(&mut self, p: usize) -> usize {
        if let Some(d) = self.regex_here::<false>(p) {
            self.put::<false>(p, p + 1, DQ_BEGIN);
            return self.dq_regex(p + 1, d);
        }
        self.dq::<false>(p)
    }

    /// Whether the string at `s` (its prefix, or its quote) is a regex, by the call it is an
    /// argument of ([`regex_calls::regex_at`]): its dialect. Not in a directive.
    #[inline(always)]
    fn regex_here<const PP: bool>(&self, s: usize) -> Option<regex::Dialect> {
        if PP || !self.rx {
            return None;
        }
        let b = self.b;
        let mut q = s;
        while q > 0 && matches!(b[q - 1], b' ' | b'\t') {
            q -= 1;
        }
        if q == 0 || !matches!(b[q - 1], b'(' | b'{' | b',' | b'<' | b')') {
            return None;
        }
        regex_calls::regex_at(b, q - 1, s, self.c)
    }

    /// A regex's text in a `"…"` string from `p`, past its quote, on [`crate::regex`] in
    /// `dialect`: the string's end and escapes the host's, the rest the regex's. A string whose
    /// line ends in `\` goes on on the next as a string ([`M_DQ`]).
    #[cold]
    #[inline(never)]
    fn dq_regex(&mut self, mut p: usize, dialect: regex::Dialect) -> usize {
        static YIELDS: [bool; 256] = regex::table(b"\"\\");
        let b = self.b;
        let n = b.len();
        let host = regex::Embed {
            dialect,
            cooked: true,
            yields: &YIELDS,
            stop: |b, p, _| b[p] == b'"',
            arg: 0,
            escape: escape_end,
            top_only: false,
            continues: true,
            base: RE_DQ,
            text: DQ,
        };
        let mut re = 0;
        while p < n {
            match b[p] {
                b'"' => {
                    self.put::<false>(p, p + 1, DQ_END);
                    return p + 1;
                }
                b'\\' if p + 1 == n => {
                    self.put::<false>(p, n, DQ_CONT);
                    self.mode = M_DQ;
                    return n;
                }
                b'\\' if b[p + 1] != b'\\' => {
                    let e = escape_end(b, p).min(n);
                    self.put::<false>(p, e, DQ_ESCAPE);
                    p = e;
                    continue;
                }
                _ => {}
            }
            self.k.close(self.out);
            let mut at = p;
            let r = regex::embedded(b, p, re, &host, &mut at, self.out);
            self.k.reopen(self.out, 16);
            match r {
                Some((e, st)) => {
                    re = st;
                    p = e.max(p + 1);
                }
                None => break,
            }
        }
        n
    }

    /// A regex's text in a raw string from `q` to its `)delim"`, or the line's end
    /// ([`M_RAW_RE`]), on [`crate::regex`] in the dialect and from the state [`Scan::delim`]
    /// keeps after the delimiter's bytes ([`RE_DIALECT`], [`RE_STATE`]): as it
    /// reads, no escapes.
    #[cold]
    #[inline(never)]
    pub(super) fn raw_regex(&mut self, q: usize) -> usize {
        static YIELDS: [bool; 256] = [false; 256];
        let b = self.b;
        let n = usize::from(self.aux);
        let kept = n.min(RE_DELIM_KEPT);
        // the string's end on this line, if it is on it
        let mut end = None;
        let mut i = q;
        loop {
            i = prim::find_swar(b, i, *b")");
            if i >= b.len() {
                break;
            }
            let e = i + 1 + n;
            if e < b.len() && b[e] == b'"' && b[i + 1..i + 1 + kept] == self.delim[..kept] {
                end = Some(i);
                break;
            }
            i += 1;
        }
        let lim = end.unwrap_or(b.len());
        if lim > q {
            let host = regex::Embed {
                dialect: regex_calls::dialect_of(self.delim[RE_DIALECT]),
                cooked: false,
                yields: &YIELDS,
                stop: |_, _, _| false,
                arg: 0,
                escape: |_, p| p,
                top_only: false,
                continues: false,
                base: RE_DQ,
                text: DQ,
            };
            self.k.close(self.out);
            let mut at = q;
            let st = self.delim[RE_STATE];
            if let Some((_, st)) = regex::embedded(&b[..lim], q, st, &host, &mut at, self.out) {
                self.delim[RE_STATE] = st;
            }
            self.k.reopen(self.out, 16);
        }
        match end {
            Some(i) => {
                let e = i + 1 + n;
                self.put::<false>(i, e + 1, DQ_END);
                self.mode = M_CODE;
                self.aux = 0;
                self.delim = [0; DELIM_KEPT];
                e + 1
            }
            None => {
                self.mode = M_RAW_RE;
                b.len()
            }
        }
    }

    /// A raw string's text from `q` to its `)delim"`, or the line's end.
    pub(super) fn raw_body<const PP: bool>(&mut self, q: usize) -> usize {
        let b = self.b;
        let n = usize::from(self.aux);
        let kept = n.min(DELIM_KEPT);
        let mut i = q;
        loop {
            i = prim::find_swar(b, i, *b")");
            if i >= b.len() {
                if b.len() > q {
                    self.put::<PP>(q, b.len(), DQ);
                }
                self.mode = M_RAW;
                return b.len();
            }
            let e = i + 1 + n;
            if e < b.len() && b[e] == b'"' && b[i + 1..i + 1 + kept] == self.delim[..kept] {
                if i > q {
                    self.put::<PP>(q, i, DQ);
                }
                self.put::<PP>(i, e + 1, DQ_END);
                self.mode = M_CODE;
                self.aux = 0;
                self.delim = [0; DELIM_KEPT];
                return e + 1;
            }
            i += 1;
        }
    }

    /// A `//` comment at `p`, to the line's end; one that ends in `\` goes on on the next
    /// ([`M_LINE_COMMENT`]).
    #[inline(never)]
    fn line_comment<const PP: bool>(&mut self, p: usize) -> usize {
        self.put::<PP>(p, p + 2, LINE_MARK);
        self.comment_line::<PP>(p + 2);
        self.b.len()
    }

    /// A `//` comment's text from `q` to the line's end.
    pub(super) fn comment_line<const PP: bool>(&mut self, q: usize) {
        let b = self.b;
        let n = b.len();
        if b.last() == Some(&b'\\') && n > q {
            if n - 1 > q {
                self.put::<PP>(q, n - 1, LINE_TEXT);
            }
            self.put::<PP>(n - 1, n, LINE_CONT);
            self.mode = M_LINE_COMMENT;
        } else {
            if n > q {
                self.put::<PP>(q, n, LINE_TEXT);
            }
            self.mode = M_CODE;
        }
    }

    /// A `/* … */` comment at `p`, on to the lines after till its end ([`M_BLOCK_COMMENT`]).
    #[inline(never)]
    fn block_comment<const PP: bool>(&mut self, p: usize) -> usize {
        self.put::<PP>(p, p + 2, COMMENT_MARK);
        self.comment_block::<PP>(p + 2)
    }

    /// A block comment's line that the line before left open: a leading `*` (not `*/`) its
    /// punctuation, as bat has it, then its text.
    pub(super) fn comment_lines<const PP: bool>(&mut self) -> usize {
        let b = self.b;
        let mut q = 0;
        while q < b.len() && CC[usize::from(b[q])] & SP != 0 {
            q += 1;
        }
        if b.get(q) == Some(&b'*') && b.get(q + 1) != Some(&b'/') {
            if q > 0 {
                self.put::<PP>(0, q, BLOCK_TEXT);
            }
            self.put::<PP>(q, q + 1, COMMENT_MARK);
            return self.comment_block::<PP>(q + 1);
        }
        self.comment_block::<PP>(0)
    }

    /// A block comment's text from `q` to its `*/`, or the line's end.
    pub(super) fn comment_block<const PP: bool>(&mut self, q: usize) -> usize {
        let b = self.b;
        let mut i = q;
        loop {
            i = prim::find_in(b, i, b.len(), *b"*");
            if i + 1 >= b.len() {
                if b.len() > q {
                    self.put::<PP>(q, b.len(), BLOCK_TEXT);
                }
                self.mode = M_BLOCK_COMMENT;
                return b.len();
            }
            if b[i + 1] == b'/' {
                if i > q {
                    self.put::<PP>(q, i, BLOCK_TEXT);
                }
                self.put::<PP>(i, i + 2, COMMENT_MARK);
                self.mode = M_CODE;
                return i + 2;
            }
            i += 1;
        }
    }
}

/// The byte at `q`, `\n` at the line's end.
#[inline(always)]
fn next_byte(b: &[u8], q: usize) -> u8 {
    b.get(q).copied().unwrap_or(b'\n')
}

/// Where a generic's `>` is, its `<` at `lt`, on bat's shape: names, blanks, `,`, `::` and the
/// like (no byte of `(){}&;*^%=<>-`), with one nested generic (itself with one) before at most
/// one parenthesized group: `<(?:T+(?:<(?:T+(?:<T*>)?)?\s*>)?)?T*(?:\(T*\))?T*>`. A comparison
/// (`a < b;`, `i < n)`) has no such end.
#[inline(always)]
fn generic_end(b: &[u8], lt: usize) -> Option<usize> {
    let run = |mut i: usize| {
        while i < b.len() && GT[usize::from(b[i])] {
            i += 1;
        }
        i
    };
    let mut i = run(lt + 1);
    if b.get(i) == Some(&b'<') && i > lt + 1 {
        let s = i + 1;
        let mut j = run(s);
        if b.get(j) == Some(&b'<') && j > s {
            let k = run(j + 1);
            if b.get(k) != Some(&b'>') {
                return None;
            }
            j = skip_blanks(b, k + 1);
        }
        if b.get(j) != Some(&b'>') {
            return None;
        }
        i = run(j + 1);
    }
    if b.get(i) == Some(&b'(') {
        let k = run(i + 1);
        if b.get(k) != Some(&b')') {
            return None;
        }
        i = run(k + 1);
    }
    (b.get(i) == Some(&b'>')).then_some(i)
}

/// The bytes a generic's text takes ([`generic_end`]).
static GT: [bool; 256] = {
    let mut t = [true; 256];
    let x = b"(){}&;*^%=<>-";
    let mut i = 0;
    while i < x.len() {
        t[x[i] as usize] = false;
        i += 1;
    }
    t
};

/// Whether the word at `q` is `w`.
fn word_at(b: &[u8], q: usize, w: &[u8]) -> bool {
    b.get(q..q + w.len()) == Some(w)
        && b.get(q + w.len())
            .is_none_or(|&c| CC[usize::from(c)] & NW == 0)
}

/// Whether the word before `p`, past its blanks, is `friend`.
#[cold]
fn after_friend(b: &[u8], p: usize) -> bool {
    let s = b[..p].trim_ascii_end();
    s.ends_with(b"friend") && (s.len() == 6 || CC[usize::from(s[s.len() - 7])] & NW == 0)
}

/// Whether a name is a macro's by its shape: upper case, digits and `_`, no lower case.
fn upper(w: &[u8]) -> bool {
    w.iter()
        .all(|&c| CC[usize::from(c)] & (UP | DG) != 0 || c == b'_')
}

/// Where the digits `digit` takes from `p` end, a `'` between two of them taken (C++14's
/// separator).
#[inline(always)]
fn digits(b: &[u8], mut p: usize, digit: impl Fn(u8) -> bool) -> usize {
    while p < b.len() {
        let c = b[p];
        if digit(c)
            || (c == b'\'' && p + 1 < b.len() && digit(b[p + 1]) && p > 0 && digit(b[p - 1]))
        {
            p += 1;
        } else {
            break;
        }
    }
    p
}

/// An exponent at `e` (`e`, a sign, digits): where it ends.
#[inline(always)]
fn exp_end(b: &[u8], e: usize) -> Option<usize> {
    if !matches!(b.get(e), Some(b'e' | b'E')) {
        return None;
    }
    let mut x = e + 1;
    if matches!(b.get(x), Some(b'+' | b'-')) {
        x += 1;
    }
    let d = digits(b, x, |c| c.is_ascii_digit());
    (d > x).then_some(d)
}

/// An integer's suffix: `u`, `l`, `ll`, `z` and their pairs, either case.
fn int_suffix(w: &[u8]) -> bool {
    let mut u = false;
    let mut rest = w;
    if let [c, r @ ..] = rest
        && (*c | 0x20) == b'u'
    {
        u = true;
        rest = r;
    }
    rest = match rest {
        [b'l', b'l', r @ ..] | [b'L', b'L', r @ ..] => r,
        [b'l' | b'L' | b'z' | b'Z', r @ ..] => r,
        r => r,
    };
    if !u
        && let [c, r @ ..] = rest
        && (*c | 0x20) == b'u'
    {
        rest = r;
    }
    rest.is_empty()
}

/// A float's suffix: `f`, `l`, `f16`…`bf16`, either case.
fn float_suffix(w: &[u8]) -> bool {
    matches!(
        w,
        b"f" | b"F"
            | b"l"
            | b"L"
            | b"f16"
            | b"f32"
            | b"f64"
            | b"f128"
            | b"F16"
            | b"F32"
            | b"F64"
            | b"F128"
            | b"bf16"
            | b"BF16"
            | b"df"
            | b"dd"
            | b"dl"
            | b"DF"
            | b"DD"
            | b"DL"
    )
}

/// An escape at `q` (its `\`, not the line's last byte): where it ends.
#[inline(always)]
fn escape_end(b: &[u8], q: usize) -> usize {
    let c = b[q + 1];
    match c {
        b'x' => {
            let e = digits(b, q + 2, |c| c.is_ascii_hexdigit());
            e.max(q + 2)
        }
        b'u' | b'U' => {
            let n = if c == b'u' { 4 } else { 8 };
            let mut e = q + 2;
            while e < b.len() && e < q + 2 + n && b[e].is_ascii_hexdigit() {
                e += 1;
            }
            e
        }
        b'0'..=b'7' => {
            let mut e = q + 2;
            while e < b.len() && e < q + 4 && matches!(b[e], b'0'..=b'7') {
                e += 1;
            }
            e
        }
        _ => q + 1 + prim::char_len(c),
    }
}

/// `printf`'s placeholder at `q`, its `%`: where it ends (bat's
/// `%(\d+\$)?[#0\- +']*[,;:_]?((-?\d+)|\*(-?\d+\$)?)?(\.((-?\d+)|\*(-?\d+\$)?)?)?(hh|h|ll|l|j|t|z|q|L|vh|vl|v|hv|hl)?[diouxXDOUeEfFgGaACcSspn%]`).
fn placeholder(b: &[u8], q: usize) -> Option<usize> {
    let at = |i: usize| b.get(i).copied().unwrap_or(0);
    let mut i = q + 1;
    // `\d+\$`
    let d = digits(b, i, |c| c.is_ascii_digit());
    if d > i && at(d) == b'$' {
        i = d + 1;
    }
    while matches!(at(i), b'#' | b'0' | b'-' | b' ' | b'+' | b'\'') {
        i += 1;
    }
    if matches!(at(i), b',' | b';' | b':' | b'_') {
        i += 1;
    }
    // a width: `-?\d+` or `\*(-?\d+\$)?`
    let width = |i: usize| -> usize {
        if at(i) == b'*' {
            let mut j = i + 1;
            let s = if at(j) == b'-' { j + 1 } else { j };
            let d = digits(b, s, |c| c.is_ascii_digit());
            if d > s && at(d) == b'$' {
                j = d + 1;
            }
            j
        } else {
            let s = if at(i) == b'-' { i + 1 } else { i };
            let d = digits(b, s, |c| c.is_ascii_digit());
            if d > s { d } else { i }
        }
    };
    i = width(i);
    if at(i) == b'.' {
        i = width(i + 1);
    }
    i += match (at(i), at(i + 1)) {
        (b'h', b'h') | (b'l', b'l') | (b'v', b'h') | (b'v', b'l') | (b'h', b'v') | (b'h', b'l') => {
            2
        }
        (b'h' | b'l' | b'j' | b't' | b'z' | b'q' | b'L' | b'v', _) => 1,
        _ => 0,
    };
    matches!(
        at(i),
        b'd' | b'i'
            | b'o'
            | b'u'
            | b'x'
            | b'X'
            | b'D'
            | b'O'
            | b'U'
            | b'e'
            | b'E'
            | b'f'
            | b'F'
            | b'g'
            | b'G'
            | b'a'
            | b'A'
            | b'C'
            | b'c'
            | b'S'
            | b's'
            | b'p'
            | b'n'
            | b'%'
    )
    .then_some(i + 1)
}
