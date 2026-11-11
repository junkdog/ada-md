//! CSS on a scanner of ada's own (X-125), modern CSS designed in, from CSS's own syntax (CSS Syntax 3,
//! CSS Nesting 1) rather than bat's grammar: one loop over a line's bytes, a byte's class one load
//! from a table, an identifier read once, and what it is decided by where it stands, not by a
//! list: a declaration's name is a property whatever its spelling, a type selector a tag, a name
//! after `:` a pseudo-class, a name before `(` a function. One table ([`words`], ignoring ASCII
//! case as CSS does) names the value keywords, colors, font names and functions' groups.
//!
//! **What the scan keeps** ([`Scan`]): frames, four bits a level, for what a `{`, `(` or `[`
//! opened (a rule list, a block of declarations and nested rules, a keyframes block, a value's
//! braces, a function's arguments, a math function's, a selector's, an `:nth-*()`'s, a
//! condition's, an attribute selector), and the item a block is reading: its start, a selector,
//! a declaration's value, an at-rule's prelude (and which at-rule). Kinds are bat's leaves where
//! bat has the construct; no theme keys on a meta scope, so none are written.
//!
//! **Nesting** (CSS Nesting 1): a block of declarations holds nested rules and at-rules too. An
//! item starting with an identifier and `:` is a declaration where a blank or the line's end
//! follows the `:`; otherwise, as the specification parses it, a declaration unless a `{` comes
//! before its `;` or `}` (`a:hover{…}` a rule, `color:red;` a declaration), or the line ends on
//! a `,` (a selector list going on). Any other item is a rule (`&`, `.`, `#`, `:`, `[`, `*`, a
//! combinator, a tag before a blank).
//!
//! **Where CSS and bat's grammar part, CSS's reading** (each a departure, X-125's derailment
//! pass): a nested rule's `}` closes it, not its parent (bat's `&:hover {` closed the outer rule);
//! a custom property's value may hold `{…}` and `[…]` (bat's `}` there closed the rule); every
//! function is one, its arguments a group (`oklch(`, `color-mix(`, `clamp(` were plain, their `)`
//! lost); every at-rule is one, its block a rule list or a block of declarations as the at-rule
//! takes (`@layer`, `@container`, `@property`, `@scope`, `@starting-style`, …); a property's name
//! whatever its spelling (`inline-size` was split, `inherits` plain); a media feature whole
//! (`prefers-color-scheme` was split) and its range syntax's comparisons; a type selector a tag
//! in any case (`DIV`); a namespace's `|`; an attribute selector's unquoted value whole and its
//! `s` flag; an unclosed string ends with its line (bat's ran on); a missing `)` doesn't outlive
//! its declaration or selector.

pub(crate) mod hosted;
mod words;

use crate::emit::{Push, Rules, Run, Runs, Sink, Spans};
use crate::prim::{char_len, find_any};
use crate::words::{Words, text_len};
use crate::{Out, State};

// ---- the leaves ----

const PLAIN: u16 = 0;
const COMMENT: u16 = 1;
const COMMENT_MARK: u16 = 2;
/// A double-quoted string's four leaves, then a single-quoted one's at [`STR_S`]: the text, its
/// opening quote, its closing quote, an escape.
const STR_D: u16 = 3;
const STR_S: u16 = 7;
const BEGIN: u16 = 1;
const END: u16 = 2;
const ESC: u16 = 3;
/// A `\` before a line's end in a string, its double-quoted leaf; the single-quoted one's next.
const STR_NL: u16 = 11;
const UNQUOTED: u16 = 13;
const TAG: u16 = 14;
const TAG_CUSTOM: u16 = 15;
const WILDCARD: u16 = 16;
const CLASS_MARK: u16 = 17;
const CLASS: u16 = 18;
const ID_MARK: u16 = 19;
const ID: u16 = 20;
const PC_MARK: u16 = 21;
const PC: u16 = 22;
const PC_VENDOR: u16 = 23;
const PE_MARK: u16 = 24;
const PE: u16 = 25;
const PE_VENDOR: u16 = 26;
const NESTING: u16 = 27;
const COMBINATOR: u16 = 28;
const SEQUENCE: u16 = 29;
const ENTITY_MARK: u16 = 30;
const ATTR_NAME: u16 = 31;
const ATTR_OP: u16 = 32;
const KEYWORD: u16 = 33;
const NS_PREFIX: u16 = 34;
const NS_SEP: u16 = 35;
const GROUP_BEGIN: u16 = 36;
const GROUP_END: u16 = 37;
const BRACE: u16 = 38;
const PROP: u16 = 39;
const VENDOR: u16 = 40;
const CUSTOM_MARK: u16 = 41;
const CUSTOM_NAME: u16 = 42;
const KEY_VALUE: u16 = 43;
const TERMINATOR: u16 = 44;
const VALUE: u16 = 45;
const FONT_NAME: u16 = 46;
const COLOR_STD: u16 = 47;
const COLOR_EXT: u16 = 48;
const COLOR_SPECIAL: u16 = 49;
const COUNTER_STYLE: u16 = 50;
const HEX_MARK: u16 = 51;
const HEX: u16 = 52;
const HEXA_MARK: u16 = 53;
const HEXA: u16 = 54;
const INT: u16 = 55;
const INT_UNIT: u16 = 56;
const FLOAT: u16 = 57;
const FLOAT_DOT: u16 = 58;
const FLOAT_UNIT: u16 = 59;
const IMPORTANT: u16 = 60;
const ARITH: u16 = 61;
const COMPARISON: u16 = 62;
const MEDIA_TYPE: u16 = 63;
const MEDIA_LOGIC: u16 = 64;
const MEDIA_FEATURE: u16 = 65;
const LOGIC: u16 = 66;
const KF_SELECTOR: u16 = 67;
const ANIMATION_NAME: u16 = 68;
const COUNTER_STYLE_NAME: u16 = 69;
const LINES_BEGIN: u16 = 70;
const LINES_END: u16 = 71;
const LINE_NAME: u16 = 72;
const RANGE_PREFIX: u16 = 73;
const RANGE: u16 = 74;
const RANGE_DASH: u16 = 75;
const PC_KEYWORD: u16 = 76;
/// A function without a group of its own, then the groups, `support.function.X.css`.
const FN_GENERIC: u16 = 77;
const FN_ATTR: u16 = 78;
const FN_CALC: u16 = 79;
const FN_IMAGE: u16 = 80;
const FN_FILTER: u16 = 81;
const FN_GRADIENT: u16 = 82;
const FN_GRID: u16 = 83;
const FN_URL: u16 = 84;
const FN_VAR: u16 = 85;
const FN_COLOR: u16 = 86;
const FN_COUNTER: u16 = 87;
const FN_FONT_FACE: u16 = 88;
const FN_TRANSFORM: u16 = 89;
const FN_TIMING: u16 = 90;
const FN_SHAPE: u16 = 91;
const FN_TOGGLE: u16 = 92;
const FN_URL_PREFIX: u16 = 93;
const FN_DOMAIN: u16 = 94;
const FN_REGEXP: u16 = 95;
const FN_ANCHOR: u16 = 96;
const FN_TIMELINE: u16 = 97;
/// The at-rules' keywords from here, two leaves each in [`AT_NAMES`]' order: the name, then its
/// `@`.
const AT_FIRST: u16 = 98;

/// `@-webkit-keyframes`' vendor prefix, after the at-rules' leaves.
const KF_VENDOR: u16 = AT_FIRST + 2 * AT_SCOPES.len() as u16;
/// `counter(`'s and `counters(`'s counter.
const COUNTER_NAME: u16 = KF_VENDOR + 1;

/// The functions' groups, from [`FN_ATTR`].
static FN_GROUPS: [&str; 20] = [
    "attr",
    "calc",
    "image",
    "filter",
    "gradient",
    "grid",
    "url",
    "var",
    "color",
    "counter",
    "font-face",
    "transform",
    "timing",
    "shape",
    "toggle",
    "url-prefix",
    "domain",
    "regexp",
    "anchor",
    "timeline",
];

/// The at-rules' scopes, `keyword.control.at-rule.X.css` (bat's names where bat has one: its
/// `@keyframes` is `keyframe`), the last for an at-rule without a leaf of its own.
static AT_SCOPES: [&str; 22] = [
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
    "keyword.control.at-rule.layer.css",
    "keyword.control.at-rule.container.css",
    "keyword.control.at-rule.property.css",
    "keyword.control.at-rule.scope.css",
    "keyword.control.at-rule.starting-style.css",
    "keyword.control.at-rule.view-transition.css",
    "keyword.control.at-rule.font-palette-values.css",
    "keyword.control.at-rule.position-try.css",
    "keyword.control.at-rule.font-feature-values.css",
    "keyword.control.at-rule.page-margin.css",
    "keyword.control.at-rule.css",
];

