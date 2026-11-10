//! Rust by a scanner of ada's own (ADR-40, speed first; X-090), designed from the color census
//! (X-089) and not from bat's contexts: what the 14 built-in themes tell apart is a token's leaf
//! and whether it is in an attribute (`meta.annotation`), and a leaf is told by the token's own
//! bytes or word (keywords, primitives, support words, constants, literals, punctuation), the
//! byte after it (a call's `(`, a macro's `!`, a field's or a parameter's `:`), the token before
//! it (`fn`, `struct`, `enum`, `trait`, `type`, `mod`, `macro_rules!` naming what follows; `::`;
//! an operator before a closure's `|`), or a frame (a function's parameters, a closure's, a
//! struct's fields, an `impl`'s head, a generic's angles, an attribute).
//!
//! So the code is one loop over a line's bytes: a blank run and the token after it in one turn,
//! one action a byte from a table, a name read once and looked up once in [`WORDS`], the previous
//! significant token in a byte ([`Scan::prev`]) and frames only where a frame colors: brackets,
//! parameters, closures, a struct's body, an `impl`'s head, generics, the type frames,
//! `macro_rules!`, attributes, strings and comments. bat's `Rust` grammar is the reference for
//! scopes and the parity oracle by color (`examples/parity.rs`); where it derails (a
//! `with_prototype` that never pops, a pattern that reads a type as a parameter) the frames
//! don't follow it, each departure named where it is.
//!
//! A kind is a leaf ([`LEAVES`], bat's scopes for the token), plus [`NLEAVES`] in an attribute
//! ([`scopes`]). The grammar's other meta scopes (a function's, a block's, a group's) aren't
//! kept. Names past ASCII are approximated: every byte past ASCII is a letter, neither upper-
//! nor lowercase.
//!
//! The state between lines is the frames open above the top level, up to [`State`]'s levels,
//! the previous token, an open raw string's `#`s, whether an attribute is open and whether a SQL
//! marker waits for its string.
//!
//! A string that `Regex::new(`, `RegexBuilder::new(` or `regex!(` takes is colored as the
//! `regex` crate's regex (ADR-40, phase 4; [`crate::regex`]), as RustRover colors it: its text
//! is the regex's, but for the string's end and (in `"…"`) its escapes.
//!
//! SQL by call site and marker (ADR-40, phase 4; [`crate::sql`]), as RustRover has it: the SQL
//! argument of sqlx's, diesel's, rusqlite's and postgres' calls (`sqlx::query!(`,
//! `query_as!(T, `, `sql_query(`, `conn.execute(`…), and the string after `// language=SQL`.
//! Its text is SQL's, but for the string's end and (in `"…"`) its escapes.

use crate::clike::{CLASS, HIGH, LOWER, SPACE, UPPER, WORD};
use crate::emit::{Rules, Run, Runs, Sink, Spans};
use crate::prim::{self, char_len};
use crate::regex;
use crate::sql;
use crate::words::{Words, text_len};
use crate::{Out, State};

// The text frames' ids, above the code's (`F_*`, below): an attribute's, the strings'
// and the comments' turns, then SQL's and a regex's in a string.
/// Where a SQL call's SQL argument is ([`F_PSQL`] plus this): at the first argument, the SQL
/// (`sql_query(`, `query!(`, `conn.execute(`…).
const PSQL_SQL: u8 = 0;
/// At the first argument: a type, then the SQL (`query_as!(`).
const PSQL_TYPE: u8 = 1;
/// In the type, waiting for its `,`.
const PSQL_SKIP: u8 = 2;
/// `#[ … ]`, its parameters, a call in them.
const ATTRIBUTE: u8 = 63;
const ATTR_PARAMS: u8 = 64;
const ATTR_CALL: u8 = 65;
// the strings' and the comments'
const STRING: u8 = 70;
const _: () = assert!(ATTR_CALL < STRING);
const FMT_STRING: u8 = 71;
const BYTE_STRING: u8 = 72;
const RAW_STRING: u8 = 73;
const FMT_RAW: u8 = 74;
const RAW_BYTES: u8 = 75;
const BLOCK_COMMENT: u8 = 76;
const DOC_COMMENT: u8 = 77;
/// SQL in the string below (ADR-40, phase 4): this, plus SQL's context there
/// ([`crate::sql`]'s `MAIN` to `HASHES`).
const SQL_IN: u8 = 0x50;
const SQL_LAST: u8 = SQL_IN + sql::HASHES;
const _: () = assert!(DOC_COMMENT < SQL_IN);
/// A regex in the string below (ADR-40, phase 4): this bit, and the regex's state.
const REGEX: u8 = 0x80;
const _: () = assert!(SQL_LAST < REGEX);

/// The leaves: a token's scopes. A kind is a leaf, plus [`NLEAVES`] in an attribute.
static LEAVES: [&str; 107] = [
    "",
    "comment.line.double-slash.rust",
    "comment.line.double-slash.rust punctuation.definition.comment.rust",
    "comment.line.double-slash.rust variable.other.rust",
    "comment.line.documentation.rust",
    "comment.line.documentation.rust punctuation.definition.comment.rust",
    "comment.line.documentation.rust variable.other.rust",
    "comment.block.rust",
    "comment.block.rust punctuation.definition.comment.rust",
    "comment.block.rust variable.other.rust",
    "comment.block.documentation.rust",
    "comment.block.documentation.rust punctuation.definition.comment.rust",
    "comment.block.documentation.rust variable.other.rust",
    "string.quoted.double.rust",
    "string.quoted.double.rust punctuation.definition.string.begin.rust",
    "string.quoted.double.rust punctuation.definition.string.end.rust",
    "string.quoted.double.rust constant.character.escape.rust",
    "string.quoted.double.rust punctuation.separator.continuation.line.rust",
    "string.quoted.double.rust storage.type.string.rust",
    "string.quoted.double.rust constant.other.placeholder.rust",
    "string.quoted.double.raw.rust",
    "string.quoted.double.raw.rust punctuation.definition.string.begin.rust",
    "string.quoted.double.raw.rust punctuation.definition.string.end.rust",
    "string.quoted.double.raw.rust storage.type.string.rust",
    "string.quoted.double.raw.rust constant.character.escape.rust",
    "string.quoted.double.raw.rust constant.other.placeholder.rust",
    "string.quoted.single.rust",
    "string.quoted.single.rust punctuation.definition.string.begin.rust",
    "string.quoted.single.rust punctuation.definition.string.end.rust",
    "string.quoted.single.rust constant.character.escape.rust",
    "string.quoted.single.rust storage.type.string.rust",
    "punctuation.terminator.rust",
    "punctuation.separator.rust",
    "punctuation.separator.type.rust",
    "punctuation.separator.generic.rust",
    "punctuation.accessor.dot.rust",
    "meta.path.rust punctuation.accessor.rust",
    "meta.path.rust storage.type.rust punctuation.accessor.rust",
    "punctuation.section.group.begin.rust",
    "punctuation.section.group.end.rust",
    "meta.group.rust punctuation.section.group.begin.rust",
    "meta.group.rust punctuation.section.group.end.rust",
    "punctuation.section.block.begin.rust",
    "meta.block.rust punctuation.section.block.end.rust",
    "meta.block.rust punctuation.section.block.begin.rust",
    "punctuation.section.parameters.begin.rust",
    "punctuation.section.parameters.end.rust",
    "meta.function.parameters.rust punctuation.section.parameters.end.rust",
    "punctuation.definition.generic.begin.rust",
    "punctuation.definition.generic.end.rust",
    "punctuation.definition.annotation.rust",
    "meta.annotation.parameters.rust meta.group.rust punctuation.section.group.begin.rust",
    "meta.annotation.parameters.rust meta.group.rust punctuation.section.group.end.rust",
    "meta.function-call.rust meta.group.rust punctuation.section.group.begin.rust",
    "meta.function-call.rust meta.group.rust punctuation.section.group.end.rust",
    "meta.function-call.rust variable.function.rust",
    "punctuation.definition.modifier-scope.begin.rust",
    "punctuation.definition.modifier-scope.end.rust",
    "punctuation.terminator.macro-matcher.rust",
    "punctuation.section.group.begin.rust keyword.operator.rust",
    "keyword.other.rust",
    "keyword.control.rust",
    "keyword.operator.rust",
    "keyword.operator.assignment.rust",
    "keyword.operator.comparison.rust",
    "keyword.operator.logical.rust",
    "keyword.operator.arithmetic.rust",
    "keyword.operator.bitwise.rust",
    "keyword.operator.range.rust",
    "storage.type.rust",
    "storage.type.numeric.rust",
    "storage.type.function.rust",
    "storage.type.struct.rust",
    "storage.type.impl.rust",
    "storage.type.enum.rust",
    "storage.type.type.rust",
    "storage.type.trait.rust",
    "storage.type.module.rust",
    "storage.modifier.rust",
    "storage.modifier.lifetime.rust",
    "meta.path.rust storage.type.rust storage.type.rust",
    "entity.name.function.rust",
    "entity.name.struct.rust",
    "entity.name.impl.rust",
    "entity.name.enum.rust",
    "entity.name.trait.rust",
    "entity.name.type.rust",
    "entity.name.module.rust",
    "entity.name.macro.rust",
    "entity.name.label.rust",
    "variable.parameter.rust",
    "variable.parameter.macro.rust",
    "variable.other.member.rust",
    "variable.language.rust",
    "variable.other.rust",
    "variable.annotation.rust",
    "support.function.rust",
    "support.macro.rust",
    "support.type.rust",
    "constant.other.rust",
    "constant.language.rust",
    "constant.numeric.integer.decimal.rust",
    "constant.numeric.integer.hexadecimal.rust",
    "constant.numeric.integer.octal.rust",
    "constant.numeric.integer.binary.rust",
    "constant.numeric.float.rust",
    "invalid.illegal.rust",
];

// The leaves by name, in the order of `LEAVES`.
const PLAIN: u16 = 0;
const COMMENT: u16 = 1;
const DOC: u16 = 4;
const BLOCK_TEXT: u16 = 7;
const DOC_BLOCK: u16 = 10;
/// After one of the four comments' text: its mark, then a `$name` in it.
const MARK: u16 = 1;
const VAR: u16 = 2;
const STR: u16 = 13;
const STR_BEGIN: u16 = 14;
const STR_END: u16 = 15;
const STR_ESCAPE: u16 = 16;
const STR_CONTINUATION: u16 = 17;
const STR_PREFIX: u16 = 18;
const STR_PLACEHOLDER: u16 = 19;
const RAW: u16 = 20;
const RAW_BEGIN: u16 = 21;
const RAW_END: u16 = 22;
const RAW_PREFIX: u16 = 23;
const RAW_ESCAPE: u16 = 24;
const RAW_PLACEHOLDER: u16 = 25;
const CHAR: u16 = 26;
const CHAR_BEGIN: u16 = 27;
const CHAR_END: u16 = 28;
const CHAR_ESCAPE: u16 = 29;
const CHAR_PREFIX: u16 = 30;
const TERMINATOR: u16 = 31;
const SEPARATOR: u16 = 32;
const SEPARATOR_TYPE: u16 = 33;
const SEPARATOR_GENERIC: u16 = 34;
const DOT: u16 = 35;
const PATH_ACCESSOR: u16 = 36;
const TYPE_ACCESSOR: u16 = 37;
const GROUP_BEGIN: u16 = 38;
const GROUP_END: u16 = 39;
const META_GROUP_BEGIN: u16 = 40;
const META_GROUP_END: u16 = 41;
const BLOCK_BEGIN: u16 = 42;
const BLOCK_END: u16 = 43;
const META_BLOCK_BEGIN: u16 = 44;
const PARAMS_BEGIN: u16 = 45;
const PARAMS_END: u16 = 46;
const FN_PARAMS_END: u16 = 47;
const GENERIC_BEGIN: u16 = 48;
const GENERIC_END: u16 = 49;
const ANNOTATION_MARK: u16 = 50;
const ANNOTATION_BEGIN: u16 = 51;
const ANNOTATION_END: u16 = 52;
const CALL_BEGIN: u16 = 53;
const CALL_END: u16 = 54;
const CALL: u16 = 55;
const MODIFIER_BEGIN: u16 = 56;
const MODIFIER_END: u16 = 57;
const MACRO_TERMINATOR: u16 = 58;
const REPEAT_BEGIN: u16 = 59;
const KEYWORD: u16 = 60;
const CONTROL: u16 = 61;
const OPERATOR: u16 = 62;
const ASSIGNMENT: u16 = 63;
const COMPARISON: u16 = 64;
const LOGICAL: u16 = 65;
const ARITHMETIC: u16 = 66;
const BITWISE: u16 = 67;
const RANGE: u16 = 68;
const STORAGE: u16 = 69;
const NUMERIC: u16 = 70;
const FN: u16 = 71;
const STRUCT: u16 = 72;
const IMPL: u16 = 73;
const ENUM: u16 = 74;
const TYPE: u16 = 75;
const TRAIT: u16 = 76;
const MODULE: u16 = 77;
const MODIFIER: u16 = 78;
const LIFETIME: u16 = 79;
const PATH_TYPE: u16 = 80;
const FN_NAME: u16 = 81;
const STRUCT_NAME: u16 = 82;
const IMPL_NAME: u16 = 83;
const ENUM_NAME: u16 = 84;
const TRAIT_NAME: u16 = 85;
const TYPE_NAME: u16 = 86;
const MODULE_NAME: u16 = 87;
const MACRO_NAME: u16 = 88;
const LABEL: u16 = 89;
const PARAMETER: u16 = 90;
const MACRO_PARAMETER: u16 = 91;
const MEMBER: u16 = 92;
const SELF: u16 = 93;
const VARIABLE: u16 = 94;
const ANNOTATION: u16 = 95;
const SUPPORT_FN: u16 = 96;
const SUPPORT_MACRO: u16 = 97;
const SUPPORT_TYPE: u16 = 98;
const CONSTANT: u16 = 99;
const LANGUAGE: u16 = 100;
const DECIMAL: u16 = 101;
const HEX: u16 = 102;
const OCTAL: u16 = 103;
const BINARY: u16 = 104;
const FLOAT: u16 = 105;
const INVALID: u16 = 106;

const NLEAVES: u16 = LEAVES.len() as u16;

/// The kinds of a regex in a `"…"` string and in a raw one: the regex's, from each.
const RE_STRING: u16 = 256;
const RE_RAW: u16 = RE_STRING + regex::KINDS as u16;
const _: () = assert!(2 * NLEAVES <= RE_STRING);
/// SQL in a `"…"` string, then in a raw one: SQL's kinds from each.
const SQL_STRING: u16 = RE_RAW + regex::KINDS as u16;
const SQL_RAW: u16 = SQL_STRING + sql::KINDS as u16;

/// The scopes under `source.rust` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    if kind >= SQL_STRING {
        let (text, base) = if kind >= SQL_RAW {
            (RAW, SQL_RAW)
        } else {
            (STR, SQL_STRING)
        };
        if usize::from(kind - base) >= sql::KINDS {
            return Vec::new();
        }
        let mut out = scopes(text);
        out.extend(sql::scopes(kind - base));
        return out;
    }
    if kind >= RE_STRING {
        let (text, base) = if kind >= RE_RAW {
            (RAW, RE_RAW)
        } else {
            (STR, RE_STRING)
        };
        let mut out = scopes(text);
        out.extend(regex::scopes(kind - base));
        return out;
    }
    let mut out = Vec::new();
    if kind >= NLEAVES {
        out.push("meta.annotation.rust");
    }
    let leaf = LEAVES[usize::from(kind % NLEAVES)];
    out.extend(leaf.split(' ').filter(|s| !s.is_empty()));
    out
}

/// How many contexts a line can hold; one pushed past it drops the bottom one.
const MAX: usize = 128;

/// A byte of a word: `\w`, every byte past ASCII among them (the core's classes, one load).
#[inline(always)]
fn is_word(c: u8) -> bool {
    CLASS[usize::from(c)] & (WORD | HIGH) != 0
}

#[inline(always)]
fn is_letter(c: u8) -> bool {
    CLASS[usize::from(c)] & (UPPER | LOWER | HIGH) != 0
}

/// `\s` as far as ASCII goes.
#[inline(always)]
fn is_space(c: u8) -> bool {
    CLASS[usize::from(c)] & SPACE != 0
}

#[inline(always)]
fn skip_spaces(b: &[u8], p: usize) -> usize {
    crate::clike::skip_blanks(b, p)
}

/// A line's indentation, four spaces a step, then the rest of its blanks a byte a step: it is
/// 14.5% of the corpus's bytes, 8 a run, nearly always spaces in fours.
#[inline(always)]
fn skip_indent(b: &[u8]) -> usize {
    let mut p = 0;
    while b.get(p..p + 4).is_some_and(|w| w == b"    ") {
        p += 4;
    }
    crate::clike::skip_blanks(b, p)
}

#[inline(always)]
fn word_end(b: &[u8], p: usize) -> usize {
    crate::clike::class_run(b, p, WORD | HIGH).end
}

/// Whether `\b` holds before `p`, at a word's start.
fn starts_word(b: &[u8], p: usize) -> bool {
    p == 0 || !is_word(b[p - 1])
}

/// Whether `\b` holds at `e`, after a word.
fn ends_word(b: &[u8], e: usize) -> bool {
    e >= b.len() || !is_word(b[e])
}

/// bat's identifier, `(?:[\p{L}][_\p{L}\p{N}]*|_[_\p{L}\p{N}]+)\b`, at `p`: where it ends.
fn ident_end(b: &[u8], p: usize) -> Option<usize> {
    let c = *b.get(p)?;
    let ok = is_letter(c) || (c == b'_' && b.get(p + 1).is_some_and(|&d| is_word(d)));
    ok.then(|| word_end(b, p))
}

/// Whether bat's identifier starts at `p`: [`ident_end`]'s test, without reading the word.
#[inline(always)]
fn ident_starts(b: &[u8], p: usize) -> bool {
    match b.get(p) {
        Some(&c) if is_letter(c) => true,
        Some(b'_') => b.get(p + 1).is_some_and(|&d| is_word(d)),
        _ => false,
    }
}

/// Whether a whole word is bat's identifier.
fn is_ident(w: &[u8]) -> bool {
    match w {
        [c, ..] if is_letter(*c) => true,
        [b'_', _, ..] => true,
        _ => false,
    }
}

/// `[\p{Lu}]|_*[\p{Lu}][\p{L}\p{N}_]*[\p{Ll}][\p{L}\p{N}_]*`, a whole word: a type's name
/// before `::`.
fn is_type_name(w: &[u8]) -> bool {
    let rest = &w[w.iter().take_while(|&&c| c == b'_').count()..];
    match rest.split_first() {
        Some((c, tail)) if c.is_ascii_uppercase() => {
            (w.len() == 1) || tail.iter().any(u8::is_ascii_lowercase)
        }
        _ => false,
    }
}

fn is_int_suffix(w: &[u8]) -> bool {
    matches!(
        w,
        b"i8"
            | b"i16"
            | b"i32"
            | b"i64"
            | b"i128"
            | b"isize"
            | b"u8"
            | b"u16"
            | b"u32"
            | b"u64"
            | b"u128"
            | b"usize"
    )
}

fn digits_end(b: &[u8], mut p: usize, digit: impl Fn(u8) -> bool) -> usize {
    while p < b.len() && (digit(b[p]) || b[p] == b'_') {
        p += 1;
    }
    p
}

