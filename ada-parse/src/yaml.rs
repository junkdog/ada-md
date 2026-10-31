//! YAML, in place of bat's `YAML`: its 47 contexts on the [machine](crate::machine), context
//! for context, each regex a matcher below. Flow collections, quoted strings and block
//! scalars run on across lines; a block scalar's body ends at the first line indented less
//! than its first (a back-reference, kept in the state).
//!
//! The contexts keep syntect's ids (the order the probe's `dump YAML` lists them in). `\s`,
//! `\S`, `\d` and `\w` are ASCII here, and a byte past ASCII counts as a character of a plain
//! scalar or a tag; on ASCII text the scopes are syntect's.

use crate::machine::{
    Ctx, Grammar, It::I, It::P, Kinds, Line, M, Pat, atoms, empty, first, is_blank, is_space, lit1,
    non_space, protos, slots,
};

const BLOCK_PAIR_0: u8 = 0;
const BLOCK_PAIR_1: u8 = 1;
const BLOCK_PAIR_2: u8 = 2;
const BLOCK_SCALAR_0: u8 = 3;
const BLOCK_SCALAR_1: u8 = 4;
const COMMENT_0: u8 = 5;
const COMMENT_1: u8 = 6;
const DIRECTIVE_0: u8 = 7;
const FLOW_MAPPING_0: u8 = 8;
const FLOW_PAIR_0: u8 = 9;
const FLOW_PAIR_1: u8 = 10;
const FLOW_PAIR_2: u8 = 11;
const DOUBLE_QUOTED_0: u8 = 12;
const PLAIN_IN_0: u8 = 13;
const PLAIN_OUT_0: u8 = 14;
const SINGLE_QUOTED_0: u8 = 15;
const FLOW_SEQUENCE_0: u8 = 16;
const PROPERTY_0: u8 = 17;
const MAIN_0: u8 = 18;
const START: u8 = 19;
const BLOCK_COLLECTION: u8 = 20;
const BLOCK_MAPPING: u8 = 21;
const BLOCK_NODE: u8 = 22;
const BLOCK_PAIR: u8 = 23;
const BLOCK_SCALAR: u8 = 24;
const BLOCK_SEQUENCE: u8 = 25;
const COMMENT: u8 = 26;
const DIRECTIVE: u8 = 27;
const DIRECTIVE_FINISH: u8 = 28;
const FLOW_ALIAS: u8 = 29;
const FLOW_COLLECTION: u8 = 30;
const FLOW_MAPPING: u8 = 31;
const FLOW_NODE: u8 = 32;
const FLOW_PAIR: u8 = 33;
const FLOW_PAIR_VALUE: u8 = 34;
const FLOW_SCALAR: u8 = 35;
const DOUBLE_QUOTED: u8 = 36;
const PLAIN_IN: u8 = 37;
const PLAIN_IN_IMPLICIT: u8 = 38;
const PLAIN_OUT: u8 = 39;
const PLAIN_OUT_IMPLICIT: u8 = 40;
const SINGLE_QUOTED: u8 = 41;
const FLOW_SEQUENCE: u8 = 42;
const MAIN: u8 = 43;
const NODE: u8 = 44;
const PROPERTY: u8 = 45;
const PROTOTYPE: u8 = 46;

atoms! {
    BLOCK_MAPPING_META = "meta.block-mapping.yaml",
    KEY_VALUE_SEP = "punctuation.separator.key-value.mapping.yaml",
    EXPECTED_NEWLINE = "invalid.illegal.expected-newline.yaml",
    PLAIN_OUT_S = "string.unquoted.plain.out.yaml",
    TAG_NAME = "entity.name.tag.yaml",
    EXPECTED_COMMENT = "invalid.illegal.expected-comment-or-newline.yaml",
    BLOCK_TEXT = "string.unquoted.block.yaml",
    COMMENT_MARK = "punctuation.definition.comment.line.number-sign.yaml",
    COMMENT_LINE = "comment.line.number-sign.yaml",
    DIRECTIVE_META = "meta.directive.yaml",
    DIRECTIVE_YAML = "keyword.other.directive.yaml.yaml",
    YAML_VERSION = "constant.numeric.yaml-version.yaml",
    DIRECTIVE_TAG = "keyword.other.directive.tag.yaml",
    TAG_HANDLE = "storage.type.tag-handle.yaml",
    TAG_PREFIX = "support.type.tag-prefix.yaml",
    DIRECTIVE_RESERVED = "support.other.directive.reserved.yaml",
    DIRECTIVE_NAME = "string.unquoted.directive-name.yaml",
    DIRECTIVE_PARAMETER = "string.unquoted.directive-parameter.yaml",
    FLOW_MAPPING_META = "meta.flow-mapping.yaml",
    MAPPING_END = "punctuation.definition.mapping.end.yaml",
    MAPPING_SEP = "punctuation.separator.mapping.yaml",
    PAIR_EXPLICIT = "meta.flow-pair.explicit.yaml",
    PAIR_KEY = "meta.flow-pair.key.yaml",
    PLAIN_IN_S = "string.unquoted.plain.in.yaml",
    DOUBLE = "string.quoted.double.yaml",
    DOUBLE_ESCAPE = "constant.character.escape.double-quoted.yaml",
    DOUBLE_ESCAPE_NEWLINE = "constant.character.escape.double-quoted.newline.yaml",
    STR_END = "punctuation.definition.string.end.yaml",
    SINGLE = "string.quoted.single.yaml",
    SINGLE_ESCAPE = "constant.character.escape.single-quoted.yaml",
    FLOW_SEQUENCE_META = "meta.flow-sequence.yaml",
    SEQUENCE_END = "punctuation.definition.sequence.end.yaml",
    SEQUENCE_SEP = "punctuation.separator.sequence.yaml",
    PROPERTY_META = "meta.property.yaml",
    ANCHOR_KW = "keyword.control.property.anchor.yaml",
    ANCHOR_MARK = "punctuation.definition.anchor.yaml",
    ANCHOR_NAME = "entity.name.other.anchor.yaml",
    BAD_ANCHOR = "invalid.illegal.character.anchor.yaml",
    BAD_TAG_HANDLE = "invalid.illegal.tag-handle.yaml",
    KEY_BEGIN = "punctuation.definition.key-value.begin.yaml",
    SCALAR_LITERAL = "keyword.control.flow.block-scalar.literal.yaml",
    SCALAR_FOLDED = "keyword.control.flow.block-scalar.folded.yaml",
    INDENTATION = "constant.numeric.integer.decimal.indentation-indicator.yaml",
    CHOMPING = "storage.modifier.chomping-indicator.yaml",
    SEQUENCE_ITEM = "punctuation.definition.block.sequence.item.yaml",
    DIRECTIVE_BEGIN = "punctuation.definition.directive.begin.yaml",
    UNRECOGNIZED = "invalid.illegal.unrecognized.yaml",
    ALIAS_KW = "keyword.control.flow.alias.yaml",
    ALIAS_MARK = "punctuation.definition.alias.yaml",
    ALIAS_NAME = "variable.other.alias.yaml",
    MAPPING_BEGIN = "punctuation.definition.mapping.begin.yaml",
    FLOW_PAIR_META = "meta.flow-pair.yaml",
    FLOW_PAIR_VALUE_META = "meta.flow-pair.value.yaml",
    STR_BEGIN = "punctuation.definition.string.begin.yaml",
    NULL = "constant.language.null.yaml",
    BOOLEAN = "constant.language.boolean.yaml",
    INT_BINARY = "constant.numeric.integer.binary.yaml",
    NUMERIC_BASE = "punctuation.definition.numeric.base.yaml",
    INT_OCTAL = "constant.numeric.integer.octal.yaml",
    INT_DECIMAL = "constant.numeric.integer.decimal.yaml",
    INT_HEX = "constant.numeric.integer.hexadecimal.yaml",
    INT_OTHER = "constant.numeric.integer.other.yaml",
    FLOAT_DECIMAL = "constant.numeric.float.decimal.yaml",
    DECIMAL_SEP = "punctuation.separator.decimal.yaml",
    FLOAT_OTHER = "constant.numeric.float.other.yaml",
    TIMESTAMP = "constant.other.timestamp.yaml",
    DATE_SEP = "punctuation.separator.date.yaml",
    TIME_SEP = "punctuation.separator.time.yaml",
    VALUE = "constant.language.value.yaml",
    MERGE = "constant.language.merge.yaml",
    SEQUENCE_BEGIN = "punctuation.definition.sequence.begin.yaml",
    DOCUMENT_BEGIN = "entity.other.document.begin.yaml",
    DOCUMENT_END = "entity.other.document.end.yaml",
}

