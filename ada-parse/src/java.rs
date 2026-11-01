//! Java by a scanner of ada's own (ADR-40, speed first): bat's `Java` grammar compiled by hand,
//! as Rust's and Kotlin's were (`rust.rs`, `kotlin.rs`), with the Javadoc it embeds and the part
//! of HTML that Javadoc includes. Each context that can be open is an arm of one `match` on the
//! top of a stack, trying its patterns in the grammar's order at the position the scan has
//! reached, which is syntect's leftmost match; where no pattern can start (a word, spaces, a
//! comment's text), the run is skipped whole.
//!
//! The grammar keeps meta scopes on nearly every context, but the built-in themes key on one of
//! them only, `meta.annotation`, so a kind is a leaf (bat's scopes for a token) in an
//! annotation or not, and the contexts that differ only in meta scopes are one here (a method's
//! body, a block, a static initializer are [`BLOCK`]; `( … )` and a call's arguments are
//! [`PARENS`]); the contexts that only hold a meta scope and pop at once aren't kept. Inside
//! Javadoc a kind also says which HTML string or comment holds it ([`scopes`]).
//!
//! It follows the grammar where it colors, odd corners included: `@Override`'s line end and the
//! next line's indentation are the annotation's; `default` is a modifier in a block, and
//! `default ->` a lambda's parameter; a line comment ends at `%>`, which then empties the stack;
//! an unknown block tag (`@hidden`) leaves the rest of its Javadoc plain to the next tag;
//! `List<String>` in Javadoc is an HTML tag. **Not followed:** CSS and JavaScript in Javadoc's
//! HTML (`<style>`, `<script>`, `style=`, `onclick=`), whose text is the comment's; past ASCII,
//! the Unicode classes are `char`'s (`\p{L}` its alphabetic, `\w` its alphanumeric), and `\s`
//! is ASCII's.
//!
//! The stack between lines is the contexts open above the top level, up to 28 (the bytes of
//! [`State`]'s levels).

use crate::machine::KEPT as HALF;
use crate::regex;
use crate::{Out, State, is_space, skip_spaces, trim_end};

// The contexts. Java's first, those with its prototype (comments, `goto`, `const`, `%>`)
// before `STRING`.
/// The top level (`main`).
const MAIN: u8 = 0;
/// A class's body (`class-body`), `}` closing it.
const CLASS_BODY: u8 = 1;
/// An enum's body: a class's, and a line of constants.
const ENUM_BODY: u8 = 2;
/// An anonymous class's body, whose `}` is a brace's.
const ANON_BODY: u8 = 3;
/// `{ … }` of statements (`code-block`): a method's, a block's, an initializer's.
const BLOCK: u8 = 4;
/// An enum's constants, to `;` or `}`.
const ENUM_CONSTANTS: u8 = 5;
/// After an enum constant: its arguments and body.
const ENUM_CONSTANT: u8 = 6;
/// `( … )` of code: a group's, a call's.
const PARENS: u8 = 7;
/// A constructor's arguments.
const CTOR_ARGS: u8 = 8;
/// After them: an anonymous class's body.
const CTOR_AFTER: u8 = 9;
/// An assignment's value.
const RHS: u8 = 10;
/// A field's value.
const STATIC_RHS: u8 = 11;
const LAMBDA_PARAMS: u8 = 12;
const LAMBDA_PARAM_LIST: u8 = 13;
const LAMBDA_ARROW: u8 = 14;
const LAMBDA_BODY: u8 = 15;
const ASSERT: u8 = 16;
/// After `for` or `try`: their `( … )`.
const DECL_PARENS: u8 = 17;
const DECL_PARENS_IN: u8 = 18;
const CATCH: u8 = 19;
const CATCH_OPEN: u8 = 20;
const CATCH_PARAMS: u8 = 21;
/// After `::`.
const METHOD_REF: u8 = 22;
const ARRAY_INIT: u8 = 23;
const ANNOT_ARRAY_INIT: u8 = 24;
const ARRAY_DEF: u8 = 25;
const ARRAY_DEF_IN: u8 = 26;
const ARRAY_DEF_AFTER: u8 = 27;
/// After `new`.
const INSTANTIATION: u8 = 28;
const NEW_TYPE: u8 = 29;
const AFTER_NEW_TYPE: u8 = 30;
const AFTER_NEW_GENERIC: u8 = 31;
const CONSTRUCTION: u8 = 32;
/// After a type in code.
const AFTER_TYPE: u8 = 33;
const AFTER_TYPE_DOT: u8 = 34;
const ARRAY_BRACKETS: u8 = 35;
/// A type's package, to its name (`object-type-fqn`).
const TYPE_FQN: u8 = 36;
const GENERIC_INVOCATION: u8 = 37;
const GENERIC_ARG: u8 = 38;
const GENERIC_BOUNDS: u8 = 39;
const MULTI_BOUNDS: u8 = 40;
const GENERIC_DECL: u8 = 41;
const GENERIC_PARAM: u8 = 42;
/// `object-and-array-types`' package, its name, what follows it.
const OBJ_ARRAY_PATH: u8 = 43;
const AFTER_OBJ_ARRAY: u8 = 44;
const OBJ_ARRAY_TYPE: u8 = 45;
/// `object-type-reference`, its package, its name, what follows it.
const TYPE_REF: u8 = 46;
const TYPE_REF_PATH: u8 = 47;
const TYPE_REF_NAME: u8 = 48;
const AFTER_TYPE_REF: u8 = 49;
const CLASS_TYPE: u8 = 50;
const CLASS_NAME: u8 = 51;
const CLASS_EXTENDS: u8 = 52;
const CLASS_SUPERS: u8 = 53;
const INHERITED_REF: u8 = 54;
const INHERITED_PATH: u8 = 55;
const INHERITED_NAME: u8 = 56;
const AFTER_INHERITED: u8 = 57;
const CLASS_BLOCK: u8 = 58;
const ENUM_BLOCK: u8 = 59;
const METHOD: u8 = 60;
const METHOD_PARAMS: u8 = 61;
const THROWS_LIST: u8 = 62;
const ANNOT_DEFAULT: u8 = 63;
const FIELD_OR_METHOD: u8 = 64;
const FIELD: u8 = 65;
const STATIC_ASSIGN: u8 = 66;
/// An annotation's type, its package, its name, what follows it.
const ANNOT_TYPE_REF: u8 = 67;
const ANNOT_PATH: u8 = 68;
const ANNOT_NAME: u8 = 69;
const ANNOT_AFTER: u8 = 70;
/// An annotation's bottom: before its `(`, then in its arguments. One of these two on the
/// stack, or [`ANNOT_DEFAULT`], is an annotation open (`meta.annotation`).
const ANNOT_PARAMS: u8 = 71;
const ANNOT_ARGS: u8 = 72;
const ANNOT_VALUE: u8 = 73;
/// `module-info.java`'s.
const MODULE_KW: u8 = 74;
const MODULE_ID: u8 = 75;
const MODULE_BODY: u8 = 76;
const MODULE_BODY_IN: u8 = 77;
const EXPORTS: u8 = 78;
const EXPORTS_TO: u8 = 79;
const REQUIRES: u8 = 80;
const PROVIDES_WITH: u8 = 81;
const PROVIDES_LIST: u8 = 82;
/// `dot-separated-identifier` under each scope that holds it.
const DOTTED_NS: u8 = 83;
const DOTTED_MODULE: u8 = 84;
const DOTTED_PKG: u8 = 85;
const DOTTED_SMOD: u8 = 86;
const PACKAGE_NAME: u8 = 87;
const IMPORT_BODY: u8 = 88;
const IMPORT_PATH: u8 = 89;
const IMPORT_AFTER: u8 = 90;
const STATIC_IMPORT: u8 = 91;
const STATIC_PATH: u8 = 92;
const STATIC_AFTER: u8 = 93;
// without the prototype
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
/// In an annotation (`meta.annotation`).
const ANNOT: u16 = 8 * WRAP;

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
    SQ = "string.quoted.single.java",
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

/// How many contexts a line can hold; one pushed past it drops the bottom one.
const MAX: usize = 128;
/// How many the state keeps between lines: [`State`]'s levels and withs.
const KEPT: usize = 2 * HALF;

/// Lex `b`, a line without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let mut s = Scan {
        b,
        stack: [MAIN; MAX],
        depth: 1,
        exact: state.is_exact(),
        annot: 0,
        tags: 0,
        text: PLAIN,
        at: 0,
        loop_at: (usize::MAX, 0),
        halt: false,
        arrow: None,
        tag_line: None,
        word_at: std::cell::Cell::new((usize::MAX, 0)),
        re: 0,
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
        s.count_in(c);
    }
    s.depth = 1 + n;
    s.refresh();
    s.run();
    s.fill(b.len());
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

