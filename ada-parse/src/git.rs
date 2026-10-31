//! Git's own files, in place of bat's `Git Ignore`, `Git Attributes`, `Git Mailmap` and
//! `Git Link`: their contexts and the parts of the hidden `Git Common` they share (comments,
//! `fnmatch` patterns, e-mail addresses), in one table on the [machine](crate::machine), each
//! syntax a [`Grammar`] that starts at its own `__start`.
//!
//! `Git Config` and `Git Rebase Todo` embed a shell, and `Git Commit` and `Git Log` bat's
//! `Diff`; they stay on syntect.
//!
//! Each context is marked with the syntax and name the probe's `dump` gives it, which
//! `check_refs_multi.py` beside the probe holds the table's references against.

use crate::machine::{
    Ctx, Grammar, It::I, It::P, Kinds, Line, M, Pat, atoms, empty, first, is_blank, is_space,
    is_word, lit1, non_space, protos, slots,
};

// Git Common
const COMMENTS: u8 = 0;
const COMMENTS_LINE: u8 = 1;
const EMAIL_DOMAIN: u8 = 2;
const EMAIL_END: u8 = 3;
const EMAIL_META: u8 = 4;
const EMAIL_NAME: u8 = 5;
const FNMATCH_BODY: u8 = 6;
const CLASS_BODY: u8 = 7;
const CLASS_OPERATOR: u8 = 8;
const CLASS_START: u8 = 9;
const DIR_PATTERN: u8 = 10;
const FNMATCH_START: u8 = 11;
// Git Ignore
const IGNORE_MAIN_0: u8 = 12;
const IGNORE_START: u8 = 13;
const IGNORE_MAIN: u8 = 14;
const IGNORE_PATTERN: u8 = 15;
// Git Mailmap
const MAILMAP_MAIN_0: u8 = 16;
const MAILMAP_START: u8 = 17;
const MAILMAP_MAIN: u8 = 18;
// Git Link
const LINK_MAIN_0: u8 = 19;
const LINK_START: u8 = 20;
const EXPECT_PATH: u8 = 21;
const EXPECT_SEPARATOR: u8 = 22;
const GITDIR: u8 = 23;
const LINK_MAIN: u8 = 24;
const PATH: u8 = 25;
const PATH_DIR_PATTERN: u8 = 26;
const PATH_START: u8 = 27;
const SEPARATOR: u8 = 28;
// Git Attributes
const VALUE_CONTENT_0: u8 = 29;
const ATTR_MAIN_0: u8 = 30;
const ATTR_START: u8 = 31;
const ATTRIBUTE_KEY: u8 = 32;
const MAYBE_VALUE_LIST: u8 = 33;
const ATTRIBUTE_SEPARATOR: u8 = 34;
const VALUE_CONTENT: u8 = 35;
const VALUE_EXPECTED: u8 = 36;
const VALUE_LIST: u8 = 37;
const ATTRIBUTES_LIST: u8 = 38;
const ELSE_POP: u8 = 39;
const MACRO: u8 = 40;
const ATTR_MAIN: u8 = 41;
const META_DIFF: u8 = 42;
const META_ENCODING: u8 = 43;
const META_EOL: u8 = 44;
const META_FILTER: u8 = 45;
const META_MERGE: u8 = 46;
const META_OTHER: u8 = 47;
const META_TEXT: u8 = 48;
const META_WHITESPACE: u8 = 49;
const PATTERN: u8 = 50;
const PATTERN_ATTRIBUTES: u8 = 51;
const PATTERN_CONTENT: u8 = 52;
const PATTERN_QUOTED: u8 = 53;
const PATTERN_UNQUOTED: u8 = 54;
const STRING_END: u8 = 55;

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

const fn ctx(items: &'static [crate::machine::It]) -> Ctx {
    Ctx::new(items)
}

