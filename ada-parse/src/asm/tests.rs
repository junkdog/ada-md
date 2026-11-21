//! Assembly's scanner's tests (X-171): a statement's parts, the dialect read from the text,
//! the preprocessors' lines and NASM's macro bodies, x86's and ARM's corners, a block against its
//! lines, long lines, a lone opener at a line's end.

use super::*;
use crate::Lang;
use crate::tests::marked;

/// Each line of `text` lexed alone by `lang`'s line function from the state the one before
/// left: its spans (checked in order, on the line) and the state after it.
fn lines(text: &str, arm: bool) -> Vec<(Vec<crate::Span>, State)> {
    let mut state = State::default();
    let mut out = Vec::new();
    for l in text.lines() {
        let mut spans = Vec::new();
        if arm {
            line_arm(l.as_bytes(), &mut state, &mut Out::new(&mut spans));
        } else {
            line(l.as_bytes(), &mut state, &mut Out::new(&mut spans));
        }
        let mut at = 0;
        for s in &spans {
            assert!(
                s.start as usize >= at && s.end >= s.start && s.end as usize <= l.len(),
                "{l:?}: {spans:?}"
            );
            at = s.end as usize;
        }
        out.push((spans, state));
    }
    out
}

fn of(lang: Lang, lines: &[&str]) -> Vec<(String, String)> {
    marked(lang, lines)
        .into_iter()
        .map(|(t, s)| (t.to_owned(), s.to_owned()))
        .collect()
}

/// `text` among `lines`' spans with `scopes` (outermost first, without the base).
fn has(lang: Lang, lines: &[&str], text: &str, scopes: &str) {
    let got = of(lang, lines);
    assert!(
        got.iter().any(|(t, s)| t == text && s == scopes),
        "{text:?} as {scopes:?} in {got:#?}"
    );
}

/// No span of `lines` is `text`'s with `scopes`.
fn hasnt(lang: Lang, lines: &[&str], text: &str, scopes: &str) {
    let got = of(lang, lines);
    assert!(
        !got.iter().any(|(t, s)| t == text && s == scopes),
        "{text:?} as {scopes:?} in {got:#?}"
    );
}

const X86: Lang = Lang::X86Asm;
const ARM_: Lang = Lang::ArmAsm;
const RV_: Lang = Lang::RiscvAsm;
const MIPS_: Lang = Lang::MipsAsm;
const Z80_: Lang = Lang::Z80Asm;
const M65_: Lang = Lang::Mos6502Asm;
const M68K_: Lang = Lang::M68kAsm;
/// Every architecture (X-181, X-183).
const EVERY: [Lang; 7] = [X86, ARM_, RV_, MIPS_, Z80_, M65_, M68K_];
const MNEM: &str = "keyword.operator.word.mnemonic.asm";
const REG: &str = "constant.language.register.asm";
const NAME_: &str = "entity.name.constant.asm";
const LINE: &str = "comment.line.asm";

#[test]
fn a_statement_reads_label_mnemonic_operands() {
    let l = &["start: mov rax, [rbx+8] ; c"];
    has(X86, l, "start", NAME_);
    has(X86, l, ":", "punctuation.separator.asm");
    has(X86, l, "mov", MNEM);
    has(X86, l, "rax", REG);
    has(X86, l, "rbx", REG);
    has(X86, l, "8", "constant.numeric.decimal.asm");
    has(
        X86,
        l,
        ";",
        "comment.line.asm punctuation.definition.comment.asm",
    );
    has(X86, l, " c", LINE);
}

#[test]
fn a_macro_or_an_unknown_mnemonic_is_a_mnemonic() {
    // what a word is comes from where it stands: AT&T's suffixes, x86inc's macros
    for l in [
        "\tmovl %eax, %ebx",
        "cglobal f, 3",
        "\tvpdpbusd zmm0, zmm1, zmm2",
    ] {
        let w = l.trim_start().split(' ').next().unwrap();
        has(X86, &[l], w, MNEM);
    }
}

#[test]
fn x86s_registers_and_operands() {
    let l = &["    vpaddw ymm1{k1}{z}, ymm2, dword [rax+rcx*8+0x10]"];
    has(X86, l, "ymm1", REG);
    has(X86, l, "k1", REG);
    has(X86, l, "z", "storage.modifier.asm");
    has(X86, l, "dword", "storage.type.asm");
    has(X86, l, "0x10", "constant.numeric.hex.asm");
    // x86inc's names for the registers
    has(X86, &["    mova m0, [r1q+r2*4]"], "m0", REG);
    has(X86, &["    mova m0, [r1q+r2*4]"], "r1q", REG);
    // x86inc's temporaries by their shape (X-186): `t0`-`t14`, a size after
    for t in ["t0", "t3d", "T0d", "t14", "t9b", "t1q"] {
        has(X86, &[&format!("    movzx {t}, byte [r0]")], t, REG);
    }
    for t in ["t15", "t1x", "tmp", "t", "t01"] {
        hasnt(X86, &[&format!("    mov {t}, 1")], t, REG);
    }
    // a prefix keeps the head: the mnemonic after it
    has(
        X86,
        &["    rep movsb"],
        "rep",
        "storage.modifier.prefix.asm",
    );
    has(X86, &["    rep movsb"], "movsb", MNEM);
    has(
        X86,
        &["    lea rax, [rel msg]"],
        "rel",
        "storage.modifier.asm",
    );
}

