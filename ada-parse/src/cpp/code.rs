//! C++'s and C's expression contexts on a loop of their own (X-057): a block, a function's
//! body, parentheses, brackets, a call's braces, C++'s and C's. These hold most of a file's
//! turns (two in three on the corpus), and bat's grammar gives each the same long chain of
//! included groups (`statements`, `expressions` and its parts) tried in order at every place.
//! Here a turn reads the byte's class once and takes the token the chain's first pattern would:
//! a blank run passed whole, the context's closer, a name read once and looked up once (a
//! keyword's leaf, a call, a plain name), a plain number, an operator, a member after `.` or
//! `->`, a comment's or a string's opener. The output is the compiled contexts' byte for byte:
//! a turn the loop doesn't know (a lookahead's push, a name past ASCII, a rare keyword, the
//! line's start) goes back to [`Scan::run`], which takes it on the context's arm at the same
//! place, so the frames are the contexts and share their ids.

use super::text::{ascii_word_end, at, find_any, is_kw, next, starts};
use super::words::{
    Kw, c_value, is_c_control, is_c_modifier, is_c_type, lookup_inline as lookup, support,
};
use super::*;
use crate::clike::{CLASS, DIGIT, HIGH, LOWER, SPACE, UPPER, WORD};
use crate::skip_spaces;

/// The classes a name starts with: a letter or `_` (`WORD` but a digit).
const NAME_START: u8 = UPPER | LOWER;

/// What [`Scan::code_run`] does with a context on top: none of its turns, an expression's
/// (C++'s or C's), a parameter list's, a comment's or a string's.
const NOT: u8 = 0;
const CXX_EXPR: u8 = 1;
const C_EXPR: u8 = 2;
const PARAMS: u8 = 3;
const TEXT: u8 = 4;
/// `ANGLE`'s: `expressions` without `generic-type` or `function-call`.
const CXX_ANGLE: u8 = 5;
/// The top level's and a namespace's statements (`MAIN`, `NS_BLOCK`): `global`.
const GLOBAL: u8 = 6;
/// A declaration's head ([`super::decl`]).
const DECL: u8 = 7;
/// C's top level's statements (`C_MAIN`): its `global`.
const C_GLOBAL: u8 = 8;

/// A frame's turn's position bit for a turn that set [`Scan::stuck`] itself: a miss (it
/// stays), or a match and a pop that took nothing after it.
pub(super) const KEEP: usize = 1 << (usize::BITS - 1);

/// A frame's kind and closer, by context id: what [`Scan::code_run`] reads once a turn.
#[derive(Clone, Copy)]
struct Frame {
    kind: u8,
    /// An `#if` branch's bytes of its own ([`F_BRANCH_OPEN`], [`F_NEGATED`], [`F_PP_CALL`]).
    flags: u8,
    /// The byte that closes it, and its leaf ([`BRANCH`] for a branch's block, whose `}` sets
    /// the branch back); `0` for a frame closed otherwise (a parameter list's `)` sets what
    /// follows it, a comment's or a string's end is its turn's).
    close: u8,
    leaf: u16,
}

/// What [`Scan::branch_byte`] makes of a byte.
enum Branch {
    /// The arm's turn.
    Arm,
    /// A match, where the scan goes on.
    Took(usize),
    /// The frame's own turn.
    Turn,
}

/// An `#if` branch's `{`, which opens the branch's block in its place.
const F_BRANCH_OPEN: u8 = 1;
/// An `#if` branch's `}`, which opens a negated block: the arm's.
const F_NEGATED: u8 = 2;
/// A statements' branch, where a name before a call is `preprocessor-if-branch-function-call`'s:
/// the arm's.
const F_PP_CALL: u8 = 4;

/// A frame's leaf bit for a branch's block, whose `}` sets the branch back.
const BRANCH: u16 = 1 << 15;

