//! C# on a scanner of ada's own (X-122, ADR-41), in place of bat's 205 contexts compiled by hand:
//! one loop over a line's bytes, a byte's action one load from a table ([`ACT`]), a name read
//! once and looked up once ([`words`]), punctuation one `match`, tokens straight to the sink.
//! C# 9–12 read as C# reads it: records, `init`, `required`, `file`, `scoped`, raw string
//! literals (`"""`, `$"""`, `$$"""`), file-scoped namespaces, `with`, list and property
//! patterns, `and`/`or`/`not`, target-typed `new(…)`, primary constructors.
//!
//! What a name is comes from where it stands: the token before ([`Scan::prev`]), the frame it is
//! in ([`Scan::frames`]: what each open bracket opened, five bits a level) and the statement so
//! far ([`Scan::st`]). No lookahead past a byte or two but at three places a name's or a group's
//! meaning hangs on:
//!
//! - **A declaration's start** ([`names`]): at a statement's start, a member's, or a list's
//!   item, a type (a path, its type arguments, `?`, `*`, `[]`) followed by a name is a
//!   declaration's: the type read once more as one ([`Scan::ty`]).
//! - **A `<` after a name** is a generic's where its type arguments close before one of the
//!   bytes C#'s grammar names (`(`, `)`, `.`, `;`…).
//! - **A `(` where an expression starts** keeps a mark of the writing ([`Group`]): a `=>` after
//!   its `)` takes the writing back and reads it again as a lambda's parameters, and a type
//!   alone in it before an operand as a cast's.
//!
//! Kinds are bat's leaves (its scopes for the token), with `meta.annotation` and
//! `meta.preprocessor` where a theme keys on them (the color census, `x122/tuples-*.txt`).
//! Strings, comments, doc comments, attributes and preprocessor lines are in [`text`]; a regex in
//! a string (the first argument of `new Regex(`, the second of `Regex.IsMatch(`…) on
//! [`crate::regex`]'s .NET dialect, as before.

mod names;
mod regex_calls;
mod text;
mod words;

#[cfg(test)]
mod tests;

use std::mem::MaybeUninit;

use crate::emit::{Mark, Rules, Run, Runs, Sink, Spans};
use crate::{Out, State, regex};

