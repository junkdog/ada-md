//! Git's own files on the line-format engine ([`crate::line_format`]): bat's `Git Ignore`,
//! `Git Attributes`, `Git Mailmap` and `Git Link`, with the parts of the hidden `Git Common`
//! they share (comments, `fnmatch` patterns, e-mail addresses). Each format is a reader of its
//! own over a line, in the order its grammar's contexts take the line's parts: a comment, or
//! an entry (a pattern and its attributes, a name and its addresses, `gitdir:` and a path).
//! Since X-034 they no longer run on a stack of bat's context ids (`git_scan.rs`, gone).
//!
//! A line leaves something open in two places only, so the state is a word, 0 for nothing: an
//! unquoted attributes pattern whose last `\` or unclosed `[` took the line's `\n` (the next
//! line goes on in the pattern), and Git Link's `gitdir` waiting for its `:`.
//!
//! Its readings are held by `tests/readings.rs`' snapshots (X-116).
//! A span's kind is fixed ([`scopes`]): a scope list of its own, an `fnmatch` leaf under the
//! pattern it is in (an ignore pattern, a quoted or an unquoted attributes pattern), or an
//! attribute's leaf under its builtin's meta scope.
//!
//! `Git Config` and `Git Rebase Todo` embed a shell, and `Git Commit` and `Git Log` bat's
//! `Diff`; they have no lexer here yet, so ada prints them plain (ADR-43).

use crate::emit::{Push, Runs, Sink};
use crate::line_format::{self, Cursor, Format, blanks, find, leads};
use crate::matcher::{Atom, Line, atoms, is_blank, is_space, is_word};
use crate::{Out, State};

atoms! {
    COMMENT = "comment.line.git",
    COMMENT_MARK = "punctuation.definition.comment.git",
    EMAIL_ENTITY = "entity.name.reference.email.git",
    DOMAIN_SEP = "punctuation.separator.domain.git",
    EMAIL_END_MARK = "punctuation.definition.reference.email.end.git",
    EMAIL_META_S = "meta.reference.email.git",
    EMAIL_SEP = "punctuation.separator.email.git",
    CLASS_BEGIN = "keyword.control.char-class.begin.fnmatch.git",
    PATH_SEP = "punctuation.separator.path.fnmatch.git",
    EXTENSION_SEP = "punctuation.separator.path.extension.fnmatch.git",
    ASTERISK = "keyword.operator.path.asterisk.fnmatch.git",
    PATH_ESCAPE = "constant.character.escape.path.fnmatch.git",
    ENV_VAR = "variable.language.environment.other.fnmatch.git",
    BAD_DRIVE = "invalid.illegal.separator.drive.fnmatch.git",
    BAD_ESCAPE = "invalid.illegal.escape.path.fnmatch.git",
    CLASS_META = "meta.char-class.fnmatch.git",
    CLASS_END = "keyword.control.char-class.end.fnmatch.git",
    CLASS_EOL = "invalid.illegal.unexpected.eol.fnmatch.git",
    CLASS_ESCAPE = "constant.character.escape.char-class.fnmatch.git",
    CLASS_BAD = "invalid.illegal.unexpected.char-class.fnmatch.git",
    CLASS_CHAR = "constant.character.char-class.fnmatch.git",
    CLASS_LOGICAL = "keyword.operator.logical.fnmatch.git",
    PARENT = "constant.language.path.parent.fnmatch.git",
    SELF = "constant.language.path.self.fnmatch.git",
    DRIVE = "punctuation.separator.drive.fnmatch.git",
    HOME = "variable.language.environment.home.fnmatch.git",
    IGNORE_NEGATE = "keyword.operator.logical.path.fnmatch.git.ignore",
    IGNORE_STRING = "string.unquoted.git.ignore",
    IGNORE_PATTERN_S = "entity.name.pattern.git.ignore",
    EMAIL_BEGIN = "punctuation.definition.reference.email.begin.git",
    USER = "meta.reference.user.git",
    LINK_VALUE = "meta.mapping.value.git.link",
    LINK_KEY = "meta.mapping.key.git.link",
    SEPARATOR_EXPECTED = "invalid.illegal.separator-expected.git.link",
    GITDIR_KW = "keyword.other.gitdir.git.link",
    GITDIR_EXPECTED = "invalid.illegal.gitdir-expected.git.link",
    LINK_PATH = "meta.path.git.link",
    LINK_STRING = "string.unquoted.git.link",
    LINK_TERMINATOR = "punctuation.terminator.path.git.link",
    LINK_PATH_SEP = "punctuation.separator.path.git.link",
    LINK_BAD_PATH = "invalid.illegal.path.git.link",
    LINK_PARENT = "constant.language.path.parent.git.link",
    LINK_SELF = "constant.language.path.self.git.link",
    LINK_DRIVE = "punctuation.separator.drive.git.link",
    LINK_HOME = "variable.language.environment.home.git.link",
    LINK_MAPPING = "meta.mapping.git.link",
    LINK_KEY_VALUE = "punctuation.separator.mapping.key-value.git.link",
    ATTR_DOUBLE = "string.quoted.double.git.attributes",
    ATTR_ESCAPE = "constant.character.escape.git.attributes",
    ATTR_KEY = "meta.mapping.key.git.attributes",
    ATTR_NAME = "variable.language.attribute.git.attributes",
    ATTR_MAPPING = "meta.mapping.git.attributes",
    ATTR_KEY_VALUE = "punctuation.separator.mapping.key-value.git.attributes",
    ATTR_PAIR = "punctuation.separator.mapping.pair.git.attributes",
    ATTR_STR_BEGIN = "punctuation.definition.string.begin.git.attributes",
    ATTR_UNQUOTED = "string.unquoted.git.attributes",
    ATTR_EXPECT_VALUE = "meta.mapping.expect-value.git.attributes",
    ATTR_VALUE = "meta.mapping.value.git.attributes",
    ATTR_VALUES_SEP = "punctuation.separator.mapping.values.git.attributes",
    ATTR_BAD_VALUE = "invalid.illegal.value.git.attributes",
    ATTR_LIST = "meta.attributes-list.git.attributes",
    ATTR_LOGICAL = "keyword.operator.logical.git.attributes",
    ATTR_BAD_NAME = "invalid.illegal.attribute-name.git.attributes",
    ATTR_BRACKETS = "meta.brackets.git.attributes",
    ATTR_BRACKETS_BEGIN = "punctuation.definition.brackets.begin.git.attributes",
    ATTR_MACRO = "entity.name.macro.git.attributes",
    ATTR_BRACKETS_END = "punctuation.definition.brackets.end.git.attributes",
    META_DIFF_S = "meta.attribute.builtin.diff.git.attributes",
    META_ENCODING_S = "meta.attribute.builtin.encoding.git.attributes",
    META_EOL_S = "meta.attribute.builtin.eol.git.attributes",
    META_FILTER_S = "meta.attribute.builtin.filter.git.attributes",
    META_MERGE_S = "meta.attribute.builtin.merge.git.attributes",
    META_OTHER_S = "meta.attribute.other.git.attributes",
    META_TEXT_S = "meta.attribute.builtin.text.git.attributes",
    META_WHITESPACE_S = "meta.attribute.builtin.whitespace.git.attributes",
    ATTR_BAD_OPERATOR = "invalid.illegal.operator.git.attributes",
    ATTR_PATTERN = "entity.name.pattern.git.attributes",
    ATTR_STR_END = "punctuation.definition.string.end.git.attributes",
    ATTR_EOL = "invalid.illegal.unexpected.eol.git.attributes",
}

