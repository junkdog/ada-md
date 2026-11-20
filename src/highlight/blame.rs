//! `git blame`'s lines, its fields styled quietly and its code in its file's language (FEAT-026).
//! A line is a commit's hash (`^` before a boundary commit's), the path and the line's number in
//! that commit when `-f` and `-n` show them, then the author, date and the line's number in
//! parentheses (`-s` leaves only the number), and the line's text after a space; `git annotate`'s
//! (and `blame -c`'s) fields are split by tabs, its text right after the `)`.
//!
//! The text is lexed in the blamed file's language as a file's lines are, from line to line;
//! where the numbers jump (another `-L` range) the state starts fresh, as a diff's hunk does. Which
//! language is the type's name ([`name`]): the sniffer reads it when it types the input, from
//! the file `git blame` was asked for when ada is its pager, else from the paths `-f` shows, else
//! from the text itself ([`crate::sniff`]).
//!
//! The fields are a commit header's (FEAT-025's scopes): the hash in color, the first of a run of
//! lines from one commit alone, so a commit's lines read as a group (as git's
//! `blame.coloring = repeatedLines`, the other way round), and the rest dimmed; an uncommitted
//! line's all-zero hash is dimmed too. A line that isn't blame's (past the sniffer's sample)
//! shows plain, and the text's state stays as it was.

use std::sync::LazyLock;

use ada_parse::{Lang, Run, Span, State};

use super::diff::{Preamble, Runs};
use super::{LineRuns, Lines, ParseRules, Resolving, SyntaxTheme};

/// The type's name of blame whose file is in no language ada lexes.
pub const NAME: &str = "Git Blame";

/// [`name`]s, one for each of ada-parse's languages, in [`Lang::ALL`]'s order.
static NAMES: LazyLock<Vec<String>> = LazyLock::new(|| {
    Lang::ALL
        .iter()
        .map(|l| format!("{NAME} ({})", l.label()))
        .collect()
});

/// The type's name of blame whose file is in `lang`: `Git Blame (Rust)`, or [`NAME`] for none.
pub fn name(lang: Option<Lang>) -> &'static str {
    match lang.and_then(|l| Lang::ALL.iter().position(|&a| a == l)) {
        Some(i) => &NAMES[i],
        None => NAME,
    }
}

/// The language of blame by its type's name ([`name`]): `None` for a name that is no blame's,
/// `Some(None)` for blame of a file in no language ada lexes.
pub fn of_name(name: &str) -> Option<Option<Lang>> {
    let rest = name.strip_prefix(NAME)?;
    if rest.is_empty() {
        return Some(None);
    }
    let label = rest.strip_prefix(" (")?.strip_suffix(')')?;
    Lang::ALL
        .iter()
        .find(|l| l.label() == label)
        .map(|&l| Some(l))
}

/// Where a blame line's fields are, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fields {
    /// The hash's, after a boundary commit's `^`.
    pub hash: (usize, usize),
    /// The `(` before the author's name; `None` with `-s`.
    pub paren: Option<usize>,
    /// The line's number, before the `)` that ends the fields.
    pub number: (usize, usize),
    /// Where the line's text starts: past the `)` and the space after it.
    pub code: usize,
}

impl Fields {
    /// The line's number in the file, if it fits a `u32`.
    fn line_number(&self, line: &str) -> Option<u32> {
        line[self.number.0..self.number.1].parse().ok()
    }

    /// The path `-f` shows (git shows it by itself when `-C` finds a line in another file),
    /// without `-n`'s number after it.
    pub fn path<'a>(&self, line: &'a str) -> Option<&'a str> {
        let end = self.paren.unwrap_or(self.number.0);
        let shown = line[self.hash.1..end].trim();
        let shown = match shown.rsplit_once(' ') {
            Some((path, n)) if n.bytes().all(|b| b.is_ascii_digit()) => path.trim_end(),
            _ if shown.bytes().all(|b| b.is_ascii_digit()) => "",
            _ => shown,
        };
        Some(shown).filter(|p| !p.is_empty())
    }
}

