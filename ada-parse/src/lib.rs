//! ada's own highlighting for the most used languages, with syntect and Oniguruma left as the
//! fallback for the long tail (ADR-40).
//!
//! Phase 0 holds the [language table](Lang): it names the languages ada highlights without
//! syntect, from a fence's token, a file's name or its first line, so that nothing loads
//! syntect's syntax set to find out a language is one of them. The lexers move in with the
//! phases after it.

mod lang;

pub use lang::{ALIASES, Lang};
