//! C and C++ by a scanner of ada's own design (X-057's redesign, ADR-41: speed first), in place
//! of bat's contexts compiled by hand (deleted at R4). Rust's shape (X-056): one loop over a line's
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
//! depth), with the token before the line's first ([`Scan::prev`]) and the statement so far
//! ([`Scan::decl`]).
//!
//! A name before `(` is a definition's or a call's by the statement it is in: at a namespace's
//! or a class's level, after a type and outside parentheses, a definition's, whose list's words
//! before `,` or `)` are its parameters ([`D_PARAMS`]); elsewhere a call's. The names after
//! `class`, `struct`, `union`, `enum`, `namespace`, `using` and in a `typedef` are told by the
//! word before them ([`P_ENTITY`]) and the byte after. bat's rules where a line's end or a macro
//! decides are followed as probed (vault `ada-parse.md`, X-057's R2).
//!
//! A string an engine's call takes as a pattern (`std::regex re("…")`, `regcomp(&re, "…", …)`,
//! `QRegularExpression(…)`, X-115) is a regex on [`crate::regex`], in the dialect the call
//! picks ([`regex_calls`]): its escapes and end the string's, the rest the regex's kinds from
//! [`RE_DQ`]. A raw one carries the regex's state over lines ([`M_RAW_RE`]).
//!
//! A string SQL calls take (`sqlite3_exec(db, "…")`, `query.exec("…")`, FMDB's `executeQuery:`,
//! X-134) is SQL on [`crate::sql`] ([`sql_calls`]), from [`SQL_DQ`]; its adjacent literals go on with
//! it, over lines.
//!
//! Objective-C and Objective-C++ (X-134) are C's and C++'s rows with Objective-C's words and
//! tokens ([`objc`]): sends, method heads, `@` keywords, Cocoa's words. Their scan is compiled
//! apart (`Scan`'s `O`), so C's and C++'s has none of their checks.
//!
//! CMake's header templates (X-140, [`template`]) are C's and C++'s scan on the template's line
//! masked (`#cmakedefine` a `#define`, a placeholder a name), the placeholders' spans laid over
//! its spans after.
//!
//! Stages (vault `ada-parse.md`, X-057's redesign): **R1** this loop and the directive lexer
//! ([`pp`]); **R2** definitions, parameters and the entity names; **R3** the tail parity
//! showed (generics and template parameters, declared names, the preprocessor's corners);
//! **R4** C and C++ on it alone, the compiled scanner deleted. Judged against syntect by
//! color (each byte's theme rule), not scope for scope: where it reads C or C++ otherwise than
//! bat, by design, is in the vault's X-057 notes. Its readings are held by
//! `tests/readings.rs`' snapshots (X-116), which replaced ada's grammar tests.

mod cocoa;
mod code;
mod objc;
mod pp;
mod regex_calls;
mod sql_calls;
pub(crate) mod support;
mod template;
mod words;

pub use template::{line_cmake_c, line_cmake_cpp};

use crate::emit::{Rules, Run, Runs, Sink, Spans};
use crate::{Out, State, regex, sql};

