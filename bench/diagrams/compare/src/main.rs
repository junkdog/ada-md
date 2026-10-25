//! ada's mermaid diagrams against the crates other viewers draw them with: mermaid-text 0.57
//! (md-tui 0.11) and mmdflux 2.6 (leaf 1.29), on the probe set in `bench/diagrams` and the
//! generated series, each called as its viewer calls it. Every figure is the best of repeated
//! runs, wall time and the process's CPU time over the same batch; pin it to one idle core.
//! ada's figure is parse and draw, to the cell grid it paints from; the crates', their render
//! to a string. Prints a table per diagram and, with `--dat`, writes the site's chart data
//! (`site/perf.py`): each kind's diagrams summed, and the 120-node flowchart.
//!
//! ```zsh
//! cargo build --release --manifest-path bench/diagrams/compare/Cargo.toml
//! taskset -c 2 bench/diagrams/compare/target/release/compare --dat bench/b2/plots/mermaid.dat --stamp $(git rev-parse --short HEAD)
//! taskset -c 2 bench/diagrams/compare/target/release/compare --secs 0.2 vault   # names matching "vault"
//! bench/diagrams/compare/target/release/compare --show class   # each renderer's drawing, untimed
//! ```

use std::hint::black_box;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::time::{Duration, Instant};
use std::{env, fs, process};

use ada::diagram;
use ada::theme::Theme;

/// A diagram at one width, and the site chart it counts toward.
struct Case {
    name: String,
    src: String,
    width: u16,
    kind: Option<&'static str>,
}

/// The renderers, as their viewers call them.
const TOOLS: [&str; 3] = ["ada", "mermaid-text", "mmdflux"];

/// One renderer's result on one case.
enum Outcome {
    Drawn {
        t: Timed,
        cols: usize,
        rows: usize,
    },
    /// It returned an error: the diagram isn't drawn.
    Failed,
    /// ada shows the source instead.
    Source,
    Panicked,
}

fn main() {
    let mut secs = 1.0;
    let (mut dat, mut stamp, mut filter, mut show) = (None, None, None, false);
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--secs" => {
                secs = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| usage());
            }
            "--dat" => dat = Some(args.next().unwrap_or_else(|| usage())),
            "--stamp" => stamp = Some(args.next().unwrap_or_else(|| usage())),
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
    // the crates' panics are results here, not noise
    panic::set_hook(Box::new(|_| {}));
    let budget = Duration::from_secs_f64(secs);
    let theme = Theme::builtin();
    if show {
        for c in &cases {
            for tool in TOOLS {
                println!("== {} at {}: {tool}", c.name, c.width);
                match panic::catch_unwind(AssertUnwindSafe(|| draw(tool, c, &theme))) {
                    Ok(Some(s)) => println!("{s}"),
                    Ok(None) => println!("(not drawn)\n"),
                    Err(_) => println!("(panicked)\n"),
                }
            }
        }
        return;
    }

    println!("| Diagram | Width | ada: parse + draw (CPU) | mermaid-text (CPU) | mmdflux (CPU) |");
    println!("|---|---|---|---|---|");
    let mut results = Vec::new();
    for c in &cases {
        let out: Vec<Outcome> = TOOLS
            .iter()
            .map(|tool| run(tool, c, budget, &theme))
            .collect();
        let cells: Vec<String> = out.iter().map(cell).collect();
        println!("| {} | {} | {} |", c.name, c.width, cells.join(" | "));
        results.push(out);
    }
    if let Some(path) = dat {
        let text = dat_file(&cases, &results, stamp.as_deref().unwrap_or("?"));
        fs::write(&path, text).unwrap_or_else(|e| panic!("{path}: {e}"));
        eprintln!("wrote {path}");
    }
}

