//! Lua on a scanner of ada's own (X-098). The floor probe's shape (`x098`'s `sol`), Lua's own lexer's
//! (`llex.c`, LuaJIT's `lj_lex.c`): one loop over a line's bytes, a byte's action one load from a
//! table, a name read once and looked up once, the token before in one byte ([`Scan::prev`]),
//! and frames only for what nests: blocks, groups, brackets, tables ([`Scan::frames`], three
//! bits a level). Long brackets and strings continued past a line's end are a mode
//! ([`Scan::mode`]). Kinds are the compiled scanner's leaves.

use crate::emit::{Rules, Run, Runs, Sink, Spans};
use crate::prim::{WORD, char_len, digit, find_in};
use crate::words::{Words, text_len};
use crate::{Out, State};

/// The leaves: a token's scopes.
static LEAVES: [&str; 88] = [
    "",
    // comments: the text, its mark; a block's text, its begin, its end
    "comment.line.lua",
    "comment.line.lua punctuation.definition.comment.lua",
    "comment.block.lua",
    "comment.block.lua punctuation.definition.comment.begin.lua",
    "comment.block.lua punctuation.definition.comment.end.lua",
    // strings: the text, then BEGIN, END, ESCAPE, ESCAPE_HEX, ESCAPE_DEC, ESCAPE_UNICODE,
    // ESCAPE_SPACES, INVALID_ESCAPE
    "string.quoted.single.lua",
    "string.quoted.single.lua punctuation.definition.string.begin.lua",
    "string.quoted.single.lua punctuation.definition.string.end.lua",
    "string.quoted.single.lua constant.character.escape.lua",
    "string.quoted.single.lua constant.character.escape.hexadecimal.lua",
    "string.quoted.single.lua constant.character.escape.decimal.lua",
    "string.quoted.single.lua constant.character.escape.unicode.lua",
    "string.quoted.single.lua constant.character.escape.whitespace.lua",
    "string.quoted.single.lua invalid.illegal.invalid-escape.lua",
    "string.quoted.double.lua",
    "string.quoted.double.lua punctuation.definition.string.begin.lua",
    "string.quoted.double.lua punctuation.definition.string.end.lua",
    "string.quoted.double.lua constant.character.escape.lua",
    "string.quoted.double.lua constant.character.escape.hexadecimal.lua",
    "string.quoted.double.lua constant.character.escape.decimal.lua",
    "string.quoted.double.lua constant.character.escape.unicode.lua",
    "string.quoted.double.lua constant.character.escape.whitespace.lua",
    "string.quoted.double.lua invalid.illegal.invalid-escape.lua",
    "string.quoted.multiline.lua",
    "string.quoted.multiline.lua punctuation.definition.string.begin.lua",
    "string.quoted.multiline.lua punctuation.definition.string.end.lua",
    // the rest
    "punctuation.terminator.statement.lua",
    "storage.type.function.lua",
    "keyword.control.break.lua",
    "keyword.control.return.lua",
    "storage.modifier.lua",
    "invalid.illegal.unexpected-end.lua",
    "punctuation.section.group.begin.lua",
    "punctuation.section.group.end.lua",
    "punctuation.separator.comma.lua",
    "invalid.unexpected-keyword.lua",
    "variable.parameter.function.lua",
    "constant.language.lua",
    "punctuation.accessor.lua",
    "variable.other.lua",
    "entity.name.function.lua",
    "meta.property.lua",
    "meta.property.lua entity.name.function.lua support.function.metamethod.lua",
    "meta.property.lua entity.name.function.lua",
    "keyword.control.lua",
    "keyword.control.conditional.lua",
    "keyword.control.loop.lua",
    "punctuation.definition.label.begin.lua",
    "entity.name.label.lua",
    "punctuation.definition.label.end.lua",
    "keyword.control.goto.lua",
    "variable.label.lua",
    "keyword.control.end.lua",
    "keyword.operator.comparison.lua",
    "keyword.operator.arithmetic.lua",
    "keyword.operator.bitwise.lua",
    "keyword.operator.concatenation.lua",
    "keyword.operator.assignment.lua",
    "keyword.operator.logical.lua",
    "keyword.operator.length.lua",
    "punctuation.section.brackets.begin.lua",
    "punctuation.section.brackets.end.lua",
    "meta.property.lua variable.function.lua",
    "meta.property.lua support.other.metaproperty.lua",
    "constant.language.boolean.true.lua",
    "constant.language.null.lua",
    "variable.language.this.lua",
    "support.constant.builtin.lua",
    "support.function.builtin.lua",
    "variable.function.lua",
    "constant.numeric.float.hexadecimal.lua",
    "constant.numeric.float.hexadecimal.lua punctuation.definition.numeric.base.lua",
    "constant.numeric.float.hexadecimal.lua punctuation.separator.decimal.lua",
    "constant.numeric.integer.hexadecimal.lua",
    "constant.numeric.integer.hexadecimal.lua punctuation.definition.numeric.base.lua",
    "constant.numeric.float.decimal.lua",
    "constant.numeric.float.decimal.lua punctuation.separator.decimal.lua",
    "constant.numeric.integer.decimal.lua",
    "punctuation.section.block.begin.lua",
    "punctuation.section.block.end.lua",
    "punctuation.separator.field.lua",
    "entity.name.function.lua support.function.metamethod.lua",
    "string.unquoted.key.lua support.other.metaproperty.lua",
    "string.unquoted.key.lua",
    "meta.mapping.lua punctuation.separator.key-value.lua",
    "meta.property.lua support.function.builtin.lua",
    "meta.property.lua support.constant.builtin.lua",
];

const PLAIN: u16 = 0;
const LINE_COMMENT: u16 = 1;
const LINE_MARK: u16 = 2;
const BLOCK_COMMENT: u16 = 3;
const BLOCK_BEGIN: u16 = 4;
const BLOCK_END: u16 = 5;
const SINGLE: u16 = 6;
const DOUBLE: u16 = 15;
// a string's leaves, from its text
const BEGIN: u16 = 1;
const END: u16 = 2;
const ESCAPE: u16 = 3;
const ESCAPE_HEX: u16 = 4;
const ESCAPE_DEC: u16 = 5;
const ESCAPE_UNICODE: u16 = 6;
const ESCAPE_SPACES: u16 = 7;
const INVALID_ESCAPE: u16 = 8;
const MULTILINE: u16 = 24;
const TERMINATOR: u16 = 27;
const FUNCTION_KEYWORD: u16 = 28;
const BREAK: u16 = 29;
const RETURN: u16 = 30;
const LOCAL: u16 = 31;
const UNEXPECTED_END: u16 = 32;
const GROUP_BEGIN: u16 = 33;
const GROUP_END: u16 = 34;
const COMMA: u16 = 35;
const UNEXPECTED_KEYWORD: u16 = 36;
const PARAMETER: u16 = 37;
const LANGUAGE: u16 = 38;
const ACCESSOR: u16 = 39;
const VARIABLE: u16 = 40;
const FUNCTION_NAME: u16 = 41;
const PROPERTY_LEAF: u16 = 42;
const PROPERTY_METAMETHOD: u16 = 43;
const PROPERTY_FUNCTION_NAME: u16 = 44;
const CONTROL: u16 = 45;
const CONDITIONAL: u16 = 46;
const LOOP: u16 = 47;
const LABEL_BEGIN: u16 = 48;
const LABEL_NAME: u16 = 49;
const LABEL_CLOSE: u16 = 50;
const GOTO_KEYWORD: u16 = 51;
const LABEL_VARIABLE: u16 = 52;
const END_KEYWORD: u16 = 53;
const COMPARISON: u16 = 54;
const ARITHMETIC: u16 = 55;
const BITWISE: u16 = 56;
const CONCATENATION: u16 = 57;
const ASSIGNMENT: u16 = 58;
const LOGICAL: u16 = 59;
const LENGTH: u16 = 60;
const BRACKETS_BEGIN: u16 = 61;
const BRACKETS_END: u16 = 62;
const PROPERTY_CALL: u16 = 63;
const PROPERTY_METAPROPERTY: u16 = 64;
const BOOLEAN: u16 = 65;
const NULL: u16 = 66;
const THIS: u16 = 67;
const SUPPORT_CONSTANT: u16 = 68;
const SUPPORT_FUNCTION: u16 = 69;
const CALL: u16 = 70;
const FLOAT_HEX: u16 = 71;
const INT_HEX: u16 = 74;
const FLOAT_DEC: u16 = 76;
const INT_DEC: u16 = 78;
const BRACES_BEGIN: u16 = 79;
const BRACES_END: u16 = 80;
const FIELD_SEPARATOR: u16 = 81;
const KEY_METAMETHOD: u16 = 82;
const KEY_METAPROPERTY: u16 = 83;
const KEY_LEAF: u16 = 84;
const KEY_VALUE: u16 = 85;
const MEMBER_FUNCTION: u16 = 86;
const MEMBER_CONSTANT: u16 = 87;

