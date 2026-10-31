//! ada-parse's comparison harness (ADR-40): one of ada's own lexers against the bat grammar it
//! stands in for, on a corpus, in every built-in theme. syntect is a guideline, not a contract:
//! a difference fails nothing. It is reviewed, kept where ada's coloring is as good or better,
//! and fixed where it is a lexing mistake; the report keeps a lexer from drifting unseen.
//!
//! For each theme, the share of bytes whose rule (the theme's `[syntax]` entry that colors it)
//! is syntect's; then the differences grouped by token: the stretch of text, its bytes over all
//! themes, how many themes differ on it, where it is first, and both rules in one of them.
//! Files with a line over 4096 bytes are left out: ada leaves such a line plain on syntect
//! (`MAX_LINE`) without parsing it, so syntect's state after it is wrong, and its own lexers
//! color it.
//!
//! The files are compared on `-j N` threads (all cores by default), each file on one, and
//! merged in the list's order, so the report is the same on any number. `--speed` compares
//! nothing and gives both highlighters' speed alone, on one thread (run it pinned): the corpus
//! in the first theme, in MB of source a second
//! of wall time (one call a file, syntect's on a set already loaded, so its first use of a
//! grammar counts once; run it pinned to a core).
//!
//! ```zsh
//! cargo run --release --example parity -- json $(cat lists/json.txt)
//! cargo run --release --example parity -- bash --show 40 @lists/bash.txt
//! taskset -c 2 target/release/examples/parity yaml --speed @lists/yaml.txt
//! ```
//! `LANG` is a fence token one of ada's own lexers takes; `@FILE` names a file of paths, one a
//! line.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use std::{env, fs, thread};

use ada::highlight::{self, CodeSpans, Rule, Syntax, SyntaxTheme};
use ada::layout::expand_tabs;
use ada::theme::Theme;

/// syntect's longest highlighted line (`highlight::MAX_LINE`).
const MAX_LINE: usize = 4096;

/// One token's differences.
#[derive(Default)]
struct Diff {
    bytes: usize,
    themes: Vec<usize>,
    first: String,
    /// In the first theme it differs in: ada's rule, syntect's.
    rules: (String, String),
}

/// What one file gave in one theme.
#[derive(PartialEq)]
enum Outcome {
    Compared,
    Unreadable,
    /// A line over [`MAX_LINE`].
    Long,
    SyntectFailed,
}

/// One file's comparison in one theme.
struct Part {
    outcome: Outcome,
    bytes: usize,
    agree: usize,
    diffs: HashMap<String, Diff>,
    ada_time: Duration,
    syntect_time: Duration,
}

