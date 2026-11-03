//! What the scanner of Git's own files (`git_scan.rs`) shares from the port it replaced, of
//! bat's `Git Ignore`, `Git Attributes`, `Git Mailmap` and `Git Link` and the parts of the
//! hidden `Git Common` they share (comments, `fnmatch` patterns, e-mail addresses): the
//! contexts' ids, which its stack holds, each syntax starting at its own `__start`, the scopes,
//! and the matchers it tests at a position.
//!
//! `Git Config` and `Git Rebase Todo` embed a shell, and `Git Commit` and `Git Log` bat's
//! `Diff`; they stay on syntect.
//!
//! Each context is marked with the syntax and name the probe's `dump` gives it, which
//! `check_refs_multi.py` beside the probe holds the table's references against.

use crate::matcher::{Line, M, atoms, is_space, is_word};

// Git Common
pub(crate) const EMAIL_DOMAIN: u8 = 2;
pub(crate) const EMAIL_META: u8 = 4;
pub(crate) const EMAIL_NAME: u8 = 5;
pub(crate) const CLASS_BODY: u8 = 7;
pub(crate) const CLASS_OPERATOR: u8 = 8;
pub(crate) const CLASS_START: u8 = 9;
pub(crate) const DIR_PATTERN: u8 = 10;
pub(crate) const FNMATCH_START: u8 = 11;
// Git Ignore
pub(crate) const IGNORE_MAIN_0: u8 = 12;
pub(crate) const IGNORE_START: u8 = 13;
pub(crate) const IGNORE_PATTERN: u8 = 15;
// Git Mailmap
pub(crate) const MAILMAP_MAIN_0: u8 = 16;
pub(crate) const MAILMAP_START: u8 = 17;
// Git Link
pub(crate) const LINK_MAIN_0: u8 = 19;
pub(crate) const LINK_START: u8 = 20;
pub(crate) const EXPECT_PATH: u8 = 21;
pub(crate) const EXPECT_SEPARATOR: u8 = 22;
pub(crate) const PATH: u8 = 25;
pub(crate) const PATH_DIR_PATTERN: u8 = 26;
pub(crate) const PATH_START: u8 = 27;
pub(crate) const SEPARATOR: u8 = 28;
// Git Attributes
pub(crate) const VALUE_CONTENT_0: u8 = 29;
pub(crate) const ATTR_MAIN_0: u8 = 30;
pub(crate) const ATTR_START: u8 = 31;
pub(crate) const ATTRIBUTE_KEY: u8 = 32;
pub(crate) const MAYBE_VALUE_LIST: u8 = 33;
pub(crate) const VALUE_CONTENT: u8 = 35;
pub(crate) const VALUE_EXPECTED: u8 = 36;
pub(crate) const VALUE_LIST: u8 = 37;
pub(crate) const ATTRIBUTES_LIST: u8 = 38;
pub(crate) const META_DIFF: u8 = 42;
pub(crate) const META_ENCODING: u8 = 43;
pub(crate) const META_EOL: u8 = 44;
pub(crate) const META_FILTER: u8 = 45;
pub(crate) const META_MERGE: u8 = 46;
pub(crate) const META_OTHER: u8 = 47;
pub(crate) const META_TEXT: u8 = 48;
pub(crate) const META_WHITESPACE: u8 = 49;
pub(crate) const PATTERN_ATTRIBUTES: u8 = 51;
pub(crate) const PATTERN_CONTENT: u8 = 52;
pub(crate) const PATTERN_QUOTED: u8 = 53;
pub(crate) const PATTERN_UNQUOTED: u8 = 54;
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

// the matchers: each finds its regex's leftmost match at or after `from`

/// `^\s*([#;]).*\n?`
pub(crate) fn comment_line(l: &Line, from: usize) -> Option<M> {
    if from > 0 {
        return None;
    }
    let q = l.run(0, is_space);
    (q < l.nl() && matches!(l.b[q], b'#' | b';')).then(|| M::new(0, l.end()).g(1, q, q + 1))
}

/// `\w` at `i`.
pub(crate) fn word(l: &Line, i: usize) -> bool {
    l.is(i, |c| is_word(c) && c != b'\n')
}

/// `\b` at `i`.
pub(crate) fn boundary(l: &Line, i: usize) -> bool {
    (i > 0 && word(l, i - 1)) != word(l, i)
}
