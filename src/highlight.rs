//! Syntax highlighting (ADR-07): syntect with two-face's syntax set, run on a worker thread so
//! the first paint never waits for it. The regexes run on Oniguruma by default; building without
//! the `onig` feature keeps them on fancy-regex, pure Rust.
//!
//! Results are colour-free. The theme's `[syntax]` table becomes a syntect theme whose rule `i`
//! "colours" a token with `i` encoded in its foreground, so a highlighted line is a list of
//! `(end byte, rule)` runs, and paint resolves each rule against the ada theme (ADR-03).
//! Results are cached by [`key`], a hash of the language and the code.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::str::FromStr;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, OnceLock};
use std::thread;

use ratatui::style::Style;
use syntect::easy::HighlightLines;
use syntect::highlighting::{
    Color, ScopeSelectors, StyleModifier, Theme as SynTheme, ThemeItem, ThemeSettings,
};
use syntect::parsing::{SyntaxReference, SyntaxSet};

use crate::app::{Computed, Msg};
use crate::doc::{CodeBlock, CodeId, code_lines};
use crate::layout::expand_tabs;

/// Longer lines are left plain: backtracking regexes can be slow on them, and nobody reads them
/// anyway.
const MAX_LINE: usize = 4096;

/// Cached blocks before the cache starts over.
const CACHE_CAP: usize = 4096;

/// bat's syntax set, loaded once. Loading is ~1 ms; syntect compiles each syntax's regexes on
/// first use, which is the expensive part: ~1 ms a language on Oniguruma, up to ~200 ms
/// (TypeScript) on fancy-regex.
pub fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(two_face::syntax::extra_newlines)
}

/// Languages that are deliberately plain.
fn is_plain(lang: &str) -> bool {
    ["text", "txt", "plain", "plaintext", "none"]
        .iter()
        .any(|p| lang.eq_ignore_ascii_case(p))
}

/// The syntax for a fence language, with the aliases the vault needs.
pub fn find_syntax(lang: &str) -> Option<&'static SyntaxReference> {
    if is_plain(lang) {
        return None;
    }
    let lower = lang.to_ascii_lowercase();
    let token = match lower.as_str() {
        "zsh" | "sh" | "shell" | "console" | "shellsession" => "bash",
        "kt" | "kts" => "kotlin",
        "ts" | "mts" | "cts" => "typescript",
        "jsonl" | "jsonc" | "json5" => "json",
        other => other,
    };
    // a syntax's own name, as a whole file's language is given (`Rust`, `TOML`)
    syntaxes()
        .find_syntax_by_name(lang)
        .or_else(|| syntaxes().find_syntax_by_token(token))
}

/// The language of a whole file, by its name alone: the name itself (`Makefile`, `.zshrc`,
/// `Cargo.lock`), then its extension. `None` when that says nothing, or says plain text.
pub fn file_language(path: &Path) -> Option<&'static str> {
    let set = syntaxes();
    let by_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| set.find_syntax_by_extension(n));
    let by_ext = || find_syntax(path.extension()?.to_str()?);
    by_name.or_else(by_ext).and_then(language)
}

/// The language a file's first line declares: a `#!` line, `<?xml`, a vim modeline.
pub fn first_line_language(line: &str) -> Option<&'static str> {
    syntaxes()
        .find_syntax_by_first_line(line)
        .and_then(language)
}