#[test]
fn att_registers_immediates_and_gas() {
    let l = &[
        "\tmovq %rsp, %rbp",
        "\tmovl $0, -4(%rbp)  # c",
        "\tcall f@PLT",
    ];
    has(X86, l, "%rsp", REG);
    has(X86, l, "$0", "constant.numeric.decimal.asm");
    has(X86, l, " c", LINE);
    has(X86, l, "@PLT", "storage.modifier.asm");
    // an AT&T register makes the file GAS's
    let s = lines("\tmovq %rsp, %rbp\n", false);
    assert!(s[0].1.arg & GAS != 0);
}

#[test]
fn labels_without_a_colon() {
    let l = &[
        "msg db 'hi', 10",
        "len equ $ - msg",
        "x = 4",
        "f PROC FRAME",
    ];
    has(X86, l, "msg", NAME_);
    has(X86, l, "db", "storage.type.asm");
    has(X86, l, "len", NAME_);
    has(X86, l, "equ", "support.function.directive.asm");
    has(X86, l, "x", NAME_);
    has(X86, l, "f", NAME_);
    // NASM's struc's fields
    has(
        X86,
        &["struc s", "    .b0 resq 1", "endstruc"],
        ".b0",
        NAME_,
    );
    // a mnemonic before its operand isn't one
    has(X86, &["    mov dword [x], 1"], "mov", MNEM);
    // armasm's: a word in the first column
    let a = &[
        "Reset_Handler   PROC",
        "loop  SUBS r0, r0, #1",
        "        BNE loop",
    ];
    has(ARM_, a, "Reset_Handler", NAME_);
    has(ARM_, a, "loop", NAME_);
    has(ARM_, a, "SUBS", MNEM);
}

/// X-180: MASM's data typed by a size word: before `?`, or in the first column before a value;
/// an instruction's sized operand stays one.
#[test]
fn masms_typed_data() {
    let l = &[
        "x DWORD ?",
        "msg BYTE \"hi\",0",
        "    pt REAL8 ?",
        "HFLIP byte, b, 1",
    ];
    for name in ["x", "msg", "pt"] {
        has(X86, l, name, NAME_);
    }
    has(X86, l, "DWORD", "storage.type.asm");
    has(X86, l, "HFLIP", MNEM);
    for l in [
        "    push dword 0",
        "    mov DWORD PTR [rax], 1",
        "mov dword [x], 1",
        "mov DWORD PTR [rax], 1",
        "BYTE PTR ES:[EDI],BYTE PTR DS:[ESI]",
    ] {
        has(X86, &[l], l.split_whitespace().next().unwrap(), MNEM);
    }
}

/// X-180: an instruction as a macro's argument: x86inc's `REPX {…}` and an `_INSTR` macro's
/// first; a brace's expression (`{arg1 + 8}`) and a decorator (`{k1}`) aren't one.
#[test]
fn a_mnemonic_as_a_macros_argument() {
    let l = &[
        "    REPX {pmulhrsw x, m5}, m0, m1",
        "    REPX {psrad x, 4}, m2",
        "AVX_INSTR addpd, sse2, 1, 0, 1",
        "    GCM_INIT arg1, {arg2 + 8}, {rsp + 8}",
    ];
    for w in ["pmulhrsw", "psrad", "addpd"] {
        has(X86, l, w, MNEM);
    }
    has(X86, l, "x", NAME_);
    has(X86, l, "sse2", NAME_);
    has(X86, l, "arg2", NAME_);
    has(X86, l, "rsp", REG);
    has(X86, &["    vpaddw ymm1{k1}, ymm2, ymm3"], "k1", REG);
    // a line led by the brace (NASM's tests' continued `testcase`): its operands after it
    let l = &["\t\t{adc dword [rdx], byte 0x7F}"];
    has(X86, l, "adc", MNEM);
    has(X86, l, "dword", "storage.type.asm");
}

/// X-180: NASM's symbol's type and visibility after `:`; a segment's register before `:`
/// keeps its operand a name.
#[test]
fn nasms_symbol_types() {
    let l = &["global f:function hidden", "global x:data 4"];
    has(X86, l, "function", "storage.type.directive.asm");
    has(X86, l, "data", "storage.type.directive.asm");
    has(
        X86,
        l,
        "hidden",
        "storage.modifier.directive.asm variable.parameter.directive.asm",
    );
    has(X86, &["    mov eax, [fs:data]"], "data", NAME_);
}

/// X-180: armasm's `|…|` symbols, a section's name after `AREA`; `|` an operator after an
/// operand and in GAS's files.
#[test]
fn armasms_quoted_symbols() {
    let l = &[
        "        AREA    |.text|, CODE, READONLY",
        "        IMPORT  |Image$$ER$$Base|",
        "        LDR     r2, =|Image$$ER$$Base|",
        "        MOV     r0, #(1|2|4)",
    ];
    has(
        ARM_,
        l,
        "|.text|",
        "entity.name.section.asm string.unquoted.asm support.constant.section.asm",
    );
    has(ARM_, l, "|Image$$ER$$Base|", NAME_);
    hasnt(ARM_, l, "|2|", NAME_);
    has(ARM_, l, "|", "keyword.operator.bitwise.asm");
    let gas = &["\t.text", "\torr\tr0, r0, #(1|2)"];
    hasnt(ARM_, gas, "|2|", NAME_);
}

