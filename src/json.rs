//! JSON lines (O-48): a token scanner in place of bat's `JSON` grammar, whose state carries the
//! nesting, so a block was highlighted on one thread at ~3 MB/s. No tree is built: a line is
//! strings, numbers, `true`/`false`/`null`, punctuation and comments, and a string is a key when
//! a `:` comes after it.
//!
//! A string can't hold a raw newline, so the only state one line hands the next is a `/* */`
//! comment left open. Each [`Kind`] carries the scopes bat's grammar gives the same text, so
//! `[syntax]` rules written for it still apply. What the grammar marks as invalid by where it
//! stands (`foo`, `NaN`, a missing `,`) is left plain; an unknown escape is still invalid.

use anpa::core::parse;
use anpa::findbyte::{eq, find_byte};

/// What a stretch of a JSON line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    KeyBegin,
    Key,
    KeyEscape,
    KeyBadEscape,
    KeyEnd,
    StrBegin,
    Str,
    Escape,
    BadEscape,
    StrEnd,
    Integer,
    Float,
    /// The `.` of a float.
    Decimal,
    /// `true`, `false`, `null`.
    Constant,
    MappingBegin,
    MappingEnd,
    SequenceBegin,
    SequenceEnd,
    /// The `:` after a key.
    KeyValue,
    Comma,
    LineComment,
    /// `//`.
    LineCommentMark,
    BlockComment,
    /// `/*`.
    BlockCommentMark,
}

impl Kind {
    pub const ALL: [Kind; 24] = [
        Kind::KeyBegin,
        Kind::Key,
        Kind::KeyEscape,
        Kind::KeyBadEscape,
        Kind::KeyEnd,
        Kind::StrBegin,
        Kind::Str,
        Kind::Escape,
        Kind::BadEscape,
        Kind::StrEnd,
        Kind::Integer,
        Kind::Float,
        Kind::Decimal,
        Kind::Constant,
        Kind::MappingBegin,
        Kind::MappingEnd,
        Kind::SequenceBegin,
        Kind::SequenceEnd,
        Kind::KeyValue,
        Kind::Comma,
        Kind::LineComment,
        Kind::LineCommentMark,
        Kind::BlockComment,
        Kind::BlockCommentMark,
    ];

    /// The scopes under `source.json`, innermost last.
    pub fn scopes(self) -> &'static str {
        match self {
            Kind::KeyBegin => {
                "meta.mapping.key.json string.quoted.double.json punctuation.definition.string.begin.json"
            }
            Kind::Key => "meta.mapping.key.json string.quoted.double.json",
            Kind::KeyEscape => {
                "meta.mapping.key.json string.quoted.double.json constant.character.escape.json"
            }
            Kind::KeyBadEscape => {
                "meta.mapping.key.json string.quoted.double.json invalid.illegal.unrecognized-string-escape.json"
            }
            Kind::KeyEnd => {
                "meta.mapping.key.json string.quoted.double.json punctuation.definition.string.end.json"
            }
            Kind::StrBegin => "string.quoted.double.json punctuation.definition.string.begin.json",
            Kind::Str => "string.quoted.double.json",
            Kind::Escape => "string.quoted.double.json constant.character.escape.json",
            Kind::BadEscape => {
                "string.quoted.double.json invalid.illegal.unrecognized-string-escape.json"
            }
            Kind::StrEnd => "string.quoted.double.json punctuation.definition.string.end.json",
            Kind::Integer => "constant.numeric.integer.decimal.json",
            Kind::Float => "constant.numeric.float.decimal.json",
            Kind::Decimal => {
                "constant.numeric.float.decimal.json punctuation.separator.decimal.json"
            }
            Kind::Constant => "constant.language.json",
            Kind::MappingBegin => "meta.mapping.json punctuation.section.mapping.begin.json",
            Kind::MappingEnd => "meta.mapping.json punctuation.section.mapping.end.json",
            Kind::SequenceBegin => "meta.sequence.json punctuation.section.sequence.begin.json",
            Kind::SequenceEnd => "meta.sequence.json punctuation.section.sequence.end.json",
            Kind::KeyValue => "meta.mapping.json punctuation.separator.mapping.key-value.json",
            // bat tells a pair's `,` from a sequence's by the nesting, which no line knows
            Kind::Comma => "punctuation.separator.json",
            Kind::LineComment => "comment.line.double-slash.js",
            Kind::LineCommentMark => {
                "comment.line.double-slash.js punctuation.definition.comment.json"
            }
            Kind::BlockComment => "comment.block.json",
            Kind::BlockCommentMark => "comment.block.json punctuation.definition.comment.json",
        }
    }
}

