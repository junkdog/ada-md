//! Groovy on a scanner of ada's own (X-135), from a floor probe (the same file built with
//! `ADA_PARSE_X135_SOL`): one loop over a line's bytes, a byte's action one load from a table
//! ([`ACT`]), a name read once and looked up once in one table of every word ([`WORDS`]: its leaf
//! and, for the words whose meaning hangs on where they stand, an action), the previous
//! significant token in a byte ([`Scan::prev`]), and frames only for what nests ([`Scan::frames`],
//! five bits a level): parentheses (a call's, colored, and a group's), braces, brackets, and a
//! string's `${…}`. A string continued past a line's end (`'''`, `"""`, slashy, dollar-slashy) and
//! a block comment are a mode ([`Scan::mode`]); the strings whose `${` is open are a nibble each
//! ([`Scan::skinds`]), and the code inside a `${…}` fills its plain text with its string's
//! ([`Scan::fill`]). Kinds are bat's leaves.

mod doc;
#[cfg(test)]
mod tests;
mod text;

use crate::emit::{Rules, Run, Runs, Sink, Spans};
use crate::prim::{WORD, char_len, is_word, is_word_starting, skip_spaces, word_end};
use crate::regex;
use crate::sql;
use crate::words::{Words, text_len};
use crate::{Out, State};

/// The floor probe (X-135's `sol`): every word its table leaf, a capitalized word a class's or a
/// constant's, `name:` a key, `name(` a call, strings with their `${…}`, comments and brackets;
/// no declaration, parameter, import, annotation argument, ternary, case, SQL or regex read.
const SOL: bool = option_env!("ADA_PARSE_X135_SOL").is_some();

/// The leaves below the strings': a token's scopes, bat's.
static LEAVES: [&str; STR_BASE as usize] = [
    "",
    "comment.line.double-slash.groovy",
    "comment.line.double-slash.groovy punctuation.definition.comment.groovy",
    "comment.block.groovy",
    "comment.block.groovy punctuation.definition.comment.groovy",
    "comment.block.documentation.javadoc text.html.javadoc",
    "comment.block.documentation.javadoc punctuation.definition.comment.begin.javadoc",
    "comment.block.documentation.javadoc punctuation.definition.comment.end.javadoc",
    "comment.block.documentation.javadoc text.html.javadoc punctuation.definition.comment.javadoc",
    "comment.block.documentation.javadoc text.html.javadoc keyword.other.documentation.javadoc punctuation.definition.keyword.javadoc",
    "comment.block.documentation.javadoc text.html.javadoc keyword.other.documentation.javadoc",
    "comment.block.documentation.javadoc text.html.javadoc variable.parameter.javadoc",
    "comment.block.documentation.javadoc text.html.javadoc punctuation.section.inline-tag.begin.javadoc",
    "comment.block.documentation.javadoc text.html.javadoc punctuation.section.inline-tag.end.javadoc",
    "comment.block.documentation.javadoc text.html.javadoc markup.raw.javadoc",
    "comment.block.documentation.javadoc text.html.javadoc markup.underline.link.javadoc",
    "comment.block.documentation.javadoc text.html.javadoc punctuation.definition.tag.begin.html",
    "comment.block.documentation.javadoc text.html.javadoc punctuation.definition.tag.end.html",
    "comment.block.documentation.javadoc text.html.javadoc entity.name.tag.other.html",
    "comment.block.empty.groovy punctuation.definition.comment.groovy",
    "comment.line.hashbang.groovy",
    "comment.line.hashbang.groovy punctuation.definition.comment.groovy",
    "keyword.control.groovy",
    "keyword.control.exception.groovy",
    "keyword.control.assert.groovy",
    "keyword.other.new.groovy",
    "keyword.other.import.groovy",
    "keyword.other.package.groovy",
    "keyword.other.throws.groovy",
    "storage.type.def.groovy",
    "storage.type.primitive.groovy",
    "storage.type.class.groovy",
    "storage.type.annotation.groovy",
    "storage.type.import.groovy",
    "storage.type.package.groovy",
    "storage.type.parameter.groovy",
    "storage.type.return-type.void.groovy",
    "storage.type.return-type.primitive.groovy",
    "storage.type.return-type.class.groovy",
    "storage.type.return-type.def.groovy",
    "storage.type.throwable.groovy",
    "storage.modifier.access-control.groovy",
    "storage.modifier.static.groovy",
    "storage.modifier.final.groovy",
    "storage.modifier.other.groovy",
    "storage.modifier.extends.groovy",
    "storage.modifier.implements.groovy",
    "storage.modifier.throws.groovy",
    "entity.name.function.groovy",
    "entity.name.function.constructor.groovy",
    "entity.name.type.class.groovy",
    "entity.other.inherited-class.groovy",
    "entity.other.inherited-class.interface.groovy",
    "variable.parameter.groovy",
    "variable.language.groovy",
    "constant.language.groovy",
    "constant.numeric.groovy",
    "constant.other.groovy",
    "constant.other.key.groovy",
    "support.function.print.groovy",
    "support.function.testing.groovy",
    "support.function.other.groovy",
    "punctuation.definition.method-parameters.begin.groovy",
    "punctuation.definition.method-parameters.end.groovy",
    "punctuation.definition.separator.parameter.groovy",
    "punctuation.accessor.dot.groovy",
    "punctuation.accessor.groovy",
    "punctuation.definition.separator.key-value.groovy",
    "punctuation.section.block.begin.groovy",
    "punctuation.section.block.end.groovy",
    "punctuation.definition.separator.groovy",
    "punctuation.definition.structure.begin.groovy",
    "punctuation.definition.structure.end.groovy",
    "punctuation.definition.parameters.begin.groovy",
    "punctuation.definition.parameters.end.groovy",
    "punctuation.definition.parameters.separator.groovy",
    "punctuation.definition.annotation-arguments.begin.groovy",
    "punctuation.definition.annotation-arguments.end.groovy",
    "punctuation.definition.case-terminator.groovy",
    "punctuation.definition.implemented.interfaces.separator.groovy",
    "punctuation.definition.throwables.separator.groovy",
    "keyword.operator.assignment.groovy",
    "keyword.operator.comparison.groovy",
    "keyword.operator.arithmetic.groovy",
    "keyword.operator.logical.groovy",
    "keyword.operator.increment-decrement.groovy",
    "keyword.operator.arrow.groovy",
    "keyword.operator.elvis.groovy",
    "keyword.operator.range.groovy",
    "keyword.operator.leftshift.groovy",
    "keyword.operator.match.groovy",
    "keyword.operator.find.groovy",
    "keyword.operator.ternary.groovy",
    "keyword.operator.ternary.expression-separator.groovy",
    "keyword.operator.assert.expression-separator.groovy",
    "keyword.operator.as.groovy",
    "keyword.operator.is.groovy",
    "keyword.operator.instanceof.groovy",
    "comment.block.documentation.javadoc text.html.javadoc constant.character.entity.named.html",
    "comment.block.documentation.javadoc text.html.javadoc entity.other.attribute-name.html",
    "comment.block.documentation.javadoc text.html.javadoc string.quoted.double.html",
    "comment.block.documentation.javadoc text.html.javadoc punctuation.separator.key-value.html",
];

const PLAIN: u16 = 0;
const LINE_COMMENT: u16 = 1;
const LINE_MARK: u16 = 2;
const BLOCK_COMMENT: u16 = 3;
const BLOCK_MARK: u16 = 4;
const DOC: u16 = 5;
const DOC_BEGIN: u16 = 6;
const DOC_END: u16 = 7;
const DOC_STAR: u16 = 8;
const DOC_TAG_MARK: u16 = 9;
const DOC_TAG: u16 = 10;
const DOC_PARAM: u16 = 11;
const DOC_INLINE_BEGIN: u16 = 12;
const DOC_INLINE_END: u16 = 13;
const DOC_RAW: u16 = 14;
const DOC_LINK: u16 = 15;
const DOC_HTML_BEGIN: u16 = 16;
const DOC_HTML_END: u16 = 17;
const DOC_HTML_NAME: u16 = 18;
const EMPTY_COMMENT: u16 = 19;
const HASHBANG: u16 = 20;
const HASHBANG_MARK: u16 = 21;
const CONTROL: u16 = 22;
const EXCEPTION: u16 = 23;
const ASSERT: u16 = 24;
const NEW: u16 = 25;
const IMPORT: u16 = 26;
const PACKAGE: u16 = 27;
const THROWS: u16 = 28;
const DEF: u16 = 29;
const PRIMITIVE: u16 = 30;
const CLASS: u16 = 31;
const ANNOTATION: u16 = 32;
const IMPORT_PATH: u16 = 33;
const PACKAGE_PATH: u16 = 34;
const PARAM_TYPE: u16 = 35;
const RET_VOID: u16 = 36;
const RET_PRIMITIVE: u16 = 37;
const RET_CLASS: u16 = 38;
const RET_DEF: u16 = 39;
const THROWABLE: u16 = 40;
const ACCESS: u16 = 41;
const STATIC: u16 = 42;
const FINAL: u16 = 43;
const MODIFIER: u16 = 44;
const EXTENDS: u16 = 45;
const IMPLEMENTS: u16 = 46;
const MOD_THROWS: u16 = 47;
const FUNCTION: u16 = 48;
const CONSTRUCTOR: u16 = 49;
const CLASS_NAME: u16 = 50;
const INHERITED: u16 = 51;
const INTERFACE: u16 = 52;
const PARAMETER: u16 = 53;
const LANGUAGE_VAR: u16 = 54;
const LANGUAGE: u16 = 55;
const NUMBER: u16 = 56;
const CONSTANT: u16 = 57;
const KEY: u16 = 58;
const PRINT: u16 = 59;
const TESTING: u16 = 60;
const SUPPORT: u16 = 61;
const CALL_BEGIN: u16 = 62;
const CALL_END: u16 = 63;
const CALL_SEP: u16 = 64;
const DOT: u16 = 65;
const SAFE_DOT: u16 = 66;
const KEY_SEP: u16 = 67;
const BLOCK_BEGIN: u16 = 68;
const BLOCK_END: u16 = 69;
const SEP: u16 = 70;
const STRUCT_BEGIN: u16 = 71;
const STRUCT_END: u16 = 72;
const PARAMS_BEGIN: u16 = 73;
const PARAMS_END: u16 = 74;
const PARAMS_SEP: u16 = 75;
const ANNOT_BEGIN: u16 = 76;
const ANNOT_END: u16 = 77;
const CASE_END: u16 = 78;
const IFACE_SEP: u16 = 79;
const THROWS_SEP: u16 = 80;
const ASSIGN: u16 = 81;
const COMPARE: u16 = 82;
const ARITH: u16 = 83;
const LOGICAL: u16 = 84;
const INCDEC: u16 = 85;
const ARROW: u16 = 86;
const ELVIS: u16 = 87;
const RANGE: u16 = 88;
const LSHIFT: u16 = 89;
const MATCH: u16 = 90;
const FIND: u16 = 91;
const TERNARY: u16 = 92;
const TERNARY_SEP: u16 = 93;
const ASSERT_SEP: u16 = 94;
const AS: u16 = 95;
const IS: u16 = 96;
const INSTANCEOF: u16 = 97;
const DOC_ENTITY: u16 = 98;
const DOC_ATTR: u16 = 99;
const DOC_ATTR_VALUE: u16 = 100;
const DOC_ATTR_SEP: u16 = 101;
/// The strings' leaves from here: [`SK_N`] flavors of [`STR_LEAVES`] each.
const STR_BASE: u16 = 102;