#[test]
fn local_and_numeric_labels() {
    has(X86, &[".loop:", "    jmp .loop"], ".loop", NAME_);
    let g = &["\t.text", "1:\tdec %ecx", "\tjnz 1b", "\tjmp 2f"];
    has(X86, g, "1", NAME_);
    has(X86, g, "1b", NAME_);
    has(X86, g, "2f", NAME_);
    // NASM's binary
    has(
        X86,
        &["    mov al, 1011b"],
        "1011b",
        "constant.numeric.binary.asm",
    );
    has(
        X86,
        &["    mov al, 0FFh"],
        "0FFh",
        "constant.numeric.hex.asm",
    );
    has(
        X86,
        &["    mov al, 17q"],
        "17q",
        "constant.numeric.octal.asm",
    );
}

#[test]
fn a_semicolon_by_the_dialect() {
    // NASM's comment
    has(X86, &["mov eax, 1 ; c"], " c", LINE);
    // GAS's separator: the next statement's head
    let g = &["\t.text", "\tpush %rax; pop %rbx"];
    has(X86, g, "pop", MNEM);
    hasnt(X86, g, " pop %rbx", LINE);
    // a `;` opening a statement is a comment, and then everywhere (Apple's arm64)
    let a = &["\t.global _main", "; set up", "\tmov x0, #1 ; one"];
    has(ARM_, a, " set up", LINE);
    has(ARM_, a, " one", LINE);
}

#[test]
fn comments_by_the_architecture() {
    let a = &["\tldr r0, [r1, #4] @ c", "\tadd x0, x1, #-16 // d", "# e"];
    has(ARM_, a, "#4", "constant.numeric.decimal.asm");
    has(ARM_, a, "#-16", "constant.numeric.decimal.asm");
    has(ARM_, a, " c", LINE);
    has(ARM_, a, " d", LINE);
    has(ARM_, a, " e", LINE);
    // `@` after a name's byte: a relocation
    has(
        ARM_,
        &["\tadrp x0, _sym@PAGE"],
        "@PAGE",
        "storage.modifier.asm",
    );
    // a block comment across lines
    let b = &["/* a", " b */ mov eax, 1"];
    has(X86, b, " b ", "comment.block.asm");
    has(X86, b, "mov", MNEM);
}

#[test]
fn arms_operands() {
    let l = &[
        "\tld1 {v0.16b, v1.16b}, [x1], #32",
        "\tadd x0, x0, :lo12:sym",
        "\tmsr sctlr_el1, x0",
        "\tcsel w0, w1, wzr, eq",
        "\tadd r0, r0, r1, lsl #2",
    ];
    has(ARM_, l, "v0.16b", REG);
    has(ARM_, l, ":lo12:", "storage.modifier.asm");
    has(ARM_, l, "sctlr_el1", REG);
    has(ARM_, l, "wzr", REG);
    has(ARM_, l, "eq", "storage.modifier.asm");
    has(ARM_, l, "lsl", "storage.modifier.asm");
    // x86's registers aren't ARM's
    has(ARM_, &["\tmov eax, 1"], "eax", NAME_);
}

#[test]
fn directives_and_sections() {
    let l = &[
        "section .text",
        "\t.section .rodata, \"a\"",
        "global _start",
        "\t.globl main",
    ];
    has(
        X86,
        l,
        ".text",
        "entity.name.section.asm string.unquoted.asm support.constant.section.asm",
    );
    has(
        X86,
        l,
        ".rodata",
        "entity.name.section.asm string.unquoted.asm support.constant.section.asm",
    );
    has(X86, l, "global", "support.function.directive.asm");
    has(X86, l, ".globl", "support.function.directive.asm");
    has(
        X86,
        &["\t.type f, %function"],
        "%function",
        "storage.type.directive.asm",
    );
}

#[test]
fn cpps_lines() {
    let l = &[
        "#include <asm/linkage.h>",
        "#define N 4",
        "#define F(a, b) \\",
        "\tadd a, b",
        "\tmov eax, N",
    ];
    has(
        X86,
        l,
        "#",
        "meta.preprocessor.asm keyword.control.preprocessor.asm punctuation.definition.keyword.preprocessor.asm",
    );
    has(
        X86,
        l,
        "include",
        "meta.preprocessor.asm keyword.control.preprocessor.asm",
    );
    has(
        X86,
        l,
        "asm/linkage.h",
        "meta.preprocessor.asm string.quoted.other.lt-gt.include.asm",
    );
    has(
        X86,
        l,
        "N",
        "meta.preprocessor.asm entity.name.constant.preprocessor.asm",
    );
    has(
        X86,
        l,
        "F",
        "meta.preprocessor.asm entity.name.function.preprocessor.asm",
    );
    has(
        X86,
        l,
        "a",
        "meta.preprocessor.asm variable.parameter.preprocessor.asm",
    );
    // the continued line is the directive's, a body of statements; the one after isn't
    has(
        X86,
        l,
        "add",
        "meta.preprocessor.asm keyword.operator.word.mnemonic.asm",
    );
    has(X86, l, "mov", MNEM);
    // a `#` that isn't cpp's opens a comment
    has(X86, &["# not a directive"], " not a directive", LINE);
}

