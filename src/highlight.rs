//! Syntax highlighting (ADR-07): syntect with two-face's syntax set, run on a worker thread so
//! the first paint never waits for it. The regexes run on Oniguruma by default; building without
//! the `onig` feature keeps them on fancy-regex, pure Rust.
//!
//! Results are color-free. The theme's `[syntax]` table becomes a syntect theme whose rule `i`
//! "colors" a token with `i` encoded in its foreground, so a highlighted line is a list of
//! `(end byte, rule)` runs, and paint resolves each rule against the ada theme (ADR-03).
//! Results are cached by [`key`], a hash of the language and the code.

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread;
use std::time::Duration;

use ratatui::style::Style;
use syntect::highlighting::{
    Color, HighlightIterator, HighlightState, Highlighter as SynHighlighter, ScopeSelectors,
    StyleModifier, Theme as SynTheme, ThemeItem, ThemeSettings,
};
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};

use crate::app::{Computed, Msg};
use crate::doc::{CodeBlock, CodeId};
use crate::json;
use crate::layout::expand_tabs;
use crate::logs::{self, Kind, Span};

/// Longer lines are left plain: backtracking regexes can be slow on them, and nobody reads them
/// anyway.
const MAX_LINE: usize = 4096;

/// Cached blocks before the cache starts over.
const CACHE_CAP: usize = 4096;

/// Lines highlighted so far, by every thread. When it stands still for [`STALL`] while a block
/// is being highlighted, syntect is stuck on a line: its parser can loop forever on a
/// non-consuming push (syntect#656), as on the Scala line `:s| < @@@` (F-06).
static LINES: AtomicU64 = AtomicU64::new(0);

/// How long syntect may take on one line before its block is given up as plain. Real lines take
/// microseconds, and the slowest grammar found so far (O-37) about a second.
const STALL: Duration = Duration::from_millis(if cfg!(test) { 300 } else { 2000 });

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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeSpans {
    pub key: u64,
    /// The [`SyntaxTheme::id`] the rules come from.
    pub rules: u64,
    /// Only the first lines, while the worker is still on the rest (O-05).
    pub partial: bool,
    lines: Vec<Box<[Run]>>,
}

impl CodeSpans {
    /// Line `n`'s runs; empty when the line is plain.
    pub fn line(&self, n: u32) -> &[Run] {
        self.lines.get(n as usize).map_or(&[], |l| l)
    }

    /// How many lines are highlighted.
    pub fn line_count(&self) -> u32 {
        self.lines.len() as u32
    }

    /// A block left plain, as a whole.
    fn plain(key: u64, rules: u64) -> Self {
        Self {
            key,
            rules,
            ..Self::default()
        }
    }
}

/// Wait on `rx` for as long as syntect gets somewhere: `None` once no line was highlighted for
/// [`STALL`], or the sender is gone.
fn recv_watched<T>(rx: &Receiver<T>) -> Option<T> {
    let mut seen = LINES.load(Ordering::Relaxed);
    loop {
        match rx.recv_timeout(STALL) {
            Ok(t) => return Some(t),
            Err(RecvTimeoutError::Disconnected) => return None,
            Err(RecvTimeoutError::Timeout) => {
                let now = LINES.load(Ordering::Relaxed);
                if now == seen {
                    return None;
                }
                seen = now;
            }
        }
    }
}

/// Highlight a code block's text. Deterministic, so replays recompute it (ADR-19).
pub fn highlight(key: u64, lang: &str, text: &str, theme: &SyntaxTheme) -> CodeSpans {
    let mut h = Highlighter::new(key, lang, text, theme);
    while h.step(usize::MAX) {}
    h.finish()
}

/// Highlight only a code block's first `lines` lines, as a partial result did (O-05).
pub fn highlight_first(
    key: u64,
    lang: &str,
    text: &str,
    theme: &SyntaxTheme,
    lines: u32,
) -> CodeSpans {
    let mut h = Highlighter::new(key, lang, text, theme);
    h.step(lines as usize);
    h.partial()
}

/// Where syntect is between two lines: what the next line's highlighting depends on, besides
/// the line.
struct LineState {
    parse: ParseState,
    scopes: HighlightState,
}

impl LineState {
    fn new(syntax: &SyntaxReference, highlighter: &SynHighlighter) -> Self {
        Self {
            parse: ParseState::new(syntax),
            scopes: HighlightState::new(highlighter, ScopeStack::new()),
        }
    }

