//! An assembly file's architecture by its text (X-171, X-181, X-183): bat's set names `.s` and
//! `.S` ARM's and `.asm` x86's, but most `.s` and `.S` files are GAS's x86-64 or AArch64, Windows'
//! armasm64 sources are `.asm`, and RISC-V's, MIPS's, Z80's, 6502's and 68k's sources take the
//! same names. The
//! head of the file (its first [`HEAD`] bytes) is read past comments, and its words vote with the
//! scanner's own tables ([`super::word_on_each`]):
//!
//! - an operand's register, for each architecture that has it (`eax`, `xmm0`, x86inc's `m0`;
//!   `x0`, `w1`, `lr`, `v0.16b`; `a0`, `t0`, `ra`; `hl`, `ix`), but ARM32's `r0`-`r7` ARM's alone
//!   (x86inc's macros are named so too, but x86inc's files are NASM's), and a one-letter register
//!   (Z80's `a`, 6502's `x`) not at all;
//! - a directive only one of x86 and ARM has (NASM's `bits`, armasm's `AREA`, IAR's `PUBWEAK`; a
//!   GAS directive by its name without the dot, as X-171 had it), an AT&T register (`%rax`) and a
//!   MIPS one (`$t0`), two votes;
//! - on RISC-V's, MIPS's, Z80's, 6502's and 68k's columns, a word marked [`VOTE`]: a mnemonic at
//!   a statement's head (`addi`, `addiu`, `djnz`, `lda`, `moveq`), which x86's and ARM's
//!   assemblers don't have, a directive at a head, two votes (`.option`, `.ent`, `.segment`,
//!   `xdef`), an operand's word (`noreorder`, `ROM0`); ACME's `!byte` at a head, two votes for
//!   6502; 68k's mnemonic with its size (`move.l`, `bne.s`) at a head, and MIT's register
//!   (`%d0`), two votes for 68k;
//! - a `$` before a hex word (`$C0`): a number, one vote each for Z80, 6502 and 68k.
//!
//! A statement's first word is its mnemonic, not a register (ARM's `bl`), and one before `(` a
//! cpp macro's call (`ENTRY (f)`, not armasm's `ENTRY`); a word before `:` is a label's, no vote
//! (`halt:`, X-184); after a word in the first column without a `:` (Z80's, 6502's and 68k's
//! labels) the next word's a mnemonic's or a directive's vote too. A name the text `#define`s is
//! its own (linux's `#define al r0`), and a GAS `.macro` line's names its parameters' (linux's
//! x86 `.macro ROUND x0, t0`, X-184). A text may start in a `/* */` comment (a diff's hunk): a
//! line led by `*` is one of its lines, and a `*/` drops the votes before it. A line led by `|`
//! is a comment (68k's GAS).
//!
//! X-186, from a diff's hunks' misreads: RISC-V's and MIPS's registers vote only in lowercase
//! (`RA`, `RB` are linux's x86 macros' arguments); a line continued by `\` (or the one after it)
//! votes a register only for its statement's mnemonic's architectures, and no x86 or ARM
//! directive; an AArch64 arrangement (`T1.2d`) votes ARM whatever its name; the operands of GAS's
//! `.type`, `.size`, `.globl` … and a label's name, as a `#define`'s, are the text's own; ACME's
//! `!` votes only as a statement's first word (not cpp's `&& !defined`).
//!
//! NASM's preprocessor (`%include`, `%macro`, `%define`, its conditionals and `%rep`, at a line's
//! start) decides x86 alone. A tie is `default`'s where it's among the most voted, else the first
//! of them in [`X86`] … [`M68K`]'s order. A `.i` is 68k's or none ([`include`]).
//!
//! On this machine's clones (X-171's, X-181's and X-183's lists, the scratch classifiers'
//! dialects): see vault `ada-parse.md`'s X-171 to X-186 sections for the agreement.

use super::{
    ARCHES, ARM, HEAD as AT_HEAD, M65, M68K, MIPS, MNEMONIC, REGISTER, RV, VOTE, X86, Z80,
    starts_name, word_on_each,
};
use crate::Lang;

/// How much of a file is read.
const HEAD: usize = 64 * 1024;

/// The language of each architecture, in [`X86`] … [`M65`]'s order.
const LANGS: [Lang; ARCHES] = [
    Lang::X86Asm,
    Lang::ArmAsm,
    Lang::RiscvAsm,
    Lang::MipsAsm,
    Lang::Z80Asm,
    Lang::Mos6502Asm,
    Lang::M68kAsm,
];

/// The votes so far, by architecture.
pub(crate) type Votes = [u32; ARCHES];

/// `text`'s architecture, `default` where nothing in its head says.
pub(crate) fn language(text: &[u8], default: Lang) -> Lang {
    let Some(votes) = votes(text) else {
        return Lang::X86Asm;
    };
    let top = votes.iter().copied().max().unwrap_or(0);
    if top == 0 {
        return default;
    }
    if let Some(i) = LANGS.iter().position(|&l| l == default)
        && votes[i] == top
    {
        return default;
    }
    LANGS[votes.iter().position(|&v| v == top).unwrap_or(0)]
}

