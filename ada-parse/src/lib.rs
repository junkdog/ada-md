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
//! Diff and INI are line lexers, each line on its own; logs (O-043) and JSON (O-048) came
//! from ada; TOML's and XML's scanners are bat's grammars compiled by hand, exact against their
//! ports; YAML's reads YAML as bat's grammar does on well-formed text. The other formats whose
//! lines carry state (Dockerfile, Java properties, `.env`, Git's ignore, attributes, mailmap and
//! link files) still run on a port of syntect's parse machine (`machine.rs`), their grammars bat's
//! context for context, until scanners replace them. Makefiles and bash are still lexed in ada.

mod cpp;
mod diff;
mod dockerfile;
mod dotenv;
mod git;
mod ini;
mod java;
mod json;
mod kotlin;
mod lang;
mod logs;
mod machine;
mod properties;
mod rust;
// the port: the scanner's reference in tests, and the source of helpers it shares
#[allow(dead_code)]
mod toml;
mod toml_scan;
// the port: the scanner's reference in tests, and the source of the matchers it shares
#[allow(dead_code)]
mod xml;
mod xml_scan;
// the port, replaced by the scanner (speed first, ADR-40); kept for the matchers the scanner
// shares, until they move into it
#[allow(dead_code)]
mod yaml;
mod yaml_scan;

pub use lang::{ALIASES, Lang};

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
/// For a language on the machine (`machine.rs`), the stack of contexts between lines, up to
/// 14 levels; nested deeper, the bottom ones are dropped and the lines after
/// may differ from syntect's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct State {
    levels: [u8; machine::KEPT],
    /// Each level's `with_prototype`, plus 1; 0 for none.
    withs: [u8; machine::KEPT],
    depth: u8,
    inexact: bool,
    /// The back-reference of the topmost level that has one ([`machine::Line::arg`]).
    arg: u16,
}

impl State {
    /// Whether the lines so far kept their whole stack, so that each got syntect's scopes.
    #[doc(hidden)]
    pub fn is_exact(&self) -> bool {
        !self.inexact
    }

    fn exact(&self) -> bool {
        !self.inexact
    }

    /// A scanner's state (a language not on the machine): a word of its own, 0 at a block's
    /// start, kept in the bytes the machine's stack would take.
    fn word(&self) -> u64 {
        let mut b = [0; 8];
        b.copy_from_slice(&self.levels[..8]);
        u64::from_le_bytes(b)
    }

    fn set_word(&mut self, word: u64) {
        self.levels[..8].copy_from_slice(&word.to_le_bytes());
    }

    fn levels(&self) -> &[u8] {
        &self.levels[..usize::from(self.depth)]
    }

    fn withs(&self) -> &[u8] {
        &self.withs[..usize::from(self.depth)]
    }

    /// A stack to keep: its top [`machine::KEPT`] levels.
    fn pack(levels: &[u8], withs: &[u8], arg: u16, exact: bool) -> State {
        let from = levels.len().saturating_sub(machine::KEPT);
        let n = levels.len() - from;
        let mut state = State {
            levels: [0; machine::KEPT],
            withs: [0; machine::KEPT],
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
/// are plain. A language ada lexes itself (Makefile, bash) gives no spans.
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
    let mut out = Out(spans);
    match lang {
        Lang::Diff => diff::line(line.as_bytes(), &mut out),
        Lang::Ini => ini::line(line.as_bytes(), &mut out),
        Lang::Toml => toml_scan::line(line.as_bytes(), state, &mut out),
        Lang::Yaml => yaml_scan::line(line.as_bytes(), state, &mut out),
        Lang::Xml => xml_scan::line(line.as_bytes(), state, &mut out),
        Lang::Rust => rust::line(line.as_bytes(), state, &mut out),
        Lang::Kotlin => kotlin::line(line.as_bytes(), state, &mut out),
        Lang::Java => java::line(line.as_bytes(), state, &mut out),
        Lang::Cpp => cpp::line(line.as_bytes(), state, &mut out),
        _ => {
            if let Some(g) = lang.grammar() {
                machine::line(g, line.as_bytes(), state, &mut out);
            }
        }
    }
}

/// A line's spans as a lexer pushes them.
struct Out<'a>(&'a mut Vec<Span>);

impl Out<'_> {
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
