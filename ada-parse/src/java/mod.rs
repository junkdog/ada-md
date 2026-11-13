//! Java by a scanner of ada's own (ADR-40, speed first), with the Javadoc it embeds and the part
//! of HTML that Javadoc includes, emitting bat's scopes, with bat's `Java` grammar the reference
//! it is checked against.
//!
//! **Code is read a token at a time** (X-015, [`code`]): on the C family's core
//! ([`crate::clike`]), a name read once and looked up once, a type read whole, and frames of
//! ada's own (a block, a class's body, a member's declaration, parentheses, parameters, an
//! annotation's arguments) that classify each token by what follows it on the line. The rare
//! declarations' headers (a class's, a type parameter's, `throws`, imports, the package,
//! `module-info.java`) stay contexts of bat's grammar compiled by hand, each an arm of one
//! `match` on the top of the stack trying its patterns in the grammar's order at the scan's
//! place (syntect's leftmost match), as do Javadoc and its HTML, strings and comments.
//!
//! The grammar keeps meta scopes on nearly every context, but the built-in themes key on one of
//! them only, `meta.annotation`, so a kind is a leaf (bat's scopes for a token) in an
//! annotation or not. Inside Javadoc a kind also says which HTML string or comment holds it
//! ([`scopes`]).
//!
//! It colors as the grammar does, odd corners included: `@Override`'s line end and the
//! next line's indentation are the annotation's; `default` is a modifier in a block, and
//! `default ->` a lambda's parameter; a line comment ends at `%>`, which then empties the stack;
//! an unknown block tag (`@hidden`) leaves the rest of its Javadoc plain to the next tag;
//! `List<String>` in Javadoc is an HTML tag. **Not followed:** CSS and JavaScript in Javadoc's
//! HTML (`<style>`, `<script>`, `style=`, `onclick=`), whose text is the comment's; past ASCII,
//! the Unicode classes are `char`'s (`\p{L}` its alphabetic, `\w` its alphanumeric), and `\s`
//! is ASCII's.
//!
//! The stack between lines is the frames and contexts open above the top level, up to 28 (the
//! bytes of [`State`]'s levels).
use crate::prim::{char_len, find_any};

mod code;
mod doc;

use crate::emit::{Push, Rules, Run, Runs, Sink};
use crate::regex;
use crate::sql;
use crate::{Out, State, is_space, skip_spaces, trim_end};
use code::is_frame;

// The frames of ada's own ([`code`]), below `FRAMES`.
/// The top level (`main`): declarations, then code.
const TOP: u8 = 0;
/// A class's body, `}` closing it.
const CLASS_BODY: u8 = 1;
/// An enum's body: a class's, and a line of constants.
const ENUM_BODY: u8 = 2;
/// An anonymous class's body, whose `}` is a brace's.
const ANON_BODY: u8 = 3;
/// `{ … }` of statements: a method's, a block's, an initializer's.
const BLOCK: u8 = 4;
/// An enum's constants, to `;` or `}`.
const ENUM_CONSTANTS: u8 = 5;
/// After an enum constant: its arguments and body.
const ENUM_CONSTANT: u8 = 6;
/// `( … )` of code: a group's, a call's.
const PARENS: u8 = 7;
/// A call's `(` whose first argument is SQL (`prepareStatement(`, `createQuery(`…), to its
/// first `,` (which makes it [`PARENS`]) or its `)` (ADR-40, phase 4).
const PARENS_SQL: u8 = 8;
/// A constructor's arguments, then what may follow them: an anonymous class's body.
const CTOR_ARGS: u8 = 9;
const CTOR_AFTER: u8 = 10;
/// A method's parameters, a lambda's, a `catch`'s.
const METHOD_PARAMS: u8 = 11;
const LAMBDA_PARAM_LIST: u8 = 12;
const CATCH_PARAMS: u8 = 13;
/// After `for` or `try`, their `( … )`; after `catch`, its.
const DECL_PARENS: u8 = 14;
const DECL_PARENS_IN: u8 = 15;
const CATCH: u8 = 16;
/// A lambda's parameters read, before its `->`; then its body, to `)`, `}` or `;`.
const LAMBDA_ARROW: u8 = 17;
const LAMBDA_BODY: u8 = 18;
const ASSERT: u8 = 19;
/// `{ … }` of an array's values; in an annotation's arguments.
const ARRAY_INIT: u8 = 20;
const ANNOT_ARRAY_INIT: u8 = 21;
/// After `new int` or a `]`: an array's `[` or values; then in its `[ … ]`.
const ARRAY_DEF: u8 = 22;
const ARRAY_DEF_IN: u8 = 23;
/// An annotation, before its `(`, then in its arguments, and a named argument's value; each
/// with SQL's variant (`@Query`, `@SqlUpdate`…: a string there is the `value`'s, SQL). One of
/// these on the stack, or [`ANNOT_DEFAULT`], is an annotation open (`meta.annotation`).
const ANNOT_PARAMS: u8 = 24;
const ANNOT_PARAMS_SQL: u8 = 25;
const ANNOT_ARGS: u8 = 26;
const ANNOT_ARGS_SQL: u8 = 27;
const ANNOT_VALUE: u8 = 28;
const ANNOT_VALUE_SQL: u8 = 29;
/// An annotation type's element's `default` value.
const ANNOT_DEFAULT: u8 = 30;
/// A method's declaration: its name, parameters, `throws`, to its body or `;`.
const METHOD: u8 = 31;
/// A member's type read, its name on a later line.
const MEMBER: u8 = 32;
/// A field's declarators; after a name (`static-assignment`); a value, to `,` or `;`.
const FIELD: u8 = 33;
const FIELD_NAMED: u8 = 34;
const STATIC_RHS: u8 = 35;
/// After `new`, its type; after the type.
const INSTANTIATION: u8 = 36;
const AFTER_NEW_TYPE: u8 = 37;
/// After `::` at a line's end.
const METHOD_REF: u8 = 38;
/// An assignment's value, to `;`, `)`, `}` or `,`.
const RHS: u8 = 39;
const FRAMES: u8 = 40;
// bat's contexts, for the rare declarations' headers, with its prototype (comments, `goto`,
// `const`, `%>`) as all those before `STRING`
const CLASS_TYPE: u8 = 40;
const CLASS_NAME: u8 = 41;
const CLASS_EXTENDS: u8 = 42;
const CLASS_SUPERS: u8 = 43;
const INHERITED_REF: u8 = 44;
const INHERITED_PATH: u8 = 45;
const INHERITED_NAME: u8 = 46;
const AFTER_INHERITED: u8 = 47;
const CLASS_BLOCK: u8 = 48;
const ENUM_BLOCK: u8 = 49;
const GENERIC_DECL: u8 = 50;
const GENERIC_PARAM: u8 = 51;
const GENERIC_BOUNDS: u8 = 52;
const MULTI_BOUNDS: u8 = 53;
/// A type's arguments that don't close on their line.
const GENERIC_INVOCATION: u8 = 54;
const GENERIC_ARG: u8 = 55;
/// `object-and-array-types`' package, its name, what follows it.
const OBJ_ARRAY_PATH: u8 = 56;
const AFTER_OBJ_ARRAY: u8 = 57;
const OBJ_ARRAY_TYPE: u8 = 58;
const ARRAY_BRACKETS: u8 = 59;
/// A type's package, to its name (`object-type-fqn`), across a line's end.
const TYPE_FQN: u8 = 60;
/// `object-type-reference`, its package, its name, what follows it.
const TYPE_REF: u8 = 61;
const TYPE_REF_PATH: u8 = 62;
const TYPE_REF_NAME: u8 = 63;
const AFTER_TYPE_REF: u8 = 64;
const THROWS_LIST: u8 = 65;
/// An annotation's package and name, across a line's end.
const ANNOT_PATH: u8 = 66;
const ANNOT_NAME: u8 = 67;
const ANNOT_AFTER: u8 = 68;
/// `module-info.java`'s.
const MODULE_KW: u8 = 69;
const MODULE_ID: u8 = 70;
const MODULE_BODY: u8 = 71;
const MODULE_BODY_IN: u8 = 72;
const EXPORTS: u8 = 73;
const EXPORTS_TO: u8 = 74;
const REQUIRES: u8 = 75;
const PROVIDES_WITH: u8 = 76;
const PROVIDES_LIST: u8 = 77;
/// `dot-separated-identifier` under each scope that holds it.
const DOTTED_NS: u8 = 78;
const DOTTED_MODULE: u8 = 79;
const DOTTED_PKG: u8 = 80;
const DOTTED_SMOD: u8 = 81;
const PACKAGE_NAME: u8 = 82;
const IMPORT_BODY: u8 = 83;
const IMPORT_PATH: u8 = 84;
const IMPORT_AFTER: u8 = 85;
const STATIC_IMPORT: u8 = 86;
const STATIC_PATH: u8 = 87;
const STATIC_AFTER: u8 = 88;
const STRING: u8 = 94;
const CHAR: u8 = 95;
const COMMENT: u8 = 96;
const LINE_COMMENT: u8 = 97;
// Javadoc's, by bat's names where one is one of them
/// `/** … */` around what follows (`#anon_main_0`).
const JD_MAIN: u8 = 98;
const JD_CONTENTS: u8 = 99;
/// The text (`#anon_contents_2`).
const JD_TEXT: u8 = 100;
/// Below the block tags, to the next line that starts with `@` (`#anon_contents_0`, `_3`).
const JD_HOLD: u8 = 101;
const JD_TAGS: u8 = 102;
/// A block tag's text (`javadoc-block-tag-base`).
const JD_TAG: u8 = 103;
const JD_PARAM_NAME: u8 = 104;
const JD_SEE: u8 = 105;
const JD_REF: u8 = 106;
const JD_REF_AFTER: u8 = 107;
const JD_REF_PARENS: u8 = 108;
/// The same in an inline tag, where `}` ends them.
const JD_IREF: u8 = 109;
const JD_IREF_AFTER: u8 = 110;
const JD_IREF_PARENS: u8 = 111;
/// `{@code …}`'s text.
const JD_RAW: u8 = 112;
const JD_BRACES: u8 = 113;
/// An inline tag, to its `}`.
const JD_INLINE: u8 = 114;
const JD_LINK_LABEL: u8 = 115;
const JD_LABEL: u8 = 116;
// HTML's, in Javadoc
/// A tag that `>` ends.
const H_TAG: u8 = 117;
/// A tag that `>` or `/>` ends.
const H_TAG_SC: u8 = 118;
const H_OTHER_NAME: u8 = 119;
const H_CUSTOM_NAME: u8 = 120;
const H_ATTR_NAME: u8 = 121;
const H_EQ: u8 = 122;
const H_VALUE: u8 = 123;
const H_DQ: u8 = 124;
const H_SQ: u8 = 125;
const H_UNQ: u8 = 126;
const H_COMMENT: u8 = 127;
const H_CDATA: u8 = 128;
/// `<?xml … ?>`.
const H_PI: u8 = 129;
const DT_NAME: u8 = 130;
const DT_TYPE: u8 = 131;
const DT_CONTENT: u8 = 132;
const DT_META: u8 = 133;
const DT_SUBSET: u8 = 134;
/// `style=` and `on…=`, whose values embed CSS and JavaScript (not followed: their text is
/// the comment's).
const H_EMBED_EQ: u8 = 135;
const H_EMBED_VALUE: u8 = 136;
const H_EMBED_DQ: u8 = 137;
const H_EMBED_SQ: u8 = 138;
/// `<style …>` and `<script …>`, then their text to the closing tag.
const H_EMBED_TAG: u8 = 139;
const H_EMBED_TEXT: u8 = 140;
/// A string that holds a regex (ADR-40, phase 4), the regex's state in [`Scan::re`].
const STRING_RE: u8 = 141;
// SQL by call sites and markers (ADR-40, phase 4): a text block, and the strings whose text is
// SQL, by SQL's context ([`PARENS_SQL`] and the annotations' are frames).
/// `"""…"""`, a text block (Java 15), which bat's grammar doesn't know: it read `""` and a
/// string to the line's end, and the block's lines as code.
const TEXT_BLOCK: u8 = 146;
/// A string whose text is SQL, plus SQL's context there ([`crate::sql`]'s `MAIN` to `HASHES`;
/// its line comments end with the line): `"…"`, then a text block.
const SQL_DQ: u8 = 160;
const SQL_DQ_LAST: u8 = SQL_DQ + sql::HASHES;
const SQL_TB: u8 = SQL_DQ_LAST + 1;
const SQL_TB_LAST: u8 = SQL_TB + sql::HASHES;

/// Bits of a kind past its leaf: the HTML string or comment that holds it in Javadoc.
const WRAP: u16 = 1 << 8;
/// In Javadoc's text.
const W_JD: u16 = WRAP;
const W_DQ: u16 = 2 * WRAP;
const W_SQ: u16 = 3 * WRAP;
const W_UNQ: u16 = 4 * WRAP;
const W_HCOMMENT: u16 = 5 * WRAP;
const W_ATTR: u16 = 6 * WRAP;
/// In a regex in a string: the leaf is the regex's kind.
const W_RE: u16 = 7 * WRAP;
const _: () = assert!(regex::KINDS <= WRAP as usize);
/// SQL in a `"…"` string, then in a text block: the leaf is SQL's kind.
const W_SQL: u16 = 8 * WRAP;
const W_SQL_TB: u16 = 9 * WRAP;
const _: () = assert!(sql::KINDS <= WRAP as usize);
/// In an annotation (`meta.annotation`).
const ANNOT: u16 = 16 * WRAP;

macro_rules! leaves {
    ($($name:ident = $s:literal,)*) => {
        /// The leaves: a token's scopes, under what [`WRAP`] adds.
        static LEAVES: &[&str] = &[$($s,)*];
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[repr(u16)]
        enum Leaf { $($name,)* }
        $(const $name: u16 = Leaf::$name as u16;)*
    };
}

