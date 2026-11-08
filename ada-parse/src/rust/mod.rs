//! Rust by a scanner of ada's own (ADR-40, speed first), on frames of ada's own: its code (the
//! statements, a path's segments, the types) on the C family's core ([`code`], X-056, X-063);
//! the headers (`impl`, `where`, `trait`, `enum`, `mod`, `struct`), a closure's parameters,
//! `macro_rules!`, attributes, strings and block comments beside them ([`items`], X-064). One
//! loop runs every frame ([`Scan::code_run`]); only a regex or SQL in a string has turns of its
//! own ([`Scan::run`]). bat's `Rust` grammar is the reference for scopes and the parity oracle:
//! the frames keep its output but where they fix it, by design (each named where it is). Frames
//! share one stack, their ids one space.
//!
//! What the 14 built-in themes tell apart is a token's own scopes and whether it is in an
//! attribute (`meta.annotation`), so a kind is a leaf (bat's scopes for the token) with or
//! without that one meta scope ([`scopes`]). The grammar's other meta scopes (a function's, a
//! block's, a group's) aren't kept; neither are its paths through some bad input, where a word
//! that starts with a digit would match from its middle. `examples/parity.rs` reads how close it
//! comes. Contexts that only differ in meta scopes are one here: a block, a function's body, an
//! impl's or a module's body are [`BLOCK`]. Names past ASCII are approximated: every byte past
//! ASCII is a letter, neither upper- nor lowercase.
//!
//! The stack between lines is the contexts open above the top level, up to 28 (the bytes of
//! [`State`]'s levels), and an open raw string's `#`s.
//!
//! A string that `Regex::new(`, `RegexBuilder::new(` or `regex!(` takes is colored as the
//! `regex` crate's regex (ADR-40, phase 4; [`crate::regex`]), as RustRover colors it: its text
//! is the regex's, but for the string's end and (in `"…"`) its escapes.
//!
//! SQL by call site and marker (ADR-40, phase 4; [`crate::sql`]), as RustRover has it: the SQL
//! argument of sqlx's, diesel's, rusqlite's and postgres' calls (`sqlx::query!(`,
//! `query_as!(T, `, `sql_query(`, `conn.execute(`…), and the string after `// language=SQL`.
//! Its text is SQL's, but for the string's end and (in `"…"`) its escapes.

mod code;
mod items;
pub(crate) mod own;

use crate::clike::{CLASS, HIGH, LOWER, SPACE, UPPER, WORD};
use crate::emit::{Rules, Run, Runs, Sink, Spans};
use crate::prim::{self, char_len};
use crate::regex;
use crate::sql;
use crate::{Out, State};

// The frames, by bat's names where one is one of its contexts. The code's and the type frames
// first ([`code`], X-056): ids up to [`LAST_FRAME`], so that telling one is a compare (X-063); then
// the headers', attributes', strings' and comments' ([`items`], X-064).
/// The top level (`statements`).
const MAIN: u8 = 0;
/// `{ … }`: a block, a function's, an impl's, a trait's, a module's or a macro's body.
const BLOCK: u8 = 1;
/// `( … )`.
const GROUP: u8 = 2;
/// `[ … ]`, where `;` separates.
const BRACKET: u8 = 3;
/// A macro body's `[ … ]`.
const MACRO_BRACKET: u8 = 4;
/// A closure's body without braces, to the end of the line or a closer.
const CLOSURE_BODY: u8 = 5;
/// After an operator: `<` starts generics, `|` a closure (a line's end may leave it).
const AFTER_OP: u8 = 6;
/// After `{` or `(`: `|` starts a closure.
const TRY_CLOSURE: u8 = 7;
/// After `name::` in an expression.
const NO_TYPE_NAMES: u8 = 8;
/// A SQL call's `( … )` (ADR-40, phase 4): [`GROUP`], plus where its SQL argument is (a
/// `PSQL_*` state added). Past that argument, or one that doesn't fit, it is [`GROUP`].
const PSQL: u8 = 9;
/// At the first argument: the SQL (`sql_query(`, `query!(`, `conn.execute(`…).
const PSQL_SQL: u8 = 0;
/// At the first argument: a type, then the SQL (`query_as!(`).
const PSQL_TYPE: u8 = 1;
/// In the type, waiting for its `,`.
const PSQL_SKIP: u8 = 2;
const PSQL_LAST: u8 = PSQL + PSQL_SKIP;
// the type frames
const RETURN_TYPE: u8 = 12;
const FN_GENERIC: u8 = 13;
const FN_PARAMS: u8 = 14;
const FN_RETURN: u8 = 15;
const STRUCT_FIELDS: u8 = 16;
const FIELD_TYPE: u8 = 17;
/// A tuple struct's fields, a type frame: its own `)` ends it, not the first at any depth as bat's
/// `with_prototype` has it (X-063).
const TUPLE_FIELDS: u8 = 18;
/// `generic-angles`.
const GENERIC: u8 = 19;
/// `generic-angles-contents`.
const GENERIC_ARGS: u8 = 20;
/// A `<` in them.
const GENERIC_NESTED: u8 = 21;
/// A `[` in them.
const GENERIC_ARRAY: u8 = 22;
/// `fn(` in a type.
const FN_TYPE: u8 = 23;
const FN_TYPE_RETURN: u8 = 24;
/// `(` in a type.
const TYPE_GROUP: u8 = 25;
/// The last frame's id.
const LAST_FRAME: u8 = TYPE_GROUP;
const _: () = assert!(PSQL_LAST < RETURN_TYPE);
/// A trait's or an enum's header: `statements-block`, and (X-064) its generics, supertraits and
/// `where` clause as a function's.
const HEAD: u8 = 26;
/// A module's header, which `;` can end.
const MOD_HEAD: u8 = 27;
const CLOSURE_PARAMS: u8 = 28;
/// `(a, b)` among parameters.
const PARAM_TUPLE: u8 = 29;
/// `: (A, B)` among parameters.
const PARAM_TUPLE_TYPE: u8 = 30;
/// A closure parameter's type.
const CLOSURE_PARAM_TYPE: u8 = 31;
const CLOSURE_RETURN: u8 = 32;
const FN_WHERE: u8 = 33;
/// An impl's, a trait's or an enum's `where` clause.
const IMPL_WHERE: u8 = 34;
/// A struct's `where` clause (X-064; bat's leaves it plain).
const STRUCT_WHERE: u8 = 35;
const IMPL_DEF: u8 = 36;
const IMPL_GENERIC: u8 = 37;
const IMPL_FOR: u8 = 38;
/// An impl's trait, before `for`.
const IMPL_TRAIT: u8 = 39;
const IMPL_IDENT: u8 = 40;
const STRUCT_ID: u8 = 41;
const STRUCT_GENERIC: u8 = 42;
const STRUCT_BODY: u8 = 43;
/// After its `;`, in a generic's `[ … ]`.
const GENERIC_LEN: u8 = 44;
/// After `extern` in a generic's arguments.
const GENERIC_EXTERN: u8 = 45;
/// `(` in a constant expression.
const CONST_GROUP: u8 = 46;
/// After `macro_rules! name`.
const MACRO_BLOCK: u8 = 47;
const MACRO_RULES: u8 = 48;
const MACRO_RULES_BRACKET: u8 = 49;
const MACRO_RULES_PAREN: u8 = 50;
const MATCH_OP: u8 = 51;
const MACRO_BODY: u8 = 52;
const MACRO_TERM: u8 = 53;
/// A formatting macro's first argument.
const FMT_FIRST: u8 = 54;
/// `write!`'s first argument, before the format string.
const WRITE_FIRST: u8 = 55;
/// A rule's matcher in `( … )`, `[ … ]` and `{ … }` (X-064: bat's, in `( … )` only).
const MATCHER: u8 = 56;
const MATCHER_SQUARE: u8 = 57;
const MATCHER_CURLY: u8 = 58;
/// `$(` in a matcher.
const MATCHER_GROUP: u8 = 59;
/// A plain group in a matcher, `( … )`, `[ … ]` or `{ … }`, so that its closer isn't the
/// matcher's (X-064).
const NEST_PAREN: u8 = 60;
const NEST_SQUARE: u8 = 61;
const NEST_CURLY: u8 = 62;
const ATTRIBUTE: u8 = 63;
const ATTR_PARAMS: u8 = 64;
const ATTR_CALL: u8 = 65;
// strings and comments last: no `with_prototype` reaches into them
const STRING: u8 = 70;
const _: () = assert!(ATTR_CALL < STRING);
const FMT_STRING: u8 = 71;
const BYTE_STRING: u8 = 72;
const RAW_STRING: u8 = 73;
const FMT_RAW: u8 = 74;
const RAW_BYTES: u8 = 75;
const BLOCK_COMMENT: u8 = 76;
const DOC_COMMENT: u8 = 77;
/// SQL in the string below (ADR-40, phase 4): this, plus SQL's context there
/// ([`crate::sql`]'s `MAIN` to `HASHES`).
const SQL_IN: u8 = 0x50;
const SQL_LAST: u8 = SQL_IN + sql::HASHES;
const _: () = assert!(DOC_COMMENT < SQL_IN);
/// A regex in the string below (ADR-40, phase 4): this bit, and the regex's state.
const REGEX: u8 = 0x80;
const _: () = assert!(SQL_LAST < REGEX);

