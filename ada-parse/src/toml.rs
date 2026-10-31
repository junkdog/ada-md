//! TOML, in place of bat's `TOML`: its 40 contexts on the [machine](crate::machine), context
//! for context, each regex a matcher below. A value's arrays, inline tables and `"""`/`'''`
//! strings run on across lines; everything else ends with its line.
//!
//! The contexts keep syntect's ids (the order `cargo xtask`'s probe `dump TOML` lists them in).

use crate::machine::{
    Ctx, Grammar, It::I, It::P, Kinds, Line, M, Pat, back_blanks, first, is_blank, is_space,
    is_word, lit1, protos, slots,
};

// the contexts
const ARRAY_SEP_0: u8 = 0;
const BASIC_STRING_0: u8 = 1;
const INLINE_KEYVALS_0: u8 = 2;
const INLINE_TABLE_0: u8 = 3;
const STRING_0: u8 = 4;
const STRING_1: u8 = 5;
const MAIN_0: u8 = 6;
const START: u8 = 7;
const ARRAY: u8 = 8;
const ARRAY_SEP: u8 = 9;
const BASIC_STRING: u8 = 10;
const BOOLEAN: u8 = 11;
const COMMENTS: u8 = 12;
const DATA_TYPES: u8 = 13;
const DATE_TIME: u8 = 14;
const FLOAT: u8 = 15;
const ILLEGAL: u8 = 16;
const INLINE_KEYVAL_SEP: u8 = 17;
const INLINE_TABLE: u8 = 18;
const INLINE_KEYVALS: u8 = 19;
const INTEGER: u8 = 20;
const KEY: u8 = 21;
const KEY_QUOTED: u8 = 22;
const KEY_SIMPLE: u8 = 23;
const KEYVAL: u8 = 24;
const KEYVAL_SEP: u8 = 25;
const LITERAL_STRING: u8 = 26;
const MAIN: u8 = 27;
const MAYBE_ARRAY_COMMENTS: u8 = 28;
const MAYBE_COMMENT_EOL: u8 = 29;
const MAYBE_DOT_KEY: u8 = 30;
const MAYBE_DOT_TABLE: u8 = 31;
const MAYBE_EMPTY_ARRAY: u8 = 32;
const STRING: u8 = 33;
const STRING_ESCAPE: u8 = 34;
const TABLE_ARRAY_NAME: u8 = 35;
const TABLE_NAME: u8 = 36;
const TABLE_NAME_INC: u8 = 37;
const TABLE_NAME_SIMPLE: u8 = 38;
const TABLES: u8 = 39;

/// The scopes, as indices into [`ATOMS`].
mod s {
    use crate::machine::Atom;
    pub const ARRAY_END: Atom = 0;
    pub const BASIC: Atom = 1;
    pub const STR_END: Atom = 2;
    pub const NON_TERMINATED: Atom = 3;
    pub const BAD_TABLE_END: Atom = 4;
    pub const INLINE_END: Atom = 5;
    pub const TRIPLE_LITERAL: Atom = 6;
    pub const TRIPLE_BASIC: Atom = 7;
    pub const ARRAY_BEGIN: Atom = 8;
    pub const ARRAY_SEP: Atom = 9;
    pub const BAD_ARRAY: Atom = 10;
    pub const STR_BEGIN: Atom = 11;
    pub const BOOL: Atom = 12;
    pub const COMMENT: Atom = 13;
    pub const COMMENT_MARK: Atom = 14;
    pub const BAD_VALUE: Atom = 15;
    pub const DATETIME: Atom = 16;
    pub const FLOAT: Atom = 17;
    pub const SIGN: Atom = 18;
    pub const ILLEGAL: Atom = 19;
    pub const KEY_VALUE: Atom = 20;
    pub const BAD_INLINE_SEP: Atom = 21;
    pub const INLINE_BEGIN: Atom = 22;
    pub const INLINE_SEP: Atom = 23;
    pub const BAD_INLINE_KEY: Atom = 24;
    pub const INTEGER: Atom = 25;
    pub const META_KEY: Atom = 26;
    pub const BAD_KEY: Atom = 27;
    pub const KEY: Atom = 28;
    pub const BAD_KEYVAL: Atom = 29;
    pub const LITERAL: Atom = 30;
    pub const BAD_TRAILING: Atom = 31;
    pub const KEY_DOT: Atom = 32;
    pub const TABLE_DOT: Atom = 33;
    pub const BAD_TABLE: Atom = 34;
    pub const ESCAPE: Atom = 35;
    pub const BAD_ESCAPE: Atom = 36;
    pub const META_TABLE_ARRAY: Atom = 37;
    pub const TABLE_ARRAY_END: Atom = 38;
    pub const META_TABLE: Atom = 39;
    pub const TABLE_END: Atom = 40;
    pub const TABLE_NAME: Atom = 41;
    pub const TABLE_ARRAY_BEGIN: Atom = 42;
    pub const TABLE_BEGIN: Atom = 43;
}

