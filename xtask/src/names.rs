//! `cargo xtask names [--check]`: what ada knows of bat's syntax set without syntect (ADR-43),
//! from two-face's set (bat's grammars): `assets/syntax-names.txt` ([`names`]), every syntax's
//! name and extensions in the set's order, and `assets/syntax-first-lines.rs` ([`first_lines`]),
//! the set's first-line regexes in its order, written for the `regex` crate. ada names a file's
//! or a fence's language from them, its own lexers' or one it shows plain.
//!
//! `--check` builds them and compares them with the files instead, exit 1 when they differ:
//! after a two-face update.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use syntect::dumps;
use syntect::parsing::SyntaxSet;

pub fn run(root: &Path, check: bool) -> Result<ExitCode, String> {
    let set = two_face_set(root)?;
    let mut stale = Vec::new();
    for (out, text) in [
        (names_path(root), names(&set)?),
        (first_lines_path(root), first_lines(&set)?),
    ] {
        if check {
            if fs::read(&out).ok().as_deref() != Some(text.as_bytes()) {
                stale.push(out.display().to_string());
            }
        } else {
            fs::write(&out, text).map_err(|e| format!("{}: {e}", out.display()))?;
        }
    }
    if stale.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    eprintln!(
        "stale: {}; `cargo xtask names` rebuilds them",
        stale.join(", ")
    );
    Ok(ExitCode::FAILURE)
}

/// two-face's set with Oniguruma's regexes, the fuller of its two: fancy-regex's leaves out the
/// grammars it can't run.
pub fn two_face_set(root: &Path) -> Result<SyntaxSet, String> {
    let source = two_face_dir(root)?.join("generated/syntaxes-onig-newlines.bin");
    let bytes = fs::read(&source).map_err(|e| format!("{}: {e}", source.display()))?;
    dumps::from_uncompressed_data(&bytes).map_err(|e| format!("{}: {e}", source.display()))
}

/// The set's syntaxes in its order, a line each: the name, then each extension, tab-separated.
/// syntect finds a syntax by name or extension from the last to the first, which ada's reading
/// of this list follows.
pub fn names(set: &SyntaxSet) -> Result<String, String> {
    let mut out = String::new();
    for s in set.syntaxes() {
        let fields = std::iter::once(&s.name).chain(&s.file_extensions);
        for (i, field) in fields.enumerate() {
            if field.is_empty() || field.contains(['\t', '\n']) {
                return Err(format!("{}: can't list {field:?}", s.name));
            }
            if i > 0 {
                out.push('\t');
            }
            out.push_str(field);
        }
        out.push('\n');
    }
    Ok(out)
}

/// The set's first-line regexes in its order, as a Rust array of `(syntax, regex)` that ada
/// `include!`s. syntect tries them from the last to the first, which ada's walk follows. Each
/// regex is Oniguruma's, as the grammar has it, made one the `regex` crate reads the same on a
/// line ([`for_regex_crate`]).
pub fn first_lines(set: &SyntaxSet) -> Result<String, String> {
    let mut out = String::from(
        "// `cargo xtask names` writes this file from two-face's syntax set: its first-line regexes,\n\
         // in its order, for the `regex` crate (ADR-43). Don't edit it by hand.\n[\n",
    );
    for s in set.syntaxes() {
        let Some(regex) = &s.first_line_match else {
            continue;
        };
        let regex = for_regex_crate(regex);
        if s.name.contains(['"', '\\']) || regex.contains("\"#") {
            return Err(format!("{}: can't write {regex:?}", s.name));
        }
        out.push_str(&format!("    (\"{}\", r#\"{regex}\"#),\n", s.name));
    }
    out.push_str("]\n");
    Ok(out)
}

