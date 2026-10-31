//! `cargo xtask lackey`: valgrind lackey's `--trace-mem=yes` log on stdin to each 256-byte line it
//! touched, once, in the order first touched: `I hexline` for a fetch, `D hexline` for a load,
//! store or modify, the line being the address without its last two hex digits, as lackey wrote
//! them. `bench/order/lackey.zsh` reads the trace through it. It replaces an awk filter that took
//! most of a trace's CPU (2026-10-31): lackey writes a line per access, tens of millions for one
//! print of S, and awk split, matched and hashed each as strings; the output is the same, byte for
//! byte.

use std::collections::HashSet;
use std::io::{self, BufRead, Write};

/// What one log line touched: (`b'I'` or `b'D'`, the line's hex digits), or `None` for the lines
/// that aren't accesses (valgrind's own messages).
fn access(line: &[u8]) -> Option<(u8, &[u8])> {
    let (kind, rest) = match line {
        [b'I', b' ', rest @ ..] => (b'I', rest),
        [b' ', b'L' | b'S' | b'M', b' ', rest @ ..] => (b'D', rest),
        _ => return None,
    };
    // the address field, as awk's `$2`: past the blanks, up to the next
    let rest = &rest[rest
        .iter()
        .position(|b| !is_blank(*b))
        .unwrap_or(rest.len())..];
    let field = &rest[..rest.iter().position(|b| is_blank(*b)).unwrap_or(rest.len())];
    // the address before the comma (none without one), less its last two digits
    let addr = &field[..field.iter().position(|&b| b == b',').unwrap_or(0)];
    Some((kind, &addr[..addr.len().saturating_sub(2)]))
}

fn is_blank(b: u8) -> bool {
    matches!(b, b' ' | b'\t')
}

/// The hex digits as a number, for the set: lackey writes an address one way, so a number stands
/// for its text. `None` for none, past 16 digits or on another byte, which the set keeps as text.
fn hex(digits: &[u8]) -> Option<u64> {
    if digits.is_empty() || digits.len() > 16 {
        return None;
    }
    digits.iter().try_fold(0u64, |v, &b| {
        let d = (b as char).to_digit(16)?;
        Some(v << 4 | u64::from(d))
    })
}

pub fn filter(input: impl BufRead, mut out: impl Write) -> io::Result<()> {
    let mut seen: [HashSet<u64>; 2] = Default::default();
    let mut odd: HashSet<(u8, Vec<u8>)> = HashSet::new();
    let mut input = input;
    let mut line = Vec::with_capacity(64);
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        let text = line.strip_suffix(b"\n").unwrap_or(&line);
        let Some((kind, digits)) = access(text) else {
            continue;
        };
        let new = match hex(digits) {
            Some(v) => seen[usize::from(kind == b'D')].insert(v),
            None => odd.insert((kind, digits.to_vec())),
        };
        if new {
            out.write_all(&[kind, b' '])?;
            out.write_all(digits)?;
            out.write_all(b"\n")?;
        }
    }
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(log: &str) -> String {
        let mut out = Vec::new();
        filter(log.as_bytes(), &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn keeps_each_line_once_in_first_touch_order() {
        let log = "==1== Lackey, an example Valgrind tool\n\
                   I  04016c5,3\n \
                   S 1ffefffd38,8\n\
                   I  04016c8,4\n \
                   L 1ffefffd40,8\n \
                   M 0401600,4\n\
                   I  0401700,2\n";
        // 04016c5 and 04016c8 share line 04016; the M at 0401600 is a D, apart from the I there
        assert_eq!(run(log), "I 04016\nD 1ffefffd\nD 04016\nI 04017\n");
    }

    #[test]
    fn reads_odd_lines_as_awk_did() {
        // no comma: an empty address; two digits or fewer: an empty line; other lines skipped
        let log = "I  0401\nI  12,1\n X 0401600,4\nI\tx\nI  0000,1\n";
        assert_eq!(run(log), "I \nI 00\n");
    }
}
