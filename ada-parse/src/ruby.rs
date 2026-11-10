//! Ruby on a scanner of ada's own (X-100), from a floor probe: one loop over a line's bytes, a
//! byte's action one load from a table, a name read once and looked up once (its leaf, whether
//! a regex may follow it, what it reads past itself), and a byte stack of frames only for what
//! nests: a string's interpolations and the strings they sit in ([`Scan::frames`]). Where bat's
//! grammar lets a regex start (after an operator, a separator, a keyword that takes an
//! expression, at a line's start) is a place on the line ([`Scan::regex_at`]), not a context.
//! Here-documents wait on X-105's [`Queue`] and their bodies are a mode. A regex literal's body
//! is on [`crate::regex`] in Ruby's dialect (X-114), the line cut where Ruby's lexer ends it.
//! Kinds are bat's leaves, and the regex scanner's under a literal's.

use crate::emit::{Rules, Run, Runs, Sink, Spans};
use crate::heredoc::{Label, Lead, Queue, Tail};
use crate::prim::{WORD, char_len, is_name_start, is_word, is_word_starting, word_end};
use crate::words::{Words, text_len};
use crate::{Out, State};

/// The leaves: a token's scopes.
static LEAVES: &[&str] = &[
    "",
    // comments
    "comment.line.number-sign.ruby",
    "comment.line.number-sign.ruby punctuation.definition.comment.ruby",
    "comment.block.documentation.ruby",
    "comment.block.documentation.ruby punctuation.definition.comment.begin.ruby",
    "comment.block.documentation.ruby punctuation.definition.comment.end.ruby",
    // strings: a kind's text, its begin, its end (in [`S_SQ`]'s order)
    "string.quoted.single.ruby",
    "string.quoted.single.ruby punctuation.definition.string.begin.ruby",
    "string.quoted.single.ruby punctuation.definition.string.end.ruby",
    "string.quoted.double.ruby",
    "string.quoted.double.ruby punctuation.definition.string.begin.ruby",
    "string.quoted.double.ruby punctuation.definition.string.end.ruby",
    "string.interpolated.ruby",
    "string.interpolated.ruby punctuation.definition.string.begin.ruby",
    "string.interpolated.ruby punctuation.definition.string.end.ruby",
    "constant.other.symbol.single-quoted.ruby",
    "constant.other.symbol.single-quoted.ruby punctuation.definition.constant.ruby",
    "constant.other.symbol.single-quoted.ruby punctuation.definition.constant.ruby",
    "constant.other.symbol.double-quoted.ruby",
    "constant.other.symbol.double-quoted.ruby punctuation.definition.constant.ruby",
    "constant.other.symbol.double-quoted.ruby punctuation.definition.constant.ruby",
    "string.quoted.other.literal.lower.ruby",
    "string.quoted.other.literal.lower.ruby punctuation.definition.string.begin.ruby",
    "string.quoted.other.literal.lower.ruby punctuation.definition.string.end.ruby",
    "string.quoted.other.literal.lower.ruby",
    "string.quoted.other.literal.lower.ruby punctuation.definition.string.begin.ruby",
    "string.quoted.other.literal.lower.ruby punctuation.definition.string.end.ruby",
    "string.quoted.other.literal.upper.ruby",
    "string.quoted.other.literal.upper.ruby punctuation.definition.string.begin.ruby",
    "string.quoted.other.literal.upper.ruby punctuation.definition.string.end.ruby",
    "string.quoted.other.literal.other.ruby",
    "string.quoted.other.literal.other.ruby punctuation.definition.string.begin.ruby",
    "string.quoted.other.literal.other.ruby punctuation.definition.string.end.ruby",
    "string.quoted.double.ruby.mod",
    "string.quoted.double.ruby.mod punctuation.definition.string.begin.ruby",
    "string.quoted.double.ruby.mod punctuation.definition.string.end.ruby",
    "string.regexp.classic.ruby",
    "string.regexp.classic.ruby punctuation.definition.string.begin.ruby",
    "string.regexp.classic.ruby punctuation.definition.string.end.ruby",
    "string.regexp.mod-r.ruby",
    "string.regexp.mod-r.ruby punctuation.definition.string.begin.ruby",
    "string.regexp.mod-r.ruby punctuation.definition.string.end.ruby",
    "string.unquoted.heredoc.ruby",
    "string.unquoted.heredoc.ruby punctuation.definition.string.begin.ruby",
    "string.unquoted.heredoc.ruby punctuation.definition.string.end.ruby",
    // an embedded language's here-document that ada leaves plain: its text, never begun or ended
    "meta.embedded.ruby",
    "meta.embedded.ruby",
    "meta.embedded.ruby",
    "string.unquoted.program-block.ruby",
    // in strings
    "constant.character.escape.ruby",
    "invalid.illegal.escape.ruby",
    "constant.other.placeholder.ruby",
    "punctuation.section.interpolation.begin.ruby",
    "punctuation.section.interpolation.end.ruby",
    "string.regexp.classic.ruby keyword.other.ruby",
    "string.regexp.classic.ruby comment.line.number-sign.regexp",
    "string.regexp.mod-r.ruby comment.line.number-sign.regexp",
    // symbols, characters
    "constant.other.symbol.ruby",
    "constant.other.symbol.ruby punctuation.definition.constant.ruby",
    "constant.character.ruby",
    "constant.character.ruby punctuation.definition.constant.ruby",
    "constant.character.ruby invalid.illegal.character.ruby",
    // variables
    "variable.other.readwrite.instance.ruby",
    "variable.other.readwrite.instance.ruby punctuation.definition.variable.ruby",
    "variable.other.readwrite.class.ruby",
    "variable.other.readwrite.class.ruby punctuation.definition.variable.ruby",
    "variable.other.readwrite.global.ruby",
    "variable.other.readwrite.global.ruby punctuation.definition.variable.ruby",
    "variable.other.readwrite.global.pre-defined.ruby",
    "variable.other.readwrite.global.pre-defined.ruby punctuation.definition.variable.ruby",
    "variable.other.constant.ruby",
    "entity.name.constant.ruby",
    "variable.language.ruby",
    "constant.language.ruby",
    // keywords
    "keyword.control.block.begin.ruby",
    "keyword.control.block.end.ruby",
    "keyword.control.block.do.ruby",
    "keyword.control.conditional.case.ruby",
    "keyword.control.conditional.if.ruby",
    "keyword.control.conditional.elseif.ruby",
    "keyword.control.conditional.then.ruby",
    "keyword.control.conditional.unless.ruby",
    "keyword.control.conditional.when.ruby",
    "keyword.control.exception.ensure.ruby",
    "keyword.control.exception.rescue.ruby",
    "keyword.control.exception.catch.ruby",
    "keyword.control.loop.for.ruby",
    "keyword.control.loop.until.ruby",
    "keyword.control.loop.while.ruby",
    "keyword.control.loop.loop.ruby",
    "keyword.control.flow.break.ruby",
    "keyword.control.flow.next.ruby",
    "keyword.control.flow.redo.ruby",
    "keyword.control.flow.retry.ruby",
    "keyword.control.flow.return.ruby",
    "keyword.control.flow.yield.ruby",
    "keyword.control.flow.throw.ruby",
    "keyword.declaration.alias.ruby",
    "keyword.declaration.undef.ruby",
    "keyword.operator.logical.ruby",
    "entity.name.function.prepocessor.ruby",
    "support.function.builtin.ruby",
    "keyword.other.special-method.ruby",
    "keyword.control.import.ruby",
    "storage.type.function.ruby keyword.declaration.function.ruby",
    "storage.type.class.ruby keyword.declaration.class.ruby",
    "storage.type.namespace.ruby keyword.declaration.namespace.ruby",
    "invalid.deprecated.ruby",
    // declarations
    "entity.name.function.ruby",
    "entity.name.class.ruby",
    "entity.name.namespace.ruby",
    "variable.parameter.ruby",
    // operators
    "keyword.operator.other.ruby",
    "keyword.operator.assignment.augmented.ruby",
    "keyword.operator.comparison.ruby",
    "keyword.operator.arithmetic.ruby",
    "keyword.operator.assignment.ruby",
    "keyword.operator.bitwise.ruby",
    "keyword.operator.conditional.ruby",
    "keyword.operator.range.ruby",
    "keyword.operator.splat.ruby",
    "keyword.operator.ruby",
    // punctuation
    "punctuation.separator.key-value.ruby",
    "punctuation.separator.sequence.ruby",
    "punctuation.separator.ruby",
    "punctuation.separator.inheritance.ruby",
    "punctuation.terminator.statement.ruby",
    "punctuation.section.array.ruby",
    "punctuation.definition.group.begin.ruby",
    "punctuation.definition.group.end.ruby",
    "punctuation.section.scope.ruby",
    "punctuation.accessor.dot.ruby",
    "punctuation.accessor.double-colon.ruby",
    "punctuation.definition.parameters.begin.ruby",
    "punctuation.definition.parameters.end.ruby",
    "punctuation.operator.assignment.ruby",
    // numbers
    "constant.numeric.integer.decimal.ruby",
    "constant.numeric.integer.hexadecimal.ruby",
    "constant.numeric.integer.hexadecimal.ruby punctuation.definition.numeric.base.ruby",
    "constant.numeric.integer.octal.ruby",
    "constant.numeric.integer.octal.ruby punctuation.definition.numeric.base.ruby",
    "constant.numeric.integer.binary.ruby",
    "constant.numeric.integer.binary.ruby punctuation.definition.numeric.base.ruby",
    "constant.numeric.integer.decimal.ruby punctuation.definition.numeric.base.ruby",
    "constant.numeric.float.decimal.ruby",
    "constant.numeric.float.decimal.ruby punctuation.separator.decimal.ruby",
    "constant.numeric.float.decimal.ruby invalid.illegal.numeric.rational.ruby",
    // a rational's and an imaginary's: by base, its text, base mark, `.`, suffix, bad suffix
    "constant.numeric.rational.decimal.ruby",
    "constant.numeric.rational.hexadecimal.ruby",
    "constant.numeric.rational.octal.ruby",
    "constant.numeric.rational.binary.ruby",
    "constant.numeric.imaginary.decimal.ruby",
    "constant.numeric.imaginary.hexadecimal.ruby",
    "constant.numeric.imaginary.octal.ruby",
    "constant.numeric.imaginary.binary.ruby",
    "constant.numeric.rational.decimal.ruby punctuation.definition.numeric.base.ruby",
    "constant.numeric.rational.hexadecimal.ruby punctuation.definition.numeric.base.ruby",
    "constant.numeric.rational.octal.ruby punctuation.definition.numeric.base.ruby",
    "constant.numeric.rational.binary.ruby punctuation.definition.numeric.base.ruby",
    "constant.numeric.imaginary.decimal.ruby punctuation.definition.numeric.base.ruby",
    "constant.numeric.imaginary.hexadecimal.ruby punctuation.definition.numeric.base.ruby",
    "constant.numeric.imaginary.octal.ruby punctuation.definition.numeric.base.ruby",
    "constant.numeric.imaginary.binary.ruby punctuation.definition.numeric.base.ruby",
    "constant.numeric.rational.decimal.ruby punctuation.separator.decimal.ruby",
    "constant.numeric.rational.hexadecimal.ruby punctuation.separator.decimal.ruby",
    "constant.numeric.rational.octal.ruby punctuation.separator.decimal.ruby",
    "constant.numeric.rational.binary.ruby punctuation.separator.decimal.ruby",
    "constant.numeric.imaginary.decimal.ruby punctuation.separator.decimal.ruby",
    "constant.numeric.imaginary.hexadecimal.ruby punctuation.separator.decimal.ruby",
    "constant.numeric.imaginary.octal.ruby punctuation.separator.decimal.ruby",
    "constant.numeric.imaginary.binary.ruby punctuation.separator.decimal.ruby",
    "constant.numeric.rational.decimal.ruby storage.type.numeric.ruby",
    "constant.numeric.rational.hexadecimal.ruby storage.type.numeric.ruby",
    "constant.numeric.rational.octal.ruby storage.type.numeric.ruby",
    "constant.numeric.rational.binary.ruby storage.type.numeric.ruby",
    "constant.numeric.imaginary.decimal.ruby storage.type.numeric.ruby",
    "constant.numeric.imaginary.hexadecimal.ruby storage.type.numeric.ruby",
    "constant.numeric.imaginary.octal.ruby storage.type.numeric.ruby",
    "constant.numeric.imaginary.binary.ruby storage.type.numeric.ruby",
    "constant.numeric.rational.decimal.ruby invalid.illegal.numeric.ruby",
    "constant.numeric.rational.hexadecimal.ruby invalid.illegal.numeric.ruby",
    "constant.numeric.rational.octal.ruby invalid.illegal.numeric.ruby",
    "constant.numeric.rational.binary.ruby invalid.illegal.numeric.ruby",
    "constant.numeric.imaginary.decimal.ruby invalid.illegal.numeric.ruby",
    "constant.numeric.imaginary.hexadecimal.ruby invalid.illegal.numeric.ruby",
    "constant.numeric.imaginary.octal.ruby invalid.illegal.numeric.ruby",
    "constant.numeric.imaginary.binary.ruby invalid.illegal.numeric.ruby",
];

/// A leaf by its text, at compile time.
const fn leaf(s: &str) -> u16 {
    let mut i = 0;
    while i < LEAVES.len() {
        if eq(LEAVES[i].as_bytes(), s.as_bytes()) {
            return i as u16;
        }
        i += 1;
    }
    panic!("no such leaf")
}

const fn eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

const COMMENT: u16 = 1;
const COMMENT_MARK: u16 = 2;
const DOC: u16 = 3;
const DOC_BEGIN: u16 = 4;
const DOC_END: u16 = 5;
/// The first string's text; a string kind's text is `STRING + 3 * kind`, its begin and end the
/// two after it.
const STRING: u16 = 6;
const HEREDOC: u16 = leaf("string.unquoted.heredoc.ruby");
const HEREDOC_BEGIN: u16 = HEREDOC + 1;
const HEREDOC_END: u16 = HEREDOC + 2;
const PROGRAM_BLOCK: u16 = leaf("string.unquoted.program-block.ruby");
const ESCAPE: u16 = leaf("constant.character.escape.ruby");
const BAD_ESCAPE: u16 = leaf("invalid.illegal.escape.ruby");
const PLACEHOLDER: u16 = leaf("constant.other.placeholder.ruby");
const INTERP_BEGIN: u16 = leaf("punctuation.section.interpolation.begin.ruby");
const INTERP_END: u16 = leaf("punctuation.section.interpolation.end.ruby");
const REGEX_TEXT: u16 = leaf("string.regexp.classic.ruby");
const REGEX_FLAGS: u16 = leaf("string.regexp.classic.ruby keyword.other.ruby");
const REGEX_R_TEXT: u16 = leaf("string.regexp.mod-r.ruby");
/// A comment in a regex's extended syntax, `/…/` then `%r…`'s.
const REGEX_COMMENT: u16 = leaf("string.regexp.classic.ruby comment.line.number-sign.regexp");
const SYMBOL: u16 = leaf("constant.other.symbol.ruby");
const SYMBOL_MARK: u16 = SYMBOL + 1;
const CHARACTER: u16 = leaf("constant.character.ruby");
const CHARACTER_MARK: u16 = CHARACTER + 1;
const CHARACTER_BAD: u16 = CHARACTER + 2;
const IVAR: u16 = leaf("variable.other.readwrite.instance.ruby");
const IVAR_MARK: u16 = IVAR + 1;
const CVAR: u16 = IVAR + 2;
const CVAR_MARK: u16 = IVAR + 3;
const GVAR: u16 = IVAR + 4;
const GVAR_MARK: u16 = IVAR + 5;
const GVAR_PRE: u16 = IVAR + 6;
const GVAR_PRE_MARK: u16 = IVAR + 7;
const CONSTANT: u16 = leaf("variable.other.constant.ruby");
const CONSTANT_NAME: u16 = leaf("entity.name.constant.ruby");
const LANGUAGE_VARIABLE: u16 = leaf("variable.language.ruby");
const BUILTIN: u16 = leaf("support.function.builtin.ruby");
const DEF: u16 = leaf("storage.type.function.ruby keyword.declaration.function.ruby");
const CLASS: u16 = leaf("storage.type.class.ruby keyword.declaration.class.ruby");
const MODULE: u16 = leaf("storage.type.namespace.ruby keyword.declaration.namespace.ruby");
const DEPRECATED: u16 = leaf("invalid.deprecated.ruby");
const OP_OTHER: u16 = leaf("keyword.operator.other.ruby");
const OP_AUGMENTED: u16 = leaf("keyword.operator.assignment.augmented.ruby");
const OP_COMPARISON: u16 = leaf("keyword.operator.comparison.ruby");
const OP_ARITHMETIC: u16 = leaf("keyword.operator.arithmetic.ruby");
const OP_ASSIGNMENT: u16 = leaf("keyword.operator.assignment.ruby");
const OP_LOGICAL: u16 = leaf("keyword.operator.logical.ruby");
const OP_BITWISE: u16 = leaf("keyword.operator.bitwise.ruby");
const OP_CONDITIONAL: u16 = leaf("keyword.operator.conditional.ruby");
const OP_RANGE: u16 = leaf("keyword.operator.range.ruby");
const KEY_VALUE: u16 = leaf("punctuation.separator.key-value.ruby");
const SEQUENCE: u16 = leaf("punctuation.separator.sequence.ruby");
const TERMINATOR: u16 = leaf("punctuation.terminator.statement.ruby");
const ARRAY: u16 = leaf("punctuation.section.array.ruby");
const GROUP_BEGIN: u16 = leaf("punctuation.definition.group.begin.ruby");
const GROUP_END: u16 = leaf("punctuation.definition.group.end.ruby");
const SCOPE: u16 = leaf("punctuation.section.scope.ruby");
const DOT: u16 = leaf("punctuation.accessor.dot.ruby");
const DOUBLE_COLON: u16 = leaf("punctuation.accessor.double-colon.ruby");
const SEPARATOR: u16 = leaf("punctuation.separator.ruby");
const INHERITANCE: u16 = leaf("punctuation.separator.inheritance.ruby");
const PARAMS_BEGIN: u16 = leaf("punctuation.definition.parameters.begin.ruby");
const PARAMS_END: u16 = leaf("punctuation.definition.parameters.end.ruby");
const FUNCTION_NAME: u16 = leaf("entity.name.function.ruby");
const CLASS_NAME: u16 = leaf("entity.name.class.ruby");
const NAMESPACE_NAME: u16 = leaf("entity.name.namespace.ruby");
const PARAMETER: u16 = leaf("variable.parameter.ruby");
const SPLAT: u16 = leaf("keyword.operator.splat.ruby");
const OPERATOR: u16 = leaf("keyword.operator.ruby");
const BARE_ASSIGNMENT: u16 = leaf("punctuation.operator.assignment.ruby");
const DECIMAL: u16 = leaf("constant.numeric.integer.decimal.ruby");
const HEX: u16 = leaf("constant.numeric.integer.hexadecimal.ruby");
const OCTAL: u16 = leaf("constant.numeric.integer.octal.ruby");
const BINARY: u16 = leaf("constant.numeric.integer.binary.ruby");
const DECIMAL_BASE: u16 =
    leaf("constant.numeric.integer.decimal.ruby punctuation.definition.numeric.base.ruby");
