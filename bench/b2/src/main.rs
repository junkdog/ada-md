//! B2: multi-facet interactive benchmark for terminal markdown viewers, in a pty.
//!
//! usage:
//!   b2 run SPEC.json OUT.jsonl [NAME...]     run the jobs (only those whose name contains a NAME, if given)
//!   b2 floor [SAMPLES] [OUT.json]            the harness's own key -> pty -> timestamp floor
//!   b2 probe CWD KEY... -- CMD...            dump the screen after each key (keys take \e \r \n \t \xNN escapes,
//!                                            and SLEEP<secs> waits)
//!   b2 flood CWD RATE SECS DOWN UP N -- CMD...
//!                                            scroll at RATE keys/s (0: as fast as the pty takes them) for SECS,
//!                                            N x DOWN then N x UP; one JSON line (see src/flood.rs)
//!
//! SPEC is a list of jobs (see mkspec.py):
//!   {"name": "ada-l", "cmd": [...], "env": {...}, "cwd": ".../corpus", "file": "l.md", "runs": 5,
//!    "first": "Successor",            // word on the first screen
//!    "scenarios": ["paint", "idle", "line", "page", "end", "resize", "search", "burst", "reload"],
//!    "keys": {"line": "j", "page": "\u001b[6~", "end": "G", "home": "g", "quit": "q",
//!             "search": ["/", "TERM", "\r"], "search_reset": []},
//!    "end_sentinel": "word", "search_term": "word", "search_sentinel": "word", "reload_from": "word"}
//!
//! A reader thread owns the pty master's read side: it timestamps every chunk (CLOCK_MONOTONIC) and answers
//! terminal queries the way a non-kitty truecolor terminal would. The main thread only writes input and records
//! when. Afterwards every chunk is replayed through vt100 to find when the screen first showed each sentinel,
//! so the emulator's cost never lands inside a measurement.

mod flood;
mod pty;
mod replay;

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::mpsc::channel;

use serde::Deserialize;
use serde_json::{Map, Value, json};

use pty::{COLS, ROWS, Session, now};
use replay::Replay;

#[derive(Deserialize)]
struct Keys {
    #[serde(default)]
    setup: Vec<String>,
    line: String,
    page: String,
    end: String,
    home: String,
    #[serde(default = "quit")]
    quit: String,
    search: Option<Vec<String>>,
    #[serde(default)]
    search_reset: Vec<String>,
}

fn quit() -> String {
    "q".into()
}

#[derive(Deserialize)]
struct Job {
    name: String,
    cmd: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    cwd: String,
    file: String,
    #[serde(default = "five")]
    runs: usize,
    first: String,
    scenarios: Vec<String>,
    keys: Keys,
    #[serde(default)]
    end_sentinel: String,
    #[serde(default)]
    search_term: String,
    #[serde(default)]
    search_sentinel: String,
    #[serde(default)]
    reload_from: String,
    #[serde(default = "twenty")]
    paint_cap: f64,
    #[serde(default = "three")]
    idle_runs: usize,
    #[serde(default = "twenty_steps")]
    steps: usize,
    #[serde(default = "five")]
    reloads: usize,
}

fn five() -> usize {
    5
}
fn three() -> usize {
    3
}
fn twenty_steps() -> usize {
    20
}
fn twenty() -> f64 {
    20.0
}

impl Job {
    fn has(&self, scenario: &str) -> bool {
        self.scenarios.iter().any(|s| s == scenario)
    }
}

fn base_env() -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = std::env::vars().collect();
    env.insert("TERM".into(), "xterm-256color".into());
    env.insert("COLORTERM".into(), "truecolor".into());
    for k in [
        "TMUX",
        "KITTY_WINDOW_ID",
        "TERM_PROGRAM",
        "WEZTERM_EXECUTABLE",
        "VTE_VERSION",
        "WT_SESSION",
    ] {
        env.remove(k);
    }
    env
}

/// The root-owned script that drops the page cache and compacts memory, so transparent huge pages find free
/// 2 MiB blocks instead of falling back to 4 KiB pages. It is oxasm's (`xtask/scripts/oxasm-drop-caches`, which
/// says how to install it and the sudoers rule that runs it without a password); `B2_DROP_CACHES` names another,
/// or `off` skips it.
const DROP_CACHES: &str = "/usr/local/sbin/oxasm-drop-caches";

