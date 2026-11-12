//! `cargo xtask size`: ada-parse's binary size (ADR-40), in all and by module. Builds ada as the
//! release does (fat LTO), but unstripped and with line tables, in `target/size`, and reports
//! ada-parse's bytes by module, ada's own, and the binary's size stripped (`strip`, as the
//! profile's `strip = true` leaves it). A report, not a gate: the size gate, ada-parse's bytes
//! against those its lexers took out of syntect's set, was retired on 2026-11-03, and the set
//! left the binary with syntect (ADR-43).
//!
//! Code is counted by instruction, not by symbol: each function's bytes go to the source files
//! the line tables (`llvm-dwarfdump --debug-line`) give its instructions, so that a scanner
//! inlined into a shared loop (`lines_run::<S>`, a carried scan's `block_runs`) counts as the
//! scanner's, and ada-parse's code inlined into ada's as ada-parse's. The helpers every scanner
//! inlines (`SHARED`: the sinks' writes, byte classes, word lookups) count as the language's in
//! a function a language's module leads, and as their own elsewhere. What has no file of the
//! binary's own (std's and the dependencies' code inlined into a function, line 0, padding) goes
//! to the function's owner: the group with most of its own bytes, when its name is ada's or
//! ada-parse's or those bytes are at least half of it; otherwise it stays "other". Data is
//! counted by its symbol's path (`ada_parse::css::TABLE`): the named statics, keyword tables and
//! masks; anonymous constants (string literals, a `const`'s table) have no name and stay "other".

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::process::{Command, ExitCode};

/// ada-parse's modules every scanner inlines (the sinks, byte classes, word lookups, context
/// stacks, the crate's loops): within a function a language's module leads, their bytes are
/// the language's; on their own, theirs.
const SHARED: &[&str] = &["emit", "prim", "words", "matcher", "stack", "lib", "lang"];

/// Where bytes go: an ada-parse module, ada's highlighting, the rest of ada, or anything else.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Group {
    Parse(String),
    Highlight,
    Ada,
    Other,
}

