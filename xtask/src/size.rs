//! `cargo xtask size`: ada-parse's binary size (ADR-40). Builds ada as the release does (fat
//! LTO), but unstripped and with line tables, in `target/size`; attributes the binary's symbols
//! to their source files with `nm -S -l`; and reports ada-parse's bytes, those of ada's lexers
//! outside it, the binary's size stripped (`strip`, as the profile's `strip = true` leaves it)
//! and the bytes ada's trimmed syntax set drops from two-face's (`cargo xtask dump`). A report,
//! not a gate: the size gate was retired on 2026-11-03, a guideline the scanners were always past.
//!
//! Fat LTO inlines generic code into the crate that instantiates it, under that crate's file
//! names or std's, so a figure for a generic-heavy dependency (syntect) comes out low; ada's
//! own modules and ada-parse's hold.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, ExitCode};

use crate::dump;

/// Where a symbol's source path puts it, first match wins: (group, a piece of the path).
const GROUPS: &[(&str, &str)] = &[
    ("ada-parse", "/ada-parse/src/"),
    ("src/json.rs", "/ada-md/src/json.rs"),
    ("src/logs.rs", "/ada-md/src/logs.rs"),
    ("src/highlight.rs", "/ada-md/src/highlight.rs"),
    ("ada, the rest", "/ada-md/src/"),
    ("syntect", "/syntect-"),
    ("onig, Oniguruma", "/onig"),
];

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

    let out = Command::new("nm")
        .args(["-S", "--size-sort", "-l"])
        .arg(&bin)
        .output()
        .map_err(|e| format!("nm: {e}"))?;
    if !out.status.success() {
        return Err(format!("nm: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let mut sizes: BTreeMap<&str, u64> = BTreeMap::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        // `ADDRESS SIZE TYPE NAME`, and `\tPATH:LINE` where the line tables say
        let mut fields = line.split_whitespace();
        let size = fields.nth(1).and_then(|s| u64::from_str_radix(s, 16).ok());
        let path = line.split_once('\t').map_or("", |(_, p)| p);
        let group = GROUPS
            .iter()
            .find(|(_, piece)| path.contains(piece))
            .map_or("other, or no line", |(g, _)| g);
        *sizes.entry(group).or_default() += size.unwrap_or(0);
    }

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
    let len = |p: &Path| {
        fs::metadata(p)
            .map(|m| m.len())
            .map_err(|e| format!("{}: {e}", p.display()))
    };
    let binary = len(&stripped)?;
    let two_face = len(&dump::source_path(&dump::two_face_dir(root)?, "onig"))?;
    let ours = len(&dump::asset_path(root, "onig"))?;
    let budget = two_face.saturating_sub(ours);

    println!("symbols by source (nm -S -l), bytes:");
    for (group, _) in GROUPS {
        println!(
            "  {group:<28} {:>9}",
            sizes.get(group).copied().unwrap_or(0)
        );
    }
    let other = sizes.get("other, or no line").copied().unwrap_or(0);
    println!("  {:<28} {other:>9}", "other, or no line");
    println!("ada, stripped: {binary} bytes");
    println!("syntax set: two-face's {two_face}, ada's {ours}: {budget} bytes dropped");
    let parse = sizes.get("ada-parse").copied().unwrap_or(0);
    println!("ada-parse: {parse} bytes, against the {budget} the trimmed set drops");
    Ok(ExitCode::SUCCESS)
}
