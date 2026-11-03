//! Go by a scanner of ada's own (ADR-40, speed first): bat's `Go` grammar compiled by hand, as
//! Rust's, Kotlin's and C#'s were. Each of its contexts that can be open is an arm of one
//! `match` on the top of a stack, trying its patterns in the grammar's order at the place the
//! scan has reached, which is syntect's leftmost match; where no pattern can start, the scan
//! steps on.
//!
//! The grammar (128 contexts, no prototype, nothing embedded) is built of `pop-*` contexts that
//! a lookahead pushes or sets and that pop at the first byte they don't take (`(?=\S)`), so
//! **syntect's guard against a loop** is followed: a pop that matches nothing right after a push
//! that matched nothing there, at the depth that push left, is passed over for the context's
//! later patterns, else the scan steps a character. The anonymous contexts are named here by
//! what they read; the const and var declarations are one set of contexts each. A kind is a
//! leaf, bat's scopes for a token ([`scopes`]); of the meta scopes only `meta.annotation`
//! (a `//go:` line) and `meta.interpolation` (a template's `{{ }}` in a string, which clears
//! the string's scope) are kept, the ones a theme can see. Past ASCII, `[[:alpha:]]` is
//! `char::is_alphabetic`, `\d` Unicode's decimal digits, and `\w` either (marks and connector
//! punctuation aren't followed).
//!
//! The stack between lines is the contexts open above the top level, up to 28 (the bytes of
//! [`State`]'s levels).
//!
//! A string that `regexp.MustCompile(` and the like take is colored as RE2's regex (ADR-40,
//! phase 4; [`crate::regex`]), as GoLand colors it: its text is the regex's, but for the
//! string's end and (in `"…"`) its escapes. Where bat's grammar sees a template or a `%` verb
//! in it, the regex has its own reading.
//!
//! SQL by call site and marker (ADR-40, phase 4; [`crate::sql`]), as GoLand has it: the SQL
//! argument of `database/sql`'s, pgx's and sqlx's calls (`db.Exec(`, `QueryRow(ctx, `,
//! `Get(&dest, `…), and the string after `// language=SQL`. Its text is SQL's, but for the
//! string's end and (in `"…"`) its escapes.

use crate::matcher::KEPT as HALF;
use crate::regex;
use crate::sql;
use crate::{Out, State};

// The contexts, by bat's names where they have one.
/// `main`: `match-any`.
const MAIN: u8 = 0;
/// `{ … }` of `match-braces`.
const BLOCK: u8 = 1;
/// `( … )` of `match-parens`.
const PARENS: u8 = 2;
/// After `]` in an expression (`match-brackets`).
const AFTER_BRACKET: u8 = 3;
/// After `.` (`match-selector`).
const SELECTOR: u8 = 4;
/// `.( … )`, `pop-type-assertion`.
const ASSERTION: u8 = 5;
/// `match-short-variable-declarations`: names before `:=`.
const SHORT_VAR: u8 = 6;
/// `pop-arguments-starting-with-type`, after `make` or `new`.
const MAKE_ARGS: u8 = 7;
/// After `make(`: its first argument a type.
const MAKE_FIRST: u8 = 8;
/// `pop-type`.
const TYPE: u8 = 9;
/// `pop-type-nested-in-parens`.
const TYPE_PARENS: u8 = 10;
/// `[ … ]` in a type, then the type.
const TYPE_BRACKETS: u8 = 11;
/// `pop-named-type`.
const NAMED_TYPE: u8 = 12;
/// After `interface`.
const IFACE_HEAD: u8 = 13;
/// An interface's `{ … }`.
const IFACE_BODY: u8 = 14;
/// An interface's method, after its name.
const IFACE_METHOD: u8 = 15;
/// After `map`.
const MAP_HEAD: u8 = 16;
/// `map[`, to `]`.
const MAP_KEY: u8 = 17;
/// The key's type.
const MAP_KEY_TYPE: u8 = 18;
/// After `map[…]`.
const MAP_VALUE: u8 = 19;
/// After `struct`.
const STRUCT_HEAD: u8 = 20;
/// A struct's `{ … }`.
const STRUCT_BODY: u8 = 21;
/// A field, after its name.
const STRUCT_FIELD: u8 = 22;
/// After `const` or `var`: the const one, and the var one [`VAR`] above it.
const DECL_HEAD: u8 = 23;
/// `const (` or `var (`, to `)`.
const DECL_GROUP: u8 = 24;
/// `pop-const-type-and-or-assignment`, `pop-var-type-and-or-assignment`.
const DECL_TYPE: u8 = 25;
/// `pop-const-assignment-or-terminate` and its var twin.
const DECL_ASSIGN: u8 = 26;
/// `pop-const-expressions` and its var twin.
const DECL_EXPRS: u8 = 27;
/// After the expressions' first byte.
const DECL_VALUES: u8 = 28;
/// The var contexts are the const ones plus this.
const VAR: u8 = 6;
/// After `type`.
const TYPE_HEAD: u8 = 35;
/// `type (`, to `)`.
const TYPE_GROUP: u8 = 36;
/// A type in a group, after its name.
const TYPE_ITEM: u8 = 37;
/// `pop-type-alias-or-typedef`.
const ALIAS: u8 = 38;
/// After `func`.
const FUNC_HEAD: u8 = 39;
/// A method's declaration, before its name.
const METHOD: u8 = 40;
/// `pop-func-signature`.
const SIGNATURE: u8 = 41;
/// `pop-func-parameter-and-return-lists`.
const PARAMS_RETURNS: u8 = 42;
/// `pop-func-return-signature`.
const RETURNS: u8 = 43;
/// `pop-func-parameter-list`.
const PARAM_LIST: u8 = 44;
/// After a parameter list's `(`.
const PARAM_OPEN: u8 = 45;
/// `pop-parameter-list-named`.
const NAMED_PARAMS: u8 = 46;
/// `pop-parameter-type`.
const PARAM_TYPE: u8 = 47;
/// `pop-parameter-list-unnamed`.
const UNNAMED_PARAMS: u8 = 48;
/// A SQL call's `( … )` (ADR-40, phase 4): [`PARENS`], plus where its SQL argument is, from
/// here to [`PSQL_LAST`] (a `PSQL_*` state added). A first argument that doesn't fit the call's
/// shape makes it [`PARENS`].
const PSQL: u8 = 49;
// The states, at an argument's start unless named `SKIP_`, which wait in one for its `,`.
/// A context, or the SQL: `Exec(`, `Query(`, `QueryRow(`, pgx's `Queue(`, sqlx's `MustExec(`….
const PSQL_LEAD: u8 = 0;
/// `Prepare(`: the SQL, or a context and pgx's statement name before it.
const PSQL_LEAD_NAME: u8 = 1;
/// A context, then the SQL: `ExecContext(`, `QueryContext(`….
const PSQL_CTX: u8 = 2;
/// A context, then a destination: sqlx's `GetContext(`, `SelectContext(`.
const PSQL_CTX_DEST: u8 = 3;
/// A destination, `&…`, then the SQL: sqlx's `Get(`, `Select(`.
const PSQL_DEST: u8 = 4;
/// pgx's statement name, then the SQL.
const PSQL_NAME: u8 = 5;
const PSQL_SQL: u8 = 6;
const PSQL_SKIP_SQL: u8 = 7;
const PSQL_SKIP_DEST: u8 = 8;
const PSQL_SKIP_NAME: u8 = 9;
/// After the SQL's string: a `+` continues it.
const PSQL_AFTER: u8 = 10;
const PSQL_LAST: u8 = PSQL + PSQL_AFTER;
// text last
const BLOCK_COMMENT: u8 = 60;
const _: () = assert!(PSQL_LAST < BLOCK_COMMENT);
/// `"…"`.
const STRING: u8 = 61;
/// `` `…` ``.
const RAW: u8 = 62;
/// A template's `{{ … }}` in a string.
const INTERP: u8 = 63;
/// SQL in the string below (ADR-40, phase 4): this, plus SQL's context there
/// ([`crate::sql`]'s `MAIN` to `HASHES`).
const SQL_IN: u8 = 0x40;
const SQL_LAST: u8 = SQL_IN + sql::HASHES;
const _: () = assert!(INTERP < SQL_IN);
/// A regex in the string below (ADR-40, phase 4): this bit, and the regex's state.
const REGEX: u8 = 0x80;
const _: () = assert!(SQL_LAST < REGEX);

/// The leaves: a token's scopes.
static LEAVES: [&str; 105] = [
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

/// How many contexts a line can hold; one pushed past it drops the bottom one.
const MAX: usize = 128;
/// How many the state keeps between lines: [`State`]'s levels and withs.
const KEPT: usize = 2 * HALF;
/// Matches that take nothing, in a row at one place, before the scan steps on regardless.
const STUCK: usize = 64;
/// syntect's stack limit (with its two bottom contexts, `__start` and `main`): at it, a push's
/// pattern matches nothing (a search that finds it gives none), so the patterns after it get
/// their turn. (A match its search cached earlier on the line would end the line instead; not
/// followed.)
const SYNTECT_DEPTH: usize = 99;

/// Lex `b`, a line of Go without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let mut s = Scan {
        b,
        n: b.len(),
        stack: [MAIN; MAX],
        depth: 1,
        exact: state.is_exact(),
        text: PLAIN,
        at: 0,
        loop_at: (usize::MAX, 0),
        looped: false,
        stuck: 0,
        wcache: std::cell::Cell::new((usize::MAX, None)),
        sql_paren: usize::MAX,
        sql_call: PSQL_LEAD,
        sql: sql::Embedded {
            is_end: usize::MAX,
            at: 0,
        },
        sql_end: usize::MAX,
        sql_nl: false,
        mark: if state.flags() & MARKED != 0 {
            0
        } else {
            usize::MAX
        },
        out,
    };
    let n = usize::from(state.depth);
    for i in 0..n {
        s.stack[1 + i] = if i < HALF {
            state.levels[i]
        } else {
            state.withs[i - HALF]
        };
    }
    s.depth = 1 + n;
    s.refresh();
    if s.top() >= SQL_IN {
        s.text = s.string_text();
    }
    s.run();
    s.fill(s.n);
    let n = s.depth - 1;
    let from = n.saturating_sub(KEPT);
    let mut packed = State {
        depth: (n - from) as u8,
        inexact: !s.exact || from > 0,
        ..State::default()
    };
    for (i, &c) in s.stack[1 + from..s.depth].iter().enumerate() {
        if i < HALF {
            packed.levels[i] = c;
        } else {
            packed.withs[i - HALF] = c;
        }
    }
    if s.mark != usize::MAX && sql::marker_holds(b, s.mark, b.len()) {
        packed.set_flags(MARKED);
    }
    *state = packed;
}

/// [`State::flags`]' bit: a SQL marker (`// language=SQL`) waits for its string.
const MARKED: u16 = 1;

/// Whether no pattern of context `c` starts at a space: all but a method's `(?=[^(])`, a
/// parameter list's `(?:{{noise}}…)*`, and the text contexts (a block comment's `^\s*`, a
/// template's `\s-`).
const fn spaces_inert(c: u8) -> bool {
    c < BLOCK_COMMENT && c != METHOD && c != PARAM_OPEN
}

/// An ASCII space other than the line's `\n`.
fn is_blank(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r')
}

/// The bytes of the UTF-8 character that starts with `c`.
fn char_len(c: u8) -> usize {
    match c {
        0xf0.. => 4,
        0xe0.. => 3,
        0xc0.. => 2,
        _ => 1,
    }
}

/// The character that starts at byte `i` of `b`, past ASCII.
#[cold]
fn wide_char(b: &[u8], i: usize) -> Option<char> {
    let e = (i + char_len(b[i])).min(b.len());
    std::str::from_utf8(&b[i..e]).ok()?.chars().next()
}

/// The character that byte `i` of `b` is in, past ASCII.
#[cold]
fn wide_char_at(b: &[u8], i: usize) -> Option<char> {
    let s = (i.saturating_sub(3)..=i)
        .rev()
        .find(|&s| b[s] & 0xc0 != 0x80)
        .unwrap_or(i);
    wide_char(b, s)
}