/// The scopes under `source.lua` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    LEAVES.get(usize::from(kind)).map_or_else(Vec::new, |l| {
        l.split(' ').filter(|s| !s.is_empty()).collect()
    })
}

// A byte's action ([`ACT`]).
const A_OTHER: u8 = 0;
const A_SPACE: u8 = 1;
const A_NAME: u8 = 2;
const A_DIGIT: u8 = 3;
const A_QUOTE: u8 = 4;
const A_DASH: u8 = 5;
const A_LBRACKET: u8 = 6;
const A_RBRACKET: u8 = 7;
const A_LPAREN: u8 = 8;
const A_RPAREN: u8 = 9;
const A_LBRACE: u8 = 10;
const A_RBRACE: u8 = 11;
const A_COMMA: u8 = 12;
const A_SEMI: u8 = 13;
const A_DOT: u8 = 14;
const A_COLON: u8 = 15;
const A_EQ: u8 = 16;
const A_LT: u8 = 17;
const A_GT: u8 = 18;
const A_TILDE: u8 = 19;
const A_SLASH: u8 = 20;
/// `+`, `*`, `%`, `^`.
const A_ARITH: u8 = 21;
/// `&`, `|`.
const A_BITWISE: u8 = 22;
const A_HASH: u8 = 23;

static ACT: [u8; 256] = {
    let mut t = [A_OTHER; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = match b {
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => A_SPACE,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => A_NAME,
            b'0'..=b'9' => A_DIGIT,
            b'"' | b'\'' => A_QUOTE,
            b'-' => A_DASH,
            b'[' => A_LBRACKET,
            b']' => A_RBRACKET,
            b'(' => A_LPAREN,
            b')' => A_RPAREN,
            b'{' => A_LBRACE,
            b'}' => A_RBRACE,
            b',' => A_COMMA,
            b';' => A_SEMI,
            b'.' => A_DOT,
            b':' => A_COLON,
            b'=' => A_EQ,
            b'<' => A_LT,
            b'>' => A_GT,
            b'~' => A_TILDE,
            b'/' => A_SLASH,
            b'+' | b'*' | b'%' | b'^' => A_ARITH,
            b'&' | b'|' => A_BITWISE,
            b'#' => A_HASH,
            _ => A_OTHER,
        };
        c += 1;
    }
    t
};

// What a word is ([`WORDS`], [`INFO`]): the reserved words first, up to [`W_END`].
const W_NONE: u8 = 0;
const W_AND_OR: u8 = 1;
const W_NOT: u8 = 2;
const W_BOOLEAN: u8 = 3;
const W_NIL: u8 = 4;
const W_FUNCTION: u8 = 5;
const W_LOCAL: u8 = 6;
const W_RETURN: u8 = 7;
const W_BREAK: u8 = 8;
const W_GOTO: u8 = 9;
const W_DO: u8 = 10;
const W_IF: u8 = 11;
const W_THEN: u8 = 12;
const W_ELSEIF: u8 = 13;
const W_ELSE: u8 = 14;
const W_WHILE: u8 = 15;
const W_FOR: u8 = 16;
const W_IN: u8 = 17;
const W_REPEAT: u8 = 18;
const W_UNTIL: u8 = 19;
const W_END: u8 = 20;
/// `self`.
const W_SELF: u8 = 21;
/// `_G`, `_VERSION`.
const W_SUPPORT: u8 = 22;
/// `support`'s builtin functions.
const W_BUILTIN: u8 = 23;
/// A builtin module, plus its index in `builtin-modules`' order.
const W_MODULE: u8 = 32;

const WORD_LIST: [&[u8]; 62] = [
    b"and",
    b"or",
    b"not",
    b"true",
    b"false",
    b"nil",
    b"function",
    b"local",
    b"return",
    b"break",
    b"goto",
    b"do",
    b"if",
    b"then",
    b"elseif",
    b"else",
    b"while",
    b"for",
    b"in",
    b"repeat",
    b"until",
    b"end",
    b"self",
    b"_G",
    b"_VERSION",
    b"assert",
    b"collectgarbage",
    b"dofile",
    b"error",
    b"getmetatable",
    b"ipairs",
    b"load",
    b"loadfile",
    b"next",
    b"pairs",
    b"pcall",
    b"print",
    b"rawequal",
    b"rawget",
    b"rawlen",
    b"rawset",
    b"select",
    b"setmetatable",
    b"tonumber",
    b"tostring",
    b"type",
    b"xpcall",
    b"require",
    b"getfenv",
    b"module",
    b"setfenv",
    b"unpack",
    b"coroutine",
    b"package",
    b"string",
    b"utf8",
    b"table",
    b"math",
    b"io",
    b"os",
    b"debug",
    b"bit32",
];
const N: usize = WORD_LIST.len();
static WORDS: Words<128, N, { text_len(&WORD_LIST) }> = Words::new(&WORD_LIST);
/// A word's [`W_NONE`]… by its id.
static INFO: [u8; N + 1] = {
    let mut t = [W_NONE; N + 1];
    let head = [
        W_AND_OR, W_AND_OR, W_NOT, W_BOOLEAN, W_BOOLEAN, W_NIL, W_FUNCTION, W_LOCAL, W_RETURN,
        W_BREAK, W_GOTO, W_DO, W_IF, W_THEN, W_ELSEIF, W_ELSE, W_WHILE, W_FOR, W_IN, W_REPEAT,
        W_UNTIL, W_END, W_SELF, W_SUPPORT, W_SUPPORT,
    ];
    let mut i = 0;
    while i < head.len() {
        t[i + 1] = head[i];
        i += 1;
    }
    while i < 52 {
        t[i + 1] = W_BUILTIN;
        i += 1;
    }
    while i < N {
        t[i + 1] = W_MODULE + (i - 52) as u8;
        i += 1;
    }
    t
};