static CTXS: [Ctx; 56] = [
    // Git Common: comments
    ctx(&[P(Pat::new(comment)
        .sc(&[s::COMMENT])
        .caps(&[(1, &[s::COMMENT_MARK])]))]),
    // Git Common: comments-line
    ctx(&[P(Pat::new(comment_line)
        .sc(&[s::COMMENT])
        .caps(&[(1, &[s::COMMENT_MARK])]))]),
    // Git Common: email-domain
    ctx(&[P(Pat::new(dot).sc(&[s::DOMAIN_SEP])), I(EMAIL_END)]).content(&[s::EMAIL_ENTITY]),
    // Git Common: email-end
    ctx(&[P(Pat::new(gt).sc(&[s::EMAIL_END_MARK]).pop())]),
    // Git Common: email-meta
    ctx(&[P(Pat::new(empty).pop())]).meta(&[s::EMAIL_META_S]),
    // Git Common: email-name
    ctx(&[
        P(Pat::new(at_sign).sc(&[s::EMAIL_SEP]).set(&[EMAIL_DOMAIN])),
        I(EMAIL_END),
    ])
    .content(&[s::EMAIL_ENTITY]),
    // Git Common: fnmatch-body
    ctx(&[
        P(Pat::new(open_bracket).sc(&[s::CLASS_BEGIN]).push(&[
            CLASS_BODY,
            CLASS_START,
            CLASS_OPERATOR,
        ])),
        P(Pat::new(slash).sc(&[s::PATH_SEP]).push(&[DIR_PATTERN])),
        P(Pat::new(extension).sc(&[s::EXTENSION_SEP])),
        P(Pat::new(asterisk).sc(&[s::ASTERISK])),
        P(Pat::new(path_escape).sc(&[s::PATH_ESCAPE])),
        P(Pat::new(env_var).sc(&[s::ENV_VAR])),
        P(Pat::new(colon).sc(&[s::BAD_DRIVE])),
        P(Pat::new(escape_before_space).sc(&[s::BAD_ESCAPE])),
    ]),
    // Git Common: fnmatch-char-class-body
    ctx(&[
        P(Pat::new(close_bracket).sc(&[s::CLASS_END]).pop()),
        P(Pat::new(eol_newline).sc(&[s::CLASS_EOL]).pop()),
        P(Pat::new(escaped_bracket).sc(&[s::CLASS_ESCAPE])),
        P(Pat::new(asterisk).sc(&[s::CLASS_BAD])),
        P(Pat::new(non_space).sc(&[s::CLASS_CHAR])),
    ])
    .meta(&[s::CLASS_META]),
    // Git Common: fnmatch-char-class-operator
    ctx(&[P(Pat::new(class_negation).sc(&[s::CLASS_LOGICAL]).pop())]),
    // Git Common: fnmatch-char-class-start
    ctx(&[P(Pat::new(maybe_close_bracket).sc(&[s::CLASS_CHAR]).pop())]),
    // Git Common: fnmatch-dir-pattern
    ctx(&[
        P(Pat::new(parent_slash).sc(&[s::PARENT]).pop()),
        P(Pat::new(self_slash).sc(&[s::SELF]).pop()),
        P(Pat::new(empty).pop()),
    ]),
    // Git Common: fnmatch-start
    ctx(&[
        P(Pat::new(drive_slash).caps(&[(1, &[s::DRIVE])]).pop()),
        P(Pat::new(home_slash).sc(&[s::HOME]).pop()),
        I(DIR_PATTERN),
    ]),
    // Git Ignore: __main
    ctx(&[I(IGNORE_MAIN)]),
    // Git Ignore: __start
    ctx(&[P(Pat::new(empty).push(&[IGNORE_MAIN_0]))]).no_proto(),
    // Git Ignore: main
    ctx(&[
        I(COMMENTS),
        P(Pat::new(negation_or_start)
            .sc(&[s::IGNORE_NEGATE])
            .push(&[IGNORE_PATTERN, FNMATCH_START])),
    ]),
    // Git Ignore: pattern-content
    ctx(&[P(Pat::new(eol).pop()), I(FNMATCH_BODY)]).meta(&[s::IGNORE_STRING, s::IGNORE_PATTERN_S]),
    // Git Mailmap: __main
    ctx(&[I(MAILMAP_MAIN)]),
    // Git Mailmap: __start
    ctx(&[P(Pat::new(empty).push(&[MAILMAP_MAIN_0]))]).no_proto(),
    // Git Mailmap: main
    ctx(&[
        I(COMMENTS),
        P(Pat::new(email_open)
            .sc(&[s::EMAIL_BEGIN])
            .push(&[EMAIL_META, EMAIL_NAME])),
        P(Pat::new(user).sc(&[s::USER])),
    ]),
    // Git Link: __main
    ctx(&[I(LINK_MAIN)]),
    // Git Link: __start
    ctx(&[P(Pat::new(empty).push(&[LINK_MAIN_0]))]).no_proto(),
    // Git Link: expect-path
    ctx(&[
        P(Pat::new(non_space_ahead).set(&[PATH, PATH_START])),
        P(Pat::new(newline).pop()),
    ])
    .content(&[s::LINK_VALUE]),
    // Git Link: expect-separator
    ctx(&[
        P(Pat::new(colon_ahead).set(&[SEPARATOR])),
        P(Pat::new(non_space).sc(&[s::SEPARATOR_EXPECTED])),
    ])
    .meta(&[s::LINK_KEY]),
    // Git Link: gitdir
    ctx(&[
        P(Pat::new(gitdir)
            .sc(&[s::GITDIR_KW])
            .push(&[EXPECT_SEPARATOR])),
        P(Pat::new(non_spaces).sc(&[s::GITDIR_EXPECTED])),
    ]),
    // Git Link: main
    ctx(&[I(COMMENTS_LINE), I(GITDIR)]),
    // Git Link: path
    ctx(&[
        P(Pat::new(path_end)
            .sc(&[s::LINK_VALUE, s::LINK_TERMINATOR])
            .pop()),
        P(Pat::new(either_slash)
            .sc(&[s::LINK_PATH_SEP])
            .push(&[PATH_DIR_PATTERN])),
        P(Pat::new(asterisk).sc(&[s::LINK_BAD_PATH])),
    ])
    .content(&[s::LINK_VALUE, s::LINK_PATH, s::LINK_STRING]),
    // Git Link: path-dir-pattern
    ctx(&[
        P(Pat::new(parent_either).sc(&[s::LINK_PARENT]).pop()),
        P(Pat::new(self_either).sc(&[s::LINK_SELF]).pop()),
        P(Pat::new(empty).pop()),
    ]),
    // Git Link: path-start
    ctx(&[
        P(Pat::new(drive_either).caps(&[(1, &[s::LINK_DRIVE])]).pop()),
        P(Pat::new(home_slash).sc(&[s::LINK_HOME]).pop()),
        I(PATH_DIR_PATTERN),
    ]),
    // Git Link: separator
    ctx(&[
        P(Pat::new(colon).sc(&[s::LINK_KEY_VALUE]).set(&[EXPECT_PATH])),
        P(Pat::new(newline).pop()),
    ])
    .meta(&[s::LINK_MAPPING]),
    // Git Attributes: #anon_attribute-value-content_0
    ctx(&[
        I(STRING_END),
        P(Pat::new(attr_escape).sc(&[s::ATTR_ESCAPE])),
    ])
    .meta(&[s::ATTR_DOUBLE]),
    // Git Attributes: __main
    ctx(&[I(ATTR_MAIN)]),
    // Git Attributes: __start
    ctx(&[P(Pat::new(empty).push(&[ATTR_MAIN_0]))]).no_proto(),
    // Git Attributes: attribute-key
    ctx(&[
        P(Pat::new(attribute_name)
            .sc(&[s::ATTR_KEY, s::ATTR_NAME])
            .set(&[MAYBE_VALUE_LIST])),
        I(ELSE_POP),
    ]),
    // Git Attributes: attribute-maybe-value-list
    ctx(&[
        P(Pat::new(equals_value)
            .sc(&[s::ATTR_KEY_VALUE])
            .set(&[VALUE_LIST, VALUE_CONTENT])),
        P(Pat::new(equals)
            .sc(&[s::ATTR_KEY_VALUE])
            .set(&[VALUE_EXPECTED])),
        I(ATTRIBUTE_SEPARATOR),
        I(ELSE_POP),
    ])
    .content(&[s::ATTR_MAPPING]),
    // Git Attributes: attribute-separator
    ctx(&[P(Pat::new(space)
        .sc(&[s::ATTR_MAPPING, s::ATTR_PAIR])
        .pop())]),
    // Git Attributes: attribute-value-content
    ctx(&[
        P(Pat::new(dquote)
            .sc(&[s::ATTR_STR_BEGIN])
            .set(&[VALUE_CONTENT_0])),
        P(Pat::new(unquoted_value).sc(&[s::ATTR_UNQUOTED]).pop()),
        I(ELSE_POP),
    ]),
    // Git Attributes: attribute-value-expected
    ctx(&[P(Pat::new(space)
        .sc(&[s::ATTR_EXPECT_VALUE, s::ATTR_PAIR])
        .pop())]),
    // Git Attributes: attribute-value-list
    ctx(&[
        I(ATTRIBUTE_SEPARATOR),
        P(Pat::new(comma_value)
            .sc(&[s::ATTR_VALUES_SEP])
            .push(&[VALUE_CONTENT])),
        P(Pat::new(comma)
            .sc(&[s::ATTR_VALUES_SEP])
            .set(&[VALUE_EXPECTED])),
        P(Pat::new(non_space).sc(&[s::ATTR_BAD_VALUE])),
        I(ELSE_POP),
    ])
    .content(&[s::ATTR_VALUE]),
    // Git Attributes: attributes-list
    ctx(&[
        P(Pat::new(eol_newline).sc(&[s::ATTR_LIST]).pop()),
        P(Pat::new(attr_logical).sc(&[s::ATTR_LOGICAL])),
        P(Pat::new(diff_ahead).push(&[META_DIFF, ATTRIBUTE_KEY])),
        P(Pat::new(encoding_ahead).push(&[META_ENCODING, ATTRIBUTE_KEY])),
        P(Pat::new(eol_ahead).push(&[META_EOL, ATTRIBUTE_KEY])),
        P(Pat::new(filter_ahead).push(&[META_FILTER, ATTRIBUTE_KEY])),
        P(Pat::new(merge_ahead).push(&[META_MERGE, ATTRIBUTE_KEY])),
        P(Pat::new(text_ahead).push(&[META_TEXT, ATTRIBUTE_KEY])),
        P(Pat::new(whitespace_ahead).push(&[META_WHITESPACE, ATTRIBUTE_KEY])),
        P(Pat::new(word_ahead).push(&[META_OTHER, ATTRIBUTE_KEY])),
        P(Pat::new(spaces).sc(&[s::ATTR_MAPPING])),
        P(Pat::new(non_spaces).sc(&[s::ATTR_BAD_NAME])),
    ])
    .content(&[s::ATTR_LIST]),
    // Git Attributes: else-pop
    ctx(&[P(Pat::new(empty).pop())]),
    // Git Attributes: macro
    ctx(&[P(Pat::new(macro_name)
        .sc(&[s::ATTR_BRACKETS])
        .caps(&[
            (1, &[s::ATTR_BRACKETS_BEGIN]),
            (2, &[s::ATTR_UNQUOTED, s::ATTR_MACRO]),
            (3, &[s::ATTR_BRACKETS_END]),
        ])
        .push(&[ATTRIBUTES_LIST]))]),
    // Git Attributes: main
    ctx(&[I(COMMENTS_LINE), I(MACRO), I(PATTERN)]),
    // Git Attributes: meta-diff
    ctx(&[I(ELSE_POP)]).meta(&[s::META_DIFF_S]),
    // Git Attributes: meta-encoding
    ctx(&[I(ELSE_POP)]).meta(&[s::META_ENCODING_S]),
    // Git Attributes: meta-eol
    ctx(&[I(ELSE_POP)]).meta(&[s::META_EOL_S]),
    // Git Attributes: meta-filter
    ctx(&[I(ELSE_POP)]).meta(&[s::META_FILTER_S]),
    // Git Attributes: meta-merge
    ctx(&[I(ELSE_POP)]).meta(&[s::META_MERGE_S]),
    // Git Attributes: meta-other
    ctx(&[I(ELSE_POP)]).meta(&[s::META_OTHER_S]),
    // Git Attributes: meta-text
    ctx(&[I(ELSE_POP)]).meta(&[s::META_TEXT_S]),
    // Git Attributes: meta-whitespace
    ctx(&[I(ELSE_POP)]).meta(&[s::META_WHITESPACE_S]),
    // Git Attributes: pattern
    ctx(&[P(Pat::new(attr_operator_or_start)
        .sc(&[s::ATTR_BAD_OPERATOR])
        .push(&[PATTERN_ATTRIBUTES, PATTERN_CONTENT]))]),
    // Git Attributes: pattern-attributes
    ctx(&[P(Pat::new(empty).set(&[ATTRIBUTES_LIST]))]),
    // Git Attributes: pattern-content
    ctx(&[
        P(Pat::new(dquote)
            .sc(&[s::ATTR_STR_BEGIN])
            .set(&[PATTERN_QUOTED, FNMATCH_START])),
        P(Pat::new(empty).set(&[PATTERN_UNQUOTED, FNMATCH_START])),
    ]),
    // Git Attributes: pattern-content-quoted
    ctx(&[I(STRING_END), I(FNMATCH_BODY)])
        .meta(&[s::ATTR_DOUBLE])
        .content(&[s::ATTR_PATTERN]),
    // Git Attributes: pattern-content-unquoted
    ctx(&[P(Pat::new(space_ahead).pop()), I(FNMATCH_BODY)])
        .meta(&[s::ATTR_UNQUOTED])
        .content(&[s::ATTR_PATTERN]),
    // Git Attributes: string-end
    ctx(&[
        P(Pat::new(dquote).sc(&[s::ATTR_STR_END]).pop()),
        P(Pat::new(eol_newline).sc(&[s::ATTR_EOL]).pop()),
    ]),
];

