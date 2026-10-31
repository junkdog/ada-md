//! INI files, in place of bat's `INI`: a line is a comment, a `[section]`, or a `key = value`
//! declaration, whose key and value may be quoted and which may end in a `;` comment. A line
//! doesn't depend on the ones before it.

use crate::{Out, skip_spaces, trim_end};

const COMMENT: u16 = 0;
const COMMENT_MARK: u16 = 1;
const SECTION: u16 = 2;
const SECTION_MARK: u16 = 3;
const SECTION_NAME: u16 = 4;
const SECTION_COMMENT: u16 = 5;
const DECLARATION: u16 = 6;
const PROPERTY: u16 = 7;
const KEY_QUOTE: u16 = 8;
const KEY: u16 = 9;
const EQUALS: u16 = 10;
const VALUE_QUOTE: u16 = 11;
const VALUE: u16 = 12;
const COMMENT_AFTER: u16 = 13;

/// Each kind's scopes under `source.ini`, as bat's grammar gives them.
pub const KINDS: &[&str] = &[
    "comment.line.semicolon.ini",
    "comment.line.semicolon.ini punctuation.definition.comment.ini",
    "meta.tag.section.ini",
    "meta.tag.section.ini punctuation.definition.section.ini",
    "meta.tag.section.ini entity.section.ini",
    "meta.tag.section.ini comment.definition.section.ini",
    "meta.declaration.ini",
    "meta.declaration.ini meta.property.ini",
    "meta.declaration.ini meta.property.ini punctuation.definition.quote.ini",
    "meta.declaration.ini meta.property.ini keyword.name.ini",
    "meta.declaration.ini meta.property.ini punctuation.definition.equals.ini",
    "meta.declaration.ini meta.value.ini punctuation.definition.quote.ini",
    "meta.declaration.ini meta.value.ini string.name.value.ini",
    "meta.declaration.ini comment.declarationline.semicolon.ini",
];

/// One line, by the first of the grammar's three patterns that matches it; the last matches
/// any line.
#[inline(never)]
pub fn line(b: &[u8], out: &mut Out) {
    let len = b.len();
    let start = skip_spaces(b, 0);
    let mut at = 0;
    let mut put = |end: usize, kind: u16| {
        out.push(at, end, kind);
        at = end;
    };
    // `^\s*(;|#).*$`
    if matches!(b.get(start), Some(b';' | b'#')) {
        put(start, COMMENT);
        put(start + 1, COMMENT_MARK);
        put(len, COMMENT);
        return;
    }
    // `^\s*(\[)(.*?)(\])\s*(;.*)?$`: the name ends at the first `]` that only spaces and a
    // comment follow
    if b.get(start) == Some(&b'[') {
        let close = (start + 1..len).find(|&i| b[i] == b']' && ends(b, i + 1).is_some());
        if let Some(close) = close {
            put(start, SECTION);
            put(start + 1, SECTION_MARK);
            put(close, SECTION_NAME);
            put(close + 1, SECTION_MARK);
            let comment = ends(b, close + 1).unwrap_or(len);
            put(comment, SECTION);
            put(len, SECTION_COMMENT);
            return;
        }
    }
    // `^(\s*(["']?)(.+?)(\2)\s*(=))?\s*((["']?)(.*?)(\7))\s*(;.*)?$`
    let mut value = 0;
    if let Some((key, quoted, equals)) = key(b, start) {
        let quote = usize::from(quoted);
        put(key.0 - quote, PROPERTY);
        put(key.0, KEY_QUOTE);
        put(key.1, KEY);
        put(key.1 + quote, KEY_QUOTE);
        put(equals, PROPERTY);
        put(equals + 1, EQUALS);
        value = equals + 1;
    }
    let value = skip_spaces(b, value);
    put(value, DECLARATION);
    let quoted = b
        .get(value)
        .filter(|&&q| q == b'"' || q == b'\'')
        .and_then(|&q| (value + 1..len).find(|&i| b[i] == q && ends(b, i + 1).is_some()));
    let comment = match quoted {
        Some(close) => {
            put(value + 1, VALUE_QUOTE);
            put(close, VALUE);
            put(close + 1, VALUE_QUOTE);
            ends(b, close + 1).unwrap_or(len)
        }
        None => {
            // the text ends where only spaces and a comment follow: before the first `;`
            let semi = (value..len).find(|&i| b[i] == b';').unwrap_or(len);
            put(trim_end(b, value, semi), VALUE);
            semi
        }
    };
    put(comment, DECLARATION);
    put(len, COMMENT_AFTER);
}

