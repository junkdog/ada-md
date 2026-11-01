//! C# by a scanner of ada's own (ADR-40, speed first): bat's `C#` grammar (205 contexts, every
//! one but its strings' and its doc comments' under a prototype of comments and preprocessor
//! lines) compiled by hand, as Rust's, Kotlin's, Java's and C++'s were.
//!
//! Each context that can be open is an arm of one `match` on the top of a stack
//! (`ctx.rs`), trying its patterns in the grammar's order at the position the scan has
//! reached, which is syntect's leftmost match; the regex pieces the patterns share (names,
//! the type lookaheads with their backtracking, numbers) are in `re.rs`. Contexts that differ
//! only in meta scopes no theme keys on are one (the many `{ … }` of statements, the
//! `( … )` of conditions), and the ones that only pass through (`class_body`,
//! `method_body_transition`, `function_call_arguments`) aren't kept.
//!
//! syntect's machinery is followed where the grammar leans on it: a pattern that matches
//! nothing at the place a step starts, with nothing to do, doesn't count for the step (a
//! placeholder's `\s*(…)?`, `.*` on a preprocessor line); a pop that matches nothing right
//! after a push that matched nothing there, at the depth that push left, is passed over for
//! the context's later patterns, and with none the scan steps a character; the line's `\n`
//! is a space the patterns see, and once it is taken the patterns that match nothing at the
//! line's end still get their turn (a line comment nested in another pops both).
//!
//! A kind is a leaf (a token's scopes, bat's) with what the stack around it adds that themes
//! key on: the string it is in (a chain of string, placeholder and format spec), a doc
//! comment's or a namespace name's scope, `meta.annotation`, `meta.preprocessor`.
//!
//! A string that `new Regex(`, `Regex.IsMatch(input, …)` and the like take is colored as
//! .NET's regex (ADR-40, phase 4; [`crate::regex`]), as Rider colors it: its text is the
//! regex's, and placeholders (`{0}`) aren't looked for in it; the string's end and escapes are
//! the host's, and in a verbatim one `""`.

mod ctx;
mod re;
pub(crate) use re::ND;

use crate::machine::KEPT as HALF;
use crate::{Out, State};

/// The contexts, by bat's names where one is one of them (`#anon_…` by their parent's).
pub(crate) mod c {
    /// `main`, the top level.
    pub const MAIN: u8 = 0;
    /// `{ … }` holding `main`: `#anon_main_0`, a namespace's, a `try`'s.
    pub const MAIN_BRACE: u8 = 1;
    /// The same, its `}` in `meta.block`: an `if`'s and an `else`'s.
    pub const MAIN_BRACE_M: u8 = 2;
    /// `#anon_main_1`: code outside a class, to a visibility or a declaration.
    pub const MAIN_LINE: u8 = 3;
    pub const LINE: u8 = 4;
    pub const LINE_IN: u8 = 5;
    pub const NOSEMI: u8 = 6;
    /// From `enum Name` to its `{`.
    pub const ENUM_HEAD: u8 = 7;
    pub const ENUM_BODY: u8 = 8;
    pub const ENUM_END: u8 = 9;
    pub const USING_NS: u8 = 10;
    pub const NS_DECL: u8 = 11;
    pub const NS_NAME: u8 = 12;
    pub const DELEGATE_TYPE: u8 = 13;
    pub const DELEGATE_NAME: u8 = 14;
    pub const DN_GENERIC: u8 = 15;
    pub const DN_AFTER: u8 = 16;
    pub const DELEGATE_PARAMS: u8 = 17;
    pub const DELEGATE_END: u8 = 18;
    pub const DATA_TYPE_SIG: u8 = 19;
    pub const DTS_GENERIC: u8 = 20;
    pub const TYPE_CONSTRAINT: u8 = 21;
    pub const DATA_TYPE_BODY: u8 = 22;
    pub const DTB_BLOCK: u8 = 23;
    pub const EVENT_HANDLER: u8 = 24;
    pub const EH_AFTER: u8 = 25;
    pub const EH_BLOCK: u8 = 26;
    pub const METHOD_NAME: u8 = 27;
    pub const MN_EXPLICIT: u8 = 28;
    pub const MN_GENERIC: u8 = 29;
    pub const MN_AFTER: u8 = 30;
    pub const MD_TYPE: u8 = 31;
    pub const METHOD_BODY: u8 = 32;
    pub const MB_BLOCK: u8 = 33;
    pub const METHOD_PARAMS: u8 = 34;
    pub const METHOD_PARAMS_BRACKET: u8 = 35;
    pub const METHOD_PARAM: u8 = 36;
    pub const METHOD_PARAM_TYPE: u8 = 37;
    pub const MPT_TYPE: u8 = 38;
    pub const CTOR_PREBODY: u8 = 39;
    pub const CTOR_COLON: u8 = 40;
    pub const CTOR_INIT: u8 = 41;
    pub const CTOR_INIT_AFTER: u8 = 42;
    pub const METHOD_ACCESSOR: u8 = 43;
    pub const ACC_BLOCK: u8 = 44;
    pub const ACC_AFTER: u8 = 45;
    pub const ATTR_IN: u8 = 46;
    pub const ATTR_ARGS: u8 = 47;
    pub const ARGUMENTS: u8 = 48;
    pub const OUT_ARG: u8 = 49;
    pub const ARG_EXPR: u8 = 50;
    pub const ACCESSOR_ARGS: u8 = 51;
    pub const LAMBDA_BODY: u8 = 52;
    pub const LAMBDA_PARAMS: u8 = 53;
    pub const MEMBER_VARS: u8 = 54;
    pub const MV_VALUE: u8 = 55;
    pub const VARS_DECL: u8 = 56;
    pub const VD_VALUE: u8 = 57;
    pub const VD_INIT: u8 = 58;
    pub const INIT_CTOR: u8 = 59;
    pub const NEW: u8 = 60;
    pub const ANON_CLASS: u8 = 61;
    pub const NEW_TYPE: u8 = 62;
    pub const NEW_BRACKETS: u8 = 63;
    pub const CTOR_ARGS: u8 = 64;
    pub const TERNARY: u8 = 65;
    pub const CAST: u8 = 66;
    pub const TUPLE: u8 = 67;
    /// `, … >` of types: `#anon_line_of_code_1`, `#anon_line_of_code_in_no_semicolon_1`
    /// and `_5`.
    pub const GEN_TYPES: u8 = 68;
    /// `( … )` of expressions, lambdas first.
    pub const GROUP: u8 = 69;
    /// `( … )` of `line_of_code_in`: `checked (`, `while (`, `switch (`.
    pub const GROUP_IN: u8 = 70;
    /// `{ … }` of statements, its `}` plain.
    pub const CODE_BRACE: u8 = 71;
    /// The same, its braces in `meta.block` (`fixed`'s and `lock`'s).
    pub const EB_BRACE: u8 = 72;
    pub const N_GENERIC_CALL: u8 = 73;
    pub const NAMEOF: u8 = 74;
    pub const TYPEOF: u8 = 75;
    pub const CHECKED: u8 = 76;
    pub const UNSAFE: u8 = 77;
    pub const LOC_GENERIC: u8 = 78;
    pub const LOC_AFTER_GENERIC: u8 = 79;
    pub const NESTED_METHOD: u8 = 80;
    pub const TYPE_NO_SPACE: u8 = 81;
    pub const TYPE_ARG: u8 = 82;
    pub const TUPLE_T: u8 = 83;
    pub const VDE: u8 = 84;
    pub const VDE_GENERIC: u8 = 85;
    pub const VDE_NAME: u8 = 86;
    pub const VAR_DECL: u8 = 87;
    pub const VD_TUPLE: u8 = 88;
    pub const FOR_BLOCK: u8 = 89;
    pub const FOR_AFTER: u8 = 90;
    pub const FOR_VAR_ASSIGN: u8 = 91;
    pub const FOREACH_VAR_ASSIGN: u8 = 92;
    pub const IF_COND: u8 = 93;
    pub const IF_GROUP: u8 = 94;
    pub const IF_BLOCK: u8 = 95;
    pub const IF_LINE: u8 = 96;
    pub const ELSE_BLOCK: u8 = 97;
    pub const ELSE_BODY: u8 = 98;
    pub const SWITCH_BLOCK: u8 = 99;
    pub const SW_BRACE: u8 = 100;
    pub const SW_CASE: u8 = 101;
    pub const TRY_BLOCK: u8 = 102;
    pub const TRY_LINE: u8 = 103;
    pub const CATCH_EXPR: u8 = 104;
    pub const CATCH_BLOCK: u8 = 105;
    pub const CATCH_WHEN: u8 = 106;
    pub const CW_OPEN: u8 = 107;
    pub const CW_GROUP: u8 = 108;
    pub const FINALLY_BLOCK: u8 = 109;
    pub const TRYCATCH_BLOCK: u8 = 110;
    pub const USING_BLOCK: u8 = 111;
    pub const UB_AFTER: u8 = 112;
    pub const EXPR_BLOCK: u8 = 113;
    pub const EB_AFTER: u8 = 114;
    pub const WHILE_BLOCK: u8 = 115;
    pub const DO_COND: u8 = 116;
    pub const WHILE_COND: u8 = 117;
    // the preprocessor's
    pub const PREPROC: u8 = 130;
    pub const PP_IF: u8 = 131;
    pub const OPTION_DONE: u8 = 132;
    pub const PP_CHECKSUM: u8 = 133;
    pub const PP_HASH: u8 = 134;
    pub const PP_WARNING: u8 = 135;
    // comments
    pub const DOC: u8 = 140;
    pub const DOC_TAG: u8 = 141;
    pub const LINE_COMMENT: u8 = 142;
    pub const BLOCK_COMMENT: u8 = 143;
    // strings
    pub const STRING: u8 = 150;
    pub const LONG_STRING: u8 = 151;
    pub const FORMAT_STRING: u8 = 152;
    pub const LONG_FORMAT_STRING: u8 = 153;
    pub const S_PH: u8 = 154;
    pub const R_PH: u8 = 155;
    /// A format spec: in a string's placeholder, a verbatim string's, an interpolation's, a
    /// verbatim interpolation's.
    pub const FS_S: u8 = 156;
    pub const FS_R: u8 = 157;
    pub const FS_I: u8 = 158;
    pub const FS_RI: u8 = 159;
    pub const INTERP: u8 = 160;
    pub const LINTERP: u8 = 161;
    /// An interpolated string (verbatim) whose scopes are gone: a `{` in its interpolation
    /// `set` that away, and syntect's set restores the scopes the interpolation cleared only
    /// for its token, then pops those two in place of the interpolation's own.
    pub const FORMAT_STRING_LOST: u8 = 162;
    pub const LONG_FORMAT_STRING_LOST: u8 = 163;
    /// A regex in the string below (ADR-40, phase 4): from here, plus the regex's state.
    pub const REGEX: u8 = 192;
    const _: () = assert!(LONG_FORMAT_STRING_LOST < REGEX);
    const _: () = assert!(REGEX as usize + crate::regex::NET_STATE as usize <= 255);
}

