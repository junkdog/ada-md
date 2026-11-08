//! Syntax highlighting (ADR-07): syntect with bat's syntax set, run on a worker thread so
//! the first paint never waits for it. The regexes run on Oniguruma by default; building without
//! the `onig` feature keeps them on fancy-regex, pure Rust.
//!
//! Results are color-free. The theme's `[syntax]` table becomes a syntect theme whose rule `i`
//! "colors" a token with `i` encoded in its foreground, so a highlighted line is a list of
//! `(end byte, rule)` runs, and paint resolves each rule against the ada theme (ADR-03).
//! Results are cached by [`key`], a hash of the language and the code.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread;
use std::time::Duration;

use crate::style::Style;
use syntect::highlighting::{
    Color, HighlightIterator, HighlightState, Highlighter as SynHighlighter, ScopeSelectors,
    StyleModifier, Theme as SynTheme, ThemeItem, ThemeSettings,
};
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};

use crate::app::{Computed, Msg};
use crate::doc::{CodeBlock, CodeId};
use crate::layout::{
    block_needs_expanding, expand_block, expand_tabs, expand_tabs_in, expand_tabs_onto,
};
use ada_parse::{FirstLine, Lang};

/// Longer lines are left plain: backtracking regexes can be slow on them, and nobody reads them
/// anyway.
const MAX_LINE: usize = 4096;

/// Cached blocks before the cache starts over.
const CACHE_CAP: usize = 4096;

/// Code a print highlights on each thread of its own: below it, the blocks' time is mostly
/// compiling their grammars' regexes, which a second thread barely shortens (PRN-01).
const CODE_PER_THREAD: usize = 16 * 1024;

/// Lines highlighted so far, by every thread. When it stands still for [`STALL`] while a block
/// is being highlighted, syntect is stuck on a line: its parser can loop forever on a
/// non-consuming push (syntect#656), as on the Scala line `:s| < @@@` (F-06).
static LINES: AtomicU64 = AtomicU64::new(0);

/// How long syntect may take on one line before its block is given up as plain. Real lines take
/// microseconds, and the slowest grammar found so far (O-37) about a second.
const STALL: Duration = Duration::from_millis(if cfg!(test) { 300 } else { 2000 });

/// The syntax set ada highlights with, once loaded.
static SET: OnceLock<SyntaxSet> = OnceLock::new();

/// bat's syntax set, as ada ships it (`assets/`, built by `cargo xtask dump`, ADR-40): two-face's,
/// with the grammars ada's own lexers stand in for emptied. Loaded once, only for a language
/// none of them covers: ~1 ms; syntect compiles each syntax's regexes on first use, which is
/// the expensive part: ~1 ms a language on Oniguruma, up to ~200 ms (TypeScript) on
/// fancy-regex.
pub fn syntaxes() -> &'static SyntaxSet {
    SET.get_or_init(|| {
        #[cfg(feature = "onig")]
        let dump = include_bytes!("../assets/syntaxes-onig.bin");
        #[cfg(not(feature = "onig"))]
        let dump = include_bytes!("../assets/syntaxes-fancy.bin");
        let set = syntect::dumps::from_uncompressed_data(dump).expect("ada's syntax dump");
        drop_read_only_pages();
        set
    })
}

/// Whether [`syntaxes`] has been loaded in this process.
pub fn syntaxes_loaded() -> bool {
    SET.get().is_some()
}

/// bat's whole syntax set, as two-face ships it, the grammars ada's own lexers stand in for
/// still in it: what those lexers are checked against (tests, examples, fuzzing). ada never
/// highlights with it.
pub fn reference_syntaxes() -> &'static SyntaxSet {
    static FULL: OnceLock<SyntaxSet> = OnceLock::new();
    FULL.get_or_init(two_face::syntax::extra_newlines)
}

/// Give back the executable's read-only pages, once the syntax set is loaded (MEM-06). syntect
/// deserializes the set from a 0.92 MiB dump in the binary, read whole once and never again
/// (syntect copies what it keeps), but its pages stay mapped. Nothing names where the dump lies,
/// so this unmaps the whole read-only segment: the pages stay in the page cache, and those read
/// again come back as minor faults. The executable's program headers and the page size come from
/// the auxiliary vector the kernel starts the process with.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn drop_read_only_pages() {
    use std::ffi::c_void;

    #[cfg(target_pointer_width = "64")]
    type Phdr = libc::Elf64_Phdr;
    #[cfg(target_pointer_width = "32")]
    type Phdr = libc::Elf32_Phdr;

    // SAFETY: `getauxval` only reads the auxiliary vector, whose `AT_PHDR` points at the
    // executable's `AT_PHNUM` program headers, mapped as long as the process runs
    let (at, headers, page) = unsafe {
        let at = libc::getauxval(libc::AT_PHDR) as usize;
        let n = libc::getauxval(libc::AT_PHNUM) as usize;
        let page = libc::getauxval(libc::AT_PAGESZ) as usize;
        if at == 0 || page == 0 {
            return;
        }
        (at, std::slice::from_raw_parts(at as *const Phdr, n), page)
    };
    // where the executable was loaded: its `PT_PHDR` lies at `at`, linked at `p_vaddr`
    let Some(base) = headers
        .iter()
        .find(|h| h.p_type == libc::PT_PHDR)
        .map(|h| at - h.p_vaddr as usize)
    else {
        return;
    };
    let read_only = headers
        .iter()
        .filter(|h| h.p_type == libc::PT_LOAD && h.p_flags == libc::PF_R);
    for h in read_only {
        let start = base + h.p_vaddr as usize;
        let end = start + h.p_memsz as usize;
        // whole pages inside the segment only
        let (start, end) = (start.next_multiple_of(page), end / page * page);
        if start < end {
            // SAFETY: the range is mapped read-only from the executable's file, which nothing
            // writes; `MADV_DONTNEED` drops the process's pages, and reads fault them back in
            unsafe {
                libc::madvise(start as *mut c_void, end - start, libc::MADV_DONTNEED);
            }
        }
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn drop_read_only_pages() {}

/// Languages that are deliberately plain.
fn is_plain(lang: &str) -> bool {
    ["text", "txt", "plain", "plaintext", "none"]
        .iter()
        .any(|p| lang.eq_ignore_ascii_case(p))
}

pub use ada_parse::ALIASES;

/// A language ada highlights: by a lexer of its own, which ada-parse's table names without the
/// syntax set, or by a syntax of the set.
#[derive(Clone, Copy, Debug)]
pub enum Syntax {
    Own(Lang),
    Set(&'static SyntaxReference),
}

impl Syntax {
    /// The syntax's name, as syntect names it (`Rust`, `JSON`).
    pub fn name(self) -> &'static str {
        match self {
            Syntax::Own(lang) => lang.name(),
            Syntax::Set(syntax) => &syntax.name,
        }
    }

    /// The name ada gives a whole file in it: [`Syntax::name`], but a Lisp dialect's own
    /// ([`Lang::label`]), which leads back to the dialect where `Lisp` would not.
    pub fn label(self) -> &'static str {
        match self {
            Syntax::Own(lang) => lang.label(),
            Syntax::Set(syntax) => &syntax.name,
        }
    }

    /// A syntax of the set, or the lexer that stands in for it: the set still names the
    /// languages ada's lexers cover, but their grammars in it are empty.
    fn of(syntax: &'static SyntaxReference) -> Self {
        Lang::from_name(&syntax.name).map_or(Syntax::Set(syntax), Syntax::Own)
    }

    /// The set's syntax named `name`, or the lexer that stands in for it; only a syntax ada's
    /// lexers don't cover loads the set.
    fn named(name: &str) -> Option<Self> {
        match Lang::from_name(name) {
            Some(lang) => Some(Syntax::Own(lang)),
            None => syntaxes().find_syntax_by_name(name).map(Syntax::Set),
        }
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

/// The syntax for a fence language, with the aliases the vault needs. Only a language the set
/// has and ada-parse doesn't cover loads the set.
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
    names::find(lang).and_then(Syntax::named)
}

/// Whether highlighting a fence in `lang` needs the syntax set: a language the set has and
/// ada-parse doesn't cover.
pub fn needs_syntaxes(lang: &str) -> bool {
    !is_plain(lang)
        && Lang::from_token(lang).is_none()
        && names::find(lang).is_some_and(|name| Lang::from_name(name).is_none())
}

/// The language of a whole file, by its name alone: the name itself (`Makefile`, `.zshrc`,
/// `Cargo.lock`), then its extension. `None` when that says nothing, or says plain text. Never
/// loads the set.
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

/// The language a file's first line declares: a `#!` line, `<?xml`, a vim modeline. Only a
/// line ada-parse can't tell about loads the set.
pub fn first_line_language(line: &str) -> Option<&'static str> {
    match Lang::first_line(line) {
        FirstLine::Lang(lang) => Some(lang.label()),
        FirstLine::None => None,
        FirstLine::Ask => syntaxes()
            .find_syntax_by_first_line(line)
            .map(Syntax::of)
            .and_then(language),
    }
}

fn language(syntax: Syntax) -> Option<&'static str> {
    Some(syntax.label()).filter(|&n| n != "Plain Text")
}

/// The set's syntaxes by name and extension, as `cargo xtask dump` lists them beside the set
/// (`assets/syntax-names-*.txt`): syntect's lookups without loading the set.
mod names {
    use ada_parse::{ALIASES, Lang};

    /// A line a syntax, in the set's order: its name, then its extensions, tab-separated.
    #[cfg(feature = "onig")]
    pub(super) static LIST: &str = include_str!("../assets/syntax-names-onig.txt");
    #[cfg(not(feature = "onig"))]
    pub(super) static LIST: &str = include_str!("../assets/syntax-names-fancy.txt");

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

/// An index into the theme's `[syntax]` rules, and a highlighted run: bytes up to `end` of the
/// tab-expanded line take `rule`. ada-parse's, which its lexers write (X-068).
pub use ada_parse::{Rule, Run};

/// The rule a style's foreground stands for ([`SyntaxTheme::new`]'s encoding).
fn decode_rule(c: Color) -> Rule {
    match c.a {
        0 => Rule(Rule::NONE),
        _ => Rule(u16::from(c.r) << 8 | u16::from(c.g)),
    }
}

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

/// A code block's text highlighted by syntect with `set`'s syntax `name`, line by line as
/// [`highlight`] goes; `None` when the set has no such syntax or syntect fails on a line. For
/// holding ada's own lexers against bat's grammars ([`reference_syntaxes`]).
pub fn highlight_syntect(
    set: &'static SyntaxSet,
    name: &str,
    text: &str,
    theme: &SyntaxTheme,
) -> Option<CodeSpans> {
    let syntax = set.find_syntax_by_name(name)?;
    let highlighter = SynHighlighter::new(&theme.theme);
    let mut state = LineState::new(set, syntax, &highlighter);
    let (mut buf, mut lines) = (String::new(), LineRuns::default());
    for line in Lines::new(text) {
        state.line(line, &highlighter, &mut buf, &mut lines)?;
    }
    Some(CodeSpans {
        key: 0,
        rules: theme.id,
        partial: false,
        lines: lines.shrink(),
    })
}

/// Where syntect is between two lines: what the next line's highlighting depends on, besides
/// the line.
struct LineState {
    set: &'static SyntaxSet,
    parse: ParseState,
    scopes: HighlightState,
}

impl LineState {
    fn new(
        set: &'static SyntaxSet,
        syntax: &SyntaxReference,
        highlighter: &SynHighlighter,
    ) -> Self {
        Self {
            set,
            parse: ParseState::new(syntax),
            scopes: HighlightState::new(highlighter, ScopeStack::new()),
        }
    }

    /// Highlight one line into `out`, moving to the state after it. `None` when syntect fails,
    /// with `out` as it was.
    fn line(
        &mut self,
        line: &str,
        highlighter: &SynHighlighter,
        buf: &mut String,
        out: &mut LineRuns,
    ) -> Option<()> {
        // expanded straight into the buffer syntect reads, with no copy between (HL-004)
        buf.clear();
        expand_tabs_onto(line, buf);
        let len = buf.len();
        if len > MAX_LINE {
            out.open_line().end();
            return Some(());
        }
        buf.push('\n');
        let ops = self.parse.parse_line(buf, self.set).ok()?;
        LINES.fetch_add(1, Ordering::Relaxed);
        let mut out = out.open_line();
        let mut end = 0;
        for (style, piece) in HighlightIterator::new(&mut self.scopes, &ops, buf, highlighter) {
            let start = end;
            end = (end + piece.len()).min(len);
            if end == start {
                continue;
            }
            out.push(end as u32, decode_rule(style.foreground));
        }
        out.end();
        Some(())
    }
}

/// The lexer of ada's own that highlights a language, if one does: logs (O-43), JSON (O-48),
/// Makefile (G-002), bash (G-003), or one of ada-parse's (ADR-40).
fn own(lang: &str) -> Option<Lang> {
    match find_syntax(lang)? {
        Syntax::Own(lang) => Some(lang),
        Syntax::Set(_) => None,
    }
}

/// A kind's place in [`ParseRules`] before its rule is resolved. No theme has this many rules.
const UNSET: Rule = Rule(Rule::UNSET);

/// The kinds' rules resolved so far, by language and theme (`SyntaxTheme::id`), for the next
/// block in them to start from: resolving takes syntect's scope repository and a selector
/// match for each kind, a sixth of a YAML corpus's time when each block did it anew (HL-004).
/// Shared, not copied, by the blocks that start from it: a table runs to the language's
/// highest kind (C#'s and Python's embedded regexes and SQL put theirs past 30,000), and
/// copying it cost a C# block ~66,000 instructions, as many as lexing 40 of its lines (X-022).
static RESOLVED: Mutex<Vec<Resolved>> = Mutex::new(Vec::new());

/// A language's and a theme's (`SyntaxTheme::id`) rules by kind in [`RESOLVED`].
type Resolved = (Lang, u64, Arc<[Rule]>);

/// Languages times themes kept in [`RESOLVED`]; past that it starts over.
const RESOLVED_MAX: usize = 64;

/// The theme's rule for each kind of an ada-parse language (ADR-40), as syntect would pick it
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
        let highlighter = SynHighlighter::new(&theme.theme);
        let mut rules = Self::lazy(lang, theme.id);
        rules.rules = (0..lang.kinds().len() as u16)
            .map(|k| rules.resolve(k, &highlighter))
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

