//! PHP on a scanner of ada's own (X-099). The floor probe's shape (`x099`'s `sol`): one loop over a
//! line's bytes, a byte's action one load from a table, a name read once and looked up once (the
//! keywords; a call's name in the support functions, a bare name in the support constants), the
//! token before in one byte ([`Scan::prev`]: also where a type, a class's head, a `use` path or a
//! function's name stands), and frames only for what nests: blocks, a class's body, groups,
//! calls, parameters, brackets, a string's `{$ … }` ([`Scan::frames`], four bits a level).
//! Strings, comments and here-documents continued past a line's end, and the HTML outside
//! `<?php … ?>`, are a mode ([`Scan::mode`]). Kinds are bat's leaves under `source.php`.

mod words;

use crate::emit::{Rules, Run, Runs, Sink, Spans};
use crate::heredoc::{Label, Lead, Tail};
use crate::prim::{WORD, char_len};
use crate::words::{Words, text_len};
use crate::{Out, State};

/// The leaves: a token's scopes under `source.php`, but [`HTML`]'s; the support functions' and
/// constants' are [`words::FUNC_GROUPS`]' and [`words::CONST_GROUPS`]'.
static LEAVES: [&str; SUPPORT_FUNCTION as usize] = [
    "",
    // HTML's text outside PHP's tags (no HTML lexed yet)
    "",
    "punctuation.section.embedded.begin.php",
    "punctuation.section.embedded.end.php",
    // comments
    "comment.line.double-slash.php",
    "comment.line.double-slash.php punctuation.definition.comment.php",
    "comment.line.number-sign.php",
    "comment.line.number-sign.php punctuation.definition.comment.php",
    "comment.block.php",
    "comment.block.php punctuation.definition.comment.begin.php",
    "comment.block.php punctuation.definition.comment.end.php",
    "comment.block.documentation.phpdoc.php",
    "comment.block.documentation.phpdoc.php punctuation.definition.comment.begin.php",
    "comment.block.documentation.phpdoc.php punctuation.definition.comment.end.php",
    "comment.block.documentation.phpdoc.php punctuation.definition.comment.php",
    "comment.block.documentation.phpdoc.php keyword.other.phpdoc.php",
    "comment.block.documentation.phpdoc.php keyword.other.phpunit.php",
    // strings: the text, then BEGIN, END, ESCAPE
    "string.quoted.single.php",
    "string.quoted.single.php punctuation.definition.string.begin.php",
    "string.quoted.single.php punctuation.definition.string.end.php",
    "string.quoted.single.php constant.character.escape.php",
    "string.quoted.double.php",
    "string.quoted.double.php punctuation.definition.string.begin.php",
    "string.quoted.double.php punctuation.definition.string.end.php",
    "string.quoted.double.php constant.character.escape.php",
    "string.interpolated.php",
    "string.interpolated.php punctuation.definition.string.begin.php",
    "string.interpolated.php punctuation.definition.string.end.php",
    "string.interpolated.php constant.character.escape.php",
    "string.unquoted.heredoc.php",
    "string.unquoted.heredoc.php punctuation.definition.string.php",
    "string.unquoted.heredoc.php punctuation.definition.string.php keyword.operator.heredoc.php",
    "string.unquoted.heredoc.php constant.character.escape.php",
    "string.unquoted.nowdoc.php",
    "string.unquoted.nowdoc.php punctuation.definition.string.php",
    "string.unquoted.nowdoc.php punctuation.definition.string.php keyword.operator.nowdoc.php",
    "punctuation.section.embedded.end.php keyword.operator.heredoc.php",
    // a string's interpolation: an escape's forms, a variable, an expression's braces
    "constant.character.escape.octal.php",
    "constant.character.escape.hex.php",
    "constant.character.escape.unicodepoint.php",
    "variable.other.php punctuation.definition.variable.php",
    "variable.other.php",
    "variable.other.member.php",
    "punctuation.accessor.arrow.php",
    "punctuation.definition.expression.php",
    "punctuation.definition.variable.php",
    // variables
    "variable.other.php punctuation.definition.variable.php",
    "variable.other.php",
    "variable.language.php punctuation.definition.variable.php",
    "variable.language.php",
    "variable.parameter.php punctuation.definition.variable.php",
    "variable.parameter.php",
    "variable.other.global.php punctuation.definition.variable.php",
    "variable.other.global.php",
    "variable.other.global.safer.php punctuation.definition.variable.php",
    "variable.other.global.safer.php",
    "variable.other.member.php",
    "variable.other.member.php punctuation.definition.variable.php",
    "variable.other.class.php",
    "variable.other.class.php punctuation.definition.variable.php",
    "variable.function.php",
    "variable.language.php",
    // names
    "entity.name.function.php",
    "entity.name.function.php support.function.magic.php",
    "entity.name.class.php",
    "entity.name.interface.php",
    "entity.name.trait.php",
    "entity.name.namespace.php",
    "entity.name.namespace.php support.other.namespace.php",
    "entity.name.namespace.php punctuation.separator.namespace.php",
    "support.class.php",
    "support.class.builtin.php",
    "support.class.exception.php",
    "entity.other.inherited-class.php",
    "entity.other.inherited-class.php support.class.builtin.php",
    "support.other.namespace.php",
    "punctuation.separator.namespace.php",
    "constant.other.php",
    "constant.other.class.php",
    "constant.class.php",
    "constant.language.php",
    "entity.name.label.php",
    "support.other.php",
    "support.function.php",
    "support.function.construct.php",
    // keywords
    "keyword.control.php",
    "keyword.control.exception.php",
    "keyword.control.exception.catch.php",
    "keyword.control.goto.php",
    "keyword.control.import.include.php",
    "storage.type.php",
    "storage.type.nullable.php",
    "storage.type.function.php",
    "storage.type.class.php",
    "storage.type.interface.php",
    "storage.type.trait.php",
    "storage.modifier.php",
    "storage.modifier.abstract.php",
    "storage.modifier.extends.php",
    "storage.modifier.implements.php",
    "storage.modifier.reference.php",
    "keyword.other.new.php",
    "keyword.other.use.php",
    "keyword.other.use-as.php",
    "keyword.other.insteadof.php",
    "keyword.other.function.use.php",
    "keyword.other.namespace.php",
    "keyword.operator.type.php",
    "keyword.operator.logical.php",
    // operators
    "keyword.operator.assignment.php",
    "keyword.operator.assignment.augmented.php",
    "keyword.operator.key.php",
    "keyword.operator.comparison.php",
    "keyword.operator.bitwise.php",
    "keyword.operator.arithmetic.php",
    "keyword.operator.increment-decrement.php",
    "keyword.operator.string.php",
    "keyword.operator.spread.php",
    "keyword.operator.error-control.php",
    "punctuation.accessor.arrow.php",
    "punctuation.accessor.double-colon.php",
    // punctuation
    "punctuation.terminator.expression.php",
    "punctuation.separator.php",
    "punctuation.separator.catch.php",
    "punctuation.section.group.begin.php",
    "punctuation.section.group.end.php",
    "punctuation.section.brackets.begin.php",
    "punctuation.section.brackets.end.php",
    "punctuation.section.array.begin.php",
    "punctuation.section.array.end.php",
    "punctuation.section.block.begin.php",
    "punctuation.section.block.end.php",
    "punctuation.section.block.php",
    // numbers
    "constant.numeric.integer.decimal.php",
    "constant.numeric.float.decimal.php",
    "constant.numeric.integer.hexadecimal.php",
    "constant.numeric.integer.hexadecimal.php punctuation.definition.numeric.hexadecimal.php",
    "constant.numeric.integer.binary.php",
    "constant.numeric.integer.binary.php punctuation.definition.numeric.binary.php",
    // a here-document's opener whose label names a language (`<<<SQL`…)
    "punctuation.section.embedded.begin.php punctuation.definition.string.php",
    "punctuation.section.embedded.begin.php punctuation.definition.string.php keyword.operator.heredoc.php",
    "punctuation.section.embedded.begin.php punctuation.definition.string.php keyword.operator.nowdoc.php",
    "keyword.operator.heredoc.php",
    "keyword.operator.nowdoc.php",
    // an attribute (PHP 8), scoped as Rust's
    "meta.annotation.php punctuation.definition.annotation.begin.php",
    "meta.annotation.php punctuation.definition.annotation.end.php",
    "meta.annotation.php variable.annotation.php",
    // a regex string's delimiters (`'/…/i'`)
    "string.quoted.single.php punctuation.definition.string.regex-delimiter.begin.php",
    "string.quoted.single.php punctuation.definition.string.regex-delimiter.end.php",
    "string.quoted.double.php punctuation.definition.string.regex-delimiter.begin.php",
    "string.quoted.double.php punctuation.definition.string.regex-delimiter.end.php",
    // phpdoc's `@access` value
    "comment.block.documentation.phpdoc.php storage.modifier.php",
    "comment.block.documentation.phpdoc.php invalid.illegal.wrong-access-type.phpdoc.php",
];

const PLAIN: u16 = 0;
const HTML: u16 = 1;
const EMBED_BEGIN: u16 = 2;
const EMBED_END: u16 = 3;
const SLASH_TEXT: u16 = 4;
const SLASH_MARK: u16 = 5;
const HASH_TEXT: u16 = 6;
const HASH_MARK: u16 = 7;
const BLOCK_TEXT: u16 = 8;
const BLOCK_BEGIN: u16 = 9;
const BLOCK_END: u16 = 10;
const DOC_TEXT: u16 = 11;
const DOC_BEGIN: u16 = 12;
const DOC_END: u16 = 13;
const DOC_STAR: u16 = 14;
const DOC_TAG: u16 = 15;
const DOC_UNIT: u16 = 16;
const SINGLE: u16 = 17;
// a string's text, then these
const BEGIN: u16 = 1;
const END: u16 = 2;
const ESCAPE: u16 = 3;
const DOUBLE: u16 = 21;
const BACKTICK: u16 = 25;
const HEREDOC: u16 = 29;
const HEREDOC_OPEN: u16 = 30;
const HEREDOC_LABEL: u16 = 31;
const HEREDOC_ESCAPE: u16 = 32;
const NOWDOC: u16 = 33;
const NOWDOC_OPEN: u16 = 34;
const NOWDOC_LABEL: u16 = 35;
const HEREDOC_CLOSE: u16 = 36;
const ESCAPE_OCTAL: u16 = 37;
const ESCAPE_HEX: u16 = 38;
const ESCAPE_UNICODE: u16 = 39;
const IN_SIGIL: u16 = 40;
const IN_VARIABLE: u16 = 41;
const IN_MEMBER: u16 = 42;
const IN_ARROW: u16 = 43;
const IN_EXPRESSION: u16 = 44;
const IN_VAR_BRACE: u16 = 45;
const SIGIL: u16 = 46;
const VARIABLE: u16 = 47;
const THIS_SIGIL: u16 = 48;
const THIS: u16 = 49;
const PARAM_SIGIL: u16 = 50;
const PARAM: u16 = 51;
const GLOBAL_SIGIL: u16 = 52;
const GLOBAL: u16 = 53;
const SAFER_SIGIL: u16 = 54;
const SAFER: u16 = 55;
const MEMBER: u16 = 56;
const MEMBER_SIGIL: u16 = 57;
const CLASS_VAR: u16 = 58;
const CLASS_VAR_SIGIL: u16 = 59;
const CALL: u16 = 60;
const LANGUAGE: u16 = 61;
const FUNCTION_NAME: u16 = 62;
const MAGIC: u16 = 63;
const CLASS_NAME: u16 = 64;
const INTERFACE_NAME: u16 = 65;
const TRAIT_NAME: u16 = 66;
const NS_DECL: u16 = 67;
const NS_DECL_PART: u16 = 68;
const NS_DECL_SEP: u16 = 69;
const SUPPORT_CLASS: u16 = 70;
const BUILTIN_CLASS: u16 = 71;
const EXCEPTION_CLASS: u16 = 72;
const INHERITED: u16 = 73;
const INHERITED_BUILTIN: u16 = 74;
const NAMESPACE: u16 = 75;
const NS_SEP: u16 = 76;
const CONSTANT: u16 = 77;
const CLASS_CONSTANT: u16 = 78;
const CLASS_KEYWORD: u16 = 79;
const LANG_CONSTANT: u16 = 80;
const LABEL: u16 = 81;
const GOTO_LABEL: u16 = 82;
const USE_FUNCTION: u16 = 83;
const CONSTRUCT: u16 = 84;
const CONTROL: u16 = 85;
const EXCEPTION: u16 = 86;
const CATCH: u16 = 87;
const GOTO: u16 = 88;
const INCLUDE: u16 = 89;
const STORAGE_TYPE: u16 = 90;
const NULLABLE: u16 = 91;
const FUNCTION_KEYWORD: u16 = 92;
const CLASS_TYPE: u16 = 93;
const INTERFACE_TYPE: u16 = 94;
const TRAIT_TYPE: u16 = 95;
const MODIFIER: u16 = 96;
const ABSTRACT: u16 = 97;
const EXTENDS: u16 = 98;
const IMPLEMENTS: u16 = 99;
const REFERENCE: u16 = 100;
const NEW: u16 = 101;
const USE: u16 = 102;
const USE_AS: u16 = 103;
const INSTEADOF: u16 = 104;
const FN_USE: u16 = 105;
const NAMESPACE_KEYWORD: u16 = 106;
const TYPE_OP: u16 = 107;
const LOGICAL: u16 = 108;
const ASSIGNMENT: u16 = 109;
const AUGMENTED: u16 = 110;
const KEY: u16 = 111;
const COMPARISON: u16 = 112;
const BITWISE: u16 = 113;
const ARITHMETIC: u16 = 114;
const INC_DEC: u16 = 115;
const CONCAT: u16 = 116;
const SPREAD: u16 = 117;
const ERROR_CONTROL: u16 = 118;
const ARROW: u16 = 119;
const DOUBLE_COLON: u16 = 120;
const TERMINATOR: u16 = 121;
const SEPARATOR: u16 = 122;
const CATCH_SEPARATOR: u16 = 123;
const GROUP_BEGIN: u16 = 124;
const GROUP_END: u16 = 125;
const BRACKETS_BEGIN: u16 = 126;
const BRACKETS_END: u16 = 127;
const ARRAY_BEGIN: u16 = 128;
// 129: an array's `)` and `]`, which bat scopes apart from a group's and brackets' (the same
// colors): written as theirs
const BLOCK_OPEN: u16 = 130;
const BLOCK_CLOSE: u16 = 131;
const BLOCK_PLAIN: u16 = 132;
const INT: u16 = 133;
const FLOAT: u16 = 134;
const HEX: u16 = 135;
const HEX_PREFIX: u16 = 136;
const BINARY: u16 = 137;
const BINARY_PREFIX: u16 = 138;
const EMBED_OPEN: u16 = 139;
const EMBED_HEREDOC: u16 = 140;
const EMBED_NOWDOC: u16 = 141;
const HEREDOC_OPERATOR: u16 = 142;
const NOWDOC_OPERATOR: u16 = 143;
const ANNOT_BEGIN: u16 = 144;
const ANNOT_END: u16 = 145;
const ANNOT_NAME: u16 = 146;
const RE_SINGLE_BEGIN: u16 = 147;
const RE_DOUBLE_BEGIN: u16 = 149;
const DOC_MODIFIER: u16 = 151;
const DOC_INVALID: u16 = 152;
/// The first support function's kind, by group ([`words::FUNC_GROUPS`]).
const SUPPORT_FUNCTION: u16 = 153;
/// The first support constant's kind, by group ([`words::CONST_GROUPS`]).
const SUPPORT_CONSTANT: u16 = SUPPORT_FUNCTION + words::FUNC_GROUPS.len() as u16;
const KINDS: u16 = SUPPORT_CONSTANT + words::CONST_GROUPS.len() as u16;
// Another language's kinds among PHP's, its own kind added: SQL in a single-quoted string, in
// a double-quoted one, in a `<<<SQL` here-document; JSON in a `<<<JSON` one.
const SQL_SINGLE: u16 = 0x1000;
const SQL_DOUBLE: u16 = 0x2000;
const SQL_HEREDOC: u16 = 0x3000;
const JSON: u16 = 0x4000;
// A regex string's kinds (`'/…/'`, `"/…/"`), the regex scanner's kind added.
const REGEX_SINGLE: u16 = 0x5000;
const REGEX_DOUBLE: u16 = 0x6000;

/// HTML's kinds among PHP's: its own kind (its CSS's and JavaScript's flags kept) with this bit.
const HTML_KIND: u16 = 0x8000;
const _: () = assert!(KINDS <= 0x100);

/// A kind folded into 12 bits, for a host that keeps PHP's kinds among its own (Nix's writers'
/// files, X-153): the embed's nibble (SQL, JSON, a regex, HTML) above a kind below 256, PHP's own
/// or the embedded language's; `None` past them (HTML's with its CSS's or JavaScript's flags).
pub(crate) fn fold(kind: u16) -> Option<u16> {
    let low = kind & 0x0fff;
    (low < 0x100).then_some(kind >> 4 & 0x0f00 | low)
}

/// The kind [`fold`] folded into `k`.
pub(crate) fn unfold(k: u16) -> u16 {
    (k & 0x0f00) << 4 | k & 0xff
}