use c::*;

/// Bits of a kind past its leaf (bits 0–8).
const LEAF: u16 = 0x1ff;
/// The leaves of a regex in a string: the regex's kinds from here.
const RE_LEAF: u16 = 256;
const _: () = assert!(Leaf::Count as u16 <= RE_LEAF);
const _: () = assert!(RE_LEAF as usize + crate::regex::KINDS <= LEAF as usize + 1);
/// The string around (`chain_scopes`), bits 9–11.
const CHAIN: u16 = 7 << 9;
const CH_S: u16 = 1 << 9;
const CH_S_PH: u16 = 2 << 9;
const CH_S_PH_FS: u16 = 3 << 9;
const CH_R: u16 = 4 << 9;
const CH_R_PH: u16 = 5 << 9;
const CH_R_PH_FS: u16 = 6 << 9;
const CH_FS: u16 = 7 << 9;
/// A meta content scope around that colors, bits 12–13.
const WRAP: u16 = 3 << 12;
const W_DOC: u16 = 1 << 12;
const W_NS: u16 = 2 << 12;
/// `meta.preprocessor` inside the string around (a preprocessor line in a verbatim string's
/// placeholder, which the prototype reaches), not outside it as a string in `#pragma checksum`.
const W_PRE_IN: u16 = 3 << 12;
/// In `meta.annotation`.
const ANN: u16 = 1 << 14;
/// In `meta.preprocessor`.
const PRE: u16 = 1 << 15;