/// The kinds with a scope list of their own, by kind; `FNMATCH` and `ATTRIBUTE` follow.
static FIXED: [&[Atom]; 35] = [
    &[],
    &[s::COMMENT],
    &[s::COMMENT, s::COMMENT_MARK],
    &[s::ATTR_BAD_OPERATOR],
    &[s::ATTR_BRACKETS, s::ATTR_BRACKETS_BEGIN],
    &[s::ATTR_BRACKETS, s::ATTR_UNQUOTED, s::ATTR_MACRO],
    &[s::ATTR_BRACKETS, s::ATTR_BRACKETS_END],
    &[s::ATTR_DOUBLE, s::ATTR_STR_BEGIN],
    &[s::ATTR_DOUBLE, s::ATTR_STR_END],
    &[s::ATTR_LIST],
    &[s::ATTR_LIST, s::ATTR_LOGICAL],
    &[s::ATTR_LIST, s::ATTR_MAPPING],
    &[s::ATTR_LIST, s::ATTR_BAD_NAME],
    &[s::USER],
    &[s::EMAIL_META_S, s::EMAIL_BEGIN],
    &[s::EMAIL_META_S],
    &[s::EMAIL_META_S, s::EMAIL_ENTITY],
    &[s::EMAIL_META_S, s::EMAIL_ENTITY, s::EMAIL_SEP],
    &[s::EMAIL_META_S, s::EMAIL_ENTITY, s::DOMAIN_SEP],
    &[s::EMAIL_META_S, s::EMAIL_END_MARK],
    &[s::LINK_KEY],
    &[s::LINK_KEY, s::GITDIR_KW],
    &[s::GITDIR_EXPECTED],
    &[s::LINK_KEY, s::SEPARATOR_EXPECTED],
    &[s::LINK_MAPPING],
    &[s::LINK_MAPPING, s::LINK_KEY_VALUE],
    &[s::LINK_VALUE],
    &[s::LINK_VALUE, s::LINK_PATH, s::LINK_STRING],
    &[s::LINK_VALUE, s::LINK_TERMINATOR],
    &[
        s::LINK_VALUE,
        s::LINK_PATH,
        s::LINK_STRING,
        s::LINK_PATH_SEP,
    ],
    &[
        s::LINK_VALUE,
        s::LINK_PATH,
        s::LINK_STRING,
        s::LINK_BAD_PATH,
    ],
    &[s::LINK_VALUE, s::LINK_PATH, s::LINK_STRING, s::LINK_PARENT],
    &[s::LINK_VALUE, s::LINK_PATH, s::LINK_STRING, s::LINK_SELF],
    &[s::LINK_VALUE, s::LINK_PATH, s::LINK_STRING, s::LINK_DRIVE],
    &[s::LINK_VALUE, s::LINK_PATH, s::LINK_STRING, s::LINK_HOME],
];

// kind 0, no scopes, and 15, `meta.reference.email` alone, are no span's: their text is
// plain, and an address's meta context closes where its name or domain does
const COMMENT: u16 = 1;
const COMMENT_MARK: u16 = 2;
const BAD_OPERATOR: u16 = 3;
const BRACKETS_BEGIN: u16 = 4;
const MACRO: u16 = 5;
const BRACKETS_END: u16 = 6;
const QUOTE_BEGIN: u16 = 7;
const QUOTE_END: u16 = 8;
const LIST: u16 = 9;
const LIST_LOGICAL: u16 = 10;
const LIST_SPACES: u16 = 11;
const LIST_BAD_NAME: u16 = 12;
const USER: u16 = 13;
const EMAIL_BEGIN: u16 = 14;
const EMAIL: u16 = 16;
const EMAIL_SEP: u16 = 17;
const DOMAIN_SEP: u16 = 18;
const EMAIL_END: u16 = 19;
const LINK_KEY: u16 = 20;
const GITDIR: u16 = 21;
const GITDIR_EXPECTED: u16 = 22;
const SEPARATOR_EXPECTED: u16 = 23;
const LINK_MAPPING: u16 = 24;
const LINK_KEY_VALUE: u16 = 25;
const LINK_VALUE: u16 = 26;
const LINK_PATH: u16 = 27;
const LINK_TERMINATOR: u16 = 28;
const LINK_PATH_SEP: u16 = 29;
const LINK_BAD_PATH: u16 = 30;
const LINK_PARENT: u16 = 31;
const LINK_SELF: u16 = 32;
const LINK_DRIVE: u16 = 33;
const LINK_HOME: u16 = 34;

