//! Arbitrary bytes → a mermaid diagram (ADR-35) → drawn at an arbitrary width, with the box or
//! the ASCII glyphs. Nothing may panic, and a drawn row is never wider than its canvas.
//!
//! ```zsh
//! cargo +nightly fuzz run diagram -- -dict=fuzz/diagram.dict -max_total_time=600
//! ```

#![no_main]

use ada::diagram::{draw, parse};
use ada::theme::{Glyphs, TableBorder};
use libfuzzer_sys::fuzz_target;
use unicode_width::UnicodeWidthStr;

fuzz_target!(|data: &[u8]| {
    let [width, flags, rest @ ..] = data else {
        return;
    };
    let text = String::from_utf8_lossy(rest);
    let Some(diagram) = parse(&text) else {
        return;
    };
    let glyphs = Glyphs {
        table_border: if flags & 1 == 0 {
            TableBorder::Rounded
        } else {
            TableBorder::Ascii
        },
        ..Glyphs::default()
    };
    let Some(drawn) = draw(&diagram, u16::from(*width).max(1), &glyphs) else {
        return;
    };
    let canvas = &drawn.canvas;
    for row in 0..canvas.height() {
        let text = canvas.row_text(row);
        assert!(text.width() <= canvas.width(), "{text:?}");
    }
    if let Some(header) = drawn.header {
        assert!(header.end <= canvas.height());
    }
});
