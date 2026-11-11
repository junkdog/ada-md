//! Haskell on a scanner of ada's own (X-136), from a floor probe: one loop over a line's bytes, a
//! byte's class one load from a table, a name read once and looked up once in one table (the
//! keywords, the pragmas' words, the calls whose strings are SQL or a regex), an operator read
//! as Haskell's lexer reads one (a run of symbol characters), and the few constructs bat colors
//! by context read by small loops of their own: a type (a signature's, `name ::` at a line's
//! start, on to the lines indented past it; a record's field's; a GADT constructor's; an
//! annotation's, `e :: T`), a module's head and an import's with their export lists, a class's
//! head, an instance's, a deriving clause. Across lines the scan carries a word
//! ([`Scan::state`]): the open block comment's depth, a pragma, a quasi-quote's kind with its
//! SQL context or regex state, a string's gap, the head and the column it opened at. Kinds are
//! bat's leaves; `meta.preprocessor` (pragmas, CPP's lines) is the one meta scope a theme keys.
//!
//! **Where the language and bat's grammar part, the language's reading** (each a departure):
//! - `--` starts a comment only as Haskell's lexer has it, dashes alone (`-->`, `--|` and `--^`
//!   are operators, as Agda's and others' code defines them);
//! - an operator is the whole run of symbol characters (`<*>`, `&&`, `^.`, `@`, `#`: bat's class
//!   leaves `*`, `&`, `^`, `#` and `@` plain, so `<*>` read as `<`, plain `*`, `>`), and an
//!   operator in parentheses is a function's name whatever its characters (`(&&)`, `(<&>)`);
//! - a quasi-quote ends at its first `|]` (bat's greedy `.*` runs to the line's last), its quoter
//!   may be qualified (`[PG.sql|`), and `[x|x <- xs]` with no `|]` on its line is a list;
//! - Template Haskell's quotes (`[|`, `[e|`, `[d|`, `[t|`, `[p|`, `[||`) are code to their `|]`
//!   (bat's quasi-quoted text), and `forall` in a type a keyword (bat's a type variable);
//! - a string's gap (`\` at a line's end, `\` on the next) goes on as the string, and a string's
//!   control escape is `\^C` (bat's `\^…` takes `^C` anywhere in a string);
//! - a character literal is Haskell's (`'λ'`, `'\65'`, `'\^C'`), and a name's `'` is the name's
//!   (`x'`, `Foo'`);
//! - numbers with `_` (`1_000`), binary (`0b101`), hexadecimal floats (`0x1.8p3`);
//! - a class's head ends where its declaration does (a line not indented past it), not at the
//!   next `where` in the file, an instance's likewise, not at its line's end, and comments are
//!   comments in every head;
//! - a type is read wherever Haskell has one (X-143), not only after `name ::` at a line's
//!   start: `a, b ::`, `(==), (/=) ::`, `::` starting a line, a record's fields (`{ f :: T`,
//!   `, g :: T`, each to its `,` or `}`), a GADT's constructors (`Con ::`, its record's fields),
//!   `where f ::`, `let f ::`, `default f ::`, `pattern P ::`, and an annotation (`(e :: T)`, to
//!   its line's end or a bracket, `,`, `=`, `<-`, `|` or keyword outside its own); a type's
//!   `::` is its keyword wherever it stands;
//! - a deriving clause's classes are one reading with or without a strategy or parentheses
//!   (`deriving Show`, `deriving stock (Eq)`, bat's constants), its strategy and `via` keywords,
//!   `via`'s type a type; `foreign import` is no module's import;
//! - a module's head has no invalid words, and an export list's parentheses nest across lines;
//! - SQL in `[sql|…|]` and hasql-th's statement quasi-quotes, and in the strings of
//!   postgresql-simple's, sqlite-simple's and persistent's calls (`query conn "…"`, `rawSql "…"`);
//!   regexes in `[re|…|]` (PCRE, Java's dialect) and the strings after `=~` and `mkRegex`
//!   (regex-tdfa's and regex-compat's POSIX ERE).

#[cfg(test)]
mod tests;
mod text;

pub(crate) mod lhs;

use crate::emit::{Rules, Run, Runs, Sink, Spans};
use crate::prim::{char_len, is_word_starting};
use crate::regex;
use crate::sql;
use crate::words::{Words, text_len};
use crate::{Out, State};

/// The floor probe's build (X-136's `sol`, `ADA_PARSE_X136_SOL` set at build time): the loop, the
/// lookups, comments, pragmas, strings and quasi-quotes, without the heads (signatures, module,
/// import, class, instance, deriving) and the embeds.
const SOL: bool = option_env!("ADA_PARSE_X136_SOL").is_some();

/// The leaves: a kind's scopes.
static LEAVES: [&str; NK as usize] = [
    "",
    "comment.line.double-dash.haskell",
    "comment.line.double-dash.haskell punctuation.definition.comment.haskell",
    "comment.block.haskell",
    "comment.block.haskell punctuation.definition.comment.begin.haskell",
    "comment.block.haskell punctuation.definition.comment.end.haskell",
    "meta.preprocessor.haskell",
    "meta.preprocessor.haskell keyword.other.preprocessor.haskell",
    "meta.preprocessor.c",
    "meta.preprocessor.c punctuation.definition.preprocessor.c",
    "keyword.other.haskell",
    "keyword.control.haskell",
    "keyword.operator.haskell",
    "keyword.operator.function.infix.haskell",
    "keyword.operator.function.infix.haskell punctuation.definition.entity.haskell",
    "keyword.other.double-colon.haskell",
    "keyword.other.arrow.haskell",
    "keyword.other.big-arrow.haskell",
    "keyword.other.splice.haskell",
    "keyword.other.quasibracket.haskell",
    "string.quasiquoted.haskell",
    "constant.other.haskell",
    "storage.type.haskell",
    "variable.other.generic-type.haskell",
    "entity.name.function.haskell",
    "entity.name.function.infix.haskell",
    "support.other.module.haskell",
    "entity.other.inherited-class.haskell",
    "support.class.prelude.haskell",
    "constant.language.unit.haskell",
    "support.constant.unit.haskell",
    "constant.language.empty-list.haskell",
    "punctuation.separator.comma.haskell",
    "constant.numeric.integer.decimal.haskell",
    "constant.numeric.integer.hexadecimal.haskell",
    "constant.numeric.integer.hexadecimal.haskell punctuation.definition.numeric.base.haskell",
    "constant.numeric.integer.octal.haskell",
    "constant.numeric.integer.octal.haskell punctuation.definition.numeric.base.haskell",
    "constant.numeric.integer.binary.haskell",
    "constant.numeric.integer.binary.haskell punctuation.definition.numeric.base.haskell",
    "constant.numeric.float.decimal.haskell",
    "constant.numeric.float.decimal.haskell punctuation.separator.decimal.haskell",
    "string.quoted.double.haskell",
    "string.quoted.double.haskell punctuation.definition.string.begin.haskell",
    "string.quoted.double.haskell punctuation.definition.string.end.haskell",
    "string.quoted.double.haskell constant.character.escape.haskell",
    "string.quoted.double.haskell constant.character.escape.octal.haskell",
    "string.quoted.double.haskell constant.character.escape.control.haskell",
    "string.quoted.single.haskell",
    "string.quoted.single.haskell punctuation.definition.string.begin.haskell",
    "string.quoted.single.haskell punctuation.definition.string.end.haskell",
    "string.quoted.single.haskell constant.character.escape.haskell",
    "string.quoted.single.haskell constant.character.escape.octal.haskell",
    "string.quoted.single.haskell constant.character.escape.hexadecimal.haskell",
    "string.quoted.single.haskell constant.character.escape.control.haskell",
    // Literate Haskell's ([`lhs`]): a Bird track, `\begin{code}` and `\end{code}`
    "punctuation.definition.bird-track.haskell",
    "support.function.be.latex",
    "support.function.be.latex punctuation.definition.backslash.latex",
    "punctuation.definition.group.brace.begin.latex",
    "punctuation.definition.group.brace.end.latex",
    "variable.parameter.function.latex",
    "constant.numeric.float.hexadecimal.haskell",
    "constant.numeric.float.hexadecimal.haskell punctuation.definition.numeric.base.haskell",
];