const ATOMS: &[&str] = &[
    "punctuation.definition.array.end.toml",
    "string.quoted.double.basic.toml",
    "punctuation.definition.string.end.toml",
    "invalid.illegal.string.non-terminated.toml",
    "invalid.illegal.table-end.toml",
    "punctuation.definition.inline-table.end.toml",
    "string.quoted.triple.literal.block.toml",
    "string.quoted.triple.basic.block.toml",
    "punctuation.definition.array.begin.toml",
    "punctuation.separator.array.toml",
    "invalid.illegal.array.toml",
    "punctuation.definition.string.begin.toml",
    "constant.language.toml",
    "comment.line.number-sign.toml",
    "punctuation.definition.comment.toml",
    "invalid.illegal.value.toml",
    "constant.other.datetime.toml",
    "constant.numeric.float.toml",
    "keyword.operator.arithmetic.toml",
    "invalid.illegal.toml",
    "punctuation.definition.key-value.toml",
    "invalid.illegal.inline-sep.toml",
    "punctuation.definition.inline-table.begin.toml",
    "punctuation.separator.inline-table.toml",
    "invalid.illegal.inline-key.toml",
    "constant.numeric.integer.toml",
    "meta.tag.key.toml",
    "invalid.illegal.key.toml",
    "entity.name.tag.toml",
    "invalid.illegal.keyval.toml",
    "string.quoted.single.literal.toml",
    "invalid.illegal.trailing.toml",
    "punctuation.separator.key.toml",
    "punctuation.separator.table.toml",
    "invalid.illegal.table.toml",
    "constant.character.escape.toml",
    "invalid.illegal.string.escape.toml",
    "meta.tag.table.array.toml",
    "punctuation.definition.table.array.end.toml",
    "meta.tag.table.toml",
    "punctuation.definition.table.end.toml",
    "entity.name.table.toml",
    "punctuation.definition.table.array.begin.toml",
    "punctuation.definition.table.begin.toml",
];

const fn ctx(items: &'static [crate::machine::It]) -> Ctx {
    Ctx::new(items)
}