/// The frames by context id.
static FRAMES: [Frame; 256] = {
    let mut t = [Frame {
        kind: NOT,
        flags: 0,
        close: 0,
        leaf: 0,
    }; 256];
    t[BLOCK as usize] = Frame {
        kind: CXX_EXPR,
        flags: 0,
        close: b'}',
        leaf: BLOCK_END,
    };
    t[FN_BLOCK as usize] = Frame {
        kind: CXX_EXPR,
        flags: 0,
        close: b'}',
        leaf: BLOCK_END,
    };
    t[PAREN as usize] = Frame {
        kind: CXX_EXPR,
        flags: 0,
        close: b')',
        leaf: GROUP_END,
    };
    t[BRACKETS as usize] = Frame {
        kind: CXX_EXPR,
        flags: 0,
        close: b']',
        leaf: BRACKETS_END,
    };
    t[BRACE_EXPR as usize] = Frame {
        kind: CXX_EXPR,
        flags: 0,
        close: b'}',
        leaf: GROUP_END,
    };
    t[ENUM_BODY as usize] = Frame {
        kind: CXX_EXPR,
        flags: 0,
        close: b'}',
        leaf: BLOCK_END,
    };
    t[FRIEND_BLOCK as usize] = Frame {
        kind: CXX_EXPR,
        flags: 0,
        close: b'}',
        leaf: BLOCK_END,
    };
    t[BLOCK_IF_G as usize] = Frame {
        kind: CXX_EXPR,
        flags: 0,
        close: b'}',
        leaf: BRANCH,
    };
    t[BLOCK_IF_S as usize] = Frame {
        kind: CXX_EXPR,
        flags: 0,
        close: b'}',
        leaf: BRANCH,
    };
    t[ANGLE as usize] = Frame {
        kind: CXX_ANGLE,
        flags: 0,
        close: b'>',
        leaf: GENERIC_END,
    };
    t[C_BLOCK as usize] = Frame {
        kind: C_EXPR,
        flags: 0,
        close: b'}',
        leaf: BLOCK_END | C,
    };
    t[C_FN_BLOCK as usize] = Frame {
        kind: C_EXPR,
        flags: 0,
        close: b'}',
        leaf: BLOCK_END | C,
    };
    t[C_PAREN as usize] = Frame {
        kind: C_EXPR,
        flags: 0,
        close: b')',
        leaf: GROUP_END | C,
    };
    t[C_BRACKETS as usize] = Frame {
        kind: C_EXPR,
        flags: 0,
        close: b']',
        leaf: BRACKETS_END | C,
    };
    t[C_BLOCK_IF_G as usize] = Frame {
        kind: C_EXPR,
        flags: 0,
        close: b'}',
        leaf: BRANCH,
    };
    t[C_BLOCK_IF_S as usize] = Frame {
        kind: C_EXPR,
        flags: 0,
        close: b'}',
        leaf: BRANCH,
    };
    t[FN_PARAMS_IN as usize] = Frame {
        kind: PARAMS,
        flags: 0,
        close: 0,
        leaf: 0,
    };
    t[METHOD_PARAMS_IN as usize] = Frame {
        kind: PARAMS,
        flags: 0,
        close: 0,
        leaf: 0,
    };
    t[C_FN_PARAMS_IN as usize] = Frame {
        kind: PARAMS,
        flags: 0,
        close: 0,
        leaf: 0,
    };
    t[C_LINE_COMMENT as usize] = Frame {
        kind: TEXT,
        flags: 0,
        close: 0,
        leaf: 0,
    };
    t[C_COMMENT as usize] = Frame {
        kind: TEXT,
        flags: 0,
        close: 0,
        leaf: 0,
    };
    t[C_STRING as usize] = Frame {
        kind: TEXT,
        flags: 0,
        close: 0,
        leaf: 0,
    };
    t[C_CHAR as usize] = Frame {
        kind: TEXT,
        flags: 0,
        close: 0,
        leaf: 0,
    };
    t[MAIN as usize] = Frame {
        kind: GLOBAL,
        flags: 0,
        close: 0,
        leaf: 0,
    };
    t[NS_BLOCK as usize] = Frame {
        kind: GLOBAL,
        flags: 0,
        close: 0,
        leaf: 0,
    };
    t[C_MAIN as usize] = Frame {
        kind: C_GLOBAL,
        flags: 0,
        close: 0,
        leaf: 0,
    };
    let open = F_BRANCH_OPEN | F_NEGATED;
    t[IF_G as usize] = Frame {
        kind: GLOBAL,
        flags: open,
        close: 0,
        leaf: 0,
    };
    t[ELIF_G as usize] = Frame {
        kind: GLOBAL,
        flags: F_NEGATED,
        close: 0,
        leaf: 0,
    };
    t[C_IF_G as usize] = Frame {
        kind: C_GLOBAL,
        flags: open,
        close: 0,
        leaf: 0,
    };
    t[C_ELIF_G as usize] = Frame {
        kind: C_GLOBAL,
        flags: F_NEGATED,
        close: 0,
        leaf: 0,
    };
    t[IF_S as usize] = Frame {
        kind: CXX_EXPR,
        flags: open | F_PP_CALL,
        close: 0,
        leaf: 0,
    };
    t[ELIF_S as usize] = Frame {
        kind: CXX_EXPR,
        flags: F_NEGATED,
        close: 0,
        leaf: 0,
    };
    t[C_IF_S as usize] = Frame {
        kind: C_EXPR,
        flags: open | F_PP_CALL,
        close: 0,
        leaf: 0,
    };
    t[C_ELIF_S as usize] = Frame {
        kind: C_EXPR,
        flags: F_NEGATED,
        close: 0,
        leaf: 0,
    };
    let decls = [
        G_MODIFIER,
        G_TYPE,
        G_MAYBE_FN,
        G_FN_ID,
        FN_PARAMS,
        FN_CONTINUE,
        IDENT_POP,
        C_G_MODIFIER,
        C_G_TYPE,
        C_G_MAYBE_FN,
        C_FN_PARAMS,
        C_FN_CONTINUE,
    ];
    let mut i = 0;
    while i < decls.len() {
        t[decls[i] as usize] = Frame {
            kind: DECL,
            flags: 0,
            close: 0,
            leaf: 0,
        };
        i += 1;
    }
    t
};

/// Whether `c` is one of the frames [`Scan::code_run`] takes turns in.
#[inline(always)]
pub(super) fn is_frame(c: u8) -> bool {
    FRAMES[usize::from(c)].kind != NOT
}