macro_rules! leaves {
    ($($name:ident = $s:literal,)*) => {
        /// The leaves: a token's scopes, bat's.
        static LEAVES: [&str; Leaf::Count as usize] = [$($s,)*];
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[repr(u16)]
        enum Leaf { $($name,)* Count }
        $(#[allow(dead_code)] pub(crate) const $name: u16 = Leaf::$name as u16;)*
    };
}

leaves! {
    PLAIN = "",
    // comments
    LC_TEXT = "comment.line.double-slash.cs",
    LC_MARK = "comment.line.double-slash.cs punctuation.definition.comment.cs",
    LC_TODO = "comment.line.double-slash.cs comment.line.todo.cs",
    LC_LINK = "comment.line.double-slash.cs markup.underline.link.cs",
    BC_TEXT = "comment.block.cs",
    BC_BEGIN = "comment.block.cs punctuation.definition.comment.begin.cs",
    BC_END = "comment.block.cs punctuation.definition.comment.end.cs",
    BC_STAR = "comment.block.cs punctuation.definition.comment.cs",
    BC_TODO = "comment.block.cs comment.line.todo.cs",
    BC_LINK = "comment.block.cs markup.underline.link.cs",
    DOC_MARK = "comment.block.documentation.cs punctuation.definition.comment.documentation.cs",
    DOC_PUNCT = "punctuation.definition.comment.documentation.cs",
    DOC_TEXT = "text.documentation.cs",
    TODO = "comment.line.todo.cs",
    LINK = "markup.underline.link.cs",
    TAG_BEGIN = "punctuation.definition.tag.begin.cs",
    TAG_END = "punctuation.definition.tag.end.cs",
    TAG_NAME_BEGIN = "entity.name.tag.begin.cs",
    TAG_NAME_END = "entity.name.tag.end.cs",
    ATTRIBUTE_NAME = "entity.other.attribute-name.cs",
    ARG_VALUE_SEP = "punctuation.separator.argument.value.cs",
    DQ = "string.quoted.double.cs",
    // strings (under their chain)
    STRING_BEGIN = "punctuation.definition.string.begin.cs",
    STRING_END = "punctuation.definition.string.end.cs",
    ESCAPE = "constant.character.escape.cs",
    LONE_ESCAPE = "invalid.illegal.lone-escape.cs",
    UNCLOSED_STRING = "invalid.illegal.unclosed-string.cs",
    GUID = "constant.other.guid.cs",
    PH_BEGIN = "punctuation.definition.placeholder.begin.cs",
    PH_BEGIN_P = "constant.other.placeholder.cs punctuation.definition.placeholder.begin.cs",
    PH_END = "punctuation.definition.placeholder.end.cs",
    DECIMAL = "constant.numeric.integer.decimal.cs",
    PH_UNCLOSED_NUM = "constant.other.placeholder.cs constant.numeric.integer.decimal.cs invalid.illegal.unclosed-string-placeholder.cs",
    UNESCAPED_PH = "invalid.illegal.unescaped-placeholder.cs",
    UNCLOSED_PH = "invalid.illegal.unclosed-string-placeholder.cs",
    UNEXPECTED_IN_PH = "invalid.illegal.unexpected-character-in-placeholder.cs",
    ARGS_SEP = "punctuation.separator.arguments.cs",
    FORMATTING = "constant.numeric.integer.decimal.formatting.cs",
    SEPARATOR = "punctuation.separator.cs",
    INTERP_BEGIN = "punctuation.section.interpolation.begin.cs",
    INTERP_END = "punctuation.section.interpolation.end.cs",
    // the preprocessor
    PP_HASH_MARK = "keyword.other.preprocessor.cs punctuation.definition.preprocessor.cs",
    KW_OTHER_PP = "keyword.other.preprocessor.cs",
    KW_CONTROL_PP = "keyword.control.preprocessor.cs",
    ENTITY_CONSTANT = "entity.name.constant.cs",
    PARENS = "punctuation.section.parens.cs",
    LOGICAL = "keyword.operator.logical.cs",
    LANGUAGE = "constant.language.cs",
    FLAG = "constant.other.flag.cs",
    INVALID = "invalid.illegal.cs",
    UNQUOTED = "string.unquoted.cs",
    SECTION = "entity.name.section.cs",
    SECTION_END = "variable.other.section.cs",
    PP_LINE_STRING = "string.quoted.double.cs",
    PP_LINE_STRING_BEGIN = "string.quoted.double.cs punctuation.definition.string.begin.cs",
    PP_LINE_STRING_END = "string.quoted.double.cs punctuation.definition.string.end.cs",
    HASH_TEXT = "string.quoted.double.hash.cs",
    HASH_BEGIN = "string.quoted.double.hash.cs punctuation.definition.string.begin.cs",
    HASH_END = "string.quoted.double.hash.cs punctuation.definition.string.end.cs",
    HASH_HEX = "string.quoted.double.hash.cs constant.numeric.integer.hexadecimal.cs",
    HASH_INVALID = "string.quoted.double.hash.cs invalid.illegal.cs",
    WARNING_NAME = "string.unquoted.warning.cs",
    // code
    STRAY = "invalid.illegal.stray.brace.cs",
    BLOCK_BEGIN = "punctuation.section.block.begin.cs",
    BLOCK_END = "punctuation.section.block.end.cs",
    M_BLOCK_BEGIN = "meta.block.cs punctuation.section.block.begin.cs",
    M_BLOCK_END = "meta.block.cs punctuation.section.block.end.cs",
    ANNOTATION_BEGIN = "punctuation.definition.annotation.begin.cs",
    ANNOTATION_END = "punctuation.definition.annotation.end.cs",
    TARGET = "storage.modifier.target.cs",
    KEY_VALUE = "punctuation.separator.key-value.cs",
    IMPORT = "keyword.control.import.cs",
    MODIFIER = "storage.modifier.cs",
    ACCESS = "storage.modifier.access.cs",
    CLASS_KW = "storage.type.class.cs",
    CLASS_NAME = "entity.name.class.cs",
    STRUCT_KW = "storage.type.struct.cs",
    STRUCT_NAME = "entity.name.struct.cs",
    ENUM = "meta.enum.cs",
    ENUM_KW = "meta.enum.cs storage.type.enum.cs",
    ENUM_NAME = "meta.enum.cs entity.name.enum.cs",
    ENUM_SEP = "meta.enum.cs punctuation.separator.key-value.type.cs",
    ENUM_TYPE = "meta.enum.cs storage.type.cs",
    ENUM_CONSTANT = "constant.other.enum.cs",
    ENUM_COMMA = "punctuation.separator.enum.cs",
    TERMINATOR = "punctuation.terminator.cs",
    INTERFACE_KW = "storage.type.interface.cs",
    INTERFACE_NAME = "entity.name.interface.cs",
    NAMESPACE_KW = "storage.type.namespace.cs",
    NS_SEP = "punctuation.separator.namespace.cs",
    DELEGATE_KW = "storage.type.delegate.cs",
    DELEGATE = "meta.delegate.cs",
    DELEGATE_NAME_LEAF = "meta.delegate.cs variable.other.member.delegate.cs",
    DELEGATE_GENERIC_BEGIN = "meta.delegate.cs meta.generic.cs punctuation.definition.generic.begin.cs",
    DELEGATE_GENERIC_END = "meta.delegate.cs meta.generic.cs punctuation.definition.generic.end.cs",
    DELEGATE_PARAMS_BEGIN = "meta.delegate.parameters.cs punctuation.section.parameters.begin.cs",
    PARAMS_END = "punctuation.section.parameters.end.cs",
    EXPECTED_COLON = "invalid.illegal.expected.colon",
    ILLEGAL = "invalid.illegal",
    GENERIC_BEGIN = "meta.generic.cs punctuation.definition.generic.begin.cs",
    GENERIC_END = "meta.generic.cs punctuation.definition.generic.end.cs",
    TYPE_SEP = "punctuation.separator.type.cs",
    INHERITED = "entity.other.inherited-class.cs",
    STORAGE_TYPE = "storage.type.cs",
    SUPPORT_TYPE = "support.type.cs",
    SUPPORT_NAMESPACE = "support.namespace.cs",
    PATH = "meta.path.cs",
    DOUBLE_COLON = "punctuation.accessor.double-colon.namespace.cs",
    NEW_OP = "keyword.operator.new.cs",
    DOT_NAMESPACE = "punctuation.accessor.dot.namespace.cs",
    INHERITED_SEP = "punctuation.separator.inherited-class.cs",
    MEMBER = "variable.other.member.cs",
    DOT = "punctuation.accessor.dot.cs",
    ADD = "storage.type.function.accessor.add.cs",
    REMOVE = "storage.type.function.accessor.remove.cs",
    DTOR = "meta.method.cs entity.name.function.destructor.cs",
    METHOD = "meta.method.cs",
    PARAMS_BEGIN = "meta.method.parameters.cs punctuation.section.parameters.begin.cs",
    CTOR = "entity.name.function.constructor.cs",
    NULLABLE = "storage.type.nullable.cs",
    BRACKETS_BEGIN = "punctuation.section.brackets.begin.cs",
    BRACKETS_END = "punctuation.section.brackets.end.cs",
    POINTER = "keyword.operator.pointer.cs",
    MODIFIER_OTHER = "storage.modifier.other.cs",
    METHOD_FUNCTION = "meta.method.cs entity.name.function.cs",
    M_GET = "storage.type.function.accessor.get.cs",
    M_GET_ARROW = "meta.method.cs storage.type.function.accessor.get.cs",
    BRACKETS_SEP = "meta.brackets.cs punctuation.separator.cs",
    FC_FUNCTION = "meta.function-call.cs variable.function.cs",
    FC_GENERIC_BEGIN = "meta.function-call.cs meta.generic.cs punctuation.definition.generic.begin.cs",
    FC_GENERIC_END = "meta.function-call.cs meta.generic.cs punctuation.definition.generic.end.cs",
    FC_GROUP_BEGIN = "meta.function-call.cs meta.group.cs punctuation.section.group.begin.cs",
    TERM_STMT = "punctuation.terminator.statement.cs",
    METHOD_GENERIC = "meta.method.cs meta.generic.cs",
    SECTION_GENERIC_BEGIN = "meta.method.cs meta.generic.cs punctuation.section.generic.begin.cs",
    GENERIC_END_P = "meta.method.cs meta.generic.cs punctuation.definition.generic.end.cs",
    DOT_P = "meta.method.cs meta.generic.cs punctuation.accessor.dot.cs",
    PARAM_TYPE_SEP = "punctuation.separator.parameter.type.cs",
    ENTITY_FUNCTION = "entity.name.function.cs",
    VAR_LANGUAGE = "variable.language.cs",
    M_BRACKETS_BEGIN = "meta.brackets.cs punctuation.section.brackets.begin.cs",
    M_BRACKETS_END = "meta.brackets.cs punctuation.section.brackets.end.cs",
    INHERITED_M = "meta.method.cs entity.other.inherited-class.cs",
    DOT_M = "meta.method.cs punctuation.accessor.dot.cs",
    PROPERTY_MEMBER = "meta.property.cs variable.other.member.cs",
    VARIABLES_SEP = "punctuation.separator.variables.cs",
    PARAMS_END_M = "meta.method.parameters.cs punctuation.section.parameters.end.cs",
    PARAMETER = "variable.parameter.cs",
    DEFAULT_VALUE = "keyword.operator.assignment.default-value.cs",
    PARAMETER_SEP = "punctuation.separator.parameter.function.cs",
    PARAM_MODIFIER = "storage.modifier.parameter.cs",
    PREBODY_SEP = "meta.method.constructor.prebody.cs punctuation.separator.function.cs",
    PREBODY = "meta.method.constructor.prebody.cs",
    GROUP_BEGIN = "punctuation.section.group.begin.cs",
    GROUP_END = "punctuation.section.group.end.cs",
    M_GROUP_BEGIN = "meta.group.cs punctuation.section.group.begin.cs",
    M_GROUP_END = "meta.group.cs punctuation.section.group.end.cs",
    FUNCTION_ARROW = "storage.type.function.cs",
    SET_KW = "storage.type.function.accessor.set.cs",
    ASSIGNMENT = "keyword.operator.assignment.cs",
    VARIABLE_ANNOTATION = "variable.annotation.cs",
    NAMESPACE_VAR = "variable.other.namespace.cs",
    ANNOTATION_SEP = "punctuation.separator.annotation.cs",
    IF_KW = "keyword.control.conditional.if.cs",
    SWITCH_KW = "keyword.control.flow.switch.cs",
    FOR_KW = "keyword.control.loop.for.cs",
    FOREACH_KW = "keyword.control.loop.foreach.cs",
    TRY_KW = "keyword.control.trycatch.try.cs",
    USING_KW = "keyword.control.using.cs",
    FIXED_KW = "keyword.control.other.fixed.cs",
    LOCK_KW = "keyword.control.other.lock.cs",
    DO_KW = "keyword.control.loop.do.cs",
    WHILE_KW = "keyword.control.loop.while.cs",
    UNEXPECTED_KW = "keyword.control.cs invalid.illegal.unexpected.keyword.cs",
    RETURN_KW = "keyword.control.flow.return.cs",
    BREAK_KW = "keyword.control.flow.break.cs",
    THROW_KW = "keyword.control.trycatch.throw.cs",
    GOTO_KW = "keyword.control.flow.goto.cs",
    CASE_KW = "keyword.control.switch.case.cs",
    LABEL_CONSTANT = "constant.other.label.cs",
    FUNCTION_CALL = "meta.function-call.cs",
    VAR_FUNCTION = "variable.function.cs",
    VAR_KW = "storage.type.variable.cs",
    ASSIGN_VARIABLE = "keyword.operator.assignment.variable.cs",
    VAR_OTHER = "variable.other.cs",
    LABEL = "entity.name.label.cs",
    LAMBDA = "storage.type.function.lambda.cs",
    LAMBDA_GROUP_BEGIN = "meta.function.anonymous.cs meta.group.cs punctuation.section.group.begin.cs",
    AWAIT = "keyword.control.other.cs",
    KW_OTHER = "keyword.other.cs",
    REFLECTION = "keyword.operator.reflection.cs",
    EXPECTED_BLOCK = "invalid.illegal.expected.block.cs",
    INSTANCE_NEW = "meta.instance.cs keyword.operator.new.cs",
    CLASS_SEP = "punctuation.separator.class.cs",
    ARGUMENT_SEP = "punctuation.separator.argument.cs",
    INSTANCE = "meta.instance.cs",
    INSTANCE_BRACES_BEGIN = "meta.instance.cs meta.braces.cs punctuation.section.braces.begin.cs",
    INSTANCE_GROUP = "meta.instance.cs meta.group.cs",
    INSTANCE_GROUP_BEGIN = "meta.instance.cs meta.group.cs punctuation.section.group.begin.cs",
    INSTANCE_GROUP_END = "meta.instance.cs meta.group.cs punctuation.section.group.end.cs",
    VALUE = "variable.language.cs",
    SQL = "keyword.other.sql.cs",
    NULL_COALESCING = "punctuation.accessor.null-coalescing.cs",
    ARROW = "punctuation.accessor.arrow.cs",
    OPERATOR = "keyword.operator.cs",
    RESERVED_CHAR = "invalid.illegal.reserved-char.cs",
    TERNARY_OP = "keyword.operator.ternary.cs",
    CAST_BRACKETS_SEP = "punctuation.separator.cs punctuation.section.brackets.end.cs",
    CAST_SEP = "punctuation.separator.cs",
    CAST_POINTER = "punctuation.separator.cs keyword.operator.pointer.cs",
    TUPLE_SEP = "punctuation.separator.tuple.cs",
    DISCARD = "variable.language.deconstruction.discard.cs",
    EXPECTED_CLOSE_PAREN = "invalid.illegal.expected-close-paren.cs",
    ARG_MODIFIER = "storage.modifier.argument.cs",
    PARAMETER_OTHER = "variable.other.parameter.cs",
    KEY_VALUE_PARAMETER = "punctuation.separator.key-value.parameter.cs",
    ACCESSOR_SEP = "punctuation.separator.accessor.cs",
    STORAGE_TYPE_OTHER = "storage.type.other.cs",
    BRACES_END_I = "meta.instance.cs meta.braces.cs punctuation.section.braces.end.cs",
    BRACES_BEGIN = "punctuation.section.braces.begin.cs",
    ARRAY_SEP = "punctuation.separator.array-element.cs",
    CHAR_LEAF = "constant.character.cs",
    CHAR_ESCAPE = "constant.character.cs constant.character.escape.cs",
    NOT_A_CHAR = "invalid.illegal.not_a_char.cs",
    HEX = "constant.numeric.integer.hexadecimal.cs",
    HEX_BASE = "constant.numeric.integer.hexadecimal.cs punctuation.definition.numeric.base.cs",
    BIN = "constant.numeric.integer.binary.cs",
    BIN_BASE = "constant.numeric.integer.binary.cs punctuation.definition.numeric.base.cs",
    FLOAT = "constant.numeric.float.decimal.cs",
    FLOAT_DOT = "constant.numeric.float.decimal.cs punctuation.separator.decimal.cs",
    FLOAT_SUFFIX = "constant.numeric.float.decimal.cs storage.type.numeric.cs",
    DEC_SUFFIX = "constant.numeric.integer.decimal.cs storage.type.numeric.cs",
    TUPLE_GROUP_BEGIN = "meta.group.tuple.cs punctuation.definition.group.begin.cs",
    TUPLE_GROUP_END = "meta.group.tuple.cs punctuation.definition.group.end.cs",
    LOOP_ASSIGN = "keyword.operator.assignment.variable.loop.cs",
    FLOW = "keyword.control.flow.cs",
    ELSEIF_KW = "keyword.control.conditional.elseif.cs",
    ELSE_KW = "keyword.control.conditional.else.cs",
    CASE_SEP = "punctuation.separator.case-statement.cs",
    CONSTANT_OTHER = "constant.other.cs",
    WHEN_KW = "keyword.control.switch.case.when.cs",
    CATCH_KW = "keyword.control.trycatch.catch.cs",
    TRYCATCH_WHEN = "keyword.control.trycatch.when.cs",
    FINALLY_KW = "keyword.control.trycatch.finally.cs",
    EXPECTED_NAMESPACE = "invalid.illegal.expected-namespace.cs",
}

/// The scopes of the string a kind is in, outermost first.
fn chain_scopes(kind: u16) -> &'static [&'static str] {
    const S: &str = "string.quoted.double.cs";
    const R: &str = "string.quoted.double.raw.cs";
    const PH: &str = "constant.other.placeholder.cs";
    const FS: &str = "constant.other.format-spec.cs";
    match kind & CHAIN {
        CH_S => &[S],
        CH_S_PH => &[S, PH],
        CH_S_PH_FS => &[S, PH, FS],
        CH_R => &[R],
        CH_R_PH => &[R, PH],
        CH_R_PH_FS => &[R, PH, FS],
        CH_FS => &[FS],
        _ => &[],
    }
}

