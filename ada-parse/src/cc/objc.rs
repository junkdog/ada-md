//! Objective-C's and Objective-C++'s tokens on C's and C++'s scanner (X-134): what C has, with
//! `@` keywords and literals, message sends, method heads and Cocoa's words. HEAD's C and C++
//! scanners ran Objective-C's census past the bar before these (the floor, vault `x134/`), so
//! Objective-C is two more rows of the same loop, each construct here out of its way: the loop
//! asks only where a frame or a head is open ([`Scan::nbr`], [`Scan::head`]) or at an `@` and a
//! `[`.
//!
//! - **A send** is a `[` where an expression starts (not after a value: a name, `)`, `]`, a
//!   string; `return [`, `in [` do start one). Its frame ([`F_RECV`]) holds while its receiver is
//!   read; a name after a blank that follows a value is its selector ([`SELECTOR`], with its `:`),
//!   and the frame then [`F_ARGS`], where a name before `:` is a keyword part ([`SELECTOR_PART`]).
//!   bat's send needs `[name<blanks>x`: a send whose receiver is a send, a member or a cast is a
//!   bracket there (`[[Foo alloc] init]`'s `init` plain, its keyword parts' `:` the ternary's);
//!   ada reads them as sends, by design. A `{` inside a frame opens a block's frame ([`F_BLOCK`]),
//!   so a block's statements are not the send's arguments; a `}` closes it and any `[` left open
//!   in it, a `;` the sends left open in its statement.
//! - **A method's head** is a line at a namespace's level whose first token is `-` or `+`:
//!   `(Type)` its return type ([`TYPE_PAREN`]), the name after it the method's ([`FUNC_DEF`]),
//!   `part:` its keyword parts ([`METHOD_PART`]), the name after a part's `(Type)` a parameter
//!   ([`METHOD_PARAM`]), to its `{` or `;`, over lines. A macro after the parts
//!   (`NS_SWIFT_NAME(run(block:))`) is a call: bat reads its `)` as a stray bracket and leaves
//!   the `@interface`, every head after it arithmetic, to the file's end.
//! - **`@` keywords** with their `@` ([`AT_STORAGE`]…), `@interface`'s and `@implementation`'s
//!   name, super class, category and protocols; `@property`'s attributes; `@selector(…)`;
//!   `@"…"` a string with `%@`; `@[`, `@{`, `@(` and a number's `@` plain, a dictionary's `:` a
//!   separator (bat's ternary). ada's own: `@required`, `@optional`, `@package` modifiers,
//!   `@autoreleasepool` a keyword, `@import` an import (bat's plain).
//!
//! The state between lines: Objective-C keeps [`LEVELS_OBJC`] brace levels, and in the two bytes
//! above them its innermost [`FRAMES_KEPT`] frames with their count, and the head ([`Scan::load`]).

use super::code::Embed;
use super::pp::skip_blanks;
use super::words::{ACT, COCOA_INFO, COCOA_WORDS};
use super::*;
use crate::words::{Words, text_len};

/// A selector whose string is SQL (FMDB's `executeQuery:`…), or a regex (`NSRegularExpression`'s
/// `regularExpressionWithPattern:`…): [`Scan::sel`].
pub(super) const SEL_SQL: u8 = 1;
pub(super) const SEL_RE: u8 = 2;

/// The selectors that take SQL or a regex: FMDB's, `NSRegularExpression`'s (ICU's syntax:
/// Java's dialect), and the string methods' with `NSRegularExpressionSearch` after them.
const SELECTOR_LIST: [&[u8]; 10] = [
    b"executeQuery",
    b"executeUpdate",
    b"executeStatements",
    b"executeQueryWithFormat",
    b"executeUpdateWithFormat",
    b"regularExpressionWithPattern",
    b"initWithPattern",
    b"rangeOfString",
    b"stringByReplacingOccurrencesOfString",
    b"replaceOccurrencesOfString",
];
static SELECTOR_WORDS: Words<32, { SELECTOR_LIST.len() }, { text_len(&SELECTOR_LIST) }> =
    Words::new(&SELECTOR_LIST);

