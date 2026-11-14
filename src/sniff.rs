//! What a text holds when its name can't tell: piped stdin, or a file with no extension. Only
//! the first few KB are looked at (ADR-30, FEAT-020). Sure signals come first, in a fixed
//! order: HTML's doctype, a git patch, a mermaid diagram, the first line as bat's syntax set
//! reads it (a shebang, `<?php`, `<?xml`), JSON, front matter, the configuration formats whose
//! every line has one shape (`conf`), YAML, `.env`, the key-value family and a log. Code and
//! markdown are then told by the weight of their lines' signals (`signals`), the C family
//! split as a `.h` is. Markdown's own signals and `.gitignore`'s globs are tried last. Text none
//! of them claims is left to the caller.

mod conf;
mod signals;

use std::sync::LazyLock;

use ada_parse::Lang;
use regex::Regex;

use crate::filetype::FileType;
use crate::{diagram, highlight};

/// How much of the text is looked at.
const SAMPLE: usize = 8 * 1024;

/// The type `text`'s contents show, or `None` when nothing stands out.
pub fn sniff(text: &str) -> Option<FileType> {
    let sample = sample(text);
    let lines: Vec<&str> = sample.lines().collect();
    let first = lines.first().copied().unwrap_or_default();
    let code = |lang| Some(FileType::Text { lang: Some(lang) });
    let lang = |lang: Lang| code(lang.label());
    // bat's first-line regexes take `<!doctype html>` for Svelte, a patch's `From <sha>` for a
    // Dockerfile's `FROM`, and `git show` for a Git Log, which leaves its diff plain
    if is_html(&lines) {
        return code("HTML");
    }
    if is_git_patch(&lines) || is_unified_diff(&lines) {
        return code("Diff");
    }
    // before front matter: a diagram's `---` holds its config, not a note's
    if diagram::is_diagram(sample) {
        return code(diagram::LANG);
    }
    // bat's Diff takes any first line that starts `--- `, Lua's and SQL's comments too
    let first_lang = highlight::first_line_language(first);
    if let Some(l) = first_lang.filter(|&l| l != "Diff" || has_hunk(&lines)) {
        return code(l);
    }
    if let Some(l) = shebang(first) {
        return lang(l);
    }
    if first_content(&lines).is_some_and(|l| l.starts_with("#lang ")) {
        return lang(Lang::Racket);
    }
    if is_json(sample) {
        return code("JSON");
    }
    if has_front_matter(&lines) {
        return Some(FileType::Markdown);
    }
    if let Some(l) = conf::sure(&lines) {
        return lang(l);
    }
    if is_yaml(&lines) {
        return code("YAML");
    }
    if conf::is_dotenv(&lines) {
        return lang(Lang::DotEnv);
    }
    if is_toml(&lines) {
        return lang(conf::key_values(&lines));
    }
    // before markdown: a log line can hold a link, but no note has a timestamp on every line
    if is_log(&lines) {
        return lang(Lang::Log);
    }
    if is_literate_haskell(&lines) {
        return lang(Lang::LiterateHaskell);
    }
    match signals::best(&lines) {
        Some(signals::MARKDOWN) => return Some(FileType::Markdown),
        Some(signals::CFAM) => return lang(Lang::for_header(sample.as_bytes())),
        Some(l) => return code(l),
        None => {}
    }
    if conf::is_gitignore(&lines) {
        return lang(Lang::GitIgnore);
    }
    if is_markdown(&lines) {
        return Some(FileType::Markdown);
    }
    conf::is_properties(&lines)
        .then(|| lang(Lang::Properties))
        .flatten()
}

/// The language a shebang's interpreter names, for those bat's first lines don't know.
fn shebang(first: &str) -> Option<Lang> {
    let line = first.strip_prefix("#!")?;
    let mut words = line.split_whitespace();
    let mut prog = words.next()?.rsplit('/').next()?;
    if prog == "env" {
        prog = words.find(|w| !w.starts_with('-') && !w.contains('='))?;
    }
    let prog = prog.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    Some(match prog {
        "groovy" => Lang::Groovy,
        "kotlin" | "kotlinc" | "kscript" => Lang::Kotlin,
        "scala" | "amm" | "scala-cli" => Lang::Scala,
        "elixir" => Lang::Elixir,
        "lua" | "luajit" => Lang::Lua,
        "pwsh" | "powershell" => Lang::PowerShell,
        "sbcl" | "clisp" | "ccl" | "ecl" => Lang::Lisp,
        "guile" | "csi" | "chicken" | "chez" | "scheme" | "chibi-scheme" | "gsi" | "gosh" => {
            Lang::Scheme
        }
        "racket" => Lang::Racket,
        "bb" | "clojure" | "clj" | "joker" | "planck" | "nbb" => Lang::Clojure,
        "emacs" => Lang::EmacsLisp,
        "ts-node" | "deno" | "tsx" => Lang::TypeScript,
        "runhaskell" | "runghc" | "stack" | "cabal" => Lang::Haskell,
        "dotnet-script" => Lang::CSharp,
        _ => return None,
    })
}