/// ASCII's word bytes, `[A-Za-z0-9_]`.
static WORD: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 128 {
        t[c] = (c as u8).is_ascii_alphanumeric() || c == b'_' as usize;
        c += 1;
    }
    t
};

/// Whether the character that byte `i` of `b` is in is a word's, `\w` (and `[[:alnum:]_]`):
/// past ASCII, a letter or a decimal digit (`₁` is neither).
fn is_word(b: &[u8], i: usize) -> bool {
    match b.get(i) {
        Some(&c) if c < 0x80 => WORD[usize::from(c)],
        Some(_) => wide_char_at(b, i).is_some_and(|c| c.is_alphabetic() || is_nd(c)),
        None => false,
    }
}

/// Whether `c` is one of Unicode's decimal digits, `\p{Nd}`.
fn is_nd(c: char) -> bool {
    let u = c as u32;
    crate::csharp::ND.iter().any(|&s| u >= s && u - s < 10) || c.is_ascii_digit()
}

/// Whether the character at `i` can start a name, `[[:alpha:]_]`.
fn is_name_start(b: &[u8], i: usize) -> bool {
    match b.get(i) {
        Some(&c) if c < 0x80 => c.is_ascii_alphabetic() || c == b'_',
        Some(_) => wide_char(b, i).is_some_and(char::is_alphabetic),
        None => false,
    }
}

fn word_end(b: &[u8], mut p: usize) -> usize {
    loop {
        // ASCII's run at once
        while p < b.len() && WORD[usize::from(b[p])] {
            p += 1;
        }
        if p < b.len() && b[p] >= 0x80 && is_word(b, p) {
            p += char_len(b[p]);
        } else {
            return p.min(b.len());
        }
    }
}

/// Whether `\b` holds before `p`, at a word's start.
fn starts_word(b: &[u8], p: usize) -> bool {
    p == 0 || !is_word(b, p - 1)
}

/// `\s` at `p`: how many bytes the space takes.
fn space_len(b: &[u8], p: usize) -> Option<usize> {
    let &c = b.get(p)?;
    if c < 0x80 {
        return matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r').then_some(1);
    }
    wide_space(b, p)
}

#[cold]
fn wide_space(b: &[u8], p: usize) -> Option<usize> {
    let c = wide_char(b, p)?;
    matches!(
        c,
        '\u{85}' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
    .then(|| c.len_utf8())
}

/// Where the spaces at `p` end.
fn skip_spaces(b: &[u8], mut p: usize) -> usize {
    while let Some(k) = space_len(b, p) {
        p += k;
    }
    p
}

/// `{{noise}}` at `p`, `(?:\s|/[*](?:[^*]|[*](?!/))*[*]/)*`: where it ends.
fn noise(b: &[u8], mut p: usize) -> usize {
    // most often none
    if b.get(p)
        .is_some_and(|&c| c < 0x80 && c != b'/' && !is_blank(c))
    {
        return p;
    }
    loop {
        p = skip_spaces(b, p);
        if b.get(p) == Some(&b'/')
            && b.get(p + 1) == Some(&b'*')
            && let Some(i) = b[p + 2..].windows(2).position(|w| w == b"*/")
        {
            p += 2 + i + 2;
            continue;
        }
        return p;
    }
}

/// A table of the bytes in `bytes`.
const fn stops(bytes: &[u8]) -> [bool; 256] {
    let mut t = [false; 256];
    let mut i = 0;
    while i < bytes.len() {
        t[bytes[i] as usize] = true;
        i += 1;
    }
    t
}

/// The bytes a pattern of a block comment, a string or a raw string starts with.
static COMMENT_STOPS: [bool; 256] = stops(b"*");
static STRING_STOPS: [bool; 256] = stops(b"\"{\\%");
static RAW_STOPS: [bool; 256] = stops(b"`{%");

/// The bytes a string's patterns start on, around a regex: `"…"`'s end and escapes, then
/// `` `…` ``'s end.
static RE_YIELDS: [[bool; 256]; 2] = [stops(b"\"\\"), stops(b"`")];

/// A regex in a `"…"` string, then in a `` `…` `` one.
static RE_EMBEDS: [regex::Embed<'static>; 2] = [
    regex::Embed {
        dialect: regex::Dialect::Re2,
        cooked: true,
        yields: &RE_YIELDS[0],
        stop: |b, p, _| b[p] == b'"',
        arg: 0,
        escape: |b, p| char_escape(b, p).unwrap_or(p + 1),
        top_only: false,
        continues: false,
        base: RE_STRING,
        text: DOUBLE,
    },
    regex::Embed {
        dialect: regex::Dialect::Re2,
        cooked: false,
        yields: &RE_YIELDS[1],
        stop: |b, p, _| b[p] == b'`',
        arg: 0,
        escape: |_, p| p + 1,
        top_only: false,
        continues: false,
        base: RE_RAW,
        text: OTHER,
    },
];

/// Whether the string whose quote opens at `p` is a regex: the first argument of
/// `regexp.MustCompile(`, `regexp.Compile(`, `regexp.MatchString(`, `regexp.Match(`,
/// `regexp.MatchReader(` or their `POSIX` twins, looked back on its line.
fn regex_string(b: &[u8], p: usize) -> bool {
    let mut q = p;
    while q > 0 && is_blank(b[q - 1]) {
        q -= 1;
    }
    if q == 0 || b[q - 1] != b'(' {
        return false;
    }
    let mut w = q - 1;
    while w > 0 && is_blank(b[w - 1]) {
        w -= 1;
    }
    // the name's last byte first: most calls end in none of these
    let names: &[&[u8]] = match w.checked_sub(1).map(|i| b[i]) {
        Some(b'e') => &[b"regexp.MustCompile", b"regexp.Compile"],
        Some(b'X') => &[b"regexp.MustCompilePOSIX", b"regexp.CompilePOSIX"],
        Some(b'g') => &[b"regexp.MatchString"],
        Some(b'h') => &[b"regexp.Match"],
        Some(b'r') => &[b"regexp.MatchReader"],
        _ => return false,
    };
    names
        .iter()
        .any(|name| b[..w].ends_with(name) && (w == name.len() || !is_word(b, w - name.len() - 1)))
}

/// The method `name` (after a `.`) whose arguments hold SQL, as GoLand reads them: the
/// [`PSQL`] state they start in. `database/sql`'s and pgx's `Exec`, `Query`, `QueryRow`,
/// `Prepare` (a context first, optionally) and their `…Context` forms, pgx's `Queue` and
/// `ExecParams`, sqlx's `MustExec`, `Queryx`, `NamedExec`, `Rebind`…, and `Get`, `Select` after
/// a destination.
fn sql_call(name: &[u8]) -> Option<u8> {
    // the last byte first: most calls end in none of these
    match *name.last()? {
        b'c' => matches!(name, b"Exec" | b"MustExec" | b"NamedExec").then_some(PSQL_LEAD),
        b'y' => matches!(name, b"Query" | b"NamedQuery").then_some(PSQL_LEAD),
        b'w' => (name == b"QueryRow").then_some(PSQL_LEAD),
        b'x' => matches!(name, b"Queryx" | b"QueryRowx" | b"Preparex").then_some(PSQL_LEAD),
        b'e' => match name {
            b"Queue" => Some(PSQL_LEAD),
            b"Prepare" => Some(PSQL_LEAD_NAME),
            _ => None,
        },
        b'd' => matches!(name, b"Rebind" | b"PrepareNamed").then_some(PSQL_LEAD),
        b's' => (name == b"ExecParams").then_some(PSQL_LEAD),
        b't' => match name {
            b"Get" | b"Select" => Some(PSQL_DEST),
            b"GetContext" | b"SelectContext" => Some(PSQL_CTX_DEST),
            b"ExecContext"
            | b"QueryContext"
            | b"QueryRowContext"
            | b"PrepareContext"
            | b"MustExecContext"
            | b"NamedExecContext"
            | b"NamedQueryContext"
            | b"QueryxContext"
            | b"QueryRowxContext"
            | b"PreparexContext"
            | b"PrepareNamedContext" => Some(PSQL_CTX),
            _ => None,
        },
        _ => None,
    }
}

/// Whether the method at `p` is on a receiver named as a prepared statement's (`stmt.Exec(`,
/// `insertStatement.Get(`, ASCII case ignored), whose arguments are the SQL's, not SQL.
fn stmt_receiver(b: &[u8], p: usize) -> bool {
    if p == 0 || b[p - 1] != b'.' {
        return false;
    }
    let e = p - 1;
    let mut s = e;
    while s > 0 && (b[s - 1].is_ascii_alphanumeric() || b[s - 1] == b'_') {
        s -= 1;
    }
    let r = b[s..e].to_ascii_lowercase();
    r.windows(4).any(|w| w == b"stmt") || r.windows(9).any(|w| w == b"statement")
}

/// Whether the argument at `p` names a context: a dotted name holding `ctx` or `context`
/// (ASCII case ignored), as `ctx`, `context.Background()`, `r.Context()`.
fn is_context(b: &[u8], p: usize) -> bool {
    let mut e = p;
    while e < b.len() && (b[e].is_ascii_alphanumeric() || matches!(b[e], b'_' | b'.')) {
        e += 1;
    }
    let w = b[p..e].to_ascii_lowercase();
    w.windows(3).any(|x| x == b"ctx") || w.windows(7).any(|x| x == b"context")
}

/// Where the string SQL is in ends on its line, from `p` inside it: its quote, or the line's
/// end.
fn string_end(b: &[u8], mut p: usize, raw: bool) -> usize {
    while p < b.len() {
        match b[p] {
            b'`' if raw => return p,
            b'"' if !raw => return p,
            b'\\' if !raw => p += 2,
            _ => p += 1,
        }
    }
    b.len()
}

/// What a whole word is among bat's keywords.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Keyword {
    Control,
    Range,
    Import,
    Package,
    Chan,
    Const,
    Func,
    Interface,
    Map,
    Struct,
    Type,
    Var,
}

/// `{{keyword}}`: the word `w`, if it is one.
fn keyword(w: &[u8]) -> Option<Keyword> {
    use Keyword::*;
    Some(match w {
        b"break" | b"case" | b"continue" | b"default" | b"defer" | b"else" | b"fallthrough"
        | b"for" | b"go" | b"goto" | b"if" | b"return" | b"select" | b"switch" => Control,
        b"range" => Range,
        b"import" => Import,
        b"package" => Package,
        b"chan" => Chan,
        b"const" => Const,
        b"func" => Func,
        b"interface" => Interface,
        b"map" => Map,
        b"struct" => Struct,
        b"type" => Type,
        b"var" => Var,
        _ => return None,
    })
}

/// `{{predeclared_type}}`.
fn is_predeclared_type(w: &[u8]) -> bool {
    matches!(
        w,
        b"bool"
            | b"byte"
            | b"complex64"
            | b"complex128"
            | b"error"
            | b"float32"
            | b"float64"
            | b"int"
            | b"int8"
            | b"int16"
            | b"int32"
            | b"int64"
            | b"rune"
            | b"string"
            | b"uint"
            | b"uint8"
            | b"uint16"
            | b"uint32"
            | b"uint64"
            | b"uintptr"
    )
}

/// `{{predeclared_func}}`.
fn is_predeclared_func(w: &[u8]) -> bool {
    matches!(
        w,
        b"append"
            | b"cap"
            | b"close"
            | b"complex"
            | b"copy"
            | b"delete"
            | b"imag"
            | b"len"
            | b"make"
            | b"new"
            | b"panic"
            | b"print"
            | b"println"
            | b"real"
            | b"recover"
    )
}

/// `\d` at `p`, Unicode's decimal digits: where it ends.
fn digit(b: &[u8], p: usize) -> Option<usize> {
    match *b.get(p)? {
        c if c.is_ascii_digit() => Some(p + 1),
        c if c < 0x80 => None,
        _ => wide_digit(b, p),
    }
}

#[cold]
fn wide_digit(b: &[u8], p: usize) -> Option<usize> {
    let c = wide_char(b, p)?;
    is_nd(c).then(|| p + c.len_utf8())
}