/// An `fnmatch` pattern's scopes, by what it is in: an ignore pattern, a quoted attributes
/// pattern, an unquoted one.
static PATTERNS: [&[Atom]; 3] = [
    &[s::IGNORE_STRING, s::IGNORE_PATTERN_S],
    &[s::ATTR_DOUBLE, s::ATTR_PATTERN],
    &[s::ATTR_UNQUOTED, s::ATTR_PATTERN],
];
const IN_IGNORE: u16 = 0;
const IN_QUOTED: u16 = 1;
const IN_UNQUOTED: u16 = 2;

/// The first `fnmatch` kind: `FNMATCH + pattern * F_LEAVES.len() + leaf`.
const FNMATCH: u16 = FIXED.len() as u16;

/// The leaves under an `fnmatch` pattern's scopes.
static F_LEAVES: [&[Atom]; 20] = [
    &[],
    &[s::IGNORE_NEGATE],
    &[s::DRIVE],
    &[s::HOME],
    &[s::PARENT],
    &[s::SELF],
    &[s::CLASS_META, s::CLASS_BEGIN],
    &[s::PATH_SEP],
    &[s::EXTENSION_SEP],
    &[s::ASTERISK],
    &[s::PATH_ESCAPE],
    &[s::ENV_VAR],
    &[s::BAD_DRIVE],
    // `\\(?=\s)`, which no text reaches: `\\[^$*?]` takes a `\` before a space first
    &[s::BAD_ESCAPE],
    &[s::CLASS_META],
    &[s::CLASS_META, s::CLASS_LOGICAL],
    &[s::CLASS_META, s::CLASS_CHAR],
    &[s::CLASS_META, s::CLASS_END],
    &[s::CLASS_META, s::CLASS_ESCAPE],
    &[s::CLASS_META, s::CLASS_BAD],
];
const F_TEXT: u16 = 0;
const F_NEGATE: u16 = 1;
const F_DRIVE: u16 = 2;
const F_HOME: u16 = 3;
const F_PARENT: u16 = 4;
const F_SELF: u16 = 5;
const F_CLASS_BEGIN: u16 = 6;
const F_PATH_SEP: u16 = 7;
const F_EXTENSION: u16 = 8;
const F_ASTERISK: u16 = 9;
const F_ESCAPE: u16 = 10;
const F_ENV: u16 = 11;
const F_BAD_DRIVE: u16 = 12;
const F_CLASS: u16 = 14;
const F_CLASS_LOGICAL: u16 = 15;
const F_CLASS_CHAR: u16 = 16;
const F_CLASS_END: u16 = 17;
const F_CLASS_ESCAPE: u16 = 18;
const F_CLASS_BAD: u16 = 19;

/// The first attribute kind: `ATTRIBUTE + meta * A_LEAVES.len() + leaf`, under `ATTR_LIST` and
/// the meta scope of the attribute's builtin (by `META_DIFF`'s order).
const ATTRIBUTE: u16 = FNMATCH + (PATTERNS.len() * F_LEAVES.len()) as u16;

static METAS: [Atom; 8] = [
    s::META_DIFF_S,
    s::META_ENCODING_S,
    s::META_EOL_S,
    s::META_FILTER_S,
    s::META_MERGE_S,
    s::META_OTHER_S,
    s::META_TEXT_S,
    s::META_WHITESPACE_S,
];

/// The leaves under an attribute's scopes.
static A_LEAVES: [&[Atom]; 14] = [
    &[],
    &[s::ATTR_KEY, s::ATTR_NAME],
    &[s::ATTR_MAPPING],
    &[s::ATTR_MAPPING, s::ATTR_KEY_VALUE],
    &[s::ATTR_MAPPING, s::ATTR_PAIR],
    &[s::ATTR_EXPECT_VALUE, s::ATTR_PAIR],
    &[s::ATTR_VALUE],
    &[s::ATTR_VALUE, s::ATTR_VALUES_SEP],
    &[s::ATTR_VALUE, s::ATTR_BAD_VALUE],
    &[s::ATTR_VALUE, s::ATTR_UNQUOTED],
    &[s::ATTR_VALUE, s::ATTR_DOUBLE],
    &[s::ATTR_VALUE, s::ATTR_DOUBLE, s::ATTR_STR_BEGIN],
    &[s::ATTR_VALUE, s::ATTR_DOUBLE, s::ATTR_STR_END],
    &[s::ATTR_VALUE, s::ATTR_DOUBLE, s::ATTR_ESCAPE],
];
const A_NONE: u16 = 0;
const A_KEY: u16 = 1;
const A_MAPPING: u16 = 2;
const A_KEY_VALUE: u16 = 3;
const A_PAIR: u16 = 4;
const A_EXPECT_PAIR: u16 = 5;
const A_VALUE: u16 = 6;
const A_VALUE_SEP: u16 = 7;
const A_BAD_VALUE: u16 = 8;
const A_UNQUOTED: u16 = 9;
const A_DOUBLE: u16 = 10;
const A_BEGIN: u16 = 11;
const A_END: u16 = 12;
const A_ESCAPE: u16 = 13;

/// The scopes under the syntax's base scope a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    let k = usize::from(kind);
    let (outer, leaf): (&[Atom], &[Atom]) = if k < FIXED.len() {
        (&[], FIXED[k])
    } else if kind < ATTRIBUTE {
        let i = k - FIXED.len();
        let Some(&p) = PATTERNS.get(i / F_LEAVES.len()) else {
            return Vec::new();
        };
        (p, F_LEAVES[i % F_LEAVES.len()])
    } else {
        let i = usize::from(kind - ATTRIBUTE);
        let Some(m) = METAS.get(i / A_LEAVES.len()) else {
            return Vec::new();
        };
        let (list, meta) = (&[s::ATTR_LIST][..], std::slice::from_ref(m));
        return list
            .iter()
            .chain(meta)
            .chain(A_LEAVES[i % A_LEAVES.len()])
            .map(|&a| ATOMS[usize::from(a)])
            .collect();
    };
    outer
        .iter()
        .chain(leaf)
        .map(|&a| ATOMS[usize::from(a)])
        .collect()
}

/// An `fnmatch` leaf's kind in pattern `pattern`.
const fn fk(pattern: u16, leaf: u16) -> u16 {
    FNMATCH + pattern * F_LEAVES.len() as u16 + leaf
}