/// The leaves below [`FN_GENERIC`].
static LEAVES: [&str; FN_GENERIC as usize] = [
    "",
    "comment.block.css",
    "comment.block.css punctuation.definition.comment.css",
    "string.quoted.double.css",
    "string.quoted.double.css punctuation.definition.string.begin.css",
    "string.quoted.double.css punctuation.definition.string.end.css",
    "string.quoted.double.css constant.character.escape.css",
    "string.quoted.single.css",
    "string.quoted.single.css punctuation.definition.string.begin.css",
    "string.quoted.single.css punctuation.definition.string.end.css",
    "string.quoted.single.css constant.character.escape.css",
    "string.quoted.double.css constant.character.escape.newline.css",
    "string.quoted.single.css constant.character.escape.newline.css",
    "string.unquoted.css",
    "entity.name.tag.css",
    "entity.name.tag.custom.css",
    "entity.name.tag.wildcard.css",
    "entity.other.attribute-name.class.css punctuation.definition.entity.css",
    "entity.other.attribute-name.class.css",
    "entity.other.attribute-name.id.css punctuation.definition.entity.css",
    "entity.other.attribute-name.id.css",
    "entity.other.pseudo-class.css punctuation.definition.entity.css",
    "entity.other.pseudo-class.css",
    "entity.other.pseudo-class.css support.type.vendor-prefix.css",
    "entity.other.pseudo-element.css punctuation.definition.entity.css",
    "entity.other.pseudo-element.css",
    "entity.other.pseudo-element.css support.type.vendor-prefix.css",
    "keyword.operator.ampersand.css",
    "punctuation.separator.combinator.css",
    "punctuation.separator.sequence.css",
    "punctuation.definition.entity.css",
    "entity.other.attribute-name.css",
    "keyword.operator.attribute-selector.css",
    "keyword.other.css",
    "entity.other.namespace-prefix.css",
    "punctuation.separator.namespace.css",
    "punctuation.definition.group.begin.css",
    "punctuation.definition.group.end.css",
    "punctuation.section.property-list.css",
    "support.type.property-name.css",
    "support.type.vendor-prefix.css",
    "support.type.custom-property.css punctuation.definition.custom-property.css",
    "support.type.custom-property.css support.type.custom-property.name.css",
    "punctuation.separator.key-value.css",
    "punctuation.terminator.rule.css",
    "support.constant.property-value.css",
    "support.constant.font-name.css",
    "support.constant.color.w3c-standard-color-name.css",
    "support.constant.color.w3c-extended-color-keywords.css",
    "support.constant.color.w3c-special-color-keyword.css",
    "support.constant.property-value.counter-style.css",
    "constant.other.color.rgb-value.css punctuation.definition.constant.css",
    "constant.other.color.rgb-value.css",
    "constant.other.color.rgba-value.css punctuation.definition.constant.css",
    "constant.other.color.rgba-value.css",
    "constant.numeric.integer.decimal.css",
    "constant.numeric.integer.decimal.css keyword.other.unit.css",
    "constant.numeric.float.decimal.css",
    "constant.numeric.float.decimal.css punctuation.separator.decimal.css",
    "constant.numeric.float.decimal.css keyword.other.unit.css",
    "keyword.other.important.css",
    "keyword.operator.arithmetic.css",
    "keyword.operator.comparison.css",
    "support.constant.media.css",
    "keyword.operator.logic.media.css",
    "support.type.property-name.media.css",
    "keyword.operator.logic.css",
    "keyword.keyframe-selector.css",
    "entity.other.animation-name.css",
    "entity.other.counter-style-name.css",
    "punctuation.section.begin.css",
    "punctuation.section.end.css",
    "string.unquoted.line-name.css",
    "support.unicode-range.css support.constant.unicode-range.prefix.css",
    "support.unicode-range.css constant.codepoint-range.css",
    "support.unicode-range.css constant.codepoint-range.css punctuation.section.range.css",
    "keyword.other.pseudo-class.css",
];

/// The scopes under `source.css` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    let split = |l: &'static str| l.split(' ').filter(|s| !s.is_empty()).collect();
    if kind < FN_GENERIC {
        return split(LEAVES[usize::from(kind)]);
    }
    if kind == FN_GENERIC {
        return vec!["support.function.css"];
    }
    if kind < AT_FIRST {
        return vec![FN_SCOPES[usize::from(kind - FN_ATTR)]];
    }
    if kind == COUNTER_NAME {
        return vec!["entity.other.counter-name.css", "string.unquoted.css"];
    }
    if kind == KF_VENDOR {
        return vec![
            "keyword.control.at-rule.keyframe.css",
            "support.type.property-vendor.css",
        ];
    }
    let i = usize::from(kind - AT_FIRST);
    match AT_SCOPES.get(i / 2) {
        Some(&s) if i % 2 == 0 => vec![s],
        Some(&s) => vec![s, "punctuation.definition.keyword.css"],
        None => Vec::new(),
    }
}

/// `support.function.X.css` for each group, from [`FN_ATTR`].
static FN_SCOPES: [&str; 20] = [
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
    "support.function.anchor.css",
    "support.function.timeline.css",
];

const _: () = assert!(FN_GROUPS.len() == FN_SCOPES.len());

/// The floor probe's build (X-125's `sol`, `ADA_PARSE_X125_SOL` set at build time): the loop, the
/// lookups and the frames without the readings that need an item's or a prelude's context (the
/// nesting decision's look-ahead, the at-rules' preludes, the selectors' functions).
const SOL: bool = option_env!("ADA_PARSE_X125_SOL").is_some();

// ---- the words ----

// A value word's leaf, the low byte of [`INFO`].
const W_CONST: u16 = VALUE;
const W_STD_COLOR: u16 = COLOR_STD;
const W_EXT_COLOR: u16 = COLOR_EXT;
const W_SPECIAL_COLOR: u16 = COLOR_SPECIAL;
const W_FONT_NAME: u16 = FONT_NAME;
const W_COUNTER_STYLE: u16 = COUNTER_STYLE;
/// A gradient's `to`, `at`, `circle`, `ellipse`: `keyword.other` in a function's arguments
/// (bat's), a value elsewhere.
const W_GRADIENT: u16 = KEYWORD;

const NW: usize = words::WORD_LIST.len();
static WORD_TEXT: [&[u8]; NW] = {
    let mut t: [&[u8]; NW] = [b""; NW];
    let mut i = 0;
    while i < NW {
        t[i] = words::WORD_LIST[i].0;
        i += 1;
    }
    t
};
static WORDS: Words<2048, NW, { text_len(&WORD_TEXT) }, true> = Words::new(&WORD_TEXT);
/// What each word is by id: its value leaf in the low byte, its function leaf in the high one.
static INFO: [u16; NW + 1] = {
    let mut t = [0u16; NW + 1];
    let mut i = 0;
    while i < NW {
        t[i + 1] = words::WORD_LIST[i].1 | words::WORD_LIST[i].2 << 8;
        i += 1;
    }
    t
};

// The at-rules: what each reads in its prelude and what its block holds.
/// A media query list: `@media`, `@custom-media`, and `@import`'s after its URL.
const A_MEDIA: u8 = 0;
const A_SUPPORTS: u8 = 1;
const A_IMPORT: u8 = 2;
/// Layer names: `@layer`.
const A_LAYER: u8 = 3;
/// A container's name, then its query.
const A_CONTAINER: u8 = 4;
const A_KEYFRAMES: u8 = 5;
/// A name, dashed or not: `@property`, `@counter-style`, `@font-palette-values`, `@position-try`,
/// `@font-feature-values`.
const A_NAME: u8 = 6;
/// A page selector: `@page`.
const A_PAGE: u8 = 7;
const A_NAMESPACE: u8 = 8;
/// `(scope) to (limit)`: `@scope`.
const A_SCOPE: u8 = 9;
/// Anything: idents plain, functions, strings, numbers.
const A_OTHER: u8 = 10;
/// A counter style's name: `@counter-style`.
const A_COUNTER: u8 = 11;
/// The prelude's kind is the low nibble of [`Scan::at`]; what the block holds the high one.
const AT_KIND: u8 = 15;
/// A block of rules (in a block of declarations, a block of both).
const AB_RULES: u8 = 0;
const AB_DECLS: u8 = 1 << 4;
const AB_KEYFRAMES: u8 = 2 << 4;

