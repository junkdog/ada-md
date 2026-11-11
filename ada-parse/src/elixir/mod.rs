//! Elixir on a scanner of ada's own (X-138), from a floor probe (the same files built with
//! `ADA_PARSE_X138_SOL`): one loop over a line's bytes, a byte's action one load from a table
//! ([`ACT`]), a name read once and looked up once in one table of every word ([`WORDS`]: its leaf
//! and, for the words whose meaning hangs on where they stand, an action), and frames only for
//! what nests ([`Scan::frames`], four bits a level): parentheses, brackets, a tuple's or a map's
//! braces, a binary's `<<…>>`, a string's `#{…}`, a function's head. A string, a charlist, a
//! sigil, a heredoc or a quoted atom continued past a line's end is a mode ([`Scan::mode`]); the
//! strings whose `#{` is open are a byte each ([`Scan::strs`]: what the string is and its
//! closing delimiter). Kinds are bat's leaves; its grammar is mostly a token's own, so the
//! context it reads is a few heads (a function's from `def` to its `do`, a module's and a
//! protocol's names), the documentation attributes, a binary's segments and an `fn`'s `when`.

mod embed;
mod heex;
#[cfg(test)]
mod tests;
mod text;

use crate::emit::{Rules, Run, Runs, Sink, Spans};
use crate::prim::{WORD, char_len, is_word, is_word_starting, word_end};
use crate::regex;
use crate::sql;
use crate::words::{Words, text_len};
use crate::{Out, State};

/// The floor probe (X-138's `sol`): every word its table leaf, `name:` a key, atoms, attributes,
/// captures, numbers, characters, strings, charlists, sigils and heredocs with their escapes and
/// `#{…}`, comments, brackets and binaries; no head, documentation, quoted key, `fn`'s `when`,
/// guard, SQL or regex read.
const SOL: bool = option_env!("ADA_PARSE_X138_SOL").is_some();

/// The leaves below the strings': a token's scopes, bat's.
static LEAVES: [&str; STR_BASE as usize] = [
    "",
    "comment.line.number-sign.elixir",
    "comment.line.number-sign.elixir punctuation.definition.comment.elixir",
    "comment.line.section.elixir",
    "comment.line.section.elixir punctuation.definition.comment.elixir",
    "keyword.control.elixir",
    "keyword.control.module.elixir",
    "keyword.control.protocol.elixir",
    "keyword.guard.elixir",
    "keyword.operator.elixir",
    "entity.name.class.elixir",
    "entity.name.protocol.elixir",
    "entity.name.function.public.elixir",
    "entity.name.function.private.elixir",
    "constant.language.elixir",
    "variable.language.elixir",
    "variable.other.module.elixir",
    "variable.other.module.elixir keyword.operator.definition.constant.elixir",
    "constant.other.capture.elixir",
    "constant.other.capture.elixir keyword.operator.capture.elixir",
    "variable.other.capture.elixir",
    "variable.other.capture.elixir keyword.operator.capture.elixir",
    "variable.other.pin.elixir",
    "variable.other.pin.elixir keyword.operator.pin.elixir",
    "constant.other.symbol.elixir",
    "constant.other.symbol.elixir punctuation.definition.constant.elixir",
    "constant.other.keywords.elixir",
    "constant.other.keywords.elixir punctuation.definition.constant.elixir",
    "constant.numeric.elixir",
    "keyword.operator.arrow.elixir",
    "keyword.operator.binary-concatenation.elixir",
    "keyword.operator.pipe.elixir",
    "keyword.operator.list-manipulation.elixir",
    "keyword.operator.range.elixir",
    "keyword.operator.map-pair.elixir",
    "keyword.operator.bitwise.elixir",
    "keyword.operator.other.elixir",
    "keyword.operator.comparison.elixir",
    "keyword.operator.logical.elixir",
    "keyword.operator.arithmetic.elixir",
    "keyword.operator.assignment.elixir",
    "punctuation.separator.statement.elixir",
    "punctuation.separator.object.elixir",
    "punctuation.separator.method.elixir",
    "punctuation.section.array.elixir",
    "punctuation.section.function.elixir",
    "punctuation.definition.parameters.elixir",
    "punctuation.section.sequence.begin.elixir",
    "punctuation.section.sequence.end.elixir",
    "punctuation.separator.sequence.elixir",
    "invalid.illegal.stray-closing-brace.elixir",
    "binary.elixir punctuation.definition.binary.begin.elixir",
    "binary.elixir punctuation.definition.binary.end.elixir",
    "binary.elixir punctuation.binary.elixir",
    "binary.elixir keyword.type.binary.elixir",
    "binary.elixir keyword.type.binary.elixir punctuation.binary.elixir",
    "binary.elixir keyword.type.binary.elixir constant.numeric.elixir",
    "binary.elixir keyword.type.binary.elixir variable.numeric.elixir",
    "binary.elixir constant.numeric.elixir",
    "comment.documentation.false",
    "storage.type.string.elixir",
    "storage.type.string.elixir invalid.illegal.sigil-string.elixir",
    "storage.type.string.elixir invalid.illegal.string-delimiter.elixir",
    "string.quoted.modifiers.elixir storage.type.string.elixir",
    "invalid.illegal.non-ascii-modifier.elixir",
    "punctuation.section.interpolation.end.elixir",
    "punctuation.section.embedded.begin.elixir",
    "punctuation.section.embedded.end.elixir",
];

const COMMENT: u16 = 1;
const COMMENT_MARK: u16 = 2;
const SECTION: u16 = 3;
const SECTION_MARK: u16 = 4;
const KEYWORD: u16 = 5;
const MODULE_KW: u16 = 6;
const PROTOCOL_KW: u16 = 7;
const GUARD: u16 = 8;
const OP_WORD: u16 = 9;
const CLASS: u16 = 10;
const PROTOCOL_NAME: u16 = 11;
const FUN_PUBLIC: u16 = 12;
const FUN_PRIVATE: u16 = 13;
const LANGUAGE: u16 = 14;
const LANGUAGE_VAR: u16 = 15;
const ATTR: u16 = 16;
const ATTR_MARK: u16 = 17;
const CAPTURE: u16 = 18;
const CAPTURE_MARK: u16 = 19;
const CAPTURE_VAR: u16 = 20;
const CAPTURE_VAR_MARK: u16 = 21;
const PIN: u16 = 22;
const PIN_MARK: u16 = 23;
const SYMBOL: u16 = 24;
const SYMBOL_MARK: u16 = 25;
const KEYS: u16 = 26;
const KEYS_MARK: u16 = 27;
const NUMBER: u16 = 28;
const OP_ARROW: u16 = 29;
const OP_CONCAT: u16 = 30;
const OP_PIPE: u16 = 31;
const OP_LIST: u16 = 32;
const OP_RANGE: u16 = 33;
const OP_MAP: u16 = 34;
const OP_BITWISE: u16 = 35;
const OP_OTHER: u16 = 36;
const OP_COMPARE: u16 = 37;
const OP_LOGICAL: u16 = 38;
const OP_ARITH: u16 = 39;
const OP_ASSIGN: u16 = 40;
const SEMI: u16 = 41;
const COMMA: u16 = 42;
const DOT: u16 = 43;
const ARRAY: u16 = 44;
const FUNCTION: u16 = 45;
const PARAMS: u16 = 46;
const SEQ_BEGIN: u16 = 47;
const SEQ_END: u16 = 48;
const SEQ_SEP: u16 = 49;
const STRAY: u16 = 50;
const BIN_BEGIN: u16 = 51;
const BIN_END: u16 = 52;
const BIN_SEP: u16 = 53;
const BIN_TYPE: u16 = 54;
const BIN_TYPE_SEP: u16 = 55;
const BIN_TYPE_NUM: u16 = 56;
const BIN_TYPE_VAR: u16 = 57;
const BIN_NUM: u16 = 58;
const DOC_FALSE: u16 = 59;
const SIGIL: u16 = 60;
const SIGIL_BAD: u16 = 61;
const SIGIL_BAD_DELIM: u16 = 62;
const MODIFIERS: u16 = 63;
const BAD_MODIFIERS: u16 = 64;
const INTERP_END: u16 = 65;
/// A HEEx island's `{` or `<%=`, and its `}` or `%>`.
const EMBED_BEGIN: u16 = 66;
const EMBED_END: u16 = 67;
/// The strings' leaves from here: [`FL_N`] flavors of [`STR_LEAVES`] each.
const STR_BASE: u16 = 68;

// A string's flavor: the scope its text takes, bat's.
const FL_DQ: u16 = 0;
const FL_SQ: u16 = 1;
const FL_TDQ: u16 = 2;
const FL_TSQ: u16 = 3;
/// A lowercase sigil's between brackets, and every `~r`'s.
const FL_INTERP: u16 = 4;
/// A lowercase sigil's between `/`, `|`, `"` or `'`.
const FL_LOWER: u16 = 5;
const FL_UPPER: u16 = 6;
/// A `~R` sigil's.
const FL_REGEX: u16 = 7;
const FL_ATOM_DQ: u16 = 8;
const FL_ATOM_SQ: u16 = 9;
/// A quoted keyword's, `"name":`.
const FL_KEY: u16 = 10;
const FL_DOC: u16 = 11;
const FL_DOC_STR: u16 = 12;
const FL_N: u16 = 13;