/// A frame of [`Scan::br`]: a bracket that isn't a send (a subscript, `@[`, a lambda's).
pub(super) const F_PLAIN: u32 = 0;
/// A send, its receiver being read.
pub(super) const F_RECV: u32 = 1;
/// A send past its selector: a name before `:` a keyword part.
pub(super) const F_ARGS: u32 = 2;
/// A block (or a dictionary) opened in a frame: its statements aren't the send's.
pub(super) const F_BLOCK: u32 = 3;

/// The frames [`State`] keeps, the innermost.
pub(super) const FRAMES_KEPT: u8 = 3;
/// The brace levels Objective-C keeps: two bytes of [`State`]'s levels hold its frames and head.
pub(super) const LEVELS_OBJC: usize = 52;
const OBJC_MASK: u128 = (1 << (2 * LEVELS_OBJC)) - 1;

// Where a method's head is ([`Scan::head`]).
/// After its `-` or `+`: its return type's `(`, or its name.
pub(super) const H_START: u8 = 1;
/// In its return type's parentheses.
const H_RET: u8 = 2;
/// After its return type: its name.
const H_NAMED: u8 = 3;
/// After a part's `:`: a type's `(`, or the parameter.
const H_PART: u8 = 4;
/// In a parameter's type's parentheses.
const H_ARG: u8 = 5;
/// After a parameter's type: the parameter.
const H_TYPED: u8 = 6;
/// After a parameter: a part, `, ...`, or what follows the head (a macro, `{`, `;`).
const H_PARAMS: u8 = 7;
/// Past the parts: attributes, macros, to its `{` or `;`.
const H_DONE: u8 = 8;

/// The `@` keywords: the word, the `@`'s leaf, the word's. The first six are read on
/// ([`AT_INTERFACE`]…).
const AT_KEYWORDS: [(&[u8], u16, u16); 26] = [
    (b"interface", AT_STORAGE, STORAGE_TYPE),
    (b"implementation", AT_STORAGE, STORAGE_TYPE),
    (b"protocol", AT_STORAGE, STORAGE_TYPE),
    (b"end", AT_STORAGE, STORAGE_TYPE),
    (b"property", AT_PROPERTY, PROPERTY),
    (b"selector", AT_STORAGE, STORAGE_TYPE),
    (b"class", AT_STORAGE, STORAGE_TYPE),
    (b"synthesize", AT_DIRECTIVE, DIRECTIVE),
    (b"dynamic", AT_DIRECTIVE, DIRECTIVE),
    (b"try", AT_EXCEPTION, EXCEPTION),
    (b"catch", AT_EXCEPTION, EXCEPTION),
    (b"finally", AT_EXCEPTION, EXCEPTION),
    (b"throw", AT_EXCEPTION, EXCEPTION),
    (b"synchronized", AT_SYNC, SYNC),
    (b"public", AT_MODIFIER, MODIFIER),
    (b"private", AT_MODIFIER, MODIFIER),
    (b"protected", AT_MODIFIER, MODIFIER),
    (b"package", AT_MODIFIER, MODIFIER),
    (b"optional", AT_MODIFIER, MODIFIER),
    (b"required", AT_MODIFIER, MODIFIER),
    (b"encode", AT_OTHER, KEYWORD_OTHER),
    (b"defs", AT_OTHER, KEYWORD_OTHER),
    (b"available", AT_OTHER, KEYWORD_OTHER),
    (b"compatibility_alias", AT_OTHER, KEYWORD_OTHER),
    (b"autoreleasepool", AT_CONTROL, CONTROL),
    (b"import", AT_IMPORT, IMPORT),
];
const AT_INTERFACE: u16 = 1;
const AT_IMPLEMENTATION: u16 = 2;
const AT_PROTOCOL: u16 = 3;
const AT_END: u16 = 4;
const AT_PROPERTY_ID: u16 = 5;
const AT_SELECTOR: u16 = 6;

