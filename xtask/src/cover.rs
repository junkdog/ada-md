//! `cargo xtask cover CONTROL...`: the inputs of a corpus that reach all of its features, from the
//! merge control files libFuzzer's replay wrote (F-020; `fuzz/cover.zsh`, which replays a corpus
//! 12 shards at a time, each shard's inner merge process, `-merge_inner=2`, writing every feature
//! of each input to its own file). The picked inputs go to stdout one per line, best first; a
//! summary goes to stderr.
//!
//! The pick is libFuzzer's `-set_cover_merge=1` (`Merger::SetCoverMerge`): greedy, the input that
//! adds the most features not yet covered first, the smaller of two that add as many, with features
//! taken modulo 2^21 as there. libFuzzer counts every remaining input's uncovered features again
//! for each pick, so a 92 K-input corpus took as long to cover as to replay (55 min, 2026-11-19).
//! Here the count an input was last given is an upper bound, kept in a heap: the top is
//! counted again, and taken when it still beats the next one's bound. That picks what the eager
//! loop picks, in seconds. On render's corpus (8,389 inputs) it kept 6,526 where libFuzzer kept
//! 6,525, 97% of them the same inputs, over the same 18,700 edges.
//!
//! With `--covered CONTROL`, those control files' inputs are the corpus: their features count as
//! covered, and the picks are the inputs of the rest that add to them, as `-set_cover_merge=1`
//! picks with them in its first corpus dir (a smoke's new inputs, against the corpus it began
//! from).
//!
//! With `--covered CONTROL... --new INPUTS FEATURES`, no replay of the new inputs: those of each
//! INPUTS dir whose features file in FEATURES has a feature the covered ones lack (`prefilter`).
//!
//! An input started without an FT line crashed, timed out or ran out of memory (the replay's
//! artifact prefix has it): counted and named on stderr, never picked. An input never started
//! means the replay stopped early, an error.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread;

/// libFuzzer's feature space in its set cover and its corpus (`kFeatureSetSize`).
const FEATURE_SET: u32 = 1 << 21;

/// Picks at which stderr reports the share of features covered, to size a smoke's sample.
const CURVE: [usize; 6] = [500, 1_000, 2_000, 5_000, 10_000, 20_000];

#[derive(Debug, Default)]
struct Input {
    name: String,
    size: u64,
    /// sorted, deduplicated, modulo [`FEATURE_SET`]; empty for a failed input
    features: Vec<u32>,
}

#[derive(Debug, Default)]
struct Control {
    inputs: Vec<Input>,
    failed: Vec<String>,
    edges: Vec<u32>,
}

const USAGE: &str = "usage: cargo xtask cover [--covered CONTROL]... CONTROL...\n       \
                     cargo xtask cover --covered CONTROL... --new INPUTS FEATURES...";