/// The scopes under `embedding.php` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    if kind & HTML_KIND != 0 {
        let mut v = vec!["text.html.basic"];
        v.extend(crate::html::scopes(kind & !HTML_KIND));
        return v;
    }
    let mut v = vec!["text.html.basic", "source.php"];
    let host: &[&str] = match kind & 0xf000 {
        SQL_SINGLE => &["string.quoted.single.php", "source.sql"],
        SQL_DOUBLE => &["string.quoted.double.php", "source.sql"],
        SQL_HEREDOC => &["source.sql"],
        JSON => &["source.json"],
        REGEX_SINGLE => &["string.quoted.single.php", "source.regexp.php"],
        REGEX_DOUBLE => &["string.quoted.double.php", "source.regexp.php"],
        _ => &[],
    };
    if !host.is_empty() {
        v.extend(host);
        let k = kind & 0x0fff;
        if kind & 0xf000 >= REGEX_SINGLE {
            // the regex scanner's leaves (bat's JavaScript regex grammar's) as bat's PHP regex
            // grammar scopes them, where their colors part
            for sc in crate::regex::scopes(k) {
                let sc = match sc {
                    "punctuation.definition.group.no-capture.regexp"
                    | "punctuation.definition.group.atomic.regexp"
                    | "punctuation.definition.group.assertion.regexp" => {
                        "constant.other.assertion.regexp"
                    }
                    "constant.other.character-class.escape.backslash.regexp" => {
                        "constant.character.character-class.regexp"
                    }
                    "constant.literal.numeric.regexp"
                    | "punctuation.separator.comma.regexp"
                    | "punctuation.definition.range.regexp" => continue,
                    s => s,
                };
                v.push(sc);
            }
        } else if kind & 0xf000 == JSON {
            if let Some(l) = crate::json::KINDS.get(usize::from(k)) {
                v.extend(l.split(' ').filter(|s| !s.is_empty()));
            }
        } else {
            v.extend(crate::sql::scopes(k));
        }
        return v;
    }
    let leaf = match kind {
        HTML => return vec!["text.html.basic"],
        SUPPORT_FUNCTION..SUPPORT_CONSTANT => {
            words::FUNC_GROUPS[usize::from(kind - SUPPORT_FUNCTION)]
        }
        SUPPORT_CONSTANT..KINDS => words::CONST_GROUPS[usize::from(kind - SUPPORT_CONSTANT)],
        _ => match LEAVES.get(usize::from(kind)) {
            Some(l) => l,
            None => return Vec::new(),
        },
    };
    v.extend(leaf.split(' ').filter(|s| !s.is_empty()));
    v
}

/// The bytes a single-quoted string's patterns start on, where its text is SQL: its end, an
/// escape.
static YIELDS_SINGLE: [bool; 256] = crate::prim::stops(b"'\\");
/// A double-quoted string's: its end, an escape, interpolation.
static YIELDS_DOUBLE: [bool; 256] = crate::prim::stops(b"\"\\${");
/// A here-document's: an escape, interpolation (its end is a line's).
static YIELDS_HEREDOC: [bool; 256] = crate::prim::stops(b"\\${");
/// A now-document's: none.
static YIELDS_NONE: [bool; 256] = [false; 256];
/// A double-quoted regex string's: an escape, a variable.
static YIELDS_REGEX: [bool; 256] = crate::prim::stops(b"\\$");

/// Where a regex string at `p` (past its quote `q`) ends, as bat's lookahead has it:
/// `/(\\.|[^q/])+/[eimsuxADJSUX]*q` on the line; its closing `/` and its closing quote.
#[inline(always)]
fn regex_form(b: &[u8], p: usize, q: u8) -> Option<(usize, usize)> {
    let n = b.len();
    let mut e = p + 1;
    while e < n {
        match b[e] {
            b'\\' => e += 2,
            b'/' => break,
            c if c == q => return None,
            _ => e += 1,
        }
    }
    if e >= n || e == p + 1 {
        return None;
    }
    let d = e;
    e += 1;
    while e < n && b"eimsuxADJSUX".contains(&b[e]) {
        e += 1;
    }
    (b.get(e) == Some(&q)).then_some((d, e))
}

/// Whether SQL starts at `p` in a string, as bat's `sql_indicator` has it: blanks, then an
/// uppercase `SELECT`, `INSERT`, `UPDATE`, `DELETE`, `CREATE`, `REPLACE` or `ALTER` ending a
/// word.
#[inline(always)]
fn sql_indicator(b: &[u8], p: usize) -> bool {
    let q = skip_space(b, p);
    let w = &b[q..];
    let len = match w.first() {
        Some(b'S') if w.starts_with(b"SELECT") => 6,
        Some(b'I') if w.starts_with(b"INSERT") => 6,
        Some(b'U') if w.starts_with(b"UPDATE") => 6,
        Some(b'D') if w.starts_with(b"DELETE") => 6,
        Some(b'C') if w.starts_with(b"CREATE") => 6,
        Some(b'R') if w.starts_with(b"REPLACE") => 7,
        Some(b'A') if w.starts_with(b"ALTER") => 5,
        _ => return false,
    };
    !w.get(len).is_some_and(|&c| NAME[usize::from(c)])
}

/// The last with's bit for a line that ends in PHP ([`Scan::state`]); its low three bits are
/// how many of HTML's levels are kept below the PHP.
const PHP_LAYOUT: u8 = 0x80;
/// In HTML's layout, the bit for a text whose first line has decided it's a template's (a `<`):
/// without it (the default state), the first non-blank line decides ([`Scan::fresh`]).
const HTML_DECIDED: u8 = 0x40;
/// HTML's levels a line in PHP keeps below it.
const HTML_HELD: usize = 5;
/// PHP's frames a line in HTML keeps (four bits each, in three withs).
const PHP_HELD: u8 = 6;

// What a here-document's body is ([`Scan::embed`]): text, SQL, JSON.
const E_TEXT: u8 = 0;
const E_SQL: u8 = 1;
const E_JSON: u8 = 2;
/// HTML's. A `<<<XML` body is a here-document's text: XML's scanner on a sink of its own was
/// 25 KB of ada for 69 of the corpus's 3,846 here-documents.
const E_HTML: u8 = 3;

// A byte's action ([`ACT`]).
const A_OTHER: u8 = 0;
const A_SPACE: u8 = 1;
const A_NAME: u8 = 2;
const A_DIGIT: u8 = 3;
const A_DOLLAR: u8 = 4;
const A_SINGLE: u8 = 5;
const A_DOUBLE: u8 = 6;
const A_BACKTICK: u8 = 7;
const A_SLASH: u8 = 8;
const A_HASH: u8 = 9;
const A_LPAREN: u8 = 10;
const A_RPAREN: u8 = 11;
const A_LBRACKET: u8 = 12;
const A_RBRACKET: u8 = 13;
const A_LBRACE: u8 = 14;
const A_RBRACE: u8 = 15;
const A_SEMI: u8 = 16;
const A_COMMA: u8 = 17;
const A_DOT: u8 = 18;
const A_COLON: u8 = 19;
const A_EQ: u8 = 20;
const A_LT: u8 = 21;
const A_GT: u8 = 22;
const A_DASH: u8 = 23;
const A_PLUS: u8 = 24;
/// `*`, `%`.
const A_STAR: u8 = 25;
const A_AMP: u8 = 26;
const A_PIPE: u8 = 27;
const A_CARET: u8 = 28;
const A_BANG: u8 = 29;
const A_QUESTION: u8 = 30;
const A_AT: u8 = 31;
const A_BACKSLASH: u8 = 32;
const A_TILDE: u8 = 33;

static ACT: [u8; 256] = {
    let mut t = [A_OTHER; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        t[c] = match b {
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => A_SPACE,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | 0x80.. => A_NAME,
            b'0'..=b'9' => A_DIGIT,
            b'$' => A_DOLLAR,
            b'\'' => A_SINGLE,
            b'"' => A_DOUBLE,
            b'`' => A_BACKTICK,
            b'/' => A_SLASH,
            b'#' => A_HASH,
            b'(' => A_LPAREN,
            b')' => A_RPAREN,
            b'[' => A_LBRACKET,
            b']' => A_RBRACKET,
            b'{' => A_LBRACE,
            b'}' => A_RBRACE,
            b';' => A_SEMI,
            b',' => A_COMMA,
            b'.' => A_DOT,
            b':' => A_COLON,
            b'=' => A_EQ,
            b'<' => A_LT,
            b'>' => A_GT,
            b'-' => A_DASH,
            b'+' => A_PLUS,
            b'*' | b'%' => A_STAR,
            b'&' => A_AMP,
            b'|' => A_PIPE,
            b'^' => A_CARET,
            b'!' => A_BANG,
            b'?' => A_QUESTION,
            b'@' => A_AT,
            b'\\' => A_BACKSLASH,
            b'~' => A_TILDE,
            _ => A_OTHER,
        };
        c += 1;
    }
    t
};

/// A name's bytes: ASCII's word bytes and every byte past ASCII (PHP's names take them).
static NAME: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 256 {
        t[c] = WORD[c] || c >= 0x80;
        c += 1;
    }
    t
};

/// Whether `c` starts a name: a letter, `_`, or a byte past ASCII.
#[inline(always)]
fn name_start(c: u8) -> bool {
    NAME[usize::from(c)] && !c.is_ascii_digit()
}

// What a keyword is ([`KEYWORDS`], [`KW_INFO`]): case aside, as bat's lists are.
const W_NONE: u8 = 0;
/// `if`, `return`… (`keyword.control`).
const W_CONTROL: u8 = 1;
/// `try`, `throw`, `finally`.
const W_EXCEPTION: u8 = 2;
/// `bool`, `int`…: a type (`storage.type`).
const W_TYPE: u8 = 3;
/// `abstract`, `final`, `const`… (`storage.modifier`).
const W_MODIFIER: u8 = 4;
const W_FUNCTION: u8 = 5;
const W_CLASS: u8 = 6;
const W_NEW: u8 = 7;
const W_USE: u8 = 8;
const W_NAMESPACE: u8 = 9;
const W_INSTANCEOF: u8 = 10;
/// `and`, `or`, `xor`, `as`.
const W_LOGICAL: u8 = 11;
/// `true`, `false`, `null`, `__CLASS__`…
const W_CONSTANT: u8 = 12;
/// `self`, `parent`.
const W_SELF: u8 = 13;
/// `echo`, `print`.
const W_ECHO: u8 = 14;
/// `require`, `include` and their `_once`.
const W_INCLUDE: u8 = 15;
const W_EXTENDS: u8 = 16;
const W_IMPLEMENTS: u8 = 17;
/// `isset`, `unset`, `empty`, `eval`, `list`: a construct before `(`, else a name.
const W_CONSTRUCT: u8 = 18;
/// `static`: a modifier, or `static::`.
const W_STATIC: u8 = 19;
const W_ARRAY: u8 = 20;
const W_CATCH: u8 = 21;
const W_INTERFACE: u8 = 22;
/// `trait`, `goto`, `insteadof`, `callable`: words only some places read.
const W_TRAIT: u8 = 23;
const W_GOTO: u8 = 24;
const W_INSTEADOF: u8 = 25;
const W_VAR: u8 = 26;
const W_CALLABLE: u8 = 27;

const KEYWORD_LIST: [&[u8]; 87] = [
    // W_CONTROL
    b"break",
    b"case",
    b"continue",
    b"declare",
    b"default",
    b"die",
    b"do",
    b"else",
    b"elseif",
    b"enddeclare",
    b"endfor",
    b"endforeach",
    b"endif",
    b"endswitch",
    b"endwhile",
    b"exit",
    b"for",
    b"foreach",
    b"if",
    b"return",
    b"switch",
    b"while",
    b"yield",
    // W_EXCEPTION
    b"try",
    b"throw",
    b"finally",
    // W_TYPE
    b"bool",
    b"boolean",
    b"double",
    b"float",
    b"int",
    b"integer",
    b"object",
    b"real",
    b"string",
    b"clone",
    // W_MODIFIER
    b"abstract",
    b"const",
    b"final",
    b"global",
    b"private",
    b"protected",
    b"public",
    // the rest
    b"function",
    b"class",
    b"new",
    b"use",
    b"namespace",
    b"instanceof",
    b"and",
    b"or",
    b"xor",
    b"as",
    b"true",
    b"false",
    b"null",
    b"__CLASS__",
    b"__DIR__",
    b"__FILE__",
    b"__FUNCTION__",
    b"__LINE__",
    b"__METHOD__",
    b"__NAMESPACE__",
    b"self",
    b"parent",
    b"echo",
    b"print",
    b"require",
    b"require_once",
    b"include",
    b"include_once",
    b"extends",
    b"implements",
    b"isset",
    b"unset",
    b"empty",
    b"eval",
    b"list",
    b"static",
    b"array",
    b"catch",
    b"interface",
    b"trait",
    b"goto",
    b"insteadof",
    b"var",
    b"callable",
];
const NK: usize = KEYWORD_LIST.len();
static KEYWORDS: Words<256, NK, { text_len(&KEYWORD_LIST) }, true> = Words::new(&KEYWORD_LIST);
/// A keyword's [`W_NONE`]… by its id.
static KW_INFO: [u8; NK + 1] = {
    let mut t = [W_NONE; NK + 1];
    let mut i = 0;
    while i < NK {
        let w = KEYWORD_LIST[i];
        t[i + 1] = match i {
            0..23 => W_CONTROL,
            23..26 => W_EXCEPTION,
            26..36 => W_TYPE,
            36..43 => W_MODIFIER,
            _ => match w {
                b"function" => W_FUNCTION,
                b"class" => W_CLASS,
                b"new" => W_NEW,
                b"use" => W_USE,
                b"namespace" => W_NAMESPACE,
                b"instanceof" => W_INSTANCEOF,
                b"and" | b"or" | b"xor" | b"as" => W_LOGICAL,
                b"self" | b"parent" => W_SELF,
                b"echo" | b"print" => W_ECHO,
                b"require" | b"require_once" | b"include" | b"include_once" => W_INCLUDE,
                b"extends" => W_EXTENDS,
                b"implements" => W_IMPLEMENTS,
                b"isset" | b"unset" | b"empty" | b"eval" | b"list" => W_CONSTRUCT,
                b"static" => W_STATIC,
                b"array" => W_ARRAY,
                b"catch" => W_CATCH,
                b"interface" => W_INTERFACE,
                b"trait" => W_TRAIT,
                b"goto" => W_GOTO,
                b"insteadof" => W_INSTEADOF,
                b"var" => W_VAR,
                b"callable" => W_CALLABLE,
                b"true" | b"false" | b"null" | b"__CLASS__" | b"__DIR__" | b"__FILE__"
                | b"__FUNCTION__" | b"__LINE__" | b"__METHOD__" | b"__NAMESPACE__" => W_CONSTANT,
                _ => W_NONE,
            },
        };
        i += 1;
    }
    t
};

const NFA: usize = words::FUNCS_A.len();
static FUNCS_A: Words<4096, NFA, { text_len(&words::FUNCS_A) }, true> = Words::new(&words::FUNCS_A);
const NFB: usize = words::FUNCS_B.len();
static FUNCS_B: Words<4096, NFB, { text_len(&words::FUNCS_B) }, true> = Words::new(&words::FUNCS_B);
/// A support function's kind by its id in [`FUNCS_A`].
static FUNC_KIND_A: [u16; NFA + 1] = {
    let mut t = [CALL; NFA + 1];
    let mut i = 0;
    while i < NFA {
        t[i + 1] = SUPPORT_FUNCTION + words::FUNC_GROUP_A[i] as u16;
        i += 1;
    }
    t
};
static FUNC_KIND_B: [u16; NFB + 1] = {
    let mut t = [CALL; NFB + 1];
    let mut i = 0;
    while i < NFB {
        t[i + 1] = SUPPORT_FUNCTION + words::FUNC_GROUP_B[i] as u16;
        i += 1;
    }
    t
};
const NC: usize = words::CONSTS.len();
static CONSTS: Words<4096, NC, { text_len(&words::CONSTS) }> = Words::new(&words::CONSTS);
/// A name's kind by its id in [`CONSTS`]: a support constant's, or [`CONSTANT`] for none.
static CONST_KIND: [u16; NC + 1] = {
    let mut t = [CONSTANT; NC + 1];
    let mut i = 0;
    while i < NC {
        t[i + 1] = SUPPORT_CONSTANT + words::CONST_GROUP[i] as u16;
        i += 1;
    }
    t
};
const NCL: usize = words::CLASSES.len();
static CLASSES: Words<512, NCL, { text_len(&words::CLASSES) }, true> = Words::new(&words::CLASSES);

/// The magic methods a function's name reads as `support.function.magic`.
const MAGIC_LIST: [&[u8]; 16] = [
    b"__callStatic",
    b"__call",
    b"__construct",
    b"__destruct",
    b"__get",
    b"__set",
    b"__isset",
    b"__unset",
    b"__toString",
    b"__clone",
    b"__set_state",
    b"__sleep",
    b"__wakeup",
    b"__autoload",
    b"__invoke",
    b"__debugInfo",
];
static MAGIC_WORDS: Words<32, 16, { text_len(&MAGIC_LIST) }> = Words::new(&MAGIC_LIST);

/// phpdoc's tags (`keyword.other.phpdoc`), then PHPUnit's (`keyword.other.phpunit`), after `@`.
const TAG_LIST: [&[u8]; 66] = [
    b"abstract",
    b"api",
    b"author",
    b"category",
    b"copyright",
    b"example",
    b"global",
    b"internal",
    b"ignore",
    b"license",
    b"link",
    b"package",
    b"param",
    b"return",
    b"see",
    b"since",
    b"static",
    b"subpackage",
    b"throws",
    b"todo",
    b"var",
    b"version",
    b"uses",
    b"deprecated",
    b"final",
    b"filesource",
    b"property",
    b"property-read",
    b"property-write",
    b"method",
    b"source",
    // PHPUnit's
    b"after",
    b"afterClass",
    b"backupGlobals",
    b"backupStaticAttributes",
    b"before",
    b"beforeClass",
    b"codeCoverageIgnor",
    b"codeCoverageIgnore",
    b"covers",
    b"coversDefaultClass",
    b"coversNothing",
    b"dataProvider",
    b"depends",
    b"doesNotPerformAssertions",
    b"expectedException",
    b"expectedExceptionCode",
    b"expectedExceptionMessage",
    b"expectedExceptionMessageRegExp",
    b"group",
    b"large",
    b"medium",
    b"preserveGlobalState",
    b"requires",
    b"runTestsInSeparateProcesses",
    b"runInSeparateProcess",
    b"small",
    b"test",
    b"testdox",
    b"testWith",
    b"ticket",
    // the rest of PHPUnit's that phpdoc's list has: none; padding the `codeCoverageIgnore*` run
    b"codeCoverageIgnoree",
    b"codeCoverageIgnoreee",
    b"codeCoverageIgnoreeee",
    b"codeCoverageIgnoreeeee",
    b"codeCoverageIgnoreeeeee",
];
/// The last of [`TAG_LIST`]'s phpdoc tags' ids: PHPUnit's follow.
const UNIT_TAGS: u16 = 31;
static TAGS: Words<256, 66, { text_len(&TAG_LIST) }> = Words::new(&TAG_LIST);

