//! The fence info-string micro-grammar (ADR-01): ```` ```rust title="x" ````, ```` ```{.kotlin} ````,
//! ```` ```rust,ignore ````.

use anpa::combinators::{not_empty, or, right, succeed};
use anpa::core::{ParserExtNoState, StrParser};
use anpa::parsers::{item_while, skip};
use anpa::whitespace::skip_whitespace;

fn word<'a>() -> impl StrParser<'a, &'a str> {
    not_empty(item_while(|c: char| {
        !c.is_whitespace() && !matches!(c, ',' | '{' | '}' | '.')
    }))
}

fn lang<'a>() -> impl StrParser<'a, &'a str> {
    let braced = right(
        skip('{'),
        right(skip_whitespace(), right(succeed(skip('.')), word())),
    );
    right(skip_whitespace(), or(braced, word()))
}

/// The language of a fenced code block, if the info string names one.
pub fn language(info: &str) -> Option<Box<str>> {
    lang().parse(info).result.map(Box::from)
}

#[cfg(test)]
mod tests {
    use super::language;

    #[test]
    fn info_strings() {
        assert_eq!(language("rust").as_deref(), Some("rust"));
        assert_eq!(language("  ts title=\"x.ts\"").as_deref(), Some("ts"));
        assert_eq!(language("rust,ignore").as_deref(), Some("rust"));
        assert_eq!(
            language("{.kotlin .numberLines}").as_deref(),
            Some("kotlin")
        );
        assert_eq!(language("{ kotlin }").as_deref(), Some("kotlin"));
        assert_eq!(language("c++").as_deref(), Some("c++"));
        assert_eq!(language(""), None);
        assert_eq!(language("   "), None);
    }
}
