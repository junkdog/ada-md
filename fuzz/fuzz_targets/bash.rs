//! G-003: bash's scanner (`ada_parse::lex` on `Lang::Bash`) against what it replaced, syntect on
//! bat's `Bourne Again Shell` grammar, as the `make` target checks Makefiles (`lexer_parity`):
//! on every byte of every line, the scope a theme's rule follows must be syntect's, for as long
//! as the scanner's state holds its whole stack.
//!
//! Lines are cut to 256 bytes, which keeps Oniguruma's backtracking short.
//!
//! ```zsh
//! cargo +nightly fuzz run -s none bash -- -dict=fuzz/bash.dict -max_len=4096
//! ```

#![no_main]

mod lexer_parity;

use ada_parse::Lang;
use lexer_parity::Stacks;
use libfuzzer_sys::fuzz_target;

/// The longest line compared.
const LINE: usize = 256;

fuzz_target!(|data: &[u8]| lexer_parity::check(Lang::Bash, data, LINE, Stacks::Rule));