// The frames, four bits a level in [`Scan::frames`]; 0 is none (the top level, or a level lost
// past 32 levels).
const F_NONE: u8 = 0;
/// `{ … }`: a block, a function's body.
const F_BLOCK: u8 = 1;
/// A class's body.
const F_CLASS: u8 = 2;
/// A group's `( … )`, `array( … )`: its `,`s plain.
const F_GROUP: u8 = 3;
/// A call's arguments: its `,`s separate.
const F_CALL: u8 = 4;
/// A function's parameters.
const F_PARAMS: u8 = 5;
/// A parameter's default value, to its `,` or `)`.
const F_DEFAULT: u8 = 6;
/// A closure's `use ( … )`.
const F_USE_VARS: u8 = 7;
/// `[ … ]`: an array, an index.
const F_BRACKET: u8 = 8;
/// `catch ( … )`.
const F_CATCH: u8 = 9;
/// `use A\{ … }`.
const F_USE_BLOCK: u8 = 10;
/// A class's `use T { … }`.
const F_TRAIT_BLOCK: u8 = 11;
/// A string's `{$ … }` or `${ … }`: its `}` goes back to the string ([`Scan::istr`]).
const F_INTERP: u8 = 12;
/// An attribute's `#[ … ]` (PHP 8; bat reads `#` as a comment).
const F_ATTR: u8 = 13;
/// An anonymous class's arguments (`new class( … )`): its head and body follow.
const F_CLASS_ARGS: u8 = 15;

// The token before ([`Scan::prev`]), and where it stands.
/// An operand may start.
const P_BEGIN: u8 = 0;
/// After an operand: `[` an index.
const P_END: u8 = 1;
/// After `->`: a member.
const P_ARROW: u8 = 2;
/// After `::`: a class's constant, static member or method.
const P_DCOLON: u8 = 3;
/// After `function`: its name.
const P_FUNC: u8 = 4;
/// After a function's name or a closure's `function`: its parameters.
const P_FNEND: u8 = 5;
/// After `class`: its name.
const P_CLASS: u8 = 6;
/// After `new`, `instanceof`: a class.
const P_NEW: u8 = 7;
/// A class's head, after its name or what it extends: `extends`, `implements`, its body.
const P_CLASS_HEAD: u8 = 8;
/// After a class's `extends`: what it extends.
const P_EXTENDS: u8 = 9;
/// After `implements` or a `,` in its list.
const P_IMPLEMENTS: u8 = 10;
/// After a name in `implements`' list.
const P_IMPL_END: u8 = 11;
/// After a function's parameters: `use (`, `:` and its return type, its body.
const P_PARAMS_END: u8 = 12;
/// After a return type's `:` or `?`: the type.
const P_RETURN: u8 = 13;
/// After a return type.
const P_RETURN_END: u8 = 14;
/// After a property's modifier in a class's body: more of them, or its type.
const P_PROP: u8 = 15;
/// After `use` or a `,` in its list: a path ([`Scan::using`] says what of).
const P_USE: u8 = 16;
/// After a `use` path.
const P_USE_END: u8 = 17;
/// After `use`'s `as`: the alias.
const P_USE_AS: u8 = 18;
/// After `namespace`: its name.
const P_NS: u8 = 19;
/// After `interface`: its name.
const P_IFACE: u8 = 20;
/// After an interface's name: `extends`.
const P_IFACE_HEAD: u8 = 21;
/// After an interface's `extends`: one name.
const P_IFACE_EXTENDS: u8 = 22;
/// After `trait`: its name.
const P_TRAIT: u8 = 23;
/// A class's `use` list of traits.
const P_TRAIT_USE: u8 = 24;
/// After `goto`: its label.
const P_GOTO: u8 = 25;
/// After a call's name: `(` opens its arguments.
const P_CALLEE: u8 = 26;
/// After a closure's `use`: its `(`.
const P_FN_USE: u8 = 27;
/// After `array`: `(` is an array's.
const P_ARRAY: u8 = 28;
/// After an attribute's `#[` or a `,` in it: an attribute's name.
const P_ATTR: u8 = 29;
/// After `instanceof`: a class (`self` a class's name there, not `new`'s language variable).
const P_INSTANCEOF: u8 = 30;

// What a `use` statement brings in ([`Scan::using`]).
const U_CLASS: u8 = 0;
const U_FUNCTION: u8 = 1;
const U_CONST: u8 = 2;

// What a line starts in ([`Scan::mode`]).
const M_CODE: u8 = 0;
/// HTML, outside `<?php … ?>`.
const M_HTML: u8 = 1;
/// A `/* … */` comment.
const M_COMMENT: u8 = 2;
/// A `/** … */` comment.
const M_DOC: u8 = 3;
/// A single-quoted string continued past the line's end.
const M_SINGLE: u8 = 4;
const M_DOUBLE: u8 = 5;
const M_BACKTICK: u8 = 6;
/// A here-document's body, to the line that starts with [`Scan::label`].
const M_HEREDOC: u8 = 7;
const M_NOWDOC: u8 = 8;

/// Lex `b`, a line of PHP without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    line_on::<_, false>(b, state, out, Spans::new());
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
    let mut k = line_on::<_, true>(b, state, out, Runs::new(rules, runs));
    k.end_line(b.len());
}

#[inline(always)]
fn line_on<K: Sink, const SHARED: bool>(b: &[u8], state: &mut State, out: &mut Out, k: K) -> K {
    let mut s = Scan::new(b, out, k);
    s.load(state);
    s.k.open(s.out, b.len() / 4 + 16);
    s.lex_line::<SHARED>();
    s.k.close(s.out);
    *state = s.state();
    s.k
}

/// A block of PHP on one scan opened from `state` ([`crate::Carry`]).
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

/// HTML's side of a scan ([`Scan::host`]): its state outside the PHP (at the line's start, or its
/// end once the line's HTML is lexed), the line's HTML lexed with its PHP blanked
/// ([`Scan::html_pass`]), whether that's this line's, and the next of its spans to write.
struct Host {
    html: State,
    spans: Vec<crate::Span>,
    line: bool,
    next: usize,
}

