//! ada-parse's lexers alone, in MB of source a second, on the census corpora (ADR-40): each
//! file's lines through `lex` from the default state, the corpus repeated to at least 4 MB a
//! round, best of 5 rounds. `$ADA_PARSE_LISTS` is the directory of lists (`<lang>.txt`, one
//! path a line); `$ONLY` picks one language, for a profile. Run pinned, in release:
//!
//! ```zsh
//! ADA_PARSE_LISTS=bench/b2/root/build/syntect-probe/lists taskset -c 2 \
//!   cargo test -p ada-parse --release --test lexer_speed -- --ignored --nocapture
//! ```

use std::time::{Duration, Instant};

use ada_parse::{Lang, State, lex};

const LANGS: &[(&str, Lang)] = &[
    ("json", Lang::Json),
    ("diff", Lang::Diff),
    ("ini", Lang::Ini),
    ("toml", Lang::Toml),
    ("xml", Lang::Xml),
    ("yaml", Lang::Yaml),
    ("dockerfile", Lang::Dockerfile),
    ("properties", Lang::Properties),
    ("dotenv", Lang::DotEnv),
    ("gitignore", Lang::GitIgnore),
    ("gitattributes", Lang::GitAttributes),
    ("mailmap", Lang::GitMailmap),
    ("rust", Lang::Rust),
    ("kotlin", Lang::Kotlin),
    ("java", Lang::Java),
    ("cpp", Lang::Cpp),
    ("c", Lang::C),
    ("csharp", Lang::CSharp),
    ("go", Lang::Go),
    ("python", Lang::Python),
    ("lua", Lang::Lua),
    ("sql", Lang::Sql),
    ("powershell", Lang::PowerShell),
    ("javascript", Lang::JavaScript),
    ("typescript", Lang::TypeScript),
    ("tsx", Lang::TypeScriptReact),
    ("css", Lang::Css),
    ("bash", Lang::Bash),
    // `make.txt` phase 0's Makefiles
    ("make", Lang::Makefile),
];

#[test]
#[ignore]
fn lexer_speed() {
    let dir = std::env::var("ADA_PARSE_LISTS").expect("ADA_PARSE_LISTS");
    let only = std::env::var("ONLY").ok();
    let mut spans = Vec::new();
    for &(name, lang) in LANGS {
        if only.as_deref().is_some_and(|o| o != name) {
            continue;
        }
        let Ok(list) = std::fs::read_to_string(format!("{dir}/{name}.txt")) else {
            continue;
        };
        let files: Vec<String> = list
            .lines()
            .filter_map(|p| std::fs::read_to_string(p).ok())
            .collect();
        let bytes: usize = files.iter().map(String::len).sum();
        let lines: usize = files.iter().map(|f| f.lines().count()).sum();
        if bytes == 0 {
            continue;
        }
        let repeat = (4_000_000 / bytes).max(1);
        let mut best = Duration::MAX;
        for _ in 0..5 {
            let t = Instant::now();
            for _ in 0..repeat {
                for text in &files {
                    let mut state = State::default();
                    for line in text.lines() {
                        lex(lang, line, &mut state, &mut spans);
                    }
                }
            }
            best = best.min(t.elapsed());
        }
        let secs = best.as_secs_f64() / repeat as f64;
        println!(
            "{name:14} {:7.1} MB/s  {:6.0} ns/line  {bytes:>9} bytes {lines:>7} lines {:>4} files",
            bytes as f64 / secs / 1e6,
            secs * 1e9 / lines as f64,
            files.len()
        );
    }
}
