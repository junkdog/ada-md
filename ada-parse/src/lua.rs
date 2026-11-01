//! Lua by a scanner of ada's own (ADR-40, speed first): bat's `Lua` grammar compiled by hand,
//! as Go's and Python's were. Each of its contexts that can be open is an arm of one `match` on
//! the top of a stack, trying its patterns in the grammar's order at the place the scan has
//! reached, which is syntect's leftmost match; where no pattern can start, the scan steps on.
//!
//! The grammar (83 contexts and 155 patterns as syntect holds it, no embeds) has a
//! **prototype**, the two comments, that every context but the strings and the comments tries
//! first, so a `--` starts a comment wherever the scan is, even in a `\z` escape's spaces
//! inside a string. The contexts whose only pattern besides it is `''` (`expression`, the
//! meta pops after a call, a function and a table's value) are applied in place where no `--`
//! follows ([`Scan::empty_matches`]), which is what syntect's next turns do there. Its expressions are a push of
//! `expression` that a lookahead makes and that sets `expression-end` over `expression-begin`,
//! each popping at the first byte it doesn't take (`(?=\S)`), so **syntect's guard against a
//! loop** is followed: a pop that matches nothing right after a push that matched nothing
//! there, at the depth that push left, is passed over for the context's later patterns, else
//! the scan steps a character (a `,` in parentheses, which the grammar doesn't take, is
//! stepped over this way). Long brackets (`[==[`, `--[==[`) end at the same number of `=`,
//! kept with the state. A kind is a leaf, bat's scopes for a token (its pattern's and its
//! captures'); of the context's meta scopes only those a theme can see are kept: a string's, a
//! comment's and a `\z` escape's. Names and keywords are ASCII (`[A-Za-z_][A-Za-z0-9_]*`), `\d`
//! Unicode's decimal digits, `\s` Unicode's spaces.
//!
//! The stack between lines is the contexts open above the top level, a byte each in
//! [`State`]'s 28 levels. Lua nests deeper than that (a callback in a call is six contexts, a
//! table's value three), so a stack deeper than 28 is kept in [`RUNS`]: the runs of contexts
//! that deep stacks repeat, a byte each.

use crate::machine::KEPT as HALF;
use crate::{Out, State};

// The contexts, by bat's names where they have one.
/// `main`: `statements`.
const MAIN: u8 = 0;
/// `block-contents`: `do … end` and a function's body.
const BLOCK: u8 = 1;
/// `if-block`.
const IF_BLOCK: u8 = 2;
/// `repeat`'s block, until `until`.
const REPEAT_BLOCK: u8 = 3;
/// `repeat`'s `until`.
const UNTIL: u8 = 4;
/// `while`'s and `for`'s `do`.
const DO: u8 = 5;
/// `for`'s `=` or `in`.
const FOR_IN: u8 = 6;
/// A label's name, after `::`.
const LABEL: u8 = 7;
/// A label's closing `::`.
const LABEL_END: u8 = 8;
/// `goto`'s label.
const GOTO: u8 = 9;
/// `if-then`.
const IF_THEN: u8 = 10;
/// `expression`.
const EXPRESSION: u8 = 11;
/// `expression-list`.
const EXPRESSION_LIST: u8 = 12;
/// `expression-end`.
const EXPR_END: u8 = 13;
/// `expression-list-end`.
const EXPR_LIST_END: u8 = 14;
/// `expression-begin`.
const EXPR_BEGIN: u8 = 15;
/// `function-arguments-meta`.
const ARGS_META: u8 = 16;
/// `function-arguments`.
const ARGS: u8 = 17;
/// A call's `( … )`.
const ARGS_GROUP: u8 = 18;
/// `parenthesized-expression`'s `( … )`.
const PAREN_GROUP: u8 = 19;
/// An accessor's `[ … ]`.
const BRACKETS: u8 = 20;
/// `property`.
const PROPERTY: u8 = 21;
/// `table-constructor`'s `{ … }`.
const MAPPING: u8 = 22;
/// A table's `[ … ]` key.
const KEY_BRACKETS: u8 = 23;
/// A table's `name =` key.
const KEY: u8 = 24;
/// A table's value after `=` (an `immediately-pop` under its expression).
const VALUE_META: u8 = 25;
/// `function-meta`.
const FUNCTION_META: u8 = 26;
/// `function-name`.
const FNAME: u8 = 27;
/// `function-name-begin`.
const FNAME_BEGIN: u8 = 28;
/// `function-name-end`.
const FNAME_END: u8 = 29;
/// `function-name-property`.
const FNAME_PROPERTY: u8 = 30;
/// `function-parameter-list`.
const PARAMS: u8 = 31;
/// A function's `( … )` of parameters.
const PARAM_GROUP: u8 = 32;
/// A string's `\z` escape: the spaces after it.
const ESCAPE_Z: u8 = 33;
/// A builtin module's name, before its `.` (the modules in [`MODULES`]'s order).
const MODULE: u8 = 34;
/// A builtin module's member, after its `.`.
const MEMBER: u8 = 44;
/// The contexts from here on are text: no prototype.
const STRING_SINGLE: u8 = 54;
const STRING_DOUBLE: u8 = 55;
/// `[==[ … ]==]`.
const LONG_STRING: u8 = 56;
const COMMENT_LINE: u8 = 57;
/// `--[==[ … ]==]`.
const COMMENT_BLOCK: u8 = 58;

