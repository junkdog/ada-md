//! C++ by a scanner of ada's own (ADR-40, speed first): bat's `C++` grammar compiled by hand,
//! as Rust's, Kotlin's and Java's were, with the 127 contexts of bat's `C` it reaches (comments,
//! strings, operators, the preprocessor's pieces, and C's whole expression grammar, which an
//! upper-case macro call on a line of its own hands its arguments to). Each context that can be
//! open is an arm of one `match` on the top of a stack, trying its patterns in the grammar's
//! order at the position the scan has reached, which is syntect's leftmost match; where no
//! pattern can start, the run is skipped whole.
//!
//! The grammar keeps meta scopes on nearly every context, but the built-in themes key on one of
//! them only, `meta.preprocessor`, so a kind is a leaf (bat's scopes for a token, without their
//! language's suffix) with bits for that meta scope, for the suffix (`.c` for what C's grammar
//! gives, `.c++` for C++'s) and for the scope of a context around it that colors (a disabled
//! branch's comment, an inherited class, a namespace's or a function's name). Contexts that
//! differ only in meta scopes are one here: every `( … )` of expressions is [`PAREN`], a
//! function's and a method's bodies [`FN_BLOCK`].
//!
//! The stack between lines is the contexts open above the top level, up to 28 (the bytes of
//! [`State`]'s levels); an open raw string keeps its delimiter in the top ones.

mod c;
mod cxx;
mod pp;
mod text;
mod words;

use crate::machine::KEPT as HALF;
use crate::{Out, State};
use text::next;