    /// Highlight one line, moving to the state after it. `None` when syntect fails.
    fn line(
        &mut self,
        line: &str,
        highlighter: &SynHighlighter,
        buf: &mut String,
    ) -> Option<Box<[Run]>> {
        let line = expand_tabs(line);
        if line.len() > MAX_LINE {
            return Some(Box::default());
        }
        buf.clear();
        buf.push_str(&line);
        buf.push('\n');
        let ops = self.parse.parse_line(buf, syntaxes()).ok()?;
        LINES.fetch_add(1, Ordering::Relaxed);
        let mut runs: Vec<Run> = Vec::new();
        let mut end = 0;
        for (style, piece) in HighlightIterator::new(&mut self.scopes, &ops, buf, highlighter) {
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
        Some(runs.into_boxed_slice())
    }
}

/// Whether a language is bat's `Log`, which ada highlights with its own parser (O-43).
fn is_log(lang: &str) -> bool {
    find_syntax(lang).is_some_and(|s| s.name == "log")
}

/// Whether a language is bat's `JSON`, which ada highlights with its own scanner (O-48).
fn is_json(lang: &str) -> bool {
    find_syntax(lang).is_some_and(|s| s.name == "JSON")
}

/// The theme's rule for each of a lexer's kinds, as syntect would pick it for the kind's
/// scopes under `base`.
fn kind_rules<const N: usize>(theme: &SyntaxTheme, base: &str, scopes: [&str; N]) -> [Rule; N] {
    let highlighter = SynHighlighter::new(&theme.theme);
    let base = Scope::new(base).expect("scope");
    scopes.map(|scopes| {
        let mut stack = vec![base];
        stack.extend(scopes.split(' ').map(|s| Scope::new(s).expect("scope")));
        highlighter
            .style_mod_for_stack(&stack)
            .foreground
            .map_or(Rule(Rule::NONE), Rule::decode)
    })
}

/// A line's runs from its spans, `(start, end, rule)` in order; empty when no span has a rule.
fn span_runs(len: usize, spans: impl Iterator<Item = (u32, u32, Rule)>) -> Box<[Run]> {
    fn push(runs: &mut Vec<Run>, end: u32, rule: Rule) {
        match runs.last_mut() {
            Some(last) if last.rule == rule => last.end = end,
            _ => runs.push(Run { end, rule }),
        }
    }
    let mut runs: Vec<Run> = Vec::new();
    let mut at = 0;
    for (start, end, rule) in spans {
        if start > at {
            push(&mut runs, start, Rule(Rule::NONE));
        }
        push(&mut runs, end, rule);
        at = end;
    }
    if runs.iter().all(|r| r.rule.0 == Rule::NONE) {
        return Box::default();
    }
    if (at as usize) < len {
        push(&mut runs, len as u32, Rule(Rule::NONE));
    }
    runs.into_boxed_slice()
}

/// The theme's rule for each log [`Kind`].
struct LogRules([Rule; Kind::ALL.len()]);

impl LogRules {
    fn new(theme: &SyntaxTheme) -> Self {
        Self(kind_rules(theme, "text.log", Kind::ALL.map(Kind::scopes)))
    }

    /// Highlight one log line; `spans` is scratch space.
    fn line(&self, line: &str, spans: &mut Vec<Span>) -> Box<[Run]> {
        let line = expand_tabs(line);
        logs::tokens(&line, spans);
        let rule = |s: &Span| (s.start, s.end, self.0[s.kind as usize]);
        span_runs(line.len(), spans.iter().map(rule))
    }
}

/// The theme's rule for each JSON [`json::Kind`].
struct JsonRules([Rule; json::Kind::ALL.len()]);

impl JsonRules {
    fn new(theme: &SyntaxTheme) -> Self {
        let scopes = json::Kind::ALL.map(json::Kind::scopes);
        Self(kind_rules(theme, "source.json", scopes))
    }

    /// Highlight one JSON line, after one that left a `/* */` comment open or not; `spans` is
    /// scratch space.
    fn line(&self, line: &str, comment: &mut bool, spans: &mut Vec<json::Span>) -> Box<[Run]> {
        let line = expand_tabs(line);
        json::tokens(&line, comment, spans);
        let rule = |s: &json::Span| (s.start, s.end, self.0[s.kind as usize]);
        span_runs(line.len(), spans.iter().map(rule))
    }
}

/// What highlights a block's lines.
enum Engine {
    Syntect(LineState),
    Log(LogRules, Vec<Span>),
    /// Whether a `/* */` comment is open.
    Json(JsonRules, Vec<json::Span>, bool),
}

/// The lines of a code block, split as [`crate::doc::code_lines`] splits them.
#[derive(Clone, Copy)]
struct Lines<'a>(Option<&'a str>);

impl<'a> Lines<'a> {
    fn new(text: &'a str) -> Self {
        Self(Some(text.strip_suffix('\n').unwrap_or(text)).filter(|t| !t.is_empty()))
    }
}

impl<'a> Iterator for Lines<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        let rest = self.0?;
        let (line, tail) = match rest.split_once('\n') {
            Some((line, tail)) => (line, Some(tail)),
            None => (rest, None),
        };
        self.0 = tail;
        Some(line.strip_suffix('\r').unwrap_or(line))
    }
}

/// Highlights a block a few lines at a time, carrying syntect's state from one step to the
/// next, so the worker can post the first lines early and stop between steps (O-05).
pub struct Highlighter<'a> {
    key: u64,
    rules: u64,
    highlighter: SynHighlighter<'a>,
    /// `None` once the block turns out plain, or syntect fails on it.
    state: Option<Engine>,
    rest: Lines<'a>,
    lines: Vec<Box<[Run]>>,
    buf: String,
}

impl<'a> Highlighter<'a> {
    pub fn new(key: u64, lang: &str, text: &'a str, theme: &'a SyntaxTheme) -> Self {
        let highlighter = SynHighlighter::new(&theme.theme);
        let state = if is_log(lang) {
            Some(Engine::Log(LogRules::new(theme), Vec::new()))
        } else if is_json(lang) {
            Some(Engine::Json(JsonRules::new(theme), Vec::new(), false))
        } else {
            find_syntax(lang).map(|s| Engine::Syntect(LineState::new(s, &highlighter)))
        };
        Self {
            key,
            rules: theme.id,
            state,
            highlighter,
            rest: Lines::new(text),
            lines: Vec::new(),
            buf: String::new(),
        }
    }

