//! CMake on a scanner of ada's own (X-104), from a floor probe: one loop over a line's bytes, a
//! byte's class one load from a table, a command's name read once and looked up once (ASCII case
//! folded, as bat's `(?i)` patterns are), an argument's word read once and looked up once in one
//! table of every keyword with the argument lists it is in, a bit each. Frames, four bits a level
//! ([`Scan::frames`]), only for what nests or runs across lines: the blocks (`if` … `endif`,
//! `foreach`, `while`, `function`, `macro`), a command's arguments, a parenthesis inside them, a
//! quoted string, `${…}` and `$<…>`, a generic command's capitalized argument (bat's runs to a
//! blank, across lines). Bracket comments, bracket arguments and a `MATCHES` regex are a mode
//! ([`Scan::mode`]) with their `=`s. Kinds are bat's leaves, but for the keywords' and commands'
//! names, which bat scopes one by one (`support.function.add_library.cmake`) and no theme tells
//! apart: one leaf each.
//!
//! **A `MATCHES [[…]]` regex is read as CMake reads it** (X-106, [`Scan::regex`]): its syntax is
//! Henry Spencer's (`^ $ . [] [^] * + ? | () \c`, a `\` in a class a character, `]` or `-` first
//! in one a character), in the scopes of bat's `Regular Expression`, and it ends at the bracket's
//! close wherever that stands, as CMake's bracket argument does. bat lexes the body as a PCRE:
//! its classes take a `\` as an escape and run past the close (`[;\]]=]`), its top level leaves
//! quantifiers and `.` plain, and it reads `{n}`, `\d`, `(?:` and `[:alpha:]`, which CMake hasn't.
//!
//! **So is one in a quoted or unquoted argument** (X-107): after `MATCHES`, after `REGEX` in
//! `file(…)`, `list(…)` and `install(…)`, after `string(REGEX MATCH|MATCHALL|REPLACE`, after
//! `source_group(… REGULAR_EXPRESSION`, on the keyword's line or a later one (X-108). CMake's
//! escapes come first (`"\\."` is the regex's `\.`, `"\."` its `.`), a `${…}` or `$<…>` is an
//! operand in CMake's colors, and the tokens sit under the string's scope where it is quoted. bat
//! leaves these arguments plain, and reads a `MATCHES [[…]]` on its keyword's line only.
//!
//! **Escapes are CMake's** (X-109): `\` before any character but a letter or digit is that
//! character, `\t`, `\r`, `\n` and `\;` are CMake's, another letter or digit is an error, in a
//! quoted argument and an unquoted one alike. bat's table calls `"\."` and `"\/"` invalid.
//!
//! **A command is read as CMake parses one** (X-119, `cmake-language(7)`): its name, blanks and
//! its `(` on the same line, or the line's rest an error and the command over (bat waits across
//! lines, and lets `set` and a generic command take anything until a `(`); a `(` in any command's
//! arguments nests to its `)` (bat ends a builtin's arguments at the first `)`, the `(` an error);
//! after a command's `)` or a bracket comment only blanks and comments, and at a command's place
//! only a name: anything else is the line's rest an error ([`Scan::stray`]). `@ONLY` is
//! `configure_file`'s and `file`'s keyword, `debug`, `optimized` and `general`
//! `target_link_libraries`'s too (bat's `\b@ONLY\b` never matches, and its
//! `target_link_libraries` lacks them).

use crate::emit::{Rules, Run, Runs, Sink, Spans};
use crate::prim::{char_len, find_any, is_name_start, is_word_starting, word_end};
use crate::words::{Words, text_len};
use crate::{Out, State};

/// The leaves: a token's scopes.
static LEAVES: [&str; 104] = [
    "",
    // comments: a line's text and its mark; a bracket comment's text, begin, end
    "comment.line.cmake",
    "comment.line.cmake punctuation.definition.comment.cmake",
    "comment.block.cmake",
    "comment.block.cmake punctuation.definition.comment.begin.cmake",
    "comment.block.cmake punctuation.definition.comment.end.cmake",
    // the same inside a generic command's capitalized argument
    "variable.parameter.generic.cmake comment.line.cmake",
    "variable.parameter.generic.cmake comment.line.cmake punctuation.definition.comment.cmake",
    "variable.parameter.generic.cmake comment.block.cmake",
    "variable.parameter.generic.cmake comment.block.cmake punctuation.definition.comment.begin.cmake",
    "variable.parameter.generic.cmake comment.block.cmake punctuation.definition.comment.end.cmake",
    // a quoted string
    "string.quoted.double.cmake",
    "string.quoted.double.cmake punctuation.definition.string.begin.cmake",
    "string.quoted.double.cmake punctuation.definition.string.end.cmake",
    "string.quoted.double.cmake constant.character.escape.cmake",
    "string.quoted.double.cmake invalid.illegal.character.escape.cmake",
    "string.quoted.double.cmake punctuation.separator.cmake",
    // a bracket argument
    "string.raw.cmake",
    "string.raw.cmake punctuation.definition.string.begin.cmake",
    "string.raw.cmake punctuation.definition.string.end.cmake",
    // a bracket argument after `MATCHES`: its `[[` the regex's, its `=`s the begin
    "string.regexp.cmake",
    "string.regexp.cmake punctuation.definition.string.begin.cmake",
    "string.regexp.cmake punctuation.definition.string.end.cmake",
    // an unquoted argument's
    "constant.character.escape.cmake",
    "invalid.illegal.character.escape.cmake",
    "punctuation.separator.cmake",
    // a command's
    "punctuation.section.parens.begin.cmake",
    "punctuation.section.parens.end.cmake",
    "invalid.illegal.stray.parenthesis.cmake",
    "invalid.illegal.expected-opening-brace.cmake",
    "invalid.illegal.stray.cmake",
    "invalid.illegal.expected-type.cmake",
    "support.function.cmake",
    "keyword.control.cmake",
    "keyword.control.import.cmake",
    "variable.function.generic.cmake",
    "variable.parameter.cmake",
    "variable.parameter.generic.cmake",
    "keyword.operator.logical.cmake",
    "variable.other.readwrite.assignment.cmake",
    "entity.name.function.cmake",
    "storage.type.cmake",
    // bat's two-word keywords, a scope a word
    "variable.parameter.USES_TERMINAL COMMAND_EXPAND_LISTS.cmake",
    "variable.parameter.INCLUDES DESTINATION.cmake",
    // `${…}` and `$<…>`: bare, in a quoted string, in a capitalized argument ([`PRE_STRING`],
    // [`PRE_CAPS`])
    "punctuation.definition.variable.substitution.cmake",
    "keyword.operator.word.cmake",
    "punctuation.section.braces.begin.cmake",
    "punctuation.section.braces.end.cmake",
    "variable.other.readwrite.cmake",
    "punctuation.definition.variable.generator-expression.cmake",
    "punctuation.section.block.begin.cmake",
    "punctuation.section.block.end.cmake",
    "punctuation.separator.generator-expression.cmake",
    "string.quoted.double.cmake punctuation.definition.variable.substitution.cmake",
    "string.quoted.double.cmake keyword.operator.word.cmake",
    "string.quoted.double.cmake punctuation.section.braces.begin.cmake",
    "string.quoted.double.cmake punctuation.section.braces.end.cmake",
    "string.quoted.double.cmake variable.other.readwrite.cmake",
    "string.quoted.double.cmake punctuation.definition.variable.generator-expression.cmake",
    "string.quoted.double.cmake punctuation.section.block.begin.cmake",
    "string.quoted.double.cmake punctuation.section.block.end.cmake",
    "string.quoted.double.cmake punctuation.separator.generator-expression.cmake",
    "variable.parameter.generic.cmake punctuation.definition.variable.substitution.cmake",
    "variable.parameter.generic.cmake keyword.operator.word.cmake",
    "variable.parameter.generic.cmake punctuation.section.braces.begin.cmake",
    "variable.parameter.generic.cmake punctuation.section.braces.end.cmake",
    "variable.parameter.generic.cmake variable.other.readwrite.cmake",
    "variable.parameter.generic.cmake punctuation.definition.variable.generator-expression.cmake",
    "variable.parameter.generic.cmake punctuation.section.block.begin.cmake",
    "variable.parameter.generic.cmake punctuation.section.block.end.cmake",
    "variable.parameter.generic.cmake punctuation.separator.generator-expression.cmake",
    // a `MATCHES` regex's tokens ([`Scan::regex`]), in bat's `source.regexp` scopes
    "string.regexp.cmake keyword.control.group.regexp",
    "string.regexp.cmake keyword.control.anchors.regexp",
    "string.regexp.cmake keyword.other.any.regexp",
    "string.regexp.cmake keyword.control.set.regexp",
    "string.regexp.cmake constant.character.escape.regexp",
    "string.regexp.cmake constant.other.range.regexp",
    "string.regexp.cmake keyword.operator.quantifier.regexp",
    "string.regexp.cmake keyword.operator.alternation.regexp",
    "string.regexp.cmake invalid.illegal.unexpected-quantifier.regexp",
    "string.regexp.cmake invalid.illegal.unmatched-brace.regexp",
    "string.regexp.cmake invalid.illegal.character.escape.regexp",
    // … in a quoted argument that is a regex (X-107), under the string's scope
    "string.quoted.double.cmake keyword.control.group.regexp",
    "string.quoted.double.cmake keyword.control.anchors.regexp",
    "string.quoted.double.cmake keyword.other.any.regexp",
    "string.quoted.double.cmake keyword.control.set.regexp",
    "string.quoted.double.cmake constant.character.escape.regexp",
    "string.quoted.double.cmake constant.other.range.regexp",
    "string.quoted.double.cmake keyword.operator.quantifier.regexp",
    "string.quoted.double.cmake keyword.operator.alternation.regexp",
    "string.quoted.double.cmake invalid.illegal.unexpected-quantifier.regexp",
    "string.quoted.double.cmake invalid.illegal.unmatched-brace.regexp",
    "string.quoted.double.cmake invalid.illegal.character.escape.regexp",
    // … in an unquoted one, bare
    "keyword.control.group.regexp",
    "keyword.control.anchors.regexp",
    "keyword.other.any.regexp",
    "keyword.control.set.regexp",
    "constant.character.escape.regexp",
    "constant.other.range.regexp",
    "keyword.operator.quantifier.regexp",
    "keyword.operator.alternation.regexp",
    "invalid.illegal.unexpected-quantifier.regexp",
    "invalid.illegal.unmatched-brace.regexp",
    "invalid.illegal.character.escape.regexp",
];

const COMMENT: u16 = 1;
const COMMENT_MARK: u16 = 2;
const BLOCK: u16 = 3;
const BLOCK_BEGIN: u16 = 4;
const BLOCK_END: u16 = 5;
/// A comment's kind in a capitalized argument: its bare kind's plus this.
const CAPS_COMMENT: u16 = 5;
const STRING: u16 = 11;
const STRING_BEGIN: u16 = 12;
const STRING_END: u16 = 13;
const S_ESCAPE: u16 = 14;
const S_BAD_ESCAPE: u16 = 15;
const S_SEPARATOR: u16 = 16;
const RAW: u16 = 17;
const RAW_BEGIN: u16 = 18;
const RAW_END: u16 = 19;
const REGEX: u16 = 20;
const REGEX_BEGIN: u16 = 21;
const REGEX_END: u16 = 22;
const ESCAPE: u16 = 23;
const BAD_ESCAPE: u16 = 24;
const SEPARATOR: u16 = 25;
const PAREN_BEGIN: u16 = 26;
const PAREN_END: u16 = 27;
const STRAY_PAREN: u16 = 28;
const EXPECTED_PAREN: u16 = 29;
const STRAY: u16 = 30;
const EXPECTED_TYPE: u16 = 31;
const FUNCTION: u16 = 32;
const CONTROL: u16 = 33;
const IMPORT: u16 = 34;
const GENERIC: u16 = 35;
const PARAM: u16 = 36;
const GENERIC_PARAM: u16 = 37;
const LOGICAL: u16 = 38;
const ASSIGNED: u16 = 39;
const FUNCTION_NAME: u16 = 40;
const STORAGE: u16 = 41;
const TWO_TERMINAL: u16 = 42;
const TWO_INCLUDES: u16 = 43;
const DOLLAR: u16 = 44;
const ENV: u16 = 45;
const BRACE_BEGIN: u16 = 46;
const BRACE_END: u16 = 47;
const VARIABLE: u16 = 48;
const GX_DOLLAR: u16 = 49;
const GX_BEGIN: u16 = 50;
const GX_END: u16 = 51;
const GX_SEPARATOR: u16 = 52;
const RX_GROUP: u16 = 71;
const RX_ANCHOR: u16 = 72;
const RX_ANY: u16 = 73;
const RX_SET: u16 = 74;
const RX_ESCAPE: u16 = 75;
const RX_RANGE: u16 = 76;
const RX_QUANTIFIER: u16 = 77;
const RX_OR: u16 = 78;
const RX_BAD_QUANTIFIER: u16 = 79;
const RX_UNMATCHED: u16 = 80;
const RX_BAD_ESCAPE: u16 = 81;
/// What a regex's kinds add in a quoted argument ([`H_QUOTED`]) …
const RX_IN_STRING: u16 = 11;
/// … and in an unquoted one ([`H_UNQUOTED`]).
const RX_BARE: u16 = 22;
/// What a `${…}`'s or `$<…>`'s kinds add in a quoted string ([`Scan::pre`]).
const PRE_STRING: u8 = 9;
/// … and in a capitalized argument.
const PRE_CAPS: u8 = 18;

/// The scopes under `source.cmake` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    LEAVES.get(usize::from(kind)).map_or_else(Vec::new, |l| {
        l.split(' ').filter(|s| !s.is_empty()).collect()
    })
}

/// The floor probe's build (X-104's `sol`, `ADA_PARSE_X104_SOL` set at build time): the loop, the
/// lookups and the frames without the corners (blocks, a first argument's name, `CACHE`'s type,
/// `RESULT_VARIABLE`, `MATCHES`, two-word keywords, capitalized arguments, the `(` awaited).
const SOL: bool = option_env!("ADA_PARSE_X104_SOL").is_some();

// A byte's class ([`CLASS`]).
const C_OTHER: u8 = 0;
/// ` ` and `\t`: what ends an unquoted argument.
const C_BLANK: u8 = 1;
/// `[A-Za-z_]`.
const C_NAME: u8 = 2;
const C_DIGIT: u8 = 3;
const C_QUOTE: u8 = 4;
const C_HASH: u8 = 5;
const C_DOLLAR: u8 = 6;
const C_LPAREN: u8 = 7;
const C_RPAREN: u8 = 8;
const C_BACKSLASH: u8 = 9;
const C_SEMICOLON: u8 = 10;
const C_BRACKET: u8 = 11;
const C_APOSTROPHE: u8 = 12;
/// A byte past ASCII.
const C_WIDE: u8 = 13;
/// `\r`, `\x0b`, `\x0c`: `\s` between a command's name and its `(`, but an unquoted argument's
/// text.
const C_SPACE: u8 = 14;

