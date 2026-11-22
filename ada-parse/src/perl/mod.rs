//! Perl on a scanner of ada's own (X-170), from a floor probe (the same files built with
//! `ADA_PARSE_X170_SOL`): one loop over a line's bytes, a byte's action one load from a table
//! ([`ACT`]), a name read once and looked up once in one table of every word ([`WORDS`]: its
//! leaf and, where its meaning hangs on where it stands, an action), and what Perl's lexer
//! expects next in a byte ([`Scan::prev`], carried across lines as Perl carries it): a term or
//! an operator, which decides what `/`, `<`, `%`, `&`, `*`, `-` and `?` start. Frames only for
//! what nests ([`Scan::frames`], four bits a level): braces by what they opened (a block, an
//! anonymous hash, a subscript, a dereference, `map`'s block), parentheses (a condition's, a
//! signature's, a DBI call's arguments), brackets.
//!
//! **Text, as Perl reads it.** A quote-like construct (`'…'`, `"…"`, `q`, `qq`, `qw`, `qx`, `m`,
//! `qr`, `s`, `tr`, `y`, `//`, `<…>`) is found to its end first, as Perl's `scan_str` finds it
//! (any delimiter, brackets nesting, a `\` taking the byte after it), and only then read: its
//! escapes, its interpolated variables with their subscripts (the code in `${…}`, `@{[…]}` and
//! `[…]` lexed by the code loop on the piece of the line it spans), its regex on
//! [`crate::regex`], its SQL on [`crate::sql`]. One open at a line's end goes on there
//! ([`M_QUOTE`]). Here-documents wait on X-105's [`Queue`] and their bodies are a mode; POD,
//! `__END__`'s and `__DATA__`'s text and `format`'s pictures are modes.
//!
//! Perl's reading where bat's grammar has none or derails: `/` a divide after a value and a
//! regex where a term starts (bat's a regex after any bareword, run on across lines); `<<~` an
//! indented here-document (bat's a shift and a `~`, the body code); two here-documents on a
//! line; a comment between `s{…}` and `{…}` (bat's `#` a delimiter, the file a string after
//! it); `__END__`'s text not code; signatures' parameters; `->@*`, `${^NAME}`, `v5.36`; a
//! literal string's `\n` text (only `\\` and `\'` are escapes in it); POD's verbatim lines
//! without formatting codes. SQL in DBI's calls (`->prepare(`, `->do(`, `->selectall_*(`, …)
//! and in a `<<SQL` here-document.
//!
//! **Guests** (X-191). A here-document whose label names HTML, CSS, JavaScript, JSON or XML
//! (`<<HTML`, `<<"EOHTML"`, `<<'JS'`: [`text`]'s `label_lang`), and POD's `=begin html` …
//! `=end html` region and `=for html` paragraph, are that language's on its own scanner: on the
//! run sink by its own rules ([`Sink::side_guest`]), written among Perl's own lexemes (an
//! interpolating body's escapes and interpolated variables, which the guest reads blanked);
//! on the span sink, or where the rules keep no guest's, Perl's text, the guest lexed for its
//! state alone ([`crate::lex_state`]), so both sinks leave the same state. While a body is open
//! the state is the guest's, and Perl's own fields one word in `withs[6..14]` (`G_*`): a body
//! opened deeper than [`G_LEVELS`] frames, beside a second here-document, in a `use` statement
//! or a DBI call stays Perl's text, and a guest's line that sets one of those bytes loses the
//! guest's state (its lines lexed each from a fresh one to the body's end).

#[cfg(test)]
mod tests;
mod text;
mod vars;

use crate::emit::{Rules, Run, Runs, Side, Sink, Spans};
use crate::heredoc::{Label, Queue};
use crate::prim::{WORD, char_len};
use crate::words::{Words, text_len};
use crate::{Out, State};

/// The floor probe (X-170's `sol`): every word its table leaf, a call before `(` and a class
/// before `->`; strings, regexes and here-documents read to their ends with their escapes and
/// `$name`s, nothing else of their text (no subscript's code, placeholder, regex token or SQL);
/// no signature, label, `use` statement's meta scope, POD formatting code or `format`.
pub(super) const SOL: bool = option_env!("ADA_PARSE_X170_SOL").is_some();

/// The leaves below [`STR`]: a token's scopes.
static HEAD_LEAVES: [&str; 8] = [
    "",
    "comment.line.number-sign.perl",
    "comment.line.number-sign.perl punctuation.definition.comment.perl",
    "comment.block.documentation.perl",
    "comment.block.documentation.perl entity.name.tag.pod.perl",
    "comment.block.documentation.perl markup.heading.perl",
    "meta.preprocessor.use.perl",
    "",
];
const COMMENT: u16 = 1;
const COMMENT_MARK: u16 = 2;
const POD: u16 = 3;
const POD_TAG: u16 = 4;
const POD_HEAD: u16 = 5;
/// A `use`, `no` or `require` statement's text between its tokens.
const PP: u16 = 6;

/// A string's leaves: [`CLASSES`]' each seven, its text, then these after it ([`str_leaf`]).
const STR: u16 = 8;
const ESC: u16 = 1;
const PLACEHOLDER: u16 = 2;
const BEGIN: u16 = 3;
const END: u16 = 4;
/// A placeholder's `%`, and its precision's `.`.
const PH_MARK: u16 = 5;
const PH_DOT: u16 = 6;
/// The leaves of a string's class.
const PER_CLASS: u16 = 7;
/// What a string's text is, by its construct.
static CLASSES: [&str; 8] = [
    "string.quoted.single.perl",
    "string.quoted.double.perl",
    "string.quoted.backtick.perl",
    "string.quoted.angle.perl",
    "string.unquoted.perl",
    "string.unquoted.heredoc.perl",
    "string.regexp.perl",
    // an HTML, CSS, JavaScript, JSON or XML here-document's text where its guest isn't lexed
    // (X-191: the span sink; a body its state doesn't fit), its interpolations Perl's
    "meta.embedded.perl",
];
const C_SQ: u8 = 0;
const C_DQ: u8 = 1;
const C_BT: u8 = 2;
const C_ANGLE: u8 = 3;
const C_UNQ: u8 = 4;
const C_HEREDOC: u8 = 5;
const C_REGEX: u8 = 6;
const C_EMBED: u8 = 7;
static STR_TAILS: [&str; PER_CLASS as usize] = [
    "",
    "constant.character.escape.perl",
    "constant.other.placeholder.perl",
    "punctuation.definition.string.begin.perl",
    "punctuation.definition.string.end.perl",
    "constant.other.placeholder.perl punctuation.definition.placeholder.perl",
    "constant.other.placeholder.perl punctuation.separator.decimal.perl",
];

/// The text leaf of a string of class `c`.
#[inline(always)]
const fn str_leaf(c: u8) -> u16 {
    STR + PER_CLASS * c as u16
}

/// The leaves past the strings', from [`LEAF`].
static LEAVES: [&str; 101] = [
    // storage and declarations
    "storage.type.variable.perl",
    "storage.type.function.perl keyword.declaration.function.perl",
    "storage.type.namespace.perl keyword.declaration.namespace.perl",
    "storage.type.format.perl",
    "keyword.control.import.use.perl",
    "keyword.control.import.no.perl",
    "keyword.control.import.require.perl",
    // control
    "keyword.control.conditional.if.perl",
    "keyword.control.conditional.unless.perl",
    "keyword.control.conditional.else.perl",
    "keyword.control.conditional.elseif.perl",
    "keyword.control.conditional.given.perl",
    "keyword.control.conditional.when.perl",
    "keyword.control.conditional.default.perl",
    "keyword.control.loop.for.perl",
    "keyword.control.loop.foreach.perl",
    "keyword.control.loop.while.perl",
    "keyword.control.loop.until.perl",
    "keyword.control.flow.return.perl",
    "keyword.control.flow.die.perl",
    "keyword.control.flow.exit.perl",
    "keyword.control.flow.next.perl",
    "keyword.control.flow.last.perl",
    "keyword.control.flow.redo.perl",
    "keyword.control.flow.goto.perl",
    "keyword.control.flow.do.perl",
    "keyword.control.flow.dump.perl",
    "keyword.control.flow.caller.perl",
    "keyword.control.flow.continue.perl",
    "keyword.control.flow.break.perl",
    "keyword.control.flow.wait.perl",
    // constants
    "constant.language.perl",
    "constant.language.filehandle.perl",
    "constant.other.perl",
    "constant.other.key.perl",
    "constant.language.flags.regexp.perl",
    "constant.numeric.integer.decimal.perl",
    "constant.numeric.float.decimal.perl",
    "constant.numeric.float.decimal.perl punctuation.separator.decimal.perl",
    "constant.numeric.integer.hexadecimal.perl",
    "constant.numeric.integer.hexadecimal.perl punctuation.definition.numeric.hexadecimal.perl",
    "constant.numeric.integer.octal.perl",
    "constant.numeric.integer.octal.perl punctuation.definition.numeric.octal.perl",
    "constant.numeric.integer.binary.perl",
    "constant.numeric.integer.binary.perl punctuation.definition.numeric.binary.perl",
    "constant.numeric.version.perl",
    "constant.numeric.version.perl punctuation.separator.decimal.perl",
    "constant.numeric.version.perl punctuation.definition.version.perl",
    // variables
    "variable.other.readwrite.perl",
    "variable.other.readwrite.perl punctuation.definition.variable.perl",
    "variable.language.perl",
    "variable.language.perl punctuation.definition.variable.perl",
    "variable.language.regexp.match-group.perl",
    "variable.language.regexp.match-group.perl punctuation.definition.variable.perl",
    "variable.language.regexp.match.perl",
    "variable.language.regexp.match.perl punctuation.definition.variable.perl",
    "variable.namespace.perl",
    "punctuation.accessor.double-colon.perl",
    "keyword.operator.dereference.perl",
    "punctuation.definition.variable.begin.perl",
    "punctuation.definition.variable.end.perl",
    "variable.parameter.perl",
    "entity.name.label.perl",
    "punctuation.separator.perl",
    "variable.label.perl",
    // functions and names
    "support.function.perl",
    "variable.function.perl",
    "variable.function.member.perl",
    "entity.name.function.perl",
    "entity.name.function.prepocessor.perl",
    "entity.name.function.callback.perl",
    "variable.annotation.attribute.perl",
    "punctuation.definition.annotation.perl",
    "entity.name.namespace.perl",
    "meta.preprocessor.use.perl entity.name.namespace.perl",
    // operators
    "keyword.operator.assignment.perl",
    "keyword.operator.logical.perl",
    "keyword.operator.arithmetic.perl",
    "keyword.operator.binary.perl",
    "keyword.operator.comparison.perl",
    "keyword.operator.concat.perl",
    "keyword.operator.reference.perl",
    "keyword.operator.filetest.perl",
    "keyword.operator.bitwise.perl",
    "keyword.operator.range.perl",
    "keyword.operator.heredoc.perl",
    // punctuation
    "punctuation.accessor.arrow.perl",
    "punctuation.separator.key-value.perl",
    "punctuation.separator.sequence.perl",
    "punctuation.terminator.statement.perl",
    "punctuation.section.group.begin.perl",
    "punctuation.section.group.end.perl",
    "punctuation.section.block.begin.perl",
    "punctuation.section.block.end.perl",
    "punctuation.section.brackets.begin.perl",
    "punctuation.section.brackets.end.perl",
    "punctuation.section.item-access.begin.perl",
    "punctuation.section.item-access.end.perl",
    "punctuation.section.parameters.begin.perl",
    "punctuation.section.parameters.end.perl",
    "punctuation.terminator.format.perl",
];