/// Times one renderer on one case.
fn run(tool: &str, c: &Case, budget: Duration, theme: &Theme) -> Outcome {
    let glyphs = &theme.glyphs;
    let width = c.width as usize;
    let src = c.src.as_str();
    // one untimed call first: whether it draws, and how big
    let first = panic::catch_unwind(AssertUnwindSafe(|| draw(tool, c, theme)));
    let drawing = match first {
        Err(_) => return Outcome::Panicked,
        Ok(None) if tool == "ada" => return Outcome::Source,
        Ok(None) => return Outcome::Failed,
        Ok(Some(s)) => s,
    };
    let t = match tool {
        "ada" => best(budget, || {
            let d = diagram::parse(black_box(src));
            black_box(d.and_then(|d| diagram::draw(&d, c.width, glyphs)));
        }),
        "mermaid-text" => best(budget, || {
            black_box(mermaid_text::render_with_width(black_box(src), Some(width)).ok());
        }),
        _ => best(budget, || {
            black_box(
                mmdflux::render_diagram(
                    black_box(src),
                    mmdflux::OutputFormat::Text,
                    &mmdflux::RenderConfig::default(),
                )
                .ok(),
            );
        }),
    };
    let lines: Vec<&str> = drawing.lines().collect();
    Outcome::Drawn {
        t,
        cols: lines.iter().map(|l| diagram::width(l)).max().unwrap_or(0),
        rows: lines.len(),
    }
}

/// One renderer's drawing of a case, as text; `None` when it doesn't draw it.
fn draw(tool: &str, c: &Case, theme: &Theme) -> Option<String> {
    let src = c.src.as_str();
    match tool {
        "ada" => {
            let d = diagram::parse(src)?;
            let drawn = diagram::draw(&d, c.width, &theme.glyphs)?;
            Some(text(&drawn))
        }
        "mermaid-text" => mermaid_text::render_with_width(src, Some(c.width as usize)).ok(),
        _ => mmdflux::render_diagram(
            src,
            mmdflux::OutputFormat::Text,
            &mmdflux::RenderConfig::default(),
        )
        .ok(),
    }
}

/// ada's drawing as text, row by row, to measure it as the crates' strings are.
fn text(d: &diagram::Drawn) -> String {
    let mut s = String::new();
    for r in 0..d.canvas.height() {
        s += &d.canvas.row_text(r);
        s.push('\n');
    }
    s
}

fn cell(o: &Outcome) -> String {
    match o {
        Outcome::Drawn { t, cols, rows } => {
            format!("{} ({}), {cols} × {rows}", fmt(t.wall), fmt(t.cpu))
        }
        Outcome::Failed => "an error".to_owned(),
        Outcome::Source => "source".to_owned(),
        Outcome::Panicked => "panicked".to_owned(),
    }
}

/// The site's chart data, in `bench/b2/plot.py`'s format: a block per chart, a row, a column per
/// renderer, `?` where it didn't draw every diagram of the chart. The kinds sum their diagrams,
/// in µs; the 120-node flowchart is in ms.
fn dat_file(cases: &[Case], results: &[Vec<Outcome>], stamp: &str) -> String {
    let mut out = format!(
        "# B2 mermaid at ada {stamp}: parse and draw, best of runs, pinned; µs (the 120-node flowchart: ms); \
         written by bench/diagrams/compare\n"
    );
    let mut blocks = Vec::new();
    let mut charts: Vec<(String, Vec<usize>, f64)> = Vec::new();
    for (i, c) in cases.iter().enumerate() {
        if let Some(k) = c.kind {
            match charts.iter_mut().find(|(t, ..)| t == k) {
                Some((_, rows, _)) => rows.push(i),
                None => charts.push((k.to_owned(), vec![i], 1e6)),
            }
        }
    }
    for (i, c) in cases.iter().enumerate() {
        if c.name == "flow120" {
            charts.push(("mermaid, a 120-node flowchart".to_owned(), vec![i], 1e3));
        }
    }
    let tools = [("ada", 0), ("mermaid-text", 1), ("mmdflux", 2)];
    for (title, rows, scale) in charts {
        let names: Vec<&str> = tools.iter().map(|t| t.0).collect();
        let vals: Vec<String> = tools
            .iter()
            .map(|&(_, k)| {
                rows.iter()
                    .map(|&r| match &results[r][k] {
                        Outcome::Drawn { t, .. } => Some(t.wall.as_secs_f64()),
                        _ => None,
                    })
                    .sum::<Option<f64>>()
                    .map_or("?".to_owned(), |s| format!("{:.6}", s * scale))
            })
            .collect();
        let list: Vec<&str> = rows.iter().map(|&r| cases[r].name.as_str()).collect();
        blocks.push(format!(
            "# {title}\n# doc {}\n# of {}\nall {}",
            names.join(" "),
            list.join(" "),
            vals.join(" ")
        ));
    }
    out += &blocks.join("\n\n\n");
    out.push('\n');
    out
}