const AT_LIST: [&[u8]; AT_KEYWORDS.len()] = {
    let mut l: [&[u8]; AT_KEYWORDS.len()] = [b""; AT_KEYWORDS.len()];
    let mut i = 0;
    while i < l.len() {
        l[i] = AT_KEYWORDS[i].0;
        i += 1;
    }
    l
};
static AT_WORDS: Words<64, { AT_LIST.len() }, { text_len(&AT_LIST) }> = Words::new(&AT_LIST);

/// The words after which a `[` starts an expression (`return [self x]`).
const EXPRESSION_LIST: [&[u8]; 8] = [
    b"return",
    b"else",
    b"case",
    b"in",
    b"do",
    b"throw",
    b"co_return",
    b"co_yield",
];
static EXPRESSION_WORDS: Words<16, { EXPRESSION_LIST.len() }, { text_len(&EXPRESSION_LIST) }> =
    Words::new(&EXPRESSION_LIST);

/// `@property`'s attributes (bat's, `class` among them).
fn property_attribute(w: &[u8]) -> bool {
    matches!(
        w,
        b"getter"
            | b"setter"
            | b"readonly"
            | b"readwrite"
            | b"assign"
            | b"retain"
            | b"copy"
            | b"nonatomic"
            | b"atomic"
            | b"strong"
            | b"weak"
            | b"unsafe_unretained"
            | b"class"
            | b"nonnull"
            | b"nullable"
            | b"null_unspecified"
            | b"null_resettable"
            | b"direct"
    )
}

impl<K: Sink, const O: bool> Scan<'_, '_, K, O> {
    /// [`Scan::load`]'s Objective-C: the brace levels below the frames and the head.
    #[inline(always)]
    pub(super) fn load_objc(&mut self, sends: u8, head: u8) {
        // most lines start with neither open: [`Scan::new`]'s zeros stand
        if sends | head != 0 {
            (self.nbr, self.br) = (sends >> 6, u32::from(sends & 0x3f));
            (self.head, self.htp) = (head & 0x0f, head >> 4 & 3);
        }
    }

    /// [`Scan::state`]'s bytes for Objective-C's frames and head, kept above its
    /// [`LEVELS_OBJC`] brace levels: the innermost [`FRAMES_KEPT`] frames and their count, the
    /// head and its type's `(`s.
    #[inline(always)]
    pub(super) fn objc_state(&self) -> (u8, u8) {
        if self.nbr | self.head == 0 {
            return (0, 0);
        }
        let kept = self.nbr.min(FRAMES_KEPT);
        let frames = self.br & ((1 << (2 * kept)) - 1);
        (
            kept << 6 | frames as u8,
            self.head & 0x0f | self.htp.min(3) << 4,
        )
    }

    /// [`Scan::settle`]'s Objective-C: as [`Scan::objc_braces`] keeps them.
    #[inline(always)]
    pub(super) fn settle_objc(&mut self) {
        self.braces &= OBJC_MASK;
        self.nbr = self.nbr.min(FRAMES_KEPT);
        self.br &= (1 << (2 * self.nbr)) - 1;
        self.htp = self.htp.min(3);
    }

    /// The innermost frame ([`F_PLAIN`]…); [`F_PLAIN`] where none is open.
    #[inline(always)]
    fn frame(&self) -> u32 {
        self.br & 3
    }

    fn push(&mut self, f: u32) {
        self.br = self.br << 2 | f;
        self.nbr = self.nbr.saturating_add(1);
    }

    fn pop(&mut self) {
        if self.nbr > 0 {
            self.br >>= 2;
            self.nbr -= 1;
        }
    }

