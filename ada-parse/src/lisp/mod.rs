//! Lisp by a scanner of ada's own (ADR-40, speed first; X-006): bat's `Lisp` grammar, which
//! names Common Lisp, Emacs Lisp and Scheme files alike, read token by token.
//!
//! bat's grammar backtracks exponentially on a run of backslashes (G-001: its identifier's
//! `(?:\\.|[^{{break_char}}])*` splits `\\\\…` every way before failing). Here an identifier is
//! read once, linearly: a run of escapes (`\` and any character) and characters that are not
//! a break (`[\s()"',:;|]`), with one character that is neither a digit nor a `.` (or an
//! escape), or `|…|`. That is bat's identifier as G-001's fix reads it, with `\` no longer a
//! character of its own, and a word of a list matches only the whole token, escapes included
//! (`car\x` is a symbol of its own, not `car`): fixes over bat by design.
//!
//! The grammar's contexts become a stack of frames of this scanner's design: one for each open
//! list, saying how its elements read (a body, a list of lists, a quoted list, a loop's body, a
//! parameter list, …), and above it the one-shot expectations a declaration sets up (the
//! function's name, then its parameter list), each a frame that pops at the first token it
//! doesn't take. A list's head (`paren-begin`) is one lookup of its token in one table of every
//! list's words ([`words`]). Tokens are classified once, at the place the scan has reached; where
//! nothing can start, the scan steps a character, as syntect's leftmost search does.
//!
//! A kind is a leaf, bat's scopes for the token (its pattern's and its captures'), with the
//! string or comment around it; no theme keys on the meta scopes (`meta.parens`,
//! `meta.function`, …), so they are left out. The state between lines is the stack, a byte a
//! frame, runs of plain list bodies as a count.
use crate::prim::{char_len, digit, is_nd, wide_char};

pub(crate) mod clojure;
mod words;

use crate::{Out, State};
use words::*;

macro_rules! leaves {
    ($($name:ident = $scope:literal,)*) => {
        #[allow(dead_code)]
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        #[repr(u16)]
        enum L { $($name,)* }
        static LEAVES: &[&str] = &[$($scope,)*];
    };
}

// An identifier's kind is followed by its `|…|` delimiters' (`Begin`, `End`).
leaves! {
    Plain = "",
    LineCommentPunct = "comment.line.semicolon.lisp punctuation.definition.comment.lisp",
    LineComment = "comment.line.semicolon.lisp",
    BlockBegin = "comment.block.lisp punctuation.definition.comment.begin.lisp",
    BlockEnd = "comment.block.lisp punctuation.definition.comment.end.lisp",
    Block = "comment.block.lisp",
    StrayCommentEnd = "invalid.illegal.stray-comment-end.lisp",
    StrayParen = "invalid.illegal.stray-bracket-end.lisp",
    ParenBegin = "punctuation.section.parens.begin.lisp",
    ParenEnd = "punctuation.section.parens.end.lisp",
    ParamsBegin = "punctuation.section.parameters.begin.lisp",
    ParamsEnd = "punctuation.section.parameters.end.lisp",
    StrBegin = "string.quoted.double.lisp punctuation.definition.string.begin.lisp",
    StrEnd = "string.quoted.double.lisp punctuation.definition.string.end.lisp",
    Str = "string.quoted.double.lisp",
    StrEscape = "string.quoted.double.lisp constant.character.escape.lisp",
    FmtEscapePunct = "string.quoted.double.lisp constant.character.escape.lisp punctuation.definition.placeholder.lisp",
    Placeholder = "string.quoted.double.lisp constant.other.placeholder.lisp",
    PlaceholderPunct = "string.quoted.double.lisp constant.other.placeholder.lisp punctuation.definition.placeholder.lisp",
    PlaceholderBase = "string.quoted.double.lisp constant.other.placeholder.lisp punctuation.definition.numeric.base.lisp",
    PlaceholderSep = "string.quoted.double.lisp constant.other.placeholder.lisp punctuation.separator.sequence.lisp",
    Pathname = "punctuation.definition.pathname.lisp",
    AnnotationPunct = "variable.annotation.lisp punctuation.definition.annotation.lisp",
    Annotation = "variable.annotation.lisp",
    AnnotationBegin = "variable.annotation.lisp punctuation.definition.symbol.begin.lisp",
    AnnotationEnd = "variable.annotation.lisp punctuation.definition.symbol.end.lisp",
    Accessor = "punctuation.accessor.lisp",
    Keyword = "keyword.other.symbol.lisp",
    KeywordBegin = "keyword.other.symbol.lisp punctuation.definition.symbol.begin.lisp",
    KeywordEnd = "keyword.other.symbol.lisp punctuation.definition.symbol.end.lisp",
    // a keyword's `:`, `::` or `#:` under the keyword's scope, so a theme colors the keyword whole
    // (X-009); bat's `punctuation.accessor` beside it, as `pkg:sym`'s still is
    KeywordMark = "keyword.other.symbol.lisp punctuation.definition.keyword.lisp",
    Control = "keyword.control.lisp",
    Quoted = "punctuation.definition.quoted.lisp",
    Unquote = "punctuation.definition.variable.lisp",
    Namespace = "variable.namespace.lisp",
    NamespaceBegin = "variable.namespace.lisp punctuation.definition.symbol.begin.lisp",
    NamespaceEnd = "variable.namespace.lisp punctuation.definition.symbol.end.lisp",
    Variable = "variable.other.lisp",
    VariableBegin = "variable.other.lisp punctuation.definition.symbol.begin.lisp",
    VariableEnd = "variable.other.lisp punctuation.definition.symbol.end.lisp",
    Parameter = "variable.parameter.lisp",
    ParameterBegin = "variable.parameter.lisp punctuation.definition.symbol.begin.lisp",
    ParameterEnd = "variable.parameter.lisp punctuation.definition.symbol.end.lisp",
    Symbol = "constant.other.symbol.lisp",
    SymbolBegin = "constant.other.symbol.lisp punctuation.definition.symbol.begin.lisp",
    SymbolEnd = "constant.other.symbol.lisp punctuation.definition.symbol.end.lisp",
    FnName = "entity.name.function.lisp",
    FnNameBegin = "entity.name.function.lisp punctuation.definition.symbol.begin.lisp",
    FnNameEnd = "entity.name.function.lisp punctuation.definition.symbol.end.lisp",
    MacroName = "entity.name.macro.lisp",
    MacroNameBegin = "entity.name.macro.lisp punctuation.definition.symbol.begin.lisp",
    MacroNameEnd = "entity.name.macro.lisp punctuation.definition.symbol.end.lisp",
    ClassName = "entity.name.class.lisp",
    ClassNameBegin = "entity.name.class.lisp punctuation.definition.symbol.begin.lisp",
    ClassNameEnd = "entity.name.class.lisp punctuation.definition.symbol.end.lisp",
    TypeName = "entity.name.type.lisp",
    TypeNameBegin = "entity.name.type.lisp punctuation.definition.symbol.begin.lisp",
    TypeNameEnd = "entity.name.type.lisp punctuation.definition.symbol.end.lisp",
    StructName = "entity.name.struct.lisp",
    StructNameBegin = "entity.name.struct.lisp punctuation.definition.symbol.begin.lisp",
    StructNameEnd = "entity.name.struct.lisp punctuation.definition.symbol.end.lisp",
    LoopName = "entity.name.loop.lisp",
    LoopNameBegin = "entity.name.loop.lisp punctuation.definition.symbol.begin.lisp",
    LoopNameEnd = "entity.name.loop.lisp punctuation.definition.symbol.end.lisp",
    Inherited = "entity.other.inherited-class.lisp",
    InheritedBegin = "entity.other.inherited-class.lisp punctuation.definition.symbol.begin.lisp",
    InheritedEnd = "entity.other.inherited-class.lisp punctuation.definition.symbol.end.lisp",
    Const = "constant.other.lisp",
    ConstBegin = "constant.other.lisp punctuation.definition.symbol.begin.lisp",
    ConstEnd = "constant.other.lisp punctuation.definition.symbol.end.lisp",
    Function = "variable.function.lisp",
    FunctionBegin = "variable.function.lisp punctuation.definition.symbol.begin.lisp",
    FunctionEnd = "variable.function.lisp punctuation.definition.symbol.end.lisp",
    Constructor = "variable.function.constructor.lisp",
    ConstructorBegin = "variable.function.constructor.lisp punctuation.definition.symbol.begin.lisp",
    ConstructorEnd = "variable.function.constructor.lisp punctuation.definition.symbol.end.lisp",
    Mutating = "variable.function.mutating.lisp",
    MutatingBegin = "variable.function.mutating.lisp punctuation.definition.symbol.begin.lisp",
    MutatingEnd = "variable.function.mutating.lisp punctuation.definition.symbol.end.lisp",
    Predicate = "variable.function.predicate.lisp",
    PredicateBegin = "variable.function.predicate.lisp punctuation.definition.symbol.begin.lisp",
    PredicateEnd = "variable.function.predicate.lisp punctuation.definition.symbol.end.lisp",
    Feature = "constant.other.feature.lisp",
    FeatureBegin = "constant.other.feature.lisp punctuation.definition.symbol.begin.lisp",
    FeatureEnd = "constant.other.feature.lisp punctuation.definition.symbol.end.lisp",
    Builtin = "support.function.lisp",
    DeclClass = "keyword.declaration.class.lisp",
    DeclFnInline = "keyword.declaration.function.inline.lisp",
    DeclFn = "keyword.declaration.function.lisp",
    DeclMacro = "keyword.declaration.macro.lisp",
    DeclStruct = "keyword.declaration.struct.lisp",
    DeclType = "keyword.declaration.type.lisp",
    DeclVar = "keyword.declaration.variable.lisp",
    DeclConst = "keyword.declaration.constant.lisp",
    Decl = "keyword.declaration.lisp",
    DeclDeclare = "keyword.declaration.declare.lisp",
    DeclName = "keyword.declaration.name.lisp",
    Conditional = "keyword.control.conditional.lisp",
    Return = "keyword.control.flow.return.lisp",
    LoopKw = "keyword.control.loop.lisp",
    Flow = "keyword.control.flow.lisp",
    Throw = "keyword.control.exception.throw.lisp",
    OpComparison = "keyword.operator.comparison.lisp",
    OpArithmetic = "keyword.operator.arithmetic.lisp",
    OpLogical = "keyword.operator.logical.lisp",
    OpBitwise = "keyword.operator.bitwise.lisp",
    Op = "keyword.operator.lisp",
    OpAssign = "keyword.operator.assignment.lisp",
    LoopOther = "keyword.other.loop.lisp",
    StorageType = "storage.type.lisp",
    Boolean = "constant.language.boolean.lisp",
    Null = "constant.language.null.lisp",
    ConstLang = "constant.language.lisp",
    VarLang = "variable.language.lisp",
    Modifier = "storage.modifier.lisp",
    NumValue = "constant.numeric.value.lisp",
    NumDot = "constant.numeric.value.lisp punctuation.separator.decimal.lisp",
    NumSlash = "constant.numeric.value.lisp keyword.operator.arithmetic.lisp",
    NumBase = "constant.numeric.base.lisp",
    BitVector = "meta.number.integer.binary constant.numeric.value.lisp",
    Vector = "punctuation.definition.vector.lisp",
    Array = "punctuation.definition.array.lisp",
    Struct = "punctuation.definition.struct.lisp",
    Complex = "punctuation.definition.complex.lisp",
    FunctionQuote = "punctuation.definition.function.lisp",
    CharStd = "constant.character.standard.lisp",
    CharStdPunct = "constant.character.standard.lisp punctuation.definition.constant.lisp",
    CharStdAttr = "constant.character.standard.lisp punctuation.definition.constant.lisp storage.modifier.attributes.lisp",
    CharSemi = "constant.character.semi-standard.lisp",
    CharSemiPunct = "constant.character.semi-standard.lisp punctuation.definition.constant.lisp",
    CharSemiAttr = "constant.character.semi-standard.lisp punctuation.definition.constant.lisp storage.modifier.attributes.lisp",
    CharOther = "constant.character.non-standard.lisp",
    CharOtherPunct = "constant.character.non-standard.lisp punctuation.definition.constant.lisp",
    CharOtherAttr = "constant.character.non-standard.lisp punctuation.definition.constant.lisp storage.modifier.attributes.lisp",
    Local = "variable.other.local.lisp",
    LocalPunct = "variable.other.local.lisp punctuation.definition.variable.lisp",
    RefBegin = "variable.other.lisp punctuation.definition.variable.begin.lisp",
    RefLocal = "variable.other.lisp variable.other.local.lisp",
    RefEnd = "variable.other.lisp punctuation.definition.variable.end.lisp",
    // Scheme's and Racket's
    VarName = "entity.name.variable.lisp",
    VarNameBegin = "entity.name.variable.lisp punctuation.definition.symbol.begin.lisp",
    VarNameEnd = "entity.name.variable.lisp punctuation.definition.symbol.end.lisp",
    Import = "keyword.control.import.lisp",
    ModuleName = "entity.name.namespace.lisp",
    StringPrefix = "storage.type.string.lisp",
    RegexBegin = "string.regexp.lisp punctuation.definition.string.begin.lisp",
    RegexEnd = "string.regexp.lisp punctuation.definition.string.end.lisp",
    Regex = "string.regexp.lisp",
    RegexEscape = "string.regexp.lisp constant.character.escape.lisp",
    DatumPunct = "comment.block.lisp punctuation.definition.comment.lisp",
}

/// The first kind of a `#rx"…"`'s or `#px"…"`'s tokens: the regex scanner's kinds from here,
/// under `string.regexp.lisp`.
const REGEX_KINDS: u16 = 256;

/// How many bytes of a run [`Scan::run_at`] reads a byte a step before sixteen a step: most
/// symbols end in these, and a step of sixteen costs more than a short run's bytes.
#[cfg(target_arch = "x86_64")]
const PRE: usize = 4;

/// How many depths a list's delimiters cycle through, each a rule of its own in a theme.
pub(crate) const DEPTHS: u8 = 6;

