//! Probe: what a line-per-box layout of the highlight runs (`Vec<Box<[Run]>>`, `CodeSpans`'
//! until HL-002) costs next to a flat one (one `Vec<Run>` plus a `u32` start per line, which
//! `CodeSpans` holds since).
//!
//! For each input it highlights the way the TUI does (`highlight::highlight`, the first built-in
//! theme), then reports:
//! - **memory:** the heap the finished `CodeSpans` holds (`malloc_usable_size`, so glibc's
//!   rounding is counted) and its allocation count, beside the same runs laid out flat;
//! - **build:** pushing the runs line by line into each layout, the boxed one as
//!   `LineState::line` did before HL-002 (a `Vec` per line, then `into_boxed_slice`), as a
//!   share of the highlighting time;
//! - **partials:** the TUI worker's partial results (a copy of every line so far at 256, 512,
//!   1,024 … lines, then dropped when the next arrives), for each layout;
//! - **drop:** freeing the finished spans.
//!
//! A Markdown file stands for its code blocks, summed; any other file is one block in the language
//! ada gives it (`file_language`).
//!
//! ```zsh
//! cargo build --release --example spans_layout
//! taskset -c 2 target/release/examples/spans_layout bench/b2/root/corpus/{code-l.rs,log.log,huge.json}
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use std::{env, fs, process};

use ada::highlight::{self, CodeSpans, Run, SyntaxTheme};
use ada::theme::Theme;

mod common;

use common::{cpu, ms};

/// Counts live heap bytes as malloc rounds them, and allocations.
struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);
static LIVE_BLOCKS: AtomicIsize = AtomicIsize::new(0);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

fn usable(p: *mut u8) -> isize {
    // SAFETY: `p` came from System's malloc family and is still live.
    unsafe { libc::malloc_usable_size(p.cast()) as isize }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded as is.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            LIVE.fetch_add(usable(p), Ordering::Relaxed);
            LIVE_BLOCKS.fetch_add(1, Ordering::Relaxed);
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        LIVE.fetch_sub(usable(p), Ordering::Relaxed);
        LIVE_BLOCKS.fetch_sub(1, Ordering::Relaxed);
        // SAFETY: forwarded as is.
        unsafe { System.dealloc(p, layout) }
    }

    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let old = usable(p);
        // SAFETY: forwarded as is.
        let q = unsafe { System.realloc(p, layout, new_size) };
        if !q.is_null() {
            LIVE.fetch_add(usable(q) - old, Ordering::Relaxed);
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        q
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

#[derive(Clone, Copy)]
struct Heap {
    bytes: isize,
    blocks: isize,
    allocs: usize,
}

fn heap() -> Heap {
    Heap {
        bytes: LIVE.load(Ordering::Relaxed),
        blocks: LIVE_BLOCKS.load(Ordering::Relaxed),
        allocs: ALLOCS.load(Ordering::Relaxed),
    }
}

impl Heap {
    fn since(self, before: Heap) -> Heap {
        Heap {
            bytes: self.bytes - before.bytes,
            blocks: self.blocks - before.blocks,
            allocs: self.allocs - before.allocs,
        }
    }
}

/// Lines boxed one by one, as `CodeSpans` held them before HL-002.
type Boxed = Vec<Box<[Run]>>;

/// The flat layout: line `n`'s runs are `runs[starts[n]..starts[n + 1]]`.
#[derive(Clone)]
struct Flat {
    runs: Vec<Run>,
    starts: Vec<u32>,
}

/// The TUI worker's step and its first partial post (`highlight::STEP`).
const STEP: usize = 256;

/// One block to highlight.
struct Block {
    lang: String,
    text: String,
}

#[derive(Default)]
struct Totals {
    bytes: u64,
    lines: u64,
    nonempty: u64,
    runs: u64,
    hl_wall: Duration,
    hl_cpu: Duration,
    heap_held: isize,
    blocks_held: isize,
    heap_flat: isize,
    blocks_flat: isize,
    hl_allocs: usize,
    build_boxed: Duration,
    build_flat: Duration,
    partial_boxed: Duration,
    partial_flat: Duration,
    partial_copies: u64,
    partial_peak_boxed: isize,
    partial_peak_flat: isize,
    drop_boxed: Duration,
    drop_flat: Duration,
}

fn main() {
    let paths: Vec<PathBuf> = env::args().skip(1).map(PathBuf::from).collect();
    if paths.is_empty() {
        eprintln!("usage: spans_layout FILE...");
        process::exit(2);
    }
    let theme = Theme::builtin();
    let syntax = SyntaxTheme::new(&theme.syntax);
    let reps = env::var("REPS")
        .ok()
        .and_then(|r| r.parse().ok())
        .unwrap_or(5usize);

    println!(
        "{:<14} {:>8} {:>9} {:>9} {:>9} | {:>10} {:>10} {:>9} | {:>9} {:>9} {:>9} | {:>9} {:>9} | {:>8} {:>8}",
        "input",
        "MB",
        "lines",
        "nonempty",
        "runs",
        "heap held",
        "heap flat",
        "blocks",
        "hl ms",
        "build box",
        "build flt",
        "part box",
        "part flt",
        "drop box",
        "drop flt",
    );
    for path in &paths {
        let blocks = blocks(path);
        let t = measure(&blocks, &syntax, reps);
        report(path, &t);
    }
}

fn blocks(path: &Path) -> Vec<Block> {
    let text = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("{}: {e}", path.display());
        process::exit(1);
    });
    if path.extension().is_some_and(|e| e == "md") {
        let doc = ada::doc::parse(&text);
        doc.code_blocks()
            .into_iter()
            .filter_map(|c| {
                let lang = c.lang.as_deref()?;
                highlight::key(c)?;
                Some(Block {
                    lang: lang.to_owned(),
                    text: c.text.to_string(),
                })
            })
            .collect()
    } else {
        let lang = highlight::file_language(path).unwrap_or_else(|| {
            eprintln!("{}: no language", path.display());
            process::exit(1);
        });
        vec![Block {
            lang: lang.to_owned(),
            text,
        }]
    }
}

