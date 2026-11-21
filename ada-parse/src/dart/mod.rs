//! Dart on a scanner of ada's own (X-169), from a floor probe (the same files built with
//! `ADA_PARSE_X169_SOL`): one loop over a line's bytes, a byte's action one load from a table
//! ([`ACT`]), a name (`$` one of its letters) read once and looked up once in one table of every
//! word ([`WORDS`]: its leaf and, for a word whose meaning hangs on where it stands, an action),
//! the previous significant token in a byte ([`Scan::prev`]), and frames only for what nests
//! ([`Scan::frames`], five bits a level): braces, parentheses, brackets, a SQL call's and a
//! `RegExp(`'s arguments, and a string's `${…}`, which records the string it resumes when its
//! `}` closes. A block comment (nested, its depth kept) and a triple-quoted string continued past
//! a line's end are a mode ([`Scan::mode`]). Kinds are bat's leaves, and a few of ada's own where
//! bat has none (`late`, `required`, Dart 3's class modifiers, a symbol).
//!
//! Dart's reading where bat's grammar has none or derails: a `'…'` or `"…"` string ends with its
//! line (bat runs it on to the file's end); a `${…}` is code to its own `}`, past braces and
//! strings inside it (bat's `\$\{[^{}]+\}` reads none with a brace in it, and its quotes then end
//! the string); `get`, `set`, `on`, `show`, `type`, `when` and the other built-in identifiers are
//! keywords only where Dart's grammar makes them one (bat's everywhere: `x.set(1)`, `var set`); a
//! name is a function before `(`, or before type arguments and `(` (bat's before any `<`: `i<n`),
//! and after `get` and `set`, not before a switch arm's `=>`; dartdoc's `[ref]`, `` `code` ``
//! and fenced code in `///` comments as in `/** */` ones (bat's `///` is a line it never reads);
//! numbers with `_` separators, `1.` not a number before a member; Dart's escapes whole
//! (`\u{1F600}`, `\x41`). Regexes in `RegExp(`'s first argument ([`crate::regex`], ECMAScript's
//! dialect) and SQL in sqflite's and drift's calls ([`crate::sql`]).

#[cfg(test)]
mod tests;
mod text;

use crate::emit::{Rules, Run, Runs, Sink, Spans};
use crate::prim::{WORD, char_len};
use crate::regex;
use crate::sql;
use crate::words::{Words, text_len};
use crate::{Out, State};

/// The floor probe (X-169's `sol`): every word its table leaf, a call and a class their own, no
/// soft keyword, generic call, getter, directive, doc reference, SQL or regex read.
const SOL: bool = option_env!("ADA_PARSE_X169_SOL").is_some();

/// The leaves: a token's scopes.
static LEAVES: [&str; 61] = [
    "",
    // comments
    "comment.line.double-slash.dart",
    "comment.block.documentation.dart",
    "comment.block.documentation.dart variable.name.source.dart",
    "comment.block.documentation.dart variable.other.source.dart",
    "comment.block.dart",
    "comment.block.empty.dart punctuation.definition.comment.dart",
    "meta.preprocessor.script.dart",
    // strings by kind ([`Scan::string`]'s `sk & 7`): the text, an escape, an interpolated name
    "string.interpolated.single.dart",
    "string.interpolated.single.dart constant.character.escape.dart",
    "string.interpolated.single.dart variable.parameter.dart",
    "string.interpolated.double.dart",
    "string.interpolated.double.dart constant.character.escape.dart",
    "string.interpolated.double.dart variable.parameter.dart",
    "string.interpolated.triple.single.dart",
    "string.interpolated.triple.single.dart constant.character.escape.dart",
    "string.interpolated.triple.single.dart variable.parameter.dart",
    "string.interpolated.triple.double.dart",
    "string.interpolated.triple.double.dart constant.character.escape.dart",
    "string.interpolated.triple.double.dart variable.parameter.dart",
    "string.quoted.single.dart",
    "string.quoted.single.dart constant.character.escape.dart",
    "string.quoted.single.dart variable.parameter.dart",
    "string.quoted.double.dart",
    "string.quoted.double.dart constant.character.escape.dart",
    "string.quoted.double.dart variable.parameter.dart",
    "string.quoted.triple.single.dart",
    "string.quoted.triple.single.dart constant.character.escape.dart",
    "string.quoted.triple.single.dart variable.parameter.dart",
    "string.quoted.triple.double.dart",
    "string.quoted.triple.double.dart constant.character.escape.dart",
    "string.quoted.triple.double.dart variable.parameter.dart",
    // keywords
    "keyword.control.dart",
    "keyword.control.catch-exception.dart",
    "keyword.control.new.dart",
    "keyword.declaration.dart",
    "keyword.cast.dart",
    "keyword.other.import.dart",
    "keyword.operator.dart",
    "storage.modifier.dart",
    "storage.type.primitive.dart",
    "storage.type.annotation.dart",
    // names and literals
    "constant.language.dart",
    "variable.language.dart",
    "constant.numeric.dart",
    "constant.other.symbol.dart",
    "support.class.dart",
    "entity.name.function.dart",
    // operators and punctuation
    "keyword.operator.ternary.dart",
    "keyword.operator.bitwise.dart",
    "keyword.operator.assignment.bitwise.dart",
    "keyword.operator.closure.dart",
    "keyword.operator.comparison.dart",
    "keyword.operator.assignment.arithmetic.dart",
    "keyword.operator.assignment.dart",
    "keyword.operator.increment-decrement.dart",
    "keyword.operator.arithmetic.dart",
    "keyword.operator.logical.dart",
    "punctuation.comma.dart",
    "punctuation.terminator.dart",
    "punctuation.dot.dart",
];

