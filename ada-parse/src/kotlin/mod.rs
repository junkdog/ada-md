//! Kotlin on a scanner of ada's own (X-130), from a floor probe (the same file built with
//! `ADA_PARSE_X130_SOL`): one loop over a line's bytes, a byte's action one load from a table, a
//! name read once and looked up once in one table of every word ([`WORDS`]: its leaf and, for the
//! few whose meaning hangs on where they stand, an action), the previous significant token in a
//! byte ([`Scan::prev`]), and frames only for what nests ([`Scan::frames`], four bits a level):
//! parentheses, braces, brackets, a declaration's parameters and type parameters, type arguments,
//! a type's parentheses, the calls and annotations whose strings are SQL, and a string's template
//! left open at a line's end. A declaration's head (after `fun`, `val`, `var`, `class`, a parameter
//! list's `)`) is a byte of the line's ([`Scan::head`]), a declared type the depth it opened at
//! ([`Scan::ty`]). A `"""` string and a block comment continued past a line's end are a mode
//! ([`Scan::mode`]). Kinds are bat's leaves.
//!
//! Kotlin's reading where bat's grammar has none or derails (X-123 folded in): `type` is no
//! keyword; `<` after a value is a comparison unless type arguments close it ([`targs_end`]);
//! braces nest; `===`, `!in`, `!is`, `as?`, `..`, `!!` are one operator each; numbers are Kotlin's
//! (`1_000`, `0b1`, `1u`, `1.` before a word the number `1`); a `"` string and a character end
//! with their line, but for a template open across lines; `"""` strings have no escapes and end at
//! a quote run's last three; block comments nest; comments are comments in a declaration's head; a
//! backticked name is a name; `${…}` ends at its own `}`; modifiers are modifiers only where they
//! modify ([`Scan::modifies`]), `in` an operator outside types. SQL and regexes in strings as
//! before X-130: SQL by call sites and markers ([`crate::sql`]'s `jvm_call`, `@Query`,
//! `@Language("SQL")`, `// language=SQL`), regexes in `Regex("…")`, `Pattern.compile("…")`,
//! `"…".toRegex()` ([`crate::regex`], Java's dialect).

#[cfg(test)]
mod tests;
mod text;

use crate::emit::{Mark, Rules, Run, Runs, Sink, Spans};
use crate::prim::{WORD, char_len, is_word, is_word_starting, word_end};
use crate::regex;
use crate::sql;
use crate::words::{Words, text_len};
use crate::{Out, State};

/// The floor probe (X-130's `sol`): every word its table leaf, no declaration, parameter, type
/// argument, accessor, SQL or regex read.
const SOL: bool = option_env!("ADA_PARSE_X130_SOL").is_some();

/// The leaves: a token's scopes (the compiled scanner's, and `..`'s range).
static LEAVES: [&str; 50] = [
    "",
    "comment.line.double-slash.kotlin",
    "comment.line.double-slash.kotlin punctuation.definition.comment.kotlin",
    "comment.block.kotlin",
    "comment.block.kotlin punctuation.definition.comment.kotlin",
    "string.quoted.third.kotlin",
    "string.quoted.third.kotlin punctuation.definition.string.begin.kotlin",
    "string.quoted.third.kotlin punctuation.definition.string.end.kotlin",
    "string.quoted.third.kotlin constant.character.escape.kotlin",
    "string.quoted.third.kotlin variable.parameter.template.kotlin",
    "string.quoted.double.kotlin",
    "string.quoted.double.kotlin punctuation.definition.string.begin.kotlin",
    "string.quoted.double.kotlin punctuation.definition.string.end.kotlin",
    "string.quoted.double.kotlin constant.character.escape.kotlin",
    "string.quoted.double.kotlin variable.parameter.template.kotlin",
    "string.quoted.single.kotlin",
    "string.quoted.single.kotlin punctuation.definition.string.begin.kotlin",
    "string.quoted.single.kotlin punctuation.definition.string.end.kotlin",
    "string.quoted.single.kotlin constant.character.escape.kotlin",
    "storage.modifier.kotlin",
    "keyword.operator.declaration.kotlin",
    "keyword.operator.assignment.kotlin",
    "entity.name.type.class.kotlin",
    "entity.other.inherited-class.kotlin",
    "keyword.other.kotlin",
    "entity.name.function.kotlin",
    "storage.type.generic.kotlin",
    "entity.name.package.kotlin",
    "variable.parameter.function.kotlin",
    "constant.language.kotlin",
    "constant.numeric.kotlin",
    "constant.other.kotlin",
    "keyword.control.catch-exception.kotlin",
    "keyword.control.kotlin",
    "keyword.operator.kotlin",
    "keyword.operator.comparison.kotlin",
    "keyword.other.by.kotlin",
    "keyword.operator.safenav.kotlin",
    "keyword.operator.dot.kotlin",
    "keyword.operator.elvis.kotlin",
    "keyword.operator.increment-decrement.kotlin",
    "keyword.operator.arithmetic.assign.kotlin",
    "keyword.operator.arithmetic.kotlin",
    "keyword.operator.logical.kotlin",
    "punctuation.terminator.kotlin",
    "storage.type.buildin.kotlin",
    "storage.type.buildin.array.kotlin",
    "storage.type.buildin.collection.kotlin",
    "entity.name.variable.kotlin",
    "keyword.operator.range.kotlin",
];

const PLAIN: u16 = 0;
const LINE_COMMENT: u16 = 1;
const LINE_MARK: u16 = 2;
const BLOCK_COMMENT: u16 = 3;
const BLOCK_MARK: u16 = 4;
/// A string's leaves: its text, then these after it.
const RAW_TEXT: u16 = 5;
const STRING_TEXT: u16 = 10;
const CHAR_TEXT: u16 = 15;
const BEGIN: u16 = 1;
const END: u16 = 2;
const ESCAPE: u16 = 3;
const TEMPLATE: u16 = 4;
const MODIFIER: u16 = 19;
const DECLARATION: u16 = 20;
const ASSIGNMENT: u16 = 21;
const CLASS_NAME: u16 = 22;
const KEYWORD: u16 = 24;
const FUNCTION: u16 = 25;
const GENERIC: u16 = 26;
const PACKAGE: u16 = 27;
const PARAMETER: u16 = 28;
const LANGUAGE: u16 = 29;
const NUMBER: u16 = 30;
const CONSTANT: u16 = 31;
const CATCH: u16 = 32;
const CONTROL: u16 = 33;
const OPERATOR: u16 = 34;
const COMPARISON: u16 = 35;
const BY: u16 = 36;
const SAFENAV: u16 = 37;
const DOT: u16 = 38;
const ELVIS: u16 = 39;
const INCDEC: u16 = 40;
const ARITH_ASSIGN: u16 = 41;
const ARITH: u16 = 42;
const LOGICAL: u16 = 43;
const TERMINATOR: u16 = 44;
const BUILTIN: u16 = 45;
const ARRAY: u16 = 46;
const COLLECTION: u16 = 47;
const VARIABLE: u16 = 48;
const RANGE: u16 = 49;

