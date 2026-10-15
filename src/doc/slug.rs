//! Heading anchors (architecture §2, "Anchors and slugs").

use std::collections::HashMap;

/// GitHub's slug: lowercase, keep alphanumerics, `-` and `_`, turn spaces into `-`, drop the rest.
pub fn github(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.trim().chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if c == '-' || c == '_' {
            out.push(c);
        } else if c == ' ' {
            out.push('-');
        }
    }
    out
}

/// Hands out unique slugs, deduplicating with `-1`, `-2`, … like GitHub.
#[derive(Default)]
pub struct Slugger {
    seen: HashMap<String, u32>,
}

impl Slugger {
    pub fn slug(&mut self, text: &str) -> String {
        let base = github(text);
        match self.seen.get_mut(&base) {
            None => {
                self.seen.insert(base.clone(), 0);
                base
            }
            Some(n) => {
                *n += 1;
                let slug = format!("{base}-{n}");
                self.seen.insert(slug.clone(), 0);
                slug
            }
        }
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
    }

    #[test]
    fn duplicates_get_suffixes() {
        let mut s = Slugger::default();
        assert_eq!(s.slug("Notes"), "notes");
        assert_eq!(s.slug("Notes"), "notes-1");
        assert_eq!(s.slug("Notes"), "notes-2");
        assert_eq!(s.slug("Other"), "other");
    }

    #[test]
    fn heading_keys_ignore_case_and_spacing() {
        assert_eq!(
            heading_key("  Known   Nondeterminism "),
            "known nondeterminism"
        );
    }
}
