//! `--print` of a big log, JSON or text file a piece at a time (O-52) must write what the whole
//! print writes, byte for byte (F-07). The input is a file of one of the three kinds a lexer
//! streams; the reader cuts it at fuzzed limits, far under the real 128 KiB and 1,024 lines, so a
//! piece's edges are where the fuzzer's short inputs are: a cut after an empty line, the file's
//! last empty line joining the piece before it, a JSON comment open across a cut, CRLF and
//! invalid UTF-8 at a cut, a line longer than a piece. Any width, with or without line numbers,
//! and any split of what `start` reads first from the rest. Nothing may panic.
//!
//! ```zsh
//! cargo +nightly fuzz run -s none -a stream -- -dict=fuzz/tokens.dict -max_len=65536
//! ```

#![no_main]

use std::path::Path;
use std::sync::OnceLock;

use ada::filetype::FileType;
use ada::layout::{LayoutParams, LineNumbers, TableMode};
use ada::print::stream::{Limits, write_bytes};
use ada::print::{Options, render};
use ada::theme::Theme;
use libfuzzer_sys::fuzz_target;

const LANGS: [Option<&str>; 3] = [None, Some("log"), Some("JSON")];

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
});