/// The leaves: a token's scopes. A kind is a leaf, plus [`NLEAVES`] in an attribute.
static LEAVES: [&str; 112] = [
    "",
    "comment.line.double-slash.rust",
    "comment.line.double-slash.rust punctuation.definition.comment.rust",
    "comment.line.double-slash.rust variable.other.rust",
    "comment.line.documentation.rust",
    "comment.line.documentation.rust punctuation.definition.comment.rust",
    "comment.line.documentation.rust variable.other.rust",
    "comment.block.rust",
    "comment.block.rust punctuation.definition.comment.rust",
    "comment.block.rust variable.other.rust",
    "comment.block.documentation.rust",
    "comment.block.documentation.rust punctuation.definition.comment.rust",
    "comment.block.documentation.rust variable.other.rust",
    "string.quoted.double.rust",
    "string.quoted.double.rust punctuation.definition.string.begin.rust",
    "string.quoted.double.rust punctuation.definition.string.end.rust",
    "string.quoted.double.rust constant.character.escape.rust",
    "string.quoted.double.rust punctuation.separator.continuation.line.rust",
    "string.quoted.double.rust storage.type.string.rust",
    "string.quoted.double.rust constant.other.placeholder.rust",
    "string.quoted.double.raw.rust",
    "string.quoted.double.raw.rust punctuation.definition.string.begin.rust",
    "string.quoted.double.raw.rust punctuation.definition.string.end.rust",
    "string.quoted.double.raw.rust storage.type.string.rust",
    "string.quoted.double.raw.rust constant.character.escape.rust",
    "string.quoted.double.raw.rust constant.other.placeholder.rust",
    "string.quoted.single.rust",
    "string.quoted.single.rust punctuation.definition.string.begin.rust",
    "string.quoted.single.rust punctuation.definition.string.end.rust",
    "string.quoted.single.rust constant.character.escape.rust",
    "string.quoted.single.rust storage.type.string.rust",
    "punctuation.terminator.rust",
    "punctuation.separator.rust",
    "punctuation.separator.type.rust",
    "punctuation.separator.generic.rust",
    "punctuation.accessor.dot.rust",
    "meta.path.rust punctuation.accessor.rust",
    "meta.path.rust storage.type.rust punctuation.accessor.rust",
    "punctuation.accessor.double-colon.rust",
    "punctuation.section.group.begin.rust",
    "punctuation.section.group.end.rust",
    "meta.group.rust punctuation.section.group.begin.rust",
    "meta.group.rust punctuation.section.group.end.rust",
    "punctuation.section.block.begin.rust",
    "meta.block.rust punctuation.section.block.end.rust",
    "meta.block.rust punctuation.section.block.begin.rust",
    "punctuation.section.parameters.begin.rust",
    "punctuation.section.parameters.end.rust",
    "meta.function.parameters.rust punctuation.section.parameters.end.rust",
    "punctuation.definition.generic.begin.rust",
    "punctuation.definition.generic.end.rust",
    "punctuation.definition.annotation.rust",
    "meta.annotation.parameters.rust meta.group.rust punctuation.section.group.begin.rust",
    "meta.annotation.parameters.rust meta.group.rust punctuation.section.group.end.rust",
    "meta.function-call.rust meta.group.rust punctuation.section.group.begin.rust",
    "meta.function-call.rust meta.group.rust punctuation.section.group.end.rust",
    "meta.function-call.rust variable.function.rust",
    "punctuation.definition.modifier-scope.begin.rust",
    "punctuation.definition.modifier-scope.end.rust",
    "punctuation.terminator.macro-matcher.rust",
    "punctuation.section.group.begin.rust keyword.operator.rust",
    "keyword.other.rust",
    "keyword.control.rust",
    "keyword.operator.rust",
    "keyword.operator.assignment.rust",
    "keyword.operator.comparison.rust",
    "keyword.operator.logical.rust",
    "keyword.operator.arithmetic.rust",
    "keyword.operator.bitwise.rust",
    "keyword.operator.range.rust",
    "storage.type.rust",
    "storage.type.numeric.rust",
    "storage.type.function.rust",
    "storage.type.struct.rust",
    "storage.type.impl.rust",
    "storage.type.enum.rust",
    "storage.type.type.rust",
    "storage.type.trait.rust",
    "storage.type.module.rust",
    "storage.modifier.rust",
    "storage.modifier.lifetime.rust",
    "meta.path.rust storage.type.rust storage.type.rust",
    "entity.name.function.rust",
    "entity.name.struct.rust",
    "entity.name.impl.rust",
    "entity.name.enum.rust",
    "entity.name.trait.rust",
    "entity.name.type.rust",
    "entity.name.module.rust",
    "entity.name.macro.rust",
    "entity.name.label.rust",
    "variable.parameter.rust",
    "variable.parameter.macro.rust",
    "variable.other.member.rust",
    "variable.language.rust",
    "variable.other.rust",
    "variable.annotation.rust",
    "variable.other.constant.rust",
    "support.function.rust",
    "support.macro.rust",
    "support.type.rust",
    "constant.other.rust",
    "constant.language.rust",
    "constant.numeric.integer.decimal.rust",
    "constant.numeric.integer.hexadecimal.rust",
    "constant.numeric.integer.octal.rust",
    "constant.numeric.integer.binary.rust",
    "constant.numeric.float.rust",
    "invalid.illegal.rust",
    "meta.path.rust",
    "meta.path.rust variable.language.rust",
    "meta.path.rust keyword.other.rust",
];

// The leaves by name, in the order of `LEAVES`.
const PLAIN: u16 = 0;
const COMMENT: u16 = 1;
const DOC: u16 = 4;
const BLOCK_TEXT: u16 = 7;
const DOC_BLOCK: u16 = 10;
/// After one of the four comments' text: its mark, then a `$name` in it.
const MARK: u16 = 1;
const VAR: u16 = 2;
const STR: u16 = 13;
const STR_BEGIN: u16 = 14;
const STR_END: u16 = 15;
const STR_ESCAPE: u16 = 16;
const STR_CONTINUATION: u16 = 17;
const STR_PREFIX: u16 = 18;
const STR_PLACEHOLDER: u16 = 19;
const RAW: u16 = 20;
const RAW_BEGIN: u16 = 21;
const RAW_END: u16 = 22;
const RAW_PREFIX: u16 = 23;
const RAW_ESCAPE: u16 = 24;
const RAW_PLACEHOLDER: u16 = 25;
const CHAR: u16 = 26;
const CHAR_BEGIN: u16 = 27;
const CHAR_END: u16 = 28;
const CHAR_ESCAPE: u16 = 29;
const CHAR_PREFIX: u16 = 30;
const TERMINATOR: u16 = 31;
const SEPARATOR: u16 = 32;
const SEPARATOR_TYPE: u16 = 33;
const SEPARATOR_GENERIC: u16 = 34;
const DOT: u16 = 35;
const PATH_ACCESSOR: u16 = 36;
const TYPE_ACCESSOR: u16 = 37;
const DOUBLE_COLON: u16 = 38;
const GROUP_BEGIN: u16 = 39;
const GROUP_END: u16 = 40;
const META_GROUP_BEGIN: u16 = 41;
const META_GROUP_END: u16 = 42;
const BLOCK_BEGIN: u16 = 43;
const BLOCK_END: u16 = 44;
const META_BLOCK_BEGIN: u16 = 45;
const PARAMS_BEGIN: u16 = 46;
const PARAMS_END: u16 = 47;
const FN_PARAMS_END: u16 = 48;
const GENERIC_BEGIN: u16 = 49;
const GENERIC_END: u16 = 50;
const ANNOTATION_MARK: u16 = 51;
const ANNOTATION_BEGIN: u16 = 52;
const ANNOTATION_END: u16 = 53;
const CALL_BEGIN: u16 = 54;
const CALL_END: u16 = 55;
const CALL: u16 = 56;
const MODIFIER_BEGIN: u16 = 57;
const MODIFIER_END: u16 = 58;
const MACRO_TERMINATOR: u16 = 59;
const REPEAT_BEGIN: u16 = 60;
const KEYWORD: u16 = 61;
const CONTROL: u16 = 62;
const OPERATOR: u16 = 63;
const ASSIGNMENT: u16 = 64;
const COMPARISON: u16 = 65;
const LOGICAL: u16 = 66;
const ARITHMETIC: u16 = 67;
const BITWISE: u16 = 68;
const RANGE: u16 = 69;
const STORAGE: u16 = 70;
const NUMERIC: u16 = 71;
const FN: u16 = 72;
const STRUCT: u16 = 73;
const IMPL: u16 = 74;
const ENUM: u16 = 75;
const TYPE: u16 = 76;
const TRAIT: u16 = 77;
const MODULE: u16 = 78;
const MODIFIER: u16 = 79;
const LIFETIME: u16 = 80;
const PATH_TYPE: u16 = 81;
const FN_NAME: u16 = 82;
const STRUCT_NAME: u16 = 83;
const IMPL_NAME: u16 = 84;
const ENUM_NAME: u16 = 85;
const TRAIT_NAME: u16 = 86;
const TYPE_NAME: u16 = 87;
const MODULE_NAME: u16 = 88;
const MACRO_NAME: u16 = 89;
const LABEL: u16 = 90;
const PARAMETER: u16 = 91;
const MACRO_PARAMETER: u16 = 92;
const MEMBER: u16 = 93;
const SELF: u16 = 94;
const VARIABLE: u16 = 95;
const ANNOTATION: u16 = 96;
const CONSTANT_VAR: u16 = 97;
const SUPPORT_FN: u16 = 98;
const SUPPORT_MACRO: u16 = 99;
const SUPPORT_TYPE: u16 = 100;
const CONSTANT: u16 = 101;
const LANGUAGE: u16 = 102;
const DECIMAL: u16 = 103;
const HEX: u16 = 104;
const OCTAL: u16 = 105;
const BINARY: u16 = 106;
const FLOAT: u16 = 107;
const INVALID: u16 = 108;
const PATH: u16 = 109;
const PATH_SELF: u16 = 110;
const PATH_SUPER: u16 = 111;

const NLEAVES: u16 = LEAVES.len() as u16;

/// The kinds of a regex in a `"…"` string and in a raw one: the regex's, from each.
const RE_STRING: u16 = 256;
const RE_RAW: u16 = RE_STRING + regex::KINDS as u16;
const _: () = assert!(2 * NLEAVES <= RE_STRING);
/// SQL in a `"…"` string, then in a raw one: SQL's kinds from each.
const SQL_STRING: u16 = RE_RAW + regex::KINDS as u16;
const SQL_RAW: u16 = SQL_STRING + sql::KINDS as u16;

/// The scopes under `source.rust` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    if kind >= SQL_STRING {
        let (text, base) = if kind >= SQL_RAW {
            (RAW, SQL_RAW)
        } else {
            (STR, SQL_STRING)
        };
        if usize::from(kind - base) >= sql::KINDS {
            return Vec::new();
        }
        let mut out = scopes(text);
        out.extend(sql::scopes(kind - base));
        return out;
    }
    if kind >= RE_STRING {
        let (text, base) = if kind >= RE_RAW {
            (RAW, RE_RAW)
        } else {
            (STR, RE_STRING)
        };
        let mut out = scopes(text);
        out.extend(regex::scopes(kind - base));
        return out;
    }
    let mut out = Vec::new();
    if kind >= NLEAVES {
        out.push("meta.annotation.rust");
    }
    let leaf = LEAVES[usize::from(kind % NLEAVES)];
    out.extend(leaf.split(' ').filter(|s| !s.is_empty()));
    out
}

/// How many contexts a line can hold; one pushed past it drops the bottom one.
const MAX: usize = 128;

/// Lex `b`, a line without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    line_on(b, state, out, Spans::new());
}

/// [`line`]'s runs by theme rule pushed to `runs` (X-068): [`crate::lex_runs`] for Rust.
#[inline(never)]
pub fn line_runs(
    b: &[u8],
    state: &mut State,
    out: &mut Out,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
) {
    let mut k = line_on(b, state, out, Runs::new(rules, runs));
    k.end_line(b.len());
}

/// `b` lexed from `state` on a scan writing to `k`, its sink as the line leaves it.
#[inline(always)]
fn line_on<K: Sink>(b: &[u8], state: &mut State, out: &mut Out, k: K) -> K {
    let mut s = Scan::new(b, out, state.is_exact(), state.arg, k);
    s.load(state);
    s.open_spans(b.len() / 4 + 16);
    s.lex_line();
    s.close_spans();
    *state = s.state();
    s.k
}

/// A block of Rust on one scan opened from `state` ([`crate::Carry`]; X-066's scan, on the shared
/// loop since X-073): its stack, the bits a line hands the next and its spans' cursor stay where
/// they are, a line resets only what a line starts with, and a [`State`] is read once at the
/// start and made once at the end. Each line ends as [`line()`]'s does (past the levels a state
/// holds, the bottom ones dropped), so a block lexes as its lines do one at a time.
pub fn block(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    let mut out = Out::new(spans);
    let mut s = Scan::new(b"", &mut out, state.is_exact(), state.arg, Spans::new());
    s.open_lines(state);
    let rest = pass(&mut s);
    *state = s.state();
    rest
}