/// The probe set, then the scale series, at the widths diagram_bench uses.
fn cases() -> Vec<Case> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let file = |name: &str| {
        fs::read_to_string(dir.join(format!("{name}.mmd")))
            .unwrap_or_else(|e| panic!("bench/diagrams/{name}.mmd: {e}"))
    };
    let mut out = Vec::new();
    let mut add = |name: &str, src: String, width: u16, kind| {
        out.push(Case {
            name: name.to_owned(),
            src,
            width,
            kind,
        });
    };
    const SEQ: Option<&str> = Some("mermaid, sequence diagrams");
    const FLOW: Option<&str> = Some("mermaid, flowcharts");
    const STATE: Option<&str> = Some("mermaid, state diagrams");
    const GANTT: Option<&str> = Some("mermaid, gantt charts");
    const CLASS: Option<&str> = Some("mermaid, class diagrams");
    add("work", file("work"), 400, None);
    add("work", file("work"), 100, SEQ);
    add("seq", file("seq"), 100, SEQ);
    for name in [
        "vault1", "vault2", "vault3", "vault4", "ci", "flow_sub", "lr_wide",
    ] {
        add(name, file(name), 100, FLOW);
    }
    for name in ["state", "state2"] {
        add(name, file(name), 100, STATE);
    }
    for name in ["gantt-probe", "gantt-mermaid"] {
        add(name, file(name), 100, GANTT);
    }
    // parse errors in mermaid-text: out of the sum, which compares the same diagrams
    for name in ["gantt-docs", "gantt-week"] {
        add(name, file(name), 100, None);
    }
    for name in ["er", "pie"] {
        add(name, file(name), 100, None);
    }
    // mermaid-text fails on `class`'s cardinality: out of the sum, as above
    add("class", file("class"), 100, None);
    for name in ["class_docs", "class_atlas", "class_terminal"] {
        add(name, file(name), 100, CLASS);
    }
    for n in [20, 40, 80, 120] {
        add(&format!("flow{n}"), flowchart(n), 100, None);
    }
    add("seq500", sequence(500), 120, None);
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

/// The fastest run of `f`, in batches big enough to time, for about `budget` and at least three
/// batches, and the process's CPU time over that batch, per run.
fn best(budget: Duration, mut f: impl FnMut()) -> Timed {
    let start = Instant::now();
    f();
    let once = start.elapsed().max(Duration::from_nanos(1));
    let batch = (Duration::from_micros(200).as_nanos() / once.as_nanos()).clamp(1, 10_000) as u32;
    let mut best = Timed {
        wall: Duration::MAX,
        cpu: Duration::MAX,
    };
    let (start, mut batches) = (Instant::now(), 0);
    while batches < 3 || start.elapsed() < budget {
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
        batches += 1;
    }
    best
}

/// A run's wall time and the process's CPU time.
#[derive(Clone, Copy)]
struct Timed {
    wall: Duration,
    cpu: Duration,
}

/// The process's CPU time, over all its threads.
fn cpu() -> Duration {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `t` is a valid timespec for the call to fill.
    unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut t) };
    Duration::new(t.tv_sec as u64, t.tv_nsec as u32)
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
        format!("{:.2} s", us / 1e6)
    }
}

fn usage() -> ! {
    eprintln!("usage: compare [--secs S] [--dat PATH] [--stamp STR] [--show] [NAME]");
    process::exit(2);
}
