//! G-003: the shell lexer (`ada::make` on bash) against what it replaced, syntect on bat's
//! `Bourne Again Shell` grammar, as the `make` target checks Makefiles (`lexer_parity`).
//!
//! Lines are cut to 256 bytes, which keeps Oniguruma's backtracking short.
//!
//! ```zsh
//! cargo +nightly fuzz run -s none bash -- -dict=fuzz/bash.dict -max_len=4096
//! ```

#![no_main]

mod lexer_parity;

use ada::make::Grammar;
use libfuzzer_sys::fuzz_target;

/// The longest line compared.
const LINE: usize = 256;

fuzz_target!(|data: &[u8]| lexer_parity::check(Grammar::Bash, data, LINE));
