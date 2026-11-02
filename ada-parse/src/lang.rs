//! The language table: which languages ada highlights with lexers of its own, by the names a
//! fence, a file name or a first line gives them.
//!
//! Each answer is the one syntect gives on bat's syntax set (two-face's), so that asking the
//! table first never changes what ada picks. The table is a fast path, not the oracle: `None`
//! means "ask syntect", whose set still names every language. ada's tests hold every `Some`
//! against syntect: every name and extension the set knows, every alias, and first lines.

/// A language ada highlights with a lexer of its own instead of a syntect grammar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    /// bat's `JSON` (O-048).
    Json,
    /// bat's `log` (O-043).
    Log,
    /// bat's `Makefile` (G-002).
    Makefile,
    /// bat's `Bourne Again Shell (bash)` (G-003).
    Bash,
    /// bat's `Diff`.
    Diff,
    /// bat's `INI`.
    Ini,
    /// bat's `TOML`.
    Toml,
    /// bat's `XML`.
    Xml,
    /// bat's `YAML`.
    Yaml,
    /// bat's `Dockerfile`.
    Dockerfile,
    /// bat's `Java Properties`.
    Properties,
    /// bat's `DotENV`.
    DotEnv,
    /// bat's `Git Ignore`.
    GitIgnore,
    /// bat's `Git Attributes`.
    GitAttributes,
    /// bat's `Git Mailmap`.
    GitMailmap,
    /// bat's `Git Link`.
    GitLink,
    /// bat's `Rust`.
    Rust,
    /// bat's `Kotlin`.
    Kotlin,
    /// bat's `Java`.
    Java,
    /// bat's `C++`, with the parts of `C` it reaches.
    Cpp,
    /// bat's `C`, on C++'s scanner from C's root.
    C,
    /// bat's `C#`.
    CSharp,
    /// bat's `Go`.
    Go,
    /// bat's `Python`.
    Python,
    /// bat's `Lua`.
    Lua,
    /// bat's `SQL`.
    Sql,
    /// bat's `PowerShell`.
    PowerShell,
    /// bat's `JavaScript (Babel)`, for every JavaScript token (ADR-40's phase 5): Sublime's
    /// `JavaScript` too, which ```` ```javascript ```` names.
    JavaScript,
    /// bat's `TypeScript`.
    TypeScript,
    /// bat's `TypeScriptReact`, TSX: TypeScript's scanner with JSX.
    TypeScriptReact,
    /// bat's `CSS`.
    Css,
    /// bat's `SCSS`: CSS's scanner with SCSS's contexts.
    Scss,
    /// bat's `HTML`, with its CSS and JavaScript on their scanners.
    Html,
    /// bat's `Lisp`, for Common Lisp (X-006).
    Lisp,
    /// bat's `Lisp` for Emacs Lisp (`.el`, ```` ```elisp ````): Lisp's scanner with Emacs's `?a`
    /// characters, which bat's grammar lacks (X-006).
    EmacsLisp,
    /// bat's `Lisp` for Scheme (`.scm`, `.ss`, `.sld`, ```` ```scheme ````): Lisp's scanner with
    /// heads, brackets and `#` forms of Scheme's, a design of ada's own (X-006).
    Scheme,
    /// bat's `Racket`: Scheme's reading of Lisp's scanner (X-006).
    Racket,
    /// bat's `Clojure`, on the Lisp family's second scanner (X-006).
    Clojure,
}

