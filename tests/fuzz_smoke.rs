//! A deterministic stand-in for `cargo fuzz` (plan P7) that runs with the other tests: random
//! markdown-ish documents, made of the fragments the parser and layout find hardest, go
//! through parse → layout → paint at random sizes, with the toggles and a search in between.
//! None may panic. The real fuzzer lives in `fuzz/`.

use ada::app::{Computed, Config, Dir, Effects, Msg, ScrollBy, Size, State};
use ada::record::{Entry, replay_headless};
use ada::theme::Theme;

const FRAGMENTS: &[&str] = &[
    "# ",
    "## ",
    "###### ",
    "\n",
    "\n\n",
    "  ",
    "\t",
    "> ",
    "> [!NOTE]\n",
    "- ",
    "* ",
    "1. ",
    "- [ ] ",
    "- [x] ",
    "```",
    "```rust\n",
    "~~~\n",
    "    code\n",
    "|",
    "| a | b |\n",
    "|---|:-:|\n",
    "| --- |",
    "**",
    "*",
    "_",
    "__",
    "~~",
    "`",
    "``",
    "[",
    "]",
    "(",
    ")",
    "[x](b.md)",
    "[[wiki]]",
    "[[a#h|alias]]",
    "![img](i.png)",
    "<https://example.com>",
    "[^1]",
    "[^1]: note\n",
    "<b>",
    "</b>",
    "<!-- c -->",
    "---\n",
    "***",
    "\\",
    "&amp;",
    "&#x1F600;",
    "✅",
    "🠶",
    "日本語",
    "e\u{301}",
    "\u{200b}",
    "\u{fe0f}",
    "wide words that wrap around the column edge ",
    "averyveryverylongwordwithoutanyspacesatallthatmustbreaksomewhere",
];

/// xorshift64*: deterministic and dependency-free.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn document(rng: &mut Rng) -> String {
    let len = rng.below(200);
    (0..len)
        .map(|_| FRAGMENTS[rng.below(FRAGMENTS.len() as u64) as usize])
        .collect()
}

fn size(rng: &mut Rng) -> Size {
    Size {
        w: 1 + rng.below(160) as u16,
        h: 1 + rng.below(50) as u16,
    }
}

#[test]
fn random_documents_never_panic() {
    let mut rng = Rng(0x00ad_a5ee_d000_0001);
    for _ in 0..400 {
        let text = document(&mut rng);
        replay(&mut rng, &text);
    }
}

/// Statements a sequence diagram is made of, and the odd ones it must survive (ADR-35).
const STATEMENTS: &[&str] = &[
    "participant A\n",
    "actor B as Bob<br/>Smith\n",
    "A->>B: hi\n",
    "B-->>A: a much longer reply that will have to wrap and then be cut short\n",
    "A->>A: to itself\n",
    "C-xA: 日本語のラベル\n",
    "A<<->>C: both ways\n",
    "B-)+C:\n",
    "C--)-B: off\n",
    "autonumber\n",
    "autonumber 7 3\n",
    "Note over A,C: a note across\n",
    "note left of A: left\n",
    "Note right of C: right\n",
    "loop forever\n",
    "alt yes\n",
    "else no\n",
    "par\n",
    "and\n",
    "end\n",
    "box Aqua group\n",
    "rect rgb(0,0,0)\n",
    "activate A\n",
    "title: T\n",
    "%% comment\n",
    "\n",
    "not a statement\n",
];

#[test]
fn random_diagrams_never_panic() {
    let mut rng = Rng(0x00ad_a5ee_d000_0002);
    for _ in 0..300 {
        let len = rng.below(40);
        let body: String = (0..len)
            .map(|_| STATEMENTS[rng.below(STATEMENTS.len() as u64) as usize])
            .collect();
        let text = format!(
            "> - ```mermaid\n>   sequenceDiagram\n{body}```\n```mermaid\nsequenceDiagram\n{body}```\n"
        );
        replay(&mut rng, &text);
    }
}

/// Statements flowcharts and state diagrams are made of, and odd ones (DG-3).
const GRAPH_STATEMENTS: &[&str] = &[
    "A[Start] --> B{ok?}\n",
    "B -->|yes| C(round)\n",
    "B -- no --> D([stadium])\n",
    "C -.-> A\n",
    "D ==> E((circle)) --- A\n",
    "E --o F[\"quoted [x]\"] --x A\n",
    "A & B --> G & H\n",
    "G <--> G\n",
    "H ---> I[日本語のラベル that is long enough to wrap]\n",
    "subgraph one [One]\n",
    "subgraph two\n",
    "end\n",
    "one --> two\n",
    "A ~~~ H\n",
    "classDef x fill:#f00\n",
    "%% comment\n",
    "\n",
    "not --> a --> [statement\n",
];

