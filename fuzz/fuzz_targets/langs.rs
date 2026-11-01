//! The languages a target fuzzes: `ADA_FUZZ_LANGS`, a comma-separated list of [`Lang`]s as
//! `Debug` spells them, in lowercase (`python`, `cpp,c`, `makefile,bash`), so that a smoke for
//! one lexer spends every run on it (`fuzz/lang.zsh`). Unset or empty, all of them.

use ada_parse::Lang;
use std::sync::OnceLock;

/// The name `ADA_FUZZ_LANGS` gives `lang`.
pub fn name(lang: Lang) -> String {
    format!("{lang:?}").to_ascii_lowercase()
}

/// The languages of `set` that `ADA_FUZZ_LANGS` names, read once: a binary is one target, which
/// passes its own set, so the first call's set is the only one. Exits on a name outside `set`;
/// `list` prints the set's names, a line each, and exits.
pub fn selected(set: &[Lang]) -> &'static [Lang] {
    static ON: OnceLock<Vec<Lang>> = OnceLock::new();
    ON.get_or_init(|| {
        let var = std::env::var("ADA_FUZZ_LANGS").unwrap_or_default();
        if var == "list" {
            for &l in set {
                println!("{}", name(l));
            }
            std::process::exit(0);
        }
        let names: Vec<&str> = var
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if names.is_empty() {
            return set.to_vec();
        }
        names
            .into_iter()
            .map(|n| {
                set.iter()
                    .copied()
                    .find(|&l| name(l) == n)
                    .unwrap_or_else(|| {
                        let known: Vec<String> = set.iter().map(|&l| name(l)).collect();
                        eprintln!("ADA_FUZZ_LANGS: no {n:?} here; one of {}", known.join(", "));
                        std::process::exit(2);
                    })
            })
            .collect()
    })
}