    fn resolve(&self, kind: u16, highlighter: &SynHighlighter) -> Rule {
        // the base scope here, not a block's set-up: syntect's scope repository is behind a lock
        let mut stack = vec![Scope::new(self.lang.base()).expect("scope")];
        stack.extend(
            self.lang
                .scopes(kind)
                .iter()
                .map(|s| Scope::new(s).expect("scope")),
        );
        let rule = decode_rule(highlighter.style_for_stack(&stack).foreground);
        debug_assert_ne!(rule, UNSET);
        rule
    }

    /// Resolve `kind`'s rule into `rules`, the first time a span has it.
    #[cold]
    #[inline(never)]
    fn resolve_into(&mut self, kind: u16, highlighter: &SynHighlighter) -> Rule {
        let (k, len) = (usize::from(kind), self.rules.len());
        let rule = self.resolve(kind, highlighter);
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
        highlighter: &LazyHighlighter,
        out: &mut LineRuns,
    ) -> Option<usize> {
        let lang = self.lang;
        // the rule looked up once a span, resolved in the same pass when it's new (HL-004)
        let rules = &mut Resolving {
            rules: self,
            highlighter,
        };
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
struct Resolving<'a, 'h> {
    rules: &'a mut ParseRules,
    highlighter: &'a LazyHighlighter<'h>,
}

impl ada_parse::Rules for Resolving<'_, '_> {
    fn table(&self) -> &[Rule] {
        &self.rules.rules
    }

    fn resolve(&mut self, kind: u16) -> Rule {
        self.rules.resolve_into(kind, self.highlighter.get())
    }
}

/// How a file printed a piece at a time is highlighted (O-52): not at all, or by one of ada's
/// own lexers. A syntect syntax's state carries from line to line and can't move between
/// threads, so none streams (O-053).
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
    /// Whether a file in `lang`, or plain text with none, has a lexer.
    pub fn exists(lang: Option<&str>) -> bool {
        lang.is_none_or(|l| is_plain(l) || own(l).is_some())
    }

    /// The lexer for a file in `lang`, or plain text with none; `None` when syntect would
    /// highlight it.
    pub fn new(lang: Option<&str>, theme: &SyntaxTheme) -> Option<Self> {
        match lang {
            None => Some(Lexer::Plain),
            Some(lang) if is_plain(lang) => Some(Lexer::Plain),
            Some(lang) => match own(lang)? {
                l if streams(l) => Some(Lexer::Parse(ParseRules::new(l, theme))),
                l => Some(Lexer::Carry(l)),
            },
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
    highlighter: LazyHighlighter<'a>,
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
            highlighter: LazyHighlighter {
                theme,
                built: OnceCell::new(),
            },
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
                &self.highlighter,
                &mut lines,
            );
        }
        let spans = CodeSpans {
            key,
            rules: self.highlighter.theme.id,
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
    Syntect(LineState),
    /// Also whether the block's text has a control character, which `expand_tabs` changes.
    Parse(ParseRules, ada_parse::State, Vec<ada_parse::Span>, bool),
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

/// syntect's highlighter for a theme, built the first time it's needed: an ada-parse block
/// needs it only for a kind no block before it resolved (HL-004: building it sorts the
/// theme's selectors, a few percent of a corpus's time when every block did).
struct LazyHighlighter<'a> {
    theme: &'a SyntaxTheme,
    built: OnceCell<SynHighlighter<'a>>,
}

impl<'a> LazyHighlighter<'a> {
    fn get(&self) -> &SynHighlighter<'a> {
        self.built
            .get_or_init(|| SynHighlighter::new(&self.theme.theme))
    }
}

/// Highlights a block a few lines at a time, carrying syntect's state from one step to the
/// next, so the worker can post the first lines early and stop between steps (O-05).
pub struct Highlighter<'a> {
    key: u64,
    rules: u64,
    highlighter: LazyHighlighter<'a>,
    /// `None` once the block turns out plain, or syntect fails on it.
    state: Option<Engine>,
    rest: Lines<'a>,
    lines: LineRuns,
    buf: String,
}