static SLOTS: [u16; 56] = slots(&CTXS);
static PROTOS: [bool; 56] = protos(&CTXS, None);

/// One of the Git syntaxes: the table, from its own `__start`.
const fn grammar(start: &'static [u8], kinds: &'static Kinds) -> Grammar {
    Grammar {
        ctxs: &CTXS,
        start,
        atoms: ATOMS,
        slots: &SLOTS,
        proto: None,
        protos: &PROTOS,
        kinds,
    }
}

static IGNORE_KINDS: Kinds = Kinds::new();
static MAILMAP_KINDS: Kinds = Kinds::new();
static LINK_KINDS: Kinds = Kinds::new();
static ATTRIBUTES_KINDS: Kinds = Kinds::new();

pub static IGNORE: Grammar = grammar(&[IGNORE_START], &IGNORE_KINDS);
pub static MAILMAP: Grammar = grammar(&[MAILMAP_START], &MAILMAP_KINDS);
pub static LINK: Grammar = grammar(&[LINK_START], &LINK_KINDS);
pub static ATTRIBUTES: Grammar = grammar(&[ATTR_START], &ATTRIBUTES_KINDS);

// the matchers: each finds its regex's leftmost match at or after `from`

/// `([#;]).*\n?`
fn comment(l: &Line, from: usize) -> Option<M> {
    let hay = l.b.get(from..)?;
    let k = from + hay.iter().position(|&c| c == b'#' || c == b';')?;
    Some(M::new(k, l.end()).g(1, k, k + 1))
}

