//! The scanners that took the place of bat's grammars, ada-parse's lexers (ADR-40), Makefile's
//! and bash's (G-002, G-003), log lines' (O-43) and JSON lines' (O-48) among them, each given every line of the input with the state one line leaves the next.
//! Their spans go to `highlight`'s runs, which slice the line at them: they must come in order,
//! not overlap, not be empty, and start and end on characters of the line.
//! Nothing may panic, and a long line must not take long. An input that starts with a NUL is a
//! size and a piece: the piece is repeated to 1–64 KiB, since a scan that reads a run again from
//! each word only shows on long lines, which libFuzzer grows to slowly (O-50).
//! A language whose scan writes a highlighter's runs itself (`lex_runs`, X-068: Rust) is lexed
//! that way too, on a state of its own: its runs must be those its spans give, by a theme's rules
//! resolved as they're met, and its state the same.
//! One language an input (by a hash of it) is lexed again as blocks (`lex_lines`, X-067: a carried
//! scanner's one scan across a block's lines), each of a few lines (1 to 8 by the input's length,
//! or all of them): each line's spans, and the state each block leaves, must be those the lines
//! give one at a time.
//! `ADA_FUZZ_LANGS` limits it to some languages (`python`, `makefile,bash`; `langs.rs`), as
//! `fuzz/lang.zsh` runs a smoke for one lexer on that language's own corpus.
//!
//! ```zsh
//! cargo +nightly fuzz run -s none -a tokens -- -dict=fuzz/tokens.dict -max_len=65536
//! ```

#![no_main]

mod langs;

use ada_parse::{Lang, Rule, Rules, Run, Span};
use libfuzzer_sys::fuzz_target;

/// The languages whose scan writes runs itself, not through spans (Rust X-068, the line-format
/// engine's X-069, JSON's, TOML's and XML's X-072, the code languages' X-074; Go's, Lua's,
/// SCSS's and bash's X-075, bash's run copy apart from its span copy since X-094; Makefile's
/// runs come from its spans since X-094; C's and C++'s X-057; Swift's X-101; CMake's X-104).
const RUN_SCANS: [Lang; 39] = [
    Lang::Bash,
    Lang::Scss,
    Lang::Rust,
    Lang::Go,
    Lang::Lua,
    Lang::Swift,
    Lang::CMake,
    Lang::Java,
    Lang::CSharp,
    Lang::Kotlin,
    Lang::TypeScript,
    Lang::TypeScriptReact,
    Lang::Python,
    Lang::Lisp,
    Lang::EmacsLisp,
    Lang::Scheme,
    Lang::Racket,
    Lang::Clojure,
    Lang::Dockerfile,
    Lang::Ini,
    Lang::Properties,
    Lang::DotEnv,
    Lang::GitIgnore,
    Lang::GitAttributes,
    Lang::GitMailmap,
    Lang::GitLink,
    Lang::Fstab,
    Lang::CMakeCache,
    Lang::KnownHosts,
    Lang::SshConfig,
    Lang::SshdConfig,
    Lang::Crontab,
    Lang::Nginx,
    Lang::Php,
    Lang::Json,
    Lang::Toml,
    Lang::Xml,
    Lang::Cpp,
    Lang::C,
];

/// A theme of a few rules over the kinds, plain for some, each resolved as it's met into a table
/// grown as ada's is.
struct Lazy(Vec<Rule>);

impl Lazy {
    fn rule(kind: u16) -> Rule {
        match kind % 5 {
            0 => Rule(Rule::NONE),
            r => Rule(r % 3),
        }
    }
}

impl Rules for Lazy {
    fn table(&self) -> &[Rule] {
        &self.0
    }

    fn resolve(&mut self, kind: u16) -> Rule {
        let k = usize::from(kind);
        if k >= self.0.len() {
            self.0
                .resize((2 * self.0.len()).max(k + 1), Rule(Rule::UNSET));
        }
        self.0[k] = Lazy::rule(kind);
        self.0[k]
    }
}

/// A line's runs from its spans, as a highlighter builds them: a gap plain, a run joined to the
/// one before of its rule, none where no span has a rule, else ending at the line's end.
fn runs_of(len: usize, spans: &[Span]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let put = |runs: &mut Vec<Run>, end: u32, rule: Rule| match runs.last_mut() {
        Some(r) if r.rule == rule => r.end = end,
        _ => runs.push(Run { end, rule }),
    };
    let mut at = 0;
    for s in spans {
        if s.start > at {
            put(&mut runs, s.start, Rule(Rule::NONE));
        }
        put(&mut runs, s.end, Lazy::rule(s.kind));
        at = s.end;
    }
    if runs.iter().all(|r| r.rule.0 == Rule::NONE) {
        return Vec::new();
    }
    if (at as usize) < len {
        put(&mut runs, len as u32, Rule(Rule::NONE));
    }
    runs
}