// The leaves by name, in the order of `LEAVES`.
const LINE_COMMENT: u16 = 1;
const DOC: u16 = 2;
const DOC_REF: u16 = 3;
const DOC_CODE: u16 = 4;
const COMMENT: u16 = 5;
const EMPTY_COMMENT: u16 = 6;
const SHEBANG: u16 = 7;
/// A string's leaves: three a kind ([`str_leaf`]), its text, then these after it.
const STR: u16 = 8;
const ESCAPE: u16 = 1;
const PARAM: u16 = 2;
const CONTROL: u16 = 32;
const CATCH: u16 = 33;
const NEW: u16 = 34;
const DECLARATION: u16 = 35;
const CAST: u16 = 36;
const IMPORT: u16 = 37;
const IS: u16 = 38;
const MODIFIER: u16 = 39;
const PRIMITIVE: u16 = 40;
const ANNOTATION: u16 = 41;
const LANGUAGE: u16 = 42;
const VAR_LANGUAGE: u16 = 43;
const NUMBER: u16 = 44;
const SYMBOL: u16 = 45;
const CLASS: u16 = 46;
const FUNCTION: u16 = 47;
const TERNARY: u16 = 48;
const BITWISE: u16 = 49;
const ASSIGN_BITWISE: u16 = 50;
const CLOSURE: u16 = 51;
const COMPARISON: u16 = 52;
const ASSIGN_ARITH: u16 = 53;
const ASSIGN: u16 = 54;
const INCDEC: u16 = 55;
const ARITH: u16 = 56;
const LOGICAL: u16 = 57;
const COMMA: u16 = 58;
const TERMINATOR: u16 = 59;
const DOT: u16 = 60;

/// The text leaf of a string of kind `sk` (quote, raw; its embed's bits ignored).
#[inline(always)]
const fn str_leaf(sk: u8) -> u16 {
    STR + 3 * (sk & 7) as u16
}

/// A regex's kinds in a string of each kind (`sk & 7`), then SQL's.
const RE_BASE: u16 = 64;
const _: () = assert!(LEAVES.len() <= RE_BASE as usize);
const SQL_BASE: u16 = RE_BASE + 8 * regex::KINDS as u16;