/// The first kind of a list's delimiters by depth, past the regex scanner's: `DEPTH_KINDS + 4 *
/// depth + i`, `i` the delimiter's place from [`L::ParenBegin`] (`(`, `)`, a parameter list's
/// `(` and `)`), its leaf under `meta.depth-N.lisp` (N from 1).
const DEPTH_KINDS: u16 = REGEX_KINDS + crate::regex::KINDS as u16;

static DEPTH_SCOPES: [&str; DEPTHS as usize] = [
    "meta.depth-1.lisp",
    "meta.depth-2.lisp",
    "meta.depth-3.lisp",
    "meta.depth-4.lisp",
    "meta.depth-5.lisp",
    "meta.depth-6.lisp",
];

/// The scopes kind `kind` carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    if kind >= DEPTH_KINDS {
        let (depth, i) = ((kind - DEPTH_KINDS) / 4, (kind - DEPTH_KINDS) % 4);
        let Some(&outer) = DEPTH_SCOPES.get(usize::from(depth)) else {
            return Vec::new();
        };
        return vec![outer, LEAVES[usize::from(L::ParenBegin as u16 + i)]];
    }
    if kind >= REGEX_KINDS {
        let mut s = vec!["string.regexp.lisp"];
        s.extend(crate::regex::scopes(kind - REGEX_KINDS));
        return s;
    }
    LEAVES.get(usize::from(kind)).map_or_else(Vec::new, |l| {
        l.split(' ').filter(|s| !s.is_empty()).collect()
    })
}

// The frames. Below the stack is the top level (`main`), never on it.
/// The top level: lists, expressions, and a stray `)`.
const TOP: u8 = 0;
// A list's frame: its elements and its `)`.
/// `paren-body` (and a function's, a macro's, a type's, `#(…)`'s, `#a(…)`'s, `#c(…)`'s body):
/// a list in it starts with a head.
const BODY: u8 = 1;
/// `lists-list-body` (and a class's slots, a struct's body, `#s(…)`'s): a list in it has no head.
const LIST: u8 = 2;
/// A quoted list (`'(…)`, `(quote …)`'s arguments): a list in it is quoted too.
const QUOTED: u8 = 3;
/// `loop`'s body: its keywords, and builtin functions among its names.
const LOOP: u8 = 4;
/// A function's, a macro's or a type's parameter list.
const PARAMS: u8 = 5;
/// A class's parent list.
const PARENTS: u8 = 6;
/// `let`'s bindings.
const VARS: u8 = 7;
/// `flet`'s and `labels`' functions.
const FLETS: u8 = 8;
/// `macrolet`'s macros.
const MACROLETS: u8 = 9;
// The one-shot frames: each takes a token (or a list) or pops at the first it doesn't.
/// `paren-begin`: a list's head.
const HEAD: u8 = 16;
const FN_NAME: u8 = 17;
const MACRO_NAME: u8 = 18;
const CLASS_NAME: u8 = 19;
const TYPE_NAME: u8 = 20;
const CONST_NAME: u8 = 21;
const VAR_NAME: u8 = 22;
/// A struct's name, or a list of its name and options.
const STRUCT_NAME: u8 = 23;
const STRUCT_QNAME: u8 = 24;
const LOOP_NAME: u8 = 25;
/// `method-qualifier`: expressions until a non-expression (the parameter list).
const QUALIFIER: u8 = 26;
const DECLARE_ID: u8 = 27;
const PARAM_NAME: u8 = 28;
const FN_PARAMS: u8 = 29;
const MACRO_PARAMS: u8 = 30;
const TYPE_PARAMS: u8 = 31;
const CLASS_PARENTS: u8 = 32;
/// `dolist`'s and `dotimes`' list.
const EXPECT_LIST: u8 = 33;
/// `do`'s list of lists.
const EXPECT_LISTS: u8 = 34;
const FLET_LIST: u8 = 35;
const MACROLET_LIST: u8 = 36;
const VAR_LIST: u8 = 37;
/// `(declare`'s specifiers: each list's first word.
const DECLARE_SPECS: u8 = 38;
/// `#s(`'s struct name.
const MSTRUCT_NAME: u8 = 39;
// Scheme's and Racket's.
/// `define`'s name, or a list of the function's name and its parameters.
const S_DEFINE_NAME: u8 = 40;
/// `define-syntax`'s.
const S_SYNTAX_NAME: u8 = 41;
/// The function's name in `(define (name …) …)`, or a curried list.
const S_FN_NAME: u8 = 42;
const S_MACRO_NAME: u8 = 43;
/// `lambda`'s parameters: a list, or one name for them all.
const S_PARAMS: u8 = 44;
/// `let`'s bindings, a named `let`'s name before them.
const S_LET_NAME: u8 = 45;
/// After `#;`: the datum it comments out.
const DATUM: u8 = 46;
/// `case-lambda`'s clauses: a list in it starts with parameters.
const CLAUSES: u8 = 10;
// Text across lines.
const STRING: u8 = 48;
/// `#p"…"`: a string without format directives.
const PATH: u8 = 49;
/// `#|…|#`, nested: one frame each.
const BLOCK: u8 = 50;
/// A list a `#;` comments out, a frame each level.
const DATUM_LIST: u8 = 51;
/// A string in it.
const DATUM_STRING: u8 = 52;
/// `#rx"…"` and `#px"…"`, the regex's state in [`State`]'s `arg`.
const REGEX: u8 = 53;
/// Guile's `#{…}#` symbol, past a line.
const XSYMBOL: u8 = 54;

/// How many frames a line holds; past it the bottom one drops (counted, if a body).
const MAX: usize = 128;
/// The state's bytes: [`State`]'s levels and withs.
const KEPT: usize = 2 * crate::matcher::KEPT;
/// A byte of the packed state with this bit is a run of list bodies, its count in the rest.
const RUN: u8 = 0x80;

// Byte classes.
const C_OTHER: u8 = 0;
const C_DIGIT: u8 = 1;
const C_DOT: u8 = 2;
const C_ESCAPE: u8 = 3;
const C_HIGH: u8 = 4;
/// `\s` (ASCII).
const C_SPACE: u8 = 5;
/// `[()"',:;|]`.
const C_BREAK: u8 = 6;

static CLASS: [u8; 256] = {
    let mut t = [C_OTHER; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = match i as u8 {
            b'0'..=b'9' => C_DIGIT,
            b'.' => C_DOT,
            b'\\' => C_ESCAPE,
            b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r' => C_SPACE,
            b'(' | b')' | b'"' | b'\'' | b',' | b':' | b';' | b'|' => C_BREAK,
            0x80.. => C_HIGH,
            _ => C_OTHER,
        };
        i += 1;
    }
    t
};

/// Scheme's and Racket's classes: `[`, `]`, `{` and `}` are lists' brackets too.
static SCHEME_CLASS: [u8; 256] = {
    let mut t = CLASS;
    t[b'[' as usize] = C_BREAK;
    t[b']' as usize] = C_BREAK;
    t[b'{' as usize] = C_BREAK;
    t[b'}' as usize] = C_BREAK;
    t
};

/// A string's bytes that end its text: its end, an escape, a directive.
static STRING_STOP: [bool; 256] = crate::regex::table(b"\"\\~");
static PATH_STOP: [bool; 256] = crate::regex::table(b"\"\\");

/// A `#rx"…"`'s and `#px"…"`'s host: its string's `"` and escapes; `\\` is the regex's `\`.
static REGEX_YIELDS: [bool; 256] = crate::regex::table(b"\"\\");
static REGEX_EMBED: crate::regex::Embed<'static> = crate::regex::Embed {
    dialect: crate::regex::Dialect::Java,
    cooked: true,
    yields: &REGEX_YIELDS,
    stop: |b, p, _| b[p] == b'"',
    arg: 0,
    escape: crate::regex::java_escape,
    top_only: false,
    continues: true,
    base: REGEX_KINDS,
    text: L::Regex as u16,
};

/// An identifier at a place: where it ends, and whether it is `|…|` or holds an escape.
#[derive(Clone, Copy)]
struct Id {
    end: usize,
    quoted: bool,
    escaped: bool,
}

/// The Lisps one scanner reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dialect {
    /// Common Lisp's, bat's `Lisp` as it is.
    Common,
    /// Emacs Lisp's: Common Lisp's reading with Emacs's `?a` characters.
    Emacs,
    /// Scheme's: heads of their own, `[…]` and `{…}` as lists, its reader's `#` forms.
    Scheme,
    /// Racket's: Scheme's, with Typed Racket's `#{x : T}` a list rather than Guile's symbol.
    Racket,
}

/// Lex `b`, a line of Lisp without its `\n`, from `state`, in `dialect`.
#[inline(never)]
pub fn line(b: &[u8], dialect: Dialect, state: &mut State, out: &mut Out) {
    let mut s = Scan::fresh(b, dialect, state, out);
    s.unpack(state);
    s.run();
    *state = s.pack();
}

/// A block of Common Lisp on one scan opened from `state` ([`crate::Carry`], X-070), `state`
/// left as the last line leaves it.
pub fn block_common(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    block(Dialect::Common, state, spans, pass)
}

/// [`block_common`]'s, in Emacs Lisp.
pub fn block_emacs(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    block(Dialect::Emacs, state, spans, pass)
}

/// [`block_common`]'s, in Scheme.
pub fn block_scheme(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    block(Dialect::Scheme, state, spans, pass)
}

/// [`block_common`]'s, in Racket.
pub fn block_racket(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    block(Dialect::Racket, state, spans, pass)
}

#[inline(always)]
fn block(
    dialect: Dialect,
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    let mut out = Out::new(spans);
    let mut c = Carried {
        s: Scan::fresh(b"", dialect, state, &mut out),
        deep: None,
    };
    c.s.unpack(state);
    let rest = pass(&mut c);
    *state = c.deep.unwrap_or_else(|| c.s.pack());
    rest
}

/// A scan kept across a block's lines, and the state the last line left where a save and load
/// don't leave its frames as they are (bodies below the stack, or more frames than a state's
/// codes): a block ending there hands on that state, as `line` would.
struct Carried<'b, 'o, 's> {
    s: Scan<'b, 'o, 's>,
    deep: Option<State>,
}

impl crate::Carry for Carried<'_, '_, '_> {
    fn next(&mut self, b: &[u8]) -> &[crate::Span] {
        let s = &mut self.s;
        s.out.0.clear();
        // SAFETY: the line is set back to an empty one below, before `b`'s borrow ends
        s.b = unsafe { crate::line_for(b) };
        s.dead = (usize::MAX, 0);
        s.quoted_fail = usize::MAX;
        s.plus = (usize::MAX, 0, 0);
        s.digits = [(usize::MAX, 0); 2];
        s.run();
        s.b = b"";
        // a regex's state is kept only while it is the top frame (`pack`)
        if s.top() != REGEX {
            s.regex = 0;
        }
        // `keep`'s codes in order and `expand`'s frames from them: the frames as they are
        self.deep = (s.base > 0 || s.depth > KEPT).then(|| s.reloaded());
        s.out.0
    }
}

impl<'b, 'o, 's> Scan<'b, 'o, 's> {
    /// A scan of `b` in `dialect`, returned as a literal so that it is built in place; its
    /// frames are [`Scan::unpack`]'s.
    #[inline(always)]
    fn fresh(b: &'b [u8], dialect: Dialect, state: &State, out: &'o mut Out<'s>) -> Self {
        let scheme = matches!(dialect, Dialect::Scheme | Dialect::Racket);
        Scan {
            b,
            racket: dialect == Dialect::Racket,
            elisp: dialect == Dialect::Emacs,
            scheme,
            class: if scheme { &SCHEME_CLASS } else { &CLASS },
            regex: state.arg as u8,
            level: DEPTH_KINDS + 4 * (state.arg >> 8).min(u16::from(DEPTHS) - 1),
            out,
            stack: [TOP; MAX + 1],
            depth: 0,
            base: 0,
            exact: state.is_exact(),
            dead: (usize::MAX, 0),
            quoted_fail: usize::MAX,
            plus: (usize::MAX, 0, 0),
            digits: [(usize::MAX, 0); 2],
        }
    }

    /// The scan as a state's save and the next line's load leave it: that state.
    #[cold]
    #[inline(never)]
    fn reloaded(&mut self) -> State {
        let state = self.pack();
        self.regex = state.arg as u8;
        self.level = DEPTH_KINDS + 4 * (state.arg >> 8).min(u16::from(DEPTHS) - 1);
        self.stack = [TOP; MAX + 1];
        self.depth = 0;
        self.base = 0;
        self.exact = state.is_exact();
        self.unpack(&state);
        state
    }
}

struct Scan<'b, 'o, 's> {
    b: &'b [u8],
    /// Emacs Lisp: `?a` is a character.
    elisp: bool,
    /// Scheme or Racket ([`Dialect::Scheme`]).
    scheme: bool,
    /// Racket ([`Dialect::Racket`]).
    racket: bool,
    /// The byte classes: Scheme's take `[]{}` as breaks.
    class: &'static [u8; 256],
    /// An open `#rx"…"`'s regex state.
    regex: u8,
    /// The lists open, modulo [`DEPTHS`], as that depth's first kind ([`DEPTH_KINDS`]): the
    /// depth the next `(` colors as.
    level: u16,
    out: &'o mut Out<'s>,
    /// The frames at `1..=depth`, bottom first; at 0 what is below them: a body body (`BODY`)
    /// while `base` counts any, else `TOP`. So the top is one load, `stack[depth]`.
    stack: [u8; MAX + 1],
    depth: usize,
    /// Plain list bodies below the stack.
    base: usize,
    exact: bool,
    /// A run of digits and dots no identifier is in: from, to.
    dead: (usize, usize),
    /// Where an unclosed `|…|` was found: no `|` there or after closes one.
    quoted_fail: usize,
    /// `\+\S+\+` in a run of non-spaces: where the run was read from, its end, and the end of
    /// its last `+` before a break (0 for none).
    plus: (usize, usize, usize),
    /// Two runs of digits, from and to, for numbers tried at each place of a run.
    digits: [(usize, usize); 2],
}