/// An attribute leaf's kind under builtin `meta`.
const fn ak(meta: u16, leaf: u16) -> u16 {
    ATTRIBUTE + meta * A_LEAVES.len() as u16 + leaf
}

/// Lex `b`, a line of Git Ignore without its `\n`, from `state`.
#[inline(never)]
pub fn ignore(b: &[u8], state: &mut State, out: &mut Out) {
    line_format::line::<Ignore, _>(b, state, out, &mut Push);
}

/// [`ignore`]'s lexemes written to `k`, a run sink (X-069).
#[inline(never)]
pub(crate) fn ignore_runs(b: &[u8], state: &mut State, out: &mut Out, k: &mut Runs) {
    line_format::line::<Ignore, _>(b, state, out, k);
}

/// Lex `b`, a line of Git Attributes without its `\n`, from `state`.
#[inline(never)]
pub fn attributes(b: &[u8], state: &mut State, out: &mut Out) {
    line_format::line::<Attributes, _>(b, state, out, &mut Push);
}

/// [`attributes`]'s lexemes written to `k`, a run sink (X-069).
#[inline(never)]
pub(crate) fn attributes_runs(b: &[u8], state: &mut State, out: &mut Out, k: &mut Runs) {
    line_format::line::<Attributes, _>(b, state, out, k);
}

/// Lex `b`, a line of Git Mailmap without its `\n`, from `state`.
#[inline(never)]
pub fn mailmap(b: &[u8], state: &mut State, out: &mut Out) {
    line_format::line::<Mailmap, _>(b, state, out, &mut Push);
}

/// [`mailmap`]'s lexemes written to `k`, a run sink (X-069).
#[inline(never)]
pub(crate) fn mailmap_runs(b: &[u8], state: &mut State, out: &mut Out, k: &mut Runs) {
    line_format::line::<Mailmap, _>(b, state, out, k);
}

/// Lex `b`, a line of Git Link without its `\n`, from `state`.
#[inline(never)]
pub fn link(b: &[u8], state: &mut State, out: &mut Out) {
    line_format::line::<Link, _>(b, state, out, &mut Push);
}

/// [`link`]'s lexemes written to `k`, a run sink (X-069).
#[inline(never)]
pub(crate) fn link_runs(b: &[u8], state: &mut State, out: &mut Out, k: &mut Runs) {
    line_format::line::<Link, _>(b, state, out, k);
}

/// What a line leaves open: an unquoted attributes pattern, or `gitdir`'s separator.
const OPEN: u64 = 1;

/// The text up to `s` is `text`, then `s..e` is `kind`: what of it is on the line (a match
/// can take the line's `\n`, which no span holds).
#[inline(always)]
fn tk<K: Sink>(c: &mut Cursor<K>, text: Option<u16>, s: usize, e: usize, kind: u16) {
    let e = e.min(c.len());
    c.tok(text, s, e, kind);
}

/// The text up to `to`, or the line's end where `to` is past it, is `text`.
#[inline(always)]
fn upto<K: Sink>(c: &mut Cursor<K>, text: Option<u16>, to: usize) {
    let to = to.min(c.len());
    c.text(text, to);
}

/// `\w` at `i`; the line's `\n` isn't one.
#[inline(always)]
fn word(b: &[u8], i: usize) -> bool {
    b.get(i).is_some_and(|&c| is_word(c))
}

/// `\b` at `i`.
#[inline(always)]
fn boundary(b: &[u8], i: usize) -> bool {
    (i > 0 && word(b, i - 1)) != word(b, i)
}

/// The length of the character at `i` (before the line's end), for `.`.
#[inline(always)]
fn char_len(b: &[u8], i: usize) -> usize {
    if b[i] < 0x80 {
        1
    } else {
        Line { b }.char_len(i).unwrap_or(1)
    }
}

// --- fnmatch patterns ---

/// The bytes where an `fnmatch` body's patterns can start, or where its pattern ends: `[`,
/// `/`, `.`, `*`, `?`, `\`, `$`, `:`, and in an attributes pattern `"` and the spaces.
static FNMATCH_STOPS: [bool; 256] = crate::prim::stops(b"[/.*?\\$:\" \t\x0b\x0c\r");
/// An ignore pattern's: it runs to the line's end, so `"` and the spaces are its text.
static IGNORE_STOPS: [bool; 256] = crate::prim::stops(b"[/.*?\\$:");

