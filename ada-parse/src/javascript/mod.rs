//! JavaScript by a scanner of ada's own (X-058's redesign, 2026-11-08: "architecting for
//! performance"), Babel's grammar the reference for the colors, not the design. One loop runs
//! every context as a frame of a small stack that means something to the colors: statement code,
//! an object literal (its key place), parentheses, brackets, a template's substitution, a
//! function's parameters, a class's head and body, a ternary's `?`, and the text frames (a string
//! continued by `\`, a template's text, a block comment, JSX's tags and children, a regex in a
//! `RegExp(…)` string).
//!
//! - **The previous significant token in a byte** ([`Scan::prev`]) tells a `/` a regex's or
//!   division, a `<` JSX's or an operator, a `{` an object's or a block's, a name a member, a key
//!   or a constructor: no look behind the place.
//! - **No lookahead past the blanks after a token.** A function's head is read by state
//!   (`function`, then `*`, its name and `(`); what Babel decides by a balanced lookahead is
//!   decided at the token that decides it. A `(` that may open an arrow's parameters keeps a
//!   mark of the writing ([`Group`]); a `=>` after its `)` takes the writing back and reads the
//!   group again as parameters, so only arrows pay. A `name =` (or a key's `name:`) keeps a mark
//!   ([`Assign`]); a value that turns out a function names it.
//! - **A name is read once** and looked up once ([`WORDS`]), its kind from its word, the frame,
//!   the previous token and the byte after it.
//! - **Flow's types are frames too** ([`Scan::type_turn`]): an annotation after a parameter's,
//!   a field's or a return type's `:`, `type X = …`, `import type`, a function's `<T>`; a type is
//!   left at the first byte no type takes at its level.
//!
//! A regex literal's body is [`crate::regex`]'s ECMAScript dialect, and so is a `RegExp(…)`
//! string's; a kind is a leaf, bat's scopes for a token ([`scopes`]), of the meta scopes only
//! those a theme can see (a string's, a regex's, a comment's). The stack between lines is the
//! frames open above the top level, up to 28 (the bytes of [`State`]'s levels), the previous
//! token beside them.

mod text;

use crate::emit::{Mark, Push, Rules, Run, Runs, Sink, Spans};
use crate::prim::{WORD, char_len, is_word, stops};
use crate::regex;
use crate::words::{Words, text_len};
use crate::{Out, State};

macro_rules! leaves {
    ($($name:ident = $scopes:literal,)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[repr(u16)]
        enum Leaf { $($name,)* }
        $(#[allow(dead_code)] const $name: u16 = Leaf::$name as u16;)*
        /// Each kind's scopes under `source.js`, outermost first.
        static LEAVES: &[&str] = &[$($scopes,)*];
    };
}

leaves! {
    PLAIN = "",
    DOC_TEXT = "comment.block.documentation.js",
    DOC_MARK = "comment.block.documentation.js punctuation.definition.comment.js",
    BLOCK_TEXT = "comment.block.js",
    BLOCK_BEGIN = "comment.block.js punctuation.definition.comment.begin.js",
    BLOCK_END = "comment.block.js punctuation.definition.comment.end.js",
    HTML_COMMENT = "comment.block.html.js punctuation.definition.comment.js",
    LINE = "comment.line.double-slash.js",
    LINE_MARK = "comment.line.double-slash.js punctuation.definition.comment.js",
    SHEBANG = "comment.line.shebang.js",
    SHEBANG_MARK = "comment.line.shebang.js punctuation.definition.comment.js",
    STR = "string.quoted.js",
    STR_BEGIN = "string.quoted.js punctuation.definition.string.begin.js",
    STR_END = "string.quoted.js punctuation.definition.string.end.js",
    STR_ESC = "string.quoted.js constant.character.escape.js",
    STR_ESC_NL = "string.quoted.js constant.character.escape.newline.js",
    KEY_FN = "string.quoted.js entity.name.function.js",
    KEY_NAME_FN = "string.unquoted.js entity.name.function.js",
    TPL_BEGIN = "string.interpolated.js keyword.other.template.begin.js",
    TPL_END = "string.interpolated.js keyword.other.template.end.js",
    TPL_TEXT = "string.interpolated.js",
    ESC = "constant.character.escape.js",
    ESC_NL = "constant.character.escape.newline.js",
    SUBST_BEGIN = "keyword.other.substitution.begin.js",
    SUBST_END = "keyword.other.substitution.end.js",
    RE = "string.regexp.js",
    RE_BEGIN = "string.regexp.js punctuation.definition.string.begin.js",
    RE_END = "string.regexp.js punctuation.definition.string.end.js",
    RE_FLAGS = "string.regexp.js keyword.other.js",
    OBJ_KEY = "constant.other.object.key.js",
    OBJ_KEY_LABEL = "constant.other.object.key.js string.unquoted.label.js",
    OBJ_KEY_SEP = "constant.other.object.key.js punctuation.separator.key-value.js",
    KEY_SEP = "punctuation.separator.key-value.js",
    ENTITY_FN = "entity.name.function.js",
    ASSIGN = "keyword.operator.assignment.js",
    STORAGE = "storage.type.js",
    STORAGE_FN = "storage.type.function.js",
    ASTERISK = "keyword.generator.asterisk.js",
    ENTITY_CLASS = "entity.name.class.js",
    ACCESSOR = "keyword.operator.accessor.js",
    PROTOTYPE = "variable.language.prototype.js",
    ARROW_KW = "storage.type.function.arrow.js",
    PARAM = "variable.parameter.function.js",
    LOOP = "keyword.control.loop.js",
    SWITCH_KW = "keyword.control.switch.js",
    FLOW_KW = "keyword.control.flow.js",
    COND = "keyword.control.conditional.js",
    TRY = "keyword.control.trycatch.js",
    RESERVED = "keyword.other.reserved.js",
    KW_OTHER = "keyword.other.js",
    MODULE = "keyword.operator.module.js",
    CLASS_KW = "storage.type.class.js",
    INTERFACE_KW = "storage.type.interface.flowtype.js",
    EXTENDS = "storage.type.extends.js",
    DECLARE = "support.type.declare.flowtype.js",
    TYPE_KW = "support.type.type.flowtype.js",
    ENTITY_TYPE = "entity.name.type.js",
    INVALID_HEX = "invalid.illegal.numeric.hex.js",
    INVALID_OCT = "invalid.illegal.numeric.octal.js",
    NUMBER = "constant.numeric.js",
    INFINITY = "constant.language.infinity.js",
    TRUE = "constant.language.boolean.true.js",
    FALSE = "constant.language.boolean.false.js",
    NULL = "constant.language.null.js",
    UNDEFINED = "constant.language.undefined.js",
    NAN = "constant.language.nan.js",
    ARGUMENTS = "variable.language.arguments.js",
    SUPER = "variable.language.super.js",
    THIS = "variable.language.this.js",
    SELF = "variable.language.self.js",
    PROTO = "variable.language.proto.js",
    CONSTRUCTOR = "variable.language.constructor.js",
    NEW = "keyword.operator.new.js",
    CTOR_NAME = "variable.function.constructor.js",
    CLASS_VAR = "variable.other.class.js",
    FUNCTION = "variable.function.js",
    CALL_PARENS = "meta.group.braces.round.function.arguments.js",
    ROUND_BEGIN = "meta.brace.round.begin.js",
    ROUND_END = "meta.brace.round.end.js",
    SQUARE_BEGIN = "meta.brace.square.begin.js",
    SQUARE_END = "meta.brace.square.end.js",
    CURLY_BEGIN = "meta.brace.curly.begin.js",
    CURLY_END = "meta.brace.curly.end.js",
    OP = "keyword.operator.js",
    LOGICAL = "keyword.operator.logical.js",
    AUGMENTED = "keyword.operator.assignment.augmented.js",
    BITWISE = "keyword.operator.bitwise.js",
    RELATIONAL = "keyword.operator.relational.js",
    COMPARISON = "keyword.operator.comparison.js",
    ARITHMETIC = "keyword.operator.arithmetic.js",
    TERNARY = "keyword.operator.ternary.js",
    SPREAD = "keyword.operator.spread.js",
    CONSTANT = "variable.other.constant.js",
    STATIC_PROP = "variable.other.property.static.js",
    OBJECT = "variable.other.object.js",
    PROPERTY = "variable.other.property.js",
    READWRITE = "variable.other.readwrite.js",
    TERMINATOR = "punctuation.terminator.statement.js",
    COMMA = "meta.delimiter.comma.js",
    DECORATOR = "tag.decorator.js",
    DEC_MARK = "tag.decorator.js punctuation.definition.tag.js",
    DEC_NAME = "tag.decorator.js entity.name.tag.js",
    BUILTIN_CLASS = "support.class.builtin.js",
    ERROR_CLASS = "support.class.error.js",
    NODE_CLASS = "support.class.node.js",
    BUILTIN_FN = "support.function.builtin.js",
    NODE_FN = "support.function.node.js",
    DOM = "support.type.object.dom.js",
    NODE_OBJ = "support.type.object.node.js",
    CONSOLE = "support.type.object.console.js",
    CONSOLE_FN = "support.function.console.js",
    PROCESS = "support.type.object.process.js",
    PROCESS_FN = "support.function.process.js",
    MODULE_OBJ = "support.type.object.module.js",
    CONFLICT_INTO = "invalid.illegal.conflict-marker.merge-into.js",
    CONFLICT_SEP = "invalid.illegal.conflict-marker.separator.js",
    CONFLICT_OTHER = "invalid.illegal.conflict-marker.other-commit.js",
    CONFLICT_TEXT = "invalid.illegal.string.js",
    MODIFIER = "storage.modifier.js",
    ACCESSOR_KW = "storage.type.accessor.js",
    ENTITY_ACCESSOR = "entity.name.accessor.js",
    FLOW_OP = "keyword.operator.flowtype.js",
    PARAMS_BEGIN = "punctuation.definition.parameters.begin.js",
    PARAMS_END = "punctuation.definition.parameters.end.js",
    PARAM_SEP = "punctuation.separator.parameter.function.js",
    FLOW_OPTIONAL = "keyword.operator.flowtype.optional.js",
    FLOW_ANNOTATION = "keyword.operator.flowtype.annotation.js",
    FLOW_OTHER = "keyword.operator.flowtype.other.js",
    FLOW_BEGIN = "punctuation.section.flowtype.begin.js",
    FLOW_END = "punctuation.section.flowtype.end.js",
    FLOW_PRIMITIVE = "constant.other.primitve.flowtype.js",
    FLOW_VAR = "variable.other.flowtype.js",
    TAG_BEGIN = "meta.tag.jsx punctuation.definition.tag.begin.jsx",
    TAG_NAME = "meta.tag.jsx entity.name.tag.jsx",
    TAG_END = "meta.tag.jsx punctuation.definition.tag.end.jsx",
    TAG_INCOMPLETE = "invalid.illegal.tag.incomplete.jsx",
    ATTR = "entity.other.attribute-name.jsx",
    ATTR_EQ = "keyword.operator.assignment.jsx",
    JSTR = "string.quoted.jsx",
    JSTR_BEGIN = "string.quoted.jsx punctuation.definition.string.begin.jsx",
    JSTR_END = "string.quoted.jsx punctuation.definition.string.end.jsx",
    JSTR_ENTITY = "string.quoted.jsx constant.character.entity.jsx",
    JSTR_BAD = "string.quoted.jsx invalid.illegal.bad-ampersand.jsx",
    ENTITY = "constant.character.entity.jsx",
    BAD_AMP = "invalid.illegal.bad-ampersand.jsx",
    EMBED_BEGIN = "punctuation.section.embedded.begin.jsx",
    EMBED_END = "punctuation.section.embedded.end.jsx",
    TAGGED = "variable.function.tagged-template.js",
    TPL_MARK_BEGIN = "punctuation.definition.string.template.begin.js",
    TPL_MARK_END = "punctuation.definition.string.template.end.js",
    PUNCT_ACCESSOR = "punctuation.accessor.js",
    STYLED_FN = "meta.function-call.with-arguments.js variable.function.js",
    GROUP_BEGIN = "punctuation.definition.group.begin.js",
    GROUP_END = "punctuation.definition.group.end.js",
}

/// The kinds of a regex's tokens: the regex's, from here, [`regex::KINDS`] for each host: a
/// literal, a quoted string, a template.
const RE_KINDS: u16 = 256;
const _: () = assert!(LEAVES.len() <= RE_KINDS as usize);

/// The scopes under `source.js` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    if kind >= RE_KINDS {
        let n = regex::KINDS as u16;
        let host = match (kind - RE_KINDS) / n {
            0 => "string.regexp.js",
            1 => "string.quoted.js",
            2 => "string.interpolated.js",
            _ => return Vec::new(),
        };
        let mut out = vec![host];
        out.extend(regex::scopes((kind - RE_KINDS) % n));
        return out;
    }
    LEAVES.get(usize::from(kind)).map_or_else(Vec::new, |l| {
        l.split(' ').filter(|s| !s.is_empty()).collect()
    })
}

/// How many frames a line can hold; one pushed past it drops the bottom one.
const MAX: usize = 128;

/// Whether `s` is at `i`.
fn has(b: &[u8], i: usize, s: &[u8]) -> bool {
    crate::has_at(b, i, s)
}