/// The scopes under `source.cs` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    let mut out = Vec::new();
    if kind & ANN != 0 {
        out.push("meta.annotation.cs");
    }
    let pre_in = kind & WRAP == W_PRE_IN;
    if kind & PRE != 0 && !pre_in {
        out.push("meta.preprocessor.cs");
    }
    // a doc comment, a namespace's name or a preprocessor line can be in a string's
    // placeholder (the prototype reaches a verbatim string's), never the other way round
    out.extend_from_slice(chain_scopes(kind));
    match kind & WRAP {
        W_DOC => out.push("comment.block.documentation.cs"),
        W_NS => out.push("entity.name.namespace.cs"),
        W_PRE_IN => out.push("meta.preprocessor.cs"),
        _ => {}
    }
    let leaf = kind & LEAF;
    if leaf >= RE_LEAF {
        out.extend(crate::regex::scopes(leaf - RE_LEAF));
    } else if let Some(leaf) = LEAVES.get(usize::from(leaf)) {
        out.extend(leaf.split(' ').filter(|s| !s.is_empty()));
    }
    out
}

/// What context `ctx` adds to the bits of the level under it (`under`): the string it is in,
/// the meta content scope around, `meta.annotation`, `meta.preprocessor`.
const fn level_bits(under: u16, ctx: u8) -> u16 {
    let chain = match ctx {
        STRING | FORMAT_STRING => CH_S,
        LONG_STRING | LONG_FORMAT_STRING => CH_R,
        S_PH => CH_S_PH,
        R_PH => CH_R_PH,
        FS_S => CH_S_PH_FS,
        FS_R => CH_R_PH_FS,
        FS_I | FS_RI => CH_FS,
        // `clear_scopes: 2` takes the interpolated string's two scopes
        INTERP | LINTERP => 0,
        _ => under & CHAIN,
    };
    let wrap = match ctx {
        DOC => W_DOC,
        NS_NAME => W_NS,
        _ => under & WRAP,
    };
    let ann = if matches!(ctx, ATTR_IN | ATTR_ARGS) {
        ANN
    } else {
        under & ANN
    };
    let (pre, wrap) = match ctx {
        PREPROC if under & PRE != 0 => (PRE, wrap),
        PREPROC if under & CHAIN != 0 => (PRE, W_PRE_IN),
        PREPROC => (PRE, wrap),
        // a string in that preprocessor line: its own chain, inside `meta.preprocessor`
        STRING if wrap == W_PRE_IN => (PRE, 0),
        _ => (under & PRE, wrap),
    };
    chain | wrap | ann | pre
}