/// Whether `c` is a frame whose line is done at its end: those that are
/// [`cxx::spaces_inert`], and the global and declaration frames whose arms match nothing
/// there (no comment or string, which leave that to their turns).
#[inline(always)]
pub(super) fn ends_at_eol(c: u8) -> bool {
    matches!(
        FRAMES[usize::from(c)].kind,
        CXX_EXPR | C_EXPR | PARAMS | CXX_ANGLE | GLOBAL | C_GLOBAL
    ) || matches!(
        c,
        G_MODIFIER
            | G_MAYBE_FN
            | G_FN_ID
            | FN_PARAMS
            | FN_CONTINUE
            | C_G_MODIFIER
            | C_G_MAYBE_FN
            | C_FN_PARAMS
            | C_FN_CONTINUE
    )
}

impl<K: Sink> Scan<'_, '_, K> {
    /// The frames' turns from `p`: while a frame is on top and its turn is one of the common
    /// ones. Where [`Scan::run`] goes on: the line's end with an expression's or a parameter
    /// list's frame on top (each [`cxx::spaces_inert`], so the line is done), a comment's or a
    /// string's that took the line's end ([`Scan::halt`]), or a place another context is on top
    /// of, or whose turn the context's arm takes.
    pub(super) fn code_run(&mut self, p: usize) -> usize {
        self.frames(p)
    }

    /// [`Scan::code_run`]'s turns, while a frame is on top.
    #[inline(always)]
    fn frames(&mut self, mut p: usize) -> usize {
        let b = self.b;
        loop {
            let top = self.top();
            let frame = FRAMES[usize::from(top)];
            let kind = frame.kind;
            if kind == NOT {
                return p;
            }
            if kind == TEXT {
                match self.text_turn(p, top) {
                    Some(q) => {
                        p = self.matched(p, q);
                        if self.halt {
                            return p;
                        }
                        continue;
                    }
                    None => return p,
                }
            }
            if kind == DECL {
                let q = if top >= C_FIRST {
                    self.c_decl_turn(p, top)
                } else {
                    self.decl_turn(p, top)
                };
                match q {
                    Some(q) => {
                        p = self.matched(p, q);
                        continue;
                    }
                    None => return p,
                }
            }
            let Some(&c0) = b.get(p) else {
                return p;
            };
            if p == 0 && (kind == GLOBAL || kind == C_GLOBAL) {
                // a namespace's `}` first, then the line's start
                let q = if top == NS_BLOCK && c0 == b'}' {
                    self.can_pop(0).then(|| self.leave(0))
                } else if kind == C_GLOBAL {
                    self.c_global_start()
                } else {
                    self.global_start()
                };
                match q {
                    Some(q) => {
                        p = self.matched(0, q);
                        continue;
                    }
                    // a byte at 0 no name starts takes the turn past the line's start would (but
                    // a comment's banner, which only 0 has)
                    None if self.dir.is_none()
                        && CLASS[usize::from(c0)] & (WORD | HIGH | SPACE) == 0
                        && !starts(b, 0, b"// =")
                        && !starts(b, 0, b"/* =") => {}
                    None => return 0,
                }
            } else if p == 0
                && CLASS[usize::from(c0)] & SPACE != 0
                && self.dir.is_none()
                && self.blanks_miss()
            {
                // blanks before a byte no line-start pattern takes: the arm's miss passes them
                p = self.lead;
                continue;
            }
            if p == 0 && kind != GLOBAL && kind != C_GLOBAL {
                // the line's start, where `^\s*` patterns may start: only a closer at once
                // (a directive's line has a `#` or a blank first)
                match self.closer_at_start(c0, top) {
                    Some(q) => {
                        p = self.matched(0, q);
                        continue;
                    }
                    None => return p,
                }
            }
            let (c, k) = if CLASS[usize::from(c0)] & SPACE != 0 {
                // no pattern of a frame starts at a blank past the line's start
                p = crate::clike::skip_blanks(b, p);
                let Some(&d) = b.get(p) else {
                    return p;
                };
                (d, CLASS[usize::from(d)])
            } else {
                (c0, CLASS[usize::from(c0)])
            };
            if frame.flags != 0 {
                match self.branch_byte(p, c, k, top, frame.flags) {
                    Branch::Arm => return p,
                    Branch::Took(q) => {
                        p = self.matched(p, q);
                        continue;
                    }
                    Branch::Turn => {}
                }
            }
            let q = match kind {
                CXX_EXPR => self.cxx_turn(p, c, k, top, true, false),
                CXX_ANGLE => self.cxx_turn(p, c, k, top, false, false),
                GLOBAL => {
                    if top == NS_BLOCK && c == b'}' {
                        self.can_pop(p).then(|| self.leave(p))
                    } else {
                        self.cxx_turn(p, c, k, top, true, true)
                    }
                }
                C_EXPR | C_GLOBAL => self.c_turn(p, c, k, top),
                _ => self.params_turn(p, c, k, top),
            };
            // an expression's turn always takes something: no match that took nothing to count
            // (but a namespace's `}`, a pop at it)
            match q {
                Some(q) if kind == GLOBAL && top == NS_BLOCK => p = self.matched(p, q),
                Some(q) if q & KEEP != 0 => p = q & !KEEP,
                Some(q) => {
                    debug_assert!(q > p);
                    self.stuck = 0;
                    p = q;
                }
                None => return p,
            }
        }
    }