macro_rules! leaves {
    ($($name:ident = [$($s:literal),*],)*) => {
        /// The leaves: a token's scopes, C++'s, C's, Objective-C's and Objective-C++'s.
        static LEAVES: [[&[&str]; 4]; Leaf::Count as usize] = [
            $([
                &[$(concat!($s, ".c++")),*],
                &[$(concat!($s, ".c")),*],
                &[$(concat!($s, ".objc")),*],
                &[$(concat!($s, ".objc++")),*],
            ],)*
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
    IMPORT_IMPORT = ["keyword.control.import.import"],
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
    // under `meta.preprocessor`, as bat has it: a theme without a rule for the name (catppuccin's)
    // paints it the directive's
    PP_CONSTANT = ["meta.preprocessor", "entity.name.constant.preprocessor"],
    LABEL = ["entity.name.label"],
    FUNC_DEF = ["entity.name.function"],
    CLASS_NAME = ["entity.name.class"],
    STRUCT_NAME = ["entity.name.struct"],
    UNION_NAME = ["entity.name.union"],
    ENUM_NAME = ["entity.name.enum"],
    NAMESPACE_NAME = ["entity.name.namespace"],
    NAMESPACE_SEP = ["entity.name.namespace", "punctuation.accessor.double-colon"],
    TYPEDEF_NAME = ["entity.name.type.typedef"],
    USING_NAME = ["entity.name.type.using"],
    // Objective-C's (X-134): an `@` keyword's `@` and word, by bat's scopes for each
    AT_STORAGE = ["storage.type", "punctuation.definition.storage.type"],
    AT_PROPERTY = ["keyword.other.property", "punctuation.definition.keyword"],
    PROPERTY = ["keyword.other.property"],
    AT_DIRECTIVE = ["keyword.other.property.directive", "punctuation.definition.keyword"],
    DIRECTIVE = ["keyword.other.property.directive"],
    AT_EXCEPTION = ["keyword.control.exception", "punctuation.definition.keyword"],
    EXCEPTION = ["keyword.control.exception"],
    AT_SYNC = ["keyword.control.synchronize", "punctuation.definition.keyword"],
    SYNC = ["keyword.control.synchronize"],
    AT_MODIFIER = ["storage.modifier", "punctuation.definition.storage.modifier"],
    AT_OTHER = ["keyword.other", "punctuation.definition.keyword"],
    KEYWORD_OTHER = ["keyword.other"],
    AT_CONTROL = ["keyword.control", "punctuation.definition.keyword"],
    AT_IMPORT = ["keyword.control.import", "punctuation.definition.keyword"],
    // `@interface Name : Super <Protocols>`
    TYPE_NAME = ["entity.name.type"],
    INHERIT_MARK = ["punctuation.definition.entity.other.inherited-class"],
    INHERITED = ["entity.other.inherited-class"],
    SCOPE_BEGIN = ["punctuation.section.scope.begin"],
    SCOPE_END = ["punctuation.section.scope.end"],
    // a send's selector: its first part, then its keyword parts
    SELECTOR = ["support.function.any-method"],
    SELECTOR_SEP = ["support.function.any-method", "punctuation.separator.arguments"],
    SELECTOR_PART = ["support.function.any-method.name-of-parameter"],
    SELECTOR_PART_SEP = [
        "support.function.any-method.name-of-parameter",
        "punctuation.separator.arguments"
    ],
    // a method's head: `- (Type)name:(Type)param part:(Type)param`
    TYPE_PAREN = ["punctuation.definition.type"],
    METHOD_SEP = ["entity.name.function.name-of-parameter", "punctuation.separator.arguments"],
    METHOD_PART = ["entity.name.function.name-of-parameter"],
    METHOD_PARAM = ["variable.parameter.function"],
    PROP_ATTR = ["keyword.other.property.attribute"],
    SELECTOR_PAREN = ["punctuation.definition.storage.type"],
    PROTOCOL_MODIFIER = ["storage.modifier.protocol"],
    ID_TYPE = ["storage.type.id"],
    SELECTOR_VAR = ["variable.other.selector"],
    NSAPP = ["support.variable.foundation"],
    MACRO_CONTROL = ["keyword.control.macro"],
    // Cocoa's words ([`cocoa`]), in its lists' order: the first and last are [`COCOA_FIRST`],
    // [`COCOA_LAST`]
    COCOA_CLASS = ["support.class.cocoa"],
    COCOA_CLASS_LEOPARD = ["support.class.cocoa.leopard"],
    QUARTZ_CLASS = ["support.class.quartz"],
    COCOA_TYPE = ["support.type.cocoa"],
    COCOA_TYPE_LEOPARD = ["support.type.cocoa.leopard"],
    QUARTZ_TYPE = ["support.type.quartz"],
    COCOA_FUNCTION = ["support.function.cocoa"],
    COCOA_FUNCTION_LEOPARD = ["support.function.cocoa.leopard"],
}

/// Cocoa's first and last leaves ([`cocoa::Cocoa`]'s order).
const COCOA_FIRST: u16 = COCOA_CLASS;
const COCOA_LAST: u16 = COCOA_FUNCTION_LEOPARD;

const _: () = assert!((Leaf::Count as u16) <= RE_DQ);

/// A regex in a string ([`regex_calls`]): [`regex::KINDS`] kinds from here, under the string's
/// scope.
const RE_DQ: u16 = 256;
/// SQL in a string ([`sql_calls`], X-134): [`sql::KINDS`] kinds from here, under the string's
/// scope.
const SQL_DQ: u16 = RE_DQ + 128;
const _: () = assert!(regex::KINDS <= 128 && sql::KINDS <= 128);

/// The scopes under `source.c++` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    scopes_in(kind, 0)
}

/// The scopes under `source.c` a kind carries, outermost first.
pub fn scopes_c(kind: u16) -> Vec<&'static str> {
    scopes_in(kind, 1)
}