/// The kinds of a regex in a `"…"` string and in a `"""…"""` one: the regex's, from each.
const RE_STRING: u16 = 64;
const RE_RAW: u16 = 192;
const _: () = assert!(LEAVES.len() <= RE_STRING as usize);
const _: () = assert!(RE_STRING as usize + regex::KINDS <= RE_RAW as usize);
/// SQL in a `"…"` string, then in a `"""…"""` one: SQL's kinds from here.
const SQL_STRING: u16 = RE_RAW + 128;
const SQL_RAW: u16 = SQL_STRING + 128;
const _: () = assert!(RE_RAW as usize + regex::KINDS <= SQL_STRING as usize);
const _: () = assert!(sql::KINDS <= 128);

/// The scopes under `source.Kotlin` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    if kind >= SQL_STRING {
        let (text, base) = if kind >= SQL_RAW {
            (RAW_TEXT, SQL_RAW)
        } else {
            (STRING_TEXT, SQL_STRING)
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
            (RAW_TEXT, RE_RAW)
        } else {
            (STRING_TEXT, RE_STRING)
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
const A_SQUOTE: u8 = 5;
const A_BACKTICK: u8 = 6;
const A_SLASH: u8 = 7;
/// `=-+!*%<>&|.?:`: an operator.
const A_OP: u8 = 8;
const A_LPAREN: u8 = 9;
const A_RPAREN: u8 = 10;
const A_LBRACE: u8 = 11;
const A_RBRACE: u8 = 12;
const A_LBRACKET: u8 = 13;
const A_RBRACKET: u8 = 14;
const A_COMMA: u8 = 15;
const A_SEMI: u8 = 16;
const A_AT: u8 = 17;
/// A byte past ASCII: a word's character, or another.
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
            b'\'' => A_SQUOTE,
            b'`' => A_BACKTICK,
            b'/' => A_SLASH,
            b'=' | b'-' | b'+' | b'!' | b'*' | b'%' | b'<' | b'>' | b'&' | b'|' | b'.' | b'?'
            | b':' => A_OP,
            b'(' => A_LPAREN,
            b')' => A_RPAREN,
            b'{' => A_LBRACE,
            b'}' => A_RBRACE,
            b'[' => A_LBRACKET,
            b']' => A_RBRACKET,
            b',' => A_COMMA,
            b';' => A_SEMI,
            b'@' => A_AT,
            _ => A_OTHER,
        };
        c += 1;
    }
    t
};

// What a word is ([`WORDS`], [`INFO`]): its leaf in the low byte, an action in the next six
// bits, and [`W_SQL`].
const W_ACTION: u16 = 0x3f << 8;
/// `fun`: a function's head follows.
const W_FUN: u16 = 1 << 8;
/// `val`, `var`: a property's head at a statement, a modifier elsewhere.
const W_VAL: u16 = 2 << 8;
/// `class`, `interface`, `object`: a class's head.
const W_CLASS: u16 = 3 << 8;
/// `companion`: a modifier before `object`.
const W_COMPANION: u16 = 4 << 8;
/// `get`: an accessor's name at a statement's start before `()`.
const W_GET: u16 = 5 << 8;
/// `set`: an accessor's name at a statement's start before `(`, its parameter after.
const W_SET: u16 = 6 << 8;
/// `as`: `as?` one operator.
const W_AS: u16 = 7 << 8;
/// `import` at a line's start: a path follows.
const W_IMPORT: u16 = 8 << 8;
/// `package` at a line's start: a name follows.
const W_PACKAGE: u16 = 9 << 8;
/// `constructor`: its `(` opens parameters.
const W_CTOR: u16 = 10 << 8;
/// `init` before `{`: a keyword.
const W_INIT: u16 = 11 << 8;
/// A collection's name: a type before `<` and a word.
const W_COLLECTION: u16 = 12 << 8;
/// A modifier's leaf that is a hard keyword (`typealias`, `typeof`): colored wherever it stands.
const W_HARD: u16 = 13 << 8;
/// `in`: an operator, but a variance modifier in a type's arguments or parameters.
const W_VARIANCE: u16 = 1 << 14;
/// A name a SQL call or annotation can have ([`sql::jvm_call`], [`sql::jvm_annotation`],
/// `exec`, `Language`): the `(` after it looks back.
const W_SQL: u16 = 1 << 15;

