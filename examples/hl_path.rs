//! HL-004: what ada's per-line path costs around an ada-parse lexer. Times, on one corpus and
//! in one binary, the lexer alone (each line through `ada_parse::lex`, as `lexer_speed` does:
//! the lines split before the timing, X-051) and `highlight::highlight` (the whole path: line
//! splitting, `expand_tabs`, the rules, the runs), warm: the corpus repeated to at least 4 MB a
//! round, best of `ROUNDS` (5). Run pinned:
//!
//! ```zsh
//! taskset -c 2 target/release/examples/hl_path yaml @lists/yaml.txt
//! MODE=ada taskset -c 2 target/release/examples/hl_path yaml @lists/yaml.txt  # one mode, for perf
//! ```
//!
//! A run takes ~10–20 ms: hold the core's clock up (`b2-bench-profile on 2 $$`), or the first
//! runs of a sweep read up to ~30% slow before the governor ramps (X-048).
//!
//! `MODE` is `lex`, `ada` or both (the default), `lex-in`: the lexer with `str::lines` in the
//! timed loop, as `lexer_speed`'s `SPLIT=in`, `digest`: a hash of every line's runs in
//! every built-in theme, for holding a speed step to the same output, or `count`: the corpus's
//! lines, spans and runs, and its files with a tab or a CR. `SECS` keeps the one mode
//! running about that long, for a profile. `BLOCK=N` cuts each file into blocks of N lines, each
//! highlighted (and lexed) from the default state, as a document's code blocks are.

use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};
use std::{env, fs, hint};

use ada::highlight::{self, Syntax, SyntaxTheme};
use ada::theme::Theme;

fn main() {
    let mut args = env::args().skip(1);
    let lang = args.next().expect("usage: hl_path LANG FILE... | @LIST");
    let mut files = Vec::new();
    for a in args {
        match a.strip_prefix('@') {
            Some(list) => files.extend(
                fs::read_to_string(list)
                    .expect("list")
                    .lines()
                    .map(str::to_owned),
            ),
            None => files.push(a),
        }
    }
    let mode = env::var("MODE").unwrap_or_default();
    // a syntect language only for the digest
    let own = match highlight::find_syntax(&lang) {
        Some(Syntax::Own(own)) => Some(own),
        _ if mode == "digest" => None,
        _ => panic!("{lang}: not an ada-parse language"),
    };
    let texts: Vec<String> = files
        .iter()
        .filter_map(|p| fs::read_to_string(p).ok())
        .collect();
    // BLOCK=N: each file as blocks of N lines, as a document's code blocks come (X-022)
    let texts = match env::var("BLOCK").ok().and_then(|b| b.parse::<usize>().ok()) {
        Some(n) if n > 0 => texts
            .iter()
            .flat_map(|t| {
                let lines: Vec<&str> = t.split_inclusive('\n').collect();
                lines.chunks(n).map(<[&str]>::concat).collect::<Vec<_>>()
            })
            .collect(),
        _ => texts,
    };
    let bytes: usize = texts.iter().map(String::len).sum();
    let themes = Theme::builtins();
    let rule = SyntaxTheme::new(&themes[0].syntax);
    let rounds: usize = env::var("ROUNDS")
        .ok()
        .and_then(|r| r.parse().ok())
        .unwrap_or(5);
    let secs: f64 = env::var("SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    let repeat = (4_000_000 / bytes.max(1)).max(1);

    // lexer_speed's loop as written there: one spans vector for every round (a fresh one each
    // round cost fstab's 3.9 KB corpus ~6%, X-048), the lines split by `str::lines` before the
    // timing (X-051), the state anew a file; `lex-in` splits inside it, as before X-051
    let split: Vec<Vec<&str>> = texts.iter().map(|t| t.lines().collect()).collect();
    let kept = std::cell::RefCell::new(Vec::new());
    let lex = || {
        let lang = own.expect("own");
        let mut spans = kept.borrow_mut();
        for file in &split {
            let mut state = ada_parse::State::default();
            for &line in file {
                ada_parse::lex(lang, line, &mut state, &mut spans);
            }
        }
        hint::black_box(&*spans);
    };
    let lex_in = || {
        let lang = own.expect("own");
        let mut spans = kept.borrow_mut();
        for text in &texts {
            let mut state = ada_parse::State::default();
            for line in text.lines() {
                ada_parse::lex(lang, line, &mut state, &mut spans);
            }
        }
        hint::black_box(&*spans);
    };
    let ada = || {
        for text in &texts {
            hint::black_box(highlight::highlight(1, &lang, text, &rule));
        }
    };
    let time = |f: &dyn Fn()| {
        let mut best = Duration::MAX;
        let start = Instant::now();
        let mut n = 0;
        while n < rounds || start.elapsed().as_secs_f64() < secs {
            let t = Instant::now();
            for _ in 0..repeat {
                f();
            }
            best = best.min(t.elapsed());
            n += 1;
        }
        bytes as f64 / (best.as_secs_f64() / repeat as f64) / 1e6
    };
    if mode == "digest" {
        // every line's runs, every file, every built-in theme: a step must leave it unchanged
        let mut h = std::hash::DefaultHasher::new();
        for theme in &themes {
            let rule = SyntaxTheme::new(&theme.syntax);
            for text in &texts {
                let spans = highlight::highlight(1, &lang, text, &rule);
                for n in 0..spans.line_count() {
                    for run in spans.line(n) {
                        (run.end, run.rule.0).hash(&mut h);
                    }
                    u32::MAX.hash(&mut h);
                }
            }
        }
        println!("{lang} {:016x}", h.finish());
        return;
    }
    if mode == "count" {
        // the corpus's shape: files, lines, the lexer's spans and ada's runs, in the first theme
        let (mut lines, mut spans, mut runs) = (0usize, 0usize, 0usize);
        let mut v = Vec::new();
        for text in &texts {
            let mut state = ada_parse::State::default();
            let body = text.strip_suffix('\n').unwrap_or(text);
            for line in body.split('\n') {
                let line = line.strip_suffix('\r').unwrap_or(line);
                ada_parse::lex(own.expect("own"), line, &mut state, &mut v);
                spans += v.len();
                lines += 1;
            }
            let out = highlight::highlight(1, &lang, text, &rule);
            runs += (0..out.line_count())
                .map(|n| out.line(n).len())
                .sum::<usize>();
        }
        let tabbed = texts.iter().filter(|t| t.contains('\t')).count();
        let crlf = texts.iter().filter(|t| t.contains('\r')).count();
        println!(
            "{lang}: {} files, {bytes} bytes, {lines} lines, {spans} spans, {runs} runs; \
             {tabbed} files with a tab, {crlf} with a CR",
            texts.len()
        );
        return;
    }
    println!("{lang}: {} files, {bytes} bytes", texts.len());
    let (mut l, mut a) = (None, None);
    if mode.is_empty() || mode == "lex" {
        l = Some(time(&lex));
        println!("lex  {:7.1} MB/s", l.unwrap());
    }
    if mode == "lex-in" {
        println!("lex-in {:5.1} MB/s", time(&lex_in));
    }
    if mode.is_empty() || mode == "ada" {
        a = Some(time(&ada));
        println!("ada  {:7.1} MB/s", a.unwrap());
    }
    if let (Some(l), Some(a)) = (l, a) {
        println!("path {:5.1}% of ada's time", 100.0 * (1.0 - a / l));
    }
}
