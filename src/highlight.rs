//! Syntax highlighting (ADR-07, ADR-40, ADR-43): ada's own lexers (ada-parse), run on a worker
//! thread so the first paint never waits for them. A language none of them covers shows plain:
//! ada still names it, by bat's syntax set's names, extensions and first lines (`assets/`), but
//! has no grammar for it, since syntect left the binary in 1.1 (ADR-43).
//!
//! Results are color-free. A lexer's kinds carry bat's scopes, and the theme's `[syntax]` table
//! gives each the rule `i` syntect would have (`selector`), so a highlighted line is a list of
//! `(end byte, rule)` runs, and paint resolves each rule against the ada theme (ADR-03).
//! Results are cached by [`key`], a hash of the language and the code.

// `highlight` and `MAX_LINE` are `examples/parity.rs`'s, which includes the module by path
#[cfg(test)]
#[allow(dead_code)]
mod reference;
mod selector;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use crate::style::Style;
use selector::Selectors;

use crate::app::{Computed, Msg};
use crate::doc::{CodeBlock, CodeId};
use crate::layout::{block_needs_expanding, expand_block, expand_tabs, expand_tabs_in};
use ada_parse::{FirstLine, Lang};

/// Cached blocks before the cache starts over.
const CACHE_CAP: usize = 4096;

/// Code a print highlights on each thread of its own: below it, a second thread barely
/// shortens the blocks' time (PRN-01, measured when syntect compiled each grammar's regexes;
/// ada's own lexers start at once and run at hundreds of MB/s).
const CODE_PER_THREAD: usize = 16 * 1024;

/// Lines highlighted so far, by every thread. When it stands still for [`STALL`] while a block
/// is being highlighted, a lexer is stuck on a line (F-06): syntect's parser could loop forever
/// on a non-consuming push (syntect#656) until 1.1 (ADR-43). ada's own lexers are linear by
/// construction and fuzzed; the watch stays for a bug in one.
static LINES: AtomicU64 = AtomicU64::new(0);

/// How long a lexer may take on one line before its block is given up as plain. Real lines take
/// microseconds; the slowest of syntect's grammars (O-37) took about a second.
const STALL: Duration = Duration::from_millis(if cfg!(test) { 300 } else { 2000 });

/// Languages that are deliberately plain.
fn is_plain(lang: &str) -> bool {
    ["text", "txt", "plain", "plaintext", "none"]
        .iter()
        .any(|p| lang.eq_ignore_ascii_case(p))
}

pub use ada_parse::ALIASES;

/// A language ada names: one a lexer of its own highlights, or another of bat's syntax set,
/// which shows plain (ADR-43).
#[derive(Clone, Copy, Debug)]
pub enum Syntax {
    Own(Lang),
    /// The set's syntax of this name, which no lexer of ada's covers.
    Named(&'static str),
}

impl Syntax {
    /// The syntax's name, as syntect names it (`Rust`, `JSON`).
    pub fn name(self) -> &'static str {
        match self {
            Syntax::Own(lang) => lang.name(),
            Syntax::Named(name) => name,
        }
    }

    /// The name ada gives a whole file in it: [`Syntax::name`], but a Lisp dialect's own
    /// ([`Lang::label`]), which leads back to the dialect where `Lisp` would not.
    pub fn label(self) -> &'static str {
        match self {
            Syntax::Own(lang) => lang.label(),
            Syntax::Named(name) => name,
        }
    }

    /// The set's syntax named `name`, or the lexer that stands in for it.
    fn named(name: &'static str) -> Self {
        Lang::from_name(name).map_or(Syntax::Named(name), Syntax::Own)
    }
}

thread_local! {
    /// Fence tokens this thread looked up lately, as written, and what [`find_syntax`] found
    /// for each. A document's blocks name a few languages over and over, and the search
    /// through the aliases, extensions and names took a 7-line block 9% of its instructions
    /// in C#, 37% in `.env` (X-036).
    static FOUND: RefCell<Vec<(Box<str>, Option<Syntax>)>> = const { RefCell::new(Vec::new()) };
}

/// Tokens kept in [`FOUND`]; past that it starts over.
const FOUND_MAX: usize = 16;

/// The syntax for a fence language, with the aliases the vault needs.
pub fn find_syntax(lang: &str) -> Option<Syntax> {
    let found = |found: &Vec<(Box<str>, Option<Syntax>)>| {
        found.iter().find(|(t, _)| **t == *lang).map(|&(_, s)| s)
    };
    if let Some(syntax) = FOUND.try_with(|f| found(&f.borrow())).ok().flatten() {
        return syntax;
    }
    let syntax = search_syntax(lang);
    let _ = FOUND.try_with(|f| {
        let mut f = f.borrow_mut();
        if f.len() >= FOUND_MAX {
            f.clear();
        }
        f.push((lang.into(), syntax));
    });
    syntax
}

/// [`find_syntax`]'s search, without the cache.
fn search_syntax(lang: &str) -> Option<Syntax> {
    if is_plain(lang) {
        return None;
    }
    if let Some(lang) = Lang::from_token(lang) {
        return Some(Syntax::Own(lang));
    }
    names::find(lang).map(Syntax::named)
}

/// The language of a whole file, by its name alone: the name itself (`Makefile`, `.zshrc`,
/// `Cargo.lock`), then its extension. `None` when that says nothing, or says plain text.
pub fn file_language(path: &Path) -> Option<&'static str> {
    let name = path.file_name().and_then(|n| n.to_str());
    if let Some(lang) = name.and_then(Lang::from_file_name) {
        return Some(lang.label());
    }
    let by_name = name.and_then(names::by_extension).map(names::label);
    let by_ext = || {
        let ext = path.extension()?.to_str()?;
        if is_plain(ext) {
            return None;
        }
        match Lang::from_token(ext) {
            Some(lang) => Some(lang.label()),
            None => names::find(ext).map(names::label),
        }
    };
    by_name.or_else(by_ext).filter(|&n| n != "Plain Text")
}

/// The language a file's first line declares: a `#!` line, `<?xml`, a vim modeline, as bat's
/// set's first-line regexes read it. ada-parse's table decides those it can, and the `regex`
/// crate the rest (`names::first_line_matches`).
pub fn first_line_language(line: &str) -> Option<&'static str> {
    match Lang::first_line_asking(line, &mut names::first_line_matches) {
        FirstLine::Lang(lang) => Some(lang.label()),
        FirstLine::Syntax(name) => language(Syntax::named(name)),
        FirstLine::None | FirstLine::Ask => None,
    }
}

fn language(syntax: Syntax) -> Option<&'static str> {
    Some(syntax.label()).filter(|&n| n != "Plain Text")
}

/// bat's syntax set by name, extension and first line, as `cargo xtask names` lists it from
/// two-face's (`assets/`): syntect's lookups, without syntect (ADR-43).
pub(crate) mod names {
    use std::sync::OnceLock;

    use ada_parse::{ALIASES, Lang};
    use regex::Regex;

    /// A line a syntax, in the set's order: its name, then its extensions, tab-separated.
    pub(crate) static LIST: &str = include_str!("../assets/syntax-names.txt");

    /// The set's first-line regexes, `(syntax, regex)` in its order, as the `regex` crate reads
    /// them.
    pub(crate) const FIRST_LINES: &[(&str, &str)] = &include!("../assets/syntax-first-lines.rs");

    /// [`FIRST_LINES`]' regexes, each compiled the first time it's asked about: `None` for one
    /// the `regex` crate refuses.
    static COMPILED: [OnceLock<Option<Regex>>; FIRST_LINES.len()] =
        [const { OnceLock::new() }; FIRST_LINES.len()];

    /// Whether the set's `i`th first-line regex matches `line`, for `Lang::first_line_asking`:
    /// `None` for XML's and Dockerfile's, whose lookarounds and back-reference the `regex`
    /// crate refuses and ada-parse's table decides itself.
    pub fn first_line_matches(i: usize, line: &str) -> Option<bool> {
        let (_, regex) = FIRST_LINES.get(i)?;
        let compiled = COMPILED[i].get_or_init(|| Regex::new(regex).ok());
        compiled.as_ref().map(|r| r.is_match(line))
    }

    /// The syntaxes as syntect searches them, the last first: each name and its extensions.
    fn syntaxes() -> impl Iterator<Item = (&'static str, std::str::Split<'static, char>)> {
        LIST.lines().rev().map(|line| {
            let mut fields = line.split('\t');
            (fields.next().unwrap_or_default(), fields)
        })
    }

    /// `find_syntax_by_extension`'s syntax: its name.
    pub fn by_extension(ext: &str) -> Option<&'static str> {
        syntaxes()
            .find_map(|(name, mut exts)| exts.any(|e| e.eq_ignore_ascii_case(ext)).then_some(name))
    }

    /// `find_syntax_by_token`'s: by extension, then by name ignoring ASCII case.
    fn by_token(token: &str) -> Option<&'static str> {
        by_extension(token).or_else(|| {
            syntaxes()
                .map(|(name, _)| name)
                .find(|name| name.eq_ignore_ascii_case(token))
        })
    }

    /// The name of the set's syntax for a fence language that isn't plain and that ada-parse's
    /// table doesn't cover: the syntax's own name (`Ruby`, as a whole file's language is
    /// given), then the token, through ada's aliases.
    pub fn find(lang: &str) -> Option<&'static str> {
        if let Some((name, _)) = syntaxes().find(|&(name, _)| name == lang) {
            return Some(name);
        }
        let lower = lang.to_ascii_lowercase();
        let token = ALIASES
            .iter()
            .find(|&&(alias, _)| alias == lower)
            .map_or(lower.as_str(), |&(_, token)| token);
        by_token(token)
    }

    /// The name ada gives a file in the syntax named `name`: [`super::Syntax::label`].
    pub fn label(name: &'static str) -> &'static str {
        Lang::from_name(name).map_or(name, Lang::label)
    }
}