/// Fence languages that no syntax names, and the token each stands for (`zsh` for bash's).
/// `fuzz/markdown.dict` names them all (F-013).
pub const ALIASES: &[(&str, &str)] = &[
    ("zsh", "bash"),
    ("sh", "bash"),
    ("shell", "bash"),
    ("console", "bash"),
    ("shellsession", "bash"),
    ("kt", "kotlin"),
    ("kts", "kotlin"),
    ("ts", "typescript"),
    ("mts", "typescript"),
    ("cts", "typescript"),
    ("jsonl", "json"),
    ("jsonc", "json"),
    ("json5", "json"),
    // Sublime's `JavaScript` by name; ada's JavaScript is Babel's, whatever the token (ADR-40)
    ("javascript", "js"),
    // the Lisps bat's `Lisp` reads, by the names fences give them (X-006)
    ("scheme", "scm"),
    ("elisp", "el"),
    ("emacs-lisp", "el"),
    ("common-lisp", "lisp"),
];

/// The extensions and whole file names each language's syntax claims, lowercase: syntect
/// compares them ignoring ASCII case.
static EXTENSIONS: &[(Lang, &[&str])] = &[
    (
        Lang::Json,
        &[
            "json",
            "sublime-settings",
            "sublime-menu",
            "sublime-keymap",
            "sublime-mousemap",
            "sublime-theme",
            "sublime-build",
            "sublime-project",
            "sublime-completions",
            "sublime-commands",
            "sublime-macro",
            "sublime-color-scheme",
            "ipynb",
            "pipfile.lock",
        ],
    ),
    (Lang::Log, &["log"]),
    (
        Lang::Makefile,
        &[
            "make",
            "gnumakefile",
            "makefile",
            "makefile.am",
            "makefile.in",
            "ocamlmakefile",
            "mak",
            "mk",
        ],
    ),
    (
        Lang::Bash,
        &[
            "sh",
            "bash",
            "zsh",
            "ash",
            ".bash_aliases",
            ".bash_completions",
            ".bash_functions",
            ".bash_login",
            ".bash_logout",
            ".bash_profile",
            ".bash_variables",
            ".bashrc",
            ".profile",
            ".textmate_init",
            ".zlogin",
            ".zlogout",
            ".zprofile",
            ".zshenv",
            ".zshrc",
            "pkgbuild",
            "ebuild",
            "eclass",
        ],
    ),
    (Lang::Diff, &["diff", "patch"]),
    (
        Lang::Ini,
        &[
            "ini",
            "inf",
            "reg",
            "lng",
            "cfg",
            "desktop",
            "url",
            ".editorconfig",
            ".coveragerc",
            ".pylintrc",
            ".gitlint",
            ".hgrc",
            "hgrc",
        ],
    ),
    (
        Lang::Toml,
        &[
            "toml",
            "tml",
            "cargo.lock",
            "gopkg.lock",
            "pipfile",
            "pdm.lock",
            "poetry.lock",
            "uv.lock",
        ],
    ),
    (
        Lang::Xml,
        &[
            "xml", "xsd", "xslt", "tld", "dtml", "rng", "rss", "opml", "svg", "xaml",
        ],
    ),
    (Lang::Yaml, &["yaml", "yml", "sublime-syntax"]),
    (Lang::Dockerfile, &["dockerfile", ".dockerfile"]),
    (Lang::Properties, &["properties"]),
    (
        Lang::DotEnv,
        &[
            ".env",
            ".env.dist",
            ".env.local",
            ".env.sample",
            ".env.example",
            ".env.template",
            ".env.test",
            ".env.test.local",
            ".env.testing",
            ".env.dev",
            ".env.development",
            ".env.development.local",
            ".env.prod",
            ".env.production",
            ".env.production.local",
            ".env.dusk.local",
            ".env.staging",
            ".env.default",
            ".env.defaults",
            ".envrc",
            ".flaskenv",
            "env",
            "env.example",
            "env.sample",
            "env.template",
        ],
    ),
    (Lang::GitIgnore, &["exclude", "gitignore", ".gitignore"]),
    (
        Lang::GitAttributes,
        &["attributes", "gitattributes", ".gitattributes"],
    ),
    (Lang::GitMailmap, &[".mailmap", "mailmap"]),
    (Lang::GitLink, &[".git"]),
    (Lang::Rust, &["rs"]),
    (Lang::Kotlin, &["kt", "kts"]),
    (Lang::Java, &["java", "bsh"]),
    (
        Lang::Cpp,
        &[
            "cpp", "cc", "cp", "cxx", "c++", "hh", "hpp", "hxx", "h++", "inl", "ipp",
        ],
    ),
    (Lang::C, &["c"]),
    (Lang::CSharp, &["cs", "csx"]),
    (Lang::Go, &["go"]),
    (
        Lang::JavaScript,
        &["js", "mjs", "jsx", "babel", "es6", "cjs"],
    ),
    (Lang::TypeScript, &["ts", "mts", "cts"]),
    (Lang::TypeScriptReact, &["tsx"]),
    (
        Lang::Python,
        &[
            "py",
            "py3",
            "pyw",
            "pyi",
            "pyx",
            "pyx.in",
            "pxd",
            "pxd.in",
            "pxi",
            "pxi.in",
            "rpy",
            "cpy",
            "sconstruct",
            "sconscript",
            "gyp",
            "gypi",
            "snakefile",
            "vpy",
            "wscript",
            "bazel",
            "bzl",
        ],
    ),
    (Lang::Lua, &["lua"]),
    (Lang::Sql, &["sql", "ddl", "dml"]),
    (Lang::PowerShell, &["ps1", "psm1", "psd1"]),
    (Lang::Css, &["css", "css.erb", "css.liquid"]),
    (Lang::Scss, &["scss"]),
    (Lang::Html, &["html", "htm", "shtml", "xhtml"]),
    (
        Lang::Lisp,
        &["lisp", "cl", "clisp", "l", "mud", "lsp", "fasl"],
    ),
    (Lang::EmacsLisp, &["el"]),
    (Lang::Scheme, &["scm", "ss", "sld"]),
    (Lang::Racket, &["rkt"]),
    (Lang::Clojure, &["clj", "cljc", "cljs", "edn"]),
];