static CTXS: [Ctx; 40] = [
    // #anon_array-sep_0
    ctx(&[
        P(Pat::new(ws_nl)),
        I(COMMENTS),
        P(Pat::new(close_bracket).sc(&[s::ARRAY_END]).pop()),
        P(Pat::new(not_nl).set(&[ARRAY_SEP, DATA_TYPES])),
    ]),
    // #anon_basic-string_0
    Ctx::new(&[
        I(STRING_ESCAPE),
        P(Pat::new(dquote).sc(&[s::STR_END]).pop()),
        P(Pat::new(newline).sc(&[s::NON_TERMINATED]).pop()),
    ])
    .meta(&[s::BASIC]),
    // #anon_inline-table-keyvals_0
    ctx(&[
        P(Pat::new(blanks)),
        P(Pat::new(close_brace).sc(&[s::BAD_TABLE_END]).pop()),
        P(Pat::new(empty).set(&[INLINE_KEYVAL_SEP, KEY])),
    ]),
    // #anon_inline-table_0
    ctx(&[
        P(Pat::new(blanks)),
        P(Pat::new(close_brace).sc(&[s::INLINE_END]).pop()),
        P(Pat::new(empty).set(&[INLINE_KEYVAL_SEP, KEY])),
    ]),
    // #anon_string_0
    Ctx::new(&[P(Pat::new(squote3).sc(&[s::STR_END]).pop())]).meta(&[s::TRIPLE_LITERAL]),
    // #anon_string_1
    Ctx::new(&[
        P(Pat::new(dquote3).sc(&[s::STR_END]).pop()),
        I(STRING_ESCAPE),
    ])
    .meta(&[s::TRIPLE_BASIC]),
    // __main
    ctx(&[I(MAIN)]),
    // __start: its `source.toml` is the base scope, left out of the kinds
    ctx(&[P(Pat::new(empty).push(&[MAIN_0]))]),
    // array
    ctx(&[P(Pat::new(open_bracket)
        .sc(&[s::ARRAY_BEGIN])
        .set(&[MAYBE_EMPTY_ARRAY, MAYBE_ARRAY_COMMENTS]))]),
    // array-sep
    ctx(&[
        P(Pat::new(ws_nl)),
        I(COMMENTS),
        P(Pat::new(close_bracket).sc(&[s::ARRAY_END]).pop()),
        P(Pat::new(comma).sc(&[s::ARRAY_SEP]).set(&[ARRAY_SEP_0])),
        P(Pat::new(word_or_char).sc(&[s::BAD_ARRAY])),
    ]),
    // basic-string
    ctx(&[P(Pat::new(dquote)
        .sc(&[s::STR_BEGIN])
        .set(&[BASIC_STRING_0]))]),
    // boolean
    ctx(&[P(Pat::new(boolean).caps(&[(1, &[s::BOOL])]).pop())]),
    // comments
    ctx(&[P(
        Pat::new(comment).caps(&[(1, &[s::COMMENT]), (2, &[s::COMMENT_MARK])])
    )]),
    // data-types
    ctx(&[
        I(INLINE_TABLE),
        I(ARRAY),
        I(STRING),
        I(DATE_TIME),
        I(FLOAT),
        I(INTEGER),
        I(BOOLEAN),
        P(Pat::new(blanks_eol).pop()),
        P(Pat::new(word_or_char).sc(&[s::BAD_VALUE]).pop()),
    ]),
    // date-time
    ctx(&[P(Pat::new(datetime).sc(&[s::DATETIME]).pop())]),
    // float
    ctx(&[
        P(Pat::new(float)
            .caps(&[(0, &[s::FLOAT]), (1, &[s::SIGN])])
            .pop()),
        P(Pat::new(inf_nan)
            .caps(&[(0, &[s::FLOAT]), (1, &[s::SIGN])])
            .pop()),
    ]),
    // illegal
    ctx(&[P(Pat::new(rest).caps(&[(1, &[s::ILLEGAL])]))]),
    // inline-keyval-sep
    ctx(&[
        P(Pat::new(equals)
            .caps(&[(1, &[s::KEY_VALUE])])
            .set(&[INLINE_KEYVALS, DATA_TYPES])),
        P(Pat::new(char_ahead).sc(&[s::BAD_INLINE_SEP]).pop()),
    ]),
    // inline-table
    ctx(&[P(Pat::new(open_brace)
        .sc(&[s::INLINE_BEGIN])
        .set(&[INLINE_TABLE_0]))]),
    // inline-table-keyvals
    ctx(&[
        P(Pat::new(blanks)),
        P(Pat::new(close_brace).sc(&[s::INLINE_END]).pop()),
        P(Pat::new(comma)
            .sc(&[s::INLINE_SEP])
            .set(&[INLINE_KEYVALS_0])),
        P(Pat::new(char_ahead).sc(&[s::BAD_INLINE_KEY]).pop()),
    ]),
    // integer
    ctx(&[P(Pat::new(integer)
        .caps(&[(0, &[s::INTEGER]), (1, &[s::SIGN])])
        .pop())]),
    // key
    Ctx::new(&[
        P(Pat::new(key_ahead).push(&[MAYBE_DOT_KEY, KEY_SIMPLE])),
        P(Pat::new(equals_ahead).pop()),
        P(Pat::new(blanks_eol).pop()),
        P(Pat::new(one_char).sc(&[s::BAD_KEY]).pop()),
    ])
    .meta(&[s::META_KEY]),
    // key-quoted
    ctx(&[I(BASIC_STRING), I(LITERAL_STRING)]),
    // key-simple
    ctx(&[
        P(Pat::new(bare_key).sc(&[s::KEY]).pop()),
        I(KEY_QUOTED),
        P(Pat::new(non_spaces).sc(&[s::BAD_KEY]).pop()),
    ]),
    // keyval
    ctx(&[P(Pat::new(key_ahead).push(&[KEYVAL_SEP, KEY]))]),
    // keyval-sep
    ctx(&[
        P(Pat::new(equals)
            .caps(&[(1, &[s::KEY_VALUE])])
            .set(&[MAYBE_COMMENT_EOL, DATA_TYPES])),
        P(Pat::new(blanks_eol).pop()),
        P(Pat::new(rest_to_eol).sc(&[s::BAD_KEYVAL]).pop()),
    ]),
    // literal-string
    ctx(&[P(Pat::new(literal)
        .sc(&[s::LITERAL])
        .caps(&[(1, &[s::STR_BEGIN]), (2, &[s::STR_END])])
        .pop())]),
    // main: only ever included, so its `source.toml` never applies
    ctx(&[
        P(Pat::new(bol_blanks)),
        I(COMMENTS),
        I(TABLES),
        I(KEYVAL),
        I(ILLEGAL),
    ]),
    // maybe-array-comments
    ctx(&[P(Pat::new(ws_nl)), I(COMMENTS), P(Pat::new(not_nl).pop())]),
    // maybe-comment-eol
    ctx(&[
        I(COMMENTS),
        P(Pat::new(eol).pop()),
        P(Pat::new(rest_nonempty).sc(&[s::BAD_TRAILING]).pop()),
    ]),
    // maybe-dot-key
    ctx(&[
        P(Pat::new(dot)
            .caps(&[(1, &[s::KEY_DOT])])
            .push(&[KEY_SIMPLE])),
        P(Pat::new(equals_or_eol_ahead).pop()),
        P(Pat::new(one_char).sc(&[s::BAD_KEY]).pop()),
    ]),
    // maybe-dot-table-name
    ctx(&[
        P(Pat::new(dot)
            .caps(&[(1, &[s::TABLE_DOT])])
            .push(&[TABLE_NAME_SIMPLE])),
        P(Pat::new(bracket_or_eol_ahead).pop()),
        P(Pat::new(one_char).sc(&[s::BAD_TABLE]).pop()),
    ]),
    // maybe-empty-array
    ctx(&[
        P(Pat::new(close_bracket).sc(&[s::ARRAY_END]).pop()),
        P(Pat::new(empty).set(&[ARRAY_SEP, DATA_TYPES])),
    ]),
    // string
    ctx(&[
        P(Pat::new(squote3).sc(&[s::STR_BEGIN]).set(&[STRING_0])),
        P(Pat::new(dquote3).sc(&[s::STR_BEGIN]).set(&[STRING_1])),
        I(BASIC_STRING),
        I(LITERAL_STRING),
    ]),
    // string-escape
    ctx(&[
        P(Pat::new(escape).sc(&[s::ESCAPE])),
        P(Pat::new(bad_escape).sc(&[s::BAD_ESCAPE])),
    ]),
    // table-array-name
    Ctx::new(&[
        P(Pat::new(close2).caps(&[(1, &[s::TABLE_ARRAY_END])]).pop()),
        I(TABLE_NAME_INC),
    ])
    .content(&[s::META_TABLE_ARRAY]),
    // table-name
    Ctx::new(&[
        P(Pat::new(close1).caps(&[(1, &[s::TABLE_END])]).pop()),
        I(TABLE_NAME_INC),
    ])
    .content(&[s::META_TABLE]),
    // table-name-inc
    ctx(&[
        P(Pat::new(key_ahead).push(&[MAYBE_DOT_TABLE, TABLE_NAME_SIMPLE])),
        P(Pat::new(table_junk).sc(&[s::BAD_TABLE]).pop()),
    ]),
    // table-name-simple
    ctx(&[
        P(Pat::new(bare_key).sc(&[s::TABLE_NAME]).pop()),
        I(KEY_QUOTED),
        P(Pat::new(one_char).sc(&[s::BAD_TABLE]).pop()),
    ]),
    // tables
    ctx(&[
        P(Pat::new(empty_table).sc(&[s::BAD_TABLE])),
        P(Pat::new(open2)
            .caps(&[(1, &[s::TABLE_ARRAY_BEGIN])])
            .push(&[MAYBE_COMMENT_EOL, TABLE_ARRAY_NAME])),
        P(Pat::new(open1)
            .caps(&[(1, &[s::TABLE_BEGIN])])
            .push(&[MAYBE_COMMENT_EOL, TABLE_NAME])),
    ]),
];