/// Where [`LEAVES`] starts among the kinds.
const LEAF: u16 = STR + PER_CLASS * CLASSES.len() as u16;
const STORAGE: u16 = LEAF;
const SUB_KW: u16 = LEAF + 1;
const PACKAGE_KW: u16 = LEAF + 2;
const FORMAT_KW: u16 = LEAF + 3;
const USE_KW: u16 = LEAF + 4;
const NO_KW: u16 = LEAF + 5;
const REQUIRE_KW: u16 = LEAF + 6;
const IF_KW: u16 = LEAF + 7;
const UNLESS_KW: u16 = LEAF + 8;
const ELSE_KW: u16 = LEAF + 9;
const ELSIF_KW: u16 = LEAF + 10;
const GIVEN_KW: u16 = LEAF + 11;
const WHEN_KW: u16 = LEAF + 12;
const DEFAULT_KW: u16 = LEAF + 13;
const FOR_KW: u16 = LEAF + 14;
const FOREACH_KW: u16 = LEAF + 15;
const WHILE_KW: u16 = LEAF + 16;
const UNTIL_KW: u16 = LEAF + 17;
const RETURN_KW: u16 = LEAF + 18;
const DIE_KW: u16 = LEAF + 19;
const EXIT_KW: u16 = LEAF + 20;
const NEXT_KW: u16 = LEAF + 21;
const LAST_KW: u16 = LEAF + 22;
const REDO_KW: u16 = LEAF + 23;
const GOTO_KW: u16 = LEAF + 24;
const DO_KW: u16 = LEAF + 25;
const DUMP_KW: u16 = LEAF + 26;
const CALLER_KW: u16 = LEAF + 27;
const CONTINUE_KW: u16 = LEAF + 28;
const BREAK_KW: u16 = LEAF + 29;
const WAIT_KW: u16 = LEAF + 30;
const CONST_LANG: u16 = LEAF + 31;
const FILEHANDLE: u16 = LEAF + 32;
const CONSTANT: u16 = LEAF + 33;
const KEY: u16 = LEAF + 34;
const FLAGS: u16 = LEAF + 35;
const INT: u16 = LEAF + 36;
const FLOAT: u16 = LEAF + 37;
const FLOAT_DOT: u16 = LEAF + 38;
/// A number's prefix (`0x`) is its kind plus one.
const HEX: u16 = LEAF + 39;
const OCT: u16 = LEAF + 41;
const BIN: u16 = LEAF + 43;
const VERSION: u16 = LEAF + 45;
const VERSION_DOT: u16 = LEAF + 46;
const VERSION_V: u16 = LEAF + 47;
const VAR: u16 = LEAF + 48;
const VAR_SIGIL: u16 = LEAF + 49;
const VAR_LANG: u16 = LEAF + 50;
const VAR_LANG_SIGIL: u16 = LEAF + 51;
const GROUP_VAR: u16 = LEAF + 52;
const GROUP_SIGIL: u16 = LEAF + 53;
const MATCH_VAR: u16 = LEAF + 54;
const MATCH_SIGIL: u16 = LEAF + 55;
const NS: u16 = LEAF + 56;
const DCOLON: u16 = LEAF + 57;
const DEREF: u16 = LEAF + 58;
const VBEGIN: u16 = LEAF + 59;
const VEND: u16 = LEAF + 60;
const PARAM: u16 = LEAF + 61;
const LABEL: u16 = LEAF + 62;
const LABEL_SEP: u16 = LEAF + 63;
const LABEL_USE: u16 = LEAF + 64;
const SUPPORT: u16 = LEAF + 65;
const FUNC: u16 = LEAF + 66;
const MEMBER: u16 = LEAF + 67;
const ENTITY: u16 = LEAF + 68;
const PREPROC: u16 = LEAF + 69;
const CALLBACK: u16 = LEAF + 70;
const ATTR: u16 = LEAF + 71;
const ATTR_MARK: u16 = LEAF + 72;
const NS_DECL: u16 = LEAF + 73;
const NS_PP: u16 = LEAF + 74;
const ASSIGN: u16 = LEAF + 75;
const LOGICAL: u16 = LEAF + 76;
const ARITH: u16 = LEAF + 77;
const BINARY: u16 = LEAF + 78;
const COMPARE: u16 = LEAF + 79;
const CONCAT: u16 = LEAF + 80;
const REFOP: u16 = LEAF + 81;
const FILETEST: u16 = LEAF + 82;
const BITWISE: u16 = LEAF + 83;
const RANGE: u16 = LEAF + 84;
const HEREDOC_OP: u16 = LEAF + 85;
const ARROW: u16 = LEAF + 86;
const KEYVALUE: u16 = LEAF + 87;
const COMMA: u16 = LEAF + 88;
const SEMI: u16 = LEAF + 89;
const GROUP_B: u16 = LEAF + 90;
const GROUP_E: u16 = LEAF + 91;
const BLOCK_B: u16 = LEAF + 92;
const BLOCK_E: u16 = LEAF + 93;
const BRACKETS_B: u16 = LEAF + 94;
const BRACKETS_E: u16 = LEAF + 95;
const ITEM_B: u16 = LEAF + 96;
const ITEM_E: u16 = LEAF + 97;
const PARAMS_B: u16 = LEAF + 98;
const PARAMS_E: u16 = LEAF + 99;
const FORMAT_END: u16 = LEAF + 100;

/// A quote-like construct's delimiters ([`delim_kind`]): braces, brackets, parentheses,
/// angles, any other character; begin, end.
const DELIMS: u16 = LEAF + LEAVES.len() as u16;
static DELIM_LEAVES: [&str; 10] = [
    "punctuation.section.braces.begin.perl",
    "punctuation.section.braces.end.perl",
    "punctuation.section.brackets.begin.perl",
    "punctuation.section.brackets.end.perl",
    "punctuation.section.parens.begin.perl",
    "punctuation.section.parens.end.perl",
    "punctuation.section.generic.begin.perl",
    "punctuation.section.generic.end.perl",
    "punctuation.section.generic.begin.perl",
    "punctuation.section.generic.end.perl",
];

/// A here-document's tag by its language ([`HEREDOC_TAGS`]' order), then its quotes.
const TAGS: u16 = DELIMS + DELIM_LEAVES.len() as u16;
static HEREDOC_TAGS: [&str; 8] = [
    "meta.tag.heredoc.perl entity.name.tag.heredoc.plain.perl",
    "meta.tag.heredoc.perl entity.name.tag.heredoc.sql.perl",
    "meta.tag.heredoc.perl entity.name.tag.heredoc.html.perl",
    "meta.tag.heredoc.perl entity.name.tag.heredoc.css.perl",
    "meta.tag.heredoc.perl entity.name.tag.heredoc.js.perl",
    "meta.tag.heredoc.perl entity.name.tag.heredoc.json.perl",
    "meta.tag.heredoc.perl entity.name.tag.heredoc.xml.perl",
    "meta.tag.heredoc.perl punctuation.definition.tag.begin.perl",
];
const TAG_QUOTE: u16 = TAGS + 7;
const TAG_QUOTE_END: u16 = TAGS + 8;
/// `format`'s pictures: a field's `@` or `^`, its `<`s, `>`s, `|`s and `#`s.
const FORMAT_MARK: u16 = TAGS + 9;
const FORMAT_FIELD: u16 = TAGS + 10;
/// A regex's comment in its extended syntax.
const RE_COMMENT: u16 = TAGS + 11;

/// POD's formatting codes: by context (plain, a command's line), a letter's two kinds (its
/// letter, its text) and then the `<` and `>` ([`pod_kind`]).
const POD_FMT: u16 = TAGS + 12;
/// The formatting codes' letters, in [`POD_FMT`]'s order.
pub(super) const POD_LETTERS: &[u8; 8] = b"BCEFILSX";
static POD_LETTER_TAGS: [&str; 8] = [
    "entity.name.tag.bold.perl",
    "entity.name.tag.code.perl",
    "entity.name.tag.escaped.perl",
    "entity.name.tag.filename.perl",
    "entity.name.tag.italic.perl",
    "entity.name.tag.link.perl",
    "entity.name.tag.none-braeking.perl",
    "entity.name.tag.index.perl",
];
static POD_LETTER_TEXT: [&str; 8] = [
    "markup.bold.perl",
    "markup.quote.perl",
    "constant.character.escape.perl",
    "string.unquoted.perl",
    "markup.italic.perl",
    "markup.underline.link.perl",
    "",
    "entity.name.label.perl",
];
/// A context's kinds: 8 letters' two, then `<` and `>`.
const POD_CTX: u16 = 18;
/// A language's name after `=begin`, `=for` or `=end` (X-191: `html`).
const POD_LANG: u16 = POD_FMT + 2 * POD_CTX;
/// Past the last kind of the leaves above.
const KINDS_END: u16 = POD_LANG + 1;

/// The kind of POD's formatting code `letter` (an index into [`POD_LETTERS`]) in context `ctx`
/// (0 plain text, 1 a command's line): its letter, or its text.
#[inline(always)]
pub(super) const fn pod_kind(ctx: u8, letter: u8, text: bool) -> u16 {
    POD_FMT + ctx as u16 * POD_CTX + 2 * letter as u16 + text as u16
}

/// A POD context's `<` (`end` false) or `>`.
#[inline(always)]
pub(super) const fn pod_angle(ctx: u8, end: bool) -> u16 {
    POD_FMT + ctx as u16 * POD_CTX + 16 + end as u16
}

/// A regex's kinds, under `string.regexp.perl`.
const RE_BASE: u16 = 256;
const _: () = assert!(KINDS_END <= RE_BASE);
/// SQL's kinds, in a string of each class that takes it ([`sql_class`]).
const SQL_BASE: u16 = RE_BASE + crate::regex::KINDS as u16;

/// Which of SQL's kind ranges a string of class `c` uses: `'…'`, `"…"`, `q`/`qq`, a
/// here-document's (its text plain, as bat's `source.sql` is).
#[inline(always)]
const fn sql_class(c: u8) -> u16 {
    match c {
        C_SQ => 0,
        C_DQ => 1,
        C_HEREDOC => 3,
        _ => 2,
    }
}

/// The scopes under `source.perl` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    let split = |s: &'static str| s.split(' ').filter(|s| !s.is_empty());
    if kind >= SQL_BASE {
        let i = (kind - SQL_BASE) / crate::sql::KINDS as u16;
        let class = match i {
            0 => C_SQ,
            1 => C_DQ,
            2 => C_UNQ,
            3 => C_EMBED,
            _ => return Vec::new(),
        };
        let mut out: Vec<_> = split(CLASSES[usize::from(class)]).collect();
        out.extend(crate::sql::scopes(
            (kind - SQL_BASE) % crate::sql::KINDS as u16,
        ));
        return out;
    }
    if kind >= RE_BASE {
        let mut out = vec!["string.regexp.perl"];
        out.extend(crate::regex::scopes(kind - RE_BASE));
        return out;
    }
    if kind >= POD_FMT {
        if kind >= KINDS_END {
            return Vec::new();
        }
        if kind == POD_LANG {
            return vec![
                "comment.block.documentation.perl",
                "constant.other.language-name.perl",
            ];
        }
        let v = kind - POD_FMT;
        let (ctx, i) = (v / POD_CTX, v % POD_CTX);
        let mut out = vec!["comment.block.documentation.perl"];
        if ctx == 1 {
            out.push("markup.heading.perl");
        }
        match i {
            16 => out.push("punctuation.definition.tag.begin.perl"),
            17 => out.push("punctuation.definition.tag.end.perl"),
            _ if i % 2 == 0 => out.push(POD_LETTER_TAGS[usize::from(i / 2)]),
            _ => out.extend(split(POD_LETTER_TEXT[usize::from(i / 2)])),
        }
        return out;
    }
    if kind >= TAGS {
        return match kind - TAGS {
            i @ 0..7 => split(HEREDOC_TAGS[usize::from(i)]).collect(),
            7 => split(HEREDOC_TAGS[7]).collect(),
            8 => vec![
                "meta.tag.heredoc.perl",
                "punctuation.definition.tag.end.perl",
            ],
            9 => vec![
                "string.unquoted.perl",
                "constant.other.placeholder.text.perl",
                "punctuation.definition.placeholder.begin.perl",
            ],
            10 => vec![
                "string.unquoted.perl",
                "constant.other.placeholder.text.perl",
            ],
            11 => vec!["string.regexp.perl", "comment.line.number-sign.regexp"],
            _ => Vec::new(),
        };
    }
    if kind >= DELIMS {
        return split(DELIM_LEAVES[usize::from(kind - DELIMS)]).collect();
    }
    if kind >= LEAF {
        return split(LEAVES[usize::from(kind - LEAF)]).collect();
    }
    if kind >= STR {
        let v = kind - STR;
        let mut out: Vec<_> = split(CLASSES[usize::from(v / PER_CLASS)]).collect();
        out.extend(split(STR_TAILS[usize::from(v % PER_CLASS)]));
        return out;
    }
    HEAD_LEAVES
        .get(usize::from(kind))
        .map_or_else(Vec::new, |l| split(l).collect())
}

/// The kind of a quote-like construct's delimiter `c` (its opener), begin or end.
#[inline(always)]
const fn delim_kind(open: u8, end: bool) -> u16 {
    let i = match open {
        b'{' => 0,
        b'[' => 2,
        b'(' => 4,
        b'<' => 6,
        _ => 8,
    };
    DELIMS + i + end as u16
}

// A byte's action ([`ACT`]).
const A_OTHER: u8 = 0;
const A_SPACE: u8 = 1;
const A_NAME: u8 = 2;
const A_DIGIT: u8 = 3;
const A_DOLLAR: u8 = 4;
const A_AT: u8 = 5;
const A_PERCENT: u8 = 6;
const A_AMP: u8 = 7;
const A_STAR: u8 = 8;
/// `'`, `"`, `` ` ``.
const A_QUOTE: u8 = 9;
const A_SLASH: u8 = 10;
const A_LT: u8 = 11;
const A_MINUS: u8 = 12;
const A_DOT: u8 = 13;
/// `+ = ! ~ > | ^ ? :`: an operator.
const A_OP: u8 = 14;
const A_LPAREN: u8 = 15;
const A_RPAREN: u8 = 16;
const A_LBRACE: u8 = 17;
const A_RBRACE: u8 = 18;
const A_LBRACKET: u8 = 19;
const A_RBRACKET: u8 = 20;
const A_COMMA: u8 = 21;
const A_SEMI: u8 = 22;
const A_HASH: u8 = 23;
const A_BSLASH: u8 = 24;

static ACT: [u8; 256] = {
    // a byte past ASCII starts a name (`use utf8`'s identifiers)
    let mut t = [A_NAME; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = match b {
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => A_SPACE,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => A_NAME,
            b'0'..=b'9' => A_DIGIT,
            b'$' => A_DOLLAR,
            b'@' => A_AT,
            b'%' => A_PERCENT,
            b'&' => A_AMP,
            b'*' => A_STAR,
            b'"' | b'\'' | b'`' => A_QUOTE,
            b'/' => A_SLASH,
            b'<' => A_LT,
            b'-' => A_MINUS,
            b'.' => A_DOT,
            b'+' | b'=' | b'!' | b'~' | b'>' | b'|' | b'^' | b'?' | b':' => A_OP,
            b'(' => A_LPAREN,
            b')' => A_RPAREN,
            b'{' => A_LBRACE,
            b'}' => A_RBRACE,
            b'[' => A_LBRACKET,
            b']' => A_RBRACKET,
            b',' => A_COMMA,
            b';' => A_SEMI,
            b'#' => A_HASH,
            b'\\' => A_BSLASH,
            _ => A_OTHER,
        };
        c += 1;
    }
    t
};

