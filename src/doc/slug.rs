//! Heading anchors (architecture §2, "Anchors and slugs").

use std::collections::HashMap;
use std::collections::hash_map::Entry;

/// GitHub's slug: lowercase, keep alphanumerics, `-` and `_`, turn spaces into `-`, drop the rest.
pub fn github(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.trim().chars() {
        // ASCII first, without the Unicode tables: 2.6–5× on headings (experiment 65)
        match c {
            'a'..='z' | '0'..='9' | '-' | '_' => out.push(c),
            'A'..='Z' => out.push(c.to_ascii_lowercase()),
            ' ' => out.push('-'),
            c if c.is_ascii() => {}
            c if c.is_alphanumeric() => out.extend(c.to_lowercase()),
            _ => {}
        }
    }
    out
}

/// Hands out unique slugs, deduplicating with `-1`, `-2`, … like GitHub. Its map hashes with
/// foldhash: slugs are short, and a document's headings needn't resist crafted collisions.
#[derive(Default)]
pub struct Slugger {
    seen: HashMap<String, u32, foldhash::fast::FixedState>,
}

impl Slugger {
    pub fn slug(&mut self, text: &str) -> String {
        // one lookup for a new slug, the usual case
        let slug = match self.seen.entry(github(text)) {
            Entry::Vacant(e) => {
                let slug = e.key().clone();
                e.insert(0);
                return slug;
            }
            Entry::Occupied(mut e) => {
                *e.get_mut() += 1;
                format!("{}-{}", e.key(), e.get())
            }
        };
        self.seen.insert(slug.clone(), 0);
        slug
    }
}

/// Wiki-style heading match: case-insensitive, whitespace collapsed.
pub fn heading_key(text: &str) -> String {
    text.split_whitespace()
        .flat_map(|w| w.chars().flat_map(char::to_lowercase).chain([' ']))
        .collect::<String>()
        .trim_end()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_slugs() {
        assert_eq!(github("Hello, World!"), "hello-world");
        assert_eq!(
            github("P1 · Core document (~3k lines)"),
            "p1--core-document-3k-lines"
        );
        assert_eq!(
            github("ADR-07 · Syntax_highlighting"),
            "adr-07--syntax_highlighting"
        );
        assert_eq!(github("Ünïcödé Heading"), "ünïcödé-heading");
        assert_eq!(github("`code` in heading"), "code-in-heading");
        assert_eq!(github("ΣΊΣΥΦΟΣ İstanbul ǅ 9½"), "σίσυφοσ-i̇stanbul-ǆ-9½");
    }

    #[test]
    fn duplicates_get_suffixes() {
        let mut s = Slugger::default();
        assert_eq!(s.slug("Notes"), "notes");
        assert_eq!(s.slug("Notes"), "notes-1");
        assert_eq!(s.slug("Notes"), "notes-2");
        assert_eq!(s.slug("Other"), "other");
        // a heading that reads as an earlier one's suffix
        assert_eq!(s.slug("Notes 1"), "notes-1-1");
    }

    #[test]
    fn heading_keys_ignore_case_and_spacing() {
        assert_eq!(
            heading_key("  Known   Nondeterminism "),
            "known nondeterminism"
        );
    }
}
