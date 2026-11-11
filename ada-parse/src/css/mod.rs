//! CSS's scanner is [`own`], one of ada's own from CSS's syntax (X-125, 2026-11-11). This module is
//! the engine bat's `CSS` grammar was compiled into before it, kept as **SCSS's** ([`scss`], a
//! dialect on it, `Scan<S = true>`): its readers, its word table and the CSS ops SCSS's contexts
//! name. Its own CSS paths (the line API, the block, HTML's entry, CSS's run loop and op lists)
//! went with X-125's R4; what follows describes the engine as SCSS uses it.
//!
//! bat's grammar (Packages at `759d6ee`, 216 contexts as syntect compiles it) is mostly word
//! lists: property names, value constants, colors, tags, and ~70 functions, each with a group of
//! its own whose patterns differ (`translateX(` takes lengths and numbers, `rgb(` percentages
//! and numbers). The scanner keeps each context's patterns in the grammar's order as a list of
//! ops and tries them **at the place the scan has reached**, which is syntect's
//! leftmost match; a word is read once and looked up once in one table ([`words`]) that names
//! every list it is in, with its rank in the lists where a regex can stop at a `-` (`auto` in
//! `auto-fill`: `\b(…|auto(hiding-scrollbar|-flow)?|…)\b` stops at the first alternative that
//! ends at a `\b`).
//!
//! What syntect's machinery decides is followed: the line ends in a `\n` every pattern sees
//! (position `n`), and the search goes on past it (`n + 1`), where only empty matches can hold
//! (`$`, `''`, a lookahead's pop); a pattern whose leftmost match is empty and does nothing counts
//! only where a search starts (`(?:\s+([iI]))?` in an attribute selector); `^` only at a line's
//! start. Nesting has no limit of syntect's: a push past [`MAX`] levels drops the bottom one
//! (X-133).
//!
//! A kind is a leaf, bat's scopes for the token, under a **wrap**: the comment or string the token
//! sits in. Meta scopes are left out (no theme selects one of CSS's). The stack between lines is
//! the contexts open above `main`, up to 28 (the bytes of [`State`]'s levels).
//!
//! `\w` past ASCII is a letter or a decimal digit, `{{nonascii}}` (`[\p{L}\p{M}\p{S}\p{N}]`) any
//! character past ASCII but spaces, controls and punctuation; `(?i)` folds ASCII case only.

pub(crate) mod own;
pub(crate) mod scss;
mod words;

use crate::emit::{Push, Sink};
use crate::words::{Words, text_len};
use crate::{Out, State};
use words::{ENTRIES, LONGEST};

// ---- the contexts ----

const MAIN: u8 = 0;
/// `/* … */`.
const COMMENT: u8 = 1;
/// `'…'`.
const STR_S: u8 = 2;
/// `"…"`.
const STR_D: u8 = 3;
/// `selector`'s push, to `/`, `@`, `{` or `)`.
const SELECTOR: u8 = 4;
/// `[` … `]` in a selector.
const ATTR_SEL: u8 = 5;
/// An attribute selector's value, after its operator.
const ATTR_VAL: u8 = 6;
/// `property-list`'s `(?=\{)`: its `}` and `rule-list`.
const PL: u8 = 7;
/// `rule-list`'s `{`: `meta.property-list`, to before a `}`.
const PL_BODY: u8 = 8;
/// `meta.property-name`, over `[-a-z]`.
const PROP_NAME: u8 = 9;
/// After `font` or `font-family`: its `:` or a pop.
const FONT_AFTER: u8 = 10;
/// `font`'s value, which takes unquoted names.
const FONT_VALUE: u8 = 11;
/// A property's value.
const PROP_VALUE: u8 = 12;
/// `line-names`, `[` … `]`.
const LINE_NAMES: u8 = 13;
/// `calc`'s function call: its `(` or a pop.
const CALC_W: u8 = 14;
/// `inside-calc-parens`.
const CALC_PARENS: u8 = 15;
/// `function-notation-terminator` alone, set by `inside-calc-parens` at its `)`.
const TERMINATOR: u8 = 16;
/// `counter()`'s rest after a `,`.
const COUNTER_REST: u8 = 17;
/// `counters()`'s rest after a `,`.
const COUNTERS_REST: u8 = 18;
/// `attr()`'s rest after the attribute's name.
const ATTR_FN_REST: u8 = 19;
/// `@charset`.
const CHARSET: u8 = 20;
/// `@counter-style`, `@font-face` and `@page`: a rule list.
const RULE_AT: u8 = 21;
/// `@custom-media`'s lookahead push, to its `;`.
const CM_OUT: u8 = 22;
/// `@custom-media`, to before a `;`.
const CM: u8 = 23;
/// `@document`.
const DOCUMENT: u8 = 24;
/// `@document`'s and `@supports`' block, to before a `}`.
const BLOCK: u8 = 25;
/// `@import`.
const IMPORT: u8 = 26;
/// `@namespace`.
const NAMESPACE: u8 = 27;
/// `@keyframes`' lookahead push, to its `}`.
const KF_OUT: u8 = 28;
/// `@keyframes`' prelude.
const KF_HEAD: u8 = 29;
/// The names in `@keyframes`' prelude.
const KF_NAMES: u8 = 30;
/// After a keyframe name, to its `,` or before a `{` or `;`.
const KF_NAME_END: u8 = 31;
/// `@keyframes`' block, to before a `}`.
const KF_BODY: u8 = 32;
/// `@media`'s lookahead push, to its `}`.
const MEDIA_OUT: u8 = 33;
/// `@media`'s prelude.
const MEDIA_HEAD: u8 = 34;
/// `media-query-list`'s push, to before a `{` or `;`.
const MQL: u8 = 35;
/// A media query's `( … )`.
const MQ_PAREN: u8 = 36;
/// `@media`'s block, to before a `}`.
const MEDIA_BODY: u8 = 37;
/// `@supports`.
const SUPPORTS: u8 = 38;
/// `@supports`' `( … )`.
const SUP_PAREN: u8 = 39;

// A function's group (`meta.group`, its arguments) is a context from 64; the function call
// around it (`meta.function-call`: its `(` and `)`) is the group's id with `WRAP` set.
const WRAP: u8 = 0x80;
/// `:dir(`, `:lang(`.
const G_DIR_LANG: u8 = 64;
/// `:matches(`, `:not(`, `:has(`.
const G_SEL: u8 = 65;
/// `:drop(`.
const G_DROP: u8 = 66;
/// `:nth-child(` and the like.
const G_NTH: u8 = 67;
/// `filter(`.
const G_FILTER: u8 = 68;
const G_COUNTER: u8 = 69;
const G_COUNTERS: u8 = 70;
const G_SYMBOLS: u8 = 71;
/// `format(`, `regexp(`: strings.
const G_STRINGS: u8 = 72;
/// `local(`, `url(`, `url-prefix(`, `domain(`: strings and unquoted text.
const G_URL: u8 = 73;
/// `matrix3d(`, `scale3d(`, `matrix(`, `scale(`.
const G_MATRIX: u8 = 74;
/// `translate(`, `translate3d(`.
const G_TRANSLATE: u8 = 75;
/// `translateX(`, `translateY(`.
const G_TRANSLATE_XY: u8 = 76;
/// `rotate(`, `rotateX(` …, `skewX(`, `skewY(`, `hue-rotate(`: an angle.
const G_ANGLE: u8 = 77;
const G_SKEW: u8 = 78;
/// `translateZ(`, `perspective(`, `blur(`: a length.
const G_LENGTH: u8 = 79;
const G_ROTATE3D: u8 = 80;
/// `scaleX(` …, `cubic-bezier(`: numbers.
const G_NUMBERS: u8 = 81;
const G_STEPS: u8 = 82;
const G_RECT: u8 = 83;
const G_INSET: u8 = 84;
/// `circle(`, `ellipse(`.
const G_CIRCLE: u8 = 85;
const G_POLYGON: u8 = 86;
const G_TOGGLE: u8 = 87;
const G_REPEAT: u8 = 88;
const G_VAR: u8 = 89;
/// `brightness(`, `contrast(`, `grayscale(`, `invert(`, `opacity(`, `saturate(`, `sepia(`.
const G_FILTER_AMOUNT: u8 = 90;
const G_DROP_SHADOW: u8 = 91;
const G_ATTR: u8 = 92;
const G_IMAGE: u8 = 93;
const G_IMAGE_SET: u8 = 94;
/// `linear-gradient(`, `repeating-linear-gradient(`.
const G_LINEAR: u8 = 95;
/// `radial-gradient(`, `repeating-radial-gradient(`.
const G_RADIAL: u8 = 96;
const G_CROSS_FADE: u8 = 97;
const G_MINMAX: u8 = 98;
/// `rgb(`, `rgba(`, `gray(`.
const G_RGB: u8 = 99;
/// `hsl(`, `hsla(`, `hwb(`.
const G_HSL: u8 = 100;
const G_CMYK: u8 = 101;
/// `color(`.
const G_COLOR: u8 = 102;
/// `red(`, `green(`, `blue(`, `alpha(`, `a(`.
const G_ADJ_CHANNEL: u8 = 103;
/// `hue(`, `h(`.
const G_ADJ_HUE: u8 = 104;
/// `saturation(`, `lightness(`, `whiteness(`, `blackness(`, `s(`, `l(`, `w(`, `b(`.
const G_ADJ_PERCENT: u8 = 105;
/// `tint(`, `shade(`.
const G_TINT: u8 = 106;
/// `blend(`, `blenda(`.
const G_BLEND: u8 = 107;

// ---- the ops: one of bat's patterns (or an include of several), tried at a place ----

/// `/*`, `comment-block`.
const P_COMMENT: u8 = 0;
/// `(?=[:.*#a-zA-Z\[])` into a selector.
const P_SELECTOR: u8 = 1;
/// The eleven at-rules' first patterns.
const P_AT_RULES: u8 = 2;
/// `(?=\{)` into `property-list`.
const P_PL: u8 = 3;
/// `rule-list`'s `\{`.
const P_RULE_LIST: u8 = 4;
/// `rule-list-terminator`, `\s*(\})`, a pop.
const P_RULE_LIST_END: u8 = 5;
/// `property-list`'s `\}`, a pop.
const P_PL_END: u8 = 6;
/// `(?=\s*\})`, a pop.
const P_PL_BODY_END: u8 = 7;
/// `(?=[-a-z])` into a property's name.
const P_PROP_NAME: u8 = 8;
/// `(:)([ \t]*)` into a property's value.
const P_VALUE: u8 = 9;
/// `\s*(;)|(?=[})])`, a pop.
const P_VALUE_END: u8 = 10;
/// `-(?:webkit|moz|ms|o)-`.
const P_VENDOR: u8 = 11;
/// `builtin-functions`.
const P_BUILTIN_FNS: u8 = 12;
/// `line-names`' `\[`.
const P_LINE_NAMES: u8 = 13;
/// `unicode-range`.
const P_UNICODE_RANGE: u8 = 14;
/// `numeric-values`.
const P_NUMERIC: u8 = 15;
/// `color-values`.
const P_COLOR_VALUES: u8 = 16;
/// `property-value-constants`.
const P_CONSTANTS: u8 = 17;
/// `literal-string`.
const P_STRING: u8 = 18;
/// `\!\s*important`.
const P_IMPORTANT: u8 = 19;
/// `comma-delimiter`.
const P_COMMA: u8 = 20;
/// `{{ident}}(\s+{{ident}})*`, `font`'s unquoted names.
const P_IDENT_RUN: u8 = 21;
/// `$|(?![-a-z])`, a pop.
const P_PROP_NAME_END: u8 = 22;
/// `\b(var-)({{ident}})(?=\s)`.
const P_VAR_DEPRECATED: u8 = 23;
/// `custom-property-name`.
const P_CUSTOM_PROP: u8 = 24;
/// `\bfont(-family)?(?!-)\b`, a push.
const P_FONT: u8 = 25;
/// The property names.
const P_PROP_NAMES: u8 = 26;
/// `font`'s `(:)([ \t]*)` into its value.
const P_FONT_VALUE: u8 = 27;
/// `''`, a pop.
const P_POP_EMPTY: u8 = 28;
/// `(?=[/@{)])`, a pop.
const P_SELECTOR_END: u8 = 29;
/// Custom elements: `\b([a-z](?:…)*-(?:…)*)\b`.
const P_CUSTOM_ELEMENT: u8 = 30;
/// The tags.
const P_TAG: u8 = 31;
/// `(\.){{ident}}`.
const P_CLASS: u8 = 32;
/// `(#){{ident}}`.
const P_ID: u8 = 33;
/// `\*`.
const P_WILDCARD: u8 = 34;
/// `({{combinators}})(?![>~+])`, then `({{combinators}}){2,}`.
const P_COMBINATOR: u8 = 35;
/// `pseudo-elements`, then `pseudo-classes`.
const P_PSEUDO: u8 = 36;
/// `\[` into an attribute selector.
const P_ATTR_SEL: u8 = 37;
/// `qualified-name`.
const P_QUALIFIED: u8 = 38;
/// An attribute selector's `({{ident}})`.
const P_ATTR_NAME: u8 = 39;
/// `\s*([~*|^$]?=)\s*` into the value.
const P_ATTR_OP: u8 = 40;
/// `(?:\s+([iI]))?`, where a search starts.
const P_CASE_FLAG: u8 = 41;
/// `\]`, a pop.
const P_ATTR_SEL_END: u8 = 42;
/// `[^\s\]\[''"]`.
const P_ATTR_UNQUOTED: u8 = 43;
/// `(?=(\s|\]))`, a pop.
const P_ATTR_VAL_END: u8 = 44;
/// `(?=\))`, a pop.
const P_GROUP_END: u8 = 45;
/// `function-notation-terminator`, `\)`, a pop.
const P_CALL_END: u8 = 46;
/// A function call's `\(` into its group.
const P_GROUP: u8 = 47;
/// `unquoted-string`, `[^\s'"]`.
const P_UNQUOTED: u8 = 48;
/// `filter-functions`.
const P_FILTER_FNS: u8 = 49;
/// `image-type`.
const P_IMAGE_TYPE: u8 = 50;
/// `var-function`.
const P_VAR_FN: u8 = 51;
/// `calc-function`.
const P_CALC_FN: u8 = 52;
/// `attr-function`.
const P_ATTR_FN: u8 = 53;
/// `minmax-function`.
const P_MINMAX_FN: u8 = 54;
/// `url-function`.
const P_URL_FN: u8 = 55;
/// `@document`'s `url-prefix-function`, `domain-function`, `regexp-function`.
const P_DOC_FNS: u8 = 56;
/// `color-adjuster-functions`.
const P_ADJ_FNS: u8 = 57;
/// `color-adjuster-operators`, `[\-\+*](?=\s+)`.
const P_ADJ_OPS: u8 = 58;
const P_PERCENT: u8 = 59;
const P_LENGTH: u8 = 60;
const P_ANGLE: u8 = 61;
const P_RESOLUTION: u8 = 62;
/// `number-type`.
const P_NUMBER: u8 = 63;
/// `integer-type`.
const P_INTEGER: u8 = 64;
/// `\b(even|odd)\b`.
const P_EVEN_ODD: u8 = 65;
/// `([-+]?\d*(n))\s*([-+]\s*\d+)?`.
const P_NTH: u8 = 66;
/// `[-+]\s+\d+n?|[-+]?\d+\s+n`.
const P_NTH_INVALID: u8 = 67;
/// `\b(active|valid|invalid)\b`.
const P_DROP_WORDS: u8 = 68;
/// A counter's name, `({{ident}})`.
const P_COUNTER_NAME: u8 = 69;
/// `(?=,)` into the rest of `counter(`/`counters(`.
const P_COUNTER_COMMA: u8 = 70;
/// `\b({{counter_styles}}|none)\b`.
const P_COUNTER_STYLES: u8 = 71;
/// `\b(cyclic|numeric|alphabetic|symbolic|fixed)\b`.
const P_SYMBOL_TYPES: u8 = 72;
/// `(end|middle|start)`.
const P_STEPS_WORDS: u8 = 73;
/// `\bauto\b`.
const P_AUTO: u8 = 74;
/// `\bround\b`.
const P_ROUND: u8 = 75;
/// `\bat\b`.
const P_AT: u8 = 76;
/// `\b(top|right|bottom|left|center|closest-side|farthest-side)\b`.
const P_CIRCLE_PLACES: u8 = 77;
/// `\b(nonzero|evenodd)\b`.
const P_FILL_RULES: u8 = 78;
/// `\b(auto-fill|auto-fit)\b`.
const P_REPETITIONS: u8 = 79;
/// `\b(max-content|min-content|auto)\b` and `\b(auto|max-content|min-content)\b`.
const P_SIZES: u8 = 80;
/// `\bto\b`.
const P_TO: u8 = 81;
/// `\b(top|right|bottom|left)\b`.
const P_SIDES: u8 = 82;
/// `\b(at|circle|ellipse)\b`.
const P_RADIAL_WORDS: u8 = 83;
/// `\b(left|center|right|top|bottom|closest-corner|…)\b`.
const P_RADIAL_PLACES: u8 = 84;
/// `\b(rgb|hsl|hwb)\b`.
const P_COLOR_SPACES: u8 = 85;
/// `[0-9]+(x)`.
const P_DENSITY: u8 = 86;
/// `attr(`'s name, `({{ident}})`, into its rest.
const P_ATTR_FN_NAME: u8 = 87;
/// `attr(`'s units.
const P_UNITS: u8 = 88;
/// `(?=\))` setting `function-notation-terminator`.
const P_CALC_END: u8 = 89;
/// `[-/*+]`.
const P_CALC_OP: u8 = 90;
/// `\(` into `inside-calc-parens`.
const P_CALC_PAREN: u8 = 91;
/// `selector` in `:not(` and the like.
const P_SELECTOR_IN: u8 = 92;
/// `{{ident}}` in `line-names`.
const P_LINE_NAME: u8 = 93;
/// `\]`, a pop of `line-names`.
const P_LINE_NAMES_END: u8 = 94;
/// `\;`.
const P_SEMI: u8 = 95;
/// `(?=;|$)`, a pop.
const P_SEMI_END: u8 = 96;
/// `@custom-media`'s `;`, a pop.
const P_CM_END: u8 = 97;
/// `\s*((@)custom-media)` into it.
const P_CM_KEYWORD: u8 = 98;
/// `\s*(?=;)`, a pop.
const P_CM_INNER_END: u8 = 99;
/// `media-query-list`'s `\s*(?=[^{;])`.
const P_MQL: u8 = 100;
/// `\{` into a block.
const P_BLOCK: u8 = 101;
/// `\}`, the block's end, a pop.
const P_BLOCK_END: u8 = 102;
/// `(?=\})`, a pop.
const P_BRACE_AHEAD: u8 = 103;
/// `\s*((@)(-webkit-|-moz-|-o-)?keyframes)` into the prelude.
const P_KF_KEYWORD: u8 = 104;
/// `\s*(\{)` into a block (`@keyframes`', `@media`'s).
const P_AT_BLOCK: u8 = 105;
/// `\s*(?=\{)`, a pop.
const P_SPACES_BRACE: u8 = 106;
/// `\s*(?=[^{;])` into the keyframes' names.
const P_KF_NAMES: u8 = 107;
/// `\s*(?=[{;])`, a pop.
const P_SPACES_BRACE_SEMI: u8 = 108;
/// `keyframe-name`: `\s*({{ident}})?` into its end.
const P_KF_NAME: u8 = 109;
/// `\s*(?:(,)|(?=[{;]))`, a pop.
const P_KF_NAME_END: u8 = 110;
/// `\b(from|to)\b`.
const P_FROM_TO: u8 = 111;
/// `\s*((@)media)` into the prelude.
const P_MEDIA_KEYWORD: u8 = 112;
/// The media types.
const P_MEDIA_TYPES: u8 = 113;
/// `\b(?i:and|or|not|only)\b`.
const P_MEDIA_LOGIC: u8 = 114;
/// A media query's `,`.
const P_MQ_COMMA: u8 = 115;
/// `\(` into a media query's parentheses.
const P_MQ_PAREN: u8 = 116;
/// `\)`, a pop.
const P_PAREN_END: u8 = 117;
/// The media features.
const P_MEDIA_FEATURE: u8 = 118;
/// `\b(portrait|landscape|progressive|interlace)\b`.
const P_ORIENTATION: u8 = 119;
/// `\s*(\d+)(/)(\d+)`.
const P_RATIO: u8 = 120;
/// `\b(?i:and|or|not)\b`.
const P_SUPPORTS_LOGIC: u8 = 121;
/// `\(` into `@supports`' parentheses.
const P_SUP_PAREN: u8 = 122;