/// An `fnmatch` pattern of kind `pat` (`IN_IGNORE`, `IN_QUOTED`, `IN_UNQUOTED`) from `p`, from
/// its start (`fnmatch-start`'s drive, `~` and `.`/`..`) where `start`: where what follows it
/// goes on, the line's end and past for a pattern its `$` closes; `None` for an unquoted one
/// still open past the line's `\n`.
// one copy for the three kinds of pattern: inlined at each of Git Attributes' four places it
// took 30 KB more, and a copy a kind 16 KB
#[inline(never)]
fn pattern<K: Sink>(c: &mut Cursor<K>, mut p: usize, start: bool, pat: u16) -> Option<usize> {
    let b = c.b;
    let n = b.len();
    let text = Some(fk(pat, F_TEXT));
    let stops = if pat == IN_IGNORE {
        &IGNORE_STOPS
    } else {
        &FNMATCH_STOPS
    };
    if start {
        p = dir_start(c, p, b'/', fk(pat, F_DRIVE), fk(pat, F_HOME), text)
            .unwrap_or_else(|| dir_pattern(c, p, b'/', fk(pat, F_PARENT), fk(pat, F_SELF), text));
    }
    // `\.(?!.*[./])`: the line's last `.` or `/`, once looked for
    let mut last_dot: Option<Option<usize>> = None;
    loop {
        // the pattern's own end
        match pat {
            // `(?m:$)`
            IN_IGNORE if p >= n => {
                upto(c, text, p);
                return Some(p);
            }
            IN_QUOTED if p < n && b[p] == b'"' => {
                tk(c, text, p, p + 1, QUOTE_END);
                return Some(p + 1);
            }
            // `(?m:$)\n?`
            IN_QUOTED if p >= n => {
                upto(c, text, n);
                return Some(n + 1);
            }
            // `(?=\s)`, the `\n` among them
            IN_UNQUOTED if p == n || (p < n && is_space(b[p])) => {
                upto(c, text, p);
                return Some(p);
            }
            // past the `\n`: nothing matches, and the pattern goes on to the next line
            IN_UNQUOTED if p > n => {
                upto(c, text, n);
                return None;
            }
            _ => {}
        }
        // `fnmatch-body`, before the `\n`
        let mut q = p;
        while q < n && !stops[usize::from(b[q])] {
            q += 1;
        }
        if q >= n {
            p = n;
            continue;
        }
        p = match b[q] {
            b'[' => {
                tk(c, text, q, q + 1, fk(pat, F_CLASS_BEGIN));
                class(c, q + 1, pat)
            }
            b'/' => {
                tk(c, text, q, q + 1, fk(pat, F_PATH_SEP));
                dir_pattern(c, q + 1, b'/', fk(pat, F_PARENT), fk(pat, F_SELF), text)
            }
            b'.' => {
                let last = *last_dot
                    .get_or_insert_with(|| b.iter().rposition(|&c| c == b'.' || c == b'/'));
                if last == Some(q) {
                    tk(c, text, q, q + 1, fk(pat, F_EXTENSION));
                }
                q + 1
            }
            b'*' | b'?' => {
                tk(c, text, q, q + 1, fk(pat, F_ASTERISK));
                q + 1
            }
            // `\\[^$*?]`, the `\n` among what follows
            b'\\' => match b.get(q + 1) {
                Some(b'$' | b'*' | b'?') => q + 1,
                None => {
                    tk(c, text, q, q + 2, fk(pat, F_ESCAPE));
                    q + 2
                }
                Some(_) => {
                    let e = q + 1 + char_len(b, q + 1);
                    tk(c, text, q, e, fk(pat, F_ESCAPE));
                    e
                }
            },
            // `\$\w+`
            b'$' => {
                let mut e = q + 1;
                while word(b, e) {
                    e += 1;
                }
                if e > q + 1 {
                    tk(c, text, q, e, fk(pat, F_ENV));
                }
                e.max(q + 1)
            }
            b':' => {
                tk(c, text, q, q + 1, fk(pat, F_BAD_DRIVE));
                q + 1
            }
            // `"` and the spaces: the end of a pattern they end, at its turn
            ch => {
                let ends = match pat {
                    IN_QUOTED => ch == b'"',
                    IN_UNQUOTED => is_space(ch),
                    _ => false,
                };
                q + usize::from(!ends)
            }
        };
    }
}

/// `fnmatch-char-class-body` and the two before it, from `p` past the `[`: where the class
/// ends, past the line's `\n` where the line ends it.
#[inline(always)]
fn class<K: Sink>(c: &mut Cursor<K>, mut p: usize, pat: u16) -> usize {
    let b = c.b;
    let n = b.len();
    let text = Some(fk(pat, F_CLASS));
    // `[!^]?`
    if p < n && (b[p] == b'!' || b[p] == b'^') {
        tk(c, text, p, p + 1, fk(pat, F_CLASS_LOGICAL));
        p += 1;
    }
    // `\]?`
    if p < n && b[p] == b']' {
        tk(c, text, p, p + 1, fk(pat, F_CLASS_CHAR));
        p += 1;
    }
    loop {
        // `(?m:$)\n?`
        if p >= n {
            upto(c, text, n);
            return n + 1;
        }
        match b[p] {
            b']' => {
                tk(c, text, p, p + 1, fk(pat, F_CLASS_END));
                return p + 1;
            }
            b'\\' if b.get(p + 1) == Some(&b']') => {
                tk(c, text, p, p + 2, fk(pat, F_CLASS_ESCAPE));
                p += 2;
            }
            b'*' | b'?' => {
                tk(c, text, p, p + 1, fk(pat, F_CLASS_BAD));
                p += 1;
            }
            ch if !is_space(ch) => {
                let e = p + char_len(b, p);
                tk(c, text, p, e, fk(pat, F_CLASS_CHAR));
                p = e;
            }
            _ => p += 1,
        }
    }
}

/// `fnmatch-start`'s and `path-start`'s own patterns at `p`, a drive or `~` (and `slash`, the
/// slash a drive takes besides `/`), giving the `:` kind `drive` and the `~` kind `home`:
/// where they end, or `None`.
#[inline(always)]
fn dir_start<K: Sink>(
    c: &mut Cursor<K>,
    p: usize,
    slash: u8,
    drive: u16,
    home: u16,
    text: Option<u16>,
) -> Option<usize> {
    let b = c.b;
    let is_slash = |i: usize| b.get(i).is_some_and(|&c| c == b'/' || c == slash);
    // `[A-Za-z](:)(?=/)`
    if b.get(p).is_some_and(u8::is_ascii_alphabetic)
        && b.get(p + 1) == Some(&b':')
        && is_slash(p + 2)
    {
        tk(c, text, p + 1, p + 2, drive);
        return Some(p + 2);
    }
    // `~(?=/)`
    if b.get(p) == Some(&b'~') && b.get(p + 1) == Some(&b'/') {
        tk(c, text, p, p + 1, home);
        return Some(p + 1);
    }
    None
}

/// A `dir-pattern` context's patterns at `p`: `..` (kind `parent`) or `.` (kind `this`)
/// before a slash, else nothing: where it ends.
#[inline(always)]
fn dir_pattern<K: Sink>(
    c: &mut Cursor<K>,
    p: usize,
    slash: u8,
    parent: u16,
    this: u16,
    text: Option<u16>,
) -> usize {
    let b = c.b;
    if b.get(p) != Some(&b'.') {
        return p;
    }
    let is_slash = |i: usize| b.get(i).is_some_and(|&c| c == b'/' || c == slash);
    if b.get(p + 1) == Some(&b'.') && is_slash(p + 2) {
        tk(c, text, p, p + 2, parent);
        p + 2
    } else if is_slash(p + 1) {
        tk(c, text, p, p + 1, this);
        p + 1
    } else {
        p
    }
}

// --- Git Ignore ---

struct Ignore;

