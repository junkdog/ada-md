//! ada's own highlighting for the most used languages, with syntect and Oniguruma left as the
//! fallback for the long tail (ADR-40).
//!
//! The [language table](Lang) names the languages ada highlights without syntect, from a
//! fence's token, a file's name or its first line, so that nothing loads syntect's syntax set to
//! find out a language is one of them. [`lex`] highlights a line of one: it gives the line's
//! [`Span`]s, each a kind of the language, and [`Lang::scopes`] gives the scopes each kind
//! carries, bat's scopes for the same text, so that themes written for bat's grammars apply.
//!
//! The lexers are scanners of ada's own, speed first (ADR-40): one pass over a line's bytes,
//! a fixed set of kinds carrying bat's scopes for the common case, syntect a guideline.
//! Diff is a line lexer, each line on its own; INI, Java properties, `.env`, Git's own files
//! (ignore, attributes, mailmap and link files), fstab, `known_hosts`, OpenSSH's configs and crontabs share the line-format engine
//! (`line_format.rs`); logs (O-043) and JSON (O-048) came from ada; Dockerfile's scanner is
//! bat's grammar compiled by hand, and XML's of ada's own design; YAML's and TOML's read
//! their formats as bat's grammars do on well-formed text (TOML's as TOML 1.1 where bat's
//! doesn't). bash's scanner is ada's own (X-093); Makefile's is too (X-097), its recipes and
//! other shell on bash's (X-094). Each is held by checks of ada's own, not by syntect (X-116):
//! its unit tests, `tests/readings.rs`' snapshots of its readings, and the `tokens` fuzz target's
//! invariants.

mod bash;
mod cc;
mod clike;
mod crontab;
mod cs;
mod css;
mod diff;
mod dockerfile;
mod dotenv;
mod emit;
mod fstab;
// the matchers and context ids its scanner shares, from the port it replaced
mod git;
mod go;
mod header;
mod heredoc;
mod html;
mod ini;
mod java;
mod javascript;
mod json;
mod known_hosts;
mod kotlin;
mod lang;
// the frame under INI, Java properties and `.env`
mod cmake;
mod cmake_cache;
mod haskell;
mod line_format;
mod lisp;
mod logs;
mod lua;
mod matcher;
mod nginx;
mod php;
mod powershell;
mod prim;
mod properties;
mod python;
mod regex;
mod ruby;
mod rust;
mod scala;
mod sql;
mod ssh;
mod ssh_config;
mod stack;
mod swift;
mod terraform;
// the matchers and context ids its scanner shares, from the port it replaced
mod toml;
mod toml_scan;
mod typescript;
mod words;
// the matchers and context ids its scanner shares, from the port it replaced
mod xml;
mod xml_scan;
// the matchers its scanner shares, from the port it replaced
mod yaml;
mod yaml_scan;

use std::mem::MaybeUninit;

use emit::Sink as _;
pub use emit::{Rule, Rules, Run};
pub use lang::{ALIASES, FirstLine, Lang, first_line_syntaxes};
use lisp::Dialect;

/// Bytes `start..end` of a line are kind `kind` of the line's language, whose scopes
/// [`Lang::scopes`] gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub kind: u16,
}

/// What one line of a language leaves the next: an open string, an array or table still open.
/// Small and `Copy`, so a line's highlighting can start on any thread from the state before it.
/// A new block starts from `State::default()`.
///
/// A scanner keeps a stack of its contexts here, up to 14 levels (twice that
/// with the bytes beside them); nested deeper, the bottom ones are dropped. Others keep a word
/// of their own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct State {
    levels: [u8; matcher::KEPT],
    /// A byte a level beside `levels`, for the scanners that keep one.
    withs: [u8; matcher::KEPT],
    depth: u8,
    /// A scanner's bits beside its stack ([`State::flags`]).
    arg: u16,
}

impl State {
    /// A scanner's state kept as a word of its own, 0 at a block's start, in the bytes a stack
    /// would take.
    fn word(&self) -> u64 {
        let mut b = [0; 8];
        b.copy_from_slice(&self.levels[..8]);
        u64::from_le_bytes(b)
    }

    fn set_word(&mut self, word: u64) {
        self.levels[..8].copy_from_slice(&word.to_le_bytes());
    }

    /// A scanner's bits beside its stack: Java's SQL marker waiting for its string (phase 4).
    fn flags(&self) -> u16 {
        self.arg
    }

    fn set_flags(&mut self, flags: u16) {
        self.arg = flags;
    }
}

/// Lex one `line`, without its newline, into `spans` (cleared first), after the lines before
/// it left `state`; `state` is left as this line leaves it. The spans come in order, don't
/// overlap, aren't empty, and start and end on characters of the line; bytes no span covers
/// are plain.
// one copy, called from each place that highlights; never inlined into those (ADR-40's size
// discipline)
#[inline(never)]
pub fn lex(lang: Lang, line: &str, state: &mut State, spans: &mut Vec<Span>) {
    scanner::<true>(lang)(line, state, spans);
}