/// A run of `digit`s at `p`, with `_` between them: `d+(?:_d+)*` (and with `lead`,
/// `_?d+(?:_d+)*`). Where it ends.
fn digits(
    b: &[u8],
    p: usize,
    lead: bool,
    digit: impl Fn(&[u8], usize) -> Option<usize>,
) -> Option<usize> {
    let mut q = p;
    if lead && b.get(q) == Some(&b'_') && digit(b, q + 1).is_some() {
        q += 1;
    }
    q = digit(b, q)?;
    loop {
        while let Some(e) = digit(b, q) {
            q = e;
        }
        if b.get(q) == Some(&b'_')
            && let Some(e) = digit(b, q + 1)
        {
            q = e;
        } else {
            return Some(q);
        }
    }
}

/// An ASCII digit that `ok` takes at `p`: where it ends.
fn ascii(b: &[u8], p: usize, ok: fn(&u8) -> bool) -> Option<usize> {
    b.get(p).filter(|c| ok(c)).map(|_| p + 1)
}

fn dec_digits(b: &[u8], p: usize) -> Option<usize> {
    digits(b, p, false, digit)
}

fn hex_digits(b: &[u8], p: usize) -> Option<usize> {
    digits(b, p, true, |b, p| ascii(b, p, u8::is_ascii_hexdigit))
}

fn oct_digits(b: &[u8], p: usize) -> Option<usize> {
    digits(b, p, true, |b, p| ascii(b, p, |c| matches!(c, b'0'..=b'7')))
}

fn bin_digits(b: &[u8], p: usize) -> Option<usize> {
    digits(b, p, true, |b, p| ascii(b, p, |c| matches!(c, b'0' | b'1')))
}

/// `[eE][-+]??{{dec_digits}}` (or with `[pP]`) at `p`: where it ends.
fn exponent(b: &[u8], p: usize, marks: [u8; 2]) -> Option<usize> {
    if !b.get(p).is_some_and(|c| marks.contains(c)) {
        return None;
    }
    dec_digits(b, p + 1).or_else(|| {
        matches!(b.get(p + 1), Some(b'-' | b'+'))
            .then(|| dec_digits(b, p + 2))
            .flatten()
    })
}

/// `0[xX]` at `p` (or `[oO]`, `[bB]`).
fn base(b: &[u8], p: usize, mark: u8) -> bool {
    b.get(p) == Some(&b'0') && b.get(p + 1).is_some_and(|c| c.to_ascii_lowercase() == mark)
}

/// A number at `p`: its end, its leaf, and its captures as `(start, end, leaf)`, in order.
type Number = (usize, u16, [(usize, usize, u16); 3]);

/// `match-imaginary`, `match-floats` and `match-integers` at `p`, in their order.
fn number(b: &[u8], p: usize) -> Option<Number> {
    const NONE: (usize, usize, u16) = (0, 0, PLAIN);
    let is = |q: usize, c: u8| b.get(q) == Some(&c);
    // `{{dec_digits}}(?:(\.){{dec_digits}}?)?{{dec_exponent}}?(i)`
    if let Some(d) = dec_digits(b, p) {
        let (f, dot) = if is(d, b'.') {
            (dec_digits(b, d + 1).unwrap_or(d + 1), Some(d))
        } else {
            (d, None)
        };
        let x = exponent(b, f, *b"eE").unwrap_or(f);
        if is(x, b'i') {
            let dot = dot.map_or(NONE, |d| (d, d + 1, IMAG_DEC + 1));
            return Some((x + 1, IMAG_DEC, [dot, (x, x + 1, IMAG_DEC + 2), NONE]));
        }
    }
    let hex = base(b, p, b'x');
    // `(0[xX]){{hex_digits}}?(?:(\.){{hex_digits}}?)?{{hex_exponent}}?(i)`, and the float
    // without the `i`, its exponent required
    if hex {
        let h = hex_digits(b, p + 2).unwrap_or(p + 2);
        let (f, dot) = if is(h, b'.') {
            (hex_digits(b, h + 1).unwrap_or(h + 1), Some(h))
        } else {
            (h, None)
        };
        let x = exponent(b, f, *b"pP");
        let e = x.unwrap_or(f);
        let base = (p, p + 2, IMAG_HEX + 1);
        if is(e, b'i') {
            let dot = dot.map_or(NONE, |d| (d, d + 1, IMAG_HEX + 2));
            return Some((e + 1, IMAG_HEX, [base, dot, (e, e + 1, IMAG_HEX + 3)]));
        }
    }
    // `(0[oO]){{oct_digits}}(i)`, `(0[bB]){{bin_digits}}(i)`
    for (mark, leaf, run) in [
        (
            b'o',
            IMAG_OCT,
            oct_digits as fn(&[u8], usize) -> Option<usize>,
        ),
        (b'b', IMAG_BIN, bin_digits),
    ] {
        if base(b, p, mark)
            && let Some(e) = run(b, p + 2)
            && is(e, b'i')
        {
            return Some((
                e + 1,
                leaf,
                [(p, p + 2, leaf + 1), (e, e + 1, leaf + 2), NONE],
            ));
        }
    }
    // decimal floats: `{{dec_digits}}(?:(\.){{dec_digits}}?{{dec_exponent}}?|{{dec_exponent}})`,
    // `(\.){{dec_digits}}{{dec_exponent}}?`
    if let Some(d) = dec_digits(b, p) {
        if is(d, b'.') {
            let f = dec_digits(b, d + 1).unwrap_or(d + 1);
            let x = exponent(b, f, *b"eE").unwrap_or(f);
            return Some((x, FLOAT_DEC, [(d, d + 1, FLOAT_DEC + 1), NONE, NONE]));
        }
        if let Some(x) = exponent(b, d, *b"eE") {
            return Some((x, FLOAT_DEC, [NONE; 3]));
        }
    } else if is(p, b'.')
        && let Some(f) = dec_digits(b, p + 1)
    {
        let x = exponent(b, f, *b"eE").unwrap_or(f);
        return Some((x, FLOAT_DEC, [(p, p + 1, FLOAT_DEC + 1), NONE, NONE]));
    }
    // `(0[xX]){{hex_digits}}?(?:(\.){{hex_digits}}?)?{{hex_exponent}}`
    if hex {
        let h = hex_digits(b, p + 2).unwrap_or(p + 2);
        let (f, dot) = if is(h, b'.') {
            (hex_digits(b, h + 1).unwrap_or(h + 1), Some(h))
        } else {
            (h, None)
        };
        if let Some(x) = exponent(b, f, *b"pP") {
            let dot = dot.map_or(NONE, |d| (d, d + 1, FLOAT_HEX + 2));
            return Some((x, FLOAT_HEX, [(p, p + 2, FLOAT_HEX + 1), dot, NONE]));
        }
    }
    // `(0){{oct_digits}}(?=\D)`, backtracking to a `_` where the run goes on in a digit
    if is(p, b'0')
        && let Some(e) = oct_digits(b, p + 1)
    {
        let not_digit = |q: usize| digit(b, q).is_none();
        let e = if not_digit(e) {
            Some(e)
        } else {
            (p + 2..e).rev().find(|&q| b[q] == b'_' && b[q - 1] != b'_')
        };
        if let Some(e) = e {
            return Some((e, INT_OCT, [(p, p + 1, INT_OCT + 1), NONE, NONE]));
        }
    }
    // `0[0-7]*[8-9]+`
    if is(p, b'0') {
        let o = (p + 1..=b.len())
            .find(|&q| !matches!(b.get(q), Some(b'0'..=b'7')))
            .unwrap_or(b.len());
        if matches!(b.get(o), Some(b'8' | b'9')) {
            let e = (o..=b.len())
                .find(|&q| !matches!(b.get(q), Some(b'8' | b'9')))
                .unwrap_or(b.len());
            return Some((e, INVALID, [NONE; 3]));
        }
    }
    // `(0[oO]){{oct_digits}}`, `(0[xX]){{hex_digits}}`, `(0[bB]){{bin_digits}}`
    for (mark, leaf, run) in [
        (
            b'o',
            INT_OCT,
            oct_digits as fn(&[u8], usize) -> Option<usize>,
        ),
        (b'x', INT_HEX, hex_digits),
        (b'b', INT_BIN, bin_digits),
    ] {
        if base(b, p, mark)
            && let Some(e) = run(b, p + 2)
        {
            return Some((e, leaf, [(p, p + 2, leaf + 1), NONE, NONE]));
        }
    }
    dec_digits(b, p).map(|e| (e, INT_DEC, [NONE; 3]))
}

/// `{{char_escape}}` at `p`, a `\`: `\\x\h{2}|\\u\h{4}|\\U\h{8}|\\[0-7]{3}|\\.`, where it
/// ends.
fn char_escape(b: &[u8], p: usize) -> Option<usize> {
    let run = |from: usize, n: usize, ok: fn(&u8) -> bool| {
        (b.len() >= from + n && b[from..from + n].iter().all(ok)).then_some(from + n)
    };
    match *b.get(p + 1)? {
        b'x' if let Some(e) = run(p + 2, 2, u8::is_ascii_hexdigit) => Some(e),
        b'u' if let Some(e) = run(p + 2, 4, u8::is_ascii_hexdigit) => Some(e),
        b'U' if let Some(e) = run(p + 2, 8, u8::is_ascii_hexdigit) => Some(e),
        b'0'..=b'7' if let Some(e) = run(p + 1, 3, |c| matches!(c, b'0'..=b'7')) => Some(e),
        c => Some((p + 1 + char_len(c)).min(b.len())),
    }
}

/// `\%(?:\[\d+\])?[ .\d#+-]*[A-Za-z]` at `p`, a `%`: where it ends.
fn fmt_verb(b: &[u8], p: usize) -> Option<usize> {
    let flags = |mut q: usize| {
        loop {
            if matches!(b.get(q), Some(b' ' | b'.' | b'#' | b'+' | b'-')) {
                q += 1;
            } else if let Some(e) = digit(b, q) {
                q = e;
            } else {
                break;
            }
        }
        b.get(q)
            .is_some_and(u8::is_ascii_alphabetic)
            .then_some(q + 1)
    };
    if b.get(p + 1) == Some(&b'[')
        && let Some(d) = dec_digits_plain(b, p + 2)
        && b.get(d) == Some(&b']')
        && let Some(e) = flags(d + 1)
    {
        return Some(e);
    }
    flags(p + 1)
}

/// `\d+` at `p`.
fn dec_digits_plain(b: &[u8], p: usize) -> Option<usize> {
    let mut e = digit(b, p)?;
    while let Some(d) = digit(b, e) {
        e = d;
    }
    Some(e)
}

struct Scan<'b, 'c> {
    b: &'b [u8],
    /// The line's length: where its `\n` is.
    n: usize,
    stack: [u8; MAX],
    depth: usize,
    exact: bool,
    /// The kind of text no pattern of the top context matches.
    text: u16,
    /// Where the spans so far end.
    at: usize,
    /// Where the last push that matched nothing was, and the depth it left: a pop there, at
    /// that depth, that matches nothing would loop, so syntect tries the patterns after it and
    /// else steps a character.
    loop_at: (usize, usize),
    /// A pop at the place was passed over (`loop_at`).
    looped: bool,
    /// Matches that took nothing, in a row at one place (reset when the scan moves).
    stuck: usize,
    /// The last place [`Scan::word`] looked at, and what it found.
    wcache: std::cell::Cell<(usize, Option<usize>)>,
    /// Where the `(` of a SQL call is ([`sql_call`]), and the state its arguments start in;
    /// `usize::MAX` for none on the line.
    sql_paren: usize,
    sql_call: u8,
    /// What SQL's scan keeps between the host's turns on a line.
    sql: sql::Embedded,
    /// Where the SQL string open on the line ends (its quote, or the line's end), past which no
    /// SQL token runs; `usize::MAX` till it is looked for.
    sql_end: usize,
    /// Whether SQL's scan took the line's end (`\s` matching its `\n`).
    sql_nl: bool,
    /// A SQL marker's: the next string that opens at or past it is SQL, unless a `;`, `,`, `)`,
    /// `{` or `}` came between ([`sql::marker_holds`], checked there and at the line's end);
    /// `usize::MAX` for none.
    mark: usize,
    out: &'b mut Out<'c>,
}

impl Scan<'_, '_> {
    fn top(&self) -> u8 {
        self.stack[self.depth - 1]
    }