// The contexts. C++'s first, then C's from `C_FIRST`.
const MAIN: u8 = 0;
/// `#anon_global_0`: after `template` at the top level.
const G_TEMPLATE: u8 = 1;
/// `#anon_global_1`: after a top-level `/* … */`.
const G_COMMENT_AFTER: u8 = 2;
/// `extern "C"`, its block, and the block after `#ifdef __cplusplus` in it.
const G_EXTERN: u8 = 3;
const G_EXTERN_BLOCK: u8 = 4;
const G_EXTERN_BLOCK2: u8 = 5;
const G_MODIFIER: u8 = 6;
const G_TYPE: u8 = 7;
const G_MAYBE_FN: u8 = 8;
/// `#anon_global-maybe-function_0`: an operator's name.
const G_OPERATOR: u8 = 9;
/// `identifiers`, then a pop where none matches (`#anon_global-maybe-function_1`,
/// `#anon_data-structures-maybe-method_0`, `#anon_using-namespace_0`).
const IDENT_POP: u8 = 10;
/// `#anon_global-modifier_0`, `_1`, `_2`: a constructor's, a destructor's, a function's name
/// after a path.
const G_MOD_CTOR: u8 = 11;
const G_MOD_DTOR: u8 = 12;
const G_MOD_FN: u8 = 13;
/// `#anon_global-type_0`: after `struct X`, `identifiers`, then `global-maybe-function`.
const G_TYPE_STRUCT: u8 = 14;
/// `#anon_global-type_2`: `function-call`, then a pop.
const G_TYPE_CALL: u8 = 15;
const G_FN_ID: u8 = 16;
const G_FN_ID_GENERIC: u8 = 17;
/// `function-definition-params`, its `( … )`, a parameter's default value, what follows them,
/// the trailing return type, the body's `{` and the body.
const FN_PARAMS: u8 = 18;
const FN_PARAMS_IN: u8 = 19;
const FN_PARAM_DEFAULT: u8 = 20;
const FN_CONTINUE: u8 = 21;
/// `function-definition-trailing-return`, `method-definition-trailing-return`: the same but
/// for where their body goes, which is the same.
const FN_TRAILING: u8 = 22;
/// `function-definition-body`, `method-definition-body`: `{`.
const FN_BODY: u8 = 23;
/// `#anon_function-definition-body_0`, `#anon_method-definition-body_0`.
const FN_BLOCK: u8 = 24;
/// `#anon_function-trailing-return-type-parens_0`.
const TRAILING_PARENS: u8 = 25;
/// `#anon_namespace_0`, `_1`, `_2`.
const NS_NAME: u8 = 26;
const NS: u8 = 27;
const NS_BLOCK: u8 = 28;
/// `data-structures`, then a class's, a struct's, a union's, an enum's definition, name,
/// after the name, and body, four ids apart by kind.
const DS: u8 = 29;
const CLASS_DEF: u8 = 30;
const STRUCT_DEF: u8 = 31;
const UNION_DEF: u8 = 32;
const ENUM_DEF: u8 = 33;
const CLASS_NAME: u8 = 34;
#[allow(dead_code)]
const STRUCT_NAME: u8 = 35;
#[allow(dead_code)]
const UNION_NAME: u8 = 36;
const ENUM_NAME: u8 = 37;
const CLASS_AFTER: u8 = 38;
const STRUCT_AFTER: u8 = 39;
const UNION_AFTER: u8 = 40;
const ENUM_AFTER: u8 = 41;
const CLASS_BODY: u8 = 42;
const STRUCT_BODY: u8 = 43;
const UNION_BODY: u8 = 44;
const ENUM_BODY: u8 = 45;
/// `#anon_data-structures-definition-common-end_0`, `_1`: the base classes.
const INHERIT: u8 = 46;
const INHERITED_NAME: u8 = 47;
/// `#anon_data-structures-body_0`.
const DS_TEMPLATE: u8 = 48;
const DS_MODIFIER: u8 = 49;
/// `#anon_data-structures-modifier_0`, `data-structures-modifier-friend`, its block.
const FRIEND: u8 = 50;
const FRIEND_DS: u8 = 51;
const FRIEND_BLOCK: u8 = 52;
/// `#anon_data-structures-modifier_1`: `static_assert`'s `(`.
const STATIC_ASSERT: u8 = 53;
const DS_TYPE: u8 = 54;
/// `#anon_data-structures-type_0`: `identifiers`, then `data-structures-maybe-method`.
const DS_TYPE_STRUCT: u8 = 55;
const DS_MAYBE_METHOD: u8 = 56;
const DS_FN_ID: u8 = 57;
const DS_FN_ID_GENERIC: u8 = 58;
const DS_FN_ID_GENERIC_IN: u8 = 59;
const METHOD_PARAMS: u8 = 60;
const METHOD_PARAMS_IN: u8 = 61;
const METHOD_PARAM_DEFAULT: u8 = 62;
const METHOD_CONTINUE: u8 = 63;
/// `#anon_method-definition-continue_0` (`:`), `_1` (the initializers), `_2` (a member's
/// `(` or `{`).
const MC_COLON: u8 = 64;
const MC_INIT: u8 = 65;
const MC_MEMBER: u8 = 66;
/// `#anon_template_0`, `_1`, `_2`.
const TEMPLATE: u8 = 67;
const TEMPLATE_ARGS: u8 = 68;
const TEMPLATE_AFTER: u8 = 69;
const TYPEDEF: u8 = 70;
const CASE: u8 = 71;
/// `#anon_keywords-angle-brackets_0`, `_1`.
const CAST: u8 = 72;
const CAST_ARGS: u8 = 73;
/// `< … >` of expressions without generic types or calls: `#anon_angle-brackets_0`,
/// `#anon_generic-type_1`, `_7`.
const ANGLE: u8 = 74;
const BLOCK: u8 = 75;
const BRACKETS: u8 = 76;
/// `( … )` of expressions, its scopes aside: a group's, a call's, a cast's, `sizeof`'s.
const PAREN: u8 = 77;
/// `{ … }` of expressions: a call's braces.
const BRACE_EXPR: u8 = 78;
/// `__attribute__((`, `__declspec(` and its arguments.
const ATTRIBUTE: u8 = 79;
const DECLSPEC: u8 = 80;
const DECLSPEC_ARGS: u8 = 81;
/// `#anon_function-call_0`.
const FC: u8 = 82;
/// `#anon_generic-type_0`, `_6`.
const GT: u8 = 83;
const GT_NAME: u8 = 84;
/// `members-after-accessor-junction` and its anonymous contexts.
const MEMBERS: u8 = 85;
const MEMBERS_TEMPLATE: u8 = 86;
const MEMBERS_TEMPLATE_CALL: u8 = 87;
const MEMBERS_GENERIC: u8 = 88;
const MEMBERS_GENERIC_ARGS: u8 = 89;
const MEMBERS_GENERIC_CALL: u8 = 90;
/// `#anon_negated-block_0`.
const NEGATED: u8 = 91;
const NUM_HEX_FLOAT: u8 = 92;
const NUM_BIN: u8 = 93;
const NUM_HEX: u8 = 94;
const NUM_OCT: u8 = 95;
const NUM_DEC: u8 = 96;
const RAW_STRING: u8 = 97;
/// `#anon_preprocessor-macro-define_0`, `_1`, `preprocessor-macro-params` and its
/// `#anon_…_0`, `preprocessor-macro-definition` and the namespace in it.
const PP_DEFINE: u8 = 98;
const PP_DEFINE_OPEN: u8 = 99;
const PP_PARAMS: u8 = 100;
const PP_PARAM: u8 = 101;
const PP_MACRO: u8 = 102;
const PP_MACRO_NS: u8 = 103;
/// `#anon_preprocessor-other_0` … `_4`.
const PP_DIRECTIVE: u8 = 104;
const PP_DIAGNOSTIC: u8 = 105;
const PP_INCLUDE: u8 = 106;
const PP_INC_DQ: u8 = 107;
const PP_INC_LT: u8 = 108;
/// `#anon_preprocessor-rule-enabled-*_0`, `_2` by what they hold (global, statements,
/// data structures), and `_1`, the same for all.
const EN_G: u8 = 109;
const EN_S: u8 = 110;
const EN_D: u8 = 111;
const EN_IN_G: u8 = 112;
const EN_IN_S: u8 = 113;
const EN_IN_D: u8 = 114;
const ELSE_COMMENT: u8 = 115;
/// `#anon_preprocessor-rule-disabled-*_0`, `_1`, and `_2`, the same for all.
const DIS_G: u8 = 116;
const DIS_S: u8 = 117;
const DIS_D: u8 = 118;
const DIS_ELSE_G: u8 = 119;
const DIS_ELSE_S: u8 = 120;
const DIS_ELSE_D: u8 = 121;
const IF_COMMENT: u8 = 122;
/// `#anon_preprocessor-rule-other-*_0`: an `#if` line.
const OTHER_G: u8 = 123;
const OTHER_S: u8 = 124;
/// `preprocessor-if-branch-*`, `-elif-else-branch-*`, `-block-if-branch-*`,
/// `-block-finish-*`, `-block-finish-if-branch-*`, global and statements.
const IF_G: u8 = 125;
const IF_S: u8 = 126;
const ELIF_G: u8 = 127;
const ELIF_S: u8 = 128;
const BLOCK_IF_G: u8 = 129;
const BLOCK_IF_S: u8 = 130;
const FINISH_G: u8 = 131;
const FINISH_S: u8 = 132;
const FINISH_IF_G: u8 = 133;
const FINISH_IF_S: u8 = 134;
/// `preprocessor-if-branch-function-call` and its arguments.
const PP_FC: u8 = 135;
const PP_FC_ARGS: u8 = 136;
/// `#anon_preprocessor-convention-ignore-uppercase-ident-lines_0`.
const UPPER_IDENT: u8 = 137;
// C's, by bat's names where one is one of them
/// From `C_FIRST` on, a context is C's: what it gives carries C's scopes.
const C_FIRST: u8 = 138;
const C_COMMENT: u8 = 138;
const C_LINE_COMMENT: u8 = 139;
const C_STRING: u8 = 140;
const C_CHAR: u8 = 141;
const C_PP_COMMENT: u8 = 142;
const C_PP_LINE_COMMENT: u8 = 143;
/// `#anon_preprocessor-disabled_0`: an `#if` in a disabled branch, to its `#endif`.
const C_DISABLED: u8 = 144;
/// `#anon_preprocessor-convention-ignore-uppercase-calls-without-semicolon_0`.
const C_MACRO_CALL: u8 = 145;
const C_PAREN: u8 = 146;
const C_BRACKETS: u8 = 147;
const C_BLOCK: u8 = 148;
const C_CASE: u8 = 149;
const C_TYPEDEF: u8 = 150;
const C_FC: u8 = 151;
const C_ATTRIBUTE: u8 = 152;
const C_DECLSPEC: u8 = 153;
const C_DECLSPEC_ARGS: u8 = 154;
const C_NUM_HEX_FLOAT: u8 = 155;
const C_NUM_HEX: u8 = 156;
const C_NUM_OCT: u8 = 157;
const C_NUM_DEC: u8 = 158;
const C_NEGATED: u8 = 159;
const C_PP_DEFINE: u8 = 160;
const C_PP_DEFINE_OPEN: u8 = 161;
const C_PP_PARAMS: u8 = 162;
const C_PP_PARAM: u8 = 163;
const C_PP_MACRO: u8 = 164;
const C_PP_DIRECTIVE: u8 = 165;
const C_PP_DIAGNOSTIC: u8 = 166;
const C_PP_INCLUDE: u8 = 167;
const C_PP_INC_DQ: u8 = 168;
const C_PP_INC_LT: u8 = 169;
const C_EN_S: u8 = 170;
const C_EN_D: u8 = 171;
const C_EN_IN_S: u8 = 172;
const C_EN_IN_D: u8 = 173;
const C_ELSE_COMMENT: u8 = 174;
const C_DIS_S: u8 = 175;
const C_DIS_D: u8 = 176;
const C_DIS_ELSE_S: u8 = 177;
const C_DIS_ELSE_D: u8 = 178;
const C_IF_COMMENT: u8 = 179;
const C_OTHER_S: u8 = 180;
const C_IF_S: u8 = 181;
const C_ELIF_S: u8 = 182;
const C_BLOCK_IF_S: u8 = 183;
const C_FINISH_S: u8 = 184;
const C_FINISH_IF_S: u8 = 185;
const C_PP_FC: u8 = 186;
const C_PP_FC_ARGS: u8 = 187;
const C_UPPER_IDENT: u8 = 188;
const C_FN_PARAMS: u8 = 189;
const C_FN_PARAMS_IN: u8 = 190;
const C_FN_CONTINUE: u8 = 191;
const C_FN_BODY: u8 = 192;
const C_FN_BLOCK: u8 = 193;
const C_DS: u8 = 194;
/// C's struct, enum and union definitions, after their names, and their bodies, three ids
/// apart by kind.
const C_STRUCT_DEF: u8 = 195;
const C_ENUM_DEF: u8 = 196;
const C_UNION_DEF: u8 = 197;
const C_STRUCT_AFTER: u8 = 198;
const C_ENUM_AFTER: u8 = 199;
const C_UNION_AFTER: u8 = 200;
const C_STRUCT_BODY: u8 = 201;
const C_ENUM_BODY: u8 = 202;
const C_UNION_BODY: u8 = 203;

