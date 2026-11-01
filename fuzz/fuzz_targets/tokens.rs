//! The scanners that took the place of bat's grammars: Makefile and shell lines (G-002 and
//! G-003, `make`), and ada-parse's lexers (ADR-40), log lines (O-43) and JSON lines (O-48)
//! among them, each given every line of the input with the state one line leaves the next.
//! Their spans go to `highlight`'s runs, which slice the line at them: they must come in order,
//! not overlap, not be empty, and start and end on characters of the line.
//! Nothing may panic, and a long line must not take long. An input that starts with a NUL is a
//! size and a piece: the piece is repeated to 1–64 KiB, since a scan that reads a run again from
//! each word only shows on long lines, which libFuzzer grows to slowly (O-50).
//! `ADA_FUZZ_LANGS` limits it to some languages (`python`, `makefile,bash`; `langs.rs`), as
//! `fuzz/lang.zsh` runs a smoke for one lexer on that language's own corpus.
//!
//! ```zsh
//! cargo +nightly fuzz run -s none -a tokens -- -dict=fuzz/tokens.dict -max_len=65536
//! ```

#![no_main]

mod langs;

use ada::make;
use ada_parse::Lang;
use libfuzzer_sys::fuzz_target;

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
    let (on_make, on_bash) = (on.contains(&Lang::Makefile), on.contains(&Lang::Bash));
    let (mut mk, mut sh) = (Vec::new(), Vec::new());
    let mut state = make::State::new(make::Grammar::Make);
    let mut bash = make::State::new(make::Grammar::Bash);
    let lexed = on.iter().copied().filter(|l| l.lexed());
    let mut parsed: Vec<_> = lexed.map(|l| (l, ada_parse::State::default())).collect();
    let mut ps = Vec::new();
    for line in text.split('\n') {
        if on_make {
            make::tokens(line, &mut state, &mut mk);
            check("make", line, mk.iter().map(|s| (s.start, s.end)));
        }
        if on_bash {
            make::tokens(line, &mut bash, &mut sh);
            check("bash", line, sh.iter().map(|s| (s.start, s.end)));
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
            eprintln!("{line:?}\n  make {mk:?}\n  bash {sh:?}");
        }
    }
});