macro_rules! leaves {
    ($($name:ident = [$($s:literal),*],)*) => {
        /// The leaves: a token's scopes.
        static LEAVES: [&[&str]; Leaf::Count as usize] = [$(&[$(concat!($s, ".cs")),*],)*];
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[repr(u16)]
        enum Leaf { $($name,)* Count }
        $(#[allow(dead_code)] pub(super) const $name: u16 = Leaf::$name as u16;)*
    };
}

leaves! {
    PLAIN = [],
    // comments
    LC_MARK = ["comment.line.double-slash", "punctuation.definition.comment"],
    LC_TEXT = ["comment.line.double-slash"],
    BC_BEGIN = ["comment.block", "punctuation.definition.comment.begin"],
    BC_END = ["comment.block", "punctuation.definition.comment.end"],
    BC_TEXT = ["comment.block"],
    DOC_MARK = ["comment.block.documentation", "punctuation.definition.comment.documentation"],
    DOC_TEXT = ["comment.block.documentation", "text.documentation"],
    DOC = ["comment.block.documentation"],
    DOC_TAG_BEGIN = ["comment.block.documentation", "punctuation.definition.tag.begin"],
    DOC_TAG_END = ["comment.block.documentation", "punctuation.definition.tag.end"],
    DOC_NAME_BEGIN = ["comment.block.documentation", "entity.name.tag.begin"],
    DOC_NAME_END = ["comment.block.documentation", "entity.name.tag.end"],
    DOC_ATTR = ["comment.block.documentation", "entity.other.attribute-name"],
    DOC_EQ = ["comment.block.documentation", "punctuation.separator.argument.value"],
    DOC_STR = ["comment.block.documentation", "string.quoted.double"],
    // strings: a regular one's, a verbatim or raw one's (bat's `raw` is the verbatim `@"…"`; C#
    // 11's raw literals take it too)
    STR = ["string.quoted.double"],
    STR_BEGIN = ["string.quoted.double", "punctuation.definition.string.begin"],
    STR_END = ["string.quoted.double", "punctuation.definition.string.end"],
    STR_ESCAPE = ["string.quoted.double", "constant.character.escape"],
    STR_SUFFIX = ["string.quoted.double", "storage.type.string"],
    STR_PH_BEGIN = ["string.quoted.double", "constant.other.placeholder", "punctuation.definition.placeholder.begin"],
    STR_PH_END = ["string.quoted.double", "constant.other.placeholder", "punctuation.definition.placeholder.end"],
    STR_PH_NUM = ["string.quoted.double", "constant.other.placeholder", "constant.numeric.integer.decimal"],
    STR_PH = ["string.quoted.double", "constant.other.placeholder"],
    STR_PH_ALIGN_SEP = ["string.quoted.double", "constant.other.placeholder", "punctuation.separator.arguments"],
    STR_PH_ALIGN = ["string.quoted.double", "constant.other.placeholder", "constant.numeric.integer.decimal.formatting"],
    STR_PH_FORMAT_SEP = ["string.quoted.double", "constant.other.placeholder", "constant.other.format-spec", "punctuation.separator"],
    STR_PH_FORMAT = ["string.quoted.double", "constant.other.placeholder", "constant.other.format-spec"],
    VSTR = ["string.quoted.double.raw"],
    VSTR_BEGIN = ["string.quoted.double.raw", "punctuation.definition.string.begin"],
    VSTR_END = ["string.quoted.double.raw", "punctuation.definition.string.end"],
    VSTR_ESCAPE = ["string.quoted.double.raw", "constant.character.escape"],
    VSTR_PH_BEGIN = ["string.quoted.double.raw", "constant.other.placeholder", "punctuation.definition.placeholder.begin"],
    VSTR_PH_END = ["string.quoted.double.raw", "constant.other.placeholder", "punctuation.definition.placeholder.end"],
    VSTR_PH_NUM = ["string.quoted.double.raw", "constant.other.placeholder", "constant.numeric.integer.decimal"],
    VSTR_PH = ["string.quoted.double.raw", "constant.other.placeholder"],
    VSTR_PH_ALIGN_SEP = ["string.quoted.double.raw", "constant.other.placeholder", "punctuation.separator.arguments"],
    VSTR_PH_ALIGN = ["string.quoted.double.raw", "constant.other.placeholder", "constant.numeric.integer.decimal.formatting"],
    VSTR_PH_FORMAT_SEP = ["string.quoted.double.raw", "constant.other.placeholder", "constant.other.format-spec", "punctuation.separator"],
    VSTR_PH_FORMAT = ["string.quoted.double.raw", "constant.other.placeholder", "constant.other.format-spec"],
    INTERP_BEGIN = ["punctuation.section.interpolation.begin"],
    INTERP_END = ["punctuation.section.interpolation.end"],
    ALIGN_SEP = ["punctuation.separator.arguments"],
    ALIGN = ["constant.numeric.integer.decimal.formatting"],
    FORMAT_SEP = ["constant.other.format-spec", "punctuation.separator"],
    FORMAT = ["constant.other.format-spec"],
    CHAR = ["constant.character"],
    CHAR_ESCAPE = ["constant.character", "constant.character.escape"],
    // numbers
    DEC = ["constant.numeric.integer.decimal"],
    DEC_SUFFIX = ["constant.numeric.integer.decimal", "storage.type.numeric"],
    FLOAT = ["constant.numeric.float.decimal"],
    FLOAT_DOT = ["constant.numeric.float.decimal", "punctuation.separator.decimal"],
    FLOAT_SUFFIX = ["constant.numeric.float.decimal", "storage.type.numeric"],
    HEX = ["constant.numeric.integer.hexadecimal"],
    HEX_BASE = ["constant.numeric.integer.hexadecimal", "punctuation.definition.numeric.base"],
    HEX_SUFFIX = ["constant.numeric.integer.hexadecimal", "storage.type.numeric"],
    BIN = ["constant.numeric.integer.binary"],
    BIN_BASE = ["constant.numeric.integer.binary", "punctuation.definition.numeric.base"],
    // the preprocessor
    PP_TEXT = ["meta.preprocessor"],
    PP_HASH = ["meta.preprocessor", "keyword.other.preprocessor", "punctuation.definition.preprocessor"],
    PP_CONTROL = ["meta.preprocessor", "keyword.control.preprocessor"],
    PP_OTHER = ["meta.preprocessor", "keyword.other.preprocessor"],
    PP_FLAG = ["meta.preprocessor", "constant.other.flag"],
    PP_LOGICAL = ["meta.preprocessor", "keyword.operator.logical"],
    PP_PARENS = ["meta.preprocessor", "punctuation.section.parens"],
    PP_LANGUAGE = ["meta.preprocessor", "constant.language"],
    PP_CONSTANT = ["meta.preprocessor", "entity.name.constant"],
    PP_SECTION = ["meta.preprocessor", "entity.name.section"],
    PP_WARNING = ["meta.preprocessor", "string.unquoted.warning"],
    PP_MESSAGE = ["meta.preprocessor", "string.unquoted"],
    PP_NUM = ["meta.preprocessor", "constant.numeric.integer.decimal"],
    PP_STR = ["meta.preprocessor", "string.quoted.double"],
    PP_STR_BEGIN = ["meta.preprocessor", "string.quoted.double", "punctuation.definition.string.begin"],
    PP_STR_END = ["meta.preprocessor", "string.quoted.double", "punctuation.definition.string.end"],
    PP_LC_MARK = ["meta.preprocessor", "comment.line.double-slash", "punctuation.definition.comment"],
    PP_LC_TEXT = ["meta.preprocessor", "comment.line.double-slash"],
    // attributes
    ANN = ["meta.annotation"],
    ANN_BEGIN = ["meta.annotation", "punctuation.definition.annotation.begin"],
    ANN_END = ["meta.annotation", "punctuation.definition.annotation.end"],
    ANN_NAME = ["meta.annotation", "variable.annotation"],
    ANN_SEP = ["meta.annotation", "punctuation.separator.annotation"],
    ANN_TARGET = ["meta.annotation", "storage.modifier.target"],
    ANN_TARGET_SEP = ["meta.annotation", "punctuation.separator.key-value"],
    ANN_DOT = ["meta.annotation", "punctuation.accessor.dot"],
    ANN_GROUP_BEGIN = ["meta.annotation", "punctuation.section.group.begin"],
    ANN_GROUP_END = ["meta.annotation", "punctuation.section.group.end"],
    ANN_ARG_SEP = ["meta.annotation", "punctuation.separator.argument"],
    ANN_ARG_VALUE_SEP = ["meta.annotation", "punctuation.separator.argument.value"],
    ANN_VAR = ["meta.annotation", "variable.other"],
    ANN_PARAM = ["meta.annotation", "variable.parameter"],
    ANN_ASSIGN = ["meta.annotation", "keyword.operator.assignment"],
    ANN_OPERATOR = ["meta.annotation", "keyword.operator"],
    ANN_TYPE = ["meta.annotation", "support.type"],
    ANN_GENERIC_BEGIN = ["meta.annotation", "punctuation.definition.generic.begin"],
    ANN_GENERIC_END = ["meta.annotation", "punctuation.definition.generic.end"],
    // punctuation
    BLOCK_BEGIN = ["punctuation.section.block.begin"],
    BLOCK_END = ["punctuation.section.block.end"],
    BRACES_BEGIN = ["punctuation.section.braces.begin"],
    BRACES_END = ["punctuation.section.braces.end"],
    GROUP_BEGIN = ["punctuation.section.group.begin"],
    GROUP_END = ["punctuation.section.group.end"],
    PARAMS_BEGIN = ["punctuation.section.parameters.begin"],
    PARAMS_END = ["punctuation.section.parameters.end"],
    BRACKETS_BEGIN = ["punctuation.section.brackets.begin"],
    BRACKETS_END = ["punctuation.section.brackets.end"],
    GENERIC_BEGIN = ["punctuation.definition.generic.begin"],
    GENERIC_END = ["punctuation.definition.generic.end"],
    TERMINATOR = ["punctuation.terminator.statement"],
    ARG_SEP = ["punctuation.separator.argument"],
    PARAM_SEP = ["punctuation.separator.parameter.function"],
    VARS_SEP = ["punctuation.separator.variables"],
    TYPE_SEP = ["punctuation.separator.type"],
    INHERITED_SEP = ["punctuation.separator.inherited-class"],
    ENUM_SEP = ["punctuation.separator.enum"],
    ARRAY_SEP = ["punctuation.separator.array-element"],
    TUPLE_SEP = ["punctuation.separator.tuple"],
    CASE_SEP = ["punctuation.separator.case-statement"],
    LABEL_SEP = ["punctuation.separator"],
    ARG_VALUE_SEP = ["punctuation.separator.argument.value"],
    PREBODY_SEP = ["punctuation.separator.function"],
    ENUM_TYPE_SEP = ["punctuation.separator.key-value.type"],
    NS_SEP = ["punctuation.separator.namespace"],
    NS_NAME_SEP = ["entity.name.namespace", "punctuation.separator.namespace"],
    DOT = ["punctuation.accessor.dot"],
    DOT_NS = ["punctuation.accessor.dot.namespace"],
    NULL_DOT = ["punctuation.accessor.null-coalescing"],
    ARROW = ["punctuation.accessor.arrow"],
    DOUBLE_COLON = ["punctuation.accessor.double-colon.namespace"],
    // operators
    OPERATOR = ["keyword.operator"],
    ASSIGN = ["keyword.operator.assignment"],
    ASSIGN_VAR = ["keyword.operator.assignment.variable"],
    ASSIGN_LOOP = ["keyword.operator.assignment.variable.loop"],
    DEFAULT_VALUE = ["keyword.operator.assignment.default-value"],
    TERNARY = ["keyword.operator.ternary"],
    POINTER = ["keyword.operator.pointer"],
    NEW_OP = ["keyword.operator.new"],
    REFLECTION = ["keyword.operator.reflection"],
    LOGICAL = ["keyword.operator.logical"],
    LAMBDA = ["storage.type.function.lambda"],
    FUNC_ARROW = ["storage.type.function"],
    // words
    STORAGE_TYPE = ["storage.type"],
    VAR_KW = ["storage.type.variable"],
    NULLABLE = ["storage.type.nullable"],
    CLASS_KW = ["storage.type.class"],
    STRUCT_KW = ["storage.type.struct"],
    INTERFACE_KW = ["storage.type.interface"],
    ENUM_KW = ["storage.type.enum"],
    NAMESPACE_KW = ["storage.type.namespace"],
    DELEGATE_KW = ["storage.type.delegate"],
    MODIFIER = ["storage.modifier"],
    ACCESS = ["storage.modifier.access"],
    PARAM_MOD = ["storage.modifier.parameter"],
    ARG_MOD = ["storage.modifier.argument"],
    MOD_OTHER = ["storage.modifier.other"],
    ACC_GET = ["storage.type.function.accessor.get"],
    ACC_SET = ["storage.type.function.accessor.set"],
    ACC_INIT = ["storage.type.function.accessor.init"],
    ACC_ADD = ["storage.type.function.accessor.add"],
    ACC_REMOVE = ["storage.type.function.accessor.remove"],
    IMPORT = ["keyword.control.import"],
    USING_KW = ["keyword.control.using"],
    IF_KW = ["keyword.control.conditional.if"],
    ELSE_KW = ["keyword.control.conditional.else"],
    ELSEIF_KW = ["keyword.control.conditional.elseif"],
    SWITCH_KW = ["keyword.control.flow.switch"],
    CASE_KW = ["keyword.control.switch.case"],
    WHEN_KW = ["keyword.control.switch.case.when"],
    TRY_WHEN = ["keyword.control.trycatch.when"],
    FOR_KW = ["keyword.control.loop.for"],
    FOREACH_KW = ["keyword.control.loop.foreach"],
    IN_KW = ["keyword.control.flow"],
    WHILE_KW = ["keyword.control.loop.while"],
    DO_KW = ["keyword.control.loop.do"],
    TRY_KW = ["keyword.control.trycatch.try"],
    CATCH_KW = ["keyword.control.trycatch.catch"],
    FINALLY_KW = ["keyword.control.trycatch.finally"],
    THROW_KW = ["keyword.control.trycatch.throw"],
    RETURN_KW = ["keyword.control.flow.return"],
    BREAK_KW = ["keyword.control.flow.break"],
    GOTO_KW = ["keyword.control.flow.goto"],
    LOCK_KW = ["keyword.control.other.lock"],
    FIXED_KW = ["keyword.control.other.fixed"],
    AWAIT = ["keyword.control.other"],
    KW_OTHER = ["keyword.other"],
    SQL = ["keyword.other.sql"],
    LANGUAGE = ["constant.language"],
    VAR_LANGUAGE = ["variable.language"],
    DISCARD = ["variable.language.deconstruction.discard"],
    // names
    VAR_OTHER = ["variable.other"],
    VAR_FUNCTION = ["variable.function"],
    MEMBER = ["variable.other.member"],
    PARAMETER = ["variable.parameter"],
    SUPPORT_TYPE = ["support.type"],
    INHERITED = ["entity.other.inherited-class"],
    FUNC = ["entity.name.function"],
    CTOR = ["entity.name.function.constructor"],
    DTOR = ["entity.name.function.destructor"],
    CLASS_NAME = ["entity.name.class"],
    STRUCT_NAME = ["entity.name.struct"],
    INTERFACE_NAME = ["entity.name.interface"],
    ENUM_NAME = ["entity.name.enum"],
    NAMESPACE_NAME = ["entity.name.namespace"],
    LABEL = ["entity.name.label"],
    LABEL_REF = ["constant.other.label"],
    ENUM_MEMBER = ["constant.other.enum"],
    CONSTANT = ["constant.other"],
    DELEGATE_NAME = ["variable.other.member.delegate"],
}

const _: () = assert!((Leaf::Count as u16) < 256);

/// A regex in a regular string: [`regex::KINDS`] kinds from here, under the string's scope.
const RE_STR: u16 = 256;
/// A regex in a verbatim string.
const RE_VSTR: u16 = RE_STR + regex::KINDS as u16;

/// The scopes under `source.cs` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    if kind >= RE_STR {
        let (s, k) = if kind >= RE_VSTR {
            ("string.quoted.double.raw.cs", kind - RE_VSTR)
        } else {
            ("string.quoted.double.cs", kind - RE_STR)
        };
        let mut out = vec![s];
        out.extend(regex::scopes(k));
        return out;
    }
    LEAVES
        .get(usize::from(kind))
        .map_or_else(Vec::new, |l| l.to_vec())
}

