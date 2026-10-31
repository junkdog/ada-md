//! ADR-40: ada-parse's lexers against what they replaced, syntect on bat's grammars, as the
//! `make` target checks Makefiles (`lexer_parity`). The first byte picks the language, of the
//! ones whose lexers give bat's scopes exactly: Diff and INI. Each kind's scopes must be
//! syntect's stack on every byte of every line.
//!
//! Lines are cut to 256 bytes, which keeps Oniguruma's backtracking short.
//!
//! ```zsh
//! cargo +nightly fuzz run -s none parse -- -dict=fuzz/parse.dict -max_len=4096
//! ```

#![no_main]

mod lexer_parity;

use ada_parse::{Lang, State};
use libfuzzer_sys::fuzz_target;

/// The longest line compared.
const LINE: usize = 256;

/// The languages whose kinds carry bat's scopes exactly.
const EXACT: [Lang; 2] = [Lang::Diff, Lang::Ini];

fuzz_target!(|data: &[u8]| {
    let Some((&pick, data)) = data.split_first() else {
        return;
    };
    let lang = EXACT[pick as usize % EXACT.len()];
    let (mut state, mut spans) = (State::default(), Vec::new());
    lexer_parity::compare(lang.name(), data, LINE, |line| {
        ada_parse::lex(lang, line, &mut state, &mut spans);
        let mut out = vec![String::new(); line.len()];
        for s in &spans {
            for b in &mut out[s.start as usize..s.end as usize] {
                *b = lang.kinds()[s.kind as usize].to_owned();
            }
        }
        out
    });
});
