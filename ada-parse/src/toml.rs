//! What TOML's scanner (`toml_scan.rs`) shares from the port of bat's `TOML` it replaced: the
//! contexts' ids, which its stack holds (syntect's, in the order the probe's `dump TOML` lists
//! them), the scopes, and the matchers of the regexes it tests at a position.

use crate::matcher::{Line, is_word};

// the contexts
pub(crate) const ARRAY_SEP_0: u8 = 0;
pub(crate) const BASIC_STRING_0: u8 = 1;
pub(crate) const INLINE_KEYVALS_0: u8 = 2;
pub(crate) const INLINE_TABLE_0: u8 = 3;
pub(crate) const STRING_0: u8 = 4;
pub(crate) const STRING_1: u8 = 5;
pub(crate) const MAIN_0: u8 = 6;
pub(crate) const START: u8 = 7;
pub(crate) const ARRAY_SEP: u8 = 9;
pub(crate) const DATA_TYPES: u8 = 13;
pub(crate) const INLINE_KEYVAL_SEP: u8 = 17;
pub(crate) const INLINE_KEYVALS: u8 = 19;
pub(crate) const KEY: u8 = 21;
pub(crate) const KEY_SIMPLE: u8 = 23;
pub(crate) const KEYVAL_SEP: u8 = 25;
pub(crate) const MAYBE_ARRAY_COMMENTS: u8 = 28;
pub(crate) const MAYBE_COMMENT_EOL: u8 = 29;
pub(crate) const MAYBE_DOT_KEY: u8 = 30;
pub(crate) const MAYBE_DOT_TABLE: u8 = 31;
pub(crate) const MAYBE_EMPTY_ARRAY: u8 = 32;
pub(crate) const TABLE_ARRAY_NAME: u8 = 35;
pub(crate) const TABLE_NAME: u8 = 36;
pub(crate) const TABLE_NAME_SIMPLE: u8 = 38;
/// The scopes, as indices into [`ATOMS`].
pub(crate) mod s {
    use crate::matcher::Atom;
    pub const ARRAY_END: Atom = 0;
    pub const BASIC: Atom = 1;
    pub const STR_END: Atom = 2;
    pub const BAD_TABLE_END: Atom = 4;
    pub const INLINE_END: Atom = 5;
    pub const TRIPLE_LITERAL: Atom = 6;
    pub const TRIPLE_BASIC: Atom = 7;
    pub const ARRAY_BEGIN: Atom = 8;
    pub const ARRAY_SEP: Atom = 9;
    pub const BAD_ARRAY: Atom = 10;
    pub const STR_BEGIN: Atom = 11;
    pub const BOOL: Atom = 12;
    pub const COMMENT: Atom = 13;
    pub const COMMENT_MARK: Atom = 14;
    pub const BAD_VALUE: Atom = 15;
    pub const DATETIME: Atom = 16;
    pub const FLOAT: Atom = 17;
    pub const SIGN: Atom = 18;
    pub const ILLEGAL: Atom = 19;
    pub const KEY_VALUE: Atom = 20;
    pub const INLINE_BEGIN: Atom = 22;
    pub const INLINE_SEP: Atom = 23;
    pub const INTEGER: Atom = 25;
    pub const META_KEY: Atom = 26;
    pub const BAD_KEY: Atom = 27;
    pub const KEY: Atom = 28;
    pub const BAD_KEYVAL: Atom = 29;
    pub const LITERAL: Atom = 30;
    pub const BAD_TRAILING: Atom = 31;
    pub const KEY_DOT: Atom = 32;
    pub const TABLE_DOT: Atom = 33;
    pub const BAD_TABLE: Atom = 34;
    pub const ESCAPE: Atom = 35;
    pub const BAD_ESCAPE: Atom = 36;
    pub const META_TABLE_ARRAY: Atom = 37;
    pub const TABLE_ARRAY_END: Atom = 38;
    pub const META_TABLE: Atom = 39;
    pub const TABLE_END: Atom = 40;
    pub const TABLE_NAME: Atom = 41;
    pub const TABLE_ARRAY_BEGIN: Atom = 42;
    pub const TABLE_BEGIN: Atom = 43;
}