const CONSOLE_FNS: &[&[u8]] = &[
    b"assert",
    b"count",
    b"dir",
    b"error",
    b"group",
    b"groupCollapsed",
    b"groupEnd",
    b"info",
    b"log",
    b"profile",
    b"profileEnd",
    b"table",
    b"time",
    b"timeEnd",
    b"trace",
    b"warn",
];
const PROCESS_PROPS: &[&[u8]] = &[
    b"arch",
    b"argv",
    b"config",
    b"env",
    b"execArgv",
    b"execPath",
    b"exitCode",
    b"mainModule",
    b"pid",
    b"platform",
    b"stderr",
    b"stdin",
    b"stdout",
    b"title",
    b"version",
    b"versions",
];
const PROCESS_FNS: &[&[u8]] = &[
    b"abort",
    b"chdir",
    b"cwd",
    b"exit",
    b"getgid",
    b"getgroups",
    b"getuid",
    b"hrtime",
    b"initgroups",
    b"kill",
    b"memoryUsage",
    b"nextTick",
    b"setgid",
    b"setgroups",
    b"setuid",
    b"umask",
    b"uptime",
];
const MODULE_PROPS: &[&[u8]] = &[
    b"children",
    b"exports",
    b"filename",
    b"id",
    b"loaded",
    b"parent",
];

/// A string escape at `p`, a `\`:
/// `\\([1-7][0-7]{0,2}|[0-7]{2,3}|[bfnrtv0'"\\]|x\h{2}|u\{\h+\}|u\h{4})`: where it ends.
fn escape(b: &[u8], p: usize) -> Option<usize> {
    let oct = |i: usize| b.get(i).is_some_and(|c| matches!(c, b'0'..=b'7'));
    let hex = |i: usize| b.get(i).is_some_and(u8::is_ascii_hexdigit);
    let c = *b.get(p + 1)?;
    let q = p + 2;
    match c {
        b'1'..=b'7' => {
            let mut e = q;
            while e < q + 2 && oct(e) {
                e += 1;
            }
            Some(e)
        }
        b'0' if oct(q) => Some(if oct(q + 1) { q + 2 } else { q + 1 }),
        b'b' | b'f' | b'n' | b'r' | b't' | b'v' | b'0' | b'\'' | b'"' | b'\\' => Some(q),
        b'x' if hex(q) && hex(q + 1) => Some(q + 2),
        b'u' if b.get(q) == Some(&b'{') && hex(q + 1) => {
            let mut e = q + 1;
            while hex(e) {
                e += 1;
            }
            (b.get(e) == Some(&b'}')).then_some(e + 1)
        }
        b'u' if (q..q + 4).all(hex) => Some(q + 4),
        _ => None,
    }
}

/// Where a regex in a `"…"` or a `'…'` yields to its string (its quote, an escape), and where a
/// regex literal's body can end.
static DQ_STOPS: [bool; 256] = stops(b"\"\\");
static SQ_STOPS: [bool; 256] = stops(b"'\\");
static REGEX_STOPS: [bool; 256] = stops(b"/");
/// A template's, with its escapes: where its patterns start in a regex template.
static TEMPLATE_RE_STOPS: [bool; 256] = stops(b"`$\\");

/// A regex literal's body: its `/` ends it where no group or class is open.
static RE_EMBED: regex::Embed<'static> = regex::Embed {
    dialect: regex::Dialect::Ecma,
    cooked: false,
    yields: &REGEX_STOPS,
    stop: |_, _, _| false,
    arg: 0,
    escape: regex::java_escape,
    top_only: true,
    continues: false,
    base: RE_KINDS,
    text: RE,
};

/// A regex in a `"…"`, a `'…'` and a template ([`RE_STR`]): the host's end, its escapes and a
/// template's `${` come first.
static RE_STR_EMBED: [regex::Embed<'static>; 3] = [
    re_str_embed(&DQ_STOPS, 1, STR),
    re_str_embed(&SQ_STOPS, 1, STR),
    regex::Embed {
        stop: |b, p, _| b[p] == b'`' || b.get(p + 1) == Some(&b'{'),
        // a `\` that isn't one of Babel's escapes is the `\` alone (`\${` is a substitution)
        escape: |b, p| {
            escape(b, p).unwrap_or(if b.get(p + 1) == Some(&b'`') {
                p + 2
            } else {
                p + 1
            })
        },
        ..re_str_embed(&TEMPLATE_RE_STOPS, 2, TPL_TEXT)
    },
];

