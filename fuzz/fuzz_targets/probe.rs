//! The terminal probe's reply parser (CS-6): `probe::Replies` reads the tty's raw bytes after the
//! queries, replies and keys typed meanwhile mixed together, in whatever pieces the reads return.
//! An input that starts with 0 is raw bytes: the parser must not panic, and the pieces must not
//! matter, so the bytes fed whole, a byte at a time and cut where the input says read the same
//! replies and keys. Any other input plants replies, keys and noise:
//!   - each reply, kitty's (OK, an error, another image's), the device attributes, the cell's and
//!     the text area's size, some twice (the last counts), in any order, the device status last
//!   - between them, keys typed (printable ASCII, Enter, Tab, Backspace, Ctrl-letters), bytes that
//!     aren't keys (UTF-8, NUL), and escape sequences that are dropped: arrows, SS3, Alt-keys,
//!     unknown CSIs, OSC, DCS, a CSI cut short by the next sequence
//!   - and the bytes after the device status, read the same way
//!
//! The parser must find exactly the replies planted and pass on exactly the keys, in order.
//!
//! ```zsh
//! cargo +nightly fuzz run -s none -a probe -- -max_len=4096
//! ```

#![no_main]

use ada::probe::{Found, Replies};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use libfuzzer_sys::arbitrary::{Result, Unstructured};
use libfuzzer_sys::fuzz_target;

/// The parser drops an unfinished sequence past this many bytes (`probe::MAX_PENDING`); inputs
/// shorter than it can't reach that, so any cut reads the same.
const MAX_PENDING: usize = 4096;

/// Feed `bytes` in pieces ending at `cuts`.
fn read(bytes: &[u8], cuts: impl Iterator<Item = usize>) -> (Found, bool, Vec<KeyEvent>) {
    let mut r = Replies::new();
    let mut keys = Vec::new();
    let mut at = 0;
    for cut in cuts.chain([bytes.len()]) {
        let cut = cut.clamp(at, bytes.len());
        r.feed(&bytes[at..cut], &mut keys);
        at = cut;
    }
    let done = r.done();
    (r.found(), done, keys)
}

/// The key a byte typed alone is in raw mode, if it's one the probe passes on.
fn key(b: u8) -> Option<KeyEvent> {
    let (code, mods) = match b {
        b'\r' => (KeyCode::Enter, KeyModifiers::NONE),
        b'\t' => (KeyCode::Tab, KeyModifiers::NONE),
        0x7f => (KeyCode::Backspace, KeyModifiers::NONE),
        0x01..=0x1a => (
            KeyCode::Char(char::from(b'a' + b - 1)),
            KeyModifiers::CONTROL,
        ),
        b'A'..=b'Z' => (KeyCode::Char(char::from(b)), KeyModifiers::SHIFT),
        b' '..=b'~' => (KeyCode::Char(char::from(b)), KeyModifiers::NONE),
        _ => return None,
    };
    Some(KeyEvent::new(code, mods))
}

fn nums(n: &[u16]) -> String {
    n.iter().map(u16::to_string).collect::<Vec<_>>().join(";")
}

/// Bytes planted between the replies, the keys among them added to `keys`. Each piece ends where
/// the next may start anything: a cut-short CSI, a lone Esc, is followed by another sequence.
fn between(u: &mut Unstructured, out: &mut Vec<u8>, keys: &mut Vec<KeyEvent>) -> Result<()> {
    for _ in 0..u.int_in_range(0..=4)? {
        match u.int_in_range(0..=9)? {
            0..=3 => {
                let b = u.int_in_range(0x01..=0x7f)?;
                if b == 0x1b {
                    continue;
                }
                out.push(b);
                keys.extend(key(b));
            }
            // not keys: a NUL, the rest of the C0 controls past Ctrl-Z, UTF-8
            4 => out.push(*u.choose(&[0x00, 0x1c, 0x1d, 0x1e, 0x1f])?),
            5 => out.extend_from_slice("é→🎉".as_bytes()),
            // arrows, F-keys, Alt-keys
            6 => out.extend_from_slice(u.choose(&[
                &b"\x1b[A"[..],
                b"\x1b[1;5D",
                b"\x1b[15~",
                b"\x1bOP",
                b"\x1bOA",
                b"\x1bx",
                b"\x1b\x7f",
                b"\x1b[0;5R",
            ])?),
            // strings: OSC by BEL and ST, DCS, PM, SOS, an APC that isn't kitty's
            7 => out.extend_from_slice(u.choose(&[
                &b"\x1b]11;rgb:0/0/0\x07"[..],
                b"\x1b]10;rgb:f/f/f\x1b\\",
                b"\x1bP1$r0m\x1b\\",
                b"\x1b^x\x1b\\",
                b"\x1bXy\x1b\\",
                b"\x1b_x=1;OK\x1b\\",
            ])?),
            // cut short by the next sequence: a CSI's start, a lone Esc
            8 => {
                out.extend_from_slice(u.choose(&[
                    &b"\x1b[6;20"[..],
                    b"\x1b[?62;",
                    b"\x1b[",
                    b"\x1b",
                ])?);
                out.extend_from_slice(b"\x1b[A");
            }
            _ => out.extend_from_slice(b"\x1b[999;1;2t"),
        }
    }
    Ok(())
}

