//! The functions in `.text` of a linker map (`ADA_LINK_MAP`, build.rs), lld's or ld64's, and
//! order.txt's names placed onto them. order.txt holds the names demangled (MEM-08): the mangled
//! names' crate hashes change with the toolchain, the target and ada's version, so `place` maps
//! the order onto a build's own names, for its second link. `bench/order/symbols.py` reads the
//! same maps for the traces.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::ops::Range;
use std::path::Path;

/// A function in `.text`: its address, size and mangled name, as the map has them.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Symbol {
    addr: u64,
    size: u64,
    name: String,
}

/// order.txt's names as a build has them.
pub struct Placement {
    /// every function a name stands for (a generic instantiated in several crates, LTO's copies of
    /// a local function), in the map's address order, by order.txt's order
    pub names: Vec<String>,
    /// how many of order.txt's names the build has
    pub found: usize,
    /// how many names order.txt has
    pub total: usize,
}

/// order.txt's names as the build mapped in `map` has them. Mach-O names carry a leading `_`
/// (`__RNv…`, `_onig_search`): matched without it, placed with it, as ld64's order file takes them.
pub fn place(map: &Path, order: &Path) -> Result<Placement, String> {
    let bytes = fs::read(map).map_err(|e| format!("{}: {e}", map.display()))?;
    let order = fs::read_to_string(order).map_err(|e| format!("{}: {e}", order.display()))?;
    let syms = text_symbols(&bytes)?;
    let mach_o = is_ld64(&bytes);
    let mut funcs: HashMap<String, Vec<&str>> = HashMap::new();
    for sym in &syms {
        let bare = if mach_o {
            sym.name.strip_prefix('_').unwrap_or(&sym.name)
        } else {
            &sym.name
        };
        funcs.entry(demangle(bare)).or_default().push(&sym.name);
    }
    let names: Vec<&str> = order
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let mut seen = HashSet::new();
    let mut placed = Vec::new();
    for mangled in names.iter().filter_map(|n| funcs.get(*n)).flatten() {
        if seen.insert(*mangled) {
            placed.push((*mangled).to_owned());
        }
    }
    Ok(Placement {
        names: placed,
        found: names.iter().filter(|n| funcs.contains_key(**n)).count(),
        total: names.len(),
    })
}

