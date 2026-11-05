//! INI files, in place of bat's `INI`, on the line-format engine: a line is a comment, a
//! `[section]`, or a `key = value` declaration, whose key and value may be quoted and which may
//! end in a `;` comment. A line doesn't depend on the ones before it.

use crate::line_format::{Cursor, Format, blanks, find, leads, trim};
use crate::{Out, State};

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

/// Lex `b`, a line without its `\n`: by the first of the grammar's three patterns that matches
/// it, the last matching any line.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    crate::line_format::line::<Ini>(b, state, out);
}

struct Ini;

impl Format for Ini {
    // `^\s*(;|#).*$` and `^\s*(\[)…`
    const LEAD: [u8; 256] = leads(b";#", b"[");
    const COMMENT_TEXT: u16 = COMMENT;
    const COMMENT_MARK: u16 = COMMENT_MARK;
    const COMMENT_BLANKS: bool = true;

    // a line leaves nothing open, and a state from elsewhere is read past
    const OPENS: bool = false;

    fn resume(_: &mut Cursor, _: u64) -> u64 {
        unreachable!("INI's lines leave nothing open")
    }

    /// The third pattern's text is its blanks.
    #[inline(always)]
    fn blank(c: &mut Cursor) {
        c.put(c.len(), DECLARATION);
    }

    /// `^\s*(\[)(.*?)(\])\s*(;.*)?$`: the name ends at the first `]` that only spaces and a
    /// comment follow.
    #[inline(always)]
    fn section(c: &mut Cursor, start: usize) -> Option<u64> {
        let b = c.b;
        let mut close = start + 1;
        let comment = loop {
            close = find(b, close, *b"]");
            if close == b.len() {
                return None;
            }
            if let Some(comment) = ends(b, close + 1) {
                break comment;
            }
            close += 1;
        };
        c.put(start, SECTION);
        c.put(start + 1, SECTION_MARK);
        c.put(close, SECTION_NAME);
        c.put(close + 1, SECTION_MARK);
        c.put(comment, SECTION);
        c.put(b.len(), SECTION_COMMENT);
        Some(0)
    }

    /// `^(\s*(["']?)(.+?)(\2)\s*(=))?\s*((["']?)(.*?)(\7))\s*(;.*)?$`
    #[inline(always)]
    fn entry(c: &mut Cursor, start: usize) -> u64 {
        let b = c.b;
        let mut value = 0;
        if let Some((key, quoted, equals)) = key(b, start) {
            let quote = usize::from(quoted);
            c.put(key.0 - quote, PROPERTY);
            c.put(key.0, KEY_QUOTE);
            c.put(key.1, KEY);
            c.put(key.1 + quote, KEY_QUOTE);
            c.put(equals, PROPERTY);
            c.put(equals + 1, EQUALS);
            value = equals + 1;
        }
        let value = blanks(b, value);
        c.put(value, DECLARATION);
        let comment = match quoted_value(b, value) {
            Some(close) => {
                c.put(value + 1, VALUE_QUOTE);
                c.put(close, VALUE);
                c.put(close + 1, VALUE_QUOTE);
                ends(b, close + 1).unwrap_or(b.len())
            }
            None => {
                // the text ends where only spaces and a comment follow: before the first `;`
                let semi = find(b, value, *b";");
                c.put(trim(b, value, semi), VALUE);
                semi
            }
        };
        c.put(comment, DECLARATION);
        c.put(b.len(), COMMENT_AFTER);
        0
    }
}

/// A quoted value's closing quote, when the value at `value` starts with one and a quote of
/// its kind is followed by spaces and a comment alone.
fn quoted_value(b: &[u8], value: usize) -> Option<usize> {
    let q = *b.get(value).filter(|&&q| q == b'"' || q == b'\'')?;
    let mut close = value + 1;
    loop {
        close = find(b, close, [q]);
        if close == b.len() {
            return None;
        }
        if ends(b, close + 1).is_some() {
            return Some(close);
        }
        close += 1;
    }
}

/// Where the comment starts when only spaces and a `;` comment follow `at`, `\s*(;.*)?$`, or
/// the line's end when only spaces do; `None` when something else does.
fn ends(b: &[u8], at: usize) -> Option<usize> {
    let at = blanks(b, at);
    (at == b.len() || b[at] == b';').then_some(at)
}

/// A declaration's key, `\s*(["']?)(.+?)(\2)\s*(=)`, the spaces before it ending at `start`:
/// the key's text, whether it's quoted, and where its `=` is.
fn key(b: &[u8], start: usize) -> Option<((usize, usize), bool, usize)> {
    let len = b.len();
    if let Some(&q) = b.get(start).filter(|&&q| q == b'"' || q == b'\'') {
        let mut close = start + 2;
        while close < len {
            close = find(b, close, [q]);
            if close < len && b.get(blanks(b, close + 1)) == Some(&b'=') {
                return Some(((start + 1, close), true, blanks(b, close + 1)));
            }
            close += 1;
        }
    }
    // `.+?` takes a byte at least, and stops before the spaces ahead of the first `=`
    let equals = find(b, start + 1, *b"=");
    if equals < len {
        return Some(((start, trim(b, start + 1, equals)), false, equals));
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