struct Scan<'b, 'c, K: Sink> {
    b: &'b [u8],
    /// What each open frame is ([`F_BLOCK`]…), four bits a level, the innermost lowest; past
    /// 32 levels the outer ones are lost.
    frames: u128,
    /// How many frames are open, up to 255.
    depth: u8,
    /// The token before ([`P_BEGIN`]…).
    prev: u8,
    /// What the line starts in ([`M_CODE`]…).
    mode: u8,
    /// PHP's rarer fields in one word, kept as a whole between lines ([`Scan::label`],
    /// [`Scan::embed`], [`Scan::json_comment`], [`Scan::sql_ctx`], [`Scan::using`],
    /// [`Scan::sql`], [`Scan::head`], [`Scan::istr`]): a line loads and saves it in one move.
    aux: u64,
    /// SQL's place in this line ([`crate::sql::Embedded`]), not kept between lines.
    sql_keep: crate::sql::Embedded,
    /// HTML's side ([`Host`]), made where a line first needs it: none in pure PHP, whose lines
    /// then set up nothing of it.
    host: Option<Box<Host>>,
    /// Nothing but blank lines (or a shebang) yet: the first other line decides whether the text
    /// is a template, from a `<` (bat's PHP, which starts in HTML), or PHP's code without its
    /// tags (a fence's snippet, as bat's Markdown embeds `source.php` for one).
    fresh: bool,
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
            prev: P_BEGIN,
            mode: M_HTML,
            aux: 0,
            sql_keep: crate::sql::Embedded {
                is_end: usize::MAX,
                at: 0,
            },
            host: None,
            fresh: false,
            k,
            out,
        }
    }

    /// The scan's fields from `state`, in one of two layouts, told apart by [`PHP_LAYOUT`] in
    /// the last with ([`Scan::state`]).
    #[inline(always)]
    fn load(&mut self, state: &State) {
        let tag = state.withs[13];
        if tag & PHP_LAYOUT == 0 {
            return self.load_html(state);
        }
        let mut w = [0u8; 16];
        w[..14].copy_from_slice(&state.levels);
        w[14] = state.withs[0];
        w[15] = state.withs[1];
        self.frames = u128::from_le_bytes(w);
        self.depth = state.depth;
        let mut x = [0u8; 8];
        x[..6].copy_from_slice(&state.withs[2..8]);
        self.aux = u64::from_le_bytes(x);
        let a = state.arg;
        self.mode = (a & 15) as u8;
        self.prev = (a >> 4 & 31) as u8;
        // HTML below the PHP: its stack's top levels, up to [`HTML_HELD`]
        if tag & 7 != 0 {
            let k = usize::from(tag & 7);
            let mut h = State::default();
            h.levels[..k].copy_from_slice(&state.withs[8..8 + k]);
            h.depth = k as u8;
            self.host().html = h;
        } else if let Some(h) = self.host.as_mut() {
            h.html = State::default();
        }
    }

    // [`Scan::aux`]'s fields: the label in 28 bits, the embed in 3, the JSON comment in 1, SQL's
    // context in 8, `using` in 2, SQL on, the head, the interpolation's string in 2 (46 bits).
    #[inline(always)]
    fn label(&self) -> u32 {
        (self.aux & 0x0fff_ffff) as u32
    }
    #[inline(always)]
    fn set_label(&mut self, v: u32) {
        self.aux = self.aux & !0x0fff_ffff | u64::from(v & 0x0fff_ffff);
    }
    #[inline(always)]
    fn embed(&self) -> u8 {
        (self.aux >> 28 & 7) as u8
    }
    #[inline(always)]
    fn set_embed(&mut self, v: u8) {
        self.aux = self.aux & !(7 << 28) | u64::from(v & 7) << 28;
    }
    #[inline(always)]
    fn json_comment(&self) -> bool {
        self.aux >> 31 & 1 != 0
    }
    #[inline(always)]
    fn set_json_comment(&mut self, v: bool) {
        self.aux = self.aux & !(1 << 31) | u64::from(v) << 31;
    }
    #[inline(always)]
    fn sql_ctx(&self) -> u8 {
        (self.aux >> 32) as u8
    }
    #[inline(always)]
    fn set_sql_ctx(&mut self, v: u8) {
        self.aux = self.aux & !(0xff << 32) | u64::from(v) << 32;
    }
    #[inline(always)]
    fn using(&self) -> u8 {
        (self.aux >> 40 & 3) as u8
    }
    #[inline(always)]
    fn set_using(&mut self, v: u8) {
        self.aux = self.aux & !(3 << 40) | u64::from(v & 3) << 40;
    }
    #[inline(always)]
    fn sql(&self) -> bool {
        self.aux >> 42 & 1 != 0
    }
    #[inline(always)]
    fn set_sql(&mut self, v: bool) {
        self.aux = self.aux & !(1 << 42) | u64::from(v) << 42;
    }
    #[inline(always)]
    fn head(&self) -> bool {
        self.aux >> 43 & 1 != 0
    }
    #[inline(always)]
    fn set_head(&mut self, v: bool) {
        self.aux = self.aux & !(1 << 43) | u64::from(v) << 43;
    }
    #[inline(always)]
    fn istr(&self) -> u8 {
        M_DOUBLE + (self.aux >> 44 & 3) as u8
    }
    #[inline(always)]
    fn set_istr(&mut self, v: u8) {
        self.aux = self.aux & !(3 << 44) | u64::from((v - M_DOUBLE) & 3) << 44;
    }

    /// HTML's side, made where it isn't yet.
    #[inline(always)]
    fn host(&mut self) -> &mut Host {
        self.host.get_or_insert_with(|| {
            Box::new(Host {
                html: State::default(),
                spans: Vec::new(),
                line: false,
                next: 0,
            })
        })
    }

    /// Whether HTML's state isn't the default: a line in PHP keeps some of it below.
    #[inline(always)]
    fn html_set(&self) -> bool {
        self.host
            .as_ref()
            .is_some_and(|h| h.html.depth != 0 || h.html.arg != 0)
    }

    /// [`Scan::load`] of a line that starts in HTML: HTML's state, and PHP's innermost
    /// [`PHP_HELD`] frames in the withs past HTML's.
    #[cold]
    #[inline(never)]
    fn load_html(&mut self, state: &State) {
        let k = usize::from(state.withs[13] & 7);
        // what a line in HTML doesn't keep: PHP's place, as at a file's start
        self.prev = P_BEGIN;
        self.aux = 0;
        self.fresh = state.withs[13] & HTML_DECIDED == 0;
        let mut w = [0u8; 4];
        w[..3].copy_from_slice(&state.withs[10..13]);
        self.frames = u128::from(u32::from_le_bytes(w));
        self.depth = k as u8;
        self.mode = M_HTML;
        let mut h = *state;
        h.withs[10..14].fill(0);
        if h.depth != 0 || h.arg != 0 {
            self.host().html = h;
        } else if let Some(x) = self.host.as_mut() {
            x.html = State::default();
        }
    }

    /// The state the scan's last line left: every field as it is, so a block lexes as its
    /// lines do one at a time. In PHP, its fields, and below them HTML's stack's top levels (its
    /// state there lost where an embed is open or they're more than [`HTML_HELD`]); in HTML,
    /// HTML's state, and PHP's innermost [`PHP_HELD`] frames.
    #[inline(always)]
    fn state(&self) -> State {
        if self.mode == M_HTML {
            return self.state_html();
        }
        let w = self.frames.to_le_bytes();
        let mut s = State::default();
        s.levels.copy_from_slice(&w[..14]);
        s.withs[0] = w[14];
        s.withs[1] = w[15];
        s.withs[2..8].copy_from_slice(&self.aux.to_le_bytes()[..6]);
        s.depth = self.depth;
        s.arg = u16::from(self.mode) | u16::from(self.prev) << 4;
        s.withs[13] = PHP_LAYOUT;
        if self.html_set() {
            self.html_below(&mut s);
        }
        s
    }

    /// HTML's state below a line that ends in PHP, kept in `s` where it can be: its stack's top
    /// levels, up to [`HTML_HELD`], no embed open.
    #[cold]
    #[inline(never)]
    fn html_below(&self, s: &mut State) {
        let Some(h) = self.host.as_ref().map(|h| &h.html) else {
            return;
        };
        let k = usize::from(h.depth);
        let mut tag = PHP_LAYOUT;
        if k <= HTML_HELD && h.arg == 0 && h.withs.iter().all(|&w| w == 0) {
            s.withs[8..8 + k].copy_from_slice(&h.levels[..k]);
            tag |= k as u8;
        }
        s.withs[13] = tag;
    }

    /// [`Scan::state`] where the line ends in HTML.
    #[cold]
    #[inline(never)]
    fn state_html(&self) -> State {
        let mut s = self.host.as_ref().map_or_else(State::default, |h| h.html);
        if s.withs[10..14] != [0; 4] {
            // HTML's (or its embed's) stack reached the withs PHP keeps its frames in
            s.withs[10..14].fill(0);
        }
        let k = self.depth.min(PHP_HELD);
        let f = (self.frames as u32) & ((1 << (4 * u32::from(k))) - 1);
        s.withs[10..13].copy_from_slice(&f.to_le_bytes()[..3]);
        s.withs[13] = k | if self.fresh { 0 } else { HTML_DECIDED };
        s
    }

    /// `b`, a block's next line, lexed on this scan, its sink closed; the scan's line an empty
    /// one again after.
    #[inline(always)]
    fn carry<const SHARED: bool>(&mut self, b: &'b [u8]) {
        self.k.start_line();
        self.b = b;
        self.lex_line::<SHARED>();
        self.k.close(self.out);
        self.b = b"";
        self.settle();
    }

    /// What [`Scan::state`] clamps or drops, clamped and dropped after a block's line too, so
    /// a block lexes as its lines do one at a time: where HTML is involved, the state's round
    /// trip (a line in PHP with HTML below it at its start; pure PHP's fields are kept whole).
    #[inline(always)]
    fn settle(&mut self) {
        if self.mode == M_HTML || self.html_set() {
            let s = self.state();
            self.load(&s);
        }
    }

    /// Bytes `s..e` are `kind`: on the run sink one call a write (inlined at each of the scan's
    /// writes, the run's write was most of the scan's size in ada, as nginx's was, X-061).
    #[inline(always)]
    fn put(&mut self, s: usize, e: usize, kind: u16) {
        self.k.merge_call(&mut self.out, s, e, kind);
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

    /// The line, from what the line before left: a mode's rest first; out of line where
    /// `SHARED` ([`Scan::lex_shared`]).
    #[inline(always)]
    fn lex_line<const SHARED: bool>(&mut self) {
        if SHARED {
            self.lex_shared();
        } else {
            self.lex_body();
        }
    }

    /// [`Scan::lex_body`] out of line: on the run sink one copy, modes and all, that the line API
    /// and a block's lines share (inlined into each, ada held the string and comment modes
    /// twice).
    #[inline(never)]
    fn lex_shared(&mut self) {
        self.lex_body();
    }

    /// [`Scan::lex_line`]'s work.
    #[inline(always)]
    fn lex_body(&mut self) {
        let mut p = 0;
        let n = self.b.len();
        self.sql_keep.is_end = usize::MAX;
        if let Some(h) = self.host.as_mut() {
            h.line = false;
        }
        match self.mode {
            M_DOC => p = self.doc_line_start(),
            M_HEREDOC | M_NOWDOC => p = self.heredoc_line_start(),
            _ => {}
        }
        loop {
            p = match self.mode {
                M_CODE => {
                    if self.top() == F_PARAMS {
                        self.params(p)
                    } else {
                        self.code(p)
                    }
                }
                M_HTML => self.html(p),
                M_COMMENT | M_DOC => self.block_comment(p),
                M_SINGLE => self.single(p),
                M_DOUBLE => self.double(p, DOUBLE),
                M_BACKTICK => self.double(p, BACKTICK),
                _ => self.heredoc_body(p),
            };
            if p >= n {
                break;
            }
        }
        // a `use` statement ends with its line, but in its `{ … }`
        if matches!(self.prev, P_USE | P_USE_END | P_USE_AS) && self.top() != F_USE_BLOCK {
            self.prev = P_BEGIN;
        }
    }

    /// HTML from `p` to `<?php`, `<?=` or `<?`: the line's HTML lexed once, its PHP blanked
    /// ([`Scan::html_pass`]), its spans in this stretch written.
    #[inline(never)]
    fn html(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        if self.fresh {
            let q = skip_space(b, p);
            if q == n || b[q..].starts_with(b"#!") {
                // a blank line, a shebang: undecided yet, the text HTML's
                self.put(p, n, HTML);
                return n;
            }
            self.fresh = false;
            if b[q] != b'<' {
                // PHP's code without its tags
                self.mode = M_CODE;
                self.prev = P_BEGIN;
                return p;
            }
        }
        let q = php_open(b, p);
        if q > p {
            if !self.host.as_ref().is_some_and(|h| h.line) {
                self.html_pass(p);
            }
            self.write_html(p, q);
        }
        if q >= n {
            return n;
        }
        let mut e = q + 2;
        if b.len() >= e + 3 && b[e..e + 3].eq_ignore_ascii_case(b"php") {
            e += 3;
        } else if b.get(e) == Some(&b'=') {
            e += 1;
        }
        self.put(q, e, EMBED_BEGIN);
        self.mode = M_CODE;
        self.prev = P_BEGIN;
        e
    }

    /// The line's HTML from `p` lexed by HTML's scanner, from [`Scan::html`], on a copy of the line
    /// with what's before `p` and each PHP island after it (`<?` to its `?>`, or the line's end)
    /// blanked: HTML's contexts run across an island as across blanks, as bat's resume after
    /// `?>`. The spans into [`Scan::hspans`], HTML's state to the line's end.
    #[cold]
    #[inline(never)]
    fn html_pass(&mut self, p: usize) {
        let b = self.b;
        let n = b.len();
        let mut buf = b.to_vec();
        buf[..p].fill(b' ');
        let mut q = p;
        loop {
            let s = php_open(b, q);
            if s >= n {
                break;
            }
            let e = php_close(b, s + 2);
            buf[s..e].fill(b' ');
            q = e;
        }
        let h = self.host();
        h.spans.clear();
        crate::html::line(&buf, &mut h.html, &mut Out::new(&mut h.spans));
        h.line = true;
        h.next = 0;
    }

    /// The line's HTML spans in `p..q` written, its text between them HTML's.
    #[inline(always)]
    fn write_html(&mut self, p: usize, q: usize) {
        let mut at = p;
        let mut i = self.host.as_ref().map_or(0, |h| h.next);
        loop {
            let sp = match self.host.as_ref() {
                Some(h) if i < h.spans.len() => h.spans[i],
                _ => break,
            };
            let (s, e) = (sp.start as usize, sp.end as usize);
            if s >= q {
                break;
            }
            if e > at {
                let s = s.max(at);
                self.put(at, s, HTML);
                let e = e.min(q);
                self.put(s, e, sp.kind | HTML_KIND);
                at = e;
            }
            if e > q {
                break;
            }
            i += 1;
        }
        if let Some(h) = self.host.as_mut() {
            h.next = i;
        }
        self.put(at, q, HTML);
    }

    /// The code from `p` to the line's end, or to where a mode starts (a string or comment
    /// continued, `?>`).
    #[inline(always)]
    fn code(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        while p < n {
            let c = b[p];
            p = match ACT[usize::from(c)] {
                A_SPACE => {
                    let mut e = p + 1;
                    while e < n && ACT[usize::from(b[e])] == A_SPACE {
                        e += 1;
                    }
                    e
                }
                A_NAME => self.name(p),
                A_DOLLAR => self.variable(p),
                A_DIGIT => self.number(p),
                A_SINGLE => {
                    if b.get(p + 1) == Some(&b'/')
                        && let Some((d, f)) = regex_form(b, p + 1, c)
                    {
                        self.regex_string(p, d, f, SINGLE)
                    } else {
                        self.put(p, p + 1, SINGLE + BEGIN);
                        self.prev = P_END;
                        self.set_head(true);
                        self.single(p + 1)
                    }
                }
                A_DOUBLE
                    if b.get(p + 1) == Some(&b'/')
                        && let Some((d, f)) = regex_form(b, p + 1, c) =>
                {
                    self.regex_string(p, d, f, DOUBLE)
                }
                A_DOUBLE => {
                    self.put(p, p + 1, DOUBLE + BEGIN);
                    self.prev = P_END;
                    self.set_head(true);
                    self.mode = M_DOUBLE;
                    return p + 1;
                }
                A_BACKTICK => {
                    self.put(p, p + 1, BACKTICK + BEGIN);
                    self.prev = P_END;
                    self.mode = M_BACKTICK;
                    return p + 1;
                }
                A_SLASH => match b.get(p + 1) {
                    Some(b'/') => return self.line_comment(p, SLASH_MARK),
                    Some(b'*') => return self.comment_open(p),
                    Some(b'=') => self.op(p, 2, AUGMENTED),
                    _ => self.op(p, 1, ARITHMETIC),
                },
                A_HASH => {
                    if b.get(p + 1) == Some(&b'[') {
                        self.attribute(p)
                    } else {
                        return self.line_comment(p, HASH_MARK);
                    }
                }
                A_LPAREN => {
                    let e = self.lparen(p);
                    if self.top() == F_PARAMS {
                        // the parameters' own loop
                        return e;
                    }
                    e
                }
                A_RPAREN => self.rparen(p),
                A_LBRACKET => {
                    let kind = if self.prev == P_END {
                        BRACKETS_BEGIN
                    } else {
                        ARRAY_BEGIN
                    };
                    self.push(F_BRACKET);
                    self.put(p, p + 1, kind);
                    self.prev = P_BEGIN;
                    p + 1
                }
                A_RBRACKET => {
                    if self.top() == F_DEFAULT {
                        self.pop();
                    }
                    match self.top() {
                        F_BRACKET => {
                            self.pop();
                            self.put(p, p + 1, BRACKETS_END);
                            self.prev = P_END;
                        }
                        F_ATTR => {
                            self.pop();
                            self.put(p, p + 1, ANNOT_END);
                            self.prev = P_BEGIN;
                            if self.top() == F_PARAMS {
                                // the parameters' own loop
                                return p + 1;
                            }
                        }
                        _ => {
                            self.put(p, p + 1, BRACKETS_END);
                            self.prev = P_END;
                        }
                    }
                    p + 1
                }
                A_LBRACE => self.lbrace(p),
                A_RBRACE => {
                    let e = self.rbrace(p);
                    if self.mode != M_CODE {
                        return e;
                    }
                    e
                }
                A_SEMI if self.top() == F_USE_BLOCK => p + 1,
                A_SEMI => {
                    if self.top() == F_DEFAULT {
                        self.pop();
                    }
                    self.put(p, p + 1, TERMINATOR);
                    self.prev = P_BEGIN;
                    p + 1
                }
                A_COMMA => {
                    let e = self.comma(p);
                    if self.top() == F_PARAMS {
                        // a default value's end: the parameters' own loop
                        return e;
                    }
                    e
                }
                A_DOT => {
                    if b.get(p + 1).is_some_and(u8::is_ascii_digit) {
                        self.number(p)
                    } else if b.get(p + 1..p + 3) == Some(b"..") {
                        self.op(p, 3, SPREAD)
                    } else if b.get(p + 1) == Some(&b'=') {
                        self.op(p, 2, AUGMENTED)
                    } else {
                        self.op(p, 1, CONCAT)
                    }
                }
                A_COLON => {
                    if self.top() == F_USE_BLOCK {
                        p + 1
                    } else if b.get(p + 1) == Some(&b':') {
                        self.put(p, p + 2, DOUBLE_COLON);
                        self.prev = P_DCOLON;
                        p + 2
                    } else if self.prev == P_PARAMS_END {
                        self.put(p, p + 1, SEPARATOR);
                        self.prev = P_RETURN;
                        p + 1
                    } else {
                        self.prev = P_BEGIN;
                        p + 1
                    }
                }
                A_EQ => match b.get(p + 1) {
                    Some(b'>') => self.op(p, 2, KEY),
                    Some(b'=') => {
                        let l = if b.get(p + 2) == Some(&b'=') { 3 } else { 2 };
                        self.op(p, l, COMPARISON)
                    }
                    Some(b'&') => {
                        self.put(p, p + 1, ASSIGNMENT);
                        self.put(p + 1, p + 2, REFERENCE);
                        self.prev = P_BEGIN;
                        p + 2
                    }
                    _ => {
                        if self.top() == F_PARAMS {
                            self.push(F_DEFAULT);
                        }
                        self.op(p, 1, ASSIGNMENT)
                    }
                },
                A_LT => match b.get(p + 1) {
                    Some(b'<') => {
                        if b.get(p + 2) == Some(&b'<')
                            && let Some(e) = self.heredoc_open(p)
                        {
                            return e;
                        }
                        if b.get(p + 2) == Some(&b'=') {
                            self.op(p, 3, AUGMENTED)
                        } else {
                            self.op(p, 2, BITWISE)
                        }
                    }
                    Some(b'=') => {
                        let l = if b.get(p + 2) == Some(&b'>') { 3 } else { 2 };
                        self.op(p, l, COMPARISON)
                    }
                    Some(b'>') => self.op(p, 2, COMPARISON),
                    _ => self.op(p, 1, COMPARISON),
                },
                A_GT => match b.get(p + 1) {
                    Some(b'>') => {
                        if b.get(p + 2) == Some(&b'=') {
                            self.op(p, 3, AUGMENTED)
                        } else {
                            self.op(p, 2, BITWISE)
                        }
                    }
                    Some(b'=') => self.op(p, 2, COMPARISON),
                    _ => self.op(p, 1, COMPARISON),
                },
                A_DASH => match b.get(p + 1) {
                    Some(b'>') => {
                        self.put(p, p + 2, ARROW);
                        self.prev = P_ARROW;
                        p + 2
                    }
                    Some(b'-') => {
                        self.put(p, p + 2, INC_DEC);
                        p + 2
                    }
                    Some(b'=') => self.op(p, 2, AUGMENTED),
                    _ => self.op(p, 1, ARITHMETIC),
                },
                A_PLUS => match b.get(p + 1) {
                    Some(b'+') => {
                        self.put(p, p + 2, INC_DEC);
                        p + 2
                    }
                    Some(b'=') => self.op(p, 2, AUGMENTED),
                    _ => self.op(p, 1, ARITHMETIC),
                },
                A_STAR => {
                    if b.get(p + 1) == Some(&b'=') {
                        self.op(p, 2, AUGMENTED)
                    } else {
                        self.op(p, 1, ARITHMETIC)
                    }
                }
                A_AMP => match b.get(p + 1) {
                    Some(b'&') => self.op(p, 2, LOGICAL),
                    Some(b'=') => self.op(p, 2, AUGMENTED),
                    Some(&d) if name_start(d) || d == b'$' => {
                        self.put(p, p + 1, REFERENCE);
                        p + 1
                    }
                    _ => {
                        if matches!(self.top(), F_PARAMS | F_USE_VARS) {
                            self.put(p, p + 1, REFERENCE);
                            p + 1
                        } else {
                            self.op(p, 1, BITWISE)
                        }
                    }
                },
                A_PIPE => match b.get(p + 1) {
                    Some(b'|') => self.op(p, 2, LOGICAL),
                    Some(b'=') => self.op(p, 2, AUGMENTED),
                    _ => {
                        if self.top() == F_CATCH {
                            self.put(p, p + 1, CATCH_SEPARATOR);
                            p + 1
                        } else {
                            self.op(p, 1, BITWISE)
                        }
                    }
                },
                A_CARET => {
                    if b.get(p + 1) == Some(&b'=') {
                        self.op(p, 2, AUGMENTED)
                    } else {
                        self.op(p, 1, BITWISE)
                    }
                }
                A_BANG => {
                    if b.get(p + 1) == Some(&b'=') {
                        let l = if b.get(p + 2) == Some(&b'=') { 3 } else { 2 };
                        self.op(p, l, COMPARISON)
                    } else {
                        self.op(p, 1, LOGICAL)
                    }
                }
                A_QUESTION => match b.get(p + 1) {
                    Some(b'>') if matches!(self.top(), F_NONE | F_BLOCK) => {
                        self.put(p, p + 2, EMBED_END);
                        self.mode = M_HTML;
                        return p + 2;
                    }
                    Some(b'?') => {
                        if b.get(p + 2) == Some(&b'=') {
                            self.op(p, 3, AUGMENTED)
                        } else {
                            self.prev = P_BEGIN;
                            p + 2
                        }
                    }
                    Some(b'-') if b.get(p + 2) == Some(&b'>') => {
                        self.put(p + 1, p + 3, ARROW);
                        self.prev = P_ARROW;
                        p + 3
                    }
                    _ => {
                        if matches!(self.prev, P_RETURN | P_PROP) || self.top() == F_PARAMS {
                            self.put(p, p + 1, NULLABLE);
                        } else {
                            self.prev = P_BEGIN;
                        }
                        p + 1
                    }
                },
                A_AT => self.op(p, 1, ERROR_CONTROL),
                A_BACKSLASH => {
                    let kind = match self.prev {
                        P_NS => NS_DECL_SEP,
                        P_ATTR => ANNOT_NAME,
                        P_BEGIN | P_END if !matches!(self.top(), F_CATCH | F_USE_BLOCK) => {
                            self.lead_sep(p)
                        }
                        _ => NS_SEP,
                    };
                    self.put(p, p + 1, kind);
                    p + 1
                }
                A_TILDE => self.op(p, 1, BITWISE),
                _ => p + char_len(c),
            };
        }
        p
    }

    /// A `\\` at `p` in an operand's place, a path's lead: a separator before a call, a class
    /// (`::`), a path's next part or a support constant; before another name bat's `constants`
    /// leave it plain.
    #[cold]
    #[inline(never)]
    fn lead_sep(&self, p: usize) -> u16 {
        let b = self.b;
        let n = b.len();
        let s = p + 1;
        if !b.get(s).is_some_and(|&c| name_start(c)) {
            return NS_SEP;
        }
        let mut e = s + 1;
        while e < n && NAME[usize::from(b[e])] {
            e += 1;
        }
        let q = skip(b, e);
        match b.get(e) {
            Some(b'\\') => return NS_SEP,
            _ if matches!(b.get(q), Some(b'(')) => return NS_SEP,
            _ if b.get(q) == Some(&b':') && b.get(q + 1) == Some(&b':') => return NS_SEP,
            _ => {}
        }
        if CONSTS.get(&b[s..e]) != 0 || KEYWORDS.value(&b[s..e], &KW_INFO) != W_NONE {
            NS_SEP
        } else {
            PLAIN
        }
    }

    /// An operator of `len` bytes at `p`: an operand may follow.
    #[inline(always)]
    fn op(&mut self, p: usize, len: usize, kind: u16) -> usize {
        self.put(p, p + len, kind);
        self.prev = P_BEGIN;
        p + len
    }

    /// `(` at `p`: a call's arguments, a function's parameters, a closure's `use`, `catch`'s, an
    /// array's, a cast, or a group.
    #[inline(always)]
    fn lparen(&mut self, p: usize) -> usize {
        let (f, kind) = match self.prev {
            P_CALLEE => (F_CALL, GROUP_BEGIN),
            P_FNEND => (F_PARAMS, GROUP_BEGIN),
            P_FN_USE => (F_USE_VARS, GROUP_BEGIN),
            P_ARRAY => (F_GROUP, ARRAY_BEGIN),
            P_CLASS_HEAD => (F_CLASS_ARGS, GROUP_BEGIN),
            _ => {
                if let Some(e) = self.cast(p) {
                    return e;
                }
                (F_GROUP, GROUP_BEGIN)
            }
        };
        self.push(f);
        self.put(p, p + 1, kind);
        self.prev = P_BEGIN;
        p + 1
    }

    /// A cast at `p`'s `(`: `(int)`, `(string)`…, the type `storage.type`, the parentheses
    /// plain; where it is one, past its `)`.
    #[inline(always)]
    fn cast(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let s = skip(b, p + 1);
        let mut e = s;
        while e < b.len() && b[e].is_ascii_alphabetic() {
            e += 1;
        }
        if !(3..=7).contains(&(e - s)) {
            return None;
        }
        let q = skip(b, e);
        if b.get(q) != Some(&b')') {
            return None;
        }
        let mut w = [0u8; 7];
        for (d, &c) in w.iter_mut().zip(&b[s..e]) {
            *d = c.to_ascii_lowercase();
        }
        let w = &w[..e - s];
        if !matches!(
            w,
            b"array"
                | b"real"
                | b"double"
                | b"float"
                | b"int"
                | b"integer"
                | b"bool"
                | b"boolean"
                | b"string"
                | b"object"
                | b"binary"
                | b"unset"
        ) {
            return None;
        }
        self.put(s, e, STORAGE_TYPE);
        self.prev = P_BEGIN;
        Some(q + 1)
    }

    /// `)` at `p`.
    #[inline(always)]
    fn rparen(&mut self, p: usize) -> usize {
        if self.top() == F_DEFAULT {
            self.pop();
        }
        match self.top() {
            F_PARAMS | F_USE_VARS => {
                self.pop();
                self.put(p, p + 1, GROUP_END);
                self.prev = P_PARAMS_END;
            }
            F_CATCH => {
                self.pop();
                self.prev = P_BEGIN;
            }
            F_CALL | F_GROUP => {
                self.pop();
                self.put(p, p + 1, GROUP_END);
                self.prev = P_END;
            }
            F_CLASS_ARGS => {
                self.pop();
                self.put(p, p + 1, GROUP_END);
                self.prev = P_CLASS_HEAD;
            }
            _ => {
                self.put(p, p + 1, GROUP_END);
                self.prev = P_END;
            }
        }
        p + 1
    }

    /// A function's parameters from `p` to their `)`, a default value's `=`, a comment, or the
    /// line's end: types, `$name`s, `,`, `...`, `?`, `&` and comments; bat's list takes nothing
    /// else, so other bytes are plain (a `(` opens nothing).
    #[inline(never)]
    fn params(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        while p < n {
            let c = b[p];
            p = match ACT[usize::from(c)] {
                A_SPACE => p + 1,
                A_NAME => {
                    let mut e = p + 1;
                    while e < n && NAME[usize::from(b[e])] {
                        e += 1;
                    }
                    self.param_type(p, e)
                }
                A_DOLLAR => {
                    let mut s = p + 1;
                    while s < n && b[s] == b'$' {
                        s += 1;
                    }
                    if !b.get(s).is_some_and(|&c| name_start(c)) {
                        s
                    } else {
                        let mut e = s + 1;
                        while e < n && NAME[usize::from(b[e])] {
                            e += 1;
                        }
                        self.put(p, s, PARAM_SIGIL);
                        self.put(s, e, PARAM);
                        e
                    }
                }
                A_COMMA => {
                    self.put(p, p + 1, SEPARATOR);
                    p + 1
                }
                A_DOT if b.get(p + 1..p + 3) == Some(b"..") => {
                    self.put(p, p + 3, SPREAD);
                    p + 3
                }
                A_QUESTION => {
                    self.put(p, p + 1, NULLABLE);
                    p + 1
                }
                A_AMP => {
                    self.put(p, p + 1, REFERENCE);
                    p + 1
                }
                A_EQ => {
                    self.put(p, p + 1, ASSIGNMENT);
                    self.push(F_DEFAULT);
                    self.prev = P_BEGIN;
                    return p + 1;
                }
                A_RPAREN => {
                    self.pop();
                    self.put(p, p + 1, GROUP_END);
                    self.prev = P_PARAMS_END;
                    return p + 1;
                }
                A_SLASH if b.get(p + 1) == Some(&b'/') => return self.line_comment(p, SLASH_MARK),
                A_SLASH if b.get(p + 1) == Some(&b'*') => return self.comment_open(p),
                A_HASH if b.get(p + 1) == Some(&b'[') => return self.attribute(p),
                A_HASH => return self.line_comment(p, HASH_MARK),
                _ => p + char_len(c),
            };
        }
        p
    }

    /// An attribute's `#[` at `p` (PHP 8): its frame open, its names `variable.annotation`
    /// as Rust's, its arguments code.
    #[cold]
    #[inline(never)]
    fn attribute(&mut self, p: usize) -> usize {
        self.put(p, p + 2, ANNOT_BEGIN);
        self.push(F_ATTR);
        self.prev = P_ATTR;
        p + 2
    }

    /// `{` at `p`.
    #[inline(always)]
    fn lbrace(&mut self, p: usize) -> usize {
        let (f, kind) = match self.prev {
            P_CLASS_HEAD | P_IMPL_END => (F_CLASS, BLOCK_PLAIN),
            P_USE | P_USE_END => (F_USE_BLOCK, BLOCK_PLAIN),
            P_TRAIT_USE => (F_TRAIT_BLOCK, BLOCK_PLAIN),
            _ => (F_BLOCK, BLOCK_OPEN),
        };
        self.push(f);
        self.put(p, p + 1, kind);
        self.prev = if f == F_USE_BLOCK { P_USE } else { P_BEGIN };
        p + 1
    }

    /// `}` at `p`: a block's, a class's, a `use` list's, or a string's `{$ … }`, back in the
    /// string.
    #[inline(always)]
    fn rbrace(&mut self, p: usize) -> usize {
        let t = self.top();
        let (kind, prev) = match t {
            F_BLOCK => (BLOCK_CLOSE, P_BEGIN),
            F_CLASS | F_TRAIT_BLOCK => (BLOCK_PLAIN, P_BEGIN),
            F_USE_BLOCK => (BLOCK_PLAIN, P_USE_END),
            F_INTERP => {
                self.pop();
                self.put(p, p + 1, IN_EXPRESSION);
                self.mode = self.istr();
                return p + 1;
            }
            _ => {
                self.put(p, p + 1, BLOCK_CLOSE);
                self.prev = P_BEGIN;
                return p + 1;
            }
        };
        self.pop();
        self.put(p, p + 1, kind);
        self.prev = prev;
        p + 1
    }

    /// `,` at `p`: a separator in a call's arguments, parameters and the lists of `use`,
    /// `implements` and a class's traits, else plain.
    #[inline(always)]
    fn comma(&mut self, p: usize) -> usize {
        if self.top() == F_DEFAULT {
            self.pop();
        }
        if self.top() == F_ATTR {
            self.prev = P_ATTR;
            return p + 1;
        }
        let (sep, prev) = match self.prev {
            P_USE_END => (true, P_USE),
            P_IMPL_END => (true, P_IMPLEMENTS),
            P_TRAIT_USE => (true, P_TRAIT_USE),
            _ => (
                matches!(
                    self.top(),
                    F_CALL | F_PARAMS | F_USE_VARS | F_USE_BLOCK | F_CLASS_ARGS
                ),
                P_BEGIN,
            ),
        };
        if sep {
            self.put(p, p + 1, SEPARATOR);
        }
        self.prev = if self.top() == F_USE_BLOCK {
            P_USE
        } else {
            prev
        };
        p + 1
    }

    /// A line comment at `p`, its mark `mark` (one or two bytes), to the line's end or a `?>`.
    #[inline(always)]
    fn line_comment(&mut self, p: usize, mark: u16) -> usize {
        let b = self.b;
        let n = b.len();
        let (len, text) = if mark == SLASH_MARK {
            (2, SLASH_TEXT)
        } else {
            (1, HASH_TEXT)
        };
        self.put(p, p + len, mark);
        let mut q = p + len;
        loop {
            q = crate::prim::find_in(b, q, n, *b"?");
            if q + 1 >= n {
                self.put(p + len, n, text);
                return n;
            }
            if b[q + 1] == b'>' {
                self.put(p + len, q, text);
                return q;
            }
            q += 1;
        }
    }

    /// `/*` at `p`: a doc comment (`/**` alone on its line, or an inline `/** @… */`), or a
    /// block comment.
    #[inline(always)]
    fn comment_open(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        if b.get(p + 2) == Some(&b'*') {
            let mut q = p + 3;
            if b.get(q..q + 3) == Some(b"#@+") {
                q += 3;
            }
            if skip_space(b, q) == n {
                // `/**` (and `#@+`) and blanks to the line's end
                self.put(p, n, DOC_BEGIN);
                self.mode = M_DOC;
                return n;
            }
            if inline_doc(b, p + 3) {
                self.put(p, p + 3, DOC_BEGIN);
                self.mode = M_DOC;
                return p + 3;
            }
        }
        self.put(p, p + 2, BLOCK_BEGIN);
        self.mode = M_COMMENT;
        p + 2
    }

    /// A doc comment's line's start: a line that doesn't start with `*` ends the doc, the rest a
    /// block comment's; else its `*`.
    #[inline(always)]
    fn doc_line_start(&mut self) -> usize {
        let b = self.b;
        let s = skip_space(b, 0);
        if b.get(s) != Some(&b'*') {
            self.mode = M_COMMENT;
            return 0;
        }
        if b.get(s + 1) == Some(&b'/') {
            return 0;
        }
        self.put(0, s, DOC_TEXT);
        self.put(s, s + 1, DOC_STAR);
        let t = skip_space(b, s + 1);
        if b.get(t + 1) == Some(&b'a') && b[t..].starts_with(b"@access") {
            return self.access(s + 1, t);
        }
        s + 1
    }

    /// phpdoc's `@access` line, at `t` (bat's rule: the rest of the line its value, a
    /// modifier, or invalid), from `p`; where it isn't one, `p`.
    #[cold]
    #[inline(never)]
    fn access(&mut self, p: usize, t: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let v = t + 7;
        let w = skip_space(b, v);
        if w == v || w >= n {
            return p;
        }
        let mut end = n;
        while end > w && matches!(b[end - 1], b' ' | b'\t' | b'\r' | 0x0b | 0x0c) {
            end -= 1;
        }
        self.put(p, t, DOC_TEXT);
        self.put(t, v, DOC_TAG);
        self.put(v, w, DOC_TEXT);
        let kind = if matches!(&b[w..end], b"var" | b"public" | b"private" | b"protected") {
            DOC_MODIFIER
        } else {
            DOC_INVALID
        };
        self.put(w, end, kind);
        end
    }

    /// A block comment's text from `p` to its `*/`, or the line's end, the mode kept; in a doc
    /// comment its tags.
    #[inline(always)]
    fn block_comment(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let doc = self.mode == M_DOC;
        let (text, end) = if doc {
            (DOC_TEXT, DOC_END)
        } else {
            (BLOCK_TEXT, BLOCK_END)
        };
        let mut at = p;
        let mut q = p;
        loop {
            q = if doc {
                crate::prim::find_in(b, q, n, *b"*@")
            } else {
                crate::prim::find_in(b, q, n, *b"*")
            };
            if q + 1 >= n {
                self.put(at, n, text);
                return n;
            }
            if b[q] == b'@' {
                // the text so far first: where no tag follows, the next text joins it
                self.put(at, q, text);
                at = q;
                if let Some(e) = self.tag(q) {
                    at = e;
                    q = e;
                } else {
                    q += 1;
                }
                continue;
            }
            if b[q + 1] == b'/' {
                self.put(at, q, text);
                self.put(q, q + 2, end);
                self.mode = M_CODE;
                return q + 2;
            }
            q += 1;
        }
    }

    /// A doc comment's `@` at `q`: a phpdoc or PHPUnit tag, or `{@inheritdoc}`'s; where one,
    /// past it.
    #[cold]
    #[inline(never)]
    fn tag(&mut self, q: usize) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        let mut e = q + 1;
        while e < n && WORD[usize::from(b[e])] {
            e += 1;
        }
        if e == q + 1 {
            return None;
        }
        if q > 0 && b[q - 1] == b'{' && &b[q + 1..e] == b"inheritdoc" && b.get(e) == Some(&b'}') {
            self.put(q, e, DOC_TAG);
            return Some(e);
        }
        if &b[q + 1..e] == b"property" && b.get(e) == Some(&b'-') {
            let mut f = e + 1;
            while f < n && b[f].is_ascii_alphabetic() {
                f += 1;
            }
            if matches!(&b[e + 1..f], b"read" | b"write")
                && !b.get(f).is_some_and(|&c| WORD[usize::from(c)])
            {
                e = f;
            }
        }
        let id = TAGS.get(&b[q + 1..e]);
        if id == 0 {
            return None;
        }
        self.put(q, e, if id > UNIT_TAGS { DOC_UNIT } else { DOC_TAG });
        Some(e)
    }

    /// A single-quoted string's text from `p` to its end, or the line's end (the mode then
    /// [`M_SINGLE`]).
    #[inline(always)]
    fn single(&mut self, p: usize) -> usize {
        if self.head() {
            self.string_head(p);
        }
        if self.sql() {
            self.mode = M_SINGLE;
            return self.sql_string(p, SINGLE);
        }
        let b = self.b;
        let n = b.len();
        let mut at = p;
        let mut q = p;
        loop {
            q = crate::prim::find_in(b, q, n, *b"'\\");
            if q >= n {
                self.put(at, n, SINGLE);
                self.mode = M_SINGLE;
                return n;
            }
            if b[q] == b'\'' {
                self.put(at, q, SINGLE);
                self.put(q, q + 1, SINGLE + END);
                self.mode = M_CODE;
                return q + 1;
            }
            if matches!(b.get(q + 1), Some(b'\\' | b'\'')) {
                self.put(at, q, SINGLE);
                self.put(q, q + 2, SINGLE + ESCAPE);
                at = q + 2;
                q += 2;
            } else {
                q += 1;
            }
        }
    }

    /// A double-quoted or backtick string's text, or a here-document's line (`base` its kinds),
    /// from `p`: escapes and interpolation, to its end, a `{$`, or the line's end (the mode
    /// kept).
    #[inline(always)]
    fn double(&mut self, p: usize, base: u16) -> usize {
        if base == DOUBLE && self.head() {
            self.string_head(p);
        }
        if self.sql() {
            return self.sql_string(p, base);
        }
        let b = self.b;
        let n = b.len();
        let quote = match base {
            DOUBLE => b'"',
            BACKTICK => b'`',
            _ => 0,
        };
        let mut at = p;
        let mut q = p;
        loop {
            q = match base {
                DOUBLE => crate::prim::find_in(b, q, n, *b"\"\\${"),
                BACKTICK => crate::prim::find_in(b, q, n, *b"`\\${"),
                _ => crate::prim::find_in(b, q, n, *b"\\${"),
            };
            if q >= n {
                self.put(at, n, base);
                return n;
            }
            let c = b[q];
            if c == quote {
                self.put(at, q, base);
                self.put(q, q + 1, base + END);
                self.mode = M_CODE;
                self.prev = P_END;
                return q + 1;
            }
            match c {
                b'\\' => {
                    let (e, kind) = if base == BACKTICK {
                        let e = match b.get(q + 1) {
                            Some(&d) => q + 1 + char_len(d),
                            None => q + 1,
                        };
                        (e.min(n), base + ESCAPE)
                    } else {
                        escape(
                            b,
                            q,
                            if base == DOUBLE {
                                DOUBLE + ESCAPE
                            } else {
                                HEREDOC_ESCAPE
                            },
                        )
                    };
                    if kind != PLAIN {
                        self.put(at, q, base);
                        self.put(q, e, kind);
                        at = e;
                    }
                    q = e.max(q + 1);
                }
                b'$' => {
                    // the text so far first: where no variable follows, the next text joins it
                    self.put(at, q, base);
                    at = q;
                    let e = self.interpolated(q);
                    if e == q {
                        q += 1;
                        continue;
                    }
                    if self.mode == M_CODE {
                        // a `${ … }` opened
                        return e;
                    }
                    at = e;
                    q = e;
                }
                _ => {
                    // `{`: an expression where a `$` follows and a `}` comes later on the line
                    if b.get(q + 1) == Some(&b'$') && crate::prim::find_in(b, q + 2, n, *b"}") < n {
                        self.put(at, q, base);
                        self.put(q, q + 1, IN_EXPRESSION);
                        self.open_interp(base);
                        return q + 1;
                    }
                    q += 1;
                }
            }
        }
    }

    /// The interpolation frame for a string of `base`'s kinds, the code mode on.
    #[inline(always)]
    fn open_interp(&mut self, base: u16) {
        self.push(F_INTERP);
        self.set_istr(match base {
            DOUBLE => M_DOUBLE,
            BACKTICK => M_BACKTICK,
            _ => M_HEREDOC,
        });
        self.mode = M_CODE;
        self.prev = P_BEGIN;
    }

    /// A `$` at `q` in a string: a variable, `$name->member`, `$name[index]`, `${name}` or
    /// `${ expression }` (the code mode on, its frame open); where one, past it (`q` where not).
    #[inline(always)]
    fn interpolated(&mut self, q: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let base = match self.mode {
            M_DOUBLE => DOUBLE,
            M_BACKTICK => BACKTICK,
            _ => HEREDOC,
        };
        if b.get(q + 1) == Some(&b'{') {
            // `${name}` or `${ expression }`, where a `}` follows
            let s = q + 2;
            let mut e = s;
            while e < n && NAME[usize::from(b[e])] {
                e += 1;
            }
            if e > s && name_start(b[s]) && b.get(e) == Some(&b'}') {
                self.put(q, s, IN_VAR_BRACE);
                self.put(s, e, IN_VARIABLE);
                self.put(e, e + 1, IN_VAR_BRACE);
                return e + 1;
            }
            if crate::prim::find_in(b, s, n, *b"}") < n {
                self.put(q, s, IN_VAR_BRACE);
                self.open_interp(base);
                return s;
            }
            return q;
        }
        let mut s = q + 1;
        while s < n && b[s] == b'$' {
            s += 1;
        }
        if !b.get(s).is_some_and(|&c| name_start(c)) {
            return q;
        }
        let mut e = s + 1;
        while e < n && NAME[usize::from(b[e])] {
            e += 1;
        }
        let (sigil, kind) = variable_kinds(&b[s..e]);
        let (sigil, kind) = if sigil == SIGIL {
            (IN_SIGIL, IN_VARIABLE)
        } else {
            (sigil, kind)
        };
        self.put(q, s, sigil);
        self.put(s, e, kind);
        // `->name`, or `[index]`
        if b.get(e..e + 2) == Some(b"->") && b.get(e + 2).is_some_and(|&c| name_start(c)) {
            let mut f = e + 3;
            while f < n && NAME[usize::from(b[f])] {
                f += 1;
            }
            self.put(e, e + 2, IN_ARROW);
            self.put(e + 2, f, IN_MEMBER);
            return f;
        }
        if b.get(e) == Some(&b'[')
            && let Some(f) = self.simple_index(e)
        {
            return f;
        }
        e
    }

    /// `[index]` at `e` in a string, after a variable: a number, a variable or a name; where
    /// one, past its `]`.
    #[inline(always)]
    fn simple_index(&mut self, e: usize) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        let s = e + 1;
        let mut f = s;
        let kind = if b.get(f).is_some_and(u8::is_ascii_digit) {
            while f < n && b[f].is_ascii_digit() {
                f += 1;
            }
            INT
        } else {
            let dollar = b.get(f) == Some(&b'$');
            if dollar {
                f += 1;
            }
            if !b.get(f).is_some_and(|&c| name_start(c)) {
                return None;
            }
            while f < n && NAME[usize::from(b[f])] {
                f += 1;
            }
            if dollar { VARIABLE } else { CONSTANT }
        };
        if b.get(f) != Some(&b']') {
            return None;
        }
        self.put(e, s, BRACKETS_BEGIN);
        if kind == VARIABLE {
            self.put(s, s + 1, SIGIL);
            self.put(s + 1, f, VARIABLE);
        } else {
            self.put(s, f, kind);
        }
        self.put(f, f + 1, BRACKETS_END);
        Some(f + 1)
    }

    /// `<<<` at `p`: a here-document's or now-document's opener (its label alone on the rest of
    /// the line), the body from the next line; where one, the line's end.
    #[cold]
    #[inline(never)]
    fn heredoc_open(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        let mut s = skip(b, p + 3);
        let quoted = b.get(s) == Some(&b'\'');
        if quoted {
            s += 1;
        }
        if !b.get(s).is_some_and(|&c| name_start(c)) {
            return None;
        }
        let mut e = s + 1;
        while e < n && NAME[usize::from(b[e])] {
            e += 1;
        }
        let mut f = e;
        if quoted {
            if b.get(f) != Some(&b'\'') {
                return None;
            }
            f += 1;
        } else if b.get(f) == Some(&b'\'') {
            f += 1;
        }
        if skip_space(b, f) != n {
            return None;
        }
        let label = &b[s..e];
        let lang = matches!(
            label,
            b"HTML" | b"XML" | b"SQL" | b"JAVASCRIPT" | b"JSON" | b"CSS"
        );
        // `<<<"…"` and a label's `'` without its pair are no language's opener
        if lang && (quoted || f == e) {
            // bat's: `<<<` and the label's blanks the opener's; JSON's `<<<` the operator
            let json = label == b"JSON";
            let (op, lab) = match (quoted, json) {
                (false, false) => (EMBED_OPEN, EMBED_HEREDOC),
                (true, false) => (EMBED_OPEN, EMBED_NOWDOC),
                (false, true) => (HEREDOC_OPERATOR, EMBED_OPEN),
                (true, true) => (NOWDOC_OPERATOR, EMBED_OPEN),
            };
            let ls = if quoted { s - 1 } else { s };
            self.put(p, p + 3, op);
            self.put(p + 3, ls, EMBED_OPEN);
            self.put(ls, f, lab);
            self.put(f, n, EMBED_OPEN);
            self.mode = if quoted { M_NOWDOC } else { M_HEREDOC };
            let embed = match label {
                b"SQL" => {
                    self.set_sql(true);
                    self.set_sql_ctx(crate::sql::MAIN);
                    E_SQL
                }
                b"JSON" => {
                    self.set_json_comment(false);
                    E_JSON
                }
                // HTML's state in the template's place while the body lasts
                b"HTML" => {
                    self.host().html = State::default();
                    E_HTML
                }
                _ => E_TEXT,
            };
            self.set_embed(embed);
            self.set_label(Label::of(label).bits());
            self.prev = P_END;
            return Some(n);
        }
        self.set_embed(E_TEXT);
        if quoted {
            self.put(p, p + 3, NOWDOC_OPEN);
            self.put(p + 3, s - 1, NOWDOC);
            self.put(s - 1, f, NOWDOC_LABEL);
            self.put(f, n, NOWDOC);
            self.mode = M_NOWDOC;
        } else {
            self.put(p, p + 3, HEREDOC_OPEN);
            self.put(p + 3, s, HEREDOC);
            self.put(s, f, HEREDOC_LABEL);
            self.put(f, n, HEREDOC);
            self.mode = M_HEREDOC;
        }
        self.set_label(Label::of(&b[s..e]).bits());
        self.prev = P_END;
        Some(n)
    }

    /// A here-document's line's start: its closer (the label after blanks, not a name's start),
    /// the code after it; else its body.
    #[inline(always)]
    fn heredoc_line_start(&mut self) -> usize {
        let b = self.b;
        let Some(c) = Label::from_bits(self.label()).close(b, Lead::Blanks, Tail::NonWord) else {
            return 0;
        };
        self.put(0, c.start, PLAIN);
        self.put(c.start, c.end, HEREDOC_CLOSE);
        self.mode = M_CODE;
        if self.embed() == E_HTML
            && let Some(h) = self.host.as_mut()
        {
            h.html = State::default();
        }
        self.set_embed(E_TEXT);
        self.set_sql(false);
        self.prev = P_END;
        let e = c.end;
        if b.get(e) == Some(&b';') && skip_space(b, e + 1) == b.len() {
            self.put(e, e + 1, TERMINATOR);
            self.prev = P_BEGIN;
            return e + 1;
        }
        e
    }

    /// A string's head from `p`, where it has seen nothing but blanks: SQL where bat's
    /// `sql_indicator` holds; the head ends at the line's first non-blank.
    #[inline(always)]
    fn string_head(&mut self, p: usize) {
        let b = self.b;
        let q = skip_space(b, p);
        if q == b.len() {
            return;
        }
        self.set_head(false);
        if sql_indicator(b, q) {
            self.set_sql(true);
            self.set_sql_ctx(crate::sql::MAIN);
        }
    }

    /// A string's (or here-document's, `base` its kinds) SQL from `p`: the host's patterns
    /// first at each place they can start (its end, escapes, interpolation), SQL's scanner
    /// between, to the string's end, a `{$`, or the line's end (SQL's context kept).
    #[cold]
    #[inline(never)]
    fn sql_string(&mut self, mut p: usize, base: u16) -> usize {
        let b = self.b;
        let n = b.len();
        let (yields, sql, quote) = match base {
            SINGLE => (&YIELDS_SINGLE, SQL_SINGLE, b'\''),
            DOUBLE => (&YIELDS_DOUBLE, SQL_DOUBLE, b'"'),
            HEREDOC => (&YIELDS_HEREDOC, SQL_HEREDOC, 0),
            _ => (&YIELDS_NONE, SQL_HEREDOC, 0),
        };
        let embed = crate::sql::Embed {
            yields,
            base: sql,
            text: sql,
        };
        while p < n {
            let c = b[p];
            if quote != 0 && c == quote {
                self.put(p, p + 1, base + END);
                self.mode = M_CODE;
                self.set_sql(false);
                self.prev = P_END;
                return p + 1;
            }
            if yields[usize::from(c)] {
                // the string's own patterns
                match c {
                    b'\\' if base == SINGLE => {
                        if matches!(b.get(p + 1), Some(b'\\' | b'\'')) {
                            self.put(p, p + 2, SINGLE + ESCAPE);
                            p += 2;
                            continue;
                        }
                    }
                    b'\\' => {
                        let (e, kind) = escape(
                            b,
                            p,
                            if base == DOUBLE {
                                DOUBLE + ESCAPE
                            } else {
                                HEREDOC_ESCAPE
                            },
                        );
                        if kind != PLAIN {
                            self.put(p, e, kind);
                            p = e;
                            continue;
                        }
                    }
                    b'$' => {
                        let e = self.interpolated(p);
                        if e != p {
                            if self.mode == M_CODE {
                                return e;
                            }
                            p = e;
                            continue;
                        }
                    }
                    b'{' if b.get(p + 1) == Some(&b'$')
                        && crate::prim::find_in(b, p + 2, n, *b"}") < n =>
                    {
                        self.put(p, p + 1, IN_EXPRESSION);
                        self.open_interp(base);
                        return p + 1;
                    }
                    _ => {}
                }
            }
            // SQL's turn, to the next place the string's patterns can start
            self.sql_keep.at = p;
            self.k.close(self.out);
            let r = crate::sql::embedded(
                b,
                p,
                self.sql_ctx(),
                &embed,
                &mut self.sql_keep,
                &mut *self.out,
            );
            self.k.reopen(self.out, 16);
            let Some((q, ctx)) = r else {
                return n;
            };
            self.set_sql_ctx(ctx);
            let q = q.min(n);
            if self.sql_keep.at < q {
                self.put(self.sql_keep.at, q, sql);
            }
            if q <= p {
                // no progress: the character is SQL's text
                let e = (p + char_len(b[p])).min(n);
                self.put(p, e, sql);
                p = e;
            } else {
                p = q;
            }
        }
        // SQL's turn at the line's end, where its line comments end; the text before is written
        self.sql_keep.at = n;
        self.k.close(self.out);
        let r = crate::sql::embedded(
            b,
            n,
            self.sql_ctx(),
            &embed,
            &mut self.sql_keep,
            &mut *self.out,
        );
        self.k.reopen(self.out, 16);
        if let Some((_, ctx)) = r {
            self.set_sql_ctx(ctx);
        }
        n
    }

    /// A regex string at `p`'s quote, its closing delimiter at `d` and its quote at `f` (bat's
    /// `regex-single-quoted` and `-double-quoted`: the whole string `/…/flags` on the line):
    /// the regex scanner's, Java's dialect nearest PCRE. In a single-quoted string a `\` and
    /// a `\\` are both the regex's `\`; in a double-quoted one the string's escapes and
    /// variables come first.
    #[cold]
    #[inline(never)]
    fn regex_string(&mut self, p: usize, d: usize, f: usize, base: u16) -> usize {
        let b = self.b;
        let double = base == DOUBLE;
        let (re, delim) = if double {
            (REGEX_DOUBLE, RE_DOUBLE_BEGIN)
        } else {
            (REGEX_SINGLE, RE_SINGLE_BEGIN)
        };
        self.put(p, p + 1, base + BEGIN);
        self.put(p + 1, p + 2, delim);
        let embed = crate::regex::Embed {
            dialect: crate::regex::Dialect::Java,
            cooked: !double,
            yields: if double { &YIELDS_REGEX } else { &YIELDS_NONE },
            stop: |_, _, _| false,
            arg: 0,
            escape: |_, p| p,
            top_only: false,
            continues: false,
            base: re,
            text: re,
        };
        let line = &b[..d];
        let (mut q, mut st, mut at) = (p + 2, 0u8, p + 2);
        while q < d {
            if double && YIELDS_REGEX[usize::from(b[q])] {
                // the string's own patterns first
                if b[q] == b'\\' {
                    let (e, kind) = escape(line, q, DOUBLE + ESCAPE);
                    if kind != PLAIN {
                        self.put(at, q, re);
                        self.put(q, e, kind);
                        (q, at) = (e, e);
                        continue;
                    }
                } else {
                    self.put(at, q, re);
                    at = q;
                    let mode = self.mode;
                    self.mode = M_DOUBLE;
                    let e = self.interpolated_in(line, q);
                    self.mode = mode;
                    if e > q {
                        (q, at) = (e, e);
                        continue;
                    }
                }
            }
            self.k.close(self.out);
            let r = crate::regex::embedded(line, q, st, &embed, &mut at, &mut *self.out);
            self.k.reopen(self.out, 16);
            let Some((e, s)) = r else {
                break;
            };
            st = s;
            q = if e > q { e } else { q + char_len(b[q]) };
        }
        self.put(at, d, re);
        self.put(d, d + 1, delim + 1);
        self.put(d + 1, f, base);
        self.put(f, f + 1, base + END);
        self.prev = P_END;
        f + 1
    }

    /// [`Scan::interpolated`] on `line` (a regex string's, cut at its delimiter): a variable
    /// only, a `${ … }` or `{$ … }` there the regex's text.
    #[inline(always)]
    fn interpolated_in(&mut self, line: &'b [u8], q: usize) -> usize {
        if line.get(q) != Some(&b'$') || line.get(q + 1) == Some(&b'{') {
            return q;
        }
        let b = self.b;
        self.b = line;
        let e = self.interpolated(q);
        self.b = b;
        e
    }

    /// A `<<<HTML` here-document's line from `p`: the line's HTML lexed once by HTML's scanner
    /// with its escapes and interpolation blanked (bat's prototype takes them first), its spans
    /// written, and the interpolation where it was blanked (a `{$` leaves the body for its
    /// code). In a now-document, all HTML's.
    #[cold]
    #[inline(never)]
    fn markup_body(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let now = self.mode == M_NOWDOC;
        if !self.host.as_ref().is_some_and(|h| h.line) {
            let mut buf = b.to_vec();
            buf[..p].fill(b' ');
            let mut q = p;
            while !now && q < n {
                let (s, e) = next_interpolation(b, q);
                if s >= n {
                    break;
                }
                buf[s..e].fill(b' ');
                q = e;
            }
            let h = self.host();
            h.spans.clear();
            crate::html::line(&buf, &mut h.html, &mut Out::new(&mut h.spans));
            h.line = true;
            h.next = 0;
        }
        let (s, e) = if now {
            (n, n)
        } else {
            next_interpolation(b, p)
        };
        self.write_html(p, s.min(n));
        if s >= n {
            return n;
        }
        // the interpolation, as a here-document's
        match b[s] {
            b'\\' => {
                let (f, kind) = escape(b, s, HEREDOC_ESCAPE);
                self.put(s, f, kind);
                f.max(s + 1)
            }
            b'{' => {
                self.put(s, s + 1, IN_EXPRESSION);
                self.open_interp(HEREDOC);
                s + 1
            }
            _ => {
                let f = self.interpolated(s);
                if f == s {
                    self.write_html(s, e);
                    e
                } else {
                    f
                }
            }
        }
    }

    /// A JSON here-document's line from `p` (its start): JSON's lexer on it, its kinds moved to
    /// PHP's.
    #[cold]
    #[inline(never)]
    fn json(&mut self, p: usize) -> usize {
        let b = self.b;
        self.k.close(self.out);
        let from = self.out.0.len();
        let mut comment = self.json_comment();
        crate::json::embedded(&b[p..], &mut comment, self.out);
        self.set_json_comment(comment);
        for s in &mut self.out.0[from..] {
            s.start += p as u32;
            s.end += p as u32;
            s.kind |= JSON;
        }
        self.k.reopen(self.out, 16);
        b.len()
    }

    /// A here-document's or now-document's body from `p` to the line's end.
    #[inline(always)]
    fn heredoc_body(&mut self, p: usize) -> usize {
        if self.embed() == E_JSON {
            return self.json(p);
        }
        if self.embed() == E_HTML {
            return self.markup_body(p);
        }
        if self.embed() == E_SQL {
            return self.sql_string(
                p,
                if self.mode == M_NOWDOC {
                    NOWDOC
                } else {
                    HEREDOC
                },
            );
        }
        if self.mode == M_NOWDOC {
            let n = self.b.len();
            self.put(p, n, NOWDOC);
            return n;
        }
        self.double(p, HEREDOC)
    }

    /// A variable at `p`'s `$`, or a variable's call (`$f(`).
    #[inline(always)]
    fn variable(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut s = p + 1;
        while s < n && b[s] == b'$' {
            s += 1;
        }
        if !b.get(s).is_some_and(|&c| name_start(c)) {
            self.prev = P_BEGIN;
            return s;
        }
        let mut e = s + 1;
        while e < n && NAME[usize::from(b[e])] {
            e += 1;
        }
        let (sigil, kind) = match self.top() {
            F_PARAMS | F_USE_VARS => (PARAM_SIGIL, PARAM),
            _ => match self.prev {
                P_ARROW => (MEMBER_SIGIL, MEMBER),
                P_DCOLON => (CLASS_VAR_SIGIL, CLASS_VAR),
                _ => variable_kinds(&b[s..e]),
            },
        };
        self.put(p, s, sigil);
        self.put(s, e, kind);
        self.prev = if kind == VARIABLE && next_byte(b, e) == b'(' {
            P_CALLEE
        } else {
            P_END
        };
        e
    }

    /// A name at `p`: a keyword, or a name by where it stands and the byte after it.
    #[inline(always)]
    fn name(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut e = p + 1;
        while e < n && NAME[usize::from(b[e])] {
            e += 1;
        }
        let prev = self.prev;
        if prev == P_ARROW {
            let callee = next_byte(b, e) == b'(';
            self.put(p, e, if callee { CALL } else { MEMBER });
            self.prev = if callee { P_CALLEE } else { P_END };
            return e;
        }
        if prev == P_DCOLON {
            let callee = next_byte(b, e) == b'(';
            let kind = if callee {
                CALL
            } else if &b[p..e] == b"class" {
                CLASS_KEYWORD
            } else {
                CLASS_CONSTANT
            };
            self.put(p, e, kind);
            self.prev = if callee { P_CALLEE } else { P_END };
            return e;
        }
        if prev == P_ATTR {
            // an attribute's name, its path whole
            let mut f = e;
            while f + 1 < n && b[f] == b'\\' && name_start(b[f + 1]) {
                f += 2;
                while f < n && NAME[usize::from(b[f])] {
                    f += 1;
                }
            }
            self.put(p, f, ANNOT_NAME);
            self.prev = if next_byte(b, f) == b'(' {
                P_CALLEE
            } else {
                P_END
            };
            return f;
        }
        // a path's part: `A\` (the prev kept for its last)
        if b.get(e) == Some(&b'\\') && b.get(e + 1).is_some_and(|&c| name_start(c)) {
            let kind = if prev == P_NS {
                NS_DECL_PART
            } else {
                NAMESPACE
            };
            self.put(p, e, kind);
            return e;
        }
        match self.top() {
            F_PARAMS => return self.param_type(p, e),
            F_CATCH => {
                self.put(p, e, EXCEPTION_CLASS);
                return e;
            }
            F_USE_VARS => return e,
            F_TRAIT_BLOCK => {
                let w = &b[p..e];
                if w.eq_ignore_ascii_case(b"as") {
                    self.put(p, e, USE_AS);
                } else if w.eq_ignore_ascii_case(b"insteadof") {
                    self.put(p, e, INSTEADOF);
                }
                return e;
            }
            _ => {}
        }
        if prev >= P_FUNC
            && let Some(e) = self.name_after(p, e)
        {
            return e;
        }
        let k = KEYWORDS.value(&b[p..e], &KW_INFO);
        if k != W_NONE {
            return self.keyword(p, e, k);
        }
        self.bare(p, e);
        e
    }

    /// A name `b[p..e]` that isn't a keyword, in an operand's place: a call's, a class's before
    /// `::`, a label, or a constant.
    #[inline(always)]
    fn bare(&mut self, p: usize, e: usize) {
        let b = self.b;
        let q = skip(b, e);
        let (kind, prev) = match b.get(q) {
            Some(b'(') => (self.call(p, e), P_CALLEE),
            Some(b':') if b.get(q + 1) == Some(&b':') => (self.class_name(p, e), P_END),
            Some(b':') if self.label_at(p) => {
                self.put(p, e, LABEL);
                self.put(q, q + 1, SEPARATOR);
                self.prev = P_BEGIN;
                return;
            }
            _ => (CONST_KIND[usize::from(CONSTS.get(&b[p..e]))], P_END),
        };
        self.put(p, e, kind);
        self.prev = prev;
    }

    /// A call's name `b[p..e]`: a support function's (not after a namespace's part), or a
    /// function's.
    #[inline(always)]
    fn call(&self, p: usize, e: usize) -> u16 {
        let b = self.b;
        let w = &b[p..e];
        if p >= 2 && b[p - 1] == b'\\' && NAME[usize::from(b[p - 2])] {
            return CALL;
        }
        if (w[0] | 0x20) <= b'm' {
            FUNCS_A.value(w, &FUNC_KIND_A)
        } else {
            FUNCS_B.value(w, &FUNC_KIND_B)
        }
    }

    /// A class's name `b[p..e]`: `self`, `static`, `parent`, a builtin class's, or a class's.
    #[inline(always)]
    fn class_name(&self, p: usize, e: usize) -> u16 {
        let w = &self.b[p..e];
        if matches!(w, b"self" | b"static" | b"parent") {
            LANGUAGE
        } else if CLASSES.get(w) != 0 {
            BUILTIN_CLASS
        } else {
            SUPPORT_CLASS
        }
    }

    /// Whether a name at `p` before a `:` is a label: first on its line, where statements
    /// are, not `default…` nor `else…`.
    #[cold]
    #[inline(never)]
    fn label_at(&self, p: usize) -> bool {
        let b = self.b;
        skip_space(b, 0) == p
            && matches!(self.top(), F_NONE | F_BLOCK | F_CLASS)
            && !b[p..].starts_with(b"default")
            && !b[p..].starts_with(b"else")
    }

    /// A name `b[p..e]` in a function's parameters: a type (`storage.type` for the basic ones,
    /// else a class's), or a default value's.
    #[inline(always)]
    fn param_type(&mut self, p: usize, e: usize) -> usize {
        let w = &self.b[p..e];
        let kind = if matches!(
            w,
            b"array" | b"callable" | b"int" | b"string" | b"bool" | b"float" | b"object"
        ) {
            STORAGE_TYPE
        } else {
            SUPPORT_CLASS
        };
        self.put(p, e, kind);
        e
    }

    /// A name `b[p..e]` where [`Scan::prev`] says what it is: a function's, a class's, a type,
    /// a `use` path's…; `None` where it's an operand's after all, [`Scan::prev`] set for
    /// [`Scan::name`]'s keywords and names to go on (no second copy of them here).
    #[inline(never)]
    fn name_after(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        let w = &b[p..e];
        let (kind, prev) = match self.prev {
            P_FUNC => {
                let kind = if w.starts_with(b"__") && MAGIC_WORDS.get(w) != 0 {
                    MAGIC
                } else {
                    FUNCTION_NAME
                };
                (kind, P_FNEND)
            }
            P_CLASS => (CLASS_NAME, P_CLASS_HEAD),
            P_IFACE => (INTERFACE_NAME, P_IFACE_HEAD),
            P_TRAIT => (TRAIT_NAME, P_BEGIN),
            P_NS => (NS_DECL, P_BEGIN),
            P_NEW => {
                if w.eq_ignore_ascii_case(b"class") {
                    (CLASS_TYPE, P_CLASS_HEAD)
                } else {
                    (self.class_name(p, e), P_END)
                }
            }
            P_INSTANCEOF => {
                let kind = if CLASSES.get(w) != 0 {
                    BUILTIN_CLASS
                } else {
                    SUPPORT_CLASS
                };
                (kind, P_END)
            }
            P_EXTENDS => (inherited(w), P_CLASS_HEAD),
            P_IMPLEMENTS => (inherited(w), P_IMPL_END),
            P_IFACE_EXTENDS => (inherited(w), P_BEGIN),
            P_TRAIT_USE => (INHERITED, P_TRAIT_USE),
            P_CLASS_HEAD | P_IMPL_END | P_IFACE_HEAD => {
                if w.eq_ignore_ascii_case(b"extends") {
                    if self.prev == P_IFACE_HEAD {
                        // an interface's `\b(extends)\b\s*` scopes its blanks (a class's doesn't)
                        let q = skip_space(b, e);
                        self.put(p, q, EXTENDS);
                        self.prev = P_IFACE_EXTENDS;
                        return Some(q);
                    }
                    (EXTENDS, P_EXTENDS)
                } else if w.eq_ignore_ascii_case(b"implements") && self.prev != P_IFACE_HEAD {
                    (IMPLEMENTS, P_IMPLEMENTS)
                } else if self.prev == P_IFACE_HEAD {
                    self.prev = P_BEGIN;
                    return None;
                } else {
                    (PLAIN, self.prev)
                }
            }
            P_RETURN => {
                let kind = if matches!(
                    w,
                    b"array" | b"callable" | b"int" | b"string" | b"bool" | b"float" | b"object"
                ) {
                    STORAGE_TYPE
                } else if CLASSES.get(w) != 0 {
                    BUILTIN_CLASS
                } else {
                    SUPPORT_CLASS
                };
                (kind, P_RETURN_END)
            }
            P_PROP => match w {
                b"const" | b"function" => {
                    self.prev = P_BEGIN;
                    return None;
                }
                b"var" | b"public" | b"private" | b"protected" | b"static" => (MODIFIER, P_PROP),
                _ => (SUPPORT_CLASS, P_BEGIN),
            },
            P_USE | P_USE_END => {
                if w.eq_ignore_ascii_case(b"as") {
                    (USE_AS, P_USE_AS)
                } else {
                    let kind = match self.using() {
                        U_CLASS => SUPPORT_CLASS,
                        U_FUNCTION => USE_FUNCTION,
                        _ => CONSTANT,
                    };
                    (kind, P_USE_END)
                }
            }
            P_USE_AS => {
                let kind = match self.using() {
                    U_CLASS => CLASS_NAME,
                    U_FUNCTION => FUNCTION_NAME,
                    _ => CONSTANT,
                };
                (kind, P_USE_END)
            }
            P_GOTO => (GOTO_LABEL, P_BEGIN),
            _ => {
                // after a function's name, its parameters, a return type, a call's name,
                // `use`, `array`: an operand's place
                self.prev = if self.prev == P_RETURN_END || self.prev == P_PARAMS_END {
                    if w.eq_ignore_ascii_case(b"use") && self.prev == P_PARAMS_END {
                        self.put(p, e, FN_USE);
                        self.prev = P_FN_USE;
                        return Some(e);
                    }
                    P_BEGIN
                } else {
                    P_END
                };
                return None;
            }
        };
        self.put(p, e, kind);
        self.prev = prev;
        Some(e)
    }

    /// A keyword `b[p..e]`, `k`.
    #[inline(always)]
    fn keyword(&mut self, p: usize, e: usize, k: u8) -> usize {
        let b = self.b;
        // bat's `::` lookahead comes before its types, modifiers and constants: `Double::for()`
        if k >= W_TYPE && k != W_STATIC {
            let q = skip(b, e);
            if b.get(q) == Some(&b':') && b.get(q + 1) == Some(&b':') {
                self.bare(p, e);
                return e;
            }
        }
        let (kind, prev) = match k {
            W_CONTROL => (CONTROL, P_BEGIN),
            W_EXCEPTION => (EXCEPTION, P_BEGIN),
            W_TYPE => (STORAGE_TYPE, P_BEGIN),
            W_MODIFIER => {
                if self.top() == F_CLASS
                    && matches!(&b[p..e], b"public" | b"private" | b"protected")
                {
                    (MODIFIER, P_PROP)
                } else if matches!(&b[p..e], b"abstract" | b"final") && next_word_is(b, e, b"class")
                {
                    // bat's `(\b(?:abstract|final)\s+)` takes the blanks
                    let q = skip_space(b, e);
                    self.put(p, q, ABSTRACT);
                    self.prev = P_BEGIN;
                    return q;
                } else {
                    (MODIFIER, P_BEGIN)
                }
            }
            W_VAR => {
                if self.top() == F_CLASS && &b[p..e] == b"var" {
                    (MODIFIER, P_PROP)
                } else {
                    self.bare(p, e);
                    return e;
                }
            }
            W_STATIC => {
                if next_byte(b, e) == b':' {
                    (LANGUAGE, P_END)
                } else if self.top() == F_CLASS && &b[p..e] == b"static" {
                    (MODIFIER, P_PROP)
                } else {
                    (MODIFIER, P_BEGIN)
                }
            }
            W_FUNCTION => {
                let q = skip(b, e);
                let q = if b.get(q) == Some(&b'&') {
                    skip(b, q + 1)
                } else {
                    q
                };
                match b.get(q) {
                    Some(&c) if name_start(c) => (FUNCTION_KEYWORD, P_FUNC),
                    Some(b'(') => (FUNCTION_KEYWORD, P_FNEND),
                    _ => (STORAGE_TYPE, P_BEGIN),
                }
            }
            W_CLASS => {
                if next_byte(b, e) != 0 && b.get(skip(b, e)).is_some_and(|&c| name_start(c)) {
                    (CLASS_TYPE, P_CLASS)
                } else {
                    (STORAGE_TYPE, P_BEGIN)
                }
            }
            W_INTERFACE => {
                if skip_space(b, 0) == p && b.get(skip(b, e)).is_some_and(|&c| name_start(c)) {
                    (INTERFACE_TYPE, P_IFACE)
                } else {
                    (STORAGE_TYPE, P_BEGIN)
                }
            }
            W_TRAIT => {
                if skip_space(b, 0) == p && b.get(skip(b, e)).is_some_and(|&c| name_start(c)) {
                    (TRAIT_TYPE, P_TRAIT)
                } else {
                    self.bare(p, e);
                    return e;
                }
            }
            W_NAMESPACE => {
                if skip_space(b, 0) == p {
                    (NAMESPACE_KEYWORD, P_NS)
                } else {
                    self.bare(p, e);
                    return e;
                }
            }
            W_USE => match self.top() {
                F_NONE | F_BLOCK => {
                    let q = skip(b, e);
                    let (using, kind_after) = if next_word_is(b, e, b"function") {
                        (U_FUNCTION, true)
                    } else if next_word_is(b, e, b"const") {
                        (U_CONST, true)
                    } else {
                        (U_CLASS, false)
                    };
                    self.set_using(using);
                    if kind_after {
                        self.put(p, e, USE);
                        let mut f = q;
                        while f < b.len() && NAME[usize::from(b[f])] {
                            f += 1;
                        }
                        self.put(q, f, STORAGE_TYPE);
                        self.prev = P_USE;
                        return f;
                    }
                    (USE, P_USE)
                }
                F_CLASS => (USE, P_TRAIT_USE),
                _ => (CONTROL, P_BEGIN),
            },
            W_NEW => (NEW, P_NEW),
            W_INSTANCEOF => (TYPE_OP, P_INSTANCEOF),
            W_LOGICAL => (LOGICAL, P_BEGIN),
            W_CONSTANT => (LANG_CONSTANT, P_END),
            W_SELF => (LANGUAGE, P_END),
            W_ECHO => (CONSTRUCT, P_BEGIN),
            W_INCLUDE => (INCLUDE, P_BEGIN),
            W_EXTENDS => (MODIFIER, P_BEGIN),
            W_IMPLEMENTS => (MODIFIER, P_BEGIN),
            W_CONSTRUCT => {
                if next_byte(b, e) == b'(' {
                    (CONSTRUCT, P_CALLEE)
                } else {
                    self.bare(p, e);
                    return e;
                }
            }
            W_ARRAY => {
                if b.get(e) == Some(&b'(') {
                    (CONSTRUCT, P_ARRAY)
                } else {
                    (STORAGE_TYPE, P_BEGIN)
                }
            }
            W_CATCH => {
                if next_byte(b, e) == b'(' {
                    self.put(p, e, CATCH);
                    let q = skip(b, e);
                    self.push(F_CATCH);
                    self.prev = P_BEGIN;
                    return q + 1;
                }
                (EXCEPTION, P_BEGIN)
            }
            W_GOTO => {
                if b.get(skip(b, e)).is_some_and(|&c| name_start(c)) && skip(b, e) > e {
                    (GOTO, P_GOTO)
                } else {
                    self.bare(p, e);
                    return e;
                }
            }
            _ => {
                // `insteadof`, `callable` and the words listed only to be told apart
                self.bare(p, e);
                return e;
            }
        };
        self.put(p, e, kind);
        self.prev = prev;
        e
    }

    /// A number at `p` (a digit, or `.` and one).
    #[inline(always)]
    fn number(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        if b[p] == b'0' && matches!(b.get(p + 1), Some(b'x' | b'X' | b'b' | b'B')) {
            let hex = matches!(b[p + 1], b'x' | b'X');
            let mut e = p + 2;
            while e < n
                && (if hex {
                    b[e].is_ascii_hexdigit()
                } else {
                    matches!(b[e], b'0' | b'1')
                } || b[e] == b'_')
            {
                e += 1;
            }
            if e < n && NAME[usize::from(b[e])] {
                // not a number's end (`\b`): the name's bytes plain
                while e < n && NAME[usize::from(b[e])] {
                    e += 1;
                }
                self.prev = P_END;
                return e;
            }
            let (prefix, kind) = if hex {
                (HEX_PREFIX, HEX)
            } else {
                (BINARY_PREFIX, BINARY)
            };
            self.put(p, p + 2, prefix);
            self.put(p + 2, e, kind);
            self.prev = P_END;
            return e;
        }
        let mut e = p;
        while e < n && (b[e].is_ascii_digit() || b[e] == b'_') {
            e += 1;
        }
        let mut kind = INT;
        if e < n && b[e] == b'.' && b.get(e + 1) != Some(&b'.') {
            kind = FLOAT;
            e += 1;
            while e < n && (b[e].is_ascii_digit() || b[e] == b'_') {
                e += 1;
            }
        }
        if e < n && matches!(b[e], b'e' | b'E') {
            let mut f = e + 1;
            if f < n && matches!(b[f], b'+' | b'-') {
                f += 1;
            }
            if f < n && b[f].is_ascii_digit() {
                kind = FLOAT;
                e = f;
                while e < n && (b[e].is_ascii_digit() || b[e] == b'_') {
                    e += 1;
                }
            }
        }
        if e < n && NAME[usize::from(b[e])] {
            while e < n && NAME[usize::from(b[e])] {
                e += 1;
            }
            self.prev = P_END;
            return e;
        }
        self.put(p, e, kind);
        self.prev = P_END;
        e
    }
}