pub(crate) const ATOMS: &[&str] = &[
    "punctuation.definition.array.end.toml",
    "string.quoted.double.basic.toml",
    "punctuation.definition.string.end.toml",
    "invalid.illegal.string.non-terminated.toml",
    "invalid.illegal.table-end.toml",
    "punctuation.definition.inline-table.end.toml",
    "string.quoted.triple.literal.block.toml",
    "string.quoted.triple.basic.block.toml",
    "punctuation.definition.array.begin.toml",
    "punctuation.separator.array.toml",
    "invalid.illegal.array.toml",
    "punctuation.definition.string.begin.toml",
    "constant.language.toml",
    "comment.line.number-sign.toml",
    "punctuation.definition.comment.toml",
    "invalid.illegal.value.toml",
    "constant.other.datetime.toml",
    "constant.numeric.float.toml",
    "keyword.operator.arithmetic.toml",
    "invalid.illegal.toml",
    "punctuation.definition.key-value.toml",
    "invalid.illegal.inline-sep.toml",
    "punctuation.definition.inline-table.begin.toml",
    "punctuation.separator.inline-table.toml",
    "invalid.illegal.inline-key.toml",
    "constant.numeric.integer.toml",
    "meta.tag.key.toml",
    "invalid.illegal.key.toml",
    "entity.name.tag.toml",
    "invalid.illegal.keyval.toml",
    "string.quoted.single.literal.toml",
    "invalid.illegal.trailing.toml",
    "punctuation.separator.key.toml",
    "punctuation.separator.table.toml",
    "invalid.illegal.table.toml",
    "constant.character.escape.toml",
    "invalid.illegal.string.escape.toml",
    "meta.tag.table.array.toml",
    "punctuation.definition.table.array.end.toml",
    "meta.tag.table.toml",
    "punctuation.definition.table.end.toml",
    "entity.name.table.toml",
    "punctuation.definition.table.array.begin.toml",
    "punctuation.definition.table.begin.toml",
];

// the matchers: each finds its regex's leftmost match at or after `from`

/// `[A-Za-z0-9_'"-]`, what a key or a table name starts with.
pub(crate) fn is_key_start(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'\'' | b'"' | b'-')
}

/// `[A-Za-z0-9_-]`.
pub(crate) fn is_bare(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-')
}

fn digit(l: &Line, i: usize) -> bool {
    l.is(i, |c| c.is_ascii_digit())
}

/// `(?<!\w)` at `p`.
pub(crate) fn word_start(l: &Line, p: usize) -> bool {
    p == 0 || l.at(p - 1).is_none_or(|c| !is_word(c) || c == b'\n')
}

/// `(?!\w)` at `p`.
pub(crate) fn word_end(l: &Line, p: usize) -> bool {
    l.at(p).is_none_or(|c| !is_word(c) || c == b'\n')
}

/// The end of `d(?:d|_d)*` from `at`, its first `d` there already.
pub(crate) fn digits_from(l: &Line, mut at: usize, d: impl Fn(u8) -> bool) -> usize {
    loop {
        if l.is(at, &d) {
            at += 1;
        } else if l.at(at) == Some(b'_') && l.is(at + 1, &d) {
            at += 2;
        } else {
            return at;
        }
    }
}

/// `[0-9](?:[0-9]|_[0-9])*` at `at`.
fn decimals(l: &Line, at: usize) -> Option<usize> {
    digit(l, at).then(|| digits_from(l, at + 1, |c| c.is_ascii_digit()))
}

/// `[eE][\+\-]?[0-9](?:[0-9]|_[0-9])*` at `at`.
fn exponent(l: &Line, at: usize) -> Option<usize> {
    if !l.is(at, |c| c == b'e' || c == b'E') {
        return None;
    }
    let at = at + 1 + usize::from(l.is(at + 1, |c| c == b'+' || c == b'-'));
    decimals(l, at)
}

/// The decimal integer part from `q`, in the regex's order: one digit, then a run that
/// doesn't start with `0`; each end `then` accepts.
pub(crate) fn int_part(l: &Line, q: usize, then: impl Fn(usize) -> Option<usize>) -> Option<usize> {
    if digit(l, q)
        && let Some(e) = then(q + 1)
    {
        return Some(e);
    }
    if l.is(q, |c| (b'1'..=b'9').contains(&c)) {
        let r = digits_from(l, q + 1, |c| c.is_ascii_digit());
        if r > q + 1 {
            return then(r);
        }
    }
    None
}