/// A flavor's string scope, its begin and end marks' scopes.
const FLAVORS: [(&str, &str, &str); FL_N as usize] = [
    (
        "string.quoted.double.elixir",
        "punctuation.definition.string.begin.elixir",
        "punctuation.definition.string.end.elixir",
    ),
    (
        "string.quoted.single.elixir",
        "punctuation.definition.string.begin.elixir",
        "punctuation.definition.string.end.elixir",
    ),
    (
        "string.quoted.triple.double.elixir",
        "punctuation.definition.string.begin.elixir",
        "punctuation.definition.string.end.elixir",
    ),
    (
        "string.quoted.triple.single.elixir",
        "punctuation.definition.string.begin.elixir",
        "punctuation.definition.string.end.elixir",
    ),
    (
        "string.interpolated.elixir",
        "punctuation.definition.string.begin.elixir",
        "punctuation.definition.string.end.elixir",
    ),
    (
        "string.quoted.other.literal.lower.elixir",
        "punctuation.definition.string.begin.elixir",
        "punctuation.definition.string.end.elixir",
    ),
    (
        "string.quoted.other.literal.upper.elixir",
        "punctuation.definition.string.begin.elixir",
        "punctuation.definition.string.end.elixir",
    ),
    (
        "string.quoted.other.literal.regex.elixir",
        "punctuation.definition.string.begin.elixir",
        "punctuation.definition.string.end.elixir",
    ),
    (
        "constant.other.symbol.double-quoted.elixir",
        "punctuation.definition.constant.elixir",
        "punctuation.definition.constant.elixir",
    ),
    (
        "constant.other.symbol.single-quoted.elixir",
        "punctuation.definition.constant.elixir",
        "punctuation.definition.constant.elixir",
    ),
    (
        "constant.other.symbol.double-quoted.elixir",
        "punctuation.definition.constant.begin.regexp.elixir",
        "punctuation.definition.constant.end.regexp.elixir",
    ),
    ("comment.documentation.heredoc", "", ""),
    ("comment.documentation.string", "", ""),
];

/// A string's leaves, after its flavor's base ([`flavor`]).
const TEXT: u16 = 0;
const BEGIN: u16 = 1;
const END: u16 = 2;
const ESC_CHAR: u16 = 3;
const ESC_HEX: u16 = 4;
const ESC_HEX_BAD: u16 = 5;
const ESC_UNI: u16 = 6;
const ESC_UNI_BAD: u16 = 7;
/// `#{`.
const INTERP: u16 = 8;
/// The text after a heredoc's opening delimiter on its line.
const BAD_OPEN: u16 = 9;
/// A heredoc regex's comment, `#` after a blank to the line's end (an extended regex's).
const RE_COMMENT: u16 = 10;
const STR_LEAVES: u16 = 11;

/// The kinds of SQL in a string, its flavor's from `SQL_BASE + flavor * 128`.
const SQL_BASE: u16 = 256;
/// The kinds of a regex in a string, its flavor's from `RE_BASE + flavor * 128`.
const RE_BASE: u16 = SQL_BASE + FL_N * 128;
const _: () = assert!(STR_BASE + FL_N * STR_LEAVES <= SQL_BASE);
const _: () = assert!(regex::KINDS <= 128 && sql::KINDS <= 128);
/// HTML's kinds in a `~H` template ([`heex`]): its own kind (its CSS's and JavaScript's flags
/// kept) with this bit.
const HTML_KIND: u16 = 0x8000;
const _: () = assert!(RE_BASE + FL_N * 128 <= 0x1000);

/// The scopes under `source.elixir` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    if kind & HTML_KIND != 0 {
        return crate::html::scopes(kind & !HTML_KIND);
    }
    if kind >= SQL_BASE {
        let (from, re) = if kind >= RE_BASE {
            (RE_BASE, true)
        } else {
            (SQL_BASE, false)
        };
        let f = (kind - from) / 128;
        let k = (kind - from) % 128;
        if f >= FL_N || (!re && usize::from(k) >= sql::KINDS) {
            return Vec::new();
        }
        let mut out = scopes(STR_BASE + f * STR_LEAVES);
        out.extend(if re { regex::scopes(k) } else { sql::scopes(k) });
        return out;
    }
    if kind >= STR_BASE {
        let i = kind - STR_BASE;
        let (f, off) = (i / STR_LEAVES, i % STR_LEAVES);
        let Some(&(s, b, e)) = FLAVORS.get(usize::from(f)) else {
            return Vec::new();
        };
        let leaf = match off {
            TEXT => "",
            BEGIN => b,
            END => e,
            ESC_CHAR => "constant.character.escape.char.elixir",
            ESC_HEX => "constant.character.escape.hex.elixir",
            ESC_HEX_BAD => "constant.character.escape.hex.elixir invalid.illegal.escape.hex.elixir",
            ESC_UNI => "constant.character.escape.unicode.elixir",
            ESC_UNI_BAD => "invalid.illegal.escape.unicode.elixir",
            INTERP => "punctuation.section.interpolation.begin.elixir",
            BAD_OPEN => "invalid.illegal.opening-heredoc.elixir",
            _ => "comment.line.number-sign.regexp.elixir",
        };
        let mut v = vec![s];
        v.extend(leaf.split(' ').filter(|s| !s.is_empty()));
        return v;
    }
    LEAVES.get(usize::from(kind)).map_or_else(Vec::new, |l| {
        l.split(' ').filter(|s| !s.is_empty()).collect()
    })
}

// A string on [`Scan::strs`], a byte: its closing delimiter in the low three bits, a heredoc's
// bit, and what it is in the high nibble.
/// The closing delimiters, by the low three bits.
const CLOSER: [u8; 8] = *b"\"'/|)]}>";
const D_DQ: u8 = 0;
const D_SQ: u8 = 1;
const D_SLASH: u8 = 2;
const D_PIPE: u8 = 3;
/// `"""` or `'''` (the delimiter [`D_DQ`] or [`D_SQ`]): ends at a line whose first non-blank
/// is the delimiter.
const HEREDOC: u8 = 8;
/// `"…"`, `'…'`, `"""…"""`, `'''…'''`: escapes and `#{…}`.
const K_STR: u8 = 1 << 4;
/// A lowercase sigil's (`~s`, `~c`, `~w`…): escapes and `#{…}`.
const K_LOWER: u8 = 2 << 4;
/// An uppercase sigil's (`~S`, `~D`, `~HTML`…): raw, a `\` taking the character after it.
const K_UPPER: u8 = 3 << 4;
/// `~r`: a regex with `#{…}`.
const K_REGEX: u8 = 4 << 4;
/// `~R`: a raw regex.
const K_REGEX_RAW: u8 = 5 << 4;
/// `:"…"`, `:'…'`.
const K_ATOM: u8 = 6 << 4;
/// `"…":`, `'…':` on one line, a keyword's.
const K_KEY: u8 = 7 << 4;
/// `@doc """…"""`, `@doc "…"`: escapes and `#{…}`.
const K_DOC: u8 = 8 << 4;
/// `@doc ~S"""…"""`: raw.
const K_DOC_RAW: u8 = 9 << 4;
/// A string of SQL ([`crate::sql`], its context in [`Scan::ctx`]).
const K_SQL: u8 = 10 << 4;
/// A `"…"` string of a regex (`Regex.compile("…")`).
const K_REGEX_STR: u8 = 11 << 4;
/// `~H`: a HEEx template, HTML with Elixir's islands ([`heex`]).
const K_HEEX: u8 = 12 << 4;
const KIND: u8 = 0xf0;

/// Whether a string of `sk` has escapes and `#{…}`.
#[inline(always)]
fn interpolates(sk: u8) -> bool {
    !matches!(sk & KIND, K_UPPER | K_REGEX_RAW | K_DOC_RAW | K_HEEX)
}

/// Whether a string of `sk` is a sigil's: modifiers after its end.
#[inline(always)]
fn is_sigil(sk: u8) -> bool {
    matches!(
        sk & KIND,
        K_LOWER | K_UPPER | K_REGEX | K_REGEX_RAW | K_HEEX
    )
}

/// A string kind's flavor.
const fn flavor_of(sk: u8) -> u16 {
    let d = sk & 7;
    let heredoc = sk & HEREDOC != 0;
    match sk & KIND {
        K_DOC | K_DOC_RAW if heredoc => FL_DOC,
        K_DOC | K_DOC_RAW => FL_DOC_STR,
        _ if heredoc => {
            if d == D_SQ {
                FL_TSQ
            } else {
                FL_TDQ
            }
        }
        K_LOWER => {
            if d <= D_PIPE {
                FL_LOWER
            } else {
                FL_INTERP
            }
        }
        K_UPPER | K_HEEX => FL_UPPER,
        K_REGEX => FL_INTERP,
        K_REGEX_RAW => FL_REGEX,
        K_ATOM => {
            if d == D_SQ {
                FL_ATOM_SQ
            } else {
                FL_ATOM_DQ
            }
        }
        K_KEY => FL_KEY,
        _ => {
            if d == D_SQ {
                FL_SQ
            } else {
                FL_DQ
            }
        }
    }
}