    /// A line's first token at `p`, in Objective-C: a method's head where it is `-` or `+` at a
    /// namespace's level, outside a statement; where the code goes on from.
    #[inline(always)]
    pub(super) fn objc_line(&mut self, p: usize) -> usize {
        let b = self.b;
        if p < b.len()
            && matches!(b[p], b'-' | b'+')
            && self.head == 0
            && self.nbr == 0
            && self.paren == 0
            && self.decl == D_NONE
            && self.level() == NS
        {
            // the `-` or `+` plain, as bat has it; no definition's name by C's rule in the head
            (self.head, self.htp, self.decl, self.prev) = (H_START, 0, D_DONE, P_OTHER);
            return p + 1;
        }
        p
    }

    /// The name `p..e` where a frame or a head is open: a selector, a keyword part, a method's
    /// name, part or parameter; `None` where it is code as anywhere.
    #[inline(always)]
    pub(super) fn objc_word<const PP: bool>(&mut self, p: usize, e: usize) -> Option<usize> {
        // the common names in a send turned away here: in its arguments a part is a name before
        // `:`, in its receiver the selector a name after a blank or at the line's start
        if self.head == 0 {
            let b = self.b;
            match self.frame() {
                F_ARGS if b.get(e) != Some(&b':') => return None,
                F_RECV if p > 0 && CC[usize::from(b[p - 1])] & SP == 0 => return None,
                F_PLAIN | F_BLOCK => return None,
                _ => {}
            }
        }
        self.objc_name::<PP>(p, e)
    }

    /// [`Scan::objc_word`]'s name that may be a selector, a part or a head's.
    #[inline(never)]
    fn objc_name<const PP: bool>(&mut self, p: usize, e: usize) -> Option<usize> {
        if self.head != 0 {
            return self.head_word::<PP>(p, e);
        }
        let b = self.b;
        match self.frame() {
            F_RECV => {
                // a name after blanks that follow a value: the selector
                let mut q = p;
                while q > 0 && CC[usize::from(b[q - 1])] & SP != 0 {
                    q -= 1;
                }
                if q == p && p > 0 {
                    return None;
                }
                // at the line's start, the receiver was on the line before
                if q > 0 {
                    let c = b[q - 1];
                    if !(CC[usize::from(c)] & NW != 0 || matches!(c, b')' | b']' | b'"' | b'\'')) {
                        return None;
                    }
                }
                self.br = self.br & !3 | F_ARGS;
                self.prev = P_OTHER;
                if b.get(e) == Some(&b':') && b.get(e + 1) != Some(&b':') {
                    self.put::<PP>(p, e, SELECTOR);
                    self.put::<PP>(e, e + 1, SELECTOR_SEP);
                    self.mark_selector(p, e);
                    return Some(e + 1);
                }
                self.put::<PP>(p, e, SELECTOR);
                Some(e)
            }
            F_ARGS
                if self.ternary == 0 && b.get(e) == Some(&b':') && b.get(e + 1) != Some(&b':') =>
            {
                self.put::<PP>(p, e, SELECTOR_PART);
                self.put::<PP>(e, e + 1, SELECTOR_PART_SEP);
                self.prev = P_OTHER;
                self.mark_selector(p, e);
                Some(e + 1)
            }
            _ => None,
        }
    }

    /// A name `p..e` in a method's head.
    fn head_word<const PP: bool>(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        let colon = b.get(e) == Some(&b':') && b.get(e + 1) != Some(&b':');
        match self.head {
            H_RET | H_ARG => {
                let leaf = match &b[p..e] {
                    b"in" | b"out" | b"inout" | b"bycopy" | b"byref" | b"oneway" => {
                        PROTOCOL_MODIFIER
                    }
                    // ada's: bat's plain
                    b"nullable" | b"nonnull" | b"null_unspecified" | b"null_resettable" => MODIFIER,
                    _ => return None,
                };
                self.put::<PP>(p, e, leaf);
                self.prev = P_OTHER;
                Some(e)
            }
            H_START | H_NAMED => {
                self.put::<PP>(p, e, FUNC_DEF);
                self.prev = P_OTHER;
                if colon {
                    self.put::<PP>(e, e + 1, METHOD_SEP);
                    self.head = H_PART;
                    return Some(e + 1);
                }
                self.head = H_DONE;
                Some(e)
            }
            H_PART | H_TYPED => {
                self.put::<PP>(p, e, METHOD_PARAM);
                self.prev = P_OTHER;
                self.head = H_PARAMS;
                Some(e)
            }
            H_PARAMS if colon => {
                self.put::<PP>(p, e, METHOD_PART);
                self.put::<PP>(e, e + 1, METHOD_SEP);
                self.prev = P_OTHER;
                self.head = H_PART;
                Some(e + 1)
            }
            _ => {
                // a macro or an attribute after the parts: code, a call's before `(`
                self.head = H_DONE;
                None
            }
        }
    }