    fn refresh(&mut self) {
        self.text = match self.top() {
            BLOCK_COMMENT => COMMENT,
            STRING => DOUBLE,
            RAW => OTHER,
            INTERP => INTERP_TEXT,
            // a regex's is set where one is pushed, and where a line starts in one
            _ => PLAIN,
        };
    }

    /// The text of the string a regex or SQL is in.
    #[cold]
    fn string_text(&self) -> u16 {
        if self.depth >= 2 && self.stack[self.depth - 2] == RAW {
            OTHER
        } else {
            DOUBLE
        }
    }

    /// The text up to `p` is the top context's.
    fn fill(&mut self, p: usize) {
        let p = p.min(self.n);
        if p > self.at {
            if self.text != PLAIN {
                self.out.push_merged(self.at, p, self.text);
            }
            self.at = p;
        }
    }

    /// Bytes `s..e` are `leaf`, after the text before them.
    fn put(&mut self, s: usize, e: usize, leaf: u16) {
        self.fill(s);
        let e = e.min(self.n);
        if leaf != PLAIN && e > self.at {
            self.out.push_merged(self.at, e, leaf);
        }
        self.at = self.at.max(e);
    }

    fn push(&mut self, c: u8) {
        if self.depth == MAX {
            self.stack.copy_within(2.., 1);
            self.depth -= 1;
            self.exact = false;
        }
        self.stack[self.depth] = c;
        self.depth += 1;
        self.refresh();
    }

    fn pop(&mut self) {
        if self.depth > 1 {
            self.depth -= 1;
        }
        self.refresh();
    }

    /// Replace the top with `cs`, bottom first.
    fn set(&mut self, cs: &[u8]) {
        self.pop();
        for &c in cs {
            self.push(c);
        }
    }

    /// `cs` pushed by a match that took nothing at `p`: syntect keeps the depth the first of
    /// them leaves for its guard against a loop.
    fn enter(&mut self, p: usize, cs: &[u8]) -> Option<usize> {
        if self.depth >= SYNTECT_DEPTH {
            return None;
        }
        self.fill(p);
        let depth = self.depth + 1;
        for &c in cs {
            self.push(c);
        }
        self.loop_at = (p, depth);
        Some(p)
    }

    /// The top replaced with `cs` by a match that took nothing at `p`.
    fn shift(&mut self, p: usize, cs: &[u8]) -> Option<usize> {
        self.fill(p);
        self.set(cs);
        Some(p)
    }

    /// A pop that takes nothing at `p`, unless it would loop: then syntect passes over it for
    /// the patterns after it (`None`).
    fn leave(&mut self, p: usize) -> Option<usize> {
        if self.loop_at == (p, self.depth) {
            self.looped = true;
            return None;
        }
        self.fill(p);
        self.pop();
        Some(p)
    }

    /// Bytes `s..e` are `leaf`; then `cs` are pushed.
    fn tok_push(&mut self, s: usize, e: usize, leaf: u16, cs: &[u8]) -> Option<usize> {
        if self.depth >= SYNTECT_DEPTH {
            return None;
        }
        self.fill(s);
        for &c in cs {
            self.push(c);
        }
        self.put(s, e, leaf);
        Some(e)
    }

    /// Bytes `s..e` are `leaf`, then the top context pops.
    fn close(&mut self, s: usize, e: usize, leaf: u16) -> Option<usize> {
        self.put(s, e, leaf);
        self.pop();
        Some(e)
    }

    /// Bytes `s..e` are `leaf`, then the top context is replaced with `cs`.
    fn then(&mut self, s: usize, e: usize, leaf: u16, cs: &[u8]) -> Option<usize> {
        self.put(s, e, leaf);
        self.set(cs);
        Some(e)
    }

    /// Bytes `s..e` are `leaf`, and the context stays.
    fn tok(&mut self, s: usize, e: usize, leaf: u16) -> Option<usize> {
        self.put(s, e, leaf);
        Some(e)
    }

    fn run(&mut self) {
        let end = self.n + 1;
        let mut p = 0;
        loop {
            // where no pattern of the context starts at a space, the spaces are skipped whole
            if p < self.n && is_blank(self.b[p]) && spaces_inert(self.top()) {
                p = skip_spaces(self.b, p);
            }
            self.looped = false;
            if let Some(q) = self.at(p) {
                self.stuck = if q == p { self.stuck + 1 } else { 0 };
                p = q;
                if self.stuck > STUCK {
                    self.stuck = 0;
                    if p >= self.n {
                        return;
                    }
                    p = self.next(p);
                }
            } else if p >= end || (self.looped && p >= self.n) {
                return;
            } else if self.looped {
                // syntect's guard: a step of a character, and a new search from there
                p = self.next(p);
                self.stuck = 0;
            } else {
                p = self.advance(p);
                self.stuck = 0;
            }
        }
    }

    /// Where the character at `p` ends (the line's `\n` is one byte).
    fn next(&self, p: usize) -> usize {
        if p >= self.n {
            return p + 1;
        }
        p + char_len(self.b[p]).min(self.n - p)
    }

    /// Where a pattern of the top context can next start, after none did at `p`.
    fn advance(&self, p: usize) -> usize {
        if p >= self.n {
            return p + 1;
        }
        let b = self.b;
        let stops = match self.top() {
            BLOCK_COMMENT => &COMMENT_STOPS,
            STRING => &STRING_STOPS,
            RAW => &RAW_STOPS,
            // `(?=[^(])`, `\s-`, and a named parameter's `(?:{{noise}}…)*` match at a space
            METHOD | INTERP | PARAM_OPEN => return self.next(p),
            _ => {
                if b[p] < 0x80 && space_len(b, p).is_some() {
                    return skip_spaces(b, p);
                }
                return self.next(p);
            }
        };
        let mut q = p + 1;
        while q < self.n && !stops[usize::from(b[q])] {
            q += 1;
        }
        q
    }

    /// The first pattern of the top context that matches at `p`, applied: where the scan goes
    /// on, or `None` when none does.
    fn at(&mut self, p: usize) -> Option<usize> {
        let top = self.top();
        match top {
            MAIN => self.any(p),
            BLOCK => self.closing(p, b'}', BRACES_END).or_else(|| self.any(p)),
            PARENS => self.parens(p),
            AFTER_BRACKET => self
                .terminator(p)
                .or_else(|| self.comments(p))
                .or_else(|| self.star(p))
                .or_else(|| self.qualifier(p))
                .or_else(|| self.blank_pop(p))
                .or_else(|| self.type_name(p))
                .or_else(|| self.nonblank_pop(p)),
            SELECTOR => self
                .comments(p)
                .or_else(|| {
                    self.byte(p, b'(')
                        .then(|| self.then(p, p + 1, PARENS_BEGIN, &[ASSERTION]))
                        .flatten()
                })
                .or_else(|| {
                    let e = self.ident(p)?;
                    if !self.call_follows(e) {
                        return None;
                    }
                    // `sql_call`'s names start uppercase and end in one of these bytes
                    if self.b[p].is_ascii_uppercase()
                        && matches!(
                            self.b[e - 1],
                            b'c' | b'y' | b'w' | b'x' | b'e' | b'd' | b's' | b't'
                        )
                    {
                        self.sql_name(p, e);
                    }
                    self.close(p, e, FUNCTION)
                })
                .or_else(|| self.blank_pop(p))
                .or_else(|| {
                    let e = self.ident(p)?;
                    self.close(p, e, MEMBER)
                })
                .or_else(|| self.nonblank_pop(p)),
            ASSERTION | TYPE_PARENS => self.closing(p, b')', PARENS_END).or_else(|| self.types(p)),
            SHORT_VAR => self
                .comments(p)
                .or_else(|| self.blank(p).then(|| self.tok(p, p + 1, BLANK)).flatten())
                .or_else(|| {
                    let e = self.ident(p)?;
                    self.tok(p, e, DECLARATION)
                })
                .or_else(|| self.comma(p))
                .or_else(|| self.nonblank_pop(p)),
            MAKE_ARGS => self
                .comments(p)
                .or_else(|| {
                    self.byte(p, b')')
                        .then(|| self.tok(p, p + 1, PARENS_END))
                        .flatten()
                })
                .or_else(|| {
                    self.byte(p, b'(')
                        .then(|| self.then(p, p + 1, PARENS_BEGIN, &[MAKE_FIRST]))
                        .flatten()
                })
                .or_else(|| self.terminator(p))
                .or_else(|| self.nonblank_pop(p)),
            MAKE_FIRST => self
                .closing(p, b')', PARENS_END)
                .or_else(|| self.comments(p))
                .or_else(|| self.nonblank_set(p, &[TYPE])),
            TYPE => self.types(p),
            TYPE_BRACKETS => {
                if self.byte(p, b']') {
                    return self.then(p, p + 1, BRACKETS_END, &[TYPE]);
                }
                self.any(p)
            }
            NAMED_TYPE => self
                .comments(p)
                .or_else(|| self.blank_pop(p))
                .or_else(|| self.qualifier(p))
                .or_else(|| {
                    self.byte(p, b'.')
                        .then(|| self.tok(p, p + 1, DOT))
                        .flatten()
                })
                .or_else(|| self.type_name(p))
                .or_else(|| self.nonblank_pop(p)),
            IFACE_HEAD | STRUCT_HEAD => self
                .comments(p)
                .or_else(|| {
                    let body = if top == IFACE_HEAD {
                        IFACE_BODY
                    } else {
                        STRUCT_BODY
                    };
                    self.byte(p, b'{')
                        .then(|| self.then(p, p + 1, BRACES_BEGIN, &[body]))
                        .flatten()
                })
                .or_else(|| self.nonblank_pop(p)),
            IFACE_BODY | STRUCT_BODY => self.type_body(p, top == STRUCT_BODY),
            IFACE_METHOD => {
                if self.byte(p, b'}')
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.params_returns(p)
            }
            MAP_HEAD => self
                .comments(p)
                .or_else(|| self.semicolon(p))
                .or_else(|| {
                    self.byte(p, b'[')
                        .then(|| self.then(p, p + 1, BRACKETS_BEGIN, &[MAP_KEY]))
                        .flatten()
                })
                .or_else(|| self.types(p)),
            MAP_KEY => {
                if self.byte(p, b']') {
                    return self.then(p, p + 1, BRACKETS_END, &[MAP_VALUE]);
                }
                self.comments(p).or_else(|| {
                    self.nonblank(p)
                        .then(|| self.enter(p, &[MAP_KEY_TYPE]))
                        .flatten()
                })
            }
            MAP_KEY_TYPE => {
                if self.byte(p, b']')
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.types(p)
            }
            MAP_VALUE => self.terminator(p).or_else(|| self.types(p)),
            STRUCT_FIELD => {
                if self.byte(p, b'}')
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.terminator(p)
                    .or_else(|| self.comments(p))
                    .or_else(|| self.types(p))
                    .or_else(|| self.any(p))
            }
            DECL_HEAD..=34 => self.declaration(p, top),
            TYPE_HEAD => self
                .comments(p)
                .or_else(|| {
                    self.byte(p, b'(')
                        .then(|| self.then(p, p + 1, PARENS_BEGIN, &[TYPE_GROUP]))
                        .flatten()
                })
                .or_else(|| {
                    self.blank(p)
                        .then(|| self.then(p, p + 1, BLANK, &[ALIAS]))
                        .flatten()
                })
                .or_else(|| {
                    let e = self.ident(p)?;
                    self.then(p, e, TYPE_NAME, &[ALIAS])
                })
                .or_else(|| self.nonblank_pop(p)),
            TYPE_GROUP => self
                .closing(p, b')', PARENS_END)
                .or_else(|| {
                    self.blank(p)
                        .then(|| self.tok_push(p, p + 1, BLANK, &[TYPE_ITEM]))
                        .flatten()
                })
                .or_else(|| {
                    let e = self.ident(p)?;
                    self.tok_push(p, e, TYPE_NAME, &[TYPE_ITEM])
                })
                .or_else(|| self.any(p)),
            TYPE_ITEM => {
                if self.byte(p, b')')
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.alias(p)
            }
            ALIAS => self.alias(p),
            FUNC_HEAD => self
                .comments(p)
                .or_else(|| {
                    self.method_follows(p)
                        .then(|| self.shift(p, &[METHOD]))
                        .flatten()
                })
                .or_else(|| {
                    let e = self.ident(p)?;
                    let q = noise(self.b, e);
                    self.byte(q, b'(')
                        .then(|| self.shift(p, &[SIGNATURE]))
                        .flatten()
                })
                .or_else(|| self.params_returns(p)),
            METHOD => {
                if p <= self.n && !self.byte(p, b'(') {
                    return self.shift(p, &[SIGNATURE]);
                }
                self.byte(p, b'(')
                    .then(|| self.enter(p, &[PARAM_LIST]))
                    .flatten()
            }
            SIGNATURE => self
                .comments(p)
                .or_else(|| {
                    self.blank(p)
                        .then(|| self.then(p, p + 1, BLANK, &[PARAMS_RETURNS]))
                        .flatten()
                })
                .or_else(|| {
                    let e = self.ident(p)?;
                    self.then(p, e, FUNCTION_NAME, &[PARAMS_RETURNS])
                })
                .or_else(|| self.nonblank_pop(p)),
            PARAMS_RETURNS => self.params_returns(p),
            RETURNS => self
                .terminator(p)
                .or_else(|| self.comments(p))
                .or_else(|| {
                    self.byte(p, b'(')
                        .then(|| self.shift(p, &[PARAM_LIST]))
                        .flatten()
                })
                .or_else(|| self.nonblank_set(p, &[TYPE])),
            PARAM_LIST => self
                .comments(p)
                .or_else(|| {
                    self.byte(p, b'(')
                        .then(|| self.then(p, p + 1, PARENS_BEGIN, &[PARAM_OPEN]))
                        .flatten()
                })
                .or_else(|| self.nonblank_pop(p)),
            PARAM_OPEN => self
                .comments(p)
                .or_else(|| {
                    self.named_params_follow(p)
                        .then(|| self.shift(p, &[NAMED_PARAMS]))
                        .flatten()
                })
                .or_else(|| self.nonblank_set(p, &[UNNAMED_PARAMS])),
            NAMED_PARAMS | UNNAMED_PARAMS => {
                let r = self
                    .closing(p, b')', PARENS_END)
                    .or_else(|| self.comments(p))
                    .or_else(|| self.keywords(p))
                    .or_else(|| self.comma(p))
                    .or_else(|| self.ellipsis(p));
                if r.is_some() {
                    return r;
                }
                if top == UNNAMED_PARAMS {
                    return self.nonblank(p).then(|| self.enter(p, &[TYPE])).flatten();
                }
                if self.blank(p) {
                    return self.tok_push(p, p + 1, BLANK, &[PARAM_TYPE]);
                }
                let e = self.ident(p)?;
                self.tok_push(p, e, PARAMETER, &[PARAM_TYPE])
            }
            PARAM_TYPE => {
                if (self.byte(p, b')') || self.byte(p, b','))
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.ellipsis(p).or_else(|| self.types(p))
            }
            BLOCK_COMMENT => {
                let b = self.b;
                if b[p.min(self.n)..].starts_with(b"*/") {
                    return self.close(p, p + 2, COMMENT_END);
                }
                if p == 0 {
                    // `^\s*(\*)(?!/)`
                    let q = skip_spaces(b, 0);
                    if self.byte(q, b'*') && !self.byte(q + 1, b'/') {
                        self.put(0, q, COMMENT);
                        return self.tok(q, q + 1, COMMENT_STAR);
                    }
                }
                None
            }
            STRING | RAW => {
                let (quote, text) = if top == STRING {
                    (b'"', DOUBLE)
                } else {
                    (b'`', OTHER)
                };
                if self.byte(p, quote) {
                    return self.close(p, p + 1, text + END);
                }
                if let Some(q) = self.template(p) {
                    return Some(q);
                }
                if top == STRING
                    && self.byte(p, b'\\')
                    && let Some(e) = char_escape(self.b, p)
                {
                    return self.tok(p, e, text + ESCAPE);
                }
                if !self.byte(p, b'%') {
                    return None;
                }
                if self.byte(p + 1, b'%') {
                    return self.tok(p, p + 2, text + ESCAPE);
                }
                let e = fmt_verb(self.b, p)?;
                self.tok(p, e, text + PLACEHOLDER)
            }
            INTERP => self.interpolation(p),
            // the rare contexts of phase 4, out of the hot `match`
            _ => self.rare(p, top),
        }
    }