fn main() {
    let mut args = env::args().skip(1);
    let lang = args
        .next()
        .expect("usage: parity LANG [--show N] [-j THREADS | --speed] FILE... | @LIST");
    let mut show = 20;
    let mut threads = thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let mut files = Vec::new();
    let mut speed_only = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--speed" => speed_only = true,
            "--show" => show = args.next().and_then(|n| n.parse().ok()).unwrap_or(show),
            "-j" => threads = args.next().and_then(|n| n.parse().ok()).unwrap_or(threads),
            list if list.starts_with('@') => {
                let list = fs::read_to_string(&list[1..]).expect("a list of paths");
                files.extend(list.lines().map(str::to_owned));
            }
            _ => files.push(arg),
        }
    }
    let Some(Syntax::Own(own)) = highlight::find_syntax(&lang) else {
        panic!("{lang}: no lexer of ada's own takes it");
    };
    let name = own.name();
    let themes = Theme::builtins();
    let rules: Vec<SyntaxTheme> = themes.iter().map(|t| SyntaxTheme::new(&t.syntax)).collect();
    // loaded once, before the threads race to
    highlight::reference_syntaxes();
    if speed_only {
        return speed(&files, &lang, name, &rules[0]);
    }

    // a task is a file in one theme (one big file in 14 themes spreads over 14 threads),
    // taken as they come and kept by its place: file by file, each theme in order
    let n = themes.len();
    let tasks = files.len() * n;
    let next = AtomicUsize::new(0);
    let mut parts: Vec<Option<Part>> = (0..tasks).map(|_| None).collect();
    thread::scope(|s| {
        let workers: Vec<_> = (0..threads.clamp(1, tasks.max(1)))
            .map(|_| {
                s.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let k = next.fetch_add(1, Ordering::Relaxed);
                        if k >= tasks {
                            return done;
                        }
                        let (f, t) = (k / n, k % n);
                        done.push((k, compare(&files[f], &lang, name, t, &themes[t], &rules[t])));
                    }
                })
            })
            .collect();
        for w in workers {
            for (k, part) in w.join().expect("a parity thread") {
                parts[k] = Some(part);
            }
        }
    });
    let parts: Vec<Part> = parts.into_iter().flatten().collect();

    let (mut read, mut bytes, mut failed, mut long) = (0, 0usize, Vec::new(), 0usize);
    let mut agree = vec![0usize; n];
    let mut diffs: HashMap<String, Diff> = HashMap::new();
    let (mut ada_time, mut syntect_time) = (Duration::ZERO, Duration::ZERO);
    for (path, file) in files.iter().zip(parts.chunks(n)) {
        match file[0].outcome {
            Outcome::Unreadable => continue,
            Outcome::Long => {
                long += 1;
                continue;
            }
            _ => {}
        }
        // syntect's speed is its first theme's, a file it fails on included
        syntect_time += file[0].syntect_time;
        if file.iter().any(|p| p.outcome == Outcome::SyntectFailed) {
            failed.push(path.as_str());
            continue;
        }
        read += 1;
        bytes += file[0].bytes;
        ada_time += file[0].ada_time;
        // in the list's order, theme by theme, so a token's first place and rules are where
        // one thread finds them
        for (t, part) in file.iter().enumerate() {
            agree[t] += part.agree;
            for (token, d) in &part.diffs {
                let all = diffs.entry(token.clone()).or_default();
                if all.themes.is_empty() {
                    all.first.clone_from(&d.first);
                    all.rules.clone_from(&d.rules);
                }
                if !all.themes.contains(&t) {
                    all.themes.push(t);
                }
                all.bytes += d.bytes;
            }
        }
    }

    println!(
        "{lang} ({name}): {read} files, {bytes} bytes; {} left out where syntect failed, \
         {long} with a line over {MAX_LINE} bytes",
        failed.len()
    );
    let pct = |n: usize, of: usize| 100.0 * n as f64 / of.max(1) as f64;
    for (t, theme) in themes.iter().enumerate() {
        println!("  {:<24} {:>8.3}%", theme.name, pct(agree[t], bytes));
    }
    let all: usize = agree.iter().sum();
    println!(
        "  {:<24} {:>8.3}%",
        "all themes",
        pct(all, bytes * themes.len())
    );
    let mut diffs: Vec<(String, Diff)> = diffs.into_iter().collect();
    diffs.sort_by(|a, b| b.1.bytes.cmp(&a.1.bytes).then_with(|| a.0.cmp(&b.0)));
    println!(
        "{} tokens differ; the first {}, by bytes over all themes:",
        diffs.len(),
        show.min(diffs.len())
    );
    for (token, d) in diffs.iter().take(show) {
        let theme = &themes[d.themes[0]].name;
        println!(
            "  {token:?}  {} B, {} themes, {}  ({theme}: ada {}, syntect {})",
            d.bytes,
            d.themes.len(),
            d.first,
            d.rules.0,
            d.rules.1,
        );
    }
    for path in failed.iter().take(5) {
        println!("  syntect failed: {path}");
    }
    let rate = |t: Duration| bytes as f64 / 1e6 / t.as_secs_f64().max(1e-9);
    match threads {
        1 => println!(
            "speed: ada {:.1} MB/s, syntect {:.2} MB/s",
            rate(ada_time),
            rate(syntect_time)
        ),
        _ => println!("speed: on {threads} threads; --speed, pinned to a core, measures it"),
    }
}

