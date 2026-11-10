//! TypeScript and TSX by a scanner of ada's own designed for throughput (X-091, 2026-11-08: "a
//! similar redesign and throughput-first mentality", as JavaScript's X-058 and Rust's X-090),
//! from the color census of bat's grammar across the 14 themes: bat is the reference for the
//! colors, not the design. One loop runs every context as a frame of a small stack that means
//! something to the colors: statement code, an object literal (its key place), parentheses,
//! brackets, a template's substitution, a function's parameters, a class's (or an interface's)
//! head and body, a declaration, an enum, an import's braces, a ternary's `?`, the type frames
//! ([`types`]) and the text frames ([`text`]: strings, templates, comments with JSDoc, regex
//! literals, JSX).
//!
//! - **The previous significant token in a byte** ([`Scan::prev`]) tells a `/` a regex's or
//!   division, a `<` a cast's, JSX's or an operator, a `{` an object's or a block's, a name a
//!   member, a declared name, a type's name, a parameter, a constructor's type.
//! - **No lookahead past the blanks after a token** but at the rare tokens a group's or a
//!   name's meaning hangs on: a `(` that may open an arrow's parameters keeps a mark of the
//!   writing ([`Group`]), and a `=>` (or `): type =>`) after its `)` takes the writing back and
//!   reads it again as parameters; a `name =` (or a key's `name:`) keeps a mark ([`Assign`]) so
//!   a value that turns out a function names it; a `<` right after a name is a generic call's
//!   only where its type arguments close before a `(` ([`types`]).
//! - **A name is read once**, its bytes' classes OR-ed as it goes (Rust's X-090), and looked up
//!   once, in one of two tables of keywords and support words by where it stands ([`words`]).
//!
//! **Not kept:** the meta scopes (no built-in theme sees one); `string.template` is kept around
//! a template's `${…}`, as themes see it there ([`WRAP`]). The stack between lines is the
//! frames open above the top level, up to 28, the previous token beside them.

mod lists;
mod text;
mod types;
mod words;

use std::mem::MaybeUninit;

use self::words::{
    DOT, DOT_WORDS, INFO, Info, NO_LEAF, W_CONTEXT, W_KW, W_KW2, W_OBJ, W_SUPPORT_FN, W_VALUE,
    WORDS,
};
use crate::emit::{Mark, Rules, Run, Runs, Sink, Spans};
use crate::prim::{char_len, is_word};
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

/// How many frames a line can hold; one pushed past it drops the bottom one.
const MAX: usize = 64;
const _: () = assert!(MAX.is_power_of_two());

// The frames. Code frames first; [`F_TYPE`] and up a type frame's ([`Scan::type_turn`]),
// [`TEXT`] and up a text frame's ([`Scan::text_turn`]).
/// The top level: statements.
const F_TOP: u8 = 0;
/// A block's `{`: statements.
const F_BLOCK: u8 = 1;
/// A `(`: a group or a call's arguments.
const F_PAREN: u8 = 2;
/// A `[`.
const F_BRACKET: u8 = 3;
/// A template's `${`, back to its text at its `}`.
const F_SUBST: u8 = 4;
/// A JSX expression's `{`, back to the element at its `}`.
const F_J_EXPR: u8 = 5;
/// A ternary's `?`, popped at its `:`.
const F_TERNARY: u8 = 6;
/// A `case`'s (or `default`'s) expression, popped at its `:`.
const F_CASE: u8 = 7;
/// An object literal's `{`: a key's place after it and after each `,`. The first of the frames
/// that place names ([`Scan::framed_name`]).
const F_OBJECT: u8 = 8;
const F_PLACES: u8 = F_OBJECT;
/// An `import`'s or `export`'s `{ … }`.
const F_IMPORT: u8 = 11;
/// An enum's body.
const F_ENUM: u8 = 12;
/// A declaration's destructuring `{` and `[`.
const F_BIND: u8 = 13;
const F_BIND_ARR: u8 = 14;
/// A computed key's `[` at a key's or a member's place: its `]` leaves [`P_KEYED`].
const F_KEY_BRACKET: u8 = 15;
/// A class field's value, popped at its `;` (or a line that ends after a value).
const F_FIELD: u8 = 16;
/// A parameter's default value: code, popped at a `,` or a closer of the frame below.
const F_DEFAULT: u8 = 17;
/// A function's parameters: a name is a parameter.
const F_PARAMS: u8 = 18;
/// A destructuring pattern's `{` or `[` among the parameters.
const F_PATTERN: u8 = 19;
/// A class's (or an interface's) body: its members.
const F_CLASS_BODY: u8 = 20;
/// After `function`: its `*`, its name, its `<T>`, its `(`.
const F_FUNC_HEAD: u8 = 21;
/// After `class`: its name, `<T>`, `extends`, `implements` and what they name, its `{`.
const F_CLASS_HEAD: u8 = 22;
/// After `interface`, as [`F_CLASS_HEAD`].
const F_IFACE_HEAD: u8 = 23;
/// An index signature's `[` in a class or an interface body: a name before `:` a parameter.
const F_INDEX: u8 = 24;
/// A `(` that ended its line where bat reads possibly an arrow's parameters: a name before
/// `:`, `?`, `,`, `=` or the line's end a parameter, else an expression's; its `(` and `)`
/// plain.
const F_PARAMS_PLAIN: u8 = 25;
/// An arrow's parameters (its `=>` seen, or its type parameters before): as [`F_PARAMS`], a
/// `:` after its `)` the arrow's return type.
const F_PARAMS_ARROW: u8 = 26;
/// The first of the type frames ([`Scan::type_turn`]): a type annotation after its `:`, left
/// at a byte no type takes at its level.
const F_TYPE: u8 = 32;
/// A type after `as` or `satisfies`: an annotation that a `:` or a `?` also leaves.
const F_TYPE_AS: u8 = 33;
/// An arrow's return type: an annotation its `=>` leaves.
const F_TYPE_RET: u8 = 34;
/// `type X = …`: to its `;`, a closer, or a line that ends after a type and goes on with none.
const F_TYPE_DECL: u8 = 35;
/// A type's `(` (a group), `[`, `{` (members) and `<` (arguments, parameters), each left at
/// its closer.
const F_T_ROUND: u8 = 36;
const F_T_SQUARE: u8 = 37;
const F_T_CURLY: u8 = 38;
const F_T_ANGLE: u8 = 39;
/// A cast's `<…>`: its `<` and `>` plain.
const F_T_CAST: u8 = 40;
/// Type parameters' `<…>` (a function's, a class's, a method's, a generic arrow's): its `>`
/// leaves [`P_KEYED`], so a `(` after it opens parameters.
const F_T_TPARAMS: u8 = 41;
/// A generic arrow's type parameters: its `>` leaves [`P_ARROW_TP`], so its `(` opens an
/// arrow's parameters.
const F_T_ARROW_TP: u8 = 42;
/// A template literal type's `${`: a type, back to the template's text at its `}`.
const F_T_SUBST: u8 = 43;
/// A generic arrow's type parameters whose `<` ended its line: its `>(` opens the arrow's
/// parameters with plain brackets, as bat's possibly-arrow `(` (`F_PARAMS_PLAIN`).
const F_T_ARROW_TP_LINE: u8 = 44;
/// A type's `(` that ended its line (bat's possibly a function type's parameters): its
/// brackets plain, a name before `:` or `?:` a parameter, else a type's group.
const F_T_ROUND_LINE: u8 = 45;
const _: () = assert!(F_T_ROUND_LINE < TEXT);
/// The first text frame.
const TEXT: u8 = 64;
/// A `"…"` continued by a `\` at a line's end.
const T_STR_D: u8 = 64;
/// A `'…'`, likewise.
const T_STR_S: u8 = 65;
/// A template's text.
const T_TPL: u8 = 66;
/// A block comment's text.
const T_BLOCK: u8 = 67;
/// A doc comment's (`/**`) text.
const T_DOC: u8 = 68;
/// A JSX element's opening tag: its attributes, until `>` or `/>`.
const T_JSX_TAG: u8 = 69;
/// A JSX element's children, until a closing tag.
const T_JSX_CHILD: u8 = 70;
/// A JSX attribute's `"…"` and `'…'`.
const T_JSX_STR_D: u8 = 71;
const T_JSX_STR_S: u8 = 72;
/// A JSDoc inline tag (`{@link …`) open past its line, above its doc comment's frame.
const T_DOC_INLINE: u8 = 73;
/// A regex in a `RegExp(…)` string or template, its state in the low bits, above the string's
/// frame.
const T_RE_STR: u8 = 0x80;
const T_RE_STR_LAST: u8 = T_RE_STR | regex::ECMA_STATE;
/// A regex literal a group or a class kept open past its line, its state in the low bits.
const T_RE_LIT: u8 = 0xa0;
const T_RE_LIT_LAST: u8 = T_RE_LIT | regex::ECMA_STATE;
const _: () = assert!(T_RE_STR & regex::ECMA_STATE == 0 && T_RE_STR_LAST < T_RE_LIT);
const _: () = assert!(T_RE_LIT & regex::ECMA_STATE == 0);

// The previous significant token, as the next one reads it.
/// A statement's start: a `{` a block's, a `/` a regex's, a name before `:` a label.
const P_START: u8 = 0;
/// An expression is expected: a `{` an object's, a `/` a regex's, a `<` a cast's or JSX's.
const P_OP: u8 = 1;
/// After a value: a `/` division, a `<` an operator, a `{` a block's.
const P_VALUE: u8 = 2;
/// After a `.`: a name is a member.
const P_DOT: u8 = 3;
/// An object's key place, after its `{` or a `,`.
const P_KEY: u8 = 4;
/// After `=>`: a `{` a block's, else an expression.
const P_ARROW: u8 = 5;
/// After `new`: an expression, its name a call's (bat's `new.expr` holds a call).
const P_NEW: u8 = 6;
/// After `extends` or `implements` in a class's or an interface's head: a name is the one
/// inherited.
const P_EXTENDS: u8 = 7;
/// A class body's member place: after its `{`, a `;`, a method's `}`, a modifier.
const P_MEMBER: u8 = 8;
/// After `get` or `set` at a key's or a member's place: the name is an accessor's.
const P_ACCESSOR: u8 = 9;
/// After a key that isn't a name (a string, a number, a computed `[…]`), or a method's type
/// parameters, at a key's or a member's place: a `(` opens a method's parameters.
const P_KEYED: u8 = 10;
/// After a function's parameters' `)`: a value's, and a `:` starts the return type.
const P_PARAMS_END: u8 = 11;
/// In a declaration's destructuring, after its `{`, `[`, `,` or a key's `:`: a name is
/// declared, a `{` or a `[` a nested pattern.
const P_BIND: u8 = 12;
/// After a declared name: a `:` its type's, a `!` definite.
const P_DECLARED: u8 = 13;
/// After `import` or `export`: a `{` its names', a `*` all, `type` a keyword.
const P_IMPORT: u8 = 14;
/// After `enum`: the name is the enum's, then its `{`. The first of the tokens that place the
/// name after them ([`Scan::placed_name`]).
const P_ENUM_NAME: u8 = 15;
const P_PLACED: u8 = P_ENUM_NAME;
/// After `namespace` or `module`: the names are a module's, then its `{`.
const P_MODULE_NAME: u8 = 16;
/// After `instanceof`: a name is a type's.
const P_TYPE_AFTER: u8 = 17;
/// After `break` or `continue`: a name is a label.
const P_JUMP: u8 = 18;
/// After `catch`: a `(` opens its parameter.
const P_CATCH: u8 = 19;
/// After `@`: a decorator's name.
const P_DECORATOR: u8 = 20;
/// After `Math.`, `console.`, `JSON.`, `process.`, `module.`: a member of its list is a support
/// word's.
const P_DOT_MATH: u8 = 21;
const P_DOT_CONSOLE: u8 = 22;
const P_DOT_JSON: u8 = 23;
const P_DOT_PROCESS: u8 = 24;
const P_DOT_MODULE: u8 = 25;
/// After an enum's name: its `{`.
const P_ENUM_BODY: u8 = 26;
/// After an arrow's parameters' `)`: a `:` starts its return type, which its `=>` leaves.
const P_ARROW_PARAMS: u8 = 27;
/// After a type's name was read in a type frame: a type's value, but a `.` keeps a path.
const P_TYPEOF: u8 = 28;
/// After `infer` in a type: the name is the type's.
const P_INFER: u8 = 29;
/// After an index signature's `[` at a member's place in an object type.
const P_INDEX: u8 = 30;
/// After `type` at a statement's start: the name is the alias's.
const P_ALIAS_NAME: u8 = 31;
/// After `asserts` in a type: the name is a parameter's.
const P_ASSERTS: u8 = 32;
/// After a heritage path's name before its `.` (`implements ts.Host`).
const P_EXTENDS_PATH: u8 = 33;
/// After a generic arrow's type parameters: its `(`.
const P_ARROW_TP: u8 = 34;
/// After `let` or `var`, and after `const`: a name is declared (no frame: bat's colors need
/// none past the name), a `{` or a `[` a destructuring.
const P_DECL: u8 = 35;
const P_DECL_CONST: u8 = 36;

/// [`State`]'s `arg`: the previous token.
const ARG_PREV: u16 = 0xff;
/// How many template substitutions are open (up to 15), from bit [`ARG_SUBSTS_SHIFT`].
const ARG_SUBSTS: u16 = 0x1e00;
const ARG_SUBSTS_SHIFT: u16 = 9;
/// The most open substitutions counted (the most [`ARG_SUBSTS`] holds).
const SUBSTS_MAX: u8 = 15;
/// A `const`'s destructuring is open (its names constants on the lines it spans).
const ARG_BIND_CONST: u16 = 0x100;

/// How many groups a line keeps a mark for at once ([`Group`]).
const GROUPS: usize = 8;

/// A `(` that may open an arrow's parameters: where the writing stood before it, and its depth.
#[derive(Clone, Copy)]
struct Group {
    mark: Mark,
    open: u32,
    depth: u16,
    /// The assignment whose value the group starts (`name = (…) =>`), else one whose
    /// value is `NO_VALUE`.
    assign: Assign,
    /// Where an `async` before the group starts, else `NO_VALUE`: a keyword if the group is an
    /// arrow's parameters, else a variable as written.
    asy: u32,
}

/// A `name =`, a key's `name:` or a field's `name =`, whose value starts at `value`: if that
/// value is a function, the name is the function's.
#[derive(Clone, Copy)]
struct Assign {
    mark: Mark,
    name: (u32, u32),
    /// The `=` or `:`.
    sep: u32,
    value: u32,
    /// [`A_NAMED`], [`A_CONST`], [`A_KEY`], [`A_QUOTED`]: what the name is, and how a function
    /// names it.
    kind: u8,
}

/// An assigned name's, a `const`'s declared name's, a key's, a quoted key's [`Assign::kind`].
const A_NAMED: u8 = 0;
const A_CONST: u8 = 1;
const A_KEY: u8 = 2;
const A_QUOTED: u8 = 3;

const NO_VALUE: u32 = u32::MAX;

const NO_ASSIGN: Assign = Assign {
    mark: Mark::NONE,
    name: (0, 0),
    sep: 0,
    value: NO_VALUE,
    kind: A_NAMED,
};

// Word ids the scan compares against.
const ID_ASYNC: u16 = words::id_of(b"async");
const ID_FROM: u16 = words::id_of(b"from");
const ID_AS: u16 = words::id_of(b"as");
const ID_SATISFIES: u16 = words::id_of(b"satisfies");
const ID_OF: u16 = words::id_of(b"of");
const ID_IN: u16 = words::id_of(b"in");
const ID_DEFAULT: u16 = words::id_of(b"default");
const ID_CASE: u16 = words::id_of(b"case");
const ID_TYPE: u16 = words::id_of(b"type");
const ID_INTERFACE: u16 = words::id_of(b"interface");
const ID_NAMESPACE: u16 = words::id_of(b"namespace");
const ID_MODULE: u16 = words::id_of(b"module");
const ID_DECLARE: u16 = words::id_of(b"declare");
const ID_ABSTRACT: u16 = words::id_of(b"abstract");
const ID_GLOBAL: u16 = words::id_of(b"global");
const ID_CONST: u16 = words::id_of(b"const");
const ID_FUNCTION: u16 = words::id_of(b"function");
const ID_CLASS: u16 = words::id_of(b"class");
const ID_IMPORT: u16 = words::id_of(b"import");
const ID_NEW: u16 = words::id_of(b"new");
const ID_YIELD: u16 = words::id_of(b"yield");
const ID_EXTENDS: u16 = words::id_of(b"extends");
const ID_IMPLEMENTS: u16 = words::id_of(b"implements");
const ID_GET: u16 = words::id_of(b"get");
const ID_SET: u16 = words::id_of(b"set");
const ID_CONSTRUCTOR: u16 = words::id_of(b"constructor");
const ID_REQUIRE: u16 = words::id_of(b"require");
const ID_REGEXP: u16 = words::id_of(b"RegExp");
const ID_KEYOF: u16 = words::id_of(b"keyof");
const ID_TYPEOF: u16 = words::id_of(b"typeof");
const ID_READONLY: u16 = words::id_of(b"readonly");
const ID_THIS: u16 = words::id_of(b"this");