impl Scan<'_, '_, '_> {
    /// The frames `state` keeps: the top ones on the stack (half of it at most, so that the line
    /// can open more), the list bodies below them counted in `base`.
    fn unpack(&mut self, state: &State) {
        // most lines: every frame fits, none below the stack
        if let Some(d) = expand(state, BODY, &mut self.stack[1..]) {
            self.depth = d;
            return;
        }
        let depth = usize::from(state.depth);
        let count = |c: u8| {
            if c & RUN != 0 {
                usize::from(c & !RUN)
            } else {
                1
            }
        };
        let total: usize = (0..depth).map(|i| count(state.level(i))).sum();
        // the frames past the room stay below the stack, all bodies but a dropped other
        let mut skip = total.saturating_sub(MAX / 2);
        for i in 0..depth {
            let c = state.level(i);
            let k = count(c);
            let below = skip.min(k);
            skip -= below;
            if c & RUN != 0 {
                self.base += below;
                for _ in below..k {
                    self.push(BODY);
                }
            } else if below > 0 {
                self.exact = false;
            } else {
                self.push(c);
            }
        }
        if self.base > 0 {
            self.stack[0] = BODY;
        }
    }

    fn pack(&self) -> State {
        let arg = ((self.level - DEPTH_KINDS) / 4) << 8
            | if self.top() == REGEX {
                u16::from(self.regex)
            } else {
                0
            };
        keep(
            &self.stack[1..=self.depth],
            BODY,
            self.base,
            self.exact,
            arg,
        )
    }

    fn top(&self) -> u8 {
        self.stack[self.depth]
    }

    fn push(&mut self, f: u8) {
        if self.depth == MAX {
            // the bottom frame dropped: a body into the count below
            if self.stack[1] == BODY {
                self.base += 1;
                self.stack[0] = BODY;
            } else {
                self.exact = false;
            }
            self.stack.copy_within(2.., 1);
            self.depth -= 1;
        }
        self.depth += 1;
        self.stack[self.depth] = f;
    }

    fn pop(&mut self) {
        if self.depth > 0 {
            self.depth -= 1;
        } else if self.base > 0 {
            self.base -= 1;
            if self.base == 0 {
                self.stack[0] = TOP;
            }
        }
    }

    /// The top frame replaced by `f`.
    fn set(&mut self, f: u8) {
        self.pop();
        self.push(f);
    }

    /// The frame below the top (a head's list) made `f`.
    fn set_below(&mut self, f: u8) {
        if self.depth >= 2 {
            self.stack[self.depth - 1] = f;
        } else if self.depth == 1 && self.base > 0 {
            // the body is in the count below the stack: take it out
            self.base -= 1;
            if self.base == 0 {
                self.stack[0] = TOP;
            }
            let top = self.stack[1];
            self.stack[1] = f;
            self.stack[2] = top;
            self.depth = 2;
        }
    }

    fn put(&mut self, from: usize, to: usize, kind: L) {
        self.out.push(from, to, kind as u16);
    }

    /// A list's delimiter `kind` (`(`, `)`, a parameter list's) at `from..to`, the kind of its
    /// depth ([`DEPTH_KINDS`]): an opener at the depth it opens, a closer at the depth it closes.
    #[inline(always)]
    fn delim(&mut self, from: usize, to: usize, kind: L) {
        let i = kind as u16 - L::ParenBegin as u16;
        debug_assert!(i < 4, "{kind:?} is no delimiter");
        // `level` is the depth's first kind: an opener's is the depth's, then the next one's
        const FIRST: u16 = DEPTH_KINDS;
        const LAST: u16 = DEPTH_KINDS + 4 * (DEPTHS as u16 - 1);
        if i & 1 != 0 {
            self.level = if self.level == FIRST {
                LAST
            } else {
                self.level - 4
            };
        }
        self.out.push(from, to, self.level + i);
        if i & 1 == 0 {
            self.level = if self.level == LAST {
                FIRST
            } else {
                self.level + 4
            };
        }
    }

    fn put_merged(&mut self, from: usize, to: usize, kind: L) {
        self.out.push_merged(from, to, kind as u16);
    }

    /// An identifier's span as kind `kind`, its `|…|` delimiters as the two kinds after it. In
    /// its callers, a `|…|` symbol called: most identifiers are a span.
    #[inline(always)]
    fn put_id(&mut self, from: usize, id: Id, kind: L) {
        if id.quoted {
            self.put_quoted(from, id, kind);
        } else {
            self.put(from, id.end, kind);
        }
    }

    /// [`Scan::put_id`] of a `|…|` symbol.
    #[cold]
    fn put_quoted(&mut self, from: usize, id: Id, kind: L) {
        let k = kind as u16;
        self.out.push(from, from + 1, k + 1);
        self.out.push(from + 1, id.end - 1, k);
        self.out.push(id.end - 1, id.end, k + 2);
    }

    fn at(&self, p: usize) -> u8 {
        self.b.get(p).copied().unwrap_or(b'\n')
    }

    fn run(&mut self) {
        // a line's indentation, eight bytes a step, where a code frame's first turn would skip
        // it; a text frame's (a string's, a comment's) spaces are its text
        let mut p = if self.top() < STRING {
            blanks(self.b, 0)
        } else {
            0
        };
        loop {
            let top = self.top();
            p = match top {
                STRING => self.string(p, &STRING_STOP),
                PATH => self.string(p, &PATH_STOP),
                BLOCK => self.block(p),
                DATUM_LIST | DATUM_STRING => self.datum_text(p, top),
                REGEX => self.regex_text(p),
                XSYMBOL => self.xsymbol_text(p),
                _ => {
                    p = self.skip_spaces(p);
                    if p >= self.b.len() {
                        return;
                    }
                    match top {
                        TOP | BODY | LIST | QUOTED | CLAUSES => self.body(p, top),
                        LOOP => self.loop_body(p),
                        PARAMS => self.params(p),
                        PARENTS => self.parents(p),
                        VARS | FLETS | MACROLETS => self.bindings(p, top),
                        HEAD => self.head(p),
                        _ => self.pending(p, top),
                    }
                }
            };
            if p >= self.b.len() {
                return;
            }
        }
    }

    /// Where the spaces at `p` end (`\s`, Unicode's too).
    fn skip_spaces(&self, mut p: usize) -> usize {
        while p < self.b.len() {
            match self.class[usize::from(self.b[p])] {
                C_SPACE => p += 1,
                C_HIGH => match self.wide_space(p) {
                    0 => break,
                    k => p += k,
                },
                _ => break,
            }
        }
        p
    }

    /// The bytes Unicode's space at `p` takes, 0 for none.
    #[cold]
    fn wide_space(&self, p: usize) -> usize {
        match wide_char(self.b, p) {
            Some(c) if c.is_whitespace() => c.len_utf8(),
            _ => 0,
        }
    }

    /// `{{break}}` at `p`: a break character, or the line's end.
    fn at_break(&self, p: usize) -> bool {
        p >= self.b.len()
            || match self.class[usize::from(self.b[p])] {
                C_SPACE | C_BREAK => true,
                C_HIGH => self.wide_space(p) > 0,
                _ => false,
            }
    }

    /// The next character's place.
    fn step(&self, p: usize) -> usize {
        (p + char_len(self.b[p])).min(self.b.len())
    }

    /// An unquoted identifier's run at `p`: escapes and characters but breaks. Its end, whether
    /// it has a character that is neither a digit nor a `.`, and whether it has an escape. In its
    /// callers ([`Scan::ident_at`] the hot one): a call saved six registers a token. Where a run
    /// is rare, [`Scan::run_at_call`].
    #[inline(always)]
    fn run_at(&self, p: usize) -> (usize, bool, bool) {
        #[cfg(target_arch = "x86_64")]
        if self.b.len() >= 16 {
            return self.run_wide(p);
        }
        self.run_bytes(p)
    }

    /// [`Scan::run_at`] sixteen bytes a step (SSE2, which every x86-64 has), on a line of
    /// sixteen or more: the bytes a run takes as they are ([`plain_masks`]) a step, an escape or a
    /// character past ASCII as [`Scan::run_bytes`] takes it.
    #[cfg(target_arch = "x86_64")]
    #[inline(always)]
    fn run_wide(&self, p: usize) -> (usize, bool, bool) {
        let (b, n) = (self.b, self.b.len());
        let (mut q, mut y, mut escaped) = (p, false, false);
        // a short run's bytes a step, cheaper than a step of sixteen: most runs end in these,
        // and the byte after them is read too, so a run of `PRE` (the commonest length) ends here
        let pre = (p + PRE).min(n - 1);
        while q <= pre {
            match self.class[usize::from(b[q])] {
                C_OTHER => {
                    y = true;
                    q += 1;
                }
                C_DIGIT | C_DOT => q += 1,
                // an escape or a character past ASCII: the steps below take it
                C_ESCAPE | C_HIGH => break,
                _ => return (q, y, escaped),
            }
        }
        while q < n {
            // the sixteen bytes from `q`, the line's last sixteen where fewer are left
            let c = q.min(n - 16);
            let s = q - c;
            let (plain, digits) = plain_masks(b, c, self.scheme);
            let stop = !(plain >> s) & (0xffff >> s);
            let len = if stop != 0 {
                stop.trailing_zeros()
            } else {
                16 - s as u32
            };
            // a byte taken that is neither a digit nor a `.`
            let taken = (1u32 << len) - 1;
            y |= (taken & !(digits >> s)) != 0;
            q += len as usize;
            if stop == 0 {
                continue;
            }
            match self.class[usize::from(b[q])] {
                C_ESCAPE => {
                    if q + 1 >= n {
                        break;
                    }
                    q += 1 + char_len(b[q + 1]);
                    (y, escaped) = (true, true);
                }
                C_HIGH => {
                    if self.wide_space(q) > 0 {
                        break;
                    }
                    if !wide_char(b, q).is_some_and(is_nd) {
                        y = true;
                    }
                    q += char_len(b[q]);
                }
                _ => break,
            }
        }
        (q.min(n), y, escaped)
    }

    /// [`Scan::run_at`] in a call, where a run is read less often than an identifier's (a
    /// character's name, a datum comment's): 1.2 KB of copies, no cycles (X-038). Scheme's
    /// `#lang` and `#!`, once a file, keep it in place: called there too, Racket's and Scheme's
    /// cycles were +0.5% (the code's layout, not the calls).
    #[inline(never)]
    fn run_at_call(&self, p: usize) -> (usize, bool, bool) {
        self.run_at(p)
    }

    /// [`Scan::run_at`] a byte a step.
    fn run_bytes(&self, p: usize) -> (usize, bool, bool) {
        let (b, n) = (self.b, self.b.len());
        let (mut q, mut y, mut escaped) = (p, false, false);
        while q < n {
            match self.class[usize::from(b[q])] {
                C_OTHER => {
                    y = true;
                    q += 1;
                }
                C_DIGIT | C_DOT => q += 1,
                C_ESCAPE => {
                    // `\\.`: a `\` ending the line is no escape (its `\n`), and no character
                    if q + 1 >= n {
                        break;
                    }
                    q += 1 + char_len(b[q + 1]);
                    (y, escaped) = (true, true);
                }
                C_HIGH => {
                    if self.wide_space(q) > 0 {
                        break;
                    }
                    let c = wide_char(b, q);
                    if !c.is_some_and(is_nd) {
                        y = true;
                    }
                    q += char_len(b[q]);
                }
                _ => break,
            }
        }
        (q.min(n), y, escaped)
    }

    /// `{{identifier}}` at `p`.
    fn ident_at(&mut self, p: usize) -> Option<Id> {
        if p >= self.b.len() {
            return None;
        }
        if self.b[p] == b'|' {
            let end = self.quoted(p)?;
            return Some(Id {
                end,
                quoted: true,
                escaped: false,
            });
        }
        if p >= self.dead.0 && p < self.dead.1 {
            return None;
        }
        let (end, y, escaped) = self.run_at(p);
        if end == p {
            return None;
        }
        if !y {
            self.dead = (p, end);
            return None;
        }
        Some(Id {
            end,
            quoted: false,
            escaped,
        })
    }

    /// `(\|)(?:\\.|[^|])*(\|)` at `p`: its end.
    fn quoted(&mut self, p: usize) -> Option<usize> {
        if p >= self.quoted_fail {
            return None;
        }
        let (b, n) = (self.b, self.b.len());
        let mut q = p + 1;
        while q < n {
            match b[q] {
                b'|' => return Some(q + 1),
                b'\\' if q + 1 < n => q += 1 + char_len(b[q + 1]),
                c => q += char_len(c),
            }
        }
        self.quoted_fail = p;
        None
    }

    /// A word of the lists at `p`, the whole token `id`, followed by a break: what it is. In
    /// its callers: a call saved six registers for a lookup of ~70 instructions. Called where a
    /// word is tried less often (a function's name, a loop's body, `declare`), it saved 3 KB of
    /// copies for +1% cycles (X-038): every caller has it in place.
    #[inline(always)]
    fn word(&self, p: usize, id: Id) -> u32 {
        if id.quoted || id.escaped || !self.at_break(id.end) {
            return 0;
        }
        lookup(&self.b[p..id.end])
    }

    /// One turn of a list's body (or the top level's).
    fn body(&mut self, p: usize, top: u8) -> usize {
        match self.b[p] {
            c if self.opens(c) => {
                self.delim(p, p + 1, L::ParenBegin);
                match top {
                    TOP | BODY => {
                        self.push(BODY);
                        self.push(HEAD);
                    }
                    LIST => self.push(BODY),
                    CLAUSES => {
                        self.push(BODY);
                        self.push(S_PARAMS);
                    }
                    _ => self.push(QUOTED),
                }
                p + 1
            }
            c if self.closes(c) => {
                if top == TOP {
                    self.put(p, p + 1, L::StrayParen);
                } else {
                    self.delim(p, p + 1, L::ParenEnd);
                    self.pop();
                }
                p + 1
            }
            _ => match self.expression(p, false) {
                Some(q) => q,
                None => self.step(p),
            },
        }
    }

    /// Whether `c` opens a list: `(`, and in Scheme `[` and `{` too.
    fn opens(&self, c: u8) -> bool {
        c == b'(' || (self.scheme && matches!(c, b'[' | b'{'))
    }

    /// Whether `c` closes one.
    fn closes(&self, c: u8) -> bool {
        c == b')' || (self.scheme && matches!(c, b']' | b'}'))
    }

    /// `expressions` at `p`: where the match ends, `None` for none.
    fn expression(&mut self, p: usize, lp: bool) -> Option<usize> {
        let c = self.b[p];
        match c {
            b';' => Some(self.line_comment(p)),
            b'#' => {
                if self.at(p + 1) == b'|' {
                    return Some(self.block_open(p));
                }
                if self.scheme
                    && let Some(q) = self.scheme_hash(p)
                {
                    return Some(q);
                }
                if let Some(q) = self.macros(p) {
                    return Some(q);
                }
                self.identifiers(p, lp)
            }
            b'|' => {
                // in Scheme a `|…|` symbol that closes is one, `|#%kernel|` too
                if self.at(p + 1) == b'#' && !(self.scheme && self.quoted(p).is_some()) {
                    self.put(p, p + 2, L::StrayCommentEnd);
                    return Some(p + 2);
                }
                self.identifiers(p, lp)
            }
            b'"' => {
                self.put(p, p + 1, L::StrBegin);
                self.push(STRING);
                Some(p + 1)
            }
            b'0'..=b'9' | b'+' | b'-' | b'.' => {
                if let Some(q) = self.number(p) {
                    return Some(q);
                }
                if c == b'.' && self.at_break(p + 1) {
                    self.put(p, p + 1, L::Control);
                    return Some(p + 1);
                }
                self.identifiers(p, lp)
            }
            b'&' => self.annotation(p).or_else(|| self.identifiers(p, lp)),
            b':' => self.keyword(p),
            b'\'' => {
                self.put(p, p + 1, L::Quoted);
                Some(self.quoted_content(p + 1))
            }
            b'`' => {
                self.put(p, p + 1, L::Quoted);
                Some(p + 1)
            }
            b'?' if self.elisp => self.elisp_char(p).or_else(|| self.identifiers(p, lp)),
            b',' => {
                let e = p + 1 + usize::from(self.at(p + 1) == b'@');
                self.put(p, e, L::Unquote);
                Some(e)
            }
            // `\d` is Unicode's decimal digits
            0x80.. if digit(self.b, p).is_some() => {
                self.number(p).or_else(|| self.identifiers(p, lp))
            }
            _ => self.identifiers(p, lp),
        }
    }

    /// Emacs Lisp's character at `p`, a `?` ([`Lang::EmacsLisp`](crate::Lang) only), a fix over bat (whose grammar reads `?a` as a symbol,
    /// and `?\\)` or `?|` as one that runs on): `?` and a character, or `?\` and an escape
    /// (`?\(`, `?\n`, `?\C-\M-x`, `?\^I`, `?\x41`, `?\N{name}`, `?\101`), then a delimiter. A
    /// longer `?…` is a symbol, as Common Lisp's `?var`.
    fn elisp_char(&mut self, p: usize) -> Option<usize> {
        let q = self.elisp_char_end(p)?;
        self.put(p, p + 1, L::CharStdPunct);
        self.put(p + 1, q, L::CharStd);
        Some(q)
    }

    /// Where [`Scan::elisp_char`]'s character at `p` ends.
    fn elisp_char_end(&self, p: usize) -> Option<usize> {
        let (b, n) = (self.b, self.b.len());
        let mut q = p + 1;
        if q >= n {
            return None;
        }
        if b[q] == b'\\' {
            q += 1;
            // modifiers: `C-`, `M-`, `S-`, `H-`, `A-`, `s-`, each maybe before another `\`
            while q + 1 < n
                && matches!(b[q], b'C' | b'M' | b'S' | b'H' | b'A' | b's')
                && b[q + 1] == b'-'
            {
                q += 2;
                if q < n && b[q] == b'\\' {
                    q += 1;
                }
            }
            if q < n && b[q] == b'^' {
                q += 1;
            }
            if q >= n {
                return None;
            }
            let c = b[q];
            q += char_len(c);
            let hex = |mut i: usize, max: usize| {
                let from = i;
                while i < n && i - from < max && b[i].is_ascii_hexdigit() {
                    i += 1;
                }
                i
            };
            q = match c {
                b'x' if q < n && b[q] == b'{' => b[q..]
                    .iter()
                    .position(|&c| c == b'}')
                    .map_or(q, |i| q + i + 1),
                b'x' => hex(q, 8),
                b'u' => hex(q, 4),
                b'U' => hex(q, 8),
                b'N' if q < n && b[q] == b'{' => b[q..]
                    .iter()
                    .position(|&c| c == b'}')
                    .map_or(q, |i| q + i + 1),
                b'0'..=b'7' => {
                    let mut i = q;
                    while i < n && i - q < 2 && (b'0'..=b'7').contains(&b[i]) {
                        i += 1;
                    }
                    i
                }
                _ => q,
            };
        } else {
            q += char_len(b[q]);
        }
        let q = q.min(n);
        (self.at_break(q) || matches!(b.get(q), Some(b'[' | b']'))).then_some(q)
    }

    /// `annotations` at `p`: `&` and an identifier.
    fn annotation(&mut self, p: usize) -> Option<usize> {
        let id = self.ident_at(p + 1)?;
        self.put(p, p + 1, L::AnnotationPunct);
        self.put_id(p + 1, id, L::Annotation);
        Some(id.end)
    }

    /// `keywords`' `(::?)({{identifier}})` at `p`.
    fn keyword(&mut self, p: usize) -> Option<usize> {
        let k = p + 1 + usize::from(self.at(p + 1) == b':');
        let id = self.ident_at(k)?;
        self.put(p, k, L::KeywordMark);
        self.put_id(k, id, L::Keyword);
        Some(id.end)
    }

    /// `quoted-content`, right after a `'`.
    fn quoted_content(&mut self, p: usize) -> usize {
        // Scheme's `'#t`, `'#(…)`, `'#{a b}#`: the body reads the `#` form
        if self.scheme && self.at(p) == b'#' && p < self.b.len() {
            return p;
        }
        if p < self.b.len() && self.opens(self.b[p]) {
            self.delim(p, p + 1, L::ParenBegin);
            self.push(QUOTED);
            return p + 1;
        }
        match self.ident_at(p) {
            Some(id) => {
                self.put_id(p, id, L::Symbol);
                id.end
            }
            None => p,
        }
    }

    /// `namespaces` at `p` (`pkg:`, `pkg::`, `keyword`): where it ends. Tried at every
    /// identifier and a head, so what can start one is tested in place: `keyword` (maybe in
    /// `|…|`), or an identifier with a `:` after it.
    #[inline(always)]
    fn namespace(&mut self, p: usize, id: Option<Id>) -> Option<usize> {
        let c = self.at(p);
        if c != b'k' && c != b'|' && !id.is_some_and(|id| self.at(id.end) == b':') {
            return None;
        }
        self.namespace_at(p, id)
    }

    /// [`Scan::namespace`] past its gate.
    fn namespace_at(&mut self, p: usize, id: Option<Id>) -> Option<usize> {
        // `(\|?)keyword(\|?)(?=:)`
        let b = self.b;
        let k = p + usize::from(self.at(p) == b'|');
        // its first byte before the compare: most identifiers aren't it
        if self.at(k) == b'k' && b.get(k..k + 7).is_some_and(|w| crate::same(w, b"keyword")) {
            let e = k + 7 + usize::from(self.at(k + 7) == b'|');
            if self.at(e) == b':' && e < self.b.len() {
                if k > p {
                    self.put(p, k, L::NamespaceBegin);
                }
                self.put(k, k + 7, L::Namespace);
                if e > k + 7 {
                    self.put(k + 7, e, L::NamespaceEnd);
                }
                return Some(e);
            }
        }
        let id = id?;
        if b[p] == b'#' || self.at(id.end) != b':' || id.end >= self.b.len() {
            return None;
        }
        self.put_id(p, id, L::Namespace);
        let e = id.end + 1 + usize::from(self.at(id.end + 1) == b':');
        self.put(id.end, e, L::Accessor);
        Some(e)
    }

    /// `identifiers` at `p`: the `identifier` context, its namespaces first.
    fn identifier(&mut self, p: usize) -> Option<usize> {
        let mut id = self.ident_at(p)?;
        let mut p = p;
        while let Some(q) = self.namespace(p, Some(id)) {
            p = q;
            match self.ident_at(p) {
                Some(i) => id = i,
                None => return Some(p),
            }
        }
        Some(self.ident_word(p, id))
    }

    /// The `identifier` context's patterns after its namespaces, for `id` at `p`.
    fn ident_word(&mut self, p: usize, id: Id) -> usize {
        let info = self.word(p, id);
        if self.scheme {
            // Common Lisp's `t`, `nil`, types, `+c+` and `*v*` aren't Scheme's
            let kind = match scheme_ident(info) {
                S_CONSTANT => L::ConstLang,
                S_KEYWORD => L::Conditional,
                _ => {
                    self.put_id(p, id, L::Variable);
                    return id.end;
                }
            };
            self.put(p, id.end, kind);
            return id.end;
        }
        let kind = match ident(info) {
            I_TYPE => Some(L::StorageType),
            I_BOOLEAN => Some(L::Boolean),
            I_NULL => Some(L::Null),
            I_CONSTANT => Some(L::ConstLang),
            _ => None,
        };
        if let Some(kind) = kind {
            self.put(p, id.end, kind);
            return id.end;
        }
        if self.b[p] == b'+'
            && let Some(e) = self.plus_constant(p)
        {
            self.put(p, e, L::Const);
            return e;
        }
        if ident(info) == I_VARIABLE {
            self.put(p, id.end, L::VarLang);
            return id.end;
        }
        self.put_id(p, id, L::Variable);
        id.end
    }

    /// `\+\S+\+{{break}}` at `p`: its end, the last `+` in the run of non-spaces before a break.
    fn plus_constant(&mut self, p: usize) -> Option<usize> {
        let (b, n) = (self.b, self.b.len());
        // the run of non-spaces, and its last `+` before a break, once a run
        if !(p >= self.plus.0 && p < self.plus.1) {
            let mut s = p + 1;
            while s < n && CLASS[usize::from(b[s])] != C_SPACE {
                if b[s] >= 0x80 && self.wide_space(s) > 0 {
                    break;
                }
                s += 1;
            }
            let mut last = 0;
            let mut q = s;
            while q > p + 1 {
                q -= 1;
                if b[q] == b'+' && self.at_break(q + 1) {
                    last = q + 1;
                    break;
                }
            }
            self.plus = (p, s, last);
        }
        let e = self.plus.2;
        (e >= p + 3).then_some(e)
    }

    /// `paren-begin`: a list's head at `p`.
    fn head(&mut self, p: usize) -> usize {
        // an Emacs Lisp character is no head (`'((?a . 1))`): the body reads it
        if self.elisp && self.b[p] == b'?' && self.elisp_char_end(p).is_some() {
            self.pop();
            return p;
        }
        if self.scheme {
            return self.scheme_head(p);
        }
        let id = self.ident_at(p);
        if let Some(id) = id {
            let info = self.word(p, id);
            let wordy = !id.quoted && !id.escaped && self.at_break(id.end);
            let h = if wordy && is_defun(&self.b[p..id.end]) {
                H_DEFUN
            } else {
                words::head(info)
            };
            if h != 0 {
                self.declaration(p, id.end, h);
                return id.end;
            }
            if let Some(q) = self.namespace(p, Some(id)) {
                return q;
            }
            if info & BUILTIN != 0 {
                self.put(p, id.end, L::Builtin);
                self.pop();
                return id.end;
            }
            if let Some(e) = self.other_function(p, id, info) {
                self.pop();
                return e;
            }
        } else if let Some(q) = self.namespace(p, None) {
            return q;
        }
        match self.comment(p) {
            Some(q) => q,
            None => {
                self.pop();
                p
            }
        }
    }

    /// A head's word `h` at `p..e`: its kind, and what the list's elements are after it.
    fn declaration(&mut self, p: usize, e: usize, h: u32) {
        let (kind, below, next): (L, u8, &[u8]) = match h {
            H_CLASS => (L::DeclClass, LIST, &[CLASS_PARENTS, CLASS_NAME]),
            H_LAMBDA => (L::DeclFnInline, BODY, &[FN_PARAMS]),
            H_DEFUN => (L::DeclFn, BODY, &[FN_PARAMS, FN_NAME]),
            H_METHOD => (L::DeclFn, BODY, &[FN_PARAMS, QUALIFIER, FN_NAME]),
            H_FLET => (L::DeclFn, BODY, &[FLET_LIST]),
            H_MACRO => (L::DeclMacro, BODY, &[MACRO_PARAMS, MACRO_NAME]),
            H_MACROLET => (L::DeclMacro, BODY, &[MACROLET_LIST]),
            H_STRUCT => (L::DeclStruct, LIST, &[STRUCT_NAME]),
            H_TYPE => (L::DeclType, BODY, &[TYPE_PARAMS, TYPE_NAME]),
            H_LET => (L::DeclVar, BODY, &[VAR_LIST]),
            H_CONSTANT => (L::DeclConst, BODY, &[CONST_NAME]),
            H_VAR => (L::DeclVar, BODY, &[VAR_NAME]),
            H_DECLARATION => (L::Decl, BODY, &[]),
            H_DECLARE => (L::DeclDeclare, BODY, &[DECLARE_SPECS]),
            H_CONDITIONAL => (L::Conditional, BODY, &[]),
            H_RETURN => (L::Return, BODY, &[]),
            H_DOLIST => (L::LoopKw, BODY, &[EXPECT_LIST]),
            H_DO => (L::LoopKw, BODY, &[EXPECT_LISTS]),
            H_CONTROL => (L::Control, BODY, &[]),
            H_LOOP => (L::LoopKw, LOOP, &[]),
            H_COMPARISON => (L::OpComparison, BODY, &[]),
            H_ARITHMETIC => (L::OpArithmetic, BODY, &[]),
            H_LOGICAL => (L::OpLogical, BODY, &[]),
            H_BITWISE => (L::OpBitwise, BODY, &[]),
            _ => (L::Builtin, QUOTED, &[]),
        };
        self.put(p, e, kind);
        if below != BODY {
            self.set_below(below);
        }
        self.pop();
        for &f in next {
            self.push(f);
        }
    }

    /// `other-function` at `p`, for identifier `id` that is word `info`: where it ends.
    fn other_function(&mut self, p: usize, id: Id, info: u32) -> Option<usize> {
        let b = self.b;
        // `(?![+*#`'&]\S)`
        if matches!(b[p], b'+' | b'*' | b'#' | b'`' | b'\'' | b'&') && !self.at_break_space(p + 1) {
            return None;
        }
        if !id.quoted {
            let e = id.end;
            // `make-{{identifier}}`
            if e - p >= 5
                && b[p] == b'm'
                && crate::same(&b[p..p + 5], b"make-")
                && let Some(rest) = self.ident_at(p + 5)
                && (rest.quoted || rest.end == e)
            {
                if rest.quoted {
                    self.put(p, p + 5, L::Constructor);
                    self.put_id(p + 5, rest, L::Constructor);
                } else {
                    self.put(p, e, L::Constructor);
                }
                return Some(rest.end);
            }
            if self.at_break(e) {
                // `{{identifier}}\!` and `{{identifier}}(?:\?|-p)`, the suffix not escaped
                let suffix = match b[e - 1] {
                    b'!' => Some((1, L::Mutating)),
                    b'?' => Some((1, L::Predicate)),
                    b'p' if e - p >= 2 && b[e - 2] == b'-' => Some((2, L::Predicate)),
                    _ => None,
                };
                if let Some((k, kind)) = suffix
                    && e - k > p
                    && self.ends_unescaped(p, e - k)
                {
                    self.put(p, e, kind);
                    return Some(e);
                }
            }
        } else {
            let e = id.end;
            let suffix = match self.at(e) {
                b'!' if e < self.b.len() => Some((1, L::Mutating)),
                b'?' if e < self.b.len() => Some((1, L::Predicate)),
                b'-' if self.at(e + 1) == b'p' && e + 1 < self.b.len() => Some((2, L::Predicate)),
                _ => None,
            };
            if let Some((k, kind)) = suffix
                && self.at_break(e + k)
            {
                self.put_id(p, id, kind);
                self.put(e, e + k, kind);
                return Some(e + k);
            }
        }
        if info & NOT_FUNCTION != 0 {
            return None;
        }
        self.put_id(p, id, L::Function);
        Some(id.end)
    }

    /// Whether `p..e`, a prefix of an identifier's run, is an identifier ending on a character
    /// of its own (not inside an escape), with a character neither a digit nor a `.`.
    fn ends_unescaped(&self, p: usize, e: usize) -> bool {
        let (end, y, _) = self.run_at_to(p, e);
        end == e && y
    }

    /// [`Scan::run_at`] stopped at `limit` (past it: an escape runs over it).
    fn run_at_to(&self, p: usize, limit: usize) -> (usize, bool, bool) {
        Scan::run_in(&self.b[..limit], p, self.class)
    }

    fn run_in(b: &[u8], p: usize, class: &[u8; 256]) -> (usize, bool, bool) {
        let n = b.len();
        let (mut q, mut y, mut escaped) = (p, false, false);
        while q < n {
            match class[usize::from(b[q])] {
                C_OTHER => {
                    y = true;
                    q += 1;
                }
                C_DIGIT | C_DOT => q += 1,
                C_ESCAPE => {
                    if q + 1 >= n {
                        // the escape runs past the limit
                        return (n + 1, y, true);
                    }
                    q += 1 + char_len(b[q + 1]);
                    (y, escaped) = (true, true);
                }
                C_HIGH => {
                    if !wide_char(b, q).is_some_and(is_nd) {
                        y = true;
                    }
                    q += char_len(b[q]);
                }
                _ => break,
            }
        }
        (q, y, escaped)
    }

    /// `\s` or the line's end at `p` (`\S`'s negation).
    fn at_break_space(&self, p: usize) -> bool {
        p >= self.b.len()
            || match self.class[usize::from(self.b[p])] {
                C_SPACE => true,
                C_HIGH => self.wide_space(p) > 0,
                _ => false,
            }
    }

    /// `comments` at `p`: where the match ends, `None` for none.
    #[inline(always)]
    fn comment(&mut self, p: usize) -> Option<usize> {
        // in place, its first byte: a head and most frames' turns try it where none is
        if !matches!(self.b[p], b';' | b'#' | b'|') {
            return None;
        }
        self.comment_at(p)
    }

    /// [`Scan::comment`] past its first byte's test.
    fn comment_at(&mut self, p: usize) -> Option<usize> {
        match (self.b[p], self.at(p + 1)) {
            (b';', _) => Some(self.line_comment(p)),
            (b'#', b'|') => Some(self.block_open(p)),
            // Scheme's `#;`: the next datum commented out, wherever a comment can be
            (b'#', b';') if self.scheme && p + 1 < self.b.len() => {
                self.put(p, p + 2, L::DatumPunct);
                self.push(DATUM);
                Some(p + 2)
            }
            (b'|', b'#') => {
                self.put(p, p + 2, L::StrayCommentEnd);
                Some(p + 2)
            }
            _ => None,
        }
    }

    fn line_comment(&mut self, p: usize) -> usize {
        let mut e = p + 1;
        while self.at(e) == b';' && e < self.b.len() {
            e += 1;
        }
        self.put(p, e, L::LineCommentPunct);
        self.put(e, self.b.len(), L::LineComment);
        self.b.len()
    }

    fn block_open(&mut self, p: usize) -> usize {
        self.put(p, p + 2, L::BlockBegin);
        self.push(BLOCK);
        p + 2
    }

    /// A block comment's text from `p`: to its end, a nested one, or the line's end.
    fn block(&mut self, p: usize) -> usize {
        let (b, n) = (self.b, self.b.len());
        let mut q = p;
        while q + 1 < n {
            match (b[q], b[q + 1]) {
                (b'|', b'#') => {
                    self.put_merged(p, q, L::Block);
                    self.put(q, q + 2, L::BlockEnd);
                    self.pop();
                    return q + 2;
                }
                (b'#', b'|') => {
                    self.put_merged(p, q, L::Block);
                    return self.block_open(q);
                }
                _ => q += 1,
            }
        }
        self.put_merged(p, n, L::Block);
        n
    }

    /// A string's text from `p`: to its end or the line's.
    fn string(&mut self, p: usize, stop: &[bool; 256]) -> usize {
        let (b, n) = (self.b, self.b.len());
        let mut from = p;
        let mut q = p;
        while q < n {
            if !stop[usize::from(b[q])] {
                q += 1;
                continue;
            }
            match b[q] {
                b'"' => {
                    self.put_merged(from, q, L::Str);
                    self.put(q, q + 1, L::StrEnd);
                    self.pop();
                    return q + 1;
                }
                b'\\' => {
                    if q + 1 >= n {
                        break;
                    }
                    let e = q + 1 + char_len(b[q + 1]);
                    self.put_merged(from, q, L::Str);
                    self.put(q, e, L::StrEscape);
                    (from, q) = (e, e);
                }
                _ => {
                    // `~`: a format directive
                    match self.directive(from, q) {
                        Some(e) => (from, q) = (e, e),
                        None => q += 1,
                    }
                }
            }
        }
        self.put_merged(from, n, L::Str);
        n
    }

    /// `string-format-mini-language` at `p`, a `~`: where it ends, its spans pushed after the
    /// string's text from `text`.
    fn directive(&mut self, text: usize, p: usize) -> Option<usize> {
        let (b, n) = (self.b, self.b.len());
        let d = digits(b, p + 1);
        // `(~)(\d*)[%&|~]`
        if matches!(self.at(d), b'%' | b'&' | b'|' | b'~') && d < n {
            self.put_merged(text, p, L::Str);
            self.put(p, p + 1, L::FmtEscapePunct);
            self.put(p + 1, d + 1, L::StrEscape);
            return Some(d + 1);
        }
        // `(~):?@?[CR]`
        let m = self.modifiers(p + 1, true);
        if matches!(self.at(m), b'C' | b'R') && m < n {
            self.put_merged(text, p, L::Str);
            self.put(p, p + 1, L::PlaceholderPunct);
            self.put(p + 1, m + 1, L::Placeholder);
            return Some(m + 1);
        }
        // the parameters' slots, a `,` each: a number (`\d*`) or a character (`(?:'.)?`)
        const N: u8 = 0;
        const C: u8 = 1;
        let forms: [(&[u8], &[u8], bool); 6] = [
            (&[N, C, C, N], b"rR", true),
            (&[C, C, N], b"bBoOdDxX", true),
            (&[N, N, C, C], b"fF", false),
            (&[N, N, N, C, C, C], b"eEgG", false),
            (&[N, N, C], b"$", true),
            (&[N, N, C], b"aAsS", true),
        ];
        for (i, (slots, letters, colon)) in forms.iter().enumerate() {
            let mut q = d;
            let mut commas = [0usize; 6];
            let mut k = 0;
            for &slot in *slots {
                if self.at(q) != b',' || q >= n {
                    break;
                }
                commas[k] = q;
                k += 1;
                q += 1;
                if slot == N {
                    q = digits(b, q);
                } else if self.at(q) == b'\'' && q + 1 < n {
                    q += 1 + char_len(b[q + 1]);
                }
            }
            let m = self.modifiers(q, *colon);
            if m < n && letters.contains(&b[m]) {
                self.put_merged(text, p, L::Str);
                self.put(p, p + 1, L::PlaceholderPunct);
                let mut at = p + 1;
                if i == 0 {
                    self.put(p + 1, d, L::PlaceholderBase);
                    at = d;
                }
                for &c in &commas[..k] {
                    self.put(at, c, L::Placeholder);
                    self.put(c, c + 1, L::PlaceholderSep);
                    at = c + 1;
                }
                self.put(at, m + 1, L::Placeholder);
                return Some(m + 1);
            }
        }
        None
    }

    /// `:?@?` at `p` (`@?` without `colon`): where it ends.
    fn modifiers(&self, mut p: usize, colon: bool) -> usize {
        if colon && self.at(p) == b':' && p < self.b.len() {
            p += 1;
        }
        if self.at(p) == b'@' && p < self.b.len() {
            p += 1;
        }
        p
    }

    /// `numbers` at `p`: where the number ends, its spans pushed.
    fn number(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let q = p + usize::from(matches!(b[p], b'+' | b'-'));
        // Scheme's `+inf.0`, `-nan.0`, `+inf.f`, …
        if self.scheme && q > p {
            for w in [
                &b"inf.0"[..],
                b"nan.0",
                b"inf.f",
                b"nan.f",
                b"inf.t",
                b"nan.t",
            ] {
                if b[q..].starts_with(w) && self.at_break(q + w.len()) {
                    self.sign(p, q);
                    self.put(q, q + w.len(), L::NumValue);
                    return Some(q + w.len());
                }
            }
        }
        let d1 = self.digits(q, 0);
        // `\d* (\.) \d+ {{exponent}}?`
        if self.at(d1) == b'.'
            && d1 < self.b.len()
            && let Some(_) = digit(b, d1 + 1)
        {
            let f = self.digits(d1 + 1, 1);
            let e = self.exponent(f).unwrap_or(f);
            if self.at_break(e) {
                self.sign(p, q);
                self.put(q, d1, L::NumValue);
                self.put(d1, d1 + 1, L::NumDot);
                self.put(d1 + 1, e, L::NumValue);
                return Some(e);
            }
        }
        if d1 == q {
            return None;
        }
        // `\d+ (?: (\.) \d* )? {{exponent}}`
        let (dot, f) = if self.at(d1) == b'.' && d1 < self.b.len() {
            (true, self.digits(d1 + 1, 1))
        } else {
            (false, d1)
        };
        if let Some(e) = self.exponent(f)
            && self.at_break(e)
        {
            self.sign(p, q);
            if dot {
                self.put(q, d1, L::NumValue);
                self.put(d1, d1 + 1, L::NumDot);
                self.put(d1 + 1, e, L::NumValue);
            } else {
                self.put(q, e, L::NumValue);
            }
            return Some(e);
        }
        // `\d+ (/) \d+`
        if self.at(d1) == b'/' && d1 < self.b.len() && digit(b, d1 + 1).is_some() {
            let r = self.digits(d1 + 1, 1);
            if self.at_break(r) {
                self.sign(p, q);
                self.put(q, d1, L::NumValue);
                self.put(d1, d1 + 1, L::NumSlash);
                self.put(d1 + 1, r, L::NumValue);
                return Some(r);
            }
        }
        // `\d+ (?: (\.)(?!\d) | {{break}} )`
        if self.at(d1) == b'.' && d1 < self.b.len() && digit(b, d1 + 1).is_none() {
            self.sign(p, q);
            self.put(q, d1, L::NumValue);
            self.put(d1, d1 + 1, L::NumDot);
            return Some(d1 + 1);
        }
        if self.at_break(d1) {
            self.sign(p, q);
            self.put(q, d1, L::NumValue);
            return Some(d1);
        }
        None
    }

    fn sign(&mut self, p: usize, q: usize) {
        if q > p {
            self.put(p, q, L::OpArithmetic);
        }
    }

    /// `{{exponent}}` at `p`: `[esfdlESFDL][-+]?\d+`.
    fn exponent(&self, p: usize) -> Option<usize> {
        if !matches!(self.at(p) | 0x20, b'e' | b's' | b'f' | b'd' | b'l') || p >= self.b.len() {
            return None;
        }
        let q = p + 1 + usize::from(matches!(self.at(p + 1), b'+' | b'-'));
        digit(self.b, q)?;
        Some(digits(self.b, q))
    }

    /// `\d*` at `p`, memoed in slot `slot` for the places of one run.
    fn digits(&mut self, p: usize, slot: usize) -> usize {
        let (from, to) = self.digits[slot];
        if p >= from && p <= to && from != usize::MAX {
            return to;
        }
        let e = digits(self.b, p);
        if e > p {
            self.digits[slot] = (p, e);
        }
        e
    }

    /// `macros` at `p`, a `#`: where the match ends, `None` for none.
    fn macros(&mut self, p: usize) -> Option<usize> {
        let (b, n) = (self.b, self.b.len());
        let d = digits(b, p + 1);
        let c = self.at(d);
        let has_c = d < n;
        // `macro-arrays`
        if has_c && c | 0x20 == b'a' {
            if self.at(d + 1) == b'(' && d + 1 < n {
                self.put(p, d + 1, L::Array);
                self.delim(d + 1, d + 2, L::ParenBegin);
                self.push(BODY);
                return Some(d + 2);
            }
            if let Some(id) = self.ident_at(d + 1) {
                self.put(p, d + 1, L::Array);
                self.put_id(d + 1, id, L::Variable);
                return Some(id.end);
            }
        }
        // `macro-characters`
        if has_c
            && c == b'\\'
            && let Some(e) = self.character(p, d)
        {
            return Some(e);
        }
        let c1 = self.at(p + 1);
        let has_c1 = p + 1 < n;
        // `macro-conditionals`
        if has_c1 && matches!(c1, b'+' | b'-') {
            self.put(p, p + 2, L::Conditional);
            if let Some(id) = self.ident_at(p + 2) {
                self.put_id(p + 2, id, L::Feature);
                return Some(id.end);
            }
            return Some(p + 2);
        }
        // `macro-functions`
        if has_c1 && matches!(c1, b'M' | b'm' | b'\'') {
            self.put(p, p + 2, L::FunctionQuote);
            return Some(self.function_name(p + 2));
        }
        // `macro-numbers`
        if let Some(e) = self.radix(p, d) {
            return Some(e);
        }
        if has_c1 && c1 | 0x20 == b'c' && self.at(p + 2) == b'(' && p + 2 < n {
            self.put(p, p + 2, L::Complex);
            self.delim(p + 2, p + 3, L::ParenBegin);
            self.push(BODY);
            return Some(p + 3);
        }
        // `macro-pathnames`
        if has_c && c | 0x20 == b'p' && self.at(d + 1) == b'"' && d + 1 < n {
            self.put(p, d + 1, L::Pathname);
            self.put(d + 1, d + 2, L::StrBegin);
            self.push(PATH);
            return Some(d + 2);
        }
        // `macro-structures`
        if has_c && c | 0x20 == b's' && self.at(d + 1) == b'(' && d + 1 < n {
            self.put(p, d + 1, L::Struct);
            self.delim(d + 1, d + 2, L::ParenBegin);
            self.push(LIST);
            self.push(MSTRUCT_NAME);
            return Some(d + 2);
        }
        // `macro-throw-exceptions`: `#[ \t\n\r)<]`, the line's `\n` past its end
        if !has_c1 {
            self.put(p, p + 1, L::Throw);
            return Some(p + 1);
        }
        if matches!(c1, b' ' | b'\t' | b'\r' | b')' | b'<') {
            self.put(p, p + 2, L::Throw);
            return Some(p + 2);
        }
        // `macro-variables`
        if matches!(c1, b'.' | b',' | b':') {
            self.put(p, p + 2, L::Unquote);
            return Some(p + 2);
        }
        if d > p + 1 {
            let s = self.skip_spaces(d);
            if self.at(s) == b'=' && s < n {
                self.put(p, p + 1, L::LocalPunct);
                self.put(p + 1, d, L::Local);
                self.put(s, s + 1, L::OpAssign);
                return Some(s + 1);
            }
            if c == b'#' && has_c {
                self.put(p, p + 1, L::RefBegin);
                self.put(p + 1, d, L::RefLocal);
                self.put(d, d + 1, L::RefEnd);
                return Some(d + 1);
            }
        }
        // `macro-vectors`
        if c == b'(' && has_c {
            self.put(p, d, L::Vector);
            self.delim(d, d + 1, L::ParenBegin);
            self.push(BODY);
            return Some(d + 1);
        }
        if c == b'*' && has_c {
            let mut e = d + 1;
            while matches!(self.at(e), b'0' | b'1') && e < n {
                e += 1;
            }
            if self.at_break(e) {
                self.put(p, d + 1, L::Vector);
                self.put(d + 1, e, L::BitVector);
                return Some(e);
            }
        }
        None
    }

    /// `macro-numbers`' `#b`, `#o`, `#x` and `#Nr` at `p` (`d`: where `#`'s digits end), in the
    /// grammar's order: binary, octal, hexadecimal (`#b` or `#2r`, …), then `#\d+r`'s letters
    /// and digits.
    fn radix(&mut self, p: usize, d: usize) -> Option<usize> {
        if d == p + 1 {
            // `#b`, `#o`, `#x`
            let base = match self.at(p + 1) | 0x20 {
                b'b' => 2,
                b'o' => 8,
                b'x' => 16,
                _ => return None,
            };
            return self.radix_value(p, p + 2, base);
        }
        if self.at(d) | 0x20 != b'r' || d >= self.b.len() {
            return None;
        }
        let base = match &self.b[p + 1..d] {
            b"2" => 2,
            b"8" => 8,
            b"16" => 16,
            _ => 0,
        };
        if base != 0
            && let Some(e) = self.radix_value(p, d + 1, base)
        {
            return Some(e);
        }
        self.radix_value(p, d + 1, 0)
    }

    /// A radix number's sign and value after its prefix `p..start`, in base `base` (0 for
    /// `[\p{L}\p{N}]`): rational, then integer.
    fn radix_value(&mut self, p: usize, start: usize, base: u8) -> Option<usize> {
        let b = self.b;
        let q = start + usize::from(matches!(self.at(start), b'+' | b'-') && start < self.b.len());
        let is = |c: u8| match base {
            2 => matches!(c, b'0' | b'1'),
            8 => matches!(c, b'0'..=b'7'),
            16 => c.is_ascii_hexdigit(),
            _ => c.is_ascii_alphanumeric(),
        };
        let run = |mut i: usize| {
            while i < b.len() {
                if is(b[i]) {
                    i += 1;
                } else if base == 0
                    && b[i] >= 0x80
                    && wide_char(b, i).is_some_and(char::is_alphanumeric)
                {
                    i += char_len(b[i]);
                } else {
                    break;
                }
            }
            i
        };
        let v = run(q);
        if v == q {
            return None;
        }
        let put_prefix = |s: &mut Self| {
            s.put(p, start, L::NumBase);
            s.sign(start, q);
        };
        if self.at(v) == b'/' && v < self.b.len() {
            let w = run(v + 1);
            if w > v + 1 && self.at_break(w) {
                put_prefix(self);
                self.put(q, v, L::NumValue);
                self.put(v, v + 1, L::NumSlash);
                self.put(v + 1, w, L::NumValue);
                return Some(w);
            }
        }
        if self.at(v) == b'.' && v < self.b.len() && run(v + 1) == v + 1 {
            put_prefix(self);
            self.put(q, v, L::NumValue);
            self.put(v, v + 1, L::NumDot);
            return Some(v + 1);
        }
        if self.at_break(v) {
            put_prefix(self);
            self.put(q, v, L::NumValue);
            return Some(v);
        }
        None
    }

    /// `macro-characters` at `p` (`d`: the `\` after `#`'s digits): where it ends.
    fn character(&mut self, p: usize, d: usize) -> Option<usize> {
        let (b, n) = (self.b, self.b.len());
        let q = d + 1;
        if q >= n {
            return None;
        }
        // where `(?:(?:[\p{L}\p{N}_]+-)+\\?)?` can end, as the regex tries them: after every
        // group and its `\`, after every group, after one group fewer, …, after none (past
        // `GROUPS` groups only the last few are tried)
        const GROUPS: usize = 16;
        let mut ends = [0usize; GROUPS];
        let mut k = 0;
        let mut group = q;
        loop {
            let w = word_chars(b, group);
            if w == group || self.at(w) != b'-' || w >= n {
                break;
            }
            group = w + 1;
            if k == GROUPS {
                ends.copy_within(1.., 0);
                k -= 1;
            }
            ends[k] = group;
            k += 1;
        }
        let mut starts = [0usize; GROUPS + 2];
        let mut m = 0;
        if k > 0 && self.at(ends[k - 1]) == b'\\' && ends[k - 1] < n {
            starts[m] = ends[k - 1] + 1;
            m += 1;
        }
        for j in (0..k).rev() {
            starts[m] = ends[j];
            m += 1;
        }
        starts[m] = q;
        m += 1;
        let names: [(&[&[u8]], L); 2] = [
            (&[b"space", b"newline"], L::CharStd),
            (
                &[
                    b"backspace",
                    b"tab",
                    b"linefeed",
                    b"page",
                    b"return",
                    b"rubout",
                ],
                L::CharSemi,
            ),
        ];
        let found = 'found: {
            for (list, kind) in names {
                for &s in &starts[..m] {
                    for w in list {
                        let e = s + w.len();
                        if e <= n && b[s..e].eq_ignore_ascii_case(w) && self.at_break(e) {
                            break 'found Some((e, kind));
                        }
                    }
                }
            }
            // a standard character: printable ASCII
            for &s in &starts[..m] {
                if s < n && (b'!'..=b'~').contains(&b[s]) && self.at_break(s + 1) {
                    break 'found Some((s + 1, L::CharStd));
                }
            }
            // `(?:\\.|[^{{break_char}}])+`: the rest of the run, a `\` ending the line in it
            let (mut e, _, _) = self.run_at_call(q);
            if e == n - 1 && b[e] == b'\\' {
                e = n;
            }
            (e > q && self.at_break(e)).then_some((e, L::CharOther))
        };
        let (e, kind) = found?;
        let k = kind as u16;
        self.out.push(p, p + 1, k + 1);
        if d > p + 1 {
            self.out.push(p + 1, d, k + 2);
        }
        self.out.push(d, q, k + 1);
        self.out.push(q, e, k);
        Some(e)
    }

    /// `macro-function-name`, right after `#'`: namespaces, then an operator, a builtin or
    /// another function.
    fn function_name(&mut self, p: usize) -> usize {
        let mut p = p;
        let mut id = self.ident_at(p);
        while let Some(q) = self.namespace(p, id) {
            p = q;
            id = self.ident_at(p);
        }
        let Some(id) = id else {
            return p;
        };
        let info = self.word(p, id);
        let kind = match words::operator(info) {
            O_COMPARISON => Some(L::OpComparison),
            O_ARITHMETIC => Some(L::OpArithmetic),
            O_LOGICAL => Some(L::OpLogical),
            O_BITWISE => Some(L::OpBitwise),
            _ if info & BUILTIN != 0 => Some(L::Builtin),
            _ => None,
        };
        if let Some(kind) = kind {
            self.put(p, id.end, kind);
            return id.end;
        }
        self.other_function(p, id, info).unwrap_or(p)
    }

    /// One turn of `loop`'s body.
    fn loop_body(&mut self, p: usize) -> usize {
        match self.b[p] {
            b')' => {
                self.delim(p, p + 1, L::ParenEnd);
                self.pop();
                return p + 1;
            }
            b'(' => {
                self.delim(p, p + 1, L::ParenBegin);
                self.push(BODY);
                self.push(HEAD);
                return p + 1;
            }
            _ => {}
        }
        let id = self.ident_at(p);
        if let Some(id) = id {
            let info = self.word(p, id);
            let kind = match loop_word(info) {
                L_NAMED => Some(L::DeclName),
                L_WITH => Some(L::DeclVar),
                L_CONTROL => Some(L::LoopKw),
                L_CONDITIONAL => Some(L::Conditional),
                L_FLOW => Some(L::Flow),
                L_RETURN => Some(L::Return),
                L_OPERATOR => Some(L::Op),
                L_INTO => Some(L::OpAssign),
                L_AND => Some(L::OpLogical),
                L_OTHER => Some(L::LoopOther),
                _ => None,
            };
            if let Some(kind) = kind {
                self.put(p, id.end, kind);
                if loop_word(info) == L_NAMED {
                    self.push(LOOP_NAME);
                }
                return id.end;
            }
        }
        // the rest of `expressions`, `identifiers` as `loop-identifier`
        match self.expression(p, true) {
            Some(q) => q,
            None => self.step(p),
        }
    }

    /// `identifiers` at `p`; in a loop's body `loop-identifier`, a builtin function first.
    fn identifiers(&mut self, p: usize, lp: bool) -> Option<usize> {
        if lp
            && let Some(id) = self.ident_at(p)
            && self.word(p, id) & BUILTIN != 0
        {
            self.put(p, id.end, L::Builtin);
            return Some(id.end);
        }
        self.identifier(p)
    }

    /// One turn of a parameter list.
    fn params(&mut self, p: usize) -> usize {
        match self.b[p] {
            c if self.closes(c) => {
                self.delim(p, p + 1, L::ParamsEnd);
                self.pop();
                p + 1
            }
            c if self.opens(c) => {
                self.delim(p, p + 1, L::ParenBegin);
                self.push(BODY);
                self.push(PARAM_NAME);
                p + 1
            }
            _ => {
                if let Some(q) = self.param_prefix(p) {
                    return q;
                }
                match self.ident_at(p) {
                    Some(id) => {
                        self.put_id(p, id, L::Parameter);
                        id.end
                    }
                    None => self.step(p),
                }
            }
        }
    }

    /// `comments`, `annotations`, `keywords` and `modifiers` at `p`, as a parameter list tries
    /// them: where the match ends.
    fn param_prefix(&mut self, p: usize) -> Option<usize> {
        if let Some(q) = self.comment(p) {
            return Some(q);
        }
        self.annotation_keyword_modifier(p)
    }

    /// `annotations`, `keywords` and `modifiers` at `p`.
    fn annotation_keyword_modifier(&mut self, p: usize) -> Option<usize> {
        match self.b[p] {
            b'#' if self.scheme && self.at(p + 1) == b':' => self.hash_keyword(p),
            b'&' => self.annotation(p),
            b':' => self.keyword(p),
            b'.' if self.at_break(p + 1) => {
                self.put(p, p + 1, L::Control);
                Some(p + 1)
            }
            b'`' | b'\'' => {
                self.put(p, p + 1, L::Quoted);
                Some(p + 1)
            }
            b',' => {
                let e = p + 1 + usize::from(self.at(p + 1) == b'@');
                self.put(p, e, L::Unquote);
                Some(e)
            }
            _ => None,
        }
    }

    /// One turn of a class's parent list.
    fn parents(&mut self, p: usize) -> usize {
        let id = self.ident_at(p);
        if let Some(q) = self.namespace(p, id) {
            return q;
        }
        if let Some(id) = id {
            self.put_id(p, id, L::Inherited);
            return id.end;
        }
        if let Some(q) = self.comment(p) {
            return q;
        }
        if self.b[p] == b')' {
            self.delim(p, p + 1, L::ParenEnd);
            self.pop();
            return p + 1;
        }
        self.step(p)
    }

    /// One turn of `let`'s bindings, `flet`'s functions or `macrolet`'s macros.
    fn bindings(&mut self, p: usize, top: u8) -> usize {
        match self.b[p] {
            c if self.opens(c) => {
                self.delim(p, p + 1, L::ParenBegin);
                self.push(BODY);
                match top {
                    VARS => self.push(VAR_NAME),
                    FLETS => {
                        self.push(FN_PARAMS);
                        self.push(FN_NAME);
                    }
                    _ => {
                        self.push(MACRO_PARAMS);
                        self.push(MACRO_NAME);
                    }
                }
                return p + 1;
            }
            c if self.closes(c) => {
                self.delim(p, p + 1, L::ParenEnd);
                self.pop();
                return p + 1;
            }
            _ => {}
        }
        if top == VARS
            && let Some(id) = self.ident_at(p)
        {
            self.put_id(p, id, L::Variable);
            return id.end;
        }
        match self.comment(p) {
            Some(q) => q,
            None => self.step(p),
        }
    }

    /// A list's head at `p` in Scheme and Racket: heads of the scanner's own design, the
    /// standard libraries' procedures, then any function.
    fn scheme_head(&mut self, p: usize) -> usize {
        if let Some(id) = self.ident_at(p) {
            let info = self.word(p, id);
            let w = &self.b[p..id.end];
            let wordy = !id.quoted && !id.escaped && self.at_break(id.end);
            // Racket's `for/list`, `for*/fold`, …
            let h = if wordy && (w.starts_with(b"for/") || w.starts_with(b"for*/")) {
                S_LOOP
            } else {
                scheme_head(info)
            };
            if h != 0 {
                self.scheme_declaration(p, id.end, h);
                return id.end;
            }
            match scheme_ident(info) {
                // `cond`'s `[else …]`, `case`'s `[(a) => f]`
                S_KEYWORD => {
                    self.put(p, id.end, L::Conditional);
                    self.pop();
                    return id.end;
                }
                // a constant is no call: the body reads it
                S_CONSTANT => {
                    self.pop();
                    return p;
                }
                _ => {}
            }
            if info & S_BUILTIN != 0 {
                self.put(p, id.end, L::Builtin);
                self.pop();
                return id.end;
            }
            if let Some(e) = self.other_function(p, id, 0) {
                self.pop();
                return e;
            }
        }
        match self.comment(p) {
            Some(q) => q,
            None => {
                self.pop();
                p
            }
        }
    }

    /// A Scheme head's word `h` at `p..e`: its kind, and what the list's elements are after it.
    fn scheme_declaration(&mut self, p: usize, e: usize, h: u32) {
        let (kind, below, next): (L, u8, &[u8]) = match h {
            S_DEFINE => (L::DeclFn, BODY, &[S_DEFINE_NAME]),
            S_DEFINE_SYNTAX => (L::DeclMacro, BODY, &[S_SYNTAX_NAME]),
            S_DEFINE_VALUES => (L::DeclVar, BODY, &[VAR_LIST]),
            S_RECORD => (L::DeclStruct, BODY, &[STRUCT_NAME]),
            S_LAMBDA => (L::DeclFnInline, BODY, &[S_PARAMS]),
            S_CASE_LAMBDA => (L::DeclFnInline, CLAUSES, &[]),
            S_LET => (L::DeclVar, BODY, &[S_LET_NAME]),
            S_DO => (L::LoopKw, BODY, &[EXPECT_LISTS]),
            S_CONDITIONAL => (L::Conditional, BODY, &[]),
            S_LOOP => (L::LoopKw, BODY, &[]),
            S_CONTROL => (L::Control, BODY, &[]),
            S_LOGICAL => (L::OpLogical, BODY, &[]),
            S_MODULE => (L::Import, BODY, &[]),
            S_ARITHMETIC => (L::OpArithmetic, BODY, &[]),
            S_COMPARISON => (L::OpComparison, BODY, &[]),
            _ => (L::Builtin, QUOTED, &[]),
        };
        self.put(p, e, kind);
        if below != BODY {
            self.set_below(below);
        }
        self.pop();
        for &f in next {
            self.push(f);
        }
    }

    /// One turn of Scheme's one-shot frames at `p`.
    fn scheme_pending(&mut self, p: usize, top: u8) -> usize {
        let c = self.b[p];
        let opens = self.opens(c);
        match top {
            DATUM => return self.datum(p),
            // `(define name …)` or `(define (name param …) …)`, curried `(define ((name a) b) …)`
            S_DEFINE_NAME | S_SYNTAX_NAME | S_FN_NAME | S_MACRO_NAME => {
                let syntax = matches!(top, S_SYNTAX_NAME | S_MACRO_NAME);
                if opens {
                    self.delim(p, p + 1, L::ParamsBegin);
                    self.set(PARAMS);
                    self.push(if syntax { S_MACRO_NAME } else { S_FN_NAME });
                    return p + 1;
                }
                if let Some(id) = self.ident_at(p) {
                    let kind = match top {
                        S_DEFINE_NAME => L::VarName,
                        S_FN_NAME => L::FnName,
                        _ => L::MacroName,
                    };
                    self.put_id(p, id, kind);
                    self.pop();
                    return id.end;
                }
            }
            // a list of parameters, or one name taking them all
            S_PARAMS => {
                if opens {
                    self.delim(p, p + 1, L::ParamsBegin);
                    self.set(PARAMS);
                    return p + 1;
                }
                if let Some(id) = self.ident_at(p) {
                    self.put_id(p, id, L::Parameter);
                    self.pop();
                    return id.end;
                }
            }
            // a named `let`'s name, then the bindings
            _ => {
                if opens {
                    self.delim(p, p + 1, L::ParenBegin);
                    self.set(VARS);
                    return p + 1;
                }
                if let Some(id) = self.ident_at(p) {
                    self.put_id(p, id, L::FnName);
                    self.set(VAR_LIST);
                    return id.end;
                }
            }
        }
        match self.comment(p) {
            Some(q) => q,
            None => {
                self.pop();
                p
            }
        }
    }

    /// Scheme's and Racket's `#` forms at `p` (before Common Lisp's): where the match ends.
    fn scheme_hash(&mut self, p: usize) -> Option<usize> {
        let (b, n) = (self.b, self.b.len());
        let rest = &b[p + 1..];
        let c1 = self.at(p + 1);
        // `#t`, `#f`, `#true`, `#false`
        for w in [&b"t"[..], b"f", b"true", b"false"] {
            if rest.starts_with(w) && self.at_break(p + 1 + w.len()) {
                let e = p + 1 + w.len();
                self.put(p, e, L::Boolean);
                return Some(e);
            }
        }
        match c1 {
            // `#;`: the next datum commented out
            b';' => {
                self.put(p, p + 2, L::DatumPunct);
                self.push(DATUM);
                Some(p + 2)
            }
            b':' => self.hash_keyword(p),
            // Typed Racket's `#{x : T}`: code, a list
            b'{' if self.racket => {
                self.put(p, p + 1, L::Vector);
                self.delim(p + 1, p + 2, L::ParenBegin);
                self.push(LIST);
                Some(p + 2)
            }
            // Guile's `#{any symbol}#`, Chez's gensym `#{name unique}`: to the first `}`, a `#`
            // after it Guile's; across lines while no `}` comes
            b'{' => {
                let Some(i) = b[p + 2..].iter().position(|&c| c == b'}') else {
                    // none closes on the line: the symbol goes on to the next
                    self.put(p, p + 2, L::VariableBegin);
                    self.put(p + 2, n, L::Variable);
                    self.push(XSYMBOL);
                    return Some(n);
                };
                let q = p + 2 + i;
                let e = q + 1 + usize::from(self.at(q + 1) == b'#' && q + 1 < n);
                self.put(p, p + 2, L::VariableBegin);
                self.put(p + 2, q, L::Variable);
                self.put(q, e, L::VariableEnd);
                Some(e)
            }
            b'"' => {
                self.put(p, p + 1, L::StringPrefix);
                self.put(p + 1, p + 2, L::StrBegin);
                self.push(STRING);
                Some(p + 2)
            }
            b'&' => {
                self.put(p, p + 2, L::Unquote);
                Some(p + 2)
            }
            // syntax quotes, as quotes
            b'\'' | b'`' => {
                self.put(p, p + 2, L::Quoted);
                Some(self.quoted_content(p + 2))
            }
            b',' => {
                let e = p + 2 + usize::from(self.at(p + 2) == b'@');
                self.put(p, e, L::Unquote);
                Some(e)
            }
            _ => {
                // `#rx"…"`, `#px"…"` and their byte forms `#rx#"…"`
                for pre in [&b"rx\""[..], b"px\"", b"rx#\"", b"px#\""] {
                    if rest.starts_with(pre) {
                        let q = p + pre.len();
                        self.put(p, q, L::StringPrefix);
                        self.put(q, q + 1, L::RegexBegin);
                        self.push(REGEX);
                        self.regex = 0;
                        return Some(q + 1);
                    }
                }
                // `#hash(…)` and kin, `#u8(…)`, `#vu8(…)`: no head in them
                for pre in [
                    &b"hash"[..],
                    b"hasheq",
                    b"hasheqv",
                    b"hashalw",
                    b"u8",
                    b"vu8",
                ] {
                    let q = p + 1 + pre.len();
                    if rest.starts_with(pre) && q < n && self.opens(b[q]) {
                        self.put(p, q, L::Vector);
                        self.delim(q, q + 1, L::ParenBegin);
                        self.push(LIST);
                        return Some(q + 1);
                    }
                }
                // `#lang racket/base`
                if rest.starts_with(b"lang") && self.at_break(p + 5) {
                    self.put(p, p + 5, L::Decl);
                    let s = self.skip_spaces(p + 5);
                    let (e, _, _) = self.run_at(s);
                    self.put(s, e, L::ModuleName);
                    return Some(e.max(p + 5));
                }
                // `#!/usr/bin/env racket` a line's comment; `#!r6rs`, `#!fold-case` directives
                if c1 == b'!' && p + 1 < n {
                    if p == 0 && self.at(2) == b'/' {
                        self.put(0, n, L::LineComment);
                        return Some(n);
                    }
                    let (e, _, _) = self.run_at(p + 2);
                    self.put(p, e, L::Decl);
                    return Some(e);
                }
                // `#e`, `#i`, `#d`: a number's exactness or radix
                if matches!(c1 | 0x20, b'e' | b'i' | b'd') && p + 2 < n {
                    let mark = self.out.len();
                    self.put(p, p + 2, L::NumBase);
                    let q = if self.at(p + 2) == b'#' {
                        self.radix(p + 2, digits(b, p + 3))
                    } else {
                        self.number(p + 2)
                    };
                    if q.is_none() {
                        self.out.truncate(mark);
                    }
                    return q;
                }
                None
            }
        }
    }

    /// A `#{…}` symbol's text from `p`, on a line after its `#{`: to its `}` (a `#` after it
    /// Guile's) or the line's end.
    fn xsymbol_text(&mut self, p: usize) -> usize {
        let (b, n) = (self.b, self.b.len());
        match b[p..].iter().position(|&c| c == b'}') {
            Some(i) => {
                let q = p + i;
                let e = q + 1 + usize::from(self.at(q + 1) == b'#' && q + 1 < n);
                self.put(p, q, L::Variable);
                self.put(q, e, L::VariableEnd);
                self.pop();
                e
            }
            None => {
                self.put(p, n, L::Variable);
                n
            }
        }
    }

    /// Racket's `#:keyword` at `p`.
    fn hash_keyword(&mut self, p: usize) -> Option<usize> {
        let id = self.ident_at(p + 2)?;
        self.put(p, p + 2, L::KeywordMark);
        self.put_id(p + 2, id, L::Keyword);
        Some(id.end)
    }

    /// After `#;`, at `p`: the datum it comments out, as a block comment. Its prefixes (`'`,
    /// `` ` ``, `,`, `#`) stay with it; a list or a string is a frame, the rest one token.
    fn datum(&mut self, p: usize) -> usize {
        let (b, n) = (self.b, self.b.len());
        let c = b[p];
        if self.opens(c) {
            self.put_merged(p, p + 1, L::Block);
            self.set(DATUM_LIST);
            return p + 1;
        }
        match c {
            b'"' => {
                self.put_merged(p, p + 1, L::Block);
                self.set(DATUM_STRING);
                return p + 1;
            }
            b';' => return self.line_comment(p),
            b'\'' | b'`' | b',' | b'@' => {
                self.put_merged(p, p + 1, L::Block);
                return p + 1;
            }
            b'#' if p + 1 < n && !matches!(b[p + 1], b'\\' | b't' | b'f') => {
                if b[p + 1] == b'|' {
                    return self.block_open(p);
                }
                self.put_merged(p, p + 1, L::Block);
                return p + 1;
            }
            _ => {}
        }
        // a token: `#\(` and other characters whole, else a run
        let from = if c == b'#' && self.at(p + 1) == b'\\' && p + 2 < n {
            p + 2 + char_len(b[p + 2])
        } else {
            p
        };
        let (e, _, _) = self.run_at_call(from);
        let e = e.max(p + char_len(c)).min(n);
        self.put_merged(p, e, L::Block);
        self.pop();
        e
    }

    /// A commented-out list's or string's text from `p`, as a block comment: to the frame's end,
    /// an inner list or string, or the line's end.
    fn datum_text(&mut self, p: usize, top: u8) -> usize {
        let (b, n) = (self.b, self.b.len());
        let mut q = p;
        while q < n {
            let c = b[q];
            if top == DATUM_STRING {
                match c {
                    b'\\' => q = (q + 2).min(n),
                    b'"' => {
                        self.put_merged(p, q + 1, L::Block);
                        self.pop();
                        return q + 1;
                    }
                    _ => q += 1,
                }
                continue;
            }
            match c {
                b'"' => {
                    self.put_merged(p, q + 1, L::Block);
                    self.push(DATUM_STRING);
                    return q + 1;
                }
                b';' => {
                    self.put_merged(p, q, L::Block);
                    return self.line_comment(q);
                }
                b'#' if self.at(q + 1) == b'\\' => q = (q + 3).min(n),
                b'\\' => q = (q + 2).min(n),
                c if self.opens(c) => {
                    self.put_merged(p, q + 1, L::Block);
                    self.push(DATUM_LIST);
                    return q + 1;
                }
                c if self.closes(c) => {
                    self.put_merged(p, q + 1, L::Block);
                    self.pop();
                    return q + 1;
                }
                _ => q += 1,
            }
        }
        self.put_merged(p, n, L::Block);
        n
    }

    /// A `#rx"…"`'s or `#px"…"`'s regex from `p`: the string's `"` and escapes (a `\\` is the
    /// regex's `\`), else the regex to the next place they can be.
    fn regex_text(&mut self, p: usize) -> usize {
        let (b, n) = (self.b, self.b.len());
        if p >= n {
            return n;
        }
        if b[p] == b'"' {
            self.put(p, p + 1, L::RegexEnd);
            self.pop();
            return p + 1;
        }
        if b[p] == b'\\' && self.at(p + 1) != b'\\' {
            if p + 1 >= n {
                self.put_merged(p, n, L::Regex);
                return n;
            }
            let e = crate::regex::java_escape(b, p).min(n);
            self.put(p, e, L::RegexEscape);
            return e;
        }
        let mut at = p;
        match crate::regex::embedded(b, p, self.regex, &REGEX_EMBED, &mut at, self.out) {
            Some((q, st)) => {
                self.regex = st;
                if q > n { n } else { q.max(p + 1) }
            }
            None => n,
        }
    }

    /// One turn of a one-shot frame `top` at `p`.
    fn pending(&mut self, p: usize, top: u8) -> usize {
        let c = self.b[p];
        let opens = self.opens(c);
        if (S_DEFINE_NAME..=DATUM).contains(&top) {
            return self.scheme_pending(p, top);
        }
        match top {
            FN_NAME | MACRO_NAME | TYPE_NAME | STRUCT_QNAME | STRUCT_NAME => {
                if top == STRUCT_NAME && opens {
                    self.delim(p, p + 1, L::ParenBegin);
                    self.set(BODY);
                    self.push(STRUCT_QNAME);
                    return p + 1;
                }
                let kind = match top {
                    FN_NAME => L::FnName,
                    MACRO_NAME => L::MacroName,
                    TYPE_NAME => L::TypeName,
                    _ => L::StructName,
                };
                let id = self.ident_at(p);
                if let Some(q) = self.namespace(p, id) {
                    return q;
                }
                if let Some(id) = id {
                    self.put_id(p, id, kind);
                    self.pop();
                    return id.end;
                }
            }
            CLASS_NAME | CONST_NAME | VAR_NAME | LOOP_NAME | MSTRUCT_NAME => {
                if let Some(id) = self.ident_at(p) {
                    let kind = match top {
                        CLASS_NAME => L::ClassName,
                        CONST_NAME => L::Const,
                        VAR_NAME => L::Variable,
                        LOOP_NAME => L::LoopName,
                        _ => L::StructName,
                    };
                    self.put_id(p, id, kind);
                    self.pop();
                    return id.end;
                }
                if top == MSTRUCT_NAME {
                    self.pop();
                    return p;
                }
            }
            QUALIFIER => {
                if !matches!(c, b'(' | b')')
                    && let Some(q) = self.expression(p, false)
                {
                    return q;
                }
                self.pop();
                return p;
            }
            DECLARE_ID => {
                if let Some(id) = self.ident_at(p)
                    && self.word(p, id) & words::DECLARE_ID != 0
                {
                    self.put(p, id.end, L::Modifier);
                    self.pop();
                    return id.end;
                }
            }
            PARAM_NAME => {
                if let Some(q) = self.annotation_keyword_modifier(p) {
                    return q;
                }
                if let Some(id) = self.ident_at(p) {
                    self.put_id(p, id, L::Parameter);
                    self.pop();
                    return id.end;
                }
            }
            DECLARE_SPECS => {
                if opens {
                    self.delim(p, p + 1, L::ParenBegin);
                    self.push(BODY);
                    self.push(DECLARE_ID);
                    return p + 1;
                }
            }
            _ => {
                if opens {
                    let (kind, f) = match top {
                        FN_PARAMS | MACRO_PARAMS | TYPE_PARAMS => (L::ParamsBegin, PARAMS),
                        CLASS_PARENTS => (L::ParenBegin, PARENTS),
                        EXPECT_LIST => (L::ParenBegin, BODY),
                        EXPECT_LISTS => (L::ParenBegin, LIST),
                        FLET_LIST => (L::ParenBegin, FLETS),
                        MACROLET_LIST => (L::ParenBegin, MACROLETS),
                        _ => (L::ParenBegin, VARS),
                    };
                    self.delim(p, p + 1, kind);
                    self.set(f);
                    return p + 1;
                }
            }
        }
        match self.comment(p) {
            Some(q) => q,
            None => {
                // `else-pop`
                self.pop();
                p
            }
        }
    }
}