/// The leaf of the text no pattern of context `ctx` matches.
const fn text_leaf(ctx: u8) -> u16 {
    match ctx {
        LINE_COMMENT => LC_TEXT,
        BLOCK_COMMENT => BC_TEXT,
        PP_HASH => HASH_TEXT,
        _ => PLAIN,
    }
}

/// Whether context `ctx` tries the prototype (comments, a preprocessor line) first: not the
/// strings and doc comments that opt out, nor (syntect's rule) any context the prototype
/// reaches: its comments, the preprocessor's contexts, and through `#pragma checksum` a
/// string with its placeholders and format specs (shared with interpolations).
const fn has_proto(ctx: u8) -> bool {
    !matches!(
        ctx,
        STRING
            | LONG_STRING
            | FORMAT_STRING
            | LONG_FORMAT_STRING
            | FORMAT_STRING_LOST
            | LONG_FORMAT_STRING_LOST
            | DOC
            | DOC_TAG
            | LINE_COMMENT
            | BLOCK_COMMENT
            | PREPROC
            | PP_IF
            | OPTION_DONE
            | PP_CHECKSUM
            | PP_HASH
            | PP_WARNING
            | S_PH
            | FS_S
            | FS_I
            | REGEX..
    )
}

/// Whether no pattern of context `ctx` can match at a space, but the prototype's at the line's
/// start (tried there before any step): a run of spaces is skipped whole. In those that
/// include `line_of_code_in_no_semicolon`, its `\s*(\??\[)` can start at a space, but
/// matches at the `[` the same; so can `\s*(\))` in an `if`'s condition.
const fn spaces_inert(ctx: u8) -> bool {
    matches!(
        ctx,
        MAIN | MAIN_BRACE
            | MAIN_BRACE_M
            | MAIN_LINE
            | DTB_BLOCK
            | MB_BLOCK
            | CODE_BRACE
            | EB_BRACE
            | SW_BRACE
            | LINE
            | LINE_IN
            | NOSEMI
            | ARG_EXPR
            | ARGUMENTS
            | GROUP
            | GROUP_IN
            | TERNARY
            | LAMBDA_BODY
            | ACCESSOR_ARGS
            | MV_VALUE
            | VD_VALUE
            | ANON_CLASS
            | IF_GROUP
            | CW_GROUP
            | TUPLE
            | ENUM_BODY
            | INIT_CTOR
            // the contexts that wait for a block or a keyword after a statement's head
            | IF_BLOCK
            | IF_LINE
            | ELSE_BLOCK
            | ELSE_BODY
            | SWITCH_BLOCK
            | TRY_BLOCK
            | CATCH_EXPR
            | CATCH_BLOCK
            | CATCH_WHEN
            | CW_OPEN
            | FINALLY_BLOCK
            | TRYCATCH_BLOCK
            | USING_BLOCK
            | UB_AFTER
            | EXPR_BLOCK
            | EB_AFTER
            | FOR_BLOCK
            | FOR_AFTER
            | FOR_VAR_ASSIGN
            | FOREACH_VAR_ASSIGN
            | WHILE_BLOCK
            | DO_COND
            | WHILE_COND
            | CHECKED
            | UNSAFE
            | NEW
            | NEW_BRACKETS
            | CTOR_ARGS
            // declarations
            | NS_DECL
            | ENUM_HEAD
            | ENUM_END
            | DATA_TYPE_SIG
            | TYPE_CONSTRAINT
            | EVENT_HANDLER
            | EH_AFTER
            | EH_BLOCK
            | METHOD_BODY
            | METHOD_ACCESSOR
            | ACC_BLOCK
            | ACC_AFTER
            | MEMBER_VARS
            | VARS_DECL
            | METHOD_PARAMS
            | METHOD_PARAMS_BRACKET
            | METHOD_PARAM
            | METHOD_PARAM_TYPE
            | CTOR_PREBODY
            | CTOR_INIT_AFTER
            | ATTR_IN
            | LAMBDA_PARAMS
            // types, whose `\s*(\.)`, `\s*(\?)` and `\s*(<)` match at the next byte the same
            | GEN_TYPES
            | TYPE_ARG
            | TYPEOF
            | NAMEOF
            | N_GENERIC_CALL
            | MN_EXPLICIT
            | LOC_GENERIC
            | VDE_GENERIC
            | DTS_GENERIC
            | MN_GENERIC
            | DN_GENERIC
            | TUPLE_T
            | VDE
            | VDE_NAME
            | VAR_DECL
            | VD_TUPLE
            // a member's name, whose `\s*(\()` and `(?=\s*\{)` match at the next byte the same
            | METHOD_NAME
    )
}

