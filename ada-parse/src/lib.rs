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
//! (ignore, attributes, mailmap and link files), fstab and `known_hosts` share the line-format engine
//! (`line_format.rs`); logs (O-043) and JSON (O-048) came from ada; Dockerfile's scanner is
//! bat's grammar compiled by hand; these are exact against syntect (the `parse` fuzz target),
//! and XML's, of ada's own design, is too; YAML's and TOML's read
//! their formats as bat's grammars do on well-formed text (TOML's as TOML 1.1 where bat's
//! doesn't). bash and Makefiles share a scanner, bat's contexts matched in place.

mod bash;
mod clike;
mod cpp;
mod csharp;
mod css;
mod diff;
mod dockerfile;
mod dotenv;
mod emit;
mod fstab;
// the matchers and context ids its scanner shares, from the port it replaced
mod git;
mod go;
mod html;
mod ini;
mod java;
mod javascript;
mod json;
mod known_hosts;
mod kotlin;
mod lang;
// the frame under INI, Java properties and `.env`
mod line_format;
mod lisp;
mod logs;
mod lua;
mod matcher;
mod powershell;
mod prim;
mod properties;
mod python;
mod regex;
mod rust;
mod sql;
mod ssh;
mod stack;
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
/// with the bytes beside them); nested deeper, the bottom ones are dropped and the lines
/// after may differ from syntect's ([`State::is_exact`]). Others keep a word of their own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct State {
    levels: [u8; matcher::KEPT],
    /// A byte a level beside `levels`, for the scanners that keep one.
    withs: [u8; matcher::KEPT],
    depth: u8,
    inexact: bool,
    /// A scanner's bits beside its stack ([`State::flags`]).
    arg: u16,
}

impl State {
    /// Whether the lines so far kept their whole stack, so that each got syntect's scopes.
    #[doc(hidden)]
    pub fn is_exact(&self) -> bool {
        !self.inexact
    }

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

