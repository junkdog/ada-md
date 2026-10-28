//! G-002 and G-003: ada's Makefile and shell lexer (`src/make`) against syntect on bat's
//! grammars, byte for byte; `--bash` takes the files as shell scripts.
//!
//! Each line is expanded as the highlighter expands it, then parsed by both, carrying their
//! state from line to line; every byte whose scope stack differs is counted, and the first
//! differences are shown with both stacks. With `--time`, both are timed instead, over every
//! file, best of five.
//!
//! ```zsh
//! cargo run --release --example make_parity -- Makefile src/*.mk
//! cargo run --release --example make_parity -- --show 40 Makefile
//! cargo run --release --example make_parity -- --time $(cat corpus.txt)
//! cargo run --release --example make_parity -- --bash $(cat scripts.txt)
//! ```

use std::time::{Duration, Instant};
use std::{env, fs};

use ada::highlight::syntaxes;
use ada::layout::expand_tabs;
use ada::make::{self, Grammar, Span, State};
use syntect::parsing::{ParseState, ScopeStack};

mod common;

use common::{cpu, ms};

fn main() {
    let mut show = 10usize;
    let mut time = false;
    let mut dump = false;
    let mut grammar = Grammar::Make;
    let mut files = Vec::new();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--show" => show = args.next().and_then(|n| n.parse().ok()).unwrap_or(10),
            "--time" => time = true,
            "--dump" => dump = true,
            "--bash" => grammar = Grammar::Bash,
            _ => files.push(arg),
        }
    }
    let texts: Vec<(String, String)> = files
        .into_iter()
        .filter_map(|f| fs::read_to_string(&f).ok().map(|t| (f, t)))
        .collect();
    if time {
        return timing(grammar, &texts);
    }
    if dump {
        for (_, text) in &texts {
            dump_file(grammar, text);
        }
        return;
    }
    let (mut bytes, mut differ, mut files_differ) = (0usize, 0usize, 0usize);
    for (name, text) in &texts {
        let (b, d, shown) = compare(grammar, name, text, show);
        bytes += b;
        differ += d;
        files_differ += usize::from(d > 0);
        show -= shown;
    }
    println!(
        "{} files, {bytes} bytes: {differ} differ ({:.3}%), in {files_differ} files",
        texts.len(),
        differ as f64 * 100.0 / bytes.max(1) as f64
    );
}

fn lines(text: &str) -> impl Iterator<Item = String> + '_ {
    let text = text.strip_suffix('\n').unwrap_or(text);
    text.split('\n')
        .map(|l| expand_tabs(l.strip_suffix('\r').unwrap_or(l)).into_owned())
}

/// The stack under the base scope of each byte of `line`, from syntect.
fn syntect_line(ps: &mut ParseState, stack: &mut ScopeStack, line: &str) -> Vec<String> {
    let buf = format!("{line}\n");
    let ops = ps.parse_line(&buf, syntaxes()).expect("parse");
    let mut out = Vec::with_capacity(line.len());
    let fill = |stack: &ScopeStack, end: usize, out: &mut Vec<String>| {
        let names: Vec<String> = stack
            .as_slice()
            .iter()
            .skip(1)
            .map(|s| s.build_string())
            .collect();
        let s = names.join(" ");
        while out.len() < end.min(line.len()) {
            out.push(s.clone());
        }
    };
    for (i, op) in &ops {
        fill(stack, *i, &mut out);
        stack.apply(op).expect("apply");
    }
    fill(stack, line.len(), &mut out);
    out
}

fn ours_line(state: &mut State, spans: &mut Vec<Span>, line: &str) -> Vec<String> {
    make::tokens(line, state, spans);
    let mut out = vec![String::new(); line.len()];
    for s in spans.iter() {
        let names: Vec<&str> = state.stack(s.stack).iter().map(|sc| sc.name()).collect();
        let joined = names.join(" ");
        for b in &mut out[s.start as usize..s.end as usize] {
            b.clone_from(&joined);
        }
    }
    out
}