// A string's kind ([`Scan::skinds`], a nibble): its flavor, and how its text is read.
/// `"…"`: escapes, `$name`, `${…}`; ends with its line. (0 is no string.)
const SK_DQ: u8 = 1;
/// `'…'`: escapes; ends with its line.
const SK_SQ: u8 = 2;
/// `"""…"""`.
const SK_TDQ: u8 = 3;
/// `'''…'''`.
const SK_TSQ: u8 = 4;
/// `/…/`: `\/` an escape, `$name`, `${…}`.
const SK_SLASHY: u8 = 5;
/// `$/…/$`: `$/` and `$$` escapes, `$name`, `${…}`.
const SK_DOLLAR: u8 = 6;
/// The flavors.
const SK_N: u16 = 6;
/// A `"…"`, `'…'`, `"""…"""` or `'''…'''` string of SQL ([`crate::sql`], its context in
/// [`Scan::ctx`]).
const SK_DQ_SQL: u8 = 7;
const SK_SQ_SQL: u8 = 8;
const SK_TDQ_SQL: u8 = 9;
const SK_TSQ_SQL: u8 = 10;
/// A `"…"`, `'…'`, slashy or dollar-slashy string of a regex ([`crate::regex`], Java's
/// dialect, its state in [`Scan::ctx`]).
const SK_DQ_RE: u8 = 11;
const SK_SQ_RE: u8 = 12;
const SK_SLASHY_RE: u8 = 13;
const SK_DOLLAR_RE: u8 = 14;
/// A string kind's flavor ([`SK_DQ`]…[`SK_DOLLAR`]).
const BASE: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 1, 2, 3, 4, 1, 2, 5, 6, 0];

/// A string kind's flavor.
#[inline(always)]
fn base(sk: u8) -> u8 {
    BASE[usize::from(sk & 15)]
}

/// The kinds of a regex in a string, its flavor's from `RE_BASE + (flavor - 1) * 128`.
const RE_BASE: u16 = 1024;
/// The kinds of SQL in a string, its flavor's from `SQL_BASE + (flavor - 1) * 128`.
const SQL_BASE: u16 = 512;
const _: () = assert!(STR_BASE as usize + (SK_N * STR_LEAVES) as usize <= SQL_BASE as usize);
const _: () = assert!(regex::KINDS <= 128 && sql::KINDS <= 128);

/// A string's leaves, after its flavor's base ([`flavor`]).
const TEXT: u16 = 0;
const BEGIN: u16 = 1;
const END: u16 = 2;
const ESCAPE: u16 = 3;
/// `$name`'s name, and each `.name` after it.
const VAR: u16 = 4;
/// `${` and its `}`.
const EMBED: u16 = 5;
/// The `.` between `$a.b`'s names.
const VAR_DOT: u16 = 6;
/// `\q`, an escape Groovy doesn't have.
const BAD_ESCAPE: u16 = 7;
/// `$` before what isn't a name or `{` (`"$5"`): to the string's end.
const BAD_DOLLAR: u16 = 8;
const STR_LEAVES: u16 = 9;
const _: () = assert!(STR_BASE as usize + (SK_N * STR_LEAVES) as usize <= 256);

/// A flavor's string scope, its begin and end marks' scopes, and its bad `$`'s.
const FLAVORS: [(&str, &str, &str, &str); SK_N as usize] = [
    (
        "string.quoted.double.groovy",
        "punctuation.definition.string.begin.groovy",
        "punctuation.definition.string.end.groovy",
        "invalid.illegal.identifier.groovy",
    ),
    (
        "string.quoted.single.groovy",
        "punctuation.definition.string.begin.groovy",
        "punctuation.definition.string.end.groovy",
        "invalid.illegal.groovy",
    ),
    (
        "string.quoted.double.block.groovy",
        "punctuation.definition.string.begin.groovy",
        "punctuation.definition.string.end.groovy",
        "invalid.illegal.groovy",
    ),
    (
        "string.quoted.single.block.groovy",
        "punctuation.definition.string.begin.groovy",
        "punctuation.definition.string.end.groovy",
        "invalid.illegal.groovy",
    ),
    (
        "string.regexp.groovy",
        "punctuation.definition.string.regexp.begin.groovy",
        "punctuation.definition.string.regexp.end.groovy",
        "invalid.illegal.groovy",
    ),
    (
        "string.quoted.other.dollar-slashy.groovy",
        "punctuation.definition.string.begin.groovy",
        "punctuation.definition.string.end.groovy",
        "invalid.illegal.groovy",
    ),
];

/// A string kind's first leaf.
#[inline(always)]
fn flavor(sk: u8) -> u16 {
    STR_BASE + u16::from(base(sk).max(1) - 1) * STR_LEAVES
}

/// The scopes under `source.groovy` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    if kind >= SQL_BASE {
        let (from, re) = if kind >= RE_BASE {
            (RE_BASE, true)
        } else {
            (SQL_BASE, false)
        };
        let f = (kind - from) / 128;
        let k = (kind - from) % 128;
        if f >= SK_N || (!re && usize::from(k) >= sql::KINDS) {
            return Vec::new();
        }
        let mut out = scopes(STR_BASE + f * STR_LEAVES);
        out.extend(if re { regex::scopes(k) } else { sql::scopes(k) });
        return out;
    }
    if kind >= STR_BASE {
        let i = kind - STR_BASE;
        let (sk, off) = (i / STR_LEAVES, i % STR_LEAVES);
        let Some(&(s, b, e, bad)) = FLAVORS.get(usize::from(sk)) else {
            return Vec::new();
        };
        let leaf = match off {
            TEXT => "",
            BEGIN => b,
            END => e,
            ESCAPE => "constant.character.escape.groovy",
            VAR => "variable.other.interpolated.groovy",
            EMBED => "punctuation.section.embedded.groovy",
            VAR_DOT => "punctuation.accessor.dot.groovy",
            BAD_ESCAPE => "invalid.illegal.escape.groovy",
            _ => bad,
        };
        let mut v = vec![s];
        if !leaf.is_empty() {
            v.push(leaf);
        }
        return v;
    }
    LEAVES.get(usize::from(kind)).map_or_else(Vec::new, |l| {
        l.split(' ').filter(|s| !s.is_empty()).collect()
    })
}

// A byte's action ([`ACT`]).
const A_OTHER: u8 = 0;
const A_SPACE: u8 = 1;
const A_NAME: u8 = 2;
const A_DIGIT: u8 = 3;
const A_DQUOTE: u8 = 4;
const A_SQUOTE: u8 = 5;
const A_SLASH: u8 = 6;
/// `=-+!*%<>&|?:~^`: an operator.
const A_OP: u8 = 7;
const A_DOT: u8 = 8;
const A_LPAREN: u8 = 9;
const A_RPAREN: u8 = 10;
const A_LBRACE: u8 = 11;
const A_RBRACE: u8 = 12;
const A_LBRACKET: u8 = 13;
const A_RBRACKET: u8 = 14;
const A_COMMA: u8 = 15;
const A_SEMI: u8 = 16;
const A_AT: u8 = 17;
const A_DOLLAR: u8 = 18;
const A_HASH: u8 = 19;
/// A byte past ASCII: a word's character, or another.
const A_WIDE: u8 = 20;