impl Lang {
    pub const ALL: [Lang; 38] = [
        Lang::Json,
        Lang::Log,
        Lang::Makefile,
        Lang::Bash,
        Lang::Diff,
        Lang::Ini,
        Lang::Toml,
        Lang::Xml,
        Lang::Yaml,
        Lang::Dockerfile,
        Lang::Properties,
        Lang::DotEnv,
        Lang::GitIgnore,
        Lang::GitAttributes,
        Lang::GitMailmap,
        Lang::GitLink,
        Lang::Rust,
        Lang::Kotlin,
        Lang::Java,
        Lang::Cpp,
        Lang::C,
        Lang::CSharp,
        Lang::Go,
        Lang::Python,
        Lang::Lua,
        Lang::JavaScript,
        Lang::Sql,
        Lang::PowerShell,
        Lang::TypeScript,
        Lang::TypeScriptReact,
        Lang::Css,
        Lang::Scss,
        Lang::Html,
        Lang::Lisp,
        Lang::EmacsLisp,
        Lang::Scheme,
        Lang::Racket,
        Lang::Clojure,
    ];

    /// The name of the syntax it stands in for, as syntect names it. Emacs Lisp's and Scheme's
    /// are `Lisp`'s: [`Lang::from_name`] gives `Lisp` for them, a file or a fence names them.
    pub fn name(self) -> &'static str {
        match self {
            Lang::Json => "JSON",
            Lang::Log => "log",
            Lang::Makefile => "Makefile",
            Lang::Bash => "Bourne Again Shell (bash)",
            Lang::Diff => "Diff",
            Lang::Ini => "INI",
            Lang::Toml => "TOML",
            Lang::Xml => "XML",
            Lang::Yaml => "YAML",
            Lang::Dockerfile => "Dockerfile",
            Lang::Properties => "Java Properties",
            Lang::DotEnv => "DotENV",
            Lang::GitIgnore => "Git Ignore",
            Lang::GitAttributes => "Git Attributes",
            Lang::GitMailmap => "Git Mailmap",
            Lang::GitLink => "Git Link",
            Lang::Rust => "Rust",
            Lang::Kotlin => "Kotlin",
            Lang::Java => "Java",
            Lang::Cpp => "C++",
            Lang::C => "C",
            Lang::CSharp => "C#",
            Lang::Go => "Go",
            Lang::Python => "Python",
            Lang::Lua => "Lua",
            Lang::Sql => "SQL",
            Lang::PowerShell => "PowerShell",
            Lang::JavaScript => "JavaScript (Babel)",
            Lang::TypeScript => "TypeScript",
            Lang::TypeScriptReact => "TypeScriptReact",
            Lang::Css => "CSS",
            Lang::Scss => "SCSS",
            Lang::Html => "HTML",
            Lang::Lisp | Lang::EmacsLisp | Lang::Scheme => "Lisp",
            Lang::Racket => "Racket",
            Lang::Clojure => "Clojure",
        }
    }

    /// The scope a whole block is in, as bat's grammar names it.
    pub fn base(self) -> &'static str {
        match self {
            Lang::Json => "source.json",
            Lang::Log => "text.log",
            Lang::Makefile => "source.makefile",
            Lang::Bash => "source.shell.bash",
            Lang::Diff => "source.diff",
            Lang::Ini => "source.ini",
            Lang::Toml => "source.toml",
            Lang::Xml => "text.xml",
            Lang::Yaml => "source.yaml",
            Lang::Dockerfile => "source.dockerfile",
            Lang::Properties => "source.java-props",
            Lang::DotEnv => "source.env",
            Lang::GitIgnore => "text.git.ignore",
            Lang::GitAttributes => "text.git.attributes",
            Lang::GitMailmap => "text.git.mailmap",
            Lang::GitLink => "text.git.link",
            Lang::Rust => "source.rust",
            Lang::Kotlin => "source.Kotlin",
            Lang::Java => "source.java",
            Lang::Cpp => "source.c++",
            Lang::C => "source.c",
            Lang::CSharp => "source.cs",
            Lang::Go => "source.go",
            Lang::Python => "source.python",
            Lang::Lua => "source.lua",
            Lang::Sql => "source.sql",
            Lang::PowerShell => "source.powershell",
            Lang::JavaScript => "source.js",
            Lang::TypeScript => "source.ts",
            Lang::TypeScriptReact => "source.tsx",
            Lang::Css => "source.css",
            Lang::Scss => "source.scss",
            Lang::Html => "text.html.basic",
            Lang::Lisp | Lang::EmacsLisp | Lang::Scheme => "source.lisp",
            Lang::Racket => "source.racket",
            Lang::Clojure => "source.clojure",
        }
    }

    /// The scopes under [`Lang::base`] each kind of a scanned language carries, space-separated,
    /// by [`crate::Span::kind`]: a fixed set. Empty for those on the machine, whose kinds are
    /// the stacks of scopes met so far ([`Lang::scopes`]).
    pub fn kinds(self) -> &'static [&'static str] {
        match self {
            Lang::Diff => crate::diff::KINDS,
            Lang::Ini => crate::ini::KINDS,
            Lang::Json => &crate::json::KINDS,
            Lang::Log => &crate::logs::KINDS,
            _ => &[],
        }
    }

    /// The scopes under [`Lang::base`] a kind of [`crate::lex`]'s spans carries, outermost
    /// first; none for a kind no span has had.
    pub fn scopes(self, kind: u16) -> Vec<&'static str> {
        match self {
            Lang::Toml => crate::toml_scan::scopes(kind),
            Lang::Yaml => crate::yaml_scan::scopes(kind),
            Lang::Xml => crate::xml_scan::scopes(kind),
            Lang::Rust => crate::rust::scopes(kind),
            Lang::Kotlin => crate::kotlin::scopes(kind),
            Lang::Java => crate::java::scopes(kind),
            Lang::Cpp => crate::cpp::scopes(kind),
            Lang::C => crate::cpp::scopes(kind),
            Lang::CSharp => crate::csharp::scopes(kind),
            Lang::Go => crate::go::scopes(kind),
            Lang::Python => crate::python::scopes(kind),
            Lang::Lua => crate::lua::scopes(kind),
            Lang::Sql => crate::sql::scopes(kind),
            Lang::PowerShell => crate::powershell::scopes(kind),
            Lang::JavaScript => crate::javascript::scopes(kind),
            Lang::TypeScript => crate::typescript::scopes(kind, false),
            Lang::TypeScriptReact => crate::typescript::scopes(kind, true),
            Lang::Css | Lang::Scss => crate::css::scopes(kind),
            Lang::Html => crate::html::scopes(kind),
            Lang::Lisp | Lang::EmacsLisp | Lang::Scheme | Lang::Racket => crate::lisp::scopes(kind),
            Lang::Clojure => crate::lisp::clojure::scopes(kind),
            Lang::DotEnv => crate::dotenv_scan::scopes(kind),
            Lang::GitIgnore | Lang::GitAttributes | Lang::GitMailmap | Lang::GitLink => {
                crate::git_scan::scopes(kind)
            }
            Lang::Bash => crate::bash::scopes(kind),
            Lang::Makefile => crate::bash::make_scopes(kind),
            _ if self.grammar().is_some() => {
                self.grammar().map_or_else(Vec::new, |g| g.scopes(kind))
            }
            _ => self
                .kinds()
                .get(usize::from(kind))
                .map_or_else(Vec::new, |k| k.split(' ').collect()),
        }
    }

    /// The context grammar it runs on, for the languages not read line by line.
    pub(crate) fn grammar(self) -> Option<&'static crate::machine::Grammar> {
        Some(match self {
            Lang::Dockerfile => &crate::dockerfile::GRAMMAR,
            Lang::Properties => &crate::properties::GRAMMAR,
            Lang::Json
            | Lang::Log
            | Lang::Makefile
            | Lang::Bash
            | Lang::Diff
            | Lang::Ini
            | Lang::Toml
            | Lang::Xml
            | Lang::Yaml
            | Lang::Rust
            | Lang::Kotlin
            | Lang::Java
            | Lang::Cpp
            | Lang::C
            | Lang::CSharp
            | Lang::Go
            | Lang::Python
            | Lang::Lua
            | Lang::Sql
            | Lang::PowerShell
            | Lang::JavaScript
            | Lang::TypeScript
            | Lang::TypeScriptReact
            | Lang::Css
            | Lang::Scss
            | Lang::DotEnv
            | Lang::GitIgnore
            | Lang::GitAttributes
            | Lang::GitMailmap
            | Lang::GitLink
            | Lang::Html
            | Lang::Lisp
            | Lang::EmacsLisp
            | Lang::Scheme
            | Lang::Racket
            | Lang::Clojure => {
                return None;
            }
        })
    }

    /// Whether [`crate::lex`] lexes it: every language in the table, since Makefiles moved onto
    /// bash's scanner.
    pub fn lexed(self) -> bool {
        true
    }

    /// Whether a line's spans never depend on the lines before it, so that lines can be lexed
    /// in any order, on any thread, from `State::default()`.
    pub fn line_by_line(self) -> bool {
        matches!(self, Lang::Diff | Lang::Ini | Lang::Log)
    }

    /// The language a syntax's own name gives, as `find_syntax_by_name` matches it: exactly.
    pub fn from_name(name: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| l.name() == name)
    }

    /// The language an extension or a whole file name gives, as `find_syntax_by_extension`
    /// matches it: ignoring ASCII case.
    pub fn from_extension(ext: &str) -> Option<Lang> {
        EXTENSIONS
            .iter()
            .find(|(_, exts)| any_eq(exts, ext))
            .map(|&(l, _)| l)
    }

    /// The language a fence's token gives (`json`, `zsh`, `Makefile`), as ada looks one up in
    /// the set: a syntax's own name, then the token through [`ALIASES`] by extension, then by
    /// name ignoring case.
    pub fn from_token(lang: &str) -> Option<Lang> {
        if let Some(l) = Lang::from_name(lang) {
            return Some(l);
        }
        let token = ALIASES
            .iter()
            .find(|(alias, _)| alias.eq_ignore_ascii_case(lang))
            .map_or(lang, |&(_, token)| token);
        Lang::from_extension(token).or_else(|| {
            Lang::ALL
                .into_iter()
                .find(|l| l.name().eq_ignore_ascii_case(token))
        })
    }

    /// The language a file's name gives, as ada reads one: the whole name (`Makefile`,
    /// `.zshrc`), then its extension as a token.
    pub fn from_file_name(name: &str) -> Option<Lang> {
        if let Some(l) = Lang::from_extension(name) {
            return Some(l);
        }
        // as `Path::extension`: after the last dot, but for a name that only starts with one
        let (stem, ext) = name.rsplit_once('.')?;
        if stem.is_empty() {
            return None;
        }
        Lang::from_token(ext)
    }

    /// The language a file's first line declares (`#!/bin/sh`, `# -*- makefile -*-`), when
    /// syntect's first-line match surely gives it; `None` leaves it to syntect.
    ///
    /// syntect tries the set's first-line regexes from its last syntax to its first, so a
    /// line bash's regex matches is still another syntax's when a later one's matches too
    /// (`#!/usr/bin/env fish` names `sh` nowhere, but `#!/home/ls/bin/sh` is LiveScript's).
    /// Each later regex that can match a line starting with `#` needs a word of its own in it
    /// (`fish`, `node`, `ls`), so a line holding none of those words is bash's. The words were
    /// read off bat 0.26.1's grammars; ada's tests list the syntaxes they stand for, and fail
    /// when the set gains another.
    pub fn from_first_line(line: &str) -> Option<Lang> {
        let line = line.strip_prefix('\u{feff}').unwrap_or(line);
        // Unicode case folding and `\s` reach past ASCII: leave such lines to syntect
        if !line.is_ascii() {
            return None;
        }
        let lower = line.to_ascii_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
        if has(AFTER_BASH) {
            return None;
        }
        if is_bash_line(line) {
            return Some(Lang::Bash);
        }
        if !has(AFTER_MAKE) && is_make_line(&lower) {
            return Some(Lang::Makefile);
        }
        None
    }
}