/// `^\s*([#;]).*\n?`
fn comment_line(l: &Line, from: usize) -> Option<M> {
    if from > 0 {
        return None;
    }
    let q = l.run(0, is_space);
    (q < l.nl() && matches!(l.b[q], b'#' | b';')).then(|| M::new(0, l.end()).g(1, q, q + 1))
}

/// `\.`
fn dot(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'.')
}

/// `>`
fn gt(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'>')
}

/// `@`
fn at_sign(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'@')
}

/// `\[`
fn open_bracket(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'[')
}

/// `\]`
fn close_bracket(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b']')
}

/// `/`
fn slash(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'/')
}

/// `:`
fn colon(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b':')
}

/// `"`
fn dquote(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'"')
}

/// `=`
fn equals(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'=')
}

/// `,`
fn comma(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b',')
}

/// `<c>(?=\S)`
fn before_non_space(l: &Line, from: usize, c: u8) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, c) {
        if k + 1 < l.nl() && !is_space(l.b[k + 1]) {
            return Some(M::new(k, k + 1));
        }
        at = k + 1;
    }
    None
}

/// `=(?=\S)`
fn equals_value(l: &Line, from: usize) -> Option<M> {
    before_non_space(l, from, b'=')
}

/// `,(?=\S)`
fn comma_value(l: &Line, from: usize) -> Option<M> {
    before_non_space(l, from, b',')
}