/// Both highlighters' speed on `files` in one theme, on this thread, as `-j 1` times them: the
/// files syntect fails on count for syntect alone, those with a line past `MAX_LINE` for neither.
fn speed(files: &[String], lang: &str, name: &str, rule: &SyntaxTheme) {
    let (mut bytes, mut ada, mut syntect) = (0usize, Duration::ZERO, Duration::ZERO);
    let mut timed = 0;
    for path in files {
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let lines = || text.strip_suffix('\n').unwrap_or(&text).split('\n');
        if lines().any(|l| expand_tabs(l).len() > MAX_LINE) {
            continue;
        }
        let start = Instant::now();
        let theirs =
            highlight::highlight_syntect(highlight::reference_syntaxes(), name, &text, rule);
        syntect += start.elapsed();
        if theirs.is_none() {
            continue;
        }
        let start = Instant::now();
        let ours = highlight::highlight(1, lang, &text, rule);
        ada += start.elapsed();
        std::hint::black_box((ours, theirs));
        timed += 1;
        bytes += lines()
            .map(|l| expand_tabs(l.strip_suffix('\r').unwrap_or(l)).len())
            .sum::<usize>();
    }
    let rate = |t: Duration| bytes as f64 / 1e6 / t.as_secs_f64().max(1e-9);
    println!(
        "{lang} ({name}): {timed} files, {bytes} bytes\nspeed: ada {:.1} MB/s, syntect {:.2} MB/s",
        rate(ada),
        rate(syntect)
    );
}

/// One file of ada's highlighting against syntect's, in theme `t`.
fn compare(
    path: &str,
    lang: &str,
    name: &str,
    t: usize,
    theme: &Theme,
    rule: &SyntaxTheme,
) -> Part {
    let mut r = Part {
        outcome: Outcome::Compared,
        bytes: 0,
        agree: 0,
        diffs: HashMap::new(),
        ada_time: Duration::ZERO,
        syntect_time: Duration::ZERO,
    };
    let Ok(text) = fs::read_to_string(path) else {
        r.outcome = Outcome::Unreadable;
        return r;
    };
    if text.split('\n').any(|l| expand_tabs(l).len() > MAX_LINE) {
        r.outcome = Outcome::Long;
        return r;
    }
    let start = Instant::now();
    let theirs = highlight::highlight_syntect(highlight::reference_syntaxes(), name, &text, rule);
    // speed is the first theme's
    if t == 0 {
        r.syntect_time = start.elapsed();
    }
    let Some(theirs) = theirs else {
        r.outcome = Outcome::SyntectFailed;
        return r;
    };
    let start = Instant::now();
    let ours = highlight::highlight(1, lang, &text, rule);
    if t == 0 {
        r.ada_time = start.elapsed();
    }
    let lines = text
        .strip_suffix('\n')
        .unwrap_or(&text)
        .split('\n')
        .map(|l| expand_tabs(l.strip_suffix('\r').unwrap_or(l)));
    for (n, line) in lines.enumerate() {
        r.bytes += line.len();
        let a = per_byte(&ours, n, line.len());
        let b = per_byte(&theirs, n, line.len());
        let mut i = 0;
        while i < line.len() {
            let pair = (a[i], b[i]);
            let end = (i..line.len())
                .find(|&j| (a[j], b[j]) != pair)
                .unwrap_or(line.len());
            if pair.0 == pair.1 {
                r.agree += end - i;
            } else {
                let token: String = line[i..end].chars().take(32).collect();
                let d = r.diffs.entry(token).or_default();
                if d.first.is_empty() {
                    d.first = format!("{path}:{}", n + 1);
                    let name = |rule: Rule| rule_name(theme, rule);
                    d.rules = (name(pair.0), name(pair.1));
                }
                d.bytes += end - i;
            }
            i = end;
        }
    }
    r
}

/// Line `n`'s rule for each of its `len` bytes; [`Rule::NONE`] where no run covers one.
fn per_byte(spans: &CodeSpans, n: usize, len: usize) -> Vec<Rule> {
    let mut out = vec![Rule(Rule::NONE); len];
    let mut start = 0;
    for run in spans.line(n as u32) {
        let end = (run.end as usize).min(len);
        for r in &mut out[start.min(end)..end] {
            *r = run.rule;
        }
        start = end;
    }
    out
}

/// A rule as the theme names it: its scope selector, or `-` for none.
fn rule_name(theme: &Theme, rule: Rule) -> String {
    let named = theme.syntax.get(usize::from(rule.0));
    named.map_or_else(|| "-".to_owned(), |(scope, _)| scope.clone())
}
