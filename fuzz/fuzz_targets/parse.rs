//! ADR-40: ada-parse's lexers against what they replaced, syntect on bat's grammars, as the
//! `make` target checks Makefiles (`lexer_parity`). The first byte picks the language, of the
//! ones whose lexers give bat's scopes exactly: Diff, INI, TOML, XML, Dockerfile, Java
//! properties, `.env`, and Git's ignore, attributes, mailmap and link files. Each kind's scopes must be
//! syntect's stack on every byte of every line, as long as the lexer's state holds its whole
//! stack and each stack of scopes has its own kind (nested deeper than either, the lexer keeps
//! the top and the innermost scopes). XML's scanner keeps each meta scope once where bat's nest
//! repeats them (a group in a group, a conditional section in one), so its stacks are compared
//! with each scope once. YAML's scanner keeps only the innermost flow collection's
//! meta scope, by design (ADR-40, speed first): it is held by `tokens` and by `examples/parity.rs`.
//!
//! Lines are cut to 256 bytes, which keeps Oniguruma's backtracking short.
//!
//! `ADA_FUZZ_LANGS` limits it to some of these (`xml`, `toml,ini`; `langs.rs`): the first byte
//! still picks from all of them, so an input means the same language with the filter or
//! without, and the others' inputs return at once.
//!
//! ```zsh
//! cargo +nightly fuzz run -s none parse -- -dict=fuzz/parse.dict -max_len=4096
//! ```

#![no_main]

mod langs;
mod lexer_parity;

use ada_parse::{Lang, State};
use libfuzzer_sys::fuzz_target;

/// The longest line compared.
const LINE: usize = 256;

/// The languages whose kinds carry bat's scopes exactly.
const EXACT: [Lang; 11] = [
    Lang::Diff,
    Lang::Ini,
    Lang::Toml,
    Lang::Xml,
    Lang::Dockerfile,
    Lang::Properties,
    Lang::DotEnv,
    Lang::GitIgnore,
    Lang::GitAttributes,
    Lang::GitMailmap,
    Lang::GitLink,
];

fuzz_target!(|data: &[u8]| {
    let Some((&pick, data)) = data.split_first() else {
        return;
    };
    let lang = EXACT[pick as usize % EXACT.len()];
    if !langs::selected(&EXACT).contains(&lang) {
        return;
    }
    let (mut state, mut spans) = (State::default(), Vec::new());
    // XML's scanner keeps a nested group's or conditional section's meta scopes once
    let once = lang == Lang::Xml;
    lexer_parity::compare(lang.name(), data, LINE, once, |line| {
        ada_parse::lex(lang, line, &mut state, &mut spans);
        if !state.is_exact() {
            return None;
        }
        let mut out = vec![String::new(); line.len()];
        for s in &spans {
            for b in &mut out[s.start as usize..s.end as usize] {
                *b = lang.scopes(s.kind).join(" ");
            }
        }
        Some(out)
    });
});
