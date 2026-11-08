//! ada-parse beside logos, Lexilla and KSyntaxHighlighting, in MB of source a second, on the
//! census corpora that lexer_speed reads (`<lists>/<lang>.txt`, one path a line).
//!
//! Each engine reads each file as it is fed in its own editor, prepared outside the timing:
//! - ada: `ada_parse::lex` line by line from the default state, the lines split outside the
//!   timing (as the others' line index and QStrings are; X-051). `ada-split` is the same with
//!   the lines split by `str::lines` inside the timing, as lexer_speed does (before X-051);
//! - logos: the whole file through the DFA, tokens counted;
//! - ladder (Rust only): logos rungs toward ada's work, keywords told by the DFA against looked
//!   up after the name (`logos_ladder.rs`);
//! - Lexilla: the whole file styled from style 0 into a TestDocument (Scintilla's model);
//! - KSyntaxHighlighting: `highlightLine` over the file's lines as QStrings (Kate's model), the
//!   format ranges counted.
//!
//! The corpus is repeated to at least `--mb` MB a round (4); the engines take turns round by
//! round, after one untimed warm-up pass each (the first call, definitions loaded, left out).
//! The best round's wall time is reported, beside that round's thread CPU time. Bytes are the
//! files' UTF-8 bytes for every engine.
//!
//! ```zsh
//! bench/work.zsh build "peers build" cargo build --release --manifest-path bench/peers/Cargo.toml
//! bench/work.zsh bench --cpus 2 "peers" bench/peers/target/release/ada-peers
//! ```

mod logos_java;
mod logos_json;
mod logos_ladder;
mod logos_rust;
mod strip;

use std::ffi::{CString, c_char, c_void};
use std::time::{Duration, Instant};

use ada_parse::{Lang, State, Span, lex};
use logos::Logos;

unsafe extern "C" {
    fn lexilla_new(lang: i32) -> *mut c_void;
    fn lexilla_free(lexer: *mut c_void);
    fn lexilla_doc_new(text: *const c_char, len: usize) -> *mut c_void;
    fn lexilla_doc_free(doc: *mut c_void);
    fn lexilla_lex(lexer: *mut c_void, doc: *mut c_void);
    fn lexilla_runs(doc: *mut c_void) -> usize;

    fn ksh_new(name: *const c_char) -> *mut c_void;
    fn ksh_free(hl: *mut c_void);
    fn ksh_doc_new(text: *const c_char, len: usize) -> *mut c_void;
    fn ksh_doc_free(doc: *mut c_void);
    fn ksh_lex(hl: *mut c_void, doc: *mut c_void);
    fn ksh_take_formats(hl: *mut c_void) -> u64;
}

struct LangSpec {
    name: &'static str,
    ada: Lang,
    lexilla: i32,
    ksh: &'static str,
    logos: Option<fn(&str) -> (u64, u64)>,
}

const LANGS: &[LangSpec] = &[
    LangSpec { name: "json", ada: Lang::Json, lexilla: 0, ksh: "JSON", logos: Some(logos_json) },
    LangSpec { name: "rust", ada: Lang::Rust, lexilla: 1, ksh: "Rust", logos: Some(logos_rust) },
    LangSpec { name: "python", ada: Lang::Python, lexilla: 2, ksh: "Python", logos: None },
    LangSpec { name: "bash", ada: Lang::Bash, lexilla: 3, ksh: "Bash", logos: None },
    LangSpec { name: "java", ada: Lang::Java, lexilla: 4, ksh: "Java", logos: Some(logos_java) },
];

fn logos_json(text: &str) -> (u64, u64) {
    let (mut ok, mut err) = (0, 0);
    for t in logos_json::Json::lexer(text) {
        if t.is_ok() { ok += 1 } else { err += 1 }
    }
    (ok, err)
}

fn logos_rust(text: &str) -> (u64, u64) {
    let (mut ok, mut err) = (0, 0);
    for t in logos_rust::Rust::lexer(text) {
        if t.is_ok() { ok += 1 } else { err += 1 }
    }
    (ok, err)
}

fn logos_java(text: &str) -> (u64, u64) {
    let (mut ok, mut err) = (0, 0);
    for t in logos_java::Java::lexer(text) {
        if t.is_ok() { ok += 1 } else { err += 1 }
    }
    (ok, err)
}