// The frames, three bits a level in [`Scan::frames`]; 0 is none (the top level, or a level
// lost past [`LEVELS`]).
const F_NONE: u8 = 0;
/// A block ended by `end`: `do`'s, `then`'s, a function's body.
const F_BLOCK: u8 = 1;
/// `repeat`'s block, ended by `until`.
const F_REPEAT: u8 = 2;
/// A call's `( … )`: its `,`s separate.
const F_CALL: u8 = 3;
/// A parenthesized expression's `( … )`.
const F_PAREN: u8 = 4;
/// A function's parameters' `( … )`.
const F_PARAMS: u8 = 5;
/// `[ … ]`: an index, or a table's key.
const F_BRACKET: u8 = 6;
/// A table's `{ … }`.
const F_TABLE: u8 = 7;
/// The levels [`Scan::frames`] holds.
const LEVELS: u8 = 42;

// The token before ([`Scan::prev`]). The first four are the common ones.
/// An operand may start: a statement's start, after an operator, an opening.
const P_BEGIN: u8 = 0;
/// After an operand: `(`, a string or `{` are a call's arguments, `[` an index, `.` an accessor.
const P_END: u8 = 1;
/// After a table's `{`, `,` or `;`: a key may start.
const P_FIELD: u8 = 2;
/// After a table's key: `=` separates its value.
const P_KEY: u8 = 3;
/// After an accessor: a property.
const P_PROP: u8 = 4;
/// After `function`: its name.
const P_FUNC: u8 = 5;
/// After a function's name or a part of it: `.`, `:` or its parameters.
const P_FNEND: u8 = 6;
/// After an accessor in a function's name.
const P_FNPROP: u8 = 7;
/// After a builtin module's name ([`Scan::module`]).
const P_MODULE: u8 = 8;
/// After a builtin module's `.`: its member.
const P_MEMBER: u8 = 9;
/// After `::`: a label's name.
const P_LABEL: u8 = 10;
/// After a label's name: its closing `::`.
const P_LABEL_END: u8 = 11;
/// After `goto`: its label.
const P_GOTO: u8 = 12;

// What a line starts in ([`Scan::mode`]).
const M_CODE: u8 = 0;
/// A long string's `[==[ … ]==]`, at [`Scan::level`].
const M_STRING: u8 = 1;
/// A block comment's `--[==[ … ]==]`, at [`Scan::level`].
const M_COMMENT: u8 = 2;
/// A single-quoted string continued past a `\` at the line's end.
const M_SINGLE: u8 = 3;
const M_DOUBLE: u8 = 4;
/// A single-quoted string's `\z` escape's spaces, past the line's end.
const M_Z_SINGLE: u8 = 5;
const M_Z_DOUBLE: u8 = 6;

/// Lex `b`, a line of Lua without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    line_on::<_, false>(b, state, out, Spans::new());
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
    let mut k = line_on::<_, true>(b, state, out, Runs::new(rules, runs));
    k.end_line(b.len());
}

#[inline(always)]
fn line_on<K: Sink, const SHARED: bool>(b: &[u8], state: &mut State, out: &mut Out, k: K) -> K {
    let mut s = Scan::new(b, out, k);
    s.load(state);
    s.k.open(s.out, b.len() / 4 + 16);
    s.lex_line::<SHARED>();
    s.k.close(s.out);
    *state = s.state();
    s.k
}

/// A block of Lua on one scan opened from `state` ([`crate::Carry`]).
pub fn block(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    let mut out = Out::new(spans);
    let mut s = Scan::new(b"", &mut out, Spans::new());
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
    let mut out = Out::new(spans);
    let mut s = Scan::new(b"", &mut out, Runs::new(rules, runs));
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
        self.carry::<false>(unsafe { crate::line_for(b) });
        self.out.0
    }
}

impl crate::CarryRuns for Scan<'_, '_, Runs<'_>> {
    fn next(&mut self, b: &[u8]) -> usize {
        // SAFETY: as above
        self.carry::<true>(unsafe { crate::line_for(b) });
        self.k.end_line(b.len())
    }
}

struct Scan<'b, 'c, K: Sink> {
    b: &'b [u8],
    /// What each open frame is ([`F_BLOCK`]…), three bits a level, the innermost lowest; past
    /// [`LEVELS`] the outer ones are lost (and the state inexact).
    frames: u128,
    /// How many frames are open, up to 255.
    depth: u8,
    exact: bool,
    /// The token before ([`P_BEGIN`]…).
    prev: u8,
    /// The statement is an expression list (`local`, `return`, `for`, `in`): its `,`s separate.
    list: bool,
    /// [`Scan::list`] at each open frame, a bit a level, the innermost lowest: what a statement
    /// a frame interrupts is again once it closes (a function's body in a `return`'s list).
    lists: u64,
    /// What the line starts in ([`M_CODE`]…).
    mode: u8,
    /// An open long bracket's `=`s.
    level: u16,
    /// The builtin module named before ([`P_MODULE`]).
    module: u8,
    k: K,
    out: &'b mut Out<'c>,
}

impl<'b, 'c, K: Sink> Scan<'b, 'c, K> {
    #[inline(always)]
    fn new(b: &'b [u8], out: &'b mut Out<'c>, k: K) -> Self {
        Scan {
            b,
            frames: 0,
            depth: 0,
            exact: true,
            prev: P_BEGIN,
            list: false,
            lists: 0,
            mode: M_CODE,
            level: 0,
            module: 0,
            k,
            out,
        }
    }

    /// The scan's fields from `state`: the frames in its levels and the first two withs, the
    /// level and the module in the next three, the rest in [`State::arg`].
    #[inline(always)]
    fn load(&mut self, state: &State) {
        let mut w = [0u8; 16];
        w[..14].copy_from_slice(&state.levels);
        w[14] = state.withs[0];
        w[15] = state.withs[1];
        self.frames = u128::from_le_bytes(w);
        self.depth = state.depth;
        self.exact = !state.inexact;
        self.level = u16::from_le_bytes([state.withs[2], state.withs[3]]);
        self.module = state.withs[4];
        let mut l = [0u8; 8];
        l.copy_from_slice(&state.withs[5..13]);
        self.lists = u64::from_le_bytes(l);
        let a = state.arg;
        self.mode = (a & 7) as u8;
        self.prev = (a >> 3 & 15) as u8;
        self.list = a & 0x80 != 0;
    }

    /// The state the scan's last line left: every field as it is, so a block lexes as its
    /// lines do one at a time.
    #[inline(always)]
    fn state(&self) -> State {
        let w = self.frames.to_le_bytes();
        let mut s = State::default();
        s.levels.copy_from_slice(&w[..14]);
        s.withs[0] = w[14];
        s.withs[1] = w[15];
        [s.withs[2], s.withs[3]] = self.level.to_le_bytes();
        s.withs[4] = self.module;
        s.withs[5..13].copy_from_slice(&self.lists.to_le_bytes());
        s.depth = self.depth;
        s.inexact = !self.exact;
        s.arg = u16::from(self.mode) | u16::from(self.prev) << 3 | u16::from(self.list) << 7;
        s
    }

    /// `b`, a block's next line, lexed on this scan, its sink closed; the scan's line an empty
    /// one again after.
    #[inline(always)]
    fn carry<const SHARED: bool>(&mut self, b: &'b [u8]) {
        self.k.start_line();
        self.b = b;
        self.lex_line::<SHARED>();
        self.k.close(self.out);
        self.b = b"";
    }

    #[inline(always)]
    fn put(&mut self, s: usize, e: usize, kind: u16) {
        self.k.merge(&mut self.out, s, e, kind);
    }

    #[inline(always)]
    fn top(&self) -> u8 {
        (self.frames & 7) as u8
    }

