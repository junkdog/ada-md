//! Timing harness for the P1 budgets (plan §1, success criteria 2 and 3): first paint ≤ 30 ms,
//! full relayout ≤ 5 ms, scroll repaint ≤ 2 ms. Also times search (P4): building the searchable
//! text once per document, and one keystroke of an incremental search, drawn. Also a robustness sweep: every note is laid out at
//! several widths and must not panic.
//!
//! ```zsh
//! cargo run --release --example bench -- ~/opt/birb-vault
//! cargo run --release --example bench -- --min-kb 100 ~/opt/birb-vault
//! ```

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{env, fs, process};

use ada::app::{Cmd, Computed, Config, Dir, Msg, ScrollBy, Size, State, update};
use ada::grid::Screen;
use ada::layout::{self, LayoutParams, TableMode};
use ada::search::SearchMap;
use ada::theme::Theme;
use ada::{doc, ui};
use ratatui::layout::Size as Cells;

const ITERATIONS: usize = 15;
const FIRST_PAINT: Duration = Duration::from_millis(30);
const RELAYOUT: Duration = Duration::from_millis(5);
const REPAINT: Duration = Duration::from_millis(2);
const SWEEP_WIDTHS: [u16; 5] = [20, 40, 80, 135, 200];

fn main() {
    let mut roots = Vec::new();
    let mut min_kb = 0u64;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--min-kb" => {
                min_kb = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| usage());
            }
            "-h" | "--help" => usage(),
            _ => roots.push(PathBuf::from(arg)),
        }
    }
    if roots.is_empty() {
        usage();
    }
    let mut files = Vec::new();
    for root in &roots {
        collect_markdown(root, &mut files);
    }
    files.sort_by_key(|p| fs::metadata(p).map_or(0, |m| m.len()));

    let theme = Theme::builtin();

    // Robustness: every note at every sweep width.
    let start = Instant::now();
    for path in &files {
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let doc = doc::parse(&text);
        for width in SWEEP_WIDTHS {
            layout::layout(&doc, &theme.glyphs, params(&theme, width));
        }
    }
    println!(
        "sweep: {} notes × {} widths in {:.0} ms, no panics\n",
        files.len(),
        SWEEP_WIDTHS.len(),
        ms(start.elapsed())
    );

    println!(
        "{:>8} {:>6} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}  file",
        "bytes", "rows", "parse", "1st paint", "relayout", "repaint", "srch map", "srch key"
    );
    let mut over = 0;
    for path in files
        .iter()
        .filter(|p| fs::metadata(p).is_ok_and(|m| m.len() >= min_kb * 1024))
    {
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let m = measure(&theme, &text);
        let bad = m.first_paint > FIRST_PAINT || m.relayout > RELAYOUT || m.repaint > REPAINT;
        over += usize::from(bad);
        println!(
            "{:>8} {:>6} {:>9.3} {:>9.3} {:>9.3} {:>9.3} {:>9.3} {:>9.3}  {}{}",
            text.len(),
            m.rows,
            ms(m.parse),
            ms(m.first_paint),
            ms(m.relayout),
            ms(m.repaint),
            ms(m.search_map),
            ms(m.search_key),
            path.display(),
            if bad { "  OVER BUDGET" } else { "" }
        );
    }
    println!(
        "\nbudgets: first paint {FIRST_PAINT:?}, relayout {RELAYOUT:?}, repaint {REPAINT:?} \
         (medians of {ITERATIONS}); {over} over"
    );
}

struct Measured {
    rows: u32,
    parse: Duration,
    first_paint: Duration,
    relayout: Duration,
    repaint: Duration,
    search_map: Duration,
    search_key: Duration,
}

fn measure(theme: &Theme, text: &str) -> Measured {
    let size = Size { w: 135, h: 40 };
    // diffed and encoded like a real frame, then dropped
    let mut terminal = Screen::new(std::io::sink(), Cells::new(size.w, size.h));

    let parse = median(|| {
        std::hint::black_box(doc::parse(text));
    });

    // First paint: parse + layout + paint, through update like the real thing. Highlighting
    // arrives after first paint and isn't counted.
    let mut state = State::new(theme.clone(), Config::default());
    let mut cmds = Vec::new();
    let first_paint = median(|| {
        state = State::new(theme.clone(), Config::default());
        update(&mut state, Msg::Init { size });
        cmds = update(
            &mut state,
            Msg::DocLoaded {
                path: "bench.md".into(),
                mtime: None,
                text: text.into(),
                doc: Computed(None),
            },
        );
        terminal.draw(|g| ui::view(&state, g)).expect("draw");
    });
    let rows = state.doc.as_ref().map_or(0, |d| d.layout.row_count());
    // ...but repaints include it.
    for cmd in cmds {
        if let Cmd::Highlight(job) = cmd {
            update(&mut state, job.run());
        }
    }

    // Relayout: alternate between two widths so neither cached layout ever matches... except the
    // previous one, which is exactly the resize-drag case. Use three widths to defeat both.
    let widths = [100, 120, 135];
    let mut i = 0;
    let relayout = median(|| {
        i += 1;
        let w = widths[i % widths.len()];
        update(
            &mut state,
            Msg::Resize {
                size: Size { w, h: size.h },
            },
        );
    });

    update(&mut state, Msg::Resize { size });
    update(&mut state, Msg::Scroll { by: ScrollBy::Top });
    terminal.draw(|g| ui::view(&state, g)).expect("draw");
    let mut dir = Dir::Next;
    let repaint = median(|| {
        if state.scroll >= state.max_scroll() {
            dir = Dir::Prev;
        } else if state.scroll == 0 {
            dir = Dir::Next;
        }
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Page(dir),
            },
        );
        terminal.draw(|g| ui::view(&state, g)).expect("draw");
    });

    let doc = doc::parse(text);
    let search_map = median(|| {
        std::hint::black_box(SearchMap::build(&doc, &theme.glyphs));
    });
    update(&mut state, Msg::Scroll { by: ScrollBy::Top });
    update(&mut state, Msg::SearchOpen);
    let patterns = ["the", "th", "t"];
    let mut i = 0;
    let search_key = median(|| {
        i += 1;
        let pattern = patterns[i % patterns.len()].to_owned();
        update(&mut state, Msg::SearchInput { pattern });
        terminal.draw(|g| ui::view(&state, g)).expect("draw");
    });

    Measured {
        rows,
        parse,
        first_paint,
        relayout,
        repaint,
        search_map,
        search_key,
    }
}

fn params(theme: &Theme, width: u16) -> LayoutParams {
    LayoutParams {
        width,
        max_width: None,
        table_mode: TableMode::Auto,
        code_wrap: true,
        line_numbers: Default::default(),
        glyphs_gen: theme.glyphs.generation(),
        images: None,
    }
}

fn median(mut f: impl FnMut()) -> Duration {
    let mut times: Vec<_> = (0..ITERATIONS)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .collect();
    times.sort();
    times[ITERATIONS / 2]
}

fn usage() -> ! {
    eprintln!("usage: bench [--min-kb N] <file-or-dir>...");
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