/// `text`'s language as a `.i` file's: 68k's where 68k is among the most voted, or where nothing
/// votes but a line assigns with `EQU` (the NDK's `_lib.i` tables: `_LVOOpen EQU -30`), else
/// none ([`crate::Lang::for_include`]).
pub(crate) fn include(text: &[u8]) -> Option<Lang> {
    let v = votes(text)?;
    let m = v[usize::from(M68K)];
    (m > 0 && v.iter().all(|&x| x <= m) || v == [0; ARCHES] && has_equ(text))
        .then_some(Lang::M68kAsm)
}

/// Whether a line of `text`'s head assigns with `EQU`: a name in the first column (its `:` too),
/// blanks, then the word.
fn has_equ(text: &[u8]) -> bool {
    text[..text.len().min(HEAD)]
        .split(|&c| c == b'\n')
        .any(|l| {
            if !l.first().is_some_and(|&c| starts_name(c) || c == b'.') {
                return false;
            }
            let e = name_end(l, 0);
            let e = e + usize::from(l.get(e) == Some(&b':'));
            let q = skip_blanks(l, e);
            q > e
                && l.get(q..q + 3)
                    .is_some_and(|w| w.eq_ignore_ascii_case(b"equ"))
                && !l
                    .get(q + 3)
                    .is_some_and(|&c| super::NAME_CONT[usize::from(c)])
        })
}

