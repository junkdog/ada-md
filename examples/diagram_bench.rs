//! Timing harness for mermaid diagrams (ada-mermaid, DG-5): the probe set in `bench/diagrams`
//! and the generated scale series, each parsed, drawn and laid out as a block, beside the goal
//! from the rejected crates' figures (mermaid-text 0.57). Every figure is the best of repeated
//! runs (wall time, and the thread's CPU time over the same batch); pin it to one idle core.
//!
//! ```zsh
//! cargo build --release --example diagram_bench
//! taskset -c 2 target/release/examples/diagram_bench
//! taskset -c 2 target/release/examples/diagram_bench --secs 0.2 vault   # names matching "vault"
//! target/release/examples/diagram_bench --show work                     # draw it, no timing
//! ```

use std::hint::black_box;
use std::path::Path;
use std::time::{Duration, Instant};
use std::{env, fs, process};

use ada::diagram::{self, Diagram};
use ada::doc;
use ada::layout::{self, LayoutParams, TableMode};
use ada::theme::Theme;
use common::cpu;

mod common;

/// A diagram at one width, and mermaid-text's time for it, ada's goal.
struct Case {
    name: String,
    src: String,
    width: u16,
    /// mermaid-text 0.57's parse and render, the goal for ada's parse and draw.
    goal: Option<Duration>,
    /// What else to say about mermaid-text's result.
    note: &'static str,
}

fn main() {
    let mut secs = 1.0;
    let mut show = false;
    let mut filter = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--secs" => {
                secs = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| usage());
            }
            "--show" => show = true,
            "-h" | "--help" => usage(),
            _ => filter = Some(arg),
        }
    }
    let cases: Vec<Case> = cases()
        .into_iter()
        .filter(|c| filter.as_ref().is_none_or(|f| c.name.contains(f.as_str())))
        .collect();
    if cases.is_empty() {
        eprintln!("no diagram matches");
        process::exit(1);
    }
    let theme = Theme::builtin();
    if show {
        for c in &cases {
            println!("{} at {} columns:", c.name, c.width);
            match diagram::parse(&c.src).and_then(|d| diagram::draw(&d, c.width, &theme.glyphs)) {
                Some(drawn) => {
                    for r in 0..drawn.canvas.height() {
                        println!("{}", drawn.canvas.row_text(r));
                    }
                    for (i, n) in drawn.notes.iter().enumerate() {
                        println!("{} {n}", diagram::note_mark(i + 1));
                    }
                }
                None => println!("(shows its source)"),
            }
            println!();
        }
        return;
    }

    let budget = Duration::from_secs_f64(secs);
    println!(
        "| Diagram | Width | Parse | Draw | Parse + draw (CPU) | Layout (CPU) | Drawn | Cut | mermaid-text | Goal |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|");
    let mut missed = 0;
    for c in &cases {
        let parse = best(budget, || {
            black_box(diagram::parse(black_box(&c.src)));
        });
        let parsed = diagram::parse(&c.src);
        let (draw, drawn) = match &parsed {
            Some(d) => (
                Some(time_draw(budget, d, c.width, &theme)),
                diagram::draw(d, c.width, &theme.glyphs),
            ),
            None => (None, None),
        };
        let md = format!("```mermaid\n{}```\n", c.src);
        let parsed_doc = doc::parse(&md);
        let params = LayoutParams {
            width: c.width,
            max_width: None,
            table_mode: TableMode::Auto,
            code_wrap: true,
            line_numbers: Default::default(),
            glyphs_gen: theme.glyphs.generation(),
            images: None,
        };
        let lay = best(budget, || {
            black_box(layout::layout(&parsed_doc, &theme.glyphs, params));
        });
        let total = draw.map(|d| parse + d);
        let verdict = match (c.goal, total) {
            (Some(goal), Some(t)) if t.wall <= goal => {
                format!(
                    "met, {:.1}× faster",
                    goal.as_secs_f64() / t.wall.as_secs_f64()
                )
            }
            (Some(goal), Some(t)) => {
                missed += 1;
                format!("**over** by {}", fmt(t.wall - goal))
            }
            _ => "—".to_owned(),
        };
        let theirs = match c.goal {
            Some(t) => format!("{}{}", fmt(t), c.note),
            None if c.note.is_empty() => "—".to_owned(),
            None => c.note.to_owned(),
        };
        let size = drawn.as_ref().map_or("source".to_owned(), |d| {
            format!("{} × {}", d.canvas.width(), d.canvas.height())
        });
        let cut = drawn.as_ref().map_or(0, |d| d.notes.len());
        println!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            c.name,
            c.width,
            fmt(parse.wall),
            draw.map_or("—".to_owned(), |d| fmt(d.wall)),
            total.map_or("—".to_owned(), both),
            both(lay),
            size,
            cut,
            theirs,
            verdict,
        );
    }
    if missed > 0 {
        println!("\n{missed} over their goal");
    }
}

fn time_draw(budget: Duration, d: &Diagram, width: u16, theme: &Theme) -> Timed {
    best(budget, || {
        black_box(diagram::draw(black_box(d), width, &theme.glyphs));
    })
}