/// The leaves: a token's scopes.
static LEAVES: [&str; 98] = [
    "",
    // comments: the text, its mark; a block's text, its begin, its end; then the same in a
    // `\z` escape's spaces in a single-quoted string, and in a double-quoted one
    "comment.line.lua",
    "comment.line.lua punctuation.definition.comment.lua",
    "comment.block.lua",
    "comment.block.lua punctuation.definition.comment.begin.lua",
    "comment.block.lua punctuation.definition.comment.end.lua",
    "string.quoted.single.lua constant.character.escape.whitespace.lua comment.line.lua",
    "string.quoted.single.lua constant.character.escape.whitespace.lua comment.line.lua punctuation.definition.comment.lua",
    "string.quoted.single.lua constant.character.escape.whitespace.lua comment.block.lua",
    "string.quoted.single.lua constant.character.escape.whitespace.lua comment.block.lua punctuation.definition.comment.begin.lua",
    "string.quoted.single.lua constant.character.escape.whitespace.lua comment.block.lua punctuation.definition.comment.end.lua",
    "string.quoted.double.lua constant.character.escape.whitespace.lua comment.line.lua",
    "string.quoted.double.lua constant.character.escape.whitespace.lua comment.line.lua punctuation.definition.comment.lua",
    "string.quoted.double.lua constant.character.escape.whitespace.lua comment.block.lua",
    "string.quoted.double.lua constant.character.escape.whitespace.lua comment.block.lua punctuation.definition.comment.begin.lua",
    "string.quoted.double.lua constant.character.escape.whitespace.lua comment.block.lua punctuation.definition.comment.end.lua",
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
/// How far a comment's leaves in a `\z` escape of a single-quoted string are; twice that, of a
/// double-quoted one.
const IN_ESCAPE: u16 = 5;
const SINGLE: u16 = 16;
const DOUBLE: u16 = 25;
// a string's leaves, from its text
const BEGIN: u16 = 1;
const END: u16 = 2;
const ESCAPE: u16 = 3;
const ESCAPE_HEX: u16 = 4;
const ESCAPE_DEC: u16 = 5;
const ESCAPE_UNICODE: u16 = 6;
const ESCAPE_SPACES: u16 = 7;
const INVALID_ESCAPE: u16 = 8;
const MULTILINE: u16 = 34;
const TERMINATOR: u16 = 37;
const FUNCTION_KEYWORD: u16 = 38;
const BREAK: u16 = 39;
const RETURN: u16 = 40;
const LOCAL: u16 = 41;
const UNEXPECTED_END: u16 = 42;
const GROUP_BEGIN: u16 = 43;
const GROUP_END: u16 = 44;
const COMMA: u16 = 45;
const UNEXPECTED_KEYWORD: u16 = 46;
const PARAMETER: u16 = 47;
const LANGUAGE: u16 = 48;
const ACCESSOR: u16 = 49;
const VARIABLE: u16 = 50;
const FUNCTION_NAME: u16 = 51;
const PROPERTY_LEAF: u16 = 52;
const PROPERTY_METAMETHOD: u16 = 53;
const PROPERTY_FUNCTION_NAME: u16 = 54;
const CONTROL: u16 = 55;
const CONDITIONAL: u16 = 56;
const LOOP: u16 = 57;
const LABEL_BEGIN: u16 = 58;
const LABEL_NAME: u16 = 59;
const LABEL_CLOSE: u16 = 60;
const GOTO_KEYWORD: u16 = 61;
const LABEL_VARIABLE: u16 = 62;
const END_KEYWORD: u16 = 63;
const COMPARISON: u16 = 64;
const ARITHMETIC: u16 = 65;
const BITWISE: u16 = 66;
const CONCATENATION: u16 = 67;
const ASSIGNMENT: u16 = 68;
const LOGICAL: u16 = 69;
const LENGTH: u16 = 70;
const BRACKETS_BEGIN: u16 = 71;
const BRACKETS_END: u16 = 72;
const PROPERTY_CALL: u16 = 73;
const PROPERTY_METAPROPERTY: u16 = 74;
const BOOLEAN: u16 = 75;
const NULL: u16 = 76;
const THIS: u16 = 77;
const SUPPORT_CONSTANT: u16 = 78;
const SUPPORT_FUNCTION: u16 = 79;
const CALL: u16 = 80;
const FLOAT_HEX: u16 = 81;
const INT_HEX: u16 = 84;
const FLOAT_DEC: u16 = 86;
const INT_DEC: u16 = 88;
const BRACES_BEGIN: u16 = 89;
const BRACES_END: u16 = 90;
const FIELD_SEPARATOR: u16 = 91;
const KEY_METAMETHOD: u16 = 92;
const KEY_METAPROPERTY: u16 = 93;
const KEY_LEAF: u16 = 94;
const KEY_VALUE: u16 = 95;
const MEMBER_FUNCTION: u16 = 96;
const MEMBER_CONSTANT: u16 = 97;

/// The scopes under `source.lua` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    LEAVES.get(usize::from(kind)).map_or_else(Vec::new, |l| {
        l.split(' ').filter(|s| !s.is_empty()).collect()
    })
}

/// How many contexts a line can hold; one pushed past it drops the bottom one.
const MAX: usize = 128;
/// How many the state keeps between lines: [`State`]'s levels and withs.
const KEPT: usize = 2 * HALF;
/// Matches that take nothing, in a row at one place, before the scan steps on regardless.
const STUCK: usize = 64;
/// syntect's stack limit (Go's): at it, a push's pattern matches nothing, so the patterns
/// after it get their turn.
const SYNTECT_DEPTH: usize = 99;

/// Lex `b`, a line of Lua without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let mut s = Scan {
        b,
        n: b.len(),
        stack: [MAIN; MAX],
        depth: 1,
        exact: state.is_exact(),
        level: usize::from(state.arg),
        at: 0,
        loop_at: (usize::MAX, 0),
        looped: false,
        out,
    };
    for i in 0..usize::from(state.depth) {
        let c = if i < HALF {
            state.levels[i]
        } else {
            state.withs[i - HALF]
        };
        match RUNS.get(usize::from(c).wrapping_sub(usize::from(RUN))) {
            Some(run) => run.iter().for_each(|&c| s.push(c)),
            None => s.push(c),
        }
    }
    s.run();
    s.fill(s.n);
    // a byte a context, or past the levels, a byte a run
    let open = &s.stack[1..s.depth];
    let mut runs;
    let codes = if open.len() <= KEPT {
        open
    } else {
        runs = [0; MAX];
        let (mut i, mut k) = (0, 0);
        while i < open.len() {
            let rest = &open[i..];
            let (code, len) = RUNS
                .iter()
                .position(|r| rest.starts_with(r))
                .map_or((rest[0], 1), |r| (RUN + r as u8, RUNS[r].len()));
            runs[k] = code;
            (i, k) = (i + len, k + 1);
        }
        &runs[..k]
    };
    let from = codes.len().saturating_sub(KEPT);
    let mut packed = State {
        depth: (codes.len() - from) as u8,
        inexact: !s.exact || from > 0,
        ..State::default()
    };
    if matches!(s.top(), LONG_STRING | COMMENT_BLOCK) {
        packed.arg = u16::try_from(s.level).unwrap_or(u16::MAX);
    }
    for (i, &c) in codes[from..].iter().enumerate() {
        if i < HALF {
            packed.levels[i] = c;
        } else {
            packed.withs[i - HALF] = c;
        }
    }
    *state = packed;
}

/// The first byte that stands for a run of [`RUNS`] (the contexts are below it).
const RUN: u8 = 64;