/// Drop only below this much free memory (GiB; `B2_DROP_BELOW_GB` sets another): oxasm and sift lost speed with
/// a couple of GB free. `MemFree`, not `MemAvailable`, which counts the page cache as available and so stays high
/// when the cache has taken the memory.
const DROP_BELOW_GB: f64 = 8.0;

/// Drop the page cache and compact memory before a measurement when free memory is low; its before/after report
/// goes to stderr, so stdout stays one JSON line.
pub fn drop_caches() -> io::Result<()> {
    let script = std::env::var("B2_DROP_CACHES").unwrap_or_else(|_| DROP_CACHES.into());
    let below: f64 = match std::env::var("B2_DROP_BELOW_GB") {
        Ok(gb) => gb
            .parse()
            .map_err(|_| io::Error::other(format!("B2_DROP_BELOW_GB={gb} is not a number")))?,
        Err(_) => DROP_BELOW_GB,
    };
    if script == "off" || mem_free_kib().is_some_and(|kib| kib as f64 >= below * 1024.0 * 1024.0) {
        return Ok(());
    }
    let out = std::process::Command::new("sudo")
        .args(["-n", &script])
        .stderr(std::process::Stdio::inherit())
        .output()?;
    if !out.status.success() {
        return Err(io::Error::other(format!(
            "`sudo -n {script}` exited with {}; install it as oxasm's xtask/scripts/oxasm-drop-caches says, \
             or set B2_DROP_CACHES=off",
            out.status
        )));
    }
    io::stderr().write_all(&out.stdout)
}