    /// A scanner's bits beside its stack: Java's and Kotlin's SQL marker waiting for its string
    /// (phase 4).
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
/// space. Rust's scan writes the runs itself (X-068), as the line-format engine's do (X-069); any
/// other language's spans are read into them.
#[inline(never)]
pub fn lex_runs(
    lang: Lang,
    line: &str,
    state: &mut State,
    spans: &mut Vec<Span>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
) {
    if lang == Lang::Rust {
        spans.clear();
        return rust::line_runs(line.as_bytes(), state, &mut Out::new(spans), rules, runs);
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

/// As [`lex_lines`], but each line's runs by theme rule pushed to `runs`, as [`lex_runs`]
/// pushes them; `each` gets where the line's runs end, `runs`' length.
#[inline]
#[allow(clippy::too_many_arguments)]
pub fn lex_lines_runs(
    lang: Lang,
    text: &str,
    max: usize,
    state: &mut State,
    (spans, buf): (&mut Vec<Span>, &mut String),
    prep: impl FnMut(&str, &mut String) -> bool,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    mut each: impl FnMut(usize),
) -> Option<usize> {
    let mut prep = prep;
    if lang == Lang::Rust {
        // Rust's scan writes the runs (X-068), on the shared loop (X-073)
        return rust::block_runs(state, spans, rules, runs, &mut |c| {
            split_lines(text, max, buf, &mut prep, |l| each(c.next(l.as_bytes())))
        });
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
            each,
        );
    }
    if let Some(block) = carried::<false>(lang) {
        let mut t = emit::Table::new(rules);
        return block(state, spans, &mut |c| {
            split_lines(text, max, buf, &mut prep, |l| {
                each(emit::runs_of(l.len(), c.next(l.as_bytes()), &mut t, runs));
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
        each,
    )
}

// [`lex_lines_runs`]' paths whose loops would be inline in it each a function of their own
// (X-073): compiled in the caller's crate as the loop was, but each on its own registers, so
// that a path added or changed doesn't move another's per-line code (+7 instructions a line on
// YAML's when Makefile joined `carried`'s table)

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
    prep: impl FnMut(&str, &mut String) -> bool,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    mut each: impl FnMut(usize),
) -> Option<usize> {
    let mut k = emit::Runs::new(rules, runs);
    let mut out = Out::new(spans);
    split_lines(text, max, buf, prep, |l| {
        k.start_line();
        scan(l.as_bytes(), state, &mut out, &mut k);
        each(k.end_line(l.len()));
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
    prep: impl FnMut(&str, &mut String) -> bool,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    mut each: impl FnMut(usize),
) -> Option<usize> {
    let mut t = emit::Table::new(rules);
    split_lines(text, max, buf, prep, |l| {
        scan(l, state, spans);
        each(emit::runs_of(l.len(), spans, &mut t, runs));
    })
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
    let mut p = 0;
    for _ in 0..max {
        let (line, e) = line_at(text, p);
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

/// As [`Carry`], a scan writing a line's runs by theme rule ([`emit::Runs`], X-068): Rust's, on
/// the shared loop (X-073).
pub(crate) trait CarryRuns {
    /// `b`, the block's next line, lexed on the scan as [`lex_runs`] lexes it after the lines
    /// before, its runs pushed: where they end.
    fn next(&mut self, b: &[u8]) -> usize;
}

/// `lang`'s block on a carried scan, where it has one. Without `SPANS`, the run API's table,
/// the languages whose scans write runs are none, as in [`scanner`].
#[inline(always)]
fn carried<const SPANS: bool>(lang: Lang) -> Option<Block> {
    Some(match lang {
        Lang::Rust if SPANS => rust::block,
        Lang::Kotlin => kotlin::block,
        Lang::Java => java::block,
        Lang::Python => python::block,
        Lang::Go => go::block,
        Lang::CSharp => csharp::block,
        Lang::TypeScript => typescript::block,
        Lang::TypeScriptReact => typescript::block_tsx,
        Lang::JavaScript => javascript::block,
        Lang::Cpp => cpp::block,
        Lang::C => cpp::block_c,
        Lang::Bash => bash::block,
        Lang::Makefile => bash::make_block,
        Lang::Lua => lua::block,
        Lang::PowerShell => powershell::block,
        Lang::Css => css::block,
        Lang::Scss => css::scss::block,
        Lang::Html => html::block,
        Lang::Xml => xml_scan::block,
        Lang::Lisp => lisp::block_common,
        Lang::EmacsLisp => lisp::block_emacs,
        Lang::Scheme => lisp::block_scheme,
        Lang::Racket => lisp::block_racket,
        Lang::Clojure => lisp::clojure::block,
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

/// The line of `text` that starts at `p` (0 or just past a `\n`), a `\r` before its `\n`
/// dropped, and where it ends: its `\n`, or `text`'s end.
#[inline(always)]
fn line_at(text: &str, p: usize) -> (&str, usize) {
    let e = prim::find_any(text.as_bytes(), p, *b"\n");
    // SAFETY: `p` is 0 or just past a `\n`, and `e` at a `\n` or the end, `p <= e`: a `\n` is a
    // character of its own, so both ends are on characters of `text`
    let line = unsafe { text.get_unchecked(p..e) };
    (line.strip_suffix('\r').unwrap_or(line), e)
}

/// One line's lexer, as [`lex`] calls it.
type Scanner = fn(&str, &mut State, &mut Vec<Span>);

/// One line's lexer writing to a run sink, as [`lex_runs`] calls it (X-069).
type RunScanner = fn(&[u8], &mut State, &mut Out, &mut emit::Runs);

/// `lang`'s lexer on a run sink, where its scan writes runs a line at a time: the line-format
/// engine's (X-069). Rust's runs come from its carried scan ([`CarryRuns`], X-068, X-073).
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
        Lang::KnownHosts => known_hosts::line_runs,
        _ => return None,
    })
}

/// `lang`'s lexer: its scanner behind the spans' [`Out`]. Without `SPANS`, the run API's table,
/// the languages whose scans write runs (Rust's, X-068; [`run_scanner`]'s, X-069) are none, so
/// that a highlighter on runs doesn't carry their span scans.
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
        | Lang::KnownHosts
            if !SPANS =>
        {
            |_, _, _| unreachable!("a line format's runs come from its own scan")
        }
        Lang::Log => |l, _, v| logs::tokens(l, v),
        Lang::Json => |l, s, v| {
            // the only state a JSON line hands on: a `/* */` comment left open
            let mut comment = s.word() != 0;
            json::tokens(l, &mut comment, v);
            s.set_word(u64::from(comment));
        },
        Lang::Diff => |l, _, v| {
            v.clear();
            diff::line(l.as_bytes(), &mut Out::new(v));
        },
        Lang::Ini => out!(ini::line),
        Lang::Toml => out!(toml_scan::line),
        Lang::Yaml => out!(yaml_scan::line),
        Lang::Xml => out!(xml_scan::line),
        Lang::Rust if !SPANS => |_, _, _| unreachable!("Rust's runs come from its own scan"),
        Lang::Rust => out!(rust::line),
        Lang::Kotlin => out!(kotlin::line),
        Lang::Java => out!(java::line),
        Lang::Cpp => out!(cpp::line),
        Lang::C => out!(cpp::line_c),
        Lang::CSharp => out!(csharp::line),
        Lang::Go => out!(go::line),
        Lang::Python => out!(python::line),
        Lang::Lua => out!(lua::line),
        Lang::Sql => out!(sql::line),
        Lang::PowerShell => out!(powershell::line),
        Lang::JavaScript => out!(javascript::line),
        Lang::TypeScript => out!(|b, s, o| typescript::line(b, false, s, o)),
        Lang::TypeScriptReact => out!(|b, s, o| typescript::line(b, true, s, o)),
        Lang::Css => out!(css::line),
        Lang::Scss => out!(css::scss::line),
        Lang::Html => out!(html::line),
        Lang::Lisp => out!(|b, s, o| lisp::line(b, Dialect::Common, s, o)),
        Lang::EmacsLisp => out!(|b, s, o| lisp::line(b, Dialect::Emacs, s, o)),
        Lang::Scheme => out!(|b, s, o| lisp::line(b, Dialect::Scheme, s, o)),
        Lang::Racket => out!(|b, s, o| lisp::line(b, Dialect::Racket, s, o)),
        Lang::Clojure => out!(lisp::clojure::line),
        Lang::Bash => out!(bash::line),
        Lang::Makefile => out!(bash::make_line),
        Lang::DotEnv => out!(dotenv::line),
        Lang::GitIgnore => out!(git::ignore),
        Lang::GitAttributes => out!(git::attributes),
        Lang::GitMailmap => out!(git::mailmap),
        Lang::GitLink => out!(git::link),
        Lang::Fstab => out!(fstab::line),
        Lang::KnownHosts => out!(known_hosts::line),
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

    /// How many spans there are.
    fn len(&self) -> usize {
        self.0.len()
    }

    /// Drops the spans past the first `len`.
    fn truncate(&mut self, len: usize) {
        self.0.truncate(len);
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
}