// A byte's action ([`ACT`]).
const B_OTHER: u8 = 0;
const B_SPACE: u8 = 1;
const B_NAME: u8 = 2;
const B_DIGIT: u8 = 3;
const B_QUOTE: u8 = 4;
const B_APOS: u8 = 5;
const B_SLASH: u8 = 6;
const B_AT: u8 = 7;
const B_DOLLAR: u8 = 8;
const B_LPAREN: u8 = 9;
const B_RPAREN: u8 = 10;
const B_LBRACE: u8 = 11;
const B_RBRACE: u8 = 12;
const B_LBRACK: u8 = 13;
const B_RBRACK: u8 = 14;
const B_SEMI: u8 = 15;
const B_COMMA: u8 = 16;
const B_DOT: u8 = 17;
const B_COLON: u8 = 18;
const B_QUEST: u8 = 19;
/// `=`, `<`, `>`, `+`, `-`, `*`, `%`, `&`, `|`, `^`, `!`, `~`: an operator.
const B_OP: u8 = 20;
/// A byte past ASCII: a name's character, or another.
const B_WIDE: u8 = 21;

static ACT: [u8; 256] = {
    let mut t = [B_WIDE; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = match b {
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => B_SPACE,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => B_NAME,
            b'0'..=b'9' => B_DIGIT,
            b'"' => B_QUOTE,
            b'\'' => B_APOS,
            b'/' => B_SLASH,
            b'@' => B_AT,
            b'$' => B_DOLLAR,
            b'(' => B_LPAREN,
            b')' => B_RPAREN,
            b'{' => B_LBRACE,
            b'}' => B_RBRACE,
            b'[' => B_LBRACK,
            b']' => B_RBRACK,
            b';' => B_SEMI,
            b',' => B_COMMA,
            b'.' => B_DOT,
            b':' => B_COLON,
            b'?' => B_QUEST,
            b'=' | b'<' | b'>' | b'+' | b'-' | b'*' | b'%' | b'&' | b'|' | b'^' | b'!' | b'~' => {
                B_OP
            }
            _ => B_OTHER,
        };
        c += 1;
    }
    t
};