/// The scopes under `source.objc` a kind carries, outermost first.
pub fn scopes_objc(kind: u16) -> Vec<&'static str> {
    scopes_in(kind, 2)
}

/// The scopes under `source.objc++` a kind carries, outermost first.
pub fn scopes_objcpp(kind: u16) -> Vec<&'static str> {
    scopes_in(kind, 3)
}

/// The scopes under `source.cmake.config.c` a kind of a C header's template carries (X-140),
/// outermost first: C's, and a placeholder's ([`template`]).
pub fn scopes_cmake_c(kind: u16) -> Vec<&'static str> {
    scopes_in(kind, 1)
}

/// The scopes under `source.cmake.config.c++` a kind of a C++ header's template carries.
pub fn scopes_cmake_cpp(kind: u16) -> Vec<&'static str> {
    scopes_in(kind, 0)
}

/// The languages' rows: [`LEAVES`]' columns, [`words::INFO`]'s rows.
const L_CPP: usize = 0;
const L_C: usize = 1;
const L_OBJC: usize = 2;
const L_OBJCPP: usize = 3;

fn scopes_in(kind: u16, lang: usize) -> Vec<&'static str> {
    if kind >= template::TPL {
        return template::scopes(kind, lang);
    }
    if kind >= SQL_DQ {
        if usize::from(kind - SQL_DQ) >= sql::KINDS {
            return Vec::new();
        }
        let mut out = scopes_in(DQ, lang);
        out.extend(sql::scopes(kind - SQL_DQ));
        return out;
    }
    if kind >= RE_DQ {
        let mut out = vec![
            [
                "string.quoted.double.c++",
                "string.quoted.double.c",
                "string.quoted.double.objc",
                "string.quoted.double.objc++",
            ][lang],
        ];
        out.extend(regex::scopes(kind - RE_DQ));
        return out;
    }
    LEAVES
        .get(usize::from(kind))
        .map_or_else(Vec::new, |l| l[lang].to_vec())
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
/// The levels [`State`] keeps of [`Scan::braces`] (its 15 bytes; the 16th is [`Scan::one`]).
const LEVELS: usize = 60;

// What a line can end in ([`Scan::mode`]).
const M_CODE: u8 = 0;
const M_BLOCK_COMMENT: u8 = 1;
/// A `//` comment whose line ends in `\`.
const M_LINE_COMMENT: u8 = 2;
/// A `"…"` string whose line ends in `\`.
const M_DQ: u8 = 3;
/// A `'…'` whose line ends in `\`.
const M_SQ: u8 = 4;
/// An `#if 0`'s lines, [`Scan::aux`] the `#if`s open in them.
const M_DISABLED: u8 = 5;
/// A raw string ([`Scan::delim`]). The raw modes are the last two: one compare tells them.
const M_RAW: u8 = 6;
/// A raw string that is a regex: its delimiter's first [`RE_DELIM_KEPT`] bytes in
/// [`Scan::delim`], then the regex's dialect ([`RE_DIALECT`]) and state ([`RE_STATE`]).
const M_RAW_RE: u8 = 7;

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
/// A word that names what follows ([`Scan::entity`]): `class`, `struct`, `union`, `enum`,
/// `namespace`, `using`, `operator`.
const P_ENTITY: u8 = 6;
/// A definition's header that a line's end ended (no `;` or `{`): a `:` next is its
/// initializer list's.
const P_DEFINED: u8 = 7;

// What a statement at a namespace's or a class's level is so far ([`Scan::decl`]), for what a
// name before `(` is: a definition's (or a declaration's) name, or a call.
/// Nothing that rules a definition out: a name before `(` after a type is one.
const D_NONE: u8 = 0;
/// No definition: after a `=` (an initializer), a `:` (an initializer list, a base list).
const D_DONE: u8 = 1;
/// After a definition's parameters: no definition, and a line's end ends the statement
/// (`void f(int n) MACRO(n)`, no `;`).
const D_DEFINED: u8 = 2;
/// A parameter's default value: code, to its `,` or the list's `)`.
const D_DEFAULT: u8 = 3;
/// A `typedef`'s, at its own level: the name before its `;` is the type's.
const D_TYPEDEF: u8 = 4;
/// A definition's parameter list: the word before a `,`, `)`, `[` or `=` is a parameter.
const D_PARAMS: u8 = 5;