static SLOTS: [u16; 40] = slots(&CTXS);
static PROTOS: [bool; 40] = protos(&CTXS, None);
static KINDS: Kinds = Kinds::new();

pub static GRAMMAR: Grammar = Grammar {
    ctxs: &CTXS,
    start: &[START],
    atoms: ATOMS,
    slots: &SLOTS,
    proto: None,
    protos: &PROTOS,
    kinds: &KINDS,
};

// the matchers: each finds its regex's leftmost match at or after `from`

/// `[A-Za-z0-9_'"-]`, what a key or a table name starts with.
fn is_key_start(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'\'' | b'"' | b'-')
}

/// `[A-Za-z0-9_-]`.
fn is_bare(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-')
}

/// `([ \t\n])*`
fn ws_nl(l: &Line, from: usize) -> Option<M> {
    (from <= l.end()).then(|| M::new(from, l.run(from, |c| matches!(c, b' ' | b'\t' | b'\n'))))
}

/// `[ \t]*`
fn blanks(l: &Line, from: usize) -> Option<M> {
    (from <= l.end()).then(|| M::new(from, l.run(from, is_blank)))
}

/// `^[ \t]*`
fn bol_blanks(l: &Line, from: usize) -> Option<M> {
    match from {
        0 => Some(M::new(0, l.run(0, is_blank))),
        _ if from <= l.end() => Some(M::new(l.end(), l.end())),
        _ => None,
    }
}

