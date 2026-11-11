//! Scala on a scanner of ada's own (X-137), from a floor probe (the same file built with
//! `ADA_PARSE_X137_SOL`): one loop over a line's bytes, a byte's action one load from a table
//! ([`ACT`]), a name read once and looked up once in one table of every word ([`WORDS`]: its leaf
//! and, for the words whose meaning hangs on where they stand, an action), the previous
//! significant token in a byte ([`Scan::prev`]), and frames only for what nests ([`Scan::frames`],
//! five bits a level): parentheses, braces, type arguments, a declaration's parameters and type
//! parameters, a type's parentheses, a case's pattern, a for's enumerators, a val's pattern, an
//! import's selectors, an annotation's arguments, the calls whose strings are SQL, and a string's
//! `${…}`. A declaration's head (after `def`, `val`, `class`, `type`, `new`, `import`, a
//! parameter list's `)`) is a byte of the line's ([`Scan::head`]), a declared type the depth it
//! opened at ([`Scan::ty`]). A `"""` string and a block comment continued past a line's end are a
//! mode ([`Scan::mode`]). Kinds are bat's leaves.
//!
//! Scala's reading where bat's grammar has none or derails: `<` is an XML literal's only where
//! Scala's scanner opens one (after a blank, `(` or `{`, before a name; bat's opens one at any
//! `<` before a letter, `a<b`, and stays in XML to the next tag); a `"` string ends with its line;
//! Scala 3's keywords (`given`, `using`, `extension`, `enum`, `export`, `then`, `end`, `derives`,
//! and the soft modifiers) where they act; `1_000` a number; `$a$b` two interpolations. SQL in
//! doobie's, Slick's and ScalikeJDBC's interpolators (`sql"…"`, `fr"…"`, `sqlu"…"`), Anorm's
//! `SQL("…")`, Spark's `spark.sql("…")` and the JVM's calls ([`crate::sql`]); regexes in
//! `"…".r`, `Regex("…")`, `Pattern.compile("…")` ([`crate::regex`], Java's dialect).

#[cfg(test)]
mod tests;
mod text;
mod xml;

use crate::emit::{Mark, Rules, Run, Runs, Sink, Spans};
use crate::prim::{char_len, is_word, is_word_starting, skip_spaces};
use crate::regex;
use crate::sql;
use crate::words::{Words, text_len};
use crate::{Out, State};

/// The floor probe (X-137's `sol`): every word its table leaf, strings, comments and brackets; no
/// declaration, parameter, pattern, type, lambda, SQL or regex read.
const SOL: bool = option_env!("ADA_PARSE_X137_SOL").is_some();

/// The leaves below the strings': a token's scopes, bat's.
static LEAVES: [&str; STR_BASE as usize] = [
    "",
    "comment.line.double-slash.scala",
    "comment.line.double-slash.scala punctuation.definition.comment.scala",
    "comment.block.scala",
    "comment.block.documentation.scala",
    "comment.block.documentation.scala punctuation.definition.comment.scala",
    "comment.block.documentation.scala keyword.other.documentation.scaladoc.scala",
    "comment.block.documentation.scala keyword.other.documentation.scaladoc.link.scala",
    "comment.block.empty.scala",
    "comment.block.documentation.scala comment.block.scala",
    "keyword.other.import.scala",
    "keyword.control.flow.scala",
    "keyword.control.flow.jump.scala",
    "keyword.control.exception.scala",
    "keyword.other.scala",
    "keyword.declaration.scala",
    "keyword.other.declaration.scala",
    "keyword.control.scala",
    "keyword.operator.comparison.scala",
    "keyword.operator.assignment.scala",
    "keyword.operator.arrow.scala",
    "keyword.operator.bound.scala",
    "keyword.operator.varargs.scala",
    "keyword.operator.at.scala",
    "storage.type.class.scala",
    "storage.type.function.scala",
    "storage.type.function.arrow.lambda.scala",
    "storage.type.function.arrow.case.scala",
    "storage.type.scala",
    "storage.type.volatile.scala",
    "storage.type.stable.scala",
    "storage.type.primitive.scala",
    "storage.modifier.access.scala",
    "storage.modifier.other.scala",
    "entity.name.function.scala",
    "entity.name.class.scala",
    "entity.name.type.scala",
    "entity.name.namespace.header.scala",
    "entity.name.namespace.scoped.scala",
    "variable.parameter.scala",
    "variable.other.constant.scala",
    "variable.other.readwrite.scala",
    "variable.language.underscore.scala",
    "variable.language.scala",
    "variable.language.hole.scala",
    "meta.annotation.identifier.scala variable.annotation.scala",
    "support.type.scala",
    "constant.language.scala",
    "constant.numeric.integer.decimal.scala",
    "constant.numeric.integer.hexadecimal.scala",
    "constant.numeric.integer.hexadecimal.scala punctuation.definition.numeric.base.scala",
    "constant.numeric.float.decimal.scala",
    "constant.numeric.float.decimal.scala punctuation.separator.decimal.scala",
    "constant.numeric.integer.decimal.scala storage.type.numeric.scala",
    "constant.numeric.float.decimal.scala storage.type.numeric.scala",
    "constant.numeric.integer.hexadecimal.scala storage.type.numeric.scala",
    "constant.character.literal.scala",
    "constant.other.symbol.scala",
    "punctuation.section.group.begin.scala",
    "punctuation.section.group.end.scala",
    "punctuation.section.block.begin.scala",
    "punctuation.section.block.end.scala",
    "punctuation.definition.generic.begin.scala",
    "punctuation.definition.generic.end.scala",
    "punctuation.accessor.scala",
    "punctuation.accessor.dot.scala",
    "punctuation.ascription.scala",
    "punctuation.separator.scala",
    "punctuation.terminator.scala",
    "meta.annotation.scala punctuation.definition.annotation.scala",
    "punctuation.definition.identifier.scala",
    "punctuation.section.arguments.annotation.begin.scala",
    "punctuation.section.arguments.annotation.end.scala",
    "punctuation.separator.arguments.annotation.scala",
    "text.xml",
    "text.xml punctuation.definition.tag.begin.xml",
    "meta.annotation.identifier.scala",
    "meta.annotation.identifier.scala punctuation.accessor.scala",
    "text.xml meta.tag.xml entity.name.tag.xml",
    "text.xml meta.tag.xml punctuation.definition.tag.end.xml",
    "text.xml meta.tag.xml entity.other.attribute-name.localname.xml",
    "text.xml meta.tag.xml punctuation.separator.key-value.xml",
    "text.xml meta.tag.xml string.quoted.double.xml",
    "text.xml meta.tag.xml string.quoted.double.xml punctuation.definition.string.begin.xml",
    "text.xml meta.tag.xml string.quoted.double.xml punctuation.definition.string.end.xml",
    "punctuation.definition.inline.begin.xml",
    "punctuation.definition.inline.end.xml",
    "text.xml comment.block.xml",
    "text.xml comment.block.xml punctuation.definition.comment.begin.xml",
    "text.xml comment.block.xml punctuation.definition.comment.end.xml",
    "text.xml constant.character.entity.xml",
    "text.xml string.unquoted.cdata.xml",
    "text.xml meta.tag.xml entity.other.attribute-name.namespace.xml",
    "text.xml meta.tag.xml entity.other.attribute-name.xml punctuation.separator.namespace.xml",
];

const PLAIN: u16 = 0;
const LINE_COMMENT: u16 = 1;
const LINE_MARK: u16 = 2;
const BLOCK_COMMENT: u16 = 3;
const DOC: u16 = 4;
const DOC_MARK: u16 = 5;
const DOC_TAG: u16 = 6;
const DOC_LINK: u16 = 7;
const EMPTY_COMMENT: u16 = 8;
const DOC_NESTED: u16 = 9;
const KW_IMPORT: u16 = 10;
const KW_FLOW: u16 = 11;
const KW_JUMP: u16 = 12;
const KW_EXCEPTION: u16 = 13;
const KW_OTHER: u16 = 14;
const KW_DECL: u16 = 15;
const KW_CASE: u16 = 16;
const KW_PACKAGE: u16 = 17;
const COMPARISON: u16 = 18;
const ASSIGN: u16 = 19;
const ARROW: u16 = 20;
const BOUND: u16 = 21;
const VARARGS: u16 = 22;
const AT: u16 = 23;
const ST_CLASS: u16 = 24;
const ST_FUNCTION: u16 = 25;
const LAMBDA_ARROW: u16 = 26;
const CASE_ARROW: u16 = 27;
const ST_TYPE: u16 = 28;
const ST_VAR: u16 = 29;
const ST_VAL: u16 = 30;
const PRIMITIVE: u16 = 31;
const MOD_ACCESS: u16 = 32;
const MOD_OTHER: u16 = 33;
const FUNCTION: u16 = 34;
const CLASS_NAME: u16 = 35;
const TYPE_NAME: u16 = 36;
const NAMESPACE: u16 = 37;
const NAMESPACE_SCOPED: u16 = 38;
const PARAMETER: u16 = 39;
const VAL_NAME: u16 = 40;
const VAR_NAME: u16 = 41;
const UNDERSCORE: u16 = 42;
const LANGUAGE_VAR: u16 = 43;
const HOLE: u16 = 44;
const ANNOTATION: u16 = 45;
const SUPPORT_TYPE: u16 = 46;
const CONSTANT: u16 = 47;
const INT: u16 = 48;
const HEX: u16 = 49;
const HEX_BASE: u16 = 50;
const FLOAT: u16 = 51;
const FLOAT_DOT: u16 = 52;
const INT_SUFFIX: u16 = 53;
const FLOAT_SUFFIX: u16 = 54;
const HEX_SUFFIX: u16 = 55;
const CHAR: u16 = 56;
const SYMBOL: u16 = 57;
const GROUP_BEGIN: u16 = 58;
const GROUP_END: u16 = 59;
const BLOCK_BEGIN: u16 = 60;
const BLOCK_END: u16 = 61;
const GENERIC_BEGIN: u16 = 62;
const GENERIC_END: u16 = 63;
const ACCESSOR: u16 = 64;
const ACCESSOR_DOT: u16 = 65;
const ASCRIPTION: u16 = 66;
const SEPARATOR: u16 = 67;
const TERMINATOR: u16 = 68;
const ANNOTATION_MARK: u16 = 69;
const IDENTIFIER_MARK: u16 = 70;
const ANNOT_BEGIN: u16 = 71;
const ANNOT_END: u16 = 72;
const ANNOT_SEPARATOR: u16 = 73;
const XML_TEXT: u16 = 74;
const XML_TAG_BEGIN: u16 = 75;
const ANNOT_PREFIX: u16 = 76;
const ANNOT_PREFIX_DOT: u16 = 77;
const XML_TAG_NAME: u16 = 78;
const XML_TAG_END: u16 = 79;
const XML_ATTR: u16 = 80;
const XML_EQ: u16 = 81;
const XML_STR: u16 = 82;
const XML_STR_BEGIN: u16 = 83;
const XML_STR_END: u16 = 84;
const XML_INLINE_BEGIN: u16 = 85;
const XML_INLINE_END: u16 = 86;
const XML_COMMENT: u16 = 87;
const XML_COMMENT_BEGIN: u16 = 88;
const XML_COMMENT_END: u16 = 89;
const XML_ENTITY: u16 = 90;
const XML_CDATA: u16 = 91;
const XML_NS: u16 = 92;
const XML_NS_SEP: u16 = 93;

