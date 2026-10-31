//! Rust by a scanner of ada's own (ADR-40, speed first): bat's `Rust` grammar compiled by hand.
//! Each of its contexts that can be open is an arm of one `match` on the top of a stack, and each
//! tries its patterns, in the grammar's order, at the position the scan has reached: syntect's
//! leftmost match is the first pattern that matches at the first position where one does. Where
//! no pattern can start inside a run of bytes (a word, spaces, a string's text), the run is
//! skipped whole.
//!
//! What the 14 built-in themes tell apart is a token's own scopes and whether it is in an
//! attribute (`meta.annotation`), so a kind is a leaf (bat's scopes for the token) with or
//! without that one meta scope ([`scopes`]). The grammar's other meta scopes (a function's, a
//! block's, a group's) aren't kept; neither are its paths through some bad input, where a word
//! that starts with a digit would match from its middle. `examples/parity.rs` reads how close it
//! comes. Contexts that only differ in meta scopes are one here: a block, a function's body, an
//! impl's or a module's body are [`BLOCK`]. Names past ASCII are approximated: every byte past
//! ASCII is a letter, neither upper- nor lowercase.
//!
//! The stack between lines is the contexts open above the top level, up to 28 (the bytes of
//! [`State`]'s levels), and an open raw string's `#`s.

use crate::machine::KEPT as HALF;
use crate::{Out, State, is_space, skip_spaces};

// The contexts, by bat's names where one is one of them.
/// The top level (`statements`).
const MAIN: u8 = 0;
/// `{ … }`: a block, a function's, an impl's, a trait's, a module's or a macro's body.
const BLOCK: u8 = 1;
/// `( … )`.
const GROUP: u8 = 2;
/// `[ … ]`, where `;` separates.
const BRACKET: u8 = 3;
/// A macro body's `[ … ]`.
const MACRO_BRACKET: u8 = 4;
/// A closure's body without braces, to the end of the line or a closer.
const CLOSURE_BODY: u8 = 5;
/// A trait's or an enum's header, or an impl's before `{`: `statements-block`.
const HEAD: u8 = 6;
/// A module's header, which `;` can end.
const MOD_HEAD: u8 = 7;
/// After an operator: `<` starts generics, `|` a closure.
const AFTER_OP: u8 = 8;
/// After `{` or `(`: `|` starts a closure.
const TRY_CLOSURE: u8 = 9;
const CLOSURE: u8 = 10;
const CLOSURE_PARAMS: u8 = 11;
/// `(a, b)` among parameters.
const PARAM_TUPLE: u8 = 12;
/// `: (A, B)` among parameters.
const PARAM_TUPLE_TYPE: u8 = 13;
/// A closure parameter's type.
const CLOSURE_PARAM_TYPE: u8 = 14;
const CLOSURE_RETURN: u8 = 15;
const RETURN_TYPE: u8 = 16;
const FN_GENERIC: u8 = 17;
const FN_PARAMS: u8 = 18;
const FN_RETURN: u8 = 19;
const FN_WHERE: u8 = 20;
const IMPL_DEF: u8 = 21;
const IMPL_GENERIC: u8 = 22;
const IMPL_FOR: u8 = 23;
/// An impl's trait, before `for`.
const IMPL_TRAIT: u8 = 24;
const IMPL_IDENT: u8 = 25;
const IMPL_WHERE: u8 = 26;
const STRUCT_ID: u8 = 27;
const STRUCT_GENERIC: u8 = 28;
const STRUCT_BODY: u8 = 29;
const STRUCT_FIELDS: u8 = 30;
const FIELD_TYPE: u8 = 31;
/// A tuple struct's fields, which its first `)` ends, at any depth (bat's `with_prototype`).
const TUPLE_FIELDS: u8 = 32;
/// `generic-angles`.
const GENERIC: u8 = 33;
/// `generic-angles-contents`.
const GENERIC_ARGS: u8 = 34;
/// A `<` in them.
const GENERIC_NESTED: u8 = 35;
/// A `[` in them.
const GENERIC_ARRAY: u8 = 36;
/// After its `;`.
const GENERIC_LEN: u8 = 37;
/// After `extern` in them.
const GENERIC_EXTERN: u8 = 38;
/// `(` in a constant expression.
const CONST_GROUP: u8 = 39;
/// `fn(` in a type.
const FN_TYPE: u8 = 40;
const FN_TYPE_RETURN: u8 = 41;
/// `(` in a type.
const TYPE_GROUP: u8 = 42;
/// After `name::` in an expression.
const NO_TYPE_NAMES: u8 = 43;
/// After `macro_rules! name`.
const MACRO_BLOCK: u8 = 44;
const MACRO_RULES: u8 = 45;
const MACRO_RULES_BRACKET: u8 = 46;
const MACRO_RULES_PAREN: u8 = 47;
const MATCHER: u8 = 48;
/// `$(` in a matcher.
const MATCHER_GROUP: u8 = 49;
const MATCH_OP: u8 = 50;
const MACRO_BODY: u8 = 51;
const MACRO_TERM: u8 = 52;
const ATTRIBUTE: u8 = 53;
const ATTR_PARAMS: u8 = 54;
const ATTR_CALL: u8 = 55;
/// A formatting macro's first argument.
const FMT_FIRST: u8 = 56;
/// `write!`'s first argument, before the format string.
const WRITE_FIRST: u8 = 57;
// strings and comments last: no `with_prototype` reaches into them
const STRING: u8 = 58;
const FMT_STRING: u8 = 59;
const BYTE_STRING: u8 = 60;
const RAW_STRING: u8 = 61;
const FMT_RAW: u8 = 62;
const RAW_BYTES: u8 = 63;
const BLOCK_COMMENT: u8 = 64;
const DOC_COMMENT: u8 = 65;

/// The leaves: a token's scopes. A kind is a leaf, plus [`NLEAVES`] in an attribute.
static LEAVES: [&str; 112] = [
    "",
    "comment.line.double-slash.rust",
    "comment.line.double-slash.rust punctuation.definition.comment.rust",
    "comment.line.double-slash.rust variable.other.rust",
    "comment.line.documentation.rust",
    "comment.line.documentation.rust punctuation.definition.comment.rust",
    "comment.line.documentation.rust variable.other.rust",
    "comment.block.rust",
    "comment.block.rust punctuation.definition.comment.rust",
    "comment.block.rust variable.other.rust",
    "comment.block.documentation.rust",
    "comment.block.documentation.rust punctuation.definition.comment.rust",
    "comment.block.documentation.rust variable.other.rust",
    "string.quoted.double.rust",
    "string.quoted.double.rust punctuation.definition.string.begin.rust",
    "string.quoted.double.rust punctuation.definition.string.end.rust",
    "string.quoted.double.rust constant.character.escape.rust",
    "string.quoted.double.rust punctuation.separator.continuation.line.rust",
    "string.quoted.double.rust storage.type.string.rust",
    "string.quoted.double.rust constant.other.placeholder.rust",
    "string.quoted.double.raw.rust",
    "string.quoted.double.raw.rust punctuation.definition.string.begin.rust",
    "string.quoted.double.raw.rust punctuation.definition.string.end.rust",
    "string.quoted.double.raw.rust storage.type.string.rust",
    "string.quoted.double.raw.rust constant.character.escape.rust",
    "string.quoted.double.raw.rust constant.other.placeholder.rust",
    "string.quoted.single.rust",
    "string.quoted.single.rust punctuation.definition.string.begin.rust",
    "string.quoted.single.rust punctuation.definition.string.end.rust",
    "string.quoted.single.rust constant.character.escape.rust",
    "string.quoted.single.rust storage.type.string.rust",
    "punctuation.terminator.rust",
    "punctuation.separator.rust",
    "punctuation.separator.type.rust",
    "punctuation.separator.generic.rust",
    "punctuation.accessor.dot.rust",
    "meta.path.rust punctuation.accessor.rust",
    "meta.path.rust storage.type.rust punctuation.accessor.rust",
    "punctuation.accessor.double-colon.rust",
    "punctuation.section.group.begin.rust",
    "punctuation.section.group.end.rust",
    "meta.group.rust punctuation.section.group.begin.rust",
    "meta.group.rust punctuation.section.group.end.rust",
    "punctuation.section.block.begin.rust",
    "meta.block.rust punctuation.section.block.end.rust",
    "meta.block.rust punctuation.section.block.begin.rust",
    "punctuation.section.parameters.begin.rust",
    "punctuation.section.parameters.end.rust",
    "meta.function.parameters.rust punctuation.section.parameters.end.rust",
    "punctuation.definition.generic.begin.rust",
    "punctuation.definition.generic.end.rust",
    "punctuation.definition.annotation.rust",
    "meta.annotation.parameters.rust meta.group.rust punctuation.section.group.begin.rust",
    "meta.annotation.parameters.rust meta.group.rust punctuation.section.group.end.rust",
    "meta.function-call.rust meta.group.rust punctuation.section.group.begin.rust",
    "meta.function-call.rust meta.group.rust punctuation.section.group.end.rust",
    "meta.function-call.rust variable.function.rust",
    "punctuation.definition.modifier-scope.begin.rust",
    "punctuation.definition.modifier-scope.end.rust",
    "punctuation.terminator.macro-matcher.rust",
    "punctuation.section.group.begin.rust keyword.operator.rust",
    "keyword.other.rust",
    "keyword.control.rust",
    "keyword.operator.rust",
    "keyword.operator.assignment.rust",
    "keyword.operator.comparison.rust",
    "keyword.operator.logical.rust",
    "keyword.operator.arithmetic.rust",
    "keyword.operator.bitwise.rust",
    "keyword.operator.range.rust",
    "storage.type.rust",
    "storage.type.numeric.rust",
    "storage.type.function.rust",
    "storage.type.struct.rust",
    "storage.type.impl.rust",
    "storage.type.enum.rust",
    "storage.type.type.rust",
    "storage.type.trait.rust",
    "storage.type.module.rust",
    "storage.modifier.rust",
    "storage.modifier.lifetime.rust",
    "meta.path.rust storage.type.rust storage.type.rust",
    "entity.name.function.rust",
    "entity.name.struct.rust",
    "entity.name.impl.rust",
    "entity.name.enum.rust",
    "entity.name.trait.rust",
    "entity.name.type.rust",
    "entity.name.module.rust",
    "entity.name.macro.rust",
    "entity.name.label.rust",
    "variable.parameter.rust",
    "variable.parameter.macro.rust",
    "variable.other.member.rust",
    "variable.language.rust",
    "variable.other.rust",
    "variable.annotation.rust",
    "variable.other.constant.rust",
    "support.function.rust",
    "support.macro.rust",
    "support.type.rust",
    "constant.other.rust",
    "constant.language.rust",
    "constant.numeric.integer.decimal.rust",
    "constant.numeric.integer.hexadecimal.rust",
    "constant.numeric.integer.octal.rust",
    "constant.numeric.integer.binary.rust",
    "constant.numeric.float.rust",
    "invalid.illegal.rust",
    "meta.path.rust",
    "meta.path.rust variable.language.rust",
    "meta.path.rust keyword.other.rust",
];

