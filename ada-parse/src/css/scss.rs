//! SCSS on CSS's scanner (ADR-40's phase 6), emitting bat's scopes, with bat's `SCSS` grammar as
//! the reference it is checked against.
//!
//! bat's SCSS (Packages at `759d6ee`, 58 contexts of its own) includes 219 of its `Sass` grammar's,
//! and Sass's grammar is a fork of CSS's: the same word lists (but `{{break}}` for `\b`, and
//! `true`/`false`/`null`), the same functions with Sass's expressions added to each group, numbers
//! that check the sign after them (`maybe-illegal-operator`), and Sass's own selectors, at-rules,
//! variables, maps, interpolation and `//` comments. So SCSS runs on CSS's [`Scan`] with its own
//! contexts and op lists: CSS's readers, word table ([`super::words`], with SCSS's three lists),
//! number reader and the ops whose patterns are the same (an attribute selector's, the counters',
//! a comment's) are shared, and the rest are SCSS's own, tried in the grammar's order at the place
//! the scan has reached, as CSS's are.
//!
//! SCSS has no nesting on the stack (a rule's `{` and `}` are tokens in `main`), so the stack
//! stays shallow. Text takes the wrap of the innermost context that has one: a string's
//! interpolation is the string's (`"#{$x}"`).
//!
//! The Jekyll front matter (`---` lines) embeds YAML: its lines run on ada-parse's YAML scanner,
//! its state kept beside SCSS's. **Not followed:** `(?i)`'s Unicode folding, as in CSS.

use super::{
    ANIMATION_NAME, ARITHMETIC, ATTR_NAME, ATTR_VAL, AT_COUNTER_STYLE,
    AT_CUSTOM_MEDIA, AT_DOCUMENT, AT_IMPORT, AT_KEYFRAME, AT_MEDIA, AT_NAMESPACE, AT_PAGE,
    AT_SUPPORTS, CLASS_DOT, CLASS_NAME, COLOR_SPACE, COMBINATOR, COMMENT, COMMENT_MARK,
    CONST, CONSTANT, COUNTER_STYLE_INVALID, COUNTER_STYLE_NAME, CUSTOM_MARK, CUSTOM_NAME,
    ENTITY_MARK, EXT_COLOR, EXT_COLOR_LEAF, FN_ATTR, FN_CALC, FN_COLOR, FN_COUNTER, FN_DOMAIN,
    FN_FILTER, FN_FONT_FACE, FN_GRADIENT, FN_GRID, FN_IMAGE, FN_REGEXP, FN_SHAPE, FN_TIMING,
    FN_TOGGLE, FN_TRANSFORM, FN_URL, FN_URL_PREFIX, FN_VAR, FONT_NAME_LEAF, GROUP_BEGIN,
    GROUP_END, HEX, HEX_MARK, HEXA, HEXA_MARK, ID_HASH, ID_NAME, IMPORTANT, KEY_VALUE,
    KEYWORD_OTHER, LINE_NAMES, MEDIA_FEATURE, MEDIA_FEATURE_VENDOR, MAIN, NS_PREFIX,
    P_ATTR_NAME, P_ATTR_OP, P_ATTR_SEL_END, P_ATTR_UNQUOTED, P_ATTR_VAL_END,
    P_CALL_END, P_CASE_FLAG, P_COMMA, P_COMMENT, P_COUNTER_NAME, P_COUNTER_STYLES, P_CUSTOM_PROP,
    P_DROP_WORDS, P_FROM_TO, P_GROUP, P_GROUP_END, P_LINE_NAME, P_LINE_NAMES, P_LINE_NAMES_END,
    P_MEDIA_LOGIC, P_MEDIA_TYPES, P_MQ_COMMA, P_NTH_INVALID, P_QUALIFIED, P_STRING,
    P_SUPPORTS_LOGIC, P_SYMBOL_TYPES, P_UNICODE_RANGE, P_VENDOR, PSEUDO_CLASS, PSEUDO_CLASS_MARK,
    PSEUDO_ELEMENT, PSEUDO_ELEMENT_MARK, PSEUDO_ELEMENT_VENDOR, PSEUDO_KEYWORD, PROPERTY_NAME,
    REPETITION, REPETITIONS, RULE_END, SASS_FN, SCSS_PROP, SPECIAL_COLOR, SPECIAL_COLOR_LEAF,
    STD_COLOR, STD_COLOR_LEAF, STR_D, STR_S, STRING_END, Scan, State, TAG,
    TAG_CUSTOM, TAG_NAME, TERMINATOR_LEAF, TIMING_DIRECTION, UNQUOTED, WILDCARD,
    WRAP, ESCAPE, ESCAPE_NEWLINE, F_ADJ, F_ATTR, F_CALC, F_COLOR, F_DOC, F_FILTER, F_IMAGE,
    F_MINMAX, F_REST, F_URL, F_VAR, MAX, Num, Out, Span, T_ANGLE, T_FREQUENCY, T_INTEGER, T_LENGTH,
    T_NUMBER, T_PERCENT, T_RESOLUTION, T_TIME, char_at, is_hex, lex, lookup,
    unit_type,
};

/// Lex `b`, a line of SCSS without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    if in_front_matter(state) && b != b"---" {
        return front_matter_line(b, state, out);
    }
    #[allow(
        clippy::redundant_closure_for_method_calls,
        reason = "the method itself isn't general over the scan's lifetimes"
    )]
    lex::<true>(b, state, out, |s| s.scss_run());
}

/// A kind of the front matter's embedded YAML: YAML's kind with this bit set.
pub(super) const YAML_KIND: u16 = 0x8000;

/// Whether the line starts in the front matter: [`FRONT`] on top of a stack short enough to
/// leave the state's second half free, where YAML's state is kept.
fn in_front_matter(state: &State) -> bool {
    let d = usize::from(state.depth);
    (1..=super::HALF).contains(&d) && state.levels[d - 1] == FRONT
}

/// A line of the front matter: YAML's scanner, its state in the state's second half (`---` on
/// a line of its own ends it, through [`Scan::front`]).
fn front_matter_line(b: &[u8], state: &mut State, out: &mut Out) {
    let mut w = [0u8; 8];
    w.copy_from_slice(&state.withs[..8]);
    let mut y = State::default();
    y.levels[..8].copy_from_slice(&w);
    let mut spans = Vec::new();
    crate::yaml_scan::line(b, &mut y, &mut Out(&mut spans));
    // YAML's spans, the gaps between them `frontmatter.jekyll source.yaml` (YAML's kind 0)
    let mut at = 0;
    for sp in spans {
        out.push(at, sp.start as usize, YAML_KIND);
        out.push(sp.start as usize, sp.end as usize, sp.kind | YAML_KIND);
        at = sp.end as usize;
    }
    out.push(at, b.len(), YAML_KIND);
    state.withs[..8].copy_from_slice(&y.levels[..8]);
}

// ---- the contexts: CSS's ids where an op of CSS's pushes one (strings, comments, an attribute
// selector's value, line names), SCSS's own otherwise ----

/// `//`, to the line's end: its wrap is `comment.line.double-slash.sass`.
pub(super) const LINE: u8 = 63;
/// The Jekyll front matter, between `---` lines.
const FRONT: u8 = 4;
/// After `font`, `font-family` or `family`: its `:` or a pop.
const FONT_PROP: u8 = 5;
/// `font`'s value.
const FONT_VAL: u8 = 7;
/// `property-value-wrapper`: a property's `:`, `;` or a pop.
const PVW: u8 = 8;
/// A property's value, to before a `;`, `}`, `)` or `{`.
const PV: u8 = 9;
/// `sass-maps`' `(` … `)`.
const MAP: u8 = 10;
/// `#{` … `}`.
const INTERP: u8 = 11;
/// `function-arguments`' inner `(` … `)`.
const ARGS_PAREN: u8 = 12;
/// A selector's `selector-parts`.
const SEL: u8 = 14;
/// After `&`.
const AMP: u8 = 15;
/// After `&`'s `[-_]+`.
const AMP_SUF: u8 = 16;
const CLASS: u8 = 17;
const ID: u8 = 18;
/// `%placeholder`.
const PLACEHOLDER: u8 = 19;
/// `(?=\[)`: its `[` and `selector-parts`.
const ATTR_OUT: u8 = 20;
/// `[` … `]`.
const ATTR: u8 = 21;
/// `interpolated-selector-body`.
const ISEL: u8 = 22;
/// Its `{` … `}`.
const ISEL_IN: u8 = 23;
/// `:dir(`, `:lang(`: the call, its `(` or a pop.
const PS_DIR: u8 = 24;
/// `:matches(`, `:is(`, `:where(`, `:not(`, `:has(`.
const PS_SEL: u8 = 25;
const PS_DROP: u8 = 26;
/// `:nth-child(` and the like.
const PS_NTH: u8 = 27;
/// `:-moz-…`: `selector-parts`.
const PS_VENDOR: u8 = 28;
/// `:local(`, `:global(`.
const PS_LOCAL: u8 = 29;
/// `@each`.
const EACH: u8 = 30;
/// `@each`'s `in` …
const EACH_IN: u8 = 31;
/// `@for`, `@while`, `@if`, `@else`: to before a `{`.
const AT_EXPR: u8 = 32;
/// `@warn`, `@error`, `@debug`: to a `;`.
const AT_WARN: u8 = 33;
const EXTEND: u8 = 34;
/// `@mixin`, `@function`.
const MIXIN: u8 = 35;
const MIXIN_PARAMS: u8 = 36;
/// A parameter's default.
const PARAM_VAL: u8 = 37;
/// A `{` in a mixin's parameters: `main`.
const MIXIN_BLOCK: u8 = 38;
const INCLUDE: u8 = 39;
const INCLUDE_ARGS: u8 = 40;
const RETURN: u8 = 41;
/// `@use`, `@forward`.
const USE: u8 = 42;
/// Their `show` and `hide`.
const USE_SHOW: u8 = 43;
/// Their `with (`.
const USE_WITH: u8 = 44;
/// `@charset`, `@namespace`, `@font-face`.
const AT_CSS: u8 = 45;
/// Their `url(` … `)`.
const URL_G: u8 = 46;
const COUNTER_STYLE: u8 = 47;
/// `@custom-media`'s lookahead push, to its `;`.
const CM_OUT: u8 = 48;
const CM: u8 = 49;
const DOCUMENT: u8 = 50;
const IMPORT: u8 = 51;
const KEYFRAMES: u8 = 52;
const MEDIA: u8 = 53;
/// `@namespace` after spaces.
const NAMESPACE: u8 = 54;
const PAGE: u8 = 55;
const SUPPORTS: u8 = 56;
const SUP_PAREN: u8 = 57;
/// `@content`.
const CONTENT: u8 = 58;
/// A media query's `(` … `)`.
const MQ_PAREN: u8 = 59;
/// After a media feature: its `:` or before a `)`.
const MF_AFTER: u8 = 60;
/// `var(`'s rest after a `,`.
const VAR_REST: u8 = 61;
/// `inside-calc-parens`.
const CALC_PARENS: u8 = 62;
/// `:import`, `:export`.
const CMOD: u8 = 128;
const CMOD_G: u8 = 129;
const CMOD_BLOCK: u8 = 130;
const CMOD_VAL: u8 = 131;
const COUNTER_REST: u8 = 132;
const COUNTERS_REST: u8 = 133;
/// `attr(`'s rest after the attribute's name.
const ATTR_FN_REST: u8 = 134;
/// `calc(`'s (and `clamp(`'s, `max(`'s, `min(`'s) function call: its `(` or a pop.
const CALC_W: u8 = 135;
/// The pseudo-classes' groups.
const G_PDIR: u8 = 136;
const G_PSEL: u8 = 137;
const G_PDROP: u8 = 138;
const G_PNTH: u8 = 139;
const G_PLOCAL: u8 = 140;

// A function's group is a context from 64; its call around it (`\)` and `\(`) is the group's id
// with `WRAP` set.
const G_ATTR: u8 = 64;
const G_CROSS_FADE: u8 = 65;
/// `blur(`, `translateZ(`, `perspective(`.
const G_LENGTH: u8 = 66;
/// `brightness(` and the like.
const G_FILTER_AMOUNT: u8 = 67;
const G_DROP_SHADOW: u8 = 68;
/// `hue-rotate(`, `rotate(`, `rotateX(` …, `skewX(`, `skewY(`.
const G_ANGLE: u8 = 69;
/// `conic-gradient(`, `repeating-conic-gradient(`.
const G_CONIC: u8 = 70;
const G_LINEAR: u8 = 71;
const G_RADIAL: u8 = 72;
const G_IMAGE: u8 = 73;
const G_IMAGE_SET: u8 = 74;
const G_MINMAX: u8 = 75;
/// `url(`, `url-prefix(`, `domain(`.
const G_URL: u8 = 76;
const G_VAR: u8 = 77;
const G_ADJ_CHANNEL: u8 = 78;
const G_ADJ_HUE: u8 = 79;
const G_ADJ_PERCENT: u8 = 80;
const G_TINT: u8 = 81;
const G_BLEND: u8 = 82;
const G_FILTER: u8 = 83;
const G_COUNTER: u8 = 84;
const G_COUNTERS: u8 = 85;
const G_SYMBOLS: u8 = 86;
/// `format(`, `regexp(`.
const G_FORMAT: u8 = 87;
const G_LOCAL: u8 = 88;
const G_MATRIX: u8 = 89;
const G_TRANSLATE: u8 = 90;
const G_TRANSLATE_XY: u8 = 91;
const G_SKEW: u8 = 92;
const G_ROTATE3D: u8 = 93;
/// `scaleX(` …, `cubic-bezier(`.
const G_NUMBERS: u8 = 94;
const G_STEPS: u8 = 95;
const G_RECT: u8 = 96;
const G_INSET: u8 = 97;
const G_CIRCLE: u8 = 98;
const G_POLYGON: u8 = 99;
const G_TOGGLE: u8 = 100;
const G_REPEAT: u8 = 101;
const G_RGB: u8 = 102;
const G_HSL: u8 = 103;
const G_GRAY: u8 = 104;
const G_CMYK: u8 = 105;
const G_COLOR: u8 = 106;
/// `function-arguments`: a Sass function's, or a custom one's.
const G_ARGS: u8 = 107;

// ---- the leaves of SCSS's own, from `SCSS_FIRST` ([`LEAVES`]) ----

/// The first of SCSS's leaves: CSS's end before it.
pub(super) const SCSS_FIRST: u16 = super::LEAF_COUNT_CSS;

macro_rules! leaves {
    ($($name:ident = $scopes:literal,)*) => {
        leaves!(@ 0u16; $($name,)*);
        /// SCSS's leaves' scopes, from [`SCSS_FIRST`].
        pub(super) static LEAVES: &[&str] = &[$($scopes,)*];
    };
    (@ $n:expr; $name:ident, $($rest:ident,)*) => {
        #[allow(dead_code, reason = "a mark leaf is its name's plus one")]
        const $name: u16 = SCSS_FIRST + $n;
        leaves!(@ $n + 1; $($rest,)*);
    };
    (@ $n:expr;) => {};
}

leaves! {
    AMP_LEAF = "keyword.operator.ampersand.sass",
    PH_MARK = "entity.other.attribute-name.placeholder.sass punctuation.definition.entity.sass",
    PH_HEAD = "entity.other.attribute-name.placeholder.sass",
    PH_NAME = "entity.other.attribute-name.placeholder.css",
    SEP = "punctuation.separator.css",
    INTERP_HASH = "punctuation.definition.variable.sass",
    BEGIN_SASS = "punctuation.definition.group.begin.sass",
    END_SASS = "punctuation.definition.group.end.sass",
    VAR = "variable.other.sass",
    VAR_MARK = "variable.other.sass punctuation.definition.variable.sass",
    VAR_NS = "variable.other.sass entity.name.namespace.sass",
    VAR_NS_DOT = "variable.other.sass punctuation.separator.namespace.sass",
    VAR_KEY_VALUE = "variable.other.sass punctuation.separator.key-value.css",
    DECL = "variable.declaration.sass",
    DECL_MARK = "variable.declaration.sass punctuation.definition.variable.sass",
    PROP_BARE = "support.type.property-name",
    PROP_BARE_HASH = "support.type.property-name punctuation.definition.variable.sass",
    PROP_BARE_BEGIN = "support.type.property-name punctuation.definition.group.begin.sass",
    PROP_BARE_VAR = "support.type.property-name variable.other.sass",
    PROP_BARE_VAR_MARK = "support.type.property-name variable.other.sass punctuation.definition.variable.sass",
    PROP_BARE_END = "support.type.property-name punctuation.definition.group.end.sass",
    CONST_LANG = "constant.language.sass",
    KW_SASS = "keyword.other.sass",
    INVALID_SASS = "invalid.illegal.sass",
    ARITH_SASS = "keyword.operator.arithmetic.sass",
    OP_SASS = "keyword.operator.sass",
    FN_SASS = "support.function.sass",
    FN_SASS_NS = "support.function.sass entity.name.namespace.sass",
    FN_SASS_NS_DOT = "support.function.sass punctuation.separator.namespace.sass",
    PARAM = "variable.parameter.sass",
    PARAM_MARK = "variable.parameter.sass punctuation.definition.variable.sass",
    NUM_VALUE = "constant.numeric.value.css",
    NUM_SUFFIX = "constant.numeric.suffix.css",
    NUM_DOT = "constant.numeric.value.css punctuation.separator.decimal.css",
    ILLEGAL_OP = "invalid.illegal.operator.css",
    NUM_PLAIN = "constant.numeric.css",
    AT_FLOW = "keyword.control.flow.sass",
    AT_FLOW_MARK = "keyword.control.flow.sass punctuation.definition.keyword.sass",
    AT_COND = "keyword.control.flow.conditional.sass",
    AT_COND_MARK = "keyword.control.flow.conditional.sass punctuation.definition.keyword.sass",
    AT_WARN_LEAF = "keyword.control.at-rule.warn.sass",
    AT_WARN_MARK = "keyword.control.at-rule.warn.sass punctuation.definition.keyword.sass",
    AT_DEBUG = "keyword.control.at-rule.debugger.sass",
    AT_DEBUG_MARK = "keyword.control.at-rule.debugger.sass punctuation.definition.keyword.sass",
    AT_ROOT = "keyword.control.at-rule.at-root.sass",
    AT_ROOT_MARK = "keyword.control.at-rule.at-root.sass punctuation.definition.keyword.sass",
    AT_EXTEND = "keyword.control.at-rule.extend.sass",
    AT_EXTEND_MARK = "keyword.control.at-rule.extend.sass punctuation.definition.keyword.sass",
    AT_SASS = "keyword.control.at-rule.sass",
    AT_SASS_MARK = "keyword.control.at-rule.sass punctuation.definition.keyword.sass",
    AT_RETURN = "keyword.control.at-rule.return.sass",
    AT_RETURN_MARK = "keyword.control.at-rule.return.sass punctuation.definition.keyword.sass",
    AT_MODULE = "keyword.control.at-rule.module.sass",
    AT_MODULE_MARK = "keyword.control.at-rule.module.sass punctuation.definition.keyword.sass",
    AT_CONTENT = "keyword.control.at-rule.content.sass",
    AT_CONTENT_MARK = "keyword.control.at-rule.content.sass punctuation.definition.keyword.sass",
    AT_CSS_LEAF = "keyword.control.at-rule.css",
    AT_CSS_MARK = "keyword.control.at-rule.css punctuation.definition.keyword.css",
    FN_NAME = "entity.name.function.sass",
    INC_AT = "variable.function.sass keyword.control.at-rule.sass",
    INC_AT_MARK = "variable.function.sass keyword.control.at-rule.sass punctuation.definition.keyword.sass",
    INC = "variable.function.sass",
    INC_NS = "variable.function.sass entity.name.namespace.sass",
    INC_NS_DOT = "variable.function.sass punctuation.separator.namespace.sass",
    UNQUOTED_SASS = "string.unquoted.sass",
    KW_AS = "keyword.control.as.sass",
    IMPORT_ALL = "constant.language.import-all.sass",
    KW_SHOW = "keyword.control.show-hide.sass",
    KW_WITH = "keyword.control.with.sass",
    CONST_OTHER_SASS = "constant.other.sass",
    PSEUDO_MODULE = "entity.other.pseudo-class.css-modules.css",
    PSEUDO_MODULE_MARK = "entity.other.pseudo-class.css-modules.css punctuation.definition.entity.css",
    PROP_MODULE = "support.type.property-name.css-modules.css",
    SECTION_BEGIN = "punctuation.section.group.begin.css",
    KW_GRADIENT = "keyword.other.gradient.css",
    PL_BEGIN = "punctuation.section.property-list.begin.css",
    PL_END = "punctuation.section.property-list.end.css",
    PL_BEGIN_SASS = "punctuation.section.property-list.begin.sass",
    PL_END_SASS = "punctuation.section.property-list.end.sass",
    TERM_SASS = "punctuation.terminator.rule.sass",
    FRONT_BEGIN = "frontmatter.jekyll punctuation.section.frontmatter.begin.jekyll",
    FRONT_END = "frontmatter.jekyll punctuation.section.frontmatter.end.jekyll",
    PSEUDO_VENDOR = "entity.other.pseudo-class.css support.type.vendor-prefix.css",
}