    /// An `#if` branch's own bytes at `p` ([`Frame::flags`]): its `{`, which opens the branch's
    /// block; its `}` and a name before a call, the arm's; else the frame's turn.
    #[cold]
    #[inline(never)]
    fn branch_byte(&mut self, p: usize, c: u8, k: u8, top: u8, flags: u8) -> Branch {
        match c {
            b'}' if flags & F_NEGATED != 0 => Branch::Arm,
            b'{' if flags & F_BRANCH_OPEN != 0 => {
                let (block, lang) = match top {
                    IF_G => (BLOCK_IF_G, 0),
                    IF_S => (BLOCK_IF_S, 0),
                    C_IF_G => (C_BLOCK_IF_G, C),
                    _ => (C_BLOCK_IF_S, C),
                };
                Branch::Took(self.then(p, 1, &[block], BLOCK_BEGIN | lang))
            }
            _ if flags & F_PP_CALL != 0
                && (k & (NAME_START | DIGIT) != 0 || c == b'_' || c == b':')
                && self.pp_call_ahead(p, top >= C_FIRST) =>
            {
                Branch::Arm
            }
            _ => Branch::Turn,
        }
    }

    /// The context's text up to `e`, within the line: [`Scan::fill`] inlined in a frame's turn.
    #[inline(always)]
    fn text_to(&mut self, e: usize) {
        if e > self.at {
            if self.text & !C != PLAIN {
                self.k.merge(&mut self.out, self.at, e, self.text);
            }
            self.at = e;
        }
    }

    /// Whether a frame's line that starts with blanks (no directive) has none of the line's
    /// start's patterns at its first byte past them: not a name a label's `name:` or the
    /// upper-case workarounds may read, nor a byte past ASCII. Every frame but the global
    /// ones misses its blanks then (`statements`' and `expressions`' line-start patterns).
    #[inline(always)]
    fn blanks_miss(&self) -> bool {
        let b = self.b;
        let q = self.lead;
        let Some(&c) = b.get(q) else {
            return true;
        };
        if c >= 0x80 {
            return false;
        }
        if !(c.is_ascii_alphabetic() || c == b'_') {
            return true;
        }
        let a = ascii_word_end(b, q + 1);
        if a < b.len() && (b[a] >= 0x80 || b[a] == b':') {
            return false;
        }
        !((c.is_ascii_uppercase() || c == b'_')
            && (self.upper_ident_line() || self.upper_call().is_some()))
    }

    /// [`Scan::stuck`] after a frame's turn from `p` to `q`, as [`Scan::run`] has it after a
    /// match (one more where it took nothing, else none), unless the turn set it itself and
    /// said so in `q` ([`KEEP`]); where the scan goes on.
    #[inline(always)]
    fn matched(&mut self, p: usize, q: usize) -> usize {
        if q & KEEP != 0 {
            return q & !KEEP;
        }
        self.stuck = if q == p { self.stuck + 1 } else { 0 };
        q
    }

    /// An expression's or a parameter list's frame at the line's start, `c` its first byte: the
    /// frame's closer, which every one of them tries before a pattern of the line's start.
    #[inline(always)]
    fn closer_at_start(&mut self, c: u8, top: u8) -> Option<usize> {
        // a branch's block tries its directives first, which need a `#` or a blank first
        self.closer(0, c, top)
    }

    /// The frame's closer at `p`, `c` its byte: a closing bracket that pops it, or a branch's
    /// block's `}` that sets its branch back ([`FRAMES`]).
    #[inline(always)]
    fn closer(&mut self, p: usize, c: u8, top: u8) -> Option<usize> {
        let Frame { close, leaf, .. } = FRAMES[usize::from(top)];
        if c != close {
            return None;
        }
        if leaf & BRANCH == 0 {
            return Some(self.close(p, 1, leaf));
        }
        let (branch, lang) = match top {
            BLOCK_IF_G => (IF_G, 0),
            BLOCK_IF_S => (IF_S, 0),
            C_BLOCK_IF_G => (C_IF_G, C),
            _ => (C_IF_S, C),
        };
        Some(self.then(p, 1, &[branch], BLOCK_END | lang))
    }

    /// Bytes `s..e` are `leaf`, after the context's text before them: [`Scan::put`] where `s`
    /// is at or past the spans' end and `e` past `s` and no further than the line, as every
    /// frame's token is.
    #[inline(always)]
    fn tok(&mut self, s: usize, e: usize, leaf: u16) -> usize {
        debug_assert!(self.at <= s && s < e && e <= self.b.len());
        // the context's text before it: `fill`, `s` within the line
        if s > self.at {
            if self.text & !C != PLAIN {
                self.k.merge(&mut self.out, self.at, s, self.text);
            }
            self.at = s;
        }
        let k = self.kind(leaf);
        self.k.merge(&mut self.out, s, e, k);
        self.at = e;
        e
    }