const COMMENT: u16 = 1;
const COMMENT_MARK: u16 = 2;
const BLOCK: u16 = 3;
const BLOCK_BEGIN: u16 = 4;
const BLOCK_END: u16 = 5;
const PRAGMA: u16 = 6;
const PRAGMA_KW: u16 = 7;
const CPP: u16 = 8;
const CPP_MARK: u16 = 9;
const KEYWORD: u16 = 10;
const CONTROL: u16 = 11;
const OPERATOR: u16 = 12;
const INFIX_FN: u16 = 13;
const INFIX_TICK: u16 = 14;
const DOUBLE_COLON: u16 = 15;
const ARROW: u16 = 16;
const BIG_ARROW: u16 = 17;
const SPLICE: u16 = 18;
const QUASI_BRACKET: u16 = 19;
const QUASI: u16 = 20;
const CONSTANT: u16 = 21;
const STORAGE: u16 = 22;
const GENERIC: u16 = 23;
const FUNCTION: u16 = 24;
const FUNCTION_INFIX: u16 = 25;
const MODULE: u16 = 26;
const INHERITED: u16 = 27;
const PRELUDE: u16 = 28;
const UNIT: u16 = 29;
const UNIT_TYPE: u16 = 30;
const EMPTY_LIST: u16 = 31;
const COMMA: u16 = 32;
const INT: u16 = 33;
const HEX: u16 = 34;
const HEX_BASE: u16 = 35;
const OCT: u16 = 36;
const OCT_BASE: u16 = 37;
const BIN: u16 = 38;
const BIN_BASE: u16 = 39;
const FLOAT: u16 = 40;
const FLOAT_DOT: u16 = 41;
const STRING: u16 = 42;
const STRING_BEGIN: u16 = 43;
const STRING_END: u16 = 44;
const ESCAPE: u16 = 45;
const ESCAPE_NUM: u16 = 46;
const ESCAPE_CONTROL: u16 = 47;
const CHAR: u16 = 48;
const CHAR_BEGIN: u16 = 49;
const CHAR_END: u16 = 50;
const CHAR_ESCAPE: u16 = 51;
const CHAR_OCTAL: u16 = 52;
const CHAR_HEX: u16 = 53;
const CHAR_CONTROL: u16 = 54;
const BIRD: u16 = 55;
const LATEX_BE: u16 = 56;
const LATEX_BACKSLASH: u16 = 57;
const LATEX_BRACE_BEGIN: u16 = 58;
const LATEX_BRACE_END: u16 = 59;
const LATEX_ENV: u16 = 60;
const HEX_FLOAT: u16 = 61;
const HEX_FLOAT_BASE: u16 = 62;
/// How many leaves there are.
const NK: u16 = 63;

// The embedded languages' kinds, each a range above the leaves.
/// A regex's in a quasi-quote (`[re|…|]`), its text [`QUASI`]'s.
const RE_Q: u16 = 128;
/// A regex's in a string (after `=~`, `mkRegex`), its text [`STRING`]'s.
const RE_S: u16 = 256;
/// SQL's in a quasi-quote (`[sql|…|]`), its text [`QUASI`]'s.
const SQL_Q: u16 = 384;
/// SQL's in a string (`query conn "…"`), its text [`STRING`]'s.
const SQL_S: u16 = 512;
const SQL_END: u16 = 640;
const _: () = assert!(NK <= RE_Q);
const _: () = assert!(RE_Q as usize + regex::KINDS <= RE_S as usize);
const _: () = assert!(RE_S as usize + regex::KINDS <= SQL_Q as usize);
const _: () = assert!(sql::KINDS <= 128);

/// The scopes under `source.haskell` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    let split = |s: &'static str| s.split(' ').filter(|s| !s.is_empty());
    // an embedded language's scopes under its host's text: the string's or the quasi-quote's
    let host = |text: u16, inner: Vec<&'static str>| {
        let mut out: Vec<&str> = split(LEAVES[usize::from(text)]).collect();
        out.extend(inner);
        out
    };
    match kind {
        k if k >= SQL_END => Vec::new(),
        k if k >= SQL_S => match usize::from(k - SQL_S) {
            i if i < sql::KINDS => host(STRING, sql::scopes(k - SQL_S)),
            _ => Vec::new(),
        },
        k if k >= SQL_Q => match usize::from(k - SQL_Q) {
            i if i < sql::KINDS => host(QUASI, sql::scopes(k - SQL_Q)),
            _ => Vec::new(),
        },
        k if k >= RE_S => host(STRING, regex::scopes(k - RE_S)),
        k if k >= RE_Q => host(QUASI, regex::scopes(k - RE_Q)),
        k if k >= NK => Vec::new(),
        k => split(LEAVES[usize::from(k)]).collect(),
    }
}

// A byte's action ([`ACT`]).
const A_OTHER: u8 = 0;
/// ` `, `\t`, `\r`, `\x0b`, `\x0c`.
const A_SPACE: u8 = 1;
/// `[a-z_]`: a variable's name, a keyword.
const A_LOWER: u8 = 2;
/// `[A-Z]`: a constructor's, a type's or a module's name.
const A_UPPER: u8 = 3;
const A_DIGIT: u8 = 4;
const A_DQUOTE: u8 = 5;
const A_SQUOTE: u8 = 6;
const A_BACKTICK: u8 = 7;
const A_LPAREN: u8 = 8;
const A_LBRACKET: u8 = 9;
const A_LBRACE: u8 = 10;
const A_COMMA: u8 = 11;
/// A symbol character but `-`: `!#$%&*+./<=>?@\^|~:`.
const A_SYM: u8 = 12;
const A_DASH: u8 = 13;
/// `)`, `]`, `}`, `;`: plain, and a head's end where one is open.
const A_CLOSE: u8 = 14;
/// A byte past ASCII: a name's letter, an operator's (`→`, `∷`), or another.
const A_WIDE: u8 = 15;

static ACT: [u8; 256] = {
    let mut t = [A_WIDE; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = match b {
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => A_SPACE,
            b'a'..=b'z' | b'_' => A_LOWER,
            b'A'..=b'Z' => A_UPPER,
            b'0'..=b'9' => A_DIGIT,
            b'"' => A_DQUOTE,
            b'\'' => A_SQUOTE,
            b'`' => A_BACKTICK,
            b'(' => A_LPAREN,
            b'[' => A_LBRACKET,
            b'{' => A_LBRACE,
            b',' => A_COMMA,
            b'-' => A_DASH,
            b'!' | b'#' | b'$' | b'%' | b'&' | b'*' | b'+' | b'.' | b'/' | b'<' | b'=' | b'>'
            | b'?' | b'@' | b'\\' | b'^' | b'|' | b'~' | b':' => A_SYM,
            b')' | b']' | b'}' | b';' => A_CLOSE,
            _ => A_OTHER,
        };
        c += 1;
    }
    t
};

/// A name's bytes after its first in ASCII, `[A-Za-z0-9_']`.
static NAME: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = b.is_ascii_alphanumeric() || b == b'_' || b == b'\'';
        c += 1;
    }
    t
};

/// Haskell's symbol characters in ASCII, `!#$%&*+./<=>?@\^|-~:`.
static SYM: [bool; 256] = {
    let mut t = [false; 256];
    let s = b"!#$%&*+./<=>?@\\^|-~:";
    let mut i = 0;
    while i < s.len() {
        t[s[i] as usize] = true;
        i += 1;
    }
    t
};

/// Where the name that starts at `p` ends: `[A-Za-z0-9_']` and letters and digits past ASCII.
#[inline(always)]
fn name_end(b: &[u8], p: usize) -> usize {
    let n = b.len();
    let mut e = p + 1;
    loop {
        while e < n && NAME[usize::from(b[e])] {
            e += 1;
        }
        if e < n && b[e] >= 0x80 && is_word_starting(b, e) {
            e += char_len(b[e]);
        } else {
            return e.min(n);
        }
    }
}

/// [`name_end`] for a name whose first letter is past ASCII.
#[inline(never)]
fn wide_name_end(b: &[u8], p: usize) -> usize {
    let first = (p + char_len(b[p])).min(b.len());
    name_end(b, first - 1).max(first)
}

/// A wide character that is one of Haskell's symbols with an ASCII spelling (UnicodeSyntax):
/// `∷`, `→`, `⇒`, `←`, `∀`, `★`; how many bytes it takes and what it spells.
#[inline]
fn wide_symbol(b: &[u8], p: usize) -> Option<(usize, &'static [u8])> {
    let t = b.get(p..p + 3)?;
    Some((
        3,
        match t {
            [0xe2, 0x88, 0xb7] => b"::",
            [0xe2, 0x86, 0x92] => b"->",
            [0xe2, 0x87, 0x92] => b"=>",
            [0xe2, 0x86, 0x90] => b"<-",
            [0xe2, 0x88, 0x80] => b"forall",
            [0xe2, 0x98, 0x85] => b"*",
            _ => return None,
        },
    ))
}

/// Where the operator that starts at `p` ends: Haskell's symbol characters, the wide ones too.
#[inline(always)]
fn op_end(b: &[u8], p: usize) -> usize {
    let n = b.len();
    let mut e = p;
    loop {
        while e < n && SYM[usize::from(b[e])] {
            e += 1;
        }
        match wide_symbol(b, e) {
            Some((k, _)) if e < n => e += k,
            _ => return e,
        }
    }
}

