//! Throughput in source bytes a second (BEN-009; ada-throughput in the vault): markdown as a
//! whole (B2's documents), each syntax on its own (generated documents), and ada's own lexers,
//! stage by stage, each beside the ceiling kernels for that input. Every cell is the median of
//! its samples of this thread's CPU time, wall time, cycles and instructions, a JSON line on
//! stdout; progress goes to stderr. Linux only: the counters are `perf_event_open(2)`'s, and
//! `perf_event_paranoid` must be 1.
//!
//! Pinned to one CPU, so print paints on one thread; `bench/throughput/run.zsh` runs it under the
//! bench profile and keeps the records.
//!
//! ```zsh
//! cargo build --release --example throughput
//! taskset -c 2 target/release/examples/throughput ~/b2/corpus > cells.jsonl
//! taskset -c 2 target/release/examples/throughput ~/b2/corpus --only xl,prose --samples 5
//! ```

#[cfg(target_os = "linux")]
#[path = "throughput/ceilings.rs"]
mod ceilings;
#[cfg(target_os = "linux")]
mod common;
#[cfg(target_os = "linux")]
#[path = "throughput/counters.rs"]
mod counters;
#[cfg(target_os = "linux")]
#[path = "throughput/generate.rs"]
mod generate;

#[cfg(target_os = "linux")]
fn main() {
    linux::main();
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("throughput: Linux only (perf_event_open)");
    std::process::exit(2);
}

