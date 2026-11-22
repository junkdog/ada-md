//! What a file holds, and so how ada shows it: markdown it renders, an image it shows, and code
//! or other text it shows whole, as one highlighted block (see [`doc::file`]).

use std::borrow::Cow;
use std::path::Path;

use crate::doc::{self, Dialect, Doc, Embeds};
use crate::highlight;
use crate::layout::{LayoutParams, LineNumbers};
use crate::{links, sniff};

/// The language of a header named `name` by its `text`: C, C++, Objective-C or Objective-C++
/// for a `.h` (X-134; bat's set names every `.h` Objective-C); a C or a C++ CMake template for
/// a `.h` header's (`config.h.in`, `x.h.cmake`, X-140); `None` for any other name. As bat, a
/// suffix it ignores comes off first where the name is neither (`x.h.orig`, X-145).
pub(crate) fn header_by_text(name: &str, text: &str) -> Option<ada_parse::Lang> {
    use ada_parse::Lang;
    if Lang::is_header(name) {
        return Some(Lang::for_header(text.as_bytes()));
    }
    if Lang::header_template(name).is_some() {
        let (stem, _) = name.rsplit_once('.')?;
        return Lang::is_header(stem).then(|| Lang::for_header_template(text.as_bytes()));
    }
    header_by_text(Lang::strip_ignored_suffix(name)?, text)
}

/// The language of an assembly file at `path` by its `text` (X-171): x86's or ARM's for a
/// `.s`, `.S` or `.asm` (bat's set names `.s` ARM's, though most are GAS's x86-64), where the
/// text doesn't say its directory's (`arch/x86/`, X-180), else its name's; a `.i` 68k's where
/// its text says, else `None` ([`ada_parse::Lang::for_include`], X-186); `None` for any other
/// name. As bat, a suffix it ignores comes off first where the name is neither.
pub(crate) fn assembly_by_text(path: &Path, text: &str) -> Option<ada_parse::Lang> {
    use ada_parse::Lang;
    fn by_name(name: &str) -> Option<Lang> {
        Lang::assembly_by_text(name).or_else(|| by_name(Lang::strip_ignored_suffix(name)?))
    }
    let file = path.file_name()?.to_str()?;
    if Lang::is_include(file) {
        // the Amiga's and the Atari ST's includes 68k's by their text, SWIG's and `gcc -E`'s
        // `.i` plain (X-186)
        return Lang::for_include(text.as_bytes());
    }
    let name = by_name(file)?;
    let dir = path.to_str().and_then(Lang::assembly_by_dir);
    Some(Lang::for_assembly(text.as_bytes(), dir.unwrap_or(name)))
}

/// Text files no syntax claims, shown plain rather than handed to the system's opener.
const PLAIN: &[&str] = &["txt", "text", "log", "out", "conf", "cfg", "env", "list"];