/// A variable's name `w`'s sigil and kinds: `$this`, a superglobal, or a variable.
#[inline(always)]
fn variable_kinds(w: &[u8]) -> (u16, u16) {
    match w {
        b"this" => (THIS_SIGIL, THIS),
        b"_COOKIE" | b"_FILES" | b"_GET" | b"_POST" | b"_REQUEST" | b"argv" | b"argc" => {
            (GLOBAL_SIGIL, GLOBAL)
        }
        _ if w.starts_with(b"GLOBALS")
            || w.starts_with(b"_ENV")
            || w.starts_with(b"_SERVER")
            || w.starts_with(b"_SESSION") =>
        {
            (SAFER_SIGIL, SAFER)
        }
        _ => (SIGIL, VARIABLE),
    }
}

/// What a class's head names as its parent or an interface `w`: a builtin class, or a class.
#[inline(always)]
fn inherited(w: &[u8]) -> u16 {
    if CLASSES.get(w) != 0 {
        INHERITED_BUILTIN
    } else {
        INHERITED
    }
}

/// An escape at `q`'s `\` in a double-quoted string or a here-document: its end and kind
/// (`simple` for `\n`, `\t`, `\\`, `\$`, `\"`…), or `PLAIN` where it isn't one.
#[inline(always)]
fn escape(b: &[u8], q: usize, simple: u16) -> (usize, u16) {
    let n = b.len();
    match b.get(q + 1) {
        Some(b'0'..=b'7') => {
            let mut e = q + 2;
            while e < n && e < q + 4 && matches!(b[e], b'0'..=b'7') {
                e += 1;
            }
            (e, ESCAPE_OCTAL)
        }
        Some(b'x') if b.get(q + 2).is_some_and(u8::is_ascii_hexdigit) => {
            let mut e = q + 3;
            if e < n && b[e].is_ascii_hexdigit() {
                e += 1;
            }
            (e, ESCAPE_HEX)
        }
        Some(b'u') if b.get(q + 2) == Some(&b'{') => {
            let mut e = q + 3;
            while e < n && b[e].is_ascii_hexdigit() {
                e += 1;
            }
            if e > q + 3 && b.get(e) == Some(&b'}') {
                (e + 1, ESCAPE_UNICODE)
            } else {
                (q + 1, PLAIN)
            }
        }
        Some(b'n' | b'r' | b't' | b'\\' | b'$' | b'"') => (q + 2, simple),
        _ => (q + 1, PLAIN),
    }
}