/// The cache key of a code block, or `None` when it has no language worth highlighting.
pub fn key(code: &CodeBlock) -> Option<u64> {
    key_of(code.lang.as_deref(), code.hash)
}

/// [`key`] of a code block in `lang` whose text hashes to `hash`.
pub fn key_of(lang: Option<&str>, hash: u64) -> Option<u64> {
    let lang = lang.filter(|l| !is_plain(l))?;
    let mut h = DefaultHasher::new();
    lang.hash(&mut h);
    hash.hash(&mut h);
    Some(h.finish())
}

/// The theme's `[syntax]` rules as scope selectors: rule `i` is the `i`th; text no rule
/// matches takes [`Rule::NONE`].
#[derive(Debug, PartialEq, Eq)]
pub struct SyntaxTheme {
    selectors: Selectors,
    /// The rules' selectors as the theme writes them, for what checks ada against syntect.
    scopes: Vec<Box<str>>,
    /// A hash of the rules' scope selectors: spans from another set of rules mean other rules.
    pub id: u64,
}

impl SyntaxTheme {
    pub fn new(rules: &[(String, Style)]) -> Self {
        let mut h = DefaultHasher::new();
        for (scope, _) in rules {
            scope.hash(&mut h);
        }
        let scopes: Vec<Box<str>> = rules
            .iter()
            .take(usize::from(Rule::NONE))
            .map(|(scope, _)| scope.as_str().into())
            .collect();
        Self {
            selectors: Selectors::new(scopes.iter().map(|s| &**s)),
            scopes,
            id: h.finish(),
        }
    }

    /// The rules' selectors as the theme writes them, rule `i` the `i`th: what syntect's
    /// reference theme is built from in ada's checks against it.
    #[doc(hidden)]
    pub fn scopes(&self) -> impl Iterator<Item = &str> {
        self.scopes.iter().map(|s| &**s)
    }

    /// The rule a scope stack takes, its outermost scope first, as `selector` picks it.
    pub fn rule(&self, stack: &[&str]) -> Rule {
        self.selectors.rule(stack)
    }
}

/// An index into the theme's `[syntax]` rules, and a highlighted run: bytes up to `end` of the
/// tab-expanded line take `rule`. ada-parse's, which its lexers write (X-068).
pub use ada_parse::{Rule, Run};

/// `rule`'s style among the theme's `rules`; none for [`Rule::NONE`].
pub fn rule_style(rule: Rule, rules: &[(String, Style)]) -> Option<Style> {
    rules.get(usize::from(rule.0)).map(|(_, s)| *s)
}

/// The highlighting of one code block. Empty when the language isn't known.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeSpans {
    pub key: u64,
    /// The [`SyntaxTheme::id`] the rules come from.
    pub rules: u64,
    /// Only the first lines, while the worker is still on the rest (O-05).
    pub partial: bool,
    lines: LineRuns,
}

impl CodeSpans {
    /// Line `n`'s runs; empty when the line is plain.
    pub fn line(&self, n: u32) -> &[Run] {
        self.lines.line(n as usize).unwrap_or_default()
    }

    /// How many lines are highlighted.
    pub fn line_count(&self) -> u32 {
        self.lines.len() as u32
    }

    /// A block's spans from its lines' runs, as something other than ada's lexers found them:
    /// syntect, in ada's checks against it.
    #[doc(hidden)]
    pub fn from_lines(rules: u64, lines: impl IntoIterator<Item = Vec<Run>>) -> Self {
        let mut runs = LineRuns::default();
        for line in lines {
            let mut open = runs.open_line();
            for run in line {
                open.push(run.end, run.rule);
            }
            open.end();
        }
        Self {
            key: 0,
            rules,
            partial: false,
            lines: runs.shrink(),
        }
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

/// Lines' runs end to end: line `n`'s are `runs[ends[n - 1]..ends[n]]`, from the first run for
/// line 0. One heap block for all of them, not one a line (HL-002: a box per line held 2–3×
/// the runs on code, and each partial result copied every box).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct LineRuns {
    runs: Vec<Run>,
    ends: Vec<u32>,
}

impl LineRuns {
    fn len(&self) -> usize {
        self.ends.len()
    }

    /// Where line `n`'s runs start: where the line before it ended.
    fn start(&self, n: usize) -> usize {
        n.checked_sub(1).map_or(0, |p| self.ends[p] as usize)
    }

    fn line(&self, n: usize) -> Option<&[Run]> {
        let end = *self.ends.get(n)? as usize;
        Some(&self.runs[self.start(n)..end])
    }

    /// A new line after the last, to push runs to until it ends.
    fn open_line(&mut self) -> OpenLine<'_> {
        OpenLine {
            open: self.runs.len(),
            runs: &mut self.runs,
            ends: &mut self.ends,
        }
    }

    fn clear(&mut self) {
        self.runs.clear();
        self.ends.clear();
    }

    /// Append `other`'s lines from line `from` on.
    fn append(&mut self, other: LineRuns, from: usize) {
        if self.ends.is_empty() && from == 0 {
            *self = other;
            return;
        }
        let (skip, base) = (other.start(from), self.runs.len());
        self.runs.extend_from_slice(&other.runs[skip..]);
        let ends = other.ends[from..].iter().map(|&e| e as usize - skip + base);
        self.ends
            .extend(ends.map(|e| u32::try_from(e).expect("a block of under 2³² runs")));
    }

    /// Give back what the vectors grew by beyond their length.
    fn shrink(mut self) -> Self {
        self.runs.shrink_to_fit();
        self.ends.shrink_to_fit();
        self
    }

    /// The lines as a finished block keeps them, at their size, leaving these empty: copied
    /// out of a vector [`BlockScratch`] keeps for the next block, else moved and shrunk.
    fn done(&mut self) -> Self {
        if self.runs.capacity() > RUNS_KEPT || self.ends.capacity() > ENDS_KEPT {
            return std::mem::take(self).shrink();
        }
        let done = Self {
            runs: self.runs.as_slice().to_vec(),
            ends: self.ends.as_slice().to_vec(),
        };
        self.clear();
        done
    }
}

/// A thread's vectors for highlighting a block, which the next block on the thread starts
/// from: grown from empty, a 7-line block's spans, runs and line ends took ~10 reallocations,
/// a ninth of its instructions in C# and JSON, and `shrink` copied the runs once more (X-036).
/// Only small ones are kept, so a thread holds 64 KB of them at most.
struct BlockScratch {
    spans: Vec<ada_parse::Span>,
    lines: LineRuns,
    /// The block's lines, when tabs expand them ([`expand_block`]).
    buf: String,
}

/// The most of each vector [`BlockScratch`] keeps between blocks, by length.
const SPANS_KEPT: usize = 1024;
const RUNS_KEPT: usize = 4096;
const ENDS_KEPT: usize = 4096;
const BUF_KEPT: usize = 4096;

thread_local! {
    static SCRATCH: Cell<BlockScratch> = const { Cell::new(BlockScratch::EMPTY) };
}

impl BlockScratch {
    const EMPTY: Self = Self {
        spans: Vec::new(),
        lines: LineRuns {
            runs: Vec::new(),
            ends: Vec::new(),
        },
        buf: String::new(),
    };

    /// The thread's scratch, empty; a fresh one while another block on the thread holds it.
    fn take() -> Self {
        SCRATCH
            .try_with(|s| s.replace(Self::EMPTY))
            .unwrap_or(Self::EMPTY)
    }

    /// Hand the vectors back to the thread for its next block, those that grew large dropped.
    fn put(mut self) {
        fn keep<T>(v: &mut Vec<T>, most: usize) {
            if v.capacity() > most {
                *v = Vec::new();
            } else {
                v.clear();
            }
        }
        keep(&mut self.spans, SPANS_KEPT);
        keep(&mut self.lines.runs, RUNS_KEPT);
        keep(&mut self.lines.ends, ENDS_KEPT);
        if self.buf.capacity() > BUF_KEPT {
            self.buf = String::new();
        } else {
            self.buf.clear();
        }
        let _ = SCRATCH.try_with(|s| s.set(self));
    }
}

/// A line of [`LineRuns`] being written: its runs are those from `open` on. Where the line
/// starts is read once, not for each run from `ends` through `&mut LineRuns`, which highlighted
/// the one-line 3.75 MB JSON (906,077 runs) ~10% slower than the box per line had.
struct OpenLine<'a> {
    runs: &'a mut Vec<Run>,
    ends: &'a mut Vec<u32>,
    open: usize,
}

impl OpenLine<'_> {
    /// Add a run, or stretch the line's last run when the rule is the same.
    fn push(&mut self, end: u32, rule: Rule) {
        let n = self.runs.len();
        if n > self.open && self.runs[n - 1].rule == rule {
            self.runs[n - 1].end = end;
        } else {
            self.runs.push(Run { end, rule });
        }
    }

    fn end(self) {
        self.ends.push(run_count(self.runs.len()));
    }
}

/// Wait on `rx` for as long as the lexers get somewhere: `None` once no line was highlighted
/// for [`STALL`], or the sender is gone.
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