    /// The contexts of phase 4 at `p`: a SQL call's arguments, SQL or a regex in a string.
    #[cold]
    #[inline(never)]
    fn rare(&mut self, p: usize, top: u8) -> Option<usize> {
        match top {
            PSQL..=PSQL_LAST => self.sql_parens(p, top),
            SQL_IN..=SQL_LAST => self.in_sql(p, top),
            _ => self.in_regex(p, top),
        }
    }

    /// In a regex in a string, at `p`: the string's end and (in `"…"`) its escapes but `\\`,
    /// which is the regex's, else the regex to the next place they can start.
    #[cold]
    #[inline(never)]
    fn in_regex(&mut self, p: usize, top: u8) -> Option<usize> {
        if top < REGEX || p >= self.n {
            return None;
        }
        let raw = self.string_text() == OTHER;
        let text = if raw { OTHER } else { DOUBLE };
        let b = self.b;
        match b[p] {
            b'`' if raw => {
                self.pop();
                return self.close(p, p + 1, text + END);
            }
            b'"' if !raw => {
                self.pop();
                return self.close(p, p + 1, text + END);
            }
            b'\\'
                if !raw
                    && b.get(p + 1) != Some(&b'\\')
                    && let Some(e) = char_escape(b, p) =>
            {
                return self.tok(p, e, text + ESCAPE);
            }
            _ => {}
        }
        let (q, st) = regex::embedded(
            b,
            p,
            top & regex::STATE,
            &RE_EMBEDS[usize::from(raw)],
            &mut self.at,
            self.out,
        )?;
        self.stack[self.depth - 1] = REGEX | st;
        Some(q)
    }

    /// A string at `p`, `"` or `` ` ``, with a regex in it where a call makes it one.
    fn string(&mut self, p: usize, raw: bool) -> Option<usize> {
        let (leaf, c) = if raw { (OTHER, RAW) } else { (DOUBLE, STRING) };
        let e = self.tok_push(p, p + 1, leaf + BEGIN, &[c])?;
        self.regex_in(p);
        Some(e)
    }

    /// A string opened at `p`: SQL in it after a marker, else a regex in it where a call makes
    /// it one.
    // out of line, so that `literals` keeps its inlined paths
    #[inline(never)]
    fn regex_in(&mut self, p: usize) {
        if self.depth >= SYNTECT_DEPTH {
            return;
        }
        if self.mark <= p && self.marked(p) {
            self.sql_in();
        } else if regex_string(self.b, p) {
            self.push(REGEX);
            self.text = self.string_text();
        }
    }

    /// `match-parens`' patterns at `p`.
    #[inline(always)]
    fn parens(&mut self, p: usize) -> Option<usize> {
        self.closing(p, b')', PARENS_END)
            .or_else(|| self.call_or_cast(p, true))
            .or_else(|| self.any(p))
    }

    /// A method's name `b[p..e]` before its call's `(`: that `(` opens a SQL call's arguments
    /// if the name is one of [`sql_call`]'s.
    #[inline(never)]
    fn sql_name(&mut self, p: usize, e: usize) {
        if let Some(st) = sql_call(&self.b[p..e])
            && !stmt_receiver(self.b, p)
        {
            let q = noise(self.b, e);
            if self.byte(q, b'(') {
                self.sql_paren = q;
                self.sql_call = st;
            }
        }
    }

    /// A SQL call's arguments at `p`: [`PARENS`]' patterns, with the state followed at each
    /// argument's start and its `,`, and the SQL argument's string opened as SQL. An argument
    /// that doesn't fit the call's shape makes the context [`PARENS`].
    #[cold]
    #[inline(never)]
    fn sql_parens(&mut self, p: usize, top: u8) -> Option<usize> {
        let b = self.b;
        if p < self.n
            && b[p] != b')'
            && !(b[p] == b'/' && matches!(b.get(p + 1), Some(b'/' | b'*')))
        {
            let c = b[p];
            let quote = matches!(c, b'"' | b'`');
            let st = top - PSQL;
            let next = if c == b',' {
                match st {
                    PSQL_SKIP_SQL => PSQL + PSQL_SQL,
                    PSQL_SKIP_DEST => PSQL + PSQL_DEST,
                    PSQL_SKIP_NAME => PSQL + PSQL_NAME,
                    _ => PARENS,
                }
            } else {
                match st {
                    PSQL_LEAD | PSQL_LEAD_NAME | PSQL_SQL if quote => {
                        self.stack[self.depth - 1] = PSQL + PSQL_AFTER;
                        if let Some(e) = self.sql_string(p, c == b'`') {
                            return Some(e);
                        }
                        PSQL + PSQL_AFTER
                    }
                    PSQL_LEAD if is_context(b, p) => PSQL + PSQL_SKIP_SQL,
                    PSQL_LEAD_NAME if is_context(b, p) => PSQL + PSQL_SKIP_NAME,
                    PSQL_CTX if !quote => PSQL + PSQL_SKIP_SQL,
                    PSQL_CTX_DEST if !quote => PSQL + PSQL_SKIP_DEST,
                    PSQL_DEST if c == b'&' => PSQL + PSQL_SKIP_SQL,
                    PSQL_NAME => PSQL + PSQL_SKIP_SQL,
                    PSQL_AFTER if c == b'+' => PSQL + PSQL_SQL,
                    PSQL_SKIP_SQL | PSQL_SKIP_DEST | PSQL_SKIP_NAME => top,
                    _ => PARENS,
                }
            };
            self.stack[self.depth - 1] = next;
        }
        self.parens(p)
    }

    /// A string at `p`, `"` or `` ` ``, whose text is SQL.
    fn sql_string(&mut self, p: usize, raw: bool) -> Option<usize> {
        let (leaf, c) = if raw { (OTHER, RAW) } else { (DOUBLE, STRING) };
        let e = self.tok_push(p, p + 1, leaf + BEGIN, &[c])?;
        if self.depth < SYNTECT_DEPTH {
            self.sql_in();
        }
        Some(e)
    }

    /// SQL in the string just opened. It takes a marker's turn.
    fn sql_in(&mut self) {
        self.mark = usize::MAX;
        self.sql_end = usize::MAX;
        self.sql.is_end = usize::MAX;
        self.push(SQL_IN + sql::MAIN);
        self.text = self.string_text();
    }

    /// Whether the marker before `p` still holds there ([`sql::marker_holds`]); it's dropped
    /// if not.
    #[cold]
    fn marked(&mut self, p: usize) -> bool {
        let holds = sql::marker_holds(self.b, self.mark, p);
        if !holds {
            self.mark = usize::MAX;
        }
        holds
    }

    /// In SQL in a string, at `p`: the string's end and (in `"…"`) its escapes, else SQL's
    /// scanner, embedded, to the next place they can start ([`sql::embedded`]). At the line's
    /// end, SQL's patterns may take it as `\s`, and its line comment ends there.
    #[cold]
    #[inline(never)]
    fn in_sql(&mut self, p: usize, top: u8) -> Option<usize> {
        let raw = self.string_text() == OTHER;
        if p >= self.n {
            if p == self.n && !self.sql_nl {
                self.sql_scan(p, top);
                self.sql_nl = true;
            }
            if matches!(self.top().wrapping_sub(SQL_IN), sql::DASHES | sql::HASHES) {
                let d = self.depth - 1;
                self.stack[d] = SQL_IN + sql::MAIN;
            }
            return None;
        }
        let text = if raw { OTHER } else { DOUBLE };
        let b = self.b;
        match b[p] {
            b'`' if raw => {
                self.pop();
                return self.close(p, p + 1, text + END);
            }
            b'"' if !raw => {
                self.pop();
                return self.close(p, p + 1, text + END);
            }
            b'\\' if !raw => {
                if let Some(e) = char_escape(b, p) {
                    return self.tok(p, e, text + ESCAPE);
                }
            }
            _ => {}
        }
        self.sql_scan(p, top).map(|(q, _)| q)
    }