fn planted(u: &mut Unstructured) -> Result<()> {
    let mut want = Found::default();
    let (mut out, mut keys) = (Vec::new(), Vec::new());
    let replies = u.int_in_range(0..=8)?;
    between(u, &mut out, &mut keys)?;
    for _ in 0..replies {
        match u.int_in_range(0..=4)? {
            0 => {
                let (ok, end) = (u.arbitrary::<bool>()?, u.arbitrary::<bool>()?);
                let message = if ok { "OK" } else { "ENOTSUPPORTED:no" };
                let keys = *u.choose(&["i=31", "i=31,p=1", "p=2,i=31"])?;
                out.extend_from_slice(format!("\x1b_G{keys};{message}").as_bytes());
                // ST, or BEL, which kitty doesn't send but the string's end is the same
                out.extend_from_slice(if end { b"\x1b\\" } else { b"\x07" });
                want.kitty = Some(ok);
            }
            1 => {
                // another image's reply isn't the probe's
                out.extend_from_slice(b"\x1b_Gi=7;OK\x1b\\");
            }
            2 => {
                let n: Vec<u16> = (0..u.int_in_range(1..=9)?)
                    .map(|_| u.arbitrary())
                    .collect::<Result<_>>()?;
                out.extend_from_slice(format!("\x1b[?{}c", nums(&n)).as_bytes());
                want.attributes = Some(n);
            }
            3 => {
                let (w, h): (u16, u16) = u.arbitrary()?;
                out.extend_from_slice(format!("\x1b[6;{h};{w}t").as_bytes());
                want.cell = Some((w, h));
            }
            _ => {
                let (w, h): (u16, u16) = u.arbitrary()?;
                out.extend_from_slice(format!("\x1b[4;{h};{w}t").as_bytes());
                want.text_area = Some((w, h));
            }
        }
        between(u, &mut out, &mut keys)?;
    }
    out.extend_from_slice(if u.arbitrary()? {
        b"\x1b[0n"
    } else {
        b"\x1b[3n"
    });
    // read after the end too: crossterm never sees these
    between(u, &mut out, &mut keys)?;
    let cuts: Vec<usize> = (0..u.int_in_range(0..=8)?)
        .map(|_| u.int_in_range(0..=out.len()))
        .collect::<Result<_>>()?;
    let mut sorted = cuts.clone();
    sorted.sort_unstable();
    let (found, done, got) = read(&out, sorted.into_iter());
    assert!(done, "no end in {:?}", out.escape_ascii().to_string());
    assert_eq!(found, want, "{:?}", out.escape_ascii().to_string());
    assert_eq!(got, keys, "{:?}", out.escape_ascii().to_string());
    Ok(())
}

fuzz_target!(|data: &[u8]| {
    match data {
        [0, cuts @ ..] if data.len() <= MAX_PENDING => {
            // the first bytes up to a 0 are the cuts; the rest is what the tty gave
            let split = cuts.iter().position(|&b| b == 0).unwrap_or(cuts.len());
            let (cuts, bytes) = (&cuts[..split], cuts.get(split + 1..).unwrap_or(&[]));
            let whole = read(bytes, std::iter::empty());
            let one = read(bytes, 1..bytes.len());
            let mut at = 0;
            let cut = read(
                bytes,
                cuts.iter().map(|&c| {
                    at += usize::from(c);
                    at
                }),
            );
            assert_eq!(
                whole,
                one,
                "a byte at a time: {:?}",
                bytes.escape_ascii().to_string()
            );
            assert_eq!(
                whole,
                cut,
                "cut at {cuts:?}: {:?}",
                bytes.escape_ascii().to_string()
            );
            // keys come only from bytes typed alone, so never more of them than bytes
            assert!(whole.2.len() <= bytes.len());
        }
        _ => {
            let _ = planted(&mut Unstructured::new(data));
        }
    }
});
