//! Python on a scanner of ada's own (X-162), from a floor probe (the same files built with
//! `ADA_PARSE_X162_SOL`), in the shape of Python's own tokenizer: one loop over a line's bytes, a
//! byte's action one load from a table ([`ACT`]), a name read once and looked up once in one
//! table of every word ([`WORDS`]: its class or keyword leaf and, for the words whose meaning hangs
//! on where they stand, an action), the previous significant token in a byte ([`Scan::prev`]),
//! and frames only for what nests ([`Scan::f`], a byte a level): brackets by what opened them (a
//! call's arguments, a def's parameters, a class's bases, an import list, type parameters, a
//! subscript, a display), and an f-string's replacement field, which holds the string to go on
//! with after its `}`. A statement's head (`def`, `class`, `import`, `for`, `with`, a compound
//! statement whose `:` ends it) is a byte carried while brackets are open or a `\` continues the
//! line ([`Scan::head`]). A string open at a line's end (a triple-quoted one, a `\`-continued
//! one) is the line's start: its kind ([`SK`]) in [`State::arg`].
//!
//! Python's reading where bat's grammar has none or derails: a string is a regex where Python's
//! `re` takes one (a regex call's pattern, raw or cooked; `match=`/`pattern=`) or where a raw
//! string's text is a valid pattern ([`text`]'s `re_valid`), never by its `r` alone; SQL by bat's
//! uppercase keyword with text after it, or by an execute-like call's first argument in any case;
//! a docstring is any string statement first on its line (any quote); PEP 695's type parameters and
//! `type` statement, `match`/`case`, PEP 701's f-strings (quotes, comments and lines in a field),
//! `t"…"` strings; `print` a builtin but at a statement's start before an operand.

#[cfg(test)]
mod tests;
mod text;

use crate::emit::{Rules, Run, Runs, Sink, Spans};
use crate::prim::{WORD, char_len, is_word_starting, word_end};
use crate::regex;
use crate::sql;
use crate::words::{Words, text_len};
use crate::{Out, State};

/// The floor probe (X-162's `sol`): every word its table leaf and a call its own, strings with
/// their escapes and an f-string's fields, numbers, comments, brackets; no head, parameter,
/// keyword argument, decorator, docstring, placeholder, SQL or regex read.
const SOL: bool = option_env!("ADA_PARSE_X162_SOL").is_some();

/// The code's leaves: a token's scopes, outermost first (137).
static LEAVES: [&str; 137] = [
    "",
    "comment.line.number-sign.python",
    "comment.line.number-sign.python punctuation.definition.comment.python",
    "keyword.control.import.python",
    "keyword.control.import.from.python",
    "keyword.control.import.relative.python",
    "keyword.control.import.as.python",
    "constant.language.import-all.python",
    "keyword.control.flow.return.python",
    "keyword.control.flow.break.python",
    "keyword.control.flow.continue.python",
    "keyword.control.flow.pass.python",
    "keyword.control.flow.raise.python",
    "keyword.control.flow.raise.from.python",
    "keyword.control.flow.assert.python",
    "keyword.control.flow.yield.python",
    "keyword.control.flow.yield-from.python",
    "keyword.control.flow.with.python",
    "keyword.control.flow.with.as.python",
    "keyword.control.loop.for.python",
    "keyword.control.loop.for.in.python",
    "keyword.control.loop.for.generator.python",
    "keyword.control.loop.while.python",
    "keyword.control.conditional.if.python",
    "keyword.control.conditional.elseif.python",
    "keyword.control.conditional.else.python",
    "keyword.control.exception.try.python",
    "keyword.control.exception.catch.python",
    "keyword.control.exception.catch.as.python",
    "keyword.control.exception.finally.python",
    "keyword.other.del.python",
    "keyword.other.print.python",
    "keyword.other.exec.python",
    "keyword.other.await.python",
    "storage.modifier.async.python",
    "storage.modifier.global.python",
    "storage.modifier.nonlocal.python",
    "storage.type.function.python keyword.declaration.function.python",
    "storage.type.class.python keyword.declaration.class.python",
    "storage.type.function.inline.python keyword.declaration.function.inline.python",
    "keyword.control.conditional.match.python",
    "keyword.control.conditional.case.python",
    "storage.type.type.python keyword.declaration.type.python",
    "keyword.operator.logical.python",
    "constant.language.python",
    "keyword.operator.assignment.python",
    "keyword.operator.assignment.augmented.python",
    "keyword.operator.assignment.inline.python",
    "keyword.operator.comparison.python",
    "keyword.operator.arithmetic.python",
    "keyword.operator.matrix.python",
    "keyword.operator.unpacking.sequence.python",
    "keyword.operator.unpacking.mapping.python",
    "invalid.deprecated.operator.python",
    "punctuation.accessor.dot.python",
    "punctuation.terminator.statement.python",
    "punctuation.definition.annotation.python",
    "punctuation.separator.continuation.line.python",
    "variable.parameter.python",
    "variable.parameter.class-inheritance.python",
    "variable.parameter.type.python",
    "storage.modifier.positional-args-only.python",
    "storage.type.string.python",
    "storage.modifier.conversion.python",
    "storage.modifier.debug.python",
    "constant.other.format-spec.python",
    "invalid.illegal.stray.brace.round.python",
    "invalid.illegal.stray.brace.square.python",
    "invalid.illegal.stray.brace.curly.python",
    "punctuation.separator.annotation.return.python",
    "punctuation.separator.annotation.parameter.python",
    "punctuation.separator.annotation.variable.python",
    "punctuation.separator.mapping.key-value.python",
    "punctuation.separator.slice.python",
    "constant.numeric.integer.decimal.python",
    "constant.numeric.integer.decimal.python storage.type.numeric.python",
    "constant.numeric.float.decimal.python",
    "constant.numeric.float.decimal.python punctuation.separator.decimal.python",
    "constant.numeric.integer.hexadecimal.python",
    "constant.numeric.integer.hexadecimal.python punctuation.definition.numeric.base.python",
    "constant.numeric.integer.hexadecimal.python storage.type.numeric.python",
    "constant.numeric.integer.octal.python",
    "constant.numeric.integer.octal.python punctuation.definition.numeric.base.python",
    "constant.numeric.integer.octal.python storage.type.numeric.python",
    "constant.numeric.integer.binary.python",
    "constant.numeric.integer.binary.python punctuation.definition.numeric.base.python",
    "constant.numeric.integer.binary.python storage.type.numeric.python",
    "constant.numeric.imaginary.decimal.python",
    "constant.numeric.imaginary.decimal.python punctuation.separator.decimal.python",
    "constant.numeric.imaginary.decimal.python storage.type.numeric.python",
    "punctuation.section.arguments.begin.python",
    "punctuation.section.arguments.end.python",
    "punctuation.section.parameters.begin.python",
    "punctuation.section.parameters.end.python",
    "punctuation.section.inheritance.begin.python",
    "punctuation.section.inheritance.end.python",
    "punctuation.section.group.begin.python",
    "punctuation.section.group.end.python",
    "punctuation.section.sequence.begin.python",
    "punctuation.section.sequence.end.python",
    "punctuation.section.brackets.begin.python",
    "punctuation.section.brackets.end.python",
    "punctuation.section.mapping-or-set.begin.python",
    "punctuation.section.mapping-or-set.end.python",
    "punctuation.section.mapping.end.python",
    "punctuation.section.import-list.begin.python",
    "punctuation.section.import-list.end.python",
    "punctuation.section.interpolation.begin.python",
    "punctuation.section.interpolation.end.python",
    "punctuation.section.type-parameters.begin.python",
    "punctuation.section.type-parameters.end.python",
    "punctuation.section.function.begin.python",
    "punctuation.section.class.begin.python",
    "punctuation.section.block.conditional.if.python",
    "punctuation.section.block.conditional.elseif.python",
    "punctuation.section.block.conditional.else.python",
    "punctuation.section.block.loop.for.python",
    "punctuation.section.block.loop.while.python",
    "punctuation.section.block.with.python",
    "punctuation.section.block.exception.try.python",
    "punctuation.section.block.exception.catch.python",
    "punctuation.section.block.exception.finally.python",
    "punctuation.section.block.conditional.match.python",
    "punctuation.section.block.conditional.case.python",
    "punctuation.separator.arguments.python",
    "punctuation.separator.parameters.python",
    "punctuation.separator.inheritance.python",
    "punctuation.separator.sequence.python",
    "punctuation.separator.tuple.python",
    "punctuation.separator.mapping.python",
    "punctuation.separator.set.python",
    "punctuation.separator.import-list.python",
    "punctuation.separator.target-list.python",
    "punctuation.separator.with-resources.python",
    "punctuation.separator.storage-list.python",
    "punctuation.separator.type-parameters.python",
    "entity.name.type.python",
];
const PLAIN: u16 = 0;
const COMMENT: u16 = 1;
const COMMENT_MARK: u16 = 2;
const K_IMPORT: u16 = 3;
const K_FROM: u16 = 4;
const K_RELATIVE: u16 = 5;
const K_IMPORT_AS: u16 = 6;
const IMPORT_ALL: u16 = 7;
const K_RETURN: u16 = 8;
const K_BREAK: u16 = 9;
const K_CONTINUE: u16 = 10;
const K_PASS: u16 = 11;
const K_RAISE: u16 = 12;
const K_RAISE_FROM: u16 = 13;
const K_ASSERT: u16 = 14;
const K_YIELD: u16 = 15;
const K_YIELD_FROM: u16 = 16;
const K_WITH: u16 = 17;
const K_WITH_AS: u16 = 18;
const K_FOR: u16 = 19;
const K_FOR_IN: u16 = 20;
const K_GENERATOR: u16 = 21;
const K_WHILE: u16 = 22;
const K_IF: u16 = 23;
const K_ELIF: u16 = 24;
const K_ELSE: u16 = 25;
const K_TRY: u16 = 26;
const K_EXCEPT: u16 = 27;
const K_EXCEPT_AS: u16 = 28;
const K_FINALLY: u16 = 29;
const K_DEL: u16 = 30;
const K_PRINT: u16 = 31;
const K_EXEC: u16 = 32;
const K_AWAIT: u16 = 33;
const K_ASYNC: u16 = 34;
const K_GLOBAL: u16 = 35;
const K_NONLOCAL: u16 = 36;
const K_DEF: u16 = 37;
const K_CLASS: u16 = 38;
const K_LAMBDA: u16 = 39;
const K_MATCH: u16 = 40;
const K_CASE: u16 = 41;
const K_TYPE: u16 = 42;
const LOGICAL: u16 = 43;
const CONSTANT: u16 = 44;
const ASSIGN: u16 = 45;
const AUGMENTED: u16 = 46;
const INLINE: u16 = 47;
const COMPARISON: u16 = 48;
const ARITH: u16 = 49;
const MATRIX: u16 = 50;
const UNPACK_SEQ: u16 = 51;
const UNPACK_MAP: u16 = 52;
const DEPRECATED_OP: u16 = 53;
const DOT: u16 = 54;
const TERMINATOR: u16 = 55;
const DECO_MARK: u16 = 56;
const CONTINUATION: u16 = 57;
const PARAMETER: u16 = 58;
const CLASS_KWARG: u16 = 59;
const TYPE_PARAM: u16 = 60;
const POSITIONAL: u16 = 61;
const STRING_PREFIX: u16 = 62;
const CONVERSION: u16 = 63;
const DEBUG: u16 = 64;
const FORMAT_SPEC: u16 = 65;
const STRAY_ROUND: u16 = 66;
const STRAY_SQUARE: u16 = 67;
const STRAY_CURLY: u16 = 68;
const RETURN_ANN: u16 = 69;
const PARAM_ANN: u16 = 70;
const VAR_ANN: u16 = 71;
const KEY_VALUE: u16 = 72;
const SLICE: u16 = 73;
const INT: u16 = 74;
const INT_SUFFIX: u16 = 75;
const FLOAT: u16 = 76;
const FLOAT_DOT: u16 = 77;
const HEX: u16 = 78;
const HEX_BASE: u16 = 79;
const HEX_SUFFIX: u16 = 80;
const OCT: u16 = 81;
const OCT_BASE: u16 = 82;
const OCT_SUFFIX: u16 = 83;
const BIN: u16 = 84;
const BIN_BASE: u16 = 85;
const BIN_SUFFIX: u16 = 86;
const IMAG: u16 = 87;
const IMAG_DOT: u16 = 88;
const IMAG_SUFFIX: u16 = 89;
const ARGS_B: u16 = 90;
const ARGS_E: u16 = 91;
const PARAMS_B: u16 = 92;
const PARAMS_E: u16 = 93;
const INHERIT_B: u16 = 94;
const INHERIT_E: u16 = 95;
const GROUP_B: u16 = 96;
const GROUP_E: u16 = 97;
const SEQ_B: u16 = 98;
const SEQ_E: u16 = 99;
const BRACKETS_B: u16 = 100;
const BRACKETS_E: u16 = 101;
const MAPSET_B: u16 = 102;
const MAPSET_E: u16 = 103;
const MAPPING_E: u16 = 104;
const IMPORT_B: u16 = 105;
const IMPORT_E: u16 = 106;
const INTERP_B: u16 = 107;
const INTERP_E: u16 = 108;
const TPARAMS_B: u16 = 109;
const TPARAMS_E: u16 = 110;
const FUNC_BEGIN: u16 = 111;
const CLASS_BEGIN: u16 = 112;
const BLOCK_IF: u16 = 113;
const BLOCK_ELIF: u16 = 114;
const BLOCK_ELSE: u16 = 115;
const BLOCK_FOR: u16 = 116;
const BLOCK_WHILE: u16 = 117;
const BLOCK_WITH: u16 = 118;
const BLOCK_TRY: u16 = 119;
const BLOCK_EXCEPT: u16 = 120;
const BLOCK_FINALLY: u16 = 121;
const BLOCK_MATCH: u16 = 122;
const BLOCK_CASE: u16 = 123;
const SEP_ARGS: u16 = 124;
const SEP_PARAMS: u16 = 125;
const SEP_INHERIT: u16 = 126;
const SEP_SEQ: u16 = 127;
const SEP_TUPLE: u16 = 128;
const SEP_MAPPING: u16 = 129;
const SEP_SET: u16 = 130;
const SEP_IMPORT: u16 = 131;
const SEP_TARGETS: u16 = 132;
const SEP_WITH: u16 = 133;
const SEP_STORAGE: u16 = 134;
const SEP_TPARAMS: u16 = 135;
const TYPE_NAME: u16 = 136;