/// `[ \t]*((#).*)`
fn comment(l: &Line, from: usize) -> Option<M> {
    let k = first(l, from, b'#')?;
    let nl = l.nl();
    Some(
        M::new(back_blanks(l, from, k), nl)
            .g(1, k, nl)
            .g(2, k, k + 1),
    )
}

/// `]`
fn close_bracket(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b']')
}

/// `\[`
fn open_bracket(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'[')
}

/// `\}`
fn close_brace(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'}')
}

/// `\{`
fn open_brace(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'{')
}

/// `,`
fn comma(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b',')
}

/// `"`
fn dquote(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'"')
}

/// `"""`
fn dquote3(l: &Line, from: usize) -> Option<M> {
    l.lit(from, b"\"\"\"")
}

/// `'''`
fn squote3(l: &Line, from: usize) -> Option<M> {
    l.lit(from, b"'''")
}

/// `\n`
fn newline(l: &Line, from: usize) -> Option<M> {
    (from <= l.nl()).then(|| M::new(l.nl(), l.end()))
}

/// The empty regex.
fn empty(l: &Line, from: usize) -> Option<M> {
    (from <= l.end()).then(|| M::new(from, from))
}

/// `(?=[^\n])`, and `(?=\w+|.)`: a character other than the `\n` ahead.
fn not_nl(l: &Line, from: usize) -> Option<M> {
    (from < l.nl()).then(|| M::new(from, from))
}

/// `(?=\w+|.)`
fn char_ahead(l: &Line, from: usize) -> Option<M> {
    not_nl(l, from)
}

/// `.`
fn one_char(l: &Line, from: usize) -> Option<M> {
    (from < l.nl()).then(|| M::new(from, from + l.char_len(from).unwrap_or(1)))
}

/// `\w+|.`
fn word_or_char(l: &Line, from: usize) -> Option<M> {
    if from >= l.nl() {
        return None;
    }
    match is_word(l.b[from]) {
        true => Some(M::new(from, l.run(from, |c| is_word(c) && c != b'\n'))),
        false => one_char(l, from),
    }
}

/// `(.*)`
fn rest(l: &Line, from: usize) -> Option<M> {
    let e = from.max(l.nl());
    (from <= l.end()).then(|| M::new(from, e).g(1, from, e))
}

/// `.+(?m:$)`
fn rest_to_eol(l: &Line, from: usize) -> Option<M> {
    (from < l.nl()).then(|| M::new(from, l.nl()))
}

/// `.+`
fn rest_nonempty(l: &Line, from: usize) -> Option<M> {
    rest_to_eol(l, from)
}

/// `(?m:$)`
fn eol(l: &Line, from: usize) -> Option<M> {
    match from {
        _ if from <= l.nl() => Some(M::new(l.nl(), l.nl())),
        _ if from == l.end() => Some(M::new(from, from)),
        _ => None,
    }
}

/// `[ \t]*(?m:$)`
fn blanks_eol(l: &Line, from: usize) -> Option<M> {
    if from <= l.nl() {
        let p = back_blanks(l, from, l.nl());
        return Some(M::new(p, l.nl()));
    }
    eol(l, from)
}

/// `(true|false)`
fn boolean(l: &Line, from: usize) -> Option<M> {
    let t = l.lit(from, b"true");
    let f = l.lit(from, b"false");
    let m = match (t, f) {
        (Some(t), Some(f)) => {
            if f.s < t.s {
                f
            } else {
                t
            }
        }
        (t, f) => t.or(f)?,
    };
    Some(m.g(1, m.s as usize, m.e as usize))
}

/// `[ \t]*(=)[ \t]*`
fn equals(l: &Line, from: usize) -> Option<M> {
    let k = first(l, from, b'=')?;
    let e = l.run(k + 1, is_blank);
    Some(M::new(back_blanks(l, from, k), e).g(1, k, k + 1))
}

/// `(?=[ \t]*=)`
fn equals_ahead(l: &Line, from: usize) -> Option<M> {
    let k = first(l, from, b'=')?;
    let p = back_blanks(l, from, k);
    Some(M::new(p, p))
}

/// `(?=[A-Za-z0-9_'"-])`
fn key_ahead(l: &Line, from: usize) -> Option<M> {
    let hay = l.b.get(from..)?;
    let i = hay.iter().position(|&c| is_key_start(c))?;
    Some(M::new(from + i, from + i))
}