/// A string kind's first leaf, by its byte.
static FLAVOR: [u16; 256] = {
    let mut t = [0u16; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = STR_BASE + flavor_of(i as u8) * STR_LEAVES;
        i += 1;
    }
    t
};

/// A string kind's first leaf.
#[inline(always)]
fn flavor(sk: u8) -> u16 {
    FLAVOR[usize::from(sk)]
}

// A byte's action ([`ACT`]).
const A_OTHER: u8 = 0;
const A_SPACE: u8 = 1;
const A_NAME: u8 = 2;
const A_DIGIT: u8 = 3;
const A_DQUOTE: u8 = 4;
const A_SQUOTE: u8 = 5;
const A_COLON: u8 = 6;
const A_AT: u8 = 7;
const A_HASH: u8 = 8;
const A_TILDE: u8 = 9;
const A_QUEST: u8 = 10;
const A_AMP: u8 = 11;
const A_CARET: u8 = 12;
/// `=-+*/<>|!\`: an operator.
const A_OP: u8 = 13;
const A_DOT: u8 = 14;
const A_COMMA: u8 = 15;
const A_SEMI: u8 = 16;
const A_LPAREN: u8 = 17;
const A_RPAREN: u8 = 18;
const A_LBRACKET: u8 = 19;
const A_RBRACKET: u8 = 20;
const A_LBRACE: u8 = 21;
const A_RBRACE: u8 = 22;
/// A byte past ASCII: a word's character, or another.
const A_WIDE: u8 = 23;
/// `%`: an EEx tag's `%>` in a template's island, else plain.
const A_PERCENT: u8 = 24;

static ACT: [u8; 256] = {
    let mut t = [A_WIDE; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = match b {
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => A_SPACE,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => A_NAME,
            b'0'..=b'9' => A_DIGIT,
            b'"' => A_DQUOTE,
            b'\'' => A_SQUOTE,
            b':' => A_COLON,
            b'@' => A_AT,
            b'#' => A_HASH,
            b'~' => A_TILDE,
            b'?' => A_QUEST,
            b'&' => A_AMP,
            b'^' => A_CARET,
            b'=' | b'-' | b'+' | b'*' | b'/' | b'<' | b'>' | b'|' | b'!' | b'\\' => A_OP,
            b'.' => A_DOT,
            b',' => A_COMMA,
            b';' => A_SEMI,
            b'(' => A_LPAREN,
            b')' => A_RPAREN,
            b'[' => A_LBRACKET,
            b']' => A_RBRACKET,
            b'{' => A_LBRACE,
            b'}' => A_RBRACE,
            b'%' => A_PERCENT,
            _ => A_OTHER,
        };
        c += 1;
    }
    t
};

// What a word is ([`WORDS`], [`INFO`]): its leaf in the low byte, a class in the next bits, an
// action above.
const W_LEAF: u16 = 0xff;
/// A keyword: bat's `(?<!\.)\b(…)\b(?![?!:])`.
const W_KW: u16 = 1 << 8;
/// `and`, `or`, `not`, `xor`: bat's logical operators, anywhere.
const W_LOGICAL: u16 = 2 << 8;
/// `when`, `in`: bat's `(?<!\.)\b(…)\b`.
const W_OPWORD: u16 = 3 << 8;
/// `nil`, `true`, `false`, `__MODULE__`…: bat's `\b(…)\b(?![?!])`.
const W_CONST: u16 = 4 << 8;
/// A guard's word, a function's head colors it.
const W_GUARD: u16 = 5 << 8;
/// A call's name that makes a string after it SQL or a regex ([`embed::sql_call`],
/// [`embed::regex_call`]): `fragment`, `execute`, `query`, `compile`.
const W_CALL: u16 = 6 << 8;
// The actions.
const W_ACTION: u16 = 0xf << 11;
/// `def`, `defmacro`: a public function's head at a line's start.
const W_DEF: u16 = 1 << 11;
/// `defp`, `defmacrop`: a private one's.
const W_DEFP: u16 = 2 << 11;
/// `defmodule`: a module's head at a line's start.
const W_MODULE: u16 = 3 << 11;
/// `defprotocol`, `defimpl`: a protocol's.
const W_PROTOCOL: u16 = 4 << 11;
/// `fn`: a `when` before its `->` is a keyword.
const W_FN: u16 = 5 << 11;
/// `do`: a function's head ends.
const W_DO: u16 = 6 << 11;
/// `when`.
const W_WHEN: u16 = 7 << 11;
/// `unquote`: a function's name after `def`.
const W_UNQUOTE: u16 = 8 << 11;
/// A SQL call's name.
const W_SQL: u16 = 9 << 11;
/// A regex call's name.
const W_RE: u16 = 10 << 11;

/// The words, each with what it is.
const WORD_LIST: &[(&[u8], u16)] = &[
    (b"do", W_KW | W_DO | KEYWORD),
    (b"end", W_KW | KEYWORD),
    (b"case", W_KW | KEYWORD),
    (b"for", W_KW | KEYWORD),
    (b"if", W_KW | KEYWORD),
    (b"cond", W_KW | KEYWORD),
    (b"unless", W_KW | KEYWORD),
    (b"try", W_KW | KEYWORD),
    (b"receive", W_KW | KEYWORD),
    (b"fn", W_KW | W_FN | KEYWORD),
    (b"defmodule", W_KW | W_MODULE | KEYWORD),
    (b"def", W_KW | W_DEF | KEYWORD),
    (b"defp", W_KW | W_DEFP | KEYWORD),
    (b"defprotocol", W_KW | W_PROTOCOL | KEYWORD),
    (b"defimpl", W_KW | W_PROTOCOL | KEYWORD),
    (b"defrecord", W_KW | KEYWORD),
    (b"defstruct", W_KW | KEYWORD),
    (b"defmacro", W_KW | W_DEF | KEYWORD),
    (b"defmacrop", W_KW | W_DEFP | KEYWORD),
    (b"defdelegate", W_KW | KEYWORD),
    (b"defexception", W_KW | KEYWORD),
    (b"defoverridable", W_KW | KEYWORD),
    (b"defguard", W_KW | KEYWORD),
    (b"defguardp", W_KW | KEYWORD),
    (b"exit", W_KW | KEYWORD),
    (b"after", W_KW | KEYWORD),
    (b"rescue", W_KW | KEYWORD),
    (b"catch", W_KW | KEYWORD),
    (b"else", W_KW | KEYWORD),
    (b"raise", W_KW | KEYWORD),
    (b"reraise", W_KW | KEYWORD),
    (b"throw", W_KW | KEYWORD),
    (b"import", W_KW | KEYWORD),
    (b"require", W_KW | KEYWORD),
    (b"alias", W_KW | KEYWORD),
    (b"use", W_KW | KEYWORD),
    (b"quote", W_KW | KEYWORD),
    (b"unquote", W_KW | W_UNQUOTE | KEYWORD),
    (b"super", W_KW | KEYWORD),
    (b"with", W_KW | KEYWORD),
    (b"and", W_LOGICAL | OP_LOGICAL),
    (b"or", W_LOGICAL | OP_LOGICAL),
    (b"not", W_LOGICAL | OP_LOGICAL),
    (b"xor", W_LOGICAL | OP_LOGICAL),
    (b"when", W_OPWORD | W_WHEN | OP_WORD),
    (b"in", W_OPWORD | OP_WORD),
    (b"nil", W_CONST | LANGUAGE),
    (b"true", W_CONST | LANGUAGE),
    (b"false", W_CONST | LANGUAGE),
    (b"__CALLER__", W_CONST | LANGUAGE_VAR),
    (b"__ENV__", W_CONST | LANGUAGE_VAR),
    (b"__MODULE__", W_CONST | LANGUAGE_VAR),
    (b"__DIR__", W_CONST | LANGUAGE_VAR),
    // bat's guards, colored in a function's head
    (b"is_atom", W_GUARD),
    (b"is_binary", W_GUARD),
    (b"is_bitstring", W_GUARD),
    (b"is_boolean", W_GUARD),
    (b"is_float", W_GUARD),
    (b"is_function", W_GUARD),
    (b"is_integer", W_GUARD),
    (b"is_list", W_GUARD),
    (b"is_map", W_GUARD),
    (b"is_nil", W_GUARD),
    (b"is_number", W_GUARD),
    (b"is_pid", W_GUARD),
    (b"is_port", W_GUARD),
    (b"is_record", W_GUARD),
    (b"is_reference", W_GUARD),
    (b"is_tuple", W_GUARD),
    (b"is_exception", W_GUARD),
    (b"abs", W_GUARD),
    (b"bit_size", W_GUARD),
    (b"byte_size", W_GUARD),
    (b"div", W_GUARD),
    (b"elem", W_GUARD),
    (b"hd", W_GUARD),
    (b"length", W_GUARD),
    (b"map_size", W_GUARD),
    (b"node", W_GUARD),
    (b"rem", W_GUARD),
    (b"round", W_GUARD),
    (b"tl", W_GUARD),
    (b"trunc", W_GUARD),
    (b"tuple_size", W_GUARD),
    // the embeds' calls
    (b"fragment", W_CALL | W_SQL),
    (b"execute", W_CALL | W_SQL),
    (b"query", W_CALL | W_SQL),
    (b"compile", W_CALL | W_RE),
];