pub fn run(args: &[String]) -> Result<ExitCode, String> {
    let (mut base, mut paths, mut new) = (Vec::new(), Vec::new(), Vec::new());
    let mut args = args.iter();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--covered" => base.push(PathBuf::from(args.next().ok_or(USAGE)?)),
            "--new" => {
                let inputs = PathBuf::from(args.next().ok_or(USAGE)?);
                new.push((inputs, PathBuf::from(args.next().ok_or(USAGE)?)));
            }
            _ => paths.push(PathBuf::from(a)),
        }
    }
    if paths.is_empty() == new.is_empty() {
        return Err(USAGE.into());
    }
    let read = |paths: &[PathBuf]| -> Result<Control, String> {
        let controls = thread::scope(|s| {
            let jobs: Vec<_> = paths.iter().map(|p| s.spawn(|| parse_file(p))).collect();
            jobs.into_iter()
                .map(|j| j.join().map_err(|_| "a parser panicked".to_string())?)
                .collect::<Result<Vec<_>, String>>()
        })?;
        let mut all = Control::default();
        for c in controls {
            all.inputs.extend(c.inputs);
            all.failed.extend(c.failed);
            all.edges.extend(c.edges);
        }
        all.edges.sort_unstable();
        all.edges.dedup();
        Ok(all)
    };
    let base = read(&base)?;
    let mut covered = vec![0u64; (FEATURE_SET / 64) as usize];
    for i in &base.inputs {
        mark(&mut covered, &i.features);
    }
    if !new.is_empty() {
        return prefilter(&new, &covered);
    }
    let Control {
        inputs,
        failed,
        edges,
    } = read(&paths)?;

    let picks = cover(&inputs, covered.clone());

    let mut out = BufWriter::new(io::stdout().lock());
    for &k in &picks {
        writeln!(out, "{}", inputs[k].name).map_err(|e| e.to_string())?;
    }
    out.flush().map_err(|e| e.to_string())?;

    let all = union(inputs.iter().map(|i| &i.features[..]));
    let bytes: u64 = picks.iter().map(|&k| inputs[k].size).sum();
    if base.inputs.is_empty() {
        eprintln!(
            "cover: {} inputs replayed, {} failed; {} features, {} edges; keeps {} inputs ({:.1} MB)",
            inputs.len(),
            failed.len(),
            all,
            edges.len(),
            picks.len(),
            bytes as f64 / 1e6
        );
        let mut curve = Vec::new();
        for n in CURVE.into_iter().filter(|&n| n < picks.len()) {
            let got = union(picks[..n].iter().map(|&k| &inputs[k].features[..]));
            curve.push(format!(
                "{n} {:.1}%",
                100.0 * got as f64 / all.max(1) as f64
            ));
        }
        if !curve.is_empty() {
            eprintln!(
                "cover: the first picks' share of the features: {}",
                curve.join(", ")
            );
        }
    } else {
        let had = count(&covered);
        let mut after = covered;
        for &k in &picks {
            mark(&mut after, &inputs[k].features);
        }
        let new_edges = edges
            .iter()
            .filter(|e| base.edges.binary_search(e).is_err())
            .count();
        eprintln!(
            "cover: {} inputs covered ({had} features, {} edges); {} more replayed, {} failed; \
             keeps {} of them ({:.1} MB): +{} features, +{new_edges} edges",
            base.inputs.len(),
            base.edges.len(),
            inputs.len(),
            failed.len(),
            picks.len(),
            bytes as f64 / 1e6,
            count(&after) - had,
        );
    }
    for name in base.failed.iter().chain(&failed) {
        eprintln!("failed: {name}");
    }
    Ok(ExitCode::SUCCESS)
}

/// `--new`: the inputs plain libFuzzer workers kept (INPUTS) that may add to the covered features,
/// by the features file a worker writes for each (`-features_dir=FEATURES`, named as the input):
/// the features the input added to that worker, raw `u32`s. Any feature the corpus lacks is in
/// the file of some input the workers kept, so the inputs with one in theirs, and those without a
/// file, hold all the workers found; they go to stdout, to be replayed and covered exactly.
/// `-fork`'s parent filters its jobs' inputs the same way (`RunOneMergeJob`).
fn prefilter(new: &[(PathBuf, PathBuf)], covered: &[u64]) -> Result<ExitCode, String> {
    let (mut kept, mut unknown, mut all) = (0usize, 0usize, 0usize);
    let mut out = BufWriter::new(io::stdout().lock());
    for (inputs, features) in new {
        let dir = fs::read_dir(inputs).map_err(|e| format!("{}: {e}", inputs.display()))?;
        for entry in dir {
            let entry = entry.map_err(|e| e.to_string())?;
            if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
                continue;
            }
            all += 1;
            let keep = match fs::read(features.join(entry.file_name())) {
                Ok(bytes) => bytes.as_chunks::<4>().0.iter().any(|b| {
                    let f = u32::from_le_bytes(*b) % FEATURE_SET;
                    covered[(f / 64) as usize] & (1 << (f % 64)) == 0
                }),
                Err(_) => {
                    unknown += 1;
                    true
                }
            };
            if keep {
                kept += 1;
                writeln!(out, "{}", entry.path().display()).map_err(|e| e.to_string())?;
            }
        }
    }
    out.flush().map_err(|e| e.to_string())?;
    eprintln!(
        "prefilter: {all} inputs kept by the workers, {kept} may add to the covered features \
         ({unknown} of them without a features file)"
    );
    Ok(ExitCode::SUCCESS)
}

