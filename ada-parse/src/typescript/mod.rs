//! TypeScript and TSX by a scanner of ada's own (ADR-40; the user's decision of 2026-11-01:
//! not bat's grammar compiled). bat's `TypeScript` and `TypeScriptReact` are Microsoft's
//! TypeScript-TmLanguage converted: 516 patterns, lookaheads copied into each other to three
//! levels of nesting, and contexts pushed and popped on lookaheads alone. This scanner keeps the
//! grammar's contexts where their scopes show (a block, an object literal, a class body, a
//! parameter list, a type) as frames on a stack, and gives each token the leaf scopes bat's
//! grammar gives it; the lookaheads that decide a name's kind (a call, an arrow's parameters, a
//! name assigned a function) are scans of the line ([`look`]). syntect on bat's grammar is the
//! oracle (`parity`), not the design: its quirks aren't reproduced.
//!
//! **Not kept:** the meta scopes (no built-in theme sees one), and four scopes the grammar puts
//! around whole stretches of code, `new.expr`, `cast.expr`, `switch-statement.expr` with its
//! `switch-block.expr` and `case-clause.expr` (no theme sees those either). `string.template`
//! is kept around a template's `${…}`, as themes see it there ([`WRAP`]).
//!
//! The stack between lines is the frames open above the top level, up to 28 (the bytes of
//! [`State`]'s levels).
use crate::prim::char_len;

mod expr;
mod jsx;
mod look;
mod stmt;
mod text;
mod types;
mod words;

use std::cell::Cell;

use crate::stack::HELD as KEPT;
use crate::{Out, State, regex};

