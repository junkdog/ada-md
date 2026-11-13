//! What `lex` gives: each line's spans, with their text and scopes.
//!
//! ```zsh
//! cargo run -p ada-parse --example spans                    # a built-in Rust snippet
//! cargo run -p ada-parse --example spans -- json < x.json   # stdin, in a fence token's language
//! ```

use std::io::Read;

use ada_parse::{Lang, State, lex};

const SNIPPET: &str = r#"/// Greets the world.
fn main() {
    let name = "world";
    println!("hello, {name}!"); // 42 times
}"#;

fn main() {
    let (lang, text) = match std::env::args().nth(1) {
        None => (Lang::from_token("rust").unwrap(), SNIPPET.to_string()),
        Some(token) => {
            // a fence token, a file extension or a language name: `py`, `zsh`, `Makefile`
            let Some(lang) = Lang::from_token(&token) else {
                eprintln!("no lexer for {token:?}");
                std::process::exit(2);
            };
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .expect("reading stdin");
            (lang, text)
        }
    };

    println!("{}, base scope {}", lang.name(), lang.base());
    let mut state = State::default(); // a block starts from the default
    let mut spans = Vec::new(); // reused from line to line
    for (n, line) in text.lines().enumerate() {
        // one line at a time, without its newline; `state` carries what it leaves open
        lex(lang, line, &mut state, &mut spans);
        println!("{:>3} | {line}", n + 1);
        for span in &spans {
            let (start, end) = (span.start as usize, span.end as usize);
            // scopes under the base scope, outermost first; bytes between spans are plain
            let scopes = lang.scopes(span.kind).join(" ");
            println!(
                "    | {start:>3}..{end:<3} {:<16} {scopes}",
                format!("{:?}", &line[start..end])
            );
        }
    }
}