    #[inline(always)]
    fn push(&mut self, f: u8) {
        self.frames = self.frames << 3 | u128::from(f);
        self.lists = self.lists << 1 | u64::from(self.list);
        self.list = false;
        self.depth = self.depth.saturating_add(1);
        if self.depth > LEVELS {
            self.exact = false;
        }
    }

    #[inline(always)]
    fn pop(&mut self) {
        if self.depth > 0 {
            self.frames >>= 3;
            self.depth -= 1;
            self.list = self.lists & 1 != 0;
            self.lists >>= 1;
        }
    }

    /// Whether the token before ends an operand, so that `(`, `[` and `.` follow one.
    #[inline(always)]
    fn after_operand(&self) -> bool {
        matches!(self.prev, P_END | P_KEY | P_MODULE)
    }

    /// The line, from what the line before left: a mode's rest first; the loop out of line where
    /// `SHARED` ([`Scan::code_shared`]).
    #[inline(always)]
    fn lex_line<const SHARED: bool>(&mut self) {
        let mut p = if self.mode == M_CODE {
            0
        } else {
            self.resume()
        };
        if self.top() == F_PARAMS {
            p = self.params(p);
        }
        if SHARED {
            self.code_shared(p);
        } else {
            self.code(p);
        }
    }

    /// [`Scan::code`] out of line: on the run sink, one copy that the line API and a block's
    /// lines share (inlined into each, ada held two, +24 KB); on the span sink, which ada doesn't
    /// hold, inlined (out of line it ran 2% slower alone).
    #[inline(never)]
    fn code_shared(&mut self, p: usize) {
        self.code(p);
    }

    /// The code from `p` to the line's end.
    #[inline(always)]
    fn code(&mut self, mut p: usize) {
        let b = self.b;
        let n = b.len();
        while p < n {
            let c = b[p];
            p = match ACT[usize::from(c)] {
                A_SPACE => {
                    let mut e = p + 1;
                    while e < n && ACT[usize::from(b[e])] == A_SPACE {
                        e += 1;
                    }
                    e
                }
                A_NAME => self.name(p),
                A_DIGIT => self.number(p),
                A_QUOTE => self.string(p, c),
                A_DASH => {
                    if b.get(p + 1) == Some(&b'-') {
                        self.comment(p)
                    } else {
                        self.op(p, 1, ARITHMETIC)
                    }
                }
                A_LBRACKET => self.lbracket(p),
                A_RBRACKET => {
                    if self.top() == F_BRACKET {
                        self.pop();
                        self.put(p, p + 1, BRACKETS_END);
                        self.prev = if self.top() == F_TABLE { P_KEY } else { P_END };
                    }
                    p + 1
                }
                A_LPAREN => {
                    let f = match self.prev {
                        P_FUNC | P_FNEND => F_PARAMS,
                        P_END | P_KEY | P_MODULE => F_CALL,
                        _ => F_PAREN,
                    };
                    self.push(f);
                    self.put(p, p + 1, GROUP_BEGIN);
                    self.prev = P_BEGIN;
                    if f == F_PARAMS {
                        self.params(p + 1)
                    } else {
                        p + 1
                    }
                }
                A_RPAREN => {
                    match self.top() {
                        F_CALL | F_PAREN => {
                            self.pop();
                            self.put(p, p + 1, GROUP_END);
                            self.prev = P_END;
                        }
                        _ => {}
                    }
                    p + 1
                }
                A_LBRACE => {
                    self.push(F_TABLE);
                    self.put(p, p + 1, BRACES_BEGIN);
                    self.prev = P_FIELD;
                    p + 1
                }
                A_RBRACE => {
                    if self.top() == F_TABLE {
                        self.pop();
                        self.put(p, p + 1, BRACES_END);
                        self.prev = P_END;
                    }
                    p + 1
                }
                A_COMMA => {
                    match self.top() {
                        F_TABLE => {
                            self.put(p, p + 1, FIELD_SEPARATOR);
                            self.prev = P_FIELD;
                        }
                        F_CALL => {
                            self.put(p, p + 1, COMMA);
                            self.prev = P_BEGIN;
                        }
                        F_PAREN | F_BRACKET => {}
                        _ => {
                            if self.list {
                                self.put(p, p + 1, COMMA);
                                self.prev = P_BEGIN;
                            }
                        }
                    }
                    p + 1
                }
                A_SEMI => {
                    match self.top() {
                        F_TABLE => {
                            self.put(p, p + 1, FIELD_SEPARATOR);
                            self.prev = P_FIELD;
                        }
                        F_NONE | F_BLOCK | F_REPEAT => {
                            self.put(p, p + 1, TERMINATOR);
                            (self.prev, self.list) = (P_BEGIN, false);
                        }
                        _ => {}
                    }
                    p + 1
                }
                A_DOT => self.dot(p),
                A_COLON if b.get(p + 1) == Some(&b':') && self.prev != P_LABEL_END => {
                    self.put(p, p + 2, LABEL_BEGIN);
                    self.prev = P_LABEL;
                    p + 2
                }
                A_COLON if b.get(p + 1) == Some(&b':') && self.prev == P_LABEL_END => {
                    self.put(p, p + 2, LABEL_CLOSE);
                    self.prev = P_BEGIN;
                    p + 2
                }
                A_COLON => {
                    self.put(p, p + 1, ACCESSOR);
                    self.prev = if self.prev == P_FNEND {
                        P_FNPROP
                    } else {
                        P_PROP
                    };
                    p + 1
                }
                A_EQ => {
                    if b.get(p + 1) == Some(&b'=') {
                        self.op(p, 2, COMPARISON)
                    } else if self.prev == P_KEY {
                        self.op(p, 1, KEY_VALUE)
                    } else {
                        self.op(p, 1, ASSIGNMENT)
                    }
                }
                A_LT | A_GT => match b.get(p + 1) {
                    Some(&b'=') => self.op(p, 2, COMPARISON),
                    Some(&d) if d == c => self.op(p, 2, BITWISE),
                    _ => self.op(p, 1, COMPARISON),
                },
                A_TILDE => {
                    if b.get(p + 1) == Some(&b'=') {
                        self.op(p, 2, COMPARISON)
                    } else {
                        self.op(p, 1, BITWISE)
                    }
                }
                A_SLASH => {
                    let l = if b.get(p + 1) == Some(&b'/') { 2 } else { 1 };
                    self.op(p, l, ARITHMETIC)
                }
                A_ARITH => self.op(p, 1, ARITHMETIC),
                A_BITWISE => self.op(p, 1, BITWISE),
                A_HASH => self.op(p, 1, LENGTH),
                _ => p + char_len(c),
            };
        }
    }

    /// A function's parameters from `p`, after its `(`: names, `,`, `...` and comments, to its
    /// `)`, where its body opens, or the line's end; bat's list takes nothing else, so other
    /// bytes are plain, and a reserved word ends it as its `)` would.
    #[inline(never)]
    fn params(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        while p < n {
            let c = b[p];
            match ACT[usize::from(c)] {
                A_NAME => {
                    let mut e = p + 1;
                    while e < n && WORD[usize::from(b[e])] {
                        e += 1;
                    }
                    let w = WORDS.value(&b[p..e], &INFO);
                    if w != W_NONE && w <= W_END {
                        return self.body(p, e, UNEXPECTED_KEYWORD);
                    }
                    self.put(p, e, PARAMETER);
                    p = e;
                }
                A_COMMA => {
                    self.put(p, p + 1, COMMA);
                    p += 1;
                }
                A_RPAREN => return self.body(p, p + 1, GROUP_END),
                A_DOT if b.get(p + 1..p + 3) == Some(b"..") => {
                    self.put(p, p + 3, LANGUAGE);
                    p += 3;
                }
                A_DASH if b.get(p + 1) == Some(&b'-') => {
                    p = self.comment(p);
                    if self.mode != M_CODE {
                        return p;
                    }
                }
                _ => p += char_len(c),
            }
        }
        p
    }