fn thread_cpu() -> Duration {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: a valid timespec to write into
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

/// One engine's state for one language: what a pass runs on, and what it produced.
enum Engine<'a> {
    Ada { lang: Lang, lines: Vec<Vec<&'a str>>, spans: Vec<Span> },
    AdaSplit { lang: Lang, files: &'a [String], spans: Vec<Span> },
    Logos { f: fn(&str) -> (u64, u64), files: &'a [String] },
    Lexilla { lexer: *mut c_void, docs: Vec<*mut c_void> },
    Ksh { hl: *mut c_void, docs: Vec<*mut c_void> },
    Ladder { rung: logos_ladder::Rung, files: &'a [String], lines: Vec<Vec<&'a str>>, spans: Vec<Span> },
}

impl Engine<'_> {
    fn name(&self) -> &'static str {
        match self {
            Engine::Ada { .. } => "ada-parse",
            Engine::AdaSplit { .. } => "ada-parse, split in",
            Engine::Logos { .. } => "logos",
            Engine::Lexilla { .. } => "Lexilla",
            Engine::Ksh { .. } => "KSyntaxHighlighting",
            Engine::Ladder { rung, .. } => rung.name(),
        }
    }

    /// One pass over the corpus; returns (output units, errors): spans, tokens, format ranges
    /// (Lexilla's style runs are counted after the timing, so 0 here).
    fn pass(&mut self) -> (u64, u64) {
        match self {
            Engine::Ada { lang, lines, spans } => {
                let mut n = 0u64;
                for file in lines.iter() {
                    let mut state = State::default();
                    for line in file {
                        lex(*lang, line, &mut state, spans);
                        n += spans.len() as u64;
                    }
                }
                (n, 0)
            }
            Engine::AdaSplit { lang, files, spans } => {
                let mut n = 0u64;
                for text in files.iter() {
                    let mut state = State::default();
                    for line in text.lines() {
                        lex(*lang, line, &mut state, spans);
                        n += spans.len() as u64;
                    }
                }
                (n, 0)
            }
            Engine::Logos { f, files } => files.iter().fold((0, 0), |(a, b), t| {
                let (x, y) = f(t);
                (a + x, b + y)
            }),
            Engine::Lexilla { lexer, docs } => {
                for &d in docs.iter() {
                    // SAFETY: lexer and docs come from the shim and live until drop
                    unsafe { lexilla_lex(*lexer, d) };
                }
                (0, 0)
            }
            Engine::Ladder { rung, files, lines, spans } => {
                files.iter().zip(lines.iter()).fold((0, 0), |(a, b), (t, l)| {
                    let (x, y) = logos_ladder::pass(*rung, t, l, spans);
                    (a + x, b + y)
                })
            }
            Engine::Ksh { hl, docs } => {
                for &d in docs.iter() {
                    // SAFETY: as above
                    unsafe { ksh_lex(*hl, d) };
                }
                // SAFETY: as above
                (unsafe { ksh_take_formats(*hl) }, 0)
            }
        }
    }

    fn output_after(&self, counted: u64) -> u64 {
        match self {
            // SAFETY: as above
            Engine::Lexilla { docs, .. } => docs.iter().map(|&d| unsafe { lexilla_runs(d) } as u64).sum(),
            _ => counted,
        }
    }
}

impl Drop for Engine<'_> {
    fn drop(&mut self) {
        // SAFETY: each pointer was made by the shim and is freed once
        unsafe {
            match self {
                Engine::Lexilla { lexer, docs } => {
                    docs.iter().for_each(|&d| lexilla_doc_free(d));
                    lexilla_free(*lexer);
                }
                Engine::Ksh { hl, docs } => {
                    docs.iter().for_each(|&d| ksh_doc_free(d));
                    ksh_free(*hl);
                }
                _ => {}
            }
        }
    }
}

struct Opts {
    lists: String,
    only: Option<String>,
    engines: Option<Vec<String>>,
    rounds: usize,
    mb: usize,
    strip: strip::Strip,
}

fn opts() -> Opts {
    let mut o = Opts {
        lists: std::env::var("ADA_PARSE_LISTS").unwrap_or_else(|_| {
            concat!(env!("CARGO_MANIFEST_DIR"), "/../b2/root/build/syntect-probe/lists").into()
        }),
        only: None,
        engines: None,
        rounds: 5,
        mb: 4,
        strip: strip::Strip { comments: false, strings: false },
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut v = || args.next().unwrap_or_else(|| panic!("{a} wants a value"));
        match a.as_str() {
            "--lists" => o.lists = v(),
            "--only" => o.only = Some(v()),
            "--engines" => o.engines = Some(v().split(',').map(str::to_lowercase).collect()),
            "--rounds" => o.rounds = v().parse().expect("--rounds N"),
            "--mb" => o.mb = v().parse().expect("--mb N"),
            "--strip" => {
                for w in v().split(',') {
                    match w {
                        "comments" => o.strip.comments = true,
                        "strings" => o.strip.strings = true,
                        _ => panic!("--strip comments[,strings]"),
                    }
                }
            }
            _ => panic!("usage: ada-peers [--lists DIR] [--only LANG] [--engines ada,ada-split,logos,ladder,lookup,lexilla,ksh] [--rounds N] [--mb N] [--strip comments[,strings]]"),
        }
    }
    o
}

fn wanted(o: &Opts, key: &str) -> bool {
    o.engines.as_ref().is_none_or(|e| e.iter().any(|x| x == key))
}

