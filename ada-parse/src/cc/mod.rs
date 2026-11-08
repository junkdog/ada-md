//! C and C++ by a scanner of ada's own design (X-057's redesign, ADR-41: speed first), in place
//! of bat's compiled contexts ([`crate::cpp`]). Rust's shape (X-056): one loop over a line's
//! bytes, a byte's class one load ([`CC`]), a blank run and the token after it in one turn, a
//! name read once and looked up once in one table of every word ([`words`]), punctuation one
//! `match`, tokens straight to the sink. What a token is comes from its own bytes, the token
//! before it and the byte after it, never a lookahead re-reading a path.
//!
//! What the 14 built-in themes tell apart (the census, `x057/stacks-*.txt`: 28 color tuples
//! for C++, 27 for C) is a token's own scopes and, of the meta scopes, `meta.preprocessor`
//! alone, for a directive line's plain text ([`PP_TEXT`]). So a kind is a leaf (bat's scopes
//! for the token, without the language's suffix), and a directive line's gaps are filled with
//! [`PP_TEXT`].
//!
//! The state between lines is ada's own, not bat's stack ([`State`]'s bytes, [`Scan::load`]):
//! what each open `{` opened, two bits a level (a namespace's level, a class's body, code, an
//! initializer), and what a line can end in (a block comment, a string's or a line comment's
//! `\` continuation, a raw string and its delimiter, a directive continued, an `#if 0`'s
//! depth), with the token before the line's first ([`Scan::prev`]).
//!
//! Stages (vault `ada-parse.md`, X-057's redesign): **R1** this loop and the directive lexer
//! ([`pp`]); R2 definitions, parameters and the names after `class`, `namespace`, `typedef`,
//! `using`; R3 the tail parity shows (templates' `<>`, casts, attributes, raw strings' corners);
//! R4 the switch, [`crate::cpp`] deleted. Until R4 it runs where ada-parse is built with
//! `ADA_PARSE_X057` set ([`crate::X057`]).

mod code;
mod pp;
pub(crate) mod support;
mod words;

use crate::emit::{Rules, Run, Runs, Sink, Spans};
use crate::{Out, State};