// A byte's class: what the loop does at it.
const A_OTHER: u8 = 0;
const A_BLANK: u8 = 1;
const A_NAME: u8 = 2;
const A_DIGIT: u8 = 3;
const A_QUOTE: u8 = 4;
const A_SLASH: u8 = 5;
const A_PAREN: u8 = 6;
const A_CLOSE_PAREN: u8 = 7;
const A_BRACKET: u8 = 8;
const A_CLOSE_BRACKET: u8 = 9;
const A_CURLY: u8 = 10;
const A_CLOSE_CURLY: u8 = 11;
const A_DOT: u8 = 12;
const A_SEMI: u8 = 13;
const A_COMMA: u8 = 14;
const A_COLON: u8 = 15;
const A_QUESTION: u8 = 16;
const A_EQ: u8 = 17;
const A_OP: u8 = 18;
const A_LT: u8 = 19;
const A_TICK: u8 = 20;
const A_SIGN: u8 = 21;
const A_HIGH: u8 = 22;
const A_AT: u8 = 23;
const A_STAR: u8 = 24;
const A_HASH: u8 = 25;
const A_BANG: u8 = 26;

static ACT: [u8; 256] = {
    let mut t = [A_OTHER; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        t[c] = match b {
            b' ' | b'\t' => A_BLANK,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' => A_NAME,
            b'0'..=b'9' => A_DIGIT,
            b'"' | b'\'' => A_QUOTE,
            b'/' => A_SLASH,
            b'(' => A_PAREN,
            b')' => A_CLOSE_PAREN,
            b'[' => A_BRACKET,
            b']' => A_CLOSE_BRACKET,
            b'{' => A_CURLY,
            b'}' => A_CLOSE_CURLY,
            b'.' => A_DOT,
            b';' => A_SEMI,
            b',' => A_COMMA,
            b':' => A_COLON,
            b'?' => A_QUESTION,
            b'=' => A_EQ,
            b'%' | b'&' | b'|' | b'^' | b'~' | b'>' => A_OP,
            b'!' => A_BANG,
            b'*' => A_STAR,
            b'<' => A_LT,
            b'`' => A_TICK,
            b'+' | b'-' => A_SIGN,
            b'@' => A_AT,
            b'#' => A_HASH,
            0x80..=0xff => A_HIGH,
            _ => A_OTHER,
        };
        c += 1;
    }
    t
};

/// A name's bytes' classes, OR-ed as it is read (0 for a byte no name holds past its first): a
/// capital, a lowercase letter, a `$`, a byte past ASCII (X-090's).
const C_NAME: u8 = 1;
const C_UP: u8 = 2;
const C_LO: u8 = 4;
const C_HI: u8 = 8;
const C_DOLLAR: u8 = 16;
static CLS: [u8; 256] = {
    let mut t = [0; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        t[c] = match b {
            b'A'..=b'Z' => C_NAME | C_UP,
            b'a'..=b'z' => C_NAME | C_LO,
            b'0'..=b'9' | b'_' => C_NAME,
            b'$' => C_NAME | C_DOLLAR,
            0x80..=0xff => C_NAME | C_HI,
            _ => 0,
        };
        c += 1;
    }
    t
};

/// A name's ASCII bytes past its first: `[$\w]`.
static NAME: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 128 {
        t[c] = CLS[c] != 0;
        c += 1;
    }
    t
};

/// The scan of a line: its frames, the previous token, and where the writing goes.
pub(super) struct Scan<'b, 'c, K: Sink> {
    b: &'b [u8],
    stack: [u8; MAX],
    depth: usize,
    /// The top frame, `stack[depth - 1]`.
    cur: u8,
    /// The previous significant token ([`P_START`] …).
    prev: u8,
    /// TSX: a `<` where an expression starts is JSX's.
    tsx: bool,
    /// Whether the open destructuring is a `const`'s (its names constants).
    bind_const: bool,
    /// How many template substitutions are open, up to [`SUBSTS_MAX`] (what a state keeps, so a
    /// block counts as its lines do): a bracket in one is the template's string (bat's
    /// `meta.template.expression` under `string.template`).
    substs: u8,
    /// The depth a declaration on the line declared a name at (0 for none) and whether it is a
    /// `const`'s: a `,` there declares the next.
    decl: (usize, bool),
    /// Where the spans so far end, for a text frame's fill: kept by the text frames only.
    at: usize,
    groups: [MaybeUninit<Group>; GROUPS],
    ngroups: usize,
    assign: Assign,
    /// Where a string after `RegExp(` would start: it's a regex.
    re_arg: usize,
    /// An `async` before a `(` (`async (…) =>`): the `(`'s place, the `async`'s, and the
    /// writing's mark from before it.
    asy: (usize, usize, Mark),
    k: K,
    out: &'b mut Out<'c>,
}

impl<'b, 'c, K: Sink> Scan<'b, 'c, K> {
    /// A scan of `b`, its spans from `from`, returned as a literal so that it is built in place;
    /// the state is [`Scan::load`]'s.
    #[inline(always)]
    fn fresh(b: &'b [u8], tsx: bool, out: &'b mut Out<'c>, k: K) -> Self {
        Scan {
            b,
            stack: [F_TOP; MAX],
            depth: 1,
            cur: F_TOP,
            prev: P_START,
            tsx,
            bind_const: false,
            substs: 0,
            decl: (0, false),
            at: 0,
            // read only below `ngroups`
            groups: [const { MaybeUninit::uninit() }; GROUPS],
            ngroups: 0,
            assign: NO_ASSIGN,
            re_arg: usize::MAX,
            asy: (usize::MAX, 0, Mark::NONE),
            k,
            out,
        }
    }

    /// The stack, and what is read from it and the state's bits, from `state`.
    #[inline(always)]
    fn load(&mut self, state: &State) {
        self.depth = state.load(&mut self.stack);
        self.cur = self.stack[(self.depth - 1) & (MAX - 1)];
        let arg = state.flags();
        self.prev = (arg & ARG_PREV) as u8;
        self.substs = ((arg & ARG_SUBSTS) >> ARG_SUBSTS_SHIFT) as u8;
        self.bind_const = arg & ARG_BIND_CONST != 0;
    }

    /// The state's `arg`: the previous token, the open substitutions, a `const`'s
    /// destructuring.
    #[inline(always)]
    fn arg(&self) -> u16 {
        u16::from(self.saved_prev())
            | u16::from(self.substs) << ARG_SUBSTS_SHIFT
            | if self.bind_const { ARG_BIND_CONST } else { 0 }
    }

    /// The previous token a state keeps: a line that ended after a value at the top level or in
    /// a block a statement's start, as the next line would read it ([`Scan::line_start`]), so
    /// a closed state is the default one.
    #[inline(always)]
    fn saved_prev(&self) -> u8 {
        if self.prev == P_VALUE && self.cur <= F_BLOCK {
            P_START
        } else {
            self.prev
        }
    }

    /// The state this line leaves.
    #[inline(always)]
    fn keep(&self) -> State {
        State::keep(&self.stack, self.depth, self.arg())
    }

    /// The line lexed.
    #[inline(always)]
    fn lex(&mut self) {
        self.ngroups = 0;
        self.assign.value = NO_VALUE;
        self.re_arg = usize::MAX;
        self.asy.0 = usize::MAX;
        self.decl.0 = 0;
        let mut p = 0;
        if self.prev == P_VALUE {
            self.line_start();
        }
        if self.cur < TEXT {
            p = indent(self.b);
            if self.substs != 0 && p > 0 && self.b.get(p) != Some(&b'/') {
                // as the run loop's blanks: before a `/`, `slash`'s to write
                self.tok_far(0, p, TPL);
            }
        }
        if self.depth == 1 && self.prev == P_START && self.b.starts_with(b"#!") {
            p = self.shebang();
        }
        self.run(p);
    }

    /// A `#!` line at the top level, a statement's start: a shebang's (bat's only on a file's
    /// first line, which a state can't tell).
    #[cold]
    #[inline(never)]
    fn shebang(&mut self) -> usize {
        self.tok_far(0, 2, SHEBANG_MARK);
        self.tok_far(2, self.b.len(), SHEBANG)
    }

    /// A line after one that ended after a value: the frames that line closed (a
    /// declaration's, a field's, an annotation's; a type alias's unless the line goes on with
    /// one of a type's operators), and the place it starts at as bat reads a line (a
    /// statement's, a member's).
    #[inline(always)]
    fn line_start(&mut self) {
        loop {
            match self.cur {
                F_TOP | F_BLOCK => {
                    self.prev = P_START;
                    return;
                }
                F_CLASS_BODY => {
                    self.prev = P_MEMBER;
                    return;
                }
                F_T_CURLY => {
                    self.prev = P_KEY;
                    return;
                }
                F_TYPE | F_TYPE_AS | F_TYPE_RET | F_FIELD => self.pop(),
                F_TYPE_DECL => {
                    if self.type_goes_on() {
                        return;
                    }
                    self.pop();
                }
                _ => return,
            }
        }
    }

    /// Whether the line goes on with a type: a type's operator first (bat's alias ends at an
    /// empty line), or a comment, which leaves the previous token, so the line after it is
    /// asked again.
    #[inline(never)]
    fn type_goes_on(&self) -> bool {
        let b = self.b;
        let s = skip_blanks(b, 0);
        matches!(b.get(s), Some(b'|' | b'&' | b'?' | b':' | b'=' | b'.'))
            || (b.get(s) == Some(&b'/') && matches!(b.get(s + 1), Some(b'/' | b'*')))
            || (b.len() - s.min(b.len()) >= 7 && &b[s..s + 7] == b"extends")
    }

    #[inline(always)]
    fn top(&self) -> u8 {
        self.cur
    }

    /// The frame under the top one.
    #[inline(always)]
    fn under(&self) -> u8 {
        if self.depth >= 2 {
            self.stack[(self.depth - 2) & (MAX - 1)]
        } else {
            F_TOP
        }
    }

    #[inline(always)]
    fn push(&mut self, c: u8) {
        if self.depth == MAX {
            self.full();
        }
        // below MAX here; the mask (MAX a power of two) spares each push its bounds check
        self.stack[self.depth & (MAX - 1)] = c;
        self.depth += 1;
        self.cur = c;
    }

    #[cold]
    #[inline(never)]
    fn full(&mut self) {
        crate::stack::drop_bottom(&mut self.stack, &mut self.depth);
    }

    #[inline(always)]
    fn pop(&mut self) {
        if self.depth > 1 {
            self.depth -= 1;
            // 1 ≤ depth ≤ MAX: as `push`'s mask
            self.cur = self.stack[(self.depth - 1) & (MAX - 1)];
        }
    }

    #[inline(always)]
    fn set_top(&mut self, c: u8) {
        self.stack[(self.depth - 1) & (MAX - 1)] = c;
        self.cur = c;
    }

    /// Bytes `s..e` are `leaf`.
    #[inline(always)]
    fn tok(&mut self, s: usize, e: usize, leaf: u16) -> usize {
        self.k.merge(&mut self.out, s, e, leaf);
        e
    }

    /// A leaf no built-in theme colors (a support word's, a DOM member's): in a template's
    /// substitution under `string.template` ([`WRAP`]), the template's string as bat's is.
    #[inline(always)]
    fn plainish(&self, leaf: u16) -> u16 {
        if self.substs == 0 { leaf } else { leaf | WRAP }
    }

    /// [`Scan::tok`] out of line, for the paths that run seldom.
    #[inline(never)]
    fn tok_far(&mut self, s: usize, e: usize, leaf: u16) -> usize {
        self.tok(s, e, leaf)
    }

    /// The text from [`Scan::at`] to `p` is `text`, and the spans end at `p`.
    #[inline(always)]
    fn fill_as(&mut self, p: usize, text: u16) {
        let p = p.min(self.b.len());
        if p > self.at {
            if text != PLAIN {
                self.k.merge(&mut self.out, self.at, p, text);
            }
            self.at = p;
        }
    }

    /// The line from `p`.
    fn run(&mut self, mut p: usize) {
        let b = self.b;
        let n = b.len();
        loop {
            let top = self.top();
            if top >= F_TYPE {
                p = if top >= TEXT {
                    self.text_turn(p, top)
                } else {
                    self.type_turn(p, top)
                };
                if p >= n {
                    return;
                }
                continue;
            }
            if p >= n {
                return;
            }
            let mut c = b[p];
            if c == b' ' || c == b'\t' {
                let s = p;
                p += 1;
                while p < n && matches!(b[p], b' ' | b'\t') {
                    p += 1;
                }
                if self.substs != 0 && b.get(p) != Some(&b'/') {
                    // a substitution's blanks are the template's string (before a `/`,
                    // `slash`'s to write: a comment's or a regex literal's may take them)
                    self.tok_far(s, p, TPL);
                }
                if p >= n {
                    return;
                }
                c = b[p];
            }
            p = match ACT[usize::from(c)] {
                A_NAME => self.name(p, top),
                A_PAREN => self.open_paren(p, top),
                A_CLOSE_PAREN => self.close_paren(p),
                A_DOT => self.dot(p),
                A_COMMA => self.comma(p, top),
                A_SEMI => self.semi(p),
                A_QUOTE => self.string(p, c),
                A_EQ => self.equals(p, top),
                A_CURLY => self.open_curly(p, top),
                A_CLOSE_CURLY => self.close_curly(p),
                A_BRACKET => self.open_bracket(p, top),
                A_CLOSE_BRACKET => self.close_bracket(p),
                A_DIGIT => self.number(p),
                A_COLON => self.colon(p, top),
                A_SLASH => self.slash(p),
                A_QUESTION => self.question(p, top),
                A_OP => self.op(p, c),
                A_BANG => self.bang(p),
                A_STAR => self.star(p, top),
                A_SIGN => self.sign(p, c),
                A_LT => self.less(p, top),
                A_TICK => self.template(p),
                A_HIGH => self.high(p, top),
                A_AT => self.decorator(p),
                A_HASH => self.hash(p, top),
                _ => p + 1,
            };
        }
    }

    // --- names ---

    /// A name at `p`, its first byte ASCII's, in code frame `top`.
    #[inline(always)]
    fn name(&mut self, p: usize, top: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let mut bits = CLS[usize::from(b[p])];
        let mut e = p + 1;
        while e < n {
            let k = CLS[usize::from(b[e])];
            if k == 0 {
                break;
            }
            bits |= k;
            e += 1;
        }
        if bits & C_HI != 0 {
            return self.wide_name(p, top);
        }
        self.name_to(p, e, top, bits)
    }

    /// A name `p..e` in code frame `top`, its bytes' classes `bits` (with [`C_HI`], one with
    /// bytes past ASCII, not a word).
    #[inline(always)]
    fn name_to(&mut self, p: usize, e: usize, top: u8, bits: u8) -> usize {
        let b = self.b;
        let prev = self.prev;
        if prev == P_DOT {
            let id = if bits & C_HI == 0 {
                DOT_WORDS.get(&b[p..e])
            } else {
                0
            };
            return self.member(p, e, id, bits);
        }
        let id = if bits & C_HI == 0 {
            WORDS.get(&b[p..e])
        } else {
            0
        };
        if prev >= P_PLACED
            && let Some(q) = self.placed_name(p, e, id, bits)
        {
            return q;
        }
        if top >= F_PLACES
            && let Some(q) = self.framed_name(p, e, top, id, bits)
        {
            return q;
        }
        if id != 0 {
            let info = INFO[usize::from(id)];
            if info.class != words::W_NONE
                && let Some(q) = self.word(p, e, id, info, top)
            {
                return q;
            }
        }
        if prev == P_IMPORT {
            return self.import_default(p, e);
        }
        self.plain(p, e, top, bits)
    }

    /// A default import's name `p..e` (`import N, { … }`): an alias's; its `,` keeps the
    /// import's place for the `{` or `*` after it.
    #[cold]
    #[inline(never)]
    fn import_default(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        self.tok_far(p, e, ALIAS);
        let s = skip_blanks(b, e);
        if b.get(s) == Some(&b',') {
            self.prev = P_IMPORT;
            return self.tok_far(s, s + 1, COMMA);
        }
        self.prev = P_VALUE;
        e
    }