/// Bytes `start..end` of a line are a `kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub kind: Kind,
}

/// The spans of one line, in order, into `spans` (cleared first). `comment` says whether a
/// `/* */` comment is open at the line's start, and is left saying whether one is at its end.
pub fn tokens(line: &str, comment: &mut bool, spans: &mut Vec<Span>) {
    spans.clear();
    let b = line.as_bytes();
    let mut push = |start: usize, end: usize, kind: Kind| {
        spans.push(Span {
            start: start as u32,
            end: end as u32,
            kind,
        });
    };
    let mut i = 0;
    if *comment {
        match find(b, 0, b"*/") {
            Some(end) => {
                push(0, end + 2, Kind::BlockComment);
                i = end + 2;
                *comment = false;
            }
            None => {
                if !b.is_empty() {
                    push(0, b.len(), Kind::BlockComment);
                }
                return;
            }
        }
    }
    while i < b.len() {
        let start = i;
        match b[i] {
            b'"' => i = string(b, i, &mut push),
            b'{' => (i, _) = (i + 1, push(i, i + 1, Kind::MappingBegin)),
            b'}' => (i, _) = (i + 1, push(i, i + 1, Kind::MappingEnd)),
            b'[' => (i, _) = (i + 1, push(i, i + 1, Kind::SequenceBegin)),
            b']' => (i, _) = (i + 1, push(i, i + 1, Kind::SequenceEnd)),
            b':' => (i, _) = (i + 1, push(i, i + 1, Kind::KeyValue)),
            b',' => (i, _) = (i + 1, push(i, i + 1, Kind::Comma)),
            b'-' | b'0'..=b'9' => i = number(b, i, &mut push),
            b'/' if b.get(i + 1) == Some(&b'/') => {
                push(i, i + 2, Kind::LineCommentMark);
                if b.len() > i + 2 {
                    push(i + 2, b.len(), Kind::LineComment);
                }
                return;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                push(i, i + 2, Kind::BlockCommentMark);
                match find(b, i + 2, b"*/") {
                    Some(end) => {
                        push(i + 2, end + 2, Kind::BlockComment);
                        i = end + 2;
                    }
                    None => {
                        if b.len() > i + 2 {
                            push(i + 2, b.len(), Kind::BlockComment);
                        }
                        *comment = true;
                        return;
                    }
                }
            }
            c if is_word(c) => {
                i += b[i..].iter().take_while(|&&c| is_word(c)).count();
                if let b"true" | b"false" | b"null" = &b[start..i] {
                    push(start, i, Kind::Constant);
                }
            }
            // spaces, and what JSON doesn't have: left plain
            _ => i += 1,
        }
    }
}

/// A bare word: `true`, `NaN`, an unquoted key. Non-ASCII bytes count, so a scan never stops
/// inside a character.
fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80
}

