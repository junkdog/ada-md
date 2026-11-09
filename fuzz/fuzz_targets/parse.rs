//! ADR-40: ada-parse's lexers against what they replaced, syntect on bat's grammars, as the
//! `make` and `bash` targets check Makefile's and bash's scanners (`lexer_parity`). The first byte picks the language, of the
//! ones whose lexers give bat's scopes exactly: Diff, XML, Dockerfile, Java
//! properties, `.env`, Git's ignore, attributes, mailmap and link files, fstab and `known_hosts`. Each kind's scopes must be
//! syntect's stack on every byte of every line, as long as the lexer's state holds its whole
//! stack and each stack of scopes has its own kind (nested deeper than either, the lexer keeps
//! the top and the innermost scopes). XML's scanner keeps each meta scope once where bat's nest
//! repeats them (a group in a group, a conditional section in one), so its stacks are compared
//! with each scope once. YAML's scanner keeps only the innermost flow collection's
//! meta scope, by design (ADR-40, speed first), and TOML's reads TOML 1.1 where bat's grammar
//! doesn't (X-020), and INI's takes a comment after a value where INI's readers do (a `;` or `#`
//! after a blank) where bat's ends the value at its first `;`: they are held by `tokens` and by
//! `examples/parity.rs`.
//!
//! Lines are cut to 256 bytes, which keeps Oniguruma's backtracking short.
//!
//! `ADA_FUZZ_LANGS` limits it to some of these (`xml`, `dotenv,fstab`; `langs.rs`): the first byte
//! still picks from all of them, so an input means the same language with the filter or
//! without, and the others' inputs return at once.
//!
//! ```zsh
//! cargo +nightly fuzz run -s none parse -- -dict=fuzz/parse.dict -max_len=4096
//! ```

#![no_main]

mod langs;
mod lexer_parity;

use ada_parse::Lang;
use lexer_parity::Stacks;
use libfuzzer_sys::fuzz_target;

/// The longest line compared.
const LINE: usize = 256;

/// The languages whose kinds carry bat's scopes exactly.
const EXACT: [Lang; 12] = [
    Lang::Diff,
    Lang::Xml,
    Lang::Dockerfile,
    Lang::Properties,
    Lang::DotEnv,
    Lang::GitIgnore,
    Lang::GitAttributes,
    Lang::GitMailmap,
    Lang::GitLink,
    Lang::Fstab,
    Lang::CMakeCache,
    Lang::KnownHosts,
];

fuzz_target!(|data: &[u8]| {
    let Some((&pick, data)) = data.split_first() else {
        return;
    };
    let lang = EXACT[pick as usize % EXACT.len()];
    if !langs::selected(&EXACT).contains(&lang) {
        return;
    }
    // XML's scanner keeps a nested group's or conditional section's meta scopes once
    let stacks = if lang == Lang::Xml {
        Stacks::Once
    } else {
        Stacks::Whole
    };
    lexer_parity::check(lang, data, LINE, stacks);
});
