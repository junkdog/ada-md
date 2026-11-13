//! Go on a scanner of ada's own (X-168), from a floor probe (the same files built with
//! `ADA_PARSE_X168_SOL`): one loop over a line's bytes, a byte's action one load from a table
//! ([`ACT`]), a name read once and looked up once in one table of every word ([`WORDS`]: its leaf
//! and, for the words whose meaning hangs on where they stand, an action), the previous
//! significant token in a byte ([`Scan::prev`]), and frames only for what nests ([`Scan::frames`],
//! five bits a level): braces, parentheses, brackets, a struct's and an interface's body, a
//! signature's parameters and results, type parameters and arguments, a map's key, an array's
//! length, the `var`, `const` and `type` groups, a SQL call's arguments and a type switch's body.
//! What a name is where it stands (a type, a declared name, a parameter, a field) is a byte of the
//! line's ([`Scan::role`]), set by the token before it and at a line's start by the frame the line
//! is in. A raw string and a block comment continued past a line's end are a mode
//! ([`Scan::mode`]). Kinds are bat's leaves.
//!
//! Go's reading where bat's grammar has none or derails: type parameters and arguments
//! (`func Map[T any](…)`, `List[T]`, a receiver's `[T]`, `~int | ~string` constraints); `any`,
//! `comparable`, `min`, `max` and `clear` predeclared; a `[` after a value an index, so
//! `a[0] * b` multiplies; a `"…"` string ends with its line; a template's quotes in a `"…"`
//! string are `\"` and the template ends with its string, so an escaped quote in `{{ … }}`
//! doesn't run the template on to the file's end; fmt's verbs as fmt reads them (`%a`, `%z` and
//! `% o` in "50% off" aren't); an escape Go doesn't have is invalid; a type switch's cases are
//! types; operators by Go's longest match (`&^=`, `<<=`). SQL by call site and marker
//! ([`crate::sql`]: `database/sql`'s, pgx's and sqlx's calls, `// language=SQL`) and regexes in
//! `regexp`'s calls ([`crate::regex`], RE2) as before X-168.

#[cfg(test)]
mod tests;
mod text;

use crate::emit::{Rules, Run, Runs, Sink, Spans};
use crate::prim::{WORD, char_len, is_name_start, word_end};
use crate::regex;
use crate::sql;
use crate::words::{Words, text_len};
use crate::{Out, State};

/// The floor probe (X-168's `sol`): every word its table leaf, a call and a member their own, no
/// declaration, signature, type, field, SQL, regex or template read.
const SOL: bool = option_env!("ADA_PARSE_X168_SOL").is_some();

/// The leaves: a token's scopes (the compiled scanner's, then an invalid escape's in a string and
/// in a rune).
static LEAVES: [&str; 107] = [
    "",
    // comments
    "comment.line.go",
    "comment.line.go punctuation.definition.comment.go",
    "comment.line.go meta.annotation.go",
    "comment.line.go meta.annotation.go punctuation.definition.comment.go",
    "comment.line.go meta.annotation.go meta.keyword.annotation.go",
    "comment.line.go meta.annotation.go meta.punctuation.accessor.colon.go",
    "comment.line.go meta.annotation.go meta.variable.function.go",
    "comment.line.go meta.annotation.go meta.variable.parameter.go",
    "comment.block.go",
    "comment.block.go punctuation.definition.comment.begin.go",
    "comment.block.go punctuation.definition.comment.end.go",
    "comment.block.go punctuation.definition.comment.go",
    // strings: the text, then BEGIN, END, ESCAPE, PLACEHOLDER
    "string.quoted.double.go",
    "string.quoted.double.go punctuation.definition.string.begin.go",
    "string.quoted.double.go punctuation.definition.string.end.go",
    "string.quoted.double.go constant.character.escape.go",
    "string.quoted.double.go constant.other.placeholder.go",
    "string.quoted.other.go",
    "string.quoted.other.go punctuation.definition.string.begin.go",
    "string.quoted.other.go punctuation.definition.string.end.go",
    "string.quoted.other.go constant.character.escape.go",
    "string.quoted.other.go constant.other.placeholder.go",
    // a template's interpolation
    "meta.interpolation.go",
    "meta.interpolation.go punctuation.section.interpolation.begin.go",
    "meta.interpolation.go punctuation.section.interpolation.end.go",
    "meta.interpolation.go keyword.operator.template.right.trim.go",
    "meta.interpolation.go keyword.operator.template.left.trim.go",
    "meta.interpolation.go keyword.operator.assignment.go",
    "meta.interpolation.go keyword.operator.template.pipe.go",
    "meta.interpolation.go punctuation.accessor.dot.go",
    "meta.interpolation.go variable.other.member.go",
    "meta.interpolation.go variable.other.template.go",
    "meta.interpolation.go variable.other.template.go punctuation.definition.variable.go",
    "meta.interpolation.go keyword.control.go",
    "meta.interpolation.go variable.function.go support.function.builtin.go",
    // keywords
    "keyword.control.go",
    "keyword.other.go",
    "keyword.other.import.go",
    "keyword.other.package.go",
    "storage.type.keyword.const.go",
    "storage.type.keyword.function.go",
    "storage.type.keyword.type.go",
    "storage.type.keyword.var.go",
    "storage.type.keyword.chan.go",
    "storage.type.keyword.struct.go",
    "storage.type.keyword.interface.go",
    "storage.type.keyword.map.go",
    // names
    "constant.language.go",
    "variable.function.go",
    "variable.function.go support.function.builtin.go",
    "variable.function.go support.type.builtin.go",
    "variable.language.blank.go",
    "variable.other.go",
    "variable.declaration.go",
    "variable.other.member.go",
    "variable.other.constant.declaration.go",
    "variable.parameter.go",
    "variable.other.member.declaration.go",
    "entity.name.function.go",
    "entity.name.type.go",
    "entity.other.inherited-class.go",
    "entity.other.inherited-class.go support.type.builtin.go",
    "storage.type.go",
    "storage.type.go support.type.builtin.go",
    // operators and punctuation
    "keyword.operator.go",
    "keyword.operator.assignment.go",
    "keyword.operator.variadic.go",
    "punctuation.separator.go",
    "punctuation.terminator.go",
    "punctuation.accessor.dot.go",
    "punctuation.section.parens.begin.go",
    "punctuation.section.parens.end.go",
    "punctuation.section.brackets.begin.go",
    "punctuation.section.brackets.end.go",
    "punctuation.section.braces.begin.go",
    "punctuation.section.braces.end.go",
    // numbers: each kind, then its captures
    "constant.numeric.imaginary.decimal.go",
    "constant.numeric.imaginary.decimal.go punctuation.separator.decimal.go",
    "constant.numeric.imaginary.decimal.go storage.type.numeric.go",
    "constant.numeric.imaginary.hexadecimal.go",
    "constant.numeric.imaginary.hexadecimal.go punctuation.definition.numeric.base.go",
    "constant.numeric.imaginary.hexadecimal.go punctuation.separator.decimal.go",
    "constant.numeric.imaginary.hexadecimal.go storage.type.numeric.go",
    "constant.numeric.imaginary.octal.go",
    "constant.numeric.imaginary.octal.go punctuation.definition.numeric.base.go",
    "constant.numeric.imaginary.octal.go storage.type.numeric.go",
    "constant.numeric.imaginary.binary.go",
    "constant.numeric.imaginary.binary.go punctuation.definition.numeric.base.go",
    "constant.numeric.imaginary.binary.go storage.type.numeric.go",
    "constant.numeric.float.decimal.go",
    "constant.numeric.float.decimal.go punctuation.separator.decimal.go",
    "constant.numeric.float.hexadecimal.go",
    "constant.numeric.float.hexadecimal.go punctuation.definition.numeric.base.go",
    "constant.numeric.float.hexadecimal.go punctuation.separator.decimal.go",
    "constant.numeric.integer.octal.go",
    "constant.numeric.integer.octal.go punctuation.definition.numeric.base.go",
    "constant.numeric.integer.hexadecimal.go",
    "constant.numeric.integer.hexadecimal.go punctuation.definition.numeric.base.go",
    "constant.numeric.integer.binary.go",
    "constant.numeric.integer.binary.go punctuation.definition.numeric.base.go",
    "constant.numeric.integer.decimal.go",
    "invalid.illegal.go",
    "constant.character.go",
    "constant.character.go constant.character.escape.go",
    // an escape Go doesn't have
    "string.quoted.double.go invalid.illegal.unknown-escape.go",
    "constant.character.go invalid.illegal.unknown-escape.go",
];