const fn ctx(items: &'static [crate::machine::It]) -> Ctx {
    Ctx::new(items)
}

/// The implicit types' groups and their scopes, as both variants of the regex number them.
const IMPLICIT: &[(u8, &[crate::machine::Atom])] = &[
    (1, &[s::NULL]),
    (2, &[s::BOOLEAN]),
    (4, &[s::INT_BINARY]),
    (5, &[s::NUMERIC_BASE]),
    (6, &[s::INT_OCTAL]),
    (7, &[s::NUMERIC_BASE]),
    (8, &[s::INT_DECIMAL]),
    (9, &[s::INT_HEX]),
    (10, &[s::NUMERIC_BASE]),
    (11, &[s::INT_OTHER]),
    (13, &[s::FLOAT_DECIMAL]),
    (14, &[s::DECIMAL_SEP]),
    (15, &[s::FLOAT_OTHER]),
    (16, &[s::DECIMAL_SEP]),
    (17, &[s::FLOAT_OTHER]),
    (18, &[s::DECIMAL_SEP]),
    (19, &[s::FLOAT_OTHER]),
    (20, &[s::DECIMAL_SEP]),
    (21, &[s::TIMESTAMP]),
    (22, &[s::DATE_SEP]),
    (23, &[s::DATE_SEP]),
    (24, &[s::DATE_SEP]),
    (25, &[s::DATE_SEP]),
    (26, &[s::TIME_SEP]),
    (27, &[s::TIME_SEP]),
    (28, &[s::TIME_SEP]),
    (29, &[s::TIME_SEP]),
    (30, &[s::VALUE]),
    (31, &[s::MERGE]),
];