macro_rules! leaves {
    ($($name:ident = [$($s:literal),*] $([$($p:literal),*])?,)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[repr(u16)]
        enum Leaf { $($name,)* }
        $(#[allow(dead_code)] const $name: u16 = Leaf::$name as u16;)*
        /// Each kind's scopes under `source.ts`, outermost first.
        static TS: &[&[&str]] = &[$(&[$(concat!($s, ".ts"),)* $($($p,)*)?],)*];
        /// Each kind's scopes under `source.tsx`.
        static TSX: &[&[&str]] = &[$(&[$(concat!($s, ".tsx"),)* $($($p,)*)?],)*];
    };
}

// A kind's scopes: those in the first brackets take the language's suffix (`.ts`, `.tsx`), those
// in the second (JSDoc's, a regex's) are as they are.
leaves! {
    PLAIN = [],
    // comments
    DOC = ["comment.block.documentation"],
    DOC_MARK = ["comment.block.documentation", "punctuation.definition.comment"],
    BLOCK_TEXT = ["comment.block"],
    BLOCK_MARK = ["comment.block", "punctuation.definition.comment"],
    INTERNAL = ["comment.block", "storage.type.internaldeclaration"],
    INTERNAL_MARK = ["comment.block", "storage.type.internaldeclaration", "punctuation.decorator.internaldeclaration"],
    LINE = ["comment.line.double-slash"],
    LINE_MARK = ["comment.line.double-slash", "punctuation.definition.comment"],
    LINE_INTERNAL = ["comment.line.double-slash", "storage.type.internaldeclaration"],
    LINE_INTERNAL_MARK = ["comment.line.double-slash", "storage.type.internaldeclaration", "punctuation.decorator.internaldeclaration"],
    LEADING = ["punctuation.whitespace.comment.leading"],
    SHEBANG = ["comment.line.shebang"],
    SHEBANG_MARK = ["comment.line.shebang", "punctuation.definition.comment"],
    DIRECTIVE = ["comment.line.triple-slash.directive"],
    DIRECTIVE_MARK = ["comment.line.triple-slash.directive", "punctuation.definition.comment"],
    DIRECTIVE_TAG_MARK = ["comment.line.triple-slash.directive", "punctuation.definition.tag.directive"],
    DIRECTIVE_TAG = ["comment.line.triple-slash.directive", "entity.name.tag.directive"],
    DIRECTIVE_ATTR = ["comment.line.triple-slash.directive", "entity.other.attribute-name.directive"],
    DIRECTIVE_EQ = ["comment.line.triple-slash.directive", "keyword.operator.assignment"],
    DIRECTIVE_STR_S = ["comment.line.triple-slash.directive", "string.quoted.single"],
    DIRECTIVE_STR_S_BEGIN = ["comment.line.triple-slash.directive", "string.quoted.single", "punctuation.definition.string.begin"],
    DIRECTIVE_STR_S_END = ["comment.line.triple-slash.directive", "string.quoted.single", "punctuation.definition.string.end"],
    DIRECTIVE_STR_S_ESC = ["comment.line.triple-slash.directive", "string.quoted.single", "constant.character.escape"],
    DIRECTIVE_STR_D_ESC = ["comment.line.triple-slash.directive", "string.quoted.double", "constant.character.escape"],
    DIRECTIVE_STR_D = ["comment.line.triple-slash.directive", "string.quoted.double"],
    DIRECTIVE_STR_D_BEGIN = ["comment.line.triple-slash.directive", "string.quoted.double", "punctuation.definition.string.begin"],
    DIRECTIVE_STR_D_END = ["comment.line.triple-slash.directive", "string.quoted.double", "punctuation.definition.string.end"],
    // JSDoc, in a `/** … */`
    JD_TAG = ["comment.block.documentation"] ["storage.type.class.jsdoc"],
    JD_TAG_AT = ["comment.block.documentation"] ["storage.type.class.jsdoc", "punctuation.definition.block.tag.jsdoc"],
    JD_TAG_AT_INLINE = ["comment.block.documentation"] ["storage.type.class.jsdoc", "punctuation.definition.inline.tag.jsdoc"],
    JD_VAR = ["comment.block.documentation"] ["variable.other.jsdoc"],
    JD_TYPE = ["comment.block.documentation"] ["entity.name.type.instance.jsdoc"],
    JD_TYPE_BEGIN = ["comment.block.documentation"] ["entity.name.type.instance.jsdoc", "punctuation.definition.bracket.curly.begin.jsdoc"],
    JD_TYPE_END = ["comment.block.documentation"] ["entity.name.type.instance.jsdoc", "punctuation.definition.bracket.curly.end.jsdoc"],
    JD_TYPE_BAD = ["comment.block.documentation"] ["invalid.illegal.type.jsdoc"],
    JD_ACCESS = ["comment.block.documentation"] ["constant.language.access-type.jsdoc"],
    JD_SYMBOL = ["comment.block.documentation"] ["constant.language.symbol-type.jsdoc"],
    JD_LINK = ["comment.block.documentation"] ["variable.other.link.underline.jsdoc"],
    JD_EMAIL = ["comment.block.documentation"] ["constant.other.email.link.underline.jsdoc"],
    JD_ANGLE_BEGIN = ["comment.block.documentation"] ["punctuation.definition.bracket.angle.begin.jsdoc"],
    JD_ANGLE_END = ["comment.block.documentation"] ["punctuation.definition.bracket.angle.end.jsdoc"],
    JD_AS = ["comment.block.documentation"] ["keyword.operator.control.jsdoc"],
    JD_OPT_BEGIN = ["comment.block.documentation"] ["variable.other.jsdoc", "punctuation.definition.optional-value.begin.bracket.square.jsdoc"],
    JD_OPT_END = ["comment.block.documentation"] ["variable.other.jsdoc", "punctuation.definition.optional-value.end.bracket.square.jsdoc"],
    JD_OPT_EQ = ["comment.block.documentation"] ["variable.other.jsdoc", "keyword.operator.assignment.jsdoc"],
    JD_OPT_VALUE = ["comment.block.documentation"] ["variable.other.jsdoc", "source.embedded"],
    JD_OPT_BAD = ["comment.block.documentation"] ["variable.other.jsdoc", "invalid.illegal.syntax.jsdoc"],
    JD_STR_BEGIN = ["comment.block.documentation"] ["variable.other.jsdoc", "punctuation.definition.string.begin.jsdoc"],
    JD_STR_END = ["comment.block.documentation"] ["variable.other.jsdoc", "punctuation.definition.string.end.jsdoc"],
    JD_EXAMPLE_CODE = ["comment.block.documentation", "source.embedded"],
    JD_CAPTION_TAG = ["comment.block.documentation"] ["entity.name.tag.inline.jsdoc"],
    JD_CAPTION_ANGLE_BEGIN = ["comment.block.documentation"] ["entity.name.tag.inline.jsdoc", "punctuation.definition.bracket.angle.begin.jsdoc"],
    JD_CAPTION_ANGLE_END = ["comment.block.documentation"] ["entity.name.tag.inline.jsdoc", "punctuation.definition.bracket.angle.end.jsdoc"],
    JD_CAPTION = ["comment.block.documentation"] ["constant.other.description.jsdoc"],
    JD_DESC = ["comment.block.documentation"] ["constant.other.description.jsdoc"],
    JD_DESC_BEGIN = ["comment.block.documentation"] ["constant.other.description.jsdoc", "punctuation.definition.bracket.square.begin.jsdoc"],
    JD_DESC_END = ["comment.block.documentation"] ["constant.other.description.jsdoc", "punctuation.definition.bracket.square.end.jsdoc"],
    JD_INLINE = ["comment.block.documentation"] ["entity.name.type.instance.jsdoc"],
    JD_INLINE_BEGIN = ["comment.block.documentation"] ["entity.name.type.instance.jsdoc", "punctuation.definition.bracket.curly.begin.jsdoc"],
    JD_INLINE_END = ["comment.block.documentation"] ["entity.name.type.instance.jsdoc", "punctuation.definition.bracket.curly.end.jsdoc"],
    JD_INLINE_TAG = ["comment.block.documentation"] ["entity.name.type.instance.jsdoc", "storage.type.class.jsdoc"],
    JD_INLINE_TAG_AT = ["comment.block.documentation"] ["entity.name.type.instance.jsdoc", "storage.type.class.jsdoc", "punctuation.definition.inline.tag.jsdoc"],
    JD_INLINE_LINK = ["comment.block.documentation"] ["entity.name.type.instance.jsdoc", "variable.other.link.underline.jsdoc"],
    JD_INLINE_DESC = ["comment.block.documentation"] ["entity.name.type.instance.jsdoc", "variable.other.description.jsdoc"],
    JD_INLINE_PIPE = ["comment.block.documentation"] ["entity.name.type.instance.jsdoc", "punctuation.separator.pipe.jsdoc"],
    // strings
    STR_S = ["string.quoted.single"],
    STR_S_BEGIN = ["string.quoted.single", "punctuation.definition.string.begin"],
    STR_S_END = ["string.quoted.single", "punctuation.definition.string.end"],
    STR_S_ESC = ["string.quoted.single", "constant.character.escape"],
    STR_S_BAD = ["string.quoted.single", "invalid.illegal.newline"],
    STR_D = ["string.quoted.double"],
    STR_D_BEGIN = ["string.quoted.double", "punctuation.definition.string.begin"],
    STR_D_END = ["string.quoted.double", "punctuation.definition.string.end"],
    STR_D_ESC = ["string.quoted.double", "constant.character.escape"],
    STR_D_BAD = ["string.quoted.double", "invalid.illegal.newline"],
    TPL = ["string.template"],
    TPL_BEGIN = ["string.template", "punctuation.definition.string.template.begin"],
    TPL_END = ["string.template", "punctuation.definition.string.template.end"],
    TPL_ESC = ["string.template", "constant.character.escape"],
    TPL_TAG = ["string.template", "entity.name.function.tagged-template"],
    SUBST_BEGIN = ["string.template", "punctuation.definition.template-expression.begin"],
    SUBST_END = ["string.template", "punctuation.definition.template-expression.end"],
    // regexes
    RE = ["string.regexp"],
    RE_BEGIN = ["string.regexp", "punctuation.definition.string.begin"],
    RE_END = ["string.regexp", "punctuation.definition.string.end"],
    RE_FLAGS = ["string.regexp", "keyword.other"],
    // numbers and constants
    HEX = ["constant.numeric.hex"],
    BINARY = ["constant.numeric.binary"],
    OCTAL = ["constant.numeric.octal"],
    DECIMAL = ["constant.numeric.decimal"],
    HEX_BIG = ["constant.numeric.hex", "storage.type.numeric.bigint"],
    BINARY_BIG = ["constant.numeric.binary", "storage.type.numeric.bigint"],
    OCTAL_BIG = ["constant.numeric.octal", "storage.type.numeric.bigint"],
    DECIMAL_BIG = ["constant.numeric.decimal", "storage.type.numeric.bigint"],
    DECIMAL_PERIOD = ["constant.numeric.decimal", "meta.delimiter.decimal.period"],
    TRUE = ["constant.language.boolean.true"],
    FALSE = ["constant.language.boolean.false"],
    NULL = ["constant.language.null"],
    UNDEFINED = ["constant.language.undefined"],
    NAN = ["constant.language.nan"],
    INFINITY = ["constant.language.infinity"],
    THIS = ["variable.language.this"],
    SUPER = ["variable.language.super"],
    ARGUMENTS = ["variable.language.arguments"],
    IMPORT_ALL = ["constant.language.import-export-all"],
    // keywords
    STORAGE = ["storage.type"],
    MODIFIER = ["storage.modifier"],
    ASYNC = ["storage.modifier.async"],
    FUNCTION_KW = ["storage.type.function"],
    ARROW = ["storage.type.function.arrow"],
    CLASS_KW = ["storage.type.class"],
    INTERFACE_KW = ["storage.type.interface"],
    ENUM_KW = ["storage.type.enum"],
    TYPE_KW = ["storage.type.type"],
    NAMESPACE_KW = ["storage.type.namespace"],
    PROPERTY_KW = ["storage.type.property"],
    EXPORT = ["keyword.control.export"],
    IMPORT = ["keyword.control.import"],
    FROM = ["keyword.control.from"],
    AS = ["keyword.control.as"],
    DEFAULT = ["keyword.control.default"],
    REQUIRE = ["keyword.control.require"],
    TYPE_CONTROL = ["keyword.control.type"],
    INTRINSIC = ["keyword.control.intrinsic"],
    NEW_CONTROL = ["keyword.control.new"],
    TRY = ["keyword.control.trycatch"],
    LOOP = ["keyword.control.loop"],
    FLOW = ["keyword.control.flow"],
    SWITCH = ["keyword.control.switch"],
    CONDITIONAL = ["keyword.control.conditional"],
    WITH = ["keyword.control.with"],
    CONTROL = ["keyword.control"],
    DEBUGGER = ["keyword.other.debugger"],
    ASTERISK = ["keyword.generator.asterisk"],
    NEW = ["keyword.operator.new"],
    DELETE = ["keyword.operator.expression.delete"],
    IN = ["keyword.operator.expression.in"],
    OF = ["keyword.operator.expression.of"],
    INSTANCEOF = ["keyword.operator.expression.instanceof"],
    TYPEOF = ["keyword.operator.expression.typeof"],
    VOID = ["keyword.operator.expression.void"],
    KEYOF = ["keyword.operator.expression.keyof"],
    INFER = ["keyword.operator.expression.infer"],
    IS = ["keyword.operator.expression.is"],
    ASSERTS = ["keyword.operator.type.asserts"],
    IMPORT_OP = ["keyword.operator.expression.import"],
    // operators
    SPREAD = ["keyword.operator.spread"],
    REST = ["keyword.operator.rest"],
    COMPOUND = ["keyword.operator.assignment.compound"],
    COMPOUND_BITWISE = ["keyword.operator.assignment.compound.bitwise"],
    SHIFT = ["keyword.operator.bitwise.shift"],
    COMPARISON = ["keyword.operator.comparison"],
    RELATIONAL = ["keyword.operator.relational"],
    LOGICAL = ["keyword.operator.logical"],
    BITWISE = ["keyword.operator.bitwise"],
    ASSIGN = ["keyword.operator.assignment"],
    DECREMENT = ["keyword.operator.decrement"],
    INCREMENT = ["keyword.operator.increment"],
    ARITHMETIC = ["keyword.operator.arithmetic"],
    TERNARY = ["keyword.operator.ternary"],
    OPTIONAL = ["keyword.operator.optional"],
    DEFINITE = ["keyword.operator.definiteassignment"],
    TYPE_OP = ["keyword.operator.type"],
    TYPE_MODIFIER_OP = ["keyword.operator.type.modifier"],
    ANNOTATION = ["keyword.operator.type.annotation"],
    // punctuation
    BLOCK_BRACE = ["punctuation.definition.block"],
    ROUND = ["meta.brace.round"],
    SQUARE = ["meta.brace.square"],
    ANGLE = ["meta.brace.angle"],
    COMMA = ["punctuation.separator.comma"],
    PARAM_SEP = ["punctuation.separator.parameter"],
    KEY_VALUE = ["punctuation.separator.key-value"],
    LABEL_SEP = ["punctuation.separator.label"],
    CASE_SEP = ["punctuation.definition.section.case-statement"],
    TERMINATOR = ["punctuation.terminator.statement"],
    ACCESSOR = ["punctuation.accessor"],
    ACCESSOR_OPT = ["punctuation.accessor.optional"],
    PARAMS_BEGIN = ["punctuation.definition.parameters.begin"],
    PARAMS_END = ["punctuation.definition.parameters.end"],
    TP_BEGIN = ["punctuation.definition.typeparameters.begin"],
    TP_END = ["punctuation.definition.typeparameters.end"],
    BIND_OBJ = ["punctuation.definition.binding-pattern.object"],
    BIND_ARR = ["punctuation.definition.binding-pattern.array"],
    DESTRUCTURING = ["punctuation.destructuring"],
    DECORATOR = ["punctuation.decorator"],
    // names
    ENTITY_FN = ["entity.name.function"],
    THIS_FN = ["entity.name.function", "variable.language.this"],
    CONST_FN = ["variable.other.constant", "entity.name.function"],
    LABEL = ["entity.name.label"],
    TYPE = ["entity.name.type"],
    TYPE_CLASS = ["entity.name.type.class"],
    TYPE_INTERFACE = ["entity.name.type.interface"],
    TYPE_ENUM = ["entity.name.type.enum"],
    TYPE_ALIAS = ["entity.name.type.alias"],
    TYPE_MODULE = ["entity.name.type.module"],
    INHERITED = ["entity.other.inherited-class"],
    PARAM = ["variable.parameter"],
    THIS_PARAM = ["variable.parameter", "variable.language.this"],
    READWRITE = ["variable.other.readwrite"],
    ALIAS = ["variable.other.readwrite.alias"],
    CONSTANT = ["variable.other.constant"],
    CONSTANT_PROP = ["variable.other.constant.property"],
    CONSTANT_OBJ = ["variable.other.constant.object"],
    CONSTANT_OBJ_PROP = ["variable.other.constant.object.property"],
    OBJECT = ["variable.other.object"],
    OBJECT_PROP = ["variable.other.object.property"],
    PROPERTY = ["variable.other.property"],
    OBJ_PROPERTY = ["variable.object.property"],
    ENUM_MEMBER = ["variable.other.enummember"],
    // support
    SUPPORT_CLASS = ["support.class"],
    BUILTIN_CLASS = ["support.class.builtin"],
    ERROR_CLASS = ["support.class.error"],
    PROMISE = ["support.class.promise"],
    SUPPORT_FN = ["support.function"],
    MATH = ["support.constant.math"],
    MATH_FN = ["support.function.math"],
    MATH_CONST = ["support.constant.property.math"],
    CONSOLE = ["support.class.console"],
    CONSOLE_FN = ["support.function.console"],
    JSON = ["support.constant.json"],
    JSON_FN = ["support.function.json"],
    IMPORT_META = ["support.variable.property.importmeta"],
    NEW_TARGET = ["support.variable.property.target"],
    SUPPORT_PROP = ["support.variable.property"],
    SUPPORT_CONST = ["support.constant"],
    DOM_VAR = ["support.variable.dom"],
    DOM_CLASS = ["support.class.dom"],
    DOM_CONST = ["support.constant.dom"],
    DOM_PROP = ["support.variable.property.dom"],
    NODE_CLASS = ["support.class.node"],
    PROCESS = ["support.variable.object.process"],
    PROCESS_PROP = ["support.variable.property.process"],
    PROCESS_FN = ["support.function.process"],
    MODULE = ["support.type.object.module"],
    NODE_OBJ = ["support.variable.object.node"],
    EVENT_HANDLER = ["support.function.event-handler"],
    SUPPORT_FN_DOM = ["support.function.dom"],
    PROMISE_FN = ["support.function.promise"],
    PRIMITIVE = ["support.type.primitive"],
    BUILTIN_TYPE = ["support.type.builtin"],
    // TSX's JSX
    JSX_TAG_BEGIN = ["punctuation.definition.tag.begin"],
    JSX_TAG_END = ["punctuation.definition.tag.end"],
    JSX_TAG_NS = ["entity.name.tag.namespace"],
    JSX_NS_SEP = ["punctuation.separator.namespace"],
    JSX_TAG_NAME = ["entity.name.tag"],
    JSX_COMPONENT = ["entity.name.tag", "support.class.component"],
    JSX_ATTR_NS = ["entity.other.attribute-name.namespace"],
    JSX_ATTR = ["entity.other.attribute-name"],
    JSX_ILLEGAL = ["invalid.illegal.attribute"],
    JSX_EMBED_BEGIN = ["punctuation.section.embedded.begin"],
    JSX_EMBED_END = ["punctuation.section.embedded.end"],
    JSX_ENTITY = ["constant.character.entity"],
    JSX_ENTITY_MARK = ["constant.character.entity", "punctuation.definition.entity"],
    JSX_BAD_AMP = ["invalid.illegal.bad-ampersand"],
    JSX_STR_D_ENTITY = ["string.quoted.double", "constant.character.entity"],
    JSX_STR_D_ENTITY_MARK = ["string.quoted.double", "constant.character.entity", "punctuation.definition.entity"],
    JSX_STR_D_BAD_AMP = ["string.quoted.double", "invalid.illegal.bad-ampersand"],
    JSX_STR_S_ENTITY = ["string.quoted.single", "constant.character.entity"],
    JSX_STR_S_ENTITY_MARK = ["string.quoted.single", "constant.character.entity", "punctuation.definition.entity"],
    JSX_STR_S_BAD_AMP = ["string.quoted.single", "invalid.illegal.bad-ampersand"],
}

/// A kind's bit for a token inside a template's `${…}`: under `string.template` too.
pub(crate) const WRAP: u16 = 0x8000;

/// The kinds of a regex's tokens (ADR-40's phase 4, [`crate::regex`]'s, ECMAScript's
/// dialect): from here, [`regex::KINDS`] for each host, under the host's scope: a literal's
/// (`string.regexp`), then `RegExp('…')`'s, `RegExp("…")`'s and ``RegExp(`…`)``'s.
const RE_KINDS: u16 = 512;
const _: () = assert!(TS.len() <= RE_KINDS as usize);
/// The leaf each host's regex kinds are under, by [`RE_KINDS`]' order.
const RE_HOSTS: [u16; 4] = [RE, STR_S, STR_D, TPL];

/// The scopes under `source.ts` (`source.tsx` for TSX) a kind carries, outermost first.
pub fn scopes(kind: u16, tsx: bool) -> Vec<&'static str> {
    let table = if tsx { TSX } else { TS };
    let k = kind & !WRAP;
    let (leaf, re) = if k >= RE_KINDS {
        let n = regex::KINDS as u16;
        let (host, re) = ((k - RE_KINDS) / n, (k - RE_KINDS) % n);
        let Some(&host) = RE_HOSTS.get(usize::from(host)) else {
            return Vec::new();
        };
        (table[usize::from(host)], Some(re))
    } else {
        let Some(leaf) = table.get(usize::from(k)) else {
            return Vec::new();
        };
        (*leaf, None)
    };
    let mut v = Vec::with_capacity(leaf.len() + 1);
    if kind & WRAP != 0 {
        v.push(table[usize::from(TPL)][0]);
    }
    for &s in leaf.iter() {
        if v.last() != Some(&s) {
            v.push(s);
        }
    }
    if let Some(re) = re {
        v.extend(regex::scopes(re));
    }
    v
}