/// As [`lex`], but the line's runs by theme rule pushed to `runs`, as a highlighter builds them
/// from its spans: a span's rule is `rules`' for its kind, a gap between spans is
/// [`Rule::NONE`]'s, a run joins the one before where their rules are the same, a line where no
/// rule is met has no runs, and one where one is ends at the line's end. `spans` is scratch
/// space. Rust's scan writes the runs itself (X-068), as the line-format engine's do (X-069),
/// JSON's, TOML's and XML's (X-072), X-074's carried scans (Java's, C#'s, Kotlin's,
/// TypeScript's, Python's, the Lisp engine's, Clojure's) and X-075's (Go's, Lua's, SCSS's,
/// bash's); any other language's spans (Makefile's since X-094)
/// are read into them.
#[inline(never)]
pub fn lex_runs(
    lang: Lang,
    line: &str,
    state: &mut State,
    spans: &mut Vec<Span>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
) {
    if let Some((_, scan)) = carried_runs(lang) {
        spans.clear();
        return scan(line.as_bytes(), state, &mut Out::new(spans), rules, runs);
    }
    if let Some(scan) = run_scanner(lang) {
        spans.clear();
        let mut k = emit::Runs::new(rules, runs);
        scan(line.as_bytes(), state, &mut Out::new(spans), &mut k);
        k.end_line(line.len());
        return;
    }
    scanner::<false>(lang)(line, state, spans);
    emit::runs_of(line.len(), spans, &mut emit::Table::new(rules), runs);
}

/// Lex up to `max` of `text`'s lines from `state`, each as [`lex`] lexes it, calling `each`
/// after each with the line and its spans. A line ends at a `\n`, a `\r` before it dropped;
/// `prep` may write a line's replacement into `buf` and return true, and the replacement is
/// lexed and handed to `each` in its place. Returns where the lines left start, or `None` once
/// `text`'s last line (one without a `\n`) is done. The language's scanner is found once a
/// call, not once a line (HL-006).
#[inline]
pub fn lex_lines(
    lang: Lang,
    text: &str,
    max: usize,
    state: &mut State,
    (spans, buf): (&mut Vec<Span>, &mut String),
    prep: impl FnMut(&str, &mut String) -> bool,
    each: impl FnMut(&str, &[Span]),
) -> Option<usize> {
    let (mut prep, mut each) = (prep, each);
    if let Some(block) = carried::<true>(lang) {
        // a carried scanner's: one scan across the lines, through one loop (X-067)
        return block(state, spans, &mut |c| {
            split_lines(text, max, buf, &mut prep, |l| each(l, c.next(l.as_bytes())))
        });
    }
    let scan = scanner::<true>(lang);
    split_lines(text, max, buf, prep, |l| {
        scan(l, state, spans);
        each(l, spans);
    })
}

/// A line's replacement before it's lexed, as [`lex_lines`]' `prep`: written into the buffer,
/// true when it was. A plain function, so that a carried scan's loop is compiled with the scan
/// (X-086).
pub type Prep = fn(&str, &mut String) -> bool;

/// As [`lex_lines`], but each line's runs by theme rule pushed to `runs`, as [`lex_runs`]
/// pushes them, and where the line's runs end (`runs`' length) pushed to `ends`; a line is
/// replaced by `prep`'s where it has one.
#[allow(clippy::too_many_arguments)]
pub fn lex_lines_runs(
    lang: Lang,
    text: &str,
    max: usize,
    state: &mut State,
    (spans, buf): (&mut Vec<Span>, &mut String),
    prep: Option<Prep>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    ends: &mut Vec<u32>,
) -> Option<usize> {
    if let Some((block, _)) = carried_runs(lang) {
        // a carried scan writing the runs (X-068, X-074), the loop compiled with it (X-086)
        let mut lines = RunLines {
            text,
            max,
            buf,
            prep,
            ends,
        };
        return block(state, spans, rules, runs, &mut lines);
    }
    if let Some(scan) = run_scanner(lang) {
        return lines_run_scanned(
            scan,
            text,
            max,
            state,
            (spans, buf),
            prep,
            rules,
            runs,
            ends,
        );
    }
    if let Some(block) = carried::<false>(lang) {
        let mut t = emit::Table::new(rules);
        return block(state, spans, &mut |c| {
            split_lines(text, max, buf, prepped(prep), |l| {
                ends.push(run_count(emit::runs_of(
                    l.len(),
                    c.next(l.as_bytes()),
                    &mut t,
                    runs,
                )));
            })
        });
    }
    let scan = scanner::<false>(lang);
    lines_scanned(
        scan,
        text,
        max,
        state,
        (spans, buf),
        prep,
        rules,
        runs,
        ends,
    )
}

// [`lex_lines_runs`]' paths whose loops would be inline in it each a function of their own
// (X-073): each on its own registers, so that a path added or changed doesn't move another's
// per-line code (+7 instructions a line on YAML's when Makefile joined `carried`'s table)

/// [`lex_lines_runs`] on a run scanner: a line at a time on one run sink, opened once a block
/// (X-069).
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn lines_run_scanned(
    scan: RunScanner,
    text: &str,
    max: usize,
    state: &mut State,
    (spans, buf): (&mut Vec<Span>, &mut String),
    prep: Option<Prep>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    ends: &mut Vec<u32>,
) -> Option<usize> {
    let mut k = emit::Runs::new(rules, runs);
    let mut out = Out::new(spans);
    split_lines(text, max, buf, prepped(prep), |l| {
        k.start_line();
        scan(l.as_bytes(), state, &mut out, &mut k);
        ends.push(run_count(k.end_line(l.len())));
    })
}

/// [`lex_lines_runs`] on a line's scanner, its spans read into runs.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn lines_scanned(
    scan: Scanner,
    text: &str,
    max: usize,
    state: &mut State,
    (spans, buf): (&mut Vec<Span>, &mut String),
    prep: Option<Prep>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    ends: &mut Vec<u32>,
) -> Option<usize> {
    let mut t = emit::Table::new(rules);
    split_lines(text, max, buf, prepped(prep), |l| {
        scan(l, state, spans);
        ends.push(run_count(emit::runs_of(l.len(), spans, &mut t, runs)));
    })
}