// ---- the ops: CSS's below 256 (`super::P_*`, run by `Scan::op`), SCSS's own from 256 ----

/// `/*`, CSS's.
const COMMENT_BLOCK: u16 = P_COMMENT as u16;
/// `//`.
const O_COMMENT_LINE: u16 = 256;
/// `literal-string`'s quotes, CSS's.
const STRING: u16 = P_STRING as u16;
/// `^---$`.
const O_FRONT: u16 = 257;
/// `selectors`.
const O_SELECTORS: u16 = 258;
/// SCSS's `properties`.
const O_PROPERTIES: u16 = 259;
/// `comma-delimiter`, CSS's.
const COMMA: u16 = P_COMMA as u16;
/// `numeric-values`.
const O_NUMERIC: u16 = 260;
/// `interpolated-selectors`.
const O_ISEL: u16 = 261;
/// `custom-element-selectors`.
const O_CUSTOM_EL: u16 = 262;
/// SCSS's `at-rules`.
const O_AT_RULES: u16 = 263;
/// `curly-braces`.
const O_CURLY: u16 = 264;
/// `css-modules`.
const O_CSS_MODULES: u16 = 265;
/// `\b(from|to)\b`, CSS's.
const FROM_TO: u16 = P_FROM_TO as u16;
/// `(\s*)(;)`, a pop.
const O_SEMI_POP: u16 = 266;
/// `(?=\s*[;})])`, a pop.
const O_VALUE_AHEAD: u16 = 267;
/// `\s*(:)` into a value (`:` in `font`'s, with no `\s*`).
const O_COLON_VALUE: u16 = 268;
const O_COLON_FONT: u16 = 269;
/// `''`, a pop.
const O_POP: u16 = 270;
/// `property-values`.
const O_PV: u16 = 271;
/// `(?={)`, a pop.
const O_BRACE_AHEAD: u16 = 272;
/// `sass-script-expression`.
const O_SSE: u16 = 273;
/// `sass-value-expression`.
const O_SVE: u16 = 274;
/// `sass-maps`' `(`.
const O_MAP: u16 = 275;
const O_MAP_END: u16 = 276;
/// `(\$)(…)\s*(:)` in a map.
const O_MAP_VAR_KEY: u16 = 277;
/// `({{ident}})\s*(:)` in a map.
const O_MAP_KEY: u16 = 278;
/// `sass-variables`: a variable, then `sass-interpolation`.
const O_VARIABLES: u16 = 279;
/// `sass-interpolation`.
const O_INTERP: u16 = 280;
/// `(})`, a pop.
const O_INTERP_END: u16 = 281;
const O_SASS_FN: u16 = 282;
const O_CUSTOM_FN: u16 = 283;
/// `sass-operators`.
const O_OPS: u16 = 284;
/// `unquoted-concatenated-string`.
const O_CONCAT: u16 = 285;
/// `unquoted-string`, `\b[^\s'"]+\b`.
const O_UNQUOTED: u16 = 286;
/// `function-arguments`' `\b[^\s'"\(\)]+\b`.
const O_ARG_UNQUOTED: u16 = 287;
/// `builtin-functions`.
const O_BUILTIN: u16 = 288;
const O_COLOR_VALUES: u16 = 289;
const O_CONSTANTS: u16 = 290;
/// `\b(false|true|null)\b`.
const O_TRUE_FALSE: u16 = 291;
const O_VAR_FN: u16 = 292;
const O_CALC_FN: u16 = 293;
const O_ATTR_FN: u16 = 294;
const O_MINMAX_FN: u16 = 295;
const O_URL_FN: u16 = 296;
const O_DOC_FNS: u16 = 297;
const O_ADJ_FNS: u16 = 298;
const O_COLOR_FNS: u16 = 299;
const O_IMAGE_TYPE: u16 = 300;
const O_FILTER_FNS: u16 = 301;
const O_HEX: u16 = 302;
const O_PERCENT: u16 = 303;
const O_LENGTH: u16 = 304;
const O_ANGLE: u16 = 305;
const O_RESOLUTION: u16 = 306;
const O_NUMBER: u16 = 307;
const O_INTEGER: u16 = 308;
/// `color-adjuster-operators`.
const O_ADJ_OPS: u16 = 309;
/// A call's `\(` into its group, CSS's; conic gradients' own.
const GROUP: u16 = P_GROUP as u16;
const O_GROUP_CONIC: u16 = 310;
/// `(?=\))`, CSS's; `\)` as a call's end, CSS's.
const GROUP_AHEAD: u16 = P_GROUP_END as u16;
const CALL_CLOSE: u16 = P_CALL_END as u16;
/// `\)`, the group's end, a pop.
const O_PAREN_END: u16 = 311;
/// `(?=,)` into `var(`'s rest.
const O_VAR_COMMA: u16 = 312;
const O_CUSTOM_PROP: u16 = P_CUSTOM_PROP as u16;
/// `(?=,)` into `counter(`'s and `counters(`'s rest.
const O_COUNTER_COMMA: u16 = 313;
/// `attr(`'s name into its rest; its units.
const O_ATTR_FN_NAME: u16 = 314;
const O_UNITS: u16 = 315;
/// `(https?|ftp)://`.
const O_URL_SCHEME: u16 = 316;
/// `([0-9]+)(x)`.
const O_DENSITY: u16 = 317;
/// The groups' word lists, each `\b(…){{break}}`.
const O_W_AT_FROM: u16 = 318;
const O_W_CONIC_PLACES: u16 = 319;
const O_W_TO: u16 = 320;
const O_W_SIDES: u16 = 321;
const O_W_RADIAL: u16 = 322;
const O_W_RADIAL_PLACES: u16 = 323;
const O_W_SIZES_A: u16 = 324;
const O_W_SIZES_B: u16 = 325;
const O_W_COLOR_SPACES: u16 = 326;
const O_W_STEPS: u16 = 327;
const O_W_AUTO: u16 = 328;
const O_W_ROUND: u16 = 329;
const O_W_AT: u16 = 330;
const O_W_CIRCLE_PLACES: u16 = 331;
const O_W_FILL_RULES: u16 = 332;
const O_W_REPETITIONS: u16 = 333;
const O_W_EVEN_ODD: u16 = 334;
/// `calc(`'s `(`; `inside-calc-parens`' `)` and `(`.
const O_CALC_OPEN: u16 = 335;
const O_CALC_CLOSE: u16 = 336;
const O_CALC_PAREN: u16 = 337;
/// `[-+*/]` in `inside-calc-parens`.
const O_CALC_ARITH: u16 = 402;
/// `function-arguments`' inner `(`, and its `:`.
const O_ARGS_PAREN: u16 = 338;
const O_ARG_COLON: u16 = 339;
/// `selector-parts`.
const O_SP: u16 = 340;
/// `pseudo-elements`, `pseudo-classes`.
const O_PSEUDO_EL: u16 = 341;
const O_PSEUDO_CL: u16 = 342;
/// `{{ident}}` as each selector context's name.
const O_SEL_IDENT: u16 = 343;
/// `&`'s `[-_]+`.
const O_AMP_DASH: u16 = 344;
/// `\[` into an attribute selector.
const O_ATTR_OPEN: u16 = 345;
const QUALIFIED: u16 = P_QUALIFIED as u16;
const ATTR_NAME_OP: u16 = P_ATTR_NAME as u16;
const ATTR_OP: u16 = P_ATTR_OP as u16;
const CASE_FLAG: u16 = P_CASE_FLAG as u16;
const ATTR_END: u16 = P_ATTR_SEL_END as u16;
const ATTR_UNQUOTED: u16 = P_ATTR_UNQUOTED as u16;
const ATTR_VAL_END: u16 = P_ATTR_VAL_END as u16;
/// `({)` into an interpolated selector's body.
const O_ISEL_OPEN: u16 = 346;
/// A pseudo-class's `\(` into its group.
const O_PS_OPEN: u16 = 347;
/// `','` in `:not(` and the like.
const O_PS_COMMA: u16 = 348;
const DROP_WORDS: u16 = P_DROP_WORDS as u16;
/// `(([-+]?)(\d*)(n))\s*(([-+])(\s*\d+))?`.
const O_NTH: u16 = 349;
const NTH_INVALID: u16 = P_NTH_INVALID as u16;
/// `@each`'s `in`.
const O_IN: u16 = 350;
/// `(;)` with `.sass`'s terminator, a pop.
const O_SEMI_SASS: u16 = 351;
/// `\!\s*(default|global|optional)`.
const O_BANG_FLAG: u16 = 352;
/// `\s*(?=[\{;])`, a pop.
const O_BRACE_SEMI_AHEAD: u16 = 353;
/// `\(` into a mixin's parameters, or `@include`'s arguments.
const O_MIXIN_OPEN: u16 = 354;
const O_INCLUDE_OPEN: u16 = 355;
/// `\)` with `.sass`'s scope, a pop.
const O_CLOSE_SASS: u16 = 356;
/// `(\$)(…)` into a parameter's default.
const O_PARAM: u16 = 357;
/// `\{` into a mixin's block; its `\}`.
const O_MIXIN_BLOCK: u16 = 358;
const O_MIXIN_BLOCK_END: u16 = 359;
/// `(?=[,\)])`, a pop.
const O_PARAM_END: u16 = 360;
/// `:` as `punctuation.separator.key-value.css`.
const O_KEY_VALUE: u16 = 361;
/// `\s*(;)`, its `;` `.sass`'s or `.css`'s terminator, a pop.
const O_SEMI_INC: u16 = 362;
const O_SEMI_RET: u16 = 363;
/// `{{ident}}` as `string.unquoted.sass`.
const O_IDENT_SASS: u16 = 364;
/// `@use`'s ops: its `;`, `as`, `show`/`hide`, `with (`; `show`'s `(?=;)`; a plain name.
const O_USE_SEMI: u16 = 365;
const O_USE_AS: u16 = 366;
const O_USE_SHOW: u16 = 367;
const O_USE_WITH: u16 = 368;
const O_SEMI_AHEAD: u16 = 369;
const O_IDENT_PLAIN: u16 = 370;
/// `(url)(\()` in `@charset` and the like.
const O_URL_OPEN: u16 = 371;
/// `(?=\{)`, a pop.
const O_CURLY_AHEAD: u16 = 372;
/// `;` as `punctuation.terminator.css`, a pop; `\s*((@)custom-media)` into it; `\s*(?=;)`.
const O_CM_END: u16 = 373;
const O_CM_KEYWORD: u16 = 374;
const O_SPACES_SEMI_AHEAD: u16 = 375;
/// `media-query`.
const O_MEDIA_QUERY: u16 = 376;
/// `(?=[\{;])`, a pop.
const O_BRACE_SEMI_NOW: u16 = 377;
/// `;` as `.sass`'s terminator, a pop.
const O_SEMI_SASS_POP: u16 = 378;
/// `(\()(reference|…)(\))`.
const O_IMPORT_ARGS: u16 = 379;
/// `@keyframes`' name and `,`.
const O_KF_NAME: u16 = 380;
const O_KF_COMMA: u16 = 381;
/// `;` as `.css`'s rule terminator; `(?=;|$)`, a pop.
const O_SEMI_RULE: u16 = 382;
const O_SEMI_EOL_AHEAD: u16 = 383;
/// `\s*(?=\{)`, a pop.
const O_SPACES_CURLY_AHEAD: u16 = 384;
const SUPPORTS_LOGIC: u16 = P_SUPPORTS_LOGIC as u16;
/// `\(` into `@supports`' parentheses.
const O_SUP_OPEN: u16 = 385;
/// `(?=$)`, a pop.
const O_EOL_AHEAD: u16 = 386;
const MEDIA_TYPES: u16 = P_MEDIA_TYPES as u16;
const MEDIA_LOGIC: u16 = P_MEDIA_LOGIC as u16;
const MQ_COMMA: u16 = P_MQ_COMMA as u16;
/// `\(` into a media query's parentheses; the media features; `(:)|(?=\))`.
const O_MQ_OPEN: u16 = 387;
const O_MEDIA_FEATURE: u16 = 388;
const O_MF_END: u16 = 389;
/// `\b(portrait|landscape|progressive|interlace)`.
const O_ORIENTATION: u16 = 390;
/// `\s*(\d+)(/)(\d+)`.
const O_RATIO: u16 = 391;
/// `css-modules`' ops.
const O_CMOD_OPEN: u16 = 392;
const O_CMOD_BLOCK: u16 = 393;
const O_CMOD_BLOCK_END: u16 = 394;
const O_CMOD_KEY: u16 = 395;
const O_CMOD_VAL_END: u16 = 396;
/// `\!\s*important{{break}}`; `;` as `invalid.illegal.sass`; `/`.
const O_IMPORTANT: u16 = 397;
const O_SEMI_INVALID: u16 = 398;
const O_SLASH: u16 = 399;
/// `{{ident}}` as `string.unquoted.css`.
const O_IDENT_UNQUOTED: u16 = 400;
/// `font`'s value's names: `{{ident}}(\s+{{ident}})*\b(?!:)`.
const O_FONT_IDENTS: u16 = 401;
const VENDOR: u16 = P_VENDOR as u16;
const LINE_NAMES_OPEN: u16 = P_LINE_NAMES as u16;
const LINE_NAME: u16 = P_LINE_NAME as u16;
const LINE_NAMES_CLOSE: u16 = P_LINE_NAMES_END as u16;
const UNICODE_RANGE: u16 = P_UNICODE_RANGE as u16;
const COUNTER_NAME_OP: u16 = P_COUNTER_NAME as u16;
const COUNTER_STYLES: u16 = P_COUNTER_STYLES as u16;
const SYMBOL_TYPES: u16 = P_SYMBOL_TYPES as u16;

/// `sass-script-expression`.
const SSE: &[u16] = &[
    O_COMMENT_LINE,
    COMMENT_BLOCK,
    COMMA,
    O_MAP,
    O_COLOR_VALUES,
    O_NUMERIC,
    O_VARIABLES,
    O_SASS_FN,
    O_CUSTOM_FN,
    O_OPS,
    O_INTERP,
    O_TRUE_FALSE,
    STRING,
    O_UNQUOTED,
];

/// `sass-value-expression`.
const SVE: &[u16] = &[
    O_COMMENT_LINE,
    COMMENT_BLOCK,
    COMMA,
    O_VAR_FN,
    O_VARIABLES,
    O_SASS_FN,
    O_CUSTOM_FN,
    O_INTERP,
    O_OPS,
];

/// `property-values`.
const PROPERTY_VALUES: &[u16] = &[
    O_SEMI_INVALID,
    O_IMPORTANT,
    O_BANG_FLAG,
    O_SLASH,
    COMMA,
    VENDOR,
    O_COMMENT_LINE,
    COMMENT_BLOCK,
    O_BUILTIN,
    LINE_NAMES_OPEN,
    UNICODE_RANGE,
    O_NUMERIC,
    O_COLOR_VALUES,
    O_CONSTANTS,
    STRING,
    O_MAP,
    O_VARIABLES,
    O_SASS_FN,
    O_CUSTOM_FN,
    O_INTERP,
    O_CONCAT,
    O_OPS,
    O_IDENT_UNQUOTED,
];

/// `selector-parts`.
const SELECTOR_PARTS: &[u16] = &[
    1000, // `\s*(,)\s*`, a pop
    1001, // `\s*({{combinators}})(?![>~+])\s*`, a pop
    1002, // `\s*(?=\{)`, a pop
    1003, // `(?=[.#\[&])`, a pop
    GROUP_AHEAD,
    COMMENT_BLOCK,
    O_PSEUDO_EL,
    O_PSEUDO_CL,
    1004, // `\s*(?=$|//|/\*)`, a pop
    1005, // `\s*(?=\b)`, a pop
];

/// `media-query` (SCSS's).
const MEDIA_QUERY: &[u16] = &[
    COMMENT_BLOCK,
    O_COMMENT_LINE,
    MEDIA_TYPES,
    MEDIA_LOGIC,
    MQ_COMMA,
    O_MQ_OPEN,
];

/// `main`'s, which a mixin's block also includes.
const MAIN_OPS: &[u16] = &[
    O_FRONT,
    COMMENT_BLOCK,
    O_COMMENT_LINE,
    O_SELECTORS,
    O_PROPERTIES,
    FROM_TO,
    COMMA,
    O_NUMERIC,
    O_ISEL,
    O_CUSTOM_EL,
    O_AT_RULES,
    O_CURLY,
    COMMA,
    O_CSS_MODULES,
];

