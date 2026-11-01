//! Makefile's scanner against the port of syntect's machine it replaces (`make`, G-002), whose
//! scope stacks are syntect's. `$MAKE_LIST` names a file of paths, one a line.
//!
//! ```zsh
//! MAKE_LIST=$PWD/bench/b2/root/build/syntect-probe/lists/make.txt \
//!   cargo test -p ada-parse --release --test make_port -- --ignored --nocapture
//! ```

use std::collections::HashMap;

use ada_parse::make::{self, Grammar};

fn paths() -> Vec<String> {
    let list = std::env::var("MAKE_LIST").expect("MAKE_LIST");
    std::fs::read_to_string(list)
        .expect("list")
        .lines()
        .map(str::to_string)
        .collect()
}

/// The port's stacks over the corpus, their `meta.*` scopes left out, by bytes.
#[test]
#[ignore]
fn make_chains() {
    let mut chains: HashMap<String, (usize, String)> = HashMap::new();
    let mut spans = Vec::new();
    for path in paths() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut port = make::State::new(Grammar::Make);
        for line in text.lines() {
            if line.len() > 4096 {
                break;
            }
            make::tokens(line, &mut port, &mut spans);
            for s in &spans {
                let chain: Vec<&str> = port
                    .stack(s.stack)
                    .iter()
                    .map(|sc| sc.name())
                    .filter(|n| !n.starts_with("meta."))
                    .collect();
                let e = chains.entry(chain.join(" ")).or_default();
                e.0 += (s.end - s.start) as usize;
                if e.1.is_empty() {
                    e.1 = format!("{path}: {line}");
                }
            }
        }
    }
    let mut v: Vec<_> = chains.into_iter().collect();
    v.sort_by_key(|e| std::cmp::Reverse(e.1.0));
    for (chain, (bytes, first)) in &v {
        println!("{bytes:>10}  {chain}\n            {first:.160}");
    }
    println!("{} chains", v.len());
}

/// What a theme's rule follows in a stack of scopes, outermost first: the deepest scope that
/// isn't `meta.*` or `source.*`, and whether a `variable.parameter` scope around a
/// `variable.*` one outranks it.
fn key(stack: &[&str]) -> String {
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

#[test]
#[ignore]
fn make_against_the_port() {
    use ada_parse::{Lang, State, lex};
    let show: usize = std::env::var("SHOW")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let paths = paths();
    let (mut bytes, mut differ, mut files_differ, mut shown) = (0usize, 0usize, 0usize, 0usize);
    let (mut spans, mut port_spans) = (Vec::new(), Vec::new());
    for path in &paths {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let mut state = State::default();
        let mut port = make::State::new(Grammar::Make);
        let mut file_differs = false;
        for (n, line) in text.lines().enumerate() {
            if line.len() > 4096 {
                break;
            }
            lex(Lang::Makefile, line, &mut state, &mut spans);
            make::tokens(line, &mut port, &mut port_spans);
            let mut ours = vec![String::new(); line.len()];
            for s in &spans {
                let k = key(&Lang::Makefile.scopes(s.kind));
                for b in &mut ours[s.start as usize..s.end as usize] {
                    b.clone_from(&k);
                }
            }
            let mut theirs = vec![String::new(); line.len()];
            for s in &port_spans {
                let names: Vec<&str> = port.stack(s.stack).iter().map(|sc| sc.name()).collect();
                let k = key(&names);
                for b in &mut theirs[s.start as usize..s.end as usize] {
                    b.clone_from(&k);
                }
            }
            bytes += line.len();
            let mut first = None;
            for i in 0..line.len() {
                if ours[i] != theirs[i] {
                    differ += 1;
                    first.get_or_insert(i);
                }
            }
            if let Some(i) = first {
                file_differs = true;
                if shown < show {
                    shown += 1;
                    println!(
                        "{path}:{}:{i}: ours {:?}, port {:?}\n    {line}",
                        n + 1,
                        ours[i],
                        theirs[i]
                    );
                }
            }
        }
        if file_differs {
            files_differ += 1;
        }
    }
    println!(
        "{} files, {bytes} bytes: {differ} differ ({:.4}%), in {files_differ} files",
        paths.len(),
        100.0 * differ as f64 / bytes.max(1) as f64
    );
}