/// `\.(?!.*[./])`: the last `.` with no `/` after it.
fn extension(l: &Line, from: usize) -> Option<M> {
    let hay = l.b.get(from..)?;
    let k = from + hay.iter().rposition(|&c| c == b'.' || c == b'/')?;
    (l.b[k] == b'.').then(|| M::new(k, k + 1))
}

/// `[*?]`
fn asterisk(l: &Line, from: usize) -> Option<M> {
    let hay = l.b.get(from..)?;
    let k = from + hay.iter().position(|&c| c == b'*' || c == b'?')?;
    Some(M::new(k, k + 1))
}

/// `\\[^$*?]`, the `\n` among what follows
fn path_escape(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, b'\\') {
        match l.at(k + 1) {
            Some(b'$' | b'*' | b'?') | None => at = k + 1,
            Some(_) if k + 1 == l.nl() => return Some(M::new(k, l.end())),
            Some(_) => return Some(M::new(k, k + 1 + l.char_len(k + 1).unwrap_or(1))),
        }
    }
    None
}

/// `\$\w+`
fn env_var(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, b'$') {
        let e = l.run(k + 1, |c| is_word(c) && c != b'\n');
        if e > k + 1 {
            return Some(M::new(k, e));
        }
        at = k + 1;
    }
    None
}