/// A word each first-line regex of a syntax after bash's in the set needs, to match a line
/// starting with `#`: Textile, CMakeCache, CoffeeScript, Crystal, Elixir, Fish, GLSL,
/// JavaScript (Babel), Julia, LiveScript, Requirements.txt, JQ, Svelte, Swift and Odin.
const AFTER_BASH: &[&str] = &[
    "textile",
    "cmakecache",
    "coffee",
    "crystal",
    "elixir",
    "iex",
    "fish",
    "glsl",
    "node",
    "js",
    "julia",
    "ls",
    "pip",
    "requirements",
    "jq",
    "jaq",
    "yq",
    "xq",
    "html",
    "swift",
    "odin",
];

/// The same for the syntaxes between Makefile's and bash's: PHP, Perl, Python, Ruby, and
/// bash itself, whose shell names and `shell-script` all hold `sh`.
const AFTER_MAKE: &[&str] = &["php", "perl", "python", "ruby", "sh"];

/// Whether `word` is one of `words`, ignoring ASCII case. Never inlined: inlined, the compiler
/// unrolls the search over every word of the table into a compare of its own.
#[inline(never)]
fn any_eq(words: &[&str], word: &str) -> bool {
    words.iter().any(|w| w.eq_ignore_ascii_case(word))
}