/// The strings' leaves from here: a flavor's ([`DQ`]…) [`PARTS`] each, its text first.
const STR_BASE: u16 = 94;
/// A string's flavor: its scope (bat's `string.quoted.*`), [`STR_BASE`] plus its first part.
const DQ: u16 = STR_BASE;
const TQ: u16 = STR_BASE + PARTS;
const IQ: u16 = STR_BASE + 2 * PARTS;
const RIQ: u16 = STR_BASE + 3 * PARTS;
const TIQ: u16 = STR_BASE + 4 * PARTS;
/// A flavor's parts, each after its text.
const PARTS: u16 = 10;
const BEGIN: u16 = 1;
const END: u16 = 2;
const ESCAPE: u16 = 3;
const LONE: u16 = 4;
const VAR_MARK: u16 = 5;
const VAR: u16 = 6;
const EXPR: u16 = 7;
const FUNC: u16 = 8;
const FORMAT: u16 = 9;
static FLAVORS: [&str; 5] = [
    "string.quoted.double.scala",
    "string.quoted.triple.scala",
    "string.quoted.interpolated.scala",
    "string.quoted.raw.interpolated.scala",
    "string.quoted.triple.interpolated.scala",
];
static PART_SCOPES: [&str; PARTS as usize] = [
    "",
    "punctuation.definition.string.begin.scala",
    "punctuation.definition.string.end.scala",
    "constant.character.escape.scala",
    "invalid.illegal.lone-escape.scala",
    "punctuation.definition.variable.scala variable.other.scala",
    "variable.other.scala",
    "punctuation.definition.expression.scala",
    "support.function.scala",
    "constant.other.formatting.scala",
];
const STR_END: u16 = STR_BASE + 5 * PARTS;

/// The kinds of a regex in a `"…"` string and in a `"""…"""` one: the regex's, from each.
const RE_DQ: u16 = 256;
const RE_TQ: u16 = 384;
const _: () = assert!(STR_END <= RE_DQ);
const _: () = assert!(RE_DQ as usize + regex::KINDS <= RE_TQ as usize);
/// SQL in a string by flavor (`"…"`, `"""…"""`, `sql"…"`, `sql"""…"""`): SQL's kinds from each.
const SQL_DQ: u16 = 512;
const SQL_TQ: u16 = SQL_DQ + 128;
const SQL_IQ: u16 = SQL_DQ + 256;
const SQL_TIQ: u16 = SQL_DQ + 384;
const SQL_END: u16 = SQL_DQ + 512;
/// A kind's bit inside an annotation's arguments: under `meta.annotation.parameters.scala`, which
/// ten of the fourteen themes color.
const META: u16 = 1024;
const _: () = assert!(STR_BASE as usize == LEAVES.len());
const _: () = assert!(SQL_END <= META);
const _: () = assert!(RE_TQ as usize + regex::KINDS <= SQL_DQ as usize);
const _: () = assert!(sql::KINDS <= 128);

/// The scopes under `source.scala` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    let split = |s: &'static str| s.split(' ').filter(|s| !s.is_empty());
    if kind & META != 0 {
        if kind & !META >= SQL_END {
            return Vec::new();
        }
        let mut out = vec!["meta.annotation.parameters.scala"];
        out.extend(scopes(kind & !META));
        return out;
    }
    if kind >= SQL_END {
        return Vec::new();
    }
    if kind >= SQL_DQ {
        let (flavor, base) = match kind {
            k if k >= SQL_TIQ => (TIQ, SQL_TIQ),
            k if k >= SQL_IQ => (IQ, SQL_IQ),
            k if k >= SQL_TQ => (TQ, SQL_TQ),
            _ => (DQ, SQL_DQ),
        };
        if usize::from(kind - base) >= sql::KINDS {
            return Vec::new();
        }
        let mut out = scopes(flavor);
        out.extend(sql::scopes(kind - base));
        return out;
    }
    if kind >= RE_DQ {
        let (flavor, base) = if kind >= RE_TQ {
            (TQ, RE_TQ)
        } else {
            (DQ, RE_DQ)
        };
        let mut out = scopes(flavor);
        out.extend(regex::scopes(kind - base));
        return out;
    }
    if kind >= STR_END {
        return Vec::new();
    }
    if kind >= STR_BASE {
        let f = (kind - STR_BASE) / PARTS;
        let part = (kind - STR_BASE) % PARTS;
        let mut out: Vec<&str> = split(FLAVORS[usize::from(f)]).collect();
        out.extend(split(PART_SCOPES[usize::from(part)]));
        return out;
    }
    split(LEAVES[usize::from(kind)]).collect()
}

// A byte's action ([`ACT`]).
const A_OTHER: u8 = 0;
const A_SPACE: u8 = 1;
const A_NAME: u8 = 2;
const A_DIGIT: u8 = 3;
const A_DQUOTE: u8 = 4;
const A_SQUOTE: u8 = 5;
const A_BACKTICK: u8 = 6;
const A_SLASH: u8 = 7;
/// An operator's character but `/`, `.`, `@`: `!#%&*+-:<=>?\^|~`.
const A_OP: u8 = 8;
const A_LPAREN: u8 = 9;
const A_RPAREN: u8 = 10;
const A_LBRACE: u8 = 11;
const A_RBRACE: u8 = 12;
const A_LBRACKET: u8 = 13;
const A_RBRACKET: u8 = 14;
const A_COMMA: u8 = 15;
const A_SEMI: u8 = 16;
const A_AT: u8 = 17;
const A_DOT: u8 = 18;
/// A byte past ASCII: a name's character, an operator's (`⇒`, `←`), or another.
const A_WIDE: u8 = 19;

static ACT: [u8; 256] = {
    let mut t = [A_WIDE; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = match b {
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => A_SPACE,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' => A_NAME,
            b'0'..=b'9' => A_DIGIT,
            b'"' => A_DQUOTE,
            b'\'' => A_SQUOTE,
            b'`' => A_BACKTICK,
            b'/' => A_SLASH,
            b'!' | b'#' | b'%' | b'&' | b'*' | b'+' | b'-' | b':' | b'<' | b'=' | b'>' | b'?'
            | b'\\' | b'^' | b'|' | b'~' => A_OP,
            b'(' => A_LPAREN,
            b')' => A_RPAREN,
            b'{' => A_LBRACE,
            b'}' => A_RBRACE,
            b'[' => A_LBRACKET,
            b']' => A_RBRACKET,
            b',' => A_COMMA,
            b';' => A_SEMI,
            b'@' => A_AT,
            b'.' => A_DOT,
            _ => A_OTHER,
        };
        c += 1;
    }
    t
};

/// A Scala name's bytes in ASCII, `[A-Za-z0-9_$]`.
static NAME: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
        c += 1;
    }
    t
};

/// An operator's characters in ASCII, `!#%&*+-/:<=>?@\^|~`.
static OPCHAR: [bool; 256] = {
    let mut t = [false; 256];
    let s = b"!#%&*+-/:<=>?@\\^|~";
    let mut i = 0;
    while i < s.len() {
        t[s[i] as usize] = true;
        i += 1;
    }
    t
};

// What a word is ([`WORDS`], [`INFO`]): its leaf in the low byte, an action in the next six
// bits, and [`W_SQL`].
const W_ACTION: u16 = 0x3f << 8;
/// `def`: a function's head follows.
const W_DEF: u16 = 1 << 8;
/// `val`: a pattern of names follows.
const W_VAL: u16 = 2 << 8;
/// `var`: a name follows.
const W_VAR: u16 = 3 << 8;
/// `class`, `trait`, `object`, `enum`: a class's head.
const W_CLASS: u16 = 4 << 8;
/// `type`: an alias's head at a statement, `keyword.other` after `.`.
const W_TYPE: u16 = 5 << 8;
/// `case`: a pattern to `=>`, or `case class`.
const W_CASE: u16 = 6 << 8;
/// `import`, `export`: a path and selectors to the line's end.
const W_IMPORT: u16 = 7 << 8;
/// `package`: a name follows.
const W_PACKAGE: u16 = 8 << 8;
/// `new`: a type follows.
const W_NEW: u16 = 9 << 8;
/// `for`: enumerators follow.
const W_FOR: u16 = 10 << 8;
/// `extends`, `with`, `derives`: types follow.
const W_EXTENDS: u16 = 11 << 8;
/// `if`: a case's guard.
const W_IF: u16 = 12 << 8;
/// A soft modifier (`inline`, `opaque`, `transparent`, `open`, `infix`): a modifier before a
/// declaration's keyword or another modifier.
const W_SOFT: u16 = 13 << 8;
/// `given`: an instance's head at a statement, a name elsewhere.
const W_GIVEN: u16 = 14 << 8;
/// `using`: a modifier at a parameter list's start.
const W_USING: u16 = 15 << 8;
/// `extension`: a keyword before `(` or `[` at a statement.
const W_EXTENSION: u16 = 16 << 8;
/// `end`: a marker at a line's start before a name or a keyword and the line's end.
const W_END: u16 = 17 << 8;
/// `then`: Scala 3's keyword.
const W_THEN: u16 = 18 << 8;
/// `private`, `protected`: a qualifier in `[…]` may follow.
const W_ACCESS: u16 = 19 << 8;
/// `enum`: a class's head at a statement, a name elsewhere.
const W_ENUM: u16 = 20 << 8;
/// `implicit`: a modifier, and a lambda's parameter after it.
const W_IMPLICIT: u16 = 21 << 8;
/// `this`, `super`: a value.
const W_SELF: u16 = 22 << 8;
/// A name a SQL call can have ([`sql::jvm_call`], `SQL`, `Language`): the `(` after it looks
/// back.
const W_SQL: u16 = 1 << 15;