const FLOAT: u16 = leaf("constant.numeric.float.decimal.ruby");
const FLOAT_DOT: u16 = FLOAT + 1;
const FLOAT_BAD: u16 = FLOAT + 2;
/// A rational's or an imaginary's text by base: `+ 0..4` rational (decimal, hex, octal,
/// binary), `+ 4..8` imaginary; then its base mark at `+ 8`, `.` at `+ 16`, suffix at `+ 24`,
/// bad suffix at `+ 32`.
const RATIONAL: u16 = leaf("constant.numeric.rational.decimal.ruby");

/// The scopes under `source.ruby` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    if kind >= RE_BASE {
        let k = kind - RE_BASE;
        let outer = if k >= RE_KINDS {
            "string.regexp.mod-r.ruby"
        } else {
            "string.regexp.classic.ruby"
        };
        let mut out = vec![outer];
        out.extend(crate::regex::scopes(k % RE_KINDS));
        return out;
    }
    if kind >= BASH_BASE {
        return crate::bash::scopes(kind - BASH_BASE);
    }
    if kind >= SQL_BASE {
        return crate::sql::scopes(kind - SQL_BASE);
    }
    LEAVES.get(usize::from(kind)).map_or_else(Vec::new, |l| {
        l.split(' ').filter(|s| !s.is_empty()).collect()
    })
}

// A byte's action ([`ACT`]).
const A_OTHER: u8 = 0;
const A_SPACE: u8 = 1;
/// `[a-z_]`: a name.
const A_NAME: u8 = 2;
/// `[A-Z]`: a constant's name.
const A_UPPER: u8 = 3;
const A_DIGIT: u8 = 4;
const A_DQ: u8 = 5;
const A_SQ: u8 = 6;
const A_BACKTICK: u8 = 7;
const A_HASH: u8 = 8;
const A_COLON: u8 = 9;
const A_AT: u8 = 10;
const A_DOLLAR: u8 = 11;
const A_PERCENT: u8 = 12;
const A_SLASH: u8 = 13;
/// `<`: a here-document's opener, else an operator.
const A_LT: u8 = 14;
/// `=-+*!&|^~>`: an operator.
const A_OP: u8 = 15;
const A_QUESTION: u8 = 16;
const A_DOT: u8 = 17;
const A_COMMA: u8 = 18;
const A_SEMI: u8 = 19;
const A_LPAREN: u8 = 20;
const A_RPAREN: u8 = 21;
const A_LBRACKET: u8 = 22;
const A_RBRACKET: u8 = 23;
const A_LBRACE: u8 = 24;
const A_RBRACE: u8 = 25;
/// A byte past ASCII: a word's character, or another.
const A_WIDE: u8 = 26;
/// `\\`: an escape in a RUBY here-document's code.
const A_BACKSLASH: u8 = 27;

static ACT: [u8; 256] = {
    let mut t = [A_WIDE; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = match b {
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => A_SPACE,
            b'a'..=b'z' | b'_' => A_NAME,
            b'A'..=b'Z' => A_UPPER,
            b'0'..=b'9' => A_DIGIT,
            b'"' => A_DQ,
            b'\'' => A_SQ,
            b'`' => A_BACKTICK,
            b'#' => A_HASH,
            b':' => A_COLON,
            b'@' => A_AT,
            b'$' => A_DOLLAR,
            b'%' => A_PERCENT,
            b'/' => A_SLASH,
            b'<' => A_LT,
            b'=' | b'-' | b'+' | b'*' | b'!' | b'&' | b'|' | b'^' | b'~' | b'>' => A_OP,
            b'?' => A_QUESTION,
            b'.' => A_DOT,
            b',' => A_COMMA,
            b';' => A_SEMI,
            b'(' => A_LPAREN,
            b')' => A_RPAREN,
            b'[' => A_LBRACKET,
            b']' => A_RBRACKET,
            b'{' => A_LBRACE,
            b'}' => A_RBRACE,
            b'\\' => A_BACKSLASH,
            _ => A_OTHER,
        };
        c += 1;
    }
    t
};

// What a word is ([`WORDS`], [`INFO`]): its leaf in the low byte, then flags.
/// A regex may follow it (bat's `after-keyword`, `function-call-arguments`).
const W_REGEX: u16 = 1 << 8;
/// Not a match before `?` or `!` (`(?![?!])`).
const W_NO_QB: u16 = 1 << 9;
/// Not a match before `?`, `!` or `=` (`(?![!?=])`).
const W_NO_QBE: u16 = 1 << 10;
/// One of bat's well-known methods: the same after `.` and `::`.
const W_METHOD: u16 = 1 << 11;
/// What it reads past itself (the high bits).
const W_ACTION: u16 = 0xf << 12;
const W_DO: u16 = 1 << 12;
const W_ELSE: u16 = 2 << 12;
const W_DEF: u16 = 3 << 12;
const W_CLASS: u16 = 4 << 12;
const W_MODULE: u16 = 5 << 12;

const KW: u16 = W_NO_QB;
const KWR: u16 = W_NO_QB | W_REGEX;
const WK: u16 = W_METHOD | W_REGEX;
const WKE: u16 = W_METHOD | W_REGEX | W_NO_QBE;

/// The words bat's grammar names, each with what its first pattern in `expressions`' order
/// makes of it.
const WORD_LIST: &[(&[u8], &str, u16)] = &[
    // constants
    (b"nil", "constant.language.ruby", KW),
    (b"true", "constant.language.ruby", KW),
    (b"false", "constant.language.ruby", KW),
    (b"__FILE__", "variable.language.ruby", KW),
    (b"__LINE__", "variable.language.ruby", KW),
    (b"__ENCODING__", "variable.language.ruby", KW),
    (b"self", "variable.language.ruby", KW),
    // class, module, def, do
    (
        b"class",
        "storage.type.class.ruby keyword.declaration.class.ruby",
        W_CLASS,
    ),
    (
        b"module",
        "storage.type.namespace.ruby keyword.declaration.namespace.ruby",
        W_MODULE,
    ),
    (
        b"def",
        "storage.type.function.ruby keyword.declaration.function.ruby",
        W_DEF,
    ),
    (b"do", "keyword.control.block.do.ruby", W_DO),
    // keywords
    (b"begin", "keyword.control.block.begin.ruby", KWR),
    (b"end", "keyword.control.block.end.ruby", KW),
    (b"case", "keyword.control.conditional.case.ruby", KWR),
    (b"else", "keyword.control.conditional.if.ruby", KWR | W_ELSE),
    (b"elsif", "keyword.control.conditional.elseif.ruby", KWR),
    (b"if", "keyword.control.conditional.if.ruby", KWR),
    (b"then", "keyword.control.conditional.then.ruby", KW),
    (b"unless", "keyword.control.conditional.unless.ruby", KWR),
    (b"when", "keyword.control.conditional.when.ruby", KWR),
    (b"ensure", "keyword.control.exception.ensure.ruby", KWR),
    (b"rescue", "keyword.control.exception.rescue.ruby", KWR),
    (b"for", "keyword.control.loop.for.ruby", KWR),
    (b"until", "keyword.control.loop.until.ruby", KWR),
    (b"while", "keyword.control.loop.while.ruby", KWR),
    (b"break", "keyword.control.flow.break.ruby", KW),
    (b"next", "keyword.control.flow.next.ruby", KW),
    (b"redo", "keyword.control.flow.redo.ruby", KW),
    (b"retry", "keyword.control.flow.retry.ruby", KW),
    (b"return", "keyword.control.flow.return.ruby", KW),
    (b"yield", "keyword.control.flow.yield.ruby", KW),
    (b"alias", "keyword.declaration.alias.ruby", KWR),
    (b"alias_method", "keyword.declaration.alias.ruby", KWR),
    (b"undef", "keyword.declaration.undef.ruby", KWR),
    (b"and", "keyword.operator.logical.ruby", W_REGEX),
    (b"not", "keyword.operator.logical.ruby", W_REGEX),
    (b"or", "keyword.operator.logical.ruby", W_REGEX),
    (b"in", "keyword.operator.logical.ruby", KWR),
    (b"BEGIN", "entity.name.function.prepocessor.ruby", KW),
    (b"END", "entity.name.function.prepocessor.ruby", KW),
    (b"defined?", "support.function.builtin.ruby", W_REGEX),
    (b"block_given?", "support.function.builtin.ruby", W_REGEX),
    (b"super", "support.function.builtin.ruby", KWR),
    // well-known methods
    (
        b"catch",
        "keyword.control.exception.catch.ruby",
        WK | W_NO_QB,
    ),
    (b"fail", "keyword.control.flow.throw.ruby", WK | W_NO_QB),
    (b"raise", "keyword.control.flow.throw.ruby", WK | W_NO_QB),
    (b"throw", "keyword.control.flow.throw.ruby", WK | W_NO_QB),
    (b"loop", "keyword.control.loop.loop.ruby", WK | W_NO_QB),
    (
        b"initialize",
        "keyword.other.special-method.ruby",
        WK | W_NO_QB,
    ),
    (b"new", "keyword.other.special-method.ruby", WK | W_NO_QB),
    (
        b"include",
        "keyword.other.special-method.ruby",
        WK | W_NO_QB,
    ),
    (b"extend", "keyword.other.special-method.ruby", WK | W_NO_QB),
    (
        b"prepend",
        "keyword.other.special-method.ruby",
        WK | W_NO_QB,
    ),
    (
        b"attr_reader",
        "keyword.other.special-method.ruby",
        WK | W_NO_QB,
    ),
    (
        b"attr_writer",
        "keyword.other.special-method.ruby",
        WK | W_NO_QB,
    ),
    (
        b"attr_accessor",
        "keyword.other.special-method.ruby",
        WK | W_NO_QB,
    ),
    (b"attr", "keyword.other.special-method.ruby", WK | W_NO_QB),
    (
        b"module_function",
        "keyword.other.special-method.ruby",
        WK | W_NO_QB,
    ),
    (b"public", "keyword.other.special-method.ruby", WK | W_NO_QB),
    (
        b"protected",
        "keyword.other.special-method.ruby",
        WK | W_NO_QB,
    ),
    (
        b"private",
        "keyword.other.special-method.ruby",
        WK | W_NO_QB,
    ),
    (b"require", "keyword.control.import.ruby", W_METHOD),
    (b"require_relative", "keyword.control.import.ruby", W_METHOD),
    (b"gem", "keyword.control.import.ruby", W_METHOD),
    (b"to_ary", "support.function.builtin.ruby", WKE),
    (b"to_a", "support.function.builtin.ruby", WKE),
    (b"to_c", "support.function.builtin.ruby", WKE),
    (b"to_enum", "support.function.builtin.ruby", WKE),
    (b"to_f", "support.function.builtin.ruby", WKE),
    (b"to_hash", "support.function.builtin.ruby", WKE),
    (b"to_h", "support.function.builtin.ruby", WKE),
    (b"to_int", "support.function.builtin.ruby", WKE),
    (b"to_io", "support.function.builtin.ruby", WKE),
    (b"to_i", "support.function.builtin.ruby", WKE),
    (b"to_proc", "support.function.builtin.ruby", WKE),
    (b"to_r", "support.function.builtin.ruby", WKE),
    (b"to_str", "support.function.builtin.ruby", WKE),
    (b"to_sym", "support.function.builtin.ruby", WKE),
    (b"to_s", "support.function.builtin.ruby", WKE),
    (b"gsub!", "support.function.builtin.ruby", WK),
    (b"sub!", "support.function.builtin.ruby", WK),
    (b"gsub", "support.function.builtin.ruby", WK),
    (b"sub", "support.function.builtin.ruby", WK),
    (b"match?", "support.function.builtin.ruby", WK),
    (b"match", "support.function.builtin.ruby", WK),
    (b"assert_match", "support.function.builtin.ruby", WKE),
    (b"assert_no_match", "support.function.builtin.ruby", WKE),
    (b"index", "support.function.builtin.ruby", WKE),
    (b"rindex", "support.function.builtin.ruby", WKE),
    (b"scan", "support.function.builtin.ruby", WKE),
    (b"eql?", "support.function.builtin.ruby", WK),
    (b"instance_of?", "support.function.builtin.ruby", WK),
    (
        b"instance_variable_defined?",
        "support.function.builtin.ruby",
        WK,
    ),
    (b"is_a?", "support.function.builtin.ruby", WK),
    (b"kind_of?", "support.function.builtin.ruby", WK),
    (b"nil?", "support.function.builtin.ruby", WK),
    (b"respond_to?", "support.function.builtin.ruby", WK),
    (b"respond_to_missing?", "support.function.builtin.ruby", WK),
    (b"tainted?", "support.function.builtin.ruby", WK),
    (b"untrusted?", "support.function.builtin.ruby", WK),
    // Object's (`frozen?` in bat's list is `froze` and `frozen`)
    (b"clone", "support.function.builtin.ruby", WKE),
    (
        b"define_singleton_method",
        "support.function.builtin.ruby",
        WKE,
    ),
    (b"display", "support.function.builtin.ruby", WKE),
    (b"dup", "support.function.builtin.ruby", WKE),
    (b"enum_for", "support.function.builtin.ruby", WKE),
    (b"freeze", "support.function.builtin.ruby", WKE),
    (b"froze", "support.function.builtin.ruby", WKE),
    (b"frozen", "support.function.builtin.ruby", WKE),
    (b"hash", "support.function.builtin.ruby", WKE),
    (b"inspect", "support.function.builtin.ruby", WKE),
    (
        b"instance_variable_get",
        "support.function.builtin.ruby",
        WKE,
    ),
    (
        b"instance_variable_set",
        "support.function.builtin.ruby",
        WKE,
    ),
    (b"instance_variables", "support.function.builtin.ruby", WKE),
    (b"itself", "support.function.builtin.ruby", WKE),
    (b"method", "support.function.builtin.ruby", WKE),
    (b"methods", "support.function.builtin.ruby", WKE),
    (b"object_id", "support.function.builtin.ruby", WKE),
    (b"private_methods", "support.function.builtin.ruby", WKE),
    (b"protected_methods", "support.function.builtin.ruby", WKE),
    (b"public_method", "support.function.builtin.ruby", WKE),
    (b"public_methods", "support.function.builtin.ruby", WKE),
    (b"public_send", "support.function.builtin.ruby", WKE),
    (
        b"remove_instance_variable",
        "support.function.builtin.ruby",
        WKE,
    ),
    (b"send", "support.function.builtin.ruby", WKE),
    (b"singleton_class", "support.function.builtin.ruby", WKE),
    (b"singleton_method", "support.function.builtin.ruby", WKE),
    (b"singleton_methods", "support.function.builtin.ruby", WKE),
    (b"taint", "support.function.builtin.ruby", WKE),
    (b"tap", "support.function.builtin.ruby", WKE),
    (b"trust", "support.function.builtin.ruby", WKE),
    (b"untaint", "support.function.builtin.ruby", WKE),
    (b"untrust", "support.function.builtin.ruby", WKE),
    // Kernel's
    (b"autoload?", "support.function.builtin.ruby", WK),
    (b"iterator?", "support.function.builtin.ruby", WK),
    (b"exit!", "support.function.builtin.ruby", WK),
    (b"Array", "support.function.builtin.ruby", WKE),
    (b"Complex", "support.function.builtin.ruby", WKE),
    (b"Float", "support.function.builtin.ruby", WKE),
    (b"Hash", "support.function.builtin.ruby", WKE),
    (b"Integer", "support.function.builtin.ruby", WKE),
    (b"Rational", "support.function.builtin.ruby", WKE),
    (b"String", "support.function.builtin.ruby", WKE),
    (b"__callee__", "support.function.builtin.ruby", WKE),
    (b"__dir__", "support.function.builtin.ruby", WKE),
    (b"__method__", "support.function.builtin.ruby", WKE),
    (b"abort", "support.function.builtin.ruby", WKE),
    (b"at_exit", "support.function.builtin.ruby", WKE),
    (b"autoload", "support.function.builtin.ruby", WKE),
    (b"binding", "support.function.builtin.ruby", WKE),
    (b"callcc", "support.function.builtin.ruby", WKE),
    (b"caller", "support.function.builtin.ruby", WKE),
    (b"caller_locations", "support.function.builtin.ruby", WKE),
    (b"chomp", "support.function.builtin.ruby", WKE),
    (b"chop", "support.function.builtin.ruby", WKE),
    (b"eval", "support.function.builtin.ruby", WKE),
    (b"exec", "support.function.builtin.ruby", WKE),
    (b"exit", "support.function.builtin.ruby", WKE),
    (b"fork", "support.function.builtin.ruby", WKE),
    (b"format", "support.function.builtin.ruby", WKE),
    (b"gets", "support.function.builtin.ruby", WKE),
    (b"global_variables", "support.function.builtin.ruby", WKE),
    (b"lambda", "support.function.builtin.ruby", WKE),
    (b"load", "support.function.builtin.ruby", WKE),
    (b"local_variables", "support.function.builtin.ruby", WKE),
    (b"open", "support.function.builtin.ruby", WKE),
    (b"p", "support.function.builtin.ruby", WKE),
    (b"print", "support.function.builtin.ruby", WKE),
    (b"printf", "support.function.builtin.ruby", WKE),
    (b"proc", "support.function.builtin.ruby", WKE),
    (b"putc", "support.function.builtin.ruby", WKE),
    (b"puts", "support.function.builtin.ruby", WKE),
    (b"rand", "support.function.builtin.ruby", WKE),
    (b"readline", "support.function.builtin.ruby", WKE),
    (b"readlines", "support.function.builtin.ruby", WKE),
    (b"select", "support.function.builtin.ruby", WKE),
    (b"set_trace_func", "support.function.builtin.ruby", WKE),
    (b"sleep", "support.function.builtin.ruby", WKE),
    (b"spawn", "support.function.builtin.ruby", WKE),
    (b"sprintf", "support.function.builtin.ruby", WKE),
    (b"srand", "support.function.builtin.ruby", WKE),
    (b"syscall", "support.function.builtin.ruby", WKE),
    (b"system", "support.function.builtin.ruby", WKE),
    (b"test", "support.function.builtin.ruby", WKE),
    (b"trace_var", "support.function.builtin.ruby", WKE),
    (b"trap", "support.function.builtin.ruby", WKE),
    (b"untrace_var", "support.function.builtin.ruby", WKE),
    (b"warn", "support.function.builtin.ruby", WKE),
    (
        b"class_variable_defined?",
        "support.function.builtin.ruby",
        WK,
    ),
    (b"const_defined?", "support.function.builtin.ruby", WK),
    (b"include?", "support.function.builtin.ruby", WK),
    (b"instance_methods?", "support.function.builtin.ruby", WK),
    (b"method_defined?", "support.function.builtin.ruby", WK),
    (
        b"private_method_defined?",
        "support.function.builtin.ruby",
        WK,
    ),
    (
        b"protected_method_defined?",
        "support.function.builtin.ruby",
        WK,
    ),
    (
        b"public_method_defined?",
        "support.function.builtin.ruby",
        WK,
    ),
    (b"singleton_class?", "support.function.builtin.ruby", WK),
    // Module's
    (b"ancestors", "support.function.builtin.ruby", WKE),
    (b"append_features", "support.function.builtin.ruby", WKE),
    (b"class_eval", "support.function.builtin.ruby", WKE),
    (b"class_exec", "support.function.builtin.ruby", WKE),
    (b"class_variable_get", "support.function.builtin.ruby", WKE),
    (b"class_variable_set", "support.function.builtin.ruby", WKE),
    (b"class_variables", "support.function.builtin.ruby", WKE),
    (b"const_get", "support.function.builtin.ruby", WKE),
    (b"const_missing", "support.function.builtin.ruby", WKE),
    (b"const_set", "support.function.builtin.ruby", WKE),
    (b"constants", "support.function.builtin.ruby", WKE),
    (b"define_method", "support.function.builtin.ruby", WKE),
    (b"extend_object", "support.function.builtin.ruby", WKE),
    (b"extended", "support.function.builtin.ruby", WKE),
    (b"included", "support.function.builtin.ruby", WKE),
    (b"included_modules", "support.function.builtin.ruby", WKE),
    (b"method_added", "support.function.builtin.ruby", WKE),
    (b"method_removed", "support.function.builtin.ruby", WKE),
    (b"method_undefined", "support.function.builtin.ruby", WKE),
    (b"module_eval", "support.function.builtin.ruby", WKE),
    (b"module_exec", "support.function.builtin.ruby", WKE),
    (b"name", "support.function.builtin.ruby", WKE),
    (b"prepend_features", "support.function.builtin.ruby", WKE),
    (b"prepended", "support.function.builtin.ruby", WKE),
    (
        b"private_class_method",
        "support.function.builtin.ruby",
        WKE,
    ),
    (b"private_constant", "support.function.builtin.ruby", WKE),
    (
        b"private_instance_methods",
        "support.function.builtin.ruby",
        WKE,
    ),
    (
        b"protected_instance_methods",
        "support.function.builtin.ruby",
        WKE,
    ),
    (b"public_class_method", "support.function.builtin.ruby", WKE),
    (b"public_constant", "support.function.builtin.ruby", WKE),
    (
        b"public_instance_method",
        "support.function.builtin.ruby",
        WKE,
    ),
    (
        b"public_instance_methods",
        "support.function.builtin.ruby",
        WKE,
    ),
    (b"refine", "support.function.builtin.ruby", WKE),
    (
        b"remove_class_variable",
        "support.function.builtin.ruby",
        WKE,
    ),
    (b"remove_const", "support.function.builtin.ruby", WKE),
    (b"remove_method", "support.function.builtin.ruby", WKE),
    (b"undef_method", "support.function.builtin.ruby", WKE),
    (b"using", "support.function.builtin.ruby", WKE),
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
static WORDS: Words<1024, N, { text_len(&WORD_TEXT) }> = Words::new(&WORD_TEXT);
/// A word's kind and flags by its id; 0 for none.
static INFO: [u16; N + 1] = {
    let mut t = [0u16; N + 1];
    let mut i = 0;
    while i < N {
        t[i + 1] = leaf(WORD_LIST[i].1) | WORD_LIST[i].2;
        i += 1;
    }
    t
};

// The frames ([`Scan::frames`]), a byte each but a string's two and a regex's three: below
// 0x80 code's, a string's kind byte (0x80 set) above its `aux` byte, a regex's above its
// regex state ([`crate::regex::RUBY_STATE`]) too.
/// A string's interpolation, `#{` … `}`, the string under it.
const F_INTERP: u8 = 1;
/// A `{` … `}` in an interpolation or a default value (bat's `nest-curly-expressions`,
/// `nest-all`).
const F_BRACE: u8 = 2;
/// A method's parameters in parentheses, `def f(` … `)` ([`Scan::params`]).
const F_PARAMS: u8 = 3;
/// A parameter's default value in parentheses: code to the `,` or `)` outside its brackets.
const F_DEFAULT: u8 = 4;
/// A `(` … `)` in a frame's code (bat's `nest-all` in a default value).
const F_PAREN: u8 = 5;
/// A `[` … `]` in a frame's code.
const F_BRACKET: u8 = 6;
/// A destructured parameter, `def f((a, b))` ([`Scan::destruct`]).
const F_DESTRUCT: u8 = 7;
/// A method's parameters without parentheses, to `;`, `#` or the line's end ([`Scan::bare`]).
const F_BARE: u8 = 8;
/// A bare parameter's default value: code to the `,` or `;` outside its brackets, or the
/// line's end.
const F_BARE_DEFAULT: u8 = 9;
/// A block's parameters, `|` … `|` ([`Scan::block_params`]).
const F_BLOCK: u8 = 10;
/// A block parameter's default value: code to the `,` or `|` outside its brackets.
const F_BLOCK_DEFAULT: u8 = 11;
/// After `do` or `{` at a line's end: a later line's `|` opens the block's parameters
/// (bat's `maybe-block-parameters`, which blank lines keep open).
const F_MAYBE_BLOCK: u8 = 12;
/// `ENV[` … `]`.
const F_ENV: u8 = 13;
/// An interpolation in a RUBY here-document's code, `#{` … `}`.
const F_CODE_INTERP: u8 = 14;
/// A SQL here-document's text on SQL's scanner ([`Scan::sql_body`]), over a byte of SQL's
/// context (its top bit: the body interpolates).
const F_SQL: u8 = 15;
/// Where SQL's kinds start among Ruby's (an embedded SQL here-document's).
const SQL_BASE: u16 = 256;
const _: () = assert!(LEAVES.len() <= SQL_BASE as usize);
const _: () = assert!(SQL_BASE as usize + crate::sql::KINDS <= BASH_BASE as usize);
/// Where bash's kinds start (a shell here-document's line on bash's scanner, [`Scan::shell`]).
const BASH_BASE: u16 = 512;
/// The bytes a SQL here-document's own patterns can start on, where it interpolates: `#{`,
/// `#@x`, an escape.
static SQL_YIELDS: [bool; 256] = {
    let mut t = [false; 256];
    t[b'#' as usize] = true;
    t[b'\\' as usize] = true;
    t
};
static SQL_LITERAL: [bool; 256] = [false; 256];
/// Where a regex literal's tokens' kinds start (on [`crate::regex`], Ruby's dialect): a `/…/`
/// literal's [`RE_KINDS`], then a `%r…` literal's.
const RE_BASE: u16 = 0x600;
const RE_KINDS: u16 = crate::regex::KINDS as u16;
const _: () = assert!(BASH_BASE + 2 * crate::bash::TWIN <= RE_BASE);
/// A regex literal's body, the line cut at the host's next place (its closing delimiter, an
/// interpolation): `/…/`'s, then `%r…`'s, each plain and in the extended syntax (`#` outside a
/// class a comment).
static RE_EMBEDS: [crate::regex::Embed<'static>; 4] = [
    re_embed(&NO_STOPS, RE_BASE, REGEX_TEXT),
    re_embed(&HASH_STOPS, RE_BASE, REGEX_TEXT),
    re_embed(&NO_STOPS, RE_BASE + RE_KINDS, REGEX_R_TEXT),
    re_embed(&HASH_STOPS, RE_BASE + RE_KINDS, REGEX_R_TEXT),
];
static NO_STOPS: [bool; 256] = crate::regex::table(b"");
static HASH_STOPS: [bool; 256] = crate::regex::table(b"#");

const fn re_embed(
    yields: &'static [bool; 256],
    base: u16,
    text: u16,
) -> crate::regex::Embed<'static> {
    crate::regex::Embed {
        dialect: crate::regex::Dialect::Ruby,
        cooked: false,
        yields,
        stop: |_, _, _| false,
        arg: 0,
        escape: crate::regex::java_escape,
        top_only: false,
        continues: false,
        base,
        text,
    }
}
/// The bytes [`Scan::frames`] holds in a line.
const FRAMES: usize = 64;
/// The frames a [`State`] keeps between lines, the innermost.
const KEPT: usize = 20;