fn measure(blocks: &[Block], syntax: &SyntaxTheme, reps: usize) -> Totals {
    let mut t = Totals::default();
    for (i, b) in blocks.iter().enumerate() {
        let key = i as u64 + 1;
        // first use compiles the grammar's regexes
        drop(black_box(highlight::highlight(
            key, &b.lang, &b.text, syntax,
        )));

        // highlighting, and the heap its result holds
        let mut best = (Duration::MAX, Duration::MAX);
        let mut spans = CodeSpans::default();
        let mut held = Heap {
            bytes: 0,
            blocks: 0,
            allocs: 0,
        };
        for _ in 0..reps {
            drop(spans);
            let before = heap();
            let (w, c) = (Instant::now(), cpu());
            spans = highlight::highlight(key, &b.lang, &b.text, syntax);
            let (w, c) = (w.elapsed(), cpu() - c);
            held = heap().since(before);
            best = (best.0.min(w), best.1.min(c));
        }
        t.bytes += b.text.len() as u64;
        t.hl_wall += best.0;
        t.hl_cpu += best.1;
        t.heap_held += held.bytes;
        t.blocks_held += held.blocks;
        t.hl_allocs += held.allocs;

        let lines: Vec<&[Run]> = (0..spans.line_count()).map(|n| spans.line(n)).collect();
        t.lines += lines.len() as u64;
        t.nonempty += lines.iter().filter(|l| !l.is_empty()).count() as u64;
        t.runs += lines.iter().map(|l| l.len() as u64).sum::<u64>();

        // the flat layout's heap, at exact capacity as `into_boxed_slice` left the boxed one
        let before = heap();
        let flat = shrink(build_flat(&lines));
        let h = heap().since(before);
        t.heap_flat += h.bytes;
        t.blocks_flat += h.blocks;
        drop(flat);

        // building each layout line by line
        t.build_boxed += best_of(reps, || drop(black_box(build_boxed(&lines))));
        t.build_flat += best_of(reps, || drop(black_box(shrink(build_flat(&lines)))));

        // the worker's partial copies
        let boxed = build_boxed(&lines);
        let flat = shrink(build_flat(&lines));
        let (d, copies, peak) = partials(lines.len(), |n| {
            let copy: Boxed = boxed[..n].to_vec();
            black_box(copy)
        });
        t.partial_boxed += d;
        t.partial_copies += copies;
        t.partial_peak_boxed = t.partial_peak_boxed.max(peak);
        let (d, _, peak) = partials(lines.len(), |n| {
            let end = flat.starts[n] as usize;
            let copy = Flat {
                runs: flat.runs[..end].to_vec(),
                starts: flat.starts[..=n].to_vec(),
            };
            black_box(copy)
        });
        t.partial_flat += d;
        t.partial_peak_flat = t.partial_peak_flat.max(peak);
        drop((boxed, flat));

        // freeing the finished spans
        t.drop_boxed += best_of(reps, || {
            let v = build_boxed(&lines);
            let s = Instant::now();
            drop(black_box(v));
            s.elapsed()
        });
        t.drop_flat += best_of(reps, || {
            let v = shrink(build_flat(&lines));
            let s = Instant::now();
            drop(black_box(v));
            s.elapsed()
        });
        drop(spans);
    }
    t
}