/// Whether an inline doc comment follows `/**` at `p`: blanks, `@`, anything, a blank, `*/` and
/// blanks to the line's end.
#[inline(always)]
fn inline_doc(b: &[u8], p: usize) -> bool {
    let s = skip_space(b, p);
    if s == p || b.get(s) != Some(&b'@') {
        return false;
    }
    let mut e = b.len();
    while e > s && matches!(b[e - 1], b' ' | b'\t' | b'\r' | 0x0b | 0x0c) {
        e -= 1;
    }
    e >= s + 3 && b[e - 2..e] == *b"*/" && matches!(b[e - 3], b' ' | b'\t' | b'\r' | 0x0b | 0x0c)
}

/// Whether the word after the blanks from `e` is `w`, ASCII case aside.
#[inline(always)]
fn next_word_is(b: &[u8], e: usize, w: &[u8]) -> bool {
    let q = skip(b, e);
    q > e
        && b.len() >= q + w.len()
        && b[q..q + w.len()].eq_ignore_ascii_case(w)
        && !b.get(q + w.len()).is_some_and(|&c| NAME[usize::from(c)])
}

/// Past the blanks from `e`.
#[inline(always)]
fn skip(b: &[u8], mut e: usize) -> usize {
    while e < b.len() && matches!(b[e], b' ' | b'\t') {
        e += 1;
    }
    e
}