#[inline(always)]
fn is_blank(c: u8) -> bool {
    ACT[usize::from(c)] == A_SPACE
}

/// Where the blanks at `p` end.
#[inline(always)]
fn blanks(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && is_blank(b[p]) {
        p += 1;
    }
    p
}

// A word's action, above its kind's byte ([`INFO`]).
const W_ACTION: u16 = 0xf << 8;
/// `module`: a module's head, to its `where`.
const W_MODULE: u16 = 1 << 8;
/// `import`: an import's head, to the line's end.
const W_IMPORT: u16 = 2 << 8;
/// `class`: a class's head, to its `where`.
const W_CLASS: u16 = 3 << 8;
/// `instance`: an instance's head, to its `where` or by layout.
const W_INSTANCE: u16 = 4 << 8;
/// `deriving`: a deriving clause, its strategy, its classes, its `via`.
const W_DERIVING: u16 = 5 << 8;
/// `where`: a head's end.
const W_WHERE: u16 = 6 << 8;
/// `qualified`, `as`, `hiding`: keywords in an import's head.
const W_IMPORT_KW: u16 = 7 << 8;
/// `forall`: a keyword in a type.
const W_FORALL: u16 = 8 << 8;
/// A call whose string is SQL ([`text`]): its first argument (`rawSql "…"`) or its second
/// (`query conn "…"`).
const W_SQL: u16 = 9 << 8;
/// A call whose string is a regex, POSIX's ERE: `mkRegex "…"`.
const W_REGEX: u16 = 10 << 8;
/// A deriving strategy, a keyword after `deriving`: `stock`, `newtype`, `anyclass`.
const W_STRATEGY: u16 = 11 << 8;
/// `via`: a keyword after `deriving` or its classes, a type after it.
const W_VIA: u16 = 12 << 8;
/// `let`, `default`: a signature may follow on their line (`let f :: Int`), as after `where`.
const W_DECL: u16 = 13 << 8;
/// `pattern` at a line's start before a capital: a pattern synonym's keyword, its signature.
const W_PATTERN: u16 = 14 << 8;
/// A pragma's word ([`PRAGMA_KW`] in `{-# … #-}`), a name elsewhere.
const W_PRAGMA: u16 = 1 << 12;
/// A class of the Prelude's: [`PRELUDE`] in a class's head.
const W_PRELUDE: u16 = 1 << 13;

const WORD_LIST: &[(&[u8], u16)] = &[
    (b"module", KEYWORD | W_MODULE),
    (b"import", KEYWORD | W_IMPORT),
    (b"class", KEYWORD | W_CLASS),
    (b"instance", KEYWORD | W_INSTANCE),
    (b"deriving", KEYWORD | W_DERIVING),
    (b"where", KEYWORD | W_WHERE),
    (b"data", KEYWORD),
    (b"type", KEYWORD),
    (b"case", KEYWORD),
    (b"of", KEYWORD),
    (b"let", KEYWORD | W_DECL),
    (b"in", KEYWORD),
    (b"newtype", KEYWORD | W_STRATEGY),
    (b"stock", W_STRATEGY),
    (b"anyclass", W_STRATEGY),
    (b"via", W_VIA),
    (b"default", KEYWORD | W_DECL),
    (b"pattern", W_PATTERN),
    (b"foreign", KEYWORD),
    (b"infix", OPERATOR),
    (b"infixl", OPERATOR),
    (b"infixr", OPERATOR),
    (b"do", CONTROL),
    (b"mdo", CONTROL),
    (b"if", CONTROL),
    (b"then", CONTROL),
    (b"else", CONTROL),
    (b"qualified", W_IMPORT_KW),
    (b"as", W_IMPORT_KW),
    (b"hiding", W_IMPORT_KW),
    (b"forall", W_FORALL),
    // postgresql-simple's, sqlite-simple's, mysql-simple's calls (a connection, then the query)
    (b"query", W_SQL),
    (b"query_", W_SQL),
    (b"queryWith", W_SQL),
    (b"queryWith_", W_SQL),
    (b"queryNamed", W_SQL),
    (b"execute", W_SQL),
    (b"execute_", W_SQL),
    (b"executeMany", W_SQL),
    (b"executeNamed", W_SQL),
    (b"returning", W_SQL),
    (b"fold_", W_SQL),
    (b"forEach_", W_SQL),
    // persistent's (the query first)
    (b"rawSql", W_SQL),
    (b"rawExecute", W_SQL),
    (b"rawExecuteCount", W_SQL),
    (b"rawQuery", W_SQL),
    // regex-compat's and regex-base's
    (b"mkRegex", W_REGEX),
    (b"mkRegexWithOpts", W_REGEX),
    (b"makeRegex", W_REGEX),
    // the pragmas' words bat lists
    (b"LANGUAGE", W_PRAGMA),
    (b"OPTIONS_GHC", W_PRAGMA),
    (b"INCLUDE", W_PRAGMA),
    (b"WARNING", W_PRAGMA),
    (b"DEPRECATED", W_PRAGMA),
    (b"MINIMAL", W_PRAGMA),
    (b"UNPACK", W_PRAGMA),
    (b"NOUNPACK", W_PRAGMA),
    (b"SOURCE", W_PRAGMA),
    (b"OVERLAPPING", W_PRAGMA),
    (b"OVERLAPPABLE", W_PRAGMA),
    (b"OVERLAPS", W_PRAGMA),
    (b"INCOHERENT", W_PRAGMA),
    (b"INLINE", W_PRAGMA),
    (b"NOINLINE", W_PRAGMA),
    (b"INLINABLE", W_PRAGMA),
    (b"CONLIKE", W_PRAGMA),
    (b"LINE", W_PRAGMA),
    (b"RULES", W_PRAGMA),
    (b"SPECIALIZE", W_PRAGMA),
    (b"SPECIALISE", W_PRAGMA),
    // the Prelude's classes, bat's list
    (b"Monad", W_PRELUDE),
    (b"Monoid", W_PRELUDE),
    (b"Functor", W_PRELUDE),
    (b"Applicative", W_PRELUDE),
    (b"Foldable", W_PRELUDE),
    (b"Traversable", W_PRELUDE),
    (b"Eq", W_PRELUDE),
    (b"Ord", W_PRELUDE),
    (b"Read", W_PRELUDE),
    (b"Show", W_PRELUDE),
    (b"Num", W_PRELUDE),
    (b"Fractional", W_PRELUDE),
    (b"Rational", W_PRELUDE),
    (b"Enum", W_PRELUDE),
    (b"Bounded", W_PRELUDE),
    (b"Real", W_PRELUDE),
    (b"RealFrac", W_PRELUDE),
    (b"RealFloat", W_PRELUDE),
    (b"Integral", W_PRELUDE),
    (b"Floating", W_PRELUDE),
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
/// What each word is by id: its kind in the low byte, its action and flags above.
static INFO: [u16; N + 1] = {
    let mut t = [0u16; N + 1];
    let mut i = 0;
    while i < N {
        t[i + 1] = WORD_LIST[i].1;
        i += 1;
    }
    t
};

/// What word `w` is ([`INFO`]): 0 for none.
#[inline(always)]
fn info(w: &[u8]) -> u16 {
    WORDS.value(w, &INFO)
}

// What a line starts in ([`Scan::mode`]).
const M_CODE: u8 = 0;
/// A block comment, [`Scan::nest`] deep.
const M_COMMENT: u8 = 1;
/// A pragma, `{-# … #-}`.
const M_PRAGMA: u8 = 2;
/// A quasi-quote's body, its kind in [`Scan::quasi`].
const M_QUASI: u8 = 3;
/// A string's gap, `\` at the line's end: the string goes on after the next line's `\`, its
/// kind in [`Scan::gap`].
const M_GAP: u8 = 4;

// A head the scan is in ([`Scan::head`]): what colors a name there.
const H_NONE: u8 = 0;
/// A type signature's type: after `name ::` or `a, b ::` at a line's start, a record's field
/// (`{ name ::`, `, name ::`), a GADT constructor (`Con ::`), or `::` starting a line; on to
/// the lines indented past [`Scan::indent`], to a `,`, `}`, `)`, `]` or `;` outside its
/// brackets ([`Scan::paren`] deep), or at a `=`, `<-`, `|` or a keyword outside them.
const H_SIG: u8 = 1;
/// A module's head, to its `where`; its export list where [`Scan::paren`] is past 0.
const H_MODULE: u8 = 2;
/// An import's head, to the line's end or a `;`; its list where [`Scan::paren`] is past 0.
const H_IMPORT: u8 = 3;
/// A class's head, to its `where` or a line not indented past [`Scan::indent`].
const H_CLASS: u8 = 4;
/// An instance's head, to its `where` or a line not indented past [`Scan::indent`] (bat's to
/// the line's end), a data instance's to its `=` ([`Scan::paren`] 1); `via`'s type in a
/// deriving clause.
const H_INSTANCE: u8 = 5;
/// A deriving clause: its strategy, its classes (in parentheses [`Scan::paren`] deep, or one
/// bare), then a `via` or its end; [`Scan::indent`] 1 once its classes are read.
const H_DERIVING: u8 = 6;
/// An annotation's type, `e :: T` past a line's start: [`H_SIG`]'s reading and ends, and the
/// line's end; a type application's bracketed type (`@(Maybe a)`, [`Scan::indent`] 1) to its
/// close.
const H_ANNOT: u8 = 7;

// A quasi-quote's kind ([`Scan::quasi`]).
/// Text: `[json|…|]`, `[r|…|]`.
const Q_TEXT: u8 = 0;
/// Template Haskell's quote, `[|…|]`, `[e|`, `[d|`, `[t|`, `[p|`: text with `$(` splices.
const Q_TH: u8 = 1;
/// SQL: `[sql|…|]`, hasql-th's statements.
const Q_SQL: u8 = 2;
/// A regex: `[re|…|]`, `[rex|…|]` (PCRE).
const Q_RE: u8 = 3;

// A string's kind across a gap ([`Scan::gap`]).
const S_PLAIN: u8 = 0;
const S_SQL: u8 = 1;
const S_RE: u8 = 2;

/// A SQL call's place carried to the next line ([`Scan::sqlc`]): right after the call's word,
/// or after its connection's name too.
const C_CALL: u8 = 1;
const C_ARG: u8 = 2;

/// Lex `b`, a line of Haskell without its `\n`, from `state`.
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
    s.lex_line::<SHARED>(0);
    s.k.close(s.out);
    *state = s.state();
    s.k
}