static ACT: [u8; 256] = {
    let mut t = [A_WIDE; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = match b {
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => A_SPACE,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => A_NAME,
            b'0'..=b'9' => A_DIGIT,
            b'"' => A_DQUOTE,
            b'\'' => A_SQUOTE,
            b'/' => A_SLASH,
            b'=' | b'-' | b'+' | b'!' | b'*' | b'%' | b'<' | b'>' | b'&' | b'|' | b'?' | b':'
            | b'~' | b'^' => A_OP,
            b'.' => A_DOT,
            b'(' => A_LPAREN,
            b')' => A_RPAREN,
            b'{' => A_LBRACE,
            b'}' => A_RBRACE,
            b'[' => A_LBRACKET,
            b']' => A_RBRACKET,
            b',' => A_COMMA,
            b';' => A_SEMI,
            b'@' => A_AT,
            b'$' => A_DOLLAR,
            b'#' => A_HASH,
            _ => A_OTHER,
        };
        c += 1;
    }
    t
};

// What a word is ([`WORDS`], [`INFO`]): its leaf in the low byte, an action in the next bits.
const W_LEAF: u16 = 0xff;
const W_ACTION: u16 = 0x1f << 8;
/// `class`, `interface`, `enum`, `trait`, `record`: a type's head.
const W_CLASS: u16 = 1 << 8;
/// `import` at a line's start: a path follows.
const W_IMPORT: u16 = 2 << 8;
/// `package` at a line's start: a path follows.
const W_PACKAGE: u16 = 3 << 8;
/// `assert` before a blank: to the line's end, its `:` a separator.
const W_ASSERT: u16 = 4 << 8;
/// `case`: to its `:`.
const W_CASE: u16 = 5 << 8;
/// `void`: a method's result type.
const W_VOID: u16 = 6 << 8;
/// `extends`, `implements`: a type's parents follow.
const W_EXTENDS: u16 = 7 << 8;
const W_IMPLEMENTS: u16 = 8 << 8;
/// `throws`: a method's exceptions follow.
const W_THROWS: u16 = 9 << 8;
/// A word bat's `(\w+)(\()` reads as a call's name before `(`: its rule comes before the
/// word's own (a type's, a modifier's, an operator's word).
const W_CALLS: u16 = 1 << 13;
/// A name a SQL call can have ([`sql_call`]): a string after it looks back.
const W_SQL: u16 = 1 << 14;
/// A name a regex call can have (`compile`, `matches`, `replaceAll`, `replaceFirst`): a
/// string after its `(` looks back ([`regex_call`]).
const W_RE: u16 = 1 << 15;