// The frames, the grammar's contexts that can be open. The top level is `MAIN`.
/// `main`: `statements`.
const MAIN: u8 = 0;
/// `decl-block`'s `{ … }`: `statements`.
const BLOCK: u8 = 1;
/// `paren-expression`'s `( … )`: `expression`.
const PAREN: u8 = 2;
/// `array-literal`'s `[ … ]`.
const ARRAY: u8 = 3;
/// `object-literal`'s `{ … }`: `object-member`.
const OBJ: u8 = 4;
/// An object member's value, after its `:`, up to `,` or `}`.
const OBJ_VAL: u8 = 5;
/// An object member's `...`, up to `,` or `}`.
const OBJ_SPREAD: u8 = 6;
/// An object member `name = …`, up to `,`, `}` or the line's end.
const OBJ_ASSIGN: u8 = 7;
/// A computed key's `[ … ]`, up to its `:`.
const OBJ_KEY_SQ: u8 = 8;
/// A quoted key, up to its `:`.
const OBJ_KEY_STR: u8 = 9;
/// A numeric key, up to its `:`.
const OBJ_KEY_NUM: u8 = 10;
/// An object's method, up to `}`, `;` or `,` (`object-literal-method-declaration`).
const OBJ_METHOD: u8 = 11;
/// A method's name, up to its `(` or `<` (`method-declaration-name`).
const METHOD_NAME: u8 = 12;
/// A ternary's `? …`, up to its `:`.
const TERNARY_F: u8 = 13;
/// After `new`, its expression (`new-expr`).
const NEW_F: u8 = 14;
/// After `return`, up to `;`, `}` or the line's end.
const RETURN: u8 = 15;
/// `var-expr` of `var` or `let`.
const VAR: u8 = 16;
/// `var-expr` of `const`.
const VAR_CONST: u8 = 17;
/// After `,` at a `var` line's end.
const VAR_MORE: u8 = 18;
/// After `,` at a `const` line's end.
const VAR_CONST_MORE: u8 = 19;
/// After a declared name, up to `=`, `,`, `;` or the line's end (`var-single-variable`'s).
const VAR_NAME: u8 = 20;
/// `variable-initializer`, its value on the line.
const INIT: u8 = 21;
/// `variable-initializer` at the line's end.
const INIT_EOL: u8 = 22;
/// `destructuring-variable`'s first, a `{ … }` declared.
const DESTRUCT: u8 = 23;
/// `destructuring-const`'s first.
const DESTRUCT_CONST: u8 = 24;
/// `object-binding-pattern`'s `{ … }`.
const BIND_OBJ_F: u8 = 25;
/// `object-binding-pattern-const`'s.
const BIND_OBJ_CONST: u8 = 26;
/// `array-binding-pattern`'s `[ … ]`.
const BIND_ARR_F: u8 = 27;
/// `array-binding-pattern-const`'s.
const BIND_ARR_CONST: u8 = 28;
/// An object binding element's `key: …`, up to `,` or `}`.
const BIND_ELEM: u8 = 29;
/// The const one's.
const BIND_ELEM_CONST: u8 = 30;
/// A binding element's key, up to its `:` (`object-binding-element-propertyName`).
const BIND_KEY: u8 = 31;
/// A parameter's `{ … }` (`parameter-object-binding-pattern`, `destructuring-parameter`).
const PBIND_OBJ: u8 = 32;
/// A parameter's `[ … ]`.
const PBIND_ARR: u8 = 33;
/// A parameter binding element's `key: …`.
const PBIND_ELEM: u8 = 34;
/// A template's `${ … }`.
const SUBST: u8 = 35;
/// A decorator, `@` up to a blank.
const DECORATOR_F: u8 = 36;
/// After `as`, a type.
const AS_F: u8 = 37;
/// After `typeof`, an expression.
const TYPEOF_F: u8 = 38;
/// After `instanceof`, a type.
const INSTANCEOF_F: u8 = 39;
/// A cast's `< … >`.
const CAST: u8 = 40;
/// A call's target, its names up to `(` (`function-call`'s inner push).
const CALL_TARGET: u8 = 41;
/// A call, after its target: type arguments and arguments (`function-call`).
const CALL: u8 = 42;
/// An arrow's head before its `=>` (`arrow-function`'s second).
const ARROW_HEAD: u8 = 43;
/// After an arrow's `=>`, up to its body.
const ARROW_F: u8 = 44;
/// `paren-expression-possibly-arrow`, up to after its `)`.
const MAYBE_ARROW: u8 = 45;
/// `expression-inside-possibly-arrow-parens`' `( … )`.
const ARROW_PARENS: u8 = 46;
/// `arrow-return-type`, up to `=>` or `{`.
const ARROW_RET: u8 = 47;
/// `possibly-arrow-return-type`, up to `=>` or `{`.
const MAYBE_ARROW_RET: u8 = 48;
/// After `(?<=[:])(?=\s*\{)` in an arrow's return type: a type object.
const RET_OBJ: u8 = 49;
/// `function-declaration`, `meta.function`.
const FUNC: u8 = 50;
/// `function-expression`.
const FUNC_EXPR: u8 = 51;
/// `method-declaration`.
const METHOD: u8 = 52;
/// `function-parameters`' `( … )`.
const PARAMS: u8 = 53;
/// `parameter-type-annotation`, up to `,`, `)` or `=`.
const PARAM_TYPE: u8 = 54;
/// `return-type`, with something after its `:` on the line.
const RETURN_TYPE: u8 = 55;
/// `return-type`, at the line's end.
const RETURN_TYPE_EOL: u8 = 56;
/// `class-declaration`, `class-expression`.
const CLASS: u8 = 57;
/// `interface-declaration`.
const INTERFACE: u8 = 58;
/// `class-or-interface-body`'s `{ … }`.
const CLASS_BODY: u8 = 59;
/// After `extends` or `implements`, up to `{`.
const HERITAGE: u8 = 60;
/// In a class body after `:`, an expression (`(?<=:)\s*`).
const CLASS_COLON: u8 = 61;
/// `field-declaration`.
const FIELD: u8 = 62;
/// `indexer-declaration`'s `[ … ]`.
const INDEXER: u8 = 63;
/// `indexer-mapped-type-declaration`.
const MAPPED: u8 = 64;
/// `enum-declaration`.
const ENUM: u8 = 65;
/// The enum's `{ … }`.
const ENUM_BODY: u8 = 66;
/// An enum member, up to `,`, `}` or the line's end.
const ENUM_MEMBER_F: u8 = 67;
/// `namespace-declaration`.
const NAMESPACE: u8 = 68;
/// `type-alias-declaration`.
const TYPE_DECL: u8 = 69;
/// After a type alias's `=`.
const TYPE_DECL_VALUE: u8 = 70;
/// `import-declaration`.
const IMPORT_F: u8 = 71;
/// After `import`, up to `from`.
const IMPORT_CLAUSE: u8 = 72;
/// `import-export-block`'s `{ … }`.
const IMPORT_BLOCK: u8 = 73;
/// `export-declaration`'s `meta.export`.
const EXPORT_F: u8 = 74;
/// `export default` and `export =`.
const EXPORT_DEFAULT: u8 = 75;
/// `import x = require(…)`.
const IMPORT_REQUIRE: u8 = 76;
/// `import x = a.b`.
const IMPORT_ALIAS: u8 = 77;
/// After `for`, up to its `)`.
const FOR: u8 = 78;
/// A `for`'s `( … )`.
const FOR_PARENS: u8 = 79;
/// `if-statement`, an `if (…)` without a `{` after it, up to `;`, `}` or the line's end.
const IF: u8 = 80;
/// The `if`'s `( … )`.
const IF_COND: u8 = 81;
/// `switch-statement`, up to its `}`.
const SWITCH_F: u8 = 82;
/// The switch's `( … )`.
const SWITCH_EXPR: u8 = 83;
/// The switch's `{ … }`.
const SWITCH_BLOCK: u8 = 84;
/// After `case` or `default`, up to `:`.
const CASE: u8 = 85;
/// A case's `: {` … `}`.
const CASE_BLOCK: u8 = 86;
/// A label's `{ … }` (`label`'s first).
const LABEL_BLOCK: u8 = 87;
/// `yield` before a comment and `*`.
const YIELD_STAR: u8 = 88;
/// `type-annotation`, something after its `:` on the line.
const TYPE_ANNOT: u8 = 89;
/// `type-annotation` at the line's end.
const TYPE_ANNOT_EOL: u8 = 90;
/// `type-arguments`' `< … >`.
const TYPE_ARGS: u8 = 91;
/// `type-parameters`' `< … >`.
const TYPE_PARAMS: u8 = 92;
/// `type-name`'s `name< … >`.
const TYPE_NAME_ARGS: u8 = 93;
/// `type-tuple`'s `[ … ]`.
const TUPLE: u8 = 94;
/// `type-object`'s `{ … }`.
const TYPE_OBJ: u8 = 95;
/// `type-paren-or-function-parameters`' `( … )`.
const TYPE_PAREN: u8 = 96;
/// `type-fn-type-parameters`' third, a function type's parameters.
const TYPE_FN: u8 = 97;
/// `new (…)` in a type, up to after its `)`.
const TYPE_CTOR: u8 = 98;
/// `new <…>` in a type.
const TYPE_CTOR_TP: u8 = 99;
/// A function type's `=> …`, something after it on the line.
const TYPE_FN_RET: u8 = 100;
/// A function type's `=>` at the line's end.
const TYPE_FN_RET_EOL: u8 = 101;
/// After `=>` before `{`, a type object up to after its `}`.
const TYPE_RET_OBJ: u8 = 102;
/// `type-conditional`, `extends` up to after its `:`.
const TYPE_COND: u8 = 103;
/// The conditional's `? …`, up to its `:`.
const TYPE_COND_Q: u8 = 104;
/// `&` or `|` before `{`, a type object up to after its `}`.
const TYPE_OP_OBJ: u8 = 105;
/// Blanks up to the next character, lines included: after a type's `&` or `|`, and after `var`'s,
/// `let`'s or `const`'s `\s*` that took the line's end.
const GAP: u8 = 106;
/// A type object's `...`.
const TYPE_SPREAD: u8 = 107;
/// `template-type`'s template.
const TPL_TYPE: u8 = 108;
/// `template-type-substitution-element`'s `${ … }`.
const SUBST_TYPE: u8 = 109;
/// After a type alias's `= intrinsic`.
const TYPE_DECL_INTRINSIC: u8 = 110;
/// A `'` string.
const STR_S_F: u8 = 111;
/// A `"` string.
const STR_D_F: u8 = 112;
/// A template's text.
const TPL_F: u8 = 113;
/// A `/* … */` comment.
const BLOCK_F: u8 = 114;
/// A `/** … */` comment.
const DOC_F: u8 = 115;
/// A regex literal's body, its state in [`Scan::re`] ([`regex::ECMA_STATE`]: its groups open
/// and whether a class is).
const REGEX_F: u8 = 116;
/// A group the regex below has open, kept as a frame where something sits above the regex or
/// the line ends ([`Scan::spill`]).
const RE_GROUP_F: u8 = 117;
/// A regex string, above the string's or template's frame: `RegExp('…')`, its state in
/// [`Scan::re`] as a literal's.
const RE_STR_F: u8 = 118;
/// A class the regex below has open, kept as [`RE_GROUP_F`].
const RE_SET_F: u8 = 119;
/// JSDoc's `@example`, up to `@` or `*/`.
const DOC_EXAMPLE: u8 = 120;
/// `@example`'s `<caption> … </caption>`.
const DOC_CAPTION: u8 = 121;
/// A JSDoc type, `{ … }` (`jsdoctype`).
const DOC_TYPE: u8 = 122;
/// `brackets` inside a JSDoc type.
const DOC_BRACKETS: u8 = 123;
/// `brackets`' `[ … ]`.
const DOC_BRACKETS_SQ: u8 = 124;
/// After `@typedef`, its type and name.
const DOC_TYPEDEF: u8 = 125;
/// After `@param` and the like before `{`, the type and name.
const DOC_PARAM: u8 = 126;
/// After `@define` and the like, the type.
const DOC_TYPED: u8 = 127;
/// `@default '…'`'s string.
const DOC_STRING_S: u8 = 128;
/// `@default "…"`'s string.
const DOC_STRING_D: u8 = 129;
/// An inline tag, `{@link … }`.
const DOC_INLINE: u8 = 130;
/// A `/// <reference … />` directive.
const DIRECTIVE_F: u8 = 131;
/// The directive's tag.
const DIRECTIVE_TAG_F: u8 = 132;
/// The directive tag's `'` string.
const DIRECTIVE_STR_S_F: u8 = 133;
/// The directive tag's `"` string.
const DIRECTIVE_STR_D_F: u8 = 134;
/// A line comment that takes the line's end too (`single-line-comment-consuming-line-ending`).
const LINE_EOL: u8 = 135;
/// A template call's tag and type arguments before its `` ` `` (`template-call`).
const TPL_CALL: u8 = 136;
/// An `if`'s regex after its `)`.
const IF_REGEX: u8 = 137;
/// The value of an object member that is an arrow's parameters.
const OBJ_ARROW_PARENS: u8 = 139;
/// `object-member`'s `(?<=:)\s*(async)?(?=\<\s*$)` type parameters.
const OBJ_ARROW_TP: u8 = 140;
/// After `]`/a quote before `(` or `<`: a method (`object-member`'s fourth).
const OBJ_KEY_METHOD: u8 = 141;
/// A member of an enum with a quoted or computed name.
const ENUM_MEMBER_KEY: u8 = 142;
/// `meta.object.member`'s `as` before a type.
const OBJ_AS: u8 = 143;
/// A template call's tag, its names up to the type arguments or `` ` `` (`template-call`'s inner push).
const TPL_CALL_TARGET: u8 = 144;
/// A call's arguments, `( … )`: the call (`CALL`) again after its `)` if another follows.
const CALL_ARGS: u8 = 145;
/// `function-call`'s second: a target and `<` before `{`, `[` or `(` at the line's end.
const CALL_EOL: u8 = 146;
/// Its target.
const CALL_EOL_TARGET: u8 = 147;
/// `destructuring-variable`'s second, before `[` (`DESTRUCT` is its first, before `{`).
const DESTRUCT_ARR: u8 = 148;
/// `destructuring-const`'s second.
const DESTRUCT_ARR_CONST: u8 = 149;
// TSX's JSX ([`jsx`]).
/// `jsx-tag-without-attributes-in-expression`: elements without attributes, one after another.
const JSX_WA_EXPR: u8 = 150;
/// `jsx-tag-in-expression`: elements, one after another.
const JSX_EXPR: u8 = 151;
/// An element without attributes, `<name>`, up to its closing tag: its children.
const JSX_WA: u8 = 152;
/// An element (`jsx-tag`), from its `<` to `/>` or its closing tag.
const JSX_TAG_F: u8 = 153;
/// After the tag's name: type arguments and the attributes, up to `>` or `/>`.
const JSX_ATTRS: u8 = 154;
/// The attributes, after their first blank.
const JSX_ATTRS2: u8 = 155;
/// The element's children, after its `>`.
const JSX_CHILDREN: u8 = 156;
/// `{ … }` among the children or as an attribute's value.
const JSX_EMBED: u8 = 157;
/// An attribute's `"` string.
const JSX_STR_D: u8 = 158;
/// An attribute's `'` string.
const JSX_STR_S: u8 = 159;