/// The state that keeps `frames` (bottom first) over `base` plain bodies below them, `body` the
/// frame a run counts (the engine's list body, Clojure's list tail): a code a frame, a run of
/// `body` frames a code (`RUN` and up to 127 of them), the top [`KEPT`] codes kept and the bottom
/// dropped past them (inexact then); `arg` beside them. The codes go through a ring of `KEPT`, so
/// no buffer of every code is cleared a line.
#[inline(always)]
pub(super) fn keep(frames: &[u8], body: u8, base: usize, exact: bool, arg: u16) -> State {
    // most lines: no bodies below the stack, and frames enough for the state's codes, so the
    // codes are written in order (a run's code stands for one frame or more), no ring
    if base == 0 && frames.len() <= KEPT {
        let mut codes = [0u8; KEPT];
        let (mut k, mut run) = (0, 0u8);
        for &f in frames {
            if f == body {
                run += 1;
                continue;
            }
            if run > 0 {
                codes[k] = RUN | run;
                k += 1;
                run = 0;
            }
            codes[k] = f;
            k += 1;
        }
        if run > 0 {
            codes[k] = RUN | run;
            k += 1;
        }
        let mut state = State {
            depth: k as u8,
            inexact: !exact,
            arg,
            ..State::default()
        };
        let (levels, withs) = codes.split_at(crate::matcher::KEPT);
        state.levels.copy_from_slice(levels);
        state.withs.copy_from_slice(withs);
        return state;
    }
    struct Ring {
        codes: [u8; KEPT],
        /// Codes written.
        k: usize,
        /// The next code's place.
        w: usize,
    }
    impl Ring {
        #[inline(always)]
        fn put(&mut self, c: u8) {
            self.codes[self.w] = c;
            self.w = if self.w + 1 == KEPT { 0 } else { self.w + 1 };
            self.k += 1;
        }
        #[inline(always)]
        fn flush(&mut self, run: &mut usize) {
            while *run > 0 {
                let n = (*run).min(0x7f);
                self.put(RUN | n as u8);
                *run -= n;
            }
        }
    }
    let mut ring = Ring {
        codes: [0; KEPT],
        k: 0,
        w: 0,
    };
    // the bodies below the stack: past the state's bytes of runs, the bottom drops anyway
    let mut run = base.min(0x7f * KEPT);
    for &f in frames {
        if f == body {
            run += 1;
            continue;
        }
        ring.flush(&mut run);
        ring.put(f);
    }
    ring.flush(&mut run);
    let kept = ring.k.min(KEPT);
    let mut state = State {
        depth: kept as u8,
        inexact: !exact || ring.k > KEPT || base > 0x7f * KEPT,
        arg,
        ..State::default()
    };
    // the oldest code kept: the ring's start until it wraps, then the next place
    let mut r = if ring.k > KEPT { ring.w } else { 0 };
    for i in 0..kept {
        state.set_level(i, ring.codes[r]);
        r = if r + 1 == KEPT { 0 } else { r + 1 };
    }
    state
}