/// The words, each with what it is.
const WORD_LIST: &[(&[u8], u16)] = &[
    (b"try", EXCEPTION),
    (b"catch", EXCEPTION),
    (b"finally", EXCEPTION),
    (b"throw", EXCEPTION),
    (b"return", CONTROL),
    (b"break", CONTROL),
    (b"continue", CONTROL),
    (b"default", CONTROL),
    (b"do", CONTROL),
    (b"while", CONTROL),
    (b"for", CONTROL),
    (b"switch", CONTROL),
    (b"if", CONTROL),
    (b"else", CONTROL),
    (b"case", W_CASE | CONTROL),
    (b"new", NEW),
    (b"assert", W_ASSERT),
    (b"throws", W_THROWS | THROWS),
    (b"as", W_CALLS | AS),
    (b"is", W_CALLS | IS),
    (b"instanceof", W_CALLS | INSTANCEOF),
    (b"def", W_CALLS | DEF),
    (b"boolean", W_CALLS | PRIMITIVE),
    (b"byte", W_CALLS | PRIMITIVE),
    (b"char", W_CALLS | PRIMITIVE),
    (b"short", W_CALLS | PRIMITIVE),
    (b"int", W_CALLS | PRIMITIVE),
    (b"float", W_CALLS | PRIMITIVE),
    (b"long", W_CALLS | PRIMITIVE),
    (b"double", W_CALLS | PRIMITIVE),
    (b"void", W_CALLS | W_VOID),
    (b"private", W_CALLS | ACCESS),
    (b"protected", W_CALLS | ACCESS),
    (b"public", W_CALLS | ACCESS),
    (b"static", W_CALLS | STATIC),
    (b"final", W_CALLS | FINAL),
    (b"native", W_CALLS | MODIFIER),
    (b"synchronized", W_CALLS | MODIFIER),
    (b"abstract", W_CALLS | MODIFIER),
    (b"threadsafe", W_CALLS | MODIFIER),
    (b"transient", W_CALLS | MODIFIER),
    (b"this", LANGUAGE_VAR),
    (b"super", LANGUAGE_VAR),
    (b"true", LANGUAGE),
    (b"false", LANGUAGE),
    (b"null", LANGUAGE),
    (b"class", W_CALLS | W_CLASS),
    (b"interface", W_CALLS | W_CLASS),
    (b"enum", W_CALLS | W_CLASS),
    (b"trait", W_CALLS | W_CLASS),
    (b"record", W_CALLS | W_CLASS),
    (b"import", W_CALLS | W_IMPORT),
    (b"package", W_CALLS | W_PACKAGE),
    (b"extends", W_CALLS | W_EXTENDS),
    (b"implements", W_CALLS | W_IMPLEMENTS),
    // bat's support functions
    (b"sprintf", PRINT),
    (b"print", PRINT),
    (b"printf", PRINT),
    (b"println", PRINT),
    (b"shouldFail", TESTING),
    (b"fail", TESTING),
    (b"failNotEquals", TESTING),
    (b"assume", TESTING),
    (b"assertScript", TESTING),
    (b"assertSame", TESTING),
    (b"assertNotSame", TESTING),
    (b"assertNotNull", TESTING),
    (b"assertNull", TESTING),
    (b"assertContains", TESTING),
    (b"assertThat", TESTING),
    (b"assertToString", TESTING),
    (b"assertTrue", TESTING),
    (b"assertInspect", TESTING),
    (b"assertEquals", TESTING),
    (b"assertFalse", TESTING),
    (b"assertLength", TESTING),
    (b"assertArrayEquals", TESTING),
    (b"sleep", SUPPORT),
    (b"inspect", SUPPORT),
    (b"dump", SUPPORT),
    (b"use", SUPPORT),
    (b"with", SUPPORT),
    // SQL's calls: groovy.sql.Sql's, the JVM's ([`sql::jvm_call`])
    (b"rows", W_CALLS | W_SQL),
    (b"eachRow", W_CALLS | W_SQL),
    (b"firstRow", W_CALLS | W_SQL),
    (b"executeInsert", W_CALLS | W_SQL),
    (b"executeUpdate", W_CALLS | W_SQL),
    (b"executeQuery", W_CALLS | W_SQL),
    (b"executeLargeUpdate", W_CALLS | W_SQL),
    (b"execute", W_CALLS | W_SQL),
    (b"execSQL", W_CALLS | W_SQL),
    (b"query", W_CALLS | W_SQL),
    (b"prepareStatement", W_CALLS | W_SQL),
    (b"prepareCall", W_CALLS | W_SQL),
    (b"addBatch", W_CALLS | W_SQL),
    (b"batchUpdate", W_CALLS | W_SQL),
    (b"createQuery", W_CALLS | W_SQL),
    (b"createNativeQuery", W_CALLS | W_SQL),
    (b"createSQLQuery", W_CALLS | W_SQL),
    (b"queryForObject", W_CALLS | W_SQL),
    (b"queryForList", W_CALLS | W_SQL),
    (b"queryForMap", W_CALLS | W_SQL),
    (b"queryForRowSet", W_CALLS | W_SQL),
    (b"queryForLong", W_CALLS | W_SQL),
    (b"queryForInt", W_CALLS | W_SQL),
    (b"update", W_CALLS | W_SQL),
    (b"select", W_CALLS | W_SQL),
    (b"rawQuery", W_CALLS | W_SQL),
    // regexes' calls
    (b"compile", W_CALLS | W_RE),
    (b"matches", W_CALLS | W_RE),
    (b"replaceAll", W_CALLS | W_RE),
    (b"replaceFirst", W_CALLS | W_RE),
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
static WORDS: Words<256, N, { text_len(&WORD_TEXT) }> = Words::new(&WORD_TEXT);
/// A word's leaf and action by its id; [`W_CALLS`] for none (a name).
static INFO: [u16; N + 1] = {
    let mut t = [W_CALLS; N + 1];
    let mut i = 0;
    while i < N {
        t[i + 1] = WORD_LIST[i].1;
        i += 1;
    }
    t
};

// The frames, five bits a level in [`Scan::frames`]; 0 is none (the top level, or a level lost
// past [`LEVELS`]).
/// `(` … `)`, a group's: plain.
const F_PAREN: u8 = 1;
/// `name(` … `)`, a call's: its parentheses and `,`s colored.
const F_CALL: u8 = 2;
/// `{` … `}`, a block's or a closure's.
const F_BLOCK: u8 = 3;
/// `[` … `]`, a list's, a map's, an index's.
const F_BRACKET: u8 = 4;
/// A call whose first argument is SQL ([`sql_call`]), to its first `,`: a call's otherwise.
const F_SQLCALL: u8 = 13;
/// A string's `${` … `}`: code to its `}`, its string after ([`Scan::skinds`]).
const F_TMPL: u8 = 5;
/// An annotation's arguments, `@A(` … `)`: `name =` a key.
const F_ANNOT: u8 = 6;
/// `{` … `}`, a declaration's body (a type's, a method's): plain, as bat's heads take it.
const F_BODY: u8 = 7;
/// A method's parameters, `(` … `)`: types and names.
const F_PARAMS: u8 = 8;
/// A parameter's default value, after its `=`: code to the next `,` or `)`.
const F_PDEFAULT: u8 = 9;
/// A ternary's `?` … `:` (bat's: to its `:`, the line's end or a `}`): no keys in it.
const F_TERN: u8 = 10;
/// A `case` to its `:` (or `->`): no keys in it.
const F_CASE: u8 = 11;
/// `assert` to the line's end or a `}`: its `:` the message's separator, no keys.
const F_ASSERT: u8 = 12;
/// The levels [`Scan::frames`] holds.
const LEVELS: u8 = 24;
const FRAME_BITS: u32 = 5;
const FRAME_MASK: u128 = (1 << (FRAME_BITS * LEVELS as u32)) - 1;

/// Whether frame `f` is closed by `)`.
#[inline(always)]
fn is_paren_frame(f: u8) -> bool {
    matches!(
        f,
        F_PAREN | F_CALL | F_SQLCALL | F_ANNOT | F_PARAMS | F_PDEFAULT
    )
}

/// Whether frame `f` is one a `)` or a `}` drops when it's left open above theirs.
#[inline(always)]
fn is_open_frame(f: u8) -> bool {
    matches!(f, F_BRACKET | F_TERN | F_CASE | F_ASSERT)
}

/// Whether frame `f` takes no `name:` keys (bat's `groovy-code-minus-map-keys`, a parameter
/// list, an annotation's arguments).
#[inline(always)]
fn no_keys(f: u8) -> bool {
    matches!(f, F_TERN | F_CASE | F_ASSERT | F_PARAMS | F_ANNOT)
}

// What a line starts in ([`Scan::mode`]): code, a comment, or a string's text, its kind in
// [`Scan::skinds`]' low nibble.
const M_CODE: u8 = 0;
const M_COMMENT: u8 = 1;
const M_DOC: u8 = 2;
const M_STRING: u8 = 3;

// What the previous significant token was ([`Scan::prev`]).
/// A line's or a statement's start: after `;`, `{`.
const P_START: u8 = 0;
/// A name or a word that ends a value (`this`, a type).
const P_NAME: u8 = 1;
/// A value's end: `)`, `]`, `}`, a number, a string.
const P_VALUE: u8 = 2;
/// An operator, `(`, `,`, a keyword: an operand may start.
const P_OP: u8 = 3;
/// `.`, `?.`, `*.`: a member's name follows.
const P_DOT: u8 = 4;
/// A word bat's `(\w+)(\()` reads as a call's name, right before `(` ([`W_CALLS`]).
const P_CALLER: u8 = 5;
/// `~`, `=~`, `==~`: a regex may follow (a slashy one, or a quoted one).
const P_REGEX: u8 = 6;

// A declaration's head ([`Scan::head`]).
const H_NONE: u8 = 0;
/// After `class`, `interface`, `enum`, `trait`, `record`: the name.
const H_CLASS: u8 = 1;
/// After a type's name: `extends`, `implements`, type parameters; its `{` the body's.
const H_CLASS_HEAD: u8 = 2;
/// After a method's name: its `(` opens the parameters.
const H_FUN: u8 = 3;
/// After a method's parameters: `throws`; its `{` the body's.
const H_RET: u8 = 4;
/// [`State::arg`]'s bits for a head carried to the next line ([`H_CLASS_HEAD`], [`H_RET`]).
const CARRIED: u16 = 4;

/// Lex `b`, a line of Groovy without its `\n`, from `state`.
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

/// A block of Groovy on one scan opened from `state` ([`crate::Carry`]).
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
    /// What each open frame is ([`F_PAREN`]…), five bits a level, the innermost lowest; past
    /// [`LEVELS`] the outer ones are lost.
    frames: u128,
    /// How many frames are open, up to 255.
    depth: u8,
    /// What the line starts in ([`M_CODE`]…).
    mode: u8,
    /// The kinds of the strings whose `${` is open, a nibble each, the innermost lowest; a
    /// string's kind in the low nibble while [`M_STRING`] holds.
    skinds: u32,
    /// A SQL string's context, or a regex string's state, across a `${…}` or a line's end.
    ctx: u8,
    /// The text kind the plain bytes inside a `${…}` take (its string's), 0 outside one.
    fill: u16,
    /// Where the writes inside a `${…}` end, for the plain text after them.
    gap: usize,
    // The line's own, reset at its start:
    /// The previous significant token ([`P_START`]…).
    prev: u8,
    /// A declaration's head ([`H_NONE`]…), the line's, or carried from the line before.
    head: u8,
    /// A head the line ended in, for the next line's `{` ([`H_CLASS_HEAD`], [`H_RET`]).
    carried: u8,
    /// Where the last qualified class name (`a.b.C`) checked and not found on the line ends:
    /// its lowercase names aren't checked again.
    chain_end: usize,
    /// The last [`W_SQL`] or [`W_RE`] word on the line, `start..end` (`end` `usize::MAX` for
    /// none).
    callw: (usize, usize),
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
            frames: 0,
            depth: 0,
            mode: M_CODE,
            skinds: 0,
            ctx: 0,
            fill: 0,
            gap: 0,
            prev: P_START,
            head: H_NONE,
            carried: H_NONE,
            chain_end: 0,
            callw: (0, usize::MAX),
            sql: sql::Embedded {
                is_end: usize::MAX,
                at: 0,
            },
            k,
            out,
        }
    }

    /// The scan's fields from `state`: the frames in its levels and the first two withs, the
    /// string kinds in the next four, the mode in [`State::arg`].
    #[inline(always)]
    fn load(&mut self, state: &State) {
        let mut w = [0u8; 16];
        w[..14].copy_from_slice(&state.levels);
        w[14] = state.withs[0];
        w[15] = state.withs[1];
        self.frames = u128::from_le_bytes(w);
        self.depth = state.depth;
        self.skinds = u32::from_le_bytes([
            state.withs[2],
            state.withs[3],
            state.withs[4],
            state.withs[5],
        ]);
        self.ctx = state.withs[6];
        self.mode = (state.arg & 3) as u8;
        self.carried = (state.arg >> CARRIED) as u8 & 7;
    }

    /// The state the scan's last line left: every field as it is, so a block lexes as its
    /// lines do one at a time.
    #[inline(always)]
    fn state(&self) -> State {
        let w = self.frames.to_le_bytes();
        let mut s = State::default();
        s.levels.copy_from_slice(&w[..14]);
        s.withs[0] = w[14];
        s.withs[1] = w[15];
        s.withs[2..6].copy_from_slice(&self.skinds.to_le_bytes());
        s.withs[6] = self.ctx;
        s.depth = self.depth;
        s.arg = u16::from(self.mode) | u16::from(self.carried) << CARRIED;
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
        if self.fill != 0 {
            return self.put_fill(s, e, kind);
        }
        self.k.merge_call(&mut self.out, s, e, kind);
    }

    /// A write inlined, for the common tokens.
    #[inline(always)]
    fn put_hot(&mut self, s: usize, e: usize, kind: u16) {
        if self.fill != 0 {
            return self.put_fill(s, e, kind);
        }
        self.k.merge(&mut self.out, s, e, kind);
    }

    /// A write inside a `${…}`: the plain text before it as its string's, then it.
    #[cold]
    #[inline(never)]
    fn put_fill(&mut self, s: usize, e: usize, kind: u16) {
        if s > self.gap {
            self.k.merge_call(&mut self.out, self.gap, s, self.fill);
        }
        self.k.merge_call(
            &mut self.out,
            s,
            e,
            if kind == PLAIN { self.fill } else { kind },
        );
        self.gap = self.gap.max(e);
    }

    /// The plain text inside a `${…}` up to `p` as its string's.
    #[inline(always)]
    fn fill_to(&mut self, p: usize) {
        if self.fill != 0 && p > self.gap {
            self.k.merge_call(&mut self.out, self.gap, p, self.fill);
            self.gap = p;
        }
    }

    /// The fill the strings whose `${` is open give ([`Scan::skinds`] in code): the
    /// innermost's text, or none; its mark past `at`, where the scan stands (what's before it is
    /// written: a comment's, a string's, an inner `${…}`'s).
    #[inline(always)]
    fn refill(&mut self, at: usize) {
        let sk = (self.skinds & 15) as u8;
        self.fill = if sk != 0 { flavor(sk) + TEXT } else { 0 };
        self.gap = self.gap.max(at);
    }

    #[inline(always)]
    fn top(&self) -> u8 {
        (self.frames & 31) as u8
    }

    #[inline(always)]
    fn set_top(&mut self, f: u8) {
        if self.depth > 0 {
            self.frames = self.frames & !31 | u128::from(f);
        }
    }

    #[inline(always)]
    fn push(&mut self, f: u8) {
        self.frames = (self.frames << FRAME_BITS | u128::from(f)) & FRAME_MASK;
        self.depth = self.depth.saturating_add(1);
    }

    #[inline(always)]
    fn pop(&mut self) {
        if self.depth > 0 {
            self.frames >>= FRAME_BITS;
            self.depth -= 1;
        }
    }

    /// The line, from what the line before left: a mode's rest first; the loop out of line
    /// where `SHARED` ([`Scan::code_shared`]).
    #[inline(always)]
    fn lex_line<const SHARED: bool>(&mut self) {
        self.prev = P_START;
        self.head = self.carried;
        self.carried = H_NONE;
        self.chain_end = 0;
        self.callw = (0, usize::MAX);
        self.fill = 0;
        self.gap = 0;
        if self.skinds != 0 && self.mode == M_CODE {
            // inside a `${…}` open across lines
            self.refill(0);
        }
        let p = match self.mode {
            M_CODE => {
                if self.b.starts_with(b"#!") && self.depth == 0 {
                    return self.hashbang();
                }
                0
            }
            M_COMMENT => self.comment(0, false),
            M_DOC if SOL => self.comment(0, true),
            M_DOC => self.doc(0, false),
            _ => self.string_rest(),
        };
        if self.skinds != 0 && self.fill == 0 && self.mode == M_CODE {
            // a comment that ended on the line inside a `${…}`
            self.refill(p);
        }
        if SHARED {
            self.code_shared(p);
        } else {
            self.code(p);
        }
        self.line_end();
    }

    /// The line's end: a `${…}` open in a string that ends with its line ends with it; a
    /// ternary's or an assert's frame ends; a type's or a method's head goes on to the next
    /// line's `{`.
    #[inline(always)]
    fn line_end(&mut self) {
        if self.fill != 0 && self.mode == M_CODE {
            self.line_end_cold();
        }
        if !SOL {
            while matches!(self.top(), F_TERN | F_ASSERT) {
                self.pop();
            }
            if matches!(self.head, H_CLASS_HEAD | H_RET) && self.mode == M_CODE {
                self.carried = self.head;
            }
        }
    }

    #[cold]
    #[inline(never)]
    fn line_end_cold(&mut self) {
        let n = self.b.len();
        self.fill_to(n);
        // the `${…}` of a `"…"` string ends with its line (bat's `(?=\n)`), and so does its
        // string; a slashy string's (bat's `\n`) ends, its string goes on
        loop {
            let sk = (self.skinds & 15) as u8;
            if !matches!(base(sk), SK_DQ | SK_SLASHY) {
                break;
            }
            while self.depth > 0 && self.top() != F_TMPL {
                self.pop();
            }
            self.pop();
            self.skinds >>= 4;
            if base(sk) == SK_SLASHY {
                self.skinds = self.skinds << 4 | u32::from(sk);
                self.mode = M_STRING;
                break;
            }
        }
        self.fill = 0;
    }

    /// `#!` at a line's start: bat's `^(#!).+$`.
    #[cold]
    #[inline(never)]
    fn hashbang(&mut self) {
        let n = self.b.len();
        if n > 2 {
            self.put(0, 2, HASHBANG_MARK);
            self.put(2, n, HASHBANG);
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
            let mut act = ACT[usize::from(c)];
            if act == A_WIDE && is_word_starting(b, p) {
                act = A_NAME;
            }
            p = match act {
                A_SPACE => {
                    let mut e = p + 1;
                    while e < n && ACT[usize::from(b[e])] == A_SPACE {
                        e += 1;
                    }
                    e
                }
                A_NAME => self.name(p),
                A_DIGIT => self.number(p),
                A_DQUOTE => self.dquote(p),
                A_SQUOTE => self.squote(p),
                A_SLASH => self.slash(p),
                A_OP => self.op(p),
                A_DOT => self.dot(p),
                A_LPAREN => self.lparen(p),
                A_RPAREN => self.rparen(p),
                A_LBRACE => self.lbrace(p),
                A_RBRACE => self.rbrace(p),
                A_LBRACKET => {
                    self.push(F_BRACKET);
                    self.put_hot(p, p + 1, STRUCT_BEGIN);
                    self.prev = P_OP;
                    p + 1
                }
                A_RBRACKET => self.rbracket(p),
                A_COMMA => self.comma(p),
                A_SEMI => {
                    self.prev = P_START;
                    self.head = H_NONE;
                    p + 1
                }
                A_AT => self.at_sign(p),
                A_DOLLAR => self.dollar(p),
                A_WIDE => p + char_len(c),
                _ => {
                    self.prev = P_OP;
                    p + 1
                }
            };
        }
    }

    /// `(` at `p`: a call's, right after a name bat reads as one ([`P_CALLER`]), else a group's.
    #[inline(always)]
    fn lparen(&mut self, p: usize) -> usize {
        if !SOL && self.head == H_FUN {
            self.head = H_NONE;
            self.push(F_PARAMS);
            self.put(p, p + 1, PARAMS_BEGIN);
            self.prev = P_OP;
            return p + 1;
        }
        if self.prev == P_CALLER {
            let f = if !SOL && self.callw.1 == p && sql_call(self.b, self.callw.0, p) {
                F_SQLCALL
            } else {
                F_CALL
            };
            self.push(f);
            self.put_hot(p, p + 1, CALL_BEGIN);
        } else {
            self.push(F_PAREN);
        }
        self.prev = P_OP;
        p + 1
    }

    /// `)` at `p`: a call's, a group's or an annotation's, with any bracket left open above it.
    #[inline(always)]
    fn rparen(&mut self, p: usize) -> usize {
        self.prev = P_VALUE;
        let mut f = self.top();
        if !is_paren_frame(f) {
            if !is_open_frame(f) {
                return p + 1;
            }
            while is_open_frame(f) {
                self.pop();
                f = self.top();
            }
            if !is_paren_frame(f) {
                return p + 1;
            }
        }
        self.pop();
        match f {
            F_CALL | F_SQLCALL => self.put_hot(p, p + 1, CALL_END),
            F_ANNOT => self.put(p, p + 1, ANNOT_END),
            F_PARAMS | F_PDEFAULT => {
                self.put(p, p + 1, PARAMS_END);
                self.head = H_RET;
            }
            _ => {}
        }
        p + 1
    }

    /// `{` at `p`: a block.
    #[inline(always)]
    fn lbrace(&mut self, p: usize) -> usize {
        self.prev = P_START;
        if !SOL && self.head != H_NONE {
            self.head_brace(p);
            return p + 1;
        }
        self.push(F_BLOCK);
        self.put_hot(p, p + 1, BLOCK_BEGIN);
        p + 1
    }

    /// A `{` after a head: a type's or a method's body (plain, as bat's heads take it), else a
    /// block.
    #[cold]
    #[inline(never)]
    fn head_brace(&mut self, p: usize) {
        if matches!(self.head, H_CLASS_HEAD | H_RET) {
            self.push(F_BODY);
        } else {
            self.push(F_BLOCK);
            self.put(p, p + 1, BLOCK_BEGIN);
        }
        self.head = H_NONE;
    }

    /// `}` at `p`: the innermost block closed, what it left open dropped; a `${…}`'s, its
    /// string goes on; a stray one nothing.
    #[inline(always)]
    fn rbrace(&mut self, p: usize) -> usize {
        self.prev = P_VALUE;
        match self.top() {
            F_BLOCK => {
                self.pop();
                self.put_hot(p, p + 1, BLOCK_END);
            }
            F_BODY => {
                self.pop();
                self.head = H_NONE;
            }
            F_TMPL => return self.template_close(p),
            _ => {
                if let Some(e) = self.unwind_brace(p) {
                    return e;
                }
            }
        }
        p + 1
    }

    /// A `}` at `p` with frames above its block or `${…}`: they're dropped, if one is open
    /// below them.
    #[cold]
    fn unwind_brace(&mut self, p: usize) -> Option<usize> {
        let levels = self.depth.min(LEVELS);
        let mut i = 0;
        while i < levels {
            let f = ((self.frames >> (FRAME_BITS * u32::from(i))) & 31) as u8;
            match f {
                F_BLOCK | F_BODY => {
                    for _ in 0..=i {
                        self.pop();
                    }
                    if f == F_BLOCK {
                        self.put(p, p + 1, BLOCK_END);
                    }
                    return None;
                }
                F_TMPL => {
                    for _ in 0..i {
                        self.pop();
                    }
                    return Some(self.template_close(p));
                }
                _ => {}
            }
            i += 1;
        }
        None
    }

    /// `]` at `p`.
    #[inline(always)]
    fn rbracket(&mut self, p: usize) -> usize {
        self.prev = P_VALUE;
        if self.top() == F_BRACKET {
            self.pop();
            self.put_hot(p, p + 1, STRUCT_END);
        }
        p + 1
    }

    /// `,` at `p`: a call's or a list's separator, colored; plain elsewhere.
    #[inline(always)]
    fn comma(&mut self, p: usize) -> usize {
        self.prev = P_OP;
        match self.top() {
            F_CALL => self.put_hot(p, p + 1, CALL_SEP),
            F_SQLCALL => {
                self.set_top(F_CALL);
                self.put(p, p + 1, CALL_SEP);
            }
            F_BRACKET | F_ANNOT => self.put_hot(p, p + 1, SEP),
            F_PARAMS | F_PDEFAULT if !SOL => {
                self.set_top(F_PARAMS);
                self.put(p, p + 1, PARAMS_SEP);
            }
            _ => {}
        }
        p + 1
    }

    /// A word at `p`: a keyword, a type, a constant, a key, a call's name, or a name.
    #[inline(always)]
    fn name(&mut self, p: usize) -> usize {
        let b = self.b;
        let mut e = p + 1;
        while e < b.len() && WORD[usize::from(b[e])] {
            e += 1;
        }
        if e < b.len() && b[e] >= 0x80 {
            e = word_end(b, e);
        }
        let w = WORDS.value(&b[p..e], &INFO);
        if !SOL && w & (W_SQL | W_RE) != 0 {
            self.callw = (p, e);
        }
        if !SOL
            && (self.head != H_NONE || self.prev == P_START || self.top() == F_PARAMS)
            && let Some(r) = self.name_cold(p, e, w)
        {
            return r;
        }
        if !SOL
            && w & W_ACTION != 0
            && let Some(r) = self.word_action(p, e, w)
        {
            return r;
        }
        let leaf = w & W_LEAF;
        if leaf != 0 && !(w & W_CALLS != 0 && b.get(e) == Some(&b'(')) {
            // (bat's `(\w+)(\()` comes before a type's, a modifier's and an operator's word)
            self.put_hot(p, e, leaf);
            self.prev = if matches!(leaf, LANGUAGE_VAR | LANGUAGE) {
                P_NAME
            } else {
                P_OP
            };
            return e;
        }
        let c = b[p];
        if c.is_ascii_uppercase() && e - p >= 2 && e - p == word_len_ascii(b, p, e) {
            // bat's `[A-Z][A-Z0-9_]+` a constant, `[A-Z]\w+` a class
            let kind = if is_constant(&b[p..e]) {
                CONSTANT
            } else {
                CLASS
            };
            self.put_hot(p, e, kind);
            self.prev = P_NAME;
            return e;
        }
        match b.get(e) {
            Some(b'(') => {
                self.prev = P_CALLER;
                return e;
            }
            Some(b':' | b' ' | b'\t' | b'=') => {
                let q = if b[e] == b' ' || b[e] == b'\t' {
                    skip_spaces(b, e)
                } else {
                    e
                };
                if SOL || !no_keys(self.top()) {
                    if b.get(q) == Some(&b':') && b.get(q + 1) != Some(&b':') {
                        // bat's `(\w+)\s*(:)`: a map's key, a named argument, a label
                        self.put_hot(p, e, KEY);
                        self.put_hot(q, q + 1, KEY_SEP);
                        self.prev = P_OP;
                        return q + 1;
                    }
                } else if self.top() == F_ANNOT
                    && b.get(q) == Some(&b'=')
                    && b.get(q + 1) != Some(&b'=')
                {
                    // an annotation's `name =`
                    self.put(p, e, KEY);
                }
            }
            Some(b'.') if !SOL && p >= self.chain_end && c.is_ascii_lowercase() => {
                if let Some(f) = self.chain(p, e) {
                    return f;
                }
            }
            _ => {}
        }
        self.prev = P_NAME;
        e
    }

    /// A word `b[p..e]` (`w` its info) in a parameter list, a head, or at a statement's start
    /// in a body: a parameter's type or name, a head's word, a declaration's head; `None` for
    /// the word read as anywhere else.
    #[inline(never)]
    fn name_cold(&mut self, p: usize, e: usize, w: u16) -> Option<usize> {
        let top = self.top();
        if top == F_PARAMS {
            return Some(self.param_word(p, e, w));
        }
        if self.head != H_NONE
            && let Some(r) = self.head_word(p, e, w)
        {
            return Some(r);
        }
        if self.prev == P_START && matches!(top, 0 | F_BODY) && is_decl_word(self.b, p, w) {
            return self.decl(p, e, w);
        }
        None
    }

    /// A word `b[p..e]` in a head: a type's name, `extends` and `implements` with their types,
    /// `throws` with its exceptions; `None` where the head ends before it.
    fn head_word(&mut self, p: usize, e: usize, w: u16) -> Option<usize> {
        let b = self.b;
        match self.head {
            H_CLASS => {
                self.put(p, e, CLASS_NAME);
                self.head = H_CLASS_HEAD;
                self.prev = P_NAME;
                Some(e)
            }
            H_CLASS_HEAD => match w & W_ACTION {
                W_EXTENDS => {
                    self.put(p, e, EXTENDS);
                    // bat's `(extends)\s+([a-zA-Z0-9_\.]+(?:<(?:[a-zA-Z0-9_, ])+>)?)`
                    let s = skip_spaces(b, e);
                    let mut q = s;
                    while q < b.len() && (WORD[usize::from(b[q])] || b[q] == b'.') {
                        q += 1;
                    }
                    if q > s {
                        if b.get(q) == Some(&b'<') {
                            let mut r = q + 1;
                            while r < b.len()
                                && (WORD[usize::from(b[r])] || matches!(b[r], b',' | b' '))
                            {
                                r += 1;
                            }
                            if r > q + 1 && b.get(r) == Some(&b'>') {
                                q = r + 1;
                            }
                        }
                        self.put(s, q, INHERITED);
                    }
                    self.prev = P_NAME;
                    Some(q)
                }
                W_IMPLEMENTS => {
                    self.put(p, e, IMPLEMENTS);
                    self.prev = P_NAME;
                    Some(self.type_list(e, INTERFACE, IFACE_SEP))
                }
                _ if self.prev == P_START => {
                    // a head carried to a line that doesn't go on with it
                    self.head = H_NONE;
                    None
                }
                _ => {
                    // bat's head takes no other word
                    self.prev = P_NAME;
                    Some(e)
                }
            },
            H_RET if w & W_ACTION == W_THROWS => {
                self.put(p, e, MOD_THROWS);
                self.prev = P_NAME;
                Some(self.type_list(e, THROWABLE, THROWS_SEP))
            }
            _ => {
                self.head = H_NONE;
                None
            }
        }
    }

    /// The types after `implements` or `throws` from `e`: bat's
    /// `((?:[a-z]\w*.)*[A-Z]\w*)\s*(?:(,)|$|\{)` again and again, each type `kind`, each `,`
    /// `sep`; where the list ends.
    #[cold]
    fn type_list(&mut self, e: usize, kind: u16, sep: u16) -> usize {
        let b = self.b;
        let n = b.len();
        let mut q = e;
        loop {
            let s = skip_spaces(b, q);
            let Some(t) = qualified_type(b, s) else {
                return q;
            };
            let a = skip_spaces(b, t);
            match b.get(a) {
                Some(b',') => {
                    self.put(s, t, kind);
                    self.put(a, a + 1, sep);
                    q = a + 1;
                }
                None | Some(b'{') => {
                    self.put(s, t, kind);
                    return t;
                }
                _ if a == n => {
                    self.put(s, t, kind);
                    return t;
                }
                _ => return q,
            }
        }
    }

    /// A word whose meaning hangs on where it stands (`w`'s action): a type's head, `import`
    /// and `package` at a line's start, `assert`, `case`; `None` for the word read as a name
    /// or by its leaf.
    #[inline(never)]
    fn word_action(&mut self, p: usize, e: usize, w: u16) -> Option<usize> {
        let b = self.b;
        if self.prev == P_DOT {
            return None;
        }
        match w & W_ACTION {
            W_CLASS if is_word_starting(b, skip_spaces(b, e)) => {
                // bat's `class` (and ada's `interface`, `enum`, `trait`, `record`)
                self.put(p, e, CLASS);
                self.head = H_CLASS;
                self.prev = P_OP;
                Some(e)
            }
            W_IMPORT | W_PACKAGE if self.depth == 0 && skip_spaces(b, 0) == p => {
                Some(self.header(p, e, w & W_ACTION == W_PACKAGE))
            }
            W_ASSERT if matches!(b.get(e), None | Some(b' ' | b'\t')) => {
                // bat's `\b(assert)\s`
                self.put(p, e, ASSERT);
                self.push(F_ASSERT);
                self.prev = P_OP;
                Some(e)
            }
            W_CASE => {
                self.put(p, e, CONTROL);
                self.push(F_CASE);
                self.prev = P_OP;
                Some(e)
            }
            _ => None,
        }
    }

    /// `package` or `import` at `p..e`, a line's first word at the top level: bat's
    /// `^\s*(package)\s+([^ ;]+)` and `^\s*(import)\s+([^ ;$]+)`, `import static`'s `static` a
    /// modifier (bat's takes it for the path).
    #[cold]
    #[inline(never)]
    fn header(&mut self, p: usize, e: usize, package: bool) -> usize {
        let b = self.b;
        let n = b.len();
        let mut s = skip_spaces(b, e);
        if s == e || s >= n {
            self.prev = P_NAME;
            return e;
        }
        self.put(p, e, if package { PACKAGE } else { IMPORT });
        if !package && b[s..].starts_with(b"static") && !is_word(b, s + 6) {
            self.put(s, s + 6, STATIC);
            s = skip_spaces(b, s + 6);
        }
        let mut q = s;
        while q < n && !matches!(b[q], b' ' | b'\t' | b';') && (package || b[q] != b'$') {
            q += 1;
        }
        if q > s {
            self.put(s, q, if package { PACKAGE_PATH } else { IMPORT_PATH });
        }
        self.prev = P_NAME;
        q
    }

    /// A method's or a constructor's head, or a type's after modifiers, from the word `b[p..e]`
    /// at a statement's start in a body: bat's `methods` (modifiers, a result type, the name
    /// before `(`; a constructor's capitalized name before `(`), but anywhere a statement
    /// starts in a type's body or a script, and with `<T>` type parameters, a qualified or
    /// generic result type; `None` where no head follows.
    #[cold]
    #[inline(never)]
    fn decl(&mut self, p: usize, e: usize, w: u16) -> Option<usize> {
        let b = self.b;
        // the modifiers
        let mut q = p;
        let mut qe = e;
        let mut qw = w;
        let mut mods = 0;
        while matches!(qw & W_LEAF, ACCESS | STATIC | FINAL | MODIFIER) {
            mods += 1;
            q = skip_spaces(b, qe);
            if !is_word_starting(b, q) {
                if mods > 0 && b.get(q) == Some(&b'<') {
                    break;
                }
                return None;
            }
            qe = word_end(b, q);
            qw = WORDS.value(&b[q..qe], &INFO);
        }
        if qw & W_ACTION == W_CLASS && mods > 0 {
            self.write_mods(p, q);
            self.put(q, qe, CLASS);
            self.head = H_CLASS;
            self.prev = P_OP;
            return Some(qe);
        }
        // type parameters
        if b.get(q) == Some(&b'<') {
            let t = angle_end(b, q)?;
            q = skip_spaces(b, t);
            if !is_word_starting(b, q) {
                return None;
            }
            qe = word_end(b, q);
            qw = WORDS.value(&b[q..qe], &INFO);
        }
        // the result type
        let (ty, te) = match (qw & W_LEAF, qw & W_ACTION) {
            (DEF, _) => (RET_DEF, qe),
            (PRIMITIVE, _) => (RET_PRIMITIVE, qe),
            (_, W_VOID) => (RET_VOID, qe),
            (0, 0) => match class_type_end(b, q) {
                Some(t) => (RET_CLASS, t),
                None => (PLAIN, q),
            },
            _ => (PLAIN, q),
        };
        if ty == PLAIN && mods == 0 {
            return None;
        }
        let ns = if ty == PLAIN { q } else { skip_spaces(b, te) };
        if ty == RET_CLASS && b.get(ns) == Some(&b'(') && te == qe && mods <= 1 {
            // a constructor: bat's `([A-Z](?:[a-zA-Z0-9_])+)\s*(?=\()`
            self.write_mods(p, q);
            self.put(q, qe, CONSTRUCTOR);
            self.head = H_FUN;
            self.prev = P_NAME;
            return Some(qe);
        }
        let c = *b.get(ns)?;
        if !(c.is_ascii_alphanumeric() || c == b'_') {
            return None;
        }
        let ne = word_end(b, ns);
        if b.get(skip_spaces(b, ne)) != Some(&b'(') || (ty == PLAIN && ns != q) {
            return None;
        }
        self.write_mods(p, q);
        if ty != PLAIN {
            self.put(q, te, ty);
        }
        self.put(ns, ne, FUNCTION);
        self.head = H_FUN;
        self.prev = P_NAME;
        Some(ne)
    }

    /// The modifiers in `b[p..q]`, each its leaf.
    fn write_mods(&mut self, p: usize, q: usize) {
        let b = self.b;
        let mut i = p;
        while i < q {
            if is_word_starting(b, i) {
                let e = word_end(b, i);
                let leaf = WORDS.value(&b[i..e], &INFO) & W_LEAF;
                if leaf != 0 {
                    self.put(i, e, leaf);
                }
                i = e;
            } else {
                i += 1;
            }
        }
    }

    /// A word `b[p..e]` in a method's parameters: a type (before another word: a qualified
    /// one, its type arguments and `[]`s with it), or a name; `def` and `final` their own.
    #[inline(never)]
    fn param_word(&mut self, p: usize, e: usize, w: u16) -> usize {
        let b = self.b;
        self.prev = P_NAME;
        let leaf = w & W_LEAF;
        if matches!(leaf, DEF | FINAL) {
            self.put(p, e, leaf);
            return e;
        }
        // the type's extent: `a.b.C`, `<…>`, `[]`s
        let mut t = e;
        while b.get(t) == Some(&b'.') && is_word_starting(b, t + 1) {
            t = word_end(b, t + 1);
        }
        if b.get(t) == Some(&b'<')
            && let Some(a) = angle_end(b, t)
        {
            t = a;
        }
        loop {
            let s = skip_spaces(b, t);
            if b.get(s) == Some(&b'[') && b.get(skip_spaces(b, s + 1)) == Some(&b']') {
                t = skip_spaces(b, s + 1) + 1;
            } else {
                break;
            }
        }
        let mut a = skip_spaces(b, t);
        if b[a..].starts_with(b"...") {
            a = skip_spaces(b, a + 3);
        }
        if is_word_starting(b, a) || b[p].is_ascii_uppercase() || leaf == PRIMITIVE {
            self.put(p, t, PARAM_TYPE);
            return t;
        }
        self.put(p, e, PARAMETER);
        e
    }

    /// A qualified class name from the lowercase word `b[p..e]` before `.`: bat's
    /// `(?:[a-z]\w*\.)*[A-Z]\w+`, one span (`java.util.List`, `groovy.transform.Field`), but for
    /// one name before a constant's (`env.BUILD_ID`: a property, not a class); `None` (and
    /// [`Scan::chain_end`] past the names looked at) where none is.
    #[cold]
    #[inline(never)]
    fn chain(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        let mut q = e;
        let mut names = 1;
        while b.get(q) == Some(&b'.') {
            let s = q + 1;
            let Some(&c) = b.get(s) else {
                break;
            };
            if c.is_ascii_uppercase() {
                let f = word_end(b, s);
                if f - s >= 2 && word_len_ascii(b, s, f) == f - s {
                    if names == 1 && is_constant(&b[s..f]) {
                        break;
                    }
                    self.put(p, f, CLASS);
                    self.prev = P_NAME;
                    return Some(f);
                }
                break;
            }
            if !c.is_ascii_lowercase() {
                break;
            }
            q = word_end(b, s);
            names += 1;
        }
        self.chain_end = q;
        None
    }

    /// `@` at `p`: an annotation's name (`@A`, `@a.b.C`), its arguments after `(`.
    #[inline(never)]
    fn at_sign(&mut self, p: usize) -> usize {
        let b = self.b;
        let mut e = p + 1;
        while is_word_starting(b, e) {
            e = word_end(b, e);
            if b.get(e) == Some(&b'.') && is_word_starting(b, e + 1) {
                e += 1;
            } else {
                break;
            }
        }
        if e == p + 1 {
            self.prev = P_OP;
            return e;
        }
        self.put(p, e, ANNOTATION);
        if b.get(e) == Some(&b'(') {
            self.push(F_ANNOT);
            self.put(e, e + 1, ANNOT_BEGIN);
            self.prev = P_OP;
            return e + 1;
        }
        self.prev = P_START;
        e
    }

    /// `.` at `p`: an accessor (bat's `(?<=\S)\.(?=\S)`), a range, or a number's start.
    #[inline(always)]
    fn dot(&mut self, p: usize) -> usize {
        let b = self.b;
        match b.get(p + 1) {
            Some(b'.') => {
                if b.get(p + 2) == Some(&b'.') {
                    // varargs' `...`: plain, as bat's parameters take it
                    self.prev = P_OP;
                    return p + 3;
                }
                let n = if b.get(p + 2) == Some(&b'<') { 3 } else { 2 };
                return self.op_token(p, n, RANGE);
            }
            Some(c) if c.is_ascii_digit() && !matches!(self.prev, P_NAME | P_VALUE) => {
                return self.number(p);
            }
            Some(&c)
                if p > 0 && !matches!(c, b' ' | b'\t') && !matches!(b[p - 1], b' ' | b'\t') =>
            {
                self.put_hot(p, p + 1, DOT);
            }
            _ => {}
        }
        self.prev = P_DOT;
        p + 1
    }

    /// `n` bytes at `p` an operator of `kind`.
    #[inline(always)]
    fn op_token(&mut self, p: usize, n: usize, kind: u16) -> usize {
        self.put_hot(p, p + n, kind);
        self.prev = P_OP;
        p + n
    }

    /// `/` at `p`: a comment, a slashy string where an operand starts, or division.
    #[inline(always)]
    fn slash(&mut self, p: usize) -> usize {
        match self.b.get(p + 1) {
            Some(b'/') => self.line_comment(p),
            Some(b'*') => self.comment_open(p),
            _ if matches!(self.prev, P_START | P_OP | P_REGEX) => self.slashy(p),
            _ => self.op_token(p, 1, ARITH),
        }
    }

    /// `$` at `p`: a dollar-slashy string's `$/`, or a name's `$`.
    #[inline(always)]
    fn dollar(&mut self, p: usize) -> usize {
        if self.b.get(p + 1) == Some(&b'/') {
            return self.dollar_slashy(p);
        }
        self.prev = P_NAME;
        p + 1
    }

    /// An operator at `p` (`=-+!*%<>&|?:~^`).
    #[inline(always)]
    fn op(&mut self, p: usize) -> usize {
        let b = self.b;
        let c = b[p];
        let c1 = b.get(p + 1).copied().unwrap_or(0);
        match c {
            b'=' => match c1 {
                b'=' => match b.get(p + 2) {
                    Some(b'~') => {
                        self.op_token(p, 3, MATCH);
                        self.prev = P_REGEX;
                        p + 3
                    }
                    Some(b'=') => self.op_token(p, 3, COMPARE),
                    _ => self.op_token(p, 2, COMPARE),
                },
                b'~' => {
                    self.op_token(p, 2, FIND);
                    self.prev = P_REGEX;
                    p + 2
                }
                _ => {
                    if !SOL && self.top() == F_PARAMS {
                        // a parameter's default value
                        self.set_top(F_PDEFAULT);
                    }
                    self.op_token(p, 1, ASSIGN)
                }
            },
            b'!' => {
                if c1 == b'=' {
                    self.op_token(p, 2, COMPARE)
                } else {
                    self.op_token(p, 1, LOGICAL)
                }
            }
            b'<' => match c1 {
                b'=' => {
                    let n = if b.get(p + 2) == Some(&b'>') { 3 } else { 2 };
                    self.op_token(p, n, COMPARE)
                }
                b'<' => self.op_token(p, 2, LSHIFT),
                b'>' => self.op_token(p, 2, COMPARE),
                _ => self.op_token(p, 1, COMPARE),
            },
            b'>' => {
                if c1 == b'=' {
                    self.op_token(p, 2, COMPARE)
                } else {
                    self.op_token(p, 1, COMPARE)
                }
            }
            b'-' => match c1 {
                b'>' => {
                    if !SOL && self.top() == F_CASE {
                        // a switch expression's `case … ->`
                        self.pop();
                    }
                    self.op_token(p, 2, ARROW)
                }
                b'-' => {
                    self.put_hot(p, p + 2, INCDEC);
                    p + 2
                }
                b'0'..=b'9' => self.number(p),
                _ => self.op_token(p, 1, ARITH),
            },
            b'+' => match c1 {
                b'+' => {
                    self.put_hot(p, p + 2, INCDEC);
                    p + 2
                }
                b'0'..=b'9' => self.number(p),
                _ => self.op_token(p, 1, ARITH),
            },
            b'*' => {
                if c1 == b'.' {
                    // the spread `*.`: bat's `*` and accessor
                    self.put_hot(p, p + 1, ARITH);
                    self.put_hot(p + 1, p + 2, DOT);
                    self.prev = P_DOT;
                    return p + 2;
                }
                self.op_token(p, 1, ARITH)
            }
            b'%' => self.op_token(p, 1, ARITH),
            b'~' => {
                // bat's plain `~`: a pattern's operator
                self.prev = P_REGEX;
                p + 1
            }
            b'&' if c1 == b'&' => self.op_token(p, 2, LOGICAL),
            b'|' if c1 == b'|' => self.op_token(p, 2, LOGICAL),
            b'?' => match c1 {
                b':' => self.op_token(p, 2, ELVIS),
                b'.' if p > 0 && b.get(p + 2).is_some_and(|&c| c != b' ') => {
                    self.put_hot(p, p + 2, SAFE_DOT);
                    self.prev = P_DOT;
                    p + 2
                }
                _ => {
                    if !SOL {
                        self.push(F_TERN);
                    }
                    self.op_token(p, 1, TERNARY)
                }
            },
            b':' if !SOL => self.colon(p),
            _ => {
                self.prev = P_OP;
                p + 1
            }
        }
    }

    /// `:` at `p`: a ternary's, a case's or an assert's separator; plain elsewhere.
    #[inline(always)]
    fn colon(&mut self, p: usize) -> usize {
        self.prev = P_OP;
        match self.top() {
            F_TERN => {
                self.pop();
                self.put(p, p + 1, TERNARY_SEP);
            }
            F_CASE => {
                self.pop();
                self.put(p, p + 1, CASE_END);
                self.prev = P_START;
            }
            F_ASSERT => self.put(p, p + 1, ASSERT_SEP),
            _ => {}
        }
        p + 1
    }

    /// A number at `p` (a digit, `.` before one, or a sign before one).
    #[inline(always)]
    fn number(&mut self, p: usize) -> usize {
        let e = number_end(self.b, p);
        self.put_hot(p, e, NUMBER);
        self.prev = P_VALUE;
        e
    }
}