    /// A name `p..e` (`id` its word) in a frame that places names but not here: a keyword's,
    /// or a plain name's.
    #[inline(never)]
    fn unplaced(&mut self, p: usize, e: usize, top: u8, id: u16) -> usize {
        if id != 0 {
            let info = INFO[usize::from(id)];
            if info.class != words::W_NONE
                && let Some(q) = self.word_far(p, e, id, info, top)
            {
                return q;
            }
        }
        let bits = classes(&self.b[p..e]);
        self.plain(p, e, top, bits)
    }

    /// A name `p..e` that is a value where it stands: a call's, an object's, a tagged
    /// template's, a parameter's before `=>`, an assigned name's, a label's, else a variable.
    #[inline(always)]
    fn plain(&mut self, p: usize, e: usize, top: u8, bits: u8) -> usize {
        let b = self.b;
        let start = self.prev == P_START;
        self.prev = P_VALUE;
        let kind = match b.get(e).copied() {
            Some(b'(') => ENTITY_FN,
            Some(b'.') => {
                if b.get(e + 1) == Some(&b'.') {
                    var_kind(b[p], bits)
                } else if b.get(e + 1) == Some(&b'p') && prototype(b, e + 1) {
                    // bat's `X.prototype`: a class's
                    SUPPORT_CLASS
                } else if is_const(b[p], bits) {
                    CONSTANT_OBJ
                } else {
                    OBJECT
                }
            }
            Some(b'`') => TPL_TAG,
            Some(b'<') => return self.maybe_generic_call(p, e, bits),
            Some(b' ' | b'\t' | b'=' | b':' | b'?' | b'!') => {
                return self.name_before(p, e, top, bits, start);
            }
            _ => var_kind(b[p], bits),
        };
        self.tok(p, e, kind)
    }

    /// A name `p..e` before a blank, a `=`, a `:`, a `?` or a `!`: a parameter before `=>`, an
    /// assigned or labeled name, a call through `?.(`, else a variable.
    #[inline(never)]
    fn name_before(&mut self, p: usize, e: usize, top: u8, bits: u8, start: bool) -> usize {
        let b = self.b;
        let s = skip_blanks(b, e);
        let kind = match b.get(s) {
            Some(b'=') => match b.get(s + 1) {
                Some(b'>') => {
                    if self.assign.value == p as u32 {
                        self.function_named();
                    } else if self.async_value(p) {
                        // read before `function_named` clears it
                        let v = self.assign.value as usize;
                        self.function_named();
                        self.tok_far(v, v + 5, ASYNC);
                    }
                    self.tok_far(p, e, PARAM);
                    return e;
                }
                Some(b'=') => var_kind(b[p], bits),
                _ => {
                    self.assigned(p, e, s, A_NAMED);
                    var_kind(b[p], bits)
                }
            },
            Some(b':')
                if matches!(top, F_TOP | F_BLOCK) && start && b.get(s + 1) != Some(&b':') =>
            {
                self.tok_far(p, e, LABEL);
                self.tok_far(s, s + 1, LABEL_SEP);
                self.prev = P_START;
                return s + 1;
            }
            Some(b'(') if s == e => ENTITY_FN,
            Some(b'?') if b.get(s + 1) == Some(&b'.') => match b.get(s + 2) {
                Some(b'(') => ENTITY_FN,
                Some(b'0'..=b'9') => var_kind(b[p], bits),
                _ if s == e => OBJECT,
                _ => var_kind(b[p], bits),
            },
            Some(b'!') if b.get(s + 1) == Some(&b'(') => ENTITY_FN,
            Some(b'`') => TPL_TAG,
            _ => var_kind(b[p], bits),
        };
        self.tok(p, e, kind)
    }

    /// A name `p..e` in a frame that places names ([`F_PLACES`] and up): `None` where it is
    /// the plain loop's.
    #[inline(always)]
    fn framed_name(&mut self, p: usize, e: usize, top: u8, id: u16, bits: u8) -> Option<usize> {
        match top {
            F_PARAMS | F_PARAMS_PLAIN | F_PARAMS_ARROW | F_PATTERN => {
                Some(self.param(p, e, top, id))
            }
            F_CLASS_BODY => Some(self.class_member(p, e, id)),
            F_OBJECT if matches!(self.prev, P_KEY | P_ACCESSOR) => Some(self.key(p, e, id, bits)),
            F_BIND | F_BIND_ARR => Some(self.bound(p, e, top, bits)),
            F_ENUM if matches!(self.prev, P_START | P_OP) => {
                self.prev = P_VALUE;
                Some(self.tok_far(p, e, ENUM_MEMBER))
            }
            F_IMPORT => Some(self.imported(p, e, id)),
            F_FUNC_HEAD => {
                self.prev = P_VALUE;
                Some(self.tok_far(p, e, ENTITY_FN))
            }
            F_CLASS_HEAD | F_IFACE_HEAD => Some(self.head_name(p, e, top, id)),
            F_INDEX if self.prev == P_OP => {
                let s = skip_blanks(self.b, e);
                self.prev = P_VALUE;
                if self.b.get(s) == Some(&b':') {
                    return Some(self.tok_far(p, e, PARAM));
                }
                None
            }
            _ => None,
        }
    }

    /// A name `p..e` the previous token places ([`P_PLACED`] and up): `None` where it is a
    /// plain one after all.
    #[inline(never)]
    fn placed_name(&mut self, p: usize, e: usize, id: u16, bits: u8) -> Option<usize> {
        let b = self.b;
        match self.prev {
            P_TYPE_AFTER => {
                // `instanceof a.b.C`: the path's names a module's, its last the type's
                let mut q = e;
                let mut s = p;
                while b.get(q) == Some(&b'.') && b.get(q + 1).is_some_and(|&c| NAME[usize::from(c)])
                {
                    self.tok_far(s, q, TYPE_MODULE);
                    self.tok_far(q, q + 1, ACCESSOR);
                    s = q + 1;
                    q = s + 1;
                    while q < b.len() && NAME[usize::from(b[q])] {
                        q += 1;
                    }
                }
                self.prev = P_VALUE;
                self.tok_far(s, q, TYPE);
                if b.get(q) == Some(&b'<') {
                    return Some(self.type_args(q));
                }
                Some(q)
            }
            P_DECL | P_DECL_CONST => Some(self.declared(p, e, self.prev == P_DECL_CONST)),
            P_ENUM_NAME => {
                self.prev = P_ENUM_BODY;
                Some(self.tok_far(p, e, TYPE_ENUM))
            }
            P_MODULE_NAME => {
                // `namespace a.b.c`
                Some(self.tok_far(p, e, TYPE_MODULE))
            }
            P_JUMP => {
                self.prev = P_VALUE;
                Some(self.tok_far(p, e, LABEL))
            }
            P_DECORATOR => {
                let leaf = if b.get(e) == Some(&b'.') {
                    OBJECT
                } else if matches!(self.top(), F_PARAMS | F_PARAMS_PLAIN | F_PARAMS_ARROW) {
                    // a parameter's decorator: bat's parameters read it a name
                    READWRITE
                } else {
                    ENTITY_FN
                };
                self.prev = if leaf != OBJECT
                    && matches!(
                        self.top(),
                        F_CLASS_BODY | F_PARAMS | F_PARAMS_PLAIN | F_PARAMS_ARROW
                    )
                    && b.get(e) != Some(&b'(')
                {
                    P_MEMBER
                } else {
                    P_VALUE
                };
                Some(self.tok_far(p, e, leaf))
            }
            P_DOT_MATH..=P_DOT_MODULE => {
                let _ = id;
                let id = if bits & C_HI == 0 {
                    DOT_WORDS.get(&b[p..e])
                } else {
                    0
                };
                Some(self.support_member(p, e, id, bits))
            }
            _ => {
                let _ = bits;
                None
            }
        }
    }

    /// A word of [`WORDS`] at `p..e` (`id`, `info`) in code frame `top`: where the scan goes, or
    /// `None` where it is a name here.
    #[inline(always)]
    fn word(&mut self, p: usize, e: usize, id: u16, info: Info, top: u8) -> Option<usize> {
        match info.class {
            W_VALUE => {
                if info.leaf == THIS && self.b.get(e) == Some(&b'.') {
                    self.prev = P_VALUE;
                    return Some(self.tok(p, e, THIS));
                }
                self.prev = P_VALUE;
                if self.b.get(e) == Some(&b'<') {
                    self.tok_far(p, e, info.leaf);
                    if types::type_args_end(self.b, e, false)
                        .is_some_and(|q| self.b.get(q) == Some(&b'('))
                    {
                        return Some(self.type_args(e));
                    }
                    return Some(e);
                }
                if id == ID_REGEXP {
                    let s = skip_blanks(self.b, e);
                    if self.b.get(s) == Some(&b'(') {
                        self.re_arg = skip_blanks(self.b, s + 1);
                    }
                }
                let leaf = self.plainish(info.leaf);
                Some(self.tok(p, e, leaf))
            }
            W_KW => {
                self.prev = info.after;
                Some(self.tok(p, e, info.leaf))
            }
            W_KW2 => Some(self.keyword(p, e, id, info, top)),
            W_SUPPORT_FN => {
                let s = skip_blanks(self.b, e);
                if self.b.get(s) != Some(&b'(') {
                    return None;
                }
                self.prev = P_VALUE;
                let leaf = if id == ID_REQUIRE && import_equals(self.b, p) {
                    REQUIRE
                } else {
                    info.leaf
                };
                Some(self.tok_far(p, e, leaf))
            }
            W_OBJ => {
                if id == ID_MODULE
                    && self.prev == P_START
                    && let Some(q) = self.namespace(p, e)
                {
                    return Some(q);
                }
                self.prev = if self.b.get(e) == Some(&b'.') {
                    info.after
                } else {
                    P_VALUE
                };
                let leaf = self.plainish(info.leaf);
                Some(self.tok_far(p, e, leaf))
            }
            W_CONTEXT => self.context_word(p, e, id, info, top),
            _ => None,
        }
    }

    /// [`Scan::word`] out of line, for the paths that run seldom.
    #[inline(never)]
    fn word_far(&mut self, p: usize, e: usize, id: u16, info: Info, top: u8) -> Option<usize> {
        self.word(p, e, id, info, top)
    }

    /// A keyword `p..e` (`id`, `info`) in code frame `top` that starts something of its own.
    #[inline(never)]
    fn keyword(&mut self, p: usize, e: usize, id: u16, info: Info, top: u8) -> usize {
        let b = self.b;
        match info.after {
            P_DECL => {
                if id == ID_CONST && has(b, skip_blanks(b, e), b"enum ") {
                    // `const enum`: a modifier
                    self.prev = P_START;
                    return self.tok_far(p, e, MODIFIER);
                }
                self.prev = if id == ID_CONST { P_DECL_CONST } else { P_DECL };
                self.tok(p, e, info.leaf)
            }
            P_IMPORT => {
                if id == ID_IMPORT {
                    match b.get(e) {
                        Some(b'(') => {
                            self.prev = P_VALUE;
                            return self.tok_far(p, e, IMPORT_OP);
                        }
                        Some(b'.') => {
                            self.tok_far(p, e, IMPORT);
                            if has(b, e + 1, b"meta")
                                && !b.get(e + 5).is_some_and(|&c| NAME[usize::from(c)])
                            {
                                self.tok_far(e, e + 1, ACCESSOR);
                                self.prev = P_VALUE;
                                return self.tok_far(e + 1, e + 5, IMPORT_META);
                            }
                            self.prev = P_VALUE;
                            return e;
                        }
                        _ => {}
                    }
                }
                self.tok(p, e, info.leaf);
                self.prev = P_IMPORT;
                e
            }
            _ => {
                if id == ID_FUNCTION {
                    return self.function(p, e);
                }
                if id == ID_CLASS {
                    self.tok_far(p, e, info.leaf);
                    self.push(F_CLASS_HEAD);
                    self.prev = P_OP;
                    return e;
                }
                if id == ID_CASE || (id == ID_DEFAULT && self.prev != P_IMPORT) {
                    if id == ID_DEFAULT && !matches!(self.top(), F_BLOCK) {
                        // `export default` reads as `default` after a value too
                        self.tok_far(p, e, DEFAULT);
                        self.prev = P_OP;
                        return e;
                    }
                    self.tok_far(p, e, info.leaf);
                    self.push(F_CASE);
                    self.prev = P_OP;
                    return e;
                }
                if id == ID_DEFAULT {
                    self.tok_far(p, e, DEFAULT);
                    self.prev = P_OP;
                    return e;
                }
                if id == ID_NEW && b.get(e) == Some(&b'.') {
                    self.tok_far(p, e, NEW);
                    if has(b, e + 1, b"target") {
                        self.tok_far(e, e + 1, ACCESSOR);
                        self.prev = P_VALUE;
                        return self.tok_far(e + 1, e + 7, NEW_TARGET);
                    }
                    self.prev = P_VALUE;
                    return e;
                }
                if id == ID_YIELD && b.get(skip_blanks(b, e)) == Some(&b'*') {
                    let s = skip_blanks(b, e);
                    self.tok_far(p, e, info.leaf);
                    self.prev = P_OP;
                    return self.tok_far(s, s + 1, ASTERISK);
                }
                if info.after == P_ENUM_NAME && top == F_CLASS_BODY {
                    return self.plain(p, e, top, 0);
                }
                self.tok(p, e, info.leaf);
                self.prev = info.after;
                e
            }
        }
    }

    /// A contextual word `p..e` (`id`, `info`): a keyword where it starts something, else
    /// `None`.
    #[inline(never)]
    fn context_word(&mut self, p: usize, e: usize, id: u16, info: Info, _top: u8) -> Option<usize> {
        let b = self.b;
        let s = skip_blanks(b, e);
        let next = b.get(s).copied();
        let named =
            s > e && next.is_some_and(|c| c.is_ascii_alphabetic() || c == b'_' || c == b'$');
        match id {
            ID_ASYNC => {
                // before `function`, an arrow's parameter, a method's name; before a `(`, a
                // variable until the group turns out an arrow's parameters
                match next {
                    Some(b'(') => {
                        let mark = self.k.mark(self.out);
                        self.asy = (s, p, mark);
                        self.tok_far(p, e, READWRITE);
                    }
                    Some(b'<') => {
                        self.tok_far(p, e, ASYNC);
                    }
                    _ if named => {
                        self.tok_far(p, e, ASYNC);
                    }
                    _ => return None,
                }
                self.prev = P_OP;
                Some(e)
            }
            ID_OF if matches!(self.prev, P_VALUE | P_DECLARED) => {
                self.tok_far(p, e, info.leaf);
                self.prev = P_OP;
                Some(e)
            }
            ID_FROM if self.prev == P_VALUE || self.prev == P_IMPORT => {
                self.tok(p, e, info.leaf);
                self.prev = P_OP;
                Some(e)
            }
            ID_AS | ID_SATISFIES => {
                if self.prev == P_IMPORT || (self.prev == P_VALUE && self.after_star(p)) {
                    if id == ID_SATISFIES {
                        return None;
                    }
                    self.tok_far(p, e, info.leaf);
                    self.prev = P_IMPORT;
                    return Some(e);
                }
                if self.prev != P_VALUE || id == ID_SATISFIES && !named {
                    return None;
                }
                if id == ID_SATISFIES {
                    // bat's grammar doesn't know `satisfies`: a name, and a name after it
                    return None;
                }
                if has(b, s, b"const") && !b.get(s + 5).is_some_and(|&c| NAME[usize::from(c)]) {
                    // `as const`: a modifier, unless a type's name follows (`as const T`)
                    let t = skip_blanks(b, s + 5);
                    if !b.get(t).is_some_and(|&c| NAME[usize::from(c)]) {
                        self.tok_far(p, e, AS);
                        self.prev = P_VALUE;
                        return Some(self.tok_far(s, s + 5, MODIFIER));
                    }
                }
                self.tok_far(p, e, AS);
                self.push(F_TYPE_AS);
                self.prev = P_OP;
                Some(e)
            }
            ID_TYPE => {
                if self.prev == P_IMPORT && !(named && types::alias_ahead(b, s)) {
                    if named || matches!(next, Some(b'{' | b'*')) {
                        self.tok_far(p, e, TYPE_CONTROL);
                        return Some(e);
                    }
                    return None;
                }
                if !(named && self.statement_place()) {
                    return None;
                }
                self.tok_far(p, e, TYPE_KW);
                self.push(F_TYPE_DECL);
                self.prev = P_ALIAS_NAME;
                Some(e)
            }
            ID_INTERFACE => {
                if !(named && self.statement_place()) {
                    return None;
                }
                self.tok_far(p, e, INTERFACE_KW);
                self.push(F_IFACE_HEAD);
                self.prev = P_OP;
                Some(e)
            }
            ID_NAMESPACE => {
                if !(named && self.statement_place()) {
                    return None;
                }
                self.namespace(p, e)
            }
            ID_DECLARE | ID_ABSTRACT => {
                if !(named && self.statement_place()) {
                    return None;
                }
                self.tok_far(p, e, MODIFIER);
                self.prev = P_START;
                Some(e)
            }
            ID_GLOBAL => {
                if self.prev == P_START && next == Some(b'{') {
                    self.tok_far(p, e, NODE_OBJ);
                    self.prev = P_VALUE;
                    return Some(e);
                }
                None
            }
            _ => None,
        }
    }