/// `\\(?=\s)`
fn escape_before_space(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, b'\\') {
        if l.is(k + 1, is_space) {
            return Some(M::new(k, k + 1));
        }
        at = k + 1;
    }
    None
}

/// `(?m:$)\n?`
fn eol_newline(l: &Line, from: usize) -> Option<M> {
    match from {
        _ if from <= l.nl() => Some(M::new(l.nl(), l.end())),
        _ if from == l.end() => Some(M::new(from, from)),
        _ => None,
    }
}

/// `(?m:$)`
fn eol(l: &Line, from: usize) -> Option<M> {
    match from {
        _ if from <= l.nl() => Some(M::new(l.nl(), l.nl())),
        _ if from == l.end() => Some(M::new(from, from)),
        _ => None,
    }
}

/// `\n`
fn newline(l: &Line, from: usize) -> Option<M> {
    (from <= l.nl()).then(|| M::new(l.nl(), l.end()))
}

/// `\\\]`
fn escaped_bracket(l: &Line, from: usize) -> Option<M> {
    l.lit(from, b"\\]")
}

/// `[!^]?`
fn class_negation(l: &Line, from: usize) -> Option<M> {
    let n = usize::from(l.is(from, |c| c == b'!' || c == b'^'));
    (from <= l.end()).then(|| M::new(from, from + n))
}

/// `\]?`
fn maybe_close_bracket(l: &Line, from: usize) -> Option<M> {
    let n = usize::from(l.at(from) == Some(b']'));
    (from <= l.end()).then(|| M::new(from, from + n))
}

/// `<lit>(?=<after>)`
fn lit_before(l: &Line, from: usize, lit: &[u8], after: &[u8]) -> Option<M> {
    let mut at = from;
    while let Some(m) = l.lit(at, lit) {
        if l.is(m.e as usize, |c| after.contains(&c)) {
            return Some(m);
        }
        at = m.s as usize + 1;
    }
    None
}

/// `\.\.(?=/)`
fn parent_slash(l: &Line, from: usize) -> Option<M> {
    lit_before(l, from, b"..", b"/")
}

/// `\.(?=/)`
fn self_slash(l: &Line, from: usize) -> Option<M> {
    lit_before(l, from, b".", b"/")
}

/// `\.\.(?=[\\/])`
fn parent_either(l: &Line, from: usize) -> Option<M> {
    lit_before(l, from, b"..", b"\\/")
}

/// `\.(?=[\\/])`
fn self_either(l: &Line, from: usize) -> Option<M> {
    lit_before(l, from, b".", b"\\/")
}

/// `~(?=/)`
fn home_slash(l: &Line, from: usize) -> Option<M> {
    lit_before(l, from, b"~", b"/")
}