    /// The parameters end at `b[p..e]`, `kind`: the function's body opens.
    #[inline(always)]
    fn body(&mut self, p: usize, e: usize, kind: u16) -> usize {
        self.put(p, e, kind);
        self.pop();
        self.push(F_BLOCK);
        self.prev = P_BEGIN;
        e
    }

    /// An operator of `len` bytes at `p`: an operand may follow.
    #[inline(always)]
    fn op(&mut self, p: usize, len: usize, kind: u16) -> usize {
        self.put(p, p + len, kind);
        self.prev = P_BEGIN;
        p + len
    }

    /// A name at `p`: a keyword, a builtin, or a name by where it stands.
    #[inline(always)]
    fn name(&mut self, p: usize) -> usize {
        let b = self.b;
        let mut e = p + 1;
        while e < b.len() && WORD[usize::from(b[e])] {
            e += 1;
        }
        let w = WORDS.value(&b[p..e], &INFO);
        if w == W_NONE || w > W_END {
            self.ident(p, e, w);
        } else {
            self.keyword(p, e, w);
        }
        e
    }

    /// A name `b[p..e]` that isn't a reserved word (`w` what else it is).
    #[inline(always)]
    fn ident(&mut self, p: usize, e: usize, w: u8) {
        let top = self.top();
        if self.prev > P_KEY {
            return self.ident_after(p, e, w);
        }
        let b = self.b;
        let q = skip(b, e);
        let next = b.get(q).copied().unwrap_or(0);
        // a table's key: `name =`, not `==`
        if top == F_TABLE && self.prev != P_BEGIN && next == b'=' && b.get(q + 1) != Some(&b'=') {
            return self.key(p, e, q);
        }
        self.variable(p, e, w, q, next);
    }

    /// A name in an operand's place, `q` past its blanks and `next` the byte there: a
    /// variable, a function's (`name = function`), a call's, or a builtin.
    #[inline(always)]
    fn variable(&mut self, p: usize, e: usize, w: u8, q: usize, next: u8) {
        if self.after_operand() {
            self.list = false;
        }
        let kind = match w {
            W_NONE => match next {
                b'(' | b'"' | b'\'' | b'{' => CALL,
                b'=' if fn_assign(self.b, q) => FUNCTION_NAME,
                b'[' if long_open(self.b, q).is_some() => CALL,
                _ => VARIABLE,
            },
            W_SELF => THIS,
            W_SUPPORT => SUPPORT_CONSTANT,
            W_BUILTIN => SUPPORT_FUNCTION,
            _ => {
                self.put(p, e, SUPPORT_CONSTANT);
                self.module = w - W_MODULE;
                self.prev = P_MODULE;
                return;
            }
        };
        self.put(p, e, kind);
        self.prev = P_END;
    }

    /// A table's key `b[p..e]`, its `=` at `q`.
    #[cold]
    #[inline(never)]
    fn key(&mut self, p: usize, e: usize, q: usize) {
        let w = &self.b[p..e];
        let method = w.starts_with(b"__") && is_metamethod(w);
        let kind = if fn_assign(self.b, q) {
            if method {
                KEY_METAMETHOD
            } else {
                FUNCTION_NAME
            }
        } else if method {
            KEY_METAMETHOD
        } else if w.starts_with(b"__") && is_metaproperty(w) {
            KEY_METAPROPERTY
        } else {
            KEY_LEAF
        };
        self.put(p, e, kind);
        self.prev = P_KEY;
    }

    /// A name after an accessor, `function`, a module's `.`, a label's `::` or `goto`.
    #[inline(never)]
    fn ident_after(&mut self, p: usize, e: usize, w: u8) {
        let b = self.b;
        let (kind, prev) = match self.prev {
            P_PROP => (self.property(p, e), P_END),
            P_MEMBER => (
                member(usize::from(self.module), &b[p..e]).unwrap_or_else(|| self.property(p, e)),
                P_END,
            ),
            P_FUNC => {
                let dot = matches!(b.get(skip(b, e)), Some(b'.' | b':'));
                (if dot { VARIABLE } else { FUNCTION_NAME }, P_FNEND)
            }
            P_FNPROP => {
                let kind = if matches!(b.get(skip(b, e)), Some(b'.' | b':')) {
                    PROPERTY_LEAF
                } else if is_metamethod(&b[p..e]) {
                    PROPERTY_METAMETHOD
                } else {
                    PROPERTY_FUNCTION_NAME
                };
                (kind, P_FNEND)
            }
            P_LABEL => (LABEL_NAME, P_LABEL_END),
            P_GOTO => (LABEL_VARIABLE, P_BEGIN),
            _ => {
                // after a function's name, a module's or a label's: a statement starts
                self.prev = P_END;
                return self.ident(p, e, w);
            }
        };
        self.put(p, e, kind);
        self.prev = prev;
    }

    /// A property `b[p..e]`, after an accessor: its kind.
    #[inline(always)]
    fn property(&self, p: usize, e: usize) -> u16 {
        let b = self.b;
        let w = &b[p..e];
        let meta = w.starts_with(b"__");
        let q = skip(b, e);
        match b.get(q) {
            Some(b'=') if fn_assign(b, q) => {
                if meta && is_metamethod(w) {
                    PROPERTY_METAMETHOD
                } else {
                    PROPERTY_FUNCTION_NAME
                }
            }
            Some(b'(' | b'"' | b'\'' | b'{') => PROPERTY_CALL,
            Some(b'[') if long_open(b, q).is_some() => PROPERTY_CALL,
            _ if meta && is_metaproperty(w) => PROPERTY_METAPROPERTY,
            _ => PROPERTY_LEAF,
        }
    }

    /// A reserved word `b[p..e]`, `w`.
    #[inline(always)]
    fn keyword(&mut self, p: usize, e: usize, w: u8) {
        if (self.prev > P_KEY || matches!(self.top(), F_CALL | F_PAREN | F_BRACKET))
            && self.misplaced(p, e, w)
        {
            return;
        }
        // a statement's word ends an expression list; the frame it opens or closes keeps
        // or gives back its own
        if !matches!(
            w,
            W_AND_OR | W_NOT | W_BOOLEAN | W_NIL | W_FUNCTION | W_LOCAL | W_RETURN | W_FOR | W_IN
        ) {
            self.list = false;
        }
        let (kind, prev) = match w {
            W_AND_OR | W_NOT => (LOGICAL, P_BEGIN),
            W_BOOLEAN => (BOOLEAN, P_END),
            W_NIL => (NULL, P_END),
            W_FUNCTION => (FUNCTION_KEYWORD, P_FUNC),
            W_LOCAL | W_RETURN => {
                self.list = true;
                (if w == W_LOCAL { LOCAL } else { RETURN }, P_BEGIN)
            }
            W_BREAK => (BREAK, P_BEGIN),
            W_GOTO => (GOTO_KEYWORD, P_GOTO),
            W_DO => {
                self.push(F_BLOCK);
                (CONTROL, P_BEGIN)
            }
            W_IF | W_ELSE => (CONDITIONAL, P_BEGIN),
            W_THEN => {
                self.push(F_BLOCK);
                (CONDITIONAL, P_BEGIN)
            }
            W_ELSEIF => {
                if self.top() == F_BLOCK {
                    self.pop();
                }
                (CONDITIONAL, P_BEGIN)
            }
            W_WHILE => (LOOP, P_BEGIN),
            W_FOR | W_IN => {
                self.list = true;
                (LOOP, P_BEGIN)
            }
            W_REPEAT => {
                self.push(F_REPEAT);
                (LOOP, P_BEGIN)
            }
            W_UNTIL => {
                if self.top() == F_REPEAT {
                    self.pop();
                }
                (LOOP, P_BEGIN)
            }
            _ => {
                // `end`
                if self.top() == F_BLOCK {
                    // a function's body in a list ends an operand; a statement's block, a
                    // statement
                    self.pop();
                    (END_KEYWORD, if self.list { P_END } else { P_BEGIN })
                } else {
                    (UNEXPECTED_END, P_BEGIN)
                }
            }
        };
        self.put(p, e, kind);
        self.prev = prev;
    }