/// Where the indentation from `p` ends: eight spaces a step (C#'s lines are indented deep), then
/// a blank at a time.
#[inline(always)]
fn indent(b: &[u8], mut p: usize) -> usize {
    while let Some(c) = b.get(p..p + 8) {
        if u64::from_le_bytes(c.try_into().unwrap_or([0; 8])) != u64::from_le_bytes(*b"        ") {
            break;
        }
        p += 8;
    }
    while p < b.len() && ACT[usize::from(b[p])] == B_SPACE {
        p += 1;
    }
    p
}

/// Whether `b[..p]` is blanks only (a line's indentation).
fn blanks_before(b: &[u8], p: usize) -> bool {
    b[..p].iter().all(|&c| ACT[usize::from(c)] == B_SPACE)
}

/// A name's bytes past its first: ASCII's word bytes (a byte past ASCII is looked at apart).
static NAME: [bool; 256] = crate::prim::WORD;

// The frames, five bits a level in [`Scan::frames`]; 0 is none (the top level, or a level lost
// past [`LEVELS`]). What each `{`, `(` and `[` opened.
/// A namespace's body.
const F_NS: u8 = 1;
/// A class's, a struct's, an interface's or a record's body: members.
const F_TYPE: u8 = 2;
/// An enum's body.
const F_ENUM: u8 = 3;
/// A block of statements: a method's body, a statement's block.
const F_CODE: u8 = 4;
/// A lambda's or an anonymous method's body, in an expression.
const F_LAMBDA: u8 = 5;
/// A property's, an indexer's or an event's accessors.
const F_ACCESSORS: u8 = 6;
/// An initializer's braces: an object's, a collection's, an array's, an anonymous type's, a
/// property pattern's.
const F_INIT: u8 = 7;
/// A switch expression's arms.
const F_SWITCH: u8 = 8;
/// A regular interpolated string's hole, `$"…{` … `}`.
const F_INTERP: u8 = 9;
/// A verbatim interpolated string's hole, `$@"…{` … `}`.
const F_VINTERP: u8 = 10;
/// A raw interpolated string's hole, its `$`s in [`Scan::rawd`].
const F_RINTERP: u8 = 11;
/// Parentheses in an expression: a group, a call's arguments, a tuple.
const F_PAREN: u8 = 12;
/// A statement's head: `if (`, `while (`, `switch (`, `foreach (`, `using (`, `lock (`, `fixed (`,
/// `catch (`.
const F_COND: u8 = 13;
/// A `for`'s head.
const F_FOR: u8 = 14;
/// A method's, a constructor's, a delegate's, an operator's parameters.
const F_PARAMS: u8 = 15;
/// A type's parameters in its header: a record's, a primary constructor's.
const F_HPARAMS: u8 = 16;
/// A lambda's or an anonymous method's parameters.
const F_LPARAMS: u8 = 17;
/// An indexer's parameters, `this[` … `]`.
const F_IPARAMS: u8 = 18;
/// Brackets in an expression: an index, a collection expression, a list pattern.
const F_BRACKET: u8 = 19;
/// An attribute's list across lines, `[` … `]` ([`text`]).
const F_ATTR: u8 = 20;
/// An attribute's arguments across lines, `(` … `)`, [`Scan::attr`] deep.
const F_ATTR_ARGS: u8 = 21;
/// A call's arguments, `f(` … `)`: a name before `:` a named argument's.
const F_ARGS: u8 = 22;
/// Bits a level, and the levels [`Scan::frames`] holds.
const FBITS: u32 = 5;
const LEVELS: u8 = 24;
const FMASK: u128 = (1 << FBITS) - 1;
/// The frames' bits [`State`] keeps: [`LEVELS`] levels, in its levels' 14 bytes and `withs[0]`.
const KEPT_FRAMES: u128 = (1 << (FBITS * LEVELS as u32)) - 1;
const _: () = assert!(FBITS * LEVELS as u32 <= 120);