// What a word is ([`WORDS`], [`INFO`]): its leaf in the low byte (an offset from [`LEAF`], 0
// for none), an action in the next five bits, flags above.
const W_LEAF: u16 = 0xff;
const W_ACTION: u16 = 0x1f << 8;
/// A keyword: a term follows.
const W_KW: u16 = 1 << 8;
/// `my`, `our`, `local`, `state`.
const W_MY: u16 = 2 << 8;
/// `if`, `unless`, `elsif`, `while`, `until`, `given`, `when`: a condition's `(` next.
const W_COND: u16 = 3 << 8;
/// `for`, `foreach`: a condition's `(` after the loop's variable.
const W_FOR: u16 = 4 << 8;
/// `else`, `default`, `continue`: a block next.
const W_BLOCK: u16 = 5 << 8;
/// `sub`.
const W_SUB: u16 = 6 << 8;
/// `package`.
const W_PKG: u16 = 7 << 8;
/// `use`, `no`.
const W_USE: u16 = 8 << 8;
/// `require`.
const W_REQUIRE: u16 = 9 << 8;
/// `next`, `last`, `redo`, `goto`, `dump`: a label may follow.
const W_LABEL: u16 = 10 << 8;
/// `do`, `eval`: `{` a block whose `}` ends a value.
const W_DO: u16 = 11 << 8;
/// `map`, `grep`, `sort`: `{` a block whose `}` leaves a term to come.
const W_LIST: u16 = 12 << 8;
/// A built-in function: a term follows (but [`W_VALUE`]).
const W_FN: u16 = 13 << 8;
/// `and`, `or`, `not`, `xor`, `eq`, `ne`, `lt`, `gt`, `le`, `ge`, `cmp`, `isa`: an operator.
const W_WORDOP: u16 = 14 << 8;
/// `x`: repetition after a value.
const W_X: u16 = 15 << 8;
/// `q`, `qq`, `qw`, `qx`, `m`, `qr`, `s`, `tr`, `y`.
const W_QUOTE: u16 = 16 << 8;
/// `format`.
const W_FORMAT: u16 = 17 << 8;
/// `__END__`, `__DATA__`: the file's code ends.
const W_END: u16 = 18 << 8;
/// A filehandle (`STDIN`, `STDERR`, …).
const W_FH: u16 = 19 << 8;
/// `BEGIN`, `END`, `INIT`, `CHECK`, `UNITCHECK`: a phase's block.
const W_PHASE: u16 = 20 << 8;
/// `AUTOLOAD`, `DESTROY`: a callback's name after `sub`.
const W_CALLBACK: u16 = 21 << 8;
/// A value after it, not a term: `time`, `wantarray`, `shift` (`shift // 0`), `__FILE__`.
const W_VALUE: u16 = 1 << 13;
/// A special variable's name after a sigil (`$ENV`, `@ARGV`, `$_`).
const W_VARLANG: u16 = 1 << 14;
/// A DBI method that takes SQL after `->` (`prepare`, `do`, `selectall_arrayref`, …).
const W_SQLM: u16 = 1 << 15;

/// The leaf offset of kind `k` for [`WORD_LIST`]: past [`LEAF`], plus one (0 is none).
const fn lf(k: u16) -> u16 {
    k - LEAF + 1
}