const N: usize = WORD_LIST.len();
static WORD_TEXT: [&[u8]; N] = {
    let mut t: [&[u8]; N] = [b""; N];
    let mut i = 0;
    while i < N {
        t[i] = WORD_LIST[i].0;
        i += 1;
    }
    t
};
static WORDS: Words<256, N, { text_len(&WORD_TEXT) }> = Words::new(&WORD_TEXT);
/// A word's leaf, class and action by its id; 0 for none (a name).
static INFO: [u16; N + 1] = {
    let mut t = [0; N + 1];
    let mut i = 0;
    while i < N {
        t[i + 1] = WORD_LIST[i].1;
        i += 1;
    }
    t
};

// The frames, four bits a level in [`Scan::frames`]; 0 is none (the top level, or a level lost
// past [`LEVELS`]).
/// `(` … `)`.
const F_PAREN: u8 = 1;
/// `[` … `]`.
const F_BRACKET: u8 = 2;
/// `{` … `}`, a tuple's or a map's: its `,`s a sequence's.
const F_TUPLE: u8 = 3;
/// A string's `#{` … `}`: code to its `}`, its string after ([`Scan::strs`]).
const F_INTERP: u8 = 4;
/// `<<` … `>>`: its segments' numbers, strings, `,`s and types colored, the rest plain.
const F_BINARY: u8 = 5;
/// A segment's type, after `::`, to its `,` or the binary's `>>`.
const F_BINTYPE: u8 = 6;
/// A function's head, from `def name` to its `do` or `do:`: its guards colored.
const F_HEAD: u8 = 7;
/// A call's parentheses whose strings are SQL ([`embed::sql_call`]).
const F_SQLCALL: u8 = 8;
/// A call's parentheses whose first string is a regex ([`embed::regex_call`]).
const F_RECALL: u8 = 9;
/// A template's `{` … `}` island ([`heex`]): code to its `}`, the template after.
const F_ISLAND: u8 = 10;
/// A template's `<%=` … `%>` island.
const F_EEX: u8 = 11;
/// The levels [`Scan::frames`] holds.
const LEVELS: u8 = 28;
const FRAME_BITS: u32 = 4;
const FRAME_MASK: u128 = (1 << (FRAME_BITS * LEVELS as u32)) - 1;

// What a line starts in ([`Scan::mode`]): code, or a string's text, its kind the innermost on
// [`Scan::strs`].
const M_CODE: u8 = 0;
const M_STRING: u8 = 1;

/// [`State::arg`]'s bit for a function's head whose line ended where it goes on (a `,`, an
/// operator, `when`).
const ARG_CONT: u16 = 1 << 2;

/// No `fn` waiting for its `->` on the line ([`Scan::fn_depth`]).
const NO_FN: u16 = u16::MAX;

/// Lex `b`, a line of Elixir without its `\n`, from `state`.
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
    if s.heex.is_some() {
        s.spare_heex();
    }
    s.k
}