/// Which contexts hold `meta.preprocessor`: in their meta scope (the tokens that open and
/// close them carry it too), or in their meta content scope only.
fn pre_flags(c: u8) -> u8 {
    match c {
        PP_PARAMS | PP_DIRECTIVE | PP_DIAGNOSTIC | PP_INCLUDE | OTHER_G | OTHER_S | C_PP_PARAMS
        | C_PP_DIRECTIVE | C_PP_DIAGNOSTIC | C_PP_INCLUDE | C_OTHER_S => META,
        PP_DEFINE | PP_MACRO | C_PP_DEFINE | C_PP_MACRO => CONTENT,
        _ => 0,
    }
}
const META: u8 = 1;
const CONTENT: u8 = 2;

/// The scope of a context that colors what it holds, as a kind's wrap.
fn wrap_of(c: u8) -> u16 {
    match c {
        IF_COMMENT => W_IF,
        ELSE_COMMENT => W_ELSE,
        C_IF_COMMENT => W_IF | C,
        C_ELSE_COMMENT => W_ELSE | C,
        INHERITED_NAME => W_INHERITED,
        NS_NAME | PP_MACRO_NS => W_NAMESPACE,
        DS_FN_ID | DS_FN_ID_GENERIC_IN => W_FUNCTION,
        _ => 0,
    }
}