/// Of the sixteen bytes of `b` at `q` (`q + 16 <= b.len()`), a bit for each an identifier's run
/// takes as it is (an ASCII byte but a space, a break and `\`; [`CLASS`], Scheme's [`SCHEME_CLASS`]
/// with `scheme`), and a bit for each digit and `.`.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn plain_masks(b: &[u8], q: usize, scheme: bool) -> (u32, u32) {
    use std::arch::x86_64::*;
    assert!(q + 16 <= b.len());
    // SAFETY: SSE2 is part of x86-64; the load reads `b[q..q + 16]`, inside `b` (above), and
    // needs no alignment
    unsafe {
        let w = _mm_loadu_si128(b.as_ptr().add(q).cast());
        let eq = |x: __m128i, c: u8| _mm_cmpeq_epi8(x, _mm_set1_epi8(c as i8));
        // `x - lo` at most `n`, unsigned: `x` in `lo..=lo + n`
        let within = |x: __m128i, lo: u8, n: u8| {
            let d = _mm_sub_epi8(x, _mm_set1_epi8(lo as i8));
            _mm_cmpeq_epi8(_mm_min_epu8(d, _mm_set1_epi8(n as i8)), d)
        };
        let or = |a, b| _mm_or_si128(a, b);
        // `| 0x20` takes `\` to `|`, `[` to `{` and `]` to `}`, and no other byte to those
        let up = _mm_or_si128(w, _mm_set1_epi8(0x20));
        // `\t`..`\r` and ` `; `(` `)`, `:` `;`, `"`, `'`, `,`; `|` and `\`
        let space = or(within(w, b'\t', 4), eq(w, b' '));
        let breaks = or(
            or(within(w, b'(', 1), within(w, b':', 1)),
            or(or(eq(w, b'"'), eq(w, b'\'')), eq(w, b',')),
        );
        let mut stop = or(or(space, breaks), eq(up, b'|'));
        if scheme {
            stop = or(stop, or(eq(up, b'{'), eq(up, b'}')));
        }
        // bytes past ASCII stop too: their top bit
        let stop = (_mm_movemask_epi8(or(stop, w)) as u32) & 0xffff;
        let digits = or(within(w, b'0', 9), eq(w, b'.'));
        (!stop & 0xffff, _mm_movemask_epi8(digits) as u32)
    }
}