/// Past Oniguruma's `\s` on a line from `e`.
#[inline(always)]
fn skip_space(b: &[u8], mut e: usize) -> usize {
    while e < b.len() && matches!(b[e], b' ' | b'\t' | b'\r' | 0x0b | 0x0c) {
        e += 1;
    }
    e
}

/// The byte past the blanks from `e`, 0 at the line's end.
#[inline(always)]
fn next_byte(b: &[u8], e: usize) -> u8 {
    b.get(skip(b, e)).copied().unwrap_or(0)
}

/// Where PHP opens from `p` in HTML: bat's `<\?(?i:php|=)?`, the first `<?`; the line's end where
/// none does.
#[inline(always)]
fn php_open(b: &[u8], mut p: usize) -> usize {
    let n = b.len();
    loop {
        p = crate::prim::find_in(b, p, n, *b"<");
        if p >= n || b.get(p + 1) == Some(&b'?') {
            return p.min(n);
        }
        p += 1;
    }
}

/// Where a PHP island from `p` (past its `<?`) ends for HTML's blanking: past the first `?>`
/// outside a quoted string, or the line's end. PHP's scan finds its own end.
#[inline(always)]
fn php_close(b: &[u8], mut p: usize) -> usize {
    let n = b.len();
    while p < n {
        match b[p] {
            b'?' if b.get(p + 1) == Some(&b'>') => return p + 2,
            q @ (b'\'' | b'"') => {
                p += 1;
                while p < n && b[p] != q {
                    p += if b[p] == b'\\' { 2 } else { 1 };
                }
                p += 1;
            }
            _ => p += 1,
        }
    }
    n
}