    /// Whether the place is a statement's start: after a statement's end, a block's `{`, or a
    /// modifier (`export`, `declare`).
    #[inline(always)]
    fn statement_place(&self) -> bool {
        matches!(self.prev, P_START | P_IMPORT)
    }

    /// `namespace` or `module` at `p..e` before a name or a string: a module's head.
    #[inline(never)]
    fn namespace(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        let s = skip_blanks(b, e);
        let next = b.get(s).copied()?;
        if s == e
            || !(next.is_ascii_alphabetic()
                || next == b'_'
                || next == b'$'
                || next == b'"'
                || next == b'\'')
        {
            return None;
        }
        self.tok_far(p, e, NAMESPACE_KW);
        self.prev = P_MODULE_NAME;
        Some(e)
    }

    /// `function` at `p..e`: its head's frame pushed.
    #[inline(never)]
    fn function(&mut self, p: usize, e: usize) -> usize {
        if self.assign.value == p as u32 {
            self.function_named();
        } else if self.async_value(p) {
            let v = self.assign.value as usize;
            self.function_named();
            self.tok_far(v, v + 5, ASYNC);
        }
        self.tok_far(p, e, FUNCTION_KW);
        self.push(F_FUNC_HEAD);
        self.prev = P_OP;
        e
    }

    /// A name `p..e` after a `.` (`id` its word): a member.
    #[inline(always)]
    fn member(&mut self, p: usize, e: usize, id: u16, bits: u8) -> usize {
        let b = self.b;
        self.prev = P_VALUE;
        let next = b.get(e).copied();
        let dot = DOT[usize::from(id)];
        let kind = match next {
            Some(b'(') => {
                if dot.call != NO_LEAF {
                    dot.call
                } else {
                    ENTITY_FN
                }
            }
            Some(b'.') if b.get(e + 1) != Some(&b'.') => {
                if dot.plain != NO_LEAF {
                    self.plainish(dot.plain)
                } else if is_const(b[p], bits) {
                    CONSTANT_OBJ_PROP
                } else {
                    OBJECT_PROP
                }
            }
            Some(b' ' | b'\t' | b'=' | b'<' | b'`' | b'?' | b'!') => {
                return self.member_before(p, e, dot.plain, dot.call, bits);
            }
            _ => {
                if dot.plain != NO_LEAF {
                    self.plainish(dot.plain)
                } else if is_const(b[p], bits) {
                    CONSTANT_PROP
                } else {
                    PROPERTY
                }
            }
        };
        if kind == PLAIN {
            return self.plain_member(p, e);
        }
        self.tok(p, e, kind)
    }

    /// A member `p..e` whose leaf is plain (`Number.MAX_VALUE`'s): no span, but the template's
    /// string in a substitution.
    #[cold]
    #[inline(never)]
    fn plain_member(&mut self, p: usize, e: usize) -> usize {
        if self.substs != 0 {
            return self.tok_far(p, e, TPL);
        }
        e
    }

    /// A member `p..e` before a blank, a `=`, a `<`, a `` ` ``, a `?` or a `!`: a call's (`a.b
    /// (`, `a.b<T>(`, `a.b?.(`), a function's assigned (`a.b = () =>`), a tag's, else a
    /// property.
    #[inline(never)]
    fn member_before(&mut self, p: usize, e: usize, plain: u16, call: u16, bits: u8) -> usize {
        let b = self.b;
        let s = skip_blanks(b, e);
        let wrapped = self.plainish(plain);
        let prop = |b: &[u8]| {
            if plain != NO_LEAF {
                wrapped
            } else if is_const(b[p], bits) {
                CONSTANT_PROP
            } else {
                PROPERTY
            }
        };
        let kind = match b.get(s) {
            Some(b'(') => {
                if call != NO_LEAF {
                    call
                } else {
                    ENTITY_FN
                }
            }
            Some(b'<') if s == e => {
                if types::generic_call(b, e) {
                    self.tok_far(p, e, ENTITY_FN);
                    return self.type_args(e);
                }
                prop(b)
            }
            Some(b'`') => TPL_TAG,
            Some(b'?') if b.get(s + 1) == Some(&b'.') && b.get(s + 2) == Some(&b'(') => ENTITY_FN,
            Some(b'!') if b.get(s + 1) == Some(&b'(') => ENTITY_FN,
            Some(b'=') if !matches!(b.get(s + 1), Some(b'=' | b'>')) => {
                self.assigned(p, e, s, A_NAMED);
                prop(b)
            }
            _ => prop(b),
        };
        if kind == PLAIN {
            return self.plain_member(p, e);
        }
        self.tok(p, e, kind)
    }

    /// A member `p..e` of a support object (`Math.`, `console.`, …): its list's leaf, else a
    /// member.
    #[inline(never)]
    fn support_member(&mut self, p: usize, e: usize, id: u16, bits: u8) -> usize {
        let w = &self.b[p..e];
        let cl = words::BITS[usize::from(id)];
        let found = match self.prev {
            P_DOT_MATH => {
                if cl & lists::MATH_FN != 0 {
                    Some(MATH_FN)
                } else if cl & lists::MATH_CONST != 0 {
                    Some(MATH_CONST)
                } else {
                    None
                }
            }
            P_DOT_CONSOLE => (cl & lists::CONSOLE_FN != 0).then_some(CONSOLE_FN),
            P_DOT_JSON => matches!(w, b"parse" | b"stringify").then_some(JSON_FN),
            P_DOT_PROCESS => {
                if cl & lists::PROCESS_PROP != 0 {
                    Some(PROCESS_PROP)
                } else if cl & lists::PROCESS_FN != 0 {
                    Some(PROCESS_FN)
                } else {
                    None
                }
            }
            _ => matches!(
                w,
                b"exports" | b"id" | b"filename" | b"loaded" | b"parent" | b"children"
            )
            .then_some(MODULE),
        };
        match found {
            Some(leaf) => {
                self.prev = P_VALUE;
                self.tok_far(p, e, leaf)
            }
            None => {
                self.prev = P_DOT;
                self.member(p, e, id, bits)
            }
        }
    }

    /// `name =` (or a key's `name:`) at `p..e`, its separator at `s`: a mark kept, for a
    /// value that turns out a function.
    #[inline(always)]
    fn assigned(&mut self, p: usize, e: usize, s: usize, kind: u8) {
        let mark = self.k.mark(self.out);
        self.assigned_at(mark, p, e, s, kind);
    }

    /// [`Scan::assigned`] with the writing's mark from before the name (a quoted key's).
    #[inline(always)]
    fn assigned_at(&mut self, mark: Mark, p: usize, e: usize, s: usize, kind: u8) {
        let v = skip_blanks(self.b, s + 1);
        self.assign = Assign {
            mark,
            name: (p as u32, e as u32),
            sep: s as u32,
            value: v as u32,
            kind,
        };
    }

    /// The function whose value starts at [`Assign::value`] names the assigned name: what was
    /// written since its mark taken back and written again with the name a function's.
    #[inline(never)]
    fn function_named(&mut self) {
        let a = self.assign;
        self.assign.value = NO_VALUE;
        self.k.undo(self.out, a.mark);
        let (s, e) = (a.name.0 as usize, a.name.1 as usize);
        let sep = a.sep as usize;
        match a.kind {
            A_KEY => {
                self.tok_far(s, e, ENTITY_FN);
                self.tok_far(sep, sep + 1, KEY_VALUE);
            }
            A_QUOTED => {
                let q = if self.b[s] == b'"' {
                    (STR_D_BEGIN, STR_D, STR_D_END)
                } else {
                    (STR_S_BEGIN, STR_S, STR_S_END)
                };
                self.tok_far(s, s + 1, q.0);
                self.tok_far(s + 1, e - 1, q.1);
                self.tok_far(e - 1, e, q.2);
                self.tok_far(sep, sep + 1, KEY_VALUE);
            }
            A_CONST => {
                self.tok_far(s, e, CONST_FN);
                self.tok_far(sep, sep + 1, ASSIGN);
            }
            _ => {
                self.tok_far(s, e, ENTITY_FN);
                self.tok_far(sep, sep + 1, ASSIGN);
            }
        }
    }

    /// Whether the assignment's value is `async` before the `(` (or name) at `p`.
    #[inline(always)]
    fn async_value(&self, p: usize) -> bool {
        let v = self.assign.value;
        v != NO_VALUE && {
            let v = v as usize;
            v + 5 <= p && has(self.b, v, b"async") && skip_blanks(self.b, v + 5) == p
        }
    }

    /// A declared name `p..e` (a `const`'s if `konst`): a constant's or a variable's; a
    /// function's where its value is one.
    #[inline(always)]
    fn declared(&mut self, p: usize, e: usize, konst: bool) -> usize {
        let b = self.b;
        self.prev = P_DECLARED;
        self.decl = (self.depth, konst);
        let mut leaf = if konst { CONSTANT } else { READWRITE };
        let s = skip_blanks(b, e);
        match b.get(s) {
            Some(b'=') if !matches!(b.get(s + 1), Some(b'=' | b'>')) => {
                self.assigned(p, e, s, if konst { A_CONST } else { A_NAMED });
            }
            Some(b':') if types::fn_typed(b, s + 1) || types::annotated_fn(b, s + 1) => {
                // typed as a function, or a function its value after the type
                leaf = if konst { CONST_FN } else { ENTITY_FN };
            }
            _ => {}
        }
        self.tok(p, e, leaf)
    }

    /// A name `p..e` in a declaration's destructuring (`top`): a key before `:` a property's,
    /// else a declared name.
    #[inline(never)]
    fn bound(&mut self, p: usize, e: usize, top: u8, bits: u8) -> usize {
        let b = self.b;
        let _ = bits;
        if matches!(self.prev, P_BIND | P_OP | P_KEY) {
            let s = skip_blanks(b, e);
            if top == F_BIND && b.get(s) == Some(&b':') {
                self.tok_far(p, e, OBJ_PROPERTY);
                self.tok_far(s, s + 1, DESTRUCTURING);
                self.prev = P_BIND;
                return s + 1;
            }
            self.prev = P_VALUE;
            let leaf = if self.bind_const { CONSTANT } else { READWRITE };
            return self.tok_far(p, e, leaf);
        }
        self.prev = P_VALUE;
        self.tok_far(p, e, READWRITE)
    }

    /// A name `p..e` in an import's or an export's `{ … }` (`id` its word).
    #[inline(never)]
    fn imported(&mut self, p: usize, e: usize, id: u16) -> usize {
        match id {
            ID_AS => {
                self.prev = P_IMPORT;
                return self.tok_far(p, e, AS);
            }
            ID_DEFAULT => {
                self.prev = P_VALUE;
                return self.tok_far(p, e, DEFAULT);
            }
            _ => {}
        }
        let s = skip_blanks(self.b, e);
        // the name after `as` an alias (`{type Props as P}`)
        let leaf = if has(self.b, s, b"as ") && !after_as(self.b, p) {
            READWRITE
        } else {
            ALIAS
        };
        self.prev = P_VALUE;
        self.tok(p, e, leaf)
    }

    /// A name `p..e` in a class's or an interface's head (`top`): its name, `extends`,
    /// `implements` and what they name.
    #[inline(never)]
    fn head_name(&mut self, p: usize, e: usize, top: u8, id: u16) -> usize {
        let b = self.b;
        if id == ID_EXTENDS || id == ID_IMPLEMENTS {
            self.prev = P_EXTENDS;
            return self.tok_far(p, e, MODIFIER);
        }
        if self.prev == P_EXTENDS {
            // a path's names, the last the class inherited (a function's where it's called:
            // `extends mixin(A)`, `extends mock<T>()`)
            let leaf = match b.get(e) {
                Some(b'.') => TYPE_MODULE,
                Some(b'(') => ENTITY_FN,
                Some(b'<')
                    if types::type_args_end(b, e, false)
                        .is_some_and(|q| b.get(q) == Some(&b'(')) =>
                {
                    ENTITY_FN
                }
                _ => INHERITED,
            };
            self.tok_far(p, e, leaf);
            self.prev = if leaf == TYPE_MODULE {
                P_EXTENDS_PATH
            } else {
                P_EXTENDS
            };
            if b.get(e) == Some(&b'<') {
                return self.type_args(e);
            }
            return e;
        }
        if self.prev == P_VALUE {
            // after the name: a word no head takes
            return self.unplaced(p, e, top, id);
        }
        self.prev = P_VALUE;
        self.tok_far(
            p,
            e,
            if top == F_CLASS_HEAD {
                TYPE_CLASS
            } else {
                TYPE_INTERFACE
            },
        )
    }

    /// A key `p..e` at an object's key place (`id` its word): a key before `:`, a method's
    /// name before `(` or `<`, an accessor's or a modifier's word before a name, else a
    /// shorthand's variable.
    #[inline(never)]
    fn key(&mut self, p: usize, e: usize, id: u16, bits: u8) -> usize {
        let b = self.b;
        let s = if b.get(e).is_some_and(|&c| c == b' ' || c == b'\t') {
            skip_blanks(b, e)
        } else {
            e
        };
        match b.get(s) {
            Some(b':') => {
                self.assigned(p, e, s, A_KEY);
                self.tok(s, s + 1, KEY_VALUE);
                self.prev = P_OP;
                s + 1
            }
            Some(b'(') => {
                self.tok_far(p, e, ENTITY_FN);
                self.prev = P_KEYED;
                e
            }
            Some(b'<') => {
                self.tok_far(p, e, ENTITY_FN);
                self.prev = P_KEYED;
                self.type_params(s)
            }
            Some(b'?') if b.get(s + 1) == Some(&b'(') => {
                self.tok_far(p, e, ENTITY_FN);
                self.tok_far(s, s + 1, OPTIONAL);
                self.prev = P_KEYED;
                s + 1
            }
            Some(&c)
                if s > e
                    && self.prev == P_KEY
                    && (c.is_ascii_alphabetic()
                        || c == b'_'
                        || c == b'$'
                        || c == b'*'
                        || c == b'['
                        || c == b'"'
                        || c == b'\'') =>
            {
                // `get name(`, `set name(`, `async name(`: a modifier
                let leaf = match id {
                    ID_GET | ID_SET => PROPERTY_KW,
                    ID_ASYNC => ASYNC,
                    _ => {
                        self.prev = P_VALUE;
                        return self.tok_far(p, e, var_kind(b[p], bits));
                    }
                };
                self.tok_far(p, e, leaf);
                // the key place stays, an accessor's after `get` or `set`
                if leaf == PROPERTY_KW {
                    self.prev = P_ACCESSOR;
                }
                e
            }
            next => {
                self.prev = P_VALUE;
                // a shorthand's name is bat's variable, a support word's too (`{ document }`)
                let leaf = if id != 0
                    && INFO[usize::from(id)].class == W_VALUE
                    && !matches!(next, None | Some(b',' | b'}'))
                {
                    INFO[usize::from(id)].leaf
                } else {
                    var_kind(b[p], bits)
                };
                self.tok_far(p, e, leaf)
            }
        }
    }