/// The frames `state`'s codes ([`keep`]) stand for, bottom first, written to `stack` (a run's
/// code as that many `body` frames): their count, or `None` where they're more than half the
/// stack (the caller's slow path leaves the bottom ones below it).
#[inline(always)]
fn expand(state: &State, body: u8, stack: &mut [u8]) -> Option<usize> {
    const { assert!(KEPT == 2 * crate::matcher::KEPT) };
    let mut codes = [0u8; KEPT];
    let (levels, withs) = codes.split_at_mut(crate::matcher::KEPT);
    levels.copy_from_slice(&state.levels);
    withs.copy_from_slice(&state.withs);
    let codes = &codes[..usize::from(state.depth).min(KEPT)];
    let count = |c: u8| {
        if c & RUN != 0 {
            usize::from(c & !RUN)
        } else {
            1
        }
    };
    if codes.iter().map(|&c| count(c)).sum::<usize>() > MAX / 2 {
        return None;
    }
    let mut d = 0;
    for &c in codes {
        if c & RUN != 0 {
            // eight frames a store: the bytes past the run are above the stack's top, never read
            // before a push writes them (`d` stays at most half the stack, so a store fits)
            let mut k = usize::from(c & !RUN);
            while k > 0 {
                stack[d..d + 8].fill(body);
                let step = k.min(8);
                d += step;
                k -= step;
            }
        } else {
            stack[d] = c;
            d += 1;
        }
    }
    Some(d)
}