// What a word names ([`Scan::entity`], [`ENTITY_LEAF`]).
const E_CLASS: u8 = 1;
const E_STRUCT: u8 = 2;
const E_UNION: u8 = 3;
const E_ENUM: u8 = 4;
const E_NAMESPACE: u8 = 5;
const E_USING: u8 = 6;
const E_OPERATOR: u8 = 7;
/// `template`: a `<` next opens its parameters ([`Scan::tpl`]).
const E_TEMPLATE: u8 = 8;
/// A cast's word (`static_cast`): a `<` next opens its type.
const E_CAST: u8 = 9;
/// Each entity's leaf for the name it names.
const ENTITY_LEAF: [u16; 10] = [
    PLAIN,
    CLASS_NAME,
    STRUCT_NAME,
    UNION_NAME,
    ENUM_NAME,
    NAMESPACE_NAME,
    USING_NAME,
    FUNC_DEF,
    PLAIN,
    PLAIN,
];

// [`State::arg`]'s bits.
const A_MODE: u16 = 0x7;
const A_PP: u32 = 3;
const A_AUX: u32 = 5;
const A_PENDING: u32 = 10;
const A_PREV: u32 = 12;
/// A `?` waits for its `:`.
const A_TERNARY: u16 = 0x8000;
// What the next `{` opens where a word said ([`Scan::pending`]), in [`State`]'s two bits (code
// is never pending: a `{` at a namespace's or a class's level opens it unasked).
/// No `{` pending.
const NO_PENDING: u8 = 0;
/// A namespace's level: `namespace`, C++'s `extern "C"`.
const PEND_NS: u8 = 1;
/// A class's body: `class`, `struct`, `union`.
const PEND_CLASS: u8 = 2;
/// An initializer's braces: `enum`'s.
const PEND_INIT: u8 = 3;

/// A raw string's delimiter's bytes [`State`] keeps; a longer one is told by these and its
/// length.
const DELIM_KEPT: usize = 8;
/// A regex's raw string's: the last two bytes keep the regex's dialect and state.
const RE_DELIM_KEPT: usize = DELIM_KEPT - 2;
/// Where [`Scan::delim`] keeps a raw regex's dialect ([`regex_calls::dialect_code`]).
const RE_DIALECT: usize = DELIM_KEPT - 2;
/// Where [`Scan::delim`] keeps a raw regex's state ([`crate::regex`]'s byte).
const RE_STATE: usize = DELIM_KEPT - 1;
/// Where [`State::withs`] keeps the statement's state: [`Scan::decl`], [`Scan::paren`] and
/// [`Scan::angle`] in one byte, then [`Scan::td`], [`Scan::entity`] (with [`Scan::tpl`] and
/// [`Scan::kw`]), [`Scan::ifd`].
const W_DECL: usize = 2 + DELIM_KEPT;

/// Lex `b`, a line of C++ without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    line_on(b, state, out, L_CPP, Spans::new());
}

/// Lex `b`, a line of C, from `state`.
#[inline(never)]
pub fn line_c(b: &[u8], state: &mut State, out: &mut Out) {
    line_on(b, state, out, L_C, Spans::new());
}

/// Lex `b`, a line of Objective-C, from `state`.
#[inline(never)]
pub fn line_objc(b: &[u8], state: &mut State, out: &mut Out) {
    line_on(b, state, out, L_OBJC, Spans::new());
}

/// Lex `b`, a line of Objective-C++, from `state`.
#[inline(never)]
pub fn line_objcpp(b: &[u8], state: &mut State, out: &mut Out) {
    line_on(b, state, out, L_OBJCPP, Spans::new());
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
    let mut k = line_on(b, state, out, L_CPP, Runs::new(rules, runs));
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
    let mut k = line_on(b, state, out, L_C, Runs::new(rules, runs));
    k.end_line(b.len());
}

/// [`line_objc`]'s runs by theme rule pushed to `runs`.
#[inline(never)]
pub fn line_runs_objc(
    b: &[u8],
    state: &mut State,
    out: &mut Out,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
) {
    let mut k = line_on(b, state, out, L_OBJC, Runs::new(rules, runs));
    k.end_line(b.len());
}

