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
    /// bat's `fstab`: fstab, crypttab and mtab.
    Fstab,
    /// bat's `Known Hosts`: OpenSSH's `known_hosts`.
    KnownHosts,
    /// bat's `SSH Config`: OpenSSH's client config.
    SshConfig,
    /// bat's `SSHD Config`: OpenSSH's server config.
    SshdConfig,
    /// bat's `Crontab`: cron's tables.
    Crontab,
    /// bat's `nginx`: nginx's configs.
    Nginx,
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
    /// bat's `Objective-C`: C's scanner with Objective-C's words and tokens (X-134). A header
    /// (`.h`, which bat's set names Objective-C) is told by its text ([`Lang::for_header`]).
    ObjC,
    /// bat's `Objective-C++`: C++'s scanner with Objective-C's words and tokens (X-134).
    ObjCpp,
    /// bat's `C#`.
    CSharp,
    /// bat's `Go`.
    Go,
    /// bat's `Python`.
    Python,
    /// bat's `Lua`.
    Lua,
    /// bat's `PHP`: HTML with PHP's `<?php … ?>` islands over `PHP Source`.
    Php,
    /// bat's `Swift`.
    Swift,
    /// bat's `CMake`.
    CMake,
    /// bat's `CMakeCache`: `CMakeCache.txt`.
    CMakeCache,
    /// bat's `Terraform`: Terraform's files and HCL's (`.hcl`).
    Terraform,
    /// bat's `Scala`: Scala's files, sbt's and scala-cli's scripts.
    Scala,
    /// bat's `Haskell`.
    Haskell,
    /// bat's `Literate Haskell`: prose around Haskell's code (Bird tracks, `\begin{code}`,
    /// markdown-unlit's fences).
    LiterateHaskell,
    /// bat's `Ruby`.
    Ruby,
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
    // GitHub's names for C# and PowerShell, which bat's set doesn't know
    ("csharp", "cs"),
    ("pwsh", "powershell"),
    ("posh", "powershell"),
    // GitHub's names for Objective-C and Objective-C++ (X-134)
    ("objc", "m"),
    ("obj-c", "m"),
    ("objectivec", "m"),
    ("objc++", "mm"),
    ("obj-c++", "mm"),
    ("objectivec++", "mm"),
];