// The leaves by name, in the order of `LEAVES`.
const PLAIN: u16 = 0;
const LINE_COMMENT: u16 = 1;
const LINE_MARK: u16 = 2;
const ANNOTATION: u16 = 3;
const ANN_MARK: u16 = 4;
const ANN_KEYWORD: u16 = 5;
const ANN_COLON: u16 = 6;
const ANN_FUNCTION: u16 = 7;
const ANN_PARAMETER: u16 = 8;
const COMMENT: u16 = 9;
const COMMENT_BEGIN: u16 = 10;
const COMMENT_END: u16 = 11;
const COMMENT_STAR: u16 = 12;
/// A string's leaves: its text, then these after it.
const DOUBLE: u16 = 13;
const OTHER: u16 = 18;
const BEGIN: u16 = 1;
const END: u16 = 2;
const ESCAPE: u16 = 3;
const PLACEHOLDER: u16 = 4;
const INTERP_TEXT: u16 = 23;
const INTERP_BEGIN: u16 = 24;
const INTERP_END: u16 = 25;
const RIGHT_TRIM: u16 = 26;
const LEFT_TRIM: u16 = 27;
const INTERP_ASSIGN: u16 = 28;
const PIPE: u16 = 29;
const INTERP_DOT: u16 = 30;
const INTERP_MEMBER: u16 = 31;
const TEMPLATE_VAR: u16 = 32;
const TEMPLATE_VAR_MARK: u16 = 33;
const INTERP_KEYWORD: u16 = 34;
const INTERP_FUNCTION: u16 = 35;
const CONTROL: u16 = 36;
const OTHER_KEYWORD: u16 = 37;
const IMPORT: u16 = 38;
const PACKAGE: u16 = 39;
const CONST_KEYWORD: u16 = 40;
const FUNC_KEYWORD: u16 = 41;
const TYPE_KEYWORD: u16 = 42;
const VAR_KEYWORD: u16 = 43;
const CHAN_KEYWORD: u16 = 44;
const STRUCT_KEYWORD: u16 = 45;
const INTERFACE_KEYWORD: u16 = 46;
const MAP_KEYWORD: u16 = 47;
const LANGUAGE: u16 = 48;
const FUNCTION: u16 = 49;
const BUILTIN_FUNCTION: u16 = 50;
const BUILTIN_CAST: u16 = 51;
const BLANK: u16 = 52;
const VARIABLE: u16 = 53;
const DECLARATION: u16 = 54;
const MEMBER: u16 = 55;
const CONSTANT_DECLARATION: u16 = 56;
const PARAMETER: u16 = 57;
const MEMBER_DECLARATION: u16 = 58;
const FUNCTION_NAME: u16 = 59;
const TYPE_NAME: u16 = 60;
const INHERITED: u16 = 61;
const INHERITED_BUILTIN: u16 = 62;
const STORAGE: u16 = 63;
const STORAGE_BUILTIN: u16 = 64;
const OPERATOR: u16 = 65;
const ASSIGNMENT: u16 = 66;
const VARIADIC: u16 = 67;
const SEPARATOR: u16 = 68;
const TERMINATOR: u16 = 69;
const DOT: u16 = 70;
const PARENS_BEGIN: u16 = 71;
const PARENS_END: u16 = 72;
const BRACKETS_BEGIN: u16 = 73;
const BRACKETS_END: u16 = 74;
const BRACES_BEGIN: u16 = 75;
const BRACES_END: u16 = 76;
const IMAG_DEC: u16 = 77;
const IMAG_HEX: u16 = 80;
const IMAG_OCT: u16 = 84;
const IMAG_BIN: u16 = 87;
const FLOAT_DEC: u16 = 90;
const FLOAT_HEX: u16 = 92;
const INT_OCT: u16 = 95;
const INT_HEX: u16 = 97;
const INT_BIN: u16 = 99;
const INT_DEC: u16 = 101;
const INVALID: u16 = 102;
const RUNE: u16 = 103;
const RUNE_ESCAPE: u16 = 104;
const STRING_INVALID: u16 = 105;
const RUNE_INVALID: u16 = 106;

/// The kinds of a regex in a `"…"` string and in a `` `…` `` one: the regex's, from each.
const RE_STRING: u16 = 128;
const RE_RAW: u16 = RE_STRING + regex::KINDS as u16;
const _: () = assert!(LEAVES.len() <= RE_STRING as usize);
/// SQL in a `"…"` string, then in a `` `…` `` one: SQL's kinds from each.
const SQL_STRING: u16 = RE_RAW + regex::KINDS as u16;
const SQL_RAW: u16 = SQL_STRING + sql::KINDS as u16;

/// The scopes under `source.go` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    if kind >= SQL_STRING {
        let (text, base) = if kind >= SQL_RAW {
            (OTHER, SQL_RAW)
        } else {
            (DOUBLE, SQL_STRING)
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
            (OTHER, RE_RAW)
        } else {
            (DOUBLE, RE_STRING)
        };
        let mut out = scopes(text);
        out.extend(regex::scopes(kind - base));
        return out;
    }
    LEAVES.get(usize::from(kind)).map_or_else(Vec::new, |l| {
        l.split(' ').filter(|s| !s.is_empty()).collect()
    })
}

// A byte's action ([`ACT`]).
const A_OTHER: u8 = 0;
const A_SPACE: u8 = 1;
const A_NAME: u8 = 2;
const A_DIGIT: u8 = 3;
const A_DQUOTE: u8 = 4;
const A_BQUOTE: u8 = 5;
const A_SQUOTE: u8 = 6;
const A_SLASH: u8 = 7;
const A_DOT: u8 = 8;
/// `+-*%&|^<>=!:~`: an operator.
const A_OP: u8 = 9;
const A_LPAREN: u8 = 10;
const A_RPAREN: u8 = 11;
const A_LBRACE: u8 = 12;
const A_RBRACE: u8 = 13;
const A_LBRACKET: u8 = 14;
const A_RBRACKET: u8 = 15;
const A_COMMA: u8 = 16;
const A_SEMI: u8 = 17;
/// A byte past ASCII: a name's letter, or another.
const A_WIDE: u8 = 18;

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
            b'`' => A_BQUOTE,
            b'\'' => A_SQUOTE,
            b'/' => A_SLASH,
            b'.' => A_DOT,
            b'+' | b'-' | b'*' | b'%' | b'&' | b'|' | b'^' | b'<' | b'>' | b'=' | b'!' | b':'
            | b'~' => A_OP,
            b'(' => A_LPAREN,
            b')' => A_RPAREN,
            b'{' => A_LBRACE,
            b'}' => A_RBRACE,
            b'[' => A_LBRACKET,
            b']' => A_RBRACKET,
            b',' => A_COMMA,
            b';' => A_SEMI,
            _ => A_OTHER,
        };
        c += 1;
    }
    t
};

// What a word is ([`WORDS`], [`INFO`]): its leaf in the low byte, an action in the next five
// bits, and [`W_SQL`].
const W_ACTION: u16 = 0x1f << 8;
/// A keyword whose leaf is all there is: `prev` an operator's.
const W_KEYWORD: u16 = 1 << 8;
/// `case`: in a type switch, types follow.
const W_CASE: u16 = 2 << 8;
/// `func`: a declaration's head at a file's statement, else a signature.
const W_FUNC: u16 = 3 << 8;
/// `type`: a declaration's head.
const W_TYPE: u16 = 4 << 8;
/// `var`: names, then a type or a value.
const W_VAR: u16 = 5 << 8;
/// `const`: as `var`.
const W_CONST: u16 = 6 << 8;
/// `struct`: its `{` opens fields.
const W_STRUCT: u16 = 7 << 8;
/// `interface`: its `{` opens methods and types.
const W_IFACE: u16 = 8 << 8;
/// `map`: its `[` opens the key's type.
const W_MAP: u16 = 9 << 8;
/// `chan`: a type follows.
const W_CHAN: u16 = 10 << 8;
/// A predeclared type: its leaf as a type, a conversion's before `(`.
const W_BTYPE: u16 = 11 << 8;
/// A predeclared function: its leaf before `(` only.
const W_BFUNC: u16 = 12 << 8;
/// `make`, `new`: a predeclared function whose first argument is a type.
const W_MAKE: u16 = 13 << 8;
/// A method whose arguments hold SQL ([`text::sql_call`]): the `(` after it looks back.
const W_SQL: u16 = 1 << 15;

