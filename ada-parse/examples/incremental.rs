//! Highlighting again after an edit, as an editor would. Keep the `State` each line starts
//! from; after an edit, lex from the edited line on, and stop at the first line that leaves
//! the same state it left before: the lines after it highlight as they did.
//!
//! For a language whose `Lang::line_by_line()` is true, no states are needed at all: every
//! line starts from `State::default()`, in any order, on any thread.
//!
//! ```zsh
//! cargo run -p ada-parse --example incremental
//! ```

use ada_parse::{Lang, Span, State, lex};

struct Buffer {
    lang: Lang,
    lines: Vec<String>,
    /// `starts[i]` is the state line `i` is lexed from; one more than there are lines.
    starts: Vec<State>,
    /// Each line's spans, as a renderer would keep them.
    spans: Vec<Vec<Span>>,
}

impl Buffer {
    fn new(lang: Lang, text: &str) -> Buffer {
        let lines: Vec<String> = text.lines().map(String::from).collect();
        let mut buffer = Buffer {
            lang,
            starts: vec![State::default(); lines.len() + 1],
            spans: vec![Vec::new(); lines.len()],
            lines,
        };
        // the first pass lexes every line
        let mut state = State::default();
        for i in 0..buffer.lines.len() {
            lex(lang, &buffer.lines[i], &mut state, &mut buffer.spans[i]);
            buffer.starts[i + 1] = state;
        }
        buffer
    }

    /// Replace line `i`, lex again what it changes, and return how many lines that took.
    fn edit(&mut self, i: usize, line: &str) -> usize {
        self.lines[i] = line.to_string();
        let mut state = self.starts[i];
        for j in i..self.lines.len() {
            lex(self.lang, &self.lines[j], &mut state, &mut self.spans[j]);
            if state == self.starts[j + 1] {
                return j - i + 1; // `State` is `Eq`: the rest is as it was
            }
            self.starts[j + 1] = state;
        }
        self.lines.len() - i
    }

    /// The outermost scope of a line's first span, for a glance at what it became.
    fn first_scope(&self, i: usize) -> &'static str {
        self.spans[i]
            .first()
            .and_then(|s| self.lang.scopes(s.kind).first().copied())
            .unwrap_or("(plain)")
    }
}

const TEXT: &str = r#"fn main() {
    let msg = "hello";
    let n = 42;
    if n > 0 {
        println!("{msg}, {n}");
    }
}"#;

fn main() {
    let mut buffer = Buffer::new(Lang::from_token("rust").unwrap(), TEXT);
    let total = buffer.lines.len();
    let edits = [
        (1, r#"    let msg = "hi there";"#, "a string's text changes"),
        (1, r#"    let msg = "hello"; /*"#, "a block comment opens"),
        (5, r#"    }*/"#, "and closes"),
        (1, r#"    let msg = "hello";"#, "the opening is undone"),
    ];
    for (i, line, what) in edits {
        let lexed = buffer.edit(i, line);
        println!("line {}: {what}: lexed {lexed} of {total} lines", i + 1);
        println!("    line 3 now starts with {}", buffer.first_scope(2));
    }
}