/// A block of Haskell on one scan opened from `state` ([`crate::Carry`]).
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
    // Carried across lines ([`Scan::state`]):
    /// What the line starts in ([`M_CODE`]…).
    mode: u8,
    /// The head the scan is in ([`H_NONE`]…).
    head: u8,
    /// An open block comment's depth, up to 255.
    nest: u8,
    /// The column a signature's or a class's head opened at (its line's blanks), up to 255; a
    /// deriving clause's place; 1 for a type application's bracketed type.
    indent: u8,
    /// How deep an export list's or a deriving clause's parentheses are, or a signature's
    /// brackets; 0 outside one, up to 255. 1 in a data instance's head.
    paren: u8,
    /// An open quasi-quote's kind ([`Q_TEXT`]…).
    quasi: u8,
    /// An open quasi-quote's or string's SQL context, or its regex's state.
    ctx: u8,
    /// A string's kind across its gap ([`S_PLAIN`]…).
    gap: u8,
    /// A SQL call's place the line before ended at ([`C_CALL`], [`C_ARG`]), 0 for none.
    sqlc: u8,
    /// How many quotes of Template Haskell's are open (`[|`, `[d|`…: code to their `|]`), up to
    /// 255.
    th: u8,
    /// Where a Literate Haskell's line stands ([`lhs::L_PROSE`]…).
    lit: u8,
    /// Whether the lines are Literate Haskell's ([`lhs`]): a block's line goes through its host.
    literate: bool,
    // The line's own:
    /// Where the line's code starts: 0, or past a Bird track's `>` ([`lhs`]).
    base: usize,
    /// Where the line's first token is, when the line starts in code: `^\s*`'s end. `usize::MAX`
    /// where it doesn't.
    lead: usize,
    /// Where the last SQL call's word ends on the line ([`W_SQL`]), `usize::MAX` for none; 0
    /// with [`Scan::sqlc`] carried.
    sqlw: usize,
    /// Where the last regex call's word or `=~` ends on the line, `usize::MAX` for none.
    rew: usize,
    /// Where the line's last `|]` and `<-` start, plus one ([`Scan::line_tail`]): `usize::MAX`
    /// until asked.
    tail: (usize, usize),
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
            mode: M_CODE,
            head: H_NONE,
            nest: 0,
            indent: 0,
            paren: 0,
            quasi: Q_TEXT,
            ctx: 0,
            gap: S_PLAIN,
            sqlc: 0,
            th: 0,
            lit: lhs::L_PROSE,
            literate: false,
            base: 0,
            lead: usize::MAX,
            sqlw: usize::MAX,
            rew: usize::MAX,
            tail: (usize::MAX, 0),
            sql: sql::Embedded {
                is_end: usize::MAX,
                at: 0,
            },
            k,
            out,
        }
    }

    /// The scan's fields from `state`: one word in its levels.
    #[inline(always)]
    fn load(&mut self, state: &State) {
        let w = state.word();
        self.mode = (w & 7) as u8;
        self.head = (w >> 3 & 7) as u8;
        self.quasi = (w >> 6 & 7) as u8;
        self.gap = (w >> 9 & 3) as u8;
        self.sqlc = (w >> 11 & 3) as u8;
        self.nest = (w >> 16) as u8;
        self.indent = (w >> 24) as u8;
        self.paren = (w >> 32) as u8;
        self.ctx = (w >> 40) as u8;
        self.th = (w >> 48) as u8;
        self.lit = state.withs[0];
    }

    /// The state the scan's last line left: every field as it is, so a block lexes as its
    /// lines do one at a time.
    #[inline(always)]
    fn state(&self) -> State {
        let mut s = State::default();
        s.set_word(
            u64::from(self.mode)
                | u64::from(self.head) << 3
                | u64::from(self.quasi) << 6
                | u64::from(self.gap) << 9
                | u64::from(self.sqlc) << 11
                | u64::from(self.nest) << 16
                | u64::from(self.indent) << 24
                | u64::from(self.paren) << 32
                | u64::from(self.ctx) << 40
                | u64::from(self.th) << 48,
        );
        s.withs[0] = self.lit;
        s
    }

    /// `b`, a block's next line, lexed on this scan, its sink closed; the scan's line an empty
    /// one again after.
    #[inline(always)]
    fn carry<const SHARED: bool>(&mut self, b: &'b [u8]) {
        self.k.start_line();
        self.b = b;
        if self.literate {
            self.literate_line::<SHARED>();
        } else {
            self.lex_line::<SHARED>(0);
        }
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

    /// The line from `base` (0, or past a Bird track), from what the line before left: a
    /// mode's rest first, a head's end by the line's indent; the loop out of line where
    /// `SHARED` ([`Scan::code_shared`]).
    #[inline(always)]
    fn lex_line<const SHARED: bool>(&mut self, base: usize) {
        self.base = base;
        self.sqlw = usize::MAX;
        self.rew = usize::MAX;
        self.lead = usize::MAX;
        self.tail.0 = usize::MAX;
        let p = match self.mode {
            M_CODE => {
                let lead = blanks(self.b, base);
                self.lead = lead;
                if !SOL && matches!(self.head, H_SIG | H_CLASS | H_INSTANCE) {
                    self.layout(lead);
                }
                if !SOL && self.sqlc != 0 {
                    self.sqlw = base;
                }
                base
            }
            M_COMMENT => self.comment(base),
            M_PRAGMA => self.pragma(base),
            M_QUASI => self.quasi_rest(base),
            _ => self.gap_rest(base),
        };
        if SHARED {
            self.code_shared(p);
        } else {
            self.run(p);
        }
        self.line_end();
    }

    /// A signature's, a class's or an instance's head at a line starting at `lead`: it goes on where the line
    /// is indented past it, blank or a comment; else it ends (bat's
    /// `^(?!\s*(?:--|{-|$)|\1\s)`).
    #[inline(always)]
    fn layout(&mut self, lead: usize) {
        let b = self.b;
        if lead == b.len() || lead - self.base > usize::from(self.indent) {
            return;
        }
        let rest = &b[lead..];
        if rest.starts_with(b"--") || rest.starts_with(b"{-") {
            return;
        }
        self.head = H_NONE;
    }

    /// What the line's end leaves: an import's head and an annotation's type end, a SQL call's
    /// place is carried.
    #[inline(always)]
    fn line_end(&mut self) {
        if self.head == H_ANNOT || (self.head == H_IMPORT && self.paren == 0) {
            self.head = H_NONE;
        }
        self.sqlc = 0;
        if !SOL && self.sqlw != usize::MAX && self.mode == M_CODE && self.head == H_NONE {
            self.sqlc = self.sql_place(self.b.len());
        }
    }

    /// [`Scan::run`] out of line: on the run sink, one copy that the line API and a block's
    /// lines share; on the span sink, which ada doesn't hold, inlined.
    #[inline(never)]
    fn code_shared(&mut self, p: usize) {
        self.run(p);
    }

    /// The line from `p`: code, or a head's loop while one is open.
    #[inline(always)]
    fn run(&mut self, mut p: usize) {
        let n = self.b.len();
        while p < n {
            p = if self.head == H_NONE {
                self.code(p)
            } else {
                self.head_code(p)
            };
        }
    }

    /// The code from `p` to the line's end, or to where a head opens.
    #[inline(always)]
    fn code(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        while p < n {
            let c = b[p];
            p = match ACT[usize::from(c)] {
                A_SPACE => blanks(b, p + 1),
                A_LOWER => {
                    let e = name_end(b, p);
                    let i = info(&b[p..e]);
                    if p == self.lead
                        && !SOL
                        && i & 0xff == 0
                        && let Some(q) = self.signature(p, e)
                    {
                        // `query :: …`: a name, though a call's word elsewhere
                        return q;
                    }
                    if i == 0 {
                        e
                    } else {
                        let q = self.word(p, e, i);
                        if self.head != H_NONE {
                            return q;
                        }
                        q
                    }
                }
                A_UPPER => {
                    // a constructor's, a type's or a module's name (`\b[A-Z]\w*\b`, its `'`s
                    // with it), read once; a GADT's constructors where `::` or `,` follows
                    let e = name_end(b, p);
                    if p == self.lead
                        && !SOL
                        && matches!(b.get(blanks(b, e)), Some(b',' | b':' | 0xe2))
                        && let Some(q) = self.decl_list(p, self.col(p))
                    {
                        // a GADT's constructors, `Con ::`
                        return q;
                    }
                    self.put_hot(p, e, CONSTANT);
                    e
                }
                A_DIGIT => self.number(p),
                A_DQUOTE => self.dquote(p),
                A_SQUOTE => self.squote(p),
                A_BACKTICK => self.backtick(p),
                A_LPAREN => {
                    if p == self.lead
                        && !SOL
                        && let Some(q) = self.op_signature(p)
                    {
                        return q;
                    }
                    self.paren(p)
                }
                A_LBRACKET => self.bracket(p),
                A_LBRACE => {
                    let q = self.brace(p);
                    if self.head != H_NONE {
                        return q;
                    }
                    q
                }
                A_COMMA => {
                    self.put_hot(p, p + 1, COMMA);
                    if p == self.lead
                        && !SOL
                        && let Some(q) = self.field(p + 1, self.col(p))
                    {
                        // a record's next field, `, name ::`
                        return q;
                    }
                    p + 1
                }
                A_DASH | A_SYM => {
                    let q = self.op(p);
                    if self.head != H_NONE {
                        return q;
                    }
                    q
                }
                A_WIDE => {
                    let q = self.wide(p);
                    if self.head != H_NONE {
                        return q;
                    }
                    q
                }
                _ => p + 1,
            };
        }
        n
    }

    /// A keyword or a call's word, `b[p..e]`, word `i` ([`INFO`]): its kind, its action.
    #[inline(always)]
    fn word(&mut self, p: usize, e: usize, i: u16) -> usize {
        let kind = i & 0xff;
        if kind != 0 {
            self.put_hot(p, e, kind);
        }
        if SOL {
            return e;
        }
        match i & W_ACTION {
            0 => e,
            W_MODULE => {
                self.head = H_MODULE;
                self.paren = 0;
                e
            }
            W_IMPORT if !after_word(&self.b[self.base..p], b"foreign") => {
                self.head = H_IMPORT;
                self.paren = 0;
                e
            }
            W_CLASS => {
                self.head = H_CLASS;
                self.indent = self.line_indent();
                e
            }
            W_INSTANCE => {
                self.head = H_INSTANCE;
                self.indent = self.line_indent();
                let before = &self.b[self.base..p];
                self.paren =
                    u8::from(after_word(before, b"data") || after_word(before, b"newtype"));
                e
            }
            W_DERIVING => {
                // its classes one reading with or without a strategy or parentheses (bat's only
                // after `deriving (`)
                self.head = H_DERIVING;
                self.paren = 0;
                self.indent = 0;
                e
            }
            W_SQL => {
                self.sqlw = e;
                e
            }
            W_REGEX => {
                self.rew = e;
                e
            }
            W_WHERE | W_DECL => self.keyword_sig(p, e).unwrap_or(e),
            W_PATTERN if p == self.lead => {
                let q = blanks(self.b, e);
                match self.b.get(q) {
                    Some(c) if c.is_ascii_uppercase() || *c == b'(' => {
                        self.put(p, e, KEYWORD);
                        self.decl_list(q, self.col(p)).unwrap_or(e)
                    }
                    _ => e,
                }
            }
            _ => e,
        }
    }

    /// A signature after `where`, `let` or `default` (the word `b[p..e]`) on its line:
    /// `where f :: T`, `let g, h :: T`, `default m :: T`; the type's layout a `let`'s or a
    /// `where`'s block's column (its name's), a `default`'s own.
    #[inline(always)]
    fn keyword_sig(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        let q = blanks(b, e);
        if q == e || !matches!(ACT[usize::from(*b.get(q)?)], A_LOWER | A_LPAREN) {
            return None;
        }
        let indent = if b[p] == b'd' {
            self.col(p)
        } else {
            self.col(q)
        };
        self.decl_list(q, indent)
    }

    /// The column the line's first token is at, from its code's start: a head's indent, up to
    /// 255.
    #[inline(always)]
    fn line_indent(&self) -> u8 {
        let lead = if self.lead == usize::MAX {
            blanks(self.b, self.base)
        } else {
            self.lead
        };
        (lead - self.base).min(255) as u8
    }

    /// `name ::` at the line's start, the name `b[p..e]`: a signature, `name` a function's
    /// name. Where the `::` ends, or `None` where the line isn't one.
    #[inline(always)]
    fn signature(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        let q = blanks(b, e);
        let Some(k) = double_colon(b, q) else {
            // `a, b :: T`
            return if b.get(q) == Some(&b',') {
                self.decl_list(p, self.col(p))
            } else {
                None
            };
        };
        self.put(p, e, FUNCTION);
        self.put(q, q + k, DOUBLE_COLON);
        self.open_sig(self.col(p));
        Some(q + k)
    }

    /// `(op) ::` at the line's start, `(` at `p`: a signature, whatever the operator's characters
    /// (bat's `\([|!%$+\-.,=</>]+\)` turns away `(.&&.)`), or a list of them (`(==), (/=) ::`).
    #[cold]
    #[inline(never)]
    fn op_signature(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if infix_name_end(b, p) == p {
            return None;
        }
        self.decl_list(p, self.col(p))
    }

    /// The column `p` is at from the line's code's start, up to 255.
    #[inline(always)]
    fn col(&self, p: usize) -> u8 {
        (p - self.base).min(255) as u8
    }

    /// A signature's type opens: its head, the column its layout keeps to, its brackets.
    #[inline(always)]
    fn open_sig(&mut self, indent: u8) {
        self.head = H_SIG;
        self.indent = indent;
        self.paren = 0;
    }

    /// A record's field after its `{` or `,`, from `p`: `name ::`, `a, b ::`, the signature's
    /// type opened with `indent` (the `{`'s or a leading `,`'s column). `None` where none is.
    #[inline(always)]
    fn field(&mut self, p: usize, indent: u8) -> Option<usize> {
        let q = blanks(self.b, p);
        if ACT[usize::from(*self.b.get(q)?)] != A_LOWER {
            return None;
        }
        self.decl_list(q, indent)
    }

    /// `{` at `p` in code, not a comment's: a record's first field after it, or plain.
    #[inline(never)]
    fn lbrace(&mut self, p: usize) -> usize {
        let indent = self.col(p);
        self.field(p + 1, indent).unwrap_or(p + 1)
    }

    /// The names a signature declares from `p`, then its `::`: `name ::`, `a, b ::`,
    /// `(==), (/=) ::`, a GADT's `Con ::` and `A, B ::`. The names a function's (a constructor's
    /// a constant as elsewhere), the commas, the `::`; the signature's type opened with `indent`.
    /// Where the `::` ends, or `None` (and nothing written) where there's no `::` after them.
    #[cold]
    #[inline(never)]
    fn decl_list(&mut self, p: usize, indent: u8) -> Option<usize> {
        let e = self.decl_names(p)?;
        self.open_sig(indent);
        Some(e)
    }

    /// [`Scan::decl_list`]'s names and `::` written, the type not opened: where the `::` ends.
    #[inline(always)]
    fn decl_names(&mut self, p: usize) -> Option<usize> {
        let (end, k) = decl_check(self.b, p)?;
        Some(self.decl_write(p, end, k))
    }

    /// [`Scan::decl_names`]' writes, `b[p..end]` the names and commas, `k` bytes of `::` after.
    #[inline(always)]
    fn decl_write(&mut self, p: usize, end: usize, k: usize) -> usize {
        let b = self.b;
        let mut q = p;
        while q < end {
            q = match ACT[usize::from(b[q])] {
                A_SPACE => blanks(b, q),
                A_COMMA => {
                    self.put(q, q + 1, COMMA);
                    q + 1
                }
                c => {
                    let e = decl_name_end(b, q).unwrap_or(q + 1);
                    self.put(q, e, if c == A_UPPER { CONSTANT } else { FUNCTION });
                    e
                }
            };
        }
        self.put(end, end + k, DOUBLE_COLON);
        end + k
    }

    /// A GADT record's field in a constructor's type, from `p` (after its `{`, or a `,` a
    /// bracket deep): its names and `::` as a field's, the type going on. Where the `::` ends.
    #[cold]
    #[inline(never)]
    fn type_field(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let q = blanks(b, p);
        if ACT[usize::from(*b.get(q)?)] != A_LOWER {
            return None;
        }
        let (end, k) = decl_check(b, q)?;
        if b[p - 1] == b',' {
            self.put(p - 1, p, COMMA);
        }
        Some(self.decl_write(q, end, k))
    }

    /// A number at `p`: bat's decimal, hexadecimal, octal and float forms, and Haskell's binary
    /// and `_`s; a number running into a name's byte is plain (bat's `\b`).
    #[inline(always)]
    fn number(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let digits = |mut q: usize, ok: fn(u8) -> bool| {
            while q < n && (ok(b[q]) || (b[q] == b'_' && q + 1 < n && ok(b[q + 1]))) {
                q += 1;
            }
            q
        };
        let ends = |q: usize| q >= n || !NAME[usize::from(b[q])] || b[q] == b'\'';
        if b[p] == b'0' && p + 2 < n {
            let (ok, kind, base): (fn(u8) -> bool, u16, u16) = match b[p + 1] {
                b'x' | b'X' => (|c| c.is_ascii_hexdigit(), HEX, HEX_BASE),
                b'o' | b'O' => (|c| matches!(c, b'0'..=b'7'), OCT, OCT_BASE),
                b'b' | b'B' => (|c| matches!(c, b'0' | b'1'), BIN, BIN_BASE),
                _ => (|_| false, 0, 0),
            };
            if kind != 0 && ok(b[p + 2]) {
                let mut e = digits(p + 2, ok);
                if kind == HEX && !SOL {
                    // HexFloatLiterals: `0x1.8p3`, `0x1p-2`
                    let mut float = false;
                    if e + 1 < n && b[e] == b'.' && b[e + 1].is_ascii_hexdigit() {
                        e = digits(e + 1, ok);
                        float = true;
                    }
                    if e < n && matches!(b[e], b'p' | b'P') {
                        let mut q = e + 1;
                        if q < n && matches!(b[q], b'+' | b'-') {
                            q += 1;
                        }
                        if q < n && b[q].is_ascii_digit() {
                            e = digits(q, |c| c.is_ascii_digit());
                            float = true;
                        }
                    }
                    if float && ends(e) {
                        self.put(p, p + 2, HEX_FLOAT_BASE);
                        self.put(p + 2, e, HEX_FLOAT);
                        return e;
                    }
                    if float {
                        return name_end(b, p);
                    }
                }
                if ends(e) {
                    self.put(p, p + 2, base);
                    self.put(p + 2, e, kind);
                    return e;
                }
                return name_end(b, p);
            }
        }
        let dec = |c: u8| c.is_ascii_digit();
        let mut e = digits(p, dec);
        let mut float = false;
        let mut dot = usize::MAX;
        if e + 1 < n && b[e] == b'.' && b[e + 1].is_ascii_digit() {
            dot = e;
            e = digits(e + 1, dec);
            float = true;
        }
        if e < n && matches!(b[e], b'e' | b'E') {
            let mut q = e + 1;
            if q < n && matches!(b[q], b'+' | b'-') {
                q += 1;
            }
            if q < n && b[q].is_ascii_digit() {
                e = digits(q, dec);
                float = true;
            }
        }
        if !ends(e) {
            // `12abc`: bat's `\b` turns it away
            return name_end(b, p);
        }
        if float {
            if dot != usize::MAX {
                self.put(p, dot, FLOAT);
                self.put(dot, dot + 1, FLOAT_DOT);
                self.put(dot + 1, e, FLOAT);
            } else {
                self.put(p, e, FLOAT);
            }
        } else {
            self.put_hot(p, e, INT);
        }
        e
    }

    /// `` ` `` at `p`: a name used as an operator, `` `div` ``, `` `M.member` ``; else plain.
    #[inline(never)]
    fn backtick(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut e = p + 1;
        while e < n {
            match ACT[usize::from(b[e])] {
                A_LOWER | A_UPPER => e = name_end(b, e),
                A_WIDE if is_word_starting(b, e) => e = wide_name_end(b, e),
                _ if b[e] == b'.' && e > p + 1 => e += 1,
                _ => break,
            }
        }
        if e < n && b[e] == b'`' {
            self.put(p, p + 1, INFIX_TICK);
            self.put(p + 1, e, INFIX_FN);
            self.put(e, e + 1, INFIX_TICK);
            return e + 1;
        }
        p + 1
    }

    /// `(` at `p`: `()`, an operator's name (`(+)`, `(<$>)`, `(,)`), or plain.
    #[inline(always)]
    fn paren(&mut self, p: usize) -> usize {
        let b = self.b;
        match b.get(p + 1) {
            Some(b')') => {
                self.put(p, p + 2, UNIT);
                p + 2
            }
            Some(&c) if SYM[usize::from(c)] || c == b',' || c >= 0x80 => {
                let e = infix_name_end(b, p);
                if e > p {
                    self.put(p, e, FUNCTION_INFIX);
                    e
                } else {
                    p + 1
                }
            }
            _ => p + 1,
        }
    }

    /// `[` at `p`: `[]`, a quasi-quote, or plain.
    #[inline(always)]
    fn bracket(&mut self, p: usize) -> usize {
        let b = self.b;
        match b.get(p + 1) {
            Some(b']') => {
                self.put(p, p + 2, EMPTY_LIST);
                p + 2
            }
            Some(&c) if c == b'|' || c.is_ascii_alphabetic() || c == b'_' => self.quasi_open(p),
            _ => p + 1,
        }
    }

    /// `{` at `p`: a pragma, a block comment, or plain.
    #[inline(always)]
    fn brace(&mut self, p: usize) -> usize {
        let b = self.b;
        if b.get(p + 1) != Some(&b'-') {
            if !SOL && self.head == H_NONE {
                return self.lbrace(p);
            }
            return p + 1;
        }
        if b.get(p + 2) == Some(&b'#') {
            self.put(p, p + 3, PRAGMA);
            self.mode = M_PRAGMA;
            return self.pragma(p + 3);
        }
        self.put(p, p + 2, BLOCK_BEGIN);
        self.mode = M_COMMENT;
        self.nest = 1;
        self.comment(p + 2)
    }

    /// An operator at `p`: a run of symbol characters, a comment where it's dashes alone, `$(`
    /// a splice, `#` at the line's start a CPP directive, `@` before a type a type application,
    /// `::` a type's start.
    #[inline(always)]
    fn op(&mut self, p: usize) -> usize {
        let b = self.b;
        let e = op_end(b, p);
        if self.th != 0 && b[p] == b'|' && b.get(e) == Some(&b']') && e - p <= 2 {
            return self.th_close(p, e);
        }
        if b[p] == b'-' && e > p + 1 && b[p + 1] == b'-' && b[p..e].iter().all(|&c| c == b'-') {
            return self.line_comment(p);
        }
        if SOL && b[p] == b'-' && b.get(p + 1) == Some(&b'-') {
            return self.line_comment(p);
        }
        if e == p + 1 {
            match b[p] {
                b'#' if p == self.lead => {
                    if let Some(q) = self.cpp(p) {
                        return q;
                    }
                }
                b'$' if b.get(e) == Some(&b'(') => {
                    self.put(p, p + 2, SPLICE);
                    return p + 2;
                }
                b'@' if !SOL => {
                    if let Some(q) = self.type_app(p) {
                        return q;
                    }
                }
                _ => {}
            }
        }
        if !SOL && e == p + 2 && b[p] == b'=' && b[p + 1] == b'~' {
            self.rew = e;
        }
        if !SOL && double_colon(b, p) == Some(e - p) {
            self.put(p, e, DOUBLE_COLON);
            if p == self.lead {
                // `::` starting a line: a signature whose name is on the line before; its type
                // on to the lines indented as far as the `::`
                self.open_sig(self.col(p).saturating_sub(1));
            } else {
                // `(e :: T)`, `x :: Int <- m`, `data T :: Type where`
                self.head = H_ANNOT;
                self.paren = 0;
                self.indent = 0;
            }
            return e;
        }
        self.put_hot(p, e, OPERATOR);
        e
    }

    /// `@` at `p` before a type, after a blank or a bracket (Haskell's lexer's type application,
    /// where `x@p` is an as-pattern): `@` an operator, the type after it a type's (a name, a
    /// qualified one, a type variable, or a bracketed type to its close: an annotation's head
    /// with [`Scan::indent`] 1). Where it ends, `None` where it isn't one.
    #[cold]
    #[inline(never)]
    fn type_app(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if p > self.base && !matches!(b[p - 1], b' ' | b'\t' | b'(' | b'[' | b',') {
            return None;
        }
        let q = p + 1;
        let c = *b.get(q)?;
        match ACT[usize::from(c)] {
            A_UPPER => {
                self.put(p, q, OPERATOR);
                let mut e = name_end(b, q);
                self.put(q, e, STORAGE);
                while e + 1 < b.len() && b[e] == b'.' && b[e + 1].is_ascii_uppercase() {
                    let s = e + 1;
                    e = name_end(b, s);
                    self.put(s, e, STORAGE);
                }
                Some(e)
            }
            A_LOWER if c != b'_' => {
                self.put(p, q, OPERATOR);
                let e = name_end(b, q);
                self.put(q, trim_primes(b, q, e), GENERIC);
                Some(e)
            }
            A_LPAREN | A_LBRACKET => {
                self.put(p, q, OPERATOR);
                if b.get(q + 1) == Some(&b')') {
                    self.put(q, q + 2, UNIT_TYPE);
                    return Some(q + 2);
                }
                self.head = H_ANNOT;
                self.paren = 0;
                self.indent = 1;
                Some(q)
            }
            _ => None,
        }
    }

    /// `#` at the line's start: a CPP directive (bat's `^\s*(#)\s*\w+`), its blanks, `#` and
    /// word under `meta.preprocessor.c`; the rest of the line is Haskell's.
    #[cold]
    #[inline(never)]
    fn cpp(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let q = blanks(b, p + 1);
        if q >= b.len() || !(crate::prim::WORD[usize::from(b[q])] || is_word_starting(b, q)) {
            return None;
        }
        let e = crate::prim::word_end(b, q);
        self.put(self.base, p, CPP);
        self.put(p, p + 1, CPP_MARK);
        self.put(p + 1, e, CPP);
        Some(e)
    }

    /// A byte past ASCII at `p`: a name's letter (a constructor's where it's uppercase), one of
    /// Haskell's symbols (`→`), or another character, plain.
    #[cold]
    #[inline(never)]
    fn wide(&mut self, p: usize) -> usize {
        let b = self.b;
        if is_word_starting(b, p) {
            let e = wide_name_end(b, p);
            if crate::prim::wide_char(b, p).is_some_and(char::is_uppercase) {
                self.put(p, e, CONSTANT);
            }
            return e;
        }
        if wide_symbol(b, p).is_some() {
            return self.op(p);
        }
        p + char_len(b[p]).min(b.len() - p)
    }

    /// A head's tokens from `p`, to the line's end or the head's: names, types and lists as
    /// bat's head contexts color them ([`H_SIG`]…).
    #[inline(never)]
    fn head_code(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        while p < n && self.head != H_NONE {
            let c = b[p];
            let head = self.head;
            if head == H_DERIVING && self.paren == 0 {
                p = self.deriving(p);
                continue;
            }
            p = match ACT[usize::from(c)] {
                A_SPACE => blanks(b, p + 1),
                A_LOWER => self.head_lower(p),
                A_UPPER => self.head_upper(p),
                A_LBRACE if is_type(head) && b.get(p + 1) != Some(&b'-') => {
                    // a GADT record's braces in a constructor's type
                    self.paren = self.paren.saturating_add(1);
                    self.type_field(p + 1).unwrap_or(p + 1)
                }
                A_LBRACE => self.brace(p),
                A_LBRACKET if is_type(head) => {
                    self.paren = self.paren.saturating_add(1);
                    p + 1
                }
                A_DASH if b.get(p + 1) == Some(&b'-') && dashes(b, p) => self.line_comment(p),
                A_SYM if c == b'#' && p == self.lead => match self.cpp(p) {
                    // a CPP line in a head: the rest of it isn't the head's
                    Some(_) => n,
                    None => op_end(b, p),
                },
                A_SYM if c == b'|' && self.th != 0 && b.get(p + 1) == Some(&b']') => {
                    self.head = H_NONE;
                    self.paren = 0;
                    self.th_close(p, p + 1)
                }
                A_DASH | A_SYM if matches!(head, H_SIG | H_ANNOT | H_INSTANCE) => self.type_op(p),
                A_WIDE
                    if matches!(head, H_SIG | H_ANNOT | H_INSTANCE)
                        && wide_symbol(b, p).is_some() =>
                {
                    self.type_op(p)
                }
                A_LPAREN => self.head_paren(p),
                A_CLOSE => self.head_close(p),
                A_COMMA if head == H_SIG && self.paren == 0 => {
                    // a record's field ends, the next may follow
                    self.head = H_NONE;
                    self.put(p, p + 1, COMMA);
                    self.field(p + 1, self.indent).unwrap_or(p + 1)
                }
                A_COMMA if head == H_ANNOT && self.paren == 0 => {
                    self.head = H_NONE;
                    p
                }
                A_COMMA if is_type(head) && self.paren == 1 => {
                    self.type_field(p + 1).unwrap_or(p + 1)
                }
                A_COMMA if self.exports() => {
                    self.put(p, p + 1, COMMA);
                    p + 1
                }
                A_DIGIT => name_end(b, p),
                A_WIDE if is_word_starting(b, p) => wide_name_end(b, p),
                A_WIDE => p + char_len(c).min(n - p),
                A_DASH | A_SYM => op_end(b, p),
                _ => p + 1,
            };
        }
        p
    }

    /// Whether the scan is in an export list's own level: a module's or an import's
    /// parentheses, not one inside them.
    #[inline(always)]
    fn exports(&self) -> bool {
        self.paren == 1 && matches!(self.head, H_MODULE | H_IMPORT)
    }

    /// A lowercase name at `p` in a head.
    #[inline(always)]
    fn head_lower(&mut self, p: usize) -> usize {
        let b = self.b;
        let e = name_end(b, p);
        let w = &b[p..e];
        let i = info(w);
        let head = self.head;
        if i & W_ACTION == W_WHERE
            && matches!(head, H_MODULE | H_CLASS | H_INSTANCE)
            && self.paren == 0
        {
            self.put(p, e, KEYWORD);
            self.head = H_NONE;
            return e;
        }
        match head {
            H_SIG | H_ANNOT if matches!(i & 0xff, KEYWORD | CONTROL) && self.paren == 0 => {
                // `where`, `of`, `in`, `then`, `deriving`: the type's end, left to the code
                self.head = H_NONE;
                return p;
            }
            H_SIG | H_ANNOT | H_INSTANCE => {
                if i & W_ACTION == W_FORALL || (head == H_INSTANCE && i & W_ACTION == W_INSTANCE) {
                    // `deriving via T instance C`'s `instance` too
                    self.put(p, e, KEYWORD);
                } else if b[p] != b'_' {
                    // bat's `\b[a-z][a-zA-Z0-9_']*\b`: a trailing `'` isn't the name's
                    self.put(p, trim_primes(b, p, e), GENERIC);
                }
            }
            H_CLASS => {
                if b[p] != b'_' {
                    self.put(p, trim_primes(b, p, e), GENERIC);
                }
            }
            H_MODULE | H_IMPORT if self.paren == 1 => self.put(p, e, FUNCTION),
            H_IMPORT if self.paren == 0 && i & W_ACTION == W_IMPORT_KW => self.put(p, e, KEYWORD),
            _ => {}
        }
        e
    }

    /// A capitalized name at `p` in a head; a module's name with its dots in a module's or an
    /// import's head.
    #[inline(always)]
    fn head_upper(&mut self, p: usize) -> usize {
        let b = self.b;
        let head = self.head;
        if matches!(head, H_MODULE | H_IMPORT) && self.paren == 0 {
            // bat's `[A-Z][A-Za-z._']*`
            let mut e = p + 1;
            while e < b.len() && (b[e].is_ascii_alphabetic() || matches!(b[e], b'.' | b'_' | b'\''))
            {
                e += 1;
            }
            self.put(p, e, MODULE);
            return e;
        }
        let e = name_end(b, p);
        match head {
            H_SIG | H_ANNOT | H_INSTANCE => self.put(p, e, STORAGE),
            H_MODULE | H_IMPORT if self.paren == 1 => self.put(p, e, STORAGE),
            H_CLASS => {
                let kind = if info(&b[p..e]) & W_PRELUDE != 0 {
                    PRELUDE
                } else {
                    INHERITED
                };
                self.put(p, e, kind);
            }
            H_DERIVING => self.put(p, e, INHERITED),
            _ => {}
        }
        e
    }

    /// An operator at `p` in a type: `->` and `=>` its arrows, others plain; `=`, `<-` and `|`
    /// outside a signature's or an annotation's brackets its end, left to the code, and `=` a
    /// data instance's head's.
    #[inline(always)]
    fn type_op(&mut self, p: usize) -> usize {
        let b = self.b;
        let e = op_end(b, p);
        let t = &b[p..e];
        let kind = match t {
            b"->" => ARROW,
            b"=>" => BIG_ARROW,
            b"=" | b"<-" | b"|" if is_type(self.head) && self.paren == 0 => {
                self.head = H_NONE;
                return p;
            }
            // `data instance T a` ⏎ `= C a`: the instance's head ends at its constructors (a
            // `type instance`'s right side a type)
            b"=" if self.head == H_INSTANCE && self.paren == 1 => {
                self.head = H_NONE;
                return p;
            }
            _ => match wide_symbol(b, p) {
                Some((3, b"->")) if e == p + 3 => ARROW,
                Some((3, b"=>")) if e == p + 3 => BIG_ARROW,
                Some((3, b"<-")) if e == p + 3 && is_type(self.head) && self.paren == 0 => {
                    self.head = H_NONE;
                    return p;
                }
                _ => 0,
            },
        };
        if kind != 0 {
            self.put(p, e, kind);
        }
        e
    }

    /// `(` at `p` in a head: `()` in a type, an export list's open, an operator's name in one.
    #[inline(always)]
    fn head_paren(&mut self, p: usize) -> usize {
        let b = self.b;
        match self.head {
            H_SIG | H_ANNOT | H_INSTANCE => {
                if b.get(p + 1) == Some(&b')') {
                    self.put(p, p + 2, UNIT_TYPE);
                    return p + 2;
                }
                if is_type(self.head) {
                    self.paren = self.paren.saturating_add(1);
                }
                p + 1
            }
            H_DERIVING => {
                self.paren = self.paren.saturating_add(1);
                p + 1
            }
            H_MODULE | H_IMPORT => {
                if self.paren == 1 {
                    let e = infix_name_end(b, p);
                    if e > p {
                        self.put(p, e, FUNCTION_INFIX);
                        return e;
                    }
                }
                self.paren = self.paren.saturating_add(1);
                p + 1
            }
            _ => p + 1,
        }
    }

    /// `)`, `]`, `}` or `;` at `p` in a head: an export list's close, an import's end, a
    /// deriving clause's classes read; a signature's bracket closed, or its type's end outside
    /// them (left to the code).
    #[inline(always)]
    fn head_close(&mut self, p: usize) -> usize {
        let c = self.b[p];
        match self.head {
            H_SIG | H_ANNOT if self.paren > 0 && c != b';' => {
                self.paren -= 1;
                if self.paren == 0 && self.head == H_ANNOT && self.indent == 1 {
                    // a type application's bracketed type, `@(Maybe a)`, read
                    self.head = H_NONE;
                }
            }
            H_SIG | H_ANNOT => {
                self.head = H_NONE;
                return p;
            }
            H_MODULE | H_IMPORT if c == b')' && self.paren > 0 => self.paren -= 1,
            H_IMPORT if c == b';' && self.paren == 0 => self.head = H_NONE,
            H_DERIVING if c == b')' => {
                self.paren -= 1;
                if self.paren == 0 {
                    self.indent = 1;
                }
            }
            _ => {}
        }
        p + 1
    }

    /// A deriving clause outside its parentheses, at `p`: its strategy (`stock`, `newtype`,
    /// `anyclass`) and `instance` before its classes, a bare class (qualified or not) or its
    /// parentheses' open, `via` with a type after it (an instance's head's reading); anything else ends it, left to the code.
    #[inline(never)]
    fn deriving(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let before = self.indent == 0;
        match ACT[usize::from(b[p])] {
            A_SPACE => blanks(b, p + 1),
            A_DASH if b.get(p + 1) == Some(&b'-') && dashes(b, p) => self.line_comment(p),
            A_LBRACE if b.get(p + 1) == Some(&b'-') => self.brace(p),
            A_SYM if b[p] == b'#' && p == self.lead && self.cpp(p).is_some() => n,
            A_LPAREN if before => {
                self.paren = 1;
                p + 1
            }
            A_UPPER if before => {
                let mut e = name_end(b, p);
                self.put(p, e, INHERITED);
                while e + 1 < n && b[e] == b'.' && b[e + 1].is_ascii_uppercase() {
                    let s = e + 1;
                    e = name_end(b, s);
                    self.put(s, e, INHERITED);
                }
                self.indent = 1;
                e
            }
            A_LOWER => {
                let e = name_end(b, p);
                let a = info(&b[p..e]) & W_ACTION;
                if a == W_VIA || (before && a == W_INSTANCE) {
                    self.put(p, e, KEYWORD);
                    self.head = H_INSTANCE;
                    self.indent = self.line_indent();
                    self.paren = 0;
                } else if before && a == W_STRATEGY {
                    self.put(p, e, KEYWORD);
                } else {
                    self.head = H_NONE;
                    return p;
                }
                e
            }
            _ => {
                self.head = H_NONE;
                p
            }
        }
    }
}