/// The runs of contexts that deep stacks repeat, longest first, each kept as a byte between
/// lines once a stack is deeper than [`KEPT`] (`RUN` and up): a call's open arguments, a
/// function among them and its body, a table's value; picked on the stacks of the corpus that
/// went past the levels.
static RUNS: [&[u8]; RUN_COUNT] = [
    &[
        BLOCK,
        EXPR_END,
        ARGS_META,
        ARGS_GROUP,
        EXPR_LIST_END,
        FUNCTION_META,
        BLOCK,
        EXPR_END,
    ],
    &[
        BLOCK,
        EXPR_END,
        ARGS_META,
        ARGS_GROUP,
        EXPR_LIST_END,
        FUNCTION_META,
        BLOCK,
        EXPR_LIST_END,
    ],
    &[
        DO,
        BLOCK,
        EXPR_END,
        ARGS_META,
        ARGS_GROUP,
        EXPR_LIST_END,
        FUNCTION_META,
        BLOCK,
    ],
    &[
        EXPR_LIST_END,
        FUNCTION_META,
        BLOCK,
        EXPR_END,
        ARGS_META,
        ARGS_GROUP,
        EXPR_LIST_END,
        FUNCTION_META,
    ],
    &[
        ARGS_META,
        ARGS_GROUP,
        EXPR_LIST_END,
        FUNCTION_META,
        BLOCK,
        EXPR_END,
        ARGS_META,
        ARGS_GROUP,
    ],
    &[
        ARGS_META,
        ARGS_GROUP,
        EXPR_LIST_END,
        FUNCTION_META,
        BLOCK,
        EXPR_LIST_END,
        ARGS_META,
        ARGS_GROUP,
    ],
    &[
        ARGS_GROUP,
        EXPR_LIST_END,
        FUNCTION_META,
        BLOCK,
        EXPR_END,
        ARGS_META,
        ARGS_GROUP,
        EXPR_LIST_END,
    ],
    &[
        ARGS_GROUP,
        EXPR_LIST_END,
        FUNCTION_META,
        BLOCK,
        EXPR_LIST_END,
        ARGS_META,
        ARGS_GROUP,
        EXPR_LIST_END,
    ],
    &[
        FUNCTION_META,
        BLOCK,
        EXPR_END,
        ARGS_META,
        ARGS_GROUP,
        EXPR_LIST_END,
        FUNCTION_META,
        BLOCK,
    ],
    &[
        BLOCK,
        EXPR_END,
        ARGS_META,
        ARGS_GROUP,
        EXPR_LIST_END,
        FUNCTION_META,
        BLOCK,
    ],
    &[
        EXPR_END,
        ARGS_META,
        ARGS_GROUP,
        EXPR_LIST_END,
        FUNCTION_META,
        BLOCK,
        EXPR_END,
    ],
    &[
        EXPR_END,
        ARGS_META,
        ARGS_GROUP,
        EXPR_LIST_END,
        FUNCTION_META,
        BLOCK,
        EXPR_LIST_END,
    ],
    &[
        EXPR_LIST_END,
        FUNCTION_META,
        BLOCK,
        EXPR_END,
        ARGS_META,
        ARGS_GROUP,
        EXPR_LIST_END,
    ],
    &[
        ARGS_META,
        ARGS_GROUP,
        EXPR_LIST_END,
        FUNCTION_META,
        BLOCK,
        EXPR_END,
        ARGS_META,
    ],
    &[DO, BLOCK, EXPR_END, ARGS_META, ARGS_GROUP, EXPR_LIST_END],
    &[
        EXPR_END,
        ARGS_META,
        ARGS_GROUP,
        EXPR_LIST_END,
        FUNCTION_META,
        BLOCK,
    ],
    &[
        EXPR_LIST_END,
        FUNCTION_META,
        BLOCK,
        EXPR_END,
        ARGS_META,
        ARGS_GROUP,
    ],
    &[
        ARGS_META,
        ARGS_GROUP,
        EXPR_LIST_END,
        FUNCTION_META,
        BLOCK,
        EXPR_END,
    ],
    &[
        ARGS_META,
        ARGS_GROUP,
        EXPR_LIST_END,
        FUNCTION_META,
        BLOCK,
        EXPR_LIST_END,
    ],
    &[MAPPING, VALUE_META, EXPR_END, MAPPING, VALUE_META, EXPR_END],
    &[
        FUNCTION_META,
        BLOCK,
        EXPR_END,
        ARGS_META,
        ARGS_GROUP,
        EXPR_LIST_END,
    ],
    &[BLOCK, EXPR_END, ARGS_META, ARGS_GROUP, EXPR_LIST_END],
    &[EXPR_END, MAPPING, VALUE_META, EXPR_END, MAPPING],
    &[EXPR_LIST_END, FUNCTION_META, BLOCK, EXPR_END, ARGS_META],
    &[ARGS_META, ARGS_GROUP, EXPR_LIST_END, FUNCTION_META, BLOCK],
    &[FUNCTION_META, BLOCK, EXPR_END, ARGS_META, ARGS_GROUP],
    &[BLOCK, EXPR_END, ARGS_META, ARGS_GROUP],
    &[EXPR_END, ARGS_META, ARGS_GROUP, EXPR_LIST_END],
    &[EXPR_END, MAPPING, VALUE_META, EXPR_END],
    &[EXPR_LIST_END, FUNCTION_META, BLOCK, EXPR_END],
    &[EXPR_LIST_END, FUNCTION_META, BLOCK, EXPR_LIST_END],
    &[ARGS_GROUP, EXPR_LIST_END, FUNCTION_META, BLOCK],
    &[MAPPING, VALUE_META, EXPR_END, MAPPING],
    &[FUNCTION_META, BLOCK, EXPR_END, ARGS_META],
    &[BLOCK, EXPR_END, ARGS_META],
    &[EXPR_END, ARGS_META, ARGS_GROUP],
    &[EXPR_LIST_END, FUNCTION_META, BLOCK],
    &[ARGS_META, ARGS_GROUP, EXPR_LIST_END],
    &[MAPPING, VALUE_META, EXPR_END],
    &[VALUE_META, EXPR_END, MAPPING],
    &[FUNCTION_META, BLOCK, EXPR_END],
    &[FUNCTION_META, BLOCK, EXPR_LIST_END],
    &[BLOCK, EXPR_END],
    &[EXPR_END, ARGS_META],
    &[ARGS_META, ARGS_GROUP],
    &[ARGS_GROUP, EXPR_LIST_END],
    &[FUNCTION_META, BLOCK],
];
const RUN_COUNT: usize = 47;