/// A sign at `p`, `[\+\-]?`: where the number after it starts.
pub(crate) fn sign(l: &Line, p: usize) -> usize {
    p + usize::from(l.is(p, |c| c == b'+' || c == b'-'))
}

/// The float regex's fraction or exponent at `r`, then `(?!\w)`.
pub(crate) fn fraction(l: &Line, r: usize) -> Option<usize> {
    if l.at(r) == Some(b'.')
        && let Some(e) = decimals(l, r + 1)
    {
        if word_end(l, e) {
            return Some(e);
        }
        if let Some(e) = exponent(l, e).filter(|&e| word_end(l, e)) {
            return Some(e);
        }
    }
    exponent(l, r).filter(|&e| word_end(l, e))
}

/// A class of digits.
pub(crate) type Digit = fn(u8) -> bool;

/// `[0-9]{n}` at `at`.
fn fixed_digits(l: &Line, at: usize, n: usize) -> bool {
    (at..at + n).all(|i| digit(l, i))
}

/// `[lo-hi][0-9]` at `at`.
fn pair(l: &Line, at: usize, hi: u8) -> bool {
    l.is(at, |c| (b'0'..=hi).contains(&c)) && digit(l, at + 1)
}

/// `[0-9]{4}-[0-1][0-9]-[0-3][0-9]` at `at`: its end.
pub(crate) fn date(l: &Line, at: usize) -> Option<usize> {
    let ok = fixed_digits(l, at, 4)
        && l.at(at + 4) == Some(b'-')
        && pair(l, at + 5, b'1')
        && l.at(at + 7) == Some(b'-')
        && pair(l, at + 8, b'3');
    ok.then_some(at + 10)
}

/// `[0-2][0-9]:[0-5][0-9]:[0-6][0-9](?:\.[0-9]+)?` at `at`: its end.
pub(crate) fn time(l: &Line, at: usize) -> Option<usize> {
    let ok = pair(l, at, b'2')
        && l.at(at + 2) == Some(b':')
        && pair(l, at + 3, b'5')
        && l.at(at + 5) == Some(b':')
        && pair(l, at + 6, b'6');
    if !ok {
        return None;
    }
    let e = at + 8;
    Some(match l.at(e) == Some(b'.') && digit(l, e + 1) {
        true => l.run(e + 1, |c| c.is_ascii_digit()),
        false => e,
    })
}

/// `[zZ]|[+-][0-2][0-9]:[0-5][0-9]` at `at`: its end.
pub(crate) fn offset(l: &Line, at: usize) -> Option<usize> {
    match l.at(at)? {
        b'z' | b'Z' => Some(at + 1),
        b'+' | b'-'
            if pair(l, at + 1, b'2') && l.at(at + 3) == Some(b':') && pair(l, at + 4, b'5') =>
        {
            Some(at + 6)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::Lang;
    use crate::tests::marked;

    fn kinds(lines: &[&'static str]) -> Vec<(&'static str, &'static str)> {
        marked(Lang::Toml, lines)
            .into_iter()
            .map(|(t, s)| (t, s.rsplit(' ').next().unwrap()))
            .collect()
    }

    #[test]
    fn keys_and_values() {
        assert_eq!(
            kinds(&["name = \"ada\" # why"]),
            [
                ("name", "entity.name.tag.toml"),
                ("=", "punctuation.definition.key-value.toml"),
                ("\"", "punctuation.definition.string.begin.toml"),
                ("ada", "string.quoted.double.basic.toml"),
                ("\"", "punctuation.definition.string.end.toml"),
                ("#", "punctuation.definition.comment.toml"),
                (" why", "comment.line.number-sign.toml"),
            ]
        );
    }

    #[test]
    fn arrays_run_across_lines() {
        let got = kinds(&["x = [", "  1, -2.5,", "]"]);
        assert!(
            got.contains(&("-", "keyword.operator.arithmetic.toml")),
            "{got:?}"
        );
        assert!(
            got.contains(&("1", "constant.numeric.integer.toml")),
            "{got:?}"
        );
        assert_eq!(
            got.last(),
            Some(&("]", "punctuation.definition.array.end.toml"))
        );
    }
}