static CLASS: [u8; 256] = {
    let mut t = [C_WIDE; 256];
    let mut c = 0;
    while c < 128 {
        t[c] = match c as u8 {
            b' ' | b'\t' => C_BLANK,
            b'\r' | 0x0b | 0x0c => C_SPACE,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => C_NAME,
            b'0'..=b'9' => C_DIGIT,
            b'"' => C_QUOTE,
            b'#' => C_HASH,
            b'$' => C_DOLLAR,
            b'(' => C_LPAREN,
            b')' => C_RPAREN,
            b'\\' => C_BACKSLASH,
            b';' => C_SEMICOLON,
            b'[' => C_BRACKET,
            b'\'' => C_APOSTROPHE,
            _ => C_OTHER,
        };
        c += 1;
    }
    t
};

/// What ends an unquoted argument's plain run: its end (`[ \t()#"\\]`), and `$` and `;`.
static UNQUOTED_STOP: [bool; 256] = crate::prim::stops(b" \t()#\"\\$;");
/// What a quoted string's plain run stops at.
static STRING_STOP: [bool; 256] = crate::prim::stops(b"\"\\$;");
/// What ends a capitalized argument (`[\t ()#"\\]`), and `$`.
static CAPS_STOP: [bool; 256] = crate::prim::stops(b" \t()#\"\\$");
/// What a `\` escapes, in a quoted argument and an unquoted one (X-109, `cmake-language(7)`):
/// any character but a letter or digit (`escape_identity`, `\;` its `escape_semicolon`), and
/// `t`, `r`, `n` (`escape_encoded`); another letter or digit is an error to CMake.
static ESCAPES: [bool; 256] = {
    let mut t = [true; 256];
    let mut c = 0;
    while c < 256 {
        t[c] = !(c as u8).is_ascii_alphanumeric() || matches!(c as u8, b't' | b'r' | b'n');
        c += 1;
    }
    t
};

// The commands bat's grammar names apart from the builtins, their ids after the builtins'.
const C_IF: u8 = 104;
const C_ELSEIF: u8 = 105;
const C_ELSE: u8 = 106;
const C_ENDIF: u8 = 107;
const C_FOREACH: u8 = 108;
const C_ENDFOREACH: u8 = 109;
const C_WHILE: u8 = 110;
const C_ENDWHILE: u8 = 111;
const C_FUNCTION: u8 = 112;
const C_ENDFUNCTION: u8 = 113;
const C_MACRO: u8 = 114;
const C_ENDMACRO: u8 = 115;
const C_SET: u8 = 116;
const C_INCLUDE: u8 = 117;
const C_BREAK: u8 = 118;
const C_CONTINUE: u8 = 119;
/// Any other name: bat's generic command.
const C_GENERIC: u8 = 120;

const CONTROL_WORDS: [&[u8]; 16] = [
    b"if",
    b"elseif",
    b"else",
    b"endif",
    b"foreach",
    b"endforeach",
    b"while",
    b"endwhile",
    b"function",
    b"endfunction",
    b"macro",
    b"endmacro",
    b"set",
    b"include",
    b"break",
    b"continue",
];

const NC: usize = BUILTINS.len() + CONTROL_WORDS.len();
static COMMAND_TEXT: [&[u8]; NC] = {
    let mut t: [&[u8]; NC] = [b""; NC];
    let mut i = 0;
    while i < NC {
        t[i] = if i < BUILTINS.len() {
            BUILTINS[i]
        } else {
            CONTROL_WORDS[i - BUILTINS.len()]
        };
        i += 1;
    }
    t
};
/// The commands by name, ASCII case folded: a command's id is its word's id less one.
static COMMANDS: Words<256, NC, { text_len(&COMMAND_TEXT) }, true> = Words::new(&COMMAND_TEXT);

// The argument lists' keyword sets: the builtins' by id, then these.
/// `if`'s, `elseif`'s, `while`'s, a generic command's, a parenthesis's (bat's `if-args-inline`).
const S_IF: u32 = 104;
const S_FOREACH: u32 = 105;
/// `set`'s after its name.
const S_SET: u32 = 106;
const S_INCLUDE: u32 = 107;
/// A list without keywords: `function`'s parameters, `break`'s, `continue`'s.
const S_NONE: u32 = 108;
/// The builtins whose `REGEX` a regex follows (X-107): `file(STRINGS … REGEX`, `file(COPY …
/// REGEX`, `list(FILTER … REGEX`, `install(DIRECTORY … REGEX` (X-108); `string(REGEX
/// MATCH|MATCHALL|REPLACE`; and `source_group(… REGULAR_EXPRESSION` (X-108).
const S_FILE: u32 = 38;
const S_LIST: u32 = 61;
const S_STRING: u32 = 84;
const S_INSTALL: u32 = 55;
const S_SOURCE_GROUP: u32 = 83;
/// `add_custom_command`'s, whose `USES_TERMINAL COMMAND_EXPAND_LISTS` is one keyword.
const S_ADD_CUSTOM_COMMAND: u32 = 1;
/// `target_link_libraries`'s, which takes `debug`, `optimized` and `general` as
/// `link_libraries` does (X-119; bat's list lacks them).
const S_TARGET_LINK_LIBRARIES: u32 = 92;
const _: () = assert!(
    eq(
        BUILTINS[S_TARGET_LINK_LIBRARIES as usize],
        b"target_link_libraries"
    ) && eq(BUILTINS[S_FILE as usize], b"file")
        && eq(BUILTINS[S_LIST as usize], b"list")
        && eq(BUILTINS[S_STRING as usize], b"string")
        && eq(BUILTINS[S_INSTALL as usize], b"install")
        && eq(BUILTINS[S_SOURCE_GROUP as usize], b"source_group")
        && eq(
            BUILTINS[S_ADD_CUSTOM_COMMAND as usize],
            b"add_custom_command"
        )
);

// A keyword's flags, in its mask's top bits.
/// `NOT`, `AND`, `OR`: an operator's leaf.
const K_LOGICAL: u128 = 1 << 127;
/// `CACHE`, `RESULT_VARIABLE`, `MATCHES`, `REGEX`, `REGULAR_EXPRESSION`: what follows is read
/// apart ([`Scan::keyword`]).
const K_ACTION: u128 = 1 << 126;
/// `USES_TERMINAL`, `INCLUDES`: bat's two-word keywords' first.
const K_TWO: u128 = 1 << 125;

const NK: usize = KEYWORD_LIST.len() + 1;
static KEYWORD_TEXT: [&[u8]; NK] = {
    let mut t: [&[u8]; NK] = [b""; NK];
    let mut i = 0;
    while i < KEYWORD_LIST.len() {
        t[i] = KEYWORD_LIST[i].0;
        i += 1;
    }
    t[KEYWORD_LIST.len()] = b"INCLUDES";
    t
};
static KEYWORDS: Words<1024, NK, { text_len(&KEYWORD_TEXT) }> = Words::new(&KEYWORD_TEXT);
/// A keyword's sets and flags by its id; 0 for none.
static KEYWORD_INFO: [u128; NK + 1] = {
    let mut t = [0u128; NK + 1];
    let mut i = 0;
    while i < NK {
        let w = KEYWORD_TEXT[i];
        let mut m = if i < KEYWORD_LIST.len() {
            KEYWORD_LIST[i].1
        } else {
            0
        };
        if eq(w, b"NOT") || eq(w, b"AND") || eq(w, b"OR") {
            m |= K_LOGICAL;
        }
        if eq(w, b"CACHE")
            || eq(w, b"RESULT_VARIABLE")
            || eq(w, b"MATCHES")
            || eq(w, b"REGEX")
            || eq(w, b"REGULAR_EXPRESSION")
        {
            m |= K_ACTION;
        }
        // `install(DIRECTORY … REGEX "…")` (X-108): bat's `install` has `PATTERN`, not its twin
        if eq(w, b"REGEX") {
            m |= 1 << S_INSTALL;
        }
        if eq(w, b"debug") || eq(w, b"optimized") || eq(w, b"general") {
            m |= 1 << S_TARGET_LINK_LIBRARIES;
        }
        if eq(w, b"USES_TERMINAL") || eq(w, b"INCLUDES") {
            m |= K_TWO;
        }
        t[i + 1] = m;
        i += 1;
    }
    t
};

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

// The frames, four bits a level in [`Scan::frames`]; 0 is none (the file's top, or a level
// lost past 32 levels).
/// `if(…)` … `endif`, and `elseif(…)`'s.
const F_IF: u8 = 1;
/// `else(…)` … `endif`.
const F_ELSE: u8 = 2;
const F_FOREACH: u8 = 3;
const F_WHILE: u8 = 4;
const F_FUNCTION: u8 = 5;
const F_MACRO: u8 = 6;
/// A command's arguments, from its name ([`Scan::cmd`], [`Scan::sub`]).
const F_ARGS: u8 = 7;
/// A parenthesis in a generic command's, `if`'s or another parenthesis's arguments.
const F_PAREN: u8 = 8;
/// `${…}`, `$ENV{…}`.
const F_SUBST: u8 = 9;
/// `$<…`, before its `:`.
const F_GENEX: u8 = 10;
/// `$<…:…>`, after it.
const F_GENEX_ARGS: u8 = 11;
/// `"…"`.
const F_QUOTED: u8 = 12;
/// A generic command's capitalized argument, to a blank.
const F_CAPS: u8 = 13;
/// An unquoted argument under a `${…}` or `$<…>`: it ends with its line. A regex where
/// [`Scan::rx`] has [`RX_UNQUOTED`].
const F_UNQUOTED: u8 = 14;
/// A quoted argument that is a regex (X-107), as [`F_QUOTED`].
const F_QUOTED_RX: u8 = 15;

// Where a command's arguments stand ([`Scan::sub`]).
/// Before the `(`: blanks, then `(` on the name's line, or the line's rest an error (X-119).
const SUB_AWAIT: u8 = 0;
/// Before the `(`, in the floor probe ([`SOL`]): anything until it, across lines.
const SUB_LOOSE: u8 = 1;
/// In the arguments.
const SUB_OPEN: u8 = 2;
/// `set(`'s name next, on its line.
const SUB_SET: u8 = 3;
/// `function(`'s or `macro(`'s name next, on its line.
const SUB_NAME: u8 = 4;
/// `include(… RESULT_VARIABLE`'s variable next, on its line.
const SUB_RESULT: u8 = 5;

// What a line starts in ([`Scan::mode`]).
const M_CODE: u8 = 0;
/// A bracket comment, [`Scan::eqs`] `=`s.
const M_BLOCK: u8 = 1;
/// A bracket argument.
const M_RAW: u8 = 2;
/// A bracket argument after `MATCHES`.
const M_REGEX: u8 = 3;

// What a regex's quantifier may follow ([`Scan::regex`]).
/// Nothing: the regex's start, `(`, `|` (CMake's "?+* follows nothing").
const Q_NONE: u8 = 0;
/// A character, `.`, a class, a group: a quantifier's operand.
const Q_ATOM: u8 = 1;
/// `^` or `$`: `?` only (CMake's "*+ operand could be empty").
const Q_ANCHOR: u8 = 2;
/// A quantifier (CMake's "nested *?+").
const Q_QUANTIFIER: u8 = 3;
/// The groups open a regex keeps count of ([`Scan::rx`]'s low bits): past it a group isn't counted.
const RX_DEPTH: u8 = 63;
/// [`Scan::rx`]'s bit: the unquoted argument under a `${…}` ([`F_UNQUOTED`]) is a regex.
const RX_UNQUOTED: u8 = 64;
/// [`Scan::rx`]'s bit: a class is open.
const RX_IN_SET: u8 = 128;

// What a regex is spelled in ([`Scan::regex`]).
/// A bracket argument (`MATCHES [[…]]`, X-106): its text is the regex's.
const H_BRACKET: u8 = 0;
/// A quoted argument (X-107): CMake's escapes come first, `${…}` and `$<…>` interpolate.
const H_QUOTED: u8 = 1;
/// An unquoted argument (X-107): as a quoted one, to a blank, a parenthesis, `#` or `"`.
const H_UNQUOTED: u8 = 2;
/// What ends an unquoted argument that is a regex.
static UNQUOTED_END: [bool; 256] = crate::prim::stops(b" \t()#\"");

/// Lex `b`, a line of CMake without its `\n`, from `state`.
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