/// [`lex_lines_runs`]' `prep` as [`split_lines`] takes it: a line without one is lexed as it is.
#[inline(always)]
fn prepped(prep: Option<Prep>) -> impl FnMut(&str, &mut String) -> bool {
    move |l, buf| prep.is_some_and(|p| p(l, buf))
}

/// Where a line's runs end, as [`lex_lines_runs`] pushes it to `ends`.
#[inline(always)]
fn run_count(n: usize) -> u32 {
    // a run is a byte or more of the text, so this takes a block of over 4 GiB
    u32::try_from(n).expect("a block of under 2³² runs")
}

/// A block's lines as a carried scan writing runs lexes them ([`lex_lines_runs`]): up to `max`
/// of `text`'s lines, each `prep`'d, where its runs end pushed to `ends`. The loop is compiled
/// with each scan ([`RunLines::pass`], X-086), so a line is no table call and the scan's `next`
/// is inline in it; the loop was in the caller's crate, the scan called a line through
/// [`CarryRuns`]' table (X-073).
pub(crate) struct RunLines<'t, 'a> {
    text: &'t str,
    max: usize,
    buf: &'a mut String,
    prep: Option<Prep>,
    ends: &'a mut Vec<u32>,
}

impl RunLines<'_, '_> {
    /// The lines lexed on `c`, a scan opened for the block: where the lines left start, as
    /// [`lex_lines`] returns it.
    #[inline(always)]
    pub(crate) fn pass(&mut self, c: &mut impl CarryRuns) -> Option<usize> {
        let ends = &mut *self.ends;
        split_lines(self.text, self.max, self.buf, prepped(self.prep), |l| {
            ends.push(run_count(c.next(l.as_bytes())));
        })
    }
}

/// [`lex_lines`]' loop: each of up to `max` lines split, `prep`'d and handed to `lex`.
#[inline(always)]
fn split_lines(
    text: &str,
    max: usize,
    buf: &mut String,
    mut prep: impl FnMut(&str, &mut String) -> bool,
    mut lex: impl FnMut(&str),
) -> Option<usize> {
    let mut nl = Newlines::new(text.as_bytes());
    let mut p = 0;
    for _ in 0..max {
        let e = nl.next();
        let line = line_to(text, p, e);
        lex(if prep(line, buf) { buf.as_str() } else { line });
        if e == text.len() {
            return None;
        }
        p = e + 1;
    }
    Some(p)
}

/// A scanner's scan kept across a block's lines (X-067, HL-006's stage B in X-066's shape): its
/// stack and what a line hands the next stay where they are, a line resets only what a line
/// starts with, and a [`State`] is read once at the block's start and made once at its end.
/// Each line ends as a state's save and load would leave it (past the levels a state holds, the
/// bottom ones dropped), so a block lexes as its lines do one at a time.
pub(crate) trait Carry {
    /// `b`, the block's next line, lexed on the scan as [`lex`] lexes it after the lines
    /// before: its spans.
    fn next(&mut self, b: &[u8]) -> &[Span];
}

/// A carried scanner's block: its scan opened from the state on the spans' vector, handed to
/// the loop (compiled in the caller's crate; the scanner's code is in this one, called through
/// [`Carry`]'s table), and the state left as the last line leaves it.
type Block = fn(
    &mut State,
    &mut Vec<Span>,
    &mut dyn FnMut(&mut dyn Carry) -> Option<usize>,
) -> Option<usize>;

/// As [`Carry`], a scan writing a line's runs by theme rule ([`emit::Runs`], X-068): Rust's
/// (X-073), X-074's and X-075's ([`carried_runs`]), each a line of [`RunLines::pass`]' loop,
/// compiled with the scan (X-086).
pub(crate) trait CarryRuns {
    /// `b`, the block's next line, lexed on the scan as [`lex_runs`] lexes it after the lines
    /// before, its runs pushed: where they end.
    fn next(&mut self, b: &[u8]) -> usize;
}

/// A carried scanner's block on a run sink ([`CarryRuns`]), as [`Block`] on the spans: the runs
/// pushed to the vector by the rules, its lines' loop compiled with the scan ([`RunLines`]).
type RunBlock =
    fn(&mut State, &mut Vec<Span>, &mut dyn Rules, &mut Vec<Run>, &mut RunLines) -> Option<usize>;

/// A carried scanner's line on a run sink, as [`lex_runs`] calls it.
type LineRuns = fn(&[u8], &mut State, &mut Out, &mut dyn Rules, &mut Vec<Run>);