    /// A C++ decimal or hexadecimal integer at `p`, a digit at a word's start, with nothing after
    /// its digits that `numbers` would read on (a float's `.`, an exponent, a suffix, a `'`,
    /// another base): its spans, and no `NUM_DEC` or `NUM_HEX` pushed, as `numbers` has it;
    /// `None` for `numbers`.
    #[inline(always)]
    fn cxx_decimal(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if c == b'0' && matches!(at(b, p + 1), b'x' | b'X') && p + 1 < b.len() {
            // `numbers`' hexadecimal integer that pushes no `NUM_HEX`: digits and `'`s, then
            // nothing a name or a float takes
            let mut q = p + 2;
            while q < b.len() && (b[q].is_ascii_hexdigit() || b[q] == b'\'') {
                q += 1;
            }
            if q == p + 2
                || (q < b.len() && (CLASS[usize::from(b[q])] & (WORD | HIGH) != 0 || b[q] == b'.'))
                || self.depth >= MAX
            {
                return None;
            }
            self.tok(p, p + 2, HEX_BASE);
            return Some(self.tok(p + 2, q, HEX));
        }
        let mut e = p + 1;
        while e < b.len() && b[e].is_ascii_digit() {
            e += 1;
        }
        if (c == b'0' && e > p + 1)
            || (e < b.len()
                && (CLASS[usize::from(b[e])] & (WORD | HIGH) != 0 || matches!(b[e], b'.' | b'\'')))
            || self.depth >= MAX
        {
            return None;
        }
        Some(self.tok(p, e, DEC))
    }

    /// Whether the byte before `p` is a word's (or past ASCII, maybe one): a name or a digit
    /// there doesn't start at a `\b`, which the frames leave to the arms.
    #[inline(always)]
    pub(super) fn inside_word(&self, p: usize) -> bool {
        p > 0 && CLASS[usize::from(self.b[p - 1])] & (WORD | HIGH) != 0
    }

