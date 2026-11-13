//! How well stdin's content sniffer (`sniff.rs`, ADR-30) tells a file's type without its name.
//! Each file of a corpus is typed twice: by its path, as ada types a file it's started on (the
//! truth), and as `-`, as piped stdin is. A file its name doesn't type is left out, and so is a
//! binary one (stdin refuses it).
//!
//! The report gives, for each true type, its files, how many the sniffer types the same, and what
//! the rest came out as; then, for each type the sniffer gave, how many files of another type it
//! claimed (false claims, the cost of a looser rule). `--show N` lists the paths of the first N
//! misses of each type; `--tsv FILE` writes every file's `truth`, `sniffed` and path.
//! `--explain FILE...` instead shows why each file sniffs as it does: the line signals' scores
//! and the signals that matched.
//!
//! ```zsh
//! bench/sniff/corpus.zsh /tmp/sniff     # the census lists on disk, as one manifest
//! cargo run --release --example sniff_census -- @lists/rust.txt @lists/go.txt
//! target/release/examples/sniff_census --show 5 --tsv out.tsv @/tmp/sniff/corpus.txt
//! ```
//! `@FILE` names a file of paths, one a line; other arguments are paths. A line `TYPE<TAB>PATH`
//! gives the file's type where its name would mislead (the crontab census's `*crontab.txt`).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::{env, fs, io};

use ada::filetype::{self, FileType};

fn label(t: FileType) -> &'static str {
    match t {
        FileType::Markdown => "Markdown",
        FileType::Image => "Image",
        FileType::Text { lang: None } => "Plain",
        FileType::Text { lang: Some(lang) } => lang,
    }
}

#[derive(Default)]
struct Row {
    files: usize,
    hits: usize,
    /// What the misses came out as, and their paths.
    misses: BTreeMap<&'static str, Vec<String>>,
}

fn main() -> io::Result<()> {
    let mut args = env::args().skip(1);
    let mut paths = Vec::new();
    let mut show = 0;
    let mut tsv = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--show" => show = args.next().and_then(|n| n.parse().ok()).expect("--show N"),
            "--tsv" => tsv = Some(fs::File::create(args.next().expect("--tsv FILE"))?),
            "--explain" => {
                for path in args.by_ref() {
                    let text = String::from_utf8_lossy(&fs::read(&path)?).into_owned();
                    print!("{path}\n{}", ada::sniff::explain(&text));
                }
                return Ok(());
            }
            list if list.starts_with('@') => {
                let text = fs::read_to_string(&list[1..])?;
                paths.extend(text.lines().filter(|l| !l.is_empty()).map(
                    |l| match l.split_once('\t') {
                        Some((truth, path)) => (Some(truth.to_owned()), path.to_owned()),
                        None => (None, l.to_owned()),
                    },
                ));
            }
            path => paths.push((None, path.to_owned())),
        }
    }
    let mut tsv = tsv.map(io::BufWriter::new);

    let (mut untyped, mut unreadable, mut binary) = (0, 0, 0);
    let mut rows: BTreeMap<&'static str, Row> = BTreeMap::new();
    // per sniffed type: files of another type it claimed
    let mut claims: BTreeMap<&'static str, BTreeMap<&'static str, usize>> = BTreeMap::new();
    // a type given in the list stays for the report
    let given: Vec<&'static str> = {
        let mut names: Vec<String> = paths.iter().filter_map(|(t, _)| t.clone()).collect();
        names.sort();
        names.dedup();
        names.into_iter().map(|n| &*n.leak()).collect()
    };
    for (truth, path) in &paths {
        let p = Path::new(path);
        let truth = truth
            .as_deref()
            .map(|t| *given.iter().find(|g| **g == t).unwrap());
        if truth.is_none() && FileType::by_path(p).is_none() {
            untyped += 1;
            continue;
        }
        let Ok(bytes) = fs::read(p) else {
            unreadable += 1;
            continue;
        };
        if filetype::is_binary(&bytes) {
            binary += 1;
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        let truth = truth.unwrap_or_else(|| label(FileType::of(p, &text)));
        // a piped image is told by its magic bytes before the sniffer, as `read_stdin` does
        let sniffed = if filetype::image_ext(&bytes).is_some() {
            "Image"
        } else {
            label(FileType::of(Path::new("-"), &text))
        };
        if let Some(out) = &mut tsv {
            writeln!(out, "{truth}\t{sniffed}\t{path}")?;
        }
        let row = rows.entry(truth).or_default();
        row.files += 1;
        if truth == sniffed {
            row.hits += 1;
        } else {
            row.misses.entry(sniffed).or_default().push(path.clone());
            *claims.entry(sniffed).or_default().entry(truth).or_default() += 1;
        }
    }

    let out = &mut io::stdout().lock();
    let (files, hits) = rows
        .values()
        .fold((0, 0), |(f, h), r| (f + r.files, h + r.hits));
    writeln!(
        out,
        "{files} files: {hits} sniffed as typed ({:.1}%), {} missed; left out: {untyped} untyped by name, {binary} binary, {unreadable} unreadable\n",
        pct(hits, files),
        files - hits,
    )?;
    writeln!(
        out,
        "{:<28} {:>6} {:>6} {:>6}  missed as",
        "type", "files", "hits", "%"
    )?;
    for (truth, row) in &rows {
        let mut misses: Vec<_> = row.misses.iter().map(|(t, p)| (p.len(), *t)).collect();
        misses.sort_by(|a, b| b.cmp(a));
        let missed = misses
            .iter()
            .map(|(n, t)| format!("{t} {n}"))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            out,
            "{truth:<28} {:>6} {:>6} {:>5.1}%  {missed}",
            row.files,
            row.hits,
            pct(row.hits, row.files),
        )?;
    }
    writeln!(
        out,
        "\nfalse claims (sniffed as, but typed otherwise by name)"
    )?;
    for (sniffed, from) in &claims {
        if *sniffed == "Plain" {
            continue;
        }
        let total: usize = from.values().sum();
        let mut from: Vec<_> = from.iter().map(|(t, n)| (*n, *t)).collect();
        from.sort_by(|a, b| b.cmp(a));
        let from = from
            .iter()
            .map(|(n, t)| format!("{t} {n}"))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(out, "{sniffed:<28} {total:>6}  {from}")?;
    }
    if show > 0 {
        writeln!(out, "\nmisses")?;
        for (truth, row) in &rows {
            for (sniffed, paths) in &row.misses {
                for path in paths.iter().take(show) {
                    writeln!(out, "{truth} -> {sniffed}\t{path}")?;
                }
            }
        }
    }
    Ok(())
}

fn pct(n: usize, of: usize) -> f64 {
    if of == 0 {
        0.0
    } else {
        n as f64 * 100.0 / of as f64
    }
}