static CTXS: [Ctx; 47] = [
    // #anon_block-pair_0
    ctx(&[
        P(Pat::new(question_ahead).pop()),
        P(Pat::new(bol_colon).caps(&[(1, &[s::KEY_VALUE_SEP])]).pop()),
        P(Pat::new(colon).sc(&[s::EXPECTED_NEWLINE]).pop()),
        I(BLOCK_NODE),
    ])
    .meta(&[s::BLOCK_MAPPING_META]),
    // #anon_block-pair_1
    ctx(&[
        I(PLAIN_OUT_IMPLICIT),
        P(Pat::new(plain_out_end).pop()),
        P(Pat::new(plain_out_start).set(&[BLOCK_PAIR_2])),
    ]),
    // #anon_block-pair_2
    ctx(&[P(Pat::new(plain_out_end).pop())])
        .meta(&[s::PLAIN_OUT_S, s::TAG_NAME])
        .no_proto(),
    // #anon_block-scalar_0
    ctx(&[
        P(Pat::new(bol_indent).set(&[BLOCK_SCALAR_1])),
        P(Pat::new(bol_non_space).pop()),
        I(COMMENT),
        P(Pat::new(rest).sc(&[s::EXPECTED_COMMENT])),
    ])
    .no_proto(),
    // #anon_block-scalar_1
    ctx(&[P(Pat::new(block_end).pop().nocache())])
        .meta(&[s::BLOCK_TEXT])
        .no_proto()
        .backref(),
    // #anon_comment_0
    ctx(&[P(Pat::new(hash).sc(&[s::COMMENT_MARK]).set(&[COMMENT_1]))]),
    // #anon_comment_1
    ctx(&[P(Pat::new(newline_or_end).pop())]).meta(&[s::COMMENT_LINE]),
    // #anon_directive_0
    ctx(&[
        P(Pat::new(directive_yaml)
            .caps(&[(1, &[s::DIRECTIVE_YAML]), (2, &[s::YAML_VERSION])])
            .set(&[DIRECTIVE_FINISH])),
        P(Pat::new(directive_tag)
            .caps(&[
                (1, &[s::DIRECTIVE_TAG]),
                (2, &[s::TAG_HANDLE]),
                (3, &[s::TAG_PREFIX]),
            ])
            .set(&[DIRECTIVE_FINISH])),
        P(Pat::new(directive_reserved)
            .caps(&[
                (1, &[s::DIRECTIVE_RESERVED]),
                (2, &[s::DIRECTIVE_NAME]),
                (3, &[s::DIRECTIVE_PARAMETER]),
            ])
            .set(&[DIRECTIVE_FINISH])),
        P(Pat::new(empty).set(&[DIRECTIVE_FINISH])),
    ])
    .meta(&[s::DIRECTIVE_META]),
    // #anon_flow-mapping_0
    ctx(&[
        P(Pat::new(close_brace).sc(&[s::MAPPING_END]).pop()),
        P(Pat::new(comma).sc(&[s::MAPPING_SEP])),
        I(FLOW_PAIR),
    ])
    .meta(&[s::FLOW_MAPPING_META]),
    // #anon_flow-pair_0
    ctx(&[
        P(Pat::new(flow_close_ahead).pop()),
        I(FLOW_PAIR),
        I(FLOW_NODE),
        P(Pat::new(flow_colon)
            .sc(&[s::KEY_VALUE_SEP])
            .set(&[FLOW_PAIR_VALUE])),
    ])
    .meta(&[s::PAIR_EXPLICIT]),
    // #anon_flow-pair_1
    ctx(&[
        I(PLAIN_IN_IMPLICIT),
        P(Pat::new(plain_in_end).pop()),
        P(Pat::new(plain_in_start).set(&[FLOW_PAIR_2])),
    ])
    .meta(&[s::PAIR_KEY]),
    // #anon_flow-pair_2
    ctx(&[P(Pat::new(plain_in_end).pop())])
        .meta(&[s::PLAIN_IN_S, s::TAG_NAME])
        .no_proto(),
    // #anon_flow-scalar-double-quoted_0
    ctx(&[
        P(Pat::new(double_escape).sc(&[s::DOUBLE_ESCAPE])),
        P(Pat::new(escaped_newline).sc(&[s::DOUBLE_ESCAPE_NEWLINE])),
        P(Pat::new(dquote).sc(&[s::STR_END]).pop()),
    ])
    .meta(&[s::DOUBLE])
    .no_proto(),
    // #anon_flow-scalar-plain-in_0
    ctx(&[P(Pat::new(plain_in_end).pop())])
        .meta(&[s::PLAIN_IN_S])
        .no_proto(),
    // #anon_flow-scalar-plain-out_0
    ctx(&[P(Pat::new(plain_out_end).pop())])
        .meta(&[s::PLAIN_OUT_S])
        .no_proto(),
    // #anon_flow-scalar-single-quoted_0
    ctx(&[
        P(Pat::new(squote2).sc(&[s::SINGLE_ESCAPE])),
        P(Pat::new(squote).sc(&[s::STR_END]).pop()),
    ])
    .meta(&[s::SINGLE])
    .no_proto(),
    // #anon_flow-sequence_0
    ctx(&[
        P(Pat::new(close_bracket).sc(&[s::SEQUENCE_END]).pop()),
        P(Pat::new(comma).sc(&[s::SEQUENCE_SEP])),
        I(FLOW_PAIR),
        I(FLOW_NODE),
    ])
    .meta(&[s::FLOW_SEQUENCE_META]),
    // #anon_property_0
    ctx(&[
        P(Pat::new(anchor)
            .caps(&[
                (1, &[s::ANCHOR_KW, s::ANCHOR_MARK]),
                (2, &[s::ANCHOR_NAME]),
                (3, &[s::BAD_ANCHOR]),
            ])
            .pop()),
        P(Pat::new(tag_handle).sc(&[s::TAG_HANDLE]).pop()),
        P(Pat::new(non_spaces).sc(&[s::BAD_TAG_HANDLE]).pop()),
    ])
    .meta(&[s::PROPERTY_META]),
    // __main
    ctx(&[I(MAIN)]),
    // __start: its `source.yaml` is the base scope
    ctx(&[P(Pat::new(empty).push(&[MAIN_0]))]).no_proto(),
    // block-collection
    ctx(&[I(BLOCK_SEQUENCE), I(BLOCK_MAPPING)]),
    // block-mapping
    ctx(&[I(BLOCK_PAIR)]),
    // block-node
    ctx(&[
        I(BLOCK_SCALAR),
        I(BLOCK_COLLECTION),
        I(PLAIN_OUT),
        I(FLOW_NODE),
    ]),
    // block-pair
    ctx(&[
        P(Pat::new(question).sc(&[s::KEY_BEGIN]).push(&[BLOCK_PAIR_0])),
        P(Pat::new(block_key_ahead).push(&[BLOCK_PAIR_1])),
        P(Pat::new(block_colon).sc(&[s::KEY_VALUE_SEP])),
    ]),
    // block-scalar
    ctx(&[P(Pat::new(block_scalar)
        .caps(&[
            (1, &[s::SCALAR_LITERAL]),
            (2, &[s::SCALAR_FOLDED]),
            (3, &[s::INDENTATION]),
            (4, &[s::CHOMPING]),
        ])
        .push(&[BLOCK_SCALAR_0]))]),
    // block-sequence
    ctx(&[P(Pat::new(dash).sc(&[s::SEQUENCE_ITEM]))]),
    // comment
    ctx(&[P(Pat::new(comment_ahead).push(&[COMMENT_0]))]),
    // directive
    ctx(&[P(Pat::new(bol_percent)
        .sc(&[s::DIRECTIVE_BEGIN])
        .push(&[DIRECTIVE_0]))]),
    // directive-finish
    ctx(&[
        P(Pat::new(directive_end_ahead).pop()),
        P(Pat::new(non_spaces).sc(&[s::UNRECOGNIZED])),
    ]),
    // flow-alias
    ctx(&[P(Pat::new(alias).caps(&[
        (1, &[s::ALIAS_KW, s::ALIAS_MARK]),
        (2, &[s::ALIAS_NAME]),
        (3, &[s::BAD_ANCHOR]),
    ]))]),
    // flow-collection
    ctx(&[I(FLOW_SEQUENCE), I(FLOW_MAPPING)]),
    // flow-mapping
    ctx(&[P(Pat::new(open_brace)
        .sc(&[s::MAPPING_BEGIN])
        .push(&[FLOW_MAPPING_0]))]),
    // flow-node
    ctx(&[I(FLOW_ALIAS), I(FLOW_COLLECTION), I(FLOW_SCALAR)]),
    // flow-pair
    ctx(&[
        P(Pat::new(question).sc(&[s::KEY_BEGIN]).push(&[FLOW_PAIR_0])),
        P(Pat::new(flow_key_ahead).push(&[FLOW_PAIR_1])),
        I(FLOW_NODE),
        P(Pat::new(flow_colon)
            .sc(&[s::FLOW_PAIR_META, s::KEY_VALUE_SEP])
            .push(&[FLOW_PAIR_VALUE])),
    ]),
    // flow-pair-value
    ctx(&[I(FLOW_NODE), P(Pat::new(flow_close_ahead).pop())]).content(&[s::FLOW_PAIR_VALUE_META]),
    // flow-scalar
    ctx(&[I(DOUBLE_QUOTED), I(SINGLE_QUOTED), I(PLAIN_IN)]),
    // flow-scalar-double-quoted
    ctx(&[P(Pat::new(dquote)
        .sc(&[s::STR_BEGIN])
        .push(&[DOUBLE_QUOTED_0]))]),
    // flow-scalar-plain-in
    ctx(&[
        I(PLAIN_IN_IMPLICIT),
        P(Pat::new(plain_in_start).push(&[PLAIN_IN_0])),
    ]),
    // flow-scalar-plain-in-implicit-type
    ctx(&[P(Pat::new(implicit_in).caps(IMPLICIT))]),
    // flow-scalar-plain-out
    ctx(&[
        I(PLAIN_OUT_IMPLICIT),
        P(Pat::new(plain_out_start).push(&[PLAIN_OUT_0])),
    ]),
    // flow-scalar-plain-out-implicit-type
    ctx(&[P(Pat::new(implicit_out).caps(IMPLICIT))]),
    // flow-scalar-single-quoted
    ctx(&[P(Pat::new(squote)
        .sc(&[s::STR_BEGIN])
        .push(&[SINGLE_QUOTED_0]))]),
    // flow-sequence
    ctx(&[P(Pat::new(open_bracket)
        .sc(&[s::SEQUENCE_BEGIN])
        .push(&[FLOW_SEQUENCE_0]))]),
    // main: only ever included, so its `source.yaml` never applies
    ctx(&[
        I(DIRECTIVE),
        P(Pat::new(document_begin).sc(&[s::DOCUMENT_BEGIN])),
        P(Pat::new(document_end).sc(&[s::DOCUMENT_END])),
        I(NODE),
    ]),
    // node
    ctx(&[I(BLOCK_NODE)]),
    // property
    ctx(&[P(Pat::new(property_ahead).push(&[PROPERTY_0]))]),
    // prototype
    ctx(&[I(COMMENT), I(PROPERTY)]).no_proto(),
];