impl Format for Ignore {
    // `([#;]).*\n?` past the blanks
    const LEAD: [u8; 256] = leads(b"#;", b"");
    const COMMENT_TEXT: u16 = COMMENT;
    const COMMENT_MARK: u16 = COMMENT_MARK;
    const COMMENT_BLANKS: bool = false;
    const OPENS: bool = false;

    fn resume<K: Sink>(_: &mut Cursor<K>, _: u64) -> u64 {
        unreachable!("Git Ignore's lines leave nothing open")
    }

    /// `!|(?=\S)`, then the pattern to the line's end.
    #[inline(always)]
    fn entry<K: Sink>(c: &mut Cursor<K>, q: usize) -> u64 {
        let negate = c.b[q] == b'!';
        tk(c, None, q, q + usize::from(negate), fk(IN_IGNORE, F_NEGATE));
        pattern(c, q + usize::from(negate), true, IN_IGNORE);
        0
    }
}

// --- Git Attributes ---

struct Attributes;

impl Format for Attributes {
    // `^\s*([#;]).*\n?`, and `[` for a macro
    const LEAD: [u8; 256] = leads(b"#;", b"[");
    const COMMENT_TEXT: u16 = COMMENT;
    const COMMENT_MARK: u16 = COMMENT_MARK;
    const COMMENT_BLANKS: bool = true;

    /// An unquoted pattern the line before left open, then its attributes.
    #[inline(always)]
    fn resume<K: Sink>(c: &mut Cursor<K>, _: u64) -> u64 {
        match pattern(c, 0, false, IN_UNQUOTED) {
            Some(p) => attribute_list(c, p),
            None => OPEN,
        }
    }

    /// `(\[)([^-!]\S*)(\])` and its attributes, or `None` for a `[` that starts a pattern.
    #[inline(always)]
    fn section<K: Sink>(c: &mut Cursor<K>, q: usize) -> Option<u64> {
        let j = macro_end(c.b, q)?;
        tk(c, None, q, q + 1, BRACKETS_BEGIN);
        tk(c, None, q + 1, j, MACRO);
        tk(c, None, j, j + 1, BRACKETS_END);
        Some(attribute_list(c, j + 1))
    }

    /// `[-!\[]|(?=\S)`, a pattern, then its attributes.
    #[inline(always)]
    fn entry<K: Sink>(c: &mut Cursor<K>, q: usize) -> u64 {
        let b = c.b;
        let n = usize::from(matches!(b[q], b'-' | b'!' | b'['));
        tk(c, None, q, q + n, BAD_OPERATOR);
        let p = q + n;
        // `pattern-content`
        let end = if b.get(p) == Some(&b'"') {
            tk(c, None, p, p + 1, QUOTE_BEGIN);
            pattern(c, p + 1, true, IN_QUOTED)
        } else {
            pattern(c, p, true, IN_UNQUOTED)
        };
        match end {
            Some(p) => attribute_list(c, p),
            None => OPEN,
        }
    }
}

/// `(\[)([^-!]\S*)(\])` at `k`, a `[`: where the name's `]` is (it gives back to its last one).
fn macro_end(b: &[u8], k: usize) -> Option<usize> {
    let l = Line { b };
    if l.at(k + 1).is_none_or(|c| c == b'-' || c == b'!') {
        return None;
    }
    let first = l.char_len(k + 1).unwrap_or(1);
    let run = l.run(k + 1 + first, |c| !is_space(c));
    (k + 1 + first..run).rev().find(|&j| b[j] == b']')
}

/// `attributes-list` from `p` to the line's end: what the line leaves open.
// one copy, for Git Attributes' three places (an entry, a macro, a line resumed)
#[inline(never)]
fn attribute_list<K: Sink>(c: &mut Cursor<K>, mut p: usize) -> u64 {
    let b = c.b;
    let n = b.len();
    let text = Some(LIST);
    loop {
        // `(?m:$)\n?`
        if p >= n {
            upto(c, text, n);
            return 0;
        }
        let ch = b[p];
        // `[-!](?=\w)`
        if matches!(ch, b'-' | b'!') && word(b, p + 1) {
            tk(c, text, p, p + 1, LIST_LOGICAL);
            p += 1;
        } else if is_word(ch) {
            c.text(text, p);
            p = attribute(c, p, builtin(b, p));
        } else if is_space(ch) {
            // `\s+`, through the `\n`
            let mut e = p + 1;
            while e < n && is_space(b[e]) {
                e += 1;
            }
            tk(c, text, p, e, LIST_SPACES);
            p = if e == n { n + 1 } else { e };
        } else {
            // `\S+`
            let mut e = p + 1;
            while e < n && !is_space(b[e]) {
                e += 1;
            }
            tk(c, text, p, e, LIST_BAD_NAME);
            p = e;
        }
    }
}

/// The builtin an attribute's name at `p` names, by `META_DIFF`'s order: `(?=diff\b)` and the
/// others, else `other`.
#[inline(always)]
fn builtin(b: &[u8], p: usize) -> u16 {
    let named = |w: &[u8]| b[p..].starts_with(w) && boundary(b, p + w.len());
    match b[p] {
        b'd' if named(b"diff") => 0,
        b'e' if named(b"encoding") => 1,
        b'e' if named(b"eol") => 2,
        b'f' if named(b"filter") => 3,
        b'm' if named(b"merge") => 4,
        b't' if named(b"text") => 6,
        b'w' if named(b"whitespace") => 7,
        _ => 5,
    }
}