#[test]
fn nasms_preprocessor_and_macro_bodies() {
    let l = &[
        "%macro SAD 2 ; w, h",
        "cglobal pixel_sad_%1x%2, 4",
        "%%loop:",
        "    jnz %%loop",
        "%endmacro",
        "    mov eax, ebx",
        "%define X 1",
        "%ifdef X",
        "%if ARCH",
    ];
    has(
        X86,
        l,
        "SAD",
        "meta.preprocessor.asm entity.name.function.preprocessor.asm",
    );
    has(
        X86,
        l,
        "2",
        "meta.preprocessor.asm variable.parameter.preprocessor.asm",
    );
    // the body: the preprocessor's text, its plain text too
    has(
        X86,
        l,
        "cglobal",
        "meta.preprocessor.asm keyword.operator.word.mnemonic.asm",
    );
    has(
        X86,
        l,
        "1",
        "meta.preprocessor.asm variable.other.preprocessor.asm",
    );
    has(
        X86,
        l,
        "loop",
        "meta.preprocessor.asm entity.name.constant.special.asm",
    );
    has(X86, l, ",", "meta.preprocessor.asm operator.separator.asm");
    has(
        X86,
        &["%macro A 0", "  mov eax, ebx", "%endmacro"],
        "  ",
        "meta.preprocessor.asm",
    );
    // past `%endmacro`, code again
    has(X86, l, "ebx", REG);
    has(
        X86,
        l,
        "X",
        "meta.preprocessor.asm entity.name.constant.preprocessor.asm",
    );
    // a condition's keyword only
    has(X86, l, "ARCH", NAME_);
}

#[test]
fn gas_macros() {
    let l = &[
        ".macro m reg, n=1",
        "\tadd \\reg, \\n",
        "\tvmov v\\n\\().16b, \\@",
        ".endm",
    ];
    has(X86, l, "m", "entity.name.function.preprocessor.asm");
    has(X86, l, "reg", "variable.parameter.preprocessor.asm");
    has(
        X86,
        l,
        "\\",
        "punctuation.definition.variable.preprocessor.asm keyword.control.preprocessor.asm",
    );
    has(X86, l, "reg", "variable.other.preprocessor.asm");
    has(X86, l, "\\@", "variable.other.preprocessor.asm");
}

#[test]
fn strings_by_the_dialect() {
    // NASM's `"…"` takes no escape; its backquoted string does
    let n = &["db \"a\\n\", `b\\n`"];
    hasnt(
        X86,
        n,
        "\\n",
        "string.quoted.double.asm constant.character.escape.asm",
    );
    has(
        X86,
        n,
        "\\n",
        "string.quoted.other.asm constant.character.escape.asm",
    );
    // GAS's does, and its `'a` is a character
    let g = &["\t.ascii \"a\\n\"", "\tcmpb $'a, %al"];
    has(
        X86,
        g,
        "\\n",
        "string.quoted.double.asm constant.character.escape.asm",
    );
    has(X86, g, "a", "string.quoted.single.asm");
    has(X86, g, "%al", REG);
}

/// The state's fields: each kept as it is, the macros saturating.
#[test]
fn the_state_keeps_what_a_line_hands_on() {
    let s = lines("/* a\n", false);
    assert!(s[0].1.arg & M_COMMENT != 0);
    let s = lines("#define X \\\n", false);
    assert!(s[0].1.arg & CONT != 0);
    let deep = "%macro a 0\n".repeat(5) + &"%endmacro\n".repeat(5);
    let s = lines(&deep, false);
    assert_eq!(s[4].1.arg >> MACROS & 3, 3, "saturates");
    assert_eq!(s.last().unwrap().1.arg >> MACROS & 3, 0);
}

#[test]
fn a_block_lexes_as_its_lines() {
    for arm in [false, true] {
        let mut text = String::new();
        text.push_str("; nasm first\n%macro A 1\n%macro B 0\n%macro C 0\n%macro D 0\n");
        text.push_str("    mov eax, %1\n%endmacro\n%endmacro\n/* a\n b */ x:\n");
        text.push_str("#define F(a) \\\n  add a, a\n\t.text\n\tpush %rax; pop %rbx\n; c\n");
        text.push_str("\tldr x0, [x1, #8] @ c\n%endmacro\n%endmacro\n%endmacro\n");
        let one = lines(&text, arm);
        let mut state = State::default();
        let mut all = Vec::new();
        let mut v = Vec::new();
        let pass = &mut |c: &mut dyn crate::Carry| {
            for l in text.lines() {
                all.push(c.next(l.as_bytes()).to_vec());
            }
            None
        };
        if arm {
            block_arm(&mut state, &mut v, pass);
        } else {
            block(&mut state, &mut v, pass);
        }
        assert_eq!(all, one.iter().map(|(s, _)| s.clone()).collect::<Vec<_>>());
        assert_eq!(state, one.last().unwrap().1);
        // NASM's macros are x86's
        assert_eq!(
            one.iter().any(|(_, s)| s.arg >> MACROS & 3 == 3),
            !arm,
            "past the macros it keeps"
        );
        assert!(one.iter().any(|(_, s)| s.arg & (GAS | SEMI) == GAS | SEMI));
    }
}