// ---- the gates: where each op can match (X-031) ----

// A place's class, one bit: its byte's, or the line's `\n` at `n`, or past it.

// ---- the wraps and the leaves ----

const W_NONE: u8 = 0;
const W_COMMENT: u8 = 1;
const W_SINGLE: u8 = 2;
const W_DOUBLE: u8 = 3;
/// SCSS's `//`.
const W_LINE: u8 = 4;

/// Each wrap's scope.
static WRAPS: [&str; 5] = [
    "",
    "comment.block.css",
    "string.quoted.single.css",
    "string.quoted.double.css",
    "comment.line.double-slash.sass",
];

/// The wrap a context gives the tokens in it.
#[inline(always)]
fn own_wrap(ctx: u8) -> u8 {
    OWN_WRAPS[usize::from(ctx)]
}

/// [`own_wrap`] by context: one load where the stack changes.
static OWN_WRAPS: [u8; 256] = {
    let mut t = [W_NONE; 256];
    t[COMMENT as usize] = W_COMMENT;
    t[STR_S as usize] = W_SINGLE;
    t[STR_D as usize] = W_DOUBLE;
    t[scss::LINE as usize] = W_LINE;
    t
};

/// A kind is a leaf in its low bits, the wrap above them.
const LEAF_BITS: u16 = 8;

// The leaves: an index into [`LEAVES`].
const PLAIN: u16 = 0;
const COMMENT_MARK: u16 = 1;
const STRING_BEGIN: u16 = 2;
const STRING_END: u16 = 3;
const ESCAPE: u16 = 4;
const ESCAPE_NEWLINE: u16 = 5;
const TAG_CUSTOM: u16 = 6;
const TAG_NAME: u16 = 7;
const CLASS_DOT: u16 = 8;
const CLASS_NAME: u16 = 9;
const ID_HASH: u16 = 10;
const ID_NAME: u16 = 11;
const WILDCARD: u16 = 12;
const COMBINATOR: u16 = 13;
const BAD_COMBINATOR: u16 = 14;
const PSEUDO_ELEMENT_MARK: u16 = 15;
const PSEUDO_ELEMENT: u16 = 16;
const PSEUDO_ELEMENT_VENDOR: u16 = 17;
const PSEUDO_CLASS_MARK: u16 = 18;
const PSEUDO_CLASS: u16 = 19;
const ENTITY_MARK: u16 = 20;
const ATTR_NAME: u16 = 21;
const ATTR_OP: u16 = 22;
const UNQUOTED: u16 = 23;
const KEYWORD_OTHER: u16 = 24;
const COMMA: u16 = 25;
const NS_PREFIX: u16 = 26;
const NS_WILDCARD: u16 = 27;
const NS_SEPARATOR: u16 = 28;
const GROUP_BEGIN: u16 = 29;
const CALL_END: u16 = 30;
const GROUP_END: u16 = 31;
const PSEUDO_KEYWORD: u16 = 32;
const INTEGER: u16 = 33;
const INTEGER_UNIT: u16 = 34;
const FLOAT: u16 = 35;
const FLOAT_DOT: u16 = 36;
const FLOAT_UNIT: u16 = 37;
const BAD_NUMBER: u16 = 38;
const PL_MARK: u16 = 39;
const KEY_VALUE: u16 = 40;
const RULE_END: u16 = 41;
const VENDOR: u16 = 42;
const DEPRECATED_PREFIX: u16 = 43;
const DEPRECATED_NAME: u16 = 44;
const CUSTOM_MARK: u16 = 45;
const CUSTOM_NAME: u16 = 46;
const PROPERTY_NAME: u16 = 47;
const CONSTANT: u16 = 48;
const FONT_NAME_LEAF: u16 = 49;
const STD_COLOR_LEAF: u16 = 50;
const EXT_COLOR_LEAF: u16 = 51;
const SPECIAL_COLOR_LEAF: u16 = 52;
const HEX_MARK: u16 = 53;
const HEX: u16 = 54;
const HEXA_MARK: u16 = 55;
const HEXA: u16 = 56;
const IMPORTANT: u16 = 57;
const LINE_NAMES_BEGIN: u16 = 58;
const LINE_NAMES_END: u16 = 59;
const LINE_NAME: u16 = 60;
const RANGE_PREFIX: u16 = 61;
const RANGE: u16 = 62;
const RANGE_DASH: u16 = 63;
const OPERATOR: u16 = 64;
const COUNTER_NAME: u16 = 65;
const COUNTER_STYLE_LEAF: u16 = 66;
const SYMBOL_TYPE: u16 = 67;
const TIMING_DIRECTION: u16 = 68;
const REPETITIONS: u16 = 69;
const COLOR_SPACE: u16 = 70;
const UNIT: u16 = 71;
const COUNTER_STYLE_INVALID: u16 = 72;
const COUNTER_STYLE_NAME: u16 = 73;
const TERMINATOR_LEAF: u16 = 74;
const BLOCK_BEGIN: u16 = 75;
const BLOCK_END: u16 = 76;
const ANIMATION_NAME: u16 = 77;
const REPETITION: u16 = 78;
const KF_VENDOR: u16 = 79;
const KF_SELECTOR: u16 = 80;
const MEDIA_TYPE: u16 = 81;
const MEDIA_LOGIC: u16 = 82;
const MEDIA_FEATURE: u16 = 83;
const MEDIA_FEATURE_VENDOR: u16 = 84;
const ARITHMETIC: u16 = 85;
const LOGIC: u16 = 86;
/// The functions' names: `support.function.X.css`, from `FN_ATTR` in [`FN_SCOPES`]' order.
const FN_FIRST: u16 = 87;
const FN_ATTR: u16 = 87;
const FN_CALC: u16 = 88;
const FN_IMAGE: u16 = 89;
const FN_FILTER: u16 = 90;
const FN_GRADIENT: u16 = 91;
const FN_GRID: u16 = 92;
const FN_URL: u16 = 93;
const FN_VAR: u16 = 94;
const FN_COLOR: u16 = 95;
const FN_COUNTER: u16 = 96;
const FN_FONT_FACE: u16 = 97;
const FN_TRANSFORM: u16 = 98;
const FN_TIMING: u16 = 99;
const FN_SHAPE: u16 = 100;
const FN_TOGGLE: u16 = 101;
const FN_URL_PREFIX: u16 = 102;
const FN_DOMAIN: u16 = 103;
const FN_REGEXP: u16 = 104;
/// The at-rules: `keyword.control.at-rule.X.css` and its `@`, two leaves each from `AT_FIRST`
/// in [`AT_SCOPES`]' order.
const AT_FIRST: u16 = 105;
const AT_CHARSET: u16 = 105;
const AT_COUNTER_STYLE: u16 = 107;
const AT_CUSTOM_MEDIA: u16 = 109;
const AT_DOCUMENT: u16 = 111;
const AT_FONT_FACE: u16 = 113;
const AT_IMPORT: u16 = 115;
const AT_KEYFRAME: u16 = 117;
const AT_MEDIA: u16 = 119;
const AT_NAMESPACE: u16 = 121;
const AT_PAGE: u16 = 123;
const AT_SUPPORTS: u16 = 125;
/// CSS's leaves end here; SCSS's follow ([`scss::LEAVES`]).
const LEAF_COUNT_CSS: u16 = 127;

/// Each leaf's scopes under the wrap, below `FN_FIRST` ([`FN_SCOPES`] and [`AT_SCOPES`] give
/// the rest).
static LEAVES: [&str; FN_FIRST as usize] = [
    "",
    "punctuation.definition.comment.css",
    "punctuation.definition.string.begin.css",
    "punctuation.definition.string.end.css",
    "constant.character.escape.css",
    "constant.character.escape.newline.css",
    "entity.name.tag.custom.css",
    "entity.name.tag.css",
    "entity.other.attribute-name.class.css punctuation.definition.entity.css",
    "entity.other.attribute-name.class.css",
    "entity.other.attribute-name.id.css punctuation.definition.entity.css",
    "entity.other.attribute-name.id.css",
    "entity.name.tag.wildcard.css",
    "punctuation.separator.combinator.css",
    "invalid.illegal.combinator.css",
    "entity.other.pseudo-element.css punctuation.definition.entity.css",
    "entity.other.pseudo-element.css",
    "entity.other.pseudo-element.css support.type.vendor-prefix.css",
    "entity.other.pseudo-class.css punctuation.definition.entity.css",
    "entity.other.pseudo-class.css",
    "punctuation.definition.entity.css",
    "entity.other.attribute-name.css",
    "keyword.operator.attribute-selector.css",
    "string.unquoted.css",
    "keyword.other.css",
    "punctuation.separator.sequence.css",
    "entity.other.namespace-prefix.css",
    "entity.name.namespace.wildcard.css",
    "punctuation.separator.namespace.css",
    "punctuation.definition.group.begin.css",
    "meta.group.css punctuation.definition.group.end.css",
    "punctuation.definition.group.end.css",
    "keyword.other.pseudo-class.css",
    "constant.numeric.integer.decimal.css",
    "constant.numeric.integer.decimal.css keyword.other.unit.css",
    "constant.numeric.float.decimal.css",
    "constant.numeric.float.decimal.css punctuation.separator.decimal.css",
    "constant.numeric.float.decimal.css keyword.other.unit.css",
    "invalid.illegal.numeric.css",
    "punctuation.section.property-list.css",
    "punctuation.separator.key-value.css",
    "punctuation.terminator.rule.css",
    "support.type.vendor-prefix.css",
    "invalid.deprecated.custom-property.css keyword.other.custom-property.prefix.css",
    "invalid.deprecated.custom-property.css support.type.custom-property.name.css",
    "support.type.custom-property.css punctuation.definition.custom-property.css",
    "support.type.custom-property.css support.type.custom-property.name.css",
    "support.type.property-name.css",
    "support.constant.property-value.css",
    "support.constant.font-name.css",
    "support.constant.color.w3c-standard-color-name.css",
    "support.constant.color.w3c-extended-color-keywords.css",
    "support.constant.color.w3c-special-color-keyword.css",
    "constant.other.color.rgb-value.css punctuation.definition.constant.css",
    "constant.other.color.rgb-value.css",
    "constant.other.color.rgba-value.css punctuation.definition.constant.css",
    "constant.other.color.rgba-value.css",
    "keyword.other.important.css",
    "punctuation.section.begin.css",
    "punctuation.section.end.css",
    "string.unquoted.line-name.css",
    "support.unicode-range.css support.constant.unicode-range.prefix.css",
    "support.unicode-range.css constant.codepoint-range.css",
    "support.unicode-range.css constant.codepoint-range.css punctuation.section.range.css",
    "keyword.operator.css",
    "entity.other.counter-name.css string.unquoted.css",
    "support.constant.property-value.counter-style.css",
    "support.constant.symbol-type.css",
    "support.keyword.timing-direction.css",
    "support.keyword.repetitions.css",
    "keyword.other.color-space.css",
    "keyword.other.unit.css",
    "invalid.illegal.counter-style-name.css",
    "entity.other.counter-style-name.css",
    "punctuation.terminator.css",
    "punctuation.definition.block.begin.css",
    "meta.block.css punctuation.definition.block.end.css",
    "entity.other.animation-name.css",
    "punctuation.definition.arbitrary-repetition.css",
    "keyword.control.at-rule.keyframe.css support.type.property-vendor.css",
    "keyword.keyframe-selector.css",
    "support.constant.media.css",
    "keyword.operator.logic.media.css",
    "support.type.property-name.media.css",
    "support.type.property-name.media.css support.type.vendor-prefix.css",
    "keyword.operator.arithmetic.css",
    "keyword.operator.logic.css",
];

/// The scopes under `source.css` (or `source.scss`) kind `kind` carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    if kind & scss::YAML_KIND != 0 {
        // SCSS's front matter's YAML
        let mut v = vec!["frontmatter.jekyll", "source.yaml"];
        v.extend(crate::yaml_scan::scopes(kind & !scss::YAML_KIND));
        return v;
    }
    let (wrap, leaf) = (
        usize::from(kind >> LEAF_BITS),
        kind & ((1 << LEAF_BITS) - 1),
    );
    let Some(w) = WRAPS.get(wrap) else {
        return Vec::new();
    };
    let mut v: Vec<&'static str> = w.split(' ').filter(|s| !s.is_empty()).collect();
    if leaf >= scss::SCSS_FIRST {
        match scss::LEAVES.get(usize::from(leaf - scss::SCSS_FIRST)) {
            Some(l) => v.extend(l.split(' ')),
            None => return Vec::new(),
        }
    } else if leaf < FN_FIRST {
        v.extend(
            LEAVES[usize::from(leaf)]
                .split(' ')
                .filter(|s| !s.is_empty()),
        );
    } else if leaf < AT_FIRST {
        v.push(FN_SCOPES[usize::from(leaf - FN_FIRST)]);
    } else {
        let i = usize::from(leaf - AT_FIRST);
        v.push(AT_SCOPES[i / 2]);
        if i % 2 == 1 {
            v.push("punctuation.definition.keyword.css");
        }
    }
    v
}