    /// A parameter `p..e` (`id` its word), in [`F_PARAMS`] or a [`F_PATTERN`].
    #[inline(always)]
    fn param(&mut self, p: usize, e: usize, top: u8, id: u16) -> usize {
        let b = self.b;
        if self.prev != P_OP && self.prev != P_KEY && self.prev != P_MEMBER {
            // after a value, something else
            return self.unplaced(p, e, top, id);
        }
        if top == F_PARAMS_PLAIN
            && (!param_follows(b, e)
                || (id != 0 && id != ID_THIS && INFO[usize::from(id)].class == W_VALUE))
        {
            // `undefined` at a line's end stays bat's constant
            return self.unplaced(p, e, top, id);
        }
        if top == F_PATTERN
            && self.under() != F_PATTERN
            && b.get(skip_blanks(b, e)) == Some(&b':')
            && self.prev == P_KEY
        {
            // a key before `:` in an object pattern renames
            let s = skip_blanks(b, e);
            self.tok_far(p, e, OBJ_PROPERTY);
            self.tok_far(s, s + 1, DESTRUCTURING);
            self.prev = P_OP;
            return s + 1;
        }
        if matches!(top, F_PARAMS | F_PARAMS_PLAIN | F_PARAMS_ARROW)
            && id != 0
            && self.prev != P_KEY
        {
            let info = INFO[usize::from(id)];
            if info.class == W_CONTEXT && info.after == P_MEMBER {
                // `public`, `private`, `readonly`: a parameter property's modifier
                let s = skip_blanks(b, e);
                if s > e && b.get(s).is_some_and(|&c| NAME[usize::from(c)]) {
                    self.prev = P_MEMBER;
                    return self.tok_far(p, e, MODIFIER);
                }
            }
            if id == ID_THIS {
                self.prev = P_VALUE;
                return self.tok_far(p, e, THIS_PARAM);
            }
        }
        self.prev = P_VALUE;
        // a parameter typed as a function is one's (`cb: () => void`)
        // (its `:` or `?:` right after it, as bat's lookahead)
        let typed = match b.get(e) {
            Some(b':') => types::fn_typed(b, e + 1),
            Some(b'?') => b.get(e + 1) == Some(&b':') && types::fn_typed(b, e + 2),
            _ => false,
        };
        self.tok(p, e, if typed { ENTITY_FN } else { PARAM })
    }

    /// A class member's name `p..e` (`id` its word), or its modifier.
    #[inline(never)]
    fn class_member(&mut self, p: usize, e: usize, id: u16) -> usize {
        let b = self.b;
        if !matches!(self.prev, P_MEMBER | P_ACCESSOR | P_START) {
            return self.unplaced(p, e, F_CLASS_BODY, id);
        }
        let s = skip_blanks(b, e);
        let next = b.get(s).copied();
        if id != 0 && self.prev != P_ACCESSOR {
            let info = INFO[usize::from(id)];
            let named = s > e
                && next.is_some_and(|c| {
                    c.is_ascii_alphabetic()
                        || c == b'_'
                        || c == b'$'
                        || c == b'['
                        || c == b'*'
                        || c == b'#'
                        || c == b'"'
                        || c == b'\''
                        || c == b'{'
                });
            if info.class == W_CONTEXT && named {
                match info.after {
                    P_MEMBER => {
                        self.tok(p, e, MODIFIER);
                        return e;
                    }
                    P_ACCESSOR => {
                        self.tok_far(p, e, PROPERTY_KW);
                        self.prev = P_ACCESSOR;
                        return e;
                    }
                    _ if id == ID_ASYNC => {
                        self.tok_far(p, e, ASYNC);
                        return e;
                    }
                    _ if id == ID_DECLARE || id == ID_ABSTRACT => {
                        self.tok_far(p, e, MODIFIER);
                        return e;
                    }
                    _ => {}
                }
            }
            if id == ID_CONSTRUCTOR && next == Some(b'(') {
                self.tok_far(p, e, STORAGE);
                self.prev = P_KEYED;
                return e;
            }
        }
        match next {
            Some(b'(') => {
                self.tok(p, e, ENTITY_FN);
                self.prev = P_KEYED;
                s
            }
            Some(b'<') => {
                self.tok_far(p, e, ENTITY_FN);
                self.prev = P_KEYED;
                self.type_params(s)
            }
            Some(b'?') | Some(b'!') => {
                let t = skip_blanks(b, s + 1);
                if b.get(t) == Some(&b'(') {
                    self.tok_far(p, e, ENTITY_FN);
                    self.tok_far(s, s + 1, OPTIONAL);
                    self.prev = P_KEYED;
                    return t;
                }
                // typed as a function where its `?:` is right after the name, as bat's lookahead
                let leaf =
                    if s == e && t == s + 1 && b.get(t) == Some(&b':') && types::fn_typed(b, t + 1)
                    {
                        ENTITY_FN
                    } else {
                        OBJ_PROPERTY
                    };
                self.tok_far(p, e, leaf);
                self.tok_far(
                    s,
                    s + 1,
                    if next == Some(b'?') {
                        OPTIONAL
                    } else {
                        DEFINITE
                    },
                );
                self.prev = P_DECLARED;
                self.push(F_FIELD);
                s + 1
            }
            Some(b'=') if b.get(s + 1) != Some(&b'>') => {
                self.assigned(p, e, s, A_NAMED);
                self.tok_far(p, e, OBJ_PROPERTY);
                self.tok_far(s, s + 1, ASSIGN);
                self.push(F_FIELD);
                self.prev = P_OP;
                s + 1
            }
            Some(b':') => {
                let leaf = if s == e && (types::fn_typed(b, s + 1) || types::annotated_fn(b, s + 1))
                {
                    ENTITY_FN
                } else {
                    OBJ_PROPERTY
                };
                self.tok(p, e, leaf);
                self.push(F_FIELD);
                self.prev = P_DECLARED;
                s
            }
            _ => {
                self.prev = P_MEMBER;
                self.tok_far(p, e, OBJ_PROPERTY)
            }
        }
    }

    /// A name past ASCII at `p` (or one with such bytes): read by characters.
    #[inline(never)]
    fn wide_name(&mut self, p: usize, top: u8) -> usize {
        let b = self.b;
        let mut e = p;
        while e < b.len() {
            let c = b[e];
            if c >= 0x80 {
                if !is_word(b, e) {
                    break;
                }
                e += char_len(c);
            } else if NAME[usize::from(c)] {
                e += 1;
            } else {
                break;
            }
        }
        if e == p {
            return p + char_len(b[p]);
        }
        self.name_to(p, e, top, C_HI | C_LO)
    }

    /// A byte past ASCII at `p` in code: a name's start, else a character of its own.
    #[inline(never)]
    fn high(&mut self, p: usize, top: u8) -> usize {
        if is_word(self.b, p) {
            return self.wide_name(p, top);
        }
        p + char_len(self.b[p])
    }

    /// `#` at `p`: a private name's (`#x`, `this.#x`), else a byte of its own.
    #[inline(never)]
    fn hash(&mut self, p: usize, top: u8) -> usize {
        let b = self.b;
        if !b.get(p + 1).is_some_and(|&c| NAME[usize::from(c)]) {
            return p + 1;
        }
        let mut e = p + 2;
        while e < b.len() && NAME[usize::from(b[e])] {
            e += 1;
        }
        if top == F_CLASS_BODY && matches!(self.prev, P_MEMBER | P_ACCESSOR) {
            return self.class_member(p, e, 0);
        }
        if self.prev == P_DOT {
            return self.member(p, e, 0, C_LO);
        }
        self.prev = P_VALUE;
        self.tok_far(p, e, READWRITE)
    }

    // --- punctuation ---

    /// A `(` at `p` in code frame `top`.
    #[inline(always)]
    fn open_paren(&mut self, p: usize, top: u8) -> usize {
        if top == F_FUNC_HEAD {
            self.pop();
            return self.open_params(p, F_PARAMS);
        }
        if self.prev == P_KEYED {
            return self.open_params(p, F_PARAMS);
        }
        if self.prev == P_ARROW_TP {
            return self.open_params(p, F_PARAMS_ARROW);
        }
        if self.prev == P_CATCH {
            // `catch (e: T)`: a parameter's, `catch (e)` a group's
            if types::annotated_name(self.b, p + 1) {
                return self.open_params(p, F_PARAMS);
            }
            self.push(F_PAREN);
            self.prev = P_OP;
            return p + 1;
        }
        if self.prev != P_VALUE
            && line_end_group(self.b, p + 1, self.assign.value == p as u32)
            && (possibly_arrow(self.b, p)
                || (self.assign.value == p as u32 && self.assign.kind == A_KEY))
        {
            // a `(` that ends its line after `(`, `=`, `,`, `=>` or `return`: bat reads an
            // arrow's parameters (`paren-expression-possibly-arrow`), and an assigned name the
            // function's
            return self.line_end_params(p);
        }
        // a group after an operator may be an arrow's parameters: kept with a mark
        if self.prev != P_VALUE && self.ngroups < GROUPS {
            let (mut mark, mut asy) = (self.k.mark(self.out), NO_VALUE);
            if self.asy.0 == p {
                mark = self.asy.2;
                asy = self.asy.1 as u32;
            }
            let assign = if self.assign.value == p as u32 || self.async_value(p) {
                self.assign
            } else {
                NO_ASSIGN
            };
            self.groups[self.ngroups].write(Group {
                mark,
                open: p as u32,
                depth: self.depth as u16 + 1,
                assign,
                asy,
            });
            self.ngroups += 1;
        }
        self.push(F_PAREN);
        self.prev = P_OP;
        self.wrapped(p)
    }

    /// A bracket at `p`, plain but in a template's substitution, where it is the template's
    /// string: where the scan goes on.
    #[inline(always)]
    fn wrapped(&mut self, p: usize) -> usize {
        if self.substs != 0 {
            return self.tok_far(p, p + 1, TPL);
        }
        p + 1
    }

    /// A `(` at `p` that ends its line where an expression starts: an arrow's parameters'.
    #[inline(never)]
    fn line_end_params(&mut self, p: usize) -> usize {
        self.arrow_head(p);
        self.push(F_PARAMS_PLAIN);
        self.prev = P_OP;
        p + 1
    }

    /// Before an arrow's parameters known at their `(` at `p`: the name assigned a value it
    /// starts a function's, an `async` before it the arrow's.
    #[inline(never)]
    fn arrow_head(&mut self, p: usize) {
        if self.assign.value == p as u32 {
            self.function_named();
        } else if self.async_value(p) {
            let v = self.assign.value as usize;
            self.function_named();
            self.tok_far(v, v + 5, ASYNC);
        } else if self.asy.0 == p {
            let a = self.asy.1;
            self.k.undo(self.out, self.asy.2);
            self.tok_far(a, a + 5, ASYNC);
        }
    }

    /// A function's parameters' `(` at `p`.
    #[inline(always)]
    fn open_params(&mut self, p: usize, frame: u8) -> usize {
        self.push(frame);
        self.tok(p, p + 1, PARAMS_BEGIN);
        self.prev = P_OP;
        p + 1
    }

    /// A destructuring's frame closed: past the outermost, its `const` read no more (a closed
    /// state the default one).
    #[inline(always)]
    fn bind_closed(&mut self) {
        if !matches!(self.cur, F_BIND | F_BIND_ARR) {
            self.bind_const = false;
        }
    }

    /// A `)` at `p`.
    #[inline(always)]
    fn close_paren(&mut self, p: usize) -> usize {
        let mut top = self.top();
        if top != F_PAREN {
            top = self.unwind(top, b')');
        }
        match top {
            F_PAREN => {
                if self.ngroups > 0
                    // SAFETY: entries below the count are written
                    && usize::from(unsafe { self.groups[self.ngroups - 1].assume_init_ref() }.depth)
                        == self.depth
                {
                    self.ngroups -= 1;
                    // SAFETY: as above
                    let g = unsafe { self.groups[self.ngroups].assume_init() };
                    if let Some(q) = self.arrow_group(p, g) {
                        return q;
                    }
                }
                self.pop();
                if matches!(
                    self.top(),
                    F_CLASS_BODY | F_PARAMS | F_PARAMS_PLAIN | F_PARAMS_ARROW
                ) {
                    // a decorator's arguments: the member's (or the parameter's) place again
                    self.prev = P_MEMBER;
                    return p + 1;
                }
                self.prev = P_VALUE;
                let b = self.b;
                if (b.get(p + 1) == Some(&b':')
                    || (b.get(p + 1) == Some(&b' ') && b.get(p + 2) == Some(&b':')))
                    && self.line_group_return(p)
                {
                    self.prev = P_ARROW_PARAMS;
                }
                return self.wrapped(p);
            }
            F_PARAMS | F_PARAMS_PLAIN | F_PARAMS_ARROW => {
                self.pop();
                if top != F_PARAMS_PLAIN {
                    self.tok(p, p + 1, PARAMS_END);
                }
                // a declaration's (or a method's) `):` is its return type's, an arrow's the
                // return type its `=>` ends; a possibly-arrow group without one a value
                // (`(…) as T`)
                self.prev = if top == F_PARAMS {
                    P_PARAMS_END
                } else if top == F_PARAMS_PLAIN
                    && !matches!(self.b.get(skip_blanks(self.b, p + 1)), Some(b':' | b'='))
                {
                    P_VALUE
                } else {
                    P_ARROW_PARAMS
                };
                return p + 1;
            }
            _ => {}
        }
        self.prev = P_VALUE;
        p + 1
    }

    /// The group `g` closing at `p`: if a `=>` follows (or a return type and one), its writing
    /// taken back and its `(` read again as parameters' (and an assigned name named), `Some`
    /// with where to go on.
    #[inline(always)]
    fn arrow_group(&mut self, p: usize, g: Group) -> Option<usize> {
        let b = self.b;
        let t = skip_blanks(b, p + 1);
        let arrow = match b.get(t) {
            Some(b'=') => b.get(t + 1) == Some(&b'>'),
            Some(b':') => types::arrow_return(b, t + 1),
            _ => false,
        };
        if !arrow {
            return None;
        }
        Some(self.arrow_params(g))
    }

    /// Whether the `:` after a group's `)` at `p` that no mark of this line holds (its `(` lines
    /// before) is an arrow's return type: its type runs to a `=>` on the line, and no ternary
    /// or `case` takes the `:`. Its names stay as written, as bat's.
    #[cold]
    #[inline(never)]
    fn line_group_return(&self, p: usize) -> bool {
        !matches!(self.top(), F_TERNARY | F_CASE)
            && types::arrow_return(self.b, skip_blanks(self.b, p + 1) + 1)
    }

    /// The group this line marked whose first name the `:` at `p` follows (`(name:`), taken
    /// off the marks.
    #[cold]
    #[inline(never)]
    fn first_name_group(&mut self, p: usize) -> Option<Group> {
        if self.ngroups == 0 {
            return None;
        }
        // SAFETY: entries below the count are written
        let g = unsafe { self.groups[self.ngroups - 1].assume_init() };
        if usize::from(g.depth) != self.depth {
            return None;
        }
        let b = self.b;
        let s = g.open as usize + 1;
        let mut e = s;
        while e < p && NAME[usize::from(b[e])] {
            e += 1;
        }
        if e == s || skip_blanks(b, e) != p {
            return None;
        }
        self.ngroups -= 1;
        Some(g)
    }

    #[inline(never)]
    fn arrow_params(&mut self, g: Group) -> usize {
        let open = g.open as usize;
        self.pop();
        // groups opened inside it are gone with its writing
        if g.assign.value == g.open {
            self.assign = g.assign;
            self.function_named();
        } else if g.assign.value != NO_VALUE {
            let v = g.assign.value as usize;
            self.assign = g.assign;
            self.function_named();
            self.tok_far(v, v + 5, ASYNC);
        } else {
            self.k.undo(self.out, g.mark);
            if g.asy != NO_VALUE {
                let a = g.asy as usize;
                self.tok_far(a, a + 5, ASYNC);
            }
        }
        self.open_params(open, F_PARAMS_ARROW)
    }

    /// The frames above the one a closer `c` closes popped (a ternary's, a `case`'s, a
    /// default's, a declaration's, a field's): the top after.
    #[inline(never)]
    fn unwind(&mut self, mut top: u8, c: u8) -> u8 {
        while matches!(top, F_TERNARY | F_CASE | F_DEFAULT | F_FIELD) && self.depth > 1 {
            if top == F_FIELD && c != b'}' {
                break;
            }
            self.pop();
            top = self.top();
        }
        top
    }

    /// A `[` at `p`.
    #[inline(always)]
    fn open_bracket(&mut self, p: usize, top: u8) -> usize {
        let frame = match top {
            F_OBJECT if matches!(self.prev, P_KEY | P_ACCESSOR) => F_KEY_BRACKET,
            F_CLASS_BODY if matches!(self.prev, P_MEMBER | P_ACCESSOR) => {
                if types::index_signature(self.b, p + 1) {
                    self.push(F_INDEX);
                    self.prev = P_OP;
                    return p + 1;
                }
                F_KEY_BRACKET
            }
            F_PARAMS | F_PARAMS_PLAIN | F_PARAMS_ARROW | F_PATTERN
                if matches!(self.prev, P_OP | P_KEY) =>
            {
                self.push(F_PATTERN);
                self.prev = P_OP;
                return self.tok_far(p, p + 1, BIND_ARR);
            }
            _ if self.prev >= P_DECL
                || (matches!(top, F_BIND | F_BIND_ARR) && self.prev == P_BIND) =>
            {
                if self.prev >= P_DECL {
                    self.bind_const = self.prev == P_DECL_CONST;
                    // its `,` after the pattern declares the next
                    self.decl = (self.depth, self.bind_const);
                }
                self.push(F_BIND_ARR);
                self.prev = P_BIND;
                return self.tok_far(p, p + 1, BIND_ARR);
            }
            _ => F_BRACKET,
        };
        self.push(frame);
        self.prev = P_OP;
        if frame == F_BRACKET {
            return self.wrapped(p);
        }
        p + 1
    }