/// The scopes under `source.dart` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    if kind >= SQL_BASE {
        let i = (kind - SQL_BASE) / sql::KINDS as u16;
        if i >= 8 {
            return Vec::new();
        }
        let mut out = scopes(str_leaf(i as u8));
        out.extend(sql::scopes((kind - SQL_BASE) % sql::KINDS as u16));
        return out;
    }
    if kind >= RE_BASE {
        let i = (kind - RE_BASE) / regex::KINDS as u16;
        let mut out = scopes(str_leaf(i as u8));
        out.extend(regex::scopes((kind - RE_BASE) % regex::KINDS as u16));
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
const A_QUOTE: u8 = 4;
const A_SLASH: u8 = 5;
const A_DOT: u8 = 6;
/// `+-*%&|^<>=!~?:`: an operator.
const A_OP: u8 = 7;
const A_LPAREN: u8 = 8;
const A_RPAREN: u8 = 9;
const A_LBRACE: u8 = 10;
const A_RBRACE: u8 = 11;
const A_LBRACKET: u8 = 12;
const A_RBRACKET: u8 = 13;
const A_COMMA: u8 = 14;
const A_SEMI: u8 = 15;
const A_AT: u8 = 16;
const A_HASH: u8 = 17;
/// A byte past ASCII: Dart's names are ASCII's, so a character of its own.
const A_WIDE: u8 = 18;

static ACT: [u8; 256] = {
    let mut t = [A_WIDE; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = match b {
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => A_SPACE,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' => A_NAME,
            b'0'..=b'9' => A_DIGIT,
            b'"' | b'\'' => A_QUOTE,
            b'/' => A_SLASH,
            b'.' => A_DOT,
            b'+' | b'-' | b'*' | b'%' | b'&' | b'|' | b'^' | b'<' | b'>' | b'=' | b'!' | b'~'
            | b'?' | b':' => A_OP,
            b'(' => A_LPAREN,
            b')' => A_RPAREN,
            b'{' => A_LBRACE,
            b'}' => A_RBRACE,
            b'[' => A_LBRACKET,
            b']' => A_RBRACKET,
            b',' => A_COMMA,
            b';' => A_SEMI,
            b'@' => A_AT,
            b'#' => A_HASH,
            _ => A_OTHER,
        };
        c += 1;
    }
    t
};

// What a word is ([`WORDS`], [`INFO`]): its leaf in the low byte, an action in the next bits.
const W_ACTION: u16 = 0xf << 8;
/// A reserved word: its leaf wherever it stands.
const W_RESERVED: u16 = 1 << 8;
/// A built-in identifier: a keyword where a name follows it (`required int`, `late final`,
/// `abstract class`, `on Exception`), a name elsewhere (`x.on(`, `var late`, `type: 1`).
const W_SOFT: u16 = 2 << 8;
/// `get`, `set`: as [`W_SOFT`], and the name after the keyword a function's.
const W_GETSET: u16 = 3 << 8;
/// `import`, `export`, `part`, `library`: a directive's head at a file's statement.
const W_IMPORT: u16 = 4 << 8;
/// `as`, `show`, `hide`, `deferred`, `of`: a directive's keywords; outside one `as` a cast.
const W_DIR: u16 = 5 << 8;
/// `async`, `sync`: a keyword before `{`, `*` or `=>`.
const W_ASYNC: u16 = 6 << 8;
/// `await`, `yield`: a keyword but as a member, a label or an assignment's target.
const W_AWAIT: u16 = 7 << 8;
/// `when`: a guard's keyword after a pattern.
const W_WHEN: u16 = 8 << 8;
/// `type`: a keyword after `extension`.
const W_TYPE: u16 = 9 << 8;
/// `extension`: as [`W_SOFT`], `type` after it a keyword.
const W_EXT: u16 = 10 << 8;
/// `operator`: a keyword before an operator's token.
const W_OPERATOR: u16 = 11 << 8;
/// `RegExp`: a class; its `(` opens a regex's arguments.
const W_REGEXP: u16 = 12 << 8;
/// A SQL method after a `.` (`db.execute(`).
const W_SQL_DOT: u16 = 13 << 8;
/// A SQL method with a receiver or without (drift's `customSelect(` in a database's class).
const W_SQL: u16 = 14 << 8;

/// The words, each with what it is.
const WORD_LIST: &[(&[u8], u16)] = &[
    // reserved words
    (b"assert", W_RESERVED | CONTROL),
    (b"break", W_RESERVED | CONTROL),
    (b"case", W_RESERVED | CONTROL),
    (b"continue", W_RESERVED | CONTROL),
    (b"default", W_RESERVED | CONTROL),
    (b"do", W_RESERVED | CONTROL),
    (b"else", W_RESERVED | CONTROL),
    (b"for", W_RESERVED | CONTROL),
    (b"if", W_RESERVED | CONTROL),
    (b"in", W_RESERVED | CONTROL),
    (b"return", W_RESERVED | CONTROL),
    (b"switch", W_RESERVED | CONTROL),
    (b"while", W_RESERVED | CONTROL),
    (b"try", W_RESERVED | CATCH),
    (b"catch", W_RESERVED | CATCH),
    (b"finally", W_RESERVED | CATCH),
    (b"throw", W_RESERVED | CATCH),
    (b"rethrow", W_RESERVED | CATCH),
    (b"new", W_RESERVED | NEW),
    (b"class", W_RESERVED | DECLARATION),
    (b"enum", W_RESERVED | DECLARATION),
    (b"extends", W_RESERVED | DECLARATION),
    (b"with", W_RESERVED | DECLARATION),
    (b"is", W_RESERVED | IS),
    (b"const", W_RESERVED | MODIFIER),
    (b"final", W_RESERVED | MODIFIER),
    (b"var", W_RESERVED | PRIMITIVE),
    (b"void", W_RESERVED | PRIMITIVE),
    (b"true", W_RESERVED | LANGUAGE),
    (b"false", W_RESERVED | LANGUAGE),
    (b"null", W_RESERVED | LANGUAGE),
    (b"this", W_RESERVED | VAR_LANGUAGE),
    (b"super", W_RESERVED | VAR_LANGUAGE),
    // built-in identifiers and contextual keywords
    (b"abstract", W_SOFT | DECLARATION),
    (b"base", W_SOFT | DECLARATION),
    (b"covariant", W_SOFT | DECLARATION),
    (b"external", W_SOFT | DECLARATION),
    (b"factory", W_SOFT | DECLARATION),
    (b"implements", W_SOFT | DECLARATION),
    (b"interface", W_SOFT | DECLARATION),
    (b"mixin", W_SOFT | DECLARATION),
    (b"native", W_SOFT | DECLARATION),
    (b"sealed", W_SOFT | DECLARATION),
    (b"typedef", W_SOFT | DECLARATION),
    (b"late", W_SOFT | MODIFIER),
    (b"required", W_SOFT | MODIFIER),
    (b"static", W_SOFT | MODIFIER),
    (b"on", W_SOFT | CATCH),
    (b"get", W_GETSET | DECLARATION),
    (b"set", W_GETSET | DECLARATION),
    (b"import", W_IMPORT | IMPORT),
    (b"export", W_IMPORT | IMPORT),
    (b"part", W_IMPORT | IMPORT),
    (b"library", W_IMPORT | IMPORT),
    (b"as", W_DIR | CAST),
    (b"show", W_DIR | IMPORT),
    (b"hide", W_DIR | IMPORT),
    (b"deferred", W_DIR | IMPORT),
    (b"of", W_DIR | IMPORT),
    (b"async", W_ASYNC | CONTROL),
    (b"sync", W_ASYNC | CONTROL),
    (b"await", W_AWAIT | CONTROL),
    (b"yield", W_AWAIT | CONTROL),
    (b"when", W_WHEN | CONTROL),
    (b"type", W_TYPE | DECLARATION),
    (b"extension", W_EXT | DECLARATION),
    (b"operator", W_OPERATOR | DECLARATION),
    // types bat colors as classes whose names aren't capitalized
    (b"bool", CLASS),
    (b"num", CLASS),
    (b"int", CLASS),
    (b"double", CLASS),
    (b"dynamic", CLASS),
    // embeds' calls ([`text`])
    (b"RegExp", W_REGEXP | CLASS),
    (b"execute", W_SQL_DOT),
    (b"rawQuery", W_SQL),
    (b"rawInsert", W_SQL),
    (b"rawUpdate", W_SQL),
    (b"rawDelete", W_SQL),
    (b"customSelect", W_SQL),
    (b"customStatement", W_SQL),
    (b"customUpdate", W_SQL),
    (b"customInsert", W_SQL),
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
/// `{` … `}`: a block, a class's body, a map's or a set's literal.
const F_BRACE: u8 = 1;
/// `(` … `)`.
const F_PAREN: u8 = 2;
/// `[` … `]`.
const F_BRACKET: u8 = 3;
/// A SQL call's arguments ([`Scan::sqls`] the argument's state).
const F_SQLCALL: u8 = 4;
/// `RegExp(`'s arguments, before its first `,`: a string a regex.
const F_RECALL: u8 = 5;
/// A string's `${` … `}`: the string's kind past this, its quote's two bits and its embed's
/// ([`text`]'s `E_*`) above them.
const F_INTERP: u8 = 16;
/// The bits of a level.
const FW: u32 = 5;
const FMASK: u128 = (1 << FW) - 1;
/// The levels [`Scan::frames`] holds.
const LEVELS: u8 = 25;

/// Whether frame `f` is a string's interpolation.
#[inline(always)]
const fn is_interp(f: u8) -> bool {
    f >= F_INTERP
}

/// Whether frame `f` is closed by `)`.
#[inline(always)]
fn is_paren_frame(f: u8) -> bool {
    matches!(f, F_PAREN | F_SQLCALL | F_RECALL)
}

// What the previous significant token was ([`Scan::prev`]).
/// A statement's start: a line's in a block, after `;`, `{`, `}`.
const P_START: u8 = 0;
/// An operator, `(`, `[`, `,`, a keyword: an operand may start.
const P_OP: u8 = 1;
/// A value's end: a name, a literal, `)`, `]`.
const P_VALUE: u8 = 2;
/// `.`, `?.`, `..`: a member's name follows.
const P_DOT: u8 = 3;
/// `extension`: `type` after it is a keyword.
const P_EXT: u8 = 4;
/// `get`, `set` as keywords: the name after them is a function's.
const P_GETSET: u8 = 5;

// What a line starts in ([`Scan::mode`]).
const M_CODE: u8 = 0;
/// A block comment ([`Scan::aux`] its depth).
const M_COMMENT: u8 = 1;
/// A documentation comment, `/** … */` ([`Scan::aux`] its depth).
const M_DOC: u8 = 2;
/// A triple-quoted string ([`Scan::aux`] its kind).
const M_STR: u8 = 3;
/// [`State::arg`]'s bits beside the mode: in a dartdoc's fenced code; in a directive.
const FENCE: u16 = 4;
const DIRECTIVE: u16 = 8;

/// Lex `b`, a line of Dart without its `\n`, from `state`.
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

/// A block of Dart on one scan opened from `state` ([`crate::Carry`]).
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
    /// A comment's depth ([`M_COMMENT`], [`M_DOC`]) or a string's kind ([`M_STR`]).
    aux: u8,
    /// A SQL string's context, or a regex's state, between its pieces.
    ctx: u8,
    /// The innermost SQL call's argument ([`text`]'s `S_*`).
    sqls: u8,
    /// How many interpolations are open, up to 255.
    inter: u8,
    /// In a dartdoc's fenced code (```` ``` ````).
    fence: bool,
    /// In a directive (`import … ;`): `as`, `show`, `hide`, `deferred`, `of` its keywords.
    dir: bool,
    /// The interpolated name's leaf of the innermost open interpolation's string, 0 for none:
    /// a plain name in `${…}` is its (bat's `variable.parameter`). Found from the frames.
    ipar: u16,
    // The line's own, reset at its start:
    /// The previous significant token ([`P_START`]…).
    prev: u8,
    /// What the `(` right after the last name opens: [`F_SQLCALL`], [`F_RECALL`], or 0.
    call: u8,
    /// What SQL's scan keeps between the host's turns on a line.
    sql: sql::Embedded,
    k: K,
    out: &'b mut Out<'c>,
}

/// Where a name at `p` ends: ASCII's letters, digits, `_` and `$` ([`name_run`]).
#[inline(always)]
fn name_end(b: &[u8], p: usize) -> usize {
    name_run(b, p + 1)
}

/// Whether `c` is one of a name's bytes, `[A-Za-z0-9_$]`.
#[inline(always)]
const fn is_name_byte(c: u8) -> bool {
    WORD[c as usize] || c == b'$'
}

/// Whether `c` can start a name, `[A-Za-z_$]`.
#[inline(always)]
const fn is_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b'$'
}

