//! What the `make` and `bash` targets share: a text's lines through ada's lexer (`ada::make`)
//! and through syntect on bat's grammar, each with its state carried from line to line, and
//! the first byte whose scope stack differs.
//!
//! The input is made ASCII (each byte's high bit dropped): Oniguruma's `\s`, `\w` and
//! `[[:alpha:]]` take Unicode classes where the lexer reads ASCII whitespace, a known
//! difference.

use ada::highlight::syntaxes;
use ada::layout::expand_tabs;
use ada::make::{self, Grammar, State};
use syntect::parsing::{ParseState, ScopeStack};

/// Each byte's stack under the base scope, from syntect.
fn syntect_line(ps: &mut ParseState, stack: &mut ScopeStack, line: &str) -> Vec<String> {
    let buf = format!("{line}\n");
    let ops = ps.parse_line(&buf, syntaxes()).expect("syntect parses");
    let mut out = Vec::with_capacity(line.len());
    let fill = |stack: &ScopeStack, end: usize, out: &mut Vec<String>| {
        let names: Vec<String> = stack
            .as_slice()
            .iter()
            .skip(1)
            .map(|s| s.build_string())
            .collect();
        let joined = names.join(" ");
        while out.len() < end.min(line.len()) {
            out.push(joined.clone());
        }
    };
    for (i, op) in &ops {
        fill(stack, *i, &mut out);
        stack.apply(op).expect("apply");
    }
    fill(stack, line.len(), &mut out);
    out
}

fn ours_line(state: &mut State, spans: &mut Vec<make::Span>, line: &str) -> Vec<String> {
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

/// Panics at the first byte of `data`'s lines, each cut to `max` bytes, whose stack differs.
pub fn check(grammar: Grammar, data: &[u8], max: usize) {
    let ascii: Vec<u8> = data.iter().map(|b| b & 0x7f).collect();
    let text = String::from_utf8(ascii).expect("ascii");
    let trace = std::env::var_os("ADA_FUZZ_TRACE").is_some();
    let syntax = syntaxes()
        .find_syntax_by_name(grammar.syntax())
        .expect("syntax");
    let mut ps = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut state = State::new(grammar);
    let mut spans = Vec::new();
    for (n, line) in text.split('\n').enumerate() {
        let line = expand_tabs(&line[..line.len().min(max)]).into_owned();
        let line = &line[..line.len().min(max)];
        let want = syntect_line(&mut ps, &mut stack, line);
        let got = ours_line(&mut state, &mut spans, line);
        if trace {
            eprintln!("{}: {line:?}", n + 1);
        }
        if let Some(i) = (0..line.len()).find(|&i| want[i] != got[i]) {
            panic!(
                "line {} byte {i} of {line:?}:\n  syntect: {}\n  ada:     {}",
                n + 1,
                want[i],
                got[i]
            );
        }
    }
}