/// Spans `(start, end)` of `line`, in order, as `highlight` takes them.
fn check(what: &str, line: &str, spans: impl Iterator<Item = (u32, u32)>) {
    let mut at = 0;
    for (start, end) in spans {
        let (s, e) = (start as usize, end as usize);
        assert!(
            at <= s && s < e && e <= line.len(),
            "{what}: span {s}..{e} after {at}, in {} bytes: {line:?}",
            line.len()
        );
        assert!(
            line.is_char_boundary(s) && line.is_char_boundary(e),
            "{what}: span {s}..{e} splits a character: {line:?}"
        );
        at = e;
    }
}

fuzz_target!(|data: &[u8]| {
    let repeated;
    let data = match data {
        [0, size, piece @ ..] if !piece.is_empty() => {
            let len = 1 << (10 + size % 7);
            repeated = piece.repeat(len / piece.len() + 1);
            &repeated[..]
        }
        _ => data,
    };
    let text = String::from_utf8_lossy(data);
    let trace = std::env::var_os("ADA_FUZZ_TRACE").is_some();
    let on = langs::selected(&Lang::ALL);
    let lexed = on.iter().copied().filter(|l| l.lexed());
    let mut parsed: Vec<_> = lexed.map(|l| (l, ada_parse::State::default())).collect();
    let mut ran: Vec<_> = on
        .iter()
        .copied()
        .filter(|l| RUN_SCANS.contains(l))
        .map(|l| (l, ada_parse::State::default(), Lazy(Vec::new())))
        .collect();
    let (mut ps, mut scratch, mut runs) = (Vec::new(), Vec::new(), Vec::new());
    for line in text.split('\n') {
        for (lang, state, rules) in &mut ran {
            let mut spans_state = *state;
            ada_parse::lex(*lang, line, &mut spans_state, &mut ps);
            runs.clear();
            ada_parse::lex_runs(*lang, line, state, &mut scratch, rules, &mut runs);
            assert_eq!(runs, runs_of(line.len(), &ps), "{lang:?} runs: {line:?}");
            assert_eq!(*state, spans_state, "{lang:?} state after runs: {line:?}");
        }
        for (lang, state) in &mut parsed {
            ada_parse::lex(*lang, line, state, &mut ps);
            check(lang.name(), line, ps.iter().map(|s| (s.start, s.end)));
            assert!(
                ps.iter().all(|s| !lang.scopes(s.kind).is_empty()),
                "{lang:?}: {ps:?}"
            );
        }
        if trace {
            eprintln!("{line:?}");
        }
    }
    // one language an input, so the check costs a fortieth of the lexing above
    let lexed: Vec<_> = on.iter().copied().filter(|l| l.lexed()).collect();
    if !lexed.is_empty() {
        let max = match text.len() % 9 {
            0 => usize::MAX,
            n => n,
        };
        let h = data
            .iter()
            .fold(0usize, |h, &c| h.wrapping_mul(31) ^ usize::from(c));
        blocks(lexed[h % lexed.len()], &text, max);
    }
});

/// `text` lexed in `lang` as blocks of `max` lines, against its lines lexed one at a time (a
/// `\r` before a `\n` dropped, as `lex_lines` drops it).
fn blocks(lang: Lang, text: &str, max: usize) {
    let (mut want, mut state, mut spans) = (Vec::new(), ada_parse::State::default(), Vec::new());
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        ada_parse::lex(lang, line, &mut state, &mut spans);
        want.push((spans.clone(), state));
    }
    let (mut got, mut state, mut buf) = (Vec::new(), ada_parse::State::default(), String::new());
    let mut rest = Some(text);
    while let Some(t) = rest {
        let each = |_: &str, s: &[Span]| got.push(s.to_vec());
        let next = ada_parse::lex_lines(
            lang,
            t,
            max,
            &mut state,
            (&mut spans, &mut buf),
            |_, _| false,
            each,
        );
        let n = got.len();
        assert_eq!(
            state,
            want[n - 1].1,
            "{lang:?} block of {max}: state after line {n}"
        );
        rest = next.map(|p| &t[p..]);
    }
    assert_eq!(got.len(), want.len(), "{lang:?} block of {max}: lines");
    for (i, (g, w)) in got.iter().zip(&want).enumerate() {
        assert_eq!(*g, w.0, "{lang:?} block of {max}: line {}", i + 1);
    }
}