/// `lang`'s block and line on a carried scan that writes runs: Rust's (X-068, X-073), X-074's
/// (Java's, C#'s, Kotlin's, TypeScript's and TSX's, Python's, the Lisp engine's and Clojure's),
/// X-075's (Go's, Lua's, SCSS's, bash's: since X-094 its one copy of the scan, which its span
/// API and Makefile's recipes read back as spans; CSS's span copy stays in ada for
/// HTML's embeds and SQL's for its hosts', so their runs are read from their spans, and
/// PowerShell's ran no faster), and X-057's (C++'s and C's).
#[inline(always)]
fn carried_runs(lang: Lang) -> Option<(RunBlock, LineRuns)> {
    Some(match lang {
        Lang::Rust => (rust::block_runs, rust::line_runs),
        Lang::Java => (java::block_runs, java::line_runs),
        Lang::CSharp => (cs::block_runs, cs::line_runs),
        Lang::Kotlin => (kotlin::block_runs, kotlin::line_runs),
        Lang::TypeScript => (typescript::block_runs, typescript::line_runs),
        Lang::TypeScriptReact => (typescript::block_runs_tsx, typescript::line_runs_tsx),
        Lang::Python => (python::block_runs, python::line_runs),
        Lang::Lisp => (lisp::block_runs_common, lisp::line_runs_common),
        Lang::EmacsLisp => (lisp::block_runs_emacs, lisp::line_runs_emacs),
        Lang::Scheme => (lisp::block_runs_scheme, lisp::line_runs_scheme),
        Lang::Racket => (lisp::block_runs_racket, lisp::line_runs_racket),
        Lang::Clojure => (lisp::clojure::block_runs, lisp::clojure::line_runs),
        Lang::Go => (go::block_runs, go::line_runs),
        Lang::Lua => (lua::block_runs, lua::line_runs),
        Lang::Php => (php::block_runs, php::line_runs),
        Lang::Swift => (swift::block_runs, swift::line_runs),
        Lang::CMake => (cmake::block_runs, cmake::line_runs),
        Lang::Terraform => (terraform::block_runs, terraform::line_runs),
        Lang::Scala => (scala::block_runs, scala::line_runs),
        Lang::Haskell => (haskell::block_runs, haskell::line_runs),
        Lang::LiterateHaskell => (haskell::lhs::block_runs, haskell::lhs::line_runs),
        Lang::Ruby => (ruby::block_runs, ruby::line_runs),
        Lang::Scss => (css::scss::block_runs, css::scss::line_runs),
        Lang::Bash => (bash::block_runs, bash::line_runs),
        Lang::Cpp => (cc::block_runs, cc::line_runs),
        Lang::C => (cc::block_runs_c, cc::line_runs_c),
        Lang::ObjC => (cc::block_runs_objc, cc::line_runs_objc),
        Lang::ObjCpp => (cc::block_runs_objcpp, cc::line_runs_objcpp),
        Lang::JavaScript => (javascript::block_runs, javascript::line_runs),
        _ => return None,
    })
}

/// `lang`'s block on a carried scan, where it has one. Without `SPANS`, the run API's table,
/// the languages whose scans write runs are none, as in [`scanner`]: XML's runs are a line at a
/// time on [`run_scanner`]'s path, all its lines hand on being a word.
#[inline(always)]
fn carried<const SPANS: bool>(lang: Lang) -> Option<Block> {
    Some(match lang {
        Lang::Rust if SPANS => rust::block,
        Lang::Kotlin if SPANS => kotlin::block,
        Lang::Java if SPANS => java::block,
        Lang::Python if SPANS => python::block,
        Lang::Go if SPANS => go::block,
        Lang::CSharp if SPANS => cs::block,
        Lang::TypeScript if SPANS => typescript::block,
        Lang::TypeScriptReact if SPANS => typescript::block_tsx,
        Lang::JavaScript => javascript::block,
        Lang::Cpp if SPANS => cc::block,
        Lang::C if SPANS => cc::block_c,
        Lang::ObjC if SPANS => cc::block_objc,
        Lang::ObjCpp if SPANS => cc::block_objcpp,
        Lang::Bash if SPANS => bash::block,
        Lang::Lua if SPANS => lua::block,
        Lang::Php if SPANS => php::block,
        Lang::Swift if SPANS => swift::block,
        Lang::CMake if SPANS => cmake::block,
        Lang::Terraform if SPANS => terraform::block,
        Lang::Scala if SPANS => scala::block,
        Lang::Haskell if SPANS => haskell::block,
        Lang::LiterateHaskell if SPANS => haskell::lhs::block,
        Lang::Ruby if SPANS => ruby::block,
        Lang::PowerShell => powershell::block,
        Lang::Css => css::block,
        Lang::Scss if SPANS => css::scss::block,
        Lang::Html => html::block,
        Lang::Xml if SPANS => xml_scan::block,
        Lang::Lisp if SPANS => lisp::block_common,
        Lang::EmacsLisp if SPANS => lisp::block_emacs,
        Lang::Scheme if SPANS => lisp::block_scheme,
        Lang::Racket if SPANS => lisp::block_racket,
        Lang::Clojure if SPANS => lisp::clojure::block,
        _ => return None,
    })
}

/// `b` as a scan's line for one [`Carry::next`], which sets the scan's line back to an empty
/// one before it returns.
///
/// # Safety
///
/// The scan holding the slice drops it before `b`'s borrow ends.
#[inline(always)]
unsafe fn line_for<'b>(b: &[u8]) -> &'b [u8] {
    // SAFETY: the caller's
    unsafe { &*std::ptr::from_ref(b) }
}

/// The line of `text` from `p` (0 or just past a `\n`) to `e` (the next `\n`, or `text`'s
/// end), a `\r` before its `\n` dropped.
#[inline(always)]
fn line_to(text: &str, p: usize, e: usize) -> &str {
    debug_assert!(p <= e && e <= text.len());
    // SAFETY: `p` is 0 or just past a `\n`, and `e` at a `\n` or the end, `p <= e`
    // ([`Newlines::next`]): a `\n` is a character of its own, so both ends are on characters
    // of `text`
    let line = unsafe { text.get_unchecked(p..e) };
    line.strip_suffix('\r').unwrap_or(line)
}