/// `(?=[ \t]*(?:<c>|(?m:$)))`
fn blanks_then_ahead(l: &Line, from: usize, c: u8) -> Option<M> {
    let mut p = from;
    while p <= l.end() {
        let q = l.run(p, is_blank);
        if l.eol(q) || l.at(q) == Some(c) {
            return Some(M::new(p, p));
        }
        p = q + 1;
    }
    None
}

/// `(?=[ \t]*(?:=|(?m:$)))`
fn equals_or_eol_ahead(l: &Line, from: usize) -> Option<M> {
    blanks_then_ahead(l, from, b'=')
}

/// `(?=[ \t]*(?:\]|(?m:$)))`
fn bracket_or_eol_ahead(l: &Line, from: usize) -> Option<M> {
    blanks_then_ahead(l, from, b']')
}

/// `[A-Za-z0-9_-]+`
fn bare_key(l: &Line, from: usize) -> Option<M> {
    let hay = l.b.get(from..)?;
    let i = from + hay.iter().position(|&c| is_bare(c))?;
    Some(M::new(i, l.run(i, is_bare)))
}

/// `\S+`
fn non_spaces(l: &Line, from: usize) -> Option<M> {
    let hay = l.b.get(from..)?;
    let i = from + hay.iter().position(|&c| !is_space(c))?;
    Some(M::new(i, l.run(i, |c| !is_space(c))))
}

/// `(')[^']*(')`
fn literal(l: &Line, from: usize) -> Option<M> {
    let k = first(l, from, b'\'')?;
    let j = first(l, k + 1, b'\'')?;
    Some(M::new(k, j + 1).g(1, k, k + 1).g(2, j, j + 1))
}

/// `[ \t]*(\.)[ \t]*(?=[A-Za-z0-9_'"-])`
fn dot(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, b'.') {
        let q = l.run(k + 1, is_blank);
        if l.is(q, is_key_start) {
            return Some(M::new(back_blanks(l, from, k), q).g(1, k, k + 1));
        }
        at = k + 1;
    }
    None
}