/// Whole file names a syntax of bat's set that none of these is claims, where their extension
/// is one of these languages' (`resolv.conf`, nginx's `conf`): syntect finds a file's whole name
/// before its extension.
static OTHERS: &[&str] = &["resolv.conf"];

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
    (Lang::Fstab, &["fstab", "crypttab", "mtab"]),
    (Lang::KnownHosts, &["known_hosts", "known_hosts.old"]),
    (Lang::SshConfig, &["ssh_config"]),
    (Lang::SshdConfig, &["sshd_config"]),
    (Lang::Crontab, &["tab", "crontab", "cron.d"]),
    (
        Lang::Nginx,
        &[
            "conf.erb",
            "conf",
            "nginx.conf",
            "mime.types",
            "fastcgi_params",
            "scgi_params",
            "uwsgi_params",
        ],
    ),
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
    (Lang::ObjC, &["m", "h"]),
    (Lang::ObjCpp, &["mm"]),
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
    (
        Lang::Php,
        &[
            "php", "php3", "php4", "php5", "php7", "phps", "phpt", "phtml",
        ],
    ),
    (Lang::Swift, &["swift"]),
    (Lang::CMake, &["cmakelists.txt", "cmake"]),
    (Lang::CMakeCache, &["cmakecache.txt"]),
    (Lang::Terraform, &["tf", "tfvars", "hcl"]),
    (Lang::Scala, &["scala", "sbt", "sc"]),
    (Lang::Haskell, &["hs"]),
    (Lang::LiterateHaskell, &["lhs"]),
    (
        Lang::Ruby,
        &[
            "rb",
            "Appfile",
            "Appraisals",
            "Berksfile",
            "Brewfile",
            "capfile",
            "cgi",
            "Cheffile",
            "config.ru",
            "Deliverfile",
            "Fastfile",
            "fcgi",
            "Gemfile",
            "gemspec",
            "Guardfile",
            "irbrc",
            "jbuilder",
            "Podfile",
            "podspec",
            "prawn",
            "rabl",
            "rake",
            "Rakefile",
            "Rantfile",
            "rbx",
            "rjs",
            "ruby.rail",
            "Scanfile",
            "simplecov",
            "Snapfile",
            "thor",
            "Thorfile",
            "Vagrantfile",
        ],
    ),
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
    pub const ALL: [Lang; 55] = [
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
        Lang::Fstab,
        Lang::KnownHosts,
        Lang::SshConfig,
        Lang::SshdConfig,
        Lang::Crontab,
        Lang::Nginx,
        Lang::Rust,
        Lang::Kotlin,
        Lang::Java,
        Lang::Cpp,
        Lang::C,
        Lang::ObjC,
        Lang::ObjCpp,
        Lang::CSharp,
        Lang::Go,
        Lang::Python,
        Lang::Lua,
        Lang::Php,
        Lang::Swift,
        Lang::CMake,
        Lang::CMakeCache,
        Lang::Terraform,
        Lang::Scala,
        Lang::Haskell,
        Lang::LiterateHaskell,
        Lang::Ruby,
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
            Lang::Fstab => "fstab",
            Lang::KnownHosts => "Known Hosts",
            Lang::SshConfig => "SSH Config",
            Lang::SshdConfig => "SSHD Config",
            Lang::Crontab => "Crontab",
            Lang::Nginx => "nginx",
            Lang::Rust => "Rust",
            Lang::Kotlin => "Kotlin",
            Lang::Java => "Java",
            Lang::Cpp => "C++",
            Lang::C => "C",
            Lang::ObjC => "Objective-C",
            Lang::ObjCpp => "Objective-C++",
            Lang::CSharp => "C#",
            Lang::Go => "Go",
            Lang::Python => "Python",
            Lang::Lua => "Lua",
            Lang::Php => "PHP",
            Lang::Swift => "Swift",
            Lang::CMake => "CMake",
            Lang::CMakeCache => "CMakeCache",
            Lang::Terraform => "Terraform",
            Lang::Scala => "Scala",
            Lang::Haskell => "Haskell",
            Lang::LiterateHaskell => "Literate Haskell",
            Lang::Ruby => "Ruby",
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

    /// The name ada gives a whole file in it, and `-l`: [`Lang::name`], but Emacs Lisp's and
    /// Scheme's own, which lead back to them ([`Lang::from_token`]) where `Lisp` would give
    /// Common Lisp's reading.
    pub fn label(self) -> &'static str {
        match self {
            Lang::EmacsLisp => "Emacs Lisp",
            Lang::Scheme => "Scheme",
            _ => self.name(),
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
            Lang::Fstab => "source.fstab",
            Lang::KnownHosts => "text.known_hosts",
            Lang::SshConfig => "source.ssh_config",
            Lang::SshdConfig => "source.sshd_config",
            Lang::Crontab => "source.crontab",
            Lang::Nginx => "source.nginx",
            Lang::Rust => "source.rust",
            Lang::Kotlin => "source.Kotlin",
            Lang::Java => "source.java",
            Lang::Cpp => "source.c++",
            Lang::C => "source.c",
            Lang::ObjC => "source.objc",
            Lang::ObjCpp => "source.objc++",
            Lang::CSharp => "source.cs",
            Lang::Go => "source.go",
            Lang::Python => "source.python",
            Lang::Lua => "source.lua",
            Lang::Php => "embedding.php",
            Lang::Swift => "source.swift",
            Lang::CMake => "source.cmake",
            Lang::CMakeCache => "source.cmakecache",
            Lang::Terraform => "source.terraform",
            Lang::Scala => "source.scala",
            Lang::Haskell => "source.haskell",
            Lang::LiterateHaskell => "text.tex.latex.haskell",
            Lang::Ruby => "source.ruby",
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

    /// The scopes under [`Lang::base`] each kind of a line lexer's language carries,
    /// space-separated, by [`crate::Span::kind`]; empty for the scanners, whose kinds
    /// [`Lang::scopes`] gives.
    pub fn kinds(self) -> &'static [&'static str] {
        match self {
            Lang::Diff => crate::diff::KINDS,
            Lang::Ini => crate::ini::KINDS,
            Lang::Fstab => crate::fstab::KINDS,
            Lang::CMakeCache => crate::cmake_cache::KINDS,
            Lang::KnownHosts => crate::known_hosts::KINDS,
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
            Lang::Cpp => crate::cc::scopes(kind),
            Lang::C => crate::cc::scopes_c(kind),
            Lang::ObjC => crate::cc::scopes_objc(kind),
            Lang::ObjCpp => crate::cc::scopes_objcpp(kind),
            Lang::CSharp => crate::cs::scopes(kind),
            Lang::Go => crate::go::scopes(kind),
            Lang::Python => crate::python::scopes(kind),
            Lang::Lua => crate::lua::scopes(kind),
            Lang::Php => crate::php::scopes(kind),
            Lang::Swift => crate::swift::scopes(kind),
            Lang::CMake => crate::cmake::scopes(kind),
            Lang::Terraform => crate::terraform::scopes(kind),
            Lang::Scala => crate::scala::scopes(kind),
            Lang::Haskell | Lang::LiterateHaskell => crate::haskell::scopes(kind),
            Lang::Ruby => crate::ruby::scopes(kind),
            Lang::Sql => crate::sql::scopes(kind),
            Lang::PowerShell => crate::powershell::scopes(kind),
            Lang::JavaScript => crate::javascript::scopes(kind),
            Lang::TypeScript => crate::typescript::scopes(kind, false),
            Lang::TypeScriptReact => crate::typescript::scopes(kind, true),
            Lang::Css | Lang::Scss => crate::css::scopes(kind),
            Lang::Html => crate::html::scopes(kind),
            Lang::Lisp | Lang::EmacsLisp | Lang::Scheme | Lang::Racket => crate::lisp::scopes(kind),
            Lang::Clojure => crate::lisp::clojure::scopes(kind),
            Lang::DotEnv => crate::dotenv::scopes(kind),
            Lang::GitIgnore | Lang::GitAttributes | Lang::GitMailmap | Lang::GitLink => {
                crate::git::scopes(kind)
            }
            Lang::Bash => crate::bash::scopes(kind),
            Lang::Makefile => crate::bash::make_scopes(kind),
            Lang::Dockerfile => crate::dockerfile::scopes(kind),
            Lang::Properties => crate::properties::scopes(kind),
            Lang::SshConfig => crate::ssh_config::scopes(kind, false),
            Lang::SshdConfig => crate::ssh_config::scopes(kind, true),
            Lang::Crontab => crate::crontab::scopes(kind),
            Lang::Nginx => crate::nginx::scopes(kind),
            _ => self
                .kinds()
                .get(usize::from(kind))
                .map_or_else(Vec::new, |k| k.split(' ').collect()),
        }
    }

    /// Whether [`crate::lex`] lexes it: every language in the table, since Makefiles moved onto
    /// bash's scanner.
    pub fn lexed(self) -> bool {
        true
    }

    /// Whether a line's spans never depend on the lines before it, so that lines can be lexed
    /// in any order, on any thread, from `State::default()`.
    pub fn line_by_line(self) -> bool {
        matches!(
            self,
            Lang::Diff
                | Lang::Ini
                | Lang::Log
                | Lang::Fstab
                | Lang::SshConfig
                | Lang::SshdConfig
                | Lang::Crontab
        )
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
            Lang::ALL.into_iter().find(|l| {
                l.name().eq_ignore_ascii_case(token) || l.label().eq_ignore_ascii_case(token)
            })
        })
    }

    /// The language a file's name gives, as ada reads one: the whole name (`Makefile`,
    /// `.zshrc`), then its extension as a token.
    pub fn from_file_name(name: &str) -> Option<Lang> {
        if let Some(l) = Lang::from_extension(name) {
            return Some(l);
        }
        if any_eq(OTHERS, name) {
            return None;
        }
        // as `Path::extension`: after the last dot, but for a name that only starts with one
        let (stem, ext) = name.rsplit_once('.')?;
        if stem.is_empty() {
            return None;
        }
        Lang::from_token(ext)
    }

    /// The language of a header (`.h`) whose text starts with `text`, one of C, C++, Objective-C
    /// and Objective-C++, by the words in its first 64 KB (X-134): bat's set names every `.h`
    /// Objective-C, its last claimant.
    pub fn for_header(text: &[u8]) -> Lang {
        crate::header::language(text)
    }

    /// Whether a file's name is a header's that [`Lang::for_header`] decides: `.h`.
    pub fn is_header(name: &str) -> bool {
        name.rsplit_once('.')
            .is_some_and(|(stem, ext)| !stem.is_empty() && ext.eq_ignore_ascii_case("h"))
    }

    /// The language a file's first line declares (`#!/bin/sh`, `# -*- makefile -*-`), when
    /// syntect's first-line match on bat's set surely gives one of these; `None` when it gives
    /// another syntax, none, or only the set can tell ([`Lang::first_line`]).
    pub fn from_first_line(line: &str) -> Option<Lang> {
        match Lang::first_line(line) {
            FirstLine::Lang(lang) => Some(lang),
            FirstLine::None | FirstLine::Syntax(_) | FirstLine::Ask => None,
        }
    }

    /// What syntect's first-line match on bat's set makes of a file's first line, as far as
    /// that can be told without the set: one of these languages, no syntax at all, or a
    /// question for the set.
    ///
    /// syntect tries the set's first-line regexes from its last syntax to its first, and the
    /// first that matches names the syntax. A table holds each regex in the set's
    /// order as a test that holds on every line the regex can match, and, for some, the regex
    /// itself; the walk goes the same way, past each regex the line surely doesn't match, and
    /// stops at the first it may: one of these languages when that regex is one of theirs and
    /// matches, the set when only the set can tell.
    pub fn first_line(line: &str) -> FirstLine {
        Lang::walk_first_lines(line, &mut |_, _| None, false)
    }

    /// [`Lang::first_line`], with `ask` answering for each regex the table can't decide on
    /// `line`: given the regex's place in the set's order ([`first_line_syntaxes`]), whether it
    /// matches, or `None` when it can't tell either. A regex `ask` can't tell about is decided
    /// by the table's test for ASCII lines where there is one, though the line isn't ASCII;
    /// `Ask` comes back only when there is none. ada answers with the set's regexes, as the
    /// `regex` crate reads them (ADR-43).
    pub fn first_line_asking(
        line: &str,
        ask: &mut dyn FnMut(usize, &str) -> Option<bool>,
    ) -> FirstLine {
        Lang::walk_first_lines(line, ask, true)
    }

    fn walk_first_lines(
        line: &str,
        ask: &mut dyn FnMut(usize, &str) -> Option<bool>,
        lenient: bool,
    ) -> FirstLine {
        let line = line.strip_prefix('\u{feff}').unwrap_or(line);
        // `^` also holds after a line feed
        if line.contains('\n') {
            return FirstLine::Ask;
        }
        let text = FirstLineText::new(line);
        for (i, regex) in FIRST_LINES.iter().enumerate().rev() {
            if !(regex.may)(&text) {
                continue;
            }
            let exact = || regex.exact.and_then(|exact| exact(line));
            // `\b`, `\s` and case folding reach past ASCII: such lines are asked about
            let decided = if text.ascii { exact() } else { None };
            let answer = decided
                .or_else(|| ask(i, line))
                .or_else(|| exact().filter(|_| lenient));
            match answer {
                Some(true) => {
                    return match regex.lang {
                        Some(lang) => FirstLine::Lang(lang),
                        None if lenient => FirstLine::Syntax(regex.syntax),
                        None => FirstLine::Ask,
                    };
                }
                Some(false) => {}
                None => return FirstLine::Ask,
            }
        }
        FirstLine::None
    }
}