/// The fields of `line`, a blame line, or `None` when it isn't one. `tabbed`: its fields are
/// split by tabs (`git annotate`, `blame -c`; [`is_tabbed`]), and its text starts right after the
/// `)`.
pub fn fields(line: &str, tabbed: bool) -> Option<Fields> {
    let b = line.as_bytes();
    let start = usize::from(b.first() == Some(&b'^'));
    let hex = b[start..]
        .iter()
        .take_while(|c| c.is_ascii_hexdigit())
        .count();
    // git abbreviates a hash to 4 digits at the least (`--abbrev=4`)
    let end = start + hex;
    if hex < 4 || !b.get(end).is_some_and(u8::is_ascii_whitespace) {
        return None;
    }
    // the fields end at the first `)` after a number: an author's can hold one of its own
    // (`--contents`' `External file (--contents)`)
    let mut from = end;
    let (number, close) = loop {
        let close = from + memchr::memchr(b')', &b[from..])?;
        let digits = b[..close]
            .iter()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .count();
        // the hash's whitespace ends a run of digits from the `)` back at the latest
        let number = close - digits;
        if digits > 0 && b[number - 1].is_ascii_whitespace() {
            break (number, close);
        }
        from = close + 1;
    };
    let paren = memchr::memchr(b'(', &b[end..number]).map(|i| end + i);
    let space = !tabbed && b.get(close + 1) == Some(&b' ');
    Some(Fields {
        hash: (start, end),
        paren,
        number: (number, close),
        code: close + 1 + usize::from(space),
    })
}

/// Whether `line`, as the input has it, is `git annotate`'s: a tab after its hash.
pub fn is_tabbed(line: &str) -> bool {
    let b = line.as_bytes();
    let start = usize::from(b.first() == Some(&b'^'));
    let hex = b[start..]
        .iter()
        .take_while(|c| c.is_ascii_hexdigit())
        .count();
    b.get(start + hex) == Some(&b'\t')
}

/// Blame's reading carried from line to line.
pub(super) struct Blame {
    /// The file's language's rules; `None` for a file in no language ada lexes.
    code: Option<ParseRules>,
    state: State,
    spans: Vec<Span>,
    runs: Vec<Run>,
    preamble: Preamble,
    /// The hash of the line before, if it was blame's.
    last: String,
    /// The number the next line has when it follows the one before in the file.
    next: Option<u32>,
}

impl Blame {
    pub(super) fn new(lang: Option<Lang>, theme: &SyntaxTheme, spans: Vec<Span>) -> Self {
        Blame {
            code: lang.map(|l| ParseRules::cached(l, theme.id)),
            state: State::default(),
            spans,
            runs: Vec::new(),
            preamble: Preamble::new(theme),
            last: String::new(),
            next: None,
        }
    }

    /// Hand the rules resolved to the blocks after this one, and the spans' vector back.
    pub(super) fn done(mut self) -> Vec<Span> {
        if let Some(rules) = &mut self.code {
            rules.keep();
        }
        self.spans
    }

    /// Highlight up to `max` of `text`'s lines into `out`, after the lines before them: where
    /// the lines left start, `None` once `text` is done (as [`ada_parse::lex_lines`]). When
    /// `expand`, the lines are expanded into `buf` first ([`super::lex_block`]).
    pub(super) fn lines(
        &mut self,
        text: &str,
        max: usize,
        (buf, expand): (&mut String, bool),
        theme: &SyntaxTheme,
        out: &mut LineRuns,
    ) -> Option<usize> {
        if !expand {
            return self.split(text, None, max, theme, out);
        }
        if max == 0 {
            return Some(0);
        }
        let next = super::expand_block(text, max, buf);
        let done = self.split(buf, Some(text), usize::MAX, theme, out);
        debug_assert!(done.is_none(), "every expanded line highlighted");
        next
    }

    /// [`Blame::lines`] on lines as they are, or expanded from `raw`'s, which tell
    /// `git annotate`'s tabs.
    fn split(
        &mut self,
        text: &str,
        raw: Option<&str>,
        max: usize,
        theme: &SyntaxTheme,
        out: &mut LineRuns,
    ) -> Option<usize> {
        let mut raw = raw.map(|r| Lines(Some(r)));
        let mut at = 0;
        for _ in 0..max {
            let rest = &text[at..];
            let (line, next) = match memchr::memchr(b'\n', rest.as_bytes()) {
                Some(i) => (&rest[..i], Some(at + i + 1)),
                None => (rest, None),
            };
            let line = line.strip_suffix('\r').unwrap_or(line);
            let tabbed = match &mut raw {
                Some(raw) => raw.next().is_some_and(is_tabbed),
                None => false,
            };
            self.line(line, tabbed, theme, out);
            at = next?;
        }
        Some(at)
    }