/// A refill stops once more than this many `\n`s are held.
const NL_FULL: usize = 128;
/// How many `\n`s [`Newlines`] holds: past [`NL_FULL`], a step adding up to 64.
const NL_HELD: usize = NL_FULL + 64;
/// The 64-byte steps a refill takes at most: a fixed count on ordinary text, so its loop's exit
/// predicts.
const NL_STEPS: usize = 32;

/// A block's `\n`s, found 64 bytes a step a stretch ahead of its lines ([`split_lines`], X-087):
/// a line's end is the next one taken from the buffer. A search a line from the line's start
/// ended on a branch whose step count the line's length set, missed on most lines (35–40% of
/// Rust's per-line cycles and misses).
struct Newlines<'t> {
    b: &'t [u8],
    /// Where the search has reached: every `\n` before it is held or taken.
    at: usize,
    /// Where the held `\n`s' offsets count from.
    base: usize,
    /// The `\n`s found, `ends[taken..held]` not yet taken, at `base` plus each.
    ends: [MaybeUninit<u32>; NL_HELD],
    taken: usize,
    held: usize,
}

impl<'t> Newlines<'t> {
    #[inline(always)]
    fn new(b: &'t [u8]) -> Self {
        Newlines {
            b,
            at: 0,
            base: 0,
            ends: [MaybeUninit::uninit(); NL_HELD],
            taken: 0,
            held: 0,
        }
    }

    /// The next `\n` past the last one taken, or `b`'s end past the last.
    #[inline(always)]
    fn next(&mut self) -> usize {
        if self.taken == self.held && !self.refill() {
            return self.b.len();
        }
        // SAFETY: `taken < held`, and `refill` wrote `ends[..held]`
        let e = unsafe { self.ends.get_unchecked(self.taken).assume_init() };
        self.taken += 1;
        self.base + e as usize
    }

    /// The next stretch's `\n`s held, up to [`NL_STEPS`] steps or past [`NL_FULL`] found: false at
    /// `b`'s end with none left.
    #[inline(never)]
    fn refill(&mut self) -> bool {
        let b = self.b;
        loop {
            if self.at >= b.len() {
                return false;
            }
            let base = self.at;
            let mut at = base;
            let mut held = 0;
            // the steps that read 64 of `b`'s bytes, a fixed count but where `\n`s crowd
            let full = ((b.len() - at) / 64).min(NL_STEPS);
            let mut steps = 0;
            while steps < full && held <= NL_FULL {
                // SAFETY: `at + 64 <= b.len()`, from `full`
                let c = unsafe { &*b.as_ptr().add(at).cast::<[u8; 64]>() };
                // an offset under `NL_STEPS * 64`
                held = self.hold(prim::newlines64(c), (at - base) as u32, held);
                at += 64;
                steps += 1;
            }
            if steps == full && full < NL_STEPS && held <= NL_FULL && at < b.len() {
                // the tail, under 64 bytes, on a copy
                let mut c = [0; 64];
                c[..b.len() - at].copy_from_slice(&b[at..]);
                held = self.hold(prim::newlines64(&c), (at - base) as u32, held);
                at = b.len();
            }
            (self.at, self.base, self.taken, self.held) = (at, base, 0, held);
            if held != 0 {
                return true;
            }
        }
    }

    /// A step's `\n`s, `m` and their count `n`, held past the `held` already, at `off` from
    /// the base: how many are held now.
    #[inline(always)]
    fn hold(&mut self, (m, n): (u64, usize), off: u32, held: usize) -> usize {
        // four written whatever the count (most steps hold up to four: 94–98% of the code
        // lists'), so the loop's exit doesn't depend on it (simdjson's flattening); garbage past
        // `n` is overwritten or never read, so the bit or-ed in only spares `trailing_zeros`
        // its zero case
        let mut m = m;
        let mut w = held;
        for _ in 0..4 {
            self.ends[w] = MaybeUninit::new(off + (m | 1 << 63).trailing_zeros());
            m &= m.wrapping_sub(1);
            w += 1;
        }
        while m != 0 {
            self.ends[w] = MaybeUninit::new(off + m.trailing_zeros());
            m &= m - 1;
            w += 1;
        }
        held + n
    }
}

/// One line's lexer, as [`lex`] calls it.
type Scanner = fn(&str, &mut State, &mut Vec<Span>);

/// One line's lexer writing to a run sink, as [`lex_runs`] calls it (X-069).
type RunScanner = fn(&[u8], &mut State, &mut Out, &mut emit::Runs);

/// `lang`'s lexer on a run sink, where its scan writes runs a line at a time: the line-format
/// engine's (X-069), JSON's, TOML's and XML's (X-072; YAML's ran 1–2% slower in ada, X-072),
/// Dockerfile's (X-074; Diff's ran 3–4% slower in ada, its span API 10% slower alone; X-075's
/// SQL and log lexer ran slower in ada).
/// The carried scans' runs come from [`carried_runs`] (Rust's, X-068, X-073; X-074's).
#[inline(always)]
fn run_scanner(lang: Lang) -> Option<RunScanner> {
    Some(match lang {
        Lang::Ini => ini::line_runs,
        Lang::Properties => properties::line_runs,
        Lang::DotEnv => dotenv::line_runs,
        Lang::GitIgnore => git::ignore_runs,
        Lang::GitAttributes => git::attributes_runs,
        Lang::GitMailmap => git::mailmap_runs,
        Lang::GitLink => git::link_runs,
        Lang::Fstab => fstab::line_runs,
        Lang::CMakeCache => cmake_cache::line_runs,
        Lang::KnownHosts => known_hosts::line_runs,
        Lang::SshConfig => ssh_config::line_runs,
        Lang::SshdConfig => ssh_config::sshd_line_runs,
        Lang::Crontab => crontab::line_runs,
        Lang::Nginx => nginx::line_runs,
        Lang::Json => json::line_runs,
        Lang::Toml => toml_scan::line_runs,
        Lang::Xml => xml_scan::line_runs,
        Lang::Dockerfile => dockerfile::line_runs,
        _ => return None,
    })
}