leaves! {
    PLAIN = "",
    LINE_TEXT = "comment.line.double-slash.java",
    LINE_MARK = "comment.line.double-slash.java punctuation.definition.comment.java",
    BLOCK_TEXT = "comment.block.java",
    BLOCK_MARK = "comment.block.java punctuation.definition.comment.java",
    EMPTY_COMMENT = "comment.block.empty.java punctuation.definition.comment.java",
    ILLEGAL_KEYWORD = "invalid.illegal.keyword.java",
    DQ = "string.quoted.double.java",
    DQ_BEGIN = "string.quoted.double.java punctuation.definition.string.begin.java",
    DQ_END = "string.quoted.double.java punctuation.definition.string.end.java",
    DQ_ESCAPE = "string.quoted.double.java constant.character.escape.java",
    TB = "string.quoted.triple.java",
    TB_BEGIN = "string.quoted.triple.java punctuation.definition.string.begin.java",
    TB_END = "string.quoted.triple.java punctuation.definition.string.end.java",
    TB_ESCAPE = "string.quoted.triple.java constant.character.escape.java",
    SQ ="string.quoted.single.java",
    SQ_BEGIN = "string.quoted.single.java punctuation.definition.string.begin.java",
    SQ_END = "string.quoted.single.java punctuation.definition.string.end.java",
    SQ_ESCAPE = "string.quoted.single.java constant.character.escape.java",
    LANGUAGE = "constant.language.java",
    VAR_LANGUAGE = "variable.language.java",
    HEX_FLOAT = "constant.numeric.float.hexadecimal.java",
    HEX_FLOAT_PREFIX = "constant.numeric.float.hexadecimal.java punctuation.definition.numeric.hexadecimal.java",
    HEX_FLOAT_INVALID = "constant.numeric.float.hexadecimal.java invalid.illegal.numeric.java",
    HEX_FLOAT_DOT = "constant.numeric.float.hexadecimal.java punctuation.separator.decimal.java",
    FLOAT = "constant.numeric.float.decimal.java",
    FLOAT_INVALID = "constant.numeric.float.decimal.java invalid.illegal.numeric.java",
    FLOAT_DOT = "constant.numeric.float.decimal.java punctuation.separator.decimal.java",
    FLOAT_SUFFIX = "constant.numeric.float.decimal.java storage.type.numeric.java",
    BIN = "constant.numeric.integer.binary.java",
    BIN_PREFIX = "constant.numeric.integer.binary.java punctuation.definition.numeric.binary.java",
    BIN_INVALID = "constant.numeric.integer.binary.java invalid.illegal.numeric.java",
    BIN_SUFFIX = "constant.numeric.integer.binary.java storage.type.numeric.java",
    HEX = "constant.numeric.integer.hexadecimal.java",
    HEX_PREFIX = "constant.numeric.integer.hexadecimal.java punctuation.definition.numeric.hexadecimal.java",
    HEX_INVALID = "constant.numeric.integer.hexadecimal.java invalid.illegal.numeric.java",
    HEX_SUFFIX = "constant.numeric.integer.hexadecimal.java storage.type.numeric.java",
    OCT = "constant.numeric.integer.octal.java",
    OCT_PREFIX = "constant.numeric.integer.octal.java punctuation.definition.numeric.octal.java",
    OCT_INVALID = "constant.numeric.integer.octal.java invalid.illegal.numeric.java",
    OCT_SUFFIX = "constant.numeric.integer.octal.java storage.type.numeric.java",
    DEC = "constant.numeric.integer.decimal.java",
    DEC_INVALID = "constant.numeric.integer.decimal.java invalid.illegal.numeric.java",
    DEC_SUFFIX = "constant.numeric.integer.decimal.java storage.type.numeric.java",
    ASSIGN = "keyword.operator.assignment.java",
    TERNARY = "keyword.operator.ternary.java",
    INSTANCEOF = "keyword.operator.word.instanceof.java",
    BITSHIFT = "keyword.operator.bitshift.java",
    COMPARISON = "keyword.operator.comparison.java",
    INCDEC = "keyword.operator.increment-decrement.java",
    ARITH = "keyword.operator.arithmetic.java",
    LOGICAL = "keyword.operator.logical.java",
    BITWISE = "keyword.operator.bitwise.java",
    WILDCARD = "keyword.operator.wildcard.java",
    ASTERISK = "keyword.operator.wildcard.asterisk.java",
    BOUNDS_AND = "keyword.operator.multiple-bounds.java",
    VARIADIC = "keyword.operator.variadic.java",
    DOT = "punctuation.accessor.dot.java",
    DOUBLE_COLON = "punctuation.accessor.double-colon.java",
    COMMA = "punctuation.separator.comma.java",
    TERMINATOR = "punctuation.terminator.java",
    PAREN_BEGIN = "punctuation.section.parens.begin.java",
    PAREN_END = "punctuation.section.parens.end.java",
    BRACKET_BEGIN = "punctuation.section.brackets.begin.java",
    BRACKET_END = "punctuation.section.brackets.end.java",
    BRACE_BEGIN = "punctuation.section.braces.begin.java",
    BRACE_END = "punctuation.section.braces.end.java",
    BLOCK_BEGIN = "punctuation.section.block.begin.java",
    BLOCK_END = "punctuation.section.block.end.java",
    GENERIC_BEGIN = "punctuation.definition.generic.begin.java",
    GENERIC_END = "punctuation.definition.generic.end.java",
    DIAMOND = "punctuation.definition.generic.diamond.java",
    BAR = "punctuation.separator.bar.java",
    EXPR_SEP = "punctuation.separator.expressions.java",
    ANNOT_MARK = "punctuation.definition.annotation.java",
    MODIFIER = "storage.modifier.java",
    ARRAY_MOD = "storage.modifier.array.java",
    PRIMITIVE = "storage.type.primitive.java",
    VOID = "storage.type.void.java",
    STORAGE_TYPE = "storage.type.java",
    STORAGE_TYPE_MARK = "storage.type.java punctuation.definition.type.java",
    VAR = "storage.type.var.java",
    ARROW = "storage.type.function.anonymous.java",
    NEW = "keyword.other.storage.new.java",
    CATCH_KW = "keyword.control.exception.catch.java",
    FINALLY_KW = "keyword.control.exception.finally.java",
    TRY_KW = "keyword.control.exception.try.java",
    ASSERT_KW = "keyword.control.flow.assert.java",
    BREAK_KW = "keyword.control.flow.break.java",
    CONTINUE_KW = "keyword.control.flow.continue.java",
    RETURN_KW = "keyword.control.flow.return.java",
    THROW_KW = "keyword.control.flow.throw.java",
    IF_KW = "keyword.control.conditional.if.java",
    ELSE_KW = "keyword.control.conditional.else.java",
    SWITCH_KW = "keyword.control.conditional.switch.java",
    CASE_KW = "keyword.control.conditional.case.java",
    DEFAULT_KW = "keyword.control.conditional.default.java",
    DO_KW = "keyword.control.loop.do-while.java",
    FOR_KW = "keyword.control.loop.for.java",
    WHILE_KW = "keyword.control.loop.while.java",
    IMPORT_KW = "keyword.control.import.java",
    STATIC_KW = "keyword.control.import.static.java",
    PACKAGE_KW = "keyword.other.package.java",
    EXTENDS_KW = "keyword.declaration.extends.java",
    IMPLEMENTS_KW = "keyword.declaration.implements.java",
    SUPER_KW = "keyword.declaration.super.java",
    THROWS_KW = "keyword.declaration.throws.java",
    DEFAULT_DECL = "keyword.declaration.default.java",
    EXPORTS_KW = "keyword.other.module.exports.java",
    OPENS_KW = "keyword.other.module.opens.java",
    REQUIRES_KW = "keyword.other.module.requires.java",
    USES_KW = "keyword.other.module.uses.java",
    PROVIDES_KW = "keyword.other.module.provides.java",
    TO_KW = "keyword.other.module.to.java",
    WITH_KW = "keyword.other.module.with.java",
    TRANSITIVE_KW = "keyword.other.module.transitive.java",
    FUNCTION_CALL = "variable.function.java",
    FUNCTION_REF = "variable.function.reference.java",
    PARAMETER = "variable.parameter.java",
    TYPE_PARAMETER = "variable.parameter.type.java",
    ANNOTATION = "variable.annotation.java",
    ANNOTATION_PKG = "variable.annotation.package.java",
    CLASS = "support.class.java",
    IMPORT_CLASS = "support.class.import.java",
    PACKAGE = "support.type.package.java",
    PACKAGE_DOT = "support.type.package.java punctuation.accessor.dot.java",
    IMPORT_FUNCTION = "support.function.import.java",
    CONSTANT = "constant.other.java",
    IMPORT_CONSTANT = "constant.other.import.java",
    ENUM_NAME = "constant.other.enum.java",
    ENTITY_CLASS = "entity.name.class.java",
    ENTITY_FUNCTION = "entity.name.function.java",
    ENTITY_CTOR = "entity.name.function.constructor.java",
    ENTITY_CONSTANT = "entity.name.constant.java",
    NAMESPACE = "entity.name.namespace.java",
    NAMESPACE_DOT = "entity.name.namespace.java punctuation.accessor.dot.java",
    INHERITED = "entity.other.inherited-class.java",
    INHERITED_PKG = "entity.other.inherited-class.package.java",
    MODULE_NAME = "entity.name.module.java",
    MODULE_DOT = "entity.name.module.java punctuation.accessor.dot.java",
    SUPPORT_MODULE = "support.type.module.java",
    SUPPORT_MODULE_DOT = "support.type.module.java punctuation.accessor.dot.java",
    STRAY_TERMINATOR = "invalid.illegal.stray-terminator-end",
    STRAY_BRACE = "invalid.illegal.stray-brace-end",
    STRAY_PAREN = "invalid.illegal.stray-parens-end",
    MISSING_PARAM_END = "invalid.illegal.missing-parameter-end",
    // Javadoc's, under `W_JD` but the comment's ends
    JD_BEGIN = "comment.block.documentation.javadoc punctuation.definition.comment.begin.javadoc",
    JD_END = "comment.block.documentation.javadoc punctuation.definition.comment.end.javadoc",
    JD_STAR = "punctuation.definition.comment.javadoc",
    TAG_PARAM = "keyword.other.documentation.param.javadoc",
    TAG_PARAM_AT = "keyword.other.documentation.param.javadoc punctuation.definition.keyword.javadoc",
    TAG_SEE = "keyword.other.documentation.see.javadoc",
    TAG_SEE_AT = "keyword.other.documentation.see.javadoc punctuation.definition.keyword.javadoc",
    TAG_THROWS = "keyword.other.documentation.throws.javadoc",
    TAG_THROWS_AT = "keyword.other.documentation.throws.javadoc punctuation.definition.keyword.javadoc",
    TAG_USES = "keyword.other.documentation.uses-or-provides.javadoc",
    TAG_USES_AT = "keyword.other.documentation.uses-or-provides.javadoc punctuation.definition.keyword.javadoc",
    TAG_DOC = "keyword.other.documentation.javadoc",
    TAG_DOC_AT = "keyword.other.documentation.javadoc punctuation.definition.keyword.javadoc",
    TAG_CODE = "keyword.other.documentation.code-or-literal.javadoc",
    TAG_CODE_AT = "keyword.other.documentation.code-or-literal.javadoc punctuation.definition.keyword.javadoc",
    TAG_LINK = "keyword.other.documentation.link.javadoc",
    TAG_LINK_AT = "keyword.other.documentation.link.javadoc punctuation.definition.keyword.javadoc",
    TAG_VALUE = "keyword.other.documentation.value.javadoc",
    TAG_VALUE_AT = "keyword.other.documentation.value.javadoc punctuation.definition.keyword.javadoc",
    INLINE_BEGIN = "punctuation.section.inline-tag.begin.javadoc",
    INLINE_END = "punctuation.section.inline-tag.end.javadoc",
    JD_PARAM = "variable.parameter.javadoc",
    RAW = "markup.raw.javadoc",
    LINK = "markup.underline.link.javadoc",
    // HTML's, under `W_JD` or a string's or a comment's wrap
    TAG_OPEN = "punctuation.definition.tag.begin.html",
    TAG_CLOSE = "punctuation.definition.tag.end.html",
    NAME_STRUCTURE = "entity.name.tag.structure.any.html",
    NAME_BLOCK = "entity.name.tag.block.any.html",
    NAME_FORM_BLOCK = "entity.name.tag.block.form.html",
    NAME_INLINE = "entity.name.tag.inline.any.html",
    NAME_FORM = "entity.name.tag.inline.form.html",
    NAME_A = "entity.name.tag.inline.a.html",
    NAME_TABLE = "entity.name.tag.inline.table.html",
    NAME_OTHER = "entity.name.tag.other.html",
    NAME_CUSTOM = "entity.name.tag.custom.html",
    NAME_CUSTOM_BAD = "entity.name.tag.custom.html invalid.illegal.custom-tag-name.html",
    NAME_STYLE = "entity.name.tag.style.html",
    NAME_SCRIPT = "entity.name.tag.script.html",
    NAME_XML = "entity.name.tag.xml.html",
    ATTR_ID = "entity.other.attribute-name.id.html",
    ATTR_CLASS = "entity.other.attribute-name.class.html",
    ATTR_STYLE = "entity.other.attribute-name.style.html",
    ATTR_EVENT = "entity.other.attribute-name.event.html",
    BAD_NAME = "invalid.illegal.attribute-name.html",
    KEY_VALUE = "punctuation.separator.key-value.html",
    STR_BEGIN = "punctuation.definition.string.begin.html",
    STR_END = "punctuation.definition.string.end.html",
    BAD_VALUE = "invalid.illegal.attribute-value.html",
    EMBED_DQ_BEGIN = "string.quoted.double punctuation.definition.string.begin.html",
    EMBED_DQ_END = "string.quoted.double punctuation.definition.string.end.html",
    EMBED_SQ_BEGIN = "string.quoted.single punctuation.definition.string.begin.html",
    EMBED_SQ_END = "string.quoted.single punctuation.definition.string.end.html",
    ENT_NAMED = "constant.character.entity.named.html",
    ENT_NAMED_MARK = "constant.character.entity.named.html punctuation.definition.entity.html",
    ENT_NAMED_END = "constant.character.entity.named.html punctuation.terminator.entity.html",
    ENT_DEC = "constant.character.entity.decimal.html",
    ENT_DEC_MARK = "constant.character.entity.decimal.html punctuation.definition.entity.html",
    ENT_DEC_END = "constant.character.entity.decimal.html punctuation.terminator.entity.html",
    ENT_HEX = "constant.character.entity.hexadecimal.html",
    ENT_HEX_MARK = "constant.character.entity.hexadecimal.html punctuation.definition.entity.html",
    ENT_HEX_END = "constant.character.entity.hexadecimal.html punctuation.terminator.entity.html",
    HCOMMENT_BEGIN = "punctuation.definition.comment.begin.html",
    HCOMMENT_END = "punctuation.definition.comment.end.html",
    HCOMMENT_BAD = "invalid.illegal.bad-comments-or-CDATA.html",
    CDATA_KW = "keyword.declaration.cdata.html",
    CDATA_TEXT = "string.unquoted.cdata.html",
    DOCTYPE_KW = "keyword.declaration.doctype.html",
    DOCTYPE_NAME = "constant.language.doctype.html",
    DOCTYPE_EXTERNAL = "keyword.content.external.html",
    HBRACKET_BEGIN = "punctuation.section.brackets.begin.html",
    HBRACKET_END = "punctuation.section.brackets.end.html",
    INCOMPLETE = "invalid.illegal.incomplete.html",
}