/// The next interpolation or escape in a here-document's line from `p` (bat's `interpolation`:
/// an escape, `{$…}`, `${…}`, `$name` and a member or an index), where it starts and ends; the
/// line's end where none. Its end is a blanking's, for the markup's scanner: the scan of it
/// finds its own.
fn next_interpolation(b: &[u8], mut p: usize) -> (usize, usize) {
    let n = b.len();
    while p < n {
        p = crate::prim::find_in(b, p, n, *b"\\${");
        if p >= n {
            break;
        }
        match b[p] {
            b'\\' => {
                let (e, kind) = escape(b, p, HEREDOC_ESCAPE);
                if kind != PLAIN {
                    return (p, e);
                }
                p += 1;
            }
            b'{' if b.get(p + 1) == Some(&b'$') => {
                let e = crate::prim::find_in(b, p, n, *b"}");
                if e < n {
                    return (p, e + 1);
                }
                p += 1;
            }
            b'$' if b.get(p + 1) == Some(&b'{') => {
                let e = crate::prim::find_in(b, p, n, *b"}");
                if e < n {
                    return (p, e + 1);
                }
                p += 1;
            }
            b'$' if b.get(p + 1).is_some_and(|&c| name_start(c)) => {
                let mut e = p + 2;
                while e < n && NAME[usize::from(b[e])] {
                    e += 1;
                }
                if b.get(e..e + 2) == Some(b"->") && b.get(e + 2).is_some_and(|&c| name_start(c)) {
                    e += 3;
                    while e < n && NAME[usize::from(b[e])] {
                        e += 1;
                    }
                } else if b.get(e) == Some(&b'[') {
                    let f = crate::prim::find_in(b, e, n, *b"]");
                    if f < n {
                        e = f + 1;
                    }
                }
                return (p, e);
            }
            _ => p += 1,
        }
    }
    (n, n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Lang;
    use crate::tests::marked;

    fn of(lines: &[&str]) -> Vec<(String, String)> {
        marked(Lang::Php, lines)
            .into_iter()
            .map(|(t, s)| (t.to_owned(), s.to_owned()))
            .collect()
    }

    /// `text` lexed somewhere in `lines` as `scopes`, under `text.html.basic source.php` (`-`
    /// for HTML's own, under `text.html.basic` alone).
    fn has(lines: &[&str], text: &str, scopes: &str) {
        let want = match scopes.strip_prefix("- ") {
            Some(html) => format!("text.html.basic {html}"),
            None => format!("text.html.basic source.php {scopes}"),
        };
        let got = of(lines);
        assert!(
            got.iter().any(|(t, s)| t == text && *s == want),
            "{text:?} as {want:?} in {got:#?}"
        );
    }

    #[test]
    fn declarations_and_names() {
        let l = [
            "<?php",
            "namespace App\\Http;",
            "use Foo\\Bar as Baz, Qux;",
            "use function strlen;",
            "abstract class A extends B implements C, \\Countable {",
            "    use T;",
            "    public static ?int $n = 0;",
            "    public function __construct(private Foo $f, int ...$r): ?static {",
            "        return parent::x() ?? self::Y ?? E_ALL ?? MY_CONST ?? A::class;",
            "    }",
            "}",
            "interface I extends J {}",
            "label:",
        ];
        has(&l, "namespace", "keyword.other.namespace.php");
        has(&l, "Http", "entity.name.namespace.php");
        has(
            &l,
            "App",
            "entity.name.namespace.php support.other.namespace.php",
        );
        has(&l, "Bar", "support.class.php");
        has(&l, "Baz", "entity.name.class.php");
        has(&l, "strlen", "support.function.php");
        has(&l, "abstract ", "storage.modifier.abstract.php");
        has(&l, "A", "entity.name.class.php");
        has(&l, "B", "entity.other.inherited-class.php");
        has(
            &l,
            "Countable",
            "entity.other.inherited-class.php support.class.builtin.php",
        );
        has(&l, "T", "entity.other.inherited-class.php");
        has(&l, "?", "storage.type.nullable.php");
        has(&l, "int", "support.class.php");
        has(
            &l,
            "__construct",
            "entity.name.function.php support.function.magic.php",
        );
        has(&l, "private", "support.class.php");
        has(&l, "f", "variable.parameter.php");
        has(&l, "...", "keyword.operator.spread.php");
        has(&l, ":", "punctuation.separator.php");
        has(&l, "static", "support.class.php");
        has(&l, "parent", "variable.language.php");
        has(&l, "x", "variable.function.php");
        has(&l, "Y", "constant.other.class.php");
        has(&l, "E_ALL", "support.constant.core.php");
        has(&l, "MY_CONST", "constant.other.php");
        has(&l, "class", "constant.class.php");
        has(&l, "I", "entity.name.interface.php");
        has(&l, "extends ", "storage.modifier.extends.php");
        has(&l, "label", "entity.name.label.php");
    }

    #[test]
    fn expressions() {
        let l = [
            "<?php",
            "$x = strtolower($a->b) . Foo\\bar(1, 2.5, 0x1F, 0b1) <=> $c?->d[0];",
            "$y = (int) $z + array(1, 2) + [3] ?? new \\DateTime() instanceof self;",
            "foreach ($list as $k => &$v) { echo $this->{$v}; }",
            "try {} catch (A | \\B\\C $e) {}",
            "$f = function ($p) use (&$q): int { return $_GET['a']; };",
        ];
        has(&l, "strtolower", "support.function.string.php");
        has(&l, "->", "punctuation.accessor.arrow.php");
        has(&l, "b", "variable.other.member.php");
        has(&l, ".", "keyword.operator.string.php");
        has(&l, "Foo", "support.other.namespace.php");
        has(&l, "bar", "variable.function.php");
        has(&l, ",", "punctuation.separator.php");
        has(&l, "2.5", "constant.numeric.float.decimal.php");
        has(
            &l,
            "0x",
            "constant.numeric.integer.hexadecimal.php punctuation.definition.numeric.hexadecimal.php",
        );
        has(&l, "<=>", "keyword.operator.comparison.php");
        has(&l, "int", "storage.type.php");
        has(&l, "array", "support.function.construct.php");
        has(&l, "DateTime", "support.class.builtin.php");
        has(&l, "self", "support.class.php");
        has(&l, "as", "keyword.operator.logical.php");
        has(&l, "&", "storage.modifier.reference.php");
        has(&l, "echo", "support.function.construct.php");
        has(&l, "this", "variable.language.php");
        has(&l, "catch", "keyword.control.exception.catch.php");
        has(&l, "|", "punctuation.separator.catch.php");
        has(&l, "C", "support.class.exception.php");
        has(&l, "e", "variable.other.php");
        has(&l, "use", "keyword.other.function.use.php");
        has(&l, "q", "variable.parameter.php");
        has(&l, "_GET", "variable.other.global.php");
    }

    #[test]
    fn strings_comments_and_docs() {
        let l = [
            "<?php",
            "$a = 'it\\'s' . \"x $y->z {$w['k']} ${n} \\x41 \\u{1F600}\" . `ls $d`;",
            "/**",
            " * Doc, {@inheritdoc}.",
            " * @param int $x",
            " * @access private",
            " */",
            "// line # not",
            "# hash",
            "/* block */",
        ];
        has(
            &l,
            "\\'",
            "string.quoted.single.php constant.character.escape.php",
        );
        has(&l, "z", "variable.other.member.php");
        has(&l, "{", "punctuation.definition.expression.php");
        has(&l, "${", "punctuation.definition.variable.php");
        has(&l, "\\x41", "constant.character.escape.hex.php");
        has(
            &l,
            "\\u{1F600}",
            "constant.character.escape.unicodepoint.php",
        );
        has(
            &l,
            "`",
            "string.interpolated.php punctuation.definition.string.begin.php",
        );
        has(
            &l,
            "*",
            "comment.block.documentation.phpdoc.php punctuation.definition.comment.php",
        );
        has(
            &l,
            "@inheritdoc",
            "comment.block.documentation.phpdoc.php keyword.other.phpdoc.php",
        );
        has(
            &l,
            "@param",
            "comment.block.documentation.phpdoc.php keyword.other.phpdoc.php",
        );
        has(
            &l,
            "private",
            "comment.block.documentation.phpdoc.php storage.modifier.php",
        );
        has(&l, " line # not", "comment.line.double-slash.php");
        has(
            &l,
            "#",
            "comment.line.number-sign.php punctuation.definition.comment.php",
        );
        has(&l, " block ", "comment.block.php");
    }

    #[test]
    fn sql_regexes_and_heredocs() {
        let l = [
            "<?php",
            "$q = \"",
            "    SELECT id FROM t WHERE n = $n -- c",
            "\";",
            "preg_match('/^\\d+(?:x)$/i', $s);",
            "$h = <<<EOT",
            "  a $b",
            "  EOT;",
            "$s = <<<SQL",
            "  DELETE FROM t",
            "  SQL;",
            "$j = <<<'JSON'",
            "{\"a\": true}",
            "JSON;",
            "$x = <<<HTML",
            "<b class=\"c\">$v</b>",
            "HTML;",
        ];
        has(
            &l,
            "SELECT",
            "string.quoted.double.php source.sql keyword.other.DML.sql",
        );
        has(&l, "n", "variable.other.php");
        has(
            &l,
            "--",
            "string.quoted.double.php source.sql comment.line.double-dash.sql punctuation.definition.comment.sql",
        );
        has(
            &l,
            "/",
            "string.quoted.single.php punctuation.definition.string.regex-delimiter.begin.php",
        );
        has(
            &l,
            "\\d",
            "string.quoted.single.php source.regexp.php constant.character.character-class.regexp",
        );
        has(
            &l,
            "?:",
            "string.quoted.single.php source.regexp.php constant.other.assertion.regexp",
        );
        has(
            &l,
            "EOT",
            "string.unquoted.heredoc.php punctuation.definition.string.php keyword.operator.heredoc.php",
        );
        has(&l, "  a ", "string.unquoted.heredoc.php");
        has(
            &l,
            "EOT",
            "punctuation.section.embedded.end.php keyword.operator.heredoc.php",
        );
        has(&l, "DELETE", "source.sql keyword.other.DML.sql");
        has(&l, "true", "source.json constant.language.json");
        has(&l, "b", "- entity.name.tag.inline.any.html");
        has(&l, "v", "variable.other.php");
    }

    #[test]
    fn attributes() {
        let l = [
            "<?php",
            "#[Route('/x', name: 'y')]",
            "function f(#[\\SensitiveParameter] $p, #[A] int $q) {}",
        ];
        has(
            &l,
            "#[",
            "meta.annotation.php punctuation.definition.annotation.begin.php",
        );
        has(&l, "Route", "meta.annotation.php variable.annotation.php");
        has(
            &l,
            "\\SensitiveParameter",
            "meta.annotation.php variable.annotation.php",
        );
        has(
            &l,
            "]",
            "meta.annotation.php punctuation.definition.annotation.end.php",
        );
        has(&l, "p", "variable.parameter.php");
        has(&l, "int", "storage.type.php");
        has(&l, "q", "variable.parameter.php");
    }

    #[test]
    fn templates_and_fences() {
        let l = [
            "<!DOCTYPE html>",
            "<div class=\"<?= $c ?>\">",
            "<?php if ($x) { ?>",
            "  <script>var a = '<?php echo $b ?>';</script>",
            "<?php } ?>",
        ];
        has(&l, "div", "- entity.name.tag.block.any.html");
        has(&l, "<?=", "punctuation.section.embedded.begin.php");
        has(&l, "?>", "punctuation.section.embedded.end.php");
        has(&l, "if", "keyword.control.php");
        has(&l, "var", "- source.js storage.type.js");
        has(&l, "b", "variable.other.php");
        // a fence's snippet without its tags is PHP's code; a shebang waits for `<?php`
        has(&["$x = 1;"], "x", "variable.other.php");
        has(&["", "  echo 1;"], "echo", "support.function.construct.php");
        has(
            &["#!/usr/bin/env php", "<?php echo 1;"],
            "echo",
            "support.function.construct.php",
        );
    }

    fn lines(text: &str) -> Vec<(Vec<crate::Span>, State)> {
        let mut state = State::default();
        let mut out = Vec::new();
        for l in text.lines() {
            let mut spans = Vec::new();
            line(l.as_bytes(), &mut state, &mut Out::new(&mut spans));
            out.push((spans, state));
        }
        out
    }

    #[test]
    fn a_block_lexes_as_its_lines() {
        // every field a line hands the next: frames past the levels, a string's head and its SQL,
        // a here-document's label and embed, a doc comment, an attribute; a template's HTML
        // with PHP's frames past the ones it keeps beside it, and HTML's levels below an island
        // past the ones a line in PHP keeps
        let deep = "( ".repeat(40) + &") ".repeat(38);
        let blocks = "{ ".repeat(9);
        let text = format!(
            "<?php\n$a = {deep}\n));\n$q = \"\n  SELECT 1 FROM t -- c\n  WHERE x = $y\";\n\
             $h = <<<HTML\n<div><span>\nHTML;\n$j = <<<JSON\n{{\"a\": /* x\n */ 1}}\nJSON;\n\
             /**\n * @param x\n */\n#[A(\n  1)]\nif (1) {blocks}?>\n<div><p><b><i><u><s><a href=\"x\n\
             <?php echo 1;\n?>\">t</a>\n<?php }}}}}}}}}} ?>\n<script>var s = 1;\n</script>\nend\n"
        );
        let one = lines(&text);
        let mut state = State::default();
        let mut all = Vec::new();
        let mut v = Vec::new();
        block(&mut state, &mut v, &mut |c| {
            for l in text.lines() {
                all.push(c.next(l.as_bytes()).to_vec());
            }
            None
        });
        assert_eq!(all, one.iter().map(|(s, _)| s.clone()).collect::<Vec<_>>());
        assert_eq!(state, one.last().unwrap().1);
    }

    #[test]
    fn long_lines_lex_in_linear_time() {
        for unit in [
            "$a->b(",
            "f(",
            "[",
            "'SELECT x', ",
            "\"SELECT $a {$b} ",
            "<div class=\"<?= $x ?>\">",
            "#[A(",
            "'/a+/' . ",
            "/** @param x ",
            "function f(int $a = [",
            "\\A\\B::c ",
        ] {
            let l = format!("<?php {}", unit.repeat(20_000));
            let t = std::time::Instant::now();
            lines(&l);
            assert!(t.elapsed().as_millis() < 300, "{unit:?}: {:?}", t.elapsed());
        }
    }
}
