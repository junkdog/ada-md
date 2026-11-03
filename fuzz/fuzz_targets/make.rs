//! G-002: Makefile's scanner (`ada_parse::lex` on `Lang::Makefile`) against what it replaced,
//! syntect on bat's grammar. Every line, expanded as the highlighter expands it, must give the
//! scope a theme's rule follows on every byte as syntect does, with each one's state carried
//! from line to line, for as long as the scanner's state holds its whole stack (`lexer_parity`).
//!
//! Lines are cut to 96 bytes, as the grammar backtracks ~n³ on unbalanced parens before a `:`
//! (seconds a line from ~300 bytes), and past ~400 Oniguruma's retry limit gives up and
//! changes its answer.
//!
//! ```zsh
//! cargo +nightly fuzz run -s none make -- -dict=fuzz/make.dict -max_len=4096
//! ```

#![no_main]

mod lexer_parity;

use ada_parse::Lang;
use lexer_parity::Stacks;
use libfuzzer_sys::fuzz_target;

/// The longest line compared.
const LINE: usize = 96;

fuzz_target!(|data: &[u8]| lexer_parity::check(Lang::Makefile, data, LINE, Stacks::Rule));