/// [`block`]'s runs by theme rule pushed to `runs` (X-068's sink, [`crate::CarryRuns`]).
pub fn block_runs(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    lines: &mut crate::RunLines,
) -> Option<usize> {
    let mut out = Out::new(spans);
    let k = Runs::new(rules, runs);
    let mut s = Scan::new(b"", &mut out, state.is_exact(), state.arg, k);
    s.open_lines(state);
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

impl<'b, 'c, K: Sink> Scan<'b, 'c, K> {
    /// The stack as `state` left it, on a scan [`Scan::new`] made from its bits: in place, as
    /// a scan returned by value is a `memcpy` call a line (X-056's trap, measured again at X-066).
    #[inline(always)]
    fn load(&mut self, state: &State) {
        self.depth = state.load(&mut self.stack);
        self.cur = self.stack[self.depth - 1];
        if state.arg & OPEN != 0 {
            self.count_open();
        }
    }

    /// The state the scan's last line left.
    #[inline(always)]
    fn state(&self) -> State {
        State::keep(&self.stack, self.depth, self.exact, self.arg())
    }

    /// The scan opened for a block from `state`, its spans' cursor at the vector's start.
    #[inline(always)]
    fn open_lines(&mut self, state: &State) {
        self.load(state);
        self.out.0.clear();
        self.open_spans(64);
    }

    /// `b`, the block's next line, lexed on this scan, its spans closed; the scan's line an
    /// empty one again after.
    #[inline(always)]
    fn carry(&mut self, b: &'b [u8]) {
        self.k.start_line();
        self.b = b;
        self.lex_next();
        self.close_spans();
        self.b = b"";
    }

    /// The next line, [`Scan::b`], lexed on this scan: what a line starts with reset, the rest
    /// kept from the line before.
    #[inline(always)]
    fn lex_next(&mut self) {
        self.last_opened = 0;
        self.at = 0;
        self.sql_paren = usize::MAX;
        self.sql_call = PSQL_SQL;
        self.sql = sql::Embedded {
            is_end: usize::MAX,
            at: 0,
        };
        self.sql_end = usize::MAX;
        self.sql_nl = false;
        self.lex_line();
    }

    /// A line's scan, its stack the base alone, `arg` the bits the line before left.
    #[inline(always)]
    fn new(b: &'b [u8], out: &'b mut Out<'c>, exact: bool, arg: u16, k: K) -> Self {
        Scan {
            b,
            stack: [MAIN; MAX],
            depth: 1,
            cur: MAIN,
            opened: arg & (OPENED_AFTER | OPENED_TRY),
            last_opened: 0,
            hashes: usize::from(arg & HASHES),
            exact,
            annot: 0,
            text: 0,
            at: 0,
            sql_paren: usize::MAX,
            sql_call: PSQL_SQL,
            sql: sql::Embedded {
                is_end: usize::MAX,
                at: 0,
            },
            sql_end: usize::MAX,
            sql_nl: false,
            mark: if arg & MARKED != 0 { 0 } else { usize::MAX },
            k,
            out,
        }
    }

    /// The open attributes, counted on the stack.
    #[inline(always)]
    fn count_open(&mut self) {
        for &l in &self.stack[1..self.depth] {
            self.annot += usize::from(l == ATTRIBUTE);
        }
    }

    /// The line lexed from the stack it starts with, ended as a state ends it (past the levels
    /// a state holds, the bottom ones dropped), a SQL marker that still holds kept at the next
    /// line's start.
    #[inline(always)]
    fn lex_line(&mut self) {
        let b = self.b;
        self.refresh();
        if self.top() >= SQL_IN {
            self.text = self.string_text();
        }
        self.run();
        self.fill(b.len());
        if self.mark != usize::MAX {
            self.mark = if sql::marker_holds(b, self.mark, b.len()) {
                0
            } else {
                usize::MAX
            };
        }
        let n = self.depth - 1;
        if n > crate::stack::HELD {
            let from = n - crate::stack::HELD;
            self.stack.copy_within(1 + from..self.depth, 1);
            self.depth -= from;
            self.exact = false;
            self.count();
        }
    }

    /// The [`State::arg`] the scan's last line leaves.
    #[inline(always)]
    fn arg(&self) -> u16 {
        let mut arg = if matches!(self.string_top(), RAW_STRING | FMT_RAW | RAW_BYTES) {
            self.hashes as u16 & HASHES
        } else {
            0
        };
        if self.mark != usize::MAX {
            arg |= MARKED;
        }
        if self.annot != 0 {
            arg |= OPEN;
        }
        arg | self.opened
    }
}

/// [`State::arg`]'s bit beside an open raw string's `#`s: a SQL marker (`// language=SQL`)
/// waits for its string.
const MARKED: u16 = 0x8000;
/// [`State::arg`]'s bit for an attribute open on the stack, so that a
/// line counts them only then.
const OPEN: u16 = 0x4000;
/// [`State::arg`]'s bits for the frames' [`Scan::opened`] at the line's end.
const OPENED_AFTER: u16 = 0x2000;
const OPENED_TRY: u16 = 0x1000;
/// [`State::arg`]'s bits for an open raw string's `#`s (Rust allows 255).
const HASHES: u16 = 0x0fff;

struct Scan<'b, 'c, K: Sink> {
    b: &'b [u8],
    stack: [u8; MAX],
    depth: usize,
    /// The top context, `stack[depth - 1]`, read without the stack.
    cur: u8,
    /// Whether the frames' next token may start a closure ([`OPENED_AFTER`] after an operator,
    /// where a generic may start too; [`OPENED_TRY`] after `(` or `{`), in place of bat's
    /// `after-operator` and `try-closure`; and the current token's, taken from it.
    opened: u16,
    last_opened: u16,
    /// The open raw string's `#`s.
    hashes: usize,
    exact: bool,
    /// How many attributes are open.
    annot: usize,
    /// The kind of text no pattern matches.
    text: u16,
    /// Where the spans so far end.
    at: usize,
    /// Where the `(` of a SQL call is ([`sql_call`]), and the state its arguments start in;
    /// `usize::MAX` for none on the line.
    sql_paren: usize,
    sql_call: u8,
    /// What SQL's scan keeps between the host's turns on a line.
    sql: sql::Embedded,
    /// Where the SQL string open on the line ends (its quote, or the line's end), past which no
    /// SQL token runs; `usize::MAX` till it is looked for.
    sql_end: usize,
    /// Whether SQL's scan took the line's end (`\s` matching its `\n`).
    sql_nl: bool,
    /// A SQL marker's: the next string that opens at or past it is SQL, unless a `;`, `,`, `)`,
    /// `{` or `}` came between ([`sql::marker_holds`], checked there and at the line's end);
    /// `usize::MAX` for none.
    mark: usize,
    /// Where the lexemes go, a line's spans or its runs by theme rule (X-068), on a write
    /// cursor in the scan's own fields (X-065).
    k: K,
    out: &'b mut Out<'c>,
}

/// A byte of a word: `\w`, every byte past ASCII among them (the core's classes, one load).
#[inline(always)]
fn is_word(c: u8) -> bool {
    CLASS[usize::from(c)] & (WORD | HIGH) != 0
}

#[inline(always)]
fn is_letter(c: u8) -> bool {
    CLASS[usize::from(c)] & (UPPER | LOWER | HIGH) != 0
}

/// `\s` as far as ASCII goes.
#[inline(always)]
fn is_space(c: u8) -> bool {
    CLASS[usize::from(c)] & SPACE != 0
}

#[inline(always)]
fn skip_spaces(b: &[u8], p: usize) -> usize {
    crate::clike::skip_blanks(b, p)
}

/// A line's indentation, four spaces a step, then the rest of its blanks a byte a step: it is
/// 14.5% of the corpus's bytes, 8 a run, nearly always spaces in fours.
#[inline(always)]
fn skip_indent(b: &[u8]) -> usize {
    let mut p = 0;
    while b.get(p..p + 4).is_some_and(|w| w == b"    ") {
        p += 4;
    }
    crate::clike::skip_blanks(b, p)
}

#[inline(always)]
fn word_end(b: &[u8], p: usize) -> usize {
    crate::clike::class_run(b, p, WORD | HIGH).end
}

/// Whether `\b` holds before `p`, at a word's start.
fn starts_word(b: &[u8], p: usize) -> bool {
    p == 0 || !is_word(b[p - 1])
}

/// Whether `\b` holds at `e`, after a word.
fn ends_word(b: &[u8], e: usize) -> bool {
    e >= b.len() || !is_word(b[e])
}

/// bat's identifier, `(?:[\p{L}][_\p{L}\p{N}]*|_[_\p{L}\p{N}]+)\b`, at `p`: where it ends.
fn ident_end(b: &[u8], p: usize) -> Option<usize> {
    let c = *b.get(p)?;
    let ok = is_letter(c) || (c == b'_' && b.get(p + 1).is_some_and(|&d| is_word(d)));
    ok.then(|| word_end(b, p))
}

/// Whether bat's identifier starts at `p`: [`ident_end`]'s test, without reading the word.
#[inline(always)]
fn ident_starts(b: &[u8], p: usize) -> bool {
    match b.get(p) {
        Some(&c) if is_letter(c) => true,
        Some(b'_') => b.get(p + 1).is_some_and(|&d| is_word(d)),
        _ => false,
    }
}

/// Whether a whole word is bat's identifier.
fn is_ident(w: &[u8]) -> bool {
    match w {
        [c, ..] if is_letter(*c) => true,
        [b'_', _, ..] => true,
        _ => false,
    }
}

/// `[\p{Lu}]|_*[\p{Lu}][\p{L}\p{N}_]*[\p{Ll}][\p{L}\p{N}_]*`, a whole word: a type's name
/// before `::`.
fn is_type_name(w: &[u8]) -> bool {
    let rest = &w[w.iter().take_while(|&&c| c == b'_').count()..];
    match rest.split_first() {
        Some((c, tail)) if c.is_ascii_uppercase() => {
            (w.len() == 1) || tail.iter().any(u8::is_ascii_lowercase)
        }
        _ => false,
    }
}

/// What a word is to bat's grammar, by [`lookup`]: a keyword (its leaf), one of the `support.type`
/// names, a primitive type (or `Self`), or another name.
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Name {
    Other,
    Keyword(u16),
    Support,
    Primitive,
}

/// The word `w` among bat's keywords and type names, dispatching on its first byte: the
/// copy [`code::WORDS`]'s table is tested against.
#[cfg(test)]
fn lookup(w: &[u8]) -> Name {
    use Name::{Keyword as K, Other, Primitive, Support};
    let Some(&c) = w.first() else {
        return Other;
    };
    match c {
        b'a' => match w {
            b"as" => K(OPERATOR),
            b"alignof" | b"abstract" => K(INVALID),
            _ => Other,
        },
        b'b' => match w {
            b"break" => K(CONTROL),
            b"box" => K(OPERATOR),
            b"become" => K(INVALID),
            b"bool" => Primitive,
            _ => Other,
        },
        b'c' => match w {
            b"const" => K(STORAGE),
            b"crate" => K(KEYWORD),
            b"continue" => K(CONTROL),
            b"char" => Primitive,
            _ => Other,
        },
        b'd' => match w {
            b"do" => K(INVALID),
            _ => Other,
        },
        b'e' => match w {
            b"else" => K(CONTROL),
            b"enum" => K(ENUM),
            b"extern" => K(KEYWORD),
            _ => Other,
        },
        b'f' => match w {
            b"fn" => K(FN),
            b"for" => K(CONTROL),
            b"false" => K(LANGUAGE),
            b"final" => K(INVALID),
            b"f32" | b"f64" => Primitive,
            _ => Other,
        },
        b'i' => match w {
            b"if" => K(CONTROL),
            b"impl" => K(IMPL),
            b"in" => K(OPERATOR),
            b"i8" | b"i16" | b"i32" | b"i64" | b"i128" | b"isize" => Primitive,
            _ => Other,
        },
        b'l' => match w {
            b"let" => K(STORAGE),
            b"loop" => K(CONTROL),
            _ => Other,
        },
        b'm' => match w {
            b"mod" => K(MODULE),
            b"mut" | b"move" => K(MODIFIER),
            b"match" => K(CONTROL),
            b"macro" => K(INVALID),
            _ => Other,
        },
        b'o' => match w {
            b"offsetof" | b"override" => K(INVALID),
            _ => Other,
        },
        b'p' => match w {
            b"pub" => K(MODIFIER),
            b"priv" | b"proc" | b"pure" => K(INVALID),
            _ => Other,
        },
        b'r' => match w {
            b"ref" => K(MODIFIER),
            b"return" => K(CONTROL),
            _ => Other,
        },
        b's' => match w {
            b"static" => K(STORAGE),
            b"struct" => K(STRUCT),
            b"sizeof" => K(INVALID),
            b"str" => Primitive,
            _ => Other,
        },
        b't' => match w {
            b"type" => K(TYPE),
            b"trait" => K(TRAIT),
            b"true" => K(LANGUAGE),
            b"typeof" => K(INVALID),
            _ => Other,
        },
        b'u' => match w {
            b"use" => K(KEYWORD),
            b"unsafe" => K(MODIFIER),
            b"unsized" => K(INVALID),
            b"u8" | b"u16" | b"u32" | b"u64" | b"u128" | b"usize" => Primitive,
            _ => Other,
        },
        b'v' => match w {
            b"virtual" => K(INVALID),
            _ => Other,
        },
        b'w' => match w {
            b"where" => K(KEYWORD),
            b"while" => K(CONTROL),
            _ => Other,
        },
        b'y' => match w {
            b"yield" => K(INVALID),
            _ => Other,
        },
        b'A' => match w {
            b"AsRef" | b"AsMut" => Support,
            _ => Other,
        },
        b'B' => match w {
            b"Box" => Support,
            _ => Other,
        },
        b'C' => match w {
            b"Copy" | b"Clone" => Support,
            _ => Other,
        },
        b'D' => match w {
            b"Drop" | b"Default" | b"DoubleEndedIterator" => Support,
            _ => Other,
        },
        b'E' => match w {
            b"Eq" | b"Extend" | b"ExactSizeIterator" | b"Err" => Support,
            _ => Other,
        },
        b'F' => match w {
            b"Fn" | b"FnMut" | b"FnOnce" | b"From" => Support,
            _ => Other,
        },
        b'I' => match w {
            b"Into" | b"Iterator" | b"IntoIterator" => Support,
            _ => Other,
        },
        b'N' => match w {
            b"None" => Support,
            _ => Other,
        },
        b'O' => match w {
            b"Ord" | b"Option" | b"Ok" => Support,
            _ => Other,
        },
        b'P' => match w {
            b"PartialEq" | b"PartialOrd" => Support,
            _ => Other,
        },
        b'R' => match w {
            b"Result" => Support,
            _ => Other,
        },
        b'S' => match w {
            b"Send" | b"Sized" | b"Sync" | b"Some" | b"SliceConcatExt" | b"String" => Support,
            b"Self" => Primitive,
            _ => Other,
        },
        b'T' => match w {
            b"ToOwned" | b"ToString" => Support,
            _ => Other,
        },
        b'V' => match w {
            b"Vec" => Support,
            _ => Other,
        },
        _ => Other,
    }
}

fn is_int_suffix(w: &[u8]) -> bool {
    matches!(
        w,
        b"i8"
            | b"i16"
            | b"i32"
            | b"i64"
            | b"i128"
            | b"isize"
            | b"u8"
            | b"u16"
            | b"u32"
            | b"u64"
            | b"u128"
            | b"usize"
    )
}

fn digits_end(b: &[u8], mut p: usize, digit: impl Fn(u8) -> bool) -> usize {
    while p < b.len() && (digit(b[p]) || b[p] == b'_') {
        p += 1;
    }
    p
}

/// A character escape at `p` (a `\`): `\\(x\h{2}|n|r|t|0|"|'|\\)`, and with `unicode`
/// `|u\{\h{1,6}\}`. Where it ends.
fn escape(b: &[u8], p: usize, unicode: bool) -> Option<usize> {
    match *b.get(p + 1)? {
        b'n' | b'r' | b't' | b'0' | b'"' | b'\'' | b'\\' => Some(p + 2),
        b'x' if b.get(p + 2..p + 4)?.iter().all(u8::is_ascii_hexdigit) => Some(p + 4),
        b'u' if unicode && b.get(p + 2) == Some(&b'{') => {
            let n = b[p + 3..]
                .iter()
                .take_while(|c| c.is_ascii_hexdigit())
                .count();
            ((1..=6).contains(&n) && b.get(p + 3 + n) == Some(&b'}')).then_some(p + 4 + n)
        }
        _ => None,
    }
}

/// The bytes a string's patterns start on, around a regex: `"…"`'s end and escapes, then a raw
/// string's end.
static RE_YIELDS: [[bool; 256]; 2] = {
    let mut t = [[false; 256]; 2];
    t[0][b'"' as usize] = true;
    t[0][b'\\' as usize] = true;
    t[1][b'"' as usize] = true;
    t
};

/// Whether the string whose prefix or quote is at `p` is a regex: the first argument of
/// `Regex::new(`, `RegexBuilder::new(` (`bytes::` ones too) or `regex!(`, looked back on its
/// line.
fn regex_string(b: &[u8], p: usize) -> bool {
    let mut q = p;
    while q > 0 && is_space(b[q - 1]) {
        q -= 1;
    }
    if q == 0 || b[q - 1] != b'(' {
        return false;
    }
    let mut w = q - 1;
    while w > 0 && is_space(b[w - 1]) {
        w -= 1;
    }
    // the name's last byte first: most calls end in neither
    let names: &[&[u8]] = match w.checked_sub(1).map(|i| b[i]) {
        Some(b'w') => &[b"Regex::new", b"RegexBuilder::new"],
        Some(b'!') => &[b"regex!"],
        _ => return false,
    };
    names
        .iter()
        .any(|name| b[..w].ends_with(name) && (w == name.len() || !is_word(b[w - name.len() - 1])))
}

/// The call `name` (a macro's with `mac`) whose arguments hold SQL, as RustRover reads them: the
/// [`PSQL`] state they start in. sqlx's `query`, `query_as`, `query_scalar` and their `_with`
/// forms, its macros (`query!`, `query_scalar!`, `query_unchecked!`…, and `query_as!` after a
/// type), diesel's `sql_query`, `sql` and `push_sql`, rusqlite's and postgres' `execute`,
/// `execute_batch`, `prepare`, `query_row`, `query`, `query_one`, `batch_execute`….
fn sql_call(name: &[u8], mac: bool) -> Option<u8> {
    if mac {
        return match name {
            b"query" | b"query_scalar" | b"query_unchecked" | b"query_scalar_unchecked" => {
                Some(PSQL_SQL)
            }
            b"query_as" | b"query_as_unchecked" => Some(PSQL_TYPE),
            _ => None,
        };
    }
    // the first byte first: most calls start with none of these
    let sql = match name.first()? {
        b'q' => matches!(
            name,
            b"query"
                | b"query_as"
                | b"query_scalar"
                | b"query_with"
                | b"query_as_with"
                | b"query_scalar_with"
                | b"query_row"
                | b"query_one"
                | b"query_opt"
                | b"query_raw"
                | b"query_typed"
        ),
        b's' => matches!(name, b"sql_query" | b"sql" | b"simple_query"),
        b'p' => matches!(
            name,
            b"push_sql" | b"prepare" | b"prepare_cached" | b"prepare_typed"
        ),
        b'e' => matches!(name, b"execute" | b"execute_batch"),
        b'b' => name == b"batch_execute",
        _ => false,
    };
    sql.then_some(PSQL_SQL)
}

/// Whether the `.` at `p` follows a receiver named as a prepared statement's (`stmt.execute(`,
/// `insert_statement.query(`, ASCII case ignored), whose arguments are the SQL's, not SQL.
fn stmt_receiver(b: &[u8], p: usize) -> bool {
    let mut s = p;
    while s > 0 && (b[s - 1].is_ascii_alphanumeric() || b[s - 1] == b'_') {
        s -= 1;
    }
    let r = b[s..p].to_ascii_lowercase();
    r.windows(4).any(|w| w == b"stmt") || r.windows(9).any(|w| w == b"statement")
}

/// A turbofish's `<…>` at `p`, its `<`, on the line: where the byte after its `>` is.
fn turbofish_end(b: &[u8], p: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, &c) in b[p..].iter().enumerate() {
        match c {
            b'<' => depth += 1,
            b'>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(p + i + 1);
                }
            }
            b'(' | b')' | b';' | b'{' | b'}' | b'"' => return None,
            _ => {}
        }
    }
    None
}