/// The lexer of ada's own that highlights a language, if one does (ADR-40).
fn own(lang: &str) -> Option<Lang> {
    match find_syntax(lang)? {
        Syntax::Own(lang) => Some(lang),
        Syntax::Named(_) => None,
    }
}

/// A kind's place in [`ParseRules`] before its rule is resolved. No theme has this many rules.
const UNSET: Rule = Rule(Rule::UNSET);

/// The kinds' rules resolved so far, by language and theme (`SyntaxTheme::id`), for the next
/// block in them to start from: resolving takes a selector match for each kind, a sixth of a
/// YAML corpus's time when each block did it anew (HL-004, on syntect's matcher).
/// Shared, not copied, by the blocks that start from it: a table runs to the language's
/// highest kind (C#'s and Python's embedded regexes and SQL put theirs past 30,000), and
/// copying it cost a C# block ~66,000 instructions, as many as lexing 40 of its lines (X-022).
static RESOLVED: Mutex<Vec<Resolved>> = Mutex::new(Vec::new());

/// A language's and a theme's (`SyntaxTheme::id`) rules by kind in [`RESOLVED`].
type Resolved = (Lang, u64, Arc<[Rule]>);

/// Languages times themes kept in [`RESOLVED`]; past that it starts over.
const RESOLVED_MAX: usize = 64;

/// The theme's rule for each kind of an ada-parse language (ADR-40), as `selector` picks it
/// for the kind's scopes: all of them up front for a language with a fixed set
/// (`ParseRules::new`), else each the first time a span has it (`ParseRules::lazy`).
pub struct ParseRules {
    lang: Lang,
    /// By `Span::kind`; [`UNSET`] until resolved, and past the end for a kind not met yet.
    /// [`RESOLVED`]'s table until this block resolves a kind of its own, then a copy.
    rules: Arc<[Rule]>,
    /// [`RESOLVED`]'s table when this block started from it: if it's still the cache's when
    /// the block ends, nothing was resolved meanwhile and `rules` holds all of it.
    start: Arc<[Rule]>,
    /// The theme, and whether `rules` has rules [`RESOLVED`] lacks.
    theme: u64,
    fresh: bool,
}

impl ParseRules {
    /// Every kind's rule, for a language whose kinds are fixed (`Lang::kinds`).
    fn new(lang: Lang, theme: &SyntaxTheme) -> Self {
        let mut rules = Self::lazy(lang, theme.id);
        rules.rules = (0..lang.kinds().len() as u16)
            .map(|k| rules.resolve(k, theme))
            .collect();
        rules
    }

    /// No rule yet: each is resolved when a span first has its kind.
    fn lazy(lang: Lang, theme: u64) -> Self {
        Self {
            lang,
            rules: Arc::new([]),
            start: Arc::new([]),
            theme,
            fresh: false,
        }
    }

    /// The rules earlier blocks in `lang` and the theme resolved, the rest resolved as met.
    fn cached(lang: Lang, theme: u64) -> Self {
        let cache = RESOLVED.lock().unwrap_or_else(PoisonError::into_inner);
        // the cache's table, not `lazy`'s two empty ones made and dropped: an allocation and
        // a free each, ~180 instructions a block (X-036)
        match cache.iter().find(|(l, t, _)| *l == lang && *t == theme) {
            Some((_, _, r)) => Self {
                lang,
                rules: Arc::clone(r),
                start: Arc::clone(r),
                theme,
                fresh: false,
            },
            None => Self::lazy(lang, theme),
        }
    }

    /// Hand the rules this block resolved to the blocks after it ([`RESOLVED`]).
    fn keep(&mut self) {
        if !std::mem::take(&mut self.fresh) {
            return;
        }
        // without the unresolved tail `resolve_into`'s doubling left
        let used = self
            .rules
            .iter()
            .rposition(|&r| r != UNSET)
            .map_or(0, |i| i + 1);
        let mine: Arc<[Rule]> = if used == self.rules.len() {
            Arc::clone(&self.rules)
        } else {
            self.rules[..used].into()
        };
        let mut cache = RESOLVED.lock().unwrap_or_else(PoisonError::into_inner);
        let (lang, theme) = (self.lang, self.theme);
        let kept = match cache.iter_mut().find(|(l, t, _)| *l == lang && *t == theme) {
            // the table this block started from: it has all the cache's rules and more
            Some((_, _, r)) if Arc::ptr_eq(r, &self.start) => {
                *r = mine;
                Arc::clone(r)
            }
            Some((_, _, r)) => {
                // what another block resolved meanwhile, joined with this one's
                let mut joined = mine.to_vec();
                if joined.len() < r.len() {
                    joined.resize(r.len(), UNSET);
                }
                for (m, theirs) in joined.iter_mut().zip(r.iter()) {
                    if *m == UNSET {
                        *m = *theirs;
                    }
                }
                *r = joined.into();
                Arc::clone(r)
            }
            None => {
                if cache.len() >= RESOLVED_MAX {
                    cache.clear();
                }
                cache.push((lang, theme, Arc::clone(&mine)));
                mine
            }
        };
        self.start = kept;
    }

    fn resolve(&self, kind: u16, theme: &SyntaxTheme) -> Rule {
        let mut stack = vec![self.lang.base()];
        stack.extend(self.lang.scopes(kind));
        let rule = theme.rule(&stack);
        debug_assert_ne!(rule, UNSET);
        rule
    }

    /// Resolve `kind`'s rule into `rules`, the first time a span has it.
    #[cold]
    #[inline(never)]
    fn resolve_into(&mut self, kind: u16, theme: &SyntaxTheme) -> Rule {
        let (k, len) = (usize::from(kind), self.rules.len());
        let rule = self.resolve(kind, theme);
        match Arc::get_mut(&mut self.rules) {
            Some(rules) if k < len => rules[k] = rule,
            _ => {
                // the shared table copied, once a block, or the table grown: doubled, as a `Vec`
                // grows, so a block meeting kinds far apart copies it a few times at most
                let mut rules = self.rules.to_vec();
                if k >= len {
                    rules.resize((2 * len).max(k + 1), UNSET);
                }
                rules[k] = rule;
                self.rules = rules.into();
            }
        }
        self.fresh = true;
        rule
    }

    /// Highlight one line into `out`, after the lines before it left `state`; `spans` and
    /// `buf` (the line when tabs expand it) are scratch space. Every kind's rule must be
    /// resolved ([`ParseRules::new`]). For lines handed out one at a time (`lines_on`).
    #[inline(never)]
    fn line(
        &self,
        line: &str,
        state: &mut ada_parse::State,
        (spans, buf): (&mut Vec<ada_parse::Span>, &mut String),
        out: &mut LineRuns,
    ) {
        let line = expand_tabs_in(line, buf);
        let rules = &mut AllResolved(&self.rules);
        ada_parse::lex_runs(self.lang, line, state, spans, rules, &mut out.runs);
        out.ends.push(run_count(out.runs.len()));
    }

    /// Highlight up to `max` of `text`'s lines into `out`, after the lines before them left
    /// `state`; returns where the lines left start, `None` once `text` is done (as
    /// [`ada_parse::lex_lines`]). The lines are expanded only when `expand`, their block's
    /// text holding a control character, into `buf` ([`lex_block`]); `spans` and `buf` are
    /// scratch space. Every kind's rule must be resolved ([`ParseRules::new`]).
    fn lines(
        &self,
        text: &str,
        max: usize,
        state: &mut ada_parse::State,
        scratch: (&mut Vec<ada_parse::Span>, &mut String, bool),
        out: &mut LineRuns,
    ) -> Option<usize> {
        let rules = &mut AllResolved(&self.rules);
        lex_block(self.lang, text, max, state, scratch, rules, out)
    }

    /// As [`ParseRules::lines`], resolving the rules of kinds met for the first time.
    fn lines_resolving(
        &mut self,
        text: &str,
        max: usize,
        state: &mut ada_parse::State,
        scratch: (&mut Vec<ada_parse::Span>, &mut String, bool),
        theme: &SyntaxTheme,
        out: &mut LineRuns,
    ) -> Option<usize> {
        let lang = self.lang;
        // the rule looked up once a span, resolved in the same pass when it's new (HL-004)
        let rules = &mut Resolving { rules: self, theme };
        lex_block(lang, text, max, state, scratch, rules, out)
    }
}

/// [`ada_parse::lex_lines_runs`] on up to `max` of `text`'s lines into `out`: where the lines
/// left start in `text`, `None` once it's done. When `expand`, the block's text holding a
/// control character, the lines are expanded into `buf` first, a pass over the block
/// ([`expand_block`], X-088), and lexed from there: expanded a line at a time (ada-parse's
/// `prep`), each line's length-dependent loops missed their exits. ada-parse's `prep` stays,
/// `None` here: taken out of its line loop, Rust, Kotlin and Lua ran 0.5–1.7% slower in the
/// default layout (placement; their lines have no tabs).
fn lex_block(
    lang: Lang,
    text: &str,
    max: usize,
    state: &mut ada_parse::State,
    (spans, buf, expand): (&mut Vec<ada_parse::Span>, &mut String, bool),
    rules: &mut dyn ada_parse::Rules,
    out: &mut LineRuns,
) -> Option<usize> {
    let LineRuns { runs, ends } = out;
    if !expand {
        return ada_parse::lex_lines_runs(
            lang,
            text,
            max,
            state,
            (spans, buf),
            None,
            rules,
            runs,
            ends,
        );
    }
    if max == 0 {
        return Some(0);
    }
    let next = expand_block(text, max, buf);
    let done = ada_parse::lex_lines_runs(
        lang,
        buf,
        usize::MAX,
        state,
        (spans, &mut String::new()),
        None,
        rules,
        runs,
        ends,
    );
    debug_assert!(done.is_none(), "every expanded line lexed");
    next
}

