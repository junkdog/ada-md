//! Spike B: highlighting cost.
//!
//! Measures how long two-face's `extra_newlines()` SyntaxSet takes to load, then highlights every
//! fenced code block found under the given paths with syntect + fancy-regex, grouped by language.
//! The theme is syntect's empty default: colour lookup is cheap next to parsing, which is what we
//! are measuring.
//!
//! ```zsh
//! cargo run --release --example spike_highlight -- ~/opt/birb-vault
//! cargo run --release --example spike_highlight -- ~/opt/birb-vault --lang typescript
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{env, fs, process};

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use syntect::easy::HighlightLines;
use syntect::highlighting::Theme;
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

fn main() {
    let mut roots = Vec::new();
    let mut only_lang: Option<String> = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--lang" => only_lang = Some(args.next().unwrap_or_else(|| usage())),
            "-h" | "--help" => usage(),
            _ => roots.push(PathBuf::from(arg)),
        }
    }

    let start = Instant::now();
    let syntaxes = two_face::syntax::extra_newlines();
    let load = start.elapsed();
    println!(
        "two-face extra_newlines(): {} syntaxes loaded in {:.1} ms",
        syntaxes.syntaxes().len(),
        ms(load)
    );

    // a second load is what a cold start pays once the dump is in the page cache
    let start = Instant::now();
    drop(std::hint::black_box(two_face::syntax::extra_newlines()));
    println!("second load: {:.1} ms", ms(start.elapsed()));

    if roots.is_empty() {
        return;
    }

    let mut files = Vec::new();
    for root in &roots {
        collect_markdown(root, &mut files);
    }
    let mut blocks = Vec::new();
    for path in &files {
        if let Ok(text) = fs::read_to_string(path) {
            extract_code_blocks(&text, &mut blocks);
        }
    }
    if let Some(lang) = &only_lang {
        blocks.retain(|b| b.lang.as_deref() == Some(lang.as_str()));
    }

    let theme = Theme::default();
    // pass 1 is cold: syntect compiles a syntax's regexes lazily, on first use
    let cold = highlight_all(&syntaxes, &theme, &blocks);
    let warm = highlight_all(&syntaxes, &theme, &blocks);

    println!("\n{} files, {} fenced blocks\n", files.len(), blocks.len());
    println!(
        "{:<36} {:>6} {:>6} {:>8} {:>9} {:>9} {:>9} {:>9}",
        "language → syntax",
        "blocks",
        "lines",
        "bytes",
        "cold ms",
        "cold max",
        "warm ms",
        "µs/line"
    );
    let (mut cold_total, mut warm_total) = (Duration::ZERO, Duration::ZERO);
    for (lang, c) in &cold.stats {
        let w = &warm.stats[lang];
        cold_total += c.total;
        warm_total += w.total;
        println!(
            "{:<36} {:>6} {:>6} {:>8} {:>9.2} {:>9.2} {:>9.2} {:>9.1}",
            lang,
            c.blocks,
            c.lines,
            c.bytes,
            ms(c.total),
            ms(c.max),
            ms(w.total),
            w.total.as_secs_f64() * 1e6 / w.lines.max(1) as f64,
        );
    }
    println!(
        "\nall resolved blocks: cold {:.1} ms, warm {:.1} ms",
        ms(cold_total),
        ms(warm_total)
    );
    if !cold.unresolved.is_empty() {
        println!("unresolved (rendered plain): {:?}", cold.unresolved);
    }
}

struct Pass {
    stats: BTreeMap<String, Stats>,
    unresolved: BTreeMap<String, usize>,
}

fn highlight_all(syntaxes: &SyntaxSet, theme: &Theme, blocks: &[CodeBlock]) -> Pass {
    let mut stats: BTreeMap<String, Stats> = BTreeMap::new();
    let mut unresolved: BTreeMap<String, usize> = BTreeMap::new();
    for block in blocks {
        let lang = block.lang.clone().unwrap_or_else(|| "(none)".to_string());
        let Some(syntax) = resolve(syntaxes, block.lang.as_deref()) else {
            *unresolved.entry(lang).or_default() += 1;
            continue;
        };

        let start = Instant::now();
        let mut highlighter = HighlightLines::new(syntax, theme);
        let mut spans = 0;
        for line in LinesWithEndings::from(&block.text) {
            spans += highlighter
                .highlight_line(line, syntaxes)
                .map_or(0, |s| s.len());
        }
        let elapsed = start.elapsed();
        std::hint::black_box(spans);

        let s = stats
            .entry(format!("{lang} → {}", syntax.name))
            .or_default();
        s.blocks += 1;
        s.lines += block.text.lines().count();
        s.bytes += block.text.len();
        s.total += elapsed;
        s.max = s.max.max(elapsed);
    }
    Pass { stats, unresolved }
}

#[derive(Default)]
struct Stats {
    blocks: usize,
    lines: usize,
    bytes: usize,
    total: Duration,
    max: Duration,
}

struct CodeBlock {
    lang: Option<String>,
    text: String,
}

/// Fence-token lookup with the aliases from preen-plan P3. Untagged and `text` blocks stay plain.
fn resolve<'a>(syntaxes: &'a SyntaxSet, lang: Option<&str>) -> Option<&'a SyntaxReference> {
    let token = match lang? {
        "text" | "txt" | "plain" => return None,
        "zsh" | "sh" | "shell" => "bash",
        "kt" => "kotlin",
        other => other,
    };
    syntaxes.find_syntax_by_token(token)
}

fn usage() -> ! {
    eprintln!("usage: spike_highlight [--lang NAME] [<file-or-dir>...]");
    process::exit(2);
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn collect_markdown(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
            {
                continue;
            }
            collect_markdown(&p, out);
        }
    } else if path.extension().is_some_and(|e| e == "md") {
        out.push(path.to_path_buf());
    }
}

fn extract_code_blocks(text: &str, out: &mut Vec<CodeBlock>) {
    let mut current: Option<CodeBlock> = None;
    for event in Parser::new_ext(text, Options::ENABLE_GFM | Options::ENABLE_TABLES) {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                let lang = info.split_whitespace().next().map(str::to_ascii_lowercase);
                current = Some(CodeBlock {
                    lang,
                    text: String::new(),
                });
            }
            Event::Text(t) => {
                if let Some(block) = &mut current {
                    block.text.push_str(&t);
                }
            }
            Event::End(TagEnd::CodeBlock) => out.extend(current.take()),
            _ => {}
        }
    }
}