    /// A `]` at `p`.
    #[inline(always)]
    fn close_bracket(&mut self, p: usize) -> usize {
        let mut top = self.top();
        if top != F_BRACKET {
            top = self.unwind(top, b']');
        }
        match top {
            F_BRACKET => {
                self.pop();
                self.prev = P_VALUE;
                self.wrapped(p)
            }
            F_KEY_BRACKET => {
                self.pop();
                self.prev = P_KEYED;
                if self.top() == F_CLASS_BODY {
                    return self.after_computed(p + 1);
                }
                p + 1
            }
            F_INDEX => {
                self.pop();
                self.prev = P_VALUE;
                // its type, then the member's end
                let s = skip_blanks(self.b, p + 1);
                if self.b.get(s) == Some(&b':') {
                    self.push(F_FIELD);
                    self.prev = P_DECLARED;
                }
                p + 1
            }
            F_PATTERN | F_BIND_ARR => {
                self.pop();
                self.prev = if top == F_BIND_ARR {
                    self.bind_closed();
                    P_DECLARED
                } else {
                    P_VALUE
                };
                self.tok_far(p, p + 1, BIND_ARR)
            }
            _ => {
                self.prev = P_VALUE;
                p + 1
            }
        }
    }

    /// After a computed member's `]` at `p` in a class body: a method's `(`, a field's
    /// annotation or value.
    #[inline(never)]
    fn after_computed(&mut self, p: usize) -> usize {
        let b = self.b;
        let s = skip_blanks(b, p);
        match b.get(s) {
            Some(b'(') | Some(b'<') => p,
            Some(b'?') | Some(b'!') => {
                let t = skip_blanks(b, s + 1);
                if b.get(t) == Some(&b'(') {
                    self.tok_far(s, s + 1, OPTIONAL);
                    return t;
                }
                self.tok_far(s, s + 1, OPTIONAL);
                self.push(F_FIELD);
                self.prev = P_DECLARED;
                s + 1
            }
            Some(b':') => {
                self.push(F_FIELD);
                self.prev = P_DECLARED;
                p
            }
            Some(b'=') => {
                self.tok_far(s, s + 1, ASSIGN);
                self.push(F_FIELD);
                self.prev = P_OP;
                s + 1
            }
            _ => {
                self.prev = P_MEMBER;
                p
            }
        }
    }

    /// A `{` at `p`: a class's body, a pattern's, an object's or a block's.
    #[inline(always)]
    fn open_curly(&mut self, p: usize, top: u8) -> usize {
        let (frame, prev, leaf) = match top {
            F_CLASS_HEAD | F_IFACE_HEAD => {
                self.pop();
                (F_CLASS_BODY, P_MEMBER, BLOCK_BRACE)
            }
            F_PARAMS | F_PARAMS_PLAIN | F_PARAMS_ARROW | F_PATTERN
                if matches!(self.prev, P_OP | P_KEY) =>
            {
                (F_PATTERN, P_KEY, BIND_OBJ)
            }
            _ if self.prev >= P_DECL
                || (matches!(top, F_BIND | F_BIND_ARR) && self.prev == P_BIND) =>
            {
                if self.prev >= P_DECL {
                    self.bind_const = self.prev == P_DECL_CONST;
                    // its `,` after the pattern declares the next
                    self.decl = (self.depth, self.bind_const);
                }
                (F_BIND, P_BIND, BIND_OBJ)
            }
            _ => match self.prev {
                P_OP | P_KEY | P_NEW => (F_OBJECT, P_KEY, BLOCK_BRACE),
                P_IMPORT => (F_IMPORT, P_IMPORT, BLOCK_BRACE),
                P_ENUM_BODY => (F_ENUM, P_START, BLOCK_BRACE),
                _ => (F_BLOCK, P_START, BLOCK_BRACE),
            },
        };
        self.push(frame);
        self.prev = prev;
        self.tok(p, p + 1, leaf)
    }

    /// A `}` at `p`.
    #[inline(always)]
    fn close_curly(&mut self, p: usize) -> usize {
        let mut top = self.top();
        if matches!(top, F_TERNARY | F_CASE | F_DEFAULT | F_FIELD) {
            top = self.unwind(top, b'}');
        }
        match top {
            F_SUBST => {
                self.pop();
                self.substs = self.substs.saturating_sub(1);
                self.tok(p, p + 1, SUBST_END);
                self.at = p + 1;
                return p + 1;
            }
            F_J_EXPR => {
                self.pop();
                self.tok(p, p + 1, JSX_EMBED_END);
                self.at = p + 1;
                return p + 1;
            }
            F_OBJECT | F_IMPORT => {
                self.pop();
                self.prev = P_VALUE;
            }
            F_ENUM => {
                self.pop();
                self.prev = P_START;
            }
            F_PATTERN | F_BIND => {
                self.pop();
                self.prev = if top == F_BIND {
                    self.bind_closed();
                    P_DECLARED
                } else {
                    P_VALUE
                };
                return self.tok_far(p, p + 1, BIND_OBJ);
            }
            F_BLOCK => {
                self.pop();
                self.prev = if self.top() == F_CLASS_BODY {
                    P_MEMBER
                } else {
                    P_START
                };
            }
            F_CLASS_BODY => {
                self.pop();
                self.prev = P_START;
            }
            _ => self.prev = P_START,
        }
        self.tok(p, p + 1, BLOCK_BRACE)
    }

    /// A `.` at `p`: a number's, a spread's, or an accessor.
    #[inline(always)]
    fn dot(&mut self, p: usize) -> usize {
        let b = self.b;
        match b.get(p + 1) {
            Some(b'0'..=b'9') => return self.number(p),
            Some(b'.') if b.get(p + 2) == Some(&b'.') => {
                let leaf = if matches!(
                    self.top(),
                    F_PARAMS | F_PARAMS_PLAIN | F_PARAMS_ARROW | F_PATTERN | F_BIND | F_BIND_ARR
                ) {
                    REST
                } else {
                    SPREAD
                };
                self.prev = if leaf == REST { self.prev } else { P_OP };
                return self.tok_far(p, p + 3, leaf);
            }
            _ => {}
        }
        self.tok(p, p + 1, ACCESSOR);
        if self.prev < P_DOT_MATH || self.prev > P_DOT_MODULE {
            self.prev = match self.prev {
                P_MODULE_NAME => P_MODULE_NAME,
                P_EXTENDS_PATH => P_EXTENDS,
                _ => P_DOT,
            };
        }
        p + 1
    }

    /// A `,` at `p` in frame `top`.
    #[inline(always)]
    fn comma(&mut self, p: usize, mut top: u8) -> usize {
        if matches!(top, F_TERNARY | F_DEFAULT) {
            top = self.unwind(top, b',');
        }
        match top {
            F_OBJECT => {
                self.prev = P_KEY;
                self.tok(p, p + 1, COMMA)
            }
            F_PARAMS | F_PARAMS_PLAIN | F_PARAMS_ARROW => {
                self.prev = P_OP;
                self.tok(p, p + 1, PARAM_SEP)
            }
            F_PATTERN => {
                self.prev = if self.stack[self.depth - 1] == F_PATTERN {
                    P_KEY
                } else {
                    P_OP
                };
                self.tok_far(p, p + 1, COMMA)
            }
            F_BIND | F_BIND_ARR => {
                self.prev = P_BIND;
                self.tok_far(p, p + 1, COMMA)
            }
            F_IMPORT => {
                self.prev = P_IMPORT;
                self.tok(p, p + 1, COMMA)
            }
            F_ENUM => {
                self.prev = P_START;
                self.tok_far(p, p + 1, COMMA)
            }
            F_CLASS_HEAD | F_IFACE_HEAD => self.tok_far(p, p + 1, COMMA),
            _ => {
                if self.depth == self.decl.0 {
                    // a declaration's next declared name
                    self.prev = if self.decl.1 { P_DECL_CONST } else { P_DECL };
                } else if self.prev != P_IMPORT {
                    self.prev = P_OP;
                }
                self.tok(p, p + 1, COMMA)
            }
        }
    }

    /// A `;` at `p`.
    #[inline(always)]
    fn semi(&mut self, p: usize) -> usize {
        let top = self.top();
        self.decl.0 = 0;
        if matches!(top, F_TERNARY | F_CASE | F_FIELD | F_DEFAULT) {
            self.unwind(top, b';');
            if self.top() == F_FIELD {
                self.pop();
            }
        }
        self.prev = if self.top() == F_CLASS_BODY {
            P_MEMBER
        } else {
            P_START
        };
        self.tok(p, p + 1, TERMINATOR)
    }

    /// A `:` at `p` in frame `top`.
    #[inline(always)]
    fn colon(&mut self, p: usize, top: u8) -> usize {
        match self.prev {
            P_PARAMS_END | P_DECLARED => return self.annotation(p, F_TYPE),
            P_ARROW_PARAMS => return self.annotation(p, F_TYPE_RET),
            _ => {}
        }
        match top {
            F_TERNARY => {
                self.pop();
                self.prev = P_OP;
                self.tok_far(p, p + 1, TERNARY)
            }
            F_CASE => {
                self.pop();
                self.prev = P_START;
                self.tok_far(p, p + 1, CASE_SEP)
            }
            // `(name:`: an arrow's parameters, as bat's lookahead reads them (the group read
            // again as them)
            F_PAREN if let Some(g) = self.first_name_group(p) => self.arrow_params(g),
            // a group's `:` after a pattern's `}` or a name: its parameter's type (a group whose
            // `=>` is lines on, as bat reads it)
            F_PARAMS | F_PARAMS_PLAIN | F_PARAMS_ARROW | F_INDEX | F_FIELD | F_PAREN => {
                self.annotation(p, F_TYPE)
            }
            F_PATTERN if self.prev == P_VALUE => self.annotation(p, F_TYPE),
            _ => {
                self.prev = P_OP;
                self.tok_far(p, p + 1, KEY_VALUE)
            }
        }
    }

    /// An annotation's `:` at `p`: the type after it in `frame`.
    #[inline(always)]
    fn annotation(&mut self, p: usize, frame: u8) -> usize {
        self.tok(p, p + 1, ANNOTATION);
        self.push(frame);
        self.prev = P_OP;
        p + 1
    }

    /// A `?` at `p`: optional chaining's `?.`, `??`, a parameter's or a field's `?`, or a
    /// ternary's.
    #[inline(always)]
    fn question(&mut self, p: usize, top: u8) -> usize {
        let b = self.b;
        match b.get(p + 1) {
            Some(b'.') if !matches!(b.get(p + 2), Some(b'0'..=b'9')) => {
                self.tok_far(p, p + 2, ACCESSOR_OPT);
                self.prev = if matches!(b.get(p + 2), Some(b'(' | b'[')) {
                    P_VALUE
                } else {
                    P_DOT
                };
                return p + 2;
            }
            Some(b'?') => {
                let e = if b.get(p + 2) == Some(&b'=') {
                    p + 3
                } else {
                    p + 2
                };
                self.prev = P_OP;
                return self.tok_far(p, e, if e == p + 3 { COMPOUND } else { LOGICAL });
            }
            _ => {}
        }
        if matches!(
            top,
            F_PARAMS | F_PARAMS_PLAIN | F_PARAMS_ARROW | F_PATTERN | F_INDEX
        ) && self.prev == P_VALUE
            && matches!(
                b.get(skip_blanks(b, p + 1)),
                Some(b':' | b',' | b')' | b'=') | None
            )
        {
            // a parameter's `?:` (a `?` before a value a ternary's)
            return self.tok_far(p, p + 1, OPTIONAL);
        }
        self.push(F_TERNARY);
        self.prev = P_OP;
        self.tok_far(p, p + 1, TERNARY)
    }

    /// A `=` at `p` in frame `top`: `=>`, a comparison, or an assignment.
    #[inline(always)]
    fn equals(&mut self, p: usize, top: u8) -> usize {
        let b = self.b;
        let (e, leaf, prev) = match b.get(p + 1) {
            Some(b'>') => (p + 2, ARROW, P_ARROW),
            Some(b'=') if b.get(p + 2) == Some(&b'=') => (p + 3, COMPARISON, P_OP),
            Some(b'=') => (p + 2, COMPARISON, P_OP),
            _ => {
                if matches!(top, F_PARAMS | F_PARAMS_PLAIN | F_PARAMS_ARROW | F_PATTERN) {
                    self.tok_far(p, p + 1, ASSIGN);
                    self.push(F_DEFAULT);
                    self.prev = P_OP;
                    return p + 1;
                }
                (p + 1, ASSIGN, P_OP)
            }
        };
        self.prev = prev;
        self.tok(p, e, leaf)
    }

    /// `literal-operators`' punctuation at `p` (`c`), as one `match` on the bytes there.
    #[inline(always)]
    fn op(&mut self, p: usize, c: u8) -> usize {
        let b = self.b;
        let at1 = b.get(p + 1).copied();
        let at2 = b.get(p + 2).copied();
        let at3 = b.get(p + 3).copied();
        let (len, leaf) = match c {
            b'&' | b'|' if at1 == Some(c) && at2 == Some(b'=') => (3, COMPOUND),
            b'&' | b'|' if at1 == Some(c) => (2, LOGICAL),
            b'%' if at1 == Some(b'=') => (2, COMPOUND),
            b'&' | b'^' | b'|' if at1 == Some(b'=') => (2, COMPOUND_BITWISE),
            b'>' if at1 == Some(b'>') && at2 == Some(b'=') => (3, COMPOUND_BITWISE),
            b'>' if at1 == Some(b'>') && at2 == Some(b'>') && at3 == Some(b'=') => {
                (4, COMPOUND_BITWISE)
            }
            b'~' => (1, BITWISE),
            b'>' if at1 == Some(b'>') && at2 == Some(b'>') => (3, SHIFT),
            b'>' if at1 == Some(b'>') => (2, SHIFT),
            b'&' | b'^' | b'|' => (1, BITWISE),
            b'>' if at1 == Some(b'=') => (2, RELATIONAL),
            b'>' => (1, RELATIONAL),
            _ => (1, ARITHMETIC),
        };
        self.prev = P_OP;
        self.tok(p, p + len, leaf)
    }

    /// A `!` at `p`: `!=`, `!==`, a declaration's definite `!`, or logical (a value's
    /// non-null `!` too).
    #[inline(always)]
    fn bang(&mut self, p: usize) -> usize {
        let b = self.b;
        if b.get(p + 1) == Some(&b'=') {
            let e = if b.get(p + 2) == Some(&b'=') {
                p + 3
            } else {
                p + 2
            };
            self.prev = P_OP;
            return self.tok_far(p, e, COMPARISON);
        }
        if self.prev == P_DECLARED {
            return self.tok_far(p, p + 1, DEFINITE);
        }
        if self.prev != P_VALUE {
            self.prev = P_OP;
        }
        self.tok(p, p + 1, LOGICAL)
    }

    /// A `*` at `p`: a generator's (after `function`, at a member's or key's place), an import's
    /// or an export's all, else an operator.
    #[inline(always)]
    fn star(&mut self, p: usize, top: u8) -> usize {
        let b = self.b;
        if top == F_FUNC_HEAD
            || (top == F_CLASS_BODY && self.prev == P_MEMBER)
            || (self.prev == P_KEY && top == F_OBJECT)
        {
            return self.tok_far(p, p + 1, ASTERISK);
        }
        if self.prev == P_IMPORT {
            return self.tok_far(p, p + 1, IMPORT_ALL);
        }
        let (len, leaf) = match (b.get(p + 1), b.get(p + 2)) {
            (Some(b'*'), Some(b'=')) => (3, COMPOUND),
            (Some(b'*'), _) => (2, ARITHMETIC),
            (Some(b'='), _) => (2, COMPOUND),
            _ => (1, ARITHMETIC),
        };
        self.prev = P_OP;
        self.tok_far(p, p + len, leaf)
    }

    /// Whether a `*` is before `p`, past blanks (`import * as x`).
    fn after_star(&self, p: usize) -> bool {
        let b = self.b;
        let mut k = p;
        while k > 0 && matches!(b[k - 1], b' ' | b'\t') {
            k -= 1;
        }
        k > 0 && b[k - 1] == b'*'
    }