/// Bits of a kind past its leaf.
/// The scopes are C's (`.c`), not C++'s.
const C: u16 = 1 << 8;
/// In `meta.preprocessor`.
const PRE: u16 = 1 << 9;
/// The context around that colors, `wrap_of`'s.
const WRAP: u16 = 1 << 10;
const W_IF: u16 = WRAP;
const W_ELSE: u16 = 2 * WRAP;
const W_INHERITED: u16 = 3 * WRAP;
const W_NAMESPACE: u16 = 4 * WRAP;
const W_FUNCTION: u16 = 5 * WRAP;
const WRAPS: u16 = 7 * WRAP;

macro_rules! leaves {
    ($($name:ident = [$($s:literal),*],)*) => {
        /// The leaves: a token's scopes, C++'s and C's.
        static LEAVES: [[&[&str]; 2]; Leaf::Count as usize] = [
            $([&[$(concat!($s, ".c++")),*], &[$(concat!($s, ".c")),*]],)*
        ];
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[repr(u16)]
        enum Leaf { $($name,)* Count }
        $(const $name: u16 = Leaf::$name as u16;)*
    };
}

leaves! {
    PLAIN = [],
    BLOCK_TEXT = ["comment.block"],
    COMMENT_MARK = ["comment.block", "punctuation.definition.comment"],
    BLOCK_CONT = ["comment.block", "punctuation.separator.continuation"],
    LINE_TEXT = ["comment.line.double-slash"],
    LINE_MARK = ["comment.line.double-slash", "punctuation.definition.comment"],
    LINE_CONT = ["comment.line.double-slash", "punctuation.separator.continuation"],
    BANNER_LINE = ["comment.line.banner"],
    STRAY_COMMENT_END = ["invalid.illegal.stray-comment-end"],
    DQ = ["string.quoted.double"],
    DQ_PREFIX = ["string.quoted.double", "storage.type.string"],
    DQ_BEGIN = ["string.quoted.double", "punctuation.definition.string.begin"],
    DQ_END = ["string.quoted.double", "punctuation.definition.string.end"],
    DQ_ESCAPE = ["string.quoted.double", "constant.character.escape"],
    DQ_BAD_ESCAPE = ["string.quoted.double", "invalid.illegal.unknown-escape"],
    DQ_CONT = ["string.quoted.double", "punctuation.separator.continuation"],
    DQ_PLACEHOLDER = ["string.quoted.double", "constant.other.placeholder"],
    SQ = ["string.quoted.single"],
    SQ_PREFIX = ["string.quoted.single", "storage.type.string"],
    SQ_BEGIN = ["string.quoted.single", "punctuation.definition.string.begin"],
    SQ_END = ["string.quoted.single", "punctuation.definition.string.end"],
    SQ_ESCAPE = ["string.quoted.single", "constant.character.escape"],
    SQ_BAD_ESCAPE = ["string.quoted.single", "invalid.illegal.unknown-escape"],
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
    HEX_FLOAT_BAD = ["constant.numeric.float.hexadecimal", "invalid.illegal.numeric.digit"],
    BIN = ["constant.numeric.integer.binary"],
    BIN_BASE = ["constant.numeric.integer.binary", "punctuation.definition.numeric.base"],
    BIN_SUFFIX = ["constant.numeric.integer.binary", "storage.type.numeric"],
    BIN_BAD = ["constant.numeric.integer.binary", "invalid.illegal.numeric.digit"],
    HEX = ["constant.numeric.integer.hexadecimal"],
    HEX_BASE = ["constant.numeric.integer.hexadecimal", "punctuation.definition.numeric.base"],
    HEX_SUFFIX = ["constant.numeric.integer.hexadecimal", "storage.type.numeric"],
    HEX_BAD_SUFFIX = ["constant.numeric.integer.hexadecimal", "invalid.illegal.numeric.suffix"],
    OCT = ["constant.numeric.integer.octal"],
    OCT_BASE = ["constant.numeric.integer.octal", "punctuation.definition.numeric.base"],
    OCT_SUFFIX = ["constant.numeric.integer.octal", "storage.type.numeric"],
    OCT_BAD = ["constant.numeric.integer.octal", "invalid.illegal.numeric.digit"],
    OCT_BAD_SUFFIX = ["constant.numeric.integer.octal", "invalid.illegal.numeric.suffix"],
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
    ALIAS = ["keyword.operator.alias"],
    ACCESSOR = ["punctuation.accessor"],
    ARROW = ["punctuation.accessor.arrow"],
    DOT = ["punctuation.accessor.dot"],
    DOUBLE_COLON = ["punctuation.accessor.double-colon"],
    ILLEGAL_SYNTAX = ["invalid.illegal.syntax"],
    MEMBER = ["variable.other.member"],
    SEPARATOR = ["punctuation.separator"],
    TERMINATOR = ["punctuation.terminator"],
    INIT_LIST = ["punctuation.separator.initializer-list"],
    CLASS_SECTION = ["punctuation.section.class"],
    CONT = ["punctuation.separator.continuation"],
    SPACE_AFTER_CONT = ["invalid.illegal.space-after-continuation"],
    GROUP_BEGIN = ["punctuation.section.group.begin"],
    GROUP_END = ["punctuation.section.group.end"],
    BLOCK_BEGIN = ["punctuation.section.block.begin"],
    BLOCK_END = ["punctuation.section.block.end"],
    BRACKETS_BEGIN = ["punctuation.section.brackets.begin"],
    BRACKETS_END = ["punctuation.section.brackets.end"],
    GENERIC_BEGIN = ["punctuation.section.generic.begin"],
    GENERIC_END = ["punctuation.section.generic.end"],
    STRAY_BRACKET = ["invalid.illegal.stray-bracket-end"],
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
    IMPORT_PRAGMA = ["keyword.control.import.pragma"],
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
    CONSTANT_OTHER = ["constant.other"],
    VAR_LANGUAGE = ["variable.language"],
    MEMBER_VAR = ["variable.other.readwrite.member"],
    GLOBAL_MAC = ["variable.other.readwrite.global.mac-classic"],
    STATIC_MAC = ["variable.other.readwrite.static.mac-classic"],
    FUNCTION = ["variable.function"],
    FUNCTION_ACCESSOR = ["variable.function", "punctuation.accessor"],
    MEMBER_FUNCTION = ["variable.function.member"],
    MACRO_FUNCTION = ["variable.function.assumed-macro"],
    BASE_CLASS = ["variable.other.base-class"],
    PARAMETER = ["variable.parameter"],
    ENTITY_FUNCTION = ["entity.name.function"],
    CTOR = ["entity.name.function.constructor"],
    DTOR = ["entity.name.function.destructor"],
    CTOR_ACCESSOR = ["entity.name.function.constructor", "punctuation.accessor"],
    DTOR_ACCESSOR = ["entity.name.function.destructor", "punctuation.accessor"],
    PP_FUNCTION = ["entity.name.function.preprocessor"],
    PP_CONSTANT = ["entity.name.constant.preprocessor"],
    CLASS_NAME_LEAF = ["entity.name.class"],
    CLASS_FWD = ["entity.name.class.forward-decl"],
    STRUCT_NAME_LEAF = ["entity.name.struct"],
    STRUCT_FWD = ["entity.name.struct.forward-decl"],
    UNION_NAME_LEAF = ["entity.name.union"],
    UNION_FWD = ["entity.name.union.forward-decl"],
    ENUM_NAME_LEAF = ["entity.name.enum"],
    ENUM_FWD = ["entity.name.enum.forward-decl"],
    TYPEDEF_NAME = ["entity.name.type.typedef"],
    USING_NAME = ["entity.name.type.using"],
    LABEL = ["entity.name.label"],
    UNEXPECTED_CHAR = ["invalid.illegal.unexpected-character"],
}