/// Whether the call named `b[s..e]`, its `(` or its string right after, takes SQL first:
/// groovy.sql.Sql's `rows`, `eachRow`, `firstRow`, `executeInsert` on a receiver; its `execute`,
/// `executeUpdate`, `query` on a receiver named as a database's (`sql`, `db`…); the JVM's
/// ([`sql::jvm_call`]).
fn sql_call(b: &[u8], s: usize, e: usize) -> bool {
    let name = &b[s..e];
    let dotted = s > 0 && b[s - 1] == b'.';
    match name {
        b"rows" | b"eachRow" | b"firstRow" | b"executeInsert" => dotted,
        b"execute" | b"executeUpdate" | b"query" => dotted && sql_receiver(b, s - 1),
        _ => sql::jvm_call(b, s, e),
    }
}

/// Whether the receiver before the `.` at `d` is named as a database's: `sql`, `db`, a name
/// ending in `Sql`, `sql`, `Db`, `db`, or one [`sql::db_receiver`] takes.
fn sql_receiver(b: &[u8], d: usize) -> bool {
    let mut s = d;
    while s > 0 && WORD[usize::from(b[s - 1])] {
        s -= 1;
    }
    let r = &b[s..d];
    r.ends_with(b"sql")
        || r.ends_with(b"Sql")
        || r.ends_with(b"db")
        || r.ends_with(b"Db")
        || r.ends_with(b"SQL")
        || sql::db_receiver(b, d + 1)
}