/// Each at-rule CSS has: its name, its leaf's index in [`AT_SCOPES`], its prelude, its block.
static AT_NAMES: [(&[u8], u8, u8); 34] = [
    (b"charset", 0, A_OTHER | AB_DECLS),
    (b"counter-style", 1, A_COUNTER | AB_DECLS),
    (b"custom-media", 2, A_MEDIA | AB_DECLS),
    (b"document", 3, A_OTHER | AB_RULES),
    (b"-moz-document", 3, A_OTHER | AB_RULES),
    (b"font-face", 4, A_OTHER | AB_DECLS),
    (b"import", 5, A_IMPORT | AB_DECLS),
    (b"keyframes", 6, A_KEYFRAMES | AB_KEYFRAMES),
    (b"-webkit-keyframes", 6, A_KEYFRAMES | AB_KEYFRAMES),
    (b"-moz-keyframes", 6, A_KEYFRAMES | AB_KEYFRAMES),
    (b"-o-keyframes", 6, A_KEYFRAMES | AB_KEYFRAMES),
    (b"media", 7, A_MEDIA | AB_RULES),
    (b"namespace", 8, A_NAMESPACE | AB_DECLS),
    (b"page", 9, A_PAGE | AB_DECLS),
    (b"supports", 10, A_SUPPORTS | AB_RULES),
    (b"layer", 11, A_LAYER | AB_RULES),
    (b"container", 12, A_CONTAINER | AB_RULES),
    (b"property", 13, A_NAME | AB_DECLS),
    (b"scope", 14, A_SCOPE | AB_RULES),
    (b"starting-style", 15, A_OTHER | AB_RULES),
    (b"view-transition", 16, A_OTHER | AB_DECLS),
    (b"font-palette-values", 17, A_NAME | AB_DECLS),
    (b"position-try", 18, A_NAME | AB_DECLS),
    (b"font-feature-values", 19, A_NAME | AB_RULES),
    (b"top-left-corner", 20, A_OTHER | AB_DECLS),
    (b"top-left", 20, A_OTHER | AB_DECLS),
    (b"top-center", 20, A_OTHER | AB_DECLS),
    (b"top-right", 20, A_OTHER | AB_DECLS),
    (b"top-right-corner", 20, A_OTHER | AB_DECLS),
    (b"bottom-left-corner", 20, A_OTHER | AB_DECLS),
    (b"bottom-left", 20, A_OTHER | AB_DECLS),
    (b"bottom-center", 20, A_OTHER | AB_DECLS),
    (b"bottom-right", 20, A_OTHER | AB_DECLS),
    (b"bottom-right-corner", 20, A_OTHER | AB_DECLS),
];
/// An at-rule CSS doesn't have (Tailwind's `@apply`, `@theme`, `@utility`, a preprocessor's): its
/// leaf's index, its prelude read as anything, its block a block of both.
const AT_UNKNOWN: (u8, u8) = (21, A_OTHER | AB_DECLS);

static AT_TEXT: [&[u8]; AT_NAMES.len()] = {
    let mut t: [&[u8]; AT_NAMES.len()] = [b""; AT_NAMES.len()];
    let mut i = 0;
    while i < AT_NAMES.len() {
        t[i] = AT_NAMES[i].0;
        i += 1;
    }
    t
};
static AT_WORDS: Words<128, { AT_NAMES.len() }, { text_len(&AT_TEXT) }, true> =
    Words::new(&AT_TEXT);
static AT_INFO: [(u8, u8); AT_NAMES.len() + 1] = {
    let mut t = [AT_UNKNOWN; AT_NAMES.len() + 1];
    let mut i = 0;
    while i < AT_NAMES.len() {
        t[i + 1] = (AT_NAMES[i].1, AT_NAMES[i].2);
        i += 1;
    }
    t
};

// ---- the bytes ----

const C_OTHER: u8 = 0;
/// ` `, `\t`, `\r`, `\x0b`, `\x0c`.
const C_BLANK: u8 = 1;
/// `[A-Za-z_]`.
const C_NAME: u8 = 2;
const C_DIGIT: u8 = 3;
const C_DASH: u8 = 4;
const C_DOT: u8 = 5;
const C_HASH: u8 = 6;
const C_COLON: u8 = 7;
const C_SEMI: u8 = 8;
const C_COMMA: u8 = 9;
const C_LBRACE: u8 = 10;
const C_RBRACE: u8 = 11;
const C_LPAREN: u8 = 12;
const C_RPAREN: u8 = 13;
const C_LBRACKET: u8 = 14;
const C_RBRACKET: u8 = 15;
const C_QUOTE: u8 = 16;
const C_SLASH: u8 = 17;
const C_AT: u8 = 18;
const C_BANG: u8 = 19;
const C_STAR: u8 = 20;
const C_PLUS: u8 = 21;
/// `>`, `~`.
const C_GT: u8 = 22;
const C_AMP: u8 = 23;
const C_PIPE: u8 = 24;
/// `<`, `=`.
const C_LT: u8 = 25;
const C_BACKSLASH: u8 = 26;
/// A byte past ASCII.
const C_WIDE: u8 = 27;
/// `^`, `$`: an attribute selector's operators' first byte.
const C_CARET: u8 = 28;

static BYTE_CLASS: [u8; 256] = {
    let mut t = [C_WIDE; 256];
    let mut c = 0;
    while c < 128 {
        t[c] = match c as u8 {
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => C_BLANK,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => C_NAME,
            b'0'..=b'9' => C_DIGIT,
            b'-' => C_DASH,
            b'.' => C_DOT,
            b'#' => C_HASH,
            b':' => C_COLON,
            b';' => C_SEMI,
            b',' => C_COMMA,
            b'{' => C_LBRACE,
            b'}' => C_RBRACE,
            b'(' => C_LPAREN,
            b')' => C_RPAREN,
            b'[' => C_LBRACKET,
            b']' => C_RBRACKET,
            b'"' | b'\'' => C_QUOTE,
            b'/' => C_SLASH,
            b'@' => C_AT,
            b'!' => C_BANG,
            b'*' => C_STAR,
            b'+' => C_PLUS,
            b'>' | b'~' => C_GT,
            b'&' => C_AMP,
            b'|' => C_PIPE,
            b'<' | b'=' => C_LT,
            b'\\' => C_BACKSLASH,
            b'^' | b'$' => C_CARET,
            _ => C_OTHER,
        };
        c += 1;
    }
    t
};

/// An identifier's ASCII bytes after its first: `[A-Za-z0-9_-]`.
static NAME: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 128 {
        t[c] = (c as u8).is_ascii_alphanumeric() || c == b'_' as usize || c == b'-' as usize;
        c += 1;
    }
    t
};

#[inline(always)]
fn class(c: u8) -> u8 {
    BYTE_CLASS[usize::from(c)]
}

#[inline(always)]
fn is_blank(c: u8) -> bool {
    class(c) == C_BLANK
}

/// Where the blanks at `p` end.
#[inline(always)]
fn blanks(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && is_blank(b[p]) {
        p += 1;
    }
    p
}

/// Whether an identifier starts at `p`: a letter, `_`, a byte past ASCII, an escape, or a `-`
/// before one of those or another `-`.
#[inline(always)]
fn ident_starts(b: &[u8], p: usize) -> bool {
    match b.get(p) {
        Some(&c) => match class(c) {
            C_NAME | C_WIDE => true,
            C_BACKSLASH => p + 1 < b.len(),
            C_DASH => match b.get(p + 1) {
                Some(&d) => {
                    matches!(class(d), C_NAME | C_WIDE | C_DASH) || (d == b'\\' && p + 2 < b.len())
                }
                None => false,
            },
            _ => false,
        },
        None => false,
    }
}

/// Where the identifier at `p` ends: its name bytes, characters past ASCII, escapes (a `\` and
/// the character after it; a hex escape's digits and one blank after them).
#[inline(always)]
fn ident_end(b: &[u8], p: usize) -> usize {
    let n = b.len();
    let mut e = p;
    loop {
        while e < n && NAME[usize::from(b[e])] {
            e += 1;
        }
        if e >= n {
            return n;
        }
        let c = b[e];
        if c >= 0x80 {
            e = (e + char_len(c)).min(n);
        } else if c == b'\\' && e + 1 < n {
            e = escape_end(b, e);
        } else {
            return e;
        }
    }
}

/// Where the escape at `p` (a `\` with a byte after it) ends: up to six hex digits and one blank
/// after them, or the character after the `\`.
#[inline(always)]
fn escape_end(b: &[u8], p: usize) -> usize {
    let n = b.len();
    let mut e = p + 1;
    if b[e].is_ascii_hexdigit() {
        while e < n && e < p + 7 && b[e].is_ascii_hexdigit() {
            e += 1;
        }
        if e < n && is_blank(b[e]) {
            e += 1;
        }
        e
    } else {
        (e + char_len(b[e])).min(n)
    }
}

/// Whether `w` is `lit`, ignoring ASCII case; `lit` lowercase.
#[inline(always)]
fn is(w: &[u8], lit: &[u8]) -> bool {
    w.len() == lit.len()
        && w.iter()
            .zip(lit)
            .all(|(&a, &b)| a.to_ascii_lowercase() == b)
}

/// Where a vendor prefix (`-webkit-`, `-moz-`, `-ms-`, `-o-`, …: a `-`, a name, a `-`) at the
/// start of identifier `w` ends; 0 for none.
#[inline(always)]
fn vendor_len(w: &[u8]) -> usize {
    if w.len() < 4 || w[0] != b'-' || w[1] == b'-' {
        return 0;
    }
    match w[1..].iter().position(|&c| c == b'-') {
        Some(i) if i > 0 && i + 2 < w.len() => i + 2,
        _ => 0,
    }
}

// ---- the frames: four bits a level in [`Scan::frames`] ----