/// A name's leaves: by its role ([`R_BARE`]…, eight a role) and its class ([`NC_GENERIC`]…).
const NAMES: u16 = 160;
const _: () = assert!(LEAVES.len() <= NAMES as usize);
static NAME_LEAVES: [[&str; 2]; 14] = [
    ["", ""],
    ["variable.function.python", ""],
    ["variable.annotation.python", ""],
    ["variable.annotation.function.python", ""],
    ["entity.name.function.python", ""],
    ["entity.name.class.python", ""],
    ["", ""],
    ["", ""],
    ["", ""],
    ["", "support.function.builtin.python"],
    ["", "support.type.python"],
    ["", "support.type.exception.python"],
    ["", "support.function.magic.python"],
    ["", "support.variable.magic.python"],
];
static CLASS_LEAVES: [&str; 8] = [
    "",
    "support.function.builtin.python",
    "support.type.python",
    "support.type.exception.python",
    "support.function.magic.python",
    "support.variable.magic.python",
    "variable.language.python",
    "variable.other.constant.python",
];
const R_BARE: u16 = 0;
const R_CALL: u16 = 1;
const R_DECO: u16 = 2;
const R_DECO_CALL: u16 = 3;
const R_DEF: u16 = 4;
const R_CLASS: u16 = 5;
// A name's class, from the word table or its spelling (a constant's).
const NC_GENERIC: u16 = 0;
const NC_BUILTIN: u16 = 1;
const NC_TYPE: u16 = 2;
const NC_EXCEPTION: u16 = 3;
const NC_MAGIC_FN: u16 = 4;
const NC_MAGIC_VAR: u16 = 5;
const NC_LANGUAGE: u16 = 6;
const NC_CONSTANT: u16 = 7;

/// A string's leaves: [`STR`] and 32 a text scope ([`T_SINGLE`]…), each one of these.
const STR: u16 = 256;
const S_TEXT: u16 = 0;
const S_BEGIN: u16 = 1;
const S_END: u16 = 2;
const S_ESC: u16 = 3;
const S_ESC_HEX: u16 = 4;
const S_ESC_OCT: u16 = 5;
const S_ESC_U16: u16 = 6;
const S_ESC_U32: u16 = 7;
const S_ESC_NAME: u16 = 8;
const S_ESC_BAD: u16 = 9;
const S_CONT: u16 = 10;
const S_PH: u16 = 11;
const S_PH_VAR: u16 = 12;
const S_PH_B: u16 = 13;
const S_PH_E: u16 = 14;
const S_PH_CONV: u16 = 15;
const S_PH_SPEC: u16 = 16;
const S_RE_COMMENT: u16 = 17;
const S_RE_COMMENT_MARK: u16 = 18;
static STRING_LEAVES: [&str; 19] = [
    "",
    "punctuation.definition.string.begin.python",
    "punctuation.definition.string.end.python",
    "constant.character.escape.python",
    "constant.character.escape.hex.python",
    "constant.character.escape.octal.python",
    "constant.character.escape.unicode.16-bit-hex.python",
    "constant.character.escape.unicode.32-bit-hex.python",
    "constant.character.escape.unicode.name.python",
    "invalid.deprecated.character.escape.python",
    "punctuation.separator.continuation.line.python",
    "constant.other.placeholder.python",
    "constant.other.placeholder.python variable.other.placeholder.python",
    "constant.other.placeholder.python punctuation.definition.placeholder.begin.python",
    "constant.other.placeholder.python punctuation.definition.placeholder.end.python",
    "constant.other.placeholder.python storage.modifier.conversion.python",
    "constant.other.placeholder.python constant.other.format-spec.python",
    "comment.line.number-sign.regexp",
    "comment.line.number-sign.regexp punctuation.definition.comment.regexp",
];
// A string's text scope: its quotes', or a docstring's.
const T_DOC: u16 = 4;
static TEXT_SCOPES: [&str; 5] = [
    "string.quoted.single.python",
    "string.quoted.double.python",
    "string.quoted.single.block.python",
    "string.quoted.double.block.python",
    "comment.block.documentation.python",
];

/// A regex in a string of text scope `t`: [`RE`] + `t` * 128 + the regex's kind.
const RE: u16 = 512;
/// SQL in a string of text scope `t`: [`SQL`] + `t` * 128 + SQL's kind.
const SQL: u16 = 1024;
const _: () = assert!(regex::KINDS <= 128 && sql::KINDS <= 128);
/// A kind's bit inside a decorator: `meta.annotation`, which a theme colors the plain text under.
const META: u16 = 2048;

/// The scopes under `source.python` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    let mut out = Vec::new();
    if kind & META != 0 {
        out.push("meta.annotation.python");
    }
    let kind = kind & !META;
    let split = |out: &mut Vec<&'static str>, s: &'static str| {
        out.extend(s.split(' ').filter(|s| !s.is_empty()));
    };
    if kind >= SQL {
        let t = usize::from((kind - SQL) / 128);
        if t < 4 && usize::from((kind - SQL) % 128) < sql::KINDS {
            split(&mut out, TEXT_SCOPES[t]);
            out.extend(sql::scopes((kind - SQL) % 128));
        }
    } else if kind >= RE {
        let t = usize::from((kind - RE) / 128);
        if t < 4 && usize::from((kind - RE) % 128) < regex::KINDS {
            split(&mut out, TEXT_SCOPES[t]);
            // the regex scanner's leaves (bat's JavaScript regex grammar's) as bat's Python regex
            // grammar scopes them, where their colors part
            for sc in regex::scopes((kind - RE) % 128) {
                out.push(match sc {
                    "punctuation.definition.group.assertion.regexp" => {
                        "constant.other.assertion.regexp"
                    }
                    "constant.other.character-class.escape.backslash.regexp" => {
                        "constant.character.character-class.regexp"
                    }
                    "punctuation.definition.group.named.begin.regexp"
                    | "punctuation.definition.group.named.end.regexp" => {
                        "punctuation.definition.group.capture.regexp"
                    }
                    "variable.other.group.regexp" => "keyword.other.back-reference.named.regexp",
                    "constant.literal.numeric.regexp"
                    | "punctuation.separator.comma.regexp"
                    | "punctuation.definition.range.regexp" => continue,
                    s => s,
                });
            }
        }
    } else if kind >= STR {
        let (t, l) = (
            usize::from((kind - STR) / 32),
            usize::from((kind - STR) % 32),
        );
        if t < 5 && l < STRING_LEAVES.len() {
            split(&mut out, TEXT_SCOPES[t]);
            let leaf = match (t, l) {
                (4, 1) => "punctuation.definition.comment.begin.python",
                (4, 2) => "punctuation.definition.comment.end.python",
                _ => STRING_LEAVES[l],
            };
            split(&mut out, leaf);
        }
    } else if kind >= NAMES {
        let (role, class) = (
            usize::from((kind - NAMES) / 8),
            usize::from((kind - NAMES) % 8),
        );
        if role < 6 {
            split(&mut out, NAME_LEAVES[role][0]);
            split(&mut out, CLASS_LEAVES[class]);
        }
    } else if let Some(l) = LEAVES.get(usize::from(kind)) {
        split(&mut out, l);
    }
    out
}

/// The kind of leaf `l` of a string of text scope `t`.
#[inline(always)]
const fn sk_leaf(t: u16, l: u16) -> u16 {
    STR + t * 32 + l
}

// A byte's action ([`ACT`]).
const A_OTHER: u8 = 0;
const A_SPACE: u8 = 1;
const A_NAME: u8 = 2;
const A_DIGIT: u8 = 3;
const A_QUOTE: u8 = 4;
const A_HASH: u8 = 5;
const A_DOT: u8 = 6;
/// `=+-*/%&|^~<>!`: an operator.
const A_OP: u8 = 7;
const A_LPAREN: u8 = 8;
const A_RPAREN: u8 = 9;
const A_LBRACKET: u8 = 10;
const A_RBRACKET: u8 = 11;
const A_LBRACE: u8 = 12;
const A_RBRACE: u8 = 13;
const A_COMMA: u8 = 14;
const A_COLON: u8 = 15;
const A_SEMI: u8 = 16;
const A_AT: u8 = 17;
const A_BACKSLASH: u8 = 18;
/// A byte past ASCII: a name's character, or another.
const A_WIDE: u8 = 19;

static ACT: [u8; 256] = {
    let mut t = [A_WIDE; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = match b {
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => A_SPACE,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => A_NAME,
            b'0'..=b'9' => A_DIGIT,
            b'"' | b'\'' => A_QUOTE,
            b'#' => A_HASH,
            b'.' => A_DOT,
            b'=' | b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^' | b'~' | b'<' | b'>'
            | b'!' => A_OP,
            b'(' => A_LPAREN,
            b')' => A_RPAREN,
            b'[' => A_LBRACKET,
            b']' => A_RBRACKET,
            b'{' => A_LBRACE,
            b'}' => A_RBRACE,
            b',' => A_COMMA,
            b':' => A_COLON,
            b';' => A_SEMI,
            b'@' => A_AT,
            b'\\' => A_BACKSLASH,
            _ => A_OTHER,
        };
        c += 1;
    }
    t
};

// What a word is ([`WORDS`], [`INFO`]): its class (a name's) or its leaf (a keyword's) in the
// low byte, an action in the high one. The names' actions come first, the keywords' from
// [`W_KW`].
const W_ACTION: u16 = 0xff << 8;
/// A regex function's name: `re.`'s or `regex.`'s call takes a pattern first.
const W_RECALL: u16 = 1 << 8;
/// A call whose first argument is SQL where it starts with a statement's keyword.
const W_SQLCALL: u16 = 2 << 8;
/// `type`: a statement at a line's start before a name and `=` or `[`.
const W_TYPE: u16 = 3 << 8;
/// `print`, `exec`: Python 2's statements at a line's start before an operand.
const W_PRINT: u16 = 4 << 8;
const W_EXEC: u16 = 5 << 8;
/// `match`, `case`: soft keywords at a line's start where the line is their statement's head.
const W_MATCH: u16 = 6 << 8;
const W_CASE: u16 = 7 << 8;
/// A keyword whose leaf is in the low byte, an operand after it.
const W_KW: u16 = 8 << 8;
const W_IMPORT: u16 = 9 << 8;
const W_FROM: u16 = 10 << 8;
const W_AS: u16 = 11 << 8;
const W_IN: u16 = 12 << 8;
const W_DEF: u16 = 13 << 8;
const W_CLASS: u16 = 14 << 8;
const W_LAMBDA: u16 = 15 << 8;
const W_IF: u16 = 16 << 8;
const W_ELIF: u16 = 17 << 8;
const W_ELSE: u16 = 18 << 8;
const W_FOR: u16 = 19 << 8;
const W_WHILE: u16 = 20 << 8;
const W_WITH: u16 = 21 << 8;
const W_TRY: u16 = 22 << 8;
const W_EXCEPT: u16 = 23 << 8;
const W_FINALLY: u16 = 24 << 8;
const W_ASYNC: u16 = 25 << 8;
const W_YIELD: u16 = 26 << 8;
const W_RAISE: u16 = 27 << 8;
const W_GLOBAL: u16 = 28 << 8;
const W_NONLOCAL: u16 = 29 << 8;
/// `and`, `or`, `not`, `is`.
const W_LOGIC: u16 = 30 << 8;
/// `True`, `False`, `None`, `Ellipsis`, `NotImplemented`, `__debug__`.
const W_CONST: u16 = 31 << 8;