/// What a file's first line says, by [`Lang::first_line`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirstLine {
    /// The set's first-line match gives this language.
    Lang(Lang),
    /// No first-line regex of the set matches the line.
    None,
    /// The set's first-line match gives this syntax, one no lexer here covers, by its name: only
    /// from [`Lang::first_line_asking`].
    Syntax(&'static str),
    /// Only the set can tell: a syntax not covered here, or a regex the table can't decide.
    Ask,
}

/// The syntaxes of bat's set that have a first-line regex, in the set's order: what
/// [`Lang::first_line`] walks. ada's tests hold it to the set's names and order, and a
/// snapshot of the regexes fails when two-face changes one.
#[doc(hidden)]
pub fn first_line_syntaxes() -> impl Iterator<Item = &'static str> {
    FIRST_LINES.iter().map(|r| r.syntax)
}

/// One first-line regex of the set: its syntax, a test holding on every line the regex can
/// match (it may hold on more), and, where the table decides it, the regex on an ASCII line
/// (`None` when it can't tell) and the language it names.
struct FirstLineRegex {
    syntax: &'static str,
    may: fn(&FirstLineText) -> bool,
    exact: Option<fn(&str) -> Option<bool>>,
    lang: Option<Lang>,
}

const fn may(syntax: &'static str, may: fn(&FirstLineText) -> bool) -> FirstLineRegex {
    FirstLineRegex {
        syntax,
        may,
        exact: None,
        lang: None,
    }
}