/// The style sheet's top level (or a level lost past 32): rules and at-rules.
const F_TOP: u8 = 0;
/// A block of rules: `@media`'s, `@supports`', `@layer`'s, … at the top or in another.
const F_RULES: u8 = 1;
/// A block of declarations, nested rules and nested at-rules: a style rule's, `@font-face`'s,
/// `@page`'s, any at-rule's inside one.
const F_DECLS: u8 = 2;
/// `@keyframes`' block: keyframe selectors and their blocks.
const F_KEYFRAMES: u8 = 3;
/// `{ … }` in a declaration's value (a custom property's).
const F_BRACES: u8 = 4;
/// A function's arguments in a value.
const F_FN: u8 = 5;
/// A math function's arguments (`calc(`, `min(`, …) and a group in them: operators read.
const F_MATH: u8 = 6;
/// A selector's arguments: `:is(`, `:not(`, `:has(`, …, `@scope`'s `( … )`, `selector(`.
const F_SELP: u8 = 7;
/// An `:nth-*(`'s arguments, before an `of`.
const F_NTH: u8 = 8;
/// A condition's `( … )`: a media query's, a container query's, `@supports`', before its `:`.
const F_COND: u8 = 9;
/// A condition's `( … )` after its `:`: a value.
const F_COND_VAL: u8 = 10;
/// Arguments read as plain words: `:lang(`, `::part(`, an unknown prelude's group.
const F_ARGS: u8 = 11;
/// `[ … ]` in a value: grid line names, or a custom property's brackets.
const F_LINES: u8 = 12;
/// An attribute selector, `[`, before its operator.
const F_ATTR: u8 = 13;
/// An attribute selector after its operator, before its value.
const F_ATTR_VAL: u8 = 14;
/// An attribute selector after its value: its flag.
const F_ATTR_FLAG: u8 = 15;

/// Whether frame `f` is a block (a `{`'s that a `}` closes, the top level among them).
#[inline(always)]
fn is_block(f: u8) -> bool {
    f <= F_KEYFRAMES
}

// ---- what a block's item is reading: [`Scan::mode`] ----

/// An item's start.
const M_START: u8 = 0;
/// A selector, to its `{`.
const M_SEL: u8 = 1;
/// A declaration's value, to its `;` or `}`.
const M_VALUE: u8 = 2;
/// An at-rule's prelude, to its `{` or `;` ([`Scan::at`]).
const M_AT: u8 = 3;

// A value's flags, [`Scan::vflags`].
/// A custom property's value: anything, `{ … }` and `[ … ]` nesting.
const V_CUSTOM: u8 = 1;
/// `font`'s and `font-family`'s: a name no list has is a family's, `string.unquoted`.
const V_FONT: u8 = 2;

// What the line before left open, [`Scan::open`].
const O_NONE: u8 = 0;
const O_COMMENT: u8 = 1;
/// A string continued by a `\` at the line's end, its quote: `"` or `'`.
const O_STR_D: u8 = 2;
const O_STR_S: u8 = 3;

/// The state of a block of declarations alone, where HTML's `style="…"` starts.
pub(crate) fn rule_list_body() -> State {
    let mut s = State {
        depth: 1,
        ..State::default()
    };
    s.levels[0] = F_DECLS;
    s
}

/// Lex `b`, a line of CSS without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    line_on::<_, false>(b, 0, state, out, Spans::new());
}

/// Lex `b`, a line holding CSS from `from` (HTML's `<style>` and `style="…"`), from `state`, on
/// the vector as HTML has it (its spans not joined while it says so).
#[inline(never)]
pub(crate) fn line_from(b: &[u8], from: usize, state: &mut State, out: &mut Out) {
    line_on::<_, false>(b, from, state, out, Push);
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
    let mut k = line_on::<_, true>(b, 0, state, out, Runs::new(rules, runs));
    k.end_line(b.len());
}

#[inline(always)]
fn line_on<K: Sink, const SHARED: bool>(
    b: &[u8],
    from: usize,
    state: &mut State,
    out: &mut Out,
    k: K,
) -> K {
    let mut s = Scan::new(b, out, k);
    s.load(state);
    s.k.open(s.out, (b.len() - from.min(b.len())) / 3 + 16);
    s.lex_line::<SHARED>(from);
    s.k.close(s.out);
    *state = s.state();
    s.k
}

/// A block of CSS on one scan opened from `state` ([`crate::Carry`]).
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