/// `name` demangled without its crate hashes or LTO's `.NNN`: `ada::render::draw`, not
/// `_RNvNtCs1a2b3c_3ada6render4draw`. The hashes change with the toolchain, the target and the
/// package's version; the rest holds, so order.txt keeps these. So does a local copy's name
/// without the `.1505` LTO numbers it by, which changes with any code around it (MEM-07): its
/// copies all take the one name. A name rustc didn't mangle (onig's C functions, libc's) passes
/// through as it is.
pub fn demangle(name: &str) -> String {
    match rustc_demangle::try_demangle(name) {
        // `{:#}` leaves out the hashes
        Ok(d) => unnumbered(&format!("{d:#}")).to_owned(),
        Err(_) => name.to_owned(),
    }
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

fn is_ld64(map: &[u8]) -> bool {
    map.starts_with(b"# Path: ")
}

/// The functions in the map's `.text`, sorted. The map holds the binary's own names whether or not
/// it was stripped. Its bytes aren't all UTF-8: ld64's lists C string literals by their contents.
fn text_symbols(map: &[u8]) -> Result<Vec<Symbol>, String> {
    let lines: Vec<&[u8]> = map
        .split(|&b| b == b'\n')
        .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
        .collect();
    let mut syms = if is_ld64(map) {
        ld64_text_symbols(&lines)?
    } else {
        lld_text_symbols(&lines)?
    };
    syms.sort();
    Ok(syms)
}

/// lld's map: a header naming the columns, then a row per output section (`.text` under `Out`),
/// per input section (under `In`) and per symbol (under `Symbol`): address, LMA, size, alignment,
/// name.
fn lld_text_symbols(lines: &[&[u8]]) -> Result<Vec<Symbol>, String> {
    let head = lines.first().copied().unwrap_or_default();
    let column = |name: &str| {
        let name = name.as_bytes();
        head.windows(name.len())
            .position(|w| w == name)
            .ok_or_else(|| format!("lld map: no {} column", String::from_utf8_lossy(name)))
    };
    let (sym_col, out_col) = (column("Symbol")?, column("Out")?);
    let mut syms = Vec::new();
    let mut in_text = false;
    for line in &lines[1..] {
        let fields = fields(line);
        let Some(&(col, name)) = fields.get(4) else {
            continue;
        };
        if col <= out_col + 2 {
            // an output section
            in_text = name == b".text";
        } else if in_text && col + 2 >= sym_col && !name.starts_with(b"$") {
            // not AArch64's `$x`, `$d`: mapping symbols
            syms.push(Symbol {
                addr: hex(fields[0].1)?,
                size: hex(fields[2].1)?,
                name: lossy(name),
            });
        }
    }
    Ok(syms)
}

/// ld64's map: `# Sections:` rows (address, size, segment, section), then `# Symbols:` rows
/// (address, size, `[ n]` for the object file, name), then `# Dead Stripped Symbols:`. A name can
/// hold spaces (a literal's).
fn ld64_text_symbols(lines: &[&[u8]]) -> Result<Vec<Symbol>, String> {
    let at = |head: &str| {
        lines
            .iter()
            .position(|l| *l == head.as_bytes())
            .ok_or_else(|| format!("ld64 map: no `{head}`"))
    };
    let (sections, symbols) = (at("# Sections:")?, at("# Symbols:")?);
    let mut text: Option<Range<u64>> = None;
    for line in lines.get(sections + 2..symbols).unwrap_or_default() {
        let p: Vec<&[u8]> = fields(line).into_iter().map(|(_, f)| f).collect();
        if p.len() >= 4 && p[2] == b"__TEXT" && p[3] == b"__text" {
            let lo = hex(p[0])?;
            text = Some(lo..lo + hex(p[1])?);
        }
    }
    let text = text.ok_or("ld64 map: no __TEXT,__text section")?;
    let mut syms = Vec::new();
    for line in lines.get(symbols + 2..).unwrap_or_default() {
        if line.starts_with(b"#") {
            break;
        }
        if let Some((addr, size, name)) = ld64_symbol(line)
            && text.contains(&addr)
        {
            syms.push(Symbol {
                addr,
                size,
                name: lossy(name),
            });
        }
    }
    Ok(syms)
}

/// An ld64 `# Symbols:` row: `0xADDR  0xSIZE  [  n] name`.
fn ld64_symbol(line: &[u8]) -> Option<(u64, u64, &[u8])> {
    let (addr, rest) = prefixed_hex(line)?;
    let (size, rest) = prefixed_hex(spaced(rest)?)?;
    let rest = spaced(rest)?.strip_prefix(b"[")?.trim_ascii_start();
    let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
    let rest = rest[digits..].strip_prefix(b"]").filter(|_| digits > 0)?;
    let (space, name) = rest.split_first()?;
    space.is_ascii_whitespace().then_some((addr, size, name))
}

/// `line` past the whitespace it starts with, if it starts with any.
fn spaced(line: &[u8]) -> Option<&[u8]> {
    let rest = line.trim_ascii_start();
    (rest.len() < line.len()).then_some(rest)
}

/// A `0x…` number at the start of `line`, and the rest of it.
fn prefixed_hex(line: &[u8]) -> Option<(u64, &[u8])> {
    let line = line.strip_prefix(b"0x")?;
    let n = line.iter().take_while(|b| b.is_ascii_hexdigit()).count();
    let value = u64::from_str_radix(str::from_utf8(&line[..n]).ok()?, 16).ok()?;
    Some((value, &line[n..]))
}

/// The whitespace-separated fields of `line`, each with the column it starts at.
fn fields(line: &[u8]) -> Vec<(usize, &[u8])> {
    let mut fields = Vec::new();
    let mut start = None;
    for (i, b) in line.iter().enumerate() {
        match (b.is_ascii_whitespace(), start) {
            (true, Some(s)) => {
                fields.push((s, &line[s..i]));
                start = None;
            }
            (false, None) => start = Some(i),
            _ => {}
        }
    }
    if let Some(s) = start {
        fields.push((s, &line[s..]));
    }
    fields
}

fn hex(field: &[u8]) -> Result<u64, String> {
    let digits = field.strip_prefix(b"0x").unwrap_or(field);
    str::from_utf8(digits)
        .ok()
        .and_then(|d| u64::from_str_radix(d, 16).ok())
        .ok_or_else(|| format!("map: `{}` isn't a hex number", lossy(field)))
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(map: &str) -> Vec<(u64, u64, String)> {
        text_symbols(map.as_bytes())
            .unwrap()
            .into_iter()
            .map(|s| (s.addr, s.size, s.name))
            .collect()
    }

    #[test]
    fn lld_map() {
        // a raw string: `\` and a newline would drop the header's indent, and its columns
        let map = r"             VMA              LMA     Size Align Out     In      Symbol
             2e0              2e0       1c     1 .interp
             2e0              2e0       1c     1         <internal>:(.interp)
          23c600           23c600     1000    64 .text
          23c630           23c630      60a    16         ada.o:(.text.main)
          23c630           23c630      60a     1                 main
          23c600           23c600       26    16         Scrt1.o:(.text)
          23c600           23c600       26     1                 _start
          23c600           23c600        0     1                 $x
          23d600           23d600       10    16 .rodata
          23d600           23d600       10     1                 not_text
";
        assert_eq!(
            names(map),
            [
                (0x23c600, 0x26, "_start".into()),
                (0x23c630, 0x60a, "main".into())
            ]
        );
    }

    #[test]
    fn ld64_map() {
        let map = "\
# Path: ada
# Arch: arm64
# Sections:
# Address\tSize    \tSegment\tSection
0x100000500\t0x00001000\t__TEXT\t__text
0x100001500\t0x00000100\t__TEXT\t__cstring
# Symbols:
# Address\tSize    \tFile  Name
0x100000540\t0x00000020\t[  2] __RNvCs1a2b3c_3ada4main
0x100000500\t0x00000040\t[  1] _onig_search
0x100001500\t0x00000006\t[  3] literal string: a b
# Dead Stripped Symbols:
<<dead>> \t0x00000008\t[  2] _gone
";
        assert_eq!(
            names(map),
            [
                (0x100000500, 0x40, "_onig_search".into()),
                (0x100000540, 0x20, "__RNvCs1a2b3c_3ada4main".into()),
            ]
        );
    }

    #[test]
    fn demangled_without_hashes_or_numbers() {
        assert_eq!(demangle("_RNvCs1a2b3c_3ada4main"), "ada::main");
        assert_eq!(
            demangle("_ZN3ada6render4draw17h0123456789abcdefE.1505"),
            "ada::render::draw"
        );
        assert_eq!(demangle("onig_search"), "onig_search");
    }
}