/// The votes of `text`'s head by architecture, in [`X86`] … [`M68K`]'s order; `None` where NASM's
/// preprocessor decides x86's.
pub(crate) fn votes(text: &[u8]) -> Option<Votes> {
    let b = &text[..text.len().min(HEAD)];
    let n = b.len();
    let mut votes: Votes = [0; ARCHES];
    let mut comment = false;
    let mut p = 0;
    let mut line_start = true;
    // the next word is a statement's first: a label, a mnemonic, a directive
    let mut head = true;
    // the next word follows a word in the first column without a `:`: maybe a label's, so this
    // one may be the statement's mnemonic or directive
    let mut after_label = false;
    // the names the text's `#define`s give (linux's `#define al r0`): its own, not registers
    let mut defined: Vec<&[u8]> = Vec::new();
    // the architectures the statement's mnemonic voted for (a bit each): its one-letter
    // registers vote for them (Z80's `ld a, l`, 6502's `lda $10, x`)
    let mut stmt = 0u8;
    // a line continued by `\`, or one that continues the line before, is a cpp macro's body or
    // call (a hunk may start inside one), whose names are often its parameters (linux's x86
    // crypto `vpxor x0, x3, t0; \`): a register votes only for its statement's mnemonic's
    // architectures (riscv-tests' `li x10, 2; \`), an x86 or ARM directive not at all (X-186).
    // `cont`: the line continues the one before; `pending`: the line's votes that count only if
    // it doesn't end with `\`, settled at its end (no look ahead along each line)
    let mut cont = false;
    let mut pending: Votes = [0; ARCHES];
    while p < n {
        let c = b[p];
        if comment {
            if c == b'*' && b.get(p + 1) == Some(&b'/') {
                comment = false;
                p += 2;
            } else {
                p += 1;
            }
            continue;
        }
        if c == b'\n' {
            cont = settle(&mut votes, &mut pending, b, p);
            if decided(&votes) {
                // the rest of the file won't overturn a lead this wide (X-181: the sniff's cost
                // in ada's path is a file's head, not its 64 KB)
                break;
            }
            line_start = true;
            head = true;
            after_label = false;
            stmt = 0;
            p += 1;
            continue;
        }
        if c == b' ' || c == b'\t' || c == b'\r' {
            p += 1;
            continue;
        }
        let first = line_start;
        line_start = false;
        match c {
            b'/' if b.get(p + 1) == Some(&b'*') => {
                comment = true;
                p += 2;
            }
            // a text that starts in a `/* */` comment (a diff's hunk, X-180): a `*/` ends it,
            // and what voted before it was the comment's; a line led by `*` is one of its lines,
            // as no statement starts so
            b'*' if b.get(p + 1) == Some(&b'/') => {
                votes = [0; ARCHES];
                pending = [0; ARCHES];
                p += 2;
            }
            b'*' if first => p = line_end(b, p),
            // 68k's GAS comment (linux's, GCC's MiNT output)
            b'|' if first => p = line_end(b, p),
            // a comment to the line's end: `;`, `//`, a `#` or `@` opening a line or after a blank
            b';' => p = line_end(b, p),
            // a string's words are its text's (pokered's `text "…if you…"`)
            b'"' => {
                p = b[p + 1..]
                    .iter()
                    .position(|&c| c == b'"' || c == b'\n')
                    .map_or(n, |i| p + 1 + i + usize::from(b[p + 1 + i] == b'"'));
                after_label = false;
            }
            b'/' if b.get(p + 1) == Some(&b'/') => p = line_end(b, p),
            b'#' | b'@' if first || p > 0 && matches!(b[p - 1], b' ' | b'\t') => {
                if c == b'#'
                    && let Some(name) = define(b, p + 1)
                {
                    defined.push(name);
                    // a `#define`'s body, its lines continued by `\`: its parameters' names are
                    // its own (X-184: linux's x86 crypto `x0`, `t0`)
                    p = line_end(b, p);
                    while p > 0 && p < n && b[..p].trim_ascii_end().ends_with(b"\\") {
                        p = line_end(b, p + 1);
                    }
                    continue;
                }
                p = line_end(b, p);
            }
            // NASM's macro-local (`%%A1`) and context-local (`%$x`) names: the text's own
            b'%' if matches!(b.get(p + 1), Some(b'%' | b'$')) => {
                p = name_end(b, p + 1);
                head = false;
                after_label = false;
            }
            b'%' if b.get(p + 1).is_some_and(|&c| starts_name(c)) => {
                let e = name_end(b, p + 1);
                let w = &b[p + 1..e];
                if first && is_nasm(w) {
                    return None;
                }
                let info = word_on_each(w);
                if info[usize::from(X86)] & 0xff == REGISTER {
                    votes[usize::from(X86)] += 2;
                }
                // MIT's `%d0`, `%sp` (X-183)
                if info[usize::from(M68K)] & 0xff == REGISTER {
                    votes[usize::from(M68K)] += 2;
                }
                p = e;
            }
            // a GAS macro's parameter (`\t6`, `\r2`): its own name, no register
            b'\\' if b.get(p + 1).is_some_and(|&c| starts_name(c)) => {
                p = name_end(b, p + 1);
                head = false;
                after_label = false;
            }
            // a number's run whole (`0FFh`, `3D0`): its letters are no name's
            _ if c.is_ascii_digit() => {
                p = name_end(b, p);
                head = false;
                after_label = false;
            }
            b'$' if b
                .get(p + 1)
                .is_some_and(|&c| starts_name(c) || c.is_ascii_digit()) =>
            {
                // MIPS's `$t0`; another `$name` votes as its name did (X-171)
                let e = name_end(b, p + 1);
                let w = &b[p + 1..e];
                if w.iter().all(u8::is_ascii_digit) {
                    // `$31`: MIPS's register by number, or an 8-bit's `$10`: no vote
                    head = false;
                    after_label = false;
                    p = e;
                    continue;
                }
                if w.iter().all(u8::is_ascii_hexdigit) {
                    // Z80's, 6502's and 68k's hex number (`$C0`, `$a4`, `$03D0`: a font's or a
                    // tune's data, X-182), its letters not a name's (ARM's `c0`, 68k's `d0`);
                    // MIPS's `$a0` and `$f4` are spelled so, and vote for none
                    if word_on_each(w)[usize::from(MIPS)] & 0xff != REGISTER {
                        votes[usize::from(Z80)] += 1;
                        votes[usize::from(M65)] += 1;
                        votes[usize::from(M68K)] += 1;
                    }
                    head = false;
                    after_label = false;
                    p = e;
                    continue;
                }
                if word_on_each(w)[usize::from(MIPS)] & 0xff == REGISTER {
                    votes[usize::from(MIPS)] += 2;
                } else {
                    p += 1;
                    continue;
                }
                head = false;
                after_label = false;
                p = e;
            }
            b'!' if (first || after_label || head && b[..p].trim_ascii_end().ends_with(b":"))
                && b.get(p + 1).is_some_and(|&c| starts_name(c)) =>
            {
                // ACME's `!byte`, `!macro`: a statement's first word, so not cpp's `&& !defined`
                // on a continued `#if` line (X-186)
                votes[usize::from(M65)] += 2;
                head = false;
                p = name_end(b, p + 1);
            }
            b'.' if (head || after_label) && b.get(p + 1).is_some_and(|&c| starts_name(c)) => {
                // a dotted directive at a head votes where the new columns mark it, and no more:
                // GAS's `.data`, `.if`, `.fill` are every architecture's (X-171 read them without
                // the dot, as armasm's `DATA`, `IF`, `FILL`)
                let e = name_end(b, p + 1);
                if b[p + 1..e].eq_ignore_ascii_case(b"macro") {
                    // a GAS macro's name and parameters, the text's own (X-184: linux's x86
                    // `.macro ROUND x0, t0`, RISC-V's names)
                    p = line_end(b, p);
                    continue;
                }
                let w = word_on_each(&b[p..e]);
                for a in usize::from(RV)..ARCHES {
                    if w[a] & VOTE != 0 && w[a] & AT_HEAD != 0 {
                        votes[a] += 2;
                    }
                }
                if symbols(&b[p + 1..e]) {
                    // GAS's directives whose operands are symbols (`.type f7, @object`: glibc's
                    // labels named as RISC-V's registers, X-186)
                    p = line_end(b, e);
                    continue;
                }
                head = b.get(e) == Some(&b':');
                after_label = false;
                p = e;
            }
            _ if starts_name(c) || c >= 0x80 && super::wide_name(b, p) => {
                let e = name_end(b, p);
                let first_word = head;
                let lead = head || after_label;
                let col0 = p == 0 || b[p - 1] == b'\n';
                head = b.get(e) == Some(&b':');
                after_label = false;
                if head {
                    // a label's name, no mnemonic's (X-184: NASM's `halt:` was Z80's `halt`), and
                    // the text's own after (X-186: glibc's `f7:` in `ASM_SIZE_DIRECTIVE(f7)`); a
                    // table's word only, so the list stays short
                    if word_on_each(&b[p..e]) != [0; ARCHES] {
                        defined.push(&b[p..e]);
                    }
                    p = e;
                    continue;
                }
                let whole = &b[p..e];
                if lead && sized(whole) {
                    // 68k's mnemonic with its size (`move.l`, `bne.s`; X-183), not MIPS's `move`
                    // nor RISC-V's `bne`
                    votes[usize::from(M68K)] += 2;
                    stmt = 1 << M68K;
                    p = e;
                    continue;
                }
                // a dotted word as a table has it (`ds.b`, not x86's `ds`), else a register's
                // arrangement or lane (`v0.16b`) as the register
                let mut info = word_on_each(whole);
                let mut w = whole;
                if info == [0; ARCHES]
                    && let Some(d) = whole.iter().position(|&c| c == b'.')
                {
                    if arrangement(&whole[d + 1..]) {
                        // AArch64's `.2d`, `.16b`: ARM's register whatever its name (linux's
                        // `T1.2d`, which x86's and RISC-V's `t1` had voted for; X-186)
                        votes[usize::from(ARM)] += 1;
                        p = e;
                        continue;
                    }
                    w = &whole[..d];
                    info = word_on_each(w);
                    // no x86 register takes a suffix
                    info[usize::from(X86)] = 0;
                }
                if first_word && is_req(b, e) {
                    // ARM's register alias (`t1 .req x10`): the text's own name, as a `#define`'s
                    defined.push(w);
                }
                if info != [0; ARCHES] && !defined.iter().any(|d| d.eq_ignore_ascii_case(w)) {
                    let voted = vote(
                        (&mut votes, &mut pending),
                        &info,
                        w,
                        (first_word, lead, stmt, cont),
                        b,
                        e,
                    );
                    if lead {
                        stmt = voted;
                    }
                }
                if first_word
                    && col0
                    && !head
                    && !info
                        .iter()
                        .any(|&i| i & AT_HEAD != 0 || i & 0xff == MNEMONIC)
                    && label_before(b, e)
                {
                    after_label = true;
                }
                p = e;
            }
            _ => p += 1,
        }
    }
    settle(&mut votes, &mut pending, b, p.min(n));
    Some(votes)
}

