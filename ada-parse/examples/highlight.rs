//! A small `cat` with colors: the language from the file's name, else from its first line
//! (`#!/usr/bin/env python3`), and each span's color picked from its scopes by a tiny theme.
//!
//! ```zsh
//! cargo run -p ada-parse --example highlight -- ada-parse/src/lib.rs
//! ```

use std::path::Path;

use ada_parse::{Lang, State, lex};

/// Scope selectors and the ANSI SGR codes they color with. A selector matches a scope that is
/// it, or starts with it and a dot: `string` matches `string.quoted.double.rust`.
const THEME: &[(&str, &str)] = &[
    ("comment", "2;3"),                  // dim italic
    ("string", "32"),                    // green
    ("constant.character.escape", "36"), // cyan
    ("constant", "35"),                  // magenta: numbers, `true`, `None`
    ("keyword", "31"),                   // red
    ("storage", "31"),                   // `fn`, `struct`, `let`
    ("entity.name.function", "1;34"),    // bold blue
    ("support.function", "34"),          // blue
    ("entity.name", "33"),               // yellow: types, tags, headings
    ("support.type", "33"),
    ("entity.other.attribute-name", "33"),
    ("variable.parameter", "3"), // italic
    ("invalid", "37;41"),        // white on red
];

/// A span's color, as a TextMate theme picks one: the innermost scope any selector matches
/// decides, by its longest selector.
fn color(scopes: &[&str]) -> Option<&'static str> {
    scopes.iter().rev().find_map(|scope| {
        THEME
            .iter()
            .filter(|(sel, _)| {
                scope
                    .strip_prefix(sel)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
            })
            .max_by_key(|(sel, _)| sel.len())
            .map(|&(_, sgr)| sgr)
    })
}

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: highlight FILE");
        std::process::exit(2);
    };
    let text = std::fs::read_to_string(&path).expect("reading the file");

    // the file's name (`Makefile`, `x.rs`, `.zshrc`), then its first line
    let name = Path::new(&path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let lang =
        Lang::from_file_name(name).or_else(|| text.lines().next().and_then(Lang::from_first_line));
    let Some(lang) = lang else {
        print!("{text}"); // no lexer for it: plain text
        return;
    };

    let mut state = State::default();
    let mut spans = Vec::new();
    let mut out = String::new();
    for line in text.lines() {
        lex(lang, line, &mut state, &mut spans);
        let mut at = 0;
        for span in &spans {
            let (start, end) = (span.start as usize, span.end as usize);
            out.push_str(&line[at..start]); // the plain bytes before it
            match color(&lang.scopes(span.kind)) {
                Some(sgr) => out.push_str(&format!("\x1b[{sgr}m{}\x1b[0m", &line[start..end])),
                None => out.push_str(&line[start..end]),
            }
            at = end;
        }
        out.push_str(&line[at..]);
        out.push('\n');
    }
    print!("{out}");
}