/// Why [`sniff`] reads `text` as it does: what it gives, each candidate's score from the line
/// signals, and the signals that matched with the first line each matched. For the census
/// harness (`examples/sniff_census.rs --explain`).
#[doc(hidden)]
pub fn explain(text: &str) -> String {
    use std::fmt::Write;
    let lines: Vec<&str> = sample(text).lines().collect();
    let mut hits: Vec<(&str, usize, String)> = Vec::new();
    let scores = signals::scores(&lines, |re, line| {
        match hits.iter_mut().find(|(r, _, _)| *r == re) {
            Some(h) => h.1 += 1,
            None => hits.push((re, 1, line.to_owned())),
        }
    });
    let mut out = format!("sniffed: {:?}\n", sniff(text));
    let mut scores: Vec<_> = scores.into_iter().filter(|(_, s)| *s > 0).collect();
    scores.sort_by_key(|s| std::cmp::Reverse(s.1));
    for (lang, score) in scores {
        let _ = writeln!(out, "  {lang}: {score}");
    }
    for (re, n, line) in hits {
        let _ = writeln!(out, "  {n:>3}x {re}\n        {line}");
    }
    out
}

/// A unified diff anywhere in the sample, after a patch's description too: a file header
/// (`--- `, then `+++ `) and a hunk header after it.
fn is_unified_diff(lines: &[&str]) -> bool {
    static HUNK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^@@ -\d+(,\d+)? \+\d+(,\d+)? @@").unwrap());
    lines
        .windows(3)
        .any(|w| w[0].starts_with("--- ") && w[1].starts_with("+++ ") && HUNK.is_match(w[2]))
}

/// A diff's hunk or its file header's second line.
fn has_hunk(lines: &[&str]) -> bool {
    lines
        .iter()
        .any(|l| l.starts_with("@@ ") || l.starts_with("+++ ") || l.starts_with("diff "))
}

/// Literate Haskell: `\begin{code}`, or bird tracks (`> `) holding Haskell.
fn is_literate_haskell(lines: &[&str]) -> bool {
    static HASKELL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^> ?(module [A-Z]|import [A-Z]|import qualified|data |newtype |instance |[a-z_][\w']*\s+::\s)").unwrap()
    });
    lines.iter().any(|l| l.starts_with(r"\begin{code}"))
        || lines.iter().filter(|l| l.starts_with('>')).count() >= 2
            && lines.iter().any(|l| HASKELL.is_match(l))
}

/// The text's first [`SAMPLE`] bytes, cut at the last whole line when there are more.
fn sample(text: &str) -> &str {
    if text.len() <= SAMPLE {
        return text;
    }
    let mut end = SAMPLE;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let head = &text[..end];
    head.rfind('\n').map_or(head, |nl| &head[..nl])
}

fn first_content<'a>(lines: &[&'a str]) -> Option<&'a str> {
    lines.iter().map(|l| l.trim()).find(|l| !l.is_empty())
}

fn is_html(lines: &[&str]) -> bool {
    first_content(lines).is_some_and(|l| {
        let l = l.as_bytes();
        let starts =
            |p: &str| l.len() >= p.len() && l[..p.len()].eq_ignore_ascii_case(p.as_bytes());
        starts("<!doctype html") || starts("<html")
    })
}

/// `git show`, `git log -p` or `git format-patch` output: a commit header, then a diff.
fn is_git_patch(lines: &[&str]) -> bool {
    static HEADER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(commit|From) [0-9a-f]{7,64}\b").unwrap());
    lines.first().is_some_and(|l| HEADER.is_match(l))
        && lines.iter().any(|l| l.starts_with("diff --git "))
}