/// An attribute at `p`, a word, under builtin `meta`: its key, `=` and its values, or the
/// space after it: where the list goes on.
#[inline(always)]
fn attribute<K: Sink>(c: &mut Cursor<K>, mut p: usize, meta: u16) -> usize {
    let b = c.b;
    let n = b.len();
    let text = Some(ak(meta, A_NONE));
    // `attribute-key`: `\w+[-\w]*\b`, the run giving back to a `\b`
    let mut run = p + 1;
    while run < n && (is_word(b[run]) || b[run] == b'-') {
        run += 1;
    }
    let mut e = run;
    while e > p + 1 && !boundary(b, e) {
        e -= 1;
    }
    tk(c, text, p, e, ak(meta, A_KEY));
    p = e;
    // `attribute-maybe-value-list`
    let mapping = Some(ak(meta, A_MAPPING));
    match b.get(p) {
        Some(b'=') => {
            tk(c, mapping, p, p + 1, ak(meta, A_KEY_VALUE));
            // `=(?=\S)`, else `=`
            p = if p + 1 < n && !is_space(b[p + 1]) {
                values(c, p + 1, meta)
            } else {
                expected(c, p + 1, meta)
            };
        }
        // `attribute-separator`: `\s`, the `\n` among them
        None => {
            c.text(mapping, n);
            p = n + 1;
        }
        Some(&ch) if is_space(ch) => {
            tk(c, mapping, p, p + 1, ak(meta, A_PAIR));
            p += 1;
        }
        // `else-pop`
        Some(_) => c.text(mapping, p),
    }
    // the builtin's context pops
    upto(c, text, p);
    p
}

/// `attribute-value-expected` at `p`: the text to the next `\s`, which it takes.
#[inline(always)]
fn expected<K: Sink>(c: &mut Cursor<K>, p: usize, meta: u16) -> usize {
    let b = c.b;
    let mut i = p;
    while i < b.len() && !is_space(b[i]) {
        i += 1;
    }
    tk(c, Some(ak(meta, A_NONE)), i, i + 1, ak(meta, A_EXPECT_PAIR));
    i + 1
}

/// `attribute-value-list` from `p`, a value's start: its values and the space after them.
#[inline(always)]
fn values<K: Sink>(c: &mut Cursor<K>, p: usize, meta: u16) -> usize {
    let b = c.b;
    let n = b.len();
    let text = Some(ak(meta, A_VALUE));
    let mut p = value(c, p, meta);
    loop {
        match b.get(p) {
            // `\s`, the `\n` among them
            None if p == n => {
                c.text(text, n);
                return n + 1;
            }
            Some(&ch) if is_space(ch) => {
                tk(c, text, p, p + 1, ak(meta, A_PAIR));
                return p + 1;
            }
            Some(b',') => {
                tk(c, text, p, p + 1, ak(meta, A_VALUE_SEP));
                // `,(?=\S)`, else `,`
                if p + 1 < n && !is_space(b[p + 1]) {
                    p = value(c, p + 1, meta);
                } else {
                    return expected(c, p + 1, meta);
                }
            }
            // `\S`
            Some(_) => {
                let e = p + char_len(b, p);
                tk(c, text, p, e, ak(meta, A_BAD_VALUE));
                p = e;
            }
            // `else-pop`, past the `\n`
            None => {
                upto(c, text, p);
                return p;
            }
        }
    }
}

/// `attribute-value-content` at `p`: a quoted value or `[^,\s]*`, where it ends.
#[inline(always)]
fn value<K: Sink>(c: &mut Cursor<K>, p: usize, meta: u16) -> usize {
    let b = c.b;
    let n = b.len();
    let text = Some(ak(meta, A_VALUE));
    if b.get(p) != Some(&b'"') {
        let mut e = p;
        while e < n && b[e] != b',' && !is_space(b[e]) {
            e += 1;
        }
        tk(c, text, p, e, ak(meta, A_UNQUOTED));
        return e;
    }
    tk(c, text, p, p + 1, ak(meta, A_BEGIN));
    // `#anon_attribute-value-content_0`
    let text = Some(ak(meta, A_DOUBLE));
    let mut p = p + 1;
    loop {
        let q = find(b, p, *b"\"\\");
        // `(?m:$)\n?`
        if q >= n {
            c.text(text, n);
            return n + 1;
        }
        if b[q] == b'"' {
            tk(c, text, q, q + 1, ak(meta, A_END));
            return q + 1;
        }
        // `\\[\\\"\'fnrst]`
        if b.get(q + 1).is_some_and(|c| b"\\\"'fnrst".contains(c)) {
            tk(c, text, q, q + 2, ak(meta, A_ESCAPE));
            p = q + 2;
        } else {
            p = q + 1;
        }
    }
}

// --- Git Mailmap ---

struct Mailmap;

impl Format for Mailmap {
    // `[#;]` and the rest of the line
    const LEAD: [u8; 256] = leads(b"#;", b"");
    const COMMENT_TEXT: u16 = COMMENT;
    const COMMENT_MARK: u16 = COMMENT_MARK;
    const COMMENT_BLANKS: bool = false;
    const OPENS: bool = false;

    fn resume<K: Sink>(_: &mut Cursor<K>, _: u64) -> u64 {
        unreachable!("Git Mailmap's lines leave nothing open")
    }

    /// A line of blanks: past spaces and tabs, a name can start at any other.
    #[inline(always)]
    fn blank<K: Sink>(c: &mut Cursor<K>) {
        if c.b.iter().any(|&ch| !is_blank(ch)) {
            mailmap_main(c);
        }
    }

    /// `main` from the line's start: a name can start at a blank past spaces and tabs.
    #[inline(always)]
    fn entry<K: Sink>(c: &mut Cursor<K>, _: usize) -> u64 {
        mailmap_main(c);
        0
    }
}

/// Git Mailmap's `main` over the line: names, e-mail addresses, a comment.
fn mailmap_main<K: Sink>(c: &mut Cursor<K>) {
    let b = c.b;
    let n = b.len();
    let mut p = 0;
    // `user` can't start before this: a failed attempt's text runs to it
    let mut no_user = 0;
    // for `<(?=\S+?>)`: the first `>` or space at or after `.0` is at `.1`
    let mut stop = (usize::MAX, 0);
    while p < n {
        match b[p] {
            b'#' | b';' => {
                tk(c, None, p, p + 1, COMMENT_MARK);
                tk(c, None, p + 1, n, COMMENT);
                return;
            }
            b'<' if email_open(b, p, &mut stop) => {
                tk(c, None, p, p + 1, EMAIL_BEGIN);
                p = email(c, p + 1);
                continue;
            }
            b' ' | b'\t' | b'<' => {
                p += 1;
                continue;
            }
            _ => {}
        }
        if p >= no_user {
            match user(b, p) {
                Ok(e) => {
                    tk(c, None, p, e, USER);
                    p = e;
                    continue;
                }
                Err(run) => no_user = run + 1,
            }
        }
        p += 1;
    }
}