/// Each construct repeated on one long line lexes in linear time (the guide's mistake 17).
#[test]
fn long_lines_are_linear() {
    for unit in [
        "mov eax, [rbx+4] ",
        "x: ",
        "msg db 'a', ",
        "%1 %%a %define ",
        "#4 @x ",
        "\\a\\() ",
        "{k1}{z} ",
        "/* */ ",
        "; ",
        "v0.16b :lo12:a sctlr_el1 ",
        "%hi(x) $t0 $31 8(sp) ",
        "ld a, (hl) ex af, af' jp nz, ",
        "lda #$10 @l: :+ ",
        "+ - * ",
        "`0123 \\1 ",
        "move.l (a0)+,-(sp) x ",
        "\tdc.w (a b ",
        "%sp@- %d0 @17 1$ ",
        "<a b> | ",
    ] {
        let line = unit.repeat(20_000);
        for lang in EVERY {
            let t = std::time::Instant::now();
            let _ = of(lang, &[&line]);
            assert!(t.elapsed().as_secs() < 2, "{unit:?}: {:?}", t.elapsed());
        }
    }
}

/// An opener alone at a line's end: the loop moves on (the guide's mistake 33).
#[test]
fn a_lone_opener_at_a_lines_end() {
    for l in [
        "\\",
        "'",
        "\"",
        "`",
        "/*",
        "%",
        "%%",
        "#",
        "@",
        "$",
        ".",
        "..",
        "{",
        ":",
        "%{",
        "x.",
        "\t.macro",
        "#define",
        "%define",
        "%macro",
        "#include <",
        "1:",
        "\tcmpb $'",
        "!",
        "#$",
        "#%",
        "$f",
        "%h",
        "af'",
        ":+",
        "*",
        "+",
        "\tjp ",
        "\tret c",
        "x :",
        "x :=",
    ] {
        for lang in EVERY {
            let _ = of(lang, &[l, l]);
        }
    }
}

const MOD_: &str = "storage.modifier.asm";
const HEX_: &str = "constant.numeric.hex.asm";
const BIN_: &str = "constant.numeric.binary.asm";
const DIR_: &str = "support.function.directive.asm";

/// X-181: RISC-V's registers (ABI names, `x5`, CSRs, a vector's mask), relocations, `#`
/// comments, a GAS file's `;` and numeric labels from the start, vector types and rounding modes.
#[test]
fn risc_v() {
    let l = &[
        "\t.option norelax",
        "f:\taddi\tsp, sp, -16 # frame",
        "\tsd\tra, 8(sp); mv a0, s1",
        "1:\tlui\ta5, %hi(x)",
        "\tcsrr\tt0, mstatus",
        "\tvsetvli\tt0, a0, e32, m1, ta, ma",
        "\tvadd.vv\tv1, v2, v3, v0.t",
        "\tfcvt.w.s\ta0, fa0, rtz",
        "\tbnez\tx5, 1b",
    ];
    has(RV_, l, "sp", REG);
    has(RV_, l, "ra", REG);
    has(RV_, l, "mstatus", REG);
    has(RV_, l, "x5", REG);
    has(RV_, l, "v0.t", REG);
    has(RV_, l, "fa0", REG);
    has(RV_, l, "%hi", MOD_);
    has(RV_, l, "e32", MOD_);
    has(RV_, l, "rtz", MOD_);
    has(RV_, l, "norelax", MOD_);
    has(RV_, l, " frame", LINE);
    has(RV_, l, "mv", MNEM);
    has(RV_, l, "1b", NAME_);
    has(RV_, l, "1", NAME_);
}

/// X-181: MIPS's `$` registers (named, numbered, float), regdef.h's bare names, `%hi`/`%lo`,
/// `.set noreorder`, MARS's macros and their `%` parameters.
#[test]
fn mips() {
    let l = &[
        "\t.set\tnoreorder",
        "\tlui\t$t0, %hi(msg)",
        "\taddiu\t$sp, $sp, -8 # c",
        "\tmove\tv1, a3",
        "\tjr\t$31",
        "\tmtc1\t$t1, $f2",
        ".macro push (%reg)",
        "\tsw %reg, ($sp)",
        ".end_macro",
    ];
    has(MIPS_, l, "$t0", REG);
    has(MIPS_, l, "$sp", REG);
    has(MIPS_, l, "$31", REG);
    has(MIPS_, l, "$f2", REG);
    has(MIPS_, l, "v1", REG);
    has(MIPS_, l, "%hi", MOD_);
    has(MIPS_, l, "noreorder", MOD_);
    has(MIPS_, l, " c", LINE);
    has(MIPS_, l, "push", "entity.name.function.preprocessor.asm");
    has(MIPS_, l, "reg", "variable.other.preprocessor.asm");
}