/// The words, each with what it is.
const WORD_LIST: &[(&[u8], u16)] = &[
    (b"return", KW_JUMP),
    (b"throw", KW_JUMP),
    (b"else", KW_FLOW),
    (b"if", W_IF | KW_FLOW),
    (b"do", KW_FLOW),
    (b"while", KW_FLOW),
    (b"for", W_FOR | KW_FLOW),
    (b"yield", KW_FLOW),
    (b"match", KW_FLOW),
    (b"then", W_THEN | KW_FLOW),
    (b"catch", KW_EXCEPTION),
    (b"finally", KW_EXCEPTION),
    (b"try", KW_EXCEPTION),
    (b"macro", KW_OTHER),
    (b"eq", COMPARISON),
    (b"ne", COMPARISON),
    (b"extends", W_EXTENDS | KW_DECL),
    (b"with", W_EXTENDS | KW_DECL),
    (b"derives", W_EXTENDS | KW_DECL),
    (b"forSome", KW_DECL),
    (b"end", W_END | KW_DECL),
    (b"class", W_CLASS | ST_CLASS),
    (b"trait", W_CLASS | ST_CLASS),
    (b"object", W_CLASS | ST_CLASS),
    (b"enum", W_ENUM | ST_CLASS),
    (b"def", W_DEF | ST_FUNCTION),
    (b"type", W_TYPE | ST_TYPE),
    (b"var", W_VAR | ST_VAR),
    (b"val", W_VAL | ST_VAL),
    (b"package", W_PACKAGE | KW_PACKAGE),
    (b"import", W_IMPORT | KW_IMPORT),
    (b"export", W_IMPORT | KW_IMPORT),
    (b"new", W_NEW | KW_OTHER),
    (b"case", W_CASE | KW_CASE),
    (b"private", W_ACCESS | MOD_ACCESS),
    (b"protected", W_ACCESS | MOD_ACCESS),
    (b"abstract", MOD_OTHER),
    (b"final", MOD_OTHER),
    (b"lazy", MOD_OTHER),
    (b"sealed", MOD_OTHER),
    (b"implicit", W_IMPLICIT | MOD_OTHER),
    (b"override", MOD_OTHER),
    (b"inline", W_SOFT | MOD_OTHER),
    (b"opaque", W_SOFT | MOD_OTHER),
    (b"transparent", W_SOFT | MOD_OTHER),
    (b"open", W_SOFT | MOD_OTHER),
    (b"infix", W_SOFT | MOD_OTHER),
    (b"given", W_GIVEN | MOD_OTHER),
    (b"using", W_USING | MOD_OTHER),
    (b"extension", W_EXTENSION | ST_FUNCTION),
    (b"this", W_SELF | LANGUAGE_VAR),
    (b"super", W_SELF | LANGUAGE_VAR),
    (b"true", CONSTANT),
    (b"false", CONSTANT),
    (b"null", CONSTANT),
    (b"Unit", PRIMITIVE),
    (b"Boolean", PRIMITIVE),
    (b"Byte", PRIMITIVE),
    (b"Char", PRIMITIVE),
    (b"Short", PRIMITIVE),
    (b"Int", PRIMITIVE),
    (b"Float", PRIMITIVE),
    (b"Long", PRIMITIVE),
    (b"Double", PRIMITIVE),
    (b"Any", PRIMITIVE),
    (b"AnyRef", PRIMITIVE),
    (b"AnyVal", PRIMITIVE),
    (b"Nothing", PRIMITIVE),
    // SQL's calls ([`sql::jvm_call`]), Anorm's `SQL(`, IntelliJ's `@Language(`
    (b"sql", W_SQL),
    (b"SQL", W_SQL),
    (b"Language", W_SQL),
    (b"execute", W_SQL),
    (b"executeQuery", W_SQL),
    (b"executeUpdate", W_SQL),
    (b"executeLargeUpdate", W_SQL),
    (b"execSQL", W_SQL),
    (b"prepareStatement", W_SQL),
    (b"prepareCall", W_SQL),
    (b"addBatch", W_SQL),
    (b"batchUpdate", W_SQL),
    (b"createQuery", W_SQL),
    (b"createNativeQuery", W_SQL),
    (b"createSQLQuery", W_SQL),
    (b"createUpdate", W_SQL),
    (b"createCall", W_SQL),
    (b"compileStatement", W_SQL),
    (b"queryForObject", W_SQL),
    (b"queryForList", W_SQL),
    (b"queryForMap", W_SQL),
    (b"query", W_SQL),
    (b"select", W_SQL),
    (b"update", W_SQL),
    (b"rawQuery", W_SQL),
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
/// A word's kind and action by its id; 0 for none.
static INFO: [u16; N + 1] = {
    let mut t = [0u16; N + 1];
    let mut i = 0;
    while i < N {
        t[i + 1] = WORD_LIST[i].1;
        i += 1;
    }
    t
};

/// Whether `leaf` is a keyword's: a word that ends a name's run (a parameter's, a pattern's).
fn is_keyword(leaf: u16) -> bool {
    matches!(
        leaf,
        KW_FLOW
            | KW_JUMP
            | KW_EXCEPTION
            | KW_OTHER
            | KW_DECL
            | KW_CASE
            | KW_IMPORT
            | KW_PACKAGE
            | ST_CLASS
            | ST_FUNCTION
            | ST_TYPE
            | ST_VAR
            | ST_VAL
    )
}

// The frames, five bits a level in [`Scan::frames`]; 0 is none (the top level, or a level lost
// past [`LEVELS`]).
/// `(` … `)` of code.
const F_PAREN: u8 = 1;
/// `{` … `}`: statements.
const F_BRACE: u8 = 2;
/// `[` … `]`: a type's arguments.
const F_TARGS: u8 = 3;
/// A declaration's or a lambda's parameters, where a name stands.
const F_PARAMS: u8 = 4;
/// A parameter's type, after its `:`.
const F_PTYPE: u8 = 5;
/// A parameter's default value, after its `=`.
const F_PDEFAULT: u8 = 6;
/// A declaration's type parameters, `[` … `]`, where a name stands.
const F_TPARAMS: u8 = 7;
/// A type parameter's bound, after `<:`, `>:` or `:`.
const F_TBOUND: u8 = 8;
/// `(` … `)` in a type: a function type's parameters, a tuple's.
const F_TPAREN: u8 = 9;
/// A string's `${` … `}`: code to its `}`, the string after ([`Scan::skinds`]).
const F_TMPL: u8 = 10;
/// A call whose first argument is SQL ([`sql::jvm_call`], `SQL(`), to its first `,`.
const F_SQLCALL: u8 = 11;
/// A case's pattern, from `case` to its `=>`.
const F_CASE: u8 = 12;
/// A case's guard, from its `if` to the `=>`.
const F_GUARD: u8 = 13;
/// `(` … `)` in a pattern (a case's, a val's, an enumerator's).
const F_PPAREN: u8 = 14;
/// A for's `(` … `)`, in an enumerator's pattern.
const F_FORP: u8 = 15;
/// A for's `(` … `)`, past an enumerator's `<-`, `=` or `if`.
const F_FORPX: u8 = 16;
/// A for's `{` … `}`, in an enumerator's pattern (a line's start).
const F_FORB: u8 = 17;
/// A for's `{` … `}`, past an enumerator's `<-`, `=` or `if`, to the line's end or a `;`.
const F_FORBX: u8 = 18;
/// An import's selectors, `{` … `}`.
const F_SELECT: u8 = 19;
/// An annotation's arguments, `(` … `)`.
const F_ANNOT: u8 = 20;
/// `{` … `}` in a type: a refinement.
const F_TBRACE: u8 = 21;
/// A lambda's parameters, `(` … `)` before `=>`, where a name stands.
const F_LPARAMS: u8 = 22;
/// A lambda's parameter's type, after its `:`.
const F_LPTYPE: u8 = 23;
/// An XML literal's element content, to its end tag.
const F_XML: u8 = 24;
/// An XML literal's start tag, to its `>` or `/>`.
const F_XTAG: u8 = 25;
/// An XML literal's `{` … `}`: code, the XML after.
const F_XEXPR: u8 = 26;
/// The levels [`Scan::frames`] holds: 24 of five bits in the 120 [`State`] keeps.
const LEVELS: u8 = 24;
const FRAME_BITS: u32 = 5;
const FRAME_MASK: u128 = (1 << (FRAME_BITS * LEVELS as u32)) - 1;

/// Whether frame `f` is a type's: its words colored as types.
fn is_type_frame(f: u8) -> bool {
    matches!(
        f,
        F_PTYPE | F_TBOUND | F_TARGS | F_TPAREN | F_TBRACE | F_LPTYPE
    )
}

/// Whether frame `f` is a pattern's: its lowercase names bound.
fn is_pattern_frame(f: u8) -> bool {
    matches!(f, F_CASE | F_PPAREN | F_FORP | F_FORB)
}

/// Whether frame `f` is closed by `)`.
fn is_paren_frame(f: u8) -> bool {
    matches!(
        f,
        F_PAREN
            | F_PARAMS
            | F_PTYPE
            | F_PDEFAULT
            | F_TPAREN
            | F_SQLCALL
            | F_PPAREN
            | F_FORP
            | F_FORPX
            | F_ANNOT
            | F_LPARAMS
            | F_LPTYPE
    )
}

/// Whether frame `f` is closed by `}`.
fn is_brace_frame(f: u8) -> bool {
    matches!(f, F_BRACE | F_FORB | F_FORBX | F_SELECT | F_TBRACE)
}

// What a line starts in ([`Scan::mode`]).
const M_CODE: u8 = 0;
/// A block comment, [`Scan::nest`] deep.
const M_COMMENT: u8 = 1;
/// A `"""` string, its kind in [`Scan::skinds`]' low nibble ([`SK_TQ`]…).
const M_TRIPLE: u8 = 2;
/// A `"""` string of SQL, SQL's context in [`Scan::ctx`], its kind in [`Scan::skinds`].
const M_TRIPLE_SQL: u8 = 3;
/// A `"""` string of a regex, the regex's state in [`Scan::ctx`].
const M_TRIPLE_RE: u8 = 4;
/// [`State::arg`]'s bit beside the mode: a SQL marker waits for its string.
const MARKED: u16 = 8;
/// [`State::arg`]'s shift to a carried head ([`Scan::carried`]), four bits.
const CARRIED: u16 = 4;
/// [`Scan::nest`]'s bit: the outermost comment is a documentation comment.
const NEST_DOC: u8 = 0x80;

// A string's kind ([`Scan::skinds`], a nibble a `${` open, the innermost lowest).
/// `"…"`: escapes.
const SK_DQ: u8 = 0;
/// `s"…"` and any `id"…"`: escapes and interpolations.
const SK_IQ: u8 = 1;
/// `f"…"`: escapes, interpolations and formats.
const SK_FQ: u8 = 2;
/// `raw"…"`: interpolations.
const SK_RQ: u8 = 3;
/// `"""…"""`: `\uXXXX`.
const SK_TQ: u8 = 4;
/// `id"""…"""`: `\uXXXX` and interpolations.
const SK_TIQ: u8 = 5;
/// `f"""…"""`: escapes, interpolations and formats (bat's interpolated flavor).
const SK_TFQ: u8 = 6;
/// `sql"…"`: SQL, interpolations.
const SK_SQLI: u8 = 7;
/// `sql"""…"""`: SQL, interpolations.
const SK_TSQLI: u8 = 8;
/// `"…"` of SQL (a call's argument).
const SK_SQL: u8 = 9;
/// `"""…"""` of SQL.
const SK_TSQL: u8 = 10;

/// Whether kind `sk` is a `"""` string's.
fn is_triple(sk: u8) -> bool {
    matches!(sk, SK_TQ | SK_TIQ | SK_TFQ | SK_TSQLI | SK_TSQL)
}

// What the previous significant token was ([`Scan::prev`]).
/// A line's or a statement's start: after `;`, `{`.
const P_START: u8 = 0;
/// A modifier or an annotation: a declaration may go on.
const P_MOD: u8 = 1;
/// A name or a word that ends a value (`this`, a type).
const P_NAME: u8 = 2;
/// A value's end: `)`, `]`, `}`, a number, a string.
const P_VALUE: u8 = 3;
/// An operator, `(`, `,`, a keyword: an operand may start.
const P_OP: u8 = 4;
/// `.`: a member's name follows.
const P_DOT: u8 = 5;
/// A lambda's parameters' end: `=>` is the lambda's arrow.
const P_LAMBDA: u8 = 6;

// A declaration's head on the line ([`Scan::head`]).
const H_NONE: u8 = 0;
/// After `def`: the name.
const H_DEF: u8 = 1;
/// After a function's name or a parameter list's `)`: `[` type parameters, `(` parameters, `:`
/// the result's type.
const H_DEF_NAMED: u8 = 2;
/// After `val`: names and patterns to `=` or `:`.
const H_VAL: u8 = 3;
/// After `var`: a name.
const H_VAR: u8 = 4;
/// After a class's name: `[` type parameters, modifiers, `(` parameters.
const H_CLASS: u8 = 5;
/// After `type`: the name.
const H_TYPE: u8 = 6;
/// After a type's name: `[` type parameters, bounds, `=`.
const H_TYPE_NAMED: u8 = 7;
/// After `import`: a path and selectors.
const H_IMPORT: u8 = 8;
/// After `new`, `extends`, `with`: a type.
const H_NEW: u8 = 9;
/// After `given`: an instance's name or type.
const H_GIVEN: u8 = 10;
/// After `extension`: `(` its parameter.
const H_EXTENSION: u8 = 11;
/// After `extends`, `with`, `derives` in a class's head: its parents, plain (bat's
/// `entity.other.inherited-class`).
const H_EXTENDS: u8 = 12;

/// How far a lambda's `(` and an XML literal look for their close.
const LOOK: usize = 512;

/// Lex `b`, a line of Scala without its `\n`, from `state`.
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

/// A block of Scala on one scan opened from `state` ([`crate::Carry`]).
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
    /// An open block comment's depth, [`NEST_DOC`] where the outermost is a documentation one.
    nest: u8,
    /// A `"""` string's SQL context ([`M_TRIPLE_SQL`]) or regex state ([`M_TRIPLE_RE`]).
    ctx: u8,
    /// The kinds of the strings whose `${` is open, a nibble each, the innermost lowest; a
    /// `"""` string's kind in the low nibble while [`M_TRIPLE`] holds.
    skinds: u32,
    /// A SQL marker waits for its string past the line ([`MARKED`]).
    marked: bool,
    /// A def's or a class's head the line ended in ([`H_DEF_NAMED`], [`H_CLASS`]), for the next
    /// line's parameters.
    carried: u8,
    /// The depth an annotation's arguments opened at, 0 for none: inside them every byte is
    /// [`META`]'s.
    annot: u8,
    /// [`META`] inside an annotation's arguments, else 0: the bit every write takes.
    meta: u16,
    /// Where the writes inside an annotation's arguments end, for the plain text after them.
    gap: usize,
    // The line's own, reset at its start:
    /// The previous significant token ([`P_START`]…).
    prev: u8,
    /// A declaration's head ([`H_NONE`]…).
    head: u8,
    /// In a declared type, after its `:` (a val's, a function's result), to its `=`, `{` or
    /// `;`: the depth it opened at plus one, 0 for none.
    ty: u16,
    /// Where a SQL marker's string may open from (`usize::MAX` for none).
    mark: usize,
    /// The last [`W_SQL`] word on the line, `start..end` (`end` `usize::MAX` for none).
    sqlw: (usize, usize),
    /// The last string opened on the line not as SQL or a regex: where its text starts, and
    /// where the sink's writing stood then, for `.r` after it ([`Scan::relex`]).
    opened: Option<(usize, Mark)>,
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
            nest: 0,
            ctx: 0,
            skinds: 0,
            marked: false,
            carried: H_NONE,
            annot: 0,
            meta: 0,
            gap: 0,
            prev: P_START,
            head: H_NONE,
            ty: 0,
            mark: usize::MAX,
            sqlw: (0, usize::MAX),
            opened: None,
            sql: sql::Embedded {
                is_end: usize::MAX,
                at: 0,
            },
            k,
            out,
        }
    }

    /// The scan's fields from `state`: the frames in its levels and the first two withs, the
    /// comment's depth, the string's context and the string kinds in the next six, the mode and
    /// the marker in [`State::arg`].
    #[inline(always)]
    fn load(&mut self, state: &State) {
        let mut w = [0u8; 16];
        w[..14].copy_from_slice(&state.levels);
        w[14] = state.withs[0];
        w[15] = state.withs[1];
        self.frames = u128::from_le_bytes(w);
        self.depth = state.depth;
        self.nest = state.withs[2];
        self.ctx = state.withs[3];
        self.skinds = u32::from_le_bytes([
            state.withs[4],
            state.withs[5],
            state.withs[6],
            state.withs[7],
        ]);
        self.mode = (state.arg & 7) as u8;
        self.marked = state.arg & MARKED != 0;
        self.carried = (state.arg >> CARRIED) as u8 & 15;
        self.annot = state.withs[8];
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
        s.withs[2] = self.nest;
        s.withs[3] = self.ctx;
        s.withs[4..8].copy_from_slice(&self.skinds.to_le_bytes());
        s.withs[8] = self.annot;
        s.depth = self.depth;
        s.arg = u16::from(self.mode)
            | if self.marked { MARKED } else { 0 }
            | u16::from(self.carried) << CARRIED;
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

    /// A write inside an annotation's arguments: the plain text before it and it under [`META`].
    #[cold]
    #[inline(never)]
    fn put_meta(&mut self, s: usize, e: usize, kind: u16) {
        if s > self.gap {
            self.k.merge_call(&mut self.out, self.gap, s, META);
        }
        self.k.merge_call(&mut self.out, s, e, kind | META);
        self.gap = self.gap.max(e);
    }

    /// The annotation's arguments end at `p` (they're closed, or the line ends inside them).
    #[cold]
    #[inline(never)]
    fn end_meta(&mut self, p: usize) {
        if self.meta != 0 && p > self.gap {
            self.k.merge_call(&mut self.out, self.gap, p, META);
        }
        self.gap = p;
        self.meta = 0;
    }

    /// A write inlined, for the common tokens.
    #[inline(always)]
    fn put_hot(&mut self, s: usize, e: usize, kind: u16) {
        if self.meta != 0 {
            return self.put_meta(s, e, kind);
        }
        self.k.merge(&mut self.out, s, e, kind);
    }

    #[inline(always)]
    fn top(&self) -> u8 {
        (self.frames & 31) as u8
    }

    /// Whether the scan is in a declared type ([`Scan::ty`]), not in a bracket inside it.
    #[inline(always)]
    fn in_type(&self) -> bool {
        self.ty == u16::from(self.depth) + 1
    }

    /// Whether a word here is a type's: in a type's frame, a declared type, or after `new`.
    #[inline(always)]
    fn typed(&self) -> bool {
        is_type_frame(self.top()) || self.in_type()
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
        // a def's or a class's head goes on to the next line's parameters
        self.head = self.carried;
        self.carried = H_NONE;
        self.ty = 0;
        self.mark = if self.marked { 0 } else { usize::MAX };
        self.sqlw = (0, usize::MAX);
        self.opened = None;
        if self.annot != 0 {
            self.meta = META;
            self.gap = 0;
        }
        let p = match self.mode {
            M_CODE if matches!(self.top(), F_XML | F_XTAG) => self.xml_rest(0),
            M_CODE => {
                if !SOL && matches!(self.top(), F_FORBX | F_FORB) {
                    // a for's next enumerator, where the line has its `<-` or `=` (bat's)
                    self.set_top(if enumerator_line(self.b) {
                        F_FORB
                    } else {
                        F_FORBX
                    });
                }
                0
            }
            M_COMMENT => self.comment(0),
            _ => self.triple_rest(),
        };
        if SHARED {
            self.code_shared(p);
        } else {
            self.code(p);
        }
        if self.meta != 0 {
            self.end_meta(self.b.len());
        }
        if !SOL {
            let n = self.b.len();
            self.marked = self.mark != usize::MAX && sql::marker_holds(self.b, self.mark, n);
            if matches!(self.head, H_DEF_NAMED | H_CLASS)
                && matches!(self.prev, P_NAME | P_VALUE)
                && self.mode == M_CODE
            {
                self.carried = self.head;
            }
            if self.top() == F_CASE && self.prev != P_OP && self.mode == M_CODE {
                // a case without its `=>` on the line, `case A, B` in an enum
                self.pop();
            }
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
                // a name's letter past ASCII: the name's arm, one copy of it
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
                A_BACKTICK => self.backtick(p),
                A_SLASH => match b.get(p + 1) {
                    Some(b'/') => self.line_comment(p),
                    Some(b'*') => self.comment_open(p),
                    _ => self.op_cold(p),
                },
                A_OP => self.op(p),
                A_DOT => self.dot(p),
                A_LPAREN => self.lparen(p),
                A_RPAREN => self.rparen(p),
                A_LBRACE => self.lbrace(p),
                A_RBRACE => self.rbrace(p),
                A_LBRACKET => self.lbracket(p),
                A_RBRACKET => self.rbracket(p),
                A_COMMA => self.comma(p),
                A_SEMI => {
                    if !matches!(self.top(), F_FORP | F_FORPX) {
                        self.put(p, p + 1, TERMINATOR);
                    }
                    self.semi();
                    p + 1
                }
                A_AT => self.at_sign(p),
                A_WIDE => self.wide(p),
                _ => {
                    self.prev = P_OP;
                    p + 1
                }
            };
        }
    }

    /// `;`: a statement's end.
    #[inline(always)]
    fn semi(&mut self) {
        self.head = H_NONE;
        self.ty = 0;
        self.prev = P_START;
        if !SOL {
            match self.top() {
                F_FORPX => self.set_top(F_FORP),
                F_FORBX => self.set_top(F_FORB),
                _ => {}
            }
        }
    }

    /// A byte past ASCII at `p` that starts no name: `⇒` and `←` as `=>` and `<-`, or another
    /// character.
    #[inline(never)]
    fn wide(&mut self, p: usize) -> usize {
        let b = self.b;
        match b.get(p..p + 3) {
            Some([0xe2, 0x87, 0x92]) => self.arrow(p, 3),
            Some([0xe2, 0x86, 0x90]) => self.left_arrow(p, 3),
            _ => {
                self.prev = P_OP;
                p + char_len(b[p])
            }
        }
    }

    /// `(` at `p`: what it opens by the head or frame it's in, a lambda's parameters, a SQL
    /// call's argument.
    #[inline(always)]
    fn lparen(&mut self, p: usize) -> usize {
        let b = self.b;
        let prev = self.prev;
        self.prev = P_OP;
        if SOL {
            self.put_hot(p, p + 1, GROUP_BEGIN);
            self.push(F_PAREN);
            return p + 1;
        }
        if b.get(p + 1) == Some(&b')')
            && !matches!(prev, P_NAME | P_VALUE)
            && self.head == H_NONE
            && !is_arrow_next(b, p + 2)
        {
            // `()`, the unit value
            self.put(p, p + 2, CONSTANT);
            self.prev = P_VALUE;
            return p + 2;
        }
        let top = self.top();
        let f = if is_type_frame(top) || (self.in_type() && prev != P_NAME) {
            F_TPAREN
        } else if is_pattern_frame(top) {
            F_PPAREN
        } else {
            match self.head {
                H_DEF | H_DEF_NAMED | H_CLASS | H_EXTENSION => {
                    self.head = if self.head == H_CLASS {
                        H_CLASS
                    } else {
                        H_DEF_NAMED
                    };
                    F_PARAMS
                }
                H_VAL => F_PPAREN,
                H_GIVEN if prev != P_NAME => {
                    self.head = H_DEF_NAMED;
                    F_PARAMS
                }
                _ => {
                    if matches!(self.head, H_NEW | H_EXTENDS) {
                        self.head = H_NONE;
                    }
                    if matches!(prev, P_NAME | P_VALUE) {
                        if self.sqlw.1 != usize::MAX && crate::trim_end(b, 0, p) == self.sqlw.1 {
                            self.sql_paren()
                        } else {
                            F_PAREN
                        }
                    } else if lambda_params(b, p) {
                        F_LPARAMS
                    } else {
                        F_PAREN
                    }
                }
            }
        };
        self.put_hot(p, p + 1, GROUP_BEGIN);
        self.push(f);
        p + 1
    }

    /// `)` at `p`: a parenthesis's, with any bracket left open above it.
    #[inline(always)]
    fn rparen(&mut self, p: usize) -> usize {
        self.prev = P_VALUE;
        if matches!(self.head, H_NEW | H_EXTENDS) {
            self.head = H_NONE;
        }
        let mut f = self.top();
        if !is_paren_frame(f) {
            if !matches!(f, F_TARGS | F_TPARAMS | F_TBOUND | F_CASE | F_GUARD) {
                // a stray one
                self.put_hot(p, p + 1, GROUP_END);
                return p + 1;
            }
            // what the parenthesis left open
            while matches!(f, F_TARGS | F_TPARAMS | F_TBOUND | F_CASE | F_GUARD) {
                self.pop();
                f = self.top();
            }
            if !is_paren_frame(f) {
                self.put_hot(p, p + 1, GROUP_END);
                return p + 1;
            }
        }
        if f == F_ANNOT {
            self.put(p, p + 1, ANNOT_END);
            self.pop();
            self.annot_closed(p + 1);
            self.prev = P_MOD;
            return p + 1;
        }
        self.put_hot(p, p + 1, GROUP_END);
        self.pop();
        if matches!(f, F_LPARAMS | F_LPTYPE) || (f == F_PAREN && is_arrow_next(self.b, p + 1)) {
            self.prev = P_LAMBDA;
        } else if matches!(f, F_PARAMS | F_PTYPE | F_PDEFAULT) && self.head != H_CLASS {
            self.head = H_DEF_NAMED;
        }
        p + 1
    }

    /// `{` at `p`: statements, a for's enumerators, an import's selectors, a refinement.
    #[inline(always)]
    fn lbrace(&mut self, p: usize) -> usize {
        if SOL {
            self.put_hot(p, p + 1, BLOCK_BEGIN);
            self.push(F_BRACE);
            self.prev = P_START;
            return p + 1;
        }
        let f = match self.head {
            H_IMPORT => F_SELECT,
            _ if self.typed() && self.prev != P_NAME => F_TBRACE,
            _ => F_BRACE,
        };
        self.put_hot(
            p,
            p + 1,
            if f == F_SELECT {
                GROUP_BEGIN
            } else {
                BLOCK_BEGIN
            },
        );
        self.push(f);
        if f != F_SELECT {
            self.head = H_NONE;
            self.ty = 0;
        }
        self.prev = P_START;
        p + 1
    }

    /// `}` at `p`: the innermost brace closed, what it left open dropped; a stray one nothing.
    #[inline(always)]
    fn rbrace(&mut self, p: usize) -> usize {
        self.prev = P_VALUE;
        let top = self.top();
        if top == F_TMPL {
            return self.template_close(p);
        }
        if top == F_XEXPR {
            return self.xml_expr_close(p);
        }
        if top == F_SELECT {
            self.put(p, p + 1, GROUP_END);
            self.pop();
            return p + 1;
        }
        self.head = H_NONE;
        self.ty = 0;
        if is_brace_frame(top) {
            self.put_hot(p, p + 1, BLOCK_END);
            self.pop();
        } else if let Some(e) = self.unwind_brace(p) {
            return e;
        }
        self.annot_closed(p + 1);
        p + 1
    }

    /// A `}` at `p` with frames above its brace or template: they're dropped, if one is open
    /// below them; a template's string goes on after it.
    #[cold]
    fn unwind_brace(&mut self, p: usize) -> Option<usize> {
        let levels = self.depth.min(LEVELS);
        let mut i = 0;
        while i < levels {
            let f = ((self.frames >> (FRAME_BITS * u32::from(i))) & 31) as u8;
            if is_brace_frame(f) {
                for _ in 0..=i {
                    self.pop();
                }
                break;
            }
            if f == F_TMPL || f == F_XEXPR {
                for _ in 0..i {
                    self.pop();
                }
                return Some(if f == F_TMPL {
                    self.template_close(p)
                } else {
                    self.xml_expr_close(p)
                });
            }
            i += 1;
        }
        self.put(p, p + 1, BLOCK_END);
        None
    }

    /// `[` at `p`: type parameters in a declaration's head, else a type's arguments.
    #[inline(always)]
    fn lbracket(&mut self, p: usize) -> usize {
        self.put_hot(p, p + 1, GENERIC_BEGIN);
        let prev = self.prev;
        self.prev = P_OP;
        if SOL {
            self.push(F_TARGS);
            return p + 1;
        }
        let f = match self.head {
            _ if self.typed() => F_TARGS,
            H_DEF_NAMED | H_CLASS | H_TYPE_NAMED | H_EXTENSION if self.top() != F_PARAMS => {
                F_TPARAMS
            }
            H_GIVEN if prev != P_NAME => {
                self.head = H_DEF_NAMED;
                F_TPARAMS
            }
            _ if self.top() == F_TPARAMS => F_TPARAMS,
            _ => F_TARGS,
        };
        self.push(f);
        p + 1
    }

    /// `]` at `p`.
    #[inline(always)]
    fn rbracket(&mut self, p: usize) -> usize {
        let end = if self.annot != 0 && self.annot == self.depth && self.top() == F_TARGS {
            ANNOT_END
        } else {
            GENERIC_END
        };
        self.put_hot(p, p + 1, end);
        self.prev = P_NAME;
        let mut f = self.top();
        while f == F_TPAREN {
            // a type's parenthesis left open
            self.pop();
            f = self.top();
        }
        if matches!(f, F_TARGS | F_TPARAMS | F_TBOUND) {
            self.pop();
        }
        self.annot_closed(p + 1);
        p + 1
    }

    /// `,` at `p`: a parameter's, a type parameter's, a SQL call's first argument's end.
    #[inline(always)]
    fn comma(&mut self, p: usize) -> usize {
        self.prev = P_OP;
        if self.head == H_NEW {
            self.head = H_NONE;
        }
        let top = self.top();
        if top == F_ANNOT {
            self.put(p, p + 1, ANNOT_SEPARATOR);
            return p + 1;
        }
        if SOL {
            self.put_hot(p, p + 1, SEPARATOR);
            return p + 1;
        }
        if top != F_LPARAMS {
            // bat leaves an untyped lambda's `,` plain
            self.put_hot(p, p + 1, SEPARATOR);
        }
        match top {
            F_PTYPE | F_PDEFAULT => self.set_top(F_PARAMS),
            F_LPTYPE => self.set_top(F_LPARAMS),
            F_TBOUND => self.set_top(F_TPARAMS),
            F_SQLCALL => self.set_top(F_PAREN),
            _ => {}
        }
        if self.in_type() && self.head != H_VAL {
            self.ty = 0;
        }
        p + 1
    }

    /// A name at `p` (`[A-Za-z_$]`, or a letter past ASCII): its end, with an operator's
    /// characters after a final `_` (`unary_!`, `x_=`).
    #[inline(always)]
    fn name_end(&self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut e = (p + char_len(b[p])).min(n);
        loop {
            e = name_run(b, e);
            if e < n && b[e] >= 0x80 && is_word(b, e) {
                e += char_len(b[e]);
                continue;
            }
            break;
        }
        if e > p + 1 && b[e - 1] == b'_' && e < n && OPCHAR[usize::from(b[e])] {
            while e < n && OPCHAR[usize::from(b[e])] {
                e += 1;
            }
        }
        e
    }

    /// A word at `p`: a keyword, a type, a declaration's name, a parameter, or a name.
    #[inline(always)]
    fn name(&mut self, p: usize) -> usize {
        let b = self.b;
        let e = self.name_end(p);
        if e < b.len() && b[e] == b'"' {
            return self.interpolated(p, e);
        }
        if e == p + 1 && b[p] == b'_' {
            return self.underscore(p);
        }
        let w = WORDS.value(&b[p..e], &INFO);
        let leaf = w & 0xff;
        if SOL {
            if leaf != 0 {
                self.put_hot(p, e, leaf);
            }
            self.prev = P_NAME;
            return e;
        }
        if w & W_SQL != 0 {
            self.sqlw = (p, e);
        }
        let top = self.top();
        if self.prev == P_DOT {
            if is_type_frame(top) || self.in_type() || self.head == H_NEW {
                // a type's path: `java.lang.Integer`, `jl.Double`
                return self.type_word(p, e, w);
            }
            // a member's name: `x.type` is a keyword's, `.eq(` a comparison, `ClassTag.Unit` a
            // primitive's (bat's)
            if w & W_ACTION == W_TYPE {
                self.put(p, e, KW_OTHER);
            } else if leaf == COMPARISON || leaf == PRIMITIVE {
                self.put(p, e, leaf);
            }
            self.prev = P_NAME;
            return e;
        }
        if top >= F_TARGS
            && !matches!(top, F_TMPL | F_SQLCALL | F_ANNOT | F_PDEFAULT | F_GUARD)
            && let Some(r) = self.frame_word(p, e, w, top)
        {
            return r;
        }
        if self.head != H_NONE {
            return self.head_word(p, e, w);
        }
        if self.in_type() {
            return self.type_word(p, e, w);
        }
        if w & W_ACTION != 0 {
            return self.word_action(p, e, w);
        }
        if leaf != 0 {
            self.put_hot(p, e, leaf);
            self.prev = if matches!(leaf, MOD_OTHER | MOD_ACCESS) {
                P_MOD
            } else if is_keyword(leaf) || leaf == COMPARISON {
                P_OP
            } else {
                P_NAME
            };
            return e;
        }
        self.plain_name(p, e)
    }

    /// A name with no word of its own at `p..e`: a lambda's parameter before `=>`, else plain.
    #[inline(always)]
    fn plain_name(&mut self, p: usize, e: usize) -> usize {
        if matches!(self.prev, P_START | P_OP) && is_arrow_next(self.b, e) && self.top() != F_GUARD
        {
            self.put(p, e, PARAMETER);
            self.prev = P_LAMBDA;
            return e;
        }
        self.prev = P_NAME;
        e
    }

    /// `_` at `p`: the underscore, `_*` varargs in a pattern or an argument.
    #[inline(always)]
    fn underscore(&mut self, p: usize) -> usize {
        let b = self.b;
        if !SOL
            && (self.in_type() || is_pattern_frame(self.top()))
            && b.get(skip_spaces(b, p + 1)) == Some(&b'*')
        {
            let q = skip_spaces(b, p + 1);
            if !b.get(q + 1).is_some_and(|&c| OPCHAR[usize::from(c)]) {
                self.put(p, q + 1, VARARGS);
                self.prev = P_VALUE;
                return q + 1;
            }
        }
        self.put_hot(p, p + 1, UNDERSCORE);
        self.prev = if !SOL && matches!(self.prev, P_START | P_OP) && is_arrow_next(b, p + 1) {
            P_LAMBDA
        } else {
            P_NAME
        };
        p + 1
    }

    /// A word `b[p..e]` (`w` its info) in a frame whose names read apart (parameters, type
    /// parameters, types, patterns, a for's enumerators, an import's selectors); `None` where
    /// it reads as in code.
    #[inline(never)]
    fn frame_word(&mut self, p: usize, e: usize, w: u16, top: u8) -> Option<usize> {
        let leaf = w & 0xff;
        match top {
            F_PARAMS => {
                let word_next = || {
                    let q = skip_spaces(self.b, e);
                    q > e
                        && self.b.get(q).is_some_and(|&c| {
                            ACT[usize::from(c)] == A_NAME || c == b'`' || c >= 0x80
                        })
                };
                let kind = match w & W_ACTION {
                    W_VAL if word_next() => ST_TYPE,
                    W_VAR if word_next() => ST_VAR,
                    W_USING | W_IMPLICIT | W_SOFT | W_GIVEN if word_next() => MOD_OTHER,
                    W_ACCESS if word_next() => MOD_ACCESS,
                    _ if leaf == MOD_OTHER && word_next() => MOD_OTHER,
                    _ => {
                        if is_colon_next(self.b, e) {
                            PARAMETER
                        } else {
                            // bat's parameter is a name before `:`
                            leaf
                        }
                    }
                };
                if kind != PLAIN {
                    self.put(p, e, kind);
                }
                self.prev = if matches!(kind, MOD_OTHER | MOD_ACCESS | ST_TYPE | ST_VAR) {
                    P_MOD
                } else {
                    P_NAME
                };
                Some(e)
            }
            F_LPARAMS => {
                if leaf != 0 && w & W_ACTION != W_IMPLICIT {
                    return None;
                }
                let kind = if w & W_ACTION == W_IMPLICIT {
                    MOD_OTHER
                } else {
                    PARAMETER
                };
                self.put(p, e, kind);
                self.prev = P_NAME;
                Some(e)
            }
            F_TPARAMS => {
                let kind = self.type_leaf(p, e, w);
                if kind != PLAIN {
                    self.put(p, e, kind);
                }
                self.prev = P_NAME;
                Some(e)
            }
            F_PTYPE | F_TBOUND | F_TARGS | F_TPAREN | F_TBRACE | F_LPTYPE => {
                if top == F_TBRACE && w & W_ACTION != 0 {
                    return None;
                }
                Some(self.type_word(p, e, w))
            }
            F_CASE | F_PPAREN | F_FORP | F_FORB => Some(self.pattern_word(p, e, w)),
            F_FORBX | F_FORPX => {
                if w & W_ACTION == W_IF {
                    self.put(p, e, KW_FLOW);
                    self.prev = P_OP;
                    return Some(e);
                }
                None
            }
            F_SELECT => {
                // `as` in a selector, Scala 3's rename
                if &self.b[p..e] == b"as" {
                    self.put(p, e, KW_OTHER);
                    self.prev = P_OP;
                } else {
                    self.prev = P_NAME;
                }
                Some(e)
            }
            _ => None,
        }
    }

    /// What a word `b[p..e]` is in a type: a primitive, a keyword (`with`, `forSome`), a
    /// lowercase name `support.type` (a path's, a type member's), `_`, `?`; an uppercase name
    /// plain.
    #[inline(always)]
    fn type_leaf(&self, p: usize, _e: usize, w: u16) -> u16 {
        let leaf = w & 0xff;
        if leaf == PRIMITIVE || leaf == KW_DECL || leaf == CONSTANT || leaf == LANGUAGE_VAR {
            return leaf;
        }
        if w & W_ACTION == W_TYPE {
            return KW_OTHER;
        }
        if is_lower_name(self.b, p) {
            SUPPORT_TYPE
        } else {
            PLAIN
        }
    }

    /// A word in a type.
    #[inline(always)]
    fn type_word(&mut self, p: usize, e: usize, w: u16) -> usize {
        let kind = self.type_leaf(p, e, w);
        if kind != PLAIN {
            self.put(p, e, kind);
        }
        self.prev = if kind == KW_DECL { P_OP } else { P_NAME };
        e
    }

    /// A word in a pattern (a case's, a val's, an enumerator's): a lowercase name a variable it
    /// binds (a parameter's color, a val's in a val's), but before `(` or `.` (an extractor, a
    /// path); an uppercase one plain; keywords and literals as themselves.
    #[inline(never)]
    fn pattern_word(&mut self, p: usize, e: usize, w: u16) -> usize {
        let b = self.b;
        let leaf = w & 0xff;
        if self.in_type() && w & W_ACTION != W_IF {
            return self.type_word(p, e, w);
        }
        self.prev = P_NAME;
        match w & W_ACTION {
            W_IF if matches!(self.top(), F_CASE | F_FORP | F_FORB) => {
                self.put(p, e, KW_FLOW);
                self.ty = 0;
                let t = self.top();
                self.set_top(match t {
                    F_CASE => F_GUARD,
                    F_FORP => F_FORPX,
                    _ => F_FORBX,
                });
                self.prev = P_OP;
                return e;
            }
            W_VAL => {
                self.put(p, e, ST_VAL);
                self.prev = P_MOD;
                return e;
            }
            W_CASE => {
                // `case` in a for's pattern
                self.put(p, e, KW_CASE);
                self.prev = P_OP;
                return e;
            }
            _ => {}
        }
        if leaf != 0 && leaf != MOD_OTHER && w & W_ACTION != W_END {
            // a keyword (`extends`, `with` in an enum's case), a literal, a primitive
            self.put(p, e, leaf);
            return e;
        }
        if is_lower_name(b, p) {
            let q = skip_spaces(b, e);
            let next = b.get(q).copied().unwrap_or(0);
            if next != b'(' && next != b'.' && b.get(e) != Some(&b'.') {
                let kind = if self.head == H_VAL {
                    VAL_NAME
                } else {
                    PARAMETER
                };
                self.put(p, e, kind);
            }
        }
        e
    }

    /// A word `b[p..e]` in a declaration's head: a function's, a val's, a class's or a type's
    /// name, or a keyword between.
    #[inline(never)]
    fn head_word(&mut self, p: usize, e: usize, w: u16) -> usize {
        let b = self.b;
        let leaf = w & 0xff;
        match self.head {
            H_DEF => {
                self.put(p, e, FUNCTION);
                self.head = H_DEF_NAMED;
                self.prev = P_NAME;
                e
            }
            H_VAL | H_VAR => {
                // a soft keyword is a name here (`val open = 2`)
                if leaf != 0
                    && !matches!(
                        w & W_ACTION,
                        W_SOFT | W_GIVEN | W_USING | W_ENUM | W_EXTENSION | W_END
                    )
                {
                    self.head = H_NONE;
                    return self.name_again(p, e, w);
                }
                let q = skip_spaces(b, e);
                if b.get(q) == Some(&b'(') || b.get(e) == Some(&b'.') {
                    // an extractor's name, `val Some(x) = …`
                    self.prev = P_NAME;
                    return e;
                }
                let kind = if self.head == H_VAR {
                    VAR_NAME
                } else if is_lower_name(b, p) || self.top() != F_PPAREN {
                    VAL_NAME
                } else {
                    PLAIN
                };
                self.put(p, e, kind);
                self.prev = P_NAME;
                e
            }
            H_TYPE => {
                self.put(p, e, TYPE_NAME);
                self.head = H_TYPE_NAMED;
                self.prev = P_NAME;
                e
            }
            H_IMPORT => {
                if &b[p..e] == b"as" && self.prev == P_NAME {
                    self.put(p, e, KW_OTHER);
                    self.prev = P_OP;
                } else {
                    self.prev = P_NAME;
                }
                e
            }
            H_EXTENDS => {
                if w & W_ACTION == W_EXTENDS {
                    self.put(p, e, KW_DECL);
                    self.prev = P_OP;
                    return e;
                }
                if leaf != 0 && leaf != PRIMITIVE {
                    self.head = H_NONE;
                    return self.name_again(p, e, w);
                }
                if leaf == PRIMITIVE {
                    self.put(p, e, leaf);
                }
                self.prev = P_NAME;
                e
            }
            H_NEW => {
                if w & W_ACTION == W_EXTENDS {
                    self.put(p, e, KW_DECL);
                    self.prev = P_OP;
                    return e;
                }
                if self.prev == P_NAME || (leaf != 0 && leaf != PRIMITIVE) {
                    // the type ended: `new A shouldBe x`, `new if`
                    self.head = H_NONE;
                    return if w & W_ACTION != 0 || leaf != 0 {
                        self.name_again(p, e, w)
                    } else {
                        self.plain_name(p, e)
                    };
                }
                let kind = self.type_leaf(p, e, w);
                if kind != PLAIN {
                    self.put(p, e, kind);
                }
                self.prev = P_NAME;
                e
            }
            H_GIVEN => {
                // a given's name before `:`, else its type
                if leaf != 0 && w & W_ACTION != W_USING {
                    self.head = H_NONE;
                    return self.name_again(p, e, w);
                }
                if is_colon_next(b, e) {
                    self.put(p, e, VAL_NAME);
                    self.head = H_DEF_NAMED;
                } else if is_lower_name(b, p) && matches!(b.get(e), Some(b'[' | b'(')) {
                    self.put(p, e, FUNCTION);
                    self.head = H_DEF_NAMED;
                } else {
                    let kind = self.type_leaf(p, e, w);
                    if kind != PLAIN {
                        self.put(p, e, kind);
                    }
                }
                self.prev = P_NAME;
                e
            }
            _ => {
                // after a name: a keyword or modifier reads as itself
                if leaf != 0 || w & W_ACTION != 0 {
                    let head = self.head;
                    if !(head == H_CLASS && matches!(leaf, MOD_ACCESS | MOD_OTHER)) {
                        self.head = H_NONE;
                    }
                    if w & W_ACTION == W_EXTENDS {
                        self.head = H_EXTENDS;
                        self.put(p, e, KW_DECL);
                        self.prev = P_OP;
                        return e;
                    }
                    return self.name_again(p, e, w);
                }
                self.prev = P_NAME;
                e
            }
        }
    }

    /// A word read again outside the head it ended.
    #[inline(always)]
    fn name_again(&mut self, p: usize, e: usize, w: u16) -> usize {
        if w & W_ACTION != 0 {
            return self.word_action(p, e, w);
        }
        let leaf = w & 0xff;
        if leaf != 0 {
            self.put(p, e, leaf);
        }
        self.prev = if matches!(leaf, MOD_OTHER | MOD_ACCESS) {
            P_MOD
        } else if leaf == PRIMITIVE || leaf == CONSTANT {
            P_NAME
        } else {
            P_OP
        };
        e
    }

    /// Whether a word ending at `e` stands where a declaration's modifier does: before another
    /// modifier, a declaration's keyword or `@`.
    #[inline(never)]
    fn modifies(&self, e: usize) -> bool {
        let b = self.b;
        let q = skip_spaces(b, e);
        let Some(&c) = b.get(q) else {
            return false;
        };
        if c == b'@' {
            return true;
        }
        if !c.is_ascii_alphabetic() {
            return false;
        }
        let w = WORDS.value(&b[q..self.name_end(q)], &INFO);
        matches!(
            w & 0xff,
            MOD_OTHER | MOD_ACCESS | ST_CLASS | ST_FUNCTION | ST_TYPE | ST_VAL | ST_VAR
        ) || w & W_ACTION == W_CASE
    }

    /// Whether the scan stands at a statement's start: the top level, a brace, after `;`, a
    /// modifier or an annotation.
    #[inline(always)]
    fn at_statement(&self, prev: u8) -> bool {
        matches!(self.top(), 0 | F_BRACE | F_TMPL) && matches!(prev, P_START | P_MOD)
    }

    /// A word `b[p..e]` whose meaning hangs on where it stands (`w`'s action).
    #[inline(never)]
    fn word_action(&mut self, p: usize, e: usize, w: u16) -> usize {
        let b = self.b;
        let leaf = w & 0xff;
        let prev = self.prev;
        self.prev = P_OP;
        match w & W_ACTION {
            W_DEF => {
                self.put(p, e, ST_FUNCTION);
                self.head = H_DEF;
                return self.op_name_after(e);
            }
            W_VAL => {
                self.put(p, e, ST_VAL);
                self.head = H_VAL;
            }
            W_VAR => {
                self.put(p, e, ST_VAR);
                self.head = H_VAR;
            }
            W_CLASS => {
                self.put(p, e, ST_CLASS);
                return self.class_name(e);
            }
            W_ENUM => {
                if self.at_statement(prev) && is_name_next(b, e) {
                    self.put(p, e, ST_CLASS);
                    return self.class_name(e);
                }
                self.prev = P_NAME;
            }
            W_TYPE => {
                self.put(p, e, ST_TYPE);
                self.head = H_TYPE;
            }
            W_CASE => {
                let q = skip_spaces(b, e);
                let next = &b[q..];
                if (next.starts_with(b"class") && !is_word(b, q + 5))
                    || (next.starts_with(b"object") && !is_word(b, q + 6))
                {
                    // bat's `case\s+` before `class`
                    self.put(p, q, ST_CLASS);
                    return q;
                }
                self.put(p, e, KW_CASE);
                self.push(F_CASE);
            }
            W_IMPORT => {
                if w & 0xff == KW_IMPORT && (b[p] == b'i' || self.at_statement(prev)) {
                    self.put(p, e, KW_IMPORT);
                    self.head = H_IMPORT;
                } else {
                    self.prev = P_NAME;
                }
            }
            W_PACKAGE => return self.package(p, e),
            W_NEW => {
                self.put(p, e, KW_OTHER);
                self.head = H_NEW;
            }
            W_FOR => {
                self.put(p, e, KW_FLOW);
                let q = skip_spaces(b, e);
                match b.get(q) {
                    Some(b'(') => {
                        self.put(q, q + 1, GROUP_BEGIN);
                        self.push(F_FORP);
                        return q + 1;
                    }
                    Some(b'{') => {
                        self.put(q, q + 1, BLOCK_BEGIN);
                        self.push(F_FORB);
                        self.prev = P_START;
                        return q + 1;
                    }
                    _ => {}
                }
            }
            W_EXTENDS => {
                self.put(p, e, KW_DECL);
                self.head = H_EXTENDS;
            }
            W_ACCESS => {
                self.put(p, e, MOD_ACCESS);
                self.prev = P_MOD;
                if b.get(e) == Some(&b'[') {
                    return self.qualifier(e);
                }
            }
            W_SOFT => {
                if self.modifies(e) || (self.top() == F_PARAMS) {
                    self.put(p, e, MOD_OTHER);
                    self.prev = P_MOD;
                } else {
                    self.prev = P_NAME;
                }
            }
            W_IMPLICIT => {
                self.put(p, e, MOD_OTHER);
                self.prev = P_MOD;
                if !self.at_statement(prev) || prev == P_START {
                    // `implicit x =>`, a lambda's
                    let q = skip_spaces(b, e);
                    if b.get(q).is_some_and(|&c| c.is_ascii_lowercase()) {
                        let r = self.name_end(q);
                        if is_arrow_next(b, r) {
                            self.put(q, r, PARAMETER);
                            self.prev = P_LAMBDA;
                            return r;
                        }
                    }
                }
            }
            W_GIVEN => {
                if self.at_statement(prev) || prev == P_START {
                    self.put(p, e, MOD_OTHER);
                    self.head = H_GIVEN;
                    self.prev = P_MOD;
                } else {
                    self.prev = P_NAME;
                }
            }
            W_USING => {
                self.prev = P_NAME;
            }
            W_EXTENSION => {
                let q = skip_spaces(b, e);
                if self.at_statement(prev) && matches!(b.get(q), Some(b'(' | b'[')) {
                    self.put(p, e, ST_FUNCTION);
                    self.head = H_EXTENSION;
                } else {
                    self.prev = P_NAME;
                }
            }
            W_END => {
                if prev == P_START && self.top() != F_PARAMS && end_marker(b, e) {
                    self.put(p, e, KW_DECL);
                    let q = skip_spaces(b, e);
                    let r = self.name_end(q);
                    let w2 = WORDS.value(&b[q..r], &INFO);
                    if w2 & 0xff != 0 {
                        self.put(q, r, w2 & 0xff);
                    }
                    self.prev = P_VALUE;
                    return r;
                }
                return self.plain_name(p, e);
            }
            W_THEN => {
                self.put(p, e, KW_FLOW);
            }
            W_IF => {
                self.put(p, e, KW_FLOW);
            }
            W_SELF => {
                self.put(p, e, LANGUAGE_VAR);
                self.prev = P_NAME;
            }
            _ => {
                if leaf != 0 {
                    self.put(p, e, leaf);
                }
                self.prev = P_NAME;
            }
        }
        e
    }

    /// After `def` at `e`: an operator's name (`def +(`, `def ::`) the function's.
    #[inline(always)]
    fn op_name_after(&mut self, e: usize) -> usize {
        let b = self.b;
        let q = skip_spaces(b, e);
        if b.get(q).is_some_and(|&c| OPCHAR[usize::from(c)])
            && !b[q..].starts_with(b"//")
            && !b[q..].starts_with(b"/*")
        {
            let mut r = q;
            while r < b.len() && OPCHAR[usize::from(b[r])] {
                r += 1;
            }
            self.put(q, r, FUNCTION);
            self.head = H_DEF_NAMED;
            self.prev = P_NAME;
            return r;
        }
        e
    }

    /// A class's name after its keyword at `e`, the blanks before it with it (bat's
    /// `\s+name`); its head after.
    #[inline(always)]
    fn class_name(&mut self, e: usize) -> usize {
        let b = self.b;
        let q = skip_spaces(b, e);
        self.head = H_CLASS;
        self.prev = P_NAME;
        if q > e || q < b.len() {
            let r = match b.get(q) {
                Some(b'`') => crate::prim::find_in(b, q + 1, b.len(), *b"`")
                    .saturating_add(1)
                    .min(b.len()),
                Some(&c) if ACT[usize::from(c)] == A_NAME || c >= 0x80 => self.name_end(q),
                _ => return q,
            };
            if q == e {
                return q;
            }
            self.put(e, r, CLASS_NAME);
            return r;
        }
        q
    }

    /// `package` at `p..e`: `package object X`, a scoped package `package a.b {`, or the file's
    /// package's name.
    #[cold]
    #[inline(never)]
    fn package(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        self.put(p, e, KW_PACKAGE);
        self.prev = P_OP;
        let s = skip_spaces(b, e);
        if s == e {
            return e;
        }
        if b[s..].starts_with(b"object") && !is_word(b, s + 6) {
            self.put(s, s + 6, ST_CLASS);
            return self.class_name(s + 6);
        }
        let mut q = s;
        while q < b.len() && (NAME[usize::from(b[q])] || b[q] == b'.' || b[q] >= 0x80) {
            q += 1;
        }
        if q == s {
            return s;
        }
        let r = skip_spaces(b, q);
        let kind = if b.get(r) == Some(&b'{') {
            NAMESPACE_SCOPED
        } else {
            NAMESPACE
        };
        self.put(s, q, kind);
        self.prev = P_NAME;
        q
    }

    /// An access modifier's qualifier `[x]` at `p`: its name a type's.
    #[cold]
    fn qualifier(&mut self, p: usize) -> usize {
        let b = self.b;
        let Some(c) = crate::prim::find_in(b, p + 1, b.len().min(p + 64), *b"]").checked_add(0)
        else {
            return p;
        };
        if c >= b.len() || c >= p + 64 {
            return p;
        }
        self.put(p, p + 1, GENERIC_BEGIN);
        let s = skip_spaces(b, p + 1);
        let e = crate::trim_end(b, s, c);
        if e > s {
            let kind = if &b[s..e] == b"this" {
                LANGUAGE_VAR
            } else if is_lower_name(b, s) {
                SUPPORT_TYPE
            } else {
                PLAIN
            };
            if kind != PLAIN {
                self.put(s, e, kind);
            }
        }
        self.put(c, c + 1, GENERIC_END);
        self.prev = P_MOD;
        c + 1
    }

    /// `@` at `p`: an annotation's name and its arguments, or a pattern's binder.
    #[inline(never)]
    fn at_sign(&mut self, p: usize) -> usize {
        let b = self.b;
        let s = p + 1;
        if SOL
            || !b
                .get(s)
                .is_some_and(|&c| ACT[usize::from(c)] == A_NAME || c >= 0x80)
        {
            return self.op_cold(p);
        }
        if is_pattern_frame(self.top()) || matches!(self.prev, P_NAME | P_VALUE) {
            return self.op_cold(p);
        }
        self.put(p, p + 1, ANNOTATION_MARK);
        // `@a.b.C`: the last name the annotation's, the path before it its identifier's
        let mut q = s;
        let mut e = self.name_end(q);
        while b.get(e) == Some(&b'.')
            && b.get(e + 1).is_some_and(|&c| ACT[usize::from(c)] == A_NAME)
        {
            self.put(q, e, ANNOT_PREFIX);
            self.put(e, e + 1, ANNOT_PREFIX_DOT);
            q = e + 1;
            e = self.name_end(q);
        }
        self.put(q, e, ANNOTATION);
        let w = WORDS.value(&b[q..e], &INFO);
        if w & W_SQL != 0 {
            self.sqlw = (q, e);
        }
        self.prev = P_MOD;
        let r = skip_spaces(b, e);
        if b.get(r) == Some(&b'(') {
            if &b[q..e] == b"Language"
                && let Some(end) = sql::language_annotation(b, e)
            {
                self.mark = end;
            }
            self.push(F_ANNOT);
            self.open_meta(e);
            self.put(r, r + 1, ANNOT_BEGIN);
            self.prev = P_OP;
            return r + 1;
        }
        if b.get(r) == Some(&b'[') && self.meta == 0 {
            // `@throws[E]`: its type argument the annotation's
            self.push(F_TARGS);
            self.open_meta(e);
            self.put(r, r + 1, ANNOT_BEGIN);
            self.prev = P_OP;
            return r + 1;
        }
        e
    }

    /// An annotation's arguments open after its name at `e`, in the frame just pushed.
    #[cold]
    fn open_meta(&mut self, e: usize) {
        if self.meta == 0 {
            self.annot = self.depth;
            self.meta = META;
            self.gap = e;
        }
    }

    /// Whether a frame closed below the annotation's arguments' one at `p` (its end): the meta
    /// scope ends there.
    #[inline(always)]
    pub(super) fn annot_closed(&mut self, p: usize) {
        if self.annot > self.depth {
            self.annot = 0;
            self.end_meta(p);
        }
    }

    /// `.` at `p`: a member's accessor, an import path's, or a number's start.
    #[inline(always)]
    fn dot(&mut self, p: usize) -> usize {
        let b = self.b;
        if b.get(p + 1).is_some_and(u8::is_ascii_digit) && !matches!(self.prev, P_NAME | P_VALUE) {
            return self.number(p);
        }
        if b.get(p + 1)
            .is_some_and(|&c| OPCHAR[usize::from(c)] || c == b'.')
            && !(self.head == H_IMPORT && b.get(p + 1) == Some(&b'*'))
        {
            return self.op_cold(p);
        }
        let kind = if !SOL && self.head == H_IMPORT {
            ACCESSOR_DOT
        } else {
            ACCESSOR
        };
        if SOL || !is_pattern_frame(self.top()) || pattern_dot(b, p) {
            self.put_hot(p, p + 1, kind);
        }
        self.prev = P_DOT;
        p + 1
    }

    /// An operator at `p`: its run of operator characters read once and classed.
    #[inline(always)]
    fn op(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut e = p + 1;
        while e < n && (OPCHAR[usize::from(b[e])] || b[e] == b'.') {
            if b[e] == b'/' && matches!(b.get(e + 1), Some(b'/' | b'*')) {
                break;
            }
            if b[e] == b'.' && !(OPCHAR[usize::from(b[e - 1])] && b[e - 1] == b'.') {
                break;
            }
            e += 1;
        }
        // the run by its length and first two bytes: no slice compare
        let run = op_id(b, p, e);
        if SOL {
            let kind = match run {
                O_EQ => ASSIGN,
                O_EQEQ | O_NE => COMPARISON,
                O_ARROW => LAMBDA_ARROW,
                O_COLON => ASCRIPTION,
                _ => PLAIN,
            };
            if kind != PLAIN {
                self.put_hot(p, e, kind);
            }
            self.prev = P_OP;
            return e;
        }
        if self.head == H_DEF {
            // an operator's name, `def +(`
            self.put(p, e, FUNCTION);
            self.head = H_DEF_NAMED;
            self.prev = P_NAME;
            return e;
        }
        match run {
            O_EQ => self.equals(p),
            O_ARROW => self.arrow(p, 2),
            O_COLON => self.colon(p),
            O_LARROW => self.left_arrow(p, 2),
            O_EQEQ | O_NE => {
                if b.get(e).is_none_or(|&c| {
                    c.is_ascii_alphanumeric()
                        || matches!(
                            c,
                            b' ' | b'\t' | b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'\''
                        )
                        || c >= 0x80
                }) {
                    self.put_hot(p, e, COMPARISON);
                }
                self.prev = P_OP;
                e
            }
            O_QQQ => {
                self.put(p, e, KW_OTHER);
                self.prev = P_VALUE;
                e
            }
            O_SUB | O_SUPER
                if matches!(self.top(), F_TPARAMS | F_TBOUND) || self.head == H_TYPE_NAMED =>
            {
                self.put(p, e, BOUND);
                if self.top() == F_TPARAMS {
                    self.set_top(F_TBOUND);
                } else {
                    self.ty = u16::from(self.depth) + 1;
                }
                self.prev = P_OP;
                e
            }
            O_PLUS | O_MINUS if self.top() == F_TPARAMS => {
                self.put(p, e, BOUND);
                self.prev = P_OP;
                e
            }
            O_STAR if matches!(self.top(), F_PTYPE) && matches!(self.prev, P_NAME | P_VALUE) => {
                self.put(p, e, VARARGS);
                self.prev = P_VALUE;
                e
            }
            O_AT if is_pattern_frame(self.top()) || self.head == H_VAL => {
                self.put(p, e, AT);
                self.prev = P_OP;
                e
            }
            O_HASH if self.typed() => {
                self.put(p, e, ACCESSOR);
                self.prev = P_DOT;
                e
            }
            O_LT => self.lt(p),
            O_QUESTION if self.typed() => {
                self.put(p, e, HOLE);
                self.prev = P_NAME;
                e
            }
            O_SUB | O_SUPER => {
                self.prev = P_OP;
                e
            }
            _ => {
                if self.typed() || self.head == H_NEW {
                    self.put(p, e, SUPPORT_TYPE);
                }
                if self.head == H_IMPORT && run == O_STAR {
                    // Scala 3's wildcard
                }
                self.prev = P_OP;
                e
            }
        }
    }

    /// [`Scan::op`] out of line, for the rarer places an operator starts (`/`, after `@`, `.`).
    #[cold]
    #[inline(never)]
    fn op_cold(&mut self, p: usize) -> usize {
        self.op(p)
    }

    /// `=` at `p`: a parameter's default, a type's alias, or an assignment.
    #[inline(always)]
    fn equals(&mut self, p: usize) -> usize {
        self.prev = P_OP;
        let top = self.top();
        if matches!(top, F_PARAMS | F_PTYPE) {
            self.set_top(F_PDEFAULT);
            self.put_hot(p, p + 1, ASSIGN);
            return p + 1;
        }
        match top {
            F_FORP | F_FORB => self.set_top(if top == F_FORP { F_FORPX } else { F_FORBX }),
            F_TBOUND => {
                self.pop();
            }
            _ => {}
        }
        let head = self.head;
        self.head = H_NONE;
        self.ty = 0;
        if head == H_TYPE_NAMED || head == H_TYPE {
            // bat's `=\s*` before an alias's type
            let e = skip_spaces(self.b, p + 1);
            self.put_hot(p, e, ASSIGN);
            self.ty = u16::from(self.depth) + 1;
            return e;
        }
        self.put_hot(p, p + 1, ASSIGN);
        p + 1
    }

    /// `=>` (or `⇒`, `n` bytes) at `p`: a case's arrow, a lambda's, or a function type's.
    #[inline(always)]
    fn arrow(&mut self, p: usize, n: usize) -> usize {
        let top = self.top();
        let kind = match top {
            F_CASE | F_GUARD => {
                self.pop();
                self.ty = 0;
                CASE_ARROW
            }
            F_SELECT => ARROW,
            _ if self.prev == P_LAMBDA => LAMBDA_ARROW,
            _ if self.typed() || self.head == H_IMPORT => ARROW,
            _ => {
                if self.ty != 0 {
                    // a lambda's typed parameter, `x: Int =>`
                    self.ty = 0;
                }
                LAMBDA_ARROW
            }
        };
        self.put_hot(p, p + n, kind);
        if kind != ARROW {
            self.head = H_NONE;
        }
        self.prev = P_OP;
        p + n
    }

    /// `<-` (or `←`) at `p`: a for's enumerator's arrow, `keyword.operator.assignment` there.
    #[inline(always)]
    fn left_arrow(&mut self, p: usize, n: usize) -> usize {
        self.ty = 0;
        let top = self.top();
        if matches!(top, F_FORP | F_FORB) {
            self.set_top(if top == F_FORP { F_FORPX } else { F_FORBX });
            self.put(p, p + n, ASSIGN);
        } else if matches!(top, F_FORPX | F_FORBX) {
            self.put(p, p + n, ASSIGN);
        }
        self.prev = P_OP;
        p + n
    }

    /// `:` at `p`: a parameter's or a declaration's type, a type parameter's context bound, an
    /// ascription.
    #[inline(always)]
    fn colon(&mut self, p: usize) -> usize {
        self.prev = P_OP;
        match self.top() {
            F_PARAMS => {
                self.put_hot(p, p + 1, ASCRIPTION);
                self.set_top(F_PTYPE);
            }
            F_LPARAMS => {
                self.put_hot(p, p + 1, ASCRIPTION);
                self.set_top(F_LPTYPE);
            }
            F_TPARAMS => {
                self.put(p, p + 1, BOUND);
                self.set_top(F_TBOUND);
            }
            F_TBOUND => {
                self.put(p, p + 1, BOUND);
            }
            _ => {
                self.put_hot(p, p + 1, ASCRIPTION);
                if matches!(self.head, H_CLASS | H_NEW) && line_ends_after(self.b, p + 1) {
                    // Scala 3's `class A:` before an indented body
                    self.head = H_NONE;
                    return p + 1;
                }
                if !self.typed() && !self.lambda_after(p + 1) {
                    self.head = H_NONE;
                    self.ty = u16::from(self.depth) + 1;
                }
            }
        }
        p + 1
    }

    /// Whether a lambda's parameters follow a `:` ending at `p`, the line's end after its `=>`:
    /// Scala 3's fewer braces, `xs.map: x =>` (bat reads `map` as a parameter there).
    #[inline(never)]
    fn lambda_after(&self, p: usize) -> bool {
        let b = self.b;
        let q = skip_spaces(b, p);
        let e = match b.get(q) {
            Some(b'(') => {
                if !lambda_params(b, q) {
                    return false;
                }
                // the `)` before `=>`, then the line's end
                let mut d = 0u32;
                let mut i = q;
                while i < b.len() {
                    match b[i] {
                        b'(' | b'[' => d += 1,
                        b')' | b']' => {
                            d -= 1;
                            if d == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                let a = skip_spaces(b, i + 1);
                return line_ends_after(b, a + 2);
            }
            Some(&c) if c.is_ascii_lowercase() || c == b'_' => self.name_end(q),
            _ => return false,
        };
        if !is_arrow_next(b, e) {
            return false;
        }
        let a = skip_spaces(b, e);
        line_ends_after(b, a + 2)
    }

    /// `<` at `p`: an XML literal's start where Scala's scanner opens one, else an operator.
    #[inline(always)]
    fn lt(&mut self, p: usize) -> usize {
        if !SOL && self.xml_starts(p) {
            return self.xml_open(p);
        }
        self.prev = P_OP;
        if self.typed() {
            self.put(p, p + 1, SUPPORT_TYPE);
        }
        p + 1
    }

    /// A number at `p` (a digit, or `.` before one): bat's readings, with Scala's `_`s.
    #[inline(always)]
    fn number(&mut self, p: usize) -> usize {
        let e = self.number_at(p);
        self.prev = P_VALUE;
        e
    }

    /// A backticked name at `p`: a declaration's name in a head, a parameter's in a list; else
    /// plain.
    fn backtick(&mut self, p: usize) -> usize {
        let b = self.b;
        let close = crate::prim::find_in(b, p + 1, b.len(), *b"`");
        if close >= b.len() {
            self.prev = P_OP;
            return b.len();
        }
        let e = close + 1;
        if SOL {
            self.prev = P_NAME;
            return e;
        }
        let kind = match (self.top(), self.head) {
            (F_PARAMS, _) if is_colon_next(b, e) => PARAMETER,
            (_, H_DEF) => {
                self.head = H_DEF_NAMED;
                FUNCTION
            }
            (_, H_VAL) => VAL_NAME,
            (_, H_VAR) => VAR_NAME,
            (_, H_TYPE) => {
                self.head = H_TYPE_NAMED;
                TYPE_NAME
            }
            (F_CASE | F_PPAREN, _) => {
                self.put(p, p + 1, IDENTIFIER_MARK);
                self.put(close, e, IDENTIFIER_MARK);
                PLAIN
            }
            _ if self.typed() || self.head == H_NEW => SUPPORT_TYPE,
            _ => PLAIN,
        };
        if kind != PLAIN {
            self.put(p, e, kind);
        }
        self.prev = P_NAME;
        e
    }
}

// An operator's run as [`op_id`] tells it.
const O_OTHER: u8 = 0;
const O_EQ: u8 = 1;
const O_ARROW: u8 = 2;
const O_COLON: u8 = 3;
const O_LARROW: u8 = 4;
const O_EQEQ: u8 = 5;
const O_NE: u8 = 6;
const O_QQQ: u8 = 7;
const O_SUB: u8 = 8;
const O_SUPER: u8 = 9;
const O_PLUS: u8 = 10;
const O_MINUS: u8 = 11;
const O_STAR: u8 = 12;
const O_AT: u8 = 13;
const O_HASH: u8 = 14;
const O_LT: u8 = 15;
const O_QUESTION: u8 = 16;

/// The operator run `b[p..e]`: one of the few that read apart ([`O_EQ`]…), else [`O_OTHER`].
#[inline(always)]
fn op_id(b: &[u8], p: usize, e: usize) -> u8 {
    let c0 = b[p];
    match e - p {
        1 => match c0 {
            b'=' => O_EQ,
            b':' => O_COLON,
            b'+' => O_PLUS,
            b'-' => O_MINUS,
            b'*' => O_STAR,
            b'@' => O_AT,
            b'#' => O_HASH,
            b'<' => O_LT,
            b'?' => O_QUESTION,
            _ => O_OTHER,
        },
        2 => match (c0, b[p + 1]) {
            (b'=', b'>') => O_ARROW,
            (b'<', b'-') => O_LARROW,
            (b'=', b'=') => O_EQEQ,
            (b'!', b'=') => O_NE,
            (b'<', b':') => O_SUB,
            (b'>', b':') => O_SUPER,
            _ => O_OTHER,
        },
        3 if c0 == b'?' && b[p + 1] == b'?' && b[p + 2] == b'?' => O_QQQ,
        _ => O_OTHER,
    }
}

/// Whether the character at `p` starts a lowercase name (bat's `\p{Ll}`, or `_` before a
/// name's character).
fn is_lower_name(b: &[u8], p: usize) -> bool {
    match b.get(p) {
        Some(&c) if c.is_ascii_lowercase() => true,
        Some(b'_') => b
            .get(p + 1)
            .is_some_and(|&c| c.is_ascii_alphanumeric() || c >= 0x80),
        Some(&c) if c >= 0x80 => crate::prim::wide_char(b, p).is_some_and(char::is_lowercase),
        _ => false,
    }
}

/// Whether a line in a for's braces is an enumerator's: a `<-` (`←`) or a lone `=` in it.
#[cold]
fn enumerator_line(b: &[u8]) -> bool {
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'<' if b.get(i + 1) == Some(&b'-') => return true,
            b'=' => {
                let before = i > 0 && OPCHAR[usize::from(b[i - 1])];
                let after = b.get(i + 1).is_some_and(|&c| OPCHAR[usize::from(c)]);
                if !before && !after {
                    return true;
                }
            }
            0xe2 if b.get(i + 1..i + 3) == Some(&[0x86, 0x90]) => return true,
            b'"' | b'/' => return false,
            _ => {}
        }
        i += 1;
    }
    false
}