/// Where a run of ` ` at `p` ends, eight bytes a step while eight are left: a line's
/// indentation (10–12 bytes on average in the corpora), which a byte loop walks a byte a turn.
/// The bytes after it are the caller's: a tab, the line's last few, a space past ASCII.
#[inline(always)]
fn blanks(b: &[u8], mut p: usize) -> usize {
    const SPACES: u64 = u64::from_le_bytes([b' '; 8]);
    while let Some(w) = b.get(p..p + 8) {
        let x = u64::from_le_bytes(w.try_into().unwrap()) ^ SPACES;
        if x != 0 {
            // the first byte that isn't a space: the lowest nonzero byte
            return p + (x.trailing_zeros() / 8) as usize;
        }
        p += 8;
    }
    p
}

/// `\d*` at `p`: where it ends.
fn digits(b: &[u8], mut p: usize) -> usize {
    while let Some(e) = digit(b, p) {
        p = e;
    }
    p
}

/// `[\p{L}\p{N}_]*` at `p`: where it ends.
fn word_chars(b: &[u8], mut p: usize) -> usize {
    while p < b.len() {
        let c = b[p];
        if c.is_ascii_alphanumeric() || c == b'_' {
            p += 1;
        } else if c >= 0x80 && wide_char(b, p).is_some_and(char::is_alphanumeric) {
            p += char_len(c);
        } else {
            break;
        }
    }
    p
}

#[cfg(test)]
mod tests;