/// A block of Elixir on one scan opened from `state` ([`crate::Carry`]).
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
    /// What each open frame is ([`F_PAREN`]…), four bits a level, the innermost lowest; past
    /// [`LEVELS`] the outer ones are lost.
    frames: u128,
    /// How many frames are open, up to 255.
    depth: u8,
    /// What the line starts in ([`M_CODE`], [`M_STRING`]).
    mode: u8,
    /// The strings whose `#{` is open, a byte each ([`K_STR`]…), the innermost lowest; a
    /// string's in the low byte while [`M_STRING`] holds. Past eight the outer ones are lost.
    strs: u64,
    /// A SQL string's context, or a regex string's state, across a `#{…}` or a line's end.
    ctx: u8,
    /// [`State::arg`]'s carried bits ([`ARG_CONT`]).
    carried: u16,
    // The line's own, reset at its start:
    /// Where the line's first non-blank is, if it starts in code; `usize::MAX` if not.
    lead: usize,
    /// The depth an `fn` stood at whose `->` is on the line ([`NO_FN`] for none).
    fn_depth: u16,
    /// The last SQL or regex call's name on the line, `start..end` (`end` `usize::MAX` for
    /// none).
    callw: (usize, usize),
    /// What SQL's scan keeps between the host's turns on a line.
    sql: sql::Embedded,
    /// A `~H` template's HTML ([`heex`]), made where a line first needs it.
    heex: Option<Box<heex::Heex>>,
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
            mode: M_CODE,
            strs: 0,
            ctx: 0,
            carried: 0,
            lead: usize::MAX,
            fn_depth: NO_FN,
            callw: (0, usize::MAX),
            sql: sql::Embedded {
                is_end: usize::MAX,
                at: 0,
            },
            heex: None,
            k,
            out,
        }
    }

    /// The scan's fields from `state`: the frames in its levels, the strings in the first eight
    /// withs and the SQL or regex context in the ninth, the mode and the carried bits in
    /// [`State::arg`].
    #[inline(always)]
    fn load(&mut self, state: &State) {
        if state.withs[13] != 0 {
            // a line in a template's text
            return self.load_heex(state);
        }
        let mut w = [0u8; 16];
        w[..14].copy_from_slice(&state.levels);
        self.frames = u128::from_le_bytes(w);
        self.depth = state.depth;
        let mut s = [0u8; 8];
        s.copy_from_slice(&state.withs[..8]);
        self.strs = u64::from_le_bytes(s);
        self.ctx = state.withs[8];
        self.mode = (state.arg & 3) as u8;
        self.carried = state.arg & ARG_CONT;
        if state.withs[9] != 0 {
            // a template's HTML below an island
            self.load_below(state);
        }
    }

    /// The state the scan's last line left: every field as it is, so a block lexes as its
    /// lines do one at a time.
    #[inline(always)]
    fn state(&self) -> State {
        if self.heex.is_some() {
            return self.state_cold();
        }
        self.state_code()
    }

    /// [`Scan::state`] where a template was open: its layout ([`Scan::state_heex`]) where the
    /// line ends in its text, HTML's state below an island ([`Scan::html_below`]) where it ends
    /// in one.
    #[cold]
    #[inline(never)]
    fn state_cold(&self) -> State {
        let sk = (self.strs & 0xff) as u8;
        if self.mode == M_STRING && sk & KIND == K_HEEX {
            return self.state_heex(sk);
        }
        let mut s = self.state_code();
        if (0..8).any(|i| (self.strs >> (8 * i)) as u8 & KIND == K_HEEX) {
            self.html_below(&mut s);
        }
        s
    }

    #[inline(always)]
    fn state_code(&self) -> State {
        let w = self.frames.to_le_bytes();
        let mut s = State::default();
        s.levels.copy_from_slice(&w[..14]);
        s.withs[..8].copy_from_slice(&self.strs.to_le_bytes());
        s.withs[8] = self.ctx;
        s.depth = self.depth;
        s.arg = u16::from(self.mode) | self.carried;
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
        if self.heex.is_some() {
            // a block keeps what the state keeps, as its lines one at a time do
            let s = self.state();
            self.load(&s);
        }
    }

    /// A write: one call on the run sink.
    #[inline(always)]
    fn put(&mut self, s: usize, e: usize, kind: u16) {
        self.k.merge_call(&mut self.out, s, e, kind);
    }

    /// A write inlined, for the common tokens.
    #[inline(always)]
    fn put_hot(&mut self, s: usize, e: usize, kind: u16) {
        self.k.merge(&mut self.out, s, e, kind);
    }

    #[inline(always)]
    fn top(&self) -> u8 {
        (self.frames & 15) as u8
    }

    #[inline(always)]
    fn set_top(&mut self, f: u8) {
        if self.depth > 0 {
            self.frames = self.frames & !15 | u128::from(f);
        }
    }

    #[inline(always)]
    fn push(&mut self, f: u8) {
        self.frames = (self.frames << FRAME_BITS | u128::from(f)) & FRAME_MASK;
        self.depth = self.depth.saturating_add(1);
    }

    #[inline(always)]
    fn pop(&mut self) {
        if self.depth > 0 {
            self.frames >>= FRAME_BITS;
            self.depth -= 1;
        }
    }

    /// The frame `i` levels below the top (0 the top); 0 past the kept levels.
    #[inline(always)]
    fn frame(&self, i: u8) -> u8 {
        if i >= self.depth.min(LEVELS) {
            return 0;
        }
        ((self.frames >> (FRAME_BITS * u32::from(i))) & 15) as u8
    }

    /// The line, from what the line before left: a string's rest first; the loop out of line
    /// where `SHARED` ([`Scan::code_shared`]).
    #[inline(always)]
    fn lex_line<const SHARED: bool>(&mut self) {
        self.fn_depth = NO_FN;
        self.callw = (0, usize::MAX);
        self.lead = usize::MAX;
        if let Some(h) = self.heex.as_mut() {
            h.line = false;
        }
        let mut p = 0;
        if self.mode != M_CODE {
            p = self.string_rest();
        } else {
            let b = self.b;
            while p < b.len() && matches!(b[p], b' ' | b'\t') {
                p += 1;
            }
            self.lead = p;
            if !SOL && self.top() == F_HEAD {
                self.head_line_start(p);
            }
        }
        if matches!(self.top(), F_BINARY | F_BINTYPE) {
            // bat's segment type goes on past a line's end, its blanks with it
            p = self.binary(if self.mode == M_CODE && p == self.lead {
                0
            } else {
                p
            });
        }
        if SHARED {
            self.code_shared(p);
        } else {
            self.code(p);
        }
        if !SOL && self.top() == F_HEAD && self.mode == M_CODE {
            self.head_line_end();
        }
    }

    /// [`Scan::code`] out of line: on the run sink, one copy that the line API and a block's
    /// lines share; on the span sink, which ada doesn't hold, inlined.
    #[inline(never)]
    fn code_shared(&mut self, p: usize) {
        self.code(p);
    }

    /// The code from `p` to the line's end.
    #[inline(always)]
    fn code(&mut self, mut p: usize) {
        let b = self.b;
        let n = b.len();
        while p < n {
            let c = b[p];
            let mut act = ACT[usize::from(c)];
            if act == A_WIDE && is_word_starting(b, p) {
                act = A_NAME;
            }
            p = match act {
                A_SPACE => {
                    let mut e = p + 1;
                    while e < n && ACT[usize::from(b[e])] == A_SPACE {
                        e += 1;
                    }
                    e
                }
                A_NAME => self.name(p),
                A_DIGIT => self.number(p),
                A_DQUOTE => self.dquote(p),
                A_SQUOTE => self.squote(p),
                A_COLON => self.colon(p),
                A_AT => self.at_sign(p),
                A_HASH => self.hash(p),
                A_TILDE => self.tilde(p),
                A_QUEST => self.quest(p),
                A_AMP => self.amp(p),
                A_CARET => self.caret(p),
                A_OP => self.op(p),
                A_DOT => {
                    if b.get(p + 1) == Some(&b'.') {
                        self.put(p, p + 2, OP_RANGE);
                        p + 2
                    } else {
                        self.put_hot(p, p + 1, DOT);
                        p + 1
                    }
                }
                A_COMMA => {
                    let k = if self.top() == F_TUPLE {
                        SEQ_SEP
                    } else {
                        COMMA
                    };
                    self.put_hot(p, p + 1, k);
                    p + 1
                }
                A_SEMI => {
                    self.put(p, p + 1, SEMI);
                    p + 1
                }
                A_LPAREN => {
                    let f = if !SOL && self.callw.1 != usize::MAX {
                        self.call_paren(p)
                    } else {
                        F_PAREN
                    };
                    self.push(f);
                    self.put_hot(p, p + 1, FUNCTION);
                    p + 1
                }
                A_RPAREN => self.rparen(p),
                A_LBRACKET => {
                    self.push(F_BRACKET);
                    self.put_hot(p, p + 1, ARRAY);
                    p + 1
                }
                A_RBRACKET => {
                    if self.top() == F_BRACKET {
                        self.pop();
                    }
                    self.put_hot(p, p + 1, ARRAY);
                    p + 1
                }
                A_LBRACE => {
                    self.push(F_TUPLE);
                    self.put_hot(p, p + 1, SEQ_BEGIN);
                    p + 1
                }
                A_RBRACE => self.rbrace(p),
                A_PERCENT => self.percent(p),
                A_WIDE => p + char_len(c),
                _ => p + 1,
            };
        }
    }

    /// `)` at `p`: a group's or a call's closed, with any bracket left open above it.
    #[inline(always)]
    fn rparen(&mut self, p: usize) -> usize {
        match self.top() {
            F_PAREN | F_SQLCALL | F_RECALL => self.pop(),
            F_BRACKET if matches!(self.frame(1), F_PAREN | F_SQLCALL | F_RECALL) => {
                self.pop();
                self.pop();
            }
            _ => {}
        }
        if self.fn_depth != NO_FN && u16::from(self.depth) <= self.fn_depth {
            // bat's `fn` head ends at a `)`
            self.fn_depth = NO_FN;
        }
        self.put_hot(p, p + 1, FUNCTION);
        p + 1
    }

    /// `}` at `p`: a tuple's or a map's closed, a `#{…}`'s (its string goes on), or a stray
    /// one (bat's invalid).
    #[inline(always)]
    fn rbrace(&mut self, p: usize) -> usize {
        match self.top() {
            F_TUPLE => {
                self.pop();
                self.put_hot(p, p + 1, SEQ_END);
                p + 1
            }
            F_INTERP => self.interp_close(p),
            F_ISLAND => self.island_close(p, 1),
            _ => self.unwind_brace(p),
        }
    }

    /// A `}` at `p` with frames above its tuple or `#{…}`: they're dropped, if one is open below
    /// them; a stray one if none is.
    #[cold]
    #[inline(never)]
    fn unwind_brace(&mut self, p: usize) -> usize {
        let levels = self.depth.min(LEVELS);
        let mut i = 0;
        while i < levels {
            match self.frame(i) {
                F_TUPLE => {
                    for _ in 0..=i {
                        self.pop();
                    }
                    self.put(p, p + 1, SEQ_END);
                    return p + 1;
                }
                F_INTERP => {
                    for _ in 0..i {
                        self.pop();
                    }
                    return self.interp_close(p);
                }
                F_ISLAND => {
                    for _ in 0..i {
                        self.pop();
                    }
                    return self.island_close(p, 1);
                }
                _ => {}
            }
            i += 1;
        }
        self.put(p, p + 1, STRAY);
        p + 1
    }

    /// A word at `p`: a keyword, an operator's word, a constant, a key, a module's name, or a
    /// name.
    #[inline(always)]
    fn name(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut e = p + 1;
        while e < n && WORD[usize::from(b[e])] {
            e += 1;
        }
        if e < n && b[e] >= 0x80 {
            e = word_end(b, e);
        }
        let next = if e < n { b[e] } else { 0 };
        let w = WORDS.value(&b[p..e], &INFO);
        if w != 0 {
            let class = w & (7 << 8);
            let after_dot = p > 0 && b[p - 1] == b'.';
            match class {
                W_KW if !after_dot && !matches!(next, b'?' | b'!' | b':') => {
                    if !SOL
                        && w & W_ACTION != 0
                        && let Some(r) = self.word_action(p, e, w)
                    {
                        return r;
                    }
                    self.put_hot(p, e, KEYWORD);
                    return e;
                }
                W_LOGICAL => {
                    self.put(p, e, OP_LOGICAL);
                    return e;
                }
                W_OPWORD if !after_dot => {
                    let mut kind = OP_WORD;
                    if !SOL && w & W_ACTION == W_WHEN {
                        kind = self.when_kind();
                    }
                    self.put(p, e, kind);
                    return e;
                }
                W_CONST if !matches!(next, b'?' | b'!') => {
                    self.put_hot(p, e, w & W_LEAF);
                    return e;
                }
                W_GUARD if !SOL && next != b':' && self.in_head() => {
                    self.put(p, e, GUARD);
                    return e;
                }
                W_CALL if !SOL => self.callw = (p, e),
                _ => {}
            }
        }
        // bat's `[\p{L}_][\w@]*[?!]?:` a keyword's key, not before `::`
        let mut q = e;
        if matches!(next, b'?' | b'!' | b'@') {
            q = key_end(b, e);
        }
        if b.get(q) == Some(&b':') && b.get(q + 1) != Some(&b':') {
            if !SOL && self.top() == F_HEAD && &b[p..q] == b"do" {
                // bat's `\bdo:` ends a function's head as one token
                self.pop();
                self.put(p, q + 1, KEYS);
                return q + 1;
            }
            self.put_hot(p, q, KEYS);
            self.put_hot(q, q + 1, KEYS_MARK);
            return q + 1;
        }
        let c = b[p];
        if c.is_ascii_uppercase() && is_ascii_word(&b[p..e]) && next != b':' {
            // bat's `\b[A-Z][a-zA-Z0-9_]*\b(?!:)`: an alias
            self.put_hot(p, e, CLASS);
            return e;
        }
        e
    }

    /// A word `b[p..e]` (`w` its info) whose meaning hangs on where it stands: a function's or
    /// a module's head at a line's start, `fn`, `do`; `None` for the keyword as anywhere else.
    #[inline(never)]
    fn word_action(&mut self, p: usize, e: usize, w: u16) -> Option<usize> {
        match w & W_ACTION {
            W_DEF | W_DEFP if p == self.lead => self.def_head(p, e, w & W_ACTION == W_DEFP),
            W_MODULE if p == self.lead => {
                self.put(p, e, MODULE_KW);
                Some(self.module_head(e, false))
            }
            W_PROTOCOL if p == self.lead => {
                self.put(p, e, PROTOCOL_KW);
                Some(self.module_head(e, true))
            }
            W_FN => {
                // bat's `\b(fn)\b(?=.*->)`: a `when` before the `->` is a keyword
                if self.fn_depth == NO_FN
                    && !matches!(self.top(), F_TUPLE | F_BINARY | F_BINTYPE)
                    && has_arrow(self.b, e)
                {
                    self.fn_depth = u16::from(self.depth);
                }
                None
            }
            W_DO if self.top() == F_HEAD => {
                self.pop();
                self.put(p, e, MODULE_KW);
                Some(e)
            }
            _ => None,
        }
    }

    /// A `(` at `p` after a SQL or regex call's name (`query(`, `query!(`): its frame.
    #[cold]
    #[inline(never)]
    fn call_paren(&mut self, p: usize) -> u8 {
        let b = self.b;
        let (s, e) = self.callw;
        self.callw = (0, usize::MAX);
        if !(e == p || (e + 1 == p && b[e] == b'!')) {
            return F_PAREN;
        }
        let w = WORDS.value(&b[s..e], &INFO);
        if w & W_ACTION == W_SQL && embed::sql_call(b, s, e) {
            F_SQLCALL
        } else if w & W_ACTION == W_RE && embed::regex_call(b, s, e) {
            F_RECALL
        } else {
            F_PAREN
        }
    }

    /// `when`'s kind: a keyword in an `fn`'s head (bat's), an operator elsewhere.
    #[inline(never)]
    fn when_kind(&mut self) -> u16 {
        if self.fn_depth != NO_FN && u16::from(self.depth) == self.fn_depth {
            self.fn_depth = NO_FN;
            return KEYWORD;
        }
        OP_WORD
    }

    /// Whether a function's head is the context bat reads a word in: the innermost frame but
    /// parentheses and brackets (which aren't contexts in bat's grammar).
    #[inline(never)]
    fn in_head(&self) -> bool {
        let levels = self.depth.min(LEVELS);
        let mut i = 0;
        while i < levels {
            match self.frame(i) {
                F_HEAD => return true,
                F_PAREN | F_BRACKET | F_SQLCALL | F_RECALL => {}
                _ => return false,
            }
            i += 1;
        }
        false
    }

    /// `def name` at a line's start (bat's `^\s*(def|defmacro)\s+([\p{L}_]\w*[!?]?)(\()?`): the
    /// keyword, the function's name, its parameters' `(`, and the head to its `do`.
    #[inline(never)]
    fn def_head(&mut self, p: usize, e: usize, private: bool) -> Option<usize> {
        let b = self.b;
        let mut q = e;
        while q < b.len() && matches!(b[q], b' ' | b'\t') {
            q += 1;
        }
        if q == e || q >= b.len() {
            return None;
        }
        let c = b[q];
        if !(c.is_ascii_alphabetic() || c == b'_' || (c >= 0x80 && is_word_starting(b, q))) {
            return None;
        }
        let mut r = word_end(b, q);
        let w = WORDS.value(&b[q..r], &INFO);
        if matches!(b.get(r), Some(b'!' | b'?')) {
            r += 1;
        }
        self.put(p, e, MODULE_KW);
        // ada's: `def unquote(name)(…)`'s name is `unquote`'s, a keyword (bat's the name's)
        let kind = if w & W_ACTION == W_UNQUOTE && r == q + 7 {
            KEYWORD
        } else if private {
            FUN_PRIVATE
        } else {
            FUN_PUBLIC
        };
        self.put(q, r, kind);
        // a head left from a line before ends here (bat's nests one in it)
        if self.top() == F_HEAD {
            self.pop();
        }
        self.push(F_HEAD);
        if b.get(r) == Some(&b'(') {
            self.push(F_PAREN);
            self.put(r, r + 1, PARAMS);
            return Some(r + 1);
        }
        Some(r)
    }

    /// A module's or a protocol's head after its keyword, from `p` (bat's to `do`, here to the
    /// line's end too): its names colored, the rest plain.
    #[inline(never)]
    fn module_head(&mut self, mut p: usize, protocol: bool) -> usize {
        let b = self.b;
        let n = b.len();
        let (name, kw) = if protocol {
            (PROTOCOL_NAME, PROTOCOL_KW)
        } else {
            (CLASS, MODULE_KW)
        };
        while p < n {
            let c = b[p];
            if c == b'#' {
                // a comment, as Elixir reads one (bat's head reads on through it)
                return p;
            }
            if c.is_ascii_alphanumeric() || c == b'_' || (c >= 0x80 && is_word_starting(b, p)) {
                let e = word_end(b, p);
                if &b[p..e] == b"do" {
                    self.put(p, e, kw);
                    return e;
                }
                if c.is_ascii_uppercase() && is_ascii_word(&b[p..e]) {
                    self.put(p, e, name);
                }
                p = e;
                continue;
            }
            p += char_len(c);
        }
        n
    }

    /// A line that starts with a function's head open: it goes on where the line before ended
    /// in a `,`, an operator or `when`, or this one starts with `when`, `do` or an operator (as
    /// Elixir's tokenizer drops a line's end before an operator); else it ended with its line
    /// (a head without a body: a protocol's, a default's).
    #[cold]
    #[inline(never)]
    fn head_line_start(&mut self, p: usize) {
        let cont = self.carried & ARG_CONT != 0;
        self.carried &= !ARG_CONT;
        if cont {
            return;
        }
        let b = self.b;
        let Some(&c) = b.get(p) else {
            // a blank line: the head waits on
            return self.carried |= ARG_CONT;
        };
        let goes_on = match c {
            b'|' | b'=' | b'-' | b'+' | b'*' | b'/' | b'<' | b'>' | b'&' | b'!' | b'\\' | b'.'
            | b',' => true,
            b'#' => {
                // a comment line: the head waits on
                self.carried |= ARG_CONT;
                true
            }
            b'w' | b'd' => {
                let e = word_end(b, p);
                matches!(&b[p..e], b"when" | b"do")
            }
            _ => false,
        };
        if !goes_on {
            self.pop();
        }
    }

    /// A line that ends with a function's head open: whether its last token goes on to the next
    /// line (a `,`, an operator, `when`), its trailing comment aside.
    #[cold]
    #[inline(never)]
    fn head_line_end(&mut self) {
        let b = self.b;
        let mut e = b.len();
        // a trailing comment: a `#` outside the line's strings, roughly the last one after code
        if let Some(h) = b.iter().position(|&c| c == b'#') {
            e = h;
        }
        while e > 0 && matches!(b[e - 1], b' ' | b'\t' | b'\r') {
            e -= 1;
        }
        let goes_on = e > 0
            && (matches!(
                b[e - 1],
                b',' | b'|' | b'=' | b'-' | b'+' | b'*' | b'/' | b'<' | b'>' | b'&' | b'\\'
            ) || (e >= 4
                && &b[e - 4..e] == b"when"
                && (e == 4 || !WORD[usize::from(b[e - 5])])));
        if goes_on {
            self.carried |= ARG_CONT;
        }
    }

    /// A digit at `p`: a number (bat's, `\b` both sides), or a word's run if a word character
    /// follows it.
    #[inline(always)]
    fn number(&mut self, p: usize) -> usize {
        let b = self.b;
        let e = number_end(b, p);
        if e < b.len() && WORD[usize::from(b[e])] {
            let mut f = e;
            while f < b.len() && WORD[usize::from(b[f])] {
                f += 1;
            }
            return f;
        }
        self.put_hot(p, e, NUMBER);
        e
    }

    /// `:` at `p`: an atom (`:name`, `:"…"`, `:+`), `::`, or a plain `:`.
    #[inline(always)]
    fn colon(&mut self, p: usize) -> usize {
        let b = self.b;
        if p > 0 && b[p - 1] == b':' {
            return p + 1;
        }
        let Some(&c) = b.get(p + 1) else {
            return p + 1;
        };
        if c.is_ascii_alphabetic() || c == b'_' || (c >= 0x80 && is_word_starting(b, p + 1)) {
            // bat's `:[\p{L}_][\w@]*([?!]|=(?![>=]))?`
            let mut e = word_end(b, p + 1);
            while b.get(e) == Some(&b'@') {
                e = word_end(b, e + 1);
            }
            match b.get(e) {
                Some(b'?' | b'!') => e += 1,
                Some(b'=') if !matches!(b.get(e + 1), Some(b'>' | b'=')) => e += 1,
                _ => {}
            }
            self.put_hot(p, p + 1, SYMBOL_MARK);
            self.put_hot(p + 1, e, SYMBOL);
            return e;
        }
        match c {
            b'"' | b'\'' => {
                let sk = K_ATOM | if c == b'"' { D_DQ } else { D_SQ };
                self.put(p, p + 2, flavor(sk) + BEGIN);
                return self.str_body(p + 2, sk);
            }
            b':' if b.get(p + 2) != Some(&b':') => return p + 2,
            _ => {}
        }
        let k = op_atom_len(b, p + 1);
        if k > 0 {
            self.put(p, p + 1, SYMBOL_MARK);
            self.put(p + 1, p + 1 + k, SYMBOL);
            return p + 1 + k;
        }
        p + 1
    }

    /// `@` at `p`: a module attribute (bat's `@[a-z]\w*`), a documentation attribute, or a
    /// plain `@`.
    #[inline(always)]
    fn at_sign(&mut self, p: usize) -> usize {
        let b = self.b;
        if !b.get(p + 1).is_some_and(u8::is_ascii_lowercase) {
            return p + 1;
        }
        let e = word_end(b, p + 1);
        if !SOL
            && b.get(e) == Some(&b' ')
            && matches!(&b[p + 1..e], b"doc" | b"moduledoc" | b"typedoc")
            && let Some(r) = self.doc(p, e + 1)
        {
            return r;
        }
        self.put_hot(p, p + 1, ATTR_MARK);
        self.put_hot(p + 1, e, ATTR);
        e
    }

    /// `?` at `p`: a character (bat's `(?<!\w)\?(\\(x\h{1,2}…|[^xMC])|[^\s\\])`), or a plain
    /// `?`.
    #[inline(always)]
    fn quest(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        if word_before(b, p) {
            return p + 1;
        }
        let Some(&c) = b.get(p + 1) else {
            return p + 1;
        };
        let e = match c {
            b'\\' => match b.get(p + 2) {
                None => p + 2,
                Some(b'x') => {
                    let mut e = p + 3;
                    while e < n && e < p + 5 && b[e].is_ascii_hexdigit() {
                        e += 1;
                    }
                    if e == p + 3
                        || (e < n && b[e].is_ascii_hexdigit())
                        || (e < n && WORD[usize::from(b[e])])
                    {
                        return p + 1;
                    }
                    e
                }
                Some(b'M' | b'C') => return p + 1,
                Some(&d) => (p + 2 + char_len(d)).min(n),
            },
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => return p + 1,
            _ => (p + 1 + char_len(c)).min(n),
        };
        self.put(p, e, NUMBER);
        e
    }

    /// `&` at `p`: `&&&`, `&&`, a capture (`&1`, `&name/2`, `&name(`), or a plain `&`.
    #[inline(always)]
    fn amp(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        match b.get(p + 1) {
            Some(b'&') => {
                if b.get(p + 2) == Some(&b'&') {
                    self.put(p, p + 3, OP_BITWISE);
                    return p + 3;
                }
                self.put(p, p + 2, OP_LOGICAL);
                return p + 2;
            }
            Some(c) if c.is_ascii_digit() => {
                let mut e = p + 2;
                while e < n && b[e].is_ascii_digit() {
                    e += 1;
                }
                self.put(p, p + 1, CAPTURE_MARK);
                self.put(p + 1, e, CAPTURE);
                return e;
            }
            Some(&c) if WORD[usize::from(c)] || (c >= 0x80 && is_word(b, p + 1)) => {
                let mut e = word_end(b, p + 1);
                // ada's: a name's `?` or `!` in it (`&valid?/1`), bat's `\w+` leaves it out
                if !SOL && matches!(b.get(e), Some(b'?' | b'!')) {
                    e += 1;
                }
                let calls = match b.get(e) {
                    Some(b'(') => true,
                    Some(b'/') => b.get(e + 1).is_some_and(u8::is_ascii_digit),
                    _ => false,
                };
                if calls {
                    self.put(p, p + 1, CAPTURE_VAR_MARK);
                    self.put(p + 1, e, CAPTURE_VAR);
                    return e;
                }
            }
            _ => {}
        }
        p + 1
    }

    /// `^` at `p`: `^^^`, a pinned variable (`^x`), or a plain `^`.
    #[inline(always)]
    fn caret(&mut self, p: usize) -> usize {
        let b = self.b;
        if b.get(p + 1..p + 3) == Some(b"^^") {
            self.put(p, p + 3, OP_BITWISE);
            return p + 3;
        }
        if b.get(p + 1).is_some_and(u8::is_ascii_lowercase) {
            let e = word_end(b, p + 1);
            self.put(p, p + 1, PIN_MARK);
            self.put(p + 1, e, PIN);
            return e;
        }
        p + 1
    }

    /// `~` at `p`: a sigil, `~~~`, or bat's other operator.
    #[inline(always)]
    fn tilde(&mut self, p: usize) -> usize {
        let b = self.b;
        match b.get(p + 1) {
            Some(c) if c.is_ascii_alphabetic() => self.sigil(p),
            Some(b'~') if b.get(p + 2) == Some(&b'~') => {
                self.put(p, p + 3, OP_BITWISE);
                p + 3
            }
            _ => {
                self.put(p, p + 1, OP_OTHER);
                p + 1
            }
        }
    }

    /// An operator at `p`, bat's punctuation, longest first.
    #[inline(always)]
    fn op(&mut self, p: usize) -> usize {
        let b = self.b;
        let c = b[p];
        let c1 = b.get(p + 1).copied().unwrap_or(0);
        let c2 = b.get(p + 2).copied().unwrap_or(0);
        let (len, kind) = match c {
            b'<' => match c1 {
                b'<' if c2 == b'<' => (3, OP_BITWISE),
                b'<' => return self.binary_open(p),
                b'-' => {
                    self.fn_arrow();
                    (2, OP_ARROW)
                }
                b'>' => (2, OP_CONCAT),
                b'=' => (2, OP_COMPARE),
                _ => (1, OP_COMPARE),
            },
            b'>' => match c1 {
                b'>' if c2 == b'>' => (3, OP_BITWISE),
                b'>' if matches!(self.top(), F_BINARY | F_BINTYPE) => {
                    return self.binary_close(p);
                }
                b'=' => (2, OP_COMPARE),
                _ => (1, OP_COMPARE),
            },
            b'-' => match c1 {
                b'>' => {
                    self.fn_arrow();
                    (2, OP_ARROW)
                }
                b'-' => (2, OP_LIST),
                _ => (1, OP_ARITH),
            },
            b'+' => {
                if c1 == b'+' {
                    (2, OP_LIST)
                } else {
                    (1, OP_ARITH)
                }
            }
            b'*' => {
                if c1 == b'*' {
                    (2, OP_OTHER)
                } else {
                    (1, OP_ARITH)
                }
            }
            b'/' => (1, OP_ARITH),
            b'=' => match c1 {
                b'=' => (if c2 == b'=' { 3 } else { 2 }, OP_COMPARE),
                b'>' => (2, OP_MAP),
                _ => (1, OP_ASSIGN),
            },
            b'!' => {
                if c1 == b'=' {
                    (if c2 == b'=' { 3 } else { 2 }, OP_COMPARE)
                } else if p > 0 && matches!(b[p - 1], b' ' | b'\t') {
                    // bat's `(?<=[ \t])!+`
                    let mut e = p + 1;
                    while e < b.len() && b[e] == b'!' {
                        e += 1;
                    }
                    (e - p, OP_LOGICAL)
                } else {
                    return p + 1;
                }
            }
            b'|' => match c1 {
                b'>' => (2, OP_PIPE),
                b'|' if c2 == b'|' => (3, OP_BITWISE),
                _ => (1, OP_OTHER),
            },
            _ => {
                // `\`
                if c1 == b'\\' {
                    (2, OP_OTHER)
                } else {
                    return p + 1;
                }
            }
        };
        self.put_hot(p, p + len, kind);
        p + len
    }

    /// An arrow (`->`, `<-`) at the `fn`'s depth: bat's `fn` head ends there.
    #[inline(always)]
    fn fn_arrow(&mut self) {
        self.fn_depth = NO_FN;
    }

    /// `<<` at `p`: a binary, its segments read by [`Scan::binary`].
    #[inline(never)]
    fn binary_open(&mut self, p: usize) -> usize {
        self.push(F_BINARY);
        self.put(p, p + 2, BIN_BEGIN);
        self.binary(p + 2)
    }

    /// `>>` at `p` closing a binary.
    #[inline(never)]
    fn binary_close(&mut self, p: usize) -> usize {
        if self.top() == F_BINTYPE {
            self.pop();
        }
        self.pop();
        self.put(p, p + 2, BIN_END);
        p + 2
    }

    /// A binary's segments from `p` (bat's `binary` context, its type's after `::`): numbers,
    /// strings, `,` and `::` colored, the rest plain, to its `>>` or the line's end; a string
    /// whose `#{` opens hands the line to the code loop.
    #[inline(never)]
    fn binary(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        if self.top() == F_BINTYPE {
            p = self.bintype(p);
        }
        while p < n {
            let c = b[p];
            match c {
                b'>' if b.get(p + 1) == Some(&b'>') && b.get(p + 2) != Some(&b'>') => {
                    return self.binary_close(p);
                }
                b',' => {
                    self.put(p, p + 1, BIN_SEP);
                    p += 1;
                }
                b':' if b.get(p + 1) == Some(&b':') => {
                    self.put(p, p + 2, BIN_TYPE_SEP);
                    self.push(F_BINTYPE);
                    p = self.bintype(p + 2);
                }
                b'"' | b'\'' => {
                    let sk = K_STR | if c == b'"' { D_DQ } else { D_SQ };
                    self.put(p, p + 1, flavor(sk) + BEGIN);
                    p = self.str_body(p + 1, sk);
                    if self.mode != M_CODE || self.top() == F_INTERP {
                        // the string goes on past the line, or its `#{…}` is code
                        return p;
                    }
                }
                b'0'..=b'9' if !word_before(b, p) => {
                    let e = number_end(b, p);
                    if e < n && WORD[usize::from(b[e])] {
                        p = skip_word(b, e);
                    } else {
                        self.put(p, e, BIN_NUM);
                        p = e;
                    }
                }
                _ if WORD[usize::from(c)] => p = skip_word(b, p + 1),
                _ => p += char_len(c),
            }
        }
        n
    }

    /// A segment's type from `p` (bat's after `::`: every byte its, a name with a number or a
    /// name in parentheses, `size(8)`, `size(len)`, a number), to its `,` or the binary's `>>`
    /// (popped there), or the line's end.
    #[inline(never)]
    fn bintype(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut s = p;
        while p < n {
            let c = b[p];
            match c {
                b',' => break,
                b'>' if b.get(p + 1) == Some(&b'>') && b.get(p + 2) != Some(&b'>') => break,
                b'a'..=b'z' => {
                    let mut e = p + 1;
                    while e < n && (WORD[usize::from(b[e])] || b[e] == b'-') {
                        e += 1;
                    }
                    p = e;
                    if b.get(e) == Some(&b'(') {
                        let d = e + 1;
                        let mut f = d;
                        while f < n && b[f].is_ascii_digit() {
                            f += 1;
                        }
                        if f > d && b.get(f) == Some(&b')') {
                            self.put(s, d, BIN_TYPE);
                            self.put(d, f, BIN_TYPE_NUM);
                            (s, p) = (f, f + 1);
                            continue;
                        }
                        if b.get(d)
                            .is_some_and(|&x| x.is_ascii_lowercase() || x == b'_')
                        {
                            let mut f = d + 1;
                            while f < n && WORD[usize::from(b[f])] {
                                f += 1;
                            }
                            if f > d + 1 && b.get(f) == Some(&b')') {
                                self.put(s, d, BIN_TYPE);
                                self.put(d, f, BIN_TYPE_VAR);
                                (s, p) = (f, f + 1);
                                continue;
                            }
                        }
                    }
                }
                b'0'..=b'9' if !word_before(b, p) => {
                    let e = number_end(b, p);
                    if e < n && WORD[usize::from(b[e])] {
                        p = skip_word(b, e);
                    } else {
                        self.put(s, p, BIN_TYPE);
                        self.put(p, e, BIN_TYPE_NUM);
                        (s, p) = (e, e);
                    }
                }
                _ if WORD[usize::from(c)] => p = skip_word(b, p + 1),
                _ => p += char_len(c),
            }
        }
        let p = p.min(n);
        self.put(s, p, BIN_TYPE);
        if p < n {
            self.pop();
        }
        p
    }
}