macro_rules! leaves {
    ($($name:ident = [$($s:literal),*],)*) => {
        /// The leaves: a token's scopes, C++'s and C's.
        static LEAVES: [[&[&str]; 2]; Leaf::Count as usize] = [
            $([&[$(concat!($s, ".c++")),*], &[$(concat!($s, ".c")),*]],)*
        ];
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[repr(u16)]
        enum Leaf { $($name,)* Count }
        $(#[allow(dead_code)] const $name: u16 = Leaf::$name as u16;)*
    };
}

leaves! {
    PLAIN = [],
    PP_TEXT = ["meta.preprocessor"],
    IF_BRANCH = ["comment.block.preprocessor.if-branch"],
    BLOCK_TEXT = ["comment.block"],
    COMMENT_MARK = ["comment.block", "punctuation.definition.comment"],
    LINE_TEXT = ["comment.line.double-slash"],
    LINE_MARK = ["comment.line.double-slash", "punctuation.definition.comment"],
    LINE_CONT = ["comment.line.double-slash", "punctuation.separator.continuation"],
    DQ = ["string.quoted.double"],
    DQ_PREFIX = ["string.quoted.double", "storage.type.string"],
    DQ_BEGIN = ["string.quoted.double", "punctuation.definition.string.begin"],
    DQ_END = ["string.quoted.double", "punctuation.definition.string.end"],
    DQ_ESCAPE = ["string.quoted.double", "constant.character.escape"],
    DQ_CONT = ["string.quoted.double", "punctuation.separator.continuation"],
    DQ_PLACEHOLDER = ["string.quoted.double", "constant.other.placeholder"],
    SQ = ["string.quoted.single"],
    SQ_PREFIX = ["string.quoted.single", "storage.type.string"],
    SQ_BEGIN = ["string.quoted.single", "punctuation.definition.string.begin"],
    SQ_END = ["string.quoted.single", "punctuation.definition.string.end"],
    SQ_ESCAPE = ["string.quoted.single", "constant.character.escape"],
    SQ_CONT = ["string.quoted.single", "punctuation.separator.continuation"],
    UNQUOTED = ["string.unquoted"],
    INC_DQ = ["string.quoted.double.include"],
    INC_DQ_BEGIN = ["string.quoted.double.include", "punctuation.definition.string.begin"],
    INC_DQ_END = ["string.quoted.double.include", "punctuation.definition.string.end"],
    INC_LT = ["string.quoted.other.lt-gt.include"],
    INC_LT_BEGIN = ["string.quoted.other.lt-gt.include", "punctuation.definition.string.begin"],
    INC_LT_END = ["string.quoted.other.lt-gt.include", "punctuation.definition.string.end"],
    FLOAT = ["constant.numeric.float.decimal"],
    FLOAT_DOT = ["constant.numeric.float.decimal", "punctuation.separator.decimal"],
    FLOAT_SUFFIX = ["constant.numeric.float.decimal", "storage.type.numeric"],
    FLOAT_BAD_SUFFIX = ["constant.numeric.float.decimal", "invalid.illegal.numeric.suffix"],
    HEX_FLOAT = ["constant.numeric.float.hexadecimal"],
    HEX_FLOAT_BASE = ["constant.numeric.float.hexadecimal", "punctuation.definition.numeric.base"],
    HEX_FLOAT_DOT = ["constant.numeric.float.hexadecimal", "punctuation.separator.decimal"],
    HEX_FLOAT_SUFFIX = ["constant.numeric.float.hexadecimal", "storage.type.numeric"],
    BIN = ["constant.numeric.integer.binary"],
    BIN_BASE = ["constant.numeric.integer.binary", "punctuation.definition.numeric.base"],
    BIN_SUFFIX = ["constant.numeric.integer.binary", "storage.type.numeric"],
    HEX = ["constant.numeric.integer.hexadecimal"],
    HEX_BASE = ["constant.numeric.integer.hexadecimal", "punctuation.definition.numeric.base"],
    HEX_SUFFIX = ["constant.numeric.integer.hexadecimal", "storage.type.numeric"],
    HEX_BAD_SUFFIX = ["constant.numeric.integer.hexadecimal", "invalid.illegal.numeric.suffix"],
    OCT = ["constant.numeric.integer.octal"],
    OCT_BASE = ["constant.numeric.integer.octal", "punctuation.definition.numeric.base"],
    OCT_SUFFIX = ["constant.numeric.integer.octal", "storage.type.numeric"],
    DEC = ["constant.numeric.integer.decimal"],
    DEC_SUFFIX = ["constant.numeric.integer.decimal", "storage.type.numeric"],
    DEC_BAD_SUFFIX = ["constant.numeric.integer.decimal", "invalid.illegal.numeric.suffix"],
    ARITH = ["keyword.operator.arithmetic"],
    ASSIGN_AUG = ["keyword.operator.assignment.augmented"],
    COMPARISON = ["keyword.operator.comparison"],
    OPERATOR = ["keyword.operator"],
    ASSIGN = ["keyword.operator.assignment"],
    TERNARY = ["keyword.operator.ternary"],
    VARIADIC = ["keyword.operator.variadic"],
    WORD_OP = ["keyword.operator.word"],
    CAST_KW = ["keyword.operator.word.cast"],
    ARROW = ["punctuation.accessor.arrow"],
    DOT = ["punctuation.accessor.dot"],
    DOUBLE_COLON = ["punctuation.accessor.double-colon"],
    SEPARATOR = ["punctuation.separator"],
    TERMINATOR = ["punctuation.terminator"],
    INIT_LIST = ["punctuation.separator.initializer-list"],
    CLASS_SECTION = ["punctuation.section.class"],
    CONT = ["punctuation.separator.continuation"],
    GROUP_BEGIN = ["punctuation.section.group.begin"],
    GROUP_END = ["punctuation.section.group.end"],
    BLOCK_BEGIN = ["punctuation.section.block.begin"],
    BLOCK_END = ["punctuation.section.block.end"],
    BRACKETS_BEGIN = ["punctuation.section.brackets.begin"],
    BRACKETS_END = ["punctuation.section.brackets.end"],
    GENERIC_BEGIN = ["punctuation.section.generic.begin"],
    GENERIC_END = ["punctuation.section.generic.end"],
    CONTROL = ["keyword.control"],
    BREAK_KW = ["keyword.control.flow.break"],
    CONTINUE_KW = ["keyword.control.flow.continue"],
    GOTO_KW = ["keyword.control.flow.goto"],
    RETURN_KW = ["keyword.control.flow.return"],
    THROW_KW = ["keyword.control.flow.throw"],
    IMPORT = ["keyword.control.import"],
    IMPORT_ELSE = ["keyword.control.import.else"],
    IMPORT_DEFINE = ["keyword.control.import.define"],
    IMPORT_INCLUDE = ["keyword.control.import.include"],
    IMPORT_ERROR = ["keyword.control.import.error"],
    DEFINED = ["keyword.control"],
    STORAGE_TYPE = ["storage.type"],
    TEMPLATE_KW = ["storage.type.template"],
    MODIFIER = ["storage.modifier"],
    C99 = ["support.function.C99"],
    SYS_TYPE = ["support.type.sys-types"],
    PTHREAD_TYPE = ["support.type.pthread"],
    STDINT_TYPE = ["support.type.stdint"],
    MICROSOFT_TYPE = ["support.type.microsoft"],
    WINDOWS_TYPE = ["support.type.windows"],
    MAC_TYPE = ["support.type.mac-classic"],
    LANGUAGE = ["constant.language"],
    SUPPORT_CONSTANT = ["support.constant"],
    MAC_CONSTANT = ["constant.other.variable.mac-classic"],
    MAC_SUPPORT_CONSTANT = ["support.constant.mac-classic"],
    VAR_LANGUAGE = ["variable.language"],
    MEMBER_VAR = ["variable.other.readwrite.member"],
    GLOBAL_MAC = ["variable.other.readwrite.global.mac-classic"],
    STATIC_MAC = ["variable.other.readwrite.static.mac-classic"],
    FUNCTION = ["variable.function"],
    MEMBER_FUNCTION = ["variable.function.member"],
    PARAMETER = ["variable.parameter"],
    PP_FUNCTION = ["entity.name.function.preprocessor"],
    PP_CONSTANT = ["entity.name.constant.preprocessor"],
    LABEL = ["entity.name.label"],
}

/// The scopes under `source.c++` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    LEAVES
        .get(usize::from(kind))
        .map_or_else(Vec::new, |l| l[0].to_vec())
}