/// The probe set, then the scale series. mermaid-text's figures: `render_with_width` at the
/// case's width, as md-tui calls it, best of runs for a second, pinned (`bench/diagrams/compare`,
/// 2026-10-22).
fn cases() -> Vec<Case> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("bench/diagrams");
    let file = |name: &str| {
        fs::read_to_string(dir.join(format!("{name}.mmd")))
            .unwrap_or_else(|e| panic!("bench/diagrams/{name}.mmd: {e}"))
    };
    let us = |n: f64| Some(Duration::from_secs_f64(n / 1e6));
    let mut out = Vec::new();
    let mut add = |name: &str, src: String, width: u16, goal, note| {
        out.push(Case {
            name: name.to_owned(),
            src,
            width,
            goal,
            note,
        });
    };
    // the stand-in for the work diagram: five participants, twelve long messages
    for width in [400, 120, 100] {
        add("work", file("work"), width, us(20.2), ", 357 columns");
    }
    let probe = [
        ("vault1", us(103.0)),
        ("vault2", us(403.3)),
        ("vault3", us(5_140.0)),
        ("vault4", us(481.3)),
        ("seq", us(6.23)),
        ("ci", us(37.0)),
        ("flow_sub", us(90.8)),
        ("lr_wide", us(97.1)),
        ("state", us(199.8)),
        ("state2", us(139.7)),
        ("gantt-probe", us(2.05)),
    ];
    for (name, goal) in probe {
        add(name, file(name), 100, goal, "");
    }
    add("gantt-probe", file("gantt-probe"), 60, us(2.05), "");
    add("gantt-mermaid", file("gantt-mermaid"), 100, us(3.40), "");
    add(
        "gantt-docs",
        file("gantt-docs"),
        100,
        None,
        "a parse error (end dates)",
    );
    add(
        "gantt-week",
        file("gantt-week"),
        100,
        None,
        "a parse error (a 0d milestone)",
    );
    // class diagrams: mermaid-text fails on all but the last (`class` on its cardinality)
    for name in ["class", "class_docs", "class_atlas"] {
        add(name, file(name), 100, None, "a parse error");
    }
    add(
        "class_terminal",
        file("class_terminal"),
        100,
        us(26.5),
        ", 137 columns",
    );
    // kinds ada leaves as source
    for name in ["er", "pie"] {
        add(name, file(name), 100, None, "");
    }
    let series = [
        (20, us(4_530.0)),
        (40, us(298_210.0)),
        (80, us(3_940_000.0)),
        (120, us(16_700_000.0)),
    ];
    for (n, goal) in series {
        add(&format!("flow{n}"), flowchart(n), 100, goal, "");
    }
    add("flow300", flowchart(300), 100, None, "over 30 s");
    add("seq500", sequence(500), 120, us(426.3), "");
    out
}

/// The probe's flowchart series: `n` nodes in a chain, each with an edge further on.
fn flowchart(n: usize) -> String {
    let mut s = String::from("graph TD\n");
    for i in 1..=n {
        s += &format!("  n{i}[node {i}] --> n{}\n", i + 1);
        s += &format!("  n{i} --> n{}\n", (i * 13) % n + 1);
    }
    s
}

/// The probe's long sequence diagram: `n` messages between eight participants.
fn sequence(n: usize) -> String {
    let mut s = String::from("sequenceDiagram\n");
    for i in 1..=n {
        let from = i % 8;
        s += &format!("  P{from}->>P{}: message {i}\n", from * 3 % 8);
    }
    s
}

/// The fastest run of `f`, in batches big enough to time, for about `budget`, and this
/// thread's CPU time over that batch, per run.
fn best(budget: Duration, mut f: impl FnMut()) -> Timed {
    let start = Instant::now();
    f();
    let once = start.elapsed().max(Duration::from_nanos(1));
    let batch = (Duration::from_micros(200).as_nanos() / once.as_nanos()).clamp(1, 10_000) as u32;
    let mut best = Timed {
        wall: Duration::MAX,
        cpu: Duration::MAX,
    };
    let start = Instant::now();
    while best.wall == Duration::MAX || start.elapsed() < budget {
        let (t, c) = (Instant::now(), cpu());
        for _ in 0..batch {
            f();
        }
        let wall = t.elapsed() / batch;
        if wall < best.wall {
            best = Timed {
                wall,
                cpu: cpu().saturating_sub(c) / batch,
            };
        }
    }
    best
}

/// A run's wall time and its thread's CPU time.
#[derive(Clone, Copy)]
struct Timed {
    wall: Duration,
    cpu: Duration,
}

impl std::ops::Add for Timed {
    type Output = Timed;

    fn add(self, o: Timed) -> Timed {
        Timed {
            wall: self.wall + o.wall,
            cpu: self.cpu + o.cpu,
        }
    }
}

/// Wall time, and CPU time after it in parentheses.
fn both(t: Timed) -> String {
    format!("{} ({})", fmt(t.wall), fmt(t.cpu))
}

fn fmt(d: Duration) -> String {
    let us = d.as_secs_f64() * 1e6;
    if us < 10.0 {
        format!("{us:.2} µs")
    } else if us < 1_000.0 {
        format!("{us:.1} µs")
    } else if us < 1e6 {
        format!("{:.2} ms", us / 1e3)
    } else {
        format!("{:.1} s", us / 1e6)
    }
}

fn usage() -> ! {
    eprintln!("usage: diagram_bench [--secs S] [--show] [NAME]");
    process::exit(2);
}