#[cfg(target_os = "linux")]
mod linux {
    use std::fmt::Write as _;
    use std::hint::black_box;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};
    use std::{env, fs, process, thread};

    use ada::doc::{self, Block, BlockKind, Doc};
    use ada::highlight::{self, SyntaxTheme};
    use ada::layout::{self, LayoutParams};
    use ada::theme::Theme;
    use ada::{json, logs, print};
    use syntect::parsing::ParseState;
    use syntect::util::LinesWithEndings;

    use crate::ceilings::{self, Set};
    use crate::common::cpu;
    use crate::counters::{self, Counters};
    use crate::generate;

    /// Samples a cell; each sample repeats the call until it takes about [`SAMPLE`].
    const SAMPLES: usize = 15;
    /// syntect's, a reference that takes seconds a call on the lexers' inputs (90 s on the
    /// one-line JSON): one sample, no warm-up.
    const SYNTECT_SAMPLES: usize = 1;
    const SAMPLE: Duration = Duration::from_millis(5);
    /// B2's print width.
    const WIDTH: u16 = 100;

    /// The markdown inputs: B2's documents, then the generated ones.
    const MARKDOWN: [(&str, Option<&str>); 7] = [
        ("s", Some("s.md")),
        ("m", Some("m.md")),
        ("l", Some("l.md")),
        ("xl", Some("xl.md")),
        ("prose", None),
        ("tables", None),
        ("links", None),
    ];

    /// ada's own lexers: id, input file, language, the format's set for C1.
    const LEXERS: [(&str, &str, &str, &[u8]); 2] = [
        ("log", "log.log", "log", ceilings::LOG),
        ("json", "huge.json", "json", ceilings::JSON),
    ];

    struct Args {
        corpus: PathBuf,
        only: Option<Vec<String>>,
        samples: usize,
    }

    fn usage() -> ! {
        eprintln!("usage: throughput CORPUS_DIR [--only ID,ID…] [--samples N]");
        process::exit(2);
    }

    fn args() -> Args {
        let mut corpus = None;
        let mut only = None;
        let mut samples = SAMPLES;
        let mut args = env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--only" => {
                    let ids = args.next().unwrap_or_else(|| usage());
                    only = Some(ids.split(',').map(str::to_owned).collect());
                }
                "--samples" => {
                    samples = args
                        .next()
                        .and_then(|n| n.parse().ok())
                        .unwrap_or_else(|| usage());
                }
                _ if arg.starts_with('-') => usage(),
                _ => corpus = Some(PathBuf::from(arg)),
            }
        }
        Args {
            corpus: corpus.unwrap_or_else(|| usage()),
            only,
            samples,
        }
    }

    fn fail(msg: &str) -> ! {
        eprintln!("throughput: {msg}");
        process::exit(1);
    }

    /// One cell's medians, per call.
    struct Sample {
        reps: usize,
        cpu_ns: f64,
        wall_ns: f64,
        cycles: f64,
        instructions: f64,
    }

    fn median(mut v: Vec<f64>) -> f64 {
        v.sort_by(f64::total_cmp);
        let n = v.len();
        if n % 2 == 1 {
            v[n / 2]
        } else {
            (v[n / 2 - 1] + v[n / 2]) / 2.0
        }
    }

    /// Time `f`: one call to warm up and size the samples, then `samples` samples of `reps`
    /// calls each. What the calls return is kept until a sample's counters stop, so freeing it
    /// isn't timed.
    fn measure<T>(counters: &Counters, samples: usize, mut f: impl FnMut() -> T) -> Sample {
        let t = Instant::now();
        black_box(f());
        let once = t.elapsed().max(Duration::from_nanos(1));
        let reps = (SAMPLE.as_nanos() / once.as_nanos()).clamp(1, 10_000) as usize;
        sample(counters, samples, reps, f)
    }

    /// [`measure`] without the warm-up: one call a sample, for a reference that takes seconds.
    fn measure_cold<T>(counters: &Counters, samples: usize, f: impl FnMut() -> T) -> Sample {
        sample(counters, samples, 1, f)
    }

    fn sample<T>(
        counters: &Counters,
        samples: usize,
        reps: usize,
        mut f: impl FnMut() -> T,
    ) -> Sample {
        let (mut cpus, mut walls, mut cycles, mut instructions) = (vec![], vec![], vec![], vec![]);
        for _ in 0..samples {
            let mut keep = Vec::with_capacity(reps);
            counters.start();
            let (c, w) = (cpu(), Instant::now());
            for _ in 0..reps {
                keep.push(f());
            }
            let (w, c) = (w.elapsed(), cpu() - c);
            let count = counters.stop();
            drop(black_box(keep));
            let per = reps as f64;
            cpus.push(c.as_nanos() as f64 / per);
            walls.push(w.as_nanos() as f64 / per);
            cycles.push(count.cycles as f64 / per);
            instructions.push(count.instructions as f64 / per);
        }
        Sample {
            reps,
            cpu_ns: median(cpus),
            wall_ns: median(walls),
            cycles: median(cycles),
            instructions: median(instructions),
        }
    }

    /// Where a cell came from, for its JSON line.
    struct Input<'a> {
        id: &'a str,
        group: &'a str,
        bytes: usize,
        /// The generator's version, for a generated input.
        version: Option<u32>,
        hash: u64,
    }

    fn emit(input: &Input, stage: &str, s: &Sample, extra: &str) {
        let mut line = format!(
            "{{\"input\":\"{}\",\"group\":\"{}\",\"stage\":\"{stage}\",\"bytes\":{},\"hash\":\"{:016x}\"",
            input.id, input.group, input.bytes, input.hash
        );
        if let Some(v) = input.version {
            write!(line, ",\"gen\":{v}").unwrap();
        }
        write!(
            line,
            ",\"reps\":{},\"cpu_ns\":{:.1},\"wall_ns\":{:.1},\"cycles\":{:.1},\"instructions\":{:.1}{extra}}}",
            s.reps, s.cpu_ns, s.wall_ns, s.cycles, s.instructions
        )
        .unwrap();
        println!("{line}");
        eprintln!(
            "  {:<8} {:<10} {:>9.1} MB/s  {:>6.2} cyc/B  {:>6.2} ins/B",
            input.id,
            stage,
            input.bytes as f64 / s.cpu_ns * 1e3,
            s.cycles / input.bytes as f64,
            s.instructions / input.bytes as f64
        );
    }

    /// The code blocks as `txt`, so print doesn't highlight: highlighting is the lexers' and
    /// ada-highlighting's, and runs on a thread of its own the counters don't see.
    fn plain(doc: &mut Doc) {
        fn walk(blocks: &mut [Block]) {
            for block in blocks {
                match &mut block.kind {
                    BlockKind::Code(code) => code.lang = Some("txt".into()),
                    BlockKind::List(list) => {
                        list.items.iter_mut().for_each(|i| walk(&mut i.blocks));
                    }
                    BlockKind::Quote { blocks, .. } => walk(blocks),
                    _ => {}
                }
            }
        }
        walk(&mut doc.blocks);
        for note in &mut doc.footnotes {
            walk(&mut note.blocks);
        }
        assert!(
            doc.code_blocks()
                .iter()
                .all(|b| highlight::key(b).is_none())
        );
    }

    fn read(corpus: &Path, file: &str) -> String {
        let path = corpus.join(file);
        fs::read_to_string(&path).unwrap_or_else(|e| fail(&format!("{}: {e}", path.display())))
    }

    pub fn main() {
        let args = args();
        counters::check_paranoid().unwrap_or_else(|e| fail(&e));
        if thread::available_parallelism().map_or(0, std::num::NonZero::get) != 1 {
            fail("pin it to one CPU (taskset -c 2), so print paints on one thread");
        }
        let counters = Counters::new().unwrap_or_else(|e| fail(&format!("perf_event_open: {e}")));
        let wanted = |id: &str| args.only.as_ref().is_none_or(|o| o.iter().any(|w| w == id));
        let n = args.samples;
        println!(
            "{{\"meta\":{{\"classifier\":\"{}\",\"samples\":{n},\"sample_ms\":{},\"width\":{WIDTH}}}}}",
            ceilings::classifier(),
            SAMPLE.as_millis()
        );

        let theme = Theme::builtin();
        let params = LayoutParams {
            width: WIDTH,
            max_width: None,
            table_mode: Default::default(),
            code_wrap: true,
            line_numbers: Default::default(),
            glyphs_gen: theme.glyphs.generation(),
            images: None,
        };
        let markdown = Set::new(ceilings::MARKDOWN);

        for (id, file) in MARKDOWN.into_iter().filter(|(id, _)| wanted(id)) {
            let (text, group, version) = match file {
                Some(file) => (read(&args.corpus, file), "whole", None),
                None => {
                    let g = generate::by_id(id).expect("a generator");
                    g.check(&doc::parse(&g.text))
                        .unwrap_or_else(|e| fail(&format!("generator {id}: {e}")));
                    (g.text, "syntax", Some(g.version))
                }
            };
            let input = Input {
                id,
                group,
                bytes: text.len(),
                version,
                hash: generate::fnv1a(text.as_bytes()),
            };
            eprintln!("{id}: {} bytes", text.len());

            emit(
                &input,
                "c0",
                &measure(&counters, n, || ceilings::read(text.as_bytes())),
                "",
            );
            emit(
                &input,
                "c1-parse",
                &measure(&counters, n, || markdown.classify(text.as_bytes())),
                "",
            );
            emit(
                &input,
                "c2",
                &measure(&counters, n, || ceilings::events(&text)),
                "",
            );
            emit(
                &input,
                "parse",
                &measure(&counters, n, || doc::parse(&text)),
                "",
            );

            let doc = doc::parse(&text);
            let words = ceilings::Text::new(&ceilings::text_of(&doc));
            let extra = format!(",\"text_bytes\":{}", words.len());
            emit(
                &input,
                "c1-layout",
                &measure(&counters, n, || ceilings::widths(&words)),
                &extra,
            );
            let rows = layout::layout(&doc, &theme.glyphs, params).rows.len();
            let extra = format!(",\"rows\":{rows}");
            let s = measure(&counters, n, || layout::layout(&doc, &theme.glyphs, params));
            emit(&input, "layout", &s, &extra);

            let mut doc = doc::parse(&text);
            plain(&mut doc);
            let out = print::render(&doc, &theme, params, print::Options::default());
            let extra = format!(",\"out_bytes\":{}", out.len());
            let mut into = vec![0u8; out.len()];
            let s = measure(&counters, n, || ceilings::write(out.as_bytes(), &mut into));
            emit(&input, "c1-print", &s, &extra);
            // `render` copies its output into a String once more than a print to stdout does:
            // about a C1 print's worth, a few percent of the stage
            let s = measure(&counters, n, || {
                print::render(&doc, &theme, params, print::Options::default())
            });
            emit(&input, "print", &s, &extra);
        }

        let syntax_theme = SyntaxTheme::new(&theme.syntax);
        let syntaxes = highlight::syntaxes();
        for (id, file, lang, set) in LEXERS.into_iter().filter(|(id, ..)| wanted(id)) {
            let text = read(&args.corpus, file);
            let input = Input {
                id,
                group: "lexer",
                bytes: text.len(),
                version: None,
                hash: generate::fnv1a(text.as_bytes()),
            };
            eprintln!("{id}: {} bytes", text.len());
            let set = Set::new(set);
            emit(
                &input,
                "c0",
                &measure(&counters, n, || ceilings::read(text.as_bytes())),
                "",
            );
            emit(
                &input,
                "c1-lex",
                &measure(&counters, n, || set.classify(text.as_bytes())),
                "",
            );
            let tokens = || {
                let mut count = 0;
                match id {
                    "log" => {
                        let mut spans = Vec::new();
                        text.lines().for_each(|line| {
                            logs::tokens(line, &mut spans);
                            count += spans.len();
                        });
                    }
                    _ => {
                        let mut spans = Vec::new();
                        let mut comment = false;
                        text.lines().for_each(|line| {
                            json::tokens(line, &mut comment, &mut spans);
                            count += spans.len();
                        });
                    }
                }
                count
            };
            emit(&input, "tokens", &measure(&counters, n, tokens), "");
            let s = measure(&counters, n, || {
                highlight::highlight(1, lang, &text, &syntax_theme)
            });
            emit(&input, "highlight", &s, "");
            if let Some(syntax) = highlight::find_syntax(lang) {
                let s = measure_cold(&counters, SYNTECT_SAMPLES.min(n), || {
                    let mut state = ParseState::new(syntax);
                    LinesWithEndings::from(&text)
                        .map(|line| state.parse_line(line, syntaxes).map_or(0, |ops| ops.len()))
                        .sum::<usize>()
                });
                emit(
                    &input,
                    "syntect",
                    &s,
                    &format!(",\"syntax\":\"{}\"", syntax.name),
                );
            }
        }
    }
}