/// Where a key's name that a `?`, `!` or `@` follows at `e` ends: bat's `[\w@]*[?!]?`.
fn key_end(b: &[u8], mut e: usize) -> usize {
    while b.get(e) == Some(&b'@') {
        e = word_end(b, e + 1);
    }
    if matches!(b.get(e), Some(b'?' | b'!')) {
        e += 1;
    }
    e
}

/// Whether `w` is ASCII letters, digits and `_` only.
#[inline(always)]
fn is_ascii_word(w: &[u8]) -> bool {
    w.iter().all(|&c| WORD[usize::from(c)])
}

/// Whether a word's character (bat's `\w`) is right before `p`.
#[inline(always)]
fn word_before(b: &[u8], p: usize) -> bool {
    if p == 0 {
        return false;
    }
    let c = b[p - 1];
    if c < 0x80 {
        return WORD[usize::from(c)];
    }
    // the character that ends at `p`
    let mut s = p - 1;
    while s > 0 && b[s] & 0xc0 == 0x80 {
        s -= 1;
    }
    is_word(b, s)
}

/// Where the run of word characters from `p` ends.
#[inline(always)]
fn skip_word(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && WORD[usize::from(b[p])] {
        p += 1;
    }
    p
}

/// Whether `->` is on the line from `p` (bat's `(?=.*->)`).
fn has_arrow(b: &[u8], p: usize) -> bool {
    b.get(p..).is_some_and(|r| r.windows(2).any(|w| w == b"->"))
}