    /// A comment's or a string's turn at `p` ([`C_LINE_COMMENT`], [`C_COMMENT`], [`C_STRING`],
    /// [`C_CHAR`]): the arm's match, else its advance to where a pattern can start, the turns
    /// that match nothing between taken at once. `None` at the line's end of a block comment or
    /// a string, whose arm takes nothing there, so the line is done ([`Scan::halt`] set, as no
    /// pattern is left); else where the scan goes on.
    #[inline(always)]
    fn text_turn(&mut self, mut p: usize, top: u8) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        match top {
            C_LINE_COMMENT => {
                // the line's last byte a `\` is its continuation; nothing else matches before
                // the line's end, which pops the comment
                if n > 0 && b[n - 1] == b'\\' && p < n {
                    if p + 1 < n {
                        return Some(n - 1);
                    }
                    self.put(p, n, LINE_CONT | C);
                } else {
                    self.pop(n);
                }
                self.halt = true;
                Some(n)
            }
            C_COMMENT => {
                if p == 0 {
                    let q = self.lead;
                    let c = at(b, 0);
                    if !(c == b'*' && at(b, 1) == b'/')
                        && at(b, q) == b'*'
                        && q < n
                        && at(b, q + 1) != b'/'
                    {
                        return Some(self.tok(q, q + 1, COMMENT_MARK | C));
                    }
                }
                loop {
                    if p >= n {
                        self.halt = true;
                        return Some(n);
                    }
                    if b[p] == b'*' && at(b, p + 1) == b'/' {
                        return Some(self.close(p, 2, COMMENT_MARK | C));
                    }
                    p = find_any(b, p + 1, *b"*");
                }
            }
            _ => {
                let dq = top == C_STRING;
                loop {
                    if p >= n {
                        self.halt = true;
                        return Some(n);
                    }
                    let c = b[p];
                    let quote = if dq { b'"' } else { b'\'' };
                    if c == quote {
                        return Some(self.close(p, 1, if dq { DQ_END } else { SQ_END } | C));
                    }
                    if c == b'\\' {
                        if p + 1 == n {
                            self.put(p, n, if dq { DQ_CONT } else { SQ_CONT } | C);
                            self.halt = true;
                            return Some(n);
                        }
                        let (leaf, e) = match c::escape_end(b, p) {
                            Some(e) => (if dq { DQ_ESCAPE } else { SQ_ESCAPE }, e),
                            None => (
                                if dq { DQ_BAD_ESCAPE } else { SQ_BAD_ESCAPE },
                                next(b, p + 1),
                            ),
                        };
                        return Some(self.token(p, e - p, leaf | C));
                    }
                    if c == b'%'
                        && dq
                        && let Some(e) = c::placeholder(b, p)
                    {
                        return Some(self.token(p, e - p, DQ_PLACEHOLDER | C));
                    }
                    p = if dq {
                        find_any(b, p + 1, *b"\"\\%")
                    } else {
                        find_any(b, p + 1, *b"'\\")
                    };
                }
            }
        }
    }

    /// A parameter list's turn at `p` past the line's start (`FN_PARAMS_IN`, `METHOD_PARAMS_IN`,
    /// C's `C_FN_PARAMS_IN`), a byte `c` that isn't a blank: its `)`, `void`, a parameter's
    /// name before `[`, `,`, `)` (C++'s `=` too), C++'s default value's `=`, else
    /// `expressions`' (with `angle-brackets`, not `generic-type`); `None` for the arm's.
    #[inline(always)]
    fn params_turn(&mut self, p: usize, c: u8, k: u8, top: u8) -> Option<usize> {
        let b = self.b;
        let cl = top == C_FN_PARAMS_IN;
        let lang = if cl { C } else { 0 };
        if c == b')' {
            let next = match top {
                FN_PARAMS_IN => FN_CONTINUE,
                METHOD_PARAMS_IN => METHOD_CONTINUE,
                _ => C_FN_CONTINUE,
            };
            return Some(self.then(p, 1, &[next], GROUP_END | lang));
        }
        if k & NAME_START != 0 || c == b'_' {
            if self.inside_word(p) {
                return None;
            }
            let e = ascii_word_end(b, p + 1);
            if e < b.len() && b[e] >= 0x80 {
                return None;
            }
            if is_kw(b, p, b"void") {
                return Some(self.tok(p, p + 4, STORAGE_TYPE | lang));
            }
            let q = skip_spaces(b, e);
            if q < b.len() && matches!(b[q], b'[' | b',' | b')')
                || (!cl && at(b, q) == b'=' && q < b.len())
            {
                return Some(self.tok(p, e, PARAMETER | lang));
            }
            return if cl {
                self.c_word(p)
            } else {
                self.cxx_name(p, true)
            };
        }
        if c == b'=' && !cl {
            let d = if top == FN_PARAMS_IN {
                FN_PARAM_DEFAULT
            } else {
                METHOD_PARAM_DEFAULT
            };
            self.put(p, p + 1, ASSIGN);
            self.push(p + 1, d);
            return Some(p + 1);
        }
        if cl {
            // C's `expressions` as its parentheses have them, the `)` taken above
            return self.c_turn(p, c, k, C_PAREN);
        }
        // `angle-brackets` in place of `generic-type`: a `<` is the arm's
        if c == b'<' || c == b'}' || c == b']' {
            return None;
        }
        self.cxx_turn(p, c, k, PAREN, true, false)
    }

    /// A C++ frame's turn at `p`, a byte `c` that isn't a blank: the context's closer, else
    /// `statements`' and `expressions`' first pattern that takes it past the line's start.
    #[inline(always)]
    fn cxx_turn(
        &mut self,
        p: usize,
        c: u8,
        k: u8,
        top: u8,
        call: bool,
        global: bool,
    ) -> Option<usize> {
        let b = self.b;
        if let Some(q) = self.closer(p, c, top) {
            return Some(q);
        }
        if k & NAME_START != 0 || c == b'_' {
            if self.inside_word(p) {
                return None;
            }
            // a function's body looks for a data structure's keyword first
            if top == FN_BLOCK && matches!(c, b's' | b'u' | b'e' | b'c') && self.ds_ahead(p) {
                return None;
            }
            // `global`'s own words: `template`, `namespace`
            if global && matches!(c, b't' | b'n') {
                let e = ascii_word_end(b, p + 1);
                if matches!(&b[p..e], b"template" | b"namespace") {
                    return None;
                }
            }
            return self.cxx_name(p, call);
        }
        if k & DIGIT != 0 {
            if self.inside_word(p) {
                return None;
            }
            return Some(self.cxx_decimal(p, c))
                .flatten()
                .or_else(|| self.numbers(p, c));
        }
        let d = at(b, p + 1);
        let r = match c {
            b'(' => self.open(p, 1, PAREN, GROUP_BEGIN),
            b'[' => self.open(p, 1, BRACKETS, BRACKETS_BEGIN),
            b'{' => self.open(p, 1, BLOCK, BLOCK_BEGIN),
            b',' => self.tok(p, p + 1, SEPARATOR),
            b';' => self.tok(p, p + 1, TERMINATOR),
            b')' | b'}' => self.tok(p, p + 1, STRAY_BRACKET),
            b'.' => {
                if d.is_ascii_digit() {
                    // a float's `.5`
                    return None;
                }
                if d == b'.' {
                    if at(b, p + 2) == b'.' {
                        self.tok(p, p + 3, VARIADIC | C)
                    } else {
                        self.tok(p, p + 2, ILLEGAL_SYNTAX | C)
                    }
                } else {
                    let m = self.open(p, 1, MEMBERS, DOT);
                    return Some(self.member(m));
                }
            }
            b'-' if d == b'>' => {
                let m = self.open(p, 2, MEMBERS, ARROW);
                return Some(self.member(m));
            }
            b'/' => match d {
                b'/' => self.open(p, 2, C_LINE_COMMENT, LINE_MARK | C),
                // `global`'s comment is a context of its own around it
                b'*' if global => return None,
                b'*' => self.open(p, 2, C_COMMENT, COMMENT_MARK | C),
                _ => return self.operators(p, c),
            },
            b'*' if d == b'/' && at(b, p + 2) != b'*' => self.tok(p, p + 2, STRAY_COMMENT_END | C),
            b'<' if d == b'=' => self.tok(p, p + 2, COMPARISON | C),
            // `angle-brackets` in place of `generic-type`: its `<` is the arm's
            b'<' if !call => return None,
            // `::` starts a path (`generic-type`, `identifiers`)
            b':' if d == b':' => return None,
            b'"' | b'\'' => return self.c_strings(p, c),
            _ => return self.operators(p, c),
        };
        Some(r)
    }

    /// An ASCII name at `p` in a C++ frame, at a word's start: `expressions`' first pattern
    /// for it, as [`Scan::name`] has them with `function-call`; `None` for the arm's.
    #[inline(always)]
    fn cxx_name(&mut self, p: usize, call: bool) -> Option<usize> {
        let b = self.b;
        let e = ascii_word_end(b, p + 1);
        if e < b.len() && b[e] >= 0x80 {
            return None;
        }
        let q = skip_spaces(b, e);
        let open = at(b, q);
        if open == b':' && at(b, q + 1) == b':' {
            return if call {
                self.qualified_call(p, e, q)
            } else {
                None
            };
        }
        if matches!(open, b'<' | b':' | b'"' | b'\'') && q < b.len() {
            return None;
        }
        let kw = lookup(&b[p..e]);
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
            return Some(self.tok(p, e, leaf));
        }
        if !(q < b.len() && matches!(open, b'(' | b'{')) {
            self.text_to(e);
            return Some(e);
        }
        if !call || self.stuck >= STUCK || starts(b, p, b"template") {
            return None;
        }
        self.put(p, e, if kw == Kw::C99 { C99 | C } else { FUNCTION });
        self.put(q, q + 1, GROUP_BEGIN);
        self.push(q + 1, if open == b'(' { PAREN } else { BRACE_EXPR });
        Some(q + 1)
    }

    /// `members-after-accessor-junction`'s turn at `p`, just after `.` or `->`, for an ASCII name
    /// right there: a call's name and its group, or a member that pops the context. Where the
    /// scan goes on; `p` itself, the context left on top, for anything else.
    #[inline(always)]
    fn member(&mut self, p: usize) -> usize {
        let b = self.b;
        let Some(&c) = b.get(p) else {
            return p;
        };
        if CLASS[usize::from(c)] & NAME_START == 0 && c != b'_' {
            return p;
        }
        let e = ascii_word_end(b, p + 1);
        if e < b.len() && b[e] >= 0x80 {
            return p;
        }
        let w = &b[p..e];
        if w == b"template" || w == b"operator" {
            return p;
        }
        let s = skip_spaces(b, e);
        match at(b, s) {
            b'(' if s < b.len() => {
                self.put(p, e, MEMBER_FUNCTION);
                self.then(s, 1, &[PAREN], GROUP_BEGIN)
            }
            b'<' if s < b.len() => p,
            b':' if at(b, s + 1) == b':' => p,
            _ => self.close(p, e - p, MEMBER_VAR),
        }
    }

    /// A C frame's turn at `p`, a byte `c` that isn't a blank: the context's closer, else C's
    /// `statements`' and `expressions`' first pattern that takes it past the line's start.
    #[inline(always)]
    fn c_turn(&mut self, p: usize, c: u8, k: u8, top: u8) -> Option<usize> {
        let b = self.b;
        if let Some(q) = self.closer(p, c, top) {
            return Some(q);
        }
        if k & NAME_START != 0 || c == b'_' {
            if self.inside_word(p) {
                return None;
            }
            if top == C_FN_BLOCK && matches!(c, b's' | b'u' | b'e') && c::c_ds_ahead(b, p) {
                return None;
            }
            return self.c_word(p);
        }
        if k & DIGIT != 0 {
            if self.inside_word(p) {
                return None;
            }
            return self.c_decimal(p, c);
        }
        let d = at(b, p + 1);
        let r = match c {
            b'(' => self.open(p, 1, C_PAREN, GROUP_BEGIN | C),
            b'[' => self.open(p, 1, C_BRACKETS, BRACKETS_BEGIN | C),
            b'{' => self.open(p, 1, C_BLOCK, BLOCK_BEGIN | C),
            b',' => self.tok(p, p + 1, SEPARATOR | C),
            b';' => self.tok(p, p + 1, TERMINATOR | C),
            b')' | b'}' => self.tok(p, p + 1, STRAY_BRACKET | C),
            b'.' => {
                if d.is_ascii_digit() {
                    return None;
                }
                if d == b'.' {
                    if at(b, p + 2) == b'.' {
                        self.tok(p, p + 3, VARIADIC | C)
                    } else {
                        self.tok(p, p + 2, ILLEGAL_SYNTAX | C)
                    }
                } else {
                    return self.c_access(p);
                }
            }
            b'/' => match d {
                b'/' => self.open(p, 2, C_LINE_COMMENT, LINE_MARK | C),
                b'*' => self.open(p, 2, C_COMMENT, COMMENT_MARK | C),
                _ => return self.operators(p, c),
            },
            b'*' if d == b'/' && at(b, p + 2) != b'*' => self.tok(p, p + 2, STRAY_COMMENT_END | C),
            b':' if d == b':' => return None,
            b'"' | b'\'' => return self.c_strings(p, c),
            _ => return self.operators(p, c),
        };
        Some(r)
    }

    /// An ASCII name at `p` in a C frame, at a word's start: C's `expressions`' first pattern
    /// for it, as [`Scan::c_name`], `c_early` and `c_late` have them; `None` for the arm's.
    #[inline(always)]
    fn c_word(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let e = ascii_word_end(b, p + 1);
        if e < b.len() && (b[e] >= 0x80 || b[e] == b'"' || b[e] == b'\'') {
            return None;
        }
        let w = &b[p..e];
        let kw = lookup(w);
        if !matches!(kw, Kw::Other | Kw::C99) {
            return self.c_keyword(p, e);
        }
        let q = skip_spaces(b, e);
        if q < b.len() && b[q] == b'(' {
            if self.stuck >= STUCK {
                return None;
            }
            self.put(p, e, if kw == Kw::C99 { C99 } else { FUNCTION } | C);
            self.put(q, q + 1, GROUP_BEGIN | C);
            self.push(q + 1, C_PAREN);
            return Some(q + 1);
        }
        // no pattern takes it: a miss, and the advance past the word, which leaves the matches
        // that took nothing as they are
        self.text_to(e);
        Some(self.miss(e))
    }

    /// A word at `p..e` some list of C++'s holds, in a C frame: C's patterns for it in their
    /// order (`c_early`'s, then `c_late`'s); `None` for the arm's.
    #[inline(never)]
    fn c_keyword(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        let w = &b[p..e];
        let leaf = match w {
            b"case" | b"default" | b"typedef" | b"sizeof" | b"__attribute__" | b"__declspec" => {
                return None;
            }
            b"break" => BREAK_KW,
            b"continue" => CONTINUE_KW,
            b"goto" => GOTO_KW,
            b"return" => RETURN_KW,
            _ if is_c_control(w) => CONTROL,
            _ => {
                if let Some(leaf) = c_value(w) {
                    return Some(self.tok(p, e, leaf));
                }
                if is_c_modifier(w) {
                    MODIFIER
                } else if is_c_type(w) {
                    STORAGE_TYPE
                } else if let Some(leaf) = support(w) {
                    return Some(self.tok(p, e, leaf));
                } else {
                    // C++'s alone: `function-call`'s lookahead before `(`, else a miss
                    let q = skip_spaces(b, e);
                    if q < b.len() && b[q] == b'(' {
                        return None;
                    }
                    self.fill(e);
                    return Some(self.miss(e));
                }
            }
        };
        Some(self.tok(p, e, leaf | C))
    }

    /// A number at `p` in a C frame, a digit at a word's start: a decimal integer's digits with
    /// no suffix, float or other base after them (C's `numbers`, its `C_NUM_DEC` pushed and
    /// popped at once by its empty suffix, a match that takes nothing); `None` for the arm's.
    #[inline(always)]
    fn c_decimal(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if c == b'0' && matches!(at(b, p + 1), b'x' | b'X') && p + 1 < b.len() {
            return self.c_hex(p);
        }
        let mut e = p + 1;
        while e < b.len() && b[e].is_ascii_digit() {
            e += 1;
        }
        if (c == b'0' && e > p + 1)
            || (e < b.len() && (CLASS[usize::from(b[e])] & (WORD | HIGH) != 0 || b[e] == b'.'))
            || self.depth >= MAX
        {
            return None;
        }
        self.put(p, e, DEC | C);
        self.stuck = 1;
        Some(e | KEEP)
    }

    /// C's hexadecimal integer at `p`, `0x` (not a float's): `C_NUM_HEX` opened, its digits
    /// passed (no pattern of it starts at one, the arm's misses a byte a turn), and its first
    /// match after them, a suffix (empty at the word's end) or a bad one, closing it; `None`
    /// for the arm's where neither is there.
    #[inline(never)]
    fn c_hex(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if super::cxx::hex_float_ahead(b, p + 2, false) || self.depth >= MAX {
            return None;
        }
        let mut e = p + 2;
        while e < b.len() && b[e].is_ascii_hexdigit() {
            e += 1;
        }
        let (x, leaf) = if let Some(x) = c::int_suffix(b, e) {
            (x, HEX_SUFFIX)
        } else if matches!(at(b, e), b'g'..=b'z' | b'G'..=b'Z' | b'_') && e < b.len() {
            (super::text::ident_end(b, e + 1), HEX_BAD_SUFFIX)
        } else {
            return None;
        };
        self.open(p, 2, C_NUM_HEX, HEX_BASE | C);
        self.close(e, x - e, leaf | C);
        self.stuck = usize::from(x == e);
        Some(x | KEEP)
    }

    /// C's `.` at `p`, not `..` or a float's: `access`, a member's name after it but before a
    /// call's `(`; else the accessor alone.
    #[inline(always)]
    fn c_access(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let s = p + 1;
        let Some(&d) = b.get(s) else {
            return Some(self.tok(p, p + 1, ACCESSOR | C));
        };
        if CLASS[usize::from(d)] & NAME_START != 0 || d == b'_' {
            let e = ascii_word_end(b, s + 1);
            if e < b.len() && b[e] >= 0x80 {
                return None;
            }
            if at(b, skip_spaces(b, e)) != b'(' || skip_spaces(b, e) >= b.len() {
                self.put(p, s, ACCESSOR | C);
                return Some(self.tok(s, e, MEMBER | C));
            }
        } else if d >= 0x80 {
            return None;
        }
        Some(self.tok(p, p + 1, ACCESSOR | C))
    }
}