const fn re_str_embed(yields: &'static [bool; 256], host: u16, text: u16) -> regex::Embed<'static> {
    regex::Embed {
        dialect: regex::Dialect::Ecma,
        cooked: true,
        yields,
        stop: |_, _, _| true,
        arg: 0,
        escape: |b, p| {
            escape(b, p).unwrap_or_else(|| b.get(p + 1).map_or(p + 1, |&d| p + 1 + char_len(d)))
        },
        top_only: false,
        continues: true,
        base: RE_KINDS + host * regex::KINDS as u16,
        text,
    }
}

// The frames. Code frames first; [`TEXT`] and up are a text frame's, each with a turn of its
// own ([`Scan::text_turn`]).
/// The top level: statements.
const F_TOP: u8 = 0;
/// A block's `{`: statements.
const F_BLOCK: u8 = 1;
/// An object literal's `{`: a key's place after it and after each `,`.
const F_OBJECT: u8 = 2;
/// A `(`: a group or a call's arguments.
const F_PAREN: u8 = 3;
/// A `[`.
const F_BRACKET: u8 = 4;
/// A template's `${`, back to its text at its `}`.
const F_SUBST: u8 = 5;
/// A JSX expression's `{`, back to the element at its `}`.
const F_J_EXPR: u8 = 6;
/// A ternary's `?`, popped at its `:`.
const F_TERNARY: u8 = 7;
/// A `case`'s (or `default`'s) expression, popped at its `:`.
const F_CASE: u8 = 8;
/// A function's parameters: a name is a parameter.
const F_PARAMS: u8 = 9;
/// A destructuring pattern's `{` or `[` among the parameters.
const F_PATTERN: u8 = 10;
/// A parameter's default value: code, popped at a `,` or a closer of the frame below.
const F_DEFAULT: u8 = 11;
/// A class's body: its members.
const F_CLASS_BODY: u8 = 12;
/// After `function`: its `*`, its name, its `(`.
const F_FUNC_HEAD: u8 = 13;
/// After `class`: its name, `extends` and what it extends, its `{`.
const F_CLASS_HEAD: u8 = 14;
/// A class field's value: code, popped at its `;`.
const F_FIELD: u8 = 15;
/// A styled-components template's `${`, back to its plain text at its `}`.
const F_SUBST_PLAIN: u8 = 16;
/// A computed key's `[` at a key's or a member's place: its `]` leaves [`P_KEYED`].
const F_KEY_BRACKET: u8 = 17;
/// The first of Flow's type frames ([`Scan::type_turn`]): a type annotation after its `:`, left
/// at a byte no type takes at its level.
const F_TYPE: u8 = 18;
/// A type's `(`, `[`, `{` and `<`, each left at its closer.
const F_TYPE_ROUND: u8 = 19;
const F_TYPE_SQUARE: u8 = 20;
const F_TYPE_CURLY: u8 = 21;
const F_TYPE_ANGLE: u8 = 22;
/// `type X = …` (and `import type`, `export type`): to its `;` or `from`.
const F_TYPE_DECL: u8 = 23;
const _: () = assert!(F_TYPE_DECL < TEXT);
/// The first text frame.
const TEXT: u8 = 32;
/// A `"…"` continued by a `\` at a line's end.
const T_STR_D: u8 = 32;
/// A `'…'`, likewise.
const T_STR_S: u8 = 33;
/// A template's text.
const T_TPL: u8 = 34;
/// A styled-components (or `gql`) template's text: plain.
const T_TPL_PLAIN: u8 = 35;
/// A block comment's text.
const T_BLOCK: u8 = 36;
/// A doc comment's (`/**`) text.
const T_DOC: u8 = 37;
/// A JSX element's opening tag: its attributes, until `>` or `/>`.
const T_JSX_TAG: u8 = 38;
/// A JSX element's children, until a closing tag.
const T_JSX_CHILD: u8 = 39;
/// A JSX attribute's `"…"`.
const T_JSX_STR_D: u8 = 40;
/// A JSX attribute's `'…'`.
const T_JSX_STR_S: u8 = 41;
/// A regex in a `RegExp(…)` string or template, its state in the low bits, above the string's
/// frame.
const T_RE_STR: u8 = 0x40;
const T_RE_STR_LAST: u8 = T_RE_STR | regex::ECMA_STATE;
const _: () = assert!(T_RE_STR & regex::ECMA_STATE == 0 && T_RE_STR_LAST < 0x80);

// The previous significant token, as the next one reads it.
/// A statement's start: a `{` a block's, a `/` a regex's, a name before `:` a label.
const P_START: u8 = 0;
/// An expression is expected: a `{` an object's, a `/` a regex's, a `<` JSX's.
const P_OP: u8 = 1;
/// After a value: a `/` division, a `<` an operator, a `{` a block's.
const P_VALUE: u8 = 2;
/// After a `.`: a name is a member.
const P_DOT: u8 = 3;
/// An object's key place, after its `{` or a `,`.
const P_KEY: u8 = 4;
/// After `=>`: a `{` a block's, else an expression.
const P_ARROW: u8 = 5;
/// After `new`: a name is a constructor's.
const P_NEW: u8 = 6;
/// After `extends`: a name is the class extended.
const P_EXTENDS: u8 = 7;
/// After `console.`, `process.`, `module.` or `exports.`: a member of its list is a support
/// word's.
const P_DOT_CONSOLE: u8 = 8;
const P_DOT_PROCESS: u8 = 9;
const P_DOT_MODULE: u8 = 10;
/// A class body's member place: after its `{`, a `;`, a method's `}`.
const P_MEMBER: u8 = 11;
/// After `get` or `set` at a key's or a member's place: the name is an accessor's.
const P_ACCESSOR: u8 = 12;
/// After a key that isn't a name (a string, a number, a computed `[…]`) at a key's or a
/// member's place: a `(` opens a method's parameters.
const P_KEYED: u8 = 13;
/// After a function's parameters' `)`: a value's, and a `:` starts the return type.
const P_PARAMS_END: u8 = 14;

/// [`State`]'s `arg`: the previous token, and whether a line came before.
const ARG_PREV: u16 = 0xff;
const ARG_LATER: u16 = 0x100;

/// How many groups a line keeps a mark for at once ([`Group`]).
const GROUPS: usize = 8;

/// A `(` that may open an arrow's parameters: where the writing stood before it, and its depth.
#[derive(Clone, Copy)]
struct Group {
    mark: Mark,
    open: u32,
    depth: u16,
    /// The assignment whose value the group starts (`name = (…) =>`), else one whose
    /// value is `NO_VALUE`.
    assign: Assign,
    /// Where an `async` before the group starts, else `NO_VALUE`: a keyword if the group is an
    /// arrow's parameters, else a variable as written.
    asy: u32,
}

/// A `name =`, or a key's `name:`, whose value starts at `value`: if that value is a function,
/// the name is the function's.
#[derive(Clone, Copy)]
struct Assign {
    mark: Mark,
    name: (u32, u32),
    /// The `=` or `:`; for [`A_CLASS_MEMBER`] the capitalized name's start.
    sep: (u32, u32),
    value: u32,
    /// [`A_NAMED`], [`A_KEY`], [`A_QUOTED`] or [`A_CLASS_MEMBER`]: what the name is, and how a
    /// function names it.
    kind: u8,
}

/// An assigned name's, a key's, a quoted key's, a capitalized name's member's (`Foo.bar =`, the
/// mark from before `Foo`) [`Assign::kind`].
const A_NAMED: u8 = 0;
const A_KEY: u8 = 1;
const A_QUOTED: u8 = 2;
const A_CLASS_MEMBER: u8 = 3;

const NO_VALUE: u32 = u32::MAX;

const NO_ASSIGN: Assign = Assign {
    mark: Mark::NONE,
    name: (0, 0),
    sep: (0, 0),
    value: NO_VALUE,
    kind: A_NAMED,
};

// The words: what a name is to the scan.
/// A keyword: its leaf, and the previous token it leaves.
const W_KW: u8 = 1;
/// A constant (`true`, `null`, `undefined`, …): its leaf, a value.
const W_CONST: u8 = 2;
/// `this`, `super`, `arguments`, `self`: its leaf, a value.
const W_LANG: u8 = 3;
/// A support class or function: its leaf, a value.
const W_SUPPORT: u8 = 4;
/// `console`, `process`, `module`, `exports`: a support object with members.
const W_SUPPORT_OBJ: u8 = 5;
/// `function`.
const W_FUNCTION: u8 = 6;
/// `class`.
const W_CLASS: u8 = 7;
/// `new`.
const W_NEW: u8 = 8;
/// `case`, `default`.
const W_CASE: u8 = 9;
/// Words that are names but where they start something: `async`, `get`, `set`, `static`,
/// `from`, `as`, `of`.
const W_CONTEXT: u8 = 10;
/// `extends`.
const W_EXTENDS: u8 = 11;
/// `styled`, `injectGlobal`, `keyframes`, `gql`: a template after them is plain.
const W_STYLED: u8 = 12;
/// Flow's `type` and `declare` at a statement's start.
const W_FLOW: u8 = 13;

const N: usize = 132;

/// Every word the scan tells apart: its class, its leaf, and the previous token it leaves.
const LIST: [(&[u8], u8, u16, u8); N] = [
    (b"Array", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"ArrayBuffer", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"Boolean", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"DataView", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"Date", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"Float32Array", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"Float64Array", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"Function", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"Int16Array", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"Int32Array", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"Int8Array", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"JSON", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"Map", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"Math", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"Number", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"Object", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"Promise", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"Proxy", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"Reflect", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"RegExp", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"Set", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"String", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"Symbol", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"System", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"TypeError", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"Uint16Array", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"Uint32Array", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"Uint8Array", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"Uint8ClampedArray", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"WeakMap", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"WeakSet", W_SUPPORT, BUILTIN_CLASS, P_VALUE),
    (b"Error", W_SUPPORT, ERROR_CLASS, P_VALUE),
    (b"EvalError", W_SUPPORT, ERROR_CLASS, P_VALUE),
    (b"RangeError", W_SUPPORT, ERROR_CLASS, P_VALUE),
    (b"ReferenceError", W_SUPPORT, ERROR_CLASS, P_VALUE),
    (b"SyntaxError", W_SUPPORT, ERROR_CLASS, P_VALUE),
    (b"URIError", W_SUPPORT, ERROR_CLASS, P_VALUE),
    (b"Buffer", W_SUPPORT, NODE_CLASS, P_VALUE),
    (b"Infinity", W_CONST, PLAIN, P_VALUE),
    (b"NaN", W_CONST, PLAIN, P_VALUE),
    (b"decodeURI", W_SUPPORT, BUILTIN_FN, P_VALUE),
    (b"decodeURIComponent", W_SUPPORT, BUILTIN_FN, P_VALUE),
    (b"encodeURI", W_SUPPORT, BUILTIN_FN, P_VALUE),
    (b"encodeURIComponent", W_SUPPORT, BUILTIN_FN, P_VALUE),
    (b"escape", W_SUPPORT, BUILTIN_FN, P_VALUE),
    (b"eval", W_SUPPORT, BUILTIN_FN, P_VALUE),
    (b"isFinite", W_SUPPORT, BUILTIN_FN, P_VALUE),
    (b"isNaN", W_SUPPORT, BUILTIN_FN, P_VALUE),
    (b"parseFloat", W_SUPPORT, BUILTIN_FN, P_VALUE),
    (b"parseInt", W_SUPPORT, BUILTIN_FN, P_VALUE),
    (b"unescape", W_SUPPORT, BUILTIN_FN, P_VALUE),
    (b"clearImmediate", W_SUPPORT, NODE_FN, P_VALUE),
    (b"clearInterval", W_SUPPORT, NODE_FN, P_VALUE),
    (b"clearTimeout", W_SUPPORT, NODE_FN, P_VALUE),
    (b"require", W_SUPPORT, NODE_FN, P_VALUE),
    (b"setImmediate", W_SUPPORT, NODE_FN, P_VALUE),
    (b"setInterval", W_SUPPORT, NODE_FN, P_VALUE),
    (b"setTimeout", W_SUPPORT, NODE_FN, P_VALUE),
    (b"document", W_SUPPORT, DOM, P_VALUE),
    (b"window", W_SUPPORT, DOM, P_VALUE),
    (b"global", W_SUPPORT, NODE_OBJ, P_VALUE),
    (b"GLOBAL", W_SUPPORT, NODE_OBJ, P_VALUE),
    (b"root", W_SUPPORT, NODE_OBJ, P_VALUE),
    (b"__dirname", W_SUPPORT, NODE_OBJ, P_VALUE),
    (b"__filename", W_SUPPORT, NODE_OBJ, P_VALUE),
    (b"console", W_SUPPORT_OBJ, CONSOLE, P_DOT_CONSOLE),
    (b"process", W_SUPPORT_OBJ, PROCESS, P_DOT_PROCESS),
    (b"exports", W_SUPPORT_OBJ, MODULE_OBJ, P_DOT_MODULE),
    (b"module", W_SUPPORT_OBJ, MODULE_OBJ, P_DOT_MODULE),
    (b"const", W_KW, STORAGE, P_OP),
    (b"let", W_KW, STORAGE, P_OP),
    (b"var", W_KW, STORAGE, P_OP),
    (b"await", W_KW, FLOW_KW, P_OP),
    (b"yield", W_KW, FLOW_KW, P_OP),
    (b"return", W_KW, FLOW_KW, P_OP),
    (b"if", W_KW, COND, P_OP),
    (b"else", W_KW, COND, P_START),
    (b"catch", W_KW, TRY, P_START),
    (b"finally", W_KW, TRY, P_START),
    (b"throw", W_KW, TRY, P_OP),
    (b"try", W_KW, TRY, P_START),
    (b"break", W_KW, LOOP, P_VALUE),
    (b"continue", W_KW, LOOP, P_VALUE),
    (b"do", W_KW, LOOP, P_START),
    (b"goto", W_KW, LOOP, P_OP),
    (b"while", W_KW, LOOP, P_OP),
    (b"for", W_KW, LOOP, P_OP),
    (b"switch", W_KW, SWITCH_KW, P_OP),
    (b"case", W_CASE, SWITCH_KW, P_OP),
    (b"default", W_CASE, SWITCH_KW, P_OP),
    (b"enum", W_KW, RESERVED, P_OP),
    (b"public", W_KW, RESERVED, P_OP),
    (b"package", W_KW, RESERVED, P_OP),
    (b"private", W_KW, RESERVED, P_OP),
    (b"protected", W_KW, RESERVED, P_OP),
    (b"interface", W_KW, RESERVED, P_OP),
    (b"debugger", W_KW, KW_OTHER, P_VALUE),
    (b"import", W_KW, MODULE, P_OP),
    (b"export", W_KW, MODULE, P_OP),
    (b"from", W_CONTEXT, MODULE, P_OP),
    (b"as", W_CONTEXT, MODULE, P_OP),
    (b"of", W_CONTEXT, OP, P_OP),
    (b"class", W_CLASS, CLASS_KW, P_OP),
    (b"extends", W_EXTENDS, EXTENDS, P_EXTENDS),
    (b"true", W_CONST, TRUE, P_VALUE),
    (b"false", W_CONST, FALSE, P_VALUE),
    (b"null", W_CONST, NULL, P_VALUE),
    (b"undefined", W_CONST, UNDEFINED, P_VALUE),
    (b"arguments", W_LANG, ARGUMENTS, P_VALUE),
    (b"super", W_LANG, SUPER, P_VALUE),
    (b"this", W_LANG, THIS, P_VALUE),
    (b"self", W_LANG, SELF, P_VALUE),
    (b"new", W_NEW, NEW, P_NEW),
    (b"delete", W_KW, OP, P_OP),
    (b"instanceof", W_KW, OP, P_OP),
    (b"in", W_KW, OP, P_OP),
    (b"typeof", W_KW, OP, P_OP),
    (b"void", W_KW, OP, P_OP),
    (b"with", W_KW, OP, P_OP),
    (b"async", W_CONTEXT, STORAGE, P_OP),
    (b"function", W_FUNCTION, STORAGE_FN, P_OP),
    (b"static", W_CONTEXT, MODIFIER, P_MEMBER),
    (b"get", W_CONTEXT, ACCESSOR_KW, P_MEMBER),
    (b"set", W_CONTEXT, ACCESSOR_KW, P_MEMBER),
    (b"styled", W_STYLED, OBJECT, P_VALUE),
    (b"injectGlobal", W_STYLED, TAGGED, P_VALUE),
    (b"keyframes", W_STYLED, TAGGED, P_VALUE),
    (b"gql", W_STYLED, PLAIN, P_VALUE),
    (b"prototype", W_LANG, PROTOTYPE, P_VALUE),
    (b"__proto__", W_LANG, PROTO, P_VALUE),
    (b"type", W_FLOW, TYPE_KW, P_OP),
    (b"declare", W_FLOW, DECLARE, P_START),
];

const WORD_LIST: [&[u8]; N] = {
    let mut w: [&[u8]; N] = [b""; N];
    let mut i = 0;
    while i < N {
        w[i] = LIST[i].0;
        i += 1;
    }
    w
};
static WORDS: Words<512, N, { text_len(&WORD_LIST) }> = Words::new(&WORD_LIST);

/// A listed word's id, at compile time.
const fn id_of(w: &[u8]) -> u16 {
    let mut i = 0;
    while i < N {
        let x = LIST[i].0;
        if x.len() == w.len() {
            let mut k = 0;
            while k < w.len() && x[k] == w[k] {
                k += 1;
            }
            if k == w.len() {
                return i as u16 + 1;
            }
        }
        i += 1;
    }
    panic!("not listed")
}

const ID_ASYNC: u16 = id_of(b"async");
const ID_FROM: u16 = id_of(b"from");
const ID_AS: u16 = id_of(b"as");
const ID_OF: u16 = id_of(b"of");
const ID_DEFAULT: u16 = id_of(b"default");
const ID_REGEXP: u16 = id_of(b"RegExp");
const ID_GQL: u16 = id_of(b"gql");
const ID_TYPE: u16 = id_of(b"type");
const ID_STYLED: u16 = id_of(b"styled");

/// What a word is by id: its class, leaf and the previous token after it; 0 for none.
static INFO: [(u8, u16, u8); N + 1] = {
    let mut t = [(0, 0, 0); N + 1];
    let mut i = 0;
    while i < N {
        t[i + 1] = (LIST[i].1, LIST[i].2, LIST[i].3);
        i += 1;
    }
    t
};

// A byte's class: what the loop does at it.
const A_OTHER: u8 = 0;
const A_BLANK: u8 = 1;
const A_NAME: u8 = 2;
const A_DIGIT: u8 = 3;
const A_QUOTE: u8 = 4;
const A_SLASH: u8 = 5;
const A_PAREN: u8 = 6;
const A_CLOSE_PAREN: u8 = 7;
const A_BRACKET: u8 = 8;
const A_CLOSE_BRACKET: u8 = 9;
const A_CURLY: u8 = 10;
const A_CLOSE_CURLY: u8 = 11;
const A_DOT: u8 = 12;
const A_SEMI: u8 = 13;
const A_COMMA: u8 = 14;
const A_COLON: u8 = 15;
const A_QUESTION: u8 = 16;
const A_EQ: u8 = 17;
const A_OP: u8 = 18;
const A_LT: u8 = 19;
const A_TICK: u8 = 20;
const A_SIGN: u8 = 21;
const A_HIGH: u8 = 22;
const A_AT: u8 = 23;
const A_STAR: u8 = 24;
const A_HASH: u8 = 25;

static ACT: [u8; 256] = {
    let mut t = [A_OTHER; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        t[c] = match b {
            b' ' | b'\t' => A_BLANK,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' => A_NAME,
            b'0'..=b'9' => A_DIGIT,
            b'"' | b'\'' => A_QUOTE,
            b'/' => A_SLASH,
            b'(' => A_PAREN,
            b')' => A_CLOSE_PAREN,
            b'[' => A_BRACKET,
            b']' => A_CLOSE_BRACKET,
            b'{' => A_CURLY,
            b'}' => A_CLOSE_CURLY,
            b'.' => A_DOT,
            b';' => A_SEMI,
            b',' => A_COMMA,
            b':' => A_COLON,
            b'?' => A_QUESTION,
            b'=' => A_EQ,
            b'!' | b'%' | b'&' | b'|' | b'^' | b'~' | b'>' => A_OP,
            b'*' => A_STAR,
            b'<' => A_LT,
            b'`' => A_TICK,
            b'+' | b'-' => A_SIGN,
            b'@' => A_AT,
            b'#' => A_HASH,
            0x80..=0xff => A_HIGH,
            _ => A_OTHER,
        };
        c += 1;
    }
    t
};

/// A name's bytes past its first, in ASCII: `[$\w]`.
static NAME: [bool; 256] = {
    let mut t = WORD;
    t[b'$' as usize] = true;
    t
};

/// A name's ASCII byte or a byte past ASCII: where a name may go on.
static NAME_OR_HIGH: [bool; 256] = {
    let mut t = NAME;
    let mut c = 0x80;
    while c < 256 {
        t[c] = true;
        c += 1;
    }
    t
};

/// The scan of a line: its frames, the previous token, and where the writing goes.
pub(super) struct Scan<'b, 'c, K: Sink> {
    b: &'b [u8],
    stack: [u8; MAX],
    depth: usize,
    /// The top frame, `stack[depth - 1]`.
    cur: u8,
    /// The previous significant token ([`P_START`] …).
    prev: u8,
    /// Whether a line came before this one in the state's run (a shebang only on the first).
    later: bool,
    /// Where the spans so far end, for a text frame's fill: kept by the text frames only.
    at: usize,
    groups: [std::mem::MaybeUninit<Group>; GROUPS],
    ngroups: usize,
    assign: Assign,
    /// Where a string after `RegExp(` would start: it's a regex.
    re_arg: usize,
    /// An `async` before a `(` (`async (…) =>`): the `(`'s place, the `async`'s, and the
    /// writing's mark from before it.
    asy: (usize, usize, Mark),
    /// A capitalized name before a `.` (`Foo.`): the `.`'s place, the name's, and the writing's
    /// mark from before it, for `Foo.bar = function`, which names `Foo` a class.
    cls: (usize, usize, Mark),
    k: K,
    out: &'b mut Out<'c>,
}

impl<'b, 'c, K: Sink> Scan<'b, 'c, K> {
    /// A scan of `b`, its spans from `from`, returned as a literal so that it is built in place
    /// (a scan changed before it's returned is copied whole, a line's fixed cost); the state is
    /// [`Scan::load`]'s.
    #[inline(always)]
    fn fresh(b: &'b [u8], from: usize, out: &'b mut Out<'c>, k: K) -> Self {
        Scan {
            b,
            stack: [F_TOP; MAX],
            depth: 1,
            cur: F_TOP,
            prev: P_START,
            later: false,
            at: from,
            // read only below `ngroups`
            groups: [const { std::mem::MaybeUninit::uninit() }; GROUPS],
            ngroups: 0,
            assign: NO_ASSIGN,
            re_arg: usize::MAX,
            asy: (usize::MAX, 0, Mark::NONE),
            cls: (usize::MAX, 0, Mark::NONE),
            k,
            out,
        }
    }

    /// The stack, and what is read from it and the state's bits, from `state`.
    #[inline(always)]
    fn load(&mut self, state: &State) {
        self.depth = state.load(&mut self.stack);
        self.cur = self.stack[self.depth - 1];
        let arg = state.flags();
        self.prev = (arg & ARG_PREV) as u8;
        self.later = arg & ARG_LATER != 0;
    }

    /// The state this line leaves.
    #[inline(always)]
    fn keep(&self) -> State {
        let arg = u16::from(self.prev) | ARG_LATER;
        State::keep(&self.stack, self.depth, arg)
    }

    /// The line lexed from `from`.
    #[inline(always)]
    fn lex(&mut self, from: usize) {
        self.ngroups = 0;
        self.assign.value = NO_VALUE;
        self.re_arg = usize::MAX;
        self.asy.0 = usize::MAX;
        self.cls.0 = usize::MAX;
        let mut p = from;
        if matches!(self.cur, F_TYPE | F_TYPE_DECL) && self.prev == P_VALUE {
            // a type the line before ended (no `;`, `,` or closer after it)
            self.pop();
        }
        if self.cur == F_FIELD && matches!(self.prev, P_VALUE | P_START) {
            // a field's value ended with the line before (no `;`)
            self.pop();
            self.prev = P_MEMBER;
        }
        if !self.later && from == 0 && self.depth == 1 && has(self.b, 0, b"#!") {
            self.tok(0, 2, SHEBANG_MARK);
            self.tok(2, self.b.len(), SHEBANG);
            p = self.b.len();
        }
        self.run(p);
        self.later = true;
    }

    #[inline(always)]
    fn top(&self) -> u8 {
        self.cur
    }

    #[inline(always)]
    fn push(&mut self, c: u8) {
        if self.depth == MAX {
            self.full();
        }
        self.stack[self.depth] = c;
        self.depth += 1;
        self.cur = c;
    }

    #[cold]
    #[inline(never)]
    fn full(&mut self) {
        crate::stack::drop_bottom(&mut self.stack, &mut self.depth);
    }

    #[inline(always)]
    fn pop(&mut self) {
        if self.depth > 1 {
            self.depth -= 1;
            self.cur = self.stack[self.depth - 1];
        }
    }

    #[inline(always)]
    fn set_top(&mut self, c: u8) {
        self.stack[self.depth - 1] = c;
        self.cur = c;
    }

    /// Bytes `s..e` are `leaf`.
    #[inline(always)]
    fn tok(&mut self, s: usize, e: usize, leaf: u16) -> usize {
        self.k.merge(&mut self.out, s, e, leaf);
        e
    }

    /// [`Scan::tok`] out of line, for the paths that run seldom.
    #[inline(never)]
    fn tok_far(&mut self, s: usize, e: usize, leaf: u16) -> usize {
        self.tok(s, e, leaf)
    }

    /// The text from [`Scan::at`] to `p` is `text`, and the spans end at `p`.
    #[inline(always)]
    fn fill_as(&mut self, p: usize, text: u16) {
        let p = p.min(self.b.len());
        if p > self.at {
            if text != PLAIN {
                self.k.merge(&mut self.out, self.at, p, text);
            }
            self.at = p;
        }
    }

    /// The line from `p`.
    fn run(&mut self, mut p: usize) {
        let b = self.b;
        let n = b.len();
        loop {
            let top = self.top();
            if top >= F_TYPE {
                p = if top >= TEXT {
                    self.text_turn(p, top)
                } else {
                    self.type_turn(p, top)
                };
                if p >= n {
                    return;
                }
                continue;
            }
            if p >= n {
                return;
            }
            let mut c = b[p];
            if c == b' ' || c == b'\t' {
                p += 1;
                while p < n && matches!(b[p], b' ' | b'\t') {
                    p += 1;
                }
                if p >= n {
                    return;
                }
                c = b[p];
            }
            p = match ACT[usize::from(c)] {
                A_NAME => self.name(p, top),
                A_PAREN => self.open_paren(p, top),
                A_CLOSE_PAREN => self.close_paren(p),
                A_DOT => self.dot(p),
                A_COMMA => self.comma(p, top),
                A_SEMI => self.semi(p),
                A_QUOTE => self.string(p, c),
                A_EQ => self.equals(p),
                A_CURLY => self.open_curly(p, top),
                A_CLOSE_CURLY => self.close_curly(p),
                A_BRACKET => self.open_bracket(p, top),
                A_CLOSE_BRACKET => self.close_bracket(p),
                A_DIGIT => self.number(p),
                A_COLON => self.colon(p, top),
                A_SLASH => self.slash(p),
                A_QUESTION => self.question(p),
                A_OP => self.op(p, c),
                A_STAR => self.star(p, top),
                A_SIGN => self.sign(p, c),
                A_LT => self.less(p),
                A_TICK => self.template(p, false),
                A_HIGH => self.high(p, top),
                A_AT => self.decorator(p),
                A_BLANK => p + 1,
                _ => p + 1,
            };
        }
    }

    // --- names ---

    /// A name at `p`, its first byte ASCII's, in code frame `top`.
    #[inline(always)]
    fn name(&mut self, p: usize, top: u8) -> usize {
        let b = self.b;
        let mut e = p + 1;
        while e < b.len() && NAME[usize::from(b[e])] {
            e += 1;
        }
        if e < b.len() && b[e] >= 0x80 {
            return self.wide_name(p, top);
        }
        self.name_to(p, e, top, true)
    }

    /// A name `p..e` in code frame `top` (`ascii`: all its bytes ASCII's, so it may be a word).
    #[inline(always)]
    fn name_to(&mut self, p: usize, e: usize, top: u8, ascii: bool) -> usize {
        let b = self.b;
        let prev = self.prev;
        if (P_DOT_CONSOLE..=P_DOT_MODULE).contains(&prev) {
            return self.support_member(p, e);
        }
        if prev == P_DOT {
            return self.member(p, e);
        }
        if top >= F_PARAMS {
            match top {
                F_PARAMS | F_PATTERN => return self.param(p, e, top),
                F_CLASS_BODY => return self.class_member(p, e),
                F_FUNC_HEAD => {
                    self.tok(p, e, ENTITY_FN);
                    self.prev = P_VALUE;
                    return e;
                }
                F_CLASS_HEAD if prev != P_EXTENDS => {
                    if ascii && &b[p..e] == b"extends" {
                        self.tok(p, e, EXTENDS);
                        self.prev = P_EXTENDS;
                        return e;
                    }
                    self.tok(p, e, ENTITY_CLASS);
                    self.prev = P_VALUE;
                    return e;
                }
                _ => {}
            }
        }
        if (prev == P_KEY || prev == P_ACCESSOR) && top == F_OBJECT {
            return self.key(p, e);
        }
        let id = if ascii { WORDS.get(&b[p..e]) } else { 0 };
        if id != 0
            && let Some(q) = self.word(p, e, id, top)
        {
            return q;
        }
        if prev == P_NEW {
            // `new a.b.C(`: the path before its last name plain, as Babel's constructor reads it,
            // its last name the constructor's (a support word's too: `new errors.TypeError(`)
            let (mut s, mut q) = (p, e);
            while b.get(q) == Some(&b'.') && b.get(q + 1).is_some_and(|&c| NAME[usize::from(c)]) {
                s = q + 1;
                q = s + 1;
                while q < b.len() && NAME[usize::from(b[q])] {
                    q += 1;
                }
            }
            if s > p && q < b.len() && b[q] >= 0x80 {
                // a last name past ASCII: read by characters from its start, after `new` still
                return s;
            }
            self.tok(s, q, CTOR_NAME);
            self.prev = P_VALUE;
            return q;
        }
        let next = b.get(e).copied();
        let kind = match next {
            Some(b'(') => FUNCTION,
            Some(b'.') => {
                if is_upper(b[p]) {
                    if has(b, e + 1, b"prototype") {
                        ENTITY_CLASS
                    } else {
                        self.cls = (e, p, self.k.mark(self.out));
                        CLASS_VAR
                    }
                } else {
                    OBJECT
                }
            }
            Some(b'`') => TAGGED,
            Some(b' ' | b'\t' | b'=' | b':') => return self.name_before(p, e, top),
            _ => var_kind(b, p, e),
        };
        self.tok(p, e, kind);
        self.prev = P_VALUE;
        e
    }

    /// A name `p..e` before a blank, a `=` or a `:`: a parameter before `=>`, an assigned or
    /// labeled name, else a variable.
    #[inline(never)]
    fn name_before(&mut self, p: usize, e: usize, top: u8) -> usize {
        let b = self.b;
        let s = skip_blanks(b, e);
        let kind = match b.get(s) {
            Some(b'=') => match b.get(s + 1) {
                Some(b'>') => {
                    if self.assign.value == p as u32 {
                        self.function_named(p);
                    }
                    self.tok(p, e, PARAM);
                    self.prev = P_VALUE;
                    return e;
                }
                Some(b'=') => var_kind(b, p, e),
                _ => {
                    self.assigned(p, e, s, false);
                    var_kind(b, p, e)
                }
            },
            Some(b':') if matches!(top, F_TOP | F_BLOCK) && b.get(s + 1) != Some(&b':') => {
                // a label
                self.tok(p, e, OBJ_KEY_LABEL);
                self.tok(e, s, OBJ_KEY);
                self.tok(s, s + 1, OBJ_KEY_SEP);
                self.prev = P_START;
                return s + 1;
            }
            Some(b'(') => FUNCTION,
            Some(b'`') => TAGGED,
            _ => var_kind(b, p, e),
        };
        self.tok(p, e, kind);
        self.prev = P_VALUE;
        e
    }

    /// `name =` (or a key's `name:`, `key`) at `p..e`, its separator at `s`: a mark kept, for
    /// a value that turns out a function.
    #[inline(always)]
    fn assigned(&mut self, p: usize, e: usize, s: usize, key: bool) {
        let mark = self.k.mark(self.out);
        self.assigned_at(mark, p, e, s, if key { A_KEY } else { A_NAMED });
    }

    /// [`Scan::assigned`] with the writing's mark from before the name (a quoted key's).
    #[inline(always)]
    fn assigned_at(&mut self, mark: Mark, p: usize, e: usize, s: usize, kind: u8) {
        let v = skip_blanks(self.b, s + 1);
        self.assign = Assign {
            mark,
            name: (p as u32, e as u32),
            sep: (s as u32, e as u32),
            value: v as u32,
            kind,
        };
    }

    /// The function whose value starts at [`Assign::value`] names the assigned name: what was
    /// written since its mark taken back and written again with the name a function's.
    #[inline(never)]
    fn function_named(&mut self, _at: usize) {
        let a = self.assign;
        self.assign.value = NO_VALUE;
        self.k.undo(self.out, a.mark);
        let (s, e) = (a.name.0 as usize, a.name.1 as usize);
        let sep = a.sep.0 as usize;
        if a.kind == A_KEY {
            self.tok(s, e, KEY_NAME_FN);
            self.tok(e, sep, OBJ_KEY);
            self.tok(sep, sep + 1, OBJ_KEY_SEP);
        } else if a.kind == A_QUOTED {
            self.tok(s, s + 1, STR_BEGIN);
            self.tok(s + 1, e - 1, KEY_FN);
            self.tok(e - 1, e, STR_END);
            self.tok(sep, sep + 1, KEY_SEP);
        } else if a.kind == A_CLASS_MEMBER {
            // `Foo.bar = function`: `Foo` a class's name
            self.tok(a.sep.1 as usize, s - 1, ENTITY_CLASS);
            self.tok(s - 1, s, ACCESSOR);
            self.tok(s, e, ENTITY_FN);
            self.tok(sep, sep + 1, ASSIGN);
        } else {
            self.tok(s, e, ENTITY_FN);
            self.tok(sep, sep + 1, ASSIGN);
        }
    }

    /// A word of [`WORDS`] at `p..e`, `id`, in code frame `top`: where the scan goes, or `None`
    /// where it is a name here.
    #[inline(always)]
    fn word(&mut self, p: usize, e: usize, id: u16, _top: u8) -> Option<usize> {
        let (class, leaf, after) = INFO[usize::from(id)];
        match class {
            W_KW | W_CONST | W_LANG | W_SUPPORT => {
                if leaf == PLAIN {
                    // `Infinity`, `NaN`: plain unless signed ([`Scan::sign`])
                    self.prev = after;
                    return Some(e);
                }
                if class == W_SUPPORT && id == ID_REGEXP {
                    let s = skip_blanks(self.b, e);
                    if self.b.get(s) == Some(&b'(') {
                        self.re_arg = skip_blanks(self.b, s + 1);
                    }
                }
                self.tok(p, e, leaf);
                self.prev = after;
                Some(e)
            }
            W_SUPPORT_OBJ => {
                self.tok(p, e, leaf);
                self.prev = if self.b.get(e) == Some(&b'.') {
                    after
                } else {
                    P_VALUE
                };
                Some(e)
            }
            W_FUNCTION => Some(self.function(p, e)),
            W_CLASS => {
                self.tok(p, e, leaf);
                self.push(F_CLASS_HEAD);
                self.prev = P_OP;
                Some(e)
            }
            W_NEW => {
                self.tok(p, e, leaf);
                self.prev = P_NEW;
                Some(e)
            }
            W_CASE => {
                if id == ID_DEFAULT && self.prev != P_START {
                    // `export default`
                    self.tok(p, e, LOOP);
                    self.prev = P_OP;
                    return Some(e);
                }
                self.tok(p, e, leaf);
                self.push(F_CASE);
                self.prev = P_OP;
                Some(e)
            }
            W_CONTEXT => self.context_word(p, e, id, leaf),
            W_EXTENDS => None,
            W_STYLED => self.styled(p, e, id),
            W_FLOW => self.flow_word(p, e, id),
            _ => None,
        }
    }

    /// `async`, `of`, `from`, `as` at `p..e`: a keyword where it starts something, else `None`.
    #[inline(never)]
    fn context_word(&mut self, p: usize, e: usize, id: u16, leaf: u16) -> Option<usize> {
        let b = self.b;
        let s = skip_blanks(b, e);
        let next = b.get(s).copied();
        match id {
            ID_ASYNC => {
                // before `function`, an arrow's parameter, a method's name; before a `(`, a
                // variable until the group turns out an arrow's parameters
                match next {
                    Some(b'(') => {
                        let mark = self.k.mark(self.out);
                        self.asy = (s, p, mark);
                        self.tok(p, e, READWRITE);
                    }
                    Some(c) if (c.is_ascii_alphabetic() || c == b'_' || c == b'$') && s > e => {
                        self.tok(p, e, leaf);
                    }
                    _ => return None,
                }
                self.prev = P_OP;
                Some(e)
            }
            ID_OF | ID_FROM | ID_AS
                if self.prev == P_VALUE || (id == ID_AS && self.after_star(p)) =>
            {
                self.tok(p, e, leaf);
                self.prev = P_OP;
                Some(e)
            }
            _ => None,
        }
    }

    /// `function` at `p..e`: its head's frame pushed.
    #[inline(never)]
    fn function(&mut self, p: usize, e: usize) -> usize {
        if self.assign.value == p as u32 {
            self.function_named(p);
        } else if self.assign.value != NO_VALUE {
            // `name = async function`: the `async` before it
            let v = self.assign.value as usize;
            if has(self.b, v, b"async") && skip_blanks(self.b, v + 5) == p {
                self.function_named(p);
                self.tok(v, v + 5, STORAGE);
            }
        }
        self.tok(p, e, STORAGE_FN);
        self.push(F_FUNC_HEAD);
        self.prev = P_OP;
        e
    }

    /// A name `p..e` after a `.`: a member.
    #[inline(always)]
    fn member(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        self.prev = P_VALUE;
        let kind = match b.get(e) {
            Some(b'(') => FUNCTION,
            Some(b'.') => {
                if e - p == 9 && &b[p..e] == b"prototype" {
                    PROTOTYPE
                } else {
                    PROPERTY
                }
            }
            Some(b' ' | b'\t' | b'=') => {
                let s = skip_blanks(b, e);
                if b.get(s) == Some(&b'=') && !matches!(b.get(s + 1), Some(b'=' | b'>')) {
                    if p > 0 && self.cls.0 == p - 1 {
                        // `Foo.bar =`: the mark from before `Foo`
                        self.assigned_at(self.cls.2, p, e, s, A_CLASS_MEMBER);
                        self.assign.sep.1 = self.cls.1 as u32;
                    } else {
                        self.assigned(p, e, s, false);
                    }
                }
                if b.get(s) == Some(&b'(') {
                    FUNCTION
                } else {
                    PROPERTY
                }
            }
            _ => {
                if is_upper(b[p]) && upper_only(&b[p..e]) {
                    CONSTANT
                } else {
                    PROPERTY
                }
            }
        };
        self.tok(p, e, kind)
    }

    /// A member `p..e` of a support object.
    #[inline(never)]
    fn support_member(&mut self, p: usize, e: usize) -> usize {
        let w = &self.b[p..e];
        let found: Option<u16> = match self.prev {
            P_DOT_CONSOLE => CONSOLE_FNS.contains(&w).then_some(CONSOLE_FN),
            P_DOT_PROCESS => {
                if PROCESS_PROPS.contains(&w) {
                    Some(PROCESS)
                } else {
                    PROCESS_FNS.contains(&w).then_some(PROCESS_FN)
                }
            }
            _ => MODULE_PROPS.contains(&w).then_some(MODULE_OBJ),
        };
        match found {
            Some(leaf) => {
                self.prev = P_VALUE;
                self.tok(p, e, leaf)
            }
            None => {
                self.prev = P_DOT;
                self.member(p, e)
            }
        }
    }

    /// A key `p..e` at an object's key place: a key before `:`, a method's name before `(`, an
    /// accessor's or a modifier's word before a name, else a shorthand's variable.
    #[inline(never)]
    fn key(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let s = skip_blanks(b, e);
        match b.get(s) {
            Some(b':') => {
                self.assigned(p, e, s, true);
                self.tok(p, e, OBJ_KEY_LABEL);
                self.tok(e, s, OBJ_KEY);
                self.tok(s, s + 1, OBJ_KEY_SEP);
                self.prev = P_OP;
                s + 1
            }
            Some(b'(') => {
                let leaf = if self.prev == P_ACCESSOR {
                    ENTITY_ACCESSOR
                } else {
                    ENTITY_FN
                };
                self.tok(p, e, leaf);
                self.open_params(s)
            }
            Some(&c)
                if s > e
                    && self.prev == P_KEY
                    && (c.is_ascii_alphabetic()
                        || c == b'_'
                        || c == b'$'
                        || c == b'*'
                        || c == b'[') =>
            {
                // `get name(`, `set name(`, `async name(`, `static`: a modifier
                let leaf = match &b[p..e] {
                    b"get" | b"set" => ACCESSOR_KW,
                    b"async" => STORAGE,
                    b"static" => MODIFIER,
                    _ => {
                        self.tok(p, e, var_kind(b, p, e));
                        self.prev = P_VALUE;
                        return e;
                    }
                };
                self.tok(p, e, leaf);
                // the key place stays, an accessor's after `get` or `set`
                if leaf == ACCESSOR_KW {
                    self.prev = P_ACCESSOR;
                }
                e
            }
            _ => {
                self.tok(p, e, shorthand_kind(b, p, e));
                self.prev = P_VALUE;
                e
            }
        }
    }

    /// A parameter `p..e`, in [`F_PARAMS`] or a [`F_PATTERN`].
    #[inline(always)]
    fn param(&mut self, p: usize, e: usize, top: u8) -> usize {
        let b = self.b;
        if top == F_PATTERN && self.stack[self.depth - 2] != F_PATTERN {
            // a key before `:` in an object pattern renames
            let s = skip_blanks(b, e);
            if b.get(s) == Some(&b':') {
                self.tok(p, e, OBJ_KEY_LABEL);
                self.tok(s, s + 1, OBJ_KEY_SEP);
                return s + 1;
            }
        }
        self.tok(p, e, PARAM);
        self.prev = P_VALUE;
        e
    }

    /// A class member's name `p..e`, or its modifier.
    #[inline(never)]
    fn class_member(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let s = skip_blanks(b, e);
        let next = b.get(s).copied();
        match next {
            Some(b'(') => {
                // a class's accessors are its methods' color in Babel, an object's not
                self.tok(p, e, ENTITY_FN);
                return self.open_params(s);
            }
            Some(b'=') => {
                self.tok(p, e, PROPERTY);
                self.tok(s, s + 1, ASSIGN);
                self.push(F_FIELD);
                self.prev = P_OP;
                return s + 1;
            }
            Some(b':') => {
                // a field's annotation, then its value
                self.tok(p, e, PROPERTY);
                self.push(F_FIELD);
                return self.annotation(s);
            }
            Some(b'<') => {
                // a method's type parameters
                self.tok(p, e, ENTITY_FN);
                self.prev = P_KEYED;
                return self.type_open(s, F_TYPE_ANGLE);
            }
            _ => {}
        }
        let leaf = match &b[p..e] {
            b"static" => MODIFIER,
            b"get" | b"set" => ACCESSOR_KW,
            b"async" => STORAGE,
            _ => PROPERTY,
        };
        self.tok(p, e, leaf);
        self.prev = if leaf == ACCESSOR_KW {
            P_ACCESSOR
        } else {
            P_MEMBER
        };
        e
    }

    /// A name past ASCII at `p` (or one with such bytes): read by characters.
    #[inline(never)]
    fn wide_name(&mut self, p: usize, top: u8) -> usize {
        let b = self.b;
        let mut e = p;
        while e < b.len() && NAME_OR_HIGH[usize::from(b[e])] {
            if b[e] >= 0x80 {
                if !is_word(b, e) {
                    break;
                }
                e += char_len(b[e]);
            } else {
                e += 1;
            }
        }
        if e == p {
            return p + char_len(b[p]);
        }
        self.name_to(p, e, top, false)
    }

    /// A byte past ASCII at `p` in code: a name's start, else a character of its own.
    #[inline(never)]
    fn high(&mut self, p: usize, top: u8) -> usize {
        if is_word(self.b, p) {
            return self.wide_name(p, top);
        }
        p + char_len(self.b[p])
    }

    /// `styled`, `injectGlobal`, `keyframes`, `gql` at `p..e`: before a template (or
    /// `styled.x` before one), the template's text plain.
    #[inline(never)]
    fn styled(&mut self, p: usize, e: usize, id: u16) -> Option<usize> {
        let b = self.b;
        if id == ID_STYLED {
            if b.get(e) == Some(&b'.') {
                let m = e + 1;
                let mut me = m;
                while me < b.len() && NAME[usize::from(b[me])] {
                    me += 1;
                }
                let s = skip_blanks(b, me);
                if me > m && b.get(s) == Some(&b'`') {
                    self.tok(p, e, OBJECT);
                    self.tok(e, m, PUNCT_ACCESSOR);
                    self.tok(m, me, TAGGED);
                    return Some(self.template(s, true));
                }
            }
            return None;
        }
        let s = skip_blanks(b, e);
        if b.get(s) != Some(&b'`') {
            return None;
        }
        if id != ID_GQL {
            self.tok(p, e, TAGGED);
        }
        Some(self.template(s, true))
    }

    // --- punctuation ---

    /// A `(` at `p` in code frame `top`.
    #[inline(always)]
    fn open_paren(&mut self, p: usize, top: u8) -> usize {
        if top == F_FUNC_HEAD {
            self.pop();
            return self.open_params(p);
        }
        // a group after an operator may be an arrow's parameters: kept with a mark
        if self.prev == P_KEYED {
            return self.open_params(p);
        }
        if self.prev != P_VALUE && self.ngroups < GROUPS {
            let (mut mark, mut asy) = (self.k.mark(self.out), NO_VALUE);
            if self.asy.0 == p {
                mark = self.asy.2;
                asy = self.asy.1 as u32;
            }
            let assign = if self.assign.value == p as u32 || self.async_value(p) {
                self.assign
            } else {
                NO_ASSIGN
            };
            self.groups[self.ngroups].write(Group {
                mark,
                open: p as u32,
                depth: self.depth as u16 + 1,
                assign,
                asy,
            });
            self.ngroups += 1;
        }
        self.push(F_PAREN);
        self.tok(p, p + 1, ROUND_BEGIN);
        self.prev = P_OP;
        p + 1
    }

    /// A function's parameters' `(` at `p`.
    #[inline(always)]
    fn open_params(&mut self, p: usize) -> usize {
        self.push(F_PARAMS);
        self.tok(p, p + 1, PARAMS_BEGIN);
        self.prev = P_OP;
        p + 1
    }

    /// A `)` at `p`.
    #[inline(always)]
    fn close_paren(&mut self, p: usize) -> usize {
        let mut top = self.top();
        if top != F_PAREN {
            top = self.unwind(top, b')');
        }
        match top {
            F_PAREN => {
                if self.ngroups > 0
                    // SAFETY: entries below the count are written
                    && usize::from(unsafe { self.groups[self.ngroups - 1].assume_init_ref() }.depth)
                        == self.depth
                {
                    self.ngroups -= 1;
                    // SAFETY: as above
                    let g = unsafe { self.groups[self.ngroups].assume_init() };
                    if let Some(q) = self.arrow_group(p, g) {
                        return q;
                    }
                }
                self.pop();
                self.tok(p, p + 1, ROUND_END);
            }
            F_PARAMS => {
                self.pop();
                self.tok(p, p + 1, PARAMS_END);
                self.prev = P_PARAMS_END;
                return p + 1;
            }
            _ => {
                self.tok(p, p + 1, ROUND_END);
            }
        }
        self.prev = P_VALUE;
        p + 1
    }

    /// The group `g` closing at `p`: if a `=>` follows, its writing taken back and its `(` read
    /// again as parameters' (and an assigned name named), `Some` with where to go on.
    #[inline(always)]
    fn arrow_group(&mut self, p: usize, g: Group) -> Option<usize> {
        let b = self.b;
        let t = skip_blanks(b, p + 1);
        if !(b.get(t) == Some(&b'=') && b.get(t + 1) == Some(&b'>')) {
            return None;
        }
        Some(self.arrow_params(g))
    }

    #[inline(never)]
    fn arrow_params(&mut self, g: Group) -> usize {
        let open = g.open as usize;
        self.pop();
        // groups opened inside it are gone with its writing
        if g.assign.value == g.open {
            self.assign = g.assign;
            self.function_named(open);
        } else if g.assign.value != NO_VALUE {
            let v = g.assign.value as usize;
            self.assign = g.assign;
            self.function_named(open);
            self.tok(v, v + 5, STORAGE);
        } else {
            self.k.undo(self.out, g.mark);
            if g.asy != NO_VALUE {
                let a = g.asy as usize;
                self.tok(a, a + 5, STORAGE);
            }
        }
        self.open_params(open)
    }

    /// Whether the assignment's value is `async` before the `(` at `p`.
    #[inline(always)]
    fn async_value(&self, p: usize) -> bool {
        let v = self.assign.value;
        v != NO_VALUE && {
            let v = v as usize;
            v + 5 <= p && has(self.b, v, b"async") && skip_blanks(self.b, v + 5) == p
        }
    }

    /// The frames above the one a closer `c` closes popped (a ternary's, a `case`'s, a
    /// default's): the top after.
    #[inline(never)]
    fn unwind(&mut self, mut top: u8, c: u8) -> u8 {
        while matches!(top, F_TERNARY | F_CASE | F_DEFAULT | F_FIELD) && self.depth > 1 {
            if top == F_FIELD && c != b'}' {
                break;
            }
            self.pop();
            top = self.top();
        }
        top
    }

    /// A `[` at `p`.
    #[inline(always)]
    fn open_bracket(&mut self, p: usize, top: u8) -> usize {
        if (top == F_OBJECT && matches!(self.prev, P_KEY | P_ACCESSOR))
            || (top == F_CLASS_BODY && matches!(self.prev, P_MEMBER | P_ACCESSOR))
        {
            self.push(F_KEY_BRACKET);
            self.tok(p, p + 1, SQUARE_BEGIN);
            self.prev = P_OP;
            return p + 1;
        }
        if matches!(top, F_PARAMS | F_PATTERN) {
            self.push(F_PATTERN);
            self.tok(p, p + 1, SQUARE_BEGIN);
            self.prev = P_OP;
            return p + 1;
        }
        self.push(F_BRACKET);
        self.tok(p, p + 1, SQUARE_BEGIN);
        self.prev = P_OP;
        p + 1
    }

    /// A `]` at `p`.
    #[inline(always)]
    fn close_bracket(&mut self, p: usize) -> usize {
        let mut top = self.top();
        if top != F_BRACKET {
            top = self.unwind(top, b']');
        }
        let keyed = top == F_KEY_BRACKET;
        if matches!(top, F_BRACKET | F_PATTERN | F_KEY_BRACKET) {
            self.pop();
        }
        self.tok(p, p + 1, SQUARE_END);
        self.prev = if keyed { P_KEYED } else { P_VALUE };
        p + 1
    }

    /// A `{` at `p`: a class's body, a pattern's, an object's or a block's.
    #[inline(always)]
    fn open_curly(&mut self, p: usize, top: u8) -> usize {
        let (frame, prev) = match top {
            F_CLASS_HEAD => {
                self.pop();
                (F_CLASS_BODY, P_MEMBER)
            }
            F_PARAMS | F_PATTERN => (F_PATTERN, P_OP),
            _ => match self.prev {
                P_OP | P_KEY | P_NEW => (F_OBJECT, P_KEY),
                _ => (F_BLOCK, P_START),
            },
        };
        self.push(frame);
        self.tok(p, p + 1, CURLY_BEGIN);
        self.prev = prev;
        p + 1
    }

    /// A `}` at `p`.
    #[inline(always)]
    fn close_curly(&mut self, p: usize) -> usize {
        let mut top = self.top();
        if top >= F_TERNARY && top != F_CLASS_BODY && top != F_PATTERN {
            top = self.unwind(top, b'}');
        }
        match top {
            F_SUBST => {
                self.pop();
                self.tok(p, p + 1, SUBST_END);
                self.at = p + 1;
                return p + 1;
            }
            F_SUBST_PLAIN => {
                self.pop();
                self.at = p + 1;
                return p + 1;
            }
            F_J_EXPR => {
                self.pop();
                self.tok(p, p + 1, EMBED_END);
                self.at = p + 1;
                return p + 1;
            }
            F_OBJECT | F_PATTERN => {
                self.pop();
                self.prev = P_VALUE;
            }
            F_BLOCK => {
                self.pop();
                self.prev = if self.top() == F_CLASS_BODY {
                    P_MEMBER
                } else {
                    P_START
                };
            }
            F_CLASS_BODY => {
                self.pop();
                self.prev = P_VALUE;
            }
            _ => self.prev = P_START,
        }
        self.tok(p, p + 1, CURLY_END);
        p + 1
    }

    /// A `.` at `p`: a number's, a spread's, or an accessor.
    #[inline(always)]
    fn dot(&mut self, p: usize) -> usize {
        let b = self.b;
        match b.get(p + 1) {
            Some(b'0'..=b'9') => return self.number(p),
            Some(b'.') if b.get(p + 2) == Some(&b'.') => {
                self.tok(p, p + 3, SPREAD);
                self.prev = P_OP;
                return p + 3;
            }
            _ => {}
        }
        self.tok(p, p + 1, ACCESSOR);
        if self.prev < P_DOT_CONSOLE {
            self.prev = P_DOT;
        }
        p + 1
    }

    /// A `,` at `p` in frame `top`.
    #[inline(always)]
    fn comma(&mut self, p: usize, mut top: u8) -> usize {
        if matches!(top, F_TERNARY | F_DEFAULT) {
            top = self.unwind(top, b',');
        }
        match top {
            F_OBJECT => {
                self.prev = P_KEY;
                self.tok(p, p + 1, COMMA)
            }
            F_PARAMS | F_PATTERN => {
                self.prev = P_OP;
                self.tok(p, p + 1, PARAM_SEP)
            }
            _ => {
                self.prev = P_OP;
                self.tok(p, p + 1, COMMA)
            }
        }
    }

    /// A `;` at `p`.
    #[inline(always)]
    fn semi(&mut self, p: usize) -> usize {
        let top = self.top();
        if matches!(top, F_TERNARY | F_CASE | F_FIELD) {
            self.unwind(top, b';');
            if self.top() == F_FIELD {
                self.pop();
            }
        }
        self.prev = if self.top() == F_CLASS_BODY {
            P_MEMBER
        } else {
            P_START
        };
        self.tok(p, p + 1, TERMINATOR)
    }

    /// A `:` at `p` in frame `top`.
    #[inline(always)]
    fn colon(&mut self, p: usize, top: u8) -> usize {
        if top == F_PARAMS || self.prev == P_PARAMS_END {
            return self.annotation(p);
        }
        match top {
            F_TERNARY => {
                self.pop();
                self.prev = P_OP;
                self.tok(p, p + 1, TERNARY)
            }
            F_CASE => {
                self.pop();
                self.prev = P_START;
                self.tok(p, p + 1, KEY_SEP)
            }
            _ => {
                self.prev = P_OP;
                self.tok(p, p + 1, KEY_SEP)
            }
        }
    }

    /// A `?` at `p`: optional chaining's `?.`, `??`, or a ternary's.
    #[inline(always)]
    fn question(&mut self, p: usize) -> usize {
        let b = self.b;
        if self.top() == F_PARAMS && b.get(skip_blanks(b, p + 1)) == Some(&b':') {
            // a parameter's `?:`
            return self.tok(p, p + 1, FLOW_OPTIONAL);
        }
        match b.get(p + 1) {
            Some(b'.') if !matches!(b.get(p + 2), Some(b'0'..=b'9')) => {
                self.tok(p, p + 2, ACCESSOR);
                self.prev = P_DOT;
                p + 2
            }
            Some(b'?') => {
                let e = if b.get(p + 2) == Some(&b'=') {
                    p + 3
                } else {
                    p + 2
                };
                self.prev = P_OP;
                self.tok(p, e, LOGICAL)
            }
            _ => {
                self.push(F_TERNARY);
                self.prev = P_OP;
                self.tok(p, p + 1, TERNARY)
            }
        }
    }

    /// A `=` at `p`: `=>`, a comparison, or an assignment.
    #[inline(always)]
    fn equals(&mut self, p: usize) -> usize {
        let b = self.b;
        let (e, leaf, prev) = match b.get(p + 1) {
            Some(b'>') => (p + 2, ARROW_KW, P_ARROW),
            Some(b'=') if b.get(p + 2) == Some(&b'=') => (p + 3, COMPARISON, P_OP),
            Some(b'=') => (p + 2, COMPARISON, P_OP),
            _ => {
                if self.top() == F_PARAMS || self.top() == F_PATTERN {
                    self.tok(p, p + 1, ASSIGN);
                    self.push(F_DEFAULT);
                    self.prev = P_OP;
                    return p + 1;
                }
                (p + 1, ASSIGN, P_OP)
            }
        };
        self.prev = prev;
        self.tok(p, e, leaf)
    }

    /// `literal-operators`' punctuation at `p` (`c`), as one `match` on the bytes there.
    #[inline(always)]
    fn op(&mut self, p: usize, c: u8) -> usize {
        let b = self.b;
        let at1 = b.get(p + 1).copied();
        let at2 = b.get(p + 2).copied();
        let at3 = b.get(p + 3).copied();
        let (len, leaf) = match c {
            b'!' if at1 != Some(b'=') => (1, LOGICAL),
            b'!' if at2 == Some(b'=') => (3, COMPARISON),
            b'!' => (2, COMPARISON),
            b'&' | b'|' if at1 == Some(c) && at2 == Some(b'=') => (3, AUGMENTED),
            b'&' | b'|' if at1 == Some(c) => (2, LOGICAL),
            b'%' | b'&' | b'*' | b'/' | b'^' | b'|' if at1 == Some(b'=') => (2, AUGMENTED),
            b'>' if at1 == Some(b'>') && at2 == Some(b'=') => (3, AUGMENTED),
            b'>' if at1 == Some(b'>') && at2 == Some(b'>') && at3 == Some(b'=') => (4, AUGMENTED),
            b'~' => (1, BITWISE),
            b'>' if at1 == Some(b'>') && at2 == Some(b'>') => (3, BITWISE),
            b'>' if at1 == Some(b'>') => (2, BITWISE),
            b'&' | b'^' | b'|' => (1, BITWISE),
            b'>' if at1 == Some(b'=') => (2, RELATIONAL),
            b'>' => (1, RELATIONAL),
            b'*' if at1 == Some(b'*') && at2 == Some(b'=') => (3, AUGMENTED),
            b'*' if at1 == Some(b'*') => (2, ARITHMETIC),
            b'/' | b'%' | b'*' => (1, ARITHMETIC),
            _ => (1, OP),
        };
        self.prev = P_OP;
        self.tok(p, p + len, leaf)
    }

    /// A `*` at `p`: a generator's (after `function`, `yield`, or at a member's or key's
    /// place), else an operator.
    #[inline(always)]
    fn star(&mut self, p: usize, top: u8) -> usize {
        if top == F_FUNC_HEAD
            || top == F_CLASS_BODY
            || (self.prev == P_KEY && top == F_OBJECT)
            || (self.prev == P_OP && p > 0 && self.after_yield(p))
        {
            return self.tok(p, p + 1, ASTERISK);
        }
        self.op(p, b'*')
    }

    /// Whether a `*` is before `p`, past blanks (`import * as x`).
    fn after_star(&self, p: usize) -> bool {
        let b = self.b;
        let mut k = p;
        while k > 0 && matches!(b[k - 1], b' ' | b'\t') {
            k -= 1;
        }
        k > 0 && b[k - 1] == b'*'
    }

    #[inline(never)]
    fn after_yield(&self, p: usize) -> bool {
        let b = self.b;
        let mut k = p;
        while k > 0 && matches!(b[k - 1], b' ' | b'\t') {
            k -= 1;
        }
        k >= 5 && &b[k - 5..k] == b"yield"
    }

    /// A `+` or `-` at `p`: a signed number's sign (`\B[-+]` before a digit), `++`, `--`, an
    /// augmented assignment, or arithmetic.
    #[inline(always)]
    fn sign(&mut self, p: usize, c: u8) -> usize {
        let b = self.b;
        match b.get(p + 1) {
            Some(&d) if d == c => {
                if c == b'-'
                    && b.get(p + 2) == Some(&b'>')
                    && b[..p].iter().all(|&x| matches!(x, b' ' | b'\t'))
                {
                    // `-->` at a line's start: an HTML-like comment's close (`x-->0` elsewhere
                    // is `--` and `>`)
                    self.prev = P_START;
                    return self.tok_far(p, p + 3, HTML_COMMENT);
                }
                // `++`, `--`: the value stays one
                let prev = if self.prev == P_VALUE { P_VALUE } else { P_OP };
                self.tok(p, p + 2, ARITHMETIC);
                self.prev = prev;
                return p + 2;
            }
            Some(b'=') => {
                self.prev = P_OP;
                return self.tok(p, p + 2, AUGMENTED);
            }
            Some(b'0'..=b'9' | b'.' | b'I') if p == 0 || !NAME[usize::from(b[p - 1])] => {
                if b[p + 1] == b'I' {
                    if has(b, p + 1, b"Infinity")
                        && !b.get(p + 9).is_some_and(|&c| NAME[usize::from(c)])
                    {
                        self.prev = P_VALUE;
                        return self.tok(p, p + 9, INFINITY);
                    }
                } else if b[p + 1] != b'.' || matches!(b.get(p + 2), Some(b'0'..=b'9')) {
                    return self.number(p);
                }
            }
            _ => {}
        }
        self.prev = P_OP;
        self.tok(p, p + 1, ARITHMETIC)
    }

    /// A `<` at `p`: JSX's tag where an expression may start, else an operator.
    #[inline(always)]
    fn less(&mut self, p: usize) -> usize {
        let b = self.b;
        if matches!(self.top(), F_FUNC_HEAD | F_CLASS_HEAD) {
            return self.type_open(p, F_TYPE_ANGLE);
        }
        if matches!(self.prev, P_START | P_OP | P_ARROW | P_KEY)
            && matches!(
                b.get(p + 1),
                Some(b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' | b'>')
            )
        {
            return self.jsx_open(p);
        }
        if has(b, p, b"<!--") {
            self.tok_far(p, p + 4, HTML_COMMENT);
            return p + 4;
        }
        let at1 = b.get(p + 1).copied();
        let (len, leaf) = match at1 {
            Some(b'<') if b.get(p + 2) == Some(&b'=') => (3, AUGMENTED),
            Some(b'<') => (2, BITWISE),
            Some(b'=') => (2, RELATIONAL),
            _ => (1, RELATIONAL),
        };
        self.prev = P_OP;
        self.tok(p, p + len, leaf)
    }

    /// A `/` at `p`: a comment's, a regex literal's, or division.
    #[inline(always)]
    fn slash(&mut self, p: usize) -> usize {
        let b = self.b;
        match b.get(p + 1) {
            Some(b'/') => return self.line_comment(p),
            Some(b'*') => return self.block_comment(p),
            _ => {}
        }
        if matches!(self.prev, P_START | P_OP | P_ARROW | P_KEY) && p + 1 < b.len() {
            // Babel's regex takes the blanks before its `/`
            let mut s = p;
            while s > 0 && matches!(b[s - 1], b' ' | b'\t') {
                s -= 1;
            }
            if s < p {
                self.tok_far(s, p, RE);
            }
            return self.regex_literal(p);
        }
        self.prev = P_OP;
        if b.get(p + 1) == Some(&b'=') {
            return self.tok(p, p + 2, AUGMENTED);
        }
        self.tok(p, p + 1, ARITHMETIC)
    }

    /// `@` at `p`: a decorator's.
    #[inline(never)]
    fn decorator(&mut self, p: usize) -> usize {
        let b = self.b;
        let mut e = p + 1;
        while e < b.len() && NAME[usize::from(b[e])] {
            e += 1;
        }
        if e == p + 1 {
            return p + 1;
        }
        self.tok(p, p + 1, DEC_MARK);
        self.tok(p + 1, e, DEC_NAME);
        self.prev = P_VALUE;
        e
    }

    /// A number at `p` (a digit, a `.` before a digit, or a sign before one): a hex, octal or
    /// binary literal, else digits, a fraction, an exponent; `_` separators and `n` too.
    #[inline(always)]
    fn number(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let digits = |mut e: usize, ok: fn(u8) -> bool| {
            while e < n && (ok(b[e]) || b[e] == b'_') {
                e += 1;
            }
            e
        };
        let mut e = p;
        if matches!(b[e], b'-' | b'+') {
            e += 1;
        }
        if b.get(e) == Some(&b'.') {
            e = digits(e + 1, |c| c.is_ascii_digit());
            self.prev = P_VALUE;
            return self.tok(p, e, NUMBER);
        }
        if b.get(e) == Some(&b'0')
            && matches!(b.get(e + 1), Some(b'x' | b'X' | b'b' | b'B' | b'o' | b'O'))
        {
            e = digits(e + 2, |c| c.is_ascii_hexdigit());
        } else {
            e = digits(e, |c| c.is_ascii_digit());
            if b.get(e) == Some(&b'.') && b.get(e + 1) != Some(&b'.') {
                e = digits(e + 1, |c| c.is_ascii_digit());
            }
            if matches!(b.get(e), Some(b'e' | b'E')) {
                let s = if matches!(b.get(e + 1), Some(b'-' | b'+')) {
                    e + 2
                } else {
                    e + 1
                };
                if matches!(b.get(s), Some(b'0'..=b'9')) {
                    e = digits(s, |c| c.is_ascii_digit());
                }
            }
        }
        if b.get(e) == Some(&b'n') {
            e += 1;
        }
        self.prev = if self.prev == P_KEY { P_KEYED } else { P_VALUE };
        self.tok(p, e, NUMBER)
    }
}

impl<K: Sink> Scan<'_, '_, K> {
    // --- Flow ---

    /// An annotation's `:` at `p`: the type after it.
    #[inline(never)]
    fn annotation(&mut self, p: usize) -> usize {
        self.tok(p, p + 1, FLOW_ANNOTATION);
        self.push(F_TYPE);
        self.prev = P_OP;
        p + 1
    }

    /// A type's opener at `p`, its frame `frame`.
    #[inline(never)]
    fn type_open(&mut self, p: usize, frame: u8) -> usize {
        self.tok(p, p + 1, FLOW_BEGIN);
        let prev = self.prev;
        self.push(frame);
        // a method's or a function's type parameters keep what follows them
        self.prev = if prev == P_KEYED { P_KEYED } else { P_OP };
        p + 1
    }

    /// `type` or `declare` at `p..e` (`id`): Flow's where a statement starts with it (or after
    /// `import` or `export`) and a name or `{` follows, else `None`.
    #[inline(never)]
    fn flow_word(&mut self, p: usize, e: usize, id: u16) -> Option<usize> {
        let b = self.b;
        let s = skip_blanks(b, e);
        let next = b.get(s).copied()?;
        let named = s > e && (next.is_ascii_alphabetic() || next == b'_' || next == b'$');
        if id == ID_TYPE {
            let module = self.prev == P_OP && {
                let mut k = p;
                while k > 0 && matches!(b[k - 1], b' ' | b'\t') {
                    k -= 1;
                }
                k >= 6 && matches!(&b[k - 6..k], b"import" | b"export")
            };
            if !(self.prev == P_START || module) || !(named || (module && next == b'{')) {
                return None;
            }
            self.tok(p, e, TYPE_KW);
            self.push(F_TYPE_DECL);
            self.prev = P_OP;
            return Some(e);
        }
        if self.prev != P_START || !named {
            return None;
        }
        self.tok(p, e, DECLARE);
        Some(e)
    }

    /// A type frame's turn at `p` (`top`): a type's token, or the frame left at a byte no type
    /// takes there.
    #[inline(never)]
    fn type_turn(&mut self, p: usize, top: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let p = skip_blanks(b, p);
        if p >= n {
            return n;
        }
        let c = b[p];
        let leave = |s: &mut Self| {
            s.pop();
            p
        };
        match c {
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' => {
                let mut e = p + 1;
                while e < n && NAME[usize::from(b[e])] {
                    e += 1;
                }
                let w = &b[p..e];
                if top == F_TYPE_DECL && w == b"from" {
                    return leave(self);
                }
                let leaf = match w {
                    b"any" | b"bool" | b"boolean" | b"mixed" | b"number" | b"string" | b"void" => {
                        FLOW_PRIMITIVE
                    }
                    b"typeof" => FLOW_OTHER,
                    _ => {
                        let (class, leaf, _) = INFO[usize::from(WORDS.get(w))];
                        if class == W_SUPPORT
                            && matches!(leaf, BUILTIN_CLASS | ERROR_CLASS | NODE_CLASS)
                        {
                            leaf
                        } else {
                            FLOW_VAR
                        }
                    }
                };
                self.prev = P_VALUE;
                self.tok(p, e, leaf)
            }
            b'|' | b'&' | b'*' | b'.' | b'?' | b':' => {
                self.prev = P_OP;
                self.tok(p, p + 1, FLOW_OTHER)
            }
            b'<' => self.type_open(p, F_TYPE_ANGLE),
            b'[' => self.type_open(p, F_TYPE_SQUARE),
            b'(' => self.type_open(p, F_TYPE_ROUND),
            b'{' => {
                if top == F_TYPE && self.prev == P_VALUE {
                    // a return type's end: the body's `{`
                    return leave(self);
                }
                self.type_open(p, F_TYPE_CURLY)
            }
            b'>' | b']' | b')' | b'}' => {
                let closes = match c {
                    b'>' => F_TYPE_ANGLE,
                    b']' => F_TYPE_SQUARE,
                    b')' => F_TYPE_ROUND,
                    _ => F_TYPE_CURLY,
                };
                if top != closes {
                    return leave(self);
                }
                self.tok(p, p + 1, FLOW_END);
                self.pop();
                // after a method's or a function's type parameters, its `(`
                self.prev = if c == b'>'
                    && (matches!(self.top(), F_CLASS_BODY | F_FUNC_HEAD | F_CLASS_HEAD)
                        || self.prev == P_KEYED)
                {
                    P_KEYED
                } else {
                    P_VALUE
                };
                p + 1
            }
            b'=' if b.get(p + 1) == Some(&b'>') => {
                self.prev = P_OP;
                self.tok(p, p + 2, FLOW_OP)
            }
            b'=' if top == F_TYPE_DECL => {
                self.prev = P_OP;
                p + 1
            }
            b'"' | b'\'' => self.string(p, c),
            b'0'..=b'9' => self.number(p),
            b'/' if b.get(p + 1) == Some(&b'/') => self.line_comment(p),
            b'/' if b.get(p + 1) == Some(&b'*') => self.block_comment(p),
            b',' if top != F_TYPE && top != F_TYPE_DECL => p + 1,
            _ => leave(self),
        }
    }
}

/// Whether `a` is a name to look up only as itself: `[A-Z_$][A-Z0-9_$]*` past its first.
#[inline(always)]
fn is_upper(c: u8) -> bool {
    c.is_ascii_uppercase()
}

/// Whether name `w` is a constant's: no lowercase letter, a capital among its first bytes.
#[inline(always)]
fn upper_only(w: &[u8]) -> bool {
    w.iter().all(|&c| !c.is_ascii_lowercase())
}

/// A shorthand key's kind (`{ a, setTimeout }`): a support word's, else a variable's.
#[inline(never)]
fn shorthand_kind(b: &[u8], p: usize, e: usize) -> u16 {
    let (class, leaf, _) = INFO[usize::from(WORDS.get(&b[p..e]))];
    if matches!(class, W_SUPPORT | W_SUPPORT_OBJ) {
        leaf
    } else {
        var_kind(b, p, e)
    }
}

/// A plain name's kind from its bytes: a constant's capitals, a class's capital, else a
/// variable.
#[inline(always)]
fn var_kind(b: &[u8], p: usize, e: usize) -> u16 {
    let mut u = p;
    while u < e && b[u] == b'_' {
        u += 1;
    }
    if u < e && b[u].is_ascii_uppercase() {
        if upper_only(&b[u..e]) && e - u > 1 {
            return CONSTANT;
        }
        return CLASS_VAR;
    }
    READWRITE
}

#[inline(always)]
fn skip_blanks(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && matches!(b[p], b' ' | b'\t') {
        p += 1;
    }
    p
}

// The entry points, as [`super`]'s.

/// Lex `b`, a line of JavaScript without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let mut s = Scan::fresh(b, 0, out, Spans::new());
    s.load(state);
    s.k.open(s.out, b.len() / 4 + 16);
    s.lex(0);
    s.k.close(s.out);
    *state = s.keep();
}