    /// The selector part `p..e` (its `:` at `e`): where it takes SQL or a regex, the string after
    /// it is checked ([`Scan::sel_at`]).
    fn mark_selector(&mut self, p: usize, e: usize) {
        let b = self.b;
        let id = SELECTOR_WORDS.get(&b[p..e]);
        let sel = match id {
            0 => return,
            1..=5 => SEL_SQL,
            6 | 7 => SEL_RE,
            // a string method's pattern where `NSRegularExpressionSearch` follows on its line
            _ if b[e..]
                .windows(25)
                .any(|w| w == b"NSRegularExpressionSearch") =>
            {
                SEL_RE
            }
            _ => return,
        };
        (self.sel, self.sel_at, self.rx) = (sel, e + 1, self.rx | RX_SEL);
    }

    /// A `(` or `)` (`open`) at `p` in a method's head: a type's parentheses, or `None` where it
    /// is code's.
    #[inline(never)]
    pub(super) fn head_paren<const PP: bool>(&mut self, p: usize, open: bool) -> Option<usize> {
        if open {
            if self.htp > 0 {
                // a block's or a function pointer's type: `(void (^)(BOOL))`
                self.htp = self.htp.saturating_add(1);
                self.put::<PP>(p, p + 1, GROUP_BEGIN);
            } else if matches!(self.head, H_START | H_PART) {
                self.htp = 1;
                self.head = if self.head == H_START { H_RET } else { H_ARG };
                self.put::<PP>(p, p + 1, TYPE_PAREN);
            } else {
                return None;
            }
        } else if self.htp > 1 {
            self.htp -= 1;
            self.put::<PP>(p, p + 1, GROUP_END);
        } else if self.htp == 1 {
            self.htp = 0;
            self.head = if self.head == H_RET { H_NAMED } else { H_TYPED };
            self.put::<PP>(p, p + 1, TYPE_PAREN);
        } else {
            return None;
        }
        self.prev = P_OTHER;
        Some(p + 1)
    }

    /// A `{` where a frame or a head is open: a head's body (the head ends), a block's frame.
    #[inline(never)]
    pub(super) fn objc_open_brace(&mut self) {
        (self.head, self.htp) = (0, 0);
        if self.nbr > 0 {
            self.push(F_BLOCK);
        }
    }

    /// A `}` where a frame is open: the block's frame closed, and any `[` left open in it.
    #[inline(never)]
    pub(super) fn objc_close_brace(&mut self) {
        while self.nbr > 0 && self.frame() != F_BLOCK {
            self.pop();
        }
        self.pop();
    }

    /// A `;` where a frame or a head is open: a head's declaration ends, and any send its
    /// statement left open.
    #[inline(never)]
    pub(super) fn objc_semicolon(&mut self) {
        (self.head, self.htp) = (0, 0);
        while self.nbr > 0 && self.frame() != F_BLOCK {
            self.pop();
        }
    }