static SLOTS: [u16; 47] = slots(&CTXS);
static PROTOS: [bool; 47] = protos(&CTXS, Some(PROTOTYPE));
static KINDS: Kinds = Kinds::new();

pub static GRAMMAR: Grammar = Grammar {
    ctxs: &CTXS,
    start: &[START],
    atoms: ATOMS,
    slots: &SLOTS,
    proto: Some(PROTOTYPE),
    protos: &PROTOS,
    kinds: &KINDS,
};

// the matchers: each finds its regex's leftmost match at or after `from`

/// `[-?:,\[\]{}#&*!|>'"%@`]`: what a plain scalar can't start with (or only before a
/// character, for `?`, `:` and `-`).
fn is_indicator(c: u8) -> bool {
    b"-?:,[]{}#&*!|>'\"%@`".contains(&c)
}

/// `[\[\]{},]`
fn is_flow(c: u8) -> bool {
    b"[]{},".contains(&c)
}

/// Where the spaces from `p` end, the `\n` among them.
fn ws(l: &Line, p: usize) -> usize {
    l.run(p, is_space)
}

/// The length of the character at `p` past a plain scalar's first test (`\S`, or a class
/// that takes any byte past ASCII).
fn char_at(l: &Line, p: usize) -> usize {
    l.char_len(p).unwrap_or(1)
}

/// A plain scalar's first characters at `p`: `[^\s<indicators>]`, or `[?:-]` and then one
/// of `next`; its end.
fn plain_start_at(l: &Line, p: usize, next: impl Fn(u8) -> bool) -> Option<usize> {
    let c = l.at(p)?;
    if p < l.nl() && !is_space(c) && !is_indicator(c) {
        return Some(p + char_at(l, p));
    }
    if matches!(c, b'?' | b':' | b'-') && l.is(p + 1, &next) && p + 1 < l.nl() {
        return Some(p + 1 + char_at(l, p + 1));
    }
    None
}

/// `(?:[^\s…]|[?:-]\S)`, a plain scalar's start out of flow collections.
fn plain_out_start(l: &Line, from: usize) -> Option<M> {
    (from..l.nl()).find_map(|p| plain_start_at(l, p, |c| !is_space(c)).map(|e| M::new(p, e)))
}

/// `(?:[^\s…]|[?:-][^\s[\[\]{},]])`, a plain scalar's start in them.
fn plain_in_start(l: &Line, from: usize) -> Option<M> {
    (from..l.nl())
        .find_map(|p| plain_start_at(l, p, |c| !is_space(c) && !is_flow(c)).map(|e| M::new(p, e)))
}

/// Where a plain scalar ends, out of flow collections: `\s*$`, `\s+\#`, or `\s*:(\s|$)`
/// after `p`.
pub(crate) fn out_end_at(l: &Line, p: usize) -> bool {
    let q = ws(l, p);
    q == l.end()
        || (q > p && l.at(q) == Some(b'#'))
        || (l.at(q) == Some(b':') && (l.is(q + 1, is_space) || l.eol(q + 1)))
}

/// And in them: also `\s*:[\[\]{},]` and `\s*[\[\]{},]`.
pub(crate) fn in_end_at(l: &Line, p: usize) -> bool {
    out_end_at(l, p) || {
        let q = ws(l, p);
        l.is(q, is_flow) || (l.at(q) == Some(b':') && l.is(q + 1, is_flow))
    }
}

/// The leftmost `p` where `end` holds; the positions in a run of spaces share its answer.
fn end_ahead(l: &Line, from: usize, end: impl Fn(&Line, usize) -> bool) -> Option<M> {
    let mut p = from;
    while p <= l.end() {
        if end(l, p) {
            return Some(M::new(p, p));
        }
        p = match l.is(p, is_space) {
            true => ws(l, p).max(p + 1),
            false => p + 1,
        };
    }
    None
}

fn plain_out_end(l: &Line, from: usize) -> Option<M> {
    end_ahead(l, from, out_end_at)
}