    /// Highlight `line` into `out`.
    fn line(&mut self, line: &str, tabbed: bool, theme: &SyntaxTheme, out: &mut LineRuns) {
        let Some(f) = fields(line, tabbed) else {
            self.last.clear();
            self.next = None;
            return out.open_line().end();
        };
        let p = &self.preamble;
        let mut open = Runs::new(out.open_line());
        let hash = &line[f.hash.0..f.hash.1];
        let quiet = hash == self.last || hash.bytes().all(|b| b == b'0');
        open.to(f.hash.0 as u32, p.dim);
        open.to(f.hash.1 as u32, if quiet { p.dim } else { p.hash });
        open.to(f.code as u32, p.dim);
        self.last.clear();
        self.last.push_str(hash);
        // a line that doesn't follow the one before: another `-L` range, from a fresh state
        let number = f.line_number(line);
        if number.is_none() || number != self.next {
            self.state = State::default();
        }
        self.next = number.and_then(|n| n.checked_add(1));
        if let Some(rules) = &mut self.code {
            let lang = rules.lang;
            self.runs.clear();
            ada_parse::lex_runs(
                lang,
                &line[f.code..],
                &mut self.state,
                &mut self.spans,
                &mut Resolving { rules, theme },
                &mut self.runs,
            );
            for r in &self.runs {
                open.to(r.end + f.code as u32, r.rule);
            }
        }
        open.end();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every shape git writes, from `git blame` and `git annotate` on this repository.
    #[test]
    fn fields_of_every_format() {
        fn code(line: &str, tabbed: bool) -> Option<&str> {
            fields(line, tabbed).map(|f| &line[f.code..])
        }
        fn path(line: &str) -> Option<&str> {
            fields(line, false).and_then(|f| f.path(line))
        }
        let plain = "7401e68 (Adrian Papari 2026-11-13 16:17:54 +0100 3) //! order";
        assert_eq!(code(plain, false), Some("//! order"));
        assert_eq!(path(plain), None);
        let f = fields(plain, false).unwrap();
        assert_eq!((f.hash, f.paren), ((0, 7), Some(8)));
        assert_eq!(f.line_number(plain), Some(3));
        // `-s`, a short number and a padded one, and an empty line's text
        assert_eq!(code("7401e68 3) x", false), Some("x"));
        assert_eq!(code("34ff1b5  12)  indented", false), Some(" indented"));
        assert_eq!(code("34ff1b5 2) ", false), Some(""));
        assert_eq!(code("34ff1b5 2)", false), Some(""));
        // `-f`, `-n`, both with `-s`
        let f = "7401e68 src/sniff.rs (Adrian Papari 2026-11-13 16:17:54 +0100 3) x";
        assert_eq!((code(f, false), path(f)), (Some("x"), Some("src/sniff.rs")));
        let n = "7401e68 3 (Adrian Papari 2026-11-13 16:17:54 +0100 3) x";
        assert_eq!((code(n, false), path(n)), (Some("x"), None));
        let fns = "7401e68 src/sniff.rs 3 3) x";
        assert_eq!(
            (code(fns, false), path(fns)),
            (Some("x"), Some("src/sniff.rs"))
        );
        let padded = "7401e68 a.rs      12 (A 2026-11-13 16:17:54 +0100 3) x";
        assert_eq!(path(padded), Some("a.rs"));
        // `-e`, `-t`, `-l`, `--abbrev=4`, a boundary commit, an uncommitted line
        for line in [
            "7401e68 (<a@b.c> 2026-11-13 16:17:54 +0100 4) x",
            "7401e68 (Adrian Papari 1794583074 +0100 3) x",
            "7401e68db1fba3d47651d70d9debd66efe124f05 (A 2026-11-13 16:17:54 +0100 3) x",
            "7401 (A 2026-11-13 16:17:54 +0100 3) x",
            "^34ff1b5 (A 2026-10-15 19:25:17 +0200 1) x",
            "0000000 (External file (--contents) 2026-11-20 23:00:17 +0100 1) x",
        ] {
            assert_eq!(code(line, false), Some("x"), "{line}");
        }
        assert_eq!(fields("^34ff1b5 (A 1) x", false).unwrap().hash, (1, 8));
        // `git annotate`, `blame -c`: its text right after the `)`, a space of its own kept
        let annotate = "7401e68\t(Adrian Papari\t2026-11-13 16:17:54 +0100\t3) x";
        assert!(is_tabbed(annotate));
        assert!(!is_tabbed(plain));
        assert_eq!(code(annotate, true), Some(" x"));
        // no blame's
        for line in [
            "",
            "fn main() {",
            "abc (x 1) y",
            "7401e68(A 1) x",
            "7401e68 (A x) y)",
        ] {
            assert_eq!(fields(line, false), None, "{line:?}");
        }
    }

    /// `runs`' rules past byte `at`, their ends from it: a line's text's, past its fields.
    fn past(runs: &[Run], at: u32) -> Vec<Run> {
        let mut out: Vec<Run> = Vec::new();
        for r in runs.iter().filter(|r| r.end > at) {
            let end = r.end - at;
            match out.last_mut() {
                Some(last) if last.rule == r.rule => last.end = end,
                _ => out.push(Run { end, rule: r.rule }),
            }
        }
        out
    }

    fn theme() -> SyntaxTheme {
        SyntaxTheme::new(&crate::theme::Theme::builtin().syntax)
    }

    /// The fields quiet, a commit's first line's hash in color; the text lexed as the file's
    /// lines are, from a fresh state where the numbers jump; a line of no blame's plain.
    #[test]
    fn fields_quiet_and_text_in_its_language() {
        let st = theme();
        let p = Preamble::new(&st);
        let text = "7401e68 (A 2026-11-13 16:17:54 +0100 1) /* open\n\
                    7401e68 (A 2026-11-13 16:17:54 +0100 2) still */ fn x() {}\n\
                    abcdef1 (B 2026-11-13 16:17:54 +0100 3) let s = \"a\";\n\
                    abcdef1 (B 2026-11-13 16:17:54 +0100 9) still */ fn x() {}\n\
                    not blame\n\
                    0000000 (Not Committed Yet 2026-11-20 23:00:17 +0100 10) // new\n";
        let spans = super::super::highlight(1, name(Some(Lang::Rust)), text, &st);
        let rust = super::super::highlight(1, "Rust", "/* open\nstill */ fn x() {}\n", &st);
        let fresh = super::super::highlight(1, "Rust", "still */ fn x() {}\nlet s = \"a\";\n", &st);
        let at = 40; // past `7401e68 (A 2026-11-13 16:17:54 +0100 1) `
        assert_eq!(past(spans.line(0), at), rust.line(0));
        assert_eq!(past(spans.line(1), at), rust.line(1));
        assert_eq!(past(spans.line(2), at), fresh.line(1));
        assert_eq!(
            past(spans.line(3), at),
            fresh.line(0),
            "a jump starts fresh"
        );
        assert!(spans.line(4).is_empty(), "no blame's line is plain");
        // the hash in color on a commit's first line, dimmed on the lines after it
        let rule = |n: u32, at: u32| spans.line(n).iter().find(|r| r.end > at).unwrap().rule;
        assert_ne!(p.hash, p.dim);
        assert_eq!(
            (rule(0, 0), rule(1, 0), rule(2, 0)),
            (p.hash, p.dim, p.hash)
        );
        assert_eq!(rule(5, 0), p.dim, "an uncommitted line's");
        // the author, the date and the number dimmed
        assert_eq!(
            (rule(0, 9), rule(0, 25), rule(0, 39)),
            (p.dim, p.dim, p.dim)
        );
        // a comment's rule may be the fields' (one run): `let`'s isn't
        assert_eq!(spans.line(2)[1].end, at);
    }

    /// `git annotate`'s fields are split by tabs: its text starts right after the `)`, where
    /// the expanded line has it.
    #[test]
    fn annotates_text_after_its_tabs() {
        let st = theme();
        let text = "7401e68\t(A\t2026-11-13 16:17:54 +0100\t1)fn x() {}\n";
        let spans = super::super::highlight(1, name(Some(Lang::Rust)), text, &st);
        let rust = super::super::highlight(1, "Rust", "fn x() {}", &st);
        let expanded = crate::layout::expand_tabs(text.lines().next().unwrap());
        let at = expanded.find(')').unwrap() as u32 + 1;
        assert_eq!(past(spans.line(0), at), rust.line(0));
    }

    /// Blame printed a piece at a time reads as the whole does, cut at each line in turn: the
    /// text's state and the commit before carried.
    #[test]
    fn streamed_blame_reads_as_the_whole() {
        let st = theme();
        let lang = Some(Lang::Python);
        let text = "7401e68 (A 2026-11-13 16:17:54 +0100 1) s = \"\"\"open\n\
                    7401e68 (A 2026-11-13 16:17:54 +0100 2) still\"\"\"\n\
                    abcdef1 (B 2026-11-13 16:17:54 +0100 3) x = 1\n\
                    abcdef1 (B 2026-11-13 16:17:54 +0100 3)\tx = 1\n";
        let whole = super::super::highlight(1, name(lang), text, &st);
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        for cut in 1..lines.len() {
            let mut carrier = super::super::Carrier::new(super::super::Carry::Blame(lang), &st);
            let mut n = 0;
            for (a, b) in [(0, cut), (cut, lines.len())] {
                let spans = carrier.highlight(1, &lines[a..b].concat(), &[]);
                for i in 0..(b - a) as u32 {
                    assert_eq!(spans.line(i), whole.line(n), "cut {cut}, line {n}");
                    n += 1;
                }
            }
        }
    }

    #[test]
    fn names_lead_back_to_their_language() {
        assert_eq!(of_name(NAME), Some(None));
        assert_eq!(name(None), NAME);
        for &lang in Lang::ALL {
            assert_eq!(of_name(name(Some(lang))), Some(Some(lang)), "{lang:?}");
        }
        assert_eq!(of_name("Rust"), None);
        assert_eq!(of_name("Git Blamed"), None);
        assert_eq!(of_name("Git Blame (Nope)"), None);
    }
}