/// `lang`'s lexer: its scanner behind the spans' [`Out`]. Without `SPANS`, the run API's table,
/// the languages whose scans write runs ([`carried_runs`]'; [`run_scanner`]'s, X-069, X-072) are
/// none, so that a highlighter on runs doesn't carry their span scans.
#[inline(always)]
fn scanner<const SPANS: bool>(lang: Lang) -> Scanner {
    macro_rules! out {
        ($f:expr) => {
            |l: &str, s: &mut State, v: &mut Vec<Span>| {
                v.clear();
                $f(l.as_bytes(), s, &mut Out::new(v))
            }
        };
    }
    match lang {
        Lang::Ini
        | Lang::Properties
        | Lang::DotEnv
        | Lang::GitIgnore
        | Lang::GitAttributes
        | Lang::GitMailmap
        | Lang::GitLink
        | Lang::Fstab
        | Lang::CMakeCache
        | Lang::KnownHosts
        | Lang::SshConfig
        | Lang::SshdConfig
        | Lang::Crontab
        | Lang::Nginx
            if !SPANS =>
        {
            |_, _, _| unreachable!("a line format's runs come from its own scan")
        }
        Lang::Log => |l, _, v| logs::tokens(l, v),
        Lang::Json if !SPANS => |_, _, _| unreachable!("JSON's runs come from its own scan"),
        Lang::Json => |l, s, v| {
            // the only state a JSON line hands on: a `/* */` comment left open
            let mut comment = s.word() != 0;
            json::tokens(l, &mut comment, v);
            s.set_word(u64::from(comment));
        },
        Lang::Dockerfile if !SPANS => {
            |_, _, _| unreachable!("Dockerfile's runs come from its own scan")
        }
        Lang::Diff => |l, _, v| {
            v.clear();
            diff::line(l.as_bytes(), &mut Out::new(v));
        },
        Lang::Ini => out!(ini::line),
        Lang::Toml if !SPANS => |_, _, _| unreachable!("TOML's runs come from its own scan"),
        Lang::Toml => out!(toml_scan::line),
        Lang::Yaml => out!(yaml_scan::line),
        Lang::Xml if !SPANS => |_, _, _| unreachable!("XML's runs come from its own scan"),
        Lang::Xml => out!(xml_scan::line),
        Lang::Rust if !SPANS => |_, _, _| unreachable!("Rust's runs come from its own scan"),
        Lang::Rust => out!(rust::line),
        Lang::Kotlin if !SPANS => |_, _, _| unreachable!("Kotlin's runs come from its own scan"),
        Lang::Kotlin => out!(kotlin::line),
        Lang::Java if !SPANS => |_, _, _| unreachable!("Java's runs come from its own scan"),
        Lang::Java => out!(java::line),
        Lang::Cpp => out!(cc::line),
        Lang::C => out!(cc::line_c),
        Lang::ObjC => out!(cc::line_objc),
        Lang::ObjCpp => out!(cc::line_objcpp),
        Lang::CMakeCHeader => out!(cc::line_cmake_c),
        Lang::CMakeCppHeader => out!(cc::line_cmake_cpp),
        Lang::CSharp if !SPANS => |_, _, _| unreachable!("C#'s runs come from its own scan"),
        Lang::CSharp => out!(cs::line),
        Lang::Go if !SPANS => |_, _, _| unreachable!("Go's runs come from its own scan"),
        Lang::Go => out!(go::line),
        Lang::Python if !SPANS => |_, _, _| unreachable!("Python's runs come from its own scan"),
        Lang::Python => out!(python::line),
        Lang::Lua if !SPANS => |_, _, _| unreachable!("Lua's runs come from its own scan"),
        Lang::Lua => out!(lua::line),
        Lang::Php if !SPANS => |_, _, _| unreachable!("PHP's runs come from its own scan"),
        Lang::Php => out!(php::line),
        Lang::Swift if !SPANS => |_, _, _| unreachable!("Swift's runs come from its own scan"),
        Lang::Swift => out!(swift::line),
        Lang::CMake if !SPANS => |_, _, _| unreachable!("CMake's runs come from its own scan"),
        Lang::CMake => out!(cmake::line),
        Lang::Terraform if !SPANS => {
            |_, _, _| unreachable!("Terraform's runs come from its own scan")
        }
        Lang::Terraform => out!(terraform::line),
        Lang::Scala if !SPANS => |_, _, _| unreachable!("Scala's runs come from its own scan"),
        Lang::Scala => out!(scala::line),
        Lang::Haskell if !SPANS => |_, _, _| unreachable!("Haskell's runs come from its own scan"),
        Lang::Haskell => out!(haskell::line),
        Lang::LiterateHaskell if !SPANS => {
            |_, _, _| unreachable!("Literate Haskell's runs come from its own scan")
        }
        Lang::LiterateHaskell => out!(haskell::lhs::line),
        Lang::Ruby if !SPANS => |_, _, _| unreachable!("Ruby's runs come from its own scan"),
        Lang::Ruby => out!(ruby::line),
        Lang::Sql => out!(sql::line),
        Lang::PowerShell => out!(powershell::line),
        Lang::JavaScript => out!(javascript::line),
        Lang::TypeScript | Lang::TypeScriptReact if !SPANS => {
            |_, _, _| unreachable!("TypeScript's runs come from its own scan")
        }
        Lang::TypeScript => out!(|b, s, o| typescript::line(b, false, s, o)),
        Lang::TypeScriptReact => out!(|b, s, o| typescript::line(b, true, s, o)),
        Lang::Scss if !SPANS => |_, _, _| unreachable!("SCSS's runs come from its own scan"),
        Lang::Css => out!(css::line),
        Lang::Scss => out!(css::scss::line),
        Lang::Html => out!(html::line),
        Lang::Lisp | Lang::EmacsLisp | Lang::Scheme | Lang::Racket | Lang::Clojure if !SPANS => {
            |_, _, _| unreachable!("Lisp's and Clojure's runs come from their own scans")
        }
        Lang::Lisp => out!(|b, s, o| lisp::line(b, Dialect::Common, s, o)),
        Lang::EmacsLisp => out!(|b, s, o| lisp::line(b, Dialect::Emacs, s, o)),
        Lang::Scheme => out!(|b, s, o| lisp::line(b, Dialect::Scheme, s, o)),
        Lang::Racket => out!(|b, s, o| lisp::line(b, Dialect::Racket, s, o)),
        Lang::Clojure => out!(lisp::clojure::line),
        Lang::Bash if !SPANS => |_, _, _| unreachable!("bash's runs come from its own scan"),
        Lang::Bash => out!(bash::line),
        Lang::Makefile => out!(bash::make_line),
        Lang::DotEnv => out!(dotenv::line),
        Lang::GitIgnore => out!(git::ignore),
        Lang::GitAttributes => out!(git::attributes),
        Lang::GitMailmap => out!(git::mailmap),
        Lang::GitLink => out!(git::link),
        Lang::Fstab => out!(fstab::line),
        Lang::CMakeCache => out!(cmake_cache::line),
        Lang::KnownHosts => out!(known_hosts::line),
        Lang::SshConfig => out!(ssh_config::line),
        Lang::SshdConfig => out!(ssh_config::sshd_line),
        Lang::Crontab => out!(crontab::line),
        Lang::Nginx => out!(nginx::line),
        Lang::Dockerfile => out!(dockerfile::line),
        Lang::Properties => out!(properties::line),
    }
}

