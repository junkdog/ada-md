// Links the `ada` binary with the functions a typical run fetches first in `.text`
// (`bench/order/order.txt`, from `bench/order/trace.zsh`; MEM-02), so a run maps ~1 MiB less of
// the binary. `--symbol-ordering-file` is lld's: only on x86_64 Linux with glibc, where rustc links
// through rust-lld by default since 1.90 (before, through `cc` and GNU ld, which rejects it), and not
// when RUSTFLAGS pick a linker. `ADA_SYMBOL_ORDER=off` turns it off. A name lld doesn't find (a stale
// file) is skipped, so a stale file costs only the gain.
// `ADA_LINK_MAP=PATH` writes lld's map of the link, names mangled: the symbols of the binary as
// built, stripped or not. Tracing reads them there: a build with other profile settings (`strip`)
// hashes every symbol name differently, and lld would skip them all.
use std::env;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::process::Command;

/// The minor version of the rustc building ada: 88 for 1.88.0.
fn rustc_minor() -> Option<u32> {
    let out = Command::new(env::var_os("RUSTC")?)
        .arg("-V")
        .output()
        .ok()?;
    let version = String::from_utf8(out.stdout).ok()?;
    version
        .split_whitespace()
        .nth(1)?
        .split('.')
        .nth(1)?
        .parse()
        .ok()
}

fn main() {
    println!("cargo:rerun-if-changed=bench/order");
    println!("cargo:rerun-if-env-changed=ADA_SYMBOL_ORDER");
    println!("cargo:rerun-if-env-changed=ADA_LINK_MAP");
    let var = |k: &str| env::var(k).unwrap_or_default();
    let lld = var("CARGO_CFG_TARGET_ARCH") == "x86_64"
        && var("CARGO_CFG_TARGET_OS") == "linux"
        && var("CARGO_CFG_TARGET_ENV") == "gnu"
        && !var("CARGO_ENCODED_RUSTFLAGS").contains("link")
        && rustc_minor().is_some_and(|minor| minor >= 90);
    if !lld {
        return;
    }
    let order = Path::new(&var("CARGO_MANIFEST_DIR")).join("bench/order/order.txt");
    if var("ADA_SYMBOL_ORDER") != "off" && order.is_file() {
        println!(
            "cargo:rustc-link-arg-bins=-Wl,--symbol-ordering-file={}",
            order.display()
        );
        println!("cargo:rustc-link-arg-bins=-Wl,--no-warn-symbol-ordering");
        // the arguments don't change with the file, so cargo wouldn't link again: its hash does
        let mut h = DefaultHasher::new();
        fs::read(&order).unwrap_or_default().hash(&mut h);
        println!("cargo:rustc-env=ADA_SYMBOL_ORDER_HASH={:016x}", h.finish());
    }
    let map = var("ADA_LINK_MAP");
    if !map.is_empty() {
        println!("cargo:rustc-link-arg-bins=-Wl,-Map={map}");
        println!("cargo:rustc-link-arg-bins=-Wl,--no-demangle");
    }
}