fn mark(covered: &mut [u64], features: &[u32]) {
    for &f in features {
        covered[(f / 64) as usize] |= 1 << (f % 64);
    }
}

fn count(set: &[u64]) -> usize {
    set.iter().map(|w| w.count_ones() as usize).sum()
}

fn union<'a>(sets: impl Iterator<Item = &'a [u32]>) -> usize {
    let mut seen = vec![0u64; (FEATURE_SET / 64) as usize];
    for set in sets {
        mark(&mut seen, set);
    }
    count(&seen)
}

/// The greedy set cover, lazily, of what `covered` (a bit a feature) lacks: indices into
/// `inputs`, best first.
fn cover(inputs: &[Input], mut covered: Vec<u64>) -> Vec<usize> {
    let fresh = |fs: &[u32], covered: &[u64]| {
        fs.iter()
            .filter(|&&f| covered[(f / 64) as usize] & (1 << (f % 64)) == 0)
            .count()
    };
    // (uncovered features, the smaller first, the earlier first): a total order, so the picks
    // are the eager loop's
    let mut heap: BinaryHeap<(usize, Reverse<u64>, Reverse<usize>)> = inputs
        .iter()
        .enumerate()
        .filter(|(_, i)| !i.features.is_empty())
        .map(|(k, i)| (i.features.len(), Reverse(i.size), Reverse(k)))
        .collect();
    let mut picks = Vec::new();
    while let Some((bound, size, Reverse(k))) = heap.pop() {
        let now = fresh(&inputs[k].features, &covered);
        if now == 0 {
            continue;
        }
        if now < bound {
            let key = (now, size, Reverse(k));
            if heap.peek().is_some_and(|top| *top > key) {
                heap.push(key);
                continue;
            }
        }
        mark(&mut covered, &inputs[k].features);
        picks.push(k);
    }
    picks
}

fn parse_file(path: &Path) -> Result<Control, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(BufReader::with_capacity(1 << 20, file)).map_err(|e| format!("{}: {e}", path.display()))
}

/// A control file as `Merger::Parse` reads it: the input count, the first corpus's count, the
/// names, then `STARTED i size`, `FT i features...` and `COV i edges...` lines in input order.
fn parse(mut r: impl BufRead) -> Result<Control, String> {
    let mut line = Vec::new();
    let mut next = |line: &mut Vec<u8>| -> Result<bool, String> {
        line.clear();
        let n = r.read_until(b'\n', line).map_err(|e| e.to_string())?;
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        Ok(n > 0)
    };
    let mut header = |line: &mut Vec<u8>, what: &str| -> Result<usize, String> {
        if !next(line)? {
            return Err(format!("no {what}"));
        }
        number(line).ok_or_else(|| format!("{what}: {}", String::from_utf8_lossy(line)))
    };
    let count = header(&mut line, "input count")?;
    let _first_corpus = header(&mut line, "first corpus count")?;
    let mut c = Control::default();
    for _ in 0..count {
        if !next(&mut line)? {
            return Err(format!("fewer names than {count}"));
        }
        let name = String::from_utf8(line.clone()).map_err(|e| e.to_string())?;
        c.inputs.push(Input {
            name,
            ..Input::default()
        });
    }

    // inputs[..started] were started; `done` says whether the last one has its FT line
    let (mut started, mut done) = (0usize, true);
    let mut ft = Vec::new();
    while next(&mut line)? {
        let mut words = line.split(|&b| b == b' ').filter(|w| !w.is_empty());
        let marker = words.next().unwrap_or_default();
        let id = words
            .next()
            .and_then(number)
            .ok_or("a line without an input id")?;
        match marker {
            b"STARTED" => {
                if id != started || id >= count {
                    return Err(format!("STARTED {id} after {started} inputs"));
                }
                if !done {
                    c.failed.push(c.inputs[id - 1].name.clone());
                }
                c.inputs[id].size = words.next().and_then(number).unwrap_or(0) as u64;
                started += 1;
                done = false;
            }
            b"FT" if id + 1 == started && !done => {
                ft.clear();
                for w in words {
                    let f = number64(w).ok_or("a feature that isn't a number")?;
                    ft.push((f % u64::from(FEATURE_SET)) as u32);
                }
                ft.sort_unstable();
                ft.dedup();
                c.inputs[id].features = ft.clone();
                done = true;
            }
            b"COV" if id + 1 == started => {
                for w in words {
                    c.edges
                        .push(number(w).ok_or("an edge that isn't a number")? as u32);
                }
            }
            _ => {
                return Err(format!(
                    "unexpected line: {}",
                    String::from_utf8_lossy(&line)
                ));
            }
        }
    }
    if !done {
        c.failed.push(c.inputs[started - 1].name.clone());
    }
    if started < count {
        return Err(format!(
            "{} of {count} inputs not replayed",
            count - started
        ));
    }
    Ok(c)
}