/// `[A-Za-z](:)(?=<after>)`
fn drive(l: &Line, from: usize, after: &[u8]) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, b':') {
        if k > from && l.b[k - 1].is_ascii_alphabetic() && l.is(k + 1, |c| after.contains(&c)) {
            return Some(M::new(k - 1, k + 1).g(1, k, k + 1));
        }
        at = k + 1;
    }
    None
}

/// `[A-Za-z](:)(?=/)`
fn drive_slash(l: &Line, from: usize) -> Option<M> {
    drive(l, from, b"/")
}

/// `[A-Za-z](:)(?=[\\/])`
fn drive_either(l: &Line, from: usize) -> Option<M> {
    drive(l, from, b"\\/")
}

/// `!|(?=\S)`
fn negation_or_start(l: &Line, from: usize) -> Option<M> {
    let m = non_space(l, from)?;
    let p = m.s as usize;
    Some(M::new(p, p + usize::from(l.b[p] == b'!')))
}

/// `[-!\[]|(?=\S)`
fn attr_operator_or_start(l: &Line, from: usize) -> Option<M> {
    let m = non_space(l, from)?;
    let p = m.s as usize;
    Some(M::new(
        p,
        p + usize::from(matches!(l.b[p], b'-' | b'!' | b'[')),
    ))
}

/// `<(?=\S+?>)`: a `<` and a `>` after it with no space between, but a character.
fn email_open(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, b'<') {
        let run = l.run(k + 1, |c| !is_space(c));
        if run > k + 2 && l.b[k + 2..run].contains(&b'>') {
            return Some(M::new(k, k + 1));
        }
        at = k + 1;
    }
    None
}

/// `[^ \t<#][^<#\n]+?(?=[ \t]*(<|(?m:$)))`
fn user(l: &Line, from: usize) -> Option<M> {
    let body = |c: u8| !matches!(c, b'<' | b'#' | b'\n');
    let ahead = |e: usize| {
        let q = l.run(e, is_blank);
        l.at(q) == Some(b'<') || l.eol(q)
    };
    let mut p = from;
    while p < l.nl() {
        if matches!(l.b[p], b' ' | b'\t' | b'<' | b'#') {
            p += 1;
            continue;
        }
        let run = l.run(p + 1, body);
        if let Some(e) = (p + 2..=run).find(|&e| ahead(e)) {
            return Some(M::new(p, e));
        }
        // a later start in the run has fewer ends to try, none of which works
        p = run + 1;
    }
    None
}

/// `(?=\S)`
fn non_space_ahead(l: &Line, from: usize) -> Option<M> {
    non_space(l, from).map(|m| M::new(m.s as usize, m.s as usize))
}

/// `(?=:)`
fn colon_ahead(l: &Line, from: usize) -> Option<M> {
    first(l, from, b':').map(|k| M::new(k, k))
}

/// `\bgitdir\b`
fn gitdir(l: &Line, from: usize) -> Option<M> {
    let w = |i: usize| l.is(i, |c| is_word(c) && c != b'\n');
    let mut at = from;
    while let Some(m) = l.lit(at, b"gitdir") {
        let (s, e) = (m.s as usize, m.e as usize);
        if (s == 0 || !w(s - 1)) && !w(e) {
            return Some(m);
        }
        at = s + 1;
    }
    None
}

/// `\S+`
fn non_spaces(l: &Line, from: usize) -> Option<M> {
    let m = non_space(l, from)?;
    Some(M::new(m.s as usize, l.run(m.s as usize, |c| !is_space(c))))
}

/// `\.?\n`
fn path_end(l: &Line, from: usize) -> Option<M> {
    let nl = l.nl();
    if from > nl {
        return None;
    }
    let s = if nl > from && l.b[nl - 1] == b'.' {
        nl - 1
    } else {
        nl
    };
    Some(M::new(s, l.end()))
}

/// `[\\/]`
fn either_slash(l: &Line, from: usize) -> Option<M> {
    let hay = l.b.get(from..)?;
    let k = from + hay.iter().position(|&c| c == b'\\' || c == b'/')?;
    Some(M::new(k, k + 1))
}

/// `\\[\\\"\'fnrst]`
fn attr_escape(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, b'\\') {
        if l.is(k + 1, |c| b"\\\"'fnrst".contains(&c)) {
            return Some(M::new(k, k + 2));
        }
        at = k + 1;
    }
    None
}

