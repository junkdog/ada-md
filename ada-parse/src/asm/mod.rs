//! Assembly on a scanner of ada's own (X-171), from a floor probe (the same file built with
//! `ADA_PARSE_X171_SOL`): x86 and x86-64 (bat's `x86_64 Assembly`: NASM, YASM, MASM, and GAS in
//! AT&T's and Intel's syntax) and ARM (bat's `ARM Assembly`: ARM32 and AArch64, GAS, armasm and
//! IAR); since X-181 RISC-V's and MIPS's (GAS's, MARS's and RARS's), Z80's (SM83's too: rgbds,
//! sjasmplus, z80asm, sdas, TASM) and 6502's (ca65, DASM, ACME, 64tass, NESASM); since X-183
//! 68k's (Motorola's syntax: vasm, Devpac, ASM-One, asm68k, AS, Easy68K; GAS's, MIT's `%d0` too),
//! which bat's set has no grammar for either. One scan, the architecture a const generic of it
//! ([`X86`] … [`M68K`]): its table's column, its comments, its immediates' `#`, `$` and `%`, its
//! labels.
//!
//! A line is a statement or a few: a label, then a mnemonic, a directive or a macro's name at the
//! statement's head, then operands. One loop over a line's bytes, a byte's action one load from a
//! table ([`ACT`]), a name read once and looked up once in one table of every word ([`WORDS`]:
//! registers, directives, data and size words, prefixes, operand modifiers, each by architecture)
//! and the statement's head in a bool: what a word is comes from where it stands, not from a list
//! of mnemonics, so every mnemonic and macro an assembler takes reads as one.
//!
//! The dialect is read from the text as it goes and kept across lines ([`State::arg`]): a GAS
//! directive at a head (`.text`, `.globl`), an AT&T register (`%rax`), a numeric label (`1:`) or,
//! on x86, a C preprocessor line make a file GAS's, where `;` separates statements (but at a
//! statement's start, where it's a comment: Apple's arm64), `"…"` takes escapes, `'a` is a
//! character and `1b` a label's; elsewhere `;` is a comment, `1b` is binary. The preprocessors'
//! lines (NASM's `%define`, `%macro`; cpp's `#define`, `#include` in a `.S` file) and NASM's macro
//! bodies are `meta.preprocessor`, which every theme colors, their plain text filled. RISC-V's
//! and MIPS's files are GAS's from the start; Z80's and 6502's never (`;` a comment, `$FF` and
//! `%0101` numbers, a word in the first column a label unless it's one of the architecture's
//! few mnemonics). 68k's read as theirs, but for GAS's (its `|` comments, `'a`, `1b`) where the
//! text says, and Motorola's comment field: past a statement's operands, a blank starts the
//! comment, no mark needed ([`Scan::field`]).

pub(crate) mod sniff;
#[cfg(test)]
mod tests;

use crate::emit::{Rules, Run, Runs, Sink, Spans};
use crate::words::{Words, text_len};
use crate::{Out, State};

/// The floor probe (X-171's `sol`): a statement's head and its operands, comments, strings and
/// numbers; no dialect, preprocessor, macro or label reading past `:`.
const SOL: bool = option_env!("ADA_PARSE_X171_SOL").is_some();

// The architectures, [`Scan`]'s `A`.
/// x86 and x86-64 (X-171).
pub(crate) const X86: u8 = 0;
/// ARM32 and AArch64 (X-171).
pub(crate) const ARM: u8 = 1;
/// RISC-V (X-181).
pub(crate) const RV: u8 = 2;
/// MIPS (X-181).
pub(crate) const MIPS: u8 = 3;
/// Z80, and the Game Boy's SM83 (X-181).
pub(crate) const Z80: u8 = 4;
/// 6502, 65C02 and 65816 (X-181).
pub(crate) const M65: u8 = 5;
/// 68000 to 68060, ColdFire (X-183).
pub(crate) const M68K: u8 = 6;
/// How many there are.
pub(crate) const ARCHES: usize = 7;

/// The leaves: a token's scopes. A kind past [`META`] is its leaf in `meta.preprocessor`.
static LEAVES: [&str; 49] = [
    "",
    // comments
    "comment.line.asm",
    "comment.line.asm punctuation.definition.comment.asm",
    "comment.block.asm",
    "comment.block.asm punctuation.definition.comment.asm",
    // a statement's parts
    "entity.name.constant.asm",
    "punctuation.separator.asm",
    "keyword.operator.word.mnemonic.asm",
    "storage.modifier.prefix.asm",
    "support.function.directive.asm",
    "storage.type.asm",
    "storage.type.directive.asm",
    "storage.modifier.asm",
    "entity.name.section.asm string.unquoted.asm support.constant.section.asm",
    "constant.language.register.asm",
    "variable.language.asm",
    "entity.name.constant.special.asm",
    // numbers
    "constant.numeric.hex.asm",
    "constant.numeric.decimal.asm",
    "constant.numeric.binary.asm",
    "constant.numeric.octal.asm",
    "constant.numeric.decimal.floating-point.asm",
    // operators and punctuation
    "keyword.operator.arithmetic.asm",
    "keyword.operator.bitwise.asm",
    "operator.separator.asm",
    // strings: the text, then BEGIN, END, ESCAPE
    "string.quoted.single.asm",
    "string.quoted.single.asm punctuation.definition.string.begin.asm",
    "string.quoted.single.asm punctuation.definition.string.end.asm",
    "string.quoted.single.asm constant.character.escape.asm",
    "string.quoted.double.asm",
    "string.quoted.double.asm punctuation.definition.string.begin.asm",
    "string.quoted.double.asm punctuation.definition.string.end.asm",
    "string.quoted.double.asm constant.character.escape.asm",
    "string.quoted.other.asm",
    "string.quoted.other.asm punctuation.definition.string.begin.asm",
    "string.quoted.other.asm punctuation.definition.string.end.asm",
    "string.quoted.other.asm constant.character.escape.asm",
    // the preprocessors' and macros'
    "keyword.control.preprocessor.asm",
    "keyword.control.preprocessor.asm punctuation.definition.keyword.preprocessor.asm",
    "entity.name.function.preprocessor.asm",
    "variable.parameter.preprocessor.asm",
    "variable.other.preprocessor.asm",
    "punctuation.definition.variable.preprocessor.asm keyword.control.preprocessor.asm",
    "string.quoted.other.lt-gt.include.asm",
    "string.quoted.other.lt-gt.include.asm punctuation.definition.string.begin.asm",
    "string.quoted.other.lt-gt.include.asm punctuation.definition.string.end.asm",
    "punctuation.separator.continuation.asm",
    "entity.name.constant.preprocessor.asm",
    "storage.modifier.directive.asm variable.parameter.directive.asm",
];

// The leaves by name, in the order of `LEAVES`.
const COMMENT: u16 = 1;
const COMMENT_MARK: u16 = 2;
const BLOCK: u16 = 3;
const BLOCK_MARK: u16 = 4;
const NAME: u16 = 5;
const LABEL_SEP: u16 = 6;
const MNEMONIC: u16 = 7;
const PREFIX: u16 = 8;
const DIRECTIVE: u16 = 9;
const DATA: u16 = 10;
const TYPE_DIRECTIVE: u16 = 11;
const MODIFIER: u16 = 12;
const SECTION: u16 = 13;
const REGISTER: u16 = 14;
const CURRENT: u16 = 15;
const SPECIAL: u16 = 16;
const HEX: u16 = 17;
const DECIMAL: u16 = 18;
const BINARY: u16 = 19;
const OCTAL: u16 = 20;
const FLOAT: u16 = 21;
const ARITHMETIC: u16 = 22;
const BITWISE: u16 = 23;
const SEPARATOR: u16 = 24;
/// A string's leaves: its text, then these after it.
const SINGLE: u16 = 25;
const DOUBLE: u16 = 29;
const BACKTICK: u16 = 33;
const BEGIN: u16 = 1;
const END: u16 = 2;
const ESCAPE: u16 = 3;
const PP_KEYWORD: u16 = 37;
const PP_MARK: u16 = 38;
const PP_NAME: u16 = 39;
const PP_PARAM: u16 = 40;
const PP_VAR: u16 = 41;
const PP_VAR_MARK: u16 = 42;
const INCLUDE: u16 = 43;
const CONTINUATION: u16 = 46;
const PP_CONST: u16 = 47;
/// A symbol's visibility after its type (NASM's `global f:function hidden`).
const VISIBILITY: u16 = 48;
/// A kind's bit for `meta.preprocessor` around its leaf; [`META`] alone is the plain text there.
const META: u16 = 64;

/// The scopes under the language's base a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    let (mut out, leaf) = if kind & META != 0 {
        (vec!["meta.preprocessor.asm"], kind & !META)
    } else {
        (Vec::new(), kind)
    };
    if let Some(l) = LEAVES.get(usize::from(leaf)) {
        out.extend(l.split(' ').filter(|s| !s.is_empty()));
    } else {
        out.clear();
    }
    out
}

// A byte's action ([`ACT`]).
const A_OTHER: u8 = 0;
const A_SPACE: u8 = 1;
const A_NAME: u8 = 2;
const A_DIGIT: u8 = 3;
const A_DQUOTE: u8 = 4;
const A_SQUOTE: u8 = 5;
const A_BQUOTE: u8 = 6;
const A_SLASH: u8 = 7;
const A_DOT: u8 = 8;
const A_SEMI: u8 = 9;
const A_HASH: u8 = 10;
const A_AT: u8 = 11;
const A_PERCENT: u8 = 12;
const A_DOLLAR: u8 = 13;
const A_COMMA: u8 = 14;
const A_COLON: u8 = 15;
/// `+-*`: arithmetic.
const A_ARITH: u8 = 16;
/// `&|^~!<>`: bitwise.
const A_BIT: u8 = 17;
const A_BACKSLASH: u8 = 18;
const A_LBRACE: u8 = 19;
/// A byte past ASCII: a name's where its character is ([`wide_name`]).
const A_WIDE: u8 = 20;

static ACT: [u8; 256] = {
    let mut t = [A_WIDE; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = match b {
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => A_SPACE,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'?' => A_NAME,
            b'0'..=b'9' => A_DIGIT,
            b'"' => A_DQUOTE,
            b'\'' => A_SQUOTE,
            b'`' => A_BQUOTE,
            b'/' => A_SLASH,
            b'.' => A_DOT,
            b';' => A_SEMI,
            b'#' => A_HASH,
            b'@' => A_AT,
            b'%' => A_PERCENT,
            b'$' => A_DOLLAR,
            b',' => A_COMMA,
            b':' => A_COLON,
            b'+' | b'-' | b'*' => A_ARITH,
            b'&' | b'|' | b'^' | b'~' | b'!' | b'<' | b'>' => A_BIT,
            b'\\' => A_BACKSLASH,
            b'{' => A_LBRACE,
            _ => A_OTHER,
        };
        c += 1;
    }
    t
};

/// A name's bytes past its first: letters, digits, `_`, `?`, `$` (GAS's and NASM's symbols);
/// past ASCII, a character [`wide_name`] takes.
static NAME_CONT: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'?' | b'$');
        c += 1;
    }
    t
};

/// A name's first byte.
#[inline(always)]
fn starts_name(c: u8) -> bool {
    ACT[usize::from(c)] == A_NAME
}

/// Whether the character at `p` of `b`, past ASCII, is a name's (X-186): a letter or a digit
/// (the Atari ST's and the Amiga's labels, `DONNÉES_AFFICH`), or U+FFFD, which stands for such
/// a letter in a file of another charset read as UTF-8 (the Atari ST's `É`, 0x90); not `←`, `×`.
#[cold]
#[inline(never)]
fn wide_name(b: &[u8], p: usize) -> bool {
    crate::prim::wide_char(b, p)
        .is_some_and(|c| c.is_alphabetic() || crate::prim::is_nd(c) || c == '\u{fffd}')
}

/// The end of the name's characters past ASCII from `e`, a byte past ASCII: the rest of a
/// character the name started with, then each one [`wide_name`] takes; `e` where none.
#[cold]
#[inline(never)]
fn wide_name_end(b: &[u8], mut e: usize) -> usize {
    while e < b.len() && b[e] >= 0x80 {
        if b[e] < 0xc0 {
            e += 1;
        } else if wide_name(b, e) {
            e += crate::prim::char_len(b[e]).min(b.len() - e);
        } else {
            break;
        }
    }
    e
}