/// Where the string SQL is in ends on its line, from `p` inside it: its quote (a raw one's
/// with `hashes` `#`s after it), or the line's end.
fn string_end(b: &[u8], mut p: usize, hashes: Option<usize>) -> usize {
    while p < b.len() {
        match (b[p], hashes) {
            (b'"', None) => return p,
            (b'"', Some(h))
                if b.len() > p + h && b[p + 1..p + 1 + h].iter().all(|&c| c == b'#') =>
            {
                return p;
            }
            (b'\\', None) => p += 2,
            _ => p += 1,
        }
    }
    b.len()
}

/// A character literal's body at `p`, `[^'\\]` or an escape: where it ends, and whether it
/// is an escape.
fn char_body(b: &[u8], p: usize, unicode: bool) -> Option<(usize, bool)> {
    match *b.get(p)? {
        b'\\' => escape(b, p, unicode).map(|e| (e, true)),
        b'\'' => None,
        c => Some((p + char_len(c), false)),
    }
}

/// bat's format placeholder at `p`, a `{`: where it ends.
///
/// `\{ (\d+|name)? (: (.?[<>^])? [+-]? \#? 0? (\d\$?)? (\.(\d\$?|\*)?)? (\?|name)? )? \}`:
/// a width or a precision of one digit only.
fn placeholder(b: &[u8], p: usize) -> Option<usize> {
    let mut i = p + 1;
    if b.get(i).is_some_and(u8::is_ascii_digit) {
        i += b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
    } else if let Some(e) = ident_end(b, i) {
        i = e;
    }
    match b.get(i) {
        Some(b'}') => Some(i + 1),
        Some(b':') => spec(b, i + 1),
        _ => None,
    }
}