/// The words, each with what it is.
const WORD_LIST: &[(&[u8], u16)] = &[
    (b"my", W_MY | lf(STORAGE)),
    (b"our", W_MY | lf(STORAGE)),
    (b"local", W_MY | lf(STORAGE)),
    (b"state", W_MY | lf(STORAGE)),
    (b"sub", W_SUB | lf(SUB_KW)),
    (b"package", W_PKG | lf(PACKAGE_KW)),
    (b"format", W_FORMAT | lf(FORMAT_KW)),
    (b"use", W_USE | lf(USE_KW)),
    (b"no", W_USE | lf(NO_KW)),
    (b"require", W_REQUIRE | lf(REQUIRE_KW)),
    (b"if", W_COND | lf(IF_KW)),
    (b"unless", W_COND | lf(UNLESS_KW)),
    (b"elsif", W_COND | lf(ELSIF_KW)),
    (b"while", W_COND | lf(WHILE_KW)),
    (b"until", W_COND | lf(UNTIL_KW)),
    (b"given", W_COND | lf(GIVEN_KW)),
    (b"when", W_COND | lf(WHEN_KW)),
    (b"for", W_FOR | lf(FOR_KW)),
    (b"foreach", W_FOR | lf(FOREACH_KW)),
    (b"else", W_BLOCK | lf(ELSE_KW)),
    (b"default", W_BLOCK | lf(DEFAULT_KW)),
    (b"continue", W_BLOCK | lf(CONTINUE_KW)),
    (b"return", W_KW | lf(RETURN_KW)),
    (b"die", W_KW | lf(DIE_KW)),
    (b"exit", W_KW | lf(EXIT_KW)),
    (b"caller", W_KW | lf(CALLER_KW)),
    (b"break", W_KW | lf(BREAK_KW)),
    (b"wait", W_KW | W_VALUE | lf(WAIT_KW)),
    (b"next", W_LABEL | lf(NEXT_KW)),
    (b"last", W_LABEL | lf(LAST_KW)),
    (b"redo", W_LABEL | lf(REDO_KW)),
    (b"goto", W_LABEL | lf(GOTO_KW)),
    (b"dump", W_LABEL | lf(DUMP_KW)),
    (b"do", W_DO | W_SQLM | lf(DO_KW)),
    (b"eval", W_DO | lf(SUPPORT)),
    (b"map", W_LIST | lf(SUPPORT)),
    (b"grep", W_LIST | lf(SUPPORT)),
    (b"sort", W_LIST | lf(SUPPORT)),
    (b"and", W_WORDOP | lf(LOGICAL)),
    (b"or", W_WORDOP | lf(LOGICAL)),
    (b"not", W_WORDOP | lf(LOGICAL)),
    (b"xor", W_WORDOP | lf(LOGICAL)),
    (b"eq", W_WORDOP | lf(LOGICAL)),
    (b"ne", W_WORDOP | lf(LOGICAL)),
    (b"lt", W_WORDOP | lf(LOGICAL)),
    (b"gt", W_WORDOP | lf(LOGICAL)),
    (b"le", W_WORDOP | lf(LOGICAL)),
    (b"ge", W_WORDOP | lf(LOGICAL)),
    (b"cmp", W_WORDOP | lf(LOGICAL)),
    (b"x", W_X),
    (b"q", W_QUOTE | lf(SUPPORT)),
    (b"qq", W_QUOTE | lf(SUPPORT)),
    (b"qw", W_QUOTE | lf(SUPPORT)),
    (b"qx", W_QUOTE | lf(SUPPORT)),
    (b"m", W_QUOTE | lf(SUPPORT)),
    (b"qr", W_QUOTE | lf(SUPPORT)),
    (b"s", W_QUOTE | lf(SUPPORT)),
    (b"tr", W_QUOTE | lf(SUPPORT)),
    (b"y", W_QUOTE | lf(SUPPORT)),
    (b"__END__", W_END | lf(CONST_LANG)),
    (b"__DATA__", W_END | lf(CONST_LANG)),
    (b"__FILE__", W_VALUE | lf(CONST_LANG)),
    (b"__LINE__", W_VALUE | lf(CONST_LANG)),
    (b"__PACKAGE__", W_VALUE | lf(CONST_LANG)),
    (b"__SUB__", W_VALUE | lf(CONST_LANG)),
    (b"__CLASS__", W_VALUE | lf(CONST_LANG)),
    (b"STDIN", W_FH | lf(FILEHANDLE)),
    (b"STDOUT", W_FH | lf(FILEHANDLE)),
    (b"STDERR", W_FH | lf(FILEHANDLE)),
    (b"ARGV", W_FH | W_VARLANG | lf(FILEHANDLE)),
    (b"ARGVOUT", W_FH | lf(FILEHANDLE)),
    (b"DATA", W_FH | lf(FILEHANDLE)),
    (b"IN", W_FH | lf(FILEHANDLE)),
    (b"OUT", W_FH | lf(FILEHANDLE)),
    (b"BEGIN", W_PHASE),
    (b"END", W_PHASE),
    (b"INIT", W_PHASE),
    (b"CHECK", W_PHASE),
    (b"UNITCHECK", W_PHASE),
    (b"AUTOLOAD", W_CALLBACK),
    (b"DESTROY", W_CALLBACK),
    // built-in functions
    (b"abs", W_FN | lf(SUPPORT)),
    (b"accept", W_FN | lf(SUPPORT)),
    (b"alarm", W_FN | lf(SUPPORT)),
    (b"atan2", W_FN | lf(SUPPORT)),
    (b"bind", W_FN | lf(SUPPORT)),
    (b"binmode", W_FN | lf(SUPPORT)),
    (b"bless", W_FN | lf(SUPPORT)),
    (b"chdir", W_FN | lf(SUPPORT)),
    (b"chmod", W_FN | lf(SUPPORT)),
    (b"chomp", W_FN | lf(SUPPORT)),
    (b"chop", W_FN | lf(SUPPORT)),
    (b"chown", W_FN | lf(SUPPORT)),
    (b"chr", W_FN | lf(SUPPORT)),
    (b"chroot", W_FN | lf(SUPPORT)),
    (b"close", W_FN | lf(SUPPORT)),
    (b"closedir", W_FN | lf(SUPPORT)),
    (b"connect", W_FN | lf(SUPPORT)),
    (b"cos", W_FN | lf(SUPPORT)),
    (b"crypt", W_FN | lf(SUPPORT)),
    (b"dbmclose", W_FN | lf(SUPPORT)),
    (b"dbmopen", W_FN | lf(SUPPORT)),
    (b"defined", W_FN | lf(SUPPORT)),
    (b"delete", W_FN | lf(SUPPORT)),
    (b"each", W_FN | lf(SUPPORT)),
    (b"endgrent", W_FN | W_VALUE | lf(SUPPORT)),
    (b"endhostent", W_FN | W_VALUE | lf(SUPPORT)),
    (b"endnetent", W_FN | W_VALUE | lf(SUPPORT)),
    (b"endprotoent", W_FN | W_VALUE | lf(SUPPORT)),
    (b"endpwent", W_FN | W_VALUE | lf(SUPPORT)),
    (b"endservent", W_FN | W_VALUE | lf(SUPPORT)),
    (b"eof", W_FN | lf(SUPPORT)),
    (b"evalbytes", W_FN | lf(SUPPORT)),
    (b"exec", W_FN | lf(SUPPORT)),
    (b"exists", W_FN | lf(SUPPORT)),
    (b"exp", W_FN | lf(SUPPORT)),
    (b"fc", W_FN | lf(SUPPORT)),
    (b"fcntl", W_FN | lf(SUPPORT)),
    (b"fileno", W_FN | lf(SUPPORT)),
    (b"flock", W_FN | lf(SUPPORT)),
    (b"fork", W_FN | W_VALUE | lf(SUPPORT)),
    (b"formline", W_FN | lf(SUPPORT)),
    (b"getc", W_FN | lf(SUPPORT)),
    (b"getgrent", W_FN | W_VALUE | lf(SUPPORT)),
    (b"getgrgid", W_FN | lf(SUPPORT)),
    (b"getgrnam", W_FN | lf(SUPPORT)),
    (b"gethostbyaddr", W_FN | lf(SUPPORT)),
    (b"gethostbyname", W_FN | lf(SUPPORT)),
    (b"gethostent", W_FN | W_VALUE | lf(SUPPORT)),
    (b"getlogin", W_FN | W_VALUE | lf(SUPPORT)),
    (b"getnetbyaddr", W_FN | lf(SUPPORT)),
    (b"getnetbyname", W_FN | lf(SUPPORT)),
    (b"getnetent", W_FN | W_VALUE | lf(SUPPORT)),
    (b"getpeername", W_FN | lf(SUPPORT)),
    (b"getpgrp", W_FN | lf(SUPPORT)),
    (b"getppid", W_FN | W_VALUE | lf(SUPPORT)),
    (b"getpriority", W_FN | lf(SUPPORT)),
    (b"getprotobyname", W_FN | lf(SUPPORT)),
    (b"getprotobynumber", W_FN | lf(SUPPORT)),
    (b"getprotoent", W_FN | W_VALUE | lf(SUPPORT)),
    (b"getpwent", W_FN | W_VALUE | lf(SUPPORT)),
    (b"getpwnam", W_FN | lf(SUPPORT)),
    (b"getpwuid", W_FN | lf(SUPPORT)),
    (b"getservbyname", W_FN | lf(SUPPORT)),
    (b"getservbyport", W_FN | lf(SUPPORT)),
    (b"getservent", W_FN | W_VALUE | lf(SUPPORT)),
    (b"getsockname", W_FN | lf(SUPPORT)),
    (b"getsockopt", W_FN | lf(SUPPORT)),
    (b"glob", W_FN | lf(SUPPORT)),
    (b"gmtime", W_FN | lf(SUPPORT)),
    (b"hex", W_FN | lf(SUPPORT)),
    (b"index", W_FN | lf(SUPPORT)),
    (b"int", W_FN | lf(SUPPORT)),
    (b"ioctl", W_FN | lf(SUPPORT)),
    (b"join", W_FN | lf(SUPPORT)),
    (b"keys", W_FN | lf(SUPPORT)),
    (b"kill", W_FN | lf(SUPPORT)),
    (b"lc", W_FN | lf(SUPPORT)),
    (b"lcfirst", W_FN | lf(SUPPORT)),
    (b"length", W_FN | lf(SUPPORT)),
    (b"link", W_FN | lf(SUPPORT)),
    (b"listen", W_FN | lf(SUPPORT)),
    (b"localtime", W_FN | lf(SUPPORT)),
    (b"lock", W_FN | lf(SUPPORT)),
    (b"log", W_FN | lf(SUPPORT)),
    (b"lstat", W_FN | lf(SUPPORT)),
    (b"mkdir", W_FN | lf(SUPPORT)),
    (b"msgctl", W_FN | lf(SUPPORT)),
    (b"msgget", W_FN | lf(SUPPORT)),
    (b"msgrcv", W_FN | lf(SUPPORT)),
    (b"msgsnd", W_FN | lf(SUPPORT)),
    (b"oct", W_FN | lf(SUPPORT)),
    (b"open", W_FN | lf(SUPPORT)),
    (b"opendir", W_FN | lf(SUPPORT)),
    (b"ord", W_FN | lf(SUPPORT)),
    (b"pack", W_FN | lf(SUPPORT)),
    (b"pipe", W_FN | lf(SUPPORT)),
    (b"pop", W_FN | W_VALUE | lf(SUPPORT)),
    (b"pos", W_FN | lf(SUPPORT)),
    (b"print", W_FN | lf(SUPPORT)),
    (b"printf", W_FN | lf(SUPPORT)),
    (b"prototype", W_FN | lf(SUPPORT)),
    (b"push", W_FN | lf(SUPPORT)),
    (b"quotemeta", W_FN | lf(SUPPORT)),
    (b"rand", W_FN | lf(SUPPORT)),
    (b"read", W_FN | lf(SUPPORT)),
    (b"readdir", W_FN | lf(SUPPORT)),
    (b"readline", W_FN | lf(SUPPORT)),
    (b"readlink", W_FN | lf(SUPPORT)),
    (b"readpipe", W_FN | lf(SUPPORT)),
    (b"recv", W_FN | lf(SUPPORT)),
    (b"ref", W_FN | lf(SUPPORT)),
    (b"rename", W_FN | lf(SUPPORT)),
    (b"reset", W_FN | lf(SUPPORT)),
    (b"reverse", W_FN | lf(SUPPORT)),
    (b"rewinddir", W_FN | lf(SUPPORT)),
    (b"rindex", W_FN | lf(SUPPORT)),
    (b"rmdir", W_FN | lf(SUPPORT)),
    (b"say", W_FN | lf(SUPPORT)),
    (b"scalar", W_FN | lf(SUPPORT)),
    (b"seek", W_FN | lf(SUPPORT)),
    (b"seekdir", W_FN | lf(SUPPORT)),
    (b"select", W_FN | lf(SUPPORT)),
    (b"semctl", W_FN | lf(SUPPORT)),
    (b"semget", W_FN | lf(SUPPORT)),
    (b"semop", W_FN | lf(SUPPORT)),
    (b"send", W_FN | lf(SUPPORT)),
    (b"setgrent", W_FN | W_VALUE | lf(SUPPORT)),
    (b"sethostent", W_FN | lf(SUPPORT)),
    (b"setnetent", W_FN | lf(SUPPORT)),
    (b"setpgrp", W_FN | lf(SUPPORT)),
    (b"setpriority", W_FN | lf(SUPPORT)),
    (b"setprotoent", W_FN | lf(SUPPORT)),
    (b"setpwent", W_FN | W_VALUE | lf(SUPPORT)),
    (b"setservent", W_FN | lf(SUPPORT)),
    (b"setsockopt", W_FN | lf(SUPPORT)),
    (b"shift", W_FN | W_VALUE | lf(SUPPORT)),
    (b"shmctl", W_FN | lf(SUPPORT)),
    (b"shmget", W_FN | lf(SUPPORT)),
    (b"shmread", W_FN | lf(SUPPORT)),
    (b"shmwrite", W_FN | lf(SUPPORT)),
    (b"shutdown", W_FN | lf(SUPPORT)),
    (b"sin", W_FN | lf(SUPPORT)),
    (b"sleep", W_FN | lf(SUPPORT)),
    (b"socket", W_FN | lf(SUPPORT)),
    (b"socketpair", W_FN | lf(SUPPORT)),
    (b"splice", W_FN | lf(SUPPORT)),
    (b"split", W_FN | lf(SUPPORT)),
    (b"sprintf", W_FN | lf(SUPPORT)),
    (b"sqrt", W_FN | lf(SUPPORT)),
    (b"srand", W_FN | lf(SUPPORT)),
    (b"stat", W_FN | lf(SUPPORT)),
    (b"study", W_FN | lf(SUPPORT)),
    (b"substr", W_FN | lf(SUPPORT)),
    (b"symlink", W_FN | lf(SUPPORT)),
    (b"syscall", W_FN | lf(SUPPORT)),
    (b"sysopen", W_FN | lf(SUPPORT)),
    (b"sysread", W_FN | lf(SUPPORT)),
    (b"sysseek", W_FN | lf(SUPPORT)),
    (b"system", W_FN | lf(SUPPORT)),
    (b"syswrite", W_FN | lf(SUPPORT)),
    (b"tell", W_FN | lf(SUPPORT)),
    (b"telldir", W_FN | lf(SUPPORT)),
    (b"tie", W_FN | lf(SUPPORT)),
    (b"tied", W_FN | lf(SUPPORT)),
    (b"time", W_FN | W_VALUE | lf(SUPPORT)),
    (b"times", W_FN | W_VALUE | lf(SUPPORT)),
    (b"truncate", W_FN | lf(SUPPORT)),
    (b"uc", W_FN | lf(SUPPORT)),
    (b"ucfirst", W_FN | lf(SUPPORT)),
    (b"umask", W_FN | lf(SUPPORT)),
    (b"undef", W_FN | lf(SUPPORT)),
    (b"unlink", W_FN | lf(SUPPORT)),
    (b"unpack", W_FN | lf(SUPPORT)),
    (b"unshift", W_FN | lf(SUPPORT)),
    (b"untie", W_FN | lf(SUPPORT)),
    (b"utime", W_FN | lf(SUPPORT)),
    (b"values", W_FN | lf(SUPPORT)),
    (b"vec", W_FN | lf(SUPPORT)),
    (b"waitpid", W_FN | lf(SUPPORT)),
    (b"wantarray", W_FN | W_VALUE | lf(SUPPORT)),
    (b"warn", W_FN | lf(SUPPORT)),
    (b"write", W_FN | lf(SUPPORT)),
    // special variables' names after a sigil (bat's list; `$a` and `$b` are `$`'s only)
    (b"_", W_VARLANG),
    (b"a", W_VARLANG),
    (b"b", W_VARLANG),
    (b"F", W_VARLANG),
    (b"ENV", W_VARLANG),
    (b"INC", W_VARLANG),
    (b"ISA", W_VARLANG),
    (b"SIG", W_VARLANG),
    (b"ARG", W_VARLANG),
    (b"ARGC", W_VARLANG),
    (b"ACCUMULATOR", W_VARLANG),
    (b"BASETIME", W_VARLANG),
    (b"CHILD_ERROR", W_VARLANG),
    (b"COMPILING", W_VARLANG),
    (b"DEBUGGING", W_VARLANG),
    (b"EFFECTIVE_GROUP_ID", W_VARLANG),
    (b"EFFECTIVE_USER_ID", W_VARLANG),
    (b"EGID", W_VARLANG),
    (b"ERRNO", W_VARLANG),
    (b"EUID", W_VARLANG),
    (b"EVAL_ERROR", W_VARLANG),
    (b"EXCEPTIONS_BEING_CAUGHT", W_VARLANG),
    (b"EXECUTABLE_NAME", W_VARLANG),
    (b"EXTENDED_OS_ERROR", W_VARLANG),
    (b"FORMAT_FORMFEED", W_VARLANG),
    (b"FORMAT_LINE_BREAK_CHARACTERS", W_VARLANG),
    (b"FORMAT_LINES_LEFT", W_VARLANG),
    (b"FORMAT_LINES_PER_PAGE", W_VARLANG),
    (b"FORMAT_NAME", W_VARLANG),
    (b"FORMAT_PAGE_NUMBER", W_VARLANG),
    (b"FORMAT_TOP_NAME", W_VARLANG),
    (b"GID", W_VARLANG),
    (b"INPLACE_EDIT", W_VARLANG),
    (b"INPUT_LINE_NUMBER", W_VARLANG),
    (b"INPUT_RECORD_SEPARATOR", W_VARLANG),
    (b"LAST_MATCH_END", W_VARLANG),
    (b"LAST_MATCH_START", W_VARLANG),
    (b"LAST_PAREN_MATCH", W_VARLANG),
    (b"LAST_REGEXP_CODE_RESULT", W_VARLANG),
    (b"LAST_SUBMATCH_RESULT", W_VARLANG),
    (b"LIST_SEPARATOR", W_VARLANG),
    (b"MATCH", W_VARLANG),
    (b"NR", W_VARLANG),
    (b"OFS", W_VARLANG),
    (b"OLD_PERL_VERSION", W_VARLANG),
    (b"ORS", W_VARLANG),
    (b"OS_ERROR", W_VARLANG),
    (b"OSNAME", W_VARLANG),
    (b"OUTPUT_AUTOFLUSH", W_VARLANG),
    (b"OUTPUT_FIELD_SEPARATOR", W_VARLANG),
    (b"OUTPUT_RECORD_SEPARATOR", W_VARLANG),
    (b"PERL_VERSION", W_VARLANG),
    (b"PERLDB", W_VARLANG),
    (b"PID", W_VARLANG),
    (b"POSTMATCH", W_VARLANG),
    (b"PREMATCH", W_VARLANG),
    (b"PROCESS_ID", W_VARLANG),
    (b"PROGRAM_NAME", W_VARLANG),
    (b"REAL_GROUP_ID", W_VARLANG),
    (b"REAL_USER_ID", W_VARLANG),
    (b"RS", W_VARLANG),
    (b"SUBSCRIPT_SEPARATOR", W_VARLANG),
    (b"SUBSEP", W_VARLANG),
    (b"SYSTEM_FD_MAX", W_VARLANG),
    (b"UID", W_VARLANG),
    (b"WARNING", W_VARLANG),
    // DBI's methods that take SQL ([`W_SQLM`]; `do` above)
    (b"prepare", W_SQLM),
    (b"prepare_cached", W_SQLM),
    (b"selectall_arrayref", W_SQLM),
    (b"selectall_hashref", W_SQLM),
    (b"selectall_array", W_SQLM),
    (b"selectrow_array", W_SQLM),
    (b"selectrow_arrayref", W_SQLM),
    (b"selectrow_hashref", W_SQLM),
    (b"selectcol_arrayref", W_SQLM),
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

/// A word's leaf kind from its info, 0 for none.
#[inline(always)]
const fn leaf_of(w: u16) -> u16 {
    let l = w & W_LEAF;
    if l == 0 { 0 } else { LEAF + l - 1 }
}

// The frames, four bits a level in [`Scan::frames`]; 0 is none (the top level, or a level lost
// past [`LEVELS`]).
/// `{` … `}`: a block, whose `}` ends a statement.
const F_BLOCK: u8 = 1;
/// An anonymous sub's, `do`'s or `eval`'s block: its `}` ends a value.
const F_TBLOCK: u8 = 2;
/// `map`'s, `grep`'s or `sort`'s block (or a call's that takes one): its `}` leaves a term to
/// come.
const F_LBLOCK: u8 = 3;
/// An anonymous hash.
const F_HASH: u8 = 4;
/// A subscript's `{` … `}`.
const F_SUBSCRIPT: u8 = 5;
/// A dereference's `${` … `}`, `@{` … `}`.
const F_DEREF: u8 = 6;
/// `(` … `)`.
const F_PAREN: u8 = 7;
/// A condition's `(` … `)`: a block's `{` after it.
const F_COND: u8 = 8;
/// A sub's signature or prototype.
const F_SIG: u8 = 9;
/// A DBI call's arguments ([`Scan::sqls`] the first's state).
const F_SQL: u8 = 10;
/// `[` … `]`: an anonymous array.
const F_BRACKET: u8 = 11;
/// A subscript's `[` … `]`.
const F_INDEX: u8 = 12;
/// The bits of a level.
const FW: u32 = 4;
const FMASK: u128 = (1 << FW) - 1;
/// The levels [`Scan::frames`] holds: fourteen bytes of [`State::levels`].
const LEVELS: u8 = 28;
/// The kept levels' bits.
const KEPT: u128 = (1 << (FW * LEVELS as u32)) - 1;

/// Whether frame `f` is closed by `}`.
#[inline(always)]
const fn is_brace(f: u8) -> bool {
    f >= F_BLOCK && f <= F_DEREF
}

/// Whether frame `f` is closed by `)`.
#[inline(always)]
const fn is_paren(f: u8) -> bool {
    f >= F_PAREN && f <= F_SQL
}

// What Perl's lexer expects ([`Scan::prev`]).
/// A statement's start.
const P_START: u8 = 0;
/// A term.
const P_TERM: u8 = 1;
/// An operator: a value ended (a variable, a literal, `)`, `]`, a subscript's `}`).
const P_VALUE: u8 = 2;
/// After `->`: a method's name, a subscript, a postfix dereference.
const P_ARROW: u8 = 3;
/// After a condition's keyword: its `(`, or a term (a statement modifier's).
const P_COND: u8 = 4;
/// A block's `{` next: after a condition's `)`, `else`, a phase's name.
const P_BLOCK: u8 = 5;
/// After `sub`: a name, a signature, attributes, or an anonymous sub's `{`.
const P_SUB: u8 = 6;
/// After a sub's name or signature: a signature, attributes, its block.
const P_SUBNAME: u8 = 7;
/// After `map`, `grep`, `sort`: a block whose `}` leaves a term to come.
const P_LIST: u8 = 8;
/// After `do`, `eval`: a block whose `}` ends a value.
const P_ANON: u8 = 9;
/// After `package`, `use`, `no`, `require`: a module's name or a version.
const P_PKG: u8 = 10;
/// After `next`, `last`, `redo`, `goto`, `dump`: a label.
const P_LABEL: u8 = 11;
/// After a bareword read as a call: a term, but `/ ` a divide (Perl's guess).
const P_FUNC: u8 = 12;
/// After a module's name in `use`: a version, or the import list.
const P_MODULE: u8 = 13;
/// In a signature or prototype where a parameter starts: after its `(` or a `,`.
const P_PARAM: u8 = 14;
/// After a bareword in capitals: a value (`PI / 2`), but a filehandle's here-document after it
/// (`print OUT <<EOT`).
const P_CONST: u8 = 15;

// What a line starts in ([`Scan::mode`]).
const M_CODE: u8 = 0;
/// POD, to `=cut`.
const M_POD: u8 = 1;
/// After `__END__` or `__DATA__`: text, but POD.
const M_DATA: u8 = 2;
/// A quote-like construct ([`Scan::qk`]).
const M_QUOTE: u8 = 3;
/// A here-document's body: the queue's first.
const M_HEREDOC: u8 = 4;
/// `format`'s pictures, to a `.` line.
const M_FORMAT: u8 = 5;
/// A quote-like operator whose delimiter is on a later line ([`Scan::qk`] its construct).
const M_QPEND: u8 = 6;

// [`State::arg`]: the mode, what's expected, flags.
const ARG_MODE: u16 = 7;
const ARG_PREV: u32 = 3;
/// `for`'s or `foreach`'s condition's `(` not yet met.
const ARG_FOR: u16 = 1 << 7;
/// POD in `__END__`'s text: the text again after `=cut`.
const ARG_PODDATA: u16 = 1 << 8;
/// The here-documents waiting, two bits.
const ARG_QLEN: u32 = 9;

/// The here-documents a line may open that wait for their bodies.
const QUEUED: usize = 2;
/// How deep a string's interpolated code may nest strings with code of their own.
const NEST: u8 = 6;

// A guest's body open at a line's end (X-191): the state is the guest's, Perl's own fields one
// word in `withs[6..14]` (little-endian), which none of the five guests' lines set (X-189's
// footprint census, `x189/foot.txt`).
/// The frames a body keeps beside its guest, four bits each from bit 0.
const G_LEVELS: u8 = 5;
/// A here-document's label, 28 bits from here; a POD region's [`G_FOR`] and [`G_PODDATA`] in
/// its place.
const G_LABEL: u32 = 20;
const G_FOR: u64 = 1 << 20;
const G_PODDATA: u64 = 1 << 21;
const G_INTERP: u64 = 1 << 48;
const G_INDENT: u64 = 1 << 49;
/// The guest's language, 3 bits ([`GL_LANG`]).
const G_LANG: u32 = 50;
/// The guest's state lost: its lines each lexed from a fresh one to the body's end.
const G_LOST: u64 = 1 << 53;
const G_FORVAR: u64 = 1 << 54;
/// The layout's mark: `withs[12]`'s top bit, which Perl's own (a DBI call's argument, 0 or 1)
/// never sets.
const G_ON: u64 = 1 << 55;
const G_PREV: u32 = 56;
const G_DEPTH: u32 = 60;
/// A POD region's, not a here-document's.
const G_POD: u64 = 1 << 63;
// [`Scan::glang`]: the language (`text::label_lang`'s, 2–6), a POD region's, `=for`'s.
const GL_LANG: u8 = 7;
const GL_POD: u8 = 8;
const GL_FOR: u8 = 16;

/// The scratch a guest's line is lexed with ([`Scan::glang`]), a thread's, taken by the line, so
/// the scan, made at every line, holds no field to drop or allocate.
#[derive(Default)]
struct Guest {
    side: Side,
    /// The line as the guest reads it: what isn't its text blanked.
    buf: Vec<u8>,
    /// The line's escapes and interpolated variables, Perl's.
    isl: Vec<(u32, u32)>,
}

std::thread_local! {
    static GUEST: std::cell::RefCell<Guest> = std::cell::RefCell::new(Guest::default());
}

/// Lex `b`, a line of Perl without its `\n`, from `state`.
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
    s.save(state);
    s.k
}

