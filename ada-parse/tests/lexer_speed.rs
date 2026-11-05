//! ada-parse's lexers alone, in MB of source a second, on the census corpora (ADR-40): each
//! file's lines through `lex` from the default state, the corpus repeated to at least 4 MB a
//! round, best of 5 rounds. The lines are split before the timing, so only the lexer is timed:
//! `str::lines` inside the loop was 11–19% of a lexer's time and doubled its branch misses where
//! lines are short (X-051); `SPLIT=in` times the old loop, the split in it. `$ADA_PARSE_LISTS` is
//! the directory of lists (`<lang>.txt`, one path a line); `$ONLY` picks one language, for a
//! profile. Run pinned, in release:
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
    // token-cost's `log.txt`, log-corpus/'s snapshot
    ("log", Lang::Log),
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
    ("fstab", Lang::Fstab),
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
    ("scss", Lang::Scss),
    ("html", Lang::Html),
    ("lisp", Lang::Lisp),
    ("elisp", Lang::EmacsLisp),
    ("scheme", Lang::Scheme),
    ("racket", Lang::Racket),
    ("clojure", Lang::Clojure),
    ("bash", Lang::Bash),
    // `make.txt` phase 0's Makefiles
    ("make", Lang::Makefile),
];

#[test]
#[ignore]
fn lexer_speed() {
    let dir = std::env::var("ADA_PARSE_LISTS").expect("ADA_PARSE_LISTS");
    let only = std::env::var("ONLY").ok();
    let split_in = std::env::var("SPLIT").is_ok_and(|s| s == "in");
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
        // each file's lines, split as `str::lines` splits them (a `\r` before a `\n` dropped)
        let split: Vec<Vec<&str>> = files.iter().map(|f| f.lines().collect()).collect();
        let lines: usize = split.iter().map(Vec::len).sum();
        if bytes == 0 {
            continue;
        }
        let repeat = (4_000_000 / bytes).max(1);
        let mut best = Duration::MAX;
        for _ in 0..5 {
            let t = Instant::now();
            if split_in {
                for _ in 0..repeat {
                    for text in &files {
                        let mut state = State::default();
                        for line in text.lines() {
                            lex(lang, line, &mut state, &mut spans);
                        }
                    }
                }
            } else {
                for _ in 0..repeat {
                    for file in &split {
                        let mut state = State::default();
                        for &line in file {
                            lex(lang, line, &mut state, &mut spans);
                        }
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