fn plain_in_end(l: &Line, from: usize) -> Option<M> {
    end_ahead(l, from, in_end_at)
}

/// The block-pair look-ahead, a mapping key and its `:` ahead:
/// `(?=FIRST(?:[^\s:]|:\S|\s+(?![#\s]))*\s*:(\s|$))`, and in flow collections
/// `(?=FIRST(?:[^\s:<flow>]|:[^\s<flow>]|\s+(?![#\s]))*\s*:(\s|$))`. Each step of the
/// repetition is decided by the byte it starts at, so its path from a start is one; a path
/// that fails at a position fails from any start that reaches it, and `dead` keeps those.
fn key_ahead(l: &Line, from: usize, flow: bool) -> Option<M> {
    let len = l.end() + 1;
    let mut dead = vec![false; len];
    let next = |c: u8| !is_space(c) && !(flow && is_flow(c));
    for p in from..l.nl() {
        let Some(mut r) = plain_start_at(l, p, next) else {
            continue;
        };
        let mut path = Vec::new();
        let found = loop {
            if r >= len || dead[r] {
                break false;
            }
            path.push(r);
            let q = ws(l, r);
            if l.at(q) == Some(b':') && (l.is(q + 1, is_space) || l.eol(q + 1)) {
                break true;
            }
            let Some(c) = l.at(r) else {
                break false;
            };
            r = if is_space(c) {
                if l.at(q).is_some_and(|c| c == b'#' || is_space(c)) {
                    break false;
                }
                q
            } else if c == b':' {
                if r + 1 >= l.nl() || !next(l.b[r + 1]) {
                    break false;
                }
                r + 1 + char_at(l, r + 1)
            } else if flow && is_flow(c) {
                break false;
            } else {
                r + char_at(l, r)
            };
        };
        if found {
            return Some(M::new(p, p));
        }
        for r in path {
            dead[r] = true;
        }
    }
    None
}

fn block_key_ahead(l: &Line, from: usize) -> Option<M> {
    key_ahead(l, from, false)
}

fn flow_key_ahead(l: &Line, from: usize) -> Option<M> {
    key_ahead(l, from, true)
}

/// `(?=\?)`
fn question_ahead(l: &Line, from: usize) -> Option<M> {
    first(l, from, b'?').map(|k| M::new(k, k))
}

/// `\?`
fn question(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'?')
}

/// `^ *(:)`
fn bol_colon(l: &Line, from: usize) -> Option<M> {
    if from > 0 {
        return None;
    }
    let q = l.run(0, |c| c == b' ');
    (l.at(q) == Some(b':')).then(|| M::new(0, q + 1).g(1, q, q + 1))
}

/// `:`
fn colon(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b':')
}

/// `:(?=\s|$)`
fn block_colon(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, b':') {
        if l.is(k + 1, is_space) || l.eol(k + 1) {
            return Some(M::new(k, k + 1));
        }
        at = k + 1;
    }
    None
}

/// `:(?=\s|$|[\[\]{},])`
fn flow_colon(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, b':') {
        if l.is(k + 1, |c| is_space(c) || is_flow(c)) || l.eol(k + 1) {
            return Some(M::new(k, k + 1));
        }
        at = k + 1;
    }
    None
}

/// `(?=[},\]])`
fn flow_close_ahead(l: &Line, from: usize) -> Option<M> {
    let hay = l.b.get(from..)?;
    let i = from + hay.iter().position(|c| b"},]".contains(c))?;
    Some(M::new(i, i))
}

/// `^([ ]+)(?! )`
fn bol_indent(l: &Line, from: usize) -> Option<M> {
    let q = l.run(0, |c| c == b' ');
    (from == 0 && q > 0).then(|| M::new(0, q).g(1, 0, q))
}

/// `^(?=\S)`
fn bol_non_space(l: &Line, from: usize) -> Option<M> {
    (from == 0 && l.b.first().is_some_and(|&c| !is_space(c))).then(|| M::new(0, 0))
}

/// `.+`
fn rest(l: &Line, from: usize) -> Option<M> {
    (from < l.nl()).then(|| M::new(from, l.nl()))
}

/// `^(?!\1|\s*$)`, `\1` the block's indentation: a line indented less, not blank.
fn block_end(l: &Line, from: usize) -> Option<M> {
    if from > 0 {
        return None;
    }
    let indent = usize::from(l.arg.get());
    let indented = l.b.len() >= indent && l.b[..indent].iter().all(|&c| c == b' ');
    let blank = ws(l, 0) == l.end();
    (!indented && !blank).then(|| M::new(0, 0))
}

/// `#`
fn hash(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'#')
}

/// `\n|\z`
fn newline_or_end(l: &Line, from: usize) -> Option<M> {
    match from {
        _ if from <= l.nl() => Some(M::new(l.nl(), l.end())),
        _ if from == l.end() => Some(M::new(from, from)),
        _ => None,
    }
}

/// `(?:^[ \t]*|[ \t]+)(?=\#)`
fn comment_ahead(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, b'#') {
        let mut p = k;
        while p > from && is_blank(l.b[p - 1]) {
            p -= 1;
        }
        if p < k || p == 0 {
            return Some(M::new(p, k));
        }
        at = k + 1;
    }
    None
}

/// `(YAML)[ \t]+(\d+\.\d+)`
pub(crate) fn directive_yaml(l: &Line, from: usize) -> Option<M> {
    let digits = |p: usize| l.run(p, |c| c.is_ascii_digit());
    let mut at = from;
    while let Some(m) = l.lit(at, b"YAML") {
        let k = m.s as usize;
        let v = l.run(k + 4, is_blank);
        let d = digits(v);
        if v > k + 4 && d > v && l.at(d) == Some(b'.') && digits(d + 1) > d + 1 {
            return Some(
                M::new(k, digits(d + 1))
                    .g(1, k, k + 4)
                    .g(2, v, digits(d + 1)),
            );
        }
        at = k + 1;
    }
    None
}

/// `%[0-9A-Fa-f]{2}` at `p`.
fn percent_hex(l: &Line, p: usize) -> bool {
    l.at(p) == Some(b'%')
        && l.is(p + 1, |c| c.is_ascii_hexdigit())
        && l.is(p + 2, |c| c.is_ascii_hexdigit())
}

/// The end of a run of `%HH` or bytes in `set` from `p`.
fn uri_run(l: &Line, mut p: usize, set: &[u8]) -> usize {
    loop {
        if percent_hex(l, p) {
            p += 3;
        } else if l.is(p, |c| c.is_ascii_alphanumeric() || set.contains(&c)) {
            p += 1;
        } else {
            return p;
        }
    }
}