/// At the end of a line (`p`, its `\n` or the text's end), its pending votes counted unless it
/// ends with `\`; whether it does (the next line continues it).
fn settle(votes: &mut Votes, pending: &mut Votes, b: &[u8], p: usize) -> bool {
    let mut q = p;
    while q > 0 && matches!(b[q - 1], b' ' | b'\t' | b'\r') {
        q -= 1;
    }
    let continued = q > 0 && b[q - 1] == b'\\';
    if !continued {
        for (v, q) in votes.iter_mut().zip(pending.iter()) {
            *v += q;
        }
    }
    *pending = [0; ARCHES];
    continued
}

/// Whether `w` is 68k's mnemonic with its size (`move.l`, `bne.s`): no table's word whole, its
/// size a letter after its last `.`, its rest a 68k mnemonic.
fn sized(w: &[u8]) -> bool {
    let k = w.len();
    k > 2
        && w[k - 2] == b'.'
        && matches!(w[k - 1] | 0x20, b'b' | b'w' | b'l' | b's')
        && word_on_each(&w[..k - 2])[usize::from(M68K)] & 0xff == MNEMONIC
        && word_on_each(w) == [0; ARCHES]
}

/// How far ahead the most voted architecture must be for the sniff to stop reading.
const MARGIN: u32 = 64;

/// Whether one architecture leads every other by [`MARGIN`] votes.
fn decided(votes: &Votes) -> bool {
    let (mut top, mut second) = (0, 0);
    for &v in votes {
        if v > top {
            second = top;
            top = v;
        } else if v > second {
            second = v;
        }
    }
    top >= second + MARGIN
}