/// How many frames a line can hold; one pushed past it drops the bottom one.
const MAX: usize = 128;
/// Matches that take nothing, in a row at one place, before the scan steps on regardless.
const STUCK: usize = 48;

/// Lex `b`, a line of TypeScript (TSX if `tsx`) without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], tsx: bool, state: &mut State, out: &mut Out) {
    let mut s = Scan {
        b,
        i: 0,
        stack: [MAIN; MAX],
        depth: 1,
        tsx,
        exact: state.is_exact(),
        text: PLAIN,
        wrap: 0,
        tpl: 0,
        tick: b.iter().rposition(|&c| c == b'`').map_or(0, |k| k + 1),
        last_name: Cell::new((usize::MAX, 0)),
        done: 0,
        zero: (usize::MAX, 0),
        stuck: 0,
        g_link: false,
        g_desc: false,
        g_caption: false,
        g_type_open: false,
        g_type_brace: false,
        var_kw_eol: false,
        re: 0,
        out,
    };
    s.depth = state.load(&mut s.stack);
    if state.arg == PACKED {
        s.depth = 1 + unpack(&mut s.stack, s.depth - 1);
    }
    s.tpl = s.stack[1..s.depth].iter().filter(|&&f| is_tpl(f)).count();
    s.refresh();
    s.run();
    if s.re != 0 && matches!(s.top(), REGEX_F | RE_STR_F) {
        s.spill();
    }
    // deeper than the state keeps: the pairs packed, so that the top `KEPT` hold them all
    let (mut depth, mut arg) = (s.depth, 0);
    if depth - 1 > KEPT {
        depth = 1 + pack(&mut s.stack, depth - 1);
        arg = PACKED;
    }
    *state = State::keep(&s.stack, depth, s.exact, arg);
}