/// The fastest of `reps` runs of `f`, which times itself when it returns a `Duration`.
fn best_of<R: Timed>(reps: usize, mut f: impl FnMut() -> R) -> Duration {
    (0..reps)
        .map(|_| {
            let s = Instant::now();
            let r = f();
            r.time(s.elapsed())
        })
        .min()
        .unwrap_or_default()
}

trait Timed {
    fn time(self, elapsed: Duration) -> Duration;
}

impl Timed for () {
    fn time(self, elapsed: Duration) -> Duration {
        elapsed
    }
}

impl Timed for Duration {
    fn time(self, _: Duration) -> Duration {
        self
    }
}

/// `LineState::line`'s pattern before HL-002: a `Vec` per line, boxed at its length.
fn build_boxed(lines: &[&[Run]]) -> Boxed {
    let mut out = Vec::new();
    for line in lines {
        let mut runs: Vec<Run> = Vec::new();
        for r in *line {
            runs.push(*r);
        }
        out.push(runs.into_boxed_slice());
    }
    out
}

fn build_flat(lines: &[&[Run]]) -> Flat {
    let mut runs = Vec::new();
    let mut starts = vec![0];
    for line in lines {
        runs.extend_from_slice(line);
        starts.push(runs.len() as u32);
    }
    Flat { runs, starts }
}

fn shrink(mut f: Flat) -> Flat {
    f.runs.shrink_to_fit();
    f.starts.shrink_to_fit();
    f
}

/// The worker's posts: after each `STEP` lines, once the count reaches the next post, a copy of
/// every line so far, which replaces (drops) the copy before it. Total time, copy count, and the
/// largest copy's heap.
fn partials<T>(lines: usize, mut copy: impl FnMut(usize) -> T) -> (Duration, u64, isize) {
    let start = Instant::now();
    let mut held: Option<T> = None;
    let mut post_at = STEP;
    let mut copies = 0;
    let mut peak = 0;
    let mut done = 0;
    while done < lines {
        done = (done + STEP).min(lines);
        // the worker posts only while lines remain; the finished result moves, not copies
        if done < lines && done >= post_at {
            post_at = done * 2;
            let before = heap();
            let c = copy(done);
            peak = peak.max(heap().since(before).bytes);
            held = Some(c);
            copies += 1;
        }
    }
    drop(held);
    (start.elapsed(), copies, peak)
}

fn mib(b: isize) -> f64 {
    b as f64 / (1024.0 * 1024.0)
}

fn report(path: &Path, t: &Totals) {
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let hl = ms(t.hl_wall);
    println!(
        "{:<14} {:>8.2} {:>9} {:>9} {:>9} | {:>8.2}Mi {:>8.2}Mi {:>9} | {:>9.1} {:>9.2} {:>9.2} | {:>9.2} {:>9.2} | {:>8.2} {:>8.2}",
        name,
        t.bytes as f64 / 1e6,
        t.lines,
        t.nonempty,
        t.runs,
        mib(t.heap_held),
        mib(t.heap_flat),
        t.blocks_held,
        hl,
        ms(t.build_boxed),
        ms(t.build_flat),
        ms(t.partial_boxed),
        ms(t.partial_flat),
        ms(t.drop_boxed),
        ms(t.drop_flat),
    );
    println!(
        "{:<14} hl cpu {:.1} ms, {} allocs while highlighting; build box {:.1}% / flat {:.1}% of hl; {} partial copies, largest {:.2} / {:.2} MiB",
        "",
        ms(t.hl_cpu),
        t.hl_allocs,
        100.0 * ms(t.build_boxed) / hl,
        100.0 * ms(t.build_flat) / hl,
        t.partial_copies,
        mib(t.partial_peak_boxed),
        mib(t.partial_peak_flat),
    );
}