/// The scopes under `source.c` a kind carries, outermost first.
pub fn scopes_c(kind: u16) -> Vec<&'static str> {
    LEAVES
        .get(usize::from(kind))
        .map_or_else(Vec::new, |l| l[1].to_vec())
}

// What a `{` opened, two bits a level ([`Scan::braces`]).
/// A namespace's level: the top, a namespace's block, `extern "C" {`.
const NS: u8 = 0;
/// A class's, a struct's or a union's body.
const CLASS: u8 = 1;
/// Code: a function's body, a block in one.
const CODE: u8 = 2;
/// An initializer, an enum's enumerators.
const INIT: u8 = 3;
/// The levels [`Scan::braces`] holds.
const LEVELS: usize = 64;

// What a line can end in ([`Scan::mode`]).
const M_CODE: u8 = 0;
const M_BLOCK_COMMENT: u8 = 1;
/// A `//` comment whose line ends in `\`.
const M_LINE_COMMENT: u8 = 2;
/// A `"…"` string whose line ends in `\`.
const M_DQ: u8 = 3;
/// A `'…'` whose line ends in `\`.
const M_SQ: u8 = 4;
/// A raw string ([`Scan::delim`]).
const M_RAW: u8 = 5;
/// An `#if 0`'s lines, [`Scan::aux`] the `#if`s open in them.
const M_DISABLED: u8 = 6;