/// Whether `s` starts a JSON object or array, and holds only JSON values as far as it goes: a
/// document, or JSON lines. It may stop anywhere, the sample being cut.
fn is_json(s: &str) -> bool {
    #[derive(Clone, Copy, PartialEq)]
    enum Want {
        /// Only an object or array: the start, and between JSON lines.
        Top,
        Value,
        ValueOrClose,
        Key,
        KeyOrClose,
        Colon,
        /// A comma or the closer, after a value.
        Next,
    }
    let b = s.as_bytes();
    let mut closers = Vec::new();
    let mut want = Want::Top;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if matches!(c, b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
            continue;
        }
        let close = closers.last() == Some(&c);
        match want {
            Want::Top | Want::Value | Want::ValueOrClose if c == b'{' => {
                closers.push(b'}');
                want = Want::KeyOrClose;
            }
            Want::Top | Want::Value | Want::ValueOrClose if c == b'[' => {
                closers.push(b']');
                want = Want::ValueOrClose;
            }
            Want::ValueOrClose | Want::KeyOrClose | Want::Next if close => {
                closers.pop();
                want = if closers.is_empty() {
                    Want::Top
                } else {
                    Want::Next
                };
            }
            Want::Next if c == b',' => {
                want = if closers.last() == Some(&b'}') {
                    Want::Key
                } else {
                    Want::Value
                };
            }
            Want::Colon if c == b':' => want = Want::Value,
            Want::Key | Want::KeyOrClose if c == b'"' => {
                match string_end(b, i) {
                    Str::End(end) => i = end,
                    Str::Cut => return true,
                    Str::Bad => return false,
                }
                want = Want::Colon;
                continue;
            }
            Want::Value | Want::ValueOrClose => {
                let end = match c {
                    b'"' => match string_end(b, i) {
                        Str::End(end) => Some(end),
                        Str::Cut => None,
                        Str::Bad => return false,
                    },
                    b'-' | b'0'..=b'9' => {
                        let n = b[i..]
                            .iter()
                            .take_while(|c| {
                                matches!(c, b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                            })
                            .count();
                        Some(i + n).filter(|&e| e < b.len())
                    }
                    _ => {
                        let word = ["true", "false", "null"]
                            .into_iter()
                            .find(|w| w.as_bytes()[0] == c);
                        let Some(word) = word.map(str::as_bytes) else {
                            return false;
                        };
                        let rest = &b[i..];
                        if rest.len() < word.len() {
                            return word.starts_with(rest);
                        }
                        if !rest.starts_with(word) {
                            return false;
                        }
                        Some(i + word.len())
                    }
                };
                let Some(end) = end else {
                    return true;
                };
                i = end;
                want = Want::Next;
                continue;
            }
            _ => return false,
        }
        i += 1;
    }
    // something past the opening bracket
    want != Want::Top || closers.is_empty() && !s.trim().is_empty()
}

/// How a JSON string goes on.
enum Str {
    /// It closes, and the text goes on here.
    End(usize),
    /// The text ends first.
    Cut,
    /// A raw newline: not JSON.
    Bad,
}

/// Where the string opening at `b[start]` ends.
fn string_end(b: &[u8], start: usize) -> Str {
    let mut i = start + 1;
    while i < b.len() {
        match b[i] {
            b'"' => return Str::End(i + 1),
            b'\\' => i += 2,
            b'\n' => return Str::Bad,
            _ => i += 1,
        }
    }
    Str::Cut
}

