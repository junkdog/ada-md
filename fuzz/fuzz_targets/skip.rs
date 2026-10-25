//! Input the targets leave out: a module of each target that uses it.
//!
//! Makefile highlighting: syntect's grammar backtracks ~n³ on unbalanced `(` before a `:`
//! (G-002), so every `print` slow unit since round 6 was a Makefile fence. It stays out until an
//! anpa scanner replaces the grammar, post-1.0; then this goes.

/// Whether `text` could be highlighted as a Makefile: a fence whose tag names it (`Makefile`,
/// `GNUmakefile`, `make`, `mk`, `mak`), anywhere in a line, as a fence can sit in a list or a
/// quote and a suffix of the text can start mid-line; or a first line (`#!…make`,
/// `-*- makefile -*-`) that names it. A superset: a few inputs that wouldn't go out too.
pub fn makefile(text: &str) -> bool {
    text.lines().any(|line| {
        let lower = line.to_ascii_lowercase();
        let first_line = (lower.contains("#!") || lower.contains("-*-")) && lower.contains("make");
        first_line
            || fence_tags(&lower).any(|tag| tag.contains("make") || tag == "mk" || tag == "mak")
    })
}

/// The tag after each fence opener in a lowercase `line`, `{.` and spaces stripped.
fn fence_tags(line: &str) -> impl Iterator<Item = &str> {
    ["```", "~~~"].into_iter().flat_map(move |fence| {
        line.match_indices(fence).map(move |(at, _)| {
            let rest = line[at..].trim_start_matches(['`', '~']);
            let rest = rest.trim_start_matches([' ', '\t', '{', '.']);
            let end = rest
                .find(|c: char| c.is_whitespace() || matches!(c, ',' | '}' | '`' | '~'))
                .unwrap_or(rest.len());
            &rest[..end]
        })
    })
}