impl<'a> Highlighter<'a> {
    pub fn new(key: u64, lang: &str, text: &'a str, theme: &'a SyntaxTheme) -> Self {
        let highlighter = LazyHighlighter {
            theme,
            built: OnceCell::new(),
        };
        let BlockScratch { spans, lines, buf } = BlockScratch::take();
        let state = find_syntax(lang).map(|syntax| match syntax {
            Syntax::Own(lang) => Engine::Parse(
                ParseRules::cached(lang, theme.id),
                ada_parse::State::default(),
                spans,
                block_needs_expanding(text),
            ),
            Syntax::Set(s) => Engine::Syntect(LineState::new(syntaxes(), s, highlighter.get())),
        });
        Self {
            key,
            rules: theme.id,
            state,
            highlighter,
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
                        &self.highlighter,
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
            Engine::Syntect(state) => {
                for _ in 0..n {
                    let Some(line) = self.rest.next() else {
                        break;
                    };
                    if state
                        .line(line, self.highlighter.get(), &mut self.buf, out)
                        .is_none()
                    {
                        // plain, as a whole
                        self.state = None;
                        self.lines.clear();
                        return false;
                    }
                }
            }
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
    match syntax {
        // a `/* */` comment carries over lines: rare in JSON this big, and fast on one thread
        Syntax::Own(Lang::Json) if text.contains("/*") => highlight(key, lang, text, theme),
        // without one, its lines don't depend on each other
        Syntax::Own(Lang::Json) => highlight_parse_on(key, Lang::Json, text, theme, threads),
        Syntax::Own(own) if own.line_by_line() => {
            highlight_parse_on(key, own, text, theme, threads)
        }
        Syntax::Own(_) => highlight(key, lang, text, theme),
        Syntax::Set(syntax) => highlight_on(key, syntax, text, theme, threads),
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
    let mut lines = LineRuns::default();
    let mut k = 0;
    loop {
        let Some(out) = outs.get_mut(k).and_then(Option::take) else {
            // syntect failed on a line: plain, as a whole
            lines.clear();
            break;
        };
        lines.append(out.lines, out.from);
        lines.append(out.over, 0);
        match out.meet {
            Some(m) => k = m,
            None => break,
        }
    }
    CodeSpans {
        key,
        rules: theme.id,
        partial: false,
        lines: lines.shrink(),
    }
}

/// What a chunk's thread found. Its lines from `from` on are true, then `over`, the lines it
/// highlighted after its chunk, run up to chunk `meet`'s checkpoint, or to the block's end.
struct Chunk {
    from: usize,
    lines: LineRuns,
    over: LineRuns,
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
    let mut state = LineState::new(syntaxes(), syntax, &highlighter);
    let mut buf = String::new();
    let mut lines = LineRuns::default();
    lines.ends.reserve_exact(chunks[k].len());
    let mut checkpoints = Vec::new();
    let mut failed = false;
    for (i, line) in chunks[k].iter().enumerate() {
        if i > 0 && i % CHECKPOINT_EVERY == 0 {
            checkpoints.push(fingerprint(&state));
        }
        if state
            .line(line, &highlighter, &mut buf, &mut lines)
            .is_none()
        {
            failed = true;
            break;
        }
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
            lines: LineRuns::default(),
            over: LineRuns::default(),
            meet: None,
        });
    };
    if failed {
        // the failing line comes after the checkpoint the true state met, so it fails for true
        seams.fail();
        return None;
    }
    let mut over = LineRuns::default();
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
            if state
                .line(line, &highlighter, &mut buf, &mut over)
                .is_none()
            {
                seams.fail();
                return None;
            }
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
        // the syntax set loads with the first block ada's own lexers don't cover, not before:
        // a note with none never loads it (ADR-40)
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
        let set = reference_syntaxes();
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
        let by_name = name.and_then(|n| reference_syntaxes().find_syntax_by_extension(n));
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
        for s in reference_syntaxes().syntaxes() {
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
    /// and misses none of the ones it covers; and ada, the table first and its trimmed set
    /// after, names what the whole set names: as a fence's token, a file's whole name and its
    /// extension. Two departures are by design (ADR-40): ada's JavaScript is Babel's grammar
    /// whatever the token, where the set gives Sublime's `JavaScript` (```` ```JavaScript ````,
    /// and in the fancy build, whose set has no Babel, `js` itself) or nothing (the fancy
    /// build's `mjs`, `jsx`, …); and the fancy build's set has no PowerShell (its regexes need
    /// Oniguruma), where ada's own lexer needs none.
    #[test]
    fn the_language_table_agrees_with_the_set() {
        let covered = |name: Option<&str>| name.and_then(Lang::from_name);
        let fancy_only = |ours: Option<&str>, full: Option<&str>| {
            !cfg!(feature = "onig") && full.is_none() && ours == Some(Lang::PowerShell.name())
        };
        let departs = |ours: Option<&str>, full: Option<&str>| {
            fancy_only(ours, full)
                || ours == Some(Lang::JavaScript.name())
                    && (full == Some("JavaScript") || (!cfg!(feature = "onig") && full.is_none()))
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
    /// own, or no syntax at all; and ada's answer is syntect's.
    fn first_lines_agree(lines: &[String]) -> (usize, usize) {
        let full = |l: &str| {
            let found = reference_syntaxes().find_syntax_by_first_line(l);
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
            "{} lines: {claimed} claimed, {asked} asked the set",
            lines.len()
        );
    }

    /// ada-parse's first-line table walks the set's first-line regexes in the set's order.
    /// The table may list a syntax the set lacks (the fancy build has no `JavaScript (Babel)`
    /// regex), which only sends more lines to the set.
    #[test]
    fn the_first_line_table_follows_the_set() {
        let set: Vec<&str> = reference_syntaxes()
            .syntaxes()
            .iter()
            .filter(|s| s.first_line_match.is_some())
            .map(|s| &*s.name)
            .collect();
        let mut table = ada_parse::first_line_syntaxes();
        for name in &set {
            assert!(
                table.any(|t| t == *name),
                "{name}: not in ada-parse's first-line table, or out of the set's order"
            );
        }
    }

    /// Every first-line regex of the set: ada-parse's table holds a test for each. A new one,
    /// or a changed regex, needs a look at that table.
    #[test]
    fn the_first_line_regexes() {
        let regexes: Vec<String> = reference_syntaxes()
            .syntaxes()
            .iter()
            .filter_map(|s| Some(format!("{}: {}", s.name, s.first_line_match.as_ref()?)))
            .collect();
        // two-face rewrites a few regexes for fancy-regex
        let flavor = if cfg!(feature = "onig") {
            "onig"
        } else {
            "fancy"
        };
        insta::with_settings!({ snapshot_suffix => flavor }, {
            insta::assert_snapshot!(regexes.join("\n"));
        });
    }

    /// A file's or a fence's language is named without the set (`file_language` never loads
    /// it, `find_syntax` only for a syntax ada's lexers don't cover): the list `cargo xtask
    /// dump` writes beside the set is the set's names and extensions, in its order.
    #[test]
    fn the_name_list_is_the_sets() {
        let mut from_set = String::new();
        for s in syntaxes().syntaxes() {
            from_set.push_str(&s.name);
            for e in &s.file_extensions {
                from_set.push('\t');
                from_set.push_str(e);
            }
            from_set.push('\n');
        }
        assert!(from_set == names::LIST, "run `cargo xtask dump`");
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
        rules_match_the_grammar("json", "JSON", text);
    }

    /// ada-parse's diff lexer picks the rules bat's grammar picks, in every built-in theme
    /// (ADR-40).
    #[test]
    fn diff_rules_match_the_grammar() {
        let text = concat!(
            "diff --git a/src/x.rs b/src/x.rs\n",
            "Index: src/x.rs\n",
            "===================================================================\n",
            "--- a/src/x.rs\t2026-10-31\n",
            "+++ b/src/x.rs\n",
            "@@ -1,3 +1,4 @@ fn main() {\n",
            " context\n",
            "-removed\n",
            "+added\n",
            "@@  @@\n",
            "***************\n",
            "*** 1,5 ****\n",
            "! changed\n",
            "--- 1,5 ----\n",
            "12,15c12\n",
            "< old\n",
            "---\n",
            "> new\n",
            "==== //depot/a.c#3 - /home/a.c ====\n",
        );
        rules_match_the_grammar("diff", "Diff", text);
    }

    /// ada-parse's INI lexer picks the rules bat's grammar picks, in every built-in theme
    /// (ADR-40).
    #[test]
    fn ini_rules_match_the_grammar() {
        let text = concat!(
            "; comment\n",
            "  # another\n",
            "[section] ; why\n",
            "[a]b]\n",
            "key = value ; note\n",
            "\"quoted key\" = 'quoted ; value' ; c\n",
            "  = x\n",
            "bare words\n",
            "\n",
        );
        rules_match_the_grammar("ini", "INI", text);
    }

    /// ada-parse's TOML lexer picks the rules bat's grammar picks, in every built-in theme
    /// (ADR-40), on text where it reads TOML as bat's grammar does. It reads TOML 1.1 where the
    /// grammar doesn't (X-020): `truex` is one bad value where the grammar gives `true` and a bad
    /// `x`, and the blanks after a value are no error, so `= ] x` is two bad values where the
    /// grammar makes `] x` one (`toml_scan.rs`'s tests hold those departures).
    #[test]
    fn toml_rules_match_the_grammar() {
        let text = concat!(
            "# comment\n",
            "[package]\n",
            "name = \"ada-md\" # why\n",
            "version.workspace = true\n",
            "\"quoted key\" = 'literal'\n",
            "[[bin]] # array of tables\n",
            "[ a . \"b\" . 'c' ]\n",
            "dates = [1979-05-27T07:32:00Z, 1979-05-27 07:32:00.999, 1979-05-27, 07:32:00]\n",
            "nums = [ +1, -0x_1, 0xdead_BEEF, 0o17, 0b101, 1_000, 3.14, -2e-3, 6.0E+2, inf, -nan, 01 ]\n",
            "odd = [ 0o0k, 0x1g, x0b1, 1a ]\n",
            "inline = { a = 1, b.c = \"x\\n\\q\", d = [ ] }\n",
            "multi = \"\"\"\n",
            "  a \\u00e9 b\n",
            "  \"\"\"\n",
            "lit = '''\n",
            "raw \\n'''\n",
            "nested = [\n",
            "  [1, 2], # c\n",
            "  { x = 1 },\n",
            "  \"s\",\n",
            "]\n",
            "bad = ]\n",
            "[]\n",
            "key\n",
            "= 1\n",
            "a = \"unterminated\n",
            "b = yes\n",
        );
        rules_match_the_grammar("toml", "TOML", text);
    }

    /// ada-parse's XML lexer picks the rules bat's grammar picks, in every built-in theme
    /// (ADR-40).
    #[test]
    fn xml_rules_match_the_grammar() {
        let text = concat!(
            "<?xml version=\"1.0\" encoding='UTF-8'?>\n",
            "<?xml-stylesheet href=\"a.xsl\"?>\n",
            "<?php echo 1; ?>\n",
            "<!DOCTYPE note SYSTEM \"note.dtd\" [\n",
            "  <!ELEMENT note (to,from?,(a|b)*)>\n",
            "  <!ELEMENT br EMPTY>\n",
            "  <!ATTLIST note id ID #REQUIRED kind (x|y) \"x\" ref IDREFS #IMPLIED>\n",
            "  <!ENTITY % param \"value\">\n",
            "  <!ENTITY pic SYSTEM \"p.gif\" NDATA gif>\n",
            "  <!NOTATION gif PUBLIC \"image/gif\">\n",
            "  %param;\n",
            "  <!BOGUS x>\n",
            "]>\n",
            "<!doctype html>\n",
            "<ns:root xmlns:ns=\"urn:x\" a='1' b=bare c = \"&amp;&#65;&#x41;&bad\">\n",
            "  <!-- a comment -- with -- dashes -->\n",
            "  <![CDATA[ raw <text> ]]>\n",
            "  <item key=\"v\"/>text &lt; & more < x\n",
            "  <a:b:c d:e:f=\"1\" ?>\n",
            "  <bad$name 1x=\"2\" / >\n",
            "  <multi\n",
            "    attr=\"v\"\n",
            "  >\n",
            "</ns:root>\n",
            "<!-- open\n",
            "comment -->\n",
        );
        rules_match_the_grammar("xml", "XML", text);
    }

    /// ada-parse's YAML lexer picks the rules bat's grammar picks, in every built-in theme
    /// (ADR-40).
    #[test]
    fn yaml_rules_match_the_grammar() {
        let text = concat!(
            "%YAML 1.2\n",
            "%TAG !e! tag:example.com,2000:app/\n",
            "%FOO bar baz # c\n",
            "---\n",
            "# a comment\n",
            "name: ada # trailing\n",
            "on: yes\n",
            "nums: [0b101, 0o17, 017, 019, -42, 0x_ff, 1:30, 1_000, .5, -1.5e+3, 1.2.3, .inf, -.Inf, .NaN, 1:30.5]\n",
            "misc: {a: ~, b: null, c: =, <<: *base, ? d : e}\n",
            "when: 2001-12-14t21:59:43.10-05:00\n",
            "day: 2002-12-14\n",
            "spaced: 2001-12-14 21:59:43.10 -5 # tz\n",
            "loose: 2001-1-2 3:04:05   \n",
            "anchor: &base !!map\n",
            "  key with spaces: value with: colon\n",
            "  \"quoted\": 'it''s'\n",
            "  - item\n",
            "  - \"esc \\n \\x41 \\q\"\n",
            "  -not a dash\n",
            "tagged: !<tag:x> v\n",
            "local: !e!thing v\n",
            "bad: &a[b *c]d !\n",
            "literal: |2-\n",
            "    indented\n",
            "\n",
            "   still\n",
            "  back\n",
            "folded: >\n",
            "  text\n",
            "  # not a comment\n",
            "next: x\n",
            "flow: [a, {b: c}, d: e, \"f\": g\n",
            "  , h]\n",
            "multi: \"line one\n",
            "  line two\\\n",
            "  end\"\n",
            "? complex\n",
            ": value\n",
            "...\n",
        );
        rules_match_the_grammar("yaml", "YAML", text);
    }

    /// ada-parse's Rust scanner (X-090's, of ada's own) picks the rules bat's grammar picks, in
    /// every built-in theme (ADR-40), run for run: its odd corners included (a one-digit width in
    /// a placeholder, `write!`'s first argument, `impl` names past `for`). Not `writeln!(out)`
    /// without a format string, whose first-argument context never ends in bat's and derails the
    /// lines after, nor `if a<b {`, whose `{` bat's reads as invalid (X-090, by design). Not
    /// `?Sized`: bat's grammar marks its `?` invalid and ends the generic there, which ada's type
    /// frames don't (X-056, by design; ada-parse's `rust::tests::types`); nor a tuple struct's
    /// `pub` or a group in its fields, whose first `)` ends them in bat's (X-063, by design;
    /// `rust::tests::tuple_fields`); nor a trait's or an enum's generics and supertraits, or a
    /// struct's `where`, which bat's grammar leaves plain (X-064, by design;
    /// `rust::tests::headers`).
    #[test]
    fn rust_rules_match_the_grammar() {
        let text = concat!(
            "//! crate docs $x\n",
            "#![allow(dead_code)]\n",
            "#[derive(Debug, Clone)] #[cfg(not(feature = \"x\"))]\n",
            "pub(crate) struct S<'a, T: Sized> { pub a: &'a T, b: Vec<[u8; 4]> }\n",
            "struct Tuple(u32, Option<u8>);\n",
            "enum E { A(u8), B { x: i32 }, C = 3 }\n",
            "trait Tr { type Out; fn f(&self) -> Self::Out; }\n",
            "impl<'a, T> fmt::Display for S<'a, T> where T: Copy {\n",
            "    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {\n",
            "        write!(f.out(), \"{:>8} {:10} {0:.2} {name:?} {{}}\", self.a)?;\n",
            "        println!(\"{:#?} {x:08x}\", 1u8);\n",
            "        let r = r##\"raw \"# still\"##; let b = b\"\\x00\\q\"; let c = ['\\n', b'a', '\\u{1F600}'];\n",
            "        let f = |x: u32, (a, b): (u8, u8)| -> u32 { x + 1 };\n",
            "        'outer: loop { break 'outer; }\n",
            "        let v: Vec<_> = xs.iter().map(|x| x * 2).collect::<Vec<_>>();\n",
            "        match self { Self::A if a >= 0x1F_u8 => 1.5e-3f64, _ => 0o17 as f64 + 0b1010 as f64 };\n",
            "        unsafe { *ptr = MAX_LEN << 2; std::mem::drop(x); }\n",
            "        /* block /* nested */ still */ /** doc\n",
            "          * star\n",
            "          */ x..=y; a != b && !c || d\n",
            "    }\n",
            "}\n",
            "macro_rules! m { ($x:expr, $($y:ident),*) => { $x + 1 }; }\n",
            "type Alias = fn(u8) -> u8;\n",
            "mod tests { use super::*; }\n",
            "const C_T: c_int = 0; static mut size_t: usize = 1.;\n",
            "fn g<T>(\n",
            "    a: impl Iterator<Item = u8>,\n",
            ") -> Box<dyn Fn() -> u8 + Send> where T: Default {}\n",
        );
        rules_match_the_grammar("rust", "Rust", text);
    }

    /// ada-parse's Kotlin lexer picks the rules bat's grammar picks, in every built-in theme
    /// (ADR-40): its odd corners included (`type` inside a word opening a typedef, a word before
    /// `<` opening type arguments, a function's `}` closing its class, a header's every word its
    /// class's name, backtick names past ASCII).
    #[test]
    fn kotlin_rules_match_the_grammar() {
        let text = concat!(
            "package sift.core.api // the api\n",
            "import kotlin.reflect.KClass as K\n",
            "import java.util.*\n",
            "/* block\n",
            "   comment */ @Suppress(\"UNCHECKED_CAST\")\n",
            "data class Foo<T : Comparable<T>, out R>(val x: Int, var y: List<String> = listOf()) : Bar(x), Baz {\n",
            "    companion object { const val MAX = 0x1FL; private val e = 1.5e-3f }\n",
            "    override fun <R> List<T>.map(f: (T) -> R, vararg n: Int): Map<String, R>? = TODO()\n",
            "    val name: String get() = \"x\"\n",
            "    var size: Int\n",
            "        get() { return field }\n",
            "        set(value) { field = value }\n",
            "    fun `a name — with an arrow → too`() {\n",
            "        val s = \"a $x ${y.z} \\n \\u00e9\" + \"\"\"raw $w\n",
            "            still\"\"\".trimIndent() + 'c' + '\\t'\n",
            "        if (a?.b ?: 0 >= 1 && c !is D || !e) return null else x++\n",
            "        for (i in 0 until n) if (i<n) y += i * 2 % 3 / 4 - 1\n",
            "        when (val v = q) { is Int -> v; else -> throw IllegalStateException() }\n",
            "        try { a::class.java; Foo::bar } catch (e: Exception) { } finally { }\n",
            "        val prototype = isPrototype && a < b; 1.toString(); 1..10; x.5\n",
            "        typealias Fn = (Int) -> Unit\n",
            "        val lazyValue: String by lazy { \"v\" }; array[0] = `weird name`\n",
            "    }\n",
            "    interface I { fun f(a: Array<Int>, b: IntArray): Unit }\n",
            "}\n",
            "enum class E { A, B; fun x() = A }\n",
            "sealed interface S\n",
            "fun interface Op { fun run() }\n",
            "object Single : Any() { suspend inline fun <reified T> t(): T? = null }\n",
            "val x = 100<200; val y = 1_000; val z = 1e10; val w = .5\n",
        );
        rules_match_the_grammar("kotlin", "Kotlin", text);
    }

    /// ada-parse's Java lexer picks the rules bat's grammar picks, in every built-in theme
    /// (ADR-40): its odd corners included (an annotation's line end and the next line's indent,
    /// `default ->` a lambda's parameter, `%>` emptying the stack, Javadoc's HTML, an unknown
    /// block tag and a tag at a line's first byte leaving the comment plain).
    #[test]
    fn java_rules_match_the_grammar() {
        let text = concat!(
            "package org.example.app; // the app\n",
            "import java.util.*;\n",
            "import static org.junit.Assert.assertEquals;\n",
            "/** Doc with <a href=\"x\" id=y class='z'>link</a>, <my-tag Bad>x</my-tag>, <!-- c -->\n",
            " * &amp; &#169; &bad <> a<b List<String> {@code a {b} c} {@inheritDoc} {@value Foo#BAR}\n",
            " * {@linkplain java.util.List#add(Object, int) the add}\n",
            " * @see Foo#bar(int, String)\n",
            " * @hidden unknown\n",
            " *   so this stays <b>plain</b>\n",
            " * @param <T> the type\n",
            "@param at the line's start\n",
            " * @return the {@code value\n",
            " *   across} lines */\n",
            "@SuppressWarnings({\"unchecked\", \"rawtypes\"})\n",
            "@org.example.Ann(value = 1, other = @Inner(x = \"y\"), arr = {1, 2})\n",
            "public abstract class Foo<T extends Comparable<? super T> & Serializable> extends com.x.Bar<T> implements Baz {\n",
            "    private static final long A = 0x1FL, B = 0b1010_1010L, C = 0_17, D = 09, E = 1.5e-3f, F = .5d, G = 0x1.8p3;\n",
            "    protected int[] arr = new int[] {1, 2}, more[] = {};\n",
            "    char c = '\\'', d = '\\u00e9'; String s = \"unterminated\n",
            "    static { init(); }\n",
            "    @Override\n",
            "    public <R> List<R> map(final java.util.function.Function<? super T, ? extends R> f, int... xs) throws IOException {\n",
            "        var list = new ArrayList<String>(10); Runnable r = () -> { System.out.println(\"x\"); };\n",
            "        list.forEach(e -> f(e)); list.stream().map(String::valueOf); Foo.<String>bar(); Foo.class; T.x;\n",
            "        int x = a < b ? -1 : 1, y = x >>> 2; x >>= 1; x %= 2; boolean q = !(x != y) && x == y || ~x > 0;\n",
            "        if (o instanceof String s) { return null; } else { }\n",
            "        switch (x) { case 1: break; case FOO -> g(); default -> {} default: continue; }\n",
            "        for (int i = 0; i < 10; i++) { } try (var in = open(f)) { } catch (IOException | RuntimeException e) { } finally { }\n",
            "        assert x > 0 : \"msg\"; synchronized (this) { this.x = super.y; }\n",
            "        Object o = new Object() { @Override public String toString() { return \"\"; } };\n",
            "        goto label; const int K = 1; 1abc(); $x = 1; foo$bar(); x = 1.foo;\n",
            "    }\n",
            "    public @interface Ann { String value() default \"x\"; Class<?> c() default Object.class; }\n",
            "    enum Color { RED, GREEN(\"g\") { void x() {} }, BLUE;\n",
            "        private final int v;\n",
            "    }\n",
            "    record P(int x, int y) implements I { }\n",
            "    x = y %> z;\n",
            "}\n",
            "/* block /* not nested */ /** doc */ /***/ /**/ // line /* with a block\n",
            "module com.example.mod { requires transitive java.base; exports com.example.api to other.mod; provides a.Spi with a.Impl; }\n",
        );
        rules_match_the_grammar("java", "Java", text);
    }

    /// ada-parse's C++ scanner (X-057's, of ada's own) colors each byte as bat's grammar does,
    /// in every built-in theme (ADR-40, by rule a byte, not run for run: its kinds are leaves):
    /// the preprocessor's rules (a disabled branch a comment, `#if 1`'s else branch, `#pragma
    /// mark`), an upper-case macro call on its own line, raw strings across lines, the class,
    /// method and function definitions' corners (a friend class, a trailing return type,
    /// `delete[]`). Not where it departs from bat by design (ada-parse's `cc` tests): an enum's
    /// underlying type and a base list's type words colored as types, where bat leaves them
    /// plain; a friend function's name and parameters a declaration's, where bat leaves them
    /// plain; a lambda's `-> int` a type, where bat reads a member.
    #[test]
    fn cpp_rules_match_the_grammar() {
        let text = concat!(
            "#include <vector>\n",
            "#include \"local.h\" // why\n",
            "#define MAX(a, b) ((a) > (b) ? (a) : (b))\n",
            "#define NAME \"x\" \\\n",
            "  \"continued\"\n",
            "#if 0\n",
            "int disabled(); #if nested\n",
            "#endif\n",
            "#else\n",
            "int enabled;\n",
            "#endif\n",
            "#if 1\n",
            "int on;\n",
            "#else\n",
            "int off;\n",
            "#endif\n",
            "#ifdef __cplusplus\n",
            "extern \"C\" {\n",
            "#endif\n",
            "/* = banner = */\n",
            "namespace foo::bar {\n",
            "using namespace std;\n",
            "using Alias = std::map<int, std::string>;\n",
            "template <typename T, class... Rest> class Widget final : public Base<T>, private virtual Other {\n",
            " public:\n",
            "  Widget(int x) : x_(x), y_{2} {}\n",
            "  ~Widget() override;\n",
            "  explicit operator bool() const noexcept { return mFlag && m_ready; }\n",
            "  Widget& operator=(const Widget&) = delete;\n",
            "  static_assert(sizeof(T) > 0, \"size\");\n",
            "  friend class Other;\n",
            "  virtual void run() = 0;\n",
            "  auto get() const -> decltype(x_) { return this->x_; }\n",
            " private:\n",
            "  int x_ = 0b1010'1010, y_ = 0x1.8p3, z_ = 017, w_ = 1'000'000ull;\n",
            "  double d_ = 1.5e-3f, e_ = .5L;\n",
            "};\n",
            "struct Point { int x, y; };\n",
            "enum class Color { Red, Green = 2 };\n",
            "union U { int i; float f; };\n",
            "typedef struct node node_t;\n",
            "int Widget::count = 0;\n",
            "std::vector<int> make(size_t n, const char* s = nullptr) {\n",
            "  auto raw = R\"delim(one\n",
            ")\" still )delim\";\n",
            "  auto lam = [&](int a) mutable { return a * 2; };\n",
            "  if (n <= 0 || !s) { throw std::runtime_error(\"bad %d\\n\"); }\n",
            "  for (auto& v : list) v.push_back(static_cast<int>(n));\n",
            "  switch (n) { case 1: break; default: goto done; }\n",
            "done:\n",
            "  printf(\"%5.2f %s %%\", 1.0, u8\"x\");\n",
            "  ptr->member.call<int>(1)->field;\n",
            "  delete[] arr; new int[3];\n",
            "  TEST_MACRO(a, sizeof(b), \"c\")\n",
            "  return std::move(n);\n",
            "}\n",
            "}  // namespace foo::bar\n",
            "#ifdef __cplusplus\n",
            "}\n",
            "#endif\n",
            "#pragma mark Section\n",
            "#error \"stop\" here\n",
        );
        colors_match_the_grammar("cpp", "C++", text);
    }

    /// ada-parse's C scanner (C++'s, X-057's) colors each byte as bat's grammar does, in every
    /// built-in theme, by rule a byte as [`cpp_rules_match_the_grammar`]: C's top level (a
    /// type, a function's name before its parameters, a call's group, a struct and an enum),
    /// the global `#if` rules and branches, a continued define, a float ending on its `.`, a
    /// hex number before a name, and C's strings, escapes and placeholders. The continued
    /// `#define` comes last: bat's C keeps its body open past its last line, through the next
    /// directive's block (an `#if 0` after it code), where ada's ends it with that line.
    #[test]
    fn c_rules_match_the_grammar() {
        let text = concat!(
            "#include <stdio.h>\n",
            "#include \"local.h\" /* why */\n",
            "#define MAX(a, b) ((a) > (b) ? (a) : (b))\n",
            "#define HALF 1.\n",
            "#if 0\n",
            "int disabled(void); #if nested\n",
            "#endif\n",
            "#else\n",
            "int enabled;\n",
            "#endif\n",
            "#if 1\n",
            "static int on;\n",
            "#else\n",
            "int off;\n",
            "#endif\n",
            "#ifdef HAVE_X\n",
            "{ int braced; }\n",
            "#elif defined(Y)\n",
            "extern long y;\n",
            "#endif\n",
            "typedef struct node { struct node *next; int value; } node_t;\n",
            "enum color { RED, GREEN = 2 };\n",
            "union u { int i; float f; };\n",
            "static const char *names[] = { \"a\", \"b\\n\", L\"wide\" };\n",
            "GType qmi_get_type (void) G_GNUC_CONST;\n",
            "API_EXPORT int\n",
            "count_items(const struct list *l, size_t n)\n",
            "{\n",
            "  int total = 0x1fUL + 017 + 1e3 + .5f + 0x__bad;\n",
            "  for (size_t i = 0; i < n; i++) {\n",
            "    if (l->items[i].ok && !l->skip) total += sizeof(l->items[i]);\n",
            "    else continue;\n",
            "  }\n",
            "  switch (total) { case 1: break; default: goto done; }\n",
            "done:\n",
            "  printf(\"%5.2f %s %%\\n\", 1.0, u8\"x\");\n",
            "  memcpy(dst, src, n); FOO_BAR(x)\n",
            "  return total > 0 ? total : -1;\n",
            "}\n",
            "#pragma mark Section\n",
            "#error \"stop\" here\n",
            "#define NAME \\\n",
            "          OTHER_NAME\n",
        );
        colors_match_the_grammar("c", "C", text);
    }

    /// ada-parse's C# lexer picks the rules bat's grammar picks, in every built-in theme
    /// (ADR-40): its corners included (comments the prototype doesn't reach into, a line
    /// comment's `/*` opening a block comment, a placeholder's format spec, an interpolation
    /// clearing its string, a verbatim string across lines, `_3D` and `٠m` as floats, a
    /// preprocessor line's rest invalid, doc comments' tags, a property's accessors).
    #[test]
    fn csharp_rules_match_the_grammar() {
        let text = concat!(
            "// a note // nested /* opens a block\n",
            "still the block */ after\n",
            "using System; using static System.Math; using Alias = Some.Name<int>;\n",
            "#region Things\n",
            "#if DEBUG && !RELEASE // why\n",
            "#pragma warning disable CS0168, IDE0001\n",
            "#nullable enable\n",
            "#endif\n",
            "namespace A.B.C\n",
            "{\n",
            "    /// <summary>A <see cref=\"Foo\"/> thing, TODO: https://x.y/z</summary>\n",
            "    /// <param name=\"x\">the x</param>\n",
            "    [Serializable, Obsolete(\"old\", error: true)]\n",
            "    [assembly: global::Xunit.Auto._47DEQ.Attr]\n",
            "    public sealed partial class Foo<T, in U> : Bar<T>, IBaz where T : class, new()\n",
            "    {\n",
            "        private readonly int _x = 0x1F_FF, _y = 0b1010;\n",
            "        public event EventHandler<int> Changed;\n",
            "        public string Name { get; private set; } = \"n\";\n",
            "        public int this[int i] => i * 2;\n",
            "        static Foo() : base() { }\n",
            "        ~Foo() { }\n",
            "        public static Foo operator +(Foo a, Foo b) => a;\n",
            "        internal async Task<List<int>> GetAsync(ref int a, out var b, params object[] c)\n",
            "        {\n",
            "            var s = $\"a {x,-10:N2} {{b}} {(y ? 1 : 2)}\" + @\"c:\\dir\"\" {0}\" + $@\"{z}\";\n",
            "            var f = string.Format(\"{0:D3} {1}\", 1.5e-3f, 'c') + '\\u0041' + 10UL + .5m;\n",
            "            Texture.Create(TextureType._3D, x٠m, 1_000);\n",
            "            if (a is int n && b as string != null) { return; } else if (x) y++; else z--;\n",
            "            foreach (var (k, v) in dict) { switch (k) { case 1: case Kind.A when v > 0: break; default: goto done; } }\n",
            "            for (int i = 0; i < 10; i++) while (true) do { } while (false);\n",
            "            try { throw new InvalidOperationException(nameof(x)); }\n",
            "            catch (Exception e) when (e.Message != null) { }\n",
            "            finally { lock (o) { } }\n",
            "            using (var r = new StreamReader(p)) { }\n",
            "            Func<int, int> sq = x => x * x; var t = (a: 1, b: 2); (int q, string w) = (1, \"\");\n",
            "            var list = new List<int> { 1, 2 }; var anon = new { A = 1, B = \"b\" };\n",
            "            var arr = new int[] { 1 }; int? m = null; var c = (Foo)obj; checked(x + 1);\n",
            "            var q2 = from c in list where c > 1 select c;\n",
            "            yield return default(T); typeof(List<>); unsafe { int* p = &x; }\n",
            "        done:\n",
            "            return await Task.FromResult(new List<int>());\n",
            "        }\n",
            "    }\n",
            "    public enum Color : byte { Red = 1, Green, }\n",
            "    public delegate void Handler<T>(object sender, T args);\n",
            "    interface IBaz { void Do(); }\n",
            "    public readonly ref struct Span2 { }\n",
            "}\n",
        );
        rules_match_the_grammar("cs", "C#", text);
    }

    /// ada-parse's Go lexer picks the rules bat's grammar picks, in every built-in theme
    /// (ADR-40): its corners included (a `//go:` line's annotation, `&^=` read as `&` and `^=`
    /// since the grammar's `\&^` holds a line-start anchor, a template in a string clearing
    /// its scope, a raw string across lines, a call in parentheses closing them, `make`'s type
    /// argument, a method's receiver, embedded types and tags, `iota`, a type switch, Unicode
    /// digits, a `(?=\S)` pop passed over by syntect's loop guard).
    #[test]
    fn go_rules_match_the_grammar() {
        let text = concat!(
            "//go:build linux && !cgo\n",
            "// Package x does things.\n",
            "package x\n",
            "\n",
            "import (\n",
            "\t\"fmt\"\n",
            "\tstr \"strings\"\n",
            ")\n",
            "\n",
            "//go:generate stringer -type=Op\n",
            "type Op int\n",
            "\n",
            "const (\n",
            "\tA Op = iota // first\n",
            "\tB, C\n",
            "\t_ = 1 << 2\n",
            ")\n",
            "\n",
            "var (\n",
            "\tm   = map[string][]*Op{\"a\": nil}\n",
            "\tch  chan<- struct{}\n",
            "\tfn  func(int, ...string) (bool, error)\n",
            ")\n",
            "\n",
            "type (\n",
            "\tPair[K comparable, V any] struct {\n",
            "\t\tKey K `json:\"key\"`\n",
            "\t\tVal V /* the value */\n",
            "\t\tio.Reader\n",
            "\t\t*Embedded\n",
            "\t}\n",
            "\tAlias = Pair[string, int]\n",
            "\tI     interface {\n",
            "\t\tfmt.Stringer\n",
            "\t\tRead(p []byte) (n int, err error)\n",
            "\t\tClose() error\n",
            "\t}\n",
            ")\n",
            "\n",
            "func (p *Pair[K, V]) Swap(a, b int, rest ...float64) (r Pair[V, K]) {\n",
            "\tx, y := 0x1F, 0o17i + 1.5e-3 + .5 + 0x1p-2 + 089 + 1_000 + '\\n' + '\\x41' + 'é'\n",
            "\tx &^= y; x <<= 1; x >>= 2; ok := x != y && !(x <= y) || x >= y\n",
            "\ts := \"%5.2f %[1]q %% {{.Name | printf \\\"%v\\\" | html}} \\u00e9\\t\" + `raw {{ $v := .X }}\n",
            "still raw %d`\n",
            "\tv := make([]map[string]int, 3); w := new(Pair[int, int]); n := len(v) + cap(v)\n",
            "\tswitch t := any(v).(type) { case int, string: fallthrough; default: _ = t }\n",
            "\tfor i := range v { if i > 0 { continue } else { break } }\n",
            "\tgo func() { defer close(ch2) }(); select { case <-done: return }\n",
            "\tf := func(a int) int { return a * 2 }; _ = (f(1)); goto end\n",
            "end:\n",
            "\tfmt.Println(str.ToUpper(\"x\"), int64(x), []byte(s), true, false, nil, ٣, x.y.z)\n",
            "\tvar q struct{ a, b int }; var e error = nil; const k, l = 1, 2\n",
            "\tm := map[string]\n",
            "\treturn\n",
            "}\n",
        );
        rules_match_the_grammar("go", "Go", text);
    }

    /// ada-parse's Python lexer picks the rules bat's grammar picks, in every built-in theme
    /// (ADR-40): its corners included (a docstring, decorators with and without a call, a
    /// lookahead's dict, set and tuple, f-strings with a conversion, a format spec and a
    /// nested replacement, a raw string's regex, a raw f-string's regex with a replacement
    /// that clears every scope, SQL in a triple-quoted string with the string's escapes and
    /// placeholders in it, printf and `str.format` placeholders, py2 numbers, a line
    /// continuation, `print` as a statement and as a call, and a docstring-like string after a
    /// continuation).
    #[test]
    fn python_rules_match_the_grammar() {
        let text = concat!(
            "#!/usr/bin/env python3\n",
            "\"\"\"A module's docstring, \\N{BULLET} \\x41.\"\"\"\n",
            "from __future__ import annotations\n",
            "from .pkg import (a as b, c)  # names\n",
            "import os.path, re\n",
            "\n",
            "@dataclass\n",
            "@app.route(\"/x/<id>\", methods=['GET', \"POST\"])\n",
            "class Point(Base, metaclass=ABCMeta):\n",
            "    '''Raw: \\n stays.'''\n",
            "    x: int = 0\n",
            "    MAX_SIZE = 0x1F + 0o17 + 0b101 + 017 + 10L + 1_000j + .5e-3 + 1.\n",
            "\n",
            "    async def go(self, a, /, *args, b: str = \"x\", **kw) -> dict[str, int]:\n",
            "        global count; nonlocal_ = lambda x, *y: x + y\n",
            "        d = {k: v for k, v in kw.items() if k != 'z'}; s = {1, 2}; t = (1, 2)\n",
            "        e = {'a': 1, **d}; f = {*s}; g = [*t]; h = {}; i = ()\n",
            "        msg = f\"{a!r:>{width}} {b=} {{literal}} {x['k']}\" + rf'\\d{{2}}{a}\\w+'\n",
            "        rx = re.compile(r\"^(?P<name>[a-z_]\\w*)\\s*(?:=|:)\\s*(?#c)(.+?)$\", re.I)\n",
            "        sql = \"\"\"\n",
            "            SELECT a.id, COUNT(*) FROM t AS a -- the rows\n",
            "            WHERE a.name = '%s' AND b IS NOT NULL ORDER BY 1 DESC\n",
            "        \"\"\"\n",
            "        old = \"%(name)-10s %5.2f %%\" % vars() + \"{0:>{1}} {name!s}\".format(1, 2)\n",
            "        when = time.strftime('%Y-%m-%d %-H') + b'\\x00\\777' + u'\\u00e9'\n",
            "        print >> sys.stderr, \"x\"\n",
            "        print(x, end=\"\")\n",
            "        x = y if z else \\\n",
            "            \"\"\"not a docstring\"\"\"\n",
            "        with open(p) as fh, lock: pass\n",
            "        try:\n",
            "            yield from gen()\n",
            "        except (KeyError, ValueError) as err:\n",
            "            raise RuntimeError(\"x\") from err\n",
            "        finally:\n",
            "            return await x[1:2], ..., None, __name__, __init__\n",
            "    for i in range(10): del i\n",
            "    while not done: break\n",
        );
        rules_match_the_grammar("python", "Python", text);
    }

    /// ada-parse's Lua lexer picks the rules bat's grammar picks, in every built-in theme
    /// (ADR-40): its corners included (comments from the prototype anywhere, even in a `\z`
    /// escape's spaces inside a string, long brackets with levels across lines, every escape,
    /// hex floats, a function's dotted name and method, metamethods as keys and properties,
    /// the builtin modules' members, `false` as `boolean.true` (sic), labels, a `,` in
    /// parentheses stepped over by syntect's loop guard, and keywords out of place).
    #[test]
    fn lua_rules_match_the_grammar() {
        let text = concat!(
            "#!/usr/bin/env lua\n",
            "--[==[ a long comment\n",
            "]] still ]==] local M = {}\n",
            "local mt = { __index = M, __call = function(self, ...) end, __mode = 'k' }\n",
            "function M.util:run(a, b, ...) -- the method\n",
            "  local s = 'a\\n\\x41\\65\\u{48}\\q' .. \"b\\z   -- inside\n",
            "     c\" .. [[x]] .. [=[y\n",
            "]]z]=]\n",
            "  for i = 1, #a do print(a[i], string.format('%d', math.pi), io.stdout) end\n",
            "  for k, v in pairs(t) do if k ~= nil and not v then goto skip elseif x then else end end\n",
            "  ::skip:: while x >= 0x1p4 + 0xA.8 + 1e-3 + .5 + 0xff + ٣ do x = x // 2 >> 1 end\n",
            "  repeat local y <const> = (a, b) until y == false\n",
            "  self.x.__index = function() return self, _G, _VERSION, true end\n",
            "  t[1]:m{1, [2] = 3; x = f\"s\"}; f[[long]] f'q' (g)(h)\n",
            "  return setmetatable({}, mt), coroutine.wrap(f), utf8.charpattern, x.__metatable\n",
            "end end then\n",
            "local f = function(a, end) return a end\n",
        );
        rules_match_the_grammar("lua", "Lua", text);
    }

    /// ada-parse's Lisp lexer picks the rules bat's grammar picks, in every built-in theme
    /// (X-006): its corners included (every declaration and the frames it sets up, a class's
    /// parents and a struct's options, `let`'s, `flet`'s and `macrolet`'s bindings, `declare`'s
    /// specifiers, `loop`'s keywords and builtins, `(quote …)` and `'(…)`, namespaces before a
    /// head, `make-`, `!`, `?` and `-p` functions, `|…|` symbols, annotations, every `#` macro,
    /// numbers of every base, format directives, nested block comments across lines, a stray `)`
    /// and `|#`). Backslashes in symbols aren't among them: those are read as the Lisp reader
    /// reads them, a fix over bat (G-001).
    #[test]
    fn lisp_rules_match_the_grammar() {
        let text = concat!(
            ";;; -*- lisp -*-\n",
            "(defun foo (x &optional (y 2) &rest r) \"Doc ~a~% ~2,'0d ~:@r ~10,2f ~5$\" (car x))\n",
            "(defmacro m (a &body b) `(progn ,a ,@b))\n",
            "(defclass c (pkg:a b) ((s :initarg :s)))\n",
            "(defstruct (pt (:conc-name pt-)) x y) (deftype ty (&optional n) 'integer)\n",
            "(defvar *v* nil) (defparameter *p* t) (defconstant +c+ 3.5d0)\n",
            "(let ((a 1) b) (declare (ignore b) (type fixnum a)) (flet ((f (z) z)) (f a)))\n",
            "(macrolet ((mm (q) q)) (labels ((g () 1)) (g)))\n",
            "(defmethod run :around ((o c) n) (call-next-method))\n",
            "(loop named outer for i from 0 below 10 with s = 0 collect (1+ i) into xs\n",
            "      finally (return xs))\n",
            "(dolist (e l) (dotimes (i 3) (do ((k 0 (1+ k))) ((> k 2)) (print k))))\n",
            "(cl:defun g () (quote (a (b c))) '(d e) 'f #'car #'(lambda (q) q) #'+)\n",
            "(make-pt :x 1) (set! a 1) (null? a) (foo-p a) (|f| x) (when (eq a b) (return-from g))\n",
            "#| block #| nested |# still\n",
            "   comment |# (if t nil) #\\Space #\\a #\\( #2\\x #+sbcl :a #-(or a b) b #. x #:g\n",
            "#1=(a . #1#) #(1 2) #3(a) #*1010 #b101 #B-1/10 #o17 #x1F #16rFF #3r12 #c(1 2)\n",
            "#p\"/tmp/x\" #s(pt :x 1) #2A((1 2)) #a #< 1.5 .5 1. 1/2 -3 +4 1e10 12.x 1.2.3\n",
            "+a+b+ *print-base* array string true false keyword:k :key ::k |a b| &x |#\n",
            "(*x* 1) (&rest) ('a) (t) (:k) (1 2) (\"s\") () (:a . b))\n",
        );
        // a keyword's `:` under the keyword (X-009) is a rule bat's sibling accessor never meets:
        // compared without it, pinned in `a_literals_punctuation_reads_in_its_color`
        // and the parens by depth (X-030), which bat's grammar has no scope for
        let mut left_out = vec![LISP_KEYWORD_MARK];
        left_out.extend(LISP_DEPTHS);
        rules_match_the_grammar_without("lisp", "Lisp", text, &left_out);
    }

    /// ada-parse's Clojure lexer picks the rules bat's grammar picks, in every built-in theme
    /// (X-006): every head (`def…` as a prefix, `defprotocol`, `definterface`, `deftype`,
    /// `defrecord`, `defmethod`, `reify`, `proxy`, `extend-*`, `fn`, `declare`) and the tails
    /// they set up, the reader macros and dispatch (`#_`, `##Inf`, tagged literals, `#?`, `#:`),
    /// metadata, characters, keywords, every number and its suffixes, a deprecated octal, strings
    /// across lines, comments and commas, stray closers. Regex literals aren't among them: their
    /// bodies are Java's dialect on ada-parse's regex scanner, a fix over bat by design. Nor are
    /// heads of `clojure.core`'s (`ns`, `let`, `when`, `str`, `+`): ada reads them by what they do, where
    /// bat calls every head a function (`clojure_heads` in ada-parse), so the calls here are
    /// none of its words.
    #[test]
    fn clojure_rules_match_the_grammar() {
        let text = concat!(
            "#!/usr/bin/env bb\n",
            "(in my.app (:require [clojure.string :as str])) ; c\n",
            "(defn- f \"doc\" [x & ys] {:pre [(valid x)]} (str/join \",\" ys))\n",
            "(def ^:private x 1) (def ^{:a 1} y 2N) (default z)\n",
            "(defprotocol P (m [this] \"doc\") (n [this a]))\n",
            "(definterface I (go [])) (deftype T [a] P (m [_] a)) (defrecord R [a b])\n",
            "(defmethod mm :k [x] x) (reify P (m [_] 1)) (proxy [Object] [] (toString [] \"s\"))\n",
            "(extend-protocol P String (m [s] s)) (extend-type T P (m [_] nil))\n",
            "(fn [x] x) (fn* [y] y) (fn name [z] z) (declare a b) (nil 1) (true) (f / 2)\n",
            "[1 -2 +3 012 1N 0x1F 0XffN 2r101 36rZZ 1/2 1.5 1.5e10 1e5 1M 1.5M 1. 1# x# x']\n",
            "{:k 'q `r ~s ~@t @u ::v :a/b \\a \\newline \\( #_ (skip) ##Inf ##-Inf #inst \"2020\"}\n",
            "#{1 2} #(g % 1) #?(:clj 1 :cljs 2) #?@(:clj [1]) #:a{:b 1} #'var , \"multi\n",
            "line\" \"esc\\\"aped\" ) ] } (h [a 1] a)\n",
        );
        // the delimiters by depth (X-030) bat's grammar has no scope for
        rules_match_the_grammar_without("clojure", "Clojure", text, &LISP_DEPTHS);
    }

    /// ada-parse's SQL lexer picks the rules bat's grammar picks, in every built-in theme
    /// (ADR-40): its corners included (`create`'s name popped at the line's end, `drop`'s
    /// objects as prefixes, `drop table` at a word's end, `comment on … is` with the spaces
    /// before it, `alter`'s spaces ending its match, the types' sizes and time zones,
    /// `is not null`, the multi-word DML, quotes doubled and escaped, interpolation, `%{}` and
    /// `%r{}`, a block comment's `*` lines, `#` and `--` comments).
    #[test]
    fn sql_rules_match_the_grammar() {
        let text = concat!(
            "-- schema\n",
            "CREATE OR REPLACE FUNCTION s.\"f\"(a int) RETURNS text AS $$ SELECT 1 $$;\n",
            "CREATE UNIQUE INDEX\n",
            "  idx ON t (a);\n",
            "create table if not exists `t` (id bigint primary key, n varchar(255) not null,\n",
            "  p numeric(10,2) default 0, d timestamp(3) with time zone, c character varying(10),\n",
            "  x double precision, y bit varying(3), z var char(2), f float(8), g float,\n",
            "  FOREIGN KEY (a) REFERENCES u(id) ON DELETE CASCADE, CHECK (n <> ''));\n",
            "drop tables; DROP TABLE x CASCADE; backdrop table y; drop index_x;\n",
            "ALTER TABLE t COMMENT ON COLUMN a.b IS 'x';\n",
            "  COMMENT ON TABLE t IS 'it''s \\'q\\'';\n",
            "/* block\n",
            " * star\n",
            " */ SELECT DISTINCT a.b, count(*), COUNT (x), counter(y), current_user, lower(n)\n",
            "FROM t NATURAL LEFT OUTER JOIN u INNER JOIN v USING (id) # mysql\n",
            "WHERE x IS NOT NULL AND y not null OR z IS null AND q <=> 1 AND r != 2 || 'a'\n",
            "GROUP BY a ORDER  BY b DESC LIMIT 10 UNION ALL SELECT TOP 5 * FROM w;\n",
            "INSERT IGNORE INTO t VALUES (1, \"a\"\"b #{x} c\", %{p #{q} r}, %r{s}, ());\n",
            "BEGIN WORK; START TRANSACTION; GRANT WITH GRANT OPTION; REVOKE; COMMIT;\n",
            // a fraction or an exponent (`1.5`, `1e3`) reads as one number, bat's as two or
            // none: ada-parse's `decimals_and_exponents` (X-008)
            "SELECT 1 AS one, true, FALSE, 12ab, ٣, x.y.z ASC;\n",
        );
        rules_match_the_grammar("sql", "SQL", text);
    }

    /// ada-parse's PowerShell lexer picks the rules bat's grammar picks, in every built-in
    /// theme (ADR-40): its corners included (`#requires` with a hashtable, `using namespace`,
    /// a block comment's docs, `function` with a scope, attributes and their parameters, types,
    /// commands with a path before them, every variable form, operators after `-`, numbers with
    /// suffixes and units, strings with escapes and interpolation, here-strings, `--%`, `class`,
    /// `%` and `?`, dot-sourcing, a range, a type's name after a digit). An exponent's sign and
    /// its digits past the first aren't bat's, by design: ada-parse's
    /// `exponents_with_signs_and_digits` (X-008).
    #[test]
    fn powershell_rules_match_the_grammar() {
        let text = concat!(
            "#requires -Version 5.1 -Modules @{ ModuleName = 'x'; RequiredVersion = '1.0' }\n",
            "using namespace System.IO.\n",
            "<# .SYNOPSIS\n",
            "  Does x.\n",
            ".PARAMETER Name\n",
            "  The name.  #> <# .NOTES\n",
            "#>\n",
            "function global:Get-Thing {\n",
            "    [CmdletBinding(SupportsShouldProcess = $true)]\n",
            "    param([Parameter(Mandatory, Position=0)][ValidateSet('a', 'b')][string[]] $Name, [int]$N = 0x1FUL)\n",
            "    begin { $ErrorActionPreference = 'Stop' } process {\n",
            "        Get-ChildItem C:\\x\\Get-Foo.ps1 -Path $env:TEMP 2>&1 | ForEach-Object { $_.FullName } | Where-Object Length -gt 10mb\n",
            "        $h = @{ 'k' = 1; Name = \"a $x `n $($y.z) `u{1F600} `u{ \"\"q\"\" ${env:path}.Length\" }; $a = @(1, 2..3, -.5, 1.5e3d, 1e1, _)\n",
            "        $s = @\"\n",
            "text $v `t $(Get-Date) \"\" ${global:x}\n",
            "\"@\n",
            "        $t = @'\n",
            "raw $v ''\n",
            "'@\n",
            "        if ($global:x -is [string] -and -not $script:y.Count -ceq 1 -or $z -bxor 2) { cmd.exe /c x --% raw %PATH% } elseif ($?) { return $null } else { throw }\n",
            "        $x.foreach({ $PSItem }) ; $y | % { $_ } ; x.% foo ; 'it''s' + \u{2018}smart\u{2019} -f $Host.UI.RawUI\n",
            "    }\n",
            "}\n",
            "class Foo : Bar { hidden static [string] $Name }  # done ## x `\n",
            "filter f { $input }; .\\x.ps1 ; . $p; [System.IO.File]::Exists($p) ; [2abc] ; a2>> b ; \"x `\n",
            "y\"\n",
        );
        rules_match_the_grammar("powershell", "PowerShell", text);
    }

    /// ada-parse's JavaScript lexer picks the rules bat's `JavaScript (Babel)` picks, in every
    /// built-in theme (ADR-40's phase 5): its corners included (a shebang, labels and quoted keys
    /// with functions and arrows, classes with properties, static and async methods, an object's
    /// accessor, prototype assignments, a regex after `(` and `=` but a division after a name, a
    /// template with a substitution, escapes and a line continuation, JSX across lines with
    /// attributes, entities and expressions, Flow annotations, `-Infinity`, hex, octal and
    /// legacy octal numbers, `{{`, `switch` and `for`, a constructor, node's and the console's
    /// objects, `<!--` and a line's `-->`). A regex's body is one kind by design, so the regexes
    /// here hold no token bat's regex grammar colors. Not what X-058's scanner departs from
    /// Babel in, by design (ada-parse's `javascript` tests): a ternary's `:`, which Babel reads
    /// as a key's; a class field's `;` and the blank after a class's `get`, which Babel leaves
    /// plain and colors (and a field without its `;` carries into a `static` after it); a Flow return type, which Babel reads as operators; `07.5`, which
    /// Babel marks invalid; a `-->` past a line's start (`x-->0`), Babel's comment anywhere.
    #[test]
    fn javascript_rules_match_the_grammar() {
        let text = concat!(
            "#!/usr/bin/env node\n",
            "/** The module. */\n",
            "import React, { useState } from 'react'; // a comment\n",
            "export default class Foo extends Bar {\n",
            "  static async *load(id, ...rest) { await fetch(`/api/${id}?q=${rest[0]}`); }\n",
            "  static count = 0\n",
            "  state = { open: false, 'quoted': \"x\\ty\\u00e9\" }\n",
            "  @observable value: string = 'v'\n",
            "  render() {\n",
            "    const { a, b } = this.props, n = a ?? b ?? 1;\n",
            "    return (\n",
            "      <div className=\"box\" onClick={() => this.toggle(n)}>\n",
            "        &amp; {n > 1 && <Item key={n} label='one' />}\n",
            "      </div>\n",
            "    );\n",
            "  }\n",
            "}\n",
            "Foo.prototype.reset = function reset() { this.n = 0x1F + 0o17 + 07 + 1e-3 + -Infinity; };\n",
            "Foo.prototype.add = (a, b = 2) => a + b;\n",
            "const re = /abc/gi, half = total / 2, ok = re.test(s) && !(x instanceof Y);\n",
            "const obj = {\n",
            "  get size() { return this.items.length; },\n",
            "  method: function (x) { return typeof x === 'number'; },\n",
            "  arrow: async (y) => y * 2,\n",
            "  'key': value,\n",
            "  [computed]: null,\n",
            "};\n",
            "switch (kind) { case 'a': break; default: console.log(process.env.HOME); }\n",
            "for (let i = 0; i < 10; i++) { module.exports[i] = new Map(); }\n",
            "const s = \"line \\\n",
            "next\"; const t = `multi\n",
            "line ${ {{ }} }`;\n",
            "function* gen(a: number, b?: string) { yield* other(); }\n",
            "if (x) { throw new TypeError('bad'); } else { debugger; }\n",
            "<!-- legacy\n",
            "-->\n",
        );
        rules_match_the_grammar("js", "JavaScript (Babel)", text);
    }

    /// ada-parse's TypeScript scanner picks the rules bat's `TypeScript` picks, in every
    /// built-in theme (ADR-40's phase 5): declarations of every kind (interfaces, type aliases
    /// with conditional, mapped and template literal types, enums, namespaces, abstract classes
    /// with modifiers, decorators, overloads, accessors and index signatures), generics in calls
    /// and arrows, the `<T>x` cast, `as const`, optional chaining, destructuring, a `const` at a
    /// line's end, JSDoc with types, `@example` and its caption, a regex with groups and
    /// classes, templates with a tag, and objects nested past the 28 levels a state keeps
    /// unpacked.
    #[test]
    fn typescript_rules_match_the_grammar() {
        let mut text = String::from(concat!(
            "/// <reference path=\"./types.d.ts\" />\n",
            "import type { A, B as C } from './a';\n",
            "import * as fs from 'fs';\n",
            "export interface Point<T extends number = number> { readonly x: T; y?: T; [key: string]: unknown }\n",
            "type Maybe<T> = T extends null | undefined ? never : T;\n",
            "type Keys = { [K in keyof Point as `get${Capitalize<K & string>}`]: () => Point[K] };\n",
            "export enum Color { Red = 1, Green = 'g', ['Blue'] }\n",
            "declare namespace NS { export const v: number; }\n",
            "/**\n",
            " * Makes one.\n",
            " * @param {string} name - the name\n",
            " * @returns {Promise<Foo>} {@link Foo}\n",
            " * @example <caption>Use</caption>\n",
            " * make('a');\n",
            " */\n",
            "@Component({ selector: 'app' })\n",
            "export abstract class Foo<T> extends Base<T> implements I {\n",
            "  private static readonly count: number = 0;\n",
            "  #secret = 1;\n",
            "  constructor(public name: string, protected opts?: Options) { super(); }\n",
            "  get size(): number { return this.items?.length ?? 0; }\n",
            "  abstract run(): void;\n",
            "  async *load<U>(id: U, ...rest: unknown[]): AsyncGenerator<U> { yield* other<U>(id); }\n",
            "  handler = (e: Event): void => { console.log(e.target as HTMLElement); };\n",
            "}\n",
            "function over(a: string): string;\n",
            "function over(a: any) { return <string>a; }\n",
            "const { a, b: [c, d] } = obj, n = a! + 0x1F + 1_000n;\n",
            "const pick = <T, K extends keyof T>(o: T, k: K): T[K] => o[k];\n",
            // a regex's body is ECMAScript's grammar's since phase 4 (ada-parse's `typescript`
            // tests), not bat's TypeScript's: these few tokens color alike in both
            "let re = /x+$/gi, half = total / 2;\n",
            "const tag = html`<p>${name}</p>`, plain = `a ${b ? 'c' : \"d\"}`;\n",
            "const conf = { mode: 'x', list: [1, 2] } as const;\n",
            "const\n",
            "  // a comment after the keyword\n",
            "  late = typeof window !== 'undefined' && window instanceof Object;\n",
            "for (const [k, v] of Object.entries(map)) { if (k in v) continue; }\n",
            "switch (kind) { case 'a': break; default: throw new Error(`bad ${kind}`); }\n",
            "export default async function main(): Promise<void> { await Promise.all([]); }\n",
            // the regexes' backtracking and lookahead scopes met in scrambled input
            "function spread(a, undefined...more) {}\n",
            "if (a : B) { go(); }\n",
            "if (a) /x+/.test(b);\n",
            "const p = (\n",
            "  \"s\" :as  { a: 1 });\n",
            "type F = (cb = () => void, d? : () => void) => void;\n",
            "interface I { a : () => void; b? : () => void; c: () => void }\n",
            "type R = { -readonly\n",
            "};\n",
            "let t: <T>`x${K}`, u: .tag`y`;\n",
            "x = async => 1, y = (a): => 2, z = f<typeof o<d>S>(o);\n",
            "import type = x;\n",
            "import type import y from 'z';\n",
            "/** @return$s x @type {T} y @type {U} z {@linkx Bar} @returns {\n",
            " */\n",
        ));
        // objects nested past 28 frames: the state packs them between lines
        for _ in 0..20 {
            text.push_str("({ key: {\n");
        }
        for _ in 0..20 {
            text.push_str("} })\n");
        }
        rules_match_the_grammar("ts", "TypeScript", &text);
    }

    /// ada-parse's TSX, TypeScript's scanner with JSX, picks the rules bat's `TypeScriptReact`
    /// picks, in every built-in theme (ADR-40's phase 5): elements with and without attributes,
    /// components with namespaces and type arguments, entities in children and in attribute
    /// strings, expressions as children and values, a generic arrow `<T,>` that is no function
    /// for a name, and `<T extends …>` that is.
    #[test]
    fn tsx_rules_match_the_grammar() {
        let text = concat!(
            "import React from 'react';\n",
            "export const List = <T,>({ items }: Props<T>) => (\n",
            "  <ul className=\"list\" data-n='1 &amp; 2'>\n",
            "    {items.map((item, i) => <Item<T> key={i} value={item} />)}\n",
            "    <svg:rect width={10} /> &copy; &#169; &#xA9; & more\n",
            "    <></>\n",
            "  </ul>\n",
            ");\n",
            "const pick = <T extends object>(o: T) => o;\n",
            "function App(): JSX.Element {\n",
            "  return <Layout.Main title={`a ${b}`} {...rest}>\n",
            "    <p>text</p>\n",
            "  </Layout.Main>;\n",
            "}\n",
            "const ns = <svg:>y</svg:>;\n",
        );
        rules_match_the_grammar("tsx", "TypeScriptReact", text);
    }

    /// ada-parse's CSS lexer picks the rules bat's grammar picks, in every built-in theme
    /// (ADR-40's phase 6): its corners included (every at-rule and its prelude, a comment across
    /// lines, selectors with custom elements, classes, ids, pseudo elements and classes and their
    /// functions, attribute selectors with a namespace and a flag, combinators bad and good;
    /// properties with vendor prefixes, custom and deprecated ones, `font`'s unquoted names; the
    /// functions' groups, numbers of each type with units, `0` in a length-only group, colors,
    /// constants that stop at a `-`, strings with escapes and an escaped newline, unicode ranges,
    /// `!important` and `progid`).
    #[test]
    fn css_rules_match_the_grammar() {
        let text = concat!(
            "@charset \"utf-8\"; x\n",
            "@import url(\"a.css\") screen and (orientation: landscape);\n",
            "@namespace svg url(http://www.w3.org/2000/svg);\n",
            "@custom-media --narrow (max-width: 30em);\n",
            "/* a comment\n",
            " * across lines */\n",
            "@media only screen and (-webkit-min-device-pixel-ratio: 2), print and (aspect-ratio: 16/9) {\n",
            "  my-element > .btn-primary:hover::before, a[href$=\".pdf\" i] ~ #main *, svg|rect >>> b {\n",
            "    -webkit-transition: opacity .3s ease-in-out, transform 0.5s cubic-bezier(.17, .67, .83, .67);\n",
            "    --main-color: #ffcc00aa; var-x y; color: var(--main-color, red) !important;\n",
            "    font: italic bold 12px/30px Georgia, \"Times New Roman\", serif;\n",
            "    background: url(img.png) no-repeat, linear-gradient(to right, rgba(0, 0, 0, .5) 0%, #fff 100%);\n",
            "    grid-template-columns: repeat(auto-fill, minmax(100px, 1fr)) [line-a] auto-flow;\n",
            "    transform: translateX(-10%) rotate(45deg) scale(1.5) translateZ(0.5px); width: calc(100% - (2 * 1.5em));\n",
            "    content: \"\\201C x \\\"\" attr(data-x px, 1); filter: blur(10) progid:DXImageTransform.Microsoft.gradient(startColorstr='#80000000');\n",
            "    list-style: decimal-leading-zero; counter-reset: c 1; content: counter(c, upper-roman) counters(c, \".\", lower-alpha);\n",
            "    margin: 0 auto -1.5e2px +2em; z-index: 10; line-height: 1.5; color: CurrentColor; outline: 1px solid aliceblue;\n",
            "    content: 'a\\\n",
            "b';\n",
            "  }\n",
            "  li:nth-child(2n + 1):not(.a, :first-child) { clip: rect(0, auto, 1px, 2px) }\n",
            "}\n",
            "@font-face { font-family: \"X\"; src: local(Arial), url(x.woff2) format(\"woff2\"); unicode-range: U+0025-00FF, u+4??; }\n",
            "@keyframes spin { from { transform: rotate(0deg) } 50% { opacity: .5 } to { transform: rotate(1turn) } }\n",
            "@-webkit-keyframes a, b { }\n",
            "@supports (display: grid) and (not (display: inline-grid)) { .x { display: grid } }\n",
            "@page :first { margin: 1in }\n",
            "@counter-style thumbs { system: cyclic; symbols: \"x\"; }\n",
            "@document url(http://x), url-prefix(y), domain(z), regexp(\"a\") { b { c: d } }\n",
            "div.a#b, h1 + p, clipPath, :root, ::-moz-selection, :lang(en), :drop(active) { -moz-box-sizing: border-box; }\n",
            "@tailwind base; .é-x { top: 5q; transition: all 200ms steps(4, jump-end); color: hsl(120deg, 50%, 50%) }\n",
        );
        rules_match_the_grammar("css", "CSS", text);
    }

    /// ada-parse's SCSS lexer (CSS's scanner with SCSS's contexts) picks the rules bat's grammar
    /// picks, in every built-in theme (ADR-40's phase 6): the front matter's YAML, `//` comments,
    /// variables (namespaced, declared, with flags), maps with keys of both kinds, interpolation
    /// in selectors, property names, strings and values, every Sass at-rule with its prelude,
    /// mixins with parameters and includes with arguments, `&` and placeholders, nested and
    /// partial property names, Sass's functions and custom ones, operators, numbers with the sign
    /// after them read as an illegal operator, the CSS at-rules in SCSS's form, css-modules.
    #[test]
    fn scss_rules_match_the_grammar() {
        let text = concat!(
            "---\n",
            "title: x\n",
            "---\n",
            "@use \"sass:math\" as m;\n",
            "@use \"a\" as *; @forward \"b\" show f, $g; @use \"c\" with ($d: 1);\n",
            "@import \"x\", url(y) screen; @import (reference) \"z\";\n",
            "// a line comment\n",
            "$map: (key: 1px, 'k2': (a: #fff, $v: red), b: math.$pi) !default;\n",
            "$x: 10px - 2 * 3 / 4 !global;\n",
            "@mixin m($a, $b: 10px) { width: $a + $b; @content; }\n",
            "@function f($x) { @return math.div($x, 2) * 1%; }\n",
            "@each $k, $v in $map { .icon-#{$k} { color: map-get($map, $k); } }\n",
            "@for $i from 1 through 3 { .m-#{$i} { margin: #{$i * 4}px -2px; } }\n",
            "@if $a == b and not $c { @warn \"w #{$d}\"; } @else if $a != c { @debug $x; } @else { @error 'e'; }\n",
            "@while $i > 0 { $i: $i - 1; }\n",
            "%ph { a: b; } .a { @extend %ph; @extend .b !optional; }\n",
            ".btn, #id > a:hover::before, my-el:not(.x, y), &__el, & + &, [data-x=\"y\" i] {\n",
            "  @include ns.mx(1px, $b: 2) { c: d; }\n",
            "  @include bp(md);\n",
            "  font: { family: \"X\", sans-serif; size: 12px; }\n",
            "  border: { top-width: 1px; }\n",
            "  margin-#{$side}: 0; --#{$v}: 1; --custom: #{$z};\n",
            "  -webkit-transition: opacity .3s, transform 0.5s cubic-bezier(.17, .67, .83, .67);\n",
            "  background: url(#{$p}/img.png) no-repeat, linear-gradient(to right, rgba($c, .5) 0%, darken($c, 10%) 100%);\n",
            "  width: calc(100% - #{$gutter}) clamp(1px, 2vw, 3px); color: Red; content: \"a#{$b}c\\\"\" + d;\n",
            "  transform: translate(-50%, 1px) rotate(45deg); grid: repeat(auto-fill, minmax(10px, 1fr)) / auto;\n",
            "  @at-root .x { y: z; }\n",
            "}\n",
            "@media screen and (min-width: $bp) and (orientation: landscape), print { .p { q: r } }\n",
            "@keyframes #{$n} { from { a: b } 50% { c: d } to { e: f } }\n",
            "@supports (display: grid) and (not (x: y)) { .s { t: u } }\n",
            "@font-face { src: url(x.woff2) format(\"woff2\"); } @charset \"utf-8\";\n",
            "@page :first { margin: 1in } @counter-style thumbs { system: cyclic; }\n",
            ":export { primary: $primary; } :global(.g) { h: i }\n",
            "a { b: if(true, null, false) nth($l, 1) e(\"x\") my-fn(1) ns.fn(2) 1px+2px; }\n",
        );
        rules_match_the_grammar("scss", "SCSS", text);
    }

    /// ada-parse's HTML lexer, with CSS's and JavaScript's for what HTML embeds, picks the rules
    /// bat's grammar picks, in every built-in theme (ADR-40's phase 6): the doctype with its
    /// external id and internal subset, `<?xml`, comments good and bad, CDATA, every tag list
    /// and its end (`/>` where it may self-close), custom tags with bad characters, other tags,
    /// attributes of each kind with quoted, unquoted and bad values, character references in
    /// text, strings and unquoted values, `<style>` and `<script>` of each type with their closing
    /// tags (`<!--`, `-->`, `</style >`'s escape alone), `style="…"` and `on…='…'` with an
    /// escaped quote and a `//` comment past the quote, a `//` comment past `</script>`, a regex
    /// class's text cut at `</script` and at an attribute's quote. Not a ternary's `:` nor a regex
    /// at an attribute's start, where JavaScript's scanner departs from Babel by design (X-058;
    /// [`javascript_rules_match_the_grammar`]).
    #[test]
    fn html_rules_match_the_grammar() {
        let text = concat!(
            "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0//EN\" 'x.dtd' [ <!-- s --> ]>\n",
            "<?xml version=\"1.0\" encoding='utf-8'?>\n",
            "<!doctype html><!-- a <!-- b --!> c --> <!--> x --> <![CDATA[ <b> ]]> <>\n",
            "<html lang=en><head><meta charset=\"utf-8\"><title>T &amp; &#169; &#x1F600; &x</title>\n",
            "<link rel=stylesheet href='a.css'/><base href=/x/></head>\n",
            "<body class=\"a b\" id='main' data-x=\"1\" hidden><div><p>x<br/><hr/><HR>\n",
            "<form action=\"/go\"><fieldset><input type=text value=\"a&quot;b\" disabled/><button>b</button>\n",
            "<a href=\"#x\" a\"b=1 c=d\"e`f<g>l</a><table><tr><td>1</td></tr></table></fieldset></form>\n",
            "<my-el foo=bar></my-el><x-É-Y.z\\.w></x-É-Y.z\\.w><foo:bar baz='1'/></foo:bar><svg-x/>\n",
            "<span style=\"color: red; font-family: Arial\" onclick=\"go(1); return false\">s</span>\n",
            "<style type=\"text/css\" media=print>\n",
            "  body > .a::before { color: #fc0; margin: 0 -1em; }\n",
            "  /* c */ @media (min-width: 40em) { a:hover { b: c } }\n",
            "</style><style type=text/less>@x: 1;</style><style>a { b: c }</style >x\n",
            "y</style><style/>\n",
            "<script>\n",
            "<!--\n",
            "  var x = 1 < 2 && \"a\" || 'b'; // </script> stays a comment\n",
            "  function f(a) { return `t ${a}` / 2; }\n",
            "-->\n",
            "</script>\n",
            "<script type=\"module\" src=x.js></script><script type='text/html'><b>t</b></script>\n",
            "<script type=\"text/template\"><div>{{x}}</div></script><script>f()</SCRIPT >\n",
            "<script>r = /[a</SCRIPT>b]/; s = /x[^c</script>d]/g;</script>\n",
            "<b onclick=\"r = /[a\" c=\"]/\" onmouseover='x = /a[^b'>x</b>\n",
            "</div></body></html>\n",
            // last: the comment runs past the quote, so the attribute stays open to the end
            "<i style='content:\"\\'x\"' onmouseover='a // b'>i</i>\n",
        );
        rules_match_the_grammar("html", "HTML", text);
    }

    /// ada-parse's Dockerfile lexer picks the rules bat's grammar picks, in every built-in
    /// theme (ADR-40).
    #[test]
    fn dockerfile_rules_match_the_grammar() {
        let text = concat!(
            "# syntax=docker/dockerfile:1\n",
            "ARG BASE=alpine\n",
            "FROM rust:1.88-slim@sha256:abc AS build\n",
            "  # indented comment\n",
            "WORKDIR /src\n",
            "COPY --from=deps /cache /cache\n",
            "copy . .\n",
            "RUN apt-get update && \\\n",
            "    apt-get install -y \"curl\" 'jq' \\\n",
            "\n",
            "    && echo \"a \\\" b\"\n",
            "ENV PATH=\"/x:$PATH\" \\ \n",
            "ONBUILD RUN make\n",
            "MAINTAINER someone\n",
            "EXPOSE 8080\n",
            "RUN\n",
            "echo continues\n",
            "CMD [\"ada\", \"--help\"]\n",
            "ENTRYPOINT [\"ada\"]\n",
            "LABEL x=\"open\n",
            "lower case line\n",
            "HEALTHCHECK CMD true\n",
            "FROM scratch\n",
            "garbage here\n",
        );
        rules_match_the_grammar("dockerfile", "Dockerfile", text);
    }

    /// ada-parse's Java properties lexer picks the rules bat's grammar picks, in every
    /// built-in theme (ADR-40).
    #[test]
    fn properties_rules_match_the_grammar() {
        let text = concat!(
            "# comment\n",
            "  ! another\n",
            "key = value\n",
            "key2:value2\n",
            "key3 value3\n",
            "a\\ b = c\\td \\u00e9 \\uZZ\n",
            "multi = one, \\\n",
            "  two, \\\n",
            "  three\n",
            "cont\\\n",
            "inued = x\n",
            "empty=\n",
            "   \n",
            "lone\n",
        );
        rules_match_the_grammar("properties", "Java Properties", text);
    }

    /// ada-parse's fstab lexer picks the rules bat's grammar picks, in every built-in theme
    /// (ADR-40).
    #[test]
    fn fstab_rules_match_the_grammar() {
        let text = concat!(
            "# <file system> <dir> <type> <options> <dump> <pass>\n",
            "UUID=ab12\t/\text4\trw,relatime,x=1\t0 1\n",
            "/dev/sda2 none swap sw 0 0\n",
            "server:/export /nfs nfs4 _netdev 0 0 # c\n",
            "//srv/share /mnt cifs user=a 3 x y\n",
        );
        rules_match_the_grammar("fstab", "fstab", text);
    }

    /// ada-parse's `known_hosts` lexer picks the rules bat's grammar picks, in every built-in
    /// theme (ADR-40).
    #[test]
    fn known_hosts_rules_match_the_grammar() {
        let text = concat!(
            "# c\n",
            "@revoked [a,b*]:22,1.2.3.4,::1 ssh-rsa AAAAB3NzaC1yc2E= a label\n",
            "|1|abcdefghijklmnopqrstuvwxyz0=|abcdefghijklmnopqrstuvwxyz0= ssh-dss x\n",
            "@cert-authority !*.example.com ecdsa-sha2-nistp256 AAAAE2VjZHNh\n",
        );
        rules_match_the_grammar("known_hosts", "Known Hosts", text);
    }

    /// ada-parse's `.env` lexer picks the rules bat's grammar picks, in every built-in theme
    /// (ADR-40).
    #[test]
    fn dotenv_rules_match_the_grammar() {
        let text = concat!(
            "# comment\n",
            "export DATABASE_URL=\"postgres://${USER}:$PASS@host/db\\n\" # c\n",
            "DEBUG = true\n",
            "N=0x1F\n",
            "F=1.5e+3\n",
            "G=.5\n",
            "H=1. 2.x\n",
            "S='single $NOT'\n",
            "M=\"multi\n",
            "line {x} $$y\"\n",
            "EXPORTER=null\n",
            "a=b=c\n",
        );
        rules_match_the_grammar("env", "DotENV", text);
    }

    /// ada-parse's Git lexers pick the rules bat's grammars pick, in every built-in theme
    /// (ADR-40).
    #[test]
    fn git_rules_match_the_grammar() {
        let ignore = concat!(
            "# build output\n",
            "/target/\n",
            "!keep.me\n",
            "*.log ; not a comment\n",
            "**/node_modules\n",
            "docs/[!a-c]*.md\n",
            "a[]b]c\n",
            "a[x\n",
            "\\#literal \\ \n",
            "$HOME/../.cache/./x\n",
            "C:/drive ~/home\n",
            "   \n",
        );
        rules_match_the_grammar("gitignore", "Git Ignore", ignore);
        let attributes = concat!(
            "# attributes\n",
            "* text=auto eol=lf\n",
            "*.png binary -diff\n",
            "*.rs diff=rust merge=union filter=lfs encoding=UTF-16 whitespace=tab-in-indent,trailing-space\n",
            "\"quoted path\" -text !eol custom=a,b, x=\"q\\\"v\"\n",
            "[attr]binary -diff -merge -text\n",
            "-bad op\n",
            "path/[ab]*.c foo= bar\n",
        );
        rules_match_the_grammar("gitattributes", "Git Attributes", attributes);
        let mailmap = concat!(
            "# mailmap\n",
            "Proper Name <proper@email.xx>\n",
            "<proper@email.xx> <commit@email.xx>\n",
            "Proper Name <proper@email.xx> Commit Name <commit@email.xx>\n",
            "Name Only\n",
            "<bad email> x\n",
        );
        rules_match_the_grammar("mailmap", "Git Mailmap", mailmap);
        let link = concat!(
            "gitdir: ../.git/modules/sub\n",
            "  gitdir :C:\\repo\\.git.\n",
            "gitdir: ~/x/*\n",
            "notgitdir: x\n",
            "# comment\n",
            "gitdir\n",
        );
        rules_match_the_grammar(".git", "Git Link", link);
    }

    /// The Makefile lexer picks the rules bat's grammar picks, in every built-in theme (G-002).
    #[test]
    fn make_rules_match_the_grammar() {
        let text = concat!(
            "# build \\\n",
            "  continued comment\n",
            "include config.mk\n",
            "-include $(wildcard *.d)\n",
            "vpath %.c src\n",
            "CC ?= gcc\n",
            "CFLAGS := -O2 -Wall # flags\n",
            "OBJS = $(patsubst %.c,%.o,$(SRCS)) ${EXTRA:.c=.o}\n",
            "override LDFLAGS += -lm\n",
            "export PATH := $(HOME)/bin:$(PATH)\n",
            "REV != git rev-parse --short HEAD\n",
            "RUN = sh -c 'echo \"$$1\" && exit 0'\n",
            "define HELP =\n",
            "  usage: make [target]\n",
            "endef\n",
            "ifeq ($(CC),gcc)\n",
            "  WARN = -Wextra\n",
            "else ifneq \"$(CC)\" 'clang'\n",
            "endif\n",
            "all: $(OBJS) | dirs ; @echo inline\n",
            "%.o: %.c config.h\n",
            "\t@$(CC) $(CFLAGS) -c $< -o $@\n",
            "\t-rm -f \"$@.tmp\" 2>/dev/null || true\n",
            "\tfor f in $^; do [ -e \"$$f\" ] && echo `basename $$f`; done\n",
            "\tf() { printf '%s\\n' $$1; }; f x\n",
            "test: CFLAGS += -g\n",
            "test:: all\n",
            "\t$(info $(call show,$(OBJS),\"quoted\"))\n",
            "\t$(Q)$(kecho) \"  SYNC    $@\"\n",
            "\tVAR=1 cd build && $(MAKE) -C sub $(shell uname -s)\n",
            "\tif [ -n \"$${HOME}\" ]; then echo ok; else exit 1; fi\n",
            "\tcase $$x in a|b) echo $$((1 + 2));; esac\n",
            "endef\n",
            "a((m)    ((g)    (i)    : b\n",
        );
        rules_match_the_grammar("make", "Makefile", text);
    }

    /// The shell lexer picks the rules bat's bash grammar picks, in every built-in theme
    /// (G-003).
    #[test]
    fn bash_rules_match_the_grammar() {
        let text = concat!(
            "#!/usr/bin/env bash\n",
            "set -euo pipefail # strict\n",
            "readonly DIR=\"$(cd \"$(dirname \"${BASH_SOURCE[0]}\")\" && pwd)\"\n",
            "declare -A seen=([a]=1 [b]=2)\n",
            "export PATH=\"$HOME/bin:$PATH\" LANG=C\n",
            "usage() {\n",
            "  printf 'usage: %s [-v] file...\\n' \"${0##*/}\" >&2\n",
            "  exit \"${1:-1}\"\n",
            "}\n",
            "function log { echo \"[$(date +%T)] $*\"; }\n",
            "while getopts 'vh' opt; do\n",
            "  case $opt in\n",
            "    v) verbose=1 ;;\n",
            "    h|*) usage 0 ;;\n",
            "  esac\n",
            "done\n",
            "shift $((OPTIND - 1))\n",
            "for f in \"$@\" *.txt; do\n",
            "  [[ -f $f && $f != *.bak ]] || continue\n",
            "  if (( ${#f} > 3 )); then echo \"${f%.txt}\" | tr a-z A-Z; fi\n",
            "done 2>/dev/null\n",
            "cat <<-EOF > \"$DIR/out\"\n",
            "\tname: ${USER}\n",
            "\tEOF\n",
            // the prototype's comment on the opening line keeps the here-document's string
            "cat <<'EOF'\t# literal\n",
            "$x\n",
            "EOF\n",
            "coproc worker { while read -r l; do echo \"$l\"; done; }\n",
            "x=`basename $PWD` y=$'tab\\there' z=~/src\n",
            "arr+=( \"${arr[@]/#/-I}\" ) && echo ${arr[*]:1:2} {a,b}{1..3}\n",
            "trap 'rm -f \"$tmp\"' EXIT; wait $!; exit $?\n",
        );
        rules_match_the_grammar("bash", "Bourne Again Shell (bash)", text);
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

    /// The themes' rule that colors a Lisp keyword's `:` as the keyword (X-009).
    const LISP_KEYWORD_MARK: &str = "keyword.other.symbol punctuation";

    /// The themes' rules that color a Lisp list's parens by depth (X-030).
    const LISP_DEPTHS: [&str; 6] = [
        "meta.depth-1 punctuation",
        "meta.depth-2 punctuation",
        "meta.depth-3 punctuation",
        "meta.depth-4 punctuation",
        "meta.depth-5 punctuation",
        "meta.depth-6 punctuation",
    ];

    fn rules_match_the_grammar(lang: &str, name: &str, text: &str) {
        rules_match_the_grammar_without(lang, name, text, &[]);
    }

    /// As [`rules_match_the_grammar`], each theme without the rules `left_out`: those that color
    /// a by-design departure from bat's scopes, so the rest is still compared rule for rule.
    fn rules_match_the_grammar_without(lang: &str, name: &str, text: &str, left_out: &[&str]) {
        assert_eq!(find_syntax(lang).unwrap().name(), name);
        for theme in Theme::builtins() {
            let mut syntax = theme.syntax.clone();
            let before = syntax.len();
            syntax.retain(|(scope, _)| !left_out.contains(&scope.as_str()));
            assert_eq!(
                syntax.len(),
                before - left_out.len(),
                "{}: {left_out:?}",
                theme.name
            );
            let theme = SyntaxTheme::new(&syntax);
            let want = highlight_syntect(reference_syntaxes(), name, text, &theme).unwrap();
            assert_eq!(painted(&highlight(1, lang, text, &theme)), painted(&want));
        }
    }

    /// As [`rules_match_the_grammar`], byte for byte rather than run for run: each byte's rule,
    /// the theme entry that colors it, must be syntect's, however either cuts its runs. For a
    /// scanner of ada's own whose kinds are a token's leaf scopes, not bat's stacks (X-057's C
    /// and C++), so its runs may part where the rule doesn't; `examples/parity.rs` measures it so.
    fn colors_match_the_grammar(lang: &str, name: &str, text: &str) {
        assert_eq!(find_syntax(lang).unwrap().name(), name);
        let by_byte = |spans: &CodeSpans, n: u32, len: usize| {
            let mut out = vec![Rule::NONE; len];
            let mut start = 0;
            for run in spans.line(n) {
                let end = (run.end as usize).min(len);
                out[start.min(end)..end].fill(run.rule.0);
                start = end;
            }
            out
        };
        // each line that differs, in the first theme it differs in: ada's rules, then syntect's
        let mut bad: Vec<(u32, String)> = Vec::new();
        for theme in Theme::builtins() {
            let rules = SyntaxTheme::new(&theme.syntax);
            let want = highlight_syntect(reference_syntaxes(), name, text, &rules).unwrap();
            let got = highlight(1, lang, text, &rules);
            for (n, line) in (0..).zip(text.lines()) {
                let differs = by_byte(&got, n, line.len()) != by_byte(&want, n, line.len());
                if differs && !bad.iter().any(|(m, _)| *m == n) {
                    let (ours, theirs) = (
                        rule_names(&theme, &got, line, n),
                        rule_names(&theme, &want, line, n),
                    );
                    let at = format!("{}: line {}", theme.name, n + 1);
                    bad.push((
                        n,
                        format!("{at}\n  ada:     {ours:?}\n  syntect: {theirs:?}"),
                    ));
                }
            }
        }
        bad.sort_unstable();
        let bad: Vec<String> = bad.into_iter().map(|(_, b)| b).collect();
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }

    /// A block's runs line by line, a line of plain runs as none: syntect's way gives a plain
    /// line its run, a lexer's leaves it empty, and both paint it plain.
    fn painted(spans: &CodeSpans) -> Vec<&[Run]> {
        (0..spans.line_count())
            .map(|n| spans.line(n))
            .map(|l| match l.iter().all(|r| r.rule.0 == Rule::NONE) {
                true => &[][..],
                false => l,
            })
            .collect()
    }

    /// G-002 on real Makefiles: `ADA_MAKE_CORPUS` names a file of paths, one a line, and each
    /// file's runs must be syntect's in every built-in theme. Files with a line over
    /// `MAX_LINE` are left out, as syntect leaves those lines plain and the lexer doesn't.
    /// Slow: `ADA_MAKE_CORPUS=list cargo test --release make_corpus -- --ignored`.
    #[test]
    #[ignore]
    fn make_corpus_matches_the_grammar() {
        corpus_matches_the_grammar("ADA_MAKE_CORPUS", "make");
    }

    /// G-003 on real shell scripts, as [`make_corpus_matches_the_grammar`]:
    /// `ADA_BASH_CORPUS=list cargo test --release bash_corpus -- --ignored`.
    #[test]
    #[ignore]
    fn bash_corpus_matches_the_grammar() {
        corpus_matches_the_grammar("ADA_BASH_CORPUS", "bash");
    }

    fn corpus_matches_the_grammar(var: &str, lang: &str) {
        let Some(list) = std::env::var_os(var) else {
            return;
        };
        let list = std::fs::read_to_string(list).unwrap();
        let name = find_syntax(lang).unwrap().name();
        let themes: Vec<SyntaxTheme> = Theme::builtins()
            .iter()
            .map(|t| SyntaxTheme::new(&t.syntax))
            .collect();
        let (mut files, mut bad) = (0, Vec::new());
        for path in list.lines() {
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            if Lines::new(&text).any(|l| expand_tabs(l).len() > MAX_LINE) {
                continue;
            }
            files += 1;
            for (t, theme) in themes.iter().enumerate() {
                let Some(want) = highlight_syntect(reference_syntaxes(), name, &text, theme) else {
                    break;
                };
                let want = want.lines;
                let got = highlight(1, lang, &text, theme).lines;
                // a line with no rule but none is one run in syntect's form, none in the lexer's
                let plain = |runs: &[Run]| runs.iter().all(|r| r.rule.0 == Rule::NONE);
                let same = |n: usize| match (got.line(n), want.line(n)) {
                    (Some(g), Some(w)) => g == w || (plain(g) && plain(w)),
                    _ => false,
                };
                if let Some(n) = (0..want.len()).find(|&n| !same(n)) {
                    bad.push(format!("{path}:{} in theme {t}", n + 1));
                    break;
                }
            }
        }
        assert!(
            bad.is_empty(),
            "{} of {files} files differ: {bad:?}",
            bad.len()
        );
        eprintln!("{files} files match in {} themes", themes.len());
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
        let ruby = "def f\n  s = <<~EOS\n    a \"quoted\" #{x}\n  EOS\n  # note\n  n = 0x1f\nend\n";
        let log =
            "2026-10-17 12:00:01 INFO started pid=42\n2026-10-17 12:00:02 ERROR failed: \"x\"\n";
        let swift = "let x = 1 // note\n";
        let json = "  {\"k\": [1, -2.5e3, true, \"s\\\"\"], \"open\": \"x\n  \"n\": null},\n";
        let diff = "--- a/x\n+++ b/x\n@@ -1 +1 @@ fn main\n-old\n+new\n same\n";
        let ini = "[core]\n; note\neditor = \"vim\" ; the one\n";
        for (lang, unit, reps) in [
            ("ruby", ruby, 60),
            ("log", log, 150),
            ("swift", swift, 300),
            ("json", json, 150),
            ("diff", diff, 60),
            ("ini", ini, 100),
        ] {
            let mut text = unit.repeat(reps);
            if lang == "swift" {
                text.insert_str(0, "/* never closed\n");
            }
            let one = highlight(1, lang, &text, &syntax);
            assert!(one.line_count() > 200, "{lang}: highlighted at all");
            for threads in 2..=5 {
                let many = match find_syntax(lang).unwrap() {
                    Syntax::Own(l) if streams(l) => {
                        highlight_parse_on(1, l, &text, &syntax, threads)
                    }
                    Syntax::Own(_) => panic!("{lang}: on one thread"),
                    Syntax::Set(found) => highlight_on(1, found, &text, &syntax, threads),
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

    /// With code enough for several threads, the block syntect gets stuck on prints plain, and
    /// every other block as one thread highlights it (F-06, PRN-01).
    #[test]
    fn print_on_several_threads_leaves_only_the_stuck_block_plain() {
        let theme = Arc::new(SyntaxTheme::new(&Theme::builtin().syntax));
        let mut text = format!("```rust\nfn first() {{}}\n```\n\n```scala\n{STUCK}```\n\n");
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
