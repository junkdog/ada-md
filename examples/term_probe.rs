//! CS-2's terminal survey (the clean-slate plan): in the terminal it runs in, what
//! ratatui-image's `Picker::from_query_stdio` finds (what ada uses today), then the raw replies
//! to ada's own probe (`probe::QUERY`) and what `probe::Replies` reads from them. The report goes
//! to a file, since the terminal's window may close when it ends.
//!
//! ```zsh
//! cargo build --release --example term_probe
//! foot -e target/release/examples/term_probe /tmp/probe-foot.txt
//! ```
//!
//! Keys typed while it waits are in the report too (`keys`), as the probe passes them on.

use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use ada::probe::{QUERY, Replies};
use ratatui::crossterm::terminal::{self, disable_raw_mode, enable_raw_mode};
use ratatui_image::picker::Picker;

/// How long the probe waits for the `CSI 5n` reply, as CS-2 plans it.
const DEADLINE: Duration = Duration::from_secs(2);

/// Bytes as text: printable ASCII as it is, ESC as `\e`, the rest as `\xNN`.
fn escaped(bytes: &[u8]) -> String {
    let mut s = String::new();
    for &b in bytes {
        match b {
            0x1b => s.push_str("\\e"),
            b'\\' => s.push_str("\\\\"),
            0x20..=0x7e => s.push(b as char),
            _ => write!(s, "\\x{b:02x}").unwrap(),
        }
    }
    s
}

fn main() {
    let out = std::env::args().nth(1).expect("usage: term_probe OUT.txt");
    let mut report = String::new();
    for var in [
        "TERM",
        "TERM_PROGRAM",
        "TERM_PROGRAM_VERSION",
        "KONSOLE_VERSION",
        "VTE_VERSION",
        "WEZTERM_EXECUTABLE",
        "KITTY_WINDOW_ID",
        "GHOSTTY_RESOURCES_DIR",
        "TMUX",
    ] {
        if let Ok(v) = std::env::var(var) {
            writeln!(report, "env {var}={v}").unwrap();
        }
    }
    if let Ok(s) = terminal::window_size() {
        writeln!(
            report,
            "winsize {}x{} cells, {}x{} px",
            s.columns, s.rows, s.width, s.height
        )
        .unwrap();
    }

    enable_raw_mode().expect("raw mode");

    let t = Instant::now();
    match Picker::from_query_stdio() {
        Ok(p) => writeln!(
            report,
            "picker {:?}, font {}x{}, tmux {}, {:?} ({} ms)",
            p.protocol_type(),
            p.font_size().width,
            p.font_size().height,
            p.tmux_detected(),
            p.capabilities(),
            t.elapsed().as_millis()
        ),
        Err(e) => writeln!(report, "picker error: {e}"),
    }
    .unwrap();

    let mut tty = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .expect("/dev/tty");
    let mut reader = tty.try_clone().expect("/dev/tty");
    let (tx, rx) = mpsc::channel();
    // left blocked in read() at the end; the process exits under it
    thread::spawn(move || {
        let mut buf = [0u8; 1024];
        while let Ok(n @ 1..) = reader.read(&mut buf) {
            if tx.send((Instant::now(), buf[..n].to_vec())).is_err() {
                return;
            }
        }
    });

    let t = Instant::now();
    tty.write_all(QUERY)
        .and_then(|()| tty.flush())
        .expect("query");
    let mut replies = Replies::new();
    let (mut raw, mut keys, mut reads) = (Vec::new(), Vec::new(), Vec::new());
    while !replies.done() {
        let Some(left) = DEADLINE.checked_sub(t.elapsed()) else {
            break;
        };
        let Ok((at, chunk)) = rx.recv_timeout(left) else {
            break;
        };
        reads.push((at.duration_since(t).as_micros(), chunk.len()));
        raw.extend_from_slice(&chunk);
        replies.feed(&chunk, &mut keys);
    }
    let elapsed = t.elapsed();
    disable_raw_mode().expect("cooked mode");

    writeln!(report, "query {}", escaped(QUERY)).unwrap();
    writeln!(report, "raw {}", escaped(&raw)).unwrap();
    writeln!(report, "reads (µs, bytes) {reads:?}").unwrap();
    let done = replies.done();
    writeln!(report, "done {done} after {} µs", elapsed.as_micros()).unwrap();
    let found = replies.found();
    writeln!(report, "found {found:?}, sixel {}", found.sixel()).unwrap();
    writeln!(report, "keys {keys:?}").unwrap();
    std::fs::write(&out, &report).expect("write the report");
    print!("{report}");
}