fn language(syntax: &'static SyntaxReference) -> Option<&'static str> {
    Some(&*syntax.name).filter(|&n| n != "Plain Text")
}

/// The cache key of a code block, or `None` when it has no language worth highlighting.
pub fn key(code: &CodeBlock) -> Option<u64> {
    let lang = code.lang.as_deref().filter(|l| !is_plain(l))?;
    let mut h = DefaultHasher::new();
    lang.hash(&mut h);
    code.hash.hash(&mut h);
    Some(h.finish())
}

/// The theme's `[syntax]` rules as a syntect theme. Rule `i` sets the foreground to an encoding
/// of `i`; text no rule matches keeps the transparent default.
#[derive(Debug, PartialEq)]
pub struct SyntaxTheme {
    theme: SynTheme,
    /// A hash of the rules' scope selectors: spans from another set of rules mean other rules.
    pub id: u64,
}

impl Eq for SyntaxTheme {}

impl SyntaxTheme {
    pub fn new(rules: &[(String, Style)]) -> Self {
        let mut h = DefaultHasher::new();
        for (scope, _) in rules {
            scope.hash(&mut h);
        }
        let scopes = rules
            .iter()
            .enumerate()
            .take(usize::from(Rule::NONE))
            .filter_map(|(i, (scope, _))| {
                let scope = ScopeSelectors::from_str(scope).ok()?;
                let foreground = Some(Color {
                    r: (i >> 8) as u8,
                    g: i as u8,
                    b: 0,
                    a: 0xff,
                });
                Some(ThemeItem {
                    scope,
                    style: StyleModifier {
                        foreground,
                        background: None,
                        font_style: None,
                    },
                })
            })
            .collect();
        let settings = ThemeSettings {
            foreground: Some(Color {
                r: 0,
                g: 0,
                b: 0,
                a: 0,
            }),
            ..ThemeSettings::default()
        };
        Self {
            theme: SynTheme {
                settings,
                scopes,
                ..SynTheme::default()
            },
            id: h.finish(),
        }
    }
}

/// An index into the theme's `[syntax]` rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule(pub u16);

impl Rule {
    pub const NONE: u16 = u16::MAX;

    fn decode(c: Color) -> Self {
        match c.a {
            0 => Rule(Self::NONE),
            _ => Rule(u16::from(c.r) << 8 | u16::from(c.g)),
        }
    }

    pub fn style(self, rules: &[(String, Style)]) -> Option<Style> {
        rules.get(usize::from(self.0)).map(|(_, s)| *s)
    }
}

/// A highlighted run: bytes up to `end` of the tab-expanded line take `rule`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub end: u32,
    pub rule: Rule,
}

/// The highlighting of one code block. Empty when the language isn't known.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CodeSpans {
    pub key: u64,
    /// The [`SyntaxTheme::id`] the rules come from.
    pub rules: u64,
    lines: Vec<Box<[Run]>>,
}

impl CodeSpans {
    /// Line `n`'s runs; empty when the line is plain.
    pub fn line(&self, n: u32) -> &[Run] {
        self.lines.get(n as usize).map_or(&[], |l| l)
    }
}

/// Highlight a code block's text. Deterministic, so replays recompute it (ADR-19).
pub fn highlight(key: u64, lang: &str, text: &str, theme: &SyntaxTheme) -> CodeSpans {
    let plain = CodeSpans {
        key,
        rules: theme.id,
        lines: Vec::new(),
    };
    let Some(syntax) = find_syntax(lang) else {
        return plain;
    };
    let mut h = HighlightLines::new(syntax, &theme.theme);
    let mut lines = Vec::new();
    let mut buf = String::new();
    for line in code_lines(text) {
        let line = expand_tabs(line);
        if line.len() > MAX_LINE {
            lines.push(Box::default());
            continue;
        }
        buf.clear();
        buf.push_str(&line);
        buf.push('\n');
        let Ok(regions) = h.highlight_line(&buf, syntaxes()) else {
            return plain;
        };
        let mut runs: Vec<Run> = Vec::new();
        let mut end = 0;
        for (style, piece) in regions {
            let start = end;
            end = (end + piece.len()).min(line.len());
            if end == start {
                continue;
            }
            let rule = Rule::decode(style.foreground);
            match runs.last_mut() {
                Some(last) if last.rule == rule => last.end = end as u32,
                _ => runs.push(Run {
                    end: end as u32,
                    rule,
                }),
            }
        }
        lines.push(runs.into_boxed_slice());
    }
    CodeSpans {
        key,
        rules: theme.id,
        lines,
    }
}