/// An operator's atom after `:` at `q`, its length (bat's alternation, first that matches):
/// `<>`, `===`, `!==`, `<<>>`, `<<<`, `>>>`, `~~~`, `::`, `<-`, `|>`, `=>`, `=~`, `=`, `/`,
/// `\\`, `**`, `...`, `>=`, `<=`, `&&&`, `++`, `--`, `|||`, `!`, `@`, `%{}`, `%`, `[]`, `^^^`.
fn op_atom_len(b: &[u8], q: usize) -> usize {
    let at = |i: usize| b.get(q + i).copied().unwrap_or(0);
    let starts = |s: &[u8]| b.get(q..q + s.len()) == Some(s);
    match at(0) {
        b'<' => {
            if starts(b"<>") {
                2
            } else if starts(b"<<>>") {
                4
            } else if starts(b"<<<") {
                3
            } else if starts(b"<-") || at(1) == b'=' {
                2
            } else {
                1
            }
        }
        b'=' => {
            if starts(b"==") {
                if at(2) == b'=' { 3 } else { 2 }
            } else if matches!(at(1), b'>' | b'~') {
                2
            } else {
                1
            }
        }
        b'!' => {
            if starts(b"!=") {
                if at(2) == b'=' { 3 } else { 2 }
            } else {
                1
            }
        }
        b'>' => {
            if starts(b">>>") {
                3
            } else if at(1) == b'=' {
                2
            } else {
                1
            }
        }
        b'~' if starts(b"~~~") => 3,
        b':' if at(1) == b':' => 2,
        b'|' => {
            if at(1) == b'>' {
                2
            } else if at(1) == b'|' {
                if at(2) == b'|' { 3 } else { 2 }
            } else {
                1
            }
        }
        b'/' => 1,
        b'\\' if at(1) == b'\\' => 2,
        b'*' => {
            if at(1) == b'*' {
                2
            } else {
                1
            }
        }
        b'.' => {
            if at(1) == b'.' {
                if at(2) == b'.' { 3 } else { 2 }
            } else {
                1
            }
        }
        b'&' => {
            if at(1) == b'&' {
                if at(2) == b'&' { 3 } else { 2 }
            } else {
                1
            }
        }
        b'+' => {
            if at(1) == b'+' {
                2
            } else {
                1
            }
        }
        b'-' => {
            if at(1) == b'-' {
                2
            } else {
                1
            }
        }
        b'@' => 1,
        b'%' => {
            if starts(b"%{}") {
                3
            } else {
                1
            }
        }
        b'{' if at(1) == b'}' => 2,
        b'[' if at(1) == b']' => 2,
        b'^' => {
            if starts(b"^^^") {
                3
            } else {
                1
            }
        }
        _ => 0,
    }
}