/// Mermaid files, by extension or as `-l` names them: drawn like a mermaid block (ADR-35).
const MERMAID: &[&str] = &["mmd", "mermaid"];

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
    /// A mermaid file: one code block in mermaid, drawn when ada draws its kind.
    pub const MERMAID: FileType = FileType::Text {
        lang: Some(crate::diagram::LANG),
    };

    /// A man page as `man` shows it (X-188): its bold and underline, struck over or as escapes,
    /// read without them, the page highlighted as ada reads it.
    pub const MANPAGE: FileType = FileType::Text {
        lang: Some("Manpage"),
    };

    /// Text in its own colors (FEAT-022): colored stdin that isn't a diff ada colors itself, or
    /// any text `-l ansi` names.
    pub const ANSI: FileType = FileType::Text {
        lang: Some(highlight::ANSI),
    };

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
        // cargo's config, named so before `config.toml` (X-179): TOML, which only its path tells
        // from a Git config of the same sections (`[alias]`, `[http]`)
        if path.file_name().is_some_and(|n| n == "config")
            && path
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|d| d == ".cargo")
        {
            return Some(FileType::Text {
                lang: Some(ada_parse::Lang::Toml.label()),
            });
        }
        let ext = path.extension().and_then(|e| e.to_str());
        if ext.is_some_and(is_mermaid) {
            return Some(FileType::MERMAID);
        }
        let named = highlight::named_language(path);
        if let Some(Some(lang)) = named {
            // markdown that only its language's table names (`README.md.orig`, X-189) still renders
            return Some(if lang == "Markdown" {
                FileType::Markdown
            } else {
                FileType::Text { lang: Some(lang) }
            });
        }
        let plain = ext.is_some_and(|e| PLAIN.iter().any(|p| e.eq_ignore_ascii_case(p)));
        if plain {
            return Some(FileType::Text { lang: None });
        }
        if named.is_some() {
            return None;
        }
        // as bat: a name nothing claims, without a suffix it ignores (`README.md.orig`,
        // `notes.txt.bak`, `Makefile.in`, X-145)
        let name = path.file_name()?.to_str()?;
        let rest = ada_parse::Lang::strip_ignored_suffix(name)?;
        FileType::by_path(&path.with_file_name(rest))
    }

    /// What a file holds, by its path and then its contents. Stdin and a file with no extension
    /// are [sniffed](sniff::sniff); a file of an extension no syntax knows only by its first line.
    /// What neither tells is plain text, kept line for line, but for a file with no extension:
    /// `README`, `TODO` and the like are read as markdown, as they always were. Colored stdin is
    /// sniffed without its escapes, and keeps its own colors unless that reads as a diff whose
    /// colors say no more than its marks ([`highlight::keeps_own_colors`], FEAT-022). `man`'s
    /// text struck over itself (bold, underlined) without colors too (PGR-007); `man`'s page,
    /// in either form, is highlighted as a man page (X-188).
    pub fn of(path: &Path, text: &str) -> FileType {
        let bytes = text.as_bytes();
        if is_stdin(path)
            && (crate::stdin::has_escape(bytes) || crate::stdin::has_overstrike(bytes))
        {
            // as it shows: no escape, and no character struck over another
            let plain = match crate::stdin::has_overstrike(bytes) {
                true => highlight::ansi_shown(text),
                false => crate::stdin::strip(text),
            };
            let filetype = FileType::of_plain(path, &plain);
            if filetype == FileType::MANPAGE {
                return filetype;
            }
            let diff = filetype == (FileType::Text { lang: Some("Diff") });
            return match highlight::keeps_own_colors(text, diff) {
                true => FileType::ANSI,
                false => filetype,
            };
        }
        FileType::of_plain(path, text)
    }

    /// [`FileType::of`] text that shows as it is.
    fn of_plain(path: &Path, text: &str) -> FileType {
        let name = path.file_name().and_then(|n| n.to_str());
        if let Some(lang) = name
            .and_then(|n| header_by_text(n, text))
            .or_else(|| assembly_by_text(path, text))
        {
            return FileType::Text {
                lang: Some(lang.label()),
            };
        }
        if let Some(t) = FileType::by_path(path) {
            return t;
        }
        let stdin = is_stdin(path);
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

    /// The type `-l NAME` asks for: `md` or `markdown`, a plain-text name (`txt`, `text`),
    /// `mmd` or `mermaid`, or a syntax by its name or an extension (`Rust`, `rs`, `yaml`).
    pub fn named(name: &str) -> Option<FileType> {
        if name.eq_ignore_ascii_case("ansi") {
            return Some(FileType::ANSI);
        }
        if ["txt", "text", "plain"]
            .iter()
            .any(|p| name.eq_ignore_ascii_case(p))
        {
            return Some(FileType::Text { lang: None });
        }
        if is_mermaid(name) {
            return Some(FileType::MERMAID);
        }
        let syntax = highlight::find_syntax(name)?;
        Some(match syntax.label() {
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

    /// The file `path`, holding `text`, as a document; markdown read in `dialect` (ADR-45).
    pub fn parse(&self, path: &Path, text: &str, dialect: Dialect) -> Doc {
        self.parse_with(path, text, dialect, &Embeds::default())
    }

    /// [`FileType::parse`], a note transcluding the notes its embeds name from `embeds`, as its
    /// load read them (OBS-007).
    pub fn parse_with(&self, path: &Path, text: &str, dialect: Dialect, embeds: &Embeds) -> Doc {
        if *self == FileType::ANSI {
            return doc::file::colored(text);
        }
        let text = &*self.shown_text(path, text);
        match self {
            FileType::Markdown => doc::parse_with(text, dialect, path, embeds),
            FileType::Text { lang } => doc::file::code(text, *lang),
            FileType::Image => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                doc::file::image(&name)
            }
        }
    }

    /// The text `path`'s document of this type shows, of `text` as read: stdin without its
    /// escapes, unless it is in its own colors (FEAT-022), a man page without its characters
    /// struck over too (X-188); a file as it is.
    pub fn shown_text<'a>(&self, path: &Path, text: &'a str) -> Cow<'a, str> {
        if is_stdin(path) && *self == FileType::MANPAGE {
            highlight::ansi_shown(text)
        } else if is_stdin(path) && *self != FileType::ANSI {
            crate::stdin::strip(text)
        } else {
            Cow::Borrowed(text)
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
                // `man` filled its page to the terminal's width (one column short of it), so
                // the viewer takes no columns of its own either, or every full line wraps (X-188)
                if *self == FileType::MANPAGE {
                    params.bare = true;
                }
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
        // text in the colors its escapes give it (FEAT-022)
        ("ansi".to_owned(), "ANSI"),
    ];
    names.extend(MERMAID.iter().map(|&m| (m.to_owned(), "Mermaid")));
    // bat's syntaxes, a line each: the name, then its extensions (ADR-43)
    for line in highlight::names::LIST.lines() {
        let mut fields = line.split('\t');
        let name = fields.next().unwrap_or_default();
        for ext in fields {
            let word = ext
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_+-.".contains(c));
            // the name must lead back to this syntax: an extension two of them claim goes to one
            let named = || match FileType::named(ext) {
                Some(FileType::Markdown) => name == "Markdown",
                Some(FileType::Text { lang }) => lang == Some(name),
                _ => false,
            };
            if word && !ext.starts_with('.') && named() {
                names.push((ext.to_owned(), name));
            }
        }
    }
    names.sort();
    names.dedup_by(|a, b| a.0 == b.0);
    names
}

/// Whether `path` is stdin's, `-`.
fn is_stdin(path: &Path) -> bool {
    path.as_os_str() == "-"
}

fn is_mermaid(name: &str) -> bool {
    MERMAID.iter().any(|m| name.eq_ignore_ascii_case(m))
}

/// Whether `bytes` look like a binary file: a NUL among the first 8 KiB, as git and grep judge.
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|&b| b == 0)
}