/// Whether a pattern's `.` at `p` is bat's accessor: beside a lowercase name (`a.B`, `A.b`).
fn pattern_dot(b: &[u8], p: usize) -> bool {
    if is_lower_name(b, p + 1) {
        return true;
    }
    let mut s = p;
    while s > 0 && NAME[usize::from(b[s - 1])] {
        s -= 1;
    }
    // `a.b.C`: bat's `LOWER(?:(\.)LOWER)+` takes `a.b` whole, so the `.` after it isn't one
    s < p && is_lower_name(b, s) && (s == 0 || b[s - 1] != b'.')
}

/// Where the run of a name's ASCII bytes (`[A-Za-z0-9_$]`, [`NAME`]) from `from` ends: sixteen
/// bytes a step on x86-64 (SSE2's compares, the line's last step read again from its end less
/// sixteen, as [`crate::prim::find_in`] does), a byte a step elsewhere.
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
                let dollar = _mm_cmpeq_epi8(w, _mm_set1_epi8(b'$' as i8));
                let m = _mm_or_si128(_mm_or_si128(alpha, digit), _mm_or_si128(us, dollar));
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
    while e < n && NAME[usize::from(b[e])] {
        e += 1;
    }
    e
}

/// Whether `=>` (or `⇒`) follows `e` past blanks.
#[inline(always)]
fn is_arrow_next(b: &[u8], e: usize) -> bool {
    // one blank and `=>` without a call: most names are followed by neither
    let mut q = e;
    match b.get(q) {
        Some(b' ') => {
            q += 1;
            match b.get(q) {
                Some(b'=' | 0xe2) => {}
                Some(b' ' | b'\t') => q = skip_spaces(b, q),
                _ => return false,
            }
        }
        Some(b'=' | 0xe2) => {}
        Some(b'\t') => q = skip_spaces(b, q),
        _ => return false,
    }
    match b.get(q) {
        Some(b'=') => {
            b.get(q + 1) == Some(&b'>') && !b.get(q + 2).is_some_and(|&c| OPCHAR[usize::from(c)])
        }
        Some(0xe2) => b.get(q..q + 3) == Some(&[0xe2, 0x87, 0x92]),
        _ => false,
    }
}