// A directive the line continues ([`Scan::pp`]), beside a mode.
/// None.
const PP_NONE: u8 = 0;
/// A directive's rest, plain but for comments (`#if`'s, `#pragma`'s…).
const PP_REST: u8 = 1;
/// A `#define`'s body: code.
const PP_BODY: u8 = 2;
/// `#error`'s and `#warning`'s message.
const PP_MESSAGE: u8 = 3;

// The token before ([`Scan::prev`]): what a name or a `:` after it is.
/// A statement's start: the file's, after `;`, `{`, `}`.
const P_STMT: u8 = 0;
/// `.`, `->`: a member next.
const P_ACCESSOR: u8 = 1;
/// `)`.
const P_CLOSE: u8 = 2;
/// `public`, `private`, `protected`.
const P_ACCESS: u8 = 3;
/// Anything else.
const P_OTHER: u8 = 4;
/// C's `->`: bat's C leaves the member after it plain (C++ colors it, as it does after `.`).
const P_ARROW: u8 = 5;

// [`State::arg`]'s bits.
const A_MODE: u16 = 0x7;
const A_PP: u32 = 3;
const A_AUX: u32 = 5;
const A_PENDING: u32 = 10;
const A_PREV: u32 = 12;
/// A `?` waits for its `:`.
const A_TERNARY: u16 = 0x8000;
/// No `{` pending ([`Scan::pending`]).
const NO_PENDING: u8 = 0;

/// A raw string's delimiter's bytes [`State`] keeps; a longer one is told by these and its
/// length.
const DELIM_KEPT: usize = 12;

/// Lex `b`, a line of C++ without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    line_on(b, state, out, false, Spans::new());
}

/// Lex `b`, a line of C, from `state`.
#[inline(never)]
pub fn line_c(b: &[u8], state: &mut State, out: &mut Out) {
    line_on(b, state, out, true, Spans::new());
}

/// [`line`]'s runs by theme rule pushed to `runs`.
#[inline(never)]
pub fn line_runs(
    b: &[u8],
    state: &mut State,
    out: &mut Out,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
) {
    let mut k = line_on(b, state, out, false, Runs::new(rules, runs));
    k.end_line(b.len());
}

/// [`line_c`]'s runs by theme rule pushed to `runs`.
#[inline(never)]
pub fn line_runs_c(
    b: &[u8],
    state: &mut State,
    out: &mut Out,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
) {
    let mut k = line_on(b, state, out, true, Runs::new(rules, runs));
    k.end_line(b.len());
}

#[inline(always)]
fn line_on<K: Sink>(b: &[u8], state: &mut State, out: &mut Out, c: bool, k: K) -> K {
    let mut s = Scan::new(b, out, c, k);
    s.load(state);
    s.k.open(s.out, b.len() / 4 + 16);
    s.lex_line();
    s.k.close(s.out);
    *state = s.state();
    s.k
}

/// A block of C++ on one scan opened from `state` ([`crate::Carry`]): what a line hands the
/// next stays in the scan's fields, a [`State`] read once at the start and made once at the end.
pub fn block(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    block_on(state, spans, false, pass)
}

/// [`block`] for C.
pub fn block_c(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    block_on(state, spans, true, pass)
}

fn block_on(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    c: bool,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    let mut out = Out::new(spans);
    let mut s = Scan::new(b"", &mut out, c, Spans::new());
    s.load(state);
    s.out.0.clear();
    s.k.open(s.out, 64);
    let rest = pass(&mut s);
    *state = s.state();
    rest
}

/// [`block`]'s runs by theme rule pushed to `runs` ([`crate::CarryRuns`]).
pub fn block_runs(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    lines: &mut crate::RunLines,
) -> Option<usize> {
    block_runs_on(state, spans, false, rules, runs, lines)
}

/// [`block_runs`] for C.
pub fn block_runs_c(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    lines: &mut crate::RunLines,
) -> Option<usize> {
    block_runs_on(state, spans, true, rules, runs, lines)
}

