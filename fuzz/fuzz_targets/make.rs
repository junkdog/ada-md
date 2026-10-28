//! G-002: the Makefile lexer (`ada::make`) against what it replaced, syntect on bat's grammar.
//! Every line, expanded as the highlighter expands it, must get the same scope stack on every
//! byte from both, with each one's state carried from line to line.
//!
//! The input is made ASCII (each byte's high bit dropped): Oniguruma's `\s`, `\w` and
//! `[[:alpha:]]` take Unicode classes where the lexer reads ASCII whitespace, a known
//! difference. Lines are cut to 96 bytes, as the grammar backtracks ~n³ on unbalanced parens
//! before a `:` (seconds a line from ~300 bytes), and past ~400 Oniguruma's retry limit gives
//! up and changes its answer.
//!
//! ```zsh
//! cargo +nightly fuzz run -s none make -- -dict=fuzz/make.dict -max_len=4096
//! ```

#![no_main]

use ada::highlight::syntaxes;
use ada::layout::expand_tabs;
use ada::make::{self, State};
use libfuzzer_sys::fuzz_target;
use syntect::parsing::{ParseState, ScopeStack};

/// The longest line compared.
const LINE: usize = 96;

/// Each byte's stack under `source.makefile`, from syntect.
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

fuzz_target!(|data: &[u8]| {
    let ascii: Vec<u8> = data.iter().map(|b| b & 0x7f).collect();
    let text = String::from_utf8(ascii).expect("ascii");
    let trace = std::env::var_os("ADA_FUZZ_TRACE").is_some();
    let syntax = syntaxes()
        .find_syntax_by_name("Makefile")
        .expect("Makefile");
    let mut ps = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut state = State::new();
    let mut spans = Vec::new();
    for (n, line) in text.split('\n').enumerate() {
        let line = expand_tabs(&line[..line.len().min(LINE)]).into_owned();
        let line = &line[..line.len().min(LINE)];
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
});