/// How many bytes [`image_ext`] looks at.
pub const IMAGE_MAGIC: usize = 8;

/// The extension of the image `bytes` start as, by its magic bytes: PNG's signature, a JPEG's
/// start of image or a GIF's header, the formats ada decodes. Stdin has no name to tell it by.
pub fn image_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else {
        None
    }
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
    fn headers_by_their_text() {
        let of = |text: &str| FileType::of(Path::new("dir/a.h"), text);
        assert_eq!(of("int f(void);\n"), lang("C").unwrap());
        assert_eq!(of("namespace a { class B; }\n"), lang("C++").unwrap());
        assert_eq!(
            of("@interface A : NSObject\n@end\n"),
            lang("Objective-C").unwrap()
        );
        assert_eq!(
            of("@class A;\nclass B {};\n"),
            lang("Objective-C++").unwrap()
        );
        // by its name alone, bat's set's: Objective-C
        assert_eq!(by_path("a.h"), lang("Objective-C"));
        assert_eq!(by_path("a.m"), lang("Objective-C"));
        assert_eq!(by_path("a.mm"), lang("Objective-C++"));
        // a header's CMake template (X-140): `.h`'s C's or C++'s by its text, the others C++'s
        let tpl = |name: &str, text: &str| FileType::of(Path::new(name), text);
        assert_eq!(
            tpl("config.h.in", "#cmakedefine X\n"),
            lang("CMake C Header").unwrap()
        );
        assert_eq!(
            tpl("llvm-config.h.cmake", "#cmakedefine X\nnamespace a {}\n"),
            lang("CMake C++ Header").unwrap()
        );
        assert_eq!(
            tpl("version.hpp.in", "#define V @V@\n"),
            lang("CMake C++ Header").unwrap()
        );
        assert_eq!(by_path("config.h.in"), lang("CMake C Header"));
        // a C source's template is C's (X-144), its text not asked
        assert_eq!(
            tpl("x.c.in", "namespace a {}\n"),
            lang("CMake C Header").unwrap()
        );
        // a suffix bat ignores off, then the text (X-145)
        assert_eq!(of("int f(void);\n"), tpl("a.h.orig", "int f(void);\n"));
        assert_eq!(
            tpl("a.h~", "namespace a { class B; }\n"),
            lang("C++").unwrap()
        );
        assert_eq!(
            tpl("config.h.in.bak", "#cmakedefine X\nnamespace a {}\n"),
            lang("CMake C++ Header").unwrap()
        );
    }

    /// An assembly file's architecture by its text (X-171), where that doesn't say its
    /// directory's, else its name's (X-180).
    #[test]
    fn assembly_by_its_text_then_its_directory() {
        let of = |path: &str, text: &str| FileType::of(Path::new(path), text);
        let (x86, arm) = (lang("x86_64 Assembly"), lang("ARM Assembly"));
        assert_eq!(Some(of("a.S", "\tmovq\t%rax, %rbx\n")), x86);
        assert_eq!(Some(of("a.asm", "\tldr\tx0, [x1]\n")), arm);
        let none = "#include <sysdep.h>\nENTRY (f)\n";
        assert_eq!(Some(of("a.S", none)), arm);
        assert_eq!(Some(of("sysdeps/x86_64/a.S", none)), x86);
        assert_eq!(Some(of("src/arm/64/a.asm", none)), arm);
        assert_eq!(Some(of("arch/x86/a.S.orig", none)), x86);
        // the file's own name is no directory
        assert_eq!(ada_parse::Lang::assembly_by_dir("x86"), None);
        // a `.i` 68k's where its text says, else plain: SWIG's, `gcc -E`'s (X-186)
        let m68k = lang("68k Assembly");
        assert_eq!(
            Some(of("exec/types.i", "\tSTRUCTURE\tLN,0\n\tLABEL\tLN_SIZE\n")),
            m68k
        );
        assert_eq!(Some(of("exec_lib.i", "_LVOOpen\tEQU\t-30\n")), m68k);
        let plain = Some(FileType::Text { lang: None });
        assert_eq!(
            Some(of("example.i", "%module example\n%include \"x.h\"\n")),
            plain
        );
        assert_eq!(Some(of("a.i", "# 1 \"a.c\"\nextern int f(void);\n")), plain);
    }

    /// A name with a suffix bat ignores is named by the rest, as bat names it, and the rest
    /// renders or shows as it would (X-145).
    #[test]
    fn suffixes_bat_ignores() {
        assert_eq!(by_path("Config.cmake.in"), lang("CMake"));
        assert_eq!(by_path("FooConfig.cmake.in"), lang("CMake"));
        assert_eq!(by_path("CMakeLists.txt.in"), lang("CMake"));
        assert_eq!(by_path("Makefile.in"), lang("Makefile"));
        assert_eq!(by_path("GNUmakefile.in"), lang("Makefile"));
        assert_eq!(by_path("setup.py.in"), lang("Python"));
        assert_eq!(by_path("x.py.in.bak"), lang("Python"));
        assert_eq!(by_path("main.rs.orig"), lang("Rust"));
        assert_eq!(by_path("app.conf.dpkg-dist"), by_path("app.conf"));
        assert_eq!(by_path("x.pl.rpmnew"), lang("Perl"));
        assert_eq!(by_path("README.md.orig"), Some(FileType::Markdown));
        assert_eq!(
            by_path("notes.txt.bak"),
            Some(FileType::Text { lang: None })
        );
        assert_eq!(by_path("pic.png~"), Some(FileType::Image));
        assert_eq!(by_path("x.in"), None);
        assert_eq!(by_path(".in"), None);
        assert_eq!(by_path("x.IN"), None);
        // the set's whole names first: `Makefile.in` is Makefile's own, `known_hosts.old`
        // Known Hosts'
        assert_eq!(by_path("known_hosts.old"), lang("Known Hosts"));
        // another language's `configure_file` template named for CMake: the language
        // (X-144), where ada lexes it
        assert_eq!(by_path("SDLEntryTestActivity.java.cmake"), lang("Java"));
        assert_eq!(by_path("AndroidManifest.xml.cmake"), lang("XML"));
        assert_eq!(
            by_path("prefix.sh.cmake"),
            lang("Bourne Again Shell (bash)")
        );
        assert_eq!(by_path("local.properties.cmake"), lang("Java Properties"));
        // pkg-config's templates pkg-config's (X-146)
        assert_eq!(by_path("protobuf.pc.cmake"), lang("pkg-config"));
        assert_eq!(by_path("zlib.pc.in"), lang("pkg-config"));
        assert_eq!(by_path("zlib.pc"), lang("pkg-config"));
        assert_eq!(by_path("prefix.sh.fish.cmake"), lang("CMake"));
        assert_eq!(by_path("arm.toolchain.cmake"), lang("CMake"));
        assert_eq!(by_path("lists.in.cmake"), lang("CMake"));
        assert_eq!(by_path("x.cmake.cmake"), lang("CMake"));
        assert_eq!(by_path("foo.pc.cmake.in"), lang("pkg-config"));
        assert_eq!(by_path("foo.java.cmake.in"), lang("Java"));
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
        assert_eq!(by_path("flow.mmd"), Some(FileType::MERMAID));
        assert_eq!(by_path("docs/flow.Mermaid"), Some(FileType::MERMAID));
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
        // cargo's config by its old name, which git's could be (X-179)
        let cargo = "[alias]\nb = \"build\"\n[http]\nproxy = \"host:3128\"\n";
        assert_eq!(
            of("/home/a/.cargo/config", cargo),
            FileType::Text { lang: Some("TOML") }
        );
        assert_eq!(
            of("/home/a/.gitconfig.local", cargo),
            FileType::Text {
                lang: Some("Git Config")
            }
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
        // a diagram, its front matter not a note's
        let flow = "---\ntitle: Flow\n---\nflowchart LR\n  A --> B\n";
        assert_eq!(of("-", flow), FileType::MERMAID);
        assert_eq!(of("flow", flow), FileType::MERMAID);
    }

    #[test]
    fn a_mermaid_file_draws_its_diagram() {
        let doc = FileType::MERMAID.parse(
            Path::new("flow.mmd"),
            "graph TD\n  A --> B\n",
            Dialect::CommonMark,
        );
        let [block] = &doc.blocks[..] else {
            panic!("{:?}", doc.blocks);
        };
        let doc::BlockKind::Code(code) = &block.kind else {
            panic!("{block:?}");
        };
        assert!(code.diagram.is_some());
        let er = FileType::MERMAID.parse(
            Path::new("er.mmd"),
            "erDiagram\n  A ||--o{ B : has\n",
            Dialect::CommonMark,
        );
        assert!(matches!(&er.blocks[0].kind, doc::BlockKind::Code(c) if c.diagram.is_none()));
    }

    #[test]
    fn images_by_magic_bytes() {
        assert_eq!(image_ext(b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR"), Some("png"));
        assert_eq!(image_ext(b"\xff\xd8\xff\xe0\0\x10JFIF\0"), Some("jpg"));
        assert_eq!(image_ext(b"\xff\xd8\xff\xe1\0\x18Exif\0\0"), Some("jpg"));
        assert_eq!(image_ext(b"GIF89a\x01\0\x01\0"), Some("gif"));
        assert_eq!(image_ext(b"GIF87a"), Some("gif"));
        // cut before the signature ends, or mangled by a newline conversion
        assert_eq!(image_ext(b"\x89PNG\r\n"), None);
        assert_eq!(image_ext(b"\x89PNG\n\x1a\n"), None);
        assert_eq!(image_ext(b"GIF88a"), None);
        assert_eq!(image_ext(b""), None);
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
        assert_eq!(named("mmd"), Some(FileType::MERMAID));
        assert_eq!(named("Mermaid"), Some(FileType::MERMAID));
        assert_eq!(named("nope"), None);
    }

    #[test]
    fn every_completed_language_is_taken() {
        let names = language_names();
        for want in [
            "md", "txt", "rs", "json", "yaml", "yml", "toml", "diff", "log", "zsh", "mmd",
            "mermaid",
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
                code_labels: true,
                bare: false,
                line_numbers: mode,
                glyphs_gen: 0,
                properties: Default::default(),
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
    fn colored_stdin_keeps_its_colors_but_a_diff() {
        let stdin = Path::new("-");
        // `git lg`: its own colors, its text without the escapes
        let log = "* \x1b[38;5;208mabc1234\x1b[m -\x1b[38;5;214m (HEAD -> main)\x1b[m feat: x\n";
        let ft = FileType::of(stdin, log);
        assert_eq!(ft, FileType::ANSI);
        let doc = ft.parse(stdin, log, Dialect::CommonMark);
        let code = doc.code_blocks()[0];
        assert_eq!(&*code.text, "* abc1234 - (HEAD -> main) feat: x\n");
        assert_eq!(code.highlight_text().as_ref(), log);
        // a colored diff: ada's colors, its escapes gone
        let diff = "\x1b[1mdiff --git a/x.rs b/x.rs\x1b[m\n\x1b[1m--- a/x.rs\x1b[m\n\
                    \x1b[1m+++ b/x.rs\x1b[m\n\x1b[36m@@ -1 +1 @@\x1b[m\n\x1b[31m-a\x1b[m\n\
                    \x1b[32m+b\x1b[m\n";
        let ft = FileType::of(stdin, diff);
        assert_eq!(ft, FileType::Text { lang: Some("Diff") });
        let doc = ft.parse(stdin, diff, Dialect::CommonMark);
        assert!(
            doc.code_blocks()[0]
                .text
                .starts_with("diff --git a/x.rs b/x.rs\n--- a/x.rs")
        );
        assert_eq!(doc.code_blocks()[0].colors, None);
        // `-l` names a type: `ansi` keeps them, another drops them
        assert_eq!(FileType::named("ansi"), Some(FileType::ANSI));
        let named = FileType::named("txt")
            .unwrap()
            .parse(stdin, log, Dialect::CommonMark);
        assert_eq!(
            &*named.code_blocks()[0].text,
            "* abc1234 - (HEAD -> main) feat: x\n"
        );
        // a file is shown as it is
        let file = Path::new("x.log");
        assert_ne!(FileType::of(file, log), FileType::ANSI);
        assert_eq!(FileType::of(file, log).shown_text(file, log), log);
        // PGR-007: `man` with `GROFF_NO_SGR`, bold and underline struck over, no escapes
        let man = "N\x08NA\x08AM\x08ME\x08E\n       _\x08l_\x08s - list\n";
        assert_eq!(FileType::of(stdin, man), FileType::ANSI);
        let doc = FileType::ANSI.parse(stdin, man, Dialect::CommonMark);
        assert_eq!(&*doc.code_blocks()[0].text, "NAME\n       ls - list\n");
    }

    /// X-188: `man`'s page, struck over (`GROFF_NO_SGR`) or in SGR escapes, is a man page shown
    /// without its marks; as plain text too, and `-l man` names it.
    #[test]
    fn man_pages_from_man() {
        let stdin = Path::new("-");
        let title = "LS(1)                 User Commands                 LS(1)\n\n";
        let plain = format!("{title}NAME\n     ls - list\n");
        let struck = format!("{title}N\x08NA\x08AM\x08ME\x08E\n     _\x08l_\x08s - list\n");
        let sgr = format!("{title}\x1b[1mNAME\x1b[0m\n     \x1b[4mls\x1b[24m - list\n");
        for text in [&plain, &struck, &sgr] {
            assert_eq!(FileType::of(stdin, text), FileType::MANPAGE, "{text:?}");
            let doc = FileType::MANPAGE.parse(stdin, text, Dialect::CommonMark);
            assert_eq!(&*doc.code_blocks()[0].text, plain, "{text:?}");
        }
        assert_eq!(FileType::named("man"), Some(FileType::MANPAGE));
        assert_eq!(FileType::of(Path::new("ls.man"), &plain), FileType::MANPAGE);
    }

    #[test]
    fn colored_text_prints_in_its_colors() {
        let stdin = Path::new("-");
        let log = "\x1b[38;5;208mabc\x1b[m x \x1b[1;38;2;1;2;3mbold\x1b[22m rgb\x1b[m\n";
        let doc = FileType::ANSI.parse(stdin, log, Dialect::CommonMark);
        let params = LayoutParams {
            width: 80,
            max_width: None,
            table_mode: Default::default(),
            code_wrap: true,
            code_labels: false,
            bare: true,
            line_numbers: LineNumbers::Off,
            glyphs_gen: 0,
            properties: Default::default(),
            images: None,
        };
        let theme = crate::theme::Theme::builtin()
            .with_bg(false)
            .without_code_bg();
        let out = crate::print::render(
            &doc,
            &theme,
            FileType::ANSI.params(params),
            crate::print::Options::default(),
        );
        assert!(out.contains("38;5;208mabc"), "{out:?}");
        assert!(out.contains("1;38;2;1;2;3mbold"), "{out:?}");
        assert!(out.contains("22m rgb"), "{out:?}");
        assert_eq!(crate::stdin::strip(&out).trim_end(), "abc x bold rgb");
    }

    #[test]
    fn binaries_have_nul_bytes() {
        assert!(is_binary(b"\x89PNG\r\n\x1a\n\0\0"));
        assert!(!is_binary("plain ✓ text".as_bytes()));
    }
}
