//! Git's own files by a scanner: bat's `Git Ignore`, `Git Attributes`, `Git Mailmap` and
//! `Git Link`, with the parts of `Git Common` they share, compiled by hand. The port (`git.rs`)
//! runs their 56 contexts on the machine, which sets up ~2 KB of state a line and asks every
//! pattern in reach for its leftmost match through a function pointer, most of them searches;
//! here each context that can be open is an arm of one `match`, testing its patterns at the scan's
//! place in the grammar's order, and the text where none can start is passed whole.
//!
//! The stack holds the port's context ids and packs into the same [`State`], so the port is an
//! exact reference: the tests hold every byte's scopes and every line's state to it. A span's
//! kind is fixed ([`scopes`]): a scope list of its own, an `fnmatch` leaf under the pattern it is
//! in (an ignore pattern, a quoted or an unquoted attributes pattern), or an attribute's leaf
//! under its builtin's meta scope.

use std::cell::Cell;

use crate::git::{
    ATOMS, ATTR_MAIN_0, ATTR_START, ATTRIBUTE_KEY, ATTRIBUTES_LIST, CLASS_BODY, CLASS_OPERATOR,
    CLASS_START, DIR_PATTERN, EMAIL_DOMAIN, EMAIL_META, EMAIL_NAME, EXPECT_PATH, EXPECT_SEPARATOR,
    FNMATCH_START, IGNORE_MAIN_0, IGNORE_PATTERN, IGNORE_START, LINK_MAIN_0, LINK_START,
    MAILMAP_MAIN_0, MAILMAP_START, MAYBE_VALUE_LIST, META_DIFF, META_ENCODING, META_EOL,
    META_FILTER, META_MERGE, META_OTHER, META_TEXT, META_WHITESPACE, PATH, PATH_DIR_PATTERN,
    PATH_START, PATTERN_ATTRIBUTES, PATTERN_CONTENT, PATTERN_QUOTED, PATTERN_UNQUOTED, SEPARATOR,
    VALUE_CONTENT, VALUE_CONTENT_0, VALUE_EXPECTED, VALUE_LIST, boundary, comment_line, s, word,
};
use crate::machine::{Atom, Line, is_blank, is_space};
use crate::{Out, State};

/// Which of the four syntaxes a line is in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Git {
    Ignore,
    Attributes,
    Mailmap,
    Link,
}

impl Git {
    /// The stack a block starts with, once `__start` has pushed `__main`.
    fn start(self) -> [u8; 2] {
        match self {
            Git::Ignore => [IGNORE_START, IGNORE_MAIN_0],
            Git::Attributes => [ATTR_START, ATTR_MAIN_0],
            Git::Mailmap => [MAILMAP_START, MAILMAP_MAIN_0],
            Git::Link => [LINK_START, LINK_MAIN_0],
        }
    }
}

/// The kinds with a scope list of their own, by kind; [`FNMATCH`] and [`ATTRIBUTE`] follow.
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

const NONE: u16 = 0;
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
const EMAIL_META_TEXT: u16 = 15;
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
/// the meta scope of the attribute's builtin (by [`META_DIFF`]'s order).
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

/// The most levels a line holds: the deepest stack the grammars reach is 7.
const DEPTH: usize = 32;

/// A line on its way through the scanner.
struct Scan<'a, 'b, 'c> {
    l: Line<'a>,
    stack: [u8; DEPTH],
    depth: usize,
    /// The `fnmatch` pattern open (`IN_IGNORE`…), for the contexts above it.
    pattern: u16,
    /// The attribute's builtin open, by [`META_DIFF`]'s order, for the contexts above it.
    meta: u16,
    out: &'b mut Out<'c>,
    /// Where the spans so far end.
    at: usize,
    /// The last `.` or `/` of the line, for `\.(?!.*[./])`, once looked for.
    last_dot: Option<Option<usize>>,
    /// `user` can't start before this: a failed attempt's text runs to it.
    no_user: usize,
    /// For `<(?=\S+?>)`: the first `>` or space at or after `.0` is at `.1`.
    stop: (usize, usize),
}

/// Lex `b`, a line of `git` without its `\n`, from `state`.
#[inline(never)]
pub fn line(git: Git, b: &[u8], state: &mut State, out: &mut Out) {
    let mut scan = Scan {
        l: Line {
            b,
            arg: Cell::new(0),
        },
        stack: [0; DEPTH],
        depth: 0,
        pattern: IN_IGNORE,
        meta: 0,
        out,
        at: 0,
        last_dot: None,
        no_user: 0,
        stop: (usize::MAX, 0),
    };
    match state.levels() {
        [] => {
            scan.stack[..2].copy_from_slice(&git.start());
            scan.depth = 2;
        }
        levels => {
            scan.stack[..levels.len()].copy_from_slice(levels);
            scan.depth = levels.len();
            for &c in levels {
                scan.note(c);
            }
        }
    }
    scan.run();
    let nl = b.len();
    scan.flush(nl);
    let depth = scan.depth;
    *state = State::pack(
        &scan.stack[..depth],
        &[0; DEPTH][..depth],
        0,
        state.is_exact(),
    );
}