/// A block of Perl on one scan opened from `state` ([`crate::Carry`]).
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
    s.save(state);
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
    s.save(state);
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
    /// What each open frame is ([`F_BLOCK`]…), four bits a level, the innermost lowest; past
    /// [`LEVELS`] the outer ones are lost.
    frames: u128,
    /// How many frames are open, up to 255.
    depth: u8,
    /// What the line starts in ([`M_CODE`]…).
    mode: u8,
    /// What Perl's lexer expects ([`P_START`]…), carried across lines.
    prev: u8,
    /// `for`'s condition not yet opened ([`ARG_FOR`]).
    forvar: bool,
    /// POD in `__END__`'s text ([`ARG_PODDATA`]).
    poddata: bool,
    /// The open quote-like construct ([`text`]'s `Q_*` and flags), 0 for none.
    qk: u8,
    /// Its closing delimiter.
    qdelim: u8,
    /// Its nesting, where its delimiters are brackets.
    qnest: u8,
    /// A regex's state or SQL's context, between a construct's pieces and lines.
    ctx: u8,
    /// The depth plus one a `use`, `no` or `require` statement opened at; 0 for none.
    pp: u8,
    /// The innermost DBI call's first argument ([`text`]'s `S_*`).
    sqls: u8,
    /// The here-documents waiting for their bodies.
    queue: Queue<QUEUED>,
    /// The open body's guest ([`GL_LANG`], [`GL_POD`], [`GL_FOR`]), 0 for none.
    glang: u8,
    /// The language of the here-document last queued first, for [`Scan::bodies_next`].
    qlang: u8,
    /// The guest's state at the line's start ([`Scan::glang`]), its bytes in `withs[6..14]` zero.
    gst: State,
    /// [`G_LOST`].
    glost: bool,
    // The line's own, reset at its start:
    /// Where a `use` statement's text was last written to ([`Scan::pp`]).
    pp_at: usize,
    /// What the `(` right after the last name opens: [`F_SQL`], or 0.
    call: u8,
    /// How deep the interpolated code being lexed sits ([`NEST`]).
    nest: u8,
    /// What SQL's scan keeps between the host's turns on a line.
    sql: crate::sql::Embedded,
    k: K,
    out: &'b mut Out<'c>,
}

/// Whether `c` is one of a name's bytes: `[A-Za-z0-9_]`, or past ASCII.
#[inline(always)]
pub(super) const fn is_name_byte(c: u8) -> bool {
    WORD[c as usize] || c >= 0x80
}

/// Whether `c` can start a name: `[A-Za-z_]`, or past ASCII.
#[inline(always)]
pub(super) const fn is_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

/// Where the run of a name's bytes from `from` ends: sixteen bytes a step on x86-64 (SSE2's
/// compares, the line's last step read again from its end less sixteen; Dart's), a byte a step
/// elsewhere.
#[inline(always)]
pub(super) fn name_run(b: &[u8], from: usize) -> usize {
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
                // `_`, or a byte past ASCII (negative as a signed byte)
                let us = _mm_or_si128(
                    _mm_cmpeq_epi8(w, _mm_set1_epi8(b'_' as i8)),
                    _mm_cmplt_epi8(w, _mm_setzero_si128()),
                );
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
    while e < n && is_name_byte(b[e]) {
        e += 1;
    }
    e
}

/// Whether the `<<` at `p`, after a value, opens a here-document because the value is a
/// filehandle: `print`'s, `printf`'s or `say`'s scalar after a blank, or its block within
/// [`HANDLE_MAX`] bytes (`print $fh <<"EOT"`, `print {$out} <<EOT`), then a blank, and after `<<`
/// a byte neither a blank nor `=`, as Perl's lexer (toke.c) reads them; else a shift (`$x << 2`,
/// `$x <<= 1`). Read back on the line, only where `<<` follows a value.
/// How far back [`print_handle`] looks for a block's `{`, so a line of `} <<x` stays linear.
const HANDLE_MAX: usize = 256;