    /// A `[` at `p` in Objective-C: a send where an expression starts, else a bracket.
    #[inline(never)]
    pub(super) fn bracket_open<const PP: bool>(&mut self, p: usize) -> usize {
        let b = self.b;
        let mut q = p;
        while q > 0 && CC[usize::from(b[q - 1])] & SP != 0 {
            q -= 1;
        }
        let send = q == 0 || {
            let c = b[q - 1];
            if CC[usize::from(c)] & NW != 0 {
                let mut s = q - 1;
                while s > 0 && CC[usize::from(b[s - 1])] & NW != 0 {
                    s -= 1;
                }
                EXPRESSION_WORDS.get(&b[s..q]) != 0
            } else if c == b')' {
                // a cast's: `(Foo *)[x sel]`, `(void)[x sel]`; `(a)[0]` a subscript
                send_after(b, p + 1)
            } else {
                !matches!(c, b']' | b'"' | b'\'')
            }
        };
        if send {
            self.push(F_RECV);
            self.put::<PP>(p, p + 1, SCOPE_BEGIN);
        } else {
            self.push(F_PLAIN);
            self.put::<PP>(p, p + 1, BRACKETS_BEGIN);
        }
        self.prev = P_OTHER;
        p + 1
    }

    /// A `]` at `p` where a frame is open: the innermost send or bracket closed (blocks left
    /// open in it with it).
    #[inline(never)]
    pub(super) fn bracket_close<const PP: bool>(&mut self, p: usize) -> usize {
        while self.nbr > 1 && self.frame() == F_BLOCK {
            self.pop();
        }
        let leaf = match self.frame() {
            F_RECV | F_ARGS => SCOPE_END,
            _ => BRACKETS_END,
        };
        self.pop();
        self.put::<PP>(p, p + 1, leaf);
        self.prev = P_OTHER;
        p + 1
    }

    /// An `@` at `p` in Objective-C: a keyword with its `@`, a string, a literal's `@`.
    #[inline(never)]
    pub(super) fn at_sign<const PP: bool>(&mut self, p: usize) -> usize {
        let b = self.b;
        self.prev = P_OTHER;
        match b.get(p + 1) {
            Some(b'"') => {
                self.put::<PP>(p, p + 2, DQ_BEGIN);
                if self.rx != 0 {
                    match self.embed_here::<PP>(p) {
                        Embed::Re(d) => return self.dq_regex(p + 2, d),
                        Embed::Sql(ctx) => return self.dq_sql(p + 2, ctx),
                        Embed::None => {}
                    }
                }
                self.at_str = true;
                let e = self.dq_body::<PP>(p + 2);
                self.at_str = false;
                return e;
            }
            Some(b'[') => {
                self.push(F_PLAIN);
                self.put::<PP>(p + 1, p + 2, BRACKETS_BEGIN);
                return p + 2;
            }
            // a dictionary: its `:`s separators ([`INIT`]'s)
            Some(b'{') => {
                self.pending = PEND_INIT;
                return p + 1;
            }
            _ => {}
        }
        let s = p + 1;
        let mut e = s;
        while e < b.len() && CC[usize::from(b[e])] & NW != 0 {
            e += 1;
        }
        let id = AT_WORDS.get(&b[s..e]);
        if id == 0 {
            // `@(…)`, `@1`, `@YES`: the `@` plain
            return s;
        }
        let (_, mark, leaf) = AT_KEYWORDS[usize::from(id) - 1];
        self.put::<PP>(p, s, mark);
        self.put::<PP>(s, e, leaf);
        match id {
            AT_INTERFACE | AT_IMPLEMENTATION => return self.objc_decl::<PP>(e),
            AT_PROTOCOL => {
                // `@protocol(Name)` an expression, `@protocol A, B;` a forward declaration
                let q = skip_blanks(b, e);
                if b.get(q) != Some(&b'(') && !b[q..].contains(&b';') {
                    return self.objc_decl::<PP>(e);
                }
            }
            AT_END => {
                (self.head, self.htp, self.br, self.nbr) = (0, 0, 0, 0);
                self.prev = P_STMT;
                self.new_statement();
            }
            AT_PROPERTY_ID => {
                // a property's declaration: no function's name before a macro's `(`
                self.decl = D_DONE;
                return self.property::<PP>(e);
            }
            AT_SELECTOR => return self.selector::<PP>(e),
            _ => {}
        }
        e
    }