/// Highlighting results, and the blocks waiting for them.
#[derive(Debug, Default)]
pub struct Highlights {
    done: HashMap<u64, Arc<CodeSpans>>,
    pending: HashSet<u64>,
    /// Pending blocks already re-requested ahead of the queue.
    urged: HashSet<u64>,
}

impl Highlights {
    pub fn get(&self, key: u64) -> Option<&CodeSpans> {
        self.done.get(&key).map(|s| &**s)
    }

    pub fn is_pending(&self, key: u64) -> bool {
        self.pending.contains(&key)
    }

    /// Whether `key` still needs a job.
    pub fn wants(&self, key: u64) -> bool {
        !self.done.contains_key(&key) && !self.pending.contains(&key)
    }

    pub fn request(&mut self, key: u64) {
        self.pending.insert(key);
    }

    /// Mark a pending block as moved to the front of the queue; false if it already was.
    pub fn urge(&mut self, key: u64) -> bool {
        self.pending.contains(&key) && self.urged.insert(key)
    }

    pub fn insert(&mut self, spans: Arc<CodeSpans>) {
        self.pending.remove(&spans.key);
        self.urged.remove(&spans.key);
        if self.done.len() >= CACHE_CAP && !self.done.contains_key(&spans.key) {
            self.done.clear();
        }
        self.done.insert(spans.key, spans);
    }

    /// Highlight every block of a document, synchronously (print mode).
    pub fn compute_all(blocks: &[&CodeBlock], theme: &SyntaxTheme) -> Self {
        let mut out = Self::default();
        for code in blocks {
            let (Some(key), Some(lang)) = (key(code), code.lang.as_deref()) else {
                continue;
            };
            if let Entry::Vacant(slot) = out.done.entry(key) {
                slot.insert(Arc::new(highlight(key, lang, &code.text, theme)));
            }
        }
        out
    }
}

/// One block for the worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub doc_gen: u64,
    pub block: CodeId,
    pub key: u64,
    pub lang: Box<str>,
    pub text: Arc<str>,
    pub theme: Arc<SyntaxTheme>,
    /// Visible blocks go to the front of the queue.
    pub urgent: bool,
}

impl Job {
    pub fn run(&self) -> Msg {
        Msg::Highlighted {
            doc_gen: self.doc_gen,
            block: self.block,
            spans: Computed(Some(Arc::new(highlight(
                self.key,
                &self.lang,
                &self.text,
                &self.theme,
            )))),
        }
    }
}

/// Start the highlight worker. It loads the syntax set right away, off the UI thread, then
/// works through its queue, urgent jobs first, posting each result to `tx`.
pub fn spawn_worker(tx: Sender<Msg>) -> Sender<Job> {
    let (jobs_tx, jobs) = mpsc::channel::<Job>();
    thread::spawn(move || {
        syntaxes();
        let mut queue: VecDeque<Job> = VecDeque::new();
        loop {
            let mut batch = Vec::new();
            if queue.is_empty() {
                let Ok(job) = jobs.recv() else {
                    return;
                };
                batch.push(job);
            }
            batch.extend(jobs.try_iter());
            enqueue(&mut queue, batch);
            let Some(job) = queue.pop_front() else {
                continue;
            };
            if tx.send(job.run()).is_err() {
                return;
            }
        }
    });
    jobs_tx
}

