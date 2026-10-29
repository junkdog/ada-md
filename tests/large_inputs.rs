//! Documents past `u16::MAX` in some count: headings, columns, table columns, lines, links; and
//! quotes, lists and diagrams nested deeper than a recursive walk's stack holds. The fuzzer never
//! builds one (`-max_len=16384`), and `-a` doesn't catch an `as` cast that cuts a count short
//! (F-012), so these go through the TUI and `--print` in a debug build, where an overflow
//! panics. None may panic.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use ada::app::{Computed, Config, Dir, Effects, Msg, ScrollBy, Size, State};
use ada::filetype::FileType;
use ada::layout::{LayoutParams, LineNumbers, TableMode};
use ada::print::{self, Options};
use ada::record::{Entry, replay_headless};
use ada::theme::Theme;

/// Past `u16::MAX`.
const N: usize = 70_000;

fn wide(n: usize) -> String {
    "x".repeat(n)
}

/// Past the 10,000 quotes that overflowed the main thread's stack (F-012).
const DEEP: usize = 20_000;

/// `mid` inside `n` of `open` … `close`.
fn nested(open: &str, mid: &str, close: &str, n: usize) -> String {
    format!("{}{mid}{}", open.repeat(n), close.repeat(n))
}

fn fence(body: &str, kind: &str) -> String {
    format!("```mermaid\n{kind}\n{body}```\n")
}

fn table(cols: usize) -> String {
    let row = |cell: &str| format!("|{}\n", format!("{cell}|").repeat(cols));
    format!("{}{}{}", row("a"), row("-"), row("b"))
}

/// Each document, by name and file name.
fn documents() -> Vec<(&'static str, &'static str, String)> {
    vec![
        ("headings", "t.md", "# h\n\n".repeat(N)),
        ("wide heading", "t.md", format!("# {}\n\nbody\n", wide(N))),
        ("wide word", "t.md", format!("a {} b\n", wide(N))),
        ("wide words", "t.md", format!("{}\n", "abc ".repeat(N / 2))),
        (
            "wide code",
            "t.md",
            format!(
                "```\n{}\n```\n\n```rust\nlet x = \"{}\";\n```\n",
                wide(N),
                wide(N)
            ),
        ),
        (
            "wide quote",
            "t.md",
            format!("> - [ ] {}\n>   - {}\n", wide(N), "wrap me ".repeat(N / 8)),
        ),
        ("table columns", "t.md", table(N)),
        // 2 × columns, plus borders, passes u16::MAX from 32,768 on
        ("half as many table columns", "t.md", table(N / 2)),
        (
            "wide cell",
            "t.md",
            format!("| a | b |\n|---|---|\n| {} | c |\n", wide(N)),
        ),
        (
            "table rows",
            "t.md",
            format!("| a | b |\n|---|--:|\n{}", "| x | 1 |\n".repeat(N)),
        ),
        ("links", "t.md", "[x](a.md) ".repeat(N)),
        (
            "footnotes",
            "t.md",
            (0..N)
                .map(|i| format!("a[^{i}]\n\n[^{i}]: n\n\n"))
                .collect(),
        ),
        ("list items", "t.md", "- a\n".repeat(N)),
        ("deep quote", "t.md", nested(">", "a\n", "", DEEP)),
        ("deep list", "t.md", nested("- ", "[ ] a\n", "", DEEP)),
        (
            "deep subgraphs",
            "t.md",
            fence(
                &nested("subgraph s\n", "A-->B\n", "end\n", DEEP),
                "flowchart TD",
            ),
        ),
        (
            "deep states",
            "t.md",
            fence(
                &nested("state S {\n", "A-->B\n", "}\n", DEEP),
                "stateDiagram-v2",
            ),
        ),
        (
            "deep sequence",
            "t.md",
            fence(
                &nested("loop x\n", "A->>B: hi\n", "end\n", DEEP),
                "sequenceDiagram",
            ),
        ),
        ("deep json", "t.json", nested("[", "", "]", DEEP)),
        ("deep toml", "t.toml", nested("a = [", "", "]", DEEP)),
        (
            "wide link",
            "t.md",
            format!("[{}](a.md) and <https://{}.com>\n", wide(N), wide(N)),
        ),
        (
            "wide diagram",
            "t.md",
            format!(
                "```mermaid\nsequenceDiagram\nA->>B: {}\n```\n\n```mermaid\nflowchart LR\nA[{}] --> B\n```\n",
                wide(N),
                wide(N)
            ),
        ),
        ("lines", "t.txt", "a\n".repeat(N)),
        ("wide line", "t.txt", format!("{}\n", wide(N))),
        ("wide source", "t.rs", format!("fn f() {{ {} }}\n", wide(N))),
        (
            "wide log",
            "t.log",
            format!("2024-01-01 INFO {}\n", wide(N)),
        ),
        (
            "wide json",
            "t.json",
            format!("{{\"a\": \"{}\"}}\n", wide(N)),
        ),
    ]
}