/// [`State`]'s `arg` when its levels are packed ([`pack`]).
const PACKED: u16 = 1;
/// The pairs of frames, the lower first, that pack into one byte between lines when a line ends
/// deeper than the state keeps: nested object literals, arrays, calls, callbacks' blocks and JSX
/// elements, which deep data, tests and components are made of. The 96 commonest in a row in the
/// corpora's stacks over 20 deep, sorted: of the 460 distinct in `.ts` (up to 41 deep) and the 2,557
/// in `.tsx` (up to 58), every one but one fits in 28 bytes once packed.
#[rustfmt::skip]
static PAIRS: [(u8, u8); 96] = [
    (BLOCK, BLOCK), (BLOCK, RETURN), (BLOCK, VAR), (BLOCK, VAR_CONST), (BLOCK, CLASS_BODY), (BLOCK, CALL_ARGS),
    (PAREN, BLOCK), (PAREN, OBJ), (PAREN, TERNARY_F), (PAREN, CALL_ARGS), (PAREN, JSX_WA_EXPR), (PAREN, JSX_EXPR),
    (ARRAY, BLOCK), (ARRAY, PAREN), (ARRAY, OBJ), (ARRAY, CALL_ARGS), (OBJ, BLOCK), (OBJ, OBJ_VAL),
    (OBJ, OBJ_SPREAD), (OBJ, OBJ_METHOD), (OBJ_VAL, BLOCK), (OBJ_VAL, PAREN), (OBJ_VAL, ARRAY), (OBJ_VAL, OBJ),
    (OBJ_VAL, TERNARY_F), (OBJ_VAL, TPL_F), (OBJ_VAL, OBJ_ARROW_PARENS), (OBJ_VAL, CALL_ARGS), (OBJ_SPREAD, PAREN),
    (OBJ_SPREAD, CALL_ARGS), (OBJ_METHOD, PARAMS), (TERNARY_F, PAREN), (TERNARY_F, OBJ), (TERNARY_F, CALL_ARGS),
    (NEW_F, CLASS_BODY), (NEW_F, CALL_ARGS), (RETURN, OBJ), (RETURN, NEW_F), (RETURN, MAYBE_ARROW),
    (RETURN, CALL_ARGS), (VAR, INIT), (VAR_CONST, INIT), (INIT, BLOCK), (INIT, PAREN), (INIT, ARRAY), (INIT, OBJ),
    (INIT, NEW_F), (INIT, ARROW_F), (INIT, MAYBE_ARROW), (INIT, CALL_ARGS), (PBIND_OBJ, INIT),
    (PBIND_OBJ, PBIND_OBJ), (ARROW_F, MAYBE_ARROW), (MAYBE_ARROW, ARROW_PARENS), (ARROW_PARENS, OBJ),
    (ARROW_PARENS, PBIND_OBJ), (ARROW_PARENS, JSX_WA_EXPR), (ARROW_PARENS, JSX_EXPR), (PARAMS, PBIND_OBJ),
    (CLASS_BODY, BLOCK), (FIELD, INIT), (EXPORT_DEFAULT, BLOCK), (TYPE_NAME_ARGS, TYPE_PAREN),
    (TYPE_PAREN, TYPE_COND), (TYPE_COND, TYPE_NAME_ARGS), (TPL_F, SUBST), (OBJ_ARROW_PARENS, JSX_EXPR),
    (CALL_ARGS, BLOCK), (CALL_ARGS, PAREN), (CALL_ARGS, ARRAY), (CALL_ARGS, OBJ), (CALL_ARGS, NEW_F),
    (CALL_ARGS, ARROW_F), (CALL_ARGS, CLASS_BODY), (CALL_ARGS, CALL_ARGS), (JSX_WA_EXPR, JSX_WA),
    (JSX_EXPR, JSX_TAG_F), (JSX_WA, JSX_WA), (JSX_WA, JSX_TAG_F), (JSX_WA, JSX_EMBED), (JSX_TAG_F, JSX_ATTRS),
    (JSX_TAG_F, JSX_CHILDREN), (JSX_ATTRS, JSX_ATTRS2), (JSX_ATTRS2, JSX_EMBED), (JSX_CHILDREN, JSX_WA),
    (JSX_CHILDREN, JSX_TAG_F), (JSX_CHILDREN, JSX_EMBED), (JSX_EMBED, BLOCK), (JSX_EMBED, PAREN),
    (JSX_EMBED, ARRAY), (JSX_EMBED, OBJ), (JSX_EMBED, TERNARY_F), (JSX_EMBED, ARROW_F), (JSX_EMBED, CALL_ARGS),
    (JSX_EMBED, JSX_WA_EXPR), (JSX_EMBED, JSX_EXPR),
];
/// The first pair's byte: `PAIR + k` is `PAIRS[k]`.
const PAIR: u8 = 160;
const _: () = assert!(PAIR as usize + PAIRS.len() <= 256 && JSX_STR_S < PAIR);