/// Lines with content that aren't comments (`#`, INI's `;`) and aren't indented, the ones that
/// show a file's shape: what's indented belongs to the line above.
fn top_lines<'a>(lines: &'a [&'a str]) -> impl Iterator<Item = &'a str> {
    lines
        .iter()
        .copied()
        .filter(|l| !l.trim().is_empty() && !l.starts_with([' ', '\t', '#', ';']))
}

/// Whether the first top line has the shape and almost all of them do.
fn mostly(lines: &[&str], shaped: impl Fn(&str) -> bool) -> bool {
    let mut top = top_lines(lines).peekable();
    if !top.peek().is_some_and(|l| shaped(l)) {
        return false;
    }
    let (yes, total) = top.fold((0, 0), |(y, t), l| (y + usize::from(shaped(l)), t + 1));
    yes * 10 >= total * 9
}

static YAML_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^(- +)*("[^"]*"|'[^']*'|[\w./$@-]+)\s*:(\s|$)"#).unwrap());

/// Front matter, `---` or `+++` fenced, then something that isn't more YAML: a note.
fn has_front_matter(lines: &[&str]) -> bool {
    let Some(fence @ ("---" | "+++")) = lines.first().map(|l| l.trim_end()) else {
        return false;
    };
    let close = lines
        .iter()
        .skip(1)
        .position(|l| l.trim_end() == fence || fence == "---" && l.trim_end() == "...");
    close.is_some_and(|c| {
        let next = first_content(&lines[c + 2..]);
        next.is_some_and(|l| !YAML_KEY.is_match(l) && !l.starts_with("- ") && l != "---")
    })
}

fn is_yaml(lines: &[&str]) -> bool {
    let doc_start = |l: &str| l == "---" || l.starts_with("--- ");
    // `--- ` then a tag, an anchor, a block scalar or a comment: Lua's `--- doc` comments aren't
    let first_start = |l: &str| {
        l == "---"
            || l.strip_prefix("--- ")
                .is_some_and(|r| r.starts_with(['!', '&', '|', '>', '#']))
    };
    if lines.first().is_some_and(|l| first_start(l.trim_end())) {
        return true;
    }
    // a mapping: a list alone is markdown's as much as YAML's
    mostly(lines, |l| {
        YAML_KEY.is_match(l) || l.starts_with("- ") || doc_start(l.trim_end()) || l == "..."
    }) && top_lines(lines)
        .next()
        .is_some_and(|l| YAML_KEY.is_match(l))
}

/// TOML, or an INI or `.env` file, which highlight as TOML well enough.
fn is_toml(lines: &[&str]) -> bool {
    static TABLE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"^\[\[?\s*[\w."' -]+\s*\]\]?\s*(#.*)?$"#).unwrap());
    static KEY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"^("[^"]*"|'[^']*'|[\w.-]+)(\s*\.\s*("[^"]*"|[\w-]+))*\s*=\s*\S"#).unwrap()
    });
    // the closing lines of a multi-line array, table or string
    let closer = |l: &str| matches!(l.trim_end(), "]" | "}" | "\"\"\"" | "'''");
    let first = |l: &str| TABLE.is_match(l) || KEY.is_match(l);
    mostly(lines, |l| first(l) || closer(l)) && top_lines(lines).next().is_some_and(first)
}

/// Most lines led by a timestamp or a level, as loggers, journalctl and syslog write them.
fn is_log(lines: &[&str]) -> bool {
    static LEAD: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(concat!(
            r"^\[?(",
            r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}",
            r"|\d{4}/\d{2}/\d{2} \d{2}:\d{2}",
            r"|[A-Z][a-z]{2} [ \d]\d \d{2}:\d{2}:\d{2}",
            r"|\d{2}:\d{2}:\d{2}",
            r"|(TRACE|DEBUG|INFO|NOTICE|WARN|WARNING|ERROR|FATAL|CRITICAL)\b",
            r")",
        ))
        .unwrap()
    });
    let content = lines.iter().filter(|l| !l.trim().is_empty());
    let (led, total) = content.fold((0, 0), |(n, t), l| {
        (n + usize::from(LEAD.is_match(l)), t + 1)
    });
    total > 0 && led * 2 >= total
}