// What a line can end in ([`Scan::mode`]).
const M_CODE: u8 = 0;
/// A block comment.
const M_COMMENT: u8 = 1;
/// A verbatim string, `@"…`.
const M_VSTR: u8 = 2;
/// A verbatim interpolated string, `$@"…`.
const M_VISTR: u8 = 3;
/// A raw string, [`Scan::rawq`] quotes, [`Scan::rawd`] `$`s.
const M_RAW: u8 = 4;
/// A regex in a verbatim string, its state in [`Scan::re`].
const M_VREGEX: u8 = 5;

// The token before ([`Scan::prev`]): what a name, a `(`, a `{` after it is.
/// A statement's or a member's start: after `;`, `{`, `}`, a modifier, an attribute.
const P_STMT: u8 = 0;
/// After an operator, `=`, `return`: an expression's start.
const P_EXPR: u8 = 1;
/// After a list's `(`, `[` or `,`: an expression's start, or a declaration's (a tuple's element,
/// an `out` variable, a `foreach`'s), or a named argument's name.
const P_LIST: u8 = 2;
/// After an operand: a name, a literal, `)`, `]`.
const P_VALUE: u8 = 3;
/// After `.`, `?.`, `->`, `::`: a member.
const P_DOT: u8 = 4;
/// A type next: after `new`, `is`, `as`, `typeof(`, a pattern's `and`, `or`, `not`.
const P_TYPE: u8 = 5;
/// After a declaration's type: the name it declares next.
const P_DECLARE: u8 = 6;
/// After a declared name.
const P_DECLARED: u8 = 7;
/// After `class`, `struct`, `interface`, `enum`, `record` ([`Scan::ent`]): its name next.
const P_ENTITY: u8 = 8;
/// In a type's header after its name: `<`, `(`, `:`, `where`, `{`.
const P_HEAD: u8 = 9;
/// In a base list or a constraint's types: a base's name next.
const P_BASE: u8 = 10;
/// After `goto`: a label next.
const P_GOTO: u8 = 11;
/// After a statement's word whose `(` is its head ([`F_COND`]): `if`, `while`…
const P_COND: u8 = 12;
/// After `for`.
const P_FOR: u8 = 13;
/// After `typeof`, `sizeof`, `default`: a type in parentheses next.
const P_TYPEOF: u8 = 14;
/// In parameters at a parameter's start.
const P_PARAM: u8 = 15;
/// After a method's, a local function's, a delegate's `)`: `{`, `=>`, `;`, `where`, `:`.
const P_PARAMS_DONE: u8 = 16;
/// After a lambda's parameters (a name or `)`): its `=>` next.
const P_LAMBDA: u8 = 17;
/// After an accessor's word: `;`, `{`, `=>`.
const P_ACCESSOR: u8 = 18;
/// A switch expression's arm's start: a pattern.
const P_ARM: u8 = 19;
/// After `catch`: its `(` holds a type and a name.
const P_CATCH: u8 = 20;
/// After `delegate` in an expression: its parameters' `(` next.
const P_ANON: u8 = 22;
/// After a function's name (a method's, a constructor's, a local function's, a delegate's, an
/// operator's), an indexer's `this`: its parameters next, a method's type parameters.
const P_FUNC: u8 = 23;
/// After `=>`: an expression's start, a lambda's block.
const P_ARROW: u8 = 24;
/// After a switch expression's `switch`: its arms' `{` next.
const P_SWITCH: u8 = 25;
/// After `new`, `stackalloc`: a type next, or a target-typed `(`, an array's `[`, an anonymous
/// type's `{`.
const P_NEW: u8 = 26;