/// Where a number at `p` ends (bat's: `0x` hex, `0b` binary, `0o` octal, or decimal with `_`, a
/// fraction and an exponent).
fn number_end(b: &[u8], p: usize) -> usize {
    let n = b.len();
    let digits = |mut q: usize, ok: fn(u8) -> bool, under: bool| {
        // `d(_?d)*`
        if q >= n || !ok(b[q]) {
            return None;
        }
        q += 1;
        loop {
            if q < n && ok(b[q]) {
                q += 1;
            } else if under && q + 1 < n && b[q] == b'_' && ok(b[q + 1]) {
                q += 2;
            } else {
                return Some(q);
            }
        }
    };
    if b[p] == b'0' {
        match b.get(p + 1) {
            Some(b'x') => {
                if let Some(e) = digits(p + 2, |c| c.is_ascii_hexdigit(), true) {
                    return e;
                }
            }
            Some(b'b') => {
                if let Some(e) = digits(p + 2, |c| matches!(c, b'0' | b'1'), false) {
                    return e;
                }
            }
            Some(b'o') => {
                if let Some(e) = digits(p + 2, |c| matches!(c, b'0'..=b'7'), false) {
                    return e;
                }
            }
            _ => {}
        }
    }
    let mut e = digits(p, |c| c.is_ascii_digit(), true).unwrap_or(p + 1);
    if b.get(e) == Some(&b'.')
        && let Some(f) = digits(e + 1, |c| c.is_ascii_digit(), true)
    {
        e = f;
    }
    if matches!(b.get(e), Some(b'e' | b'E')) {
        let mut q = e + 1;
        if matches!(b.get(q), Some(b'-' | b'+')) {
            q += 1;
        }
        if let Some(f) = digits(q, |c| c.is_ascii_digit(), true) {
            e = f;
        }
    }
    e
}