/// [`line_objcpp`]'s runs by theme rule pushed to `runs`.
#[inline(never)]
pub fn line_runs_objcpp(
    b: &[u8],
    state: &mut State,
    out: &mut Out,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
) {
    let mut k = line_on(b, state, out, L_OBJCPP, Runs::new(rules, runs));
    k.end_line(b.len());
}

#[inline(always)]
fn line_on<K: Sink>(b: &[u8], state: &mut State, out: &mut Out, lang: usize, k: K) -> K {
    if lang >= L_OBJC {
        line_in::<K, true>(b, state, out, lang, k)
    } else {
        line_in::<K, false>(b, state, out, lang, k)
    }
}

#[inline(always)]
fn line_in<K: Sink, const O: bool>(
    b: &[u8],
    state: &mut State,
    out: &mut Out,
    lang: usize,
    k: K,
) -> K {
    let mut s = Scan::<K, O>::new(b, out, lang, k);
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
    block_on(state, spans, L_CPP, pass)
}

/// [`block`] for C.
pub fn block_c(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    block_on(state, spans, L_C, pass)
}

/// [`block`] for Objective-C.
pub fn block_objc(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    block_on(state, spans, L_OBJC, pass)
}

/// [`block`] for Objective-C++.
pub fn block_objcpp(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    block_on(state, spans, L_OBJCPP, pass)
}

fn block_on(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    lang: usize,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    if lang >= L_OBJC {
        block_in::<true>(state, spans, lang, pass)
    } else {
        block_in::<false>(state, spans, lang, pass)
    }
}

fn block_in<const O: bool>(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    lang: usize,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    let mut out = Out::new(spans);
    let mut s = Scan::<Spans, O>::new(b"", &mut out, lang, Spans::new());
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
    block_runs_on(state, spans, L_CPP, rules, runs, lines)
}

/// [`block_runs`] for C.
pub fn block_runs_c(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    lines: &mut crate::RunLines,
) -> Option<usize> {
    block_runs_on(state, spans, L_C, rules, runs, lines)
}

/// [`block_runs`] for Objective-C.
pub fn block_runs_objc(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    lines: &mut crate::RunLines,
) -> Option<usize> {
    block_runs_on(state, spans, L_OBJC, rules, runs, lines)
}

/// [`block_runs`] for Objective-C++.
pub fn block_runs_objcpp(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    lines: &mut crate::RunLines,
) -> Option<usize> {
    block_runs_on(state, spans, L_OBJCPP, rules, runs, lines)
}

fn block_runs_on(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    lang: usize,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    lines: &mut crate::RunLines,
) -> Option<usize> {
    if lang >= L_OBJC {
        block_runs_in::<true>(state, spans, lang, rules, runs, lines)
    } else {
        block_runs_in::<false>(state, spans, lang, rules, runs, lines)
    }
}

fn block_runs_in<const O: bool>(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    lang: usize,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    lines: &mut crate::RunLines,
) -> Option<usize> {
    let mut out = Out::new(spans);
    let k = Runs::new(rules, runs);
    let mut s = Scan::<Runs, O>::new(b"", &mut out, lang, k);
    s.load(state);
    s.out.0.clear();
    s.k.open(s.out, 64);
    let rest = lines.pass(&mut s);
    *state = s.state();
    rest
}

impl<const O: bool> crate::Carry for Scan<'_, '_, Spans, O> {
    fn next(&mut self, b: &[u8]) -> &[crate::Span] {
        // SAFETY: the line is set back to an empty one before `b`'s borrow ends
        self.carry(unsafe { crate::line_for(b) });
        self.out.0
    }
}

impl<const O: bool> crate::CarryRuns for Scan<'_, '_, Runs<'_>, O> {
    fn next(&mut self, b: &[u8]) -> usize {
        // SAFETY: as above
        self.carry(unsafe { crate::line_for(b) });
        self.k.end_line(b.len())
    }
}