// What the statement is so far ([`Scan::st`]): its kind in the low three bits, flags above.
const S_NONE: u8 = 0;
/// A declaration: a `,` at its level starts another declarator.
const S_DECL: u8 = 1;
/// A type's header.
const S_HEAD: u8 = 2;
/// A `where`'s constraints.
const S_WHERE: u8 = 3;
/// A delegate's declaration: its name a delegate's.
const S_DELEGATE: u8 = 4;
/// A type's base list.
const S_BASE: u8 = 5;
/// A constructor's initializer, `: base(…)`: its body's `{` next.
const S_PREBODY: u8 = 6;
const S_KIND: u8 = 7;
/// An expression is under way (an initializer's, after `=`): `{` opens an initializer.
const S_EXPR: u8 = 8;
/// A query expression is under way: its words are keywords.
const S_QUERY: u8 = 16;
/// A pattern is under way (after `is`, `case`, an arm's start): `and`, `or`, `not` are keywords.
const S_PAT: u8 = 32;
/// A `case` or `default` waits for its `:`.
const S_CASE: u8 = 64;

// What the next `{` opens, where a word said ([`Scan::pending`]).
const PEND_NONE: u8 = 0;
const PEND_NS: u8 = 1;
const PEND_TYPE: u8 = 2;
const PEND_ENUM: u8 = 3;
const PEND_CODE: u8 = 4;
const PEND_LAMBDA: u8 = 5;
const PEND_INIT: u8 = 6;
const PEND_SWITCH: u8 = 7;