/// Lex `b`, a line holding JavaScript from `from` (HTML's `<script>` and `on…="…"`), from
/// `state`.
#[inline(never)]
pub(crate) fn line_from(b: &[u8], from: usize, state: &mut State, out: &mut Out) {
    let mut s = Scan::fresh(b, from, out, Push);
    s.load(state);
    s.lex(from);
    *state = s.keep();
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
    let mut s = Scan::fresh(b, 0, out, Runs::new(rules, runs));
    s.load(state);
    s.lex(0);
    *state = s.keep();
    s.k.end_line(b.len());
}

/// A block's lines on one scan opened from `state` ([`crate::Carry`]).
pub fn block(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    let mut out = Out::new(spans);
    out.0.clear();
    let mut s = Scan::fresh(b"", 0, &mut out, Spans::new());
    s.load(state);
    s.k.open(s.out, 64);
    let rest = pass(&mut s);
    *state = s.keep_block();
    rest
}

/// [`block`]'s runs by theme rule pushed to `runs`, the lines' loop compiled with the scan.
pub fn block_runs(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    lines: &mut crate::RunLines,
) -> Option<usize> {
    let mut out = Out::new(spans);
    out.0.clear();
    let mut s = Scan::fresh(b"", 0, &mut out, Runs::new(rules, runs));
    s.load(state);
    let rest = lines.pass(&mut s);
    *state = s.keep_block();
    rest
}