/// A word's votes, into `votes`, or `pending` where they count only on a line not continued by
/// `\`: `info`, what it is on each architecture; `first_word` where it's a statement's first,
/// `lead` where it may be its mnemonic or directive (after a label without its `:` too), `stmt`
/// the architectures the statement's mnemonic voted for, `cont` where the line continues the one
/// before (a register votes for `stmt`'s alone); `e` its end. The architectures it voted for as
/// a mnemonic, a bit each.
fn vote(
    (votes, pending): (&mut Votes, &mut Votes),
    info: &[u16; ARCHES],
    w: &[u8],
    (first_word, lead, stmt, cont): (bool, bool, u8, bool),
    b: &[u8],
    e: usize,
) -> u8 {
    let (x, a) = (info[usize::from(X86)], info[usize::from(ARM)]);
    let reg = |i: u16| i & 0xff == REGISTER;
    let dir = |i: u16| i & AT_HEAD != 0 && i & 0xff != REGISTER;
    // a statement's first word before `(` is cpp's macro's call (glibc's `ENTRY (f)`), not
    // armasm's `ENTRY`
    let call = first_word && b.get(skip_blanks(b, e)) == Some(&b'(');
    let new = &info[usize::from(RV)..];
    if first_word && info.iter().any(|&i| reg(i)) {
        // a mnemonic: ARM's `bl`, x86's `bl` register
    } else if is_word(w) {
        // MIPS's bare `at` and `zero` are English too (a hunk in a comment's text: "at your
        // option"); `$at` votes
    } else if w.len() == 1 || cont {
        // Z80's `a`, 6502's `x`, and a continued line's registers: for the architectures the
        // statement's mnemonic voted for; a continued line's x86 and ARM directives not at all,
        // being a macro call's arguments (pokered's `tmhm …, \` lists' `REST`, NASM's `rest`)
        for (k, &i) in info.iter().enumerate() {
            if reg(i) && stmt & 1 << k != 0 {
                votes[k] += 1;
            }
        }
    } else if info.iter().any(|&i| reg(i)) {
        for (k, &i) in info.iter().enumerate() {
            // ARM32's `r0`-`r7` aren't x86's here (x86inc's); RISC-V's and MIPS's registers are
            // written in lowercase (99.8% in X-181's files), so `RA`, `RB` are a macro's
            // arguments (linux's x86 crypto `S(S0, RA, RB)`, X-186)
            if reg(i)
                && !(k == usize::from(X86) && is_low_r(w))
                && !((k == usize::from(RV) || k == usize::from(MIPS)) && is_upper(w))
            {
                if stmt & 1 << k != 0 {
                    votes[k] += 1;
                } else {
                    pending[k] += 1;
                }
            }
        }
    } else if (dir(x) && a == 0 || dir(a) && x == 0)
        && !call
        && lead
        && !new.iter().any(|&i| dir(i) || i & 0xff == MNEMONIC)
    {
        // a directive only x86 or ARM has, and none of the others has as a directive or a
        // mnemonic (Z80's `IF`, `db`, `cp`), where a statement starts (an operand's `data` is a
        // symbol)
        pending[usize::from(if a == 0 { X86 } else { ARM })] += 2;
    }
    // the four new columns' marked words: a mnemonic or a directive at a head, an operand's word
    let mut voted = 0;
    if call {
        return voted;
    }
    for k in usize::from(RV)..ARCHES {
        let i = info[k];
        if i & VOTE == 0 {
            continue;
        }
        if i & AT_HEAD != 0 {
            if lead {
                votes[k] += 2;
            }
        } else if i & 0xff == MNEMONIC {
            if lead {
                votes[k] += 1;
                voted |= 1 << k;
            }
        } else if !lead {
            votes[k] += 1;
        }
    }
    voted
}

/// Whether the word in the first column ending at `e` is a label without its `:`: a word follows
/// it on its line.
fn label_before(b: &[u8], e: usize) -> bool {
    let q = skip_blanks(b, e);
    q > e && q < b.len() && (starts_name(b[q]) || b[q] == b'.')
}

/// A register's name that's an English word.
fn is_word(w: &[u8]) -> bool {
    w.eq_ignore_ascii_case(b"at") || w.eq_ignore_ascii_case(b"zero")
}

/// Whether ARM's `.req`, `.qn` or `.dn` follows a word ending at `e`.
fn is_req(b: &[u8], e: usize) -> bool {
    let q = skip_blanks(b, e);
    q > e
        && [&b".req"[..], b".qn", b".dn"].iter().any(|d| {
            b.get(q..q + d.len())
                .is_some_and(|t| t.eq_ignore_ascii_case(d))
                && !b
                    .get(q + d.len())
                    .is_some_and(|&c| super::NAME_CONT[usize::from(c)])
        })
}

/// Whether `w` has no lowercase letter.
fn is_upper(w: &[u8]) -> bool {
    !w.iter().any(u8::is_ascii_lowercase)
}