/// `support.function.X.css`, from `FN_ATTR` on.
static FN_SCOPES: [&str; 18] = [
    "support.function.attr.css",
    "support.function.calc.css",
    "support.function.image.css",
    "support.function.filter.css",
    "support.function.gradient.css",
    "support.function.grid.css",
    "support.function.url.css",
    "support.function.var.css",
    "support.function.color.css",
    "support.function.counter.css",
    "support.function.font-face.css",
    "support.function.transform.css",
    "support.function.timing.css",
    "support.function.shape.css",
    "support.function.toggle.css",
    "support.function.url-prefix.css",
    "support.function.domain.css",
    "support.function.regexp.css",
];

/// `keyword.control.at-rule.X.css`, from `AT_CHARSET` on, two leaves apart.
static AT_SCOPES: [&str; 11] = [
    "keyword.control.at-rule.charset.css",
    "keyword.control.at-rule.counter-style.css",
    "keyword.control.at-rule.custom-media.css",
    "keyword.control.at-rule.document.css",
    "keyword.control.at-rule.font-face.css",
    "keyword.control.at-rule.import.css",
    "keyword.control.at-rule.keyframe.css",
    "keyword.control.at-rule.media.css",
    "keyword.control.at-rule.namespace.css",
    "keyword.control.at-rule.page.css",
    "keyword.control.at-rule.supports.css",
];

// ---- the word table ----

// The lists a word is in, as bits.
const TAG: u16 = 1;
const PROP: u16 = 1 << 1;
const CONST: u16 = 1 << 2;
const STD_COLOR: u16 = 1 << 3;
const EXT_COLOR: u16 = 1 << 4;
const SPECIAL_COLOR: u16 = 1 << 5;
const FONT_NAME: u16 = 1 << 6;
const COUNTER_STYLE: u16 = 1 << 7;
/// SCSS's property names: CSS's and `backdrop-filter`.
const SCSS_PROP: u16 = 1 << 8;
/// SCSS's partial property names (nested properties), ranked.
const PARTIAL: u16 = 1 << 9;
/// Sass's functions.
const SASS_FN: u16 = 1 << 10;
/// CSS's lists: the bits below SCSS's.
const CSS_LISTS: u16 = SCSS_PROP - 1;

/// `none`'s rank in `property-value-constants`, to tell it from the other words.
const NONE_RANK: u16 = {
    let mut i = 0;
    loop {
        let e = &ENTRIES[i];
        if e.word.len() == 4
            && e.word[0] == b'n'
            && e.word[1] == b'o'
            && e.word[2] == b'n'
            && e.word[3] == b'e'
        {
            break e.rank;
        }
        i += 1;
    }
};

/// The table's list: [`ENTRIES`]' words, as spelled for the case-sensitive lists.
#[allow(clippy::large_const_arrays)] // read only where the table is built: not in the binary
const LIST: [&[u8]; ENTRIES.len()] = {
    let mut l: [&[u8]; ENTRIES.len()] = [b""; ENTRIES.len()];
    let mut i = 0;
    while i < l.len() {
        l[i] = ENTRIES[i].word;
        i += 1;
    }
    l
};

/// [`ENTRIES`]' words, ASCII case ignored: a word's id is its index plus one.
static TABLE: Words<4096, { ENTRIES.len() }, { text_len(&LIST) }, true> = Words::new(&LIST);

/// [`ENTRIES`] without their words, by index.
static LISTS: [Lists; ENTRIES.len()] = {
    let mut t = [Lists {
        exact: 0,
        folded: 0,
        rank: 0,
        rank_folded: 0,
        prop_rank: 0,
        partial_rank: 0,
    }; ENTRIES.len()];
    let mut i = 0;
    while i < t.len() {
        let e = &ENTRIES[i];
        t[i] = Lists {
            exact: e.exact,
            folded: e.folded,
            rank: e.rank,
            rank_folded: e.rank_folded,
            prop_rank: e.prop_rank,
            partial_rank: e.partial_rank,
        };
        i += 1;
    }
    t
};

/// An entry's lists and ranks ([`words::Entry`]'s, its word in [`TABLE`]).
#[derive(Clone, Copy)]
struct Lists {
    exact: u16,
    folded: u16,
    rank: u16,
    rank_folded: u16,
    prop_rank: u16,
    partial_rank: u16,
}

/// What the table says of a word: the lists it is in, and its ranks.
#[derive(Clone, Copy)]
struct Word {
    lists: u16,
    /// In `property-value-constants`.
    rank: u16,
    /// In `{{counter_styles}}` (as in `property-value-constants`, ignoring case).
    counter_rank: u16,
    /// In the property names.
    prop_rank: u16,
    /// In SCSS's partial property names.
    partial_rank: u16,
}

const NO_WORD: Word = Word {
    lists: 0,
    rank: u16::MAX,
    counter_rank: u16::MAX,
    prop_rank: u16::MAX,
    partial_rank: u16::MAX,
};

/// Word `w`: its case-sensitive lists if spelled as listed, its case-insensitive ones if not.
fn lookup(w: &[u8]) -> Word {
    let (id, spelled) = TABLE.get_spelled(w);
    word(id, spelled)
}

/// What the table says of the word of id `id` (0 for none), spelled as listed or not.
#[inline(always)]
fn word(id: u16, spelled: bool) -> Word {
    if id == 0 {
        return NO_WORD;
    }
    let e = &LISTS[usize::from(id - 1)];
    if spelled {
        Word {
            lists: e.exact | e.folded,
            rank: e.rank.min(e.rank_folded),
            counter_rank: e.rank_folded,
            prop_rank: e.prop_rank,
            partial_rank: e.partial_rank,
        }
    } else {
        Word {
            lists: e.folded,
            rank: e.rank_folded,
            counter_rank: e.rank_folded,
            prop_rank: u16::MAX,
            partial_rank: u16::MAX,
        }
    }
}

/// A word's prefix that is a word of the table: its length, its id, and whether the word as
/// listed spells it as the prefix's entry does.
#[derive(Clone, Copy)]
struct Prefix {
    len: u8,
    spelled: bool,
    id: u16,
}

/// Entry `i`'s prefixes that end before a `-` that no `-` precedes ([`Scan::word_ends`]' ends)
/// and are words of the table, shortest first, into `out` from `at` (with `out` `None`, only
/// counted): where the next entry's go.
const fn prefixes_of(i: usize, out: Option<&mut [Prefix]>, at: usize) -> usize {
    let w = ENTRIES[i].word;
    let mut out = out;
    let mut n = at;
    let mut k = 1;
    while k < w.len() {
        if w[k] == b'-' && w[k - 1] != b'-' {
            let (pre, _) = w.split_at(k);
            let id = TABLE.find(pre);
            if id != 0 {
                // the prefix's entry as listed, against the word's bytes
                let listed = LIST[(id - 1) as usize];
                let mut same = true;
                let mut j = 0;
                while j < k {
                    same &= listed[j] == pre[j];
                    j += 1;
                }
                if let Some(o) = &mut out {
                    o[n] = Prefix {
                        len: k as u8,
                        spelled: same,
                        id,
                    };
                }
                n += 1;
            }
        }
        k += 1;
    }
    n
}

/// How many prefixes [`PREFIXES`] holds.
const PREFIX_COUNT: usize = {
    let mut n = 0;
    let mut i = 0;
    while i < ENTRIES.len() {
        n = prefixes_of(i, None, n);
        i += 1;
    }
    n
};

/// Every word's prefixes ([`prefixes_of`]), one word's after another's: a word read whole as
/// listed has its prefixes' entries without a lookup each (X-018). Word `i`'s are
/// `PREFIXES[PREFIX_AT[i]..PREFIX_AT[i + 1]]`.
static PREFIXES: [Prefix; PREFIX_COUNT] = {
    let mut t = [Prefix {
        len: 0,
        spelled: false,
        id: 0,
    }; PREFIX_COUNT];
    let mut n = 0;
    let mut i = 0;
    while i < ENTRIES.len() {
        n = prefixes_of(i, Some(&mut t), n);
        i += 1;
    }
    t
};

/// Where each word's prefixes start in [`PREFIXES`], and past the last word's, their end.
static PREFIX_AT: [u16; ENTRIES.len() + 1] = {
    assert!(PREFIX_COUNT <= u16::MAX as usize);
    let mut t = [0u16; ENTRIES.len() + 1];
    let mut n = 0;
    let mut i = 0;
    while i < ENTRIES.len() {
        t[i] = n as u16;
        n = prefixes_of(i, None, n);
        i += 1;
    }
    t[ENTRIES.len()] = n as u16;
    t
};

// ---- characters ----

/// `\w` at a character `c`: ASCII letters, digits and `_`, and past ASCII letters and digits.
fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The character at `i` of `b` (a UTF-8 line), and its length.
fn char_at(b: &[u8], i: usize) -> (char, usize) {
    let c = b[i];
    if c < 0x80 {
        return (char::from(c), 1);
    }
    let len = match c {
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    };
    let end = (i + len).min(b.len());
    match std::str::from_utf8(&b[i..end])
        .ok()
        .and_then(|s| s.chars().next())
    {
        Some(ch) => (ch, len),
        None => ('\u{fffd}', 1),
    }
}

/// The character that ends at `i` of `b`.
fn char_before(b: &[u8], i: usize) -> char {
    let mut s = i - 1;
    while s > 0 && b[s] & 0xc0 == 0x80 && i - s < 4 {
        s -= 1;
    }
    char_at(b, s).0
}

/// `{{nonascii}}`: `[\p{L}\p{M}\p{S}\p{N}]` past ASCII, taken as anything past ASCII but
/// spaces, controls, format characters and punctuation.
fn nonascii(c: char) -> bool {
    if c.is_ascii() || c.is_whitespace() || c.is_control() {
        return false;
    }
    if c.is_alphanumeric() {
        return true;
    }
    !matches!(c,
        '\u{a1}' | '\u{a7}' | '\u{ab}' | '\u{ad}' | '\u{b6}' | '\u{b7}' | '\u{bb}' | '\u{bf}'
        | '\u{37e}' | '\u{387}' | '\u{55a}'..='\u{55f}' | '\u{589}' | '\u{58a}'
        | '\u{5be}' | '\u{5c0}' | '\u{5c3}' | '\u{5c6}' | '\u{5f3}' | '\u{5f4}'
        | '\u{609}' | '\u{60a}' | '\u{60c}' | '\u{60d}' | '\u{61b}' | '\u{61d}'..='\u{61f}'
        | '\u{66a}'..='\u{66d}' | '\u{6d4}'
        | '\u{200b}'..='\u{2027}' | '\u{202a}'..='\u{202e}' | '\u{2030}'..='\u{2043}'
        | '\u{2045}'..='\u{2051}' | '\u{2053}'..='\u{205e}' | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{206f}' | '\u{207d}' | '\u{207e}' | '\u{208d}' | '\u{208e}'
        | '\u{2308}'..='\u{230b}' | '\u{2329}' | '\u{232a}' | '\u{2768}'..='\u{2775}'
        | '\u{27c5}' | '\u{27c6}' | '\u{27e6}'..='\u{27ef}' | '\u{2983}'..='\u{2998}'
        | '\u{29d8}'..='\u{29db}' | '\u{29fc}' | '\u{29fd}' | '\u{2cf9}'..='\u{2cfc}'
        | '\u{2cfe}' | '\u{2cff}' | '\u{2e00}'..='\u{2e4f}' | '\u{3001}'..='\u{3003}'
        | '\u{3008}'..='\u{3011}' | '\u{3014}'..='\u{301f}' | '\u{3030}' | '\u{303d}'
        | '\u{30a0}' | '\u{30fb}' | '\u{fe10}'..='\u{fe19}' | '\u{fe30}'..='\u{fe52}'
        | '\u{fe54}'..='\u{fe61}' | '\u{fe63}' | '\u{fe68}' | '\u{fe6a}' | '\u{fe6b}'
        | '\u{feff}' | '\u{ff01}'..='\u{ff03}' | '\u{ff05}'..='\u{ff0a}'
        | '\u{ff0c}'..='\u{ff0f}' | '\u{ff1a}' | '\u{ff1b}' | '\u{ff1f}' | '\u{ff20}'
        | '\u{ff3b}'..='\u{ff3d}' | '\u{ff3f}' | '\u{ff5b}' | '\u{ff5d}' | '\u{ff5f}'..='\u{ff65}'
        | '\u{e000}'..='\u{f8ff}' | '\u{fff9}'..='\u{fffb}' | '\u{fffe}' | '\u{ffff}'
        | '\u{f0000}'..='\u{10ffff}')
}

/// `custom_element_chars` past ASCII (`·` and the ranges of the grammar's variable).
fn custom_element_wide(c: char) -> bool {
    matches!(c,
        '\u{b7}' | '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{2ff}'
        | '\u{300}'..='\u{37d}' | '\u{37f}'..='\u{1fff}' | '\u{200c}' | '\u{200d}'
        | '\u{203f}' | '\u{2040}' | '\u{2070}'..='\u{218f}' | '\u{2c00}'..='\u{2fef}'
        | '\u{3001}'..='\u{d7ff}' | '\u{f900}'..='\u{fdcf}' | '\u{fdf0}'..='\u{fffd}'
        | '\u{10000}'..='\u{effff}')
}

/// ASCII's classes as bits of [`CLASSES`]: a test of a few of them is one load, where
/// `is_ascii_alphanumeric` is a chain of compares (X-018).
const ALNUM: u8 = 1;
const UNDER: u8 = 1 << 1;
const DASH: u8 = 1 << 2;
/// [`is_space`]'s bytes: `[ \t\n\x0b\x0c\r]`.
const SPACE: u8 = 1 << 3;
/// `[A-Za-z0-9_]`, `\w` in ASCII.
const WORDS: u8 = ALNUM | UNDER;
/// `[A-Za-z0-9-]`.
const NAMES: u8 = ALNUM | DASH;
/// `[A-Za-z0-9_-]`, `{{nmchar}}` in ASCII.
const NMCHARS: u8 = ALNUM | UNDER | DASH;
/// `[a-z0-9]`, beside [`ALNUM`]: with [`DASH`], a property's name's bytes (SCSS's
/// `property_shape`, X-037: three ranges compared a byte).
const LOWNUM: u8 = 1 << 4;

/// Each byte's classes: [`ALNUM`] (and [`LOWNUM`]), [`UNDER`], [`DASH`], [`SPACE`]; none past
/// ASCII.
static CLASSES: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = if b.is_ascii_lowercase() || b.is_ascii_digit() {
            ALNUM | LOWNUM
        } else if b.is_ascii_alphanumeric() {
            ALNUM
        } else if b == b'_' {
            UNDER
        } else if b == b'-' {
            DASH
        } else if is_space(b) {
            SPACE
        } else {
            0
        };
        c += 1;
    }
    t
};

/// Whether byte `c` is in one of the classes `m`.
#[inline(always)]
fn is(c: u8, m: u8) -> bool {
    CLASSES[usize::from(c)] & m != 0
}

const fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

fn is_hex(c: u8) -> bool {
    c.is_ascii_hexdigit()
}

// ---- the functions ----

// The function families, as bits: which include names a function.
const F_ATTR: u16 = 1;
const F_CALC: u16 = 1 << 1;
/// `image-type`: `cross-fade(`, the gradients, `image(`, `image-set(` (and `url(`, its own).
const F_IMAGE: u16 = 1 << 2;
const F_FILTER: u16 = 1 << 3;
const F_MINMAX: u16 = 1 << 4;
const F_URL: u16 = 1 << 5;
const F_VAR: u16 = 1 << 6;
const F_ADJ: u16 = 1 << 7;
/// `color-functions`.
const F_COLOR: u16 = 1 << 8;
/// The rest of `builtin-functions`: `filter(`, the counters, the transforms, the shapes….
const F_REST: u16 = 1 << 9;
/// `@document`'s `url-prefix(`, `domain(`, `regexp(`.
const F_DOC: u16 = 1 << 10;
const BUILTIN: u16 =
    F_ATTR | F_CALC | F_IMAGE | F_FILTER | F_MINMAX | F_URL | F_VAR | F_ADJ | F_REST;