/// A mixin's block: its `\}`, then `main`'s.
const MIXIN_BLOCK_OPS: &[u16] = &[
    O_MIXIN_BLOCK_END,
    O_FRONT,
    COMMENT_BLOCK,
    O_COMMENT_LINE,
    O_SELECTORS,
    O_PROPERTIES,
    FROM_TO,
    COMMA,
    O_NUMERIC,
    O_ISEL,
    O_CUSTOM_EL,
    O_AT_RULES,
    O_CURLY,
    COMMA,
    O_CSS_MODULES,
];

/// Each context's ops in the grammar's order (an include as one op that tries its list).
fn ops(ctx: u8) -> &'static [u16] {
    match ctx {
        MAIN => MAIN_OPS,
        MIXIN_BLOCK => MIXIN_BLOCK_OPS,
        FONT_PROP => &[O_SEMI_POP, O_VALUE_AHEAD, O_COLON_FONT, O_POP],
        FONT_VAL => &[
            O_VALUE_AHEAD,
            O_COMMENT_LINE,
            COMMENT_BLOCK,
            O_NUMERIC,
            O_CONSTANTS,
            O_VAR_FN,
            O_MAP,
            O_VARIABLES,
            O_SASS_FN,
            O_CUSTOM_FN,
            O_FONT_IDENTS,
            STRING,
            O_CONCAT,
            O_OPS,
            COMMA,
            O_BRACE_AHEAD,
        ],
        PVW => &[O_SEMI_POP, O_VALUE_AHEAD, O_COLON_VALUE, O_POP],
        PV => &[O_VALUE_AHEAD, O_PV, O_BRACE_AHEAD],
        MAP => &[O_MAP_END, O_MAP_VAR_KEY, O_MAP_KEY, O_PV],
        INTERP | ISEL_IN => &[O_INTERP_END, O_SSE],
        ARGS_PAREN => &[O_PAREN_END, O_SSE],
        SEL | ATTR_OUT | PS_VENDOR => match ctx {
            ATTR_OUT => &[O_ATTR_OPEN, O_SP],
            _ => &[O_SP],
        },
        AMP => &[O_AMP_DASH, O_SP],
        AMP_SUF | CLASS | ID | PLACEHOLDER => &[O_SEL_IDENT, O_INTERP, O_SP],
        ATTR => &[ATTR_END, QUALIFIED, ATTR_NAME_OP, ATTR_OP, CASE_FLAG],
        ATTR_VAL => &[ATTR_UNQUOTED, STRING, ATTR_VAL_END],
        ISEL => &[O_ISEL_OPEN, O_SP],
        PS_DIR | PS_SEL | PS_DROP | PS_NTH | PS_LOCAL => &[O_PS_OPEN, O_POP],
        G_PDIR => &[O_PAREN_END, O_VAR_FN, O_UNQUOTED],
        G_PSEL => &[O_PAREN_END, O_PS_COMMA, O_SELECTORS, O_CUSTOM_EL],
        G_PDROP => &[O_PAREN_END, DROP_WORDS, O_VAR_FN],
        G_PNTH => &[O_PAREN_END, O_W_EVEN_ODD, O_NTH, NTH_INVALID, O_VAR_FN, O_INTEGER],
        G_PLOCAL => &[O_PAREN_END, O_SELECTORS, STRING],
        LINE_NAMES => &[LINE_NAME, LINE_NAMES_CLOSE],
        EACH => &[O_VARIABLES, COMMA, O_BRACE_AHEAD, O_IN],
        EACH_IN => &[
            O_BRACE_AHEAD,
            COMMENT_BLOCK,
            O_COMMENT_LINE,
            COMMA,
            O_VAR_FN,
            O_VARIABLES,
            O_SASS_FN,
            O_CUSTOM_FN,
            O_INTERP,
            O_MAP,
            O_UNQUOTED,
            O_OPS,
        ],
        AT_EXPR => &[O_BRACE_AHEAD, O_SSE],
        AT_WARN => &[O_SEMI_SASS, O_SSE],
        EXTEND => &[O_SEMI_SASS, O_BANG_FLAG, O_SELECTORS, O_CUSTOM_EL],
        MIXIN => &[O_BRACE_SEMI_AHEAD, O_MIXIN_OPEN],
        MIXIN_PARAMS => &[O_CLOSE_SASS, COMMA, O_PARAM, O_MIXIN_BLOCK],
        PARAM_VAL => &[O_PARAM_END, O_KEY_VALUE, O_TRUE_FALSE, O_PV],
        INCLUDE => &[O_SEMI_INC, O_BRACE_SEMI_AHEAD, O_INCLUDE_OPEN],
        INCLUDE_ARGS => &[
            COMMENT_BLOCK,
            O_COMMENT_LINE,
            O_VARIABLES,
            O_SASS_FN,
            O_OPS,
            COMMA,
            O_PV,
            STRING,
            O_IDENT_SASS,
            O_SELECTORS,
            O_CUSTOM_EL,
            O_CLOSE_SASS,
        ],
        RETURN => &[O_SEMI_RET, O_PV],
        USE => &[
            O_USE_SEMI,
            O_USE_AS,
            O_USE_SHOW,
            O_USE_WITH,
            COMMENT_BLOCK,
            O_COMMENT_LINE,
            STRING,
        ],
        USE_SHOW => &[
            O_SEMI_AHEAD,
            COMMENT_BLOCK,
            O_COMMENT_LINE,
            O_VARIABLES,
            O_SASS_FN,
            O_OPS,
            COMMA,
            O_IDENT_PLAIN,
        ],
        USE_WITH => &[
            O_CLOSE_SASS,
            O_KEY_VALUE,
            COMMENT_BLOCK,
            O_COMMENT_LINE,
            O_VARIABLES,
            O_SASS_FN,
            O_OPS,
            COMMA,
            O_PV,
        ],
        AT_CSS => &[
            O_SEMI_INC,
            O_BRACE_SEMI_AHEAD,
            O_URL_OPEN,
            COMMENT_BLOCK,
            O_COMMENT_LINE,
            STRING,
        ],
        URL_G => &[O_PAREN_END, STRING, O_UNQUOTED],
        COUNTER_STYLE => &[COMMENT_BLOCK, O_COMMENT_LINE, O_CURLY_AHEAD],
        CM_OUT => &[O_CM_END, O_CM_KEYWORD],
        CM => &[O_SPACES_SEMI_AHEAD, O_MEDIA_QUERY],
        DOCUMENT => &[
            O_BRACE_SEMI_NOW,
            COMMENT_BLOCK,
            O_COMMENT_LINE,
            O_URL_FN,
            O_DOC_FNS,
            COMMA,
        ],
        IMPORT => &[
            O_SEMI_SASS_POP,
            O_IMPORT_ARGS,
            COMMENT_BLOCK,
            O_COMMENT_LINE,
            STRING,
            O_URL_FN,
            O_MEDIA_QUERY,
        ],
        KEYFRAMES => &[
            O_BRACE_SEMI_AHEAD,
            COMMENT_BLOCK,
            O_COMMENT_LINE,
            O_INTERP,
            O_KF_NAME,
            O_KF_COMMA,
        ],
        MEDIA => &[O_MEDIA_QUERY, O_CURLY_AHEAD],
        NAMESPACE => &[
            O_SEMI_RULE,
            O_SEMI_EOL_AHEAD,
            STRING,
            O_URL_FN,
            COMMENT_BLOCK,
            O_COMMENT_LINE,
        ],
        PAGE => &[COMMENT_BLOCK, O_COMMENT_LINE, O_SPACES_CURLY_AHEAD],
        SUPPORTS => &[O_CURLY_AHEAD, SUPPORTS_LOGIC, O_SUP_OPEN],
        SUP_PAREN => &[
            O_PAREN_END,
            SUPPORTS_LOGIC,
            O_SUP_OPEN,
            COMMENT_BLOCK,
            O_COMMENT_LINE,
            O_PROPERTIES,
        ],
        CONTENT => &[O_SEMI_SASS_POP, O_EOL_AHEAD],
        MQ_PAREN => &[
            O_PAREN_END,
            COMMENT_BLOCK,
            O_COMMENT_LINE,
            O_MEDIA_FEATURE,
            O_ORIENTATION,
            O_RATIO,
            O_NUMERIC,
            O_SVE,
        ],
        MF_AFTER => &[O_MF_END],
        CMOD => &[O_CMOD_OPEN, O_CMOD_BLOCK, O_POP],
        CMOD_G => &[O_PAREN_END, O_SELECTORS, STRING],
        CMOD_BLOCK => &[O_CMOD_BLOCK_END, O_COMMENT_LINE, COMMENT_BLOCK, O_CMOD_KEY],
        CMOD_VAL => &[O_CMOD_VAL_END, STRING, O_UNQUOTED],
        VAR_REST => &[GROUP_AHEAD, O_PV],
        COUNTER_REST => &[GROUP_AHEAD, COMMA, O_VAR_FN, COUNTER_STYLES, O_SVE],
        COUNTERS_REST => &[GROUP_AHEAD, COMMA, O_VAR_FN, STRING, COUNTER_STYLES, O_SVE],
        ATTR_FN_REST => &[
            O_UNITS,
            GROUP_AHEAD,
            COMMA,
            O_VAR_FN,
            O_CONSTANTS,
            O_NUMERIC,
            O_COLOR_VALUES,
            O_SVE,
        ],
        CALC_W => &[O_CALC_OPEN, O_POP],
        CALC_PARENS => &[
            O_CALC_CLOSE,
            COMMENT_BLOCK,
            O_ATTR_FN,
            O_CALC_FN,
            O_VAR_FN,
            O_NUMERIC,
            O_CALC_ARITH,
            O_SVE,
            O_CALC_PAREN,
        ],
        G_ATTR => &[GROUP_AHEAD, O_VAR_FN, QUALIFIED, STRING, O_SVE, O_ATTR_FN_NAME],
        G_CROSS_FADE => &[
            GROUP_AHEAD,
            COMMA,
            O_VAR_FN,
            O_COLOR_VALUES,
            O_IMAGE_TYPE,
            O_PERCENT,
            STRING,
            O_SVE,
            O_UNQUOTED,
        ],
        G_LENGTH => &[GROUP_AHEAD, O_VAR_FN, O_CALC_FN, O_LENGTH, O_SVE],
        G_FILTER_AMOUNT => &[
            GROUP_AHEAD,
            O_VAR_FN,
            O_CALC_FN,
            O_PERCENT,
            O_NUMBER,
            O_SVE,
        ],
        G_DROP_SHADOW => &[
            GROUP_AHEAD,
            O_VAR_FN,
            O_CALC_FN,
            O_LENGTH,
            O_COLOR_VALUES,
            O_SVE,
        ],
        G_ANGLE => &[GROUP_AHEAD, O_VAR_FN, O_CALC_FN, O_ANGLE, O_SVE],
        G_CONIC => &[
            GROUP_AHEAD,
            COMMA,
            O_W_AT_FROM,
            O_W_CONIC_PLACES,
            O_COLOR_VALUES,
            O_ANGLE,
            O_LENGTH,
            O_PERCENT,
            O_SVE,
        ],
        G_LINEAR => &[
            GROUP_AHEAD,
            COMMA,
            O_VAR_FN,
            O_COLOR_VALUES,
            O_ANGLE,
            O_LENGTH,
            O_PERCENT,
            O_W_TO,
            O_W_SIDES,
            O_SVE,
        ],
        G_RADIAL => &[
            GROUP_AHEAD,
            COMMA,
            O_VAR_FN,
            O_COLOR_VALUES,
            O_ANGLE,
            O_LENGTH,
            O_PERCENT,
            O_W_RADIAL,
            O_W_RADIAL_PLACES,
            O_SVE,
        ],
        G_IMAGE => &[
            GROUP_AHEAD,
            COMMA,
            O_VAR_FN,
            O_COLOR_VALUES,
            O_IMAGE_TYPE,
            STRING,
            O_SVE,
            O_UNQUOTED,
        ],
        G_IMAGE_SET => &[
            GROUP_AHEAD,
            COMMA,
            O_VAR_FN,
            O_COLOR_VALUES,
            O_IMAGE_TYPE,
            O_RESOLUTION,
            O_DENSITY,
            O_SVE,
            STRING,
            O_UNQUOTED,
        ],
        G_MINMAX => &[
            GROUP_AHEAD,
            COMMA,
            O_CALC_FN,
            O_VAR_FN,
            O_PERCENT,
            O_LENGTH,
            O_W_SIZES_A,
            O_SVE,
        ],
        G_URL => &[
            GROUP_AHEAD,
            O_VAR_FN,
            STRING,
            O_URL_SCHEME,
            O_SVE,
            O_UNQUOTED,
        ],
        G_VAR => &[GROUP_AHEAD, O_VAR_FN, O_CUSTOM_PROP, O_VAR_COMMA, O_SVE],
        G_ADJ_CHANNEL => &[
            GROUP_AHEAD,
            O_VAR_FN,
            O_CALC_FN,
            O_ADJ_OPS,
            O_PERCENT,
            O_NUMBER,
            O_SVE,
        ],
        G_ADJ_HUE => &[GROUP_AHEAD, O_VAR_FN, O_CALC_FN, O_ADJ_OPS, O_ANGLE, O_SVE],
        G_ADJ_PERCENT => &[
            GROUP_AHEAD,
            O_VAR_FN,
            O_CALC_FN,
            O_ADJ_OPS,
            O_PERCENT,
            O_SVE,
        ],
        G_TINT => &[GROUP_AHEAD, O_VAR_FN, O_CALC_FN, O_PERCENT, O_SVE],
        G_BLEND => &[
            GROUP_AHEAD,
            O_VAR_FN,
            O_CALC_FN,
            O_COLOR_VALUES,
            O_PERCENT,
            O_W_COLOR_SPACES,
            O_SVE,
        ],
        G_FILTER => &[
            GROUP_AHEAD,
            COMMA,
            O_VAR_FN,
            O_IMAGE_TYPE,
            STRING,
            O_FILTER_FNS,
            O_SVE,
        ],
        G_COUNTER | G_COUNTERS => &[GROUP_AHEAD, COUNTER_NAME_OP, O_COUNTER_COMMA, O_VAR_FN],
        G_SYMBOLS => &[
            GROUP_AHEAD,
            SYMBOL_TYPES,
            COMMA,
            O_VAR_FN,
            STRING,
            O_IMAGE_TYPE,
            O_SVE,
        ],
        G_FORMAT => &[GROUP_AHEAD, O_VAR_FN, STRING, O_SVE],
        G_LOCAL => &[GROUP_AHEAD, O_VAR_FN, O_SVE, STRING, O_UNQUOTED],
        G_MATRIX => &[GROUP_AHEAD, COMMA, O_CALC_FN, O_NUMBER, O_VAR_FN, O_SVE],
        G_TRANSLATE => &[
            GROUP_AHEAD,
            COMMA,
            O_VAR_FN,
            O_CALC_FN,
            O_PERCENT,
            O_LENGTH,
            O_NUMBER,
            O_SVE,
        ],
        G_TRANSLATE_XY => &[
            GROUP_AHEAD,
            O_VAR_FN,
            O_CALC_FN,
            O_PERCENT,
            O_LENGTH,
            O_NUMBER,
            O_SVE,
        ],
        G_SKEW => &[GROUP_AHEAD, COMMA, O_VAR_FN, O_CALC_FN, O_ANGLE, O_SVE],
        G_ROTATE3D => &[
            GROUP_AHEAD,
            COMMA,
            O_VAR_FN,
            O_CALC_FN,
            O_ANGLE,
            O_NUMBER,
            O_SVE,
        ],
        G_NUMBERS => &[GROUP_AHEAD, COMMA, O_VAR_FN, O_CALC_FN, O_NUMBER, O_SVE],
        G_STEPS => &[
            GROUP_AHEAD,
            COMMA,
            O_VAR_FN,
            O_CALC_FN,
            O_INTEGER,
            O_W_STEPS,
            O_SVE,
        ],
        G_RECT => &[GROUP_AHEAD, O_W_AUTO, O_VAR_FN, O_CALC_FN, O_LENGTH, O_SVE],
        G_INSET => &[
            GROUP_AHEAD,
            O_W_ROUND,
            O_VAR_FN,
            O_CALC_FN,
            O_LENGTH,
            O_PERCENT,
            O_SVE,
        ],
        G_CIRCLE => &[
            GROUP_AHEAD,
            O_W_AT,
            O_W_CIRCLE_PLACES,
            O_VAR_FN,
            O_CALC_FN,
            O_LENGTH,
            O_PERCENT,
            O_SVE,
        ],
        G_POLYGON => &[
            COMMENT_BLOCK,
            GROUP_AHEAD,
            COMMA,
            O_VAR_FN,
            O_LENGTH,
            O_PERCENT,
            O_CALC_FN,
            O_W_FILL_RULES,
            O_SVE,
        ],
        G_TOGGLE => &[
            GROUP_AHEAD,
            COMMA,
            O_VAR_FN,
            O_CALC_FN,
            VENDOR,
            O_CONSTANTS,
            O_NUMERIC,
            O_COLOR_VALUES,
            STRING,
            O_SVE,
        ],
        G_REPEAT => &[
            GROUP_AHEAD,
            COMMA,
            O_CALC_FN,
            O_VAR_FN,
            O_LENGTH,
            O_PERCENT,
            O_MINMAX_FN,
            O_INTEGER,
            LINE_NAMES_OPEN,
            O_W_REPETITIONS,
            O_W_SIZES_B,
            O_SVE,
        ],
        G_RGB => &[
            GROUP_AHEAD,
            COMMA,
            O_CALC_FN,
            O_VAR_FN,
            O_PERCENT,
            O_NUMBER,
            O_COLOR_VALUES,
            O_SVE,
        ],
        G_HSL => &[
            GROUP_AHEAD,
            COMMA,
            O_CALC_FN,
            O_VAR_FN,
            O_ANGLE,
            O_PERCENT,
            O_NUMBER,
            O_SVE,
        ],
        G_GRAY => &[
            GROUP_AHEAD,
            COMMA,
            O_CALC_FN,
            O_VAR_FN,
            O_PERCENT,
            O_NUMBER,
            O_SVE,
        ],
        G_CMYK => &[
            GROUP_AHEAD,
            COMMA,
            O_CALC_FN,
            O_ADJ_FNS,
            O_VAR_FN,
            O_COLOR_VALUES,
            O_PERCENT,
            O_NUMBER,
            O_SVE,
        ],
        G_COLOR => &[
            GROUP_AHEAD,
            COMMA,
            O_CALC_FN,
            O_ADJ_FNS,
            O_VAR_FN,
            O_COLOR_VALUES,
            O_ANGLE,
            O_NUMBER,
            O_SVE,
        ],
        G_ARGS => &[
            GROUP_AHEAD,
            O_ARGS_PAREN,
            COMMENT_BLOCK,
            O_COMMENT_LINE,
            COMMA,
            O_ARG_COLON,
            O_VAR_FN,
            O_MINMAX_FN,
            O_ADJ_OPS,
            O_ADJ_FNS,
            O_COLOR_FNS,
            O_HEX,
            O_NUMERIC,
            O_INTEGER,
            O_IMAGE_TYPE,
            STRING,
            O_VARIABLES,
            O_SASS_FN,
            O_CUSTOM_FN,
            O_INTERP,
            O_OPS,
            O_ARG_UNQUOTED,
        ],
        _ if ctx & WRAP != 0 && (64..=G_ARGS).contains(&(ctx & !WRAP)) => {
            if ctx & !WRAP == G_CONIC {
                &[CALL_CLOSE, O_GROUP_CONIC]
            } else {
                &[CALL_CLOSE, GROUP]
            }
        }
        _ => &[],
    }
}