/// Every word with a class or an action, its info; a sorted list.
const WORD_LIST: &[(&[u8], u16)] = &[
    (b"ArithmeticError", NC_EXCEPTION),
    (b"AssertionError", NC_EXCEPTION),
    (b"AttributeError", NC_EXCEPTION),
    (b"BaseException", NC_EXCEPTION),
    (b"BaseExceptionGroup", NC_EXCEPTION),
    (b"BlockingIOError", NC_EXCEPTION),
    (b"BrokenPipeError", NC_EXCEPTION),
    (b"BufferError", NC_EXCEPTION),
    (b"BytesWarning", NC_EXCEPTION),
    (b"ChildProcessError", NC_EXCEPTION),
    (b"ConnectionAbortedError", NC_EXCEPTION),
    (b"ConnectionError", NC_EXCEPTION),
    (b"ConnectionRefusedError", NC_EXCEPTION),
    (b"ConnectionResetError", NC_EXCEPTION),
    (b"DeprecationWarning", NC_EXCEPTION),
    (b"EOFError", NC_EXCEPTION),
    (b"Ellipsis", W_CONST),
    (b"EncodingWarning", NC_EXCEPTION),
    (b"EnvironmentError", NC_EXCEPTION),
    (b"Exception", NC_EXCEPTION),
    (b"ExceptionGroup", NC_EXCEPTION),
    (b"False", W_CONST),
    (b"FileExistsError", NC_EXCEPTION),
    (b"FileNotFoundError", NC_EXCEPTION),
    (b"FloatingPointError", NC_EXCEPTION),
    (b"FutureWarning", NC_EXCEPTION),
    (b"GeneratorExit", NC_EXCEPTION),
    (b"IOError", NC_EXCEPTION),
    (b"ImportError", NC_EXCEPTION),
    (b"ImportWarning", NC_EXCEPTION),
    (b"IndentationError", NC_EXCEPTION),
    (b"IndexError", NC_EXCEPTION),
    (b"InterruptedError", NC_EXCEPTION),
    (b"IsADirectoryError", NC_EXCEPTION),
    (b"KeyError", NC_EXCEPTION),
    (b"KeyboardInterrupt", NC_EXCEPTION),
    (b"LookupError", NC_EXCEPTION),
    (b"MemoryError", NC_EXCEPTION),
    (b"ModuleNotFoundError", NC_EXCEPTION),
    (b"NameError", NC_EXCEPTION),
    (b"None", W_CONST),
    (b"NotADirectoryError", NC_EXCEPTION),
    (b"NotImplemented", W_CONST),
    (b"NotImplementedError", NC_EXCEPTION),
    (b"OSError", NC_EXCEPTION),
    (b"OverflowError", NC_EXCEPTION),
    (b"PendingDeprecationWarning", NC_EXCEPTION),
    (b"PermissionError", NC_EXCEPTION),
    (b"ProcessLookupError", NC_EXCEPTION),
    (b"PythonFinalizationError", NC_EXCEPTION),
    (b"RecursionError", NC_EXCEPTION),
    (b"ReferenceError", NC_EXCEPTION),
    (b"ResourceWarning", NC_EXCEPTION),
    (b"RuntimeError", NC_EXCEPTION),
    (b"RuntimeWarning", NC_EXCEPTION),
    (b"SQL", W_SQLCALL),
    (b"StandardError", NC_EXCEPTION),
    (b"StopAsyncIteration", NC_EXCEPTION),
    (b"StopIteration", NC_EXCEPTION),
    (b"SyntaxError", NC_EXCEPTION),
    (b"SyntaxWarning", NC_EXCEPTION),
    (b"SystemError", NC_EXCEPTION),
    (b"SystemExit", NC_EXCEPTION),
    (b"TabError", NC_EXCEPTION),
    (b"TimeoutError", NC_EXCEPTION),
    (b"True", W_CONST),
    (b"TypeError", NC_EXCEPTION),
    (b"UnboundLocalError", NC_EXCEPTION),
    (b"UnicodeDecodeError", NC_EXCEPTION),
    (b"UnicodeEncodeError", NC_EXCEPTION),
    (b"UnicodeError", NC_EXCEPTION),
    (b"UnicodeTranslateError", NC_EXCEPTION),
    (b"UnicodeWarning", NC_EXCEPTION),
    (b"UserWarning", NC_EXCEPTION),
    (b"VMSError", NC_EXCEPTION),
    (b"ValueError", NC_EXCEPTION),
    (b"Warning", NC_EXCEPTION),
    (b"WindowsError", NC_EXCEPTION),
    (b"ZeroDivisionError", NC_EXCEPTION),
    (b"_", NC_LANGUAGE),
    (b"__abs__", NC_MAGIC_FN),
    (b"__add__", NC_MAGIC_FN),
    (b"__aenter__", NC_MAGIC_FN),
    (b"__aexit__", NC_MAGIC_FN),
    (b"__aiter__", NC_MAGIC_FN),
    (b"__all__", NC_MAGIC_VAR),
    (b"__and__", NC_MAGIC_FN),
    (b"__anext__", NC_MAGIC_FN),
    (b"__annotations__", NC_MAGIC_VAR),
    (b"__await__", NC_MAGIC_FN),
    (b"__bases__", NC_MAGIC_VAR),
    (b"__bool__", NC_MAGIC_FN),
    (b"__buffer__", NC_MAGIC_FN),
    (b"__builtins__", NC_MAGIC_VAR),
    (b"__bytes__", NC_MAGIC_FN),
    (b"__call__", NC_MAGIC_FN),
    (b"__ceil__", NC_MAGIC_FN),
    (b"__class__", NC_MAGIC_VAR),
    (b"__class_getitem__", NC_MAGIC_FN),
    (b"__closure__", NC_MAGIC_VAR),
    (b"__cmp__", NC_MAGIC_FN),
    (b"__code__", NC_MAGIC_VAR),
    (b"__coerce__", NC_MAGIC_FN),
    (b"__complex__", NC_MAGIC_FN),
    (b"__contains__", NC_MAGIC_FN),
    (b"__debug__", W_CONST),
    (b"__defaults__", NC_MAGIC_VAR),
    (b"__del__", NC_MAGIC_FN),
    (b"__delattr__", NC_MAGIC_FN),
    (b"__delete__", NC_MAGIC_FN),
    (b"__delitem__", NC_MAGIC_FN),
    (b"__delslice__", NC_MAGIC_FN),
    (b"__dict__", NC_MAGIC_VAR),
    (b"__dir__", NC_MAGIC_FN),
    (b"__div__", NC_MAGIC_FN),
    (b"__divmod__", NC_MAGIC_FN),
    (b"__doc__", NC_MAGIC_VAR),
    (b"__enter__", NC_MAGIC_FN),
    (b"__eq__", NC_MAGIC_FN),
    (b"__exit__", NC_MAGIC_FN),
    (b"__file__", NC_MAGIC_VAR),
    (b"__float__", NC_MAGIC_FN),
    (b"__floor__", NC_MAGIC_FN),
    (b"__floordiv__", NC_MAGIC_FN),
    (b"__format__", NC_MAGIC_FN),
    (b"__fspath__", NC_MAGIC_FN),
    (b"__func__", NC_MAGIC_VAR),
    (b"__ge__", NC_MAGIC_FN),
    (b"__get__", NC_MAGIC_FN),
    (b"__getattr__", NC_MAGIC_FN),
    (b"__getattribute__", NC_MAGIC_FN),
    (b"__getitem__", NC_MAGIC_FN),
    (b"__getnewargs__", NC_MAGIC_FN),
    (b"__getnewargs_ex__", NC_MAGIC_FN),
    (b"__getslice__", NC_MAGIC_FN),
    (b"__getstate__", NC_MAGIC_FN),
    (b"__globals__", NC_MAGIC_VAR),
    (b"__gt__", NC_MAGIC_FN),
    (b"__hash__", NC_MAGIC_FN),
    (b"__hex__", NC_MAGIC_FN),
    (b"__iadd__", NC_MAGIC_FN),
    (b"__iand__", NC_MAGIC_FN),
    (b"__idiv__", NC_MAGIC_FN),
    (b"__ifloordiv__", NC_MAGIC_FN),
    (b"__ilshift__", NC_MAGIC_FN),
    (b"__imatmul__", NC_MAGIC_FN),
    (b"__imod__", NC_MAGIC_FN),
    (b"__import__", NC_BUILTIN),
    (b"__imul__", NC_MAGIC_FN),
    (b"__index__", NC_MAGIC_FN),
    (b"__init__", NC_MAGIC_FN),
    (b"__init_subclass__", NC_MAGIC_FN),
    (b"__instancecheck__", NC_MAGIC_FN),
    (b"__int__", NC_MAGIC_FN),
    (b"__invert__", NC_MAGIC_FN),
    (b"__ior__", NC_MAGIC_FN),
    (b"__ipow__", NC_MAGIC_FN),
    (b"__irshift__", NC_MAGIC_FN),
    (b"__isub__", NC_MAGIC_FN),
    (b"__iter__", NC_MAGIC_FN),
    (b"__itruediv__", NC_MAGIC_FN),
    (b"__ixor__", NC_MAGIC_FN),
    (b"__kwdefaults__", NC_MAGIC_VAR),
    (b"__le__", NC_MAGIC_FN),
    (b"__len__", NC_MAGIC_FN),
    (b"__length_hint__", NC_MAGIC_FN),
    (b"__loader__", NC_MAGIC_VAR),
    (b"__long__", NC_MAGIC_FN),
    (b"__lshift__", NC_MAGIC_FN),
    (b"__lt__", NC_MAGIC_FN),
    (b"__match_args__", NC_MAGIC_VAR),
    (b"__matmul__", NC_MAGIC_FN),
    (b"__members__", NC_MAGIC_VAR),
    (b"__metaclass__", NC_MAGIC_VAR),
    (b"__methods__", NC_MAGIC_VAR),
    (b"__missing__", NC_MAGIC_FN),
    (b"__mod__", NC_MAGIC_FN),
    (b"__module__", NC_MAGIC_VAR),
    (b"__mro__", NC_MAGIC_VAR),
    (b"__mro_entries__", NC_MAGIC_FN),
    (b"__mul__", NC_MAGIC_FN),
    (b"__name__", NC_MAGIC_VAR),
    (b"__ne__", NC_MAGIC_FN),
    (b"__neg__", NC_MAGIC_FN),
    (b"__new__", NC_MAGIC_FN),
    (b"__next__", NC_MAGIC_FN),
    (b"__nonzero__", NC_MAGIC_FN),
    (b"__oct__", NC_MAGIC_FN),
    (b"__or__", NC_MAGIC_FN),
    (b"__package__", NC_MAGIC_VAR),
    (b"__path__", NC_MAGIC_VAR),
    (b"__pos__", NC_MAGIC_FN),
    (b"__post_init__", NC_MAGIC_FN),
    (b"__pow__", NC_MAGIC_FN),
    (b"__prepare__", NC_MAGIC_VAR),
    (b"__qualname__", NC_MAGIC_VAR),
    (b"__radd__", NC_MAGIC_FN),
    (b"__rand__", NC_MAGIC_FN),
    (b"__rcmp__", NC_MAGIC_FN),
    (b"__rdiv__", NC_MAGIC_FN),
    (b"__rdivmod__", NC_MAGIC_FN),
    (b"__reduce__", NC_MAGIC_FN),
    (b"__reduce_ex__", NC_MAGIC_FN),
    (b"__release_buffer__", NC_MAGIC_FN),
    (b"__repr__", NC_MAGIC_FN),
    (b"__reversed__", NC_MAGIC_FN),
    (b"__rfloordiv__", NC_MAGIC_FN),
    (b"__rlshift__", NC_MAGIC_FN),
    (b"__rmatmul__", NC_MAGIC_FN),
    (b"__rmod__", NC_MAGIC_FN),
    (b"__rmul__", NC_MAGIC_FN),
    (b"__ror__", NC_MAGIC_FN),
    (b"__round__", NC_MAGIC_FN),
    (b"__rpow__", NC_MAGIC_FN),
    (b"__rrshift__", NC_MAGIC_FN),
    (b"__rshift__", NC_MAGIC_FN),
    (b"__rsub__", NC_MAGIC_FN),
    (b"__rtruediv__", NC_MAGIC_FN),
    (b"__rxor__", NC_MAGIC_FN),
    (b"__self__", NC_MAGIC_VAR),
    (b"__set__", NC_MAGIC_FN),
    (b"__set_name__", NC_MAGIC_FN),
    (b"__setattr__", NC_MAGIC_FN),
    (b"__setitem__", NC_MAGIC_FN),
    (b"__setslice__", NC_MAGIC_FN),
    (b"__setstate__", NC_MAGIC_FN),
    (b"__slots__", NC_MAGIC_VAR),
    (b"__spec__", NC_MAGIC_VAR),
    (b"__str__", NC_MAGIC_FN),
    (b"__sub__", NC_MAGIC_FN),
    (b"__subclasscheck__", NC_MAGIC_FN),
    (b"__subclasses__", NC_MAGIC_FN),
    (b"__truediv__", NC_MAGIC_FN),
    (b"__trunc__", NC_MAGIC_FN),
    (b"__type_params__", NC_MAGIC_VAR),
    (b"__unicode__", NC_MAGIC_FN),
    (b"__weakref__", NC_MAGIC_VAR),
    (b"__xor__", NC_MAGIC_FN),
    (b"abs", NC_BUILTIN),
    (b"aiter", NC_BUILTIN),
    (b"all", NC_BUILTIN),
    (b"and", W_LOGIC),
    (b"anext", NC_BUILTIN),
    (b"any", NC_BUILTIN),
    (b"apply", NC_BUILTIN),
    (b"as", W_AS),
    (b"ascii", NC_BUILTIN),
    (b"assert", K_ASSERT | W_KW),
    (b"async", W_ASYNC),
    (b"await", K_AWAIT | W_KW),
    (b"basestring", NC_TYPE),
    (b"bin", NC_BUILTIN),
    (b"bool", NC_TYPE),
    (b"break", K_BREAK | W_KW),
    (b"breakpoint", NC_BUILTIN),
    (b"buffer", NC_TYPE),
    (b"bytearray", NC_TYPE),
    (b"bytes", NC_TYPE),
    (b"callable", NC_BUILTIN),
    (b"case", W_CASE),
    (b"chr", NC_BUILTIN),
    (b"class", W_CLASS),
    (b"classmethod", NC_BUILTIN),
    (b"cls", NC_LANGUAGE),
    (b"cmp", NC_BUILTIN),
    (b"coerce", NC_BUILTIN),
    (b"compile", NC_BUILTIN | W_RECALL),
    (b"complex", NC_TYPE),
    (b"continue", K_CONTINUE | W_KW),
    (b"def", W_DEF),
    (b"del", K_DEL | W_KW),
    (b"delattr", NC_BUILTIN),
    (b"dict", NC_TYPE),
    (b"dir", NC_BUILTIN),
    (b"divmod", NC_BUILTIN),
    (b"elif", W_ELIF),
    (b"else", W_ELSE),
    (b"enumerate", NC_BUILTIN),
    (b"eval", NC_BUILTIN),
    (b"except", W_EXCEPT),
    (b"exec", NC_BUILTIN | W_EXEC),
    (b"execfile", NC_BUILTIN),
    (b"execute", W_SQLCALL),
    (b"executemany", W_SQLCALL),
    (b"executescript", W_SQLCALL),
    (b"filter", NC_BUILTIN),
    (b"finally", W_FINALLY),
    (b"findall", W_RECALL),
    (b"finditer", W_RECALL),
    (b"float", NC_TYPE),
    (b"for", W_FOR),
    (b"format", NC_BUILTIN),
    (b"from", W_FROM),
    (b"frozenset", NC_TYPE),
    (b"fullmatch", W_RECALL),
    (b"getattr", NC_BUILTIN),
    (b"global", W_GLOBAL),
    (b"globals", NC_BUILTIN),
    (b"hasattr", NC_BUILTIN),
    (b"hash", NC_BUILTIN),
    (b"help", NC_BUILTIN),
    (b"hex", NC_BUILTIN),
    (b"id", NC_BUILTIN),
    (b"if", W_IF),
    (b"import", W_IMPORT),
    (b"in", W_IN),
    (b"input", NC_BUILTIN),
    (b"int", NC_TYPE),
    (b"intern", NC_BUILTIN),
    (b"is", W_LOGIC),
    (b"isinstance", NC_BUILTIN),
    (b"issubclass", NC_BUILTIN),
    (b"iter", NC_BUILTIN),
    (b"lambda", W_LAMBDA),
    (b"len", NC_BUILTIN),
    (b"list", NC_TYPE),
    (b"locals", NC_BUILTIN),
    (b"long", NC_TYPE),
    (b"map", NC_BUILTIN),
    (b"match", W_MATCH),
    (b"max", NC_BUILTIN),
    (b"memoryview", NC_TYPE),
    (b"min", NC_BUILTIN),
    (b"mogrify", W_SQLCALL),
    (b"next", NC_BUILTIN),
    (b"nonlocal", W_NONLOCAL),
    (b"not", W_LOGIC),
    (b"object", NC_TYPE),
    (b"oct", NC_BUILTIN),
    (b"open", NC_BUILTIN),
    (b"or", W_LOGIC),
    (b"ord", NC_BUILTIN),
    (b"pass", K_PASS | W_KW),
    (b"pow", NC_BUILTIN),
    (b"print", NC_BUILTIN | W_PRINT),
    (b"property", NC_BUILTIN),
    (b"raise", W_RAISE),
    (b"range", NC_BUILTIN),
    (b"raw", W_SQLCALL),
    (b"raw_input", NC_BUILTIN),
    (b"read_sql", W_SQLCALL),
    (b"read_sql_query", W_SQLCALL),
    (b"reduce", NC_BUILTIN),
    (b"reload", NC_BUILTIN),
    (b"repr", NC_BUILTIN),
    (b"return", K_RETURN | W_KW),
    (b"reversed", NC_BUILTIN),
    (b"round", NC_BUILTIN),
    (b"search", W_RECALL),
    (b"self", NC_LANGUAGE),
    (b"set", NC_TYPE),
    (b"setattr", NC_BUILTIN),
    (b"slice", NC_TYPE),
    (b"sorted", NC_BUILTIN),
    (b"split", W_RECALL),
    (b"staticmethod", NC_BUILTIN),
    (b"str", NC_TYPE),
    (b"sub", W_RECALL),
    (b"subn", W_RECALL),
    (b"sum", NC_BUILTIN),
    (b"super", NC_BUILTIN),
    (b"text", W_SQLCALL),
    (b"try", W_TRY),
    (b"tuple", NC_TYPE),
    (b"type", NC_BUILTIN | W_TYPE),
    (b"unichr", NC_BUILTIN),
    (b"unicode", NC_TYPE),
    (b"vars", NC_BUILTIN),
    (b"while", W_WHILE),
    (b"with", W_WITH),
    (b"xrange", NC_TYPE),
    (b"yield", W_YIELD),
    (b"zip", NC_BUILTIN),
];
const N: usize = WORD_LIST.len();
static WORD_TEXT: [&[u8]; N] = {
    let mut t: [&[u8]; N] = [b""; N];
    let mut i = 0;
    while i < N {
        t[i] = WORD_LIST[i].0;
        i += 1;
    }
    t
};
static WORDS: Words<1024, N, { text_len(&WORD_TEXT) }> = Words::new(&WORD_TEXT);
/// A word's info by its id; 0 for none.
static INFO: [u16; N + 1] = {
    let mut t = [0u16; N + 1];
    let mut i = 0;
    while i < N {
        t[i + 1] = WORD_LIST[i].1;
        i += 1;
    }
    t
};

