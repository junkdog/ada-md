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
//! ```zsh
//! cargo run --release --example parity -- json $(cat lists/json.txt)
//! cargo run --release --example parity -- bash --show 40 @lists/bash.txt
//! ```
//! `LANG` is a fence token ada's own lexer takes (`json`, `log`, `make`, `bash`); `@FILE` names a
//! file of paths, one a line.

use std::collections::HashMap;
use std::{env, fs};

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

fn main() {
    let mut args = env::args().skip(1);
    let lang = args
        .next()
        .expect("usage: parity LANG [--show N] FILE... | @LIST");
    let mut show = 20;
    let mut files = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--show" => show = args.next().and_then(|n| n.parse().ok()).unwrap_or(show),
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

    let (mut read, mut bytes, mut failed, mut long) = (0, 0usize, Vec::new(), 0usize);
    let mut agree = vec![0usize; themes.len()];
    let mut diffs: HashMap<String, Diff> = HashMap::new();
    for path in &files {
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        if text.split('\n').any(|l| expand_tabs(l).len() > MAX_LINE) {
            long += 1;
            continue;
        }
        let reference = highlight::reference_syntaxes();
        let mut theirs = Vec::with_capacity(rules.len());
        for r in &rules {
            match highlight::highlight_syntect(reference, name, &text, r) {
                Some(spans) => theirs.push(spans),
                None => break,
            }
        }
        if theirs.len() < rules.len() {
            failed.push(path.as_str());
            continue;
        }
        read += 1;
        let lines: Vec<String> = text
            .strip_suffix('\n')
            .unwrap_or(&text)
            .split('\n')
            .map(|l| expand_tabs(l.strip_suffix('\r').unwrap_or(l)).into_owned())
            .collect();
        for (t, r) in rules.iter().enumerate() {
            let ours = highlight::highlight(1, &lang, &text, r);
            for (n, line) in lines.iter().enumerate() {
                if t == 0 {
                    bytes += line.len();
                }
                let a = per_byte(&ours, n, line.len());
                let b = per_byte(&theirs[t], n, line.len());
                let mut i = 0;
                while i < line.len() {
                    let pair = (a[i], b[i]);
                    let end = (i..line.len())
                        .find(|&j| (a[j], b[j]) != pair)
                        .unwrap_or(line.len());
                    if pair.0 == pair.1 {
                        agree[t] += end - i;
                    } else {
                        let token: String = line[i..end].chars().take(32).collect();
                        let d = diffs.entry(token).or_default();
                        if d.themes.is_empty() {
                            d.first = format!("{path}:{}", n + 1);
                            let rule = |r: Rule| rule_name(&themes[t], r);
                            d.rules = (rule(pair.0), rule(pair.1));
                        }
                        if !d.themes.contains(&t) {
                            d.themes.push(t);
                        }
                        d.bytes += end - i;
                    }
                    i = end;
                }
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