    /// Highlight up to `n` more lines. False once the block is done.
    pub fn step(&mut self, n: usize) -> bool {
        for _ in 0..n {
            let Some(state) = &mut self.state else {
                return false;
            };
            let Some(line) = self.rest.next() else {
                return false;
            };
            let runs = match state {
                Engine::Syntect(state) => state.line(line, &self.highlighter, &mut self.buf),
                Engine::Log(rules, spans) => {
                    LINES.fetch_add(1, Ordering::Relaxed);
                    Some(rules.line(line, spans))
                }
                Engine::Json(rules, spans, comment) => {
                    LINES.fetch_add(1, Ordering::Relaxed);
                    Some(rules.line(line, comment, spans))
                }
            };
            match runs {
                Some(runs) => self.lines.push(runs),
                None => {
                    // plain, as a whole
                    self.state = None;
                    self.lines.clear();
                    return false;
                }
            }
        }
        self.state.is_some() && self.rest.0.is_some()
    }

    /// How many lines are done.
    pub fn line_count(&self) -> u32 {
        self.lines.len() as u32
    }

    /// The lines done so far, as a partial result.
    pub fn partial(&self) -> CodeSpans {
        CodeSpans {
            key: self.key,
            rules: self.rules,
            partial: true,
            lines: self.lines.clone(),
        }
    }

    pub fn finish(self) -> CodeSpans {
        CodeSpans {
            key: self.key,
            rules: self.rules,
            partial: false,
            lines: self.lines,
        }
    }
}

/// Blocks this big are highlighted on several threads in print mode (O-04).
const PARALLEL_BYTES: usize = 1 << 20;
const PARALLEL_LINES: usize = 20_000;

/// How often a chunk shows its state, to compare with the state the chunk before it really ends
/// in.
const CHECKPOINT_EVERY: usize = 32;

/// [`highlight`] for print mode (O-04): a block of 1 MiB or 20,000 lines and up is cut into one
/// chunk per thread, each highlighted at once from a fresh state, as if it began the block. The
/// thread with the true state at a chunk's end (the first one, to begin with) then highlights
/// on into the next chunk until its state matches one of that chunk's checkpoints. From there
/// on the chunk's guess is what one thread would have produced, since a line's highlighting
/// depends only on the line and the state before it, and that chunk's thread carries on. Most
/// syntaxes come back to the same state within a few lines; for one that never does, the first
/// thread does all the work. The result is [`highlight`]'s.
pub fn highlight_parallel(key: u64, lang: &str, text: &str, theme: &SyntaxTheme) -> CodeSpans {
    let Some(syntax) = find_syntax(lang) else {
        return highlight(key, lang, text, theme);
    };
    if text.len() < PARALLEL_BYTES && newlines(text) < PARALLEL_LINES {
        return highlight(key, lang, text, theme);
    }
    // not for every block: it reads the cgroup's CPU quota
    let threads = thread::available_parallelism().map_or(1, |n| n.get().min(8));
    if threads < 2 {
        return highlight(key, lang, text, theme);
    }
    match &*syntax.name {
        "log" => highlight_log_on(key, text, theme, threads),
        // a `/* */` comment carries over lines: rare in JSON this big, and fast on one thread
        "JSON" if text.contains("/*") => highlight(key, lang, text, theme),
        "JSON" => highlight_json_on(key, text, theme, threads),
        _ => highlight_on(key, syntax, text, theme, threads),
    }
}

/// A log on `threads` threads: its lines don't depend on each other.
fn highlight_log_on(key: u64, text: &str, theme: &SyntaxTheme, threads: usize) -> CodeSpans {
    let rules = LogRules::new(theme);
    lines_on(key, text, theme, threads, |line, spans| {
        rules.line(line, spans)
    })
}

/// JSON with no `/* */` comment on `threads` threads: its lines don't depend on each other.
fn highlight_json_on(key: u64, text: &str, theme: &SyntaxTheme, threads: usize) -> CodeSpans {
    let rules = JsonRules::new(theme);
    lines_on(key, text, theme, threads, |line, spans| {
        rules.line(line, &mut false, spans)
    })
}