/// A character escape at `p` (a `\`): `\\(x\h{2}|n|r|t|0|"|'|\\)`, and with `unicode`
/// `|u\{\h{1,6}\}`. Where it ends.
fn escape(b: &[u8], p: usize, unicode: bool) -> Option<usize> {
    match *b.get(p + 1)? {
        b'n' | b'r' | b't' | b'0' | b'"' | b'\'' | b'\\' => Some(p + 2),
        b'x' if b.get(p + 2..p + 4)?.iter().all(u8::is_ascii_hexdigit) => Some(p + 4),
        b'u' if unicode && b.get(p + 2) == Some(&b'{') => {
            let n = b[p + 3..]
                .iter()
                .take_while(|c| c.is_ascii_hexdigit())
                .count();
            ((1..=6).contains(&n) && b.get(p + 3 + n) == Some(&b'}')).then_some(p + 4 + n)
        }
        _ => None,
    }
}

/// The bytes a string's patterns start on, around a regex: `"…"`'s end and escapes, then a raw
/// string's end.
static RE_YIELDS: [[bool; 256]; 2] = {
    let mut t = [[false; 256]; 2];
    t[0][b'"' as usize] = true;
    t[0][b'\\' as usize] = true;
    t[1][b'"' as usize] = true;
    t
};

/// Whether the string whose prefix or quote is at `p` is a regex: the first argument of
/// `Regex::new(`, `RegexBuilder::new(` (`bytes::` ones too) or `regex!(`, looked back on its
/// line.
fn regex_string(b: &[u8], p: usize) -> bool {
    let mut q = p;
    while q > 0 && is_space(b[q - 1]) {
        q -= 1;
    }
    if q == 0 || b[q - 1] != b'(' {
        return false;
    }
    let mut w = q - 1;
    while w > 0 && is_space(b[w - 1]) {
        w -= 1;
    }
    // the name's last byte first: most calls end in neither
    let names: &[&[u8]] = match w.checked_sub(1).map(|i| b[i]) {
        Some(b'w') => &[b"Regex::new", b"RegexBuilder::new"],
        Some(b'!') => &[b"regex!"],
        _ => return false,
    };
    names
        .iter()
        .any(|name| b[..w].ends_with(name) && (w == name.len() || !is_word(b[w - name.len() - 1])))
}

/// The call `name` (a macro's with `mac`) whose arguments hold SQL, as RustRover reads them: the
/// `PSQL_*` state they start in (on [`F_PSQL`]). sqlx's `query`, `query_as`, `query_scalar` and their `_with`
/// forms, its macros (`query!`, `query_scalar!`, `query_unchecked!`…, and `query_as!` after a
/// type), diesel's `sql_query`, `sql` and `push_sql`, rusqlite's and postgres' `execute`,
/// `execute_batch`, `prepare`, `query_row`, `query`, `query_one`, `batch_execute`….
fn sql_call(name: &[u8], mac: bool) -> Option<u8> {
    if mac {
        return match name {
            b"query" | b"query_scalar" | b"query_unchecked" | b"query_scalar_unchecked" => {
                Some(PSQL_SQL)
            }
            b"query_as" | b"query_as_unchecked" => Some(PSQL_TYPE),
            _ => None,
        };
    }
    // the first byte first: most calls start with none of these
    let sql = match name.first()? {
        b'q' => matches!(
            name,
            b"query"
                | b"query_as"
                | b"query_scalar"
                | b"query_with"
                | b"query_as_with"
                | b"query_scalar_with"
                | b"query_row"
                | b"query_one"
                | b"query_opt"
                | b"query_raw"
                | b"query_typed"
        ),
        b's' => matches!(name, b"sql_query" | b"sql" | b"simple_query"),
        b'p' => matches!(
            name,
            b"push_sql" | b"prepare" | b"prepare_cached" | b"prepare_typed"
        ),
        b'e' => matches!(name, b"execute" | b"execute_batch"),
        b'b' => name == b"batch_execute",
        _ => false,
    };
    sql.then_some(PSQL_SQL)
}

/// Whether the `.` at `p` follows a receiver named as a prepared statement's (`stmt.execute(`,
/// `insert_statement.query(`, ASCII case ignored), whose arguments are the SQL's, not SQL.
fn stmt_receiver(b: &[u8], p: usize) -> bool {
    let mut s = p;
    while s > 0 && (b[s - 1].is_ascii_alphanumeric() || b[s - 1] == b'_') {
        s -= 1;
    }
    let r = b[s..p].to_ascii_lowercase();
    r.windows(4).any(|w| w == b"stmt") || r.windows(9).any(|w| w == b"statement")
}

/// A turbofish's `<…>` at `p`, its `<`, on the line: where the byte after its `>` is.
fn turbofish_end(b: &[u8], p: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, &c) in b[p..].iter().enumerate() {
        match c {
            b'<' => depth += 1,
            b'>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(p + i + 1);
                }
            }
            b'(' | b')' | b';' | b'{' | b'}' | b'"' => return None,
            _ => {}
        }
    }
    None
}

/// Where the string SQL is in ends on its line, from `p` inside it: its quote (a raw one's
/// with `hashes` `#`s after it), or the line's end.
fn string_end(b: &[u8], mut p: usize, hashes: Option<usize>) -> usize {
    while p < b.len() {
        match (b[p], hashes) {
            (b'"', None) => return p,
            (b'"', Some(h))
                if b.len() > p + h && b[p + 1..p + 1 + h].iter().all(|&c| c == b'#') =>
            {
                return p;
            }
            (b'\\', None) => p += 2,
            _ => p += 1,
        }
    }
    b.len()
}

/// A character literal's body at `p`, `[^'\\]` or an escape: where it ends, and whether it
/// is an escape.
fn char_body(b: &[u8], p: usize, unicode: bool) -> Option<(usize, bool)> {
    match *b.get(p)? {
        b'\\' => escape(b, p, unicode).map(|e| (e, true)),
        b'\'' => None,
        c => Some((p + char_len(c), false)),
    }
}

/// bat's format placeholder at `p`, a `{`: where it ends.
///
/// `\{ (\d+|name)? (: (.?[<>^])? [+-]? \#? 0? (\d\$?)? (\.(\d\$?|\*)?)? (\?|name)? )? \}`:
/// a width or a precision of one digit only.
fn placeholder(b: &[u8], p: usize) -> Option<usize> {
    let mut i = p + 1;
    if b.get(i).is_some_and(u8::is_ascii_digit) {
        i += b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
    } else if let Some(e) = ident_end(b, i) {
        i = e;
    }
    match b.get(i) {
        Some(b'}') => Some(i + 1),
        Some(b':') => spec(b, i + 1),
        _ => None,
    }
}

/// A placeholder's spec at `i`, after its `:`, and its `}`: where they end. Each optional part
/// is tried as the regex tries it, greedy first.
fn spec(b: &[u8], i: usize) -> Option<usize> {
    let at = |j: usize| b.get(j).copied();
    let align = |c: Option<u8>| matches!(c, Some(b'<' | b'>' | b'^'));
    let mut fills = [None; 3];
    if let Some(c) = at(i) {
        let n = char_len(c);
        if align(at(i + n)) {
            fills[0] = Some(i + n + 1);
        }
    }
    if align(at(i)) {
        fills[1] = Some(i + 1);
    }
    fills[2] = Some(i);
    for j in fills.into_iter().flatten() {
        let mut j = j;
        if matches!(at(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if at(j) == Some(b'#') {
            j += 1;
        }
        let zeros = [(at(j) == Some(b'0')).then_some(j + 1), Some(j)];
        for z in zeros.into_iter().flatten() {
            let digit = at(z).is_some_and(|c| c.is_ascii_digit());
            let widths = [
                (digit && at(z + 1) == Some(b'$')).then_some(z + 2),
                digit.then_some(z + 1),
                Some(z),
            ];
            for w in widths.into_iter().flatten() {
                let dot = at(w) == Some(b'.');
                let digit = dot && at(w + 1).is_some_and(|c| c.is_ascii_digit());
                let precisions = [
                    (digit && at(w + 2) == Some(b'$')).then_some(w + 3),
                    digit.then_some(w + 2),
                    (dot && at(w + 1) == Some(b'*')).then_some(w + 2),
                    dot.then_some(w + 1),
                    Some(w),
                ];
                for r in precisions.into_iter().flatten() {
                    let types = [
                        (at(r) == Some(b'?')).then_some(r + 1),
                        ident_end(b, r),
                        Some(r),
                    ];
                    for t in types.into_iter().flatten() {
                        if at(t) == Some(b'}') {
                            return Some(t + 1);
                        }
                    }
                }
            }
        }
    }
    None
}

/// `\s+for\s+` at `j`, the last `\s+` matching the line's `\n`.
fn for_at(b: &[u8], j: usize) -> bool {
    if !b.get(j).is_some_and(|&c| is_space(c)) {
        return false;
    }
    let k = skip_spaces(b, j);
    b[k..].starts_with(b"for") && b.get(k + 3).is_none_or(|&c| is_space(c))
}

/// impl-for's look-ahead at `p`: `(?=\s*(?:::|name|\$|<)+(<.*?>)?\s+for\s+)`, whether the impl
/// names a trait before `for`.
fn impl_for_ahead(b: &[u8], p: usize) -> bool {
    let mut i = skip_spaces(b, p);
    let (mut tokens, mut angle) = (0, None);
    loop {
        if b[i..].starts_with(b"::") {
            i += 2;
        } else if let Some(e) = ident_end(b, i) {
            i = e;
        } else if b.get(i) == Some(&b'$') {
            i += 1;
        } else if b.get(i) == Some(&b'<') {
            if tokens > 0 && angle.is_none() {
                angle = Some(i);
            }
            i += 1;
        } else {
            break;
        }
        tokens += 1;
    }
    if tokens == 0 {
        return false;
    }
    if for_at(b, i) {
        return true;
    }
    // a prefix of the tokens, then `<` to any `>` the rest follows
    angle.is_some_and(|a| (a + 1..b.len()).any(|j| b[j] == b'>' && for_at(b, j + 1)))
}

/// What text no pattern matches is in a context.
fn text_of(c: u8) -> u16 {
    match c {
        STRING | FMT_STRING | BYTE_STRING => STR,
        RAW_STRING | FMT_RAW | RAW_BYTES => RAW,
        BLOCK_COMMENT => BLOCK_TEXT,
        DOC_COMMENT => DOC_BLOCK,
        _ => PLAIN,
    }
}

/// `(:)\s*(\()` at `p`: where its `(` is.
fn colon_paren(b: &[u8], p: usize) -> Option<usize> {
    let s = skip_spaces(b, p + 1);
    (b[p] == b':' && b.get(s) == Some(&b'(')).then_some(s)
}

// --- frames ---------------------------------------------------------------------------------

/// The top level.
const F_MAIN: u8 = 0;
/// `{ … }`: a block, a function's, an impl's, a trait's, a module's or an enum's body, a struct
/// literal.
const F_BLOCK: u8 = 1;
/// `( … )`.
const F_GROUP: u8 = 2;
/// `[ … ]`, where `;` separates.
const F_BRACKET: u8 = 3;
/// The last frame whose turns are the plain loop's: frames above it have turns of their own
/// first ([`Scan::frame_turn`]).
const F_PLAIN_LAST: u8 = F_BRACKET;
/// A function's parameters: a name before `:` is a parameter, and `self`.
const F_PARAMS: u8 = 4;
/// A closure's parameters: names are parameters, `:` starts a type.
const F_CLOSURE: u8 = 5;
/// A closure parameter's type, to its `,` or the closing `|`.
const F_CLOSURE_TYPE: u8 = 6;
/// A generic's `< … >`.
const F_GENERIC: u8 = 7;
/// A generic after a function's name: its `>` leaves the name's place, where `(` opens the
/// parameters.
const F_GENERIC_FN: u8 = 8;
/// A struct's body: a name before `:` is a field.
const F_STRUCT: u8 = 9;
/// A struct's head, from `struct` to its body (`{`), its fields (`(`) or `;`.
const F_STRUCT_HEAD: u8 = 10;
/// A tuple struct's fields, `( … )`, in its head.
const F_TUPLE: u8 = 11;
/// An impl's head before its name: the name is the impl's, or a trait's if `for` follows.
const F_IMPL: u8 = 12;
/// An impl's trait, to `for`.
const F_IMPL_TRAIT: u8 = 13;
/// After an impl's `for`: the next name is the impl's.
const F_IMPL_FOR: u8 = 14;
/// An impl's head past its name, to its `{`.
const F_IMPL_NAMED: u8 = 15;
/// `write!(`'s first argument, to its `,`: the format string comes next.
const F_WRITE: u8 = 16;
/// A SQL call's `( … )`: [`F_GROUP`], plus where its SQL argument is (a `PSQL_*` state added).
const F_PSQL: u8 = 17;
const F_PSQL_LAST: u8 = F_PSQL + PSQL_SKIP;
// the type frames, where a name is read as bat reads one in a type (`type-any-identifier`): a
// primitive, `ref`/`mut`/`const`, `fn(`, a `CamelCase::` path, a name before `<`; else plain,
// a `support.type` word too
/// A parameter's type, to its `,` or the parameters' `)`.
const F_PARAM_TYPE: u8 = 20;
/// A field's type, to its `,` (plain) or the body's `}`.
const F_FIELD_TYPE: u8 = 21;
/// A function's or a closure's return type, to a byte no type holds (`{`, `;`, `where`).
const F_RET_TYPE: u8 = 22;
/// A type's `( … )` (a tuple's, `fn(…)`'s, `Fn(…)`'s).
const F_TYPE_GROUP: u8 = 23;
/// A generic's array, `[T; N]`.
const F_GENERIC_ARRAY: u8 = 24;
/// A `where` clause, to `{` or `;`.
const F_WHERE: u8 = 25;
/// A trait's or an enum's head, from `trait` or `enum` to its body: its generics and supertraits
/// plain, as bat's (`statements-block`).
const F_HEAD: u8 = 26;
/// A struct's head past a `where`, and the body after it, where bat reads a field's `,` as a
/// separator.
const F_STRUCT_HEAD_W: u8 = 27;
const F_STRUCT_W: u8 = 28;
const F_FIELD_TYPE_W: u8 = 29;
/// `( … )` among parameters (a tuple's pattern): names are parameters, the rest plain.
const F_PARAM_TUPLE: u8 = 30;
/// `: ( … )` among parameters, a tuple's type.
const F_PARAM_TUPLE_TYPE: u8 = 31;
// `macro_rules!`'s frames (X-064's, kept from the C family's frames): the block after its name, its rules, a
// rule's matcher (in any of the three brackets), a repetition and a plain group in it, the `=>`,
// the body (code), the rule's `;`
const F_MACRO_BLOCK: u8 = 32;
const F_MACRO_RULES: u8 = 33;
const F_MACRO_RULES_BRACKET: u8 = 34;
const F_MACRO_RULES_PAREN: u8 = 35;
const F_MATCHER: u8 = 36;
const F_MATCHER_SQUARE: u8 = 37;
const F_MATCHER_CURLY: u8 = 38;
const F_MATCHER_GROUP: u8 = 39;
const F_NEST_PAREN: u8 = 40;
const F_NEST_SQUARE: u8 = 41;
const F_NEST_CURLY: u8 = 42;
const F_MATCH_OP: u8 = 43;
const F_MACRO_BODY: u8 = 44;
const F_MACRO_TERM: u8 = 45;
const _: () = assert!(F_MACRO_TERM < ATTRIBUTE);
/// A matcher's frame's closer, by its id less [`F_MATCHER`].
const CLOSERS: [u8; 7] = *b")]}))]}";
const _: () = assert!(F_NEST_CURLY - F_MATCHER == 6 && F_MATCHER_GROUP - F_MATCHER == 3);
// the attributes', strings' and comments' frames are the text frames' ids, above: `ATTRIBUTE`,
// `ATTR_PARAMS`, `ATTR_CALL`, `STRING` … `DOC_COMMENT`, `SQL_IN`, `REGEX`

// --- the previous token ---------------------------------------------------------------------

/// A statement's or an item's start: after `;`, `{`, `}`, an attribute.
const P_START: u8 = 0;
/// An expression's start: after an operator, `[`, `&`, `move`, a keyword. A `|` here opens a
/// closure: it has no left operand.
const P_OP: u8 = 1;
/// After a value: a name, a literal, a closer, `?`, `>` (see [`Scan::greater`]).
const P_VALUE: u8 = 2;
const P_FN: u8 = 3;
/// A function's name: its `<` opens a generic, its `(` the parameters.
const P_FN_NAME: u8 = 4;
const P_STRUCT: u8 = 5;
const P_ENUM: u8 = 6;
const P_TRAIT: u8 = 7;
const P_TYPE: u8 = 8;
const P_MOD: u8 = 9;
const P_MACRO: u8 = 10;
/// A type's or an item's name: its `<` opens a generic.
const P_DEF_NAME: u8 = 11;
/// After `::`.
const P_PATH: u8 = 12;
/// After `:` or `->`: a type.
const P_COLON: u8 = 13;
/// A format macro's first argument: a string there is a format string.
const P_FMT: u8 = 14;
/// After `let` (and `mut` after it).
const P_LET: u8 = 15;
/// After a function's parameters' or a closure's `)`/`|`: `->` opens a return type.
const P_PARAMS_END: u8 = 16;
/// After `=`, `=>`, `,`, `;`, `return` (bat's `after-operator`): `|` opens a closure, `<` a
/// generic.
const P_SEP: u8 = 17;
/// After `(` or `{` (bat's `try-closure`): `|` opens a closure.
const P_OPEN: u8 = 18;
/// A function's parameter's name: its `:` is a separator.
const P_PARAM: u8 = 19;
/// Leave a keyword's `prev` at [`P_START`] if it was there, else [`P_OP`] (`pub`, `unsafe`,
/// `extern`: an item goes on).
const P_KEEP: u8 = 0xff;

// --- words ----------------------------------------------------------------------------------

const G_KEYWORD: u8 = 1;
const G_PRIMITIVE: u8 = 2;
const G_SUPPORT: u8 = 3;
/// A word asked for by its id.
const G_ASKED: u8 = 4;
/// A call SQL's may be ([`sql_call`]).
const G_SQL: u8 = 5;

const N: usize = 139;

/// Every word the scan tells apart: its group, its leaf, and the previous-token byte it leaves.
/// The item words first, so that their ids are small.
const LIST: [(&[u8], u8, u16, u8); N] = [
    (b"pub", G_KEYWORD, MODIFIER, P_KEEP),
    (b"mod", G_KEYWORD, MODULE, P_MOD),
    (b"fn", G_KEYWORD, FN, P_FN),
    (b"struct", G_KEYWORD, STRUCT, P_STRUCT),
    (b"type", G_KEYWORD, TYPE, P_TYPE),
    (b"trait", G_KEYWORD, TRAIT, P_TRAIT),
    (b"enum", G_KEYWORD, ENUM, P_ENUM),
    (b"impl", G_KEYWORD, IMPL, P_OP),
    (b"return", G_KEYWORD, CONTROL, P_SEP),
    (b"macro_rules", G_ASKED, 0, P_VALUE),
    (b"b", G_ASKED, 0, P_VALUE),
    (b"br", G_ASKED, 0, P_VALUE),
    (b"r", G_ASKED, 0, P_VALUE),
    (b"self", G_ASKED, 0, P_VALUE),
    (b"super", G_ASKED, 0, P_VALUE),
    (b"format", G_ASKED, 0, P_VALUE),
    (b"print", G_ASKED, 0, P_VALUE),
    (b"println", G_ASKED, 0, P_VALUE),
    (b"eprint", G_ASKED, 0, P_VALUE),
    (b"eprintln", G_ASKED, 0, P_VALUE),
    (b"write", G_ASKED, 0, P_VALUE),
    (b"writeln", G_ASKED, 0, P_VALUE),
    (b"as", G_KEYWORD, OPERATOR, P_COLON),
    (b"alignof", G_KEYWORD, INVALID, P_OP),
    (b"abstract", G_KEYWORD, INVALID, P_OP),
    (b"break", G_KEYWORD, CONTROL, P_OP),
    (b"box", G_KEYWORD, OPERATOR, P_OP),
    (b"become", G_KEYWORD, INVALID, P_OP),
    (b"const", G_KEYWORD, STORAGE, P_KEEP),
    (b"crate", G_KEYWORD, KEYWORD, P_VALUE),
    (b"continue", G_KEYWORD, CONTROL, P_OP),
    (b"do", G_KEYWORD, INVALID, P_OP),
    (b"else", G_KEYWORD, CONTROL, P_OP),
    (b"extern", G_KEYWORD, KEYWORD, P_KEEP),
    (b"for", G_KEYWORD, CONTROL, P_OP),
    (b"false", G_KEYWORD, LANGUAGE, P_VALUE),
    (b"final", G_KEYWORD, INVALID, P_OP),
    (b"if", G_KEYWORD, CONTROL, P_OP),
    (b"in", G_KEYWORD, OPERATOR, P_OP),
    (b"let", G_KEYWORD, STORAGE, P_LET),
    (b"loop", G_KEYWORD, CONTROL, P_OP),
    (b"mut", G_KEYWORD, MODIFIER, 0xfe),
    (b"move", G_KEYWORD, MODIFIER, P_OP),
    (b"match", G_KEYWORD, CONTROL, P_OP),
    (b"macro", G_KEYWORD, INVALID, P_OP),
    (b"offsetof", G_KEYWORD, INVALID, P_OP),
    (b"override", G_KEYWORD, INVALID, P_OP),
    (b"priv", G_KEYWORD, INVALID, P_OP),
    (b"proc", G_KEYWORD, INVALID, P_OP),
    (b"pure", G_KEYWORD, INVALID, P_OP),
    (b"ref", G_KEYWORD, MODIFIER, 0xfe),
    (b"static", G_KEYWORD, STORAGE, P_KEEP),
    (b"sizeof", G_KEYWORD, INVALID, P_OP),
    (b"true", G_KEYWORD, LANGUAGE, P_VALUE),
    (b"typeof", G_KEYWORD, INVALID, P_OP),
    (b"use", G_KEYWORD, KEYWORD, P_OP),
    (b"unsafe", G_KEYWORD, MODIFIER, P_KEEP),
    (b"unsized", G_KEYWORD, INVALID, P_OP),
    (b"virtual", G_KEYWORD, INVALID, P_OP),
    (b"where", G_KEYWORD, KEYWORD, P_OP),
    (b"while", G_KEYWORD, CONTROL, P_OP),
    (b"yield", G_KEYWORD, INVALID, P_OP),
    (b"bool", G_PRIMITIVE, STORAGE, P_VALUE),
    (b"char", G_PRIMITIVE, STORAGE, P_VALUE),
    (b"f32", G_PRIMITIVE, STORAGE, P_VALUE),
    (b"f64", G_PRIMITIVE, STORAGE, P_VALUE),
    (b"i8", G_PRIMITIVE, STORAGE, P_VALUE),
    (b"i16", G_PRIMITIVE, STORAGE, P_VALUE),
    (b"i32", G_PRIMITIVE, STORAGE, P_VALUE),
    (b"i64", G_PRIMITIVE, STORAGE, P_VALUE),
    (b"i128", G_PRIMITIVE, STORAGE, P_VALUE),
    (b"isize", G_PRIMITIVE, STORAGE, P_VALUE),
    (b"str", G_PRIMITIVE, STORAGE, P_VALUE),
    (b"u8", G_PRIMITIVE, STORAGE, P_VALUE),
    (b"u16", G_PRIMITIVE, STORAGE, P_VALUE),
    (b"u32", G_PRIMITIVE, STORAGE, P_VALUE),
    (b"u64", G_PRIMITIVE, STORAGE, P_VALUE),
    (b"u128", G_PRIMITIVE, STORAGE, P_VALUE),
    (b"usize", G_PRIMITIVE, STORAGE, P_VALUE),
    (b"Self", G_PRIMITIVE, STORAGE, P_VALUE),
    (b"AsRef", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"AsMut", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"Box", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"Copy", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"Clone", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"Drop", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"Default", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"DoubleEndedIterator", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"Eq", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"Extend", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"ExactSizeIterator", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"Err", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"Fn", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"FnMut", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"FnOnce", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"From", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"Into", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"Iterator", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"IntoIterator", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"None", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"Ord", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"Option", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"Ok", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"PartialEq", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"PartialOrd", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"Result", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"Send", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"Sized", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"Sync", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"Some", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"SliceConcatExt", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"String", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"ToOwned", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"ToString", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"Vec", G_SUPPORT, SUPPORT_TYPE, P_VALUE),
    (b"query", G_SQL, 0, P_VALUE),
    (b"query_as", G_SQL, 0, P_VALUE),
    (b"query_scalar", G_SQL, 0, P_VALUE),
    (b"query_with", G_SQL, 0, P_VALUE),
    (b"query_as_with", G_SQL, 0, P_VALUE),
    (b"query_scalar_with", G_SQL, 0, P_VALUE),
    (b"query_row", G_SQL, 0, P_VALUE),
    (b"query_one", G_SQL, 0, P_VALUE),
    (b"query_opt", G_SQL, 0, P_VALUE),
    (b"query_raw", G_SQL, 0, P_VALUE),
    (b"query_typed", G_SQL, 0, P_VALUE),
    (b"query_unchecked", G_SQL, 0, P_VALUE),
    (b"query_scalar_unchecked", G_SQL, 0, P_VALUE),
    (b"query_as_unchecked", G_SQL, 0, P_VALUE),
    (b"sql_query", G_SQL, 0, P_VALUE),
    (b"sql", G_SQL, 0, P_VALUE),
    (b"simple_query", G_SQL, 0, P_VALUE),
    (b"push_sql", G_SQL, 0, P_VALUE),
    (b"prepare", G_SQL, 0, P_VALUE),
    (b"prepare_cached", G_SQL, 0, P_VALUE),
    (b"prepare_typed", G_SQL, 0, P_VALUE),
    (b"execute", G_SQL, 0, P_VALUE),
    (b"execute_batch", G_SQL, 0, P_VALUE),
    (b"batch_execute", G_SQL, 0, P_VALUE),
];

/// `mut` and `ref`: the previous token stays (`let mut c = |…|`, `&mut self`).
const P_SAME: u8 = 0xfe;

const W_FN: u8 = id_of(b"fn");
const W_STRUCT: u8 = id_of(b"struct");
const W_TRAIT: u8 = id_of(b"trait");
const W_ENUM: u8 = id_of(b"enum");
const W_PUB: u8 = id_of(b"pub");
const W_IMPL: u8 = id_of(b"impl");
const W_MACRO_RULES: u8 = id_of(b"macro_rules");
const W_SELF: u8 = id_of(b"self");
const W_SUPER: u8 = id_of(b"super");
const W_FORMAT: u8 = id_of(b"format");
const W_EPRINTLN: u8 = id_of(b"eprintln");
const W_WRITE: u8 = id_of(b"write");
const W_WRITELN: u8 = id_of(b"writeln");
const W_FOR: u8 = id_of(b"for");
const W_REF: u8 = id_of(b"ref");
const W_MUT: u8 = id_of(b"mut");
const W_CONST: u8 = id_of(b"const");
const W_AS: u8 = id_of(b"as");
const W_WHERE: u8 = id_of(b"where");
const _: () = assert!(W_FORMAT + 4 == W_EPRINTLN && W_WRITE + 1 == W_WRITELN);

const fn id_of(w: &[u8]) -> u8 {
    let mut i = 0;
    while i < N {
        let x = LIST[i].0;
        if x.len() == w.len() {
            let mut k = 0;
            while k < w.len() && x[k] == w[k] {
                k += 1;
            }
            if k == w.len() {
                return i as u8 + 1;
            }
        }
        i += 1;
    }
    panic!("not listed")
}

const WORD_LIST: [&[u8]; N] = {
    let mut w: [&[u8]; N] = [b""; N];
    let mut i = 0;
    while i < N {
        w[i] = LIST[i].0;
        i += 1;
    }
    w
};
static WORDS: Words<512, N, { text_len(&WORD_LIST) }> = Words::new(&WORD_LIST);

/// Each id's group, leaf and previous-token byte (index 0 for none).
static INFO: [(u8, u16, u8); N + 1] = {
    let mut t = [(0, 0, P_VALUE); N + 1];
    let mut i = 0;
    while i < N {
        t[i + 1] = (LIST[i].1, LIST[i].2, LIST[i].3);
        i += 1;
    }
    t
};

// --- bytes ----------------------------------------------------------------------------------

const A_OTHER: u8 = 0;
const A_BLANK: u8 = 1;
const A_NAME: u8 = 2;
const A_DIGIT: u8 = 3;
const A_QUOTE: u8 = 4;
const A_TICK: u8 = 5;
const A_SLASH: u8 = 6;
const A_PAREN: u8 = 7;
const A_CLOSE_PAREN: u8 = 8;
const A_BRACKET: u8 = 9;
const A_CLOSE_BRACKET: u8 = 10;
const A_CURLY: u8 = 11;
const A_CLOSE_CURLY: u8 = 12;
const A_DOT: u8 = 13;
const A_COLON: u8 = 14;
const A_SEMI: u8 = 15;
const A_COMMA: u8 = 16;
const A_EQ: u8 = 17;
const A_LT: u8 = 18;
const A_GT: u8 = 19;
const A_MINUS: u8 = 20;
const A_ARITH: u8 = 21;
const A_STAR: u8 = 22;
const A_AMP: u8 = 23;
const A_PIPE: u8 = 24;
const A_BANG: u8 = 25;
const A_HASH: u8 = 26;
const A_DOLLAR: u8 = 27;
const A_OP: u8 = 28;
const A_CARET: u8 = 29;

static ACT: [u8; 256] = {
    let mut t = [A_OTHER; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        t[c] = match b {
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => A_BLANK,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | 0x80..=0xff => A_NAME,
            b'0'..=b'9' => A_DIGIT,
            b'"' => A_QUOTE,
            b'\'' => A_TICK,
            b'/' => A_SLASH,
            b'(' => A_PAREN,
            b')' => A_CLOSE_PAREN,
            b'[' => A_BRACKET,
            b']' => A_CLOSE_BRACKET,
            b'{' => A_CURLY,
            b'}' => A_CLOSE_CURLY,
            b'.' => A_DOT,
            b':' => A_COLON,
            b';' => A_SEMI,
            b',' => A_COMMA,
            b'=' => A_EQ,
            b'<' => A_LT,
            b'>' => A_GT,
            b'-' => A_MINUS,
            b'+' | b'%' => A_ARITH,
            b'^' => A_CARET,
            b'*' => A_STAR,
            b'&' => A_AMP,
            b'|' => A_PIPE,
            b'!' => A_BANG,
            b'#' => A_HASH,
            b'$' => A_DOLLAR,
            b'?' | b'@' | b'~' => A_OP,
            _ => A_OTHER,
        };
        c += 1;
    }
    t
};

/// A name's bytes: `\w`, and every byte past ASCII.
static NAME: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        t[c] = matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_') || b >= 0x80;
        c += 1;
    }
    t
};