/// Whether `:` (not `::`) follows `e` past blanks.
fn is_colon_next(b: &[u8], e: usize) -> bool {
    let q = skip_spaces(b, e);
    b.get(q) == Some(&b':') && !b.get(q + 1).is_some_and(|&c| OPCHAR[usize::from(c)])
}

/// Whether a name follows `e` past blanks.
fn is_name_next(b: &[u8], e: usize) -> bool {
    let q = skip_spaces(b, e);
    q > e
        && b.get(q)
            .is_some_and(|&c| ACT[usize::from(c)] == A_NAME || c == b'`' || c >= 0x80)
}

/// Whether only blanks (or a comment) follow `p` on the line.
fn line_ends_after(b: &[u8], p: usize) -> bool {
    let q = skip_spaces(b, p);
    q >= b.len() || b[q..].starts_with(b"//")
}

/// Whether `end` at `..e` is an end marker: a name or one of the keywords it can close, then the
/// line's end.
fn end_marker(b: &[u8], e: usize) -> bool {
    let q = skip_spaces(b, e);
    if q == e || q >= b.len() {
        return false;
    }
    let mut r = q;
    while r < b.len() && (NAME[usize::from(b[r])] || b[r] >= 0x80) {
        r += 1;
    }
    r > q && line_ends_after(b, r)
}

/// Whether the `(` at `p` opens a lambda's parameters: its `)` (on the line, within [`LOOK`]
/// bytes) is followed by `=>`, and only a parameter list's bytes are between.
fn lambda_params(b: &[u8], p: usize) -> bool {
    let end = b.len().min(p + LOOK);
    let mut depth = 0u32;
    let mut i = p;
    while i < end {
        match b[i] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return is_arrow_next(b, i + 1);
                }
            }
            b'"' | b'\'' | b'{' | b'}' | b';' | b'/' => return false,
            b'=' if b.get(i + 1) != Some(&b'>') => return false,
            _ => {}
        }
        i += 1;
    }
    false
}
