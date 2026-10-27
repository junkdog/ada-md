//! ada's build tasks, run through cargo's aliases (.cargo/config.toml).
//!
//! `cargo ordered [CARGO_ARGS...]`: `cargo build --release CARGO_ARGS` with the hot code first in
//! `.text` (MEM-02, MEM-08): the binary the release ships and the benches measure. A plain build
//! isn't ordered: `bench/order/order.txt`'s names are demangled, and the linker needs the build's
//! own. So the build links with a map, `place` maps order.txt onto the map's names, and the build
//! links again with them (`ADA_SYMBOL_ORDER`, build.rs). The order doesn't change a name, so the
//! next run links with the last one's and links once more only if it no longer fits: after a code
//! change. The files go in CARGO_TARGET_DIR (default: target)/order/TARGET (`--target`'s, or
//! `host`). Exits 1 when the build fails, 2 when it built but matched under 90% of order.txt's
//! names (`bench/order/trace.zsh` says how many are left after a large change); on a target lld
//! or ld64 doesn't link (build.rs) the build has no map, and goes unordered.
//!
//! `cargo xtask demangle`: symbol names on stdin, one per line, to the same lines as order.txt
//! keeps them (`symbols::demangle`); `bench/order/symbols.py` reads names through it.

mod symbols;

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, BufRead, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let done = match args.next().as_deref() {
        Some("ordered") => ordered(&args.collect::<Vec<_>>()),
        Some("demangle") => demangle()
            .map(|()| ExitCode::SUCCESS)
            .map_err(|e| e.to_string()),
        _ => Err("usage: cargo ordered [CARGO_ARGS...] | cargo xtask demangle < NAMES".into()),
    };
    done.unwrap_or_else(|e| {
        eprintln!("{e}");
        ExitCode::FAILURE
    })
}

fn ordered(args: &[String]) -> Result<ExitCode, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let target_dir =
        env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from);
    let work = target_dir
        .join("order")
        .join(target(args).unwrap_or("host"));
    fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let (order, map) = (work.join("ada.order"), work.join("ada.map"));
    let order_txt = root.join("bench/order/order.txt");

    // a build cargo finds fresh doesn't link: the last link's map is still the binary's
    let build = || -> Result<Option<symbols::Placement>, String> {
        let status = Command::new(env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")))
            .args(["build", "--release", "--manifest-path"])
            .arg(root.join("Cargo.toml"))
            .args(args)
            .env("ADA_SYMBOL_ORDER", &order)
            .env("ADA_LINK_MAP", &map)
            .status()
            .map_err(|e| format!("cargo: {e}"))?;
        if !status.success() {
            return Err("symbol order: the build failed".into());
        }
        if !map.exists() {
            return Ok(None);
        }
        symbols::place(&map, &order_txt).map(Some)
    };

    let last = fs::read_to_string(&order).ok();
    let Some(placed) = build()? else {
        println!("symbol order: not linked by lld or ld64 on this target, unordered");
        return Ok(ExitCode::SUCCESS);
    };
    let text: String = placed.names.iter().map(|n| format!("{n}\n")).collect();
    if last.as_deref() != Some(text.as_str()) {
        fs::write(&order, &text).map_err(|e| format!("{}: {e}", order.display()))?;
        if build()?.map(|again| again.names) != Some(placed.names.clone()) {
            return Err("symbol order: the second link renamed functions".into());
        }
    }
    let (found, total) = (placed.found, placed.total);
    println!(
        "symbol order: {found} of {total} names ({:.1}%), {} functions",
        100.0 * found as f64 / total.max(1) as f64,
        placed.names.len()
    );
    Ok(if found as f64 >= 0.9 * total as f64 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    })
}

/// The triple `--target` names, as `--target=T` or `--target T`.
fn target(args: &[String]) -> Option<&str> {
    args.iter()
        .find_map(|a| a.strip_prefix("--target="))
        .or_else(|| {
            args.windows(2)
                .find(|w| w[0] == "--target")
                .map(|w| w[1].as_str())
        })
}

fn demangle() -> io::Result<()> {
    let mut out = BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        writeln!(out, "{}", symbols::demangle(&line?))?;
    }
    out.flush()
}