struct Scan<'b, 'c> {
    b: &'b [u8],
    stack: [u8; MAX],
    depth: usize,
    exact: bool,
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
    /// The last `)` followed by `\s*->`, once looked for.
    arrow: Option<Option<usize>>,
    /// Whether the line is `^\s*\*?\s*@`, once looked at.
    tag_line: Option<bool>,
    /// The last whole word looked up: where it starts and ends.
    word_at: std::cell::Cell<(usize, usize)>,
    /// The state of the regex in [`STRING_RE`], which ends with its line.
    re: u8,
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

/// The bytes of the UTF-8 character that starts with `c`.
fn char_len(c: u8) -> usize {
    match c {
        0xf0.. => 4,
        0xe0.. => 3,
        0xc0.. => 2,
        _ => 1,
    }
}

/// The character byte `i` of `b` is in, and where it starts.
#[cold]
fn wide(b: &[u8], i: usize) -> char {
    let s = (i.saturating_sub(3)..=i)
        .rev()
        .find(|&s| b[s] & 0xc0 != 0x80)
        .unwrap_or(i);
    let e = (s + char_len(b[s])).min(b.len());
    std::str::from_utf8(&b[s..e])
        .ok()
        .and_then(|t| t.chars().next())
        .unwrap_or('\u{fffd}')
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

/// The character after the one at `i`.
#[inline]
fn next(b: &[u8], i: usize) -> usize {
    (i + char_len(b[i])).min(b.len())
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
fn cased(
    b: &[u8],
    p: usize,
    first: fn(&[u8], usize) -> bool,
    rest: fn(&[u8], usize) -> bool,
) -> Option<(usize, usize)> {
    let q = run_of(b, p, |b, i| matches!(b[i], b'_' | b'$'));
    first(b, q).then(|| {
        let m = next(b, q);
        (m, run_of(b, m, rest))
    })
}

fn lower_rest(b: &[u8], i: usize) -> bool {
    is_lower(b, i) || is_number(b, i) || matches!(b[i], b'_' | b'$')
}

fn upper_rest(b: &[u8], i: usize) -> bool {
    is_upper(b, i) || is_number(b, i) || matches!(b[i], b'_' | b'$')
}

/// `(?:[_$]*\p{Ll}[\p{Ll}\p{N}_$]*\b)` at `p`: a package's name.
fn lower_word(b: &[u8], p: usize) -> Option<usize> {
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
    cased(b, p, is_lower, lower_rest)
        .is_some_and(|(_, e)| boundary(b, e) && b.get(skip_spaces(b, e)) == Some(&b'.'))
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

/// Where the first byte from `from` that is one of `set` is, or the end: eight bytes at a time
/// (each byte's zero test: a borrow only ever marks bytes above a true match, so the lowest
/// mark is the first).
fn find_any<const N: usize>(b: &[u8], from: usize, set: [u8; N]) -> usize {
    const LO: u64 = 0x0101_0101_0101_0101;
    const HI: u64 = 0x8080_8080_8080_8080;
    let mut i = from;
    while let Some(chunk) = b.get(i..i + 8) {
        let w = u64::from_le_bytes(chunk.try_into().unwrap_or([0; 8]));
        let mut m = 0;
        for x in set {
            let v = w ^ (LO * u64::from(x));
            m |= v.wrapping_sub(LO) & !v & HI;
        }
        if m != 0 {
            return i + (m.trailing_zeros() / 8) as usize;
        }
        i += 8;
    }
    while i < b.len() && !set.contains(&b[i]) {
        i += 1;
    }
    i.min(b.len())
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

impl<'b> Scan<'b, '_> {
    fn top(&self) -> u8 {
        self.stack[self.depth - 1]
    }

    /// `k` here: in an annotation, its variant.
    fn kind(&self, k: u16) -> u16 {
        if self.annot > 0 { k | ANNOT } else { k }
    }

    /// Bytes `s..e` are `leaf`, after the text before them.
    fn put(&mut self, s: usize, e: usize, leaf: u16) {
        self.fill(s);
        let k = self.kind(leaf);
        if k != PLAIN && e > s {
            self.out.push_merged(s, e, k);
        }
        self.at = self.at.max(e);
    }

    /// The text up to `p` is the context's.
    fn fill(&mut self, p: usize) {
        if p > self.at {
            if self.text != PLAIN {
                self.out.push_merged(self.at, p, self.text);
            }
            self.at = p;
        }
    }

    fn refresh(&mut self) {
        let t = match self.top() {
            STRING | STRING_RE => DQ,
            CHAR => SQ,
            COMMENT => BLOCK_TEXT,
            LINE_COMMENT => LINE_TEXT,
            H_OTHER_NAME => W_JD | NAME_OTHER,
            H_CUSTOM_NAME => W_JD | NAME_CUSTOM,
            H_ATTR_NAME => W_ATTR,
            H_DQ => W_DQ,
            H_SQ => W_SQ,
            H_UNQ => W_UNQ,
            H_COMMENT => W_HCOMMENT,
            H_CDATA => W_JD | CDATA_TEXT,
            JD_MAIN.. => W_JD,
            _ => PLAIN,
        };
        self.text = self.kind(t);
    }

    fn count_in(&mut self, c: u8) {
        self.annot += usize::from(matches!(c, ANNOT_PARAMS | ANNOT_ARGS | ANNOT_DEFAULT));
        self.tags += usize::from(c == JD_TAGS);
    }

    fn count_out(&mut self, c: u8) {
        self.annot -= usize::from(matches!(c, ANNOT_PARAMS | ANNOT_ARGS | ANNOT_DEFAULT));
        self.tags -= usize::from(c == JD_TAGS);
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
        let end = if boundary(b, p) && is_word(b, p) {
            word_end(b, p)
        } else {
            p
        };
        self.word_at.set((p, end));
        &b[p..end]
    }

    fn word_kind(&self, p: usize) -> Word {
        lookup(self.word(p))
    }

    fn is_word_at(&self, p: usize, w: &[u8]) -> bool {
        self.word(p) == w
    }

    fn run(&mut self) {
        let mut p = 0;
        loop {
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
        let c = b[p];
        let top = self.top();
        match top {
            STRING | STRING_RE => find_any(b, p + 1, *b"\"\\"),
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
            _ if is_space(c) => {
                // `\s?(?={)` starts at the last space before `{`
                let q = skip_spaces(b, p);
                if q > p + 1 && b.get(q) == Some(&b'{') {
                    q - 1
                } else {
                    q
                }
            }
            BLOCK | PARENS | CTOR_ARGS | RHS | STATIC_RHS | LAMBDA_ARROW | LAMBDA_BODY | ASSERT
            | DECL_PARENS_IN | ARRAY_INIT | ANNOT_ARRAY_INIT | ARRAY_DEF_IN | ANNOT_DEFAULT
            | ANNOT_ARGS | ANNOT_VALUE
                if is_word(b, p) =>
            {
                // patterns past a word's start begin with an identifier, which would have
                // matched at the start unless that is a digit
                let w = self.word(p);
                let e = if w.is_empty() {
                    word_end(b, p)
                } else {
                    p + w.len()
                };
                if c.is_ascii_digit() {
                    run_of(b, p, |b, i| !is_ident_start(b, i)).min(e)
                } else {
                    e
                }
            }
            _ => next(b, p),
        }
    }

    /// The first pattern of the top context that matches at `p`, applied: where the scan goes
    /// on, or `None` when none does.
    fn at(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let top = self.top();
        if p == b.len() {
            return self.eol(top);
        }
        let c = b[p];
        if top < STRING
            && let Some(r) = self.prototype(p, c)
        {
            return Some(r);
        }
        if top >= JD_MAIN {
            if top == STRING_RE {
                return self.in_regex(p, c);
            }
            return self.javadoc(p, c, top);
        }
        match top {
            MAIN => self.main(p, c),
            CLASS_BODY | ANON_BODY => match c {
                b'}' => Some(self.close(
                    p,
                    1,
                    if top == ANON_BODY {
                        BRACE_END
                    } else {
                        BLOCK_END
                    },
                )),
                _ => self.class_body(p, c),
            },
            ENUM_BODY => {
                if c == b'}' {
                    return Some(self.close(p, 1, BLOCK_END));
                }
                if p == 0 && self.enum_line() {
                    return Some(self.enter(0, &[ENUM_CONSTANTS]));
                }
                self.class_body(p, c)
            }
            BLOCK => match c {
                b'}' => Some(self.close(p, 1, BLOCK_END)),
                _ => self.code_block(p, c),
            },
            ENUM_CONSTANTS => match c {
                b';' | b'}' => Some(self.leave(p)),
                b',' => Some(self.token(p, 1, COMMA)),
                _ if is_word(b, p) => {
                    let e = word_end(b, p);
                    self.put(p, e, ENUM_NAME);
                    self.push(e, ENUM_CONSTANT);
                    Some(e)
                }
                _ => None,
            },
            ENUM_CONSTANT => match c {
                b'{' => Some(self.open(p, 1, ENUM_BODY, BLOCK_BEGIN)),
                b'(' => Some(self.open(p, 1, PARENS, PAREN_BEGIN)),
                _ => self.any_pop(p),
            },
            PARENS => match c {
                b')' => Some(self.close(p, 1, PAREN_END)),
                _ => self.illegal_parens(p, c).or_else(|| self.code(p, c)),
            },
            CTOR_ARGS => match c {
                b')' => Some(self.then(p, 1, &[CTOR_AFTER], PAREN_END)),
                _ => self.illegal_parens(p, c).or_else(|| self.code(p, c)),
            },
            CTOR_AFTER => match c {
                b'{' => Some(self.then(p, 1, &[ANON_BODY], BRACE_BEGIN)),
                _ => self.any_pop(p),
            },
            RHS => match c {
                b';' | b')' | b'}' | b',' => Some(self.leave(p)),
                _ => self.code(p, c),
            },
            STATIC_RHS => match c {
                b',' | b';' => Some(self.leave(p)),
                _ => {
                    if self.before_next_field(p) {
                        return Some(self.leave(p));
                    }
                    self.code(p, c)
                        .or_else(|| (c == b')').then(|| self.token(p, 1, STRAY_PAREN)))
                }
            },
            LAMBDA_PARAMS => {
                if c == b'(' {
                    return Some(self.then(p, 1, &[LAMBDA_PARAM_LIST], PAREN_BEGIN));
                }
                let e = ident(b, p)?;
                Some(self.then(p, e - p, &[LAMBDA_ARROW], PARAMETER))
            }
            LAMBDA_PARAM_LIST => match c {
                b')' => Some(self.then(p, 1, &[LAMBDA_ARROW], PAREN_END)),
                _ => self.parameters(p, c),
            },
            LAMBDA_ARROW => (c == b'-' && b.get(p + 1) == Some(&b'>'))
                .then(|| self.then(p, 2, &[LAMBDA_BODY], ARROW)),
            LAMBDA_BODY => match c {
                b')' | b'}' | b';' => Some(self.leave(p)),
                _ => self.code(p, c),
            },
            ASSERT => match c {
                b';' => Some(self.leave(p)),
                b':' => Some(self.token(p, 1, EXPR_SEP)),
                _ => self.code(p, c),
            },
            DECL_PARENS => match c {
                b'(' => Some(self.then(p, 1, &[DECL_PARENS_IN], PAREN_BEGIN)),
                _ => self.any_pop(p),
            },
            DECL_PARENS_IN => match c {
                b')' => Some(self.close(p, 1, PAREN_END)),
                _ => self.open_block(p, c).or_else(|| self.code_block(p, c)),
            },
            CATCH => match c {
                b'(' => Some(self.shift(p, &[CATCH_OPEN])),
                _ => self.any_pop(p),
            },
            CATCH_OPEN => (c == b'(').then(|| self.then(p, 1, &[CATCH_PARAMS], PAREN_BEGIN)),
            CATCH_PARAMS => match c {
                b')' => Some(self.close(p, 1, PAREN_END)),
                b'|' => Some(self.token(p, 1, BAR)),
                _ => self.parameters(p, c),
            },
            METHOD_REF => match ident(b, p) {
                Some(e) => Some(self.close(p, e - p, FUNCTION_REF)),
                None => self.any_pop(p),
            },
            ARRAY_INIT | ANNOT_ARRAY_INIT => match c {
                b'}' => Some(self.close(p, 1, BRACE_END)),
                b'{' => Some(self.open(p, 1, ARRAY_INIT, BRACE_BEGIN)),
                b'@' if top == ANNOT_ARRAY_INIT => Some(self.annotation(p)),
                _ => self.code(p, c),
            },
            ARRAY_DEF => match c {
                b'[' => Some(self.then(p, 1, &[ARRAY_DEF_IN], BRACKET_BEGIN)),
                _ => self.any_pop(p),
            },
            ARRAY_DEF_IN => match c {
                b']' => Some(self.then(p, 1, &[ARRAY_DEF_AFTER], BRACKET_END)),
                _ => self.code(p, c),
            },
            ARRAY_DEF_AFTER => match c {
                b'[' => Some(self.shift(p, &[ARRAY_DEF])),
                b'{' => Some(self.then(p, 1, &[ARRAY_INIT], BRACE_BEGIN)),
                _ => self.any_pop(p),
            },
            INSTANTIATION => {
                if self.word_kind(p) == Word::Primitive {
                    let e = word_end(b, p);
                    return Some(self.then(p, e - p, &[ARRAY_DEF], PRIMITIVE));
                }
                if package_ahead(b, p) {
                    return Some(self.shift(p, &[AFTER_NEW_TYPE, TYPE_FQN]));
                }
                self.new_type(p)
            }
            NEW_TYPE => self.new_type(p),
            AFTER_NEW_TYPE => match c {
                b'[' => Some(self.shift(p, &[ARRAY_DEF])),
                b'(' => Some(self.shift(p, &[CONSTRUCTION])),
                b'<' if b.get(p + 1) == Some(&b'>') => {
                    Some(self.then(p, 2, &[CONSTRUCTION], DIAMOND))
                }
                b'<' => Some(self.shift(p, &[AFTER_NEW_GENERIC, GENERIC_INVOCATION])),
                b'.' => Some(self.then(p, 1, &[NEW_TYPE], DOT)),
                _ => self.any_pop(p),
            },
            AFTER_NEW_GENERIC => match c {
                b'[' => Some(self.shift(p, &[ARRAY_DEF])),
                b'(' => Some(self.then(p, 1, &[CTOR_ARGS], PAREN_BEGIN)),
                _ => self.any_pop(p),
            },
            CONSTRUCTION => match c {
                b'(' => Some(self.then(p, 1, &[CTOR_ARGS], PAREN_BEGIN)),
                _ => self.any_pop(p),
            },
            AFTER_TYPE => match c {
                b'<' => Some(self.shift(p, &[ARRAY_BRACKETS, GENERIC_INVOCATION])),
                b'.' if b.get(p + 1) != Some(&b'.') => {
                    Some(self.then(p, 1, &[AFTER_TYPE_DOT], DOT))
                }
                _ => self.array_brackets(p, c),
            },
            AFTER_TYPE_DOT => {
                if c == b'<' {
                    return Some(self.shift(p, &[GENERIC_INVOCATION]));
                }
                if b[p..].starts_with(b"class") && boundary(b, p + 5) {
                    return Some(self.close(p, 5, VAR_LANGUAGE));
                }
                if let Some(e) = upper_word(b, p) {
                    return Some(self.close(p, e - p, CONSTANT));
                }
                if let Some(e) = type_name(b, p) {
                    return Some(self.then(p, e - p, &[AFTER_TYPE], CLASS));
                }
                self.any_pop(p)
            }
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
                    if b[p..].starts_with(kw) && boundary(b, p + kw.len()) {
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
                if !b[p..].starts_with(b"extends")
                    && !b[p..].starts_with(b"implements")
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
                    if b[p..].starts_with(kw) && boundary(b, p + kw.len()) {
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
            METHOD => self.method(p, c),
            METHOD_PARAMS => match c {
                b')' => Some(self.close(p, 1, PAREN_END)),
                _ => self.parameters(p, c).or_else(|| {
                    (!is_space(c)).then(|| self.close(p, char_len(c), MISSING_PARAM_END))
                }),
            },
            ANNOT_DEFAULT => match c {
                b';' => Some(self.leave(p)),
                _ => self.code(p, c),
            },
            FIELD_OR_METHOD => {
                if self.call_ahead(p) {
                    return Some(self.shift(p, &[METHOD]));
                }
                if !is_space(c) {
                    return Some(self.shift(p, &[FIELD]));
                }
                None
            }
            FIELD => {
                if self.before_next_field(p) {
                    return Some(self.leave(p));
                }
                let found = upper_word(b, p)
                    .map(|e| (e, ENTITY_CONSTANT))
                    .or_else(|| ident(b, p).map(|e| (e, PLAIN)));
                if let Some((e, leaf)) = found {
                    self.put(p, e, leaf);
                    self.push(e, STATIC_ASSIGN);
                    self.push(e, ARRAY_BRACKETS);
                    return Some(e);
                }
                if c == b',' {
                    return Some(self.token(p, 1, COMMA));
                }
                self.any_pop(p)
            }
            STATIC_ASSIGN => match c {
                b'=' => Some(self.then(p, 1, &[STATIC_RHS], ASSIGN)),
                _ => self.any_pop(p),
            },
            ANNOT_TYPE_REF => {
                if package_ahead(b, p) {
                    return Some(self.shift(p, &[ANNOT_PATH]));
                }
                self.annotation_name(p)
            }
            ANNOT_PATH => {
                if let Some(e) = lower_word(b, p) {
                    return Some(self.token(p, e - p, ANNOTATION_PKG));
                }
                if c == b'.' {
                    return Some(self.token(p, 1, DOT));
                }
                self.annotation_name(p)
            }
            ANNOT_NAME => self.annotation_name(p),
            ANNOT_AFTER => match c {
                b'.' => Some(self.then(p, 1, &[ANNOT_NAME], DOT)),
                _ => self.any_pop(p),
            },
            ANNOT_PARAMS => match c {
                b'(' => Some(self.then(p, 1, &[ANNOT_ARGS], PAREN_BEGIN)),
                _ => self.any_pop(p),
            },
            ANNOT_ARGS => {
                if c == b')' {
                    return Some(self.close(p, 1, PAREN_END));
                }
                // `((?:[\p{L}_$][\p{L}\p{N}_$]*))\s*(=)`
                if let Some(e) = ident(b, p) {
                    let q = skip_spaces(b, e);
                    if b.get(q) == Some(&b'=') {
                        self.put(p, e, PARAMETER);
                        self.put(q, q + 1, ASSIGN);
                        self.push(q + 1, ANNOT_VALUE);
                        return Some(q + 1);
                    }
                }
                match c {
                    b'{' => Some(self.open(p, 1, ANNOT_ARRAY_INIT, BRACE_BEGIN)),
                    b'@' => Some(self.annotation(p)),
                    _ => self.code(p, c),
                }
            }
            ANNOT_VALUE => match c {
                b',' | b'}' | b')' => Some(self.leave(p)),
                b'@' => Some(self.annotation(p)),
                _ => self.code(p, c),
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
            JD_CONTENTS => Some(self.enter(n, &[JD_TEXT])),
            JD_LINK_LABEL => Some(self.shift(n, &[JD_LABEL])),
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
            stop: |b, p| b[p] == b'"',
            top_only: false,
            base: self.kind(W_RE),
            text: self.text,
        };
        let (q, st) = regex::embedded(b, p, self.re, &host, &mut self.at, self.out)?;
        self.re = st;
        Some(q)
    }

    /// The prototype: comments, `goto` and `const`, `%>`.
    fn prototype(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        match c {
            b'/' => match b.get(p + 1) {
                Some(b'*') => {
                    if b[p + 2..].starts_with(b"*/") {
                        return Some(self.token(p, 4, EMPTY_COMMENT));
                    }
                    if b.get(p + 2) == Some(&b'*') {
                        self.put(p, p + 3, JD_BEGIN);
                        self.push(p + 3, JD_MAIN);
                        self.push(p + 3, JD_CONTENTS);
                        return Some(p + 3);
                    }
                    Some(self.open(p, 2, COMMENT, BLOCK_MARK))
                }
                Some(b'/') => Some(self.open(p, 2, LINE_COMMENT, LINE_MARK)),
                _ => None,
            },
            b'g' | b'c' if self.word_kind(p) == Word::Illegal => {
                Some(self.token(p, 4 + usize::from(c == b'c'), ILLEGAL_KEYWORD))
            }
            // each context pops, down to the top level, where syntect's guard against a loop
            // leaves the `%` to the patterns after
            b'%' if b.get(p + 1) == Some(&b'>') && self.depth > 1 => Some(self.leave(p)),
            _ => None,
        }
    }

    /// `main`'s own patterns, then `code`.
    fn main(&mut self, p: usize, c: u8) -> Option<usize> {
        match self.word(p) {
            b"package" => {
                self.put(p, p + 7, PACKAGE_KW);
                self.push(p + 7, PACKAGE_NAME);
                return Some(p + 7);
            }
            b"import" => return Some(self.open(p, 6, IMPORT_BODY, IMPORT_KW)),
            b"open" | b"module" if self.module_ahead(p) => {
                return Some(self.enter(p, &[MODULE_KW]));
            }
            _ => {}
        }
        if self.class_ahead(p) {
            return Some(self.enter(p, &[CLASS_TYPE]));
        }
        match c {
            b'@' => return Some(self.annotation(p)),
            b'}' => return Some(self.token(p, 1, STRAY_BRACE)),
            _ => {}
        }
        if let Some(r) = self.modifier(p) {
            return Some(r);
        }
        self.code(p, c)
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
        let mut q = p;
        loop {
            for kw in [&b"class"[..], b"interface", b"@interface", b"enum"] {
                if b[q..].starts_with(kw) && boundary(b, q + kw.len()) {
                    return true;
                }
            }
            let Some(m) = MODIFIERS.iter().find(|m| b[q..].starts_with(m)) else {
                return false;
            };
            let r = q + m.len();
            let s = skip_spaces(b, r);
            if s == r {
                return false;
            }
            q = s;
        }
    }

    /// `storage-modifiers` at `p`.
    fn modifier(&mut self, p: usize) -> Option<usize> {
        (self.word_kind(p) == Word::Modifier).then(|| {
            let e = word_end(self.b, p);
            self.token(p, e - p, MODIFIER)
        })
    }

    /// `class-body` at `p`.
    fn class_body(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if self.class_ahead(p) {
            return Some(self.enter(p, &[CLASS_TYPE]));
        }
        if c == b'@' {
            return Some(self.annotation(p));
        }
        // fields-and-methods
        let w = self.word_kind(p);
        if w == Word::Void {
            self.put(p, p + 4, VOID);
            self.push(p + 4, METHOD);
            return Some(p + 4);
        }
        if self.call_ahead(p) {
            return Some(self.enter(p, &[METHOD]));
        }
        if package_ahead(b, p) {
            return Some(self.enter(p, &[FIELD_OR_METHOD, AFTER_OBJ_ARRAY, TYPE_FQN]));
        }
        if boundary(b, p)
            && let Some(e) = type_name(b, p)
        {
            self.put(p, e, CLASS);
            self.push(e, FIELD_OR_METHOD);
            self.push(e, AFTER_OBJ_ARRAY);
            return Some(e);
        }
        if w == Word::Primitive {
            let e = word_end(b, p);
            self.put(p, e, PRIMITIVE);
            self.push(e, FIELD_OR_METHOD);
            self.push(e, ARRAY_BRACKETS);
            return Some(e);
        }
        if let Some(r) = self.constants(p, c) {
            return Some(r);
        }
        if let Some(r) = self.modifier(p) {
            return Some(r);
        }
        match c {
            b'{' => Some(self.open(p, 1, BLOCK, BLOCK_BEGIN)),
            b',' => Some(self.token(p, 1, COMMA)),
            b';' => Some(self.token(p, 1, TERMINATOR)),
            b'<' => Some(self.enter(p, &[GENERIC_DECL])),
            _ => None,
        }
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

    /// `before-next-field`: `(?=\b(?:modifiers|primitives|void)\b)` at `p`.
    fn before_next_field(&self, p: usize) -> bool {
        matches!(
            self.word_kind(p),
            Word::Modifier | Word::Primitive | Word::Void
        )
    }

    /// `before-next-import`: `(?=\bimport\b)`, `(?=\b(?:modifiers|class|interface|enum)\b)`.
    fn before_next_import(&self, p: usize) -> bool {
        matches!(self.word(p), b"import" | b"class" | b"interface" | b"enum")
            || self.word_kind(p) == Word::Modifier
    }

    /// `method`'s patterns at `p`.
    fn method(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if let Some(e) = ident(b, p) {
            let q = skip_spaces(b, e);
            if b.get(q) == Some(&b'(') {
                let leaf = if type_name(b, p) == Some(e) {
                    ENTITY_CTOR
                } else {
                    ENTITY_FUNCTION
                };
                return Some(self.token(p, e - p, leaf));
            }
        }
        match c {
            b'(' => return Some(self.open(p, 1, METHOD_PARAMS, PAREN_BEGIN)),
            b'{' => return Some(self.then(p, 1, &[BLOCK], BLOCK_BEGIN)),
            _ => {}
        }
        match self.word(p) {
            b"throws" => {
                self.put(p, p + 6, THROWS_KW);
                self.push(p + 6, THROWS_LIST);
                self.push(p + 6, TYPE_REF);
                Some(p + 6)
            }
            b"default" => Some(self.open(p, 7, ANNOT_DEFAULT, DEFAULT_DECL)),
            _ => self.any_pop(p),
        }
    }

    /// `parameters` at `p`.
    fn parameters(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        let w = self.word_kind(p);
        if self.is_word_at(p, b"final") {
            return Some(self.token(p, 5, MODIFIER));
        }
        if c == b'@' {
            return Some(self.annotation(p));
        }
        if w == Word::Primitive {
            let e = word_end(b, p);
            self.put(p, e, PRIMITIVE);
            self.push(e, ARRAY_BRACKETS);
            return Some(e);
        }
        if let Some(r) = self.object_and_array_types(p) {
            return Some(r);
        }
        if b[p..].starts_with(b"...") {
            return Some(self.token(p, 3, VARIADIC));
        }
        if let Some(e) = ident(b, p) {
            self.put(p, e, PARAMETER);
            self.push(e, ARRAY_BRACKETS);
            return Some(e);
        }
        (c == b',').then(|| self.token(p, 1, COMMA))
    }

    /// `object-and-array-types` at `p`.
    fn object_and_array_types(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if package_ahead(b, p) {
            return Some(self.enter(p, &[OBJ_ARRAY_PATH]));
        }
        if boundary(b, p)
            && let Some(w) = PRIMITIVES.iter().find(|w| b[p..].starts_with(w))
            && b.get(skip_spaces(b, p + w.len())) == Some(&b'[')
        {
            self.put(p, p + w.len(), PRIMITIVE);
            self.push(p + w.len(), ARRAY_BRACKETS);
            return Some(p + w.len());
        }
        if boundary(b, p)
            && let Some(e) = type_name(b, p)
        {
            self.put(p, e, CLASS);
            self.push(e, AFTER_OBJ_ARRAY);
            return Some(e);
        }
        None
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

    /// `object-type-instantiation-no-fqn` at `p`.
    fn new_type(&mut self, p: usize) -> Option<usize> {
        match type_name(self.b, p) {
            Some(e) => Some(self.then(p, e - p, &[AFTER_NEW_TYPE], CLASS)),
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
    fn annotation_name(&mut self, p: usize) -> Option<usize> {
        match type_name(self.b, p) {
            Some(e) => Some(self.then(p, e - p, &[ANNOT_AFTER], ANNOTATION)),
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

    /// `illegal-parens-terminators` at `p`.
    fn illegal_parens(&mut self, p: usize, c: u8) -> Option<usize> {
        if c == b';' {
            return Some(self.close(p, 1, STRAY_TERMINATOR));
        }
        self.open_block(p, c)
    }

    /// `illegal-open-block`: `\s?(?={)` at `p`.
    fn open_block(&mut self, p: usize, c: u8) -> Option<usize> {
        if c == b'{' {
            return Some(self.leave(p));
        }
        (is_space(c) && self.b.get(p + 1) == Some(&b'{'))
            .then(|| self.close(p, 1, STRAY_TERMINATOR))
    }

    /// `annotations` at `p`, an `@`.
    fn annotation(&mut self, p: usize) -> usize {
        self.push(p, ANNOT_PARAMS);
        self.push(p, ANNOT_TYPE_REF);
        self.token(p, 1, ANNOT_MARK)
    }

    /// `code-block` at `p`: modifiers, `var`, then `code`, annotations, a stray `)`.
    fn code_block(&mut self, p: usize, c: u8) -> Option<usize> {
        match self.word_kind(p) {
            Word::Modifier => return self.modifier(p),
            Word::Var => return Some(self.token(p, 3, VAR)),
            _ => {}
        }
        if let Some(r) = self.code(p, c) {
            return Some(r);
        }
        match c {
            b'@' => Some(self.annotation(p)),
            b')' => Some(self.token(p, 1, STRAY_PAREN)),
            _ => None,
        }
    }

    /// `code` at `p`.
    fn code(&mut self, p: usize, c: u8) -> Option<usize> {
        if c >= 0x80 || c == b'$' || c == b'_' || c.is_ascii_alphanumeric() {
            self.code_word(p)
        } else {
            self.code_punct(p, c)
        }
    }

    /// `code` at a byte of a word, or `$`.
    fn code_word(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let at_word = boundary(b, p);
        let w = self.word(p);
        let e = p + w.len();
        let name = lookup(w);
        // constants-and-special-vars
        match name {
            Word::Language => return Some(self.token(p, e - p, LANGUAGE)),
            Word::Special => return Some(self.token(p, e - p, VAR_LANGUAGE)),
            _ => {}
        }
        if at_word
            && b[p].is_ascii_digit()
            && let Some(r) = self.number(p)
        {
            return Some(r);
        }
        // the identifier at `p`: past ASCII's `\w` only over `$` and bytes past ASCII
        let id = is_ident_start(b, p).then(|| {
            if e == p {
                ident_end(b, next(b, p))
            } else if b.get(e).is_some_and(|&c| c == b'$' || c >= 0x80) {
                ident_end(b, e)
            } else {
                e
            }
        });
        // lambdas, `[\p{L}_$][\p{L}\p{N}_$]*\s*->`
        if let Some(ie) = id {
            let q = skip_spaces(b, ie);
            if b[q..].starts_with(b"->") {
                return Some(self.enter(p, &[LAMBDA_PARAMS]));
            }
        }
        // anonymous-classes-and-new
        if name == Word::New {
            return Some(self.open(p, 3, INSTANTIATION, NEW));
        }
        // keywords-control
        if let Word::Control(leaf) = name {
            let n = e - p;
            return Some(match leaf {
                CATCH_KW => self.open(p, n, CATCH, leaf),
                TRY_KW | FOR_KW => self.open(p, n, DECL_PARENS, leaf),
                ASSERT_KW => self.open(p, n, ASSERT, leaf),
                _ => self.token(p, n, leaf),
            });
        }
        if w == b"default" {
            return Some(self.token(p, 7, DEFAULT_KW));
        }
        // method-invocations
        if let Some(ie) = id {
            let q = skip_spaces(b, ie);
            if b.get(q) == Some(&b'(') {
                self.put(p, ie, FUNCTION_CALL);
                self.put(q, q + 1, PAREN_BEGIN);
                self.push(q + 1, PARENS);
                return Some(q + 1);
            }
        }
        if at_word {
            // uppercase-identifiers
            let single = !w.is_empty() && is_upper(b, p) && next(b, p) == e;
            if matches!(w, b"UUID" | b"URL" | b"URI") || single {
                return Some(self.open(p, e - p, AFTER_TYPE, CLASS));
            }
            if (matches!(b[p], b'_' | b'$') || is_upper(b, p))
                && let Some(ue) = upper_word(b, p)
            {
                return Some(self.token(p, ue - p, CONSTANT));
            }
            // all-types
            if name == Word::Primitive {
                self.put(p, e, PRIMITIVE);
                self.push(e, ARRAY_BRACKETS);
                return Some(e);
            }
            // a package's name runs to the identifier's end, then `.`
            if id.is_some_and(|ie| b.get(ie) == Some(&b'.')) && self.fqn_ahead(p) {
                return Some(self.enter(p, &[AFTER_TYPE, TYPE_FQN]));
            }
            if let Some(te) = type_name(b, p)
                && let Some(te) = back_to(b, next(b, p), te, |_| true)
            {
                self.put(p, te, CLASS);
                self.push(te, AFTER_TYPE);
                return Some(te);
            }
        }
        // keywords
        (name == Word::Instanceof).then(|| self.token(p, e - p, INSTANCEOF))
    }

    /// `(?=\b(?:(?:[_$]*\p{Ll}[\p{Ll}\p{N}_$]*\b)\.)+\p{Lu})` at `p`.
    fn fqn_ahead(&self, p: usize) -> bool {
        let b = self.b;
        let mut q = p;
        let mut n = 0;
        loop {
            if n > 0 && is_upper(b, q) {
                return true;
            }
            match cased(b, q, is_lower, lower_rest) {
                Some((_, e)) if b.get(e) == Some(&b'.') && word_before(b, e) => {
                    q = e + 1;
                    n += 1;
                }
                _ => return false,
            }
        }
    }

    /// `code` at a byte that isn't a word's.
    fn code_punct(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        let next = b.get(p + 1).copied();
        let (n, leaf) = match c {
            b'.' => {
                if let Some(r) = self.number(p) {
                    return Some(r);
                }
                // `(\.)\s*(?=<)`
                let q = skip_spaces(b, p + 1);
                if b.get(q) == Some(&b'<') {
                    self.put(p, p + 1, DOT);
                    self.push(q, GENERIC_INVOCATION);
                    return Some(q);
                }
                // `(\.)(class\b)?`
                self.put(p, p + 1, DOT);
                if b[p + 1..].starts_with(b"class") && boundary(b, p + 6) {
                    return Some(self.token(p + 1, 5, VAR_LANGUAGE));
                }
                return Some(p + 1);
            }
            b'(' => {
                if self.arrow_after(p) {
                    return Some(self.enter(p, &[LAMBDA_PARAMS]));
                }
                return Some(self.open(p, 1, PARENS, PAREN_BEGIN));
            }
            b'"' if regex_string(b, p) => {
                self.re = 0;
                return Some(self.open(p, 1, STRING_RE, DQ_BEGIN));
            }
            b'"' => return Some(self.open(p, 1, STRING, DQ_BEGIN)),
            b'\'' => return Some(self.open(p, 1, CHAR, SQ_BEGIN)),
            b'{' => return Some(self.open(p, 1, BLOCK, BLOCK_BEGIN)),
            b'=' if next != Some(b'=') => {
                self.put(p, p + 1, ASSIGN);
                self.push(p + 1, RHS);
                return Some(p + 1);
            }
            b'|' | b'&' | b'^' | b'*' | b'/' | b'+' | b'-' if next == Some(b'=') => {
                self.put(p, p + 2, ASSIGN);
                self.push(p + 2, RHS);
                return Some(p + 2);
            }
            b':' if next == Some(b':') => {
                self.put(p, p + 2, DOUBLE_COLON);
                self.push(p + 2, METHOD_REF);
                return Some(p + 2);
            }
            b'?' | b':' => (1, TERNARY),
            b'<' if next == Some(b'<') => (2, BITSHIFT),
            b'>' if next == Some(b'>') => {
                (if b.get(p + 2) == Some(&b'>') { 3 } else { 2 }, BITSHIFT)
            }
            b'=' | b'!' if next == Some(b'=') => (2, COMPARISON),
            b'<' if matches!(next, Some(b'=' | b'>')) => (2, COMPARISON),
            b'>' if next == Some(b'=') => (2, COMPARISON),
            b'<' | b'>' => (1, COMPARISON),
            b'-' | b'+' if next == Some(c) => (2, INCDEC),
            b'-' | b'+' | b'*' | b'/' | b'%' => (1, ARITH),
            b'!' => (1, LOGICAL),
            b'&' | b'|' if next == Some(c) => (2, LOGICAL),
            b'~' | b'^' | b'&' | b'|' => (1, BITWISE),
            b',' => (1, COMMA),
            b';' => (1, TERMINATOR),
            _ => return None,
        };
        Some(self.token(p, n, leaf))
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

    /// `constants-and-special-vars` at `p`.
    fn constants(&mut self, p: usize, c: u8) -> Option<usize> {
        match self.word_kind(p) {
            Word::Language => {
                let e = word_end(self.b, p);
                return Some(self.token(p, e - p, LANGUAGE));
            }
            Word::Special => {
                let e = word_end(self.b, p);
                return Some(self.token(p, e - p, VAR_LANGUAGE));
            }
            _ => {}
        }
        if (c == b'.' || (c.is_ascii_digit() && boundary(self.b, p)))
            && let Some(r) = self.number(p)
        {
            return Some(r);
        }
        None
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

    // Javadoc, and the HTML in it

    /// Whether the line is `^\s*\*?\s*@`.
    fn is_tag_line(&mut self) -> bool {
        let b = self.b;
        *self.tag_line.get_or_insert_with(|| {
            let mut q = skip_spaces(b, 0);
            if b.get(q) == Some(&b'*') {
                q = skip_spaces(b, q + 1);
            }
            b.get(q) == Some(&b'@')
        })
    }

    /// The `with_prototype`s of the levels below: `(?=\*/)` above `contents`, and
    /// `(?=^\s*\*?\s*@)` from the block tags up.
    fn jd_withs(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        // a pop that would loop is passed over for the patterns after it
        if self.loop_at == (p, self.depth) {
            return None;
        }
        if b[p] == b'*' && b.get(p + 1) == Some(&b'/') {
            return Some(self.leave(p));
        }
        if p == 0 && self.tags > 0 && self.is_tag_line() {
            return Some(self.leave(p));
        }
        None
    }

    /// Javadoc's prototype at the line's start: `^\s*(\*)(?!/)\s*(?!\s*@)`.
    fn jd_star(&mut self) -> Option<usize> {
        let b = self.b;
        let q = skip_spaces(b, 0);
        if b.get(q) != Some(&b'*') || b.get(q + 1) == Some(&b'/') {
            return None;
        }
        let e = skip_spaces(b, q + 1);
        if b.get(e) == Some(&b'@') {
            return None;
        }
        self.put(q, q + 1, W_JD | JD_STAR);
        self.fill(e);
        Some(e)
    }

    /// The Javadoc and HTML contexts at `p`.
    fn javadoc(&mut self, p: usize, c: u8, top: u8) -> Option<usize> {
        let b = self.b;
        if top == JD_MAIN {
            return (c == b'*' && b.get(p + 1) == Some(&b'/')).then(|| self.close(p, 2, JD_END));
        }
        if let Some(r) = self.jd_withs(p) {
            return Some(r);
        }
        // the prototype, but in `contents`, below the tags and in HTML's contexts
        if p == 0
            && !matches!(top, JD_CONTENTS | JD_HOLD)
            && top < H_TAG
            && let Some(e) = self.jd_star()
        {
            return Some(e);
        }
        match top {
            JD_CONTENTS => {
                // `\s*(?=@)`, then `` pushing the text
                let q = skip_spaces(b, p);
                if b.get(q) == Some(&b'@') {
                    return Some(self.tags_at(p, q));
                }
                Some(self.enter(p, &[JD_TEXT]))
            }
            JD_TEXT => {
                if p == 0 && self.is_tag_line() {
                    // `^\s*(\*)?\s*(?=@)`
                    let mut q = skip_spaces(b, 0);
                    if b[q] == b'*' {
                        self.put(q, q + 1, W_JD | JD_STAR);
                        q = skip_spaces(b, q + 1);
                    }
                    return Some(self.tags_at(0, q));
                }
                self.inline_formatting(p, c)
            }
            JD_HOLD => (p == 0 && self.is_tag_line()).then(|| self.leave(p)),
            JD_TAGS => {
                if c != b'@' {
                    return None;
                }
                let tags: [(&[u8], u16, u16, &[u8]); 16] = [
                    (b"param", TAG_PARAM, TAG_PARAM_AT, &[JD_TAG, JD_PARAM_NAME]),
                    (b"see", TAG_SEE, TAG_SEE_AT, &[JD_TAG, JD_SEE]),
                    (b"throws", TAG_THROWS, TAG_THROWS_AT, &[JD_TAG, JD_REF]),
                    (b"exception", TAG_THROWS, TAG_THROWS_AT, &[JD_TAG, JD_REF]),
                    (b"uses", TAG_USES, TAG_USES_AT, &[JD_TAG, JD_REF]),
                    (b"provides", TAG_USES, TAG_USES_AT, &[JD_TAG, JD_REF]),
                    (b"return", TAG_DOC, TAG_DOC_AT, &[JD_TAG]),
                    (b"deprecated", TAG_DOC, TAG_DOC_AT, &[JD_TAG]),
                    (b"author", TAG_DOC, TAG_DOC_AT, &[JD_TAG]),
                    (b"version", TAG_DOC, TAG_DOC_AT, &[JD_TAG]),
                    (b"since", TAG_DOC, TAG_DOC_AT, &[JD_TAG]),
                    (b"apiNote", TAG_DOC, TAG_DOC_AT, &[JD_TAG]),
                    (b"implNote", TAG_DOC, TAG_DOC_AT, &[JD_TAG]),
                    (b"implSpec", TAG_DOC, TAG_DOC_AT, &[JD_TAG]),
                    (b"moduleGraph", TAG_DOC, TAG_DOC_AT, &[JD_TAG]),
                    (b"serial", TAG_DOC, TAG_DOC_AT, &[JD_TAG]),
                ];
                let w = &b[p + 1..word_end(b, p + 1)];
                let w = match w {
                    b"serialField" | b"serialData" => &b"serial"[..],
                    _ => w,
                };
                if let Some(&(_, leaf, at, cs)) = tags.iter().find(|t| t.0 == w) {
                    let e = word_end(b, p + 1);
                    for &cx in cs {
                        self.push(p, cx);
                    }
                    self.put(p, p + 1, W_JD | at);
                    self.put(p + 1, e, W_JD | leaf);
                    return Some(e);
                }
                Some(self.close(p, 1, W_JD))
            }
            JD_TAG => self.inline_formatting(p, c),
            JD_PARAM_NAME => (!is_space(c)).then(|| {
                let e = next_of(b, p, is_space);
                self.close(p, e - p, W_JD | JD_PARAM)
            }),
            JD_SEE => match c {
                b'\'' | b'<' => Some(self.leave(p)),
                _ if !is_space(c) => Some(self.shift(p, &[JD_REF])),
                _ => None,
            },
            JD_REF | JD_IREF => {
                if top == JD_IREF && c == b'}' {
                    return Some(self.leave(p));
                }
                let e = self.reference(p)?;
                let to = if top == JD_REF {
                    JD_REF_AFTER
                } else {
                    JD_IREF_AFTER
                };
                Some(self.then(p, e - p, &[to], W_JD | LINK))
            }
            JD_REF_AFTER | JD_IREF_AFTER => {
                if top == JD_IREF_AFTER && c == b'}' {
                    return Some(self.leave(p));
                }
                if c == b'(' {
                    let to = if top == JD_REF_AFTER {
                        JD_REF_PARENS
                    } else {
                        JD_IREF_PARENS
                    };
                    return Some(self.then(p, 1, &[to], W_JD | LINK));
                }
                Some(self.leave(p))
            }
            JD_REF_PARENS | JD_IREF_PARENS => {
                if top == JD_IREF_PARENS && c == b'}' {
                    return Some(self.leave(p));
                }
                if c == b')' {
                    return Some(self.close(p, 1, W_JD | LINK));
                }
                // `.`, to where another pattern could start
                let e = next_of(b, p + 1, |c| matches!(c, b')' | b'}' | b'*'));
                Some(self.token(p, e - p, W_JD | LINK))
            }
            JD_RAW | JD_BRACES => match c {
                b'}' => Some(self.close(p, 1, W_JD | if top == JD_RAW { INLINE_END } else { RAW })),
                b'{' => Some(self.open(p, 1, JD_BRACES, W_JD | RAW)),
                _ if p == 0 && top == JD_RAW && is_space(c) => {
                    // `^\s+`
                    let e = skip_spaces(b, 0);
                    self.fill(e);
                    Some(e)
                }
                _ => {
                    // `.`, to where another pattern could start
                    let e = next_of(b, p + 1, |c| matches!(c, b'{' | b'}' | b'*'));
                    Some(self.token(p, e - p, W_JD | RAW))
                }
            },
            JD_INLINE => (c == b'}').then(|| self.close(p, 1, W_JD | INLINE_END)),
            JD_LINK_LABEL => {
                let e = skip_spaces(b, p);
                self.fill(e);
                Some(self.shift(e, &[JD_LABEL]))
            }
            JD_LABEL => {
                if let Some(r) = self.html(p, c) {
                    return Some(r);
                }
                (c == b'}').then(|| self.leave(p))
            }
            _ => self.html_context(p, c, top),
        }
    }

    /// The block tags, pushed by a match from `p` to `q` (`contents_0` or `_3` under them).
    fn tags_at(&mut self, p: usize, q: usize) -> usize {
        if q == p {
            return self.enter(p, &[JD_HOLD, JD_TAGS]);
        }
        self.push(q, JD_HOLD);
        self.push(q, JD_TAGS);
        q
    }

    /// `reference`'s `(?:(?:ident)\.)*(?:ident)(?:#(?:ident))?|#(?:ident)` at `p`.
    fn reference(&self, p: usize) -> Option<usize> {
        let b = self.b;
        let hash = |q: usize| {
            if b.get(q) == Some(&b'#') {
                ident(b, q + 1)
            } else {
                None
            }
        };
        let Some(mut e) = ident(b, p) else {
            return hash(p);
        };
        while b.get(e) == Some(&b'.')
            && let Some(f) = ident(b, e + 1)
        {
            e = f;
        }
        Some(hash(e).unwrap_or(e))
    }

    /// `inline-formatting`: the inline tags, then HTML.
    fn inline_formatting(&mut self, p: usize, c: u8) -> Option<usize> {
        if c == b'{'
            && let Some(r) = self.inline_tag(p)
        {
            return Some(r);
        }
        self.html(p, c)
    }

    /// `javadoc-inline-tags` at `p`, a `{`.
    fn inline_tag(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if b.get(p + 1) != Some(&b'@') {
            return None;
        }
        let s = p + 2;
        let w = &b[s..word_end(b, s)];
        let e = s + w.len();
        // `({)((@)(?:code|literal))(?:\s+|(?=\}))`
        for kw in [&b"code"[..], b"literal"] {
            let k = s + kw.len();
            if b[s..].starts_with(kw) && (k == b.len() || is_space(b[k]) || b[k] == b'}') {
                let q = skip_spaces(b, k);
                return Some(self.inline_open(p, k, TAG_CODE, TAG_CODE_AT, &[JD_RAW], q));
            }
        }
        match w {
            b"link" | b"linkplain" => {
                let cs = [JD_INLINE, JD_LINK_LABEL, JD_IREF];
                Some(self.inline_open(p, e, TAG_LINK, TAG_LINK_AT, &cs, e))
            }
            b"value" => {
                Some(self.inline_open(p, e, TAG_VALUE, TAG_VALUE_AT, &[JD_INLINE, JD_IREF], e))
            }
            b"index" | b"extLink" => {
                Some(self.inline_open(p, e, TAG_DOC, TAG_DOC_AT, &[JD_INLINE], e))
            }
            b"inheritDoc" if b.get(e) == Some(&b'}') => {
                self.put(p, p + 1, W_JD | INLINE_BEGIN);
                self.put(p + 1, s, W_JD | TAG_DOC_AT);
                self.put(s, e, W_JD | TAG_DOC);
                Some(self.token(e, 1, W_JD | INLINE_END))
            }
            _ => None,
        }
    }

    /// An inline tag at `p`, its name ending at `e`, pushing `cs`; the match ends at `to`.
    fn inline_open(
        &mut self,
        p: usize,
        e: usize,
        leaf: u16,
        at: u16,
        cs: &[u8],
        to: usize,
    ) -> usize {
        for &c in cs {
            self.push(p, c);
        }
        self.put(p, p + 1, W_JD | INLINE_BEGIN);
        self.put(p + 1, p + 2, W_JD | at);
        self.put(p + 2, e, W_JD | leaf);
        self.fill(to);
        to
    }

    /// HTML's `main` at `p`, in Javadoc: tags, comments, entities.
    fn html(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if c == b'&' {
            return self.entity(p, W_JD);
        }
        if c != b'<' {
            return None;
        }
        let rest = &b[p + 1..];
        if rest.starts_with(b"!--") {
            // `(<!--)(-?>)?`
            self.push(p, H_COMMENT);
            self.put(p, p + 4, W_HCOMMENT | HCOMMENT_BEGIN);
            let mut e = p + 4;
            if b[e..].starts_with(b"->") {
                e += 2;
            } else if b[e..].starts_with(b">") {
                e += 1;
            }
            self.put(p + 4, e, W_HCOMMENT | HCOMMENT_BAD);
            return Some(e);
        }
        if rest.starts_with(b"![CDATA[") {
            self.push(p, H_CDATA);
            self.put(p, p + 3, W_JD | TAG_OPEN);
            self.put(p + 3, p + 8, W_JD | CDATA_KW);
            return Some(self.token(p + 8, 1, W_JD | TAG_OPEN));
        }
        if rest.starts_with(b"!DOCTYPE") {
            for cx in [DT_META, DT_CONTENT, DT_TYPE, DT_NAME] {
                self.push(p, cx);
            }
            self.put(p, p + 2, W_JD | TAG_OPEN);
            return Some(self.token(p + 2, 7, W_JD | DOCTYPE_KW));
        }
        if rest.starts_with(b"?xml") {
            self.push(p, H_PI);
            self.put(p, p + 2, W_JD | TAG_OPEN);
            return Some(self.token(p + 2, 3, W_JD | NAME_XML));
        }
        let close = rest.first() == Some(&b'/');
        let s = p + 1 + usize::from(close);
        let e = (s..b.len())
            .find(|&i| ends_tag_name(b, i))
            .unwrap_or(b.len());
        let name = &b[s..e];
        let lower = |w: &[u8]| name.eq_ignore_ascii_case(w);
        if !close && (lower(b"style") || lower(b"script")) {
            let leaf = if lower(b"style") {
                NAME_STYLE
            } else {
                NAME_SCRIPT
            };
            self.push(p, H_EMBED_TAG);
            self.put(p, s, W_JD | TAG_OPEN);
            return Some(self.token(s, e - s, W_JD | leaf));
        }
        if !name.is_empty() {
            let found = HTML_TAGS
                .iter()
                .find(|(names, _, _)| names.iter().any(|w| lower(w)));
            if let Some(&(_, leaf, cx)) = found {
                self.push(p, cx);
                self.put(p, s, W_JD | TAG_OPEN);
                return Some(self.token(s, e - s, W_JD | leaf));
            }
        }
        if b.get(s).is_some_and(u8::is_ascii_alphabetic) {
            let custom = name.contains(&b'-');
            self.push(p, H_TAG_SC);
            self.push(p, if custom { H_CUSTOM_NAME } else { H_OTHER_NAME });
            return Some(self.token(p, s - p, W_JD | TAG_OPEN));
        }
        if rest.first() == Some(&b'>') {
            return Some(self.token(p, 2, W_JD | INCOMPLETE));
        }
        None
    }

    /// `entities` at `p`, an `&`, in `wrap`.
    fn entity(&mut self, p: usize, wrap: u16) -> Option<usize> {
        let b = self.b;
        let (mark, body, leaves) = if b[p + 1..].starts_with(b"#x") || b[p + 1..].starts_with(b"#X")
        {
            (
                3,
                next_of(b, p + 3, |c| !c.is_ascii_hexdigit()),
                [ENT_HEX, ENT_HEX_MARK, ENT_HEX_END],
            )
        } else if b.get(p + 1) == Some(&b'#') {
            (
                2,
                next_of(b, p + 2, |c| !c.is_ascii_digit()),
                [ENT_DEC, ENT_DEC_MARK, ENT_DEC_END],
            )
        } else {
            (
                1,
                next_of(b, p + 1, |c| !c.is_ascii_alphanumeric()),
                [ENT_NAMED, ENT_NAMED_MARK, ENT_NAMED_END],
            )
        };
        if body == p + mark || b.get(body) != Some(&b';') {
            return None;
        }
        self.put(p, p + mark, wrap | leaves[1]);
        self.put(p + mark, body, wrap | leaves[0]);
        Some(self.token(body, 1, wrap | leaves[2]))
    }

    /// HTML's contexts at `p`.
    fn html_context(&mut self, p: usize, c: u8, top: u8) -> Option<usize> {
        let b = self.b;
        match top {
            H_TAG | H_TAG_SC | H_EMBED_TAG => {
                if c == b'>' {
                    if top == H_EMBED_TAG {
                        return Some(self.then(p, 1, &[H_EMBED_TEXT], W_JD | TAG_CLOSE));
                    }
                    return Some(self.close(p, 1, W_JD | TAG_CLOSE));
                }
                if c == b'/' && b.get(p + 1) == Some(&b'>') && top != H_TAG {
                    return Some(self.close(p, 2, W_JD | TAG_CLOSE));
                }
                self.attributes(p, c)
            }
            H_EMBED_TEXT => {
                let rest = &b[p..];
                let closes = |w: &[u8]| {
                    rest.len() >= 2 + w.len()
                        && rest.starts_with(b"</")
                        && rest[2..2 + w.len()].eq_ignore_ascii_case(w)
                };
                (closes(b"style") || closes(b"script")).then(|| self.leave(p))
            }
            H_PI => {
                if c == b'?' && b.get(p + 1) == Some(&b'>') {
                    return Some(self.close(p, 2, W_JD | TAG_CLOSE));
                }
                self.generic_attribute(p, c)
                    .or_else(|| self.html_string(p, c))
            }
            H_OTHER_NAME => ends_tag_name(b, p).then(|| self.leave(p)),
            H_CUSTOM_NAME => {
                if ends_tag_name(b, p) {
                    return Some(self.leave(p));
                }
                let ok = |i: usize| {
                    let c = b[i];
                    matches!(c, b'-' | b'_' | b'a'..=b'z' | b'0'..=b'9') || c >= 0x80
                };
                if c == b'\\' && p + 1 < b.len() {
                    let e = next(b, p + 1);
                    return Some(self.token(p, e - p, W_JD | NAME_CUSTOM));
                }
                if ok(p) {
                    let e = run_of(b, p, |_, i| ok(i));
                    return Some(self.token(p, e - p, W_JD | NAME_CUSTOM));
                }
                Some(self.token(p, char_len(c), W_JD | NAME_CUSTOM_BAD))
            }
            H_ATTR_NAME => {
                if is_html_space(b, p) || matches!(c, b'=' | b'/' | b'>') {
                    return Some(self.leave(p));
                }
                if matches!(c, b'"' | b'\'' | b'`' | b'<') {
                    return Some(self.token(p, 1, W_ATTR | BAD_NAME));
                }
                let e = next_of(b, p + 1, |c| {
                    matches!(
                        c,
                        b'\t'
                            | b'\n'
                            | b'\x0c'
                            | b' '
                            | b'='
                            | b'/'
                            | b'>'
                            | b'"'
                            | b'\''
                            | b'`'
                            | b'<'
                            | b'*'
                    )
                });
                self.fill(e);
                Some(e)
            }
            H_EQ | H_EMBED_EQ => match c {
                b'=' => {
                    let to = if top == H_EQ { H_VALUE } else { H_EMBED_VALUE };
                    Some(self.then(p, 1, &[to], W_JD | KEY_VALUE))
                }
                _ => self.any_pop(p),
            },
            H_VALUE => match c {
                b'"' => Some(self.then(p, 1, &[H_DQ], W_DQ | STR_BEGIN)),
                b'\'' => Some(self.then(p, 1, &[H_SQ], W_SQ | STR_BEGIN)),
                b'\t' | b'\n' | b'\x0c' | b' ' | b'=' | b'>' => self.any_pop(p),
                _ => Some(self.shift(p, &[H_UNQ])),
            },
            H_EMBED_VALUE => match c {
                b'"' => Some(self.then(p, 1, &[H_EMBED_DQ], W_JD | EMBED_DQ_BEGIN)),
                b'\'' => Some(self.then(p, 1, &[H_EMBED_SQ], W_JD | EMBED_SQ_BEGIN)),
                _ => self.any_pop(p),
            },
            H_EMBED_DQ | H_EMBED_SQ => {
                let (q, leaf) = if top == H_EMBED_DQ {
                    (b'"', EMBED_DQ_END)
                } else {
                    (b'\'', EMBED_SQ_END)
                };
                (c == q).then(|| self.close(p, 1, W_JD | leaf))
            }
            H_DQ | H_SQ => {
                let (q, wrap) = if top == H_DQ {
                    (b'"', W_DQ)
                } else {
                    (b'\'', W_SQ)
                };
                if c == q {
                    return Some(self.close(p, 1, wrap | STR_END));
                }
                if c == b'&' {
                    return self.entity(p, wrap);
                }
                None
            }
            H_UNQ => {
                if is_html_space(b, p) || c == b'>' || (c == b'/' && b.get(p + 1) == Some(&b'>')) {
                    return Some(self.leave(p));
                }
                if matches!(c, b'"' | b'\'' | b'`' | b'<') {
                    return Some(self.token(p, 1, W_UNQ | BAD_VALUE));
                }
                if c == b'&' {
                    return self.entity(p, W_UNQ);
                }
                None
            }
            H_COMMENT => {
                // `(<!-)?(--\s*>)`
                let dashes = |q: usize| {
                    b[q..].starts_with(b"--").then(|| {
                        let s = skip_spaces(b, q + 2);
                        (b.get(s) == Some(&b'>')).then_some(s + 1)
                    })?
                };
                if b[p..].starts_with(b"<!-")
                    && let Some(e) = dashes(p + 3)
                {
                    self.put(p, p + 3, W_HCOMMENT | HCOMMENT_BAD);
                    self.put(p + 3, e, W_HCOMMENT | HCOMMENT_END);
                    self.pop(e);
                    return Some(e);
                }
                if let Some(e) = dashes(p) {
                    return Some(self.close(p, e - p, W_HCOMMENT | HCOMMENT_END));
                }
                // `<!--(?!-?>)|--!>`
                if b[p..].starts_with(b"<!--")
                    && !b[p + 4..].starts_with(b">")
                    && !b[p + 4..].starts_with(b"->")
                {
                    return Some(self.token(p, 4, W_HCOMMENT | HCOMMENT_BAD));
                }
                if b[p..].starts_with(b"--!>") {
                    return Some(self.token(p, 4, W_HCOMMENT | HCOMMENT_BAD));
                }
                None
            }
            H_CDATA => b[p..]
                .starts_with(b"]]>")
                .then(|| self.close(p, 3, W_JD | TAG_CLOSE)),
            DT_NAME => {
                if c.is_ascii_alphabetic() {
                    let e = (p + 1..b.len())
                        .find(|&i| ends_tag_name(b, i))
                        .unwrap_or(b.len());
                    return Some(self.close(p, e - p, W_JD | DOCTYPE_NAME));
                }
                self.any_pop(p)
            }
            DT_TYPE => {
                for w in [&b"PUBLIC"[..], b"SYSTEM"] {
                    if b[p..].starts_with(w) && boundary(b, p + 6) {
                        return Some(self.close(p, 6, W_JD | DOCTYPE_EXTERNAL));
                    }
                }
                self.any_pop(p)
            }
            DT_CONTENT => {
                if c == b'[' {
                    return Some(self.then(p, 1, &[DT_SUBSET], W_JD | HBRACKET_BEGIN));
                }
                self.html_string(p, c).or_else(|| self.any_pop(p))
            }
            DT_META => (c == b'>').then(|| self.close(p, 1, W_JD | TAG_CLOSE)),
            DT_SUBSET => {
                if c == b']' {
                    return Some(self.close(p, 1, W_JD | HBRACKET_END));
                }
                if b[p..].starts_with(b"<!--") {
                    return self.html(p, c);
                }
                None
            }
            _ => None,
        }
    }

    /// `string-double-quoted`, `string-single-quoted` at `p`.
    fn html_string(&mut self, p: usize, c: u8) -> Option<usize> {
        match c {
            b'"' => Some(self.open(p, 1, H_DQ, W_DQ | STR_BEGIN)),
            b'\'' => Some(self.open(p, 1, H_SQ, W_SQ | STR_BEGIN)),
            _ => None,
        }
    }

    /// `tag-attributes` at `p`.
    fn attributes(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if boundary(b, p) && is_word(b, p) {
            let w = &b[p..word_end(b, p)];
            let found = match w {
                b"id" => Some((ATTR_ID, H_EQ)),
                b"class" => Some((ATTR_CLASS, H_EQ)),
                b"style" => Some((ATTR_STYLE, H_EMBED_EQ)),
                _ if w.starts_with(b"on") && EVENTS.contains(&&w[2..]) => {
                    Some((ATTR_EVENT, H_EMBED_EQ))
                }
                _ => None,
            };
            if let Some((leaf, cx)) = found {
                return Some(self.open(p, w.len(), cx, W_JD | leaf));
            }
        }
        self.generic_attribute(p, c)
    }

    /// `tag-generic-attribute`: `(?=[^[\t\n\f =/>]])`.
    fn generic_attribute(&mut self, p: usize, c: u8) -> Option<usize> {
        if is_html_space(self.b, p) || matches!(c, b'=' | b'/' | b'>') {
            return None;
        }
        Some(self.enter(p, &[H_EQ, H_ATTR_NAME]))
    }
}

/// HTML's tag names in its `main`, by the leaf of their name and the context their body is.
static HTML_TAGS: [(&[&[u8]], u16, u8); 8] = [
    (&[b"body", b"head", b"html"], NAME_STRUCTURE, H_TAG),
    (
        &[
            b"address",
            b"applet",
            b"article",
            b"aside",
            b"blockquote",
            b"center",
            b"dd",
            b"dir",
            b"div",
            b"dl",
            b"dt",
            b"figcaption",
            b"figure",
            b"footer",
            b"frame",
            b"frameset",
            b"h1",
            b"h2",
            b"h3",
            b"h4",
            b"h5",
            b"h6",
            b"header",
            b"iframe",
            b"menu",
            b"nav",
            b"noframes",
            b"object",
            b"ol",
            b"p",
            b"pre",
            b"section",
            b"ul",
        ],
        NAME_BLOCK,
        H_TAG,
    ),
    (&[b"hr"], NAME_BLOCK, H_TAG_SC),
    (&[b"form", b"fieldset"], NAME_FORM_BLOCK, H_TAG),
    (
        &[
            b"abbr",
            b"acronym",
            b"area",
            b"audio",
            b"b",
            b"base",
            b"basefont",
            b"bdi",
            b"bdo",
            b"big",
            b"br",
            b"canvas",
            b"caption",
            b"cite",
            b"code",
            b"del",
            b"details",
            b"dfn",
            b"dialog",
            b"em",
            b"font",
            b"head",
            b"html",
            b"i",
            b"img",
            b"ins",
            b"isindex",
            b"kbd",
            b"li",
            b"link",
            b"map",
            b"mark",
            b"menu",
            b"menuitem",
            b"meta",
            b"noscript",
            b"param",
            b"picture",
            b"q",
            b"rp",
            b"rt",
            b"rtc",
            b"ruby",
            b"s",
            b"samp",
            b"script",
            b"small",
            b"source",
            b"span",
            b"strike",
            b"strong",
            b"style",
            b"sub",
            b"summary",
            b"sup",
            b"time",
            b"title",
            b"track",
            b"tt",
            b"u",
            b"var",
            b"video",
            b"wbr",
        ],
        NAME_INLINE,
        H_TAG_SC,
    ),
    (
        &[
            b"button",
            b"datalist",
            b"input",
            b"label",
            b"legend",
            b"meter",
            b"optgroup",
            b"option",
            b"output",
            b"progress",
            b"select",
            b"template",
            b"textarea",
        ],
        NAME_FORM,
        H_TAG_SC,
    ),
    (&[b"a"], NAME_A, H_TAG_SC),
    (
        &[
            b"col",
            b"colgroup",
            b"table",
            b"tbody",
            b"td",
            b"tfoot",
            b"th",
            b"thead",
            b"tr",
        ],
        NAME_TABLE,
        H_TAG_SC,
    ),
];

/// The events of `tag-event-attribute`, after `on`.
static EVENTS: [&[u8]; 64] = [
    b"abort",
    b"autocomplete",
    b"autocompleteerror",
    b"auxclick",
    b"blur",
    b"cancel",
    b"canplay",
    b"canplaythrough",
    b"change",
    b"click",
    b"close",
    b"contextmenu",
    b"cuechange",
    b"dblclick",
    b"drag",
    b"dragend",
    b"dragenter",
    b"dragexit",
    b"dragleave",
    b"dragover",
    b"dragstart",
    b"drop",
    b"durationchange",
    b"emptied",
    b"ended",
    b"error",
    b"focus",
    b"input",
    b"invalid",
    b"keydown",
    b"keypress",
    b"keyup",
    b"load",
    b"loadeddata",
    b"loadedmetadata",
    b"loadstart",
    b"mousedown",
    b"mouseenter",
    b"mouseleave",
    b"mousemove",
    b"mouseout",
    b"mouseover",
    b"mouseup",
    b"mousewheel",
    b"pause",
    b"play",
    b"playing",
    b"progress",
    b"ratechange",
    b"reset",
    b"resize",
    b"scroll",
    b"seeked",
    b"seeking",
    b"select",
    b"show",
    b"sort",
    b"stalled",
    b"submit",
    b"suspend",
    b"timeupdate",
    b"toggle",
    b"volumechange",
    b"waiting",
];

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
}