/// The words, each with what it is.
const WORD_LIST: &[(&[u8], u16)] = &[
    (b"break", W_KEYWORD | CONTROL),
    (b"case", W_CASE | CONTROL),
    (b"continue", W_KEYWORD | CONTROL),
    (b"default", W_KEYWORD | CONTROL),
    (b"defer", W_KEYWORD | CONTROL),
    (b"else", W_KEYWORD | CONTROL),
    (b"fallthrough", W_KEYWORD | CONTROL),
    (b"for", W_KEYWORD | CONTROL),
    (b"go", W_KEYWORD | CONTROL),
    (b"goto", W_KEYWORD | CONTROL),
    (b"if", W_KEYWORD | CONTROL),
    (b"return", W_KEYWORD | CONTROL),
    (b"select", W_KEYWORD | CONTROL),
    (b"switch", W_KEYWORD | CONTROL),
    (b"range", W_KEYWORD | OTHER_KEYWORD),
    (b"import", W_KEYWORD | IMPORT),
    (b"package", W_KEYWORD | PACKAGE),
    (b"func", W_FUNC | FUNC_KEYWORD),
    (b"type", W_TYPE | TYPE_KEYWORD),
    (b"var", W_VAR | VAR_KEYWORD),
    (b"const", W_CONST | CONST_KEYWORD),
    (b"struct", W_STRUCT | STRUCT_KEYWORD),
    (b"interface", W_IFACE | INTERFACE_KEYWORD),
    (b"map", W_MAP | MAP_KEYWORD),
    (b"chan", W_CHAN | CHAN_KEYWORD),
    // the predeclared constants, and `iota` (Go's spec: a constant wherever it isn't redeclared)
    (b"true", LANGUAGE),
    (b"false", LANGUAGE),
    (b"nil", LANGUAGE),
    (b"iota", INT_DEC),
    (b"_", BLANK),
    // the predeclared types, `any` and `comparable` with Go 1.18
    (b"bool", W_BTYPE),
    (b"byte", W_BTYPE),
    (b"complex64", W_BTYPE),
    (b"complex128", W_BTYPE),
    (b"error", W_BTYPE),
    (b"float32", W_BTYPE),
    (b"float64", W_BTYPE),
    (b"int", W_BTYPE),
    (b"int8", W_BTYPE),
    (b"int16", W_BTYPE),
    (b"int32", W_BTYPE),
    (b"int64", W_BTYPE),
    (b"rune", W_BTYPE),
    (b"string", W_BTYPE),
    (b"uint", W_BTYPE),
    (b"uint8", W_BTYPE),
    (b"uint16", W_BTYPE),
    (b"uint32", W_BTYPE),
    (b"uint64", W_BTYPE),
    (b"uintptr", W_BTYPE),
    (b"any", W_BTYPE),
    (b"comparable", W_BTYPE),
    // the predeclared functions, `clear`, `min` and `max` with Go 1.21
    (b"append", W_BFUNC),
    (b"cap", W_BFUNC),
    (b"clear", W_BFUNC),
    (b"close", W_BFUNC),
    (b"complex", W_BFUNC),
    (b"copy", W_BFUNC),
    (b"delete", W_BFUNC),
    (b"imag", W_BFUNC),
    (b"len", W_BFUNC),
    (b"max", W_BFUNC),
    (b"min", W_BFUNC),
    (b"panic", W_BFUNC),
    (b"print", W_BFUNC),
    (b"println", W_BFUNC),
    (b"real", W_BFUNC),
    (b"recover", W_BFUNC),
    (b"make", W_MAKE),
    (b"new", W_MAKE),
    // SQL's calls ([`text::sql_call`])
    (b"Exec", W_SQL),
    (b"MustExec", W_SQL),
    (b"NamedExec", W_SQL),
    (b"Query", W_SQL),
    (b"NamedQuery", W_SQL),
    (b"QueryRow", W_SQL),
    (b"Queryx", W_SQL),
    (b"QueryRowx", W_SQL),
    (b"Preparex", W_SQL),
    (b"Queue", W_SQL),
    (b"Prepare", W_SQL),
    (b"Rebind", W_SQL),
    (b"PrepareNamed", W_SQL),
    (b"ExecParams", W_SQL),
    (b"Get", W_SQL),
    (b"Select", W_SQL),
    (b"GetContext", W_SQL),
    (b"SelectContext", W_SQL),
    (b"ExecContext", W_SQL),
    (b"QueryContext", W_SQL),
    (b"QueryRowContext", W_SQL),
    (b"PrepareContext", W_SQL),
    (b"MustExecContext", W_SQL),
    (b"NamedExecContext", W_SQL),
    (b"NamedQueryContext", W_SQL),
    (b"QueryxContext", W_SQL),
    (b"QueryRowxContext", W_SQL),
    (b"PreparexContext", W_SQL),
    (b"PrepareNamedContext", W_SQL),
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
static WORDS: Words<512, N, { text_len(&WORD_TEXT) }> = Words::new(&WORD_TEXT);
/// A word's kind and action by its id; 0 for none.
static INFO: [u16; N + 1] = {
    let mut t = [0u16; N + 1];
    let mut i = 0;
    while i < N {
        t[i + 1] = WORD_LIST[i].1;
        i += 1;
    }
    t
};

// The frames, five bits a level in [`Scan::frames`]; 0 is none (the top level, or a level lost
// past [`LEVELS`]).
/// `{` … `}`: statements, or a composite literal's elements.
const F_BRACE: u8 = 1;
/// `(` … `)` of code.
const F_PAREN: u8 = 2;
/// `[` … `]` of code: an index, a slice, a generic call's arguments.
const F_BRACKET: u8 = 3;
/// A struct's fields.
const F_STRUCT: u8 = 4;
/// An interface's methods and types.
const F_IFACE: u8 = 5;
/// A signature's parameters, named: a name stands at an entry's start.
const F_PARAMS: u8 = 6;
/// A signature's parameters, unnamed: types.
const F_UPARAMS: u8 = 7;
/// A signature's parameters, not yet known to be either (a list opened at a line's end).
const F_PLIST: u8 = 8;
/// A signature's results, named.
const F_RES: u8 = 9;
/// A signature's results, unnamed.
const F_URES: u8 = 10;
/// A signature's results, not yet known.
const F_RLIST: u8 = 11;
/// A method's receiver, named.
const F_RECV: u8 = 12;
/// A method's receiver, its type alone.
const F_URECV: u8 = 13;
/// A function's type parameters.
const F_TPARAMS_F: u8 = 14;
/// A type's type parameters.
const F_TPARAMS_T: u8 = 15;
/// A type's type arguments, `[` … `]`.
const F_TARGS: u8 = 16;
/// A map type's key, `[` … `]`: its value's type follows.
const F_MAPKEY: u8 = 17;
/// An array type's length, `[` … `]`: an expression; its element's type follows.
const F_ALEN: u8 = 18;
/// `(` … `)` around a type, or a type assertion's.
const F_TPAREN: u8 = 19;
/// `var (` … `)`: a spec a line.
const F_VARGROUP: u8 = 20;
/// `const (` … `)`.
const F_CONSTGROUP: u8 = 21;
/// `type (` … `)`.
const F_TYPEGROUP: u8 = 22;
/// A SQL call's arguments ([`Scan::sqls`] the argument's state).
const F_SQLCALL: u8 = 23;
/// A type switch's `{` … `}`: its cases are types.
const F_TSWITCH: u8 = 24;
/// The bits of a level.
const FW: u32 = 5;
const FMASK: u128 = (1 << FW) - 1;
/// The levels [`Scan::frames`] holds.
const LEVELS: u8 = 25;

// What a name is where it stands ([`Scan::role`]).
/// Code: a name, a call, a member.
const R_CODE: u8 = 0;
/// A type.
const R_TYPE: u8 = 1;
/// A type's name after its package's `.`.
const R_QUAL: u8 = 2;
/// After a type's name, before its `[`: type arguments.
const R_TARGS: u8 = 3;
/// After `map`: its `[` opens the key.
const R_MAP: u8 = 4;
/// After `struct`: its `{` opens fields.
const R_BODY_S: u8 = 5;
/// After `interface`: its `{` opens methods and types.
const R_BODY_I: u8 = 6;
/// A signature: `[` type parameters, `(` parameters.
const R_SIG: u8 = 7;
/// After a declaration's `func`: a receiver or the function's name.
const R_FNAME: u8 = 8;
/// After a receiver: the method's name.
const R_MNAME: u8 = 9;
/// After a signature's parameters: its results.
const R_RESULT: u8 = 10;
/// After `type`: the type's name, or a group.
const R_TNAME: u8 = 11;
/// After a type declaration's name: type parameters, `=`, the type.
const R_TDEF: u8 = 12;
/// After a type declaration's type parameters: `=` or the type.
const R_TDEF2: u8 = 13;
/// A `var`'s names, or a group.
const R_VNAME: u8 = 14;
/// A `const`'s names, or a group.
const R_CNAME: u8 = 15;
/// A parameter's name.
const R_PNAME: u8 = 16;
/// A type parameter's name.
const R_TPNAME: u8 = 17;
/// A struct's field's start: its names, or an embedded type.
const R_FIELD: u8 = 18;
/// An embedded field's type after its `*`.
const R_EMB: u8 = 19;
/// An embedded type's name after its package's `.`.
const R_EMBQ: u8 = 20;
/// An interface's element's start: a method, an embedded interface, a type set.
const R_ELEM: u8 = 21;
/// A short variable declaration's names, to its `:=`.
const R_DECLS: u8 = 22;
/// After `make`, `new`: its `(` opens a type.
const R_MAKE: u8 = 23;
/// A parameter list's line not yet known to be named or not.
const R_PLIST: u8 = 24;

// What the previous significant token was ([`Scan::prev`]).
/// A statement's start: a line's in a block, after `;`, `{`.
const P_START: u8 = 0;
/// An operator, `(`, `[`, `,`, a keyword: an operand may start.
const P_OP: u8 = 1;
/// A value's end: a name, a literal, `)`, `]`, `}`.
const P_VALUE: u8 = 2;
/// `.`: a member's name follows.
const P_DOT: u8 = 3;

// What a SQL call's argument is ([`Scan::sqls`]), as GoLand reads the calls.
/// Not SQL, nor any later argument.
const S_DONE: u8 = 0;
/// The first: SQL, or a context before it.
const S_LEAD: u8 = 1;
/// The first of `Prepare`: SQL, or a context before a statement's name.
const S_LEAD_NAME: u8 = 2;
/// A context, then SQL.
const S_CTX: u8 = 3;
/// A context, then a destination, then SQL.
const S_CTX_DEST: u8 = 4;
/// A destination (`&x`), then SQL.
const S_DEST: u8 = 5;
/// pgx's statement's name, then SQL.
const S_NAME: u8 = 6;
/// SQL.
const S_SQL: u8 = 7;
/// After the SQL string: a `+` goes on with SQL.
const S_AFTER: u8 = 8;
/// An argument to skip, SQL after it.
const S_SKIP_SQL: u8 = 9;
/// An argument to skip, a destination after it.
const S_SKIP_DEST: u8 = 10;
/// An argument to skip, a statement's name after it.
const S_SKIP_NAME: u8 = 11;
/// [`Scan::sqls`]' bit: the argument starts at the next token, on a later line.
const S_PENDING: u8 = 0x80;

// What a line starts in ([`Scan::mode`]).
const M_CODE: u8 = 0;
/// A block comment.
const M_COMMENT: u8 = 1;
/// A raw string.
const M_RAW: u8 = 2;
/// A raw string of SQL, SQL's context in [`Scan::ctx`].
const M_RAW_SQL: u8 = 3;
/// A raw string of a regex, the regex's state in [`Scan::ctx`].
const M_RAW_RE: u8 = 4;
/// [`State::arg`]'s bit beside the mode: a SQL marker waits for its string.
const MARKED: u16 = 8;

/// How far a parameter list's reading looks along the line for its first named entry.
const LOOK: usize = 512;

/// Lex `b`, a line of Go without its `\n`, from `state`.
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

/// A block of Go on one scan opened from `state` ([`crate::Carry`]).
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
    /// What each open frame is ([`F_BRACE`]…), five bits a level, the innermost lowest; past
    /// [`LEVELS`] the outer ones are lost.
    frames: u128,
    /// How many frames are open, up to 255.
    depth: u8,
    /// What the line starts in ([`M_CODE`]…).
    mode: u8,
    /// A raw string's SQL context ([`M_RAW_SQL`]) or regex state ([`M_RAW_RE`]).
    ctx: u8,
    /// The innermost SQL call's argument ([`S_DONE`]…, [`S_PENDING`]).
    sqls: u8,
    /// A SQL marker waits for its string past the line ([`MARKED`]).
    marked: bool,
    // The line's own, reset at its start:
    /// The previous significant token ([`P_START`]…).
    prev: u8,
    /// What a name is where the scan stands ([`R_CODE`]…).
    role: u8,
    /// The SQL call whose `(` comes next ([`S_LEAD`]…; [`S_DONE`] for none).
    sqlcall: u8,
    /// A type assertion's `.(type)` seen: the `{` after it opens a type switch.
    tswitch: bool,
    /// In a type switch's `case`, to its `:`: each of its `,`s starts a type.
    case_ty: bool,
    /// Where the last search for a short variable declaration's `:=` failed: the names before
    /// it don't search again ([`Scan::decl_follows`]).
    nodecl: usize,
    /// Where the line's next `:=` at or past the last name that asked is (`usize::MAX` for none),
    /// found once and again only past it ([`Scan::decl_follows`]).
    defs: usize,
    /// Where a SQL marker's string may open from (`usize::MAX` for none): the next string at or
    /// past it is SQL, unless a `;`, `,`, `)`, `{` or `}` comes between.
    mark: usize,
    /// What SQL's scan keeps between the host's turns on a line.
    sql: sql::Embedded,
    k: K,
    out: &'b mut Out<'c>,
}

