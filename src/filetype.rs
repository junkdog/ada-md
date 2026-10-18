//! What a file holds, and so how ada shows it: markdown it renders, an image it shows, and code
//! or other text it shows whole, as one highlighted block (see [`doc::file`]).

use std::path::Path;

use crate::doc::{self, Doc};
use crate::highlight;
use crate::layout::{LayoutParams, LineNumbers};
use crate::links;

/// Text files no syntax claims, shown plain rather than handed to the system's opener.
const PLAIN: &[&str] = &["txt", "text", "log", "out", "conf", "cfg", "env", "list"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileType {
    Markdown,
    Image,
    /// Code in a language, as a syntax names it (`Rust`, `TOML`), or plain text.
    Text {
        lang: Option<&'static str>,
    },
}

impl FileType {
    /// What the path alone says, or `None` when only the contents can tell. Stdin (`-`) is
    /// markdown.
    pub fn by_path(path: &Path) -> Option<FileType> {
        if path.as_os_str() == "-" || links::is_markdown(path) {
            return Some(FileType::Markdown);
        }
        if links::is_image(path) {
            return Some(FileType::Image);
        }
        if let Some(lang) = highlight::file_language(path) {
            return Some(FileType::Text { lang: Some(lang) });
        }
        let plain = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| PLAIN.iter().any(|p| e.eq_ignore_ascii_case(p)));
        plain.then_some(FileType::Text { lang: None })
    }

    /// What a file holds, by its path and then its first line. A file neither names is plain
    /// text, unless it has no extension at all: `README`, `TODO` and the like are read as
    /// markdown, as they always were.
    pub fn of(path: &Path, text: &str) -> FileType {
        if let Some(t) = FileType::by_path(path) {
            return t;
        }
        let first = text.lines().next().unwrap_or_default();
        match highlight::first_line_language(first) {
            Some(lang) => FileType::Text { lang: Some(lang) },
            None if path.extension().is_none() => FileType::Markdown,
            None => FileType::Text { lang: None },
        }
    }

    /// Whether following a link to a file of this type opens it in ada. Images open in the
    /// full-screen view instead, and files of no known type in the system's opener.
    pub fn opens_in_ada(path: &Path) -> bool {
        matches!(
            FileType::by_path(path),
            Some(FileType::Markdown | FileType::Text { .. })
        )
    }

    /// The file `path`, holding `text`, as a document.
    pub fn parse(&self, path: &Path, text: &str) -> Doc {
        match self {
            FileType::Markdown => doc::parse(text),
            FileType::Text { lang } => doc::file::code(text, *lang),
            FileType::Image => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                doc::file::image(&name)
            }
        }
    }

    /// How a document of this type is laid out. Code isn't prose: it takes the terminal's
    /// whole width. An image takes the whole screen, but for its caption. Line numbers come out
    /// `Off` or not by whether the mode covers the file, and the same for every mode that does,
    /// so that toggling them for the other kind of file doesn't relayout this one.
    pub fn params(&self, mut params: LayoutParams) -> LayoutParams {
        let numbers = &mut params.line_numbers;
        match self {
            FileType::Markdown => {
                if *numbers < LineNumbers::All {
                    *numbers = LineNumbers::Off;
                }
            }
            FileType::Text { .. } => {
                params.max_width = None;
                *numbers = (*numbers).min(LineNumbers::Code);
            }
            FileType::Image => {
                *numbers = LineNumbers::Off;
                params.max_width = None;
                if let Some(fit) = &mut params.images {
                    fit.max_rows = fit.screen_rows.saturating_sub(1).max(1);
                }
            }
        }
        params
    }
}

/// Whether `bytes` look like a binary file: a NUL among the first 8 KiB, as git and grep judge.
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|&b| b == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn by_path(path: &str) -> Option<FileType> {
        FileType::by_path(Path::new(path))
    }

    fn lang(lang: &'static str) -> Option<FileType> {
        Some(FileType::Text { lang: Some(lang) })
    }

    #[test]
    fn by_name_and_extension() {
        assert_eq!(by_path("-"), Some(FileType::Markdown));
        assert_eq!(by_path("notes/a.MD"), Some(FileType::Markdown));
        assert_eq!(by_path("pic.jpeg"), Some(FileType::Image));
        assert_eq!(by_path("src/main.rs"), lang("Rust"));
        assert_eq!(by_path("Cargo.toml"), lang("TOML"));
        assert_eq!(by_path("Cargo.lock"), lang("TOML"));
        assert_eq!(by_path("a/Makefile"), lang("Makefile"));
        assert_eq!(by_path(".zshrc"), lang("Bourne Again Shell (bash)"));
        assert_eq!(by_path("App.kt"), lang("Kotlin"));
        assert_eq!(by_path("ci.yml"), lang("YAML"));
        assert_eq!(by_path("notes.txt"), Some(FileType::Text { lang: None }));
        assert_eq!(by_path("data.bin"), None);
        assert_eq!(by_path("README"), None);
    }

    #[test]
    fn by_first_line() {
        let of = |path: &str, text: &str| FileType::of(Path::new(path), text);
        assert_eq!(
            of("bin/run", "#!/usr/bin/env python3\nprint()\n"),
            FileType::Text {
                lang: Some("Python")
            }
        );
        assert_eq!(of("README", "# ada\n"), FileType::Markdown);
        assert_eq!(of("x.unknown", "hello\n"), FileType::Text { lang: None });
        assert_eq!(
            of("main.rs", "#!/bin/sh\n"),
            FileType::Text { lang: Some("Rust") }
        );
    }

    #[test]
    fn links_open_text_in_ada() {
        let opens = |p: &str| FileType::opens_in_ada(Path::new(p));
        assert!(opens("note.md"));
        assert!(opens("src/lib.rs"));
        assert!(opens("build.log"));
        assert!(!opens("pic.png"));
        assert!(!opens("report.pdf"));
        assert!(!opens("plain"));
    }

    #[test]
    fn line_numbers_resolve_to_off_or_one_mode_per_kind_of_file() {
        let numbers = |ft: FileType, mode| {
            let params = LayoutParams {
                width: 80,
                max_width: None,
                table_mode: Default::default(),
                code_wrap: true,
                line_numbers: mode,
                glyphs_gen: 0,
                images: None,
            };
            ft.params(params).line_numbers
        };
        let code = || FileType::Text { lang: None };
        use LineNumbers::{All, Code, Off};
        assert_eq!(
            [Off, Code, All].map(|m| numbers(code(), m)),
            [Off, Code, Code]
        );
        let md = [Off, Code, All].map(|m| numbers(FileType::Markdown, m));
        assert_eq!(md, [Off, Off, All]);
        assert_eq!(numbers(FileType::Image, All), Off);
    }

    #[test]
    fn binaries_have_nul_bytes() {
        assert!(is_binary(b"\x89PNG\r\n\x1a\n\0\0"));
        assert!(!is_binary("plain ✓ text".as_bytes()));
    }
}