/// The scopes under `source.c++` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    let lang = usize::from(kind & C != 0);
    let suffix = |s: &'static str, c: &'static str| if lang == 1 { c } else { s };
    let mut out = Vec::new();
    let wrap = kind & WRAPS;
    let pre = kind & PRE != 0;
    let comment = matches!(wrap, W_IF | W_ELSE);
    if pre && !comment {
        out.push(suffix("meta.preprocessor.c++", "meta.preprocessor.c"));
    }
    out.extend(match wrap {
        W_IF => Some(suffix(
            "comment.block.preprocessor.if-branch.c++",
            "comment.block.preprocessor.if-branch.c",
        )),
        W_ELSE => Some(suffix(
            "comment.block.preprocessor.else-branch.c++",
            "comment.block.preprocessor.else-branch.c",
        )),
        W_INHERITED => Some("entity.other.inherited-class.c++"),
        W_NAMESPACE => Some("entity.name.namespace.c++"),
        W_FUNCTION => Some("entity.name.function.c++"),
        _ => None,
    });
    if pre && comment {
        out.push(suffix("meta.preprocessor.c++", "meta.preprocessor.c"));
    }
    if let Some(leaf) = LEAVES.get(usize::from(kind & (C - 1))) {
        out.extend(leaf[lang].iter().copied());
    }
    out
}