/// A line's spans as a lexer pushes them; with `false`, [`Out::push_merged`] joins none (HTML's
/// embeds, which tell a token from a context's text by its span).
struct Out<'a>(&'a mut Vec<Span>, bool);

impl<'a> Out<'a> {
    fn new(spans: &'a mut Vec<Span>) -> Out<'a> {
        Out(spans, true)
    }

    /// Bytes `start..end` are `kind`; an empty stretch is skipped.
    fn push(&mut self, start: usize, end: usize, kind: u16) {
        if end > start {
            self.0.push(Span {
                start: start as u32,
                end: end as u32,
                kind,
            });
        }
    }

    /// As [`Out::push`], but a stretch right after one of the same kind joins it.
    fn push_merged(&mut self, start: usize, end: usize, kind: u16) {
        if let Some(last) = self.0.last_mut()
            && last.end as usize == start
            && last.kind == kind
            && self.1
        {
            last.end = end as u32;
            return;
        }
        self.push(start, end, kind);
    }
}

/// `\s` as far as ASCII goes.
fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

/// Whether `a` and `b` are equal, without a call to `memcmp` for a word or an operator: up to
/// 16 bytes, two loads a side that overlap in the middle; longer ones compare as slices.
#[inline(always)]
fn same(a: &[u8], b: &[u8]) -> bool {
    fn at<const N: usize>(s: &[u8], i: usize) -> [u8; N] {
        s[i..i + N].try_into().unwrap()
    }
    let n = a.len();
    if n != b.len() {
        return false;
    }
    match n {
        0 => true,
        1 => a[0] == b[0],
        2..4 => at::<2>(a, 0) == at::<2>(b, 0) && at::<2>(a, n - 2) == at::<2>(b, n - 2),
        4..8 => at::<4>(a, 0) == at::<4>(b, 0) && at::<4>(a, n - 4) == at::<4>(b, n - 4),
        8..=16 => at::<8>(a, 0) == at::<8>(b, 0) && at::<8>(a, n - 8) == at::<8>(b, n - 8),
        _ => a == b,
    }
}

/// Whether `b` has `s` at `i`, as [`same`] compares.
#[inline(always)]
fn has_at(b: &[u8], i: usize, s: &[u8]) -> bool {
    i <= b.len() && b.len() - i >= s.len() && same(&b[i..i + s.len()], s)
}

/// Whether `b` starts with `s`, as [`same`] compares.
#[inline(always)]
fn starts(b: &[u8], s: &[u8]) -> bool {
    has_at(b, 0, s)
}

/// Where the spaces at `at` in `b` end.
fn skip_spaces(b: &[u8], mut at: usize) -> usize {
    while at < b.len() && is_space(b[at]) {
        at += 1;
    }
    at
}

/// Where the spaces that end `b[..end]` start, not before `from`.
fn trim_end(b: &[u8], from: usize, mut end: usize) -> usize {
    while end > from && is_space(b[end - 1]) {
        end -= 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The marked pieces of `lines` in `lang`, as `(text, scopes)`, after checking the spans'
    /// order.
    pub fn marked<'a>(lang: Lang, lines: &[&'a str]) -> Vec<(&'a str, &'static str)> {
        let (mut spans, mut state, mut out) = (Vec::new(), State::default(), Vec::new());
        for line in lines {
            lex(lang, line, &mut state, &mut spans);
            let mut last = 0;
            for s in &spans {
                assert!(s.start >= last && s.end > s.start, "{line}: {spans:?}");
                last = s.end;
            }
            assert!(last as usize <= line.len());
            out.extend(spans.iter().map(|s| {
                let text = &line[s.start as usize..s.end as usize];
                (text, &*lang.scopes(s.kind).join(" ").leak())
            }));
        }
        out
    }