pub fn run(root: &Path) -> Result<ExitCode, String> {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| root.join("target"), Into::into)
        .join("size");
    let status = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args(["build", "--release", "--bin", "ada", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", &target)
        .env("CARGO_PROFILE_RELEASE_STRIP", "false")
        .env("CARGO_PROFILE_RELEASE_DEBUG", "line-tables-only")
        .status()
        .map_err(|e| format!("cargo: {e}"))?;
    if !status.success() {
        return Err("size: the build failed".into());
    }
    let bin = target.join("release/ada");

    let ranges = line_ranges(&bin, root)?;
    let (code, data) = attribute(&bin, &ranges)?;

    let stripped = target.join("ada.stripped");
    let status = Command::new("strip")
        .arg("-o")
        .arg(&stripped)
        .arg(&bin)
        .status()
        .map_err(|e| format!("strip: {e}"))?;
    if !status.success() {
        return Err("strip failed".into());
    }
    let binary = fs::metadata(&stripped)
        .map(|m| m.len())
        .map_err(|e| format!("{}: {e}", stripped.display()))?;

    let get = |m: &HashMap<Group, u64>, g: &Group| m.get(g).copied().unwrap_or(0);
    let mut modules: Vec<(&str, u64, u64)> = code
        .keys()
        .chain(data.keys())
        .filter_map(|g| match g {
            Group::Parse(m) => Some(m.as_str()),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|m| {
            let g = Group::Parse(m.to_string());
            (m, get(&code, &g), get(&data, &g))
        })
        .collect();
    modules.sort_by_key(|&(m, c, d)| (std::cmp::Reverse(c + d), m));
    let (parse_code, parse_data) = modules
        .iter()
        .fold((0, 0), |(c, d), &(_, mc, md)| (c + mc, d + md));
    let parse = parse_code + parse_data;
    let pct = |n: u64, of: u64| 100.0 * n as f64 / of.max(1) as f64;

    println!("ada-parse by module (code by line tables, named statics), bytes:");
    println!(
        "  {:<16} {:>9} {:>9} {:>9} {:>7}",
        "module", "code", "data", "total", "share"
    );
    for &(m, c, d) in &modules {
        println!(
            "  {m:<16} {c:>9} {d:>9} {:>9} {:>6.1}%",
            c + d,
            pct(c + d, parse)
        );
    }
    println!(
        "  {:<16} {parse_code:>9} {parse_data:>9} {parse:>9}",
        "ada-parse"
    );
    println!("by group, bytes (code + named data):");
    for (label, g) in [
        ("ada-parse", None),
        ("src/highlight", Some(Group::Highlight)),
        ("ada, the rest", Some(Group::Ada)),
        ("other", Some(Group::Other)),
    ] {
        let n = g.map_or(parse, |g| get(&code, &g) + get(&data, &g));
        println!("  {label:<16} {n:>9}");
    }
    println!(
        "ada, stripped: {binary} bytes; ada-parse {:.1}% of it",
        pct(parse, binary)
    );
    Ok(ExitCode::SUCCESS)
}

/// The group of a source path from the line tables: absolute, or relative to the workspace.
fn group_of_path(path: &str, root: &str) -> Group {
    let rel = path
        .strip_prefix(root)
        .map_or(path, |p| p.trim_start_matches('/'));
    if let Some(i) = rel.find("ada-parse/src/") {
        let rest = &rel[i + "ada-parse/src/".len()..];
        let first = rest.split('/').next().unwrap_or(rest);
        return Group::Parse(module_name(first.trim_end_matches(".rs")));
    }
    if rel.starts_with("src/highlight") {
        Group::Highlight
    } else if rel.starts_with("src/") || rel == "build.rs" {
        Group::Ada
    } else {
        Group::Other
    }
}

/// A module's name as the report gives it: a format's `_scan` beside its matchers is the format's.
fn module_name(m: &str) -> String {
    m.strip_suffix("_scan").unwrap_or(m).to_string()
}

/// The group a demangled symbol's path names, by its crate and first module.
fn group_of_name(name: &str, modules: &[String]) -> Group {
    let path = name.trim_start_matches('<');
    if let Some(rest) = path.strip_prefix("ada_parse::") {
        let first = rest.split("::").next().unwrap_or(rest);
        let m = module_name(first);
        return Group::Parse(if modules.contains(&m) {
            m
        } else {
            "lib".into()
        });
    }
    if path.starts_with("ada::highlight") {
        Group::Highlight
    } else if path.starts_with("ada::") {
        Group::Ada
    } else {
        Group::Other
    }
}

/// The live code's address ranges by group, sorted, from the line tables.
fn line_ranges(bin: &Path, root: &Path) -> Result<Vec<(u64, u64, Group)>, String> {
    let out = Command::new("llvm-dwarfdump")
        .arg("--debug-line")
        .arg(bin)
        .output()
        .map_err(|e| format!("llvm-dwarfdump: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "llvm-dwarfdump: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let root = root.to_string_lossy();
    let (mut dirs, mut files) = (HashMap::new(), HashMap::<u64, Group>::new());
    let (mut file, mut name) = (None::<u64>, String::new());
    let mut ranges = Vec::new();
    let mut prev: Option<(u64, u64)> = None;
    let mut dead = false;
    let unquote = |s: &str| s.trim().trim_matches('"').to_string();
    for line in text.lines() {
        let t = line.trim_start();
        if t.starts_with("debug_line[") {
            dirs.clear();
            files.clear();
            prev = None;
        } else if let Some(rest) = t.strip_prefix("include_directories[") {
            if let Some((i, dir)) = rest.split_once("] =") {
                dirs.insert(i.trim().parse::<u64>().unwrap_or(0), unquote(dir));
            }
        } else if let Some(rest) = t.strip_prefix("file_names[") {
            file = rest.split(']').next().and_then(|i| i.trim().parse().ok());
        } else if let Some(n) = t.strip_prefix("name: ") {
            name = unquote(n);
        } else if let Some(d) = t.strip_prefix("dir_index: ") {
            if let Some(i) = file.take() {
                let dir = d.trim().parse::<u64>().ok().and_then(|d| dirs.get(&d));
                let path = dir.map_or_else(|| name.clone(), |d| format!("{d}/{name}"));
                files.insert(i, group_of_path(&path, &root));
            }
        } else if t.starts_with("0x") {
            // `ADDRESS LINE COLUMN FILE ISA DISCRIMINATOR OPINDEX FLAGS…`
            let mut f = t.split_whitespace();
            let addr = f
                .next()
                .and_then(|a| u64::from_str_radix(a.trim_start_matches("0x"), 16).ok());
            let file = f.nth(2).and_then(|i| i.parse::<u64>().ok());
            let (Some(addr), Some(file)) = (addr, file) else {
                continue;
            };
            if prev.is_none() {
                // a sequence that starts at 0 is code the linker dropped
                dead = addr == 0;
            }
            if let Some((start, pf)) = prev
                && addr > start
                && !dead
            {
                let g = files.get(&pf).cloned().unwrap_or(Group::Other);
                ranges.push((start, addr, g));
            }
            prev = (!t.contains("end_sequence")).then_some((addr, file));
        }
    }
    ranges.sort_by_key(|&(s, _, _)| s);
    Ok(ranges)
}

/// The code's bytes and the named data's by group: each function's instructions by their line
/// ranges, its unplaced bytes to its owner; each data symbol by its name.
#[allow(clippy::type_complexity)]
fn attribute(
    bin: &Path,
    ranges: &[(u64, u64, Group)],
) -> Result<(HashMap<Group, u64>, HashMap<Group, u64>), String> {
    let out = Command::new("nm")
        .args(["-S", "--defined-only"])
        .arg(bin)
        .output()
        .map_err(|e| format!("nm: {e}"))?;
    if !out.status.success() {
        return Err(format!("nm: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let modules: Vec<String> = ranges
        .iter()
        .filter_map(|(_, _, g)| match g {
            Group::Parse(m) => Some(m.clone()),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let (mut code, mut data) = (HashMap::new(), HashMap::new());
    // `ADDRESS SIZE TYPE NAME`; a symbol at the same address twice (an alias) counts once
    let mut seen = std::collections::HashSet::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut f = line.splitn(4, ' ');
        let (Some(addr), Some(size), Some(kind), Some(raw)) =
            (f.next(), f.next(), f.next(), f.next())
        else {
            continue;
        };
        let (Ok(addr), Ok(size)) = (u64::from_str_radix(addr, 16), u64::from_str_radix(size, 16))
        else {
            continue;
        };
        if size == 0 || !seen.insert((addr, kind == "t" || kind == "T")) {
            continue;
        }
        let name = rustc_demangle::try_demangle(raw)
            .map_or_else(|_| raw.to_string(), |d| format!("{d:#}"));
        match kind {
            "t" | "T" => {
                let end = addr + size;
                let mut own: BTreeMap<Group, u64> = BTreeMap::new();
                let first = ranges.partition_point(|&(_, e, _)| e <= addr);
                for (s, e, g) in ranges[first..].iter().take_while(|&&(s, _, _)| s < end) {
                    let n = e.min(&end) - s.max(&addr);
                    if *g != Group::Other {
                        *own.entry(g.clone()).or_default() += n;
                    }
                }
                let placed: u64 = own.values().sum();
                let named = group_of_name(&name, &modules) != Group::Other;
                let mut owner = own
                    .iter()
                    .max_by_key(|&(_, n)| *n)
                    .map(|(g, _)| g.clone())
                    .filter(|_| named || placed * 2 >= size)
                    .unwrap_or(Group::Other);
                // the helpers inlined into a language's code are the language's: its sink's writes,
                // its byte classes and word lookups
                let language = own
                    .iter()
                    .filter(|(g, _)| matches!(g, Group::Parse(m) if !SHARED.contains(&m.as_str())))
                    .max_by_key(|&(_, n)| *n)
                    .map(|(g, _)| g.clone());
                if let (Group::Parse(_), Some(language)) = (&owner, language) {
                    let shared: u64 = own
                        .iter()
                        .filter(
                            |(g, _)| matches!(g, Group::Parse(m) if SHARED.contains(&m.as_str())),
                        )
                        .map(|(_, n)| n)
                        .sum();
                    own.retain(
                        |g, _| !matches!(g, Group::Parse(m) if SHARED.contains(&m.as_str())),
                    );
                    *own.entry(language.clone()).or_default() += shared;
                    owner = language;
                }
                for (g, n) in own {
                    *code.entry(g).or_default() += n;
                }
                *code.entry(owner).or_default() += size.saturating_sub(placed);
            }
            "r" | "R" | "d" | "D" => {
                *data.entry(group_of_name(&name, &modules)).or_default() += size;
            }
            _ => {}
        }
    }
    Ok((code, data))
}