fn main() {
    let o = opts();
    println!(
        "{:7} {:20} {:>9} {:>9} {:>8} {:>10} {:>7} {:>6}",
        "lang", "engine", "wall MB/s", "CPU MB/s", "ns/line", "out/pass", "errors", "× ada"
    );
    for spec in LANGS {
        if o.only.as_deref().is_some_and(|x| x != spec.name) {
            continue;
        }
        let Ok(list) = std::fs::read_to_string(format!("{}/{}.txt", o.lists, spec.name)) else {
            eprintln!("{}: no list in {}", spec.name, o.lists);
            continue;
        };
        let files: Vec<String> =
            list.lines().filter_map(|p| std::fs::read_to_string(p).ok()).collect();
        let files: Vec<String> = if o.strip.comments || o.strip.strings {
            assert!(spec.name == "java", "--strip reads Java's comments and strings only");
            files.iter().map(|t| strip::strip(t, &o.strip)).collect()
        } else {
            files
        };
        let bytes: usize = files.iter().map(String::len).sum();
        let lines: usize = files.iter().map(|f| f.lines().count()).sum();
        if bytes == 0 {
            continue;
        }
        let repeat = (o.mb * 1_000_000 / bytes).max(1);

        let mut engines: Vec<Engine> = Vec::new();
        if wanted(&o, "ada") {
            let lines = files.iter().map(|t| t.lines().collect()).collect();
            engines.push(Engine::Ada { lang: spec.ada, lines, spans: Vec::new() });
        }
        if wanted(&o, "ada-split") {
            engines.push(Engine::AdaSplit { lang: spec.ada, files: &files, spans: Vec::new() });
        }
        if let (Some(f), true) = (spec.logos, wanted(&o, "logos")) {
            engines.push(Engine::Logos { f, files: &files });
        }
        if spec.name == "rust" && (wanted(&o, "ladder") || wanted(&o, "lookup")) {
            let split: Vec<Vec<&str>> = files.iter().map(|t| t.lines().collect()).collect();
            match split.iter().try_fold(0, |n, l| logos_ladder::same_leaves(l).map(|w| n + w)) {
                Ok(w) => eprintln!("ladder: told and looked-up leaves agree, {w} words told a pass"),
                Err(e) => panic!("ladder: told and looked-up leaves differ at {e}"),
            }
            let rungs: &[logos_ladder::Rung] =
                if wanted(&o, "ladder") { &logos_ladder::Rung::ALL } else { &[logos_ladder::Rung::LookedUp] };
            for &rung in rungs {
                engines.push(Engine::Ladder { rung, files: &files, lines: split.clone(), spans: Vec::new() });
            }
        }
        if wanted(&o, "lexilla") {
            // SAFETY: the shim copies each text
            let lexer = unsafe { lexilla_new(spec.lexilla) };
            assert!(!lexer.is_null());
            let docs = files.iter().map(|t| unsafe { lexilla_doc_new(t.as_ptr().cast(), t.len()) }).collect();
            engines.push(Engine::Lexilla { lexer, docs });
        }
        if wanted(&o, "ksh") {
            let name = CString::new(spec.ksh).unwrap();
            // SAFETY: a NUL-terminated name; the shim copies each text
            let hl = unsafe { ksh_new(name.as_ptr()) };
            if hl.is_null() {
                eprintln!("{}: KSyntaxHighlighting has no definition {}", spec.name, spec.ksh);
            } else {
                let docs = files.iter().map(|t| unsafe { ksh_doc_new(t.as_ptr().cast(), t.len()) }).collect();
                engines.push(Engine::Ksh { hl, docs });
            }
        }

        // the warm-up pass, and what one pass produces
        let mut out: Vec<(u64, u64)> = engines.iter_mut().map(Engine::pass).collect();
        let mut best = vec![(Duration::MAX, Duration::ZERO); engines.len()];
        for _ in 0..o.rounds {
            for (i, e) in engines.iter_mut().enumerate() {
                let (t, c) = (Instant::now(), thread_cpu());
                let mut last = (0, 0);
                for _ in 0..repeat {
                    last = e.pass();
                }
                let (wall, cpu) = (t.elapsed(), thread_cpu() - c);
                if wall < best[i].0 {
                    best[i] = (wall, cpu);
                }
                out[i] = last;
            }
        }

        let mbs = |d: Duration| bytes as f64 * repeat as f64 / d.as_secs_f64() / 1e6;
        let ada_wall = engines.iter().position(|e| matches!(e, Engine::Ada { .. })).map(|i| best[i].0);
        for (i, e) in engines.iter().enumerate() {
            let (wall, cpu) = best[i];
            let ratio = ada_wall.map_or(String::new(), |a| format!("{:.2}", a.as_secs_f64() / wall.as_secs_f64()));
            println!(
                "{:7} {:20} {:9.1} {:9.1} {:8.0} {:>10} {:>7} {:>6}",
                spec.name,
                e.name(),
                mbs(wall),
                mbs(cpu),
                wall.as_secs_f64() / repeat as f64 * 1e9 / lines as f64,
                e.output_after(out[i].0),
                out[i].1,
                ratio,
            );
        }
        println!(
            "{:7} corpus: {} files, {bytes} bytes, {lines} lines; {repeat} passes a round, best of {}",
            "",
            files.len(),
            o.rounds
        );
    }
}