/// Where `needle` next starts in `b`, from `from`.
fn find(b: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    b[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// The string opened at `i`, up to its closing quote or the line's end; where it stops.
fn string(b: &[u8], i: usize, push: &mut impl FnMut(usize, usize, Kind)) -> usize {
    // a key when a `:` comes after it, so the end is found first
    let end = string_end(b, i + 1);
    let close = end < b.len();
    let after = if close { end + 1 } else { end };
    let key = close
        && b[after..]
            .iter()
            .find(|&&c| c != b' ')
            .is_some_and(|&c| c == b':');
    let (begin, text, escape, bad, end_kind) = match key {
        true => (
            Kind::KeyBegin,
            Kind::Key,
            Kind::KeyEscape,
            Kind::KeyBadEscape,
            Kind::KeyEnd,
        ),
        false => (
            Kind::StrBegin,
            Kind::Str,
            Kind::Escape,
            Kind::BadEscape,
            Kind::StrEnd,
        ),
    };
    push(i, i + 1, begin);
    let mut at = i + 1;
    while at < end {
        let stop = next_special(b, at, end);
        if stop > at {
            push(at, stop, text);
        }
        if stop == end {
            break;
        }
        if stop + 1 == end {
            // a `\` ending the line escapes nothing
            push(stop, end, text);
            break;
        }
        let (len, ok) = escape_len(&b[stop..end]);
        push(stop, stop + len, if ok { escape } else { bad });
        at = stop + len;
    }
    if close {
        push(end, end + 1, end_kind);
    }
    after
}

/// The closing quote of a string whose text starts at `at`, or the line's length when it has
/// none.
fn string_end(b: &[u8], mut at: usize) -> usize {
    loop {
        let stop = next_special(b, at, b.len());
        match b.get(stop) {
            Some(b'"') => return stop,
            // past the escaped byte; a `\` ending the line escapes nothing
            Some(_) => at = (stop + 2).min(b.len()),
            None => return b.len(),
        }
    }
}

/// The next `"` or `\` in `b[at..end]`, or `end`: anpa's scan, 8 bytes at a time.
fn next_special(b: &[u8], at: usize, end: usize) -> usize {
    let rest = &b[at..end];
    let found = parse(find_byte(eq(b'"') | eq(b'\\'), false), rest);
    match found.result {
        Some(_) => end - found.state.len(),
        None => end,
    }
}

/// How long the escape at the start of `b` is, and whether JSON has it. `b` holds the byte
/// after the `\`.
fn escape_len(b: &[u8]) -> (usize, bool) {
    match b.get(1) {
        None => (1, false),
        Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => (2, true),
        Some(b'u') if b.len() >= 6 && b[2..6].iter().all(u8::is_ascii_hexdigit) => (6, true),
        Some(&c) => {
            // the whole character after the `\`
            let len = match c {
                0xf0.. => 4,
                0xe0.. => 3,
                0xc0.. => 2,
                _ => 1,
            };
            ((1 + len).min(b.len()), false)
        }
    }
}

/// The number at `i`: `-?digits(.digits)?([eE][+-]?digits)?`, a float when it has a fraction or
/// an exponent. A `-` with no digit after it is left plain.
fn number(b: &[u8], i: usize, push: &mut impl FnMut(usize, usize, Kind)) -> usize {
    let digits = |at: usize| at + b[at..].iter().take_while(|c| c.is_ascii_digit()).count();
    let int_start = i + usize::from(b[i] == b'-');
    let int_end = digits(int_start);
    if int_end == int_start {
        return i + 1;
    }
    let mut end = int_end;
    let mut dot = None;
    if b.get(end) == Some(&b'.') && b.get(end + 1).is_some_and(u8::is_ascii_digit) {
        dot = Some(end);
        end = digits(end + 1);
    }
    let mut exp = end;
    if matches!(b.get(exp), Some(b'e' | b'E')) {
        exp += 1;
        if matches!(b.get(exp), Some(b'+' | b'-')) {
            exp += 1;
        }
        if b.get(exp).is_some_and(u8::is_ascii_digit) {
            end = digits(exp);
        }
    }
    match dot {
        Some(dot) => {
            push(i, dot, Kind::Float);
            push(dot, dot + 1, Kind::Decimal);
            push(dot + 1, end, Kind::Float);
        }
        None if end > int_end => push(i, end, Kind::Float),
        None => push(i, end, Kind::Integer),
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The marked pieces of `lines`, as `(text, kind)`, with a `/* */` comment carried over.
    fn marked<'a>(lines: &[&'a str]) -> Vec<(&'a str, Kind)> {
        let (mut spans, mut comment, mut out) = (Vec::new(), false, Vec::new());
        for line in lines {
            tokens(line, &mut comment, &mut spans);
            let mut last = 0;
            for s in &spans {
                assert!(s.start >= last && s.end > s.start, "{line}: {spans:?}");
                last = s.end;
            }
            assert!(last as usize <= line.len());
            out.extend(
                spans
                    .iter()
                    .map(|s| (&line[s.start as usize..s.end as usize], s.kind)),
            );
        }
        out
    }

    use Kind::*;

    #[test]
    fn keys_and_values() {
        assert_eq!(
            marked(&[r#"  "name" : "ada", "ok": true, "n": null, "#]),
            [
                ("\"", KeyBegin),
                ("name", Key),
                ("\"", KeyEnd),
                (":", KeyValue),
                ("\"", StrBegin),
                ("ada", Str),
                ("\"", StrEnd),
                (",", Comma),
                ("\"", KeyBegin),
                ("ok", Key),
                ("\"", KeyEnd),
                (":", KeyValue),
                ("true", Constant),
                (",", Comma),
                ("\"", KeyBegin),
                ("n", Key),
                ("\"", KeyEnd),
                (":", KeyValue),
                ("null", Constant),
                (",", Comma),
            ]
        );
    }

    #[test]
    fn punctuation() {
        assert_eq!(
            marked(&["[{}, []]"]),
            [
                ("[", SequenceBegin),
                ("{", MappingBegin),
                ("}", MappingEnd),
                (",", Comma),
                ("[", SequenceBegin),
                ("]", SequenceEnd),
                ("]", SequenceEnd),
            ]
        );
    }

    #[test]
    fn escapes() {
        assert_eq!(
            marked(&[&format!(r#""a\"b\{}\uZZ\é\""#, "u00e9")]),
            [
                ("\"", StrBegin),
                ("a", Str),
                ("\\\"", Escape),
                ("b", Str),
                ("\\u00e9", Escape),
                ("\\u", BadEscape),
                ("ZZ", Str),
                ("\\é", BadEscape),
                ("\\\"", Escape),
            ]
        );
        assert_eq!(
            marked(&[r#""k\"" : 1"#]),
            [
                ("\"", KeyBegin),
                ("k", Key),
                ("\\\"", KeyEscape),
                ("\"", KeyEnd),
                (":", KeyValue),
                ("1", Integer),
            ]
        );
    }

    /// A string left open runs to the line's end; a `\` ending it escapes nothing.
    #[test]
    fn open_strings() {
        assert_eq!(
            marked(&[r#""a": "b\"#, r#"c"#]),
            [
                ("\"", KeyBegin),
                ("a", Key),
                ("\"", KeyEnd),
                (":", KeyValue),
                ("\"", StrBegin),
                ("b", Str),
                ("\\", Str),
            ]
        );
    }

    #[test]
    fn numbers() {
        assert_eq!(
            marked(&["[0, -12, 1.5, -0.25e+3, 1E9, 1., -, 2x]"])
                .into_iter()
                .filter(|(_, k)| *k != Comma)
                .collect::<Vec<_>>(),
            [
                ("[", SequenceBegin),
                ("0", Integer),
                ("-12", Integer),
                ("1", Float),
                (".", Decimal),
                ("5", Float),
                ("-0", Float),
                (".", Decimal),
                ("25e+3", Float),
                ("1E9", Float),
                ("1", Integer),
                ("2", Integer),
                ("]", SequenceEnd),
            ]
        );
    }

    /// Bare words other than the three constants are left plain, `trueish` among them.
    #[test]
    fn words() {
        assert_eq!(
            marked(&["[NaN, trueish, false, Ünïcode]"]),
            [
                ("[", SequenceBegin),
                (",", Comma),
                (",", Comma),
                ("false", Constant),
                (",", Comma),
                ("]", SequenceEnd),
            ]
        );
    }

    #[test]
    fn comments() {
        assert_eq!(
            marked(&[
                r#"{"a": 1, // one"#,
                r#" /* two"#,
                r#""#,
                r#"  "/*" */ "b": "//" /**/ }"#,
            ]),
            [
                ("{", MappingBegin),
                ("\"", KeyBegin),
                ("a", Key),
                ("\"", KeyEnd),
                (":", KeyValue),
                ("1", Integer),
                (",", Comma),
                ("//", LineCommentMark),
                (" one", LineComment),
                ("/*", BlockCommentMark),
                (" two", BlockComment),
                ("  \"/*\" */", BlockComment),
                ("\"", KeyBegin),
                ("b", Key),
                ("\"", KeyEnd),
                (":", KeyValue),
                ("\"", StrBegin),
                ("//", Str),
                ("\"", StrEnd),
                ("/*", BlockCommentMark),
                ("*/", BlockComment),
                ("}", MappingEnd),
            ]
        );
    }
}