// A string's kind ([`string_byte`]'s low four bits): its leaves are at `STRING + 3 * kind`.
const S_SQ: u8 = 0;
const S_DQ: u8 = 1;
/// `` ` `` and `%x`: `string.interpolated`.
const S_BT: u8 = 2;
const S_SYM_SQ: u8 = 3;
const S_SYM_DQ: u8 = 4;
/// `%q` `%w` `%s` `%i` with a bracket: its delimiter and `\\` escapes.
const S_LOWER: u8 = 5;
/// `%q` `%w` `%s` `%i` with another delimiter: `\.` passed over, no escapes.
const S_LOWER_ANY: u8 = 6;
const S_UPPER: u8 = 7;
const S_OTHER: u8 = 8;
/// `%{` and `%Q{`: `string.quoted.double.ruby.mod`.
const S_MOD: u8 = 9;
const S_REGEX: u8 = 10;
const S_REGEX_R: u8 = 11;
/// A here-document's text, which only its closing line ends ([`Scan::body`]).
const S_HEREDOC: u8 = 12;
/// An HTML, JavaScript, CSS or shell here-document's text: plain, its interpolations and
/// escapes Ruby's.
const S_EMBED: u8 = 13;

// A string's delimiter ([`string_byte`]'s bits 4–6): one of the kind's, a bracket's (`aux`
// its nesting), or another character (`aux` the character).
const D_KIND: u8 = 0;
const D_PAREN: u8 = 1;
const D_BRACKET: u8 = 2;
const D_BRACE: u8 = 3;
const D_ANGLE: u8 = 4;
const D_CHAR: u8 = 5;

#[inline(always)]
const fn string_byte(kind: u8, delim: u8) -> u8 {
    0x80 | delim << 4 | kind
}

/// Whether a string kind takes interpolations and escapes.
#[inline(always)]
fn interpolates(kind: u8) -> bool {
    !matches!(kind, S_SQ | S_SYM_SQ | S_LOWER | S_LOWER_ANY)
}

// What a line starts in ([`Scan::mode`]).
const M_CODE: u8 = 0;
/// `=begin` … `=end`.
const M_DOC: u8 = 1;
/// A here-document's body: the queue's first.
const M_BODY: u8 = 2;
/// After `__END__`: text to the end.
const M_DATA: u8 = 3;

// A here-document's flags in the queue: the body's language in the low three bits, an
// indented closer in the fourth. Whether it interpolates is [`Scan::literal`]'s bit.
const H_PLAIN: u8 = 0;
const H_RUBY: u8 = 1;
const H_CSS: u8 = 2;
const H_JS: u8 = 3;
const H_HTML: u8 = 4;
const H_SHELL: u8 = 5;
const H_SQL: u8 = 6;
const H_INDENTED: u8 = 8;
/// The here-documents a line may open that wait for their bodies.
const QUEUED: usize = 2;

/// Lex `b`, a line of Ruby without its `\n`, from `state`.
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

/// A block of Ruby on one scan opened from `state` ([`crate::Carry`]).
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
    /// What each open frame is, the innermost last ([`F_INTERP`], a string's two bytes…).
    frames: [u8; FRAMES],
    /// How many of [`Scan::frames`]' bytes are open.
    depth: u8,
    exact: bool,
    /// What the line starts in ([`M_CODE`]…).
    mode: u8,
    /// The here-documents waiting for their bodies, in their openers' order.
    queue: Queue<QUEUED>,
    /// Which of [`Scan::queue`]'s here-documents don't interpolate (`<<~'EOS'`), a bit each.
    literal: u8,
    /// The frames open where the bodies started: what a body opens ends at its closer.
    base: u8,
    /// Whether the line is a RUBY here-document's that interpolates: its `#{` and escapes are
    /// the body's (bat's `interpolated-heredoc-ruby`, `escaped-char`).
    ruby_interp: bool,
    /// Where a regex may start on the line: the end of the last token after which bat's grammar
    /// tries one (an operator, a separator, a keyword), the line's start, or past the line.
    regex_at: usize,
    /// Where the line's last `/` is ([`usize::MAX`] until it's looked for).
    last_slash: usize,
    /// The last constant list [`Scan::assigned`] read on the line: where it ended, and whether
    /// it was assigned; it holds for the constants between [`Scan::chain_from`] and its end.
    chain: (usize, bool),
    chain_from: usize,
    /// Whether the open regex is in the extended syntax ([`extended`]), once looked for on the
    /// line: 0 not yet, 1 no, 2 yes. A regex opened or closed looks again, so a body resumed
    /// after each of a line's interpolations doesn't read on to the line's end each time.
    re_x: u8,
    k: K,
    out: &'b mut Out<'c>,
}

impl<'b, 'c, K: Sink> Scan<'b, 'c, K> {
    #[inline(always)]
    fn new(b: &'b [u8], out: &'b mut Out<'c>, k: K) -> Self {
        Scan {
            b,
            frames: [0; FRAMES],
            depth: 0,
            exact: true,
            mode: M_CODE,
            queue: Queue::new(),
            literal: 0,
            base: 0,
            ruby_interp: false,
            regex_at: 0,
            last_slash: usize::MAX,
            chain: (0, false),
            chain_from: 0,
            re_x: 0,
            k,
            out,
        }
    }

    /// The scan's fields from `state`: the innermost [`KEPT`] frames in its levels and first
    /// withs, the queue in the next eight withs, the mode, the queue's length and its literal
    /// bits in [`State::arg`].
    #[inline(always)]
    fn load(&mut self, state: &State) {
        let d = usize::from(state.depth).min(KEPT);
        if d > 0 {
            self.frames[..14].copy_from_slice(&state.levels);
            self.frames[14..KEPT].copy_from_slice(&state.withs[..KEPT - 14]);
        }
        self.depth = d as u8;
        self.exact = !state.inexact;
        let arg = state.arg;
        self.mode = (arg & 3) as u8;
        let queued = usize::from(arg >> 2 & 3);
        if queued > 0 {
            self.queue = Queue::load(&state.withs[6..14], queued);
            self.literal = (arg >> 4 & 3) as u8;
            self.base = ((arg >> 6) as u8 & 31).min(self.depth);
        }
    }

    /// The state the scan's last line left: every field as it is, so a block lexes as its
    /// lines do one at a time.
    #[inline(always)]
    fn state(&self) -> State {
        let mut s = State::default();
        let mut d = usize::from(self.depth);
        let mut exact = self.exact;
        let mut base = self.base.min(self.depth);
        if d > 0 {
            let from = d.saturating_sub(KEPT);
            base = base.saturating_sub(from as u8);
            if from > 0 {
                exact = false;
                d = KEPT;
            }
            let mut w = [0u8; KEPT];
            w[..d].copy_from_slice(&self.frames[from..from + d]);
            s.levels.copy_from_slice(&w[..14]);
            s.withs[..KEPT - 14].copy_from_slice(&w[14..]);
        }
        s.depth = d as u8;
        s.inexact = !exact;
        let mut arg = u16::from(self.mode);
        if !self.queue.is_empty() {
            self.queue.save(&mut s.withs[6..14]);
            arg |= (self.queue.len() as u16) << 2
                | u16::from(self.literal & 3) << 4
                | u16::from(base.min(31)) << 6;
        }
        s.arg = arg;
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
        if usize::from(self.depth) > KEPT {
            self.settle();
        }
        // as `state()` and `load` keep it: a body's base never above the frames open
        self.base = self.base.min(self.depth);
    }

    /// The frames past [`KEPT`] dropped, the outermost, as [`Scan::state`] drops them: a block
    /// lexes as its lines do one at a time.
    #[cold]
    fn settle(&mut self) {
        let d = usize::from(self.depth);
        let from = d - KEPT;
        self.frames.copy_within(from..d, 0);
        self.depth = KEPT as u8;
        self.exact = false;
        self.base = self.base.saturating_sub(from as u8);
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
        if self.depth == 0 {
            0
        } else {
            self.frames[usize::from(self.depth) - 1]
        }
    }

    #[inline(always)]
    fn push(&mut self, f: u8) {
        if usize::from(self.depth) < FRAMES {
            self.frames[usize::from(self.depth)] = f;
            self.depth += 1;
        } else {
            self.exact = false;
        }
    }