    /// A reserved word where bat reads it as out of place, `invalid.unexpected-keyword`: after an
    /// accessor; a statement's word in a group or brackets (which it ends). Whether it was (in
    /// a function's parameters, [`Scan::params`]).
    #[cold]
    #[inline(never)]
    fn misplaced(&mut self, p: usize, e: usize, w: u8) -> bool {
        if matches!(self.prev, P_PROP | P_MEMBER) {
            self.put(p, e, UNEXPECTED_KEYWORD);
            self.prev = P_END;
            return true;
        }
        match self.top() {
            F_CALL | F_PAREN | F_BRACKET
                if !matches!(w, W_NOT | W_BOOLEAN | W_NIL | W_FUNCTION)
                    && !(w == W_AND_OR && self.after_operand()) =>
            {
                self.put(p, e, UNEXPECTED_KEYWORD);
                self.pop();
                self.prev = P_END;
                true
            }
            _ => false,
        }
    }

    /// A number at `p` (a digit, or `.` and one).
    #[inline(always)]
    fn number(&mut self, p: usize) -> usize {
        if self.after_operand() {
            self.list = false;
        }
        let Some((e, kind, caps)) = number(self.b, p) else {
            return p + 1;
        };
        let mut at = p;
        for (s, ce, k) in caps {
            if ce > s {
                self.put(at, s, kind);
                self.put(s, ce, k);
                at = ce;
            }
        }
        self.put(at, e, kind);
        self.prev = P_END;
        e
    }

    /// `.` at `p`: `...`, a number, `..`, or an accessor.
    #[inline(always)]
    fn dot(&mut self, p: usize) -> usize {
        let b = self.b;
        let operand = self.after_operand();
        if b.get(p + 1) == Some(&b'.') {
            if !operand && b.get(p + 2) == Some(&b'.') {
                self.put(p, p + 3, LANGUAGE);
                self.prev = P_END;
                return p + 3;
            }
            return self.op(p, 2, CONCATENATION);
        }
        if !operand && b.get(p + 1).is_some_and(u8::is_ascii_digit) {
            return self.number(p);
        }
        self.put(p, p + 1, ACCESSOR);
        self.prev = match self.prev {
            P_MODULE => P_MEMBER,
            P_FNEND => P_FNPROP,
            _ => P_PROP,
        };
        p + 1
    }

    /// `[` at `p`: a long string, or a bracket.
    #[inline(always)]
    fn lbracket(&mut self, p: usize) -> usize {
        if let Some((e, level)) = long_open(self.b, p) {
            self.put(p, e, MULTILINE + BEGIN);
            self.prev = P_END;
            self.level = u16::try_from(level).unwrap_or(u16::MAX);
            return self.long(e, M_STRING);
        }
        self.push(F_BRACKET);
        self.put(p, p + 1, BRACKETS_BEGIN);
        self.prev = P_BEGIN;
        p + 1
    }

    /// A long bracket's text from `p` to its close at [`Scan::level`] (`mode` a string's or a
    /// comment's), or to the line's end, the mode kept.
    #[inline(always)]
    fn long(&mut self, p: usize, mode: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let (text, end) = if mode == M_STRING {
            (MULTILINE, MULTILINE + END)
        } else {
            (BLOCK_COMMENT, BLOCK_END)
        };
        let level = usize::from(self.level);
        let mut q = p;
        loop {
            q = find_in(b, q, n, *b"]");
            if q >= n {
                self.put(p, n, text);
                self.mode = mode;
                return n;
            }
            let c = q + 1 + level;
            if c < n && b[c] == b']' && b[q + 1..c].iter().all(|&x| x == b'=') {
                self.put(p, q, text);
                self.put(q, c + 1, end);
                self.mode = M_CODE;
                return c + 1;
            }
            q += 1;
        }
    }

    /// `--` at `p`: a block comment or a line's.
    #[inline(always)]
    fn comment(&mut self, p: usize) -> usize {
        if let Some((e, level)) = long_open(self.b, p + 2) {
            self.put(p, e, BLOCK_BEGIN);
            self.level = u16::try_from(level).unwrap_or(u16::MAX);
            return self.long(e, M_COMMENT);
        }
        let n = self.b.len();
        self.put(p, p + 2, LINE_MARK);
        self.put(p + 2, n, LINE_COMMENT);
        n
    }

    /// A quoted string at `p`, its quote `q`.
    #[inline(always)]
    fn string(&mut self, p: usize, q: u8) -> usize {
        let base = if q == b'\'' { SINGLE } else { DOUBLE };
        self.put(p, p + 1, base + BEGIN);
        self.prev = P_END;
        self.text(p + 1, q, base)
    }

    /// A quoted string's text from `p`: to its closing quote, or the line's end.
    #[inline(always)]
    fn text(&mut self, mut p: usize, q: u8, base: u16) -> usize {
        let b = self.b;
        let n = b.len();
        loop {
            let e = find_in(b, p, n, [q, b'\\']);
            self.put(p, e, base);
            if e >= n {
                return n;
            }
            if b[e] == q {
                self.put(e, e + 1, base + END);
                return e + 1;
            }
            p = self.escape(e, q, base);
            if p >= n {
                return n;
            }
        }
    }

    /// An escape at `p`, in a string of `q` (`base` its text's kind): where it ends.
    #[cold]
    #[inline(never)]
    fn escape(&mut self, p: usize, q: u8, base: u16) -> usize {
        let b = self.b;
        let n = b.len();
        let Some(&c) = b.get(p + 1) else {
            // the line's `\n` escaped: the string goes on
            self.put(p, n, base + ESCAPE);
            self.mode = if q == b'\'' { M_SINGLE } else { M_DOUBLE };
            return n;
        };
        let hex = |i: usize| b.get(i).is_some_and(u8::is_ascii_hexdigit);
        let (e, kind) = match c {
            b'a' | b'b' | b'f' | b'n' | b'r' | b't' | b'v' | b'\\' | b'\'' | b'"' | b'[' | b']' => {
                (p + 2, ESCAPE)
            }
            b'x' if hex(p + 2) && hex(p + 3) => (p + 4, ESCAPE_HEX),
            b'0'..=b'9' => {
                let mut e = p + 2;
                while e < (p + 4).min(n) && b[e].is_ascii_digit() {
                    e += 1;
                }
                (e, ESCAPE_DEC)
            }
            b'u' if b.get(p + 2) == Some(&b'{') && hex(p + 3) => {
                let mut e = p + 4;
                while hex(e) {
                    e += 1;
                }
                if b.get(e) == Some(&b'}') {
                    (e + 1, ESCAPE_UNICODE)
                } else {
                    (p + 2, INVALID_ESCAPE)
                }
            }
            b'z' => {
                let e = skip(b, p + 2);
                self.put(p, e, base + ESCAPE_SPACES);
                if e >= n {
                    self.mode = if q == b'\'' { M_Z_SINGLE } else { M_Z_DOUBLE };
                }
                return e;
            }
            _ => (p + 1 + char_len(c), INVALID_ESCAPE),
        };
        let e = e.min(n);
        self.put(p, e, base + kind);
        e
    }