fn number(w: &[u8]) -> Option<usize> {
    number64(w).and_then(|n| usize::try_from(n).ok())
}

fn number64(w: &[u8]) -> Option<u64> {
    if w.is_empty() {
        return None;
    }
    w.iter().try_fold(0u64, |n, &b| {
        b.is_ascii_digit()
            .then(|| n.checked_mul(10)?.checked_add(u64::from(b - b'0')))
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> Vec<u64> {
        vec![0; (FEATURE_SET / 64) as usize]
    }

    #[test]
    fn covered_features_are_not_picked_again() {
        let inputs = [input("old", 1, &[1, 2, 3]), input("new", 1, &[3, 4])];
        let mut covered = none();
        mark(&mut covered, &[1, 2, 3, 4]);
        assert!(cover(&inputs, covered.clone()).is_empty());
        let mut some = none();
        mark(&mut some, &[1, 2]);
        let names: Vec<_> = cover(&inputs, some)
            .iter()
            .map(|&k| inputs[k].name.as_str())
            .collect();
        assert_eq!(names, ["new"]);
    }

    fn input(name: &str, size: u64, features: &[u32]) -> Input {
        Input {
            name: name.into(),
            size,
            features: features.to_vec(),
        }
    }

    #[test]
    fn parses_a_replay_with_a_crash() {
        let text = "4\n0\na\nb\nc\nd\nSTARTED 0 10\nFT 0 1 5 2097153\nCOV 0 7 8\n\
                    STARTED 1 3\nSTARTED 2 4\nFT 2 9\nCOV 2\nSTARTED 3 1\n";
        let c = parse(text.as_bytes()).unwrap();
        assert_eq!(c.inputs[0].features, [1, 5]); // 2^21 + 1 is 1 again
        assert_eq!(c.inputs[2].features, [9]);
        assert_eq!(c.failed, ["b", "d"]);
        assert_eq!(c.edges, [7, 8]);
    }

    #[test]
    fn an_unfinished_replay_is_an_error() {
        assert!(parse("2\n0\na\nb\nSTARTED 0 1\nFT 0 1\nCOV 0\n".as_bytes()).is_err());
    }

    #[test]
    fn picks_as_the_eager_greedy_does() {
        let inputs = [
            input("small", 1, &[1, 2]),
            input("big", 9, &[1, 2, 3, 4]),
            input("rest", 2, &[3, 4, 5]),
            input("tie", 1, &[5, 6]),
            input("tie-bigger", 3, &[5, 6]),
            input("nothing", 1, &[1]),
        ];
        let names: Vec<_> = cover(&inputs, none())
            .iter()
            .map(|&k| inputs[k].name.as_str())
            .collect();
        // big takes 1–4; then tie and rest add 2 (5, 6) and 1 (5): tie, the smaller of the two
        // that add 2; then small: nothing left
        assert_eq!(names, ["big", "tie"]);
    }

    #[test]
    fn a_stale_bound_waits_for_a_better_input() {
        let inputs = [
            input("a", 1, &[1, 2, 3, 4, 5]),
            input("b", 1, &[1, 2, 3, 6]), // bound 4, adds 1 after a
            input("c", 1, &[7, 8]),
        ];
        let names: Vec<_> = cover(&inputs, none())
            .iter()
            .map(|&k| inputs[k].name.as_str())
            .collect();
        assert_eq!(names, ["a", "c", "b"]);
    }
}