/// Markdown's own syntax, when the line signals found too little: one sure sign (a fence, a
/// table, a link, a heading with blank lines around it, not one of a block of `#` comments nor a
/// sentence, a setext underline), or a few lines of the weaker ones (list items at the margin or nested,
/// not a block comment's ` * `; quotes; bold). Backticks alone aren't one: compilers quote code
/// in them.
fn is_markdown(lines: &[&str]) -> bool {
    static FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^ {0,3}(```|~~~)").unwrap());
    static TABLE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)+\|?\s*$").unwrap());
    static LINK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\[[^\]]+\]\([^)\s]+\)|\[\[[^\]]+\]\]").unwrap());
    static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#{1,6}\s+\S").unwrap());
    static SETEXT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^={3,}\s*$").unwrap());
    static WEAK: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(( {2,}|\t+)?([-*+]|\d{1,3}[.)])\s+\S| {0,3}>\s)|\*\*[^*]+\*\*").unwrap()
    });
    let blank = |i: usize| lines.get(i).is_none_or(|l| l.trim().is_empty());
    let sure = lines.iter().enumerate().any(|(i, l)| {
        FENCE.is_match(l)
            || TABLE.is_match(l)
            || LINK.is_match(l)
            || HEADING.is_match(l)
                && blank(i + 1)
                && (i == 0 || blank(i - 1))
                && !l.trim_end().ends_with(['.', ',', ';', ':'])
            || SETEXT.is_match(l) && i > 0 && !blank(i - 1)
    });
    let content = lines.iter().filter(|l| !l.trim().is_empty()).count();
    let weak = lines.iter().filter(|l| WEAK.is_match(l)).count();
    sure || weak >= 2 && weak * 5 >= content
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lang(lang: &'static str) -> Option<FileType> {
        Some(FileType::Text { lang: Some(lang) })
    }

    #[test]
    fn samples() {
        let log = lang("log");
        let cases: &[(&str, &str, Option<FileType>)] = &[
            ("empty", "", None),
            ("blank", "\n  \n", None),
            (
                "script",
                "#!/usr/bin/env zsh\nprint -r -- $1\n",
                lang("Bourne Again Shell (bash)"),
            ),
            ("xml", "<?xml version=\"1.0\"?>\n<a/>\n", lang("XML")),
            (
                "html",
                "\n<!DOCTYPE html>\n<html><body>hi</body></html>\n",
                lang("HTML"),
            ),
            (
                "git diff",
                "diff --git a/src/a.rs b/src/a.rs\nindex 1..2 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-a\n+b\n",
                lang("Diff"),
            ),
            (
                "diff -u",
                "--- a.txt\n+++ b.txt\n@@ -1 +1 @@\n-a\n+b\n",
                lang("Diff"),
            ),
            (
                "git show",
                "commit 6bf2b19e0c\nAuthor: A <a@b>\n\n    feat: x\n\ndiff --git a/x b/x\n",
                lang("Diff"),
            ),
            (
                "format-patch",
                "From 6bf2b19e0c4a Mon Sep 17 00:00:00 2001\nFrom: A <a@b>\nSubject: [PATCH] x\n\n---\ndiff --git a/x b/x\n",
                lang("Diff"),
            ),
            (
                "git log",
                "commit 6bf2b19e0c\nAuthor: A <a@b>\n\n    feat: x\n",
                lang("Git Log"),
            ),
            (
                "json",
                "{\n  \"name\": \"ada\",\n  \"tags\": [1, -2.5e3, true, null]\n}\n",
                lang("JSON"),
            ),
            ("json array", "[{\"a\": 1}, {\"a\": 2}]", lang("JSON")),
            (
                "json lines",
                "{\"level\":\"info\",\"msg\":\"up\"}\n{\"level\":\"warn\",\"msg\":\"slow\"}\n",
                lang("JSON"),
            ),
            (
                "json, cut",
                "{\"a\": [1, 2, {\"b\": \"unfinish",
                lang("JSON"),
            ),
            (
                "a link isn't json",
                "[ada](https://example.com) is a viewer\n",
                Some(FileType::Markdown),
            ),
            (
                "a footnote isn't json",
                "[1] Knuth, 1984\n[2] Gruber, 2004\n",
                None,
            ),
            ("a template isn't json", "{{ range .Items }}\n", None),
            (
                "kubectl yaml",
                "apiVersion: v1\nkind: Pod\nmetadata:\n  name: ada\n  labels:\n    app: ada\nspec:\n  containers:\n  - name: ada\n    image: ada:1\n",
                lang("YAML"),
            ),
            (
                "yaml documents",
                "---\nkind: A\n---\nkind: B\n",
                lang("YAML"),
            ),
            (
                "yaml with a comment",
                "# CI\non: push\njobs:\n  test:\n    runs-on: ubuntu\n",
                lang("YAML"),
            ),
            (
                "front matter",
                "---\ntitle: Ada\ntags: [cat]\n---\n\nAda is a cat.\n",
                Some(FileType::Markdown),
            ),
            (
                "toml front matter",
                "+++\ntitle = \"Ada\"\n+++\nAda is a cat.\n",
                Some(FileType::Markdown),
            ),
            (
                "toml",
                "# the manifest\n[package]\nname = \"ada-md\"\nversion = \"0.1.0\"\n\n[dependencies]\nregex = \"1\"\nfeatures = [\n  \"std\",\n]\n",
                lang("TOML"),
            ),
            ("env", "HOME=/home/ada\nEDITOR=nvim\n", lang("DotENV")),
            (
                "journalctl",
                "Oct 18 09:12:01 birb systemd[1]: Started ada.service.\nOct 18 09:12:02 birb ada[812]: listening\n",
                log,
            ),
            (
                "tracing",
                "2026-10-18T09:12:01.123Z  INFO ada: up\n2026-10-18T09:12:02.456Z  WARN ada: slow [see](x)\n  at frame 1\n",
                log,
            ),
            (
                "levels",
                "INFO starting\nWARN no config\nERROR gave up\n",
                log,
            ),
            (
                "mvn test compile",
                "[INFO] Scanning for projects...\n[INFO] \n[INFO] --------------< magnetar:ada >--------------\n[INFO] Building ada 1.0-SNAPSHOT\n[WARNING] /src/A.java:[12,5] deprecated\n[ERROR] Tests run: 3, Failures: 1, Errors: 0, Skipped: 0\n[INFO] BUILD FAILURE\n",
                log,
            ),
            (
                "readme",
                "# ada\n\nA terminal markdown viewer.\n",
                Some(FileType::Markdown),
            ),
            (
                "note",
                "Feeding\n=======\n\nAda eats at 7, 12 and 19.\n",
                Some(FileType::Markdown),
            ),
            (
                "list",
                "- kibble\n- water\n- the vet on Monday\n",
                Some(FileType::Markdown),
            ),
            (
                "table",
                "| meal | at |\n|---|---|\n| one | 7 |\n",
                Some(FileType::Markdown),
            ),
            (
                "fence",
                "Run it:\n```sh\nada -\n```\n",
                Some(FileType::Markdown),
            ),
            ("prose", "Ada is a cat.\nShe eats a lot.\n", None),
            (
                "cargo",
                "error[E0425]: cannot find value `x` in this scope\n --> src/main.rs:2:5\n  |\n2 |     x\n  |     ^ not found\n",
                None,
            ),
            ("git branch", "* main\n  image-previewer\n", None),
            (
                "crontab",
                "# m h dom mon dow command\n0 5 * * 1 tar -zcf /var/backups/home.tgz /home/\n",
                lang("Crontab"),
            ),
            (
                "ls -l",
                "total 8\n-rw-r--r-- 1 ada ada 12 Oct 18 09:12 a.md\n",
                None,
            ),
        ];
        let wrong: Vec<_> = cases
            .iter()
            .filter(|(_, text, want)| sniff(text) != *want)
            .map(|(name, text, want)| format!("{name}: {:?}, not {want:?}", sniff(text)))
            .collect();
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    /// FEAT-020: a language of each family by its lines, and the misreadings the census showed
    /// (`examples/sniff_census.rs` measures them on the corpora).
    #[test]
    fn languages_by_their_lines() {
        let md = Some(FileType::Markdown);
        let cases: &[(&str, &str, Option<FileType>)] = &[
            (
                "rust",
                "use std::fs;\n\npub fn read(p: &str) -> Result<String, Error> {\n    let mut s = String::new();\n    Ok(s)\n}\n",
                lang("Rust"),
            ),
            (
                "go",
                "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tx := 1\n\tfmt.Println(x)\n}\n",
                lang("Go"),
            ),
            (
                "python",
                "import os\n\nclass A:\n    def f(self, x):\n        return None\n",
                lang("Python"),
            ),
            (
                "c",
                "#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\\n\");\n    return 0;\n}\n",
                lang("C"),
            ),
            (
                "c++",
                "#include <vector>\n\nint main() {\n    std::vector<int> v;\n    return 0;\n}\n",
                lang("C++"),
            ),
            (
                "objective-c, a message named class",
                "#import \"A.h\"\n\n@implementation A\n- (id)f { return [NSString class]; }\n@end\n",
                lang("Objective-C"),
            ),
            (
                "java",
                "package a.b;\n\nimport java.util.List;\n\npublic class A {\n    @Override\n    public String toString() { return \"\"; }\n}\n",
                lang("Java"),
            ),
            (
                "kotlin",
                "package a.b\n\nimport kotlin.math.max\n\nfun main() {\n    val x = listOf(1)\n}\n",
                lang("Kotlin"),
            ),
            (
                "gradle",
                "plugins {\n    id 'java'\n}\n\ndependencies {\n    implementation 'a:b:1'\n}\n",
                lang("Groovy"),
            ),
            (
                "groovy shebang",
                "#!/usr/bin/env groovy\nprintln 'hi'\n",
                lang("Groovy"),
            ),
            (
                "scala",
                "package a.b\n\nimport scala.collection.mutable\n\nobject A {\n  def f(x: Int): Int = x + 1\n}\n",
                lang("Scala"),
            ),
            (
                "swift",
                "import Foundation\n\nstruct A: Codable {\n    let name: String\n    func f() -> Int { 1 }\n}\n",
                lang("Swift"),
            ),
            (
                "javascript",
                "const fs = require('fs');\n\nfunction read(p) {\n  return fs.readFileSync(p);\n}\nmodule.exports = { read };\n",
                lang("JavaScript (Babel)"),
            ),
            (
                "typescript",
                "import { a } from './a';\n\nexport interface B {\n  name: string;\n}\nexport const c = (x: number): void => {};\n",
                lang("TypeScript"),
            ),
            (
                "css",
                ".a {\n  color: red;\n  margin-top: 4px;\n}\n\nbody {\n  font-size: 12px;\n}\n",
                lang("CSS"),
            ),
            (
                "scss",
                "$c: red;\n\n.a {\n  color: $c;\n  @include b;\n}\n",
                lang("SCSS"),
            ),
            (
                "sql",
                "CREATE TABLE a (\n  id INTEGER PRIMARY KEY,\n  name VARCHAR(20) NOT NULL\n);\n",
                lang("SQL"),
            ),
            (
                "lua's --- comments aren't yaml nor a diff",
                "--- the module\n-- @module a\nlocal M = {}\n\nlocal function f(x)\n  if x ~= nil then\n    return x\n  end\nend\n\nreturn M\n",
                lang("Lua"),
            ),
            (
                "terraform",
                "resource \"aws_s3_bucket\" \"a\" {\n  bucket = \"b\"\n}\n",
                lang("Terraform"),
            ),
            (
                "racket",
                "#lang racket/base\n(define (f x) x)\n",
                lang("Racket"),
            ),
            (
                "elixir",
                "defmodule A do\n  def f(x), do: x\nend\n",
                lang("Elixir"),
            ),
            (
                "haskell",
                "module Main where\n\nimport qualified Data.Map as M\n\nmain :: IO ()\nmain = pure ()\n",
                lang("Haskell"),
            ),
            (
                "known_hosts",
                "github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl\n",
                lang("Known Hosts"),
            ),
            (
                "pkg-config",
                "prefix=/usr\nlibdir=${prefix}/lib\n\nName: a\nDescription: b\nVersion: 1.0\nLibs: -L${libdir} -la\n",
                lang("pkg-config"),
            ),
            (
                "ssh config",
                "Host a\n    HostName a.example\n    User ada\n    IdentityFile ~/.ssh/id\n",
                lang("SSH Config"),
            ),
            (
                "ini",
                "; settings\n[core]\nname = ada\npath = /usr/bin\n",
                lang("INI"),
            ),
            (
                "gitignore",
                "# build output\ntarget/\n*.o\n/dist\n",
                lang("Git Ignore"),
            ),
            (
                "a crontab's commented header",
                "# Edit this file to introduce tasks to be run by cron.\n# m h  dom mon dow   command\n",
                lang("Crontab"),
            ),
            (
                "readme with a code fence",
                "Some notes.\n\n```python\nimport os\ndef f():\n    pass\n```\n",
                md,
            ),
            (
                "a man page's -f isn't powershell",
                "grep [OPTION]... -f PATTERN_FILE [FILE]...\n    Select only those lines containing matches.\n",
                None,
            ),
            (
                "irc nicks aren't jsx",
                "<Meuuh> intf: added --vout\n<Sam> ok\n<Meuuh> thanks\n",
                None,
            ),
        ];
        let wrong: Vec<_> = cases
            .iter()
            .filter(|(_, text, want)| sniff(text) != *want)
            .map(|(name, text, want)| format!("{name}: {:?}, not {want:?}", sniff(text)))
            .collect();
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    #[test]
    fn only_the_sample_is_read() {
        let mut text = "# ada\n\nA viewer.\n".to_owned();
        let head = sample(&text).len();
        assert_eq!(head, text.len());
        text = "x".repeat(SAMPLE - 2) + "\nyy\n";
        assert_eq!(sample(&text), "x".repeat(SAMPLE - 2));
        // a multi-byte character across the cut, and no newline before it
        text = "x".repeat(SAMPLE - 1) + "é tail";
        assert_eq!(sample(&text), "x".repeat(SAMPLE - 1));
    }
}