    /// The rest of a mode the line before left open: where the code starts.
    #[cold]
    #[inline(never)]
    fn resume(&mut self) -> usize {
        let mode = self.mode;
        self.mode = M_CODE;
        match mode {
            M_STRING | M_COMMENT => self.long(0, mode),
            M_SINGLE => self.text(0, b'\'', SINGLE),
            M_DOUBLE => self.text(0, b'"', DOUBLE),
            _ => {
                let (q, base) = if mode == M_Z_SINGLE {
                    (b'\'', SINGLE)
                } else {
                    (b'"', DOUBLE)
                };
                let e = skip(self.b, 0);
                self.put(0, e, base + ESCAPE_SPACES);
                if e >= self.b.len() {
                    self.mode = mode;
                    return e;
                }
                self.text(e, q, base)
            }
        }
    }
}

/// Whether `\s*=\s*function\b` is at `q`, its `=` there: a function assigned.
#[inline(always)]
fn fn_assign(b: &[u8], q: usize) -> bool {
    let r = skip(b, q + 1);
    b.get(r..r + 8) == Some(b"function") && !b.get(r + 8).is_some_and(|&c| WORD[usize::from(c)])
}

/// Past the blanks at `p`.
#[inline(always)]
fn skip(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && ACT[usize::from(b[p])] == A_SPACE {
        p += 1;
    }
    p
}

/// A module's member at its `.`: its constants', then its functions' leaf, or none.
fn member(module: usize, w: &[u8]) -> Option<u16> {
    let constant = match module {
        1 => matches!(
            w,
            b"config" | b"cpath" | b"loaded" | b"path" | b"preload" | b"searchers" | b"loaders"
        ),
        3 => w == b"charpattern",
        5 => matches!(w, b"huge" | b"maxinteger" | b"mininteger" | b"pi"),
        6 => matches!(w, b"stderr" | b"stdin" | b"stdout"),
        _ => false,
    };
    if constant {
        return Some(MEMBER_CONSTANT);
    }
    let function = match module {
        0 => matches!(
            w,
            b"create" | b"isyieldable" | b"resume" | b"running" | b"status" | b"wrap" | b"yield"
        ),
        1 => matches!(w, b"loadlib" | b"searchpath" | b"seeall"),
        2 => matches!(
            w,
            b"byte"
                | b"char"
                | b"dump"
                | b"find"
                | b"format"
                | b"gmatch"
                | b"gsub"
                | b"len"
                | b"lower"
                | b"match"
                | b"pack"
                | b"packsize"
                | b"rep"
                | b"reverse"
                | b"sub"
                | b"unpack"
                | b"upper"
        ),
        3 => matches!(w, b"char" | b"codes" | b"codepoint" | b"len" | b"offset"),
        4 => matches!(
            w,
            b"concat" | b"insert" | b"move" | b"pack" | b"remove" | b"sort" | b"unpack" | b"maxn"
        ),
        5 => matches!(
            w,
            b"abs"
                | b"acos"
                | b"asin"
                | b"atan"
                | b"ceil"
                | b"cos"
                | b"deg"
                | b"exp"
                | b"floor"
                | b"fmod"
                | b"log"
                | b"max"
                | b"min"
                | b"modf"
                | b"rad"
                | b"random"
                | b"randomseed"
                | b"sin"
                | b"sqrt"
                | b"tan"
                | b"tointeger"
                | b"type"
                | b"ult"
                | b"atan2"
                | b"cosh"
                | b"frexp"
                | b"ldexp"
                | b"log10"
                | b"pow"
                | b"sinh"
                | b"tanh"
        ),
        6 => matches!(
            w,
            b"close"
                | b"flush"
                | b"input"
                | b"lines"
                | b"open"
                | b"output"
                | b"popen"
                | b"read"
                | b"tmpfile"
                | b"type"
                | b"write"
        ),
        7 => matches!(
            w,
            b"clock"
                | b"date"
                | b"difftime"
                | b"execute"
                | b"exit"
                | b"getenv"
                | b"remove"
                | b"rename"
                | b"setlocale"
                | b"time"
                | b"tmpname"
        ),
        8 => matches!(
            w,
            b"debug"
                | b"gethook"
                | b"getinfo"
                | b"getlocal"
                | b"getmetatable"
                | b"getregistry"
                | b"getupvalue"
                | b"getuservalue"
                | b"sethook"
                | b"setlocal"
                | b"setmetatable"
                | b"setupvalue"
                | b"setuservalue"
                | b"traceback"
                | b"upvalueid"
                | b"upvaluejoin"
                | b"getfenv"
                | b"setfenv"
        ),
        9 => matches!(
            w,
            b"arshift"
                | b"band"
                | b"bnot"
                | b"bor"
                | b"btest"
                | b"bxor"
                | b"extract"
                | b"lrotate"
                | b"lshift"
                | b"replace"
                | b"rrotate"
                | b"rshift"
        ),
        _ => false,
    };
    function.then_some(MEMBER_FUNCTION)
}

/// `{{metamethod}}`, the word after its `__`.
fn is_metamethod(w: &[u8]) -> bool {
    w.strip_prefix(b"__").is_some_and(|m| {
        matches!(
            m,
            b"index"
                | b"newindex"
                | b"call"
                | b"tostring"
                | b"len"
                | b"pairs"
                | b"ipairs"
                | b"gc"
                | b"unm"
                | b"add"
                | b"sub"
                | b"mul"
                | b"div"
                | b"idiv"
                | b"mod"
                | b"pow"
                | b"concat"
                | b"band"
                | b"bor"
                | b"bxor"
                | b"bnot"
                | b"shl"
                | b"shr"
                | b"eq"
                | b"lt"
                | b"le"
        )
    })
}

/// `{{metaproperty}}`.
fn is_metaproperty(w: &[u8]) -> bool {
    matches!(w, b"__metatable" | b"__mode")
}

/// `\d*` at `p`: where it ends.
fn digits(b: &[u8], mut p: usize) -> usize {
    while let Some(e) = digit(b, p) {
        p = e;
    }
    p
}

/// `\h*` at `p`: where it ends.
fn hex_digits(b: &[u8], mut p: usize) -> usize {
    while b.get(p).is_some_and(u8::is_ascii_hexdigit) {
        p += 1;
    }
    p
}

/// `[Ee][-+]?\d*` (or with `[Pp]`) at `p`: where it ends.
fn exponent(b: &[u8], p: usize, mark: u8) -> Option<usize> {
    if b.get(p).map(u8::to_ascii_lowercase) != Some(mark) {
        return None;
    }
    let q = p + usize::from(matches!(b.get(p + 1), Some(b'-' | b'+')));
    Some(digits(b, q + 1))
}

/// A number at `p`: its end, its leaf, and its captures as `(start, end, leaf)`, in order.
type Number = (usize, u16, [(usize, usize, u16); 2]);