/// Where a line's runs end, as [`LineRuns::ends`] holds it.
#[inline(always)]
fn run_count(n: usize) -> u32 {
    // a run is a byte or more of the text, so this takes a block of over 4 GiB
    u32::try_from(n).expect("a block of under 2³² runs")
}

/// A table whose kinds' rules are all resolved ([`ParseRules::new`]): a kind past it has none.
struct AllResolved<'a>(&'a [Rule]);

impl ada_parse::Rules for AllResolved<'_> {
    fn table(&self) -> &[Rule] {
        self.0
    }

    fn resolve(&mut self, _: u16) -> Rule {
        Rule(Rule::NONE)
    }
}

/// A block's rules, each resolved into its table the first time a span has its kind.
struct Resolving<'a> {
    rules: &'a mut ParseRules,
    theme: &'a SyntaxTheme,
}

impl ada_parse::Rules for Resolving<'_> {
    fn table(&self) -> &[Rule] {
        &self.rules.rules
    }

    fn resolve(&mut self, kind: u16) -> Rule {
        self.rules.resolve_into(kind, self.theme)
    }
}

/// How a file printed a piece at a time is highlighted (O-52): not at all, or by one of ada's
/// own lexers. Every file streams since syntect's languages went plain (ADR-43).
pub enum Lexer {
    Plain,
    /// An ada-parse language whose lines don't depend on each other (`Lang::line_by_line`),
    /// or JSON: each piece is highlighted on the thread that paints it.
    Parse(ParseRules),
    /// Any other ada-parse language, whose state carries from line to line (PRN-003): one
    /// [`Carrier`] highlights the pieces in order, and the threads that paint them get them
    /// highlighted.
    Carry(Lang),
}

impl Lexer {
    /// The lexer for a file in `lang`: plain for none, or one no lexer of ada's covers.
    pub fn new(lang: Option<&str>, theme: &SyntaxTheme) -> Self {
        match lang.and_then(own) {
            None => Lexer::Plain,
            Some(l) if streams(l) => Lexer::Parse(ParseRules::new(l, theme)),
            Some(l) => Lexer::Carry(l),
        }
    }

    /// Highlight `code`'s lines as [`highlight`] would, after the lines before it left
    /// `state` (JSON's open `/* */`; a line-by-line language's is always the default). Also
    /// returns the state its last line leaves. `None` when the block is plain. Not for
    /// [`Lexer::Carry`], whose pieces a [`Carrier`] highlights.
    pub fn highlight(
        &self,
        code: &CodeBlock,
        theme: &SyntaxTheme,
        mut state: ada_parse::State,
    ) -> (Option<CodeSpans>, ada_parse::State) {
        let Some(key) = key(code) else {
            return (None, state);
        };
        let rules = match self {
            Lexer::Plain => return (None, state),
            Lexer::Carry(_) => unreachable!("a carried piece comes highlighted"),
            Lexer::Parse(rules) => rules,
        };
        let mut scratch = BlockScratch::take();
        let BlockScratch { spans, lines, buf } = &mut scratch;
        // the lines as `CodeBlock::lines` splits them, which `Lines` and `lex_lines` share
        if let Some(text) = Lines::new(&code.text).0 {
            let expand = block_needs_expanding(text);
            rules.lines(text, usize::MAX, &mut state, (spans, buf, expand), lines);
        }
        let spans = CodeSpans {
            key,
            rules: theme.id,
            partial: false,
            lines: lines.done(),
        };
        scratch.put();
        (Some(spans), state)
    }

    /// The state `text`'s lines leave, after the lines before them left `state`, as
    /// [`Lexer::highlight`] would find: only JSON's carries anything, a `/* */` comment open.
    /// A comment opens only on a line with `/*` in it and closes only on one with `*/`, so
    /// only the next such line is scanned, and the text between is skipped.
    pub fn state_after(&self, text: &str, mut state: ada_parse::State) -> ada_parse::State {
        if !matches!(self, Lexer::Parse(r) if r.lang == Lang::Json) {
            return state;
        }
        let (mut spans, mut runs) = (Vec::new(), Vec::new());
        let mut rest = text;
        // both marks hold a `*`, which JSON hardly has otherwise
        let mark = |rest: &str, comment: bool| {
            let b = rest.as_bytes();
            let mut from = 0;
            while let Some(i) = rest[from..].find('*').map(|i| from + i) {
                let found = if comment {
                    b.get(i + 1) == Some(&b'/')
                } else {
                    i > 0 && b[i - 1] == b'/'
                };
                if found {
                    return Some(i);
                }
                from = i + 1;
            }
            None
        };
        // JSON's state is a comment open, or the default
        while let Some(at) = mark(rest, state != ada_parse::State::default()) {
            let start = rest[..at].rfind('\n').map_or(0, |i| i + 1);
            let end = rest[at..].find('\n').map_or(rest.len(), |i| at + i);
            let line = &rest[start..end];
            let line = line.strip_suffix('\r').unwrap_or(line);
            // on the run API, as every other line is: a call to `lex` would link the span scans
            // of the languages whose scans write runs (JSON's among them) beside them (X-072)
            runs.clear();
            ada_parse::lex_runs(
                Lang::Json,
                &expand_tabs(line),
                &mut state,
                &mut spans,
                &mut Plain,
                &mut runs,
            );
            rest = rest.get(end + 1..).unwrap_or_default();
        }
        state
    }
}

/// A theme with no rules, for a scan run only for the state it leaves.
struct Plain;

impl ada_parse::Rules for Plain {
    fn table(&self) -> &[ada_parse::Rule] {
        &[]
    }

    fn resolve(&mut self, _: u16) -> ada_parse::Rule {
        ada_parse::Rule(ada_parse::Rule::NONE)
    }
}

/// Whether a streamed print highlights `lang`'s pieces on the threads that paint them: its
/// lines don't depend on each other, or it is JSON, whose one state (a `/* */` open) the reader
/// finds without lexing every line. Any other ada-parse language is a [`Lexer::Carry`].
fn streams(lang: Lang) -> bool {
    lang == Lang::Json || lang.line_by_line()
}

/// Highlights the pieces of a file printed a piece at a time in a [`Lexer::Carry`] language, in
/// order, carrying the lexer's state from each piece to the next (PRN-003). Its pieces' lines
/// get [`highlight`]'s spans for the whole file: a line's spans depend only on the line and the
/// state before it, and a piece's flag for tab expansion leaves a line without a control
/// character as it is, as the whole block's does.
pub struct Carrier<'a> {
    rules: ParseRules,
    theme: &'a SyntaxTheme,
    state: ada_parse::State,
    spans: Vec<ada_parse::Span>,
    /// The piece's runs, kept from piece to piece: [`LineRuns::done`] copies them out.
    lines: LineRuns,
    buf: String,
}

impl<'a> Carrier<'a> {
    pub fn new(lang: Lang, theme: &'a SyntaxTheme) -> Self {
        Self {
            rules: ParseRules::cached(lang, theme.id),
            theme,
            state: ada_parse::State::default(),
            spans: Vec::new(),
            lines: LineRuns::default(),
            buf: String::new(),
        }
    }

    /// Highlight the next piece, `text`, whose block has the cache key `key`.
    pub fn highlight(&mut self, key: u64, text: &str) -> CodeSpans {
        let expand = block_needs_expanding(text);
        let mut lines = std::mem::take(&mut self.lines);
        if let Some(text) = Lines::new(text).0 {
            self.rules.lines_resolving(
                text,
                usize::MAX,
                &mut self.state,
                (&mut self.spans, &mut self.buf, expand),
                self.theme,
                &mut lines,
            );
        }
        let spans = CodeSpans {
            key,
            rules: self.theme.id,
            partial: false,
            lines: lines.done(),
        };
        self.lines = lines;
        spans
    }
}

impl Drop for Carrier<'_> {
    fn drop(&mut self) {
        self.rules.keep();
    }
}

/// What highlights a block's lines.
enum Engine {
    /// Also whether the block's text has a control character, which `expand_tabs` changes.
    Parse(ParseRules, ada_parse::State, Vec<ada_parse::Span>, bool),
    /// A lexer stuck on a line, for the stall watch's tests (F-06).
    #[cfg(test)]
    Stuck,
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
        // the memchr crate's search, vectorized: `split_once`'s was a fifth of a Diff corpus's
        // time in ada (HL-004)
        let (line, tail) = match memchr::memchr(b'\n', rest.as_bytes()) {
            Some(i) => (&rest[..i], Some(&rest[i + 1..])),
            None => (rest, None),
        };
        self.0 = tail;
        Some(line.strip_suffix('\r').unwrap_or(line))
    }
}

/// Highlights a block a few lines at a time, carrying the lexer's state from one step to the
/// next, so the worker can post the first lines early and stop between steps (O-05).
pub struct Highlighter<'a> {
    key: u64,
    rules: u64,
    theme: &'a SyntaxTheme,
    /// `None` for a block that shows plain.
    state: Option<Engine>,
    rest: Lines<'a>,
    lines: LineRuns,
    buf: String,
}