/// [`Scan::name_end`]'s rest from `e`, a byte past ASCII: the name's characters past ASCII
/// ([`wide_name_end`]), its ASCII bytes and its `.`-joined parts, out of the scan's loops.
#[cold]
#[inline(never)]
fn wide_name_rest(b: &[u8], mut e: usize) -> usize {
    let n = b.len();
    loop {
        e = wide_name_end(b, e);
        while e < n && NAME_CONT[usize::from(b[e])] {
            e += 1;
        }
        if e < n && b[e] >= 0x80 && wide_name_end(b, e) > e {
            continue;
        }
        if e + 1 < n && b[e] == b'.' && NAME_CONT[usize::from(b[e + 1])] {
            e += 2;
            continue;
        }
        return e;
    }
}

// What a word of a table is on an architecture ([`INFO`]): its leaf in the low byte, then where it
// counts; 0 for a word that isn't one of the architecture's.
/// At a statement's head: a directive, a data word, a prefix (not a mnemonic).
const HEAD: u16 = 1 << 8;
/// Among the operands: a register, a data or size word, a modifier.
const OPERAND: u16 = 1 << 9;
/// A prefix: the statement's head is still to come.
const KEEP: u16 = 1 << 10;
/// A word that makes the name before it at a head a label (`msg db`, `x equ 4`, `f PROC`).
const NEXT: u16 = 1 << 11;
/// A section's directive: its first operand is a section's name.
const SECT: u16 = 1 << 12;
/// A dot directive MASM has: no sign of GAS.
const MASM: u16 = 1 << 13;
/// GAS's `.macro`: a macro's name and parameters follow.
const MACDEF: u16 = 1 << 14;
/// On RISC-V's, MIPS's, Z80's and 6502's columns (where [`MASM`] means nothing): a word that
/// votes for the architecture in the sniff ([`sniff`]): a mnemonic, a directive or an operand's
/// word that x86's and ARM's assemblers don't have.
const VOTE: u16 = MASM;
/// On Z80's column, a mnemonic's (no [`HEAD`], where [`KEEP`] means nothing): a branch, whose
/// first operand may be a condition (`jp c, x`, `ret z`).
const BRANCH: u16 = KEEP;
/// On 68k's column: a mnemonic or a directive without operands (`rts`, `even`, `endc`), so what
/// follows it on a line outside GAS's files is the comment field (Motorola's, without a mark).
const BARE: u16 = 1 << 15;

const REG: u16 = REGISTER | OPERAND;
const DIR: u16 = DIRECTIVE | HEAD;
/// A data word: `db` at a head, `times 4 db 0`'s after a count, `x dd 0`'s after a name.
const DAT: u16 = DATA | HEAD | OPERAND;
/// A size word: an operand's (`dword`, `ptr`).
const SIZ: u16 = DATA | OPERAND;
const MOD: u16 = MODIFIER | OPERAND;
const PRE: u16 = PREFIX | HEAD | KEEP;
/// A directive that comes after a name (`x equ 4`).
const DIR2: u16 = DIRECTIVE | HEAD | OPERAND;

include!("words.rs");