/// `\\(?:[btnfr"\\]|u\h{4}|U\h{8})`
fn escape(l: &Line, from: usize) -> Option<M> {
    let hex = |at: usize, n: usize| (at..at + n).all(|i| l.is(i, |c| c.is_ascii_hexdigit()));
    let mut at = from;
    while let Some(k) = first(l, at, b'\\') {
        let n = match l.at(k + 1) {
            Some(b'b' | b't' | b'n' | b'f' | b'r' | b'"' | b'\\') => 2,
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

/// `\\.`
fn bad_escape(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, b'\\') {
        if let Some(n) = l.char_len(k + 1) {
            return Some(M::new(k, k + 1 + n));
        }
        at = k + 1;
    }
    None
}

/// `[ \t]*(\]\])`
fn close2(l: &Line, from: usize) -> Option<M> {
    let k = l.lit(from, b"]]")?.s as usize;
    Some(M::new(back_blanks(l, from, k), k + 2).g(1, k, k + 2))
}

/// `[ \t]*(\])`
fn close1(l: &Line, from: usize) -> Option<M> {
    let k = first(l, from, b']')?;
    Some(M::new(back_blanks(l, from, k), k + 1).g(1, k, k + 1))
}

/// `[^\]]+\]?`, the `\n` among what it takes.
fn table_junk(l: &Line, from: usize) -> Option<M> {
    let p = (from..l.end()).find(|&i| l.at(i) != Some(b']'))?;
    let q = l.run(p, |c| c != b']');
    Some(M::new(p, q + usize::from(l.at(q) == Some(b']'))))
}

/// `\[?\[[ \t]*\]\]?`
fn empty_table(l: &Line, from: usize) -> Option<M> {
    let close = |at: usize| {
        let q = l.run(at, is_blank);
        (l.at(q) == Some(b']')).then(|| q + 1 + usize::from(l.at(q + 1) == Some(b']')))
    };
    let mut at = from;
    while let Some(k) = first(l, at, b'[') {
        let e = match l.at(k + 1) {
            Some(b'[') => close(k + 2).or_else(|| close(k + 1)),
            _ => close(k + 1),
        };
        if let Some(e) = e {
            return Some(M::new(k, e));
        }
        at = k + 1;
    }
    None
}

/// `(\[\[)[ \t]*`
fn open2(l: &Line, from: usize) -> Option<M> {
    let k = l.lit(from, b"[[")?.s as usize;
    Some(M::new(k, l.run(k + 2, is_blank)).g(1, k, k + 2))
}

/// `(\[)[ \t]*`
fn open1(l: &Line, from: usize) -> Option<M> {
    let k = first(l, from, b'[')?;
    Some(M::new(k, l.run(k + 1, is_blank)).g(1, k, k + 1))
}

fn digit(l: &Line, i: usize) -> bool {
    l.is(i, |c| c.is_ascii_digit())
}

/// `(?<!\w)` at `p`.
fn word_start(l: &Line, p: usize) -> bool {
    p == 0 || l.at(p - 1).is_none_or(|c| !is_word(c) || c == b'\n')
}

/// `(?!\w)` at `p`.
fn word_end(l: &Line, p: usize) -> bool {
    l.at(p).is_none_or(|c| !is_word(c) || c == b'\n')
}

/// The end of `d(?:d|_d)*` from `at`, its first `d` there already.
fn digits_from(l: &Line, mut at: usize, d: impl Fn(u8) -> bool) -> usize {
    loop {
        if l.is(at, &d) {
            at += 1;
        } else if l.at(at) == Some(b'_') && l.is(at + 1, &d) {
            at += 2;
        } else {
            return at;
        }
    }
}

/// `[0-9](?:[0-9]|_[0-9])*` at `at`.
fn decimals(l: &Line, at: usize) -> Option<usize> {
    digit(l, at).then(|| digits_from(l, at + 1, |c| c.is_ascii_digit()))
}

/// `[eE][\+\-]?[0-9](?:[0-9]|_[0-9])*` at `at`.
fn exponent(l: &Line, at: usize) -> Option<usize> {
    if !l.is(at, |c| c == b'e' || c == b'E') {
        return None;
    }
    let at = at + 1 + usize::from(l.is(at + 1, |c| c == b'+' || c == b'-'));
    decimals(l, at)
}

/// The decimal integer part from `q`, in the regex's order: one digit, then a run that
/// doesn't start with `0`; each end `then` accepts.
fn int_part(l: &Line, q: usize, then: impl Fn(usize) -> Option<usize>) -> Option<usize> {
    if digit(l, q)
        && let Some(e) = then(q + 1)
    {
        return Some(e);
    }
    if l.is(q, |c| (b'1'..=b'9').contains(&c)) {
        let r = digits_from(l, q + 1, |c| c.is_ascii_digit());
        if r > q + 1 {
            return then(r);
        }
    }
    None
}

/// A sign at `p`, `[\+\-]?`: where the number after it starts.
fn sign(l: &Line, p: usize) -> usize {
    p + usize::from(l.is(p, |c| c == b'+' || c == b'-'))
}

/// The float regex's fraction or exponent at `r`, then `(?!\w)`.
fn fraction(l: &Line, r: usize) -> Option<usize> {
    if l.at(r) == Some(b'.')
        && let Some(e) = decimals(l, r + 1)
    {
        if word_end(l, e) {
            return Some(e);
        }
        if let Some(e) = exponent(l, e).filter(|&e| word_end(l, e)) {
            return Some(e);
        }
    }
    exponent(l, r).filter(|&e| word_end(l, e))
}

/// `(?<!\w)([\+\-]?)(?:[0-9]|[1-9](?:[0-9]|_[0-9])+)(?:\.…|\.…[eE]…|[eE]…)(?!\w)`
fn float(l: &Line, from: usize) -> Option<M> {
    l.find(from, |p| {
        if !word_start(l, p) {
            return None;
        }
        let q = sign(l, p);
        let e = int_part(l, q, |r| fraction(l, r))?;
        Some(M::new(p, e).g(1, p, q))
    })
}

/// `(?<!\w)([+-])?(inf|nan)(?!\w)`
fn inf_nan(l: &Line, from: usize) -> Option<M> {
    l.find(from, |p| {
        if !word_start(l, p) {
            return None;
        }
        let q = sign(l, p);
        let word = l.b.get(q..q + 3)?;
        if (word == b"inf" || word == b"nan") && word_end(l, q + 3) {
            let m = M::new(p, q + 3);
            return Some(if q > p { m.g(1, p, q) } else { m });
        }
        None
    })
}

/// `(?<!\w)0x…|0o…|0b…|([\+\-]?)(?:[0-9]|[1-9](?:[0-9]|_[0-9])+)(?!\w)`: the top-level `|`
/// binds loosest, so the look-behind is the hex branch's alone and the look-ahead the
/// decimal one's.
fn integer(l: &Line, from: usize) -> Option<M> {
    l.find(from, |p| {
        if l.at(p) == Some(b'0') {
            let radix: [(u8, Digit); 3] = [
                (b'x', |c| c.is_ascii_hexdigit()),
                (b'o', |c| (b'0'..=b'7').contains(&c)),
                (b'b', |c| c == b'0' || c == b'1'),
            ];
            for (n, (mark, d)) in radix.into_iter().enumerate() {
                if (n > 0 || word_start(l, p)) && l.at(p + 1) == Some(mark) && l.is(p + 2, d) {
                    return Some(M::new(p, digits_from(l, p + 3, d)));
                }
            }
        }
        let q = sign(l, p);
        let e = int_part(l, q, |r| word_end(l, r).then_some(r))?;
        Some(M::new(p, e).g(1, p, q))
    })
}

/// A class of digits.
type Digit = fn(u8) -> bool;

/// `[0-9]{n}` at `at`.
fn fixed_digits(l: &Line, at: usize, n: usize) -> bool {
    (at..at + n).all(|i| digit(l, i))
}

/// `[lo-hi][0-9]` at `at`.
fn pair(l: &Line, at: usize, hi: u8) -> bool {
    l.is(at, |c| (b'0'..=hi).contains(&c)) && digit(l, at + 1)
}

/// `[0-9]{4}-[0-1][0-9]-[0-3][0-9]` at `at`: its end.
fn date(l: &Line, at: usize) -> Option<usize> {
    let ok = fixed_digits(l, at, 4)
        && l.at(at + 4) == Some(b'-')
        && pair(l, at + 5, b'1')
        && l.at(at + 7) == Some(b'-')
        && pair(l, at + 8, b'3');
    ok.then_some(at + 10)
}

/// `[0-2][0-9]:[0-5][0-9]:[0-6][0-9](?:\.[0-9]+)?` at `at`: its end.
fn time(l: &Line, at: usize) -> Option<usize> {
    let ok = pair(l, at, b'2')
        && l.at(at + 2) == Some(b':')
        && pair(l, at + 3, b'5')
        && l.at(at + 5) == Some(b':')
        && pair(l, at + 6, b'6');
    if !ok {
        return None;
    }
    let e = at + 8;
    Some(match l.at(e) == Some(b'.') && digit(l, e + 1) {
        true => l.run(e + 1, |c| c.is_ascii_digit()),
        false => e,
    })
}

/// `[zZ]|[+-][0-2][0-9]:[0-5][0-9]` at `at`: its end.
fn offset(l: &Line, at: usize) -> Option<usize> {
    match l.at(at)? {
        b'z' | b'Z' => Some(at + 1),
        b'+' | b'-'
            if pair(l, at + 1, b'2') && l.at(at + 3) == Some(b':') && pair(l, at + 4, b'5') =>
        {
            Some(at + 6)
        }
        _ => None,
    }
}

/// The date-time regex: an offset date-time, a local date-time, a date, a time, in that
/// order.
fn datetime(l: &Line, from: usize) -> Option<M> {
    l.find(from, |p| {
        if let Some(d) = date(l, p) {
            if l.is(d, |c| matches!(c, b't' | b'T' | b' '))
                && let Some(t) = time(l, d + 1)
            {
                return Some(M::new(p, offset(l, t).unwrap_or(t)));
            }
            return Some(M::new(p, d));
        }
        time(l, p).map(|t| M::new(p, t))
    })
}

#[cfg(test)]
mod tests {
    use crate::Lang;
    use crate::tests::marked;

    fn kinds(lines: &[&'static str]) -> Vec<(&'static str, &'static str)> {
        marked(Lang::Toml, lines)
            .into_iter()
            .map(|(t, s)| (t, s.rsplit(' ').next().unwrap()))
            .collect()
    }

    #[test]
    fn keys_and_values() {
        assert_eq!(
            kinds(&["name = \"ada\" # why"]),
            [
                ("name", "entity.name.tag.toml"),
                ("=", "punctuation.definition.key-value.toml"),
                ("\"", "punctuation.definition.string.begin.toml"),
                ("ada", "string.quoted.double.basic.toml"),
                ("\"", "punctuation.definition.string.end.toml"),
                ("#", "punctuation.definition.comment.toml"),
                (" why", "comment.line.number-sign.toml"),
            ]
        );
    }

    #[test]
    fn arrays_run_across_lines() {
        let got = kinds(&["x = [", "  1, -2.5,", "]"]);
        assert!(
            got.contains(&("-", "keyword.operator.arithmetic.toml")),
            "{got:?}"
        );
        assert!(
            got.contains(&("1", "constant.numeric.integer.toml")),
            "{got:?}"
        );
        assert_eq!(
            got.last(),
            Some(&("]", "punctuation.definition.array.end.toml"))
        );
    }
}