// The frames, a byte a level in [`Scan::f`]; 0 is a level lost past [`LEVELS`].
/// `(` … `)` of code: a group, a tuple, a generator.
const F_GROUP: u8 = 1;
/// A call's arguments.
const F_ARGS: u8 = 2;
/// A call's arguments whose first one is SQL where it starts with a statement's keyword.
const F_SQLARG: u8 = 3;
/// A regex call's arguments: the first one is its pattern.
const F_REARG: u8 = 4;
/// A def's or a lambda's parameters, where a name stands.
const F_PARAMS: u8 = 5;
/// A parameter's annotation, after its `:`.
const F_PANN: u8 = 6;
/// A parameter's default, after its `=`.
const F_PDEF: u8 = 7;
/// A class's bases.
const F_BASES: u8 = 8;
/// `from … import (` … `)`.
const F_IMPORT: u8 = 9;
/// `[` … `]`: a list, a comprehension.
const F_LIST: u8 = 10;
/// `[` … `]` after a value: a subscript, a slice.
const F_SUB: u8 = 11;
/// PEP 695's type parameters, where a name stands.
const F_TPARAMS: u8 = 12;
/// A type parameter's bound or default.
const F_TBOUND: u8 = 13;
/// `{` … `}` before a `:`: a set, or a dict yet.
const F_BRACE: u8 = 14;
/// `{` … `}` after a key's `:`: a dict.
const F_DICT: u8 = 15;
/// An f-string's replacement field: this bit and the string's kind to go on with after its `}`
/// ([`field`]).
const F_FIELD: u8 = 0x80;
/// The levels [`Scan::f`] holds: [`State::levels`] and the first twelve of its withs.
const LEVELS: usize = 26;

/// The field frame of an f-string of kind `sk`, `spec` in its format spec.
#[inline(always)]
const fn field(sk: u8, spec: bool) -> u8 {
    let c = match content(sk) {
        C_SQL | C_RE => content(sk),
        _ => C_PLAIN,
    };
    F_FIELD | (sk & (Q_MASK | RAW)) | if spec { 8 } else { 0 } | c << 4
}

/// The string kind a field frame `f` goes on with.
#[inline(always)]
const fn field_sk(f: u8) -> u8 {
    (f & (Q_MASK | RAW)) | FSTR | ((f >> 4) & 3) << 5
}

// A string's kind ([`SK`]): its quotes, prefix and content, a byte.
/// `'`, `"`, `'''`, `"""`: the quotes' kind, and the text scope's.
const Q_MASK: u8 = 3;
/// Triple-quoted.
const Q_TRIPLE: u8 = 2;
const RAW: u8 = 4;
/// An f-string (or a t-string).
const FSTR: u8 = 8;
const BYTES: u8 = 16;
/// What the string's text is, in the top three bits.
const C_SHIFT: u8 = 5;
const C_PLAIN: u8 = 0;
const C_DOC: u8 = 1;
const C_SQL: u8 = 2;
const C_RE: u8 = 3;
/// Undecided yet: its first text decides SQL (bat's uppercase keyword) or plain.
const C_DETECT_UP: u8 = 4;
/// Undecided yet: its first text decides SQL (a statement's keyword in any case) or plain.
const C_DETECT_ANY: u8 = 5;
/// A string's kind: see [`Q_MASK`].
type SK = u8;

#[inline(always)]
const fn content(sk: SK) -> u8 {
    sk >> C_SHIFT
}

#[inline(always)]
const fn with_content(sk: SK, c: u8) -> SK {
    (sk & 0x1f) | c << C_SHIFT
}

/// The text scope of a string of kind `sk`.
#[inline(always)]
const fn text_scope(sk: SK) -> u16 {
    if content(sk) == C_DOC {
        T_DOC
    } else {
        (sk & Q_MASK) as u16
    }
}

// What the previous significant token was ([`Scan::prev`]).
/// A statement's start: a line's, or after a block head's `:` or `;`.
const P_START: u8 = 0;
/// After `(`, `[`, `{` or `,` in brackets: an operand, or a keyword argument's name.
const P_OPEN: u8 = 1;
/// An operator or a keyword: an operand may start.
const P_OP: u8 = 2;
/// A value's end: a name, `)`, `]`, `}`, a number, a string.
const P_VALUE: u8 = 3;
/// `.`: a member's name follows.
const P_DOT: u8 = 4;
/// `@` at a statement's start, or a `.` in a decorator's name: the decorator's name goes on.
const P_DECO: u8 = 5;
/// After `yield`: `from` is `yield from`'s.
const P_YIELD: u8 = 6;

// A statement's head ([`Scan::head`]), five bits.
const H_NONE: u8 = 0;
const H_IMPORT: u8 = 1;
/// `from` … before its `import`.
const H_FROM: u8 = 2;
/// After `def`: its name.
const H_DEF: u8 = 3;
/// After a def's name: type parameters, the parameters.
const H_DEF_NAMED: u8 = 4;
/// After a def's parameters: `->`, its `:`.
const H_DEF_RET: u8 = 5;
const H_CLASS: u8 = 6;
/// After a class's name: type parameters, the bases, its `:`.
const H_CLASS_NAMED: u8 = 7;
const H_IF: u8 = 8;
const H_ELIF: u8 = 9;
const H_ELSE: u8 = 10;
const H_WHILE: u8 = 11;
/// `for` … before its `in`: the targets.
const H_FOR: u8 = 12;
/// `for … in` …: the iterable.
const H_FOR_IN: u8 = 13;
const H_WITH: u8 = 14;
const H_TRY: u8 = 15;
const H_EXCEPT: u8 = 16;
const H_FINALLY: u8 = 17;
const H_MATCH: u8 = 18;
const H_CASE: u8 = 19;
/// `global`, `nonlocal`.
const H_GLOBAL: u8 = 20;
const H_RAISE: u8 = 21;
/// `type` …: its name.
const H_TYPE: u8 = 22;
/// After a type alias's name: type parameters, `=`.
const H_TYPE_NAMED: u8 = 23;
/// `import` after `from …`: the names imported.
const H_FROM_IMPORT: u8 = 24;

// What the next string is, set by what it follows ([`Scan::want`]).
const WANT_NONE: u8 = 0;
const WANT_RE: u8 = 1;