/// A block of CMake on one scan opened from `state` ([`crate::Carry`]).
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
    /// What each open frame is ([`F_IF`]…), four bits a level, the innermost lowest; past
    /// 32 levels the outer ones are lost.
    frames: u128,
    /// How many frames are open, up to 255.
    depth: u8,
    /// The command whose arguments are open ([`F_ARGS`]): a builtin's id, or [`C_IF`]….
    cmd: u8,
    /// Where its arguments stand ([`SUB_AWAIT`]…).
    sub: u8,
    /// What the open `${…}`s' and `$<…>`s' kinds add: 0, [`PRE_STRING`], [`PRE_CAPS`].
    pre: u8,
    /// `MATCHES`, a `REGEX` or `REGULAR_EXPRESSION` was the last argument: the next one is a
    /// regex, on this line or a later one, past comments (X-108).
    matches: bool,
    /// What the line starts in ([`M_CODE`]…).
    mode: u8,
    /// An open bracket's `=`s, up to 255 (where 255 is any more).
    eqs: u8,
    /// In a regex ([`M_REGEX`], [`F_QUOTED_RX`], an unquoted one): the groups open, up to
    /// [`RX_DEPTH`], [`RX_IN_SET`] where a class is, [`RX_UNQUOTED`] where it is an unquoted
    /// argument's; a byte as the state keeps it, so a line that isn't in one only copies it.
    rx: u8,
    /// A command's `)` or a bracket comment ended on this line: only blanks and comments may
    /// follow it ([`Scan::commands`], CMake's "Expected a newline"). Not kept between lines.
    done: bool,
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
            cmd: 0,
            sub: SUB_AWAIT,
            pre: 0,
            matches: false,
            mode: M_CODE,
            eqs: 0,
            rx: 0,
            done: false,
            k,
            out,
        }
    }

    /// The scan's fields from `state`: the frames in its levels and the first two withs, the
    /// command, the `=`s and a regex's groups and class in the next three, the mode, the
    /// arguments' place and the prefix in [`State::arg`].
    #[inline(always)]
    fn load(&mut self, state: &State) {
        let mut w = [0u8; 16];
        w[..14].copy_from_slice(&state.levels);
        w[14] = state.withs[0];
        w[15] = state.withs[1];
        self.frames = u128::from_le_bytes(w);
        self.depth = state.depth;
        self.cmd = state.withs[2];
        self.eqs = state.withs[3];
        self.rx = state.withs[4];
        let a = state.arg;
        self.mode = (a & 3) as u8;
        self.sub = (a >> 2 & 7) as u8;
        self.pre = (a >> 5 & 31) as u8;
        self.matches = a >> 10 & 1 != 0;
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
        s.withs[2] = self.cmd;
        s.withs[3] = self.eqs;
        s.withs[4] = self.rx;
        s.depth = self.depth;
        s.arg = u16::from(self.mode)
            | u16::from(self.sub) << 2
            | u16::from(self.pre) << 5
            | u16::from(self.matches) << 10;
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

    /// A write: one call on the run sink ([`Sink::merge_call`]).
    #[inline(always)]
    fn put(&mut self, s: usize, e: usize, kind: u16) {
        self.k.merge_call(&mut self.out, s, e, kind);
    }

    /// A write inlined, for the common tokens: a command's name, a keyword, a parenthesis, a
    /// quoted string's text and quotes, a `${…}`'s.
    #[inline(always)]
    fn put_hot(&mut self, s: usize, e: usize, kind: u16) {
        self.k.merge(&mut self.out, s, e, kind);
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

    /// The top frame replaced by `f`.
    #[inline(always)]
    fn set_top(&mut self, f: u8) {
        self.frames = self.frames & !15 | u128::from(f);
    }

    /// The line, from what the line before left: a mode's rest first; the loop out of line
    /// where `SHARED` ([`Scan::code_shared`]).
    #[inline(always)]
    fn lex_line<const SHARED: bool>(&mut self) {
        // a bracket comment that closes on this line is one a command can't follow
        self.done = self.mode == M_BLOCK;
        let p = match self.mode {
            M_CODE => 0,
            _ => self.bracket(0),
        };
        if SHARED {
            self.code_shared(p);
        } else {
            self.code(p);
        }
        self.line_end();
    }

    /// [`Scan::code`] out of line: on the run sink, one copy that the line API and a block's
    /// lines share; on the span sink, which ada doesn't hold, inlined.
    #[inline(never)]
    fn code_shared(&mut self, p: usize) {
        self.code(p);
    }

    /// What a line's end ends: an unquoted argument, a command still before its `(` (CMake's
    /// "Expected "(", got newline", X-119), and what bat reads only on its line (the first
    /// argument's name, `RESULT_VARIABLE`'s variable). A regex awaited ([`Scan::matches`])
    /// stays: CMake reads the next argument wherever it stands (X-108).
    #[inline(always)]
    fn line_end(&mut self) {
        match self.top() {
            F_UNQUOTED => self.pop(),
            F_ARGS if self.sub == SUB_AWAIT => self.pop(),
            F_ARGS if self.sub > SUB_OPEN => self.sub = SUB_OPEN,
            _ => {}
        }
    }

    /// The line from `p`, each frame's loop in turn.
    #[inline(always)]
    fn code(&mut self, mut p: usize) {
        let n = self.b.len();
        while p < n {
            p = match self.top() {
                F_ARGS => match self.sub {
                    SUB_AWAIT => self.await_paren(p),
                    SUB_LOOSE => self.await_loose(p),
                    _ => self.args(p),
                },
                F_PAREN => self.args(p),
                F_QUOTED => self.quoted(p),
                F_QUOTED_RX => self.regex::<H_QUOTED>(p, Q_ATOM),
                F_SUBST => self.subst(p),
                F_GENEX | F_GENEX_ARGS => self.genex(p),
                F_CAPS => self.caps(p),
                F_UNQUOTED => self.unquoted(p),
                _ => self.commands(p),
            };
        }
    }

    /// Commands, from `p` in a block or at the top: until one's name. A line holds one command,
    /// after blanks and before blanks and comments (X-119): a name after a command's `)` or a
    /// bracket comment, or anything else but blanks and comments, is [`Scan::stray`]; a
    /// template's placeholder is read wherever a command may follow ([`Scan::templated`]).
    #[inline(always)]
    fn commands(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        while p < n {
            match CLASS[usize::from(b[p])] {
                C_BLANK | C_SPACE => p += 1,
                C_NAME if !self.done => return self.command(p, word_end(b, p)),
                C_HASH => {
                    p = self.comment(p);
                    self.done = true;
                }
                // a template's placeholder (`@PACKAGE_INIT@`), or a name that starts with one;
                // after a command's `)` too, where configuring puts more commands
                // (`set(@var@_KEYS "@keys@")@code@`, X-145)
                C_OTHER if b[p] == b'@' => match self.templated(p) {
                    Some(e) => return e,
                    None => return self.stray(p),
                },
                // a byte order mark: CMake skips one
                C_WIDE if b[p..].starts_with("\u{feff}".as_bytes()) => p += 3,
                _ => return self.stray(p),
            }
        }
        p
    }

    /// A command's name with a `configure_file` template's placeholders in it, at `p`: a run of
    /// name characters and `@VAR@`s (CMake's `@[A-Za-z0-9_/.+-]+@`), at least one placeholder
    /// (`@PACKAGE_INIT@`, `z_vcpkg_@PORT@_get_link_libraries(`). CMake never reads one: the
    /// template's configuring replaces it first, so it is no error (X-119). Its `@`s as a `${…}`'s
    /// `$`, its name as the variable's, the name's parts a generic command's; its end, the
    /// command's `(` awaited. `None` where `p` starts no such name.
    #[cold]
    #[inline(never)]
    fn templated(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        // the placeholder at `q`, an `@`: its closing `@`
        let placeholder = |q: usize| {
            let mut e = q + 1;
            while e < n && (b[e].is_ascii_alphanumeric() || b"_/.+-".contains(&b[e])) {
                e += 1;
            }
            (e > q + 1 && b.get(e) == Some(&b'@')).then_some(e)
        };
        // the run's pieces: name parts and placeholders
        let mut q = p;
        let mut any = false;
        while q < n {
            if b[q] == b'@'
                && let Some(c) = placeholder(q)
            {
                any = true;
                q = c + 1;
            } else if CLASS[usize::from(b[q])] == C_NAME
                || q > p && CLASS[usize::from(b[q])] == C_DIGIT
            {
                q += 1;
            } else {
                break;
            }
        }
        if !any {
            return None;
        }
        let e = q;
        let mut q = p;
        while q < e {
            if b[q] == b'@' {
                let c = placeholder(q).unwrap_or(q);
                self.put(q, q + 1, DOLLAR);
                self.put(q + 1, c, VARIABLE);
                self.put(c, c + 1, DOLLAR);
                q = c + 1;
            } else {
                let s = q;
                while q < e && b[q] != b'@' {
                    q += 1;
                }
                self.put(s, q, GENERIC);
            }
        }
        self.open_args(C_GENERIC, SUB_AWAIT);
        Some(e)
    }

    /// What CMake's parser stops at, at a command's place (X-119): a token that isn't a name
    /// ("Expected a command name"), or one after a command or a bracket comment ("Expected a
    /// newline"); the line's rest an error, as bat's after a name without its `(`.
    #[cold]
    #[inline(never)]
    fn stray(&mut self, p: usize) -> usize {
        let n = self.b.len();
        self.put(p, n, STRAY);
        n
    }

    /// A command's name, `b[p..e]`: its leaf, and its arguments' frame (bat's pattern order: the
    /// block's own words, the builtins, the control words, the stray ones, a generic command).
    #[inline(always)]
    fn command(&mut self, p: usize, e: usize) -> usize {
        if self.b.get(e) == Some(&b'@')
            && let Some(e) = self.templated(p)
        {
            return e;
        }
        let id = COMMANDS.get(&self.b[p..e]);
        let cmd = if id == 0 { C_GENERIC } else { (id - 1) as u8 };
        if cmd < C_IF {
            self.put_hot(p, e, FUNCTION);
            self.open_args(cmd, if SOL { SUB_LOOSE } else { SUB_AWAIT });
            return e;
        }
        self.control(p, e, cmd)
    }

    /// A command's name that isn't a builtin's.
    #[inline(never)]
    fn control(&mut self, p: usize, e: usize, cmd: u8) -> usize {
        if SOL {
            let kind = if cmd == C_GENERIC { GENERIC } else { leaf(cmd) };
            self.put(p, e, kind);
            self.open_args(cmd, SUB_LOOSE);
            return e;
        }
        let block = self.top();
        // a block's own words: those that end it replace its frame
        let closes = matches!(
            (block, cmd),
            (F_IF, C_ELSEIF | C_ELSE | C_ENDIF)
                | (F_ELSE, C_ENDIF)
                | (F_FOREACH, C_ENDFOREACH)
                | (F_WHILE, C_ENDWHILE)
                | (F_FUNCTION, C_ENDFUNCTION)
                | (F_MACRO, C_ENDMACRO)
        );
        let (kind, sub) = match cmd {
            _ if closes => {
                self.pop();
                (leaf(cmd), SUB_AWAIT)
            }
            C_BREAK if matches!(block, F_IF | F_ELSE | F_FOREACH | F_WHILE) => (CONTROL, SUB_AWAIT),
            C_CONTINUE if matches!(block, F_FOREACH | F_WHILE) => (CONTROL, SUB_AWAIT),
            C_IF | C_FOREACH | C_WHILE | C_FUNCTION | C_MACRO | C_INCLUDE => (leaf(cmd), SUB_AWAIT),
            C_SET => (FUNCTION, SUB_AWAIT),
            C_CONTINUE | C_GENERIC => {
                self.put_hot(p, e, GENERIC);
                self.open_args(C_GENERIC, SUB_AWAIT);
                return e;
            }
            // a block's word outside it: its arguments as a closing word's (no block opened)
            _ => {
                self.put(p, e, STRAY);
                let as_closing = if matches!(cmd, C_ELSEIF | C_ELSE) {
                    C_ENDIF
                } else {
                    cmd
                };
                self.open_args(as_closing, SUB_AWAIT);
                return e;
            }
        };
        self.put(p, e, kind);
        self.open_args(cmd, sub);
        e
    }

    #[inline(always)]
    fn open_args(&mut self, cmd: u8, sub: u8) {
        self.cmd = cmd;
        self.sub = sub;
        self.push(F_ARGS);
    }

    /// Between a command's name and its `(`: blanks on the name's line ([`Scan::line_end`] ends
    /// the command at the line's end), then its `(`; a comment is read and ends it, anything else
    /// is the line's rest an error and the command over (X-119: CMake's "Expected "("").
    #[inline(always)]
    fn await_paren(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        while p < n && matches!(CLASS[usize::from(b[p])], C_BLANK | C_SPACE) {
            p += 1;
        }
        if p == n {
            return n;
        }
        match b[p] {
            b'(' => self.open_paren(p),
            b'#' => {
                self.pop();
                let e = self.comment(p);
                self.done = true;
                e
            }
            _ => {
                self.put(p, n, EXPECTED_PAREN);
                self.pop();
                n
            }
        }
    }

    /// Between a command's name and its `(` in the floor probe ([`SOL`]): anything until it is
    /// plain.
    #[inline(always)]
    fn await_loose(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        while p < n {
            match b[p] {
                b'(' => return self.open_paren(p),
                b'#' => return self.comment(p),
                b'$' => {
                    if let Some(e) = self.dollar(p) {
                        return e;
                    }
                    p += 1;
                }
                _ => p += 1,
            }
        }
        n
    }

    /// A command's `(` at `p`: its block opened under its arguments.
    #[inline(always)]
    fn open_paren(&mut self, p: usize) -> usize {
        self.put_hot(p, p + 1, PAREN_BEGIN);
        let (block, sub) = match self.cmd {
            C_IF | C_ELSEIF => (F_IF, SUB_OPEN),
            C_ELSE => (F_ELSE, SUB_OPEN),
            C_FOREACH => (F_FOREACH, SUB_OPEN),
            C_WHILE => (F_WHILE, SUB_OPEN),
            C_FUNCTION => (F_FUNCTION, SUB_NAME),
            C_MACRO => (F_MACRO, SUB_NAME),
            C_SET => (0, SUB_SET),
            _ => (0, SUB_OPEN),
        };
        if block != 0 && !SOL {
            self.set_top(block);
            self.push(F_ARGS);
        }
        self.sub = if SOL { SUB_OPEN } else { sub };
        p + 1
    }

    /// A command's arguments ([`F_ARGS`]) or a parenthesis's in them ([`F_PAREN`]), from `p`:
    /// until a frame opens or closes.
    #[inline(always)]
    fn args(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let frame = self.top();
        // the list's keyword set (a parenthesis's its command's, X-119), and whether a
        // capitalized word runs (bat's generic arguments); a `(` nests in every command's (bat's
        // ends a builtin's at the first `)`), and a bracket argument is one in every command's
        // (X-110: bat's generic command takes none, so `check_c_source_compiles([[#include …]])`
        // lexed its text as CMake)
        let (set, caps) = match self.cmd {
            c if c < C_IF => (u32::from(c), false),
            C_IF | C_ELSEIF | C_ELSE | C_ENDIF | C_WHILE | C_ENDWHILE => (S_IF, false),
            C_FOREACH | C_ENDFOREACH => (S_FOREACH, false),
            C_SET => (S_SET, false),
            C_INCLUDE => (S_INCLUDE, false),
            C_FUNCTION | C_MACRO | C_BREAK | C_CONTINUE => (S_NONE, false),
            _ if frame == F_PAREN => (S_IF, false),
            _ => (S_IF, true),
        };
        // the floor probe's ([`SOL`]) as bat's
        let nests = !SOL || frame == F_PAREN || set == S_IF;
        let brackets = !SOL || !caps;
        while p < n {
            let c = b[p];
            let class = CLASS[usize::from(c)];
            if class == C_BLANK {
                p += 1;
                while p < n && CLASS[usize::from(b[p])] == C_BLANK {
                    p += 1;
                }
                continue;
            }
            if (self.sub > SUB_OPEN || self.matches)
                && let Some(e) = self.first(p)
            {
                p = e;
                if self.top() != frame {
                    return p;
                }
                continue;
            }
            match class {
                C_NAME | C_DIGIT => {
                    let e = word_end(b, p);
                    let m = KEYWORDS.value(&b[p..e], &KEYWORD_INFO);
                    if m >> set & 1 != 0 {
                        p = if SOL || m & (K_LOGICAL | K_ACTION) == 0 {
                            self.put_hot(p, e, PARAM);
                            e
                        } else {
                            self.keyword(p, e, m, set)
                        };
                        continue;
                    }
                    if !SOL
                        && m & K_TWO != 0
                        && let Some(e2) = self.two_words(p, e, set)
                    {
                        p = e2;
                        continue;
                    }
                    if !SOL && caps && is_caps(b, p, e) {
                        self.push(F_CAPS);
                        return self.caps(p);
                    }
                    p = self.unquoted_run(e);
                }
                C_RPAREN => {
                    self.put_hot(p, p + 1, PAREN_END);
                    self.pop();
                    // the command's: only blanks and comments may follow on its line
                    self.done = frame == F_ARGS;
                    return p + 1;
                }
                C_LPAREN => {
                    if nests {
                        self.put(p, p + 1, PAREN_BEGIN);
                        self.push(F_PAREN);
                        return p + 1;
                    }
                    self.put(p, p + 1, STRAY_PAREN);
                    p += 1;
                }
                C_QUOTE => {
                    self.put_hot(p, p + 1, STRING_BEGIN);
                    self.push(F_QUOTED);
                    return p + 1;
                }
                C_HASH => p = self.comment(p),
                C_DOLLAR => {
                    if let Some(e) = self.dollar(p) {
                        return e;
                    }
                    p = self.unquoted_run(p + 1);
                }
                C_BRACKET if brackets => {
                    p = match self.bracket_open(p, M_RAW) {
                        Some(e) => e,
                        None => self.unquoted_run(p + 1),
                    };
                }
                C_APOSTROPHE => p += 1,
                C_OTHER if c == b'@' => p = self.at_word(p, set),
                C_WIDE if is_word_starting(b, p) => {
                    let e = word_end(b, p);
                    p = self.unquoted_run(e);
                }
                _ => p = self.unquoted_run(p),
            }
            if self.top() != frame {
                return p;
            }
        }
        p
    }

    /// A token at `p` where the arguments' first name, `RESULT_VARIABLE`'s variable or a regex
    /// (after `MATCHES`, a `REGEX` or `REGULAR_EXPRESSION`) may stand: where it does, its end;
    /// else the reading over and `None`. A comment before a regex is read, the regex still
    /// awaited (X-108).
    #[inline(never)]
    fn first(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if self.matches {
            if b[p] == b'#' {
                return Some(self.comment(p));
            }
            self.matches = false;
            self.rx = 0;
            return match b[p] {
                b'[' => self
                    .bracket_open(p, M_REGEX)
                    .or_else(|| Some(self.regex::<H_UNQUOTED>(p, Q_NONE))),
                b'"' => {
                    self.put_hot(p, p + 1, STRING_BEGIN);
                    self.push(F_QUOTED_RX);
                    Some(self.regex::<H_QUOTED>(p + 1, Q_NONE))
                }
                b')' | b'(' => None,
                _ => Some(self.regex::<H_UNQUOTED>(p, Q_NONE)),
            };
        }
        if is_name_start(b, p) {
            let e = word_end(b, p);
            let kind = match self.sub {
                SUB_SET => ASSIGNED,
                SUB_NAME => FUNCTION_NAME,
                _ => VARIABLE,
            };
            self.put(p, e, kind);
            self.sub = SUB_OPEN;
            return Some(e);
        }
        match b[p] {
            // bat's prototype: the name may follow
            b'#' if self.sub != SUB_RESULT => Some(self.comment(p)),
            b'$' if self.sub != SUB_RESULT && self.opens(p) => self.dollar(p),
            _ => {
                self.sub = SUB_OPEN;
                None
            }
        }
    }

    /// A keyword `b[p..e]` whose leaf or what follows is its own (`m`'s flags), by the word and
    /// the list it is in (`RESULT_VARIABLE` is `file`'s and `include`'s, `REGEX` `string`'s).
    #[inline(never)]
    fn keyword(&mut self, p: usize, e: usize, m: u128, set: u32) -> usize {
        if m & K_LOGICAL != 0 {
            self.put(p, e, LOGICAL);
            return e;
        }
        self.put(p, e, PARAM);
        match (set, &self.b[p..e]) {
            // `CACHE`'s type, on its line: a known one's spelling a prefix of the word
            (S_SET, b"CACHE") => self.cache_type(e),
            (S_INCLUDE, b"RESULT_VARIABLE") => {
                self.sub = SUB_RESULT;
                e
            }
            (S_IF, b"MATCHES")
            | (S_FILE | S_LIST | S_INSTALL, b"REGEX")
            | (S_SOURCE_GROUP, b"REGULAR_EXPRESSION") => {
                self.matches = true;
                e
            }
            (S_STRING, b"REGEX") => self.regex_mode(e),
            _ => e,
        }
    }

    /// `string(REGEX`'s mode from `p`, on its line: blanks, then `MATCH`, `MATCHALL` or
    /// `REPLACE`, a regex after it; its end where it is one.
    #[cold]
    fn regex_mode(&mut self, p: usize) -> usize {
        let b = self.b;
        let mut q = p;
        while q < b.len() && CLASS[usize::from(b[q])] == C_BLANK {
            q += 1;
        }
        if q == p || q == b.len() || CLASS[usize::from(b[q])] != C_NAME {
            return p;
        }
        let e = word_end(b, q);
        if !matches!(&b[q..e], b"MATCH" | b"MATCHALL" | b"REPLACE") {
            return p;
        }
        self.put(q, e, PARAM);
        self.matches = true;
        e
    }

    /// `CACHE`'s type from `p`: blanks, then `FILEPATH`, `PATH`, `STRING`, `BOOL`, `INTERNAL`
    /// (the word's start), or the run to a blank an error. A `#` right after `CACHE` is a
    /// comment: bat's prototype ties with the type's `\s*` there, and comes first.
    #[cold]
    fn cache_type(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        if b.get(p) == Some(&b'#') {
            return self.comment(p);
        }
        while p < n && matches!(CLASS[usize::from(b[p])], C_BLANK | C_SPACE) {
            p += 1;
        }
        for t in [&b"FILEPATH"[..], b"PATH", b"STRING", b"BOOL", b"INTERNAL"] {
            if b[p..].starts_with(t) {
                self.put(p, p + t.len(), STORAGE);
                return p + t.len();
            }
        }
        let mut e = p;
        while e < n && !matches!(CLASS[usize::from(b[e])], C_BLANK | C_SPACE) {
            e += 1;
        }
        self.put(p, e, EXPECTED_TYPE);
        e
    }

    /// An argument at `p`, an `@`: `@ONLY` where the list has it (`configure_file`'s, `file`'s;
    /// X-119: bat's `\b@ONLY\b` needs a word before the `@`), else an unquoted argument.
    #[cold]
    fn at_word(&mut self, p: usize, set: u32) -> usize {
        let b = self.b;
        if b.get(p + 1)
            .is_some_and(|&c| CLASS[usize::from(c)] == C_NAME)
        {
            let e = word_end(b, p + 1);
            let m = KEYWORDS.value(&b[p..e], &KEYWORD_INFO);
            // the whole argument (`@ONLY@` is a word of a template's)
            if m >> set & 1 != 0 && b.get(e).is_none_or(|&c| UNQUOTED_END[usize::from(c)]) {
                self.put(p, e, PARAM);
                return e;
            }
        }
        self.unquoted_run(p)
    }

    /// bat's two-word keywords, `USES_TERMINAL COMMAND_EXPAND_LISTS` (`add_custom_command`'s)
    /// and `INCLUDES DESTINATION` (`install`'s), from `b[p..e]`: their end where they are.
    #[cold]
    fn two_words(&mut self, p: usize, e: usize, set: u32) -> Option<usize> {
        let b = self.b;
        let (second, kind): (&[u8], u16) = match (set, b[p]) {
            (S_ADD_CUSTOM_COMMAND, b'U') => (b" COMMAND_EXPAND_LISTS", TWO_TERMINAL),
            (S_INSTALL, b'I') => (b" DESTINATION", TWO_INCLUDES),
            _ => return None,
        };
        let e2 = e + second.len();
        if !b[e..].starts_with(second) || is_word_starting(b, e2) {
            return None;
        }
        self.put(p, e2, kind);
        Some(e2)
    }

    /// An unquoted argument from `p`, its start's word read: to its end (`[ \t()#"\\]`), its
    /// escapes and `;`s written, or to a `${…}`'s or `$<…>`'s start, opened on its frame.
    #[inline(always)]
    fn unquoted_run(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        loop {
            while p < n && !UNQUOTED_STOP[usize::from(b[p])] {
                p += 1;
            }
            if p == n {
                return n;
            }
            match b[p] {
                b'\\' => p = self.escape(p, false),
                b';' => {
                    self.put(p, p + 1, SEPARATOR);
                    p += 1;
                }
                b'$' => {
                    if self.opens(p) {
                        self.push(F_UNQUOTED);
                        self.rx = 0;
                        return self.dollar(p).unwrap_or(p + 1);
                    }
                    p += 1;
                }
                _ => return p,
            }
        }
    }

    /// An unquoted argument's rest after a `${…}` or `$<…>` in it closed: a regex's where it is
    /// one ([`RX_UNQUOTED`]).
    #[inline(never)]
    fn unquoted(&mut self, p: usize) -> usize {
        let e = if self.rx & RX_UNQUOTED != 0 {
            self.regex::<H_UNQUOTED>(p, Q_ATOM)
        } else {
            self.unquoted_run(p)
        };
        if self.top() == F_UNQUOTED {
            self.pop();
        }
        e
    }

    /// A `\` at `p`, in a quoted string or not: CMake's escape ([`ESCAPES`]), before a letter or
    /// digit but `t`, `r`, `n` an error (X-109). At the line's end, text: a quoted string's line
    /// continuation; in an unquoted argument CMake's parse error, but the wide list's two are in a
    /// generic command's bracket argument, which bat's grammar doesn't read (`write_pkgconfig([[`).
    #[inline(always)]
    fn escape(&mut self, p: usize, quoted: bool) -> usize {
        let b = self.b;
        let Some(&c) = b.get(p + 1) else {
            if quoted {
                self.put(p, p + 1, STRING);
            }
            return p + 1;
        };
        let e = (p + 1 + char_len(c)).min(b.len());
        let kind = match (quoted, ESCAPES[usize::from(c)]) {
            (true, true) => S_ESCAPE,
            (true, false) => S_BAD_ESCAPE,
            (false, true) => ESCAPE,
            (false, false) => BAD_ESCAPE,
        };
        self.put(p, e, kind);
        e
    }

    /// A quoted string's text from `p`: to its `"`, or a `${…}`'s or `$<…>`'s start.
    #[inline(always)]
    fn quoted(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        loop {
            let s = p;
            while p < n && !STRING_STOP[usize::from(b[p])] {
                p += 1;
            }
            self.put_hot(s, p, STRING);
            if p == n {
                return n;
            }
            match b[p] {
                b'"' => {
                    self.put_hot(p, p + 1, STRING_END);
                    self.pop();
                    return p + 1;
                }
                b'\\' => p = self.escape(p, true),
                b';' => {
                    self.put(p, p + 1, S_SEPARATOR);
                    p += 1;
                }
                _ => {
                    if let Some(e) = self.dollar(p) {
                        return e;
                    }
                    self.put(p, p + 1, STRING);
                    p += 1;
                }
            }
        }
    }

    /// Whether a `${`, `$ENV{` or `$<` starts at `p`.
    #[inline(always)]
    fn opens(&self, p: usize) -> bool {
        match self.b.get(p + 1) {
            Some(b'{' | b'<') => true,
            Some(b'E') => self.b[p + 1..].starts_with(b"ENV{"),
            _ => false,
        }
    }

    /// A `${`, `$ENV{` or `$<` at `p`, opened on its frame: its end; `None` where `$` is text.
    /// Its kinds by what it's in ([`Scan::pre`]). Out of line: inlined in each of the nine loops
    /// that meet a `$`, its writes held most of the run loop's size.
    #[inline(never)]
    fn dollar(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let (e, gx) = match b.get(p + 1) {
            Some(b'{') => (p + 2, false),
            Some(b'<') => (p + 2, true),
            Some(b'E') if b[p + 1..].starts_with(b"ENV{") => (p + 5, false),
            _ => return None,
        };
        match self.top() {
            F_SUBST | F_GENEX | F_GENEX_ARGS => {}
            F_QUOTED | F_QUOTED_RX => self.pre = PRE_STRING,
            F_CAPS => self.pre = PRE_CAPS,
            _ => self.pre = 0,
        }
        let pre = u16::from(self.pre);
        if gx {
            self.put(p, p + 1, GX_DOLLAR + pre);
            self.put(p + 1, e, GX_BEGIN + pre);
            self.push(F_GENEX);
        } else {
            self.put_hot(p, p + 1, DOLLAR + pre);
            if e == p + 5 {
                self.put(p + 1, p + 4, ENV + pre);
            }
            self.put_hot(e - 1, e, BRACE_BEGIN + pre);
            self.push(F_SUBST);
        }
        Some(e)
    }

    /// A `${…}`'s inside from `p`: names, nested ones, comments, to its `}`.
    #[inline(always)]
    fn subst(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let pre = u16::from(self.pre);
        while p < n {
            let c = b[p];
            match CLASS[usize::from(c)] {
                C_NAME => {
                    let e = word_end(b, p);
                    self.put_hot(p, e, VARIABLE + pre);
                    p = e;
                }
                C_HASH => p = self.comment(p),
                C_DOLLAR => {
                    if let Some(e) = self.dollar(p) {
                        return e;
                    }
                    p = self.text(p, p + 1);
                }
                C_WIDE => p = self.wide_name(p, pre),
                C_DIGIT => p = self.text(p, word_end(b, p)),
                _ if c == b'}' => {
                    self.put_hot(p, p + 1, BRACE_END + pre);
                    self.pop();
                    return p + 1;
                }
                _ => p = self.text(p, p + 1),
            }
        }
        n
    }

    /// `b[p..e]`, text in a `${…}` or `$<…>`: the string's or capitalized argument's it is in.
    #[inline(always)]
    fn text(&mut self, p: usize, e: usize) -> usize {
        match self.pre {
            0 => {}
            PRE_STRING => self.put(p, e, STRING),
            _ => self.put(p, e, GENERIC_PARAM),
        }
        e
    }

    /// A `$<…>`'s inside from `p`: names, its first `:`, nested ones, to its `>`; comments before
    /// the `:` (bat's prototype, which the part after it doesn't take).
    #[inline(always)]
    fn genex(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let pre = u16::from(self.pre);
        while p < n {
            let c = b[p];
            match CLASS[usize::from(c)] {
                C_NAME => {
                    let e = word_end(b, p);
                    self.put(p, e, VARIABLE + pre);
                    p = e;
                }
                C_HASH if self.top() == F_GENEX => p = self.comment(p),
                C_DOLLAR => {
                    if let Some(e) = self.dollar(p) {
                        return e;
                    }
                    p = self.text(p, p + 1);
                }
                C_DIGIT => p = self.text(p, word_end(b, p)),
                C_WIDE => p = self.wide_name(p, pre),
                _ if c == b'>' => {
                    self.put(p, p + 1, GX_END + pre);
                    self.pop();
                    return p + 1;
                }
                _ if c == b':' && self.top() == F_GENEX => {
                    self.put(p, p + 1, GX_SEPARATOR + pre);
                    self.set_top(F_GENEX_ARGS);
                    p += 1;
                }
                _ => p = self.text(p, p + 1),
            }
        }
        n
    }

    /// A character past ASCII at `p` in a `${…}` or `$<…>`: a name where it starts one.
    #[cold]
    fn wide_name(&mut self, p: usize, pre: u16) -> usize {
        let b = self.b;
        if is_name_start(b, p) {
            let e = word_end(b, p);
            self.put(p, e, VARIABLE + pre);
            e
        } else if is_word_starting(b, p) {
            self.text(p, word_end(b, p))
        } else {
            self.text(p, (p + char_len(b[p])).min(b.len()))
        }
    }

    /// A capitalized argument from `p` (on [`F_CAPS`]): to a blank, a parenthesis, `"` or `\`,
    /// across lines.
    #[inline(always)]
    fn caps(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        loop {
            let s = p;
            while p < n && !CAPS_STOP[usize::from(b[p])] {
                p += 1;
            }
            self.put(s, p, GENERIC_PARAM);
            if p == n {
                return n;
            }
            match b[p] {
                b'#' => return self.comment(p),
                b'$' => {
                    if let Some(e) = self.dollar(p) {
                        return e;
                    }
                    self.put(p, p + 1, GENERIC_PARAM);
                    p += 1;
                }
                _ => {
                    self.pop();
                    return p;
                }
            }
        }
    }

    /// A comment at `p`: a bracket one (`#[==[`), or to the line's end. Out of line, as
    /// [`Scan::dollar`].
    #[inline(never)]
    fn comment(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let pre = if self.top() == F_CAPS {
            CAPS_COMMENT
        } else {
            0
        };
        if b.get(p + 1) == Some(&b'[')
            && let Some(e) = self.bracket_open(p + 1, M_BLOCK)
        {
            return e;
        }
        self.put_hot(p, p + 1, COMMENT_MARK + pre);
        self.put_hot(p + 1, n, COMMENT + pre);
        n
    }

    /// A bracket at `p` (`[`, `=`s, `[`; after `#` a comment's) opened as `mode`, and lexed to
    /// its close or the line's end: its end; `None` where `p` opens none.
    #[inline(always)]
    fn bracket_open(&mut self, p: usize, mode: u8) -> Option<usize> {
        let b = self.b;
        let mut e = p + 1;
        while e < b.len() && b[e] == b'=' {
            e += 1;
        }
        if b.get(e) != Some(&b'[') {
            return None;
        }
        let eqs = e - p - 1;
        e += 1;
        let pre = if self.top() == F_CAPS {
            CAPS_COMMENT
        } else {
            0
        };
        match mode {
            M_BLOCK => self.put(p - 1, e, BLOCK_BEGIN + pre),
            M_RAW => self.put(p, e, RAW_BEGIN),
            _ => {
                self.put(p, p + 1, REGEX);
                self.put(p + 1, e - 1, REGEX_BEGIN);
                self.put(e - 1, e, REGEX);
            }
        }
        self.mode = mode;
        self.eqs = eqs.min(255) as u8;
        if mode == M_REGEX {
            self.rx = 0;
            return Some(self.regex::<H_BRACKET>(e, Q_NONE));
        }
        Some(self.bracket(e))
    }

    /// Where the open bracket's close (`]`, its `=`s, `]`) that starts at `s`, a `]`, ends; `None`
    /// where it is no close.
    #[inline(always)]
    fn closes(&self, s: usize) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        let want = usize::from(self.eqs);
        let mut e = s + 1;
        while e < n && b[e] == b'=' {
            e += 1;
        }
        let k = e - s - 1;
        (b.get(e) == Some(&b']') && (k == want || want == 255 && k >= 255)).then_some(e + 1)
    }

    /// An open bracket's text from `p` ([`Scan::mode`]): to its close (`]`, its `=`s, `]`), the
    /// mode over; or the line's end. A regex's on [`Scan::regex`].
    #[inline(always)]
    fn bracket(&mut self, p: usize) -> usize {
        if self.mode == M_REGEX {
            // the line before's newline the regex's character
            return self.regex::<H_BRACKET>(p, Q_ATOM);
        }
        let b = self.b;
        let n = b.len();
        let pre = if self.mode == M_BLOCK && self.top() == F_CAPS {
            CAPS_COMMENT
        } else {
            0
        };
        let (text, end) = match self.mode {
            M_BLOCK => (BLOCK + pre, BLOCK_END + pre),
            _ => (RAW, RAW_END),
        };
        let mut q = p;
        loop {
            let s = find_any(b, q, *b"]");
            if s >= n {
                break;
            }
            if let Some(e) = self.closes(s) {
                self.put(p, s, text);
                self.put(s, e, end);
                self.mode = M_CODE;
                return e;
            }
            q = s + 1;
        }
        self.put(p, n, text);
        n
    }

    /// A regex's text from `p`, read as CMake reads it (X-106, X-107), spelled in `H`: a bracket
    /// argument's ([`H_BRACKET`], [`M_REGEX`]) to its close, which ends it wherever it stands (in a
    /// class or a group too), the mode over; a quoted argument's ([`H_QUOTED`], [`F_QUOTED_RX`])
    /// to its `"`, the frame popped; an unquoted one's ([`H_UNQUOTED`]) to its end. Or to a
    /// `${…}`'s or `$<…>`'s start, opened on its frame, the regex's state in [`Scan::rx`] (the
    /// regex goes on after it, the substitution an operand); or to the line's end. `prev` is what
    /// a quantifier at `p` would follow ([`Q_NONE`]…).
    ///
    /// CMake's regexes are Henry Spencer's: `^`, `$` and `.` anywhere, `\` and any character
    /// that character, `*`, `+`, `?` after an operand (one; a second, or one after nothing, an
    /// error), `|`, `(…)`; a class `[…]` or `[^…]` whose first `]` or `-` is a character, whose
    /// `x-y` is a range, and where a `\` is a character. A `\` right before the argument's end
    /// escapes nothing (CMake's "trailing \"), a `)` with no group open is unmatched.
    ///
    /// In a quoted or unquoted argument CMake's escapes come first ([`Scan::unit`]): `\\` is the
    /// regex's `\`, and `\` before another character is that character, colored as the regex's
    /// operator where it reads as one (`"\."` is any character, `"\("` a group), else as the
    /// string's escape. A `;` is the string's separator, an operand to the regex.
    #[inline(never)]
    fn regex<const H: u8>(&mut self, p: usize, mut prev: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let (off, plain) = match H {
            H_BRACKET => (0, REGEX),
            H_QUOTED => (RX_IN_STRING, STRING),
            _ => (RX_BARE, 0),
        };
        let mut depth = self.rx & RX_DEPTH;
        let mut set = self.rx & RX_IN_SET != 0;
        // where the regex's plain text not yet written starts
        let mut text = p;
        let mut q = p;
        // a class's first character: a `]` there is one
        let mut first = false;
        macro_rules! flush {
            ($s:expr) => {{
                let s = $s;
                if text < s && plain != 0 {
                    self.put(text, s, plain);
                }
            }};
        }
        macro_rules! token {
            ($s:expr, $e:expr, $kind:expr) => {{
                let (s, e) = ($s, $e);
                flush!(s);
                self.put(s, e, $kind);
                text = e;
                q = e;
            }};
        }
        while q < n {
            let c = b[q];
            if H == H_BRACKET {
                if c == b']'
                    && let Some(e) = self.closes(q)
                {
                    flush!(q);
                    self.put(q, e, REGEX_END);
                    self.mode = M_CODE;
                    self.rx = 0;
                    return e;
                }
            } else if self.ends::<H>(q) {
                flush!(q);
                self.rx = 0;
                if H == H_QUOTED {
                    self.put_hot(q, q + 1, STRING_END);
                    self.pop();
                    return q + 1;
                }
                return q;
            } else if self.yields(q) {
                match c {
                    b'$' => {
                        flush!(q);
                        self.rx = depth | if set { RX_IN_SET } else { 0 };
                        if H == H_UNQUOTED {
                            self.rx |= RX_UNQUOTED;
                            if self.top() != F_UNQUOTED {
                                self.push(F_UNQUOTED);
                            }
                        }
                        return self.dollar(q).unwrap_or(q + 1);
                    }
                    b';' => {
                        let kind = if H == H_QUOTED {
                            S_SEPARATOR
                        } else {
                            SEPARATOR
                        };
                        token!(q, q + 1, kind);
                        first = false;
                        if !set {
                            prev = Q_ATOM;
                        }
                    }
                    // a `\` ending the line: the string's, which goes on to the next one
                    _ => break,
                }
                continue;
            }
            let (u, e, host) = self.unit::<H>(q);
            if set {
                if u == b']' && !first {
                    token!(q, e, RX_SET + off);
                    set = false;
                    prev = Q_ATOM;
                    continue;
                }
                first = false;
                // a range `x-y`, its end no `]`
                if e < n && !self.stops::<H>(e) {
                    let (d, de, _) = self.unit::<H>(e);
                    if d == b'-' && de < n && !self.stops::<H>(de) {
                        let (z, ze, _) = self.unit::<H>(de);
                        if z != b']' {
                            token!(q, ze, RX_RANGE + off);
                            continue;
                        }
                    }
                }
                if host {
                    token!(q, e, escape_kind::<H>(u));
                } else {
                    q = e;
                }
                continue;
            }
            match u {
                b'\\' if !host => {
                    // the regex's escape and the character after it
                    if e == n {
                        // at the line's end the newline's, but where the argument ends there
                        if H == H_UNQUOTED {
                            token!(q, n, RX_BAD_ESCAPE + off);
                        } else {
                            token!(q, n, RX_ESCAPE + off);
                            prev = Q_ATOM;
                        }
                    } else if self.ends_at::<H>(e) {
                        token!(q, e, RX_BAD_ESCAPE + off);
                    } else {
                        // a `$<` escaped is the regex's `$` (`"^\\$<LINK_ONLY:"`): CMake
                        // doesn't evaluate a generator expression in a regex's argument
                        let ee = if self.stops::<H>(e) && !b[e..].starts_with(b"$<") {
                            e
                        } else {
                            self.unit::<H>(e).1
                        };
                        token!(q, ee, RX_ESCAPE + off);
                        prev = Q_ATOM;
                    }
                }
                b'^' | b'$' => {
                    token!(q, e, RX_ANCHOR + off);
                    prev = Q_ANCHOR;
                }
                b'.' => {
                    token!(q, e, RX_ANY + off);
                    prev = Q_ATOM;
                }
                b'(' => {
                    token!(q, e, RX_GROUP + off);
                    if depth < RX_DEPTH {
                        depth += 1;
                    }
                    prev = Q_NONE;
                }
                b')' => {
                    if depth > 0 {
                        token!(q, e, RX_GROUP + off);
                        depth -= 1;
                    } else {
                        token!(q, e, RX_UNMATCHED + off);
                    }
                    prev = Q_ATOM;
                }
                b'|' => {
                    token!(q, e, RX_OR + off);
                    prev = Q_NONE;
                }
                b'*' | b'+' | b'?' => {
                    if prev == Q_ATOM || prev == Q_ANCHOR && u == b'?' {
                        token!(q, e, RX_QUANTIFIER + off);
                        prev = Q_QUANTIFIER;
                    } else {
                        token!(q, e, RX_BAD_QUANTIFIER + off);
                    }
                }
                b'[' => {
                    let e = if e < n && !self.stops::<H>(e) && self.unit::<H>(e).0 == b'^' {
                        self.unit::<H>(e).1
                    } else {
                        e
                    };
                    token!(q, e, RX_SET + off);
                    set = true;
                    first = true;
                    prev = Q_ATOM;
                }
                _ if host => {
                    token!(q, e, escape_kind::<H>(u));
                    prev = Q_ATOM;
                }
                _ => {
                    q = e;
                    prev = Q_ATOM;
                }
            }
        }
        flush!(n);
        self.rx = if H == H_UNQUOTED {
            0
        } else {
            depth | if set { RX_IN_SET } else { 0 }
        };
        n
    }

    /// The regex's character at `q` as `H` spells it: its byte, where it ends, and whether it is
    /// the host's escape. In a quoted or unquoted argument `\\` is the regex's `\`, and `\` before
    /// another character that character (but at the line's end, where the caller stops first).
    #[inline(always)]
    fn unit<const H: u8>(&self, q: usize) -> (u8, usize, bool) {
        let b = self.b;
        let n = b.len();
        let c = b[q];
        if H != H_BRACKET && c == b'\\' && q + 1 < n {
            let d = b[q + 1];
            if d == b'\\' {
                return (b'\\', q + 2, false);
            }
            return (d, (q + 1 + char_len(d)).min(n), true);
        }
        (c, (q + char_len(c)).min(n), false)
    }

    /// Whether a quoted or unquoted regex argument ends at `q`: its `"`, or a blank, a
    /// parenthesis, `#` or `"`.
    #[inline(always)]
    fn ends<const H: u8>(&self, q: usize) -> bool {
        let c = self.b[q];
        if H == H_QUOTED {
            c == b'"'
        } else {
            UNQUOTED_END[usize::from(c)]
        }
    }

    /// Whether the regex's argument ends at `q`, which is in the line: [`Scan::ends`], or a
    /// bracket's close.
    #[inline(always)]
    fn ends_at<const H: u8>(&self, q: usize) -> bool {
        if H == H_BRACKET {
            self.b[q] == b']' && self.closes(q).is_some()
        } else {
            self.ends::<H>(q)
        }
    }

    /// Whether the host reads `q` before the regex does, a quoted or unquoted argument's: a `${`,
    /// `$ENV{` or `$<`, a `;`, a `\` ending the line.
    #[inline(always)]
    fn yields(&self, q: usize) -> bool {
        let b = self.b;
        match b[q] {
            b'$' => self.opens(q),
            b';' => true,
            b'\\' => q + 1 == b.len(),
            _ => false,
        }
    }

    /// Whether a token of the regex that would take `q` (a range's end, an escape's character)
    /// stops before it, in a quoted or unquoted argument: its end, or a byte the host reads first.
    /// A bracket's close is a `]`, which neither takes.
    #[inline(always)]
    fn stops<const H: u8>(&self, q: usize) -> bool {
        H != H_BRACKET && (self.ends::<H>(q) || self.yields(q))
    }
}