/// The URI characters of a tag prefix, beside letters and digits.
const URI: &[u8] = b"-#;/?:@&=+$,_.!~*'()[]";

/// A tag handle at `p`, `!(?:[0-9A-Za-z\-]*!)?`, its longest first: its ends.
fn handles(l: &Line, p: usize) -> Option<(usize, Option<usize>)> {
    if l.at(p) != Some(b'!') {
        return None;
    }
    let r = l.run(p + 1, |c| c.is_ascii_alphanumeric() || c == b'-');
    let long = (l.at(r) == Some(b'!')).then_some(r + 1);
    Some((p + 1, long))
}

/// `(TAG)(?:[ \t]+(!(?:[0-9A-Za-z\-]*!)?)(?:[ \t]+(URI))?)?`
pub(crate) fn directive_tag(l: &Line, from: usize) -> Option<M> {
    let k = l.lit(from, b"TAG")?.s as usize;
    let mut m = M::new(k, k + 3).g(1, k, k + 3);
    let h = l.run(k + 3, is_blank);
    if h == k + 3 {
        return Some(m);
    }
    let Some((short, long)) = handles(l, h) else {
        return Some(m);
    };
    let he = long.unwrap_or(short);
    m = m.g(2, h, he);
    m.e = he as u32;
    let u = l.run(he, is_blank);
    if u > he {
        // `!` and any URI characters, or a URI that doesn't start with `,![]{}`
        let e = if l.at(u) == Some(b'!') {
            Some(uri_run(l, u + 1, URI))
        } else if !l.is(u, |c| b",![]{}".contains(&c)) {
            Some(uri_run(l, u, URI)).filter(|&e| e > u)
        } else {
            None
        };
        if let Some(e) = e {
            m = m.g(3, u, e);
            m.e = e as u32;
        }
    }
    Some(m)
}

/// `(\w+)(?:[ \t]+(\w+)(?:[ \t]+(\w+))?)?`
pub(crate) fn directive_reserved(l: &Line, from: usize) -> Option<M> {
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80;
    let hay = l.b.get(from..)?;
    let k = from + hay.iter().position(|&c| word(c))?;
    let e1 = l.run(k, word);
    let mut m = M::new(k, e1).g(1, k, e1);
    let s2 = l.run(e1, is_blank);
    let e2 = l.run(s2, word);
    if s2 > e1 && e2 > s2 {
        m = m.g(2, s2, e2);
        m.e = e2 as u32;
        let s3 = l.run(e2, is_blank);
        let e3 = l.run(s3, word);
        if s3 > e2 && e3 > s3 {
            m = m.g(3, s3, e3);
            m.e = e3 as u32;
        }
    }
    Some(m)
}

/// `(?=$|[ \t]+($|#))`
fn directive_end_ahead(l: &Line, from: usize) -> Option<M> {
    l.find(from, |p| {
        let q = l.run(p, is_blank);
        let hit = l.eol(p) || (q > p && (l.eol(q) || l.at(q) == Some(b'#')));
        hit.then(|| M::new(p, p))
    })
}

/// `\S+`
fn non_spaces(l: &Line, from: usize) -> Option<M> {
    let m = non_space(l, from)?;
    Some(M::new(m.s as usize, l.run(m.s as usize, |c| !is_space(c))))
}

/// `\}`, `\{`, `\]`, `\[`, `,`, `"`, `'`
fn close_brace(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'}')
}

fn open_brace(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'{')
}

fn close_bracket(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b']')
}

fn open_bracket(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'[')
}

fn comma(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b',')
}

fn dquote(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'"')
}

fn squote(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'\'')
}

/// `''`
fn squote2(l: &Line, from: usize) -> Option<M> {
    l.lit(from, b"''")
}

/// `\\(?:[0abtnvfre "/\\N_Lp]|x\h{2}|u\h{4}|U\h{8})`
fn double_escape(l: &Line, from: usize) -> Option<M> {
    let hex = |at: usize, n: usize| (at..at + n).all(|i| l.is(i, |c| c.is_ascii_hexdigit()));
    let mut at = from;
    while let Some(k) = first(l, at, b'\\') {
        let n = match l.at(k + 1) {
            Some(c) if b"0abtnvfre \"/\\N_Lp".contains(&c) && k + 1 < l.nl() => 2,
            Some(b'x') if hex(k + 2, 2) => 4,
            Some(b'u') if hex(k + 2, 4) => 6,
            Some(b'U') if hex(k + 2, 8) => 10,
            _ => 0,
        };
        if n > 0 {
            return Some(M::new(k, k + n));
        }
        at = k + 1;
    }
    None
}

/// `\\\n`
fn escaped_newline(l: &Line, from: usize) -> Option<M> {
    let k = l.nl().checked_sub(1)?;
    (k >= from && l.b[k] == b'\\').then(|| M::new(k, l.end()))
}

/// `(&)([^\s\[\]/{},]+)(\S+)?`
fn anchor(l: &Line, from: usize) -> Option<M> {
    let name = |c: u8| !is_space(c) && !b"[]/{},".contains(&c);
    let mut at = from;
    while let Some(k) = first(l, at, b'&') {
        let e = l.run(k + 1, name);
        if e > k + 1 {
            let rest = l.run(e, |c| !is_space(c));
            let m = M::new(k, rest).g(1, k, k + 1).g(2, k + 1, e);
            return Some(if rest > e { m.g(3, e, rest) } else { m });
        }
        at = k + 1;
    }
    None
}

/// `(\*)([^\s\[\]/{},]+)([^\s\]},]\S*)?`
fn alias(l: &Line, from: usize) -> Option<M> {
    let name = |c: u8| !is_space(c) && !b"[]/{},".contains(&c);
    let mut at = from;
    while let Some(k) = first(l, at, b'*') {
        let e = l.run(k + 1, name);
        if e > k + 1 {
            let m = M::new(k, e).g(1, k, k + 1).g(2, k + 1, e);
            if l.is(e, |c| !is_space(c) && !b"]},".contains(&c)) {
                let rest = l.run(e, |c| !is_space(c));
                let mut m = m.g(3, e, rest);
                m.e = rest as u32;
                return Some(m);
            }
            return Some(m);
        }
        at = k + 1;
    }
    None
}