/// How many contexts a line can hold; one pushed past it drops the bottom one.
const MAX: usize = 128;
/// How many the state keeps between lines: [`State`]'s levels and withs.
const KEPT: usize = 2 * HALF;
/// A raw string's longest delimiter.
const DELIM: usize = 16;

/// Lex `b`, a line without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let mut s = Scan {
        b,
        stack: [MAIN; MAX],
        depth: 1,
        exact: state.is_exact(),
        pre: 0,
        wrap: 0,
        text: PLAIN,
        at: 0,
        loop_at: (usize::MAX, 0),
        halt: false,
        delim: [0; DELIM],
        delim_len: 0,
        word_at: std::cell::Cell::new((usize::MAX, 0)),
        kw_at: std::cell::Cell::new((usize::MAX, words::Kw::Other)),
        id_cache: std::cell::Cell::new((usize::MAX, None)),
        out,
    };
    let stored = usize::from(state.depth);
    // an open raw string's delimiter is in the top bytes, its length plus 1 in `arg`
    let d = usize::from(state.arg).saturating_sub(1).min(stored);
    let n = stored - d;
    let byte = |i: usize| {
        if i < HALF {
            state.levels[i]
        } else {
            state.withs[i - HALF]
        }
    };
    for i in 0..n {
        let c = byte(i);
        s.stack[1 + i] = c;
        s.count_in(c);
    }
    for i in 0..d {
        s.delim[i] = byte(n + i);
    }
    s.delim_len = d;
    s.depth = 1 + n;
    s.wrap = s.stack[..s.depth]
        .iter()
        .rev()
        .map(|&c| wrap_of(c))
        .find(|&w| w != 0)
        .unwrap_or(0);
    s.refresh();
    s.run();
    s.fill(b.len());
    let raw = s.stack[1..s.depth].contains(&RAW_STRING);
    let d = if raw { s.delim_len } else { 0 };
    let n = s.depth - 1;
    let from = n.saturating_sub(KEPT - d);
    let mut packed = State {
        depth: (n - from + d) as u8,
        inexact: !s.exact || from > 0,
        arg: if raw { d as u16 + 1 } else { 0 },
        ..State::default()
    };
    let bytes = s.stack[1 + from..s.depth]
        .iter()
        .chain(&s.delim[..d])
        .copied();
    for (i, c) in bytes.enumerate() {
        if i < HALF {
            packed.levels[i] = c;
        } else {
            packed.withs[i - HALF] = c;
        }
    }
    *state = packed;
}

struct Scan<'b, 'c> {
    b: &'b [u8],
    stack: [u8; MAX],
    depth: usize,
    exact: bool,
    /// How many contexts on the stack hold `meta.preprocessor`.
    pre: usize,
    /// The wrap of the topmost context that has one.
    wrap: u16,
    /// The kind of text no pattern matches.
    text: u16,
    /// Where the spans so far end.
    at: usize,
    /// Where the last push that matched nothing was, and the depth its first context left: a
    /// pop there, at that depth, that matches nothing would loop, so syntect tries the patterns
    /// after it and else steps a character.
    loop_at: (usize, usize),
    /// Nothing more to match: the line's `\n` was taken, or a loop at its end.
    halt: bool,
    /// The open raw string's delimiter.
    delim: [u8; DELIM],
    delim_len: usize,
    /// The last whole word looked up: where it starts and ends.
    word_at: std::cell::Cell<(usize, usize)>,
    /// What the last word looked up at a place is to C++'s expressions.
    kw_at: std::cell::Cell<(usize, words::Kw)>,
    /// Where the last identifier looked for at a place ends ([`text::id`]).
    id_cache: std::cell::Cell<(usize, Option<usize>)>,
    out: &'b mut Out<'c>,
}