fn block_runs_on(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    c: bool,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    lines: &mut crate::RunLines,
) -> Option<usize> {
    let mut out = Out::new(spans);
    let k = Runs::new(rules, runs);
    let mut s = Scan::new(b"", &mut out, c, k);
    s.load(state);
    s.out.0.clear();
    s.k.open(s.out, 64);
    let rest = lines.pass(&mut s);
    *state = s.state();
    rest
}

impl crate::Carry for Scan<'_, '_, Spans> {
    fn next(&mut self, b: &[u8]) -> &[crate::Span] {
        // SAFETY: the line is set back to an empty one before `b`'s borrow ends
        self.carry(unsafe { crate::line_for(b) });
        self.out.0
    }
}

impl crate::CarryRuns for Scan<'_, '_, Runs<'_>> {
    fn next(&mut self, b: &[u8]) -> usize {
        // SAFETY: as above
        self.carry(unsafe { crate::line_for(b) });
        self.k.end_line(b.len())
    }
}

struct Scan<'b, 'c, K: Sink> {
    b: &'b [u8],
    /// What each open `{` opened, two bits a level, the innermost lowest; past [`LEVELS`] the
    /// outer ones are lost (and the state inexact).
    braces: u128,
    /// How many `{` are open, up to 255.
    depth: u8,
    exact: bool,
    /// What the line ends in so far ([`M_CODE`]…).
    mode: u8,
    /// The directive the line is in ([`PP_NONE`]…).
    pp: u8,
    /// An `#if 0`'s open `#if`s, or a raw string's delimiter's length.
    aux: u8,
    /// What the next `{` opens, where a word said ([`NO_PENDING`], else the level plus one).
    pending: u8,
    /// The token before ([`P_STMT`]…).
    prev: u8,
    /// A `case` or `default` waits for its `:`.
    case: bool,
    /// The `?`s open since the statement's start, whose `:`s are the ternary's (a line hands
    /// the next whether any is).
    ternary: u8,
    /// A raw string's delimiter, its first [`DELIM_KEPT`] bytes.
    delim: [u8; DELIM_KEPT],
    /// Where a directive's text is filled to ([`PP_TEXT`]).
    at: usize,
    /// What each word is in the language ([`words::INFO`]'s row).
    info: &'static [u16],
    /// C, not C++.
    c: bool,
    k: K,
    out: &'b mut Out<'c>,
}

impl<'b, 'c, K: Sink> Scan<'b, 'c, K> {
    #[inline(always)]
    fn new(b: &'b [u8], out: &'b mut Out<'c>, c: bool, k: K) -> Self {
        Scan {
            b,
            braces: 0,
            depth: 0,
            exact: true,
            mode: M_CODE,
            pp: PP_NONE,
            aux: 0,
            pending: NO_PENDING,
            prev: P_STMT,
            case: false,
            ternary: 0,
            delim: [0; DELIM_KEPT],
            at: 0,
            info: &words::INFO[usize::from(c)],
            c,
            k,
            out,
        }
    }

    /// The scan's fields from `state`: the levels' bits in its first 16 bytes, a raw string's
    /// delimiter in the next, the rest in [`State::arg`].
    #[inline(always)]
    fn load(&mut self, state: &State) {
        let mut w = [0u8; 16];
        w[..14].copy_from_slice(&state.levels);
        w[14..].copy_from_slice(&state.withs[..2]);
        self.braces = u128::from_le_bytes(w);
        self.depth = state.depth;
        self.exact = !state.inexact;
        let a = state.arg;
        self.mode = (a & A_MODE) as u8;
        self.pp = (a >> A_PP & 3) as u8;
        self.aux = (a >> A_AUX & 31) as u8;
        self.pending = (a >> A_PENDING & 3) as u8;
        self.prev = (a >> A_PREV & 7) as u8;
        self.ternary = u8::from(a & A_TERNARY != 0);
        if self.mode == M_RAW {
            self.delim.copy_from_slice(&state.withs[2..]);
        }
    }

