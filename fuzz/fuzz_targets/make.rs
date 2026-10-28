//! G-002: the Makefile lexer (`ada::make`) against what it replaced, syntect on bat's grammar.
//! Every line, expanded as the highlighter expands it, must get the same scope stack on every
//! byte from both, with each one's state carried from line to line (`lexer_parity`).
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

use ada::make::Grammar;
use libfuzzer_sys::fuzz_target;

/// The longest line compared.
const LINE: usize = 96;

fuzz_target!(|data: &[u8]| lexer_parity::check(Grammar::Make, data, LINE));