/// The tag characters after a handle, beside letters and digits.
const TAG: &[u8] = b"-#;/?:@&=+$_.~*'()";

/// `(?:!<URI+>|HANDLE(?:%HH|TAG)+|!)(?=\ |\t|$)`
pub(crate) fn tag_handle(l: &Line, from: usize) -> Option<M> {
    let ahead = |e: usize| l.is(e, is_blank) || l.eol(e);
    let mut at = from;
    while let Some(p) = first(l, at, b'!') {
        if l.at(p + 1) == Some(b'<') {
            let e = uri_run(l, p + 2, URI);
            if e > p + 2 && l.at(e) == Some(b'>') && ahead(e + 1) {
                return Some(M::new(p, e + 1));
            }
        }
        if let Some((short, long)) = handles(l, p) {
            for h in long.into_iter().chain([short]) {
                let e = uri_run(l, h, TAG);
                if e > h && ahead(e) {
                    return Some(M::new(p, e));
                }
            }
        }
        if ahead(p + 1) {
            return Some(M::new(p, p + 1));
        }
        at = p + 1;
    }
    None
}

/// `(?=!|&)`
fn property_ahead(l: &Line, from: usize) -> Option<M> {
    let hay = l.b.get(from..)?;
    let i = from + hay.iter().position(|&c| c == b'!' || c == b'&')?;
    Some(M::new(i, i))
}

/// `(?:(\|)|(>))([1-9])?([-+])?`
fn block_scalar(l: &Line, from: usize) -> Option<M> {
    let hay = l.b.get(from..)?;
    let k = from + hay.iter().position(|&c| c == b'|' || c == b'>')?;
    let mut m = M::new(k, k + 1).g(if l.b[k] == b'|' { 1 } else { 2 }, k, k + 1);
    let mut e = k + 1;
    if l.is(e, |c| (b'1'..=b'9').contains(&c)) {
        m = m.g(3, e, e + 1);
        e += 1;
    }
    if l.is(e, |c| c == b'-' || c == b'+') {
        m = m.g(4, e, e + 1);
        e += 1;
    }
    m.e = e as u32;
    Some(m)
}

/// `(-)(?!\S)`
fn dash(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, b'-') {
        if l.at(k + 1).is_none_or(is_space) {
            return Some(M::new(k, k + 1));
        }
        at = k + 1;
    }
    None
}

/// `^%`
fn bol_percent(l: &Line, from: usize) -> Option<M> {
    (from == 0 && l.b.first() == Some(&b'%')).then(|| M::new(0, 1))
}

/// `^---`
fn document_begin(l: &Line, from: usize) -> Option<M> {
    (from == 0 && l.b.starts_with(b"---")).then(|| M::new(0, 3))
}

/// `^\.{3}`
fn document_end(l: &Line, from: usize) -> Option<M> {
    (from == 0 && l.b.starts_with(b"...")).then(|| M::new(0, 3))
}

fn implicit_out(l: &Line, from: usize) -> Option<M> {
    l.find(from, |p| implicit_at(l, p, out_end_at))
}

fn implicit_in(l: &Line, from: usize) -> Option<M> {
    l.find(from, |p| implicit_at(l, p, in_end_at))
}

/// `\d` at `i`.
fn digit(l: &Line, i: usize) -> bool {
    l.is(i, |c| c.is_ascii_digit())
}

/// The end of a run of bytes in `set` from `p`, at least `min` long.
fn run_of(l: &Line, p: usize, min: usize, set: impl Fn(u8) -> bool) -> Option<usize> {
    let e = l.run(p, set);
    (e >= p + min).then_some(e)
}