/// The scopes under `source.java` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    let mut out = Vec::new();
    if kind & ANNOT != 0 {
        out.push("meta.annotation.java");
    }
    let wrap = (kind & (ANNOT - WRAP)) / WRAP;
    if wrap * WRAP == W_RE {
        out.push("string.quoted.double.java");
        out.extend(regex::scopes(kind % WRAP));
        return out;
    }
    if wrap * WRAP == W_SQL || wrap * WRAP == W_SQL_TB {
        let k = kind % WRAP;
        if usize::from(k) >= sql::KINDS {
            return Vec::new();
        }
        out.push(if wrap * WRAP == W_SQL {
            "string.quoted.double.java"
        } else {
            "string.quoted.triple.java"
        });
        out.extend(sql::scopes(k));
        return out;
    }
    if wrap != 0 {
        out.extend(["comment.block.documentation.javadoc", "text.html.javadoc"]);
        out.extend(match wrap * WRAP {
            W_DQ => Some("string.quoted.double.html"),
            W_SQ => Some("string.quoted.single.html"),
            W_UNQ => Some("string.unquoted.html"),
            W_HCOMMENT => Some("comment.block.html"),
            W_ATTR => Some("entity.other.attribute-name.html"),
            _ => None,
        });
    }
    if let Some(leaf) = LEAVES.get(usize::from(kind % WRAP)) {
        out.extend(leaf.split(' ').filter(|s| !s.is_empty()));
    }
    out
}

/// By context: the kind of its text, plus [`COUNTED`] for one that holds an annotation open
/// (`meta.annotation`): one lookup where a `match` grew with each context.
static CTX: [u16; 256] = {
    let mut t = [PLAIN; 256];
    let mut i = JD_MAIN as usize;
    while i < STRING_RE as usize {
        t[i] = W_JD;
        i += 1;
    }
    let mut i = SQL_DQ as usize;
    while i <= SQL_DQ_LAST as usize {
        t[i] = DQ;
        t[i - SQL_DQ as usize + SQL_TB as usize] = TB;
        i += 1;
    }
    t[STRING as usize] = DQ;
    t[STRING_RE as usize] = DQ;
    t[TEXT_BLOCK as usize] = TB;
    t[CHAR as usize] = SQ;
    t[COMMENT as usize] = BLOCK_TEXT;
    t[LINE_COMMENT as usize] = LINE_TEXT;
    t[H_OTHER_NAME as usize] = W_JD | NAME_OTHER;
    t[H_CUSTOM_NAME as usize] = W_JD | NAME_CUSTOM;
    t[H_ATTR_NAME as usize] = W_ATTR;
    t[H_DQ as usize] = W_DQ;
    t[H_SQ as usize] = W_SQ;
    t[H_UNQ as usize] = W_UNQ;
    t[H_COMMENT as usize] = W_HCOMMENT;
    t[H_CDATA as usize] = W_JD | CDATA_TEXT;
    t[ANNOT_PARAMS as usize] |= COUNTED;
    t[ANNOT_ARGS as usize] |= COUNTED;
    t[ANNOT_DEFAULT as usize] |= COUNTED;
    t[ANNOT_PARAMS_SQL as usize] |= COUNTED;
    t[ANNOT_ARGS_SQL as usize] |= COUNTED;
    t
};
const COUNTED: u16 = 1 << 15;

/// By context of bat's patterns, what the scan may pass without trying them: [`BLANKS`].
static PASSES: [u8; 256] = {
    let mut t = [0; 256];
    // those within a declaration (below `MODULE_KW`; the module's contexts not checked): no
    // pattern starts at a blank, none matches at the line's end
    let mut i = FRAMES as usize;
    while i < MODULE_KW as usize {
        t[i] = BLANKS | QUIET_EOL;
        i += 1;
    }
    t
};
/// A run of blanks matches nothing.
const BLANKS: u8 = 1;
/// At the line's end nothing matches ([`Scan::eol`]'s `None`).
const QUIET_EOL: u8 = 4;

/// How many contexts a line can hold; one pushed past it drops the bottom one.
const MAX: usize = 128;

/// Lex `b`, a line without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let mut s = Scan::fresh(b, out, Push);
    s.load_state(state);
    s.lex();
    *state = s.state();
}

/// [`line`]'s runs by theme rule pushed to `runs` (X-074, X-068's sink): [`crate::lex_runs`] for
/// Java.
#[inline(never)]
pub fn line_runs(
    b: &[u8],
    state: &mut State,
    out: &mut Out,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
) {
    let mut s = Scan::fresh(b, out, Runs::new(rules, runs));
    s.load_state(state);
    s.lex();
    *state = s.state();
    s.k.end_line(b.len());
}

/// A block's lines on one scan opened from `state` ([`crate::Carry`], X-067), `state` left as
/// the last one leaves it.
pub fn block(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    let mut out = Out::new(spans);
    let mut s = Scan::fresh(b"", &mut out, Push);
    s.load_state(state);
    let rest = pass(&mut s);
    *state = s.state();
    rest
}

/// [`block`]'s runs by theme rule pushed to `runs` ([`crate::CarryRuns`], X-074).
pub fn block_runs(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    lines: &mut crate::RunLines,
) -> Option<usize> {
    let mut out = Out::new(spans);
    out.0.clear();
    let mut s = Scan::fresh(b"", &mut out, Runs::new(rules, runs));
    s.load_state(state);
    let rest = lines.pass(&mut s);
    *state = s.state();
    rest
}

impl crate::Carry for Scan<'_, '_, Push> {
    fn next(&mut self, b: &[u8]) -> &[crate::Span] {
        self.out.0.clear();
        // SAFETY: the line is set back to an empty one below, before `b`'s borrow ends
        self.carry(unsafe { crate::line_for(b) });
        self.out.0
    }
}

impl crate::CarryRuns for Scan<'_, '_, Runs<'_>> {
    fn next(&mut self, b: &[u8]) -> usize {
        self.k.start_line();
        // SAFETY: as above
        self.carry(unsafe { crate::line_for(b) });
        self.k.end_line(b.len())
    }
}

impl<'b, 'c, K: Sink> Scan<'b, 'c, K> {
    /// `b`, the block's next line, lexed on this scan; the scan's line an empty one again after.
    #[inline(always)]
    fn carry(&mut self, b: &'b [u8]) {
        self.b = b;
        self.line_start();
        self.lex();
        // the line's end as a state's save and load leave it (`line` leaves that to its state)
        if self.depth - 1 > crate::stack::HELD {
            self.reload();
        }
        self.b = b"";
    }

    /// A scan of `b`, returned as a literal so that it is built in place; the state is
    /// [`Scan::load_state`]'s.
    #[inline(always)]
    fn fresh(b: &'b [u8], out: &'b mut Out<'c>, k: K) -> Self {
        Scan {
            b,
            stack: [TOP; MAX],
            depth: 1,
            annot: 0,
            tags: 0,
            text: PLAIN,
            at: 0,
            loop_at: (usize::MAX, 0),
            halt: false,
            enum_checked: false,
            arrow: None,
            tag_line: None,
            word_at: std::cell::Cell::new((usize::MAX, 0)),
            kind_at: std::cell::Cell::new((usize::MAX, Word::Other)),
            class_at: std::cell::Cell::new((usize::MAX, 0, false)),
            fqn: std::cell::Cell::new((usize::MAX, 0, false)),
            re: 0,
            sql: sql::Embedded {
                is_end: usize::MAX,
                at: 0,
            },
            sql_end: usize::MAX,
            sql_nl: false,
            mark: usize::MAX,
            k,
            out,
        }
    }

    /// What [`Scan::fresh`] leaves to `state`: the stack and what is read from it, apart from
    /// the literal, which a constructor that changed it before returning it copied (a `memcpy`
    /// a line, X-056's trap).
    #[inline(always)]
    fn load_state(&mut self, state: &State) {
        self.load(state);
    }

    /// The stack, its counts, and what else a line hands the next, from `state`.
    #[inline(always)]
    fn load(&mut self, state: &State) {
        (self.annot, self.tags) = (0, 0);
        let n = usize::from(state.depth);
        for i in 0..n {
            let c = state.level(i);
            self.stack[1 + i] = c;
            self.count_in(c);
        }
        self.depth = 1 + n;
        self.mark = if state.flags() & MARKED != 0 {
            0
        } else {
            usize::MAX
        };
    }

    /// What a line starts with, on a scan kept from the line before.
    #[inline(always)]
    fn line_start(&mut self) {
        self.text = PLAIN;
        self.at = 0;
        self.loop_at = (usize::MAX, 0);
        self.halt = false;
        self.enum_checked = false;
        self.arrow = None;
        self.tag_line = None;
        self.word_at.set((usize::MAX, 0));
        self.kind_at.set((usize::MAX, Word::Other));
        self.class_at.set((usize::MAX, 0, false));
        self.fqn.set((usize::MAX, 0, false));
        self.re = 0;
        self.sql = sql::Embedded {
            is_end: usize::MAX,
            at: 0,
        };
        self.sql_end = usize::MAX;
        self.sql_nl = false;
    }

    /// The line lexed from the stack it starts with, a SQL marker that still holds kept for the
    /// next line.
    #[inline(always)]
    fn lex(&mut self) {
        let b = self.b;
        self.refresh();
        self.run();
        self.fill(b.len());
        let marked = self.mark != usize::MAX && sql::marker_holds(b, self.mark, b.len());
        self.mark = if marked { 0 } else { usize::MAX };
    }

    /// The scan as a state's save and load leave it, its stack deeper than a state holds.
    #[cold]
    #[inline(never)]
    fn reload(&mut self) {
        let state = self.state();
        self.load(&state);
    }

    /// The state the scan's last line leaves.
    #[inline(always)]
    fn state(&self) -> State {
        let flags = if self.mark != usize::MAX { MARKED } else { 0 };
        State::keep(&self.stack, self.depth, flags)
    }
}

/// [`State::flags`]' bit: a SQL marker (`@Language("SQL")`, `// language=SQL`) waits for its
/// string.
const MARKED: u16 = 1;

struct Scan<'b, 'c, K: Sink> {
    b: &'b [u8],
    stack: [u8; MAX],
    depth: usize,
    /// How many annotations are open.
    annot: usize,
    /// How many Javadoc block tags are open (0 or 1).
    tags: usize,
    /// The kind of text no pattern matches.
    text: u16,
    /// Where the spans so far end.
    at: usize,
    /// Where the last push that matched nothing was, and the depth its first context left: a
    /// pop there, at that depth, that matches nothing would loop, so syntect tries the patterns
    /// after it and else steps a character.
    loop_at: (usize, usize),
    /// A loop at the line's end: nothing more to match.
    halt: bool,
    /// Whether an enum's body was asked at the line's start for a line of constants.
    enum_checked: bool,
    /// The last `)` followed by `\s*->`, once looked for.
    arrow: Option<Option<usize>>,
    /// Whether the line is `^\s*\*?\s*@`, once looked at.
    tag_line: Option<bool>,
    /// The last whole word looked up: where it starts and ends.
    word_at: std::cell::Cell<(usize, usize)>,
    /// The last word looked up among the keywords: where it starts, and its class.
    kind_at: std::cell::Cell<(usize, Word)>,
    /// The last chain [`Scan::class_ahead`] read: where it started, the start of its last word,
    /// and its answer, the same at each of its modifiers.
    class_at: std::cell::Cell<(usize, usize, bool)>,
    /// The last chain [`Scan::fqn_ahead`] read: where it started, where it stopped, and its answer, the same for
    /// each of its words (a long `a.a.a.…` read again from each was quadratic: X-003).
    fqn: std::cell::Cell<(usize, usize, bool)>,
    /// The state of the regex in [`STRING_RE`], which ends with its line.
    re: u8,
    /// What SQL's scan keeps between the host's turns on a line.
    sql: sql::Embedded,
    /// Where the SQL string open on the line ends (its quote, or the line's end), past which
    /// no SQL token runs; `usize::MAX` till it is looked for.
    sql_end: usize,
    /// Whether SQL's scan took the line's end (`\s` matching its `\n`).
    sql_nl: bool,
    /// A SQL marker's: the next string that opens at or past it is SQL, unless a `;`, `,`, `)`,
    /// `{` or `}` came between ([`sql::marker_holds`], checked there and at the line's end);
    /// `usize::MAX` for none.
    mark: usize,
    /// Where the lexemes go, a line's spans or its runs by theme rule (X-074, X-068's sink).
    k: K,
    /// The spans' vector: the span API's, and where an embedded scan pushes its spans for the
    /// run sink to take.
    out: &'b mut Out<'c>,
}

/// The bytes a string's patterns start on, around a regex: its end and its escapes.
static RE_YIELDS: [bool; 256] = {
    let mut t = [false; 256];
    t[b'"' as usize] = true;
    t[b'\\' as usize] = true;
    t
};

/// Whether the string whose quote opens at `p` is a regex: right after `Pattern.compile(`,
/// `.matches(`, `.replaceAll(`, `.replaceFirst(` or `.split(` (their first argument), or
/// `regexp =` (Bean Validation's `@Pattern(regexp = …)`).
fn regex_string(b: &[u8], p: usize) -> bool {
    let q = trim_end(b, 0, p);
    let Some(&c) = q.checked_sub(1).map(|i| &b[i]) else {
        return false;
    };
    let w = trim_end(b, 0, q - 1);
    let named = |name: &[u8]| {
        b[..w].ends_with(name)
            && (name[0] == b'.' || w == name.len() || !word_before(b, w - name.len()))
    };
    match c {
        b'(' => [
            &b"Pattern.compile"[..],
            b".matches",
            b".replaceAll",
            b".replaceFirst",
            b".split",
        ]
        .into_iter()
        .any(named),
        b'=' => named(b"regexp"),
        _ => false,
    }
}

/// The bytes a string's patterns start on, around SQL: its end and its escapes.
static SQL_YIELDS: [bool; 256] = RE_YIELDS;