    /// SQL's scan from `p` in the string below `top`: where it stopped, its context there set
    /// on the stack. No SQL token runs past the string's end.
    fn sql_scan(&mut self, p: usize, top: u8) -> Option<(usize, u8)> {
        let raw = self.string_text() == OTHER;
        let host = sql::Embed {
            yields: &RE_YIELDS[usize::from(raw)],
            base: if raw { SQL_RAW } else { SQL_STRING },
            text: if raw { OTHER } else { DOUBLE },
        };
        let b = self.b;
        if self.sql_end == usize::MAX {
            self.sql_end = string_end(b, p, raw);
        }
        let end = self.sql_end;
        self.fill(p);
        self.sql.at = self.at;
        let r = sql::embedded(&b[..end], p, top - SQL_IN, &host, &mut self.sql, self.out);
        self.at = self.sql.at;
        let (mut q, ctx) = r?;
        if q > end {
            self.sql_nl |= end == b.len();
            q = end;
        }
        let d = self.depth - 1;
        self.stack[d] = SQL_IN + ctx;
        Some((q, ctx))
    }

    /// Whether byte `p` is `c` (on the line, not its `\n`).
    fn byte(&self, p: usize, c: u8) -> bool {
        p < self.n && self.b[p] == c
    }

    /// `\S` at `p` (the line's `\n` is a space).
    fn nonblank(&self, p: usize) -> bool {
        p < self.n && space_len(self.b, p).is_none()
    }

    /// `pop-before-nonblank`: `(?=\S)`, a pop.
    fn nonblank_pop(&mut self, p: usize) -> Option<usize> {
        if self.nonblank(p) {
            self.leave(p)
        } else {
            None
        }
    }

    /// `(?=\S)`, set to `cs`.
    fn nonblank_set(&mut self, p: usize, cs: &[u8]) -> Option<usize> {
        if self.nonblank(p) {
            self.shift(p, cs)
        } else {
            None
        }
    }

    /// `c` at `p` closes the context, as `leaf`.
    #[inline(always)]
    fn closing(&mut self, p: usize, c: u8, leaf: u16) -> Option<usize> {
        if self.byte(p, c) {
            self.close(p, p + 1, leaf)
        } else {
            None
        }
    }

    /// `pop-on-semicolon`.
    fn semicolon(&mut self, p: usize) -> Option<usize> {
        self.closing(p, b';', TERMINATOR)
    }

    /// `pop-on-terminator`: `;`, or the line's end.
    fn terminator(&mut self, p: usize) -> Option<usize> {
        if p >= self.n {
            return self.leave(p);
        }
        self.semicolon(p)
    }

    fn comma(&mut self, p: usize) -> Option<usize> {
        if self.byte(p, b',') {
            self.tok(p, p + 1, SEPARATOR)
        } else {
            None
        }
    }

    fn ellipsis(&mut self, p: usize) -> Option<usize> {
        if p < self.n && self.b[p..].starts_with(b"...") {
            self.tok(p, p + 3, VARIADIC)
        } else {
            None
        }
    }

    fn star(&mut self, p: usize) -> Option<usize> {
        if self.byte(p, b'*') {
            self.tok(p, p + 1, OPERATOR)
        } else {
            None
        }
    }

    /// The whole word at `p`, where it ends.
    fn word(&self, p: usize) -> Option<usize> {
        let (at, e) = self.wcache.get();
        if at == p {
            return e;
        }
        let e = if p < self.n && is_word(self.b, p) && starts_word(self.b, p) {
            Some(word_end(self.b, p))
        } else {
            None
        };
        self.wcache.set((p, e));
        e
    }

    /// `{{ident}}` at `p`: where it ends.
    fn ident(&self, p: usize) -> Option<usize> {
        if !is_name_start(self.b, p) {
            return None;
        }
        let e = self.word(p)?;
        keyword(&self.b[p..e]).is_none().then_some(e)
    }

    /// `\b_\b` at `p`.
    fn blank(&self, p: usize) -> bool {
        self.byte(p, b'_') && starts_word(self.b, p) && !is_word(self.b, p + 1)
    }

    /// `\b_\b`, a pop.
    fn blank_pop(&mut self, p: usize) -> Option<usize> {
        if self.blank(p) {
            self.close(p, p + 1, BLANK)
        } else {
            None
        }
    }

    /// `{{ident}}(?={{noise}}\.)`: a package's name before its member.
    fn qualifier(&mut self, p: usize) -> Option<usize> {
        let e = self.ident(p)?;
        self.byte(noise(self.b, e), b'.')
            .then(|| self.tok(p, e, VARIABLE))
            .flatten()
    }

    /// `pop-type-identifier`.
    fn type_name(&mut self, p: usize) -> Option<usize> {
        let e = self.ident(p)?;
        let leaf = if is_predeclared_type(&self.b[p..e]) {
            STORAGE_BUILTIN
        } else {
            STORAGE
        };
        self.close(p, e, leaf)
    }

    /// `(?=(?:{{noise}}\))*{{noise}}\()` at `e`: a call.
    #[inline(always)]
    fn call_follows(&self, e: usize) -> bool {
        let mut q = noise(self.b, e);
        while self.byte(q, b')') {
            q = noise(self.b, q + 1);
        }
        self.byte(q, b'(')
    }

    /// `match-comments`.
    fn comments(&mut self, p: usize) -> Option<usize> {
        if !self.byte(p, b'/') {
            return None;
        }
        let b = self.b;
        match b.get(p + 1) {
            Some(b'/') => {
                if self.depth >= SYNTECT_DEPTH {
                    return None;
                }
                if b[p + 2..].starts_with(b"go:") {
                    self.annotation(p);
                } else {
                    self.line_marker(p);
                    self.put(p, p + 2, LINE_MARK);
                    self.put(p + 2, self.n, LINE_COMMENT);
                }
                // `$\n?` takes the line's `\n`
                Some(self.n + 1)
            }
            Some(b'*') => {
                if sql::marker_comment(b, p + 2) {
                    self.mark = b[p + 2..]
                        .windows(2)
                        .position(|w| w == b"*/")
                        .map_or(self.n, |i| p + 4 + i);
                }
                self.tok_push(p, p + 2, COMMENT_BEGIN, &[BLOCK_COMMENT])
            }
            _ => None,
        }
    }

    /// A line comment at `p`: a SQL marker (`// language=SQL`) for the next string, if it is one.
    #[cold]
    #[inline(never)]
    fn line_marker(&mut self, p: usize) {
        if sql::marker_comment(self.b, p + 2) {
            self.mark = self.n;
        }
    }

    /// `(//)(go)(:)({{ident}})?` at `p`, then `\S+` to the line's end.
    fn annotation(&mut self, p: usize) {
        let b = self.b;
        self.put(p, p + 2, ANN_MARK);
        self.put(p + 2, p + 4, ANN_KEYWORD);
        self.put(p + 4, p + 5, ANN_COLON);
        let mut q = p + 5;
        if let Some(e) = self.ident(q) {
            self.put(q, e, ANN_FUNCTION);
            q = e;
        }
        while q < self.n {
            let s = skip_spaces(b, q);
            self.put(q, s, ANNOTATION);
            let mut e = s;
            while e < self.n && space_len(b, e).is_none() {
                e += char_len(b[e]);
            }
            self.put(s, e, ANN_PARAMETER);
            q = e;
        }
    }

    /// `match-any`: `match-comments`, then `match-tokens`.
    fn any(&mut self, p: usize) -> Option<usize> {
        if p >= self.n {
            return None;
        }
        let b = self.b;
        let c = b[p];
        if is_word(b, p) {
            if let Some(e) = self.word(p) {
                if let Some(k) = keyword(&b[p..e]) {
                    return self.keyword_at(p, e, k);
                }
                if let Some(q) = self.identifiers(p, e) {
                    return Some(q);
                }
            }
            return self.literals(p);
        }
        match c {
            b'/' => self.comments(p).or_else(|| self.operators(p)),
            b'.' | b'\'' | b'"' | b'`' => self.literals(p).or_else(|| self.punctuation(p)),
            _ => self.operators(p).or_else(|| self.punctuation(p)),
        }
    }

    /// `match-keywords` at `p`.
    fn keywords(&mut self, p: usize) -> Option<usize> {
        let e = self.word(p)?;
        let k = keyword(&self.b[p..e])?;
        self.keyword_at(p, e, k)
    }

    /// The keyword `k`, `b[p..e]`.
    fn keyword_at(&mut self, p: usize, e: usize, k: Keyword) -> Option<usize> {
        match k {
            Keyword::Control => self.tok(p, e, CONTROL),
            Keyword::Range => self.tok(p, e, OTHER_KEYWORD),
            Keyword::Import => self.tok(p, e, IMPORT),
            Keyword::Package => self.tok(p, e, PACKAGE),
            Keyword::Chan => self.tok_push(p, e, CHAN_KEYWORD, &[TYPE]),
            Keyword::Const => self.tok_push(p, e, CONST_KEYWORD, &[DECL_HEAD]),
            Keyword::Var => self.tok_push(p, e, VAR_KEYWORD, &[DECL_HEAD + VAR]),
            Keyword::Func => self.tok_push(p, e, FUNC_KEYWORD, &[FUNC_HEAD]),
            Keyword::Interface => self.tok_push(p, e, INTERFACE_KEYWORD, &[IFACE_HEAD]),
            Keyword::Map => self.tok_push(p, e, MAP_KEYWORD, &[MAP_HEAD]),
            Keyword::Struct => self.tok_push(p, e, STRUCT_KEYWORD, &[STRUCT_HEAD]),
            Keyword::Type => self.tok_push(p, e, TYPE_KEYWORD, &[TYPE_HEAD]),
        }
    }

    /// `match-identifiers` at `p`, the whole word `b[p..e]`, not a keyword.
    fn identifiers(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        let w = &b[p..e];
        if matches!(w, b"true" | b"false" | b"nil") {
            return self.tok(p, e, LANGUAGE);
        }
        if !is_name_start(b, p) {
            return None;
        }
        // `{{ident}}` from here on
        if self.call_follows(e)
            && let Some(q) = self.call(p, e, false)
        {
            return Some(q);
        }
        if self.short_var_from(e)
            && let Some(q) = self.enter(p, &[SHORT_VAR])
        {
            return Some(q);
        }
        if w == b"_" {
            return self.tok(p, e, BLANK);
        }
        self.tok(p, e, VARIABLE)
    }

    /// `match-call-or-cast` at `p`, or with `pop`, `pop-call-or-cast`.
    fn call_or_cast(&mut self, p: usize, pop: bool) -> Option<usize> {
        let e = self.ident(p)?;
        if !self.call_follows(e) {
            return None;
        }
        self.call(p, e, pop)
    }

    /// The name `b[p..e]` before a call's `(`, among `match-call-or-cast`'s patterns.
    fn call(&mut self, p: usize, e: usize, pop: bool) -> Option<usize> {
        let w = &self.b[p..e];
        if matches!(w, b"make" | b"new") {
            let q = if pop {
                self.then(p, e, BUILTIN_FUNCTION, &[MAKE_ARGS])
            } else {
                self.tok_push(p, e, BUILTIN_FUNCTION, &[MAKE_ARGS])
            };
            if q.is_some() {
                return q;
            }
        }
        let leaf = if is_predeclared_type(w) {
            BUILTIN_CAST
        } else if is_predeclared_func(w) {
            BUILTIN_FUNCTION
        } else {
            FUNCTION
        };
        if pop {
            self.close(p, e, leaf)
        } else {
            self.tok(p, e, leaf)
        }
    }

    /// `(?={{ident}}(?:{{noise}},{{noise}}{{ident}})*{{noise}}:=)`, after the first name, at
    /// `e`.
    fn short_var_from(&self, mut e: usize) -> bool {
        loop {
            let q = noise(self.b, e);
            if self.b[q.min(self.n)..].starts_with(b":=") {
                return true;
            }
            if !self.byte(q, b',') {
                return false;
            }
            let Some(f) = self.ident(noise(self.b, q + 1)) else {
                return false;
            };
            e = f;
        }
    }