/// Bytes, differing bytes, and how many differences were shown.
fn compare(grammar: Grammar, name: &str, text: &str, show: usize) -> (usize, usize, usize) {
    let syntax = syntaxes()
        .find_syntax_by_name(grammar.syntax())
        .expect("syntax");
    let mut ps = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut state = State::new(grammar);
    let mut spans = Vec::new();
    let (mut bytes, mut differ, mut shown) = (0, 0, 0);
    for (n, line) in lines(text).enumerate() {
        let want = syntect_line(&mut ps, &mut stack, &line);
        let got = ours_line(&mut state, &mut spans, &line);
        bytes += line.len();
        let mut i = 0;
        while i < line.len() {
            if want[i] == got[i] {
                i += 1;
                continue;
            }
            let start = i;
            while i < line.len()
                && want[i] != got[i]
                && want[i] == want[start]
                && got[i] == got[start]
            {
                i += 1;
            }
            differ += i - start;
            if shown < show {
                shown += 1;
                println!(
                    "{name}:{}:{start}-{i} {:?}\n  in line {:?}\n  syntect: {}\n  ada:     {}",
                    n + 1,
                    &line[start..i],
                    line,
                    want[start],
                    got[start]
                );
            }
        }
    }
    (bytes, differ, shown)
}

fn timing(grammar: Grammar, texts: &[(String, String)]) {
    let all: Vec<Vec<String>> = texts.iter().map(|(_, t)| lines(t).collect()).collect();
    let n_bytes: usize = all.iter().flatten().map(|l| l.len() + 1).sum();
    let syntax = syntaxes()
        .find_syntax_by_name(grammar.syntax())
        .expect("syntax");
    // the fastest of five rounds by wall time, with its CPU time; the first round compiles
    // syntect's regexes
    let best = |f: &mut dyn FnMut()| {
        (0..5)
            .map(|_| {
                let (t, c) = (Instant::now(), cpu());
                f();
                (t.elapsed(), cpu() - c)
            })
            .min()
            .unwrap_or_default()
    };
    let mut buf = String::new();
    let syn = best(&mut || {
        for file in &all {
            let mut ps = ParseState::new(syntax);
            for line in file {
                buf.clear();
                buf.push_str(line);
                buf.push('\n');
                std::hint::black_box(ps.parse_line(&buf, syntaxes()).ok());
            }
        }
    });
    let mut spans = Vec::new();
    let ours = best(&mut || {
        for file in &all {
            let mut state = State::new(grammar);
            for line in file {
                make::tokens(line, &mut state, &mut spans);
                std::hint::black_box(&spans);
            }
        }
    });
    let mbs = |d: Duration| n_bytes as f64 / d.as_secs_f64() / 1e6;
    println!(
        "{} files, {n_bytes} bytes, best of 5: syntect {:.2} ms (CPU {:.2}, {:.1} MB/s), ada {:.2} ms (CPU {:.2}, {:.1} MB/s), {:.1}x",
        texts.len(),
        ms(syn.0),
        ms(syn.1),
        mbs(syn.0),
        ms(ours.0),
        ms(ours.1),
        mbs(ours.0),
        syn.0.as_secs_f64() / ours.0.as_secs_f64()
    );
}

/// Every line's stacks from both, a run of equal stacks a row.
fn dump_file(grammar: Grammar, text: &str) {
    let syntax = syntaxes()
        .find_syntax_by_name(grammar.syntax())
        .expect("syntax");
    let mut ps = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut state = State::new(grammar);
    let mut spans = Vec::new();
    for (n, line) in lines(text).enumerate() {
        let want = syntect_line(&mut ps, &mut stack, &line);
        let got = ours_line(&mut state, &mut spans, &line);
        println!("{}: {line:?}", n + 1);
        let mut i = 0;
        while i < line.len() {
            let s = i;
            while i < line.len() && want[i] == want[s] && got[i] == got[s] {
                i += 1;
            }
            let mark = if want[s] == got[s] { ' ' } else { '!' };
            println!(
                "  {mark} {s}-{i} {:?}\n      syntect: {}\n      ada:     {}",
                &line[s..i],
                want[s],
                got[s]
            );
        }
    }
}