    /// `@interface`'s, `@implementation`'s or `@protocol`'s rest from `e`: its name, its super
    /// class after `:`, a category in `(…)`, its protocols in `<…>`, a generic's parameters.
    #[cold]
    fn objc_decl<const PP: bool>(&mut self, e: usize) -> usize {
        let b = self.b;
        let mut q = skip_blanks(b, e);
        let name = word_end(b, q);
        if name > q {
            self.put::<PP>(q, name, TYPE_NAME);
            q = name;
        }
        loop {
            q = skip_blanks(b, q);
            match b.get(q) {
                Some(b':') if b.get(q + 1) != Some(&b':') => {
                    self.put::<PP>(q, q + 1, INHERIT_MARK);
                    let s = skip_blanks(b, q + 1);
                    let w = word_end(b, s);
                    if w > s {
                        self.put::<PP>(s, w, INHERITED);
                    }
                    q = w;
                }
                Some(b'(') => {
                    self.put::<PP>(q, q + 1, GROUP_BEGIN);
                    let s = skip_blanks(b, q + 1);
                    let w = skip_blanks(b, word_end(b, s));
                    if b.get(w) != Some(&b')') {
                        q = w;
                        break;
                    }
                    self.put::<PP>(w, w + 1, GROUP_END);
                    q = w + 1;
                }
                Some(b'<') => q = self.protocols::<PP>(q),
                _ => break,
            }
        }
        (self.prev, self.decl, self.kw) = (P_STMT, D_NONE, false);
        q
    }

    /// A protocol list's `<` at `q` (`<NSCopying, NSCoding>`, a generic's parameters): its names
    /// plain, to its `>` on the line; where it ended.
    pub(super) fn protocols<const PP: bool>(&mut self, mut q: usize) -> usize {
        let b = self.b;
        self.put::<PP>(q, q + 1, SCOPE_BEGIN);
        q += 1;
        loop {
            q = skip_blanks(b, q);
            match b.get(q) {
                Some(b'>') => {
                    self.put::<PP>(q, q + 1, SCOPE_END);
                    return q + 1;
                }
                Some(b',') => {
                    self.put::<PP>(q, q + 1, SEPARATOR);
                    q += 1;
                }
                Some(&c) if CC[usize::from(c)] & NS0 != 0 => q = word_end(b, q),
                _ => return q,
            }
        }
    }

    /// `@property`'s rest from `e`: its attributes in `(…)`; the declaration after them code.
    #[cold]
    fn property<const PP: bool>(&mut self, e: usize) -> usize {
        let b = self.b;
        let q = skip_blanks(b, e);
        if b.get(q) != Some(&b'(') {
            return e;
        }
        self.put::<PP>(q, q + 1, SCOPE_BEGIN);
        let mut q = q + 1;
        while q < b.len() {
            let c = b[q];
            match c {
                b')' => {
                    self.put::<PP>(q, q + 1, SCOPE_END);
                    return q + 1;
                }
                b',' => {
                    self.put::<PP>(q, q + 1, SEPARATOR);
                    q += 1;
                }
                b'=' => {
                    self.put::<PP>(q, q + 1, ASSIGN);
                    q += 1;
                }
                b':' => {
                    // a setter's name: `setter=setOn:`
                    q += 1;
                }
                _ if CC[usize::from(c)] & NS0 != 0 => {
                    let w = word_end(b, q);
                    if property_attribute(&b[q..w]) {
                        self.put::<PP>(q, w, PROP_ATTR);
                    }
                    q = w;
                }
                _ if CC[usize::from(c)] & SP != 0 => q += 1,
                // anything else ends the attributes: the loop reads it
                _ => return q,
            }
        }
        q
    }