/// `MemFree` from /proc/meminfo, in KiB; `None` where there is none, which drops.
fn mem_free_kib() -> Option<u64> {
    fs::read_to_string("/proc/meminfo")
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("MemFree:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn run_job(job: &Job, out: &mut File) -> io::Result<()> {
    let mut env = base_env();
    env.extend(job.env.clone());
    // every job gets its own copy of its document: the reload test rewrites it, and lanes run in parallel
    let src = Path::new(&job.cwd);
    let cwd = src.parent().unwrap_or(src).join("jobs").join(&job.name);
    fs::create_dir_all(&cwd)?;
    let path = cwd.join(&job.file);
    let original = fs::read(src.join(&job.file))?;
    fs::write(&path, &original)?;
    let cwd = cwd.to_string_lossy().into_owned();
    for run in 0..job.runs {
        let mut rec = Map::new();
        rec.insert("name".into(), json!(job.name));
        rec.insert("run".into(), json!(run));
        let mut s = Session::spawn(&job.cmd, &env, &cwd)?;
        let result = script(job, run, &mut s, &mut rec, &path, &original);
        s.close(&job.keys.quit);
        fs::write(&path, &original)?;
        if let Err(e) = result {
            rec.insert("error".into(), json!(e.to_string()));
        }
        analyze(job, &s, &mut rec);
        writeln!(out, "{}", Value::Object(rec.clone()))?;
        out.flush()?;
        let brief: Map<String, Value> = rec
            .into_iter()
            .filter(|(_, v)| !v.is_array() && !v.is_object())
            .map(|(k, v)| match v.as_f64() {
                Some(f) if v.is_f64() => (k, json!((f * 10.0).round() / 10.0)),
                _ => (k, v),
            })
            .collect();
        eprintln!("{}", Value::Object(brief));
    }
    Ok(())
}

/// One run's interactions; the timings come from `analyze` afterwards.
fn script(
    job: &Job,
    run: usize,
    s: &mut Session,
    rec: &mut Map<String, Value>,
    path: &Path,
    original: &[u8],
) -> io::Result<()> {
    let keys = &job.keys;
    // paint: wait for the first screen, then for it to hold still for 0.6 s (as B1b)
    let settled = s.wait_stable(0.6, job.paint_cap, Some(&job.first));
    rec.insert("settle_capped".into(), json!(!settled));
    rec.insert("paint_bytes".into(), json!(s.bytes()));
    for k in &keys.setup {
        // e.g. treemd: Tab moves focus from the outline to the content
        s.send(k, "setup");
        s.wait_stable(0.3, 3.0, None);
    }
    match s.hwm_kib() {
        Ok(kib) => rec.insert("hwm_paint_kib".into(), json!(kib)),
        Err(e) => {
            rec.insert("died".into(), json!(true));
            return Err(e);
        }
    };
    if job.has("idle") && run < job.idle_runs {
        let (t1, c1, b1) = (s.cpu_ms()?, s.ctxsw()?, s.bytes());
        let end = s.mark("idle") + 5.0;
        while now() < end {
            s.pump(end - now());
        }
        rec.insert("idle_cpu_ms".into(), json!(s.cpu_ms()? - t1));
        rec.insert("idle_ctxsw_s".into(), json!((s.ctxsw()? - c1) as f64 / 5.0));
        rec.insert("idle_bytes".into(), json!(s.bytes() - b1));
    }
    for (kind, key) in [("line", &keys.line), ("page", &keys.page)] {
        if job.has(kind) {
            let t1 = s.cpu_ms()?;
            for i in 0..job.steps {
                s.send(key, format!("{kind}:{i}"));
                s.wait_stable(0.15, 2.0, None);
            }
            rec.insert(format!("{kind}_cpu_ms"), json!(s.cpu_ms()? - t1));
            s.send(&keys.home, "home");
            s.wait_stable(0.3, 5.0, None);
        }
    }
    if job.has("end") {
        for i in 0..3 {
            s.send(&keys.end, format!("end:{i}"));
            s.wait_stable(0.3, 5.0, Some(&job.end_sentinel));
            s.send(&keys.home, format!("endhome:{i}"));
            s.wait_stable(0.3, 5.0, Some(&job.first));
        }
    }
    if let (true, Some([opener, rest @ ..])) = (job.has("search"), keys.search.as_deref()) {
        for i in 0..3 {
            s.send(opener, "type");
            s.wait_stable(0.15, 2.0, None);
            // the term and Enter in one write, as a paste: incremental and on-Enter search pay alike
            s.send(
                &rest.concat().replace("TERM", &job.search_term),
                format!("search:{i}"),
            );
            s.wait_stable(0.4, 5.0, Some(&job.search_sentinel));
            for k in &keys.search_reset {
                s.send(k, "reset");
                s.wait_stable(0.15, 2.0, None);
            }
            s.send(&keys.home, "home");
            s.wait_stable(0.4, 5.0, Some(&job.first));
        }
    }
    if job.has("burst") {
        // key repeat: 60 page-downs at 30 Hz, then wait for the screen to catch up
        let (t1, b1) = (s.cpu_ms()?, s.bytes());
        for i in 0..60 {
            let next = s.send(&keys.page, format!("burst:{i}")) + 1.0 / 30.0;
            while now() < next {
                s.pump(next - now());
            }
        }
        s.mark("burst:last");
        s.wait_stable(0.5, 60.0, None);
        rec.insert("burst_cpu_ms".into(), json!(s.cpu_ms()? - t1));
        rec.insert("burst_bytes".into(), json!(s.bytes() - b1));
        s.send(&keys.home, "home");
        s.wait_stable(0.4, 10.0, Some(&job.first));
    }
    if job.has("reload") {
        let from = job.reload_from.as_bytes();
        for i in 0..job.reloads {
            let word = format!("Zqreload{run}x{i}");
            let data = replace_first(original, from, word.as_bytes());
            let t_w = now() + 0.23 * i as f64; // spread the writes across a 1 s poll period
            while now() < t_w {
                s.pump(t_w - now());
            }
            fs::write(path, data)?; // in place, like `>`
            s.mark(format!("reload:{i}:{word}"));
            s.wait_stable(0.3, 8.0, Some(&word));
        }
        fs::write(path, original)?;
    }
    if job.has("resize") {
        for i in 0..5 {
            s.resize(ROWS, 90, format!("resize:narrow:{i}"));
            s.wait_stable(0.3, 5.0, None);
            s.resize(ROWS, COLS, format!("resize:wide:{i}"));
            s.wait_stable(0.3, 5.0, None);
        }
    }
    if let Ok(kib) = s.hwm_kib() {
        rec.insert("hwm_kib".into(), json!(kib));
    }
    if let Ok(kib) = s.anon_huge_kib() {
        rec.insert("anon_huge_kib".into(), json!(kib));
    }
    s.mark("done");
    Ok(())
}

fn replace_first(hay: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    match hay.windows(from.len().max(1)).position(|w| w == from) {
        Some(j) if !from.is_empty() => [&hay[..j], to, &hay[j + from.len()..]].concat(),
        _ => hay.to_vec(),
    }
}

fn analyze(job: &Job, s: &Session, rec: &mut Map<String, Value>) {
    let (ch, marks) = (&s.chunks, &s.marks);
    let mut rp = Replay::new();
    let pre: Vec<f64> = marks
        .iter()
        .map(|(t, _)| *t)
        .chain([f64::INFINITY])
        .collect();
    let p = rp.scan(ch, s.t0, pre[0], Some(&job.first));
    // launch until settled, plus the 0.6 s still window
    rec.insert(
        "paint_cpu_ms".into(),
        json!(s.mark_cpu.first().copied().flatten()),
    );
    rec.insert("total_cpu_ms".into(), json!(s.total_cpu_ms));
    rec.insert("total_wall_ms".into(), json!(s.total_wall_ms));
    rec.insert("first_ms".into(), json!(p.shown));
    rec.insert("settle_ms".into(), json!(p.done));
    let mut lat = Map::new();
    let mut push = |k: String, v: Value| {
        lat.entry(k)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .unwrap()
            .push(v);
    };
    let mut screens = Map::new();
    let debug = std::env::var_os("B2_DEBUG").is_some();
    for (idx, (t, label)) in marks.iter().enumerate() {
        let (t, t_next) = (*t, pre[idx + 1]);
        let kind = label.split(':').next().unwrap_or("");
        if label.starts_with("resize:") {
            rp.advance_to(ch, t);
            rp.resize(ROWS, if label.contains(":narrow:") { 90 } else { COLS });
        }
        let needle = match kind {
            "end" => Some(job.end_sentinel.as_str()),
            "search" => Some(job.search_sentinel.as_str()),
            "endhome" => Some(job.first.as_str()),
            "reload" => label.rsplit(':').next(),
            _ => None,
        };
        if [
            "line", "page", "resize", "end", "search", "reload", "endhome",
        ]
        .contains(&kind)
        {
            let r = rp.scan(ch, t, t_next, needle);
            push(format!("{kind}_first"), json!(r.first));
            push(format!("{kind}_done"), json!(r.done));
            push(format!("{kind}_bytes"), json!(r.bytes));
            push(format!("{kind}_changed"), json!(r.changed));
            push(format!("{kind}_shown"), json!(r.shown));
            let c0 = s.mark_cpu[idx];
            let c1 = s.mark_cpu.get(idx + 1).copied().flatten();
            push(format!("{kind}_cpu"), json!(c0.zip(c1).map(|(a, b)| b - a)));
        } else if label == "burst:last" {
            let first_burst = marks
                .iter()
                .find(|(_, l)| l == "burst:0")
                .map_or(t, |(tt, _)| *tt);
            let r = rp.scan(ch, t, t_next, None);
            rec.insert("burst_lag_ms".into(), json!(r.done.unwrap_or(0.0)));
            rec.insert("burst_span_ms".into(), json!((t - first_burst) * 1e3));
        }
        if debug && !screens.contains_key(kind) && kind != "type" && kind != "reset" {
            rp.advance_to(ch, t_next);
            screens.insert(kind.into(), json!(rp.text()));
        }
    }
    rp.advance_to(ch, f64::INFINITY);
    if debug {
        rec.insert("screens".into(), Value::Object(screens));
    }
    rec.extend(lat);
    rec.insert(
        "final_screen_sample".into(),
        json!(rp.text().chars().take(400).collect::<String>()),
    );
}

fn cmd_run(args: &[String]) -> io::Result<()> {
    let [spec, out, only @ ..] = args else {
        return Err(io::Error::other(
            "usage: b2 run SPEC.json OUT.jsonl [NAME...]",
        ));
    };
    let jobs: Vec<Job> = serde_json::from_slice(&fs::read(spec)?).map_err(io::Error::other)?;
    let mut out = File::options().create(true).append(true).open(out)?;
    // Once, not before every run: paint includes startup, and a dropped cache would make each a cold start.
    // When it drops, the first run of each viewer is cold; the report's median across runs absorbs it, and also
    // the other lanes' drops, which start together with this one.
    drop_caches()?;
    for job in &jobs {
        if only.is_empty() || only.iter().any(|o| job.name.contains(o.as_str())) {
            run_job(job, &mut out)?;
        }
    }
    Ok(())
}

/// The harness's own floor: a key written to the master, the line discipline's echo, the reader thread's
/// timestamp. No program is involved; a thread drains the slave so its line buffer never fills.
fn cmd_floor(args: &[String]) -> io::Result<()> {
    let n: usize = args
        .first()
        .map_or(Ok(2000), |a| a.parse())
        .map_err(io::Error::other)?;
    let (master, mut slave) = pty::open_pty(ROWS, COLS)?;
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while slave.read(&mut buf).is_ok_and(|n| n > 0) {}
    });
    let (tx, rx) = channel();
    let rm = master.try_clone()?;
    std::thread::spawn(move || pty::reader(rm, tx));
    let mut seed = now().to_bits() | 1;
    let mut lat = Vec::with_capacity(n);
    for _ in 0..n {
        // 2-20 ms apart, like keys from a script
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let gap = 0.002 + (seed % 18_000) as f64 * 1e-6;
        let until = now() + gap;
        while let Ok(_stale) =
            rx.recv_timeout(std::time::Duration::from_secs_f64((until - now()).max(0.0)))
        {}
        let t = now();
        (&master).write_all(b"a\n")?;
        let (tc, _) = rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .map_err(io::Error::other)?;
        lat.push((tc - t) * 1e3);
    }
    let mut sorted = lat.clone();
    sorted.sort_by(f64::total_cmp);
    let q = |p: f64| sorted[((sorted.len() - 1) as f64 * p).round() as usize];
    let summary = json!({"samples": n, "median_ms": q(0.5), "p90_ms": q(0.9), "p99_ms": q(0.99),
                         "min_ms": sorted[0], "max_ms": sorted[n - 1]});
    println!("{summary}");
    if let Some(out) = args.get(1) {
        fs::write(
            out,
            json!({"summary": summary, "samples_ms": lat}).to_string(),
        )?;
    }
    Ok(())
}