/// The kind of CMake's escape of `c` in a regex argument spelled in `H` ([`ESCAPES`], as
/// [`Scan::escape`] reads it in other arguments).
#[inline(always)]
fn escape_kind<const H: u8>(c: u8) -> u16 {
    match (H == H_QUOTED, ESCAPES[usize::from(c)]) {
        (true, true) => S_ESCAPE,
        (true, false) => S_BAD_ESCAPE,
        (false, true) => ESCAPE,
        (false, false) => BAD_ESCAPE,
    }
}

/// The leaf of a command's name that isn't a builtin's.
#[inline(always)]
fn leaf(cmd: u8) -> u16 {
    match cmd {
        C_FUNCTION | C_ENDFUNCTION | C_MACRO | C_ENDMACRO | C_SET => FUNCTION,
        C_INCLUDE => IMPORT,
        _ => CONTROL,
    }
}

/// Whether `b[p..e]`, a word, is bat's capitalized generic argument: `[A-Z_][A-Z0-9_]*`, the
/// byte after it no `-`, `<`, `>`, `=` or `$` (nor a word's, which it can't be).
#[inline(always)]
fn is_caps(b: &[u8], p: usize, e: usize) -> bool {
    matches!(b[p], b'A'..=b'Z' | b'_')
        && b[p + 1..e]
            .iter()
            .all(|&c| matches!(c, b'A'..=b'Z' | b'0'..=b'9' | b'_'))
        && !matches!(b.get(e), Some(b'-' | b'<' | b'>' | b'=' | b'$'))
}