/// A name's bytes' classes, OR-ed as it is read (0 for a byte no name holds): a capital, a
/// lowercase letter, a byte past ASCII; so a constant's and a call's shape is told without
/// reading the name again.
const C_NAME: u8 = 1;
const C_UP: u8 = 2;
const C_LO: u8 = 4;
const C_HI: u8 = 8;
static CLS: [u8; 256] = {
    let mut t = [0; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        t[c] = match b {
            b'A'..=b'Z' => C_NAME | C_UP,
            b'a'..=b'z' => C_NAME | C_LO,
            b'0'..=b'9' | b'_' => C_NAME,
            0x80..=0xff => C_NAME | C_HI,
            _ => 0,
        };
        c += 1;
    }
    t
};

/// `[a-z_][a-z0-9_]*`, a name whose first byte is `first` and whose classes are `bits`: a
/// call's or a macro's name as bat's patterns take it.
#[inline(always)]
fn lower(first: u8, bits: u8) -> bool {
    matches!(first, b'a'..=b'z' | b'_') && bits & (C_UP | C_HI) == 0
}

/// The bytes a type holds, which a generic's `< … >` reads through: a name's, blanks and the
/// punctuation of paths, bounds, references, tuples, arrays, lifetimes and `fn(…) -> …`. Any
/// other ends the generic (`if a<b {`: bat's frame never ends there).
static IN_TYPE: [bool; 256] = {
    let mut t = NAME;
    let s = b" \t,:<>'&*[]()-+?!=_;#$/";
    let mut i = 0;
    while i < s.len() {
        t[s[i] as usize] = true;
        i += 1;
    }
    t
};

/// [`State::arg`]'s bits: an open raw string's `#`s, the previous token, an attribute open on
/// the stack, a SQL marker waiting for its string.
const ARG_HASHES: u16 = 0x00ff;
const ARG_PREV: u16 = 0x1f00;
const ARG_PREV_SHIFT: u16 = 8;
const ARG_OPEN: u16 = 0x4000;
const ARG_MARKED: u16 = 0x8000;
const _: () = assert!(P_PARAM < 0x20);

// --- the scan -------------------------------------------------------------------------------

struct Scan<'b, 'c, K: Sink> {
    b: &'b [u8],
    stack: [u8; MAX],
    depth: usize,
    /// The top frame, `stack[depth - 1]`.
    cur: u8,
    /// The previous significant token ([`P_START`] …).
    prev: u8,
    /// The open raw string's `#`s.
    hashes: usize,
    /// How many attributes are open.
    annot: usize,
    /// The kind of a text frame's text (0 in code).
    text: u16,
    /// Where the spans so far end, for a text frame's fill; in code it lags (a code token
    /// doesn't move it), and a text frame's push moves it to its place first.
    at: usize,
    /// Where the `(` of a SQL call is ([`sql_call`]), and the state its arguments start in;
    /// `usize::MAX` for none on the line.
    sql_paren: usize,
    sql_call: u8,
    /// What SQL's scan keeps between the host's turns on a line.
    sql: sql::Embedded,
    /// Where the SQL string open on the line ends; `usize::MAX` till it is looked for.
    sql_end: usize,
    /// Whether SQL's scan took the line's end.
    sql_nl: bool,
    /// A SQL marker's place ([`sql::marker_holds`]); `usize::MAX` for none.
    mark: usize,
    k: K,
    out: &'b mut Out<'c>,
}