impl<'a> Highlighter<'a> {
    pub fn new(key: u64, lang: &str, text: &'a str, theme: &'a SyntaxTheme) -> Self {
        let BlockScratch { spans, lines, buf } = BlockScratch::take();
        // a header's fence (```` ```h ````) by its text, as a `.h` file (X-134); a header
        // template's (```` ```h.in ````) too (X-140)
        let own = if lang.eq_ignore_ascii_case("h") {
            Some(Lang::for_header(text.as_bytes()))
        } else if lang.eq_ignore_ascii_case("h.in") {
            Some(Lang::for_header_template(text.as_bytes()))
        } else {
            own(lang)
        };
        let state = match own {
            #[cfg(test)]
            _ if lang == tests::STUCK => Some(Engine::Stuck),
            Some(lang) => Some(Engine::Parse(
                ParseRules::cached(lang, theme.id),
                ada_parse::State::default(),
                spans,
                block_needs_expanding(text),
            )),
            None => None,
        };
        Self {
            key,
            rules: theme.id,
            state,
            theme,
            rest: Lines::new(text),
            lines,
            buf,
        }
    }

    /// Highlight up to `n` more lines. False once the block is done.
    pub fn step(&mut self, n: usize) -> bool {
        let Some(state) = &mut self.state else {
            return false;
        };
        let out = &mut self.lines;
        match state {
            Engine::Parse(rules, state, spans, expand) => {
                // the stall watch counts lines; an ada-parse lexer never stalls, so they're
                // counted a batch at a time, a call a batch, not with an atomic add a line
                // (HL-004; the loop is ada-parse's since X-086)
                const BATCH: usize = 1024;
                let Some(mut text) = self.rest.0 else {
                    return false;
                };
                let mut left = n;
                loop {
                    let (max, done) = (left.min(BATCH), out.ends.len());
                    let next = rules.lines_resolving(
                        text,
                        max,
                        state,
                        (spans, &mut self.buf, *expand),
                        self.theme,
                        out,
                    );
                    LINES.fetch_add((out.ends.len() - done) as u64, Ordering::Relaxed);
                    left -= max;
                    match next {
                        Some(p) if left > 0 => text = &text[p..],
                        next => {
                            self.rest = Lines(next.map(|p| &text[p..]));
                            break;
                        }
                    }
                }
            }
            #[cfg(test)]
            Engine::Stuck => loop {
                thread::park();
            },
        }
        self.rest.0.is_some()
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

    pub fn finish(mut self) -> CodeSpans {
        CodeSpans {
            key: self.key,
            rules: self.rules,
            partial: false,
            lines: self.lines.done(),
        }
    }
}

impl Drop for Highlighter<'_> {
    fn drop(&mut self) {
        let mut spans = Vec::new();
        if let Some(Engine::Parse(rules, _, s, _)) = &mut self.state {
            rules.keep();
            spans = std::mem::take(s);
        }
        BlockScratch {
            spans,
            lines: std::mem::take(&mut self.lines),
            buf: std::mem::take(&mut self.buf),
        }
        .put();
    }
}

/// Blocks this big are highlighted on several threads in print mode (O-04).
const PARALLEL_BYTES: usize = 1 << 20;
const PARALLEL_LINES: usize = 20_000;