/// X-181: Z80's and SM83's registers, the shadow `af'`, a branch's conditions, `$FF` and `%0101`,
/// rgbds' labels (`::`, `.local` in the first column), `\1`, graphics' numbers; labels without
/// `:` in the first column but its mnemonics; `;` a comment.
#[test]
fn z80() {
    let l = &[
        "SECTION \"Main\", ROM0",
        "Func::",
        "\tld a, [hl+] ; c",
        "\tex af, af'",
        "\tjp c, .done",
        "\tjr nz, Func",
        "\tret z",
        "\tld c, $FF",
        "\tld b, %0101",
        ".done",
        "loop\tdjnz loop",
        "x\tequ 5",
        "MACRO m",
        "\tld a, \\1",
        "\tENDM",
        "\tdw `01233210",
        "\tld a, #$10",
    ];
    has(Z80_, l, "hl", REG);
    has(Z80_, l, "af'", REG);
    has(Z80_, l, "c", MOD_);
    has(Z80_, l, "nz", MOD_);
    has(Z80_, l, "z", MOD_);
    has(Z80_, l, "c", REG);
    has(Z80_, l, "$FF", HEX_);
    has(Z80_, l, "%0101", BIN_);
    has(Z80_, l, "#$10", HEX_);
    has(Z80_, l, "`01233210", BIN_);
    has(Z80_, l, "Func", NAME_);
    has(Z80_, l, ".done", NAME_);
    has(Z80_, l, "loop", NAME_);
    has(Z80_, l, "djnz", MNEM);
    has(Z80_, l, "x", NAME_);
    has(Z80_, l, "m", "entity.name.function.preprocessor.asm");
    has(Z80_, l, "1", "variable.other.preprocessor.asm");
    has(Z80_, l, "ROM0", MOD_);
    has(Z80_, l, " c", LINE);
}

/// X-181: 6502's immediates (`#$10`, `#%01`), `$` hex and `%` binary, `@` and anonymous labels
/// (ca65's `:+`, ACME's `+`, `-`), `*` the location counter, DASM's labels in the first column and
/// its `'A`, ACME's `!byte`, ca65's `:=`.
#[test]
fn m6502() {
    let l = &[
        "\t.segment \"CODE\"",
        "start:\tlda #$10",
        "\tsta $2000,x",
        "\tldx #%0101",
        "@loop:\tdey",
        "\tbne @loop",
        ":\tinx",
        "\tbne :-",
        "-\tdex",
        "\tbne -",
        "\tbcc +",
        "+",
        "\t*= $1000",
        "\tjmp *",
        "Start\tsei",
        "\tlda #'A",
        "\t!byte 1, 2",
        "lda\tlda #0",
        "x := other",
    ];
    has(M65_, l, "#$10", HEX_);
    has(M65_, l, "$2000", HEX_);
    has(M65_, l, "#%0101", BIN_);
    has(M65_, l, "x", REG);
    has(M65_, l, "@loop", NAME_);
    has(M65_, l, ":-", NAME_);
    has(M65_, l, "-", NAME_);
    has(M65_, l, "+", NAME_);
    has(M65_, l, "*", "variable.language.asm");
    has(M65_, l, "Start", NAME_);
    has(M65_, l, "sei", MNEM);
    has(M65_, l, "!byte", DIR_);
    has(M65_, l, "other", NAME_);
    hasnt(M65_, l, "other", MNEM);
    // a mnemonic in the first column stays one (ca65's `lda #0` there)
    has(M65_, &["lda\t#0"], "lda", MNEM);
}

/// X-181: a block lexes as its lines on the four new scans, a comment's and a cpp line's state
/// across them, GAS's `;` from the start on RISC-V's and MIPS's.
#[test]
fn a_block_lexes_as_its_lines_on_the_others() {
    let text =
        "/* a\n b */ x:\n#define F(a) \\\n  addi a, a, 1\n; c\n\tli a0, 1; li a1, 2\n\tld a, $FF\n";
    for (lang, block) in [
        (RV_, block_riscv as Block),
        (MIPS_, block_mips),
        (Z80_, block_z80),
        (M65_, block_6502),
        (M68K_, block_68k),
    ] {
        let one: Vec<_> = {
            let mut state = State::default();
            text.lines()
                .map(|l| {
                    let mut spans = Vec::new();
                    crate::lex(lang, l, &mut state, &mut spans);
                    (spans, state)
                })
                .collect()
        };
        let mut state = State::default();
        let mut all = Vec::new();
        let mut v = Vec::new();
        block(&mut state, &mut v, &mut |c: &mut dyn crate::Carry| {
            for l in text.lines() {
                all.push(c.next(l.as_bytes()).to_vec());
            }
            None
        });
        assert_eq!(
            all,
            one.iter().map(|(s, _)| s.clone()).collect::<Vec<_>>(),
            "{lang:?}"
        );
        assert_eq!(state, one.last().unwrap().1, "{lang:?}");
    }
}