// ---- the functions ----

/// The function named `w` before a `(`: its family, its name's leaf, its group (`CALC_W` for
/// `calc` and `clamp`, `max`, `min`).
fn function(w: &[u8]) -> Option<(u16, u16, u8)> {
    if w.len() <= 5
        && (w.eq_ignore_ascii_case(b"clamp")
            || w.eq_ignore_ascii_case(b"max")
            || w.eq_ignore_ascii_case(b"min"))
    {
        return Some((F_CALC, FN_CALC, CALC_W));
    }
    Some(match w {
        b"attr" => (F_ATTR, FN_ATTR, G_ATTR),
        b"calc" => (F_CALC, FN_CALC, CALC_W),
        b"cross-fade" => (F_IMAGE, FN_IMAGE, G_CROSS_FADE),
        b"blur" => (F_FILTER, FN_FILTER, G_LENGTH),
        b"brightness" | b"contrast" | b"grayscale" | b"invert" | b"opacity" | b"saturate"
        | b"sepia" => (F_FILTER, FN_FILTER, G_FILTER_AMOUNT),
        b"drop-shadow" => (F_FILTER, FN_FILTER, G_DROP_SHADOW),
        b"hue-rotate" => (F_FILTER, FN_FILTER, G_ANGLE),
        b"conic-gradient" | b"repeating-conic-gradient" => (F_IMAGE, FN_GRADIENT, G_CONIC),
        b"linear-gradient" | b"repeating-linear-gradient" => (F_IMAGE, FN_GRADIENT, G_LINEAR),
        b"radial-gradient" | b"repeating-radial-gradient" => (F_IMAGE, FN_GRADIENT, G_RADIAL),
        b"image" => (F_IMAGE, FN_IMAGE, G_IMAGE),
        b"image-set" => (F_IMAGE, FN_IMAGE, G_IMAGE_SET),
        b"minmax" => (F_MINMAX, FN_GRID, G_MINMAX),
        b"url" => (F_URL, FN_URL, G_URL),
        b"var" => (F_VAR, FN_VAR, G_VAR),
        b"red" | b"green" | b"blue" | b"alpha" | b"a" => (F_ADJ, FN_COLOR, G_ADJ_CHANNEL),
        b"hue" | b"h" => (F_ADJ, FN_COLOR, G_ADJ_HUE),
        b"saturation" | b"lightness" | b"whiteness" | b"blackness" | b"s" | b"l" | b"w"
        | b"b" => (F_ADJ, FN_COLOR, G_ADJ_PERCENT),
        b"tint" | b"shade" => (F_ADJ, FN_COLOR, G_TINT),
        b"blenda" | b"blend" => (F_ADJ, FN_COLOR, G_BLEND),
        b"filter" => (F_REST, FN_FILTER, G_FILTER),
        b"counter" => (F_REST, FN_COUNTER, G_COUNTER),
        b"counters" => (F_REST, FN_COUNTER, G_COUNTERS),
        b"symbols" => (F_REST, FN_COUNTER, G_SYMBOLS),
        b"format" => (F_REST, FN_FONT_FACE, G_FORMAT),
        b"local" => (F_REST, FN_FONT_FACE, G_LOCAL),
        b"matrix3d" | b"scale3d" | b"matrix" | b"scale" => (F_REST, FN_TRANSFORM, G_MATRIX),
        b"translate" | b"translate3d" => (F_REST, FN_TRANSFORM, G_TRANSLATE),
        b"translateX" | b"translateY" => (F_REST, FN_TRANSFORM, G_TRANSLATE_XY),
        b"rotate" | b"rotateX" | b"rotateY" | b"rotateZ" | b"skewX" | b"skewY" => {
            (F_REST, FN_TRANSFORM, G_ANGLE)
        }
        b"skew" => (F_REST, FN_TRANSFORM, G_SKEW),
        b"translateZ" | b"perspective" => (F_REST, FN_TRANSFORM, G_LENGTH),
        b"rotate3d" => (F_REST, FN_TRANSFORM, G_ROTATE3D),
        b"scaleX" | b"scaleY" | b"scaleZ" => (F_REST, FN_TRANSFORM, G_NUMBERS),
        b"cubic-bezier" => (F_REST, FN_TIMING, G_NUMBERS),
        b"steps" => (F_REST, FN_TIMING, G_STEPS),
        b"rect" => (F_REST, FN_SHAPE, G_RECT),
        b"inset" => (F_REST, FN_SHAPE, G_INSET),
        b"circle" | b"ellipse" => (F_REST, FN_SHAPE, G_CIRCLE),
        b"polygon" => (F_REST, FN_SHAPE, G_POLYGON),
        b"toggle" => (F_REST, FN_TOGGLE, G_TOGGLE),
        b"repeat" => (F_REST, FN_GRID, G_REPEAT),
        b"rgb" | b"rgba" => (F_COLOR, FN_COLOR, G_RGB),
        b"hsl" | b"hsla" | b"hwb" => (F_COLOR, FN_COLOR, G_HSL),
        b"gray" => (F_COLOR, FN_COLOR, G_GRAY),
        b"device-cmyk" => (F_COLOR, FN_COLOR, G_CMYK),
        b"color" => (F_COLOR, FN_COLOR, G_COLOR),
        b"url-prefix" => (F_DOC, FN_URL_PREFIX, G_URL),
        b"domain" => (F_DOC, FN_DOMAIN, G_URL),
        b"regexp" => (F_DOC, FN_REGEXP, G_FORMAT),
        _ => return None,
    })
}

/// `builtin-functions`' families.
const BUILTIN: u16 = F_ATTR | F_CALC | F_IMAGE | F_FILTER | F_MINMAX | F_URL | F_VAR | F_ADJ | F_REST;

/// `regular_pseudo_classes`.
static REGULAR_PSEUDO: &[&[u8]] = &[
    b"active", b"any-link", b"blank", b"checked", b"current", b"default", b"defined",
    b"disabled", b"drop", b"empty", b"enabled", b"first", b"first-child", b"first-of-type",
    b"fullscreen", b"future", b"focus", b"focus-visible", b"focus-within", b"host", b"hover",
    b"indeterminate", b"in-range", b"invalid", b"last-child", b"last-of-type", b"left", b"link",
    b"local-link", b"only-child", b"only-of-type", b"optional", b"out-of-range", b"past",
    b"placeholder-shown", b"read-only", b"read-write", b"required", b"right", b"root", b"scope",
    b"target", b"target-within", b"user-invalid", b"valid", b"visited",
];

/// `functional_pseudo_classes`.
static FUNCTIONAL_PSEUDO: &[&[u8]] = &[
    b"dir", b"lang", b"matches", b"not", b"has", b"drop", b"nth-last-child", b"nth-child",
    b"nth-last-of-type", b"nth-of-type",
];

/// `sass-operators`' second pattern's symbols, in its order (`$` left out: an empty match).
static SASS_OP_SYMBOLS: &[&[u8]] = &[
    b"%", b"~", b"===", b"==", b"=", b"!=", b"!==", b"<=", b">=", b"<<=", b">>=", b">>>=", b"<>",
    b"<", b">", b"!", b"&&", b"||", b"?:", b"%=", b"+=", b"-=", b"&=",
];

/// `\w` in a class (`[\w-]`) past ASCII: a letter or one of Unicode's decimal digits, as in the
/// other scanners (`²` is neither).
fn class_word(c: char) -> bool {
    let u = c as u32;
    c.is_alphabetic() || crate::csharp::ND.iter().any(|&s| u >= s && u - s < 10)
}

/// A media feature's end and its two vendor prefixes' places.
type Feature = (usize, Option<Span>, Option<Span>);

/// `custom_element_chars`: `[-_a-z0-9·]` and the ranges past ASCII.
fn custom_char(c: char) -> bool {
    matches!(c, '-' | '_' | 'a'..='z' | '0'..='9') || super::custom_element_wide(c)
}