    /// A `+` or `-` at `p`: `++`, `--`, an augmented assignment, or arithmetic.
    #[inline(always)]
    fn sign(&mut self, p: usize, c: u8) -> usize {
        let b = self.b;
        match b.get(p + 1) {
            Some(&d) if d == c => {
                // `++`, `--`: the value stays one
                let prev = if self.prev == P_VALUE { P_VALUE } else { P_OP };
                self.tok_far(p, p + 2, if c == b'+' { INCREMENT } else { DECREMENT });
                self.prev = prev;
                return p + 2;
            }
            Some(b'=') => {
                self.prev = P_OP;
                return self.tok_far(p, p + 2, COMPOUND);
            }
            _ => {}
        }
        self.prev = P_OP;
        self.tok_far(p, p + 1, ARITHMETIC)
    }

    /// A `<` at `p` in frame `top`: a head's type parameters, a cast's or a generic arrow's
    /// (TypeScript) or JSX's (TSX) where an expression starts, else an operator.
    #[inline(always)]
    fn less(&mut self, p: usize, top: u8) -> usize {
        let b = self.b;
        if matches!(top, F_FUNC_HEAD | F_CLASS_HEAD | F_IFACE_HEAD) {
            return self.type_params(p);
        }
        if matches!(self.prev, P_START | P_OP | P_ARROW | P_KEY)
            && let Some(q) = self.angle_start(p)
        {
            return q;
        }
        if self.prev == P_VALUE && p > 0 && b[p - 1] == b')' && types::call_type_args(b, p) {
            // a call's result called with type arguments (`styled(X)<P>(…)`)
            return self.type_args(p);
        }
        let at1 = b.get(p + 1).copied();
        let (len, leaf) = match at1 {
            Some(b'<') if b.get(p + 2) == Some(&b'=') => (3, COMPOUND_BITWISE),
            Some(b'<') => (2, SHIFT),
            Some(b'=') => (2, RELATIONAL),
            _ => (1, RELATIONAL),
        };
        self.prev = P_OP;
        self.tok_far(p, p + len, leaf)
    }

    /// A `/` at `p`: a comment's, a regex literal's, or division.
    #[inline(always)]
    fn slash(&mut self, p: usize) -> usize {
        let b = self.b;
        match b.get(p + 1) {
            Some(b'/') => {
                if self.substs != 0 {
                    self.slash_blanks(p, true);
                }
                return self.line_comment(p);
            }
            Some(b'*') => {
                if self.substs != 0 {
                    self.slash_blanks(p, false);
                }
                return self.block_comment(p);
            }
            _ => {}
        }
        if matches!(self.prev, P_START | P_OP | P_ARROW | P_KEY | P_IMPORT) && p + 1 < b.len() {
            // bat's regex takes the blanks before its `/`
            let s = blanks_before(b, p);
            if s < p {
                self.tok_far(s, p, RE);
            }
            return self.regex_literal(p);
        }
        if self.substs != 0 {
            self.slash_blanks(p, false);
        }
        self.prev = P_OP;
        if b.get(p + 1) == Some(&b'=') {
            return self.tok_far(p, p + 2, COMPOUND);
        }
        self.tok_far(p, p + 1, ARITHMETIC)
    }

    /// The blanks before a `/` at `p` in a template's substitution, which the run loop leaves
    /// unwritten: a line comment's where they start the line (its `LEADING`), else the
    /// template's string.
    #[cold]
    #[inline(never)]
    fn slash_blanks(&mut self, p: usize, line_comment: bool) {
        let s = blanks_before(self.b, p);
        if s < p && !(line_comment && s == 0) {
            self.tok_far(s, p, TPL);
        }
    }

    /// `@` at `p`: a decorator's.
    #[inline(never)]
    fn decorator(&mut self, p: usize) -> usize {
        self.prev = P_DECORATOR;
        self.tok_far(p, p + 1, DECORATOR)
    }

    /// A number at `p` (a digit, or a `.` before a digit): a hex, octal or binary literal,
    /// else digits, a fraction, an exponent; `_` separators, and a BigInt's `n`.
    #[inline(always)]
    fn number(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let digits = |mut e: usize, ok: fn(u8) -> bool| {
            while e < n && (ok(b[e]) || b[e] == b'_') {
                e += 1;
            }
            e
        };
        let mut e = p;
        let mut leaf = DECIMAL;
        if b[e] == b'.' {
            e = digits(e + 1, |c| c.is_ascii_digit());
        } else if b[e] == b'0'
            && matches!(b.get(e + 1), Some(b'x' | b'X' | b'b' | b'B' | b'o' | b'O'))
        {
            leaf = match b[e + 1] {
                b'x' | b'X' => HEX,
                b'b' | b'B' => BINARY,
                _ => OCTAL,
            };
            e = digits(e + 2, |c| c.is_ascii_hexdigit());
        } else {
            e = digits(e, |c| c.is_ascii_digit());
            if b.get(e) == Some(&b'.') && b.get(e + 1) != Some(&b'.') {
                e = digits(e + 1, |c| c.is_ascii_digit());
            }
        }
        if leaf == DECIMAL && matches!(b.get(e), Some(b'e' | b'E')) {
            let s = if matches!(b.get(e + 1), Some(b'-' | b'+')) {
                e + 2
            } else {
                e + 1
            };
            if matches!(b.get(s), Some(b'0'..=b'9')) {
                e = digits(s, |c| c.is_ascii_digit());
            }
        }
        self.prev = if matches!(self.prev, P_KEY | P_MEMBER)
            && matches!(self.top(), F_OBJECT | F_CLASS_BODY)
        {
            P_KEYED
        } else {
            P_VALUE
        };
        if b.get(e) == Some(&b'n') {
            self.tok_far(p, e, leaf);
            let big = match leaf {
                HEX => HEX_BIG,
                BINARY => BINARY_BIG,
                OCTAL => OCTAL_BIG,
                _ => DECIMAL_BIG,
            };
            return self.tok_far(e, e + 1, big);
        }
        self.tok(p, e, leaf)
    }
}

/// Whether name `p..`'s first byte `first` and classes `bits` make it a constant's:
/// `[A-Z][A-Z0-9_$]*` (bat's `variable.other.constant`).
#[inline(always)]
fn is_const(first: u8, bits: u8) -> bool {
    first.is_ascii_uppercase() && bits & (C_LO | C_HI) == 0
}

/// A plain name's kind from its first byte and classes: a constant's capitals, else a
/// variable's.
#[inline(always)]
fn var_kind(first: u8, bits: u8) -> u16 {
    if is_const(first, bits) {
        CONSTANT
    } else {
        READWRITE
    }
}

/// Whether `require` at `p` is an `import X = require(…)`'s: `=` before it, the line an
/// `import`'s (or an `export import`'s).
#[cold]
#[inline(never)]
fn import_equals(b: &[u8], p: usize) -> bool {
    let k = blanks_before(b, p);
    if k == 0 || b[k - 1] != b'=' {
        return false;
    }
    let s = skip_blanks(b, 0);
    let s = if has(b, s, b"export ") {
        skip_blanks(b, s + 7)
    } else {
        s
    };
    has(b, s, b"import ")
}

/// Where the blanks before `p` start.
#[inline(always)]
fn blanks_before(b: &[u8], mut p: usize) -> usize {
    while p > 0 && matches!(b[p - 1], b' ' | b'\t') {
        p -= 1;
    }
    p
}

#[inline(always)]
fn skip_blanks(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && matches!(b[p], b' ' | b'\t') {
        p += 1;
    }
    p
}

/// Whether `s` is at `i`.
#[inline(always)]
fn has(b: &[u8], i: usize, s: &[u8]) -> bool {
    crate::has_at(b, i, s)
}

// The entry points.

/// Lex `b`, a line of TypeScript (TSX if `tsx`) without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], tsx: bool, state: &mut State, out: &mut Out) {
    let mut s = Scan::fresh(b, tsx, out, Spans::new());
    s.load(state);
    s.k.open(s.out, b.len() / 4 + 16);
    s.lex();
    s.k.close(s.out);
    *state = s.keep();
}

/// [`line`]'s runs by theme rule pushed to `runs` (X-074, X-068's sink): [`crate::lex_runs`] for
/// TypeScript.
#[inline(never)]
pub fn line_runs(
    b: &[u8],
    state: &mut State,
    out: &mut Out,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
) {
    line_runs_of(false, b, state, out, rules, runs);
}

/// [`line_runs`] for TSX.
#[inline(never)]
pub fn line_runs_tsx(
    b: &[u8],
    state: &mut State,
    out: &mut Out,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
) {
    line_runs_of(true, b, state, out, rules, runs);
}

#[inline(never)]
fn line_runs_of(
    tsx: bool,
    b: &[u8],
    state: &mut State,
    out: &mut Out,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
) {
    let mut s = Scan::fresh(b, tsx, out, Runs::new(rules, runs));
    s.load(state);
    s.lex();
    *state = s.keep();
    s.k.end_line(b.len());
}

/// A block of TypeScript on one scan opened from `state` ([`crate::Carry`], X-067), `state`
/// left as the last line leaves it.
pub fn block(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    block_of(false, state, spans, pass)
}

/// [`block`] for TSX.
pub fn block_tsx(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    block_of(true, state, spans, pass)
}

fn block_of(
    tsx: bool,
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    let mut out = Out::new(spans);
    out.0.clear();
    let mut s = Scan::fresh(b"", tsx, &mut out, Spans::new());
    s.load(state);
    s.k.open(s.out, 64);
    let rest = pass(&mut s);
    *state = s.keep_block();
    rest
}

/// [`block`]'s runs by theme rule pushed to `runs` ([`crate::CarryRuns`], X-074), the lines'
/// loop compiled with the scan.
pub fn block_runs(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    lines: &mut crate::RunLines,
) -> Option<usize> {
    block_runs_of(false, state, spans, rules, runs, lines)
}

/// [`block_runs`] for TSX.
pub fn block_runs_tsx(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    lines: &mut crate::RunLines,
) -> Option<usize> {
    block_runs_of(true, state, spans, rules, runs, lines)
}

fn block_runs_of(
    tsx: bool,
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    lines: &mut crate::RunLines,
) -> Option<usize> {
    let mut out = Out::new(spans);
    out.0.clear();
    let mut s = Scan::fresh(b"", tsx, &mut out, Runs::new(rules, runs));
    s.load(state);
    let rest = lines.pass(&mut s);
    *state = s.keep_block();
    rest
}

impl<K: Sink> Scan<'_, '_, K> {
    /// The state a block's pass leaves: as the last line left it (a block that lexed no line
    /// leaves the state it was given).
    fn keep_block(&self) -> State {
        State::keep(&self.stack, self.depth, self.arg())
    }
}

impl crate::Carry for Scan<'_, '_, Spans> {
    fn next(&mut self, b: &[u8]) -> &[crate::Span] {
        self.k.start_line();
        // SAFETY: the line is set back to an empty one before `b`'s borrow ends
        self.carry(unsafe { crate::line_for(b) });
        self.k.close(self.out);
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

impl<'b, K: Sink> Scan<'b, '_, K> {
    /// `b`, the block's next line, lexed on this scan; the scan's line an empty one again after.
    #[inline(always)]
    fn carry(&mut self, b: &'b [u8]) {
        self.b = b;
        self.at = 0;
        self.lex();
        // the line's end as a state's save and load leave it
        crate::stack::hold(&mut self.stack, &mut self.depth);
        self.b = b"";
    }
}

/// Whether a `(` before `p` ends its line, or (an assigned value's, `assigned`) only a
/// pattern's `{` or `[` follows it there.
#[inline(always)]
fn line_end_group(b: &[u8], p: usize, assigned: bool) -> bool {
    let s = skip_blanks(b, p);
    s >= b.len() || (assigned && matches!(b[s], b'{' | b'[') && skip_blanks(b, s + 1) >= b.len())
}

/// Whether the token before `p` is one after which bat's `paren-expression-possibly-arrow`
/// starts at a `(` that ends the line: `(`, `=` (an assignment's), `,`, `return`, `async`
/// after one of them.
#[inline(never)]
fn possibly_arrow(b: &[u8], p: usize) -> bool {
    let mut k = p;
    while k > 0 && matches!(b[k - 1], b' ' | b'\t') {
        k -= 1;
    }
    if k >= 5 && &b[k - 5..k] == b"async" {
        k -= 5;
        while k > 0 && matches!(b[k - 1], b' ' | b'\t') {
            k -= 1;
        }
    }
    if k == 0 {
        return false;
    }
    match b[k - 1] {
        b'(' | b',' => true,
        b'=' => !(k >= 2 && matches!(b[k - 2], b'=' | b'!' | b'<' | b'>')),
        b'n' => k >= 6 && &b[k - 6..k] == b"return" && (k == 6 || !NAME[usize::from(b[k - 7])]),
        _ => false,
    }
}

/// Whether the token before `p` is `as`.
fn after_as(b: &[u8], p: usize) -> bool {
    let k = blanks_before(b, p);
    k >= 2 && &b[k - 2..k] == b"as" && (k == 2 || !NAME[usize::from(b[k - 3])])
}

/// Whether `prototype` is the whole name at `p`.
#[inline(never)]
fn prototype(b: &[u8], p: usize) -> bool {
    has(b, p, b"prototype") && !b.get(p + 9).is_some_and(|&c| NAME[usize::from(c)])
}

/// Whether a name ending at `e` in a possibly-arrow `(` is a parameter: `:`, `?` (not `?.`),
/// `,`, `=` (not `==`, `=>`) or the line's end after it.
#[inline(never)]
fn param_follows(b: &[u8], e: usize) -> bool {
    let s = skip_blanks(b, e);
    match b.get(s) {
        None => true,
        Some(b':' | b',') => true,
        Some(b'?') => b.get(s + 1) != Some(&b'.'),
        Some(b'=') => !matches!(b.get(s + 1), Some(b'=' | b'>')),
        _ => false,
    }
}

/// Where a line's indentation ends: sixteen bytes a step (SSE2's compares, x86-64's baseline),
/// a short line's a byte a step.
#[inline(always)]
fn indent(b: &[u8]) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::*;
        let mut p = 0;
        while p + 16 <= b.len() {
            // SAFETY: 16 bytes from `p` are in `b`; SSE2 is in x86-64's baseline
            let m = unsafe {
                let v = _mm_loadu_si128(b.as_ptr().add(p).cast());
                let blank = _mm_or_si128(
                    _mm_cmpeq_epi8(v, _mm_set1_epi8(b' ' as i8)),
                    _mm_cmpeq_epi8(v, _mm_set1_epi8(b'\t' as i8)),
                );
                _mm_movemask_epi8(blank) as u32
            };
            if m != 0xffff {
                return p + m.trailing_ones() as usize;
            }
            p += 16;
        }
        skip_blanks(b, p)
    }
    #[cfg(not(target_arch = "x86_64"))]
    skip_blanks(b, 0)
}

/// A name's bytes' classes OR-ed ([`CLS`]), for a capitalized name's.
#[inline(never)]
fn classes(w: &[u8]) -> u8 {
    w.iter().fold(0, |k, &c| k | CLS[usize::from(c)])
}

#[cfg(test)]
mod tests {
    use crate::tests::marked;
    use crate::{Lang, State};

