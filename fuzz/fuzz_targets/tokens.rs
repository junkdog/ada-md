//! The scanners that took the place of bat's grammars: log lines (O-43, `logs`) and JSON lines
//! (O-48, `json`), each given every line of the input, JSON with the `/* */` comment state one
//! line leaves the next. Their spans go to `highlight`'s runs, which slice the line at them: they
//! must come in order, not overlap, not be empty, and start and end on characters of the line.
//! Nothing may panic, and a long line must not take long. An input that starts with a NUL is a
//! size and a piece: the piece is repeated to 1–64 KiB, since a scan that reads a run again from
//! each word only shows on long lines, which libFuzzer grows to slowly (O-50).
//!
//! ```zsh
//! cargo +nightly fuzz run -s none -a tokens -- -dict=fuzz/tokens.dict -max_len=65536
//! ```

#![no_main]

use ada::{json, logs};
use libfuzzer_sys::fuzz_target;

/// Spans `(start, end)` of `line`, in order, as `highlight` takes them.
fn check(what: &str, line: &str, spans: impl Iterator<Item = (u32, u32)>) {
    let mut at = 0;
    for (start, end) in spans {
        let (s, e) = (start as usize, end as usize);
        assert!(
            at <= s && s < e && e <= line.len(),
            "{what}: span {s}..{e} after {at}, in {} bytes: {line:?}",
            line.len()
        );
        assert!(
            line.is_char_boundary(s) && line.is_char_boundary(e),
            "{what}: span {s}..{e} splits a character: {line:?}"
        );
        at = e;
    }
}

fuzz_target!(|data: &[u8]| {
    let repeated;
    let data = match data {
        [0, size, piece @ ..] if !piece.is_empty() => {
            let len = 1 << (10 + size % 7);
            repeated = piece.repeat(len / piece.len() + 1);
            &repeated[..]
        }
        _ => data,
    };
    let text = String::from_utf8_lossy(data);
    let trace = std::env::var_os("ADA_FUZZ_TRACE").is_some();
    let (mut log, mut js) = (Vec::new(), Vec::new());
    let mut comment = false;
    for line in text.split('\n') {
        logs::tokens(line, &mut log);
        check("log", line, log.iter().map(|s| (s.start, s.end)));
        let open = comment;
        json::tokens(line, &mut comment, &mut js);
        check("json", line, js.iter().map(|s| (s.start, s.end)));
        if trace {
            eprintln!("{line:?}\n  log {log:?}\n  json {open} -> {comment} {js:?}");
        }
    }
});
