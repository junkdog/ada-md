//! What `.env`'s scanner (`dotenv_scan.rs`) shares from the port of bat's `DotENV` it replaced:
//! the contexts' ids, which its state holds (syntect's, in the order the probe's `dump DotENV`
//! lists them), the scopes, and the matchers it tests at a position. Names' `\x{7f}-\x{ff}` is
//! any byte past ASCII here, and `\w` ASCII plus those.

use crate::matcher::{Line, atoms, is_word};

pub(crate) const INTERPOLATION_0: u8 = 0;
pub(crate) const DOUBLE_0: u8 = 1;
pub(crate) const SINGLE_0: u8 = 2;
pub(crate) const MAIN_0: u8 = 3;
pub(crate) const START: u8 = 4;
atoms! {
    INTERPOLATED = "string.interpolated.env",
    TEMPLATE_END = "keyword.other.template.end.env",
    VARIABLE_MARK = "punctuation.definition.variable.env",
    VARIABLE_S = "variable.other.env",
    DOUBLE = "string.quoted.double.env",
    STR_END = "punctuation.definition.string.end",
    SINGLE = "string.quoted.single.env",
    ESCAPE = "constant.character.escape.env",
    TEMPLATE_BEGIN = "keyword.other.template.begin.env",
    COMMENT = "comment.line.number-sign.env",
    COMMENT_MARK = "punctuation.definition.comment.env",
    STR_BEGIN = "punctuation.definition.string.begin.env",
    ASSIGNMENT = "keyword.operator.assignment.env",
    KEYWORD = "keyword.other.env",
    CONSTANT = "constant.language.env",
    NUMERIC = "constant.numeric.env",
}

// the matchers: each finds its regex's leftmost match at or after `from`

/// `\w` at `i`, the `\n` not.
pub(crate) fn word(l: &Line, i: usize) -> bool {
    l.is(i, |c| is_word(c) && c != b'\n')
}

/// `\b` at `i`.
pub(crate) fn boundary(l: &Line, i: usize) -> bool {
    (i > 0 && word(l, i - 1)) != word(l, i)
}

/// `[a-zA-Z_\x{7f}-\x{ff}]`
pub(crate) fn is_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x7f
}

/// `[a-zA-Z0-9_\x{7f}-\x{ff}]`
fn is_name(c: u8) -> bool {
    is_name_start(c) || c.is_ascii_digit()
}

/// `NAME*?\b` from a name's first character at `q`: the shortest run of name characters
/// that a `\b` follows.
pub(crate) fn lazy_name(l: &Line, q: usize) -> Option<usize> {
    let mut e = q + 1;
    loop {
        if boundary(l, e) {
            return Some(e);
        }
        if !l.is(e, is_name) {
            return None;
        }
        e += 1;
    }
}