// [`State::arg`]: the open string's kind, whether there is one, the head, and the `\`.
const ARG_INSTR: u16 = 1 << 8;
const ARG_HEAD_SHIFT: u16 = 9;
const ARG_CONT: u16 = 1 << 14;

/// How far a line's lookaheads go ([`Scan::kwarg_eq`], the block head's `:`).
const LOOK: usize = 256;

/// Lex `b`, a line of Python without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    line_on::<_, false>(b, state, out, Spans::new());
}

/// [`line`]'s runs by theme rule pushed to `runs`.
#[inline(never)]
pub fn line_runs(
    b: &[u8],
    state: &mut State,
    out: &mut Out,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
) {
    let mut k = line_on::<_, true>(b, state, out, Runs::new(rules, runs));
    k.end_line(b.len());
}

#[inline(always)]
fn line_on<K: Sink, const SHARED: bool>(b: &[u8], state: &mut State, out: &mut Out, k: K) -> K {
    let mut s = Scan::new(b, out, k);
    s.load(state);
    s.k.open(s.out, b.len() / 4 + 16);
    s.lex_line::<SHARED>();
    s.k.close(s.out);
    *state = s.state();
    s.k
}

/// A block of Python on one scan opened from `state` ([`crate::Carry`]).
pub fn block(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    let mut out = Out::new(spans);
    let mut s = Scan::new(b"", &mut out, Spans::new());
    s.load(state);
    s.out.0.clear();
    s.k.open(s.out, 64);
    let rest = pass(&mut s);
    *state = s.state();
    rest
}

/// [`block`]'s runs by theme rule pushed to `runs` ([`crate::CarryRuns`]).
pub fn block_runs(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    lines: &mut crate::RunLines,
) -> Option<usize> {
    let mut out = Out::new(spans);
    let mut s = Scan::new(b"", &mut out, Runs::new(rules, runs));
    s.load(state);
    s.out.0.clear();
    s.k.open(s.out, 64);
    let rest = lines.pass(&mut s);
    *state = s.state();
    rest
}

impl crate::Carry for Scan<'_, '_, Spans> {
    fn next(&mut self, b: &[u8]) -> &[crate::Span] {
        // SAFETY: the line is set back to an empty one before `b`'s borrow ends
        self.carry::<false>(unsafe { crate::line_for(b) });
        self.out.0
    }
}

impl crate::CarryRuns for Scan<'_, '_, Runs<'_>> {
    fn next(&mut self, b: &[u8]) -> usize {
        // SAFETY: as above
        self.carry::<true>(unsafe { crate::line_for(b) });
        self.k.end_line(b.len())
    }
}

struct Scan<'b, 'c, K: Sink> {
    b: &'b [u8],
    /// What each open frame is ([`F_GROUP`]…), outermost first; past [`LEVELS`] the inner
    /// ones are lost (read as 0).
    f: [u8; LEVELS],
    /// How many frames are open, up to 255.
    depth: u8,
    /// The innermost frame, [`Scan::f`]'s at `depth - 1` (0 for none, or a level lost).
    tf: u8,
    /// The string open at the line's start ([`Scan::instr`]): its kind.
    sk: SK,
    instr: bool,
    /// The open string's regex state or SQL context.
    ctx: u8,
    /// [`Scan::ctx`] as an f-string's field found it, to go on with after the field.
    fctx: u8,
    /// The statement's head ([`H_NONE`]…), carried while brackets are open or a `\` continues
    /// the line.
    head: u8,
    /// The line ends in a `\`: the next one goes on with the statement.
    cont: bool,
    /// The depth a decorator's arguments opened at, 0 for none: inside them every write is
    /// [`META`]'s.
    deco: u8,
    /// [`META`] inside a decorator, else 0: the bit every write takes.
    meta: u16,
    /// Where the writes inside a decorator end, for the plain text after them.
    gap: usize,
    // The line's own, reset at its start:
    /// The previous significant token ([`P_START`]…).
    prev: u8,
    /// The depth a lambda's parameters opened at plus one, 0 for none; past a `=` its default.
    lam: u8,
    lam_def: bool,
    /// The depth a `for`'s targets opened at plus one, 0 for none: to its `in`.
    fortgt: u8,
    /// What the string at [`Scan::want_at`] is ([`WANT_RE`]…): a keyword argument's.
    want: u8,
    want_at: usize,
    /// The last call's name is a regex function's or a SQL call's: its `(` opens [`F_REARG`]
    /// or [`F_SQLARG`].
    callee: u8,
    /// Where the last string closed and what its text was ([`C_SQL`], [`C_RE`] by a call or
    /// keyword): an adjacent string is the same.
    str_end: usize,
    str_c: u8,
    /// Where the SQL string open on the line ends (its quote or the line's end), found once for
    /// its turns between fields; `usize::MAX` for not yet.
    send: usize,
    /// What SQL's scan keeps between the host's turns on a line.
    sql: sql::Embedded,
    k: K,
    out: &'b mut Out<'c>,
}

impl<'b, 'c, K: Sink> Scan<'b, 'c, K> {
    #[inline(always)]
    fn new(b: &'b [u8], out: &'b mut Out<'c>, k: K) -> Self {
        Scan {
            b,
            f: [0; LEVELS],
            depth: 0,
            tf: 0,
            sk: 0,
            instr: false,
            ctx: 0,
            fctx: 0,
            head: H_NONE,
            cont: false,
            deco: 0,
            meta: 0,
            gap: 0,
            prev: P_START,
            lam: 0,
            lam_def: false,
            fortgt: 0,
            want: WANT_NONE,
            want_at: 0,
            callee: 0,
            str_end: usize::MAX,
            str_c: 0,
            send: usize::MAX,
            sql: sql::Embedded {
                is_end: usize::MAX,
                at: 0,
            },
            k,
            out,
        }
    }

    /// The scan's fields from `state`: the frames in its levels and the first twelve withs, the
    /// string's context and the decorator's depth in the next two, the string's kind, the head
    /// and the `\` in [`State::arg`].
    #[inline(always)]
    fn load(&mut self, state: &State) {
        self.f[..14].copy_from_slice(&state.levels);
        self.f[14..].copy_from_slice(&state.withs[..12]);
        self.depth = state.depth;
        self.tf = self.read_top();
        self.ctx = state.withs[12];
        self.fctx = state.withs[12];
        self.deco = state.withs[13];
        self.sk = state.arg as u8;
        self.instr = state.arg & ARG_INSTR != 0;
        self.head = (state.arg >> ARG_HEAD_SHIFT) as u8 & 31;
        self.cont = state.arg & ARG_CONT != 0;
    }