impl<K: Sink> Scan<'_, '_, K> {
    /// The state a block's pass leaves: as the last line left it (a block that lexed no line
    /// leaves the state it was given).
    fn keep_block(&self) -> State {
        let arg = u16::from(self.prev) | if self.later { ARG_LATER } else { 0 };
        State::keep(&self.stack, self.depth, arg)
    }
}

impl crate::Carry for Scan<'_, '_, Spans> {
    fn next(&mut self, b: &[u8]) -> &[crate::Span] {
        self.k.start_line();
        // SAFETY: the line is set back to an empty one before `b`'s borrow ends
        self.carry(unsafe { crate::line_for(b) });
        self.k.close(self.out);
        self.out.0
    }
}

impl crate::CarryRuns for Scan<'_, '_, Runs<'_>> {
    fn next(&mut self, b: &[u8]) -> usize {
        self.k.start_line();
        // SAFETY: as above
        self.carry(unsafe { crate::line_for(b) });
        self.k.end_line(b.len())
    }
}

impl<'b, K: Sink> Scan<'b, '_, K> {
    /// `b`, the block's next line, lexed on this scan; the scan's line an empty one again after.
    #[inline(always)]
    fn carry(&mut self, b: &'b [u8]) {
        self.b = b;
        self.at = 0;
        self.lex(0);
        // the line's end as a state's save and load leave it
        crate::stack::hold(&mut self.stack, &mut self.depth);
        self.b = b"";
    }
}