/// A byte `\b` counts as part of a word. Every non-ASCII byte is one, so that a boundary
/// found here is one for Oniguruma too.
fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// Whether `word` stands alone in `s` somewhere at `from` or after, between `\b`s.
fn has_word(s: &str, from: usize, word: &str) -> bool {
    let b = s.as_bytes();
    let mut at = from;
    while let Some(i) = s.get(at..).and_then(|r| r.find(word)).map(|i| at + i) {
        let end = i + word.len();
        let before = i == 0 || !is_word(b[i - 1]);
        let after = end == b.len() || !is_word(b[end]);
        if before && after {
            return true;
        }
        at = i + 1;
    }
    false
}

/// Whether the text between an editor's `-*-` marks holds `wanted`: `^\#\s*-\*-[^*]*` then
/// `wanted`, then `[^*]*-\*-`. `wanted` says whether a stretch of the marks' text, `*`-free,
/// holds what the regex asks.
fn modeline(line: &str, wanted: impl Fn(&str) -> bool) -> bool {
    let Some(rest) = line.strip_prefix('#') else {
        return false;
    };
    let Some(rest) = rest.trim_start_matches(is_space).strip_prefix("-*-") else {
        return false;
    };
    // the closing `-*-` takes the first `*`: `[^*]*` can't pass one
    let Some(star) = rest.find('*') else {
        return false;
    };
    let closed = star >= 1 && rest[..star].ends_with('-') && rest[star + 1..].starts_with('-');
    closed && wanted(&rest[..star - 1])
}