    /// [`Newlines`] gives each `\n` in turn, then the end: across its 64-byte steps, its
    /// refills (a long line, more than four `\n`s a step, more than [`NL_FULL`] a refill) and the
    /// tail.
    #[test]
    fn newlines_each_in_turn() {
        let mut texts: Vec<Vec<u8>> = vec![
            b"".to_vec(),
            b"\n".to_vec(),
            b"a".to_vec(),
            b"a\r\nb\n".to_vec(),
            vec![b'\n'; 1000],
            vec![b'x'; 5000],
        ];
        for len in [63, 64, 65, 127, 128, 129, 1023, 1024, 1025, 3000] {
            for every in [1, 2, 3, 7, 9, 36, 65, 700, 1100, 2048] {
                texts.push(
                    (0..len)
                        .map(|i| if i % every == every - 1 { b'\n' } else { b'a' })
                        .collect(),
                );
            }
        }
        let mut long = vec![b'a'; 2000];
        long.extend([b'\n'; 70]);
        long.extend(vec![b'b'; 1100]);
        long.push(b'\n');
        texts.push(long);
        for t in &texts {
            let want: Vec<usize> = (0..t.len())
                .filter(|&i| t[i] == b'\n')
                .chain([t.len()])
                .collect();
            let mut nl = Newlines::new(t);
            let got: Vec<usize> = (0..want.len()).map(|_| nl.next()).collect();
            assert_eq!(got, want, "{} bytes", t.len());
            assert_eq!(nl.next(), t.len(), "past the end");
        }
    }

    /// `lex_lines` lexes as `lex` does line by line in every language: the lines split at `\n`
    /// with a `\r` before it dropped, the state carried, `max` honored, and `prep`'s line lexed
    /// in place.
    #[test]
    fn lex_lines_matches_lex() {
        let text = "fn main() {\r\n    let s = r#\"raw\n\n\r\nstill\"#;\r\r\n\t// a tab\n}\n\nlast";
        let expand = |l: &str| l.replace('\t', "    ");
        // every language: the carried scanners' blocks (X-067) and the line-by-line ones
        for lang in Lang::ALL {
            let (mut want, mut state, mut spans) = (Vec::new(), State::default(), Vec::new());
            for line in text.split('\n') {
                let line = expand(line.strip_suffix('\r').unwrap_or(line));
                lex(lang, &line, &mut state, &mut spans);
                want.push((line, spans.clone()));
            }
            let want_state = state;
            for max in [1, 2, 3, usize::MAX] {
                let (mut got, mut state) = (Vec::new(), State::default());
                let (mut spans, mut buf) = (Vec::new(), String::new());
                let mut rest = Some(text);
                while let Some(t) = rest {
                    let prep = |l: &str, buf: &mut String| {
                        l.contains('\t') && {
                            *buf = expand(l);
                            true
                        }
                    };
                    let each = |l: &str, s: &[Span]| got.push((l.to_owned(), s.to_vec()));
                    let next =
                        lex_lines(lang, t, max, &mut state, (&mut spans, &mut buf), prep, each);
                    rest = next.map(|p| &t[p..]);
                }
                assert_eq!(got, want, "{lang:?} max {max}");
                assert_eq!(state, want_state, "{lang:?} max {max}");
            }
        }
    }

    /// An HTML block whose first line starts inside an embed, in a string continued by a `\` at
    /// the line before's end, lexes as its lines do one at a time: the block's scan is opened
    /// from the state without lexing an empty line, which ends the continuation (X-080, from
    /// F-018's input).
    #[test]
    fn html_block_opens_inside_an_embed() {
        let text =
            "<style>\na { content: 'a\\\nb'; }\n</style>\n<script>\nlet s = 'a\\\nb';\n</script>";
        let (mut want, mut state, mut spans) = (Vec::new(), State::default(), Vec::new());
        for line in text.split('\n') {
            lex(Lang::Html, line, &mut state, &mut spans);
            want.push(spans.clone());
        }
        let want_state = state;
        for max in [1, 2, usize::MAX] {
            let (mut got, mut state, mut buf) = (Vec::new(), State::default(), String::new());
            let mut rest = Some(text);
            while let Some(t) = rest {
                let each = |_: &str, s: &[Span]| got.push(s.to_vec());
                let next = lex_lines(
                    Lang::Html,
                    t,
                    max,
                    &mut state,
                    (&mut spans, &mut buf),
                    |_, _| false,
                    each,
                );
                rest = next.map(|p| &t[p..]);
            }
            assert_eq!(got, want, "max {max}");
            assert_eq!(state, want_state, "max {max}");
        }
    }
}
