//! Java properties files by a scanner: bat's `Java Properties` compiled by hand. A line is a
//! comment, or a key, a separator and a value, each a run the scan passes in one step; a key or
//! value ending in `\` runs on into the next line, which is all the state there is.
//!
//! Exact against bat's grammar on syntect: the `parse` fuzz target holds every byte's scopes to
//! syntect's.

use crate::matcher::{Atom, atoms, is_space};
use crate::{Out, State};

atoms! {
    PROPERTY = "meta.property.java-props",
    VALUE = "string.unquoted.java-props",
    SEPARATOR = "punctuation.separator.java-props",
    CONTINUATION = "punctuation.separator.continuation.java-props",
    UNICODE = "constant.character.escape.unicode.java-props",
    ESCAPE = "constant.character.escape.java-props",
    COMMENT = "comment.line.java-props",
    COMMENT_MARK = "punctuation.definition.comment.java-props",
    KEY = "entity.name.key.java-props",
}

const COMMENT: u16 = 0;
const COMMENT_MARK: u16 = 1;
/// Between a key and its value: the separator's spaces.
const META: u16 = 2;
const SEPARATOR: u16 = 3;
/// A key's kinds; a value's are the same, [`IN_VALUE`] on.
const KEY: u16 = 4;
const ESCAPE: u16 = 5;
const UNICODE: u16 = 6;
const CONTINUATION: u16 = 7;
const IN_VALUE: u16 = 4;

/// Each kind's scopes under `source.java-props`, by kind.
static KINDS: [&[Atom]; 12] = [
    &[s::COMMENT],
    &[s::COMMENT, s::COMMENT_MARK],
    &[s::PROPERTY],
    &[s::PROPERTY, s::SEPARATOR],
    &[s::PROPERTY, s::KEY],
    &[s::PROPERTY, s::KEY, s::ESCAPE],
    &[s::PROPERTY, s::KEY, s::UNICODE],
    &[s::PROPERTY, s::KEY, s::CONTINUATION],
    &[s::PROPERTY, s::VALUE],
    &[s::PROPERTY, s::VALUE, s::ESCAPE],
    &[s::PROPERTY, s::VALUE, s::UNICODE],
    &[s::PROPERTY, s::VALUE, s::CONTINUATION],
];

/// The scopes under `source.java-props` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    KINDS.get(usize::from(kind)).map_or_else(Vec::new, |k| {
        k.iter().map(|&a| ATOMS[usize::from(a)]).collect()
    })
}

/// What a line leaves open: nothing, a key or a value that ends in `\`.
const OPEN_KEY: u64 = 1;
const OPEN_VALUE: u64 = 2;

/// Lex `b`, a line without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let open = match state.word() {
        OPEN_KEY => run(b, 0, KEY, out),
        OPEN_VALUE => run(b, 0, KEY + IN_VALUE, out),
        _ => main(b, out),
    };
    state.set_word(open);
}

/// `main`: a comment, `^\s*([#!]).*`, or a property at the first byte that isn't a space.
fn main(b: &[u8], out: &mut Out) -> u64 {
    let Some(q) = b.iter().position(|&c| !is_space(c)) else {
        return 0;
    };
    if matches!(b[q], b'#' | b'!') {
        out.push(0, q, COMMENT);
        out.push(q, q + 1, COMMENT_MARK);
        out.push(q + 1, b.len(), COMMENT);
        return 0;
    }
    run(b, q, KEY, out)
}

/// A key (`kind` [`KEY`]) or a value ([`KEY`] + [`IN_VALUE`]) from `p`, with its escapes; a
/// key goes on to its separator and value. What the line leaves open.
fn run(b: &[u8], mut p: usize, kind: u16, out: &mut Out) -> u64 {
    let value = kind != KEY;
    let mut start = p;
    loop {
        // to the next `\`, or a key's end: `(?=[:=\s])`, which the line's end is
        let stop = |c: u8| c == b'\\' || !value && (c == b':' || c == b'=' || is_space(c));
        let k = b[p..]
            .iter()
            .position(|&c| stop(c))
            .map_or(b.len(), |i| p + i);
        if k == b.len() || b[k] != b'\\' {
            out.push_merged(start, k, kind);
            if value {
                return 0;
            }
            return separator(b, k, out);
        }
        // `(\\)\n`, then `\\u[[:xdigit:]]{4}`, then `\\[^u]`
        let (end, escape) = if k + 1 == b.len() {
            (k + 1, CONTINUATION)
        } else if b[k + 1] == b'u' {
            if b.len() >= k + 6 && b[k + 2..k + 6].iter().all(u8::is_ascii_hexdigit) {
                (k + 6, UNICODE)
            } else {
                // no escape starts here
                p = k + 1;
                continue;
            }
        } else {
            (k + 1 + char_len(b, k + 1), ESCAPE)
        };
        out.push_merged(start, k, kind);
        out.push(k, end, kind + (escape - KEY));
        if escape == CONTINUATION {
            return if value { OPEN_VALUE } else { OPEN_KEY };
        }
        p = end;
        start = end;
    }
}

/// After a key, at `p`: the first `[:=]` or byte that isn't a space, whichever comes first
/// (the separator on a tie), then the value at the first byte past the separator that isn't
/// a space; the line's end closes the property.
fn separator(b: &[u8], p: usize, out: &mut Out) -> u64 {
    let Some(q) = b[p..].iter().position(|&c| !is_space(c)).map(|i| p + i) else {
        out.push(p, b.len(), META);
        return 0;
    };
    out.push(p, q, META);
    if !matches!(b[q], b':' | b'=') {
        return run(b, q, KEY + IN_VALUE, out);
    }
    out.push(q, q + 1, SEPARATOR);
    let Some(v) = b[q + 1..]
        .iter()
        .position(|&c| !is_space(c))
        .map(|i| q + 1 + i)
    else {
        out.push(q + 1, b.len(), META);
        return 0;
    };
    out.push(q + 1, v, META);
    run(b, v, KEY + IN_VALUE, out)
}

/// The length of the character at `i`, a valid UTF-8 line's.
fn char_len(b: &[u8], i: usize) -> usize {
    match b[i] {
        0..0x80 => 1,
        0xc0..0xe0 => 2,
        0xe0..0xf0 => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use crate::Lang;
    use crate::tests::marked;

    /// The last scope of each piece: a comment, a key with an escape, its separator, a value
    /// with escapes run on by `\\`, and a key run on.
    #[test]
    fn a_property_and_its_escapes() {
        let got: Vec<(&str, &str)> = marked(
            Lang::Properties,
            &["  ! c", "a\\ b = x\\u00e9\\", "  more", "k \\"],
        )
        .into_iter()
        .map(|(t, s)| (t, s.rsplit(' ').next().unwrap()))
        .collect();
        let want = [
            ("  ", "comment.line.java-props"),
            ("!", "punctuation.definition.comment.java-props"),
            (" c", "comment.line.java-props"),
            ("a", "entity.name.key.java-props"),
            ("\\ ", "constant.character.escape.java-props"),
            ("b", "entity.name.key.java-props"),
            (" ", "meta.property.java-props"),
            ("=", "punctuation.separator.java-props"),
            (" ", "meta.property.java-props"),
            ("x", "string.unquoted.java-props"),
            ("\\u00e9", "constant.character.escape.unicode.java-props"),
            ("\\", "punctuation.separator.continuation.java-props"),
            ("  more", "string.unquoted.java-props"),
            ("k", "entity.name.key.java-props"),
            (" ", "meta.property.java-props"),
            ("\\", "punctuation.separator.continuation.java-props"),
        ];
        assert_eq!(got, want);
    }
}