/// Whether the call named `b[s..e]` takes a regex first: `Pattern.compile(`,
/// `Pattern.matches(`, a string's `.matches(`, `.replaceAll(`, `.replaceFirst(`.
fn regex_call(b: &[u8], s: usize, e: usize) -> bool {
    let name = &b[s..e];
    match name {
        b"compile" => b[..s].ends_with(b"Pattern."),
        b"matches" | b"replaceAll" | b"replaceFirst" => s > 0 && b[s - 1] == b'.',
        _ => false,
    }
}

/// Whether the word at `p` (`w` its info) may start a method's or a constructor's head: a
/// modifier, `def`, `void`, a primitive, a capitalized name.
fn is_decl_word(b: &[u8], p: usize, w: u16) -> bool {
    matches!(
        w & W_LEAF,
        ACCESS | STATIC | FINAL | MODIFIER | DEF | PRIMITIVE
    ) || w & W_ACTION == W_VOID
        || (w & (W_LEAF | W_ACTION) == 0 && b[p].is_ascii_uppercase())
}

/// Where the type arguments a `<` at `q` opens close (past their `>`): a type's bytes only
/// (names, `.`, `,`, `?`, `&`, `[]`, blanks, nested `<…>`), on the line; `None` for none.
fn angle_end(b: &[u8], q: usize) -> Option<usize> {
    let mut depth = 0u32;
    let mut i = q;
    while i < b.len() {
        match b[i] {
            b'<' => depth += 1,
            b'>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            b',' | b'.' | b'?' | b'&' | b'[' | b']' | b' ' | b'\t' => {}
            c if WORD[usize::from(c)] || c >= 0x80 => {}
            _ => return None,
        }
        i += 1;
    }
    None
}