impl<'b> Scan<'b, '_> {
    fn top(&self) -> u8 {
        self.stack[self.depth - 1]
    }

    /// `leaf` here: in `meta.preprocessor` and the wrap around.
    fn kind(&self, leaf: u16) -> u16 {
        let k = leaf | self.wrap;
        if self.pre > 0 { k | PRE } else { k }
    }

    /// Bytes `s..e` are `leaf`, after the text before them.
    fn put(&mut self, s: usize, e: usize, leaf: u16) {
        let k = self.kind(leaf);
        self.put_kind(s, e, k);
    }

    /// Bytes `s..e` are kind `k` as it is.
    fn put_kind(&mut self, s: usize, e: usize, k: u16) {
        self.fill(s);
        // never over the spans already out
        let s = s.max(self.at);
        let e = e.min(self.b.len());
        if k & !C != PLAIN && e > s {
            self.out.push_merged(s, e, k);
        }
        self.at = self.at.max(e);
    }

    /// The text up to `p` is the context's.
    fn fill(&mut self, p: usize) {
        let p = p.min(self.b.len());
        if p > self.at {
            if self.text & !C != PLAIN {
                self.out.push_merged(self.at, p, self.text);
            }
            self.at = p;
        }
    }

    fn refresh(&mut self) {
        let t = match self.top() {
            C_COMMENT | C_PP_COMMENT => BLOCK_TEXT | C,
            C_LINE_COMMENT | C_PP_LINE_COMMENT => LINE_TEXT | C,
            C_STRING => DQ | C,
            C_CHAR => SQ | C,
            RAW_STRING => DQ,
            PP_INC_DQ => INC_DQ,
            PP_INC_LT => INC_LT,
            C_PP_INC_DQ => INC_DQ | C,
            C_PP_INC_LT => INC_LT | C,
            NUM_HEX_FLOAT => HEX_FLOAT,
            NUM_BIN => BIN,
            NUM_HEX => HEX,
            NUM_OCT => OCT,
            NUM_DEC => DEC,
            C_NUM_HEX_FLOAT => HEX_FLOAT | C,
            C_NUM_HEX => HEX | C,
            C_NUM_OCT => OCT | C,
            C_NUM_DEC => DEC | C,
            c if c >= C_FIRST => C,
            _ => PLAIN,
        };
        self.text = self.kind(t);
    }

    fn count_in(&mut self, c: u8) {
        self.pre += usize::from(pre_flags(c) != 0);
    }

    fn count_out(&mut self, c: u8) {
        self.pre -= usize::from(pre_flags(c) != 0);
    }

    fn push(&mut self, at: usize, c: u8) {
        self.fill(at);
        if self.depth == MAX {
            let gone = self.stack[1];
            self.count_out(gone);
            self.stack.copy_within(2.., 1);
            self.depth -= 1;
            self.exact = false;
        }
        self.stack[self.depth] = c;
        self.depth += 1;
        self.count_in(c);
        let w = wrap_of(c);
        if w != 0 {
            self.wrap = w;
        }
        self.refresh();
    }

    fn pop(&mut self, at: usize) {
        self.fill(at);
        if self.depth > 1 {
            self.depth -= 1;
            let c = self.stack[self.depth];
            self.count_out(c);
            if wrap_of(c) != 0 {
                self.wrap = 0;
            }
        }
        self.refresh();
    }

    /// Replace the top with `cs`, bottom first.
    fn set(&mut self, at: usize, cs: &[u8]) {
        self.pop(at);
        for &c in cs {
            self.push(at, c);
        }
    }

    /// A lookahead's pop at `p`; where it would loop, a step past the character.
    fn leave(&mut self, p: usize) -> usize {
        if self.loop_at == (p, self.depth) {
            if p >= self.b.len() {
                self.halt = true;
                return p;
            }
            return next(self.b, p);
        }
        self.pop(p);
        p
    }

    /// A lookahead's push of `cs` at `p`, bottom first. syntect keeps the depth the first of
    /// them leaves, however many there are, for its guard against a loop.
    fn enter(&mut self, p: usize, cs: &[u8]) -> usize {
        let depth = self.depth + 1;
        for &c in cs {
            self.push(p, c);
        }
        self.loop_at = (p, depth);
        p
    }

    /// A lookahead's set at `p`.
    fn shift(&mut self, p: usize, cs: &[u8]) -> usize {
        self.set(p, cs);
        p
    }

    /// `n` bytes at `p` are `leaf`: where they end.
    fn token(&mut self, p: usize, n: usize, leaf: u16) -> usize {
        self.put(p, p + n, leaf);
        p + n
    }

    /// `n` bytes at `p`, `leaf`, that open `c` (whose meta scope they carry).
    fn open(&mut self, p: usize, n: usize, c: u8, leaf: u16) -> usize {
        self.push(p, c);
        self.token(p, n, leaf)
    }

    /// `n` bytes at `p`, `leaf`, that close the context.
    fn close(&mut self, p: usize, n: usize, leaf: u16) -> usize {
        self.put(p, p + n, leaf);
        self.pop(p + n);
        p + n
    }

    /// `n` bytes at `p`, `leaf`, that replace the context with `cs`.
    fn then(&mut self, p: usize, n: usize, cs: &[u8], leaf: u16) -> usize {
        self.put(p, p + n, leaf);
        self.set(p + n, cs);
        p + n
    }

    /// `(?=\S)` at `p` (the line's end is its `\n`, a space): a pop.
    fn any_pop(&mut self, p: usize) -> Option<usize> {
        (p < self.b.len() && !crate::is_space(self.b[p]) && self.can_pop(p)).then(|| self.leave(p))
    }

    /// Whether a pop that takes nothing at `p` doesn't loop: syntect's guard passes over one
    /// right after a push that took nothing there, at the depth that push left, so that the
    /// context's later patterns get their turn (and else the scan steps on).
    fn can_pop(&self, p: usize) -> bool {
        self.loop_at != (p, self.depth)
    }

    /// `(?=\S)` at `p`: a set.
    fn any_set(&mut self, p: usize, cs: &[u8]) -> Option<usize> {
        (p < self.b.len() && !crate::is_space(self.b[p])).then(|| self.shift(p, cs))
    }

    /// The whole word at `p`, `\bw\b`, if `p` starts one.
    fn word(&self, p: usize) -> &'b [u8] {
        let b = self.b;
        let (at, end) = self.word_at.get();
        if at == p {
            return &b[p..end];
        }
        let end = if text::boundary(b, p) && text::is_word(b, p) {
            text::word_end(b, p)
        } else {
            p
        };
        self.word_at.set((p, end));
        &b[p..end]
    }

    /// What the whole word at `p` is to C++'s expressions ([`words::lookup`]), once a place.
    fn kw(&self, p: usize) -> words::Kw {
        let (at, k) = self.kw_at.get();
        if at == p {
            return k;
        }
        let k = words::lookup(self.word(p));
        self.kw_at.set((p, k));
        k
    }

    /// [`text::id`] at `p`, once a place.
    fn id_at(&self, p: usize) -> Option<usize> {
        let (at, e) = self.id_cache.get();
        if at == p {
            return e;
        }
        let e = text::id(self.b, p);
        self.id_cache.set((p, e));
        e
    }

    fn run(&mut self) {
        let mut p = 0;
        // matches that take nothing, in a row at one place: past syntect's guard, a step on
        let mut stuck = 0;
        loop {
            if let Some(q) = self.at(p) {
                stuck = if q == p { stuck + 1 } else { 0 };
                p = q;
                if self.halt {
                    return;
                }
                if stuck > 64 {
                    stuck = 0;
                    if p >= self.b.len() {
                        return;
                    }
                    p = self.advance(p);
                }
            } else if p < self.b.len() {
                p = self.advance(p);
            } else {
                return;
            }
        }
    }

    /// The first pattern of the top context that matches at `p`, applied: where the scan goes
    /// on, or `None` when none does.
    fn at(&mut self, p: usize) -> Option<usize> {
        let top = self.top();
        let c = self.b.get(p).copied().unwrap_or(b'\n');
        if top >= C_FIRST {
            self.c_context(top, p, c)
        } else if (PP_DEFINE..=UPPER_IDENT).contains(&top) {
            self.pp_context(top, p, c)
        } else {
            self.cxx_context(top, p, c)
        }
    }

    /// Where the next pattern can start, after none did at `p`.
    fn advance(&self, p: usize) -> usize {
        let top = self.top();
        if top >= C_FIRST {
            self.c_advance(top, p)
        } else {
            self.cxx_advance(top, p)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Lang;
    use crate::tests::marked;

    #[test]
    fn a_class_and_its_method() {
        let m = marked(
            Lang::Cpp,
            &[
                "class Foo : public Bar {",
                " public:",
                "  int get() const { return x_; }",
                "};",
            ],
        );
        assert!(m.contains(&("Foo", "entity.name.class.c++")));
        assert!(
            m.iter()
                .any(|&(t, s)| t == "Bar" && s.contains("entity.other.inherited-class"))
        );
        assert!(
            m.iter()
                .any(|&(t, s)| t == "return" && s.contains("keyword.control.flow.return.c++"))
        );
    }

    #[test]
    fn a_raw_string_across_lines() {
        let m = marked(Lang::Cpp, &["auto s = R\"x(one", ")\" still )x\";"]);
        assert!(m.contains(&(")\" still ", "string.quoted.double.c++")));
        assert!(m.contains(&(
            ")x\"",
            "string.quoted.double.c++ punctuation.definition.string.end.c++"
        )));
    }

    #[test]
    fn a_preprocessor_line() {
        let m = marked(
            Lang::Cpp,
            &["#include <vector>", "#define MAX(a, b) ((a) > (b))"],
        );
        assert!(m.contains(&(
            "#include",
            "meta.preprocessor.c++ keyword.control.import.include.c++"
        )));
        assert!(m.contains(&(
            "MAX",
            "meta.preprocessor.c++ entity.name.function.preprocessor.c++"
        )));
    }
}