    #[inline(always)]
    fn pop(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// A string's two bytes pushed: `aux` under its kind byte.
    #[inline(always)]
    fn push_string(&mut self, byte: u8, aux: u8) {
        if usize::from(self.depth) + 2 <= FRAMES {
            self.frames[usize::from(self.depth)] = aux;
            self.frames[usize::from(self.depth) + 1] = byte;
            self.depth += 2;
        } else {
            self.exact = false;
        }
    }

    /// A regex literal's three bytes pushed: its regex state (none yet) under `aux` under its
    /// kind byte.
    #[inline(always)]
    fn push_regex(&mut self, byte: u8, aux: u8) {
        let d = usize::from(self.depth);
        if d + 3 <= FRAMES {
            self.frames[d] = 0;
            self.frames[d + 1] = aux;
            self.frames[d + 2] = byte;
            self.re_x = 0;
            self.depth += 3;
        } else {
            self.exact = false;
        }
    }

    /// The open string's kind byte and `aux`.
    #[inline(always)]
    fn string_top(&self) -> (u8, u8) {
        let d = usize::from(self.depth);
        (
            self.frames[d - 1],
            if d >= 2 { self.frames[d - 2] } else { 0 },
        )
    }

    /// The line, from what the line before left.
    #[inline(always)]
    fn lex_line<const SHARED: bool>(&mut self) {
        self.regex_at = 0;
        self.last_slash = usize::MAX;
        self.chain = (0, false);
        self.ruby_interp = false;
        self.re_x = 0;
        let p = match self.mode {
            M_CODE => 0,
            M_BODY => {
                if !self.body() {
                    return;
                }
                if self.depth <= self.base {
                    // the frames under the body are the opener line's: code goes on after it
                    if SHARED {
                        self.code_shared(0);
                    } else {
                        self.code(0);
                    }
                    self.line_end();
                    return;
                }
                0
            }
            M_DOC => {
                let b = self.b;
                if b.starts_with(b"=end") {
                    self.put(0, 4, DOC_END);
                    self.mode = M_CODE;
                    self.regex_at = usize::MAX;
                    4
                } else {
                    self.put(0, b.len(), DOC);
                    return;
                }
            }
            _ => return,
        };
        let p = if p > 0 {
            p
        } else if self.depth > 0 && self.top() < 0x80 {
            self.resume(0)
        } else if self.top() >= 0x80 {
            self.regex_at = usize::MAX;
            self.string(0)
        } else if self.b.first() == Some(&b'=') && self.b.starts_with(b"=begin") {
            self.put(0, 6, DOC_BEGIN);
            self.put(6, self.b.len(), DOC);
            self.mode = M_DOC;
            return;
        } else if self.b == b"__END__" {
            self.put(0, 7, PROGRAM_BLOCK);
            self.mode = M_DATA;
            return;
        } else {
            0
        };
        if SHARED {
            self.code_shared(p);
        } else {
            self.code(p);
        }
        self.line_end();
    }

    /// The line's end: bare parameters end with it, here-documents opened on it start their
    /// bodies.
    #[inline(always)]
    fn line_end(&mut self) {
        if self.depth > 0 {
            self.bare_end();
        }
        if !self.queue.is_empty() && self.mode == M_CODE {
            self.mode = M_BODY;
            self.base = self.depth;
        }
    }

    /// [`Scan::code`] out of line: on the run sink, one copy that the line API and a block's
    /// lines share.
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
            p = match ACT[usize::from(c)] {
                A_SPACE => {
                    let mut e = p + 1;
                    while e < n && ACT[usize::from(b[e])] == A_SPACE {
                        e += 1;
                    }
                    e
                }
                A_NAME => self.name(p),
                A_UPPER => self.constant(p),
                A_DIGIT => self.number(p),
                A_DQ => self.open(p, p + 1, string_byte(S_DQ, D_KIND), 0),
                A_SQ => self.open(p, p + 1, string_byte(S_SQ, D_KIND), 0),
                A_BACKTICK => self.open(p, p + 1, string_byte(S_BT, D_KIND), 0),
                A_HASH if self.ruby_interp && matches!(b.get(p + 1), Some(b'{' | b'@' | b'$')) => {
                    self.body_interp(p)
                }
                A_BACKSLASH if self.ruby_interp => self.escape(p),
                A_HASH => {
                    let mut e = p + 1;
                    while e < n && b[e] == b'#' {
                        e += 1;
                    }
                    self.put_hot(p, e, COMMENT_MARK);
                    self.put_hot(e, n, COMMENT);
                    n
                }
                A_COLON => self.colon(p),
                A_AT => self.at(p),
                A_DOLLAR => self.dollar(p),
                A_PERCENT => self.percent(p),
                A_SLASH => self.slash(p),
                A_LT => self.lt(p),
                A_OP => self.op(p),
                A_QUESTION => self.question(p),
                A_DOT => self.dot(p),
                A_COMMA => {
                    if self.depth > 0 && self.ends_default(p) {
                        self.resume(p)
                    } else {
                        self.put_hot(p, p + 1, SEQUENCE);
                        self.regex_at = p + 1;
                        p + 1
                    }
                }
                A_SEMI => {
                    if self.depth > 0 {
                        self.bare_end();
                    }
                    self.put(p, p + 1, TERMINATOR);
                    self.regex_at = p + 1;
                    p + 1
                }
                A_LPAREN => {
                    if self.depth > 0 {
                        self.nest(F_PAREN);
                    }
                    self.put_hot(p, p + 1, GROUP_BEGIN);
                    self.regex_at = p + 1;
                    p + 1
                }
                A_RPAREN => {
                    if self.depth > 0 {
                        self.close(p, F_PAREN)
                    } else {
                        self.put_hot(p, p + 1, GROUP_END);
                        p + 1
                    }
                }
                A_LBRACKET => {
                    if self.depth > 0 {
                        self.nest(F_BRACKET);
                    }
                    self.put_hot(p, p + 1, ARRAY);
                    self.regex_at = p + 1;
                    p + 1
                }
                A_RBRACKET => {
                    if self.depth > 0 {
                        self.close(p, F_BRACKET)
                    } else {
                        self.put_hot(p, p + 1, ARRAY);
                        p + 1
                    }
                }
                A_LBRACE => {
                    if self.depth > 0 {
                        self.push(F_BRACE);
                    }
                    self.put_hot(p, p + 1, SCOPE);
                    self.block_open(p + 1)
                }
                A_RBRACE => self.rbrace(p),
                A_WIDE => {
                    if is_word_starting(b, p) {
                        self.name(p)
                    } else {
                        p + char_len(c)
                    }
                }
                _ => p + 1,
            };
        }
    }

    /// A shell here-document's line on bash's scanner from a fresh state (bat's carries bash's
    /// contexts across the body; a body's line seldom leaves one open), its kinds from
    /// [`BASH_BASE`]: bash's run copy read back as spans (`bash::line_spans`, Crontab's), which
    /// ada holds already. `crate::lex` would put every language's span copy in ada (+1.8 MB).
    #[cold]
    #[inline(never)]
    fn shell(&mut self) {
        FOREIGN.with_borrow_mut(|spans| {
            spans.clear();
            crate::bash::line_spans(self.b, &mut State::default(), &mut Out::new(spans));
            for sp in spans.iter() {
                self.put(sp.start as usize, sp.end as usize, BASH_BASE + sp.kind);
            }
        });
    }

    /// A SQL here-document's text from `p` (its frame on top) on SQL's scanner, embedded: the
    /// body's own patterns first at each place (an interpolation, a variable's, an escape, where
    /// it interpolates), as bat's `heredoc-sql-*` has them before `scope:source.sql`. Where the
    /// code goes on: an interpolation's, or the line's end.
    #[inline(never)]
    fn sql_body(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let d = usize::from(self.depth);
        if d < 2 {
            // its context byte cut off with the frames past those kept
            self.pop();
            return p;
        }
        let interp = self.frames[d - 2] & 0x80 != 0;
        let host = crate::sql::Embed {
            yields: if interp { &SQL_YIELDS } else { &SQL_LITERAL },
            base: SQL_BASE,
            text: 0,
        };
        let mut keep = crate::sql::Embedded {
            is_end: usize::MAX,
            at: p,
        };
        while p < n {
            if interp {
                match b[p] {
                    b'#' if b.get(p + 1) == Some(&b'{') => {
                        self.put(p, p + 2, INTERP_BEGIN);
                        self.push(F_INTERP);
                        self.regex_at = p + 2;
                        return p + 2;
                    }
                    b'#' if matches!(b.get(p + 1), Some(b'@' | b'$')) => {
                        if let Some(e) = self.interp_var(p) {
                            p = e;
                            keep.at = p;
                            continue;
                        }
                    }
                    b'\\' => {
                        p = self.escape(p);
                        keep.at = p;
                        continue;
                    }
                    _ => {}
                }
            }
            let ctx = self.frames[d - 2] & 0x7f;
            self.k.close(self.out);
            let r = crate::sql::embedded(b, p, ctx, &host, &mut keep, self.out);
            self.k.reopen(self.out, 16);
            let Some((q, c)) = r else {
                break;
            };
            self.frames[d - 2] = c | self.frames[d - 2] & 0x80;
            // a place that isn't the body's (a `#` that isn't an interpolation) is SQL's next
            p = q.max(p + 1);
        }
        // SQL's line comment ends with its line
        let ctx = self.frames[d - 2] & 0x7f;
        if ctx == crate::sql::DASHES || ctx == crate::sql::HASHES {
            self.frames[d - 2] = crate::sql::MAIN | self.frames[d - 2] & 0x80;
        }
        n
    }

    /// `#` at `p` before `{`, `@` or `$` in a RUBY here-document that interpolates: an
    /// interpolation, a variable's, else a comment.
    #[inline(never)]
    fn body_interp(&mut self, p: usize) -> usize {
        let b = self.b;
        if b[p + 1] == b'{' {
            self.put(p, p + 2, INTERP_BEGIN);
            self.push(F_CODE_INTERP);
            self.regex_at = p + 2;
            return p + 2;
        }
        if let Some(e) = self.interp_var(p) {
            return e;
        }
        let n = b.len();
        let mut e = p + 1;
        while e < n && b[e] == b'#' {
            e += 1;
        }
        self.put(p, e, COMMENT_MARK);
        self.put(e, n, COMMENT);
        n
    }

    /// `}` at `p`: an interpolation's end, its string going on, or a brace's.
    #[inline(always)]
    fn rbrace(&mut self, p: usize) -> usize {
        match self.top() {
            F_CODE_INTERP => {
                self.pop();
                self.put(p, p + 1, INTERP_END);
                p + 1
            }
            F_INTERP => {
                self.pop();
                self.put(p, p + 1, INTERP_END);
                match self.top() {
                    F_SQL => self.sql_body(p + 1),
                    // the string under it cut with the frames past those kept: code goes on
                    t if t < 0x80 => p + 1,
                    _ => self.string(p + 1),
                }
            }
            F_BRACE => {
                self.pop();
                self.put_hot(p, p + 1, SCOPE);
                p + 1
            }
            _ => {
                self.put_hot(p, p + 1, SCOPE);
                p + 1
            }
        }
    }

    /// `(` or `[` at a frame's top: a frame of its own where a default value's end is looked
    /// for (bat's `nest-all`), not in an interpolation or `ENV[`.
    #[inline(always)]
    fn nest(&mut self, f: u8) {
        if !matches!(self.top(), F_INTERP | F_ENV) {
            self.push(f);
        }
    }

    /// `)` or `]` at `p` with frames open: its bracket's frame closed, a default value's end
    /// (`)`), `ENV[`'s end (`]`, plain).
    #[inline(never)]
    fn close(&mut self, p: usize, f: u8) -> usize {
        let top = self.top();
        if top == f {
            self.pop();
        } else if f == F_PAREN && top == F_DEFAULT {
            self.pop();
            return self.resume(p);
        } else if f == F_BRACKET && top == F_ENV {
            self.pop();
            return p + 1;
        }
        self.put_hot(p, p + 1, if f == F_PAREN { GROUP_END } else { ARRAY });
        p + 1
    }

    /// Whether `,` at `p` ends a default value (its frame popped).
    #[inline(always)]
    fn ends_default(&mut self, _p: usize) -> bool {
        if matches!(self.top(), F_DEFAULT | F_BARE_DEFAULT | F_BLOCK_DEFAULT) {
            self.pop();
            true
        } else {
            false
        }
    }

    /// Bare parameters' frames ended (`;`, the line's end): down to their [`F_BARE`], with
    /// what a default value left open above it.
    #[inline(never)]
    fn bare_end(&mut self) {
        let d = usize::from(self.depth);
        let mut i = d;
        while i > 0 {
            match self.frames[i - 1] {
                F_BARE => {
                    self.depth = (i - 1) as u8;
                    return;
                }
                F_BARE_DEFAULT | F_PAREN | F_BRACKET | F_BRACE => i -= 1,
                _ => return,
            }
        }
    }

    /// Where code goes on at `p` with a frame on top that is a loop of its own: a method's or
    /// a block's parameters, a destructured one, after `do` at a line's end.
    #[inline(never)]
    fn resume(&mut self, p: usize) -> usize {
        match self.top() {
            F_PARAMS => self.params(p),
            F_DESTRUCT => {
                let q = self.destruct(p);
                if self.top() == F_PARAMS {
                    self.params(q)
                } else {
                    q
                }
            }
            F_BARE => self.bare(p),
            F_BLOCK => self.block_params(p),
            F_SQL => self.sql_body(p),
            F_MAYBE_BLOCK => {
                let b = self.b;
                let s = skip_blanks(b, p);
                if s == b.len() {
                    return s;
                }
                self.pop();
                if b[s] == b'|' {
                    self.put(s, s + 1, PARAMS_BEGIN);
                    self.push(F_BLOCK);
                    return self.block_params(s + 1);
                }
                s
            }
            _ => p,
        }
    }

    /// After `do` or `{` at `p`: bat's `maybe-block-parameters`, a `|` after blanks opening the
    /// block's parameters, a line's end leaving it to the next line.
    #[inline(always)]
    fn block_open(&mut self, p: usize) -> usize {
        let b = self.b;
        match b.get(p) {
            Some(&c) if !is_blank(c) && c != b'|' => p,
            _ => self.block_open_slow(p),
        }
    }

    #[inline(never)]
    fn block_open_slow(&mut self, p: usize) -> usize {
        let b = self.b;
        let s = skip_blanks(b, p);
        if s == b.len() {
            self.push(F_MAYBE_BLOCK);
            return s;
        }
        if b[s] == b'|' {
            self.put(s, s + 1, PARAMS_BEGIN);
            self.push(F_BLOCK);
            return self.block_params(s + 1);
        }
        p
    }

    /// A block's parameters from `p`: names, `,`, `*`, `&`, a default value after `=`, to
    /// the closing `|`; any other byte plain (bat's context takes only these).
    #[inline(never)]
    fn block_params(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        while p < n {
            let c = b[p];
            match c {
                b'|' => {
                    self.put(p, p + 1, PARAMS_END);
                    self.pop();
                    self.regex_at = p + 1;
                    return p + 1;
                }
                b',' => {
                    self.put(p, p + 1, SEPARATOR);
                    p += 1;
                }
                b'*' => {
                    self.put(p, p + 1, SPLAT);
                    p += 1;
                }
                b'&' => {
                    self.put(p, p + 1, OPERATOR);
                    p += 1;
                }
                b'=' => {
                    self.put(p, p + 1, OP_ASSIGNMENT);
                    self.push(F_BLOCK_DEFAULT);
                    self.regex_at = usize::MAX;
                    return p + 1;
                }
                _ if is_name_start(b, p) => {
                    let e = word_end(b, p);
                    self.put(p, e, PARAMETER);
                    p = e;
                }
                _ => p += char_len(c),
            }
        }
        n
    }

    /// A method's parameters in parentheses from `p`: names, `,`, `*`, `&`, comments, a
    /// destructured one, a default value after `=` or `:`, to the closing `)`.
    #[inline(never)]
    fn params(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        while p < n {
            let c = b[p];
            match c {
                b')' => {
                    self.put(p, p + 1, GROUP_END);
                    self.pop();
                    return p + 1;
                }
                b',' => {
                    self.put(p, p + 1, SEPARATOR);
                    p += 1;
                }
                b'*' => {
                    self.put(p, p + 1, SPLAT);
                    p += 1;
                }
                b'&' => {
                    self.put(p, p + 1, OPERATOR);
                    p += 1;
                }
                b'#' => {
                    let mut e = p + 1;
                    while e < n && b[e] == b'#' {
                        e += 1;
                    }
                    self.put(p, e, COMMENT_MARK);
                    self.put(e, n, COMMENT);
                    return n;
                }
                b'(' => {
                    self.put(p, p + 1, GROUP_BEGIN);
                    self.push(F_DESTRUCT);
                    p = self.destruct(p + 1);
                    if self.top() == F_DESTRUCT {
                        return n;
                    }
                }
                b'=' | b':' => {
                    self.put(p, p + 1, if c == b'=' { OP_ASSIGNMENT } else { SEPARATOR });
                    self.push(F_DEFAULT);
                    self.regex_at = usize::MAX;
                    return p + 1;
                }
                _ if is_name_start(b, p) => {
                    let e = word_end(b, p);
                    self.put(p, e, PARAMETER);
                    p = e;
                }
                _ => p += char_len(c),
            }
        }
        n
    }

    /// A destructured parameter from `p`: names, `,`, `*`, to its `)` (its frame popped).
    #[inline(never)]
    fn destruct(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        while p < n {
            let c = b[p];
            match c {
                b')' => {
                    self.put(p, p + 1, GROUP_END);
                    self.pop();
                    return p + 1;
                }
                b',' => {
                    self.put(p, p + 1, SEPARATOR);
                    p += 1;
                }
                b'*' => {
                    self.put(p, p + 1, SPLAT);
                    p += 1;
                }
                _ if is_name_start(b, p) => {
                    let e = word_end(b, p);
                    self.put(p, e, PARAMETER);
                    p = e;
                }
                _ => p += char_len(c),
            }
        }
        n
    }

    /// A method's parameters without parentheses from `p`: names, `,`, `*`, `&`, a default
    /// value after `=` or `:`, to `;`, `#` or the line's end.
    #[inline(never)]
    fn bare(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        while p < n {
            let c = b[p];
            match c {
                b';' | b'#' => {
                    self.pop();
                    return p;
                }
                b',' => {
                    self.put(p, p + 1, SEPARATOR);
                    p += 1;
                }
                b'*' => {
                    self.put(p, p + 1, SPLAT);
                    p += 1;
                }
                b'&' => {
                    self.put(p, p + 1, OPERATOR);
                    p += 1;
                }
                b'=' | b':' => {
                    self.put(
                        p,
                        p + 1,
                        if c == b'=' {
                            BARE_ASSIGNMENT
                        } else {
                            SEPARATOR
                        },
                    );
                    self.push(F_BARE_DEFAULT);
                    self.regex_at = usize::MAX;
                    return p + 1;
                }
                _ if is_name_start(b, p) => {
                    let e = word_end(b, p);
                    self.put(p, e, PARAMETER);
                    p = e;
                }
                _ => p += char_len(c),
            }
        }
        self.pop();
        n
    }

    /// `def` at `p..e`: the method's name (`self.name`, an operator's, `A::name`, `a.name`), then
    /// its parameters; `def` alone where none follows.
    #[inline(never)]
    fn def(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let n = b.len();
        self.put(p, e, DEF);
        let s = skip_blanks(b, e);
        if s == n {
            return n;
        }
        let mut ne = 0;
        if b[s..].starts_with(b"self.") {
            let t = s + 5;
            let q = if is_name_start(b, t) {
                method_punct(b, word_end(b, t))
            } else {
                def_op(b, t)
            };
            if q > t {
                self.put(s, s + 4, LANGUAGE_VARIABLE);
                self.put(s + 4, t, DOT);
                self.put(t, q, FUNCTION_NAME);
                ne = q;
            }
        }
        if ne == 0 {
            let q = def_op(b, s);
            if q > s {
                self.put(s, q, FUNCTION_NAME);
                ne = q;
            }
        }
        if ne == 0 && is_name_start(b, s) {
            let w = word_end(b, s);
            let (acc, t) = if b[w..].starts_with(b"::") && is_name_start(b, w + 2) {
                (DOUBLE_COLON, w + 2)
            } else if b.get(w) == Some(&b'.') && is_name_start(b, w + 1) {
                (DOT, w + 1)
            } else {
                (0, s)
            };
            if acc != 0 {
                self.put(s, w, FUNCTION_NAME);
                self.put(w, t, acc);
            }
            ne = method_punct(b, word_end(b, t));
            self.put(t, ne, FUNCTION_NAME);
        }
        if ne == 0 {
            return s;
        }
        // bat's `method-parameters-start`: the first `(`, name or `*`, else `;`, `#`, the end
        let mut q = ne;
        while q < n {
            let c = b[q];
            match c {
                b'(' => {
                    self.put(q, q + 1, GROUP_BEGIN);
                    self.push(F_PARAMS);
                    return self.params(q + 1);
                }
                b';' | b'#' => return q,
                b'*' => {
                    self.push(F_BARE);
                    return self.bare(q);
                }
                _ if is_name_start(b, q) => {
                    self.push(F_BARE);
                    return self.bare(q);
                }
                _ => q += char_len(c),
            }
        }
        n
    }

    /// `class` at `p..e`: `class << self`, or a class's name and what it inherits.
    #[inline(never)]
    fn class(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        self.put(p, e, CLASS);
        let s = skip_blanks(b, e);
        if b.get(s) == Some(&b'<') {
            // `class << self` (and `class <<self`, Ruby's singleton class, where bat opens a
            // here-document)
            if b.get(s + 1) == Some(&b'<') {
                self.put(s, s + 2, OP_AUGMENTED);
                self.regex_at = s + 2;
                return s + 2;
            }
            return e;
        }
        if !path_at(b, s) {
            return s;
        }
        let q = self.name_parts(s, CLASS_NAME);
        let t = skip_blanks(b, q);
        if b.get(t) != Some(&b'<') {
            return q;
        }
        self.put(t, t + 1, INHERITANCE);
        let u = skip_blanks(b, t + 1);
        if path_at(b, u) {
            self.name_parts(u, 0)
        } else {
            t + 1
        }
    }

    /// `module` at `p..e`: its name.
    #[inline(never)]
    fn module(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        self.put(p, e, MODULE);
        let s = skip_blanks(b, e);
        if !path_at(b, s) {
            return s;
        }
        self.name_parts(s, NAMESPACE_NAME)
    }

    /// A class's or a module's name at `p`, bat's `name-parts`: its words `kind`, `::` and
    /// `.` accessors; where it ends.
    fn name_parts(&mut self, mut p: usize, kind: u16) -> usize {
        let b = self.b;
        loop {
            if b[p..].starts_with(b"::") {
                self.put(p, p + 2, DOUBLE_COLON);
                p += 2;
            } else if b.get(p) == Some(&b'.') {
                self.put(p, p + 1, DOT);
                p += 1;
            } else if is_name_start(b, p) {
                let e = word_end(b, p);
                if kind != 0 {
                    self.put(p, e, kind);
                }
                p = e;
                if !(b[p..].starts_with(b"::") || b.get(p) == Some(&b'.')) {
                    return p;
                }
            } else {
                return p;
            }
        }
    }

    /// A name at `p`: a keyword, a well-known method, a hash's key, or a name.
    #[inline(always)]
    fn name(&mut self, p: usize) -> usize {
        let b = self.b;
        let mut e = p + 1;
        while e < b.len() && WORD[usize::from(b[e])] {
            e += 1;
        }
        if e < b.len() && b[e] >= 0x80 {
            e = word_end(b, e);
        }
        let after = b.get(e).copied().unwrap_or(b'\n');
        if matches!(after, b':' | b'?' | b'!') {
            return self.name_suffix(p, e);
        }
        let w = WORDS.value(&b[p..e], &INFO);
        if w == 0 || after == b'=' && w & W_NO_QBE != 0 {
            return self.plain_name(e);
        }
        if w & W_ACTION != 0 {
            return self.word_action(p, e, w);
        }
        self.put_hot(p, e, w & 0xff);
        if w & W_REGEX != 0 {
            self.regex_at = e;
        }
        e
    }

    /// A plain name ending at `e`: its `=` (not `==`, `=>`, `=~`… as bat's `=(?![>=])`
    /// takes `=~` too) is the name's.
    #[inline(always)]
    fn plain_name(&mut self, e: usize) -> usize {
        let b = self.b;
        if b.get(e) == Some(&b'=') && !matches!(b.get(e + 1), Some(b'>' | b'=')) {
            return self.after_name(e + 1);
        }
        e
    }

    /// After a name that ends at `e`: bat's `after-identifier`, a `:` right after it the
    /// ternary's.
    #[inline(always)]
    fn after_name(&mut self, e: usize) -> usize {
        let b = self.b;
        if b.get(e) == Some(&b':') && b.get(e + 1) != Some(&b':') {
            self.put(e, e + 1, OP_CONDITIONAL);
            self.regex_at = e + 1;
            return e + 1;
        }
        e
    }

    /// A name at `p..e` with `:`, `?` or `!` after it: a hash's key (`key:`), a word spelled
    /// with its `?` or `!` (`defined?`, `gsub!`), or a name whose `?` or `!` is its own.
    #[inline(never)]
    fn name_suffix(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let s = b[e];
        if s == b':' {
            if b.get(e + 1) != Some(&b':') {
                self.put(p, e, SYMBOL);
                self.put(e, e + 1, SYMBOL_MARK);
                self.regex_at = e + 1;
                return e + 1;
            }
            let w = WORDS.value(&b[p..e], &INFO);
            if w != 0 {
                if w & W_ACTION != 0 {
                    return self.word_action(p, e, w);
                }
                self.put(p, e, w & 0xff);
            }
            return e;
        }
        if b.get(e + 1) == Some(&b':') && b.get(e + 2) != Some(&b':') {
            self.put(p, e + 1, SYMBOL);
            self.put(e + 1, e + 2, SYMBOL_MARK);
            self.regex_at = e + 2;
            return e + 2;
        }
        let w = WORDS.value(&b[p..e + 1], &INFO);
        if w != 0 {
            self.put(p, e + 1, w & 0xff);
            if w & W_REGEX != 0 {
                self.regex_at = e + 1;
            }
            return e + 1;
        }
        let w = WORDS.value(&b[p..e], &INFO);
        if w != 0 && w & (W_NO_QB | W_NO_QBE) == 0 {
            if w & W_ACTION != 0 {
                return self.word_action(p, e, w);
            }
            self.put(p, e, w & 0xff);
            if w & W_REGEX != 0 {
                self.regex_at = e;
            }
            return e;
        }
        self.after_name(e + 1)
    }

    /// A word `b[p..e]` whose pattern reads past it (`w`'s action).
    #[inline(never)]
    fn word_action(&mut self, p: usize, e: usize, w: u16) -> usize {
        let b = self.b;
        let kind = w & 0xff;
        match w & W_ACTION {
            W_DEF => self.def(p, e),
            W_CLASS => self.class(p, e),
            W_MODULE => self.module(p, e),
            W_DO => {
                self.put(p, e, kind);
                self.block_open(e)
            }
            W_ELSE => {
                let s = skip_blanks(b, e);
                if s > e && b[s..].starts_with(b"if") && !is_word(b, s + 2) {
                    self.put(p, s + 2, DEPRECATED);
                    self.regex_at = s + 2;
                    return s + 2;
                }
                self.put(p, e, kind);
                self.regex_at = e;
                e
            }
            _ => {
                self.put(p, e, kind);
                e
            }
        }
    }

    /// A word at `p` that starts with a capital: a constant, or a well-known method's name.
    #[inline(always)]
    fn constant(&mut self, p: usize) -> usize {
        let b = self.b;
        let e = word_end(b, p);
        let after = b.get(e).copied().unwrap_or(b'\n');
        if matches!(after, b':' | b'?' | b'!') {
            return self.name_suffix(p, e);
        }
        if matches!(after, b' ' | b'\t' | b',' | b'=') && self.assigned(e) {
            self.put(p, e, CONSTANT_NAME);
            return e;
        }
        let w = WORDS.value(&b[p..e], &INFO);
        if w != 0 && !(after == b'=' && w & W_NO_QBE != 0) {
            self.put(p, e, w & 0xff);
            if w & W_REGEX != 0 {
                self.regex_at = e;
            }
            return e;
        }
        if after == b'[' && &b[p..e] == b"ENV" {
            self.put(p, e, CONSTANT);
            self.push(F_ENV);
            return e + 1;
        }
        match after {
            b'.' if b.get(e + 1).is_some_and(|&c| is_name_byte(c)) => e,
            b'[' => e,
            _ => {
                self.put_hot(p, e, CONSTANT);
                e
            }
        }
    }

    /// Whether a constant's name ending at `e` is assigned, bat's
    /// `(?=(\s*,\s*[[:upper:]]\w*)*\s*=(?![=\>]))`: the answer kept for the constants of the
    /// same list ([`Scan::chain`]), so a line of them stays linear.
    #[inline(never)]
    fn assigned(&mut self, e: usize) -> bool {
        let (end, ok) = self.chain;
        if e <= end && e > self.chain_from {
            return ok;
        }
        let b = self.b;
        let mut q = e;
        let ok = loop {
            q = skip_blanks(b, q);
            match b.get(q) {
                Some(b'=') => break !matches!(b.get(q + 1), Some(b'=' | b'>')),
                Some(b',') => {
                    let s = skip_blanks(b, q + 1);
                    if !b.get(s).is_some_and(u8::is_ascii_uppercase) {
                        break false;
                    }
                    q = word_end(b, s);
                }
                _ => break false,
            }
        };
        self.chain_from = e - 1;
        self.chain = (q, ok);
        ok
    }

    /// `:` at `p`: `::`, a symbol, or the ternary's.
    #[inline(always)]
    fn colon(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        match b.get(p + 1).copied() {
            Some(b':') => {
                self.put(p, p + 2, DOUBLE_COLON);
                if b.get(p + 2).is_some_and(|&c| is_name_byte(c)) {
                    // bat's `support.class` takes `::Name` before `.x`, `::x` or `[` first
                    if b[p + 2].is_ascii_uppercase() && class_follows(b, word_end(b, p + 2)) {
                        return word_end(b, p + 2);
                    }
                    return self.method(p + 2);
                }
                p + 2
            }
            Some(c)
                if c.is_ascii_alphabetic()
                    || c == b'_'
                    || c >= 0x80 && is_word_starting(b, p + 1) =>
            {
                let mut e = word_end(b, p + 1);
                if e < n
                    && (matches!(b[e], b'?' | b'!')
                        || b[e] == b'=' && !matches!(b.get(e + 1), Some(b'>' | b'=')))
                {
                    e += 1;
                }
                self.put_hot(p, p + 1, SYMBOL_MARK);
                self.put_hot(p + 1, e, SYMBOL);
                e
            }
            Some(b'"') => self.open(p, p + 2, string_byte(S_SYM_DQ, D_KIND), 0),
            Some(b'\'') => self.open(p, p + 2, string_byte(S_SYM_SQ, D_KIND), 0),
            Some(b'@') => {
                let s = if b.get(p + 2) == Some(&b'@') {
                    p + 3
                } else {
                    p + 2
                };
                if b.get(s)
                    .is_some_and(|&c| c.is_ascii_alphabetic() || c == b'_')
                {
                    let e = word_end(b, s);
                    self.put(p, p + 1, SYMBOL_MARK);
                    self.put(p + 1, e, SYMBOL);
                    return e;
                }
                self.conditional(p)
            }
            Some(_) => {
                let e = op_symbol(b, p + 1);
                if e > p + 1 {
                    self.put(p, p + 1, SYMBOL_MARK);
                    self.put(p + 1, e, SYMBOL);
                    return e;
                }
                self.conditional(p)
            }
            None => self.conditional(p),
        }
    }

    /// The ternary's `:` at `p`.
    #[inline(always)]
    fn conditional(&mut self, p: usize) -> usize {
        self.put(p, p + 1, OP_CONDITIONAL);
        self.regex_at = p + 1;
        p + 1
    }

    /// `@` at `p`: an instance or class variable.
    #[inline(always)]
    fn at(&mut self, p: usize) -> usize {
        let b = self.b;
        let (s, mark, kind) = if b.get(p + 1) == Some(&b'@') {
            (p + 2, CVAR_MARK, CVAR)
        } else {
            (p + 1, IVAR_MARK, IVAR)
        };
        if !b
            .get(s)
            .is_some_and(|&c| c.is_ascii_alphabetic() || c == b'_')
        {
            return s;
        }
        let e = word_end(b, s);
        self.put_hot(p, s, mark);
        self.put_hot(s, e, kind);
        e
    }

    /// `$` at `p`: a global variable, or a pre-defined one.
    #[inline(always)]
    fn dollar(&mut self, p: usize) -> usize {
        let b = self.b;
        let Some(&c) = b.get(p + 1) else {
            return p + 1;
        };
        if c.is_ascii_alphabetic() || c == b'_' {
            let e = word_end(b, p + 1);
            self.put(p, p + 1, GVAR_MARK);
            self.put(p + 1, e, GVAR);
            return e;
        }
        let e = match c {
            b'0'..=b'9' => {
                let mut e = p + 2;
                while e < b.len() && b[e].is_ascii_digit() {
                    e += 1;
                }
                e
            }
            b'-' if b.get(p + 2).is_some_and(|c| b"0adFiIlpv".contains(c)) => p + 3,
            b'!' | b'@' | b'&' | b'`' | b'\'' | b'+' | b'~' | b'=' | b'/' | b'\\' | b',' | b';'
            | b'.' | b'<' | b'>' | b'*' | b'$' | b'?' | b':' | b'"' => p + 2,
            _ => return p + 1,
        };
        self.put(p, p + 1, GVAR_PRE_MARK);
        self.put(p + 1, e, GVAR_PRE);
        e
    }

    /// `.` at `p`: a range, or an accessor and the method after it.
    #[inline(always)]
    fn dot(&mut self, p: usize) -> usize {
        let b = self.b;
        if b.get(p + 1) == Some(&b'.') {
            let e = if b.get(p + 2) == Some(&b'.') {
                p + 3
            } else {
                p + 2
            };
            self.put(p, e, OP_RANGE);
            self.regex_at = e;
            return e;
        }
        self.put_hot(p, p + 1, DOT);
        let s = p + 1;
        if !b.get(s).is_some_and(|&c| is_name_byte(c)) {
            return s;
        }
        self.method(s)
    }

    /// A method's name at `s` after `.` or `::`: bat's well-known ones, else plain.
    #[inline(always)]
    fn method(&mut self, s: usize) -> usize {
        let b = self.b;
        let e = word_end(b, s);
        let after = b.get(e).copied().unwrap_or(b'\n');
        if matches!(after, b'?' | b'!') {
            let w = WORDS.value(&b[s..e + 1], &INFO);
            if w & W_METHOD != 0 {
                self.put(s, e + 1, w & 0xff);
                self.regex_at = e + 1;
                return e + 1;
            }
        }
        let w = WORDS.value(&b[s..e], &INFO);
        if w & W_METHOD != 0
            && !(matches!(after, b'?' | b'!') && w & (W_NO_QB | W_NO_QBE) != 0)
            && !(after == b'=' && w & W_NO_QBE != 0)
        {
            self.put_hot(s, e, w & 0xff);
            if w & W_REGEX != 0 {
                self.regex_at = e;
            }
            return e;
        }
        if &b[s..e] == b"class" && !matches!(after, b'?' | b'!' | b'=') {
            self.put(s, e, BUILTIN);
            self.regex_at = e;
            return e;
        }
        match after {
            b'?' | b'!' => self.after_name(e + 1),
            b'=' if !matches!(b.get(e + 1), Some(b'>' | b'=')) => self.after_name(e + 1),
            _ => self.after_name(e),
        }
    }

    /// `?` at `p`: a character literal, or the ternary's.
    #[inline(always)]
    fn question(&mut self, p: usize) -> usize {
        let b = self.b;
        if let Some((e, bad)) = character(b, p) {
            self.put(p, p + 1, CHARACTER_MARK);
            self.put(p + 1, e, if bad { CHARACTER_BAD } else { CHARACTER });
            return e;
        }
        self.put(p, p + 1, OP_CONDITIONAL);
        // bat's `?` takes `name:` after it as a name and the ternary's `:`
        let s = skip_blanks(b, p + 1);
        if b.get(s)
            .is_some_and(|&c| c.is_ascii_alphabetic() || c == b'_')
            || b.get(s).is_some_and(|&c| c >= 0x80) && is_name_start(b, s)
        {
            let e = word_end(b, s);
            if b.get(e) == Some(&b':') && b.get(e + 1) != Some(&b':') {
                self.put(e, e + 1, OP_CONDITIONAL);
                self.regex_at = e + 1;
                return e + 1;
            }
        }
        self.regex_at = p + 1;
        p + 1
    }

    /// An operator's run at `p`, `<` aside: bat's operators in their order.
    #[inline(always)]
    fn op(&mut self, p: usize) -> usize {
        let b = self.b;
        let c = b[p];
        if c == b'|' && self.depth > 0 && self.top() == F_BLOCK_DEFAULT {
            self.pop();
            return self.resume(p);
        }
        let at = |i: usize| b.get(p + i).copied().unwrap_or(0);
        let (len, kind) = match c {
            b'=' => match at(1) {
                b'>' => (2, KEY_VALUE),
                b'=' => (if at(2) == b'=' { 3 } else { 2 }, OP_COMPARISON),
                b'~' => (2, OP_COMPARISON),
                _ => (1, OP_ASSIGNMENT),
            },
            b'>' => match at(1) {
                b'>' => (2, OP_OTHER),
                b'=' => (2, OP_COMPARISON),
                _ => (1, OP_COMPARISON),
            },
            b'!' => match at(1) {
                b'=' | b'~' => (2, OP_COMPARISON),
                _ => {
                    let mut e = p + 1;
                    while e < b.len() && b[e] == b'!' {
                        e += 1;
                    }
                    (e - p, OP_LOGICAL)
                }
            },
            b'&' => match at(1) {
                b'&' if at(2) == b'=' => (3, OP_AUGMENTED),
                b'&' => (2, OP_LOGICAL),
                b'=' => (2, OP_AUGMENTED),
                _ => (1, OP_BITWISE),
            },
            b'|' => match at(1) {
                b'|' if at(2) == b'=' => (3, OP_AUGMENTED),
                b'|' => (2, OP_LOGICAL),
                b'=' => (2, OP_AUGMENTED),
                _ => (1, OP_BITWISE),
            },
            b'*' => match at(1) {
                b'*' if at(2) == b'=' => (3, OP_AUGMENTED),
                b'*' => (2, OP_ARITHMETIC),
                b'=' => (2, OP_AUGMENTED),
                _ => (1, OP_ARITHMETIC),
            },
            b'-' | b'+' | b'^' => {
                if at(1) == b'=' {
                    (2, OP_AUGMENTED)
                } else if c == b'^' {
                    (1, OP_BITWISE)
                } else {
                    (1, OP_ARITHMETIC)
                }
            }
            _ => (1, OP_BITWISE),
        };
        self.put_hot(p, p + len, kind);
        self.regex_at = p + len;
        p + len
    }

    /// `<` at `p`: a here-document's opener, or an operator.
    #[inline(always)]
    fn lt(&mut self, p: usize) -> usize {
        let b = self.b;
        let at = |i: usize| b.get(p + i).copied().unwrap_or(0);
        if at(1) == b'<' {
            let c = at(2);
            if (matches!(c, b'~' | b'-' | b'"' | b'\'' | b'`') || WORD[usize::from(c)])
                && let Some(e) = self.heredoc(p)
            {
                return e;
            }
        }
        let (len, kind) = match at(1) {
            b'<' if at(2) == b'=' => (3, OP_AUGMENTED),
            b'<' => (2, OP_AUGMENTED),
            b'=' if at(2) == b'>' => (3, OP_COMPARISON),
            b'=' => (2, OP_COMPARISON),
            _ => (1, OP_COMPARISON),
        };
        self.put_hot(p, p + len, kind);
        self.regex_at = p + len;
        p + len
    }

    /// `<<` at `p`: a here-document's opener, `<<[-~]?["'`]?word["'`]?`, its label queued; the
    /// rest of the line is code.
    #[inline(never)]
    fn heredoc(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let mut q = p + 2;
        let indented = matches!(b.get(q), Some(b'~' | b'-'));
        if indented {
            q += 1;
        }
        let quote = match b.get(q) {
            Some(&c @ (b'"' | b'\'' | b'`')) => {
                q += 1;
                c
            }
            _ => 0,
        };
        let s = q;
        while q < b.len() && WORD[usize::from(b[q])] {
            q += 1;
        }
        if q == s {
            return None;
        }
        let word = &b[s..q];
        let e = match quote {
            0 => q,
            c if b.get(q) == Some(&c) => q + 1,
            // bat's `["`]?` closing quote is optional, its `'` isn't
            b'\'' => return None,
            _ => q,
        };
        let lang = heredoc_language(word);
        let flags = lang | if indented { H_INDENTED } else { 0 };
        let i = self.queue.len();
        if self.queue.push(Label::of(word), flags) && quote == b'\'' {
            self.literal |= 1 << i;
        }
        self.put(p, e, HEREDOC_BEGIN);
        Some(e)
    }

    /// A line of a here-document's body: its closer (what the body opened dropped), a literal
    /// one's text, or the line to lex as the body's text or Ruby's code from what the body's
    /// lines before left (`true`).
    #[inline(never)]
    fn body(&mut self) -> bool {
        let b = self.b;
        let n = b.len();
        let Some((label, flags)) = self.queue.front() else {
            self.mode = M_CODE;
            return true;
        };
        let lead = if flags & H_INDENTED != 0 {
            Lead::Blanks
        } else {
            Lead::None
        };
        if let Some(c) = label.close(b, lead, Tail::Blanks)
            && c.end == n
        {
            self.put(0, c.start, HEREDOC);
            self.put(c.start, c.end, HEREDOC_END);
            self.queue.pop();
            self.literal >>= 1;
            self.depth = self.base.min(self.depth);
            if self.queue.is_empty() {
                self.mode = M_CODE;
            }
            return false;
        }
        let literal = self.literal & 1 != 0;
        match flags & 7 {
            H_PLAIN if literal => {
                self.put(0, n, HEREDOC);
                false
            }
            H_PLAIN => {
                if self.depth <= self.base {
                    self.push_string(string_byte(S_HEREDOC, D_KIND), 0);
                }
                true
            }
            H_RUBY => {
                self.ruby_interp = !literal;
                true
            }
            H_SHELL if literal || !ruby_marks(b) => {
                self.shell();
                false
            }
            H_CSS | H_JS | H_HTML | H_SHELL => {
                if literal {
                    return false;
                }
                if self.depth <= self.base {
                    self.push_string(string_byte(S_EMBED, D_KIND), 0);
                }
                true
            }
            H_SQL => {
                if self.depth <= self.base {
                    self.push(crate::sql::MAIN | if literal { 0 } else { 0x80 });
                    self.push(F_SQL);
                }
                true
            }
            _ => false,
        }
    }

    /// An escape at `p`, `\`: bat's `escaped-char`.
    #[inline(always)]
    fn escape(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let Some(&c) = b.get(p + 1) else {
            return n;
        };
        let e = match c {
            b'M' | b'C' if b.get(p + 2) == Some(&b'-') => meta_end(b, p).unwrap_or(p + 3),
            b'c' => meta_end(b, p).unwrap_or(p + 2),
            b'u' if b.get(p + 2) == Some(&b'{') => {
                // `\u{…}`: hex digits, an illegal escape for anything else
                let mut e = p + 3;
                self.put(p, e, ESCAPE);
                while e < n && b[e] != b'}' {
                    let s = e;
                    while e < n && e < s + 6 && b[e].is_ascii_hexdigit() {
                        e += 1;
                    }
                    if e > s {
                        self.put(s, e, ESCAPE);
                    } else if b[e] == b' ' || b[e] == b'\t' {
                        self.put(e, e + 1, ESCAPE);
                        e += 1;
                    } else {
                        let l = char_len(b[e]);
                        self.put(e, (e + l).min(n), BAD_ESCAPE);
                        e += l;
                    }
                }
                if e < n {
                    self.put(e, e + 1, ESCAPE);
                    e += 1;
                }
                return e.min(n);
            }
            b'0'..=b'7' => {
                let mut e = p + 2;
                while e < n && e < p + 4 && (b'0'..=b'7').contains(&b[e]) {
                    e += 1;
                }
                e
            }
            b'x' if b.get(p + 2).is_some_and(u8::is_ascii_hexdigit) => {
                if b.get(p + 3).is_some_and(u8::is_ascii_hexdigit) {
                    p + 4
                } else {
                    p + 3
                }
            }
            b'u' if b
                .get(p + 2..p + 6)
                .is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit)) =>
            {
                p + 6
            }
            _ => (p + 1 + char_len(c)).min(n),
        };
        self.put(p, e, ESCAPE);
        e
    }

    /// `#@x`, `#@@x`, `#$x` at `p` in an interpolating string: the variable, `#` and its marks
    /// the punctuation.
    #[inline(always)]
    fn interp_var(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let (s, kind) = match b.get(p + 1) {
            Some(b'@') if b.get(p + 2) == Some(&b'@') => (p + 3, CVAR),
            Some(b'@') => (p + 2, IVAR),
            Some(b'$') => (p + 2, GVAR),
            _ => return None,
        };
        if !b
            .get(s)
            .is_some_and(|&c| c.is_ascii_alphabetic() || c == b'_')
        {
            return None;
        }
        let e = word_end(b, s);
        self.put(p, s, kind + 1);
        self.put(s, e, kind);
        Some(e)
    }

    /// `%` at `p`: a literal (`%w[`, `%q(`, `%r{`, `%(` …), else the operator.
    #[inline(always)]
    fn percent(&mut self, p: usize) -> usize {
        let b = self.b;
        let at = |i: usize| b.get(p + i).copied().unwrap_or(b'\n');
        let c = at(1);
        let (kind, d, open) = match c {
            b'x' if !is_word_byte(at(2)) && at(2) != b'\n' => (S_BT, at(2), p + 3),
            b'r' if !is_word_byte(at(2)) && at(2) != b'\n' => (S_REGEX_R, at(2), p + 3),
            b'Q' | b'W' | b'I' if matches!(at(2), b'(' | b'[' | b'<') => (S_UPPER, at(2), p + 3),
            b'Q' | b'W' | b'I' if at(2) == b'{' => (S_MOD, at(2), p + 3),
            b'Q' | b'W' | b'I' if !is_word_byte(at(2)) && at(2) != b'\n' => (S_UPPER, at(2), p + 3),
            b'(' | b'[' | b'<' => (S_UPPER, c, p + 2),
            b'{' => (S_MOD, c, p + 2),
            b'q' | b'w' | b's' | b'i' if matches!(at(2), b'(' | b'[' | b'<' | b'{') => {
                (S_LOWER, at(2), p + 3)
            }
            b'q' | b'w' | b's' | b'i' if !is_word_byte(at(2)) && at(2) != b'\n' => {
                (S_LOWER_ANY, at(2), p + 3)
            }
            _ if !is_word_byte(c)
                && !matches!(c, b' ' | b'\t' | b'\r' | 0x0b | 0x0c | b'=' | b'\n') =>
            {
                (S_OTHER, c, p + 2)
            }
            _ => {
                self.put_hot(p, p + 1, OP_ARITHMETIC);
                self.regex_at = p + 1;
                return p + 1;
            }
        };
        if d >= 0x80 {
            // a delimiter past ASCII: none of these
            self.put_hot(p, p + 1, OP_ARITHMETIC);
            self.regex_at = p + 1;
            return p + 1;
        }
        let (delim, aux) = match d {
            b'(' => (D_PAREN, 0),
            b'[' => (D_BRACKET, 0),
            b'{' => (D_BRACE, 0),
            b'<' => (D_ANGLE, 0),
            _ => (D_CHAR, d),
        };
        if kind == S_REGEX_R {
            self.put(p, open, REGEX_R_TEXT + 1);
            self.push_regex(string_byte(kind, delim), aux);
            return self.string(open);
        }
        self.open(p, open, string_byte(kind, delim), aux)
    }

    /// `/` at `p`: bat's `/=`, a regex where its grammar tries one and the line has another
    /// `/` after it, else division.
    #[inline(always)]
    fn slash(&mut self, p: usize) -> usize {
        let b = self.b;
        if b.get(p + 1) == Some(&b'=') {
            self.put(p, p + 2, OP_AUGMENTED);
            self.regex_at = p + 2;
            return p + 2;
        }
        let tried = self.regex_at <= p && b[self.regex_at..p].iter().all(|&c| is_blank(c));
        let from = if tried { self.regex_at } else { p };
        if tried || regex_may_follow(b, p + 1) {
            if self.last_slash == usize::MAX {
                self.last_slash = b.iter().rposition(|&c| c == b'/').unwrap_or(0);
            }
            if self.last_slash > p {
                // the blanks bat's `\s*(/)` takes are the regex's
                self.put(from, p, REGEX_TEXT);
                self.put(p, p + 1, REGEX_TEXT + 1);
                self.push_regex(string_byte(S_REGEX, D_KIND), 0);
                return self.string(p + 1);
            }
        }
        self.put_hot(p, p + 1, OP_ARITHMETIC);
        self.regex_at = p + 1;
        p + 1
    }

    /// A string's opener at `p..q`: its begin, then its body.
    #[inline(always)]
    fn open(&mut self, p: usize, q: usize, byte: u8, aux: u8) -> usize {
        let kind = byte & 0xf;
        self.put_hot(p, q, STRING + 3 * u16::from(kind) + 1);
        self.push_string(byte, aux);
        self.string(q)
    }

    /// The open string's body from `p` (its frame on top): its escapes, interpolations and
    /// nested brackets, to its end (its frame popped) or the line's.
    #[inline(always)]
    fn string(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let (byte, mut aux) = self.string_top();
        let kind = byte & 0xf;
        let delim = byte >> 4 & 7;
        let text = STRING + 3 * u16::from(kind);
        let (open, close) = match delim {
            D_KIND => (
                0u8,
                match kind {
                    S_SQ | S_SYM_SQ => b'\'',
                    S_BT => b'`',
                    S_REGEX => b'/',
                    S_HEREDOC | S_EMBED => 0,
                    _ => b'"',
                },
            ),
            D_PAREN => (b'(', b')'),
            D_BRACKET => (b'[', b']'),
            D_BRACE => (b'{', b'}'),
            D_ANGLE => (b'<', b'>'),
            _ => (0, aux),
        };
        if kind == S_REGEX || kind == S_REGEX_R {
            return self.regex(p, open, close);
        }
        let interp = interpolates(kind);
        let placeholders = kind <= S_DQ;
        let mut t = p;
        while p < n {
            let c = b[p];
            if c == close {
                if kind >= S_HEREDOC {
                    p += 1;
                    continue;
                }
                if open != 0 && aux > 0 {
                    aux -= 1;
                    self.put(t, p, text);
                    self.put(p, p + 1, SCOPE);
                    p += 1;
                    t = p;
                    continue;
                }
                self.put_hot(t, p, text);
                self.put_hot(p, p + 1, text + 2);
                self.depth = self.depth.saturating_sub(2);
                return p + 1;
            }
            if c == open && open != 0 {
                aux = aux.saturating_add(1);
                self.put(t, p, text);
                self.put(p, p + 1, SCOPE);
                p += 1;
                t = p;
                continue;
            }
            match c {
                b'\\' => {
                    if interp {
                        self.put(t, p, text);
                        p = self.escape(p);
                        t = p;
                    } else if kind == S_LOWER_ANY {
                        p = (p + 2).min(n);
                    } else if matches!(b.get(p + 1), Some(&d) if d == b'\\' || d == close) {
                        self.put(t, p, text);
                        self.put(p, p + 2, ESCAPE);
                        p += 2;
                        t = p;
                    } else {
                        p += 1;
                    }
                }
                b'#' if interp && matches!(b.get(p + 1), Some(b'{' | b'@' | b'$')) => {
                    if b[p + 1] == b'{' {
                        self.put(t, p, text);
                        self.put(p, p + 2, INTERP_BEGIN);
                        let d = usize::from(self.depth);
                        if d >= 2 {
                            self.frames[d - 2] = aux;
                        }
                        self.push(F_INTERP);
                        self.regex_at = p + 2;
                        return p + 2;
                    }
                    self.put(t, p, text);
                    t = p;
                    if let Some(e) = self.interp_var(p) {
                        p = e;
                        t = p;
                    } else {
                        p += 1;
                    }
                }
                b'%' if placeholders => {
                    if let Some(e) = placeholder(b, p) {
                        self.put(t, p, text);
                        self.put(p, e, PLACEHOLDER);
                        p = e;
                        t = p;
                    } else {
                        p += 1;
                    }
                }
                _ => p += 1,
            }
        }
        self.put_hot(t, n, text);
        let d = usize::from(self.depth);
        if d >= 2 {
            self.frames[d - 2] = aux;
        }
        n
    }

    /// A regex's body from `p` (its frame on top), to its end and flags, or the line's: on
    /// [`crate::regex`] in Ruby's dialect, the line cut at each of the host's places, which Ruby's
    /// lexer finds before the regex is read: the closing delimiter (an unescaped one, a bracket's
    /// nesting counted wherever it stands, in a class too), `#{`, `#@x` and `#$x`.
    #[inline(never)]
    fn regex(&mut self, mut p: usize, open: u8, close: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let (byte, mut aux) = self.string_top();
        let kind = byte & 0xf;
        let d = usize::from(self.depth);
        let mut st = if d >= 3 {
            self.frames[d - 3] & crate::regex::RUBY_STATE
        } else {
            0
        };
        let classic = kind == S_REGEX;
        let base = 2 * usize::from(!classic);
        let comment = REGEX_COMMENT + u16::from(!classic);
        // where the host's places are looked for from: past a `#@` that isn't a variable's
        let mut from = p;
        loop {
            let mut q = from;
            while q < n {
                let c = b[q];
                if c == b'\\' {
                    q = (q + 1 + b.get(q + 1).map_or(0, |&c| char_len(c))).min(n);
                    continue;
                }
                if c == close {
                    if open != 0 && aux > 0 {
                        aux -= 1;
                        q += 1;
                        continue;
                    }
                    break;
                }
                if c == open && open != 0 {
                    aux = aux.saturating_add(1);
                } else if c == b'#' && matches!(b.get(q + 1), Some(b'{' | b'@' | b'$')) {
                    break;
                }
                q += 1;
            }
            if q < n && b[q] != close && b[q] == b'#' && b[q + 1] != b'{' && !is_interp_var(b, q) {
                // `#@` before no name: the regex's
                from = q + 1;
                continue;
            }
            if self.re_x == 0 {
                // the extended syntax from where the cut stopped: the flags after a closer, a body
                // that goes on past the line; past an interpolation, a look ahead
                let x = if q == n {
                    true
                } else if b[q] == close {
                    flags_x(b, q + 1)
                } else {
                    extended(b, q, open, close, aux)
                };
                self.re_x = 1 + u8::from(x);
            }
            let embed = &RE_EMBEDS[base + usize::from(self.re_x == 2)];
            self.regex_text(p, q, &mut st, embed, comment);
            if q == n {
                break;
            }
            if b[q] == close {
                let mut e = q + 1;
                while e < n && b"eimnosux".contains(&b[e]) {
                    e += 1;
                }
                let text = STRING + 3 * u16::from(kind);
                if classic {
                    self.put(q, q + 1, text + 2);
                    self.put(q + 1, e, REGEX_FLAGS);
                } else {
                    self.put(q, e, text + 2);
                }
                self.depth = self.depth.saturating_sub(3);
                self.re_x = 0;
                return e;
            }
            if b[q + 1] == b'{' {
                self.put(q, q + 2, INTERP_BEGIN);
                self.keep_regex(aux, st);
                self.push(F_INTERP);
                self.regex_at = q + 2;
                return q + 2;
            }
            let e = self.interp_var(q).unwrap_or(q + 1);
            p = e;
            from = e;
        }
        self.keep_regex(aux, st);
        n
    }

    /// The open regex's `aux` and regex state kept in its frame.
    #[inline(always)]
    fn keep_regex(&mut self, aux: u8, st: u8) {
        let d = usize::from(self.depth);
        if d >= 2 {
            self.frames[d - 2] = aux;
        }
        if d >= 3 {
            self.frames[d - 3] = st;
        }
    }

    /// A regex's text `s..e` on [`crate::regex`], the line cut at `e`, from regex state `st` and
    /// left there. In the extended syntax a `#` outside a class starts a comment to `e`.
    fn regex_text(
        &mut self,
        s: usize,
        e: usize,
        st: &mut u8,
        embed: &crate::regex::Embed,
        comment: u16,
    ) {
        if s >= e {
            return;
        }
        let b = &self.b[..e];
        let hash = embed.yields[usize::from(b'#')];
        self.k.close(self.out);
        if hash && b[s] == b'#' && *st & crate::regex::SET == 0 {
            // the scan takes no place where it starts: a comment from there is the host's to see
            self.out.push(s, e, comment);
        } else {
            let mut at = s;
            let mut q = s;
            while let Some((stop, ns)) = crate::regex::embedded(b, q, *st, embed, &mut at, self.out)
            {
                *st = ns;
                if stop >= e {
                    break;
                }
                // stopped at a `#`: in a class a character, which the scan takes from there,
                // else a comment
                if ns & crate::regex::SET != 0 {
                    q = stop;
                    continue;
                }
                self.out.push(stop, e, comment);
                break;
            }
        }
        self.k.reopen(self.out, 16);
    }

    /// A number at `p`: bat's forms in its order; a word that is none of them is plain.
    #[inline(always)]
    fn number(&mut self, p: usize) -> usize {
        let b = self.b;
        let mut e = p + 1;
        while e < b.len() && b[e].is_ascii_digit() {
            e += 1;
        }
        let next = b.get(e).copied().unwrap_or(b'\n');
        if !WORD[usize::from(next)] && next != b'.' && next < 0x80 && !(b[p] == b'0' && e > p + 1) {
            self.put_hot(p, e, DECIMAL);
            return e;
        }
        self.number_slow(p)
    }

    /// [`Scan::number`]'s forms past a run of decimal digits.
    #[inline(never)]
    fn number_slow(&mut self, p: usize) -> usize {
        let b = self.b;
        let Some(m) = numeric(b, p) else {
            return word_end(b, p);
        };
        for &(s, e, kind) in &m.parts[..m.n] {
            self.put(s, e, kind);
        }
        m.end
    }
}