fn cmd_probe(args: &[String]) -> io::Result<()> {
    let sep = args.iter().position(|a| a == "--");
    let (Some(sep), Some(cwd)) = (sep, args.first()) else {
        return Err(io::Error::other("usage: b2 probe CWD KEY... -- CMD..."));
    };
    let (keys, cmd) = (&args[1..sep], &args[sep + 1..]);
    let mut s = Session::spawn(cmd, &base_env(), cwd)?;
    s.wait_quiet(0.6, 20.0);
    let mut rp = Replay::new();
    let mut dump = |s: &Session, label: &str| {
        rp.advance_to(&s.chunks, f64::INFINITY);
        println!("=== after {label:?}");
        println!(
            "{}",
            rp.text()
                .lines()
                .map(str::trim_end)
                .collect::<Vec<_>>()
                .join("\n")
        );
    };
    dump(&s, "start");
    for k in keys {
        if let Some(secs) = k.strip_prefix("SLEEP").and_then(|x| x.parse::<f64>().ok()) {
            s.wait_quiet(secs, secs);
            continue;
        }
        let k = unescape(k);
        s.send(&k, "k");
        s.wait_quiet(0.4, 5.0);
        dump(&s, &k);
    }
    s.close("q");
    Ok(())
}

/// `\e`, `\r`, `\n`, `\t`, `\\`, `\xNN` and `\uNNNN`.
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let hex = |it: &mut std::str::Chars, n: usize| {
            let h: String = it.by_ref().take(n).collect();
            u32::from_str_radix(&h, 16)
                .ok()
                .and_then(char::from_u32)
                .unwrap_or('?')
        };
        match it.next() {
            Some('e') => out.push('\x1b'),
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('x') => out.push(hex(&mut it, 2)),
            Some('u') => out.push(hex(&mut it, 4)),
            Some(o) => out.push(o),
            None => out.push('\\'),
        }
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or_default();
    let result = match args.first().map(String::as_str) {
        Some("run") => cmd_run(rest),
        Some("floor") => cmd_floor(rest),
        Some("probe") => cmd_probe(rest),
        Some("flood") => flood::cmd_flood(rest),
        _ => Err(io::Error::other(
            "usage: b2 run|floor|probe|flood ... (see src/main.rs)",
        )),
    };
    if let Err(e) = result {
        eprintln!("b2: {e}");
        std::process::exit(1);
    }
}