/// Where [`State::withs`] keeps the scan's bytes past the frames' (the frames take the levels'
/// 14 bytes and `withs[0]`).
const W_PREV: usize = 1;
const W_ST: usize = 2;
const W_PENDING: usize = 3;
const W_ENT: usize = 4;
const W_RAWQ: usize = 5;
const W_RAWD: usize = 6;
const W_TERNARY: usize = 7;
const W_ATTR: usize = 8;
const W_RE: usize = 9;
const W_DOC: usize = 10;

/// Lex `b`, a line of C# without its `\n`, from `state`.
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

/// A block of C# on one scan opened from `state` ([`crate::Carry`]).
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

/// A `(` where an expression starts, on the line: the writing before it, for a `=>` or a cast
/// after its `)` to take back ([`Scan::groups`]).
#[derive(Clone, Copy)]
pub(super) struct Group {
    mark: Mark,
    /// Where its `(` is.
    open: u32,
    /// The frames' depth with it open.
    depth: u8,
}

/// The groups a line keeps marks for, innermost last.
const GROUPS: usize = 4;

struct Scan<'b, 'c, K: Sink> {
    b: &'b [u8],
    /// What each open frame is ([`F_NS`]…), [`FBITS`] a level, the innermost lowest; past
    /// [`LEVELS`] the outer ones are lost (and the state inexact).
    frames: u128,
    /// How many frames are open, up to 255.
    depth: u8,
    exact: bool,
    /// What the line ends in so far ([`M_CODE`]…).
    mode: u8,
    /// The token before ([`P_STMT`]…).
    prev: u8,
    /// What the statement is so far ([`S_NONE`]…).
    st: u8,
    /// What the next `{` opens ([`PEND_NONE`]…).
    pending: u8,
    /// What the entity's word before names ([`words::E_CLASS`]…).
    ent: u8,
    /// An open raw string's quotes, and with them whether it is a multi-line one (its high bit).
    rawq: u8,
    /// An open raw string's `$`s (0: not interpolated).
    rawd: u8,
    /// The `?`s whose `:` hasn't come, in the statement.
    ternary: u8,
    /// An attribute's open parentheses ([`F_ATTR_ARGS`]).
    attr: u8,
    /// A regex in a verbatim string across lines: its state on [`crate::regex`].
    re: u8,
    /// The line before was a doc comment's: a doc comment's line's indentation is the
    /// comment's (bat's, whose comment takes the line's end).
    doc: bool,
    /// The line's marked groups ([`Group`]), the first [`Scan::ng`] of them set: not set up
    /// with the scan (each line builds one, the guide's mistake 6).
    groups: [MaybeUninit<Group>; GROUPS],
    ng: u8,
    /// The line's `Regex.IsMatch(`-like calls, found at its first string after a `,`.
    rx: Option<regex_calls::RegexCalls>,
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
            mode: M_CODE,
            prev: P_STMT,
            st: S_NONE,
            pending: PEND_NONE,
            ent: 0,
            rawq: 0,
            rawd: 0,
            ternary: 0,
            attr: 0,
            re: 0,
            doc: false,
            groups: [const { MaybeUninit::uninit() }; GROUPS],
            ng: 0,
            rx: None,
            k,
            out,
        }
    }

    /// The scan's fields from `state`: the frames in its levels and the first with, the rest a
    /// byte each in the withs, the mode in [`State::arg`].
    #[inline(always)]
    fn load(&mut self, state: &State) {
        let mut w = [0u8; 16];
        w[..14].copy_from_slice(&state.levels);
        w[14] = state.withs[0];
        self.frames = u128::from_le_bytes(w);
        self.depth = state.depth;
        self.exact = !state.inexact;
        self.prev = state.withs[W_PREV];
        self.st = state.withs[W_ST];
        self.pending = state.withs[W_PENDING];
        self.ent = state.withs[W_ENT];
        self.rawq = state.withs[W_RAWQ];
        self.rawd = state.withs[W_RAWD];
        self.ternary = state.withs[W_TERNARY];
        self.attr = state.withs[W_ATTR];
        self.re = state.withs[W_RE];
        self.doc = state.withs[W_DOC] != 0;
        self.mode = state.arg as u8;
    }

    /// The state the scan's last line left: every field as it is, so a block lexes as its
    /// lines do one at a time.
    #[inline(always)]
    fn state(&self) -> State {
        let w = self.frames.to_le_bytes();
        let mut s = State::default();
        s.levels.copy_from_slice(&w[..14]);
        s.withs[0] = w[14];
        s.withs[W_PREV] = self.prev;
        s.withs[W_ST] = self.st;
        s.withs[W_PENDING] = self.pending;
        s.withs[W_ENT] = self.ent;
        s.withs[W_RAWQ] = self.rawq;
        s.withs[W_RAWD] = self.rawd;
        s.withs[W_TERNARY] = self.ternary;
        s.withs[W_ATTR] = self.attr;
        s.withs[W_RE] = self.re;
        s.withs[W_DOC] = u8::from(self.doc);
        s.depth = self.depth;
        s.inexact = !self.exact;
        s.arg = u16::from(self.mode);
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

    /// A write: one call on the run sink ([`Sink::merge_call`]); the common tokens' write inlined
    /// ([`Scan::put_hot`]).
    #[inline(always)]
    fn put(&mut self, s: usize, e: usize, kind: u16) {
        self.k.merge_call(&mut self.out, s, e, kind);
    }

    /// A write inlined, for the common tokens.
    #[inline(always)]
    fn put_hot(&mut self, s: usize, e: usize, kind: u16) {
        self.k.merge(&mut self.out, s, e, kind);
    }

    #[inline(always)]
    fn top(&self) -> u8 {
        (self.frames & FMASK) as u8
    }

    /// A frame `f` opened. The levels past [`LEVELS`] are dropped here, as [`State`] drops
    /// them: a block's scan keeps none that the line API's wouldn't (the guide's mistake 4).
    #[inline(always)]
    fn push(&mut self, f: u8) {
        self.frames = (self.frames << FBITS | u128::from(f)) & KEPT_FRAMES;
        self.depth = self.depth.saturating_add(1);
        if self.depth > LEVELS {
            self.exact = false;
        }
    }

    #[inline(always)]
    fn pop(&mut self) {
        if self.depth > 0 {
            self.frames >>= FBITS;
            self.depth -= 1;
        }
    }

    /// The line, from what the line before left: a mode's rest first, then a directive, a doc
    /// comment's line or the code; the loop out of line where `SHARED` ([`Scan::code_shared`]).
    #[inline(always)]
    fn lex_line<const SHARED: bool>(&mut self) {
        self.ng = 0;
        self.rx = None;
        let b = self.b;
        let doc = self.doc;
        self.doc = false;
        let p = match self.mode {
            M_CODE => {
                // a byte-order mark: nothing (bat's derails on it)
                let mut p = if b.len() > 2 && b[0] == 0xef && b[1] == 0xbb && b[2] == 0xbf {
                    3
                } else {
                    0
                };
                p = indent(b, p);
                if p < b.len() && b[p] == b'#' {
                    self.directive(p);
                    return;
                }
                if p + 2 < b.len()
                    && b[p] == b'/'
                    && b[p + 1] == b'/'
                    && b[p + 2] == b'/'
                    && b.get(p + 3) != Some(&b'/')
                {
                    if doc {
                        self.put(0, p, DOC);
                    }
                    self.doc_line(p);
                    self.doc = true;
                    return;
                }
                p
            }
            _ => self.resume(),
        };
        let p = match self.top() {
            // an attribute's list goes on: its indentation is `meta.annotation`'s too
            F_ATTR | F_ATTR_ARGS if self.mode == M_CODE && p > 0 && blanks_before(b, p) => {
                self.attr_run(0)
            }
            F_ATTR | F_ATTR_ARGS => self.attr_run(p),
            _ => p,
        };
        if SHARED {
            self.code_shared(p);
        } else {
            self.code(p);
        }
    }

    /// [`Scan::code`] out of line: on the run sink, one copy that the line API and a block's
    /// lines share; on the span sink, which ada doesn't hold, inlined.
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
                B_SPACE => {
                    let mut e = p + 1;
                    while e < n && ACT[usize::from(b[e])] == B_SPACE {
                        e += 1;
                    }
                    e
                }
                B_NAME => self.name(p),
                B_DIGIT => self.number(p),
                B_QUOTE => self.quote(p, p, 0),
                B_APOS => self.char_lit(p),
                B_SLASH => match b.get(p + 1) {
                    Some(b'/') => self.line_comment(p),
                    Some(b'*') => self.block_comment_open(p),
                    _ => self.op(p),
                },
                B_AT => self.at_sign(p),
                B_DOLLAR => self.dollar(p),
                B_LPAREN => self.lparen(p),
                B_RPAREN => self.rparen(p),
                B_LBRACE => self.lbrace(p),
                B_RBRACE => self.rbrace(p),
                B_LBRACK => self.lbrack(p),
                B_RBRACK => self.rbrack(p),
                B_SEMI => self.semi(p),
                B_COMMA => self.comma(p),
                B_DOT => self.dot(p),
                B_COLON => self.colon(p),
                B_QUEST => self.quest(p),
                B_OP => self.op(p),
                B_WIDE => self.wide(p),
                _ => p + 1,
            };
        }
    }

    /// A byte past ASCII at `p`: a name's start, a space, or a character of its own.
    #[inline(never)]
    fn wide(&mut self, p: usize) -> usize {
        let b = self.b;
        if crate::prim::is_word_starting(b, p) {
            return self.name(p);
        }
        // a byte-order mark, a space past ASCII: nothing
        p + crate::prim::char_len(b[p])
    }
}
