// Links the `ada` binary with the functions a typical run fetches first in `.text` (MEM-02), so a
// run maps ~1 MiB less of the binary: `ADA_SYMBOL_ORDER=PATH` names them, as this build's own
// symbols. `bench/order/order.txt` holds them demangled, for any toolchain and target, and
// `cargo ordered` (xtask/) builds with it: it links once with a map, maps the names onto the map's,
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
// `--version` (FEAT-013) names the commit, `-dirty` when what the build reads (src/, the manifest,
// the lockfile, this script) differs from it, and when this script last ran (`SOURCE_DATE_EPOCH`,
// when set): from git in the repository, from cargo's `.cargo_vcs_info.json` in a package (a
// crates.io install). In the repository the script then reruns when HEAD moves or src/ changes,
// which recompiles the lib too: a change to main.rs alone costs the lib's compile. Where lld or
// ld64 links, it also says whether the hot code went first: `ordered` or `unordered`.
use std::env;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// The commit being built, 7 digits and `-dirty` when what the build reads has changed since it;
/// also the files that say when it does, to rerun on.
fn commit() -> Option<String> {
    const READ: [&str; 4] = ["src", "Cargo.toml", "Cargo.lock", "build.rs"];
    let root = env::var("CARGO_MANIFEST_DIR").ok()?;
    // cargo packages the crate with the commit it was packaged from
    if let Ok(info) = fs::read_to_string(Path::new(&root).join(".cargo_vcs_info.json")) {
        let sha1 = info.split_once("\"sha1\"")?.1.split('"').nth(1)?;
        return short(sha1, info.contains("\"dirty\": true"));
    }
    // not a repository the crate sits in, as one it's vendored into
    let top = git(&["rev-parse", "--show-toplevel"])?;
    if fs::canonicalize(top).ok()? != fs::canonicalize(&root).ok()? {
        return None;
    }
    let sha1 = git(&["rev-parse", "HEAD"])?;
    let head = git(&["rev-parse", "--symbolic-full-name", "HEAD"])?;
    // a missing file reruns every build, so only the ones there: a new commit moves HEAD's ref
    // (loose, or packed by gc) and appends to HEAD's reflog
    for file in ["HEAD", "logs/HEAD", "packed-refs", head.as_str()] {
        if let Some(path) = git(&["rev-parse", "--git-path", file])
            && Path::new(&path).is_file()
        {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    for path in READ {
        println!("cargo:rerun-if-changed={path}");
    }
    let mut status = vec!["status", "--porcelain", "--untracked-files=no", "--"];
    status.extend(READ);
    let dirty = !git(&status)?.is_empty();
    short(&sha1, dirty)
}

fn short(sha1: &str, dirty: bool) -> Option<String> {
    let mark = if dirty { "-dirty" } else { "" };
    Some(format!("{}{mark}", sha1.get(..7)?))
}

/// `secs` since the epoch as `2026-10-25 14:02 UTC`.
fn utc(secs: u64) -> String {
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // days to a date in the proleptic Gregorian calendar (Howard Hinnant's `civil_from_days`)
    let z = days + 719_468;
    let (era, doe) = (z / 146_097, z % 146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    format!(
        "{year}-{month:02}-{day:02} {:02}:{:02} UTC",
        rem / 3600,
        rem % 3600 / 60
    )
}

enum Linker {
    Lld,
    Ld64,
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=ADA_SYMBOL_ORDER");
    println!("cargo:rerun-if-env-changed=ADA_LINK_MAP");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    if let Some(commit) = commit() {
        println!("cargo:rustc-env=ADA_GIT_COMMIT={commit}");
    }
    let built = env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.parse().ok())
        .or_else(|| Some(SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs()))
        .unwrap_or_default();
    println!("cargo:rustc-env=ADA_BUILD_TIME={}", utc(built));
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
    let ordered = Path::new(&order).is_file();
    let layout = if ordered { "ordered" } else { "unordered" };
    println!("cargo:rustc-env=ADA_LAYOUT={layout}");
    if ordered {
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