/// Pack `stack[1..=n]` in place, bottom up, each two frames in a row that [`PAIRS`] has as one
/// byte; the packed length.
#[cold]
fn pack(stack: &mut [u8; MAX], n: usize) -> usize {
    let (mut i, mut o) = (1, 1);
    while i <= n {
        let pair = if i < n {
            PAIRS.binary_search(&(stack[i], stack[i + 1])).ok()
        } else {
            None
        };
        if let Some(k) = pair {
            stack[o] = PAIR + k as u8;
            i += 2;
        } else {
            stack[o] = stack[i];
            i += 1;
        }
        o += 1;
    }
    o - 1
}

/// Undo [`pack`] on `stack[1..=n]`; the frames' count.
#[cold]
fn unpack(stack: &mut [u8; MAX], n: usize) -> usize {
    let mut packed = [0; KEPT];
    packed[..n].copy_from_slice(&stack[1..=n]);
    let mut o = 1;
    for &f in &packed[..n] {
        if f >= PAIR {
            (stack[o], stack[o + 1]) = PAIRS[usize::from(f - PAIR)];
            o += 2;
        } else {
            stack[o] = f;
            o += 1;
        }
    }
    o - 1
}

struct Scan<'a, 'o, 'p> {
    b: &'a [u8],
    /// The place the scan has reached; `n` is the line's `\n`.
    i: usize,
    stack: [u8; MAX],
    depth: usize,
    tsx: bool,
    exact: bool,
    /// The kind of the bytes no token covers, by the frames open.
    text: u16,
    /// [`WRAP`] inside a template.
    wrap: u16,
    /// The template frames open above the top level ([`is_tpl`]).
    tpl: usize,
    /// Just past the line's last `` ` ``, 0 if it has none: no template starts at or after it.
    tick: usize,
    /// The last name read ([`Scan::name`]): where it starts and ends. Pattern after pattern
    /// reads the name at the place.
    last_name: Cell<(usize, usize)>,
    /// Where the spans pushed so far end.
    done: usize,
    /// Where and at what depth the last push that took nothing happened: a pop that takes
    /// nothing there is passed over (syntect's guard against a loop).
    zero: (usize, usize),
    stuck: usize,
    /// An inline tag's `\G` link pattern failed on this line ([`Scan::inline_g`]).
    g_link: bool,
    /// Its description pattern failed on this line.
    g_desc: bool,
    /// `@example`'s `\G(<)caption(>)` failed on this line (as [`Scan::inline_g`]'s).
    g_caption: bool,
    /// `jsdoctype`'s `\G{(?:[^}*]|\*[^/}])+$` failed on this line ([`Scan::jsdoctype`]).
    g_type_open: bool,
    /// Its `\G({)` failed on this line.
    g_type_brace: bool,
    /// A `var`, `let` or `const` keyword took the line's end (its `\s*` the `\n`).
    var_kw_eol: bool,
    /// The state of the regex in the top frame ([`REGEX_F`], [`RE_STR_F`]).
    re: u8,
    out: &'o mut Out<'p>,
}

