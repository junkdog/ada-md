// Links the `ada` binary with the functions a typical run fetches first in `.text` (MEM-02), so a
// run maps ~1 MiB less of the binary: `ADA_SYMBOL_ORDER=PATH` names them, as this build's own
// symbols. `bench/order/order.txt` holds them demangled, for any toolchain and target, and
// `bench/order/build.py` builds with it: it links once with a map, maps the names onto the map's,
// and links again with them (MEM-08). A plain build isn't ordered. On Linux the file is lld's
// `--symbol-ordering-file`: rustc links x86_64 glibc builds through rust-lld by default since
// 1.90; elsewhere (aarch64, musl, an older rustc) `cc` is told to use the toolchain's own, as
// rustc does (`-B…/gcc-ld -fuse-ld=lld`), since GNU ld rejects the flag. On macOS it's ld64's
// `-order_file`, the names with Mach-O's leading `_`. Not when RUSTFLAGS pick a linker. A name the
// linker doesn't find is skipped, so a stale file costs only the gain.
// `ADA_LINK_MAP=PATH` writes the linker's map of the link, names mangled: the symbols of the
// binary as built, stripped or not. Tracing reads them there: a build with other profile settings
// (`strip`) hashes every symbol name differently.
// An lld link on glibc also packs the relative relocations (`-z pack-relative-relocs`, RELR;
// MEM-03): ld.so reads ~4 KiB at start instead of ~350 KiB of `.rela.dyn`, so a run maps ~0.4 MiB
// less. Only an ld.so of glibc 2.36 (2022) or later applies them, so only in a native build on
// such a glibc; the binary then needs `GLIBC_ABI_DT_RELR`, and an older glibc refuses it at start
// by name.
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

/// The directory of the toolchain's `ld.lld`, which runs its rust-lld: what rustc hands `cc` with
/// `-B` when it links through lld itself. None when the toolchain has no rust-lld.
fn gcc_ld() -> Option<String> {
    let out = Command::new(env::var_os("RUSTC")?)
        .args(["--print", "sysroot"])
        .output()
        .ok()?;
    let sysroot = String::from_utf8(out.stdout).ok()?;
    let host = env::var("HOST").ok()?;
    let dir = format!("{}/lib/rustlib/{host}/bin/gcc-ld", sysroot.trim());
    Path::new(&dir).join("ld.lld").is_file().then_some(dir)
}

enum Linker {
    Lld,
    Ld64,
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=ADA_SYMBOL_ORDER");
    println!("cargo:rerun-if-env-changed=ADA_LINK_MAP");
    let var = |k: &str| env::var(k).unwrap_or_default();
    let (arch, os, target_env) = (
        var("CARGO_CFG_TARGET_ARCH"),
        var("CARGO_CFG_TARGET_OS"),
        var("CARGO_CFG_TARGET_ENV"),
    );
    if var("CARGO_ENCODED_RUSTFLAGS").contains("link") {
        return;
    }
    let linker = if os == "macos" {
        Linker::Ld64
    } else if os == "linux"
        && matches!(arch.as_str(), "x86_64" | "aarch64")
        && matches!(target_env.as_str(), "gnu" | "musl")
    {
        let rust_lld = arch == "x86_64"
            && target_env == "gnu"
            && rustc_minor().is_some_and(|minor| minor >= 90);
        if !rust_lld {
            let Some(dir) = gcc_ld() else { return };
            println!("cargo:rustc-link-arg-bins=-B{dir}");
            println!("cargo:rustc-link-arg-bins=-fuse-ld=lld");
        }
        Linker::Lld
    } else {
        return;
    };
    if matches!(linker, Linker::Lld)
        && target_env == "gnu"
        && var("HOST") == var("TARGET")
        && glibc_minor().is_some_and(|minor| minor >= 36)
    {
        println!("cargo:rustc-link-arg-bins=-Wl,-z,pack-relative-relocs");
    }
    let order = var("ADA_SYMBOL_ORDER");
    if !order.is_empty() {
        println!("cargo:rerun-if-changed={order}");
    }
    if Path::new(&order).is_file() {
        match linker {
            Linker::Lld => {
                println!("cargo:rustc-link-arg-bins=-Wl,--symbol-ordering-file={order}");
                println!("cargo:rustc-link-arg-bins=-Wl,--no-warn-symbol-ordering");
            }
            Linker::Ld64 => println!("cargo:rustc-link-arg-bins=-Wl,-order_file,{order}"),
        }
        // the arguments don't change with the file, so cargo wouldn't link again: its hash does
        let mut h = DefaultHasher::new();
        fs::read(&order).unwrap_or_default().hash(&mut h);
        println!("cargo:rustc-env=ADA_SYMBOL_ORDER_HASH={:016x}", h.finish());
    }
    let map = var("ADA_LINK_MAP");
    if !map.is_empty() {
        match linker {
            Linker::Lld => {
                println!("cargo:rustc-link-arg-bins=-Wl,-Map={map}");
                println!("cargo:rustc-link-arg-bins=-Wl,--no-demangle");
            }
            Linker::Ld64 => println!("cargo:rustc-link-arg-bins=-Wl,-map,{map}"),
        }
    }
}