// The leaves by name, in the order of `LEAVES`.
const PLAIN: u16 = 0;
const COMMENT: u16 = 1;
const DOC: u16 = 4;
const BLOCK_TEXT: u16 = 7;
const DOC_BLOCK: u16 = 10;
/// After one of the four comments' text: its mark, then a `$name` in it.
const MARK: u16 = 1;
const VAR: u16 = 2;
const STR: u16 = 13;
const STR_BEGIN: u16 = 14;
const STR_END: u16 = 15;
const STR_ESCAPE: u16 = 16;
const STR_CONTINUATION: u16 = 17;
const STR_PREFIX: u16 = 18;
const STR_PLACEHOLDER: u16 = 19;
const RAW: u16 = 20;
const RAW_BEGIN: u16 = 21;
const RAW_END: u16 = 22;
const RAW_PREFIX: u16 = 23;
const RAW_ESCAPE: u16 = 24;
const RAW_PLACEHOLDER: u16 = 25;
const CHAR: u16 = 26;
const CHAR_BEGIN: u16 = 27;
const CHAR_END: u16 = 28;
const CHAR_ESCAPE: u16 = 29;
const CHAR_PREFIX: u16 = 30;
const TERMINATOR: u16 = 31;
const SEPARATOR: u16 = 32;
const SEPARATOR_TYPE: u16 = 33;
const SEPARATOR_GENERIC: u16 = 34;
const DOT: u16 = 35;
const PATH_ACCESSOR: u16 = 36;
const TYPE_ACCESSOR: u16 = 37;
const DOUBLE_COLON: u16 = 38;
const GROUP_BEGIN: u16 = 39;
const GROUP_END: u16 = 40;
const META_GROUP_BEGIN: u16 = 41;
const META_GROUP_END: u16 = 42;
const BLOCK_BEGIN: u16 = 43;
const BLOCK_END: u16 = 44;
const META_BLOCK_BEGIN: u16 = 45;
const PARAMS_BEGIN: u16 = 46;
const PARAMS_END: u16 = 47;
const FN_PARAMS_END: u16 = 48;
const GENERIC_BEGIN: u16 = 49;
const GENERIC_END: u16 = 50;
const ANNOTATION_MARK: u16 = 51;
const ANNOTATION_BEGIN: u16 = 52;
const ANNOTATION_END: u16 = 53;
const CALL_BEGIN: u16 = 54;
const CALL_END: u16 = 55;
const CALL: u16 = 56;
const MODIFIER_BEGIN: u16 = 57;
const MODIFIER_END: u16 = 58;
const MACRO_TERMINATOR: u16 = 59;
const REPEAT_BEGIN: u16 = 60;
const KEYWORD: u16 = 61;
const CONTROL: u16 = 62;
const OPERATOR: u16 = 63;
const ASSIGNMENT: u16 = 64;
const COMPARISON: u16 = 65;
const LOGICAL: u16 = 66;
const ARITHMETIC: u16 = 67;
const BITWISE: u16 = 68;
const RANGE: u16 = 69;
const STORAGE: u16 = 70;
const NUMERIC: u16 = 71;
const FN: u16 = 72;
const STRUCT: u16 = 73;
const IMPL: u16 = 74;
const ENUM: u16 = 75;
const TYPE: u16 = 76;
const TRAIT: u16 = 77;
const MODULE: u16 = 78;
const MODIFIER: u16 = 79;
const LIFETIME: u16 = 80;
const PATH_TYPE: u16 = 81;
const FN_NAME: u16 = 82;
const STRUCT_NAME: u16 = 83;
const IMPL_NAME: u16 = 84;
const ENUM_NAME: u16 = 85;
const TRAIT_NAME: u16 = 86;
const TYPE_NAME: u16 = 87;
const MODULE_NAME: u16 = 88;
const MACRO_NAME: u16 = 89;
const LABEL: u16 = 90;
const PARAMETER: u16 = 91;
const MACRO_PARAMETER: u16 = 92;
const MEMBER: u16 = 93;
const SELF: u16 = 94;
const VARIABLE: u16 = 95;
const ANNOTATION: u16 = 96;
const CONSTANT_VAR: u16 = 97;
const SUPPORT_FN: u16 = 98;
const SUPPORT_MACRO: u16 = 99;
const SUPPORT_TYPE: u16 = 100;
const CONSTANT: u16 = 101;
const LANGUAGE: u16 = 102;
const DECIMAL: u16 = 103;
const HEX: u16 = 104;
const OCTAL: u16 = 105;
const BINARY: u16 = 106;
const FLOAT: u16 = 107;
const INVALID: u16 = 108;
const PATH: u16 = 109;
const PATH_SELF: u16 = 110;
const PATH_SUPER: u16 = 111;

const NLEAVES: u16 = LEAVES.len() as u16;

/// The scopes under `source.rust` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    let mut out = Vec::new();
    if kind >= NLEAVES {
        out.push("meta.annotation.rust");
    }
    let leaf = LEAVES[usize::from(kind % NLEAVES)];
    out.extend(leaf.split(' ').filter(|s| !s.is_empty()));
    out
}

/// How many contexts a line can hold; one pushed past it drops the bottom one.
const MAX: usize = 128;
/// How many the state keeps between lines: [`State`]'s levels and withs.
const KEPT: usize = 2 * HALF;

/// Lex `b`, a line without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let mut s = Scan {
        b,
        stack: [MAIN; MAX],
        depth: 1,
        hashes: usize::from(state.arg),
        exact: state.is_exact(),
        annot: 0,
        tuple: 0,
        text: 0,
        at: 0,
        out,
    };
    let n = usize::from(state.depth);
    for i in 0..n {
        let c = if i < HALF {
            state.levels[i]
        } else {
            state.withs[i - HALF]
        };
        s.stack[1 + i] = c;
        s.enter(c);
    }
    s.depth = 1 + n;
    s.refresh();
    s.run();
    s.fill(b.len());
    let n = s.depth - 1;
    let from = n.saturating_sub(KEPT);
    let mut packed = State {
        depth: (n - from) as u8,
        inexact: !s.exact || from > 0,
        arg: if matches!(s.top(), RAW_STRING | FMT_RAW | RAW_BYTES) {
            s.hashes as u16
        } else {
            0
        },
        ..State::default()
    };
    for (i, &c) in s.stack[1 + from..s.depth].iter().enumerate() {
        if i < HALF {
            packed.levels[i] = c;
        } else {
            packed.withs[i - HALF] = c;
        }
    }
    *state = packed;
}

struct Scan<'b, 'c> {
    b: &'b [u8],
    stack: [u8; MAX],
    depth: usize,
    /// The open raw string's `#`s.
    hashes: usize,
    exact: bool,
    /// How many attributes are open.
    annot: usize,
    /// How many tuple structs' fields are open.
    tuple: usize,
    /// The kind of text no pattern matches.
    text: u16,
    /// Where the spans so far end.
    at: usize,
    out: &'b mut Out<'c>,
}

/// A byte of a word: `\w`, every byte past ASCII among them.
fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

fn is_letter(c: u8) -> bool {
    c.is_ascii_alphabetic() || c >= 0x80
}

fn word_end(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && is_word(b[p]) {
        p += 1;
    }
    p
}

/// Whether `\b` holds before `p`, at a word's start.
fn starts_word(b: &[u8], p: usize) -> bool {
    p == 0 || !is_word(b[p - 1])
}

/// Whether `\b` holds at `e`, after a word.
fn ends_word(b: &[u8], e: usize) -> bool {
    e >= b.len() || !is_word(b[e])
}

/// bat's identifier, `(?:[\p{L}][_\p{L}\p{N}]*|_[_\p{L}\p{N}]+)\b`, at `p`: where it ends.
fn ident_end(b: &[u8], p: usize) -> Option<usize> {
    let c = *b.get(p)?;
    let ok = is_letter(c) || (c == b'_' && b.get(p + 1).is_some_and(|&d| is_word(d)));
    ok.then(|| word_end(b, p))
}

/// Whether a whole word is bat's identifier.
fn is_ident(w: &[u8]) -> bool {
    match w {
        [c, ..] if is_letter(*c) => true,
        [b'_', _, ..] => true,
        _ => false,
    }
}

