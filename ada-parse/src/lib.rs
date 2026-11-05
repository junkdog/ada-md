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
//! Diff is a line lexer, each line on its own; INI, Java properties and `.env` share the
//! line-format engine (`line_format.rs`); logs (O-043) and JSON (O-048) came from ada; `.env`'s,
//! Git's (ignore, attributes, mailmap and link files), Dockerfile's and Java properties'
//! scanners are bat's grammars compiled by hand, exact against syntect (the `parse` fuzz
//! target), as INI's is; XML's is of ada's own design and still exact; YAML's and TOML's read
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
// the matchers and context ids its scanner shares, from the port it replaced
mod git;
mod git_scan;
mod go;
mod html;
mod ini;
mod java;
mod javascript;
mod json;
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

use git_scan::Git;
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

    fn levels(&self) -> &[u8] {
        &self.levels[..usize::from(self.depth)]
    }

    /// A stack to keep: its top [`matcher::KEPT`] levels.
    fn pack(levels: &[u8], withs: &[u8], arg: u16, exact: bool) -> State {
        let from = levels.len().saturating_sub(matcher::KEPT);
        let n = levels.len() - from;
        let mut state = State {
            levels: [0; matcher::KEPT],
            withs: [0; matcher::KEPT],
            depth: n as u8,
            inexact: !exact || from > 0,
            arg,
        };
        state.levels[..n].copy_from_slice(&levels[from..]);
        state.withs[..n].copy_from_slice(&withs[from..]);
        state
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
    match lang {
        Lang::Log => return logs::tokens(line, spans),
        Lang::Json => {
            // the only state a JSON line hands on: a `/* */` comment left open
            let mut comment = state.word() != 0;
            json::tokens(line, &mut comment, spans);
            return state.set_word(u64::from(comment));
        }
        _ => {}
    }
    spans.clear();
    let mut out = Out::new(spans);
    match lang {
        Lang::Diff => diff::line(line.as_bytes(), &mut out),
        Lang::Ini => ini::line(line.as_bytes(), state, &mut out),
        Lang::Toml => toml_scan::line(line.as_bytes(), state, &mut out),
        Lang::Yaml => yaml_scan::line(line.as_bytes(), state, &mut out),
        Lang::Xml => xml_scan::line(line.as_bytes(), state, &mut out),
        Lang::Rust => rust::line(line.as_bytes(), state, &mut out),
        Lang::Kotlin => kotlin::line(line.as_bytes(), state, &mut out),
        Lang::Java => java::line(line.as_bytes(), state, &mut out),
        Lang::Cpp => cpp::line(line.as_bytes(), state, &mut out),
        Lang::C => cpp::line_c(line.as_bytes(), state, &mut out),
        Lang::CSharp => csharp::line(line.as_bytes(), state, &mut out),
        Lang::Go => go::line(line.as_bytes(), state, &mut out),
        Lang::Python => python::line(line.as_bytes(), state, &mut out),
        Lang::Lua => lua::line(line.as_bytes(), state, &mut out),
        Lang::Sql => sql::line(line.as_bytes(), state, &mut out),
        Lang::PowerShell => powershell::line(line.as_bytes(), state, &mut out),
        Lang::JavaScript => javascript::line(line.as_bytes(), state, &mut out),
        Lang::TypeScript => typescript::line(line.as_bytes(), false, state, &mut out),
        Lang::TypeScriptReact => typescript::line(line.as_bytes(), true, state, &mut out),
        Lang::Css => css::line(line.as_bytes(), state, &mut out),
        Lang::Scss => css::scss::line(line.as_bytes(), state, &mut out),
        Lang::Html => html::line(line.as_bytes(), state, &mut out),
        Lang::Lisp => lisp::line(line.as_bytes(), Dialect::Common, state, &mut out),
        Lang::EmacsLisp => lisp::line(line.as_bytes(), Dialect::Emacs, state, &mut out),
        Lang::Scheme => lisp::line(line.as_bytes(), Dialect::Scheme, state, &mut out),
        Lang::Racket => lisp::line(line.as_bytes(), Dialect::Racket, state, &mut out),
        Lang::Clojure => lisp::clojure::line(line.as_bytes(), state, &mut out),
        Lang::Bash => bash::line(line.as_bytes(), state, &mut out),
        Lang::Makefile => bash::make_line(line.as_bytes(), state, &mut out),
        Lang::DotEnv => dotenv::line(line.as_bytes(), state, &mut out),
        Lang::GitIgnore => git_scan::line(Git::Ignore, line.as_bytes(), state, &mut out),
        Lang::GitAttributes => git_scan::line(Git::Attributes, line.as_bytes(), state, &mut out),
        Lang::GitMailmap => git_scan::line(Git::Mailmap, line.as_bytes(), state, &mut out),
        Lang::GitLink => git_scan::line(Git::Link, line.as_bytes(), state, &mut out),
        Lang::Dockerfile => dockerfile::line(line.as_bytes(), state, &mut out),
        Lang::Properties => properties::line(line.as_bytes(), state, &mut out),
        Lang::Log | Lang::Json => unreachable!(),
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
}
