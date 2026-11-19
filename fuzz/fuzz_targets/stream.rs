//! `--print` of a big text file, or of code ada lexes itself, a piece at a time (O-52) must write
//! what the whole print writes, byte for byte (F-07). The input is a file of one of the kinds a
//! lexer streams: text, a log, JSON or INI, highlighted on the painting threads, or a
//! language whose state the reader carries from piece to piece (PRN-003), a diff's (its file's
//! language and both sides' states, X-174; a change block's words, read ahead past a cut,
//! FEAT-023) among them. The reader cuts it at
//! fuzzed limits, far under the real 128 KiB and 1,024 lines, so a piece's edges are where the
//! fuzzer's short inputs are: a cut after an empty line, the file's last empty line joining the
//! piece before it, a JSON comment or a heredoc, string or block comment open across a cut, CRLF
//! and invalid UTF-8 at a cut, a line longer than a piece. Any width, with or without line numbers,
//! and any split of what `start` reads first from the rest. The same text through
//! `--color-only` must come out a line for each line in, each its text. Nothing may panic.
//!
//! ```zsh
//! cargo +nightly fuzz run -s none -a stream -- -dict=fuzz/tokens.dict -max_len=65536
//! ```

#![no_main]

use std::path::Path;
use std::sync::OnceLock;

use ada::doc::code_lines;
use ada::filetype::FileType;
use ada::layout::{LayoutParams, LineNumbers, TableMode, expand_tabs};
use ada::print::stream::{Limits, write_bytes};
use ada::print::{Options, render, write_color_only};
use ada::theme::Theme;
use libfuzzer_sys::fuzz_target;

const LANGS: [Option<&str>; 13] = [
    None,
    Some("log"),
    Some("JSON"),
    // carried by the reader since X-174, its lines in their file's language
    Some("Diff"),
    Some("INI"),
    // carried by the reader (PRN-003)
    Some("Bourne Again Shell (bash)"),
    Some("Makefile"),
    Some("Rust"),
    Some("Python"),
    Some("YAML"),
    Some("TOML"),
    Some("HTML"),
    Some("C++"),
];

fn theme() -> &'static Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(Theme::builtin)
}

fuzz_target!(|data: &[u8]| {
    let [lang, piece, lines, width, flags, first, text @ ..] = data else {
        return;
    };
    let lang = LANGS[*lang as usize % LANGS.len()];
    // 1 to ~4 KiB; a few lines, or as many as the real reader allows
    let piece = 1 + usize::from(*piece) * usize::from(*piece) / 16;
    let lines = if *lines >= 192 {
        1024
    } else {
        1 + usize::from(*lines % 64)
    };
    let params = LayoutParams {
        width: 4 + u16::from(*width % 160),
        max_width: None,
        table_mode: TableMode::Auto,
        code_wrap: flags & 2 == 0,
        code_labels: flags & 4 != 0,
        // a file of code printed on the terminal's background, as `ada -p` prints one
        bare: flags & 8 != 0,
        line_numbers: if flags & 1 == 1 {
            LineNumbers::Code
        } else {
            LineNumbers::Off
        },
        glyphs_gen: theme().glyphs.generation(),
        images: None,
    };
    let first = text.len() * usize::from(*first) / 255;

    let filetype = FileType::Text { lang };
    let doc = filetype.parse(Path::new("fuzz"), &String::from_utf8_lossy(text));
    let whole = render(&doc, theme(), filetype.params(params), Options::default());
    let mut out = Vec::new();
    let limits = Limits { piece, lines };
    write_bytes(text, lang, first, limits, theme(), params, &mut out).expect("write");
    let streamed = String::from_utf8(out).expect("streamed output is UTF-8");
    if streamed != whole {
        let (a, b): (Vec<_>, Vec<_>) = (whole.lines().collect(), streamed.lines().collect());
        let at = a
            .iter()
            .zip(&b)
            .position(|(a, b)| a != b)
            .unwrap_or(a.len().min(b.len()));
        panic!(
            "{lang:?}, {} bytes, piece {piece}, lines {lines}, first {first}, {params:?}: whole {} rows, \
             streamed {}; first difference at row {at}:\n{:?}\n{:?}",
            text.len(),
            a.len(),
            b.len(),
            a.get(at),
            b.get(at),
        );
    }
    color_only(&String::from_utf8_lossy(text), lang);
});

/// `--color-only` (X-174, FEAT-023), git's `interactive.diffFilter`: one line out for each line
/// in, as `git add -p` pairs them up, and each line its text in colors, tabs expanded (blanks at
/// its end may go, where nothing paints them).
fn color_only(text: &str, lang: Option<&str>) {
    static THEME: OnceLock<Theme> = OnceLock::new();
    let theme = THEME.get_or_init(|| theme().with_bg(false).without_code_bg());
    let mut out = Vec::new();
    write_color_only(text, lang, theme, &mut out).expect("write");
    let out = String::from_utf8(out).expect("--color-only's output is UTF-8");
    let lines: Vec<&str> = code_lines(text).collect();
    let written: Vec<&str> = out.split_terminator('\n').collect();
    assert_eq!(
        lines.len(),
        written.len(),
        "{lang:?}: {} lines in, {} out",
        lines.len(),
        written.len()
    );
    for (n, (line, written)) in lines.iter().zip(&written).enumerate() {
        // a line's own escapes can't be told from the ones written
        if line.contains('\x1b') {
            continue;
        }
        let shown = without_sgr(written);
        assert_eq!(
            expand_tabs(line).trim_end_matches(' '),
            shown.trim_end_matches(' '),
            "{lang:?}, line {n}: {written:?}"
        );
    }
}

/// `line` without its SGR sequences (`ESC [ … m`).
fn without_sgr(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find("\x1b[") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 2..];
        let end = after.find('m').expect("an SGR sequence ends");
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}