/// `<(?=\S+?>)` at `k`, a `<`: a `>` after it with no space before, but a character.
#[inline(always)]
fn email_open(b: &[u8], k: usize, stop: &mut (usize, usize)) -> bool {
    if k + 1 >= b.len() || is_space(b[k + 1]) {
        return false;
    }
    let from = k + 2;
    let j = if stop.0 <= from && from <= stop.1 {
        stop.1
    } else {
        let mut j = from;
        while j < b.len() && b[j] != b'>' && !is_space(b[j]) {
            j += 1;
        }
        *stop = (from, j);
        j
    };
    j < b.len() && b[j] == b'>'
}

/// `[^ \t<#][^<#\n]+?(?=[ \t]*(<|(?m:$)))` at `p`: where it ends, or where the run of text
/// it could take ends, as no later start in that run matches either.
///
/// The run stops at a `<`, a `#` or the line's end, and the blanks after an end can only reach
/// a `<` or the end: so the match ends where the run's last blanks start (or a character in),
/// unless a `#` ends the run.
#[inline(always)]
fn user(b: &[u8], p: usize) -> Result<usize, usize> {
    let from = (p + 1).min(b.len());
    let run = find(b, from, *b"<#");
    if run < b.len() && b[run] == b'#' {
        return Err(run);
    }
    let mut t = run;
    while t > p + 1 && is_blank(b[t - 1]) {
        t -= 1;
    }
    let e = t.max(p + 2);
    if e <= run { Ok(e) } else { Err(run) }
}

/// `email-name` and `email-domain` from `p`, past the `<`: where the address ends, past its
/// `>` (which `<(?=\S+?>)` has seen).
#[inline(always)]
fn email<K: Sink>(c: &mut Cursor<K>, mut p: usize) -> usize {
    let b = c.b;
    let text = Some(EMAIL);
    let mut domain = false;
    loop {
        let q = find(b, p, *b">@.");
        match b.get(q) {
            Some(b'>') => {
                tk(c, text, q, q + 1, EMAIL_END);
                return q + 1;
            }
            Some(b'@') if !domain => {
                tk(c, text, q, q + 1, EMAIL_SEP);
                domain = true;
            }
            Some(b'.') if domain => tk(c, text, q, q + 1, DOMAIN_SEP),
            Some(_) => {}
            None => {
                c.text(text, b.len());
                return b.len();
            }
        }
        p = q + 1;
    }
}

// --- Git Link ---

struct Link;

impl Format for Link {
    // `^\s*([#;]).*\n?`
    const LEAD: [u8; 256] = leads(b"#;", b"");
    const COMMENT_TEXT: u16 = COMMENT;
    const COMMENT_MARK: u16 = COMMENT_MARK;
    const COMMENT_BLANKS: bool = true;

    /// `gitdir` the line before, still waiting for its `:`.
    #[inline(always)]
    fn resume<K: Sink>(c: &mut Cursor<K>, _: u64) -> u64 {
        expect_separator(c, 0)
    }

    /// `\bgitdir\b`, or `\S+` each that isn't one.
    #[inline(always)]
    fn entry<K: Sink>(c: &mut Cursor<K>, mut q: usize) -> u64 {
        let b = c.b;
        loop {
            // the byte before `q` is a blank, or `q` is the line's start
            if b[q..].starts_with(b"gitdir") && !word(b, q + 6) {
                tk(c, None, q, q + 6, GITDIR);
                return expect_separator(c, q + 6);
            }
            let mut e = q + 1;
            while e < b.len() && !is_space(b[e]) {
                e += 1;
            }
            tk(c, None, q, e, GITDIR_EXPECTED);
            q = blanks(b, e);
            if q >= b.len() {
                return 0;
            }
        }
    }
}

/// `expect-separator` from `p`: what the line leaves open.
#[inline(never)]
fn expect_separator<K: Sink>(c: &mut Cursor<K>, mut p: usize) -> u64 {
    let b = c.b;
    let text = Some(LINK_KEY);
    loop {
        if p >= b.len() {
            c.text(text, b.len());
            return OPEN;
        }
        match b[p] {
            b':' => {
                c.text(text, p);
                separator(c, p);
                return 0;
            }
            // `\S`
            ch if !is_space(ch) => {
                let e = p + char_len(b, p);
                tk(c, text, p, e, SEPARATOR_EXPECTED);
                p = e;
            }
            _ => p += 1,
        }
    }
}

/// `separator` at `p`, a `:`, then `expect-path`.
#[inline(always)]
fn separator<K: Sink>(c: &mut Cursor<K>, p: usize) {
    let b = c.b;
    tk(c, Some(LINK_MAPPING), p, p + 1, LINK_KEY_VALUE);
    let text = Some(LINK_VALUE);
    let p = blanks(b, p + 1);
    c.text(text, p);
    if p < b.len() {
        path(c, p);
    }
}

/// `path-start`, then `path`, from `p` to the line's end.
#[inline(always)]
fn path<K: Sink>(c: &mut Cursor<K>, p: usize) {
    let b = c.b;
    let n = b.len();
    let text = Some(LINK_PATH);
    let mut p = dir_start(c, p, b'\\', LINK_DRIVE, LINK_HOME, text)
        .unwrap_or_else(|| dir_pattern(c, p, b'\\', LINK_PARENT, LINK_SELF, text));
    loop {
        let mut q = p;
        while q < n && !matches!(b[q], b'\\' | b'/' | b'*' | b'?') && !(b[q] == b'.' && q + 1 == n)
        {
            q += 1;
        }
        // `\.?\n`
        if q >= n {
            c.text(text, n);
            return;
        }
        match b[q] {
            b'.' => {
                tk(c, text, q, n, LINK_TERMINATOR);
                return;
            }
            b'*' | b'?' => {
                tk(c, text, q, q + 1, LINK_BAD_PATH);
                p = q + 1;
            }
            _ => {
                tk(c, text, q, q + 1, LINK_PATH_SEP);
                p = dir_pattern(c, q + 1, b'\\', LINK_PARENT, LINK_SELF, text);
            }
        }
    }
}
