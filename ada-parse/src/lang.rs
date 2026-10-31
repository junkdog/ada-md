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
];

impl Lang {
    pub const ALL: [Lang; 4] = [Lang::Json, Lang::Log, Lang::Makefile, Lang::Bash];

    /// The name of the syntax it stands in for, as syntect names it.
    pub fn name(self) -> &'static str {
        match self {
            Lang::Json => "JSON",
            Lang::Log => "log",
            Lang::Makefile => "Makefile",
            Lang::Bash => "Bourne Again Shell (bash)",
        }
    }

    /// The extensions and whole file names its syntax claims, lowercase: syntect compares them
    /// ignoring ASCII case.
    fn extensions(self) -> &'static [&'static str] {
        match self {
            Lang::Json => &[
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
            Lang::Log => &["log"],
            Lang::Makefile => &[
                "make",
                "gnumakefile",
                "makefile",
                "makefile.am",
                "makefile.in",
                "ocamlmakefile",
                "mak",
                "mk",
            ],
            Lang::Bash => &[
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
        }
    }

    /// The language a syntax's own name gives, as `find_syntax_by_name` matches it: exactly.
    pub fn from_name(name: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| l.name() == name)
    }

    /// The language an extension or a whole file name gives, as `find_syntax_by_extension`
    /// matches it: ignoring ASCII case.
    pub fn from_extension(ext: &str) -> Option<Lang> {
        Lang::ALL
            .into_iter()
            .find(|l| l.extensions().iter().any(|e| e.eq_ignore_ascii_case(ext)))
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
        assert_eq!(Lang::from_token("rust"), None);
        assert_eq!(Lang::from_token("kt"), None);
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
        assert_eq!(Lang::from_file_name("Cargo.lock"), None);
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
