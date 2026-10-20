//! Where a print's time goes, in-process, one phase after another: the theme, parsing, layout,
//! loading the syntax set, highlighting cold (compiling the regexes it meets) and warm, and the
//! whole print again, warm. A fresh process is one cold run, so run it a few times (PRN-01).
//!
//! ```zsh
//! cargo build --release --example print_phases
//! for i in 1 2 3; do taskset -c 2 target/release/examples/print_phases bench/b2/root/corpus/s.md; done
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{env, fs};

use ada::highlight::{self, Highlights, SyntaxTheme};
use ada::layout::{self, LayoutParams};
use ada::{app, doc, print, theme};

fn main() {
    app::one_arena();
    let path = env::args().nth(1).expect("a markdown file");
    let mut times: Vec<(&str, Duration)> = Vec::new();
    let mut time = |name, start: Instant| times.push((name, start.elapsed()));
    let start = Instant::now();
    let t = Instant::now();
    let (theme, _) = theme::load(None).expect("the theme");
    time("theme", t);
    let text = fs::read_to_string(&path).expect("the file");
    let t = Instant::now();
    let doc = doc::parse(&text);
    time("parse", t);
    let params = LayoutParams {
        width: 100,
        max_width: None,
        table_mode: Default::default(),
        code_wrap: true,
        line_numbers: Default::default(),
        glyphs_gen: theme.glyphs.generation(),
        images: None,
    };
    let t = Instant::now();
    let rows = layout::layout(&doc, &theme.glyphs, params).rows.len();
    time("layout", t);
    let t = Instant::now();
    highlight::syntaxes();
    time("syntax set", t);
    let syntax = Arc::new(SyntaxTheme::new(&theme.syntax));
    for name in ["highlight cold", "highlight warm"] {
        let t = Instant::now();
        Highlights::compute_all(&doc.code_blocks(), &syntax);
        time(name, t);
    }
    let t = Instant::now();
    let out = print::render(&doc, &theme, params, print::Options::default());
    time("print warm", t);
    time("total", start);
    for (name, d) in times {
        print!("{name} {:.3}  ", d.as_secs_f64() * 1e3);
    }
    println!("ms; rows {rows}, output {} bytes", out.len());
}