    /// The state the scan's last line left.
    #[inline(always)]
    fn state(&self) -> State {
        let w = self.braces.to_le_bytes();
        let mut s = State::default();
        s.levels.copy_from_slice(&w[..14]);
        s.withs[..2].copy_from_slice(&w[14..]);
        if self.mode == M_RAW {
            s.withs[2..].copy_from_slice(&self.delim);
        }
        s.depth = self.depth;
        s.inexact = !self.exact;
        s.arg = u16::from(self.mode)
            | u16::from(self.pp) << A_PP
            | u16::from(self.aux) << A_AUX
            | u16::from(self.pending) << A_PENDING
            | u16::from(self.prev) << A_PREV
            | if self.ternary != 0 { A_TERNARY } else { 0 };
        s
    }

    /// `b`, a block's next line, lexed on this scan, its sink closed; the scan's line an empty
    /// one again after.
    #[inline(always)]
    fn carry(&mut self, b: &'b [u8]) {
        self.k.start_line();
        self.b = b;
        self.lex_line();
        self.k.close(self.out);
        self.b = b"";
    }

    /// The line, from what the line before left: a mode's rest first, then a directive or the
    /// code.
    #[inline(always)]
    fn lex_line(&mut self) {
        self.case = false;
        self.at = 0;
        if self.mode != M_CODE || self.pp != PP_NONE {
            self.lex_resumed();
            return;
        }
        let b = self.b;
        let mut p = 0;
        // the indentation, then a directive or the code
        while p < b.len() && CC[usize::from(b[p])] & SP != 0 {
            p += 1;
        }
        if b.get(p) == Some(&b'#') {
            self.directive(p);
        } else {
            self.code::<false>(p);
        }
    }

    /// Bytes `s..e` are `kind` (not [`PLAIN`]); in a directive, the text before them its plain
    /// text.
    #[inline(always)]
    fn put<const PP: bool>(&mut self, s: usize, e: usize, kind: u16) {
        debug_assert!(kind != PLAIN && e > s);
        if PP {
            if s > self.at {
                self.k.merge(&mut self.out, self.at, s, PP_TEXT);
            }
            self.at = e;
        }
        self.k.merge(&mut self.out, s, e, kind);
    }

    /// What the innermost open `{` opened ([`NS`] at the top).
    #[inline(always)]
    fn level(&self) -> u8 {
        (self.braces & 3) as u8
    }

    /// A `{`: what it opens pushed.
    #[inline(always)]
    fn open_brace(&mut self) {
        let kind = match self.pending {
            NO_PENDING => match self.level() {
                // R2 tells a function's body from an initializer here
                NS | CLASS => CODE,
                l => l,
            },
            p => p - 1,
        };
        self.pending = NO_PENDING;
        self.braces = self.braces << 2 | u128::from(kind);
        if self.depth == u8::MAX || usize::from(self.depth) >= LEVELS {
            self.exact = false;
        }
        self.depth = self.depth.saturating_add(1);
    }

    /// A `}`: the innermost level closed.
    #[inline(always)]
    fn close_brace(&mut self) {
        self.pending = NO_PENDING;
        if self.depth > 0 {
            self.braces >>= 2;
            if self.depth != u8::MAX {
                self.depth -= 1;
            }
        }
    }
}

// The byte classes ([`CC`]).
/// A blank, `\s` but the line's `\n`.
const SP: u8 = 1;
/// A name's first byte: a letter, `_`, `$`, a byte past ASCII.
const NS0: u8 = 2;
/// A name's byte: [`NS0`]'s and the digits.
const NW: u8 = 4;
const DG: u8 = 8;
/// A byte past ASCII.
const HI: u8 = 16;
/// An upper-case ASCII letter.
const UP: u8 = 32;

/// Each byte's classes.
static CC: [u8; 256] = {
    let mut t = [0; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        t[c] = if b >= 0x80 {
            NS0 | NW | HI
        } else if b.is_ascii_uppercase() {
            NS0 | NW | UP
        } else if b.is_ascii_lowercase() || b == b'_' || b == b'$' {
            NS0 | NW
        } else if b.is_ascii_digit() {
            NW | DG
        } else if matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r') {
            SP
        } else {
            0
        };
        c += 1;
    }
    t
};

#[cfg(test)]
mod tests;