/// `O`: Objective-C or Objective-C++, compiled apart so that C's and C++'s scan has none of
/// its checks ([`objc`]).
struct Scan<'b, 'c, K: Sink, const O: bool> {
    b: &'b [u8],
    /// What each open `{` opened, two bits a level, the innermost lowest; past [`LEVELS`] the
    /// outer ones are lost.
    braces: u128,
    /// How many `{` are open, up to 255.
    depth: u8,
    /// What the line ends in so far ([`M_CODE`]…).
    mode: u8,
    /// The `#if`s open since an `#if 1`, it among them (0: none): the `#else` that closes it
    /// opens a disabled branch, as bat has it.
    one: u8,
    /// The directive the line is in ([`PP_NONE`]…).
    pp: u8,
    /// An `#if 0`'s open `#if`s, or a raw string's delimiter's length.
    aux: u8,
    /// What the next `{` opens, where a word said ([`NO_PENDING`], [`PEND_NS`]…).
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
    /// What the line's strings are checked for ([`RX_RE`]…; 0: none, the loop's string arm as
    /// it was): a regex engine's name came before on the line ([`words::A_REGEX`]), a SQL call's
    /// ([`words::A_SQL`]), a SQL string ended ([`Scan::sql_end`]), a selector that takes SQL or
    /// a regex ([`Scan::sel_at`]), SQL goes on from the line before ([`Scan::sqlp`]).
    rx: u8,
    /// Where the line's last SQL string ended ([`RX_SQL_END`]), and SQL's context there: an
    /// adjacent literal goes on with it.
    sql_end: usize,
    sql_ctx: u8,
    /// Where the string after an Objective-C selector that takes SQL or a regex starts its
    /// argument ([`RX_SEL`]), and which ([`objc::SEL_SQL`], [`objc::SEL_RE`]).
    sel_at: usize,
    sel: u8,
    /// SQL from the line before ([`RX_CARRY`]): `0x10 |` its context; a string that starts the
    /// line goes on with it. Between lines in [`State`]'s `aux` bits, which a line ending in code
    /// has free.
    sqlp: u8,
    /// What the statement is so far, at a namespace's or a class's level ([`D_NONE`]…).
    decl: u8,
    /// The `(`s open in the statement.
    paren: u8,
    /// The generics' `<`s open: template arguments and parameters, a cast's type.
    angle: u8,
    /// Which of the open `<`s are a `template`'s parameters, a bit a depth (the first lowest):
    /// `typename` and `class` in them are a type's words.
    tpl: u8,
    /// A `typedef`'s level plus one, while its statement is open (0: none).
    td: u8,
    /// What the word before names, where [`Scan::prev`] is [`P_ENTITY`] ([`E_CLASS`]…).
    entity: u8,
    /// The statement has a keyword (a type's, a modifier) outside parentheses: a line's end
    /// doesn't end it (`static int` before `foo(…)` on the next line; `HOOK(x, uint8_t)` ends).
    kw: bool,
    /// The level at the innermost open `#if` plus one (0: none): its `#else` or `#elif` closes
    /// what its first branch left open (a function's header in each branch, one body).
    ifd: u8,
    /// Where a directive's text is filled to ([`PP_TEXT`]).
    at: usize,
    /// What each word is in the language ([`words::INFO`]'s row).
    info: &'static [u16],
    /// C or Objective-C, not C++ or Objective-C++.
    c: bool,
    /// The `[`s open (and a send's blocks), two bits a frame, the innermost lowest
    /// ([`objc::F_PLAIN`]…): past 16 the outer ones are lost.
    br: u32,
    /// How many frames [`Scan::br`] has, up to 255.
    nbr: u8,
    /// Where a method's head is ([`objc::H_START`]…; 0: none).
    head: u8,
    /// The `(`s open in a method's head's type.
    htp: u8,
    /// The string is an `@"…"`: `%@` a placeholder.
    at_str: bool,
    k: K,
    out: &'b mut Out<'c>,
}

impl<'b, 'c, K: Sink, const O: bool> Scan<'b, 'c, K, O> {
    #[inline(always)]
    fn new(b: &'b [u8], out: &'b mut Out<'c>, lang: usize, k: K) -> Self {
        Scan {
            b,
            braces: 0,
            depth: 0,
            mode: M_CODE,
            one: 0,
            pp: PP_NONE,
            aux: 0,
            pending: NO_PENDING,
            prev: P_STMT,
            case: false,
            ternary: 0,
            delim: [0; DELIM_KEPT],
            rx: 0,
            sql_end: 0,
            sql_ctx: 0,
            sel_at: 0,
            sel: 0,
            sqlp: 0,
            decl: D_NONE,
            paren: 0,
            angle: 0,
            tpl: 0,
            td: 0,
            entity: 0,
            kw: false,
            ifd: 0,
            at: 0,
            info: &words::INFO[lang],
            c: lang == L_C || lang == L_OBJC,
            br: 0,
            nbr: 0,
            head: 0,
            htp: 0,
            at_str: false,
            k,
            out,
        }
    }