    /// The method's lookahead after `func`:
    /// `(?=\({{noise}}[^)]+{{noise}}\){{noise}}{{ident}}{{noise}}\()`.
    fn method_follows(&self, p: usize) -> bool {
        if !self.byte(p, b'(') {
            return false;
        }
        let b = self.b;
        let s = p + 1;
        let close = (s..self.n).find(|&q| b[q] == b')').unwrap_or(self.n);
        let rest = |r: usize| {
            let Some(e) = self.ident(noise(b, r + 1)) else {
                return false;
            };
            self.byte(noise(b, e), b'(')
        };
        // `[^)]+` to the first `)`, or to a comment in the noise that holds one
        if close < self.n && close > s && rest(close) {
            return true;
        }
        (s..=close.min(self.n.saturating_sub(1))).any(|j| {
            b[j..].starts_with(b"/*") && {
                let r = noise(b, j);
                r > close && self.byte(r, b')') && rest(r)
            }
        })
    }

    /// The named parameters' lookahead:
    /// `(?=(?:{{noise}}{{ident}}{{noise}},{{noise}})*{{ident}}{{noise}}(?:\.\.\.|[^\s/,).]))`.
    fn named_params_follow(&self, p: usize) -> bool {
        let b = self.b;
        let tail = |s: usize| {
            let Some(e) = self.ident(s) else {
                return false;
            };
            let t = noise(b, e);
            if t >= self.n {
                return false;
            }
            b[t..].starts_with(b"...") || !matches!(b[t], b'/' | b',' | b')' | b'.')
        };
        // the starts after each repetition, then from the last back
        let mut starts = [0usize; 64];
        let mut k = 0;
        let mut q = p;
        loop {
            if k < starts.len() {
                starts[k] = q;
                k += 1;
            }
            let Some(e) = self.ident(noise(b, q)) else {
                break;
            };
            let t = noise(b, e);
            if !self.byte(t, b',') {
                break;
            }
            q = noise(b, t + 1);
        }
        starts[..k].iter().rev().any(|&s| tail(s))
    }

    /// `match-literals` at `p`: numbers, runes, strings.
    fn literals(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        match b[p] {
            b'\'' => {
                // `\'({{char_escape}})'`, then `\'[^']*'`
                if self.byte(p + 1, b'\\')
                    && let Some(e) = char_escape(b, p + 1)
                    && self.byte(e, b'\'')
                {
                    self.put(p, p + 1, RUNE);
                    self.put(p + 1, e, RUNE_ESCAPE);
                    return self.tok(e, e + 1, RUNE);
                }
                let e = (p + 1..self.n).find(|&q| b[q] == b'\'')?;
                self.tok(p, e + 1, RUNE)
            }
            b'"' => self.string(p, false),
            b'`' => self.string(p, true),
            c if c.is_ascii_digit() || c == b'.' || (c >= 0x80 && digit(b, p).is_some()) => {
                let (e, leaf, caps) = number(b, p)?;
                for (s, ce, cl) in caps {
                    if ce > s {
                        self.put(self.at.max(p), s, leaf);
                        self.put(s, ce, cl);
                    }
                }
                self.tok(self.at.max(p), e, leaf)
            }
            _ => None,
        }
    }

    /// `match-operators` at `p`.
    fn operators(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let next = |i: usize| if p + i < self.n { b[p + i] } else { 0 };
        let (len, leaf) = match b[p] {
            b'!' if next(1) == b'=' => (2, OPERATOR),
            b'!' => (1, OPERATOR),
            b'%' if next(1) == b'=' => (2, ASSIGNMENT),
            b'%' => (1, OPERATOR),
            // `\&^=` and `\&^` never match: `^` is the line's start
            b'&' if next(1) == b'&' => (2, OPERATOR),
            b'&' if next(1) == b'=' => (2, ASSIGNMENT),
            b'&' => (1, OPERATOR),
            b'*' if next(1) == b'=' => (2, ASSIGNMENT),
            b'*' => (1, OPERATOR),
            b'+' if next(1) == b'+' => (2, OPERATOR),
            b'+' if next(1) == b'=' => (2, ASSIGNMENT),
            b'+' => (1, OPERATOR),
            b'-' if next(1) == b'-' || next(1) == b'=' => (2, ASSIGNMENT),
            b'-' => (1, OPERATOR),
            b'/' if next(1) == b'=' => (2, ASSIGNMENT),
            b'/' => (1, OPERATOR),
            b':' if next(1) == b'=' => (2, ASSIGNMENT),
            // `<-` and `<` come before `<<=`, `<<` and `<=`
            b'<' if next(1) == b'-' => (2, OPERATOR),
            b'<' => (1, OPERATOR),
            b'=' if next(1) == b'=' => (2, OPERATOR),
            b'=' => (1, ASSIGNMENT),
            b'>' => match (next(1), next(2)) {
                (b'=', _) => (2, ASSIGNMENT),
                (b'>', b'=') => (3, ASSIGNMENT),
                (b'>', _) => (2, OPERATOR),
                _ => (1, OPERATOR),
            },
            b'^' if next(1) == b'=' => (2, ASSIGNMENT),
            b'^' => (1, OPERATOR),
            b'|' if next(1) == b'=' => (2, ASSIGNMENT),
            b'|' if next(1) == b'|' => (2, OPERATOR),
            b'|' => (1, OPERATOR),
            _ => return None,
        };
        self.tok(p, p + len, leaf)
    }

    /// `match-punctuation` at `p`.
    fn punctuation(&mut self, p: usize) -> Option<usize> {
        match self.b[p] {
            b',' => self.tok(p, p + 1, SEPARATOR),
            b'.' if self.b[p..].starts_with(b"...") => self.tok(p, p + 3, VARIADIC),
            b':' => self.tok(p, p + 1, SEPARATOR),
            b';' => self.tok(p, p + 1, TERMINATOR),
            b'.' => self.tok_push(p, p + 1, DOT, &[SELECTOR]),
            b'(' if p == self.sql_paren => {
                self.tok_push(p, p + 1, PARENS_BEGIN, &[PSQL + self.sql_call])
            }
            b'(' => self.tok_push(p, p + 1, PARENS_BEGIN, &[PARENS]),
            b')' => self.tok(p, p + 1, PARENS_END),
            b'[' => self.tok(p, p + 1, BRACKETS_BEGIN),
            b']' => self.tok_push(p, p + 1, BRACKETS_END, &[AFTER_BRACKET]),
            b'{' => self.tok_push(p, p + 1, BRACES_BEGIN, &[BLOCK]),
            b'}' => self.tok(p, p + 1, BRACES_END),
            _ => None,
        }
    }

    /// `pop-type`'s patterns, in the context that includes them.
    fn types(&mut self, p: usize) -> Option<usize> {
        if p >= self.n {
            return None;
        }
        let b = self.b;
        match b[p] {
            b';' => return self.close(p, p + 1, TERMINATOR),
            b'/' => {
                if let Some(q) = self.comments(p) {
                    return Some(q);
                }
            }
            b'(' => {
                if let Some(q) = self.tok_push(p, p + 1, PARENS_BEGIN, &[TYPE_PARENS, TYPE]) {
                    return Some(q);
                }
            }
            b'[' => return self.then(p, p + 1, BRACKETS_BEGIN, &[TYPE_BRACKETS]),
            _ => {}
        }
        if let Some(q) = self.operators(p) {
            return Some(q);
        }
        if let Some(e) = self.word(p) {
            let set = match keyword(&b[p..e]) {
                Some(Keyword::Chan) => Some((CHAN_KEYWORD, TYPE)),
                Some(Keyword::Interface) => Some((INTERFACE_KEYWORD, IFACE_HEAD)),
                Some(Keyword::Map) => Some((MAP_KEYWORD, MAP_HEAD)),
                Some(Keyword::Struct) => Some((STRUCT_KEYWORD, STRUCT_HEAD)),
                Some(Keyword::Func) => Some((FUNC_KEYWORD, PARAMS_RETURNS)),
                _ => None,
            };
            if let Some((leaf, c)) = set {
                return self.then(p, e, leaf, &[c]);
            }
            if self.ident(p).is_some() {
                return self.shift(p, &[NAMED_TYPE]);
            }
        }
        self.nonblank_pop(p)
    }

    /// `pop-func-parameter-and-return-lists`.
    fn params_returns(&mut self, p: usize) -> Option<usize> {
        self.comments(p)
            .or_else(|| {
                self.byte(p, b'(')
                    .then(|| self.shift(p, &[RETURNS, PARAM_LIST]))
                    .flatten()
            })
            .or_else(|| self.nonblank_pop(p))
    }

    /// `pop-type-alias-or-typedef`.
    fn alias(&mut self, p: usize) -> Option<usize> {
        self.terminator(p)
            .or_else(|| self.comments(p))
            .or_else(|| {
                self.byte(p, b'=')
                    .then(|| self.then(p, p + 1, ASSIGNMENT, &[TYPE]))
                    .flatten()
            })
            .or_else(|| self.nonblank_set(p, &[TYPE]))
    }

    /// An interface's or a struct's body.
    fn type_body(&mut self, p: usize, is_struct: bool) -> Option<usize> {
        if let Some(q) = self
            .closing(p, b'}', BRACES_END)
            .or_else(|| self.keywords(p))
            .or_else(|| self.star(p))
            .or_else(|| self.qualifier(p))
            .or_else(|| {
                self.byte(p, b'.')
                    .then(|| self.tok(p, p + 1, DOT))
                    .flatten()
            })
        {
            return Some(q);
        }
        if let Some(e) = self.ident(p) {
            let b = self.b;
            // an embedded type: `(?={{noise}}(?:"|`|//|;|\}|$))`, `"` and `` ` `` a struct's
            let q = noise(b, e);
            let embedded = q >= self.n
                || matches!(b[q], b';' | b'}')
                || b[q..].starts_with(b"//")
                || (is_struct && matches!(b[q], b'"' | b'`'));
            if embedded {
                let leaf = if is_predeclared_type(&b[p..e]) {
                    INHERITED_BUILTIN
                } else {
                    INHERITED
                };
                return self.tok(p, e, leaf);
            }
            if is_struct {
                if self.blank(p) {
                    return self.tok(p, p + 1, BLANK);
                }
                if let Some(r) = self.tok_push(p, e, MEMBER_DECLARATION, &[STRUCT_FIELD]) {
                    return Some(r);
                }
            } else if self.byte(q, b'(')
                && let Some(r) = self.tok_push(p, e, FUNCTION_NAME, &[IFACE_METHOD])
            {
                return Some(r);
            }
        }
        self.any(p)
    }

    /// A const or var declaration's contexts (`top`, the var ones [`VAR`] above).
    fn declaration(&mut self, p: usize, top: u8) -> Option<usize> {
        let var = top >= DECL_HEAD + VAR;
        let ctx = top - if var { VAR } else { 0 };
        let shift = if var { VAR } else { 0 };
        let name = if var {
            DECLARATION
        } else {
            CONSTANT_DECLARATION
        };
        // `\b_\b(?={{noise}},)`, `{{ident}}(?={{noise}},)`, then without the lookahead and
        // `then`: what the name pushes or sets
        let names = |s: &mut Self, then: Option<bool>| -> Option<usize> {
            let e = if s.blank(p) { p + 1 } else { s.ident(p)? };
            let leaf = if e == p + 1 && s.b[p] == b'_' {
                BLANK
            } else {
                name
            };
            match then {
                None => s
                    .byte(noise(s.b, e), b',')
                    .then(|| s.tok(p, e, leaf))
                    .flatten(),
                Some(true) => s.tok_push(p, e, leaf, &[DECL_TYPE + shift]),
                Some(false) => s.then(p, e, leaf, &[DECL_TYPE + shift]),
            }
        };
        match ctx {
            DECL_HEAD => {
                if self.byte(p, b'(') {
                    return self.then(p, p + 1, PARENS_BEGIN, &[DECL_GROUP + shift]);
                }
                self.comments(p)
                    .or_else(|| self.comma(p))
                    .or_else(|| names(self, None))
                    .or_else(|| names(self, Some(false)))
                    .or_else(|| self.nonblank_pop(p))
            }
            DECL_GROUP => self
                .closing(p, b')', PARENS_END)
                .or_else(|| names(self, None))
                .or_else(|| names(self, Some(true)))
                .or_else(|| self.any(p)),
            DECL_TYPE | DECL_ASSIGN => {
                if let Some(q) = self.terminator(p).or_else(|| self.comments(p)) {
                    return Some(q);
                }
                if self.byte(p, b'=') {
                    return self.then(p, p + 1, ASSIGNMENT, &[DECL_EXPRS + shift]);
                }
                if ctx == DECL_TYPE {
                    self.nonblank_set(p, &[DECL_ASSIGN + shift, TYPE])
                } else {
                    self.nonblank_pop(p)
                }
            }
            DECL_EXPRS => self
                .semicolon(p)
                .or_else(|| self.comments(p))
                .or_else(|| self.nonblank_set(p, &[DECL_VALUES + shift])),
            _ => {
                if let Some(q) = self.terminator(p) {
                    return Some(q);
                }
                if !var
                    && let Some(e) = self.word(p)
                    && &self.b[p..e] == b"iota"
                {
                    return self.tok(p, e, INT_DEC);
                }
                self.any(p)
            }
        }
    }