/// What a frame's line starts in: the role, then the previous token.
static START: [(u8, u8); 32] = {
    let mut t = [(R_CODE, P_START); 32];
    t[F_PAREN as usize] = (R_CODE, P_OP);
    t[F_BRACKET as usize] = (R_CODE, P_OP);
    t[F_ALEN as usize] = (R_CODE, P_OP);
    t[F_SQLCALL as usize] = (R_CODE, P_OP);
    t[F_STRUCT as usize] = (R_FIELD, P_START);
    t[F_IFACE as usize] = (R_ELEM, P_START);
    t[F_PARAMS as usize] = (R_PNAME, P_OP);
    t[F_RES as usize] = (R_PNAME, P_OP);
    t[F_RECV as usize] = (R_PNAME, P_OP);
    t[F_UPARAMS as usize] = (R_TYPE, P_OP);
    t[F_URES as usize] = (R_TYPE, P_OP);
    t[F_URECV as usize] = (R_TYPE, P_OP);
    t[F_TARGS as usize] = (R_TYPE, P_OP);
    t[F_MAPKEY as usize] = (R_TYPE, P_OP);
    t[F_TPAREN as usize] = (R_TYPE, P_OP);
    t[F_PLIST as usize] = (R_PLIST, P_OP);
    t[F_RLIST as usize] = (R_PLIST, P_OP);
    t[F_TPARAMS_F as usize] = (R_TPNAME, P_OP);
    t[F_TPARAMS_T as usize] = (R_TPNAME, P_OP);
    t[F_VARGROUP as usize] = (R_VNAME, P_START);
    t[F_CONSTGROUP as usize] = (R_CNAME, P_START);
    t[F_TYPEGROUP as usize] = (R_TNAME, P_START);
    t
};

/// Whether frame `f` is closed by `)`.
fn is_paren_frame(f: u8) -> bool {
    matches!(
        f,
        F_PAREN
            | F_PARAMS
            | F_UPARAMS
            | F_PLIST
            | F_RES
            | F_URES
            | F_RLIST
            | F_RECV
            | F_URECV
            | F_TPAREN
            | F_VARGROUP
            | F_CONSTGROUP
            | F_TYPEGROUP
            | F_SQLCALL
    )
}

/// Whether frame `f` is closed by `]`.
fn is_bracket_frame(f: u8) -> bool {
    matches!(
        f,
        F_BRACKET | F_ALEN | F_TARGS | F_MAPKEY | F_TPARAMS_F | F_TPARAMS_T
    )
}

/// Whether frame `f` is closed by `}`.
fn is_brace_frame(f: u8) -> bool {
    matches!(f, F_BRACE | F_STRUCT | F_IFACE | F_TSWITCH)
}

/// Where a name at `p` ends: ASCII letters, digits and `_` ([`name_run`]), then any letter or
/// digit past ASCII.
#[inline(always)]
fn name_end(b: &[u8], p: usize) -> usize {
    let mut e = name_run(b, p + 1);
    if e < b.len() && b[e] >= 0x80 {
        e = word_end(b, e);
    }
    e
}

/// Where the run of a name's ASCII bytes (`[A-Za-z0-9_]`) from `from` ends: sixteen bytes a step
/// on x86-64 (SSE2's compares, the line's last step read again from its end less sixteen, as
/// [`crate::prim::find_in`] does; Scala's, X-137), a byte a step elsewhere.
#[inline(always)]
fn name_run(b: &[u8], from: usize) -> usize {
    let n = b.len();
    #[cfg(target_arch = "x86_64")]
    if n >= 16 && from < n {
        use std::arch::x86_64::*;
        // SAFETY: SSE2 is part of x86-64; every load reads `b[q..q + 16]` with `q + 16 <= n`
        unsafe {
            let misses = |q: usize| {
                let w = _mm_loadu_si128(b.as_ptr().add(q).cast());
                // a letter: `(c | 0x20) - 'a' < 26`, unsigned, as a signed compare biased by 128
                let l = _mm_add_epi8(
                    _mm_or_si128(w, _mm_set1_epi8(0x20)),
                    _mm_set1_epi8((128u8.wrapping_sub(b'a')) as i8),
                );
                let alpha = _mm_cmplt_epi8(l, _mm_set1_epi8((128 + 26) as u8 as i8));
                let d = _mm_add_epi8(w, _mm_set1_epi8((128u8.wrapping_sub(b'0')) as i8));
                let digit = _mm_cmplt_epi8(d, _mm_set1_epi8((128 + 10) as u8 as i8));
                let us = _mm_cmpeq_epi8(w, _mm_set1_epi8(b'_' as i8));
                let m = _mm_or_si128(_mm_or_si128(alpha, digit), us);
                !(_mm_movemask_epi8(m) as u32) & 0xffff
            };
            let mut i = from;
            while i + 16 <= n {
                let m = misses(i);
                if m != 0 {
                    return i + m.trailing_zeros() as usize;
                }
                i += 16;
            }
            if i == n {
                return n;
            }
            let q = n - 16;
            let m = misses(q) >> (i - q);
            return if m != 0 {
                i + m.trailing_zeros() as usize
            } else {
                n
            };
        }
    }
    let mut e = from;
    while e < n && WORD[usize::from(b[e])] {
        e += 1;
    }
    e
}