/// `number` at `p`, its patterns in their order.
fn number(b: &[u8], p: usize) -> Option<Number> {
    const NONE: (usize, usize, u16) = (0, 0, PLAIN);
    let is = |q: usize, c: u8| b.get(q) == Some(&c);
    if is(p, b'0') && matches!(b.get(p + 1), Some(b'x' | b'X')) {
        let base = (p, p + 2, FLOAT_HEX + 1);
        let h = hex_digits(b, p + 2);
        // `(0[Xx])\h*(?:(\.)\h*{{hex_exponent}}?|{{hex_exponent}})`
        if is(h, b'.') {
            let f = hex_digits(b, h + 1);
            let e = exponent(b, f, b'p').unwrap_or(f);
            return Some((e, FLOAT_HEX, [base, (h, h + 1, FLOAT_HEX + 2)]));
        }
        if let Some(e) = exponent(b, h, b'p') {
            return Some((e, FLOAT_HEX, [base, NONE]));
        }
        // `(0[Xx])\h+`
        if h > p + 2 {
            return Some((h, INT_HEX, [(p, p + 2, INT_HEX + 1), NONE]));
        }
    }
    // `\d+(?:(\.)\d*{{dec_exponent}}?|{{dec_exponent}})|(\.)\d+{{dec_exponent}}?`, then `\d+`
    if let Some(d) = digit(b, p) {
        let d = digits(b, d);
        if is(d, b'.') {
            let f = digits(b, d + 1);
            let e = exponent(b, f, b'e').unwrap_or(f);
            return Some((e, FLOAT_DEC, [(d, d + 1, FLOAT_DEC + 1), NONE]));
        }
        if let Some(e) = exponent(b, d, b'e') {
            return Some((e, FLOAT_DEC, [NONE, NONE]));
        }
        return Some((d, INT_DEC, [NONE, NONE]));
    }
    if is(p, b'.')
        && let Some(f) = digit(b, p + 1)
    {
        let f = digits(b, f);
        let e = exponent(b, f, b'e').unwrap_or(f);
        return Some((e, FLOAT_DEC, [(p, p + 1, FLOAT_DEC + 1), NONE]));
    }
    None
}

/// A long bracket's opening at `p`, `\[(=*)\[`: its end and its level.
fn long_open(b: &[u8], p: usize) -> Option<(usize, usize)> {
    if b.get(p) != Some(&b'[') {
        return None;
    }
    let mut q = p + 1;
    while b.get(q) == Some(&b'=') {
        q += 1;
    }
    (b.get(q) == Some(&b'[')).then_some((q + 1, q - p - 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Lang;
    use crate::tests::marked;

    fn of(lines: &[&str]) -> Vec<(String, String)> {
        marked(Lang::Lua, lines)
            .into_iter()
            .map(|(t, s)| (t.to_owned(), s.to_owned()))
            .collect()
    }

    fn has(lines: &[&str], text: &str, scopes: &str) {
        let got = of(lines);
        assert!(
            got.iter().any(|(t, s)| t == text && s == scopes),
            "{text:?} as {scopes:?} in {got:#?}"
        );
    }

    #[test]
    fn statements_and_functions() {
        let l = [
            "local M = {}",
            "function M.util:run(a, b, ...) -- go",
            "  for i = 1, #a do print(a[i]) end",
            "  return setmetatable({ __index = M, x = 1 }, { __call = function() end })",
            "end",
            "::top:: goto top",
        ];
        has(&l, "local", "storage.modifier.lua");
        has(&l, "M", "variable.other.lua");
        has(&l, "util", "meta.property.lua");
        has(&l, "run", "meta.property.lua entity.name.function.lua");
        has(&l, "a", "variable.parameter.function.lua");
        has(&l, "...", "constant.language.lua");
        has(
            &l,
            "--",
            "comment.line.lua punctuation.definition.comment.lua",
        );
        has(&l, "for", "keyword.control.loop.lua");
        has(&l, "#", "keyword.operator.length.lua");
        has(&l, "print", "support.function.builtin.lua");
        has(
            &l,
            "__index",
            "entity.name.function.lua support.function.metamethod.lua",
        );
        has(&l, "x", "string.unquoted.key.lua");
        has(
            &l,
            "__call",
            "entity.name.function.lua support.function.metamethod.lua",
        );
        has(&l, "top", "entity.name.label.lua");
        has(&l, "top", "variable.label.lua");
    }

    #[test]
    fn strings_numbers_and_comments() {
        let l = [
            "local s = 'a\\n\\x41\\65\\u{48}\\q' .. \"b\\z",
            "   c\" .. [==[long",
            "]] still]==] --[[ block",
            "]] x = 0x1p4 + 0xA.8 + 1e-3 + .5 + 0xff",
            "string.format('%d', math.pi)",
        ];
        has(
            &l,
            "\\n",
            "string.quoted.single.lua constant.character.escape.lua",
        );
        has(
            &l,
            "\\x41",
            "string.quoted.single.lua constant.character.escape.hexadecimal.lua",
        );
        has(
            &l,
            "\\q",
            "string.quoted.single.lua invalid.illegal.invalid-escape.lua",
        );
        has(
            &l,
            "   ",
            "string.quoted.double.lua constant.character.escape.whitespace.lua",
        );
        has(&l, "]] still", "string.quoted.multiline.lua");
        has(
            &l,
            "--[[",
            "comment.block.lua punctuation.definition.comment.begin.lua",
        );
        has(&l, "1p4", "constant.numeric.float.hexadecimal.lua");
        has(&l, "ff", "constant.numeric.integer.hexadecimal.lua");
        has(
            &l,
            "format",
            "meta.property.lua support.function.builtin.lua",
        );
        has(&l, "pi", "meta.property.lua support.constant.builtin.lua");
    }

    /// `text` a line at a time on the line API: each line's spans and the state after it.
    fn lines(text: &str) -> Vec<(Vec<crate::Span>, State)> {
        let mut state = State::default();
        let mut out = Vec::new();
        for l in text.lines() {
            let mut spans = Vec::new();
            line(l.as_bytes(), &mut state, &mut Out::new(&mut spans));
            out.push((spans, state));
        }
        out
    }

    #[test]
    fn a_block_lexes_as_its_lines() {
        // every field a line hands the next: frames past the levels, a long bracket's level, a
        // string continued by `\` and by `\z`, the token before, an expression list
        let deep = "{ ".repeat(50) + &"} ".repeat(48);
        let text = format!(
            "local t = {deep}\n}}}}\nx = [==[ a\n]=] b ]==] s = 'a\\\nb' z = \"c\\z\n   d\"\n\
             --[[ c\n]] function M.f\n(a,\nb) return a,\nb end\nfoo\n(1) local\nx, y = 1, 2\n"
        );
        let one = lines(&text);
        let mut state = State::default();
        let mut all = Vec::new();
        let mut v = Vec::new();
        block(&mut state, &mut v, &mut |c| {
            for l in text.lines() {
                all.push(c.next(l.as_bytes()).to_vec());
            }
            None
        });
        assert_eq!(all, one.iter().map(|(s, _)| s.clone()).collect::<Vec<_>>());
        assert_eq!(state, one.last().unwrap().1);
        assert!(!one[0].1.is_exact());
    }

    #[test]
    fn long_lines_lex_in_linear_time() {
        for unit in [
            "a.b(",
            "{ k = ",
            "x = 'q\\z",
            "f [[",
            "::a:: ",
            "function f(",
            "t[",
            "--[[ ",
        ] {
            let l = unit.repeat(20_000);
            let t = std::time::Instant::now();
            lines(&l);
            assert!(t.elapsed().as_millis() < 200, "{unit:?}: {:?}", t.elapsed());
        }
    }
}