/// Oniguruma's `\s` on a line.
#[inline(always)]
fn is_blank(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | 0x0b | 0x0c)
}

/// A byte that can start a name, `[[:alpha:]_]` (past ASCII taken as one).
#[inline(always)]
fn is_name_byte(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

/// `\w`'s ASCII bytes and any past ASCII.
#[inline(always)]
fn is_word_byte(c: u8) -> bool {
    WORD[usize::from(c)] || c >= 0x80
}

fn skip_blanks(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && is_blank(b[p]) {
        p += 1;
    }
    p
}

/// bat's `support.class` lookahead after a constant's name ending at `e`,
/// `(?=((\.|::)[[:alpha:]_]|\[))`.
fn class_follows(b: &[u8], e: usize) -> bool {
    match b.get(e) {
        Some(b'[') => true,
        Some(b'.') => b.get(e + 1).is_some_and(|&c| is_name_byte(c)),
        Some(b':') => b.get(e + 1) == Some(&b':') && b.get(e + 2).is_some_and(|&c| is_name_byte(c)),
        _ => false,
    }
}

std::thread_local! {
    static FOREIGN: std::cell::RefCell<Vec<crate::Span>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Whether a line holds a mark of the here-document's own that bat reads before the embedded
/// language's: an interpolation (`#{`, `#@`, `#$`) or an escape.
fn ruby_marks(b: &[u8]) -> bool {
    b.iter()
        .enumerate()
        .any(|(i, &c)| c == b'\\' || c == b'#' && matches!(b.get(i + 1), Some(b'{' | b'@' | b'$')))
}

/// Whether a regex's body from `p` is in the extended syntax: the flags after its closing
/// delimiter on the line hold `x`, or the body goes on past the line (a literal over several
/// lines is nearly always `%r{…}x`; its flags come only at its end).
fn extended(b: &[u8], mut p: usize, open: u8, close: u8, mut aux: u8) -> bool {
    let n = b.len();
    while p < n {
        let c = b[p];
        if c == b'\\' {
            p += 2;
            continue;
        }
        if c == close {
            if open != 0 && aux > 0 {
                aux -= 1;
                p += 1;
                continue;
            }
            return flags_x(b, p + 1);
        }
        if c == open && open != 0 {
            aux = aux.saturating_add(1);
        } else if c == b'#' && b.get(p + 1) == Some(&b'{') {
            // an interpolation's code, to its `}`
            let mut depth = 0u32;
            p += 2;
            while p < n {
                match b[p] {
                    b'{' => depth += 1,
                    b'}' if depth == 0 => break,
                    b'}' => depth -= 1,
                    _ => {}
                }
                p += 1;
            }
        }
        p += 1;
    }
    true
}

/// Whether a regex's flags from `e` hold `x`.
fn flags_x(b: &[u8], mut e: usize) -> bool {
    while e < b.len() && b"eimnosux".contains(&b[e]) {
        if b[e] == b'x' {
            return true;
        }
        e += 1;
    }
    false
}

/// Whether `#` at `p` before `@` or `$` interpolates a variable ([`Scan::interp_var`]'s).
fn is_interp_var(b: &[u8], p: usize) -> bool {
    let s = match b.get(p + 1) {
        Some(b'@') if b.get(p + 2) == Some(&b'@') => p + 3,
        Some(b'@' | b'$') => p + 2,
        _ => return false,
    };
    b.get(s)
        .is_some_and(|&c| c.is_ascii_alphabetic() || c == b'_')
}

/// bat's `regexes` lookahead `(?=/\s*[^\w\(\s@"'])` at the byte after `/`, `q`.
#[inline(always)]
fn regex_may_follow(b: &[u8], q: usize) -> bool {
    let s = skip_blanks(b, q);
    match b.get(s) {
        None => false,
        Some(&c) => !(is_word_byte(c) || matches!(c, b'(' | b'@' | b'"' | b'\'')),
    }
}

/// A here-document's language by its word, bat's `heredoc_type_*`: `([[:upper:]_]_)?TYPE`.
fn heredoc_language(w: &[u8]) -> u8 {
    let w = if w.len() > 2 && w[1] == b'_' && (w[0].is_ascii_uppercase() || w[0] == b'_') {
        &w[2..]
    } else {
        w
    };
    match w {
        b"CSS" => H_CSS,
        b"JS" | b"JAVASCRIPT" => H_JS,
        b"HTML" => H_HTML,
        b"RUBY" => H_RUBY,
        b"SH" | b"SHELL" => H_SHELL,
        b"SQL" => H_SQL,
        _ => H_PLAIN,
    }
}

/// An unquoted operator symbol's end after its `:` at `p` (bat's `===?|>[>=]?|<[<=]?|<=>|
/// [%&`/\|]|\*\*?|=?~|[-+]@?|\[\]=?`), `p` for none.
fn op_symbol(b: &[u8], p: usize) -> usize {
    let at = |i: usize| b.get(p + i).copied().unwrap_or(0);
    match at(0) {
        b'=' if at(1) == b'=' => p + if at(2) == b'=' { 3 } else { 2 },
        b'=' if at(1) == b'~' => p + 2,
        b'>' => p + if matches!(at(1), b'>' | b'=') { 2 } else { 1 },
        b'<' => p + if matches!(at(1), b'<' | b'=') { 2 } else { 1 },
        b'%' | b'&' | b'`' | b'/' | b'|' | b'~' => p + 1,
        b'*' => p + if at(1) == b'*' { 2 } else { 1 },
        b'-' | b'+' => p + if at(1) == b'@' { 2 } else { 1 },
        b'[' if at(1) == b']' => p + if at(2) == b'=' { 3 } else { 2 },
        _ => p,
    }
}

/// A character literal at `?` `p`, bat's `constant.character` forms in their order: where it
/// ends, and whether it is the illegal form (`?abc`).
fn character(b: &[u8], p: usize) -> Option<(usize, bool)> {
    let q = p + 1;
    let c = *b.get(q)?;
    if c == b'\\' {
        let d = *b.get(q + 1)?;
        if let Some(e) = meta_end(b, q) {
            return Some((e, false));
        }
        let e = match d {
            b'x' if b.get(q + 2).is_some_and(u8::is_ascii_hexdigit) => {
                let e = if b.get(q + 3).is_some_and(u8::is_ascii_hexdigit) {
                    q + 4
                } else {
                    q + 3
                };
                (!is_word(b, e)).then_some(e)
            }
            b'0' => {
                let mut e = q + 2;
                while e < b.len() && e < q + 4 && (b'0'..=b'7').contains(&b[e]) {
                    e += 1;
                }
                (!is_word(b, e)).then_some(e)
            }
            _ => None,
        };
        if let Some(e) = e {
            return Some((e, false));
        }
        let e = (q + 1 + char_len(d)).min(b.len());
        if !is_word(b, e) {
            return Some((e, false));
        }
        return illegal(b, q).map(|e| (e, true));
    }
    if (c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 && is_name_start(b, q))
        && let Some(e) = illegal(b, q)
    {
        return Some((e, true));
    }
    if is_word_byte(c) {
        let e = (q + char_len(c)).min(b.len());
        if !is_word(b, e) && !colon_after(b, e) {
            return Some((e, false));
        }
        return None;
    }
    if is_blank(c) {
        return None;
    }
    Some(((q + char_len(c)).min(b.len()), false))
}

/// bat's illegal character literal after `?`, `([[:alpha:]_\\]\S+)\b(?!\s*:)`, from `q`:
/// where it ends (the run's last word boundary not before a `:`), looked for over 256 bytes.
fn illegal(b: &[u8], q: usize) -> Option<usize> {
    let mut r = q + 1;
    let cap = (q + 256).min(b.len());
    while r < cap && !is_blank(b[r]) {
        r += 1;
    }
    let mut e = r;
    while e >= q + 2 {
        if is_word(b, e - 1) != is_word(b, e) && !colon_after(b, e) {
            return Some(e);
        }
        e -= 1;
    }
    None
}

/// Whether `\s*:` follows at `e`.
fn colon_after(b: &[u8], e: usize) -> bool {
    b.get(skip_blanks(b, e)) == Some(&b':')
}

/// `{{method_punctuation}}` after a name ending at `e`, `(?:[?!]|=(?![>=]))?`: where it ends.
fn method_punct(b: &[u8], e: usize) -> usize {
    match b.get(e) {
        Some(b'?' | b'!') => e + 1,
        Some(b'=') if !matches!(b.get(e + 1), Some(b'>' | b'=')) => e + 1,
        _ => e,
    }
}

/// An operator method's name at `p`, bat's `===?|>[>=]?|<=>|<[<=]?|[%&`/\|]|\*\*?|=?~|[-+]@?|
/// \[\]=?`: where it ends, `p` for none.
fn def_op(b: &[u8], p: usize) -> usize {
    let at = |i: usize| b.get(p + i).copied().unwrap_or(0);
    match at(0) {
        b'=' if at(1) == b'=' => p + if at(2) == b'=' { 3 } else { 2 },
        b'=' if at(1) == b'~' => p + 2,
        b'>' => p + if matches!(at(1), b'>' | b'=') { 2 } else { 1 },
        b'<' if at(1) == b'=' && at(2) == b'>' => p + 3,
        b'<' => p + if matches!(at(1), b'<' | b'=') { 2 } else { 1 },
        b'%' | b'&' | b'`' | b'/' | b'|' | b'~' => p + 1,
        b'*' => p + if at(1) == b'*' { 2 } else { 1 },
        b'-' | b'+' => p + if at(1) == b'@' { 2 } else { 1 },
        b'[' if at(1) == b']' => p + if at(2) == b'=' { 3 } else { 2 },
        _ => p,
    }
}

/// bat's `path_lookahead` at `p`, `(::)?({{identifier}}(\.|::))*{{identifier}}`: a name
/// (after a `::`) starts there.
fn path_at(b: &[u8], p: usize) -> bool {
    let p = if b[p.min(b.len())..].starts_with(b"::") {
        p + 2
    } else {
        p
    };
    is_name_start(b, p)
}

/// `(?:\\(?:[MC]-|c)){1,2}[[:ascii:]]` at `p`: where it ends.
fn meta_end(b: &[u8], p: usize) -> Option<usize> {
    let one = |q: usize| -> Option<usize> {
        if b.get(q) != Some(&b'\\') {
            return None;
        }
        match b.get(q + 1)? {
            b'M' | b'C' if b.get(q + 2) == Some(&b'-') => Some(q + 3),
            b'c' => Some(q + 2),
            _ => None,
        }
    };
    let q = one(p)?;
    let q = one(q).unwrap_or(q);
    b.get(q).is_some_and(u8::is_ascii).then_some(q + 1)
}

/// bat's `string-placeholder` at `%` `p`: where it ends.
fn placeholder(b: &[u8], p: usize) -> Option<usize> {
    let mut q = p + 1;
    loop {
        match b.get(q) {
            Some(b'#' | b'0' | b'-' | b' ' | b'+' | b'*') => q += 1,
            Some(c) if c.is_ascii_digit() => {
                let mut r = q;
                while r < b.len() && b[r].is_ascii_digit() {
                    r += 1;
                }
                if b.get(r) == Some(&b'$') {
                    q = r + 1;
                } else {
                    break;
                }
            }
            _ => break,
        }
    }
    if b.get(q) == Some(&b'-') && b.get(q + 1).is_some_and(u8::is_ascii_digit) {
        q += 1;
    }
    while q < b.len() && b[q].is_ascii_digit() {
        q += 1;
    }
    if b.get(q) == Some(&b'.') {
        q += 1;
        while q < b.len() && b[q].is_ascii_digit() {
            q += 1;
        }
    }
    let c = *b.get(q)?;
    b"diouxXDOUeEfFgGaAcCsSpnvtTbByYhHmMzZ%"
        .contains(&c)
        .then_some(q + 1)
}

/// A number's parts as bat's grammar scopes them.
struct Numeric {
    parts: [(usize, usize, u16); 6],
    n: usize,
    end: usize,
}

/// Digits of a base, `\d+(?:_\d+)*`, at `p`: where they end.
fn digits(b: &[u8], p: usize, ok: fn(u8) -> bool) -> Option<usize> {
    let mut q = p;
    if !b.get(q).is_some_and(|&c| ok(c)) {
        return None;
    }
    loop {
        while q < b.len() && ok(b[q]) {
            q += 1;
        }
        if b.get(q) == Some(&b'_') && b.get(q + 1).is_some_and(|&c| ok(c)) {
            q += 1;
        } else {
            return Some(q);
        }
    }
}

fn is_dec(c: u8) -> bool {
    c.is_ascii_digit()
}
fn is_hex(c: u8) -> bool {
    c.is_ascii_hexdigit()
}
fn is_oct(c: u8) -> bool {
    (b'0'..=b'7').contains(&c)
}
fn is_bin(c: u8) -> bool {
    c == b'0' || c == b'1'
}

/// bat's numbers at `p`, in its order: imaginary, rational, float, integer; each by base.
fn numeric(b: &[u8], p: usize) -> Option<Numeric> {
    let ends = |q: usize| !is_word(b, q);
    let mut m = Numeric {
        parts: [(0, 0, 0); 6],
        n: 0,
        end: p,
    };
    let add = |m: &mut Numeric, s: usize, e: usize, k: u16| {
        if e > s {
            m.parts[m.n] = (s, e, k);
            m.n += 1;
        }
    };
    // a based form: `0x`, `0o`/`0`, `0b`, and its digits
    let based = |p: usize| -> [Option<(usize, usize, u16)>; 3] {
        let at = |i: usize| b.get(p + i).copied().unwrap_or(0);
        let hex = (at(0) == b'0' && matches!(at(1), b'x' | b'X'))
            .then(|| digits(b, p + 2, is_hex).map(|e| (p + 2, e, 1)))
            .flatten();
        let oct = if at(0) == b'0' && matches!(at(1), b'o' | b'O') {
            digits(b, p + 2, is_oct).map(|e| (p + 2, e, 2))
        } else if at(0) == b'0' {
            digits(b, p + 1, is_oct).map(|e| (p + 1, e, 2))
        } else {
            None
        };
        let bin = (at(0) == b'0' && matches!(at(1), b'b' | b'B'))
            .then(|| digits(b, p + 2, is_bin).map(|e| (p + 2, e, 3)))
            .flatten();
        [hex, oct, bin]
    };
    let forms = based(p);
    let rat = |base: u16| RATIONAL + base;
    let img = |base: u16| RATIONAL + 4 + base;
    // imaginary, based: `(r?i)(r)?\b`
    for f in forms.iter().flatten() {
        let (ds, de, base) = *f;
        let mut q = de;
        if b.get(q) == Some(&b'r') {
            q += 1;
        }
        if b.get(q) == Some(&b'i') {
            q += 1;
            let bad = (b.get(q) == Some(&b'r')).then_some(q);
            let e = bad.map_or(q, |r| r + 1);
            if ends(e) {
                add(&mut m, p, ds, img(base) + 8);
                add(&mut m, ds, de, img(base));
                add(&mut m, de, q, img(base) + 24);
                if let Some(r) = bad {
                    add(&mut m, r, r + 1, img(base) + 32);
                }
                m.end = e;
                return Some(m);
            }
        }
    }
    // imaginary, decimal
    let dec_base = (b.get(p) == Some(&b'0') && matches!(b.get(p + 1), Some(b'd' | b'D')))
        .then(|| digits(b, p + 2, is_dec))
        .flatten();
    let dd = digits(b, p, is_dec);
    {
        let try_ri = |q: usize| -> Option<(usize, Option<usize>, usize)> {
            let mut r = q;
            if b.get(r) == Some(&b'r') {
                r += 1;
            }
            if b.get(r) != Some(&b'i') {
                return None;
            }
            r += 1;
            let bad = (b.get(r) == Some(&b'r')).then_some(r);
            let e = bad.map_or(r, |x| x + 1);
            ends(e).then_some((r, bad, e))
        };
        // `(0[dD])?ddigits(r?i)`
        if let Some(de) = dec_base
            && let Some((r, bad, e)) = try_ri(de)
        {
            add(&mut m, p, p + 2, img(0) + 8);
            add(&mut m, p + 2, de, img(0));
            add(&mut m, de, r, img(0) + 24);
            if let Some(x) = bad {
                add(&mut m, x, x + 1, img(0) + 32);
            }
            m.end = e;
            return Some(m);
        }
        if let Some(de) = dd {
            if let Some((r, bad, e)) = try_ri(de) {
                add(&mut m, p, de, img(0));
                add(&mut m, de, r, img(0) + 24);
                if let Some(x) = bad {
                    add(&mut m, x, x + 1, img(0) + 32);
                }
                m.end = e;
                return Some(m);
            }
            // `ddigits(\.)ddigits(r?i)`
            if b.get(de) == Some(&b'.')
                && let Some(fe) = digits(b, de + 1, is_dec)
                && let Some((r, bad, e)) = try_ri(fe)
            {
                add(&mut m, p, de, img(0));
                add(&mut m, de, de + 1, img(0) + 16);
                add(&mut m, de + 1, fe, img(0));
                add(&mut m, fe, r, img(0) + 24);
                if let Some(x) = bad {
                    add(&mut m, x, x + 1, img(0) + 32);
                }
                m.end = e;
                return Some(m);
            }
            // `ddigits((\.)ddigits)?exponent(r)?(i)(r)?`
            let (frac, fe) = match b.get(de) {
                Some(b'.') => match digits(b, de + 1, is_dec) {
                    Some(fe) => (true, fe),
                    None => (false, de),
                },
                _ => (false, de),
            };
            for (frac, fe) in [(frac, fe), (false, de)] {
                if let Some(xe) = exponent(b, fe) {
                    let mut r = xe;
                    let rr = (b.get(r) == Some(&b'r')).then_some(r);
                    if rr.is_some() {
                        r += 1;
                    }
                    if b.get(r) == Some(&b'i') {
                        let i = r;
                        r += 1;
                        let bad = (b.get(r) == Some(&b'r')).then_some(r);
                        let e = bad.map_or(r, |x| x + 1);
                        if ends(e) {
                            add(&mut m, p, de, img(0));
                            if frac {
                                add(&mut m, de, de + 1, img(0) + 16);
                                add(&mut m, de + 1, xe, img(0));
                            } else {
                                add(&mut m, de, xe, img(0));
                            }
                            if let Some(x) = rr {
                                add(&mut m, x, x + 1, img(0) + 32);
                            }
                            add(&mut m, i, i + 1, img(0) + 24);
                            if let Some(x) = bad {
                                add(&mut m, x, x + 1, img(0) + 32);
                            }
                            m.end = e;
                            return Some(m);
                        }
                    }
                }
                if !frac {
                    break;
                }
            }
        }
    }
    // rational, based: `(r)\b`
    for f in forms.iter().flatten() {
        let (ds, de, base) = *f;
        if b.get(de) == Some(&b'r') && ends(de + 1) {
            add(&mut m, p, ds, rat(base) + 8);
            add(&mut m, ds, de, rat(base));
            add(&mut m, de, de + 1, rat(base) + 24);
            m.end = de + 1;
            return Some(m);
        }
    }
    // rational, decimal
    if let Some(de) = dec_base
        && b.get(de) == Some(&b'r')
        && ends(de + 1)
    {
        add(&mut m, p, p + 2, rat(0) + 8);
        add(&mut m, p + 2, de, rat(0));
        add(&mut m, de, de + 1, rat(0) + 24);
        m.end = de + 1;
        return Some(m);
    }
    if let Some(de) = dd {
        if b.get(de) == Some(&b'r') && ends(de + 1) {
            add(&mut m, p, de, rat(0));
            add(&mut m, de, de + 1, rat(0) + 24);
            m.end = de + 1;
            return Some(m);
        }
        if b.get(de) == Some(&b'.')
            && let Some(fe) = digits(b, de + 1, is_dec)
            && b.get(fe) == Some(&b'r')
            && ends(fe + 1)
        {
            add(&mut m, p, de, rat(0));
            add(&mut m, de, de + 1, rat(0) + 16);
            add(&mut m, de + 1, fe, rat(0));
            add(&mut m, fe, fe + 1, rat(0) + 24);
            m.end = fe + 1;
            return Some(m);
        }
        // float: `ddigits(?:(\.)ddigits|(?:(\.)ddigits)?exponent(r)?)\b`
        if b.get(de) == Some(&b'.')
            && let Some(fe) = digits(b, de + 1, is_dec)
        {
            if let Some(xe) = exponent(b, fe) {
                let r = (b.get(xe) == Some(&b'r')).then_some(xe);
                let e = r.map_or(xe, |x| x + 1);
                if ends(e) {
                    add(&mut m, p, de, FLOAT);
                    add(&mut m, de, de + 1, FLOAT_DOT);
                    add(&mut m, de + 1, xe, FLOAT);
                    if let Some(x) = r {
                        add(&mut m, x, x + 1, FLOAT_BAD);
                    }
                    m.end = e;
                    return Some(m);
                }
            }
            if ends(fe) {
                add(&mut m, p, de, FLOAT);
                add(&mut m, de, de + 1, FLOAT_DOT);
                add(&mut m, de + 1, fe, FLOAT);
                m.end = fe;
                return Some(m);
            }
        }
        if let Some(xe) = exponent(b, de) {
            let r = (b.get(xe) == Some(&b'r')).then_some(xe);
            let e = r.map_or(xe, |x| x + 1);
            if ends(e) {
                add(&mut m, p, xe, FLOAT);
                if let Some(x) = r {
                    add(&mut m, x, x + 1, FLOAT_BAD);
                }
                m.end = e;
                return Some(m);
            }
        }
    }
    // integers, based
    let int_kind = [HEX, OCTAL, BINARY];
    for f in forms.iter().flatten() {
        let (ds, de, base) = *f;
        if ends(de) {
            let k = int_kind[usize::from(base - 1)];
            add(&mut m, p, ds, k + 1);
            add(&mut m, ds, de, k);
            m.end = de;
            return Some(m);
        }
    }
    if let Some(de) = dec_base
        && ends(de)
    {
        add(&mut m, p, p + 2, DECIMAL_BASE);
        add(&mut m, p + 2, de, DECIMAL);
        m.end = de;
        return Some(m);
    }
    if let Some(de) = dd
        && ends(de)
    {
        add(&mut m, p, de, DECIMAL);
        m.end = de;
        return Some(m);
    }
    None
}

/// `[Ee][-+]?ddigits` at `p`: where it ends.
fn exponent(b: &[u8], p: usize) -> Option<usize> {
    if !matches!(b.get(p), Some(b'e' | b'E')) {
        return None;
    }
    let mut q = p + 1;
    if matches!(b.get(q), Some(b'-' | b'+')) {
        q += 1;
    }
    digits(b, q, is_dec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Lang;
    use crate::tests::marked;

    fn of(lines: &[&str]) -> Vec<(String, String)> {
        marked(Lang::Ruby, lines)
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

    fn lacks(lines: &[&str], text: &str) {
        let got = of(lines);
        assert!(!got.iter().any(|(t, _)| t == text), "{text:?} in {got:#?}");
    }

    #[test]
    fn declarations() {
        let l = [
            "class Foo < Bar::Baz",
            "  module A::B",
            "  class << self",
            "    def self.find(a, b = f(1, 2), *c, d:, e: [1, 2], **o, &blk)",
            "    def ==(other) = other",
            "    def bare a, b = 1",
            "  FOO, BAR = 1, 2",
        ];
        has(
            &l,
            "class",
            "storage.type.class.ruby keyword.declaration.class.ruby",
        );
        has(&l, "Foo", "entity.name.class.ruby");
        has(&l, "<", "punctuation.separator.inheritance.ruby");
        lacks(&l, "Bar");
        has(&l, "A", "entity.name.namespace.ruby");
        has(&l, "<<", "keyword.operator.assignment.augmented.ruby");
        has(&l, "self", "variable.language.ruby");
        has(&l, "find", "entity.name.function.ruby");
        has(&l, "==", "entity.name.function.ruby");
        has(&l, "bare", "entity.name.function.ruby");
        for p in ["a", "b", "c", "d", "e", "o", "blk"] {
            has(&l, p, "variable.parameter.ruby");
        }
        has(&l, "*", "keyword.operator.splat.ruby");
        has(&l, "**", "keyword.operator.splat.ruby");
        has(&l, "&", "keyword.operator.ruby");
        has(&l, ":", "punctuation.separator.ruby");
        // a default value's brackets nest: `f(1, 2)`'s `,` is code's
        has(&l, ",", "punctuation.separator.sequence.ruby");
        has(&l, "=", "punctuation.operator.assignment.ruby");
        has(&l, "FOO", "entity.name.constant.ruby");
        has(&l, "BAR", "entity.name.constant.ruby");
    }

    #[test]
    fn strings_interpolations_and_here_documents() {
        let l = [
            "puts \"a #{b + \"c#{d}\"} e\\n %s\", 'f\\'g', %w[a [b] c], :\"s#{t}\"",
            "x = foo(<<~A, <<-'B')",
            "  a #{1}",
            "A",
            "  b #{2}",
            "  B",
            "y = <<~RUBY.strip",
            "  def x(a) = \"#{y}\" # c",
            "  \\n #{z}",
            "RUBY",
            "done",
        ];
        has(&l, "#{", "punctuation.section.interpolation.begin.ruby");
        has(&l, "}", "punctuation.section.interpolation.end.ruby");
        has(&l, "\\n", "constant.character.escape.ruby");
        has(&l, "%s", "constant.other.placeholder.ruby");
        has(&l, "\\'", "constant.character.escape.ruby");
        has(&l, "[", "punctuation.section.scope.ruby");
        has(
            &l,
            "<<~A",
            "string.unquoted.heredoc.ruby punctuation.definition.string.begin.ruby",
        );
        // bodies in their openers' order (bat reads `B` over `A`): `A`'s, then `B`'s
        has(
            &l,
            "A",
            "string.unquoted.heredoc.ruby punctuation.definition.string.end.ruby",
        );
        has(
            &l,
            "B",
            "string.unquoted.heredoc.ruby punctuation.definition.string.end.ruby",
        );
        has(&l, "  b #{2}", "string.unquoted.heredoc.ruby");
        // a RUBY body is Ruby, its `#{` an interpolation, its escapes escapes
        has(
            &l,
            "def",
            "storage.type.function.ruby keyword.declaration.function.ruby",
        );
        has(&l, " c", "comment.line.number-sign.ruby");
        has(
            &l,
            "RUBY",
            "string.unquoted.heredoc.ruby punctuation.definition.string.end.ruby",
        );
        lacks(&l, "done");
    }

    #[test]
    fn operators_numbers_regexes_and_symbols() {
        let l = [
            "x = y / 2 / 3; z = [1, /re+/i, foo /x]",
            "a =~ %r{b/c}",
            "v = a ? b : c; w = h ? k: 2; ?a + ?\\n",
            "n = 1_000 + 0x1F + 0b10 + 017 + 1.5 + 1e3 + 2r + 3i",
            "h = { key: 1, :sym => @iv, \"s\": $1, other: @@cv }",
            "Foo::Bar.new(ENV['HOME']).baz x.class",
        ];
        has(&l, "/", "keyword.operator.arithmetic.ruby");
        has(&l, "re", "string.regexp.classic.ruby");
        has(
            &l,
            "+",
            "string.regexp.classic.ruby keyword.operator.quantifier.regexp",
        );
        has(&l, "i", "string.regexp.classic.ruby keyword.other.ruby");
        has(&l, "b/c", "string.regexp.mod-r.ruby");
        has(&l, "?", "keyword.operator.conditional.ruby");
        has(&l, "a", "constant.character.ruby");
        has(&l, "\\n", "constant.character.ruby");
        has(&l, "1_000", "constant.numeric.integer.decimal.ruby");
        has(&l, "1F", "constant.numeric.integer.hexadecimal.ruby");
        has(&l, "17", "constant.numeric.integer.octal.ruby");
        has(
            &l,
            ".",
            "constant.numeric.float.decimal.ruby punctuation.separator.decimal.ruby",
        );
        has(&l, "1e3", "constant.numeric.float.decimal.ruby");
        has(
            &l,
            "r",
            "constant.numeric.rational.decimal.ruby storage.type.numeric.ruby",
        );
        has(&l, "key", "constant.other.symbol.ruby");
        has(&l, "sym", "constant.other.symbol.ruby");
        has(&l, "iv", "variable.other.readwrite.instance.ruby");
        has(&l, "1", "variable.other.readwrite.global.pre-defined.ruby");
        has(&l, "cv", "variable.other.readwrite.class.ruby");
        has(&l, "ENV", "variable.other.constant.ruby");
        has(&l, "new", "keyword.other.special-method.ruby");
        has(&l, "class", "support.function.builtin.ruby");
        // a constant before `.` or `::` is bat's `support.class`, which no theme colors
        lacks(&l, "Foo");
    }

    #[test]
    fn sql_here_documents() {
        let l = [
            "rows = execute <<~SQL",
            "  SELECT id FROM users -- c",
            "  WHERE name = 'a#{x}b' /* d",
            "  */ ORDER BY 1",
            "SQL",
            "q = <<-'SQL'",
            "  SELECT #{nope}",
            "  SQL",
        ];
        has(&l, "SELECT", "keyword.other.DML.sql");
        has(&l, " c", "comment.line.double-dash.sql");
        // Ruby interpolates in SQL's strings too (bat's SQL string hides the `#{`)
        has(&l, "#{", "punctuation.section.interpolation.begin.ruby");
        has(&l, "ORDER BY", "keyword.other.DML.sql");
        lacks(&l, "nope");
        has(
            &l,
            "SQL",
            "string.unquoted.heredoc.ruby punctuation.definition.string.end.ruby",
        );
    }

    #[test]
    fn block_parameters() {
        let l = [
            "items.each do |x, y = f(1, 2)|",
            "end",
            "items.map {",
            "",
            "  |a, *b, &c| a }",
            "x || y",
        ];
        has(&l, "|", "punctuation.definition.parameters.begin.ruby");
        has(&l, "|", "punctuation.definition.parameters.end.ruby");
        for p in ["x", "y", "a", "b", "c"] {
            has(&l, p, "variable.parameter.ruby");
        }
        has(&l, "||", "keyword.operator.logical.ruby");
    }

    /// `text` a line at a time on the line API: each line's spans and the state after it.
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
        // every field a line hands the next: frames past those kept (strings in interpolations),
        // here-documents queued past the queue, their bases, a doc comment, parameters and a
        // block's across lines, `%` literals' nesting
        let deep = "\"#{".repeat(30);
        let text = format!(
            "y = f(<<~A, <<B, <<C) do\n  a #{{ \"b\n  c\" }}\nA\nb\nB\nC\n\
             |p, q = [1,\n 2]|\nend\ndef g(a,\n  (b, c),\n  d = {{ e: 1,\n f: 2 }})\n=begin\n=end x\n\
             %w[a [b\nc] d]\ns = <<~RUBY\n  def h(\n    i)\n  #{{ j\n }}\nRUBY\n\
             x = [<<~RUBY, \"a #{{\n}}\" b\nRUBY\ny = 1\nk\n\
             x = {deep}\n}}\" }}\"\n<<~Z\n"
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
        // everything before the deep line ends where it should
        let k = text.lines().position(|l| l == "k").unwrap();
        assert_eq!(one[k].1.arg & 3, u16::from(M_CODE));
        assert_eq!(one[k].1.depth, 0);
        assert!(!one[k + 1].1.is_exact());
    }

    #[test]
    fn a_block_lexes_as_its_lines_with_regexes() {
        // a regex's three frame bytes across lines (its groups and classes open, its nesting),
        // under interpolations, and cut past the frames kept
        let deep = "%r{(#{".repeat(20);
        let text = format!(
            "r = %r{{\n  (a{{2}}  # x\n  [b\n  #]c)\n}}x\nq = %r(a(b\n(c)#{{d\n}}e)f)i\n\
             x = {deep}\n}}}}\n}})\ny = /a/\n"
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
    fn an_interpolation_whose_string_was_cut() {
        // three regexes' and five strings' interpolations, 27 frame bytes: the 20 kept start at the
        // second regex's interpolation, whose `}` then has no string under it
        let text = format!(
            "x = {}{}\n{}}}}}}}\n",
            "%r{#{".repeat(3),
            "\"#{".repeat(5),
            "}\"".repeat(5)
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
        assert!(!one[0].1.is_exact());
    }

    #[test]
    fn regex_literals_on_the_regex_scanner() {
        let l = [
            r"DATE = /\A(?<year>\d{4})-(?'m'\d\d)\k<year>\g<m>\z/i",
            r"c = /[a-z&&[^aeiou]][[:alpha:]]\h+\p{^Greek}(?~x)(?#n)/",
            r"g = %r{^/users/(\d+)\.(json|xml)$}, %r[[\d]+], %r!a|b!x",
            r"k = /a#{b}c#@d[#{x}]e{2,}/",
            "l = %r{",
            "  (\\d+)  # digits",
            "  [#a]",
            "}x",
            r"u = /a # b/; v = %r|a\|b|",
        ];
        let re = |leaf: &str| format!("string.regexp.classic.ruby {leaf}");
        let rr = |leaf: &str| format!("string.regexp.mod-r.ruby {leaf}");
        has(&l, "\\A", &re("keyword.control.anchor.regexp"));
        has(&l, "year", &re("entity.name.other.group.regexp"));
        has(&l, "m", &re("entity.name.other.group.regexp"));
        has(&l, "\\k", &re("keyword.other.back-reference.regexp"));
        has(&l, "\\g", &re("keyword.other.back-reference.regexp"));
        has(
            &l,
            "4",
            &re("keyword.operator.quantifier.regexp constant.literal.numeric.regexp"),
        );
        has(&l, "i", "string.regexp.classic.ruby keyword.other.ruby");
        has(
            &l,
            "&&",
            &re("constant.other.character-class.set.regexp keyword.operator.intersection.regexp"),
        );
        has(
            &l,
            "[:alpha:]",
            &re(
                "constant.other.character-class.set.regexp constant.other.character-class.posix.regexp",
            ),
        );
        has(&l, "?~", &re("punctuation.definition.group.absent.regexp"));
        has(&l, "n", &re("comment.block.regexp"));
        // `%r…`: Ruby's lexer nests its brackets wherever they stand
        has(&l, "/users/", "string.regexp.mod-r.ruby");
        has(&l, "|", &rr("keyword.operator.or.regexp"));
        has(
            &l,
            "]",
            "string.regexp.mod-r.ruby punctuation.definition.string.end.ruby",
        );
        has(
            &l,
            "!x",
            "string.regexp.mod-r.ruby punctuation.definition.string.end.ruby",
        );
        has(&l, "\\|", &rr("constant.character.escape.backslash.regexp"));
        // interpolations and variables are Ruby's, in a class too
        has(&l, "#{", "punctuation.section.interpolation.begin.ruby");
        has(&l, "d", "variable.other.readwrite.instance.ruby");
        has(
            &l,
            "[",
            &re(
                "constant.other.character-class.set.regexp punctuation.definition.character-class.begin.regexp",
            ),
        );
        has(&l, "{", &re("keyword.operator.quantifier.regexp"));
        // the extended syntax's comments where the flags hold `x` or the body spans lines; none
        // in `/a # b/`
        has(&l, "# digits", &rr("comment.line.number-sign.regexp"));
        has(&l, "#a", &rr("constant.other.character-class.set.regexp"));
        has(&l, "a # b", "string.regexp.classic.ruby");
    }

    #[test]
    fn long_lines_lex_in_linear_time() {
        for unit in [
            "a(",
            "{ ",
            "\"#{",
            "#{",
            "/a",
            "x / ",
            "a = /",
            "?a:",
            "?ab",
            "A, ",
            "A ",
            "<<A ",
            "%w[",
            "%r{(",
            "/#{a}",
            "%r{#{a}",
            "%r{(#@ ",
            "%r{[#",
            "%r{a # ",
            "def f(",
            "def f a, ",
            "{ |",
            "do |a = (",
            ":\"",
            "1.",
            "ENV[",
            "x.y",
            "else ",
        ] {
            let l = unit.repeat(20_000);
            let t = std::time::Instant::now();
            lines(&l);
            assert!(t.elapsed().as_millis() < 200, "{unit:?}: {:?}", t.elapsed());
        }
    }
}