/// The words, each with what it is.
const WORD_LIST: &[(&[u8], u16)] = &[
    // bat's modifiers, `storage.modifier`, where they modify ([`Scan::modifies`]); bat's
    // `yield` and `attribute` aren't Kotlin's, and its `in` is an operator here
    (b"public", MODIFIER),
    (b"private", MODIFIER),
    (b"protected", MODIFIER),
    (b"abstract", MODIFIER),
    (b"final", MODIFIER),
    (b"sealed", MODIFIER),
    (b"enum", MODIFIER),
    (b"open", MODIFIER),
    (b"annotation", MODIFIER),
    (b"override", MODIFIER),
    (b"inline", MODIFIER),
    (b"vararg", MODIFIER),
    (b"out", MODIFIER),
    (b"internal", MODIFIER),
    (b"data", MODIFIER),
    (b"tailrec", MODIFIER),
    (b"operator", MODIFIER),
    (b"infix", MODIFIER),
    (b"const", MODIFIER),
    (b"typealias", W_HARD | MODIFIER),
    (b"typeof", W_HARD | MODIFIER),
    (b"reified", MODIFIER),
    (b"suspend", MODIFIER),
    // Kotlin's modifiers bat's list lacks
    (b"value", MODIFIER),
    (b"lateinit", MODIFIER),
    (b"inner", MODIFIER),
    (b"external", MODIFIER),
    (b"crossinline", MODIFIER),
    (b"noinline", MODIFIER),
    (b"expect", MODIFIER),
    (b"actual", MODIFIER),
    (b"val", W_VAL | MODIFIER),
    (b"var", W_VAL | MODIFIER),
    // exceptions
    (b"try", CATCH),
    (b"catch", CATCH),
    (b"finally", CATCH),
    (b"throw", CATCH),
    // control
    (b"if", CONTROL),
    (b"else", CONTROL),
    (b"while", CONTROL),
    (b"for", CONTROL),
    (b"do", CONTROL),
    (b"return", CONTROL),
    (b"when", CONTROL),
    (b"where", CONTROL),
    (b"break", CONTROL),
    (b"continue", CONTROL),
    // operators
    (b"is", OPERATOR),
    (b"in", W_VARIANCE | OPERATOR),
    (b"as", W_AS | OPERATOR),
    (b"assert", OPERATOR),
    (b"by", BY),
    // constants
    (b"true", LANGUAGE),
    (b"false", LANGUAGE),
    (b"null", LANGUAGE),
    (b"this", LANGUAGE),
    (b"super", LANGUAGE),
    // declarations
    (b"fun", W_FUN | KEYWORD),
    (b"class", W_CLASS | MODIFIER),
    (b"interface", W_CLASS | MODIFIER),
    (b"object", W_CLASS | MODIFIER),
    (b"companion", W_COMPANION),
    (b"get", W_GET),
    (b"set", W_SET),
    (b"import", W_IMPORT),
    (b"package", W_PACKAGE),
    (b"constructor", W_CTOR | KEYWORD),
    (b"init", W_INIT),
    // types
    (b"Nothing", BUILTIN),
    (b"Any", BUILTIN),
    (b"Unit", BUILTIN),
    (b"String", BUILTIN),
    (b"CharSequence", BUILTIN),
    (b"Int", BUILTIN),
    (b"Boolean", BUILTIN),
    (b"Char", BUILTIN),
    (b"Long", BUILTIN),
    (b"Double", BUILTIN),
    (b"Float", BUILTIN),
    (b"Short", BUILTIN),
    (b"Byte", BUILTIN),
    (b"dynamic", BUILTIN),
    (b"IntArray", ARRAY),
    (b"BooleanArray", ARRAY),
    (b"CharArray", ARRAY),
    (b"LongArray", ARRAY),
    (b"DoubleArray", ARRAY),
    (b"FloatArray", ARRAY),
    (b"ShortArray", ARRAY),
    (b"ByteArray", ARRAY),
    (b"Array", W_COLLECTION),
    (b"Collection", W_COLLECTION),
    (b"List", W_COLLECTION),
    (b"Map", W_COLLECTION),
    (b"Set", W_COLLECTION),
    (b"MutableList", W_COLLECTION),
    (b"MutableMap", W_COLLECTION),
    (b"MutableSet", W_COLLECTION),
    (b"Sequence", W_COLLECTION),
    // SQL's calls and annotations
    (b"exec", W_SQL),
    (b"execute", W_SQL),
    (b"executeQuery", W_SQL),
    (b"executeUpdate", W_SQL),
    (b"executeLargeUpdate", W_SQL),
    (b"execSQL", W_SQL),
    (b"prepareStatement", W_SQL),
    (b"prepareCall", W_SQL),
    (b"addBatch", W_SQL),
    (b"batchUpdate", W_SQL),
    (b"createQuery", W_SQL),
    (b"createNativeQuery", W_SQL),
    (b"createSQLQuery", W_SQL),
    (b"createNativeMutationQuery", W_SQL),
    (b"createMutationQuery", W_SQL),
    (b"createSelectionQuery", W_SQL),
    (b"createUpdate", W_SQL),
    (b"createCall", W_SQL),
    (b"createScript", W_SQL),
    (b"compileStatement", W_SQL),
    (b"queryForObject", W_SQL),
    (b"queryForList", W_SQL),
    (b"queryForMap", W_SQL),
    (b"queryForRowSet", W_SQL),
    (b"queryForStream", W_SQL),
    (b"queryForLong", W_SQL),
    (b"queryForInt", W_SQL),
    (b"query", W_SQL),
    (b"select", W_SQL),
    (b"update", W_SQL),
    (b"sql", W_SQL),
    (b"rawQuery", W_SQL),
    (b"Language", W_SQL),
    (b"Query", W_SQL),
    (b"NativeQuery", W_SQL),
    (b"SqlQuery", W_SQL),
    (b"SqlUpdate", W_SQL),
    (b"SqlBatch", W_SQL),
    (b"SqlCall", W_SQL),
    (b"SqlScript", W_SQL),
    (b"Select", W_SQL),
    (b"Insert", W_SQL),
    (b"Update", W_SQL),
    (b"Delete", W_SQL),
    (b"NamedQuery", W_SQL),
    (b"NamedNativeQuery", W_SQL),
    (b"Formula", W_SQL),
    (b"Subselect", W_SQL),
    (b"Where", W_SQL),
    (b"SQLRestriction", W_SQL),
    (b"SQLSelect", W_SQL),
    (b"SQLInsert", W_SQL),
    (b"SQLUpdate", W_SQL),
    (b"SQLDelete", W_SQL),
    (b"SQLDeleteAll", W_SQL),
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

/// Whether `leaf` is one of bat's `keywords` (a modifier, an exception's, control's or an
/// operator's word, `by`): what a parameter list or type parameters color as itself.
fn is_keyword(leaf: u16) -> bool {
    matches!(leaf, MODIFIER | CATCH | CONTROL | OPERATOR | BY)
}

// The frames, four bits a level in [`Scan::frames`]; 0 is none (the top level, or a level lost
// past [`LEVELS`]).
/// `(` … `)` of code.
const F_PAREN: u8 = 1;
/// `{` … `}`: statements.
const F_BRACE: u8 = 2;
/// `[` … `]`.
const F_BRACKET: u8 = 3;
/// A declaration's parameters, where a name stands.
const F_PARAMS: u8 = 4;
/// A parameter's type, after its `:`.
const F_PTYPE: u8 = 5;
/// A parameter's default value, after its `=`.
const F_PDEFAULT: u8 = 6;
/// A declaration's type parameters, `<` … `>`, where a name stands.
const F_TPARAMS: u8 = 7;
/// A type parameter's bound, after its `:`.
const F_TBOUND: u8 = 8;
/// A type's arguments, `<` … `>`.
const F_TARGS: u8 = 9;
/// A call whose first argument is SQL ([`sql::jvm_call`]), to its first `,`.
const F_SQLCALL: u8 = 10;
/// A SQL annotation's arguments (`@Query(`): its `value` and the arguments
/// [`sql::jvm_argument`] names are SQL.
const F_SQLANNOT: u8 = 11;
/// A SQL annotation's argument that isn't SQL.
const F_ANNOTARG: u8 = 12;
/// A `"…"` string's `${` … `}` left open at a line's end: code to its `}`, the string after.
const F_TMPL_DQ: u8 = 13;
/// A `"""…"""` string's.
const F_TMPL_RAW: u8 = 14;
/// `(` … `)` in a type: a function type's parameters, a receiver's type.
const F_TYPEPAREN: u8 = 15;
/// The levels [`Scan::frames`] holds.
const LEVELS: u8 = 32;

/// Whether frame `f` is a type's: its words colored as types, `<` type arguments.
fn is_type_frame(f: u8) -> bool {
    matches!(f, F_PTYPE | F_TBOUND | F_TARGS | F_TYPEPAREN)
}

/// Whether frame `f` is closed by `)`.
fn is_paren_frame(f: u8) -> bool {
    matches!(
        f,
        F_PAREN
            | F_PARAMS
            | F_PTYPE
            | F_PDEFAULT
            | F_SQLCALL
            | F_SQLANNOT
            | F_ANNOTARG
            | F_TYPEPAREN
    )
}

// What a line starts in ([`Scan::mode`]).
const M_CODE: u8 = 0;
/// A block comment, [`Scan::nest`] deep.
const M_COMMENT: u8 = 1;
/// A `"""` string.
const M_RAW: u8 = 2;
/// A `"""` string of SQL, SQL's context in [`Scan::ctx`].
const M_RAW_SQL: u8 = 3;
/// A `"""` string of a regex, the regex's state in [`Scan::ctx`].
const M_RAW_RE: u8 = 4;
/// [`State::arg`]'s bit beside the mode: a SQL marker waits for its string.
const MARKED: u16 = 8;

// What the previous significant token was ([`Scan::prev`]).
/// A line's or a statement's start: after `;`, `{`.
const P_START: u8 = 0;
/// A modifier or an annotation: a declaration may go on.
const P_MOD: u8 = 1;
/// A name or a word that ends a value (`this`, a type).
const P_NAME: u8 = 2;
/// A value's end: `)`, `]`, `}`, a number, a string.
const P_VALUE: u8 = 3;
/// An operator, `(`, `,`, a keyword: an operand may start.
const P_OP: u8 = 4;
/// `.`, `?.`, `::`: a member's name follows.
const P_DOT: u8 = 5;

// A declaration's head on the line ([`Scan::head`]).
const H_NONE: u8 = 0;
/// After `fun`: type parameters, a receiver, the name.
const H_FUN: u8 = 1;
/// After a function's name: its `(` opens the parameters.
const H_FUN_NAMED: u8 = 2;
/// After `val`/`var`: type parameters, a receiver, the names.
const H_VAL: u8 = 3;
/// After `class`, `interface`, `object`: the name.
const H_CLASS: u8 = 4;
/// After a class's name: type parameters, modifiers, `constructor`, the parameters.
const H_CLASS_HEAD: u8 = 5;
/// After a parameter list's `)`: its `:` opens a type (a function's result, a class's
/// supertypes).
const H_RET: u8 = 6;
/// After `constructor` or an accessor's `set`: its `(` opens the parameters.
const H_PARAMS: u8 = 7;

/// How far [`targs_end`] and a template's `${` look for their close.
const LOOK: usize = 512;

/// Lex `b`, a line of Kotlin without its `\n`, from `state`.
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

/// A block of Kotlin on one scan opened from `state` ([`crate::Carry`]).
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
    /// What the line starts in ([`M_CODE`]…).
    mode: u8,
    /// An open block comment's depth.
    nest: u8,
    /// A `"""` string's SQL context ([`M_RAW_SQL`]) or regex state ([`M_RAW_RE`]).
    ctx: u8,
    /// A SQL marker waits for its string past the line ([`MARKED`]).
    marked: bool,
    // The line's own, reset at its start:
    /// The previous significant token ([`P_START`]…).
    prev: u8,
    /// A declaration's head ([`H_NONE`]…).
    head: u8,
    /// In a declaration's type, after its `:` (a property's, a function's result, a class's
    /// supertypes), to its `=`, `{` or `;`: the depth it opened at plus one, 0 for none; the
    /// brackets inside it (a supertype's arguments) aren't the type ([`Scan::in_type`]).
    ty: u16,
    /// Where a SQL marker's string may open from (`usize::MAX` for none): the next string at or
    /// past it is SQL, unless a `;`, `,`, `)`, `{` or `}` comes between.
    mark: usize,
    /// The last [`W_SQL`] word on the line, `start..end` (`end` `usize::MAX` for none).
    sqlw: (usize, usize),
    /// The last string opened on the line not as SQL or a regex: where its text starts, and
    /// where the sink's writing stood then, for `.toRegex()` after it ([`Scan::relex`]).
    opened: Option<(usize, Mark)>,
    /// What SQL's scan keeps between the host's turns on a line.
    sql: sql::Embedded,
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
            nest: 0,
            ctx: 0,
            marked: false,
            prev: P_START,
            head: H_NONE,
            ty: 0,
            mark: usize::MAX,
            sqlw: (0, usize::MAX),
            opened: None,
            sql: sql::Embedded {
                is_end: usize::MAX,
                at: 0,
            },
            k,
            out,
        }
    }

    /// The scan's fields from `state`: the frames in its levels and the first two withs, the
    /// comment's depth and the string's context in the next two, the mode and the marker in
    /// [`State::arg`].
    #[inline(always)]
    fn load(&mut self, state: &State) {
        let mut w = [0u8; 16];
        w[..14].copy_from_slice(&state.levels);
        w[14] = state.withs[0];
        w[15] = state.withs[1];
        self.frames = u128::from_le_bytes(w);
        self.depth = state.depth;
        self.nest = state.withs[2];
        self.ctx = state.withs[3];
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
        s.withs[2] = self.nest;
        s.withs[3] = self.ctx;
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
        (self.frames & 15) as u8
    }

    /// Whether the scan is in a declaration's type ([`Scan::ty`]), not in a bracket inside it.
    #[inline(always)]
    fn in_type(&self) -> bool {
        self.ty == u16::from(self.depth) + 1
    }

    /// Whether the modifier's word ending at `e` modifies, as Kotlin's soft modifiers do: before
    /// `@`, or before a word that is a declaration's keyword or another modifier; where `any`
    /// (in a parameter list, a type), before any word, and before a function type's `(`.
    #[inline(never)]
    fn modifies(&self, e: usize, any: bool) -> bool {
        let b = self.b;
        let q = crate::prim::skip_spaces(b, e);
        let Some(&c) = b.get(q) else {
            return false;
        };
        if c == b'@' {
            return true;
        }
        if !is_word_starting(b, q) {
            return any && c == b'(';
        }
        if any {
            return true;
        }
        let w = WORDS.value(&b[q..word_end(b, q)], &INFO);
        w & 0xff == MODIFIER
            || matches!(
                w & W_ACTION,
                W_FUN | W_VAL | W_CLASS | W_CTOR | W_GET | W_SET | W_COMPANION
            )
    }

    #[inline(always)]
    fn set_top(&mut self, f: u8) {
        if self.depth > 0 {
            self.frames = self.frames & !15 | u128::from(f);
        }
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

    /// The line, from what the line before left: a mode's rest first; the loop out of line
    /// where `SHARED` ([`Scan::code_shared`]).
    #[inline(always)]
    fn lex_line<const SHARED: bool>(&mut self) {
        self.prev = P_START;
        self.head = H_NONE;
        self.ty = 0;
        self.mark = if self.marked { 0 } else { usize::MAX };
        self.sqlw = (0, usize::MAX);
        self.opened = None;
        let p = match self.mode {
            M_CODE => 0,
            M_COMMENT => self.comment(0),
            _ => self.raw_rest(),
        };
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
                A_SQUOTE => self.char_literal(p),
                A_BACKTICK => self.backtick(p),
                A_SLASH => match b.get(p + 1) {
                    Some(b'/') => self.line_comment(p),
                    Some(b'*') => self.comment_open(p),
                    Some(b'=') => self.op_token(p, 2, ARITH_ASSIGN),
                    _ => self.op_token(p, 1, ARITH),
                },
                A_OP => self.op(p),
                A_LPAREN => self.lparen(p),
                A_RPAREN => self.rparen(p),
                A_LBRACE => {
                    self.push(F_BRACE);
                    self.head = H_NONE;
                    self.ty = 0;
                    self.prev = P_START;
                    p + 1
                }
                A_RBRACE => self.rbrace(p),
                A_LBRACKET => {
                    self.push(F_BRACKET);
                    self.prev = P_OP;
                    p + 1
                }
                A_RBRACKET => {
                    if self.top() == F_BRACKET {
                        self.pop();
                    }
                    self.prev = P_VALUE;
                    p + 1
                }
                A_COMMA => self.comma(p),
                A_SEMI => {
                    self.put(p, p + 1, TERMINATOR);
                    self.head = H_NONE;
                    self.ty = 0;
                    self.prev = P_START;
                    p + 1
                }
                A_AT => self.at_sign(p),
                A_WIDE => {
                    if is_word_starting(b, p) {
                        self.name(p)
                    } else {
                        p + char_len(c)
                    }
                }
                _ => {
                    self.prev = P_OP;
                    p + 1
                }
            };
        }
    }

    /// `n` bytes at `p` an operator of `kind`.
    #[inline(always)]
    fn op_token(&mut self, p: usize, n: usize, kind: u16) -> usize {
        self.put_hot(p, p + n, kind);
        self.prev = P_OP;
        p + n
    }

    /// `(` at `p`: what it opens by the head before it, or by the SQL call or annotation it
    /// follows.
    #[inline(always)]
    fn lparen(&mut self, p: usize) -> usize {
        let call = self.prev == P_NAME;
        self.prev = P_OP;
        if SOL {
            self.push(F_PAREN);
            return p + 1;
        }
        if is_type_frame(self.top()) {
            // a function type's parameters
            self.push(F_TYPEPAREN);
            return p + 1;
        }
        match self.head {
            // a receiver's function type, `fun ((A) -> B).name()`
            H_FUN if receiver_paren(self.b, p) => self.push(F_TYPEPAREN),
            H_FUN | H_FUN_NAMED | H_CLASS | H_CLASS_HEAD | H_PARAMS => {
                self.head = H_NONE;
                self.push(F_PARAMS);
            }
            // a declared type's function type (not a supertype's arguments)
            _ if self.in_type() && !call => self.push(F_TYPEPAREN),
            _ => {
                let f = if self.sqlw.1 != usize::MAX && crate::trim_end(self.b, 0, p) == self.sqlw.1
                {
                    self.sql_paren()
                } else {
                    F_PAREN
                };
                self.push(f);
            }
        }
        p + 1
    }

    /// `)` at `p`: a parenthesis's, with any type argument or bracket left open above it.
    #[inline(always)]
    fn rparen(&mut self, p: usize) -> usize {
        self.prev = P_VALUE;
        let mut f = self.top();
        if !is_paren_frame(f) {
            if !matches!(f, F_TARGS | F_TPARAMS | F_TBOUND | F_BRACKET) {
                return p + 1;
            }
            // what the parenthesis left open
            while matches!(f, F_TARGS | F_TPARAMS | F_TBOUND | F_BRACKET) {
                self.pop();
                f = self.top();
            }
            if !is_paren_frame(f) {
                return p + 1;
            }
        }
        self.pop();
        if matches!(f, F_PARAMS | F_PTYPE | F_PDEFAULT) {
            self.head = H_RET;
        }
        p + 1
    }

    /// `}` at `p`: the innermost brace closed, what it left open dropped; a stray one nothing.
    #[inline(always)]
    fn rbrace(&mut self, p: usize) -> usize {
        self.prev = P_VALUE;
        self.head = H_NONE;
        self.ty = 0;
        match self.top() {
            F_BRACE => self.pop(),
            F_TMPL_DQ | F_TMPL_RAW => return self.template_close(p),
            _ => {
                if let Some(e) = self.unwind_brace(p) {
                    return e;
                }
            }
        }
        p + 1
    }

    /// A `}` at `p` with frames above its brace or template: they're dropped, if one is open
    /// below them; a template's string goes on after it.
    #[cold]
    fn unwind_brace(&mut self, p: usize) -> Option<usize> {
        let levels = self.depth.min(LEVELS);
        let mut i = 0;
        while i < levels {
            match ((self.frames >> (4 * i)) & 15) as u8 {
                F_BRACE => {
                    for _ in 0..=i {
                        self.pop();
                    }
                    return None;
                }
                F_TMPL_DQ | F_TMPL_RAW => {
                    for _ in 0..i {
                        self.pop();
                    }
                    return Some(self.template_close(p));
                }
                _ => {}
            }
            i += 1;
        }
        None
    }

    /// The `}` at `p` that closes a template open across lines: its string goes on.
    #[cold]
    #[inline(never)]
    fn template_close(&mut self, p: usize) -> usize {
        let raw = self.top() == F_TMPL_RAW;
        self.pop();
        let text = if raw { RAW_TEXT } else { STRING_TEXT };
        self.put(p, p + 1, text + TEMPLATE);
        self.prev = P_VALUE;
        self.string(p + 1, raw)
    }

    /// `,` at `p`: a parameter's, a type parameter's, a SQL call's first argument's end.
    #[inline(always)]
    fn comma(&mut self, p: usize) -> usize {
        self.prev = P_OP;
        match self.top() {
            F_PTYPE | F_PDEFAULT => self.set_top(F_PARAMS),
            F_TBOUND => self.set_top(F_TPARAMS),
            F_SQLCALL => self.set_top(F_PAREN),
            F_ANNOTARG => self.set_top(F_SQLANNOT),
            _ => {}
        }
        // a destructuring's names go on, and a head goes on past its type parameters' `,`
        if self.head != H_VAL
            && !self.in_type()
            && !matches!(self.top(), F_TPARAMS | F_TBOUND | F_TARGS)
        {
            self.head = H_NONE;
        }
        p + 1
    }

    /// A word at `p`: a keyword, a type, a declaration's name, or a name.
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
        let w = WORDS.value(&b[p..e], &INFO);
        let leaf = w & 0xff;
        if SOL {
            if leaf != 0 {
                self.put_hot(p, e, leaf);
            }
            self.prev = P_NAME;
            return e;
        }
        if w & W_SQL != 0 {
            self.sqlw = (p, e);
        }
        let top = self.top();
        if matches!(top, F_PARAMS | F_TPARAMS) || is_type_frame(top) {
            return self.frame_word(p, e, w, top);
        }
        if self.head != H_NONE {
            return self.head_word(p, e, w);
        }
        if w & W_ACTION != 0 {
            return self.word_action(p, e, w);
        }
        if leaf == MODIFIER && (self.prev == P_DOT || !self.modifies(e, false)) {
            // a modifier's word as a name (`data.size`, `open()`)
            self.prev = P_NAME;
            return e;
        }
        if leaf != 0 {
            self.put_hot(p, e, leaf);
            self.prev = if leaf == MODIFIER {
                P_MOD
            } else if is_keyword(leaf) {
                P_OP
            } else {
                P_NAME
            };
            return e;
        }
        if !self.in_type() && is_constant(&b[p..e]) {
            self.put_hot(p, e, CONSTANT);
        }
        if matches!(top, F_SQLANNOT | F_ANNOTARG) {
            self.annotation_argument(p, e);
        }
        self.prev = P_NAME;
        e
    }

    /// A word `b[p..e]` (`w` its info) in a parameter list, type parameters or a type: a
    /// keyword, a parameter's or a type parameter's name, a type.
    #[inline(never)]
    fn frame_word(&mut self, p: usize, e: usize, w: u16, top: u8) -> usize {
        let leaf = w & 0xff;
        self.prev = P_NAME;
        let kind = match top {
            F_PARAMS => {
                if leaf == MODIFIER && self.modifies(e, true) {
                    self.prev = P_MOD;
                    MODIFIER
                } else if is_keyword(leaf) && leaf != MODIFIER && w & W_VARIANCE == 0 {
                    leaf
                } else {
                    PARAMETER
                }
            }
            F_TPARAMS => {
                if (leaf == MODIFIER || w & W_VARIANCE != 0) && self.modifies(e, true) {
                    self.prev = P_MOD;
                    MODIFIER
                } else {
                    GENERIC
                }
            }
            _ => self.type_leaf(e, w),
        };
        if kind != PLAIN {
            self.put_hot(p, e, kind);
        }
        e
    }

    /// What a word ending at `e` (`w` its info) is in a type: a built-in type or keyword's leaf,
    /// a collection's before `<` and a word; else plain.
    #[inline(always)]
    fn type_leaf(&self, e: usize, w: u16) -> u16 {
        let leaf = w & 0xff;
        if w & W_ACTION == W_COLLECTION {
            return if self.b.get(e) == Some(&b'<') && is_word(self.b, e + 1) {
                COLLECTION
            } else {
                PLAIN
            };
        }
        if (leaf == MODIFIER || w & W_VARIANCE != 0) && self.modifies(e, true) {
            // a variance, `suspend`, `crossinline`
            return MODIFIER;
        }
        if matches!(leaf, BUILTIN | ARRAY) || is_keyword(leaf) && leaf != MODIFIER {
            leaf
        } else {
            PLAIN
        }
    }

    /// A word `b[p..e]` in a declaration's head: a function's receiver or name, a property's,
    /// a class's name, or a keyword between.
    #[inline(never)]
    fn head_word(&mut self, p: usize, e: usize, w: u16) -> usize {
        let b = self.b;
        let leaf = w & 0xff;
        match self.head {
            H_FUN | H_VAL => {
                if self.head == H_FUN && w & W_ACTION == W_CLASS {
                    // `fun interface`
                    self.put(p, e, MODIFIER);
                    self.head = H_CLASS;
                    self.prev = P_MOD;
                    return e;
                }
                // a keyword ends the head, but a soft one before `(` names the function
                if (is_keyword(leaf) && leaf != MODIFIER || w & W_ACTION == W_FUN)
                    && !(self.head == H_FUN && b.get(crate::prim::skip_spaces(b, e)) == Some(&b'('))
                {
                    self.head = H_NONE;
                    return self.name_again(p, e, w);
                }
                if is_receiver(b, e) {
                    let kind = self.type_leaf(e, w);
                    if kind != PLAIN {
                        self.put(p, e, kind);
                    }
                } else if self.head == H_FUN {
                    self.put(p, e, FUNCTION);
                    self.head = H_FUN_NAMED;
                } else {
                    self.put(p, e, VARIABLE);
                }
                self.prev = P_NAME;
                e
            }
            H_CLASS => {
                self.put(p, e, CLASS_NAME);
                self.head = H_CLASS_HEAD;
                self.prev = P_NAME;
                e
            }
            _ => {
                if w & W_ACTION == W_CTOR {
                    self.put(p, e, KEYWORD);
                    self.head = H_PARAMS;
                    self.prev = P_OP;
                    return e;
                }
                if leaf != 0 && self.head != H_FUN_NAMED {
                    let head = self.head;
                    self.head = H_NONE;
                    let r = self.name_again(p, e, w);
                    if leaf == MODIFIER && head == H_CLASS_HEAD {
                        self.head = head;
                    }
                    return r;
                }
                self.prev = P_NAME;
                e
            }
        }
    }

    /// A word read again outside the head it ended.
    #[inline(always)]
    fn name_again(&mut self, p: usize, e: usize, w: u16) -> usize {
        if w & W_ACTION != 0 {
            return self.word_action(p, e, w);
        }
        let leaf = w & 0xff;
        if leaf != 0 {
            self.put(p, e, leaf);
        }
        self.prev = if leaf == MODIFIER { P_MOD } else { P_OP };
        e
    }

    /// A word `b[p..e]` whose meaning hangs on where it stands (`w`'s action).
    #[inline(never)]
    fn word_action(&mut self, p: usize, e: usize, w: u16) -> usize {
        let b = self.b;
        let leaf = w & 0xff;
        let prev = self.prev;
        let dotted = prev == P_DOT;
        self.prev = P_NAME;
        if dotted && !matches!(w & W_ACTION, W_AS | W_COLLECTION) {
            // a member's name (`A::class`, `x.get()`)
            return e;
        }
        match w & W_ACTION {
            W_FUN if !dotted => {
                self.put(p, e, KEYWORD);
                self.head = H_FUN;
                self.prev = P_OP;
            }
            W_VAL if !dotted => {
                if matches!(self.top(), 0 | F_BRACE) {
                    self.put(p, e, KEYWORD);
                    self.head = H_VAL;
                    self.prev = P_OP;
                } else {
                    self.put(p, e, MODIFIER);
                    self.prev = P_MOD;
                }
            }
            W_CLASS if !dotted => {
                self.put(p, e, MODIFIER);
                self.head = H_CLASS;
                self.prev = P_MOD;
            }
            W_COMPANION if !dotted => {
                let q = crate::prim::skip_spaces(b, e);
                if b[q..].starts_with(b"object") && !is_word(b, q + 6) {
                    // bat's `companion\s*`: the blanks after it too
                    self.put(p, q, MODIFIER);
                    self.prev = P_MOD;
                }
            }
            // an accessor: at a statement's start, after modifiers, or after a property's type
            W_GET | W_SET
                if matches!(self.top(), 0 | F_BRACE)
                    && (matches!(prev, P_START | P_MOD) || self.in_type()) =>
            {
                let q = crate::prim::skip_spaces(b, e);
                if b.get(q) == Some(&b'(') {
                    let get = w & W_ACTION == W_GET;
                    if !get || b.get(crate::prim::skip_spaces(b, q + 1)) == Some(&b')') {
                        self.put(p, e, FUNCTION);
                        if !get {
                            self.head = H_PARAMS;
                        }
                    }
                }
            }
            W_AS => {
                let e = if b.get(e) == Some(&b'?') && b.get(e + 1) != Some(&b'.') {
                    e + 1
                } else {
                    e
                };
                self.put(p, e, OPERATOR);
                self.prev = P_OP;
                return e;
            }
            W_IMPORT | W_PACKAGE if self.depth == 0 && crate::prim::skip_spaces(b, 0) == p => {
                return self.header(p, e, w & W_ACTION == W_PACKAGE);
            }
            W_CTOR if !dotted => {
                self.put(p, e, KEYWORD);
                self.head = H_PARAMS;
                self.prev = P_OP;
            }
            W_INIT if !dotted && matches!(self.top(), 0 | F_BRACE) => {
                if b.get(crate::prim::skip_spaces(b, e)) == Some(&b'{') {
                    self.put(p, e, KEYWORD);
                }
            }
            W_COLLECTION => {
                if b.get(e) == Some(&b'<') && is_word(b, e + 1) {
                    self.put(p, e, COLLECTION);
                }
            }
            _ => {
                if leaf != 0 {
                    self.put(p, e, leaf);
                    if leaf == MODIFIER {
                        self.prev = P_MOD;
                    }
                }
            }
        }
        e
    }

    /// `package` or `import` at `p..e`, a line's first word at the top level: bat's
    /// `^\s*(package)\b(?:\s*([^ ;$]+)\s*)?` and `^\s*(import)\s+[^ $]+\s+(as)?`.
    #[cold]
    #[inline(never)]
    fn header(&mut self, p: usize, e: usize, package: bool) -> usize {
        let b = self.b;
        let s = crate::prim::skip_spaces(b, e);
        if package {
            self.put(p, e, KEYWORD);
            let mut q = s;
            while q < b.len() && !matches!(b[q], b' ' | b';' | b'$') {
                q += 1;
            }
            self.put(s, q, PACKAGE);
            self.prev = P_NAME;
            return q;
        }
        if s == e {
            self.prev = P_NAME;
            return e;
        }
        self.put(p, e, KEYWORD);
        let mut q = s;
        while q < b.len() && !matches!(b[q], b' ' | b'\t' | b'$') {
            q += 1;
        }
        let a = crate::prim::skip_spaces(b, q);
        if a > q && b[a..].starts_with(b"as") && !is_word(b, a + 2) {
            self.put(a, a + 2, KEYWORD);
            self.prev = P_OP;
            return a + 2;
        }
        self.prev = P_NAME;
        q
    }

    /// A SQL annotation's argument name `b[p..e]`, before `=`: SQL's if it is one of
    /// [`sql::jvm_argument`]'s.
    #[cold]
    fn annotation_argument(&mut self, p: usize, e: usize) {
        let b = self.b;
        let q = crate::prim::skip_spaces(b, e);
        if b.get(q) == Some(&b'=') && b.get(q + 1) != Some(&b'=') {
            let f = if sql::jvm_argument(&b[p..e]) {
                F_SQLANNOT
            } else {
                F_ANNOTARG
            };
            self.set_top(f);
        }
    }

    /// A `(` right after a [`W_SQL`] word: a SQL call's ([`F_SQLCALL`]), a SQL annotation's
    /// ([`F_SQLANNOT`]), else a parenthesis; `@Language("SQL")` marks the next string.
    #[cold]
    #[inline(never)]
    fn sql_paren(&mut self) -> u8 {
        let b = self.b;
        let (s, w) = self.sqlw;
        let name = &b[s..w];
        if s > 0 && b[s - 1] == b'@' {
            if name == b"Language" {
                if let Some(end) = sql::language_annotation(b, w) {
                    self.mark = end;
                }
                return F_PAREN;
            }
            return if sql::jvm_annotation(name) {
                F_SQLANNOT
            } else {
                F_PAREN
            };
        }
        if name == b"exec" || sql::jvm_call(b, s, w) {
            F_SQLCALL
        } else {
            F_PAREN
        }
    }

    /// `@` at `p`: an annotation's or a label's name, plain, and a use-site target's `:`.
    #[inline(never)]
    fn at_sign(&mut self, p: usize) -> usize {
        let b = self.b;
        let s = p + 1;
        if !is_word_starting(b, s) {
            self.prev = P_OP;
            return s;
        }
        let e = word_end(b, s);
        if !SOL && self.prev != P_NAME && self.prev != P_VALUE {
            let w = WORDS.value(&b[s..e], &INFO);
            if w & W_SQL != 0 {
                self.sqlw = (s, e);
            }
        }
        // a label (`return@a`, `this@A`) or an annotation (`@A`): the name plain; a use-site
        // target's `:` and the annotation's name after it (`@field:Json`)
        if b.get(e) == Some(&b':') && b.get(e + 1) != Some(&b':') {
            self.put(e, e + 1, DECLARATION);
            self.prev = P_MOD;
            if !SOL && is_word_starting(b, e + 1) {
                let n = word_end(b, e + 1);
                if WORDS.value(&b[e + 1..n], &INFO) & W_SQL != 0 {
                    self.sqlw = (e + 1, n);
                }
                return n;
            }
            return e + 1;
        }
        self.prev = if self.prev == P_NAME || self.prev == P_VALUE {
            P_NAME
        } else {
            P_MOD
        };
        e
    }

    /// An operator at `p` (`=-+!*%<>&|.?:`).
    #[inline(always)]
    fn op(&mut self, p: usize) -> usize {
        let b = self.b;
        let c = b[p];
        let c1 = b.get(p + 1).copied().unwrap_or(0);
        match c {
            b'.' => {
                if c1 == b'.' {
                    let n = if b.get(p + 2) == Some(&b'<') { 3 } else { 2 };
                    return self.op_token(p, n, RANGE);
                }
                if c1.is_ascii_digit() && self.prev != P_NAME && self.prev != P_VALUE {
                    return self.number(p);
                }
                if !matches!(self.head, H_FUN | H_VAL) || SOL {
                    self.put_hot(p, p + 1, DOT);
                }
                self.prev = P_DOT;
                p + 1
            }
            b'?' => match c1 {
                b'.' => {
                    if !matches!(self.head, H_FUN | H_VAL) || SOL {
                        self.put_hot(p, p + 2, SAFENAV);
                    }
                    self.prev = P_DOT;
                    p + 2
                }
                b':' => self.op_token(p, 2, ELVIS),
                _ => p + 1,
            },
            b':' => {
                if c1 == b':' {
                    self.put_hot(p, p + 2, OPERATOR);
                    self.prev = P_DOT;
                    return p + 2;
                }
                self.colon(p)
            }
            b'=' => {
                if c1 == b'=' {
                    let n = if b.get(p + 2) == Some(&b'=') { 3 } else { 2 };
                    return self.op_token(p, n, COMPARISON);
                }
                self.equals(p)
            }
            b'!' => match c1 {
                b'=' => {
                    let n = if b.get(p + 2) == Some(&b'=') { 3 } else { 2 };
                    self.op_token(p, n, COMPARISON)
                }
                b'!' => {
                    self.put_hot(p, p + 2, OPERATOR);
                    self.prev = P_VALUE;
                    p + 2
                }
                b'i' if !SOL && matches!(b.get(p + 2), Some(b'n' | b's')) && !is_word(b, p + 3) => {
                    self.op_token(p, 3, OPERATOR)
                }
                _ => self.op_token(p, 1, LOGICAL),
            },
            b'<' => {
                if c1 == b'=' {
                    return self.op_token(p, 2, COMPARISON);
                }
                if SOL {
                    return self.op_token(p, 1, COMPARISON);
                }
                self.lt(p)
            }
            b'>' => {
                if !SOL && matches!(self.top(), F_TARGS | F_TPARAMS | F_TBOUND) {
                    self.pop();
                    self.prev = P_NAME;
                    return p + 1;
                }
                if c1 == b'=' {
                    return self.op_token(p, 2, COMPARISON);
                }
                self.op_token(p, 1, COMPARISON)
            }
            b'-' => match c1 {
                b'>' => self.op_token(p, 2, DECLARATION),
                b'-' => {
                    self.put_hot(p, p + 2, INCDEC);
                    p + 2
                }
                b'=' => self.op_token(p, 2, ARITH_ASSIGN),
                _ => self.op_token(p, 1, ARITH),
            },
            b'+' => match c1 {
                b'+' => {
                    self.put_hot(p, p + 2, INCDEC);
                    p + 2
                }
                b'=' => self.op_token(p, 2, ARITH_ASSIGN),
                _ => self.op_token(p, 1, ARITH),
            },
            b'*' | b'%' => {
                if c1 == b'=' {
                    self.op_token(p, 2, ARITH_ASSIGN)
                } else {
                    self.op_token(p, 1, ARITH)
                }
            }
            b'&' if c1 == b'&' => self.op_token(p, 2, LOGICAL),
            b'|' if c1 == b'|' => self.op_token(p, 2, LOGICAL),
            _ => {
                self.prev = P_OP;
                p + 1
            }
        }
    }

    /// `:` at `p`: a parameter's or a type parameter's type, a declaration's type.
    #[inline(always)]
    fn colon(&mut self, p: usize) -> usize {
        self.put_hot(p, p + 1, DECLARATION);
        self.prev = P_OP;
        if SOL {
            return p + 1;
        }
        match self.top() {
            F_PARAMS => self.set_top(F_PTYPE),
            F_TPARAMS => self.set_top(F_TBOUND),
            _ => {
                if matches!(self.head, H_VAL | H_RET | H_CLASS | H_CLASS_HEAD) {
                    self.head = H_NONE;
                    self.ty = u16::from(self.depth) + 1;
                }
            }
        }
        p + 1
    }

    /// `=` at `p`: a parameter's default, or an assignment.
    #[inline(always)]
    fn equals(&mut self, p: usize) -> usize {
        self.prev = P_OP;
        if !SOL && matches!(self.top(), F_PARAMS | F_PTYPE) {
            self.set_top(F_PDEFAULT);
            self.put_hot(p, p + 1, DECLARATION);
            return p + 1;
        }
        self.head = H_NONE;
        if self.in_type() {
            self.ty = 0;
        }
        self.put_hot(p, p + 1, ASSIGNMENT);
        p + 1
    }

    /// `<` at `p`: type parameters or arguments, or a comparison.
    #[inline(always)]
    fn lt(&mut self, p: usize) -> usize {
        let b = self.b;
        let after_word = p > 0 && is_word(b, p - 1);
        let f = if is_type_frame(self.top()) || self.in_type() {
            Some(F_TARGS)
        } else {
            match self.head {
                H_FUN | H_VAL if !after_word => Some(F_TPARAMS),
                H_FUN | H_VAL => Some(F_TARGS),
                H_CLASS_HEAD => Some(F_TPARAMS),
                _ if after_word && self.prev == P_NAME && targs_end(b, p).is_some() => {
                    Some(F_TARGS)
                }
                _ => None,
            }
        };
        match f {
            Some(f) => {
                self.push(f);
                self.prev = P_OP;
                p + 1
            }
            None => self.op_token(p, 1, COMPARISON),
        }
    }

    /// A number at `p` (a digit, or `.` before one): Kotlin's literal, `1_000`, `0x1F`,
    /// `0b1`, `1.5e3f`, `1L`, `1u`, `1uL`.
    #[inline(always)]
    fn number(&mut self, p: usize) -> usize {
        let e = number_end(self.b, p);
        self.put_hot(p, e, NUMBER);
        self.prev = P_VALUE;
        e
    }

    /// A backticked name at `p`: a declaration's name in a head, a parameter's in a list; else
    /// plain.
    fn backtick(&mut self, p: usize) -> usize {
        let b = self.b;
        let Some(e) = crate::prim::find_in(b, p + 1, b.len(), *b"`")
            .checked_add(1)
            .filter(|&e| e <= b.len())
        else {
            return b.len();
        };
        if SOL {
            self.prev = P_NAME;
            return e;
        }
        let kind = match (self.top(), self.head) {
            (F_PARAMS, _) => PARAMETER,
            (F_TPARAMS, _) => GENERIC,
            (_, H_FUN) if !is_receiver(b, e) => {
                self.head = H_FUN_NAMED;
                FUNCTION
            }
            (_, H_VAL) if !is_receiver(b, e) => VARIABLE,
            (_, H_CLASS) => {
                self.head = H_CLASS_HEAD;
                CLASS_NAME
            }
            _ => PLAIN,
        };
        if kind != PLAIN {
            self.put(p, e, kind);
        }
        self.prev = P_NAME;
        e
    }
}