/// The first byte at or past `p` that isn't a space or a tab, or the line's end.
#[inline(always)]
fn skip_blanks(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && matches!(b[p], b' ' | b'\t') {
        p += 1;
    }
    p
}

/// Whether a name of a parameter list starts at `p`: an ASCII letter or `_`, or one past ASCII.
#[inline(always)]
fn name_starts(b: &[u8], p: usize) -> bool {
    match b.get(p) {
        Some(&c) if c < 0x80 => ACT[usize::from(c)] == A_NAME,
        Some(_) => is_name_start(b, p),
        None => false,
    }
}

/// Whether a parameter list from `p` (past its `(`, or a line's start inside it) is named, as Go
/// reads one: some entry a name and a type. `None` where the line has nothing of it; an entry
/// running on past the line counts as named (a list over lines names its parameters).
fn named(b: &[u8], p: usize) -> Option<bool> {
    let end = b.len().min(p + LOOK);
    let mut q = skip_blanks(b, p);
    if q >= end || b[q..].starts_with(b"//") {
        return None;
    }
    loop {
        // an entry's start
        q = skip_blanks(b, q);
        if q >= end || b[q..].starts_with(b"//") {
            return Some(true);
        }
        if b[q] == b')' {
            return Some(false);
        }
        if !name_starts(b, q) {
            // a type's prefix: `*`, `[`, `...`, `(`, `<-`
            return Some(false);
        }
        let e = name_end(b, q);
        if matches!(
            &b[q..e],
            b"func" | b"map" | b"chan" | b"struct" | b"interface"
        ) || matches!(b.get(e), Some(b'.' | b'['))
        {
            return Some(false);
        }
        let r = skip_blanks(b, e);
        match b.get(r) {
            None => return Some(true),
            Some(b',') => q = r + 1,
            Some(b')') => return Some(false),
            Some(b'/') if b.get(r + 1) == Some(&b'/') => return Some(true),
            Some(_) => return Some(true),
        }
    }
}

/// Whether the `[` at `p` after a type declaration's name opens type parameters: right after
/// the name, a name inside followed by a constraint or a `,` (`[T any]`, `[K, V any]`), not an
/// array's length (`[N]T`, `[4]int`) or a slice (`[]T`).
fn tparams_follow(b: &[u8], p: usize) -> bool {
    if p == 0 || !crate::prim::is_word(b, p - 1) {
        return false;
    }
    let q = skip_blanks(b, p + 1);
    if !name_starts(b, q) {
        return false;
    }
    let e = name_end(b, q);
    let r = skip_blanks(b, e);
    match b.get(r) {
        Some(b',') => true,
        Some(b']') | Some(b'.') | None => false,
        Some(_) => r > e,
    }
}

impl<'b, 'c, K: Sink> Scan<'b, 'c, K> {
    #[inline(always)]
    fn new(b: &'b [u8], out: &'b mut Out<'c>, k: K) -> Self {
        Scan {
            b,
            frames: 0,
            depth: 0,
            mode: M_CODE,
            ctx: 0,
            sqls: S_DONE,
            marked: false,
            prev: P_START,
            role: R_CODE,
            sqlcall: S_DONE,
            tswitch: false,
            case_ty: false,
            nodecl: 0,
            defs: 0,
            mark: usize::MAX,
            sql: sql::Embedded {
                is_end: usize::MAX,
                at: 0,
            },
            k,
            out,
        }
    }

    /// The scan's fields from `state`: the frames in its levels and the first two withs, the
    /// string's context and the SQL call's argument in the next two, the mode and the marker in
    /// [`State::arg`].
    #[inline(always)]
    fn load(&mut self, state: &State) {
        let mut w = [0u8; 16];
        w[..14].copy_from_slice(&state.levels);
        w[14] = state.withs[0];
        w[15] = state.withs[1];
        self.frames = u128::from_le_bytes(w);
        self.depth = state.depth;
        self.ctx = state.withs[2];
        self.sqls = state.withs[3];
        self.mode = (state.arg & 7) as u8;
        self.marked = state.arg & MARKED != 0;
    }

    /// The state the scan's last line left: every field as it is, so a block lexes as its
    /// lines do one at a time.
    #[inline(always)]
    fn state(&self) -> State {
        let w = self.frames.to_le_bytes();
        let mut s = State::default();
        s.levels.copy_from_slice(&w[..14]);
        s.withs[0] = w[14];
        s.withs[1] = w[15];
        s.withs[2] = self.ctx;
        s.withs[3] = self.sqls;
        s.depth = self.depth;
        s.arg = u16::from(self.mode) | if self.marked { MARKED } else { 0 };
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
        (self.frames & FMASK) as u8
    }

    #[inline(always)]
    fn set_top(&mut self, f: u8) {
        if self.depth > 0 {
            self.frames = self.frames & !FMASK | u128::from(f);
        }
    }

    #[inline(always)]
    fn push(&mut self, f: u8) {
        self.frames = self.frames << FW | u128::from(f);
        self.depth = self.depth.saturating_add(1);
    }

    #[inline(always)]
    fn pop(&mut self) {
        if self.depth > 0 {
            self.frames >>= FW;
            self.depth -= 1;
        }
    }

    /// The role and the previous token a line or a statement starts with, by the frame it's in.
    #[inline(always)]
    fn start(&mut self) {
        let (role, prev) = START[usize::from(self.top())];
        self.role = role;
        self.prev = prev;
    }

    /// The line, from what the line before left: a mode's rest first; the loop out of line
    /// where `SHARED` ([`Scan::code_shared`]).
    #[inline(always)]
    fn lex_line<const SHARED: bool>(&mut self) {
        self.start();
        self.sqlcall = S_DONE;
        self.tswitch = false;
        self.case_ty = false;
        self.nodecl = 0;
        self.defs = 0;
        self.mark = if self.marked { 0 } else { usize::MAX };
        let mut p = match self.mode {
            M_CODE => 0,
            M_COMMENT => self.comment(0),
            _ => self.raw_rest(),
        };
        if !SOL && (self.role == R_PLIST || self.sqls & S_PENDING != 0) {
            p = self.line_head(p);
        }
        if SHARED {
            self.code_shared(p);
        } else {
            self.code(p);
        }
        if !SOL {
            let n = self.b.len();
            self.marked = self.mark != usize::MAX && sql::marker_holds(self.b, self.mark, n);
        }
    }