/// The bytes where something can start in an `fnmatch` pattern, or end one: `[`, `/`, `.`, `*`,
/// `?`, `\`, `$`, `:`, `"` and the spaces.
static FNMATCH_STOPS: [bool; 256] = {
    let mut t = [false; 256];
    let bytes = b"[/.*?\\$:\" \t\x0b\x0c\r";
    let mut i = 0;
    while i < bytes.len() {
        t[bytes[i] as usize] = true;
        i += 1;
    }
    t
};

impl Scan<'_, '_, '_> {
    fn top(&self) -> u8 {
        self.stack[self.depth - 1]
    }

    /// Context `c` is opened: what it says about the contexts above it.
    fn note(&mut self, c: u8) {
        match c {
            IGNORE_PATTERN => self.pattern = IN_IGNORE,
            PATTERN_QUOTED => self.pattern = IN_QUOTED,
            PATTERN_UNQUOTED => self.pattern = IN_UNQUOTED,
            META_DIFF..=META_WHITESPACE => self.meta = u16::from(c - META_DIFF),
            _ => {}
        }
    }

    fn push(&mut self, c: u8) {
        // the grammars nest 7 deep at most; a state from elsewhere can't take it past this
        if self.depth < DEPTH {
            self.stack[self.depth] = c;
            self.depth += 1;
        }
        self.note(c);
    }

    fn pop(&mut self) {
        self.depth -= 1;
    }

    fn set(&mut self, c: u8) {
        self.depth -= 1;
        self.push(c);
    }

    /// The kind of the open context's text.
    fn text_kind(&self) -> u16 {
        match self.top() {
            IGNORE_PATTERN | PATTERN_QUOTED | PATTERN_UNQUOTED | FNMATCH_START | DIR_PATTERN => {
                fk(self.pattern, F_TEXT)
            }
            CLASS_BODY | CLASS_START | CLASS_OPERATOR => fk(self.pattern, F_CLASS),
            ATTRIBUTES_LIST => LIST,
            META_DIFF..=META_WHITESPACE | ATTRIBUTE_KEY | VALUE_EXPECTED => ak(self.meta, A_NONE),
            MAYBE_VALUE_LIST => ak(self.meta, A_MAPPING),
            VALUE_LIST | VALUE_CONTENT => ak(self.meta, A_VALUE),
            VALUE_CONTENT_0 => ak(self.meta, A_DOUBLE),
            EMAIL_META => EMAIL_META_TEXT,
            EMAIL_NAME | EMAIL_DOMAIN => EMAIL,
            EXPECT_SEPARATOR => LINK_KEY,
            SEPARATOR => LINK_MAPPING,
            EXPECT_PATH => LINK_VALUE,
            PATH | PATH_START | PATH_DIR_PATTERN => LINK_PATH,
            _ => NONE,
        }
    }

    /// The open context's text runs to `to` (never past the line).
    fn flush(&mut self, to: usize) {
        let to = to.min(self.l.nl());
        if to > self.at {
            let kind = self.text_kind();
            if kind != NONE {
                self.out.push_merged(self.at, to, kind);
            }
            self.at = to;
        }
    }

    /// The text up to `s`, then `s..e` is `kind` (what the line holds of it).
    fn tok(&mut self, s: usize, e: usize, kind: u16) {
        self.flush(s);
        let e = e.min(self.l.nl());
        if e > self.at {
            self.out.push_merged(self.at, e, kind);
            self.at = e;
        }
    }

    fn at(&self, i: usize) -> Option<u8> {
        self.l.at(i)
    }

    fn run(&mut self) {
        let nl = self.l.nl();
        let end = self.l.end();
        let mut p = 0;
        while p <= end && self.depth > 0 {
            p = match self.top() {
                IGNORE_MAIN_0 => self.ignore_main(p),
                ATTR_MAIN_0 => self.attributes_main(p),
                MAILMAP_MAIN_0 => self.mailmap_main(p),
                LINK_MAIN_0 => self.link_main(p),
                // `fnmatch-start`
                FNMATCH_START => {
                    let pattern = self.pattern;
                    let f = |leaf| fk(pattern, leaf);
                    let q = self.dir_start(p, b'/', f(F_DRIVE), f(F_HOME));
                    if q == p {
                        self.dir_pattern(p, b'/', f(F_PARENT), f(F_SELF))
                    } else {
                        q
                    }
                }
                DIR_PATTERN => {
                    let pattern = self.pattern;
                    let f = |leaf| fk(pattern, leaf);
                    self.dir_pattern(p, b'/', f(F_PARENT), f(F_SELF))
                }
                IGNORE_PATTERN => {
                    // `(?m:$)`, at the `\n` or past it
                    if p >= nl {
                        self.flush(p);
                        self.pop();
                        p
                    } else {
                        self.fnmatch_body(p)
                    }
                }
                PATTERN_QUOTED => match self.at(p) {
                    Some(b'"') => {
                        self.tok(p, p + 1, QUOTE_END);
                        self.pop();
                        p + 1
                    }
                    // `(?m:$)\n?`
                    _ if p >= nl => {
                        self.flush(p);
                        self.pop();
                        end
                    }
                    _ => self.fnmatch_body(p),
                },
                PATTERN_UNQUOTED => {
                    // `(?=\s)`
                    if self.l.is(p, is_space) {
                        self.flush(p);
                        self.pop();
                        p
                    } else {
                        self.fnmatch_body(p)
                    }
                }
                CLASS_OPERATOR => {
                    // `[!^]?`
                    let n = usize::from(self.l.is(p, |c| c == b'!' || c == b'^'));
                    self.tok(p, p + n, fk(self.pattern, F_CLASS_LOGICAL));
                    self.flush(p);
                    self.pop();
                    p + n
                }
                CLASS_START => {
                    // `\]?`
                    let n = usize::from(self.at(p) == Some(b']'));
                    self.tok(p, p + n, fk(self.pattern, F_CLASS_CHAR));
                    self.flush(p);
                    self.pop();
                    p + n
                }
                CLASS_BODY => self.class_body(p),
                PATTERN_CONTENT => {
                    if self.at(p) == Some(b'"') {
                        self.tok(p, p + 1, QUOTE_BEGIN);
                        self.set(PATTERN_QUOTED);
                        self.push(FNMATCH_START);
                        p + 1
                    } else {
                        self.flush(p);
                        self.set(PATTERN_UNQUOTED);
                        self.push(FNMATCH_START);
                        p
                    }
                }
                PATTERN_ATTRIBUTES => {
                    self.flush(p);
                    self.set(ATTRIBUTES_LIST);
                    p
                }
                ATTRIBUTES_LIST => self.attributes_list(p),
                META_DIFF..=META_WHITESPACE => {
                    self.flush(p);
                    self.pop();
                    p
                }
                ATTRIBUTE_KEY => self.attribute_key(p),
                MAYBE_VALUE_LIST => self.maybe_value_list(p),
                VALUE_EXPECTED => {
                    // `\s`
                    match (p..=nl).find(|&i| self.l.is(i, is_space)) {
                        Some(i) => {
                            self.tok(i, i + 1, ak(self.meta, A_EXPECT_PAIR));
                            self.pop();
                            i + 1
                        }
                        None => end + 1,
                    }
                }
                VALUE_CONTENT => {
                    if self.at(p) == Some(b'"') {
                        self.tok(p, p + 1, ak(self.meta, A_BEGIN));
                        self.set(VALUE_CONTENT_0);
                        p + 1
                    } else {
                        // `[^,\s]*`
                        let e = self.l.run(p, |c| c != b',' && !is_space(c));
                        self.tok(p, e, ak(self.meta, A_UNQUOTED));
                        self.flush(p);
                        self.pop();
                        e
                    }
                }
                VALUE_CONTENT_0 => self.value_string(p),
                VALUE_LIST => self.value_list(p),
                EMAIL_META => {
                    self.flush(p);
                    self.pop();
                    p
                }
                EMAIL_NAME | EMAIL_DOMAIN => self.email(p),
                EXPECT_SEPARATOR => match self.at(p) {
                    // `(?=:)`
                    Some(b':') => {
                        self.flush(p);
                        self.set(SEPARATOR);
                        p
                    }
                    // `\S`
                    Some(c) if p < nl && !is_space(c) => {
                        let e = p + self.l.char_len(p).unwrap_or(1);
                        self.tok(p, e, SEPARATOR_EXPECTED);
                        e
                    }
                    _ => p + 1,
                },
                SEPARATOR => match self.at(p) {
                    Some(b':') => {
                        self.tok(p, p + 1, LINK_KEY_VALUE);
                        self.set(EXPECT_PATH);
                        p + 1
                    }
                    // `\n`
                    _ if p == nl => {
                        self.flush(p);
                        self.pop();
                        end
                    }
                    _ => p + 1,
                },
                EXPECT_PATH => match self.at(p) {
                    // `(?=\S)`
                    Some(c) if p < nl && !is_space(c) => {
                        self.flush(p);
                        self.set(PATH);
                        self.push(PATH_START);
                        p
                    }
                    // `\n`
                    _ if p == nl => {
                        self.flush(p);
                        self.pop();
                        end
                    }
                    _ => p + 1,
                },
                PATH_START => {
                    let q = self.dir_start(p, b'\\', LINK_DRIVE, LINK_HOME);
                    if q == p {
                        self.dir_pattern(p, b'\\', LINK_PARENT, LINK_SELF)
                    } else {
                        q
                    }
                }
                PATH_DIR_PATTERN => self.dir_pattern(p, b'\\', LINK_PARENT, LINK_SELF),
                PATH => self.path(p),
                // a context no line leaves open: nothing matches in it
                _ => end + 1,
            };
        }
    }

    /// `fnmatch-start`'s and `path-start`'s own patterns at `p`, a drive or `~` (and `slash`, the
    /// slash a drive takes besides `/`, or `/` for none), giving the `:` kind `drive` and the `~`
    /// kind `home`: where they end, or `p`.
    fn dir_start(&mut self, p: usize, slash: u8, drive: u16, home: u16) -> usize {
        let is_slash = |c: u8| c == b'/' || c == slash;
        // `[A-Za-z](:)(?=/)`
        if self.l.is(p, |c| c.is_ascii_alphabetic())
            && self.at(p + 1) == Some(b':')
            && self.l.is(p + 2, is_slash)
        {
            self.tok(p + 1, p + 2, drive);
            self.pop();
            return p + 2;
        }
        // `~(?=/)`
        if self.at(p) == Some(b'~') && self.at(p + 1) == Some(b'/') {
            self.tok(p, p + 1, home);
            self.pop();
            return p + 1;
        }
        p
    }

    /// A `dir-pattern` context's patterns at `p`: `..` or `.` before a slash, else nothing; it
    /// pops either way. `..` takes kind `parent`, `.` kind `this`.
    fn dir_pattern(&mut self, p: usize, slash: u8, parent: u16, this: u16) -> usize {
        let is_slash = |c: u8| c == b'/' || c == slash;
        let n = if self.at(p) != Some(b'.') {
            0
        } else if self.at(p + 1) == Some(b'.') && self.l.is(p + 2, is_slash) {
            2
        } else if self.l.is(p + 1, is_slash) {
            1
        } else {
            0
        };
        if n > 0 {
            self.tok(p, p + n, if n == 2 { parent } else { this });
        }
        self.flush(p);
        self.pop();
        p + n
    }

    /// `fnmatch-body`'s patterns at `p`, before the `\n`: where the step ends.
    fn fnmatch_body(&mut self, mut p: usize) -> usize {
        let b = self.l.b;
        while p < b.len() && !FNMATCH_STOPS[usize::from(b[p])] {
            p += 1;
        }
        if p >= b.len() {
            // at the `\n` only the context's own patterns can match; past it nothing
            return if p == b.len() { p } else { self.l.end() + 1 };
        }
        let pattern = self.pattern;
        let f = |leaf| fk(pattern, leaf);
        match b[p] {
            b'[' => {
                self.tok(p, p + 1, f(F_CLASS_BEGIN));
                self.push(CLASS_BODY);
                self.push(CLASS_START);
                self.push(CLASS_OPERATOR);
            }
            b'/' => {
                self.tok(p, p + 1, f(F_PATH_SEP));
                self.push(DIR_PATTERN);
            }
            // `\.(?!.*[./])`
            b'.' => {
                let last = *self
                    .last_dot
                    .get_or_insert_with(|| b.iter().rposition(|&c| c == b'.' || c == b'/'));
                if last == Some(p) {
                    self.tok(p, p + 1, f(F_EXTENSION));
                }
            }
            b'*' | b'?' => self.tok(p, p + 1, f(F_ASTERISK)),
            // `\\[^$*?]`, the `\n` among what follows
            b'\\' => match self.at(p + 1) {
                Some(b'$' | b'*' | b'?') => {}
                _ if p + 1 == b.len() => {
                    self.tok(p, p + 2, f(F_ESCAPE));
                    return p + 2;
                }
                _ => {
                    let e = p + 1 + self.l.char_len(p + 1).unwrap_or(1);
                    self.tok(p, e, f(F_ESCAPE));
                    return e;
                }
            },
            // `\$\w+`
            b'$' => {
                let e = self
                    .l
                    .run(p + 1, |c| crate::machine::is_word(c) && c != b'\n');
                if e > p + 1 {
                    self.tok(p, e, f(F_ENV));
                    return e;
                }
            }
            b':' => self.tok(p, p + 1, f(F_BAD_DRIVE)),
            // `"` and the spaces: nothing here, but their contexts' patterns at the next step
            _ => return p + usize::from(!self.ends_pattern(b[p])),
        }
        // the patterns above that step on match one byte
        if self.at <= p { p + 1 } else { self.at }
    }

    /// Whether `c` ends the open `fnmatch` pattern, for its context's turn.
    fn ends_pattern(&self, c: u8) -> bool {
        match self.top() {
            PATTERN_QUOTED => c == b'"',
            PATTERN_UNQUOTED => is_space(c),
            _ => false,
        }
    }

    /// `fnmatch-char-class-body` at `p`.
    fn class_body(&mut self, p: usize) -> usize {
        let nl = self.l.nl();
        let pattern = self.pattern;
        let f = |leaf| fk(pattern, leaf);
        match self.at(p) {
            Some(b']') if p < nl => {
                self.tok(p, p + 1, f(F_CLASS_END));
                self.pop();
                p + 1
            }
            // `(?m:$)\n?`
            _ if p >= nl => {
                self.flush(p);
                self.pop();
                self.l.end()
            }
            Some(b'\\') if self.at(p + 1) == Some(b']') => {
                self.tok(p, p + 2, f(F_CLASS_ESCAPE));
                p + 2
            }
            Some(b'*' | b'?') => {
                self.tok(p, p + 1, f(F_CLASS_BAD));
                p + 1
            }
            Some(c) if !is_space(c) => {
                let e = p + self.l.char_len(p).unwrap_or(1);
                self.tok(p, e, f(F_CLASS_CHAR));
                e
            }
            _ => p + 1,
        }
    }

    /// Git Ignore's `main` at `p`: a comment, or a pattern from the first byte that isn't a space.
    fn ignore_main(&mut self, p: usize) -> usize {
        let b = self.l.b;
        let q = p + b[p.min(b.len())..]
            .iter()
            .position(|&c| !is_space(c))
            .unwrap_or(b.len() - p.min(b.len()));
        if q >= b.len() {
            return self.l.end() + 1;
        }
        if matches!(b[q], b'#' | b';') {
            self.comment(q);
            return self.l.end() + 1;
        }
        // `!|(?=\S)`
        let n = usize::from(b[q] == b'!');
        self.pattern = IN_IGNORE;
        self.tok(q, q + n, fk(IN_IGNORE, F_NEGATE));
        self.flush(q);
        self.push(IGNORE_PATTERN);
        self.push(FNMATCH_START);
        q + n
    }

    /// `([#;]).*\n?` from `q`.
    fn comment(&mut self, q: usize) {
        self.tok(q, q + 1, COMMENT_MARK);
        self.tok(q + 1, self.l.end(), COMMENT);
    }

    /// `^\s*([#;]).*\n?`, tried at the line's start: whether it matched.
    fn comment_line(&mut self, p: usize) -> bool {
        if p != 0 {
            return false;
        }
        let Some(m) = comment_line(&self.l, 0) else {
            return false;
        };
        let (q, _) = m.group(1).unwrap_or((0, 0));
        self.tok(0, q, COMMENT);
        self.comment(q);
        true
    }

    /// Git Attributes' `main` at `p`: a comment line, a macro, or a pattern.
    fn attributes_main(&mut self, p: usize) -> usize {
        if self.comment_line(p) {
            return self.l.end() + 1;
        }
        let b = self.l.b;
        let Some(q) = (p..b.len()).find(|&i| !is_space(b[i])) else {
            return self.l.end() + 1;
        };
        // `(\[)([^-!]\S*)(\])`
        if b[q] == b'['
            && let Some(j) = macro_end(&self.l, q)
        {
            self.tok(q, q + 1, BRACKETS_BEGIN);
            self.tok(q + 1, j, MACRO);
            self.tok(j, j + 1, BRACKETS_END);
            self.push(ATTRIBUTES_LIST);
            return j + 1;
        }
        // `[-!\[]|(?=\S)`
        let n = usize::from(matches!(b[q], b'-' | b'!' | b'['));
        self.tok(q, q + n, BAD_OPERATOR);
        self.flush(q);
        self.push(PATTERN_ATTRIBUTES);
        self.push(PATTERN_CONTENT);
        q + n
    }

    /// `attributes-list` at `p`.
    fn attributes_list(&mut self, p: usize) -> usize {
        let nl = self.l.nl();
        if p >= nl {
            // `(?m:$)\n?`
            self.flush(p);
            self.pop();
            return self.l.end();
        }
        let b = self.l.b;
        let c = b[p];
        // `[-!](?=\w)`
        if matches!(c, b'-' | b'!') && word(&self.l, p + 1) {
            self.tok(p, p + 1, LIST_LOGICAL);
            return p + 1;
        }
        if word(&self.l, p) {
            // `(?=diff\b)` and the other builtins, else `(?=\w)`
            let named = |w: &[u8]| b[p..].starts_with(w) && boundary(&self.l, p + w.len());
            let meta = match c {
                b'd' if named(b"diff") => META_DIFF,
                b'e' if named(b"encoding") => META_ENCODING,
                b'e' if named(b"eol") => META_EOL,
                b'f' if named(b"filter") => META_FILTER,
                b'm' if named(b"merge") => META_MERGE,
                b't' if named(b"text") => META_TEXT,
                b'w' if named(b"whitespace") => META_WHITESPACE,
                _ => META_OTHER,
            };
            self.flush(p);
            self.push(meta);
            self.push(ATTRIBUTE_KEY);
            return p;
        }
        if is_space(c) {
            // `\s+`, through the `\n`
            let e = self.l.run(p, is_space);
            self.tok(p, e, LIST_SPACES);
            return e;
        }
        // `\S+`
        let e = self.l.run(p, |c| !is_space(c));
        self.tok(p, e, LIST_BAD_NAME);
        e
    }

    /// `attribute-key` at `p`, a word: `\w+[-\w]*\b`, the run giving back to a `\b`.
    fn attribute_key(&mut self, p: usize) -> usize {
        let run = self.l.run(p, |c| {
            (crate::machine::is_word(c) && c != b'\n') || c == b'-'
        });
        match (p + 1..=run).rev().find(|&e| boundary(&self.l, e)) {
            Some(e) if word(&self.l, p) => {
                self.tok(p, e, ak(self.meta, A_KEY));
                self.set(MAYBE_VALUE_LIST);
                e
            }
            // `else-pop`
            _ => {
                self.flush(p);
                self.pop();
                p
            }
        }
    }

    /// `attribute-maybe-value-list` at `p`.
    fn maybe_value_list(&mut self, p: usize) -> usize {
        let nl = self.l.nl();
        match self.at(p) {
            Some(b'=') if p < nl => {
                self.tok(p, p + 1, ak(self.meta, A_KEY_VALUE));
                // `=(?=\S)`, else `=`
                if p + 1 < nl && !is_space(self.l.b[p + 1]) {
                    self.set(VALUE_LIST);
                    self.push(VALUE_CONTENT);
                } else {
                    self.set(VALUE_EXPECTED);
                }
                p + 1
            }
            // `attribute-separator`: `\s`
            Some(c) if is_space(c) => {
                self.tok(p, p + 1, ak(self.meta, A_PAIR));
                self.pop();
                p + 1
            }
            // `else-pop`
            _ => {
                self.flush(p);
                self.pop();
                p
            }
        }
    }

    /// `attribute-value-list` at `p`.
    fn value_list(&mut self, p: usize) -> usize {
        let nl = self.l.nl();
        match self.at(p) {
            Some(c) if is_space(c) => {
                self.tok(p, p + 1, ak(self.meta, A_PAIR));
                self.pop();
                p + 1
            }
            Some(b',') if p < nl => {
                self.tok(p, p + 1, ak(self.meta, A_VALUE_SEP));
                // `,(?=\S)`, else `,`
                if p + 1 < nl && !is_space(self.l.b[p + 1]) {
                    self.push(VALUE_CONTENT);
                } else {
                    self.set(VALUE_EXPECTED);
                }
                p + 1
            }
            // `\S`
            Some(_) if p < nl => {
                let e = p + self.l.char_len(p).unwrap_or(1);
                self.tok(p, e, ak(self.meta, A_BAD_VALUE));
                e
            }
            // `else-pop`, past the `\n`
            _ => {
                self.flush(p);
                self.pop();
                p
            }
        }
    }

    /// `#anon_attribute-value-content_0`, a quoted value, at `p`.
    fn value_string(&mut self, mut p: usize) -> usize {
        let b = self.l.b;
        while p < b.len() && !matches!(b[p], b'"' | b'\\') {
            p += 1;
        }
        if p >= b.len() {
            // `(?m:$)\n?`
            self.flush(p);
            self.pop();
            return self.l.end();
        }
        if b[p] == b'"' {
            self.tok(p, p + 1, ak(self.meta, A_END));
            self.pop();
            return p + 1;
        }
        // `\\[\\\"\'fnrst]`
        if self.l.is(p + 1, |c| b"\\\"'fnrst".contains(&c)) {
            self.tok(p, p + 2, ak(self.meta, A_ESCAPE));
            return p + 2;
        }
        p + 1
    }

    /// Git Mailmap's `main` at `p`: a comment, an e-mail address, a name.
    fn mailmap_main(&mut self, p: usize) -> usize {
        let b = self.l.b;
        if p >= b.len() {
            return self.l.end() + 1;
        }
        let c = b[p];
        match c {
            b'#' | b';' => {
                self.comment(p);
                return self.l.end() + 1;
            }
            b'<' if self.email_open(p) => {
                self.tok(p, p + 1, EMAIL_BEGIN);
                self.push(EMAIL_META);
                self.push(EMAIL_NAME);
                return p + 1;
            }
            b' ' | b'\t' | b'<' => return p + 1,
            _ => {}
        }
        if p >= self.no_user {
            match self.user(p) {
                Ok(e) => {
                    self.tok(p, e, USER);
                    return e;
                }
                Err(run) => self.no_user = run + 1,
            }
        }
        p + 1
    }

    /// `<(?=\S+?>)` at `k`, a `<`: a `>` after it with no space before, but a character.
    fn email_open(&mut self, k: usize) -> bool {
        let b = self.l.b;
        if k + 1 >= b.len() || is_space(b[k + 1]) {
            return false;
        }
        let from = k + 2;
        let stop = if self.stop.0 <= from && from <= self.stop.1 {
            self.stop.1
        } else {
            let j = from
                + b[from.min(b.len())..]
                    .iter()
                    .position(|&c| c == b'>' || is_space(c))
                    .unwrap_or(b.len().saturating_sub(from));
            self.stop = (from, j);
            j
        };
        stop < b.len() && b[stop] == b'>'
    }

    /// `[^ \t<#][^<#\n]+?(?=[ \t]*(<|(?m:$)))` at `p`: where it ends, or where the run of text
    /// it could take ends, as no later start in that run matches either.
    ///
    /// The run stops at a `<`, a `#` or the `\n`, and the blanks after an end can only reach a
    /// `<` or the `\n` at the run's end: so the match ends where the run's last blanks start (or
    /// a character in), unless a `#` ends the run.
    fn user(&self, p: usize) -> Result<usize, usize> {
        let b = self.l.b;
        let from = (p + 1).min(b.len());
        let run = from
            + b[from..]
                .iter()
                .position(|&c| matches!(c, b'<' | b'#' | b'\n'))
                .unwrap_or(b.len() - from);
        if b.get(run) == Some(&b'#') {
            return Err(run);
        }
        let mut t = run;
        while t > p + 1 && is_blank(b[t - 1]) {
            t -= 1;
        }
        let e = t.max(p + 2);
        if e <= run { Ok(e) } else { Err(run) }
    }

    /// `email-name` and `email-domain` at `p`.
    fn email(&mut self, mut p: usize) -> usize {
        let b = self.l.b;
        let domain = self.top() == EMAIL_DOMAIN;
        while p < b.len() && !matches!(b[p], b'>' | b'@' | b'.') {
            p += 1;
        }
        match b.get(p) {
            None => self.l.end() + 1,
            Some(b'>') => {
                self.tok(p, p + 1, EMAIL_END);
                self.pop();
                p + 1
            }
            Some(b'@') if !domain => {
                self.tok(p, p + 1, EMAIL_SEP);
                self.set(EMAIL_DOMAIN);
                p + 1
            }
            Some(b'.') if domain => {
                self.tok(p, p + 1, DOMAIN_SEP);
                p + 1
            }
            _ => p + 1,
        }
    }

    /// Git Link's `main` at `p`: a comment line, `gitdir`, or anything else.
    fn link_main(&mut self, p: usize) -> usize {
        if self.comment_line(p) {
            return self.l.end() + 1;
        }
        let b = self.l.b;
        let Some(q) = (p..b.len()).find(|&i| !is_space(b[i])) else {
            return self.l.end() + 1;
        };
        // `\bgitdir\b`
        if b[q..].starts_with(b"gitdir")
            && (q == 0 || !word(&self.l, q - 1))
            && !word(&self.l, q + 6)
        {
            self.tok(q, q + 6, GITDIR);
            self.push(EXPECT_SEPARATOR);
            return q + 6;
        }
        // `\S+`
        let e = self.l.run(q, |c| !is_space(c));
        self.tok(q, e, GITDIR_EXPECTED);
        e
    }

    /// Git Link's `path` at `p`.
    fn path(&mut self, mut p: usize) -> usize {
        let b = self.l.b;
        let nl = b.len();
        while p < nl
            && !matches!(b[p], b'\\' | b'/' | b'*' | b'?')
            && !(b[p] == b'.' && p + 1 == nl)
        {
            p += 1;
        }
        if p >= nl {
            // `\.?\n`
            self.tok(p, p + 1, LINK_TERMINATOR);
            self.pop();
            return self.l.end();
        }
        match b[p] {
            b'.' => {
                self.tok(p, nl + 1, LINK_TERMINATOR);
                self.pop();
                self.l.end()
            }
            b'*' | b'?' => {
                self.tok(p, p + 1, LINK_BAD_PATH);
                p + 1
            }
            _ => {
                self.tok(p, p + 1, LINK_PATH_SEP);
                self.push(PATH_DIR_PATTERN);
                p + 1
            }
        }
    }
}