const fn decided(
    syntax: &'static str,
    lang: Lang,
    may: fn(&FirstLineText) -> bool,
    exact: fn(&str) -> Option<bool>,
) -> FirstLineRegex {
    FirstLineRegex {
        syntax,
        may,
        exact: Some(exact),
        lang: Some(lang),
    }
}

/// bat 0.26.1's first-line regexes, read off its grammars: each `may` asks for what the
/// regex can't match without (its anchor, its words), ignoring ASCII case where the regex
/// does. Shebang regexes name their interpreter, so a line naming none of a later regex's
/// words is past it.
const FIRST_LINES: &[FirstLineRegex] = &[
    // ^#!.*(osascript)
    may("AppleScript", |t| t.shebang() && t.has("osascript")),
    // -\*- C\+\+ -\*-
    may("C++", |t| t.has("-*- C++ -*-")),
    // -[*]-( Mode:)? C -[*]-
    may("C", |t| t.has(" C -*-")),
    // ^#!.*\bg?dmd\b.
    may("D", |t| t.shebang() && t.has("dmd")),
    // ^(=== modified file|====\s*//.+\s-\s.+\s+====|Index: |--- [^%]|\*\*\*.*\d{4}\s*$
    //   |\d+(,\d+)* (a|d|c) \d+(,\d+)* $|diff --git )
    may("Diff", |t| {
        ["===", "Index: ", "--- ", "***", "diff --git "]
            .iter()
            .any(|p| t.line.starts_with(p))
            || t.line.starts_with(char::is_numeric)
    }),
    // (?x: ^\#! .* \b(erlang|escript)\b | ^ \s* \%+ \s* -\*- .*? \b[Ee]rlang\b .*? -\*-)
    may("Erlang", |t| {
        t.shebang() && t.has_ci("escript") || t.has_ci("erlang")
    }),
    // ^\[core\]
    may("Git Config", |t| t.line.starts_with("[core]")),
    // ^\s*gitdir\s*:
    may("Git Link", |t| t.trimmed().starts_with("gitdir")),
    // ^commit\s+\h{7,}
    may("Git Log", |t| t.line.starts_with("commit")),
    // ^(?:drop|edit|exec|fixup|pick|reword|squash|[defprsx]) \h{7,}
    may("Git Rebase Todo", |t| {
        t.line.starts_with(['d', 'e', 'f', 'p', 'r', 's', 'x'])
            && t.line.split_once(' ').is_some_and(|(_, rest)| {
                rest.bytes().take_while(u8::is_ascii_hexdigit).count() >= 7
            })
    }),
    // (?i)<(!DOCTYPE\s*)?html
    may("HTML", html),
    // ^#!\s*/.*\b(node|bun|js)\b
    may("JavaScript", |t| {
        t.shebang() && (t.has("node") || t.has("bun") || t.has("js"))
    }),
    // This is (pdf|pdfe)?TeXk?, Version
    may("LaTeX Log", |t| t.has("TeX") && t.has(", Version ")),
    // (?xi: ^ \s* ; .*? -\*- .*? \blisp\b .*? -\*-)
    may("Lisp", |t| t.has("-*-") && t.has_ci("lisp")),
    // (?xi: ^\#! .* \bmake\b | ^\# \s* -\*- [^*]* makefile [^*]* -\*-)
    decided(
        "Makefile",
        Lang::Makefile,
        |t| t.has_ci("make"),
        |line| Some(is_make_line(&line.to_ascii_lowercase())),
    ),
    // (?i)^format:\s*complete\s*$
    may("MultiMarkdown", |t| t.lower.starts_with("format:")),
    // ^(#!.*[^-]php[0-9]?|<\?php)\b
    decided(
        "PHP",
        Lang::Php,
        |t| t.shebang() && t.has("php") || t.line.starts_with("<?php"),
        |line| Some(is_php_line(line)),
    ),
    // (?xi: ^\#! .* \bperl\b | ^\# \s* -\*- [^*]* perl [^*]* -\*-)
    may("Perl", |t| t.has_ci("perl")),
    // ^#!\s*/.*\bpython(\d(\.\d)?)?\b
    decided(
        "Python",
        Lang::Python,
        |t| t.shebang() && t.has("python"),
        |line| Some(is_python_line(line)),
    ),
    // (?xi: ^\#! .* \bj?ruby\b | ^\# \s* -\*- [^*]* ruby [^*]* -\*-)
    decided(
        "Ruby",
        Lang::Ruby,
        |t| t.has_ci("ruby"),
        |line| Some(is_ruby_line(&line.to_ascii_lowercase())),
    ),
    // (?x) ^\#! .* \b(bash|zsh|sh|tcsh|ash|dash|ksh)\b
    //    | ^\# \s* -\*- [^*]* mode: \s* shell-script [^*]* -\*-
    decided(
        "Bourne Again Shell (bash)",
        Lang::Bash,
        |t| t.shebang() && t.has("sh") || t.has("shell-script"),
        |line| Some(is_bash_line(line)),
    ),
    // textile
    may("Textile", |t| t.has("textile")),
    // (?x: ^(?: <\?xml\s
    //   | \s*<([\w-]+):Envelope\s+xmlns:\1\s*=\s*"http://schemas.xmlsoap.org/soap/envelope/"\s*>
    //   | \s*(?i:<!DOCTYPE\s+(?!html[ \t\n\f>]))))
    decided(
        "XML",
        Lang::Xml,
        |t| {
            t.line.starts_with("<?xml")
                || t.has(":Envelope")
                || t.lower
                    .trim_start_matches(is_any_space)
                    .starts_with("<!doctype")
        },
        |line| Some(is_xml_line(line)),
    ),
    // ^%YAML( ?1.\d+)?
    may("YAML", |t| t.line.starts_with("%YAML")),
    // # This is the CMakeCache file.
    decided(
        "CMakeCache",
        Lang::CMakeCache,
        |t| t.has("# This is the CMakeCache file"),
        |line| {
            Some(
                line.match_indices("# This is the CMakeCache file")
                    .any(|(i, m)| line.len() > i + m.len()),
            )
        },
    ),
    // ^#!.*\bcoffee
    may("CoffeeScript", |t| t.shebang() && t.has("coffee")),
    // ^#!/.*\bcrystal
    may("Crystal", |t| t.shebang() && t.has("crystal")),
    // ^\s*(?i:(from(?!\s+\S+\s+import)|arg))\s+
    decided(
        "Dockerfile",
        Lang::Dockerfile,
        |t| {
            let lower = t.lower.trim_start_matches(is_any_space);
            ["from", "arg"].iter().any(|w| {
                lower
                    .strip_prefix(w)
                    .is_some_and(|rest| rest.starts_with(is_any_space))
            })
        },
        |line| Some(is_dockerfile_line(line)),
    ),
    // ^#!/.*\b(?:elixirc?|iex)
    may("Elixir", |t| {
        t.shebang() && (t.has("elixir") || t.has("iex"))
    }),
    // ^#!.*\b(fish)\b
    may("Fish", |t| t.shebang() && t.has("fish")),
    // -[*]-( Mode:)? GLSL -[*]-
    may("GLSL", |t| t.has(" GLSL -*-")),
    // ^#!\s*/.*\b(node|js)$\n?
    may("JavaScript (Babel)", |t| {
        t.shebang() && (t.has("node") || t.has("js"))
    }),
    // ^{% extends ["'][^"']+["'] %}
    may("HTML (Jinja2)", |t| t.line.starts_with("{% extends")),
    // ^#!.*\bjulia\s*$
    may("Julia", |t| t.shebang() && t.has("julia")),
    // ^#!.*\bls
    may("LiveScript", |t| t.shebang() && t.has("ls")),
    // ^(syntax)\s*(=)\s*("proto\d")\s*(;)\s*$
    may("Protocol Buffer", |t| t.line.starts_with("syntax")),
    // (?xi: ^ \#! .* \bpip | ^ \s* \# .*? -\*- .*? \bpip-requirements\b .*? -\*-
    //   | ^ \s* \# (vim?|ex): .*? \brequirements\b)
    may("Requirements.txt", |t| {
        t.shebang() && t.has_ci("pip")
            || t.has_ci("requirements")
                && (t.has("-*-") || ["#vi:", "#vim:", "#ex:"].iter().any(|m| t.has_ci(m)))
    }),
    // (?xi: ^ \#! .* \b(jq|gojq|jaq|yq|xq)\b)
    may("JQ", |t| {
        t.shebang() && ["jq", "jaq", "yq", "xq"].iter().any(|w| t.has_ci(w))
    }),
    // (?i)<(!DOCTYPE\s*)?html
    may("Svelte", html),
    // ^#!/.*\bswift
    may("Swift", |t| t.shebang() && t.has("swift")),
    // (?x: ^ (?: (?:GET|HEAD|POST|PUT|DELETE|CONNECT|OPTIONS|TRACE|PATCH)\ .+\ HTTP/\d(?:\.\d)?$
    //   | (?:HTTP/\d(?:\.\d)?\ \d{3}\ )))
    may("HTTP Request and Response", |t| {
        [
            "GET ", "HEAD ", "POST ", "PUT ", "DELETE ", "CONNECT ", "OPTIONS ", "TRACE ",
            "PATCH ", "HTTP/",
        ]
        .iter()
        .any(|p| t.line.starts_with(p))
    }),
    // ^-----BEGIN [\w ]+ PRIVATE KEY-----
    may("Private Key", |t| t.line.starts_with("-----BEGIN ")),
    // -[*]-( Mode:)? Odin -[*]-
    may("Odin", |t| t.has(" Odin -*-")),
];