/// Whether `before`, a line's code before a keyword, ends in the word `w`: `foreign import`
/// (a foreign declaration, not a module's import), `data instance`.
#[inline(always)]
fn after_word(before: &[u8], w: &[u8]) -> bool {
    let mut e = before.len();
    while e > 0 && is_blank(before[e - 1]) {
        e -= 1;
    }
    before[..e].ends_with(w) && (e == w.len() || !NAME[usize::from(before[e - w.len() - 1])])
}

/// Whether `head` reads a type that ends outside its brackets: a signature's, an annotation's.
#[inline(always)]
fn is_type(head: u8) -> bool {
    matches!(head, H_SIG | H_ANNOT)
}

/// Whether the operator at `p` is dashes alone: a comment's start (`--`, `---`), where `-->`
/// isn't.
#[inline(always)]
fn dashes(b: &[u8], p: usize) -> bool {
    let e = op_end(b, p);
    b[p..e].iter().all(|&c| c == b'-')
}

/// `::` or `∷` at `q`: how many bytes it takes.
#[inline(always)]
fn double_colon(b: &[u8], q: usize) -> Option<usize> {
    // byte tests, no slice comparison: `op` asks at every operator
    match *b.get(q)? {
        b':' => (b.get(q + 1) == Some(&b':')).then_some(2),
        0xe2 => (b.get(q + 1) == Some(&0x88) && b.get(q + 2) == Some(&0xb7)).then_some(3),
        _ => None,
    }
}