/// `(\[)([^-!]\S*)(\])` at `k`, a `[`: where the name's `]` is (it gives back to its last one).
fn macro_end(l: &Line, k: usize) -> Option<usize> {
    if l.at(k + 1).is_none_or(|c| c == b'-' || c == b'!') {
        return None;
    }
    let first = l.char_len(k + 1).unwrap_or(1);
    let run = l.run(k + 1 + first, |c| !is_space(c));
    (k + 1 + first..run).rev().find(|&j| l.b[j] == b']')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::Grammar;
    use crate::{Span, git, machine};

    /// Each byte's scopes, as `kind_scopes` names a span's kind.
    fn bytes(
        len: usize,
        spans: &[Span],
        kind_scopes: impl Fn(u16) -> Vec<&'static str>,
    ) -> Vec<String> {
        let mut out = vec![String::new(); len];
        let mut last = 0;
        for s in spans {
            assert!(
                s.start >= last && s.end > s.start && s.end as usize <= len,
                "{spans:?}"
            );
            last = s.end;
            let scopes = kind_scopes(s.kind).join(" ");
            for b in &mut out[s.start as usize..s.end as usize] {
                b.clone_from(&scopes);
            }
        }
        out
    }

    fn grammar(g: Git) -> &'static Grammar {
        match g {
            Git::Ignore => &git::IGNORE,
            Git::Attributes => &git::ATTRIBUTES,
            Git::Mailmap => &git::MAILMAP,
            Git::Link => &git::LINK,
        }
    }

    /// The scanner gives each line of `block` the port's scopes and leaves it the port's state.
    fn agree(g: Git, block: &str) -> Result<(), String> {
        let (mut port, mut scan) = (State::default(), State::default());
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for (n, text) in block.split('\n').enumerate() {
            a.clear();
            b.clear();
            machine::line(
                grammar(g),
                text.as_bytes(),
                &mut port,
                &mut Out::new(&mut a),
            );
            line(g, text.as_bytes(), &mut scan, &mut Out::new(&mut b));
            let want = bytes(text.len(), &a, |k| grammar(g).scopes(k));
            let got = bytes(text.len(), &b, scopes);
            if want != got {
                let i = (0..text.len()).find(|&i| want[i] != got[i]).unwrap_or(0);
                return Err(format!(
                    "{g:?} line {n} {text:?}, byte {i}: port {:?}, scanner {:?}",
                    want[i], got[i]
                ));
            }
            if port != scan {
                return Err(format!(
                    "{g:?} line {n} {text:?}: state port {port:?}, scanner {scan:?}"
                ));
            }
        }
        Ok(())
    }

    const ALL: [Git; 4] = [Git::Ignore, Git::Attributes, Git::Mailmap, Git::Link];

    #[test]
    fn agrees_with_the_port_on_examples() {
        let blocks = [
            "# build output\n/target/\n!keep.me\n*.log ; not a comment\n**/node_modules\n",
            "docs/[!a-c]*.md\na[]b]c\na[x\n\\#literal \\ \n$HOME/../.cache/./x\nC:/drive ~/home\n   \n",
            "a[\\]b]c [^x] [*?] [ ] [é]\nx\\\ny\\$z\\*\\é \"q\" a:b ./x ../y .z/w.v\n[\n\\\n!\n!!x\n",
            "# attributes\n* text=auto eol=lf\n*.png binary -diff\n",
            "*.rs diff=rust merge=union filter=lfs encoding=UTF-16 whitespace=tab-in-indent,trailing-space\n",
            "\"quoted path\" -text !eol custom=a,b, x=\"q\\\"v\"\n[attr]binary -diff -merge -text\n-bad op\n",
            "path/[ab]*.c foo= bar\nx a=\"open\nx a=, b=,c ,d a=b,,c a==b a=b=c\n !x\n\"[a\" b\n\"a\nb\\\nc d\n",
            "x textual eolx diffx -diff- a-b- a--b _ é=é [ab] [-x] [!x] [a] b\n x\t\ty=\"\\t\\x\" z=\" \"\n",
            "# mailmap\nProper Name <proper@email.xx>\n<proper@email.xx> <commit@email.xx>\n",
            "Proper Name <proper@email.xx> Commit Name <commit@email.xx>\nName Only\n<bad email> x\n",
            "a <b@c.d.e> f ; g\n<<a>> <> <a <a@b@c.d> a\t <x>\nname ;comment <x>\n\r<a>\n",
            "gitdir: ../.git/modules/sub\n  gitdir :C:\\repo\\.git.\ngitdir: ~/x/*\nnotgitdir: x\n",
            "# comment\ngitdir\nx\ngitdir x : y\ngitdir:\ngitdir: a/..\\./b.c.\n gitdir: .\n",
            "",
            "\n\n",
        ];
        for block in blocks {
            for g in ALL {
                agree(g, block).unwrap();
            }
        }
    }

    /// SplitMix64, so the test needs no crate: any seed, zero included.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }

        fn pick<'a>(&mut self, of: &[&'a str]) -> &'a str {
            of[(self.next() % of.len() as u64) as usize]
        }
    }

    /// Blocks of the four syntaxes' tokens in random order, against the port, each block in each
    /// syntax.
    #[test]
    fn agrees_with_the_port_on_token_soup() {
        let tokens = [
            "#",
            ";",
            "!",
            "-",
            "[",
            "]",
            "^",
            "/",
            "\\",
            ".",
            "..",
            "*",
            "?",
            "$",
            "$HOME",
            ":",
            "C:",
            "~",
            "\"",
            "=",
            ",",
            "<",
            ">",
            "@",
            " ",
            "  ",
            "\t",
            "\r",
            "\n",
            "\n",
            "\n",
            "a",
            "x_y",
            "é",
            "diff",
            "text",
            "eol",
            "encoding",
            "filter",
            "merge",
            "whitespace",
            "binary",
            "gitdir",
            "gitdir:",
            "\\n",
            "\\\"",
            "a@b.c",
            "name",
            "1",
        ];
        let rounds: u64 = std::env::var("GIT_SOUP")
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(10_000);
        let mut rng = Rng(0);
        for _ in 0..rounds {
            let n = 1 + rng.next() % 40;
            let block: String = (0..n).map(|_| rng.pick(&tokens)).collect();
            for g in ALL {
                if let Err(e) = agree(g, &block) {
                    panic!("{e}\nblock: {block:?}");
                }
            }
        }
    }

    /// Each file of `$GIT_CORPUS` (a list of paths) against the port, in the syntax its name
    /// gives.
    #[test]
    #[ignore]
    fn agrees_with_the_port_on_the_corpus() {
        let list = std::env::var("GIT_CORPUS").expect("GIT_CORPUS");
        let list = std::fs::read_to_string(list).expect("a list of paths");
        for path in list.lines() {
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            let g = if path.ends_with("attributes") {
                Git::Attributes
            } else if path.ends_with("mailmap") {
                Git::Mailmap
            } else {
                Git::Ignore
            };
            if let Err(e) = agree(g, &text) {
                panic!("{path}: {e}");
            }
        }
    }
}