impl<'b, 'c, K: Sink> Scan<'b, 'c, K> {
    /// A scan of `b`, returned as a literal so that it is built in place; the state is
    /// [`Scan::load`]'s.
    #[inline(always)]
    fn fresh(b: &'b [u8], out: &'b mut Out<'c>, k: K) -> Self {
        Scan {
            b,
            stack: [F_MAIN; MAX],
            depth: 1,
            cur: F_MAIN,
            prev: P_START,
            hashes: 0,
            annot: 0,
            text: 0,
            at: 0,
            sql_paren: usize::MAX,
            sql_call: PSQL_SQL,
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

    /// The stack and the bits beside it from `state`, in place.
    #[inline(always)]
    fn load(&mut self, state: &State) {
        self.depth = state.load(&mut self.stack);
        self.cur = self.stack[self.depth - 1];
        let arg = state.flags();
        self.hashes = usize::from(arg & ARG_HASHES);
        self.prev = ((arg & ARG_PREV) >> ARG_PREV_SHIFT) as u8;
        if arg & ARG_MARKED != 0 {
            self.mark = 0;
        }
        if arg & ARG_OPEN != 0 {
            self.count_open();
        }
    }

    /// The state the scan's last line left.
    #[inline(always)]
    fn keep(&self) -> State {
        let mut arg = u16::from(self.prev) << ARG_PREV_SHIFT;
        if matches!(self.string_top(), RAW_STRING | FMT_RAW | RAW_BYTES) {
            arg |= self.hashes.min(255) as u16;
        }
        if self.mark != usize::MAX {
            arg |= ARG_MARKED;
        }
        if self.annot != 0 {
            arg |= ARG_OPEN;
        }
        State::keep(&self.stack, self.depth, arg)
    }

    #[cold]
    #[inline(never)]
    fn count_open(&mut self) {
        self.annot = self.stack[1..self.depth]
            .iter()
            .filter(|&&l| l == ATTRIBUTE)
            .count();
    }

    /// The line lexed from the stack it starts with, ended as a state ends it (past the levels
    /// a state holds, the bottom ones dropped), a SQL marker that still holds kept.
    #[inline(always)]
    fn lex_line(&mut self) {
        let b = self.b;
        self.refresh();
        if self.top() >= SQL_IN {
            self.text = self.string_text();
        }
        self.run();
        self.fill(b.len());
        if self.mark != usize::MAX {
            self.mark = if sql::marker_holds(b, self.mark, b.len()) {
                0
            } else {
                usize::MAX
            };
        }
        let n = self.depth - 1;
        if n > crate::stack::HELD {
            let from = n - crate::stack::HELD;
            self.stack.copy_within(1 + from..self.depth, 1);
            self.depth -= from;
            self.cur = self.stack[self.depth - 1];
            self.count();
        }
    }

    /// A block's next line: what a line starts with reset, the rest kept.
    #[inline(always)]
    fn lex_next(&mut self) {
        self.at = 0;
        self.sql_paren = usize::MAX;
        self.sql_call = PSQL_SQL;
        self.sql = sql::Embedded {
            is_end: usize::MAX,
            at: 0,
        };
        self.sql_end = usize::MAX;
        self.sql_nl = false;
        self.lex_line();
    }

    // --- the stack ---

    #[inline(always)]
    fn top(&self) -> u8 {
        self.cur
    }

    #[inline(always)]
    fn set_top(&mut self, c: u8) {
        self.stack[self.depth - 1] = c;
        self.cur = c;
    }

    /// A code frame pushed: no text to fill, nothing to count.
    #[inline(always)]
    fn push_code(&mut self, c: u8) {
        if self.depth == MAX {
            self.full();
        }
        self.stack[self.depth] = c;
        self.depth += 1;
        self.cur = c;
    }

    #[cold]
    #[inline(never)]
    fn full(&mut self) {
        crate::stack::drop_bottom(&mut self.stack, &mut self.depth);
        self.count();
    }

    #[inline(always)]
    fn pop_code(&mut self) {
        if self.depth > 1 {
            self.depth -= 1;
            self.cur = self.stack[self.depth - 1];
        }
    }

    /// Bytes `s..e` are `leaf`, in code (no text to fill, no attribute around).
    #[inline(always)]
    fn tok(&mut self, s: usize, e: usize, leaf: u16) -> usize {
        self.k.merge(&mut self.out, s, e, leaf);
        e
    }

    // --- the text frames' writing (X-064's) ---

    fn kind(&self, leaf: u16) -> u16 {
        if self.annot > 0 { NLEAVES + leaf } else { leaf }
    }

    /// Bytes `s..e` are `leaf`, after the text before them.
    fn put(&mut self, s: usize, e: usize, leaf: u16) {
        self.fill(s);
        let kind = self.kind(leaf);
        if kind != 0 {
            self.k.merge(&mut self.out, s, e, kind);
        }
        self.at = e;
    }

    fn token(&mut self, p: usize, n: usize, leaf: u16) -> usize {
        self.put(p, p + n, leaf);
        p + n
    }

    fn close(&mut self, p: usize, leaf: u16) -> usize {
        self.close_n(p, 1, leaf)
    }

    fn close_n(&mut self, p: usize, n: usize, leaf: u16) -> usize {
        let e = self.token(p, n, leaf);
        self.pop(e);
        e
    }

    /// The text up to `p` is the frame's.
    fn fill(&mut self, p: usize) {
        if p > self.at {
            if self.text != 0 {
                self.k.merge(&mut self.out, self.at, p, self.text);
            }
            self.at = p;
        }
    }

    fn refresh(&mut self) {
        self.text = self.kind(text_of(self.top()));
    }

    #[cold]
    #[inline(never)]
    fn string_text(&self) -> u16 {
        self.kind(text_of(self.string_top()))
    }

    /// The top frame, or under a regex or SQL the string it is in.
    fn string_top(&self) -> u8 {
        let top = self.top();
        if top >= SQL_IN && self.depth >= 2 {
            self.stack[self.depth - 2]
        } else {
            top
        }
    }

    #[cold]
    #[inline(never)]
    fn count(&mut self) {
        let open = &self.stack[..self.depth];
        self.annot = open.iter().filter(|&&c| c == ATTRIBUTE).count();
        self.refresh();
    }

    /// A frame pushed at `at`, the text before it filled.
    fn push(&mut self, at: usize, c: u8) {
        self.fill(at);
        if self.depth == MAX {
            crate::stack::drop_bottom(&mut self.stack, &mut self.depth);
            self.count();
        }
        self.stack[self.depth] = c;
        self.depth += 1;
        self.cur = c;
        self.annot += usize::from(c == ATTRIBUTE);
        self.refresh();
    }

    fn pop(&mut self, at: usize) {
        self.fill(at);
        if self.depth > 1 {
            self.depth -= 1;
            self.annot -= usize::from(self.cur == ATTRIBUTE);
            self.cur = self.stack[self.depth - 1];
        }
        self.refresh();
    }

    // --- the line ---

    /// The line from the start: the code loop, and where a regex or SQL in a string is on top,
    /// its turns till the string is again.
    fn run(&mut self) {
        let mut p = if self.b.first() == Some(&b' ') && self.top() < ATTRIBUTE {
            skip_indent(self.b)
        } else {
            0
        };
        loop {
            p = self.code_run(p);
            if self.top() < SQL_IN {
                return;
            }
            loop {
                let top = self.top();
                if top < SQL_IN {
                    break;
                }
                if p == self.b.len() {
                    if top < REGEX {
                        self.sql_eol(top);
                    }
                    return;
                }
                p = match self.embed_turn(p, top) {
                    Some(q) => q,
                    None => {
                        let b = self.b;
                        let c = b[p];
                        if is_space(c) {
                            skip_spaces(b, p)
                        } else if is_word(c) {
                            word_end(b, p)
                        } else {
                            p + 1
                        }
                    }
                };
            }
        }
    }

    /// The frames' loop from `p`: to the line's end, or a regex or SQL on top.
    fn code_run(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        loop {
            let top = self.top();
            if top >= ATTRIBUTE {
                if top >= SQL_IN {
                    return p;
                }
                if p >= n {
                    return n;
                }
                p = self.text_turn(p, top);
                continue;
            }
            if p >= n {
                return n;
            }
            let mut c = b[p];
            let mut act = ACT[usize::from(c)];
            if act == A_BLANK {
                p += 1;
                while p < n && ACT[usize::from(b[p])] == A_BLANK {
                    p += 1;
                }
                if p >= n {
                    return n;
                }
                c = b[p];
                act = ACT[usize::from(c)];
            }
            if top > F_PLAIN_LAST
                && let Some(q) = self.frame_turn(p, c, act, top)
            {
                p = q;
                continue;
            }
            p = match act {
                A_NAME => self.name(p, top),
                A_PAREN => self.open_paren(p, top),
                A_CLOSE_PAREN => self.close_paren(p, top),
                A_DOT => self.dot(p),
                A_COLON => self.colon(p),
                A_SEMI => self.semi(p, top),
                A_COMMA => self.comma(p, top),
                A_EQ => self.equals(p),
                A_CURLY => self.open_curly(p, top),
                A_CLOSE_CURLY => self.close_curly(p, top),
                A_BRACKET => {
                    self.push_code(F_BRACKET);
                    self.prev = P_OP;
                    self.tok(p, p + 1, GROUP_BEGIN)
                }
                A_CLOSE_BRACKET => {
                    self.prev = P_VALUE;
                    if top == F_BRACKET {
                        self.pop_code();
                        self.tok(p, p + 1, GROUP_END)
                    } else {
                        p + 1
                    }
                }
                A_QUOTE => self.dquote(p),
                A_DIGIT => self.digits(p),
                A_TICK => {
                    self.prev = P_VALUE;
                    self.quote(p)
                }
                A_SLASH => self.slash(p),
                A_LT => self.less(p),
                A_GT => self.greater(p),
                A_MINUS => self.minus(p),
                A_ARITH => self.op_eq(p, ARITHMETIC),
                A_CARET => self.op_eq(p, BITWISE),
                A_STAR => self.star(p),
                A_AMP => self.amp(p),
                A_PIPE => self.pipe(p),
                A_BANG => self.bang(p),
                A_HASH => self.hash(p),
                A_DOLLAR => self.dollar(p),
                A_OP => {
                    // `?` ends a value (`x? | y`); `@` and `~` start an expression
                    self.prev = if c == b'?' { P_VALUE } else { P_OP };
                    self.tok(p, p + 1, OPERATOR)
                }
                _ => p + 1,
            };
        }
    }

    /// A frame's own turn at `p` (not a blank, not the line's end): `None` where the token is
    /// the plain loop's.
    #[inline(always)]
    fn frame_turn(&mut self, p: usize, c: u8, act: u8, top: u8) -> Option<usize> {
        let b = self.b;
        match top {
            F_GENERIC | F_GENERIC_FN | F_GENERIC_ARRAY => self.generic_turn(p, c, act, top),
            F_PARAMS => match c {
                b'(' => Some(self.open_param_tuple(p)),
                b':' if let Some(o) = colon_paren(b, p) => {
                    Some(self.param_tuple_type(p, o, F_PARAM_TYPE))
                }
                b')' => {
                    self.pop_code();
                    self.prev = P_PARAMS_END;
                    Some(self.tok(p, p + 1, FN_PARAMS_END))
                }
                b':' if b.get(p + 1) != Some(&b':') => {
                    let sep = self.prev == P_PARAM || colon_paren(b, p).is_some();
                    self.set_top(F_PARAM_TYPE);
                    self.prev = P_COLON;
                    if sep {
                        return Some(self.tok(p, p + 1, SEPARATOR));
                    }
                    Some(p + 1)
                }
                b'#' => Some(p + 1),
                // a parameter's `,` plain, as bat's
                b',' => {
                    self.prev = P_OP;
                    Some(p + 1)
                }
                _ if act == A_NAME => {
                    let e = name_end(b, p);
                    self.prev = P_VALUE;
                    if e - p == 4 && has(b, p, b"self") {
                        self.prev = P_PARAM;
                        return Some(self.tok(p, e, PARAMETER));
                    }
                    // `name\s*:(?!:)`
                    let s = if matches!(b.get(e), Some(b' ' | b'\t')) {
                        skip_spaces(b, e)
                    } else {
                        e
                    };
                    if b.get(s) == Some(&b':') && b.get(s + 1) != Some(&b':') && is_ident(&b[p..e])
                    {
                        self.prev = P_PARAM;
                        return Some(self.tok(p, e, PARAMETER));
                    }
                    Some(self.type_name(p, e, false, top))
                }
                _ => self.type_turn(p, c, act, false, false),
            },
            F_PARAM_TUPLE => match c {
                b')' => {
                    self.pop_code();
                    self.prev = P_VALUE;
                    Some(self.tok(p, p + 1, META_GROUP_END))
                }
                b',' => Some(self.tok(p, p + 1, SEPARATOR)),
                _ => {
                    if let Some(e) = ident_end(b, p) {
                        return Some(self.tok(p, e, PARAMETER));
                    }
                    Some(if is_word(c) { word_end(b, p) } else { p + 1 })
                }
            },
            F_PARAM_TUPLE_TYPE => match c {
                b')' => {
                    self.pop_code();
                    self.prev = P_VALUE;
                    Some(self.tok(p, p + 1, META_GROUP_END))
                }
                b',' => Some(self.tok(p, p + 1, SEPARATOR)),
                _ => self.type_turn(p, c, act, false, false),
            },
            F_PARAM_TYPE => match c {
                b',' => {
                    self.set_top(F_PARAMS);
                    self.prev = P_OP;
                    Some(p + 1)
                }
                b')' => {
                    self.pop_code();
                    self.prev = P_PARAMS_END;
                    Some(self.tok(p, p + 1, FN_PARAMS_END))
                }
                _ => self.type_turn(p, c, act, false, false),
            },
            F_CLOSURE | F_CLOSURE_TYPE => match c {
                b'|' => {
                    self.pop_code();
                    self.prev = P_PARAMS_END;
                    Some(self.tok(p, p + 1, PARAMS_END))
                }
                b',' => {
                    self.set_top(F_CLOSURE);
                    self.prev = P_OP;
                    Some(self.tok(p, p + 1, SEPARATOR))
                }
                b'(' if top == F_CLOSURE => Some(self.open_param_tuple(p)),
                b':' if top == F_CLOSURE
                    && let Some(o) = colon_paren(b, p) =>
                {
                    Some(self.param_tuple_type(p, o, F_CLOSURE_TYPE))
                }
                b':' if top == F_CLOSURE && b.get(p + 1) != Some(&b':') => {
                    self.set_top(F_CLOSURE_TYPE);
                    self.prev = P_COLON;
                    Some(self.tok(p, p + 1, SEPARATOR))
                }
                b')' | b'}' | b']' | b';' | b'{' => {
                    // no `|` closed them: they end before a closer or a block
                    self.pop_code();
                    Some(p)
                }
                _ if top == F_CLOSURE && act == A_NAME => {
                    let e = name_end(b, p);
                    let w = &b[p..e];
                    self.prev = P_VALUE;
                    if crate::same(w, b"mut") || crate::same(w, b"ref") {
                        return Some(self.tok(p, e, MODIFIER));
                    }
                    Some(self.tok(p, e, PARAMETER))
                }
                _ if top == F_CLOSURE_TYPE => self.type_turn(p, c, act, false, false),
                _ => None,
            },
            F_STRUCT | F_STRUCT_W => match c {
                b'}' => {
                    self.pop_code();
                    self.prev = P_START;
                    Some(self.tok(p, p + 1, BLOCK_END))
                }
                b':' if b.get(p + 1) != Some(&b':') => {
                    self.set_top(if top == F_STRUCT {
                        F_FIELD_TYPE
                    } else {
                        F_FIELD_TYPE_W
                    });
                    self.prev = P_COLON;
                    Some(self.tok(p, p + 1, SEPARATOR_TYPE))
                }
                _ if act == A_NAME => {
                    let e = name_end(b, p);
                    // `name(?=\s*:)`, as bat's: `c::X` names a field `c` too
                    let s = if matches!(b.get(e), Some(b' ' | b'\t')) {
                        skip_spaces(b, e)
                    } else {
                        e
                    };
                    if b.get(s) == Some(&b':') && is_ident(&b[p..e]) {
                        self.prev = P_VALUE;
                        self.tok(p, e, MEMBER);
                        if b.get(s + 1) == Some(&b':') {
                            self.set_top(if top == F_STRUCT {
                                F_FIELD_TYPE
                            } else {
                                F_FIELD_TYPE_W
                            });
                        }
                        return Some(e);
                    }
                    // `pub`, else a word no field starts with, plain
                    Some(self.pub_crate(p, e).unwrap_or(e))
                }
                b'#' => None,
                b'/' if matches!(b.get(p + 1), Some(b'/' | b'*')) => None,
                _ => Some(p + 1),
            },
            F_FIELD_TYPE | F_FIELD_TYPE_W => match c {
                // the field's `,` plain (a separator past a struct's `where`), then the fields'
                b',' => {
                    self.prev = P_OP;
                    if top == F_FIELD_TYPE_W {
                        self.set_top(F_STRUCT_W);
                        return Some(self.tok(p, p + 1, SEPARATOR));
                    }
                    self.set_top(F_STRUCT);
                    Some(p + 1)
                }
                b'}' => {
                    self.set_top(if top == F_FIELD_TYPE {
                        F_STRUCT
                    } else {
                        F_STRUCT_W
                    });
                    Some(p)
                }
                _ => self.type_turn(p, c, act, false, false),
            },
            F_TYPE_GROUP => match c {
                b')' => {
                    self.pop_code();
                    self.prev = P_VALUE;
                    Some(self.tok(p, p + 1, GROUP_END))
                }
                _ => self.type_turn(p, c, act, false, false),
            },
            F_RET_TYPE => self.type_turn(p, c, act, false, true),
            F_HEAD => match c {
                b'{' => {
                    self.set_top(F_BLOCK);
                    self.prev = P_START;
                    Some(self.tok(p, p + 1, BLOCK_BEGIN))
                }
                b';' | b'}' | b')' | b']' => {
                    self.pop_code();
                    Some(p)
                }
                // a trait's or an enum's generics, supertraits and `where` as a function's (X-064;
                // bat's are plain)
                b':' if b.get(p + 1) != Some(&b':') => {
                    self.prev = P_COLON;
                    Some(self.tok(p, p + 1, SEPARATOR))
                }
                b'<' => None,
                _ if act == A_NAME => {
                    let e = name_end(b, p);
                    if matches!(self.prev, P_TRAIT | P_ENUM) || WORDS.get(&b[p..e]) as u8 == W_WHERE
                    {
                        return None;
                    }
                    Some(self.type_name(p, e, false, top))
                }
                _ => self.type_turn(p, c, act, false, false),
            },
            F_WHERE => match c {
                b'{' | b';' => {
                    self.pop_code();
                    Some(p)
                }
                // its `,` plain, as bat's
                b',' => {
                    self.prev = P_OP;
                    Some(p + 1)
                }
                _ => self.type_turn(p, c, act, false, false),
            },
            F_STRUCT_HEAD | F_STRUCT_HEAD_W => match c {
                b'{' => {
                    self.set_top(if top == F_STRUCT_HEAD {
                        F_STRUCT
                    } else {
                        F_STRUCT_W
                    });
                    self.prev = P_START;
                    Some(self.tok(p, p + 1, BLOCK_BEGIN))
                }
                b'(' => {
                    self.push_code(F_TUPLE);
                    self.prev = P_OP;
                    Some(self.tok(p, p + 1, GROUP_BEGIN))
                }
                b';' => {
                    self.pop_code();
                    self.prev = P_START;
                    Some(self.tok(p, p + 1, TERMINATOR))
                }
                b'}' | b')' | b']' => {
                    // a head misread from a macro's words ends at a closer
                    self.pop_code();
                    Some(p)
                }
                _ if act == A_NAME && has(b, p, b"where") && ends_word(b, p + 5) => {
                    self.set_top(F_STRUCT_HEAD_W);
                    None
                }
                _ if act == A_NAME && self.prev != P_STRUCT => Some(name_end(b, p)),
                b':' if b.get(p + 1) != Some(&b':') => Some(p + 1),
                b'$' => {
                    // a macro's `struct $name`: the name; past it a `$` ends a misread head
                    if self.prev == P_STRUCT
                        && let Some(e) = ident_end(b, p + 1)
                    {
                        self.prev = P_DEF_NAME;
                        return Some(self.tok(p, e, VARIABLE));
                    }
                    self.pop_code();
                    Some(p)
                }
                _ => None,
            },
            // its own `)` ends it, a field's attribute is one, `pub` a modifier
            F_TUPLE => match c {
                b')' => {
                    self.pop_code();
                    self.prev = P_VALUE;
                    Some(self.tok(p, p + 1, GROUP_END))
                }
                b'{' | b'}' => {
                    // no field holds a brace: a header misread (a macro's `struct $name {`)
                    self.pop_code();
                    Some(p)
                }
                b'#' => None,
                _ if act == A_NAME && has(b, p, b"pub") && ends_word(b, p + 3) => {
                    self.pub_crate(p, p + 3)
                }
                _ => self.type_turn(p, c, act, false, false),
            },
            F_IMPL | F_IMPL_TRAIT | F_IMPL_FOR | F_IMPL_NAMED => self.impl_turn(p, c, act, top),
            // its first argument plain to its first `,` (plain too), as bat's
            // (its groups balanced: `write!(f.out(), …)`; bat's never ends, `writeln!(out)` derails it)
            F_WRITE => match c {
                b',' if self.stack[self.depth - 2] == F_WRITE => Some(p + 1),
                b',' => {
                    self.set_top(F_GROUP);
                    self.prev = P_FMT;
                    Some(p + 1)
                }
                b'(' => {
                    self.push_code(F_WRITE);
                    Some(p + 1)
                }
                b')' => {
                    self.pop_code();
                    self.prev = P_VALUE;
                    if self.top() == F_WRITE {
                        return Some(p + 1);
                    }
                    Some(self.tok(p, p + 1, META_GROUP_END))
                }
                b'/' if matches!(b.get(p + 1), Some(b'/' | b'*')) => None,
                b'$' => None,
                _ => Some(if is_word(c) { word_end(b, p) } else { p + 1 }),
            },
            F_PSQL..=F_PSQL_LAST => self.sql_frame(p, c, top),
            F_MACRO_BLOCK..=F_MACRO_TERM => Some(self.macro_turn(p, c, top)),
            _ => None,
        }
    }

    /// An impl's head at `p`, as bat reads one: a trait's path before `for` (its `::` an
    /// accessor), then the impl's type, whose every name is the impl's (its `::` and brackets
    /// plain), to its body's `{`.
    #[inline(never)]
    fn impl_turn(&mut self, p: usize, c: u8, act: u8, top: u8) -> Option<usize> {
        let b = self.b;
        match c {
            b'{' => {
                self.set_top(F_BLOCK);
                self.prev = P_START;
                return Some(self.tok(p, p + 1, BLOCK_BEGIN));
            }
            b';' | b'}' | b')' => {
                self.pop_code();
                return Some(p);
            }
            _ => {}
        }
        if act != A_NAME {
            let named = top != F_IMPL_TRAIT;
            return match c {
                b':' if named && b.get(p + 1) == Some(&b':') => Some(p + 2),
                b'[' | b']' if named => Some(p + 1),
                b'*' => Some(p + 1),
                _ => None,
            };
        }
        let e = name_end(b, p);
        let id = WORDS.get(&b[p..e]) as u8;
        if id == W_WHERE {
            return None;
        }
        if id == W_FOR && top == F_IMPL_TRAIT {
            self.set_top(F_IMPL_FOR);
            self.prev = P_OP;
            return Some(self.tok(p, e, KEYWORD));
        }
        match top {
            F_IMPL_TRAIT => Some(self.type_name(p, e, false, top)),
            F_IMPL if impl_for_ahead(b, p) => {
                self.set_top(F_IMPL_TRAIT);
                Some(self.type_name(p, e, false, F_IMPL_TRAIT))
            }
            _ => {
                self.set_top(F_IMPL_NAMED);
                self.prev = P_DEF_NAME;
                Some(self.tok(p, e, IMPL_NAME))
            }
        }
    }

    // --- types ---

    /// A generic's arguments (or an array's in them, `[T; N]`) at `p`: their rules, or `None`
    /// where the token is the plain loop's (a number, an attribute, a comment, `$name`).
    #[inline(never)]
    fn generic_turn(&mut self, p: usize, c: u8, act: u8, top: u8) -> Option<usize> {
        let b = self.b;
        let next = b.get(p + 1).copied();
        let comment = c == b'/' && matches!(next, Some(b'/' | b'*'));
        if !IN_TYPE[usize::from(c)]
            || (c == b'-' && next != Some(b'>'))
            || (c == b';' && top != F_GENERIC_ARRAY)
            || (c == b'/' && !comment)
            || (c == b'>' && top == F_GENERIC_ARRAY)
        {
            // a byte no type holds: the generics end, and the frame below reads it
            while matches!(self.top(), F_GENERIC | F_GENERIC_FN | F_GENERIC_ARRAY) {
                self.pop_code();
            }
            return Some(p);
        }
        match c {
            b'>' => {
                self.prev = if top == F_GENERIC_FN {
                    P_FN_NAME
                } else {
                    P_VALUE
                };
                self.pop_code();
                Some(self.tok(p, p + 1, GENERIC_END))
            }
            b']' if top == F_GENERIC_ARRAY => {
                self.pop_code();
                Some(self.tok(p, p + 1, GROUP_END))
            }
            b';' => Some(self.tok(p, p + 1, SEPARATOR)),
            b'[' => {
                self.push_code(F_GENERIC_ARRAY);
                Some(self.tok(p, p + 1, GROUP_BEGIN))
            }
            b'-' => Some(self.tok(p, p + 2, SEPARATOR_GENERIC)),
            // plain in the arguments: `,`, `=` (an associated type's), `!` (the never type), the
            // brackets bat's grammar lets through
            b',' | b'=' | b'!' | b')' | b']' => {
                // a closer ends a value: a comparison read as a generic (`a<(b) | c`) keeps its `|`
                self.prev = if matches!(c, b')' | b']') {
                    P_VALUE
                } else {
                    P_OP
                };
                Some(p + 1)
            }
            b'+' | b'?' => Some(self.tok(p, p + 1, OPERATOR)),
            _ if act == A_NAME => {
                let e = name_end(b, p);
                if b.get(e) == Some(&b'!') {
                    // a macro in a type (`Token![,]`): its brackets hold code
                    let (frame, leaf) = match b.get(e + 1) {
                        Some(b'[') => (F_BRACKET, GROUP_BEGIN),
                        Some(b'(') => (F_GROUP, GROUP_BEGIN),
                        Some(b'{') => (F_BLOCK, BLOCK_BEGIN),
                        _ => return Some(self.type_name(p, e, true, top)),
                    };
                    self.tok(p, e + 1, SUPPORT_MACRO);
                    self.push_code(frame);
                    self.prev = P_OP;
                    return Some(self.tok(e + 1, e + 2, leaf));
                }
                if e - p == 6 && has(b, p, b"extern") {
                    return Some(self.generic_extern(p, e));
                }
                Some(self.type_name(p, e, true, top))
            }
            _ if act == A_DIGIT => None,
            _ => self.type_turn(p, c, act, true, false),
        }
    }

    /// A type frame's token at `p` that isn't its own closer: `type-any-identifier`'s rules (with
    /// `generic`, a generic argument's), or `None` where the plain loop's (an attribute, a
    /// comment, `$name`). In a return type (`ret`), a byte no type holds ends the frame.
    #[inline(always)]
    fn type_turn(&mut self, p: usize, c: u8, act: u8, generic: bool, ret: bool) -> Option<usize> {
        let b = self.b;
        if act == A_NAME {
            let e = name_end(b, p);
            return Some(self.type_name(p, e, generic, if ret { F_RET_TYPE } else { 0 }));
        }
        if act == A_DIGIT && !ret {
            return Some(name_end(b, p));
        }
        let next = b.get(p + 1).copied();
        let r = match c {
            b'&' => self.tok(p, p + 1, OPERATOR),
            b'*' => {
                let n = if has(b, p + 1, b"mut") {
                    4
                } else if has(b, p + 1, b"const") {
                    6
                } else if ret {
                    return self.end_type(p);
                } else {
                    return Some(p + 1);
                };
                if !ends_word(b, p + n) {
                    return Some(p + 1);
                }
                self.tok(p, p + n, MODIFIER)
            }
            b'\'' => match ident_end(b, p + 1) {
                Some(e) if b.get(e) != Some(&b'\'') => self.tok(p, e, LIFETIME),
                _ => p + 1,
            },
            b':' if next == Some(b':') => {
                self.prev = P_PATH;
                self.tok(p, p + 2, PATH_ACCESSOR)
            }
            b'<' => {
                self.push_code(F_GENERIC);
                self.prev = P_OP;
                self.tok(p, p + 1, GENERIC_BEGIN)
            }
            b'(' => {
                self.push_code(F_TYPE_GROUP);
                self.prev = P_OP;
                self.tok(p, p + 1, GROUP_BEGIN)
            }
            b'#' => {
                // in a generic an attribute (`#[may_dangle] T`), else a `quote!` interpolation's
                // `#`, plain; plain in a type
                let m = if next == Some(b'!') { p + 2 } else { p + 1 };
                if generic && b.get(m) == Some(&b'[') {
                    return None;
                }
                p + 1
            }
            b'$' => return None,
            b'/' if matches!(next, Some(b'/' | b'*')) => return None,
            // plain in a parameter's type, as bat's
            b'-' if next == Some(b'>') && !ret && self.top() == F_PARAM_TYPE => p + 2,
            b'-' if next == Some(b'>') && !ret => self.tok(p, p + 2, SEPARATOR),
            _ if ret => return self.end_type(p),
            b':' => self.tok(p, p + 1, SEPARATOR),
            _ => p + 1,
        };
        Some(r)
    }

    /// `extern` at `p..e` in a generic's arguments, and its ABI's string on the line.
    #[inline(never)]
    fn generic_extern(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        self.tok(p, e, KEYWORD);
        let s = skip_spaces(b, e);
        if b.get(s) != Some(&b'"') {
            return e;
        }
        let Some(q) = b[s + 1..].iter().position(|&c| c == b'"' || c == b'\\') else {
            return e;
        };
        let q = s + 1 + q;
        if b[q] != b'"' {
            return e;
        }
        self.tok(s, s + 1, STR_BEGIN);
        self.tok(s + 1, q, STR);
        self.tok(q, q + 1, STR_END)
    }

    /// `macro_rules!`'s frame `top` at `p` (not a blank, not the line's end): X-064's
    /// frames, bat's but where it derails (a matcher's groups balanced, matchers in any
    /// bracket). What none of them takes is plain: a word whole, else a byte.
    #[inline(never)]
    fn macro_turn(&mut self, p: usize, c: u8, top: u8) -> usize {
        let b = self.b;
        let plain = |b: &[u8]| if is_word(c) { word_end(b, p) } else { p + 1 };
        let is_matcher = (F_MATCHER..=F_NEST_CURLY).contains(&top);
        if c == b'/' && matches!(b.get(p + 1), Some(b'/' | b'*')) {
            return self.comment(p).unwrap_or(p + 1);
        }
        if c == b'$'
            && !is_matcher
            && let Some(e) = ident_end(b, p + 1)
        {
            return self.tok(p, e, VARIABLE);
        }
        match top {
            F_MACRO_BLOCK => {
                let (next, leaf) = match c {
                    b'{' => (F_MACRO_RULES, META_BLOCK_BEGIN),
                    b'[' => (F_MACRO_RULES_BRACKET, META_GROUP_BEGIN),
                    b'(' => (F_MACRO_RULES_PAREN, META_GROUP_BEGIN),
                    _ => {
                        self.pop_code();
                        return p;
                    }
                };
                self.set_top(next);
                self.tok(p, p + 1, leaf)
            }
            F_MACRO_RULES | F_MACRO_RULES_BRACKET | F_MACRO_RULES_PAREN => {
                let close = match top {
                    F_MACRO_RULES => b'}',
                    F_MACRO_RULES_BRACKET => b']',
                    _ => b')',
                };
                if c == close {
                    self.pop_code();
                    self.prev = P_START;
                    return self.tok(p, p + 1, if c == b'}' { BLOCK_END } else { META_GROUP_END });
                }
                let matcher = match c {
                    b'(' => F_MATCHER,
                    b'[' => F_MATCHER_SQUARE,
                    b'{' => F_MATCHER_CURLY,
                    _ => return plain(b),
                };
                for f in [F_MACRO_TERM, F_MACRO_BODY, F_MATCH_OP, matcher] {
                    self.push_code(f);
                }
                self.tok(p, p + 1, GROUP_BEGIN)
            }
            F_MATCHER..=F_NEST_CURLY => {
                let close = CLOSERS[usize::from(top - F_MATCHER)];
                if c == close {
                    return match top {
                        F_MATCHER..=F_MATCHER_CURLY => {
                            self.pop_code();
                            self.tok(p, p + 1, GROUP_END)
                        }
                        F_MATCHER_GROUP => {
                            self.tok(p, p + 1, GROUP_END);
                            // `(?:\s*[^*+]?\s*([*+]))?`
                            let j = skip_spaces(b, p + 1);
                            let rep = |k: usize| matches!(b.get(k), Some(b'*' | b'+'));
                            let op = if rep(j) {
                                Some(j)
                            } else if j < b.len() {
                                let k = skip_spaces(b, j + char_len(b[j]));
                                rep(k).then_some(k)
                            } else {
                                None
                            };
                            let end = match op {
                                Some(k) => self.tok(k, k + 1, OPERATOR),
                                None => p + 1,
                            };
                            self.pop_code();
                            end
                        }
                        // a plain group's closer, plain
                        _ => {
                            self.pop_code();
                            p + 1
                        }
                    };
                }
                match c {
                    b'$' if b.get(p + 1) == Some(&b'(') => {
                        self.push_code(F_MATCHER_GROUP);
                        self.tok(p, p + 1, REPEAT_BEGIN);
                        self.tok(p + 1, p + 2, GROUP_BEGIN)
                    }
                    b'$' => match ident_end(b, p + 1) {
                        Some(e) if b.get(e) == Some(&b':') => {
                            self.metavariable(p, e).unwrap_or(p + 1)
                        }
                        _ => p + 1,
                    },
                    // a literal string, so that a closer in it isn't the matcher's
                    b'"' => self.string_at(p).unwrap_or(p + 1),
                    b'(' | b'[' | b'{' => {
                        let nest = match c {
                            b'(' => F_NEST_PAREN,
                            b'[' => F_NEST_SQUARE,
                            _ => F_NEST_CURLY,
                        };
                        self.push_code(nest);
                        p + 1
                    }
                    _ => plain(b),
                }
            }
            F_MATCH_OP => {
                self.pop_code();
                if c == b'=' && b.get(p + 1) == Some(&b'>') {
                    return self.tok(p, p + 2, OPERATOR);
                }
                p
            }
            F_MACRO_BODY => {
                let (frame, leaf) = match c {
                    b'{' => (F_BLOCK, BLOCK_BEGIN),
                    b'(' => (F_GROUP, GROUP_BEGIN),
                    b'[' => (F_BRACKET, GROUP_BEGIN),
                    _ => {
                        self.pop_code();
                        return p;
                    }
                };
                self.set_top(frame);
                self.prev = P_OPEN;
                self.tok(p, p + 1, leaf)
            }
            // F_MACRO_TERM
            _ => {
                if c == b';' || c == b',' {
                    return self.tok(p, p + 1, MACRO_TERMINATOR);
                }
                self.pop_code();
                p
            }
        }
    }

    /// A tuple's pattern's `(` at `p` among parameters.
    #[inline(never)]
    fn open_param_tuple(&mut self, p: usize) -> usize {
        self.push_code(F_PARAM_TUPLE);
        self.prev = P_OPEN;
        self.tok(p, p + 1, META_GROUP_BEGIN)
    }

    /// `:` at `p` before a tuple's type's `(` at `o`, among parameters: the `:`, the type (the
    /// frame `then` below it, which its `)` leaves).
    #[inline(never)]
    fn param_tuple_type(&mut self, p: usize, o: usize, then: u8) -> usize {
        self.tok(p, p + 1, SEPARATOR);
        self.set_top(then);
        self.push_code(F_PARAM_TUPLE_TYPE);
        self.prev = P_OPEN;
        self.tok(o, o + 1, META_GROUP_BEGIN)
    }

    /// `pub(crate)` at `p` (`pub` to `e`) among a struct's fields, as bat reads it: `None`
    /// where it is another name.
    #[inline(never)]
    fn pub_crate(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        if WORDS.get(&b[p..e]) as u8 != W_PUB {
            return None;
        }
        self.prev = P_OP;
        self.tok(p, e, MODIFIER);
        if has(b, e, b"(crate)") {
            self.tok(e, e + 1, MODIFIER_BEGIN);
            self.tok(e + 1, e + 6, MODIFIER);
            return Some(self.tok(e + 6, e + 7, MODIFIER_END));
        }
        Some(e)
    }

    /// A return type ended before `p`: the frame below reads it.
    #[inline(never)]
    fn end_type(&mut self, p: usize) -> Option<usize> {
        self.pop_code();
        self.prev = P_VALUE;
        Some(p)
    }

    /// A name `p..e` in a type (`top` the type frame, `F_RET_TYPE` for a return type's), with
    /// `generic` a generic argument's: `ref`/`mut`/`const`, `fn(`, a path's segment, a name
    /// before `<`, a primitive, `_`; in a generic a `support.type` word and `as`; else plain.
    #[inline(never)]
    fn type_name(&mut self, p: usize, e: usize, generic: bool, top: u8) -> usize {
        let b = self.b;
        let w = &b[p..e];
        let next = b.get(e).copied();
        let id = WORDS.get(w) as u8;
        self.prev = P_VALUE;
        if top == F_RET_TYPE && id == W_WHERE {
            self.pop_code();
            return p;
        }
        match id {
            W_REF | W_MUT | W_CONST => return self.tok(p, e, MODIFIER),
            W_FN if next == Some(b'(') => {
                self.tok(p, e, FN);
                self.push_code(F_TYPE_GROUP);
                self.prev = P_OP;
                return self.tok(e, e + 1, META_GROUP_BEGIN);
            }
            _ => {}
        }
        let ident = is_ident(w);
        if next == Some(b':') && b.get(e + 1) == Some(&b':') {
            self.prev = P_PATH;
            if is_type_name(w) {
                self.tok(p, e, PATH_TYPE);
                return self.tok(e, e + 2, TYPE_ACCESSOR);
            }
            if ident {
                return self.tok(e, e + 2, PATH_ACCESSOR);
            }
        }
        let group = INFO[usize::from(id)].0;
        if next == Some(b'<') && ident {
            if group == G_SUPPORT {
                self.tok(p, e, SUPPORT_TYPE);
            }
            self.push_code(F_GENERIC);
            self.prev = P_OP;
            return self.tok(e, e + 1, GENERIC_BEGIN);
        }
        if group == G_PRIMITIVE {
            return self.tok(p, e, STORAGE);
        }
        if e - p == 1 && b[p] == b'_' {
            return self.tok(p, e, OPERATOR);
        }
        if generic {
            if group == G_SUPPORT {
                return self.tok(p, e, SUPPORT_TYPE);
            }
            if id == W_AS {
                return self.tok(p, e, OPERATOR);
            }
        }
        e
    }

    /// A path's next segment `p..e`, after `::` in code: a constant, a C type, `self`, `super`,
    /// the next segment's `::`; else plain, a call's and a macro's name too.
    #[inline(never)]
    fn path_name(&mut self, p: usize, e: usize, id: u8, next: u8, bits: u8) -> usize {
        self.prev = P_VALUE;
        match id {
            W_SELF => return self.tok(p, e, SELF),
            W_SUPER => return self.tok(p, e, KEYWORD),
            _ => {}
        }
        if id != 0 && INFO[usize::from(id)].0 == G_SQL && matches!(next, b'(' | b'!') {
            self.sql_name(p, e);
        }
        if next == b'<' {
            self.plain_name(p, e, bits);
            return self.open_generic(e);
        }
        self.plain_name(p, e, bits)
    }

    // --- names ---

    /// A name at `p` in code frame `top`.
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
        let next = if e < n { b[e] } else { 0 };
        if e - p <= 2 && matches!(next, b'"' | b'#' | b'\'') {
            // a string's prefix: `b"`, `r"`, `r#"`, `br"`, `b'`, `c"`
            if let Some(q) = self.prefixed(p) {
                return q;
            }
        }
        let id = WORDS.get(&b[p..e]) as u8;
        let prev = self.prev;
        if prev == P_PATH {
            return self.path_name(p, e, id, next, bits);
        }
        if id != 0 {
            let (group, leaf, after) = INFO[usize::from(id)];
            if group <= G_SUPPORT {
                if group == G_KEYWORD {
                    return self.keyword(p, e, id, leaf, after, top);
                }
                self.prev = P_VALUE;
                if next == b'<' {
                    // a support type's or a primitive's generic
                    self.tok(p, e, leaf);
                    return self.open_generic(e);
                }
                return self.tok(p, e, leaf);
            }
            if group == G_ASKED
                && let Some(q) = self.asked(p, e, id, next)
            {
                return q;
            }
        }
        // a name: the place before it names it, then the byte after it
        let kind = match prev {
            P_FN => {
                self.prev = P_FN_NAME;
                return self.tok(p, e, FN_NAME);
            }
            P_STRUCT => STRUCT_NAME,
            P_ENUM => ENUM_NAME,
            P_TRAIT => TRAIT_NAME,
            P_TYPE => TYPE_NAME,
            P_MOD => MODULE_NAME,
            P_MACRO => MACRO_NAME,
            _ => 0,
        };
        if kind != 0 {
            self.prev = P_DEF_NAME;
            return self.tok(p, e, kind);
        }
        self.prev = P_VALUE;
        match next {
            b'(' => {
                if lower(b[p], bits) {
                    if id != 0 && INFO[usize::from(id)].0 == G_SQL {
                        self.sql_name(p, e);
                    }
                    return self.tok(p, e, SUPPORT_FN);
                }
                return self.plain_name(p, e, bits);
            }
            b'!' => return self.macro_name(p, e, id, bits),
            b'<' => {
                self.plain_name(p, e, bits);
                return self.open_generic(e);
            }
            b' ' | b'\t' | b'=' => {
                // `name = |…|`: the closure's name
                if let Some(q) = self.closure_named(p, e) {
                    return q;
                }
            }
            b':' if b.get(e + 1) == Some(&b':')
                && id != 0
                && b.get(e + 2) == Some(&b'<')
                && INFO[usize::from(id)].0 == G_SQL =>
            {
                self.sql_name(p, e);
            }
            _ => {}
        }
        self.plain_name(p, e, bits)
    }

    /// A name `p..e` that no place names, its bytes' classes `bits`: a constant's, a C type's,
    /// `_`, else plain.
    #[inline(always)]
    fn plain_name(&mut self, p: usize, e: usize, bits: u8) -> usize {
        let b = self.b;
        let first = b[p];
        if matches!(first, b'A'..=b'Z' | b'_') {
            if e - p >= 2 && bits & (C_LO | C_HI) == 0 {
                return self.tok(p, e, CONSTANT);
            }
            if e - p == 1 && first == b'_' {
                return self.tok(p, e, OPERATOR);
            }
            if first != b'_' {
                return e;
            }
        }
        if e - p >= 3
            && ((b[p + 1] == b'_' && first == b'c') || (b[e - 2] == b'_' && b[e - 1] == b't'))
            && lower(first, bits)
        {
            return self.tok(p, e, STORAGE);
        }
        e
    }

    /// A keyword `p..e`.
    #[inline(always)]
    fn keyword(&mut self, p: usize, e: usize, id: u8, leaf: u16, after: u8, top: u8) -> usize {
        let prev = self.prev;
        self.prev = match after {
            P_KEEP => {
                if prev == P_START {
                    P_START
                } else {
                    P_OP
                }
            }
            P_SAME => prev,
            _ => after,
        };
        if id <= W_IMPL {
            match id {
                W_STRUCT => {
                    self.push_code(F_STRUCT_HEAD);
                }
                W_TRAIT | W_ENUM => {
                    self.push_code(F_HEAD);
                }
                W_IMPL => {
                    if prev == P_COLON {
                        // `impl Trait` in a type: plain, as bat's
                        self.prev = P_OP;
                        return e;
                    }
                    self.push_code(F_IMPL);
                }
                W_FN => {}
                _ => {}
            }
        } else if id == W_FOR && top == F_IMPL_TRAIT {
            self.set_top(F_IMPL_FOR);
        } else if id == W_WHERE {
            self.push_code(F_WHERE);
        }
        self.tok(p, e, leaf)
    }

    /// A word asked for by id at `p..e`, before `next`: `None` where it is a name as any.
    #[inline(never)]
    fn asked(&mut self, p: usize, e: usize, id: u8, next: u8) -> Option<usize> {
        let b = self.b;
        match id {
            W_SELF | W_SUPER if next == b'(' => {
                self.prev = P_VALUE;
                Some(self.tok(p, e, SUPPORT_FN))
            }
            W_SELF => {
                self.prev = P_VALUE;
                Some(self.tok(
                    p,
                    e,
                    if self.top() == F_PARAMS {
                        PARAMETER
                    } else {
                        SELF
                    },
                ))
            }
            W_SUPER => {
                self.prev = P_VALUE;
                Some(self.tok(p, e, KEYWORD))
            }
            W_MACRO_RULES if next == b'!' => {
                let s = skip_spaces(b, e + 1);
                if s > e + 1 && matches!(b.get(s), Some(&c) if is_letter(c) || c == b'_') {
                    let m = word_end(b, s);
                    self.tok(p, e + 1, SUPPORT_FN);
                    self.prev = P_DEF_NAME;
                    self.push_code(F_MACRO_BLOCK);
                    return Some(self.tok(s, m, MACRO_NAME));
                }
                None
            }
            W_FORMAT..=W_EPRINTLN | W_WRITE | W_WRITELN if next == b'!' => {
                let s = skip_spaces(b, e + 1);
                if b.get(s) != Some(&b'(') {
                    return None;
                }
                self.tok(p, e + 1, SUPPORT_MACRO);
                if id >= W_WRITE {
                    self.push_code(F_WRITE);
                    self.prev = P_OP;
                } else {
                    self.push_code(F_GROUP);
                    self.prev = P_FMT;
                }
                Some(self.tok(s, s + 1, META_GROUP_BEGIN))
            }
            _ => None,
        }
    }

    /// `name!` at `p..e`: a macro's call where a bracket follows.
    #[inline(never)]
    fn macro_name(&mut self, p: usize, e: usize, id: u8, bits: u8) -> usize {
        let b = self.b;
        if b.get(e + 1) == Some(&b'=') {
            return self.plain_name(p, e, bits);
        }
        let s = skip_spaces(b, e + 1);
        match b.get(s) {
            Some(b'(') if lower(b[p], bits) => {
                if id != 0 && INFO[usize::from(id)].0 == G_SQL {
                    self.sql_name(p, e);
                }
                self.tok(p, e + 1, SUPPORT_MACRO)
            }
            Some(b'(' | b'[' | b'{') if lower(b[p], bits) => self.tok(p, e + 1, SUPPORT_MACRO),
            _ => self.plain_name(p, e, bits),
        }
    }

    /// `name\s*=\s*|`, the name at `p..e`: the closure's name, its `=`, its parameters opened.
    #[inline(always)]
    fn closure_named(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        let s = skip_spaces(b, e);
        if b.get(s) != Some(&b'=') || !is_ident(&b[p..e]) {
            return None;
        }
        let t = skip_spaces(b, s + 1);
        if b.get(t) != Some(&b'|') {
            return None;
        }
        self.tok(p, e, FN_NAME);
        self.tok(s, s + 1, OPERATOR);
        Some(self.open_closure(t))
    }

    /// A string's prefix at `p` (`b`, `br`, `r`, `c`, before `"`, `#` or `'`): the string, or
    /// `None` where it is a name.
    #[inline(never)]
    fn prefixed(&mut self, p: usize) -> Option<usize> {
        let fmt = self.prev == P_FMT;
        let e = self.string_open(p, true, fmt)?;
        self.prev = P_VALUE;
        if self.b[p] != b'b' {
            self.regex_in(p, e);
        }
        Some(e)
    }

    // --- punctuation ---

    fn open_paren(&mut self, p: usize, top: u8) -> usize {
        if p == self.sql_paren {
            self.push_code(F_PSQL + self.sql_call);
            self.prev = P_OPEN;
            return self.tok(p, p + 1, GROUP_BEGIN);
        }
        if self.prev == P_FN_NAME {
            self.push_code(F_PARAMS);
            self.prev = P_OPEN;
            return self.tok(p, p + 1, PARAMS_BEGIN);
        }
        let _ = top;
        self.push_code(F_GROUP);
        self.prev = P_OPEN;
        self.tok(p, p + 1, GROUP_BEGIN)
    }

    fn close_paren(&mut self, p: usize, top: u8) -> usize {
        self.prev = P_VALUE;
        if top == F_GROUP {
            self.pop_code();
            return self.tok(p, p + 1, META_GROUP_END);
        }
        p + 1
    }

    fn open_curly(&mut self, p: usize, top: u8) -> usize {
        let _ = top;
        self.push_code(F_BLOCK);
        self.prev = P_OPEN;
        self.tok(p, p + 1, BLOCK_BEGIN)
    }

    fn close_curly(&mut self, p: usize, top: u8) -> usize {
        self.prev = P_START;
        if top == F_BLOCK {
            self.pop_code();
            return self.tok(p, p + 1, BLOCK_END);
        }
        p + 1
    }

    fn dot(&mut self, p: usize) -> usize {
        let b = self.b;
        if b.get(p + 1) == Some(&b'.') {
            let n = if b.get(p + 2) == Some(&b'.') { 3 } else { 2 };
            self.prev = P_OP;
            return self.tok(p, p + n, RANGE);
        }
        self.prev = P_OP;
        self.tok(p, p + 1, DOT)
    }

    fn colon(&mut self, p: usize) -> usize {
        let b = self.b;
        if b.get(p + 1) == Some(&b':')
            && (ident_starts(b, p + 2)
                || (p > 0 && NAME[usize::from(b[p - 1])] && !(p >= 5 && has(b, p - 5, b"crate"))))
        {
            self.prev = P_PATH;
            return self.tok(p, p + 2, PATH_ACCESSOR);
        }
        self.prev = P_COLON;
        self.tok(p, p + 1, SEPARATOR)
    }

    fn semi(&mut self, p: usize, top: u8) -> usize {
        self.prev = P_SEP;
        if top == F_BRACKET {
            return self.tok(p, p + 1, SEPARATOR);
        }
        self.tok(p, p + 1, TERMINATOR)
    }

    fn comma(&mut self, p: usize, top: u8) -> usize {
        let _ = top;
        self.prev = P_SEP;
        self.tok(p, p + 1, SEPARATOR)
    }

    fn equals(&mut self, p: usize) -> usize {
        self.prev = P_SEP;
        match self.b.get(p + 1) {
            Some(b'>') => self.tok(p, p + 2, OPERATOR),
            Some(b'=') => {
                self.prev = P_OP;
                self.tok(p, p + 2, COMPARISON)
            }
            _ => self.tok(p, p + 1, ASSIGNMENT),
        }
    }

    /// `<` at `p`: a generic's after a name, `::`, `:`, `->`, an operator or a function's
    /// name; else a comparison.
    fn less(&mut self, p: usize) -> usize {
        let b = self.b;
        let generic = (p > 0 && NAME[usize::from(b[p - 1])])
            || matches!(self.prev, P_SEP | P_PATH | P_COLON | P_FN_NAME | P_DEF_NAME);
        if generic {
            return self.open_generic(p);
        }
        self.prev = P_OP;
        self.compare(p, b'<')
    }

    /// A generic's `<` at `p`.
    #[inline(never)]
    fn open_generic(&mut self, p: usize) -> usize {
        let f = if self.prev == P_FN_NAME {
            F_GENERIC_FN
        } else {
            F_GENERIC
        };
        self.push_code(f);
        self.prev = P_OP;
        self.tok(p, p + 1, GENERIC_BEGIN)
    }

    /// `>` at `p`: a comparison. It leaves a value: a `|` after it is an operator, not a closure
    /// (`tag!(<a> | <b>)`, a generic no frame holds; nobody compares with a closure).
    fn greater(&mut self, p: usize) -> usize {
        self.prev = P_VALUE;
        self.compare(p, b'>')
    }

    fn compare(&mut self, p: usize, c: u8) -> usize {
        let b = self.b;
        let next = b.get(p + 1).copied();
        if next == Some(c) && b.get(p + 2) == Some(&b'=') {
            self.tok(p, p + 3, ASSIGNMENT)
        } else if next == Some(c) {
            // a shift: one token, so that its second `<` opens no generic after an operator
            self.tok(p, p + 2, COMPARISON)
        } else if next == Some(b'=') {
            self.tok(p, p + 2, COMPARISON)
        } else {
            self.tok(p, p + 1, COMPARISON)
        }
    }

    fn minus(&mut self, p: usize) -> usize {
        if self.b.get(p + 1) == Some(&b'>') {
            if self.prev == P_PARAMS_END {
                self.push_code(F_RET_TYPE);
            }
            self.prev = P_COLON;
            return self.tok(p, p + 2, SEPARATOR);
        }
        self.op_eq(p, ARITHMETIC)
    }

    /// An operator that `=` can follow, making it an assignment.
    fn op_eq(&mut self, p: usize, leaf: u16) -> usize {
        self.prev = P_OP;
        if self.b.get(p + 1) == Some(&b'=') {
            self.tok(p, p + 2, ASSIGNMENT)
        } else {
            self.tok(p, p + 1, leaf)
        }
    }

    fn star(&mut self, p: usize) -> usize {
        let b = self.b;
        let (n, leaf) = if has(b, p + 1, b"const") {
            (6, STORAGE)
        } else if has(b, p + 1, b"mut") {
            (4, MODIFIER)
        } else {
            return self.op_eq(p, ARITHMETIC);
        };
        if ends_word(b, p + n) {
            self.prev = P_OP;
            return self.tok(p, p + n, leaf);
        }
        self.op_eq(p, ARITHMETIC)
    }

    fn amp(&mut self, p: usize) -> usize {
        let next = self.b.get(p + 1).copied();
        let prev = self.prev;
        self.prev = P_OP;
        if next == Some(b'=') {
            self.tok(p, p + 2, ASSIGNMENT)
        } else if next == Some(b'&') {
            // `&&x` after an operator is two references; bat reads `&&` either way
            let _ = prev;
            self.tok(p, p + 2, LOGICAL)
        } else {
            self.tok(p, p + 1, BITWISE)
        }
    }

    /// `|` at `p`: a closure's parameters where an expression starts (after `=`, `,`, `(`, an
    /// operator, `move`, `&`, `[`, a keyword that takes an expression: no left operand there),
    /// else an operator.
    fn pipe(&mut self, p: usize) -> usize {
        if matches!(self.prev, P_OP | P_SEP | P_OPEN | P_COLON | P_PARAMS_END) {
            return self.open_closure(p);
        }
        self.prev = P_OP;
        match self.b.get(p + 1) {
            Some(b'=') => self.tok(p, p + 2, ASSIGNMENT),
            Some(b'|') => self.tok(p, p + 2, LOGICAL),
            _ => self.tok(p, p + 1, BITWISE),
        }
    }

    /// A closure's `|` at `p`.
    #[inline(never)]
    fn open_closure(&mut self, p: usize) -> usize {
        self.push_code(F_CLOSURE);
        self.prev = P_OP;
        self.tok(p, p + 1, PARAMS_BEGIN)
    }

    fn bang(&mut self, p: usize) -> usize {
        self.prev = P_OP;
        if self.b.get(p + 1) == Some(&b'=') {
            self.tok(p, p + 2, COMPARISON)
        } else {
            self.tok(p, p + 1, LOGICAL)
        }
    }

    /// `#` at `p`: an attribute, `#[` or `#![`, else an operator.
    fn hash(&mut self, p: usize) -> usize {
        let b = self.b;
        let m = if b.get(p + 1) == Some(&b'!') {
            p + 2
        } else {
            p + 1
        };
        if b.get(m) != Some(&b'[') {
            self.prev = P_OP;
            return self.tok(p, p + 1, OPERATOR);
        }
        self.push(p, ATTRIBUTE);
        self.put(p, m, ANNOTATION_MARK);
        self.token(m, 1, GROUP_BEGIN)
    }

    /// `$` at `p`: a macro's variable, a matcher's fragment, else an operator.
    fn dollar(&mut self, p: usize) -> usize {
        let b = self.b;
        self.prev = P_VALUE;
        if let Some(e) = ident_end(b, p + 1) {
            if b.get(e) == Some(&b':')
                && let Some(q) = self.metavariable(p, e)
            {
                return q;
            }
            return self.tok(p, e, VARIABLE);
        }
        self.prev = P_OP;
        self.tok(p, p + 1, OPERATOR)
    }

    /// `(\$name)(:)(fragment)` at `p`, the name ending at `e`.
    #[inline(never)]
    fn metavariable(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        let frag = [
            &b"ident"[..],
            b"path",
            b"expr",
            b"ty",
            b"pat",
            b"stmt",
            b"block",
            b"item",
            b"meta",
            b"tt",
        ]
        .into_iter()
        .find(|f| b[e + 1..].starts_with(f) && ends_word(b, e + 1 + f.len()))?;
        self.tok(p, e, MACRO_PARAMETER);
        self.tok(e, e + 1, SEPARATOR);
        Some(self.tok(e + 1, e + 1 + frag.len(), STORAGE))
    }

    /// `"` at `p`: a string, a format string after a format macro's `(`, SQL or a regex where
    /// a call makes it one.
    fn dquote(&mut self, p: usize) -> usize {
        let fmt = self.prev == P_FMT;
        self.prev = P_VALUE;
        self.push(p, if fmt { FMT_STRING } else { STRING });
        let e = self.token(p, 1, STR_BEGIN);
        self.regex_in(p, e);
        e
    }

    fn digits(&mut self, p: usize) -> usize {
        let b = self.b;
        self.prev = P_VALUE;
        // a decimal's digits to a byte that is no number's: the integer alone
        let r = digits_end(b, p, |c| c.is_ascii_digit());
        match b.get(r) {
            Some(&c) if c == b'.' || is_word(c) => {}
            _ => return self.tok(p, r, DECIMAL),
        }
        self.number(p, true).unwrap_or_else(|| word_end(b, p))
    }

    fn slash(&mut self, p: usize) -> usize {
        match self.comment(p) {
            Some(e) => e,
            None => self.op_eq(p, ARITHMETIC),
        }
    }

    // --- literals (X-064's) ---

    /// A `'` at `p`: a label, a lifetime, a character, or an operator.
    fn quote(&mut self, p: usize) -> usize {
        let b = self.b;
        if let Some(e) = ident_end(b, p + 1) {
            let s = skip_spaces(b, e);
            if b.get(s) == Some(&b':') && b.get(s + 1) != Some(&b':') {
                self.tok(p, e, LABEL);
                return self.tok(s, s + 1, SEPARATOR);
            }
            if b.get(e) != Some(&b'\'') {
                return self.tok(p, e, LIFETIME);
            }
        }
        if let Some(e) = self.character(p, p, true) {
            return e;
        }
        self.tok(p, p + 1, OPERATOR)
    }

    /// A character literal whose `'` is at `q`, its prefix from `p`: where it ends.
    fn character(&mut self, p: usize, q: usize, unicode: bool) -> Option<usize> {
        let (e, esc) = char_body(self.b, q + 1, unicode)?;
        if self.b.get(e) != Some(&b'\'') {
            return None;
        }
        self.put(p, q, CHAR_PREFIX);
        self.put(q, q + 1, CHAR_BEGIN);
        self.put(q + 1, e, if esc { CHAR_ESCAPE } else { CHAR });
        Some(self.token(e, 1, CHAR_END))
    }

    /// A string at `p` with a prefix (`b"`, `br#"`, `r#"`; with `chars`, `b'`), opened: its
    /// text's place, or `None`.
    fn string_open(&mut self, p: usize, chars: bool, fmt: bool) -> Option<usize> {
        let b = self.b;
        if b[p] == b'"' {
            self.push(p, if fmt { FMT_STRING } else { STRING });
            return Some(self.token(p, 1, STR_BEGIN));
        }
        let e = word_end(b, p);
        let next = b.get(e).copied();
        match (&b[p..e], next) {
            (b"b", Some(b'"')) => {
                self.push(p, BYTE_STRING);
                self.put(p, e, STR_PREFIX);
                Some(self.token(e, 1, STR_BEGIN))
            }
            (b"br" | b"r", Some(b'#' | b'"')) => {
                let h = b[e..].iter().take_while(|&&c| c == b'#').count();
                if b.get(e + h) != Some(&b'"') {
                    return None;
                }
                let raw = e - p == 1;
                self.push(
                    p,
                    if !raw {
                        RAW_BYTES
                    } else if fmt {
                        FMT_RAW
                    } else {
                        RAW_STRING
                    },
                );
                self.hashes = h;
                self.put(p, e, RAW_PREFIX);
                if raw {
                    Some(self.token(e, h + 1, RAW_BEGIN))
                } else {
                    self.put(e, e + h, RAW_BEGIN);
                    Some(self.token(e + h, 1, RAW))
                }
            }
            (b"b", Some(b'\'')) if chars => self.character(p, e, false),
            _ => None,
        }
    }

    /// `numbers` at `p`, a digit; floats only with `floats`.
    fn number(&mut self, p: usize, floats: bool) -> Option<usize> {
        let b = self.b;
        let digit = |c: u8| c.is_ascii_digit();
        let r = digits_end(b, p, digit);
        let suffix = |at: usize, s: &[u8]| b[at..].starts_with(s) && ends_word(b, at + s.len());
        if floats {
            if b.get(r) == Some(&b'.') && b[r - 1].is_ascii_digit() {
                if b.get(r + 1).is_some_and(u8::is_ascii_digit) {
                    let mut end = digits_end(b, r + 1, digit);
                    if matches!(b.get(end), Some(b'e' | b'E')) {
                        let mut s = end + 1;
                        if matches!(b.get(s), Some(b'+' | b'-')) {
                            s += 1;
                        }
                        let x = digits_end(b, s, digit);
                        if b[s..x].iter().any(u8::is_ascii_digit) {
                            end = x;
                        }
                    }
                    self.tok(p, end, FLOAT);
                    if has(b, end, b"f32") || has(b, end, b"f64") {
                        return Some(self.tok(end, end + 3, NUMERIC));
                    }
                    return Some(end);
                }
                if b.get(r + 1) != Some(&b'.') {
                    return Some(self.tok(p, r + 1, FLOAT));
                }
            }
            if suffix(r, b"f32") || suffix(r, b"f64") {
                self.tok(p, r, FLOAT);
                return Some(self.tok(r, r + 3, NUMERIC));
            }
            if matches!(b.get(r), Some(b'e' | b'E')) {
                let mut s = r + 1;
                if matches!(b.get(s), Some(b'+' | b'-')) {
                    s += 1;
                }
                let x = digits_end(b, s, digit);
                if b[s..x].iter().any(u8::is_ascii_digit) {
                    if suffix(x, b"f32") || suffix(x, b"f64") {
                        self.tok(p, x, FLOAT);
                        return Some(self.tok(x, x + 3, NUMERIC));
                    }
                    if ends_word(b, x) {
                        return Some(self.tok(p, x, FLOAT));
                    }
                }
            }
        }
        let e = word_end(b, p);
        let int = |s: &mut Self, from: usize, leaf: u16| -> Option<usize> {
            let rest = &b[from..e];
            if !rest.is_empty() && !is_int_suffix(rest) {
                return None;
            }
            if from > p {
                s.tok(p, from, leaf);
            }
            if e > from {
                s.tok(from, e, NUMERIC);
            }
            Some(e)
        };
        if let Some(q) = int(self, r, DECIMAL) {
            return Some(q);
        }
        if b[p] == b'0' {
            let based = |test: fn(u8) -> bool| {
                let d = digits_end(b, p + 2, test);
                b[p + 2..d].iter().any(|&c| test(c)).then_some(d)
            };
            let (d, leaf) = match b.get(p + 1) {
                Some(b'x') => (based(|c| c.is_ascii_hexdigit()), HEX),
                Some(b'o') => (based(|c| matches!(c, b'0'..=b'7')), OCTAL),
                Some(b'b') => (based(|c| matches!(c, b'0' | b'1')), BINARY),
                _ => (None, PLAIN),
            };
            if let Some(d) = d {
                return int(self, d, leaf);
            }
        }
        None
    }

    // --- comments (X-064's) ---

    /// A comment at `p`, a `/`: where its first match ends.
    fn comment(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        match b.get(p + 1) {
            Some(b'*') => {
                if sql::marker_comment(b, p + 2) {
                    self.mark = b[p + 2..]
                        .windows(2)
                        .position(|w| w == b"*/")
                        .map_or(b.len(), |i| p + 4 + i);
                }
                Some(self.open_comment(p))
            }
            Some(b'/') => {
                let doc = matches!(b.get(p + 2), Some(b'!' | b'/'));
                if !doc && sql::marker_comment(b, p + 2) {
                    self.mark = b.len();
                }
                // in code: no text before it to fill, no attribute around, so straight to the
                // sink (the line's end moves `at`)
                let (text, n) = if doc { (DOC, 3) } else { (COMMENT, 2) };
                self.tok(p, p + n, text + MARK);
                let mut from = p + n;
                let mut at = from;
                loop {
                    let d = prim::find_any(b, at, *b"$");
                    if d == b.len() {
                        break;
                    }
                    if let Some(e) = ident_end(b, d + 1) {
                        if d > from {
                            self.tok(from, d, text);
                        }
                        self.tok(d, e, text + VAR);
                        from = e;
                        at = e;
                    } else {
                        at = d + 1;
                    }
                }
                if b.len() > from {
                    self.tok(from, b.len(), text);
                }
                Some(b.len())
            }
            _ => None,
        }
    }

    /// `/*` at `p`: a block comment, or a documentation one.
    fn open_comment(&mut self, p: usize) -> usize {
        let b = self.b;
        let doc = matches!(b.get(p + 2), Some(b'!' | b'*'))
            && b.get(p + 3).is_none_or(|&d| d != b'*' && d != b'/');
        if doc {
            let e = b.get(p + 3).map_or(b.len(), |&d| p + 3 + char_len(d));
            self.push(p, DOC_COMMENT);
            self.token(p, e - p, DOC_BLOCK + MARK)
        } else {
            self.push(p, BLOCK_COMMENT);
            self.token(p, 2, BLOCK_TEXT + MARK)
        }
    }

    fn in_comment(&mut self, top: u8, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        let text = if top == DOC_COMMENT {
            DOC_BLOCK
        } else {
            BLOCK_TEXT
        };
        if c == b'*' && b.get(p + 1) == Some(&b'/') {
            return Some(self.close_n(p, 2, text + MARK));
        }
        if top == DOC_COMMENT && p == 0 {
            let q = skip_spaces(b, 0);
            if b.get(q) == Some(&b'*') && b.get(q + 1) != Some(&b'/') {
                return Some(self.token(q, 1, text + MARK));
            }
        }
        (c == b'/' && b.get(p + 1) == Some(&b'*')).then(|| self.open_comment(p))
    }

    // --- the text frames' turns (X-064's) ---

    /// An attribute's, a string's or a comment's turn at `p`.
    fn text_turn(&mut self, p: usize, top: u8) -> usize {
        match top {
            ATTRIBUTE..=ATTR_CALL => {
                let q = self.attr_run(p);
                if self.top() < ATTRIBUTE {
                    // the attribute closed: an item's start
                    self.prev = P_START;
                }
                q
            }
            STRING..=RAW_BYTES => self.string_run(p, top),
            BLOCK_COMMENT | DOC_COMMENT => self.comment_run(p, top),
            _ => p + 1,
        }
    }

    /// An attribute's frames from `p`, to the line's end or till the top is none of them.
    fn attr_run(&mut self, mut p: usize) -> usize {
        let b = self.b;
        loop {
            let top = self.top();
            if !(ATTRIBUTE..=ATTR_CALL).contains(&top) || p >= b.len() {
                return p;
            }
            let c = b[p];
            if is_space(c) {
                p = skip_spaces(b, p);
                continue;
            }
            if c == b'$'
                && let Some(e) = ident_end(b, p + 1)
            {
                self.put(p, e, VARIABLE);
                p = e;
                continue;
            }
            let r = if top == ATTRIBUTE {
                match c {
                    b'=' => Some(self.token(p, 1, OPERATOR)),
                    b']' => Some(self.close(p, GROUP_END)),
                    _ => match ident_end(b, p) {
                        Some(e) => {
                            self.put(p, e, ANNOTATION);
                            if b.get(e) == Some(&b'(') {
                                self.push(e, ATTR_PARAMS);
                                Some(self.token(e, 1, ANNOTATION_BEGIN))
                            } else {
                                Some(e)
                            }
                        }
                        None => self.string_at(p),
                    },
                }
            } else {
                match c {
                    b')' => {
                        let leaf = if top == ATTR_PARAMS {
                            ANNOTATION_END
                        } else {
                            CALL_END
                        };
                        self.put(p, p + 1, leaf);
                        self.pop(p + 1);
                        Some(p + 1)
                    }
                    b',' => Some(self.token(p, 1, SEPARATOR)),
                    b'=' => Some(self.token(p, 1, OPERATOR)),
                    _ if is_word(c) => {
                        let e = word_end(b, p);
                        let w = &b[p..e];
                        if is_ident(w) && b.get(e) == Some(&b'(') {
                            self.put(p, e, CALL);
                            self.push(e, ATTR_CALL);
                            Some(self.token(e, 1, CALL_BEGIN))
                        } else if matches!(w, b"b" | b"br" | b"r")
                            && let Some(q) = self.string_at(p)
                        {
                            Some(q)
                        } else {
                            Some(e)
                        }
                    }
                    _ => self.string_at(p),
                }
            };
            p = match r {
                Some(q) => q,
                None if is_word(c) => word_end(b, p),
                None => p + 1,
            };
        }
    }

    /// A string at `p` in an attribute (no characters), a regex in it where a call makes one.
    fn string_at(&mut self, p: usize) -> Option<usize> {
        let e = self.string_open(p, false, false)?;
        if b"\"br".contains(&self.b[p]) {
            self.regex_in(p, e);
        }
        Some(e)
    }

    /// A string's text and patterns from `p` (`top` its frame), to the line's end or its close.
    fn string_run(&mut self, mut p: usize, top: u8) -> usize {
        let b = self.b;
        let d = self.depth;
        while p < b.len() {
            let c = b[p];
            let r = match top {
                STRING | FMT_STRING | BYTE_STRING => match c {
                    b'"' => Some(self.close(p, STR_END)),
                    b'\\' => match escape(b, p, top != BYTE_STRING) {
                        Some(e) => Some(self.token(p, e - p, STR_ESCAPE)),
                        None => (top != BYTE_STRING && p + 1 == b.len())
                            .then(|| self.token(p, 1, STR_CONTINUATION)),
                    },
                    b'{' | b'}' if top == FMT_STRING => {
                        self.format_escape(p, c, STR_ESCAPE, STR_PLACEHOLDER)
                    }
                    _ => None,
                },
                _ => {
                    let h = self.hashes;
                    if c == b'"'
                        && b.len() > p + h
                        && b[p + 1..p + 1 + h].iter().all(|&c| c == b'#')
                    {
                        Some(self.close_n(p, 1 + h, RAW_END))
                    } else if top == FMT_RAW && (c == b'{' || c == b'}') {
                        self.format_escape(p, c, RAW_ESCAPE, RAW_PLACEHOLDER)
                    } else {
                        None
                    }
                }
            };
            match r {
                Some(q) => {
                    p = q;
                    if self.depth != d {
                        return p;
                    }
                }
                None => {
                    p = match top {
                        STRING | BYTE_STRING => prim::find_any(b, p + 1, *b"\"\\"),
                        FMT_STRING => prim::find_any(b, p + 1, *b"\"\\{}"),
                        RAW_STRING | RAW_BYTES => prim::find_any(b, p + 1, *b"\""),
                        _ => prim::find_any(b, p + 1, *b"\"{}"),
                    }
                }
            }
        }
        p
    }

    /// A `{` or `}` at `p` in a format string: `{{` or `}}` an escape, a placeholder.
    fn format_escape(&mut self, p: usize, c: u8, escape: u16, place: u16) -> Option<usize> {
        let b = self.b;
        if b.get(p + 1) == Some(&c) {
            return Some(self.token(p, 2, escape));
        }
        if c == b'}' {
            return None;
        }
        let e = placeholder(b, p)?;
        Some(self.token(p, e - p, place))
    }

    /// A block comment's text and patterns from `p`, to the line's end, its close or a nested
    /// one's open.
    fn comment_run(&mut self, mut p: usize, top: u8) -> usize {
        let b = self.b;
        let d = self.depth;
        while p < b.len() {
            let c = b[p];
            if c == b'$'
                && let Some(e) = ident_end(b, p + 1)
            {
                let leaf = if top == DOC_COMMENT {
                    DOC_BLOCK + VAR
                } else {
                    BLOCK_TEXT + VAR
                };
                self.put(p, e, leaf);
                p = e;
                continue;
            }
            match self.in_comment(top, p, c) {
                Some(q) => {
                    p = q;
                    if self.depth != d {
                        return p;
                    }
                }
                None => p = prim::find_any(b, p + 1, *b"*/$"),
            }
        }
        p
    }

    // --- regexes and SQL in strings (X-064's) ---

    #[inline(never)]
    fn regex_in(&mut self, p: usize, e: usize) {
        if self.mark <= p && self.marked(p) {
            if matches!(self.top(), STRING | RAW_STRING) {
                self.sql_in(e);
            }
        } else if regex_string(self.b, p) {
            self.push(e, REGEX);
            self.text = self.string_text();
        }
    }

    #[inline(never)]
    fn sql_name(&mut self, p: usize, e: usize) {
        let b = self.b;
        let mac = b.get(e) == Some(&b'!');
        let Some(st) = sql_call(&b[p..e], mac) else {
            return;
        };
        if p > 0 && b[p - 1] == b'.' && stmt_receiver(b, p - 1) {
            return;
        }
        let q = if mac {
            skip_spaces(b, e + 1)
        } else if has(b, e, b"::<") {
            match turbofish_end(b, e + 2) {
                Some(q) => q,
                None => return,
            }
        } else {
            e
        };
        if b.get(q) == Some(&b'(') {
            self.sql_paren = q;
            self.sql_call = st;
        }
    }

    /// A SQL call's arguments at `p`: the state followed, the SQL argument's string opened as
    /// SQL; else `None`, the token a group's.
    #[cold]
    #[inline(never)]
    fn sql_frame(&mut self, p: usize, c: u8, top: u8) -> Option<usize> {
        let b = self.b;
        if c == b')' {
            self.pop_code();
            self.prev = P_VALUE;
            return Some(self.tok(p, p + 1, META_GROUP_END));
        }
        if !(c == b'/' && matches!(b.get(p + 1), Some(b'/' | b'*'))) {
            let next = match top - F_PSQL {
                PSQL_SQL => {
                    self.set_top(F_GROUP);
                    if (c == b'"' || (c == b'r' && starts_word(b, p)))
                        && let Some(e) = self.sql_string(p)
                    {
                        self.prev = P_VALUE;
                        return Some(e);
                    }
                    F_GROUP
                }
                PSQL_TYPE if c == b'"' => F_GROUP,
                PSQL_TYPE => F_PSQL + PSQL_SKIP,
                _ if c == b',' => F_PSQL + PSQL_SQL,
                _ => top,
            };
            self.set_top(next);
        }
        None
    }

    fn sql_string(&mut self, p: usize) -> Option<usize> {
        let e = self.string_open(p, false, false)?;
        if matches!(self.top(), STRING | RAW_STRING) {
            self.sql_in(e);
        }
        Some(e)
    }

    fn sql_in(&mut self, e: usize) {
        self.mark = usize::MAX;
        self.sql_end = usize::MAX;
        self.sql.is_end = usize::MAX;
        self.push(e, SQL_IN + sql::MAIN);
        self.text = self.string_text();
    }

    #[cold]
    fn marked(&mut self, p: usize) -> bool {
        let holds = sql::marker_holds(self.b, self.mark, p);
        if !holds {
            self.mark = usize::MAX;
        }
        holds
    }

    #[cold]
    fn embed_turn(&mut self, p: usize, top: u8) -> Option<usize> {
        let c = self.b[p];
        if top < REGEX {
            self.in_sql(p, c, top)
        } else {
            self.in_regex(p, c, top)
        }
    }

    #[cold]
    #[inline(never)]
    fn in_sql(&mut self, p: usize, c: u8, top: u8) -> Option<usize> {
        let b = self.b;
        if self.string_top() == RAW_STRING {
            let h = self.hashes;
            if c == b'"' && b.len() > p + h && b[p + 1..p + 1 + h].iter().all(|&c| c == b'#') {
                self.pop(p);
                return Some(self.close_n(p, 1 + h, RAW_END));
            }
        } else {
            match c {
                b'"' => {
                    self.pop(p);
                    return Some(self.close(p, STR_END));
                }
                b'\\' => {
                    if let Some(e) = escape(b, p, true) {
                        return Some(self.token(p, e - p, STR_ESCAPE));
                    }
                    if p + 1 == b.len() {
                        return Some(self.token(p, 1, STR_CONTINUATION));
                    }
                }
                _ => {}
            }
        }
        self.sql_scan(p, top).map(|(q, _)| q)
    }

    fn sql_scan(&mut self, p: usize, top: u8) -> Option<(usize, u8)> {
        let raw = self.string_top() == RAW_STRING;
        let host = sql::Embed {
            yields: &RE_YIELDS[usize::from(raw)],
            base: if raw { SQL_RAW } else { SQL_STRING },
            text: self.text,
        };
        let b = self.b;
        if self.sql_end == usize::MAX {
            self.sql_end = string_end(b, p, raw.then_some(self.hashes));
        }
        let end = self.sql_end;
        self.fill(p);
        self.sql.at = self.at;
        self.k.close(self.out);
        let r = sql::embedded(&b[..end], p, top - SQL_IN, &host, &mut self.sql, self.out);
        self.k.reopen(self.out, 16);
        self.at = self.sql.at;
        let (mut q, ctx) = r?;
        if q > end {
            self.sql_nl |= end == b.len();
            q = end;
        }
        self.set_top(SQL_IN + ctx);
        Some((q, ctx))
    }

    #[cold]
    fn sql_eol(&mut self, top: u8) {
        if !self.sql_nl {
            self.sql_scan(self.b.len(), top);
        }
        if matches!(self.top().wrapping_sub(SQL_IN), sql::DASHES | sql::HASHES) {
            self.set_top(SQL_IN + sql::MAIN);
        }
    }

    #[cold]
    #[inline(never)]
    fn in_regex(&mut self, p: usize, c: u8, top: u8) -> Option<usize> {
        if top < REGEX {
            return None;
        }
        let b = self.b;
        let string = self.string_top();
        let raw = matches!(string, RAW_STRING | RAW_BYTES);
        let h = self.hashes;
        if raw {
            if c == b'"' && b.len() > p + h && b[p + 1..p + 1 + h].iter().all(|&c| c == b'#') {
                self.pop(p);
                return Some(self.close_n(p, 1 + h, RAW_END));
            }
        } else {
            match c {
                b'"' => {
                    self.pop(p);
                    return Some(self.close(p, STR_END));
                }
                b'\\' if b.get(p + 1) != Some(&b'\\') => {
                    if let Some(e) = escape(b, p, string != BYTE_STRING) {
                        return Some(self.token(p, e - p, STR_ESCAPE));
                    }
                    if string != BYTE_STRING && p + 1 == b.len() {
                        return Some(self.token(p, 1, STR_CONTINUATION));
                    }
                }
                _ => {}
            }
        }
        let host = regex::Embed {
            dialect: regex::Dialect::Rust,
            cooked: !raw,
            yields: &RE_YIELDS[usize::from(raw)],
            stop: |b, p, h| {
                b[p] == b'"'
                    && b.len() >= p + 1 + h
                    && b[p + 1..p + 1 + h].iter().all(|&c| c == b'#')
            },
            arg: if raw { h } else { 0 },
            escape: |b, p| escape(b, p, true).unwrap_or(p + 1),
            top_only: false,
            continues: false,
            base: if raw { RE_RAW } else { RE_STRING },
            text: self.text,
        };
        self.k.close(self.out);
        let r = regex::embedded(b, p, top & regex::STATE, &host, &mut self.at, self.out);
        self.k.reopen(self.out, 16);
        let (q, st) = r?;
        self.set_top(REGEX | st);
        Some(q)
    }
}

/// Whether `w` is at `i`: a compare of its `N` bytes, no call to `memcmp`.
#[inline(always)]
fn has<const N: usize>(b: &[u8], i: usize, w: &[u8; N]) -> bool {
    b.len() >= i + N && b[i..i + N] == *w
}

/// Where a name at `p` ends.
#[inline(always)]
fn name_end(b: &[u8], p: usize) -> usize {
    let mut e = p + 1;
    while e < b.len() && NAME[usize::from(b[e])] {
        e += 1;
    }
    e
}

// --- the entry points -------------------------------------------------------------------------

/// Lex `b`, a line without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let mut s = Scan::fresh(b, out, Spans::new());
    s.load(state);
    s.k.open(s.out, b.len() / 4 + 16);
    s.lex_line();
    s.k.close(s.out);
    *state = s.keep();
}