/// The builtin modules, in `builtin-modules`' order.
static MODULES: [&[u8]; 10] = [
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

/// `{{reserved_word}}`: whether the word `w` is one.
fn is_reserved(w: &[u8]) -> bool {
    matches!(
        w,
        b"and"
            | b"break"
            | b"do"
            | b"elseif"
            | b"else"
            | b"end"
            | b"false"
            | b"for"
            | b"function"
            | b"goto"
            | b"if"
            | b"in"
            | b"local"
            | b"nil"
            | b"not"
            | b"or"
            | b"repeat"
            | b"return"
            | b"then"
            | b"true"
            | b"until"
            | b"while"
    )
}

/// `{{reserved_word_statement}}`: the reserved words but `not`, `true`, `nil`, `false` and
/// `function`.
fn is_reserved_statement(w: &[u8]) -> bool {
    is_reserved(w) && !matches!(w, b"not" | b"true" | b"nil" | b"false" | b"function")
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

/// `support`'s builtin functions.
fn is_builtin_function(w: &[u8]) -> bool {
    matches!(
        w,
        b"assert"
            | b"collectgarbage"
            | b"dofile"
            | b"error"
            | b"getmetatable"
            | b"ipairs"
            | b"load"
            | b"loadfile"
            | b"next"
            | b"pairs"
            | b"pcall"
            | b"print"
            | b"rawequal"
            | b"rawget"
            | b"rawlen"
            | b"rawset"
            | b"select"
            | b"setmetatable"
            | b"tonumber"
            | b"tostring"
            | b"type"
            | b"xpcall"
            | b"require"
            | b"getfenv"
            | b"module"
            | b"setfenv"
            | b"unpack"
    )
}

/// ASCII's word bytes, `{{identifier_char}}`.
static WORD: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 128 {
        t[c] = (c as u8).is_ascii_alphanumeric() || c == b'_' as usize;
        c += 1;
    }
    t
};

/// The bytes of the UTF-8 character that starts with `c`.
fn char_len(c: u8) -> usize {
    match c {
        0xf0.. => 4,
        0xe0.. => 3,
        0xc0.. => 2,
        _ => 1,
    }
}

/// The character that starts at byte `i` of `b`, past ASCII.
#[cold]
fn wide_char(b: &[u8], i: usize) -> Option<char> {
    let e = (i + char_len(b[i])).min(b.len());
    std::str::from_utf8(&b[i..e]).ok()?.chars().next()
}

/// `\s` at `p` (the line's `\n` past its end isn't asked): how many bytes the space takes.
fn space_len(b: &[u8], p: usize) -> Option<usize> {
    let &c = b.get(p)?;
    if c < 0x80 {
        return matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r').then_some(1);
    }
    wide_space(b, p)
}

#[cold]
fn wide_space(b: &[u8], p: usize) -> Option<usize> {
    let c = wide_char(b, p)?;
    matches!(
        c,
        '\u{85}' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
    .then(|| c.len_utf8())
}

/// Where the spaces at `p` end.
fn skip_spaces(b: &[u8], mut p: usize) -> usize {
    while let Some(k) = space_len(b, p) {
        p += k;
    }
    p
}

/// Whether `c` is one of Unicode's decimal digits, `\p{Nd}`.
fn is_nd(c: char) -> bool {
    let u = c as u32;
    crate::csharp::ND.iter().any(|&s| u >= s && u - s < 10) || c.is_ascii_digit()
}

/// `\d` at `p`, Unicode's decimal digits: where it ends.
fn digit(b: &[u8], p: usize) -> Option<usize> {
    match *b.get(p)? {
        c if c.is_ascii_digit() => Some(p + 1),
        c if c < 0x80 => None,
        _ => wide_digit(b, p),
    }
}

#[cold]
fn wide_digit(b: &[u8], p: usize) -> Option<usize> {
    let c = wide_char(b, p)?;
    is_nd(c).then(|| p + c.len_utf8())
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

/// A long bracket's closing of `level` at `p`, `\]\1\]`: its end.
fn long_close(b: &[u8], p: usize, level: usize) -> Option<usize> {
    let e = p + level + 2;
    (b.get(p) == Some(&b']')
        && b.get(e - 1) == Some(&b']')
        && b[p + 1..e - 1].iter().all(|&c| c == b'='))
    .then_some(e)
}

struct Scan<'b, 'c> {
    b: &'b [u8],
    /// The line's length: where its `\n` is.
    n: usize,
    stack: [u8; MAX],
    depth: usize,
    exact: bool,
    /// The `=` of the long bracket open on top.
    level: usize,
    /// Where the spans so far end.
    at: usize,
    /// Where the last push that matched nothing was, and the depth it left: a pop there, at
    /// that depth, that matches nothing would loop, so syntect tries the patterns after it and
    /// else steps a character.
    loop_at: (usize, usize),
    /// A pop at the place was passed over (`loop_at`).
    looped: bool,
    out: &'b mut Out<'c>,
}

impl Scan<'_, '_> {
    fn top(&self) -> u8 {
        self.stack[self.depth - 1]
    }

    /// How far a comment's leaves are from the plain ones: by the `\z` escape and string under
    /// it, if any.
    fn comment_wrap(&self) -> u16 {
        if self.depth >= 3 && self.stack[self.depth - 2] == ESCAPE_Z {
            match self.stack[self.depth - 3] {
                STRING_SINGLE => return IN_ESCAPE,
                STRING_DOUBLE => return 2 * IN_ESCAPE,
                _ => {}
            }
        }
        0
    }

    /// The string a `\z` escape on top is in: its text's leaf.
    fn escape_host(&self) -> u16 {
        if self.depth >= 2 && self.stack[self.depth - 2] == STRING_SINGLE {
            SINGLE
        } else {
            DOUBLE
        }
    }

    /// The kind of text no pattern of the top context matches.
    fn text(&self) -> u16 {
        match self.top() {
            ..ESCAPE_Z => PLAIN,
            STRING_SINGLE => SINGLE,
            STRING_DOUBLE => DOUBLE,
            LONG_STRING => MULTILINE,
            COMMENT_LINE => LINE_COMMENT + self.comment_wrap(),
            COMMENT_BLOCK => BLOCK_COMMENT + self.comment_wrap(),
            ESCAPE_Z => self.escape_host() + ESCAPE_SPACES,
            _ => PLAIN,
        }
    }

    /// The text up to `p` is the top context's.
    fn fill(&mut self, p: usize) {
        let p = p.min(self.n);
        if p > self.at {
            let text = self.text();
            if text != PLAIN {
                self.out.push_merged(self.at, p, text);
            }
            self.at = p;
        }
    }

    /// Bytes `s..e` are `leaf`, after the text before them.
    fn put(&mut self, s: usize, e: usize, leaf: u16) {
        self.fill(s);
        let e = e.min(self.n);
        if leaf != PLAIN && e > self.at {
            self.out.push_merged(self.at, e, leaf);
        }
        self.at = self.at.max(e);
    }

    fn push(&mut self, c: u8) {
        if self.depth == MAX {
            self.stack.copy_within(2.., 1);
            self.depth -= 1;
            self.exact = false;
        }
        self.stack[self.depth] = c;
        self.depth += 1;
    }

    fn pop(&mut self) {
        if self.depth > 1 {
            self.depth -= 1;
        }
    }

    /// Replace the top with `cs`, bottom first.
    fn set(&mut self, cs: &[u8]) {
        self.pop();
        for &c in cs {
            self.push(c);
        }
    }

    /// `cs` pushed by a match that took nothing at `p`: syntect keeps the depth the first of
    /// them leaves for its guard against a loop.
    fn enter(&mut self, p: usize, cs: &[u8]) -> Option<usize> {
        if self.depth >= SYNTECT_DEPTH {
            return None;
        }
        self.fill(p);
        let depth = self.depth + 1;
        for &c in cs {
            self.push(c);
        }
        self.loop_at = (p, depth);
        Some(p)
    }

    /// The top replaced with `cs` by a match that took nothing at `p`.
    fn shift(&mut self, p: usize, cs: &[u8]) -> Option<usize> {
        self.fill(p);
        self.set(cs);
        Some(p)
    }

    /// A pop that takes nothing at `p`, unless it would loop: then syntect passes over it for
    /// the patterns after it (`None`).
    fn leave(&mut self, p: usize) -> Option<usize> {
        if self.loop_at == (p, self.depth) {
            self.looped = true;
            return None;
        }
        self.fill(p);
        self.pop();
        Some(p)
    }

    /// Bytes `s..e` are `leaf`; then `cs` are pushed.
    fn tok_push(&mut self, s: usize, e: usize, leaf: u16, cs: &[u8]) -> Option<usize> {
        if self.depth >= SYNTECT_DEPTH {
            return None;
        }
        self.fill(s);
        for &c in cs {
            self.push(c);
        }
        self.put(s, e, leaf);
        Some(e)
    }

    /// Bytes `s..e` are `leaf`, then the top context pops.
    fn close(&mut self, s: usize, e: usize, leaf: u16) -> Option<usize> {
        self.put(s, e, leaf);
        self.pop();
        Some(e)
    }

    /// Bytes `s..e` are `leaf`, then the top context is replaced with `cs`.
    fn then(&mut self, s: usize, e: usize, leaf: u16, cs: &[u8]) -> Option<usize> {
        self.put(s, e, leaf);
        self.set(cs);
        Some(e)
    }

    /// Bytes `s..e` are `leaf`, and the context stays.
    fn tok(&mut self, s: usize, e: usize, leaf: u16) -> Option<usize> {
        self.put(s, e, leaf);
        Some(e)
    }

    fn run(&mut self) {
        let n = self.n;
        let mut p = 0;
        let mut stuck = 0;
        loop {
            // where no pattern of the context starts at a space, the spaces are skipped whole
            if p < n && spaces_inert(self.top()) && space_len(self.b, p).is_some() {
                p = skip_spaces(self.b, p).min(n);
            }
            if p > n {
                return;
            }
            self.looped = false;
            if let Some(q) = self.at(p) {
                stuck = if q == p { stuck + 1 } else { 0 };
                p = q;
                self.empty_matches(p);
                if stuck > STUCK {
                    stuck = 0;
                    p = self.next(p);
                }
            } else if p >= n {
                return;
            } else {
                // after a pop passed over (syntect's guard), a step of a character; else the
                // next place a pattern can start
                p = if self.looped {
                    self.next(p)
                } else {
                    self.advance(p)
                };
                stuck = 0;
            }
        }
    }

    /// The contexts on top whose only pattern besides the prototype's comments is `''` (a set
    /// or a pop), applied at `p` where no comment starts: what the scan's next turns would do
    /// there, without them.
    fn empty_matches(&mut self, p: usize) {
        if p > self.n || self.b.get(p) == Some(&b'-') && self.b.get(p + 1) == Some(&b'-') {
            return;
        }
        loop {
            match self.top() {
                EXPRESSION => self.set(&[EXPR_END, EXPR_BEGIN]),
                EXPRESSION_LIST => self.set(&[EXPR_LIST_END, EXPR_BEGIN]),
                ARGS_META | VALUE_META | FUNCTION_META if self.loop_at != (p, self.depth) => {
                    self.pop();
                }
                _ => return,
            }
        }
    }

    /// Where the character at `p` ends (the line's `\n` is one byte).
    fn next(&self, p: usize) -> usize {
        if p >= self.n {
            return p + 1;
        }
        p + char_len(self.b[p]).min(self.n - p)
    }

    /// Where a pattern of the top context can next start, after none did at `p`.
    fn advance(&self, p: usize) -> usize {
        let b = self.b;
        let stop: fn(u8) -> bool = match self.top() {
            STRING_SINGLE => |c| matches!(c, b'\\' | b'\''),
            STRING_DOUBLE => |c| matches!(c, b'\\' | b'"'),
            LONG_STRING | COMMENT_BLOCK => |c| c == b']',
            COMMENT_LINE => return self.n,
            _ => return self.next(p),
        };
        let mut q = p + 1;
        while q < self.n && !stop(b[q]) {
            q += 1;
        }
        q
    }

    /// The ASCII word at `p`: where it ends (`p` for none).
    fn word_end(&self, p: usize) -> usize {
        let b = self.b;
        let mut e = p;
        while e < self.n && WORD[usize::from(b[e])] {
            e += 1;
        }
        e
    }

    /// The word at `p` is `w` (`w{{identifier_break}}`).
    fn word_is(&self, p: usize, w: &[u8]) -> bool {
        self.b[p.min(self.n)..].starts_with(w)
            && !self
                .b
                .get(p + w.len())
                .is_some_and(|&c| WORD[usize::from(c)])
    }

    /// `{{identifier}}` at `p`: where it ends.
    fn ident(&self, p: usize) -> Option<usize> {
        let &c = self.b.get(p)?;
        if !(c.is_ascii_alphabetic() || c == b'_') {
            return None;
        }
        let e = self.word_end(p);
        (!is_reserved(&self.b[p..e])).then_some(e)
    }

    /// `(?=\S)` at `p`.
    fn nonblank(&self, p: usize) -> bool {
        p < self.n && space_len(self.b, p).is_none()
    }

    /// `else-pop`.
    fn else_pop(&mut self, p: usize) -> Option<usize> {
        if self.nonblank(p) {
            self.leave(p)
        } else {
            None
        }
    }

    /// `\s*` then a byte `ok` takes, after `e`.
    fn ahead(&self, e: usize, ok: impl Fn(&[u8], usize) -> bool) -> bool {
        let q = skip_spaces(self.b, e);
        q < self.n && ok(self.b, q)
    }

    /// `{{function_call_ahead}}` after `e`.
    fn call_ahead(&self, e: usize) -> bool {
        self.ahead(e, args_begin)
    }

    /// `{{function_assignment_ahead}}` after `e`: `\s*=\s*function{{identifier_break}}`.
    fn assignment_ahead(&self, e: usize) -> bool {
        let q = skip_spaces(self.b, e);
        q < self.n && self.b[q] == b'=' && {
            let r = skip_spaces(self.b, q + 1);
            r < self.n && self.word_is(r, b"function")
        }
    }

    /// The first pattern of the top context that matches at `p`, applied: where the scan goes
    /// on, or `None` when none does.
    fn at(&mut self, p: usize) -> Option<usize> {
        let top = self.top();
        if top < STRING_SINGLE
            && let Some(q) = self.prototype(p)
        {
            return Some(q);
        }
        match top {
            MAIN => self.statements(p),
            BLOCK => self.end(p).or_else(|| self.statements(p)),
            IF_BLOCK => {
                if self.word_is(p, b"end") {
                    return self.close(p, p + 3, END_KEYWORD);
                }
                if self.word_is(p, b"elseif") {
                    return self.then(p, p + 6, CONDITIONAL, &[IF_THEN, EXPRESSION]);
                }
                if self.word_is(p, b"else") {
                    return self.then(p, p + 4, CONDITIONAL, &[IF_BLOCK]);
                }
                self.statements(p)
            }
            REPEAT_BLOCK => {
                if self.word_is(p, b"until")
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.statements(p)
            }
            UNTIL => {
                if self.word_is(p, b"until") {
                    return self.then(p, p + 5, LOOP, &[EXPRESSION]);
                }
                None
            }
            DO => {
                if self.word_is(p, b"do") {
                    return self.tok_push(p, p + 2, CONTROL, &[BLOCK]);
                }
                self.else_pop(p)
            }
            FOR_IN => {
                if self.b.get(p) == Some(&b'=') {
                    return self.shift(p, &[EXPR_LIST_END]);
                }
                if self.word_is(p, b"in") {
                    return self.then(p, p + 2, LOOP, &[EXPRESSION_LIST]);
                }
                self.else_pop(p)
            }
            LABEL => {
                if let Some(e) = self.ident(p) {
                    return self.then(p, e, LABEL_NAME, &[LABEL_END]);
                }
                self.else_pop(p)
            }
            LABEL_END => {
                if self.b[p.min(self.n)..].starts_with(b"::") {
                    return self.close(p, p + 2, LABEL_CLOSE);
                }
                self.else_pop(p)
            }
            GOTO => {
                if let Some(e) = self.ident(p) {
                    return self.close(p, e, LABEL_VARIABLE);
                }
                self.else_pop(p)
            }
            IF_THEN => {
                if self.word_is(p, b"then") {
                    return self.then(p, p + 4, CONDITIONAL, &[IF_BLOCK]);
                }
                self.else_pop(p)
            }
            EXPRESSION => self.shift(p, &[EXPR_END, EXPR_BEGIN]),
            EXPRESSION_LIST => self.shift(p, &[EXPR_LIST_END, EXPR_BEGIN]),
            EXPR_END => self.expression_end(p),
            EXPR_LIST_END => {
                if self.b.get(p) == Some(&b',') {
                    return self.tok_push(p, p + 1, COMMA, &[EXPR_BEGIN]);
                }
                self.expression_end(p)
            }
            EXPR_BEGIN => self.expression_begin(p),
            ARGS_META | VALUE_META | FUNCTION_META => self.leave(p),
            ARGS => self.arguments(p),
            ARGS_GROUP | PAREN_GROUP | BRACKETS => {
                let (close, leaf, inner) = match top {
                    ARGS_GROUP => (b')', GROUP_END, EXPRESSION_LIST),
                    PAREN_GROUP => (b')', GROUP_END, EXPRESSION),
                    _ => (b']', BRACKETS_END, EXPRESSION),
                };
                if self.b.get(p) == Some(&close) {
                    return self.close(p, p + 1, leaf);
                }
                let e = self.word_end(p);
                if e > p && is_reserved_statement(&self.b[p..e]) {
                    return self.close(p, e, UNEXPECTED_KEYWORD);
                }
                if self.nonblank(p) {
                    return self.enter(p, &[inner]);
                }
                None
            }
            PROPERTY => self.property(p),
            MAPPING => self.mapping(p),
            KEY_BRACKETS => {
                if self.b.get(p) == Some(&b']') {
                    return self.close(p, p + 1, BRACKETS_END);
                }
                if self.nonblank(p) {
                    return self.enter(p, &[EXPRESSION]);
                }
                None
            }
            KEY => self.key(p),
            FNAME => {
                if self.nonblank(p) {
                    return self.shift(p, &[FNAME_END, FNAME_BEGIN]);
                }
                None
            }
            FNAME_BEGIN => {
                if let Some(e) = self.ident(p) {
                    let leaf = if self.ahead(e, |b, q| matches!(b[q], b'.' | b':')) {
                        VARIABLE
                    } else {
                        FUNCTION_NAME
                    };
                    return self.close(p, e, leaf);
                }
                self.else_pop(p)
            }
            FNAME_END => {
                if matches!(self.b.get(p), Some(b'.' | b':')) {
                    return self.tok_push(p, p + 1, ACCESSOR, &[FNAME_PROPERTY]);
                }
                self.else_pop(p)
            }
            FNAME_PROPERTY => {
                if let Some(e) = self.ident(p)
                    && self.ahead(e, |b, q| matches!(b[q], b'.' | b':'))
                {
                    return self.close(p, e, PROPERTY_LEAF);
                }
                let e = self.word_end(p);
                if is_metamethod(&self.b[p..e]) {
                    return self.close(p, e, PROPERTY_METAMETHOD);
                }
                if let Some(e) = self.ident(p) {
                    return self.close(p, e, PROPERTY_FUNCTION_NAME);
                }
                self.else_pop(p)
            }
            PARAMS => {
                if self.b.get(p) == Some(&b'(') {
                    return self.then(p, p + 1, GROUP_BEGIN, &[PARAM_GROUP]);
                }
                self.else_pop(p)
            }
            PARAM_GROUP => {
                match self.b.get(p) {
                    Some(b')') => return self.close(p, p + 1, GROUP_END),
                    Some(b',') => return self.tok(p, p + 1, COMMA),
                    _ => {}
                }
                let e = self.word_end(p);
                if e > p && is_reserved(&self.b[p..e]) {
                    return self.close(p, e, UNEXPECTED_KEYWORD);
                }
                if let Some(e) = self.ident(p) {
                    return self.tok(p, e, PARAMETER);
                }
                if self.b[p.min(self.n)..].starts_with(b"...") {
                    return self.tok(p, p + 3, LANGUAGE);
                }
                None
            }
            ESCAPE_Z => self.else_pop(p),
            MODULE..MEMBER => {
                if self.b.get(p) == Some(&b'.') {
                    return self.then(p, p + 1, ACCESSOR, &[top - MODULE + MEMBER]);
                }
                self.else_pop(p)
            }
            MEMBER..STRING_SINGLE => {
                let e = self.word_end(p);
                if let Some(leaf) = member(usize::from(top - MEMBER), &self.b[p..e]) {
                    return self.close(p, e, leaf);
                }
                self.property(p)
            }
            STRING_SINGLE | STRING_DOUBLE => self.string(p, top),
            LONG_STRING => {
                if let Some(e) = long_close(self.b, p, self.level) {
                    return self.close(p, e, MULTILINE + END);
                }
                None
            }
            COMMENT_LINE => {
                if p == self.n {
                    return self.close(p, p + 1, LINE_COMMENT);
                }
                None
            }
            COMMENT_BLOCK => {
                if let Some(e) = long_close(self.b, p, self.level) {
                    let leaf = BLOCK_END + self.comment_wrap();
                    return self.close(p, e, leaf);
                }
                None
            }
            _ => None,
        }
    }

    /// The `prototype`: the comments.
    fn prototype(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if b.get(p) != Some(&b'-') || b.get(p + 1) != Some(&b'-') {
            return None;
        }
        if self.depth >= SYNTECT_DEPTH {
            return None;
        }
        if let Some((e, level)) = long_open(b, p + 2) {
            self.fill(p);
            self.push(COMMENT_BLOCK);
            self.level = level;
            let leaf = BLOCK_BEGIN + self.comment_wrap();
            self.put(p, e, leaf);
            return Some(e);
        }
        self.fill(p);
        self.push(COMMENT_LINE);
        let leaf = LINE_MARK + self.comment_wrap();
        self.put(p, p + 2, leaf);
        Some(p + 2)
    }

    /// `end`.
    fn end(&mut self, p: usize) -> Option<usize> {
        if self.word_is(p, b"end") {
            return self.close(p, p + 3, END_KEYWORD);
        }
        None
    }

    /// `statements`.
    fn statements(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let &c = b.get(p)?;
        if c == b';' {
            return self.tok(p, p + 1, TERMINATOR);
        }
        if c.is_ascii_lowercase() {
            let e = self.word_end(p);
            match &b[p..e] {
                b"function" => {
                    return self.tok_push(
                        p,
                        e,
                        FUNCTION_KEYWORD,
                        &[FUNCTION_META, BLOCK, PARAMS, FNAME],
                    );
                }
                b"do" => return self.tok_push(p, e, CONTROL, &[BLOCK]),
                b"if" => return self.tok_push(p, e, CONDITIONAL, &[IF_THEN, EXPRESSION]),
                b"while" => return self.tok_push(p, e, LOOP, &[DO, EXPRESSION]),
                b"repeat" => return self.tok_push(p, e, LOOP, &[UNTIL, REPEAT_BLOCK]),
                b"for" => return self.tok_push(p, e, LOOP, &[DO, FOR_IN, EXPRESSION_LIST]),
                b"goto" => return self.tok_push(p, e, GOTO_KEYWORD, &[GOTO]),
                b"break" => return self.tok(p, e, BREAK),
                b"return" => return self.tok_push(p, e, RETURN, &[EXPRESSION_LIST]),
                b"local" => return self.tok_push(p, e, LOCAL, &[EXPRESSION_LIST]),
                b"end" => return self.tok(p, e, UNEXPECTED_END),
                _ => {}
            }
        } else if c == b':' && b.get(p + 1) == Some(&b':') {
            return self.tok_push(p, p + 2, LABEL_BEGIN, &[LABEL]);
        }
        if self.nonblank(p) {
            return self.enter(p, &[EXPRESSION]);
        }
        None
    }

    /// `expression-end`.
    fn expression_end(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let &c = b.get(p)?;
        if c == b';' {
            return self.leave(p).or_else(|| self.expression_end_rest(p));
        }
        if args_begin(b, p) {
            return self
                .enter(p, &[ARGS_META, ARGS])
                .or_else(|| self.expression_end_rest(p));
        }
        self.expression_end_rest(p)
    }

    /// `expression-end` after its lookaheads: the infix operators, the accessors and
    /// `else-pop`.
    fn expression_end_rest(&mut self, p: usize) -> Option<usize> {
        if let Some((e, leaf)) = self.infix(p) {
            return self
                .tok_push(p, e, leaf, &[EXPR_BEGIN])
                .or_else(|| self.accessor_or_pop(p));
        }
        self.accessor_or_pop(p)
    }

    fn accessor_or_pop(&mut self, p: usize) -> Option<usize> {
        match self.b.get(p) {
            Some(b'.' | b':') => {
                if let Some(q) = self.tok_push(p, p + 1, ACCESSOR, &[PROPERTY]) {
                    return Some(q);
                }
            }
            Some(b'[') => {
                if let Some(q) = self.tok_push(p, p + 1, BRACKETS_BEGIN, &[BRACKETS]) {
                    return Some(q);
                }
            }
            _ => {}
        }
        self.else_pop(p)
    }

    /// `infix-operator` at `p`: the operator's end and leaf.
    fn infix(&self, p: usize) -> Option<(usize, u16)> {
        let b = self.b;
        let c = *b.get(p)?;
        let next = b.get(p + 1).copied();
        Some(match c {
            b'=' | b'<' | b'>' | b'~' if next == Some(b'=') => (p + 2, COMPARISON),
            b'/' if next == Some(b'/') => (p + 2, ARITHMETIC),
            b'-' | b'+' | b'*' | b'/' | b'%' | b'^' => (p + 1, ARITHMETIC),
            b'>' if next == Some(b'>') => (p + 2, BITWISE),
            b'<' if next == Some(b'<') => (p + 2, BITWISE),
            b'&' | b'|' | b'~' => (p + 1, BITWISE),
            b'<' | b'>' => (p + 1, COMPARISON),
            b'.' if next == Some(b'.') => (p + 2, CONCATENATION),
            b'=' => (p + 1, ASSIGNMENT),
            b'a' if self.word_is(p, b"and") => (p + 3, LOGICAL),
            b'o' if self.word_is(p, b"or") => (p + 2, LOGICAL),
            _ => return None,
        })
    }

    /// `expression-begin`.
    fn expression_begin(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let &c = b.get(p)?;
        match c {
            b';' => return self.leave(p).or_else(|| self.expression_begin_rest(p, c)),
            // `prefix-operator`
            b'#' => return self.tok(p, p + 1, LENGTH),
            b'-' => return self.tok(p, p + 1, ARITHMETIC),
            b'~' => return self.tok(p, p + 1, BITWISE),
            _ => {}
        }
        self.expression_begin_rest(p, c)
    }

    /// `expression-begin` after `(?=;)` and the prefix operators.
    fn expression_begin_rest(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if c.is_ascii_alphabetic() || c == b'_' {
            let e = self.word_end(p);
            let w = &b[p..e];
            match w {
                b"not" => return self.tok(p, e, LOGICAL),
                b"true" | b"false" => return self.close(p, e, BOOLEAN),
                b"nil" => return self.close(p, e, NULL),
                b"self" => return self.close(p, e, THIS),
                b"_G" | b"_VERSION" => return self.close(p, e, SUPPORT_CONSTANT),
                b"function" => {
                    return self.then(
                        p,
                        e,
                        FUNCTION_KEYWORD,
                        &[FUNCTION_META, BLOCK, PARAMS, FNAME],
                    );
                }
                _ => {}
            }
            if is_builtin_function(w) {
                return self.close(p, e, SUPPORT_FUNCTION);
            }
            if let Some(m) = MODULES.iter().position(|&m| m == w) {
                return self.then(p, e, SUPPORT_CONSTANT, &[MODULE + m as u8]);
            }
            if !is_reserved(w) {
                let leaf = if self.assignment_ahead(e) {
                    FUNCTION_NAME
                } else if self.call_ahead(e) {
                    CALL
                } else {
                    VARIABLE
                };
                return self.close(p, e, leaf);
            }
            return self.else_pop(p);
        }
        if c == b'.' && b.get(p + 1) == Some(&b'.') && b.get(p + 2) == Some(&b'.') {
            return self.close(p, p + 3, LANGUAGE);
        }
        if let Some((e, leaf, caps)) = number(b, p) {
            let mut q = p;
            for (s, ce, cl) in caps {
                if cl != PLAIN {
                    self.put(q, s, leaf);
                    self.put(s, ce, cl);
                    q = ce;
                }
            }
            return self.close(q, e, leaf);
        }
        if let Some(q) = self.string_open(p) {
            return Some(q);
        }
        match c {
            b'(' => self.then(p, p + 1, GROUP_BEGIN, &[PAREN_GROUP]),
            b'{' => self.then(p, p + 1, BRACES_BEGIN, &[MAPPING]),
            _ => self.else_pop(p),
        }
    }

    /// `string` at `p`: a string's opening, setting its context.
    fn string_open(&mut self, p: usize) -> Option<usize> {
        match self.b.get(p) {
            Some(b'\'') => self.then(p, p + 1, SINGLE + BEGIN, &[STRING_SINGLE]),
            Some(b'"') => self.then(p, p + 1, DOUBLE + BEGIN, &[STRING_DOUBLE]),
            Some(b'[') => {
                let (e, level) = long_open(self.b, p)?;
                self.put(p, e, MULTILINE + BEGIN);
                self.set(&[LONG_STRING]);
                self.level = level;
                Some(e)
            }
            _ => None,
        }
    }

    /// `function-arguments`.
    fn arguments(&mut self, p: usize) -> Option<usize> {
        if let Some(q) = self.string_open(p) {
            return Some(q);
        }
        match self.b.get(p) {
            Some(b'{') => self.then(p, p + 1, BRACES_BEGIN, &[MAPPING]),
            Some(b'(') => self.then(p, p + 1, GROUP_BEGIN, &[ARGS_GROUP]),
            _ => None,
        }
    }

    /// `property`.
    fn property(&mut self, p: usize) -> Option<usize> {
        let e = self.word_end(p);
        let w = &self.b[p..e];
        if let Some(e) = self.ident(p) {
            if self.assignment_ahead(e) {
                let leaf = if is_metamethod(w) {
                    PROPERTY_METAMETHOD
                } else {
                    PROPERTY_FUNCTION_NAME
                };
                return self.close(p, e, leaf);
            }
            if self.call_ahead(e) {
                return self.close(p, e, PROPERTY_CALL);
            }
            if is_metaproperty(w) {
                return self.close(p, e, PROPERTY_METAPROPERTY);
            }
            return self.close(p, e, PROPERTY_LEAF);
        }
        if e > p && is_reserved(w) {
            return self.close(p, e, UNEXPECTED_KEYWORD);
        }
        self.else_pop(p)
    }

    /// `table-constructor`'s `{ … }`.
    fn mapping(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let &c = b.get(p)?;
        match c {
            b'}' => return self.close(p, p + 1, BRACES_END),
            b',' | b';' => return self.tok(p, p + 1, FIELD_SEPARATOR),
            b'[' if long_open(b, p).is_none() => {
                if let Some(q) = self.tok_push(p, p + 1, BRACKETS_BEGIN, &[KEY_BRACKETS]) {
                    return Some(q);
                }
            }
            _ => {}
        }
        // `(?={{identifier}}\s*=(?!=))`
        if let Some(e) = self.ident(p) {
            let q = skip_spaces(b, e);
            if q < self.n
                && b[q] == b'='
                && b.get(q + 1) != Some(&b'=')
                && let Some(r) = self.enter(p, &[KEY])
            {
                return Some(r);
            }
        }
        if c == b'='
            && b.get(p + 1) != Some(&b'=')
            && let Some(q) = self.tok_push(p, p + 1, KEY_VALUE, &[VALUE_META, EXPRESSION])
        {
            return Some(q);
        }
        if self.nonblank(p) {
            return self.enter(p, &[EXPRESSION]);
        }
        None
    }

    /// A table's `name =` key: `(?=…)` made sure a name is at `p`.
    fn key(&mut self, p: usize) -> Option<usize> {
        let e = self.ident(p)?;
        let w = &self.b[p..e];
        // `(?:({{metamethod}})|{{identifier}})(?=\s*=\s*function\b)`
        let q = skip_spaces(self.b, e);
        if q < self.n && self.b[q] == b'=' {
            let r = skip_spaces(self.b, q + 1);
            if self.b[r.min(self.n)..].starts_with(b"function") && !is_word_char(self.b, r + 8) {
                let leaf = if is_metamethod(w) {
                    KEY_METAMETHOD
                } else {
                    FUNCTION_NAME
                };
                return self.close(p, e, leaf);
            }
        }
        if is_metamethod(w) {
            return self.close(p, e, KEY_METAMETHOD);
        }
        if is_metaproperty(w) {
            return self.close(p, e, KEY_METAPROPERTY);
        }
        self.close(p, e, KEY_LEAF)
    }

    /// A quoted string's patterns at `p`: `string-content`'s, then its closing quote.
    fn string(&mut self, p: usize, top: u8) -> Option<usize> {
        let b = self.b;
        let host = if top == STRING_SINGLE { SINGLE } else { DOUBLE };
        if p == self.n {
            // `\n`: the string is left unclosed
            return self.close(p, p + 1, host);
        }
        match b[p] {
            b'\\' => {
                let Some(&c) = b.get(p + 1) else {
                    // `\\\n`: an escape
                    return self.tok(p, p + 2, host + ESCAPE);
                };
                match c {
                    b'a' | b'b' | b'f' | b'n' | b'r' | b't' | b'v' | b'\\' | b'\'' | b'"'
                    | b'[' | b']' => return self.tok(p, p + 2, host + ESCAPE),
                    b'x' if b.get(p + 2).is_some_and(u8::is_ascii_hexdigit)
                        && b.get(p + 3).is_some_and(u8::is_ascii_hexdigit) =>
                    {
                        return self.tok(p, p + 4, host + ESCAPE_HEX);
                    }
                    b'u' if b.get(p + 2) == Some(&b'{') => {
                        let h = hex_digits(b, p + 3);
                        if h > p + 3 && b.get(h) == Some(&b'}') {
                            return self.tok(p, h + 1, host + ESCAPE_UNICODE);
                        }
                    }
                    b'z' if self.depth < SYNTECT_DEPTH => {
                        self.fill(p);
                        self.push(ESCAPE_Z);
                        self.put(p, p + 2, host + ESCAPE_SPACES);
                        return Some(p + 2);
                    }
                    _ => {}
                }
                // `\\\d{1,3}`
                if let Some(mut e) = digit(b, p + 1) {
                    for _ in 0..2 {
                        match digit(b, e) {
                            Some(f) => e = f,
                            None => break,
                        }
                    }
                    return self.tok(p, e, host + ESCAPE_DEC);
                }
                // `\\.`
                let e = p + 1 + char_len(c).min(self.n - p - 1);
                self.tok(p, e, host + INVALID_ESCAPE)
            }
            c if c == if top == STRING_SINGLE { b'\'' } else { b'"' } => {
                self.close(p, p + 1, host + END)
            }
            _ => None,
        }
    }
}

/// `\w` at `i` (Unicode's letters and decimal digits; marks and connector punctuation aren't
/// followed), for `\b`.
fn is_word_char(b: &[u8], i: usize) -> bool {
    match b.get(i) {
        Some(&c) if c < 0x80 => WORD[usize::from(c)],
        Some(_) => wide_char(b, i).is_some_and(|c| c.is_alphabetic() || is_nd(c)),
        None => false,
    }
}

/// `{{function_args_begin}}` at `p`: `\(|"|'|\[=*\[|\{`.
fn args_begin(b: &[u8], p: usize) -> bool {
    match b.get(p) {
        Some(b'(' | b'"' | b'\'' | b'{') => true,
        Some(b'[') => long_open(b, p).is_some(),
        _ => false,
    }
}

/// Whether no pattern of context `c` starts at a space: all but the ones that match nothing
/// (`''`) and the text.
const fn spaces_inert(c: u8) -> bool {
    !matches!(
        c,
        EXPRESSION | EXPRESSION_LIST | ARGS_META | VALUE_META | FUNCTION_META
    ) && c < STRING_SINGLE
}

#[cfg(test)]
mod tests {
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
}