/// A placeholder's spec at `i`, after its `:`, and its `}`: where they end. Each optional part
/// is tried as the regex tries it, greedy first.
fn spec(b: &[u8], i: usize) -> Option<usize> {
    let at = |j: usize| b.get(j).copied();
    let align = |c: Option<u8>| matches!(c, Some(b'<' | b'>' | b'^'));
    let mut fills = [None; 3];
    if let Some(c) = at(i) {
        let n = char_len(c);
        if align(at(i + n)) {
            fills[0] = Some(i + n + 1);
        }
    }
    if align(at(i)) {
        fills[1] = Some(i + 1);
    }
    fills[2] = Some(i);
    for j in fills.into_iter().flatten() {
        let mut j = j;
        if matches!(at(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if at(j) == Some(b'#') {
            j += 1;
        }
        let zeros = [(at(j) == Some(b'0')).then_some(j + 1), Some(j)];
        for z in zeros.into_iter().flatten() {
            let digit = at(z).is_some_and(|c| c.is_ascii_digit());
            let widths = [
                (digit && at(z + 1) == Some(b'$')).then_some(z + 2),
                digit.then_some(z + 1),
                Some(z),
            ];
            for w in widths.into_iter().flatten() {
                let dot = at(w) == Some(b'.');
                let digit = dot && at(w + 1).is_some_and(|c| c.is_ascii_digit());
                let precisions = [
                    (digit && at(w + 2) == Some(b'$')).then_some(w + 3),
                    digit.then_some(w + 2),
                    (dot && at(w + 1) == Some(b'*')).then_some(w + 2),
                    dot.then_some(w + 1),
                    Some(w),
                ];
                for r in precisions.into_iter().flatten() {
                    let types = [
                        (at(r) == Some(b'?')).then_some(r + 1),
                        ident_end(b, r),
                        Some(r),
                    ];
                    for t in types.into_iter().flatten() {
                        if at(t) == Some(b'}') {
                            return Some(t + 1);
                        }
                    }
                }
            }
        }
    }
    None
}

/// `\s+for\s+` at `j`, the last `\s+` matching the line's `\n`.
fn for_at(b: &[u8], j: usize) -> bool {
    if !b.get(j).is_some_and(|&c| is_space(c)) {
        return false;
    }
    let k = skip_spaces(b, j);
    b[k..].starts_with(b"for") && b.get(k + 3).is_none_or(|&c| is_space(c))
}

/// impl-for's look-ahead at `p`: `(?=\s*(?:::|name|\$|<)+(<.*?>)?\s+for\s+)`, whether the impl
/// names a trait before `for`.
fn impl_for_ahead(b: &[u8], p: usize) -> bool {
    let mut i = skip_spaces(b, p);
    let (mut tokens, mut angle) = (0, None);
    loop {
        if b[i..].starts_with(b"::") {
            i += 2;
        } else if let Some(e) = ident_end(b, i) {
            i = e;
        } else if b.get(i) == Some(&b'$') {
            i += 1;
        } else if b.get(i) == Some(&b'<') {
            if tokens > 0 && angle.is_none() {
                angle = Some(i);
            }
            i += 1;
        } else {
            break;
        }
        tokens += 1;
    }
    if tokens == 0 {
        return false;
    }
    if for_at(b, i) {
        return true;
    }
    // a prefix of the tokens, then `<` to any `>` the rest follows
    angle.is_some_and(|a| (a + 1..b.len()).any(|j| b[j] == b'>' && for_at(b, j + 1)))
}

/// What text no pattern matches is in a context.
fn text_of(c: u8) -> u16 {
    match c {
        STRING | FMT_STRING | BYTE_STRING => STR,
        RAW_STRING | FMT_RAW | RAW_BYTES => RAW,
        BLOCK_COMMENT => BLOCK_TEXT,
        DOC_COMMENT => DOC_BLOCK,
        _ => PLAIN,
    }
}

/// `(:)\s*(\()` at `p`: where its `(` is.
fn colon_paren(b: &[u8], p: usize) -> Option<usize> {
    let s = skip_spaces(b, p + 1);
    (b[p] == b':' && b.get(s) == Some(&b'(')).then_some(s)
}

impl<K: Sink> Scan<'_, '_, K> {
    #[inline(always)]
    fn top(&self) -> u8 {
        self.cur
    }

    /// The top context replaced by `c`, in place (an embedded scan's state, a SQL call's
    /// argument), with no text filled and nothing counted.
    fn set_top(&mut self, c: u8) {
        let d = self.depth - 1;
        self.stack[d] = c;
        self.cur = c;
    }

    fn kind(&self, leaf: u16) -> u16 {
        if self.annot > 0 { NLEAVES + leaf } else { leaf }
    }

    /// Bytes `s..e` are `leaf`, after the text before them.
    fn put(&mut self, s: usize, e: usize, leaf: u16) {
        self.fill(s);
        let kind = self.kind(leaf);
        if kind != 0 {
            self.merge(s, e, kind);
        }
        self.at = e;
    }

    /// Bytes `s..e` are `kind`, on the sink's write cursor.
    #[inline(always)]
    fn merge(&mut self, s: usize, e: usize, kind: u16) {
        self.k.merge(&mut self.out, s, e, kind);
    }

    /// The sink opened at the spans' end, room made for `n` more.
    #[inline(always)]
    fn open_spans(&mut self, n: usize) {
        self.k.open(self.out, n);
    }

    /// The sink closed, for another scan to push spans.
    #[inline(always)]
    fn close_spans(&mut self) {
        self.k.close(self.out);
    }

    /// The text up to `p` is the context's.
    fn fill(&mut self, p: usize) {
        if p > self.at {
            if self.text != 0 {
                self.merge(self.at, p, self.text);
            }
            self.at = p;
        }
    }

    fn refresh(&mut self) {
        // a regex's text is set where one is pushed, and where a line starts in one
        self.text = self.kind(text_of(self.top()));
    }

    /// [`Scan::refresh`] in a regex or SQL: its string's text.
    #[cold]
    #[inline(never)]
    fn string_text(&self) -> u16 {
        self.kind(text_of(self.string_top()))
    }

    /// The top context, or under a regex or SQL the string it is in.
    fn string_top(&self) -> u8 {
        let top = self.top();
        if top >= SQL_IN && self.depth >= 2 {
            self.stack[self.depth - 2]
        } else {
            top
        }
    }

    /// Count the open attributes: only past the levels a state holds, or a stack full, so out
    /// of line (inlined at each push it took ada ~30 KB, X-064).
    #[cold]
    #[inline(never)]
    fn count(&mut self) {
        let open = &self.stack[..self.depth];
        self.annot = open.iter().filter(|&&c| c == ATTRIBUTE).count();
        self.refresh();
    }

    fn enter(&mut self, c: u8) {
        self.annot += usize::from(c == ATTRIBUTE);
    }

    fn leave(&mut self, c: u8) {
        self.annot -= usize::from(c == ATTRIBUTE);
    }

    fn push(&mut self, at: usize, c: u8) {
        self.fill(at);
        if self.depth == MAX {
            crate::stack::drop_bottom(&mut self.stack, &mut self.depth, &mut self.exact);
            self.count();
        }
        self.stack[self.depth] = c;
        self.depth += 1;
        self.cur = c;
        self.enter(c);
        self.refresh();
    }

    fn pop(&mut self, at: usize) {
        self.fill(at);
        if self.depth > 1 {
            self.depth -= 1;
            self.leave(self.cur);
            self.cur = self.stack[self.depth - 1];
        }
        self.refresh();
    }

    /// The line from the start: the frames' loop ([`Scan::code_run`]), and where a regex or SQL
    /// in a string is on top, its turns ([`Scan::embed_turn`]) till the string is again.
    fn run(&mut self) {
        // a line's indentation where a statement frame is on top, which would skip it in
        // [`Scan::code_run`] a byte a turn
        let mut p = if self.b.first() == Some(&b' ') && self.top() <= CLOSURE_BODY {
            skip_indent(self.b)
        } else {
            0
        };
        loop {
            p = self.code_run(p);
            if self.top() < SQL_IN {
                // the line's end
                return;
            }
            loop {
                let top = self.top();
                if top < SQL_IN {
                    break;
                }
                if p == self.b.len() {
                    if top < REGEX {
                        self.sql_eol(top);
                    }
                    return;
                }
                p = match self.embed_turn(p, top) {
                    Some(q) => q,
                    None => {
                        // a blank run, a word, else a byte
                        let b = self.b;
                        let c = b[p];
                        if is_space(c) {
                            skip_spaces(b, p)
                        } else if is_word(c) {
                            word_end(b, p)
                        } else {
                            p + 1
                        }
                    }
                };
            }
        }
    }

    /// A regex's or SQL's turn in a string at `p` (`top`, not the line's end): where the scan
    /// goes on, or `None` when no pattern matches.
    #[cold]
    fn embed_turn(&mut self, p: usize, top: u8) -> Option<usize> {
        let c = self.b[p];
        if top < REGEX {
            self.in_sql(p, c, top)
        } else {
            self.in_regex(p, c, top)
        }
    }

    /// A struct's name ended at `e`: its generic or its body next.
    fn struct_named(&mut self, e: usize) {
        let next = if self.b.get(e) == Some(&b'<') {
            STRUCT_GENERIC
        } else {
            STRUCT_BODY
        };
        self.set_top(next);
    }

    /// Whether `w` is the whole word at `p`: its first byte first, then the rest without a
    /// call to `memcmp`.
    #[inline(always)]
    fn is_word_at(&self, p: usize, w: &[u8]) -> bool {
        let b = self.b;
        b.get(p) == w.first()
            && b.len() >= p + w.len()
            && crate::same(&b[p..p + w.len()], w)
            && ends_word(b, p + w.len())
            && starts_word(b, p)
    }

    /// `n` bytes at `p` are `leaf`: where they end.
    fn token(&mut self, p: usize, n: usize, leaf: u16) -> usize {
        self.put(p, p + n, leaf);
        p + n
    }

    /// A closer at `p`, `leaf`, that ends the context.
    fn close(&mut self, p: usize, leaf: u16) -> usize {
        self.close_n(p, 1, leaf)
    }

    fn close_n(&mut self, p: usize, n: usize, leaf: u16) -> usize {
        self.put(p, p + n, leaf);
        self.pop(p + n);
        p + n
    }

    /// A `{` at `p` that opens a block in place of the context; `try_closure` after it, as
    /// `block` and `statements-block` have and a function's body hasn't.
    fn open_block(&mut self, p: usize, try_closure: bool) -> usize {
        self.set_top(BLOCK);
        self.tok_far(p, p + 1, BLOCK_BEGIN);
        if try_closure {
            self.push_ahead_code(p + 1, TRY_CLOSURE);
        }
        p + 1
    }

    /// `{{`, `}}` or a placeholder at `p`.
    fn format_escape(&mut self, p: usize, c: u8, escape: u16, place: u16) -> Option<usize> {
        if self.b.get(p + 1) == Some(&c) {
            return Some(self.token(p, 2, escape));
        }
        let e = (c == b'{').then(|| placeholder(self.b, p)).flatten()?;
        Some(self.token(p, e - p, place))
    }

    /// A comment at `p`, `comments`: where its first match ends.
    fn comment(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if b[p] != b'/' {
            return None;
        }
        match b.get(p + 1) {
            Some(b'*') => {
                if sql::marker_comment(b, p + 2) {
                    self.mark = b[p + 2..]
                        .windows(2)
                        .position(|w| w == b"*/")
                        .map_or(b.len(), |i| p + 4 + i);
                }
                Some(self.open_comment(p))
            }
            Some(b'/') => {
                let doc = matches!(b.get(p + 2), Some(b'!' | b'/'));
                if !doc && sql::marker_comment(b, p + 2) {
                    self.mark = b.len();
                }
                let (text, n) = if doc { (DOC, 3) } else { (COMMENT, 2) };
                self.put(p, p + n, text + MARK);
                let mut from = p + n;
                let mut at = from;
                loop {
                    let d = prim::find_any(b, at, *b"$");
                    if d == b.len() {
                        break;
                    }
                    if let Some(e) = ident_end(b, d + 1) {
                        self.put(from, d, text);
                        self.put(d, e, text + VAR);
                        from = e;
                        at = e;
                    } else {
                        at = d + 1;
                    }
                }
                self.put(from, b.len(), text);
                Some(b.len())
            }
            _ => None,
        }
    }

    /// `/*` at `p`: a block comment, a documentation one for `/**` or `/*!` and a character
    /// that isn't `*` or `/` (bat's `/\*[!\*][^\*/]`, its mark taking that character).
    fn open_comment(&mut self, p: usize) -> usize {
        let b = self.b;
        let doc = matches!(b.get(p + 2), Some(b'!' | b'*'))
            && b.get(p + 3).is_none_or(|&d| d != b'*' && d != b'/');
        if doc {
            let e = b.get(p + 3).map_or(b.len(), |&d| p + 3 + char_len(d));
            self.push(p, DOC_COMMENT);
            self.token(p, e - p, DOC_BLOCK + MARK)
        } else {
            self.push(p, BLOCK_COMMENT);
            self.token(p, 2, BLOCK_TEXT + MARK)
        }
    }

    fn in_comment(&mut self, top: u8, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        let text = if top == DOC_COMMENT {
            DOC_BLOCK
        } else {
            BLOCK_TEXT
        };
        if c == b'*' && b.get(p + 1) == Some(&b'/') {
            return Some(self.close_n(p, 2, text + MARK));
        }
        // `^\s*(\*)(?!/)`, a documentation comment's line's leading `*`
        if top == DOC_COMMENT && p == 0 {
            let q = skip_spaces(b, 0);
            if b.get(q) == Some(&b'*') && b.get(q + 1) != Some(&b'/') {
                return Some(self.token(q, 1, text + MARK));
            }
        }
        (c == b'/' && b.get(p + 1) == Some(&b'*')).then(|| self.open_comment(p))
    }

    /// An operator that `=` can follow, making it an assignment.
    fn op_eq(&mut self, p: usize, leaf: u16) -> usize {
        if self.b.get(p + 1) == Some(&b'=') {
            self.tok(p, p + 2, ASSIGNMENT)
        } else {
            self.tok(p, p + 1, leaf)
        }
    }

    /// `\*const\b` or `\*mut\b` at `p`.
    fn star_keyword(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let (n, leaf) = if b[p + 1..].starts_with(b"const") {
            (6, STORAGE)
        } else if b[p + 1..].starts_with(b"mut") {
            (4, MODIFIER)
        } else {
            return None;
        };
        ends_word(b, p + n).then(|| self.tok(p, p + n, leaf))
    }

    /// `(#!?)(\[)` at `p`: an attribute.
    fn attribute(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let m = if b.get(p + 1) == Some(&b'!') {
            p + 2
        } else {
            p + 1
        };
        if b.get(m) != Some(&b'[') {
            return None;
        }
        self.push(p, ATTRIBUTE);
        self.put(p, m, ANNOTATION_MARK);
        Some(self.token(m, 1, GROUP_BEGIN))
    }

    /// A `'` at `p`: a label, a lifetime, a character, or an operator.
    fn quote(&mut self, p: usize) -> usize {
        let b = self.b;
        if let Some(e) = ident_end(b, p + 1) {
            let s = skip_spaces(b, e);
            if b.get(s) == Some(&b':') {
                self.tok(p, e, LABEL);
                return self.tok(s, s + 1, SEPARATOR);
            }
            if b.get(e) != Some(&b'\'') {
                return self.tok(p, e, LIFETIME);
            }
        }
        if let Some(e) = self.character(p, p, true) {
            return e;
        }
        self.tok(p, p + 1, OPERATOR)
    }

    /// A character literal whose `'` is at `q`, its prefix from `p`: where it ends.
    fn character(&mut self, p: usize, q: usize, unicode: bool) -> Option<usize> {
        let (e, esc) = char_body(self.b, q + 1, unicode)?;
        if self.b.get(e) != Some(&b'\'') {
            return None;
        }
        self.put(p, q, CHAR_PREFIX);
        self.put(q, q + 1, CHAR_BEGIN);
        self.put(q + 1, e, if esc { CHAR_ESCAPE } else { CHAR });
        Some(self.token(e, 1, CHAR_END))
    }

    /// `'name(?!')\b` at `p`.
    fn lifetime(&mut self, p: usize) -> Option<usize> {
        let e = ident_end(self.b, p + 1)?;
        (self.b.get(e) != Some(&b'\'')).then(|| self.token(p, e - p, LIFETIME))
    }

    /// A string at `p`, `strings`: `"`, `b"`, `br#"`, `r#"`; with `chars`, `b'` too. A regex in
    /// it where a call makes it one.
    fn string(&mut self, p: usize, chars: bool) -> Option<usize> {
        let e = self.string_open(p, chars)?;
        if b"\"br".contains(&self.b[p]) {
            self.regex_in(p, e);
        }
        Some(e)
    }

    /// A string opened at `p`, its text from `e`: SQL in it after a marker, else a regex in it
    /// where a call makes it one.
    // out of line, so that the callers' hot paths stay inlined
    #[inline(never)]
    fn regex_in(&mut self, p: usize, e: usize) {
        if self.mark <= p && self.marked(p) {
            if matches!(self.top(), STRING | RAW_STRING) {
                self.sql_in(e);
            }
        } else if regex_string(self.b, p) {
            self.push(e, REGEX);
            self.text = self.string_text();
        }
    }

    /// A name `b[p..e]` before a call's `(` (after `!` for a macro, `::<…>` for a turbofish):
    /// that `(` opens a SQL call's arguments if the name is one of [`sql_call`]'s.
    #[inline(never)]
    fn sql_name(&mut self, p: usize, e: usize) {
        let b = self.b;
        let mac = b.get(e) == Some(&b'!');
        let Some(st) = sql_call(&b[p..e], mac) else {
            return;
        };
        if p > 0 && b[p - 1] == b'.' && stmt_receiver(b, p - 1) {
            return;
        }
        let q = if mac {
            skip_spaces(b, e + 1)
        } else if b[e..].starts_with(b"::<") {
            match turbofish_end(b, e + 2) {
                Some(q) => q,
                None => return,
            }
        } else {
            e
        };
        if b.get(q) == Some(&b'(') {
            self.sql_paren = q;
            self.sql_call = st;
        }
    }

    /// A string at `p`, `"…"` or a raw one, whose text is SQL; `None` if none opens there.
    fn sql_string(&mut self, p: usize) -> Option<usize> {
        let e = self.string_open(p, false)?;
        if matches!(self.top(), STRING | RAW_STRING) {
            self.sql_in(e);
        }
        Some(e)
    }

    /// SQL in the string just opened, its text from `e`. It takes a marker's turn.
    fn sql_in(&mut self, e: usize) {
        self.mark = usize::MAX;
        self.sql_end = usize::MAX;
        self.sql.is_end = usize::MAX;
        self.push(e, SQL_IN + sql::MAIN);
        self.text = self.string_text();
    }

    /// Whether the marker before `p` still holds there ([`sql::marker_holds`]); it's dropped
    /// if not.
    #[cold]
    fn marked(&mut self, p: usize) -> bool {
        let holds = sql::marker_holds(self.b, self.mark, p);
        if !holds {
            self.mark = usize::MAX;
        }
        holds
    }

    /// In SQL in a string, at `p`: the string's end and (in `"…"`) its escapes, else SQL's
    /// scanner, embedded, to the next place they can start ([`sql::embedded`]).
    #[cold]
    #[inline(never)]
    fn in_sql(&mut self, p: usize, c: u8, top: u8) -> Option<usize> {
        let b = self.b;
        if self.string_top() == RAW_STRING {
            let h = self.hashes;
            if c == b'"' && b.len() > p + h && b[p + 1..p + 1 + h].iter().all(|&c| c == b'#') {
                self.pop(p);
                return Some(self.close_n(p, 1 + h, RAW_END));
            }
        } else {
            match c {
                b'"' => {
                    self.pop(p);
                    return Some(self.close(p, STR_END));
                }
                b'\\' => {
                    if let Some(e) = escape(b, p, true) {
                        return Some(self.token(p, e - p, STR_ESCAPE));
                    }
                    if p + 1 == b.len() {
                        return Some(self.token(p, 1, STR_CONTINUATION));
                    }
                }
                _ => {}
            }
        }
        self.sql_scan(p, top).map(|(q, _)| q)
    }

    /// SQL's scan from `p` in the string below `top`: where it stopped, its context there set
    /// on the stack. No SQL token runs past the string's end.
    fn sql_scan(&mut self, p: usize, top: u8) -> Option<(usize, u8)> {
        let raw = self.string_top() == RAW_STRING;
        let host = sql::Embed {
            yields: &RE_YIELDS[usize::from(raw)],
            base: if raw { SQL_RAW } else { SQL_STRING },
            text: self.text,
        };
        let b = self.b;
        if self.sql_end == usize::MAX {
            self.sql_end = string_end(b, p, raw.then_some(self.hashes));
        }
        let end = self.sql_end;
        self.fill(p);
        self.sql.at = self.at;
        self.close_spans();
        let r = sql::embedded(&b[..end], p, top - SQL_IN, &host, &mut self.sql, self.out);
        self.k.reopen(self.out, 16);
        self.at = self.sql.at;
        let (mut q, ctx) = r?;
        if q > end {
            self.sql_nl |= end == b.len();
            q = end;
        }
        self.set_top(SQL_IN + ctx);
        Some((q, ctx))
    }

    /// SQL at its string's line end, which SQL's patterns may take as `\s`; its line comment
    /// ends there.
    #[cold]
    fn sql_eol(&mut self, top: u8) {
        if !self.sql_nl {
            self.sql_scan(self.b.len(), top);
        }
        if matches!(self.top().wrapping_sub(SQL_IN), sql::DASHES | sql::HASHES) {
            self.set_top(SQL_IN + sql::MAIN);
        }
    }

    /// In a regex in a string, at `p`: the string's end and (in `"…"`) its escapes but `\\`,
    /// which is the regex's, else the regex to the next place they can start.
    #[cold]
    #[inline(never)]
    fn in_regex(&mut self, p: usize, c: u8, top: u8) -> Option<usize> {
        if top < REGEX {
            return None;
        }
        let b = self.b;
        let string = self.string_top();
        let raw = matches!(string, RAW_STRING | RAW_BYTES);
        let h = self.hashes;
        if raw {
            if c == b'"' && b.len() > p + h && b[p + 1..p + 1 + h].iter().all(|&c| c == b'#') {
                self.pop(p);
                return Some(self.close_n(p, 1 + h, RAW_END));
            }
        } else {
            match c {
                b'"' => {
                    self.pop(p);
                    return Some(self.close(p, STR_END));
                }
                b'\\' if b.get(p + 1) != Some(&b'\\') => {
                    if let Some(e) = escape(b, p, string != BYTE_STRING) {
                        return Some(self.token(p, e - p, STR_ESCAPE));
                    }
                    if string != BYTE_STRING && p + 1 == b.len() {
                        return Some(self.token(p, 1, STR_CONTINUATION));
                    }
                }
                _ => {}
            }
        }
        let host = regex::Embed {
            dialect: regex::Dialect::Rust,
            cooked: !raw,
            yields: &RE_YIELDS[usize::from(raw)],
            stop: |b, p, h| {
                b[p] == b'"'
                    && b.len() >= p + 1 + h
                    && b[p + 1..p + 1 + h].iter().all(|&c| c == b'#')
            },
            arg: if raw { h } else { 0 },
            escape: |b, p| escape(b, p, true).unwrap_or(p + 1),
            top_only: false,
            continues: false,
            base: if raw { RE_RAW } else { RE_STRING },
            text: self.text,
        };
        self.close_spans();
        let r = regex::embedded(b, p, top & regex::STATE, &host, &mut self.at, self.out);
        self.k.reopen(self.out, 16);
        let (q, st) = r?;
        self.set_top(REGEX | st);
        Some(q)
    }

    /// [`Scan::string`]'s string, opened.
    fn string_open(&mut self, p: usize, chars: bool) -> Option<usize> {
        let b = self.b;
        if b[p] == b'"' {
            self.push(p, STRING);
            return Some(self.token(p, 1, STR_BEGIN));
        }
        let e = word_end(b, p);
        let next = b.get(e).copied();
        match (&b[p..e], next) {
            (b"b", Some(b'"')) => {
                self.push(p, BYTE_STRING);
                self.put(p, e, STR_PREFIX);
                Some(self.token(e, 1, STR_BEGIN))
            }
            (b"br" | b"r", Some(b'#' | b'"')) => {
                let h = b[e..].iter().take_while(|&&c| c == b'#').count();
                if b.get(e + h) != Some(&b'"') {
                    return None;
                }
                let raw = e - p == 1;
                self.push(p, if raw { RAW_STRING } else { RAW_BYTES });
                self.hashes = h;
                self.put(p, e, RAW_PREFIX);
                if raw {
                    // `(r)((#*)")`: the `"` is the mark's
                    Some(self.token(e, h + 1, RAW_BEGIN))
                } else {
                    self.put(e, e + h, RAW_BEGIN);
                    Some(self.token(e + h, 1, RAW))
                }
            }
            (b"b", Some(b'\'')) if chars => self.character(p, e, false),
            _ => None,
        }
    }

    /// `numbers` at `p`, a digit; floats only with `floats`.
    fn number(&mut self, p: usize, floats: bool) -> Option<usize> {
        let b = self.b;
        let digit = |c: u8| c.is_ascii_digit();
        let r = digits_end(b, p, digit);
        let suffix = |at: usize, s: &[u8]| b[at..].starts_with(s) && ends_word(b, at + s.len());
        if floats {
            if b.get(r) == Some(&b'.') && b[r - 1].is_ascii_digit() {
                if b.get(r + 1).is_some_and(u8::is_ascii_digit) {
                    // `\d.\d`, an exponent, `f32` or `f64`
                    let mut end = digits_end(b, r + 1, digit);
                    if matches!(b.get(end), Some(b'e' | b'E')) {
                        let mut s = end + 1;
                        if matches!(b.get(s), Some(b'+' | b'-')) {
                            s += 1;
                        }
                        let x = digits_end(b, s, digit);
                        if b[s..x].iter().any(u8::is_ascii_digit) {
                            end = x;
                        }
                    }
                    self.put(p, end, FLOAT);
                    if b[end..].starts_with(b"f32") || b[end..].starts_with(b"f64") {
                        return Some(self.token(end, 3, NUMERIC));
                    }
                    return Some(end);
                }
                if b.get(r + 1) != Some(&b'.') {
                    return Some(self.token(p, r + 1 - p, FLOAT));
                }
            }
            if suffix(r, b"f32") || suffix(r, b"f64") {
                self.put(p, r, FLOAT);
                return Some(self.token(r, 3, NUMERIC));
            }
            // `\d[eE][+-]?[\d_]*\d`, with `f32`, `f64` or nothing to the word's end: a float
            // without a dot, which bat's grammar leaves plain (X-008)
            if matches!(b.get(r), Some(b'e' | b'E')) {
                let mut s = r + 1;
                if matches!(b.get(s), Some(b'+' | b'-')) {
                    s += 1;
                }
                let x = digits_end(b, s, digit);
                if b[s..x].iter().any(u8::is_ascii_digit) {
                    if suffix(x, b"f32") || suffix(x, b"f64") {
                        self.put(p, x, FLOAT);
                        return Some(self.token(x, 3, NUMERIC));
                    }
                    if ends_word(b, x) {
                        return Some(self.token(p, x - p, FLOAT));
                    }
                }
            }
        }
        let e = word_end(b, p);
        // the integer's digits, then a suffix or nothing to the word's end
        let int = |s: &mut Self, from: usize, leaf: u16| -> Option<usize> {
            let rest = &b[from..e];
            if !rest.is_empty() && !is_int_suffix(rest) {
                return None;
            }
            s.put(p, from, leaf);
            Some(s.token(from, e - from, NUMERIC))
        };
        if let Some(q) = int(self, r, DECIMAL) {
            return Some(q);
        }
        if b[p] == b'0' {
            let based = |test: fn(u8) -> bool| {
                let d = digits_end(b, p + 2, test);
                b[p + 2..d].iter().any(|&c| test(c)).then_some(d)
            };
            let (d, leaf) = match b.get(p + 1) {
                Some(b'x') => (based(|c| c.is_ascii_hexdigit()), HEX),
                Some(b'o') => (based(|c| matches!(c, b'0'..=b'7')), OCTAL),
                Some(b'b') => (based(|c| matches!(c, b'0' | b'1')), BINARY),
                _ => (None, PLAIN),
            };
            if let Some(d) = d {
                return int(self, d, leaf);
            }
        }
        None
    }

    /// `constant-integer-expression` at `p`.
    fn const_expr(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        match c {
            b'0'..=b'9' => starts_word(b, p).then(|| self.number(p, false)).flatten(),
            b'(' => {
                self.push_code(CONST_GROUP);
                Some(self.tok_far(p, p + 1, GROUP_BEGIN))
            }
            b':' if b.get(p + 1) == Some(&b':') => Some(self.tok_far(p, p + 2, DOUBLE_COLON)),
            b'-' | b'+' | b'%' | b'/' | b'*' => Some(self.tok_far(p, p + 1, ARITHMETIC)),
            _ => {
                let e = ident_end(b, p)?;
                Some(self.tok_far(p, e, CONSTANT_VAR))
            }
        }
    }

    /// `(\$name)(:)(ident|path|expr|ty|pat|stmt|block|item|meta|tt)` at `p`.
    fn metavariable(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let e = ident_end(b, p + 1)?;
        if b.get(e) != Some(&b':') {
            return None;
        }
        let frag = [
            &b"ident"[..],
            b"path",
            b"expr",
            b"ty",
            b"pat",
            b"stmt",
            b"block",
            b"item",
            b"meta",
            b"tt",
        ]
        .into_iter()
        .find(|f| b[e + 1..].starts_with(f))?;
        self.put(p, e, MACRO_PARAMETER);
        self.put(e, e + 1, SEPARATOR);
        Some(self.token(e + 1, frag.len(), STORAGE))
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::marked;
    use crate::{Lang, State};

    fn of(lines: &[&str]) -> Vec<(String, String)> {
        marked(Lang::Rust, lines)
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
    fn regexes_in_strings() {
        const RE: &str = "string.quoted.double.rust";
        const RAW: &str = "string.quoted.double.raw.rust";
        let l = [
            r##"let a = Regex::new("^(?<y>\\d+)\n[\\pL--\\p{Greek}&&[^x]]\"$x\"##,
            r#"  (x|y)\\b{start}x$").unwrap();"#,
            r##"let b = regex!(r#"["a-z]+"(?x)\x{41}\u{1F600}\<"#);"##,
            r#"let c = RegexBuilder::new(r"(?P<a.b>\p{sc!=Greek})[a-g~~b-h]");"#,
            r#"println!("^\\d+ {x}"); MyRegex::new("a+")"#,
        ];
        has(&l, "^", &format!("{RE} keyword.control.anchor.regexp"));
        has(&l, "y", &format!("{RE} entity.name.other.group.regexp"));
        has(
            &l,
            r"\\d",
            &format!("{RE} constant.other.character-class.escape.backslash.regexp"),
        );
        has(&l, r"\n", &format!("{RE} constant.character.escape.rust"));
        has(
            &l,
            "--",
            &format!(
                "{RE} constant.other.character-class.set.regexp keyword.operator.difference.regexp"
            ),
        );
        has(
            &l,
            "&&",
            &format!(
                "{RE} constant.other.character-class.set.regexp keyword.operator.intersection.regexp"
            ),
        );
        // the prototype's `$x` doesn't reach a regex; a line continued
        has(&l, "$", &format!("{RE} keyword.control.anchor.regexp"));
        has(
            &l,
            "\\",
            &format!("{RE} punctuation.separator.continuation.line.rust"),
        );
        has(&l, "|", &format!("{RE} keyword.operator.or.regexp"));
        has(
            &l,
            r"\\b{start}",
            &format!("{RE} keyword.control.anchor.regexp"),
        );
        // a raw string: its `"` aren't its end but before its `#`s
        has(
            &l,
            "\"",
            &format!("{RAW} constant.other.character-class.set.regexp"),
        );
        has(
            &l,
            "?x",
            &format!("{RAW} keyword.other.option-toggle.regexp"),
        );
        has(
            &l,
            r"\u{1F600}",
            &format!("{RAW} constant.character.escape.backslash.regexp"),
        );
        has(&l, r"\<", &format!("{RAW} keyword.control.anchor.regexp"));
        has(
            &l,
            "\"#",
            &format!("{RAW} punctuation.definition.string.end.rust"),
        );
        has(&l, "a.b", &format!("{RAW} entity.name.other.group.regexp"));
        has(
            &l,
            "!=",
            &format!("{RAW} punctuation.separator.key-value.unicode-property.regexp"),
        );
        has(
            &l,
            "~~",
            &format!(
                "{RAW} constant.other.character-class.set.regexp keyword.operator.symmetric-difference.regexp"
            ),
        );
        // not a regex: any other call
        let got = of(&l[4..]);
        assert!(!got.iter().any(|(_, s)| s.contains("regexp")), "{got:?}");
    }

    #[test]
    fn sql_in_strings() {
        const DQ: &str = "string.quoted.double.rust";
        const RAW: &str = "string.quoted.double.raw.rust";
        let dml = |s: &str| format!("{s} keyword.other.DML.sql");
        let l = [
            "// language=SQL",
            r#"const Q: &str = "SELECT a FROM t WHERE b = '{x}'\n";"#,
            r#"let rows = sqlx::query!("DELETE FROM u WHERE id = $1", id)"#,
            r##"    .fetch_all(&pool).await?; diesel::sql_query(r#"UPDATE "u" SET n = 2"#);"##,
            "let n = sqlx::query_as!(",
            "    User,",
            r##"    r#"INSERT INTO u -- a note"##,
            r##"    VALUES (1)"#, id"##,
            ");",
            r#"conn.execute("CREATE TABLE p (id INTEGER)", params![]);"#,
            r#"let s = query_scalar::<_, i64>("UNION ALL SELECT 1");"#,
            r#"stmt.execute(["LIMIT"]); stmt.query("ORDER"); f("HAVING");"#,
            r#"conn.execute(&format!("JOIN {}", t), []); query_as!(User, "WHERE")"#,
            r#"query_as::<_, User>("GROUP BY x")"#,
        ];
        // a marker's string; the host's escape between SQL's tokens, `{x}` SQL's text
        has(&l, "SELECT", &dml(DQ));
        has(&l, r"\n", &format!("{DQ} constant.character.escape.rust"));
        has(&l, "{x}", &format!("{DQ} string.quoted.single.sql"));
        // a macro's and a function's first argument, a raw string's `"` SQL's
        has(&l, "DELETE", &dml(DQ));
        has(&l, "UPDATE", &dml(RAW));
        has(&l, "u", &format!("{RAW} string.quoted.double.sql"));
        // `query_as!`'s after its type, over lines; SQL's line comment ends with its line
        has(&l[4..9], "INSERT INTO", &dml(RAW));
        has(
            &l,
            " a note",
            &format!("{RAW} comment.line.double-dash.sql"),
        );
        has(
            &l[4..9],
            "VALUES",
            &format!("{RAW} keyword.other.DML.II.sql"),
        );
        has(
            &l,
            "CREATE",
            &format!("{DQ} meta.create.sql keyword.other.create.sql"),
        );
        // a turbofish
        has(&l, "UNION ALL", &dml(DQ));
        has(&l, "GROUP BY", &dml(DQ));
        has(&l, "WHERE", &dml(DQ));
        // a prepared statement's arguments, any other call, a string in a call in the argument
        for w in ["LIMIT", "ORDER", "HAVING", "JOIN"] {
            assert!(
                of(&l)
                    .iter()
                    .any(|(t, s)| t.contains(w) && !s.contains(".sql")),
                "{w}: {:#?}",
                of(&l)
            );
        }
    }

    #[test]
    fn items() {
        let l = ["pub fn main<T>(x: u32, self) -> Option<T> where T: Copy {"];
        has(&l, "pub", "storage.modifier.rust");
        has(&l, "fn", "storage.type.function.rust");
        has(&l, "main", "entity.name.function.rust");
        has(&l, "x", "variable.parameter.rust");
        has(&l, "u32", "storage.type.rust");
        has(&l, "self", "variable.parameter.rust");
        has(&l, "Option", "support.type.rust");
        has(&l, "where", "keyword.other.rust");
        let l = ["impl<'a> fmt::Display for Foo<'a> {"];
        has(&l, "impl", "storage.type.impl.rust");
        has(&l, "for", "keyword.other.rust");
        has(&l, "Foo", "entity.name.impl.rust");
        has(&l, "'a", "storage.modifier.lifetime.rust");
        let l = [
            "struct S {",
            "    pub a: Vec<u8>,",
            "    b: &'static str,",
            "}",
        ];
        has(&l, "S", "entity.name.struct.rust");
        has(&l, "a", "variable.other.member.rust");
        has(&l, "b", "variable.other.member.rust");
        has(&l, ":", "punctuation.separator.type.rust");
    }

    /// A float with an exponent and no dot is a float, which bat's grammar leaves plain (X-008):
    /// with a sign, underscores and `f32`/`f64`; an integer suffix or no exponent digit isn't one.
    #[test]
    fn floats_without_a_dot() {
        let l = ["1e3 + 1E-3 + 2e10f64 + 1_000e+3_ + 0e0 + 1e3_f32 + 0x1e3"];
        for f in ["1e3", "1E-3", "2e10", "1_000e+3_", "0e0", "1e3_"] {
            has(&l, f, "constant.numeric.float.rust");
        }
        has(&l, "f64", "storage.type.numeric.rust");
        has(&l, "f32", "storage.type.numeric.rust");
        has(&l, "0x1e3", "constant.numeric.integer.hexadecimal.rust");
        let got = of(&["1e; 1e3u8; 1e+"]);
        assert!(!got.iter().any(|(_, s)| s.contains("numeric")), "{got:#?}");
    }

    #[test]
    fn strings_and_comments() {
        let l = [
            r##"let s = r#"raw"#; let b = b"\x00"; let c = '\n';"##,
            "println!(\"{} {x:?} {{}}\", a); // $x note",
            "/** doc",
            "  * star",
            "  */ let x = \"multi",
            "line\";",
        ];
        has(
            &l,
            "r",
            "string.quoted.double.raw.rust storage.type.string.rust",
        );
        has(
            &l,
            "#\"",
            "string.quoted.double.raw.rust punctuation.definition.string.begin.rust",
        );
        has(
            &l,
            "\\x00",
            "string.quoted.double.rust constant.character.escape.rust",
        );
        has(
            &l,
            "\\n",
            "string.quoted.single.rust constant.character.escape.rust",
        );
        has(
            &l,
            "{}",
            "string.quoted.double.rust constant.other.placeholder.rust",
        );
        has(
            &l,
            "{x:?}",
            "string.quoted.double.rust constant.other.placeholder.rust",
        );
        has(
            &l,
            "{{}}",
            "string.quoted.double.rust constant.character.escape.rust",
        );
        has(
            &l,
            "$x",
            "comment.line.double-slash.rust variable.other.rust",
        );
        has(
            &l,
            "/** ",
            "comment.block.documentation.rust punctuation.definition.comment.rust",
        );
        has(
            &l,
            "*",
            "comment.block.documentation.rust punctuation.definition.comment.rust",
        );
        has(&l, "line", "string.quoted.double.rust");
    }

    #[test]
    fn attributes_and_closures() {
        let l = [
            "#[cfg(not(feature = \"x\"))]",
            "let f = |a, b: u8| a + b;",
            "xs.iter().map(|x| x * 2)",
        ];
        has(&l, "cfg", "meta.annotation.rust variable.annotation.rust");
        has(
            &l,
            "not",
            "meta.annotation.rust meta.function-call.rust variable.function.rust",
        );
        has(&l, "feature ", "meta.annotation.rust");
        has(&l, "f", "entity.name.function.rust");
        has(&l, "a", "variable.parameter.rust");
        has(&l, "x", "variable.parameter.rust");
        has(&l, "map", "support.function.rust");
    }

    /// A block lexed in one call lexes as its lines one at a time: the scan kept across them
    /// ends each line as a state would, a stack deeper than a state holds cut at its bottom.
    #[test]
    fn a_block_lexes_as_its_lines() {
        let deep = format!("{}\n{}\n", "{(".repeat(20), "[x, ".repeat(9));
        let text = format!(
            "{deep}let s = r##\"a\n\"#\"##; // language=SQL\nlet q =\n\"SELECT 1\";\n#[a(\nb)] struct S(u8,\nu16);\n{}",
            "})".repeat(25)
        );
        let (mut want, mut state, mut spans) = (Vec::new(), State::default(), Vec::new());
        for line in text.split('\n') {
            crate::lex(Lang::Rust, line, &mut state, &mut spans);
            want.push((line.to_owned(), spans.clone(), state));
        }
        assert!(!want[1].2.is_exact(), "the stack outgrew a state");
        for max in [1, 2, usize::MAX] {
            let (mut got, mut state, mut buf) = (Vec::new(), State::default(), String::new());
            let mut rest = Some(&text[..]);
            while let Some(t) = rest {
                let next = crate::lex_lines(
                    Lang::Rust,
                    t,
                    max,
                    &mut state,
                    (&mut spans, &mut buf),
                    |_, _| false,
                    |l, s| got.push((l.to_owned(), s.to_vec())),
                );
                // a call's state is its last line's
                assert_eq!(
                    state,
                    want[got.len() - 1].2,
                    "max {max}, line {}",
                    got.len()
                );
                rest = next.map(|p| &t[p..]);
            }
            let lines: Vec<_> = want
                .iter()
                .map(|(l, s, _)| (l.clone(), s.clone()))
                .collect();
            assert_eq!(got, lines, "max {max}");
        }
    }

    /// The type frames, ada's own (X-056): a generic's bounds with `?Sized` stay a generic (bat's
    /// grammar marked the `?` invalid and read on as an impl's name), a nested `>` ends, a
    /// const generic's block and a macro in a type hold code.
    #[test]
    fn types() {
        let l = ["impl<T: ?Sized + Message> private::Sealed for *const T {}"];
        has(&l, "?", "keyword.operator.rust");
        has(&l, "Sized", "support.type.rust");
        has(&l, "for", "keyword.other.rust");
        has(&l, ">", "punctuation.definition.generic.end.rust");
        let l = ["fn f<R: Rng + ?Sized>(&self, rng: &mut R) -> Vec<Option<u8>> {}"];
        has(&l, "self", "variable.parameter.rust");
        has(&l, "rng", "variable.parameter.rust");
        has(&l, ">>", "punctuation.definition.generic.end.rust");
        let l = ["let x = Iso8601::<{ CONFIG }>; let t = p.parse::<Token![=]>()?;"];
        has(&l, "CONFIG", "constant.other.rust");
        has(&l, "Token!", "support.macro.rust");
        has(&l, "=", "keyword.operator.assignment.rust");
        let l = [
            "struct S<'a, #[may_dangle] T: 'a> {",
            "    f: Option<(",
            "        u32, // n",
            "    )>,",
            "}",
        ];
        has(
            &l,
            "may_dangle",
            "meta.annotation.rust variable.annotation.rust",
        );
        has(&l, " n", "comment.line.double-slash.rust");
    }

    /// A tuple struct's fields are a type frame (X-063): its own `)` ends them, not the first
    /// in a type inside (bat's `with_prototype`), a field's attribute is one, `pub` a modifier;
    /// a macro's `struct $name` is named, and a brace ends fields a misread header opened.
    /// The headers on frames (X-064): a struct's `where`, and a trait's or an enum's generics,
    /// supertraits and `where`, read as a function's are (bat's grammar leaves them plain); a
    /// header misread from a macro's words ends at a closer, so the block around it still does.
    #[test]
    fn headers() {
        let l = ["struct S<T> where T: Copy { a: T }"];
        has(&l, "where", "keyword.other.rust");
        has(&l, "a", "variable.other.member.rust");
        let l = ["trait Tr<T>: Clone + Send where Self: Sized {}"];
        has(&l, "<", "punctuation.definition.generic.begin.rust");
        has(&l, ":", "punctuation.separator.rust");
        has(&l, "where", "keyword.other.rust");
        let l = ["enum E<T> { A(T) }"];
        has(&l, ">", "punctuation.definition.generic.end.rust");
        // serde's `forward_to_deserialize_any!`: `struct enum` ends at the block's `}`
        let l = ["m! { map struct enum identifier }", "fn f() {}"];
        has(&l, "f", "entity.name.function.rust");
    }

    /// A `macro_rules!` matcher's groups balanced (X-064; bat's ends at the first `)` in it), a
    /// repetition inside a repetition, a matcher in `[…]`, a string and a comment in one.
    #[test]
    fn matchers() {
        let l = [
            "macro_rules! m {",
            "    (($a:expr) $( $( $b:ident ),* );* \")\" // x",
            "    ) => { $a };",
            "    [$c:tt] => {};",
            "}",
            "fn f() {}",
        ];
        has(&l, "$a", "variable.parameter.macro.rust");
        has(&l, "expr", "storage.type.rust");
        has(&l, "$b", "variable.parameter.macro.rust");
        has(&l, ")", "string.quoted.double.rust");
        has(&l, " x", "comment.line.double-slash.rust");
        has(&l, "$c", "variable.parameter.macro.rust");
        has(&l, "tt", "storage.type.rust");
        has(&l, "f", "entity.name.function.rust");
    }

    #[test]
    fn tuple_fields() {
        let l = ["pub struct W(Box<dyn Fn(u8) -> u8>, u32);"];
        has(&l, ">", "punctuation.definition.generic.end.rust");
        let l = ["struct M<K, V>(PhantomData<(K, V)>);"];
        has(&l, ">", "punctuation.definition.generic.end.rust");
        let l = ["struct A(#[serde(skip)] pub u8, String);"];
        has(&l, "serde", "meta.annotation.rust variable.annotation.rust");
        has(&l, "pub", "storage.modifier.rust");
        has(&l, "u8", "storage.type.rust");
        let l = [
            "macro_rules! m { ($name:ident) => {",
            "    pub struct $name {",
            "        message: &'static str,",
            "    }",
            "}}",
        ];
        has(&l, "message", "variable.other.member.rust");
        let l = ["struct S", "$(impl X {", "    fn f() {}", "})*"];
        has(&l, "f", "entity.name.function.rust");
    }

    #[test]
    fn state_carries_raw_strings_and_nesting() {
        let l = [
            "let s = r##\"a",
            "\"# still",
            "end\"##; fn f(",
            "  a: u8,",
            ") {}",
        ];
        has(&l, "\"# still", "string.quoted.double.raw.rust");
        has(&l, "a", "variable.parameter.rust");
    }
}