    /// `match-template-string` at `p`: `{{(?=.*}})`.
    fn template(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if p + 1 < self.n
            && b[p] == b'{'
            && b[p + 1] == b'{'
            && b[p + 2..].windows(2).any(|w| w == b"}}")
        {
            return self.tok_push(p, p + 2, INTERP_BEGIN, &[INTERP]);
        }
        None
    }

    /// A template's `{{ … }}`.
    fn interpolation(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if p >= self.n {
            return None;
        }
        let c = b[p];
        if b[p..].starts_with(b"}}") {
            return self.close(p, p + 2, INTERP_END);
        }
        // `\s-`, `-\s` (the line's `\n` a space)
        if let Some(k) = space_len(b, p)
            && self.byte(p + k, b'-')
        {
            return self.tok(p, p + k + 1, RIGHT_TRIM);
        }
        if c == b'-' {
            if p + 1 == self.n {
                return self.tok(p, p + 2, LEFT_TRIM);
            }
            if let Some(k) = space_len(b, p + 1) {
                return self.tok(p, p + 1 + k, LEFT_TRIM);
            }
        }
        match c {
            b':' if self.byte(p + 1, b'=') => return self.tok(p, p + 2, INTERP_ASSIGN),
            b'=' => return self.tok(p, p + 1, INTERP_ASSIGN),
            b'|' => return self.tok(p, p + 1, PIPE),
            b'.' | b'$' => {
                // `(\.)([\w]+)`, `(\$)[\w]+`, `[.$]`
                let e = word_end(b, p + 1);
                if e > p + 1 {
                    if c == b'.' {
                        self.put(p, p + 1, INTERP_DOT);
                        return self.tok(p + 1, e, INTERP_MEMBER);
                    }
                    self.put(p, p + 1, TEMPLATE_VAR_MARK);
                    return self.tok(p + 1, e, TEMPLATE_VAR);
                }
                return self.tok(p, p + 1, TEMPLATE_VAR);
            }
            _ => {}
        }
        if let Some(e) = self.word(p) {
            let w = &b[p..e];
            if matches!(
                w,
                b"if"
                    | b"else"
                    | b"range"
                    | b"template"
                    | b"with"
                    | b"end"
                    | b"nil"
                    | b"define"
                    | b"block"
            ) {
                return self.tok(p, e, INTERP_KEYWORD);
            }
            if matches!(
                w,
                b"and"
                    | b"call"
                    | b"html"
                    | b"index"
                    | b"slice"
                    | b"js"
                    | b"len"
                    | b"not"
                    | b"or"
                    | b"print"
                    | b"printf"
                    | b"println"
                    | b"urlquery"
                    | b"eq"
                    | b"ne"
                    | b"lt"
                    | b"le"
                    | b"gt"
                    | b"ge"
            ) {
                return self.tok(p, e, INTERP_FUNCTION);
            }
        }
        match c {
            b'/' => self.comments(p),
            b'`' => self.tok_push(p, p + 1, OTHER + BEGIN, &[RAW]),
            b'"' => self.tok_push(p, p + 1, DOUBLE + BEGIN, &[STRING]),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Lang;
    use crate::tests::marked;

    fn of(lines: &[&str]) -> Vec<(String, String)> {
        marked(Lang::Go, lines)
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
        const RE: &str = "string.quoted.double.go";
        const RAW: &str = "string.quoted.other.go";
        let l = [
            r#"var a = regexp.MustCompile("^(?P<y>\\d{4})\t[]a-z]%d{{x}}$")"#,
            "ok, _ := regexp.MatchString(`(?i:[[:alpha:]]+|\\p{^Greek})*?",
            "\\Q*\\E\\x{41}\\101`, s)",
            r#"fmt.Printf("%d{2}\\d", x); f := myregexp.MustCompile("a+")"#,
        ];
        has(&l, "^", &format!("{RE} keyword.control.anchor.regexp"));
        has(
            &l,
            "?P",
            &format!("{RE} punctuation.definition.group.begin.regexp"),
        );
        has(&l, "y", &format!("{RE} entity.name.other.group.regexp"));
        has(
            &l,
            r"\\d",
            &format!("{RE} constant.other.character-class.escape.backslash.regexp"),
        );
        // the host's escape between the regex's tokens
        has(&l, r"\t", &format!("{RE} constant.character.escape.go"));
        // a `]` first in a class is a character; `%d` and `{{` are the regex's text
        has(
            &l,
            "]",
            &format!("{RE} constant.other.character-class.set.regexp"),
        );
        let got = of(&l[..1]);
        assert!(
            !got.iter()
                .any(|(_, s)| s.contains("placeholder") || s.contains("interpolation")),
            "{got:?}"
        );
        // a raw regex across lines
        has(
            &l,
            "?i",
            &format!("{RAW} keyword.other.option-toggle.regexp"),
        );
        has(
            &l,
            "[:alpha:]",
            &format!(
                "{RAW} constant.other.character-class.set.regexp constant.other.character-class.posix.regexp"
            ),
        );
        has(&l, "^", &format!("{RAW} keyword.operator.negation.regexp"));
        has(
            &l,
            "Greek",
            &format!("{RAW} support.constant.unicode-property.regexp"),
        );
        has(
            &l,
            r"\Q",
            &format!("{RAW} constant.character.escape.backslash.regexp"),
        );
        has(
            &l,
            r"\x{41}",
            &format!("{RAW} constant.character.escape.backslash.regexp"),
        );
        has(
            &l,
            r"\101",
            &format!("{RAW} constant.character.escape.backslash.regexp"),
        );
        // not a regex: any other call
        has(&l, "%d", &format!("{RE} constant.other.placeholder.go"));
        let got = of(&l[3..]);
        assert!(!got.iter().any(|(_, s)| s.contains("regexp")), "{got:?}");
    }

    #[test]
    fn sql_in_strings() {
        const DQ: &str = "string.quoted.double.go";
        const RAW: &str = "string.quoted.other.go";
        let dml = |s: &str| format!("{s} keyword.other.DML.sql");
        let l = [
            "// language=SQL",
            r#"var q = "SELECT a FROM t WHERE b = '%d'\n""#,
            r#"rows, err := db.Query("DELETE FROM u WHERE id = $1", id)"#,
            "err = conn.QueryRow(ctx, `UPDATE u SET n = 2`).Scan(&n)",
            r#"_, err = db.Exec(query, "LIMIT")"#,
            r#"err = db.Get(&p, "INSERT INTO p VALUES (1)")"#,
            r#"v := params.Get("ORDER")"#,
            r#"_, err = conn.Prepare(ctx, "GROUP", "VALUES (2)")"#,
            "rows, _ := conn.Query(",
            "\tcontext.Background(),",
            r#"	"UNION ALL SELECT n","#,
            ")",
            "_, err = tx.ExecContext(c, `",
            "  ALTER TABLE foo -- a note",
            "  ADD id int`)",
            r#"db.Exec("HAVING " + "JOIN x", "WHERE")"#,
            r#"f("NULL"); db.Exec(fmt.Sprintf("BETWEEN %d", 1))"#,
            r#"stmt.Exec("CASE"); insertStmt.Get(&x, "ELSE")"#,
        ];
        // a marker's string; the host's escape between SQL's tokens, `%d` SQL's text
        has(&l, "SELECT", &dml(DQ));
        has(&l, r"\n", &format!("{DQ} constant.character.escape.go"));
        has(&l, "%d", &format!("{DQ} string.quoted.single.sql"));
        // the first argument, after a context, after a destination
        has(&l, "DELETE", &dml(DQ));
        has(&l, "UPDATE", &dml(RAW));
        has(&l, "INSERT INTO", &dml(DQ));
        // pgx's statement name, then its SQL
        has(&l, "VALUES", &format!("{DQ} keyword.other.DML.II.sql"));
        has(&l, "GROUP", DQ);
        // over lines, after a context
        has(&l[8..12], "UNION ALL", &dml(DQ));
        // a raw string over lines: SQL's line comment ends with its line
        has(
            &l,
            " a note",
            &format!("{RAW} comment.line.double-dash.sql"),
        );
        has(&l, "  ADD id ", RAW);
        // `+` continues the SQL; not the next argument
        has(&l, "JOIN", &dml(DQ));
        has(&l, "WHERE", DQ);
        // a variable first, a string first to `Get`, any other call, a call in the argument,
        // and a prepared statement's arguments
        for w in ["LIMIT", "ORDER", "NULL", "BETWEEN", "CASE", "ELSE"] {
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
    fn declarations() {
        let l = [
            "package main",
            "import \"fmt\"",
            "type Point struct {",
            "\tX, Y int `json:\"x\"`",
            "\tio.Reader",
            "}",
            "func (p *Point) Add(q Point, n ...int) (r Point, err error) {",
            "\tconst limit = iota",
            "\tvar m map[string][]byte",
            "\tx, _ := make([]int, 3), new(T)",
            "}",
        ];
        has(&l, "package", "keyword.other.package.go");
        has(&l, "Point", "entity.name.type.go");
        has(&l, "struct", "storage.type.keyword.struct.go");
        has(&l, "X", "variable.other.member.declaration.go");
        has(&l, "int", "storage.type.go support.type.builtin.go");
        has(&l, "Reader", "entity.other.inherited-class.go");
        has(&l, "func", "storage.type.keyword.function.go");
        has(&l, "p", "variable.parameter.go");
        has(&l, "Add", "entity.name.function.go");
        has(&l, "...", "keyword.operator.variadic.go");
        has(&l, "limit", "variable.other.constant.declaration.go");
        has(&l, "iota", "constant.numeric.integer.decimal.go");
        has(&l, "m", "variable.declaration.go");
        has(&l, "map", "storage.type.keyword.map.go");
        has(&l, "x", "variable.declaration.go");
        has(&l, "_", "variable.language.blank.go");
        has(
            &l,
            "make",
            "variable.function.go support.function.builtin.go",
        );
    }

    #[test]
    fn literals_comments_and_templates() {
        let l = [
            "//go:generate stringer -type=Op",
            "a := 0x1F + 1.5e3 + 0o17i + 'x' + '\\n' + 089 // done",
            "s := \"%5.2f {{.Name | printf}} \\u00e9\" /* block",
            " * still */ return",
        ];
        has(
            &l,
            "generate",
            "comment.line.go meta.annotation.go meta.variable.function.go",
        );
        has(
            &l,
            "-type=Op",
            "comment.line.go meta.annotation.go meta.variable.parameter.go",
        );
        has(
            &l,
            "0x",
            "constant.numeric.integer.hexadecimal.go punctuation.definition.numeric.base.go",
        );
        has(&l, "1F", "constant.numeric.integer.hexadecimal.go");
        has(
            &l,
            ".",
            "constant.numeric.float.decimal.go punctuation.separator.decimal.go",
        );
        has(
            &l,
            "i",
            "constant.numeric.imaginary.octal.go storage.type.numeric.go",
        );
        has(&l, "'x'", "constant.character.go");
        has(&l, "089", "invalid.illegal.go");
        has(
            &l,
            "%5.2f",
            "string.quoted.double.go constant.other.placeholder.go",
        );
        has(&l, "Name", "meta.interpolation.go variable.other.member.go");
        has(
            &l,
            "printf",
            "meta.interpolation.go variable.function.go support.function.builtin.go",
        );
        has(
            &l,
            "*",
            "comment.block.go punctuation.definition.comment.go",
        );
        has(&l, "return", "keyword.control.go");
    }
}