    /// `@selector`'s rest from `e`: `(name:part:)`, its name one token.
    #[cold]
    fn selector<const PP: bool>(&mut self, e: usize) -> usize {
        let b = self.b;
        let q = skip_blanks(b, e);
        if b.get(q) != Some(&b'(') {
            return e;
        }
        self.put::<PP>(q, q + 1, SELECTOR_PAREN);
        let s = skip_blanks(b, q + 1);
        let mut w = s;
        while w < b.len() && (CC[usize::from(b[w])] & NW != 0 || b[w] == b':') {
            w += 1;
        }
        if w > s {
            self.put::<PP>(s, w, SELECTOR_PART);
        }
        let c = skip_blanks(b, w);
        if b.get(c) == Some(&b')') {
            self.put::<PP>(c, c + 1, SELECTOR_PAREN);
            return c + 1;
        }
        c
    }

    /// Whether the `<` at `p` opens Objective-C's lightweight generic: right after a capitalized
    /// name (`NSArray<`, `__kindof`'s too), closing on its line by C++'s generic's shape.
    #[cold]
    pub(super) fn objc_generic(&self, p: usize) -> bool {
        let b = self.b;
        if p == 0 || CC[usize::from(b[p - 1])] & NW == 0 {
            return false;
        }
        let mut s = p - 1;
        while s > 0 && CC[usize::from(b[s - 1])] & NW != 0 {
            s -= 1;
        }
        CC[usize::from(b[s])] & UP != 0 && generic_end(b, p)
    }

    /// A name `p..e` no keyword table has, in Objective-C: Cocoa's word's entry
    /// ([`words::COCOA_INFO`]), or 0.
    #[inline(always)]
    pub(super) fn cocoa(&self, p: usize, e: usize) -> u16 {
        let w = &self.b[p..e];
        if !matches!(w[0], b'N' | b'C') {
            return 0;
        }
        let id = COCOA_WORDS.get(w);
        // SAFETY: an id is at most the table's words, the row's length less one
        let leaf = unsafe { *COCOA_INFO.get_unchecked(usize::from(id)) };
        debug_assert!(leaf >> ACT == 0);
        leaf
    }
}

/// Whether Objective-C's generic at `lt` closes on its line: names, blanks, `*`, `,` and nested
/// `<…>` (`NSDictionary<NSString *, NSArray<id> *>`) to its `>`.
fn generic_end(b: &[u8], lt: usize) -> bool {
    let mut depth = 0u32;
    for &c in &b[lt..] {
        match c {
            b'<' => depth += 1,
            b'>' => {
                depth -= 1;
                if depth == 0 {
                    return true;
                }
            }
            b'*' | b',' | b' ' | b'\t' => {}
            _ if CC[usize::from(c)] & NW != 0 => {}
            _ => return false,
        }
    }
    false
}

/// Whether a `[` whose text starts at `q`, after a `)`, is a send: a name (or a member's chain, or
/// another `[`), then a blank and a name, as bat's send has it.
fn send_after(b: &[u8], q: usize) -> bool {
    let q = skip_blanks(b, q);
    match b.get(q) {
        Some(b'[') => return true,
        Some(&c) if CC[usize::from(c)] & NS0 != 0 => {}
        _ => return false,
    }
    let mut e = q;
    loop {
        e = word_end(b, e);
        match b.get(e) {
            Some(b'.') => e += 1,
            Some(b'-') if b.get(e + 1) == Some(&b'>') => e += 2,
            _ => break,
        }
    }
    let n = skip_blanks(b, e);
    n > e && b.get(n).is_some_and(|&c| CC[usize::from(c)] & NS0 != 0)
}

/// Where the name at `q` ends (`q` where none starts there).
fn word_end(b: &[u8], q: usize) -> usize {
    if q >= b.len() || CC[usize::from(b[q])] & NS0 == 0 {
        return q;
    }
    let mut e = q + 1;
    while e < b.len() && CC[usize::from(b[e])] & NW != 0 {
        e += 1;
    }
    e
}