/// Where a class type at `q` ends: bat's `(?:\w+\.)*[A-Z]\w+`, then type arguments and `[]`s;
/// `None` where none starts.
fn class_type_end(b: &[u8], q: usize) -> Option<usize> {
    let mut s = q;
    let mut e;
    loop {
        e = word_end(b, s);
        if e == s {
            return None;
        }
        if b.get(e) == Some(&b'.') && is_word_starting(b, e + 1) {
            s = e + 1;
        } else {
            break;
        }
    }
    if !b[s].is_ascii_uppercase() || e - s < 2 {
        return None;
    }
    if b.get(e) == Some(&b'<') {
        e = angle_end(b, e)?;
    }
    loop {
        let a = skip_spaces(b, e);
        if b.get(a) == Some(&b'[') && b.get(skip_spaces(b, a + 1)) == Some(&b']') {
            e = skip_spaces(b, a + 1) + 1;
        } else {
            return Some(e);
        }
    }
}

/// Where a type in an `implements` or `throws` list at `s` ends: bat's
/// `(?:[a-z]\w*.)*[A-Z]\w*`; `None` where none starts.
fn qualified_type(b: &[u8], s: usize) -> Option<usize> {
    let mut q = s;
    loop {
        let c = *b.get(q)?;
        if c.is_ascii_uppercase() {
            return Some(word_end(b, q));
        }
        if !c.is_ascii_lowercase() {
            return None;
        }
        let e = word_end(b, q);
        if b.get(e) != Some(&b'.') {
            return None;
        }
        q = e + 1;
    }
}