/// An Oniguruma regex of the set as the `regex` crate reads it: `\h`, a hex digit, spelled out;
/// a `{` that opens no repetition escaped; and a space in a class written `\x20`, since the
/// `regex` crate's `(?x)` drops blanks there too, where Oniguruma's keeps them. Lookarounds and
/// back-references stay: the `regex` crate refuses those regexes (XML's, Dockerfile's), which
/// ada-parse's table decides itself. Nothing else in the set's first-line regexes differs
/// between the two on a single line.
pub fn for_regex_crate(regex: &str) -> String {
    let mut out = String::with_capacity(regex.len());
    // nested classes, and whether the class just opened (`[` or `[^`), where `]` is a literal
    let (mut class, mut opened) = (0usize, false);
    let mut chars = regex.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let was_opened = std::mem::take(&mut opened);
        match c {
            '\\' => match chars.next() {
                Some((_, 'h')) => out.push_str("[0-9A-Fa-f]"),
                Some((_, e)) => {
                    out.push('\\');
                    out.push(e);
                }
                None => out.push('\\'),
            },
            '[' => {
                class += 1;
                opened = true;
                out.push('[');
                if chars.peek().is_some_and(|&(_, c)| c == '^') {
                    chars.next();
                    out.push('^');
                }
            }
            ']' if class > 0 && !was_opened => {
                class -= 1;
                out.push(']');
            }
            ' ' if class > 0 => out.push_str("\\x20"),
            '{' if !opens_repetition(&regex[i + 1..]) => out.push_str("\\{"),
            c => out.push(c),
        }
    }
    out
}

/// Whether `rest`, after a `{`, is a repetition's `n}`, `n,}` or `n,m}`.
fn opens_repetition(rest: &str) -> bool {
    let Some((inside, _)) = rest.split_once('}') else {
        return false;
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    match inside.split_once(',') {
        Some((n, m)) => digits(n) && (m.is_empty() || digits(m)),
        None => digits(inside),
    }
}

/// two-face's crate directory, as the lockfile resolves it.
pub fn two_face_dir(root: &Path) -> Result<PathBuf, String> {
    let out = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--manifest-path",
        ])
        .arg(root.join("Cargo.toml"))
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let json = String::from_utf8_lossy(&out.stdout);
    // a package's object starts with its name and version (a dependency's, with its name and
    // source); its manifest path follows
    let at = json
        .find(r#"{"name":"two-face","version":"#)
        .ok_or("cargo metadata: no two-face package")?;
    let key = r#""manifest_path":""#;
    let start = json[at..]
        .find(key)
        .ok_or("cargo metadata: no manifest path")?
        + at
        + key.len();
    let end = json[start..]
        .find('"')
        .ok_or("cargo metadata: unterminated path")?
        + start;
    let manifest = PathBuf::from(&json[start..end]);
    Ok(manifest.parent().unwrap().to_path_buf())
}

pub fn names_path(root: &Path) -> PathBuf {
    root.join("assets/syntax-names.txt")
}

pub fn first_lines_path(root: &Path) -> PathBuf {
    root.join("assets/syntax-first-lines.rs")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
    }

    /// The files in `assets/` are what `cargo xtask names` builds now.
    #[test]
    fn the_assets_are_current() {
        let set = two_face_set(root()).unwrap();
        let listed = fs::read_to_string(names_path(root())).unwrap();
        assert!(listed == names(&set).unwrap(), "run `cargo xtask names`");
        let first = fs::read_to_string(first_lines_path(root())).unwrap();
        assert!(
            first == first_lines(&set).unwrap(),
            "run `cargo xtask names`"
        );
    }

    #[test]
    fn oniguruma_spellings_become_the_regex_crates() {
        assert_eq!(
            for_regex_crate(r"^commit\s+\h{7,}"),
            r"^commit\s+[0-9A-Fa-f]{7,}"
        );
        assert_eq!(for_regex_crate(r"^{% extends"), r"^\{% extends");
        assert_eq!(for_regex_crate(r"a{2}b{1,3}\{"), r"a{2}b{1,3}\{");
        assert_eq!(for_regex_crate(r"\\h"), r"\\h");
        assert_eq!(
            for_regex_crate(r"(?x) Index:[ ] | [^ a] | []] | \[ x"),
            r"(?x) Index:[\x20] | [^\x20a] | []] | \[ x"
        );
    }
}