/// The implicit types' regex at `p`, its alternatives in order, each end tried against the
/// look-ahead `end` (bat's two variants differ only there). Only the timestamp's trailing
/// blanks and zone can end a match early to meet it; elsewhere a shorter end is followed by
/// a digit, a letter, `_` or `.`, which the look-ahead never takes, so the greedy end is the
/// only one.
pub(crate) fn implicit_at(l: &Line, p: usize, end: fn(&Line, usize) -> bool) -> Option<M> {
    let ok = |e: usize| end(l, e);
    let words = |words: &[&[u8]]| {
        words
            .iter()
            .map(|w| p + w.len())
            .zip(words)
            .find(|&(e, w)| l.b.get(p..e) == Some(*w) && ok(e))
            .map(|(e, _)| e)
    };
    if let Some(e) = words(&[b"null", b"Null", b"NULL", b"~"]) {
        return Some(M::new(p, e).g(1, p, e));
    }
    let bools: [&[u8]; 22] = [
        b"y", b"Y", b"yes", b"Yes", b"YES", b"n", b"N", b"no", b"No", b"NO", b"true", b"True",
        b"TRUE", b"false", b"False", b"FALSE", b"on", b"On", b"ON", b"off", b"Off", b"OFF",
    ];
    if let Some(e) = words(&bools) {
        return Some(M::new(p, e).g(2, p, e));
    }
    let q = p + usize::from(l.is(p, |c| c == b'+' || c == b'-'));
    let int = |m: M| Some(m.g(3, p, m.e as usize));
    // `(0b)[0-1_]+`, `(0)[0-7_]+`
    if l.b.get(q..q + 2) == Some(b"0b")
        && let Some(e) = run_of(l, q + 2, 1, |c| matches!(c, b'0' | b'1' | b'_')).filter(|&e| ok(e))
    {
        return int(M::new(p, e).g(4, p, e).g(5, q, q + 2));
    }
    if l.at(q) == Some(b'0')
        && let Some(e) = run_of(l, q + 1, 1, |c| matches!(c, b'0'..=b'7' | b'_')).filter(|&e| ok(e))
    {
        return int(M::new(p, e).g(6, p, e).g(7, q, q + 1));
    }
    // `(?:0|[1-9][0-9_]*)`
    let decimal = match l.at(q) {
        Some(b'0') => Some(q + 1),
        Some(b'1'..=b'9') => Some(l.run(q + 1, |c| c.is_ascii_digit() || c == b'_')),
        _ => None,
    };
    if let Some(e) = decimal.filter(|&e| ok(e)) {
        return int(M::new(p, e).g(8, p, e));
    }
    if l.b.get(q..q + 2) == Some(b"0x")
        && let Some(e) =
            run_of(l, q + 2, 1, |c| c.is_ascii_hexdigit() || c == b'_').filter(|&e| ok(e))
    {
        return int(M::new(p, e).g(9, p, e).g(10, q, q + 2));
    }
    // `[1-9][0-9_]*(?::[0-5]?[0-9])+`
    let sexagesimal = |from: usize| {
        let mut e = from;
        while l.at(e) == Some(b':') {
            let n = match (l.is(e + 1, |c| (b'0'..=b'5').contains(&c)), digit(l, e + 2)) {
                (true, true) => 3,
                _ if digit(l, e + 1) => 2,
                _ => break,
            };
            e += n;
        }
        (e > from).then_some(e)
    };
    if l.is(q, |c| (b'1'..=b'9').contains(&c))
        && let Some(e) = sexagesimal(l.run(q + 1, |c| c.is_ascii_digit() || c == b'_'))
        && ok(e)
    {
        return int(M::new(p, e).g(11, p, e));
    }
    let float = |m: M| Some(m.g(12, p, m.e as usize));
    // `(?:[0-9][0-9_]*)?(\.)[0-9.]*(?:[eE][-+][0-9]+)?`
    let dot = match digit(l, q) {
        true => l.run(q, |c| c.is_ascii_digit() || c == b'_'),
        false => q,
    };
    if l.at(dot) == Some(b'.') {
        let mut e = l.run(dot + 1, |c| c.is_ascii_digit() || c == b'.');
        if l.is(e, |c| c == b'e' || c == b'E')
            && l.is(e + 1, |c| c == b'-' || c == b'+')
            && digit(l, e + 2)
        {
            e = l.run(e + 2, |c| c.is_ascii_digit());
        }
        if ok(e) {
            return float(M::new(p, e).g(13, p, e).g(14, dot, dot + 1));
        }
    }
    // `[0-9][0-9_]*(?::[0-5]?[0-9])+(\.)[0-9_]*`
    if digit(l, q)
        && let Some(d) = sexagesimal(l.run(q + 1, |c| c.is_ascii_digit() || c == b'_'))
        && l.at(d) == Some(b'.')
    {
        let e = l.run(d + 1, |c| c.is_ascii_digit() || c == b'_');
        if ok(e) {
            return float(M::new(p, e).g(15, p, e).g(16, d, d + 1));
        }
    }
    // `(\.)(?:inf|Inf|INF)`, then `(\.)(?:nan|NaN|NAN)` without a sign
    if l.at(q) == Some(b'.')
        && [&b"inf"[..], b"Inf", b"INF"].contains(&l.b.get(q + 1..q + 4).unwrap_or_default())
        && ok(q + 4)
    {
        return float(M::new(p, q + 4).g(17, p, q + 4).g(18, q, q + 1));
    }
    if l.at(p) == Some(b'.')
        && [&b"nan"[..], b"NaN", b"NAN"].contains(&l.b.get(p + 1..p + 4).unwrap_or_default())
        && ok(p + 4)
    {
        return float(M::new(p, p + 4).g(19, p, p + 4).g(20, p, p + 1));
    }
    if let Some(m) = timestamp(l, p, ok) {
        return Some(m);
    }
    if l.at(p) == Some(b'=') && ok(p + 1) {
        return Some(M::new(p, p + 1).g(30, p, p + 1));
    }
    if l.b.get(p..p + 2) == Some(b"<<") && ok(p + 2) {
        return Some(M::new(p, p + 2).g(31, p, p + 2));
    }
    None
}

/// The timestamp alternative at `p`: a date, or a date and a time with a fraction, blanks
/// and a zone, whose blanks and zone give back what the look-ahead `ok` needs.
fn timestamp(l: &Line, p: usize, ok: impl Fn(usize) -> bool) -> Option<M> {
    if !(p..p + 4).all(|i| digit(l, i)) || l.at(p + 4) != Some(b'-') {
        return None;
    }
    let stamp = |m: M| Some(m.g(21, p, m.e as usize));
    // `\d{4}(-)\d{2}(-)\d{2}`
    if digit(l, p + 5)
        && digit(l, p + 6)
        && l.at(p + 7) == Some(b'-')
        && digit(l, p + 8)
        && digit(l, p + 9)
        && ok(p + 10)
    {
        return stamp(M::new(p, p + 10).g(22, p + 4, p + 5).g(23, p + 7, p + 8));
    }
    // `\d{1,2}` at `at`, as many as there are: its end
    let one_two = |at: usize| digit(l, at).then(|| at + 1 + usize::from(digit(l, at + 1)));
    let month = one_two(p + 5)?;
    if l.at(month) != Some(b'-') {
        return None;
    }
    let day = one_two(month + 1)?;
    let sep = match l.at(day)? {
        b'T' | b't' => day + 1,
        c if is_blank(c) => l.run(day, is_blank),
        _ => return None,
    };
    let hour = one_two(sep)?;
    let two = |at: usize| digit(l, at) && digit(l, at + 1);
    if !(l.at(hour) == Some(b':') && two(hour + 1) && l.at(hour + 3) == Some(b':') && two(hour + 4))
    {
        return None;
    }
    let mut m = M::new(p, p)
        .g(24, p + 4, p + 5)
        .g(25, month, month + 1)
        .g(26, hour, hour + 1)
        .g(27, hour + 3, hour + 4);
    let mut e = hour + 6;
    if l.at(e) == Some(b'.') {
        m = m.g(28, e, e + 1);
        e = l.run(e + 1, |c| c.is_ascii_digit());
    }
    // the blanks, most first, and the zone's choices after them, as they backtrack
    let blanks = l.run(e, is_blank);
    for b in (e..=blanks).rev() {
        let mut zones: Vec<(usize, Option<usize>)> = Vec::new();
        match l.at(b) {
            Some(b'Z') => zones.push((b + 1, None)),
            Some(b'+' | b'-') => {
                for h in [2, 1] {
                    if (1..=h).all(|i| digit(l, b + i)) {
                        let c = b + 1 + h;
                        if l.at(c) == Some(b':') {
                            for mm in [2, 1] {
                                if (1..=mm).all(|i| digit(l, c + i)) {
                                    zones.push((c + 1 + mm, Some(c)));
                                }
                            }
                        }
                        zones.push((c, None));
                    }
                }
            }
            _ => {}
        }
        zones.push((b, None));
        for (end, colon) in zones {
            if ok(end) {
                let mut m = match colon {
                    Some(c) => m.g(29, c, c + 1),
                    None => m,
                };
                m.e = end as u32;
                return stamp(m);
            }
        }
    }
    None
}
