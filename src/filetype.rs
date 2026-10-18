//! What a file holds, and so how ada shows it: markdown it renders, an image it shows, and code
//! or other text it shows whole, as one highlighted block (see [`doc::file`]).

use std::path::Path;

use crate::doc::{self, Doc};
use crate::highlight;
use crate::layout::{LayoutParams, LineNumbers};
use crate::{links, sniff};

/// Text files no syntax claims, shown plain rather than handed to the system's opener.
const PLAIN: &[&str] = &["txt", "text", "log", "out", "conf", "cfg", "env", "list"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileType {
    Markdown,
    Image,
    /// Code in a language, as a syntax names it (`Rust`, `TOML`), or plain text.
    Text {
        lang: Option<&'static str>,
    },
}

impl FileType {
    /// What the path alone says, or `None` when only the contents can tell, as for stdin (`-`).
    pub fn by_path(path: &Path) -> Option<FileType> {
        if path.as_os_str() == "-" {
            return None;
        }
        if links::is_markdown(path) {
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

    /// What a file holds, by its path and then its contents. Stdin and a file with no extension
    /// are [sniffed](sniff::sniff); a file of an extension no syntax knows only by its first line.
    /// What neither tells is plain text, kept line for line, but for a file with no extension:
    /// `README`, `TODO` and the like are read as markdown, as they always were.
    pub fn of(path: &Path, text: &str) -> FileType {
        if let Some(t) = FileType::by_path(path) {
            return t;
        }
        let stdin = path.as_os_str() == "-";
        let guess = if stdin || path.extension().is_none() {
            sniff::sniff(text)
        } else {
            let first = text.lines().next().unwrap_or_default();
            highlight::first_line_language(first).map(|lang| FileType::Text { lang: Some(lang) })
        };
        match guess {
            Some(t) => t,
            None if !stdin && path.extension().is_none() => FileType::Markdown,
            None => FileType::Text { lang: None },
        }
    }

    /// The type `-l NAME` asks for: `md` or `markdown`, a plain-text name (`txt`, `text`), or a
    /// syntax by its name or an extension (`Rust`, `rs`, `yaml`).
    pub fn named(name: &str) -> Option<FileType> {
        if ["txt", "text", "plain"]
            .iter()
            .any(|p| name.eq_ignore_ascii_case(p))
        {
            return Some(FileType::Text { lang: None });
        }
        let syntax = highlight::find_syntax(name)?;
        Some(match &*syntax.name {
            "Markdown" | "MultiMarkdown" => FileType::Markdown,
            "Plain Text" => FileType::Text { lang: None },
            lang => FileType::Text { lang: Some(lang) },
        })
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

/// What `-l` completes to: every name [`FileType::named`] takes for a language, an extension or
/// a file name of one word (`rs`, `yml`, `Makefile`), with the syntax it names. Names with
/// spaces or shell punctuation are left out; `-l` still takes them.
pub fn language_names() -> Vec<(String, &'static str)> {
    let mut names = vec![
        ("md".to_owned(), "Markdown"),
        ("txt".to_owned(), "Plain Text"),
    ];
    for syntax in highlight::syntaxes().syntaxes() {
        if syntax.hidden {
            continue;
        }
        for ext in &syntax.file_extensions {
            let word = ext
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_+-.".contains(c));
            // the name must lead back to this syntax: an extension two of them claim goes to one
            let named = || match FileType::named(ext) {
                Some(FileType::Markdown) => syntax.name == "Markdown",
                Some(FileType::Text { lang }) => lang == Some(&*syntax.name),
                _ => false,
            };
            if word && !ext.starts_with('.') && named() {
                names.push((ext.clone(), &*syntax.name));
            }
        }
    }
    names.sort();
    names.dedup_by(|a, b| a.0 == b.0);
    names
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
        assert_eq!(by_path("-"), None);
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
    fn stdin_and_extensionless_files_by_contents() {
        let of = |path: &str, text: &str| FileType::of(Path::new(path), text);
        let json = FileType::Text { lang: Some("JSON") };
        assert_eq!(of("-", "{\"a\": 1}\n"), json);
        assert_eq!(of("out", "{\"a\": 1}\n"), json);
        assert_eq!(
            of("x.unknown", "{\"a\": 1}\n"),
            FileType::Text { lang: None }
        );
        assert_eq!(of("-", "# ada\n\nA viewer.\n"), FileType::Markdown);
        // nothing stands out: stdin stays line for line, a README is still markdown
        assert_eq!(of("-", "Ada is a cat.\n"), FileType::Text { lang: None });
        assert_eq!(of("README", "Ada is a cat.\n"), FileType::Markdown);
    }

    #[test]
    fn languages_by_name() {
        let named = FileType::named;
        assert_eq!(named("md"), Some(FileType::Markdown));
        assert_eq!(named("Markdown"), Some(FileType::Markdown));
        assert_eq!(named("txt"), Some(FileType::Text { lang: None }));
        assert_eq!(named("rs"), lang("Rust"));
        assert_eq!(named("Rust"), lang("Rust"));
        assert_eq!(named("yaml"), lang("YAML"));
        assert_eq!(named("zsh"), lang("Bourne Again Shell (bash)"));
        assert_eq!(named("diff"), lang("Diff"));
        assert_eq!(named("nope"), None);
    }

    #[test]
    fn every_completed_language_is_taken() {
        let names = language_names();
        for want in [
            "md", "txt", "rs", "json", "yaml", "yml", "toml", "diff", "log", "zsh",
        ] {
            assert!(names.iter().any(|(n, _)| n == want), "{want}");
        }
        for (name, syntax) in &names {
            assert!(FileType::named(name).is_some(), "{name} ({syntax})");
        }
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