struct Scan<'b, 'c, K: Sink> {
    b: &'b [u8],
    /// What each open frame is ([`F_RULES`]…), four bits a level, the innermost lowest; past 32
    /// levels the outer ones are lost (read as [`F_TOP`]).
    frames: u128,
    /// How many frames are open, up to 255.
    depth: u8,
    /// What the innermost block's item is reading ([`M_START`]…).
    mode: u8,
    /// In [`M_AT`], the at-rule's prelude and block ([`A_MEDIA`]…, [`AB_RULES`]…).
    at: u8,
    /// In [`M_VALUE`], the value's flags ([`V_CUSTOM`], [`V_FONT`]).
    vflags: u8,
    /// What the line ends inside ([`O_COMMENT`]…).
    open: u8,
    /// Hosted: the stretch ends at a `${` or the template's end, not the line's, so a string open
    /// there goes on.
    cut: bool,
    /// Hosted in another language's template ([`hosted`]): [`hosted::HOST_KIND`], ORed into each
    /// kind written; 0 else. A field, not a parameter: one copy of the scan a sink (X-125's R5).
    host: u16,
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
            mode: M_START,
            at: 0,
            vflags: 0,
            open: O_NONE,
            cut: false,
            host: 0,
            k,
            out,
        }
    }

    /// The scan's fields from `state`: the frames in its levels and the first two withs, the item
    /// in the next four.
    #[inline(always)]
    fn load(&mut self, state: &State) {
        let mut w = [0u8; 16];
        w[..14].copy_from_slice(&state.levels);
        w[14] = state.withs[0];
        w[15] = state.withs[1];
        self.frames = u128::from_le_bytes(w);
        self.depth = state.depth;
        self.mode = state.withs[2];
        self.at = state.withs[3];
        self.vflags = state.withs[4];
        self.open = state.withs[5];
    }

    /// The state the scan's last line left: every field as it is, so a block lexes as its lines
    /// do one at a time.
    #[inline(always)]
    fn state(&self) -> State {
        let w = self.frames.to_le_bytes();
        let mut s = State::default();
        s.levels.copy_from_slice(&w[..14]);
        s.withs[0] = w[14];
        s.withs[1] = w[15];
        s.withs[2] = self.mode;
        s.withs[3] = self.at;
        s.withs[4] = self.vflags;
        s.withs[5] = self.open;
        s.depth = self.depth;
        s
    }

    /// `b`, a block's next line, lexed on this scan, its sink closed; the scan's line an empty
    /// one again after.
    #[inline(always)]
    fn carry<const SHARED: bool>(&mut self, b: &'b [u8]) {
        self.k.start_line();
        self.b = b;
        self.lex_line::<SHARED>(0);
        self.k.close(self.out);
        self.b = b"";
    }

    /// A write: one call on the run sink ([`Sink::merge_call`]).
    #[inline(always)]
    fn put(&mut self, s: usize, e: usize, kind: u16) {
        self.k.merge_call(&mut self.out, s, e, kind | self.host);
    }

    /// A write inlined, for the common tokens.
    #[inline(always)]
    fn put_hot(&mut self, s: usize, e: usize, kind: u16) {
        self.k.merge(&mut self.out, s, e, kind | self.host);
    }

    #[inline(always)]
    fn top(&self) -> u8 {
        (self.frames & 15) as u8
    }

    #[inline(always)]
    fn push(&mut self, f: u8) {
        self.frames = self.frames << 4 | u128::from(f);
        self.depth = self.depth.saturating_add(1);
    }

    #[inline(always)]
    fn pop(&mut self) {
        if self.depth > 0 {
            self.frames >>= 4;
            self.depth -= 1;
        }
    }

    #[inline(always)]
    fn set_top(&mut self, f: u8) {
        self.frames = self.frames & !15 | u128::from(f);
    }

    /// The frames above the innermost block popped (a missing `)` or `]` doesn't outlive the
    /// item it opened in); in a value, those above a value's braces.
    #[inline(always)]
    fn pop_to_block(&mut self) {
        while self.depth > 0 && !is_block(self.top()) && self.top() != F_BRACES {
            self.pop();
        }
    }

    /// The line from `p`, from what the line before left: a comment's or a string's rest first;
    /// the loop out of line where `SHARED` ([`Scan::code_shared`]).
    #[inline(always)]
    fn lex_line<const SHARED: bool>(&mut self, mut p: usize) {
        match self.open {
            O_NONE => {}
            O_COMMENT => p = self.comment_line(p),
            o => {
                self.open = O_NONE;
                let base = if o == O_STR_D { STR_D } else { STR_S };
                p = self.string_body(p, base, if o == O_STR_D { b'"' } else { b'\'' });
            }
        }
        if SHARED {
            self.code_shared(p);
        } else {
            self.code(p);
        }
    }

    /// [`Scan::code`] out of line: on the run sink, one copy that the line API and a block's
    /// lines share; on the span sinks, inlined.
    #[inline(never)]
    fn code_shared(&mut self, p: usize) {
        self.code(p);
    }

    /// Whether the scan reads a selector at its place: in a selector's item, or in a selector's
    /// frame (`:is(`, `@scope`'s `( … )`, an attribute selector).
    #[inline(always)]
    fn in_selector(&self) -> bool {
        let top = self.top();
        if is_block(top) {
            self.mode == M_SEL
        } else {
            matches!(top, F_SELP | F_NTH | F_ATTR | F_ATTR_VAL | F_ATTR_FLAG)
        }
    }

    /// The line's code from `p`. A string or a comment is read where it opens and runs to its
    /// close or the line's end.
    #[inline(always)]
    fn code(&mut self, mut p: usize) {
        let b = self.b;
        let n = b.len();
        while p < n {
            let c = b[p];
            let was = p;
            p = match class(c) {
                C_BLANK => blanks(b, p + 1),
                C_NAME | C_WIDE => self.word(p),
                // an escape starts an identifier; a `\` at the line's end is nothing
                C_BACKSLASH if p + 1 < n => self.word(p),
                C_DASH => {
                    if b.get(p + 1) == Some(&b'-') && b.get(p + 2) == Some(&b'>') {
                        // `-->`, CSS's CDC: nothing
                        p + 3
                    } else if ident_starts(b, p) {
                        self.word(p)
                    } else if self.number_starts(p) {
                        self.number(p)
                    } else {
                        self.operator(p)
                    }
                }
                C_DIGIT => self.number(p),
                C_DOT => self.dot(p),
                C_HASH => self.hash(p),
                C_COLON => self.colon(p),
                C_SEMI => self.semicolon(p),
                C_COMMA => {
                    self.comma(p);
                    p + 1
                }
                C_LBRACE => self.lbrace(p),
                C_RBRACE => self.rbrace(p),
                C_LPAREN => self.lparen(p),
                C_RPAREN => self.rparen(p),
                C_LBRACKET => self.lbracket(p),
                C_RBRACKET => self.rbracket(p),
                C_QUOTE => self.string(p),
                C_SLASH => {
                    if b.get(p + 1) == Some(&b'*') {
                        self.comment(p)
                    } else {
                        self.operator(p)
                    }
                }
                C_AT => self.at_rule(p),
                C_BANG => self.bang(p),
                C_PLUS => {
                    if self.number_starts(p) && !self.in_selector() {
                        self.number(p)
                    } else {
                        self.operator(p)
                    }
                }
                C_STAR | C_GT | C_AMP | C_PIPE | C_LT | C_CARET => self.operator(p),
                _ => p + 1,
            };
            debug_assert!(p > was, "no progress at {was}");
        }
    }

    /// Where an item at a block's start is decided, before its first token: whether it is a
    /// rule's selector, a declaration, or (in a keyframes block) keyframe selectors. Returns the
    /// mode it takes.
    #[inline(always)]
    fn item(&mut self, p: usize) -> u8 {
        let top = self.top();
        let mode = if top == F_DECLS && self.is_declaration(p) {
            M_VALUE
        } else {
            M_SEL
        };
        self.mode = mode;
        self.vflags = 0;
        mode
    }

    /// Whether the item at `p` in a block of declarations is one: a custom property, or an
    /// identifier before a `:` (CSS Nesting's parse, [the module's notes](self)).
    #[inline(always)]
    fn is_declaration(&self, p: usize) -> bool {
        let b = self.b;
        if !ident_starts(b, p) {
            return false;
        }
        if b[p] == b'-' && b.get(p + 1) == Some(&b'-') {
            return true;
        }
        let q = blanks(b, ident_end(b, p));
        if b.get(q) != Some(&b':') {
            return false;
        }
        match b.get(q + 1) {
            None => true,
            Some(&c) if is_blank(c) => true,
            _ if SOL => true,
            _ => self.declaration_ahead(q + 1),
        }
    }

    /// From after a declaration's `:` with no blank after it: true unless a `{` comes before a
    /// `;` or `}` outside strings and comments, or the line ends on a `,`. It stops where the
    /// scan would end the item (a group doesn't hide a `;`, as the scan's `;` closes it), so a
    /// line's look-aheads never cover a byte twice.
    #[inline(never)]
    fn declaration_ahead(&self, mut r: usize) -> bool {
        let b = self.b;
        let n = b.len();
        let mut last = b':';
        while r < n {
            let c = b[r];
            match c {
                b'{' => return false,
                b';' | b'}' => return true,
                b'"' | b'\'' => {
                    r = string_end(b, r + 1, c);
                    last = c;
                    continue;
                }
                b'/' if b.get(r + 1) == Some(&b'*') => {
                    r = match find2(b, r + 2, b'*', b'/') {
                        Some(e) => e + 2,
                        None => return last != b',',
                    };
                    continue;
                }
                b'\\' => r += 1,
                _ => {}
            }
            if !is_blank(c) {
                last = c;
            }
            r += 1;
        }
        last != b','
    }

    /// The identifier at `p`, by where it stands.
    #[inline(always)]
    fn word(&mut self, p: usize) -> usize {
        let b = self.b;
        let e = ident_end(b, p);
        let top = self.top();
        if is_block(top) {
            let mode = match self.mode {
                M_START => {
                    if top == F_KEYFRAMES {
                        self.put_hot(p, e, KF_SELECTOR);
                        return e;
                    }
                    if self.item(p) == M_VALUE {
                        return self.declaration(p, e);
                    }
                    M_SEL
                }
                m => m,
            };
            return match mode {
                M_SEL => self.sel_word(p, e),
                M_VALUE => self.value_word(p, e),
                _ => self.prelude_word(p, e),
            };
        }
        match top {
            F_FN | F_MATH | F_BRACES | F_COND_VAL => self.value_word(p, e),
            F_LINES => {
                if b[p] == b'-' && b.get(p + 1) == Some(&b'-') {
                    return self.value_word(p, e);
                }
                self.put(p, e, LINE_NAME);
                e
            }
            F_SELP => self.sel_word(p, e),
            F_NTH => {
                let w = &b[p..e];
                if is(w, b"of") {
                    self.put(p, e, PC_KEYWORD);
                    self.set_top(F_SELP);
                } else if is(w, b"even") || is(w, b"odd") {
                    self.put(p, e, PC_KEYWORD);
                } else {
                    // `n`, `-n`, `n-1`: an+b's `n` a unit, as after a number
                    self.put(p, e, INT_UNIT);
                }
                e
            }
            F_COND => self.cond_word(p, e),
            F_ATTR => {
                if b.get(e) == Some(&b'|') && b.get(e + 1) != Some(&b'=') {
                    self.put(p, e, NS_PREFIX);
                    self.put(e, e + 1, NS_SEP);
                    return e + 1;
                }
                self.put(p, e, ATTR_NAME);
                e
            }
            F_ATTR_VAL => {
                self.put(p, e, UNQUOTED);
                self.set_top(F_ATTR_FLAG);
                e
            }
            F_ATTR_FLAG => {
                self.put(p, e, KEYWORD);
                e
            }
            _ => {
                // F_ARGS
                if b.get(e) == Some(&b'(') {
                    return self.function(p, e);
                }
                // a selector's arguments: `:lang(en)`, `::part(label)`
                if self.mode == M_SEL {
                    self.put(p, e, UNQUOTED);
                }
                e
            }
        }
    }

    /// A declaration's name `b[p..e]`, at an item's start: a property, its `:` and the blanks
    /// after; its value follows.
    #[inline(always)]
    fn declaration(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let w = &b[p..e];
        self.vflags = 0;
        if w.len() >= 2 && w[0] == b'-' && w[1] == b'-' {
            self.put_hot(p, p + 2, CUSTOM_MARK);
            self.put_hot(p + 2, e, CUSTOM_NAME);
            self.vflags = V_CUSTOM;
        } else {
            let v = vendor_len(w);
            if v > 0 {
                self.put(p, p + v, VENDOR);
            }
            self.put_hot(p + v, e, PROP);
            if !SOL && (is(w, b"font-family") || is(w, b"font")) {
                self.vflags = V_FONT;
            }
        }
        let q = blanks(b, e);
        // the `:` is there, but after a custom property's name, where the item was decided on
        // the `--`
        if b.get(q) != Some(&b':') {
            return e;
        }
        self.put_hot(q, q + 1, KEY_VALUE);
        q + 1
    }

    /// An identifier `b[p..e]` in a selector: a tag, a namespace's prefix before `|`.
    #[inline(always)]
    fn sel_word(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        if b.get(e) == Some(&b'|') && b.get(e + 1) != Some(&b'|') {
            self.put(p, e, NS_PREFIX);
            self.put(e, e + 1, NS_SEP);
            return e + 1;
        }
        let kind = if b[p..e].contains(&b'-') {
            TAG_CUSTOM
        } else {
            TAG
        };
        self.put_hot(p, e, kind);
        e
    }

    /// An identifier `b[p..e]` in a value: a function before `(`, a custom property's name, a
    /// keyword, a color, a font's name, a unicode range.
    #[inline(always)]
    fn value_word(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        if b.get(e) == Some(&b'(') {
            return self.function(p, e);
        }
        if b[p] == b'-' && b.get(p + 1) == Some(&b'-') {
            self.put_hot(p, p + 2, CUSTOM_MARK);
            self.put_hot(p + 2, e, CUSTOM_NAME);
            return e;
        }
        if e == p + 1 && matches!(b[p], b'u' | b'U') && b.get(e) == Some(&b'+') {
            return self.unicode_range(p);
        }
        let w = WORDS.value(&b[p..e], &INFO) & 0xff;
        if w != 0 {
            let w = if w == W_GRADIENT && self.top() != F_FN {
                VALUE
            } else {
                w
            };
            self.put_hot(p, e, w);
        } else if !SOL {
            self.value_word_rare(p, e);
        }
        e
    }

    /// An identifier `b[p..e]` in a value that no list has: a keyword after a vendor prefix, a
    /// font family's name.
    #[inline(never)]
    fn value_word_rare(&mut self, p: usize, e: usize) {
        let w = &self.b[p..e];
        let v = vendor_len(w);
        if v > 0 {
            let r = WORDS.value(&w[v..], &INFO) & 0xff;
            if r != 0 && r != W_GRADIENT {
                self.put(p, p + v, VENDOR);
                self.put(p + v, e, r);
                return;
            }
        }
        if self.vflags & V_FONT != 0 && self.top() != F_FN {
            self.put(p, e, UNQUOTED);
        }
    }

    /// A function's name `b[p..e]`, its `(` at `e`: its group, its arguments a frame (a math
    /// function's, a URL's text read whole).
    #[inline(never)]
    fn function(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let w = &b[p..e];
        let kind = WORDS.value(w, &INFO) >> 8;
        let kind = if kind == 0 { FN_GENERIC } else { kind };
        self.put(p, e, kind);
        self.put(e, e + 1, GROUP_BEGIN);
        // a URL's text, a font's name: `url(`, `local(`, `@document`'s `url-prefix(`, `domain(`
        if (kind == FN_URL && is(w, b"url"))
            || matches!(kind, FN_URL_PREFIX | FN_DOMAIN)
            || (kind == FN_FONT_FACE && is(w, b"local"))
        {
            return self.url(e + 1);
        }
        if matches!(kind, FN_ATTR | FN_COUNTER) && !SOL {
            // `attr(`'s attribute's name, `counter(`'s counter's
            self.push(F_FN);
            let q = blanks(b, e + 1);
            if ident_starts(b, q) {
                let t = ident_end(b, q);
                self.put(
                    q,
                    t,
                    if kind == FN_ATTR {
                        ATTR_NAME
                    } else {
                        COUNTER_NAME
                    },
                );
                return t;
            }
            return e + 1;
        }
        let f = if SOL {
            F_FN
        } else if self.in_selector() {
            F_SELP
        } else {
            match self.top() {
                F_COND | F_COND_VAL if is(w, b"style") => F_COND,
                F_COND | F_COND_VAL if is(w, b"selector") => F_SELP,
                F_MATH => F_MATH,
                _ if kind == FN_CALC => F_MATH,
                F_COND | F_COND_VAL => F_ARGS,
                _ if self.mode == M_AT && is_block(self.top()) => self.prelude_group(w),
                _ => F_FN,
            }
        };
        self.push(f);
        e + 1
    }

    /// The frame a function `w` opens in an at-rule's prelude: `@import`'s `supports(` a
    /// condition, `layer(` names.
    #[inline(never)]
    fn prelude_group(&mut self, w: &[u8]) -> u8 {
        match self.at & AT_KIND {
            A_IMPORT if is(w, b"supports") => {
                self.at = self.at & !AT_KIND | A_SUPPORTS;
                F_COND
            }
            A_IMPORT if is(w, b"layer") => F_ARGS,
            A_SUPPORTS if is(w, b"selector") => F_SELP,
            A_CONTAINER | A_MEDIA if is(w, b"style") => F_COND,
            _ => F_FN,
        }
    }

    /// `url(`'s text from `p`, after its `(`: a string's arguments as a function's; else the
    /// URL's text whole to its `)` (or the line's end).
    #[inline(never)]
    fn url(&mut self, p: usize) -> usize {
        let b = self.b;
        let q = blanks(b, p);
        if matches!(b.get(q), Some(b'"' | b'\'')) {
            self.push(F_FN);
            return q;
        }
        let mut e = q;
        while e < b.len() && b[e] != b')' {
            e += if b[e] == b'\\' && e + 1 < b.len() {
                2
            } else {
                1
            };
        }
        let e = e.min(b.len());
        let mut t = e;
        while t > q && is_blank(b[t - 1]) {
            t -= 1;
        }
        self.put(q, t, UNQUOTED);
        if e < b.len() {
            self.put(e, e + 1, GROUP_END);
            return e + 1;
        }
        e
    }

    /// `U+` at `p`: a unicode range, `U+0-7F`, `U+4??`.
    #[cold]
    fn unicode_range(&mut self, p: usize) -> usize {
        let b = self.b;
        let hex = |mut e: usize| {
            while e < b.len() && (b[e].is_ascii_hexdigit() || b[e] == b'?') {
                e += 1;
            }
            e
        };
        let e = hex(p + 2);
        if e == p + 2 {
            return p + 1;
        }
        self.put(p, p + 2, RANGE_PREFIX);
        self.put(p + 2, e, RANGE);
        if b.get(e) == Some(&b'-') && b.get(e + 1).is_some_and(u8::is_ascii_hexdigit) {
            let f = hex(e + 1);
            self.put(e, e + 1, RANGE_DASH);
            self.put(e + 1, f, RANGE);
            return f;
        }
        e
    }

    /// An identifier `b[p..e]` in a condition's `( … )`: a logic word, a function, a feature (a
    /// property's name in `@supports`'), a custom property.
    #[inline(never)]
    fn cond_word(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        if b.get(e) == Some(&b'(') {
            return self.function(p, e);
        }
        if b[p] == b'-' && b.get(p + 1) == Some(&b'-') {
            self.put(p, p + 2, CUSTOM_MARK);
            self.put(p + 2, e, CUSTOM_NAME);
            return e;
        }
        let w = &b[p..e];
        let supports = self.at & AT_KIND == A_SUPPORTS;
        if is(w, b"and") || is(w, b"or") || is(w, b"not") {
            self.put(p, e, if supports { LOGIC } else { MEDIA_LOGIC });
            return e;
        }
        let v = vendor_len(w);
        let kind = if supports { PROP } else { MEDIA_FEATURE };
        if v > 0 {
            self.put(p, p + v, VENDOR);
        }
        self.put(p + v, e, kind);
        e
    }

    /// An identifier `b[p..e]` in an at-rule's prelude, by the at-rule.
    #[inline(never)]
    fn prelude_word(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        if b.get(e) == Some(&b'(') {
            return self.function(p, e);
        }
        if b[p] == b'-' && b.get(p + 1) == Some(&b'-') {
            self.put(p, p + 2, CUSTOM_MARK);
            self.put(p + 2, e, CUSTOM_NAME);
            return e;
        }
        let w = &b[p..e];
        let kind = match self.at & AT_KIND {
            A_MEDIA | A_IMPORT | A_CONTAINER => {
                if is(w, b"and") || is(w, b"or") || is(w, b"not") || is(w, b"only") {
                    MEDIA_LOGIC
                } else if is_media_type(w) {
                    MEDIA_TYPE
                } else if self.at & AT_KIND == A_IMPORT && is(w, b"layer") {
                    KEYWORD
                } else {
                    PLAIN
                }
            }
            A_SUPPORTS => {
                if is(w, b"and") || is(w, b"or") || is(w, b"not") {
                    LOGIC
                } else {
                    PLAIN
                }
            }
            A_KEYFRAMES => ANIMATION_NAME,
            A_NAMESPACE => NS_PREFIX,
            A_SCOPE if is(w, b"to") => KEYWORD,
            A_COUNTER => COUNTER_STYLE_NAME,
            _ => PLAIN,
        };
        if kind != PLAIN {
            self.put(p, e, kind);
        }
        e
    }

    /// A number at `p` (`[+-]?`, digits with a fraction or not, an exponent), its unit after it.
    #[inline(always)]
    fn number(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut e = p;
        if matches!(b[e], b'+' | b'-') {
            e += 1;
        }
        while e < n && b[e].is_ascii_digit() {
            e += 1;
        }
        let mut dot = usize::MAX;
        if e + 1 < n && b[e] == b'.' && b[e + 1].is_ascii_digit() {
            dot = e;
            e += 2;
            while e < n && b[e].is_ascii_digit() {
                e += 1;
            }
        }
        if e < n && matches!(b[e], b'e' | b'E') {
            let s = if matches!(b.get(e + 1), Some(b'+' | b'-')) {
                e + 2
            } else {
                e + 1
            };
            if b.get(s).is_some_and(u8::is_ascii_digit) {
                e = s + 1;
                while e < n && b[e].is_ascii_digit() {
                    e += 1;
                }
                if dot == usize::MAX {
                    dot = usize::MAX - 1;
                }
            }
        }
        let (kind, unit) = if dot == usize::MAX {
            (INT, INT_UNIT)
        } else {
            (FLOAT, FLOAT_UNIT)
        };
        if dot < usize::MAX - 1 {
            self.put_hot(p, dot, FLOAT);
            self.put_hot(dot, dot + 1, FLOAT_DOT);
            self.put_hot(dot + 1, e, FLOAT);
        } else {
            self.put_hot(p, e, kind);
        }
        if e < n {
            if b[e] == b'%' {
                self.put_hot(e, e + 1, unit);
                return e + 1;
            }
            if ident_starts(b, e) {
                let u = ident_end(b, e);
                self.put_hot(e, u, unit);
                return u;
            }
        }
        e
    }

    /// Whether a number starts at `p`, a sign there: a digit, or `.` and a digit, after it.
    #[inline(always)]
    fn number_starts(&self, p: usize) -> bool {
        let b = self.b;
        match b.get(p + 1) {
            Some(c) if c.is_ascii_digit() => true,
            Some(b'.') => b.get(p + 2).is_some_and(u8::is_ascii_digit),
            _ => false,
        }
    }

    /// A `.` at `p`: a class in a selector, a number before a digit.
    #[inline(always)]
    fn dot(&mut self, p: usize) -> usize {
        let b = self.b;
        if self.mode == M_START && is_block(self.top()) {
            if self.top() == F_KEYFRAMES {
                return self.number_or_skip(p);
            }
            self.item(p);
        }
        if self.in_selector() {
            if ident_starts(b, p + 1) {
                let e = ident_end(b, p + 1);
                self.put_hot(p, p + 1, CLASS_MARK);
                self.put_hot(p + 1, e, CLASS);
                return e;
            }
            return p + 1;
        }
        self.number_or_skip(p)
    }

    #[inline(always)]
    fn number_or_skip(&mut self, p: usize) -> usize {
        if self.b.get(p + 1).is_some_and(u8::is_ascii_digit) {
            self.number(p)
        } else {
            p + 1
        }
    }

    /// A `#` at `p`: an id in a selector, a color in a value.
    #[inline(never)]
    fn hash(&mut self, p: usize) -> usize {
        let b = self.b;
        if self.mode == M_START && is_block(self.top()) {
            self.item(p);
        }
        if !ident_starts(b, p + 1) && !b.get(p + 1).is_some_and(u8::is_ascii_digit) {
            return p + 1;
        }
        let e = ident_end(b, p + 1);
        if self.in_selector() {
            self.put(p, p + 1, ID_MARK);
            self.put(p + 1, e, ID);
            return e;
        }
        if b[p + 1..e].iter().all(u8::is_ascii_hexdigit) {
            match e - p - 1 {
                3 | 6 => {
                    self.put(p, p + 1, HEX_MARK);
                    self.put(p + 1, e, HEX);
                }
                4 | 8 => {
                    self.put(p, p + 1, HEXA_MARK);
                    self.put(p + 1, e, HEXA);
                }
                _ => {}
            }
        }
        e
    }

    /// A `:` at `p`: a pseudo-class or a pseudo-element in a selector, a condition's separator.
    #[inline(never)]
    fn colon(&mut self, p: usize) -> usize {
        let top = self.top();
        if is_block(top) {
            if self.mode == M_START {
                if top == F_KEYFRAMES {
                    return p + 1;
                }
                self.item(p);
            }
            if self.mode == M_AT && self.at & AT_KIND == A_PAGE {
                return self.pseudo(p);
            }
        }
        if self.in_selector() {
            return self.pseudo(p);
        }
        if top == F_COND {
            self.put(p, p + 1, KEY_VALUE);
            self.set_top(F_COND_VAL);
        }
        p + 1
    }

    /// A pseudo-class or pseudo-element at `p`, its `:` or `::`; its arguments a frame.
    #[inline(never)]
    fn pseudo(&mut self, p: usize) -> usize {
        let b = self.b;
        let two = b.get(p + 1) == Some(&b':');
        let s = if two { p + 2 } else { p + 1 };
        if !ident_starts(b, s) {
            return s;
        }
        let e = ident_end(b, s);
        let w = &b[s..e];
        let element = two || is_old_element(w);
        let (mark, name, vendor) = if element {
            (PE_MARK, PE, PE_VENDOR)
        } else {
            (PC_MARK, PC, PC_VENDOR)
        };
        self.put(p, s, mark);
        let v = vendor_len(w);
        if v > 0 {
            self.put(s, s + v, vendor);
        }
        self.put(s + v, e, name);
        if b.get(e) != Some(&b'(') {
            return e;
        }
        self.put(e, e + 1, GROUP_BEGIN);
        let f = if SOL { F_SELP } else { pseudo_frame(&w[v..]) };
        self.push(f);
        e + 1
    }

    /// A `;` at `p`: a declaration's or an at-rule's end, but in a value's braces.
    #[inline(always)]
    fn semicolon(&mut self, p: usize) -> usize {
        self.pop_to_block();
        if self.top() != F_BRACES {
            self.mode = M_START;
        }
        self.put_hot(p, p + 1, TERMINATOR);
        p + 1
    }

    /// A `,` at `p`.
    #[inline(always)]
    fn comma(&mut self, p: usize) {
        let top = self.top();
        if self.mode == M_START && is_block(top) && top != F_KEYFRAMES {
            self.item(p);
        }
        self.put_hot(p, p + 1, SEQUENCE);
    }

    /// A `{` at `p`: a block's, by the item it ends; a value's braces.
    #[inline(always)]
    fn lbrace(&mut self, p: usize) -> usize {
        self.put_hot(p, p + 1, BRACE);
        if self.mode == M_VALUE {
            self.push(F_BRACES);
            return p + 1;
        }
        let mode = self.mode;
        self.pop_to_block();
        let outer = self.top();
        let f = if outer == F_KEYFRAMES {
            F_DECLS
        } else if mode == M_AT && !SOL {
            match self.at & !AT_KIND {
                AB_KEYFRAMES => F_KEYFRAMES,
                AB_RULES if outer != F_DECLS => F_RULES,
                _ => F_DECLS,
            }
        } else {
            F_DECLS
        };
        self.push(f);
        self.mode = M_START;
        p + 1
    }

    /// A `}` at `p`: a value's braces', or the innermost block's.
    #[inline(always)]
    fn rbrace(&mut self, p: usize) -> usize {
        self.pop_to_block();
        let top = self.top();
        if top == F_TOP {
            self.mode = M_START;
            return p + 1;
        }
        self.put_hot(p, p + 1, BRACE);
        self.pop();
        if top != F_BRACES {
            self.mode = M_START;
        }
        p + 1
    }

    /// A `(` at `p`, not a function's: a condition's in a prelude, a group in a value.
    #[inline(never)]
    fn lparen(&mut self, p: usize) -> usize {
        let top = self.top();
        if is_block(top) && self.mode == M_START && top != F_KEYFRAMES {
            self.item(p);
        }
        let f = if is_block(top) {
            match self.mode {
                M_AT if !SOL => match self.at & AT_KIND {
                    A_MEDIA | A_SUPPORTS | A_CONTAINER | A_IMPORT => F_COND,
                    A_SCOPE => F_SELP,
                    _ => F_ARGS,
                },
                M_VALUE => F_FN,
                M_SEL => F_SELP,
                _ => F_ARGS,
            }
        } else {
            match top {
                F_COND => F_COND,
                F_MATH => F_MATH,
                F_SELP | F_NTH => F_SELP,
                F_FN | F_COND_VAL | F_BRACES | F_LINES => F_FN,
                _ => F_ARGS,
            }
        };
        self.put(p, p + 1, GROUP_BEGIN);
        self.push(f);
        p + 1
    }

    /// A `)` at `p`: a group's end, where one is open.
    #[inline(never)]
    fn rparen(&mut self, p: usize) -> usize {
        // an attribute selector left open in a selector's group
        while matches!(self.top(), F_ATTR | F_ATTR_VAL | F_ATTR_FLAG | F_LINES) {
            self.pop();
        }
        if is_block(self.top()) || self.top() == F_BRACES {
            return p + 1;
        }
        self.put_hot(p, p + 1, GROUP_END);
        self.pop();
        p + 1
    }

    /// A `[` at `p`: an attribute selector, or a value's brackets (grid line names).
    #[inline(never)]
    fn lbracket(&mut self, p: usize) -> usize {
        let top = self.top();
        if is_block(top) && self.mode == M_START && top != F_KEYFRAMES {
            self.item(p);
        }
        if self.in_selector() {
            self.put(p, p + 1, ENTITY_MARK);
            self.push(F_ATTR);
        } else {
            self.put(p, p + 1, LINES_BEGIN);
            self.push(F_LINES);
        }
        p + 1
    }

    /// A `]` at `p`.
    #[inline(never)]
    fn rbracket(&mut self, p: usize) -> usize {
        match self.top() {
            F_ATTR | F_ATTR_VAL | F_ATTR_FLAG => {
                self.put(p, p + 1, ENTITY_MARK);
                self.pop();
            }
            F_LINES => {
                self.put(p, p + 1, LINES_END);
                self.pop();
            }
            _ => {}
        }
        p + 1
    }

    /// `@` at `p`: an at-rule's keyword at an item's start, and where a selector was left
    /// unfinished (a selector holds no `@`); plain elsewhere, a value's among them (GTK's
    /// `@theme_fg_color`, a custom property's anything).
    #[inline(never)]
    fn at_rule(&mut self, p: usize) -> usize {
        let b = self.b;
        if !(matches!(self.mode, M_START | M_SEL) && is_block(self.top()))
            || !ident_starts(b, p + 1)
        {
            return p + 1;
        }
        let e = ident_end(b, p + 1);
        let (leaf, at) = AT_WORDS.value(&b[p + 1..e], &AT_INFO);
        let kind = AT_FIRST + 2 * u16::from(leaf);
        self.put(p, p + 1, kind + 1);
        // `@-webkit-keyframes`: its prefix as bat splits it
        let v = if at & AT_KIND == A_KEYFRAMES {
            vendor_len(&b[p + 1..e])
        } else {
            0
        };
        self.put(p + 1, p + 1 + v, KF_VENDOR);
        self.put(p + 1 + v, e, kind);
        self.mode = M_AT;
        self.at = at;
        self.vflags = 0;
        e
    }

    /// `!` at `p`: `!important` in a value, its blanks with it.
    #[inline(never)]
    fn bang(&mut self, p: usize) -> usize {
        let b = self.b;
        let q = blanks(b, p + 1);
        if ident_starts(b, q) {
            let e = ident_end(b, q);
            if is(&b[q..e], b"important") {
                self.put(p, e, IMPORTANT);
                return e;
            }
        }
        p + 1
    }

    /// An operator's byte at `p`, by where it stands: a combinator, `*`, `&` or `|` in a selector,
    /// an attribute selector's operator, a comparison in a condition, arithmetic in a math
    /// function.
    #[inline(never)]
    fn operator(&mut self, p: usize) -> usize {
        let b = self.b;
        let c = b[p];
        let top = self.top();
        if self.mode == M_START && is_block(top) && top != F_KEYFRAMES {
            if c == b'<' && b[p..].starts_with(b"<!--") {
                return p + 4;
            }
            self.item(p);
        }
        match top {
            F_ATTR | F_ATTR_VAL if matches!(c, b'=' | b'~' | b'|' | b'^' | b'$' | b'*') => {
                let e = if c == b'=' {
                    p + 1
                } else if b.get(p + 1) == Some(&b'=') {
                    p + 2
                } else {
                    return p + 1;
                };
                self.put(p, e, ATTR_OP);
                self.set_top(F_ATTR_VAL);
                return e;
            }
            F_COND | F_COND_VAL if matches!(c, b'<' | b'>' | b'=') => {
                let e = if c != b'=' && b.get(p + 1) == Some(&b'=') {
                    p + 2
                } else {
                    p + 1
                };
                self.put(p, e, COMPARISON);
                return e;
            }
            F_MATH if matches!(c, b'+' | b'-' | b'*' | b'/') => {
                self.put(p, p + 1, ARITH);
                return p + 1;
            }
            // an+b's sign
            F_NTH if matches!(c, b'+' | b'-') => {
                self.put(p, p + 1, ARITH);
                return p + 1;
            }
            _ => {}
        }
        if self.in_selector() {
            match c {
                b'>' | b'~' | b'+' => self.put_hot(p, p + 1, COMBINATOR),
                b'*' => self.put_hot(p, p + 1, WILDCARD),
                b'&' => self.put_hot(p, p + 1, NESTING),
                b'|' if b.get(p + 1) == Some(&b'|') => {
                    self.put(p, p + 2, COMBINATOR);
                    return p + 2;
                }
                b'|' => self.put(p, p + 1, NS_SEP),
                _ => {}
            }
        }
        p + 1
    }

    /// A string from its quote at `p`.
    #[inline(never)]
    fn string(&mut self, p: usize) -> usize {
        let b = self.b;
        let top = self.top();
        if self.mode == M_START && is_block(top) && top != F_KEYFRAMES {
            self.item(p);
        }
        let q = b[p];
        let base = if q == b'"' { STR_D } else { STR_S };
        self.put(p, p + 1, base + BEGIN);
        if top == F_ATTR_VAL {
            self.set_top(F_ATTR_FLAG);
        }
        self.string_body(p + 1, base, q)
    }

    /// A string's text from `p` to its quote `q` (or the line's end: an unclosed string ends
    /// with its line, a `\` before the end continues it).
    #[inline(always)]
    fn string_body(&mut self, p: usize, base: u16, q: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let mut s = p;
        let mut e = p;
        while e < n {
            let c = b[e];
            if c == q {
                self.put_hot(s, e, base);
                self.put_hot(e, e + 1, base + END);
                return e + 1;
            }
            if c == b'\\' {
                self.put(s, e, base);
                if e + 1 == n {
                    self.put(e, n, STR_NL + (base - STR_D) / 4);
                    self.open = if q == b'"' { O_STR_D } else { O_STR_S };
                    return n;
                }
                let t = escape_end(b, e);
                self.put(e, t, base + ESC);
                s = t;
                e = t;
                continue;
            }
            e += 1;
        }
        self.put(s, n, base);
        if self.cut {
            self.open = if q == b'"' { O_STR_D } else { O_STR_S };
        }
        n
    }

    /// A comment from its `/*` at `p`.
    #[inline(never)]
    fn comment(&mut self, p: usize) -> usize {
        self.put(p, p + 2, COMMENT_MARK);
        self.comment_rest(p + 2)
    }

    /// A comment's line from `p`, the comment open from the line before: a `*` first on it (a
    /// gutter, `/**`'s style) its punctuation, as bat reads it.
    #[inline(never)]
    fn comment_line(&mut self, p: usize) -> usize {
        let b = self.b;
        let q = blanks(b, p);
        if b.get(q) == Some(&b'*')
            && b.get(q + 1) != Some(&b'/')
            && b[..p].iter().all(|&c| is_blank(c))
        {
            self.put(p, q, COMMENT);
            self.put(q, q + 1, COMMENT_MARK);
            return self.comment_rest(q + 1);
        }
        self.comment_rest(p)
    }

    /// A comment's text from `p` to its `*/`, or the line's end (the comment left open).
    #[inline(never)]
    fn comment_rest(&mut self, p: usize) -> usize {
        let b = self.b;
        match find2(b, p, b'*', b'/') {
            Some(e) => {
                self.put(p, e, COMMENT);
                self.put(e, e + 2, COMMENT_MARK);
                self.open = O_NONE;
                e + 2
            }
            None => {
                self.put(p, b.len(), COMMENT);
                self.open = O_COMMENT;
                b.len()
            }
        }
    }
}

