//! What the `make`, `bash` and `parse` targets share: a text's lines through one of ada-parse's
//! lexers and through syntect on bat's grammar, each with its state carried from line to line,
//! and the first byte where they part. `parse` compares whole scope stacks, for the lexers whose
//! kinds carry bat's scopes exactly; `make` and `bash` compare what a theme's rule follows
//! ([`rule`]), as their scanners' kinds are leaves.
//!
//! The input is made ASCII (each byte's high bit dropped): Oniguruma's `\s`, `\w`, `\d` and
//! `[[:alpha:]]` take Unicode classes where the lexers read ASCII, a known difference.

// bat's whole set: ada's own has these grammars emptied (ADR-40)
use ada::highlight::reference_syntaxes as syntaxes;
use ada::layout::expand_tabs;
use ada_parse::{Lang, State};
use syntect::parsing::{ParseState, ScopeStack};

/// How a byte's scopes are compared.
#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Stacks {
    /// The whole stack under the base scope.
    Whole,
    /// The whole stack with each scope once, as a lexer that keeps a repeated meta scope once
    /// gives it (XML's).
    Once,
    /// The scope a theme's rule follows ([`rule`]).
    Rule,
}

/// What a theme's rule follows in a stack of scopes, outermost first: the deepest scope that
/// isn't `meta.*` or `source.*`, and whether a `variable.parameter` scope around a `variable.*`
/// one outranks it (an option's, a make substitution's).
pub fn rule(stack: &[&str]) -> String {
    let leaf = |s: &&str| !s.starts_with("meta.") && !s.starts_with("source.");
    let Some(i) = stack.iter().rposition(leaf) else {
        return String::new();
    };
    let deepest = stack[i];
    let outranked = deepest.starts_with("variable.")
        && !deepest.starts_with("variable.parameter")
        && stack[..i]
            .iter()
            .any(|s| s.starts_with("variable.parameter"));
    if outranked {
        format!("{deepest} (in a parameter)")
    } else {
        deepest.to_string()
    }
}

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

/// Panics at the first byte of `data`'s lines, each cut to `max` bytes, where `lang`'s lexer
/// and syntect on its grammar part, compared as `stacks` says, for as long as the lexer's
/// state holds its whole stack.
pub fn check(lang: Lang, data: &[u8], max: usize, stacks: Stacks) {
    let (mut state, mut spans) = (State::default(), Vec::new());
    compare(lang.name(), data, max, stacks, |line| {
        ada_parse::lex(lang, line, &mut state, &mut spans);
        if !state.is_exact() {
            return None;
        }
        let mut out = vec![String::new(); line.len()];
        for s in &spans {
            let scopes = lang.scopes(s.kind);
            let k = match stacks {
                Stacks::Rule => rule(&scopes),
                Stacks::Whole | Stacks::Once => scopes.join(" "),
            };
            for b in &mut out[s.start as usize..s.end as usize] {
                b.clone_from(&k);
            }
        }
        Some(out)
    });
}

/// Panics at the first byte of `data`'s lines, each cut to `max` bytes, whose scopes from
/// syntect's grammar `name`, compared as `stacks` says, differ from `ours`: each byte's for a
/// line, its state carried from the line before, or `None` when the lexer no longer claims
/// syntect's scopes (a stack deeper than it keeps).
fn compare(
    name: &str,
    data: &[u8],
    max: usize,
    stacks: Stacks,
    mut ours: impl FnMut(&str) -> Option<Vec<String>>,
) {
    let ascii: Vec<u8> = data.iter().map(|b| b & 0x7f).collect();
    let text = String::from_utf8(ascii).expect("ascii");
    let trace = std::env::var_os("ADA_FUZZ_TRACE").is_some();
    let syntax = syntaxes().find_syntax_by_name(name).expect("syntax");
    let mut ps = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    for (n, line) in text.split('\n').enumerate() {
        let line = expand_tabs(&line[..line.len().min(max)]).into_owned();
        let line = &line[..line.len().min(max)];
        let mut want = syntect_line(&mut ps, &mut stack, line);
        for w in &mut want {
            match stacks {
                Stacks::Whole => {}
                Stacks::Once => {
                    let mut seen: Vec<&str> = Vec::new();
                    for s in w.split(' ') {
                        if !seen.contains(&s) {
                            seen.push(s);
                        }
                    }
                    *w = seen.join(" ");
                }
                Stacks::Rule => *w = rule(&w.split(' ').collect::<Vec<_>>()),
            }
        }
        let Some(got) = ours(line) else {
            return;
        };
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