/// Queue a batch of jobs: urgent ones go to the front, in their batch order, and replace any
/// queued copies; the rest go to the back, unless already queued.
fn enqueue(queue: &mut VecDeque<Job>, batch: Vec<Job>) {
    let (urgent, rest): (Vec<_>, Vec<_>) = batch.into_iter().partition(|j| j.urgent);
    for job in rest {
        if !queue.iter().any(|j| j.key == job.key) {
            queue.push_back(job);
        }
    }
    for job in urgent.into_iter().rev() {
        queue.retain(|j| j.key != job.key);
        queue.push_front(job);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    #[test]
    fn vault_languages_resolve() {
        for lang in [
            "typescript",
            "ts",
            "sh",
            "zsh",
            "bash",
            "kotlin",
            "kt",
            "rust",
            "json",
            "jsonl",
            "java",
            "toml",
            "python",
            "ini",
            "TypeScript",
        ] {
            assert!(find_syntax(lang).is_some(), "{lang}");
        }
        for lang in ["text", "TXT", "mermaid", "ebnf"] {
            assert!(find_syntax(lang).is_none(), "{lang}");
        }
    }

    fn rule_names(theme: &Theme, spans: &CodeSpans, line: &str, n: u32) -> Vec<(String, String)> {
        let mut start = 0;
        spans
            .line(n)
            .iter()
            .map(|r| {
                let text = line[start..r.end as usize].to_owned();
                start = r.end as usize;
                let rule = theme
                    .syntax
                    .get(usize::from(r.rule.0))
                    .map_or("-".to_owned(), |(s, _)| s.clone());
                (text, rule)
            })
            .collect()
    }

    #[test]
    fn runs_name_the_theme_rules() {
        let theme = Theme::builtin();
        let st = SyntaxTheme::new(&theme.syntax);
        let text = "fn main() {\n\tlet s = \"hi\"; // greet\n}\n";
        let spans = highlight(1, "rust", text, &st);
        let line = expand_tabs("\tlet s = \"hi\"; // greet");
        let runs = rule_names(&theme, &spans, &line, 1);
        let find = |t: &str| runs.iter().find(|(x, _)| x.contains(t)).map(|(_, r)| &**r);
        assert_eq!(find("hi"), Some("string"));
        assert_eq!(find("greet"), Some("comment"));
        assert_eq!(find("//"), Some("punctuation.definition.comment"));
        assert_eq!(find("let"), Some("storage.type"));
        // runs cover the line exactly, with no newline
        assert_eq!(
            spans.line(1).last().map(|r| r.end as usize),
            Some(line.len())
        );
        assert!(
            line.starts_with("    let"),
            "tabs are expanded before highlighting"
        );
        assert_eq!(spans.line(3), &[]);
    }

    #[test]
    fn unknown_languages_are_plain() {
        let st = SyntaxTheme::new(&Theme::builtin().syntax);
        let spans = highlight(7, "mermaid", "graph TD\n", &st);
        assert_eq!(spans.key, 7);
        assert_eq!(spans.line(0), &[]);
    }

    #[test]
    fn keys_depend_on_language_and_content() {
        let doc = crate::doc::parse(
            "```rust\nx\n```\n\n```text\nx\n```\n\n```ts\nx\n```\n\n```\nx\n```\n",
        );
        let blocks = doc.code_blocks();
        let keys: Vec<_> = blocks.iter().map(|c| key(c)).collect();
        assert!(keys[0].is_some() && keys[2].is_some());
        assert_ne!(keys[0], keys[2]);
        assert_eq!((keys[1], keys[3]), (None, None));
    }

    #[test]
    fn urgent_jobs_jump_the_queue() {
        let theme = Arc::new(SyntaxTheme::new(&[]));
        let job = |key, urgent| Job {
            doc_gen: 1,
            block: key as u32,
            key,
            lang: "rust".into(),
            text: "".into(),
            theme: theme.clone(),
            urgent,
        };
        let mut q = VecDeque::new();
        enqueue(&mut q, (0..4).map(|k| job(k, false)).collect());
        enqueue(&mut q, vec![job(1, false)]);
        enqueue(&mut q, vec![job(3, true), job(1, true), job(5, false)]);
        let keys: Vec<_> = q.iter().map(|j| j.key).collect();
        assert_eq!(keys, [3, 1, 0, 2, 5]);
    }
}