/// Where the string SQL is in ends, from `p` inside it: its `"` (a text block's `"""`), or the
/// line's end.
fn string_end(b: &[u8], mut p: usize, text_block: bool) -> usize {
    loop {
        p = find_any(b, p, *b"\"\\");
        if p >= b.len() {
            return b.len();
        }
        if b[p] == b'\\' {
            p += 2;
        } else if !text_block || b[p..].starts_with(b"\"\"\"") {
            return p;
        } else {
            p += 1;
        }
    }
}

/// The character after the one at `i`, which must be in the line (no end test: one cost Java
/// ~0.25% of its instructions).
#[inline]
fn next(b: &[u8], i: usize) -> usize {
    (i + char_len(b[i])).min(b.len())
}

/// The character byte `i` of `b` is in, and where it starts.
#[cold]
fn wide(b: &[u8], i: usize) -> char {
    crate::prim::wide_char_at(b, i).unwrap_or('\u{fffd}')
}

/// ASCII's classes, by byte: `\w`, an identifier's, past ASCII (to decode).
const W: u8 = 1;
const ID: u8 = 2;
const WIDE: u8 = 4;
static CLASSES: [u8; 256] = {
    let mut t = [0; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        t[c] = if b >= 0x80 {
            WIDE
        } else if b.is_ascii_alphanumeric() || b == b'_' {
            W | ID
        } else if b == b'$' {
            ID
        } else {
            0
        };
        c += 1;
    }
    t
};

/// `\w` at byte `i`.
#[inline]
fn is_word(b: &[u8], i: usize) -> bool {
    match b.get(i) {
        Some(&c) if c < 0x80 => CLASSES[usize::from(c)] & W != 0,
        Some(_) => wide(b, i).is_alphanumeric(),
        None => false,
    }
}

/// `\w` just before byte `i`.
#[inline]
fn word_before(b: &[u8], i: usize) -> bool {
    i > 0 && is_word(b, i - 1)
}

/// `\b` at `i`.
#[inline]
fn boundary(b: &[u8], i: usize) -> bool {
    word_before(b, i) != is_word(b, i)
}

/// `[\p{L}\p{N}_$]` at `i`: an identifier's byte.
#[inline]
fn is_ident(b: &[u8], i: usize) -> bool {
    match b.get(i) {
        Some(&c) if c < 0x80 => c.is_ascii_alphanumeric() || c == b'_' || c == b'$',
        Some(_) => {
            let c = wide(b, i);
            c.is_alphabetic() || c.is_numeric()
        }
        None => false,
    }
}

/// `[\p{L}_$]` at `i`: where an identifier can start.
#[inline]
fn is_ident_start(b: &[u8], i: usize) -> bool {
    match b.get(i) {
        Some(&c) if c < 0x80 => c.is_ascii_alphabetic() || c == b'_' || c == b'$',
        Some(_) => wide(b, i).is_alphabetic(),
        None => false,
    }
}

/// `\p{Lu}` at `i`.
#[inline]
fn is_upper(b: &[u8], i: usize) -> bool {
    match b.get(i) {
        Some(&c) if c < 0x80 => c.is_ascii_uppercase(),
        Some(_) => wide(b, i).is_uppercase(),
        None => false,
    }
}

/// `\p{Ll}` at `i`.
#[inline]
fn is_lower(b: &[u8], i: usize) -> bool {
    match b.get(i) {
        Some(&c) if c < 0x80 => c.is_ascii_lowercase(),
        Some(_) => wide(b, i).is_lowercase(),
        None => false,
    }
}

/// `\p{N}` at `i`.
fn is_number(b: &[u8], i: usize) -> bool {
    match b.get(i) {
        Some(&c) if c < 0x80 => c.is_ascii_digit(),
        Some(_) => wide(b, i).is_numeric(),
        None => false,
    }
}

/// The character before `i`.
fn prev(b: &[u8], i: usize) -> usize {
    let mut j = i - 1;
    while j > 0 && b[j] & 0xc0 == 0x80 {
        j -= 1;
    }
    j
}

fn run_of(b: &[u8], mut p: usize, f: impl Fn(&[u8], usize) -> bool) -> usize {
    while p < b.len() && f(b, p) {
        p = next(b, p);
    }
    p
}

/// Where the run of `\w` at `p` ends.
fn word_end(b: &[u8], mut p: usize) -> usize {
    while let Some(&c) = b.get(p) {
        let k = CLASSES[usize::from(c)];
        if k & W != 0 {
            p += 1;
        } else if k & WIDE != 0 && is_word(b, p) {
            p = next(b, p);
        } else {
            break;
        }
    }
    p
}

/// Where the run of `[\p{L}\p{N}_$]` at `p` ends.
fn ident_end(b: &[u8], mut p: usize) -> usize {
    while let Some(&c) = b.get(p) {
        let k = CLASSES[usize::from(c)];
        if k & ID != 0 {
            p += 1;
        } else if k & WIDE != 0 && is_ident(b, p) {
            p = next(b, p);
        } else {
            break;
        }
    }
    p
}

/// `[\p{L}_$][\p{L}\p{N}_$]*` at `p`: where it ends.
fn ident(b: &[u8], p: usize) -> Option<usize> {
    is_ident_start(b, p).then(|| ident_end(b, next(b, p)))
}

/// The last `k` in `from..=to`, backwards over characters, with `\b` at `k` and `ok(k)`: where
/// a greedy run that `\b` follows ends.
fn back_to(b: &[u8], from: usize, to: usize, ok: impl Fn(usize) -> bool) -> Option<usize> {
    let mut k = to;
    loop {
        if boundary(b, k) && ok(k) {
            return Some(k);
        }
        if k <= from {
            return None;
        }
        k = prev(b, k);
    }
}

/// `[_$]*` then `first`, then `rest*`, at `p`: where `first` ends, and where the run does.
#[inline]
fn cased(
    b: &[u8],
    p: usize,
    first: impl Fn(&[u8], usize) -> bool,
    rest: impl Fn(&[u8], usize) -> bool,
) -> Option<(usize, usize)> {
    let q = run_of(b, p, |b, i| matches!(b[i], b'_' | b'$'));
    first(b, q).then(|| {
        let m = next(b, q);
        (m, run_of(b, m, rest))
    })
}

#[inline]
fn lower_rest(b: &[u8], i: usize) -> bool {
    match b[i] {
        c @ ..0x80 => c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'$',
        _ => is_lower(b, i) || is_number(b, i),
    }
}

#[inline]
fn upper_rest(b: &[u8], i: usize) -> bool {
    match b[i] {
        c @ ..0x80 => c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_' || c == b'$',
        _ => is_upper(b, i) || is_number(b, i),
    }
}

/// [`cased`]'s run for `is_lower` and `lower_rest` at `p`: where it ends. ASCII on bytes, else
/// by characters.
#[inline]
fn lower_run(b: &[u8], p: usize) -> Option<usize> {
    let mut i = p;
    while i < b.len() && matches!(b[i], b'_' | b'$') {
        i += 1;
    }
    match b.get(i) {
        Some(c) if c.is_ascii_lowercase() => i += 1,
        Some(&c) if c >= 0x80 => return cased(b, p, is_lower, lower_rest).map(|r| r.1),
        _ => return None,
    }
    while let Some(&c) = b.get(i) {
        if c >= 0x80 {
            return cased(b, p, is_lower, lower_rest).map(|r| r.1);
        }
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'$') {
            break;
        }
        i += 1;
    }
    Some(i)
}

/// `(?:[_$]*\p{Ll}[\p{Ll}\p{N}_$]*\b)` at `p`: a package's name.
fn lower_word(b: &[u8], p: usize) -> Option<usize> {
    // the run's end where `\b` is at it, the common case
    let e = lower_run(b, p)?;
    if boundary(b, e) {
        return Some(e);
    }
    let (m, e) = cased(b, p, is_lower, lower_rest)?;
    back_to(b, m, e, |_| true)
}

/// `(?:[_$]*\p{Lu}[\p{Lu}\p{N}_$]*\b)` at `p`: a constant's name.
fn upper_word(b: &[u8], p: usize) -> Option<usize> {
    let (m, e) = cased(b, p, is_upper, upper_rest)?;
    back_to(b, m, e, |_| true)
}

/// `(?=(?:[_$]*\p{Ll}[\p{Ll}\p{N}_$]*\b)\s*\.)` at `p`: a package before its `.`.
fn package_ahead(b: &[u8], p: usize) -> bool {
    lower_run(b, p).is_some_and(|e| boundary(b, e) && b.get(skip_spaces(b, e)) == Some(&b'.'))
}

/// `(?:\p{Lu}[\p{L}\p{N}_$]*)` at `p`: a type's name.
fn type_name(b: &[u8], p: usize) -> Option<usize> {
    is_upper(b, p).then(|| ident_end(b, next(b, p)))
}

/// Where the first byte from `from` that `stop` takes is, or the end.
fn next_of(b: &[u8], from: usize, stop: impl Fn(u8) -> bool) -> usize {
    b.get(from..)
        .and_then(|rest| rest.iter().position(|&c| stop(c)))
        .map_or(b.len(), |i| from + i)
}

/// The spaces of `[\t\n\f ]`, and the line's end, which is its `\n`.
fn is_html_space(b: &[u8], i: usize) -> bool {
    matches!(b.get(i), None | Some(b'\t' | b'\n' | b'\x0c' | b' '))
}

/// `[\t\n\f /<>]` at `i`, or the line's end: what ends a tag's name.
fn ends_tag_name(b: &[u8], i: usize) -> bool {
    is_html_space(b, i) || matches!(b[i], b'/' | b'<' | b'>')
}

/// What a whole word is to bat's grammar.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Word {
    Other,
    /// `true`, `false`, `null`.
    Language,
    /// `this`, `super`.
    Special,
    New,
    Modifier,
    Primitive,
    Void,
    Var,
    Instanceof,
    /// `goto`, `const`.
    Illegal,
    /// A keyword of `keywords-control`, its leaf.
    Control(u16),
}

/// The word `w` among bat's keywords, dispatching on its first byte.
fn lookup(w: &[u8]) -> Word {
    use Word::{
        Control as K, Illegal, Instanceof, Language as L, Modifier as M, New, Other,
        Primitive as P, Special as S, Var, Void,
    };
    let Some(&c) = w.first() else {
        return Other;
    };
    match c {
        b'a' => match w {
            b"abstract" => M,
            b"assert" => K(ASSERT_KW),
            _ => Other,
        },
        b'b' => match w {
            b"boolean" | b"byte" => P,
            b"break" => K(BREAK_KW),
            _ => Other,
        },
        b'c' => match w {
            b"char" => P,
            b"catch" => K(CATCH_KW),
            b"continue" => K(CONTINUE_KW),
            b"case" => K(CASE_KW),
            b"const" => Illegal,
            _ => Other,
        },
        b'd' => match w {
            b"double" => P,
            b"default" => M,
            b"do" => K(DO_KW),
            _ => Other,
        },
        b'e' => match w {
            b"else" => K(ELSE_KW),
            _ => Other,
        },
        b'f' => match w {
            b"false" => L,
            b"final" => M,
            b"float" => P,
            b"finally" => K(FINALLY_KW),
            b"for" => K(FOR_KW),
            _ => Other,
        },
        b'g' => match w {
            b"goto" => Illegal,
            _ => Other,
        },
        b'i' => match w {
            b"int" => P,
            b"if" => K(IF_KW),
            b"instanceof" => Instanceof,
            _ => Other,
        },
        b'l' => match w {
            b"long" => P,
            _ => Other,
        },
        b'n' => match w {
            b"null" => L,
            b"new" => New,
            b"native" => M,
            _ => Other,
        },
        b'p' => match w {
            b"public" | b"private" | b"protected" => M,
            _ => Other,
        },
        b'r' => match w {
            b"return" => K(RETURN_KW),
            _ => Other,
        },
        b's' => match w {
            b"super" => S,
            b"static" | b"synchronized" | b"strictfp" => M,
            b"short" => P,
            b"switch" => K(SWITCH_KW),
            _ => Other,
        },
        b't' => match w {
            b"true" => L,
            b"this" => S,
            b"transient" => M,
            b"try" => K(TRY_KW),
            b"throw" => K(THROW_KW),
            _ => Other,
        },
        b'v' => match w {
            b"void" => Void,
            b"var" => Var,
            b"volatile" => M,
            _ => Other,
        },
        b'w' => match w {
            b"while" => K(WHILE_KW),
            _ => Other,
        },
        _ => Other,
    }
}

/// The modifiers, in `(?:public|private|…)` without `\b`.
static MODIFIERS: [&[u8]; 12] = [
    b"public",
    b"private",
    b"protected",
    b"static",
    b"final",
    b"native",
    b"synchronized",
    b"strictfp",
    b"abstract",
    b"transient",
    b"default",
    b"volatile",
];

static PRIMITIVES: [&[u8]; 8] = [
    b"boolean", b"byte", b"char", b"short", b"int", b"float", b"long", b"double",
];

/// A number's parts, each bytes `s..e` of a leaf, the rest the number's base leaf.
#[derive(Default)]
struct Parts {
    n: usize,
    v: [(usize, usize, u16); 10],
}

impl Parts {
    fn add(&mut self, s: usize, e: usize, leaf: u16) {
        if e > s && self.n < self.v.len() {
            self.v[self.n] = (s, e, leaf);
            self.n += 1;
        }
    }
}

/// `(_*)D[D_]*?(_*)` at `q`, its digits `digit`: where its first digit is, where its trailing
/// `_`s start, where it ends.
fn digits(b: &[u8], q: usize, digit: fn(u8) -> bool) -> Option<(usize, usize, usize)> {
    let l = next_of(b, q, |c| c != b'_');
    if !b.get(l).is_some_and(|&c| digit(c)) {
        return None;
    }
    let e = next_of(b, l + 1, |c| !digit(c) && c != b'_');
    let mut t = e;
    while t > l + 1 && b[t - 1] == b'_' {
        t -= 1;
    }
    Some((l, t, e))
}

fn is_dec(c: u8) -> bool {
    c.is_ascii_digit()
}

fn is_hex(c: u8) -> bool {
    c.is_ascii_hexdigit()
}

/// A group of digits at `q` into `parts`, its `_`s at either end `invalid`: where it ends.
fn group(
    b: &[u8],
    q: usize,
    digit: fn(u8) -> bool,
    invalid: u16,
    parts: &mut Parts,
) -> Option<usize> {
    let (l, t, e) = digits(b, q, digit)?;
    parts.add(q, l, invalid);
    parts.add(t, e, invalid);
    Some(e)
}