/// The names a signature declares from `p` and its `::` ([`Scan::decl_list`]): where the `::`
/// starts and its length, `None` where there's none after them.
#[inline(always)]
fn decl_check(b: &[u8], p: usize) -> Option<(usize, usize)> {
    let mut q = p;
    loop {
        q = blanks(b, decl_name_end(b, q)?);
        if b.get(q) == Some(&b',') {
            q = blanks(b, q + 1);
        } else {
            return Some((q, double_colon(b, q)?));
        }
    }
}

/// Where a name a signature declares ends, from `q`: a name (a constructor's too), `(op)`;
/// `None` where none starts.
#[inline(always)]
fn decl_name_end(b: &[u8], q: usize) -> Option<usize> {
    match ACT[usize::from(*b.get(q)?)] {
        A_LOWER | A_UPPER => Some(name_end(b, q)),
        A_LPAREN => {
            let e = infix_name_end(b, q);
            (e > q).then_some(e)
        }
        _ => None,
    }
}

/// `(` at `p` followed by an operator and `)`, or commas (`(,)`): where the name ends, `p` for
/// none.
#[inline(always)]
fn infix_name_end(b: &[u8], p: usize) -> usize {
    let mut e = p + 1;
    if b.get(e) == Some(&b',') {
        while b.get(e) == Some(&b',') {
            e += 1;
        }
    } else {
        e = op_end(b, e);
    }
    if e > p + 1 && b.get(e) == Some(&b')') {
        e + 1
    } else {
        p
    }
}

/// A name `b[p..e]` without the `'`s that end it (bat's `\b` after a type variable's name).
#[inline(always)]
fn trim_primes(b: &[u8], p: usize, mut e: usize) -> usize {
    while e > p + 1 && b[e - 1] == b'\'' {
        e -= 1;
    }
    e
}