/// [`highlight`] for print mode (O-04): a block of 1 MiB or 20,000 lines and up whose lines
/// don't depend on each other (`Lang::line_by_line`, or JSON without a `/* */`) is cut into a
/// chunk of lines per thread. Any other block goes on one thread: ada's lexers run at hundreds
/// of MB/s, and the chunks that guessed a state and met the true one were syntect's, gone with
/// it (ADR-43). The result is [`highlight`]'s.
pub fn highlight_parallel(key: u64, lang: &str, text: &str, theme: &SyntaxTheme) -> CodeSpans {
    let Some(own) = own(lang) else {
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
    match own {
        // a `/* */` comment carries over lines: rare in JSON this big, and fast on one thread
        Lang::Json if text.contains("/*") => highlight(key, lang, text, theme),
        // without one, its lines don't depend on each other
        Lang::Json => highlight_parse_on(key, Lang::Json, text, theme, threads),
        own if own.line_by_line() => highlight_parse_on(key, own, text, theme, threads),
        _ => highlight(key, lang, text, theme),
    }
}

/// An ada-parse language read line by line (`Lang::line_by_line`), or JSON with no `/* */`,
/// on `threads` threads.
fn highlight_parse_on(
    key: u64,
    lang: Lang,
    text: &str,
    theme: &SyntaxTheme,
    threads: usize,
) -> CodeSpans {
    let rules = ParseRules::new(lang, theme);
    lines_on(key, text, theme, threads, &|line, scratch, out| {
        let state = &mut ada_parse::State::default();
        rules.line(line, state, (&mut scratch.parse, &mut scratch.buf), out);
    })
}

/// A thread's scratch space for [`lines_on`]: the spans of a line.
#[derive(Default)]
struct Scratch {
    parse: Vec<ada_parse::Span>,
    /// The line, when tabs expand it.
    buf: String,
}

/// A block whose lines don't depend on each other on `threads` threads, each taking a chunk of
/// them as it comes; `line` highlights one, with scratch space of its own.
// one copy for every lexer: the thread scope it builds is a few KB a copy (ADR-40's size
// discipline)
#[inline(never)]
fn lines_on(
    key: u64,
    text: &str,
    theme: &SyntaxTheme,
    threads: usize,
    line: &(dyn Fn(&str, &mut Scratch, &mut LineRuns) + Sync),
) -> CodeSpans {
    let all: Vec<&str> = Lines::new(text).collect();
    let chunks: Vec<LineRuns> = thread::scope(|s| {
        let handles: Vec<_> = all
            .chunks(all.len().div_ceil(threads))
            .map(|chunk| {
                s.spawn(move || {
                    let mut scratch = Scratch::default();
                    let mut lines = LineRuns::default();
                    lines.ends.reserve_exact(chunk.len());
                    // the watch in print mode sees progress (F-06)
                    for part in chunk.chunks(4096) {
                        for l in part {
                            line(l, &mut scratch, &mut lines);
                        }
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
    let mut lines = LineRuns::default();
    lines
        .runs
        .reserve_exact(chunks.iter().map(|c| c.runs.len()).sum());
    lines.ends.reserve_exact(all.len());
    for chunk in chunks {
        lines.append(chunk, 0);
    }
    CodeSpans {
        key,
        rules: theme.id,
        partial: false,
        lines: lines.shrink(),
    }
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

    /// Highlight every block of a document before returning (print mode). The blocks go on
    /// threads of their own, one for each `CODE_PER_THREAD` of code up to one a core, each
    /// taking the next block as it's done with one. When no line gets done for `STALL`, the
    /// blocks taken and not done are left plain, and their threads left to them, while new ones
    /// take the rest (F-06). On one malloc arena those threads wait on its lock: give the
    /// process an arena a thread first ([`print::start`](crate::print::start)).
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
        let code: usize = todo.iter().map(|t| t.2.len()).sum();
        let cores = thread::available_parallelism().map_or(1, |n| n.get().min(8));
        let threads = cores.min(1 + code / CODE_PER_THREAD);
        while !todo.is_empty() {
            let jobs = Arc::new(todo);
            let next = Arc::new(AtomicUsize::new(0));
            let (tx, rx) = mpsc::channel();
            for _ in 0..threads.min(jobs.len()) {
                let (jobs, next, tx, rules) = (
                    Arc::clone(&jobs),
                    Arc::clone(&next),
                    tx.clone(),
                    Arc::clone(theme),
                );
                thread::spawn(move || {
                    while let Some((key, lang, text)) =
                        jobs.get(next.fetch_add(1, Ordering::Relaxed))
                    {
                        let spans = highlight_parallel(*key, lang, text, &rules);
                        if tx.send(spans).is_err() {
                            return;
                        }
                    }
                });
            }
            drop(tx);
            let mut left = jobs.len();
            while left > 0 {
                let Some(spans) = recv_watched(&rx) else {
                    break;
                };
                out.done.insert(spans.key, Arc::new(spans));
                left -= 1;
            }
            // what the stuck threads took goes plain; they take no more
            let taken = next.swap(jobs.len(), Ordering::Relaxed).min(jobs.len());
            todo = Vec::new();
            for (i, job) in jobs.iter().enumerate() {
                if out.done.contains_key(&job.0) {
                    continue;
                }
                if i < taken {
                    out.done
                        .insert(job.0, Arc::new(CodeSpans::plain(job.0, theme.id)));
                } else {
                    todo.push(job.clone());
                }
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

/// Start the highlight worker. It works through its queue, urgent jobs first, posting each
/// result to `tx`. A block goes
/// `STEP` lines at a time: in between, the worker takes new requests, and stops the block when
/// another document or theme came along. A long block posts its first lines when 256, 512,
/// 1,024 … are done, so the top shows highlighted long before the rest (O-05).
///
/// A watcher thread stands by while a block is highlighted. When a lexer gets stuck on a line
/// (F-06; syntect could, until 1.1), it posts the block as plain and starts a new worker on the
/// same queue; the stuck thread is left to spin, since a thread can't be stopped from outside,
/// and quits if it ever gets free. While the worker is idle, so is the watcher.
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

    /// The language of a block whose lexer gets stuck on its first line ([`Engine::Stuck`]),
    /// as syntect's parser did on the Scala line `:s| < @@@` (syntect#656). The stuck thread
    /// stays parked until the tests end.
    pub(super) const STUCK: &str = "stuck-for-tests";

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

    /// The cache by fence token answers as the search does, for tokens new to it and again,
    /// past the point where it starts over (X-036).
    #[test]
    fn find_syntax_cached_as_searched() {
        let tokens = "rust Rust json env dotenv ts TypeScript zsh text mermaid ruby Ruby kt cs c# \
                      gitattributes fstab ini yaml toml";
        for _ in 0..2 {
            for t in tokens.split_whitespace() {
                assert_eq!(
                    find_syntax(t).map(Syntax::name),
                    search_syntax(t).map(Syntax::name),
                    "{t}"
                );
            }
        }
    }

    /// A block highlighted on a thread whose scratch a larger block left holds no more than
    /// its own runs, the same ones a fresh thread gives, and what grew past the scratch's
    /// limit is dropped, not kept (X-036).
    #[test]
    fn scratch_leaves_blocks_as_fresh_ones() {
        let theme = SyntaxTheme::new(&Theme::builtin().syntax);
        let small = "fn main() {\n    let x = 1; // one\n}\n";
        let fresh = thread::scope(|s| s.spawn(|| highlight(1, "rust", small, &theme)).join())
            .expect("thread");
        let big = "let a = [1, \"two\", 3.0];\n".repeat(2000);
        thread::scope(|s| {
            s.spawn(|| {
                let first = highlight(1, "rust", small, &theme);
                let large = highlight(2, "rust", &big, &theme);
                assert!(large.lines.runs.len() > RUNS_KEPT);
                assert_eq!(large.lines.runs.len(), large.lines.runs.capacity());
                let kept = SCRATCH.replace(BlockScratch::EMPTY);
                assert!(kept.lines.runs.capacity() <= RUNS_KEPT);
                SCRATCH.set(kept);
                let again = highlight(1, "rust", small, &theme);
                for spans in [&first, &again] {
                    assert_eq!(spans, &fresh);
                    assert_eq!(spans.lines.runs.capacity(), spans.lines.runs.len());
                    assert_eq!(spans.lines.ends.capacity(), spans.lines.ends.len());
                }
            });
        });
    }

    /// `find_syntax` as it was before ada-parse (ADR-40): on the whole set.
    fn find_in_full(lang: &str) -> Option<&'static str> {
        let set = reference::syntaxes();
        if is_plain(lang) {
            return None;
        }
        let lower = lang.to_ascii_lowercase();
        let token = ALIASES
            .iter()
            .find(|&&(alias, _)| alias == lower)
            .map_or(lower.as_str(), |&(_, token)| token);
        let found = set.find_syntax_by_name(lang);
        found
            .or_else(|| set.find_syntax_by_token(token))
            .map(|s| &*s.name)
    }

    /// `file_language` as it was before ada-parse: on the whole set.
    fn file_in_full(path: &str) -> Option<&'static str> {
        let path = Path::new(path);
        let name = path.file_name().and_then(|n| n.to_str());
        let by_name = name.and_then(|n| reference::syntaxes().find_syntax_by_extension(n));
        let by_ext = || find_in_full(path.extension()?.to_str()?);
        by_name.map(|s| &*s.name).or_else(by_ext)
    }

    /// Every name and extension the set knows and every alias, in its own case, lowercase and
    /// uppercase.
    fn words() -> Vec<String> {
        let mut words: Vec<String> = ALIASES
            .iter()
            .flat_map(|&(a, t)| [a.to_owned(), t.to_owned()])
            .collect();
        for s in reference::syntaxes().syntaxes() {
            words.push(s.name.clone());
            words.extend(s.file_extensions.iter().cloned());
        }
        for w in words.clone() {
            words.push(w.to_ascii_lowercase());
            words.push(w.to_ascii_uppercase());
        }
        words
    }

    /// ada-parse's table names a language only where syntect on the whole set names the same,
    /// and misses none of the ones it covers; and ada, the table first and the set's list of
    /// names after (ADR-43), names what the whole set names: as a fence's token, a file's whole
    /// name and its extension. Two departures are by design: ada's JavaScript is Babel's
    /// grammar whatever the token, where the set gives Sublime's `JavaScript` for
    /// ```` ```JavaScript ```` (ADR-40); and a CMake header template is named by its whole name
    /// (`x.h.in`, `x.hpp.in`), where the set names only a file called `h.in` (X-140).
    #[test]
    fn the_language_table_agrees_with_the_set() {
        let covered = |name: Option<&str>| name.and_then(Lang::from_name);
        let departs = |ours: Option<&str>, full: Option<&str>| {
            ours == Some(Lang::JavaScript.name()) && full == Some("JavaScript")
                || full.is_none()
                    && [Lang::CMakeCHeader, Lang::CMakeCppHeader]
                        .iter()
                        .any(|l| ours == Some(l.name()))
        };
        let mut words = words();
        words.extend(["PowerShell", "powershell", "ps1", "PSM1", "psd1"].map(String::from));
        for w in words {
            let full = find_in_full(&w);
            let ours = Lang::from_token(&w).map(Lang::name);
            if departs(ours, full) {
                assert_eq!(find_syntax(&w).map(Syntax::name), ours, "token {w:?}");
            } else {
                match Lang::from_token(&w) {
                    Some(lang) => assert_eq!(full, Some(lang.name()), "token {w:?}"),
                    None => assert_eq!(covered(full), None, "token {w:?}: not in the table"),
                }
                assert_eq!(find_syntax(&w).map(Syntax::name), full, "token {w:?}");
            }
            for file in [w.clone(), format!("x.{w}")] {
                let full = file_in_full(&file);
                let name = Path::new(&file).file_name().and_then(|n| n.to_str());
                let ours = name.and_then(Lang::from_file_name).map(Lang::name);
                if departs(ours, full) {
                    assert_eq!(file_language(Path::new(&file)), ours, "file {file:?}");
                    continue;
                }
                match name.and_then(Lang::from_file_name) {
                    Some(lang) => assert_eq!(full, Some(lang.name()), "file {file:?}"),
                    None => assert_eq!(covered(full), None, "file {file:?}: not in the table"),
                }
                // a Lisp dialect's file is named for it (`Lang::label`), where the set says `Lisp`
                let full = name
                    .and_then(Lang::from_file_name)
                    .map_or(full, |l| Some(l.label()))
                    .filter(|&n| n != "Plain Text");
                assert_eq!(file_language(Path::new(&file)), full, "file {file:?}");
            }
        }
    }

    /// A Scheme or Emacs Lisp file, or `-l` naming one, is read in its dialect: the name its
    /// file is given leads back to it, not to `Lisp`'s (Common Lisp's reading).
    #[test]
    fn a_lisp_dialects_file_keeps_its_dialect() {
        for (file, label, lang) in [
            ("x.scm", "Scheme", Lang::Scheme),
            ("x.sld", "Scheme", Lang::Scheme),
            ("init.el", "Emacs Lisp", Lang::EmacsLisp),
            ("x.lisp", "Lisp", Lang::Lisp),
            ("x.rkt", "Racket", Lang::Racket),
        ] {
            assert_eq!(file_language(Path::new(file)), Some(label), "{file}");
            assert!(
                matches!(find_syntax(label), Some(Syntax::Own(l)) if l == lang),
                "{file}"
            );
        }
        for (token, lang) in [("scheme", Lang::Scheme), ("elisp", Lang::EmacsLisp)] {
            let label = find_syntax(token).map(Syntax::label);
            assert!(
                matches!(label.and_then(find_syntax), Some(Syntax::Own(l)) if l == lang),
                "{token}"
            );
        }
    }

    /// The first lines ada-parse tells about are syntect's on the whole set: a language of its
    /// own, or no syntax at all; and ada's answer, with the `regex` crate on the set's regexes
    /// for the rest (ADR-43), is syntect's.
    fn first_lines_agree(lines: &[String]) -> (usize, usize) {
        let full = |l: &str| {
            let found = reference::syntaxes().find_syntax_by_first_line(l);
            found.map(|s| &*s.name).filter(|&n| n != "Plain Text")
        };
        let (mut claimed, mut asked) = (0, 0);
        for l in lines {
            let want = full(l);
            match Lang::first_line(l) {
                FirstLine::Lang(lang) => {
                    assert_eq!(want, Some(lang.name()), "{l:?}");
                    claimed += 1;
                }
                FirstLine::None => assert_eq!(want, None, "{l:?}"),
                FirstLine::Syntax(name) => panic!("{l:?}: {name} without asking"),
                FirstLine::Ask => asked += 1,
            }
            assert_eq!(first_line_language(l), want, "{l:?}");
        }
        (claimed, asked)
    }

    #[test]
    fn first_lines_agree_with_the_set() {
        let interpreters = [
            "sh",
            "bash",
            "zsh",
            "dash",
            "ksh",
            "tcsh",
            "ash",
            "fish",
            "python",
            "python3",
            "python3.12",
            "python312",
            "python3x",
            "perl",
            "ruby",
            "node",
            "make",
            "php",
            "lua",
            "awk",
            "jq",
            "swift",
            "julia",
            "pip",
            "shell",
            "osascript",
            "dmd",
            "escript",
            "bun",
            "coffee",
            "crystal",
            "elixir",
            "iex",
            "lsc",
            "gojq",
        ];
        let paths = [
            "/bin/",
            "/usr/bin/env ",
            "/usr/bin/env -S ",
            "/opt/ls/",
            "/home/js/",
            "",
        ];
        let args = ["", " -e", " -f", " -x python", " # make", " # ruby"];
        let mut lines: Vec<String> = Vec::new();
        for i in interpreters {
            for p in paths {
                for a in args {
                    lines.push(format!("#!{p}{i}{a}"));
                    lines.push(format!("#! {p}{}{a}", i.to_ascii_uppercase()));
                }
            }
        }
        // a line each regex of the set matches, and lines close to them
        let samples = [
            "# -*- mode: shell-script -*-",
            "# -*- mode:shell-script; x: 1 -*-",
            "# -*- Mode: shell-script -*-",
            "# -*- makefile -*-",
            "#-*- GNUMakefile -*-",
            "# -*- mode: python -*-",
            "\u{feff}#!/bin/sh",
            "#!/bin/sh é",
            "#! /usr/bin/python3 # é",
            "#!/usr/bin/ma\u{212a}e",
            "# -*- \u{17f}hell-script -*-",
            "// -*- C++ -*-",
            "/* -*- Mode: C -*- */",
            "=== modified file 'x'",
            "==== //depot/a.c#3 - /home/a.c ====",
            "Index: src/x.rs",
            "--- a/x",
            "---",
            "--- %x",
            "*** 1,5 2026",
            "***",
            "12,15c12 ",
            "1. A list",
            "diff --git a/x b/x",
            "%% -*- erlang -*-",
            "[core]",
            "gitdir: ../.git",
            "  gitdir : x",
            "commit 0123456789abcdef",
            "commit message",
            "pick 0123456 msg",
            "f 0123abc x",
            "feat: 0123456",
            "<!DOCTYPE html>",
            "<!doctype HTML>",
            "<html lang=en>",
            "<p align=\"center\">",
            "This is pdfTeX, Version 3.14",
            "; -*- mode: lisp -*-",
            "format: complete",
            "Format:  Complete ",
            "<?php echo 1;",
            "<?phpx",
            "#!/usr/bin/php7",
            "# -*- perl -*-",
            "# -*- ruby -*-",
            "#!/usr/bin/jruby",
            "textile",
            "<?xml version=\"1.0\"?>",
            "<?xml",
            "<?xml\tversion=\"1.0\"?><é/>",
            "<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\">",
            "<!DOCTYPE note SYSTEM \"x.dtd\">",
            "%YAML 1.2",
            "# This is the CMakeCache file.",
            "FROM rust:1.95",
            "  arg VERSION=1",
            "from x import y",
            "\u{3000}FROM x",
            "FROM\u{3000}x",
            "Fro\u{1e3f} x",
            "From here on, text.",
            "// -*- Mode: GLSL -*-",
            "{% extends \"base.html\" %}",
            "syntax = \"proto3\";",
            "# -*- pip-requirements -*-",
            "#vim: set ft=requirements",
            "# Requirements",
            "GET /index.html HTTP/1.1",
            "HTTP/1.1 200 OK",
            "-----BEGIN RSA PRIVATE KEY-----",
            "// -*- Odin -*-",
            "# ada",
            "# Über",
            "{",
            "echo",
            "",
        ];
        for l in samples {
            lines.push(l.to_owned());
            lines.push(l.to_ascii_uppercase());
            lines.push(l.to_ascii_lowercase());
            lines.push(format!(" {l}"));
        }
        let (claimed, _) = first_lines_agree(&lines);
        assert!(claimed > 200, "the table claims {claimed} lines");
    }

    /// [`first_lines_agree_with_the_set`] on real files' first lines, a line each in the file
    /// `ADA_FIRST_LINES` names: e.g. `head -qn1` of every file under `/usr/bin` and a few
    /// checkouts.
    #[test]
    #[ignore]
    fn first_lines_of_real_files_agree() {
        let path = std::env::var("ADA_FIRST_LINES").expect("ADA_FIRST_LINES");
        let bytes = std::fs::read(path).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<String> = text.lines().map(str::to_owned).collect();
        let (claimed, asked) = first_lines_agree(&lines);
        println!(
            "{} lines: {claimed} claimed, {asked} asked the regexes",
            lines.len()
        );
    }

    /// ada-parse's first-line table walks the set's first-line regexes in the set's order, and
    /// ada's list of them (`cargo xtask names`) is in the same: `first_line_asking` asks about
    /// a regex by its place.
    #[test]
    fn the_first_line_table_follows_the_set() {
        let set: Vec<&str> = reference::syntaxes()
            .syntaxes()
            .iter()
            .filter(|s| s.first_line_match.is_some())
            .map(|s| &*s.name)
            .collect();
        let table: Vec<&str> = ada_parse::first_line_syntaxes().collect();
        let listed: Vec<&str> = names::FIRST_LINES.iter().map(|&(name, _)| name).collect();
        assert_eq!(table, set, "ada-parse's first-line table");
        assert_eq!(listed, set, "run `cargo xtask names`");
    }

    /// The `regex` crate reads every first-line regex of the set but XML's and Dockerfile's,
    /// whose lookarounds and back-reference ada-parse's table decides itself.
    #[test]
    fn the_regex_crate_reads_the_first_line_regexes() {
        let refused: Vec<&str> = names::FIRST_LINES
            .iter()
            .filter(|(_, regex)| regex::Regex::new(regex).is_err())
            .map(|&(name, _)| name)
            .collect();
        assert_eq!(refused, ["XML", "Dockerfile"]);
        for name in refused {
            assert!(
                Lang::from_name(name).is_some(),
                "{name}: decided by the table"
            );
        }
    }

    /// Every first-line regex of the set: ada-parse's table holds a test for each. A new one,
    /// or a changed regex, needs a look at that table.
    #[test]
    fn the_first_line_regexes() {
        let regexes: Vec<String> = reference::syntaxes()
            .syntaxes()
            .iter()
            .filter_map(|s| Some(format!("{}: {}", s.name, s.first_line_match.as_ref()?)))
            .collect();
        insta::with_settings!({ snapshot_suffix => "onig" }, {
            insta::assert_snapshot!(regexes.join("\n"));
        });
    }

    /// A file's or a fence's language is named without syntect (ADR-43): the list `cargo
    /// xtask names` writes is the set's names and extensions, in its order.
    #[test]
    fn the_name_list_is_the_sets() {
        let mut from_set = String::new();
        for s in reference::syntaxes().syntaxes() {
            from_set.push_str(&s.name);
            for e in &s.file_extensions {
                from_set.push('\t');
                from_set.push_str(e);
            }
            from_set.push('\n');
        }
        assert!(from_set == names::LIST, "run `cargo xtask names`");
    }

    /// A literal's punctuation takes the literal's color in every built-in theme (HL-005): a
    /// number's `.` and `0x` (past tokyonight's `punctuation.separator` too), a Clojure
    /// keyword's `:`, a character's `?`, an entity's `&` and `;`; a Lisp keyword's `:` as the
    /// keyword (X-009). An escape's keeps its own, and so does `pkg:sym`'s accessor.
    #[test]
    fn a_literals_punctuation_reads_in_its_color() {
        // language, line, the punctuation's byte, a byte of the literal's body
        let whole = [
            ("python", "x = 2.5", 5, 4),
            ("python", "x = 0x1F", 4, 6),
            ("rust", "let x = 0x1F;", 8, 10),
            ("json", "[2.5]", 2, 1),
            ("clojure", "(f :keys)", 3, 4),
            ("elisp", "(f ?a)", 3, 4),
            // X-009: a keyword's `:`, `::` and Racket's `#:`
            ("lisp", "(f :keys ::k)", 3, 4),
            ("lisp", "(f :keys ::k)", 10, 11),
            ("racket", "(f #:kw 1)", 3, 5),
            ("html", "<p>&amp;</p>", 3, 4),
            ("html", "<p>&amp;</p>", 7, 4),
        ];
        let at = |spans: &CodeSpans, byte: u32| {
            let run = spans.line(0).iter().find(|r| r.end > byte).unwrap();
            run.rule
        };
        for theme in Theme::builtins() {
            let rules = SyntaxTheme::new(&theme.syntax);
            for (lang, line, punct, body) in whole {
                let spans = highlight(1, lang, line, &rules);
                let (p, b) = (at(&spans, punct), at(&spans, body));
                assert_eq!(
                    rule_style(p, &theme.syntax),
                    rule_style(b, &theme.syntax),
                    "{}: {lang} {line:?} byte {punct}: {:?} against {:?}",
                    theme.name,
                    theme.syntax.get(usize::from(p.0)),
                    theme.syntax.get(usize::from(b.0)),
                );
            }
            // `pkg:sym`'s `:` is an accessor, not a keyword's
            let spans = highlight(1, "lisp", "(cl:car x)", &rules);
            let rule = at(&spans, 3);
            assert!(
                theme.syntax[usize::from(rule.0)]
                    .0
                    .starts_with("punctuation"),
                "{}: {:?}",
                theme.name,
                theme.syntax[usize::from(rule.0)]
            );
            // Lisp's `~~`: the escape's `~` isn't the constant's
            let spans = highlight(1, "lisp", "(format t \"~~\")", &rules);
            let rule = at(&spans, 11);
            assert!(
                !theme.syntax[usize::from(rule.0)]
                    .0
                    .starts_with("constant.character punctuation"),
                "{}: {:?}",
                theme.name,
                theme.syntax[usize::from(rule.0)]
            );
        }
    }

    /// A Lisp list's parens color by depth (X-030), cycling through the six rules, an opener and
    /// its closer alike, across lines, a parameter list's and Scheme's `[…]` too, Clojure's
    /// vectors and maps; a theme without the rules colors them as `punctuation`, as bat's
    /// grammar does.
    #[test]
    fn lisp_parens_color_by_depth() {
        // seven deep across a line, so the seventh pair is the first's depth again
        let common = "(a (b (c\n(d (e (f (g)))))) (h))\n(defun f (x) (car x))\n";
        let scheme = "(a (b [c\n(d {e (f (g))})]) [h])\n(define (f x) [car x])\n";
        let clojure = "(a (b [c\n(d {:e (f (g))})]) #{h})\n(defn f [x] {:k (car x)})\n";
        let langs = [
            ("lisp", common),
            ("elisp", common),
            ("scheme", scheme),
            ("racket", scheme),
            ("clojure", clojure),
        ];
        for (lang, text) in langs {
            for theme in Theme::builtins() {
                let rules = SyntaxTheme::new(&theme.syntax);
                let spans = highlight(1, lang, text, &rules);
                let mut depth = 0usize;
                for (n, line) in text.lines().enumerate() {
                    for (i, c) in line.bytes().enumerate() {
                        let closes = matches!(c, b')' | b']' | b'}');
                        // Clojure's `#{` is its set's opener whole
                        let set = c == b'#' && line.as_bytes().get(i + 1) == Some(&b'{');
                        if !closes && !set && !matches!(c, b'(' | b'[' | b'{') {
                            continue;
                        }
                        if i > 0 && c == b'{' && line.as_bytes()[i - 1] == b'#' {
                            continue;
                        }
                        depth -= usize::from(closes);
                        let run = spans
                            .line(n as u32)
                            .iter()
                            .find(|r| r.end as usize > i)
                            .unwrap();
                        assert_eq!(
                            theme.syntax[usize::from(run.rule.0)].0,
                            LISP_DEPTHS[depth % 6],
                            "{}: {lang} line {n} byte {i}",
                            theme.name
                        );
                        depth += usize::from(!closes);
                    }
                }
                assert_eq!(depth, 0);
                // without the rules, plain punctuation
                let mut syntax = theme.syntax.clone();
                syntax.retain(|(scope, _)| !LISP_DEPTHS.contains(&scope.as_str()));
                let spans = highlight(1, lang, text, &SyntaxTheme::new(&syntax));
                let rule = spans.line(0)[0].rule;
                assert_eq!(
                    syntax[usize::from(rule.0)].0,
                    "punctuation",
                    "{}",
                    theme.name
                );
            }
        }
    }

    /// The themes' rules that color a Lisp list's parens by depth (X-030).
    const LISP_DEPTHS: [&str; 6] = [
        "meta.depth-1 punctuation",
        "meta.depth-2 punctuation",
        "meta.depth-3 punctuation",
        "meta.depth-4 punctuation",
        "meta.depth-5 punctuation",
        "meta.depth-6 punctuation",
    ];

    /// ada's scope selectors pick syntect's rule (ADR-43), in every built-in theme: for every
    /// kind of every lexer with a fixed set of kinds, under its base scope, and for stacks made
    /// of the themes' own selectors' scopes, which reach the exclusions and the paths.
    #[test]
    fn selectors_pick_syntects_rules() {
        let mut stacks: Vec<Vec<&str>> = Vec::new();
        for lang in Lang::ALL {
            for kind in 0..lang.kinds().len() as u16 {
                let mut stack = vec![lang.base()];
                stack.extend(lang.scopes(kind));
                stacks.push(stack);
            }
        }
        let themes = Theme::builtins();
        let words: Vec<&str> = themes
            .iter()
            .flat_map(|t| &t.syntax)
            .flat_map(|(sel, _)| sel.split([',', '|', ' ', '-']))
            .filter(|w| !w.is_empty())
            .collect();
        for (i, w) in words.iter().enumerate() {
            let next = words[(i * 7 + 3) % words.len()];
            stacks.push(vec!["source.rust", w, next]);
            stacks.push(vec!["text.html", next, "meta.block", w]);
        }
        let mut checked = 0;
        for theme in &themes {
            let ours = SyntaxTheme::new(&theme.syntax);
            let theirs = reference::theme(&ours);
            for stack in &stacks {
                let want = reference::rule(&theirs, stack);
                assert_eq!(ours.rule(stack), want, "{}: {stack:?}", theme.name);
                checked += 1;
            }
        }
        assert!(checked > 10_000, "{checked} stacks");
    }

    fn rule_names(theme: &Theme, spans: &CodeSpans, line: &str, n: u32) -> Vec<(String, String)> {
        let mut start = 0;
        spans
            .line(n)
            .iter()
            .map(|r| {
                // syntect's last run takes the line's `\n` too
                let end = (r.end as usize).min(line.len()).max(start);
                let text = line[start..end].to_owned();
                start = end;
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

    /// The parallel highlighting gives what one thread gives, however the block is cut.
    #[test]
    fn parallel_highlighting_matches_one_thread() {
        let theme = Theme::builtin();
        let syntax = SyntaxTheme::new(&theme.syntax);
        let log =
            "2026-10-17 12:00:01 INFO started pid=42\n2026-10-17 12:00:02 ERROR failed: \"x\"\n";
        let json = "  {\"k\": [1, -2.5e3, true, \"s\\\"\"], \"open\": \"x\n  \"n\": null},\n";
        let diff = "--- a/x\n+++ b/x\n@@ -1 +1 @@ fn main\n-old\n+new\n same\n";
        let ini = "[core]\n; note\neditor = \"vim\" ; the one\n";
        for (lang, unit, reps) in [
            ("log", log, 150),
            ("json", json, 150),
            ("diff", diff, 60),
            ("ini", ini, 100),
        ] {
            let text = unit.repeat(reps);
            let one = highlight(1, lang, &text, &syntax);
            assert!(one.line_count() > 200, "{lang}: highlighted at all");
            for threads in 2..=5 {
                let many = match find_syntax(lang).unwrap() {
                    Syntax::Own(l) if streams(l) => {
                        highlight_parse_on(1, l, &text, &syntax, threads)
                    }
                    _ => panic!("{lang}: on one thread"),
                };
                assert!(many == one, "{lang} on {threads} threads");
            }
        }
    }

    /// A run never stretches into the line before it, and appending
    /// from a line on shifts that line's ends to the runs already there (HL-002).
    #[test]
    fn line_runs_keep_their_lines_apart() {
        let (a, b) = (Rule(3), Rule(5));
        let mut lines = LineRuns::default();
        let mut line = lines.open_line();
        line.push(2, a);
        line.push(4, a);
        line.end();
        let mut line = lines.open_line();
        line.push(1, a);
        line.push(3, b);
        line.end();
        let none = Rule(Rule::NONE);
        lines.open_line().end();
        let mut line = lines.open_line();
        line.push(1, none);
        line.push(3, a);
        line.push(5, none);
        line.end();
        let run = |end, rule| Run { end, rule };
        let want = [
            vec![run(4, a)],
            vec![run(1, a), run(3, b)],
            vec![],
            vec![run(1, none), run(3, a), run(5, none)],
        ];
        let got: Vec<_> = (0..lines.len()).map(|n| lines.line(n).unwrap()).collect();
        assert_eq!(got, want);
        assert_eq!(lines.line(4), None);

        let mut joined = LineRuns::default();
        let mut line = joined.open_line();
        line.push(9, b);
        line.end();
        joined.append(lines, 1);
        let got: Vec<_> = (0..joined.len()).map(|n| joined.line(n).unwrap()).collect();
        assert_eq!(got[0], [run(9, b)]);
        assert_eq!(got[1..], want[1..]);
    }

    /// A block a lexer gets stuck on prints plain, and the blocks after it still highlight
    /// (F-06).
    #[test]
    fn print_leaves_a_block_a_lexer_is_stuck_on_plain() {
        let theme = Arc::new(SyntaxTheme::new(&Theme::builtin().syntax));
        let doc = crate::doc::parse(&format!(
            "```{STUCK}\nx\n```\n\n```rust\nfn main() {{}}\n```\n"
        ));
        let blocks = doc.code_blocks();
        let all = Highlights::compute_all(&blocks, &theme);
        let [stuck, rust] = [0, 1].map(|i| all.get(key(blocks[i]).unwrap()).unwrap());
        assert_eq!(stuck.line_count(), 0);
        assert!(rust.line(0).len() > 1);
    }

    /// With code enough for several threads, the block a lexer gets stuck on prints plain, and
    /// every other block as one thread highlights it (F-06, PRN-01).
    #[test]
    fn print_on_several_threads_leaves_only_the_stuck_block_plain() {
        let theme = Arc::new(SyntaxTheme::new(&Theme::builtin().syntax));
        let mut text = format!("```rust\nfn first() {{}}\n```\n\n```{STUCK}\nx\n```\n\n");
        for i in 0..800 {
            let pad = "x".repeat(i % 90);
            text.push_str(&format!(
                "```rust\nfn f{i}() {{ let s = \"{pad}\"; }}\n```\n\n"
            ));
        }
        let doc = crate::doc::parse(&text);
        let blocks = doc.code_blocks();
        let code: usize = blocks.iter().map(|b| b.text.len()).sum();
        assert!(code > 2 * CODE_PER_THREAD);
        let all = Highlights::compute_all(&blocks, &theme);
        for (i, block) in blocks.iter().enumerate() {
            let key = key(block).unwrap();
            let spans = all.get(key).unwrap();
            if i == 1 {
                assert_eq!(spans.line_count(), 0);
            } else {
                assert!(
                    *spans == highlight(key, "rust", &block.text, &theme),
                    "block {i}"
                );
            }
        }
    }

    /// The worker posts a block a lexer gets stuck on as plain, and a new worker takes the next
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
        worker.send(Request::Job(job(0, STUCK, "x\n"))).unwrap();
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