impl Scan<'_, '_, '_> {
    fn run(&mut self) {
        // where the last turn failed at a space or tab in code, `usize::MAX` if the last one didn't
        let mut failed = usize::MAX;
        while self.i <= self.b.len() {
            let i0 = self.i;
            let top = self.stack[self.depth - 1];
            let q = quiet(top);
            if q != 0 {
                // the turns that can't match in these frames ([`quiet`]): at the line's end, and
                // at blanks before anything but the bytes their blank-led patterns look for
                if self.i == self.b.len() {
                    if q & Q_END != 0 {
                        break;
                    }
                } else if is_space(self.b[self.i])
                    && (q & Q_BLANK != 0 || (q & Q_BLANK_IN != 0 && self.i > 0))
                {
                    let k = self.ws(self.i);
                    let d = self.c(k);
                    if !(matches!(d, b'<' | b'/' | b'(' | b'[' | b'{' | b':' | b'?' | b'!')
                        || (d == b'a' && self.has(k, b"async")))
                    {
                        self.i = k.min(self.b.len());
                        self.stuck = 0;
                        self.zero.0 = usize::MAX;
                        failed = usize::MAX;
                        continue;
                    }
                }
            }
            if self.step(top) {
                failed = usize::MAX;
            } else if self.i < self.b.len()
                && matches!(self.b[self.i], b' ' | b'\t')
                && family(top) == Family::Code
            {
                // A turn that fails changes nothing, so when two in a row have failed at blanks,
                // every blank after them sees what the second saw: a blank behind, and the same
                // text ahead past blanks (no pattern counts the blanks before it). The run is
                // passed.
                let i = self.i;
                self.i = if failed == i.wrapping_sub(1) {
                    self.sp(i)
                } else {
                    i + 1
                };
                failed = i;
            } else {
                failed = usize::MAX;
                self.i += if self.i < self.b.len() {
                    char_len(self.b[self.i])
                } else {
                    1
                };
            }
            if self.i == i0 {
                self.stuck += 1;
                if self.stuck > STUCK {
                    self.i += if self.i < self.b.len() {
                        char_len(self.b[self.i])
                    } else {
                        1
                    };
                    self.stuck = 0;
                }
            } else {
                self.stuck = 0;
                self.zero.0 = usize::MAX;
            }
        }
        self.fill(self.b.len());
    }

    /// Byte `i`, the line's `\n` at its end, 0 past it.
    #[inline]
    fn c(&self, i: usize) -> u8 {
        // one bounds check: `n` is `b`'s length
        match self.b.get(i) {
            Some(&c) => c,
            None if i == self.b.len() => b'\n',
            None => 0,
        }
    }

    /// The bytes up to `to` that no token has covered, as the frames open make them.
    fn fill(&mut self, to: usize) {
        self.out
            .fill(&mut self.done, to.min(self.b.len()), self.text | self.wrap);
    }

    /// Bytes `s..e` are `kind`: an empty token fills nothing, and none starts before where the
    /// spans end. Its own token half, not [`Out::put`]: `put`'s second fill tests `e` against
    /// where the spans end, which TypeScript paid +0.50% instructions for (X-017 (d)).
    fn tok(&mut self, s: usize, e: usize, kind: u16) {
        let e = e.min(self.b.len());
        if e <= s {
            return;
        }
        self.fill(s);
        let k = kind | self.wrap;
        if k != PLAIN {
            self.out.push_merged(s, e, k);
        }
        self.done = e;
    }

    /// A token `s..e` of `kind`, and the scan moved past it.
    fn take(&mut self, s: usize, e: usize, kind: u16) -> bool {
        self.tok(s, e, kind);
        self.i = e;
        true
    }

    fn top(&self) -> u8 {
        self.stack[self.depth - 1]
    }

    fn push(&mut self, f: u8) {
        self.fill(self.i);
        if self.depth == MAX {
            self.tpl -= usize::from(is_tpl(self.stack[1]));
            crate::stack::drop_bottom(&mut self.stack, &mut self.depth, &mut self.exact);
        }
        self.stack[self.depth] = f;
        self.depth += 1;
        self.tpl += usize::from(is_tpl(f));
        self.refresh();
    }

    /// Push `f` at the place without taking anything.
    fn push_zero(&mut self, f: u8) -> bool {
        self.push(f);
        self.zero = (self.i, self.depth);
        true
    }

    fn pop(&mut self) -> bool {
        self.fill(self.i);
        if self.depth > 1 {
            self.depth -= 1;
            self.tpl -= usize::from(is_tpl(self.stack[self.depth]));
        }
        self.refresh();
        true
    }

    /// Pop without taking anything, unless the last push was here and took nothing.
    fn pop_zero(&mut self) -> bool {
        if self.zero == (self.i, self.depth) {
            return false;
        }
        self.pop()
    }

    /// Replace the top frame.
    fn set(&mut self, f: u8) {
        self.fill(self.i);
        if self.depth > 1 {
            self.tpl -= usize::from(is_tpl(self.stack[self.depth - 1]));
            self.tpl += usize::from(is_tpl(f));
        }
        self.stack[self.depth - 1] = f;
        self.refresh();
    }

    /// The text kind (the top frame's) and the template bit (any template open: what is
    /// inside its `${…}` is under its scope too).
    fn refresh(&mut self) {
        self.text = text_of(self.top()).unwrap_or(PLAIN);
        self.wrap = if self.tpl > 0 { WRAP } else { 0 };
    }

    /// The line from `i`, empty past its end.
    #[inline]
    fn rest(&self, i: usize) -> &[u8] {
        &self.b[i.min(self.b.len())..]
    }

    /// Whether the line has `w` at `i`: `rest(i).starts_with(w)`, compared as
    /// [`crate::same`] does, without a call to `memcmp`.
    #[inline(always)]
    fn has(&self, i: usize, w: &[u8]) -> bool {
        crate::has_at(self.b, i.min(self.b.len()), w)
    }

    /// Past blanks (`\s`, the line's `\n` too) from `i`.
    fn ws(&self, mut i: usize) -> usize {
        while is_space(self.c(i)) {
            i += 1;
        }
        i
    }

    /// Past spaces and tabs only.
    fn sp(&self, mut i: usize) -> usize {
        while i < self.b.len() && matches!(self.b[i], b' ' | b'\t') {
            i += 1;
        }
        i
    }

    /// `^\s*$` holds for the whole line.
    fn blank_line(&self) -> bool {
        self.b.iter().all(|&c| is_space(c))
    }

    /// The end of a name `[_$[:alpha:]][_$[:alnum:]]*` at `i`, `i` if none starts there.
    fn name(&self, i: usize) -> usize {
        let (s, e) = self.last_name.get();
        if s == i {
            return e;
        }
        let e = look::name(self.b, i);
        self.last_name.set((i, e));
        e
    }

    /// Whether a name character `[_$[:alnum:]]` ends at `i` (is just before it).
    fn alnum_before(&self, i: usize) -> bool {
        look::alnum_before(self.b, i)
    }

    /// Whether a name character starts at `i`.
    fn alnum_at(&self, i: usize) -> bool {
        look::alnum_at(self.b, i)
    }

    /// `«wb»`: `(?<![_$[:alnum:]])(?:(?<=\.\.\.)|(?<!\.))` at `i`.
    fn wb(&self, i: usize) -> bool {
        // `i` may be past the line's `\n` (after `ws`): `c` reads it there
        !self.alnum_before(i)
            && (i == 0 || self.c(i - 1) != b'.' || (i >= 3 && &self.b[i - 3..i] == b"..."))
    }

    /// `«we»`: `(?![_$[:alnum:]])(?:(?=\.\.\.)|(?!\.))` at `i`.
    fn we(&self, i: usize) -> bool {
        !self.alnum_at(i) && (self.c(i) != b'.' || self.has(i, b"..."))
    }

    /// The word `w` at `i` between `«wb»` and `«we»`: its end.
    fn kw(&self, i: usize, w: &[u8]) -> Option<usize> {
        let e = i + w.len();
        (self.has(i, w) && self.wb(i) && self.we(e)).then_some(e)
    }

    /// `\b` at `i`, by ASCII and Unicode's word characters.
    fn boundary(&self, i: usize) -> bool {
        look::is_word_before(self.b, i) != look::is_word_at(self.b, i)
    }

    /// The word `w` at `i` with `\b` after it (but not necessarily before).
    fn word_b(&self, i: usize, w: &[u8]) -> Option<usize> {
        let e = i + w.len();
        (self.has(i, w) && self.boundary(e)).then_some(e)
    }

    /// The name at `i` is `w`, whole.
    fn is_name(&self, i: usize, e: usize, w: &[u8]) -> bool {
        crate::same(&self.b[i..e], w)
    }

    /// One frame's turn at the place: whether it matched something. Inlined into the scan's loop
    /// with [`family`] (they stopped being on their own when phase 4 grew the text turns: +9% of
    /// TSX's instructions).
    #[inline(always)]
    fn step(&mut self, f: u8) -> bool {
        match family(f) {
            Family::Text => self.text_step(f),
            Family::Type => self.type_step(f),
            Family::Jsx => self.jsx_step(f),
            Family::Code => self.code_step(f),
        }
    }
}

/// Which turn a frame takes.
#[derive(Clone, Copy, PartialEq)]
enum Family {
    /// Comments, strings, templates, regexes, JSDoc ([`Scan::text_step`]).
    Text,
    /// Types ([`Scan::type_step`]).
    Type,
    /// TSX's JSX ([`Scan::jsx_step`]).
    Jsx,
    /// Statements and expressions ([`Scan::code_step`]).
    Code,
}

/// Frame `f`'s family.
#[inline(always)]
fn family(f: u8) -> Family {
    match f {
        STR_S_F | STR_D_F | TPL_F | BLOCK_F | DOC_F | REGEX_F | RE_GROUP_F | RE_STR_F
        | RE_SET_F | DOC_EXAMPLE | DOC_CAPTION | DOC_TYPE | DOC_BRACKETS | DOC_BRACKETS_SQ
        | DOC_TYPEDEF | DOC_PARAM | DOC_TYPED | DOC_STRING_S | DOC_STRING_D | DOC_INLINE
        | DIRECTIVE_F | DIRECTIVE_TAG_F | DIRECTIVE_STR_S_F | DIRECTIVE_STR_D_F | LINE_EOL
        | IF_REGEX | TPL_TYPE => Family::Text,
        TYPE_ANNOT | TYPE_ANNOT_EOL | TYPE_ARGS | TYPE_PARAMS | TYPE_NAME_ARGS | TUPLE
        | TYPE_OBJ | TYPE_PAREN | TYPE_FN | TYPE_CTOR | TYPE_CTOR_TP | TYPE_FN_RET
        | TYPE_FN_RET_EOL | TYPE_RET_OBJ | TYPE_COND | TYPE_COND_Q | TYPE_OP_OBJ | GAP
        | TYPE_SPREAD | SUBST_TYPE | TYPE_DECL_INTRINSIC | AS_F | INSTANCEOF_F | CAST
        | PARAM_TYPE | RETURN_TYPE | RETURN_TYPE_EOL | ARROW_RET | MAYBE_ARROW_RET | RET_OBJ
        | INDEXER | MAPPED | TYPE_DECL | TYPE_DECL_VALUE | OBJ_AS => Family::Type,
        JSX_WA_EXPR..=JSX_STR_S => Family::Jsx,
        _ => Family::Code,
    }
}

/// Frames of code whose turn can't match at the line's end, nor at a blank unless the next
/// non-blank is `<`, `/`, `(`, `[`, `{`, `:`, `?`, `!` or `async`. Their turns are `statements` or
/// `expression` and a closing bracket or two, each pattern of which needs a particular byte at the
/// place, `\s*` then one of those bytes (`\s*<` a cast, type arguments or JSX, `\s*/` a regex, a
/// division or a comment, `\s*\(` and `\s*<` an arrow's head or a call after `)`, `\s*\[` an
/// array, `\s*\{` an object after an operator, `\s*:` a return type or a ternary's end, `\?\.`
/// and `!` a call after `)`, `\s*async` an arrow's head), or a name, a digit, a quote or an
/// operator at the place; past the line's `\n` every byte reads 0.
///
/// Which of the two hold: [`Q_END`] and [`Q_BLANK`]; [`Q_BLANK_IN`] the blanks' past the line's
/// start, for frames that pop at `i == 0` (`RETURN`'s blank line and declaration, `INIT`'s `^`).
/// `OBJ`'s turn is `object_member`, whose keys start at a blank too, but not at the line's end.
fn quiet(f: u8) -> u8 {
    match f {
        MAIN | BLOCK | CASE_BLOCK | LABEL_BLOCK | PAREN | IF_COND | SWITCH_EXPR | ARRAY
        | OBJ_VAL | OBJ_SPREAD | CALL_ARGS | TERNARY_F => Q_END | Q_BLANK,
        OBJ => Q_END,
        RETURN | INIT => Q_BLANK_IN,
        _ => 0,
    }
}