/// [`line()`]'s runs by theme rule pushed to `runs` (X-068).
#[inline(never)]
pub fn line_runs(
    b: &[u8],
    state: &mut State,
    out: &mut Out,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
) {
    let mut s = Scan::fresh(b, out, Runs::new(rules, runs));
    s.load(state);
    s.lex_line();
    *state = s.keep();
    s.k.end_line(b.len());
}

/// A block's lines on one scan opened from `state` ([`crate::Carry`]).
pub fn block(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    let mut out = Out::new(spans);
    out.0.clear();
    let mut s = Scan::fresh(b"", &mut out, Spans::new());
    s.load(state);
    s.k.open(s.out, 64);
    let rest = pass(&mut s);
    *state = s.keep();
    rest
}

/// [`block`]'s runs by theme rule pushed to `runs`, the lines' loop compiled with the scan.
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
    s.load(state);
    let rest = lines.pass(&mut s);
    *state = s.keep();
    rest
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
        self.lex_next();
        self.b = b"";
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::marked;
    use crate::{Lang, State};

    fn of(lines: &[&str]) -> Vec<(String, String)> {
        marked(Lang::Rust, lines)
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
    fn regexes_in_strings() {
        const RE: &str = "string.quoted.double.rust";
        const RAW: &str = "string.quoted.double.raw.rust";
        let l = [
            r##"let a = Regex::new("^(?<y>\\d+)\n[\\pL--\\p{Greek}&&[^x]]\"$x\"##,
            r#"  (x|y)\\b{start}x$").unwrap();"#,
            r##"let b = regex!(r#"["a-z]+"(?x)\x{41}\u{1F600}\<"#);"##,
            r#"let c = RegexBuilder::new(r"(?P<a.b>\p{sc!=Greek})[a-g~~b-h]");"#,
            r#"println!("^\\d+ {x}"); MyRegex::new("a+")"#,
        ];
        has(&l, "^", &format!("{RE} keyword.control.anchor.regexp"));
        has(&l, "y", &format!("{RE} entity.name.other.group.regexp"));
        has(
            &l,
            r"\\d",
            &format!("{RE} constant.other.character-class.escape.backslash.regexp"),
        );
        has(&l, r"\n", &format!("{RE} constant.character.escape.rust"));
        has(
            &l,
            "--",
            &format!(
                "{RE} constant.other.character-class.set.regexp keyword.operator.difference.regexp"
            ),
        );
        has(
            &l,
            "&&",
            &format!(
                "{RE} constant.other.character-class.set.regexp keyword.operator.intersection.regexp"
            ),
        );
        // the prototype's `$x` doesn't reach a regex; a line continued
        has(&l, "$", &format!("{RE} keyword.control.anchor.regexp"));
        has(
            &l,
            "\\",
            &format!("{RE} punctuation.separator.continuation.line.rust"),
        );
        has(&l, "|", &format!("{RE} keyword.operator.or.regexp"));
        has(
            &l,
            r"\\b{start}",
            &format!("{RE} keyword.control.anchor.regexp"),
        );
        // a raw string: its `"` aren't its end but before its `#`s
        has(
            &l,
            "\"",
            &format!("{RAW} constant.other.character-class.set.regexp"),
        );
        has(
            &l,
            "?x",
            &format!("{RAW} keyword.other.option-toggle.regexp"),
        );
        has(
            &l,
            r"\u{1F600}",
            &format!("{RAW} constant.character.escape.backslash.regexp"),
        );
        has(&l, r"\<", &format!("{RAW} keyword.control.anchor.regexp"));
        has(
            &l,
            "\"#",
            &format!("{RAW} punctuation.definition.string.end.rust"),
        );
        has(&l, "a.b", &format!("{RAW} entity.name.other.group.regexp"));
        has(
            &l,
            "!=",
            &format!("{RAW} punctuation.separator.key-value.unicode-property.regexp"),
        );
        has(
            &l,
            "~~",
            &format!(
                "{RAW} constant.other.character-class.set.regexp keyword.operator.symmetric-difference.regexp"
            ),
        );
        // not a regex: any other call
        let got = of(&l[4..]);
        assert!(!got.iter().any(|(_, s)| s.contains("regexp")), "{got:?}");
    }

    #[test]
    fn sql_in_strings() {
        const DQ: &str = "string.quoted.double.rust";
        const RAW: &str = "string.quoted.double.raw.rust";
        let dml = |s: &str| format!("{s} keyword.other.DML.sql");
        let l = [
            "// language=SQL",
            r#"const Q: &str = "SELECT a FROM t WHERE b = '{x}'\n";"#,
            r#"let rows = sqlx::query!("DELETE FROM u WHERE id = $1", id)"#,
            r##"    .fetch_all(&pool).await?; diesel::sql_query(r#"UPDATE "u" SET n = 2"#);"##,
            "let n = sqlx::query_as!(",
            "    User,",
            r##"    r#"INSERT INTO u -- a note"##,
            r##"    VALUES (1)"#, id"##,
            ");",
            r#"conn.execute("CREATE TABLE p (id INTEGER)", params![]);"#,
            r#"let s = query_scalar::<_, i64>("UNION ALL SELECT 1");"#,
            r#"stmt.execute(["LIMIT"]); stmt.query("ORDER"); f("HAVING");"#,
            r#"conn.execute(&format!("JOIN {}", t), []); query_as!(User, "WHERE")"#,
            r#"query_as::<_, User>("GROUP BY x")"#,
        ];
        // a marker's string; the host's escape between SQL's tokens, `{x}` SQL's text
        has(&l, "SELECT", &dml(DQ));
        has(&l, r"\n", &format!("{DQ} constant.character.escape.rust"));
        has(&l, "{x}", &format!("{DQ} string.quoted.single.sql"));
        // a macro's and a function's first argument, a raw string's `"` SQL's
        has(&l, "DELETE", &dml(DQ));
        has(&l, "UPDATE", &dml(RAW));
        has(&l, "u", &format!("{RAW} string.quoted.double.sql"));
        // `query_as!`'s after its type, over lines; SQL's line comment ends with its line
        has(&l[4..9], "INSERT INTO", &dml(RAW));
        has(
            &l,
            " a note",
            &format!("{RAW} comment.line.double-dash.sql"),
        );
        has(
            &l[4..9],
            "VALUES",
            &format!("{RAW} keyword.other.DML.II.sql"),
        );
        has(
            &l,
            "CREATE",
            &format!("{DQ} meta.create.sql keyword.other.create.sql"),
        );
        // a turbofish
        has(&l, "UNION ALL", &dml(DQ));
        has(&l, "GROUP BY", &dml(DQ));
        has(&l, "WHERE", &dml(DQ));
        // a prepared statement's arguments, any other call, a string in a call in the argument
        for w in ["LIMIT", "ORDER", "HAVING", "JOIN"] {
            assert!(
                of(&l)
                    .iter()
                    .any(|(t, s)| t.contains(w) && !s.contains(".sql")),
                "{w}: {:#?}",
                of(&l)
            );
        }
    }

    #[test]
    fn items() {
        let l = ["pub fn main<T>(x: u32, self) -> Option<T> where T: Copy {"];
        has(&l, "pub", "storage.modifier.rust");
        has(&l, "fn", "storage.type.function.rust");
        has(&l, "main", "entity.name.function.rust");
        has(&l, "x", "variable.parameter.rust");
        has(&l, "u32", "storage.type.rust");
        has(&l, "self", "variable.parameter.rust");
        has(&l, "Option", "support.type.rust");
        has(&l, "where", "keyword.other.rust");
        let l = ["impl<'a> fmt::Display for Foo<'a> {"];
        has(&l, "impl", "storage.type.impl.rust");
        has(&l, "for", "keyword.other.rust");
        has(&l, "Foo", "entity.name.impl.rust");
        has(&l, "'a", "storage.modifier.lifetime.rust");
        let l = [
            "struct S {",
            "    pub a: Vec<u8>,",
            "    b: &'static str,",
            "}",
        ];
        has(&l, "S", "entity.name.struct.rust");
        has(&l, "a", "variable.other.member.rust");
        has(&l, "b", "variable.other.member.rust");
        has(&l, ":", "punctuation.separator.type.rust");
    }

    /// A float with an exponent and no dot is a float, which bat's grammar leaves plain (X-008):
    /// with a sign, underscores and `f32`/`f64`; an integer suffix or no exponent digit isn't one.
    #[test]
    fn floats_without_a_dot() {
        let l = ["1e3 + 1E-3 + 2e10f64 + 1_000e+3_ + 0e0 + 1e3_f32 + 0x1e3"];
        for f in ["1e3", "1E-3", "2e10", "1_000e+3_", "0e0", "1e3_"] {
            has(&l, f, "constant.numeric.float.rust");
        }
        has(&l, "f64", "storage.type.numeric.rust");
        has(&l, "f32", "storage.type.numeric.rust");
        has(&l, "0x1e3", "constant.numeric.integer.hexadecimal.rust");
        let got = of(&["1e; 1e3u8; 1e+"]);
        assert!(!got.iter().any(|(_, s)| s.contains("numeric")), "{got:#?}");
    }

    #[test]
    fn strings_and_comments() {
        let l = [
            r##"let s = r#"raw"#; let b = b"\x00"; let c = '\n';"##,
            "println!(\"{} {x:?} {{}}\", a); // $x note",
            "/** doc",
            "  * star",
            "  */ let x = \"multi",
            "line\";",
        ];
        has(
            &l,
            "r",
            "string.quoted.double.raw.rust storage.type.string.rust",
        );
        has(
            &l,
            "#\"",
            "string.quoted.double.raw.rust punctuation.definition.string.begin.rust",
        );
        has(
            &l,
            "\\x00",
            "string.quoted.double.rust constant.character.escape.rust",
        );
        has(
            &l,
            "\\n",
            "string.quoted.single.rust constant.character.escape.rust",
        );
        has(
            &l,
            "{}",
            "string.quoted.double.rust constant.other.placeholder.rust",
        );
        has(
            &l,
            "{x:?}",
            "string.quoted.double.rust constant.other.placeholder.rust",
        );
        has(
            &l,
            "{{}}",
            "string.quoted.double.rust constant.character.escape.rust",
        );
        has(
            &l,
            "$x",
            "comment.line.double-slash.rust variable.other.rust",
        );
        has(
            &l,
            "/** ",
            "comment.block.documentation.rust punctuation.definition.comment.rust",
        );
        has(
            &l,
            "*",
            "comment.block.documentation.rust punctuation.definition.comment.rust",
        );
        has(&l, "line", "string.quoted.double.rust");
    }

    #[test]
    fn attributes_and_closures() {
        let l = [
            "#[cfg(not(feature = \"x\"))]",
            "let f = |a, b: u8| a + b;",
            "xs.iter().map(|x| x * 2)",
        ];
        has(&l, "cfg", "meta.annotation.rust variable.annotation.rust");
        has(
            &l,
            "not",
            "meta.annotation.rust meta.function-call.rust variable.function.rust",
        );
        has(&l, "feature ", "meta.annotation.rust");
        has(&l, "f", "entity.name.function.rust");
        has(&l, "a", "variable.parameter.rust");
        has(&l, "x", "variable.parameter.rust");
        has(&l, "map", "support.function.rust");
    }

    /// A block lexed in one call lexes as its lines one at a time: the scan kept across them
    /// ends each line as a state would, a stack deeper than a state holds cut at its bottom.
    #[test]
    fn a_block_lexes_as_its_lines() {
        let deep = format!("{}\n{}\n", "{(".repeat(20), "[x, ".repeat(9));
        let text = format!(
            "{deep}let s = r##\"a\n\"#\"##; // language=SQL\nlet q =\n\"SELECT 1\";\n#[a(\nb)] struct S(u8,\nu16);\n\
             fn f<T>(\n  a: Vec<\n  u8>,\n  (b, c): (u8, u8),\n) -> Option<\nT> where T: Copy\n{{ let g = |x: u8,\n y| x; }}\n\
             macro_rules! m {{ ($x:expr, $($y:ident),*) => {{ $x }}; }}\n/* a /* nested\n*/ still */ Regex::new(\"a\n(b)\");\n\
             impl<T> a::B<T>\nfor c::D<T>\n{{}}\n{}",
            "})".repeat(25)
        );
        let (mut want, mut state, mut spans) = (Vec::new(), State::default(), Vec::new());
        for line in text.split('\n') {
            crate::lex(Lang::Rust, line, &mut state, &mut spans);
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
                    Lang::Rust,
                    t,
                    max,
                    &mut state,
                    (&mut spans, &mut buf),
                    |_, _| false,
                    |l, s| got.push((l.to_owned(), s.to_vec())),
                );
                // a call's state is its last line's
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
            assert_eq!(got, lines, "max {max}");
        }
    }

    /// The type frames, ada's own (X-056): a generic's bounds with `?Sized` stay a generic (bat's
    /// grammar marked the `?` invalid and read on as an impl's name), a nested `>` ends, a
    /// const generic's block and a macro in a type hold code.
    #[test]
    fn types() {
        let l = ["impl<T: ?Sized + Message> private::Sealed for *const T {}"];
        has(&l, "?", "keyword.operator.rust");
        has(&l, "Sized", "support.type.rust");
        has(&l, "for", "keyword.other.rust");
        has(&l, ">", "punctuation.definition.generic.end.rust");
        let l = ["fn f<R: Rng + ?Sized>(&self, rng: &mut R) -> Vec<Option<u8>> {}"];
        has(&l, "self", "variable.parameter.rust");
        has(&l, "rng", "variable.parameter.rust");
        has(&l, ">>", "punctuation.definition.generic.end.rust");
        let l = ["let x = Iso8601::<{ CONFIG }>; let t = p.parse::<Token![=]>()?;"];
        has(&l, "CONFIG", "constant.other.rust");
        has(&l, "Token!", "support.macro.rust");
        has(&l, "=", "keyword.operator.assignment.rust");
        let l = [
            "struct S<'a, #[may_dangle] T: 'a> {",
            "    f: Option<(",
            "        u32, // n",
            "    )>,",
            "}",
        ];
        has(
            &l,
            "may_dangle",
            "meta.annotation.rust variable.annotation.rust",
        );
        has(&l, " n", "comment.line.double-slash.rust");
    }

    /// A tuple struct's fields are a type frame (X-063): its own `)` ends them, not the first
    /// in a type inside (bat's `with_prototype`), a field's attribute is one, `pub` a modifier;
    /// a macro's `struct $name` is named, and a brace ends fields a misread header opened.
    /// The headers on frames (X-064): a struct's `where`, and a trait's or an enum's generics,
    /// supertraits and `where`, read as a function's are (bat's grammar leaves them plain); a
    /// header misread from a macro's words ends at a closer, so the block around it still does.
    #[test]
    fn headers() {
        let l = ["struct S<T> where T: Copy { a: T }"];
        has(&l, "where", "keyword.other.rust");
        has(&l, "a", "variable.other.member.rust");
        let l = ["trait Tr<T>: Clone + Send where Self: Sized {}"];
        has(&l, "<", "punctuation.definition.generic.begin.rust");
        has(&l, ":", "punctuation.separator.rust");
        has(&l, "where", "keyword.other.rust");
        let l = ["enum E<T> { A(T) }"];
        has(&l, ">", "punctuation.definition.generic.end.rust");
        // serde's `forward_to_deserialize_any!`: `struct enum` ends at the block's `}`
        let l = ["m! { map struct enum identifier }", "fn f() {}"];
        has(&l, "f", "entity.name.function.rust");
    }

    /// A `macro_rules!` matcher's groups balanced (X-064; bat's ends at the first `)` in it), a
    /// repetition inside a repetition, a matcher in `[…]`, a string and a comment in one.
    #[test]
    fn matchers() {
        let l = [
            "macro_rules! m {",
            "    (($a:expr) $( $( $b:ident ),* );* \")\" // x",
            "    ) => { $a };",
            "    [$c:tt] => {};",
            "}",
            "fn f() {}",
        ];
        has(&l, "$a", "variable.parameter.macro.rust");
        has(&l, "expr", "storage.type.rust");
        has(&l, "$b", "variable.parameter.macro.rust");
        has(&l, ")", "string.quoted.double.rust");
        has(&l, " x", "comment.line.double-slash.rust");
        has(&l, "$c", "variable.parameter.macro.rust");
        has(&l, "tt", "storage.type.rust");
        has(&l, "f", "entity.name.function.rust");
    }

    #[test]
    fn tuple_fields() {
        let l = ["pub struct W(Box<dyn Fn(u8) -> u8>, u32);"];
        has(&l, ">", "punctuation.definition.generic.end.rust");
        let l = ["struct M<K, V>(PhantomData<(K, V)>);"];
        has(&l, ">", "punctuation.definition.generic.end.rust");
        let l = ["struct A(#[serde(skip)] pub u8, String);"];
        has(&l, "serde", "meta.annotation.rust variable.annotation.rust");
        has(&l, "pub", "storage.modifier.rust");
        has(&l, "u8", "storage.type.rust");
        let l = [
            "macro_rules! m { ($name:ident) => {",
            "    pub struct $name {",
            "        message: &'static str,",
            "    }",
            "}}",
        ];
        has(&l, "message", "variable.other.member.rust");
        let l = ["struct S", "$(impl X {", "    fn f() {}", "})*"];
        has(&l, "f", "entity.name.function.rust");
    }

    #[test]
    fn state_carries_raw_strings_comments_and_frames() {
        let l = [
            "let s = r##\"a",
            "\"# still",
            "end\"##; fn f(",
            "  a: u8,",
            ") {} /* a",
            "/* b */ c",
            "*/ x",
        ];
        has(&l, "\"# still", "string.quoted.double.raw.rust");
        has(&l, "a", "variable.parameter.rust");
        // a nested comment's own marks, and the outer one's text after it
        has(&l, " b ", "comment.block.rust");
        has(&l, " c", "comment.block.rust");
    }

    /// Names by their place: a function's, a parameter's (a tuple pattern's too), a field's, an
    /// impl's past `for`, a closure's binding and parameters, a constant's and a C type's by their
    /// bytes; a path's call plain.
    #[test]
    fn names_by_their_place() {
        let l = [
            "pub fn f<T: Clone>(a: &mut T, (b, c): (u8, u8)) -> Result<Self, E> where T: Sized {}",
        ];
        has(&l, "f", "entity.name.function.rust");
        has(&l, "a", "variable.parameter.rust");
        has(&l, "b", "variable.parameter.rust");
        has(&l, "Clone", "support.type.rust");
        has(&l, "Result", "support.type.rust");
        let l = ["struct S { pub a: u8, b: Vec<String> } impl fmt::Debug for S {}"];
        has(&l, "a", "variable.other.member.rust");
        has(&l, "S", "entity.name.impl.rust");
        let l = ["let c = |x, y: u8| x + y; let d = Foo::new(); FOO_BAR; c_int;"];
        has(&l, "c", "entity.name.function.rust");
        has(&l, "x", "variable.parameter.rust");
        has(&l, "FOO_BAR", "constant.other.rust");
        has(&l, "c_int", "storage.type.rust");
        assert!(
            !of(&l).iter().any(|(t, _)| t == "new"),
            "a path's call plain"
        );
    }

    /// The token before: a generic's `<` after `=`, `(`, `,` (bat's `after-operator`), else a
    /// comparison; a closure's `|` wherever an expression starts, else an operator.
    #[test]
    fn closures_and_generics_by_the_token_before() {
        let l = ["let a = move |x| x; f(|y| y); let z = i<n; w = a < b; x = 1 << 3;"];
        has(&l, "x", "variable.parameter.rust");
        has(&l, "y", "variable.parameter.rust");
        has(&l, "<", "punctuation.definition.generic.begin.rust");
        has(&l, "<", "keyword.operator.comparison.rust");
        has(&l, "<<", "keyword.operator.comparison.rust");
    }

    /// The scopes of `text`'s tokens in `line`, in order.
    fn scopes_of(line: &str, text: &str) -> Vec<String> {
        of(&[line])
            .into_iter()
            .filter(|(t, _)| t == text)
            .map(|(_, s)| s)
            .collect()
    }

    /// A `|` with no left operand opens a closure: after `move`, `&`, `[`, an operator, a
    /// keyword that takes an expression (X-117).
    #[test]
    fn closures_where_an_expression_starts() {
        for line in [
            "thread::spawn(move |a| a);",
            "let f = &|a| a;",
            "let v = [|a| a];",
            "let g = x + |a| a;",
            "if c { break |a| a; }",
            "let h = async move |a| a;",
        ] {
            assert_eq!(
                scopes_of(line, "a").first().map(String::as_str),
                Some("variable.parameter.rust"),
                "{line}: {:#?}",
                of(&[line])
            );
            assert!(
                !of(&[line])
                    .iter()
                    .any(|(t, s)| t == "|" && s.contains("bitwise")),
                "{line}: {:#?}",
                of(&[line])
            );
        }
        // no parameters: `||` opens and closes them, not a logical or
        for line in ["thread::spawn(move || run());", "let v = [|| 1];"] {
            assert!(
                !of(&[line]).iter().any(|(_, s)| s.contains("logical")),
                "{line}: {:#?}",
                of(&[line])
            );
        }
    }

    /// A `|` after a value stays an operator: after a name, a closer, `?`, a generic's `>`, and
    /// after a block's `}` (`if c { a } else { b } | m`).
    #[test]
    fn pipes_after_a_value() {
        for (line, leaf) in [
            ("let m = a | b;", "bitwise"),
            ("let m = f()? | b;", "bitwise"),
            ("let m = x? || y;", "logical"),
            ("let m = v[0] | b;", "bitwise"),
            ("let m = if c { a } else { b } | k;", "bitwise"),
            ("let m = a<(b) | c;", "bitwise"),
            ("match x { A | B => 1 }", "bitwise"),
            ("tag!(<a> | <b>)", "bitwise"),
        ] {
            assert!(
                of(&[line])
                    .iter()
                    .any(|(t, s)| t.starts_with('|') && s.contains(leaf)),
                "{line}: {:#?}",
                of(&[line])
            );
            assert!(
                !of(&[line])
                    .iter()
                    .any(|(_, s)| s.contains("variable.parameter")),
                "{line}: {:#?}",
                of(&[line])
            );
        }
    }
}