/// Where the comment starts when only spaces and a `;` comment follow `at`, `\s*(;.*)?$`, or
/// the line's end when only spaces do; `None` when something else does.
fn ends(b: &[u8], at: usize) -> Option<usize> {
    let at = skip_spaces(b, at);
    (at == b.len() || b[at] == b';').then_some(at)
}

/// A declaration's key, `\s*(["']?)(.+?)(\2)\s*(=)`, the spaces before it ending at `start`:
/// the key's text, whether it's quoted, and where its `=` is.
fn key(b: &[u8], start: usize) -> Option<((usize, usize), bool, usize)> {
    let len = b.len();
    if let Some(&q) = b.get(start).filter(|&&q| q == b'"' || q == b'\'') {
        let close =
            (start + 2..len).find(|&i| b[i] == q && b.get(skip_spaces(b, i + 1)) == Some(&b'='));
        if let Some(close) = close {
            return Some(((start + 1, close), true, skip_spaces(b, close + 1)));
        }
    }
    // `.+?` takes a byte at least, and stops before the spaces ahead of the first `=`
    if let Some(equals) = (start + 1..len).find(|&i| b[i] == b'=') {
        return Some(((start, trim_end(b, start + 1, equals)), false, equals));
    }
    // a key of one space, given back by `\s*`, before an `=` that starts the text
    (start > 0 && b.get(start) == Some(&b'=')).then(|| ((start - 1, start), false, start))
}

#[cfg(test)]
mod tests {
    use crate::Lang;
    use crate::tests::marked;

    fn kinds(line: &str) -> Vec<(&str, &'static str)> {
        marked(Lang::Ini, &[line])
            .into_iter()
            .map(|(t, s)| (t, s.rsplit(' ').next().unwrap()))
            .collect()
    }

    #[test]
    fn comments_and_sections() {
        assert_eq!(
            kinds("  ; note"),
            [
                ("  ", "comment.line.semicolon.ini"),
                (";", "punctuation.definition.comment.ini"),
                (" note", "comment.line.semicolon.ini"),
            ]
        );
        assert_eq!(
            kinds("[a]b] ; c"),
            [
                ("[", "punctuation.definition.section.ini"),
                ("a]b", "entity.section.ini"),
                ("]", "punctuation.definition.section.ini"),
                (" ", "meta.tag.section.ini"),
                ("; c", "comment.definition.section.ini"),
            ]
        );
    }

    #[test]
    fn declarations() {
        assert_eq!(
            kinds(" key x = value ; c"),
            [
                (" ", "meta.property.ini"),
                ("key x", "keyword.name.ini"),
                (" ", "meta.property.ini"),
                ("=", "punctuation.definition.equals.ini"),
                (" ", "meta.declaration.ini"),
                ("value", "string.name.value.ini"),
                (" ", "meta.declaration.ini"),
                ("; c", "comment.declarationline.semicolon.ini"),
            ]
        );
        assert_eq!(
            kinds(r#""a=b"="c;d""#),
            [
                ("\"", "punctuation.definition.quote.ini"),
                ("a=b", "keyword.name.ini"),
                ("\"", "punctuation.definition.quote.ini"),
                ("=", "punctuation.definition.equals.ini"),
                ("\"", "punctuation.definition.quote.ini"),
                ("c;d", "string.name.value.ini"),
                ("\"", "punctuation.definition.quote.ini"),
            ]
        );
        assert_eq!(kinds("bare"), [("bare", "string.name.value.ini")]);
        assert_eq!(kinds(""), []);
        assert_eq!(
            kinds("  = x"),
            [
                (" ", "meta.property.ini"),
                (" ", "keyword.name.ini"),
                ("=", "punctuation.definition.equals.ini"),
                (" ", "meta.declaration.ini"),
                ("x", "string.name.value.ini"),
            ]
        );
    }
}