type Block = fn(
    &mut State,
    &mut Vec<crate::Span>,
    &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize>;

/// X-181: on Z80 and 6502, a mnemonic's word in the first column before a directive is a label
/// (ZCPR's `JR MACRO ?N`, `PER .res 1`); before an operand, the mnemonic.
#[test]
fn a_mnemonics_word_as_a_label() {
    has(Z80_, &["JR\tMACRO\t?N"], "JR", NAME_);
    has(M65_, &["PER\t.res 1"], "PER", NAME_);
    has(M65_, &["PER\t.res 1"], ".res", DIR_);
    has(M65_, &["lda\t#0"], "lda", MNEM);
    has(Z80_, &["jp\t_lock"], "jp", MNEM);
}

/// X-182: 8080's lines on Z80's scan (sjasmplus' 8080 mode, CP/M's sources): `m` and `psw`
/// registers, its mnemonics in the first column; `jp m` still a condition.
#[test]
fn i8080_on_z80() {
    let l = &[
        "\tMOV\tA,M",
        "\tPUSH\tPSW",
        "\tLXI\tH,BUF",
        "MVI\tA,0",
        "\tjp m, x",
    ];
    has(Z80_, l, "M", REG);
    has(Z80_, l, "PSW", REG);
    has(Z80_, l, "H", REG);
    has(Z80_, l, "MVI", MNEM);
    has(Z80_, l, "m", MOD_);
}

/// X-182: ca65's scoped names (`MGTK::Rect`, `::Init`) one symbol, its `:=` past the first
/// column; Kick Assembler's multi-labels (`!:`, `!loop:`, `bne !-`, `jmp !loop+`), a macro's
/// call in the first column, `.for`'s `;`; 64tass' bank byte.
#[test]
fn m6502_dialects() {
    let l = &[
        "\t.tag MGTK::Rect",
        "\tjsr ::Init",
        "\tlda OPTK::Record::left",
        "\tleft  := opr_copy + 1",
        "BasicUpstart2(Entry)",
        "\t!:",
        "\tbne !-",
        "!loop:\tdex",
        "\tjmp !loop+",
        "\t.for (var i = 0; i < 8; i++) {",
        "\tlda #`label ; bank",
        "\t.byte \"(\" ; a note",
    ];
    has(M65_, l, "MGTK::Rect", NAME_);
    has(M65_, l, "::Init", NAME_);
    has(M65_, l, "OPTK::Record::left", NAME_);
    has(M65_, l, "left", NAME_);
    hasnt(M65_, l, "left", MNEM);
    has(M65_, l, "BasicUpstart2", MNEM);
    has(M65_, l, "Entry", NAME_);
    has(M65_, l, "!", NAME_);
    has(M65_, l, "!-", NAME_);
    has(M65_, l, "!loop", NAME_);
    has(M65_, l, "dex", MNEM);
    has(M65_, l, "!loop+", NAME_);
    has(M65_, l, ";", "operator.separator.asm");
    hasnt(M65_, l, " i < 8; i++) {", LINE);
    has(M65_, l, "`", "keyword.operator.bitwise.asm");
    has(M65_, l, " bank", LINE);
    has(M65_, l, " a note", LINE);
    // ACME's directive stays one
    has(M65_, &["\t!byte 1"], "!byte", DIR_);
    // X-184: Merlin 32's `*` comment line, `:` locals, `]` variables, strings, `asc`; DASM's `'A`
    // and 64tass' `*=` as they were
    let m = &[
        "* a comment",
        ":L1000   lda   #$01",
        "         bne   :L1000",
        "]PTR     equ   $02",
        "         sta   ]PTR",
        "         asc   'low ASCII str'",
        "\tlda #'A ; it's",
        "*\t= $0801",
    ];
    has(M65_, m, " a comment", LINE);
    has(M65_, m, ":L1000", NAME_);
    has(M65_, m, "]PTR", NAME_);
    has(M65_, m, "low ASCII str", "string.quoted.single.asm");
    has(M65_, m, "asc", "storage.type.asm");
    has(M65_, m, "A", "string.quoted.single.asm");
    has(M65_, m, " it's", LINE);
    has(M65_, m, "*", "variable.language.asm");
}

/// X-183: 68k in Motorola's syntax: sized mnemonics, `d0`-`d7`/`a0`-`a7`, an index's size,
/// `$FF`, `%0101`, `@17`, `#` immediates, labels in the first column (`.loop`, `@loop`, `1$`),
/// `*` and `;` comments, the comment field without a mark, Devpac's `<…>` arguments.
#[test]
fn m68k_motorola() {
    let l = &[
        "* a comment",
        "start\tmove.l\t#$FF,d0\t\tcopy it",
        "\tmove.w\t(a0,d1.w*4),-(sp)",
        "\tmovem.l\td0-d7/a0-a6,-(sp) ; save",
        ".loop\tdbra\td0,.loop",
        "@wait:\tbtst\t#%0101,$bfe001",
        "1$\tbne.s\t1$",
        "\trts\t\treturn",
        "\tlea\t$dff000.l,a5",
        "x\tequ\t@17",
        "\tEdT\t169,<Library not opened>",
        "\tdc.l\t*+4",
        "\tif Revision = 0",
        "\tmove.b\t#' ', d2",
        "\tsection code,code_c",
    ];
    has(M68K_, l, " a comment", LINE);
    has(M68K_, l, "start", NAME_);
    has(M68K_, l, "move.l", MNEM);
    has(M68K_, l, "#$FF", HEX_);
    has(M68K_, l, "d0", REG);
    has(M68K_, l, "copy it", LINE);
    has(M68K_, l, "d1.w", REG);
    has(M68K_, l, "sp", REG);
    has(M68K_, l, "a6", REG);
    has(M68K_, l, " save", LINE);
    has(M68K_, l, ".loop", NAME_);
    has(M68K_, l, "dbra", MNEM);
    has(M68K_, l, "@wait", NAME_);
    has(M68K_, l, "#%0101", BIN_);
    has(M68K_, l, "1$", NAME_);
    has(M68K_, l, "return", LINE);
    has(M68K_, l, ".l", "storage.type.asm");
    has(M68K_, &["\tmove.l\t4.w,a6"], ".w", "storage.type.asm");
    has(M68K_, l, "@17", "constant.numeric.octal.asm");
    has(M68K_, l, "Library not opened", "string.quoted.other.asm");
    has(M68K_, l, "*", "variable.language.asm");
    hasnt(M68K_, l, "0", LINE);
    has(M68K_, l, "d2", REG);
    has(
        M68K_,
        l,
        "code",
        "entity.name.section.asm string.unquoted.asm support.constant.section.asm",
    );
    has(M68K_, l, "code_c", MOD_);
    has(
        M68K_,
        &["\tLONG\tcli_Code\t* from the command"],
        " from the command",
        LINE,
    );
    // a dotted word in the first column before a mnemonic is a label, GAS's `.text` alone not
    has(M68K_, &[".Skip\tmove.l\td0,d1\tA tester"], ".Skip", NAME_);
    has(M68K_, &[".Skip\tmove.l\td0,d1\tA tester"], "A tester", LINE);
    has(M68K_, &[".text"], ".text", DIR_);
    has(M68K_, &["\tmove.w\td2,(a1)+\tLongueur"], "Longueur", LINE);
    // `|` the operator outside GAS's files
    has(M68K_, &["x = 0|y"], "|", "keyword.operator.bitwise.asm");
}

/// X-183: 68k under GAS: MIT's `%d0`, `%sp@-`, `|` comments, `#` an immediate but a comment
/// before a blank; no comment field without a mark.
#[test]
fn m68k_gas() {
    let l = &[
        "#include <linux/linkage.h>",
        "\t.text",
        "\tmovel\t%sp@(24),%a0\t| switch_stack pointer",
        "\tmovel\t%a1,%sp@-",
        "\tmove.l #0x10, %d0",
        "\tmov.l\t\tEXC_EXTWPTR(%a6),%a0\t# put base in a0",
        "| a comment",
    ];
    has(M68K_, l, "%sp", REG);
    has(M68K_, l, "%a0", REG);
    has(M68K_, l, " switch_stack pointer", LINE);
    has(M68K_, l, "@-", "keyword.operator.arithmetic.asm");
    has(M68K_, l, "%d0", REG);
    has(M68K_, l, " put base in a0", LINE);
    has(M68K_, l, " a comment", LINE);
    hasnt(M68K_, l, "%d0", LINE);
    // an index's size the register's, its scale's `:` the operator (X-186)
    let l = &["\tmovel\t%a0@(%d1:l:4),%a0", "\tmovel\t%a1@(%d0:w),%a1"];
    has(M68K_, l, "%d1:l", REG);
    has(M68K_, l, "%d0:w", REG);
    has(M68K_, l, ":", "keyword.operator.arithmetic.asm");
    has(M68K_, l, "4", "constant.numeric.decimal.asm");
}

/// X-186: a macro's size argument (`move.\0`) and labels with letters past ASCII (the Atari
/// ST's and the Amiga's sources).
#[test]
fn m68k_corners() {
    let l = &["\tTST.\\0\t\\1", "\tbra.\\0\t.newblock"];
    has(M68K_, l, "TST.", MNEM);
    has(M68K_, l, "bra.", MNEM);
    hasnt(M68K_, l, ".", "variable.language.asm");
    let l = &[
        "DONNÉES_AFFICH\tdc.w\t0",
        "\tLEA\tDONNÉES_AFFICH,A6",
        "\tMOVE\t#$4E75,MODIFIÉ\tÉcran précédent",
        "Ç:\trts",
    ];
    has(M68K_, l, "DONNÉES_AFFICH", NAME_);
    has(M68K_, l, "MODIFIÉ", NAME_);
    has(M68K_, l, "Écran précédent", LINE);
    has(M68K_, l, "Ç", NAME_);
    // a name's letters past ASCII on the others too, and U+FFFD (a letter of another charset
    // read as UTF-8); not a symbol's (cosmopolitan's `#← `)
    for lang in EVERY {
        has(lang, &["\tjmp\tété"], "été", NAME_);
        has(lang, &["\tjmp\tDONN\u{fffd}ES"], "DONN\u{fffd}ES", NAME_);
        hasnt(lang, &["\tjmp\tx × y"], "×", NAME_);
    }
}