    /// The scan's fields from `state`: the levels' bits in its first 15 bytes, [`Scan::one`],
    /// a raw string's delimiter in the next, the statement's at [`W_DECL`], the rest in
    /// [`State::arg`].
    #[inline(always)]
    fn load(&mut self, state: &State) {
        let mut w = [0u8; 16];
        if O {
            // Objective-C's: its brace levels in 13 bytes, its frames and its head in the next two
            w[..13].copy_from_slice(&state.levels[..13]);
            self.load_objc(state.levels[13], state.withs[0]);
        } else {
            w[..14].copy_from_slice(&state.levels);
            w[14] = state.withs[0];
        }
        self.braces = u128::from_le_bytes(w);
        self.one = state.withs[1];
        self.depth = state.depth;
        let a = state.arg;
        self.mode = (a & A_MODE) as u8;
        self.pp = (a >> A_PP & 3) as u8;
        self.aux = (a >> A_AUX & 31) as u8;
        self.pending = (a >> A_PENDING & 3) as u8;
        self.prev = (a >> A_PREV & 7) as u8;
        self.ternary = u8::from(a & A_TERNARY != 0);
        if self.mode >= M_RAW {
            self.delim.copy_from_slice(&state.withs[2..W_DECL]);
        }
        let d = state.withs[W_DECL];
        (self.decl, self.paren, self.angle) = (d & 7, d >> 3 & 7, d >> 6);
        self.td = state.withs[W_DECL + 1];
        self.entity = state.withs[W_DECL + 2] & 0x0f;
        self.tpl = state.withs[W_DECL + 2] >> 4 & 7;
        self.kw = state.withs[W_DECL + 2] & 0x80 != 0;
        self.ifd = state.withs[W_DECL + 3];
    }

    /// The state the scan's last line left.
    #[inline(always)]
    fn state(&self) -> State {
        let w = self.braces.to_le_bytes();
        let mut s = State::default();
        if O {
            s.levels[..13].copy_from_slice(&w[..13]);
            (s.levels[13], s.withs[0]) = self.objc_state();
        } else {
            s.levels.copy_from_slice(&w[..14]);
            s.withs[0] = w[14];
        }
        s.withs[1] = self.one;
        if self.mode >= M_RAW {
            s.withs[2..W_DECL].copy_from_slice(&self.delim);
        }
        s.withs[W_DECL] = self.decl | self.paren.min(7) << 3 | self.angle.min(3) << 6;
        s.withs[W_DECL + 1] = self.td;
        s.withs[W_DECL + 2] = if self.prev == P_ENTITY {
            self.entity
        } else {
            0
        } | (self.tpl & 7) << 4
            | if self.kw { 0x80 } else { 0 };
        s.withs[W_DECL + 3] = self.ifd;
        s.depth = self.depth;
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
        self.settle();
        self.k.close(self.out);
        self.b = b"";
    }

    /// The fields as [`State`] keeps them ([`Scan::state`], [`Scan::load`]): a block's next
    /// line starts from what the line API's would, so a block lexes as its lines one at a time.
    /// One bit for the open `?`s, 7 `(`s, 3 `<`s, the word naming what follows only before
    /// that name, the first [`LEVELS`] brace levels (Objective-C's [`objc::LEVELS_OBJC`], its
    /// innermost [`objc::FRAMES_KEPT`] frames, its head's type's 3 `(`s).
    #[inline(always)]
    fn settle(&mut self) {
        self.ternary = self.ternary.min(1);
        self.paren = self.paren.min(7);
        self.angle = self.angle.min(3);
        if self.prev != P_ENTITY {
            self.entity = 0;
        }
        if O {
            self.settle_objc();
        } else {
            self.braces &= (1u128 << (2 * LEVELS)) - 1;
        }
    }

