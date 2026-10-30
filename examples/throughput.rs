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
//! taskset -c 2 target/release/examples/throughput ~/b2/corpus --vault ~/opt/birb-vault --only vault
//! # one stage alone, to profile it
//! taskset -c 2 target/release/examples/throughput ~/b2/corpus --only tables --stages layout --samples 100
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
    use ada::layout::{self, LayoutParams, TableMode};
    use ada::theme::Theme;
    use ada::{json, logs, make, print};
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

    /// The markdown inputs: B2's documents, then the generated ones. The vault, when given,
    /// comes after them.
    const MARKDOWN: [(&str, Option<&str>); 18] = [
        ("s", Some("s.md")),
        ("m", Some("m.md")),
        ("l", Some("l.md")),
        ("xl", Some("xl.md")),
        ("prose", None),
        ("unicode", None),
        ("emphasis", None),
        ("code-span", None),
        ("links", None),
        ("footnotes", None),
        ("headings", None),
        ("lists", None),
        ("quotes", None),
        ("tables", None),
        ("table-wide", None),
        ("fences", None),
        ("html", None),
        ("long-lines", None),
    ];

    /// ada's own lexers: id, input file, language, the format's set for C1.
    const LEXERS: [(&str, &str, &str, &[u8]); 6] = [
        ("log", "log.log", "log", ceilings::LOG),
        ("json", "huge.json", "json", ceilings::JSON),
        ("make", "linux.mk", "make", ceilings::MAKE),
        ("make-4m", "kernel-4m.mk", "make", ceilings::MAKE),
        ("bash", "ltmain.sh", "bash", ceilings::BASH),
        ("bash-4m", "scripts-4m.sh", "bash", ceilings::BASH),
    ];

    struct Args {
        corpus: PathBuf,
        vault: Option<PathBuf>,
        only: Option<Vec<String>>,
        /// The stages to run, all when `None`: one alone is what a profile wants.
        stages: Option<Vec<String>>,
        samples: usize,
    }

    impl Args {
        fn stage(&self, stage: &str) -> bool {
            self.stages
                .as_ref()
                .is_none_or(|s| s.iter().any(|w| w == stage))
        }
    }

    fn usage() -> ! {
        eprintln!(
            "usage: throughput CORPUS_DIR [--vault DIR] [--only ID,ID…] [--stages STAGE,…] [--samples N]"
        );
        process::exit(2);
    }

    fn args() -> Args {
        let mut corpus = None;
        let mut vault = None;
        let mut only = None;
        let mut stages = None;
        let mut samples = SAMPLES;
        let mut args = env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--only" => {
                    let ids = args.next().unwrap_or_else(|| usage());
                    only = Some(ids.split(',').map(str::to_owned).collect());
                }
                "--stages" => {
                    let ids = args.next().unwrap_or_else(|| usage());
                    stages = Some(ids.split(',').map(str::to_owned).collect());
                }
                "--vault" => vault = Some(PathBuf::from(args.next().unwrap_or_else(|| usage()))),
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
            vault,
            only,
            stages,
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
        /// How many notes, for the vault.
        notes: Option<usize>,
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
        if let Some(n) = input.notes {
            write!(line, ",\"notes\":{n}").unwrap();
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

    /// Every note under `dir`, `*.md` outside hidden directories, in path order.
    fn notes(dir: &Path) -> Vec<String> {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            let entries = fs::read_dir(dir)
                .unwrap_or_else(|e| fail(&format!("{}: {e}", dir.display())))
                .filter_map(Result::ok);
            for entry in entries {
                let path = entry.path();
                if entry.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                match entry.file_type() {
                    Ok(t) if t.is_dir() => walk(&path, out),
                    Ok(t) if t.is_file() && path.extension().is_some_and(|e| e == "md") => {
                        out.push(path);
                    }
                    _ => {}
                }
            }
        }
        let mut paths = Vec::new();
        walk(dir, &mut paths);
        paths.sort();
        paths
            .iter()
            .map(|p| match fs::read(p) {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(e) => fail(&format!("{}: {e}", p.display())),
            })
            .collect()
    }

    /// What every markdown input's cells share.
    struct Markdown<'a> {
        counters: &'a Counters,
        samples: usize,
        theme: &'a Theme,
        set: &'a Set,
        args: &'a Args,
    }

    impl Markdown<'_> {
        /// Measure `f` as `input`'s `stage` and emit it, unless `--stages` leaves it out.
        fn cell<T>(&self, input: &Input, stage: &str, extra: &str, f: impl FnMut() -> T) {
            if self.args.stage(stage) {
                emit(
                    input,
                    stage,
                    &measure(self.counters, self.samples, f),
                    extra,
                );
            }
        }

        /// An input's cells over each of `texts` in turn, one document or the vault's notes: the
        /// ceilings, pulldown-cmark's HTML renderer, and ada's stages. A call's result is the
        /// results for every text, freed after the sample.
        fn cells(&self, input: &Input, texts: &[String], params: LayoutParams) {
            let theme = self.theme;
            let bytes = || texts.iter().map(String::as_bytes);
            self.cell(input, "c0", "", || {
                bytes().map(ceilings::read).fold(0, u64::wrapping_add)
            });
            self.cell(input, "c1-parse", "", || {
                bytes()
                    .map(|b| self.set.classify(b))
                    .fold((0, 0), |a, c| (a.0 + c.0, a.1 + c.1))
            });
            self.cell(input, "c2", "", || {
                texts.iter().map(|t| ceilings::events(t)).sum::<usize>()
            });
            self.cell(input, "html", "", || {
                texts.iter().map(|t| ceilings::html(t)).collect::<Vec<_>>()
            });
            self.cell(input, "parse", "", || {
                texts.iter().map(|t| doc::parse(t)).collect::<Vec<_>>()
            });
            if !["c1-layout", "layout", "c1-print", "print"]
                .iter()
                .any(|s| self.args.stage(s))
            {
                return;
            }

            let docs: Vec<Doc> = texts.iter().map(|t| doc::parse(t)).collect();
            let runs: Vec<&str> = docs.iter().flat_map(ceilings::text_of).collect();
            let words = ceilings::Text::new(&runs);
            let extra = format!(",\"text_bytes\":{}", words.len());
            self.cell(input, "c1-layout", &extra, || ceilings::widths(&words));
            let lay = || {
                docs.iter()
                    .map(|d| layout::layout(d, &theme.glyphs, params))
                    .collect::<Vec<_>>()
            };
            if self.args.stage("layout") {
                let rows: usize = lay().iter().map(|l| l.rows.len()).sum();
                let extra = format!(",\"rows\":{rows}");
                self.cell(input, "layout", &extra, lay);
            }
            if !self.args.stage("c1-print") && !self.args.stage("print") {
                return;
            }

            let mut docs: Vec<Doc> = texts.iter().map(|t| doc::parse(t)).collect();
            docs.iter_mut().for_each(plain);
            let render = || {
                docs.iter()
                    .map(|d| print::render(d, theme, params, print::Options::default()))
                    .collect::<Vec<_>>()
            };
            let out = render().concat();
            let extra = format!(",\"out_bytes\":{}", out.len());
            let mut into = vec![0u8; out.len()];
            self.cell(input, "c1-print", &extra, || {
                ceilings::write(out.as_bytes(), &mut into);
            });
            // `render` ends with `String::from_utf8_lossy` over its output, which a print to stdout
            // doesn't: UTF-8 validation and a copy, 6-7% of the stage (BEN-010)
            self.cell(input, "print", &extra, render);
        }
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
        let set = Set::new(ceilings::MARKDOWN);
        let markdown = Markdown {
            counters: &counters,
            samples: n,
            theme: &theme,
            set: &set,
            args: &args,
        };

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
                notes: None,
                hash: generate::fnv1a(text.as_bytes()),
            };
            eprintln!("{id}: {} bytes", text.len());
            // the distribution under pressure: a grid however wide, not record cards
            let params = match id {
                "table-wide" => LayoutParams {
                    table_mode: TableMode::Grid,
                    ..params
                },
                _ => params,
            };
            markdown.cells(&input, &[text], params);
        }

        if let Some(dir) = args.vault.as_deref().filter(|_| wanted("vault")) {
            let texts = notes(dir);
            let all = texts.concat();
            let input = Input {
                id: "vault",
                group: "whole",
                bytes: all.len(),
                version: None,
                notes: Some(texts.len()),
                hash: generate::fnv1a(all.as_bytes()),
            };
            eprintln!("vault: {} notes, {} bytes", texts.len(), all.len());
            markdown.cells(&input, &texts, params);
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
                notes: None,
                hash: generate::fnv1a(text.as_bytes()),
            };
            eprintln!("{id}: {} bytes", text.len());
            let set = Set::new(set);
            if args.stage("c0") {
                let s = measure(&counters, n, || ceilings::read(text.as_bytes()));
                emit(&input, "c0", &s, "");
            }
            if args.stage("c1-lex") {
                let s = measure(&counters, n, || set.classify(text.as_bytes()));
                emit(&input, "c1-lex", &s, "");
            }
            let tokens = || {
                let mut count = 0;
                match lang {
                    "log" => {
                        let mut spans = Vec::new();
                        text.lines().for_each(|line| {
                            logs::tokens(line, &mut spans);
                            count += spans.len();
                        });
                    }
                    "json" => {
                        let mut spans = Vec::new();
                        let mut comment = false;
                        text.lines().for_each(|line| {
                            json::tokens(line, &mut comment, &mut spans);
                            count += spans.len();
                        });
                    }
                    _ => {
                        let grammar = match lang {
                            "make" => make::Grammar::Make,
                            _ => make::Grammar::Bash,
                        };
                        let mut state = make::State::new(grammar);
                        let mut spans = Vec::new();
                        text.lines().for_each(|line| {
                            make::tokens(line, &mut state, &mut spans);
                            count += spans.len();
                        });
                    }
                }
                count
            };
            if args.stage("tokens") {
                emit(&input, "tokens", &measure(&counters, n, tokens), "");
            }
            if args.stage("highlight") {
                let s = measure(&counters, n, || {
                    highlight::highlight(1, lang, &text, &syntax_theme)
                });
                emit(&input, "highlight", &s, "");
            }
            if let Some(syntax) = highlight::find_syntax(lang).filter(|_| args.stage("syntect")) {
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