/// An exponent at `q`, `[eE][-+]?` (or `[pP]`) then decimal digits: where it ends.
fn exponent(b: &[u8], q: usize, marks: [u8; 2], invalid: u16, parts: &mut Parts) -> Option<usize> {
    if !b.get(q).is_some_and(|c| marks.contains(c)) {
        return None;
    }
    let s = if matches!(b.get(q + 1), Some(b'-' | b'+')) {
        q + 2
    } else {
        q + 1
    };
    group(b, s, is_dec, invalid, parts)
}

/// `(?:E)? ([dDfF])? \b` at `q`, tried in a backtracking engine's order, with `E` an exponent
/// of `marks` (none without) and the suffix only if `suffixed`: where it ends.
fn tail(
    b: &[u8],
    q: usize,
    marks: Option<[u8; 2]>,
    suffixed: bool,
    invalid: u16,
    suffix: u16,
    parts: &mut Parts,
) -> Option<usize> {
    let n = parts.n;
    if let Some(m) = marks
        && let Some(x) = exponent(b, q, m, invalid, parts)
    {
        if let Some(e) = ends(b, x, suffixed, suffix, parts) {
            return Some(e);
        }
        parts.n = n;
    }
    ends(b, q, suffixed, suffix, parts)
}

/// `([dDfF])?\b` at `q`.
fn ends(b: &[u8], q: usize, suffixed: bool, suffix: u16, parts: &mut Parts) -> Option<usize> {
    if suffixed && matches!(b.get(q), Some(b'd' | b'D' | b'f' | b'F')) && boundary(b, q + 1) {
        parts.add(q, q + 1, suffix);
        return Some(q + 1);
    }
    boundary(b, q).then_some(q)
}