/// A block whose lines don't depend on each other on `threads` threads, each taking a chunk of
/// them as it comes; `line` highlights one, with scratch space of its own.
fn lines_on<S: Default>(
    key: u64,
    text: &str,
    theme: &SyntaxTheme,
    threads: usize,
    line: impl Fn(&str, &mut S) -> Box<[Run]> + Sync,
) -> CodeSpans {
    let all: Vec<&str> = Lines::new(text).collect();
    let chunks: Vec<Vec<Box<[Run]>>> = thread::scope(|s| {
        let handles: Vec<_> = all
            .chunks(all.len().div_ceil(threads))
            .map(|chunk| {
                let line = &line;
                s.spawn(move || {
                    let mut scratch = S::default();
                    let mut lines = Vec::with_capacity(chunk.len());
                    // the watch in print mode sees progress (F-06)
                    for part in chunk.chunks(4096) {
                        lines.extend(part.iter().map(|l| line(l, &mut scratch)));
                        LINES.fetch_add(part.len() as u64, Ordering::Relaxed);
                    }
                    lines
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("highlight thread"))
            .collect()
    });
    CodeSpans {
        key,
        rules: theme.id,
        partial: false,
        lines: chunks.into_iter().flatten().collect(),
    }
}

/// [`highlight_parallel`] on `threads` threads, whatever the size.
fn highlight_on(
    key: u64,
    syntax: &SyntaxReference,
    text: &str,
    theme: &SyntaxTheme,
    threads: usize,
) -> CodeSpans {
    let all: Vec<&str> = Lines::new(text).collect();
    let chunks: Vec<&[&str]> = all.chunks(all.len().div_ceil(threads)).collect();
    let seams = Seams::new(chunks.len());
    let mut outs: Vec<Option<Chunk>> = thread::scope(|s| {
        let handles: Vec<_> = (0..chunks.len())
            .map(|k| {
                let (chunks, seams) = (&chunks, &seams);
                s.spawn(move || chunk(k, chunks, syntax, theme, seams))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("highlight thread"))
            .collect()
    });
    let mut lines = Vec::with_capacity(all.len());
    let mut k = 0;
    loop {
        let Some(out) = outs.get_mut(k).and_then(Option::take) else {
            // syntect failed on a line: plain, as a whole
            lines.clear();
            break;
        };
        lines.extend(out.lines.into_iter().skip(out.from));
        lines.extend(out.over);
        match out.meet {
            Some(m) => k = m,
            None => break,
        }
    }
    CodeSpans {
        key,
        rules: theme.id,
        partial: false,
        lines,
    }
}

/// What a chunk's thread found. Its lines from `from` on are true, then `over`, the lines it
/// highlighted after its chunk, run up to chunk `meet`'s checkpoint, or to the block's end.
struct Chunk {
    from: usize,
    lines: Vec<Box<[Run]>>,
    over: Vec<Box<[Run]>>,
    meet: Option<usize>,
}

/// A state's fingerprint, to compare it with another thread's: its `Debug` text, which shows
/// every field's value. Not for a state that holds a match's captures: an Oniguruma `Region`
/// shows its pointers, and states can't move between threads to be compared directly.
fn fingerprint(state: &LineState) -> Option<String> {
    let text = format!("{:?}{:?}", state.parse, state.scopes);
    (!text.contains("captures: Some")).then_some(text)
}

/// Where each chunk's thread learns from the one before it whether, and from which line, its
/// guess is true.
struct Seams {
    seams: Mutex<(Vec<Seam>, bool)>,
    changed: Condvar,
}

#[derive(Default)]
struct Seam {
    /// The fingerprints before every `CHECKPOINT_EVERY`th line, once guessed; empty if syntect
    /// failed on the guess.
    checkpoints: Option<Vec<Option<String>>>,
    /// The first true line of the guess; `None` inside when the true state ran past the chunk.
    from: Option<Option<usize>>,
}

impl Seams {
    fn new(n: usize) -> Self {
        Self {
            seams: Mutex::new(((0..n).map(|_| Seam::default()).collect(), false)),
            changed: Condvar::new(),
        }
    }

    fn update(&self, f: impl FnOnce(&mut Vec<Seam>, &mut bool)) {
        let mut guard = self.seams.lock().unwrap_or_else(PoisonError::into_inner);
        let (seams, failed) = &mut *guard;
        f(seams, failed);
        self.changed.notify_all();
    }

    /// Wait until `get` has an answer, or the block fails.
    fn wait<T>(&self, mut get: impl FnMut(&[Seam]) -> Option<T>) -> Option<T> {
        let mut guard = self.seams.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if guard.1 {
                return None;
            }
            if let Some(answer) = get(&guard.0) {
                return Some(answer);
            }
            guard = self
                .changed
                .wait(guard)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn fail(&self) {
        self.update(|_, failed| *failed = true);
    }
}

/// Chunk `k`'s thread: guess the chunk, then, if the state at its end turns out true, carry it
/// on until it meets a later chunk's checkpoint. `None` when syntect fails on a true line.
fn chunk(
    k: usize,
    chunks: &[&[&str]],
    syntax: &SyntaxReference,
    theme: &SyntaxTheme,
    seams: &Seams,
) -> Option<Chunk> {
    let highlighter = SynHighlighter::new(&theme.theme);
    let mut state = LineState::new(syntax, &highlighter);
    let mut buf = String::new();
    let mut lines = Vec::with_capacity(chunks[k].len());
    let mut checkpoints = Vec::new();
    let mut failed = false;
    for (i, line) in chunks[k].iter().enumerate() {
        if i > 0 && i % CHECKPOINT_EVERY == 0 {
            checkpoints.push(fingerprint(&state));
        }
        let Some(runs) = state.line(line, &highlighter, &mut buf) else {
            failed = true;
            break;
        };
        lines.push(runs);
    }
    if failed {
        checkpoints.clear();
    }
    seams.update(|s, _| s[k].checkpoints = Some(checkpoints));
    let from = match k {
        0 => Some(0),
        _ => seams.wait(|s| s[k].from)?,
    };
    let Some(from) = from else {
        // the true state ran past this chunk: the guess isn't used
        return Some(Chunk {
            from: 0,
            lines: Vec::new(),
            over: Vec::new(),
            meet: None,
        });
    };
    if failed {
        // the failing line comes after the checkpoint the true state met, so it fails for true
        seams.fail();
        return None;
    }
    let mut over = Vec::new();
    for (m, chunk) in chunks.iter().enumerate().skip(k + 1) {
        let theirs = seams.wait(|s| s[m].checkpoints.clone())?;
        for (i, line) in chunk.iter().enumerate() {
            if i > 0 && i % CHECKPOINT_EVERY == 0 {
                let mine = fingerprint(&state);
                if mine.is_some() && theirs.get(i / CHECKPOINT_EVERY - 1) == Some(&mine) {
                    seams.update(|s, _| s[m].from = Some(Some(i)));
                    return Some(Chunk {
                        from,
                        lines,
                        over,
                        meet: Some(m),
                    });
                }
            }
            let Some(runs) = state.line(line, &highlighter, &mut buf) else {
                seams.fail();
                return None;
            };
            over.push(runs);
        }
        seams.update(|s, _| s[m].from = Some(None));
    }
    Some(Chunk {
        from,
        lines,
        over,
        meet: None,
    })
}

fn newlines(text: &str) -> usize {
    text.bytes().filter(|&b| b == b'\n').count()
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

    /// Whether `key` still needs a job: nothing, or only the first lines, came back for it, and
    /// none is under way.
    pub fn wants(&self, key: u64) -> bool {
        !self.pending.contains(&key) && self.done.get(&key).is_none_or(|s| s.partial)
    }

    /// Forget the jobs under way: the worker drops them when another document or theme comes
    /// along (O-05), so they're asked for again on the way back.
    pub fn forget_pending(&mut self) {
        self.pending.clear();
        self.urged.clear();
    }

    pub fn request(&mut self, key: u64) {
        self.pending.insert(key);
    }

    /// Mark a pending block as moved to the front of the queue; false if it already was.
    pub fn urge(&mut self, key: u64) -> bool {
        self.pending.contains(&key) && self.urged.insert(key)
    }

    pub fn insert(&mut self, spans: Arc<CodeSpans>) {
        if spans.partial {
            // the first lines, unless the whole block is already here
            if self.done.get(&spans.key).is_none_or(|s| s.partial) {
                self.done.insert(spans.key, spans);
            }
            return;
        }
        self.pending.remove(&spans.key);
        self.urged.remove(&spans.key);
        if self.done.len() >= CACHE_CAP && !self.done.contains_key(&spans.key) {
            self.done.clear();
        }
        self.done.insert(spans.key, spans);
    }

    /// Highlight every block of a document before returning (print mode). The blocks go on a
    /// thread of their own: a block syntect gets stuck on is left plain, and the thread left to
    /// it, while a new one takes the rest (F-06).
    pub fn compute_all(blocks: &[&CodeBlock], theme: &Arc<SyntaxTheme>) -> Self {
        let mut out = Self::default();
        let mut todo: Vec<(u64, Box<str>, Arc<str>)> = Vec::new();
        for code in blocks {
            let (Some(key), Some(lang)) = (key(code), &code.lang) else {
                continue;
            };
            if !todo.iter().any(|t| t.0 == key) {
                todo.push((key, lang.clone(), code.text.clone()));
            }
        }
        let todo = Arc::new(todo);
        let mut next = 0;
        while next < todo.len() {
            let (tx, rx) = mpsc::channel();
            let (blocks, rules) = (Arc::clone(&todo), Arc::clone(theme));
            thread::spawn(move || {
                for (key, lang, text) in &blocks[next..] {
                    if tx
                        .send(highlight_parallel(*key, lang, text, &rules))
                        .is_err()
                    {
                        return;
                    }
                }
            });
            while next < todo.len() {
                let Some(spans) = recv_watched(&rx) else {
                    break;
                };
                out.done.insert(spans.key, Arc::new(spans));
                next += 1;
            }
            if let Some((key, ..)) = todo.get(next) {
                out.done
                    .insert(*key, Arc::new(CodeSpans::plain(*key, theme.id)));
                next += 1;
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
    /// Only the first lines, as a replayed partial result had them (O-05). Replays run these
    /// inline; the worker never gets one.
    pub first: Option<u32>,
}

impl Job {
    pub fn run(&self) -> Msg {
        let spans = match self.first {
            Some(n) => highlight_first(self.key, &self.lang, &self.text, &self.theme, n),
            None => highlight(self.key, &self.lang, &self.text, &self.theme),
        };
        Msg::Highlighted {
            doc_gen: self.doc_gen,
            block: self.block,
            lines: self.first,
            spans: Computed(Some(Arc::new(spans))),
        }
    }

    /// The document and rules the job is for.
    fn epoch(&self) -> Epoch {
        Epoch {
            doc_gen: self.doc_gen,
            rules: self.theme.id,
        }
    }
}

/// A document generation and a set of `[syntax]` rules: the jobs worth finishing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Epoch {
    pub doc_gen: u64,
    pub rules: u64,
}

/// What the worker is asked.
#[derive(Debug)]
pub enum Request {
    Job(Job),
    /// Only jobs for this epoch are wanted from now on: drop the rest, and stop the one
    /// running if it isn't (O-05).
    Only(Epoch),
}

/// Lines highlighted between two looks at the queue.
const STEP: usize = 256;

/// Start the highlight worker. It loads the syntax set right away, off the UI thread, then
/// works through its queue, urgent jobs first, posting each result to `tx`. A block goes
/// `STEP` lines at a time: in between, the worker takes new requests, and stops the block when
/// another document or theme came along. A long block posts its first lines when 256, 512,
/// 1,024 … are done, so the top shows highlighted long before the rest (O-05).
///
/// A watcher thread stands by while a block is highlighted. When syntect gets stuck on a line
/// (F-06), it posts the block as plain and starts a new worker on the same queue; the stuck
/// thread is left to spin, since a thread can't be stopped from outside, and quits if it ever
/// gets free. While the worker is idle, so is the watcher.
pub fn spawn_worker(tx: Sender<Msg>) -> Sender<Request> {
    let (requests, rx) = mpsc::channel::<Request>();
    let shared = Arc::new(Worker {
        inbox: Mutex::new((rx, Queue::default())),
        busy: Mutex::new(Busy::default()),
        changed: Condvar::new(),
        tx,
    });
    let first = Arc::clone(&shared);
    thread::spawn(move || first.work(0));
    thread::spawn(move || shared.watch());
    requests
}

/// The worker's state, shared by the worker that runs and the watcher that stands in for it.
struct Worker {
    /// The requests and the queue, held only between two steps of a block.
    inbox: Mutex<(Receiver<Request>, Queue)>,
    busy: Mutex<Busy>,
    changed: Condvar,
    tx: Sender<Msg>,
}

/// What the worker is on.
#[derive(Default)]
struct Busy {
    /// The block being highlighted, by the worker of which generation, numbered by `started`.
    job: Option<(Job, u64)>,
    started: u64,
    /// The worker of this generation is the one that counts; an older one got stuck.
    generation: u64,
    /// The app is gone.
    done: bool,
}

impl Worker {
    fn busy(&self) -> MutexGuard<'_, Busy> {
        self.busy.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Mark the worker of generation `generation` on `job`, or done with it. False when that
    /// worker is no longer the one that counts.
    fn set_job(&self, generation: u64, job: Option<&Job>) -> bool {
        let mut busy = self.busy();
        if busy.generation != generation {
            return false;
        }
        busy.started += u64::from(job.is_some());
        busy.job = job.map(|j| (j.clone(), busy.started));
        self.changed.notify_all();
        true
    }

    fn post(&self, generation: u64, msg: Msg) -> bool {
        self.busy().generation == generation && self.tx.send(msg).is_ok()
    }

    /// The worker of generation `generation`.
    fn work(&self, generation: u64) {
        syntaxes();
        loop {
            let job = {
                let mut inbox = self.inbox.lock().unwrap_or_else(PoisonError::into_inner);
                let (rx, queue) = &mut *inbox;
                let mut batch = Vec::new();
                if queue.jobs.is_empty() {
                    let Ok(request) = rx.recv() else {
                        self.busy().done = true;
                        self.changed.notify_all();
                        return;
                    };
                    batch.push(request);
                }
                batch.extend(rx.try_iter());
                queue.add(batch);
                queue.jobs.pop_front()
            };
            let Some(job) = job else {
                continue;
            };
            if !self.set_job(generation, Some(&job)) {
                return;
            }
            let mut h = Highlighter::new(job.key, &job.lang, &job.text, &job.theme);
            let mut post_at = STEP as u32;
            let mut stopped = false;
            while h.step(STEP) {
                let mut inbox = self.inbox.lock().unwrap_or_else(PoisonError::into_inner);
                if self.busy().generation != generation {
                    return;
                }
                let (rx, queue) = &mut *inbox;
                queue.add(rx.try_iter().collect());
                if queue.epoch != Some(job.epoch()) {
                    stopped = true;
                    break;
                }
                drop(inbox);
                if h.line_count() >= post_at {
                    post_at = h.line_count() * 2;
                    let msg = Msg::Highlighted {
                        doc_gen: job.doc_gen,
                        block: job.block,
                        lines: Some(h.line_count()),
                        spans: Computed(Some(Arc::new(h.partial()))),
                    };
                    if !self.post(generation, msg) {
                        return;
                    }
                }
            }
            if !self.set_job(generation, None) {
                return;
            }
            if stopped {
                continue;
            }
            let msg = Msg::Highlighted {
                doc_gen: job.doc_gen,
                block: job.block,
                lines: None,
                spans: Computed(Some(Arc::new(h.finish()))),
            };
            if !self.post(generation, msg) {
                return;
            }
        }
    }

    /// Stand by while a block is highlighted: if no line gets done for [`STALL`], post the
    /// block as plain and start a worker of the next generation.
    fn watch(self: Arc<Self>) {
        let mut busy = self.busy();
        loop {
            busy = self
                .changed
                .wait_while(busy, |b| b.job.is_none() && !b.done)
                .unwrap_or_else(PoisonError::into_inner);
            if busy.done {
                return;
            }
            let started = busy.started;
            let seen = LINES.load(Ordering::Relaxed);
            let (guard, wait) = self
                .changed
                .wait_timeout_while(busy, STALL, |b| {
                    b.job.as_ref().is_some_and(|j| j.1 == started) && !b.done
                })
                .unwrap_or_else(PoisonError::into_inner);
            busy = guard;
            if !wait.timed_out() || LINES.load(Ordering::Relaxed) != seen {
                continue;
            }
            let Some((job, _)) = busy.job.take() else {
                continue;
            };
            busy.generation += 1;
            let generation = busy.generation;
            let msg = Msg::Highlighted {
                doc_gen: job.doc_gen,
                block: job.block,
                lines: None,
                spans: Computed(Some(Arc::new(CodeSpans::plain(job.key, job.theme.id)))),
            };
            if self.tx.send(msg).is_err() {
                return;
            }
            let next = Arc::clone(&self);
            thread::spawn(move || next.work(generation));
        }
    }
}

/// The worker's queue, and the epoch its jobs are for.
#[derive(Debug, Default)]
struct Queue {
    jobs: VecDeque<Job>,
    epoch: Option<Epoch>,
}

impl Queue {
    /// Take a batch of requests. A job for another epoch than the queue's starts a new one, as
    /// `Only` does, and drops the jobs queued for the old. Then urgent jobs go to the front, in
    /// their batch order, and replace any queued copies; the rest go to the back, unless
    /// already queued.
    fn add(&mut self, batch: Vec<Request>) {
        let mut jobs = Vec::new();
        for request in batch {
            let epoch = match &request {
                Request::Job(job) => job.epoch(),
                Request::Only(epoch) => *epoch,
            };
            if self.epoch != Some(epoch) {
                self.epoch = Some(epoch);
                self.jobs.retain(|j| j.epoch() == epoch);
                jobs.retain(|j: &Job| j.epoch() == epoch);
            }
            if let Request::Job(job) = request {
                jobs.push(job);
            }
        }
        let (urgent, rest): (Vec<_>, Vec<_>) = jobs.into_iter().partition(|j| j.urgent);
        for job in rest {
            if !self.jobs.iter().any(|j| j.key == job.key) {
                self.jobs.push_back(job);
            }
        }
        for job in urgent.into_iter().rev() {
            self.jobs.retain(|j| j.key != job.key);
            self.jobs.push_front(job);
        }
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

    /// On valid JSON, the scanner picks the rules bat's grammar picks, in every built-in theme
    /// (O-48).
    #[test]
    fn json_rules_match_the_grammar() {
        let text = concat!(
            "{\n",
            "  \"name\": \"ada\", \"tab\":\t\"a\\tb \\\"q\\\" \\u00e9 \\x\",\n",
            "  \"n\" : [0, -12, 1.5, -0.25e+3, 1E9, true, false, null],\n",
            "  \"o\": {\"k\": [], \"e\": {}}, // note\n",
            "  /* a comment\n",
            "     over lines */ \"k\\\"ey\": \"//\" /* inline */\n",
            "}\n",
        );
        let grammar = find_syntax("json").unwrap();
        for theme in Theme::builtins() {
            let theme = SyntaxTheme::new(&theme.syntax);
            let highlighter = SynHighlighter::new(&theme.theme);
            let mut state = LineState::new(grammar, &highlighter);
            let mut buf = String::new();
            let want: Vec<_> = Lines::new(text)
                .map(|l| state.line(l, &highlighter, &mut buf).unwrap())
                .collect();
            assert_eq!(highlight(1, "json", text, &theme).lines, want);
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

    fn job(theme: &Arc<SyntaxTheme>, doc_gen: u64, key: u64, urgent: bool) -> Job {
        Job {
            doc_gen,
            block: key as u32,
            key,
            lang: "rust".into(),
            text: "".into(),
            theme: theme.clone(),
            urgent,
            first: None,
        }
    }

    fn keys(q: &Queue) -> Vec<u64> {
        q.jobs.iter().map(|j| j.key).collect()
    }

    #[test]
    fn urgent_jobs_jump_the_queue() {
        let theme = Arc::new(SyntaxTheme::new(&[]));
        let job = |key, urgent| Request::Job(job(&theme, 1, key, urgent));
        let mut q = Queue::default();
        q.add((0..4).map(|k| job(k, false)).collect());
        q.add(vec![job(1, false)]);
        q.add(vec![job(3, true), job(1, true), job(5, false)]);
        assert_eq!(keys(&q), [3, 1, 0, 2, 5]);
    }

    /// Another document's or theme's jobs drop the queued ones (O-05).
    #[test]
    fn a_new_epoch_drops_the_old_jobs() {
        let theme = Arc::new(SyntaxTheme::new(&[]));
        let other = Arc::new(SyntaxTheme::new(&[("comment".to_owned(), Style::new())]));
        let mut q = Queue::default();
        q.add(
            (0..3)
                .map(|k| Request::Job(job(&theme, 1, k, false)))
                .collect(),
        );
        // the same document again, as a cached one comes back: kept
        q.add(vec![Request::Job(job(&theme, 1, 7, false))]);
        assert_eq!(keys(&q), [0, 1, 2, 7]);
        // another document with no code: only `Only` says so
        q.add(vec![Request::Only(Epoch {
            doc_gen: 2,
            rules: theme.id,
        })]);
        assert_eq!(keys(&q), Vec::<u64>::new());
        // a batch that switches mid-way keeps only the jobs after the switch
        q.add(vec![
            Request::Job(job(&theme, 2, 4, false)),
            Request::Job(job(&other, 2, 5, true)),
            Request::Job(job(&other, 2, 6, false)),
        ]);
        assert_eq!(keys(&q), [5, 6]);
        assert_eq!(
            q.epoch,
            Some(Epoch {
                doc_gen: 2,
                rules: other.id
            })
        );
    }

    /// The first lines stand in until the whole block comes, and leave the block wanted if the
    /// rest never does (O-05).
    #[test]
    fn partial_results_stand_in() {
        let theme = SyntaxTheme::new(&Theme::builtin().syntax);
        let text = "fn a() {}\n".repeat(10);
        let first = Arc::new(highlight_first(9, "rust", &text, &theme, 4));
        let whole = Arc::new(highlight(9, "rust", &text, &theme));
        assert!(first.partial && first.line_count() == 4 && whole.line_count() == 10);
        let mut h = Highlights::default();
        h.request(9);
        h.insert(first.clone());
        assert!(h.is_pending(9) && !h.wants(9), "still under way");
        assert_eq!(h.get(9).map(CodeSpans::line_count), Some(4));
        // stopped for another document: wanted again, the first lines still shown
        h.forget_pending();
        assert!(h.wants(9));
        h.request(9);
        h.insert(whole.clone());
        assert!(!h.wants(9) && !h.is_pending(9));
        // a late partial doesn't replace the whole
        h.insert(first);
        assert_eq!(h.get(9), Some(&*whole));
    }

    /// A long block posts its first 256, 512, 1,024 … lines, then all of it; another document
    /// stops it, and the next document's block comes next (O-05).
    #[test]
    fn the_worker_posts_early_and_stops_for_another_document() {
        let theme = Arc::new(SyntaxTheme::new(&Theme::builtin().syntax));
        let (tx, rx) = mpsc::channel();
        let worker = spawn_worker(tx);
        let long = |doc_gen, lines: usize| Job {
            doc_gen,
            block: 0,
            key: doc_gen,
            lang: "rust".into(),
            text: "let x = 1; // n\n".repeat(lines).into(),
            theme: theme.clone(),
            urgent: true,
            first: None,
        };
        worker.send(Request::Job(long(1, 1500))).unwrap();
        let mut posted = Vec::new();
        loop {
            let Msg::Highlighted { lines, spans, .. } =
                rx.recv_timeout(std::time::Duration::from_secs(20)).unwrap()
            else {
                panic!("not a highlight");
            };
            let spans = spans.0.unwrap();
            assert_eq!(spans.partial, lines.is_some());
            posted.push(spans.line_count());
            if lines.is_none() {
                break;
            }
        }
        assert_eq!(posted, [256, 512, 1024, 1500]);

        // far too long to finish; the next document's block comes instead, once it runs
        worker.send(Request::Job(long(2, 1_000_000))).unwrap();
        let Ok(Msg::Highlighted { doc_gen: 2, .. }) =
            rx.recv_timeout(std::time::Duration::from_secs(20))
        else {
            panic!("the long block's first lines");
        };
        worker
            .send(Request::Only(Epoch {
                doc_gen: 3,
                rules: theme.id,
            }))
            .unwrap();
        worker.send(Request::Job(long(3, 10))).unwrap();
        let next_whole = loop {
            let Msg::Highlighted { doc_gen, lines, .. } =
                rx.recv_timeout(std::time::Duration::from_secs(20)).unwrap()
            else {
                panic!("not a highlight");
            };
            if lines.is_none() {
                break doc_gen;
            }
            assert_eq!(
                doc_gen, 2,
                "only the stopped block's first lines come before"
            );
        };
        assert_eq!(next_whole, 3);
    }

    /// The parallel highlighting gives what one thread gives, however the block is cut: with
    /// states that hold captures (raw strings, heredocs), and with one that never comes back
    /// (a comment left open).
    #[test]
    fn parallel_highlighting_matches_one_thread() {
        let theme = Theme::builtin();
        let syntax = SyntaxTheme::new(&theme.syntax);
        let rust = "fn main() {\n    let s = r#\"a \"quoted\"\n line\"#;\n    // note\n    let n = 0x1f;\n}\n";
        let bash = "cat <<EOF\nhello $USER\nEOF\nfor f in *; do echo \"$f\"; done\n";
        let log =
            "2026-10-17 12:00:01 INFO started pid=42\n2026-10-17 12:00:02 ERROR failed: \"x\"\n";
        let c = "int x = 1; // note\n";
        let json = "  {\"k\": [1, -2.5e3, true, \"s\\\"\"], \"open\": \"x\n  \"n\": null},\n";
        for (lang, unit, reps) in [
            ("rust", rust, 60),
            ("bash", bash, 80),
            ("log", log, 150),
            ("c", c, 300),
            ("json", json, 150),
        ] {
            let mut text = unit.repeat(reps);
            if lang == "c" {
                text.insert_str(0, "/* never closed\n");
            }
            let one = highlight(1, lang, &text, &syntax);
            assert!(one.line_count() > 200, "{lang}: highlighted at all");
            let found = find_syntax(lang).unwrap();
            for threads in 2..=5 {
                let many = match lang {
                    "log" => highlight_log_on(1, &text, &syntax, threads),
                    "json" => highlight_json_on(1, &text, &syntax, threads),
                    _ => highlight_on(1, found, &text, &syntax, threads),
                };
                assert!(many == one, "{lang} on {threads} threads");
            }
        }
    }

    /// syntect's parser never returns on this Scala line (syntect#656); the stuck thread spins on
    /// until the tests end.
    const STUCK: &str = ":s| < @@@\n";

    /// A block syntect gets stuck on prints plain, and the blocks after it still highlight (F-06).
    #[test]
    fn print_leaves_a_block_syntect_is_stuck_on_plain() {
        let theme = Arc::new(SyntaxTheme::new(&Theme::builtin().syntax));
        let doc = crate::doc::parse(&format!(
            "```scala\n{STUCK}```\n\n```rust\nfn main() {{}}\n```\n"
        ));
        let blocks = doc.code_blocks();
        let all = Highlights::compute_all(&blocks, &theme);
        let [scala, rust] = [0, 1].map(|i| all.get(key(blocks[i]).unwrap()).unwrap());
        assert_eq!(scala.line_count(), 0);
        assert!(rust.line(0).len() > 1);
    }

    /// The worker posts a block syntect gets stuck on as plain, and a new worker takes the next
    /// one (F-06).
    #[test]
    fn the_worker_gives_up_on_a_stuck_block_and_goes_on() {
        let theme = Arc::new(SyntaxTheme::new(&Theme::builtin().syntax));
        let (tx, rx) = mpsc::channel();
        let worker = spawn_worker(tx);
        let job = |block, lang: &str, text: &str| Job {
            doc_gen: 1,
            block,
            key: u64::from(block),
            lang: lang.into(),
            text: text.into(),
            theme: theme.clone(),
            urgent: false,
            first: None,
        };
        worker.send(Request::Job(job(0, "scala", STUCK))).unwrap();
        worker
            .send(Request::Job(job(1, "rust", "fn main() {}\n")))
            .unwrap();
        let mut posted = Vec::new();
        while posted.len() < 2 {
            let Ok(Msg::Highlighted { block, spans, .. }) =
                rx.recv_timeout(Duration::from_secs(20))
            else {
                panic!("no highlight");
            };
            posted.push((block, spans.0.unwrap().line_count()));
        }
        assert_eq!(posted, [(0, 0), (1, 1)]);
    }
}