/// The function named `w` before a `(`: its family, its name's leaf, its group (`CALC_W` for
/// `calc`).
fn function(w: &[u8]) -> Option<(u16, u16, u8)> {
    Some(match w {
        b"attr" => (F_ATTR, FN_ATTR, G_ATTR),
        b"calc" => (F_CALC, FN_CALC, CALC_W),
        b"cross-fade" => (F_IMAGE, FN_IMAGE, G_CROSS_FADE),
        b"blur" => (F_FILTER, FN_FILTER, G_LENGTH),
        b"brightness" | b"contrast" | b"grayscale" | b"invert" | b"opacity" | b"saturate"
        | b"sepia" => (F_FILTER, FN_FILTER, G_FILTER_AMOUNT),
        b"drop-shadow" => (F_FILTER, FN_FILTER, G_DROP_SHADOW),
        b"hue-rotate" => (F_FILTER, FN_FILTER, G_ANGLE),
        b"linear-gradient" | b"repeating-linear-gradient" => (F_IMAGE, FN_GRADIENT, G_LINEAR),
        b"radial-gradient" | b"repeating-radial-gradient" => (F_IMAGE, FN_GRADIENT, G_RADIAL),
        b"image" => (F_IMAGE, FN_IMAGE, G_IMAGE),
        b"image-set" => (F_IMAGE, FN_IMAGE, G_IMAGE_SET),
        b"minmax" => (F_MINMAX, FN_GRID, G_MINMAX),
        b"url" => (F_URL, FN_URL, G_URL),
        b"var" => (F_VAR, FN_VAR, G_VAR),
        b"red" | b"green" | b"blue" | b"alpha" | b"a" => (F_ADJ, FN_COLOR, G_ADJ_CHANNEL),
        b"hue" | b"h" => (F_ADJ, FN_COLOR, G_ADJ_HUE),
        b"saturation" | b"lightness" | b"whiteness" | b"blackness" | b"s" | b"l" | b"w" | b"b" => {
            (F_ADJ, FN_COLOR, G_ADJ_PERCENT)
        }
        b"tint" | b"shade" => (F_ADJ, FN_COLOR, G_TINT),
        b"blenda" | b"blend" => (F_ADJ, FN_COLOR, G_BLEND),
        b"filter" => (F_REST, FN_FILTER, G_FILTER),
        b"counter" => (F_REST, FN_COUNTER, G_COUNTER),
        b"counters" => (F_REST, FN_COUNTER, G_COUNTERS),
        b"symbols" => (F_REST, FN_COUNTER, G_SYMBOLS),
        b"format" => (F_REST, FN_FONT_FACE, G_STRINGS),
        b"local" => (F_REST, FN_FONT_FACE, G_URL),
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
        b"rgb" | b"rgba" | b"gray" => (F_COLOR, FN_COLOR, G_RGB),
        b"hsl" | b"hsla" | b"hwb" => (F_COLOR, FN_COLOR, G_HSL),
        b"device-cmyk" => (F_COLOR, FN_COLOR, G_CMYK),
        b"color" => (F_COLOR, FN_COLOR, G_COLOR),
        b"url-prefix" => (F_DOC, FN_URL_PREFIX, G_URL),
        b"domain" => (F_DOC, FN_DOMAIN, G_URL),
        b"regexp" => (F_DOC, FN_REGEXP, G_STRINGS),
        _ => return None,
    })
}

// ---- the numbers ----

/// The numeric types, for [`Scan::numeric`].
const T_ANGLE: u8 = 0;
const T_FREQUENCY: u8 = 1;
const T_LENGTH: u8 = 2;
const T_RESOLUTION: u8 = 3;
const T_TIME: u8 = 4;
const T_PERCENT: u8 = 5;
const T_NUMBER: u8 = 6;
const T_INTEGER: u8 = 7;

/// `numeric-values`' order.
static NUMERIC: [u8; 7] = [
    T_ANGLE,
    T_FREQUENCY,
    T_LENGTH,
    T_RESOLUTION,
    T_TIME,
    T_PERCENT,
    T_NUMBER,
];

/// No numeric type: what [`unit_type`] gives a word that is no unit.
const NO_TYPE: u8 = 0xff;

/// The type `u` is a unit of, ASCII case ignored; [`NO_TYPE`] for none.
fn unit_type(u: &[u8]) -> u8 {
    if u.is_empty() || u.len() > 4 {
        return NO_TYPE;
    }
    let mut l = [0u8; 4];
    for (d, s) in l.iter_mut().zip(u) {
        *d = s.to_ascii_lowercase();
    }
    match &l[..u.len()] {
        b"deg" | b"grad" | b"rad" | b"turn" => T_ANGLE,
        b"hz" | b"khz" => T_FREQUENCY,
        b"em" | b"ex" | b"ch" | b"rem" | b"vw" | b"vh" | b"vmin" | b"vmax" | b"cm" | b"mm"
        | b"q" | b"in" | b"pt" | b"pc" | b"px" | b"fr" => T_LENGTH,
        b"dpi" | b"dpcm" | b"dppx" => T_RESOLUTION,
        b"s" | b"ms" => T_TIME,
        _ => NO_TYPE,
    }
}

/// Whether `u` is one of type `t`'s units, ASCII case ignored.
fn unit_of(t: u8, u: &[u8]) -> bool {
    let mut l = [0u8; 4];
    if u.is_empty() || u.len() > 4 {
        return false;
    }
    for (d, s) in l.iter_mut().zip(u) {
        *d = s.to_ascii_lowercase();
    }
    let u = &l[..u.len()];
    match t {
        T_ANGLE => matches!(u, b"deg" | b"grad" | b"rad" | b"turn"),
        T_FREQUENCY => matches!(u, b"hz" | b"khz"),
        T_LENGTH => matches!(
            u,
            b"em"
                | b"ex"
                | b"ch"
                | b"rem"
                | b"vw"
                | b"vh"
                | b"vmin"
                | b"vmax"
                | b"cm"
                | b"mm"
                | b"q"
                | b"in"
                | b"pt"
                | b"pc"
                | b"px"
                | b"fr"
        ),
        T_RESOLUTION => matches!(u, b"dpi" | b"dpcm" | b"dppx"),
        T_TIME => matches!(u, b"s" | b"ms"),
        _ => false,
    }
}

/// A number's shape at a place: `[-+]?`, then `\d+` (`int`), `{{float}}`'s first form
/// `\d*(\.)\d+{{exponent}}?` (`dot`, `frac`), or its second `\d+{{exponent}}` (`exp`).
#[derive(Clone, Copy)]
struct Num {
    /// After the sign.
    digits: usize,
    /// After `\d+`, or `digits` for none.
    int: usize,
    /// The first form's `.` and end, if it matches.
    dot: Option<(usize, usize)>,
    /// The second form's end, if it matches.
    exp: Option<usize>,
}

// ---- the scan ----

/// How many contexts a line can hold; one pushed past it drops the bottom one.
const MAX: usize = 128;

/// Lex `b` from `state` with `run`: CSS's scan or SCSS's ([`scss`]), the stack kept between lines
/// in `state` either way; its lexemes written to `k`, returned as the line leaves it.
#[inline(always)]
fn lex<const S: bool, K: Sink>(
    b: &[u8],
    from: usize,
    state: &mut State,
    out: &mut Out,
    k: K,
    run: impl FnOnce(&mut Scan<'_, '_, '_, S, K>),
) -> K {
    let mut s = Scan::fresh(b, from, out, k);
    s.load_state(state);
    run(&mut s);
    *state = State::keep(&s.stack, s.depth, 0);
    s.k
}

/// CSS's scan kept across a block's lines, and whether its lines are still syntect's, which
/// a line's scan leaves to its caller.
struct Carried<'b, 'o, 's, const S: bool, K: Sink> {
    s: Scan<'b, 'o, 's, S, K>,
}

impl<'b, 'o, 's, const S: bool, K: Sink> Scan<'b, 'o, 's, S, K> {
    /// A scan of `b`, its spans from `from`, returned as a literal so that it is built in place;
    /// the state is [`Scan::load_state`]'s.
    #[inline(always)]
    fn fresh(b: &'b [u8], from: usize, out: &'o mut Out<'s>, k: K) -> Self {
        Scan {
            b,
            out,
            k,
            stack: [MAIN; MAX],
            depth: 1,
            ctx: MAIN,
            at: from,
            spaces_run: Run::new((usize::MAX, 0)),
            digits_run: Run::new((usize::MAX, 0)),
            letters_run: Run::new((usize::MAX, 0)),
            names_run: Run::new((usize::MAX, 0)),
            custom_failed: std::cell::Cell::new(0),
            isel_end: std::cell::Cell::new(usize::MAX),
            class_runs: [const { Run::new((usize::MAX, 0)) }; 3],
            nm_ends: std::cell::RefCell::new(Vec::new()),
            swrap: W_NONE,
            wrap: 0,
            shape_run: std::cell::Cell::new((usize::MAX, 0, 0)),
        }
    }

    /// What [`Scan::fresh`] leaves to `state`: the stack and what is read from it, apart from
    /// the literal, which a constructor that changed it before returning it copied (a `memcpy`
    /// a line, X-056's trap).
    #[inline(always)]
    fn load_state(&mut self, state: &State) {
        self.depth = state.load(&mut self.stack);
        self.ctx = self.stack[self.depth - 1];
        self.rewrap();
    }

    /// What a line starts with, its spans from `from`, on a scan kept from the line before.
    #[inline(always)]
    fn line_start(&mut self, from: usize) {
        self.ctx = self.stack[self.depth - 1];
        self.at = from;
        self.spaces_run.set((usize::MAX, 0));
        self.digits_run.set((usize::MAX, 0));
        self.letters_run.set((usize::MAX, 0));
        self.names_run.set((usize::MAX, 0));
        self.custom_failed.set(0);
        self.isel_end.set(usize::MAX);
        for r in &self.class_runs {
            r.set((usize::MAX, 0));
        }
        self.nm_ends.get_mut().clear();
        self.swrap = W_NONE;
        self.rewrap();
        self.shape_run.set((usize::MAX, 0, 0));
    }
}

/// Bytes `start..end` of the line.
type Span = (usize, usize);

/// The CSS ops [`scss`] names, in its contexts' lists, its includes and its calls: the ops its
/// copy of [`Scan::op`] is called with. `scss_css_ops!(m)` is `m!(P_… P_… …)`, the names handed
/// to `m` (`scss`'s tests check its lists against them).
macro_rules! scss_css_ops {
    ($m:ident) => {
        $m!(
            P_ATTR_NAME
            P_ATTR_OP
            P_ATTR_SEL_END
            P_ATTR_UNQUOTED
            P_ATTR_VAL_END
            P_CALL_END
            P_CASE_FLAG
            P_COMMA
            P_COMMENT
            P_COUNTER_NAME
            P_COUNTER_STYLES
            P_CUSTOM_PROP
            P_DROP_WORDS
            P_FROM_TO
            P_GROUP
            P_GROUP_END
            P_LINE_NAME
            P_LINE_NAMES
            P_LINE_NAMES_END
            P_MEDIA_LOGIC
            P_MEDIA_TYPES
            P_MQ_COMMA
            P_NTH_INVALID
            P_QUALIFIED
            P_STRING
            P_SUPPORTS_LOGIC
            P_SYMBOL_TYPES
            P_UNICODE_RANGE
            P_VENDOR
        )
    };
}

/// Whether [`scss_css_ops`] holds `op`.
#[cfg(test)]
fn scss_css_op(op: u8) -> bool {
    macro_rules! any {
        ($($o:ident)*) => {
            matches!(op, $($o)|*)
        };
    }
    scss_css_ops!(any)
}

/// A run's memo: the place a scan of a run of one class of bytes started and where the run ends,
/// so a scan from anywhere in it is answered at once (a run passed a place at a time stays linear).
type Run = std::cell::Cell<(usize, usize)>;

/// A line's scan; `S` for SCSS ([`scss`]), whose text takes the wrap of the innermost context
/// that has one (a string's interpolation is the string's), where CSS's takes its top's.
struct Scan<'b, 'o, 's, const S: bool = false, K: Sink = Push> {
    b: &'b [u8],
    /// The spans' vector: the span API's (HTML's embeds among them), and where SCSS's front
    /// matter pushes YAML's spans for the run sink to take.
    out: &'o mut Out<'s>,
    /// Where the lexemes go, a line's spans or its runs by theme rule (X-075, X-068's sink).
    k: K,
    stack: [u8; MAX],
    depth: usize,
    /// The top context, `stack[depth - 1]`, kept as the stack changes (X-037: read at every
    /// place and after every token, it was a load of the depth and a bounds test each time).
    ctx: u8,
    /// Where the spans pushed so far end: a context's text fills up to the next token.
    at: usize,
    /// The last run of `\s`, of digits, of letters, of `[A-Za-z0-9-]` scanned.
    spaces_run: Run,
    digits_run: Run,
    letters_run: Run,
    names_run: Run,
    /// Where the run of custom element characters a failed custom element read ends: a custom
    /// element starting in it fails too.
    custom_failed: std::cell::Cell<usize>,
    /// SCSS's: the last `}` of the line an interpolated selector's lookahead takes, once read
    /// (`usize::MAX` before, 0 for none).
    isel_end: std::cell::Cell<usize>,
    /// SCSS's runs of `[a-zA-Z0-9_-]`, `[a-zA-Z_-]` and `[a-z_-]`.
    class_runs: [Run; 3],
    /// SCSS's, on a long line: where `{{nmchar}}*` from each place ends, as read (`u32::MAX` not
    /// yet).
    nm_ends: std::cell::RefCell<Vec<u32>>,
    /// SCSS's: the wrap of the innermost context that has one, kept as the stack changes.
    swrap: u8,
    /// The wrap the text and tokens take, as a kind's bits ([`Scan::wrap`]): CSS's top's,
    /// SCSS's [`Scan::swrap`], kept as the stack changes.
    wrap: u16,
    /// SCSS's: the last run of `[a-z0-9-]` a property's shape read: where it starts and ends,
    /// and its last `-` that no letter or `-` follows (`usize::MAX` for none).
    shape_run: std::cell::Cell<(usize, usize, usize)>,
}