/// `\w` at `i`.
fn word(l: &Line, i: usize) -> bool {
    l.is(i, |c| is_word(c) && c != b'\n')
}

/// `\b` at `i`.
fn boundary(l: &Line, i: usize) -> bool {
    (i > 0 && word(l, i - 1)) != word(l, i)
}

/// `\w+[-\w]*\b`: the run gives back until a `\b` follows.
fn attribute_name(l: &Line, from: usize) -> Option<M> {
    let mut p = from;
    while p < l.nl() {
        if !word(l, p) {
            p += 1;
            continue;
        }
        let run = l.run(p, |c| (is_word(c) && c != b'\n') || c == b'-');
        if let Some(e) = (p + 1..=run).rev().find(|&e| boundary(l, e)) {
            return Some(M::new(p, e));
        }
        p += 1;
    }
    None
}

/// `\s`
fn space(l: &Line, from: usize) -> Option<M> {
    (from..=l.nl())
        .find(|&i| l.is(i, is_space))
        .map(|i| M::new(i, i + 1))
}

/// `(?=\s)`
fn space_ahead(l: &Line, from: usize) -> Option<M> {
    space(l, from).map(|m| M::new(m.s as usize, m.s as usize))
}

/// `\s+`
fn spaces(l: &Line, from: usize) -> Option<M> {
    let m = space(l, from)?;
    Some(M::new(m.s as usize, l.run(m.s as usize, is_space)))
}

/// `[^,\s]*`
fn unquoted_value(l: &Line, from: usize) -> Option<M> {
    (from <= l.end()).then(|| M::new(from, l.run(from, |c| c != b',' && !is_space(c))))
}

/// `[-!](?=\w)`
fn attr_logical(l: &Line, from: usize) -> Option<M> {
    (from..l.nl())
        .find(|&i| matches!(l.b[i], b'-' | b'!') && word(l, i + 1))
        .map(|i| M::new(i, i + 1))
}

/// `(?=<word>\b)`
fn word_ahead_named(l: &Line, from: usize, name: &[u8]) -> Option<M> {
    let mut at = from;
    while let Some(m) = l.lit(at, name) {
        if boundary(l, m.e as usize) {
            return Some(M::new(m.s as usize, m.s as usize));
        }
        at = m.s as usize + 1;
    }
    None
}

fn diff_ahead(l: &Line, from: usize) -> Option<M> {
    word_ahead_named(l, from, b"diff")
}

fn encoding_ahead(l: &Line, from: usize) -> Option<M> {
    word_ahead_named(l, from, b"encoding")
}

fn eol_ahead(l: &Line, from: usize) -> Option<M> {
    word_ahead_named(l, from, b"eol")
}

fn filter_ahead(l: &Line, from: usize) -> Option<M> {
    word_ahead_named(l, from, b"filter")
}

fn merge_ahead(l: &Line, from: usize) -> Option<M> {
    word_ahead_named(l, from, b"merge")
}

fn text_ahead(l: &Line, from: usize) -> Option<M> {
    word_ahead_named(l, from, b"text")
}

fn whitespace_ahead(l: &Line, from: usize) -> Option<M> {
    word_ahead_named(l, from, b"whitespace")
}

/// `(?=\w)`
fn word_ahead(l: &Line, from: usize) -> Option<M> {
    (from..l.nl()).find(|&i| word(l, i)).map(|i| M::new(i, i))
}

/// `(\[)([^-!]\S*)(\])`: the name gives back up to its last `]`.
fn macro_name(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, b'[') {
        if l.at(k + 1).is_some_and(|c| c != b'-' && c != b'!') {
            let first_len = l.char_len(k + 1).unwrap_or(1);
            let run = l.run(k + 1 + first_len, |c| !is_space(c));
            if let Some(j) = (k + 1 + first_len..run).rev().find(|&j| l.b[j] == b']') {
                return Some(
                    M::new(k, j + 1)
                        .g(1, k, k + 1)
                        .g(2, k + 1, j)
                        .g(3, j, j + 1),
                );
            }
        }
        at = k + 1;
    }
    None
}
