//! Symbol names on stdin, one per line, to the same lines demangled without their crate hashes
//! (MEM-08): `ada::render::draw`, not `_RNvNtCs1a2b3c_3ada6render4draw`. The hashes change with
//! the toolchain, the target and the package's version; the rest holds, so `order.txt` keeps
//! these. So does a local copy's name without the `.1505` LTO numbers it by, which changes with
//! any code around it (MEM-07): its copies all take the one name. A name rustc didn't mangle
//! (onig's C functions, libc's) passes through as it is.
//!
//! ```zsh
//! cargo build --release --manifest-path bench/order/demangle/Cargo.toml
//! bench/order/demangle/target/release/demangle < names.txt
//! ```

use std::io::{self, BufRead, BufWriter, Write};

fn main() -> io::Result<()> {
    let mut out = BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let name = line?;
        match rustc_demangle::try_demangle(&name) {
            // `{:#}` leaves out the hashes
            Ok(d) => writeln!(out, "{}", unnumbered(&format!("{d:#}")))?,
            Err(_) => writeln!(out, "{name}")?,
        }
    }
    out.flush()
}

/// `name` without its trailing `.NNN` parts.
fn unnumbered(mut name: &str) -> &str {
    while let Some((head, n)) = name.rsplit_once('.')
        && !n.is_empty()
        && n.bytes().all(|b| b.is_ascii_digit())
    {
        name = head;
    }
    name
}