/// The bytes a pattern of a context starts with, as a table for [`Scan::advance`].
const fn stops(bytes: &[u8]) -> [u8; 256] {
    let mut t = [0; 256];
    let mut i = 0;
    while i < bytes.len() {
        t[bytes[i] as usize] = 1;
        i += 1;
    }
    t
}

/// A line comment's: a to-do (`todo`, `hack`, `xxx`, any case), `/!\`, a link.
static LINE_COMMENT_STOPS: [u8; 256] = stops(b"tThHxX/");
/// A block comment's: those, and `*/`.
static BLOCK_COMMENT_STOPS: [u8; 256] = stops(b"tThHxX/*");
/// A string's and an interpolated one's: escapes, quotes, braces.
static STRING_STOPS: [u8; 256] = stops(b"\\\"{}");
/// A verbatim string's: quotes and braces.
static LONG_STRING_STOPS: [u8; 256] = stops(b"\"{}");

/// How many contexts a line can hold; one pushed past it drops the bottom one.
const MAX: usize = 128;
/// How many the state keeps between lines: [`State`]'s levels and withs.
const KEPT: usize = 2 * HALF;
/// Matches that take nothing in a row at one place before the scan steps on.
const STUCK: usize = 64;
/// syntect's stack limit (with its two bottom contexts): a push at it ends the line.
const SYNTECT_DEPTH: usize = 99;

/// Lex `b`, a line of C# without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let mut s = Scan {
        b,
        n: b.len(),
        stack: [MAIN; MAX],
        bits: [0; MAX],
        depth: 1,
        exact: state.is_exact(),
        text: PLAIN,
        at: 0,
        step: 0,
        loop_at: (usize::MAX, 0),
        looped: false,
        halt: false,
        stuck: 0,
        wcache: std::cell::Cell::new((usize::MAX, 0)),
        ncache: std::cell::Cell::new((usize::MAX, None)),
        regex_calls: None,
        out,
    };
    let n = usize::from(state.depth);
    for i in 0..n {
        let c = if i < HALF {
            state.levels[i]
        } else {
            state.withs[i - HALF]
        };
        s.stack[1 + i] = c;
        s.bits[1 + i] = level_bits(s.bits[i], c);
    }
    s.depth = 1 + n;
    s.refresh();
    s.run();
    s.fill(s.n);
    let n = s.depth - 1;
    let from = n.saturating_sub(KEPT);
    let mut packed = State {
        depth: (n - from) as u8,
        inexact: !s.exact || from > 0,
        ..State::default()
    };
    for (i, &c) in s.stack[1 + from..s.depth].iter().enumerate() {
        if i < HALF {
            packed.levels[i] = c;
        } else {
            packed.withs[i - HALF] = c;
        }
    }
    *state = packed;
}

pub(crate) struct Scan<'b, 'c> {
    b: &'b [u8],
    /// The line's length: where its `\n` is.
    n: usize,
    stack: [u8; MAX],
    /// Each level's [`level_bits`].
    bits: [u16; MAX],
    depth: usize,
    exact: bool,
    /// The kind of text no pattern of the top context matches.
    text: u16,
    /// Where the spans so far end.
    at: usize,
    /// Where the step started: a pattern that matches nothing with nothing to do counts only
    /// there (syntect's search from it gives none for the step).
    step: usize,
    /// Where the last push that matched nothing was, and the depth its first context left: a
    /// pop there, at that depth, that matches nothing would loop, so syntect tries the patterns
    /// after it and else steps a character.
    loop_at: (usize, usize),
    /// A pop at the place was passed over (`loop_at`).
    looped: bool,
    /// Nothing more to match on the line.
    halt: bool,
    /// Matches that took nothing, in a row at one place: past syntect's guard, a step on.
    stuck: usize,
    /// The last run of `\w` looked up: where it starts and ends ([`Scan::word_end`]).
    wcache: std::cell::Cell<(usize, usize)>,
    /// The last place [`Scan::ascii_name`] looked at, and what it found.
    ncache: std::cell::Cell<(usize, Option<usize>)>,
    /// The line's regex calls, found at its first string ([`RegexCalls`]).
    regex_calls: Option<RegexCalls>,
    out: &'b mut Out<'c>,
}

/// The commas after the first argument of `Regex.IsMatch(` and the like on a line (up to
/// eight), found once a line, at its first string after a comma, so that a string's check
/// stays short.
pub(crate) struct RegexCalls {
    commas: [u32; 8],
    len: u8,
}

/// The static methods of `Regex` that take the regex second, after the input.
const REGEX_METHODS: [&[u8]; 8] = [
    b"IsMatch",
    b"Match",
    b"Matches",
    b"Replace",
    b"Split",
    b"Count",
    b"EnumerateMatches",
    b"EnumerateSplits",
];

/// Where a string's escape at `p` (a `\\` not before another) ends: `{{escaped_char}}`, or the
/// `\\` alone.
fn escape_end(b: &[u8], p: usize) -> usize {
    let hex = |q: usize| b.get(q).is_some_and(u8::is_ascii_hexdigit);
    match b.get(p + 1) {
        Some(b'u') if (p + 2..p + 6).all(hex) => p + 6,
        Some(b'U') if (p + 2..p + 10).all(hex) => p + 10,
        Some(b'x') if hex(p + 2) => (p + 3..p + 6).find(|&q| !hex(q)).unwrap_or(p + 6),
        Some(b'0'..=b'9') => (p + 2..p + 4)
            .find(|&q| !b.get(q).is_some_and(u8::is_ascii_digit))
            .unwrap_or(p + 4),
        Some(b'a' | b'b' | b'f' | b'n' | b'r' | b't' | b'v' | b'"' | b'\'') => p + 2,
        _ => p + 1,
    }
}

/// Whether `c` can be in a name.
fn word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

/// Where the blanks before `p` start.
fn blank_back(b: &[u8], p: usize) -> usize {
    let mut q = p;
    while q > 0 && matches!(b[q - 1], b' ' | b'\t') {
        q -= 1;
    }
    q
}

/// Whether `b[..w]` ends with the word `name`.
fn ends_word(b: &[u8], w: usize, name: &[u8]) -> bool {
    b[..w].ends_with(name) && (w == name.len() || !word_byte(b[w - name.len() - 1]))
}

impl RegexCalls {
    /// The line `b`'s.
    #[cold]
    fn find(b: &[u8]) -> Self {
        let mut calls = RegexCalls {
            commas: [0; 8],
            len: 0,
        };
        let mut i = 0;
        while usize::from(calls.len) < calls.commas.len()
            && let Some(at) = b[i..].iter().position(|&c| c == b'R')
        {
            let s = i + at;
            i = s + 1;
            if !b[s..].starts_with(b"Regex.") {
                continue;
            }
            i = s + 6;
            if s > 0 && word_byte(b[s - 1]) {
                continue;
            }
            let mut e = i;
            while e < b.len() && word_byte(b[e]) {
                e += 1;
            }
            if !REGEX_METHODS.contains(&&b[i..e]) {
                continue;
            }
            let mut o = e;
            while o < b.len() && matches!(b[o], b' ' | b'\t') {
                o += 1;
            }
            if b.get(o) == Some(&b'(')
                && let Some(c) = first_argument(b, o + 1)
            {
                calls.commas[usize::from(calls.len)] = c as u32;
                calls.len += 1;
            }
        }
        calls
    }