impl<'b, K: Sink> Scan<'b, '_, K> {
    fn top(&self) -> u8 {
        self.stack[self.depth - 1]
    }

    /// `k` here: in an annotation, its variant.
    fn kind(&self, k: u16) -> u16 {
        if self.annot > 0 { k | ANNOT } else { k }
    }

    /// Bytes `s..e` are `leaf`, after the text before them.
    #[inline]
    fn put(&mut self, s: usize, e: usize, leaf: u16) {
        self.fill(s);
        let k = self.kind(leaf);
        if k != PLAIN && e > s {
            self.k.merge(&mut self.out, s, e, k);
        }
        self.at = self.at.max(e);
    }

    /// The text up to `p` is the context's.
    fn fill(&mut self, p: usize) {
        if p > self.at {
            if self.text != PLAIN {
                self.k.merge(&mut self.out, self.at, p, self.text);
            }
            self.at = p;
        }
    }

    fn refresh(&mut self) {
        self.text = self.kind(CTX[usize::from(self.top())] & !COUNTED);
    }

    fn count_in(&mut self, c: u8) {
        self.annot += usize::from(CTX[usize::from(c)] >> 15);
        self.tags += usize::from(c == JD_TAGS);
    }

    fn count_out(&mut self, c: u8) {
        self.annot -= usize::from(CTX[usize::from(c)] >> 15);
        self.tags -= usize::from(c == JD_TAGS);
    }

    fn push(&mut self, at: usize, c: u8) {
        self.fill(at);
        if self.depth == MAX {
            let gone = self.stack[1];
            self.count_out(gone);
            crate::stack::drop_bottom(&mut self.stack, &mut self.depth);
        }
        self.stack[self.depth] = c;
        self.depth += 1;
        self.count_in(c);
        self.refresh();
    }

    fn pop(&mut self, at: usize) {
        self.fill(at);
        if self.depth > 1 {
            self.depth -= 1;
            let c = self.stack[self.depth];
            self.count_out(c);
        }
        self.refresh();
    }

    /// Replace the top with `cs`, bottom first.
    fn set(&mut self, at: usize, cs: &[u8]) {
        self.fill(at);
        let top = self.depth - 1;
        let c = self.stack[top];
        self.count_out(c);
        self.depth -= 1;
        for &c in cs {
            self.push(at, c);
        }
    }

    /// A lookahead's pop at `p`; where it would loop, a step past the character.
    fn leave(&mut self, p: usize) -> usize {
        if self.loop_at == (p, self.depth) {
            if p == self.b.len() {
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

    /// `n` bytes at `p`, `leaf`, that open `c`.
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
        self.set(p, cs);
        self.token(p, n, leaf)
    }

    /// `any_POP`, `(?=\S)`, at `p`.
    fn any_pop(&mut self, p: usize) -> Option<usize> {
        (!is_space(self.b[p])).then(|| self.leave(p))
    }

    /// The token at `p` to `e`, its parts as `parts` gives them and the rest `base`.
    fn emit(&mut self, p: usize, e: usize, base: u16, parts: &Parts) -> usize {
        let mut at = p;
        let mut v = parts.v;
        v[..parts.n].sort_unstable_by_key(|x| x.0);
        for &(s, t, leaf) in &v[..parts.n] {
            if s > at {
                self.put(at, s, base);
            }
            self.put(s, t, leaf);
            at = t;
        }
        if e > at {
            self.put(at, e, base);
        }
        e
    }

    /// The whole word at `p`, `\bw\b`, if `p` starts one.
    fn word(&self, p: usize) -> &'b [u8] {
        let b = self.b;
        let (at, end) = self.word_at.get();
        if at == p {
            return &b[p..end];
        }
        // `\b\w`: ASCII's on the table, else by characters
        let end = match b.get(p) {
            Some(&c) if c < 0x80 && (p == 0 || b[p - 1] < 0x80) => {
                let before = p > 0 && CLASSES[usize::from(b[p - 1])] & W != 0;
                if CLASSES[usize::from(c)] & W != 0 && !before {
                    word_end(b, p)
                } else {
                    p
                }
            }
            _ if boundary(b, p) && is_word(b, p) => word_end(b, p),
            _ => p,
        };
        self.word_at.set((p, end));
        &b[p..end]
    }

    #[inline]
    fn word_kind(&self, p: usize) -> Word {
        // every keyword starts with an ASCII lowercase letter
        if !self.b.get(p).is_some_and(u8::is_ascii_lowercase) {
            return Word::Other;
        }
        let (at, kind) = self.kind_at.get();
        if at == p {
            return kind;
        }
        let kind = lookup(self.word(p));
        self.kind_at.set((p, kind));
        kind
    }

    fn is_word_at(&self, p: usize, w: &[u8]) -> bool {
        self.word(p) == w
    }

    fn run(&mut self) {
        let b = self.b;
        let mut p = 0;
        loop {
            if is_frame(self.top()) {
                p = self.code_run(p);
                if is_frame(self.top()) {
                    return;
                }
                continue;
            }
            // a run of blanks no pattern of the context starts in
            if p < b.len() && is_space(b[p]) && PASSES[usize::from(self.top())] & BLANKS != 0 {
                p = skip_spaces(b, p);
            }
            // the line's end, where the context's patterns take nothing
            if p == b.len() && PASSES[usize::from(self.top())] & QUIET_EOL != 0 {
                return;
            }
            if let Some(q) = self.at(p) {
                p = q;
                if self.halt {
                    return;
                }
            } else if p < self.b.len() {
                p = self.advance(p);
            } else {
                return;
            }
        }
    }

    /// Where the next pattern can start, after none did at `p`.
    fn advance(&self, p: usize) -> usize {
        let b = self.b;
        let top = self.top();
        match top {
            STRING | STRING_RE | TEXT_BLOCK => find_any(b, p + 1, *b"\"\\"),
            CHAR => find_any(b, p + 1, *b"'\\"),
            COMMENT | JD_MAIN | JD_HOLD => find_any(b, p + 1, *b"*"),
            LINE_COMMENT => find_any(b, p + 1, *b"%"),
            JD_TEXT | JD_TAG | JD_LABEL => find_any(b, p + 1, *b"*{<&}"),
            JD_INLINE => find_any(b, p + 1, *b"*}"),
            H_DQ => find_any(b, p + 1, *b"\"&*"),
            H_SQ => find_any(b, p + 1, *b"'&*"),
            H_COMMENT => next_of(b, p + 1, |c| matches!(c, b'-' | b'<' | b'*')),
            H_CDATA => next_of(b, p + 1, |c| c == b']' || c == b'*'),
            H_OTHER_NAME => next_of(b, p + 1, |c| {
                matches!(
                    c,
                    b'\t' | b'\n' | b'\x0c' | b' ' | b'/' | b'<' | b'>' | b'*'
                )
            }),
            H_EMBED_TEXT | H_EMBED_DQ | H_EMBED_SQ => {
                next_of(b, p + 1, |c| matches!(c, b'<' | b'*' | b'"' | b'\''))
            }
            _ if is_space(b[p]) => skip_spaces(b, p),
            _ => next(b, p),
        }
    }

    /// The first pattern of the top context, one of bat's, that matches at `p`, applied: where
    /// the scan goes on, or `None` when none does.
    fn at(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let top = self.top();
        if p == b.len() {
            return self.eol(top);
        }
        let c = b[p];
        if top < STRING
            && matches!(c, b'/' | b'g' | b'c' | b'%')
            && let Some(r) = self.prototype(p, c)
        {
            return Some(r);
        }
        if top >= JD_MAIN {
            if top == STRING_RE {
                return self.in_regex(p, c);
            }
            if top >= TEXT_BLOCK {
                return self.sql_context(p, c, top);
            }
            return self.javadoc(p, c, top);
        }
        match top {
            ARRAY_BRACKETS => self.array_brackets(p, c),
            TYPE_FQN => {
                if let Some(r) = self.package(p, c, PACKAGE) {
                    return Some(r);
                }
                if let Some(e) = type_name(b, p) {
                    return Some(self.close(p, e - p, CLASS));
                }
                self.any_pop(p)
            }
            GENERIC_INVOCATION => match c {
                b'<' => Some(self.then(p, 1, &[GENERIC_ARG], GENERIC_BEGIN)),
                _ => self.any_pop(p),
            },
            GENERIC_ARG => match c {
                b'?' => Some(self.open(p, 1, GENERIC_BOUNDS, WILDCARD)),
                b',' => Some(self.token(p, 1, COMMA)),
                _ => self
                    .generic_end(p, c)
                    .or_else(|| self.object_and_array_types(p)),
            },
            GENERIC_BOUNDS => match c {
                b',' => Some(self.close(p, 1, COMMA)),
                b'>' => Some(self.leave(p)),
                _ => match self.word(p) {
                    b"extends" => {
                        self.put(p, p + 7, EXTENDS_KW);
                        self.push(p + 7, MULTI_BOUNDS);
                        self.push(p + 7, TYPE_REF);
                        Some(p + 7)
                    }
                    b"super" => Some(self.open(p, 5, TYPE_REF, SUPER_KW)),
                    _ => None,
                },
            },
            MULTI_BOUNDS => match c {
                b'&' => Some(self.then(p, 1, &[MULTI_BOUNDS, TYPE_REF], BOUNDS_AND)),
                _ => self.any_pop(p),
            },
            GENERIC_DECL => match c {
                b'<' => Some(self.open(p, 1, GENERIC_PARAM, GENERIC_BEGIN)),
                _ => self.any_pop(p),
            },
            GENERIC_PARAM => {
                if boundary(b, p)
                    && let Some(e) = ident(b, p)
                    && let Some(e) = back_to(b, next(b, p), e, |_| true)
                {
                    self.put(p, e, TYPE_PARAMETER);
                    self.push(e, GENERIC_BOUNDS);
                    return Some(e);
                }
                self.generic_end(p, c)
            }
            OBJ_ARRAY_PATH => self
                .package(p, c, PACKAGE)
                .or_else(|| self.obj_array_type(p)),
            OBJ_ARRAY_TYPE => self.obj_array_type(p),
            AFTER_OBJ_ARRAY => match c {
                b'<' => Some(self.shift(p, &[ARRAY_BRACKETS, GENERIC_INVOCATION])),
                b'.' if b.get(p + 1) != Some(&b'.') => {
                    Some(self.then(p, 1, &[OBJ_ARRAY_TYPE], DOT))
                }
                _ => self.array_brackets(p, c),
            },
            TYPE_REF => {
                if package_ahead(b, p) {
                    return Some(self.shift(p, &[TYPE_REF_PATH]));
                }
                self.type_ref_name(p)
            }
            TYPE_REF_PATH => self
                .package(p, c, PACKAGE)
                .or_else(|| self.type_ref_name(p)),
            TYPE_REF_NAME => self.type_ref_name(p),
            AFTER_TYPE_REF => match c {
                b'<' => Some(self.shift(p, &[GENERIC_INVOCATION])),
                b'.' => Some(self.then(p, 1, &[TYPE_REF_NAME], DOT)),
                _ => self.any_pop(p),
            },
            CLASS_TYPE => {
                if self.word_kind(p) == Word::Modifier {
                    let e = word_end(b, p);
                    return Some(self.token(p, e - p, MODIFIER));
                }
                for (kw, cs) in [
                    (&b"class"[..], CLASS_BLOCK),
                    (b"interface", CLASS_BLOCK),
                    (b"@interface", CLASS_BLOCK),
                    (b"enum", ENUM_BLOCK),
                ] {
                    if crate::has_at(b, p, kw) && boundary(b, p + kw.len()) {
                        let e = p + kw.len();
                        self.set(p, &[cs, CLASS_EXTENDS, GENERIC_DECL, CLASS_NAME]);
                        if c == b'@' {
                            self.put(p, p + 1, STORAGE_TYPE_MARK);
                        }
                        self.put(p + usize::from(c == b'@'), e, STORAGE_TYPE);
                        return Some(e);
                    }
                }
                self.any_pop(p)
            }
            CLASS_NAME => {
                if !crate::has_at(b, p, b"extends")
                    && !crate::has_at(b, p, b"implements")
                    && let Some(e) = ident(b, p)
                    && let Some(e) = back_to(b, next(b, p), e, |_| true)
                {
                    return Some(self.close(p, e - p, ENTITY_CLASS));
                }
                self.any_pop(p)
            }
            CLASS_EXTENDS => {
                for (kw, leaf) in [
                    (&b"extends"[..], EXTENDS_KW),
                    (b"implements", IMPLEMENTS_KW),
                ] {
                    if crate::has_at(b, p, kw) && boundary(b, p + kw.len()) {
                        let e = p + kw.len();
                        self.put(p, e, leaf);
                        self.push(e, CLASS_SUPERS);
                        self.push(e, INHERITED_REF);
                        return Some(e);
                    }
                }
                self.any_pop(p)
            }
            CLASS_SUPERS | PROVIDES_LIST | THROWS_LIST => match c {
                b',' => {
                    let to = if top == CLASS_SUPERS {
                        INHERITED_REF
                    } else {
                        TYPE_REF
                    };
                    Some(self.open(p, 1, to, COMMA))
                }
                _ => self.any_pop(p),
            },
            INHERITED_REF => {
                if package_ahead(b, p) {
                    return Some(self.shift(p, &[INHERITED_PATH]));
                }
                self.inherited_name(p)
            }
            INHERITED_PATH => {
                if let Some(e) = lower_word(b, p) {
                    return Some(self.token(p, e - p, INHERITED_PKG));
                }
                if c == b'.' {
                    return Some(self.token(p, 1, DOT));
                }
                self.inherited_name(p)
            }
            INHERITED_NAME => self.inherited_name(p),
            AFTER_INHERITED => match c {
                b'<' => Some(self.shift(p, &[GENERIC_INVOCATION])),
                b'.' => Some(self.then(p, 1, &[INHERITED_NAME], DOT)),
                _ => self.any_pop(p),
            },
            CLASS_BLOCK => match c {
                b'{' => Some(self.then(p, 1, &[CLASS_BODY], BLOCK_BEGIN)),
                _ => self.any_pop(p),
            },
            ENUM_BLOCK => match c {
                b'{' => Some(self.then(p, 1, &[ENUM_BODY], BLOCK_BEGIN)),
                _ => self.any_pop(p),
            },
            ANNOT_PATH => {
                if let Some(e) = lower_word(b, p) {
                    return Some(self.token(p, e - p, ANNOTATION_PKG));
                }
                if c == b'.' {
                    return Some(self.token(p, 1, DOT));
                }
                self.annotation_type(p)
            }
            ANNOT_NAME => self.annotation_type(p),
            ANNOT_AFTER => match c {
                b'.' => Some(self.then(p, 1, &[ANNOT_NAME], DOT)),
                _ => self.any_pop(p),
            },
            STRING | CHAR => {
                let (q, end, escape) = if top == STRING {
                    (b'"', DQ_END, DQ_ESCAPE)
                } else {
                    (b'\'', SQ_END, SQ_ESCAPE)
                };
                if c == q {
                    return Some(self.close(p, 1, end));
                }
                // `\\.`
                (c == b'\\' && p + 1 < b.len()).then(|| {
                    let e = next(b, p + 1);
                    self.token(p, e - p, escape)
                })
            }
            COMMENT => {
                (c == b'*' && b.get(p + 1) == Some(&b'/')).then(|| self.close(p, 2, BLOCK_MARK))
            }
            LINE_COMMENT => (c == b'%' && b.get(p + 1) == Some(&b'>')).then(|| self.leave(p)),
            _ => self.module(p, c, top),
        }
    }

    /// At the line's end, after the last match: the patterns that match the `\n`, or nothing.
    fn eol(&mut self, top: u8) -> Option<usize> {
        let n = self.b.len();
        match top {
            STRING | STRING_RE | CHAR | LINE_COMMENT | DOTTED_NS | DOTTED_MODULE | DOTTED_PKG
            | DOTTED_SMOD | JD_REF_AFTER | JD_IREF_AFTER | H_OTHER_NAME | H_CUSTOM_NAME
            | H_ATTR_NAME | H_UNQ => Some(self.leave(n)),
            SQL_DQ..=SQL_DQ_LAST => Some(self.leave(n)),
            JD_CONTENTS => Some(self.enter(n, &[JD_TEXT])),
            JD_LINK_LABEL => Some(self.shift(n, &[JD_LABEL])),
            SQL_TB..=SQL_TB_LAST => {
                self.sql_eol(top);
                None
            }
            _ => None,
        }
    }

    /// In a regex in a string, at `p`: the string's end and its escapes but `\\`, which is the
    /// regex's, else the regex to the next place they can start.
    fn in_regex(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if c == b'"' {
            return Some(self.close(p, 1, DQ_END));
        }
        if c == b'\\' && p + 1 < b.len() && b[p + 1] != b'\\' {
            let e = next(b, p + 1);
            return Some(self.token(p, e - p, DQ_ESCAPE));
        }
        let host = regex::Embed {
            dialect: regex::Dialect::Java,
            cooked: true,
            yields: &RE_YIELDS,
            stop: |b, p, _| b[p] == b'"',
            arg: 0,
            escape: regex::java_escape,
            top_only: false,
            continues: false,
            base: self.kind(W_RE),
            text: self.text,
        };
        self.k.close(self.out);
        let r = regex::embedded(b, p, self.re, &host, &mut self.at, self.out);
        self.k.reopen(self.out, 16);
        let (q, st) = r?;
        self.re = st;
        Some(q)
    }

    /// The contexts past `TEXT_BLOCK`, out of the hot `match`: a text block, a string whose
    /// text is SQL.
    #[inline(never)]
    fn sql_context(&mut self, p: usize, c: u8, top: u8) -> Option<usize> {
        let b = self.b;
        match top {
            TEXT_BLOCK => match c {
                b'"' => crate::has_at(b, p, b"\"\"\"").then(|| self.close(p, 3, TB_END)),
                b'\\' => Some(self.tb_escape(p)),
                _ => None,
            },
            SQL_DQ..=SQL_TB_LAST => self.in_sql(p, c, top),
            _ => None,
        }
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

    /// A string at `p` whose text is SQL: `"…"`, or a text block. It takes a marker's turn.
    #[inline(never)]
    fn sql_string(&mut self, p: usize) -> usize {
        self.mark = usize::MAX;
        self.sql_end = usize::MAX;
        self.sql.is_end = usize::MAX;
        if self.b[p + 1..].starts_with(b"\"\"") {
            self.open(p, 3, SQL_TB + sql::MAIN, TB_BEGIN)
        } else {
            self.open(p, 1, SQL_DQ + sql::MAIN, DQ_BEGIN)
        }
    }

    /// A text block's escape at `p`: `\\.`, or a `\\` that ends its line (which joins the next).
    fn tb_escape(&mut self, p: usize) -> usize {
        let e = if p + 1 < self.b.len() {
            next(self.b, p + 1)
        } else {
            p + 1
        };
        self.token(p, e - p, TB_ESCAPE)
    }

    /// In a string whose text is SQL, at `p`: the string's end and its escapes, else SQL's
    /// scanner, embedded, to the next place they can start ([`sql::embedded`]).
    fn in_sql(&mut self, p: usize, c: u8, top: u8) -> Option<usize> {
        let b = self.b;
        let tb = top >= SQL_TB;
        match c {
            b'"' if !tb => return Some(self.close(p, 1, DQ_END)),
            b'"' if b[p..].starts_with(b"\"\"\"") => return Some(self.close(p, 3, TB_END)),
            b'\\' if tb => return Some(self.tb_escape(p)),
            b'\\' if p + 1 < b.len() => {
                let e = next(b, p + 1);
                return Some(self.token(p, e - p, DQ_ESCAPE));
            }
            _ => {}
        }
        self.sql_scan(p, top).map(|(q, _)| q)
    }

    /// SQL's scan from `p` in the string `top` holds: where it stopped, its context there set on
    /// the stack.
    fn sql_scan(&mut self, p: usize, top: u8) -> Option<(usize, u8)> {
        let (base, wrap) = if top >= SQL_TB {
            (SQL_TB, W_SQL_TB)
        } else {
            (SQL_DQ, W_SQL)
        };
        let host = sql::Embed {
            yields: &SQL_YIELDS,
            base: self.kind(wrap),
            text: self.text,
        };
        if self.sql_end == usize::MAX {
            self.sql_end = string_end(self.b, p, top >= SQL_TB);
        }
        let end = self.sql_end;
        self.fill(p);
        self.sql.at = self.at;
        self.k.close(self.out);
        let r = sql::embedded(
            &self.b[..end],
            p,
            top - base,
            &host,
            &mut self.sql,
            self.out,
        );
        self.k.reopen(self.out, 16);
        self.at = self.sql.at;
        let (mut q, ctx) = r?;
        if q > end {
            self.sql_nl |= end == self.b.len();
            q = end;
        }
        self.stack[self.depth - 1] = base + ctx;
        Some((q, ctx))
    }

    /// A text block's SQL at its line's end, which SQL's patterns may take as `\s`; its line
    /// comment ends there.
    fn sql_eol(&mut self, top: u8) {
        let n = self.b.len();
        if !self.sql_nl {
            self.sql_scan(n, top);
        }
        if matches!(self.top() - SQL_TB, sql::DASHES | sql::HASHES) {
            self.stack[self.depth - 1] = SQL_TB + sql::MAIN;
        }
    }

    /// The prototype: comments, `goto` and `const`, `%>`.
    fn prototype(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        match c {
            b'/' => self.comment(p),
            b'g' | b'c' if self.word_kind(p) == Word::Illegal => {
                Some(self.token(p, 4 + usize::from(c == b'c'), ILLEGAL_KEYWORD))
            }
            // each context pops, down to the top level, where syntect's guard against a loop
            // leaves the `%` to the patterns after
            b'%' if b.get(p + 1) == Some(&b'>') && self.depth > 1 => Some(self.leave(p)),
            _ => None,
        }
    }

    /// A comment at `p`, a `/`: `/**/`, Javadoc, a block or a line comment (a SQL marker's
    /// noted).
    fn comment(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        match b.get(p + 1) {
            Some(b'*') => {
                if crate::has_at(b, p + 2, b"*/") {
                    return Some(self.token(p, 4, EMPTY_COMMENT));
                }
                if b.get(p + 2) == Some(&b'*') {
                    self.put(p, p + 3, JD_BEGIN);
                    self.push(p + 3, JD_MAIN);
                    self.push(p + 3, JD_CONTENTS);
                    return Some(p + 3);
                }
                if sql::marker_comment(b, p + 2) {
                    self.mark = b[p + 2..]
                        .windows(2)
                        .position(|w| w == b"*/")
                        .map_or(b.len(), |i| p + 4 + i);
                }
                Some(self.open(p, 2, COMMENT, BLOCK_MARK))
            }
            Some(b'/') => {
                if sql::marker_comment(b, p + 2) {
                    self.mark = b.len();
                }
                Some(self.open(p, 2, LINE_COMMENT, LINE_MARK))
            }
            _ => None,
        }
    }

    /// `(?=\b(?:open\s+)?module\b)` at `p`.
    fn module_ahead(&self, p: usize) -> bool {
        let b = self.b;
        let module = |q: usize| b[q..].starts_with(b"module") && boundary(b, q + 6);
        if b[p..].starts_with(b"open") {
            let s = skip_spaces(b, p + 4);
            if s > p + 4 && module(s) {
                return true;
            }
        }
        module(p)
    }

    /// `class`'s `(?=((?:modifiers)\s+)*(?:class|(?:@)?interface|enum)\b)` at `p`.
    fn class_ahead(&self, p: usize) -> bool {
        let b = self.b;
        // the first bytes of the keywords and the modifiers
        if !matches!(
            b[p],
            b'c' | b'i' | b'@' | b'e' | b'p' | b's' | b'f' | b'n' | b'a' | b't' | b'd' | b'v'
        ) {
            return false;
        }
        // a later modifier of the chain last read (a start after its blanks): the chain's answer
        let (from, last, known) = self.class_at.get();
        if p > from && p <= last && is_space(b[p - 1]) {
            return known;
        }
        let mut q = p;
        let found = loop {
            if [&b"class"[..], b"interface", b"@interface", b"enum"]
                .iter()
                .any(|kw| b[q..].starts_with(kw) && boundary(b, q + kw.len()))
            {
                break true;
            }
            let Some(m) = MODIFIERS.iter().find(|m| b[q..].starts_with(m)) else {
                break false;
            };
            let r = q + m.len();
            let s = skip_spaces(b, r);
            if s == r {
                break false;
            }
            q = s;
        };
        self.class_at.set((p, q, found));
        found
    }

    /// `enum-body`'s `^(?=\s*([\p{Lu}_][\p{Lu}\p{Nd}_]*|(?!primitives|modifiers)
    /// [\p{Ll}_][\p{L}\p{N}_]*)\s*[,;{(])`, at the line's start.
    fn enum_line(&self) -> bool {
        let b = self.b;
        let q = skip_spaces(b, 0);
        let sep = |e: usize| matches!(b.get(skip_spaces(b, e)), Some(b',' | b';' | b'{' | b'('));
        let upper = |b: &[u8], i: usize| is_upper(b, i) || b[i] == b'_';
        if q < b.len() && upper(b, q) {
            let e = run_of(b, next(b, q), |b, i| upper(b, i) || b[i].is_ascii_digit());
            if sep(e) {
                return true;
            }
        }
        if q < b.len()
            && (is_lower(b, q) || b[q] == b'_')
            && !PRIMITIVES
                .iter()
                .chain(&MODIFIERS)
                .any(|w| b[q..].starts_with(w))
        {
            let e = run_of(b, next(b, q), |b, i| {
                b[i] == b'_' || (b[i] != b'$' && is_ident(b, i))
            });
            return sep(e);
        }
        false
    }

    /// `(?=(?:[\p{L}_$][\p{L}\p{N}_$]*)\s*\()` at `p`.
    fn call_ahead(&self, p: usize) -> bool {
        let b = self.b;
        ident(b, p).is_some_and(|e| b.get(skip_spaces(b, e)) == Some(&b'('))
    }

    /// `before-next-import`: `(?=\bimport\b)`, `(?=\b(?:modifiers|class|interface|enum)\b)`.
    fn before_next_import(&self, p: usize) -> bool {
        matches!(self.word(p), b"import" | b"class" | b"interface" | b"enum")
            || self.word_kind(p) == Word::Modifier
    }

    /// `object-and-array-types` at `p`.
    fn object_and_array_types(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if package_ahead(b, p) {
            return Some(self.enter(p, &[OBJ_ARRAY_PATH]));
        }
        // a primitive's name then `\s*\[`: the whole word (the cached one)
        if self.word_kind(p) == Word::Primitive {
            let e = p + self.word(p).len();
            if b.get(skip_spaces(b, e)) == Some(&b'[') {
                self.put(p, e, PRIMITIVE);
                self.push(e, ARRAY_BRACKETS);
                return Some(e);
            }
        }
        if boundary(b, p)
            && let Some(e) = type_name(b, p)
        {
            self.put(p, e, CLASS);
            return Some(self.after(e, AFTER_OBJ_ARRAY, b"<.["));
        }
        None
    }

    /// `c` pushed at `e`, a context whose patterns start at `stops`: where it would pop at once
    /// ([`Scan::pops_at`]), not pushed, and the scan goes on where it would have popped.
    fn after(&mut self, e: usize, c: u8, stops: &[u8]) -> usize {
        match self.pops_at(e, stops) {
            Some(r) => r,
            None => {
                self.push(e, c);
                e
            }
        }
    }

    /// `object-and-array-types-no-fqn` at `p`.
    fn obj_array_type(&mut self, p: usize) -> Option<usize> {
        match type_name(self.b, p) {
            Some(e) => Some(self.then(p, e - p, &[AFTER_OBJ_ARRAY], CLASS)),
            None => self.any_pop(p),
        }
    }

    /// `object-type-reference-no-fqn` at `p`.
    fn type_ref_name(&mut self, p: usize) -> Option<usize> {
        match type_name(self.b, p) {
            Some(e) => Some(self.then(p, e - p, &[AFTER_TYPE_REF], CLASS)),
            None => self.any_pop(p),
        }
    }

    /// `inherited-object-type-reference-no-fqn` at `p`.
    fn inherited_name(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let kw = [&b"class"[..], b"extends", b"implements", b"interface"];
        if !kw.iter().any(|w| b[p..].starts_with(w))
            && let Some(e) = ident(b, p)
        {
            return Some(self.then(p, e - p, &[AFTER_INHERITED], INHERITED));
        }
        self.any_pop(p)
    }

    /// `annotation-type-no-fqn` at `p`.
    fn annotation_type(&mut self, p: usize) -> Option<usize> {
        match type_name(self.b, p) {
            Some(e) => {
                self.annotation_name(p, e);
                self.set(e, &[ANNOT_AFTER]);
                Some(e)
            }
            None => self.any_pop(p),
        }
    }

    /// `package` at `p`: a package's name as `leaf`, or a `.`.
    fn package(&mut self, p: usize, c: u8, leaf: u16) -> Option<usize> {
        if let Some(e) = lower_word(self.b, p) {
            return Some(self.token(p, e - p, leaf));
        }
        (c == b'.').then(|| self.token(p, 1, DOT))
    }

    /// `array-brackets`: `\[\s*\]`, then `any_POP`.
    fn array_brackets(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if c == b'[' {
            let q = skip_spaces(b, p + 1);
            if b.get(q) == Some(&b']') {
                return Some(self.token(p, q + 1 - p, ARRAY_MOD));
            }
        }
        self.any_pop(p)
    }

    /// `generic-type-terminator` at `p`.
    fn generic_end(&mut self, p: usize, c: u8) -> Option<usize> {
        match c {
            b';' => Some(self.close(p, 1, STRAY_TERMINATOR)),
            b'{' | b'}' | b'(' | b')' => Some(self.leave(p)),
            b'>' => Some(self.close(p, 1, GENERIC_END)),
            _ => None,
        }
    }

    /// A context pushed at `e` whose patterns start only at one of `stops` or the prototype's
    /// (`/`, `%`, `goto`, `const`), else `any_POP`: where it would pop, the first byte past `e`'s
    /// blanks on the line, if it would there. The caller then skips the push and goes on from
    /// that byte in its own context: the blanks are both contexts' text, and no lookahead's loop
    /// guard can be past `e`.
    fn pops_at(&self, e: usize, stops: &[u8]) -> Option<usize> {
        let b = self.b;
        let r = skip_spaces(b, e);
        let &c = b.get(r)?;
        // near the stack's limit a push drops the bottom context: no shortcut there
        if self.depth + 4 >= MAX || stops.contains(&c) || matches!(c, b'/' | b'%') {
            return None;
        }
        if matches!(c, b'g' | b'c') && self.word_kind(r) == Word::Illegal {
            return None;
        }
        Some(r)
    }

    /// `(?=\b(?:(?:[_$]*\p{Ll}[\p{Ll}\p{N}_$]*\b)\.)+\p{Lu})` at `p`.
    fn fqn_ahead(&self, p: usize) -> bool {
        let b = self.b;
        // a word after a `.` of the last chain read: the chain's answer (its `.`s all part it)
        let (from, to, known) = self.fqn.get();
        if p > from && p < to && b[p - 1] == b'.' {
            return known;
        }
        let mut q = p;
        let mut n = 0;
        let found = loop {
            if n > 0 && is_upper(b, q) {
                break true;
            }
            match lower_run(b, q) {
                Some(e) if b.get(e) == Some(&b'.') && word_before(b, e) => {
                    q = e + 1;
                    n += 1;
                }
                _ => break false,
            }
        };
        self.fqn.set((p, q, found));
        found
    }

    /// `lambdas`' `\(.*\)\s*->` at `p`, a `(`: a `)` later on the line with `->` after it.
    fn arrow_after(&mut self, p: usize) -> bool {
        let b = self.b;
        let last = *self.arrow.get_or_insert_with(|| {
            let mut last = None;
            let mut i = 0;
            while let Some(k) = b[i..].windows(2).position(|w| w == b"->") {
                let a = i + k;
                let mut j = a;
                while j > 0 && is_space(b[j - 1]) {
                    j -= 1;
                }
                if j > 0 && b[j - 1] == b')' {
                    last = Some(j - 1);
                }
                i = a + 2;
            }
            last
        });
        last.is_some_and(|q| q > p)
    }

    /// The numbers of `constants-and-special-vars` at `p`, in their order: a hexadecimal
    /// float, a decimal one, a binary, hexadecimal, octal, decimal integer.
    fn number(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let mut parts = Parts::default();
        if b[p] == b'.' {
            // `(\.) (?:(_*)\d[\d_]*?(_*)) (?:[eE]…)? ([dDfF])? \b`
            parts.add(p, p + 1, FLOAT_DOT);
            let g = group(b, p + 1, is_dec, FLOAT_INVALID, &mut parts)?;
            let e = tail(
                b,
                g,
                Some(*b"eE"),
                true,
                FLOAT_INVALID,
                FLOAT_SUFFIX,
                &mut parts,
            )?;
            return Some(self.emit(p, e, FLOAT, &parts));
        }
        let hex = b[p] == b'0' && matches!(b.get(p + 1), Some(b'x' | b'X'));
        if hex && let Some(e) = self.hex_float(p, &mut parts) {
            return Some(self.emit(p, e, HEX_FLOAT, &parts));
        }
        parts = Parts::default();
        if let Some(e) = self.decimal_float(p, &mut parts) {
            return Some(self.emit(p, e, FLOAT, &parts));
        }
        parts = Parts::default();
        let suffix = |e: usize| -> Option<usize> {
            if matches!(b.get(e), Some(b'l' | b'L')) && boundary(b, e + 1) {
                return Some(e + 1);
            }
            boundary(b, e).then_some(e)
        };
        // `\b(0[bB])(_*)[01][01_]*?(_*)([lL])?\b`
        if b[p] == b'0' && matches!(b.get(p + 1), Some(b'b' | b'B')) {
            parts.add(p, p + 2, BIN_PREFIX);
            if let Some(g) = group(
                b,
                p + 2,
                |c| c == b'0' || c == b'1',
                BIN_INVALID,
                &mut parts,
            ) && let Some(e) = suffix(g)
            {
                parts.add(g, e, BIN_SUFFIX);
                return Some(self.emit(p, e, BIN, &parts));
            }
            parts = Parts::default();
        }
        // `\b(0[xX])(?:(_*)\h[\h_]*?(_*))([lL])?\b`
        if hex {
            parts.add(p, p + 2, HEX_PREFIX);
            if let Some(g) = group(b, p + 2, is_hex, HEX_INVALID, &mut parts)
                && let Some(e) = suffix(g)
            {
                parts.add(g, e, HEX_SUFFIX);
                return Some(self.emit(p, e, HEX, &parts));
            }
            parts = Parts::default();
        }
        // `\b(0)(?:(_+)|[0-7_]+?(_*)|([\d_]+))([lL])?\b`
        if b[p] == b'0' {
            parts.add(p, p + 1, OCT_PREFIX);
            let us = next_of(b, p + 1, |c| c != b'_');
            if us > p + 1
                && let Some(e) = suffix(us)
            {
                parts.add(p + 1, us, OCT_INVALID);
                parts.add(us, e, OCT_SUFFIX);
                return Some(self.emit(p, e, OCT, &parts));
            }
            let r = next_of(b, p + 1, |c| !matches!(c, b'0'..=b'7' | b'_'));
            if r > p + 1
                && let Some(e) = suffix(r)
            {
                let mut t = r;
                while t > p + 2 && b[t - 1] == b'_' {
                    t -= 1;
                }
                parts.add(t, r, OCT_INVALID);
                parts.add(r, e, OCT_SUFFIX);
                return Some(self.emit(p, e, OCT, &parts));
            }
            let d = next_of(b, p + 1, |c| !c.is_ascii_digit() && c != b'_');
            if d > p + 1
                && let Some(e) = suffix(d)
            {
                parts.add(p + 1, d, OCT_INVALID);
                parts.add(d, e, OCT_SUFFIX);
                return Some(self.emit(p, e, OCT, &parts));
            }
            parts = Parts::default();
        }
        // `\b\d[\d_]*?(_*)([lL])?\b`
        let r = next_of(b, p + 1, |c| !c.is_ascii_digit() && c != b'_');
        let mut t = r;
        while t > p + 1 && b[t - 1] == b'_' {
            t -= 1;
        }
        let e = suffix(r)?;
        parts.add(t, r, DEC_INVALID);
        parts.add(r, e, DEC_SUFFIX);
        Some(self.emit(p, e, DEC, &parts))
    }

    /// The hexadecimal float at `p`, a `0x`: where it ends.
    fn hex_float(&self, p: usize, parts: &mut Parts) -> Option<usize> {
        let b = self.b;
        let q = p + 2;
        parts.add(p, q, HEX_FLOAT_PREFIX);
        let n = parts.n;
        if let Some(g) = group(b, q, is_hex, HEX_FLOAT_INVALID, parts) {
            if b.get(g) == Some(&b'.') {
                // `(\.) (?: G? P? \b )?`
                parts.add(g, g + 1, HEX_FLOAT_DOT);
                let m = parts.n;
                let frac = group(b, g + 1, is_hex, HEX_FLOAT_INVALID, parts);
                if let Some(f) = frac
                    && let Some(e) = tail(b, f, Some(*b"pP"), false, HEX_FLOAT_INVALID, 0, parts)
                {
                    return Some(e);
                }
                parts.n = m;
                if let Some(e) = tail(b, g + 1, Some(*b"pP"), false, HEX_FLOAT_INVALID, 0, parts) {
                    return Some(e);
                }
                parts.n = m;
                return Some(g + 1);
            }
            // `P \b`
            if let Some(e) = exponent(b, g, *b"pP", HEX_FLOAT_INVALID, parts)
                && boundary(b, e)
            {
                return Some(e);
            }
            parts.n = n;
        }
        // `(\.) G P? \b`
        if b.get(q) == Some(&b'.') {
            parts.add(q, q + 1, HEX_FLOAT_DOT);
            let g = group(b, q + 1, is_hex, HEX_FLOAT_INVALID, parts)?;
            return tail(b, g, Some(*b"pP"), false, HEX_FLOAT_INVALID, 0, parts);
        }
        None
    }

    /// The decimal float at `p`, a digit where `\b` holds: where it ends.
    fn decimal_float(&self, p: usize, parts: &mut Parts) -> Option<usize> {
        let b = self.b;
        let r = next_of(b, p + 1, |c| !c.is_ascii_digit() && c != b'_');
        let mut t = r;
        while t > p + 1 && b[t - 1] == b'_' {
            t -= 1;
        }
        parts.add(t, r, FLOAT_INVALID);
        let exp = Some(*b"eE");
        match b.get(r) {
            Some(b'.') => {
                parts.add(r, r + 1, FLOAT_DOT);
                let m = parts.n;
                if let Some(f) = group(b, r + 1, is_dec, FLOAT_INVALID, parts)
                    && let Some(e) = tail(b, f, exp, true, FLOAT_INVALID, FLOAT_SUFFIX, parts)
                {
                    return Some(e);
                }
                parts.n = m;
                if let Some(e) = tail(b, r + 1, exp, true, FLOAT_INVALID, FLOAT_SUFFIX, parts) {
                    return Some(e);
                }
                parts.n = m;
                Some(r + 1)
            }
            Some(b'e' | b'E') => {
                let x = exponent(b, r, *b"eE", FLOAT_INVALID, parts)?;
                tail(b, x, None, true, FLOAT_INVALID, FLOAT_SUFFIX, parts)
            }
            Some(b'd' | b'D' | b'f' | b'F') if boundary(b, r + 1) => {
                parts.add(r, r + 1, FLOAT_SUFFIX);
                Some(r + 1)
            }
            _ => None,
        }
    }

    /// `module-info.java`'s contexts, and the imports' and the package's.
    fn module(&mut self, p: usize, c: u8, top: u8) -> Option<usize> {
        let b = self.b;
        match top {
            MODULE_KW => match self.word(p) {
                b"open" => Some(self.token(p, 4, MODIFIER)),
                b"module" => Some(self.then(p, 6, &[MODULE_BODY, MODULE_ID], STORAGE_TYPE)),
                _ => None,
            },
            MODULE_ID => match ident(b, p) {
                Some(e) => Some(self.then(p, e - p, &[DOTTED_MODULE], MODULE_NAME)),
                None => self.any_pop(p),
            },
            MODULE_BODY => match c {
                b'{' => Some(self.then(p, 1, &[MODULE_BODY_IN], BRACE_BEGIN)),
                _ => self.any_pop(p),
            },
            MODULE_BODY_IN => {
                let w = self.word(p);
                let n = w.len();
                match w {
                    b"exports" => return Some(self.open(p, n, EXPORTS, EXPORTS_KW)),
                    b"opens" => return Some(self.open(p, n, EXPORTS, OPENS_KW)),
                    b"requires" => return Some(self.open(p, n, REQUIRES, REQUIRES_KW)),
                    b"uses" => return Some(self.open(p, n, TYPE_REF, USES_KW)),
                    b"provides" => {
                        self.put(p, p + n, PROVIDES_KW);
                        self.push(p + n, PROVIDES_WITH);
                        self.push(p + n, TYPE_REF);
                        return Some(p + n);
                    }
                    _ => {}
                }
                match c {
                    b';' => Some(self.token(p, 1, TERMINATOR)),
                    b'}' => Some(self.close(p, 1, BRACE_END)),
                    _ => None,
                }
            }
            EXPORTS | EXPORTS_TO | REQUIRES => {
                if top == EXPORTS && self.is_word_at(p, b"to") {
                    return Some(self.then(p, 2, &[EXPORTS_TO], TO_KW));
                }
                if top == REQUIRES && self.is_word_at(p, b"transitive") {
                    return Some(self.token(p, 10, TRANSITIVE_KW));
                }
                if matches!(c, b';' | b'}')
                    || matches!(
                        self.word(p),
                        b"requires" | b"exports" | b"uses" | b"provides" | b"opens"
                    )
                {
                    return Some(self.leave(p));
                }
                if let Some(e) = ident(b, p) {
                    let (leaf, to) = if top == EXPORTS {
                        (PACKAGE, DOTTED_PKG)
                    } else {
                        (SUPPORT_MODULE, DOTTED_SMOD)
                    };
                    self.put(p, e, leaf);
                    self.push(e, to);
                    return Some(e);
                }
                (top == EXPORTS_TO && c == b',').then(|| self.token(p, 1, COMMA))
            }
            PROVIDES_WITH => {
                if self.is_word_at(p, b"with") {
                    return Some(self.then(p, 4, &[PROVIDES_LIST, TYPE_REF], WITH_KW));
                }
                self.any_pop(p)
            }
            DOTTED_NS | DOTTED_MODULE | DOTTED_PKG | DOTTED_SMOD => {
                let (leaf, dot) = match top {
                    DOTTED_NS => (NAMESPACE, NAMESPACE_DOT),
                    DOTTED_MODULE => (MODULE_NAME, MODULE_DOT),
                    DOTTED_PKG => (PACKAGE, PACKAGE_DOT),
                    _ => (SUPPORT_MODULE, SUPPORT_MODULE_DOT),
                };
                if let Some(e) = ident(b, p) {
                    return Some(self.token(p, e - p, leaf));
                }
                if c == b'.' {
                    return Some(self.token(p, 1, dot));
                }
                Some(self.leave(p))
            }
            PACKAGE_NAME => match ident(b, p) {
                Some(e) => Some(self.then(p, e - p, &[DOTTED_NS], NAMESPACE)),
                None => self.any_pop(p),
            },
            IMPORT_BODY => {
                if self.is_word_at(p, b"static") {
                    return Some(self.then(p, 6, &[STATIC_IMPORT], STATIC_KW));
                }
                self.import_head(p, IMPORT_PATH)
            }
            STATIC_IMPORT => self.import_head(p, STATIC_PATH),
            IMPORT_PATH | STATIC_PATH => {
                if self.before_next_import(p) {
                    return Some(self.leave(p));
                }
                if let Some(r) = self.package(p, c, PACKAGE) {
                    return Some(r);
                }
                if top == IMPORT_PATH && c == b'*' {
                    return Some(self.close(p, 1, ASTERISK));
                }
                if let Some(e) = type_name(b, p) {
                    let to = if top == IMPORT_PATH {
                        IMPORT_AFTER
                    } else {
                        STATIC_AFTER
                    };
                    return Some(self.then(p, e - p, &[to], IMPORT_CLASS));
                }
                self.any_pop(p)
            }
            IMPORT_AFTER | STATIC_AFTER => {
                if self.before_next_import(p) {
                    return Some(self.leave(p));
                }
                match c {
                    b'.' => return Some(self.token(p, 1, DOT)),
                    b'*' => return Some(self.token(p, 1, ASTERISK)),
                    _ => {}
                }
                if top == STATIC_AFTER
                    && let Some(e) = upper_word(b, p)
                {
                    return Some(self.token(p, e - p, IMPORT_CONSTANT));
                }
                if let Some(e) = type_name(b, p) {
                    return Some(self.token(p, e - p, IMPORT_CLASS));
                }
                if top == STATIC_AFTER
                    && let Some(e) = ident(b, p)
                {
                    return Some(self.token(p, e - p, IMPORT_FUNCTION));
                }
                self.any_pop(p)
            }
            _ => None,
        }
    }

    /// `import-statement-body`'s and `static-import-statement-body`'s patterns but `static`.
    fn import_head(&mut self, p: usize, to: u8) -> Option<usize> {
        if self.before_next_import(p) {
            return Some(self.leave(p));
        }
        match lower_word(self.b, p) {
            Some(e) => Some(self.then(p, e - p, &[to], PACKAGE)),
            None => self.any_pop(p),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Lang;
    use crate::tests::marked;

    fn of(lines: &[&str]) -> Vec<(String, String)> {
        marked(Lang::Java, lines)
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
    fn declarations() {
        let l = [
            "package org.example.app;",
            "import java.util.List;",
            "import static org.junit.Assert.assertEquals;",
            "@Deprecated",
            "public final class Foo<T extends Comparable<T>> extends Bar implements Baz {",
            "    private static final int MAX = 0x1F;",
            "    public <R> List<R> map(final String name, int... xs) throws IOException {",
            "        return null;",
            "    }",
            "}",
        ];
        has(&l, "package", "keyword.other.package.java");
        has(&l, "org", "entity.name.namespace.java");
        has(&l, "import", "keyword.control.import.java");
        has(&l, "List", "support.class.import.java");
        has(&l, "assertEquals", "support.function.import.java");
        has(
            &l,
            "Deprecated",
            "meta.annotation.java variable.annotation.java",
        );
        has(&l, "class", "storage.type.java");
        has(&l, "Foo", "entity.name.class.java");
        has(&l, "T", "variable.parameter.type.java");
        has(&l, "Bar", "entity.other.inherited-class.java");
        has(&l, "MAX", "entity.name.constant.java");
        has(&l, "map", "entity.name.function.java");
        has(&l, "name", "variable.parameter.java");
        has(&l, "...", "keyword.operator.variadic.java");
        has(&l, "throws", "keyword.declaration.throws.java");
        has(&l, "return", "keyword.control.flow.return.java");
    }

    #[test]
    fn code_and_numbers() {
        let l = [
            "class A { void f() {",
            "  var x = new ArrayList<>(); x.add(1.5e3f); y += 0b1010L | 017 ^ 1_000;",
            "  list.forEach(e -> System.out.println(e)); Foo::bar; String.class;",
            "  if (a instanceof B && !c) throw new IllegalStateException(\"x\\n\");",
            "} }",
        ];
        has(&l, "var", "storage.type.var.java");
        has(&l, "new", "keyword.other.storage.new.java");
        has(&l, "ArrayList", "support.class.java");
        has(&l, "<>", "punctuation.definition.generic.diamond.java");
        has(&l, "add", "variable.function.java");
        has(&l, "5e3", "constant.numeric.float.decimal.java");
        has(
            &l,
            "f",
            "constant.numeric.float.decimal.java storage.type.numeric.java",
        );
        has(
            &l,
            "0b",
            "constant.numeric.integer.binary.java punctuation.definition.numeric.binary.java",
        );
        has(&l, "e", "variable.parameter.java");
        has(&l, "->", "storage.type.function.anonymous.java");
        has(&l, "bar", "variable.function.reference.java");
        has(&l, "class", "variable.language.java");
        has(&l, "instanceof", "keyword.operator.word.instanceof.java");
        has(
            &l,
            "\\n",
            "string.quoted.double.java constant.character.escape.java",
        );
    }

    #[test]
    fn javadoc() {
        let l = [
            "/**",
            " * Some <b>bold</b> {@code x < y} and {@link Foo#bar(int) label} &lt;.",
            " * @param x the value",
            " */",
        ];
        has(
            &l,
            "*",
            "comment.block.documentation.javadoc text.html.javadoc punctuation.definition.comment.javadoc",
        );
        has(
            &l,
            "b",
            "comment.block.documentation.javadoc text.html.javadoc entity.name.tag.inline.any.html",
        );
        has(
            &l,
            "x < y",
            "comment.block.documentation.javadoc text.html.javadoc markup.raw.javadoc",
        );
        has(
            &l,
            "Foo#bar(int)",
            "comment.block.documentation.javadoc text.html.javadoc markup.underline.link.javadoc",
        );
        has(
            &l,
            "x",
            "comment.block.documentation.javadoc text.html.javadoc variable.parameter.javadoc",
        );
        has(
            &l,
            "*/",
            "comment.block.documentation.javadoc punctuation.definition.comment.end.javadoc",
        );
    }

    #[test]
    fn regexes_in_strings() {
        const RE: &str = "string.quoted.double.java";
        let l = [
            "class A {",
            r#"  static final Pattern P = Pattern.compile("^(?<y>\\d{4})\\.\"[\\w&&[^_]]++$", Pattern.CASE_INSENSITIVE);"#,
            r#"  @Pattern(regexp = "[a-z]+(?i:x)")"#,
            r#"  String f(String s) { return s.replaceAll("\\s{2,}", "\\$1").split("\\\\")[0] + s.concat("a+"); }"#,
            "}",
        ];
        has(&l, "^", &format!("{RE} keyword.control.anchor.regexp"));
        has(&l, "y", &format!("{RE} entity.name.other.group.regexp"));
        has(
            &l,
            r"\\d",
            &format!("{RE} constant.other.character-class.escape.backslash.regexp"),
        );
        has(
            &l,
            "4",
            &format!("{RE} keyword.operator.quantifier.regexp constant.literal.numeric.regexp"),
        );
        // the host's escape between the regex's tokens
        has(&l, r#"\""#, &format!("{RE} constant.character.escape.java"));
        has(
            &l,
            "&&",
            &format!(
                "{RE} constant.other.character-class.set.regexp keyword.operator.intersection.regexp"
            ),
        );
        has(
            &l,
            "++",
            &format!("{RE} keyword.operator.quantifier.regexp"),
        );
        has(&l, "$", &format!("{RE} keyword.control.anchor.regexp"));
        // in an annotation
        has(
            &l,
            "z",
            &format!(
                "meta.annotation.java {RE} constant.other.character-class.set.regexp constant.other.character-class.range.regexp"
            ),
        );
        has(
            &l,
            "?i",
            &format!("meta.annotation.java {RE} keyword.other.option-toggle.regexp"),
        );
        // the first argument only; an escaped `\`; not any call's
        has(
            &l,
            "2",
            &format!("{RE} keyword.operator.quantifier.regexp constant.literal.numeric.regexp"),
        );
        let got = of(&[l[3]]);
        assert!(
            !got.iter().any(|(t, s)| t == "$1" && s.contains("regexp")),
            "{got:?}"
        );
        has(
            &l,
            r"\\\\",
            &format!("{RE} constant.character.escape.backslash.regexp"),
        );
        assert!(
            !got.iter().any(|(t, s)| t == "+" && s.contains("regexp")),
            "{got:?}"
        );
    }

    #[test]
    fn text_blocks() {
        const TB: &str = "string.quoted.triple.java";
        let l = [
            "class A {",
            r#"  String s = """"#,
            r#"    a "q" "" \n \"""
"#
            .trim_end(),
            r#"    int x = 1; \"#,
            r#"    """;"#,
            "  int y;",
            "}",
        ];
        has(
            &l,
            r#"""""#,
            &format!("{TB} punctuation.definition.string.begin.java"),
        );
        has(&l, r#"    a "q" "" "#, TB);
        has(&l, r"\n", &format!("{TB} constant.character.escape.java"));
        // `\"` then `""`, not the end
        has(&l, r#"\""#, &format!("{TB} constant.character.escape.java"));
        has(&l, r#"    int x = 1; "#, TB);
        // a `\` at a line's end joins the next
        has(&l, r"\", &format!("{TB} constant.character.escape.java"));
        has(
            &l,
            r#"""""#,
            &format!("{TB} punctuation.definition.string.end.java"),
        );
        has(&l, "int", "storage.type.primitive.java");
    }

    #[test]
    fn sql_in_strings() {
        const DQ: &str = "string.quoted.double.java";
        const TB: &str = "string.quoted.triple.java";
        const AN: &str = "meta.annotation.java string.quoted.double.java";
        let l = [
            "class A {",
            "  // language=SQL",
            r#"  static final String Q = "SELECT a FROM t WHERE b = \"x\"";"#,
            r#"  @Language("PostgreSQL")"#,
            r#"  String q = """"#,
            r#"    DELETE FROM t /* a"#,
            r#"    */ WHERE c = 'x' -- d"#,
            r#"    """;"#,
            r#"  @Query("UPDATE u SET n = ?1")"#,
            r#"  @NamedQuery(name = "HAVING", query = "SELECT n FROM N")"#,
            "  void f() {",
            "    var ps = conn.prepareStatement(",
            r#"      "SELECT id " +"#,
            r#"      "FROM users WHERE id = ? AND n = :n", "ORDER");"#,
            r#"    jdbcTemplate.update("INSERT INTO t"); doc.select("UNION"); s.concat("LIMIT");"#,
            r#"    String p = "GROUP";"#,
            "  }",
            "}",
        ];
        has(&l, "SELECT", &format!("{DQ} keyword.other.DML.sql"));
        // the host's escape between SQL's tokens
        has(&l, r#"\""#, &format!("{DQ} constant.character.escape.java"));
        // a text block's SQL, its comment across lines, a line comment to its line's end
        has(&l, "DELETE", &format!("{TB} keyword.other.DML.sql"));
        has(&l, "    */", &format!("{TB} comment.block.c"));
        has(&l, "WHERE", &format!("{TB} keyword.other.DML.sql"));
        has(&l, " d", &format!("{TB} comment.line.double-dash.sql"));
        has(&l, "UPDATE", &format!("{AN} keyword.other.DML.sql"));
        // JPA's positional parameter, JDBC's `?`, a named one (X-129)
        has(&l, "?1", &format!("{AN} constant.other.placeholder.sql"));
        // `query =`'s value, not `name =`'s
        has(&l, "HAVING", AN);
        has(&[l[9]], "SELECT", &format!("{AN} keyword.other.DML.sql"));
        // a call's first argument, over lines and `+`; not its second
        has(&l, "FROM", &format!("{DQ} keyword.other.DML.sql"));
        has(&l, "?", &format!("{DQ} constant.other.placeholder.sql"));
        has(&l, ":n", &format!("{DQ} constant.other.placeholder.sql"));
        has(&l, "ORDER", DQ);
        has(&l, "INSERT INTO", &format!("{DQ} keyword.other.DML.sql"));
        // jsoup's `select`, any call, a plain string
        for w in ["UNION", "LIMIT", "GROUP"] {
            has(&l, w, DQ);
        }
    }
}