const STATE_STATEMENTS: &[&str] = &[
    "[*] --> Idle\n",
    "Idle --> Busy : work\n",
    "Busy --> Idle\n",
    "Busy --> Busy : again\n",
    "Busy --> [*]\n",
    "state Busy {\n",
    "state \"A long name\" as L\n",
    "state F <<fork>>\n",
    "state C <<choice>>\n",
    "L --> F\n",
    "F --> C\n",
    "--\n",
    "}\n",
    "note right of Idle : a note\n",
    "note left of L\n",
    "end note\n",
    "direction LR\n",
    "Idle : waiting\n",
];

#[test]
fn random_graphs_never_panic() {
    let mut rng = Rng(0x00ad_a5ee_d000_0003);
    for i in 0..300 {
        let (header, statements) = match i % 3 {
            0 => ("flowchart TD", GRAPH_STATEMENTS),
            1 => ("graph LR", GRAPH_STATEMENTS),
            _ => ("stateDiagram-v2", STATE_STATEMENTS),
        };
        let len = rng.below(40);
        let body: String = (0..len)
            .map(|_| statements[rng.below(statements.len() as u64) as usize])
            .collect();
        let text =
            format!("> ```mermaid\n> {header}\n{body}```\n```mermaid\n{header}\n{body}```\n");
        replay(&mut rng, &text);
    }
}

/// Statements gantt charts are made of, and odd ones (DG-4).
const GANTT_STATEMENTS: &[&str] = &[
    "title A plan\n",
    "dateFormat YYYY-MM-DD\n",
    "dateFormat HH:mm\n",
    "axisFormat %a %e\n",
    "tickInterval 1day\n",
    "excludes weekends, 2024-01-03\n",
    "section Build\n",
    "Parser : done, p, 2024-01-01, 3d\n",
    "Layout : active, l, after p, 2d\n",
    "Paint : crit, after l q, 1w\n",
    "Quick : q, 09:30, 90m\n",
    "Release : milestone, until p\n",
    "A task with a name long enough to be cut short : 1d\n",
    "Far : 9999-12-30, 1y\n",
    "not a task\n",
];

#[test]
fn random_gantts_never_panic() {
    let mut rng = Rng(0x00ad_a5ee_d000_0005);
    for _ in 0..300 {
        let len = rng.below(30);
        let body: String = (0..len)
            .map(|_| GANTT_STATEMENTS[rng.below(GANTT_STATEMENTS.len() as u64) as usize])
            .collect();
        let text = format!("> ```mermaid\n> gantt\n{body}```\n```mermaid\ngantt\n{body}```\n");
        replay(&mut rng, &text);
    }
}

fn replay(rng: &mut Rng, text: &str) {
    {
        let mut msgs = vec![
            Msg::Init { size: size(rng) },
            Msg::DocLoaded {
                path: "fuzz.md".into(),
                mtime: None,
                text: text.into(),
                doc: Computed(None),
            },
        ];
        for _ in 0..12 {
            msgs.push(match rng.below(10) {
                0 => Msg::Resize { size: size(rng) },
                1 => Msg::ToggleTableMode,
                2 => Msg::ToggleCodeWrap,
                3 => Msg::ScrollCodeX { by: 8 },
                4 => Msg::FocusLink { dir: Dir::Next },
                5 => Msg::SearchOpen,
                6 => Msg::SearchInput {
                    pattern: "a|w".into(),
                },
                7 => Msg::SearchSubmit,
                8 => Msg::Scroll {
                    by: ScrollBy::Rows(rng.below(40) as i32 - 20),
                },
                _ => Msg::Scroll {
                    by: ScrollBy::Bottom,
                },
            });
        }
        msgs.push(Msg::Help);
        let entries: Vec<Entry> = msgs
            .into_iter()
            .map(|msg| Entry { at_ms: 0, msg })
            .collect();
        let mut state = State::new(Theme::builtin(), Config::default());
        let frames = replay_headless(&entries, &mut state, &mut Effects::new(None), false);
        assert!(frames.is_ok(), "{text:?}");
    }
}