/// `\s` as far as ASCII goes.
fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r')
}

/// bat's bash first-line regex: `^\#! .* \b(bash|zsh|sh|tcsh|ash|dash|ksh)\b`, or
/// `^\# \s* -\*- [^*]* mode: \s* shell-script [^*]* -\*-`.
fn is_bash_line(line: &str) -> bool {
    const SHELLS: [&str; 7] = ["bash", "zsh", "sh", "tcsh", "ash", "dash", "ksh"];
    if line.starts_with("#!") {
        return SHELLS.iter().any(|w| has_word(line, 2, w));
    }
    modeline(line, |text| {
        text.match_indices("mode:").any(|(i, m)| {
            text[i + m.len()..]
                .trim_start_matches(is_space)
                .contains("shell-script")
        })
    })
}

/// bat's Makefile first-line regex, ignoring case (`lower` is the line in lowercase):
/// `^\#! .* \bmake\b`, or `^\# \s* -\*- [^*]* makefile [^*]* -\*-`.
fn is_make_line(lower: &str) -> bool {
    if lower.starts_with("#!") {
        return has_word(lower, 2, "make");
    }
    modeline(lower, |text| text.contains("makefile"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens() {
        assert_eq!(Lang::from_token("json"), Some(Lang::Json));
        assert_eq!(Lang::from_token("JSONL"), Some(Lang::Json));
        assert_eq!(Lang::from_token("JSON"), Some(Lang::Json));
        assert_eq!(Lang::from_token("zsh"), Some(Lang::Bash));
        assert_eq!(Lang::from_token("console"), Some(Lang::Bash));
        assert_eq!(Lang::from_token("Makefile"), Some(Lang::Makefile));
        assert_eq!(Lang::from_token("mk"), Some(Lang::Makefile));
        assert_eq!(Lang::from_token("log"), Some(Lang::Log));
        assert_eq!(Lang::from_token("rust"), Some(Lang::Rust));
        assert_eq!(Lang::from_token("rs"), Some(Lang::Rust));
        assert_eq!(Lang::from_token("kt"), Some(Lang::Kotlin));
        assert_eq!(Lang::from_token("kotlin"), Some(Lang::Kotlin));
        assert_eq!(Lang::from_token("cs"), Some(Lang::CSharp));
        assert_eq!(Lang::from_token("c#"), Some(Lang::CSharp));
        assert_eq!(Lang::from_token("csharp"), None);
        assert_eq!(Lang::from_token("go"), Some(Lang::Go));
        assert_eq!(Lang::from_token("python"), Some(Lang::Python));
        assert_eq!(Lang::from_token("py"), Some(Lang::Python));
        assert_eq!(Lang::from_token("lua"), Some(Lang::Lua));
        assert_eq!(Lang::from_token("sql"), Some(Lang::Sql));
        assert_eq!(Lang::from_token("powershell"), Some(Lang::PowerShell));
        assert_eq!(Lang::from_token("ps1"), Some(Lang::PowerShell));
        assert_eq!(Lang::from_token("css"), Some(Lang::Css));
        assert_eq!(Lang::from_token("CSS"), Some(Lang::Css));
        assert_eq!(Lang::from_file_name("site.css"), Some(Lang::Css));
        assert_eq!(Lang::from_token("scss"), Some(Lang::Scss));
        assert_eq!(Lang::from_token("SCSS"), Some(Lang::Scss));
        assert_eq!(Lang::from_file_name("_variables.scss"), Some(Lang::Scss));
        assert_eq!(Lang::from_token("html"), Some(Lang::Html));
        assert_eq!(Lang::from_token("HTML"), Some(Lang::Html));
        assert_eq!(Lang::from_file_name("index.html"), Some(Lang::Html));
        assert_eq!(Lang::from_token("java"), Some(Lang::Java));
    }

    #[test]
    fn file_names() {
        assert_eq!(Lang::from_file_name("Makefile"), Some(Lang::Makefile));
        assert_eq!(Lang::from_file_name("x.mk"), Some(Lang::Makefile));
        assert_eq!(Lang::from_file_name(".zshrc"), Some(Lang::Bash));
        assert_eq!(Lang::from_file_name("run.SH"), Some(Lang::Bash));
        assert_eq!(Lang::from_file_name("a.jsonl"), Some(Lang::Json));
        assert_eq!(Lang::from_file_name("Pipfile.lock"), Some(Lang::Json));
        assert_eq!(Lang::from_file_name("x.log"), Some(Lang::Log));
        assert_eq!(Lang::from_file_name(".json"), None);
        assert_eq!(Lang::from_file_name("Cargo.lock"), Some(Lang::Toml));
        assert_eq!(Lang::from_file_name("Cargo.toml"), Some(Lang::Toml));
    }

    #[test]
    fn first_lines() {
        let bash = [
            "#!/bin/sh",
            "#!/usr/bin/env bash",
            "#!/bin/bash -eu",
            "\u{feff}#!/bin/zsh",
            "#!bash",
            "# -*- mode: shell-script -*-",
            "#  -*- mode:shell-script; tab-width: 4 -*-",
            // bash's regex is tried before Makefile's
            "#!/usr/bin/make -f # sh",
        ];
        for line in bash {
            assert_eq!(Lang::from_first_line(line), Some(Lang::Bash), "{line}");
        }
        let make = [
            "#!/usr/bin/make -f",
            "#!/usr/bin/env -S MAKE -f",
            "# -*- Makefile -*-",
        ];
        for line in make {
            assert_eq!(Lang::from_first_line(line), Some(Lang::Makefile), "{line}");
        }
        let neither = [
            "#!/usr/bin/env fish",
            "#!/home/ls/bin/sh",
            "#!/usr/bin/env node",
            "#!/bin/shell",
            "#!/usr/bin/python3",
            "# -*- mode: shell-script",
            "# -*- mode: shell-* -*-",
            "#!/bin/sh é",
            "echo hi",
            "",
        ];
        for line in neither {
            assert_eq!(Lang::from_first_line(line), None, "{line}");
        }
    }
}