    /// The line, from what the line before left: a mode's rest first, then a directive or the
    /// code.
    #[inline(always)]
    fn lex_line(&mut self) {
        self.case = false;
        self.rx = 0;
        self.at = 0;
        // SQL the line before left open ([`Scan::sqlp`])
        if self.aux != 0 && self.mode == M_CODE {
            if self.pp == PP_NONE {
                (self.sqlp, self.rx) = (self.aux, RX_CARRY);
            }
            self.aux = 0;
        }
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
            self.code_line(p);
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
                // a function's body; an initializer's braces are code to the statement too
                NS | CLASS => CODE,
                l => l,
            },
            PEND_NS => NS,
            PEND_CLASS => CLASS,
            _ => INIT,
        };
        self.pending = NO_PENDING;
        self.braces = self.braces << 2 | u128::from(kind);
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

    /// A `}` at a line's first byte where code is open: the end of a function's body (or an
    /// initializer) at a namespace's or a class's level, so every code level is closed, back to
    /// that level. Braces an `#if`'s branches leave unbalanced (`{` in one, none in the other)
    /// are made good here, ada's own: bat's contexts stay a level deep.
    #[cold]
    fn resync(&mut self) {
        loop {
            self.close_brace();
            if self.depth == 0 || self.level() < CODE {
                break;
            }
        }
    }

    /// An `#else` or `#elif`: the levels its `#if`'s first branch opened closed again.
    fn branch(&mut self) {
        if self.ifd != 0 && self.depth != u8::MAX {
            let d = self.ifd - 1;
            if self.depth > d {
                let n = u32::from(self.depth - d);
                self.braces = self.braces.checked_shr(2 * n).unwrap_or(0);
                self.depth = d;
            }
        }
    }

    /// A statement's start (`;`, `{`, `}`, a class's section): nothing seen yet. Back at a
    /// `typedef`'s level (`typedef struct { … } name;`), its statement goes on.
    #[inline(always)]
    fn new_statement(&mut self) {
        (self.decl, self.paren, self.angle, self.tpl, self.kw) = (D_NONE, 0, 0, 0, false);
        if self.td != 0 {
            // a typedef's level plus one, as `td` was made
            let level = self.depth.saturating_add(1);
            if self.td == level {
                self.decl = D_TYPEDEF;
            } else if self.td > level {
                self.td = 0;
            }
        }
    }

    /// What a directive keeps of the code around it, to set back after its line
    /// ([`Scan::restore`]).
    #[inline(always)]
    fn kept(&self) -> Kept {
        Kept {
            prev: self.prev,
            pending: self.pending,
            ternary: self.ternary,
            case: self.case,
            decl: self.decl,
            paren: self.paren,
            angle: self.angle,
            tpl: self.tpl,
            td: self.td,
            entity: self.entity,
            kw: self.kw,
            br: self.br,
            nbr: self.nbr,
            head: self.head,
            htp: self.htp,
        }
    }

    #[inline(always)]
    fn restore(&mut self, k: Kept) {
        (self.prev, self.pending, self.ternary, self.case) = (k.prev, k.pending, k.ternary, k.case);
        (
            self.decl,
            self.paren,
            self.angle,
            self.tpl,
            self.td,
            self.entity,
            self.kw,
        ) = (k.decl, k.paren, k.angle, k.tpl, k.td, k.entity, k.kw);
        (self.br, self.nbr, self.head, self.htp) = (k.br, k.nbr, k.head, k.htp);
    }
}

/// The code's state a directive's line sets back ([`Scan::kept`]).
#[derive(Clone, Copy)]
struct Kept {
    prev: u8,
    pending: u8,
    ternary: u8,
    case: bool,
    decl: u8,
    paren: u8,
    angle: u8,
    tpl: u8,
    td: u8,
    entity: u8,
    kw: bool,
    br: u32,
    nbr: u8,
    head: u8,
    htp: u8,
}

// What a line's strings are checked for ([`Scan::rx`]).
/// A regex engine's name.
const RX_RE: u8 = 1;
/// A SQL call's name.
const RX_SQL: u8 = 2;
/// A SQL string ended ([`Scan::sql_end`]).
const RX_SQL_END: u8 = 4;
/// A selector took a string ([`Scan::sel_at`]).
const RX_SEL: u8 = 8;
/// SQL from the line before ([`Scan::sqlp`]).
const RX_CARRY: u8 = 16;
/// [`Scan::delim`]'s dialect byte for a raw string that is SQL, its state SQL's context.
const SQL_CODE: u8 = 0xff;

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