/// Whether a word ending at `e` is a declaration's receiver: `.` (not `..`), `?.` or `<` next.
fn is_receiver(b: &[u8], e: usize) -> bool {
    match b.get(e) {
        Some(b'.') => b.get(e + 1) != Some(&b'.'),
        Some(b'?') => b.get(e + 1) == Some(&b'.'),
        Some(b'<') => true,
        _ => false,
    }
}

/// Whether the `(` at `p` opens a receiver's type: its `)` (on the line, within [`LOOK`] bytes)
/// is right before `.` or `?.`.
fn receiver_paren(b: &[u8], p: usize) -> bool {
    let end = b.len().min(p + LOOK);
    let mut depth = 0u32;
    let mut i = p;
    while i < end {
        match b[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return match b.get(i + 1) {
                        Some(b'.') => true,
                        Some(b'?') => b.get(i + 2) == Some(&b'.'),
                        _ => false,
                    };
                }
            }
            b'"' | b'{' | b'}' | b'=' => return false,
            _ => {}
        }
        i += 1;
    }
    false
}

/// `[A-Z][A-Z0-9_]+`, a whole word: a constant's name.
fn is_constant(w: &[u8]) -> bool {
    w.len() >= 2
        && w[0].is_ascii_uppercase()
        && w[1..]
            .iter()
            .all(|&c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
}

/// Where the type arguments a `<` at `p` opens close, as Kotlin's parser reads them: only a
/// type's bytes (names, `.`, `,`, `?`, `*`, `&`, `@`, `:`, blanks, parentheses balanced, `->`)
/// to the `>` that closes it on the line, within [`LOOK`] bytes; `None` for a comparison.
fn targs_end(b: &[u8], p: usize) -> Option<usize> {
    let end = b.len().min(p + LOOK);
    let (mut angles, mut parens) = (1u32, 0u32);
    let mut i = p + 1;
    while i < end {
        let c = b[i];
        match c {
            b'<' => angles += 1,
            b'>' => {
                angles -= 1;
                if angles == 0 {
                    return Some(i + 1);
                }
            }
            b'(' => parens += 1,
            b')' => {
                if parens == 0 {
                    return None;
                }
                parens -= 1;
            }
            b'-' if b.get(i + 1) == Some(&b'>') => i += 1,
            // `T & Any`, not `&&`
            b'&' if b.get(i + 1) == Some(&b'&') => return None,
            b',' | b'.' | b'?' | b'*' | b'&' | b'@' | b':' | b' ' | b'\t' => {}
            _ if WORD[usize::from(c)] || c >= 0x80 => {}
            _ => return None,
        }
        i += 1;
    }
    None
}

/// Where Kotlin's number at `p` ends (a digit, or `.` before one).
fn number_end(b: &[u8], p: usize) -> usize {
    let n = b.len();
    let digits = |mut i: usize, ok: fn(u8) -> bool| {
        while i < n && (ok(b[i]) || b[i] == b'_') {
            i += 1;
        }
        i
    };
    let mut e;
    if b[p] == b'0' && matches!(b.get(p + 1), Some(b'x' | b'X')) {
        e = digits(p + 2, |c| c.is_ascii_hexdigit());
    } else if b[p] == b'0' && matches!(b.get(p + 1), Some(b'b' | b'B')) {
        e = digits(p + 2, |c| matches!(c, b'0' | b'1'));
    } else {
        e = if b[p] == b'.' {
            p
        } else {
            digits(p, |c| c.is_ascii_digit())
        };
        let mut float = false;
        if b.get(e) == Some(&b'.') && b.get(e + 1).is_some_and(u8::is_ascii_digit) {
            e = digits(e + 1, |c| c.is_ascii_digit());
            float = true;
        }
        if matches!(b.get(e), Some(b'e' | b'E')) {
            let mut s = e + 1;
            if matches!(b.get(s), Some(b'+' | b'-')) {
                s += 1;
            }
            if b.get(s).is_some_and(u8::is_ascii_digit) {
                e = digits(s, |c| c.is_ascii_digit());
                float = true;
            }
        }
        if matches!(b.get(e), Some(b'f' | b'F')) {
            return e + 1;
        }
        if float {
            return e;
        }
    }
    if matches!(b.get(e), Some(b'u' | b'U')) {
        e += 1;
    }
    if b.get(e) == Some(&b'L') {
        e += 1;
    }
    e
}