/// A first line as [`FIRST_LINES`]' tests read it: as it is, and folded to lowercase for the
/// regexes that ignore case.
struct FirstLineText<'a> {
    line: &'a str,
    /// The line in ASCII lowercase, with each character that a regex ignoring case can match
    /// to ASCII letters written as those letters (`ſ` as `s`, `ﬁ` as `fi`).
    lower: String,
    ascii: bool,
}

impl<'a> FirstLineText<'a> {
    fn new(line: &'a str) -> FirstLineText<'a> {
        let ascii = line.is_ascii();
        let lower = if ascii {
            line.to_ascii_lowercase()
        } else {
            fold(line)
        };
        FirstLineText { line, lower, ascii }
    }

    fn shebang(&self) -> bool {
        self.line.starts_with("#!")
    }

    fn has(&self, s: &str) -> bool {
        self.line.contains(s)
    }

    /// Whether the line holds `s`, ASCII lowercase, ignoring case.
    fn has_ci(&self, s: &str) -> bool {
        self.lower.contains(s)
    }

    /// The line past `^\s*`.
    fn trimmed(&self) -> &str {
        self.line.trim_start_matches(is_any_space)
    }
}

/// HTML's and Svelte's first-line regex, `(?i)<(!DOCTYPE\s*)?html`.
fn html(t: &FirstLineText) -> bool {
    t.has_ci("<html") || t.has_ci("<!doctype")
}

/// `line` in ASCII lowercase, each character Unicode case folding takes to ASCII letters
/// written as them: `ſ` and the Kelvin sign, `İ`'s `i`, `ß` and the Latin ligatures. Any other
/// character stays as it is, and so can't make an ASCII word appear.
fn fold(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for c in line.chars() {
        match c {
            '\u{17f}' => out.push('s'),
            '\u{212a}' => out.push('k'),
            '\u{130}' => out.push('i'),
            '\u{df}' | '\u{1e9e}' => out.push_str("ss"),
            '\u{fb00}' => out.push_str("ff"),
            '\u{fb01}' => out.push_str("fi"),
            '\u{fb02}' => out.push_str("fl"),
            '\u{fb03}' => out.push_str("ffi"),
            '\u{fb04}' => out.push_str("ffl"),
            '\u{fb05}' | '\u{fb06}' => out.push_str("st"),
            c => out.push(c.to_ascii_lowercase()),
        }
    }
    out
}

/// At least every character Oniguruma's and fancy-regex's Unicode `\s` take, for a test
/// skipping a regex's leading `\s*`: skipping more only makes the test hold more often.
fn is_any_space(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\u{180e}' | '\u{200b}' | '\u{feff}')
}

/// bat's Python first-line regex, `^#!\s*/.*\bpython(\d(\.\d)?)?\b`, on an ASCII line.
fn is_python_line(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("#!") else {
        return false;
    };
    let Some(rest) = rest.trim_start_matches(is_space).strip_prefix('/') else {
        return false;
    };
    let b = rest.as_bytes();
    rest.match_indices("python").any(|(i, word)| {
        // the `/` before the first byte is a boundary
        let starts = i == 0 || !is_word(b[i - 1]);
        // `\b` after `python`, after one more digit, or after `\d\.\d`, whose `.` is one
        let ends = match &b[i + word.len()..] {
            [] => true,
            [c, ..] if !is_word(*c) => true,
            [d, rest @ ..] if d.is_ascii_digit() => rest.first().is_none_or(|&c| !is_word(c)),
            _ => false,
        };
        starts && ends
    })
}

/// `^(#!.*[^-]php[0-9]?|<\?php)\b` on an ASCII line (X-099).
fn is_php_line(line: &str) -> bool {
    let b = line.as_bytes();
    let ends = |i: usize| b.get(i).is_none_or(|&c| !is_word(c));
    if let Some(rest) = line.strip_prefix("<?php") {
        return ends(line.len() - rest.len());
    }
    if !line.starts_with("#!") {
        return false;
    }
    line.match_indices("php").any(|(i, _)| {
        // `.*[^-]`: a byte that isn't `-` between `#!` and `php`
        let after = i + 3;
        i >= 3 && b[i - 1] != b'-' && (ends(after) || b[after].is_ascii_digit() && ends(after + 1))
    })
}

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

/// bat's XML first-line regex: `^<\?xml\s`, a SOAP envelope
/// (`^\s*<([\w-]+):Envelope\s+xmlns:\1\s*=\s*"http://schemas.xmlsoap.org/soap/envelope/"\s*>`), or
/// a doctype other than HTML's (`^\s*(?i:<!DOCTYPE\s+(?!html[ \t\n\f>]))`). `\s` is Unicode's
/// blank, as Oniguruma's: the `regex` crate can't read this regex, so ada asks this test about
/// a line that isn't ASCII too (`Lang::first_line_asking`); a name's `\w` stays ASCII's.
fn is_xml_line(line: &str) -> bool {
    if line
        .strip_prefix("<?xml")
        .is_some_and(|rest| rest.starts_with(char::is_whitespace))
    {
        return true;
    }
    let rest = line.trim_start_matches(char::is_whitespace);
    if let Some(doctype) = strip_prefix_ci(rest, "<!doctype") {
        // `\s+` backs off to one blank when there are more, and a blank isn't `html`
        let mut blanks = doctype.chars().take_while(|&c| c.is_whitespace());
        return match (blanks.next(), blanks.next()) {
            (None, _) => false,
            (Some(_), Some(_)) => true,
            (Some(blank), None) => {
                let after = &doctype[blank.len_utf8()..];
                let html = strip_prefix_ci(after, "html")
                    .is_some_and(|r| r.starts_with([' ', '\t', '\n', '\x0c', '>']));
                !html
            }
        };
    }
    is_soap_envelope(rest)
}

/// XML's SOAP envelope, past the line's leading blanks: `<NAME:Envelope`, blanks, then
/// `xmlns:NAME` declared as SOAP's namespace, and the tag's `>`.
fn is_soap_envelope(rest: &str) -> bool {
    let is_name = |b: u8| is_word(b) || b == b'-';
    let Some(tag) = rest.strip_prefix('<') else {
        return false;
    };
    let n = tag.bytes().take_while(|&b| is_name(b)).count();
    let (name, rest) = tag.split_at(n);
    let Some(rest) = rest.strip_prefix(":Envelope") else {
        return false;
    };
    let attr = rest.trim_start_matches(char::is_whitespace);
    if n == 0 || attr.len() == rest.len() {
        return false;
    }
    let Some(rest) = attr
        .strip_prefix("xmlns:")
        .and_then(|r| r.strip_prefix(name))
        .and_then(|r| r.trim_start_matches(char::is_whitespace).strip_prefix('='))
    else {
        return false;
    };
    rest.trim_start_matches(char::is_whitespace)
        .strip_prefix("\"http://schemas.xmlsoap.org/soap/envelope/\"")
        .is_some_and(|r| r.trim_start_matches(char::is_whitespace).starts_with('>'))
}

/// bat's Dockerfile first-line regex, `^\s*(?i:(from(?!\s+\S+\s+import)|arg))\s+`: `FROM` or
/// `ARG` and a blank, but not Python's `from x import y`. `\s` is Unicode's blank, as for
/// [`is_xml_line`].
fn is_dockerfile_line(line: &str) -> bool {
    let rest = line.trim_start_matches(char::is_whitespace);
    if let Some(after) = strip_prefix_ci(rest, "arg") {
        return after.starts_with(char::is_whitespace);
    }
    let Some(after) = strip_prefix_ci(rest, "from").filter(|a| a.starts_with(char::is_whitespace))
    else {
        return false;
    };
    // `\S+` takes the whole word, as the blanks after it must follow it, and `\s+` every blank
    // before `import`, which isn't one
    let word = after.trim_start_matches(char::is_whitespace);
    let Some(end) = word.find(char::is_whitespace) else {
        return true;
    };
    let import = strip_prefix_ci(
        word[end..].trim_start_matches(char::is_whitespace),
        "import",
    )
    .is_some();
    !import
}

/// `s` past `prefix`, ignoring ASCII case.
fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

/// bat's Makefile first-line regex, ignoring case (`lower` is the line in lowercase):
/// `^\#! .* \bmake\b`, or `^\# \s* -\*- [^*]* makefile [^*]* -\*-`.
fn is_make_line(lower: &str) -> bool {
    if lower.starts_with("#!") {
        return has_word(lower, 2, "make");
    }
    modeline(lower, |text| text.contains("makefile"))
}

/// Ruby's first line, lowercase: `#!` and the word `ruby` or `jruby` after it, or a
/// `# -*- … ruby … -*-` modeline.
fn is_ruby_line(lower: &str) -> bool {
    if lower.starts_with("#!") {
        return has_word(lower, 2, "ruby") || has_word(lower, 2, "jruby");
    }
    modeline(lower, |text| text.contains("ruby"))
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
        assert_eq!(Lang::from_token("csharp"), Some(Lang::CSharp));
        assert_eq!(Lang::from_token("go"), Some(Lang::Go));
        assert_eq!(Lang::from_token("python"), Some(Lang::Python));
        assert_eq!(Lang::from_token("py"), Some(Lang::Python));
        assert_eq!(Lang::from_token("lua"), Some(Lang::Lua));
        assert_eq!(Lang::from_token("sql"), Some(Lang::Sql));
        assert_eq!(Lang::from_token("powershell"), Some(Lang::PowerShell));
        assert_eq!(Lang::from_token("ps1"), Some(Lang::PowerShell));
        assert_eq!(Lang::from_token("pwsh"), Some(Lang::PowerShell));
        assert_eq!(Lang::from_token("posh"), Some(Lang::PowerShell));
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
        let python = [
            "#!/usr/bin/python3",
            "#!/usr/bin/env python",
            "#! /usr/bin/env python3.12 -u",
            "#!/usr/local/bin/python2.7",
            "#!/opt/python/bin/python",
        ];
        for line in python {
            assert_eq!(Lang::from_first_line(line), Some(Lang::Python), "{line}");
        }
        // the set tries Ruby's regex before Python's
        let ruby = [
            "#!/usr/bin/env ruby",
            "#!/usr/bin/jruby -w",
            "#!/usr/bin/env python3 # ruby",
            "# -*- mode: ruby -*-",
        ];
        for line in ruby {
            assert_eq!(Lang::from_first_line(line), Some(Lang::Ruby), "{line}");
        }
        assert_eq!(
            Lang::from_first_line("<?xml version=\"1.0\"?>"),
            Some(Lang::Xml)
        );
        let neither = [
            "#!/usr/bin/env fish",
            "#!/home/ls/bin/sh",
            "#!/usr/bin/env node",
            "#!/bin/shell",
            "#!python3",
            "#!/usr/bin/python312",
            "#!/usr/bin/rubyx",
            "<?xml",
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

    /// Lines no first-line regex can match are told apart without the set.
    #[test]
    fn first_lines_of_no_syntax() {
        let lines = [
            "# ada",
            "<p align=\"center\">",
            "{",
            "[package]",
            "Some text, from here.",
            "# Requirements",
            "# Über",
            "---",
            "",
        ];
        for line in lines {
            assert_eq!(Lang::first_line(line), FirstLine::None, "{line}");
        }
        assert_eq!(Lang::first_line("#!/usr/bin/env perl"), FirstLine::Ask);
        // ignoring case, the Kelvin sign is a `k` and `ſ` an `s`
        assert_eq!(Lang::first_line("#!/usr/bin/ma\u{212a}e"), FirstLine::Ask);
        assert_eq!(
            Lang::first_line("#!/usr/bin/pip-requirement\u{17f}"),
            FirstLine::Ask
        );
        assert_eq!(Lang::first_line("#!/usr/bin/\u{212a}sh"), FirstLine::Ask);
    }

    /// XML's and Dockerfile's regexes, which the `regex` crate can't read, decided here; ada's
    /// tests hold them to syntect's on lines of every kind (`first_lines_agree_with_the_set`).
    #[test]
    fn xml_and_dockerfile_first_lines_are_decided() {
        assert!(is_xml_line("<?xml version=\"1.0\"?>"));
        assert!(!is_xml_line("<?xml"));
        assert!(is_xml_line("  <!DOCTYPE note SYSTEM \"note.dtd\">"));
        assert!(
            is_xml_line("<!doctype  html>"),
            "`\\s+` backs off to one blank"
        );
        assert!(is_xml_line("<!DOCTYPE htmlx>"));
        assert!(is_xml_line("\u{3000}<!DOCTYPE\u{3000}note>"));
        assert!(!is_xml_line("<!DOCTYPE html>"));
        assert!(!is_xml_line("<!DOCTYPE HTML>"));
        // nothing after `html` for `[ \t\n\f>]` to match: the lookahead holds
        assert!(is_xml_line("<!DOCTYPE HTML"));
        let soap = "<soap:Envelope xmlns:soap = \"http://schemas.xmlsoap.org/soap/envelope/\" >";
        assert!(is_xml_line(soap));
        assert!(!is_xml_line(&soap.replace("xmlns:soap", "xmlns:s")));
        assert!(!is_xml_line(
            &soap.replace("soap:Envelope ", "soap:Envelope")
        ));
        for line in [
            "FROM rust:1.95",
            "  arg VERSION=1",
            "from rust",
            "from x as y",
            "\u{3000}FROM x",
        ] {
            assert!(is_dockerfile_line(line), "{line}");
        }
        for line in ["from os import path", "FROM x IMPORT y", "fromage", "ARG"] {
            assert!(!is_dockerfile_line(line), "{line}");
        }
    }

    #[test]
    fn asking_names_a_syntax_not_covered() {
        // Odin's, a syntax no lexer covers, is the set's last regex: the walk asks about it first
        let mut asked = Vec::new();
        let found = Lang::first_line_asking("// -*- Odin -*-", &mut |i, _| {
            asked.push(first_line_syntaxes().nth(i).unwrap());
            Some(true)
        });
        assert_eq!(found, FirstLine::Syntax("Odin"));
        assert_eq!(asked, ["Odin"]);
        let none = Lang::first_line_asking("// -*- Odin -*-", &mut |_, _| Some(false));
        assert_eq!(none, FirstLine::None);
        // a regex nobody can tell about, with no test for ASCII lines to fall back on
        let unknown = Lang::first_line_asking("// -*- Odin -*-", &mut |_, _| None);
        assert_eq!(unknown, FirstLine::Ask);
        // the Kelvin sign: the table's test for ASCII lines stands in when asking can't tell
        let make = Lang::first_line_asking("#!/usr/bin/ma\u{212a}e", &mut |_, _| None);
        assert_eq!(make, FirstLine::None);
    }
}