    fn has_comma(&self, c: usize) -> bool {
        self.commas[..usize::from(self.len)]
            .iter()
            .any(|&x| x as usize == c)
    }
}

/// The comma after a call's first argument, which starts at `s`: `None` if the call has one
/// argument, or its first runs past the line.
fn first_argument(b: &[u8], s: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = s;
    while i < b.len() {
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.checked_sub(1)?,
            b',' if depth == 0 => return Some(i),
            b'/' if b.get(i + 1) == Some(&b'/') => return None,
            b'"' => {
                // a verbatim string's `""`, any other's `\"`
                let verbatim = i > 0 && (b[i - 1] == b'@' || (i > 1 && b[i - 2] == b'@'));
                i += 1;
                loop {
                    match b.get(i)? {
                        b'"' if verbatim && b.get(i + 1) == Some(&b'"') => i += 1,
                        b'"' => break,
                        b'\\' if !verbatim => i += 1,
                        _ => {}
                    }
                    i += 1;
                }
            }
            b'\'' => {
                i += 1;
                loop {
                    match b.get(i)? {
                        b'\'' => break,
                        b'\\' => i += 1,
                        _ => {}
                    }
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Whether the string after `b[..q]`, its `(`, is a regex: the first argument of `new Regex(`
/// (`new System.Text.RegularExpressions.Regex(`), `[GeneratedRegex(` or
/// `[RegularExpression(`, looked back on its line. The second argument of `Regex.IsMatch(`,
/// `Regex.Replace(` and the like ([`REGEX_METHODS`]) is found by the line's [`RegexCalls`].
fn regex_first_argument(b: &[u8], q: usize) -> bool {
    let w = blank_back(b, q - 1);
    if b[..w].ends_with(b"Regex") {
        let mut s = w - 5;
        if s > 0 && word_byte(b[s - 1]) {
            return ends_word(b, w, b"GeneratedRegex");
        }
        // the namespace before it, then `new`
        while s > 0 && (b[s - 1] == b'.' || word_byte(b[s - 1])) {
            s -= 1;
        }
        let t = blank_back(b, s);
        return t < s && ends_word(b, t, b"new");
    }
    ends_word(b, w, b"RegularExpression")
        || ends_word(b, w, b"GeneratedRegexAttribute")
        || ends_word(b, w, b"RegularExpressionAttribute")
}

impl Scan<'_, '_> {
    fn top(&self) -> u8 {
        self.stack[self.depth - 1]
    }

    /// The bits of the top level.
    fn cur(&self) -> u16 {
        self.bits[self.depth - 1]
    }

    fn refresh(&mut self) {
        self.text = self.cur() | text_leaf(self.top());
    }

    /// The text up to `p` is the top context's.
    fn fill(&mut self, p: usize) {
        let p = p.min(self.n);
        if p > self.at {
            if self.text != PLAIN {
                self.out.push_merged(self.at, p, self.text);
            }
            self.at = p;
        }
    }

    /// Bytes `s..e` are `leaf` in the top context, after the text before them.
    fn put(&mut self, s: usize, e: usize, leaf: u16) {
        let k = self.cur() | leaf;
        self.put_kind(s, e, k);
    }

    /// Bytes `s..e` are kind `k` as it is.
    fn put_kind(&mut self, s: usize, e: usize, k: u16) {
        self.fill(s);
        let s = s.max(self.at);
        let e = e.min(self.n);
        if k != PLAIN && e > s {
            self.out.push_merged(s, e, k);
        }
        self.at = self.at.max(e);
    }

    fn push(&mut self, c: u8) {
        if self.depth == MAX {
            self.stack.copy_within(2.., 1);
            for i in 1..MAX - 1 {
                self.bits[i] = level_bits(self.bits[i - 1], self.stack[i]);
            }
            self.depth -= 1;
            self.exact = false;
        }
        self.stack[self.depth] = c;
        self.bits[self.depth] = level_bits(self.bits[self.depth - 1], c);
        self.depth += 1;
        self.refresh();
    }

    fn pop(&mut self) {
        if self.depth > 1 {
            self.depth -= 1;
        }
        self.refresh();
    }

    /// Replace the top with `cs`, bottom first.
    fn set(&mut self, cs: &[u8]) {
        self.pop();
        for &c in cs {
            self.push(c);
        }
    }

    /// Text before `p` is the top context's; then `cs` are pushed, bottom first, by a match
    /// that took nothing at `p`. syntect keeps the depth the first of them leaves, however
    /// many there are, for its guard against a loop.
    fn enter(&mut self, p: usize, cs: &[u8]) -> Option<usize> {
        if self.depth >= SYNTECT_DEPTH {
            self.halt = true;
            return Some(p);
        }
        self.fill(p);
        let depth = self.depth + 1;
        for &c in cs {
            self.push(c);
        }
        self.loop_at = (p, depth);
        Some(p)
    }

    /// Text before `p` is the top context's; then it is replaced with `cs`, by a match that
    /// took nothing.
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

    /// Bytes `s..e` are `leaf`; then `cs` are pushed (a consuming push: their meta scopes
    /// are on the bytes only through `leaf`, see [`Scan::open`]).
    fn tok_push(&mut self, s: usize, e: usize, leaf: u16, cs: &[u8]) -> Option<usize> {
        if self.depth >= SYNTECT_DEPTH {
            self.halt = true;
            return Some(s);
        }
        self.put(s, e, leaf);
        for &c in cs {
            self.push(c);
        }
        Some(e)
    }

    /// `cs` pushed at `s`, then bytes `s..e` are `leaf` in the top one: the pushed context's
    /// meta scope is on them (a string's, `meta.annotation`, `meta.preprocessor`).
    fn open(&mut self, s: usize, e: usize, leaf: u16, cs: &[u8]) -> Option<usize> {
        if self.depth >= SYNTECT_DEPTH {
            self.halt = true;
            return Some(s);
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
        let end = self.n + 1;
        let mut p = 0;
        loop {
            self.looped = false;
            if let Some(q) = self.at(p) {
                if self.halt {
                    return;
                }
                self.stuck = if q == p { self.stuck + 1 } else { 0 };
                p = q;
                self.step = q;
                if self.stuck > STUCK {
                    self.stuck = 0;
                    if p >= self.n {
                        return;
                    }
                    p = self.next(p);
                    self.step = p;
                }
            } else if p >= end || (self.looped && p >= self.n) {
                return;
            } else if self.looped {
                // syntect's guard: a step of a character, and a new search from there
                p = self.next(p);
                self.step = p;
            } else {
                p = self.advance(p);
            }
        }
    }

    /// Where a pattern of the top context can next start, after none did at `p`: in a comment
    /// or a string, the next byte one of its patterns starts with (or the line's end).
    fn advance(&self, p: usize) -> usize {
        let stops = match self.top() {
            LINE_COMMENT => &LINE_COMMENT_STOPS,
            BLOCK_COMMENT => &BLOCK_COMMENT_STOPS,
            STRING | FORMAT_STRING | FORMAT_STRING_LOST => &STRING_STOPS,
            LONG_STRING | LONG_FORMAT_STRING | LONG_FORMAT_STRING_LOST => &LONG_STRING_STOPS,
            top if spaces_inert(top) && p < self.n && re::space_len(&self.b[p..]).is_some() => {
                return self.sp_line(p);
            }
            _ => return self.next(p),
        };
        if p >= self.n {
            return p + 1;
        }
        let b = self.b;
        let mut q = p + 1;
        while q < self.n && stops[usize::from(b[q])] == 0 {
            q += 1;
        }
        q
    }

    /// Where the character at `p` ends (the line's `\n` is one byte).
    fn next(&self, p: usize) -> usize {
        if p >= self.n {
            return p + 1;
        }
        p + re::char_len(self.b[p]).min(self.n - p)
    }

    /// The first pattern of the top context that matches at `p` (the prototype's first),
    /// applied: where the scan goes on, or `None` when none does.
    fn at(&mut self, p: usize) -> Option<usize> {
        let top = self.top();
        // at a space, the line's end or past it, such a context has none but the prototype's
        // `^\s*` patterns, at the line's start
        if spaces_inert(top) && (p > self.n || self.space(p)) {
            return if p == 0 { self.proto(p) } else { None };
        }
        if has_proto(top)
            && let Some(q) = self.proto(p)
        {
            return Some(q);
        }
        self.context(top, p)
    }

    /// The prototype: a doc comment, a line or block comment, a preprocessor line.
    fn proto(&mut self, p: usize) -> Option<usize> {
        if let Some(q) = self.comments(p) {
            return Some(q);
        }
        let b = self.b;
        if p == 0 {
            let s = self.sp_line(0);
            if s < self.n && b[s] == b'#' {
                self.fill(0);
                self.push(PREPROC);
                self.put(0, s, PLAIN);
                self.put(s, s + 1, PP_HASH_MARK);
                let e = self.sp(s + 1);
                self.put(s + 1, e, KW_OTHER_PP);
                return Some(e);
            }
        }
        None
    }

    /// `comments`: a doc comment, a line or block comment.
    pub(super) fn comments(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if p == 0 {
            let s = self.sp_line(0);
            if b[s..].starts_with(b"///") {
                // the indentation is the context's; the marks open the doc comment
                self.put(s, s + 3, DOC_MARK);
                self.push(DOC);
                return Some(s + 3);
            }
        }
        if p + 1 < self.n && b[p] == b'/' {
            match b[p + 1] {
                b'/' => return self.open(p, p + 2, LC_MARK, &[LINE_COMMENT]),
                b'*' => return self.open(p, p + 2, BC_BEGIN, &[BLOCK_COMMENT]),
                _ => {}
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::Lang;
    use crate::tests::marked;

    #[test]
    fn a_class_and_its_method() {
        let m = marked(
            Lang::CSharp,
            &[
                "namespace A.B {",
                "  public class Foo : Bar {",
                "    public int Get(string s) { return s.Length; }",
                "  }",
                "}",
            ],
        );
        assert!(m.contains(&("Foo", "entity.name.class.cs")));
        assert!(m.contains(&("Get", "meta.method.cs entity.name.function.cs")));
        assert!(m.contains(&("return", "keyword.control.flow.return.cs")));
        assert!(m.contains(&("A", "entity.name.namespace.cs")));
    }

    #[test]
    fn an_interpolated_string() {
        let m = marked(Lang::CSharp, &["var s = $\"a {x:N2} b\";"]);
        assert!(m.contains(&("a ", "string.quoted.double.cs")));
        assert!(m.contains(&("x", "variable.other.cs")));
        assert!(m.contains(&("N2", "constant.other.format-spec.cs")));
    }

    #[test]
    fn a_preprocessor_line_and_an_attribute() {
        let m = marked(Lang::CSharp, &["#region Things", "[Obsolete(\"x\")]"]);
        assert!(m.contains(&("Things", "meta.preprocessor.cs entity.name.section.cs")));
        assert!(m.contains(&("Obsolete", "meta.annotation.cs variable.annotation.cs")));
    }

    #[test]
    fn regex_calls_stay_linear() {
        let lines = [
            "Regex.Match(a, \"(x\", ".repeat(5000),
            "Regex.Match(Regex.Match(".repeat(4000),
            format!("Regex.Split({}", "(".repeat(60000)),
            "new Regex(@\"[a-[b]]\", ".repeat(5000),
            "f(a, \"x\"), Regex".repeat(5000),
        ];
        let t = std::time::Instant::now();
        let (mut state, mut spans) = (crate::State::default(), Vec::new());
        for line in &lines {
            crate::lex(Lang::CSharp, line, &mut state, &mut spans);
        }
        assert!(t.elapsed().as_secs() < 2, "{:?}", t.elapsed());
    }

    #[test]
    fn regexes_in_strings() {
        const S: &str = "string.quoted.double.cs";
        const R: &str = "string.quoted.double.raw.cs";
        const A: &str = "meta.annotation.cs string.quoted.double.raw.cs";
        let lines = [
            r#"var r = new Regex("^(?<y>\\d{4})\t[]a-z-[aeiou]]{2,}$", RegexOptions.None);"#,
            r#"if (Regex.IsMatch(Get(a, b), @"(?'n'x)(?<-n>y)(?#note)\k'n'""q""", o)) {"#,
            r#"  var s = System.Text.RegularExpressions.Regex.Replace(s, @"(?(n)a|b)(?>c)*+", "$1");"#,
            r#"[GeneratedRegex(@"\p{IsLatin-1Supplement}\pL(?imnsx-i:z)")]"#,
            r#"var v = new Regex(@"multi"#,
            r#"(a)+", "{0}"); var w = myRegex.Replace(x, "{0}+");"#,
        ];
        let m = marked(Lang::CSharp, &lines);
        let has = |text: &str, scopes: &str| {
            assert!(
                m.iter().any(|&(t, s)| t == text && s == scopes),
                "{text:?} as {scopes:?} in {m:#?}"
            );
        };
        has("^", &format!("{S} keyword.control.anchor.regexp"));
        has("y", &format!("{S} entity.name.other.group.regexp"));
        has(
            r"\\d",
            &format!("{S} constant.other.character-class.escape.backslash.regexp"),
        );
        // the host's escape, and no placeholder: `{2,}` is a quantifier
        has(r"\t", &format!("{S} constant.character.escape.cs"));
        has(
            "2",
            &format!("{S} keyword.operator.quantifier.regexp constant.literal.numeric.regexp"),
        );
        // `]` first is a character; a subtraction
        has(
            "-",
            &format!(
                "{S} constant.other.character-class.set.regexp keyword.operator.difference.regexp"
            ),
        );
        // the second argument, a verbatim string: `""` is the host's
        has("n", &format!("{R} entity.name.other.group.regexp"));
        has("-", &format!("{R} keyword.operator.balancing-group.regexp"));
        has("note", &format!("{R} comment.block.regexp"));
        has(r"\k", &format!("{R} keyword.other.back-reference.regexp"));
        has(r#""""#, &format!("{R} constant.character.escape.cs"));
        has(
            "?(",
            &format!("{R} punctuation.definition.group.assertion.conditional.regexp"),
        );
        has(
            "?>",
            &format!("{R} punctuation.definition.group.atomic.regexp"),
        );
        // no possessive quantifier: `*`, then `+`
        has("*", &format!("{R} keyword.operator.quantifier.regexp"));
        has(
            "IsLatin-1Supplement",
            &format!("{A} support.constant.unicode-property.regexp"),
        );
        has(
            r"\p",
            &format!("{A} constant.character.escape.backslash.regexp"),
        );
        has(
            "?imnsx-i",
            &format!("{A} keyword.other.option-toggle.regexp"),
        );
        // across lines in a verbatim string
        has(
            "(",
            &format!("{R} punctuation.definition.group.begin.regexp"),
        );
        has("+", &format!("{R} keyword.operator.quantifier.regexp"));
        // not regexes: a third argument, an instance's call
        assert!(
            !m.iter().any(|&(t, s)| t == "$1" && s.contains("regexp")),
            "{m:#?}"
        );
        has(
            "0",
            &format!("{S} constant.other.placeholder.cs constant.numeric.integer.decimal.cs"),
        );
    }
}