/// Where the run of ASCII word bytes from `p` ends, as a length from `p`, within `..e`.
#[inline(always)]
fn word_len_ascii(b: &[u8], p: usize, e: usize) -> usize {
    let mut q = p;
    while q < e && b[q] < 0x80 {
        q += 1;
    }
    q - p
}

/// `[A-Z][A-Z0-9_]+`, a whole word: a constant's name.
fn is_constant(w: &[u8]) -> bool {
    w.len() >= 2
        && w[0].is_ascii_uppercase()
        && w[1..]
            .iter()
            .all(|&c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
}

/// Where Groovy's number at `p` ends (a digit, `.` before one, or a sign before one):
/// `1_000`, `0x1F`, `0b1`, `017`, `1.5e3`, and the suffixes `G`, `L`, `I`, `D`, `F`, either case.
fn number_end(b: &[u8], mut p: usize) -> usize {
    let n = b.len();
    if matches!(b[p], b'-' | b'+') {
        p += 1;
    }
    let digits = |mut i: usize, ok: fn(u8) -> bool| {
        while i < n && (ok(b[i]) || b[i] == b'_') {
            i += 1;
        }
        i
    };
    let mut e;
    if b[p] == b'0' && matches!(b.get(p + 1), Some(b'x' | b'X')) {
        e = digits(p + 2, |c| c.is_ascii_hexdigit());
    } else if b[p] == b'0' && matches!(b.get(p + 1), Some(b'b' | b'B')) {
        e = digits(p + 2, |c| matches!(c, b'0' | b'1'));
    } else {
        e = if b[p] == b'.' {
            p
        } else {
            digits(p, |c| c.is_ascii_digit())
        };
        if b.get(e) == Some(&b'.') && b.get(e + 1).is_some_and(u8::is_ascii_digit) {
            e = digits(e + 1, |c| c.is_ascii_digit());
        }
        if matches!(b.get(e), Some(b'e' | b'E')) {
            let mut s = e + 1;
            if matches!(b.get(s), Some(b'+' | b'-')) {
                s += 1;
            }
            if b.get(s).is_some_and(u8::is_ascii_digit) {
                e = digits(s, |c| c.is_ascii_digit());
            }
        }
    }
    if matches!(
        b.get(e),
        Some(b'g' | b'G' | b'l' | b'L' | b'i' | b'I' | b'd' | b'D' | b'f' | b'F')
    ) && !is_word(b, e + 1)
    {
        e += 1;
    }
    e
}