/// [`quiet`]: no turn matches at the line's end.
const Q_END: u8 = 1;
/// No turn matches at a blank before other bytes than those listed.
const Q_BLANK: u8 = 2;
/// The same past the line's start.
const Q_BLANK_IN: u8 = 4;

/// A template's frame: what is open inside it is under `string.template` too.
fn is_tpl(f: u8) -> bool {
    matches!(f, TPL_F | TPL_TYPE | TPL_CALL)
}

/// The kind of the bytes no token covers in frame `f`, if it has its own.
fn text_of(f: u8) -> Option<u16> {
    Some(match f {
        STR_S_F | JSX_STR_S => STR_S,
        STR_D_F | JSX_STR_D => STR_D,
        TPL_F | TPL_TYPE => TPL,
        BLOCK_F => BLOCK_TEXT,
        DOC_F | DOC_EXAMPLE | DOC_TYPEDEF | DOC_PARAM | DOC_TYPED => DOC,
        DOC_CAPTION => JD_CAPTION,
        DOC_TYPE | DOC_BRACKETS | DOC_BRACKETS_SQ => JD_TYPE,
        DOC_STRING_S | DOC_STRING_D => JD_VAR,
        DOC_INLINE => JD_INLINE,
        REGEX_F => RE,
        DIRECTIVE_F | DIRECTIVE_TAG_F => DIRECTIVE,
        DIRECTIVE_STR_S_F => DIRECTIVE_STR_S,
        DIRECTIVE_STR_D_F => DIRECTIVE_STR_D,
        LINE_EOL => LINE,
        _ => return None,
    })
}

/// Whether name `w` holds a `$`: a byte loop, as names are short (`<[u8]>::contains` calls
/// memchr, which took 4% of the scan's time on the census, 2026-11-01).
#[allow(clippy::manual_contains)]
fn has_dollar(w: &[u8]) -> bool {
    w.iter().any(|&c| c == b'$')
}

/// An ASCII byte that starts a name, `[_$[:alpha:]]`.
fn is_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b'$'
}

/// `\s`: a blank, the line's `\n` too.
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Lang, lex};

    /// [`pack`] looks pairs up by a binary search, and each pair is two frames, not a pair's byte.
    #[test]
    fn the_pairs_are_sorted_frames() {
        assert!(PAIRS.windows(2).all(|w| w[0] < w[1]));
        assert!(PAIRS.iter().all(|&(a, b)| a < PAIR && b < PAIR));
    }

    /// A `...` at a line's end, where a parameter's name is looked for past it: the blanks after
    /// it take the line's `\n`, and the lookbehind there read past the line (a panic until
    /// 2026-11-01, found by the whole-corpus digests over the scrambled sets).
    #[test]
    fn a_rest_at_the_line_end_reads_no_further() {
        for lang in [Lang::TypeScript, Lang::TypeScriptReact] {
            for text in [
                "function f(...",
                "type F = (...",
                "type F = (a, ...",
                "let x: (...",
                "class A { f(...",
                "type T = { f(...",
            ] {
                let (mut state, mut spans) = (State::default(), Vec::new());
                lex(lang, text, &mut state, &mut spans);
                lex(lang, "...", &mut state, &mut spans);
            }
        }
    }

    /// Objects and JSX nested past the 28 frames a state keeps unpacked: every line's state
    /// exact, the same scopes at each depth on the way out as on the way in, and the top level
    /// again at the end.
    #[test]
    fn deep_nesting_stays_exact() {
        for (lang, open, close) in [
            (Lang::TypeScript, "key: {", "},"),
            (Lang::TypeScriptReact, "<div a={1}>", "</div>"),
        ] {
            let (mut state, mut spans) = (State::default(), Vec::new());
            let start = if lang == Lang::TypeScriptReact {
                "x = ("
            } else {
                "const schema = {"
            };
            lex(lang, start, &mut state, &mut spans);
            let mut ins = Vec::new();
            for _ in 0..24 {
                lex(lang, open, &mut state, &mut spans);
                assert!(state.is_exact(), "{lang:?}");
                ins.push(
                    spans
                        .iter()
                        .map(|s| lang.scopes(s.kind))
                        .collect::<Vec<_>>(),
                );
            }
            for _ in 0..24 {
                lex(lang, close, &mut state, &mut spans);
                assert!(state.is_exact(), "{lang:?}");
            }
            assert!(ins.windows(2).all(|w| w[0] == w[1]), "{lang:?}");
            lex(
                lang,
                if lang == Lang::TypeScript { "};" } else { ");" },
                &mut state,
                &mut spans,
            );
            assert_eq!(state, State::default(), "{lang:?}");
        }
    }

    fn has(m: &[(&str, &str)], text: &str, scopes: &str) {
        assert!(m.contains(&(text, scopes)), "{text:?} {scopes:?} in {m:?}");
    }

    /// A literal's body is the regex scanner's, in ECMAScript's dialect (phase 4), as
    /// JavaScript's: its fixes over bat's grammar (a `.` in a class, `\u{…}`, the `v` flag) too.
    #[test]
    fn regex_literals_are_ecmascripts() {
        let m = crate::tests::marked(
            Lang::TypeScript,
            &[r"x = /^a[/.]b\/(?<n>c)\k<n>{2,}\u{1F600}/gv;", "y = /(a/"],
        );
        has(&m, "^", "string.regexp.ts keyword.control.anchor.regexp");
        has(
            &m,
            "/.",
            "string.regexp.ts constant.other.character-class.set.regexp",
        );
        has(&m, "n", "string.regexp.ts entity.name.other.group.regexp");
        has(
            &m,
            r"\u{1F600}",
            "string.regexp.ts constant.character.escape.backslash.regexp",
        );
        has(&m, "gv", "string.regexp.ts keyword.other.ts");
        // a group open at the line's end: the literal goes on, its `/` the regex's
        let (mut state, mut spans) = (State::default(), Vec::new());
        lex(Lang::TypeScript, "y = /(a/", &mut state, &mut spans);
        lex(Lang::TypeScript, "b)/; z = 1", &mut state, &mut spans);
        let k = spans.iter().find(|s| s.start == 1).unwrap().kind;
        assert!(
            Lang::TypeScript
                .scopes(k)
                .contains(&"punctuation.definition.group.end.regexp")
        );
        assert_eq!(state, State::default());
    }

    /// `RegExp(…)`'s first argument, a string or a template, is a regex (as WebStorm colors it):
    /// the string's escapes, end and `${…}` come first, and a group open over a `${…}` or a
    /// line's end is kept.
    #[test]
    fn regexes_in_regexp_strings() {
        const D: &str = "string.quoted.double.ts";
        const T: &str = "string.template.ts";
        let m = crate::tests::marked(
            Lang::TypeScript,
            &[
                r#"r = new RegExp("^(\\d+)\"x$", 'g'), s = RegExp('a\'(');"#,
                r"t = new RegExp(`^(${p}|x)[0-9]\\${q}`);",
                "u = foo(\"(a)\"), v = RegExp(`(a",
                "b)c`);",
            ],
        );
        has(&m, "^", &format!("{D} keyword.control.anchor.regexp"));
        has(
            &m,
            r"\\d",
            &format!("{D} constant.other.character-class.escape.backslash.regexp"),
        );
        has(&m, "\\\"", &format!("{D} constant.character.escape.ts"));
        has(
            &m,
            "\"",
            &format!("{D} punctuation.definition.string.end.ts"),
        );
        has(
            &m,
            "(",
            "string.quoted.single.ts punctuation.definition.group.begin.regexp",
        );
        // the flags are a plain string
        has(&m, "g", "string.quoted.single.ts");
        has(
            &m,
            "${",
            &format!("{T} punctuation.definition.template-expression.begin.ts"),
        );
        has(&m, "|", &format!("{T} keyword.operator.or.regexp"));
        // `\\` before a `${`: the regex's `\`, escaping what the substitution starts with
        has(
            &m,
            r"\\",
            &format!("{T} constant.character.escape.backslash.regexp"),
        );
        // not a regex
        has(&m, "(a)", D);
        // a group across the line's end
        has(
            &m,
            ")",
            &format!("{T} punctuation.definition.group.end.regexp"),
        );
        let (mut state, mut spans) = (State::default(), Vec::new());
        for line in ["v = RegExp(`(a", "b)c`);"] {
            lex(Lang::TypeScript, line, &mut state, &mut spans);
        }
        assert_eq!(state, State::default());
    }
}