/// Whether `t`, a register's suffix past its `.`, is AArch64's arrangement (`16b`, `2d`, `4s`).
fn arrangement(t: &[u8]) -> bool {
    match t {
        [d @ .., k] if !d.is_empty() && d.iter().all(u8::is_ascii_digit) => {
            matches!(k | 0x20, b'b' | b'h' | b's' | b'd' | b'q')
        }
        _ => false,
    }
}

/// GAS's directives (without the dot) whose operands are symbols, not registers.
fn symbols(w: &[u8]) -> bool {
    [
        &b"type"[..],
        b"size",
        b"globl",
        b"global",
        b"hidden",
        b"weak",
        b"local",
        b"protected",
        b"internal",
        b"ent",
        b"end",
        b"func",
        b"endfunc",
    ]
    .iter()
    .any(|k| w.eq_ignore_ascii_case(k))
}

/// ARM32's `r0`-`r7`, which x86inc's macros are named too.
fn is_low_r(w: &[u8]) -> bool {
    w.len() == 2 && (w[0] | 0x20) == b'r' && (b'0'..=b'7').contains(&w[1])
}

/// NASM's preprocessor's words that only x86's assemblers have (X-186: its conditionals, `%rep`
/// and a macro's end too: NASM's tests' `%ifdef ERROR` among `jp .L1`s, a hunk of FFmpeg's
/// `%if`, `%endmacro`).
fn is_nasm(w: &[u8]) -> bool {
    [
        &b"include"[..],
        b"macro",
        b"define",
        b"xdefine",
        b"idefine",
        b"assign",
        b"imacro",
        b"endmacro",
        b"endm",
        b"unmacro",
        b"undef",
        b"if",
        b"ifdef",
        b"ifndef",
        b"ifidn",
        b"ifidni",
        b"ifmacro",
        b"ifctx",
        b"ifnum",
        b"ifstr",
        b"ifid",
        b"elif",
        b"elifdef",
        b"elifndef",
        b"else",
        b"endif",
        b"rep",
        b"endrep",
        b"exitrep",
        b"rotate",
        b"strlen",
        b"substr",
    ]
    .iter()
    .any(|k| w.eq_ignore_ascii_case(k))
}

/// The name a `#define` gives, `p` past its `#`.
fn define(b: &[u8], p: usize) -> Option<&[u8]> {
    let q = skip_blanks(b, p);
    if !b[q..].starts_with(b"define") {
        return None;
    }
    let r = skip_blanks(b, q + 6);
    (r > q + 6 && r < b.len() && starts_name(b[r])).then(|| &b[r..name_end(b, r)])
}

fn skip_blanks(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && matches!(b[p], b' ' | b'\t') {
        p += 1;
    }
    p
}

fn line_end(b: &[u8], p: usize) -> usize {
    b[p..]
        .iter()
        .position(|&c| c == b'\n')
        .map_or(b.len(), |i| p + i)
}