#[cfg(test)]
mod tests {
    use crate::Lang;
    use crate::tests::marked;

    fn js(lines: &[&'static str]) -> Vec<(&'static str, &'static str)> {
        marked(Lang::JavaScript, lines)
    }

    fn has(m: &[(&str, &str)], text: &str, scopes: &str) {
        assert!(m.contains(&(text, scopes)), "{text:?} {scopes:?} in {m:?}");
    }

    #[test]
    fn a_function_and_its_parts() {
        let m = js(&["function foo(a, b = 1) { return a / b; }"]);
        has(&m, "function", "storage.type.function.js");
        has(&m, "foo", "entity.name.function.js");
        has(&m, "a", "variable.parameter.function.js");
        has(&m, "1", "constant.numeric.js");
        has(&m, "return", "keyword.control.flow.js");
        // after a value, a `/` is division
        has(&m, "/", "keyword.operator.arithmetic.js");
    }

    /// An arrow's parameters are read as a group's until its `=>`, then again as parameters;
    /// a name its value is assigned to, or a key's, is the function's name.
    #[test]
    fn arrows_are_decided_at_their_arrow() {
        let m = js(&[
            "const f = (a, { b }, c = g(1)) => a;",
            "x = async (y) => y; o = { k: (z) => z, 'q': function () {} };",
            "h(n => n);",
            "(p) + 1;",
        ]);
        has(&m, "f", "entity.name.function.js");
        has(&m, "a", "variable.parameter.function.js");
        has(&m, "b", "variable.parameter.function.js");
        has(&m, "g", "variable.function.js");
        has(&m, "=>", "storage.type.function.arrow.js");
        has(&m, "x", "entity.name.function.js");
        has(&m, "async", "storage.type.js");
        has(&m, "k", "string.unquoted.js entity.name.function.js");
        // a quoted key's quotes stay the string's
        has(&m, "q", "string.quoted.js entity.name.function.js");
        has(
            &m,
            "'",
            "string.quoted.js punctuation.definition.string.begin.js",
        );
        has(&m, "n", "variable.parameter.function.js");
        // a group no `=>` follows stays one
        has(&m, "p", "variable.other.readwrite.js");
    }

    /// `Foo.bar = function` and `Foo.bar = (…) =>` name `Foo` a class and `bar` a function;
    /// a value that isn't a function leaves `Foo` a class's variable.
    #[test]
    fn a_capitalized_names_member_assigned_a_function() {
        let m = js(&[
            "Foo.bar = function () {};",
            "Baz.qux = (a) => a;",
            "Kept.x = 1;",
        ]);
        has(&m, "Foo", "entity.name.class.js");
        has(&m, "bar", "entity.name.function.js");
        has(&m, "Baz", "entity.name.class.js");
        has(&m, "qux", "entity.name.function.js");
        has(&m, "Kept", "variable.other.class.js");
        has(&m, "x", "variable.other.property.js");
        // a `Foo.` on the line before, its `.` where this line's is, names nothing here
        let m = js(&[
            "Foo.y;",
            "abc.d = function () {};",
            "",
            ".e = function () {};",
        ]);
        has(&m, "abc", "variable.other.object.js");
        has(&m, "d", "entity.name.function.js");
        has(&m, "e", "entity.name.function.js");
    }

    /// `<!--` is an HTML-like comment's open anywhere, `-->` its close only at a line's start:
    /// elsewhere it's `--` and `>` (`x-->0`).
    #[test]
    fn html_like_comments() {
        let m = js(&["<!-- legacy", "  -->", "while (x-->0) {}"]);
        const H: &str = "comment.block.html.js punctuation.definition.comment.js";
        has(&m, "<!--", H);
        has(&m, "-->", H);
        has(&m, "--", "keyword.operator.arithmetic.js");
        has(&m, ">", "keyword.operator.relational.js");
    }

    /// After `new`, a path's last name is the constructor's, a support word's too; the path
    /// before it plain.
    #[test]
    fn a_constructor_after_new() {
        let m = js(&["a = new Foo(1);", "b = new errs.inner.TypeError('m');"]);
        has(&m, "Foo", "variable.function.constructor.js");
        has(&m, "TypeError", "variable.function.constructor.js");
    }

    /// Methods are told by their frame: a class's body, an object's key place.
    #[test]
    fn methods_keys_and_accessors() {
        let m = js(&[
            "class A extends B {",
            "  static get size() { return 1; }",
            "  [Symbol.iterator]() {}",
            "  run(x) { return { key: x, other() {}, get v() {} }; }",
            "}",
        ]);
        has(&m, "class", "storage.type.class.js");
        has(&m, "A", "entity.name.class.js");
        has(&m, "extends", "storage.type.extends.js");
        has(&m, "static", "storage.modifier.js");
        has(&m, "get", "storage.type.accessor.js");
        has(&m, "size", "entity.name.function.js");
        has(&m, "run", "entity.name.function.js");
        has(&m, "x", "variable.parameter.function.js");
        has(
            &m,
            "key",
            "constant.other.object.key.js string.unquoted.label.js",
        );
        has(&m, "other", "entity.name.function.js");
        // an object's accessor, Babel's own color
        has(&m, "v", "entity.name.accessor.js");
        // a quoted method's name, in an object and a class body
        let q = js(&[
            "o = { \"Arrow[p]\"(node) {}, 'q'(x) {} };",
            "class C { \"m\"(z) {} }",
        ]);
        has(&q, "Arrow[p]", "string.quoted.js entity.name.function.js");
        has(&q, "q", "string.quoted.js entity.name.function.js");
        has(&q, "m", "string.quoted.js entity.name.function.js");
        has(&q, "node", "variable.parameter.function.js");
        has(&q, "z", "variable.parameter.function.js");
        // a computed method's parameters open after its `]`
        assert!(
            m.iter()
                .filter(|(t, s)| *t == "(" && s.contains("parameters"))
                .count()
                >= 4,
            "{m:?}"
        );
    }

    /// A ternary's `:` is the ternary's, in an object's value too; a label's and a key's are
    /// their own.
    #[test]
    fn colons() {
        let m = js(&["o = { a: c ? d : e }; top: for (;;) {} switch (x) { case 1: break; }"]);
        let colons: Vec<_> = m
            .iter()
            .filter(|(t, _)| *t == ":")
            .map(|(_, s)| *s)
            .collect();
        assert_eq!(
            colons,
            [
                "constant.other.object.key.js punctuation.separator.key-value.js",
                "keyword.operator.ternary.js",
                "constant.other.object.key.js punctuation.separator.key-value.js",
                "punctuation.separator.key-value.js",
            ],
            "{m:?}"
        );
    }

    /// The previous token tells a regex from division, an object from a block, JSX from `<`.
    #[test]
    fn the_previous_token_decides() {
        let m = js(&[
            "a = b / c / d; e = /re/g.test(f); if (g) /x/;",
            "return { k: 1 }; } catch { h(); }",
            "i = j < k; l = <div>{m}</div>;",
        ]);
        // after `)`, `/x/` is division as in Babel's grammar
        assert_eq!(
            m.iter()
                .filter(|(_, s)| *s == "keyword.operator.arithmetic.js")
                .count(),
            4,
            "{m:?}"
        );
        has(&m, "re", "string.regexp.js");
        has(&m, "g", "string.regexp.js keyword.other.js");
        has(
            &m,
            "k",
            "constant.other.object.key.js string.unquoted.label.js",
        );
        // `catch {`'s block: a call in it is a call
        has(&m, "h", "variable.function.js");
        has(&m, "<", "keyword.operator.relational.js");
        has(&m, "div", "meta.tag.jsx entity.name.tag.jsx");
        has(&m, "m", "variable.other.readwrite.js");
    }

    #[test]
    fn jsx_closes_across_lines() {
        let m = js(&[
            "const a = (",
            "  <div className=\"x\" on={() => 1}>",
            "    text &amp; {b}",
            "    <>",
            "      <br />",
            "    </>",
            "  </div>",
            ");",
        ]);
        has(&m, "div", "meta.tag.jsx entity.name.tag.jsx");
        has(&m, "className", "entity.other.attribute-name.jsx");
        has(&m, "x", "string.quoted.jsx");
        has(&m, "&amp;", "constant.character.entity.jsx");
        has(&m, "b", "variable.other.readwrite.js");
        has(&m, "br", "meta.tag.jsx entity.name.tag.jsx");
        has(&m, ";", "punctuation.terminator.statement.js");
    }

    /// Strings, templates and comments across lines, as frames of their own.
    #[test]
    fn text_across_lines() {
        let m = js(&[
            "s = 'a\\",
            "b'; t = `x ${y + `z`}",
            "w`; /* c",
            "d */ /** doc */ u; // line",
        ]);
        has(
            &m,
            "\\",
            "string.quoted.js constant.character.escape.newline.js",
        );
        has(&m, "b", "string.quoted.js");
        has(&m, "${", "keyword.other.substitution.begin.js");
        has(&m, "y", "variable.other.readwrite.js");
        has(&m, "z", "string.interpolated.js");
        has(&m, "w", "string.interpolated.js");
        has(&m, "d ", "comment.block.js");
        has(&m, " doc ", "comment.block.documentation.js");
        has(&m, " line", "comment.line.double-slash.js");
        has(&m, "u", "variable.other.readwrite.js");
    }

    #[test]
    fn flow_annotations() {
        let m = js(&[
            "import type {Node} from 'x';",
            "type T = ?string | Array<number>;",
            "function f<P>(a: P, b?: {k: boolean}): void {}",
            "class C { p: mixed = 1; }",
        ]);
        has(&m, "type", "support.type.type.flowtype.js");
        has(&m, "Node", "variable.other.flowtype.js");
        has(&m, "from", "keyword.operator.module.js");
        has(&m, "string", "constant.other.primitve.flowtype.js");
        has(&m, "|", "keyword.operator.flowtype.other.js");
        has(&m, "Array", "support.class.builtin.js");
        has(&m, "<", "punctuation.section.flowtype.begin.js");
        has(&m, ":", "keyword.operator.flowtype.annotation.js");
        has(&m, "?", "keyword.operator.flowtype.optional.js");
        has(&m, "void", "constant.other.primitve.flowtype.js");
        has(&m, "p", "variable.other.property.js");
        has(&m, "mixed", "constant.other.primitve.flowtype.js");
        has(&m, "{", "meta.brace.curly.begin.js");
    }

    /// A regex literal ends with its line (JavaScript's), a group open at its end included.
    #[test]
    fn a_regex_literal_ends_with_its_line() {
        let m = js(&["x = /(a/;", "b)/g; y"]);
        has(&m, "b", "variable.other.readwrite.js");
        has(&m, "y", "variable.other.readwrite.js");
    }

    #[test]
    fn a_regex_literals_body_is_ecmascripts_regex() {
        let has = |m: &[(&str, &str)], text: &str, scopes: &str| {
            assert!(m.contains(&(text, scopes)), "{text:?} {scopes:?} in {m:?}");
        };
        // a `/` in a class, escaped or in a group doesn't end it
        let m = js(&[r"x = /^a[/.]b\/(?<n>c/d)\k<n>{2,}?\u{1F600}\p{L}\A/gsu;"]);
        has(
            &m,
            "/",
            "string.regexp.js punctuation.definition.string.begin.js",
        );
        has(&m, "^", "string.regexp.js keyword.control.anchor.regexp");
        has(
            &m,
            "[",
            "string.regexp.js constant.other.character-class.set.regexp punctuation.definition.character-class.begin.regexp",
        );
        // a `.` in a class is a character, the class's text as the `/` is
        has(
            &m,
            "/.",
            "string.regexp.js constant.other.character-class.set.regexp",
        );
        has(
            &m,
            r"\/",
            "string.regexp.js constant.character.escape.backslash.regexp",
        );
        has(&m, "n", "string.regexp.js entity.name.other.group.regexp");
        has(&m, "n", "string.regexp.js variable.other.group.regexp");
        has(
            &m,
            "2",
            "string.regexp.js keyword.operator.quantifier.regexp constant.literal.numeric.regexp",
        );
        has(
            &m,
            r"\u{1F600}",
            "string.regexp.js constant.character.escape.backslash.regexp",
        );
        has(
            &m,
            "L",
            "string.regexp.js support.constant.unicode-property.regexp",
        );
        // not Java's anchor
        has(
            &m,
            r"\A",
            "string.regexp.js constant.character.escape.backslash.regexp",
        );
        has(&m, "gsu", "string.regexp.js keyword.other.js");
        has(&m, ";", "punctuation.terminator.statement.js");
        // Java's constructs are none of ECMAScript's
        let m = js(&[r"x = /(?i)a++[a-z&&[^q]]/;"]);
        assert!(!m.iter().any(|(t, _)| *t == "?i" || *t == "&&"), "{m:?}");
    }

    /// `RegExp(…)`'s first argument, a string or a template, is a regex (as WebStorm colors it):
    /// the string's escapes, end and `${…}` come first, and a group open over a `${…}` or a
    /// line's end is kept.
    #[test]
    fn regexes_in_regexp_strings() {
        let has = |m: &[(&str, &str)], text: &str, scopes: &str| {
            assert!(m.contains(&(text, scopes)), "{text:?} {scopes:?} in {m:?}");
        };
        const Q: &str = "string.quoted.js";
        const T: &str = "string.interpolated.js";
        let m = js(&[
            r#"r = new RegExp("^(\\d+)\"x\d$", 'g'), s = RegExp('a\'(');"#,
            r"t = new RegExp(`^(${p}|x)[0-9]\${q}`);",
            "u = foo(\"(a)\"), v = RegExp(`(a",
            "b)c`), w = RegExp(\"[\\",
            "]\");",
        ]);
        has(&m, "^", &format!("{Q} keyword.control.anchor.regexp"));
        has(
            &m,
            r"\\d",
            &format!("{Q} constant.other.character-class.escape.backslash.regexp"),
        );
        has(&m, "\\\"", &format!("{Q} constant.character.escape.js"));
        // `\d` is the string's `d`, no escape of Babel's
        has(&m, "x\\d", Q);
        has(
            &m,
            "(",
            &format!("{Q} punctuation.definition.group.begin.regexp"),
        );
        has(&m, "g", Q);
        has(&m, "|", &format!("{T} keyword.operator.or.regexp"));
        // `\${` is a `\` and a substitution to Babel: the regex's `\` before it
        has(
            &m,
            "\\",
            &format!("{T} constant.character.escape.backslash.regexp"),
        );
        has(&m, "${", "keyword.other.substitution.begin.js");
        has(&m, "(a)", Q);
        has(
            &m,
            ")",
            &format!("{T} punctuation.definition.group.end.regexp"),
        );
        // a `\` ending the line is the string's continuation, not the regex's
        has(
            &m,
            "\\",
            &format!("{Q} constant.character.escape.newline.js"),
        );
        has(
            &m,
            "]",
            &format!(
                "{Q} constant.other.character-class.set.regexp punctuation.definition.character-class.end.regexp"
            ),
        );
    }
}