#[cold]
#[inline(never)]
fn print_handle(b: &[u8], p: usize) -> bool {
    if matches!(b.get(p + 2), None | Some(b' ' | b'\t' | b'=')) {
        return false;
    }
    let blanks = |mut q: usize| {
        while q > 0 && matches!(b[q - 1], b' ' | b'\t') {
            q -= 1;
        }
        q
    };
    let name = |mut q: usize| {
        while q > 0 && is_name_byte(b[q - 1]) {
            q -= 1;
        }
        q
    };
    let ve = blanks(p);
    if ve == p || ve == 0 {
        return false;
    }
    // the handle: a scalar after a blank, or a block (`print {$fh} <<EOT`)
    let (h, blank) = if b[ve - 1] == b'}' {
        let mut depth = 0u32;
        let mut q = ve;
        loop {
            if q == 0 || ve - q > HANDLE_MAX {
                return false;
            }
            q -= 1;
            match b[q] {
                b'}' => depth += 1,
                b'{' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        (q, false)
    } else {
        let v = name(ve);
        if v == ve || v == 0 || b[v - 1] != b'$' {
            return false;
        }
        (v - 1, true)
    };
    let we = blanks(h);
    let w = name(we);
    (!blank || we < h)
        && matches!(&b[w..we], b"print" | b"printf" | b"say")
        && (w == 0 || !(is_name_byte(b[w - 1]) || b"$@%&:>".contains(&b[w - 1])))
}

/// The first byte at or past `p` that isn't a blank, or the line's end.
#[inline(always)]
pub(super) fn skip_blanks(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && matches!(b[p], b' ' | b'\t' | b'\r' | 0x0b | 0x0c) {
        p += 1;
    }
    p
}

/// Whether `b[p..]` is `=>` after blanks: a bareword before it is a string.
#[inline(always)]
fn fat_comma(b: &[u8], p: usize) -> bool {
    let q = skip_blanks(b, p);
    b.get(q) == Some(&b'=') && b.get(q + 1) == Some(&b'>')
}

/// Whether the name `b[p..e]` is in capitals, digits and `_`, two bytes or more, a capital
/// first: bat's constant.
#[inline(always)]
fn all_caps(b: &[u8], p: usize, e: usize) -> bool {
    e - p >= 2
        && (b[p].is_ascii_uppercase() || b[p] == b'_')
        && b[p..e]
            .iter()
            .all(|&c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
        && b[p..e].iter().any(u8::is_ascii_uppercase)
}

/// Where a qualified name's part from `p` ends, past its `::`s (`Foo::Bar::baz`): the end of
/// its last word, or past a trailing `::`.
#[inline(always)]
fn qualified_end(b: &[u8], mut e: usize) -> usize {
    while b.get(e) == Some(&b':') && b.get(e + 1) == Some(&b':') {
        e += 2;
        if b.get(e).is_some_and(|&c| is_name_start(c)) {
            e = name_run(b, e + 1);
        }
    }
    e
}

impl<'b, 'c, K: Sink> Scan<'b, 'c, K> {
    #[inline(always)]
    fn new(b: &'b [u8], out: &'b mut Out<'c>, k: K) -> Self {
        Scan {
            b,
            frames: 0,
            depth: 0,
            mode: M_CODE,
            prev: P_START,
            forvar: false,
            poddata: false,
            qk: 0,
            qdelim: 0,
            qnest: 0,
            ctx: 0,
            pp: 0,
            sqls: 0,
            queue: Queue::new(),
            glang: 0,
            qlang: 0,
            gst: State::default(),
            glost: false,
            pp_at: 0,
            call: 0,
            nest: 0,
            sql: crate::sql::Embedded {
                is_end: usize::MAX,
                at: 0,
            },
            k,
            out,
        }
    }

    /// The scan's fields from `state`: the frames in its levels, the first here-document
    /// waiting in the first four withs, the open construct, its delimiter, nesting and context,
    /// a `use` statement's depth and a DBI call's argument in the others but those markdown's
    /// fences keep (`withs[6]`, `[8]`, `[9]`), which only a second here-document waiting sets;
    /// the mode, what's expected and flags in [`State::arg`].
    #[inline(always)]
    fn load(&mut self, state: &State) {
        if state.withs[12] & 0x80 != 0 {
            return self.load_guest(state);
        }
        let mut w = [0u8; 16];
        w[..14].copy_from_slice(&state.levels);
        self.frames = u128::from_le_bytes(w);
        self.depth = state.depth;
        let a = state.arg;
        self.mode = (a & ARG_MODE) as u8;
        self.prev = ((a >> ARG_PREV) & 0xf) as u8;
        self.forvar = a & ARG_FOR != 0;
        self.poddata = a & ARG_PODDATA != 0;
        let qlen = usize::from((a >> ARG_QLEN) & 3);
        if qlen != 0 {
            self.load_queue(state, qlen);
        }
        self.qk = state.withs[4];
        self.qdelim = state.withs[5];
        self.qnest = state.withs[7];
        self.ctx = state.withs[10];
        self.pp = state.withs[11];
        self.sqls = state.withs[12];
    }

    #[cold]
    fn load_queue(&mut self, state: &State, len: usize) {
        let w = &state.withs;
        let q = [w[0], w[1], w[2], w[3], w[6], w[8], w[9], w[13]];
        self.queue = Queue::load(&q, len.min(QUEUED));
    }

    /// A body's line from `state` ([`G_ON`]): Perl's fields from its word, the guest's state
    /// the rest.
    #[cold]
    #[inline(never)]
    fn load_guest(&mut self, state: &State) {
        let mut w = [0u8; 8];
        w.copy_from_slice(&state.withs[6..14]);
        let v = u64::from_le_bytes(w);
        self.frames = u128::from(v & ((1 << (FW * u32::from(G_LEVELS))) - 1));
        self.depth = ((v >> G_DEPTH) & 7) as u8;
        self.prev = ((v >> G_PREV) & 15) as u8;
        self.forvar = v & G_FORVAR != 0;
        let lang = ((v >> G_LANG) & 7) as u8;
        if v & G_POD != 0 {
            self.mode = M_POD;
            self.poddata = v & G_PODDATA != 0;
            self.qk = text::POD_PARA;
            self.glang = lang | GL_POD | if v & G_FOR != 0 { GL_FOR } else { 0 };
        } else {
            self.mode = M_HEREDOC;
            let mut flags = text::H_EMBED;
            if v & G_INTERP != 0 {
                flags |= text::H_INTERP;
            }
            if v & G_INDENT != 0 {
                flags |= text::H_INDENT;
            }
            self.queue
                .push(Label::from_bits((v >> G_LABEL) as u32), flags);
            self.glang = lang;
        }
        self.gst = *state;
        self.gst.withs[6..14].fill(0);
        self.glost = v & G_LOST != 0;
    }

    /// The state a body's line leaves ([`Scan::glang`]): the guest's, Perl's fields in its word.
    #[cold]
    #[inline(never)]
    fn save_guest(&self, s: &mut State) {
        *s = if self.glost {
            State::default()
        } else {
            self.gst
        };
        let mut v = (self.frames as u64) & ((1 << (FW * u32::from(G_LEVELS))) - 1)
            | u64::from(self.depth) << G_DEPTH
            | u64::from(self.prev) << G_PREV
            | u64::from(self.glang & GL_LANG) << G_LANG
            | G_ON
            | if self.glost { G_LOST } else { 0 }
            | if self.forvar { G_FORVAR } else { 0 };
        if self.glang & GL_POD != 0 {
            v |= G_POD
                | if self.glang & GL_FOR != 0 { G_FOR } else { 0 }
                | if self.poddata { G_PODDATA } else { 0 };
        } else if let Some((label, flags)) = self.queue.front() {
            v |= u64::from(label.bits()) << G_LABEL
                | if flags & text::H_INTERP != 0 {
                    G_INTERP
                } else {
                    0
                }
                | if flags & text::H_INDENT != 0 {
                    G_INDENT
                } else {
                    0
                };
        }
        s.withs[6..14].copy_from_slice(&v.to_le_bytes());
    }

    /// Whether Perl's fields fit beside a guest's state ([`G_LEVELS`]'s frames, no `use`
    /// statement or DBI call open, no second here-document).
    #[inline(always)]
    fn guest_fits(&self) -> bool {
        self.depth <= G_LEVELS
            && self.frames >> (FW * u32::from(G_LEVELS)) == 0
            && self.pp == 0
            && self.sqls == 0
            && self.queue.len() <= 1
    }

    /// A body in guest `glang` opens: its state fresh.
    fn guest_open(&mut self, glang: u8) {
        self.glang = glang;
        self.gst = State::default();
        self.glost = false;
    }

    /// The state the scan's last line left, written to `s`: every field as it is, so a block lexes as its
    /// lines do one at a time.
    #[inline(always)]
    fn save(&self, s: &mut State) {
        if self.glang != 0 {
            return self.save_guest(s);
        }
        let w = (self.frames & KEPT).to_le_bytes();
        *s = State::default();
        s.levels.copy_from_slice(&w[..14]);
        s.depth = self.depth;
        s.withs[4] = self.qk;
        s.withs[5] = self.qdelim;
        s.withs[7] = self.qnest;
        s.withs[10] = self.ctx;
        s.withs[11] = self.pp;
        s.withs[12] = self.sqls;
        let qlen = self.queue.len();
        if qlen != 0 {
            let mut q = [0u8; 8];
            self.queue.save(&mut q);
            let w = &mut s.withs;
            (w[0], w[1], w[2], w[3]) = (q[0], q[1], q[2], q[3]);
            (w[6], w[8], w[9], w[13]) = (q[4], q[5], q[6], q[7]);
        }
        s.arg = u16::from(self.mode)
            | u16::from(self.prev) << ARG_PREV
            | if self.forvar { ARG_FOR } else { 0 }
            | if self.poddata { ARG_PODDATA } else { 0 }
            | (qlen as u16) << ARG_QLEN;
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
        if self.pp != 0 {
            self.pp_fill(s, e);
        }
        self.k.merge_call(&mut self.out, s, e, kind);
    }

    /// A write inlined, for the common tokens.
    #[inline(always)]
    fn put_hot(&mut self, s: usize, e: usize, kind: u16) {
        if self.pp != 0 {
            self.pp_fill(s, e);
        }
        self.k.merge(&mut self.out, s, e, kind);
    }

    /// A `use` statement's text up to `s`, before a write that ends at `e`.
    #[cold]
    #[inline(never)]
    fn pp_fill(&mut self, s: usize, e: usize) {
        if s > self.pp_at {
            self.k.merge_call(&mut self.out, self.pp_at, s, PP);
        }
        if e > self.pp_at {
            self.pp_at = e;
        }
    }

    /// A `use` statement ends at `p`: its text filled to there.
    #[cold]
    fn pp_end(&mut self, p: usize) {
        self.pp_fill(p, p);
        self.pp = 0;
    }

    /// Writes made past [`Scan::put`] (an embedded scanner's) end at `e`.
    #[inline(always)]
    fn wrote(&mut self, e: usize) {
        if e > self.pp_at {
            self.pp_at = e;
        }
    }

    #[inline(always)]
    fn top(&self) -> u8 {
        (self.frames & FMASK) as u8
    }

    #[inline(always)]
    fn push(&mut self, f: u8) {
        // masked to the levels `State` keeps, so a block's scan loses what its lines lose
        self.frames = (self.frames << FW | u128::from(f)) & KEPT;
        self.depth = self.depth.saturating_add(1);
    }

    #[inline(always)]
    fn pop(&mut self) {
        if self.depth > 0 {
            self.frames >>= FW;
            self.depth -= 1;
            if self.pp > self.depth + 1 {
                // a closer the `use` statement didn't open ends it
                self.pp = 0;
            }
        }
    }

    /// Whether a term may start here: not after a value or `->`.
    #[inline(always)]
    fn term(&self) -> bool {
        !matches!(self.prev, P_VALUE | P_ARROW | P_CONST)
    }

    /// The line, from what the line before left: a mode's rest first; the loop out of line
    /// where `SHARED` ([`Scan::code_shared`]).
    #[inline(always)]
    fn lex_line<const SHARED: bool>(&mut self) {
        self.pp_at = 0;
        self.call = 0;
        let p = match self.mode {
            M_CODE => {
                let b = self.b;
                if b.first() == Some(&b'=') && b.get(1).is_some_and(u8::is_ascii_alphabetic) {
                    self.pod_open();
                    return;
                }
                0
            }
            _ => match self.rest_of_mode() {
                Some(p) => p,
                None => return,
            },
        };
        if SHARED {
            self.code_shared(p);
        } else {
            self.code(p);
        }
        self.line_end();
    }

    /// A line in a mode other than code: where the code goes on, or `None` for none.
    #[cold]
    #[inline(never)]
    fn rest_of_mode(&mut self) -> Option<usize> {
        match self.mode {
            M_POD => {
                self.pod_line();
                None
            }
            M_DATA => {
                let b = self.b;
                if b.first() == Some(&b'=') && b.get(1).is_some_and(u8::is_ascii_alphabetic) {
                    self.poddata = true;
                    self.pod_open();
                }
                None
            }
            M_HEREDOC => {
                self.heredoc_line();
                None
            }
            M_FORMAT => {
                self.format_line();
                None
            }
            M_QPEND => {
                let p = self.quote_pending();
                self.after_quote(p)
            }
            _ => {
                let p = self.quote_body(0);
                self.after_quote(p)
            }
        }
    }

    /// Where the code goes on after a construct's piece ended at `p`, `None` past the line.
    #[inline(always)]
    fn after_quote(&mut self, p: usize) -> Option<usize> {
        if self.mode == M_CODE {
            Some(p)
        } else {
            self.line_end();
            None
        }
    }

    /// What the line's end leaves: a `use` statement's text filled, here-documents' bodies next.
    #[inline(always)]
    fn line_end(&mut self) {
        if self.pp != 0 {
            let n = self.b.len();
            self.pp_fill(n, n);
            if self.prev == P_VALUE && self.pp == self.depth.saturating_add(1) && !self.b.is_empty()
            {
                // a line ended at the statement's depth after a value, its `;` lost (Ruby's
                // `require File.join(…)` in a `.pl`): its meta scope ends
                self.pp = 0;
            }
        }
        if !self.queue.is_empty() && self.mode != M_HEREDOC {
            self.bodies_next();
        }
    }

    /// Here-documents wait at the line's end: their bodies come next; a construct left open on
    /// the line is dropped (Perl reads it on after the bodies). The first body is its guest's
    /// where Perl's fields fit beside the guest's state (X-191).
    #[cold]
    fn bodies_next(&mut self) {
        if matches!(self.mode, M_CODE | M_QUOTE | M_QPEND) {
            self.mode = M_HEREDOC;
            self.qk = 0;
            self.ctx = 0;
            if !SOL
                && self.queue.len() == 1
                && self
                    .queue
                    .front()
                    .is_some_and(|(_, f)| f & text::H_EMBED != 0)
                && self.guest_fits()
                && !self.poddata
            {
                (self.qdelim, self.qnest) = (0, 0);
                self.guest_open(self.qlang);
            }
        }
    }

    /// [`Scan::code`] out of line: on the run sink, one copy that the line API, a block's lines
    /// and a string's interpolated code share; on the span sink, which ada doesn't hold, too.
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
                A_SPACE => {
                    let mut e = p + 1;
                    while e < n && ACT[usize::from(b[e])] == A_SPACE {
                        e += 1;
                    }
                    e
                }
                A_NAME => self.name(p),
                A_DOLLAR => self.dollar(p),
                A_DIGIT => self.number(p),
                A_QUOTE => self.quote(p),
                A_LPAREN => self.lparen(p),
                A_RPAREN => self.rparen(p),
                A_LBRACE => self.lbrace(p),
                A_RBRACE => self.rbrace(p),
                A_LBRACKET => self.lbracket(p),
                A_RBRACKET => self.rbracket(p),
                A_COMMA => {
                    self.put_hot(p, p + 1, COMMA);
                    self.prev = P_TERM;
                    match self.top() {
                        F_SQL => self.sqls = text::S_DONE,
                        F_SIG => self.prev = P_PARAM,
                        _ => {}
                    }
                    p + 1
                }
                A_SEMI => self.semi(p),
                A_MINUS => self.minus(p),
                A_OP => self.op(p),
                A_AT => self.at(p),
                A_PERCENT => self.percent(p),
                A_AMP => self.amp(p),
                A_STAR => self.star(p),
                A_SLASH => self.slash(p),
                A_LT => self.lt(p),
                A_DOT => self.dot(p),
                A_HASH => self.comment(p),
                A_BSLASH if self.prev == P_PARAM => self.sigil(p),
                A_BSLASH => {
                    let mut e = p + 1;
                    while e < n && b[e] == b'\\' {
                        e += 1;
                    }
                    self.put_hot(p, e, REFOP);
                    self.prev = P_TERM;
                    e
                }
                _ => {
                    self.prev = P_TERM;
                    p + 1
                }
            };
            debug_assert!(p > from, "Perl's code loop stood at {from}");
        }
    }

    /// `n` bytes at `p` an operator of `kind`, a term next.
    #[inline(always)]
    fn op_token(&mut self, p: usize, n: usize, kind: u16) -> usize {
        self.put(p, p + n, kind);
        self.prev = P_TERM;
        p + n
    }

    /// `;` at `p`: a statement's end.
    #[inline(always)]
    fn semi(&mut self, p: usize) -> usize {
        if self.top() == F_SIG {
            // a prototype's `;`
            self.put(p, p + 1, COMMA);
            self.prev = P_PARAM;
            return p + 1;
        }
        if self.pp != 0 {
            self.pp_end(p);
        }
        self.put_hot(p, p + 1, SEMI);
        self.prev = P_START;
        self.forvar = false;
        p + 1
    }

    /// An operator at `p` (`+ = ! ~ > | ^ ? :`), by Perl's longest match, in bat's leaves.
    #[inline(always)]
    fn op(&mut self, p: usize) -> usize {
        let b = self.b;
        let c = b[p];
        let at = |i: usize| b.get(p + i).copied().unwrap_or(b'\n');
        let c1 = at(1);
        let (n, kind) = match c {
            b'=' => match c1 {
                b'=' => (2, COMPARE),
                b'~' => (2, BINARY),
                b'>' => (2, KEYVALUE),
                _ => (1, ASSIGN),
            },
            b'!' => match c1 {
                b'=' => (2, COMPARE),
                b'~' if at(2) == b'~' => (3, LOGICAL),
                b'~' => (2, BINARY),
                _ => (1, LOGICAL),
            },
            b'>' => match c1 {
                b'>' if at(2) == b'=' => (3, ASSIGN),
                b'>' => (2, BITWISE),
                b'=' => (2, COMPARE),
                _ => (1, COMPARE),
            },
            b'|' => match c1 {
                b'|' if at(2) == b'=' => (3, ASSIGN),
                b'|' => (2, LOGICAL),
                b'=' => (2, ASSIGN),
                b'.' if at(2) == b'=' => (3, ASSIGN),
                b'.' => (2, BITWISE),
                _ => (1, BITWISE),
            },
            b'^' => match c1 {
                b'^' if at(2) == b'=' => (3, ASSIGN),
                b'^' => (2, LOGICAL),
                b'=' => (2, ASSIGN),
                b'.' if at(2) == b'=' => (3, ASSIGN),
                b'.' => (2, BITWISE),
                _ => (1, BITWISE),
            },
            b'~' => match c1 {
                b'~' => (2, LOGICAL),
                b'.' => (2, BITWISE),
                _ => (1, BITWISE),
            },
            b'+' => match c1 {
                b'+' => {
                    // `$i++` ends a value; `++$i` a term next
                    self.put_hot(p, p + 2, ARITH);
                    if self.prev != P_VALUE {
                        self.prev = P_TERM;
                    }
                    return p + 2;
                }
                b'=' => (2, ASSIGN),
                _ => (1, ARITH),
            },
            b'?' => (1, LOGICAL),
            // `:`: `::` a package's (`::foo()`), else the ternary's or an attribute's
            _ => {
                if c1 == b':' {
                    return self.name(p);
                }
                if !SOL && matches!(self.prev, P_SUB | P_SUBNAME) {
                    return self.attributes(p);
                }
                (1, LOGICAL)
            }
        };
        if kind == KEYVALUE {
            self.put_hot(p, p + 2, KEYVALUE);
            self.prev = P_TERM;
            return p + 2;
        }
        self.op_token(p, n, kind)
    }

    /// `-` at `p`: `->`, `--`, `-=`, a file test where a term starts, or minus.
    #[inline(always)]
    fn minus(&mut self, p: usize) -> usize {
        let b = self.b;
        match b.get(p + 1).copied().unwrap_or(b'\n') {
            b'>' => return self.arrow(p),
            b'-' => {
                self.put_hot(p, p + 2, ARITH);
                if self.prev != P_VALUE {
                    self.prev = P_TERM;
                }
                return p + 2;
            }
            b'=' => return self.op_token(p, 2, ASSIGN),
            c if self.term()
                && b"rwxoRWXOezsfdlpSbctugkTBAMC".contains(&c)
                && !b.get(p + 2).is_some_and(|&c| is_name_byte(c))
                && !fat_comma(b, p + 2) =>
            {
                return self.op_token(p, 2, FILETEST);
            }
            _ => {}
        }
        self.op_token(p, 1, ARITH)
    }

    /// `->` at `p`: a method's name, a subscript, a call, or a postfix dereference next.
    #[inline(always)]
    fn arrow(&mut self, p: usize) -> usize {
        let b = self.b;
        self.put_hot(p, p + 2, ARROW);
        self.prev = P_ARROW;
        let q = p + 2;
        match b.get(q).copied().unwrap_or(b'\n') {
            b'@' | b'%' | b'$' | b'&' | b'*' => {
                // `->@*`, `->$#*`, `->%*`, `->@[…]`, `->%{…}`, `->**`
                let mut e = q + 1;
                if b[q] == b'$' && b.get(e) == Some(&b'#') {
                    e += 1;
                }
                match b.get(e) {
                    Some(b'*') => {
                        self.put(q, e + 1, DEREF);
                        self.prev = P_VALUE;
                        e + 1
                    }
                    Some(b'[' | b'{') if b[q] != b'$' => {
                        self.put(q, e, DEREF);
                        self.prev = P_VALUE;
                        e
                    }
                    _ => q,
                }
            }
            _ => q,
        }
    }

    /// `.` at `p`: concatenation, a range, `...`, or a number's `.5` where a term starts.
    #[inline(always)]
    fn dot(&mut self, p: usize) -> usize {
        let b = self.b;
        match b.get(p + 1).copied().unwrap_or(b'\n') {
            b'0'..=b'9' if self.term() => self.number(p),
            b'.' => {
                let n = if b.get(p + 2) == Some(&b'.') { 3 } else { 2 };
                self.op_token(p, n, RANGE)
            }
            b'=' => self.op_token(p, 2, CONCAT),
            _ => self.op_token(p, 1, CONCAT),
        }
    }

    /// `/` at `p`: a regex where a term starts, else a divide, `//` defined-or, `/=`, `//=`.
    #[inline(always)]
    fn slash(&mut self, p: usize) -> usize {
        let b = self.b;
        let c1 = b.get(p + 1).copied().unwrap_or(b'\n');
        let regex = match self.prev {
            P_VALUE | P_ARROW | P_CONST => false,
            // after an unknown call, `/ ` and `/=` a divide (Perl's guess)
            P_FUNC => !matches!(c1, b' ' | b'\t' | b'=' | b'\n'),
            _ => true,
        };
        if regex {
            return self.slash_regex(p);
        }
        match c1 {
            b'/' if b.get(p + 2) == Some(&b'=') => self.op_token(p, 3, ASSIGN),
            b'/' => self.op_token(p, 2, LOGICAL),
            b'=' => self.op_token(p, 2, ASSIGN),
            _ => self.op_token(p, 1, ARITH),
        }
    }

    /// `<` at `p`: where a term starts, a here-document (`<<"EOT"`, `<<~EOT`) or a filehandle's
    /// read or a glob (`<FH>`, `<$fh>`, `<*.c>`); else a comparison or a shift.
    #[inline(always)]
    fn lt(&mut self, p: usize) -> usize {
        let b = self.b;
        let c1 = b.get(p + 1).copied().unwrap_or(b'\n');
        let r = if c1 == b'<'
            && (self.term() || self.prev == P_CONST || (self.prev == P_VALUE && print_handle(b, p)))
        {
            self.heredoc_open(p)
                .or_else(|| if self.term() { self.angle(p) } else { None })
        } else if self.term() {
            self.angle(p)
        } else {
            None
        };
        if let Some(e) = r {
            return e;
        }
        let at = |i: usize| b.get(p + i).copied().unwrap_or(b'\n');
        let (n, kind) = match c1 {
            b'<' if at(2) == b'=' => (3, ASSIGN),
            b'<' => (2, BITWISE),
            b'=' if at(2) == b'>' => (3, COMPARE),
            b'=' => (2, COMPARE),
            _ => (1, COMPARE),
        };
        self.op_token(p, n, kind)
    }

    /// `%` at `p`: a hash where a term starts, else modulo.
    #[inline(always)]
    fn percent(&mut self, p: usize) -> usize {
        if self.term() {
            return self.sigil(p);
        }
        let n = if self.b.get(p + 1) == Some(&b'=') {
            2
        } else {
            1
        };
        self.op_token(p, n, if n == 2 { ASSIGN } else { ARITH })
    }

    /// `&` at `p`: a sub's call where a term starts (`&foo`, `&$code`, `&{…}`), else `&&`,
    /// `&&=`, `&=`, `&.`, `&`.
    #[inline(always)]
    fn amp(&mut self, p: usize) -> usize {
        let b = self.b;
        let at = |i: usize| b.get(p + i).copied().unwrap_or(b'\n');
        if self.term() && at(1) != b'&' {
            return self.sigil(p);
        }
        let (n, kind) = match at(1) {
            b'&' if at(2) == b'=' => (3, ASSIGN),
            b'&' => (2, LOGICAL),
            b'=' => (2, ASSIGN),
            b'.' if at(2) == b'=' => (3, ASSIGN),
            b'.' => (2, BITWISE),
            _ => (1, BITWISE),
        };
        self.op_token(p, n, kind)
    }

    /// `*` at `p`: a glob where a term starts (`*STDOUT`, `*{…}`), else `**`, `**=`, `*=`, `*`.
    #[inline(always)]
    fn star(&mut self, p: usize) -> usize {
        let b = self.b;
        let at = |i: usize| b.get(p + i).copied().unwrap_or(b'\n');
        if self.term() && (is_name_start(at(1)) || matches!(at(1), b'{' | b'$' | b':')) {
            return self.sigil(p);
        }
        let (n, kind) = match at(1) {
            b'*' if at(2) == b'=' => (3, ASSIGN),
            b'*' => (2, ARITH),
            b'=' => (2, ASSIGN),
            _ => (1, ARITH),
        };
        self.op_token(p, n, kind)
    }

    /// `(` at `p`: a condition's, a signature's, a DBI call's arguments, or a group.
    #[inline(always)]
    fn lparen(&mut self, p: usize) -> usize {
        let f = if self.call != 0 {
            let f = self.call;
            self.call = 0;
            if f == F_SQL {
                self.sqls = text::S_SQL;
            }
            f
        } else {
            match self.prev {
                P_COND if !SOL => F_COND,
                _ if self.forvar => F_COND,
                P_SUB | P_SUBNAME if !SOL => F_SIG,
                _ => F_PAREN,
            }
        };
        self.forvar = false;
        self.push(f);
        if f == F_SIG {
            self.put_hot(p, p + 1, PARAMS_B);
            return self.signature(p + 1);
        }
        self.put_hot(p, p + 1, GROUP_B);
        self.prev = P_TERM;
        p + 1
    }

    /// `)` at `p`: a parenthesis's, with any bracket left open above it.
    #[inline(always)]
    fn rparen(&mut self, p: usize) -> usize {
        let f = self.close(is_paren);
        match f {
            F_COND => {
                self.put_hot(p, p + 1, GROUP_E);
                self.prev = P_BLOCK;
            }
            F_SIG => {
                self.put_hot(p, p + 1, PARAMS_E);
                self.prev = P_SUBNAME;
            }
            _ => {
                if f == F_SQL {
                    self.sqls = text::S_DONE;
                }
                self.put_hot(p, p + 1, GROUP_E);
                self.prev = P_VALUE;
            }
        }
        p + 1
    }

    /// `[` at `p`: a subscript after a value, else an anonymous array.
    #[inline(always)]
    fn lbracket(&mut self, p: usize) -> usize {
        if self.prev == P_VALUE || self.prev == P_ARROW {
            self.push(F_INDEX);
            self.put_hot(p, p + 1, ITEM_B);
        } else {
            self.push(F_BRACKET);
            self.put_hot(p, p + 1, BRACKETS_B);
        }
        self.prev = P_TERM;
        p + 1
    }

    /// `]` at `p`: a bracket's, with any parenthesis left open above it.
    #[inline(always)]
    fn rbracket(&mut self, p: usize) -> usize {
        let f = self.close(|f| matches!(f, F_BRACKET | F_INDEX));
        self.put_hot(p, p + 1, if f == F_INDEX { ITEM_E } else { BRACKETS_E });
        self.prev = P_VALUE;
        p + 1
    }

    /// `{` at `p`: what it opens by what came before: a subscript after a value, a block where a
    /// statement or a block is due, an anonymous hash where a term starts.
    #[inline(always)]
    fn lbrace(&mut self, p: usize) -> usize {
        let f = match self.prev {
            P_VALUE | P_ARROW | P_CONST => {
                // `$h{…}`, `->{…}`; `) {` a block (`if (…) {` past a lost condition)
                if p > 0 && matches!(self.b[p - 1], b' ' | b'\t') && self.prev != P_ARROW {
                    F_BLOCK
                } else {
                    return self.subscript(p);
                }
            }
            P_START | P_BLOCK | P_SUBNAME | P_PKG | P_MODULE => F_BLOCK,
            P_SUB | P_ANON => F_TBLOCK,
            P_LIST | P_FUNC => F_LBLOCK,
            _ => F_HASH,
        };
        self.forvar = false;
        self.push(f);
        self.put_hot(p, p + 1, BLOCK_B);
        self.prev = if f == F_HASH { P_TERM } else { P_START };
        p + 1
    }

    /// A subscript's `{` at `p`: `{word}` and `{-word}` a key, else code.
    #[inline(always)]
    fn subscript(&mut self, p: usize) -> usize {
        let b = self.b;
        if !SOL {
            let q = skip_blanks(b, p + 1);
            let w = if b.get(q) == Some(&b'-') { q + 1 } else { q };
            if b.get(w).is_some_and(|&c| is_name_start(c)) {
                let e = name_run(b, w + 1);
                let r = skip_blanks(b, e);
                if b.get(r) == Some(&b'}') {
                    self.put_hot(p, p + 1, ITEM_B);
                    if w > q {
                        self.put(q, w, ARITH);
                    }
                    self.put_hot(w, e, KEY);
                    self.put_hot(r, r + 1, ITEM_E);
                    self.prev = P_VALUE;
                    return r + 1;
                }
            }
        }
        self.push(F_SUBSCRIPT);
        self.put_hot(p, p + 1, ITEM_B);
        self.prev = P_TERM;
        p + 1
    }

    /// `}` at `p`: the innermost brace closed, what it left open dropped; a stray one nothing.
    #[inline(always)]
    fn rbrace(&mut self, p: usize) -> usize {
        let f = self.close(is_brace);
        let (kind, prev) = match f {
            F_SUBSCRIPT => (ITEM_E, P_VALUE),
            F_DEREF => (VEND, P_VALUE),
            F_HASH | F_TBLOCK => (BLOCK_E, P_VALUE),
            F_LBLOCK => (BLOCK_E, P_TERM),
            _ => (BLOCK_E, P_START),
        };
        self.put_hot(p, p + 1, kind);
        self.prev = prev;
        p + 1
    }

    /// A closer's frame closed: the top one where it `closes` it; one a level lost past the
    /// kept ones (0, with frames open) whatever it was; else the innermost it closes, what's open
    /// above it dropped; the frame closed, or 0 (and nothing closed).
    #[inline(always)]
    fn close(&mut self, closes: fn(u8) -> bool) -> u8 {
        let mut f = self.top();
        if !closes(f) {
            if f == 0 {
                if self.depth > 0 {
                    self.pop();
                }
                return 0;
            }
            f = self.unwind(closes);
        }
        if f != 0 {
            self.pop();
        }
        f
    }

    /// A closer with frames above its opener's: they're dropped, if one it closes is open
    /// below them; the frame it closes, or 0 (and nothing dropped).
    #[cold]
    fn unwind(&mut self, closes: fn(u8) -> bool) -> u8 {
        let levels = self.depth.min(LEVELS);
        let mut i = 0;
        while i < levels {
            let f = ((self.frames >> (FW * u32::from(i))) & FMASK) as u8;
            if closes(f) {
                for _ in 0..i {
                    self.pop();
                }
                return f;
            }
            i += 1;
        }
        0
    }

    /// A word at `p`: a keyword, a quote-like operator, a function, a key, a name.
    #[inline(always)]
    fn name(&mut self, p: usize) -> usize {
        let b = self.b;
        let e = if b[p] == b':' { p } else { name_run(b, p + 1) };
        if b.get(e) == Some(&b':') && b.get(e + 1) == Some(&b':') {
            if self.prev == P_PKG && !SOL && is_name_start(b[p]) {
                let r = self.module(p, if self.pp != 0 { NS_PP } else { NS_DECL });
                if r > p {
                    return r;
                }
            }
            return self.qualified(p, e);
        }
        if self.prev >= P_ARROW {
            if self.prev == P_ARROW {
                return self.method(p, e);
            }
            if self.prev == P_PKG && !SOL {
                // `package # comment` ⏎ `Foo::Bar;`
                let r = self.module(p, if self.pp != 0 { NS_PP } else { NS_DECL });
                if r > p {
                    return r;
                }
            }
        }
        let w = WORDS.value(&b[p..e], &INFO);
        if w & (W_ACTION | W_LEAF) != 0 {
            if fat_comma(b, e) {
                self.put_hot(p, e, KEY);
                self.prev = P_VALUE;
                return e;
            }
            if SOL {
                let leaf = leaf_of(w);
                if leaf != 0 {
                    self.put_hot(p, e, leaf);
                }
                self.prev = if w & W_VALUE != 0 { P_VALUE } else { P_TERM };
                return e;
            }
            return self.word(p, e, w);
        }
        self.bareword(p, e)
    }

    /// A word `b[p..e]` with an entry in [`WORDS`] (`w` its info).
    #[inline(never)]
    fn word(&mut self, p: usize, e: usize, w: u16) -> usize {
        let b = self.b;
        let leaf = leaf_of(w);
        let action = w & W_ACTION;
        match action {
            W_FN | W_KW => {
                self.put_hot(p, e, leaf);
                self.prev = if w & W_VALUE != 0 { P_VALUE } else { P_TERM };
                e
            }
            W_MY => {
                self.put_hot(p, e, leaf);
                if self.prev != P_COND {
                    self.prev = P_TERM;
                }
                e
            }
            W_COND | W_FOR => {
                self.put_hot(p, e, leaf);
                self.prev = P_COND;
                self.forvar = action == W_FOR;
                e
            }
            W_BLOCK => {
                self.put_hot(p, e, leaf);
                self.prev = P_BLOCK;
                e
            }
            W_WORDOP => {
                self.put_hot(p, e, leaf);
                self.prev = P_TERM;
                e
            }
            W_X => {
                // repetition after a value: `x`, `x=`, `x3`
                if matches!(self.prev, P_VALUE | P_CONST) {
                    let n = if b.get(e) == Some(&b'=') { e + 1 } else { e };
                    self.put_hot(p, n, ARITH);
                    self.prev = P_TERM;
                    return n;
                }
                self.bareword_out(p, e)
            }
            W_QUOTE => self.quote_op(p, e),
            W_SUB => {
                self.put_hot(p, e, leaf);
                self.prev = P_SUB;
                self.sub_name(e)
            }
            W_PKG => {
                self.put_hot(p, e, leaf);
                self.prev = P_PKG;
                self.module(e, NS_DECL)
            }
            W_USE | W_REQUIRE => {
                if self.pp == 0 && !SOL {
                    self.pp = self.depth.saturating_add(1);
                    self.pp_at = p;
                }
                self.put_hot(p, e, leaf);
                self.prev = P_PKG;
                self.module(e, NS_PP)
            }
            W_LABEL => {
                self.put_hot(p, e, leaf);
                self.prev = P_LABEL;
                let q = skip_blanks(b, e);
                if b.get(q).is_some_and(|&c| is_name_start(c)) {
                    let r = name_run(b, q + 1);
                    if WORDS.value(&b[q..r], &INFO) & W_ACTION == 0 && b.get(r) != Some(&b':') {
                        self.put(q, r, LABEL_USE);
                        self.prev = P_VALUE;
                        return r;
                    }
                }
                e
            }
            W_DO => {
                self.put_hot(p, e, leaf);
                self.prev = P_ANON;
                e
            }
            W_LIST => {
                self.put_hot(p, e, leaf);
                self.prev = P_LIST;
                e
            }
            W_FORMAT => self.format_head(p, e),
            W_END => {
                self.put(p, e, leaf);
                self.mode = M_DATA;
                self.queue = Queue::new();
                self.frames = 0;
                self.depth = 0;
                self.pp = 0;
                b.len()
            }
            W_FH => {
                self.put_hot(p, e, leaf);
                self.prev = P_TERM;
                e
            }
            W_PHASE => {
                // `BEGIN {`: a phase's block
                if next_is(b, e, b'{') {
                    self.put(p, e, PREPROC);
                    self.prev = P_BLOCK;
                    return e;
                }
                self.bareword_out(p, e)
            }
            W_CALLBACK => self.bareword_out(p, e),
            _ => {
                // a constant (`__FILE__`), a word with no action
                if leaf != 0 {
                    self.put_hot(p, e, leaf);
                    self.prev = P_VALUE;
                    return e;
                }
                self.bareword_out(p, e)
            }
        }
    }

    /// [`Scan::bareword`] out of line, for the rare callers (a word with an entry that isn't one
    /// where it stands, a quote-like word without a delimiter, `format` not a format's head): one
    /// copy of it beside the code loop's.
    #[inline(never)]
    fn bareword_out(&mut self, p: usize, e: usize) -> usize {
        self.bareword(p, e)
    }

    /// A bareword `b[p..e]` no table names: a label at a statement's start, a call before `(`
    /// or a term, a class before `->`, a constant in capitals, a key before `=>`.
    #[inline(always)]
    fn bareword(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let q = skip_blanks(b, e);
        let c = b.get(q).copied().unwrap_or(b'\n');
        let c1 = b.get(q + 1).copied().unwrap_or(b'\n');
        match c {
            b'=' if c1 == b'>' => {
                self.put_hot(p, e, KEY);
                self.prev = P_VALUE;
                return e;
            }
            b'(' => {
                self.put_hot(p, e, FUNC);
                self.prev = P_FUNC;
                return e;
            }
            b'-' if c1 == b'>' => {
                self.put_hot(p, e, NS);
                self.prev = P_VALUE;
                return e;
            }
            b':' if !SOL && q == e && c1 != b':' && self.prev == P_START => {
                // `LINE:`
                self.put(p, e, LABEL);
                self.put(e, e + 1, LABEL_SEP);
                return e + 1;
            }
            _ => {}
        }
        if all_caps(b, p, e) {
            self.put_hot(p, e, CONSTANT);
            self.prev = P_CONST;
            return e;
        }
        // a call: bat's, a bareword before a term (a string, a variable, a word or a number,
        // `/`, `<<`), `;`, a comment or the line's end; and Perl's before `\`, `!`, a block or an
        // array (`explain \@x`, `builder {`, `ArrayRef[Int]`)
        let call = match c {
            b'\n'
            | b';'
            | b'#'
            | b'\''
            | b'"'
            | b'`'
            | b'$'
            | b'@'
            | b'%'
            | b'/'
            | b'\\'
            | b'{'
            | b'['
            | b'0'..=b'9' => true,
            b'<' => c1 == b'<',
            b'!' => !matches!(c1, b'=' | b'~'),
            _ if is_name_start(c) && q > e => {
                let r = name_run(b, q + 1);
                WORDS.value(&b[q..r], &INFO) & W_ACTION != W_WORDOP && !(r == q + 1 && b[q] == b'x')
            }
            _ => false,
        };
        if call {
            self.put_hot(p, e, FUNC);
            self.prev = P_FUNC;
            return e;
        }
        // bat's fallback: a bareword not called, a constant (`use constant`'s, a sub's name)
        self.put_hot(p, e, CONSTANT);
        self.prev = P_CONST;
        e
    }

    /// A name after `->` at `p` to `e`: a method's (a DBI one's `(` its SQL's arguments).
    #[inline(always)]
    fn method(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        self.put_hot(p, e, MEMBER);
        self.prev = P_VALUE;
        if !SOL && b.get(e) == Some(&b'(') && WORDS.value(&b[p..e], &INFO) & W_SQLM != 0 {
            self.call = F_SQL;
        }
        e
    }

    /// A qualified name from `p`, its first word ending at `e` (`Foo::Bar->new`,
    /// `Foo::bar()`, `CORE::say`, `::foo`): its packages' names, then its last word: a class
    /// before `->` or `::`, else a function (a built-in's `support`).
    #[inline(never)]
    fn qualified(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let end = qualified_end(b, e);
        // the last word's start: past the last `::`
        let mut last = end;
        while last > p && is_name_byte(b[last - 1]) {
            last -= 1;
        }
        let trailing = last == end;
        if fat_comma(b, end) {
            self.put_hot(p, end, KEY);
            self.prev = P_VALUE;
            return end;
        }
        let class = trailing || {
            let q = skip_blanks(b, end);
            b.get(q) == Some(&b'-') && b.get(q + 1) == Some(&b'>')
        };
        let mut q = p;
        while q < last {
            if b[q] == b':' {
                self.put_hot(q, q + 2, DCOLON);
                q += 2;
            } else {
                let w = name_run(b, q + 1);
                self.put_hot(q, w, NS);
                q = w;
            }
        }
        if !trailing {
            let kind = if class {
                NS
            } else if self.prev == P_ARROW {
                MEMBER
            } else {
                match WORDS.value(&b[last..end], &INFO) & W_ACTION {
                    W_FN | W_LIST | W_DO => SUPPORT,
                    _ => FUNC,
                }
            };
            self.put_hot(last, end, kind);
        }
        self.prev = if class { P_VALUE } else { P_FUNC };
        end
    }

    /// A module's name after `package`, `use`, `no` or `require` from `p`, its words written as
    /// `kind` (`Foo::Bar`), and a version after it (`package Foo 1.02;`); or a version alone
    /// (`use v5.36;`, `use 5.010_001;`); or nothing (`require $file`).
    #[inline(never)]
    fn module(&mut self, p: usize, kind: u16) -> usize {
        let b = self.b;
        let q = skip_blanks(b, p);
        let Some(&c) = b.get(q) else {
            return p;
        };
        if is_version(b, q) {
            return self.version(q);
        }
        if c == b'#' {
            // `package # comment` ⏎ `Foo;`: the name on a later line
            return p;
        }
        if !is_name_start(c) || fat_comma(b, name_run(b, q + 1)) {
            // no module's name (`require $file`, `use X => 1`): a term
            self.prev = P_TERM;
            return p;
        }
        let end = qualified_end(b, name_run(b, q + 1));
        let mut r = q;
        while r < end {
            if b[r] == b':' {
                self.put_hot(r, r + 2, DCOLON);
                r += 2;
            } else {
                let w = name_run(b, r + 1);
                self.put_hot(r, w, kind);
                r = w;
            }
        }
        self.prev = P_MODULE;
        let v = skip_blanks(b, end);
        if is_version(b, v) {
            return self.version(v);
        }
        end
    }

    /// A version at `p` (`v5.36.0`, `1.02`, `5.010_001`): `v`, digits, `.`s.
    #[inline(never)]
    fn version(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut q = p;
        if b[q] == b'v' {
            self.put(q, q + 1, VERSION_V);
            q += 1;
        }
        loop {
            let mut e = q;
            while e < n && (b[e].is_ascii_digit() || b[e] == b'_') {
                e += 1;
            }
            self.put(q, e, VERSION);
            if e + 1 < n && b[e] == b'.' && b[e + 1].is_ascii_digit() {
                self.put(e, e + 1, VERSION_DOT);
                q = e + 1;
            } else {
                self.prev = P_VALUE;
                return e;
            }
        }
    }
}

/// Whether a version starts at `p`: a digit, or `v` and a digit.
#[inline(always)]
fn is_version(b: &[u8], p: usize) -> bool {
    match b.get(p) {
        Some(c) if c.is_ascii_digit() => true,
        Some(b'v') => b.get(p + 1).is_some_and(u8::is_ascii_digit),
        _ => false,
    }
}

/// Whether the first byte past `e` that isn't a blank is `c`.
#[inline(always)]
fn next_is(b: &[u8], e: usize, c: u8) -> bool {
    b.get(skip_blanks(b, e)) == Some(&c)
}