    /// A line's first token in a parameter list not yet known to be named, or in a SQL call
    /// whose argument starts on this line: the list read from its first entry here, the
    /// argument's state from its first token.
    #[cold]
    #[inline(never)]
    fn line_head(&mut self, p: usize) -> usize {
        let b = self.b;
        let q = skip_blanks(b, p);
        if self.role == R_PLIST {
            match named(b, q) {
                Some(n) => {
                    let res = self.top() == F_RLIST;
                    let f = match (res, n) {
                        (false, true) => F_PARAMS,
                        (false, false) => F_UPARAMS,
                        (true, true) => F_RES,
                        (true, false) => F_URES,
                    };
                    self.set_top(f);
                    self.role = if n { R_PNAME } else { R_TYPE };
                }
                None => self.role = R_CODE,
            }
        }
        if self.sqls & S_PENDING != 0 && self.top() == F_SQLCALL && q < b.len() {
            self.sqls &= !S_PENDING;
            self.sql_arg(q);
        }
        p
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
            let from = p;
            p = match ACT[usize::from(c)] {
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
                A_BQUOTE => self.bquote(p),
                A_SQUOTE => self.rune(p),
                A_SLASH => match b.get(p + 1) {
                    Some(b'/') => self.line_comment(p),
                    Some(b'*') => self.comment_open(p),
                    Some(b'=') => self.op_token(p, 2, ASSIGNMENT),
                    _ => self.op_token(p, 1, OPERATOR),
                },
                A_DOT => self.dot(p),
                A_OP => self.op(p),
                A_LPAREN => self.lparen(p),
                A_RPAREN => self.rparen(p),
                A_LBRACE => self.lbrace(p),
                A_RBRACE => self.rbrace(p),
                A_LBRACKET => self.lbracket(p),
                A_RBRACKET => self.rbracket(p),
                A_COMMA => self.comma(p),
                A_SEMI => {
                    self.put_hot(p, p + 1, TERMINATOR);
                    self.start();
                    self.case_ty = false;
                    p + 1
                }
                A_WIDE => {
                    // a letter starts a name; a digit past ASCII doesn't (Go's spec)
                    if is_name_start(b, p) {
                        self.name(p)
                    } else {
                        p + char_len(c).min(n - p)
                    }
                }
                _ => {
                    self.prev = P_OP;
                    p + 1
                }
            };
            debug_assert!(p > from, "Go's code loop stood at {from}");
        }
    }

    /// `n` bytes at `p` an operator of `kind`.
    #[inline(always)]
    fn op_token(&mut self, p: usize, n: usize, kind: u16) -> usize {
        self.put_hot(p, p + n, kind);
        self.prev = P_OP;
        self.role = R_CODE;
        p + n
    }

    /// An operator at `p`, by Go's longest match; a type's `*`, `<-`, `~` and `|` keep it a type.
    #[inline(always)]
    fn op(&mut self, p: usize) -> usize {
        let b = self.b;
        let c = b[p];
        let c1 = b.get(p + 1).copied().unwrap_or(b'\n');
        let (n, kind) = match c {
            b'=' => {
                if c1 == b'=' {
                    (2, OPERATOR)
                } else {
                    (1, ASSIGNMENT)
                }
            }
            b':' => {
                if c1 == b'=' {
                    (2, ASSIGNMENT)
                } else {
                    (1, SEPARATOR)
                }
            }
            b'+' | b'-' => {
                if c1 == c {
                    (2, OPERATOR)
                } else if c1 == b'=' {
                    (2, ASSIGNMENT)
                } else {
                    (1, OPERATOR)
                }
            }
            b'&' => match c1 {
                b'&' => (2, OPERATOR),
                b'=' => (2, ASSIGNMENT),
                b'^' if b.get(p + 2) == Some(&b'=') => (3, ASSIGNMENT),
                b'^' => (2, OPERATOR),
                _ => (1, OPERATOR),
            },
            b'|' => match c1 {
                b'|' => (2, OPERATOR),
                b'=' => (2, ASSIGNMENT),
                _ => (1, OPERATOR),
            },
            b'<' | b'>' => {
                if c == b'<' && c1 == b'-' {
                    (2, OPERATOR)
                } else if c1 == c {
                    if b.get(p + 2) == Some(&b'=') {
                        (3, ASSIGNMENT)
                    } else {
                        (2, OPERATOR)
                    }
                } else if c1 == b'=' {
                    (2, OPERATOR)
                } else {
                    (1, OPERATOR)
                }
            }
            b'!' => {
                if c1 == b'=' {
                    (2, OPERATOR)
                } else {
                    (1, OPERATOR)
                }
            }
            b'~' => (1, OPERATOR),
            // `*`, `%`, `^`
            _ => {
                if c1 == b'=' {
                    (2, ASSIGNMENT)
                } else {
                    (1, OPERATOR)
                }
            }
        };
        self.put_hot(p, p + n, kind);
        self.prev = P_OP;
        if SOL {
            return p + n;
        }
        if n == 1
            && (self.role != R_CODE
                || matches!(c, b'|' | b'~' | b':')
                || (c == b'+' && self.sqls == S_AFTER))
        {
            self.op_role(c);
        } else if self.role != R_CODE {
            // a channel type's `<-` keeps it a type; `:=` ends a short declaration's names
            self.role = if c == b'<' && c1 == b'-' && matches!(self.role, R_TYPE | R_RESULT) {
                R_TYPE
            } else {
                R_CODE
            };
        }
        p + n
    }

    /// What a one-byte operator `c` makes of the role: a type's prefix keeps it a type, a field's
    /// `*` an embedded type, a constraint's `|` and `~` a type; `:` ends a type switch's case; a
    /// SQL call's `+` after its string goes on with SQL.
    #[cold]
    #[inline(never)]
    fn op_role(&mut self, c: u8) {
        let top = self.top();
        self.role = match (c, self.role) {
            (b'*' | b'~', R_TYPE | R_RESULT | R_TDEF | R_TDEF2) => R_TYPE,
            (b'*', R_FIELD) => R_EMB,
            (b'~', R_ELEM | R_TPNAME) => R_TYPE,
            (b'~', _) => R_TYPE,
            (b'|', _) if matches!(top, F_TPARAMS_F | F_TPARAMS_T | F_IFACE) => R_TYPE,
            (b'=', R_TDEF | R_TDEF2) => R_TYPE,
            (b':', _) => {
                self.case_ty = false;
                R_CODE
            }
            (b'+', _) => {
                if top == F_SQLCALL && self.sqls == S_AFTER {
                    self.sqls = S_SQL;
                }
                R_CODE
            }
            _ => R_CODE,
        };
    }

    /// `.` at `p`: an accessor, `...`, a float's `.5`, or a type assertion's `.(`.
    #[inline(always)]
    fn dot(&mut self, p: usize) -> usize {
        let b = self.b;
        match b.get(p + 1) {
            Some(b'0'..=b'9') => return self.number(p),
            Some(b'.') if b.get(p + 2) == Some(&b'.') => {
                self.put_hot(p, p + 3, VARIADIC);
                self.prev = P_OP;
                return p + 3;
            }
            Some(b'(') if !SOL => {
                self.put_hot(p, p + 1, DOT);
                self.put_hot(p + 1, p + 2, PARENS_BEGIN);
                self.push(F_TPAREN);
                self.role = R_TYPE;
                self.prev = P_OP;
                return p + 2;
            }
            _ => {}
        }
        self.put_hot(p, p + 1, DOT);
        self.prev = P_DOT;
        if !matches!(self.role, R_QUAL | R_EMBQ) {
            self.role = R_CODE;
        }
        p + 1
    }

    /// `,` at `p`: a list's next entry, by the frame it's in and the names before it.
    #[inline(always)]
    fn comma(&mut self, p: usize) -> usize {
        self.put_hot(p, p + 1, SEPARATOR);
        self.prev = P_OP;
        if SOL {
            return p + 1;
        }
        if !matches!(
            self.role,
            R_VNAME | R_CNAME | R_PNAME | R_TPNAME | R_DECLS | R_FIELD
        ) {
            self.role = match self.top() {
                F_PARAMS | F_RES | F_RECV => R_PNAME,
                F_UPARAMS | F_URES | F_URECV | F_TARGS => R_TYPE,
                F_TPARAMS_F | F_TPARAMS_T => R_TPNAME,
                F_TSWITCH if self.case_ty => R_TYPE,
                F_SQLCALL => {
                    self.sql_comma(p + 1);
                    R_CODE
                }
                _ => R_CODE,
            };
        }
        p + 1
    }

    /// `(` at `p`: what it opens by the role before it.
    #[inline(always)]
    fn lparen(&mut self, p: usize) -> usize {
        self.put_hot(p, p + 1, PARENS_BEGIN);
        self.prev = P_OP;
        if SOL {
            self.push(F_PAREN);
            return p + 1;
        }
        if self.role == R_CODE {
            if self.sqlcall != S_DONE {
                return self.sql_paren(p);
            }
            self.push(F_PAREN);
            return p + 1;
        }
        self.lparen_role(p)
    }

    /// [`Scan::lparen`] where a role stands: a receiver, parameters, results, a group, a type's.
    #[inline(never)]
    fn lparen_role(&mut self, p: usize) -> usize {
        let (f, role) = match self.role {
            R_FNAME => match named(self.b, p + 1) {
                Some(false) => (F_URECV, R_TYPE),
                _ => (F_RECV, R_PNAME),
            },
            R_SIG => match named(self.b, p + 1) {
                Some(true) => (F_PARAMS, R_PNAME),
                Some(false) => (F_UPARAMS, R_TYPE),
                None => (F_PLIST, R_CODE),
            },
            R_RESULT => match named(self.b, p + 1) {
                Some(true) => (F_RES, R_PNAME),
                Some(false) => (F_URES, R_TYPE),
                None => (F_RLIST, R_CODE),
            },
            R_MAKE => (F_PAREN, R_TYPE),
            R_TYPE | R_TDEF | R_TDEF2 => (F_TPAREN, R_TYPE),
            R_TNAME => (F_TYPEGROUP, R_TNAME),
            R_VNAME => (F_VARGROUP, R_VNAME),
            R_CNAME => (F_CONSTGROUP, R_CNAME),
            _ => (F_PAREN, R_CODE),
        };
        self.push(f);
        self.role = role;
        p + 1
    }

    /// `)` at `p`: a parenthesis's, with any bracket left open above it; what follows by what it
    /// closed.
    #[inline(always)]
    fn rparen(&mut self, p: usize) -> usize {
        self.put_hot(p, p + 1, PARENS_END);
        self.prev = P_VALUE;
        let mut f = self.top();
        if !is_paren_frame(f) {
            if !(is_bracket_frame(f) && self.depth > 0) {
                self.role = R_CODE;
                return p + 1;
            }
            // what the parenthesis left open
            while is_bracket_frame(f) && self.depth > 0 {
                self.pop();
                f = self.top();
            }
            if !is_paren_frame(f) {
                self.role = R_CODE;
                return p + 1;
            }
        }
        self.pop();
        if SOL {
            return p + 1;
        }
        self.role = match f {
            F_RECV | F_URECV => R_MNAME,
            F_PARAMS | F_UPARAMS | F_PLIST => R_RESULT,
            F_SQLCALL => {
                self.sqls = S_DONE;
                R_CODE
            }
            _ => R_CODE,
        };
        p + 1
    }

    /// `[` at `p`: what it opens by the role before it, or an index after a value.
    #[inline(always)]
    fn lbracket(&mut self, p: usize) -> usize {
        self.put_hot(p, p + 1, BRACKETS_BEGIN);
        if SOL {
            self.prev = P_OP;
            self.push(F_BRACKET);
            return p + 1;
        }
        if self.role == R_CODE && self.prev == P_VALUE {
            self.prev = P_OP;
            self.push(F_BRACKET);
            return p + 1;
        }
        self.lbracket_role(p)
    }

    /// [`Scan::lbracket`] where a role stands, or at an operand's start: a type's `[]` and
    /// `[N]`, type arguments, a map's key, type parameters.
    #[inline(never)]
    fn lbracket_role(&mut self, p: usize) -> usize {
        let b = self.b;
        self.prev = P_OP;
        let (f, role) = match self.role {
            R_TARGS => (F_TARGS, R_TYPE),
            R_MAP => (F_MAPKEY, R_TYPE),
            R_SIG => (F_TPARAMS_F, R_TPNAME),
            R_TDEF if tparams_follow(b, p) => (F_TPARAMS_T, R_TPNAME),
            R_CODE | R_TYPE | R_RESULT | R_TDEF | R_TDEF2 => {
                // a slice's `[]`, an array's `[N]`: a type follows
                if b.get(p + 1) == Some(&b']') {
                    self.put_hot(p + 1, p + 2, BRACKETS_END);
                    self.role = R_TYPE;
                    return p + 2;
                }
                (F_ALEN, R_CODE)
            }
            _ => (F_BRACKET, R_CODE),
        };
        self.push(f);
        self.role = role;
        p + 1
    }

    /// `]` at `p`: a bracket's, with any parenthesis left open above it; what follows by what it
    /// closed.
    #[inline(always)]
    fn rbracket(&mut self, p: usize) -> usize {
        self.put_hot(p, p + 1, BRACKETS_END);
        self.prev = P_VALUE;
        let mut f = self.top();
        if !is_bracket_frame(f) {
            if !((is_paren_frame(f) && f != F_SQLCALL) && self.depth > 0) {
                self.role = R_CODE;
                return p + 1;
            }
            while is_paren_frame(f) && f != F_SQLCALL && self.depth > 0 {
                self.pop();
                f = self.top();
            }
            if !is_bracket_frame(f) {
                self.role = R_CODE;
                return p + 1;
            }
        }
        self.pop();
        if SOL {
            return p + 1;
        }
        self.role = match f {
            F_ALEN | F_MAPKEY => {
                self.prev = P_OP;
                R_TYPE
            }
            F_TPARAMS_F => {
                self.prev = P_OP;
                R_SIG
            }
            F_TPARAMS_T => {
                self.prev = P_OP;
                R_TDEF2
            }
            _ => R_CODE,
        };
        p + 1
    }

    /// `{` at `p`: a struct's or an interface's body, a type switch's, a block or a literal.
    #[inline(always)]
    fn lbrace(&mut self, p: usize) -> usize {
        self.put_hot(p, p + 1, BRACES_BEGIN);
        let f = if SOL {
            F_BRACE
        } else {
            match self.role {
                R_BODY_S => F_STRUCT,
                R_BODY_I => F_IFACE,
                _ if self.tswitch => F_TSWITCH,
                _ => F_BRACE,
            }
        };
        self.tswitch = false;
        self.push(f);
        self.start();
        p + 1
    }

    /// `}` at `p`: the innermost brace closed, what it left open dropped; a stray one nothing.
    #[inline(always)]
    fn rbrace(&mut self, p: usize) -> usize {
        self.put_hot(p, p + 1, BRACES_END);
        if is_brace_frame(self.top()) {
            self.pop();
        } else {
            self.unwind_brace();
        }
        self.prev = P_VALUE;
        self.role = R_CODE;
        self.case_ty = false;
        p + 1
    }

    /// A `}` with frames above its brace: they're dropped, if one is open below them.
    #[cold]
    fn unwind_brace(&mut self) {
        let levels = self.depth.min(LEVELS);
        let mut i = 0;
        while i < levels {
            if is_brace_frame(((self.frames >> (FW * u32::from(i))) & FMASK) as u8) {
                for _ in 0..=i {
                    self.pop();
                }
                return;
            }
            i += 1;
        }
    }

    /// A word at `p`: a keyword, a type, a declared name, a call, a member, or a name.
    #[inline(always)]
    fn name(&mut self, p: usize) -> usize {
        let b = self.b;
        let e = name_end(b, p);
        let w = WORDS.value(&b[p..e], &INFO);
        if SOL {
            return self.sol_word(p, e, w);
        }
        if self.role != R_CODE {
            return self.role_word(p, e, w);
        }
        self.code_word(p, e, w)
    }

    /// The floor probe's word: its table leaf, a call, a member or a name.
    #[inline(always)]
    fn sol_word(&mut self, p: usize, e: usize, w: u16) -> usize {
        let call = self.b.get(e) == Some(&b'(');
        let leaf = w & 0xff;
        let kind = match w & W_ACTION {
            W_BTYPE => {
                if call {
                    BUILTIN_CAST
                } else {
                    STORAGE_BUILTIN
                }
            }
            W_BFUNC | W_MAKE if call => BUILTIN_FUNCTION,
            _ if leaf != 0 => leaf,
            _ if call => FUNCTION,
            _ if self.prev == P_DOT => MEMBER,
            _ => VARIABLE,
        };
        self.put_hot(p, e, kind);
        self.prev = if leaf != 0 && w & W_ACTION != 0 && w & W_ACTION != W_BTYPE {
            P_OP
        } else {
            P_VALUE
        };
        e
    }

    /// A word `b[p..e]` (`w` its info) in code.
    #[inline(always)]
    fn code_word(&mut self, p: usize, e: usize, w: u16) -> usize {
        if w & W_ACTION != 0 {
            return self.word_action(p, e, w);
        }
        let leaf = w & 0xff;
        if leaf != 0 {
            // a predeclared constant, `iota`, `_`
            let kind = if self.prev == P_DOT { MEMBER } else { leaf };
            self.put_hot(p, e, kind);
            self.prev = P_VALUE;
            return e;
        }
        let b = self.b;
        let c = b.get(e).copied().unwrap_or(b'\n');
        let kind = if c == b'(' {
            if w & W_SQL != 0 && self.prev == P_DOT {
                self.sql_name(p, e);
            }
            FUNCTION
        } else if self.prev == P_DOT {
            MEMBER
        } else if (c == b',' || c == b':' || (c == b' ' && b.get(e + 1) == Some(&b':')))
            && self.decl_follows(e)
        {
            self.role = R_DECLS;
            DECLARATION
        } else {
            VARIABLE
        };
        self.put_hot(p, e, kind);
        self.prev = P_VALUE;
        e
    }

    /// Whether the names from `e` (past one) run to a short variable declaration's `:=`:
    /// `(, name)* :=`. A search that fails marks where, so the names it passed don't search
    /// again (a long list of arguments isn't quadratic).
    #[inline(never)]
    fn decl_follows(&mut self, e: usize) -> bool {
        if e < self.nodecl || self.defs == usize::MAX {
            return false;
        }
        let b = self.b;
        if self.defs < e {
            // the line's next `:=`: a line without one asks no more
            self.defs = next_define(b, e);
            if self.defs == usize::MAX {
                return false;
            }
        }
        let mut q = e;
        loop {
            q = skip_blanks(b, q);
            match b.get(q) {
                Some(b':') => {
                    if b.get(q + 1) == Some(&b'=') {
                        return true;
                    }
                    break;
                }
                Some(b',') => {
                    q = skip_blanks(b, q + 1);
                    if !name_starts(b, q) {
                        break;
                    }
                    q = name_end(b, q);
                }
                _ => break,
            }
        }
        self.nodecl = q.max(e + 1);
        false
    }

    /// A keyword, a predeclared type or function at `p..e` (`w` its info) in code.
    #[inline(never)]
    fn word_action(&mut self, p: usize, e: usize, w: u16) -> usize {
        let b = self.b;
        let leaf = w & 0xff;
        let call = b.get(e) == Some(&b'(');
        let action = w & W_ACTION;
        if self.prev == P_DOT && action >= W_BTYPE {
            // a member that spells a predeclared name (`x.len`, `t.string`)
            self.put_hot(p, e, if call { FUNCTION } else { MEMBER });
            self.prev = P_VALUE;
            return e;
        }
        let (kind, prev, role) = match action {
            W_KEYWORD => (leaf, P_OP, R_CODE),
            W_CASE => {
                if self.top() == F_TSWITCH {
                    self.case_ty = true;
                    (leaf, P_OP, R_TYPE)
                } else {
                    (leaf, P_OP, R_CODE)
                }
            }
            W_FUNC => {
                let decl = self.depth == 0 && self.prev == P_START;
                (leaf, P_OP, if decl { R_FNAME } else { R_SIG })
            }
            W_TYPE => (leaf, P_OP, R_TNAME),
            W_VAR => (leaf, P_OP, R_VNAME),
            W_CONST => (leaf, P_OP, R_CNAME),
            W_STRUCT => (leaf, P_OP, R_BODY_S),
            W_IFACE => (leaf, P_OP, R_BODY_I),
            W_MAP => (leaf, P_OP, R_MAP),
            W_CHAN => (leaf, P_OP, R_TYPE),
            // a predeclared type's name declared (`rune, size := …`)
            W_BTYPE
                if !call
                    && self.prev != P_DOT
                    && (matches!(b.get(e), Some(b',' | b':'))
                        || (b.get(e) == Some(&b' ') && b.get(e + 1) == Some(&b':')))
                    && self.decl_follows(e) =>
            {
                return self.code_word(p, e, w & !W_ACTION);
            }
            W_BTYPE => (
                if call { BUILTIN_CAST } else { STORAGE_BUILTIN },
                P_VALUE,
                R_CODE,
            ),
            // W_BFUNC, W_MAKE
            _ if call => (
                BUILTIN_FUNCTION,
                P_VALUE,
                if action == W_MAKE { R_MAKE } else { R_CODE },
            ),
            _ => {
                return self.code_word(p, e, w & !W_ACTION);
            }
        };
        self.put_hot(p, e, kind);
        self.prev = prev;
        self.role = role;
        e
    }

    /// A word `b[p..e]` (`w` its info) where a role stands: a type, a declared name, a parameter,
    /// a field, or code where the role doesn't take it.
    #[inline(never)]
    fn role_word(&mut self, p: usize, e: usize, w: u16) -> usize {
        let b = self.b;
        let leaf = w & 0xff;
        let action = w & W_ACTION;
        // the next byte that isn't a blank
        let next = || b.get(skip_blanks(b, e)).copied().unwrap_or(b'\n');
        let kind = match self.role {
            R_TYPE | R_QUAL => return self.type_word(p, e, w),
            R_RESULT | R_TDEF | R_TDEF2 => {
                self.role = R_TYPE;
                return self.type_word(p, e, w);
            }
            R_FNAME | R_MNAME if named_word(action) => {
                self.role = R_SIG;
                FUNCTION_NAME
            }
            R_TNAME if named_word(action) => {
                self.role = R_TDEF;
                if leaf == BLANK { BLANK } else { TYPE_NAME }
            }
            R_VNAME | R_CNAME | R_PNAME | R_TPNAME | R_DECLS if named_word(action) => {
                let kind = if leaf == BLANK {
                    BLANK
                } else {
                    match self.role {
                        R_VNAME | R_DECLS => DECLARATION,
                        R_CNAME => CONSTANT_DECLARATION,
                        _ => PARAMETER,
                    }
                };
                if self.role != R_DECLS {
                    self.role = match next() {
                        b',' => self.role,
                        b'=' if b.get(skip_blanks(b, e) + 1) != Some(&b'=') => R_CODE,
                        b'\n' | b';' | b'/' | b')' | b']' => R_CODE,
                        _ => R_TYPE,
                    };
                }
                kind
            }
            R_FIELD | R_EMB | R_EMBQ if named_word(action) => {
                return self.field_word(p, e, w);
            }
            R_ELEM if named_word(action) => return self.elem_word(p, e, w),
            R_PLIST => {
                self.role = R_CODE;
                return self.code_word(p, e, w);
            }
            _ => {
                self.role = R_CODE;
                return self.code_word(p, e, w);
            }
        };
        self.put_hot(p, e, kind);
        self.prev = P_VALUE;
        e
    }

    /// A word `b[p..e]` (`w` its info) where a type stands: a type's keyword, a predeclared
    /// type, a package's name before its `.`, a type's name.
    fn type_word(&mut self, p: usize, e: usize, w: u16) -> usize {
        let b = self.b;
        let leaf = w & 0xff;
        let (kind, role, prev) = match w & W_ACTION {
            W_MAP => (leaf, R_MAP, P_OP),
            W_CHAN => (leaf, R_TYPE, P_OP),
            W_FUNC => (leaf, R_SIG, P_OP),
            W_STRUCT => (leaf, R_BODY_S, P_OP),
            W_IFACE => (leaf, R_BODY_I, P_OP),
            W_TYPE => {
                // `.(type)`: the `{` after it opens a type switch
                self.tswitch = self.top() == F_TPAREN;
                (leaf, R_CODE, P_VALUE)
            }
            W_BTYPE => (STORAGE_BUILTIN, after_type(b, e), P_VALUE),
            0 if leaf == 0 => {
                if self.role == R_TYPE && b.get(e) == Some(&b'.') {
                    (VARIABLE, R_QUAL, P_VALUE)
                } else {
                    (STORAGE, after_type(b, e), P_VALUE)
                }
            }
            // a constant in a case (`case nil:`), `_`
            0 => (leaf, R_CODE, P_VALUE),
            _ => {
                self.role = R_CODE;
                return self.code_word(p, e, w);
            }
        };
        self.put_hot(p, e, kind);
        self.role = role;
        self.prev = prev;
        e
    }

    /// A word `b[p..e]` (`w` its info) at a struct's field's start: a field's name, or an
    /// embedded type (`T`, `*T`, `pkg.T`, `T[A]`).
    fn field_word(&mut self, p: usize, e: usize, w: u16) -> usize {
        let b = self.b;
        let builtin = w & W_ACTION == W_BTYPE;
        let embedded = if builtin {
            INHERITED_BUILTIN
        } else {
            INHERITED
        };
        let (kind, role) = match self.role {
            R_FIELD => {
                let q = skip_blanks(b, e);
                match b.get(q).copied().unwrap_or(b'\n') {
                    b'.' if q == e => (VARIABLE, R_EMBQ),
                    b'[' if q == e => (embedded, R_TARGS),
                    b',' => (MEMBER_DECLARATION, R_FIELD),
                    b'\n' | b'"' | b'`' | b';' | b'}' => (embedded, R_CODE),
                    b'/' if matches!(b.get(q + 1), Some(b'/' | b'*')) => (embedded, R_CODE),
                    _ => (MEMBER_DECLARATION, R_TYPE),
                }
            }
            R_EMB => match b.get(e) {
                Some(b'.') => (VARIABLE, R_EMBQ),
                Some(b'[') => (embedded, R_TARGS),
                _ => (embedded, R_CODE),
            },
            // R_EMBQ
            _ => (
                embedded,
                if b.get(e) == Some(&b'[') {
                    R_TARGS
                } else {
                    R_CODE
                },
            ),
        };
        self.put_hot(p, e, kind);
        self.role = role;
        self.prev = P_VALUE;
        e
    }

    /// A word `b[p..e]` (`w` its info) at an interface's element's start: a method's name, an
    /// embedded interface (`Reader`, `io.Reader`), or a type set's first type.
    fn elem_word(&mut self, p: usize, e: usize, w: u16) -> usize {
        let b = self.b;
        let q = skip_blanks(b, e);
        let (kind, role) = match b.get(q).copied().unwrap_or(b'\n') {
            b'(' => (FUNCTION_NAME, R_SIG),
            b'.' if q == e => (VARIABLE, R_EMBQ),
            b'\n' | b';' | b'}' => (
                if w & W_ACTION == W_BTYPE {
                    INHERITED_BUILTIN
                } else {
                    INHERITED
                },
                R_CODE,
            ),
            b'/' if matches!(b.get(q + 1), Some(b'/' | b'*')) => (
                if w & W_ACTION == W_BTYPE {
                    INHERITED_BUILTIN
                } else {
                    INHERITED
                },
                R_CODE,
            ),
            _ => {
                self.role = R_TYPE;
                return self.type_word(p, e, w);
            }
        };
        self.put_hot(p, e, kind);
        self.role = role;
        self.prev = P_VALUE;
        e
    }
}

/// Where the line's next `:=` from `p` is, or `usize::MAX`.
fn next_define(b: &[u8], mut p: usize) -> usize {
    let n = b.len();
    loop {
        p = crate::prim::find_in(b, p, n, *b":");
        if p >= n {
            return usize::MAX;
        }
        if b.get(p + 1) == Some(&b'=') {
            return p;
        }
        p += 1;
    }
}

/// Whether a word of `action` can be declared: no keyword, but a predeclared type's or
/// function's name (`func max(`, `func (c *conn) close()`, a parameter `min int`).
#[inline(always)]
fn named_word(action: u16) -> bool {
    action == 0 || action >= W_BTYPE
}

/// The role after a type's name ending at `e`: its `[` opens type arguments, else the type is
/// whole.
#[inline(always)]
fn after_type(b: &[u8], e: usize) -> u8 {
    if b.get(e) == Some(&b'[') {
        R_TARGS
    } else {
        R_CODE
    }
}