fn session(size: Size) -> Vec<Msg> {
    vec![
        Msg::Init { size },
        Msg::Scroll {
            by: ScrollBy::Bottom,
        },
        Msg::Scroll {
            by: ScrollBy::Page(Dir::Prev),
        },
        Msg::JumpHeading { dir: Dir::Prev },
        Msg::ToggleLineNumbers,
        Msg::ToggleCodeWrap,
        Msg::ScrollCodeX { by: i16::MAX },
        Msg::ScrollCodeX { by: i16::MAX },
        Msg::ScrollCodeX { by: i16::MAX },
        Msg::FocusLink { dir: Dir::Prev },
        Msg::SearchOpen,
        Msg::SearchInput {
            pattern: "x|a|b|1".into(),
        },
        Msg::SearchSubmit,
        Msg::SearchNext { dir: Dir::Prev },
        Msg::Press {
            col: 0,
            row: 0,
            count: 3,
        },
        Msg::Drag {
            col: size.w - 1,
            row: size.h - 1,
        },
        Msg::Release,
        Msg::Yank,
        Msg::OpenOutline,
        Msg::OutlineMove { by: i32::MIN },
        Msg::OutlineMove { by: i32::MAX },
        Msg::OutlineClose,
        Msg::OpenLinks,
        Msg::OutlineMove { by: i32::MAX },
        Msg::OutlineClose,
        Msg::ToggleTableMode,
        Msg::SortOpen,
        Msg::OutlineMove { by: i32::MAX },
        Msg::OutlinePick,
        Msg::SortTable { dir: Dir::Prev },
        Msg::ToggleTableMode,
        Msg::Resize {
            size: Size { w: 7, h: 3 },
        },
        Msg::Resize { size },
        Msg::Scroll { by: ScrollBy::Top },
    ]
}

/// Run `check` on each document, a thread each: some take seconds in a debug build.
fn each_document(check: impl Fn(&str, &Path, &str) + Sync) {
    let docs = documents();
    let next = AtomicUsize::new(0);
    // eight at a time: a session of the largest holds ~300 MB in a debug build
    std::thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| {
                while let Some((name, path, text)) = docs.get(next.fetch_add(1, Ordering::Relaxed))
                {
                    check(name, Path::new(path), text);
                }
            });
        }
    });
}

#[test]
fn large_documents_hold_in_the_tui() {
    each_document(|name, path, text| {
        for size in [Size { w: 80, h: 24 }, Size { w: 400, h: 100 }] {
            let mut msgs = session(size);
            msgs.insert(
                1,
                Msg::DocLoaded {
                    path: path.into(),
                    mtime: None,
                    text: text.into(),
                    doc: Computed(None),
                },
            );
            let entries: Vec<Entry> = msgs
                .into_iter()
                .map(|msg| Entry { at_ms: 0, msg })
                .collect();
            let mut state = State::new(Theme::builtin(), Config::default());
            let frames = replay_headless(&entries, &mut state, &mut Effects::new(None), false);
            assert!(frames.is_ok(), "{name} at {size:?}");
        }
    });
}

#[test]
fn large_documents_hold_in_print() {
    each_document(|name, path, text| {
        let theme = Theme::builtin();
        let filetype = FileType::of(path, text);
        let doc = filetype.parse(path, text);
        for (width, max_width, table_mode, line_numbers) in [
            (80, None, TableMode::Auto, LineNumbers::Off),
            (u16::MAX, None, TableMode::Grid, LineNumbers::All),
            (300, Some(60), TableMode::Records, LineNumbers::Code),
        ] {
            // a code row holds its fill to the width: 64 KB a row at the widest
            if width == u16::MAX && text.lines().nth(1000).is_some() {
                continue;
            }
            let params = LayoutParams {
                width,
                max_width,
                table_mode,
                code_wrap: true,
                line_numbers,
                glyphs_gen: theme.glyphs.generation(),
                images: None,
            };
            let opts = Options {
                align: Some(N as u32 / 2),
                rows: Some(2000),
                ..Options::default()
            };
            let out = print::render(&doc, &theme, filetype.params(params), opts);
            assert!(!out.is_empty(), "{name} at {width}");
        }
    });
}