impl Scan<'_, '_, '_, true> {
    // ---- reading the line ----

    /// `{{break}}` at `i`: an ASCII character but `[-_[:alnum:]\\]` follows, or the line's end.
    fn brk(&self, i: usize) -> bool {
        if i >= self.n {
            return true;
        }
        let c = self.b[i];
        c < 0x80 && !(c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'\\'))
    }

    /// Where the run of class `k` at `i` ends (0 `[a-zA-Z0-9_-]`, 1 `[a-zA-Z_-]`, 2 `[a-z_-]`),
    /// memoized as CSS's runs are.
    fn class_run(&self, i: usize, k: usize) -> usize {
        let (a, e) = self.class_runs[k].get();
        if a <= i && i <= e {
            return e;
        }
        let in_class = |c: u8| match k {
            0 => c.is_ascii_alphanumeric() || c == b'_' || c == b'-',
            1 => c.is_ascii_alphabetic() || c == b'_' || c == b'-',
            _ => c.is_ascii_lowercase() || c == b'_' || c == b'-',
        };
        let mut j = i;
        while j < self.n && in_class(self.b[j]) {
            j += 1;
        }
        self.class_runs[k].set((i, j));
        j
    }

    /// Where `{{nmchar}}*` at `i` ends: CSS's [`Scan::nmchars`], memoized on a long line, where
    /// an op that reads an ident at each place in one would be quadratic (`@media (aaaa…`). Each
    /// place's end is the end of the place after its first character's, filled in on the way.
    fn nmchars_m(&self, i: usize) -> usize {
        if self.n < 512 {
            return self.nmchars(i);
        }
        let mut memo = self.nm_ends.borrow_mut();
        if memo.is_empty() {
            memo.resize(self.n + 2, u32::MAX);
        }
        let mut path = Vec::new();
        let mut k = i;
        let end = loop {
            if k > self.n {
                break k;
            }
            if memo[k] != u32::MAX {
                break memo[k] as usize;
            }
            match self.nmchar(k) {
                Some(q) => {
                    path.push(k);
                    k = q;
                }
                None => {
                    memo[k] = k as u32;
                    break k;
                }
            }
        };
        for p in path {
            memo[p] = end as u32;
        }
        end
    }

    /// `{{ident}}` at `i`, as CSS's [`Scan::ident`], with [`Scan::nmchars_m`].
    fn ident_m(&self, i: usize) -> Option<usize> {
        if self.b.get(i) == Some(&b'-') && self.b.get(i + 1) == Some(&b'-') {
            let e = self.nmchars_m(i + 2);
            if e > i + 2 {
                return Some(e);
            }
        }
        let s = if self.b.get(i) == Some(&b'-') { i + 1 } else { i };
        let q = self.nmstart(s)?;
        Some(self.nmchars_m(q))
    }

    /// Whether `{{ident}}` starts at `i`, read from its first characters.
    fn ident_at(&self, i: usize) -> bool {
        if i >= self.n {
            return false;
        }
        let b = self.b;
        if b[i] == b'-' {
            if b.get(i + 1) == Some(&b'-') && i + 2 < self.n && self.nmchar(i + 2).is_some() {
                return true;
            }
            return i + 1 < self.n && self.nmstart(i + 1).is_some();
        }
        self.nmstart(i).is_some()
    }

    /// Where a run of `\w` (and `-` if `dash`) at `i` ends.
    fn word_run(&self, mut i: usize, dash: bool) -> usize {
        while i < self.n {
            let c = self.b[i];
            if c < 0x80 {
                if c.is_ascii_alphanumeric() || c == b'_' || (dash && c == b'-') {
                    i += 1;
                    continue;
                }
                break;
            }
            let (ch, len) = char_at(self.b, i);
            if !class_word(ch) {
                break;
            }
            i = (i + len).min(self.n);
        }
        i
    }

    /// Where a run of the characters `\b` takes as a word's (CSS's `\w`) at `i` ends.
    fn bword_end(&self, mut i: usize) -> usize {
        while i < self.n && self.word_at(i) {
            i = self.next(i);
        }
        i
    }

    /// Where `[a-zA-Z0-9_-][\w-]*` at `i` ends: a variable's name.
    fn var_name(&self, i: usize) -> Option<usize> {
        let c = *self.b.get(i)?;
        if i >= self.n || !(c.is_ascii_alphanumeric() || c == b'_' || c == b'-') {
            return None;
        }
        Some(self.word_run(i + 1, true))
    }

    /// The word at `p` that one of `words` is, followed by `{{break}}`, ASCII case ignored if
    /// `ci`; a `\b` before it.
    fn words_brk(&mut self, p: usize, words: &[&[u8]], ci: bool, leaf: u16) -> Option<usize> {
        let b = self.b;
        if !b.get(p)?.is_ascii_alphabetic() || !self.boundary(p) {
            return None;
        }
        for w in words {
            let e = p + w.len();
            let Some(x) = b.get(p..e) else { continue };
            if (x == *w || (ci && x.eq_ignore_ascii_case(w))) && self.brk(e) {
                self.tok(p, e, leaf);
                return Some(e);
            }
        }
        None
    }

    /// `pseudo_elements` at `i` (no spaces before it): the colons' end, the vendor prefix, the
    /// end.
    fn pseudo_element_at(&self, i: usize) -> Option<(usize, Option<Span>, usize)> {
        let b = self.b;
        if b.get(i) != Some(&b':') {
            return None;
        }
        let two = b.get(i + 1) == Some(&b':');
        for colons in [2, 1] {
            if colons == 2 && !two {
                continue;
            }
            let s = i + colons;
            for w in [&b"before"[..], b"after", b"first-line", b"first-letter"] {
                let e = s + w.len();
                if b.get(s..e) == Some(w) && self.brk(e) {
                    return Some((s, None, e));
                }
            }
        }
        if two {
            let s = i + 2;
            for v in [&b"-moz-"[..], b"-ms-", b"-webkit-"] {
                if b.get(s..s + v.len()) == Some(v)
                    && let Some(e) = self.ident_m(s + v.len())
                    && self.brk(e)
                {
                    return Some((s, Some((s, s + v.len())), e));
                }
            }
            if let Some(e) = self.ident_m(s)
                && self.brk(e)
            {
                return Some((s, None, e));
            }
        }
        None
    }

    /// `:{{regular_pseudo_classes}}` at `i`: its end.
    fn regular_pseudo_at(&self, i: usize) -> Option<usize> {
        if self.b.get(i) != Some(&b':') {
            return None;
        }
        // `\b(…)\b(?![-])`: the word to a `\b` no `-` follows
        let mut e = i + 1;
        while e < self.n && (self.word_at(e) || self.b[e] == b'-') {
            e = self.next(e);
        }
        let w = &self.b[(i + 1).min(e)..e];
        REGULAR_PSEUDO.contains(&w).then_some(e)
    }

    /// `:({{functional_pseudo_classes}})\(` at `i`.
    fn functional_pseudo_at(&self, i: usize) -> bool {
        let b = self.b;
        if b.get(i) != Some(&b':') {
            return false;
        }
        let mut e = i + 1;
        while e < self.n && (b[e].is_ascii_lowercase() || b[e] == b'-') {
            e += 1;
        }
        b.get(e) == Some(&b'(') && FUNCTIONAL_PSEUDO.contains(&&b[i + 1..e])
    }

    /// `:(-(moz|ms|webkit)-){{ident}}` at `i`: the vendor prefix's end, the end.
    fn vendor_pseudo_at(&self, i: usize) -> Option<(usize, usize)> {
        let b = self.b;
        if b.get(i) != Some(&b':') {
            return None;
        }
        for v in [&b"-moz-"[..], b"-ms-", b"-webkit-"] {
            let s = i + 1;
            if b.get(s..s + v.len()) == Some(v) {
                let e = self.ident_m(s + v.len())?;
                return Some((s + v.len(), e));
            }
        }
        None
    }

    /// The four pseudo lookaheads of `selectors`' tags (and custom elements, interpolated
    /// selectors), at `i`; `which` of them (0–3), or any.
    fn pseudo_ahead(&self, i: usize, which: Option<u8>) -> bool {
        let w = |k| which.is_none_or(|x| x == k);
        (w(0) && self.pseudo_element_at(i).is_some())
            || (w(1) && self.regular_pseudo_at(i).is_some())
            || (w(2) && self.functional_pseudo_at(i))
            || (w(3) && self.vendor_pseudo_at(i).is_some())
    }

    /// The ends of `{{ident}}` at `i`, longest first (each one shorter is an ident too).
    fn ident_ends(&self, i: usize) -> Vec<usize> {
        let b = self.b;
        let mut ends = Vec::new();
        if b.get(i) == Some(&b'-') && b.get(i + 1) == Some(&b'-') {
            let mut k = i + 2;
            let mut alt = Vec::new();
            while let Some(q) = self.nmchar(k) {
                k = q;
                alt.push(k);
            }
            ends.extend(alt.into_iter().rev());
        }
        let s = if b.get(i) == Some(&b'-') { i + 1 } else { i };
        if s < self.n
            && let Some(q) = self.nmstart(s)
        {
            let mut k = q;
            let mut alt = vec![q];
            while let Some(r) = self.nmchar(k) {
                k = r;
                alt.push(k);
            }
            ends.extend(alt.into_iter().rev());
        }
        ends
    }

    // ---- the run ----

    /// The line, a place at a time, as CSS's [`Scan::run`]: each context's ops in order at the
    /// place the scan has reached; when none matches, the next character. A pop that consumes
    /// nothing right after a push that consumed nothing, at the same place and depth, would loop:
    /// as syntect, it is passed over, and if nothing else matches there the scan steps a
    /// character and searches afresh.
    fn scss_run(&mut self) {
        let n = self.n;
        let mut p = 0;
        let mut fresh = true;
        let mut guard = 0u32;
        let mut nc_push = (usize::MAX, 0usize);
        loop {
            let ctx = self.top();
            let mut looped = false;
            let q = match ctx {
                COMMENT => self.comment(p),
                STR_S | STR_D => self.scss_string(p, ctx),
                LINE => {
                    // `\n`, a pop: the rest of the line is the comment's
                    self.fill(n);
                    if p > n {
                        None
                    } else {
                        self.pop(n);
                        Some(n + 1)
                    }
                }
                FRONT => self.front(p),
                _ => {
                    let check = nc_push == (p, self.depth);
                    let mut hit = None;
                    for &op in ops(ctx) {
                        let d0 = self.depth;
                        if let Some(q) = self.sop(op, p, ctx, fresh) {
                            if q == p && self.depth < d0 && check {
                                // a pop that would loop: passed over
                                self.depth = d0;
                                looped = true;
                                continue;
                            }
                            if q == p && self.depth > d0 {
                                nc_push = (p, self.depth);
                            }
                            hit = Some(q);
                            break;
                        }
                    }
                    hit
                }
            };
            match q {
                Some(q) => {
                    if q == p {
                        guard += 1;
                        if guard > 4 * MAX as u32 {
                            guard = 0;
                            if p > n {
                                break;
                            }
                            p = self.next(p);
                            fresh = false;
                            continue;
                        }
                    } else {
                        guard = 0;
                    }
                    p = q;
                    fresh = true;
                }
                None => {
                    if p > n {
                        break;
                    }
                    p = self.next(p);
                    fresh = looped;
                }
            }
        }
        self.fill(n);
    }

    /// The front matter from `p`: a `---` line ends it (the others are YAML's, [`line`]).
    fn front(&mut self, p: usize) -> Option<usize> {
        if p > self.n {
            return None;
        }
        if p == 0 && self.b == b"---" {
            self.tok(0, 3, FRONT_END);
            self.pop(3);
            return Some(3);
        }
        Some(self.n + 1)
    }

    /// A string from `p`: its end, a newline's pop, escapes, interpolation.
    fn scss_string(&mut self, p: usize, ctx: u8) -> Option<usize> {
        let (b, n) = (self.b, self.n);
        if p > n {
            return None;
        }
        let quote = if ctx == STR_S { b'\'' } else { b'"' };
        let mut i = p;
        while i < n {
            let c = b[i];
            if c == quote {
                self.tok(i, i + 1, STRING_END);
                self.pop(i + 1);
                return Some(i + 1);
            }
            if c == b'\\' {
                let s = self.spaces(i + 1);
                if s > n {
                    self.tok(i, n, ESCAPE_NEWLINE);
                    return Some(n + 1);
                }
                if i + 1 < n {
                    let mut q = i + 1;
                    if is_hex(b[q]) {
                        while q < n && q - i <= 6 && is_hex(b[q]) {
                            q += 1;
                        }
                    } else {
                        q = self.next(q);
                    }
                    self.tok(i, q, ESCAPE);
                    return Some(q);
                }
            }
            if c == b'#' && b.get(i + 1) == Some(&b'{') && self.can_push() {
                self.interp(i);
                return Some(i + 2);
            }
            i += 1;
        }
        self.pop(n);
        Some(n + 1)
    }

    /// `(#)({)` at `p` into an interpolation.
    fn interp(&mut self, p: usize) {
        self.push(INTERP, p);
        self.tok(p, p + 1, INTERP_HASH);
        self.tok(p + 1, p + 2, BEGIN_SASS);
    }

    /// Op `op` at `p` in context `ctx`: where the scan goes on if it matched.
    fn sop(&mut self, op: u16, p: usize, ctx: u8, fresh: bool) -> Option<usize> {
        if op < 256 {
            return self.op(op as u8, p, ctx, fresh);
        }
        if op >= 1000 {
            return self.selector_part(op, p);
        }
        let (b, n) = (self.b, self.n);
        let c = self.ch(p);
        match op {
            O_COMMENT_LINE => {
                if c == Some(b'/') && b.get(p + 1) == Some(&b'/') && p + 1 < n && self.can_push() {
                    self.push(LINE, p);
                    self.tok(p, p + 2, COMMENT_MARK);
                    return Some(p + 2);
                }
                None
            }
            O_FRONT => {
                if p == 0 && b == b"---" && self.can_push() {
                    self.tok(0, 3, FRONT_BEGIN);
                    self.push(FRONT, 3);
                    return Some(3);
                }
                None
            }
            O_SELECTORS => self.selectors(p),
            O_PROPERTIES => self.properties(p),
            O_NUMERIC => self.s_numeric(p, &[T_ANGLE, T_FREQUENCY, T_LENGTH, T_RESOLUTION, T_TIME, T_PERCENT, T_NUMBER]),
            O_PERCENT => self.s_numeric(p, &[T_PERCENT]),
            O_LENGTH => self.s_numeric(p, &[T_LENGTH]),
            O_ANGLE => self.s_numeric(p, &[T_ANGLE]),
            O_RESOLUTION => self.s_numeric(p, &[T_RESOLUTION]),
            O_NUMBER => self.s_numeric(p, &[T_NUMBER]),
            O_INTEGER => self.s_numeric(p, &[T_INTEGER]),
            O_ISEL => self.interpolated_selector(p),
            O_CUSTOM_EL => self.custom_elements(p),
            O_AT_RULES => self.s_at_rules(p),
            O_CURLY => match c {
                Some(b'{') => {
                    self.tok(p, p + 1, PL_BEGIN);
                    Some(p + 1)
                }
                Some(b'}') => {
                    self.tok(p, p + 1, PL_END);
                    Some(p + 1)
                }
                _ => None,
            },
            O_CSS_MODULES => {
                let s = self.spaces(p);
                if self.ch(s) == Some(b':') && s < n {
                    for w in [&b"import"[..], b"export"] {
                        if b.get(s + 1..s + 1 + w.len()) == Some(w) && self.can_push() {
                            let e = s + 1 + w.len();
                            self.tok(p, s, PSEUDO_MODULE);
                            self.tok(s, s + 1, PSEUDO_MODULE_MARK);
                            self.tok(s + 1, e, PSEUDO_MODULE);
                            self.push(CMOD, e);
                            return Some(e);
                        }
                    }
                }
                None
            }
            O_SEMI_POP => {
                let s = self.spaces(p);
                if self.ch(s) == Some(b';') {
                    self.tok(s, s + 1, RULE_END);
                    self.pop(s + 1);
                    return Some(s + 1);
                }
                None
            }
            O_VALUE_AHEAD => {
                let s = self.spaces(p);
                if matches!(self.ch(s), Some(b';' | b'}' | b')')) {
                    self.pop(p);
                    return Some(p);
                }
                None
            }
            O_COLON_VALUE => {
                let s = self.spaces(p);
                if self.ch(s) == Some(b':') && self.can_push() {
                    self.tok(s, s + 1, KEY_VALUE);
                    self.push(PV, s + 1);
                    return Some(s + 1);
                }
                None
            }
            O_COLON_FONT => {
                if c == Some(b':') && self.can_push() {
                    self.tok(p, p + 1, KEY_VALUE);
                    self.push(FONT_VAL, p + 1);
                    return Some(p + 1);
                }
                None
            }
            O_POP => {
                self.pop(p);
                Some(p)
            }
            O_PV => self.list(PROPERTY_VALUES, p, ctx, fresh),
            O_SSE => self.list(SSE, p, ctx, fresh),
            O_SVE => self.list(SVE, p, ctx, fresh),
            O_MEDIA_QUERY => self.list(MEDIA_QUERY, p, ctx, fresh),
            O_SP => self.list(SELECTOR_PARTS, p, ctx, fresh),
            O_BRACE_AHEAD | O_CURLY_AHEAD => {
                if c == Some(b'{') {
                    self.pop(p);
                    return Some(p);
                }
                None
            }
            O_MAP => {
                if c == Some(b'(') && self.can_push() {
                    self.push(MAP, p);
                    self.tok(p, p + 1, BEGIN_SASS);
                    return Some(p + 1);
                }
                None
            }
            O_MAP_END | O_CLOSE_SASS | O_INTERP_END => {
                let close = if op == O_INTERP_END { b'}' } else { b')' };
                if c == Some(close) {
                    self.tok(p, p + 1, END_SASS);
                    self.pop(p + 1);
                    return Some(p + 1);
                }
                None
            }
            O_MAP_VAR_KEY => {
                if c != Some(b'$') {
                    return None;
                }
                let e = self.var_name(p + 1)?;
                let s = self.spaces(e);
                if self.ch(s) == Some(b':') && s < n {
                    self.tok(p, p + 1, VAR_MARK);
                    self.tok(p + 1, s, VAR);
                    self.tok(s, s + 1, VAR_KEY_VALUE);
                    return Some(s + 1);
                }
                None
            }
            O_MAP_KEY => {
                let e = self.ident_m(p)?;
                let s = self.spaces(e);
                if self.ch(s) == Some(b':') && s < n {
                    self.tok(p, e, PARAM);
                    self.tok(s, s + 1, KEY_VALUE);
                    return Some(s + 1);
                }
                None
            }
            O_VARIABLES => {
                // `(({{ident}})(\.))?(\$)([a-zA-Z0-9_-][\w-]*)`, then `sass-interpolation`
                if let Some(e) = self.ident_m(p)
                    && b.get(e) == Some(&b'.')
                    && b.get(e + 1) == Some(&b'$')
                    && let Some(f) = self.var_name(e + 2)
                {
                    self.tok(p, e, VAR_NS);
                    self.tok(e, e + 1, VAR_NS_DOT);
                    self.tok(e + 1, e + 2, VAR_MARK);
                    self.tok(e + 2, f, VAR);
                    return Some(f);
                }
                if c == Some(b'$')
                    && let Some(f) = self.var_name(p + 1)
                {
                    self.tok(p, p + 1, VAR_MARK);
                    self.tok(p + 1, f, VAR);
                    return Some(f);
                }
                self.sop(O_INTERP, p, ctx, fresh)
            }
            O_INTERP => {
                if c == Some(b'#') && b.get(p + 1) == Some(&b'{') && self.can_push() {
                    self.interp(p);
                    return Some(p + 2);
                }
                None
            }
            O_SASS_FN => {
                if !c?.is_ascii_lowercase() || !self.boundary(p) {
                    return None;
                }
                let e = self.name_end(p);
                if b.get(e) != Some(&b'(') || lookup(&b[p..e]).lists & SASS_FN == 0 {
                    return None;
                }
                if lookup(&b[p..e]).lists & SASS_FN == 0 || &b[p..e] != b[p..e].to_ascii_lowercase().as_slice() {
                    return None;
                }
                self.call(p, e, FN_SASS, G_ARGS, GROUP_BEGIN)
            }
            O_CUSTOM_FN => {
                if !self.boundary(p) {
                    return None;
                }
                let run = |i: usize| self.class_run(i, 2);
                if let Some(e) = self.ident_m(p)
                    && b.get(e) == Some(&b'.')
                {
                    let f = run(e + 1);
                    if f > e + 1 && b.get(f) == Some(&b'(') {
                        self.tok(p, e, FN_SASS_NS);
                        self.tok(e, e + 1, FN_SASS_NS_DOT);
                        return self.call(e + 1, f, FN_SASS, G_ARGS, GROUP_BEGIN);
                    }
                }
                let f = run(p);
                if f > p && b.get(f) == Some(&b'(') {
                    return self.call(p, f, FN_SASS, G_ARGS, GROUP_BEGIN);
                }
                None
            }
            O_OPS => self.sass_operators(p),
            O_CONCAT => self.concat(p),
            O_UNQUOTED | O_ARG_UNQUOTED => {
                if !self.boundary(p) {
                    return None;
                }
                let parens = op == O_ARG_UNQUOTED;
                let mut i = p;
                let mut last = None;
                while i < n {
                    let ch = b[i];
                    if self.space(i).is_some()
                        || ch == b'\''
                        || ch == b'"'
                        || (parens && (ch == b'(' || ch == b')'))
                    {
                        break;
                    }
                    i = self.next(i);
                    if self.boundary(i) {
                        last = Some(i);
                    }
                }
                let e = last?;
                self.tok(p, e, UNQUOTED);
                Some(e)
            }
            O_BUILTIN => self.s_function_at(p, BUILTIN),
            O_VAR_FN => self.s_function_at(p, F_VAR),
            O_CALC_FN => self.s_function_at(p, F_CALC),
            O_ATTR_FN => self.s_function_at(p, F_ATTR),
            O_MINMAX_FN => self.s_function_at(p, F_MINMAX),
            O_URL_FN => self.s_function_at(p, F_URL),
            O_DOC_FNS => self.s_function_at(p, F_DOC),
            O_ADJ_FNS => self.s_function_at(p, F_ADJ),
            O_COLOR_FNS => self.s_function_at(p, F_COLOR),
            O_IMAGE_TYPE => self.s_function_at(p, F_IMAGE | F_URL),
            O_FILTER_FNS => self.s_function_at(p, F_FILTER),
            O_COLOR_VALUES => self.s_color_values(p),
            O_HEX => self.hex(p),
            O_CONSTANTS => self.s_constants(p),
            O_TRUE_FALSE => {
                // `\b(false|true|null)\b`
                if !self.boundary(p) {
                    return None;
                }
                for w in [&b"false"[..], b"true", b"null"] {
                    let e = p + w.len();
                    if b.get(p..e) == Some(w) && self.boundary(e) {
                        self.tok(p, e, CONST_LANG);
                        return Some(e);
                    }
                }
                None
            }
            O_ADJ_OPS => {
                // `[-+*](?=\s)`, then `[-+*/]`
                let ch = c?;
                if matches!(ch, b'-' | b'+' | b'*') && self.space(p + 1).is_some() {
                    self.tok(p, p + 1, ARITHMETIC);
                    return Some(p + 1);
                }
                if matches!(ch, b'-' | b'+' | b'*' | b'/') {
                    self.tok(p, p + 1, ILLEGAL_OP);
                    return Some(p + 1);
                }
                None
            }
            O_GROUP_CONIC => {
                if c == Some(b'(') && self.can_push() {
                    self.tok(p, p + 1, SECTION_BEGIN);
                    self.push(ctx & !WRAP, p + 1);
                    return Some(p + 1);
                }
                None
            }
            O_PAREN_END => {
                if c == Some(b')') {
                    self.tok(p, p + 1, GROUP_END);
                    self.pop(p + 1);
                    return Some(p + 1);
                }
                None
            }
            O_VAR_COMMA | O_COUNTER_COMMA => {
                if c == Some(b',') && self.can_push() {
                    let to = match ctx {
                        G_COUNTER => COUNTER_REST,
                        G_COUNTERS => COUNTERS_REST,
                        _ => VAR_REST,
                    };
                    self.push(to, p);
                    return Some(p);
                }
                None
            }
            O_ATTR_FN_NAME => {
                let e = self.ident_m(p)?;
                if !self.can_push() {
                    return None;
                }
                self.tok(p, e, ATTR_NAME);
                self.push(ATTR_FN_REST, e);
                Some(e)
            }
            O_UNITS => {
                // `\b(units)\b`
                if !c?.is_ascii_alphabetic() || !self.boundary(p) {
                    return None;
                }
                let e = self.unit_end(p);
                if self.boundary(e) && unit_type(&b[p..e]) != super::NO_TYPE {
                    self.tok(p, e, NUM_SUFFIX);
                    return Some(e);
                }
                None
            }
            O_URL_SCHEME => {
                for w in [&b"https://"[..], b"http://", b"ftp://"] {
                    if b.get(p..p + w.len()) == Some(w) {
                        self.tok(p, p + w.len(), UNQUOTED);
                        return Some(p + w.len());
                    }
                }
                None
            }
            O_DENSITY => {
                let d = self.digits(p);
                if d > p && b.get(d) == Some(&b'x') {
                    self.tok(p, d, NUM_VALUE);
                    self.tok(d, d + 1, NUM_SUFFIX);
                    return Some(d + 1);
                }
                None
            }
            O_W_AT_FROM => self.words_brk(p, &[b"at", b"from"], true, KW_GRADIENT),
            O_W_CONIC_PLACES => self.words_brk(
                p,
                &[b"bottom", b"center", b"left", b"right", b"top"],
                true,
                CONSTANT,
            ),
            O_W_TO => self.words_brk(p, &[b"to"], false, KEYWORD_OTHER),
            O_W_SIDES => self.words_brk(p, &[b"bottom", b"left", b"right", b"top"], false, CONSTANT),
            O_W_RADIAL => self.words_brk(p, &[b"at", b"circle", b"ellipse"], false, KEYWORD_OTHER),
            O_W_RADIAL_PLACES => self.words_brk(
                p,
                &[
                    b"left",
                    b"center",
                    b"right",
                    b"top",
                    b"bottom",
                    b"closest-corner",
                    b"closest-side",
                    b"farthest-corner",
                    b"farthest-side",
                ],
                false,
                CONSTANT,
            ),
            O_W_SIZES_A => self.words_brk(
                p,
                &[b"auto", b"max-content", b"min-content"],
                false,
                CONSTANT,
            ),
            O_W_SIZES_B => self.words_brk(
                p,
                &[b"max-content", b"min-content", b"auto"],
                false,
                CONSTANT,
            ),
            O_W_COLOR_SPACES => self.words_brk(p, &[b"rgb", b"hsl", b"hwb"], true, COLOR_SPACE),
            O_W_STEPS => self.words_brk(p, &[b"end", b"middle", b"start"], false, TIMING_DIRECTION),
            O_W_AUTO => self.words_brk(p, &[b"auto"], false, CONSTANT),
            O_W_ROUND => self.words_brk(p, &[b"round"], false, KEYWORD_OTHER),
            O_W_AT => self.words_brk(p, &[b"at"], false, KEYWORD_OTHER),
            O_W_CIRCLE_PLACES => self.words_brk(
                p,
                &[
                    b"top",
                    b"right",
                    b"bottom",
                    b"left",
                    b"center",
                    b"closest-side",
                    b"farthest-side",
                ],
                false,
                CONSTANT,
            ),
            O_W_FILL_RULES => self.words_brk(p, &[b"nonzero", b"evenodd"], false, CONSTANT),
            O_W_REPETITIONS => self.words_brk(p, &[b"auto-fill", b"auto-fit"], false, REPETITIONS),
            O_W_EVEN_ODD => self.words_brk(p, &[b"even", b"odd"], false, PSEUDO_KEYWORD),
            O_CALC_OPEN | O_CALC_PAREN => {
                if c == Some(b'(') && self.can_push() {
                    self.tok(p, p + 1, GROUP_BEGIN);
                    self.push(CALC_PARENS, p + 1);
                    return Some(p + 1);
                }
                None
            }
            O_CALC_CLOSE => {
                // `\)`, set to `maybe-illegal-operator`, which pops
                if c == Some(b')') {
                    self.tok(p, p + 1, GROUP_END);
                    self.pop(p + 1);
                    return Some(self.maybe_illegal(p + 1));
                }
                None
            }
            O_ARGS_PAREN => {
                if c == Some(b'(') && self.can_push() {
                    self.tok(p, p + 1, GROUP_BEGIN);
                    self.push(ARGS_PAREN, p + 1);
                    return Some(p + 1);
                }
                None
            }
            O_CALC_ARITH => {
                if matches!(c, Some(b'-' | b'+' | b'*' | b'/')) && p < n {
                    self.tok(p, p + 1, ARITHMETIC);
                    return Some(p + 1);
                }
                None
            }
            O_ARG_COLON => {
                if c == Some(b':') {
                    self.tok(p, p + 1, SEP);
                    return Some(p + 1);
                }
                None
            }
            O_PSEUDO_EL => self.pseudo_element(p),
            O_PSEUDO_CL => self.pseudo_class(p),
            O_SEL_IDENT => {
                let e = self.ident_m(p)?;
                let leaf = match ctx {
                    CLASS => CLASS_NAME,
                    ID => ID_NAME,
                    PLACEHOLDER => PH_NAME,
                    _ => ATTR_NAME,
                };
                self.tok(p, e, leaf);
                Some(e)
            }
            O_AMP_DASH => {
                let mut e = p;
                while e < n && matches!(b[e], b'-' | b'_') {
                    e += 1;
                }
                if e == p || !self.can_push() {
                    return None;
                }
                self.tok(p, e, ATTR_NAME);
                self.push(AMP_SUF, e);
                Some(e)
            }
            O_ATTR_OPEN => {
                if c == Some(b'[') && self.can_push() {
                    self.push(ATTR, p);
                    self.tok(p, p + 1, ENTITY_MARK);
                    return Some(p + 1);
                }
                None
            }
            O_ISEL_OPEN => {
                if c == Some(b'{') && self.can_push() {
                    self.tok(p, p + 1, BEGIN_SASS);
                    self.push(ISEL_IN, p + 1);
                    return Some(p + 1);
                }
                None
            }
            O_PS_OPEN => {
                if c == Some(b'(') && self.can_push() {
                    let g = match ctx {
                        PS_DIR => G_PDIR,
                        PS_SEL => G_PSEL,
                        PS_DROP => G_PDROP,
                        PS_NTH => G_PNTH,
                        _ => G_PLOCAL,
                    };
                    self.tok(p, p + 1, GROUP_BEGIN);
                    self.push(g, p + 1);
                    return Some(p + 1);
                }
                None
            }
            O_PS_COMMA => {
                if c == Some(b',') {
                    self.tok(p, p + 1, SEP);
                    return Some(p + 1);
                }
                None
            }
            O_NTH => self.nth_sass(p),
            O_IN => {
                if b.get(p..p + 2) == Some(b"in") && self.boundary(p) && self.boundary(p + 2) && self.can_push() {
                    self.tok(p, p + 2, OP_SASS);
                    self.push(EACH_IN, p + 2);
                    return Some(p + 2);
                }
                None
            }
            O_SEMI_SASS | O_SEMI_SASS_POP => {
                if c == Some(b';') {
                    self.tok(p, p + 1, TERM_SASS);
                    self.pop(p + 1);
                    return Some(p + 1);
                }
                None
            }
            O_BANG_FLAG => {
                if c == Some(b'!') {
                    let s = self.spaces(p + 1);
                    for w in [&b"default"[..], b"global", b"optional"] {
                        if b.get(s..s + w.len()) == Some(w) {
                            self.tok(p, s + w.len(), KW_SASS);
                            return Some(s + w.len());
                        }
                    }
                }
                None
            }
            O_IMPORTANT => {
                if c == Some(b'!') {
                    let s = self.spaces(p + 1);
                    if b.get(s..s + 9) == Some(b"important") && self.brk(s + 9) {
                        self.tok(p, s + 9, IMPORTANT);
                        return Some(s + 9);
                    }
                }
                None
            }
            O_SEMI_INVALID => {
                if c == Some(b';') {
                    self.tok(p, p + 1, INVALID_SASS);
                    return Some(p + 1);
                }
                None
            }
            O_SLASH => {
                if c == Some(b'/') {
                    self.tok(p, p + 1, ARITHMETIC);
                    return Some(p + 1);
                }
                None
            }
            O_IDENT_UNQUOTED | O_IDENT_SASS | O_IDENT_PLAIN => {
                let e = self.ident_m(p)?;
                let leaf = match op {
                    O_IDENT_UNQUOTED => UNQUOTED,
                    O_IDENT_SASS => UNQUOTED_SASS,
                    _ => super::PLAIN,
                };
                self.tok(p, e, leaf);
                Some(e)
            }
            O_FONT_IDENTS => self.font_idents(p),
            O_BRACE_SEMI_AHEAD | O_SPACES_CURLY_AHEAD | O_SPACES_SEMI_AHEAD => {
                let s = self.spaces(p);
                let ok = match op {
                    O_BRACE_SEMI_AHEAD => matches!(self.ch(s), Some(b'{' | b';')),
                    O_SPACES_CURLY_AHEAD => self.ch(s) == Some(b'{'),
                    _ => self.ch(s) == Some(b';'),
                };
                if ok {
                    self.pop(s);
                    return Some(s);
                }
                None
            }
            O_BRACE_SEMI_NOW => {
                if matches!(c, Some(b'{' | b';')) {
                    self.pop(p);
                    return Some(p);
                }
                None
            }
            O_MIXIN_OPEN | O_INCLUDE_OPEN => {
                if c == Some(b'(') && self.can_push() {
                    self.tok(p, p + 1, BEGIN_SASS);
                    self.push(if op == O_MIXIN_OPEN { MIXIN_PARAMS } else { INCLUDE_ARGS }, p + 1);
                    return Some(p + 1);
                }
                None
            }
            O_PARAM => {
                if c != Some(b'$') || !self.can_push() {
                    return None;
                }
                let e = self.var_name(p + 1)?;
                self.tok(p, p + 1, PARAM_MARK);
                self.tok(p + 1, e, PARAM);
                self.push(PARAM_VAL, e);
                Some(e)
            }
            O_MIXIN_BLOCK => {
                if c == Some(b'{') && self.can_push() {
                    self.tok(p, p + 1, PL_BEGIN_SASS);
                    self.push(MIXIN_BLOCK, p + 1);
                    return Some(p + 1);
                }
                None
            }
            O_MIXIN_BLOCK_END => {
                if c == Some(b'}') {
                    self.tok(p, p + 1, PL_END_SASS);
                    self.pop(p + 1);
                    return Some(p + 1);
                }
                None
            }
            O_PARAM_END => {
                if matches!(c, Some(b',' | b')')) {
                    self.pop(p);
                    return Some(p);
                }
                None
            }
            O_KEY_VALUE => {
                if c == Some(b':') {
                    self.tok(p, p + 1, KEY_VALUE);
                    return Some(p + 1);
                }
                None
            }
            O_SEMI_INC | O_SEMI_RET | O_USE_SEMI => {
                let s = self.spaces(p);
                if self.ch(s) == Some(b';') {
                    match op {
                        O_SEMI_INC => self.tok(s, s + 1, TERM_SASS),
                        O_SEMI_RET => self.tok(s, s + 1, RULE_END),
                        _ => self.tok(p, s + 1, TERM_SASS),
                    }
                    self.pop(s + 1);
                    return Some(s + 1);
                }
                None
            }
            O_USE_AS => {
                // `\b(as)\b\s+([a-zA-Z0-9_-]*(\*)?)`
                if b.get(p..p + 2) == Some(b"as") && self.boundary(p) && self.boundary(p + 2) {
                    let s = self.spaces(p + 2);
                    if s > p + 2 && s <= n {
                        let mut e = s;
                        while e < n && (b[e].is_ascii_alphanumeric() || b[e] == b'_' || b[e] == b'-') {
                            e += 1;
                        }
                        self.tok(p, p + 2, KW_AS);
                        if b.get(e) == Some(&b'*') {
                            self.tok(e, e + 1, IMPORT_ALL);
                            return Some(e + 1);
                        }
                        return Some(e);
                    }
                }
                None
            }
            O_USE_SHOW => {
                for w in [&b"show"[..], b"hide"] {
                    let e = p + w.len();
                    if b.get(p..e) == Some(w) && self.boundary(p) && self.boundary(e) && self.can_push() {
                        self.tok(p, e, KW_SHOW);
                        self.push(USE_SHOW, e);
                        return Some(e);
                    }
                }
                None
            }
            O_USE_WITH => {
                if b.get(p..p + 4) == Some(b"with") {
                    let s = self.spaces(p + 4);
                    if s > p + 4 && self.ch(s) == Some(b'(') && self.can_push() {
                        self.tok(p, p + 4, KW_WITH);
                        self.tok(s, s + 1, BEGIN_SASS);
                        self.push(USE_WITH, s + 1);
                        return Some(s + 1);
                    }
                }
                None
            }
            O_SEMI_AHEAD => {
                if c == Some(b';') {
                    self.pop(p);
                    return Some(p);
                }
                None
            }
            O_URL_OPEN => {
                if b.get(p..p + 4) == Some(b"url(") && self.can_push() {
                    self.tok(p, p + 3, FN_URL);
                    self.tok(p + 3, p + 4, GROUP_BEGIN);
                    self.push(URL_G, p + 4);
                    return Some(p + 4);
                }
                None
            }
            O_CM_END => {
                if c == Some(b';') {
                    self.tok(p, p + 1, TERMINATOR_LEAF);
                    self.pop(p + 1);
                    return Some(p + 1);
                }
                None
            }
            O_CM_KEYWORD => {
                let s = self.spaces(p);
                if b.get(s..s + 13) == Some(b"@custom-media") && self.can_push() {
                    self.at_pair(s, s + 13, AT_CUSTOM_MEDIA);
                    self.push(CM, s + 13);
                    return Some(s + 13);
                }
                None
            }
            O_IMPORT_ARGS => {
                if c != Some(b'(') {
                    return None;
                }
                for w in [
                    &b"reference"[..],
                    b"inline",
                    b"less",
                    b"css",
                    b"once",
                    b"multiple",
                    b"optional",
                ] {
                    let e = p + 1 + w.len();
                    if b.get(p + 1..e) == Some(w) && b.get(e) == Some(&b')') {
                        self.tok(p, p + 1, BEGIN_SASS);
                        self.tok(p + 1, e, CONST_OTHER_SASS);
                        self.tok(e, e + 1, END_SASS);
                        return Some(e + 1);
                    }
                }
                None
            }
            O_KF_NAME => {
                // `\s*({{ident}})?`: it matches at least empty, so it counts only where a search
                // starts, and only when it takes something
                if !fresh {
                    return None;
                }
                let s = self.spaces(p);
                if let Some(e) = self.ident_m(s)
                    && s < n
                {
                    self.tok(s, e, ANIMATION_NAME);
                    return Some(e);
                }
                (s > p).then_some(s)
            }
            O_KF_COMMA => {
                let s = self.spaces(p);
                if self.ch(s) == Some(b',') {
                    self.tok(s, s + 1, REPETITION);
                    return Some(s + 1);
                }
                None
            }
            O_SEMI_RULE => {
                if c == Some(b';') {
                    self.tok(p, p + 1, RULE_END);
                    return Some(p + 1);
                }
                None
            }
            O_SEMI_EOL_AHEAD | O_EOL_AHEAD => {
                if (op == O_SEMI_EOL_AHEAD && c == Some(b';')) || self.eol(p) {
                    self.pop(p);
                    return Some(p);
                }
                None
            }
            O_SUP_OPEN | O_MQ_OPEN => {
                if c == Some(b'(') && self.can_push() {
                    self.tok(p, p + 1, GROUP_BEGIN);
                    self.push(if op == O_SUP_OPEN { SUP_PAREN } else { MQ_PAREN }, p + 1);
                    return Some(p + 1);
                }
                None
            }
            O_MEDIA_FEATURE => self.media_feature_scss(p),
            O_MF_END => {
                if c == Some(b':') {
                    self.tok(p, p + 1, KEY_VALUE);
                    self.pop(p + 1);
                    return Some(p + 1);
                }
                if c == Some(b')') {
                    self.pop(p);
                    return Some(p);
                }
                None
            }
            O_ORIENTATION => {
                if !self.boundary(p) {
                    return None;
                }
                for w in [&b"portrait"[..], b"landscape", b"progressive", b"interlace"] {
                    if b.get(p..p + w.len()) == Some(w) {
                        self.tok(p, p + w.len(), CONSTANT);
                        return Some(p + w.len());
                    }
                }
                None
            }
            O_RATIO => {
                let s = self.spaces(p);
                let a = self.digits(s);
                if a > s && b.get(a) == Some(&b'/') {
                    let d = self.digits(a + 1);
                    if d > a + 1 {
                        self.tok(s, a, NUM_PLAIN);
                        self.tok(a, a + 1, ARITHMETIC);
                        self.tok(a + 1, d, NUM_PLAIN);
                        return Some(d);
                    }
                }
                None
            }
            O_CMOD_OPEN => {
                if c == Some(b'(') && self.can_push() {
                    self.tok(p, p + 1, GROUP_BEGIN);
                    self.push(CMOD_G, p + 1);
                    return Some(p + 1);
                }
                None
            }
            O_CMOD_BLOCK => {
                let s = self.spaces(p);
                if self.ch(s) == Some(b'{') && self.can_push() {
                    // `\s*(\{)`'s scope is the whole match's
                    self.tok(p, s + 1, PL_BEGIN);
                    self.push(CMOD_BLOCK, s + 1);
                    return Some(s + 1);
                }
                None
            }
            O_CMOD_BLOCK_END => {
                if c == Some(b'}') {
                    self.tok(p, p + 1, PL_END);
                    self.pop(p + 1);
                    return Some(p + 1);
                }
                None
            }
            O_CMOD_KEY => {
                let mut e = p;
                while e < n && (b[e].is_ascii_alphanumeric() || b[e] == b'_' || b[e] == b'-') {
                    e += 1;
                }
                if e == p {
                    return None;
                }
                let s = self.spaces(e);
                if self.ch(s) == Some(b':') && s < n && self.can_push() {
                    self.tok(p, e, PROP_MODULE);
                    self.tok(s, s + 1, KEY_VALUE);
                    self.push(CMOD_VAL, s + 1);
                    return Some(s + 1);
                }
                None
            }
            O_CMOD_VAL_END => {
                if c == Some(b';') {
                    self.tok(p, p + 1, RULE_END);
                    self.pop(p + 1);
                    return Some(p + 1);
                }
                // `\s*(?=$)`, in the capture that is the terminator's scope
                let s = self.spaces(p);
                if s >= n {
                    self.tok(p, n, RULE_END);
                    self.pop(s);
                    return Some(s);
                }
                None
            }
            _ => None,
        }
    }

    /// The ops of an include, tried in order at `p`.
    fn list(&mut self, ops: &[u16], p: usize, ctx: u8, fresh: bool) -> Option<usize> {
        for &op in ops {
            if let Some(q) = self.sop(op, p, ctx, fresh) {
                return Some(q);
            }
        }
        None
    }

    /// A function's name at `p..e` as `leaf`, its call pushed and its group after the `(` at `e`
    /// (`begin` its leaf).
    fn call(&mut self, p: usize, e: usize, leaf: u16, group: u8, begin: u16) -> Option<usize> {
        if !self.can_push() {
            return None;
        }
        self.tok(p, e, leaf);
        if group == CALC_W {
            self.push(CALC_W, e);
            return Some(e);
        }
        self.push(group | WRAP, e);
        if self.can_push() {
            self.tok(e, e + 1, begin);
            self.push(group, e + 1);
            return Some(e + 1);
        }
        Some(e)
    }

    /// A builtin function of one of `families` at `p`.
    fn s_function_at(&mut self, p: usize, families: u16) -> Option<usize> {
        let b = self.b;
        let c = *b.get(p)?;
        if !c.is_ascii_alphabetic() || !self.boundary(p) {
            return None;
        }
        let e = self.name_end(p);
        if b.get(e) != Some(&b'(') {
            return None;
        }
        let (family, leaf, group) = function(&b[p..e])?;
        if family & families == 0 {
            return None;
        }
        let begin = if group == G_CONIC { SECTION_BEGIN } else { GROUP_BEGIN };
        self.call(p, e, leaf, group, begin)
    }

    /// `color-hex-values` at `p`.
    fn hex(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if b.get(p) != Some(&b'#') {
            return None;
        }
        let mut h = p + 1;
        while h < self.n && is_hex(b[h]) {
            h += 1;
        }
        let k = h - p - 1;
        for (lens, mark, leaf) in [([3, 6], HEX_MARK, HEX), ([4, 8], HEXA_MARK, HEXA)] {
            for l in lens {
                if k >= l && self.boundary(p + 1 + l) {
                    self.tok(p, p + 1, mark);
                    self.tok(p + 1, p + 1 + l, leaf);
                    return Some(p + 1 + l);
                }
            }
        }
        None
    }

    /// `color-values` at `p`.
    fn s_color_values(&mut self, p: usize) -> Option<usize> {
        if let Some(e) = self.s_function_at(p, F_COLOR) {
            return Some(e);
        }
        if let Some(e) = self.hex(p) {
            return Some(e);
        }
        let b = self.b;
        if !b.get(p)?.is_ascii_alphabetic() || !self.boundary(p) {
            return None;
        }
        // `\b(?i:…){{break}}`: the word to a break, which no name character is
        let e = self.name_end(p);
        if !self.brk(e) {
            return None;
        }
        // `(?i:…)`, all three lists: the word looked up in lowercase
        let mut low = [0u8; 32];
        if e - p > low.len() {
            return None;
        }
        for (d, c) in low.iter_mut().zip(&b[p..e]) {
            *d = c.to_ascii_lowercase();
        }
        let w = lookup(&low[..e - p]);
        for (bit, leaf) in [
            (STD_COLOR, STD_COLOR_LEAF),
            (EXT_COLOR, EXT_COLOR_LEAF),
            (SPECIAL_COLOR, SPECIAL_COLOR_LEAF),
        ] {
            if w.lists & bit != 0 {
                self.tok(p, e, leaf);
                return Some(e);
            }
        }
        None
    }

    /// `property-value-constants` at `p`.
    fn s_constants(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if !b.get(p)?.is_ascii_alphabetic() || !self.boundary(p) {
            return None;
        }
        // `{{break}}(?=[\s;{),/])`: the word must end at one of those
        let delim = |i: usize| {
            i >= self.n
                || matches!(self.b[i], b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r' | b';' | b'{' | b')' | b',' | b'/')
        };
        if b.get(p..p + 23) == Some(b"progid:DXImageTransform") {
            let m = self.next(p + 23);
            if m < self.n && b.get(m..m + 9) == Some(b"Microsoft") {
                let s = self.next(m + 9);
                for x in [&b"Alpha"[..], b"Blur", b"dropshadow", b"gradient", b"Shadow"] {
                    let e = s + x.len();
                    if s < self.n && b.get(s..e) == Some(x) && delim(e) {
                        self.tok(p, e, CONSTANT);
                        return Some(e);
                    }
                }
            }
        }
        let e = self.name_end(p);
        if delim(e) && lookup(&b[p..e]).lists & CONST != 0 {
            self.tok(p, e, CONSTANT);
            return Some(e);
        }
        // `\b(?i:sans-serif|serif|monospace|fantasy|cursive|system-ui)\b(?=\s*[;,\n}{])`
        for w in [
            &b"sans-serif"[..],
            b"serif",
            b"monospace",
            b"fantasy",
            b"cursive",
            b"system-ui",
        ] {
            let e = p + w.len();
            if b.get(p..e).is_some_and(|x| x.eq_ignore_ascii_case(w)) && self.boundary(e) {
                let s = self.spaces(e);
                if s > self.n || matches!(self.ch(s), Some(b';' | b',' | b'}' | b'{')) {
                    self.tok(p, e, FONT_NAME_LEAF);
                    return Some(e);
                }
            }
        }
        // `\b(true|false|null)\b`
        for w in [&b"true"[..], b"false", b"null"] {
            let e = p + w.len();
            if b.get(p..e) == Some(w) && self.boundary(e) {
                self.tok(p, e, CONST_LANG);
                return Some(e);
            }
        }
        None
    }

    /// The first of `types` that matches at `p`, each tried in its patterns' order; the number
    /// pushes `maybe-illegal-operator`, which checks the sign after it and pops.
    fn s_numeric(&mut self, p: usize, types: &[u8]) -> Option<usize> {
        let b = self.b;
        let c = *b.get(p)?;
        if !(c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.')) || p >= self.n {
            return None;
        }
        if !self.can_push() {
            return None;
        }
        let num = self.num_at(p);
        let unit = |f: usize| {
            let u = self.unit_end(f);
            let t = if self.boundary(u) {
                unit_type(&b[f..u])
            } else {
                super::NO_TYPE
            };
            (u, t)
        };
        let float = num
            .and_then(|nm| nm.dot.map(|(_, e)| e).or(nm.exp))
            .map(|f| (f, unit(f)));
        let int = num.filter(|nm| nm.int > nm.digits).map(|nm| (nm.int, unit(nm.int)));
        let zero = c == b'0' && self.boundary(p + 1) && b.get(p + 1) != Some(&b'%');
        for &t in types {
            let hit = match t {
                T_PERCENT => {
                    if let Some((f, _)) = float
                        && b.get(f) == Some(&b'%')
                    {
                        Some((f, f + 1, true))
                    } else if let Some((i, _)) = int
                        && b.get(i) == Some(&b'%')
                    {
                        Some((i, i + 1, false))
                    } else {
                        None
                    }
                }
                T_NUMBER | T_INTEGER => {
                    if t == T_NUMBER
                        && let Some((f, _)) = float
                    {
                        Some((f, f, true))
                    } else {
                        int.map(|(i, _)| (i, i, false))
                    }
                }
                _ => {
                    if let Some((f, (u, ut))) = float
                        && ut == t
                    {
                        Some((f, u, true))
                    } else if let Some((i, (u, ut))) = int
                        && ut == t
                    {
                        Some((i, u, false))
                    } else {
                        None
                    }
                }
            };
            if let Some((end, unit, is_float)) = hit {
                let nm = num?;
                self.number_toks(p, &nm, end, unit, is_float);
                return Some(self.maybe_illegal(unit));
            }
            if zero && (t == T_LENGTH || t == T_ANGLE) {
                self.tok(p, p + 1, NUM_VALUE);
                return Some(self.maybe_illegal(p + 1));
            }
        }
        None
    }

    /// A number's tokens: its sign, its value (a float's `.` in it), its unit or `%`.
    fn number_toks(&mut self, p: usize, nm: &Num, end: usize, unit: usize, float: bool) {
        self.tok(p, nm.digits, ARITHMETIC);
        match nm.dot {
            Some((dot, _)) if float => {
                self.tok(nm.digits, dot, NUM_VALUE);
                self.tok(dot, dot + 1, NUM_DOT);
                self.tok(dot + 1, end, NUM_VALUE);
            }
            _ => self.tok(nm.digits, end, NUM_VALUE),
        }
        self.tok(end, unit, NUM_SUFFIX);
    }

    /// `maybe-illegal-operator` at `e`, after a number or a `calc(`'s `)`: `[-+](?=\s*\d)`, then
    /// `\s*([-+])(?=\d)`, each a pop, else `''`'s; where the scan goes on.
    fn maybe_illegal(&mut self, e: usize) -> usize {
        let b = self.b;
        if matches!(b.get(e), Some(b'-' | b'+')) && e < self.n {
            let s = self.spaces(e + 1);
            if s < self.n && b[s].is_ascii_digit() {
                self.tok(e, e + 1, ILLEGAL_OP);
                return e + 1;
            }
        }
        let s = self.spaces(e);
        if s < self.n && matches!(b[s], b'-' | b'+') && b.get(s + 1).is_some_and(u8::is_ascii_digit) && s + 1 < self.n {
            self.tok(s, s + 1, ILLEGAL_OP);
            return s + 1;
        }
        e
    }

    /// `(([-+]?)(\d*)(n))\s*(([-+])(\s*\d+))?` at `p`.
    fn nth_sass(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let mut i = p;
        if matches!(b.get(i), Some(b'-' | b'+')) {
            i += 1;
        }
        let d = self.digits(i);
        if b.get(d) != Some(&b'n') || d >= self.n {
            return None;
        }
        self.tok(p, i, ARITHMETIC);
        self.tok(i, d, NUM_VALUE);
        self.tok(d, d + 1, NUM_SUFFIX);
        let nend = d + 1;
        let s = self.spaces(nend);
        if s < self.n && matches!(b[s], b'-' | b'+') {
            let t = self.spaces(s + 1);
            let e = self.digits(t);
            if e > t {
                self.tok(s, s + 1, ARITHMETIC);
                self.tok(s + 1, e, NUM_VALUE);
                return Some(e);
            }
        }
        Some(s.min(self.n + 1))
    }

    /// `sass-operators` at `p`.
    fn sass_operators(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let c = self.ch(p)?;
        // `/|\*|\-\-|\-|\+\+|\+|~`
        if p < self.n {
            let e = match c {
                b'/' | b'*' | b'~' => p + 1,
                b'-' | b'+' => {
                    if b.get(p + 1) == Some(&c) {
                        p + 2
                    } else {
                        p + 1
                    }
                }
                _ => 0,
            };
            if e > 0 {
                self.tok(p, e, ARITH_SASS);
                return Some(e);
            }
            for w in SASS_OP_SYMBOLS {
                if b[p..].starts_with(w) {
                    self.tok(p, p + w.len(), OP_SASS);
                    return Some(p + w.len());
                }
            }
            if self.boundary(p) {
                for w in [&b"not"[..], b"or", b"and", b"when"] {
                    let e = p + w.len();
                    if b.get(p..e) == Some(w) && self.boundary(e) {
                        self.tok(p, e, OP_SASS);
                        return Some(e);
                    }
                }
            }
        }
        // `(?:\s*)\b(not|or|and|when|from|to|through|in)(?=\s)`
        let s = self.spaces(p);
        if s < self.n && self.boundary(s) {
            for w in [
                &b"not"[..],
                b"or",
                b"and",
                b"when",
                b"from",
                b"to",
                b"through",
                b"in",
            ] {
                let e = s + w.len();
                if b.get(s..e) == Some(w) && self.space(e).is_some() {
                    self.tok(s, e, OP_SASS);
                    return Some(e);
                }
            }
        }
        None
    }

    /// `unquoted-concatenated-string` at `p`.
    fn concat(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let n = self.n;
        let run = |i: usize| self.class_run(i, 1);
        // `\s+`: where it ends, if it takes something
        let spaces1 = |i: usize| {
            let s = self.spaces(i);
            (s > i).then_some(s)
        };
        // a run followed by `\s|$|;`
        let tail = |i: usize| {
            let r = run(i);
            (r > i && (r >= n || self.space(r).is_some() || b[r] == b';')).then_some(r)
        };
        let r1 = run(p);
        if r1 > p
            && let Some(s) = spaces1(r1)
            && s < n
            && b[s] == b'+'
        {
            if let Some(t) = spaces1(s + 1)
                && t < n
                && let Some(r2) = tail(t)
            {
                self.tok(p, r1, UNQUOTED);
                self.tok(s, s + 1, OP_SASS);
                self.tok(t, r2, UNQUOTED);
                return Some(r2);
            }
            self.tok(p, r1, UNQUOTED);
            self.tok(s, s + 1, OP_SASS);
            return Some(s + 1);
        }
        if p < n
            && b[p] == b'+'
            && let Some(t) = spaces1(p + 1)
            && t < n
            && let Some(r2) = tail(t)
        {
            self.tok(p, p + 1, OP_SASS);
            self.tok(t, r2, UNQUOTED);
            return Some(r2);
        }
        None
    }

    /// `{{ident}}(\s+{{ident}})*\b(?!:)` at `p`: the idents, backtracking to an end at a `\b`
    /// that no `:` follows.
    fn font_idents(&mut self, p: usize) -> Option<usize> {
        let mut starts = vec![p];
        let mut e = self.ident_m(p)?;
        loop {
            let s = self.spaces(e);
            if s == e || s >= self.n {
                break;
            }
            match self.ident_m(s) {
                Some(f) => {
                    starts.push(s);
                    e = f;
                }
                None => break,
            }
        }
        for &s in starts.iter().rev() {
            for end in self.ident_ends(s) {
                if self.boundary(end) && self.ch(end) != Some(b':') {
                    self.tok(p, end, UNQUOTED);
                    return Some(end);
                }
            }
        }
        None
    }

    // ---- selectors ----

    /// `selectors` at `p`.
    fn selectors(&mut self, p: usize) -> Option<usize> {
        let (b, n) = (self.b, self.n);
        let s = self.spaces(p);
        if s >= n || !self.can_push() {
            return self.selectors_tail(p);
        }
        match b[s] {
            b'*' => {
                self.tok(s, s + 1, WILDCARD);
                self.push(SEL, s + 1);
                return Some(s + 1);
            }
            b'&' => {
                self.tok(s, s + 1, AMP_LEAF);
                self.push(AMP, s + 1);
                return Some(s + 1);
            }
            _ => {}
        }
        // the tags: `\s*({{element_names}})` and a lookahead
        if b[s].is_ascii_alphabetic() && self.boundary(s) {
            let e = self.bword_end(s);
            if lookup(&b[s..e]).lists & TAG != 0 && (self.pseudo_ahead(e, None) || !matches!(b.get(e), Some(b'-' | b':'))) {
                self.tok(s, e, TAG_NAME);
                self.push(SEL, e);
                return Some(e);
            }
        }
        // classes and ids: `\s*(\.)(?={{ident}}|#)`
        if matches!(b[s], b'.' | b'#') && (self.ident_at(s + 1) || b.get(s + 1) == Some(&b'#')) {
            let (mark, ctx) = if b[s] == b'.' { (CLASS_DOT, CLASS) } else { (ID_HASH, ID) };
            self.tok(s, s + 1, mark);
            self.push(ctx, s + 1);
            return Some(s + 1);
        }
        // `\s*(%){{ident}}*`
        if b[s] == b'%' {
            let e = self.ident_m(s + 1).unwrap_or(s + 1);
            self.tok(p, s, PH_HEAD);
            self.tok(s, s + 1, PH_MARK);
            self.tok(s + 1, e, PH_HEAD);
            self.push(PLACEHOLDER, e);
            return Some(e);
        }
        // `\s*(?=\[)`
        if b[s] == b'[' {
            self.push(ATTR_OUT, s);
            return Some(s);
        }
        self.selectors_tail(p)
    }

    /// The rest of `selectors`: the pseudo-elements and -classes, then the combinators at a
    /// line's start.
    fn selectors_tail(&mut self, p: usize) -> Option<usize> {
        if let Some(e) = self.pseudo_element(p) {
            return Some(e);
        }
        if let Some(e) = self.pseudo_class(p) {
            return Some(e);
        }
        if p == 0 {
            // `^\s*({{combinators}})(?![>~+])\s*`
            let s = self.spaces(0);
            if let Some(e) = self.combinator_at(s) {
                self.tok(s, e, COMBINATOR);
                return Some(self.spaces(e).min(self.n + 1));
            }
        }
        None
    }

    /// `({{combinators}})(?![>~+])` at `i`: its end.
    fn combinator_at(&self, i: usize) -> Option<usize> {
        let b = self.b;
        let comb = |c: Option<&u8>| matches!(c, Some(b'>' | b'~' | b'+'));
        match b.get(i)? {
            b'>' => {
                let mut k = i + 1;
                while k < self.n && k - i < 3 && b[k] == b'>' {
                    k += 1;
                }
                (i + 1..=k).rev().find(|&e| !comb(b.get(e)))
            }
            b'~' | b'+' => (!comb(b.get(i + 1))).then_some(i + 1),
            _ => None,
        }
    }

    /// `selector-parts`' own patterns (codes from 1000).
    fn selector_part(&mut self, op: u16, p: usize) -> Option<usize> {
        let b = self.b;
        let s = self.spaces(p);
        match op {
            1000 => {
                if self.ch(s) == Some(b',') {
                    self.tok(s, s + 1, SEP);
                    let e = self.spaces(s + 1);
                    self.pop(e);
                    return Some(e);
                }
                None
            }
            1001 => {
                let e = self.combinator_at(s)?;
                self.tok(s, e, COMBINATOR);
                let f = self.spaces(e);
                self.pop(f);
                Some(f)
            }
            1002 => {
                if self.ch(s) == Some(b'{') {
                    self.pop(s);
                    return Some(s);
                }
                None
            }
            1003 => {
                if matches!(b.get(p), Some(b'.' | b'#' | b'[' | b'&')) && p < self.n {
                    self.pop(p);
                    return Some(p);
                }
                None
            }
            1004 => {
                if s >= self.n || b[s..].starts_with(b"//") || b[s..].starts_with(b"/*") {
                    self.pop(s);
                    return Some(s);
                }
                None
            }
            _ => {
                // `\s*(?=\b)`: the spaces given back until a `\b` holds
                let mut ends = vec![p];
                let mut k = p;
                while let Some(e) = self.space(k) {
                    k = e;
                    ends.push(k);
                }
                let e = ends.into_iter().rev().find(|&e| self.boundary(e))?;
                self.pop(e);
                Some(e)
            }
        }
    }

    /// `pseudo-elements` at `p`: `\s*{{pseudo_elements}}`, its spaces in its scope.
    fn pseudo_element(&mut self, p: usize) -> Option<usize> {
        let s = self.spaces(p);
        if s >= self.n {
            return None;
        }
        let (colons, vendor, e) = self.pseudo_element_at(s)?;
        self.tok(p, s, PSEUDO_ELEMENT);
        self.tok(s, colons, PSEUDO_ELEMENT_MARK);
        let mut i = colons;
        if let Some((a, z)) = vendor {
            self.tok(a, z, PSEUDO_ELEMENT_VENDOR);
            i = z;
        }
        self.tok(i, e, PSEUDO_ELEMENT);
        Some(e)
    }

    /// `pseudo-classes` at `p`.
    fn pseudo_class(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let s = self.spaces(p);
        if s >= self.n || b[s] != b':' {
            return None;
        }
        let at = |w: &[u8]| b.get(s + 1..s + 1 + w.len()) == Some(w);
        let call = |w: &[u8]| at(w) && b.get(s + 1 + w.len()) == Some(&b'(');
        // the calls: `\s*(:)(dir|lang)(?=\()` and the like
        for (words, to, leaf, mark) in [
            (&[&b"dir"[..], b"lang"][..], PS_DIR, PSEUDO_CLASS, PSEUDO_CLASS_MARK),
            (&[&b"matches"[..], b"is", b"where", b"not", b"has"][..], PS_SEL, PSEUDO_CLASS, PSEUDO_CLASS_MARK),
            (&[&b"drop"[..]][..], PS_DROP, PSEUDO_CLASS, PSEUDO_CLASS_MARK),
            (
                &[&b"nth-last-child"[..], b"nth-child", b"nth-last-of-type", b"nth-of-type"][..],
                PS_NTH,
                PSEUDO_CLASS,
                PSEUDO_CLASS_MARK,
            ),
        ] {
            for w in words {
                if call(w) && self.can_push() {
                    let e = s + 1 + w.len();
                    self.tok(p, s, leaf);
                    self.tok(s, s + 1, mark);
                    self.tok(s + 1, e, leaf);
                    self.push(to, e);
                    return Some(e);
                }
            }
        }
        // `\s*(:)(-(moz|ms|webkit)-){{ident}}`
        if let Some((v, e)) = self.vendor_pseudo_at(s)
            && self.can_push()
        {
            self.tok(p, s, PSEUDO_CLASS);
            self.tok(s, s + 1, PSEUDO_CLASS_MARK);
            self.tok(s + 1, v, PSEUDO_VENDOR);
            self.tok(v, e, PSEUDO_CLASS);
            self.push(PS_VENDOR, e);
            return Some(e);
        }
        if let Some(e) = self.regular_pseudo_at(s) {
            self.tok(p, s, PSEUDO_CLASS);
            self.tok(s, s + 1, PSEUDO_CLASS_MARK);
            self.tok(s + 1, e, PSEUDO_CLASS);
            return Some(e);
        }
        for w in [&b"local"[..], b"global"] {
            if at(w) {
                let e = s + 1 + w.len();
                self.tok(p, s, PSEUDO_MODULE);
                self.tok(s, s + 1, PSEUDO_MODULE_MARK);
                self.tok(s + 1, e, PSEUDO_MODULE);
                if b.get(e) == Some(&b'(') && self.can_push() {
                    self.push(PS_LOCAL, e);
                }
                return Some(e);
            }
        }
        None
    }

    /// `custom-element-selectors` at `p`: each of its five patterns, the longest element that
    /// ends at a `\b` no ident follows, with that pattern's lookahead.
    fn custom_elements(&mut self, p: usize) -> Option<usize> {
        let (b, n) = (self.b, self.n);
        let s = self.spaces(p);
        if s >= n
            || !b[s].is_ascii_lowercase()
            || !self.boundary(s)
            || !self.can_push()
            || s < self.custom_failed.get()
        {
            return None;
        }
        let mut ends = Vec::new();
        let mut i = s + 1;
        let mut dash = false;
        while i < n {
            let (ch, len) = char_at(b, i);
            if !custom_char(ch) {
                break;
            }
            dash |= ch == '-';
            i = (i + len).min(n);
            if dash && self.boundary(i) && !self.ident_at(i) {
                ends.push(i);
            }
        }
        for variant in 0..5u8 {
            for &e in ends.iter().rev() {
                let ok = if variant < 4 {
                    self.pseudo_ahead(e, Some(variant))
                } else {
                    b.get(e) != Some(&b'-') && self.ch(self.spaces(e)) != Some(b':')
                };
                if ok {
                    self.tok(s, e, if variant == 3 { TAG_NAME } else { TAG_CUSTOM });
                    self.push(SEL, e);
                    return Some(e);
                }
            }
        }
        // a later start in the run has fewer ends, the same lookaheads at each: it fails too
        self.custom_failed.set(i);
        None
    }

    /// `interpolated-selectors` at `p`: `\s*(#)(?={.*}…)`, a `}` later on the line that one of
    /// the lookaheads follows.
    fn interpolated_selector(&mut self, p: usize) -> Option<usize> {
        let (b, n) = (self.b, self.n);
        let s = self.spaces(p);
        if s + 1 >= n || b[s] != b'#' || b[s + 1] != b'{' || !self.can_push() {
            return None;
        }
        // the last `}` of the line a lookahead follows, read once
        if self.isel_end.get() == usize::MAX {
            let last = (0..n)
                .rev()
                .find(|&j| {
                    b[j] == b'}'
                        && (self.pseudo_ahead(j + 1, None)
                            || !matches!(b.get(j + 1), Some(b'-' | b':')))
                })
                .unwrap_or(0);
            self.isel_end.set(last);
        }
        if self.isel_end.get() < s + 2 {
            return None;
        }
        self.tok(s, s + 1, INTERP_HASH);
        self.push(ISEL, s + 1);
        Some(s + 1)
    }

    // ---- properties and at-rules ----

    /// SCSS's `properties` at `p`.
    fn properties(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if let Some(e) = self.sop(O_COMMENT_LINE, p, MAIN, false) {
            return Some(e);
        }
        if let Some(e) = self.op(P_COMMENT, p, MAIN, false) {
            return Some(e);
        }
        if !self.can_push() {
            return None;
        }
        let c = self.ch(p)?;
        let colon_after = |e: usize| self.ch(self.spaces(e)) == Some(b':');
        if c.is_ascii_alphabetic() && self.boundary(p) {
            // `\b(font-family|font|family)\b(?=\s*:)`
            for w in [&b"font-family"[..], b"font", b"family"] {
                let e = p + w.len();
                if b.get(p..e) == Some(w) && self.boundary(e) && colon_after(e) {
                    self.tok(p, e, PROPERTY_NAME);
                    self.push(FONT_PROP, e);
                    return Some(e);
                }
            }
            // the property names, `\b(…)\b(?=\s*:)`
            let e = self.name_end(p);
            if self.boundary(e) && colon_after(e) && lookup(&b[p..e]).lists & SCSS_PROP != 0 {
                self.tok(p, e, PROPERTY_NAME);
                self.push(PVW, e);
                return Some(e);
            }
        }
        // `(-(?:webkit|moz|ms|o)-)({{ident}})(?=\s*:)`
        if c == b'-' {
            for v in [&b"-webkit-"[..], b"-moz-", b"-ms-", b"-o-"] {
                if b.get(p..p + v.len()) == Some(v)
                    && let Some(e) = self.ident_m(p + v.len())
                    && colon_after(e)
                {
                    self.tok(p, p + v.len(), super::VENDOR);
                    self.tok(p + v.len(), e, PROPERTY_NAME);
                    self.push(PVW, e);
                    return Some(e);
                }
            }
        }
        // `(\$)([a-zA-Z0-9_-][\w-]*)`
        if c == b'$'
            && let Some(e) = self.var_name(p + 1)
        {
            self.tok(p, p + 1, DECL_MARK);
            self.tok(p + 1, e, DECL);
            self.push(PVW, e);
            return Some(e);
        }
        // `(--)({{nmchar}}+)`
        if c == b'-' && b.get(p + 1) == Some(&b'-') {
            let e = self.nmchars_m(p + 2);
            if e > p + 2 {
                self.tok(p, p + 2, CUSTOM_MARK);
                self.tok(p + 2, e, CUSTOM_NAME);
                self.push(PVW, e);
                return Some(e);
            }
        }
        // the partial names, `\b(z|y|…)\b`, ranked
        if c.is_ascii_alphabetic()
            && let Some((e, _)) = self.ranked(p, |w| w.partial_rank)
        {
            self.tok(p, e, PROPERTY_NAME);
            self.push(PVW, e);
            return Some(e);
        }
        // `[a-zA-Z0-9_-]*((#)({)((\$)([a-zA-Z0-9_-][\w-]*))(}))[a-zA-Z0-9_-]*`
        let name = |i: usize| self.class_run(i, 0);
        let h = name(p);
        if b.get(h) == Some(&b'#')
            && b.get(h + 1) == Some(&b'{')
            && b.get(h + 2) == Some(&b'$')
            && let Some(v) = self.var_name(h + 3)
            && b.get(v) == Some(&b'}')
        {
            let e = name(v + 1);
            self.tok(p, h, PROP_BARE);
            self.tok(h, h + 1, PROP_BARE_HASH);
            self.tok(h + 1, h + 2, PROP_BARE_BEGIN);
            self.tok(h + 2, h + 3, PROP_BARE_VAR_MARK);
            self.tok(h + 3, v, PROP_BARE_VAR);
            self.tok(v, v + 1, PROP_BARE_END);
            self.tok(v + 1, e, PROP_BARE);
            self.push(PVW, e);
            return Some(e);
        }
        // `\b(composes)\b`
        if b.get(p..p + 8) == Some(b"composes") && self.boundary(p) && self.boundary(p + 8) {
            self.tok(p, p + 8, PROP_BARE);
            self.push(PVW, p + 8);
            return Some(p + 8);
        }
        None
    }

    /// An at-rule's `@` and name at `s..e` as `leaf` (the `@` the next leaf).
    fn at_pair(&mut self, s: usize, e: usize, leaf: u16) {
        self.tok(s, s + 1, leaf + 1);
        self.tok(s + 1, e, leaf);
    }

    /// SCSS's `at-rules` at `p`.
    fn s_at_rules(&mut self, p: usize) -> Option<usize> {
        let (b, n) = (self.b, self.n);
        let s = self.spaces(p);
        if s >= n || b[s] != b'@' || !self.can_push() {
            return None;
        }
        let word = |w: &[u8]| b.get(s + 1..s + 1 + w.len()) == Some(w) && self.boundary(s + 1 + w.len());
        if s == p {
            // the patterns with no `\s*` before them
            for (words, leaf, to) in [
                (&[&b"each"[..]][..], AT_FLOW, Some(EACH)),
                (&[&b"for"[..], b"while"][..], AT_FLOW, Some(AT_EXPR)),
                (&[&b"if"[..], b"else if", b"else"][..], AT_COND, Some(AT_EXPR)),
                (&[&b"warn"[..], b"error"][..], AT_WARN_LEAF, Some(AT_WARN)),
                (&[&b"debug"[..]][..], AT_DEBUG, Some(AT_WARN)),
                (&[&b"at-root"[..]][..], AT_ROOT, None),
                (&[&b"extend"[..]][..], AT_EXTEND, Some(EXTEND)),
            ] {
                for w in words {
                    if word(w) {
                        let e = s + 1 + w.len();
                        self.tok(s, s + 1, leaf + 1);
                        self.tok(s + 1, e, leaf);
                        if let Some(to) = to {
                            self.push(to, e);
                        }
                        return Some(e);
                    }
                }
            }
            // `((@)(mixin|function))\s+([\w-]+)`
            for w in [&b"mixin"[..], b"function"] {
                let k = s + 1 + w.len();
                if b.get(s + 1..k) == Some(w) {
                    let t = self.spaces(k);
                    if t > k && t < n {
                        let e = self.word_run(t, true);
                        if e > t {
                            self.at_pair(s, k, AT_SASS);
                            self.tok(t, e, FN_NAME);
                            self.push(MIXIN, e);
                            return Some(e);
                        }
                    }
                }
            }
            // `((@)(include))\s+(([a-z]+)(\.))?([\w-]+)`
            if b.get(s + 1..s + 8) == Some(b"include") {
                let k = s + 8;
                let t = self.spaces(k);
                if t > k && t < n {
                    let mut l = t;
                    while l < n && b[l].is_ascii_lowercase() {
                        l += 1;
                    }
                    let (ns, start) = if l > t && b.get(l) == Some(&b'.') && self.word_run(l + 1, true) > l + 1 {
                        (Some(l), l + 1)
                    } else {
                        (None, t)
                    };
                    let e = self.word_run(start, true);
                    if e > start {
                        self.tok(s, s + 1, INC_AT_MARK);
                        self.tok(s + 1, k, INC_AT);
                        self.tok(k, t, INC);
                        if let Some(l) = ns {
                            self.tok(t, l, INC_NS);
                            self.tok(l, l + 1, INC_NS_DOT);
                        }
                        self.tok(start, e, INC);
                        self.push(INCLUDE, e);
                        return Some(e);
                    }
                }
            }
            if word(b"return") {
                self.at_pair(s, s + 7, AT_RETURN);
                self.push(RETURN, s + 7);
                return Some(s + 7);
            }
        }
        // `\s*((@)(use|forward)\b)\s*`
        for w in [&b"use"[..], b"forward"] {
            if word(w) {
                let e = s + 1 + w.len();
                self.at_pair(s, e, AT_MODULE);
                let f = self.spaces(e).min(n + 1);
                self.push(USE, f);
                return Some(f);
            }
        }
        if s == p {
            // `((@)(?:-(?:webkit|moz|o)-)?(charset|namespace|font-face)\b)`
            let mut k = s + 1;
            for v in [&b"-webkit-"[..], b"-moz-", b"-o-"] {
                if b.get(k..k + v.len()) == Some(v) {
                    k += v.len();
                    break;
                }
            }
            for w in [&b"charset"[..], b"namespace", b"font-face"] {
                let e = k + w.len();
                if b.get(k..e) == Some(w) && self.boundary(e) {
                    self.at_pair(s, e, AT_CSS_LEAF);
                    self.push(AT_CSS, e);
                    return Some(e);
                }
            }
        }
        // `\s*((@)counter-style\b)\s+(?:(?i:\b(decimal|none)\b)|({{ident}}))?\s*(?=\{|$)`
        if word(b"counter-style") {
            let k = s + 14;
            let t = self.spaces(k);
            if t > k {
                let ahead = |i: usize| {
                    let j = self.spaces(i);
                    j >= n || b[j] == b'{'
                };
                let mut name = None;
                for w in [&b"decimal"[..], b"none"] {
                    let e = t + w.len();
                    if b.get(t..e).is_some_and(|x| x.eq_ignore_ascii_case(w)) && self.boundary(e) && ahead(e) {
                        name = Some((e, COUNTER_STYLE_INVALID));
                        break;
                    }
                }
                if name.is_none()
                    && t < n
                    && let Some(e) = self.ident_m(t)
                    && ahead(e)
                {
                    name = Some((e, COUNTER_STYLE_NAME));
                }
                if name.is_some() || ahead(t) {
                    self.at_pair(s, k, AT_COUNTER_STYLE);
                    let mut e = t;
                    if let Some((f, leaf)) = name {
                        self.tok(t, f, leaf);
                        e = f;
                    }
                    let e = self.spaces(e).min(n);
                    let e = e.max(t.min(n));
                    self.push(COUNTER_STYLE, e);
                    return Some(e);
                }
            }
        }
        // `(?=\s*@custom-media\b)`
        if word(b"custom-media") {
            self.push(CM_OUT, p);
            return Some(p);
        }
        if s == p && b.get(s + 1..s + 9) == Some(b"document") {
            self.at_pair(s, s + 9, AT_DOCUMENT);
            self.push(DOCUMENT, s + 9);
            return Some(s + 9);
        }
        if word(b"import") {
            let e = s + 7;
            self.at_pair(s, e, AT_IMPORT);
            let f = self.spaces(e).min(n + 1);
            self.push(IMPORT, f);
            return Some(f);
        }
        // `\s*((@)(-webkit-|-moz-|-o-)?keyframes)`
        let mut k = s + 1;
        for v in [&b"-webkit-"[..], b"-moz-", b"-o-"] {
            if b.get(k..k + v.len()) == Some(v) {
                k += v.len();
                break;
            }
        }
        if b.get(k..k + 9) == Some(b"keyframes") {
            self.at_pair(s, k + 9, AT_KEYFRAME);
            self.push(KEYFRAMES, k + 9);
            return Some(k + 9);
        }
        if word(b"media") {
            self.at_pair(s, s + 6, AT_MEDIA);
            self.push(MEDIA, s + 6);
            return Some(s + 6);
        }
        // `\s*((@)namespace)\s+({{ident}}(?!{{nmchar}}|\())?`
        if b.get(s + 1..s + 10) == Some(b"namespace") {
            let k = s + 10;
            let t = self.spaces(k);
            if t > k {
                self.at_pair(s, k, AT_NAMESPACE);
                let mut e = t;
                if t < n
                    && let Some(f) = self.ident_m(t)
                    && b.get(f) != Some(&b'(')
                {
                    self.tok(t, f, NS_PREFIX);
                    e = f;
                }
                let e = e.min(n + 1);
                self.push(NAMESPACE, e);
                return Some(e);
            }
        }
        // `\s*((@)page)\s*(?:(:)(first|left|right))?\s*`
        if b.get(s + 1..s + 5) == Some(b"page") {
            let k = s + 5;
            self.at_pair(s, k, AT_PAGE);
            let mut t = self.spaces(k);
            if t < n && b[t] == b':' {
                for w in [&b"first"[..], b"left", b"right"] {
                    if b.get(t + 1..t + 1 + w.len()) == Some(w) {
                        self.tok(t, t + 1, ENTITY_MARK);
                        self.tok(t + 1, t + 1 + w.len(), PSEUDO_CLASS);
                        t = self.spaces(t + 1 + w.len());
                        break;
                    }
                }
            }
            let t = t.min(n + 1);
            self.push(PAGE, t);
            return Some(t);
        }
        if s == p && b.get(s + 1..s + 9) == Some(b"supports") {
            self.at_pair(s, s + 9, AT_SUPPORTS);
            self.push(SUPPORTS, s + 9);
            return Some(s + 9);
        }
        if s == p && b.get(s + 1..s + 8) == Some(b"content") {
            self.at_pair(s, s + 8, AT_CONTENT);
            self.push(CONTENT, s + 8);
            return Some(s + 8);
        }
        None
    }

    /// SCSS's media features at `p`: `(…)|grid|scan|orientation\s*(?=[:)])` into its end.
    fn media_feature_scss(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let at = |i: usize, w: &[u8]| b.get(i..i + w.len()) == Some(w);
        let mut best: Option<Feature> = None;
        'first: for v1 in [Some(&b"-webkit-"[..]), Some(b"-o-"), None] {
            let mut i = p;
            let mut vend1 = None;
            if let Some(v) = v1 {
                if !at(i, v) {
                    continue;
                }
                vend1 = Some((i, i + v.len()));
                i += v.len();
            }
            for mm in [Some(&b"min-"[..]), Some(b"max-"), None] {
                let mut j = i;
                if let Some(m) = mm {
                    if !at(j, m) {
                        continue;
                    }
                    j += m.len();
                }
                for v3 in [Some(&b"-moz-"[..]), None] {
                    let mut k = j;
                    let mut vend3 = None;
                    if let Some(v) = v3 {
                        if !at(k, v) {
                            continue;
                        }
                        vend3 = Some((k, k + v.len()));
                        k += v.len();
                    }
                    for dv in [Some(&b"device-"[..]), None] {
                        let mut l = k;
                        if let Some(d) = dv {
                            if !at(l, d) {
                                continue;
                            }
                            l += d.len();
                        }
                        for w in [&b"height"[..], b"width", b"aspect-ratio", b"pixel-ratio"] {
                            if at(l, w) {
                                best = Some((l + w.len(), vend1, vend3));
                                break 'first;
                            }
                        }
                    }
                    for w in [&b"color-index"[..], b"color", b"monochrome", b"resolution"] {
                        if at(k, w) {
                            best = Some((k + w.len(), vend1, vend3));
                            break 'first;
                        }
                    }
                }
            }
        }
        if best.is_none() {
            for w in [&b"grid"[..], b"scan"] {
                if at(p, w) {
                    best = Some((p + w.len(), None, None));
                    break;
                }
            }
        }
        if best.is_none() && at(p, b"orientation") {
            let e = p + 11;
            let s = self.spaces(e);
            if matches!(self.ch(s), Some(b':' | b')')) {
                best = Some((s, None, None));
            }
        }
        let (e, v1, v3) = best?;
        if !self.can_push() {
            return None;
        }
        let mut i = p;
        for (a, z) in [v1, v3].into_iter().flatten() {
            self.tok(i, a, MEDIA_FEATURE);
            self.tok(a, z, MEDIA_FEATURE_VENDOR);
            i = z;
        }
        self.tok(i, e, MEDIA_FEATURE);
        self.push(MF_AFTER, e);
        Some(e)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Lang, State, lex};

    fn kinds(lines: &[&'static str]) -> Vec<(&'static str, String)> {
        let mut st = State::default();
        let mut spans = Vec::new();
        let mut out = Vec::new();
        for l in lines {
            lex(Lang::Scss, l, &mut st, &mut spans);
            for s in &spans {
                let scopes = Lang::Scss.scopes(s.kind).join(" ");
                out.push((&l[s.start as usize..s.end as usize], scopes));
            }
        }
        out
    }

    #[test]
    fn a_rule_reads_as_bats_grammar_does() {
        let k = kinds(&[".btn-#{$x} { color: $red; }"]);
        let find = |t: &str| k.iter().find(|(x, _)| *x == t).map(|(_, s)| s.clone());
        assert_eq!(find("btn-").as_deref(), Some("entity.other.attribute-name.class.css"));
        assert_eq!(find("color").as_deref(), Some("support.type.property-name.css"));
        assert_eq!(find("red").as_deref(), Some("variable.other.sass"));
    }

    #[test]
    fn an_interpolation_in_a_string_is_the_strings() {
        let k = kinds(&["a { content: \"#{$x}\"; }"]);
        let x = k.iter().find(|(t, _)| *t == "x").unwrap();
        assert_eq!(x.1, "string.quoted.double.css variable.other.sass");
    }

    #[test]
    fn long_lines_stay_linear() {
        for unit in ["a-", "$a ", "#{", "(", "a:", "1px+"] {
            let line: &'static str = Box::leak(unit.repeat(4000).into_boxed_str());
            let t = std::time::Instant::now();
            let _ = kinds(&[line]);
            assert!(t.elapsed().as_secs_f64() < 2.0, "{unit}");
        }
    }
}