/// `[\p{Ll}_][\p{Ll}\p{Nd}_]*`, a whole word.
fn is_lower(w: &[u8]) -> bool {
    matches!(w.first(), Some(b'a'..=b'z' | b'_'))
        && w.iter()
            .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

/// `[\p{Lu}_][\p{Lu}\p{Nd}_]+`, a whole word: a constant's name.
fn is_constant(w: &[u8]) -> bool {
    w.len() >= 2
        && matches!(w[0], b'A'..=b'Z' | b'_')
        && w.iter()
            .all(|&c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
}

/// `c_[\p{Ll}\p{Nd}_]+|[\p{Ll}_][\p{Ll}\p{Nd}_]*_t`, a whole word: a C type's name.
fn is_c_type(w: &[u8]) -> bool {
    w.len() >= 3 && is_lower(w) && (w.starts_with(b"c_") || w.ends_with(b"_t"))
}

/// `[\p{Lu}]|_*[\p{Lu}][\p{L}\p{N}_]*[\p{Ll}][\p{L}\p{N}_]*`, a whole word: a type's name
/// before `::`.
fn is_type_name(w: &[u8]) -> bool {
    let rest = &w[w.iter().take_while(|&&c| c == b'_').count()..];
    match rest.split_first() {
        Some((c, tail)) if c.is_ascii_uppercase() => {
            (w.len() == 1) || tail.iter().any(u8::is_ascii_lowercase)
        }
        _ => false,
    }
}

/// What a word is to bat's grammar, by [`lookup`]: a keyword (its leaf), one of the `support.type`
/// names, a primitive type (or `Self`), or another name.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Name {
    Other,
    Keyword(u16),
    Support,
    Primitive,
}

/// The word `w` among bat's keywords and type names, dispatching on its first byte.
fn lookup(w: &[u8]) -> Name {
    use Name::{Keyword as K, Other, Primitive, Support};
    let Some(&c) = w.first() else {
        return Other;
    };
    match c {
        b'a' => match w {
            b"as" => K(OPERATOR),
            b"alignof" | b"abstract" => K(INVALID),
            _ => Other,
        },
        b'b' => match w {
            b"break" => K(CONTROL),
            b"box" => K(OPERATOR),
            b"become" => K(INVALID),
            b"bool" => Primitive,
            _ => Other,
        },
        b'c' => match w {
            b"const" => K(STORAGE),
            b"crate" => K(KEYWORD),
            b"continue" => K(CONTROL),
            b"char" => Primitive,
            _ => Other,
        },
        b'd' => match w {
            b"do" => K(INVALID),
            _ => Other,
        },
        b'e' => match w {
            b"else" => K(CONTROL),
            b"enum" => K(ENUM),
            b"extern" => K(KEYWORD),
            _ => Other,
        },
        b'f' => match w {
            b"fn" => K(FN),
            b"for" => K(CONTROL),
            b"false" => K(LANGUAGE),
            b"final" => K(INVALID),
            b"f32" | b"f64" => Primitive,
            _ => Other,
        },
        b'i' => match w {
            b"if" => K(CONTROL),
            b"impl" => K(IMPL),
            b"in" => K(OPERATOR),
            b"i8" | b"i16" | b"i32" | b"i64" | b"i128" | b"isize" => Primitive,
            _ => Other,
        },
        b'l' => match w {
            b"let" => K(STORAGE),
            b"loop" => K(CONTROL),
            _ => Other,
        },
        b'm' => match w {
            b"mod" => K(MODULE),
            b"mut" | b"move" => K(MODIFIER),
            b"match" => K(CONTROL),
            b"macro" => K(INVALID),
            _ => Other,
        },
        b'o' => match w {
            b"offsetof" | b"override" => K(INVALID),
            _ => Other,
        },
        b'p' => match w {
            b"pub" => K(MODIFIER),
            b"priv" | b"proc" | b"pure" => K(INVALID),
            _ => Other,
        },
        b'r' => match w {
            b"ref" => K(MODIFIER),
            b"return" => K(CONTROL),
            _ => Other,
        },
        b's' => match w {
            b"static" => K(STORAGE),
            b"struct" => K(STRUCT),
            b"sizeof" => K(INVALID),
            b"str" => Primitive,
            _ => Other,
        },
        b't' => match w {
            b"type" => K(TYPE),
            b"trait" => K(TRAIT),
            b"true" => K(LANGUAGE),
            b"typeof" => K(INVALID),
            _ => Other,
        },
        b'u' => match w {
            b"use" => K(KEYWORD),
            b"unsafe" => K(MODIFIER),
            b"unsized" => K(INVALID),
            b"u8" | b"u16" | b"u32" | b"u64" | b"u128" | b"usize" => Primitive,
            _ => Other,
        },
        b'v' => match w {
            b"virtual" => K(INVALID),
            _ => Other,
        },
        b'w' => match w {
            b"where" => K(KEYWORD),
            b"while" => K(CONTROL),
            _ => Other,
        },
        b'y' => match w {
            b"yield" => K(INVALID),
            _ => Other,
        },
        b'A' => match w {
            b"AsRef" | b"AsMut" => Support,
            _ => Other,
        },
        b'B' => match w {
            b"Box" => Support,
            _ => Other,
        },
        b'C' => match w {
            b"Copy" | b"Clone" => Support,
            _ => Other,
        },
        b'D' => match w {
            b"Drop" | b"Default" | b"DoubleEndedIterator" => Support,
            _ => Other,
        },
        b'E' => match w {
            b"Eq" | b"Extend" | b"ExactSizeIterator" | b"Err" => Support,
            _ => Other,
        },
        b'F' => match w {
            b"Fn" | b"FnMut" | b"FnOnce" | b"From" => Support,
            _ => Other,
        },
        b'I' => match w {
            b"Into" | b"Iterator" | b"IntoIterator" => Support,
            _ => Other,
        },
        b'N' => match w {
            b"None" => Support,
            _ => Other,
        },
        b'O' => match w {
            b"Ord" | b"Option" | b"Ok" => Support,
            _ => Other,
        },
        b'P' => match w {
            b"PartialEq" | b"PartialOrd" => Support,
            _ => Other,
        },
        b'R' => match w {
            b"Result" => Support,
            _ => Other,
        },
        b'S' => match w {
            b"Send" | b"Sized" | b"Sync" | b"Some" | b"SliceConcatExt" | b"String" => Support,
            b"Self" => Primitive,
            _ => Other,
        },
        b'T' => match w {
            b"ToOwned" | b"ToString" => Support,
            _ => Other,
        },
        b'V' => match w {
            b"Vec" => Support,
            _ => Other,
        },
        _ => Other,
    }
}

fn is_support(w: &[u8]) -> bool {
    lookup(w) == Name::Support
}

fn is_primitive(w: &[u8]) -> bool {
    lookup(w) == Name::Primitive
}

fn is_int_suffix(w: &[u8]) -> bool {
    matches!(
        w,
        b"i8"
            | b"i16"
            | b"i32"
            | b"i64"
            | b"i128"
            | b"isize"
            | b"u8"
            | b"u16"
            | b"u32"
            | b"u64"
            | b"u128"
            | b"usize"
    )
}

/// The bytes of the UTF-8 character that starts with `c`.
fn char_len(c: u8) -> usize {
    match c {
        0xf0.. => 4,
        0xe0.. => 3,
        0xc0.. => 2,
        _ => 1,
    }
}

fn digits_end(b: &[u8], mut p: usize, digit: impl Fn(u8) -> bool) -> usize {
    while p < b.len() && (digit(b[p]) || b[p] == b'_') {
        p += 1;
    }
    p
}

/// A character escape at `p` (a `\`): `\\(x\h{2}|n|r|t|0|"|'|\\)`, and with `unicode`
/// `|u\{\h{1,6}\}`. Where it ends.
fn escape(b: &[u8], p: usize, unicode: bool) -> Option<usize> {
    match *b.get(p + 1)? {
        b'n' | b'r' | b't' | b'0' | b'"' | b'\'' | b'\\' => Some(p + 2),
        b'x' if b.get(p + 2..p + 4)?.iter().all(u8::is_ascii_hexdigit) => Some(p + 4),
        b'u' if unicode && b.get(p + 2) == Some(&b'{') => {
            let n = b[p + 3..]
                .iter()
                .take_while(|c| c.is_ascii_hexdigit())
                .count();
            ((1..=6).contains(&n) && b.get(p + 3 + n) == Some(&b'}')).then_some(p + 4 + n)
        }
        _ => None,
    }
}

/// A character literal's body at `p`, `[^'\\]` or an escape: where it ends, and whether it
/// is an escape.
fn char_body(b: &[u8], p: usize, unicode: bool) -> Option<(usize, bool)> {
    match *b.get(p)? {
        b'\\' => escape(b, p, unicode).map(|e| (e, true)),
        b'\'' => None,
        c => Some((p + char_len(c), false)),
    }
}

/// bat's format placeholder at `p`, a `{`: where it ends.
///
/// `\{ (\d+|name)? (: (.?[<>^])? [+-]? \#? 0? (\d\$?)? (\.(\d\$?|\*)?)? (\?|name)? )? \}`:
/// a width or a precision of one digit only.
fn placeholder(b: &[u8], p: usize) -> Option<usize> {
    let mut i = p + 1;
    if b.get(i).is_some_and(u8::is_ascii_digit) {
        i += b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
    } else if let Some(e) = ident_end(b, i) {
        i = e;
    }
    match b.get(i) {
        Some(b'}') => Some(i + 1),
        Some(b':') => spec(b, i + 1),
        _ => None,
    }
}

/// A placeholder's spec at `i`, after its `:`, and its `}`: where they end. Each optional part
/// is tried as the regex tries it, greedy first.
fn spec(b: &[u8], i: usize) -> Option<usize> {
    let at = |j: usize| b.get(j).copied();
    let align = |c: Option<u8>| matches!(c, Some(b'<' | b'>' | b'^'));
    let mut fills = [None; 3];
    if let Some(c) = at(i) {
        let n = char_len(c);
        if align(at(i + n)) {
            fills[0] = Some(i + n + 1);
        }
    }
    if align(at(i)) {
        fills[1] = Some(i + 1);
    }
    fills[2] = Some(i);
    for j in fills.into_iter().flatten() {
        let mut j = j;
        if matches!(at(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if at(j) == Some(b'#') {
            j += 1;
        }
        let zeros = [(at(j) == Some(b'0')).then_some(j + 1), Some(j)];
        for z in zeros.into_iter().flatten() {
            let digit = at(z).is_some_and(|c| c.is_ascii_digit());
            let widths = [
                (digit && at(z + 1) == Some(b'$')).then_some(z + 2),
                digit.then_some(z + 1),
                Some(z),
            ];
            for w in widths.into_iter().flatten() {
                let dot = at(w) == Some(b'.');
                let digit = dot && at(w + 1).is_some_and(|c| c.is_ascii_digit());
                let precisions = [
                    (digit && at(w + 2) == Some(b'$')).then_some(w + 3),
                    digit.then_some(w + 2),
                    (dot && at(w + 1) == Some(b'*')).then_some(w + 2),
                    dot.then_some(w + 1),
                    Some(w),
                ];
                for r in precisions.into_iter().flatten() {
                    let types = [
                        (at(r) == Some(b'?')).then_some(r + 1),
                        ident_end(b, r),
                        Some(r),
                    ];
                    for t in types.into_iter().flatten() {
                        if at(t) == Some(b'}') {
                            return Some(t + 1);
                        }
                    }
                }
            }
        }
    }
    None
}

/// `\s+for\s+` at `j`, the last `\s+` matching the line's `\n`.
fn for_at(b: &[u8], j: usize) -> bool {
    if !b.get(j).is_some_and(|&c| is_space(c)) {
        return false;
    }
    let k = skip_spaces(b, j);
    b[k..].starts_with(b"for") && b.get(k + 3).is_none_or(|&c| is_space(c))
}

/// impl-for's look-ahead at `p`: `(?=\s*(?:::|name|\$|<)+(<.*?>)?\s+for\s+)`, whether the impl
/// names a trait before `for`.
fn impl_for_ahead(b: &[u8], p: usize) -> bool {
    let mut i = skip_spaces(b, p);
    let (mut tokens, mut angle) = (0, None);
    loop {
        if b[i..].starts_with(b"::") {
            i += 2;
        } else if let Some(e) = ident_end(b, i) {
            i = e;
        } else if b.get(i) == Some(&b'$') {
            i += 1;
        } else if b.get(i) == Some(&b'<') {
            if tokens > 0 && angle.is_none() {
                angle = Some(i);
            }
            i += 1;
        } else {
            break;
        }
        tokens += 1;
    }
    if tokens == 0 {
        return false;
    }
    if for_at(b, i) {
        return true;
    }
    // a prefix of the tokens, then `<` to any `>` the rest follows
    angle.is_some_and(|a| (a + 1..b.len()).any(|j| b[j] == b'>' && for_at(b, j + 1)))
}

/// The next byte at `from` or after that `stop` picks, or the line's end.
fn next_of(b: &[u8], from: usize, stop: impl Fn(u8) -> bool) -> usize {
    b[from.min(b.len())..]
        .iter()
        .position(|&c| stop(c))
        .map_or(b.len(), |i| from + i)
}

/// The contexts the prototype (`$name`) reaches: all but strings and macro matchers.
fn proto(c: u8) -> bool {
    !matches!(
        c,
        STRING
            | FMT_STRING
            | BYTE_STRING
            | RAW_STRING
            | FMT_RAW
            | RAW_BYTES
            | MATCHER
            | MATCHER_GROUP
    )
}

/// What text no pattern matches is in a context.
fn text_of(c: u8) -> u16 {
    match c {
        STRING | FMT_STRING | BYTE_STRING => STR,
        RAW_STRING | FMT_RAW | RAW_BYTES => RAW,
        BLOCK_COMMENT => BLOCK_TEXT,
        DOC_COMMENT => DOC_BLOCK,
        _ => PLAIN,
    }
}

/// `(:)\s*(\()` at `p`: where its `(` is.
fn colon_paren(b: &[u8], p: usize) -> Option<usize> {
    let s = skip_spaces(b, p + 1);
    (b[p] == b':' && b.get(s) == Some(&b'(')).then_some(s)
}

impl Scan<'_, '_> {
    fn top(&self) -> u8 {
        self.stack[self.depth - 1]
    }

    fn kind(&self, leaf: u16) -> u16 {
        if self.annot > 0 { NLEAVES + leaf } else { leaf }
    }

    /// Bytes `s..e` are `leaf`, after the text before them.
    fn put(&mut self, s: usize, e: usize, leaf: u16) {
        self.fill(s);
        let kind = self.kind(leaf);
        if kind != 0 {
            self.out.push_merged(s, e, kind);
        }
        self.at = e;
    }

    /// The text up to `p` is the context's.
    fn fill(&mut self, p: usize) {
        if p > self.at {
            if self.text != 0 {
                self.out.push_merged(self.at, p, self.text);
            }
            self.at = p;
        }
    }

    fn refresh(&mut self) {
        self.text = self.kind(text_of(self.top()));
    }

    /// Count the open attributes and tuple structs' fields.
    fn count(&mut self) {
        let open = &self.stack[..self.depth];
        self.annot = open.iter().filter(|&&c| c == ATTRIBUTE).count();
        self.tuple = open.iter().filter(|&&c| c == TUPLE_FIELDS).count();
        self.refresh();
    }

    fn enter(&mut self, c: u8) {
        self.annot += usize::from(c == ATTRIBUTE);
        self.tuple += usize::from(c == TUPLE_FIELDS);
    }

    fn leave(&mut self, c: u8) {
        self.annot -= usize::from(c == ATTRIBUTE);
        self.tuple -= usize::from(c == TUPLE_FIELDS);
    }

    fn push(&mut self, at: usize, c: u8) {
        self.fill(at);
        if self.depth == MAX {
            self.stack.copy_within(2.., 1);
            self.depth -= 1;
            self.exact = false;
            self.count();
        }
        self.stack[self.depth] = c;
        self.depth += 1;
        self.enter(c);
        self.refresh();
    }

    /// Push `after-operator` or `try-closure` at `at`, unless the line's next byte that isn't a
    /// space shows it would pop there at once, matching nothing: anything but `|` or a `$name`,
    /// and for `after-operator` a `<`. At the line's end it stays, for the next line.
    fn push_ahead(&mut self, at: usize, c: u8) {
        let q = skip_spaces(self.b, at);
        match self.b.get(q) {
            None | Some(b'|' | b'$') => {}
            Some(b'<') if c == AFTER_OP => {}
            Some(_) => return,
        }
        self.push(at, c);
    }

    fn pop(&mut self, at: usize) {
        self.fill(at);
        if self.depth > 1 {
            self.depth -= 1;
            self.leave(self.stack[self.depth]);
        }
        self.refresh();
    }

    fn set(&mut self, at: usize, c: u8) {
        self.fill(at);
        let top = self.depth - 1;
        self.leave(self.stack[top]);
        self.stack[top] = c;
        self.enter(c);
        self.refresh();
    }

    fn run(&mut self) {
        let mut p = 0;
        loop {
            if let Some(q) = self.at(p) {
                p = q;
            } else if p < self.b.len() {
                p = self.advance(p);
            } else {
                return;
            }
        }
    }

    /// Where the next pattern can start, after none did at `p`.
    fn advance(&self, p: usize) -> usize {
        let b = self.b;
        match self.top() {
            STRING => next_of(b, p + 1, |c| c == b'"' || c == b'\\'),
            BYTE_STRING => next_of(b, p + 1, |c| c == b'"' || c == b'\\'),
            FMT_STRING => next_of(b, p + 1, |c| matches!(c, b'"' | b'\\' | b'{' | b'}')),
            RAW_STRING | RAW_BYTES => next_of(b, p + 1, |c| c == b'"'),
            FMT_RAW => next_of(b, p + 1, |c| matches!(c, b'"' | b'{' | b'}')),
            BLOCK_COMMENT | DOC_COMMENT => next_of(b, p + 1, |c| matches!(c, b'*' | b'/' | b'$')),
            _ => {
                let c = b[p];
                if is_space(c) {
                    skip_spaces(b, p)
                } else if is_word(c) {
                    word_end(b, p)
                } else {
                    p + 1
                }
            }
        }
    }

    /// The first pattern of the top context that matches at `p`, applied: where the scan goes
    /// on, or `None` when none does.
    fn at(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let top = self.top();
        if p == b.len() {
            return self.eol(top);
        }
        let c = b[p];
        // a tuple struct's fields end at the first `)`, in any context opened in them
        if self.tuple > 0 && c == b')' && top < STRING {
            if top == TUPLE_FIELDS {
                self.put(p, p + 1, GROUP_END);
                self.pop(p + 1);
                return Some(p + 1);
            }
            self.pop(p);
            return Some(p);
        }
        if c == b'$'
            && proto(top)
            && let Some(e) = ident_end(b, p + 1)
        {
            let leaf = match top {
                BLOCK_COMMENT => BLOCK_TEXT + VAR,
                DOC_COMMENT => DOC_BLOCK + VAR,
                _ => VARIABLE,
            };
            self.put(p, e, leaf);
            return Some(e);
        }
        match top {
            MAIN | BLOCK | GROUP | BRACKET | MACRO_BRACKET | CLOSURE_BODY => {
                self.statements(top, p, c)
            }
            HEAD | MOD_HEAD => self.head(top, p, c),
            AFTER_OP | TRY_CLOSURE => {
                let q = skip_spaces(b, p);
                match b.get(q) {
                    None => None,
                    Some(b'|') => {
                        self.set(p, CLOSURE);
                        Some(q)
                    }
                    Some(b'<') if top == AFTER_OP => {
                        self.set(q, GENERIC);
                        Some(q)
                    }
                    // the prototype matches there first
                    Some(b'$') if q > p && ident_end(b, q + 1).is_some() => Some(q),
                    Some(_) => {
                        self.pop(q);
                        Some(q)
                    }
                }
            }
            CLOSURE => (c == b'|').then(|| {
                self.set(p, CLOSURE_RETURN);
                self.push(p, CLOSURE_PARAMS);
                self.put(p, p + 1, PARAMS_BEGIN);
                p + 1
            }),
            CLOSURE_PARAMS => self.closure_params(p, c),
            PARAM_TUPLE => match c {
                b')' => Some(self.close(p, META_GROUP_END)),
                b',' => Some(self.token(p, 1, SEPARATOR)),
                _ => {
                    let e = ident_end(b, p)?;
                    self.put(p, e, PARAMETER);
                    Some(e)
                }
            },
            PARAM_TUPLE_TYPE => match c {
                b')' => Some(self.close(p, META_GROUP_END)),
                b',' => Some(self.token(p, 1, SEPARATOR)),
                _ => self.tai(p),
            },
            CLOSURE_PARAM_TYPE => {
                if c == b',' || c == b'|' {
                    self.pop(p);
                    return Some(p);
                }
                self.tai(p)
            }
            CLOSURE_RETURN => match c {
                b'-' if b.get(p + 1) == Some(&b'>') => {
                    self.put(p, p + 2, SEPARATOR);
                    self.push(p + 2, RETURN_TYPE);
                    Some(p + 2)
                }
                b'{' => Some(self.open_block(p, true)),
                _ if is_space(c) => None,
                _ => {
                    self.set(p, CLOSURE_BODY);
                    Some(p)
                }
            },
            RETURN_TYPE => self.return_type(p, c),
            FN_GENERIC => {
                if let Some(e) = self.comment(p) {
                    return Some(e);
                }
                match c {
                    b'<' => {
                        self.push(p, GENERIC);
                        Some(p)
                    }
                    b'(' => {
                        self.set(p, FN_PARAMS);
                        self.put(p, p + 1, PARAMS_BEGIN);
                        Some(p + 1)
                    }
                    _ if is_space(c) => None,
                    _ => {
                        self.pop(p);
                        Some(p)
                    }
                }
            }
            FN_PARAMS => self.fn_params(p, c),
            FN_RETURN => {
                if let Some(e) = self.comment(p) {
                    return Some(e);
                }
                match c {
                    b'{' => Some(self.open_block(p, false)),
                    b'-' if b.get(p + 1) == Some(&b'>') => {
                        self.put(p, p + 2, SEPARATOR);
                        self.push(p + 2, RETURN_TYPE);
                        Some(p + 2)
                    }
                    _ if is_space(c) => None,
                    _ if self.is_word_at(p, b"where") => {
                        self.set(p, FN_WHERE);
                        Some(p)
                    }
                    _ => {
                        self.pop(p);
                        Some(p)
                    }
                }
            }
            FN_WHERE | IMPL_WHERE => {
                if let Some(e) = self.comment(p) {
                    return Some(e);
                }
                if c == b'{' {
                    return Some(self.open_block(p, top == IMPL_WHERE));
                }
                if self.is_word_at(p, b"for") || self.is_word_at(p, b"where") {
                    return Some(self.token(p, word_end(b, p) - p, KEYWORD));
                }
                if let Some(e) = self.tai(p) {
                    return Some(e);
                }
                match c {
                    b':' => Some(self.token(p, 1, SEPARATOR)),
                    b';' if top == FN_WHERE => {
                        self.pop(p);
                        Some(p)
                    }
                    _ => None,
                }
            }
            IMPL_DEF => {
                if let Some(e) = self.comment(p) {
                    return Some(e);
                }
                match c {
                    b'<' => {
                        self.set(p, IMPL_FOR);
                        self.push(p, IMPL_GENERIC);
                        Some(p)
                    }
                    _ if is_space(c) => None,
                    _ => {
                        self.set(p, IMPL_FOR);
                        Some(p)
                    }
                }
            }
            IMPL_GENERIC | STRUCT_GENERIC => {
                if c == b'<' {
                    self.push(p, GENERIC);
                } else if top == IMPL_GENERIC {
                    self.pop(p);
                } else {
                    self.set(p, STRUCT_BODY);
                }
                Some(p)
            }
            IMPL_FOR => {
                let next = if impl_for_ahead(b, p) {
                    IMPL_TRAIT
                } else {
                    IMPL_IDENT
                };
                self.set(p, next);
                Some(p)
            }
            IMPL_TRAIT => {
                if let Some(e) = self.comment(p) {
                    return Some(e);
                }
                if self.is_word_at(p, b"for") {
                    self.put(p, p + 3, KEYWORD);
                    self.set(p + 3, IMPL_IDENT);
                    return Some(p + 3);
                }
                self.tai(p)
            }
            IMPL_IDENT => {
                if let Some(e) = self.comment(p) {
                    return Some(e);
                }
                match c {
                    b'{' => Some(self.open_block(p, true)),
                    b'<' => {
                        self.push(p, GENERIC);
                        Some(p)
                    }
                    b'&' => Some(self.token(p, 1, OPERATOR)),
                    b'\'' => self.lifetime(p),
                    _ if self.is_word_at(p, b"where") => {
                        self.set(p, IMPL_WHERE);
                        Some(p)
                    }
                    _ => {
                        let e = ident_end(b, p)?;
                        self.put(p, e, IMPL_NAME);
                        Some(e)
                    }
                }
            }
            STRUCT_ID => {
                let e = ident_end(b, p)?;
                self.put(p, e, STRUCT_NAME);
                let next = if b.get(e) == Some(&b'<') {
                    STRUCT_GENERIC
                } else {
                    STRUCT_BODY
                };
                self.set(e, next);
                Some(e)
            }
            STRUCT_BODY => {
                if let Some(e) = self.comment(p) {
                    return Some(e);
                }
                match c {
                    b'(' => {
                        self.set(p, TUPLE_FIELDS);
                        self.put(p, p + 1, META_GROUP_BEGIN);
                        Some(p + 1)
                    }
                    b'{' => {
                        self.set(p, STRUCT_FIELDS);
                        self.put(p, p + 1, META_BLOCK_BEGIN);
                        Some(p + 1)
                    }
                    b';' => {
                        self.pop(p);
                        Some(p)
                    }
                    _ => None,
                }
            }
            STRUCT_FIELDS => self.struct_fields(p, c),
            FIELD_TYPE => match c {
                b',' => {
                    self.pop(p + 1);
                    Some(p + 1)
                }
                b'}' => {
                    self.pop(p);
                    Some(p)
                }
                _ => {
                    if let Some(e) = self.comment(p) {
                        return Some(e);
                    }
                    if c == b':' {
                        return Some(self.token(p, 1, SEPARATOR_TYPE));
                    }
                    self.tai(p)
                }
            },
            TUPLE_FIELDS => self.comment(p).or_else(|| self.tai(p)),
            GENERIC => {
                if let Some(e) = self.comment(p) {
                    return Some(e);
                }
                match c {
                    b'-' if b.get(p + 1) == Some(&b'>') => {
                        self.put(p, p + 2, SEPARATOR_GENERIC);
                        self.push(p + 2, GENERIC_ARGS);
                        Some(p + 2)
                    }
                    b'>' => Some(self.close(p, GENERIC_END)),
                    b'<' => {
                        self.push(p, GENERIC_ARGS);
                        self.put(p, p + 1, GENERIC_BEGIN);
                        Some(p + 1)
                    }
                    _ if is_space(c) => None,
                    _ => Some(self.close_n(p, char_len(c), INVALID)),
                }
            }
            GENERIC_ARGS | GENERIC_NESTED | GENERIC_ARRAY => self.generic_args(top, p, c),
            GENERIC_LEN => {
                if c == b']' {
                    return Some(self.close(p, GROUP_END));
                }
                if let Some(e) = self.const_expr(p, c) {
                    return Some(e);
                }
                if is_space(c) {
                    return None;
                }
                self.pop(p);
                Some(p)
            }
            CONST_GROUP => {
                if c == b')' {
                    return Some(self.close(p, GROUP_END));
                }
                self.const_expr(p, c)
            }
            GENERIC_EXTERN => {
                if let Some(e) = self.string(p, false) {
                    return Some(e);
                }
                if is_space(c) {
                    return None;
                }
                self.pop(p);
                Some(p)
            }
            FN_TYPE => {
                if c == b')' {
                    self.put(p, p + 1, META_GROUP_END);
                    self.set(p + 1, FN_TYPE_RETURN);
                    return Some(p + 1);
                }
                self.tai(p)
            }
            FN_TYPE_RETURN => match c {
                b'-' if b.get(p + 1) == Some(&b'>') => {
                    self.put(p, p + 2, SEPARATOR);
                    self.push(p + 2, RETURN_TYPE);
                    Some(p + 2)
                }
                _ if is_space(c) => None,
                _ => {
                    self.pop(p);
                    Some(p)
                }
            },
            TYPE_GROUP => {
                if c == b')' {
                    return Some(self.close(p, GROUP_END));
                }
                self.tai(p)
            }
            NO_TYPE_NAMES => Some(self.no_type_names(p, c)),
            MACRO_BLOCK => match c {
                b'{' => {
                    self.set(p, MACRO_RULES);
                    self.put(p, p + 1, META_BLOCK_BEGIN);
                    Some(p + 1)
                }
                b'[' | b'(' => {
                    let next = if c == b'[' {
                        MACRO_RULES_BRACKET
                    } else {
                        MACRO_RULES_PAREN
                    };
                    self.set(p, next);
                    self.put(p, p + 1, META_GROUP_BEGIN);
                    Some(p + 1)
                }
                _ if is_space(c) => None,
                _ => {
                    self.pop(p);
                    Some(p)
                }
            },
            MACRO_RULES | MACRO_RULES_BRACKET | MACRO_RULES_PAREN => {
                let close = match top {
                    MACRO_RULES => b'}',
                    MACRO_RULES_BRACKET => b']',
                    _ => b')',
                };
                if c == close {
                    let leaf = if c == b'}' { BLOCK_END } else { META_GROUP_END };
                    return Some(self.close(p, leaf));
                }
                if let Some(e) = self.comment(p) {
                    return Some(e);
                }
                (c == b'(').then(|| {
                    for c in [MACRO_TERM, MACRO_BODY, MATCH_OP, MATCHER] {
                        self.push(p, c);
                    }
                    self.put(p, p + 1, GROUP_BEGIN);
                    p + 1
                })
            }
            MATCHER => match c {
                b')' => Some(self.close(p, GROUP_END)),
                b'$' if b.get(p + 1) == Some(&b'(') => {
                    self.push(p, MATCHER_GROUP);
                    self.put(p, p + 1, REPEAT_BEGIN);
                    self.put(p + 1, p + 2, GROUP_BEGIN);
                    Some(p + 2)
                }
                b'$' => self.metavariable(p),
                _ => None,
            },
            MATCHER_GROUP => match c {
                b')' => {
                    self.put(p, p + 1, GROUP_END);
                    // `(?:\s*[^*+]?\s*([*+]))?`
                    let j = skip_spaces(b, p + 1);
                    let rep = |k: usize| matches!(b.get(k), Some(b'*' | b'+'));
                    let op = if rep(j) {
                        Some(j)
                    } else if j < b.len() {
                        let k = skip_spaces(b, j + char_len(b[j]));
                        rep(k).then_some(k)
                    } else {
                        None
                    };
                    let end = match op {
                        Some(k) => self.token(k, 1, OPERATOR),
                        None => p + 1,
                    };
                    self.pop(end);
                    Some(end)
                }
                b'$' => self.metavariable(p),
                _ => None,
            },
            MATCH_OP => {
                if c == b'=' && b.get(p + 1) == Some(&b'>') {
                    return Some(self.close_n(p, 2, OPERATOR));
                }
                if is_space(c) {
                    return None;
                }
                self.pop(p);
                Some(p)
            }
            MACRO_BODY => match c {
                b'{' => {
                    self.set(p, BLOCK);
                    self.put(p, p + 1, BLOCK_BEGIN);
                    Some(p + 1)
                }
                b'(' | b'[' => {
                    self.set(p, if c == b'(' { GROUP } else { MACRO_BRACKET });
                    self.put(p, p + 1, GROUP_BEGIN);
                    Some(p + 1)
                }
                _ if is_space(c) => None,
                _ => {
                    self.pop(p);
                    Some(p)
                }
            },
            MACRO_TERM => match c {
                b';' | b',' => Some(self.token(p, 1, MACRO_TERMINATOR)),
                _ if is_space(c) => None,
                _ => {
                    self.pop(p);
                    Some(p)
                }
            },
            ATTRIBUTE => match c {
                b'=' => Some(self.token(p, 1, OPERATOR)),
                b']' => Some(self.close(p, GROUP_END)),
                _ => {
                    if let Some(e) = ident_end(b, p) {
                        self.put(p, e, ANNOTATION);
                        if b.get(e) == Some(&b'(') {
                            self.push(e, ATTR_PARAMS);
                            self.put(e, e + 1, ANNOTATION_BEGIN);
                            return Some(e + 1);
                        }
                        return Some(e);
                    }
                    self.string(p, false)
                }
            },
            ATTR_PARAMS | ATTR_CALL => match c {
                b')' => {
                    let leaf = if top == ATTR_PARAMS {
                        ANNOTATION_END
                    } else {
                        CALL_END
                    };
                    Some(self.close(p, leaf))
                }
                b',' => Some(self.token(p, 1, SEPARATOR)),
                b'=' => Some(self.token(p, 1, OPERATOR)),
                _ => {
                    if let Some(e) = ident_end(b, p)
                        && b.get(e) == Some(&b'(')
                    {
                        self.put(p, e, CALL);
                        self.push(e, ATTR_CALL);
                        self.put(e, e + 1, CALL_BEGIN);
                        return Some(e + 1);
                    }
                    self.string(p, false)
                }
            },
            FMT_FIRST => {
                if let Some(e) = self.comment(p) {
                    return Some(e);
                }
                if c == b'"' {
                    self.push(p, FMT_STRING);
                    self.put(p, p + 1, STR_BEGIN);
                    return Some(p + 1);
                }
                if c == b'r' {
                    let h = b[p + 1..].iter().take_while(|&&c| c == b'#').count();
                    if b.get(p + 1 + h) == Some(&b'"') {
                        self.push(p, FMT_RAW);
                        self.hashes = h;
                        self.put(p, p + 1, RAW_PREFIX);
                        self.put(p + 1, p + 1 + h, RAW_BEGIN);
                        self.put(p + 1 + h, p + 2 + h, RAW);
                        return Some(p + 2 + h);
                    }
                }
                if is_space(c) {
                    return None;
                }
                self.set(p, GROUP);
                Some(p)
            }
            WRITE_FIRST => {
                if let Some(e) = self.comment(p) {
                    return Some(e);
                }
                (c == b',').then(|| {
                    self.set(p + 1, FMT_FIRST);
                    p + 1
                })
            }
            STRING | FMT_STRING | BYTE_STRING => {
                let fmt = top == FMT_STRING;
                match c {
                    b'"' => Some(self.close(p, STR_END)),
                    b'\\' => {
                        if let Some(e) = escape(b, p, top != BYTE_STRING) {
                            return Some(self.token(p, e - p, STR_ESCAPE));
                        }
                        (top != BYTE_STRING && p + 1 == b.len())
                            .then(|| self.token(p, 1, STR_CONTINUATION))
                    }
                    b'{' | b'}' if fmt => self.format_escape(p, c, STR_ESCAPE, STR_PLACEHOLDER),
                    _ => None,
                }
            }
            RAW_STRING | FMT_RAW | RAW_BYTES => {
                let h = self.hashes;
                if c == b'"' && b.len() > p + h && b[p + 1..p + 1 + h].iter().all(|&c| c == b'#') {
                    return Some(self.close_n(p, 1 + h, RAW_END));
                }
                if top == FMT_RAW && (c == b'{' || c == b'}') {
                    return self.format_escape(p, c, RAW_ESCAPE, RAW_PLACEHOLDER);
                }
                None
            }
            BLOCK_COMMENT | DOC_COMMENT => self.in_comment(top, p, c),
            _ => None,
        }
    }

    /// At the line's end, after the last match: the patterns that match the `\n`.
    fn eol(&mut self, top: u8) -> Option<usize> {
        let len = self.b.len();
        match top {
            // `(?=[};)\]\n])`, and the empty match
            CLOSURE_BODY | CLOSURE_PARAMS | NO_TYPE_NAMES | IMPL_GENERIC => self.pop(len),
            IMPL_FOR => self.set(len, IMPL_IDENT),
            STRUCT_GENERIC => self.set(len, STRUCT_BODY),
            _ => return None,
        }
        Some(len)
    }

    /// Whether `w` is the whole word at `p`.
    fn is_word_at(&self, p: usize, w: &[u8]) -> bool {
        self.b[p..].starts_with(w) && ends_word(self.b, p + w.len()) && starts_word(self.b, p)
    }

    /// `n` bytes at `p` are `leaf`: where they end.
    fn token(&mut self, p: usize, n: usize, leaf: u16) -> usize {
        self.put(p, p + n, leaf);
        p + n
    }

    /// A closer at `p`, `leaf`, that ends the context.
    fn close(&mut self, p: usize, leaf: u16) -> usize {
        self.close_n(p, 1, leaf)
    }

    fn close_n(&mut self, p: usize, n: usize, leaf: u16) -> usize {
        self.put(p, p + n, leaf);
        self.pop(p + n);
        p + n
    }

    /// A `{` at `p` that opens a block in place of the context; `try_closure` after it, as
    /// `block` and `statements-block` have and a function's body hasn't.
    fn open_block(&mut self, p: usize, try_closure: bool) -> usize {
        self.set(p, BLOCK);
        self.put(p, p + 1, BLOCK_BEGIN);
        if try_closure {
            self.push_ahead(p + 1, TRY_CLOSURE);
        }
        p + 1
    }

    /// `{{`, `}}` or a placeholder at `p`.
    fn format_escape(&mut self, p: usize, c: u8, escape: u16, place: u16) -> Option<usize> {
        if self.b.get(p + 1) == Some(&c) {
            return Some(self.token(p, 2, escape));
        }
        let e = (c == b'{').then(|| placeholder(self.b, p)).flatten()?;
        Some(self.token(p, e - p, place))
    }

    /// A comment at `p`, `comments`: where its first match ends.
    fn comment(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if b[p] != b'/' {
            return None;
        }
        match b.get(p + 1) {
            Some(b'*') => Some(self.open_comment(p)),
            Some(b'/') => {
                let doc = matches!(b.get(p + 2), Some(b'!' | b'/'));
                let (text, n) = if doc { (DOC, 3) } else { (COMMENT, 2) };
                self.put(p, p + n, text + MARK);
                let mut from = p + n;
                let mut at = from;
                while let Some(d) = b[at..].iter().position(|&c| c == b'$').map(|i| at + i) {
                    if let Some(e) = ident_end(b, d + 1) {
                        self.put(from, d, text);
                        self.put(d, e, text + VAR);
                        from = e;
                        at = e;
                    } else {
                        at = d + 1;
                    }
                }
                self.put(from, b.len(), text);
                Some(b.len())
            }
            _ => None,
        }
    }

    /// `/*` at `p`: a block comment, a documentation one for `/**` or `/*!` and a character
    /// that isn't `*` or `/` (bat's `/\*[!\*][^\*/]`, its mark taking that character).
    fn open_comment(&mut self, p: usize) -> usize {
        let b = self.b;
        let doc = matches!(b.get(p + 2), Some(b'!' | b'*'))
            && b.get(p + 3).is_none_or(|&d| d != b'*' && d != b'/');
        if doc {
            let e = b.get(p + 3).map_or(b.len(), |&d| p + 3 + char_len(d));
            self.push(p, DOC_COMMENT);
            self.token(p, e - p, DOC_BLOCK + MARK)
        } else {
            self.push(p, BLOCK_COMMENT);
            self.token(p, 2, BLOCK_TEXT + MARK)
        }
    }

    fn in_comment(&mut self, top: u8, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        let text = if top == DOC_COMMENT {
            DOC_BLOCK
        } else {
            BLOCK_TEXT
        };
        if c == b'*' && b.get(p + 1) == Some(&b'/') {
            return Some(self.close_n(p, 2, text + MARK));
        }
        // `^\s*(\*)(?!/)`, a documentation comment's line's leading `*`
        if top == DOC_COMMENT && p == 0 {
            let q = skip_spaces(b, 0);
            if b.get(q) == Some(&b'*') && b.get(q + 1) != Some(&b'/') {
                return Some(self.token(q, 1, text + MARK));
            }
        }
        (c == b'/' && b.get(p + 1) == Some(&b'*')).then(|| self.open_comment(p))
    }

    /// `statements`, at `p` in one of the contexts made of them: the context's closers first.
    fn statements(&mut self, top: u8, p: usize, c: u8) -> Option<usize> {
        match (top, c) {
            (BLOCK, b'}') => return Some(self.close(p, BLOCK_END)),
            (GROUP, b')') => return Some(self.close(p, META_GROUP_END)),
            (BRACKET | MACRO_BRACKET, b']') => return Some(self.close(p, GROUP_END)),
            (BRACKET, b';') => return Some(self.token(p, 1, SEPARATOR)),
            (CLOSURE_BODY, b'}' | b';' | b')' | b']') => {
                self.pop(p);
                return Some(p);
            }
            _ => {}
        }
        self.statement(p, c)
    }

    /// A header's `statements-block`: comments and the body's braces.
    fn head(&mut self, top: u8, p: usize, c: u8) -> Option<usize> {
        if top == MOD_HEAD && c == b';' {
            self.put(p, p + 1, TERMINATOR);
            self.set(p + 1, AFTER_OP);
            return Some(p + 1);
        }
        if let Some(e) = self.comment(p) {
            return Some(e);
        }
        match c {
            b'}' => Some(self.close(p, BLOCK_END)),
            b'{' => Some(self.open_block(p, true)),
            _ => None,
        }
    }

    /// One of the patterns `statements` holds, at `p`.
    fn statement(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        let next = b.get(p + 1).copied();
        Some(match c {
            b'\'' => self.quote(p),
            b'"' => {
                self.push(p, STRING);
                self.token(p, 1, STR_BEGIN)
            }
            b'/' => match self.comment(p) {
                Some(e) => e,
                None => self.op_eq(p, ARITHMETIC),
            },
            b'#' => match self.attribute(p) {
                Some(e) => e,
                None => self.token(p, 1, OPERATOR),
            },
            b'{' => {
                self.push(p, BLOCK);
                self.put(p, p + 1, BLOCK_BEGIN);
                self.push_ahead(p + 1, TRY_CLOSURE);
                p + 1
            }
            b'(' => {
                self.push(p, GROUP);
                self.put(p, p + 1, GROUP_BEGIN);
                self.push_ahead(p + 1, TRY_CLOSURE);
                p + 1
            }
            b'[' => {
                self.push(p, BRACKET);
                self.token(p, 1, GROUP_BEGIN)
            }
            b'0'..=b'9' => {
                if !starts_word(b, p) {
                    return None;
                }
                return self.number(p, true);
            }
            b':' => {
                if next == Some(b':') && ident_end(b, p + 2).is_some() {
                    self.put(p, p + 2, PATH_ACCESSOR);
                    self.push(p + 2, NO_TYPE_NAMES);
                    p + 2
                } else {
                    self.after_op(p, 1, SEPARATOR)
                }
            }
            b';' => self.after_op(p, 1, TERMINATOR),
            b',' => self.after_op(p, 1, SEPARATOR),
            b'-' if next == Some(b'>') => {
                self.put(p, p + 2, SEPARATOR);
                self.push(p + 2, RETURN_TYPE);
                p + 2
            }
            b'=' => match next {
                Some(b'>') => self.after_op(p, 2, OPERATOR),
                Some(b'=') => self.token(p, 2, COMPARISON),
                _ => self.after_op(p, 1, ASSIGNMENT),
            },
            b'*' => match self.star_keyword(p) {
                Some(e) => e,
                None => self.op_eq(p, ARITHMETIC),
            },
            b'-' | b'+' | b'%' => self.op_eq(p, ARITHMETIC),
            b'^' => self.op_eq(p, BITWISE),
            b'&' | b'|' => {
                if next == Some(b'=') {
                    self.token(p, 2, ASSIGNMENT)
                } else if next == Some(c) {
                    self.token(p, 2, LOGICAL)
                } else {
                    self.token(p, 1, BITWISE)
                }
            }
            b'!' => {
                if next == Some(b'=') {
                    self.token(p, 2, COMPARISON)
                } else {
                    self.token(p, 1, LOGICAL)
                }
            }
            b'<' | b'>' => {
                if next == Some(c) && b.get(p + 2) == Some(&b'=') {
                    self.token(p, 3, ASSIGNMENT)
                } else if next == Some(b'=') {
                    self.token(p, 2, COMPARISON)
                } else {
                    self.token(p, 1, COMPARISON)
                }
            }
            b'.' => {
                if next == Some(b'.') {
                    let n = if b.get(p + 2) == Some(&b'.') { 3 } else { 2 };
                    self.token(p, n, RANGE)
                } else {
                    self.token(p, 1, DOT)
                }
            }
            b'@' | b'~' | b'?' | b'$' => self.token(p, 1, OPERATOR),
            // no pattern starts with a space: the run is plain
            _ if is_space(c) => skip_spaces(b, p),
            _ if is_word(c) => return self.word(p),
            _ => return None,
        })
    }

    /// `n` bytes at `p` are `leaf`, and push `after-operator`.
    fn after_op(&mut self, p: usize, n: usize, leaf: u16) -> usize {
        self.put(p, p + n, leaf);
        self.push_ahead(p + n, AFTER_OP);
        p + n
    }

    /// An operator that `=` can follow, making it an assignment.
    fn op_eq(&mut self, p: usize, leaf: u16) -> usize {
        if self.b.get(p + 1) == Some(&b'=') {
            self.token(p, 2, ASSIGNMENT)
        } else {
            self.token(p, 1, leaf)
        }
    }

    /// `\*const\b` or `\*mut\b` at `p`.
    fn star_keyword(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let (n, leaf) = if b[p + 1..].starts_with(b"const") {
            (6, STORAGE)
        } else if b[p + 1..].starts_with(b"mut") {
            (4, MODIFIER)
        } else {
            return None;
        };
        ends_word(b, p + n).then(|| self.token(p, n, leaf))
    }

    /// `(#!?)(\[)` at `p`: an attribute.
    fn attribute(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let m = if b.get(p + 1) == Some(&b'!') {
            p + 2
        } else {
            p + 1
        };
        if b.get(m) != Some(&b'[') {
            return None;
        }
        self.push(p, ATTRIBUTE);
        self.put(p, m, ANNOTATION_MARK);
        Some(self.token(m, 1, GROUP_BEGIN))
    }

    /// A `'` at `p`: a label, a lifetime, a character, or an operator.
    fn quote(&mut self, p: usize) -> usize {
        let b = self.b;
        if let Some(e) = ident_end(b, p + 1) {
            let s = skip_spaces(b, e);
            if b.get(s) == Some(&b':') {
                self.put(p, e, LABEL);
                return self.token(s, 1, SEPARATOR);
            }
            if b.get(e) != Some(&b'\'') {
                return self.token(p, e - p, LIFETIME);
            }
        }
        if let Some(e) = self.character(p, p, true) {
            return e;
        }
        self.token(p, 1, OPERATOR)
    }

    /// A character literal whose `'` is at `q`, its prefix from `p`: where it ends.
    fn character(&mut self, p: usize, q: usize, unicode: bool) -> Option<usize> {
        let (e, esc) = char_body(self.b, q + 1, unicode)?;
        if self.b.get(e) != Some(&b'\'') {
            return None;
        }
        self.put(p, q, CHAR_PREFIX);
        self.put(q, q + 1, CHAR_BEGIN);
        self.put(q + 1, e, if esc { CHAR_ESCAPE } else { CHAR });
        Some(self.token(e, 1, CHAR_END))
    }

    /// `'name(?!')\b` at `p`.
    fn lifetime(&mut self, p: usize) -> Option<usize> {
        let e = ident_end(self.b, p + 1)?;
        (self.b.get(e) != Some(&b'\'')).then(|| self.token(p, e - p, LIFETIME))
    }

    /// A string at `p`, `strings`: `"`, `b"`, `br#"`, `r#"`; with `chars`, `b'` too.
    fn string(&mut self, p: usize, chars: bool) -> Option<usize> {
        let b = self.b;
        if b[p] == b'"' {
            self.push(p, STRING);
            return Some(self.token(p, 1, STR_BEGIN));
        }
        let e = word_end(b, p);
        let next = b.get(e).copied();
        match (&b[p..e], next) {
            (b"b", Some(b'"')) => {
                self.push(p, BYTE_STRING);
                self.put(p, e, STR_PREFIX);
                Some(self.token(e, 1, STR_BEGIN))
            }
            (b"br" | b"r", Some(b'#' | b'"')) => {
                let h = b[e..].iter().take_while(|&&c| c == b'#').count();
                if b.get(e + h) != Some(&b'"') {
                    return None;
                }
                let raw = e - p == 1;
                self.push(p, if raw { RAW_STRING } else { RAW_BYTES });
                self.hashes = h;
                self.put(p, e, RAW_PREFIX);
                if raw {
                    // `(r)((#*)")`: the `"` is the mark's
                    Some(self.token(e, h + 1, RAW_BEGIN))
                } else {
                    self.put(e, e + h, RAW_BEGIN);
                    Some(self.token(e + h, 1, RAW))
                }
            }
            (b"b", Some(b'\'')) if chars => self.character(p, e, false),
            _ => None,
        }
    }

    /// A word at `p`, at the top level.
    fn word(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if !starts_word(b, p) {
            return None;
        }
        let e = word_end(b, p);
        let w = &b[p..e];
        let ident = is_ident(w);
        let next = b.get(e).copied();
        let item = matches!(
            w,
            b"pub" | b"mod" | b"fn" | b"struct" | b"type" | b"trait" | b"enum" | b"impl"
        );
        // the items, in bat's order: a module, a closure's name, a function, a struct, a type,
        // a trait, an impl, an enum
        if item && let Some(q) = self.item(p, e, b"mod", MODULE, MODULE_NAME, Some(MOD_HEAD)) {
            return Some(q);
        }
        if ident && matches!(next, Some(b' ' | b'\t' | b'=')) {
            // `name\s*=\s*(?=\|)`
            let s = skip_spaces(b, e);
            if b.get(s) == Some(&b'=') {
                let t = skip_spaces(b, s + 1);
                if b.get(t) == Some(&b'|') {
                    self.put(p, e, FN_NAME);
                    self.put(s, s + 1, OPERATOR);
                    self.push(t, CLOSURE);
                    return Some(t);
                }
            }
        }
        if item && let Some(q) = self.items(p, e) {
            return Some(q);
        }
        self.word_rest(p, e, ident, next)
    }

    /// The items after a closure's name, for a word that can start one.
    fn items(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        let w = &b[p..e];
        if let Some(q) = self.item(p, e, b"fn", FN, FN_NAME, Some(FN_GENERIC)) {
            return Some(q);
        }
        if let Some((k, ke)) = self.keyword_at(p, e, b"struct") {
            // `struct\s+`, the `\n` among the spaces
            let end = if ke == b.len() {
                Some(ke)
            } else {
                is_space(b[ke]).then(|| skip_spaces(b, ke))
            };
            if let Some(end) = end {
                if k > p {
                    self.put(p, e, MODIFIER);
                }
                self.put(k, ke, STRUCT);
                self.push(end, STRUCT_ID);
                return Some(end);
            }
        }
        if let Some(q) = self.item(p, e, b"type", TYPE, TYPE_NAME, None) {
            return Some(q);
        }
        if let Some(q) = self.item(p, e, b"trait", TRAIT, TRAIT_NAME, Some(HEAD)) {
            return Some(q);
        }
        if w == b"impl" {
            self.put(p, e, IMPL);
            self.push(e, IMPL_DEF);
            return Some(e);
        }
        self.item(p, e, b"enum", ENUM, ENUM_NAME, Some(HEAD))
    }

    /// The patterns after the items, for the word `p..e`.
    fn word_rest(&mut self, p: usize, e: usize, ident: bool, next: Option<u8>) -> Option<usize> {
        let b = self.b;
        let w = &b[p..e];
        let name = lookup(w);
        if let Name::Keyword(leaf) = name {
            self.put(p, e, leaf);
            if w == b"return" {
                self.push_ahead(e, AFTER_OP);
            }
            return Some(e);
        }
        // `type`: a name before `<`, then the primitive types
        if next == Some(b'<') && ident {
            if name == Name::Support {
                self.put(p, e, SUPPORT_TYPE);
            }
            self.push(e, GENERIC);
            return Some(e);
        }
        if name == Name::Primitive {
            return Some(self.token(p, e - p, STORAGE));
        }
        if w == b"macro_rules" && next == Some(b'!') {
            let s = skip_spaces(b, e + 1);
            if s > e + 1 && matches!(b.get(s), Some(&c) if is_letter(c) || c == b'_') {
                let n = word_end(b, s);
                self.put(p, e + 1, SUPPORT_FN);
                self.put(s, n, MACRO_NAME);
                self.push(n, MACRO_BLOCK);
                return Some(n);
            }
        }
        if matches!(w, b"b" | b"br" | b"r")
            && let Some(q) = self.string(p, true)
        {
            return Some(q);
        }
        let lower = matches!(next, Some(b'(' | b'!')) && is_lower(w);
        if next == Some(b'(') && lower {
            return Some(self.token(p, e - p, SUPPORT_FN));
        }
        if next == Some(b'!') && lower {
            let s = skip_spaces(b, e + 1);
            let open = b.get(s).copied();
            if open == Some(b'(') {
                let first = match w {
                    b"format" | b"print" | b"println" | b"eprint" | b"eprintln" => FMT_FIRST,
                    b"write" | b"writeln" => WRITE_FIRST,
                    _ => MAIN,
                };
                if first != MAIN {
                    self.put(p, e + 1, SUPPORT_MACRO);
                    self.push(s, first);
                    return Some(self.token(s, 1, META_GROUP_BEGIN));
                }
            }
            if matches!(open, Some(b'(' | b'{' | b'[')) {
                return Some(self.token(p, e + 1 - p, SUPPORT_MACRO));
            }
        }
        if name == Name::Support {
            return Some(self.token(p, e - p, SUPPORT_TYPE));
        }
        // basic-identifiers
        if is_constant(w) {
            return Some(self.token(p, e - p, CONSTANT));
        }
        if is_c_type(w) {
            return Some(self.token(p, e - p, STORAGE));
        }
        if ident && b[e..].starts_with(b"::") {
            return Some(self.path(p, e));
        }
        match w {
            b"self" => Some(self.token(p, e - p, SELF)),
            b"super" => Some(self.token(p, e - p, KEYWORD)),
            b"_" => Some(self.token(p, 1, OPERATOR)),
            // no pattern matches inside a word: it is plain
            _ => Some(e),
        }
    }

    /// `kw` at `p`, alone or after `pub\s+` (the word `p..e`): where it starts and ends.
    fn keyword_at(&self, p: usize, e: usize, kw: &[u8]) -> Option<(usize, usize)> {
        let b = self.b;
        let w = &b[p..e];
        if w == kw {
            return Some((p, e));
        }
        if w != b"pub" {
            return None;
        }
        let k = skip_spaces(b, e);
        (k > e && b[k..].starts_with(kw) && ends_word(b, k + kw.len())).then_some((k, k + kw.len()))
    }

    /// `\b(?:(pub)\s+)?(kw)\s+(name)\b` at `p`: `kw` is `leaf`, the name `name`, and `then`
    /// is pushed after it.
    fn item(
        &mut self,
        p: usize,
        e: usize,
        kw: &[u8],
        leaf: u16,
        name: u16,
        then: Option<u8>,
    ) -> Option<usize> {
        let b = self.b;
        let (k, ke) = self.keyword_at(p, e, kw)?;
        let s = skip_spaces(b, ke);
        if s == ke {
            return None;
        }
        let n = ident_end(b, s)?;
        if k > p {
            self.put(p, e, MODIFIER);
        }
        self.put(k, ke, leaf);
        self.put(s, n, name);
        if let Some(c) = then {
            self.push(n, c);
        }
        Some(n)
    }

    /// A path's segment `p..e` before `::`: `self`, `super` or a module's name.
    fn path(&mut self, p: usize, e: usize) -> usize {
        let leaf = match &self.b[p..e] {
            b"self" => PATH_SELF,
            b"super" => PATH_SUPER,
            _ => PATH,
        };
        self.put(p, e, leaf);
        self.put(e, e + 2, PATH_ACCESSOR);
        self.push(e + 2, NO_TYPE_NAMES);
        e + 2
    }

    /// `no-type-names` at `p`: a path's next segment, or nothing (an empty match that ends it).
    fn no_type_names(&mut self, p: usize, c: u8) -> usize {
        let b = self.b;
        if let Some(e) = self.comment(p) {
            return e;
        }
        if is_word(c) && starts_word(b, p) {
            let e = word_end(b, p);
            let w = &b[p..e];
            if is_constant(w) {
                return self.token(p, e - p, CONSTANT);
            }
            if is_c_type(w) {
                return self.token(p, e - p, STORAGE);
            }
            let ident = is_ident(w);
            if ident && b[e..].starts_with(b"::") {
                return self.path(p, e);
            }
            match w {
                b"self" => return self.token(p, e - p, SELF),
                b"super" => return self.token(p, e - p, KEYWORD),
                _ if ident => return e,
                _ => {}
            }
        }
        if c == b':' && b.get(p + 1) == Some(&b':') && ident_end(b, p + 2).is_some() {
            self.put(p, p + 2, PATH_ACCESSOR);
            self.push(p + 2, NO_TYPE_NAMES);
            return p + 2;
        }
        if c == b'<' {
            self.push(p, GENERIC);
        } else {
            self.pop(p);
        }
        p
    }

    /// `numbers` at `p`, a digit; floats only with `floats`.
    fn number(&mut self, p: usize, floats: bool) -> Option<usize> {
        let b = self.b;
        let digit = |c: u8| c.is_ascii_digit();
        let r = digits_end(b, p, digit);
        let suffix = |at: usize, s: &[u8]| b[at..].starts_with(s) && ends_word(b, at + s.len());
        if floats {
            if b.get(r) == Some(&b'.') && b[r - 1].is_ascii_digit() {
                if b.get(r + 1).is_some_and(u8::is_ascii_digit) {
                    // `\d.\d`, an exponent, `f32` or `f64`
                    let mut end = digits_end(b, r + 1, digit);
                    if matches!(b.get(end), Some(b'e' | b'E')) {
                        let mut s = end + 1;
                        if matches!(b.get(s), Some(b'+' | b'-')) {
                            s += 1;
                        }
                        let x = digits_end(b, s, digit);
                        if b[s..x].iter().any(u8::is_ascii_digit) {
                            end = x;
                        }
                    }
                    self.put(p, end, FLOAT);
                    if b[end..].starts_with(b"f32") || b[end..].starts_with(b"f64") {
                        return Some(self.token(end, 3, NUMERIC));
                    }
                    return Some(end);
                }
                if b.get(r + 1) != Some(&b'.') {
                    return Some(self.token(p, r + 1 - p, FLOAT));
                }
            }
            if suffix(r, b"f32") || suffix(r, b"f64") {
                self.put(p, r, FLOAT);
                return Some(self.token(r, 3, NUMERIC));
            }
        }
        let e = word_end(b, p);
        // the integer's digits, then a suffix or nothing to the word's end
        let int = |s: &mut Self, from: usize, leaf: u16| -> Option<usize> {
            let rest = &b[from..e];
            if !rest.is_empty() && !is_int_suffix(rest) {
                return None;
            }
            s.put(p, from, leaf);
            Some(s.token(from, e - from, NUMERIC))
        };
        if let Some(q) = int(self, r, DECIMAL) {
            return Some(q);
        }
        if b[p] == b'0' {
            let based = |test: fn(u8) -> bool| {
                let d = digits_end(b, p + 2, test);
                b[p + 2..d].iter().any(|&c| test(c)).then_some(d)
            };
            let (d, leaf) = match b.get(p + 1) {
                Some(b'x') => (based(|c| c.is_ascii_hexdigit()), HEX),
                Some(b'o') => (based(|c| matches!(c, b'0'..=b'7')), OCTAL),
                Some(b'b') => (based(|c| matches!(c, b'0' | b'1')), BINARY),
                _ => (None, PLAIN),
            };
            if let Some(d) = d {
                return int(self, d, leaf);
            }
        }
        None
    }

    /// `constant-integer-expression` at `p`.
    fn const_expr(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        match c {
            b'0'..=b'9' => starts_word(b, p).then(|| self.number(p, false)).flatten(),
            b'(' => {
                self.push(p, CONST_GROUP);
                Some(self.token(p, 1, GROUP_BEGIN))
            }
            b':' if b.get(p + 1) == Some(&b':') => Some(self.token(p, 2, DOUBLE_COLON)),
            b'-' | b'+' | b'%' | b'/' | b'*' => Some(self.token(p, 1, ARITHMETIC)),
            _ => {
                let e = ident_end(b, p)?;
                Some(self.token(p, e - p, CONSTANT_VAR))
            }
        }
    }

    /// `type-any-identifier` at `p`.
    fn tai(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let c = b[p];
        match c {
            b'&' => Some(self.token(p, 1, OPERATOR)),
            b'*' => {
                let n = if b[p + 1..].starts_with(b"mut") {
                    4
                } else if b[p + 1..].starts_with(b"const") {
                    6
                } else {
                    return None;
                };
                ends_word(b, p + n).then(|| self.token(p, n, MODIFIER))
            }
            b'\'' => self.lifetime(p),
            b':' => (b.get(p + 1) == Some(&b':') && ident_end(b, p + 2).is_some())
                .then(|| self.token(p, 2, PATH_ACCESSOR)),
            b'<' => {
                self.push(p, GENERIC);
                Some(p)
            }
            b'(' => {
                self.push(p, TYPE_GROUP);
                Some(self.token(p, 1, GROUP_BEGIN))
            }
            _ if is_word(c) && starts_word(b, p) => {
                let e = word_end(b, p);
                let w = &b[p..e];
                let next = b.get(e).copied();
                match w {
                    b"ref" | b"mut" | b"const" => return Some(self.token(p, e - p, MODIFIER)),
                    b"fn" if next == Some(b'(') => {
                        self.put(p, e, FN);
                        self.push(e, FN_TYPE);
                        return Some(self.token(e, 1, META_GROUP_BEGIN));
                    }
                    _ => {}
                }
                let ident = is_ident(w);
                if b[e..].starts_with(b"::") {
                    if is_type_name(w) {
                        self.put(p, e, PATH_TYPE);
                        return Some(self.token(e, 2, TYPE_ACCESSOR));
                    }
                    if ident {
                        self.put(p, e, PATH);
                        return Some(self.token(e, 2, PATH_ACCESSOR));
                    }
                }
                if next == Some(b'<') && ident {
                    if is_support(w) {
                        self.put(p, e, SUPPORT_TYPE);
                    }
                    self.push(e, GENERIC);
                    return Some(e);
                }
                if is_primitive(w) {
                    return Some(self.token(p, e - p, STORAGE));
                }
                (w == b"_").then(|| self.token(p, 1, OPERATOR))
            }
            _ => None,
        }
    }

    fn closure_params(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        match c {
            b'}' | b';' | b')' | b']' => {
                self.pop(p);
                Some(p)
            }
            b',' => Some(self.token(p, 1, SEPARATOR)),
            b'(' => {
                self.push(p, PARAM_TUPLE);
                Some(self.token(p, 1, META_GROUP_BEGIN))
            }
            b':' => {
                let o = colon_paren(b, p)?;
                self.put(p, p + 1, SEPARATOR);
                self.push(o, PARAM_TUPLE_TYPE);
                Some(self.token(o, 1, META_GROUP_BEGIN))
            }
            b'|' => Some(self.close(p, PARAMS_END)),
            b'&' => Some(self.token(p, 1, OPERATOR)),
            b'\'' => self.lifetime(p),
            _ if is_word(c) && starts_word(b, p) => {
                let e = word_end(b, p);
                let w = &b[p..e];
                if w == b"self" {
                    return Some(self.token(p, e - p, PARAMETER));
                }
                if is_ident(w) {
                    // `name\s*(?:(:(?!:))|(?=\||,))`
                    let s = skip_spaces(b, e);
                    match b.get(s) {
                        Some(b':') if b.get(s + 1) != Some(&b':') => {
                            self.put(p, e, PARAMETER);
                            self.put(s, s + 1, SEPARATOR);
                            self.push(s + 1, CLOSURE_PARAM_TYPE);
                            return Some(s + 1);
                        }
                        Some(b'|' | b',') => {
                            self.put(p, e, PARAMETER);
                            self.push(s, CLOSURE_PARAM_TYPE);
                            return Some(s);
                        }
                        _ => {}
                    }
                }
                matches!(w, b"mut" | b"ref").then(|| self.token(p, e - p, MODIFIER))
            }
            _ => None,
        }
    }

    fn return_type(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        // `(?=\s*\{|\bwhere\b)`
        let q = skip_spaces(b, p);
        if b.get(q) == Some(&b'{') || self.is_word_at(p, b"where") {
            self.pop(p);
            return Some(p);
        }
        if is_space(c) {
            return None;
        }
        if let Some(e) = self.tai(p) {
            return Some(e);
        }
        if let Some(e) = ident_end(b, p) {
            return Some(e);
        }
        self.pop(p);
        Some(p)
    }

    fn fn_params(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if let Some(e) = self.comment(p) {
            return Some(e);
        }
        match c {
            b')' => {
                self.put(p, p + 1, FN_PARAMS_END);
                self.set(p + 1, FN_RETURN);
                return Some(p + 1);
            }
            b'(' => {
                self.push(p, PARAM_TUPLE);
                return Some(self.token(p, 1, META_GROUP_BEGIN));
            }
            b':' => {
                if let Some(o) = colon_paren(b, p) {
                    self.put(p, p + 1, SEPARATOR);
                    self.push(o, PARAM_TUPLE_TYPE);
                    return Some(self.token(o, 1, META_GROUP_BEGIN));
                }
            }
            _ if is_word(c) && starts_word(b, p) => {
                let e = word_end(b, p);
                if &b[p..e] == b"self" {
                    return Some(self.token(p, e - p, PARAMETER));
                }
                if is_ident(&b[p..e]) {
                    // `name\s*(:(?!:))`
                    let s = skip_spaces(b, e);
                    if b.get(s) == Some(&b':') && b.get(s + 1) != Some(&b':') {
                        self.put(p, e, PARAMETER);
                        return Some(self.token(s, 1, SEPARATOR));
                    }
                }
            }
            _ => {}
        }
        self.tai(p)
    }

    fn struct_fields(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if c == b'}' {
            return Some(self.close(p, BLOCK_END));
        }
        if let Some(e) = self.comment(p) {
            return Some(e);
        }
        if c == b'#' {
            return self.attribute(p);
        }
        if !(is_word(c) && starts_word(b, p)) {
            return None;
        }
        let e = word_end(b, p);
        let w = &b[p..e];
        if w == b"pub" {
            self.put(p, e, MODIFIER);
            if b[e..].starts_with(b"(crate)") {
                self.put(e, e + 1, MODIFIER_BEGIN);
                self.put(e + 1, e + 6, MODIFIER);
                return Some(self.token(e + 6, 1, MODIFIER_END));
            }
            return Some(e);
        }
        // `name(?=\s*:)`
        let s = skip_spaces(b, e);
        if is_ident(w) && b.get(s) == Some(&b':') {
            self.put(p, e, MEMBER);
            self.push(e, FIELD_TYPE);
            return Some(e);
        }
        None
    }

    fn generic_args(&mut self, top: u8, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        match (top, c) {
            (GENERIC_NESTED, b'>') => return Some(self.close(p, GENERIC_BEGIN)),
            (GENERIC_ARRAY, b']') => return Some(self.close(p, GROUP_END)),
            (GENERIC_ARRAY, b';') => {
                self.put(p, p + 1, SEPARATOR);
                self.set(p + 1, GENERIC_LEN);
                return Some(p + 1);
            }
            _ => {}
        }
        if let Some(e) = self.comment(p) {
            return Some(e);
        }
        if self.is_word_at(p, b"extern") {
            self.put(p, p + 6, KEYWORD);
            self.push(p + 6, GENERIC_EXTERN);
            return Some(p + 6);
        }
        // `(?=>|[^ \t\n\$=<_+'(),&:\[\]*\p{L}\p{N}])`
        let allowed = matches!(
            c,
            b' ' | b'\t'
                | b'$'
                | b'='
                | b'<'
                | b'_'
                | b'+'
                | b'\''
                | b'('
                | b')'
                | b','
                | b'&'
                | b':'
                | b'['
                | b']'
                | b'*'
        ) || is_word(c);
        if !allowed {
            self.pop(p);
            return Some(p);
        }
        match c {
            b'<' => {
                self.push(p, GENERIC_NESTED);
                return Some(self.token(p, 1, GENERIC_BEGIN));
            }
            b'[' => {
                self.push(p, GENERIC_ARRAY);
                return Some(self.token(p, 1, GROUP_BEGIN));
            }
            _ => {}
        }
        if let Some(e) = self.tai(p) {
            return Some(e);
        }
        match c {
            b':' => Some(self.token(p, 1, SEPARATOR)),
            b'+' => Some(self.token(p, 1, OPERATOR)),
            _ if is_word(c) && starts_word(b, p) => {
                let e = word_end(b, p);
                let w = &b[p..e];
                if is_support(w) {
                    Some(self.token(p, e - p, SUPPORT_TYPE))
                } else if w == b"as" {
                    Some(self.token(p, 2, OPERATOR))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// `(\$name)(:)(ident|path|expr|ty|pat|stmt|block|item|meta|tt)` at `p`.
    fn metavariable(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let e = ident_end(b, p + 1)?;
        if b.get(e) != Some(&b':') {
            return None;
        }
        let frag = [
            &b"ident"[..],
            b"path",
            b"expr",
            b"ty",
            b"pat",
            b"stmt",
            b"block",
            b"item",
            b"meta",
            b"tt",
        ]
        .into_iter()
        .find(|f| b[e + 1..].starts_with(f))?;
        self.put(p, e, MACRO_PARAMETER);
        self.put(e, e + 1, SEPARATOR);
        Some(self.token(e + 1, frag.len(), STORAGE))
    }
}

#[cfg(test)]
mod tests {
    use crate::Lang;
    use crate::tests::marked;

    fn of(lines: &[&str]) -> Vec<(String, String)> {
        marked(Lang::Rust, lines)
            .into_iter()
            .map(|(t, s)| (t.to_owned(), s.to_owned()))
            .collect()
    }

    fn has(lines: &[&str], text: &str, scopes: &str) {
        let got = of(lines);
        assert!(
            got.iter().any(|(t, s)| t == text && s == scopes),
            "{text:?} as {scopes:?} in {got:#?}"
        );
    }

    #[test]
    fn items() {
        let l = ["pub fn main<T>(x: u32, self) -> Option<T> where T: Copy {"];
        has(&l, "pub", "storage.modifier.rust");
        has(&l, "fn", "storage.type.function.rust");
        has(&l, "main", "entity.name.function.rust");
        has(&l, "x", "variable.parameter.rust");
        has(&l, "u32", "storage.type.rust");
        has(&l, "self", "variable.parameter.rust");
        has(&l, "Option", "support.type.rust");
        has(&l, "where", "keyword.other.rust");
        let l = ["impl<'a> fmt::Display for Foo<'a> {"];
        has(&l, "impl", "storage.type.impl.rust");
        has(&l, "for", "keyword.other.rust");
        has(&l, "Foo", "entity.name.impl.rust");
        has(&l, "'a", "storage.modifier.lifetime.rust");
        let l = [
            "struct S {",
            "    pub a: Vec<u8>,",
            "    b: &'static str,",
            "}",
        ];
        has(&l, "S", "entity.name.struct.rust");
        has(&l, "a", "variable.other.member.rust");
        has(&l, "b", "variable.other.member.rust");
        has(&l, ":", "punctuation.separator.type.rust");
    }

    #[test]
    fn strings_and_comments() {
        let l = [
            r##"let s = r#"raw"#; let b = b"\x00"; let c = '\n';"##,
            "println!(\"{} {x:?} {{}}\", a); // $x note",
            "/** doc",
            "  * star",
            "  */ let x = \"multi",
            "line\";",
        ];
        has(
            &l,
            "r",
            "string.quoted.double.raw.rust storage.type.string.rust",
        );
        has(
            &l,
            "#\"",
            "string.quoted.double.raw.rust punctuation.definition.string.begin.rust",
        );
        has(
            &l,
            "\\x00",
            "string.quoted.double.rust constant.character.escape.rust",
        );
        has(
            &l,
            "\\n",
            "string.quoted.single.rust constant.character.escape.rust",
        );
        has(
            &l,
            "{}",
            "string.quoted.double.rust constant.other.placeholder.rust",
        );
        has(
            &l,
            "{x:?}",
            "string.quoted.double.rust constant.other.placeholder.rust",
        );
        has(
            &l,
            "{{}}",
            "string.quoted.double.rust constant.character.escape.rust",
        );
        has(
            &l,
            "$x",
            "comment.line.double-slash.rust variable.other.rust",
        );
        has(
            &l,
            "/** ",
            "comment.block.documentation.rust punctuation.definition.comment.rust",
        );
        has(
            &l,
            "*",
            "comment.block.documentation.rust punctuation.definition.comment.rust",
        );
        has(&l, "line", "string.quoted.double.rust");
    }

    #[test]
    fn attributes_and_closures() {
        let l = [
            "#[cfg(not(feature = \"x\"))]",
            "let f = |a, b: u8| a + b;",
            "xs.iter().map(|x| x * 2)",
        ];
        has(&l, "cfg", "meta.annotation.rust variable.annotation.rust");
        has(
            &l,
            "not",
            "meta.annotation.rust meta.function-call.rust variable.function.rust",
        );
        has(&l, "feature ", "meta.annotation.rust");
        has(&l, "f", "entity.name.function.rust");
        has(&l, "a", "variable.parameter.rust");
        has(&l, "x", "variable.parameter.rust");
        has(&l, "map", "support.function.rust");
    }

    #[test]
    fn state_carries_raw_strings_and_nesting() {
        let l = [
            "let s = r##\"a",
            "\"# still",
            "end\"##; fn f(",
            "  a: u8,",
            ") {}",
        ];
        has(&l, "\"# still", "string.quoted.double.raw.rust");
        has(&l, "a", "variable.parameter.rust");
    }
}