    /// The state the scan's last line left: every field as it is, so a block lexes as its
    /// lines do one at a time.
    #[inline(always)]
    fn state(&self) -> State {
        let mut s = State::default();
        s.levels.copy_from_slice(&self.f[..14]);
        s.withs[..12].copy_from_slice(&self.f[14..]);
        s.withs[12] = if self.instr { self.ctx } else { self.fctx };
        s.withs[13] = self.deco;
        s.depth = self.depth;
        s.arg = u16::from(self.sk)
            | if self.instr { ARG_INSTR } else { 0 }
            | u16::from(self.head) << ARG_HEAD_SHIFT
            | if self.cont { ARG_CONT } else { 0 };
        s
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

    /// A write: one call on the run sink.
    #[inline(always)]
    fn put(&mut self, s: usize, e: usize, kind: u16) {
        if self.meta != 0 {
            return self.put_meta(s, e, kind);
        }
        self.k.merge_call(&mut self.out, s, e, kind);
    }

    /// A write inlined, for the common tokens.
    #[inline(always)]
    fn put_hot(&mut self, s: usize, e: usize, kind: u16) {
        if self.meta != 0 {
            return self.put_meta(s, e, kind);
        }
        self.k.merge(&mut self.out, s, e, kind);
    }

    /// A write inside a decorator: the plain text before it and it under [`META`].
    #[cold]
    #[inline(never)]
    fn put_meta(&mut self, s: usize, e: usize, kind: u16) {
        if s > self.gap {
            self.k.merge_call(&mut self.out, self.gap, s, META);
        }
        if e > s {
            self.k.merge_call(&mut self.out, s, e, kind | META);
        }
        self.gap = self.gap.max(e);
    }

    /// The decorator ends at `p` (its name without a call, its arguments' `)`, or the line's end
    /// inside them).
    #[cold]
    #[inline(never)]
    fn end_meta(&mut self, p: usize) {
        if self.meta != 0 && p > self.gap {
            self.k.merge_call(&mut self.out, self.gap, p, META);
        }
        self.gap = p;
        self.meta = 0;
    }

    #[inline(always)]
    fn top(&self) -> u8 {
        self.tf
    }

    /// The innermost frame from [`Scan::f`].
    #[inline(always)]
    fn read_top(&self) -> u8 {
        let d = usize::from(self.depth);
        if d == 0 || d > LEVELS {
            0
        } else {
            self.f[d - 1]
        }
    }

    #[inline(always)]
    fn set_top(&mut self, f: u8) {
        let d = usize::from(self.depth);
        if d > 0 && d <= LEVELS {
            self.f[d - 1] = f;
            self.tf = f;
        }
    }

    #[inline(always)]
    fn push(&mut self, f: u8) {
        let d = usize::from(self.depth);
        if d < LEVELS {
            self.f[d] = f;
            self.tf = f;
        } else {
            self.tf = 0;
        }
        self.depth = self.depth.saturating_add(1);
    }

    #[inline(always)]
    fn pop(&mut self) {
        if self.depth > 0 {
            self.depth -= 1;
            let d = usize::from(self.depth);
            if d < LEVELS {
                self.f[d] = 0;
            }
            self.tf = self.read_top();
        }
    }

    /// The line, from what the line before left: an open string's rest first; the loop out of
    /// line where `SHARED` ([`Scan::code_shared`]).
    #[inline(always)]
    fn lex_line<const SHARED: bool>(&mut self) {
        self.prev = if self.depth > 0 {
            P_OPEN
        } else if self.cont {
            P_OP
        } else {
            self.head = H_NONE;
            P_START
        };
        self.cont = false;
        self.lam = 0;
        self.lam_def = false;
        self.fortgt = 0;
        self.want = WANT_NONE;
        self.callee = 0;
        self.str_end = usize::MAX;
        self.send = usize::MAX;
        if self.deco != 0 {
            self.meta = META;
            self.gap = 0;
        }
        let p = if self.instr { self.string_rest() } else { 0 };
        if SHARED {
            self.code_shared(p);
        } else {
            self.code(p);
        }
        let n = self.b.len();
        if self.meta != 0 {
            // a decorator's end, or its arguments' line (they go on on the next)
            self.end_meta(n);
        }
        // the one byte the state keeps for both, so a block lexes as its lines do: the open string's
        // context, or else the one a field below saved for its string
        // (none outside brackets), and no string's kind where none is open
        if self.instr {
            self.fctx = self.ctx;
        } else {
            if self.depth == 0 {
                self.fctx = 0;
            }
            self.ctx = self.fctx;
            self.sk = 0;
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
            let q = match ACT[usize::from(c)] {
                A_SPACE => {
                    let mut e = p + 1;
                    while e < n && ACT[usize::from(b[e])] == A_SPACE {
                        e += 1;
                    }
                    e
                }
                A_NAME => self.name(p),
                A_DIGIT => self.number(p),
                A_QUOTE => self.string(p, p),
                A_HASH => self.comment(p),
                A_DOT => self.dot(p),
                A_OP => self.op(p),
                A_LPAREN => self.lparen(p),
                A_RPAREN => self.rparen(p),
                A_LBRACKET => self.lbracket(p),
                A_RBRACKET => self.rbracket(p),
                A_LBRACE => self.lbrace(p),
                A_RBRACE => self.rbrace(p),
                A_COMMA => self.comma(p),
                A_COLON => self.colon(p),
                A_SEMI => {
                    self.put(p, p + 1, TERMINATOR);
                    self.head = H_NONE;
                    self.prev = P_OP;
                    p + 1
                }
                A_AT => self.at_sign(p),
                A_BACKSLASH => {
                    if p + 1 == n {
                        self.put(p, n, CONTINUATION);
                        self.cont = true;
                    }
                    p + 1
                }
                A_WIDE => {
                    if is_word_starting(b, p) {
                        self.name(p)
                    } else {
                        p + char_len(c)
                    }
                }
                _ => {
                    self.prev = P_OP;
                    p + 1
                }
            };
            debug_assert!(q > p, "the code loop stands at {p}");
            p = q;
        }
    }

    /// A name's end from `p`: its ASCII run, then past ASCII where it goes on.
    #[inline(always)]
    fn word_end(&self, p: usize) -> usize {
        let b = self.b;
        let e = name_run(b, p + 1);
        if e < b.len() && b[e] >= 0x80 {
            word_end(b, e)
        } else {
            e
        }
    }

    /// The byte past the blanks from `e`, if any.
    #[inline(always)]
    fn after_blanks(&self, mut e: usize) -> Option<u8> {
        let b = self.b;
        while e < b.len() && matches!(b[e], b' ' | b'\t') {
            e += 1;
        }
        b.get(e).copied()
    }

    /// A name at `p`: a string's prefix before its quote, a keyword, or a name colored by its
    /// class and its role where it stands.
    #[inline(always)]
    fn name(&mut self, p: usize) -> usize {
        let b = self.b;
        let e = self.word_end(p);
        if e - p <= 2
            && let Some(&q) = b.get(e)
            && (q == b'"' || q == b'\'')
            && let Some(_) = prefix(&b[p..e])
        {
            return self.string(p, e);
        }
        let w = WORDS.value(&b[p..e], &INFO);
        if self.prev == P_DOT {
            return self.member(p, e, w);
        }
        if w >= W_KW {
            return self.keyword(p, e, w);
        }
        if SOL {
            let class = w & 0xff;
            let role = if self.after_blanks(e) == Some(b'(') {
                R_CALL
            } else {
                R_BARE
            };
            if role | class != 0 {
                self.put_hot(p, e, NAMES + role * 8 + class);
            }
            self.prev = P_VALUE;
            return e;
        }
        if w & W_ACTION != 0
            && let Some(q) = self.soft(p, e, w)
        {
            return q;
        }
        self.plain_name(p, e, w & 0xff, w & W_ACTION)
    }

    /// A name that isn't a keyword where it stands, of class `class` (from the table, or
    /// [`NC_GENERIC`]) and action `act`.
    #[inline(always)]
    fn plain_name(&mut self, p: usize, e: usize, class: u16, act: u16) -> usize {
        let b = self.b;
        let prev = self.prev;
        self.prev = P_VALUE;
        // a role where it stands
        if self.placed(prev)
            && let Some(q) = self.placed_name(p, e, class, prev)
        {
            return q;
        }
        if prev == P_DECO {
            return self.deco_name(p, e, class);
        }
        // a constant by its spelling, but a class's base (`ABC`)
        let class = if class == NC_GENERIC
            && is_constant_start(b[p])
            && is_constant_name(&b[p..e])
            && self.tf != F_BASES
        {
            NC_CONSTANT
        } else {
            class
        };
        let role = if self.after_blanks(e) == Some(b'(') {
            if act != 0 {
                self.callee_of(p, act);
            }
            R_CALL
        } else {
            R_BARE
        };
        if role | class != 0 {
            self.put_hot(p, e, NAMES + role * 8 + class);
        }
        e
    }

    /// Whether a name's role may hang on where it stands (the token before it `prev`): after a
    /// def's, a class's, a type alias's keyword, in an import, a lambda's or a def's parameters,
    /// type parameters, or first in a call's or a class's arguments (a keyword argument's).
    #[inline(always)]
    fn placed(&self, prev: u8) -> bool {
        const HEADS: u32 = 1 << H_DEF | 1 << H_CLASS | 1 << H_TYPE | 1 << H_FROM;
        const ALWAYS: u32 = 1 << F_PARAMS | 1 << F_TPARAMS;
        const OPEN: u32 = 1 << F_ARGS | 1 << F_SQLARG | 1 << F_REARG | 1 << F_BASES;
        let top = u32::from(self.tf);
        self.lam != 0
            || (HEADS >> self.head) & 1 != 0
            || (top < 32
                && ((ALWAYS >> top) & 1 != 0 || (prev == P_OPEN && (OPEN >> top) & 1 != 0)))
    }

    /// A name whose role its place decides (a def's, a class's, a type alias's, a parameter, a
    /// keyword argument's, a lambda's parameter, a type parameter), if it is one, the token before
    /// it `prev`: where it ends.
    #[inline(never)]
    fn placed_name(&mut self, p: usize, e: usize, class: u16, prev: u8) -> Option<usize> {
        let top = self.top();
        // a def's or a class's own name: a magic name keeps its class, a builtin's spelling not
        let own = if matches!(class, NC_MAGIC_FN | NC_MAGIC_VAR) {
            class
        } else {
            NC_GENERIC
        };
        let leaf = match self.head {
            H_DEF if self.depth == 0 => {
                self.head = H_DEF_NAMED;
                NAMES + R_DEF * 8 + own
            }
            H_CLASS if self.depth == 0 => {
                self.head = H_CLASS_NAMED;
                NAMES + R_CLASS * 8 + own
            }
            H_TYPE if self.depth == 0 => {
                self.head = H_TYPE_NAMED;
                TYPE_NAME
            }
            // `from`'s module: plain, whatever it spells (`from .globals import`)
            H_FROM if self.depth == 0 => return Some(e),
            _ if self.lam != 0 && self.lam == self.depth.saturating_add(1) && !self.lam_def => {
                PARAMETER
            }
            _ => match top {
                F_PARAMS => PARAMETER,
                F_TPARAMS => TYPE_PARAM,
                F_ARGS | F_SQLARG | F_REARG | F_BASES if prev == P_OPEN => {
                    let eq = self.kwarg_eq(e)?;
                    if top == F_BASES {
                        CLASS_KWARG
                    } else {
                        let w = &self.b[p..e];
                        if w == b"match" || w == b"pattern" {
                            self.want = WANT_RE;
                            self.want_at = eq + 1;
                        }
                        PARAMETER
                    }
                }
                _ => return None,
            },
        };
        self.put(p, e, leaf);
        Some(e)
    }

    /// Where a keyword argument's `=` is after its name ending at `e`: past blanks, and not
    /// `==`.
    #[inline(always)]
    fn kwarg_eq(&self, e: usize) -> Option<usize> {
        let b = self.b;
        let mut q = e;
        while q < b.len() && matches!(b[q], b' ' | b'\t') {
            q += 1;
        }
        (b.get(q) == Some(&b'=') && b.get(q + 1) != Some(&b'=')).then_some(q)
    }

    /// A decorator's name at `p`, after `@` or a `.` of its: a step of its path before a `.`,
    /// its call's name before `(`, or its last name (the decorator's end).
    #[cold]
    #[inline(never)]
    fn deco_name(&mut self, p: usize, e: usize, class: u16) -> usize {
        // the decorator's names: but a builtin, as `variable.annotation`'s with its class
        let class = if class == NC_BUILTIN {
            NC_BUILTIN
        } else {
            NC_GENERIC
        };
        match self.after_blanks(e) {
            Some(b'.') => {
                // a step of its path: plain, under the decorator
                self.put(p, e, PLAIN);
            }
            Some(b'(') => {
                self.put(p, e, NAMES + R_DECO_CALL * 8 + class);
                self.callee = CALLEE_DECO;
            }
            _ => {
                self.put(p, e, NAMES + R_DECO * 8 + class);
                self.end_meta(e);
            }
        }
        e
    }

    /// A member's name after `.` at `p`, its info `w`: a call's, a constant's, a magic name's;
    /// a builtin's or a keyword's word is plain there.
    #[inline(always)]
    fn member(&mut self, p: usize, e: usize, w: u16) -> usize {
        let b = self.b;
        self.prev = P_VALUE;
        let class = match w & 0xff {
            c @ (NC_MAGIC_FN | NC_MAGIC_VAR) if w < W_KW => c,
            _ if is_constant_start(b[p]) && is_constant_name(&b[p..e]) => NC_CONSTANT,
            _ => NC_GENERIC,
        };
        let role = if self.after_blanks(e) == Some(b'(') {
            let act = w & W_ACTION;
            if !SOL && (act == W_RECALL || act == W_SQLCALL || act == W_MATCH) {
                self.callee_of(p, act);
            }
            R_CALL
        } else {
            R_BARE
        };
        if role | class != 0 {
            self.put_hot(p, e, NAMES + role * 8 + class);
        }
        e
    }

    /// What the call named at `p` (its word's action `act`) takes first: a regex function's
    /// pattern on `re.` or `regex.`, an execute-like call's SQL.
    #[cold]
    #[inline(never)]
    fn callee_of(&mut self, p: usize, act: u16) {
        self.callee = match act {
            W_RECALL | W_MATCH if receiver_is_re(self.b, p) => CALLEE_RE,
            W_SQLCALL => CALLEE_SQL,
            _ => 0,
        };
    }

    /// A soft keyword's or Python 2's statement word at `p` (`type`, `print`, `exec`, `match`,
    /// `case`), its info `w`: where it ends if it is the keyword where it stands.
    #[cold]
    #[inline(never)]
    fn soft(&mut self, p: usize, e: usize, w: u16) -> Option<usize> {
        let at_start = self.prev == P_START && self.depth == 0;
        let leaf = match w & W_ACTION {
            W_TYPE if at_start => {
                // `type X = …`, `type X[T] = …`
                let b = self.b;
                let q = skip_blanks(b, e);
                if q == e || !is_word_starting(b, q) {
                    return None;
                }
                let r = skip_blanks(b, self.word_end(q));
                if !matches!(b.get(r), Some(b'=' | b'[')) || b.get(r + 1) == Some(&b'=') {
                    return None;
                }
                self.head = H_TYPE;
                K_TYPE
            }
            W_PRINT | W_EXEC if at_start => {
                // Python 2's statement: an operand after it, not a call, an assignment or a member
                if matches!(
                    self.after_blanks(e),
                    Some(b'(' | b'=' | b'.' | b',' | b')' | b'[' | b';' | b':')
                ) {
                    return None;
                }
                if w & W_ACTION == W_PRINT {
                    K_PRINT
                } else {
                    K_EXEC
                }
            }
            W_MATCH | W_CASE if at_start => {
                // a statement's head: its line ends with `:` (but a comment), and no `=` or `.`
                // follows the word
                if matches!(
                    self.after_blanks(e),
                    None | Some(b'=' | b'.' | b':' | b',' | b')')
                ) || !ends_block(self.b, e)
                {
                    return None;
                }
                if w & W_ACTION == W_MATCH {
                    self.head = H_MATCH;
                    K_MATCH
                } else {
                    self.head = H_CASE;
                    K_CASE
                }
            }
            _ => return None,
        };
        self.put(p, e, leaf);
        self.prev = P_OP;
        Some(e)
    }

    /// A keyword at `p` to `e`, its info `w`.
    #[inline(never)]
    fn keyword(&mut self, p: usize, e: usize, w: u16) -> usize {
        let at_start = self.prev == P_START && self.depth == 0;
        let mut prev = P_OP;
        let leaf = match w & W_ACTION {
            W_KW => w & 0xff,
            W_LOGIC => LOGICAL,
            W_CONST => {
                prev = P_VALUE;
                CONSTANT
            }
            _ if SOL => match w & W_ACTION {
                W_IMPORT => K_IMPORT,
                W_FROM => K_FROM,
                W_AS => K_IMPORT_AS,
                W_IN => LOGICAL,
                W_DEF => K_DEF,
                W_CLASS => K_CLASS,
                W_LAMBDA => K_LAMBDA,
                W_IF => K_IF,
                W_ELIF => K_ELIF,
                W_ELSE => K_ELSE,
                W_FOR => K_FOR,
                W_WHILE => K_WHILE,
                W_WITH => K_WITH,
                W_TRY => K_TRY,
                W_EXCEPT => K_EXCEPT,
                W_FINALLY => K_FINALLY,
                W_ASYNC => K_ASYNC,
                W_YIELD => K_YIELD,
                W_RAISE => K_RAISE,
                W_GLOBAL => K_GLOBAL,
                _ => K_NONLOCAL,
            },
            W_IMPORT => {
                self.head = if self.head == H_FROM {
                    H_FROM_IMPORT
                } else {
                    H_IMPORT
                };
                K_IMPORT
            }
            W_FROM => match self.prev {
                P_YIELD => K_YIELD_FROM,
                _ if self.head == H_RAISE => K_RAISE_FROM,
                _ => {
                    if at_start {
                        self.head = H_FROM;
                    }
                    K_FROM
                }
            },
            W_AS => match self.head {
                H_WITH => K_WITH_AS,
                H_EXCEPT => K_EXCEPT_AS,
                H_CASE => K_WITH_AS,
                _ => K_IMPORT_AS,
            },
            W_IN => {
                if self.fortgt != 0 && self.fortgt == self.depth.saturating_add(1) {
                    self.fortgt = 0;
                    if self.head == H_FOR && self.depth == 0 {
                        self.head = H_FOR_IN;
                    }
                    K_FOR_IN
                } else {
                    LOGICAL
                }
            }
            W_DEF => {
                self.head = H_DEF;
                K_DEF
            }
            W_CLASS => {
                self.head = H_CLASS;
                K_CLASS
            }
            W_LAMBDA => {
                self.lam = self.depth.saturating_add(1);
                self.lam_def = false;
                K_LAMBDA
            }
            W_IF => {
                if at_start {
                    self.head = H_IF;
                }
                K_IF
            }
            W_ELIF => {
                self.head = H_ELIF;
                K_ELIF
            }
            W_ELSE => {
                if at_start {
                    self.head = H_ELSE;
                }
                K_ELSE
            }
            W_FOR => {
                self.fortgt = self.depth.saturating_add(1);
                if at_start {
                    self.head = H_FOR;
                    K_FOR
                } else {
                    K_GENERATOR
                }
            }
            W_WHILE => {
                self.head = H_WHILE;
                K_WHILE
            }
            W_WITH => {
                if at_start {
                    self.head = H_WITH;
                }
                K_WITH
            }
            W_TRY => {
                self.head = H_TRY;
                K_TRY
            }
            W_EXCEPT => {
                self.head = H_EXCEPT;
                // `except*` (3.11): the keyword's
                if self.b.get(e) == Some(&b'*') {
                    self.put_hot(p, e + 1, K_EXCEPT);
                    self.prev = P_OP;
                    return e + 1;
                }
                K_EXCEPT
            }
            W_FINALLY => {
                self.head = H_FINALLY;
                K_FINALLY
            }
            W_ASYNC => {
                // `async def`, `async for`, `async with`: still the statement's start
                if at_start {
                    prev = P_START;
                }
                K_ASYNC
            }
            W_YIELD => {
                prev = P_YIELD;
                K_YIELD
            }
            W_RAISE => {
                self.head = H_RAISE;
                K_RAISE
            }
            W_GLOBAL => {
                self.head = H_GLOBAL;
                K_GLOBAL
            }
            _ => {
                self.head = H_GLOBAL;
                K_NONLOCAL
            }
        };
        self.put_hot(p, e, leaf);
        self.prev = prev;
        e
    }

    /// An operator at `p` (`=+-*/%&|^~<>!`).
    #[inline(always)]
    fn op(&mut self, p: usize) -> usize {
        let b = self.b;
        let c = b[p];
        let c1 = b.get(p + 1).copied().unwrap_or(0);
        let operand = matches!(self.prev, P_START | P_OPEN | P_OP);
        let (n, kind) = match c {
            b'=' if c1 == b'=' => (2, COMPARISON),
            b'=' => return self.equals(p),
            b'!' if c1 == b'=' => (2, COMPARISON),
            b'!' => return self.bang(p),
            b'<' | b'>' => {
                if c1 == c {
                    if b.get(p + 2) == Some(&b'=') {
                        (3, AUGMENTED)
                    } else {
                        (2, ARITH)
                    }
                } else if c1 == b'=' {
                    (2, COMPARISON)
                } else if c == b'<' && c1 == b'>' {
                    (2, DEPRECATED_OP)
                } else {
                    (1, COMPARISON)
                }
            }
            b'-' if c1 == b'>' => (2, RETURN_ANN),
            b'*' => {
                let two = c1 == b'*';
                let n = if two { 2 } else { 1 };
                if b.get(p + n) == Some(&b'=') {
                    (n + 1, AUGMENTED)
                } else if self.head == H_FROM_IMPORT && !two && self.depth == 0 {
                    (1, IMPORT_ALL)
                } else if operand && !SOL {
                    (n, if two { UNPACK_MAP } else { UNPACK_SEQ })
                } else {
                    (n, ARITH)
                }
            }
            b'/' => {
                let two = c1 == b'/';
                let n = if two { 2 } else { 1 };
                if b.get(p + n) == Some(&b'=') {
                    (n + 1, AUGMENTED)
                } else if !two && self.prev == P_OPEN && self.top() == F_PARAMS && !SOL {
                    (1, POSITIONAL)
                } else {
                    (n, ARITH)
                }
            }
            _ if c1 == b'=' && c != b'~' => (2, AUGMENTED),
            _ => (1, ARITH),
        };
        self.put_hot(p, p + n, kind);
        self.prev = P_OP;
        p + n
    }

    /// `=` at `p`: an assignment, a parameter's default, a field's `=` (its debug).
    #[inline(always)]
    fn equals(&mut self, p: usize) -> usize {
        self.prev = P_OP;
        if !SOL {
            match self.top() {
                F_PARAMS | F_PANN => self.set_top(F_PDEF),
                F_TPARAMS => self.set_top(F_TBOUND),
                0 if self.head == H_TYPE_NAMED && self.depth == 0 => self.head = H_NONE,
                f if f & F_FIELD != 0 && f & 8 == 0 && self.field_debug(p + 1) => {
                    self.put(p, p + 1, DEBUG);
                    return p + 1;
                }
                _ => {
                    if self.lam != 0 && self.lam == self.depth.saturating_add(1) {
                        self.lam_def = true;
                    }
                }
            }
        }
        self.put_hot(p, p + 1, ASSIGN);
        p + 1
    }

    /// Whether a field's `=` ends its expression (its debug): blanks, then `}`, `!` or `:`.
    fn field_debug(&self, q: usize) -> bool {
        matches!(self.after_blanks(q), Some(b'}' | b'!' | b':'))
    }

    /// `!` at `p`, not `!=`: a field's conversion (`!r`, `!s`, `!a`) before its `:` or `}`.
    #[inline(never)]
    fn bang(&mut self, p: usize) -> usize {
        let b = self.b;
        self.prev = P_OP;
        if self.top() & F_FIELD != 0
            && matches!(b.get(p + 1), Some(b'r' | b's' | b'a'))
            && matches!(self.after_blanks(p + 2), Some(b'}' | b':'))
        {
            self.put(p, p + 2, CONVERSION);
            return p + 2;
        }
        p + 1
    }

    /// `.` at `p`: a number's (`.5`), `...`, a relative import's dots, or an accessor.
    #[inline(always)]
    fn dot(&mut self, p: usize) -> usize {
        let b = self.b;
        match b.get(p + 1) {
            Some(b'0'..=b'9') => return self.number(p),
            Some(b'.') if b.get(p + 2) == Some(&b'.') && self.head != H_FROM => {
                self.put(p, p + 3, CONSTANT);
                self.prev = P_VALUE;
                return p + 3;
            }
            _ => {}
        }
        if self.head == H_FROM && self.prev == P_OP && !SOL {
            // `from .`, `from ..pkg`: the relative import's dots
            let mut e = p + 1;
            while e < b.len() && b[e] == b'.' {
                e += 1;
            }
            self.put(p, e, K_RELATIVE);
            return e;
        }
        self.put_hot(p, p + 1, DOT);
        self.prev = if self.prev == P_VALUE && self.meta != 0 && self.deco == 0 {
            P_DECO
        } else {
            P_DOT
        };
        p + 1
    }

    /// `@` at `p`: a decorator at a statement's start, else the matrix operator.
    #[inline(always)]
    fn at_sign(&mut self, p: usize) -> usize {
        if self.prev == P_START && self.depth == 0 && !SOL {
            self.meta = META;
            self.gap = p;
            self.put(p, p + 1, DECO_MARK);
            self.prev = P_DECO;
            return p + 1;
        }
        let n = if self.b.get(p + 1) == Some(&b'=') {
            2
        } else {
            1
        };
        self.put(p, p + n, if n == 2 { AUGMENTED } else { MATRIX });
        self.prev = P_OP;
        p + n
    }

    /// `(` at `p`: a def's parameters, a class's bases, an import list, a call's arguments (a
    /// regex call's, a SQL call's, a decorator's), or a group.
    #[inline(always)]
    fn lparen(&mut self, p: usize) -> usize {
        let call = self.prev == P_VALUE;
        self.prev = P_OPEN;
        if SOL {
            self.put_hot(p, p + 1, if call { ARGS_B } else { GROUP_B });
            self.push(if call { F_ARGS } else { F_GROUP });
            return p + 1;
        }
        let (kind, f) = match self.head {
            H_DEF_NAMED if self.depth == 0 => (PARAMS_B, F_PARAMS),
            H_CLASS_NAMED if self.depth == 0 => (INHERIT_B, F_BASES),
            H_FROM_IMPORT if self.depth == 0 => (IMPORT_B, F_IMPORT),
            _ if call => {
                let f = match self.callee {
                    CALLEE_RE => F_REARG,
                    CALLEE_SQL => F_SQLARG,
                    CALLEE_DECO => {
                        self.deco = self.depth.saturating_add(1);
                        F_ARGS
                    }
                    _ => F_ARGS,
                };
                self.callee = 0;
                (ARGS_B, f)
            }
            _ => {
                self.prev = P_OP;
                (GROUP_B, F_GROUP)
            }
        };
        self.put_hot(p, p + 1, kind);
        self.push(f);
        p + 1
    }

    /// `)` at `p`: the frame it closes, and what that ends (a def's parameters, a decorator's
    /// arguments).
    #[inline(always)]
    fn rparen(&mut self, p: usize) -> usize {
        self.prev = P_VALUE;
        let kind = match self.top() {
            F_GROUP => GROUP_E,
            F_ARGS | F_SQLARG | F_REARG => ARGS_E,
            F_PARAMS | F_PANN | F_PDEF => {
                if self.head == H_DEF_NAMED && self.depth == 1 {
                    self.head = H_DEF_RET;
                }
                PARAMS_E
            }
            F_BASES => INHERIT_E,
            F_IMPORT => IMPORT_E,
            0 if self.depth > LEVELS as u8 => GROUP_E,
            _ if self.depth == 0 => {
                self.put(p, p + 1, STRAY_ROUND);
                return p + 1;
            }
            // a closer that doesn't match its frame: plain, the frame left open
            _ => return p + 1,
        };
        self.put_hot(p, p + 1, kind);
        if self.deco != 0 && self.deco == self.depth {
            self.deco = 0;
            self.pop();
            self.end_meta(p + 1);
            return p + 1;
        }
        self.pop();
        p + 1
    }

    /// `[` at `p`: type parameters after a def's, class's or type alias's name, a subscript
    /// after a value, or a list.
    #[inline(always)]
    fn lbracket(&mut self, p: usize) -> usize {
        let (kind, f) = if !SOL
            && self.depth == 0
            && matches!(self.head, H_DEF_NAMED | H_CLASS_NAMED | H_TYPE_NAMED)
        {
            (TPARAMS_B, F_TPARAMS)
        } else if self.prev == P_VALUE {
            (BRACKETS_B, F_SUB)
        } else {
            (SEQ_B, F_LIST)
        };
        self.put_hot(p, p + 1, kind);
        self.push(f);
        self.prev = P_OPEN;
        p + 1
    }

    /// `]` at `p`.
    #[inline(always)]
    fn rbracket(&mut self, p: usize) -> usize {
        self.prev = P_VALUE;
        let kind = match self.top() {
            F_SUB => BRACKETS_E,
            F_LIST => SEQ_E,
            F_TPARAMS | F_TBOUND => TPARAMS_E,
            0 if self.depth > LEVELS as u8 => SEQ_E,
            _ if self.depth == 0 => {
                self.put(p, p + 1, STRAY_SQUARE);
                return p + 1;
            }
            _ => return p + 1,
        };
        self.put_hot(p, p + 1, kind);
        self.pop();
        p + 1
    }

    /// `{` at `p`: a dict or a set.
    #[inline(always)]
    fn lbrace(&mut self, p: usize) -> usize {
        self.put_hot(p, p + 1, MAPSET_B);
        self.push(F_BRACE);
        self.prev = P_OPEN;
        p + 1
    }

    /// `}` at `p`: a dict's or set's end, or an f-string field's, after which its string goes on.
    #[inline(always)]
    fn rbrace(&mut self, p: usize) -> usize {
        self.prev = P_VALUE;
        let top = self.top();
        let kind = match top {
            F_BRACE => MAPSET_E,
            F_DICT => MAPPING_E,
            f if f & F_FIELD != 0 => return self.field_close(p, f),
            0 if self.depth > LEVELS as u8 => MAPSET_E,
            _ if self.depth == 0 => {
                self.put(p, p + 1, STRAY_CURLY);
                return p + 1;
            }
            _ => return p + 1,
        };
        self.put_hot(p, p + 1, kind);
        self.pop();
        p + 1
    }

    /// `,` at `p`: its separator by the frame or the statement it's in.
    #[inline(always)]
    fn comma(&mut self, p: usize) -> usize {
        self.prev = P_OPEN;
        if SOL {
            return p + 1;
        }
        let kind = match self.top() {
            _ if self.lam != 0 && self.lam == self.depth.saturating_add(1) => {
                self.lam_def = false;
                SEP_PARAMS
            }
            _ if self.fortgt != 0 && self.fortgt == self.depth.saturating_add(1) => SEP_TARGETS,
            F_ARGS => SEP_ARGS,
            F_SQLARG | F_REARG => {
                self.set_top(F_ARGS);
                SEP_ARGS
            }
            F_PARAMS | F_PANN | F_PDEF => {
                self.set_top(F_PARAMS);
                SEP_PARAMS
            }
            F_BASES => SEP_INHERIT,
            F_IMPORT => SEP_IMPORT,
            F_LIST => SEP_SEQ,
            // `with (a as b, c as d):`'s items
            F_GROUP if self.head == H_WITH && self.depth == 1 => SEP_WITH,
            F_GROUP => SEP_TUPLE,
            F_TPARAMS | F_TBOUND => {
                self.set_top(F_TPARAMS);
                SEP_TPARAMS
            }
            F_BRACE => SEP_SET,
            F_DICT => SEP_MAPPING,
            0 if self.depth == 0 => match self.head {
                H_IMPORT | H_FROM_IMPORT => SEP_IMPORT,
                H_GLOBAL => SEP_STORAGE,
                H_WITH => SEP_WITH,
                _ => {
                    self.prev = P_OP;
                    return p + 1;
                }
            },
            _ => {
                self.prev = P_OP;
                return p + 1;
            }
        };
        self.put_hot(p, p + 1, kind);
        p + 1
    }

    /// `:` at `p`: `:=`, a lambda's, a parameter's or a variable's annotation, a dict's key's, a
    /// slice's, a field's format spec, or the end of a block's head.
    #[inline(always)]
    fn colon(&mut self, p: usize) -> usize {
        let b = self.b;
        if b.get(p + 1) == Some(&b'=') {
            self.put(p, p + 2, INLINE);
            self.prev = P_OP;
            return p + 2;
        }
        let top = self.top();
        if top & F_FIELD != 0 {
            return self.spec(p + 1, top, p);
        }
        self.prev = P_OP;
        if SOL {
            return p + 1;
        }
        if self.lam != 0 && self.lam == self.depth.saturating_add(1) {
            self.lam = 0;
            self.put(p, p + 1, FUNC_BEGIN);
            return p + 1;
        }
        let kind = match top {
            F_PARAMS => {
                self.set_top(F_PANN);
                PARAM_ANN
            }
            F_TPARAMS => {
                self.set_top(F_TBOUND);
                PARAM_ANN
            }
            F_BRACE => {
                self.set_top(F_DICT);
                KEY_VALUE
            }
            F_DICT => KEY_VALUE,
            F_SUB => SLICE,
            0 if self.depth == 0 => {
                let k = match self.head {
                    H_NONE => VAR_ANN,
                    H_DEF_RET | H_DEF_NAMED => FUNC_BEGIN,
                    H_CLASS_NAMED => CLASS_BEGIN,
                    H_IF => BLOCK_IF,
                    H_ELIF => BLOCK_ELIF,
                    H_ELSE => BLOCK_ELSE,
                    H_WHILE => BLOCK_WHILE,
                    H_FOR | H_FOR_IN => BLOCK_FOR,
                    H_WITH => BLOCK_WITH,
                    H_TRY => BLOCK_TRY,
                    H_EXCEPT => BLOCK_EXCEPT,
                    H_FINALLY => BLOCK_FINALLY,
                    H_MATCH => BLOCK_MATCH,
                    H_CASE => BLOCK_CASE,
                    _ => return p + 1,
                };
                if k != VAR_ANN {
                    // a block's head ends: a statement may follow on the line
                    self.head = H_NONE;
                    self.prev = P_START;
                }
                k
            }
            _ => return p + 1,
        };
        self.put_hot(p, p + 1, kind);
        p + 1
    }

    /// A number at `p` (a digit, or `.` before one).
    #[inline(always)]
    fn number(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        self.prev = P_VALUE;
        let digits = |mut q: usize, ok: fn(u8) -> bool| {
            while q < n && (ok(b[q]) || b[q] == b'_') {
                q += 1;
            }
            q
        };
        if b[p] == b'0' && p + 1 < n {
            let (base, ok, kind): (u16, fn(u8) -> bool, u16) = match b[p + 1] {
                b'x' | b'X' => (HEX_BASE, |c| c.is_ascii_hexdigit(), HEX),
                b'o' | b'O' => (OCT_BASE, |c| matches!(c, b'0'..=b'7'), OCT),
                b'b' | b'B' => (BIN_BASE, |c| matches!(c, b'0' | b'1'), BIN),
                b'0'..=b'7'
                    if !matches!(
                        b.get(digits(p + 1, |c| c.is_ascii_digit())),
                        Some(b'.' | b'e' | b'E' | b'j' | b'J')
                    ) =>
                {
                    // Python 2's octal, `0777`
                    let e = digits(p + 1, |c| matches!(c, b'0'..=b'7'));
                    self.put(p, p + 1, OCT_BASE);
                    self.put(p + 1, e, OCT);
                    return self.long_suffix(e, OCT_SUFFIX);
                }
                _ => (0, |_| false, 0),
            };
            if base != 0 {
                let e = digits(p + 2, ok);
                self.put(p, p + 2, base);
                self.put(p + 2, e, kind);
                let suffix = match kind {
                    HEX => HEX_SUFFIX,
                    OCT => OCT_SUFFIX,
                    _ => BIN_SUFFIX,
                };
                return self.long_suffix(e, suffix);
            }
        }
        // decimal: digits, a fraction, an exponent, `j`
        let int_end = digits(p, |c| c.is_ascii_digit());
        let mut e = int_end;
        let mut dot = None;
        if e < n && b[e] == b'.' && !matches!(b.get(e + 1), Some(b'.')) {
            dot = Some(e);
            e = digits(e + 1, |c| c.is_ascii_digit());
        }
        let mut exp = false;
        if e < n && matches!(b[e], b'e' | b'E') {
            let s = e + 1 + usize::from(matches!(b.get(e + 1), Some(b'+' | b'-')));
            if b.get(s).is_some_and(u8::is_ascii_digit) {
                e = digits(s, |c| c.is_ascii_digit());
                exp = true;
            }
        }
        let imag = e < n && matches!(b[e], b'j' | b'J');
        if dot.is_none() && !exp && !imag {
            self.put_hot(p, e, INT);
            return self.long_suffix(e, INT_SUFFIX);
        }
        let (text, sep) = if imag {
            (IMAG, IMAG_DOT)
        } else {
            (FLOAT, FLOAT_DOT)
        };
        match dot {
            Some(d) => {
                self.put(p, d, text);
                self.put(d, d + 1, sep);
                self.put(d + 1, e, text);
            }
            None => self.put(p, e, text),
        }
        if imag {
            self.put(e, e + 1, IMAG_SUFFIX);
            return e + 1;
        }
        e
    }

    /// Python 2's `L` (or `l`) after an integer ending at `e`, of `kind`.
    #[inline(always)]
    fn long_suffix(&mut self, e: usize, kind: u16) -> usize {
        if matches!(self.b.get(e), Some(b'l' | b'L'))
            && !self.b.get(e + 1).is_some_and(|&c| WORD[usize::from(c)])
        {
            self.put(e, e + 1, kind);
            return e + 1;
        }
        e
    }

    /// `#` at `p`: a comment to the line's end.
    #[inline(always)]
    fn comment(&mut self, p: usize) -> usize {
        let n = self.b.len();
        if self.meta != 0 {
            self.end_meta(p);
        }
        self.put_hot(p, p + 1, COMMENT_MARK);
        self.put_hot(p + 1, n, COMMENT);
        n
    }
}

// What a call's name says its first argument is ([`Scan::callee`]).
const CALLEE_RE: u8 = 1;
const CALLEE_SQL: u8 = 2;
/// A decorator's call: its arguments are [`META`]'s.
const CALLEE_DECO: u8 = 3;

/// A string's prefix (`r`, `b`, `u`, `f`, `t` and the pairs Python takes, any case): its bits
/// ([`RAW`], [`FSTR`], [`BYTES`]).
#[inline(always)]
fn prefix(w: &[u8]) -> Option<u8> {
    let bit = |c: u8| match c | 0x20 {
        b'r' => Some(RAW),
        b'f' | b't' => Some(FSTR),
        b'b' => Some(BYTES),
        b'u' => Some(0),
        _ => None,
    };
    match *w {
        [a] => bit(a),
        [a, b] => {
            let (x, y) = (bit(a)?, bit(b)?);
            // two of the same, `u` with another, `fb`: none of Python's
            (x != y && x | y != FSTR | BYTES && (x | y) & RAW != 0).then_some(x | y)
        }
        _ => None,
    }
}

/// Where the run of a name's ASCII bytes (letters, digits, `_`) from `from` ends: sixteen bytes a
/// step on x86-64 (SSE2's compares, the line's last step read again from its end less sixteen,
/// as [`crate::prim::find_in`] does; Scala's, X-137), a byte a step elsewhere.
#[inline(always)]
fn name_run(b: &[u8], from: usize) -> usize {
    let n = b.len();
    #[cfg(target_arch = "x86_64")]
    if n >= 16 && from < n {
        use std::arch::x86_64::*;
        // SAFETY: SSE2 is part of x86-64; every load reads `b[q..q + 16]` with `q + 16 <= n`
        unsafe {
            let misses = |q: usize| {
                let w = _mm_loadu_si128(b.as_ptr().add(q).cast());
                // a letter: `(c | 0x20) - 'a' < 26`, unsigned, as a signed compare biased by 128
                let l = _mm_add_epi8(
                    _mm_or_si128(w, _mm_set1_epi8(0x20)),
                    _mm_set1_epi8((128u8.wrapping_sub(b'a')) as i8),
                );
                let alpha = _mm_cmplt_epi8(l, _mm_set1_epi8((128 + 26) as u8 as i8));
                let d = _mm_add_epi8(w, _mm_set1_epi8((128u8.wrapping_sub(b'0')) as i8));
                let digit = _mm_cmplt_epi8(d, _mm_set1_epi8((128 + 10) as u8 as i8));
                let us = _mm_cmpeq_epi8(w, _mm_set1_epi8(b'_' as i8));
                let m = _mm_or_si128(_mm_or_si128(alpha, digit), us);
                !(_mm_movemask_epi8(m) as u32) & 0xffff
            };
            let mut i = from;
            while i + 16 <= n {
                let m = misses(i);
                if m != 0 {
                    return i + m.trailing_zeros() as usize;
                }
                i += 16;
            }
            if i == n {
                return n;
            }
            let q = n - 16;
            let m = misses(q) >> (i - q);
            return if m != 0 {
                i + m.trailing_zeros() as usize
            } else {
                n
            };
        }
    }
    let mut e = from;
    while e < n && WORD[usize::from(b[e])] {
        e += 1;
    }
    e
}

/// Whether byte `c` may start a constant's name: an uppercase letter or `_`.
#[inline(always)]
fn is_constant_start(c: u8) -> bool {
    c.is_ascii_uppercase() || c == b'_'
}

/// bat's `identifier_constant`: an uppercase name of two capitals or more (`_` and digits in it),
/// whole (`MAX_SIZE`, `_AB1`; not `A`, `A1`, `A_B`, `Abc`).
fn is_constant_name(w: &[u8]) -> bool {
    // ASCII: every byte a capital, a digit or `_`, two capitals in a row somewhere; most names
    // fail at their second byte
    let mut pair = false;
    let mut prev_up = false;
    for &c in w {
        let up = c.is_ascii_uppercase();
        if !(up || c == b'_' || c.is_ascii_digit()) {
            if c < 0x80 {
                return false;
            }
            return is_constant_wide(w);
        }
        pair |= up && prev_up;
        prev_up = up;
    }
    pair && (w[0].is_ascii_uppercase() || w[0] == b'_')
}

/// [`is_constant_name`] past ASCII.
#[cold]
fn is_constant_wide(w: &[u8]) -> bool {
    let Ok(s) = std::str::from_utf8(w) else {
        return false;
    };
    let upper = |c: char| c.is_uppercase() && !c.is_lowercase();
    let mut cs = s.chars();
    let Some(first) = cs.next() else {
        return false;
    };
    if !(upper(first) || first == '_') {
        return false;
    }
    let mut prev_upper = upper(first);
    let mut pair = false;
    for c in cs {
        let u = upper(c);
        if !(u || c == '_' || crate::prim::is_nd(c)) {
            return false;
        }
        pair |= u && prev_upper;
        prev_upper = u;
    }
    pair
}

/// Whether the name at `p` is called on `re.` or `regex.` (blanks around the `.` taken).
fn receiver_is_re(b: &[u8], p: usize) -> bool {
    let q = crate::trim_end(b, 0, p);
    if q == 0 || b[q - 1] != b'.' {
        return false;
    }
    let r = crate::trim_end(b, 0, q - 1);
    let w = |name: &[u8]| {
        r >= name.len()
            && &b[r - name.len()..r] == name
            && (r == name.len() || !crate::prim::is_word(b, r - name.len() - 1))
    };
    w(b"re") || w(b"regex")
}

/// Where the blanks from `e` end.
#[inline(always)]
fn skip_blanks(b: &[u8], mut e: usize) -> usize {
    while e < b.len() && matches!(b[e], b' ' | b'\t') {
        e += 1;
    }
    e
}

/// Whether the line from `e` ends a block's head: its last byte before a comment (outside
/// strings) is `:`. Within [`LOOK`] bytes.
fn ends_block(b: &[u8], e: usize) -> bool {
    let end = b.len().min(e + LOOK);
    let mut last = 0u8;
    let mut quote = 0u8;
    let mut q = e;
    while q < end {
        let c = b[q];
        if quote != 0 {
            if c == b'\\' {
                q += 1;
            } else if c == quote {
                quote = 0;
            }
        } else if c == b'#' {
            break;
        } else if c == b'"' || c == b'\'' {
            quote = c;
            last = c;
        } else if !matches!(c, b' ' | b'\t' | b'\r') {
            last = c;
        }
        q += 1;
    }
    last == b':'
}