    fn ts(lines: &[&'static str]) -> Vec<(&'static str, &'static str)> {
        marked(Lang::TypeScript, lines)
    }

    fn has(m: &[(&str, &str)], text: &str, scopes: &str) {
        assert!(m.contains(&(text, scopes)), "{text:?} {scopes:?} in {m:?}");
    }

    /// A call is a function's name, `new Foo(` too; a name before `.` an object's, a member a
    /// property's, a member in bat's support lists its own (`.length` plain, `.push(` a support
    /// function's).
    #[test]
    fn names_by_their_place() {
        let m = ts(&[
            "const a = foo(1), b = new Bar(2), c = obj.items.length;",
            "this.list.push(x); console.log(x); JSON.parse(s); new Map<string, T>();",
        ]);
        has(&m, "foo", "entity.name.function.ts");
        has(&m, "Bar", "entity.name.function.ts");
        has(&m, "obj", "variable.other.object.ts");
        has(&m, "items", "variable.other.object.property.ts");
        has(&m, "length", "support.variable.property.ts");
        has(&m, "push", "support.function.ts");
        has(&m, "log", "support.function.console.ts");
        has(&m, "parse", "support.function.json.ts");
        has(&m, "Map", "support.class.builtin.ts");
        has(&m, "string", "support.type.primitive.ts");
        has(&m, "a", "variable.other.constant.ts");
    }

    /// An arrow's parameters are read as a group's until its `=>` (or a return type and its
    /// `=>`), then again as parameters; a name its value is assigned to is the function's.
    #[test]
    fn arrows_are_decided_at_their_arrow() {
        let m = ts(&[
            "export const f = async (x: number, y?: string): Promise<void> => { g(x); };",
            "items.map((item) => item.id).filter(n => n > 0);",
            "const h = <T,>(v: T): T => v;",
            "(p) + 1;",
        ]);
        has(
            &m,
            "f",
            "variable.other.constant.ts entity.name.function.ts",
        );
        has(&m, "async", "storage.modifier.async.ts");
        has(&m, "x", "variable.parameter.ts");
        has(&m, "?", "keyword.operator.optional.ts");
        has(&m, "Promise", "entity.name.type.ts");
        has(&m, "item", "variable.parameter.ts");
        has(&m, "n", "variable.parameter.ts");
        has(
            &m,
            "h",
            "variable.other.constant.ts entity.name.function.ts",
        );
        has(&m, "T", "entity.name.type.ts");
        has(&m, "p", "variable.other.readwrite.ts");
    }

    /// bat's possibly-arrow: a `(` that ends its line after `=` reads a name before `:` as a
    /// parameter, the rest as an expression; the assigned name is the function's.
    #[test]
    fn a_paren_ending_its_line() {
        let m = ts(&[
            "const k = (",
            "  a: number,",
            ") => a;",
            "return (",
            "  b && c",
            ");",
        ]);
        has(
            &m,
            "k",
            "variable.other.constant.ts entity.name.function.ts",
        );
        has(&m, "a", "variable.parameter.ts");
        has(&m, "number", "support.type.primitive.ts");
        has(&m, "b", "variable.other.readwrite.ts");
    }

    /// Classes, interfaces, their heads and members; decorators; parameter properties.
    #[test]
    fn classes_and_interfaces() {
        let m = ts(&[
            "@Component({ selector: 'x' })",
            "export abstract class A<T> extends B<T> implements C, ts.D {",
            "  @Input() name: string;",
            "  private readonly cb: () => void;",
            "  static count = 0;",
            "  constructor(private readonly s: Service, @Inject(X) y: Y) { super(); }",
            "  get size(): number { return 1; }",
            "  run<U>(u: U): U { return u; }",
            "}",
            "interface I extends J { k?: string; m(n: number): void; [key: string]: any }",
        ]);
        has(&m, "Component", "entity.name.function.ts");
        has(&m, "A", "entity.name.type.class.ts");
        has(&m, "B", "entity.other.inherited-class.ts");
        has(&m, "C", "entity.other.inherited-class.ts");
        has(&m, "D", "entity.other.inherited-class.ts");
        has(&m, "name", "variable.object.property.ts");
        has(&m, "cb", "entity.name.function.ts");
        has(&m, "count", "variable.object.property.ts");
        has(&m, "readonly", "storage.modifier.ts");
        has(&m, "s", "variable.parameter.ts");
        has(&m, "Inject", "variable.other.readwrite.ts");
        has(&m, "get", "storage.type.property.ts");
        has(&m, "size", "entity.name.function.ts");
        has(&m, "run", "entity.name.function.ts");
        has(&m, "I", "entity.name.type.interface.ts");
        has(&m, "k", "variable.object.property.ts");
        has(&m, "m", "entity.name.function.ts");
        has(&m, "key", "variable.parameter.ts");
        has(&m, "any", "support.type.primitive.ts");
    }

    /// Type aliases across lines, enums, namespaces, imports and exports.
    #[test]
    fn declarations() {
        let m = ts(&[
            "import { a, type B, c as d } from './x';",
            "export type U =",
            "  | { kind: 'a' }",
            "  | keyof V;",
            "const enum E { One = 1 }",
            "namespace N.M {}",
            "let [p, q] = r, { s, t: w } = z;",
            "let n: number, m: string = '';",
        ]);
        has(&m, "type", "storage.type.type.ts");
        has(&m, "U", "entity.name.type.alias.ts");
        has(&m, "kind", "variable.object.property.ts");
        has(&m, "keyof", "keyword.operator.expression.keyof.ts");
        has(&m, "V", "entity.name.type.ts");
        has(&m, "const", "storage.modifier.ts");
        has(&m, "E", "entity.name.type.enum.ts");
        has(&m, "One", "variable.other.enummember.ts");
        has(&m, "M", "entity.name.type.module.ts");
        has(&m, "d", "variable.other.readwrite.alias.ts");
        has(&m, "[", "punctuation.definition.binding-pattern.array.ts");
        has(&m, "t", "variable.object.property.ts");
        has(&m, "string", "support.type.primitive.ts");
    }

    /// Strings, templates (a substitution's brackets the template's), comments and JSDoc across
    /// lines.
    #[test]
    fn text_across_lines() {
        let m = ts(&[
            "s = 'a\\",
            "b'; t = `x ${f(y)}",
            "w`; /* c",
            "d */ x;",
            "/**",
            " * @param {string} name - the name",
            " * @see {@link Foo}",
            " */",
            "u; // line",
        ]);
        has(&m, "b", "string.quoted.single.ts");
        has(&m, "(", "string.template.ts");
        has(&m, "w", "string.template.ts");
        has(&m, "d ", "comment.block.ts");
        has(&m, "x", "variable.other.readwrite.ts");
        has(
            &m,
            "param",
            "comment.block.documentation.ts storage.type.class.jsdoc",
        );
        has(
            &m,
            "string",
            "comment.block.documentation.ts entity.name.type.instance.jsdoc",
        );
        has(
            &m,
            "name",
            "comment.block.documentation.ts variable.other.jsdoc",
        );
        has(
            &m,
            "Foo",
            "comment.block.documentation.ts entity.name.type.instance.jsdoc variable.other.description.jsdoc",
        );
        has(&m, " line", "comment.line.double-slash.ts");
        has(&m, "u", "variable.other.readwrite.ts");
    }

    /// Generic calls by their type arguments closing before `(`; comparisons stay operators.
    #[test]
    fn generics_and_comparisons() {
        let m = ts(&["const v = config.get<T>(k), lt = a < b && c > d, g = f<string>(x);"]);
        has(&m, "get", "entity.name.function.ts");
        has(&m, "f", "entity.name.function.ts");
        has(&m, "<", "keyword.operator.relational.ts");
        has(&m, "<", "punctuation.definition.typeparameters.begin.ts");
    }

    #[test]
    fn tsx_elements() {
        let m = marked(
            Lang::TypeScriptReact,
            &[
                "const a = <div className=\"x\" on={() => 1}>",
                "  text &amp; {b}",
                "  <Foo.Bar />",
                "</div>;",
            ],
        );
        has(&m, "div", "entity.name.tag.tsx");
        has(&m, "className", "entity.other.attribute-name.tsx");
        has(
            &m,
            "&",
            "constant.character.entity.tsx punctuation.definition.entity.tsx",
        );
        has(&m, "b", "variable.other.readwrite.tsx");
        has(
            &m,
            "Foo.Bar",
            "entity.name.tag.tsx support.class.component.tsx",
        );
    }

    /// Long lines lex in linear time: a line of unclosed generics, groups and templates.
    #[test]
    fn long_lines_stay_linear() {
        for unit in ["a<b(", "(x: ", "`${", "<T,>(", "f<(", "{ a: (", "/*", "@x("] {
            let line = unit.repeat(20_000);
            let t = std::time::Instant::now();
            let _ = marked(Lang::TypeScript, &[&line]);
            let _ = marked(Lang::TypeScriptReact, &[&line]);
            assert!(t.elapsed().as_secs_f64() < 2.0, "{unit:?}");
        }
    }

    /// Spans fuzzing found out of order or without scopes (R2): a substitution's blanks before
    /// a `//` or a regex literal written twice, `Number`'s constants as a span of no kind, a
    /// template literal type's `${` left at a `;` (its text read again), `key: async x =>`'s
    /// `async` written before the line.
    #[test]
    fn spans_where_fuzzing_found_them_wrong() {
        let m = ts(&[
            "const s = `a ${",
            "\t// a comment",
            "  /x/.test(b) / 2",
            "}`;",
            "f(Number.MAX_VALUE, 1);",
            "const o = { install: async resource => 1 };",
            "type A = `x${B;",
        ]);
        assert!(
            m.iter().all(|(_, s)| !s.is_empty()),
            "a span without scopes: {m:?}"
        );
        assert!(!m.iter().any(|(t, _)| *t == "MAX_VALUE"), "{m:?}");
        has(&m, "\t", "punctuation.whitespace.comment.leading.ts");
        has(&m, "  ", "string.regexp.ts");
        has(&m, "install", "entity.name.function.ts");
        has(&m, "async", "storage.modifier.async.ts");
        has(&m, "resource", "variable.parameter.ts");
    }

    /// A `<` that ends its line after `=`, `(`, `,`, `return` or a key's `:` opens a generic
    /// arrow's type parameters, as bat's; after `&&` an operator.
    #[test]
    fn a_less_than_ending_its_line() {
        let m = ts(&[
            "export const eq = <",
            "  T extends Record<string, any>,",
            ">(",
            "  a: T,",
            ") => 1;",
            "x = y && <",
            "z;",
        ]);
        has(
            &m,
            "eq",
            "variable.other.constant.ts entity.name.function.ts",
        );
        has(&m, "<", "punctuation.definition.typeparameters.begin.ts");
        has(&m, "extends", "storage.modifier.ts");
        has(&m, "Record", "entity.name.type.ts");
        has(&m, "a", "variable.parameter.ts");
        has(&m, "<", "keyword.operator.relational.ts");
    }

    /// Values in a type after `typeof` (`import(`'s keyword), a constructor type's `new`,
    /// `import X = require(…)`, a class extending a call's value, `as T[]`.
    #[test]
    fn type_corners_as_bat_reads_them() {
        let m = ts(&[
            "let a: typeof global.fetch | typeof foo.bar.baz;",
            "type E = typeof import('x');",
            "type C = new (a: string) => X;",
            "import React = require('react');",
            "class K extends mock<I>() {}",
            "class M extends mix(A, B) {}",
            "let v = x as string[] | undefined;",
        ]);
        has(&m, "global", "support.variable.object.node.ts");
        has(&m, "fetch", "variable.other.property.ts");
        has(&m, "foo", "variable.other.object.ts");
        has(&m, "bar", "variable.other.object.property.ts");
        has(&m, "baz", "variable.other.property.ts");
        has(&m, "import", "keyword.operator.expression.import.ts");
        has(&m, "new", "keyword.control.new.ts");
        has(&m, "require", "keyword.control.require.ts");
        has(&m, "mock", "entity.name.function.ts");
        has(&m, "mix", "entity.name.function.ts");
        has(&m, "|", "keyword.operator.type.ts");
        has(&m, "undefined", "support.type.builtin.ts");
    }

    /// R3's corners from the census: a type alias going on past a comment line, a group's
    /// `):` lines after its `(` an arrow's return type, type arguments after a call's `)` and
    /// before a bracket the line ends in, `(name:` an arrow's parameters, `X.prototype` a
    /// class, a shorthand key a variable, a possibly-arrow group's ternary, `undefined` and
    /// `as T`, a default import before `{type P as A}`, a type's `(` ending its line, a name
    /// typed `Function` a function's.
    #[test]
    fn corners_bat_reads_across_lines() {
        let m = ts(&[
            "type Union =",
            "  | Before",
            "  // a comment",
            "  | AfterComment;",
            "r.reduce(",
            "  (",
            "    acc: number",
            "  ): X => {",
            "    if (acc) {}",
            "  },",
            ");",
            "const S = styled(Slider)<StyledProps>(() => 1);",
            "const [c, setC] = React.useState<{",
            "}>({});",
            "const Btn = (props: {",
            "}) => 1;",
            "f(Foo.prototype);",
            "const o = { document };",
            "const h = (",
            "  cfg?.v !== undefined",
            "    ? x",
            "    : y",
            ") as RichTextField;",
            "import N, {type Props as Alias} from 'x';",
            "type F = (",
            "  container: U,",
            ") => boolean;",
            "let fnv: Function;",
        ]);
        has(&m, "AfterComment", "entity.name.type.ts");
        has(&m, "if", "keyword.control.conditional.ts");
        has(&m, "StyledProps", "entity.name.type.ts");
        has(&m, "useState", "entity.name.function.ts");
        has(
            &m,
            "Btn",
            "variable.other.constant.ts entity.name.function.ts",
        );
        has(&m, "props", "variable.parameter.ts");
        has(&m, "Foo", "support.class.ts");
        has(&m, "document", "variable.other.readwrite.ts");
        has(&m, "cfg", "variable.other.object.ts");
        has(&m, "undefined", "constant.language.undefined.ts");
        has(&m, ":", "keyword.operator.ternary.ts");
        has(&m, "RichTextField", "entity.name.type.ts");
        has(&m, "N", "variable.other.readwrite.alias.ts");
        has(&m, "Alias", "variable.other.readwrite.alias.ts");
        has(&m, "container", "variable.parameter.ts");
        has(&m, "fnv", "entity.name.function.ts");
    }

    /// A `const`'s destructuring across lines: its names constants on every line (the state
    /// keeps it), a `let`'s variables.
    #[test]
    fn a_const_destructuring_across_lines() {
        let m = ts(&[
            "const {",
            "  filename,",
            "  b",
            "} = x;",
            "let {",
            "  c",
            "} = y;",
        ]);
        has(&m, "filename", "variable.other.constant.ts");
        has(&m, "b", "variable.other.constant.ts");
        has(&m, "c", "variable.other.readwrite.ts");
    }

    /// A block lexes as its lines do one at a time, past the levels a state holds too.
    #[test]
    fn a_block_lexes_as_its_lines() {
        let deep = format!("{}\n{}\n", "f({ a: [".repeat(12), "x, ".repeat(9));
        let text = format!(
            "{deep}const s = 'a\\\nb'; type T =\n  | A\n  | B;\nconst {{\n  fa,\n  fb\n}} = o;\n\
             class C {{\n  x: number\n  y = (\n    a: T,\n  ) => a\n}}\n\
             /** @param {{T}} v\n */ t = `a ${{f(\n1)}} b`;\nlet [p,\n q] = r\n{}",
            "]})".repeat(14)
        );
        // more substitutions open than a state counts
        let text = format!("{text}\nq = {}\n{};", "`${".repeat(17), "}`".repeat(17));
        for lang in [Lang::TypeScript, Lang::TypeScriptReact] {
            let (mut want, mut state, mut spans) = (Vec::new(), State::default(), Vec::new());
            for line in text.split('\n') {
                crate::lex(lang, line, &mut state, &mut spans);
                want.push((line.to_owned(), spans.clone(), state));
            }
            assert_eq!(
                want[1].2.held(),
                crate::stack::HELD,
                "the stack outgrew a state"
            );
            for max in [1, 2, usize::MAX] {
                let (mut got, mut state, mut buf) = (Vec::new(), State::default(), String::new());
                let mut rest = Some(&text[..]);
                while let Some(t) = rest {
                    let next = crate::lex_lines(
                        lang,
                        t,
                        max,
                        &mut state,
                        (&mut spans, &mut buf),
                        |_, _| false,
                        |l, s| got.push((l.to_owned(), s.to_vec())),
                    );
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
                assert_eq!(got, lines, "{lang:?} max {max}");
            }
        }
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
                crate::lex(lang, text, &mut state, &mut spans);
                crate::lex(lang, "...", &mut state, &mut spans);
            }
        }
    }

    /// Objects and JSX nested 24 deep, within the 28 frames a state keeps unpacked: the same
    /// scopes at each depth on the way in, and the top level again at the end.
    #[test]
    fn deep_nesting_comes_back_out() {
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
            crate::lex(lang, start, &mut state, &mut spans);
            let mut ins = Vec::new();
            for _ in 0..24 {
                crate::lex(lang, open, &mut state, &mut spans);
                ins.push(
                    spans
                        .iter()
                        .map(|s| lang.scopes(s.kind))
                        .collect::<Vec<_>>(),
                );
            }
            for _ in 0..24 {
                crate::lex(lang, close, &mut state, &mut spans);
            }
            assert!(ins.windows(2).all(|w| w[0] == w[1]), "{lang:?}");
            crate::lex(
                lang,
                if lang == Lang::TypeScript { "};" } else { ");" },
                &mut state,
                &mut spans,
            );
            assert_eq!(state, State::default(), "{lang:?}");
        }
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
        crate::lex(Lang::TypeScript, "y = /(a/", &mut state, &mut spans);
        crate::lex(Lang::TypeScript, "b)/; z = 1", &mut state, &mut spans);
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
            crate::lex(Lang::TypeScript, line, &mut state, &mut spans);
        }
        assert_eq!(state, State::default());
    }
}
