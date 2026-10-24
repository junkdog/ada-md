// Links the `ada` binary with the functions a typical run fetches first in `.text` (MEM-02), so a
// run maps ~1 MiB less of the binary: `ADA_SYMBOL_ORDER=PATH` names them, as this build's own
// symbols. `bench/order/order.txt` holds them demangled, for any toolchain and target, and
// `bench/order/build.py` builds with it: it links once with a map, maps the names onto the map's,
// and links again with them (MEM-08). A plain build isn't ordered. `--symbol-ordering-file` is
// lld's: only on x86_64 Linux with glibc, where rustc links through rust-lld by default since 1.90
// (before, through `cc` and GNU ld, which rejects it), and not when RUSTFLAGS pick a linker. A name
// lld doesn't find is skipped, so a stale file costs only the gain.
// `ADA_LINK_MAP=PATH` writes lld's map of the link, names mangled: the symbols of the binary as
// built, stripped or not. Tracing reads them there: a build with other profile settings (`strip`)
// hashes every symbol name differently.
// The same link packs the relative relocations (`-z pack-relative-relocs`, RELR; MEM-03): ld.so
// reads ~4 KiB at start instead of ~350 KiB of `.rela.dyn`, so a run maps ~0.4 MiB less. Only an
// ld.so of glibc 2.36 (2022) or later applies them, so only in a native build on such a glibc;
// the binary then needs `GLIBC_ABI_DT_RELR`, and an older glibc refuses it at start by name.
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

/// The minor version of the glibc this build script runs on: 44 for 2.44.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn glibc_minor() -> Option<u32> {
    unsafe extern "C" {
        fn gnu_get_libc_version() -> *const std::ffi::c_char;
    }
    // SAFETY: glibc returns a static NUL-terminated string
    let version = unsafe { std::ffi::CStr::from_ptr(gnu_get_libc_version()) };
    version.to_str().ok()?.split('.').nth(1)?.parse().ok()
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn glibc_minor() -> Option<u32> {
    None
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
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
    if var("HOST") == var("TARGET") && glibc_minor().is_some_and(|minor| minor >= 36) {
        println!("cargo:rustc-link-arg-bins=-Wl,-z,pack-relative-relocs");
    }
    let order = var("ADA_SYMBOL_ORDER");
    if !order.is_empty() {
        println!("cargo:rerun-if-changed={order}");
    }
    if Path::new(&order).is_file() {
        println!("cargo:rustc-link-arg-bins=-Wl,--symbol-ordering-file={order}");
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