/// A table's words, in its list's order.
const fn texts<const M: usize>(list: &[(&'static [u8], u16, u16)]) -> [&'static [u8]; M] {
    let mut t: [&[u8]; M] = [b""; M];
    let mut i = 0;
    while i < M {
        t[i] = list[i].0;
        i += 1;
    }
    t
}

/// [`texts`] for a table of one architecture.
const fn texts1<const M: usize>(list: &[(&'static [u8], u16)]) -> [&'static [u8]; M] {
    let mut t: [&[u8]; M] = [b""; M];
    let mut i = 0;
    while i < M {
        t[i] = list[i].0;
        i += 1;
    }
    t
}

/// [`info`] for a table of one architecture.
const fn info1<const M: usize>(list: &[(&[u8], u16)]) -> [u16; M] {
    let mut t = [0u16; M];
    let mut i = 0;
    while i + 1 < M {
        t[i + 1] = list[i].1;
        i += 1;
    }
    t
}

/// What a table's words are on one of its family's two architectures (`second`), by id; 0 for
/// none.
const fn info<const M: usize>(list: &[(&[u8], u16, u16)], second: bool) -> [u16; M] {
    let mut t = [0u16; M];
    let mut i = 0;
    while i + 1 < M {
        t[i + 1] = if second { list[i].2 } else { list[i].1 };
        i += 1;
    }
    t
}

// Four tables (X-181, X-183): x86's and ARM's (X-171's), RISC-V's and MIPS's, Z80's and 6502's,
// 68k's. One table of all would be past `Words`' 2,048 words, and would make each lookup pass a
// filter fuller of others' words: 68k's in Z80's and 6502's table cost them 5% of their
// instructions (X-183).
const N: usize = WORD_LIST.len();
static WORD_TEXT: [&[u8]; N] = texts(WORD_LIST);
static WORDS: Words<4096, N, { text_len(&WORD_TEXT) }, true> = Words::new(&WORD_TEXT);
const N_RM: usize = WORD_LIST_RM.len();
static WORD_TEXT_RM: [&[u8]; N_RM] = texts(WORD_LIST_RM);
static WORDS_RM: Words<2048, N_RM, { text_len(&WORD_TEXT_RM) }, true> = Words::new(&WORD_TEXT_RM);
const N_8: usize = WORD_LIST_8.len();
static WORD_TEXT_8: [&[u8]; N_8] = texts(WORD_LIST_8);
static WORDS_8: Words<2048, N_8, { text_len(&WORD_TEXT_8) }, true> = Words::new(&WORD_TEXT_8);
static INFO_X86: [u16; N + 1] = info(WORD_LIST, false);
static INFO_ARM: [u16; N + 1] = info(WORD_LIST, true);
static INFO_RM: [[u16; 2]; N_RM + 1] = pair(info(WORD_LIST_RM, false), info(WORD_LIST_RM, true));
static INFO_8: [[u16; 2]; N_8 + 1] = pair(info(WORD_LIST_8, false), info(WORD_LIST_8, true));
const N_68: usize = WORD_LIST_68.len();
static WORD_TEXT_68: [&[u8]; N_68] = texts1(WORD_LIST_68);
static WORDS_68: Words<1024, N_68, { text_len(&WORD_TEXT_68) }, true> = Words::new(&WORD_TEXT_68);
static INFO_68: [u16; N_68 + 1] = info1(WORD_LIST_68);

/// A family's two columns side by side, so that one lookup answers for the architecture a scan
/// holds as a value ([`Scan::arch`]).
const fn pair<const M: usize>(first: [u16; M], second: [u16; M]) -> [[u16; 2]; M] {
    let mut t = [[0u16; 2]; M];
    let mut i = 0;
    while i < M {
        t[i] = [first[i], second[i]];
        i += 1;
    }
    t
}

/// The scan's copy an architecture runs on (X-182): x86's and ARM's each their own, RISC-V's and
/// MIPS's one ([`RV`]), Z80's, 6502's and 68k's one ([`Z80`]), the architecture a value within
/// it. A family shares all but a few readings, so a copy each was its size twice over. 68k joined
/// Z80's and 6502's in X-186 (the user's choice: 32 KB for a few percent of the three's
/// instructions, its tests of the architecture), its table still its own.
const fn copy_of(a: u8) -> u8 {
    match a {
        RV | MIPS => RV,
        Z80 | M65 | M68K => Z80,
        _ => a,
    }
}

/// Whether `a`'s copy runs `a` alone ([`copy_of`]): its architecture known where it's built.
const fn solo(a: u8) -> bool {
    let mut n = 0;
    let mut i = 0;
    while i < ARCHES as u8 {
        n += (copy_of(i) == a) as u8;
        i += 1;
    }
    n == 1
}

/// Whether `w` is one of x86inc's temporaries (`DECLARE_REG_TMP`'s `t0`-`t14`, with a size
/// `t3d`, `t4b`; libjpeg-turbo's `T0d`), as its `r0d` an x86 register. Read by its shape, not
/// from x86's table (X-186: its 90 forms there passed more names through the table's filter, x86
/// alone −3%).
#[inline(always)]
fn x86inc_tmp(w: &[u8]) -> bool {
    let n = match w {
        [t, d, ..] if *t | 0x20 == b't' && d.is_ascii_digit() => {
            if w.get(2).is_some_and(u8::is_ascii_digit) {
                if *d != b'1' || w[2] > b'4' {
                    return false;
                }
                3
            } else {
                2
            }
        }
        _ => return false,
    };
    match &w[n..] {
        [] => true,
        [s] => matches!(s | 0x20, b'd' | b'w' | b'b' | b'q' | b'h'),
        _ => false,
    }
}

/// What `w` is on each architecture, in [`X86`] … [`M68K`]'s order: the sniff's lookup.
pub(crate) fn word_on_each(w: &[u8]) -> [u16; ARCHES] {
    let a = usize::from(WORDS.get(w));
    let r = usize::from(WORDS_RM.get(w));
    let e = usize::from(WORDS_8.get(w));
    let k = usize::from(WORDS_68.get(w));
    [
        if a == 0 && x86inc_tmp(w) {
            REG
        } else {
            INFO_X86[a]
        },
        INFO_ARM[a],
        INFO_RM[r][0],
        INFO_RM[r][1],
        INFO_8[e][0],
        INFO_8[e][1],
        INFO_68[k],
    ]
}

// [`State::arg`]'s bits.
/// What a line starts in: code, or a `/* */` comment.
const M_COMMENT: u16 = 1;
/// The file is GAS's (a GAS directive, an AT&T register, a numeric label, a cpp line on x86).
const GAS: u16 = 1 << 1;
/// In GAS's, a `;` stood at a statement's start: `;` is a comment (Apple's arm64), not a
/// separator.
const SEMI: u16 = 1 << 2;
/// The line before was a preprocessor's directive ended by `\`: this one goes on with it.
const CONT: u16 = 1 << 3;
/// NASM's `%macro` bodies open, two bits from here (saturating at 3).
const MACROS: u16 = 4;

/// An architecture's entries: a line (spans), its runs, a block on one scan, its runs; each on
/// its family's copy ([`copy_of`]), the architecture its value.
macro_rules! entries {
    ($a:expr, $what:literal, $line:ident, $line_runs:ident, $block:ident, $block_runs:ident) => {
        #[doc = concat!("Lex `b`, a line of ", $what, " assembly without its `\\n`, from `state`.")]
        #[inline(never)]
        pub fn $line(b: &[u8], state: &mut State, out: &mut Out) {
            line_on::<_, { copy_of($a) }, false>($a, b, state, out, Spans::new());
        }

        #[doc = concat!("[`", stringify!($line), "`]'s runs by theme rule pushed to `runs`.")]
        #[inline(never)]
        pub fn $line_runs(
            b: &[u8],
            state: &mut State,
            out: &mut Out,
            rules: &mut dyn Rules,
            runs: &mut Vec<Run>,
        ) {
            let mut k = line_on::<_, { copy_of($a) }, true>($a, b, state, out, Runs::new(rules, runs));
            k.end_line(b.len());
        }

        #[doc = concat!("A block of ", $what, " assembly on one scan opened from `state` ([`crate::Carry`]).")]
        pub fn $block(
            state: &mut State,
            spans: &mut Vec<crate::Span>,
            pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
        ) -> Option<usize> {
            block_on::<{ copy_of($a) }>($a, state, spans, pass)
        }

        #[doc = concat!("[`", stringify!($block), "`]'s runs by theme rule pushed to `runs` ([`crate::CarryRuns`]).")]
        pub fn $block_runs(
            state: &mut State,
            spans: &mut Vec<crate::Span>,
            rules: &mut dyn Rules,
            runs: &mut Vec<Run>,
            lines: &mut crate::RunLines,
        ) -> Option<usize> {
            block_runs_on::<{ copy_of($a) }>($a, state, spans, rules, runs, lines)
        }
    };
}

entries!(X86, "x86", line, line_runs, block, block_runs);
entries!(
    ARM,
    "ARM",
    line_arm,
    line_runs_arm,
    block_arm,
    block_runs_arm
);
entries!(
    RV,
    "RISC-V",
    line_riscv,
    line_runs_riscv,
    block_riscv,
    block_runs_riscv
);
entries!(
    MIPS,
    "MIPS",
    line_mips,
    line_runs_mips,
    block_mips,
    block_runs_mips
);
entries!(
    Z80,
    "Z80",
    line_z80,
    line_runs_z80,
    block_z80,
    block_runs_z80
);
entries!(
    M65,
    "6502",
    line_6502,
    line_runs_6502,
    block_6502,
    block_runs_6502
);
entries!(
    M68K,
    "68k",
    line_68k,
    line_runs_68k,
    block_68k,
    block_runs_68k
);

#[inline(always)]
fn line_on<K: Sink, const A: u8, const SHARED: bool>(
    arch: u8,
    b: &[u8],
    state: &mut State,
    out: &mut Out,
    k: K,
) -> K {
    let mut s = Scan::<K, A>::new(arch, b, out, k);
    s.load(state);
    s.k.open(s.out, b.len() / 4 + 16);
    s.lex_line::<SHARED>();
    s.k.close(s.out);
    *state = s.state();
    s.k
}

#[inline(always)]
fn block_on<const A: u8>(
    arch: u8,
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    let mut out = Out::new(spans);
    let mut s = Scan::<Spans, A>::new(arch, b"", &mut out, Spans::new());
    s.load(state);
    s.out.0.clear();
    s.k.open(s.out, 64);
    let rest = pass(&mut s);
    *state = s.state();
    rest
}

#[inline(always)]
fn block_runs_on<const A: u8>(
    arch: u8,
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    lines: &mut crate::RunLines,
) -> Option<usize> {
    let mut out = Out::new(spans);
    let mut s = Scan::<Runs, A>::new(arch, b"", &mut out, Runs::new(rules, runs));
    s.load(state);
    s.out.0.clear();
    s.k.open(s.out, 64);
    let rest = lines.pass(&mut s);
    *state = s.state();
    rest
}

impl<const A: u8> crate::Carry for Scan<'_, '_, Spans, A> {
    fn next(&mut self, b: &[u8]) -> &[crate::Span] {
        // SAFETY: the line is set back to an empty one before `b`'s borrow ends
        self.carry::<false>(unsafe { crate::line_for(b) });
        self.out.0
    }
}

impl<const A: u8> crate::CarryRuns for Scan<'_, '_, Runs<'_>, A> {
    fn next(&mut self, b: &[u8]) -> usize {
        // SAFETY: as above
        self.carry::<true>(unsafe { crate::line_for(b) });
        self.k.end_line(b.len())
    }
}

/// The scan of a line; `A` its copy ([`X86`], [`ARM`], [`RV`] for RISC-V's and MIPS's, [`Z80`]
/// for Z80's, 6502's and 68k's: [`copy_of`]), `arch` its architecture. Every field but `arch` and `arg`
/// is the line's own, set where the line starts ([`Scan::lex_line`]), so a block's carried scan
/// lexes as the line API does.
struct Scan<'b, 'c, K: Sink, const A: u8> {
    b: &'b [u8],
    /// The architecture ([`X86`] … [`M68K`]), one of `A`'s family.
    arch: u8,
    /// What the line before left ([`State::arg`]'s bits: [`M_COMMENT`], [`GAS`], [`SEMI`],
    /// [`CONT`], the macros open from [`MACROS`]).
    arg: u16,
    /// The next word stands at a statement's head: a label, a mnemonic, a directive.
    head: bool,
    /// Where the line's first token stands (past its blanks, and a `/* */` comment's rest).
    lead: usize,
    /// [`META`] where the line, or its rest, is the preprocessor's text; else 0.
    meta: u16,
    /// Where the preprocessor's text has been written to, the plain text after it to fill.
    fill: usize,
    /// The line is a preprocessor's directive: a `\` at its end continues it.
    pp: bool,
    /// The next operand is a section's name.
    sect: bool,
    /// The names that follow are a GAS macro's parameters (`.macro m a, b`).
    params: bool,
    /// On 6502, the line has Kick Assembler's `.for`, whose `(…)` hold `;` separators.
    kick_for: bool,
    /// On 68k, [`Scan::field`]'s walk of the line so far: where it has read to, and what it read.
    walk: Walk,
    k: K,
    out: &'b mut Out<'c>,
}

impl<'b, 'c, K: Sink, const A: u8> Scan<'b, 'c, K, A> {
    /// Z80's, 6502's and 68k's: `$FF`, `%0101`, a label in the first column; no GAS but 68k's
    /// where its text says.
    const EIGHT: bool = A == Z80;
    /// The copy runs one architecture ([`solo`]).
    const SOLO: bool = solo(A);

    /// Whether the architecture is `a`: on a copy of one architecture ([`solo`]) known where it's
    /// built, on a family's a test of [`Scan::arch`].
    #[inline(always)]
    fn is(&self, a: u8) -> bool {
        copy_of(a) == A && (Self::SOLO || self.arch == a)
    }

    /// What a word is on this architecture (its table's lookup, its column's value); 0 for none.
    #[inline(always)]
    fn word(&self, w: &[u8]) -> u16 {
        match A {
            X86 => WORDS.value(w, &INFO_X86),
            ARM => WORDS.value(w, &INFO_ARM),
            RV => WORDS_RM.value(w, &INFO_RM)[usize::from(self.arch & 1)],
            _ if self.is(M68K) => WORDS_68.value(w, &INFO_68),
            _ => WORDS_8.value(w, &INFO_8)[usize::from(self.arch & 1)],
        }
    }

    #[inline(always)]
    fn new(arch: u8, b: &'b [u8], out: &'b mut Out<'c>, k: K) -> Self {
        Scan {
            b,
            arch,
            arg: 0,
            head: true,
            lead: 0,
            meta: 0,
            fill: 0,
            pp: false,
            sect: false,
            params: false,
            kick_for: false,
            walk: Walk::default(),
            k,
            out,
        }
    }

    #[inline(always)]
    fn load(&mut self, state: &State) {
        self.arg = state.arg;
    }

    /// The state the scan's last line left.
    #[inline(always)]
    fn state(&self) -> State {
        State {
            arg: self.arg,
            ..State::default()
        }
    }

    /// `b`, a block's next line, lexed on this scan, its sink closed; the scan's line an empty
    /// one again after.
    #[inline(always)]
    fn carry<const SHARED: bool>(&mut self, b: &'b [u8]) {
        self.k.start_line();
        self.b = b;
        self.lex_line::<SHARED>();
        self.k.close(self.out);
        self.b = b"";
    }

    /// Whether the file is GAS's: RISC-V's and MIPS's always, Z80's and 6502's never, x86's,
    /// ARM's and 68k's as their text says ([`GAS`]).
    #[inline(always)]
    fn gas(&self) -> bool {
        A == RV || (A <= ARM || self.is(M68K)) && self.arg & GAS != 0
    }

    /// A write: one call on the run sink.
    #[inline(always)]
    fn put(&mut self, s: usize, e: usize, kind: u16) {
        if self.meta != 0 {
            self.put_meta(s, e, kind);
        } else {
            self.k.merge_call(&mut self.out, s, e, kind);
        }
    }

    /// A write inlined, for the common tokens.
    #[inline(always)]
    fn put_hot(&mut self, s: usize, e: usize, kind: u16) {
        if self.meta != 0 {
            self.put_meta(s, e, kind);
        } else {
            self.k.merge(&mut self.out, s, e, kind);
        }
    }

    /// A write in the preprocessor's text: the plain text before it filled.
    #[inline(never)]
    fn put_meta(&mut self, s: usize, e: usize, kind: u16) {
        if s > self.fill {
            self.k.merge_call(&mut self.out, self.fill, s, META);
        }
        self.k.merge_call(&mut self.out, s, e, kind | META);
        self.fill = e;
    }

    /// The preprocessor's text from `p` on.
    #[inline(always)]
    fn meta_on(&mut self, p: usize) {
        if self.meta == 0 {
            self.meta = META;
            self.fill = p;
        }
    }

    /// The line, from what the line before left; the loop out of line where `SHARED`
    /// ([`Scan::code_shared`]).
    #[inline(always)]
    fn lex_line<const SHARED: bool>(&mut self) {
        self.head = true;
        self.sect = false;
        self.params = false;
        if Self::EIGHT {
            self.kick_for = false;
            self.walk = Walk::default();
        }
        self.fill = 0;
        self.pp = false;
        self.meta = 0;
        if !SOL && self.arg & (CONT | 3 << MACROS) != 0 {
            self.meta = META;
            if self.arg & CONT != 0 {
                // a continued directive's line: the directive's, a macro's body of statements
                // (`pushq r; \`)
                self.arg &= !CONT;
                self.pp = true;
            }
        }
        let p = if self.arg & M_COMMENT != 0 {
            self.block_comment(0, 0)
        } else {
            0
        };
        self.lead = skip_blanks(self.b, p);
        if SHARED {
            self.code_shared(p);
        } else {
            self.code(p);
        }
        let n = self.b.len();
        if self.meta != 0 && self.fill < n {
            self.k.merge_call(&mut self.out, self.fill, n, META);
        }
    }

    /// [`Scan::code`] out of line: on the run sink, one copy that the line API and a block's
    /// lines share; on the span sink, which ada doesn't hold, inlined.
    #[inline(never)]
    fn code_shared(&mut self, p: usize) {
        self.code(p);
    }

    /// The code from `p` to the line's end.
    #[inline(always)]
    fn code(&mut self, mut p: usize) {
        let b = self.b;
        let n = b.len();
        while p < n {
            let c = b[p];
            let from = p;
            p = match ACT[usize::from(c)] {
                // a letter past ASCII starts a name too (X-186), on the one call of `name`
                a @ (A_NAME | A_WIDE) if a == A_NAME || !SOL && wide_name(b, p) => self.name(p, p),
                A_SPACE => {
                    let mut e = p + 1;
                    while e < n && ACT[usize::from(b[e])] == A_SPACE {
                        e += 1;
                    }
                    if !SOL && !self.head && e < n && self.is(M68K) {
                        self.field(p, e)
                    } else {
                        e
                    }
                }
                A_DIGIT => self.number(p, p),
                A_DOT => self.dot(p),
                A_COMMA => {
                    self.put_hot(p, p + 1, SEPARATOR);
                    p + 1
                }
                A_SEMI => self.semi(p),
                A_HASH => self.hash(p),
                A_AT => self.at(p),
                A_SLASH => match b.get(p + 1) {
                    Some(b'/') => self.line_comment(p),
                    Some(b'*') => self.block_comment(p, p + 2),
                    _ => {
                        self.put_hot(p, p + 1, ARITHMETIC);
                        p + 1
                    }
                },
                A_PERCENT => self.percent(p),
                A_DOLLAR => self.dollar(p),
                A_DQUOTE => self.string(p, DOUBLE),
                A_SQUOTE => self.squote(p),
                A_BQUOTE if !SOL && self.is(Z80) => self.backtick(p),
                A_BQUOTE if !SOL && self.is(M65) => {
                    // 64tass' bank byte (`` #`label ``), as `<` and `>` are the low and high:
                    // no 6502 assembler quotes with it
                    self.put(p, p + 1, BITWISE);
                    p + 1
                }
                A_BQUOTE => self.string(p, BACKTICK),
                A_COLON => self.colon(p),
                A_ARITH if !SOL && self.is(M65) => self.arith_6502(p),
                A_ARITH if !SOL && c == b'*' && self.is(M68K) => self.star_68k(p),
                A_ARITH => {
                    self.put_hot(p, p + 1, ARITHMETIC);
                    p + 1
                }
                A_BIT if !SOL && self.is(ARM) && c == b'|' => self.bar(p),
                A_BIT if !SOL && self.is(M65) && c == b'!' => self.bang(p),
                A_BIT if !SOL && matches!(c, b'|' | b'<') && self.is(M68K) => self.bit_68k(p),
                A_BIT => {
                    let e = if matches!(c, b'<' | b'>') && b.get(p + 1) == Some(&c) {
                        p + 2
                    } else {
                        p + 1
                    };
                    self.put_hot(p, e, BITWISE);
                    e
                }
                A_BACKSLASH if !SOL => self.backslash(p),
                A_OTHER
                    if !SOL
                        && c == b']'
                        && b.get(p + 1).is_some_and(|&c| starts_name(c))
                        && (p == self.lead
                            || matches!(b[p - 1], b' ' | b'\t' | b',' | b'#' | b'('))
                        && self.is(M65) =>
                {
                    self.merlin_var(p)
                }
                A_LBRACE if !SOL && self.is(X86) => self.brace(p),
                A_WIDE => p + crate::prim::char_len(c).min(n - p),
                _ => p + 1,
            };
            debug_assert!(p > from, "assembly's code loop stood at {from}");
        }
    }

    /// The end of a name whose first byte is at `p`: its bytes, and a `.` with a name's byte
    /// after it (`vld1.8`, `b.ne`, `v0.16b`, `foo.bar`).
    #[inline(always)]
    fn name_end(&self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut e = p + 1;
        loop {
            while e < n && NAME_CONT[usize::from(b[e])] {
                e += 1;
            }
            if e < n && b[e] >= 0x80 {
                // a letter past ASCII, or the rest of the one the name starts with (X-186)
                return wide_name_rest(b, e);
            }
            if e + 1 < n && b[e] == b'.' && NAME_CONT[usize::from(b[e + 1])] {
                e += 2;
                continue;
            }
            return e;
        }
    }

    /// A name from `p` (its text from `s`: a `$` before it): a label, a statement's head, or an
    /// operand.
    #[inline(always)]
    fn name(&mut self, s: usize, p: usize) -> usize {
        let b = self.b;
        let e = self.name_end(p);
        let w = self.word(&b[p..e]);
        if self.head {
            return self.head_word(s, e, w);
        }
        self.operand(s, p, e, w)
    }

    /// An operand's name `p..e` (its text from `s`), `w` its word.
    #[inline(always)]
    fn operand(&mut self, s: usize, p: usize, mut e: usize, w: u16) -> usize {
        let kind = if SOL {
            if w & OPERAND != 0 { w & 0xff } else { NAME }
        } else if self.sect || self.params {
            self.operand_role()
        } else if w & OPERAND != 0 {
            if self.is(Z80) && self.b.get(e) == Some(&b'\'') && w & 0xff == REGISTER {
                // the shadow register (`ex af, af'`)
                e += 1;
            }
            w & 0xff
        } else if w == 0 && (self.is(ARM) || self.is(RV) || self.is(M68K)) {
            self.arm_operand(p, e)
        } else if A == X86 && w == 0 && x86inc_tmp(&self.b[p..e]) {
            REGISTER
        } else {
            if Self::EIGHT && self.b.get(e) == Some(&b':') && self.is(M65) {
                // ca65's scoped name (`MGTK::Rect`), one symbol
                e = self.scoped(e);
            }
            NAME
        };
        self.put_hot(s, e, kind);
        e
    }

    /// ca65's scoped name's rest from `e`: each `::` and the name after it (`MGTK::Rect`,
    /// `A::B::c`, `::global`'s from its first `:`).
    #[cold]
    #[inline(never)]
    fn scoped(&self, mut e: usize) -> usize {
        let b = self.b;
        while b.get(e) == Some(&b':')
            && b.get(e + 1) == Some(&b':')
            && b.get(e + 2).is_some_and(|&c| starts_name(c))
        {
            e = self.name_end(e + 2);
        }
        e
    }

    /// A word at a statement's head, `s..e`: a label (`x:`, and `msg db`'s `msg`), a
    /// directive, data, a prefix, or a mnemonic (a macro's name too).
    #[inline(always)]
    fn head_word(&mut self, s: usize, e: usize, w: u16) -> usize {
        let b = self.b;
        if b.get(e) == Some(&b':') {
            self.put_hot(s, e, NAME);
            self.put_hot(e, e + 1, LABEL_SEP);
            return e + 1;
        }
        if w & HEAD != 0 && !(Self::EIGHT && self.names_label(s, e)) {
            self.put_hot(s, e, w & 0xff);
            self.head = w & KEEP != 0;
            self.sect = w & SECT != 0;
            if !SOL && w & BARE != 0 {
                // 68k's directive without operands (`even`, `endc`): the comment field after it
                return self.bare(e);
            }
            if !SOL && self.is(Z80) && w & MACDEF != 0 {
                // rgbds' `MACRO name` (its name and parameters), sjasmplus' `name MACRO a, b`
                // (its parameters)
                if s == self.lead {
                    return self.macro_head(e);
                }
                self.params = true;
            }
            return e;
        }
        if SOL {
            self.put_hot(s, e, MNEMONIC);
            self.head = false;
            return e;
        }
        if w & BARE != 0 {
            // 68k's mnemonic without operands (`rts`): the comment field after it
            self.put_hot(s, e, MNEMONIC);
            self.head = false;
            return self.bare(e);
        }
        if s == 0
            && (self.is(ARM) && !self.gas()
                || Self::EIGHT
                    && w & 0xff != MNEMONIC
                    && b.get(e) != Some(&b'(')
                    && !(w == 0 && self.is(M68K) && self.sized_mnemonic(s, e)))
        {
            // armasm's and IAR's label: a word in the first column; Z80's and 6502's too, but
            // one of the architecture's mnemonics (`lda #0` in ca65's first column) or a
            // macro's call (Kick Assembler's `BasicUpstart2(start)`)
            self.put_hot(s, e, NAME);
            if Self::EIGHT {
                self.assigned(e);
            }
            return e;
        }
        // the next token, read once here: a word that makes this one a label (`msg db`,
        // `x equ 4`, `f PROC`), an assignment's `=` (`x = 4`), or the mnemonic's first operand
        let n = b.len();
        let q = skip_blanks(b, e);
        if q > e && q < n {
            let c = b[q];
            if starts_name(c) {
                let t = self.name_end(q);
                let w2 = self.word(&b[q..t]);
                // a word that makes this one a label; on Z80 and 6502, any directive after a
                // mnemonic's word in the first column (ZCPR's `JR MACRO ?N`, `PER .res 1`)
                if w2 & NEXT != 0 || Self::EIGHT && s == 0 && w2 & HEAD != 0 {
                    self.put_hot(s, e, NAME);
                    return e;
                }
                if self.is(X86) && w2 & (0xff | HEAD) == DATA && self.typed_data(s, q, t) {
                    // MASM's data typed by a size word (`x DWORD ?`)
                    self.put(s, e, NAME);
                    self.put(q, t, DATA);
                    self.head = false;
                    return t;
                }
                self.put_hot(s, e, MNEMONIC);
                self.head = false;
                if self.is(X86) && w2 == 0 && b[e - 1] == b'R' && b[s..e].ends_with(b"_INSTR") {
                    // x86inc's `AVX_INSTR addpd, …`: an `_INSTR` macro's first argument is a
                    // mnemonic
                    self.put(q, t, MNEMONIC);
                    return t;
                }
                if self.is(Z80) && w & BRANCH != 0 && is_condition(&b[q..t]) {
                    // a branch's condition (`jp c, x`, `ret z`), not the register `c`
                    self.put(q, t, MODIFIER);
                    return t;
                }
                return self.operand(q, q, t, w2);
            }
            if (c == b'=' || c == b'.' || Self::EIGHT && c == b':') && self.labels_next(e)
                || Self::EIGHT && s == 0 && c == b'.' && {
                    let t = self.name_end(q);
                    t > q + 1 && self.word(&b[q..t]) & HEAD != 0
                }
            {
                self.put_hot(s, e, NAME);
                return e;
            }
        }
        // a macro's size argument (Devpac's and vasm's `move.\0`, X-186): its `.` the mnemonic's,
        // as `move.l`'s is
        let e = if b.get(e) == Some(&b'.') && b.get(e + 1) == Some(&b'\\') && self.is(M68K) {
            e + 1
        } else {
            e
        };
        self.put_hot(s, e, MNEMONIC);
        self.head = false;
        e
    }

    /// Whether a head's word from `s`, before a size word `q..t`, is MASM's data's name
    /// (`x DWORD ?`, `msg BYTE "hi",0`), not a mnemonic with a sized operand (`push dword 0`,
    /// `mov DWORD PTR [x], 1`, `mov dword [x], 1`): a size word other than `PTR` before `?` (no
    /// instruction's operand), or, for a word in the first column (where instructions aren't),
    /// before a value, not `PTR`, `[`, `,` or the line's end (`HFLIP byte, b, 1`, a macro's
    /// argument).
    #[cold]
    #[inline(never)]
    fn typed_data(&self, s: usize, q: usize, t: usize) -> bool {
        let b = self.b;
        if b[q..t].eq_ignore_ascii_case(b"ptr") {
            return false;
        }
        let r = skip_blanks(b, t);
        let Some(&c) = b.get(r) else {
            return false;
        };
        if c == b'?' {
            return !b.get(r + 1).is_some_and(|&c| NAME_CONT[usize::from(c)]);
        }
        if s != 0 || matches!(c, b'[' | b',' | b';') {
            return false;
        }
        !(starts_name(c) && b[r..self.name_end(r)].eq_ignore_ascii_case(b"ptr"))
    }

    /// Whether a head's word `s..e` that's a directive's on Z80, 6502 and 68k names a label instead
    /// (the label paths after write it): before an assignment (ca65's `mx := x`, sjasmplus' `LOAD =
    /// #EF`); on 6502, in the first column before a mnemonic or a data word (`INV  JSR MVXY`, `adrl
    /// ds 1`; X-184, as Merlin's words came in), not on Z80 (rgbds' `if DEF(x)`, `OPT pop`).
    #[cold]
    #[inline(never)]
    fn names_label(&self, s: usize, e: usize) -> bool {
        let b = self.b;
        let q = skip_blanks(b, e);
        match b.get(q) {
            Some(b'=') => b.get(q + 1) != Some(&b'='),
            Some(b':') => b.get(q + 1) == Some(&b'='),
            Some(&c) if s == 0 && q > e && starts_name(c) && self.is(M65) => {
                let t = self.name_end(q);
                let w = self.word(&b[q..t]);
                w & 0xff == MNEMONIC || w & (0xff | HEAD) == DATA | HEAD
            }
            _ => false,
        }
    }

    /// Whether 68k's word `s..e` (none of the table's) is a mnemonic with its size (`move.l`,
    /// `bne.s`): in the first column, a mnemonic, not a label.
    #[cold]
    #[inline(never)]
    fn sized_mnemonic(&self, s: usize, e: usize) -> bool {
        let t = &self.b[s..e];
        let k = t.len();
        k > 2
            && t[k - 2] == b'.'
            && matches!(t[k - 1] | 0x20, b'b' | b'w' | b'l' | b's')
            && self.word(&t[..k - 2]) & 0xff == MNEMONIC
    }

    /// Whether 68k's dotted word in the first column, `p..e`, is a local label: a mnemonic after
    /// it (`.Skip  move.l d0,d1`); nothing but a comment where it's no GAS directive without
    /// operands (`.text`, `.even`: [`BARE`]); else where the file isn't yet GAS's (AMOS' `.Skip
    /// Rjmpt x`, a macro's call), where GAS's directive takes an operand (`.globl f`).
    #[cold]
    #[inline(never)]
    fn label_68k(&self, p: usize, e: usize) -> bool {
        let b = self.b;
        let q = skip_blanks(b, e);
        if q >= b.len() || matches!(b[q], b';' | b'*' | b'|') {
            return self.word(&b[p..e]) & BARE == 0;
        }
        if starts_name(b[q]) {
            let t = self.name_end(q);
            if self.word(&b[q..t]) & 0xff == MNEMONIC || self.sized_mnemonic(q, t) {
                return true;
            }
        }
        !self.gas()
    }

    /// 68k's comment field from `e`, after a mnemonic or a directive without operands, outside
    /// GAS's files: past the blanks, what's left on the line (Devpac's and vasm's `rts  return`).
    #[cold]
    #[inline(never)]
    fn bare(&mut self, e: usize) -> usize {
        let b = self.b;
        let q = skip_blanks(b, e);
        if q > e && q < b.len() && !self.gas() && !matches!(b[q], b';' | b'*' | b'|' | b'/') {
            self.put(q, b.len(), COMMENT);
            return b.len();
        }
        e
    }

    /// 68k's blank `p..e` past a statement's head, outside GAS's files and preprocessor lines:
    /// Motorola's assemblers end the operand field at a blank and take what follows as the comment
    /// field, no mark needed (Devpac's, vasm's, ASM-One's `move.l d0,d1  copy it`). Not where the
    /// operands go on: a blank after or before an operator, a `,` or a `)`, in an open `(`, or the
    /// blank between the head and its operands. Where the code loop goes on.
    #[cold]
    #[inline(never)]
    fn field(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        if self.gas() || self.pp || self.meta != 0 || p == 0 {
            return e;
        }
        let (prev, next) = (b[p - 1], b[e]);
        if next == b'*'
            && b.get(e + 1).is_none_or(|&c| matches!(c, b' ' | b'\t'))
            && !matches!(prev, b',' | b'(' | b'+' | b'-' | b'*' | b'/' | b'=' | b'#')
            && self.walk_to(p)
        {
            // the comment field led by a `*` (Amiga's includes: `LONG x  * the count`)
            return self.line_comment(e);
        }
        // a post-increment's `+` ends its operand (`(a0)+  next`)
        let post = prev == b'+' && p >= 2 && b[p - 2] == b')';
        if matches!(next, b';' | b',' | b')')
            || is_operator(next) && next != b'~'
            || !post && (matches!(prev, b',' | b'(' | b'#') || is_operator(prev))
        {
            return e;
        }
        if self.walk_to(p) {
            self.put(e, b.len(), COMMENT);
            return b.len();
        }
        e
    }

    /// [`Scan::field`]'s walk of the line from where it last stopped to the blank at `p`, the
    /// field it ends classified: whether it is an operand field ended outside any `(` (the head's
    /// own field, a label's, don't end the operands). Strings are read whole, so that a blank in
    /// one isn't a field's end; each byte is read once a line.
    #[cold]
    #[inline(never)]
    fn walk_to(&mut self, p: usize) -> bool {
        let b = self.b;
        let mut w = self.walk;
        if w.at < self.lead {
            w.at = self.lead;
            w.run = self.lead;
        }
        while w.at < p {
            match b[w.at] {
                q @ (b'"' | b'\'') => {
                    w.at = b[w.at + 1..]
                        .iter()
                        .position(|&c| c == q)
                        .map_or(b.len(), |i| w.at + 2 + i);
                    continue;
                }
                b'(' => w.depth += 1,
                b')' => w.depth -= 1,
                b' ' | b'\t' => {
                    if w.run < w.at {
                        w.end_run(b, w.at);
                    }
                    w.run = w.at + 1;
                }
                _ => {}
            }
            w.at += 1;
        }
        let ended = w.run < p && {
            let operands = w.past_head;
            w.end_run(b, p);
            operands && w.depth <= 0
        };
        w.run = p + 1;
        w.at = p;
        self.walk = w;
        ended
    }

    /// 68k's `*`: a comment at a line's start; the location counter where an operand starts
    /// (`bra *`, `dc.l *+4`, `x-*`); else the operator.
    #[inline(never)]
    fn star_68k(&mut self, p: usize) -> usize {
        let b = self.b;
        if p == self.lead && !self.pp {
            return self.line_comment(p);
        }
        let blank_after = b.get(p + 1).is_none_or(|&c| matches!(c, b' ' | b'\t'));
        let starts = p > 0
            && (matches!(b[p - 1], b'(' | b',' | b'=' | b'#' | b'-' | b'+')
                || matches!(b[p - 1], b' ' | b'\t') && !blank_after);
        self.put(p, p + 1, if starts { CURRENT } else { ARITHMETIC });
        p + 1
    }

    /// 68k's `|` and `<`: `|` a comment at a line's start and in GAS's files (but a preprocessor
    /// line's `||`), else the operator; `<` opening a macro's argument where an operand starts
    /// (Devpac's `EdT 169,<Library not opened>`), to its `>`, else the operator.
    #[inline(never)]
    fn bit_68k(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        if b[p] == b'|' {
            if !self.pp && (p == self.lead || self.gas()) {
                return self.line_comment(p);
            }
        } else if p > 0
            && matches!(b[p - 1], b',' | b' ' | b'\t')
            && !self.head
            && b.get(p + 1)
                .is_some_and(|&c| !matches!(c, b' ' | b'\t' | b'<' | b'=' | b'>'))
            && let Some(i) = b[p + 1..].iter().position(|&c| c == b'>' || c == b';')
            && b[p + 1 + i] == b'>'
        {
            let q = p + 1 + i;
            self.put(p, p + 1, BACKTICK + BEGIN);
            if q > p + 1 {
                self.put(p + 1, q, BACKTICK);
            }
            self.put(q, q + 1, BACKTICK + END);
            return q + 1;
        }
        let e = if b[p] == b'<' && b.get(p + 1) == Some(&b'<') {
            p + 2
        } else {
            p + 1
        };
        self.put(p, e.min(n), BITWISE);
        e.min(n)
    }

    /// After a label in Z80's and 6502's first column ending at `e`, an assignment's `=` or `:=`
    /// (`X = 4`, ca65's `x := y`): what follows is its value, no statement.
    #[inline(never)]
    fn assigned(&mut self, e: usize) {
        let b = self.b;
        let mut q = skip_blanks(b, e);
        if b.get(q) == Some(&b':') {
            q += 1;
        }
        if b.get(q) == Some(&b'=') && b.get(q + 1) != Some(&b'=') {
            self.head = false;
        }
    }

    /// Whether the word at a head ending at `e` is a label without its `:`: before a word that
    /// makes it one (`msg db`, `x equ 4`, `fp .req x29`, NASM's `.b0 resq 1`), or before an
    /// assignment's `=` (`x = 4`, ca65's `x := y`).
    #[inline(never)]
    fn labels_next(&mut self, e: usize) -> bool {
        let b = self.b;
        let n = b.len();
        let mut q = e;
        while q < n && matches!(b[q], b' ' | b'\t') {
            q += 1;
        }
        if q == e || q >= n {
            return false;
        }
        let c = b[q];
        if c == b'=' {
            if b.get(q + 1) != Some(&b'=') {
                self.head = false;
                return true;
            }
            return false;
        }
        if c == b':' {
            // ca65's `x := y` (Z80's and 6502's look only)
            if b.get(q + 1) == Some(&b'=') {
                self.head = false;
                return true;
            }
            return false;
        }
        let from = if c == b'.' { q + 1 } else { q };
        if from < n && starts_name(b[from]) {
            let t = self.name_end(from);
            return self.word(&b[q..t]) & NEXT != 0;
        }
        false
    }

    /// An operand's role where a name's word doesn't decide it: a section's name, a macro's
    /// parameter.
    #[cold]
    fn operand_role(&mut self) -> u16 {
        if self.sect {
            self.sect = false;
            SECTION
        } else {
            PP_PARAM
        }
    }

    /// An ARM or RISC-V operand no word names: a register with its arrangement or lane
    /// (`v0.16b`, `v1.s`; RISC-V's mask `v0.t`), ARM's system register (`sctlr_el1`), or a name.
    #[inline(never)]
    fn arm_operand(&self, p: usize, e: usize) -> u16 {
        let t = &self.b[p..e];
        if let Some(d) = t.iter().position(|&c| c == b'.')
            && self.word(&t[..d]) & 0xff == REGISTER
        {
            return REGISTER;
        }
        let k = t.len();
        if self.is(ARM)
            && k > 4
            && t[k - 1].is_ascii_digit()
            && t[k - 4] == b'_'
            && t[k - 3].eq_ignore_ascii_case(&b'e')
            && t[k - 2].eq_ignore_ascii_case(&b'l')
        {
            return REGISTER;
        }
        NAME
    }

    /// A `.`: a directive at a statement's head (GAS's; MASM's `.code`), a local label
    /// (`.L1:`, NASM's `.loop:`), a symbol, a section's name, NASM's `..plt`, or the location
    /// counter alone.
    #[inline(always)]
    fn dot(&mut self, p: usize) -> usize {
        let b = self.b;
        match b.get(p + 1) {
            Some(&c) if starts_name(c) => {
                let e = self.name_end(p + 1);
                if b.get(e) == Some(&b':') && (SOL || b.get(e + 1) != Some(&b':')) {
                    if self.head {
                        self.put_hot(p, e, NAME);
                        self.put_hot(e, e + 1, LABEL_SEP);
                        return e + 1;
                    }
                } else if self.head {
                    if !SOL
                        && Self::EIGHT
                        && p == 0
                        && (self.word(&b[p..e]) & HEAD == 0
                            || self.is(M68K) && self.label_68k(p, e))
                    {
                        // a local label in the first column (rgbds' `.loop`, DASM's): no
                        // directive the architecture's assemblers have; on 68k, before a
                        // mnemonic or alone, whatever its name (AMOS' `.Skip`, not GAS's `.skip`)
                        self.put_hot(p, e, NAME);
                        return e;
                    }
                    if !SOL && self.labels_next(e) {
                        // NASM's struc's field (`.b0 resq 1`)
                        self.put_hot(p, e, NAME);
                        return e;
                    }
                    self.head = false;
                    if SOL {
                        self.put_hot(p, e, DIRECTIVE);
                        return e;
                    }
                    return self.directive(p, e);
                }
                let kind = if !SOL && self.sect {
                    self.sect = false;
                    SECTION
                } else if !SOL
                    && e == p + 2
                    && matches!(b[p + 1] | 0x20, b'b' | b'w' | b'l' | b's')
                    && self.is(M68K)
                {
                    // 68k's absolute address's size (`($FF8240).w`, `$24.w`)
                    DATA
                } else {
                    NAME
                };
                self.put_hot(p, e, kind);
                e
            }
            Some(b'.') if !SOL && b.get(p + 2).is_some_and(|&c| starts_name(c) || c == b'@') => {
                // NASM's special symbols (`..plt`, `..start`, `..@x`)
                let e = self.name_end(p + 2);
                self.put_hot(p, e, MODIFIER);
                e
            }
            Some(c) if c.is_ascii_digit() => self.number(p, p),
            _ => {
                self.put_hot(p, p + 1, CURRENT);
                p + 1
            }
        }
    }

    /// A dot directive at a head, `p..e`: GAS's (the file is GAS's) but where MASM has it too;
    /// a section's, a macro's. Where the code loop goes on.
    #[inline(always)]
    fn directive(&mut self, p: usize, e: usize) -> usize {
        let w = self.word(&self.b[p..e]);
        if A <= ARM && w & MASM == 0 || w != 0 && self.is(M68K) {
            // 68k's column has GAS's dotted directives only
            self.arg |= GAS;
        }
        if Self::EIGHT && e - p == 4 && self.b[p + 1..e].eq_ignore_ascii_case(b"for") {
            self.kick_for = true;
        }
        self.put_hot(p, e, DIRECTIVE);
        if w & (SECT | MACDEF) == 0 {
            return e;
        }
        self.sect = w & SECT != 0;
        if w & MACDEF != 0 {
            return self.macro_head(e);
        }
        e
    }

    /// GAS's `.macro`'s rest from `p`: its name, then its parameters (the code loop goes on
    /// past the name, where they're read as such).
    #[cold]
    #[inline(never)]
    fn macro_head(&mut self, p: usize) -> usize {
        let b = self.b;
        let q = skip_blanks(b, p);
        if q < b.len() && starts_name(b[q]) {
            let e = self.name_end(q);
            self.put(q, e, PP_NAME);
            self.params = true;
            return e;
        }
        p
    }

    /// A number from `p` (its text from `s`: an immediate's `#` or `$`): its run of letters,
    /// digits and points, by its base; GAS's `1f` and `1b` are labels', and `1:` at a head is a
    /// label.
    #[inline(always)]
    fn number(&mut self, s: usize, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut e = p;
        while e < n && (NAME_CONT[usize::from(b[e])] || b[e] == b'.') {
            e += 1;
        }
        if SOL {
            self.put_hot(s, e, DECIMAL);
            return e;
        }
        if Self::EIGHT && b[e - 1] == b'$' && s == p && self.is(M68K) {
            return self.dollar_label(p, e);
        }
        if Self::EIGHT
            && e >= p + 3
            && b[e - 2] == b'.'
            && matches!(b[e - 1] | 0x20, b'b' | b'w' | b'l' | b's')
            && self.is(M68K)
        {
            // 68k's absolute address with its size (`4.w`), no float
            let kind = self.number_kind(&b[p..e - 2]);
            self.put_hot(s, e - 2, kind);
            self.put(e - 2, e, DATA);
            return e;
        }
        if self.head && s == p && b.get(e) == Some(&b':') && b[p..e].iter().all(u8::is_ascii_digit)
        {
            // a numeric label: GAS's
            if A <= ARM || self.is(M68K) {
                self.arg |= GAS;
            }
            self.put_hot(p, e, NAME);
            self.put_hot(e, e + 1, LABEL_SEP);
            return e + 1;
        }
        // an exponent's sign (`1.5e-3`)
        if e < n
            && matches!(b[e], b'+' | b'-')
            && matches!(b[e - 1], b'e' | b'E')
            && b[p..e - 1].contains(&b'.')
            && b.get(e + 1).is_some_and(u8::is_ascii_digit)
        {
            e += 2;
            while e < n && b[e].is_ascii_alphanumeric() {
                e += 1;
            }
        }
        let kind = self.number_kind(&b[p..e]);
        self.put_hot(s, e, kind);
        e
    }

    /// 68k's `1$`, `p..e`: Devpac's and vasm's local label (its `:` at a head), not a number.
    #[cold]
    #[inline(never)]
    fn dollar_label(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        if !b[p..e - 1].iter().all(u8::is_ascii_digit) {
            let kind = self.number_kind(&b[p..e]);
            self.put(p, e, kind);
            return e;
        }
        self.put(p, e, NAME);
        if self.head && b.get(e) == Some(&b':') {
            self.put(e, e + 1, LABEL_SEP);
            return e + 1;
        }
        e
    }

    /// A number's kind by its text: its prefix (`0x`, `0b`, `0o`), its suffix (NASM's and
    /// MASM's `h`, `b`, `q`, `o`), a point; a label's (`1f`, GAS's `1b`).
    #[inline(always)]
    fn number_kind(&self, t: &[u8]) -> u16 {
        let k = t.len();
        let last = t[k - 1] | 0x20;
        if k > 1 && t[0] == b'0' {
            match t[1] | 0x20 {
                b'x' | b'h' => return HEX,
                b'b' | b'y' if k > 2 => return BINARY,
                b'o' | b'q' if k > 2 => return OCTAL,
                b'f' | b'd' | b'e' if k > 2 && t[2..].contains(&b'.') => return FLOAT,
                _ => {}
            }
        }
        if (last == b'f' || last == b'b' && self.gas()) && t[..k - 1].iter().all(u8::is_ascii_digit)
        {
            return NAME;
        }
        if t.contains(&b'.') {
            return FLOAT;
        }
        match last {
            b'h' => HEX,
            b'b' | b'y' => BINARY,
            b'q' | b'o' => OCTAL,
            _ => DECIMAL,
        }
    }

    /// A comment from `p` to the line's end: its mark (`;`, `#`, `@`, `//`), then its text.
    #[inline(always)]
    fn line_comment(&mut self, p: usize) -> usize {
        let n = self.b.len();
        let m = if self.b.get(p + 1) == Some(&b'/') && self.b[p] == b'/' {
            2
        } else {
            1
        };
        self.put(p, p + m, COMMENT_MARK);
        if p + m < n {
            self.put(p + m, n, COMMENT);
        }
        n
    }

    /// A `/* */` comment from `s` (its text from `p`), to its `*/` or the line's end (a mode).
    #[inline(never)]
    fn block_comment(&mut self, s: usize, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        if p > s {
            self.put(s, p, BLOCK_MARK);
        }
        let mut q = crate::prim::find_any(b, p, *b"*");
        while q + 1 < n {
            if b[q + 1] == b'/' {
                if q > p {
                    self.put(p, q, BLOCK);
                }
                self.put(q, q + 2, BLOCK_MARK);
                self.arg &= !M_COMMENT;
                return q + 2;
            }
            q = crate::prim::find_any(b, q + 1, *b"*");
        }
        if n > p {
            self.put(p, n, BLOCK);
        }
        self.arg |= M_COMMENT;
        n
    }

    /// A `;`: a comment, but in GAS's files a statement's separator (a comment still where it
    /// starts a statement, and then in the rest of the file: Apple's arm64); on 68k a comment in
    /// GAS's files too (SGDK's).
    #[inline(always)]
    fn semi(&mut self, p: usize) -> usize {
        if SOL || !self.gas() || self.arg & SEMI != 0 || self.is(M68K) {
            if !SOL && Self::EIGHT && self.kick_for && self.is(M65) && self.in_parens(p) {
                // Kick Assembler's `.for (var i = 0; i < 8; i++)`, whose comments are `//`
                self.put(p, p + 1, SEPARATOR);
                return p + 1;
            }
            return self.line_comment(p);
        }
        if p == self.lead {
            self.arg |= SEMI;
            return self.line_comment(p);
        }
        self.head = true;
        self.sect = false;
        self.params = false;
        p + 1
    }

    /// Whether a `(` before `p` on the line is still open, outside strings and characters
    /// (`.byte "(" ; a note`, `lda #'('`).
    #[cold]
    #[inline(never)]
    fn in_parens(&self, p: usize) -> bool {
        let t = &self.b[self.lead.min(p)..p];
        if !t.contains(&b'(') {
            return false;
        }
        let (mut depth, mut i) = (0i32, 0);
        while i < t.len() {
            match t[i] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                b'"' => {
                    i += t[i + 1..]
                        .iter()
                        .position(|&c| c == b'"')
                        .map_or(t.len(), |k| k + 1);
                }
                // a character, `'('` or DASM's `'(`
                b'\'' => i += 2,
                _ => {}
            }
            i += 1;
        }
        depth > 0
    }

    /// A `#`: a C preprocessor's directive or a comment at a line's start (GAS's), x86's,
    /// RISC-V's and MIPS's comment elsewhere; ARM's, Z80's, 6502's and 68k's immediate (`#4`,
    /// `#$10`) elsewhere (Z80's and 6502's at a line's start too; 68k's `#` before a blank a
    /// comment); in a preprocessor's directive, its `#` and `##`.
    #[inline(always)]
    fn hash(&mut self, p: usize) -> usize {
        let b = self.b;
        if p == self.lead && !self.pp {
            if !SOL && let Some(e) = self.cpp(p) {
                return e;
            }
            if SOL || !Self::EIGHT || self.is(M68K) {
                return self.line_comment(p);
            }
        }
        if !SOL && Self::EIGHT {
            if !self.pp && b.get(p + 1).is_none_or(|&c| matches!(c, b' ' | b'\t')) && self.is(M68K)
            {
                // SVR4's comment after the operands (linux's `ifpsp060`: `# lock page`)
                return self.line_comment(p);
            }
            return self.immediate(p);
        }
        if self.is(ARM) {
            return match b.get(p + 1) {
                Some(c) if c.is_ascii_digit() => self.number(p, p + 1),
                Some(b'-') if b.get(p + 2).is_some_and(u8::is_ascii_digit) => self.number(p, p + 2),
                _ => p + 1,
            };
        }
        if !SOL && self.pp {
            let e = if b.get(p + 1) == Some(&b'#') {
                p + 2
            } else {
                p + 1
            };
            self.put(p, e, PP_VAR_MARK);
            return e;
        }
        self.line_comment(p)
    }

    /// A C preprocessor's directive from its `#` at `p`, if a word cpp knows follows: its
    /// keyword, an `#include`'s file, a `#define`'s name and parameters, the rest the line's.
    #[cold]
    #[inline(never)]
    fn cpp(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let q = skip_blanks(b, p + 1);
        if q >= b.len() || !b[q].is_ascii_lowercase() {
            return None;
        }
        let e = self.name_end(q);
        let word = &b[q..e];
        if !CPP.contains(&word) {
            return None;
        }
        if self.is(X86) || self.is(M68K) {
            self.arg |= GAS;
        }
        self.meta_on(p);
        self.pp = true;
        self.head = false;
        self.put(p, p + 1, PP_MARK);
        self.put(q, e, PP_KEYWORD);
        Some(match word {
            b"include" | b"include_next" | b"import" => self.include(e),
            b"define" | b"undef" => self.pp_name(e, false),
            _ => e,
        })
    }

    /// An `#include`'s or `%include`'s file from `p`: `<…>` or `"…"`.
    #[cold]
    fn include(&mut self, p: usize) -> usize {
        let b = self.b;
        let q = skip_blanks(b, p);
        match b.get(q) {
            Some(b'<') => {
                let n = b.len();
                let e = b[q + 1..]
                    .iter()
                    .position(|&c| c == b'>')
                    .map_or(n, |i| q + 1 + i);
                self.put(q, q + 1, INCLUDE + BEGIN);
                if e > q + 1 {
                    self.put(q + 1, e, INCLUDE);
                }
                if e < n {
                    self.put(e, e + 1, INCLUDE + END);
                    return e + 1;
                }
                n
            }
            _ => q,
        }
    }

    /// A preprocessor's macro's name from `p` (`#define`'s, `%define`'s; `%macro`'s where
    /// `function`), and the parameters in its `(…)` right after it: a function's name with them,
    /// a constant's without.
    #[cold]
    fn pp_name(&mut self, p: usize, function: bool) -> usize {
        let b = self.b;
        let n = b.len();
        let q = skip_blanks(b, p);
        if q >= n || !starts_name(b[q]) {
            return q;
        }
        let e = self.name_end(q);
        let call = b.get(e) == Some(&b'(');
        self.put(q, e, if call || function { PP_NAME } else { PP_CONST });
        if !call {
            return e;
        }
        let mut i = e + 1;
        while i < n && b[i] != b')' {
            if starts_name(b[i]) {
                let t = self.name_end(i);
                self.put(i, t, PP_PARAM);
                i = t;
            } else {
                i += 1;
            }
        }
        i.min(n)
    }

    /// An `@`: ARM32's comment; after a name's byte, a relocation (`foo@PLT`, `x@PAGE`) or
    /// GAS's type (`@function`); Z80's, 6502's and 68k's local label (ca65's `@loop`, sjasmplus'
    /// `@global`, asm68k's `@loop`) or location counter (rgbds' `@`); 68k's MIT operand (`%sp@-`)
    /// or octal number (`@17`).
    #[inline(always)]
    fn at(&mut self, p: usize) -> usize {
        let b = self.b;
        let glued = p > 0 && NAME_CONT[usize::from(b[p - 1])];
        if self.is(ARM) && !glued {
            return self.line_comment(p);
        }
        if !SOL
            && self.is(M68K)
            && (glued || b.get(p + 1).is_some_and(|c| (b'0'..=b'7').contains(c)))
        {
            return self.at_68k(p, glued);
        }
        if !SOL && Self::EIGHT {
            return self.at_label(p);
        }
        match b.get(p + 1) {
            Some(&c) if starts_name(c) => {
                let e = self.name_end(p + 1);
                self.put_hot(p, e, if glued { MODIFIER } else { TYPE_DIRECTIVE });
                e
            }
            _ => p + 1,
        }
    }

    /// A `%`: NASM's directive at a statement's head (`%define`), a macro's parameter (`%1`,
    /// `%+1`), a macro's local (`%%loop`), AT&T's register (`%rax`), GAS's type (`%function`),
    /// else the operator.
    #[inline(always)]
    fn percent(&mut self, p: usize) -> usize {
        let b = self.b;
        if !SOL && A >= RV {
            return self.percent_other(p);
        }
        match b.get(p + 1) {
            Some(&c) if starts_name(c) => {
                let e = self.name_end(p + 1);
                if !SOL && self.is(X86) && self.head {
                    return self.nasm(p, e);
                }
                let w = self.word(&b[p + 1..e]);
                let kind = if w & 0xff == REGISTER {
                    if !SOL {
                        self.arg |= GAS;
                    }
                    REGISTER
                } else {
                    TYPE_DIRECTIVE
                };
                self.put_hot(p, e, kind);
                e
            }
            Some(_) if !SOL => self.percent_rare(p),
            _ => {
                self.put_hot(p, p + 1, ARITHMETIC);
                p + 1
            }
        }
    }

    /// A `%` before something not a name: NASM's macro parameters and locals, else the
    /// operator.
    #[cold]
    #[inline(never)]
    fn percent_rare(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let c = b[p + 1];
        let digits = |from: usize| {
            let mut e = from;
            while e < n && b[e].is_ascii_digit() {
                e += 1;
            }
            e
        };
        match c {
            b'0'..=b'9' => {
                let e = digits(p + 1);
                self.put(p, p + 1, PP_VAR_MARK);
                self.put(p + 1, e, PP_VAR);
                e
            }
            b'+' | b'-' if b.get(p + 2).is_some_and(u8::is_ascii_digit) => {
                let e = digits(p + 2);
                self.put(p, p + 2, PP_VAR_MARK);
                self.put(p + 2, e, PP_VAR);
                e
            }
            b'%' | b'$' => {
                // `%%loop`, `%$ctx`: a macro's and a context's locals
                let m = if c == b'$' && b.get(p + 2) == Some(&b'$') {
                    p + 3
                } else {
                    p + 2
                };
                self.put(p, m, PP_VAR_MARK);
                if m < n && starts_name(b[m]) {
                    let e = self.name_end(m);
                    self.put(m, e, SPECIAL);
                    if self.head && b.get(e) == Some(&b':') {
                        self.put(e, e + 1, LABEL_SEP);
                        return e + 1;
                    }
                    self.head = false;
                    return e;
                }
                m
            }
            b'{' | b'[' | b'+' | b'!' | b'?' | b'*' => {
                self.put(p, p + 2, PP_VAR_MARK);
                p + 2
            }
            _ => {
                self.put(p, p + 1, ARITHMETIC);
                p + 1
            }
        }
    }

    /// NASM's directive `%…` at a head, its word at `p + 1..e`, as bat reads them: a
    /// definition's, a macro's, an include's line the preprocessor's; a condition on a name's
    /// (`%ifdef`) and an assignment (`%assign`) to the name; a condition's (`%if`) and a
    /// repetition's (`%rep`) only the keyword. A `%macro`'s body opened, an `%endmacro`'s closed.
    #[cold]
    #[inline(never)]
    fn nasm(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let mut low = [0u8; 12];
        let k = (e - p - 1).min(low.len());
        for (l, &c) in low.iter_mut().zip(&b[p + 1..p + 1 + k]) {
            *l = c.to_ascii_lowercase();
        }
        let word: &[u8] = if e - p - 1 > low.len() {
            b""
        } else {
            &low[..k]
        };
        self.meta_on(p);
        self.pp = true;
        self.head = false;
        self.put(p, p + 1, PP_MARK);
        self.put(p + 1, e, PP_KEYWORD);
        let macros = (self.arg >> MACROS) & 3;
        match word {
            b"macro" | b"imacro" | b"rmacro" | b"irmacro" => {
                self.arg = self.arg & !(3 << MACROS) | (macros + 1).min(3) << MACROS;
                let q = self.pp_name(e, true);
                let r = skip_blanks(b, q);
                let mut t = r;
                while t < b.len() && matches!(b[t], b'0'..=b'9' | b'-' | b'+' | b'*' | b'.') {
                    t += 1;
                }
                if t > r {
                    self.put(r, t, PP_PARAM);
                }
                t
            }
            b"endmacro" | b"endm" => {
                self.arg = self.arg & !(3 << MACROS) | macros.saturating_sub(1) << MACROS;
                e
            }
            b"define" | b"xdefine" | b"idefine" | b"ixdefine" | b"defstr" | b"deftok"
            | b"defalias" | b"strlen" | b"substr" | b"strcat" => self.pp_name(e, false),
            b"include" | b"use" | b"pathsearch" | b"depend" | b"error" | b"warning" | b"fatal"
            | b"pragma" | b"line" => e,
            b"undef" | b"unmacro" | b"assign" | b"iassign" | b"ifdef" | b"ifndef" | b"elifdef"
            | b"elifndef" | b"ifmacro" | b"ifnmacro" | b"elifmacro" | b"elifnmacro" | b"ifctx"
            | b"ifnctx" | b"elifctx" | b"elifnctx" => {
                let q = self.pp_name(e, false);
                if macros == 0 {
                    self.meta = 0;
                }
                self.pp = false;
                q
            }
            _ => {
                // a condition's, a repetition's: only its keyword is the preprocessor's
                if macros == 0 {
                    self.meta = 0;
                }
                self.pp = false;
                e
            }
        }
    }

    /// A `$`: AT&T's immediate (`$4`), NASM's location counter (`$`, `$$`), a name; MIPS's
    /// register (`$t0`, `$31`, `$f2`); Z80's and 6502's hex number (`$FF`).
    #[inline(always)]
    fn dollar(&mut self, p: usize) -> usize {
        let b = self.b;
        if !SOL && Self::EIGHT && b.get(p + 1).is_some_and(u8::is_ascii_hexdigit) {
            return self.prefixed(p, p + 1, HEX);
        }
        if !SOL && self.is(MIPS) && b.get(p + 1).is_some_and(u8::is_ascii_digit) {
            let mut e = p + 2;
            while e < b.len() && b[e].is_ascii_digit() {
                e += 1;
            }
            self.put_hot(p, e, REGISTER);
            return e;
        }
        match b.get(p + 1) {
            Some(c) if c.is_ascii_digit() => self.number(p, p + 1),
            Some(b'$') => {
                self.put_hot(p, p + 2, CURRENT);
                p + 2
            }
            Some(&c) if starts_name(c) => self.name(p, p + 1),
            _ => {
                self.put_hot(p, p + 1, CURRENT);
                p + 1
            }
        }
    }

    /// A `:` past a label: x86's symbol type, ARM's relocation (`:lo12:sym`), 6502's anonymous
    /// label (ca65's `:+`, `:--`) or Merlin's local label (`:loop`), else a separator.
    #[inline(always)]
    fn colon(&mut self, p: usize) -> usize {
        let b = self.b;
        if !SOL && b.get(p + 1).is_some_and(|&c| starts_name(c)) {
            if self.is(X86) {
                return self.symbol_type(p);
            }
            if self.is(ARM) {
                let e = self.name_end(p + 1);
                if b.get(e) == Some(&b':') {
                    self.put_hot(p, e + 1, MODIFIER);
                    return e + 1;
                }
            }
        }
        if !SOL
            && self.is(M65)
            && b.get(p + 1) == Some(&b':')
            && b.get(p + 2).is_some_and(|&c| starts_name(c))
        {
            // ca65's name in the global scope (`::bar`)
            let e = self.scoped(p);
            self.put(p, e, NAME);
            return e;
        }
        if !SOL
            && self.is(M65)
            && b.get(p + 1).is_some_and(|&c| starts_name(c))
            && (p == self.lead || p > 0 && matches!(b[p - 1], b' ' | b'\t' | b',' | b'#' | b'('))
        {
            // Merlin's local label (`:L1000`, `bne :loop`; X-184), its `:` the name's
            let e = self.name_end(p + 1);
            self.put(p, e, NAME);
            return e;
        }
        if !SOL
            && self.is(M65)
            && let Some(&c @ (b'+' | b'-')) = b.get(p + 1)
        {
            let mut e = p + 2;
            while b.get(e) == Some(&c) {
                e += 1;
            }
            self.put(p, e, NAME);
            return e;
        }
        self.put_hot(p, p + 1, LABEL_SEP);
        p + 1
    }

    /// A `\`: a GAS macro's parameter (`\reg`, `\@`, `\()`), rgbds' and 68k's (`\1`), or a
    /// line's continuation.
    #[inline(never)]
    fn backslash(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        match b.get(p + 1) {
            Some(&c) if starts_name(c) => {
                let e = self.name_end(p + 1);
                self.put(p, p + 1, PP_VAR_MARK);
                self.put(p + 1, e, PP_VAR);
                e
            }
            Some(c) if (self.is(Z80) || self.is(M68K)) && c.is_ascii_digit() => {
                self.put(p, p + 1, PP_VAR_MARK);
                self.put(p + 1, p + 2, PP_VAR);
                p + 2
            }
            Some(b'@') => {
                self.put(p, p + 2, PP_VAR);
                p + 2
            }
            Some(b'(') if b.get(p + 2) == Some(&b')') => {
                self.put(p, p + 3, PP_VAR_MARK);
                p + 3
            }
            _ if b[p + 1..].iter().all(|&c| c == b' ' || c == b'\t') => {
                self.put(p, p + 1, CONTINUATION);
                if self.pp {
                    self.arg |= CONT;
                }
                n
            }
            _ => p + 1,
        }
    }

    /// x86's `{…}`: AVX-512's mask register (`{k1}`), a zeroing or rounding (`{z}`, `{sae}`),
    /// a broadcast (`{1to16}`), NASM's encoding (`{vex}`).
    #[inline(never)]
    fn brace(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut e = p + 1;
        while e < n && (b[e].is_ascii_alphanumeric() || b[e] == b'-' || b[e] == b'_') {
            e += 1;
        }
        if e > p + 1 && b.get(e) == Some(&b'}') {
            let w = self.word(&b[p + 1..e]);
            let kind = if w & 0xff == REGISTER {
                REGISTER
            } else {
                MODIFIER
            };
            self.put(p + 1, e, kind);
            return e + 1;
        }
        if e > p + 1 && starts_name(b[p + 1]) {
            // an instruction as a macro's argument (x86inc's `REPX {psrad x, 4}, m0`): a word
            // that's no operand's before an operand, not an expression's (`{arg1 + 8}`)
            let e = self.name_end(p + 1);
            let r = skip_blanks(b, e);
            if r > e
                && r < n
                && (starts_name(b[r]) || matches!(b[r], b'[' | b'%' | b'$' | b'0'..=b'9'))
                && self.word(&b[p + 1..e]) & OPERAND == 0
            {
                self.put(p + 1, e, MNEMONIC);
                // a line led by the brace stood at a head: its operands follow
                self.head = false;
                return e;
            }
        }
        p + 1
    }

    /// ARM's `|`: armasm's quoted symbol (`|.text|`, `|Image$$ER$$Base|`), a section's name
    /// after `AREA`, where an operand starts; else the operator, after an operand (IAR's
    /// `#(1|2)`), in a preprocessor's line and in GAS's files.
    #[inline(never)]
    fn bar(&mut self, p: usize) -> usize {
        let b = self.b;
        let operand = p > 0 && (NAME_CONT[usize::from(b[p - 1])] || b[p - 1] == b')');
        if !self.gas() && !self.pp && !operand {
            let q = crate::prim::find_any(b, p + 1, *b"|");
            if q < b.len() && q > p + 1 && !b[p + 1..q].iter().any(|&c| matches!(c, b' ' | b'\t')) {
                let kind = if self.sect {
                    self.sect = false;
                    SECTION
                } else {
                    NAME
                };
                self.put(p, q + 1, kind);
                return q + 1;
            }
        }
        self.put(p, p + 1, BITWISE);
        p + 1
    }

    /// x86's `:` before a word after an operand that's no register (`[fs:data]`): NASM's symbol
    /// type (`global f:function hidden`, `global x:data 4`) and its visibility, else a
    /// separator.
    #[cold]
    #[inline(never)]
    fn symbol_type(&mut self, p: usize) -> usize {
        let b = self.b;
        let e = self.name_end(p + 1);
        let mut s = p;
        while s > 0 && NAME_CONT[usize::from(b[s - 1])] {
            s -= 1;
        }
        let typed = [&b"function"[..], b"data", b"object", b"notype"]
            .iter()
            .any(|w| b[p + 1..e].eq_ignore_ascii_case(w))
            && self.word(&b[s..p]) & 0xff != REGISTER;
        self.put(p, p + 1, LABEL_SEP);
        if !typed {
            return p + 1;
        }
        self.put(p + 1, e, TYPE_DIRECTIVE);
        let r = skip_blanks(b, e);
        if r > e && r < b.len() && starts_name(b[r]) {
            let t = self.name_end(r);
            if [&b"hidden"[..], b"protected", b"internal", b"default"]
                .iter()
                .any(|w| b[r..t].eq_ignore_ascii_case(w))
            {
                self.put(r, t, VISIBILITY);
                return t;
            }
        }
        e
    }

    /// A number from `s` whose digits start at `p` after a prefix (`$FF`, `%0101`, `#$10`), of
    /// `kind`: its run of letters and digits.
    #[inline(never)]
    fn prefixed(&mut self, s: usize, p: usize, kind: u16) -> usize {
        let b = self.b;
        let mut e = p;
        while e < b.len() && NAME_CONT[usize::from(b[e])] {
            e += 1;
        }
        self.put_hot(s, e, kind);
        e
    }

    /// Z80's and 6502's `#`: an immediate (`#4`, `#$10`, `#%0101`, sjasmplus' `#FF` hex), its
    /// `#` the number's; before anything else (`#<label`, `#'a'`) the `#` alone, plain.
    #[inline(never)]
    fn immediate(&mut self, p: usize) -> usize {
        let b = self.b;
        match b.get(p + 1) {
            Some(c) if c.is_ascii_digit() => self.number(p, p + 1),
            Some(b'$') if b.get(p + 2).is_some_and(u8::is_ascii_hexdigit) => {
                self.prefixed(p, p + 2, HEX)
            }
            Some(b'%') if b.get(p + 2).is_some_and(|c| matches!(c, b'0' | b'1')) => {
                self.prefixed(p, p + 2, BINARY)
            }
            Some(c) if self.is(Z80) && c.is_ascii_hexdigit() => {
                let mut e = p + 1;
                while e < b.len() && b[e].is_ascii_hexdigit() {
                    e += 1;
                }
                if b.get(e).is_some_and(|&c| NAME_CONT[usize::from(c)]) {
                    // a name (`#FOO`), no hex number
                    return p + 1;
                }
                self.put(p, e, HEX);
                e
            }
            _ => p + 1,
        }
    }

    /// RISC-V's and MIPS's `%`: a relocation (`%hi(x)`, `%pcrel_lo(1b)`), a symbol's type
    /// (`%function`), MARS's and RARS's macro parameter (`%reg`); Z80's, 6502's and 68k's binary
    /// number (`%0101`); 68k's MIT register (`%d0`); else the operator.
    #[inline(never)]
    fn percent_other(&mut self, p: usize) -> usize {
        let b = self.b;
        match b.get(p + 1) {
            Some(&c) if Self::EIGHT && matches!(c, b'0' | b'1') => self.prefixed(p, p + 1, BINARY),
            Some(&c) if starts_name(c) && self.is(M68K) => {
                // MIT's register (`%d0`, `%sp`, `%fp0`): GAS's
                let e = self.name_end(p + 1);
                let t = &b[p + 1..e];
                let r = t.iter().position(|&c| c == b'.').map_or(t, |d| &t[..d]);
                if self.word(r) & 0xff == REGISTER {
                    self.arg |= GAS;
                    // an index's size and scale (`%a0@(%d1:l:4)`, Motorola's `(a0,d1.l*4)`;
                    // X-186): the size the register's, as `d1.l` is, the scale's `:` the operator
                    let mut e = e;
                    if b.get(e) == Some(&b':')
                        && b.get(e + 1)
                            .is_some_and(|&c| matches!(c | 0x20, b'w' | b'l'))
                        && !b.get(e + 2).is_some_and(|&c| NAME_CONT[usize::from(c)])
                    {
                        e += 2;
                    }
                    self.put(p, e, REGISTER);
                    if b.get(e) == Some(&b':') && b.get(e + 1).is_some_and(u8::is_ascii_digit) {
                        self.put(e, e + 1, ARITHMETIC);
                        return e + 1;
                    }
                    return e;
                }
                self.put(p, p + 1, ARITHMETIC);
                p + 1
            }
            Some(&c) if !Self::EIGHT && starts_name(c) => {
                let e = self.name_end(p + 1);
                match self.word(&b[p + 1..e]) & 0xff {
                    MODIFIER => self.put(p, e, MODIFIER),
                    TYPE_DIRECTIVE => self.put(p, e, TYPE_DIRECTIVE),
                    _ => {
                        self.put(p, p + 1, PP_VAR_MARK);
                        self.put(p + 1, e, PP_VAR);
                    }
                }
                e
            }
            _ => {
                self.put(p, p + 1, ARITHMETIC);
                p + 1
            }
        }
    }

    /// 68k's `@` after a register (MIT's indirect operand, `%sp@-`, `%a0@(8)`), the operator with
    /// its `+` or `-`; else Devpac's and vasm's octal number (`@17`).
    #[cold]
    #[inline(never)]
    fn at_68k(&mut self, p: usize, glued: bool) -> usize {
        let b = self.b;
        if glued {
            let e = if matches!(b.get(p + 1), Some(b'+' | b'-')) {
                p + 2
            } else {
                p + 1
            };
            self.put(p, e, ARITHMETIC);
            return e;
        }
        self.prefixed(p, p + 1, OCTAL)
    }

    /// Z80's and 6502's `@`: a local or global label (ca65's `@loop:`, sjasmplus' `@x`), its
    /// `:` at a head; alone, the location counter (rgbds' `@`).
    #[inline(never)]
    fn at_label(&mut self, p: usize) -> usize {
        let b = self.b;
        if !b.get(p + 1).is_some_and(|&c| starts_name(c)) {
            self.put(p, p + 1, CURRENT);
            return p + 1;
        }
        let e = self.name_end(p + 1);
        self.put(p, e, NAME);
        if self.head && b.get(e) == Some(&b':') {
            self.put(e, e + 1, LABEL_SEP);
            return e + 1;
        }
        e
    }

    /// 6502's `+`, `-` and `*`: an anonymous label (ACME's, 64tass', `+`, `--`) at a statement's
    /// head, or standing alone as the last operand (`bcc +`, `bne --`); a comment line led by `*`
    /// (Merlin's), but the location counter's assignment there (`*= $1000`); the location counter
    /// where an operand starts (`jmp *-3`); else the operator.
    #[inline(never)]
    fn arith_6502(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        let after_blank = p > 0 && matches!(b[p - 1], b' ' | b'\t');
        if c == b'*' {
            if p == self.lead && !self.pp && b[skip_blanks(b, p + 1)..].first() != Some(&b'=') {
                // Merlin's and LISA's comment line (not 64tass' and ACME's `* = $0801`; X-184)
                return self.line_comment(p);
            }
            let starts = p == self.lead
                || p > 0
                    && matches!(
                        b[p - 1],
                        b' ' | b'\t' | b'(' | b',' | b'=' | b'#' | b'<' | b'>' | b'['
                    )
                    && !b.get(p + 1).is_some_and(|&c| matches!(c, b' ' | b'\t'));
            self.put(p, p + 1, if starts { CURRENT } else { ARITHMETIC });
            return p + 1;
        }
        let mut e = p + 1;
        while e < n && b[e] == c {
            e += 1;
        }
        let r = skip_blanks(b, e);
        let alone = e == n || matches!(b[e], b' ' | b'\t' | b';');
        let last = r == n || b[r] == b';';
        if self.head && p == self.lead && alone || !self.head && after_blank && last {
            self.put(p, e, NAME);
            return e;
        }
        self.put(p, p + 1, ARITHMETIC);
        p + 1
    }

    /// Merlin's variable (`]VAR`, `]loop`; X-184) from its `]` at `p`: a name, its `]` the name's.
    #[cold]
    #[inline(never)]
    fn merlin_var(&mut self, p: usize) -> usize {
        let e = self.name_end(p + 1);
        self.put(p, e, NAME);
        if self.head {
            // a label at a head (`]VAR equ $00`, `]loop dex`)
            if self.labels_next(e) {
                return e;
            }
            let q = skip_blanks(self.b, e);
            if q > e {
                return e;
            }
            self.head = false;
        }
        e
    }

    /// 6502's `!`: at a statement's head, Kick Assembler's multi-label (`!:`, `!loop:`) or
    /// ACME's directive (`!byte`, `!macro`); where an operand starts, a multi-label's reference
    /// (`bne !-`, `bcc !++`, `jmp !loop+`); else the operator.
    #[inline(never)]
    fn bang(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let e = if b.get(p + 1).is_some_and(|&c| starts_name(c)) {
            self.name_end(p + 1)
        } else {
            p + 1
        };
        if self.head {
            if b.get(e) == Some(&b':') {
                self.put(p, e, NAME);
                self.put(e, e + 1, LABEL_SEP);
                return e + 1;
            }
            if e > p + 1 {
                self.put(p, e, DIRECTIVE);
                self.head = false;
                return e;
            }
        } else if p > 0 && matches!(b[p - 1], b' ' | b'\t' | b',' | b'(' | b'#') {
            let mut r = e;
            while r < n && matches!(b[r], b'+' | b'-') {
                r += 1;
            }
            if r > e && (r == n || matches!(b[r], b' ' | b'\t' | b',' | b';' | b')')) {
                self.put(p, r, NAME);
                return r;
            }
        }
        self.put(p, p + 1, BITWISE);
        p + 1
    }

    /// rgbds' `` ` ``: a graphics' number (`` `01233210 ``), its digits 0 to 3; else a string.
    #[inline(never)]
    fn backtick(&mut self, p: usize) -> usize {
        let b = self.b;
        let mut e = p + 1;
        while e < b.len() && matches!(b[e], b'0'..=b'3' | b'.') {
            e += 1;
        }
        if e > p + 1 && !b.get(e).is_some_and(|&c| NAME_CONT[usize::from(c)]) {
            self.put(p, e, BINARY);
            return e;
        }
        self.string(p, BACKTICK)
    }

    /// A `'`: GAS's character (`'a`, `'\n'`), 6502's too (DASM's `'A`, ca65's `'A'`), else a
    /// string (Merlin's `'text'` on 6502).
    #[inline(always)]
    fn squote(&mut self, p: usize) -> usize {
        if SOL || !self.gas() && !self.is(M65) {
            return self.string(p, SINGLE);
        }
        let b = self.b;
        let n = b.len();
        if p + 1 >= n {
            self.put(p, p + 1, SINGLE + BEGIN);
            return n;
        }
        if self.is(M65)
            && let Some(i) = b[p + 2..].iter().position(|&c| c == b'\'' || c == b';')
            && i > 0
            && b[p + 2 + i] == b'\''
        {
            // Merlin's string (`asc 'text'`; X-184): a closing quote past two characters, none of a
            // comment's between (DASM's `'A ; it's`; ca65's `'''` and `'\'` characters)
            return self.string(p, SINGLE);
        }
        let mut e = p + 1;
        let esc = b[e] == b'\\' && e + 1 < n;
        if esc {
            e += 1;
        }
        e += crate::prim::char_len(b[e]).min(n - e);
        self.put(p, p + 1, SINGLE + BEGIN);
        self.put(p + 1, e, if esc { SINGLE + ESCAPE } else { SINGLE });
        if b.get(e) == Some(&b'\'') {
            self.put(e, e + 1, SINGLE + END);
            return e + 1;
        }
        e
    }

    /// A string from its quote at `p`, to its closing quote or the line's end; `\` escapes in
    /// NASM's `` `…` `` and GAS's `"…"`.
    #[inline(always)]
    fn string(&mut self, p: usize, kind: u16) -> usize {
        let b = self.b;
        let n = b.len();
        let q = b[p];
        let escapes = kind == BACKTICK || kind == DOUBLE && (SOL || self.is(Z80) || self.gas());
        if Self::EIGHT {
            // a section's name quoted (rgbds' `SECTION "Main", ROM0`, ca65's `.segment "CODE"`)
            self.sect = false;
        }
        self.put(p, p + 1, kind + BEGIN);
        let mut t = p + 1;
        let mut i = p + 1;
        while i < n {
            let c = b[i];
            if c == q {
                if i > t {
                    self.put(t, i, kind);
                }
                self.put(i, i + 1, kind + END);
                return i + 1;
            }
            if c == b'\\' && escapes && i + 1 < n {
                if i > t {
                    self.put(t, i, kind);
                }
                let e = i + 1 + crate::prim::char_len(b[i + 1]).min(n - i - 1);
                self.put(i, e, kind + ESCAPE);
                i = e;
                t = e;
                continue;
            }
            i += 1;
        }
        if n > t {
            self.put(t, n, kind);
        }
        n
    }
}

/// Whether `c` is an expression's operator (68k's comment field: [`Scan::field`]).
fn is_operator(c: u8) -> bool {
    matches!(
        c,
        b'+' | b'-' | b'*' | b'/' | b'&' | b'|' | b'^' | b'<' | b'>' | b'=' | b'!' | b'~'
    )
}

/// Whether `w` is a Z80 branch's condition (`nz`, `c`, `po`, …).
#[cold]
fn is_condition(w: &[u8]) -> bool {
    [&b"nz"[..], b"z", b"nc", b"c", b"po", b"pe", b"p", b"m"]
        .iter()
        .any(|c| w.eq_ignore_ascii_case(c))
}

/// The C preprocessor's directives (`#include`, `#define`, …) in a `.S` file.
const CPP: &[&[u8]] = &[
    b"include",
    b"include_next",
    b"import",
    b"define",
    b"undef",
    b"if",
    b"ifdef",
    b"ifndef",
    b"elif",
    b"elifdef",
    b"elifndef",
    b"else",
    b"endif",
    b"error",
    b"warning",
    b"pragma",
    b"line",
    b"ident",
];

/// [`Scan::field`]'s walk of a 68k line: where it has read to, where the field it is in started,
/// the `(` open, whether the statement's head has gone by (its labels before it).
#[derive(Clone, Copy, Default)]
struct Walk {
    at: usize,
    run: usize,
    depth: i32,
    past_head: bool,
}

impl Walk {
    /// The field `self.run..e` read: a label (in the first column, or ending in `:`) leaves the
    /// head to come; the head's own field puts the operands next.
    fn end_run(&mut self, b: &[u8], e: usize) {
        if !self.past_head && !(self.run == 0 || b[e - 1] == b':') {
            self.past_head = true;
        }
    }
}

/// The first byte at or past `p` that isn't a space or a tab, or the line's end.
#[inline(always)]
fn skip_blanks(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && matches!(b[p], b' ' | b'\t') {
        p += 1;
    }
    p
}