/// Where the string whose text starts at `p` ends, its quote `q` (or the line's end).
fn string_end(b: &[u8], mut p: usize, q: u8) -> usize {
    while p < b.len() {
        match b[p] {
            c if c == q => return p + 1,
            b'\\' => p += 2,
            _ => p += 1,
        }
    }
    b.len()
}

/// The first place from `p` where `x` and then `y` stand.
#[inline(always)]
fn find2(b: &[u8], mut p: usize, x: u8, y: u8) -> Option<usize> {
    while p + 1 < b.len() {
        let i = find_any(b, p, [x]);
        if i + 1 >= b.len() {
            return None;
        }
        if b[i + 1] == y {
            return Some(i);
        }
        p = i + 1;
    }
    None
}

/// CSS 2's pseudo-elements, which take one `:` as well as two.
fn is_old_element(w: &[u8]) -> bool {
    is(w, b"before") || is(w, b"after") || is(w, b"first-line") || is(w, b"first-letter")
}

/// The frame a functional pseudo-class's or pseudo-element's `(` opens, by its name (the vendor
/// prefix off): selectors, an+b, or plain words.
fn pseudo_frame(w: &[u8]) -> u8 {
    if w.len() > 4 && is(&w[..4], b"nth-") {
        return F_NTH;
    }
    const SELECTORS: [&[u8]; 14] = [
        b"not",
        b"is",
        b"where",
        b"has",
        b"matches",
        b"any",
        b"host",
        b"host-context",
        b"slotted",
        b"current",
        b"past",
        b"future",
        b"cue",
        b"global",
    ];
    if SELECTORS.iter().any(|s| is(w, s)) || is(w, b"local") {
        F_SELP
    } else {
        F_ARGS
    }
}

/// The media types.
fn is_media_type(w: &[u8]) -> bool {
    const TYPES: [&[u8]; 11] = [
        b"all",
        b"print",
        b"screen",
        b"speech",
        b"tv",
        b"projection",
        b"handheld",
        b"braille",
        b"embossed",
        b"tty",
        b"aural",
    ];
    TYPES.iter().any(|t| is(w, t))
}

#[cfg(test)]
mod tests;