/// A name's end, its `.`-joined parts with it (`v0.16b`), its letters past ASCII too.
fn name_end(b: &[u8], p: usize) -> usize {
    let mut e = p + 1;
    loop {
        while e < b.len() && (super::NAME_CONT[usize::from(b[e])] || b[e] == b'.') {
            e += 1;
        }
        let w = if e < b.len() && b[e] >= 0x80 {
            super::wide_name_end(b, e)
        } else {
            e
        };
        if w == e {
            return e;
        }
        e = w;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registers_decide() {
        let att = b"\t.text\n\t.globl\tmain\nmain:\n\tmovq\t%rsp, %rbp\n\tret\n";
        assert_eq!(language(att, Lang::ArmAsm), Lang::X86Asm);
        let a64 = b"\t.text\n\t.global f\nf:\n\tstp x29, x30, [sp, #-16]!\n\tld1 {v0.16b}, [x1]\n";
        assert_eq!(language(a64, Lang::X86Asm), Lang::ArmAsm);
        let a32 = b"f:\n\tpush {r4, lr} @ save\n\tldr r0, [r1, #4]\n\tbx lr\n";
        assert_eq!(language(a32, Lang::X86Asm), Lang::ArmAsm);
        // ARM's `bl` is a mnemonic, not x86's register
        let bl = b".globl _start\n_start:\n    b reset\nreset:\n    bl notmain\n    str r1,[r0]\n";
        assert_eq!(language(bl, Lang::X86Asm), Lang::ArmAsm);
        let intel = b"section .text\nglobal _start\n_start:\n    mov rax, 60\n    syscall\n";
        assert_eq!(language(intel, Lang::ArmAsm), Lang::X86Asm);
    }

    #[test]
    fn nasm_and_x86inc_are_x86() {
        // x86inc's `r0`..`r6` are ARM's names too; NASM's preprocessor decides
        let x =
            b"%include \"x86inc.asm\"\ncglobal f, 3\n    mova m0, [r0]\n    add r1, r2\n    RET\n";
        assert_eq!(language(x, Lang::ArmAsm), Lang::X86Asm);
    }

    #[test]
    fn armasm_and_iar_are_arm() {
        let keil = b"; startup\n                AREA    RESET, DATA, READONLY\n__Vectors       DCD     0\n";
        assert_eq!(language(keil, Lang::X86Asm), Lang::ArmAsm);
        let iar =
            b"  MODULE  ?cstartup\n  PUBWEAK Reset_Handler\n  SECTION .text:CODE:REORDER(2)\n";
        assert_eq!(language(iar, Lang::X86Asm), Lang::ArmAsm);
    }

    /// X-180: a diff's hunk may start in a comment: its `*` lines, and what precedes its `*/`,
    /// don't vote.
    #[test]
    fn a_comments_rest_doesnt_vote() {
        let t = b" * cpu: times to do\n * clobbers r0-r3\n */\n\tmovq\t%rax, %rbx\n";
        assert_eq!(language(t, Lang::ArmAsm), Lang::X86Asm);
        let t = b"  the cpu, its label\n  and default bits */\n\tldr\tx0, [x1]\n";
        assert_eq!(language(t, Lang::X86Asm), Lang::ArmAsm);
        assert_eq!(language(b" * eax, ebx\n", Lang::ArmAsm), Lang::ArmAsm);
    }

    /// X-180: glibc's `ENTRY (f)` is cpp's macro, not armasm's `ENTRY`; a name the text
    /// `#define`s is its own, not x86's `al`.
    #[test]
    fn a_macros_names_dont_vote() {
        let t = b"ENTRY (f)\n\tsubq\t$64, %rsp\nENTRY(g)\n";
        assert_eq!(language(t, Lang::ArmAsm), Lang::X86Asm);
        let t = b"#define al r0\n# define ah r1\n\tmovmi\tah, ah, lsl r2\n\tmov\tal, al\n";
        assert_eq!(language(t, Lang::X86Asm), Lang::ArmAsm);
    }

    #[test]
    fn comments_dont_vote_and_a_tie_is_the_names() {
        let t = b"; mov eax, ebx\n// ldr x0, [x1]\n/* rax\n x0 */\n";
        assert_eq!(language(t, Lang::ArmAsm), Lang::ArmAsm);
        assert_eq!(language(t, Lang::X86Asm), Lang::X86Asm);
        assert_eq!(language(b"", Lang::ArmAsm), Lang::ArmAsm);
    }

    /// X-181: RISC-V's and MIPS's mnemonics and registers, `$t0`, `.set noreorder`.
    #[test]
    fn risc_v_and_mips() {
        let rv = b"\t.globl f\nf:\n\taddi\tsp, sp, -16\n\tsd\tra, 8(sp)\n\tmv\ta0, s0\n\tecall\n";
        assert_eq!(language(rv, Lang::ArmAsm), Lang::RiscvAsm);
        let mars =
            b".data\nmsg: .asciiz \"hi\"\n.text\nmain:\n\tli $v0, 4\n\tla $a0, msg\n\tsyscall\n";
        assert_eq!(language(mars, Lang::X86Asm), Lang::MipsAsm);
        // linux's MIPS: `regdef.h`'s names without `$`
        let linux =
            b"\t.set\tnoreorder\nLEAF(f)\n\tlw\tv0, 20(sp)\n\tmove\tv1, a3\n\tjr\tra\n\taddiu\tsp, sp, 8\n";
        assert_eq!(language(linux, Lang::ArmAsm), Lang::MipsAsm);
    }

    /// X-181: Z80's and SM83's registers and mnemonics; 6502's mnemonics, labels in the first
    /// column without `:`, ca65's directives.
    #[test]
    fn z80_and_6502() {
        let z80 =
            b"start:\n\tld hl, buffer\n\tld b, 16\nloop:\tld (hl), a\n\tinc hl\n\tdjnz loop\n\tret\n";
        assert_eq!(language(z80, Lang::X86Asm), Lang::Z80Asm);
        let rgbds =
            b"SECTION \"x\", ROM0\nFunc::\n\tldh a, [hJoypad]\n\tld [hli], a\n\tjr nz, .loop\n";
        assert_eq!(language(rgbds, Lang::X86Asm), Lang::Z80Asm);
        let dasm = b"\tprocessor 6502\n\torg $f000\nStart\tsei\n\tcld\n\tldx #$ff\n\ttxs\nloop\tsta WSYNC\n\tdex\n";
        assert_eq!(language(dasm, Lang::X86Asm), Lang::Mos6502Asm);
        let ca65 =
            b"\t.segment \"CODE\"\n\t.proc main\n\tlda #0\n\tsta $2000\n\tjsr init\n\trts\n\t.endproc\n";
        assert_eq!(language(ca65, Lang::ArmAsm), Lang::Mos6502Asm);
    }

    /// X-182: a font's or a tune's hex bytes (`$A0`, `$C0`) aren't MIPS's `$a0` or ARM's `c0`; an
    /// 8-bit architecture's. MIPS's `$a0`, `$f4` vote for neither; its `$t0` still MIPS's.
    #[test]
    fn hex_bytes_arent_registers() {
        let font = b"_font:\n\t.byte $A0, $A1, $C0, $C4\n\t.byte $a2, $a3, $c1, $e0\n";
        assert_eq!(votes(font).unwrap()[usize::from(MIPS)], 0);
        assert_eq!(votes(font).unwrap()[usize::from(ARM)], 0);
        assert_ne!(language(font, Lang::ArmAsm), Lang::ArmAsm);
        let fpu = b"\tlwc1 $f4, ($a0)\n\tmul.s $f10, $f4, $f4\n\tli $t0, 1\n";
        assert_eq!(language(fpu, Lang::ArmAsm), Lang::MipsAsm);
    }

    /// X-186: the hunks' misreads into another family. NASM's conditionals decide x86; an
    /// AArch64 arrangement (`T1.2d`) is ARM's whatever its name; RISC-V's and MIPS's registers in
    /// uppercase are a macro's arguments; a continued line's registers vote only with their
    /// statement's mnemonic; `.type`'s operand and a label's name are the text's own; `eret`
    /// and `sb` are AArch64's too; `!defined` is cpp's, not ACME's `!byte`.
    #[test]
    fn the_hunks_misreads() {
        let t = b"\tjp .L1\n\tjp .L1\n%ifdef ERROR\n\tjp .L1\n%endif\n";
        assert_eq!(language(t, Lang::ArmAsm), Lang::X86Asm);
        let t = b"\tld1\t{T1.2d}, [x2], #16\n\trev64\tT1.16b, T1.16b\n";
        assert_eq!(language(t, Lang::X86Asm), Lang::ArmAsm);
        let t = b"\tS(S0, RA, RB, RC, RD, RE);\tLK2(RC, RB, RD, RA, RE, 1);\n";
        assert_eq!(votes(t).unwrap()[usize::from(RV)], 0);
        let t = b"\tvpxor x0, x3, t0;\t\\\n\tvpxor x1, x0, t1;\t\\\n\tvpxor t2, x0, x0;\n";
        assert_eq!(votes(t).unwrap(), [0; ARCHES]);
        // riscv-tests' bodies: a mnemonic of its own backs a continued line's registers
        let t = b"  TEST_CASE( 2, x14, 0x3, \\\n    li x10, 0x2; \\\n    li x11, 0x1; \\\n";
        assert_eq!(language(t, Lang::ArmAsm), Lang::RiscvAsm);
        let t = b"\t.type f7,@object\nf7:\t.double -0.1\n\tASM_SIZE_DIRECTIVE(f7)\n";
        assert_eq!(votes(t).unwrap(), [0; ARCHES]);
        let t = b"\trestore_callee_saved_regs x29\n\teret\n\tsb\n";
        assert_eq!(votes(t).unwrap()[usize::from(MIPS)], 0);
        assert_eq!(
            votes(t).unwrap()[usize::from(RV)],
            votes(t).unwrap()[usize::from(ARM)]
        );
        let t = b"#if !defined (__ARM_ARCH_2__) \\\n     && !defined (__ARM_ARCH_4T__)\n";
        assert_eq!(votes(t).unwrap()[usize::from(M65)], 0);
        assert_eq!(votes(b"\t!byte 1, 2\n").unwrap()[usize::from(M65)], 2);
    }

    /// X-186: a `.i` is 68k's where its text votes for 68k, else plain (SWIG's, `gcc -E`'s).
    #[test]
    fn a_dot_i_is_68ks_or_plain() {
        let ndk = b"\tIFND\tEXEC_TYPES_I\nEXEC_TYPES_I\tSET\t1\n\tSTRUCTURE\tLN,0\n\tAPTR\tLN_SUCC\n\tLABEL\tLN_SIZE\n";
        assert_eq!(include(ndk), Some(Lang::M68kAsm));
        let swig = b"%module example\n%{\n#include \"example.h\"\n%}\n%include \"example.h\"\n";
        assert_eq!(include(swig), None);
        let cpp =
            b"# 1 \"a.c\"\nextern int printf(const char *, ...);\ntypedef unsigned long size_t;\n";
        assert_eq!(include(cpp), None);
        let lvo = b"_LVOOpen\t\tEQU\t-30\n_LVOClose:\tequ\t-36\n";
        assert_eq!(include(lvo), Some(Lang::M68kAsm));
        assert_eq!(include(b"x = equal\n"), None);
        assert_eq!(include(b""), None);
    }

    /// X-182: 8080's mnemonics vote for Z80's scan, which reads them (not 6502's `lda`, `sta`).
    #[test]
    fn i8080_is_z80s() {
        let t = b"\tLXI\tH,BUF\n\tMVI\tA,0\n\tLDA\tCOUNT\n\tSTA\tSAVE\n\tDAD\tD\n\tMOV\tA,M\n";
        assert_eq!(language(t, Lang::X86Asm), Lang::Z80Asm);
    }
}
