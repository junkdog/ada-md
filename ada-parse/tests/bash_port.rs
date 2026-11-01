//! bash's scanner against the port of syntect's machine it replaces (`make`, G-003), whose
//! scope stacks are syntect's: per byte, the scope a theme's rule follows (the deepest that
//! isn't `meta.*`, and whether an option's `variable.parameter` outranks it). A quick check
//! between `parity` runs, without syntect. `$BASH_LIST` names a file of paths, one a line.
//!
//! ```zsh
//! BASH_LIST=$PWD/bench/b2/root/build/syntect-probe/lists/bash.txt \
//!   cargo test -p ada-parse --release --test bash_port -- --ignored --nocapture
//! ```

use ada_parse::make::{self, Grammar};
use ada_parse::{Lang, State, lex};

/// What a theme's rule follows in a stack of scopes, outermost first.
fn key(stack: &[&str]) -> String {
    let Some(i) = stack.iter().rposition(|s| !s.starts_with("meta.")) else {
        return String::new();
    };
    let deepest = stack[i];
    let outranked = deepest.starts_with("variable.")
        && !deepest.starts_with("variable.parameter")
        && stack[..i]
            .iter()
            .any(|s| s.starts_with("variable.parameter"));
    if outranked {
        format!("{deepest} (in an option)")
    } else {
        deepest.to_string()
    }
}

#[test]
#[ignore]
fn bash_against_the_port() {
    let list = std::env::var("BASH_LIST").expect("BASH_LIST");
    let show: usize = std::env::var("SHOW")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let paths: Vec<String> = std::fs::read_to_string(list)
        .expect("list")
        .lines()
        .map(str::to_string)
        .collect();
    let (mut bytes, mut differ, mut files_differ, mut shown) = (0usize, 0usize, 0usize, 0usize);
    let (mut spans, mut port_spans) = (Vec::new(), Vec::new());
    for path in &paths {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let mut state = State::default();
        let mut port = make::State::new(Grammar::Bash);
        let mut file_differs = false;
        for (n, line) in text.lines().enumerate() {
            if line.len() > 4096 {
                break;
            }
            lex(Lang::Bash, line, &mut state, &mut spans);
            make::tokens(line, &mut port, &mut port_spans);
            let mut ours = vec![String::new(); line.len()];
            for s in &spans {
                let k = key(&Lang::Bash.scopes(s.kind));
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