/// bat's builtin commands (`CMakeCommands`), in its `main`'s order: a command's id is its index, and
/// bit `id` of a keyword's mask ([`KEYWORD_LIST`]) says whether the command's arguments take it.
const BUILTINS: [&[u8]; 104] = [
    b"add_compile_options",
    b"add_custom_command",
    b"add_custom_target",
    b"add_definitions",
    b"add_dependencies",
    b"add_executable",
    b"add_library",
    b"add_subdirectory",
    b"add_test",
    b"aux_source_directory",
    b"build_command",
    b"build_name",
    b"cmake_host_system_information",
    b"cmake_minimum_required",
    b"cmake_parse_arguments",
    b"cmake_policy",
    b"configure_file",
    b"create_test_sourcelist",
    b"ctest_build",
    b"ctest_configure",
    b"ctest_coverage",
    b"ctest_empty_binary_directory",
    b"ctest_memcheck",
    b"ctest_read_custom_files",
    b"ctest_run_script",
    b"ctest_sleep",
    b"ctest_start",
    b"ctest_submit",
    b"ctest_test",
    b"ctest_update",
    b"ctest_upload",
    b"define_property",
    b"enable_language",
    b"enable_testing",
    b"exec_program",
    b"execute_process",
    b"export",
    b"export_library_dependencies",
    b"file",
    b"find_file",
    b"find_library",
    b"find_package",
    b"find_path",
    b"find_program",
    b"fltk_wrap_ui",
    b"get_cmake_property",
    b"get_directory_property",
    b"get_filename_component",
    b"get_property",
    b"get_source_file_property",
    b"get_target_property",
    b"get_test_property",
    b"include_directories",
    b"include_external_msproject",
    b"include_regular_expression",
    b"install",
    b"install_files",
    b"install_programs",
    b"install_targets",
    b"link_directories",
    b"link_libraries",
    b"list",
    b"load_cache",
    b"load_command",
    b"make_directory",
    b"mark_as_advanced",
    b"math",
    b"message",
    b"option",
    b"output_required_files",
    b"project",
    b"qt_wrap_cpp",
    b"qt_wrap_ui",
    b"remove",
    b"remove_definitions",
    b"return",
    b"separate_arguments",
    b"set_directory_properties",
    b"set_property",
    b"set_source_files_properties",
    b"set_target_properties",
    b"set_tests_properties",
    b"site_name",
    b"source_group",
    b"string",
    b"subdir_depends",
    b"subdirs",
    b"target_compile_definitions",
    b"target_compile_features",
    b"target_compile_options",
    b"target_include_directories",
    b"target_link_directories",
    b"target_link_libraries",
    b"target_link_options",
    b"target_precompile_headers",
    b"target_sources",
    b"try_compile",
    b"try_run",
    b"unset",
    b"use_mangled_mesa",
    b"utility_source",
    b"variable_requires",
    b"variable_watch",
    b"write_file",
];
/// Every argument keyword bat's grammars name, with the argument lists it is in (a bit each: the
/// builtins' by id, then [`S_IF`], [`S_FOREACH`], [`S_SET`], [`S_INCLUDE`]).
const KEYWORD_LIST: &[(&[u8], u128)] = &[
    (b"OUTPUT", 0x4000000002),
    (b"COMMAND", 0x100000000000000000800000106),
    (b"ARGS", 0x2000000000000000400000002),
    (b"MAIN_DEPENDENCY", 0x2),
    (b"DEPENDS", 0x6),
    (b"BYPRODUCTS", 0x6),
    (b"IMPLICIT_DEPENDS", 0x2),
    (b"WORKING_DIRECTORY", 0x800000106),
    (b"COMMENT", 0x6),
    (b"DEPFILE", 0x2),
    (b"VERBATIM", 0x6),
    (b"APPEND", 0x800010400020000070145c0002),
    (b"TARGET", 0x100000040000001004080040402),
    (b"PRE_BUILD", 0x2),
    (b"PRE_LINK", 0x2),
    (b"POST_BUILD", 0x2),
    (b"ALL", 0x4),
    (b"USES_TERMINAL", 0x4),
    (b"COMMAND_EXPAND_LISTS", 0x4),
    (b"SOURCES", 0x1000000000000000000000004),
    (b"WIN32", 0x4000000020),
    (b"MACOSX_BUNDLE", 0x20),
    (b"EXCLUDE_FROM_ALL", 0x40000000800000000000e0),
    (b"IMPORTED", 0x60),
    (b"GLOBAL", 0x40000001000080000060),
    (b"ALIAS", 0x60),
    (b"STATIC", 0x40),
    (b"SHARED", 0x40),
    (b"MODULE", 0x40),
    (b"UNKNOWN", 0x40),
    (b"OBJECT", 0x40),
    (b"INTERFACE", 0xff8000000000000000000040),
    (b"NAME", 0x1000000000800000000100),
    (b"CONFIGURATIONS", 0x80000000000100),
    (b"CONFIGURATION", 0x40400),
    (b"PROJECT_NAME", 0x40400),
    (b"RESULT", 0x4000001000),
    (b"QUERY", 0x1000),
    (b"VERSION", 0x40000000000000a000),
    (b"FATAL_ERROR", 0x80000000000002000),
    (b"GET", 0x2000000000008000),
    (b"SET", 0x1000000008000),
    (b"NEW", 0x8000),
    (b"OLD", 0x8000),
    (b"PUSH", 0x8000),
    (b"POP", 0x8000),
    (b"COPYONLY", 0x10000),
    (b"ESCAPE_QUOTES", 0x1000000000004000010000),
    (b"@ONLY", 0x1000000000004000010000),
    (b"NEWLINE_STYLE", 0x4000010000),
    (b"EXTRA_INCLUDE", 0x20000),
    (b"FUNCTION", 0x4000020000),
    (b"BUILD", 0x105c0000),
    (b"FLAGS", 0x40000),
    (b"NUMBER_ERRORS", 0x40000),
    (b"NUMBER_WARNINGS", 0x40000),
    (b"RETURN_VALUE", 0x4395c0000),
    (b"CAPTURE_CMAKE_ERROR", 0x501c0000),
    (b"SOURCE", 0x400000010000a0080000),
    (b"OPTIONS", 0x80000),
    (b"QUIET", 0x2007c580000),
    (b"LABELS", 0x100000),
    (b"START", 0x10400000),
    (b"END", 0x10400000),
    (b"STRIDE", 0x10400000),
    (b"EXCLUDE", 0x6000004010400000),
    (b"INCLUDE", 0x2000000010400000),
    (b"EXCLUDE_LABEL", 0x10400000),
    (b"INCLUDE_LABEL", 0x10400000),
    (b"PARALLEL_LEVEL", 0x10400000),
    (b"TEST_LOAD", 0x10400000),
    (b"SCHEDULE_RANDOM", 0x10400000),
    (b"STOP_TIME", 0x10400000),
    (b"DEFECT_COUNT", 0x400000),
    (b"NEW_PROCESS", 0x1000000),
    (b"TRACK", 0x4000000),
    (b"PARTS", 0x8000000),
    (b"FILES", 0x800000380000048000000),
    (b"RETRY_COUNT", 0x8000000),
    (b"RETRY_DELAY", 0x8000000),
    (b"CDASH_UPLOAD", 0x8000000),
    (b"CDASH_UPLOAD_TYPE", 0x8000000),
    (b"DIRECTORY", 0x40000081c04080000000),
    (b"TEST", 0x100000040000001000080000000),
    (b"VARIABLE", 0x1000080000000),
    (b"CACHED_VARIABLE", 0x80000000),
    (b"PROPERTY", 0x40000001000080000000),
    (b"INHERITED", 0x80000000),
    (b"BRIEF_DOCS", 0x1000080000000),
    (b"FULL_DOCS", 0x1000080000000),
    (b"OPTIONAL", 0x800000000000080004100000000),
    (b"OUTPUT_VARIABLE", 0x3000000000000000c00000000),
    (b"TIMEOUT", 0x4800000000),
    (b"RESULT_VARIABLE", 0x800000000000000004800000000),
    (b"ERROR_VARIABLE", 0x800000000),
    (b"INPUT_FILE", 0x800000000),
    (b"OUTPUT_FILE", 0x800000000),
    (b"ERROR_FILE", 0x800000000),
    (b"OUTPUT_QUIET", 0x800000000),
    (b"ERROR_QUIET", 0x800000000),
    (b"OUTPUT_STRIP_TRAILING_WHITESPACE", 0x800000000),
    (b"ERROR_STRIP_TRAILING_WHITESPACE", 0x800000000),
    (b"ENCODING", 0x4800000000),
    (b"EXPORT", 0x80001000000000),
    (b"NAMESPACE", 0x1000000000001000000000),
    (b"FILE", 0x80005000000000),
    (b"TARGETS", 0x80001000000000),
    (b"EXPORT_LINK_INTERFACE_LIBRARIES", 0x1000000000),
    (b"PACKAGE", 0x1000000000),
    (b"ANDROID_MK", 0x1000000000),
    (b"NO_SOURCE_PERMISSIONS", 0x4000000000),
    (b"USE_SOURCE_PERMISSIONS", 0x4000000000),
    (b"PERMISSIONS", 0x80004000000000),
    (b"FILE_PERMISSIONS", 0x4000000000),
    (b"DIRECTORY_PERMISSIONS", 0x4000000000),
    (b"REGEX", 0x1000002000004000000000),
    (b"INPUT", 0x4000000000),
    (b"DESTINATION", 0x80004000000000),
    (b"VERBOSE", 0x4000000000),
    (b"READ", 0x4000000000),
    (b"OFFSET", 0x4000000000),
    (b"LIMIT", 0x4000000000),
    (b"HEX", 0x4000000000),
    (b"STRINGS", 0x4000000000),
    (b"LENGTH_MAXIMUM", 0x4000000000),
    (b"LENGTH_MINIMUM", 0x4000000000),
    (b"LIMIT_COUNT", 0x4000000000),
    (b"LIMIT_INPUT", 0x4000000000),
    (b"LIMIT_OUTPUT", 0x4000000000),
    (b"NEWLINE_CONSUME", 0x4000000000),
    (b"NO_HEX_CONVERSION", 0x4000000000),
    (b"MD5", 0x1000000000004000000000),
    (b"SHA1", 0x1000000000004000000000),
    (b"SHA224", 0x1000000000004000000000),
    (b"SHA256", 0x1000000000004000000000),
    (b"SHA384", 0x1000000000004000000000),
    (b"SHA512", 0x1000000000004000000000),
    (b"SHA3_224", 0x1000000000004000000000),
    (b"SHA3_256", 0x1000000000004000000000),
    (b"SHA3_384", 0x1000000000004000000000),
    (b"SHA3_512", 0x1000000000004000000000),
    (b"TIMESTAMP", 0x1000000000004000000000),
    (b"UTC", 0x1000000000004000000000),
    (b"GET_RUNTIME_DEPENDENCIES", 0x4000000000),
    (b"RESOLVED_DEPENDENCIES_VAR", 0x4000000000),
    (b"UNRESOLVED_DEPENDENCIES_VAR", 0x4000000000),
    (b"CONFLICTING_DEPENDENCIES_PREFIX", 0x4000000000),
    (b"EXECUTABLES", 0x4000000000),
    (b"LIBRARIES", 0x4000000000),
    (b"MODULES", 0x4000000000),
    (b"DIRECTORIES", 0x4000000000),
    (b"BUNDLE_EXECUTABLE", 0x4000000000),
    (b"PRE_INCLUDE_REGEXES", 0x4000000000),
    (b"PRE_EXCLUDE_REGEXES", 0x4000000000),
    (b"POST_INCLUDE_REGEXES", 0x4000000000),
    (b"POST_EXCLUDE_REGEXES", 0x4000000000),
    (b"POST_INCLUDE_FILES", 0x4000000000),
    (b"POST_EXCLUDE_FILES", 0x4000000000),
    (b"WRITE", 0x4000000000),
    (b"TOUCH", 0x4000000000),
    (b"TOUCH_NOCREATE", 0x4000000000),
    (b"GENERATE", 0x4000000000),
    (b"CONDITION", 0x4000000000),
    (b"CONFIGURE", 0x1000000000004000000000),
    (b"UNIX", 0x4000000000),
    (b"DOS", 0x4000000000),
    (b"LF", 0x4000000000),
    (b"CRLF", 0x4000000000),
    (b"CONTENT", 0x4000000000),
    (b"GLOB", 0x4000000000),
    (b"GLOB_RECURSE", 0x4000000000),
    (b"FOLLOW_SYMLINKS", 0x4000000000),
    (b"LIST_DIRECTORIES", 0x4000000000),
    (b"RELATIVE", 0x4000000000),
    (b"CONFIGURE_DEPENDS", 0x4000000000),
    (b"MAKE_DIRECTORY", 0x4000000000),
    (b"REMOVE", 0x4000000000),
    (b"REMOVE_RECURSE", 0x4000000000),
    (b"RENAME", 0x4000000000),
    (b"NO_REPLACE", 0x4000000000),
    (b"COPY_FILE", 0x1000000000000004000000000),
    (b"ONLY_IF_DIFFERENT", 0x4000000000),
    (b"COPY", 0x4000000000),
    (b"INSTALL", 0x40000001004000000000),
    (b"FOLLOW_SYMLINK_CHAIN", 0x4000000000),
    (b"FILES_MATCHING", 0x80004000000000),
    (b"PATTERN", 0x80004000000000),
    (b"SIZE", 0x4000000000),
    (b"READ_SYMLINK", 0x4000000000),
    (b"CREATE_LINK", 0x4000000000),
    (b"COPY_ON_ERROR", 0x4000000000),
    (b"SYMBOLIC", 0x4000000000),
    (b"CHMOD", 0x4000000000),
    (b"CHMOD_RECURSE", 0x4000000000),
    (b"REAL_PATH", 0x4000000000),
    (b"BASE_DIRECTORY", 0x4000000000),
    (b"EXPAND_TILDE", 0x4000000000),
    (b"RELATIVE_PATH", 0x4000000000),
    (b"TO_CMAKE_PATH", 0x4000000000),
    (b"TO_NATIVE_PATH", 0x4000000000),
    (b"DOWNLOAD", 0x4000000000),
    (b"UPLOAD", 0x4000000000),
    (b"INACTIVITY_TIMEOUT", 0x4000000000),
    (b"LOG", 0x4000000000),
    (b"SHOW_PROGRESS", 0x4000000000),
    (b"STATUS", 0x80000004000000000),
    (b"USERPWD", 0x4000000000),
    (b"HTTPHEADER", 0x4000000000),
    (b"NETRC", 0x4000000000),
    (b"IGNORED", 0x4000000000),
    (b"REQUIRED", 0x24000000000),
    (b"TLS_VERIFY", 0x4000000000),
    (b"TLS_CAINFO", 0x4000000000),
    (b"EXPECTED_HASH", 0x4000000000),
    (b"EXPECTED_MD5", 0x4000000000),
    (b"RANGE_START", 0x4000000000),
    (b"RANGE_END", 0x4000000000),
    (b"LOCK", 0x4000000000),
    (b"RELEASE", 0x4000000000),
    (b"GUARD", 0x4000000000),
    (b"PROCESS", 0x4000000000),
    (b"ARCHIVE_CREATE", 0x4000000000),
    (b"PATHS", 0xfc000000000),
    (b"FORMAT", 0x4000000000),
    (b"COMPRESSION", 0x4000000000),
    (b"COMPRESSION_LEVEL", 0x4000000000),
    (b"MTIME", 0x4000000000),
    (b"ARCHIVE_EXTRACT", 0x4000000000),
    (b"PATTERNS", 0x4000000000),
    (b"LIST_ONLY", 0x4000000000),
    (b"NAMES", 0xf8000000000),
    (b"HINTS", 0xf8000000000),
    (b"PATH_SUFFIXES", 0xf8000000000),
    (b"DOC", 0xd8000000000),
    (b"NO_DEFAULT_PATH", 0xf8000000000),
    (b"NO_CMAKE_ENVIRONMENT_PATH", 0xf8000000000),
    (b"NO_CMAKE_PATH", 0xf8000000000),
    (b"NO_SYSTEM_ENVIRONMENT_PATH", 0xf8000000000),
    (b"NO_CMAKE_SYSTEM_PATH", 0xf8000000000),
    (b"CMAKE_FIND_ROOT_PATH_BOTH", 0xf8000000000),
    (b"ONLY_CMAKE_FIND_ROOT_PATH", 0xf8000000000),
    (b"NO_CMAKE_FIND_ROOT_PATH", 0xf8000000000),
    (b"NAMES_PER_DIR", 0x10000000000),
    (b"EXACT", 0x20000000000),
    (b"COMPONENTS", 0x20000000000),
    (b"CONFIG", 0x20000000000),
    (b"NO_MODULE", 0x20000000000),
    (b"NO_POLICY_SCOPE", 0x800000000000000020000000000),
    (b"CONFIGS", 0x20000000000),
    (b"NO_CMAKE_PACKAGE_REGISTRY", 0x20000000000),
    (b"NO_CMAKE_BUILDS_PATH", 0x20000000000),
    (b"NO_CMAKE_SYSTEM_PACKAGE_REGISTRY", 0x20000000000),
    (b"DEFINITION", 0x400000000000),
    (b"EXT", 0x800000000000),
    (b"NAME_WE", 0x800000000000),
    (b"PATH", 0x800000000000),
    (b"BASE_DIR", 0x800000000000),
    (b"CACHE", 0x404000040000001800000000000),
    (b"PROGRAM", 0x800000000000),
    (b"PROGRAM_ARGS", 0x800000000000),
    (b"DEFINED", 0x100000000000001000000000000),
    (b"AFTER", 0x10000000000000),
    (b"BEFORE", 0x2e0000000010000000000000),
    (b"SYSTEM", 0x40000000010000000000000),
    (b"TYPE", 0x1000000020000000000000),
    (b"GUID", 0x20000000000000),
    (b"PLATFORM", 0x20000000000000),
    (b"ARCHIVE", 0x80000000000000),
    (b"LIBRARY", 0x80000000000000),
    (b"RUNTIME", 0x80000000000000),
    (b"FRAMEWORK", 0x80000000000000),
    (b"BUNDLE", 0x80000000000000),
    (b"PRIVATE_HEADER", 0x80000000000000),
    (b"PUBLIC_HEADER", 0x80000000000000),
    (b"RESOURCE", 0x80000000000000),
    (b"COMPONENT", 0x80000000000000),
    (b"NAMELINK_ONLY", 0x80000000000000),
    (b"NAMELINK_SKIP", 0x80000000000000),
    (b"PROGRAMS", 0x80000000000000),
    (b"RUNTIME_DIRECTORY", 0x400000000000000),
    (b"debug", 0x1000000000000000),
    (b"optimized", 0x1000000000000000),
    (b"general", 0x1000000000000000),
    (b"LENGTH", 0x1000002000000000000000),
    (b"FILTER", 0x2000000000000000),
    (b"FIND", 0x1000002000000000000000),
    (b"INSERT", 0x2000000000000000),
    (b"REMOVE_ITEM", 0x2000000000000000),
    (b"REMOVE_AT", 0x2000000000000000),
    (b"REMOVE_DUPLICATES", 0x2000000000000000),
    (b"REVERSE", 0x1000002000000000000000),
    (b"SORT", 0x2000000000000000),
    (b"READ_WITH_PREFIX", 0x4000000000000000),
    (b"INCLUDE_INTERNALS", 0x4000000000000000),
    (b"COMMAND_NAME", 0x8000000000000000),
    (b"CLEAR", 0x20000000000000000),
    (b"FORCE", 0x400000000020000000000000000),
    (b"EXPR", 0x40000000000000000),
    (b"WARNING", 0x80000000000000000),
    (b"AUTHOR_WARNING", 0x80000000000000000),
    (b"SEND_ERROR", 0x80000000000000000),
    (b"DEPRECATION", 0x80000000000000000),
    (b"LANGUAGES", 0x400000000000000000),
    (b"UNIX_COMMAND", 0x10000000000000000000),
    (b"WINDOWS_COMMAND", 0x10000000000000000000),
    (b"PROPERTIES", 0x3a0000000000000000000),
    (b"APPEND_STRING", 0x40000000000000000000),
    (b"REGULAR_EXPRESSION", 0x800000000000000000000),
    (b"TREE", 0x800000000000000000000),
    (b"PREFIX", 0x800000000000000000000),
    (b"REPLACE", 0x1000000000000000000000),
    (b"MATCH", 0x1000000000000000000000),
    (b"MATCHALL", 0x1000000000000000000000),
    (b"CONCAT", 0x1000000000000000000000),
    (b"TOLOWER", 0x1000000000000000000000),
    (b"TOUPPER", 0x1000000000000000000000),
    (b"SUBSTRING", 0x1000000000000000000000),
    (b"STRIP", 0x1000000000000000000000),
    (b"GENEX_STRIP", 0x1000000000000000000000),
    (b"COMPARE", 0x1000000000000000000000),
    (b"LESS", 0x100001000000000000000000000),
    (b"GREATER", 0x100001000000000000000000000),
    (b"EQUAL", 0x100001000000000000000000000),
    (b"NOTEQUAL", 0x1000000000000000000000),
    (b"LESS_EQUAL", 0x100001000000000000000000000),
    (b"GREATER_EQUAL", 0x100001000000000000000000000),
    (b"ASCII", 0x1000000000000000000000),
    (b"RANDOM", 0x1000000000000000000000),
    (b"ALPHABET", 0x1000000000000000000000),
    (b"RANDOM_SEED", 0x1000000000000000000000),
    (b"MAKE_C_IDENTIFIER", 0x1000000000000000000000),
    (b"UUID", 0x1000000000000000000000),
    (b"UPPER", 0x1000000000000000000000),
    (b"PREORDER", 0x4000000000000000000000),
    (b"PUBLIC", 0xff8000000000000000000000),
    (b"PRIVATE", 0xff8000000000000000000000),
    (b"LINK_PUBLIC", 0x100000000000000000000000),
    (b"LINK_PRIVATE", 0x100000000000000000000000),
    (b"LINK_INTERFACE_LIBRARIES", 0x100000000000000000000000),
    (b"CMAKE_FLAGS", 0x3000000000000000000000000),
    (b"COMPILE_DEFINITIONS", 0x3000000000000000000000000),
    (b"LINK_LIBRARIES", 0x3000000000000000000000000),
    (b"COPY_FILE_ERROR", 0x1000000000000000000000000),
    (b"C_STANDARD_REQUIRED", 0x1000000000000000000000000),
    (b"CXX_STANDARD_REQUIRED", 0x1000000000000000000000000),
    (b"CUDA_STANDARD_REQUIRED", 0x1000000000000000000000000),
    (b"C_EXTENSIONS", 0x1000000000000000000000000),
    (b"CXX_EXTENSIONS", 0x1000000000000000000000000),
    (b"CUDA_EXTENSIONS", 0x1000000000000000000000000),
    (b"COMPILE_OUTPUT_VARIABLE", 0x2000000000000000000000000),
    (b"RUN_OUTPUT_VARIABLE", 0x2000000000000000000000000),
    (b"PARENT_SCOPE", 0x404000000000000000000000000),
    (b"STREQUAL", 0x100000000000000000000000000),
    (b"NOT", 0x100000000000000000000000000),
    (b"AND", 0x100000000000000000000000000),
    (b"OR", 0x100000000000000000000000000),
    (b"POLICY", 0x100000000000000000000000000),
    (b"EXISTS", 0x100000000000000000000000000),
    (b"IS_NEWER_THAN", 0x100000000000000000000000000),
    (b"IS_DIRECTORY", 0x100000000000000000000000000),
    (b"IS_SYMLINK", 0x100000000000000000000000000),
    (b"IS_ABSOLUTE", 0x100000000000000000000000000),
    (b"VERSION_LESS", 0x100000000000000000000000000),
    (b"STRLESS", 0x100000000000000000000000000),
    (b"VERSION_GREATER", 0x100000000000000000000000000),
    (b"STRGREATER", 0x100000000000000000000000000),
    (b"VERSION_EQUAL", 0x100000000000000000000000000),
    (b"VERSION_LESS_EQUAL", 0x100000000000000000000000000),
    (b"STRLESS_EQUAL", 0x100000000000000000000000000),
    (b"VERSION_GREATER_EQUAL", 0x100000000000000000000000000),
    (b"STRGREATER_EQUAL", 0x100000000000000000000000000),
    (b"MATCHES", 0x100000000000000000000000000),
    (b"IN", 0x200000000000000000000000000),
    (b"LISTS", 0x200000000000000000000000000),
    (b"ITEMS", 0x200000000000000000000000000),
    (b"RANGE", 0x200000000000000000000000000),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Lang;
    use crate::tests::marked;

    fn of(lines: &[&str]) -> Vec<(String, String)> {
        marked(Lang::CMake, lines)
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
    fn commands_and_blocks() {
        let l = [
            "Add_Library(x STATIC a.c)",
            "if(A)",
            "  break()",
            "else()",
            "  elseif()",
            "ENDIF()",
            "endif()",
            "foreach(x IN LISTS y)",
            "  continue()",
            "endforeach()",
            "continue()",
            "my_fn(x)",
        ];
        has(&l, "Add_Library", "support.function.cmake");
        has(&l, "STATIC", "variable.parameter.cmake");
        has(&l, "break", "keyword.control.cmake");
        has(&l, "ENDIF", "keyword.control.cmake");
        // an `elseif` in an `else`'s block, an `endif` past the block's
        has(&l, "elseif", "invalid.illegal.stray.cmake");
        has(&l, "endif", "invalid.illegal.stray.cmake");
        has(&l, "IN", "variable.parameter.cmake");
        has(&l, "continue", "keyword.control.cmake");
        // `continue` outside a loop is bat's generic command
        has(&l, "continue", "variable.function.generic.cmake");
        has(&l, "my_fn", "variable.function.generic.cmake");
    }

    #[test]
    fn what_bat_reads_on_one_line() {
        let l = [
            "set(FOO 1 CACHE PATHS \"d\")",
            "set(",
            "  BAR 2)",
            "function(f a)",
            "endfunction()",
            "include(x RESULT_VARIABLE r)",
            "if(A MATCHES [=[x]=] OR B",
            "  [[y]])",
            "endif()",
            "if x",
            "install(TARGETS x INCLUDES DESTINATION y)",
            "set($ value)",
            "message(STATUS x) junk",
            "()",
            "set(Y 1 CACHE# BOOL)",
        ];
        has(&l, "FOO", "variable.other.readwrite.assignment.cmake");
        // a `$` that opens nothing ends the name's place
        lacks(&l, "value");
        // `#` right after `CACHE` a comment, after a blank the type's error
        has(&l, " BOOL)", "comment.line.cmake");
        // the name's place ends with the line
        lacks(&l, "BAR");
        // `CACHE`'s type a prefix of the word
        has(&l, "PATH", "storage.type.cmake");
        has(&l, "f", "entity.name.function.cmake");
        has(&l, "r", "variable.other.readwrite.cmake");
        has(
            &l,
            "=",
            "string.regexp.cmake punctuation.definition.string.begin.cmake",
        );
        has(
            &l,
            "]=]",
            "string.regexp.cmake punctuation.definition.string.end.cmake",
        );
        has(
            &l,
            "[[",
            "string.raw.cmake punctuation.definition.string.begin.cmake",
        );
        // a word after a command's `)` an error (X-119), as a name without its `(`
        has(&l, "junk", "invalid.illegal.stray.cmake");
        has(&l, "()", "invalid.illegal.stray.cmake");
        has(&l, "x", "invalid.illegal.expected-opening-brace.cmake");
        has(
            &l,
            "INCLUDES DESTINATION",
            "variable.parameter.INCLUDES DESTINATION.cmake",
        );
    }

    #[test]
    fn commands_as_cmake_parses_them() {
        // X-119: `(` nests in a builtin's arguments; a name's `(` on its line; one command a
        // line; `@ONLY`, `debug`, `optimized`, `general` keywords where CMake has them
        let l = [
            "message(STATUS (a (b) STATUS) c)",
            "list(APPEND x (y))",
            "foo bar(x)",
            "set # note",
            "(A 1)",
            "message",
            "  (STATUS y)",
            "set(B 2) # fine",
            "set(C 3) #[[ fine ]] # too",
            "#[[ a ]] message(STATUS z)",
            "${X}(y)",
            "\u{feff}project(p)",
            "configure_file(a b @ONLY)",
            "configure_file(a b @ONLY@)",
            "target_link_libraries(t debug a optimized b general c)",
            "add_library(x STATIC (y)) 1",
        ];
        // a `(` nests: the arguments go on past its `)`, the command's keywords in it
        let got = of(&l);
        // bytes in spans of a leaf (adjacent spans of one leaf merge: `))`)
        let n = |s: &str| {
            got.iter()
                .filter(|(_, b)| b == s)
                .map(|(a, _)| a.len())
                .sum::<usize>()
        };
        assert!(
            !got.iter().any(|(_, s)| s.contains("stray.parenthesis")),
            "{got:#?}"
        );
        // `STATUS` twice, `APPEND`, `@ONLY`, `debug`, `optimized`, `general`, `STATIC`
        assert_eq!(n("variable.parameter.cmake"), 2 * 6 + 6 + 5 + 5 + 9 + 7 + 6);
        assert_eq!(n("punctuation.section.parens.begin.cmake"), 13, "{got:#?}");
        assert_eq!(n("punctuation.section.parens.end.cmake"), 13, "{got:#?}");
        has(&l, "APPEND", "variable.parameter.cmake");
        // a word between a name and its `(` an error, the command over
        has(&l, "bar(x)", "invalid.illegal.expected-opening-brace.cmake");
        // a name whose `(` isn't on its line: the command over at the line's end, a comment
        // ending it too; a `(` at a command's place an error
        has(&l, "(A 1)", "invalid.illegal.stray.cmake");
        has(&l, "(STATUS y)", "invalid.illegal.stray.cmake");
        lacks(&l, "A");
        // only blanks and comments after a command or a bracket comment
        has(&l, " fine", "comment.line.cmake");
        has(&l, " fine ", "comment.block.cmake");
        has(&l, " too", "comment.line.cmake");
        has(&l, "message(STATUS z)", "invalid.illegal.stray.cmake");
        has(&l, "${X}(y)", "invalid.illegal.stray.cmake");
        has(&l, "1", "invalid.illegal.stray.cmake");
        // a byte order mark skipped
        has(&l, "project", "support.function.cmake");
        has(&l, "@ONLY", "variable.parameter.cmake");
        lacks(&l, "@ONLY@");
        has(&l, "debug", "variable.parameter.cmake");
        has(&l, "optimized", "variable.parameter.cmake");
        has(&l, "general", "variable.parameter.cmake");
    }

    #[test]
    fn templates_placeholders_and_generic_brackets() {
        // a template's placeholders where a command stands (X-119); a generic command's
        // bracket argument (X-110)
        let l = [
            "@PACKAGE_INIT@",
            "z_vcpkg_@PORT@_get_link_libraries(x)",
            "@a@ b",
            "@ x",
            "check_c_source_compiles([[#include <a.h>",
            "int main(void) { return \"\"; }]] HAVE_A)",
            "message(STATUS ok)",
            "set(K \"@k@\")@code@",
            "g() @ zz",
        ];
        has(
            &l,
            "@",
            "punctuation.definition.variable.substitution.cmake",
        );
        // after a command's `)` too, where configuring puts more commands (X-145)
        has(&l, "code", "variable.other.readwrite.cmake");
        has(&l, "@ zz", "invalid.illegal.stray.cmake");
        has(&l, "PACKAGE_INIT", "variable.other.readwrite.cmake");
        has(&l, "z_vcpkg_", "variable.function.generic.cmake");
        has(&l, "PORT", "variable.other.readwrite.cmake");
        has(&l, "_get_link_libraries", "variable.function.generic.cmake");
        // no `(` after a placeholder: the word after it an error, as after a name
        has(&l, "b", "invalid.illegal.expected-opening-brace.cmake");
        has(&l, "@ x", "invalid.illegal.stray.cmake");
        has(&l, "#include <a.h>", "string.raw.cmake");
        has(&l, "int main(void) { return \"\"; }", "string.raw.cmake");
        has(&l, "HAVE_A", "variable.parameter.generic.cmake");
        has(&l, "STATUS", "variable.parameter.cmake");
    }

    #[test]
    fn strings_substitutions_and_comments() {
        let l = [
            "foo(\"a ${B} $ENV{C} $<D:e> \\n \\q ;",
            "f\" g\\;h FOO/${I}#c",
            "J ${a_${b}} $<$<K:l>:m # n",
            ">) # done",
            "#[[ a",
            "]] x()",
        ];
        has(&l, "a ", "string.quoted.double.cmake");
        has(
            &l,
            "B",
            "string.quoted.double.cmake variable.other.readwrite.cmake",
        );
        has(
            &l,
            "ENV",
            "string.quoted.double.cmake keyword.operator.word.cmake",
        );
        has(
            &l,
            ":",
            "string.quoted.double.cmake punctuation.separator.generator-expression.cmake",
        );
        has(
            &l,
            "\\n",
            "string.quoted.double.cmake constant.character.escape.cmake",
        );
        has(
            &l,
            "\\q",
            "string.quoted.double.cmake invalid.illegal.character.escape.cmake",
        );
        has(&l, "\\;", "constant.character.escape.cmake");
        // a capitalized argument runs across the line's end, its `${…}` and comment in it
        has(&l, "FOO/", "variable.parameter.generic.cmake");
        has(
            &l,
            "I",
            "variable.parameter.generic.cmake variable.other.readwrite.cmake",
        );
        has(
            &l,
            "c",
            "variable.parameter.generic.cmake comment.line.cmake",
        );
        has(&l, "J", "variable.parameter.generic.cmake");
        // after a generator expression's `:`, no comment
        has(&l, "n", "variable.other.readwrite.cmake");
        has(&l, " done", "comment.line.cmake");
        has(&l, " a", "comment.block.cmake");
        // a command after a bracket comment an error (X-119)
        has(&l, "x()", "invalid.illegal.stray.cmake");
    }

    #[test]
    fn a_matches_regex_is_read_as_cmake_reads_it() {
        let l = [
            "if(A MATCHES [=[^a(b|c)+.d*?$x\\.y]=])",
            "if(A MATCHES [[[]a-z_0-9_-]b[^-x]a]])",
            "if(A MATCHES [[*x(+)|?)^*^?\\]])",
            "if(A MATCHES [=[;(10\\.[0-9.]+)[;\\]]=] OR B)",
            "if(A MATCHES [[([a",
            "]b)*",
            "c)]] OR C)",
        ];
        let rx = |s: &str| format!("string.regexp.cmake {s}.regexp");
        has(&l, "^", &rx("keyword.control.anchors"));
        has(&l, "$", &rx("keyword.control.anchors"));
        has(&l, "(", &rx("keyword.control.group"));
        has(&l, ")", &rx("keyword.control.group"));
        has(&l, "|", &rx("keyword.operator.alternation"));
        // at the top level as in a group: bat leaves them plain there
        has(&l, "+", &rx("keyword.operator.quantifier"));
        has(&l, ".", &rx("keyword.other.any"));
        has(&l, "\\.", &rx("constant.character.escape"));
        // a second quantifier, and one after nothing, `(`, `|` or (but `?`) an anchor
        has(&l, "?", &rx("invalid.illegal.unexpected-quantifier"));
        has(&l, "*", &rx("invalid.illegal.unexpected-quantifier"));
        has(&l, "+", &rx("invalid.illegal.unexpected-quantifier"));
        has(&l, "?", &rx("keyword.operator.quantifier"));
        has(&l, ")", &rx("invalid.illegal.unmatched-brace"));
        // a `\` before the close escapes nothing
        has(&l, "\\", &rx("invalid.illegal.character.escape"));
        // a class: its first `]` and `-` and a `\` in it characters, `x-y` a range
        has(&l, "[", &rx("keyword.control.set"));
        has(&l, "[^", &rx("keyword.control.set"));
        has(&l, "]", &rx("keyword.control.set"));
        has(&l, "]", "string.regexp.cmake");
        has(&l, "a-z", &rx("constant.other.range"));
        has(&l, "0-9", &rx("constant.other.range"));
        has(&l, "_-", "string.regexp.cmake");
        has(&l, "-x", "string.regexp.cmake");
        // bat's class takes `\]` and runs past the close; CMake's ends at `]`, and the
        // bracket at `]=]`
        has(&l, ";\\", "string.regexp.cmake");
        has(
            &l,
            "]=]",
            "string.regexp.cmake punctuation.definition.string.end.cmake",
        );
        has(&l, "OR", "keyword.operator.logical.cmake");
        // a class and a group open across lines
        has(&l, "]", &rx("keyword.control.set"));
        has(&l, "b", "string.regexp.cmake");
    }

    #[test]
    fn regex_arguments_are_read_through_cmakes_escapes() {
        let l = [
            "if(A MATCHES \"^a\\\\.(b|c)+\\.d$\" OR B MATCHES ^x[0-9]*${Y}z$)",
            "string(REGEX REPLACE \"[\\\\/]\\\\+\" \"\\\\1.\" out \"${in}\")",
            "string(REPLACE \"a.b\" \"c\" out x)",
            "list(FILTER l INCLUDE REGEX \"\\(q\\)\\t\\d;w\")",
            "file(STRINGS f v REGEX \"^#define X \\\"[^\\\"]*\\\"\")",
            "string(REGEX MATCH \"([a${C}",
            "]+)*\" m x)",
            "if(A MATCHES [a-z]+\\\\)",
        ];
        let q = |s: &str| format!("string.quoted.double.cmake {s}.regexp");
        let u = |s: &str| format!("{s}.regexp");
        // a quoted argument: `\\` the regex's `\`, under the string's scope
        has(&l, "^", &q("keyword.control.anchors"));
        has(&l, "\\\\.", &q("constant.character.escape"));
        has(&l, "(", &q("keyword.control.group"));
        has(&l, "|", &q("keyword.operator.alternation"));
        has(&l, "+", &q("keyword.operator.quantifier"));
        // CMake's `\.` is the regex's `.`, its `\(` a group
        has(&l, "\\.", &q("keyword.other.any"));
        has(&l, "\\(", &q("keyword.control.group"));
        has(&l, "\\)", &q("keyword.control.group"));
        // CMake's other escapes the string's, `\d` an error, `;` its separator
        has(
            &l,
            "\\t",
            "string.quoted.double.cmake constant.character.escape.cmake",
        );
        has(
            &l,
            "\\d",
            "string.quoted.double.cmake invalid.illegal.character.escape.cmake",
        );
        has(
            &l,
            "\\\"",
            "string.quoted.double.cmake constant.character.escape.cmake",
        );
        has(
            &l,
            ";",
            "string.quoted.double.cmake punctuation.separator.cmake",
        );
        // a `\\` in a class a character; the class's `]`
        has(&l, "[^", &q("keyword.control.set"));
        has(&l, "\\\\/", "string.quoted.double.cmake");
        has(&l, "]", &q("keyword.control.set"));
        // an unquoted argument, its `${…}` an operand
        has(&l, "^", &u("keyword.control.anchors"));
        has(&l, "0-9", &u("constant.other.range"));
        has(&l, "*", &u("keyword.operator.quantifier"));
        has(&l, "Y", "variable.other.readwrite.cmake");
        has(&l, "$", &u("keyword.control.anchors"));
        // a `\\` before the argument's end escapes nothing
        has(&l, "\\\\", &u("invalid.illegal.character.escape"));
        // `string(REPLACE` takes no regex; the replacement isn't one
        has(&l, "a.b", "string.quoted.double.cmake");
        has(&l, "1.", "string.quoted.double.cmake");
        // a class and a group across a `${…}` and a line's end
        has(
            &l,
            "C",
            "string.quoted.double.cmake variable.other.readwrite.cmake",
        );
        has(&l, ")", &q("keyword.control.group"));
        has(&l, "*", &q("keyword.operator.quantifier"));
        has(
            &l,
            "\"",
            "string.quoted.double.cmake punctuation.definition.string.end.cmake",
        );
    }

    #[test]
    fn a_regex_argument_on_a_later_line() {
        let l = [
            "if(A MATCHES",
            "  [[^y]] OR B MATCHES # why",
            "  # more",
            "  \"z+\")",
            "endif()",
            "string(REGEX REPLACE",
            "  \"a|b\" \"\\\\1\" out ${in})",
            "install(DIRECTORY d/ DESTINATION x FILES_MATCHING REGEX \"\\\\.h\")",
            "source_group(TREE . REGULAR_EXPRESSION \"x.*\" FILES a)",
            "file(LOCK f RESULT_VARIABLE $r)",
            "if(A MATCHES",
            "  )",
            "set(x ^w)",
        ];
        let q = |s: &str| format!("string.quoted.double.cmake {s}.regexp");
        // bat reads `MATCHES [[…]]` on its line only; CMake reads the next argument
        has(
            &l,
            "^",
            "string.regexp.cmake keyword.control.anchors.regexp",
        );
        // a comment between keyword and regex, read
        has(&l, " why", "comment.line.cmake");
        has(&l, " more", "comment.line.cmake");
        has(&l, "+", &q("keyword.operator.quantifier"));
        has(&l, "|", &q("keyword.operator.alternation"));
        // `install`'s `REGEX`, as `PATTERN`; `source_group`'s `REGULAR_EXPRESSION`
        has(&l, "\\\\.", &q("constant.character.escape"));
        has(&l, ".", &q("keyword.other.any"));
        has(&l, "*", &q("keyword.operator.quantifier"));
        let got = of(&l);
        // `file(LOCK … RESULT_VARIABLE`'s variable no regex; a `)` ends the awaiting
        assert!(
            !got.iter().any(|(t, s)| s.ends_with(".regexp")
                && (t == "$" || t == "^" && !s.starts_with("string.regexp"))),
            "{got:#?}"
        );
    }

    #[test]
    fn escapes_are_cmakes() {
        let l = ["message(\"\\. \\/ \\é \\t \\q \\0\" a\\.b c\\tn d\\9)"];
        let s = |k: &str| format!("string.quoted.double.cmake {k}.character.escape.cmake");
        // bat's table calls these invalid; CMake reads `\` before any but a letter or digit
        for e in ["\\.", "\\/", "\\é", "\\t"] {
            has(&l, e, &s("constant"));
        }
        for e in ["\\q", "\\0"] {
            has(&l, e, &s("invalid.illegal"));
        }
        has(&l, "\\.", "constant.character.escape.cmake");
        has(&l, "\\t", "constant.character.escape.cmake");
        has(&l, "\\9", "invalid.illegal.character.escape.cmake");
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
        // every field a line hands the next: frames past the levels, a bracket's `=`s past 255,
        // a string's `${…}` across lines, a capitalized argument, the `(` awaited, the
        // arguments' place, a regex's groups and class in a quoted argument and an unquoted one
        let deep = "if(A)\n".repeat(20) + &"foo(( ".repeat(10) + "\n" + &") ".repeat(12);
        let eqs = "=".repeat(300);
        let text = format!(
            "{deep}\n#[{eqs}[ a\n]{eqs}] b ]=] \n]{eqs}]\nfoo(\"${{a $<b:\nc>}}\" FOO\nBAR ${{x\n}}y)\n\
             if\n\n(x)\nset(\n X)\nendif()\n\
             if(A MATCHES [[(([a\n]b)\n|c)*]])\nendif()\n\
             string(REGEX MATCH \"(([a${{b\n}}c]\n)\" x ^(${{y\nz}}|q)$)\n\
             if(A MATCHES\n# c\n[[(x\n]])\nendif()\n"
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
            "a(",
            "if(",
            "(",
            "${",
            "$<",
            "\"${",
            "#[[",
            "[[",
            "]=]",
            "foo(FOO ",
            "set(CACHE ",
            "MATCHES ",
            "MATCHES [[([a-\\",
            "if(A MATCHES \"([a-\\\\",
            "if(A MATCHES ^(${a}[",
            "string(REGEX MATCH \"\\(\\[",
            "a\\",
            "x;",
            "endif() ",
        ] {
            let l = unit.repeat(20_000);
            let t = std::time::Instant::now();
            lines(&l);
            assert!(t.elapsed().as_millis() < 200, "{unit:?}: {:?}", t.elapsed());
        }
    }
}