/// Where the run of a name's bytes (`[A-Za-z0-9_$]`) from `from` ends: sixteen bytes a step on
/// x86-64 (SSE2's compares, the line's last step read again from its end less sixteen, as
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
                let us = _mm_or_si128(
                    _mm_cmpeq_epi8(w, _mm_set1_epi8(b'_' as i8)),
                    _mm_cmpeq_epi8(w, _mm_set1_epi8(b'$' as i8)),
                );
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
    while e < n && is_name_byte(b[e]) {
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

/// The first byte past `e` that isn't a blank, `\n` at the line's end.
#[inline(always)]
fn next_byte(b: &[u8], e: usize) -> u8 {
    b.get(skip_blanks(b, e)).copied().unwrap_or(b'\n')
}

/// Whether the name `b[p..e]` is a class's by its look, as bat reads one: a capital first, past
/// leading `_`s and `$`s.
#[inline(always)]
fn capitalized(b: &[u8], p: usize, e: usize) -> bool {
    let mut i = p;
    while i < e && matches!(b[i], b'_' | b'$') {
        i += 1;
    }
    i < e && b[i].is_ascii_uppercase()
}

/// Whether the `<` at `p` after a name opens type arguments whose `>` is followed by `(`: a
/// generic call or declaration (`cast<int>()`, `T first<T>(…)`), not a comparison (`i<n`). Only
/// a type's bytes between, nested `<…>` and records' parentheses, on the line.
fn generic_call(b: &[u8], p: usize) -> bool {
    let end = b.len().min(p + 256);
    let mut depth = 0u32;
    let mut q = p;
    while q < end {
        match b[q] {
            b'<' => depth += 1,
            b'>' => {
                depth -= 1;
                if depth == 0 {
                    return b.get(q + 1) == Some(&b'(');
                }
            }
            b'a'..=b'z'
            | b'A'..=b'Z'
            | b'0'..=b'9'
            | b'_'
            | b'$'
            | b' '
            | b','
            | b'.'
            | b'?'
            | b'('
            | b')' => {}
            _ => return false,
        }
        q += 1;
    }
    false
}

impl<'b, 'c, K: Sink> Scan<'b, 'c, K> {
    #[inline(always)]
    fn new(b: &'b [u8], out: &'b mut Out<'c>, k: K) -> Self {
        Scan {
            b,
            frames: 0,
            depth: 0,
            mode: M_CODE,
            aux: 0,
            ctx: 0,
            sqls: 0,
            inter: 0,
            fence: false,
            dir: false,
            ipar: 0,
            prev: P_START,
            call: 0,
            sql: sql::Embedded {
                is_end: usize::MAX,
                at: 0,
            },
            k,
            out,
        }
    }

    /// The scan's fields from `state`: the frames in its levels and the first two withs, the
    /// embed's context, the SQL call's argument, the mode's depth or kind and the open
    /// interpolations in the next four, the mode and its bits in [`State::arg`].
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
        self.aux = state.withs[4];
        self.inter = state.withs[5];
        self.mode = (state.arg & 3) as u8;
        self.fence = state.arg & FENCE != 0;
        self.dir = state.arg & DIRECTIVE != 0;
        self.ipar = if self.inter != 0 { self.find_ipar() } else { 0 };
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
        s.withs[4] = self.aux;
        s.withs[5] = self.inter;
        s.depth = self.depth;
        s.arg = u16::from(self.mode)
            | if self.fence { FENCE } else { 0 }
            | if self.dir { DIRECTIVE } else { 0 };
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
        if self.depth > LEVELS && self.inter != 0 {
            // a level lost may have been an interpolation's
            self.ipar = self.find_ipar();
        }
    }

    #[inline(always)]
    fn pop(&mut self) {
        if self.depth > 0 {
            self.frames >>= FW;
            self.depth -= 1;
        }
    }

    /// The interpolated name's leaf of the innermost interpolation the frames hold, 0 for none.
    #[cold]
    #[inline(never)]
    fn find_ipar(&self) -> u16 {
        let levels = self.depth.min(LEVELS);
        let mut i = 0;
        while i < levels {
            let f = ((self.frames >> (FW * u32::from(i))) & FMASK) as u8;
            if is_interp(f) {
                return str_leaf(text::frame_kind(f)) + PARAM;
            }
            i += 1;
        }
        0
    }

    /// The previous token a line or a statement starts with, by the frame it's in.
    #[inline(always)]
    fn start(&mut self) {
        self.prev = if self.depth == 0 || self.top() == F_BRACE {
            P_START
        } else {
            P_OP
        };
    }

    /// The line, from what the line before left: a mode's rest first; the loop out of line
    /// where `SHARED` ([`Scan::code_shared`]).
    #[inline(always)]
    fn lex_line<const SHARED: bool>(&mut self) {
        self.start();
        self.call = 0;
        let p = match self.mode {
            M_CODE => {
                if self.fence {
                    self.fence_line();
                }
                0
            }
            M_STR => self.string_rest(),
            _ => self.comment(0),
        };
        if SHARED {
            self.code_shared(p);
        } else {
            self.code(p);
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
                A_QUOTE => self.string(p, p),
                A_SLASH => match b.get(p + 1) {
                    Some(b'/') => self.line_comment(p),
                    Some(b'*') => self.comment_open(p),
                    Some(b'=') => self.op_token(p, 2, ASSIGN_ARITH),
                    _ => self.op_token(p, 1, ARITH),
                },
                A_DOT => self.dot(p),
                A_OP => self.op(p),
                A_LPAREN => self.lparen(p),
                A_RPAREN => self.rparen(p),
                A_LBRACE => {
                    self.push(F_BRACE);
                    self.prev = P_START;
                    p + 1
                }
                A_RBRACE => self.rbrace(p),
                A_LBRACKET => {
                    self.push(F_BRACKET);
                    self.prev = P_OP;
                    p + 1
                }
                A_RBRACKET => self.rbracket(p),
                A_COMMA => self.comma(p),
                A_SEMI => {
                    self.put_hot(p, p + 1, TERMINATOR);
                    self.prev = P_START;
                    self.dir = false;
                    p + 1
                }
                A_AT => self.annotation(p),
                A_HASH => self.hash(p),
                A_WIDE => p + char_len(c).min(n - p),
                _ => {
                    self.prev = P_OP;
                    p + 1
                }
            };
            debug_assert!(p > from, "Dart's code loop stood at {from}");
        }
    }

    /// `n` bytes at `p` an operator of `kind`.
    #[inline(always)]
    fn op_token(&mut self, p: usize, n: usize, kind: u16) -> usize {
        self.put_hot(p, p + n, kind);
        self.prev = P_OP;
        p + n
    }

    /// An operator at `p`, by Dart's longest match, in bat's leaves (`?` and `:` its
    /// ternary's, so `??`, `?.`'s `?` and a named argument's `:` too).
    #[inline(always)]
    fn op(&mut self, p: usize) -> usize {
        let b = self.b;
        let c = b[p];
        let at = |i: usize| b.get(p + i).copied().unwrap_or(b'\n');
        let c1 = at(1);
        let (n, kind) = match c {
            b'=' => match c1 {
                b'=' => (2, COMPARISON),
                b'>' => (2, CLOSURE),
                _ => (1, ASSIGN),
            },
            b'!' => {
                if c1 == b'=' {
                    (2, COMPARISON)
                } else {
                    (1, LOGICAL)
                }
            }
            b'<' => match c1 {
                b'<' if at(2) == b'=' => (3, ASSIGN_BITWISE),
                b'<' => (2, BITWISE),
                b'=' => (2, COMPARISON),
                _ => (1, COMPARISON),
            },
            b'>' => match c1 {
                b'>' if at(2) == b'>' && at(3) == b'=' => (4, ASSIGN_BITWISE),
                b'>' if at(2) == b'>' => (3, BITWISE),
                b'>' if at(2) == b'=' => (3, ASSIGN_BITWISE),
                b'>' => (2, BITWISE),
                b'=' => (2, COMPARISON),
                _ => (1, COMPARISON),
            },
            b'&' | b'|' => {
                if c1 == c {
                    (2, LOGICAL)
                } else if c1 == b'=' {
                    (2, ASSIGN_BITWISE)
                } else {
                    (1, BITWISE)
                }
            }
            b'^' => {
                if c1 == b'=' {
                    (2, ASSIGN_BITWISE)
                } else {
                    (1, BITWISE)
                }
            }
            b'~' => match c1 {
                b'/' if at(2) == b'=' => (3, ASSIGN_ARITH),
                b'/' => (2, ARITH),
                _ => (1, BITWISE),
            },
            b'+' | b'-' => {
                if c1 == c {
                    (2, INCDEC)
                } else if c1 == b'=' {
                    (2, ASSIGN_ARITH)
                } else {
                    (1, ARITH)
                }
            }
            b'?' => {
                if c1 == b'?' {
                    (2, TERNARY)
                } else {
                    (1, TERNARY)
                }
            }
            b':' => (1, TERNARY),
            // `*`, `%`
            _ => {
                if c1 == b'=' {
                    (2, ASSIGN_ARITH)
                } else {
                    (1, ARITH)
                }
            }
        };
        self.put_hot(p, p + n, kind);
        self.prev = P_OP;
        p + n
    }

    /// `.` at `p`: an accessor, a cascade's `..`, a spread's `...`, or a double's `.5`.
    #[inline(always)]
    fn dot(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = match b.get(p + 1) {
            Some(b'0'..=b'9') => return self.number(p),
            Some(b'.') if b.get(p + 2) == Some(&b'.') => {
                self.put_hot(p, p + 3, DOT);
                self.prev = P_OP;
                return p + 3;
            }
            Some(b'.') => 2,
            _ => 1,
        };
        self.put_hot(p, p + n, DOT);
        self.prev = P_DOT;
        p + n
    }

    /// `,` at `p`.
    #[inline(always)]
    fn comma(&mut self, p: usize) -> usize {
        self.put_hot(p, p + 1, COMMA);
        self.prev = P_OP;
        if !SOL {
            match self.top() {
                F_SQLCALL => self.sqls = text::S_DONE,
                F_RECALL => self.set_top(F_PAREN),
                _ => {}
            }
        }
        p + 1
    }

    /// `(` at `p`: a SQL call's or `RegExp(`'s arguments right after its name, else a
    /// parenthesis.
    #[inline(always)]
    fn lparen(&mut self, p: usize) -> usize {
        self.prev = P_OP;
        if !SOL && self.call != 0 {
            let f = self.call;
            self.call = 0;
            if f == F_SQLCALL {
                self.sqls = text::S_SQL;
            }
            self.push(f);
            return p + 1;
        }
        self.push(F_PAREN);
        p + 1
    }

    /// `)` at `p`: a parenthesis's, with any bracket left open above it.
    #[inline(always)]
    fn rparen(&mut self, p: usize) -> usize {
        self.prev = P_VALUE;
        let mut f = self.top();
        if !is_paren_frame(f) {
            if f != F_BRACKET || self.depth == 0 {
                return p + 1;
            }
            // what the parenthesis left open
            while f == F_BRACKET && self.depth > 0 {
                self.pop();
                f = self.top();
            }
            if !is_paren_frame(f) {
                return p + 1;
            }
        }
        self.pop();
        if f == F_SQLCALL {
            self.sqls = text::S_DONE;
        }
        p + 1
    }

    /// `]` at `p`: a bracket's, with any parenthesis left open above it.
    #[inline(always)]
    fn rbracket(&mut self, p: usize) -> usize {
        self.prev = P_VALUE;
        let mut f = self.top();
        if f != F_BRACKET {
            if !is_paren_frame(f) || self.depth == 0 {
                return p + 1;
            }
            while is_paren_frame(f) && self.depth > 0 {
                self.pop();
                f = self.top();
            }
            if f != F_BRACKET {
                return p + 1;
            }
        }
        self.pop();
        p + 1
    }

    /// `}` at `p`: the innermost brace or interpolation closed, what it left open dropped; a
    /// stray one nothing. An interpolation's resumes its string.
    #[inline(always)]
    fn rbrace(&mut self, p: usize) -> usize {
        self.prev = P_START;
        let mut f = self.top();
        if f == F_BRACE {
            self.pop();
            return p + 1;
        }
        if !is_interp(f) {
            f = self.unwind_brace();
        }
        if is_interp(f) {
            return self.interp_close(p, f);
        }
        p + 1
    }

    /// A `}` with frames above its brace: they're dropped, if one is open below them; the frame
    /// it closes, or 0 (and nothing dropped).
    #[cold]
    fn unwind_brace(&mut self) -> u8 {
        let levels = self.depth.min(LEVELS);
        let mut i = 0;
        while i < levels {
            let f = ((self.frames >> (FW * u32::from(i))) & FMASK) as u8;
            if f == F_BRACE || is_interp(f) {
                for _ in 0..i {
                    self.pop();
                }
                if f == F_BRACE {
                    self.pop();
                }
                return f;
            }
            i += 1;
        }
        0
    }

    /// `@` at `p`: an annotation's name (`@override`, `@JsonKey(`), its arguments code.
    #[inline(always)]
    fn annotation(&mut self, p: usize) -> usize {
        let b = self.b;
        if b.get(p + 1).is_some_and(|&c| is_name_start(c)) {
            let e = name_end(b, p + 1);
            self.put(p, e, ANNOTATION);
            self.prev = P_START;
            return e;
        }
        self.prev = P_OP;
        p + 1
    }

    /// `#` at `p`: a symbol (`#foo`, `#foo.bar`, `#+`), or a script's `#!` line at the file's
    /// level.
    #[cold]
    fn hash(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        if p == 0 && b.get(1) == Some(&b'!') && self.depth == 0 {
            self.put(0, n, SHEBANG);
            return n;
        }
        self.prev = P_VALUE;
        let mut e = p + 1;
        if b.get(e).is_some_and(|&c| is_name_start(c)) {
            e = name_end(b, e);
            while b.get(e) == Some(&b'.') && b.get(e + 1).is_some_and(|&c| is_name_start(c)) {
                e = name_end(b, e + 1);
            }
        } else {
            // an operator's symbol: Dart's user-definable operators
            for op in [
                &b"[]="[..],
                b"[]",
                b">>>",
                b"<<",
                b">>",
                b"<=",
                b">=",
                b"==",
                b"~/",
                b"+",
                b"-",
                b"*",
                b"/",
                b"%",
                b"<",
                b">",
                b"&",
                b"|",
                b"^",
                b"~",
            ] {
                if b[e..].starts_with(op) {
                    e += op.len();
                    break;
                }
            }
            if e == p + 1 {
                self.prev = P_OP;
                return e;
            }
        }
        self.put(p, e, SYMBOL);
        e
    }

    /// A word at `p`: a raw string's `r`, a keyword, a class, a function, or a name.
    #[inline(always)]
    fn name(&mut self, p: usize) -> usize {
        let b = self.b;
        let e = name_end(b, p);
        if e == p + 1 && b[p] == b'r' && matches!(b.get(e), Some(b'\'' | b'"')) {
            return self.string(p, e);
        }
        let w = WORDS.value(&b[p..e], &INFO);
        if w & W_ACTION != 0 && !SOL {
            return self.word_action(p, e, w);
        }
        let leaf = w & 0xff;
        if leaf != 0 {
            self.put_hot(p, e, leaf);
            self.prev = if w & W_ACTION == 0 || matches!(leaf, LANGUAGE | VAR_LANGUAGE) {
                P_VALUE
            } else {
                P_OP
            };
            return e;
        }
        self.plain_name(p, e)
    }

    /// A name `b[p..e]` that isn't a keyword where it stands: a class by its capital, a
    /// function before `(` (or type arguments and `(`) and after `get`, an interpolation's
    /// name, or plain.
    #[inline(always)]
    fn plain_name(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let c = b.get(e).copied().unwrap_or(b'\n');
        let getter = self.prev == P_GETSET;
        self.prev = P_VALUE;
        if capitalized(b, p, e) {
            self.put_hot(p, e, CLASS);
            return e;
        }
        if c == b'(' || getter || (c == b'<' && !SOL && generic_call(b, e)) {
            // bat's: a function's name has a small letter first, past `_`s and `$`s
            if !b[p..e].iter().all(|&c| matches!(c, b'_' | b'$')) {
                self.put_hot(p, e, FUNCTION);
                return e;
            }
        }
        if self.ipar != 0 {
            self.put_hot(p, e, self.ipar);
        }
        e
    }

    /// A word `b[p..e]` whose meaning hangs on where it stands (`w` its info).
    #[inline(never)]
    fn word_action(&mut self, p: usize, e: usize, w: u16) -> usize {
        let b = self.b;
        let leaf = w & 0xff;
        let action = w & W_ACTION;
        let dot = self.prev == P_DOT;
        if action == W_RESERVED {
            self.put_hot(p, e, leaf);
            self.prev = if matches!(leaf, LANGUAGE | VAR_LANGUAGE) {
                P_VALUE
            } else {
                P_OP
            };
            return e;
        }
        // a name follows, or a record's type after a blank (`static (int, int) f()`; a call has
        // none: `factory(_)`): a built-in identifier is a modifier, a declaration's keyword
        let named = || {
            let q = skip_blanks(b, e);
            b.get(q)
                .is_some_and(|&c| is_name_start(c) || (c == b'(' && q > e))
        };
        let (keyword, prev) = match action {
            W_SOFT => (!dot && named(), P_OP),
            W_GETSET => (!dot && named(), P_GETSET),
            W_IMPORT => {
                // `import 'a.dart'`, `part of x;`, `library;` at the file's level
                let c = next_byte(b, e);
                let head = !dot
                    && self.depth == 0
                    && self.prev == P_START
                    && (matches!(c, b'\'' | b'"' | b';') || is_name_start(c));
                if head {
                    self.dir = true;
                }
                (head, P_OP)
            }
            W_DIR => {
                if self.dir && !dot {
                    // `as` too: the import's keyword
                    self.put_hot(p, e, IMPORT);
                    self.prev = P_OP;
                    return e;
                } else if leaf == CAST {
                    // `x as T`, `as (int, int)`
                    (
                        !dot && matches!(next_byte(b, e), b'(' | b'_' | b'$' | b'a'..=b'z' | b'A'..=b'Z'),
                        P_OP,
                    )
                } else {
                    (false, P_OP)
                }
            }
            W_ASYNC => (!dot && matches!(next_byte(b, e), b'{' | b'*' | b'='), P_OP),
            W_AWAIT => {
                let q = skip_blanks(b, e);
                let c = b.get(q).copied().unwrap_or(b'\n');
                let c1 = b.get(q + 1).copied().unwrap_or(b'\n');
                let name = matches!(c, b'.' | b',' | b';' | b':' | b')' | b']' | b'}' | b'\n')
                    || (c == b'=' && !matches!(c1, b'=' | b'>'));
                (!dot && !name, P_OP)
            }
            W_WHEN => (
                !dot && self.prev == P_VALUE && matches!(b.get(e), Some(b' ' | b'\t')),
                P_OP,
            ),
            W_TYPE => (self.prev == P_EXT, P_OP),
            W_EXT => (!dot && named(), P_EXT),
            W_OPERATOR => (
                !dot && matches!(
                    next_byte(b, e),
                    b'=' | b'+'
                        | b'-'
                        | b'*'
                        | b'/'
                        | b'%'
                        | b'~'
                        | b'<'
                        | b'>'
                        | b'['
                        | b'&'
                        | b'|'
                        | b'^'
                ),
                P_OP,
            ),
            W_REGEXP => {
                if b.get(e) == Some(&b'(') {
                    self.call = F_RECALL;
                }
                self.put_hot(p, e, CLASS);
                self.prev = P_VALUE;
                return e;
            }
            // W_SQL, W_SQL_DOT
            _ => {
                if b.get(e) == Some(&b'(') && (dot || action == W_SQL) {
                    self.call = F_SQLCALL;
                }
                (false, P_VALUE)
            }
        };
        if keyword {
            self.put_hot(p, e, leaf);
            self.prev = prev;
            return e;
        }
        self.plain_name(p, e)
    }
}