impl<const S: bool, K: Sink> Scan<'_, '_, '_, S, K> {
    #[inline(always)]
    fn top(&self) -> u8 {
        self.ctx
    }

    /// Set the depth to `d`, the top read from the stack again.
    fn set_depth(&mut self, d: usize) {
        self.depth = d;
        self.ctx = self.stack[d - 1];
    }

    #[inline(always)]
    fn wrap(&self) -> u16 {
        self.wrap
    }

    /// The wrap read from the stack again: CSS's top's, SCSS's [`Scan::swrap`].
    fn rewrap(&mut self) {
        if S {
            self.swrap = self.stack[..self.depth]
                .iter()
                .rev()
                .map(|&c| own_wrap(c))
                .find(|&w| w != W_NONE)
                .unwrap_or(W_NONE);
            self.wrap = u16::from(self.swrap) << LEAF_BITS;
        } else {
            self.wrap = u16::from(own_wrap(self.top())) << LEAF_BITS;
        }
    }

    /// Fill the top context's text up to `to`.
    fn fill(&mut self, to: usize) {
        let to = to.min(self.b.len());
        if to > self.at {
            let w = self.wrap();
            if w != 0 {
                self.k.push_call(&mut self.out, self.at, to, w | PLAIN);
            }
            self.at = to;
        }
    }

    /// Bytes `start..end` are leaf `leaf` in the top context's wrap.
    fn tok(&mut self, start: usize, end: usize, leaf: u16) {
        self.fill(start);
        let end = end.min(self.b.len());
        if end > start {
            let kind = self.wrap() | leaf;
            if kind != 0 {
                self.k.push_call(&mut self.out, start, end, kind);
            }
            self.at = end;
        }
    }

    /// Push context `c` at `at`, the text before it filled first; at [`MAX`] the bottom
    /// context (above the base) is dropped.
    fn push(&mut self, c: u8, at: usize) {
        self.fill(at);
        if self.depth == MAX {
            crate::stack::drop_bottom(&mut self.stack, &mut self.depth);
        }
        self.stack[self.depth] = c;
        self.depth += 1;
        self.ctx = c;
        let w = own_wrap(c);
        if S {
            if w != W_NONE {
                self.swrap = w;
                self.wrap = u16::from(w) << LEAF_BITS;
            }
        } else {
            self.wrap = u16::from(w) << LEAF_BITS;
        }
    }

    /// Pop the top context at `at`, its text before it filled first.
    fn pop(&mut self, at: usize) {
        self.fill(at);
        if self.depth > 1 {
            let popped = self.ctx;
            self.set_depth(self.depth - 1);
            if !S || own_wrap(popped) != W_NONE {
                self.rewrap();
            }
        }
    }

    // ---- reading the line: `n` is its `\n`, `n + 1` its end ----

    /// The byte at `i`: the line's, its `\n` at `n`, none past it.
    fn ch(&self, i: usize) -> Option<u8> {
        if i < self.b.len() {
            Some(self.b[i])
        } else if i == self.b.len() {
            Some(b'\n')
        } else {
            None
        }
    }

    /// Whether `\w` holds at `i`.
    fn word_at(&self, i: usize) -> bool {
        if i >= self.b.len() {
            return false;
        }
        let c = self.b[i];
        if c < 0x80 {
            is(c, WORDS)
        } else {
            word_char(char_at(self.b, i).0)
        }
    }

    /// Whether `\w` holds just before `i`: ASCII where it's asked, without a call (X-037).
    #[inline(always)]
    fn word_before(&self, i: usize) -> bool {
        if i == 0 || i > self.b.len() {
            // the line's start, or past its `\n`
            return false;
        }
        let c = self.b[i - 1];
        if c < 0x80 {
            is(c, WORDS)
        } else {
            self.word_before_wide(i)
        }
    }

    /// [`Scan::word_before`] where a byte past ASCII ends at `i`.
    #[cold]
    #[inline(never)]
    fn word_before_wide(&self, i: usize) -> bool {
        word_char(char_before(self.b, i))
    }

    /// `\b` at `i`.
    fn boundary(&self, i: usize) -> bool {
        self.word_before(i) != self.word_at(i)
    }

    /// [`Scan::boundary`] between two ASCII bytes answered where it's asked: for the busiest
    /// callers only, the rest call for it (X-037: 45k calls a round in SCSS at ~31).
    #[inline(always)]
    fn boundary_hot(&self, i: usize) -> bool {
        let b = self.b;
        if i > 0 && i < b.len() && b[i - 1] < 0x80 && b[i] < 0x80 {
            return is(b[i - 1], WORDS) != is(b[i], WORDS);
        }
        self.boundary(i)
    }

    /// Where the character at `i` ends.
    fn next(&self, i: usize) -> usize {
        if i >= self.b.len() {
            return i + 1;
        }
        let c = self.b[i];
        if c < 0x80 {
            i + 1
        } else {
            (i + char_at(self.b, i).1).min(self.b.len())
        }
    }

    /// Where `\s*` at `i` ends (the `\n` at `n` is a space).
    #[inline(always)]
    fn spaces(&self, i: usize) -> usize {
        if i < self.b.len() && !matches!(self.b[i], b' ' | b'\t' | b'\r' | 0x0b | 0x0c | 0x80..) {
            // no space here: answered where it's asked, without a call
            return i;
        }
        self.spaces_run_end(i)
    }

    /// [`Scan::spaces`], one space before a byte that is no space answered where it's asked too:
    /// for the busiest callers only, the rest call for it (X-037: in all of them it was ~11 KB).
    #[inline(always)]
    fn spaces_hot(&self, i: usize) -> usize {
        let b = self.b;
        if i + 1 < b.len() && b[i] == b' ' && b[i + 1] < 0x80 && !is(b[i + 1], SPACE) {
            return i + 1;
        }
        self.spaces(i)
    }

    /// [`Scan::spaces`] where a space or the line's end is at `i`.
    #[inline(never)]
    fn spaces_run_end(&self, i: usize) -> usize {
        let b = self.b;
        if i + 1 < b.len() && b[i] == b' ' && b[i + 1] < 0x80 && !is(b[i + 1], SPACE) {
            // one space, as most are: without the memo or the loop (X-037)
            return i + 1;
        }
        let (a, e) = self.spaces_run.get();
        if a <= i && i <= e {
            return e;
        }
        let n = b.len();
        let mut k = i;
        // a run of ` ` (an indent) eight bytes a step, then ASCII's spaces a byte a step
        while let Some(w) = b.get(k..k + 8) {
            let x = u64::from_le_bytes(w.try_into().unwrap()) ^ 0x2020_2020_2020_2020;
            if x != 0 {
                k += (x.trailing_zeros() / 8) as usize;
                break;
            }
            k += 8;
        }
        while k < n && is(b[k], SPACE) {
            k += 1;
        }
        if k == n {
            // the line's `\n`, a space
            k = n + 1;
        } else if k < n && b[k] >= 0x80 {
            k = self.spaces_wide(k);
        }
        self.spaces_run.set((i, k));
        k
    }

    /// [`Scan::spaces_run_end`]'s run on from `k`, a byte past ASCII: spaces past ASCII and
    /// ASCII's as they come, and the line's `\n`. Out of line (X-037: Unicode's test inlined in
    /// it took ~15 instructions a call of saving registers).
    #[cold]
    #[inline(never)]
    fn spaces_wide(&self, mut k: usize) -> usize {
        let (b, n) = (self.b, self.b.len());
        while let Some(e) = self.space(k) {
            k = e;
            while k < n && is(b[k], SPACE) {
                k += 1;
            }
            if k < n && b[k] < 0x80 {
                break;
            }
        }
        k
    }

    /// Where the run of digits at `i` ends.
    fn digits(&self, i: usize) -> usize {
        let (a, e) = self.digits_run.get();
        if a <= i && i <= e {
            return e;
        }
        let mut k = i;
        while k < self.b.len() && self.b[k].is_ascii_digit() {
            k += 1;
        }
        self.digits_run.set((i, k));
        k
    }

    /// Where `{{exponent}}` at `p` ends, if it matches.
    fn exponent(&self, p: usize) -> Option<usize> {
        let b = self.b;
        if p < self.b.len() && (b[p] == b'e' || b[p] == b'E') {
            let mut q = p + 1;
            if q < self.b.len() && (b[q] == b'-' || b[q] == b'+') {
                q += 1;
            }
            let d = self.digits(q);
            if d > q {
                return Some(d);
            }
        }
        None
    }

    /// The number's shape at `p`, if a number starts there.
    fn num_at(&self, p: usize) -> Option<Num> {
        let b = self.b;
        let mut d = p;
        if d < self.b.len() && (b[d] == b'-' || b[d] == b'+') {
            d += 1;
        }
        let int = self.digits(d);
        let dot = if int < self.b.len() && b[int] == b'.' {
            let f = self.digits(int + 1);
            (f > int + 1).then(|| (int, self.exponent(f).unwrap_or(f)))
        } else {
            None
        };
        let exp = if int > d { self.exponent(int) } else { None };
        if int == d && dot.is_none() {
            return None;
        }
        Some(Num {
            digits: d,
            int,
            dot,
            exp,
        })
    }

    /// Where `\s` at `i` ends: an ASCII space, the line's `\n`, or a space past ASCII.
    fn space(&self, i: usize) -> Option<usize> {
        let c = self.ch(i)?;
        if c < 0x80 {
            return is_space(c).then_some(i + 1);
        }
        let (ch, len) = char_at(self.b, i);
        ch.is_whitespace().then_some((i + len).min(self.b.len()))
    }

    /// Where `[ \t]*` at `i` ends.
    fn blanks(&self, mut i: usize) -> usize {
        while i < self.b.len() && matches!(self.b[i], b' ' | b'\t') {
            i += 1;
        }
        i
    }

    /// `$` at `i`.
    fn eol(&self, i: usize) -> bool {
        i >= self.b.len()
    }

    /// Where `{{escape}}` at `i` ends.
    fn escape(&self, i: usize) -> Option<usize> {
        if i >= self.b.len() || self.b[i] != b'\\' {
            return None;
        }
        let mut q = i + 1;
        let c = self.ch(q)?;
        if is_hex(c) && q < self.b.len() {
            while q < self.b.len() && q - i <= 6 && is_hex(self.b[q]) {
                q += 1;
            }
            if let Some(s) = self.ch(q)
                && matches!(s, b' ' | b'\t' | b'\n' | b'\x0c')
            {
                q += 1;
            }
            return Some(q);
        }
        if c == b'\n' || c == b'\x0c' {
            return None;
        }
        Some(self.next(q))
    }

    /// `{{nmstart}}` at `i`: where it ends.
    fn nmstart(&self, i: usize) -> Option<usize> {
        let c = *self.b.get(i)?;
        if c.is_ascii_alphabetic() || c == b'_' {
            return Some(i + 1);
        }
        if c >= 0x80 {
            let (ch, len) = char_at(self.b, i);
            return nonascii(ch).then_some((i + len).min(self.b.len()));
        }
        self.escape(i)
    }

    /// `{{nmchar}}` at `i`: where it ends.
    fn nmchar(&self, i: usize) -> Option<usize> {
        let c = *self.b.get(i)?;
        if is(c, NMCHARS) {
            return Some(i + 1);
        }
        if c >= 0x80 {
            let (ch, len) = char_at(self.b, i);
            return (word_char(ch) || nonascii(ch)).then_some((i + len).min(self.b.len()));
        }
        self.escape(i)
    }

    /// Where `{{nmchar}}*` at `i` ends.
    fn nmchars(&self, mut i: usize) -> usize {
        let (b, n) = (self.b, self.b.len());
        loop {
            // `[-\w]` in ASCII first, the rest a character at a time
            while i < n && is(b[i], NMCHARS) {
                i += 1;
            }
            if i >= n || (b[i] < 0x80 && b[i] != b'\\') {
                // the line's end, or an ASCII byte that starts no escape: no `{{nmchar}}`
                return i;
            }
            match self.nmchar(i) {
                Some(q) => i = q,
                None => return i,
            }
        }
    }

    /// Where `{{ident}}` at `i` ends: `--{{nmchar}}+` or `-?{{nmstart}}{{nmchar}}*`.
    fn ident(&self, i: usize) -> Option<usize> {
        if self.b.get(i) == Some(&b'-') && self.b.get(i + 1) == Some(&b'-') {
            let e = self.nmchars(i + 2);
            if e > i + 2 {
                return Some(e);
            }
        }
        let s = if self.b.get(i) == Some(&b'-') {
            i + 1
        } else {
            i
        };
        let q = self.nmstart(s)?;
        Some(self.nmchars(q))
    }

    /// The run of `[A-Za-z0-9-]` at `i`: where it ends.
    fn name_end(&self, i: usize) -> usize {
        let (a, e) = self.names_run.get();
        if a <= i && i <= e {
            return e;
        }
        let mut k = i;
        while k < self.b.len() && is(self.b[k], NAMES) {
            k += 1;
        }
        self.names_run.set((i, k));
        k
    }

    // ---- the run ----

    /// The words of the table that a word at `p` (its `[A-Za-z0-9-]` run to `end`) can be, each
    /// with its end: where a `\b` follows, before each `-` and at the run's end.
    /// Where a prefix isn't a word of the table, `each` may or may not see it (its lists 0).
    fn word_ends(&self, p: usize, end: usize, mut each: impl FnMut(usize, Word)) {
        let b = self.b;
        let whole = end > p && end - p <= LONGEST && b[end - 1] != b'-' && !self.word_at(end);
        let (mut id, mut spelled) = (0, false);
        if whole {
            // the run a word as listed: its prefixes are its entry's
            (id, spelled) = TABLE.get_spelled(&b[p..end]);
            if id != 0 && spelled {
                let i = usize::from(id - 1);
                let at = usize::from(PREFIX_AT[i])..usize::from(PREFIX_AT[i + 1]);
                for x in &PREFIXES[at] {
                    each(p + usize::from(x.len), word(x.id, x.spelled));
                }
                each(end, word(id, true));
                return;
            }
        }
        let mut k = p + 1;
        // no word of the table is longer
        let lim = end.min(p + LONGEST + 1);
        while k < lim {
            if b[k] == b'-' && b[k - 1] != b'-' {
                each(k, lookup(&b[p..k]));
            }
            k += 1;
        }
        if whole {
            each(end, word(id, spelled));
        }
    }

    /// `/* … */` from `p`.
    fn comment(&mut self, p: usize) -> Option<usize> {
        let (b, n) = (self.b, self.b.len());
        if p > n {
            return None;
        }
        if p == 0 {
            // `^\s*(\*)(?!/)`: only where the line's first search starts
            let s = self.spaces(0).min(n);
            if s < n && b[s] == b'*' && b.get(s + 1) != Some(&b'/') {
                // `\*/` first, at the earliest place: none before `s`
                self.tok(s, s + 1, COMMENT_MARK);
                return Some(s + 1);
            }
        }
        let mut i = p;
        while i + 1 < n {
            match b[i..n - 1].iter().position(|&c| c == b'*') {
                Some(k) => {
                    i += k;
                    if b[i + 1] == b'/' {
                        self.tok(i, i + 2, COMMENT_MARK);
                        self.pop(i + 2);
                        return Some(i + 2);
                    }
                    i += 1;
                }
                None => break,
            }
        }
        // no `*/` on the rest of the line: its text is the comment's
        self.fill(n);
        Some(n + 1)
    }

    /// Op `op` at `p` in context `ctx`: where the scan goes on if it matched. The ops a
    /// context's list names at run time come here; the own code's ops by name are
    /// [`Scan::op_at`] at the call, its arm alone. SCSS's copy keeps only the arms of the CSS
    /// ops [`scss`] names ([`scss_css_ops`]), the only ones it's called with (X-037: the rest,
    /// CSS's at-rules, pseudo-classes and color values among them, were ~17 KB compiled again
    /// for a scanner that has its own).
    #[inline(never)]
    fn op(&mut self, op: u8, p: usize, ctx: u8, fresh: bool) -> Option<usize> {
        if S {
            // an arm an op, each with its op a constant: `op_at` folds to that arm alone
            macro_rules! arms {
                ($($o:ident)*) => {
                    match op {
                        $($o => self.op_at($o, p, ctx, fresh),)*
                        _ => {
                            debug_assert!(false, "SCSS names no CSS op {op}");
                            None
                        }
                    }
                };
            }
            return scss_css_ops!(arms);
        }
        self.op_at(op, p, ctx, fresh)
    }

    /// [`Scan::op`]'s arms, inlined where `op` is a constant: the match folds to its arm, with
    /// no call into the whole of them (X-018).
    #[inline(always)]
    fn op_at(&mut self, op: u8, p: usize, ctx: u8, fresh: bool) -> Option<usize> {
        let (b, n) = (self.b, self.b.len());
        let c = self.ch(p);
        match op {
            P_COMMENT => {
                if p + 1 < n && b[p] == b'/' && b[p + 1] == b'*' {
                    self.push(COMMENT, p);
                    self.tok(p, p + 2, COMMENT_MARK);
                    return Some(p + 2);
                }
                None
            }
            P_SELECTOR | P_SELECTOR_IN => {
                let c = c?;
                if (matches!(c, b':' | b'.' | b'*' | b'#' | b'[') || c.is_ascii_alphabetic()) {
                    self.push(SELECTOR, p);
                    return Some(p);
                }
                None
            }
            P_AT_RULES => self.at_rules(p),
            P_PL => {
                if c == Some(b'{') {
                    self.push(PL, p);
                    return Some(p);
                }
                None
            }
            P_RULE_LIST => {
                if c == Some(b'{') {
                    self.push(PL_BODY, p);
                    self.tok(p, p + 1, PL_MARK);
                    return Some(p + 1);
                }
                None
            }
            P_RULE_LIST_END => {
                let s = self.spaces(p);
                if self.ch(s) == Some(b'}') {
                    self.tok(s, s + 1, PL_MARK);
                    self.pop(s + 1);
                    return Some(s + 1);
                }
                None
            }
            P_PL_END => {
                if c == Some(b'}') {
                    self.tok(p, p + 1, PL_MARK);
                    self.pop(p + 1);
                    return Some(p + 1);
                }
                None
            }
            P_PL_BODY_END => {
                let s = self.spaces(p);
                if self.ch(s) == Some(b'}') {
                    self.pop(p);
                    return Some(p);
                }
                None
            }
            P_PROP_NAME => {
                let c = c?;
                if c == b'-' || c.is_ascii_lowercase() {
                    self.push(PROP_NAME, p);
                    return Some(p);
                }
                None
            }
            P_VALUE | P_FONT_VALUE => {
                if c == Some(b':') {
                    self.tok(p, p + 1, KEY_VALUE);
                    let q = self.blanks(p + 1);
                    self.push(
                        if op == P_VALUE {
                            PROP_VALUE
                        } else {
                            FONT_VALUE
                        },
                        q,
                    );
                    return Some(q);
                }
                None
            }
            P_VALUE_END => {
                let s = self.spaces(p);
                if self.ch(s) == Some(b';') {
                    self.tok(s, s + 1, RULE_END);
                    self.pop(s + 1);
                    return Some(s + 1);
                }
                if matches!(c, Some(b'}' | b')')) {
                    self.pop(p);
                    return Some(p);
                }
                None
            }
            P_VENDOR => {
                if c == Some(b'-') {
                    let r = &b[p + 1..n.min(p + 8)];
                    for v in [&b"webkit-"[..], b"moz-", b"ms-", b"o-"] {
                        if crate::starts(r, v) {
                            let e = p + 1 + v.len();
                            self.tok(p, e, VENDOR);
                            return Some(e);
                        }
                    }
                }
                None
            }
            P_BUILTIN_FNS => self.function_at(p, BUILTIN),
            P_FILTER_FNS => self.function_at(p, F_FILTER),
            P_IMAGE_TYPE => self.function_at(p, F_IMAGE | F_URL),
            P_VAR_FN => self.function_at(p, F_VAR),
            P_CALC_FN => self.function_at(p, F_CALC),
            P_ATTR_FN => self.function_at(p, F_ATTR),
            P_MINMAX_FN => self.function_at(p, F_MINMAX),
            P_URL_FN => self.function_at(p, F_URL),
            P_DOC_FNS => self.function_at(p, F_DOC),
            P_ADJ_FNS => self.function_at(p, F_ADJ),
            P_LINE_NAMES => {
                if c == Some(b'[') {
                    self.push(LINE_NAMES, p);
                    self.tok(p, p + 1, LINE_NAMES_BEGIN);
                    return Some(p + 1);
                }
                None
            }
            P_LINE_NAME => {
                let e = self.ident(p)?;
                self.tok(p, e, LINE_NAME);
                Some(e)
            }
            P_LINE_NAMES_END => {
                if c == Some(b']') {
                    self.tok(p, p + 1, LINE_NAMES_END);
                    self.pop(p + 1);
                    return Some(p + 1);
                }
                None
            }
            P_UNICODE_RANGE => self.unicode_range(p),
            P_NUMERIC => self.numeric(p, &NUMERIC),
            P_PERCENT => self.numeric(p, &[T_PERCENT]),
            P_LENGTH => self.numeric(p, &[T_LENGTH]),
            P_ANGLE => self.numeric(p, &[T_ANGLE]),
            P_RESOLUTION => self.numeric(p, &[T_RESOLUTION]),
            P_NUMBER => self.numeric(p, &[T_NUMBER]),
            P_INTEGER => self.numeric(p, &[T_INTEGER]),
            P_COLOR_VALUES => self.color_values(p),
            P_CONSTANTS => self.constants(p),
            P_STRING => {
                let q = match c {
                    Some(b'\'') => STR_S,
                    Some(b'"') => STR_D,
                    _ => return None,
                };
                self.push(q, p);
                self.tok(p, p + 1, STRING_BEGIN);
                Some(p + 1)
            }
            P_IMPORTANT => {
                if c == Some(b'!') {
                    let s = self.spaces(p + 1);
                    if crate::has_at(b, s, b"important") {
                        self.tok(p, s + 9, IMPORTANT);
                        return Some(s + 9);
                    }
                }
                None
            }
            P_COMMA => {
                let s = self.spaces(p);
                if self.ch(s) == Some(b',') {
                    self.tok(s, s + 1, COMMA);
                    return Some(self.spaces(s + 1));
                }
                None
            }
            P_IDENT_RUN => {
                let mut e = self.ident(p)?;
                loop {
                    let s = self.spaces(e);
                    if s == e || s > n {
                        break;
                    }
                    match self.ident(s) {
                        Some(f) => e = f,
                        None => break,
                    }
                }
                self.tok(p, e, UNQUOTED);
                Some(e)
            }
            P_PROP_NAME_END => {
                if self.eol(p) || !matches!(c, Some(b'-' | b'a'..=b'z')) {
                    self.pop(p);
                    return Some(p);
                }
                None
            }
            P_VAR_DEPRECATED => {
                if crate::has_at(b, p, b"var-") && self.boundary(p) {
                    let e = self.ident(p + 4)?;
                    if self.space(e).is_some() {
                        self.tok(p, p + 4, DEPRECATED_PREFIX);
                        self.tok(p + 4, e, DEPRECATED_NAME);
                        return Some(e);
                    }
                }
                None
            }
            P_CUSTOM_PROP => {
                if c == Some(b'-') && b.get(p + 1) == Some(&b'-') {
                    let e = self.nmchars(p + 2);
                    if e > p + 2 {
                        self.tok(p, p + 2, CUSTOM_MARK);
                        self.tok(p + 2, e, CUSTOM_NAME);
                        return Some(e);
                    }
                }
                None
            }
            P_FONT => {
                if crate::has_at(b, p, b"font") && self.boundary(p) {
                    let e = if crate::has_at(b, p + 4, b"-family") {
                        p + 11
                    } else {
                        p + 4
                    };
                    // `(-family)?` backtracks when what follows fails
                    for e in [e, p + 4] {
                        if self.ch(e) != Some(b'-') && self.boundary(e) {
                            self.tok(p, e, PROPERTY_NAME);
                            self.push(FONT_AFTER, e);
                            return Some(e);
                        }
                    }
                }
                None
            }
            P_PROP_NAMES => {
                let (e, _) = self.ranked(p, |w| w.prop_rank)?;
                self.tok(p, e, PROPERTY_NAME);
                Some(e)
            }
            P_POP_EMPTY => {
                self.pop(p);
                Some(p)
            }
            P_SELECTOR_END => {
                if matches!(c, Some(b'/' | b'@' | b'{' | b')')) {
                    self.pop(p);
                    return Some(p);
                }
                None
            }
            P_CUSTOM_ELEMENT => self.custom_element(p),
            P_TAG => {
                if !self.boundary(p) || !c?.is_ascii_alphabetic() {
                    return None;
                }
                let mut e = p;
                while e < n && is(b[e], ALNUM) {
                    e += 1;
                }
                if !self.boundary(e) {
                    return None;
                }
                if lookup(&b[p..e]).lists & TAG == 0 {
                    return None;
                }
                self.tok(p, e, TAG_NAME);
                Some(e)
            }
            P_CLASS | P_ID => {
                let mark = if op == P_CLASS { b'.' } else { b'#' };
                if c == Some(mark) {
                    let e = self.ident(p + 1)?;
                    let (m, l) = if op == P_CLASS {
                        (CLASS_DOT, CLASS_NAME)
                    } else {
                        (ID_HASH, ID_NAME)
                    };
                    self.tok(p, p + 1, m);
                    self.tok(p + 1, e, l);
                    return Some(e);
                }
                None
            }
            P_WILDCARD => {
                if c == Some(b'*') {
                    self.tok(p, p + 1, WILDCARD);
                    return Some(p + 1);
                }
                None
            }
            P_COMBINATOR => self.combinator(p),
            P_PSEUDO => self.pseudo(p),
            P_ATTR_SEL => {
                if c == Some(b'[') {
                    self.push(ATTR_SEL, p);
                    self.tok(p, p + 1, ENTITY_MARK);
                    return Some(p + 1);
                }
                None
            }
            P_QUALIFIED => {
                // `(?:({{ident}})|(\*))?([|])(?!=)`
                let (e, leaf) = if let Some(e) = self.ident(p) {
                    (e, NS_PREFIX)
                } else if c == Some(b'*') {
                    (p + 1, NS_WILDCARD)
                } else {
                    (p, PLAIN)
                };
                for (e, leaf) in [(e, leaf), (p, PLAIN)] {
                    if b.get(e) == Some(&b'|') && b.get(e + 1) != Some(&b'=') {
                        if leaf != PLAIN {
                            self.tok(p, e, leaf);
                        }
                        self.tok(e, e + 1, NS_SEPARATOR);
                        return Some(e + 1);
                    }
                }
                None
            }
            P_ATTR_NAME | P_COUNTER_NAME => {
                let e = self.ident(p)?;
                self.tok(
                    p,
                    e,
                    if op == P_ATTR_NAME {
                        ATTR_NAME
                    } else {
                        COUNTER_NAME
                    },
                );
                Some(e)
            }
            P_ATTR_FN_NAME => {
                let e = self.ident(p)?;
                self.tok(p, e, ATTR_NAME);
                self.push(ATTR_FN_REST, e);
                Some(e)
            }
            P_ATTR_OP => {
                let s = self.spaces(p);
                let o = match self.ch(s) {
                    Some(b'=') => s + 1,
                    Some(b'~' | b'*' | b'|' | b'^' | b'$') if b.get(s + 1) == Some(&b'=') => s + 2,
                    _ => return None,
                };
                self.tok(s, o, ATTR_OP);
                let e = self.spaces(o);
                self.push(ATTR_VAL, e);
                Some(e)
            }
            P_CASE_FLAG => {
                if !fresh {
                    return None;
                }
                let s = self.spaces(p);
                if s > p && s < n && matches!(b[s], b'i' | b'I') {
                    self.tok(s, s + 1, KEYWORD_OTHER);
                    return Some(s + 1);
                }
                None
            }
            P_ATTR_SEL_END => {
                if c == Some(b']') {
                    self.tok(p, p + 1, ENTITY_MARK);
                    self.pop(p + 1);
                    return Some(p + 1);
                }
                None
            }
            P_ATTR_UNQUOTED => {
                let c = c?;
                if self.space(p).is_some() || matches!(c, b']' | b'[' | b'\'' | b'"') {
                    return None;
                }
                let e = self.next(p);
                self.tok(p, e, UNQUOTED);
                Some(e)
            }
            P_ATTR_VAL_END => {
                if c == Some(b']') || self.space(p).is_some() {
                    self.pop(p);
                    return Some(p);
                }
                None
            }
            P_GROUP_END => {
                if c == Some(b')') {
                    self.pop(p);
                    return Some(p);
                }
                None
            }
            P_CALL_END => {
                if c == Some(b')') {
                    self.tok(p, p + 1, CALL_END);
                    self.pop(p + 1);
                    return Some(p + 1);
                }
                None
            }
            P_GROUP => {
                if c == Some(b'(') {
                    self.tok(p, p + 1, GROUP_BEGIN);
                    self.push(ctx & !WRAP, p + 1);
                    return Some(p + 1);
                }
                None
            }
            P_UNQUOTED => {
                let c = c?;
                if self.space(p).is_some() || c == b'\'' || c == b'"' {
                    return None;
                }
                let e = self.next(p);
                self.tok(p, e, UNQUOTED);
                Some(e)
            }
            P_ADJ_OPS => {
                if matches!(c, Some(b'-' | b'+' | b'*')) && self.space(p + 1).is_some() {
                    self.tok(p, p + 1, OPERATOR);
                    return Some(p + 1);
                }
                None
            }
            P_EVEN_ODD => self.words(p, &[b"even", b"odd"], PSEUDO_KEYWORD),
            P_DROP_WORDS => self.words(p, &[b"active", b"valid", b"invalid"], PSEUDO_KEYWORD),
            P_SYMBOL_TYPES => self.words(
                p,
                &[b"cyclic", b"numeric", b"alphabetic", b"symbolic", b"fixed"],
                SYMBOL_TYPE,
            ),
            P_AUTO => self.words(p, &[b"auto"], CONSTANT),
            P_ROUND => self.words(p, &[b"round"], KEYWORD_OTHER),
            P_AT => self.words(p, &[b"at"], KEYWORD_OTHER),
            P_CIRCLE_PLACES => self.words(
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
                CONSTANT,
            ),
            P_FILL_RULES => self.words(p, &[b"nonzero", b"evenodd"], CONSTANT),
            P_REPETITIONS => self.words(p, &[b"auto-fill", b"auto-fit"], REPETITIONS),
            P_SIZES => self.words(p, &[b"max-content", b"min-content", b"auto"], CONSTANT),
            P_TO => self.words(p, &[b"to"], KEYWORD_OTHER),
            P_SIDES => self.words(p, &[b"top", b"right", b"bottom", b"left"], CONSTANT),
            P_RADIAL_WORDS => self.words(p, &[b"at", b"circle", b"ellipse"], KEYWORD_OTHER),
            P_RADIAL_PLACES => self.words(
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
                CONSTANT,
            ),
            P_COLOR_SPACES => self.words(p, &[b"rgb", b"hsl", b"hwb"], COLOR_SPACE),
            P_FROM_TO => self.words(p, &[b"from", b"to"], KF_SELECTOR),
            P_ORIENTATION => self.words(
                p,
                &[b"portrait", b"landscape", b"progressive", b"interlace"],
                CONSTANT,
            ),
            P_STEPS_WORDS => {
                for w in [&b"end"[..], b"middle", b"start"] {
                    if crate::has_at(b, p, w) {
                        self.tok(p, p + w.len(), TIMING_DIRECTION);
                        return Some(p + w.len());
                    }
                }
                None
            }
            P_COUNTER_STYLES => {
                // `\b({{counter_styles}}|none)\b`: `none` last, as written
                let (e, _) = self.ranked(p, |w| {
                    if w.lists & COUNTER_STYLE != 0 {
                        w.counter_rank
                    } else if w.lists & CONST != 0 && w.rank == NONE_RANK {
                        u16::MAX - 1
                    } else {
                        u16::MAX
                    }
                })?;
                self.tok(p, e, COUNTER_STYLE_LEAF);
                Some(e)
            }
            P_COUNTER_COMMA => {
                if c == Some(b',') {
                    let rest = if ctx == G_COUNTER {
                        COUNTER_REST
                    } else {
                        COUNTERS_REST
                    };
                    self.push(rest, p);
                    return Some(p);
                }
                None
            }
            P_NTH => self.nth(p),
            P_NTH_INVALID => self.nth_invalid(p),
            P_DENSITY => {
                let e = self.digits(p);
                if e > p && b.get(e) == Some(&b'x') {
                    self.tok(p, e, INTEGER);
                    self.tok(e, e + 1, INTEGER_UNIT);
                    return Some(e + 1);
                }
                None
            }
            P_UNITS => {
                if !self.boundary(p) || !self.word_at(p) {
                    return None;
                }
                let mut e = p;
                while e < n && b[e].is_ascii_alphabetic() {
                    e += 1;
                }
                if !self.boundary(e) {
                    return None;
                }
                let u = &b[p..e];
                if NUMERIC[..5].iter().any(|&t| unit_of(t, u)) {
                    self.tok(p, e, UNIT);
                    return Some(e);
                }
                None
            }
            P_CALC_END => {
                if c == Some(b')') {
                    self.stack[self.depth - 1] = TERMINATOR;
                    self.ctx = TERMINATOR;
                    self.rewrap();
                    self.fill(p);
                    return Some(p);
                }
                None
            }
            P_CALC_OP => {
                if matches!(c, Some(b'-' | b'/' | b'*' | b'+')) {
                    self.tok(p, p + 1, OPERATOR);
                    return Some(p + 1);
                }
                None
            }
            P_CALC_PAREN => {
                if c == Some(b'(') {
                    self.tok(p, p + 1, GROUP_BEGIN);
                    self.push(CALC_PARENS, p + 1);
                    return Some(p + 1);
                }
                None
            }
            P_SEMI => {
                if c == Some(b';') {
                    self.tok(p, p + 1, RULE_END);
                    return Some(p + 1);
                }
                None
            }
            P_SEMI_END => {
                if c == Some(b';') || self.eol(p) {
                    self.pop(p);
                    return Some(p);
                }
                None
            }
            P_CM_END => {
                if c == Some(b';') {
                    self.tok(p, p + 1, TERMINATOR_LEAF);
                    self.pop(p + 1);
                    return Some(p + 1);
                }
                None
            }
            P_CM_KEYWORD => {
                let s = self.spaces(p);
                if crate::has_at(b, s, b"@custom-media") {
                    self.at_keyword(s, s + 13, AT_CUSTOM_MEDIA);
                    self.push(CM, s + 13);
                    return Some(s + 13);
                }
                None
            }
            P_CM_INNER_END => {
                let s = self.spaces(p);
                if self.ch(s) == Some(b';') {
                    self.pop(s);
                    return Some(s);
                }
                None
            }
            P_MQL | P_KF_NAMES => {
                // `\s*(?=[^{;])`: the spaces up to the last one before a `{` or `;`
                let s = self.spaces(p);
                let e = match self.ch(s) {
                    Some(c) if c != b'{' && c != b';' => s,
                    _ => {
                        // the greedy `\s*` gives back a space: the lookahead sees it
                        if s > p { s - 1 } else { return None }
                    }
                };
                self.push(if op == P_MQL { MQL } else { KF_NAMES }, e);
                Some(e)
            }
            P_BLOCK => {
                if c == Some(b'{') {
                    self.push(BLOCK, p);
                    self.tok(p, p + 1, BLOCK_BEGIN);
                    return Some(p + 1);
                }
                None
            }
            P_BLOCK_END => {
                if c == Some(b'}') {
                    self.tok(p, p + 1, BLOCK_END);
                    self.pop(p + 1);
                    return Some(p + 1);
                }
                None
            }
            P_BRACE_AHEAD => {
                if c == Some(b'}') {
                    self.pop(p);
                    return Some(p);
                }
                None
            }
            P_KF_KEYWORD => {
                let s = self.spaces(p);
                if b.get(s) != Some(&b'@') {
                    return None;
                }
                let mut k = s + 1;
                let mut vendor = None;
                for v in [&b"-webkit-"[..], b"-moz-", b"-o-"] {
                    if b.get(k..k + v.len()) == Some(v) {
                        vendor = Some((k, k + v.len()));
                        k += v.len();
                        break;
                    }
                }
                if !crate::has_at(b, k, b"keyframes") {
                    return None;
                }
                let e = k + 9;
                self.tok(s, s + 1, AT_KEYFRAME + 1);
                match vendor {
                    Some((v, w)) => {
                        self.tok(v, w, KF_VENDOR);
                        self.tok(w, e, AT_KEYFRAME);
                    }
                    None => self.tok(s + 1, e, AT_KEYFRAME),
                }
                self.push(KF_HEAD, e);
                Some(e)
            }
            P_AT_BLOCK => {
                let s = self.spaces(p);
                if self.ch(s) == Some(b'{') {
                    self.tok(s, s + 1, PL_MARK);
                    self.push(if ctx == KF_OUT { KF_BODY } else { MEDIA_BODY }, s + 1);
                    return Some(s + 1);
                }
                None
            }
            P_SPACES_BRACE => {
                let s = self.spaces(p);
                if self.ch(s) == Some(b'{') {
                    self.pop(s);
                    return Some(s);
                }
                None
            }
            P_SPACES_BRACE_SEMI => {
                let s = self.spaces(p);
                if matches!(self.ch(s), Some(b'{' | b';')) {
                    self.pop(s);
                    return Some(s);
                }
                None
            }
            P_KF_NAME => {
                let s = self.spaces(p);
                let e = match self.ident(s) {
                    Some(e) => {
                        self.tok(s, e, ANIMATION_NAME);
                        e
                    }
                    None => s,
                };
                self.push(KF_NAME_END, e);
                Some(e)
            }
            P_KF_NAME_END => {
                let s = self.spaces(p);
                match self.ch(s) {
                    Some(b',') => {
                        self.tok(s, s + 1, REPETITION);
                        self.pop(s + 1);
                        Some(s + 1)
                    }
                    Some(b'{' | b';') => {
                        self.pop(s);
                        Some(s)
                    }
                    _ => None,
                }
            }
            P_MEDIA_KEYWORD => {
                let s = self.spaces(p);
                if crate::has_at(b, s, b"@media") {
                    self.at_keyword(s, s + 6, AT_MEDIA);
                    self.push(MEDIA_HEAD, s + 6);
                    return Some(s + 6);
                }
                None
            }
            P_MEDIA_TYPES => self.words_ci(
                p,
                &[
                    b"all",
                    b"aural",
                    b"braille",
                    b"embossed",
                    b"handheld",
                    b"print",
                    b"projection",
                    b"screen",
                    b"speech",
                    b"tty",
                    b"tv",
                ],
                MEDIA_TYPE,
            ),
            P_MEDIA_LOGIC => self.words_ci(p, &[b"and", b"or", b"not", b"only"], MEDIA_LOGIC),
            P_SUPPORTS_LOGIC => self.words_ci(p, &[b"and", b"or", b"not"], LOGIC),
            P_MQ_COMMA => {
                if c == Some(b',') {
                    self.tok(p, p + 1, REPETITION);
                    return Some(p + 1);
                }
                None
            }
            P_MQ_PAREN | P_SUP_PAREN => {
                if c == Some(b'(') {
                    self.push(
                        if op == P_MQ_PAREN {
                            MQ_PAREN
                        } else {
                            SUP_PAREN
                        },
                        p,
                    );
                    self.tok(p, p + 1, GROUP_BEGIN);
                    return Some(p + 1);
                }
                None
            }
            P_PAREN_END => {
                if c == Some(b')') {
                    self.tok(p, p + 1, GROUP_END);
                    self.pop(p + 1);
                    return Some(p + 1);
                }
                None
            }
            P_MEDIA_FEATURE => self.media_feature(p),
            P_RATIO => {
                let s = self.spaces(p);
                let a = self.digits(s);
                if a > s && b.get(a) == Some(&b'/') {
                    let d = self.digits(a + 1);
                    if d > a + 1 {
                        self.tok(s, a, INTEGER);
                        self.tok(a, a + 1, ARITHMETIC);
                        self.tok(a + 1, d, INTEGER);
                        return Some(d);
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// `@charset` … `@supports` at `p` (after spaces where the pattern takes them).
    fn at_rules(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let s = self.spaces(p);
        if b.get(s) != Some(&b'@') {
            return None;
        }
        // `((@)document)` and `((@)supports)` take no spaces before them
        let k = &b[s + 1..];
        if k.starts_with(b"charset") && self.boundary(s + 8) {
            self.at_keyword(s, s + 8, AT_CHARSET);
            let e = self.spaces(s + 8);
            self.push(CHARSET, e);
            return Some(e);
        }
        if k.starts_with(b"counter-style")
            && self.boundary(s + 14)
            && let Some(e) = self.counter_style(s)
        {
            return Some(e);
        }
        if k.starts_with(b"custom-media") && self.boundary(s + 13) {
            self.push(CM_OUT, p);
            return Some(p);
        }
        if s == p && k.starts_with(b"document") {
            self.at_keyword(s, s + 9, AT_DOCUMENT);
            self.push(DOCUMENT, s + 9);
            return Some(s + 9);
        }
        if k.starts_with(b"font-face") {
            let e = self.spaces(s + 10);
            if self.ch(e) == Some(b'{') || self.eol(e) {
                self.at_keyword(s, s + 10, AT_FONT_FACE);
                self.push(RULE_AT, e);
                return Some(e);
            }
        }
        if k.starts_with(b"import") && self.boundary(s + 7) {
            self.at_keyword(s, s + 7, AT_IMPORT);
            let e = self.spaces(s + 7);
            self.push(IMPORT, e);
            return Some(e);
        }
        {
            let mut v = s + 1;
            for x in [&b"-webkit-"[..], b"-moz-", b"-o-"] {
                if b.get(v..v + x.len()) == Some(x) {
                    v += x.len();
                    break;
                }
            }
            if b.get(v..v + 9) == Some(b"keyframes") && self.boundary(v + 9) {
                self.push(KF_OUT, p);
                return Some(p);
            }
        }
        if k.starts_with(b"media") && self.boundary(s + 6) {
            self.push(MEDIA_OUT, p);
            return Some(p);
        }
        if k.starts_with(b"namespace") {
            let w = self.spaces(s + 10);
            if w > s + 10 {
                // `\s+({{ident}}(?!{{nmchar}}|\())?`
                let mut e = w;
                if let Some(i) = self.ident(w)
                    && self.b.get(i) != Some(&b'(')
                    && self.nmchar(i).is_none()
                {
                    e = i;
                }
                self.at_keyword(s, s + 10, AT_NAMESPACE);
                if e > w {
                    self.tok(w, e, NS_PREFIX);
                }
                self.push(NAMESPACE, e);
                return Some(e);
            }
        }
        if k.starts_with(b"page") {
            // `\s*(?:(:)(first|left|right))?\s*(?=\{|$)`
            let w = self.spaces(s + 5);
            let mut pseudo = None;
            if b.get(w) == Some(&b':') {
                for x in [&b"first"[..], b"left", b"right"] {
                    if b.get(w + 1..w + 1 + x.len()) == Some(x) {
                        pseudo = Some((w, w + 1 + x.len()));
                        break;
                    }
                }
            }
            let mut ends = Vec::with_capacity(2);
            if let Some((_, e)) = pseudo {
                ends.push((self.spaces(e), pseudo));
            }
            ends.push((w, None));
            for (e, ps) in ends {
                if self.ch(e) == Some(b'{') || self.eol(e) {
                    self.at_keyword(s, s + 5, AT_PAGE);
                    if let Some((c, x)) = ps {
                        self.tok(c, c + 1, ENTITY_MARK);
                        self.tok(c + 1, x, PSEUDO_CLASS);
                    }
                    self.push(RULE_AT, e);
                    return Some(e);
                }
            }
        }
        if s == p && k.starts_with(b"supports") {
            self.at_keyword(s, s + 9, AT_SUPPORTS);
            self.push(SUPPORTS, s + 9);
            return Some(s + 9);
        }
        None
    }

    /// Whether `w` is at `i`: its first byte tested where it's asked, the compare after it left
    /// a call for its size (X-037: SCSS's at-rules' words tried in turn, a `memcmp` call each,
    /// were ~14 calls an at-rule; CSS's, ~1k at-rules a round, gained too little for its size).
    #[inline(always)]
    fn lit(&self, i: usize, w: &[u8]) -> bool {
        self.b.get(i) == Some(&w[0]) && self.b.get(i..i + w.len()) == Some(w)
    }

    /// `@` at `s` and the keyword to `e`, at-rule `leaf`'s two leaves.
    fn at_keyword(&mut self, s: usize, e: usize, leaf: u16) {
        self.tok(s, s + 1, leaf + 1);
        self.tok(s + 1, e, leaf);
    }

    /// `\s*((@)counter-style\b)\s+(?:(?i:\b(decimal|none)\b)|({{ident}}))?\s*(?=\{|$)` with its
    /// `@` at `s`.
    fn counter_style(&mut self, s: usize) -> Option<usize> {
        let k = s + 14;
        let w = self.spaces(k);
        if w == k {
            return None;
        }
        let mut tries: Vec<(usize, usize, u16)> = Vec::with_capacity(3);
        // the name, as the decimal|none alternative, then as an ident
        let we = {
            let mut e = w;
            while e < self.b.len() && is(self.b[e], ALNUM) {
                e += 1;
            }
            e
        };
        if self.boundary(w)
            && (self.b[w..we].eq_ignore_ascii_case(b"decimal")
                || self.b[w..we].eq_ignore_ascii_case(b"none"))
            && self.boundary(we)
        {
            tries.push((w, we, COUNTER_STYLE_INVALID));
        }
        if let Some(e) = self.ident(w) {
            tries.push((w, e, COUNTER_STYLE_NAME));
        }
        tries.push((w, w, PLAIN));
        for (a, e, leaf) in tries {
            let t = self.spaces(e);
            if self.ch(t) == Some(b'{') || self.eol(t) {
                self.at_keyword(s, k, AT_COUNTER_STYLE);
                if leaf != PLAIN {
                    self.tok(a, e, leaf);
                }
                self.push(RULE_AT, t);
                return Some(t);
            }
        }
        None
    }

    /// The function at `p` before a `(`, if it is of families `families`: its name, and its
    /// call and group pushed (`calc`'s call alone).
    fn function_at(&mut self, p: usize, families: u16) -> Option<usize> {
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
        self.tok(p, e, leaf);
        if group == CALC_W {
            self.push(CALC_W, e);
            return Some(e);
        }
        self.push(group | WRAP, e);
        self.tok(e, e + 1, GROUP_BEGIN);
        self.push(group, e + 1);
        Some(e + 1)
    }

    /// `unicode-range`: `(?xi)(u\+)([0-9a-f?]{1,6}(?:(-)[0-9a-f]{1,6})?)`.
    fn unicode_range(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if !matches!(b.get(p), Some(b'u' | b'U')) || b.get(p + 1) != Some(&b'+') {
            return None;
        }
        let s = p + 2;
        let mut e = s;
        while e < self.b.len() && e - s < 6 && (is_hex(b[e]) || b[e] == b'?') {
            e += 1;
        }
        if e == s {
            return None;
        }
        let mut dash = None;
        if b.get(e) == Some(&b'-') {
            let mut f = e + 1;
            while f < self.b.len() && f - e - 1 < 6 && is_hex(b[f]) {
                f += 1;
            }
            if f > e + 1 {
                dash = Some((e, f));
            }
        }
        self.tok(p, s, RANGE_PREFIX);
        self.tok(s, e, RANGE);
        if let Some((d, f)) = dash {
            self.tok(d, d + 1, RANGE_DASH);
            self.tok(d + 1, f, RANGE);
            return Some(f);
        }
        Some(e)
    }

    /// The first of `types` that matches at `p`, each tried in its patterns' order.
    fn numeric(&mut self, p: usize, types: &[u8]) -> Option<usize> {
        let b = self.b;
        let c = *b.get(p)?;
        if !(c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.')) {
            return None;
        }
        let num = self.num_at(p);
        // where the float and the integer end; the unit after each is read once, and only for a
        // type that has units (X-037: `%` and the plain numbers don't)
        let float = num.and_then(|nm| self.float_end(&nm));
        let int = num.filter(|nm| nm.int > nm.digits).map(|nm| nm.int);
        let mut units: Option<((usize, u8), (usize, u8))> = None;
        for &t in types {
            match t {
                T_PERCENT => {
                    if let Some(nm) = num {
                        if let Some(f) = float
                            && b.get(f) == Some(&b'%')
                        {
                            return Some(self.number_tok(p, &nm, f, f + 1, true));
                        }
                        if let Some(i) = int
                            && b.get(i) == Some(&b'%')
                        {
                            return Some(self.number_tok(p, &nm, i, i + 1, false));
                        }
                    }
                }
                T_NUMBER | T_INTEGER => {
                    if let Some(nm) = num {
                        if t == T_NUMBER
                            && let Some(f) = float
                        {
                            return Some(self.number_tok(p, &nm, f, f, true));
                        }
                        if let Some(i) = int {
                            return Some(self.number_tok(p, &nm, i, i, false));
                        }
                    }
                }
                _ => {
                    if let Some(nm) = num {
                        let (fu, iu) = *units.get_or_insert_with(|| {
                            let none = (0, NO_TYPE);
                            (
                                float.map_or(none, |f| self.unit_at(f)),
                                int.map_or(none, |i| self.unit_at(i)),
                            )
                        });
                        if let Some(f) = float
                            && fu.1 == t
                        {
                            return Some(self.number_tok(p, &nm, f, fu.0, true));
                        }
                        if let Some(i) = int
                            && iu.1 == t
                        {
                            return Some(self.number_tok(p, &nm, i, iu.0, false));
                        }
                    }
                    // `0\b(?!%)`
                    if (t == T_LENGTH || t == T_ANGLE)
                        && c == b'0'
                        && self.boundary(p + 1)
                        && b.get(p + 1) != Some(&b'%')
                    {
                        self.tok(p, p + 1, INTEGER);
                        return Some(p + 1);
                    }
                }
            }
        }
        None
    }

    /// Where `{{float}}` ends in `nm`: its first form, else its second.
    fn float_end(&self, nm: &Num) -> Option<usize> {
        nm.dot.map(|(_, e)| e).or(nm.exp)
    }

    /// The unit after a number that ends at `f`: where it could end, and the type it is a unit
    /// of when a `\b` follows it ([`NO_TYPE`] if none).
    fn unit_at(&self, f: usize) -> (usize, u8) {
        let u = self.unit_end(f);
        let t = if self.boundary(u) {
            unit_type(&self.b[f..u])
        } else {
            NO_TYPE
        };
        (u, t)
    }

    /// Where a unit after `i` could end: its run of letters.
    fn unit_end(&self, i: usize) -> usize {
        let (a, e) = self.letters_run.get();
        if a <= i && i <= e {
            return e;
        }
        let mut k = i;
        while k < self.b.len() && self.b[k].is_ascii_alphabetic() {
            k += 1;
        }
        self.letters_run.set((i, k));
        k
    }

    /// The number at `p` to `end`, its unit (or `%`) to `unit`: a float's with its `.`.
    fn number_tok(&mut self, p: usize, nm: &Num, end: usize, unit: usize, float: bool) -> usize {
        if float {
            match nm.dot {
                Some((dot, _)) => {
                    self.tok(p, dot, FLOAT);
                    self.tok(dot, dot + 1, FLOAT_DOT);
                    self.tok(dot + 1, end, FLOAT);
                }
                None => self.tok(p, end, FLOAT),
            }
            self.tok(end, unit, FLOAT_UNIT);
        } else {
            self.tok(p, end, INTEGER);
            self.tok(end, unit, INTEGER_UNIT);
        }
        unit
    }

    /// The word at `p` that one of `words` is, ending at a `\b` (the first in `words`' order).
    fn words(&mut self, p: usize, words: &[&[u8]], leaf: u16) -> Option<usize> {
        let b = self.b;
        if !b.get(p)?.is_ascii_alphabetic() || !self.boundary(p) {
            return None;
        }
        for w in words {
            let e = p + w.len();
            if crate::has_at(b, p, w) && self.boundary(e) {
                self.tok(p, e, leaf);
                return Some(e);
            }
        }
        None
    }

    /// As [`Scan::words`], ASCII case ignored.
    fn words_ci(&mut self, p: usize, words: &[&[u8]], leaf: u16) -> Option<usize> {
        let b = self.b;
        if !b.get(p)?.is_ascii_alphabetic() || !self.boundary(p) {
            return None;
        }
        for w in words {
            let e = p + w.len();
            if b.get(p..e).is_some_and(|x| x.eq_ignore_ascii_case(w)) && self.boundary(e) {
                self.tok(p, e, leaf);
                return Some(e);
            }
        }
        None
    }

    /// The word of the table at `p` that ends at a `\b`, the shortest: for lists whose words
    /// have no `-`, the only one.
    fn word_list_at(&self, p: usize) -> Option<(usize, Word)> {
        let b = self.b;
        if !b.get(p)?.is_ascii_alphabetic() || !self.boundary(p) {
            return None;
        }
        let end = self.name_end(p);
        let mut e = p + 1;
        while e <= end {
            if self.boundary(e) {
                let w = lookup(&b[p..e]);
                if w.lists & CSS_LISTS != 0 {
                    return Some((e, w));
                }
            }
            e += 1;
        }
        None
    }

    /// The table's word at `p` ending at a `\b` with the lowest `rank`: a list's regex match.
    fn ranked(&self, p: usize, rank: impl Fn(&Word) -> u16) -> Option<(usize, Word)> {
        let b = self.b;
        if !b.get(p)?.is_ascii_alphabetic() || !self.boundary(p) {
            return None;
        }
        let end = self.name_end(p);
        let mut best: Option<(usize, Word, u16)> = None;
        self.word_ends(p, end, |e, w| {
            let r = rank(&w);
            if r != u16::MAX && best.is_none_or(|(_, _, x)| r < x) {
                best = Some((e, w, r));
            }
        });
        best.map(|(e, w, _)| (e, w))
    }

    /// `color-values` at `p`.
    fn color_values(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if b.get(p) == Some(&b'#') {
            // `(#)(\h{3}|\h{6})\b`, then `(#)(\h{4}|\h{8})\b`
            let mut h = p + 1;
            while h < self.b.len() && is_hex(b[h]) {
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
            return None;
        }
        if let Some(e) = self.function_at(p, F_COLOR) {
            return Some(e);
        }
        let (e, w) = self.word_list_at(p)?;
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
    fn constants(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if crate::has_at(b, p, b"progid:DXImageTransform") && self.boundary(p) {
            // `progid:DXImageTransform.Microsoft.(Alpha|Blur|dropshadow|gradient|Shadow)\b`, its
            // `.`s any character
            let m = self.next(p + 23);
            if m < self.b.len() && crate::has_at(b, m, b"Microsoft") {
                let s = self.next(m + 9);
                for x in [
                    &b"Alpha"[..],
                    b"Blur",
                    b"dropshadow",
                    b"gradient",
                    b"Shadow",
                ] {
                    if s < self.b.len() && crate::has_at(b, s, x) && self.boundary(s + x.len()) {
                        self.tok(p, s + x.len(), CONSTANT);
                        return Some(s + x.len());
                    }
                }
            }
        }
        if let Some((e, _)) = self.ranked(p, |w| w.rank) {
            self.tok(p, e, CONSTANT);
            return Some(e);
        }
        // `\b(?i:sans-serif|serif|monospace|fantasy|cursive|system-ui)\b(?=\s*[;,\n}])`
        for w in [
            &b"sans-serif"[..],
            b"serif",
            b"monospace",
            b"fantasy",
            b"cursive",
            b"system-ui",
        ] {
            let e = p + w.len();
            if self.boundary(p)
                && b.get(p..e).is_some_and(|x| x.eq_ignore_ascii_case(w))
                && self.boundary(e)
            {
                // `\s*` gives the line's `\n` back to `[;,\n}]`
                let s = self.spaces(e);
                if s > self.b.len() || matches!(self.ch(s), Some(b';' | b',' | b'}')) {
                    self.tok(p, e, FONT_NAME_LEAF);
                    return Some(e);
                }
            }
        }
        None
    }

    /// Custom elements: `\b([a-z](?:{{custom_element_chars}})*-(?:{{custom_element_chars}})*)\b`:
    /// the longest run of the characters, with a `-` after its first, that ends at a `\b`.
    fn custom_element(&mut self, p: usize) -> Option<usize> {
        let (b, n) = (self.b, self.b.len());
        if !b.get(p)?.is_ascii_lowercase() || !self.boundary(p) || p < self.custom_failed.get() {
            return None;
        }
        let mut i = p + 1;
        let mut dash = false;
        let mut last = None;
        while i < n {
            let c = b[i];
            let e = if matches!(c, b'-' | b'_' | b'a'..=b'z' | b'0'..=b'9') {
                i + 1
            } else if c == b'\\' && b.get(i + 1) == Some(&b'.') && i + 1 < n {
                // the variable's `\\\.` in a block scalar: a backslash, then a `.`
                i + 2
            } else if c >= 0x80 {
                let (ch, len) = char_at(b, i);
                if !custom_element_wide(ch) {
                    break;
                }
                (i + len).min(n)
            } else {
                break;
            };
            dash |= c == b'-';
            i = e;
            if dash && self.boundary(i) {
                last = Some(i);
            }
        }
        let Some(e) = last else {
            // a later start in the run has its `-`s and `\b`s, so it fails as well
            self.custom_failed.set(i);
            return None;
        };
        self.tok(p, e, TAG_CUSTOM);
        Some(e)
    }

    /// `({{combinators}})(?![>~+])`, then `({{combinators}}){2,}` at `p`.
    fn combinator(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let comb = |c: Option<&u8>| matches!(c, Some(b'>' | b'~' | b'+'));
        // `>{1,3}` greedy, giving back `>`s for the lookahead; `~` and `+` one each
        let most = match b.get(p) {
            Some(b'>') => {
                let mut k = p + 1;
                while k < b.len() && k - p < 3 && b[k] == b'>' {
                    k += 1;
                }
                k
            }
            Some(b'~' | b'+') => p + 1,
            _ => return None,
        };
        for e in (p + 1..=most).rev() {
            if !comb(b.get(e)) {
                self.tok(p, e, COMBINATOR);
                return Some(e);
            }
            if b[p] != b'>' {
                break;
            }
        }
        // `{2,}`: each one greedy, as many as follow (none failed above, so two at least)
        let mut e = p;
        while comb(b.get(e)) {
            if b[e] == b'>' {
                let s = e;
                e += 1;
                while e < b.len() && e - s < 3 && b[e] == b'>' {
                    e += 1;
                }
            } else {
                e += 1;
            }
        }
        self.tok(p, e, BAD_COMBINATOR);
        Some(e)
    }

    /// `pseudo-elements`, then `pseudo-classes`, at `p`.
    fn pseudo(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if b.get(p) != Some(&b':') {
            return None;
        }
        // `(:{1,2})(?:before|after|first-line|first-letter)\b`
        for colons in [2, 1] {
            if colons == 2 && b.get(p + 1) != Some(&b':') {
                continue;
            }
            let s = p + colons;
            for w in [&b"before"[..], b"after", b"first-line", b"first-letter"] {
                let e = s + w.len();
                if crate::has_at(b, s, w) && self.boundary(e) {
                    self.tok(p, s, PSEUDO_ELEMENT_MARK);
                    self.tok(s, e, PSEUDO_ELEMENT);
                    return Some(e);
                }
            }
        }
        // `(::)(-(?:moz|ms|webkit)-)?(?:{{ident}})\b`
        if b.get(p + 1) == Some(&b':') {
            let s = p + 2;
            let mut starts = Vec::with_capacity(2);
            for v in [&b"-moz-"[..], b"-ms-", b"-webkit-"] {
                if b.get(s..s + v.len()) == Some(v) {
                    starts.push(Some(s + v.len()));
                }
            }
            starts.push(None);
            for v in starts {
                let i = v.unwrap_or(s);
                if let Some(e) = self.ident_then_boundary(i) {
                    self.tok(p, s, PSEUDO_ELEMENT_MARK);
                    if v.is_some() {
                        self.tok(s, i, PSEUDO_ELEMENT_VENDOR);
                    }
                    self.tok(i, e, PSEUDO_ELEMENT);
                    return Some(e);
                }
            }
        }
        // the functional pseudo-classes, then `(:)({{ident}})`
        let s = p + 1;
        let e = self.name_end(s);
        if b.get(e) == Some(&b'(') {
            let group = match &b[s..e] {
                b"dir" | b"lang" => Some(G_DIR_LANG),
                b"matches" | b"not" | b"has" => Some(G_SEL),
                b"drop" => Some(G_DROP),
                b"nth-last-child" | b"nth-child" | b"nth-last-of-type" | b"nth-of-type" => {
                    Some(G_NTH)
                }
                _ => None,
            };
            if let Some(g) = group {
                self.tok(p, s, PSEUDO_CLASS_MARK);
                self.tok(s, e, PSEUDO_CLASS);
                self.push(g | WRAP, e);
                self.tok(e, e + 1, GROUP_BEGIN);
                self.push(g, e + 1);
                return Some(e + 1);
            }
        }
        let e = self.ident(s)?;
        self.tok(p, s, PSEUDO_CLASS_MARK);
        self.tok(s, e, PSEUDO_CLASS);
        Some(e)
    }

    /// `{{ident}}\b` at `i`: the ident's ends in the regex's order (`--{{nmchar}}+` greedy,
    /// then `-?{{nmstart}}{{nmchar}}*` greedy), the first a `\b` follows.
    fn ident_then_boundary(&self, i: usize) -> Option<usize> {
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
        if let Some(q) = self.nmstart(s) {
            let mut k = q;
            let mut alt = vec![q];
            while let Some(r) = self.nmchar(k) {
                k = r;
                alt.push(k);
            }
            ends.extend(alt.into_iter().rev());
        }
        ends.into_iter().find(|&e| self.boundary(e))
    }

    /// `([-+]?\d*(n))\s*([-+]\s*\d+)?` at `p`.
    fn nth(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let mut i = p;
        if matches!(b.get(i), Some(b'-' | b'+')) {
            i += 1;
        }
        let d = self.digits(i);
        if b.get(d) != Some(&b'n') {
            return None;
        }
        let nend = d + 1;
        self.tok(p, d, INTEGER);
        self.tok(d, nend, INTEGER_UNIT);
        let s = self.spaces(nend);
        if matches!(b.get(s), Some(b'-' | b'+')) {
            let t = self.spaces(s + 1);
            let e = self.digits(t);
            if e > t {
                self.tok(s, e, INTEGER);
                return Some(e);
            }
        }
        Some(s.min(self.b.len() + 1))
    }

    /// `[-+]\s+\d+n?|[-+]?\d+\s+n` at `p`.
    fn nth_invalid(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if matches!(b.get(p), Some(b'-' | b'+')) {
            let s = self.spaces(p + 1);
            if s > p + 1 {
                let e = self.digits(s);
                if e > s {
                    let e = if b.get(e) == Some(&b'n') { e + 1 } else { e };
                    self.tok(p, e, BAD_NUMBER);
                    return Some(e);
                }
            }
        }
        let mut i = p;
        if matches!(b.get(i), Some(b'-' | b'+')) {
            i += 1;
        }
        let d = self.digits(i);
        if d > i {
            let s = self.spaces(d);
            if s > d && b.get(s) == Some(&b'n') {
                self.tok(p, s + 1, BAD_NUMBER);
                return Some(s + 1);
            }
        }
        None
    }

    /// The media features' group at `p`, CSS's and SCSS's: `(-webkit-|-o-)?(?:min-|max-)?(-moz-)?
    /// (?:(?:device-)?(?:height|width|aspect-ratio|pixel-ratio)|color(?:-index)?|monochrome|
    /// resolution)`, its end and its two vendor prefixes' places. Each optional part is taken
    /// where it's there, the names by their first byte (X-037: the alternatives tried in turn, a
    /// slice compare each, were ~8 `memcmp` calls a feature). Exact: where the rest fails after a
    /// part, no choice without it matches either, since a part left out leaves a `-`, `m` or `d`
    /// that nothing after it starts with (`monochrome` is no `min-` or `max-`). Out of line: one
    /// copy for both callers.
    #[inline(never)]
    fn feature_name(&self, p: usize) -> Option<(usize, Option<Span>, Option<Span>)> {
        let b = self.b;
        let at = |i: usize, w: &[u8]| crate::has_at(b, i, w);
        let mut k = p;
        let vendor = |k: &mut usize, w: &[u8]| {
            let v = at(*k, w).then(|| (*k, *k + w.len()));
            if v.is_some() {
                *k += w.len();
            }
            v
        };
        let v1 = vendor(&mut k, b"-webkit-").or_else(|| vendor(&mut k, b"-o-"));
        if at(k, b"min-") || at(k, b"max-") {
            k += 4;
        }
        let v3 = vendor(&mut k, b"-moz-");
        let l = if at(k, b"device-") { k + 7 } else { k };
        let w: &[u8] = match b.get(l)? {
            b'h' => b"height",
            b'w' => b"width",
            b'a' => b"aspect-ratio",
            b'p' => b"pixel-ratio",
            // `color(?:-index)?|monochrome|resolution`, at `k`: none after `device-`
            b'c' if l == k => {
                if at(k, b"color-index") {
                    b"color-index"
                } else {
                    b"color"
                }
            }
            b'm' if l == k => b"monochrome",
            b'r' if l == k => b"resolution",
            _ => return None,
        };
        at(l, w).then(|| (l + w.len(), v1, v3))
    }

    /// The media features at `p`: `((?:(-webkit-|-o-)?(?:min-|max-)?(-moz-)?(?:…))|grid|scan|
    /// orientation)\s*(:)?`.
    fn media_feature(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        // the bytes a feature's name can start with: its prefixes' and its names' first
        if !matches!(
            b.get(p),
            Some(b'-' | b'm' | b'd' | b'h' | b'w' | b'a' | b'p' | b'c' | b'r' | b'g' | b's' | b'o')
        ) {
            return None;
        }
        let at = |i: usize, w: &[u8]| crate::has_at(b, i, w);
        // the name's end, and its two vendor prefixes' places
        let best = self.feature_name(p).or_else(|| {
            [&b"grid"[..], b"scan", b"orientation"]
                .into_iter()
                .find(|w| at(p, w))
                .map(|w| (p + w.len(), None, None))
        });
        let (e, v1, v3) = best?;
        // the name's tokens, its vendor prefixes in it
        let mut i = p;
        for (a, z) in [v1, v3].into_iter().flatten() {
            self.tok(i, a, MEDIA_FEATURE);
            self.tok(a, z, MEDIA_FEATURE_VENDOR);
            i = z;
        }
        self.tok(i, e, MEDIA_FEATURE);
        let s = self.spaces(e);
        if self.ch(s) == Some(b':') {
            self.tok(s, s + 1, KEY_VALUE);
            return Some(s + 1);
        }
        Some(s.min(self.b.len() + 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_finds_its_words() {
        for e in ENTRIES {
            let w = lookup(e.word);
            assert_eq!(
                w.lists,
                e.exact | e.folded,
                "{}",
                String::from_utf8_lossy(e.word)
            );
            assert_eq!(w.prop_rank, e.prop_rank);
        }
        // the case-sensitive lists want the listed spelling, the others take any
        assert_eq!(lookup(b"Div").lists & TAG, 0);
        assert_ne!(lookup(b"clipPath").lists & TAG, 0);
        assert_ne!(lookup(b"CURRENTCOLOR").lists & SPECIAL_COLOR, 0);
        assert_ne!(lookup(b"Lower-Alpha").lists & COUNTER_STYLE, 0);
        assert_eq!(lookup(b"NONE").lists & CONST, 0);
        // the regexes' order: a longer word before a shorter one it starts with, or after
        assert!(lookup(b"auto-flow").rank < lookup(b"auto").rank);
        assert!(lookup(b"decimal-leading-zero").counter_rank < lookup(b"decimal").counter_rank);
        assert!(lookup(b"border-top").prop_rank > lookup(b"border-top-left-radius").prop_rank);
        assert_eq!(lookup(b"colour").lists, 0);
        assert_eq!(lookup(&[b'a'; 40]).lists, 0);
    }
}
