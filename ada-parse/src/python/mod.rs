//! Python by a scanner of ada's own (ADR-40, speed first): bat's `Python` grammar (243
//! contexts, with bat's patch) and the two it embeds, `SQL` in a string that starts with a
//! SQL keyword and `Regular Expressions (Python)` in a raw one, compiled by hand as Rust's,
//! Go's and C#'s were. Each context that can be open is an arm of one `match` on the top of a
//! stack, trying its patterns in the grammar's order at the place the scan has reached, which
//! is syntect's leftmost match; where none can start, the scan steps on.
//!
//! syntect's machinery is followed where the grammar leans on it: **the loop guard** (a pop
//! that matches nothing right after a push that matched nothing there, at the depth that push
//! left, is passed over for the context's later patterns, else the scan steps a character);
//! **`with_prototype`**: the embedded contexts try their string's patterns first (its end, its
//! escapes, its placeholders, an f-string's replacements), and a context a prototype's pattern
//! pushes is out of its reach (`proto_starts`); `set` gives its token the old context's scopes
//! and the new one's; the line's `\n` is a space the patterns see.
//!
//! A kind is a leaf (`L`: a token's scopes, bat's, meta scopes dropped but `meta.annotation`)
//! with what the stack around it adds that themes key on: `meta.annotation` (a decorator), the
//! string or docstring it is in (`HOSTS`), and one layer in that (`INNERS`: a regex's
//! character class or comment, a SQL string or comment, a placeholder or a format spec).
//!
//! The stack between lines is the contexts open above the top level, up to 28 (the bytes of
//! [`State`]'s levels and withs).
use crate::prim::{
    WORD, char_len, digit, is_blank, is_name_start, is_nd, is_word, skip_spaces, space_len,
    starts_word, stops, wide_char, word_end,
};

mod regex;
mod strings;
mod words;

use crate::emit::{Push, Rules, Run, Runs, Sink};
use crate::{Out, State};

macro_rules! leaves {
    ($($name:ident = $scope:literal,)*) => {
        /// The leaves: a token's scopes, past what the stack adds.
        #[allow(dead_code)]
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        #[repr(u16)]
        enum L { $($name,)* }
        static LEAVES: &[&str] = &[$($scope,)*];
    };
}

leaves! {
    Plain = "",
    // names, in groups in the order of `words::GENERIC` … `words::LANGUAGE`, then a constant's
    // name: bare, then in a
    // call's name, a decorator's, a decorator call's, a class's and a function's
    Name = "",
    NameBuiltinFunction = "support.function.builtin.python",
    NameBuiltinType = "support.type.python",
    NameException = "support.type.exception.python",
    NameIllegal = "invalid.illegal.name.python",
    NameMagicFunction = "support.function.magic.python",
    NameMagicVariable = "support.variable.magic.python",
    NameLanguage = "variable.language.python",
    NameConstant = "variable.other.constant.python",
    Call = "variable.function.python",
    CallBuiltinFunction = "variable.function.python support.function.builtin.python",
    CallBuiltinType = "variable.function.python support.type.python",
    CallException = "variable.function.python support.type.exception.python",
    CallIllegal = "variable.function.python invalid.illegal.name.python",
    CallMagicFunction = "variable.function.python support.function.magic.python",
    CallMagicVariable = "variable.function.python support.variable.magic.python",
    CallLanguage = "variable.function.python variable.language.python",
    CallConstant = "variable.function.python variable.other.constant.python",
    Ann = "variable.annotation.python",
    AnnBuiltinFunction = "variable.annotation.python support.function.builtin.python",
    AnnBuiltinType = "variable.annotation.python support.type.python",
    AnnException = "variable.annotation.python support.type.exception.python",
    AnnIllegal = "variable.annotation.python invalid.illegal.name.python",
    AnnMagicFunction = "variable.annotation.python support.function.magic.python",
    AnnMagicVariable = "variable.annotation.python support.variable.magic.python",
    AnnLanguage = "variable.annotation.python variable.language.python",
    AnnConstant = "variable.annotation.python variable.other.constant.python",
    AnnCall = "variable.annotation.function.python",
    AnnCallBuiltinFunction = "variable.annotation.function.python support.function.builtin.python",
    AnnCallBuiltinType = "variable.annotation.function.python support.type.python",
    AnnCallException = "variable.annotation.function.python support.type.exception.python",
    AnnCallIllegal = "variable.annotation.function.python invalid.illegal.name.python",
    AnnCallMagicFunction = "variable.annotation.function.python support.function.magic.python",
    AnnCallMagicVariable = "variable.annotation.function.python support.variable.magic.python",
    AnnCallLanguage = "variable.annotation.function.python variable.language.python",
    AnnCallConstant = "variable.annotation.function.python variable.other.constant.python",
    ClassName = "entity.name.class.python",
    ClassBuiltinFunction = "entity.name.class.python support.function.builtin.python",
    ClassBuiltinType = "entity.name.class.python support.type.python",
    ClassException = "entity.name.class.python support.type.exception.python",
    ClassIllegal = "entity.name.class.python invalid.illegal.name.python",
    ClassMagicFunction = "entity.name.class.python support.function.magic.python",
    ClassMagicVariable = "entity.name.class.python support.variable.magic.python",
    ClassLanguage = "entity.name.class.python variable.language.python",
    ClassConstant = "entity.name.class.python variable.other.constant.python",
    DefName = "entity.name.function.python",
    DefBuiltinFunction = "entity.name.function.python support.function.builtin.python",
    DefBuiltinType = "entity.name.function.python support.type.python",
    DefException = "entity.name.function.python support.type.exception.python",
    DefIllegal = "entity.name.function.python invalid.illegal.name.python",
    DefMagicFunction = "entity.name.function.python support.function.magic.python",
    DefMagicVariable = "entity.name.function.python support.variable.magic.python",
    DefLanguage = "entity.name.function.python variable.language.python",
    DefConstant = "entity.name.function.python variable.other.constant.python",
    // comments
    Comment = "comment.line.number-sign.python",
    CommentMark = "comment.line.number-sign.python punctuation.definition.comment.python",
    // statements
    Raise = "keyword.control.flow.raise.python",
    RaiseFrom = "keyword.control.flow.raise.from.python",
    Assert = "keyword.control.flow.assert.python",
    Del = "keyword.other.del.python",
    Print = "keyword.other.print.python",
    Exec = "keyword.other.exec.python",
    Return = "keyword.control.flow.return.python",
    Break = "keyword.control.flow.break.python",
    Continue = "keyword.control.flow.continue.python",
    Pass = "keyword.control.flow.pass.python",
    Import = "keyword.control.import.python",
    ImportFrom = "keyword.control.import.from.python",
    ImportAs = "keyword.control.import.as.python",
    ImportRelative = "keyword.control.import.relative.python",
    ImportSeparator = "punctuation.separator.import-list.python",
    ImportUnexpectedRelative = "invalid.illegal.unexpected-relative-import.python",
    ImportListBegin = "punctuation.section.import-list.begin.python",
    ImportListEnd = "punctuation.section.import-list.end.python",
    ImportAll = "constant.language.import-all.python",
    ImportIllegal = "invalid.illegal.name.import.python",
    Dot = "punctuation.accessor.dot.python",
    Async = "storage.modifier.async.python",
    For = "keyword.control.loop.for.python",
    ForIn = "keyword.control.loop.for.in.python",
    ForBlock = "punctuation.section.block.loop.for.python",
    MissingIn = "invalid.illegal.missing-in.python",
    With = "keyword.control.flow.with.python",
    WithAs = "keyword.control.flow.with.as.python",
    WithBlock = "punctuation.section.block.with.python",
    WithSeparator = "punctuation.separator.with-resources.python",
    Except = "keyword.control.exception.catch.python",
    ExceptBlock = "punctuation.section.block.exception.catch.python",
    ExceptAs = "keyword.control.exception.catch.as.python",
    If = "keyword.control.conditional.if.python",
    IfBlock = "punctuation.section.block.conditional.if.python",
    While = "keyword.control.loop.while.python",
    WhileBlock = "punctuation.section.block.loop.while.python",
    Else = "keyword.control.conditional.else.python",
    ElseBlock = "punctuation.section.block.conditional.else.python",
    Try = "keyword.control.exception.try.python",
    TryBlock = "punctuation.section.block.exception.try.python",
    Finally = "keyword.control.exception.finally.python",
    FinallyBlock = "punctuation.section.block.exception.finally.python",
    Elif = "keyword.control.conditional.elseif.python",
    ElifBlock = "punctuation.section.block.conditional.elseif.python",
    Await = "keyword.other.await.python",
    StrayRound = "invalid.illegal.stray.brace.round.python",
    StraySquare = "invalid.illegal.stray.brace.square.python",
    StrayCurly = "invalid.illegal.stray.brace.curly.python",
    ArgsBegin = "punctuation.section.arguments.begin.python",
    ArgsEnd = "punctuation.section.arguments.end.python",
    ArgsSeparator = "punctuation.separator.arguments.python",
    BracketsBegin = "punctuation.section.brackets.begin.python",
    BracketsEnd = "punctuation.section.brackets.end.python",
    Slice = "punctuation.separator.slice.python",
    Constant = "constant.language.python",
    // numbers: each kind, then its captures
    Hex = "constant.numeric.integer.hexadecimal.python",
    HexBase = "constant.numeric.integer.hexadecimal.python punctuation.definition.numeric.base.python",
    HexSuffix = "constant.numeric.integer.hexadecimal.python storage.type.numeric.python",
    Octal = "constant.numeric.integer.octal.python",
    OctalBase = "constant.numeric.integer.octal.python punctuation.definition.numeric.base.python",
    OctalSuffix = "constant.numeric.integer.octal.python storage.type.numeric.python",
    Binary = "constant.numeric.integer.binary.python",
    BinaryBase = "constant.numeric.integer.binary.python punctuation.definition.numeric.base.python",
    BinarySuffix = "constant.numeric.integer.binary.python storage.type.numeric.python",
    Imaginary = "constant.numeric.imaginary.decimal.python",
    ImaginaryDot = "constant.numeric.imaginary.decimal.python punctuation.separator.decimal.python",
    ImaginarySuffix = "constant.numeric.imaginary.decimal.python storage.type.numeric.python",
    Float = "constant.numeric.float.decimal.python",
    FloatDot = "constant.numeric.float.decimal.python punctuation.separator.decimal.python",
    Decimal = "constant.numeric.integer.decimal.python",
    DecimalSuffix = "constant.numeric.integer.decimal.python storage.type.numeric.python",
    Global = "storage.modifier.global.python",
    Nonlocal = "storage.modifier.nonlocal.python",
    StorageSeparator = "punctuation.separator.storage-list.python",
    StorageIllegal = "invalid.illegal.name.storage.python",
    Yield = "keyword.control.flow.yield.python",
    YieldFrom = "keyword.control.flow.yield-from.python",
    InlineAssignment = "keyword.operator.assignment.inline.python",
    NotAllowed = "invalid.illegal.not-allowed-here.python",
    VariableAnnotation = "punctuation.separator.annotation.variable.python",
    Augmented = "keyword.operator.assignment.augmented.python",
    Assignment = "keyword.operator.assignment.python",
    Deprecated = "invalid.deprecated.operator.python",
    Comparison = "keyword.operator.comparison.python",
    Arithmetic = "keyword.operator.arithmetic.python",
    Logical = "keyword.operator.logical.python",
    Matrix = "keyword.operator.matrix.python",
    IllegalSyntax = "invalid.illegal.syntax.python",
    UnpackMapping = "keyword.operator.unpacking.mapping.python",
    UnpackSequence = "keyword.operator.unpacking.sequence.python",
    // classes and functions
    ClassKeyword = "storage.type.class.python keyword.declaration.class.python",
    ClassBegin = "punctuation.section.class.begin.python",
    InheritanceBegin = "punctuation.section.inheritance.begin.python",
    InheritanceEnd = "punctuation.section.inheritance.end.python",
    NoClosingParens = "invalid.illegal.no-closing-parens.python",
    InheritanceSeparator = "punctuation.separator.inheritance.python",
    InheritanceParameter = "variable.parameter.class-inheritance.python",
    Inherited = "entity.other.inherited-class.python",
    InheritedDot = "entity.other.inherited-class.python punctuation.accessor.dot.python",
    DefKeyword = "storage.type.function.python keyword.declaration.function.python",
    FunctionBegin = "punctuation.section.function.begin.python",
    ParametersBegin = "punctuation.section.parameters.begin.python",
    ParametersEnd = "punctuation.section.parameters.end.python",
    ParametersSeparator = "punctuation.separator.parameters.python",
    PositionalOnly = "storage.modifier.positional-args-only.python",
    ExpectedComma = "invalid.illegal.expected-comma.python",
    ParameterAnnotation = "punctuation.separator.annotation.parameter.python",
    Parameter = "variable.parameter.python",
    GroupBegin = "punctuation.section.group.begin.python",
    GroupEnd = "punctuation.section.group.end.python",
    IllegalDefault = "invalid.illegal.default-value.python",
    IllegalAnnotation = "invalid.illegal.annotation.python",
    ReturnAnnotation = "punctuation.separator.annotation.return.python",
    AnnotationMark = "punctuation.definition.annotation.python",
    IllegalCharacter = "invalid.illegal.character.python",
    LambdaKeyword = "storage.type.function.inline.python keyword.declaration.function.inline.python",
    ExpectedParameter = "invalid.illegal.expected-parameter.python",
    TupleSeparator = "punctuation.separator.tuple.python",
    SequenceBegin = "punctuation.section.sequence.begin.python",
    SequenceEnd = "punctuation.section.sequence.end.python",
    SequenceSeparator = "punctuation.separator.sequence.python",
    MappingBegin = "punctuation.section.mapping.begin.python",
    MappingEnd = "punctuation.section.mapping.end.python",
    SetBegin = "punctuation.section.set.begin.python",
    SetEnd = "punctuation.section.set.end.python",
    MappingOrSetBegin = "punctuation.section.mapping-or-set.begin.python",
    MappingOrSetEnd = "punctuation.section.mapping-or-set.end.python",
    SetSeparator = "punctuation.separator.set.python",
    KeyValue = "punctuation.separator.mapping.key-value.python",
    ExpectedColon = "invalid.illegal.expected-colon.python",
    MappingSeparator = "punctuation.separator.mapping.python",
    UnexpectedComma = "invalid.illegal.unexpected-comma.python",
    ColonInSet = "invalid.illegal.colon-inside-set.python",
    Continuation = "punctuation.separator.continuation.line.python",
    UnexpectedText = "invalid.illegal.unexpected-text.python",
    Generator = "keyword.control.loop.for.generator.python",
    TargetSeparator = "punctuation.separator.target-list.python",
    TargetBegin = "punctuation.section.target-list.begin.python",
    TargetEnd = "punctuation.section.target-list.end.python",
    Terminator = "punctuation.terminator.statement.python",
    // strings
    StringPrefix = "storage.type.string.python",
    StringBegin = "punctuation.definition.string.begin.python",
    StringEnd = "punctuation.definition.string.end.python",
    DocBegin = "punctuation.definition.comment.begin.python",
    DocEnd = "punctuation.definition.comment.end.python",
    EscapeHex = "constant.character.escape.hex.python",
    EscapeOctal = "constant.character.escape.octal.python",
    Escape = "constant.character.escape.python",
    EscapeDeprecated = "invalid.deprecated.character.escape.python",
    Escape16 = "constant.character.escape.unicode.16-bit-hex.python",
    Escape32 = "constant.character.escape.unicode.32-bit-hex.python",
    EscapeName = "constant.character.escape.unicode.name.python",
    FEscapeBackslash = "constant.character.escape.backslash.regexp",
    FEscapeDeprecated = "constant.character.escape.backslash.regexp invalid.deprecated.character.escape.python",
    FEscapeBraces = "constant.character.escape.backslash.regexp constant.character.escape.python",
    Placeholder = "constant.other.placeholder.python",
    PlaceholderVariable = "constant.other.placeholder.python variable.other.placeholder.python",
    PlaceholderBegin = "constant.other.placeholder.python punctuation.definition.placeholder.begin.python",
    PlaceholderConversion = "constant.other.placeholder.python storage.modifier.conversion.python",
    PlaceholderSpec = "constant.other.placeholder.python constant.other.format-spec.python",
    PlaceholderEnd = "constant.other.placeholder.python punctuation.definition.placeholder.end.python",
    PhBegin = "punctuation.definition.placeholder.begin.python",
    PhEnd = "punctuation.definition.placeholder.end.python",
    Conversion = "storage.modifier.conversion.python",
    Debug = "storage.modifier.debug.python",
    EmptyExpression = "invalid.illegal.empty-expression.python",
    StrayBrace = "invalid.illegal.stray-brace.python",
    InterpolationBegin = "punctuation.section.interpolation.begin.python",
    InterpolationEnd = "punctuation.section.interpolation.end.python",
    BackslashInFString = "invalid.illegal.backslash-in-fstring.python",
    // regular expressions
    ReComment = "comment.line.number-sign.regexp",
    ReCommentMark = "comment.line.number-sign.regexp punctuation.definition.comment.regexp",
    ReAnchor = "keyword.control.anchor.regexp",
    ReBackReference = "keyword.other.back-reference.regexp",
    ReQuantifier = "keyword.operator.quantifier.regexp",
    ReOr = "keyword.operator.or.regexp",
    ReCommentBegin = "punctuation.definition.comment.begin.regexp",
    ReCommentEnd = "punctuation.definition.comment.end.regexp",
    ReOptionToggle = "keyword.other.option-toggle.regexp",
    ReNamedBackReference = "keyword.other.back-reference.named.regexp",
    ReGroupBegin = "punctuation.definition.group.begin.regexp",
    ReAssertion = "constant.other.assertion.regexp",
    ReGroupEnd = "punctuation.definition.group.end.regexp",
    ReConditional = "punctuation.definition.group.assertion.conditional.regexp",
    ReConditionalReference = "punctuation.definition.group.assertion.conditional.regexp variable.other.back-reference.regexp",
    ReCapture = "punctuation.definition.group.capture.regexp",
    ReGroupName = "entity.name.other.group.regexp",
    ReNoCapture = "punctuation.definition.group.no-capture.regexp",
    ReClass = "constant.character.character-class.regexp",
    ReEscape = "constant.character.escape.backslash.regexp",
    ReSetBegin = "punctuation.definition.character-class.begin.regexp",
    ReNegation = "keyword.operator.negation.regexp",
    ReSetEnd = "punctuation.definition.character-class.end.regexp",
    ReRange = "constant.other.character-class.range.regexp",
    ReRangeEscape = "constant.other.character-class.range.regexp constant.character.escape.backslash.regexp",
    // SQL's kinds, from here on: `Sql` and the next `sql::KINDS - 1` leaves are SQL's kinds in
    // order, whose scopes `sql::scopes` gives (Python's SQL runs on SQL's scanner, embedded)
    Sql = "",
}

/// A wrap, what the stack around a token adds to its leaf, as bits: `meta.annotation`, then
/// the host, then the inner layer. A kind holds it as an index, [`base`]'s, above its leaf's
/// [`LEAF_BITS`].
const ANN: u16 = 1;
const HOST_SHIFT: u16 = 1;
const INNER_SHIFT: u16 = 4;
const HOST_MASK: u16 = 7 << HOST_SHIFT;
const INNER_MASK: u16 = 15 << INNER_SHIFT;
const LEAF_BITS: u16 = 9;
/// The inner layers a string can hold: all but the last of `INNERS`, the format spec.
const STRING_INNERS: u16 = 15;
/// How many wraps there are without `meta.annotation`: two outside strings (none, a format
/// spec), each string's with its inner layers, and the docstring's.
const WRAPS: u16 = 2 + 4 * STRING_INNERS + 1;

/// The kind of plain text in wrap `w`: the wrap's index above the leaf's bits.
const fn base(w: u16) -> u16 {
    let host = (w & HOST_MASK) >> HOST_SHIFT;
    let inner = (w & INNER_MASK) >> INNER_SHIFT;
    let i = match host {
        0 => (inner == I_SPEC) as u16,
        1..=4 => {
            let inner = if inner < STRING_INNERS {
                inner
            } else {
                STRING_INNERS - 1
            };
            2 + (host - 1) * STRING_INNERS + inner
        }
        _ => WRAPS - 1,
    };
    (i + if w & ANN != 0 { WRAPS } else { 0 }) << LEAF_BITS
}

/// Every wrap's [`base`], by its bits.
static BASES: [u16; 256] = {
    let mut t = [0; 256];
    let mut w = 0;
    while w < 256 {
        t[w] = base(w as u16);
        w += 1;
    }
    t
};

/// The wrap a kind's index stands for: annotation, host, inner.
fn unbase(kind: u16) -> (bool, u16, u16) {
    let i = kind >> LEAF_BITS;
    let ann = i >= WRAPS;
    let i = i % WRAPS;
    let (host, inner) = match i {
        0 => (0, 0),
        1 => (0, I_SPEC),
        _ if i == WRAPS - 1 => (H_DOC, 0),
        _ => (1 + (i - 2) / STRING_INNERS, (i - 2) % STRING_INNERS),
    };
    (ann, host, inner)
}

/// The string or docstring a token is in, by a kind's host bits.
static HOSTS: [&str; 6] = [
    "",
    "string.quoted.double.python",
    "string.quoted.single.python",
    "string.quoted.double.block.python",
    "string.quoted.single.block.python",
    "comment.block.documentation.python",
];
const H_DOC: u16 = 5;

/// The layer in a string a token is in, by a kind's inner bits; the last, a format spec, is
/// only ever outside a string (an f-string's replacement cleared the string's scope).
static INNERS: [&str; 16] = [
    "",
    "constant.other.character-class.set.regexp",
    "comment.block.regexp",
    "string.quoted.single.sql",
    "string.quoted.other.backtick.sql",
    "string.quoted.double.sql",
    "string.other.quoted.brackets.sql",
    "string.regexp.modr.sql",
    "comment.line.double-dash.sql",
    "comment.line.number-sign.sql",
    "comment.block.c",
    "constant.other.placeholder.python",
    "constant.other.placeholder.python constant.other.format-spec.python",
    "constant.other.placeholder.python constant.other.format-spec.python constant.other.placeholder.python constant.other.format-spec.python",
    "constant.other.placeholder.python constant.other.format-spec.python constant.other.placeholder.python",
    "constant.other.format-spec.python",
];
const I_CLASS: u16 = 1;
const I_RE_COMMENT: u16 = 2;
const I_SQL_SINGLE: u16 = 3;
const I_SQL_BACKTICK: u16 = 4;
const I_SQL_DOUBLE: u16 = 5;
const I_SQL_BRACKETS: u16 = 6;
const I_SQL_MODR: u16 = 7;
const I_SQL_DASHES: u16 = 8;
const I_SQL_HASH: u16 = 9;
const I_SQL_BLOCK: u16 = 10;
const I_PLACEHOLDER: u16 = 11;
const I_PH_SPEC: u16 = 12;
const I_PH_SPEC_PH_SPEC: u16 = 13;
const I_PH_SPEC_PH: u16 = 14;
const I_SPEC: u16 = 15;

/// The scopes under `source.python` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    let leaf = usize::from(kind & ((1 << LEAF_BITS) - 1));
    let (ann, host, inner) = unbase(kind);
    if kind >> LEAF_BITS >= 2 * WRAPS {
        return Vec::new();
    }
    let ann = if ann { "meta.annotation.python" } else { "" };
    let Some(host) = HOSTS.get(usize::from(host)) else {
        return Vec::new();
    };
    let inner = INNERS[usize::from(inner)];
    let pre = [ann, host, inner].into_iter().flat_map(|s| s.split(' '));
    if leaf >= L::Sql as usize {
        let k = (leaf - L::Sql as usize) as u16;
        if usize::from(k) >= crate::sql::KINDS {
            return Vec::new();
        }
        return pre
            .chain(crate::sql::scopes(k))
            .filter(|s| !s.is_empty())
            .collect();
    }
    let Some(leaf) = LEAVES.get(leaf) else {
        return Vec::new();
    };
    pre.chain(leaf.split(' '))
        .filter(|s| !s.is_empty())
        .collect()
}

// The contexts, by bat's names where they have one; `#anon_…` by what they read.
const MAIN: u8 = 0;
/// `comments`' `#`, to the line's end.
const COMMENT: u8 = 1;
/// After a line continuation's `\`, to the next line's first byte.
const LINE_CONT: u8 = 2;
const RAISE: u8 = 3;
const RAISE_FROM: u8 = 4;
const IMPORT: u8 = 5;
/// `from …`, before its `import`.
const FROM: u8 = 6;
/// `from … import`, before the names.
const FROM_IMPORT: u8 = 7;
const IMPORT_PAREN: u8 = 8;
const IMPORT_LIST: u8 = 9;
/// `import-from-name`'s path.
const IMPORT_PATH: u8 = 10;
const FOR: u8 = 11;
const FOR_IN: u8 = 12;
/// `with-body`.
const WITH: u8 = 13;
const WITH_AS: u8 = 14;
const EXCEPT: u8 = 15;
const EXCEPT_AS: u8 = 16;
const IF: u8 = 17;
const WHILE: u8 = 18;
const ELIF: u8 = 19;
const CLASS: u8 = 20;
const CLASS_NAME: u8 = 21;
/// `class X(`, to `)`.
const CLASS_BASES: u8 = 22;
const CLASS_AFTER: u8 = 23;
const INHERITED: u8 = 24;
const FUNCTION: u8 = 25;
const FUNC_NAME: u8 = 26;
const FUNC_OPEN: u8 = 27;
/// `function-parameters`.
const PARAMS: u8 = 28;
const POSONLY: u8 = 29;
/// A parameter's `=`, then its default.
const DEFAULT_EQ: u8 = 30;
const DEFAULT: u8 = 31;
/// A parameter's `:`, then its annotation.
const ANN_COLON: u8 = 32;
const PARAM_ANN: u8 = 33;
/// `function-parameters-tuple`.
const PTUPLE: u8 = 34;
const PT_DEFAULT: u8 = 35;
const PT_ANN: u8 = 36;
const FUNC_AFTER: u8 = 37;
const RETURN_ANN: u8 = 38;
/// `allow-unpack-operators`.
const UNPACK: u8 = 39;
const DECORATOR: u8 = 40;
const DEC_WRAP: u8 = 41;
const DEC_WRAP_DOT: u8 = 42;
const DEC_WRAP_NAME: u8 = 43;
/// `decorator-function-call-wrapper`.
const DEC_CALL: u8 = 44;
const DEC_ARGS: u8 = 45;
const DEC_CALL_DOT: u8 = 46;
const DEC_CALL_NAME: u8 = 47;
/// `qualified-name-until-leaf`.
const QN_LEAF: u8 = 48;
const LEAF_NAME: u8 = 49;
const LEAF_DOT: u8 = 50;
/// `item-access`.
const ITEM: u8 = 51;
const ITEM_NAME: u8 = 52;
const ITEM_ARGS: u8 = 53;
/// `function-call-wrapper`.
const CALL: u8 = 54;
const CALL_OPEN: u8 = 55;
const ARGS: u8 = 56;
const CALL_DOT: u8 = 57;
const CALL_NAME: u8 = 58;
const KWARG: u8 = 59;
const KWARG_VALUE: u8 = 60;
/// `lambda-parameters`.
const LAMBDA: u8 = 61;
const LAMBDA_COLON: u8 = 62;
const LAMBDA_BODY: u8 = 63;
const GROUP: u8 = 64;
const TUPLE: u8 = 65;
const LIST: u8 = 66;
/// `{` that may be a mapping or a set.
const MAPSET: u8 = 67;
const DICT: u8 = 68;
const DICT_UNPACK: u8 = 69;
const DICT_KEY: u8 = 70;
/// `inside-directory-value`.
const DICT_VALUE: u8 = 71;
const DICT_COMMA: u8 = 72;
const DICT_FOR: u8 = 73;
const DICT_VAL: u8 = 74;
const SET: u8 = 75;
const SET_UNPACK: u8 = 76;
/// `after-expression`.
const AFTER: u8 = 77;
const AFTER_ARGS: u8 = 78;
const AFTER_ITEM: u8 = 79;
/// After `.`: `after-expression`'s and `expression-in-a-group`'s.
const ATTR: u8 = 80;
/// After `global` or `nonlocal`.
const MODIFIER: u8 = 81;
const NAME: u8 = 82;
const DOTTED: u8 = 83;
/// `qualified-name`.
const QN: u8 = 84;
const INLINE_FOR: u8 = 85;
/// `target-list`'s `( … )`.
const TARGETS: u8 = 86;
/// `docstrings`' push, before the quotes.
const DOC_START: u8 = 87;
/// A docstring: `"""`, `'''`, and raw ones.
const DOC_DQ: u8 = 88;
const DOC_SQ: u8 = 89;
const DOC_RAW_DQ: u8 = 90;
const DOC_RAW_SQ: u8 = 91;
/// `f-string-replacement` and its contexts.
const FREPL: u8 = 92;
const FREPL_IN: u8 = 93;
const FSPEC: u8 = 94;
const FEXPR: u8 = 95;
/// `f-string-replacement-reset` and its contexts.
const FREPL_R: u8 = 96;
const FREPL_R_IN: u8 = 97;
const FEXPR_R: u8 = 98;
/// `formatting-syntax`' nested form, and its format spec.
const PLACEHOLDER: u8 = 99;
const PH_SPEC: u8 = 100;
/// The strings: a block of [`VARIANTS`] for each quote, `"`, `'`, `"""` and `'''`.
const STR: u8 = 101;
const VARIANTS: u8 = 13;
/// `[uU]?R`, raw; a block's `"""` one takes `\N{…}` and the like.
const V_RAWCAP_U: u8 = 0;
/// `[bB]R|R[bB]`.
const V_RAWCAP_B: u8 = 1;
/// A block's `[uU]?r`, before its first byte says SQL or a regex.
const V_RDETECT: u8 = 2;
/// `[uU]?r` with SQL in it.
const V_RSQL: u8 = 3;
/// `[uU]?r` with a regex in it.
const V_RRE: u8 = 4;
/// `[bB]r|r[bB]`, a regex.
const V_BRRE: u8 = 5;
/// `[fF]R|R[fF]`.
const V_RF: u8 = 6;
/// `[fF]r|r[fF]`, a regex with replacements.
const V_RFRE: u8 = 7;
/// `[fF]`.
const V_F: u8 = 8;
/// A block's `[uU]?`, before its first byte says SQL or not.
const V_DETECT: u8 = 9;
/// `[uU]?` with SQL in it.
const V_SQL: u8 = 10;
/// `[uU]?`.
const V_PLAIN: u8 = 11;
/// `[bB]`.
const V_BYTES: u8 = 12;
/// The SQL grammar's contexts.
const SQL_MAIN: u8 = STR + 4 * VARIANTS;
const SQL_DASHES: u8 = SQL_MAIN + 1;
const SQL_HASH: u8 = SQL_MAIN + 2;
const SQL_BLOCK: u8 = SQL_MAIN + 3;
const SQL_MODR: u8 = SQL_MAIN + 4;
const SQL_SINGLE: u8 = SQL_MAIN + 5;
const SQL_BACKTICK: u8 = SQL_MAIN + 6;
const SQL_DOUBLE: u8 = SQL_MAIN + 7;
const SQL_BRACKETS: u8 = SQL_MAIN + 8;
// `SQL_MAIN + 9`: `identifier_create` (named only in `TO_SQL`)
/// The regex grammar's contexts.
const RE_MAIN: u8 = SQL_MAIN + 10;
const RE_COMMENT: u8 = RE_MAIN + 1;
/// A group: `(?=`, `(?(1)`, `(`.
const RE_GROUP: u8 = RE_MAIN + 2;
const RE_SET: u8 = RE_MAIN + 3;

/// Whether context `c` is the SQL's or the regex's, under its string's prototype.
const fn embedded(c: u8) -> bool {
    c >= SQL_MAIN
}

/// The kind of the text no pattern of context `c` matches, past its wrap.
fn text_leaf(c: u8) -> L {
    match c {
        COMMENT => L::Comment,
        CLASS_NAME => L::ClassName,
        FUNC_NAME => L::DefName,
        INHERITED => L::Inherited,
        PT_DEFAULT => L::IllegalDefault,
        PT_ANN => L::IllegalAnnotation,
        CALL_DOT | CALL_NAME => L::Call,
        DEC_WRAP_DOT | DEC_WRAP_NAME => L::Ann,
        DEC_CALL_DOT | DEC_CALL_NAME => L::AnnCall,
        _ => L::Plain,
    }
}

/// What context `c` does to the wrap its parent leaves it, `w`.
fn wrap(c: u8, w: u16) -> u16 {
    let with_inner = |w: u16, i: u16| (w & !INNER_MASK) | (i << INNER_SHIFT);
    let inner = (w & INNER_MASK) >> INNER_SHIFT;
    match c {
        DECORATOR | DEC_CALL | DEC_ARGS => w | ANN,
        DOC_DQ..=DOC_RAW_SQ => (w & ANN) | (H_DOC << HOST_SHIFT),
        // `clear_scopes: 1` takes the scope on top: the string's, or the layer in it
        FREPL => {
            if inner != 0 {
                w & !INNER_MASK
            } else {
                w & !HOST_MASK
            }
        }
        // `clear_scopes: true`
        FREPL_R => 0,
        FSPEC => with_inner(w, I_SPEC),
        // a placeholder nested in a format spec: four layers kept, deeper ones as the last
        // four (the theme sees the deepest)
        PLACEHOLDER => with_inner(
            w,
            match inner {
                I_PH_SPEC | I_PH_SPEC_PH_SPEC => I_PH_SPEC_PH,
                _ => I_PLACEHOLDER,
            },
        ),
        PH_SPEC => with_inner(
            w,
            match inner {
                I_PLACEHOLDER => I_PH_SPEC,
                _ => I_PH_SPEC_PH_SPEC,
            },
        ),
        c if (STR..SQL_MAIN).contains(&c) => {
            let q = u16::from((c - STR) / VARIANTS);
            (w & ANN) | ((q + 1) << HOST_SHIFT)
        }
        SQL_DASHES => with_inner(w, I_SQL_DASHES),
        SQL_HASH => with_inner(w, I_SQL_HASH),
        SQL_BLOCK => with_inner(w, I_SQL_BLOCK),
        SQL_MODR => with_inner(w, I_SQL_MODR),
        SQL_SINGLE => with_inner(w, I_SQL_SINGLE),
        SQL_BACKTICK => with_inner(w, I_SQL_BACKTICK),
        SQL_DOUBLE => with_inner(w, I_SQL_DOUBLE),
        SQL_BRACKETS => with_inner(w, I_SQL_BRACKETS),
        RE_COMMENT => with_inner(w, I_RE_COMMENT),
        RE_SET => with_inner(w, I_CLASS),
        _ => w,
    }
}

/// The bytes a `"` string's prototype can start on in the SQL it holds: its end, an escape
/// or a line continuation, a placeholder.
static SQL_YIELDS_DQ: [bool; 256] = stops(b"\"\\%{}");
/// A `'` string's.
static SQL_YIELDS_SQ: [bool; 256] = stops(b"'\\%{}");

/// SQL's context for each of the SQL's contexts here, from [`SQL_MAIN`].
const TO_SQL: [u8; 10] = {
    use crate::sql::*;
    [
        MAIN, DASHES, HASHES, BLOCK, MODR, SINGLE, BACKTICK, DOUBLE, BRACKETS, CREATE,
    ]
};

/// The context here for each of SQL's.
const FROM_SQL: [u8; 10] = {
    let mut t = [SQL_MAIN; 10];
    let mut i = 0;
    while i < TO_SQL.len() {
        t[TO_SQL[i] as usize] = SQL_MAIN + i as u8;
        i += 1;
    }
    t
};

/// How many places [`Scan::info`] keeps what it found for.
const WCACHE: usize = 8;

/// How many contexts a line can hold; one pushed past it drops the bottom one.
const MAX: usize = 128;
/// Matches that take nothing, in a row at one place, before the scan steps on regardless.
const STUCK: usize = 64;
/// syntect's stack limit (with its two bottom contexts, `__start` and `main`): at it, a push's
/// pattern matches nothing, so the patterns after it get their turn.
const SYNTECT_DEPTH: usize = 99;

/// Lex `b`, a line of Python without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let mut s = Scan::fresh(b, out, Push);
    s.load_state(state);
    s.lex();
    *state = s.state();
}

/// [`line`]'s runs by theme rule pushed to `runs` (X-074, X-068's sink): [`crate::lex_runs`] for
/// Python.
#[inline(never)]
pub fn line_runs(
    b: &[u8],
    state: &mut State,
    out: &mut Out,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
) {
    let mut s = Scan::fresh(b, out, Runs::new(rules, runs));
    s.load_state(state);
    s.lex();
    *state = s.state();
    s.k.end_line(b.len());
}

/// A block's lines on one scan opened from `state` ([`crate::Carry`], X-067), `state` left as
/// the last one leaves it.
pub fn block(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    let mut out = Out::new(spans);
    let mut s = Scan::fresh(b"", &mut out, Push);
    s.load_state(state);
    let rest = pass(&mut s);
    *state = s.state();
    rest
}

/// [`block`]'s runs by theme rule pushed to `runs` ([`crate::CarryRuns`], X-074).
pub fn block_runs(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    pass: &mut dyn FnMut(&mut dyn crate::CarryRuns) -> Option<usize>,
) -> Option<usize> {
    let mut out = Out::new(spans);
    out.0.clear();
    let mut s = Scan::fresh(b"", &mut out, Runs::new(rules, runs));
    s.load_state(state);
    let rest = pass(&mut s);
    *state = s.state();
    rest
}

impl crate::Carry for Scan<'_, '_, Push> {
    fn next(&mut self, b: &[u8]) -> &[crate::Span] {
        self.out.0.clear();
        // SAFETY: the line is set back to an empty one before `b`'s borrow ends
        self.carry(unsafe { crate::line_for(b) });
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

impl<'b, 'c, K: Sink> Scan<'b, 'c, K> {
    /// `b`, the block's next line, lexed on this scan; the scan's line an empty one again after.
    #[inline(always)]
    fn carry(&mut self, b: &'b [u8]) {
        self.b = b;
        self.text = 0;
        self.at = 0;
        self.loop_at = (usize::MAX, 0);
        self.looped = false;
        self.stuck = 0;
        for c in &self.wcache {
            c.set((usize::MAX, Word::NONE));
        }
        self.sql = crate::sql::Embedded {
            is_end: usize::MAX,
            at: 0,
        };
        self.lex();
        // the line's end as a state's save and load leave it (`line` leaves that to its state)
        if self.depth - 1 > crate::stack::HELD {
            self.reload();
        }
        self.b = b"";
    }

    /// A scan of `b`, returned as a literal so that it is built in place; the state is
    /// [`Scan::load_state`]'s.
    #[inline(always)]
    fn fresh(b: &'b [u8], out: &'b mut Out<'c>, k: K) -> Self {
        Scan {
            b,
            stack: [MAIN; MAX],
            raws: [0; MAX],
            depth: 1,
            exact: true,
            text: 0,
            at: 0,
            loop_at: (usize::MAX, 0),
            looped: false,
            stuck: 0,
            wcache: [const { std::cell::Cell::new((usize::MAX, Word::NONE)) }; WCACHE],
            sql: crate::sql::Embedded {
                is_end: usize::MAX,
                at: 0,
            },
            k,
            out,
        }
    }

    /// What [`Scan::fresh`] leaves to `state`: the stack and what is read from it, apart from
    /// the literal, which a constructor that changed it before returning it copied (a `memcpy`
    /// a line, X-056's trap).
    #[inline(always)]
    fn load_state(&mut self, state: &State) {
        self.load(state);
    }

    /// The stack and its levels' wraps from `state`.
    #[inline(always)]
    fn load(&mut self, state: &State) {
        self.exact = state.is_exact();
        self.depth = 1;
        let n = usize::from(state.depth);
        for i in 0..n {
            let c = state.level(i);
            self.push(c);
        }
    }

    /// The line lexed from the stack it starts with.
    #[inline(always)]
    fn lex(&mut self) {
        self.refresh();
        self.run();
        self.fill(self.b.len());
    }

    /// The scan as a state's save and load leave it, its stack deeper than a state holds.
    #[cold]
    #[inline(never)]
    fn reload(&mut self) {
        let state = self.state();
        self.load(&state);
    }

    /// The state the scan's last line leaves.
    #[inline(always)]
    fn state(&self) -> State {
        State::keep(&self.stack, self.depth, self.exact, 0)
    }
}

/// Whether no pattern of context `c` starts at a space past the line's start: none starts
/// with `\s` or ` `, matches nothing (`''`, a lookahead that can see a space), or is `$`.
const fn spaces_inert(c: u8) -> bool {
    matches!(
        c,
        MAIN | GROUP
            | TUPLE
            | LIST
            | MAPSET
            | DICT
            | DICT_VALUE
            | DICT_UNPACK
            | SET
            | SET_UNPACK
            | ARGS
            | AFTER_ARGS
            | AFTER_ITEM
            | ITEM_ARGS
            | ITEM
            | PARAMS
            | POSONLY
            | DEFAULT
            | PARAM_ANN
            | PTUPLE
            | FUNC_AFTER
            | UNPACK
            | DEC_ARGS
            | CALL
            | CALL_OPEN
            | KWARG_VALUE
            | INLINE_FOR
            | TARGETS
            | DICT_COMMA
            | DICT_FOR
            | FSPEC
            | FREPL
            | FEXPR
            | FEXPR_R
            | PLACEHOLDER
            | PH_SPEC
            | DOC_START
            | CLASS_BASES
    )
}

/// The bytes a pattern of a text context can start at: a docstring's (its escapes, its end),
/// a raw docstring's, a string's (its end, escapes, placeholders, replacements, `\` at the
/// line's end), and the embedded contexts', their string's prototype's stops with their own.
static DOC_STOPS: [bool; 256] = stops(b"\\\"'");
static QUOTE_STOPS: [bool; 256] = stops(b"\"'");
static STRING_STOPS: [bool; 256] = stops(b"\"'\\{}%");
static RE_COMMENT_STOPS: [bool; 256] = stops(b"\"'\\{}%)");
static RE_STOPS: [bool; 256] = stops(b"\"'\\{}%#^$?+*|()[].");

/// Where `{{identifier}}` ends when its word stops at `e` before a character past
/// ASCII: `[[:alnum:]]` takes Unicode's other numbers (`₁`), which `\w` doesn't, so the run
/// goes on through them and then backs off to its last `\b`.
#[cold]
fn alnum_ident_end(b: &[u8], e: usize) -> usize {
    let alnum = |i: usize| match b.get(i) {
        Some(&c) if c < 0x80 => WORD[usize::from(c)],
        Some(_) => wide_char(b, i).is_some_and(char::is_alphanumeric),
        None => false,
    };
    let mut q = e;
    while alnum(q) {
        q += char_len(b[q]).min(b.len() - q);
    }
    if q == e {
        return e;
    }
    // back to the last place `\b` holds, a character at a time
    let boundary = |i: usize| (i > 0 && is_word(b, i - 1)) != is_word(b, i);
    while q > e && !boundary(q) {
        q -= 1;
        while q > e && b[q] & 0xc0 == 0x80 {
            q -= 1;
        }
    }
    q.max(e)
}

/// Where the ` `s at `p` end.
fn skip_blanks(b: &[u8], mut p: usize) -> usize {
    while b.get(p) == Some(&b' ') {
        p += 1;
    }
    p
}

/// `\d*` at `p`: where it ends.
fn digits0(b: &[u8], mut p: usize) -> usize {
    while let Some(e) = digit(b, p) {
        p = e;
    }
    p
}

/// `{{digits}}`, `\d+(?:_\d+)*`, at `p`: where it ends.
fn digits(b: &[u8], p: usize) -> Option<usize> {
    let mut q = digits0(b, digit(b, p)?);
    while b.get(q) == Some(&b'_')
        && let Some(e) = digit(b, q + 1)
    {
        q = digits0(b, e);
    }
    Some(q)
}

/// `{{exponent}}`, `[eE][-+]?{{digits}}`, at `p`: where it ends.
fn exponent(b: &[u8], p: usize) -> Option<usize> {
    if !matches!(b.get(p), Some(b'e' | b'E')) {
        return None;
    }
    let q = if matches!(b.get(p + 1), Some(b'-' | b'+')) {
        p + 2
    } else {
        p + 1
    };
    digits(b, q)
}

/// Whether `w` starts the bytes at `p`, ignoring ASCII case.
fn at_ci(b: &[u8], p: usize, w: &[u8]) -> bool {
    b.len() >= p + w.len() && b[p..p + w.len()].eq_ignore_ascii_case(w)
}

/// A number at `p`: its end, its leaf, and its captures as `(start, end, leaf)`, in order.
type Number = (usize, L, [(usize, usize, L); 2]);

/// `numbers` at `p`, in the grammar's order.
fn number(b: &[u8], p: usize) -> Option<Number> {
    const NONE: (usize, usize, L) = (0, 0, L::Plain);
    let c0 = *b.get(p)?;
    let word = starts_word(b, p);
    let lower = |q: usize| b.get(q).map(u8::to_ascii_lowercase);
    let hex = |q: usize| b.get(q).is_some_and(u8::is_ascii_hexdigit);
    let oct = |q: usize| matches!(b.get(q), Some(b'0'..=b'7'));
    let bin = |q: usize| matches!(b.get(q), Some(b'0' | b'1'));
    if word && c0 == b'0' {
        match lower(p + 1) {
            Some(b'x') => {
                // `(0x)\h*(L)`
                let mut q = p + 2;
                while hex(q) {
                    q += 1;
                }
                if lower(q) == Some(b'l') {
                    return Some((
                        q + 1,
                        L::Hex,
                        [(p, p + 2, L::HexBase), (q, q + 1, L::HexSuffix)],
                    ));
                }
                // `(0x)(_?\h)+`
                let mut q = p + 2;
                loop {
                    if b.get(q) == Some(&b'_') && hex(q + 1) {
                        q += 2;
                    } else if hex(q) {
                        q += 1;
                    } else {
                        break;
                    }
                }
                if q > p + 2 {
                    return Some((q, L::Hex, [(p, p + 2, L::HexBase), NONE]));
                }
            }
            Some(b'o') | Some(b'0'..=b'7') => {}
            Some(b'b') => {}
            _ => {}
        }
        // `(0o?)(?=o|[0-7])[0-7]*(L)`: `0` or `0o`, a digit, then `L`
        for base in [p + 2, p + 1] {
            if base == p + 2 && lower(p + 1) != Some(b'o') {
                continue;
            }
            let mut q = base;
            while oct(q) {
                q += 1;
            }
            if q > base && lower(q) == Some(b'l') {
                return Some((
                    q + 1,
                    L::Octal,
                    [(p, base, L::OctalBase), (q, q + 1, L::OctalSuffix)],
                ));
            }
        }
        // `(0)[0-7]+`
        if oct(p + 1) {
            let mut q = p + 1;
            while oct(q) {
                q += 1;
            }
            return Some((q, L::Octal, [(p, p + 1, L::OctalBase), NONE]));
        }
        // `(0o)(_?[0-7])+`
        if lower(p + 1) == Some(b'o') {
            let mut q = p + 2;
            loop {
                if b.get(q) == Some(&b'_') && oct(q + 1) {
                    q += 2;
                } else if oct(q) {
                    q += 1;
                } else {
                    break;
                }
            }
            if q > p + 2 {
                return Some((q, L::Octal, [(p, p + 2, L::OctalBase), NONE]));
            }
        }
        if lower(p + 1) == Some(b'b') {
            // `(0b)[01]*(L)`
            let mut q = p + 2;
            while bin(q) {
                q += 1;
            }
            if lower(q) == Some(b'l') {
                return Some((
                    q + 1,
                    L::Binary,
                    [(p, p + 2, L::BinaryBase), (q, q + 1, L::BinarySuffix)],
                ));
            }
            // `(0b)(_?[01])*`
            let mut q = p + 2;
            loop {
                if b.get(q) == Some(&b'_') && bin(q + 1) {
                    q += 2;
                } else if bin(q) {
                    q += 1;
                } else {
                    break;
                }
            }
            return Some((q, L::Binary, [(p, p + 2, L::BinaryBase), NONE]));
        }
    }
    let is_j = |q: usize| matches!(b.get(q), Some(b'j' | b'J'));
    // the imaginary: `\b{{digits}}(\.)?{{digits}}?{{exponent}}?([jJ])`, or
    // `(\.){{digits}}{{exponent}}?([jJ])`
    let lead = if word { digits(b, p) } else { None };
    if let Some(d) = lead {
        let (q, dot) = if b.get(d) == Some(&b'.') {
            (d + 1, Some(d))
        } else {
            (d, None)
        };
        let q = digits(b, q).unwrap_or(q);
        let q = exponent(b, q).unwrap_or(q);
        if is_j(q) {
            let dot = dot.map_or(NONE, |d| (d, d + 1, L::ImaginaryDot));
            return Some((q + 1, L::Imaginary, [dot, (q, q + 1, L::ImaginarySuffix)]));
        }
    }
    let fraction = if c0 == b'.' { digits(b, p + 1) } else { None };
    if let Some(d) = fraction {
        let q = exponent(b, d).unwrap_or(d);
        if is_j(q) {
            return Some((
                q + 1,
                L::Imaginary,
                [(p, p + 1, L::ImaginaryDot), (q, q + 1, L::ImaginarySuffix)],
            ));
        }
    }
    // the float: `\b{{digits}}(?:(\.){{digits}}?{{exponent}}?|{{exponent}})`, or
    // `(\.){{digits}}{{exponent}}?`
    if let Some(d) = lead {
        if b.get(d) == Some(&b'.') {
            let q = digits(b, d + 1).unwrap_or(d + 1);
            let q = exponent(b, q).unwrap_or(q);
            return Some((q, L::Float, [(d, d + 1, L::FloatDot), NONE]));
        }
        if let Some(q) = exponent(b, d) {
            return Some((q, L::Float, [NONE; 2]));
        }
    }
    if let Some(d) = fraction {
        let q = exponent(b, d).unwrap_or(d);
        return Some((q, L::Float, [(p, p + 1, L::FloatDot), NONE]));
    }
    if !word {
        return None;
    }
    // `\b(?i)(?:[1-9]\d*|0)(L)\b`, `\b(?i)([1-9][\d_]*|0)\b`
    let lead = match c0 {
        b'0' => Some(p + 1),
        b'1'..=b'9' => Some(digits0(b, p + 1)),
        _ => None,
    }?;
    if lower(lead) == Some(b'l') && !is_word(b, lead + 1) {
        return Some((
            lead + 1,
            L::Decimal,
            [(lead, lead + 1, L::DecimalSuffix), NONE],
        ));
    }
    let e = if c0 == b'0' {
        p + 1
    } else {
        let mut q = p + 1;
        loop {
            if b.get(q) == Some(&b'_') {
                q += 1;
            } else if let Some(e) = digit(b, q) {
                q = e;
            } else {
                break;
            }
        }
        q
    };
    (!is_word(b, e)).then_some((e, L::Decimal, [NONE; 2]))
}

/// `{{identifier_constant}}`: whether the word `w` (all of it) is one.
fn is_constant_name(w: &[u8]) -> bool {
    if w.is_ascii() {
        let up = |c: u8| c.is_ascii_uppercase();
        return (up(w[0]) || w[0] == b'_')
            && w.iter().all(|&c| up(c) || c == b'_' || c.is_ascii_digit())
            && w.windows(2).any(|p| up(p[0]) && up(p[1]));
    }
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
        if !(u || c == '_' || is_nd(c)) {
            return false;
        }
        pair |= u && prev_upper;
        prev_upper = u;
    }
    pair
}

/// A call's usual shape, `.`? then names between dots (` ` around them) and `\s*(`: each
/// name's start (`k` of them, at most eight) and the `(`.
#[derive(Clone, Copy)]
struct CallShape {
    lead: bool,
    names: [usize; 8],
    k: usize,
    paren: usize,
}

/// What the word at a place is, for the patterns that look at whole words: found once.
#[derive(Clone, Copy)]
struct Word {
    /// Where the word ends (`\b\w+`); at its start for none.
    end: usize,
    /// Where `{{identifier}}` from its start ends; at its start for none.
    ident: usize,
    /// Its keyword, `words::K_…`.
    kw: u8,
    /// Its name class, `words::…`.
    special: u16,
}

impl Word {
    const NONE: Word = Word {
        end: 0,
        ident: 0,
        kw: words::K_NONE,
        special: words::GENERIC,
    };
}

struct Scan<'b, 'c, K: Sink> {
    /// The line, without its `\n`: its length is where the `\n` is.
    b: &'b [u8],
    stack: [u8; MAX],
    /// Each level's wrap: what its context and the ones below it add to a kind, as bits
    /// (`BASES` gives its plain text's kind).
    raws: [u8; MAX],
    depth: usize,
    exact: bool,
    /// The kind of text no pattern of the top context matches.
    text: u16,
    /// Where the spans so far end.
    at: usize,
    /// Where the last push that matched nothing was, and the depth it left.
    loop_at: (usize, usize),
    /// A pop at the place was passed over (`loop_at`).
    looped: bool,
    /// Matches that took nothing, in a row at one place.
    stuck: usize,
    /// The places [`Scan::info`] looked at last, and what it found, by the place's low bits.
    wcache: [std::cell::Cell<(usize, Word)>; WCACHE],
    /// The SQL's scan between the string's turns on this line.
    sql: crate::sql::Embedded,
    /// Where the lexemes go, a line's spans or its runs by theme rule (X-074, X-068's sink).
    k: K,
    /// The spans' vector: the span API's, and where SQL's scan pushes its spans for the run sink
    /// to take.
    out: &'b mut Out<'c>,
}

impl<K: Sink> Scan<'_, '_, K> {
    fn top(&self) -> u8 {
        self.stack[self.depth - 1]
    }

    /// The top context's wrap.
    fn w(&self) -> u16 {
        BASES[usize::from(self.raws[self.depth - 1])]
    }

    /// The kind of plain text in context `c` pushed on the top one.
    fn base_of(&self, c: u8) -> u16 {
        BASES[usize::from(wrap(c, u16::from(self.raws[self.depth - 1])))]
    }

    /// The string whose prototype the embedded context on top is under: the first context
    /// below it that isn't the SQL's or the regex's.
    fn host(&self) -> u8 {
        let mut d = self.depth - 1;
        while d > 0 && embedded(self.stack[d]) {
            d -= 1;
        }
        self.stack[d]
    }

    /// Leaf `l` in the top context.
    fn k(&self, l: L) -> u16 {
        self.w() | l as u16
    }

    fn refresh(&mut self) {
        self.text = self.k(text_leaf(self.top()));
    }

    /// The byte at `p`, the line's `\n` at its end.
    fn ch(&self, p: usize) -> Option<u8> {
        match p.cmp(&self.b.len()) {
            std::cmp::Ordering::Less => Some(self.b[p]),
            std::cmp::Ordering::Equal => Some(b'\n'),
            std::cmp::Ordering::Greater => None,
        }
    }

    fn is(&self, p: usize, c: u8) -> bool {
        self.ch(p) == Some(c)
    }

    /// Whether `s` starts the line at `p`.
    fn has(&self, p: usize, s: &[u8]) -> bool {
        self.b.len() >= p + s.len() && &self.b[p..p + s.len()] == s
    }

    /// `\s` at `p`, the line's `\n` one.
    fn space(&self, p: usize) -> bool {
        p == self.b.len() || space_len(self.b, p).is_some()
    }

    /// `\S` at `p`.
    fn nonspace(&self, p: usize) -> bool {
        p < self.b.len() && space_len(self.b, p).is_none()
    }

    /// `$` at `p`.
    fn eol(&self, p: usize) -> bool {
        p >= self.b.len()
    }

    /// Where the character at `p` ends (the line's `\n` is one byte).
    fn next(&self, p: usize) -> usize {
        crate::prim::next_char(self.b, p)
    }

    /// `.` at `p`: where it ends.
    fn any(&self, p: usize) -> Option<usize> {
        (p < self.b.len()).then(|| self.next(p))
    }

    /// The word at `p`.
    #[inline]
    fn info(&self, p: usize) -> Word {
        // most places are no word's start: an ASCII byte other than `\w` takes no call
        if self
            .b
            .get(p)
            .is_none_or(|&c| c < 0x80 && !WORD[usize::from(c)])
        {
            return Word {
                end: p,
                ident: p,
                ..Word::NONE
            };
        }
        let slot = &self.wcache[p % WCACHE];
        let (at, w) = slot.get();
        if at == p {
            return w;
        }
        let w = self.find_word(p);
        slot.set((p, w));
        w
    }

    #[inline(never)]
    fn find_word(&self, p: usize) -> Word {
        let b = self.b;
        let none = Word {
            end: p,
            ident: p,
            ..Word::NONE
        };
        // `\w` at `p`, ASCII without the character's decode
        let Some(&c) = b.get(p) else {
            return none;
        };
        if !(if c < 0x80 {
            WORD[usize::from(c)]
        } else {
            is_word(b, p)
        }) || !starts_word(b, p)
        {
            return none;
        }
        let end = word_end(b, p);
        let ident = if c.is_ascii_digit() || (c >= 0x80 && !is_name_start(b, p)) {
            p
        } else if end >= self.b.len() || b[end] < 0x80 {
            end
        } else {
            alnum_ident_end(&b[..self.b.len()], end)
        };
        let (kw, special) = words::lookup(&b[p..end]);
        Word {
            end,
            ident,
            kw,
            special,
        }
    }

    /// Where the word at `p` ends, if one starts there (`\b\w`).
    #[inline]
    fn word(&self, p: usize) -> Option<usize> {
        let w = self.info(p);
        (w.end > p).then_some(w.end)
    }

    /// `\bw\b` at `p`: where it ends.
    fn kw(&self, p: usize, w: &[u8]) -> Option<usize> {
        let e = self.word(p)?;
        (&self.b[p..e] == w).then_some(e)
    }

    /// `{{identifier}}` at `p`, `\b[[:alpha:]_][[:alnum:]_]*\b`: where it ends.
    #[inline]
    fn ident(&self, p: usize) -> Option<usize> {
        let w = self.info(p);
        (w.ident > p).then_some(w.ident)
    }

    /// `{{path}}` at `p`, `({{identifier}}[ ]*\.[ ]*)*{{identifier}}`: whether one of its ends
    /// takes `then`.
    fn path_then(&self, p: usize, then: impl Fn(&Self, usize) -> bool) -> bool {
        let mut q = p;
        loop {
            let Some(e) = self.ident(q) else {
                return false;
            };
            if then(self, e) {
                return true;
            }
            let d = skip_blanks(self.b, e);
            if !self.is(d, b'.') || d >= self.b.len() {
                return false;
            }
            q = skip_blanks(self.b, d + 1);
        }
    }

    /// `(?={{path}}\s*\()` and `(?={{path}}\s*\[)` at `p`: whether one of the path's ends
    /// takes each. The walk is the one [`call`](Self::call)'s shape takes, so a call comes
    /// with that shape (`Some`, `None` where the shape doesn't hold) and isn't walked again.
    #[inline]
    fn path_follows(&self, p: usize) -> (bool, bool, Option<CallShape>) {
        let mut q = p;
        let mut item = false;
        let mut names = [0; 8];
        let mut k = 0;
        // every name `\w` to its word's end, at most eight
        let mut plain = true;
        loop {
            let w = self.info(q);
            if w.ident <= q {
                return (false, item, None);
            }
            let e = w.ident;
            plain &= w.ident == w.end && k < names.len();
            if k < names.len() {
                names[k] = q;
            }
            k += 1;
            let s = self.spaces(e);
            if s < self.b.len() {
                match self.b[s] {
                    b'(' => {
                        // the shape's `(` after blanks alone, as `qualified-name-until-leaf`'s
                        // `\s*` has them
                        let shape = (plain && skip_blanks(self.b, e) == s).then_some(CallShape {
                            lead: false,
                            names,
                            k,
                            paren: s,
                        });
                        return (true, item, shape);
                    }
                    b'[' => item = true,
                    _ => {}
                }
            }
            let d = skip_blanks(self.b, e);
            if d >= self.b.len() || self.b[d] != b'.' {
                return (false, item, None);
            }
            q = skip_blanks(self.b, d + 1);
        }
    }

    /// Where `\s*` at `p` ends, short of the line's `\n`.
    fn spaces(&self, p: usize) -> usize {
        skip_spaces(self.b, p)
    }

    /// The text up to `p` is the top context's.
    fn fill(&mut self, p: usize) {
        self.fill_with(p.min(self.b.len()), self.text);
    }

    /// Bytes `s..e` are `kind`, after the text before them ([`Out::put`]'s two fills).
    fn put(&mut self, s: usize, e: usize, kind: u16) {
        let n = self.b.len();
        self.fill_with(s.min(n), self.text);
        self.fill_with(e.min(n), kind);
    }

    /// The bytes from where the lexemes end to `p` are `kind` (plain where it is 0), and the
    /// lexemes end at `p`; a `p` at or before that leaves both ([`Out::fill`]).
    #[inline(always)]
    fn fill_with(&mut self, p: usize, kind: u16) {
        if p > self.at {
            if kind != 0 {
                self.k.merge(&mut self.out, self.at, p, kind);
            }
            self.at = p;
        }
    }

    fn push(&mut self, c: u8) {
        if self.depth == MAX {
            crate::stack::drop_bottom(&mut self.stack, &mut self.depth, &mut self.exact);
            for d in 1..self.depth {
                self.relevel(d);
            }
        }
        self.stack[self.depth] = c;
        self.depth += 1;
        self.relevel(self.depth - 1);
        self.refresh();
    }

    /// Level `d`'s wrap and prototype, from the one below it.
    fn relevel(&mut self, d: usize) {
        let c = self.stack[d];
        self.raws[d] = wrap(c, u16::from(self.raws[d - 1])) as u8;
    }

    fn pop(&mut self) {
        if self.depth > 1 {
            self.depth -= 1;
        }
        self.refresh();
    }

    /// Replace the top with `cs`, bottom first.
    fn set(&mut self, cs: &[u8]) {
        self.pop();
        for &c in cs {
            self.push(c);
        }
    }

    /// `cs` pushed by a match that took nothing at `p`: syntect keeps the depth the first of
    /// them leaves for its guard against a loop.
    fn enter(&mut self, p: usize, cs: &[u8]) -> Option<usize> {
        if self.depth >= SYNTECT_DEPTH {
            return None;
        }
        self.fill(p);
        let depth = self.depth + 1;
        for &c in cs {
            self.push(c);
        }
        self.loop_at = (p, depth);
        Some(p)
    }

    /// The top replaced with `cs` by a match that took nothing at `p`.
    fn shift(&mut self, p: usize, cs: &[u8]) -> Option<usize> {
        self.fill(p);
        self.set(cs);
        Some(p)
    }

    /// A pop that takes nothing at `p`, unless it would loop: then syntect passes over it for
    /// the patterns after it (`None`).
    fn leave(&mut self, p: usize) -> Option<usize> {
        if self.loop_at == (p, self.depth) {
            self.looped = true;
            return None;
        }
        self.fill(p);
        self.pop();
        Some(p)
    }

    /// Bytes `s..e` are `kind`; then `cs` are pushed.
    fn tok_push(&mut self, s: usize, e: usize, kind: u16, cs: &[u8]) -> Option<usize> {
        if self.depth >= SYNTECT_DEPTH {
            return None;
        }
        self.put(s, e, kind);
        for &c in cs {
            self.push(c);
        }
        Some(e)
    }

    /// Bytes `s..e` are `kind`, then the top context pops.
    fn close(&mut self, s: usize, e: usize, kind: u16) -> Option<usize> {
        self.put(s, e, kind);
        self.pop();
        Some(e)
    }

    /// Bytes `s..e` are `kind`, then the top context is replaced with `cs`.
    fn then(&mut self, s: usize, e: usize, kind: u16, cs: &[u8]) -> Option<usize> {
        self.put(s, e, kind);
        self.set(cs);
        Some(e)
    }

    /// Bytes `s..e` are `kind`, then the top context is replaced with `after-expression`, which
    /// pops at once unless what follows is a call, an item's access or an attribute's dot.
    fn then_after(&mut self, s: usize, e: usize, kind: u16) -> Option<usize> {
        if self.after_needed(e) {
            self.then(s, e, kind, &[AFTER])
        } else {
            self.close(s, e, kind)
        }
    }

    /// Bytes `s..e` are `kind`, then `after-expression` is pushed, unless it would pop at once.
    fn push_after(&mut self, s: usize, e: usize, kind: u16) -> Option<usize> {
        if self.after_needed(e) {
            self.tok_push(s, e, kind, &[AFTER])
        } else {
            self.put(s, e, kind);
            Some(e)
        }
    }

    /// Whether `after-expression` at `p` matches anything but its `''` pop.
    fn after_needed(&self, p: usize) -> bool {
        let q = self.spaces(p);
        // past those, a call needs a name at `p` itself (a `.` there went above)
        (q < self.b.len() && matches!(self.b[q], b'(' | b'[' | b'.'))
            || (self.ident(p).is_some() && self.call_follows(p))
    }

    /// Bytes `s..e` are `kind`, then `allow-unpack-operators` is pushed, unless it would pop
    /// at the next byte that isn't a space on the line (one that isn't `*` or `#`), where the
    /// context below, in which no pattern starts at a space, goes on the same.
    fn push_unpack(&mut self, s: usize, e: usize, kind: u16) -> Option<usize> {
        if self.unpack_needed(e) {
            self.tok_push(s, e, kind, &[UNPACK])
        } else {
            self.put(s, e, kind);
            Some(e)
        }
    }

    /// Whether `allow-unpack-operators` at `p` would do more than pop at the line's next byte
    /// that isn't a space.
    fn unpack_needed(&self, p: usize) -> bool {
        let q = skip_spaces(self.b, p);
        q >= self.b.len() || matches!(self.b[q], b'*' | b'#')
    }

    /// Bytes `s..e` are leaf `l` of the top context, which stays.
    fn tok(&mut self, s: usize, e: usize, l: L) -> Option<usize> {
        let k = self.k(l);
        self.put(s, e, k);
        Some(e)
    }

    fn run(&mut self) {
        let end = self.b.len() + 1;
        let mut p = 0;
        loop {
            // where no pattern of the context starts at a space, the spaces are skipped whole
            if p < self.b.len() && is_blank(self.b[p]) && spaces_inert(self.top()) && p > 0 {
                p = skip_spaces(self.b, p).min(self.b.len());
            }
            self.looped = false;
            if let Some(q) = self.at(p) {
                self.stuck = if q == p { self.stuck + 1 } else { 0 };
                p = q;
                if self.stuck > STUCK {
                    self.stuck = 0;
                    if p >= self.b.len() {
                        return;
                    }
                    p = self.next(p);
                }
            } else if p >= end || (self.looped && p >= self.b.len()) {
                return;
            } else if self.looped {
                // syntect's guard: a step of a character, and a new search from there
                p = self.next(p);
                self.stuck = 0;
            } else {
                p = self.advance(p);
                self.stuck = 0;
            }
        }
    }

    /// Where a pattern of the top context can next start, after none did at `p`: past the
    /// bytes of a text no pattern starts in, and past a run of spaces in code (a pattern that
    /// starts at a space takes the run as it would from its first).
    fn advance(&self, p: usize) -> usize {
        if p >= self.b.len() {
            return p + 1;
        }
        let b = self.b;
        let c = self.top();
        let stops: &[bool; 256] = match c {
            COMMENT => return self.b.len(),
            DOC_DQ | DOC_SQ => &DOC_STOPS,
            DOC_RAW_DQ | DOC_RAW_SQ => &QUOTE_STOPS,
            RE_COMMENT => &RE_COMMENT_STOPS,
            RE_MAIN | RE_GROUP => &RE_STOPS,
            c if (STR..SQL_MAIN).contains(&c)
                && !matches!((c - STR) % VARIANTS, V_RDETECT | V_DETECT) =>
            {
                &STRING_STOPS
            }
            // a set's range starts at any byte, `\s*\b(drop)` at a space
            c if embedded(c) => return self.next(p),
            _ => {
                if b[p] == b' ' {
                    return skip_blanks(b, p).min(self.b.len());
                }
                return self.next(p);
            }
        };
        let mut q = p + 1;
        while q < self.b.len() && !stops[usize::from(b[q])] {
            q += 1;
        }
        q
    }

    /// The SQL's context `c` on top at `p`, where its string's prototype didn't match: SQL's
    /// scanner, embedded, up to the next place the prototype can start, or past the line.
    fn sql(&mut self, p: usize, c: u8) -> Option<usize> {
        // `SQL_MAIN`'s level: the SQL keeps at most one context above it
        let d = if c == SQL_MAIN {
            self.depth - 1
        } else {
            self.depth - 2
        };
        let host = self.host();
        let dq = host < STR || ((host - STR) / VARIANTS).is_multiple_of(2);
        let w = BASES[usize::from(self.raws[d])];
        let embed = crate::sql::Embed {
            yields: if dq { &SQL_YIELDS_DQ } else { &SQL_YIELDS_SQ },
            base: w + L::Sql as u16,
            text: w,
        };
        self.fill(p);
        self.sql.at = self.at;
        let ctx = TO_SQL[usize::from(c - SQL_MAIN)];
        self.k.close(self.out);
        let r = crate::sql::embedded(
            &self.b[..self.b.len()],
            p,
            ctx,
            &embed,
            &mut self.sql,
            &mut *self.out,
        );
        self.k.reopen(self.out, 16);
        let (q, ctx) = r?;
        self.at = self.sql.at;
        // the stack as the SQL leaves it
        self.depth = d + 1;
        if ctx == crate::sql::MAIN {
            self.refresh();
        } else {
            self.push(FROM_SQL[usize::from(ctx)]);
        }
        Some(q)
    }

    /// The top context's patterns at `p`.
    fn at(&mut self, p: usize) -> Option<usize> {
        let c = self.top();
        if embedded(c) {
            let host = self.host();
            if let Some(q) = self.proto(p, host) {
                return Some(q);
            }
            return if c >= RE_MAIN {
                self.regex(p, c)
            } else {
                self.sql(p, c)
            };
        }
        if c >= STR {
            return self.string(p, c);
        }
        match c {
            MAIN => self.main(p),
            COMMENT => (p == self.b.len())
                .then(|| self.close(p, p + 1, 0))
                .flatten(),
            LINE_CONT => self.strings(p).or_else(|| {
                (self.nonspace(p)
                    || p == self.b.len()
                    || (p == 0 && self.spaces(0) >= self.b.len()))
                .then(|| self.leave(p))
                .flatten()
            }),
            RAISE => self
                .lcop(p)
                .or_else(|| {
                    let e = self.kw(p, b"from")?;
                    let k = self.k(L::RaiseFrom);
                    self.then(p, e, k, &[RAISE_FROM])
                })
                .or_else(|| self.in_statement(p)),
            RAISE_FROM => self.lcop(p).or_else(|| self.in_statement(p)),
            IMPORT => self
                .lcop(p)
                .or_else(|| match self.ch(p)? {
                    b',' => self.tok(p, p + 1, L::ImportSeparator),
                    b'.' => self.tok(p, p + 1, L::ImportUnexpectedRelative),
                    _ => None,
                })
                .or_else(|| self.import_alias(p))
                .or_else(|| self.qualified_name(p))
                .or_else(|| self.pop_at_nonspace(p)),
            FROM => self
                .lcop(p)
                .or_else(|| {
                    let e = self.kw(p, b"import")?;
                    let k = self.k(L::Import);
                    self.then(p, e, k, &[FROM_IMPORT])
                })
                .or_else(|| {
                    // `\.+`
                    if self.is(p, b'.') && p < self.b.len() {
                        let e = (p..self.b.len())
                            .find(|&q| self.b[q] != b'.')
                            .unwrap_or(self.b.len());
                        return self.tok(p, e, L::ImportRelative);
                    }
                    self.ident(p)?;
                    self.enter(p, &[IMPORT_PATH])
                })
                .or_else(|| self.pop_at_nonspace(p)),
            FROM_IMPORT => self
                .lcop(p)
                .or_else(|| {
                    let q = skip_blanks(self.b, p);
                    if !self.is(q, b'(') || q >= self.b.len() {
                        return None;
                    }
                    let k = self.k(L::ImportListBegin);
                    self.put(p, q, self.w());
                    self.then(q, q + 1, k, &[IMPORT_PAREN])
                })
                .or_else(|| self.shift(p, &[IMPORT_LIST])),
            IMPORT_PAREN => self
                .comments(p)
                .or_else(|| {
                    if self.is(p, b')') {
                        let k = self.k(L::ImportListEnd);
                        return self.close(p, p + 1, k);
                    }
                    None
                })
                .or_else(|| self.import_names(p))
                .or_else(|| self.pop_at_nonspace(p)),
            IMPORT_LIST => self
                .lcop(p)
                .or_else(|| self.import_names(p))
                .or_else(|| self.pop_at_nonspace(p)),
            IMPORT_PATH => self.import_path(p),
            FOR => self
                .lcop(p)
                .or_else(|| {
                    let e = self.kw(p, b"in")?;
                    let k = self.k(L::ForIn);
                    self.then(p, e, k, &[FOR_IN])
                })
                .or_else(|| {
                    let k = self.k(L::MissingIn);
                    self.colon(p).and_then(|e| self.close(p, e, k))
                })
                .or_else(|| self.target_list(p)),
            FOR_IN => self
                .lcop(p)
                .or_else(|| {
                    let k = self.k(L::ForBlock);
                    self.colon(p).and_then(|e| self.close(p, e, k))
                })
                .or_else(|| self.in_statement(p)),
            WITH => self
                .lcop(p)
                .or_else(|| {
                    let e = self.kw(p, b"as")?;
                    let k = self.k(L::WithAs);
                    self.then(p, e, k, &[WITH_AS])
                })
                .or_else(|| {
                    let k = self.k(L::WithBlock);
                    self.colon(p).and_then(|e| self.close(p, e, k))
                })
                .or_else(|| {
                    self.is(p, b',')
                        .then(|| self.tok(p, p + 1, L::WithSeparator))
                        .flatten()
                })
                .or_else(|| self.in_statement(p)),
            WITH_AS => self
                .lcop(p)
                .or_else(|| match self.ch(p)? {
                    b':' => {
                        let k = self.k(L::WithBlock);
                        self.close(p, p + 1, k)
                    }
                    b',' => {
                        let k = self.k(L::WithSeparator);
                        self.then(p, p + 1, k, &[WITH])
                    }
                    _ => None,
                })
                .or_else(|| self.name(p))
                .or_else(|| self.groups(p))
                .or_else(|| self.lists(p)),
            EXCEPT => self
                .lcop(p)
                .or_else(|| {
                    let k = self.k(L::ExceptBlock);
                    self.colon(p).and_then(|e| self.close(p, e, k))
                })
                .or_else(|| {
                    let e = self.kw(p, b"as")?;
                    let k = self.k(L::ExceptAs);
                    self.then(p, e, k, &[EXCEPT_AS])
                })
                .or_else(|| self.target_list(p)),
            EXCEPT_AS => self
                .lcop(p)
                .or_else(|| {
                    self.is(p, b':')
                        .then(|| {
                            let k = self.k(L::ExceptBlock);
                            self.close(p, p + 1, k)
                        })
                        .flatten()
                })
                .or_else(|| self.name(p)),
            IF | WHILE => self
                .lcop(p)
                .or_else(|| {
                    let k = self.k(if c == IF { L::IfBlock } else { L::WhileBlock });
                    self.colon(p).and_then(|e| self.close(p, e, k))
                })
                .or_else(|| self.in_statement(p)),
            ELIF => {
                let k = self.k(L::ElifBlock);
                self.colon(p)
                    .and_then(|e| self.close(p, e, k))
                    .or_else(|| match p.cmp(&self.b.len()) {
                        // `$\n?`, after the `\n` too
                        std::cmp::Ordering::Equal => self.close(p, p + 1, 0),
                        std::cmp::Ordering::Greater => self.leave(p),
                        std::cmp::Ordering::Less => None,
                    })
                    .or_else(|| self.in_statement(p))
            }
            CLASS => self
                .lcop(p)
                .or_else(|| match self.ch(p)? {
                    b':' => {
                        let k = self.k(L::ClassBegin);
                        self.close(p, p + 1, k)
                    }
                    b'(' => None,
                    _ => {
                        self.ident(p)?;
                        self.enter(p, &[CLASS_NAME])
                    }
                })
                .or_else(|| {
                    self.is(p, b'(')
                        .then(|| {
                            let k = self.k(L::InheritanceBegin);
                            self.then(p, p + 1, k, &[CLASS_BASES])
                        })
                        .flatten()
                }),
            CLASS_NAME => self
                .illegal_names(p)
                .or_else(|| self.generic(p))
                .or_else(|| self.leave(p)),
            CLASS_BASES => self.class_bases(p),
            CLASS_AFTER => self
                .lcop(p)
                .or_else(|| {
                    self.is(p, b':')
                        .then(|| {
                            let k = self.k(L::ClassBegin);
                            self.close(p, p + 1, k)
                        })
                        .flatten()
                })
                .or_else(|| self.pop_at_nonspace(p)),
            INHERITED => self
                .ident(p)
                .map(|e| {
                    let q = skip_blanks(self.b, e);
                    if self.is(q, b'.') && q < self.b.len() {
                        let r = skip_blanks(self.b, q + 1);
                        let k = self.k(L::Inherited);
                        self.put(p, q, k);
                        let k = self.k(L::InheritedDot);
                        self.put(q, q + 1, k);
                        let k = self.k(L::Inherited);
                        self.put(q + 1, r, k);
                        r
                    } else {
                        let k = self.k(L::Inherited);
                        self.put(p, e, k);
                        e
                    }
                })
                .or_else(|| self.leave(p)),
            FUNCTION => self.lcop(p).or_else(|| match self.ch(p)? {
                b':' => {
                    let k = self.k(L::FunctionBegin);
                    self.close(p, p + 1, k)
                }
                b'(' => self.shift(p, &[FUNC_OPEN]),
                _ => {
                    self.ident(p)?;
                    self.enter(p, &[FUNC_NAME])
                }
            }),
            FUNC_NAME => self
                .magic_function(p)
                .or_else(|| self.illegal_names(p))
                .or_else(|| self.generic(p))
                .or_else(|| self.leave(p)),
            FUNC_OPEN => self
                .is(p, b'(')
                .then(|| {
                    let k = self.k(L::ParametersBegin);
                    if self.unpack_needed(p + 1) {
                        self.then(p, p + 1, k, &[PARAMS, UNPACK])
                    } else {
                        self.then(p, p + 1, k, &[PARAMS])
                    }
                })
                .flatten(),
            PARAMS => self.params(p),
            POSONLY => match self.ch(p) {
                Some(b',' | b')') => self.leave(p),
                _ if self.nonspace(p) => {
                    let e = self.next(p);
                    self.tok(p, e, L::ExpectedComma)
                }
                _ => None,
            },
            DEFAULT_EQ => self
                .is(p, b'=')
                .then(|| {
                    let k = self.k(L::Assignment);
                    self.then(p, p + 1, k, &[DEFAULT])
                })
                .flatten(),
            DEFAULT => match self.ch(p) {
                Some(b',' | b')') => self.shift(p, &[PARAMS]),
                _ => self.illegal_walrus(p).or_else(|| self.in_group(p)),
            },
            ANN_COLON => self
                .is(p, b':')
                .then(|| {
                    let k = self.k(L::ParameterAnnotation);
                    self.then(p, p + 1, k, &[PARAM_ANN])
                })
                .flatten(),
            PARAM_ANN => match self.ch(p) {
                Some(b',' | b')' | b'=') => self.shift(p, &[PARAMS]),
                _ => self.illegal_walrus(p).or_else(|| self.in_group(p)),
            },
            PTUPLE => self.params_tuple(p),
            PT_DEFAULT | PT_ANN => match self.ch(p) {
                Some(b',' | b')' | b'=') => self.leave(p),
                _ => None,
            },
            FUNC_AFTER => self
                .has(p, b"->")
                .then(|| self.shift(p, &[RETURN_ANN]))
                .flatten()
                .or_else(|| {
                    self.is(p, b':')
                        .then(|| {
                            let k = self.k(L::FunctionBegin);
                            self.close(p, p + 1, k)
                        })
                        .flatten()
                })
                .or_else(|| self.comments(p))
                .or_else(|| self.pop_at_nonspace(p)),
            RETURN_ANN => self
                .has(p, b"->")
                .then(|| self.tok(p, p + 2, L::ReturnAnnotation))
                .flatten()
                .or_else(|| self.illegal_walrus(p))
                .or_else(|| {
                    self.is(p, b':')
                        .then(|| self.shift(p, &[FUNC_AFTER]))
                        .flatten()
                })
                .or_else(|| self.lcop(p))
                .or_else(|| self.in_statement(p)),
            UNPACK => self.comments(p).or_else(|| {
                if self.is(p, b'*') && p < self.b.len() {
                    let e = (p..self.b.len())
                        .find(|&q| self.b[q] != b'*')
                        .unwrap_or(self.b.len());
                    let (e, l) = match e - p {
                        1 => (e, L::UnpackSequence),
                        2 => (e, L::UnpackMapping),
                        _ => (e, L::IllegalSyntax),
                    };
                    let k = self.k(l);
                    return self.close(p, e, k);
                }
                self.pop_at_nonspace(p)
            }),
            DECORATOR => self.decorator(p),
            DEC_WRAP => {
                if self.is(p, b'.') && p < self.b.len() {
                    let k = self.k(L::Dot);
                    self.put(p, p + 1, k);
                    let q = self.spaces(p + 1);
                    let w = self.w();
                    self.then(p + 1, q, w, &[DEC_WRAP_DOT])
                } else {
                    self.shift(p, &[DEC_WRAP_NAME])
                }
            }
            DEC_WRAP_DOT | DEC_CALL_DOT | CALL_DOT | LEAF_DOT => self
                .dotted_specials(p, c)
                .or_else(|| self.generic(p))
                .or_else(|| self.leave(p)),
            DEC_WRAP_NAME | DEC_CALL_NAME | CALL_NAME | LEAF_NAME => self
                .name_specials(p, c)
                .or_else(|| self.generic(p))
                .or_else(|| self.leave(p)),
            DEC_CALL => match self.ch(p) {
                Some(b')') => {
                    let k = self.k(L::ArgsEnd);
                    self.then_after(p, p + 1, k)
                }
                Some(b'(') => {
                    let k = self.k(L::ArgsBegin);
                    self.tok_push(p, p + 1, k, &[DEC_ARGS])
                }
                Some(b'.') if p < self.b.len() => {
                    let k = self.k(L::Dot);
                    let q = self.spaces(p + 1);
                    self.put(p, p + 1, k);
                    let w = self.w();
                    self.tok_push(p + 1, q, w, &[DEC_CALL_DOT])
                }
                _ => self.enter(p, &[DEC_CALL_NAME]),
            },
            DEC_ARGS => {
                if self.is(p, b')') {
                    self.leave(p)
                } else {
                    self.arguments(p)
                }
            }
            QN_LEAF => self.qn_leaf(p),
            ITEM => match self.ch(p) {
                Some(b']') => {
                    let k = self.k(L::BracketsEnd);
                    self.then_after(p, p + 1, k)
                }
                Some(b'[') => {
                    let k = self.k(L::BracketsBegin);
                    self.tok_push(p, p + 1, k, &[ITEM_ARGS])
                }
                _ => {
                    if self.path_then(p, Self::bracket_follows) {
                        self.enter(p, &[ITEM_NAME])
                    } else {
                        None
                    }
                }
            },
            ITEM_NAME => {
                if self.bracket_follows(p) {
                    self.leave(p)
                } else {
                    self.qualified_name(p)
                }
            }
            ITEM_ARGS => match self.ch(p) {
                Some(b']') => self.leave(p),
                Some(b':') => self.tok(p, p + 1, L::Slice),
                _ => self.in_group(p),
            },
            CALL => match self.ch(p) {
                Some(b'(') => self.shift(p, &[CALL_OPEN]),
                Some(b'.') if p < self.b.len() => {
                    let q = self.spaces(p + 1);
                    self.ident(q)?;
                    let k = self.k(L::Dot);
                    self.put(p, p + 1, k);
                    let w = self.w();
                    self.tok_push(p + 1, q, w, &[CALL_DOT])
                }
                _ => {
                    self.ident(p)?;
                    self.enter(p, &[CALL_NAME])
                }
            },
            CALL_OPEN => self
                .is(p, b'(')
                .then(|| {
                    let k = self.k(L::ArgsBegin);
                    self.then(p, p + 1, k, &[ARGS])
                })
                .flatten(),
            ARGS | AFTER_ARGS => {
                if self.is(p, b')') {
                    let k = self.k(L::ArgsEnd);
                    if c == ARGS {
                        self.then_after(p, p + 1, k)
                    } else {
                        self.close(p, p + 1, k)
                    }
                } else {
                    self.arguments(p)
                }
            }
            KWARG => self
                .lcop(p)
                .or_else(|| {
                    self.is(p, b'=')
                        .then(|| {
                            let k = self.k(L::Assignment);
                            self.then(p, p + 1, k, &[KWARG_VALUE])
                        })
                        .flatten()
                })
                .or_else(|| self.illegal_names(p))
                .or_else(|| self.parameter(p)),
            KWARG_VALUE => self.illegal_walrus(p).or_else(|| match self.ch(p) {
                Some(b',' | b')' | b':') => self.leave(p),
                _ => self.in_group(p),
            }),
            LAMBDA => self.lambda_params(p),
            LAMBDA_COLON => self.shift(p, &[LAMBDA_BODY]),
            LAMBDA_BODY => self.illegal_walrus(p).or_else(|| {
                if matches!(self.ch(p), Some(b',' | b')' | b':')) || self.eol(p) {
                    return self.leave(p);
                }
                self.in_statement(p)
            }),
            GROUP | TUPLE | LIST => self.sequence(p, c),
            MAPSET => self.mapset(p),
            DICT => self.dict(p),
            DICT_UNPACK | SET_UNPACK => match self.ch(p) {
                Some(b'}') => self.leave(p),
                Some(b',') => {
                    let k = self.k(if c == DICT_UNPACK {
                        L::MappingSeparator
                    } else {
                        L::SetSeparator
                    });
                    self.close(p, p + 1, k)
                }
                _ => self.in_group(p),
            },
            DICT_KEY => {
                let q = self.spaces(p);
                if matches!(self.ch(q), Some(b'}' | b',' | b':')) && q < self.b.len() {
                    if q == p {
                        if let Some(e) = self.leave(p) {
                            return Some(e);
                        }
                    } else {
                        let w = self.w();
                        return self.close(p, q, w);
                    }
                }
                self.in_group(p)
            }
            DICT_VALUE => self.dict_value(p),
            DICT_COMMA => self
                .is(p, b',')
                .then(|| {
                    let k = self.k(L::MappingSeparator);
                    self.then(p, p + 1, k, &[DICT])
                })
                .flatten(),
            DICT_FOR => match self.ch(p) {
                Some(b'}') => self.leave(p),
                Some(b',') => self.tok(p, p + 1, L::UnexpectedComma),
                _ => self.inline_for(p).or_else(|| self.in_group(p)),
            },
            DICT_VAL => {
                let q = self.spaces(p);
                if ((matches!(self.ch(q), Some(b'}' | b',')) && q < self.b.len())
                    || self.has(q, b"for") && !is_word(self.b, q + 3))
                    && let Some(e) = self.leave(p)
                {
                    return Some(e);
                }
                self.in_group(p)
            }
            SET => self.set_body(p),
            AFTER => self.after(p),
            AFTER_ITEM => match self.ch(p) {
                Some(b']') => {
                    let k = self.k(L::BracketsEnd);
                    self.close(p, p + 1, k)
                }
                _ => self.illegal_walrus(p).or_else(|| {
                    if self.is(p, b':') {
                        return self.tok(p, p + 1, L::Slice);
                    }
                    self.in_group(p)
                }),
            },
            ATTR => self
                .magic_function(p)
                .or_else(|| self.magic_variable(p))
                .or_else(|| self.illegal_names(p))
                .or_else(|| self.generic(p))
                .or_else(|| self.leave(p)),
            MODIFIER => self
                .lcop(p)
                .or_else(|| {
                    self.is(p, b',')
                        .then(|| self.tok(p, p + 1, L::StorageSeparator))
                        .flatten()
                })
                .or_else(|| self.name(p))
                .or_else(|| {
                    if !self.nonspace(p) {
                        return None;
                    }
                    let e = (p..self.b.len())
                        .find(|&q| space_len(self.b, q).is_some())
                        .unwrap_or(self.b.len());
                    self.tok(p, e, L::StorageIllegal)
                }),
            NAME | DOTTED => {
                let s = if c == NAME {
                    self.name_specials(p, c)
                } else {
                    self.dotted_specials(p, c)
                };
                s.or_else(|| {
                    let e = self.word(p)?;
                    if !is_constant_name(&self.b[p..e]) {
                        return None;
                    }
                    self.tok(p, e, L::NameConstant)
                })
                .or_else(|| self.generic(p))
                .or_else(|| self.leave(p))
            }
            QN => self
                .name(p)
                .or_else(|| self.dotted_name(p))
                .or_else(|| self.leave(p)),
            INLINE_FOR => self
                .comments(p)
                .or_else(|| {
                    let e = self.kw(p, b"in")?;
                    let k = self.k(L::ForIn);
                    self.close(p, e, k)
                })
                .or_else(|| {
                    matches!(self.ch(p), Some(b')' | b']' | b'}'))
                        .then(|| self.leave(p))
                        .flatten()
                })
                .or_else(|| self.illegal_names_pop(p))
                .or_else(|| self.target_list(p)),
            TARGETS => self
                .comments(p)
                .or_else(|| match self.ch(p)? {
                    b',' => self.tok(p, p + 1, L::TargetSeparator),
                    b')' => {
                        let k = self.k(L::TargetEnd);
                        self.close(p, p + 1, k)
                    }
                    _ => None,
                })
                .or_else(|| self.target_list(p))
                .or_else(|| self.name(p)),
            DOC_START => self.doc_start(p),
            DOC_DQ | DOC_SQ | DOC_RAW_DQ | DOC_RAW_SQ => self.docstring(p, c),
            FREPL..=PH_SPEC => self.fstring(p, c),
            _ => None,
        }
    }

    /// `(?=\S)` pop.
    fn pop_at_nonspace(&mut self, p: usize) -> Option<usize> {
        if self.nonspace(p) {
            self.leave(p)
        } else {
            None
        }
    }

    /// `:(?!=)` at `p`: where it ends.
    fn colon(&self, p: usize) -> Option<usize> {
        (self.is(p, b':') && !self.is(p + 1, b'=')).then_some(p + 1)
    }

    /// `(?=\s*\[)` at `p`.
    fn bracket_follows(&self, p: usize) -> bool {
        self.is(self.spaces(p), b'[')
    }

    /// `(?=\s*\()` at `p`.
    fn paren_follows(&self, p: usize) -> bool {
        self.is(self.spaces(p), b'(')
    }

    /// `statements`.
    fn main(&mut self, p: usize) -> Option<usize> {
        let c = self.ch(p)?;
        if p == 0 {
            // `docstrings`, `decorators`, `classes`, `functions`: the line's start, `^\s*`
            let q = self.spaces(0);
            if q < self.b.len()
                && matches!(self.b[q], b'"' | b'\'' | b'u' | b'U' | b'r' | b'R')
                && let Some(r) = self.docstring_start(0)
            {
                return Some(r);
            }
            if q > 0 {
                // no other pattern of `statements` starts at a space
                if self.is(q, b'@') && q < self.b.len() {
                    return self.tok_push(0, q, self.w(), &[DECORATOR]);
                }
                return self.class_or_def(q);
            }
        }
        if let Some(e) = self.word(p) {
            if let Some(q) = self.statement_word(p, e) {
                return Some(q);
            }
        } else if p == 0 {
            let q = self.spaces(0);
            if self.is(q, b'@') && q < self.b.len() {
                return self.tok_push(p, q, self.w(), &[DECORATOR]);
            }
            if let Some(r) = self.class_or_def(q) {
                return Some(r);
            }
        }
        // `assignments`
        let assign = match c {
            b':' if self.is(p + 1, b'=') => Some((p + 2, L::NotAllowed)),
            b':' => Some((p + 1, L::VariableAnnotation)),
            b'+' | b'-' | b'%' | b'@' | b'&' | b'|' | b'^' if self.is(p + 1, b'=') => {
                Some((p + 2, L::Augmented))
            }
            b'*' | b'/' | b'>' | b'<' => {
                if self.is(p + 1, b'=') && c != b'>' && c != b'<' {
                    Some((p + 2, L::Augmented))
                } else if self.is(p + 1, c) && self.is(p + 2, b'=') {
                    Some((p + 3, L::Augmented))
                } else {
                    None
                }
            }
            b'=' if !self.is(p + 1, b'=') => Some((p + 1, L::Assignment)),
            b';' => Some((p + 1, L::Terminator)),
            _ => None,
        };
        if let Some((e, l)) = assign {
            return self.tok(p, e, l);
        }
        self.common(p).or_else(|| self.qualified_name(p))
    }

    /// The patterns of `statements` that start with a word at `p`, to `e`, before
    /// `assignments`.
    fn statement_word(&mut self, p: usize, e: usize) -> Option<usize> {
        use words::*;
        let kw = self.info(p).kw;
        let (l, push): (L, &[u8]) = match kw {
            K_IMPORT => (L::Import, &[IMPORT]),
            K_FROM => (L::ImportFrom, &[FROM]),
            K_RAISE => (L::Raise, &[RAISE]),
            K_ASSERT => (L::Assert, &[]),
            K_DEL => (L::Del, &[]),
            K_PRINT => {
                let q = skip_blanks(self.b, e);
                if matches!(self.ch(q), Some(b',' | b'.' | b'(' | b')' | b']' | b'}')) {
                    return self.after_statement_words(p);
                }
                (L::Print, &[])
            }
            K_EXEC => {
                let q = skip_blanks(self.b, e);
                if self.eol(q)
                    || matches!(self.ch(q), Some(b',' | b'.' | b'(' | b')' | b']' | b'}'))
                {
                    return self.after_statement_words(p);
                }
                (L::Exec, &[])
            }
            K_RETURN => (L::Return, &[]),
            K_BREAK => (L::Break, &[]),
            K_CONTINUE => (L::Continue, &[]),
            K_PASS => (L::Pass, &[]),
            K_ASYNC => {
                // `\b(async +)?(for)\b`, `\b(async +)?(with)\b`
                let q = skip_blanks(self.b, e);
                if q > e
                    && let Some(f) = self.word(q)
                {
                    let push: &[u8] = match self.info(q).kw {
                        K_FOR => &[FOR],
                        K_WITH => &[WITH],
                        _ => &[],
                    };
                    if !push.is_empty() {
                        let k = self.k(L::Async);
                        self.put(p, q, k);
                        let k = self.k(if push[0] == FOR { L::For } else { L::With });
                        return self.tok_push(q, f, k, push);
                    }
                }
                return self.after_statement_words(p);
            }
            K_FOR => (L::For, &[FOR]),
            K_WITH => (L::With, &[WITH]),
            K_EXCEPT => (L::Except, &[EXCEPT]),
            K_IF => (L::If, &[IF]),
            K_WHILE => (L::While, &[WHILE]),
            K_ELSE | K_TRY | K_FINALLY => {
                let (kw, block) = match kw {
                    K_ELSE => (L::Else, L::ElseBlock),
                    K_TRY => (L::Try, L::TryBlock),
                    _ => (L::Finally, L::FinallyBlock),
                };
                self.tok(p, e, kw);
                let q = self.spaces(e);
                if self.is(q, b':') && q < self.b.len() {
                    let k = self.w();
                    self.put(e, q, k);
                    return self.tok(q, q + 1, block);
                }
                return Some(e);
            }
            K_ELIF => (L::Elif, &[ELIF]),
            _ => return self.after_statement_words(p),
        };
        let k = self.k(l);
        if push.is_empty() {
            self.put(p, e, k);
            Some(e)
        } else {
            self.tok_push(p, e, k, push)
        }
    }

    /// The patterns of `statements` after its keywords, at `p`, a word's start.
    fn after_statement_words(&mut self, p: usize) -> Option<usize> {
        if p == 0
            && let Some(q) = self.class_or_def(p)
        {
            return Some(q);
        }
        if let Some(q) = self.modifiers(p) {
            return Some(q);
        }
        self.common(p).or_else(|| self.qualified_name(p))
    }

    /// `^\s*(class)\b`, `^\s*(?:(async)\s+)?(def)\b`, with the line's spaces to `q`.
    fn class_or_def(&mut self, q: usize) -> Option<usize> {
        let e = self.word(q)?;
        match self.info(q).kw {
            words::K_CLASS => {
                let w = self.w();
                self.put(0, q, w);
                let k = self.k(L::ClassKeyword);
                self.tok_push(q, e, k, &[CLASS])
            }
            words::K_DEF => {
                let w = self.w();
                self.put(0, q, w);
                let k = self.k(L::DefKeyword);
                self.tok_push(q, e, k, &[FUNCTION])
            }
            words::K_ASYNC => {
                let d = self.spaces(e);
                if d == e {
                    return None;
                }
                let f = self.kw(d, b"def")?;
                let w = self.w();
                self.put(0, q, w);
                let k = self.k(L::Async);
                self.put(q, e, k);
                self.put(e, d, w);
                let k = self.k(L::DefKeyword);
                self.tok_push(d, f, k, &[FUNCTION])
            }
            _ => None,
        }
    }

    /// `modifiers`.
    fn modifiers(&mut self, p: usize) -> Option<usize> {
        let e = self.word(p)?;
        let l = match self.info(p).kw {
            words::K_GLOBAL => L::Global,
            words::K_NONLOCAL => L::Nonlocal,
            _ => return None,
        };
        let k = self.k(l);
        self.tok_push(p, e, k, &[MODIFIER])
    }

    /// `line-continuation`: `(\\)(.*)$\n?`, then the next line's start.
    fn line_continuation(&mut self, p: usize) -> Option<usize> {
        if !self.is(p, b'\\') || p >= self.b.len() {
            return None;
        }
        let k = self.k(L::Continuation);
        self.put(p, p + 1, k);
        let k = self.k(L::UnexpectedText);
        self.put(p + 1, self.b.len(), k);
        self.tok_push(self.b.len(), self.b.len() + 1, 0, &[LINE_CONT])
    }

    /// `line-continuation-or-pop`.
    fn lcop(&mut self, p: usize) -> Option<usize> {
        self.line_continuation(p).or_else(|| {
            let q = self.spaces(p);
            if self.eol(q) || matches!(self.ch(q), Some(b';' | b'#')) {
                self.leave(p)
            } else {
                None
            }
        })
    }

    /// `comments`.
    fn comments(&mut self, p: usize) -> Option<usize> {
        if self.is(p, b'#') && p < self.b.len() {
            let k = self.k(L::CommentMark);
            return self.tok_push(p, p + 1, k, &[COMMENT]);
        }
        None
    }

    /// `expressions-common`.
    fn common(&mut self, p: usize) -> Option<usize> {
        let c = self.ch(p)?;
        if c == b'#' {
            return self.comments(p);
        }
        let info = self.info(p);
        let word = (info.end > p).then_some(info.end);
        if info.kw == words::K_CONSTANT {
            return self.tok(p, info.end, L::Constant);
        }
        if c == b'.' && self.has(p, b"...") && !is_word(self.b, p + 3) {
            return self.tok(p, p + 3, L::Constant);
        }
        if (c.is_ascii_digit() || c == b'.' || c >= 0x80)
            && let Some((e, l, caps)) = number(&self.b[..self.b.len()], p)
        {
            let base = self.k(l);
            let mut at = p;
            for (s, ce, cl) in caps {
                if ce > s {
                    self.put(at, s, base);
                    let k = self.k(cl);
                    self.put(s, ce, k);
                    at = ce;
                }
            }
            self.put(at, e, base);
            return Some(e);
        }
        if let Some(e) = word {
            match info.kw {
                words::K_YIELD => {
                    let k = self.k(L::Yield);
                    let q = self.spaces(e);
                    if q > e && self.kw(q, b"from").is_some() {
                        self.put(p, e, k);
                        let w = self.w();
                        self.put(e, q, w);
                        return self.tok(q, q + 4, L::YieldFrom);
                    }
                    return self.tok(p, e, L::Yield);
                }
                words::K_LOGICAL | words::K_IN => return self.tok(p, e, L::Logical),
                words::K_LAMBDA => {
                    if self.space(e) || self.is(e, b':') || self.eol(e) {
                        let k = self.k(L::LambdaKeyword);
                        return self.tok_push(p, e, k, &[LAMBDA, UNPACK]);
                    }
                }
                words::K_AWAIT => return self.tok(p, e, L::Await),
                words::K_IF => return self.tok(p, e, L::If),
                words::K_ELSE => return self.tok(p, e, L::Else),
                _ => {}
            }
        }
        // a word: the rest of the patterns that can start with one
        if word.is_some() && c != b'.' {
            if let Some(q) = self.strings(p) {
                return Some(q);
            }
            // `function-calls`, `item-access`: one walk of the path for both lookaheads
            let (call, item, shape) = self.path_follows(p);
            if call {
                return self.call_as(p, shape);
            }
            if item {
                return self.enter(p, &[ITEM]);
            }
            return None;
        }
        // `operators`
        let op = match c {
            b'<' if self.is(p + 1, b'>') => Some((p + 2, L::Deprecated)),
            b'<' | b'>' if self.is(p + 1, b'=') => Some((p + 2, L::Comparison)),
            b'=' if self.is(p + 1, b'=') => Some((p + 2, L::Comparison)),
            b'!' if self.is(p + 1, b'=') => Some((p + 2, L::Comparison)),
            b'<' | b'>' => Some((p + 1, L::Comparison)),
            b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^' | b'~' => {
                Some((p + 1, L::Arithmetic))
            }
            b'@' => Some((p + 1, L::Matrix)),
            _ => None,
        };
        if let Some((e, l)) = op {
            return self.tok(p, e, l);
        }
        if let Some(q) = self.strings(p) {
            return Some(q);
        }
        // `function-calls`: no word starts here (a word went above), so only after a `.`
        if c == b'.' && self.call_follows(p) {
            return self.call(p);
        }
        // `item-access` needs a word here, which went above
        match c {
            b'[' => return self.lists(p),
            b'{' => return self.dicts(p),
            b'(' => {
                return self.tuples(p).or_else(|| self.groups(p));
            }
            b')' => return self.tok(p, p + 1, L::StrayRound),
            b']' => return self.tok(p, p + 1, L::StraySquare),
            b'}' => return self.tok(p, p + 1, L::StrayCurly),
            b'\\' => return self.line_continuation(p),
            _ => {}
        }
        None
    }

    /// `(?=(\.\s*)?{{path}}\s*\()` at `p`.
    fn call_follows(&self, p: usize) -> bool {
        let q = if self.is(p, b'.') && p < self.b.len() {
            self.spaces(p + 1)
        } else {
            p
        };
        self.path_then(q, Self::paren_follows)
    }

    /// `function-calls`, after `(?=(\.\s*)?{{path}}\s*\()` matched at `p`: pushing
    /// `function-call-wrapper` and `qualified-name-until-leaf`. Done here without the pushes
    /// for its usual shape, `.`? then names between dots (` ` around them) and `\s*(`: the
    /// leading names as `qualified-name-until-leaf`'s contexts give them, the last as the
    /// call's, the `(` opening the arguments, and the loop guard's record the pushes leave;
    /// else the contexts are pushed.
    fn call(&mut self, p: usize) -> Option<usize> {
        let shape = || -> Option<CallShape> {
            let lead = self.is(p, b'.');
            let mut q = if lead { skip_blanks(self.b, p + 1) } else { p };
            // each name's start, then the `(`
            let mut names = [0; 8];
            let mut k = 0;
            loop {
                let w = self.info(q);
                if w.ident <= q || w.ident != w.end || k == names.len() {
                    return None;
                }
                names[k] = q;
                k += 1;
                let e = w.end;
                let dot = skip_blanks(self.b, e);
                if dot < self.b.len() && self.b[dot] == b'.' {
                    q = skip_blanks(self.b, dot + 1);
                    continue;
                }
                // only spaces before the `(`, as `qualified-name-until-leaf`'s `\s*` has them
                return (dot < self.b.len() && self.b[dot] == b'(').then_some(CallShape {
                    lead,
                    names,
                    k,
                    paren: dot,
                });
            }
        };
        let shape = shape();
        self.call_as(p, shape)
    }

    /// [`call`](Self::call) at `p` with its shape known: `None` where it doesn't hold, and
    /// the contexts are pushed.
    fn call_as(&mut self, p: usize, shape: Option<CallShape>) -> Option<usize> {
        let d = self.depth;
        let Some(CallShape {
            lead,
            names,
            k,
            paren,
        }) = shape.filter(|_| d + 3 < SYNTECT_DEPTH.min(MAX))
        else {
            return self.enter(p, &[CALL, QN_LEAF]);
        };
        let mut loop_at = (p, d + 1);
        let dot_before = |s: &Self, q: usize| {
            let mut i = q;
            while i > 0 && s.b[i - 1] == b' ' {
                i -= 1;
            }
            i - 1
        };
        for (i, &q) in names[..k].iter().enumerate() {
            let last = i + 1 == k;
            let dotted = lead || i > 0;
            if dotted {
                let dot = if i == 0 { p } else { dot_before(self, q) };
                let k = self.k(L::Dot);
                self.put(dot, dot + 1, k);
            } else {
                // a non-consuming push: `qualified-name-until-leaf`'s name, or the call's
                loop_at = (q, d + 2);
            }
            let ctx = match (last, dotted) {
                (false, false) => LEAF_NAME,
                (false, true) => LEAF_DOT,
                (true, false) => CALL_NAME,
                (true, true) => CALL_DOT,
            };
            let e = self.info(q).end;
            let special = if dotted {
                self.dotted_specials(q, ctx)
            } else {
                self.name_specials(q, ctx)
            };
            if special.is_none() {
                let k = self.w() | text_leaf(ctx) as u16;
                self.put(q, e, k);
            }
        }
        let k = self.k(L::ArgsBegin);
        self.put(paren, paren + 1, k);
        self.push(ARGS);
        self.loop_at = loop_at;
        Some(paren + 1)
    }

    /// `lists`.
    fn lists(&mut self, p: usize) -> Option<usize> {
        if !self.is(p, b'[') || p >= self.b.len() {
            return None;
        }
        let q = self.spaces(p + 1);
        if self.is(q, b']') && q < self.b.len() {
            let k = self.k(L::SequenceBegin);
            self.put(p, p + 1, k);
            let w = self.w();
            self.put(p + 1, q, w);
            let k = self.k(L::SequenceEnd);
            return self.push_after(q, q + 1, k);
        }
        let k = self.k(L::SequenceBegin);
        if self.unpack_needed(p + 1) {
            self.tok_push(p, p + 1, k, &[LIST, UNPACK])
        } else {
            self.tok_push(p, p + 1, k, &[LIST])
        }
    }

    /// `dictionaries-and-sets`.
    fn dicts(&mut self, p: usize) -> Option<usize> {
        if !self.is(p, b'{') || p >= self.b.len() {
            return None;
        }
        let q = self.spaces(p + 1);
        if self.is(q, b'}') && q < self.b.len() {
            let k = self.k(L::MappingBegin);
            self.put(p, p + 1, k);
            let w = self.w();
            self.put(p + 1, q, w);
            let k = self.k(L::MappingEnd);
            return self.push_after(q, q + 1, k);
        }
        let star = |s: &Self, n: usize| {
            let q = s.spaces(p + 1);
            (q..q + n).all(|i| s.is(i, b'*') && i < s.b.len())
        };
        // each test holds only at its byte, which the line's rest must have
        let rest = &self.b[(p + 1).min(self.b.len())..];
        if (rest.contains(&b':')
            && self.simple_expression(p + 1, &|s: &Self, q| s.is(q, b':') && q < s.b.len()))
            || star(self, 2)
        {
            let k = self.k(L::MappingBegin);
            return self.tok_push(p, p + 1, k, &[DICT]);
        }
        if (rest.iter().any(|&c| c == b',' || c == b'}')
            && self.simple_expression(p + 1, &|s: &Self, q| {
                matches!(s.ch(q), Some(b',' | b'}')) && q < s.b.len()
            }))
            || star(self, 1)
        {
            let k = self.k(L::SetBegin);
            return self.tok_push(p, p + 1, k, &[SET]);
        }
        let k = self.k(L::MappingOrSetBegin);
        self.tok_push(p, p + 1, k, &[MAPSET])
    }

    /// `tuples`.
    fn tuples(&mut self, p: usize) -> Option<usize> {
        let q = self.spaces(p + 1);
        if self.is(q, b')') && q < self.b.len() {
            let k = self.k(L::SequenceBegin);
            self.put(p, p + 1, k);
            let w = self.w();
            self.put(p + 1, q, w);
            let k = self.k(L::SequenceEnd);
            return self.push_after(q, q + 1, k);
        }
        let star_path = self.is(q, b'*') && q < self.b.len() && self.ident(q + 1).is_some();
        // the test holds only at a `,`, which the line's rest must have
        if star_path
            || (self.b[(p + 1).min(self.b.len())..].contains(&b',')
                && self.simple_expression(p + 1, &|s: &Self, q| s.is(q, b',') && q < s.b.len()))
        {
            let k = self.k(L::SequenceBegin);
            return self.tok_push(p, p + 1, k, &[TUPLE]);
        }
        None
    }

    /// `groups`.
    fn groups(&mut self, p: usize) -> Option<usize> {
        if !self.is(p, b'(') || p >= self.b.len() {
            return None;
        }
        let k = self.k(L::GroupBegin);
        self.tok_push(p, p + 1, k, &[GROUP])
    }

    /// Whether `{{simple_expression}}` at `p` can end where `then` holds: every way the
    /// grammar's backtracking could take it. Each of its pieces ends past where it starts,
    /// so the places it can reach are taken in order, once each, from a set of bits (on the
    /// stack for a line's first 1,024 places after `p`).
    #[inline(never)]
    fn simple_expression(&self, p: usize, then: &dyn Fn(&Self, usize) -> bool) -> bool {
        let n = self.b.len();
        let b = self.b;
        if p > n + 1 {
            return false;
        }
        // places reached, `p..=n + 1`, a bit each
        let len = n + 2 - p;
        let mut small = [0u64; 16];
        let mut big = Vec::new();
        let seen: &mut [u64] = if len <= 1024 {
            &mut small
        } else {
            big.resize(len.div_ceil(64), 0);
            &mut big
        };
        // the furthest place reached
        let mut last = p;
        let mark = |q: usize, seen: &mut [u64], last: &mut usize| {
            if q <= n + 1 {
                seen[(q - p) / 64] |= 1 << ((q - p) % 64);
                *last = (*last).max(q);
            }
        };
        mark(p, seen, &mut last);
        let mut q = p;
        while q <= last {
            if seen[(q - p) / 64] & (1 << ((q - p) % 64)) == 0 {
                q += 1;
                continue;
            }
            if then(self, q) {
                return true;
            }
            if q > n {
                break;
            }
            // `\s+`
            let mut s = q;
            while self.space(s) && s <= n {
                s = if s == n {
                    n + 1
                } else {
                    s + space_len(b, s).unwrap_or(1)
                };
                mark(s, seen, &mut last);
            }
            // `[urfb]*"(?:\\.|[^"])*"` and its `'` twin: the content reaches every place up to
            // the first quote not after a `\\` (one after it, the `\\.` takes), and each quote
            // it reaches ends it
            let mut r = q;
            loop {
                if let Some(quote @ (b'"' | b'\'')) = self.ch(r)
                    && r < n
                {
                    for i in r + 1..n {
                        if b[i] == quote {
                            mark(i + 1, seen, &mut last);
                            if b[i - 1] != b'\\' {
                                break;
                            }
                        }
                    }
                }
                if matches!(self.ch(r), Some(b'u' | b'r' | b'f' | b'b')) && r < n {
                    r += 1;
                } else {
                    break;
                }
            }
            // `[\d.ej]+`
            let mut s = q;
            loop {
                let e = match self.ch(s) {
                    Some(b'.' | b'e' | b'j') if s < n => s + 1,
                    _ => match digit(&b[..n], s) {
                        Some(e) => e,
                        None => break,
                    },
                };
                mark(e, seen, &mut last);
                s = e;
            }
            // `[+*/%@-] | // | and | or`
            if matches!(self.ch(q), Some(b'+' | b'*' | b'/' | b'%' | b'@' | b'-')) && q < n {
                mark(q + 1, seen, &mut last);
            }
            if self.has(q, b"//") {
                mark(q + 2, seen, &mut last);
            }
            if self.has(q, b"and") {
                mark(q + 3, seen, &mut last);
            }
            if self.has(q, b"or") {
                mark(q + 2, seen, &mut last);
            }
            // `{{path}}`: each of its ends
            let mut s = q;
            while let Some(e) = self.ident(s) {
                mark(e, seen, &mut last);
                let d = skip_blanks(b, e);
                if !self.is(d, b'.') || d >= n {
                    break;
                }
                s = skip_blanks(b, d + 1);
            }
            q += 1;
        }
        false
    }

    /// `expression-in-a-statement`.
    fn in_statement(&mut self, p: usize) -> Option<usize> {
        self.common(p)
            .or_else(|| self.illegal_names_pop(p))
            .or_else(|| self.qualified_name(p))
            .or_else(|| {
                self.has(p, b":=")
                    .then(|| self.tok(p, p + 2, L::InlineAssignment))
                    .flatten()
            })
    }

    /// `expression-in-a-group`.
    fn in_group(&mut self, p: usize) -> Option<usize> {
        self.in_statement(p).or_else(|| {
            if !self.is(p, b'.') || p >= self.b.len() {
                return None;
            }
            let q = skip_blanks(self.b, p + 1);
            self.ident(q)?;
            let k = self.k(L::Dot);
            self.put(p, p + 1, k);
            let w = self.w();
            self.tok_push(p + 1, q, w, &[ATTR])
        })
    }

    /// `illegal-assignment-expression`.
    fn illegal_walrus(&mut self, p: usize) -> Option<usize> {
        self.has(p, b":=")
            .then(|| self.tok(p, p + 2, L::NotAllowed))
            .flatten()
    }

    /// `qualified-name`: `(?={{path}})` pushing its context, which pushes `name` and
    /// `dotted-name`'s, each popping after its name. Done here without the pushes where that
    /// comes out the same: each name's token, the dots between, and the loop guard's record
    /// that the pushes would leave; else the contexts are pushed as syntect has them at the
    /// place, and the scan goes on in them.
    fn qualified_name(&mut self, p: usize) -> Option<usize> {
        self.ident(p)?;
        let d = self.depth;
        if d + 3 >= SYNTECT_DEPTH || d + 3 >= MAX {
            return self.enter(p, &[QN]);
        }
        let mut q = self.name_token(p, NAME);
        let mut inner = NAME;
        loop {
            // the name's context, then `qualified-name`'s, at `q`: a name that starts there
            // is the machinery's
            let plain = q >= self.b.len() || (self.b[q] < 0x80 && !WORD[usize::from(self.b[q])]);
            if !plain && (self.word(q).is_some() || self.is(q, b'_') || self.ident(q).is_some()) {
                self.fill(q);
                self.push(QN);
                self.push(inner);
                self.loop_at = (p, d + 2);
                return Some(q);
            }
            // `dotted-name`: `\s*(\.)\s*(?={{identifier}})`
            let dot = self.spaces(q);
            if !self.is(dot, b'.') || dot >= self.b.len() {
                break;
            }
            let r = self.spaces(dot + 1);
            if self.ident(r).is_none() {
                break;
            }
            let w = self.w();
            self.put(q, dot, w);
            let k = self.k(L::Dot);
            self.put(dot, dot + 1, k);
            self.put(dot + 1, r, w);
            q = self.name_token(r, DOTTED);
            inner = DOTTED;
        }
        self.loop_at = (p, d + 2);
        Some(q)
    }

    /// The token `name`'s context (or `dotted-name`'s, `c`) gives the name at `p`: where it
    /// ends.
    fn name_token(&mut self, p: usize, c: u8) -> usize {
        let special = if c == NAME {
            self.name_specials(p, c)
        } else {
            self.dotted_specials(p, c)
        };
        if let Some(e) = special {
            return e;
        }
        if let Some(e) = self.word(p)
            && is_constant_name(&self.b[p..e])
        {
            let k = self.k(L::NameConstant);
            self.put(p, e, k);
            return e;
        }
        let e = self.ident(p).unwrap_or(p);
        let k = self.w();
        self.put(p, e, k);
        e
    }

    /// `name`.
    fn name(&mut self, p: usize) -> Option<usize> {
        self.ident(p)?;
        self.enter(p, &[NAME])
    }

    /// `dotted-name`: `\s*(\.)\s*(?={{identifier}})`.
    fn dotted_name(&mut self, p: usize) -> Option<usize> {
        let d = self.spaces(p);
        if !self.is(d, b'.') || d >= self.b.len() {
            return None;
        }
        let q = self.spaces(d + 1);
        self.ident(q)?;
        let w = self.w();
        self.put(p, d, w);
        let k = self.k(L::Dot);
        self.put(d, d + 1, k);
        self.tok_push(d + 1, q, w, &[DOTTED])
    }

    /// The leaf group of names in context `c`.
    fn names_in(c: u8) -> L {
        match c {
            CALL_DOT | CALL_NAME => L::Call,
            DEC_WRAP_DOT | DEC_WRAP_NAME => L::Ann,
            DEC_CALL_DOT | DEC_CALL_NAME => L::AnnCall,
            CLASS_NAME => L::ClassName,
            FUNC_NAME => L::DefName,
            _ => L::Name,
        }
    }

    /// Name `which` (`words::…`) at `p..e` in context `c`.
    fn name_tok(&mut self, p: usize, e: usize, c: u8, which: u16) -> Option<usize> {
        let k = self.w() | (Self::names_in(c) as u16 + which);
        self.put(p, e, k);
        Some(e)
    }

    /// `name-specials` at `p`.
    fn name_specials(&mut self, p: usize, c: u8) -> Option<usize> {
        let w = self.info(p);
        if w.end > p && w.special != words::GENERIC {
            return self.name_tok(p, w.end, c, w.special);
        }
        // `_(?!{{identifier_continue}})`
        if self.is(p, b'_') && p < self.b.len() && !is_word(self.b, p + 1) {
            return self.name_tok(p, p + 1, c, words::LANGUAGE);
        }
        None
    }

    /// `dotted-name-specials` at `p`.
    fn dotted_specials(&mut self, p: usize, c: u8) -> Option<usize> {
        let w = self.info(p);
        if w.end > p
            && matches!(
                w.special,
                words::MAGIC_FUNCTION | words::MAGIC_VARIABLE | words::ILLEGAL
            )
        {
            return self.name_tok(p, w.end, c, w.special);
        }
        None
    }

    /// A word of name class `which` at `p`, in the top context.
    fn special(&mut self, p: usize, which: u16) -> Option<usize> {
        let w = self.info(p);
        if w.end <= p || w.special != which {
            return None;
        }
        let c = self.top();
        self.name_tok(p, w.end, c, which)
    }

    fn magic_function(&mut self, p: usize) -> Option<usize> {
        self.special(p, words::MAGIC_FUNCTION)
    }

    fn magic_variable(&mut self, p: usize) -> Option<usize> {
        self.special(p, words::MAGIC_VARIABLE)
    }

    /// `illegal-names`.
    fn illegal_names(&mut self, p: usize) -> Option<usize> {
        self.special(p, words::ILLEGAL)
    }

    /// `illegal-names-pop`.
    fn illegal_names_pop(&mut self, p: usize) -> Option<usize> {
        let w = self.info(p);
        if w.end <= p || w.special != words::ILLEGAL {
            return None;
        }
        let k = self.pop_base() | L::NameIllegal as u16;
        self.close(p, w.end, k)
    }

    /// The kind of plain text a pop's token has: a context's `meta_content_scope` is off it,
    /// and scopes its `clear_scopes` took are still off (a decorator's arguments' clear its
    /// call's `meta.annotation.function`).
    fn pop_base(&self) -> u16 {
        let d = self.depth - 1;
        match self.top() {
            DEC_ARGS if d >= 1 => BASES[usize::from(self.raws[d - 1] & !(ANN as u8))],
            DECORATOR if d >= 1 => BASES[usize::from(self.raws[d - 1])],
            _ => self.w(),
        }
    }

    /// `generic-names`: the name in the top context's text.
    fn generic(&mut self, p: usize) -> Option<usize> {
        let e = self.ident(p)?;
        let k = self.text;
        self.put(p, e, k);
        Some(e)
    }

    /// `{{identifier}}` as `variable.parameter.python`.
    fn parameter(&mut self, p: usize) -> Option<usize> {
        let e = self.ident(p)?;
        self.tok(p, e, L::Parameter)
    }

    /// `import-alias`.
    fn import_alias(&mut self, p: usize) -> Option<usize> {
        let e = self.kw(p, b"as")?;
        self.tok(p, e, L::ImportAs)
    }

    /// `import-name-list`.
    fn import_names(&mut self, p: usize) -> Option<usize> {
        if self.is(p, b',') {
            return self.tok(p, p + 1, L::ImportSeparator);
        }
        self.import_alias(p).or_else(|| self.name(p)).or_else(|| {
            if self.is(p, b'*') {
                return self.tok(p, p + 1, L::ImportAll);
            }
            if !self.nonspace(p) {
                return None;
            }
            let e = (p..self.b.len())
                .find(|&q| space_len(self.b, q).is_some())
                .unwrap_or(self.b.len());
            self.tok(p, e, L::ImportIllegal)
        })
    }

    /// `import-from-name`'s path.
    fn import_path(&mut self, p: usize) -> Option<usize> {
        // `{{illegal_names}}\b`, `{{identifier}}`
        if let Some(e) = self.word_at_any(p)
            && words::lookup(&self.b[p..e]).1 == words::ILLEGAL
        {
            return self.tok(p, e, L::NameIllegal);
        }
        if let Some(e) = self.ident(p) {
            let w = self.w();
            self.put(p, e, w);
            return Some(e);
        }
        // `\s*(\.) *(?:({{illegal_names}}\b)|({{identifier}}))`
        let d = self.spaces(p);
        if self.is(d, b'.') && d < self.b.len() {
            let q = skip_blanks(self.b, d + 1);
            let named = self
                .word_at_any(q)
                .filter(|&e| words::lookup(&self.b[q..e]).1 == words::ILLEGAL)
                .map(|e| (e, L::NameIllegal))
                .or_else(|| self.ident(q).map(|e| (e, L::Plain)));
            if let Some((e, l)) = named {
                let w = self.w();
                self.put(p, d, w);
                let k = self.k(L::Dot);
                self.put(d, d + 1, k);
                self.put(d + 1, q, w);
                return self.tok(q, e, l);
            }
        }
        // `\ *(\. *\S+)`
        let d = skip_blanks(self.b, p);
        if self.is(d, b'.') && d < self.b.len() {
            let q = skip_blanks(self.b, d + 1);
            if self.nonspace(q) {
                let e = (q..self.b.len())
                    .find(|&i| space_len(self.b, i).is_some())
                    .unwrap_or(self.b.len());
                let w = self.w();
                self.put(p, d, w);
                let k = self.k(L::NameIllegal);
                return self.close(d, e, k);
            }
        }
        self.leave(p)
    }

    /// The run of word characters at `p`, a word's start or not, to its end.
    fn word_at_any(&self, p: usize) -> Option<usize> {
        (p < self.b.len() && is_word(self.b, p)).then(|| word_end(self.b, p))
    }

    /// `target-list`.
    fn target_list(&mut self, p: usize) -> Option<usize> {
        match self.ch(p)? {
            b',' => self.tok(p, p + 1, L::TargetSeparator),
            b'(' => {
                let k = self.k(L::TargetBegin);
                self.tok_push(p, p + 1, k, &[TARGETS])
            }
            _ => self.name(p),
        }
    }

    /// `inline-for`.
    fn inline_for(&mut self, p: usize) -> Option<usize> {
        let e = self.word(p)?;
        match self.info(p).kw {
            words::K_FOR => {
                let k = self.k(L::Generator);
                self.tok_push(p, e, k, &[INLINE_FOR])
            }
            words::K_ASYNC => {
                let q = self.spaces(e);
                if q == e {
                    return None;
                }
                let f = self.kw(q, b"for")?;
                let k = self.k(L::Async);
                self.put(p, e, k);
                let w = self.w();
                self.put(e, q, w);
                let k = self.k(L::Generator);
                self.tok_push(q, f, k, &[INLINE_FOR])
            }
            _ => None,
        }
    }

    /// `arguments`.
    fn arguments(&mut self, p: usize) -> Option<usize> {
        self.keyword_arguments(p)
            .or_else(|| {
                self.is(p, b',')
                    .then(|| {
                        let k = self.k(L::ArgsSeparator);
                        self.push_unpack(p, p + 1, k)
                    })
                    .flatten()
            })
            .or_else(|| self.inline_for(p))
            .or_else(|| self.in_group(p))
    }

    /// `keyword-arguments`: `(?={{identifier}}\s*=(?!=))`.
    fn keyword_arguments(&mut self, p: usize) -> Option<usize> {
        let e = self.ident(p)?;
        let q = self.spaces(e);
        if self.is(q, b'=') && !self.is(q + 1, b'=') {
            return self.enter(p, &[KWARG]);
        }
        None
    }

    /// `after-expression`.
    fn after(&mut self, p: usize) -> Option<usize> {
        let q = self.spaces(p);
        let w = self.w();
        match self.ch(q) {
            Some(b'(') if q < self.b.len() => {
                self.put(p, q, w);
                let k = self.k(L::ArgsBegin);
                return self.tok_push(q, q + 1, k, &[AFTER_ARGS]);
            }
            Some(b'[') if q < self.b.len() => {
                self.put(p, q, w);
                let k = self.k(L::BracketsBegin);
                return self.tok_push(q, q + 1, k, &[AFTER_ITEM]);
            }
            _ => {}
        }
        if self.call_follows(p) {
            return self.call(p);
        }
        if self.is(q, b'.') && q < self.b.len() {
            self.put(p, q, w);
            let k = self.k(L::Dot);
            return self.tok_push(q, q + 1, k, &[ATTR]);
        }
        self.leave(p)
    }

    /// `qualified-name-until-leaf`.
    fn qn_leaf(&mut self, p: usize) -> Option<usize> {
        let then_dot = |s: &Self, e: usize| {
            matches!(s.ch(s.spaces(e)), Some(b'.' | b'\\')) && s.spaces(e) < s.b.len()
        };
        if let Some(e) = self.ident(p)
            && then_dot(self, e)
        {
            return self.enter(p, &[LEAF_NAME]);
        }
        if self.is(p, b'.') && p < self.b.len() {
            let q = self.spaces(p + 1);
            if let Some(e) = self.ident(q) {
                if then_dot(self, e) {
                    let k = self.k(L::Dot);
                    self.put(p, p + 1, k);
                    let w = self.w();
                    return self.tok_push(p + 1, q, w, &[LEAF_DOT]);
                }
            } else {
                return self.tok(p, p + 1, L::Dot);
            }
        }
        if self.nonspace(p) || self.eol(p) {
            return self.leave(p);
        }
        None
    }

    /// `decorators`' context.
    fn decorator(&mut self, p: usize) -> Option<usize> {
        if self.is(p, b'@') && p < self.b.len() {
            return self.tok(p, p + 1, L::AnnotationMark);
        }
        if self.eol(p)
            && let Some(q) = self.leave(p)
        {
            return Some(q);
        }
        if let Some(q) = self.lcop(p) {
            return Some(q);
        }
        // `(?=\.?\s*{{path}}\s*\()`, `(?=\.?\s*{{path}})`
        let q = if self.is(p, b'.') && p < self.b.len() {
            self.spaces(p + 1)
        } else {
            self.spaces(p)
        };
        if self.path_then(q, Self::paren_follows) {
            return self.shift(p, &[DEC_CALL, QN_LEAF]);
        }
        if self.ident(q).is_some() {
            return self.enter(p, &[DEC_WRAP, QN_LEAF]);
        }
        if self.nonspace(p) {
            let e = self.next(p);
            let k = self.pop_base() | L::IllegalCharacter as u16;
            return self.close(p, e, k);
        }
        None
    }

    /// `function-parameters`.
    fn params(&mut self, p: usize) -> Option<usize> {
        match self.ch(p)? {
            b')' => {
                let k = self.k(L::ParametersEnd);
                return self.then(p, p + 1, k, &[FUNC_AFTER]);
            }
            b'#' => return self.comments(p),
            b',' => {
                let k = self.k(L::ParametersSeparator);
                return self.push_unpack(p, p + 1, k);
            }
            b'/' => {
                let k = self.k(L::PositionalOnly);
                return self.tok_push(p, p + 1, k, &[POSONLY]);
            }
            b'=' => return self.shift(p, &[DEFAULT_EQ]),
            b':' => return self.shift(p, &[ANN_COLON]),
            b'(' => {
                let k = self.k(L::GroupBegin);
                return self.tok_push(p, p + 1, k, &[PTUPLE]);
            }
            _ => {}
        }
        self.illegal_names(p)
            .or_else(|| self.parameter(p))
            .or_else(|| self.line_continuation(p))
    }

    /// `function-parameters-tuple`'s context.
    fn params_tuple(&mut self, p: usize) -> Option<usize> {
        match self.ch(p)? {
            b')' => {
                let k = self.k(L::GroupEnd);
                return self.then_after(p, p + 1, k);
            }
            b'#' => return self.comments(p),
            b',' => {
                let k = self.k(L::ParametersSeparator);
                return self.push_unpack(p, p + 1, k);
            }
            b'=' => {
                // the token takes the pushed context's `invalid.illegal.default-value`
                let k = self.k(L::IllegalDefault);
                return self.tok_push(p, p + 1, k, &[PT_DEFAULT]);
            }
            b':' => return self.enter(p, &[PT_ANN]),
            _ => {}
        }
        self.illegal_names(p)
            .or_else(|| self.parameter(p))
            .or_else(|| self.line_continuation(p))
    }

    /// `lambda-parameters`.
    fn lambda_params(&mut self, p: usize) -> Option<usize> {
        if let Some(q) = self.lcop(p) {
            return Some(q);
        }
        match self.ch(p)? {
            b':' => {
                let k = self.k(L::FunctionBegin);
                return self.then(p, p + 1, k, &[LAMBDA_COLON]);
            }
            b',' => {
                let k = self.k(L::ParametersSeparator);
                return self.tok_push(p, p + 1, k, &[UNPACK]);
            }
            _ => {}
        }
        self.keyword_arguments(p)
            .or_else(|| {
                self.is(p, b'(')
                    .then(|| {
                        let k = self.k(L::GroupBegin);
                        self.tok_push(p, p + 1, k, &[PTUPLE])
                    })
                    .flatten()
            })
            .or_else(|| self.illegal_names(p))
            .or_else(|| self.parameter(p))
            .or_else(|| {
                if !self.nonspace(p) {
                    return None;
                }
                let e = self.next(p);
                self.tok(p, e, L::ExpectedParameter)
            })
    }

    /// `class X(`'s context.
    fn class_bases(&mut self, p: usize) -> Option<usize> {
        match self.ch(p)? {
            b')' => {
                let k = self.k(L::InheritanceEnd);
                return self.then(p, p + 1, k, &[CLASS_AFTER]);
            }
            b':' => {
                let k = self.k(L::NoClosingParens);
                return self.close(p, p + 1, k);
            }
            b',' => return self.tok(p, p + 1, L::InheritanceSeparator),
            _ => {}
        }
        if let Some(q) = self.illegal_names_pop(p) {
            return Some(q);
        }
        if let Some(e) = self.ident(p) {
            // `({{identifier}}) *(=)`
            let q = skip_blanks(self.b, e);
            if self.is(q, b'=') && q < self.b.len() {
                let k = self.k(L::InheritanceParameter);
                self.put(p, e, k);
                let w = self.w();
                self.put(e, q, w);
                return self.tok(q, q + 1, L::Assignment);
            }
            return self.enter(p, &[INHERITED]);
        }
        self.in_group(p)
    }

    /// `( … )`, a tuple's and a list's.
    fn sequence(&mut self, p: usize, c: u8) -> Option<usize> {
        let close = if c == LIST { b']' } else { b')' };
        if self.is(p, close) && p < self.b.len() {
            let k = self.k(if c == GROUP {
                L::GroupEnd
            } else {
                L::SequenceEnd
            });
            return self.then_after(p, p + 1, k);
        }
        if self.is(p, b',') {
            if c == GROUP {
                return self.tok(p, p + 1, L::TupleSeparator);
            }
            let k = self.k(L::SequenceSeparator);
            return self.push_unpack(p, p + 1, k);
        }
        self.inline_for(p).or_else(|| self.in_group(p))
    }

    /// `{` that may be a mapping or a set.
    fn mapset(&mut self, p: usize) -> Option<usize> {
        match self.ch(p)? {
            b'}' => {
                let k = self.k(L::MappingOrSetEnd);
                return self.then_after(p, p + 1, k);
            }
            b',' => {
                let k = self.k(L::SetSeparator);
                return self.then(p, p + 1, k, &[SET]);
            }
            _ => {}
        }
        if let Some(q) = self.illegal_walrus(p) {
            return Some(q);
        }
        if self.is(p, b':') {
            let k = self.k(L::KeyValue);
            return self.then(p, p + 1, k, &[DICT_VALUE]);
        }
        self.inline_for(p).or_else(|| self.in_group(p))
    }

    /// `inside-dictionary`.
    fn dict(&mut self, p: usize) -> Option<usize> {
        if self.is(p, b'}') {
            let k = self.k(L::MappingEnd);
            return self.then_after(p, p + 1, k);
        }
        if let Some(q) = self.illegal_walrus(p) {
            return Some(q);
        }
        match self.ch(p)? {
            b':' => {
                let k = self.k(L::KeyValue);
                return self.then(p, p + 1, k, &[DICT_VALUE]);
            }
            b',' => return self.tok(p, p + 1, L::ExpectedColon),
            b'*' if self.is(p + 1, b'*') => {
                let k = self.k(L::UnpackMapping);
                return self.tok_push(p, p + 2, k, &[DICT_UNPACK]);
            }
            b'#' => return self.comments(p),
            _ => {}
        }
        if self.nonspace(p) {
            return self.enter(p, &[DICT_KEY]);
        }
        None
    }

    /// `inside-directory-value`.
    fn dict_value(&mut self, p: usize) -> Option<usize> {
        match self.ch(p)? {
            b'}' => {
                let k = self.k(L::MappingEnd);
                return self.then_after(p, p + 1, k);
            }
            b',' => return self.shift(p, &[DICT_COMMA]),
            b'#' => return self.comments(p),
            _ => {}
        }
        if self.has(p, b"for") && !is_word(self.b, p + 3) {
            return self.enter(p, &[DICT_FOR]);
        }
        if self.nonspace(p) {
            return self.enter(p, &[DICT_VAL]);
        }
        None
    }

    /// `inside-set`.
    fn set_body(&mut self, p: usize) -> Option<usize> {
        if self.is(p, b'}') {
            let k = self.k(L::SetEnd);
            return self.then_after(p, p + 1, k);
        }
        if let Some(q) = self.illegal_walrus(p) {
            return Some(q);
        }
        match self.ch(p)? {
            b':' => return self.tok(p, p + 1, L::ColonInSet),
            b',' => return self.tok(p, p + 1, L::SetSeparator),
            b'*' => {
                let k = self.k(L::UnpackSequence);
                return self.tok_push(p, p + 1, k, &[SET_UNPACK]);
            }
            _ => {}
        }
        self.inline_for(p).or_else(|| self.in_group(p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(text: &str) -> Vec<(String, Vec<&'static str>)> {
        let mut state = State::default();
        let mut out = Vec::new();
        for l in text.lines() {
            let mut spans = Vec::new();
            line(l.as_bytes(), &mut state, &mut Out::new(&mut spans));
            for s in spans {
                out.push((
                    l[s.start as usize..s.end as usize].to_string(),
                    scopes(s.kind),
                ));
            }
        }
        out
    }

    #[test]
    fn a_function_and_its_parameters() {
        let t = lex("def f(a, *b, c: int = 1) -> None:\n    return a\n");
        assert!(t.contains(&("f".into(), vec!["entity.name.function.python"])));
        assert!(t.contains(&("a".into(), vec!["variable.parameter.python"])));
        assert!(t.contains(&("int".into(), vec!["support.type.python"])));
        assert!(t.contains(&("return".into(), vec!["keyword.control.flow.return.python"])));
    }

    /// The SQL in a string runs on SQL's scanner, embedded: the string's own patterns come
    /// first at each place they can start, and SQL's take the rest.
    #[test]
    fn sql_in_strings_takes_turns_with_the_string() {
        let has = |t: &[(String, Vec<&str>)], text: &str, scopes: &[&str]| {
            assert!(
                t.iter()
                    .any(|(x, s)| x == text && s.iter().copied().eq(scopes.iter().copied())),
                "{text:?} {scopes:?} in {t:?}"
            );
        };
        let dq = "string.quoted.double.python";
        // an escape ending in a word's byte leaves no `\b` before `FROM`
        let t = lex(r#"q = "SELECT a\nFROM b WHERE c = %s""#);
        has(&t, "SELECT", &[dq, "keyword.other.DML.sql"]);
        has(&t, r"\n", &[dq, "constant.character.escape.python"]);
        has(&t, "WHERE", &[dq, "keyword.other.DML.sql"]);
        has(&t, "%s", &[dq, "constant.other.placeholder.python"]);
        assert!(!t.iter().any(|(x, _)| x == "FROM"), "{t:?}");
        // the string's end inside a SQL string and a SQL comment
        let t = lex(r#"q = "SELECT 'a" + "INSERT INTO t -- c" + x"#);
        has(
            &t,
            "'",
            &[
                dq,
                "string.quoted.single.sql",
                "punctuation.definition.string.begin.sql",
            ],
        );
        has(&t, " c", &[dq, "comment.line.double-dash.sql"]);
        has(&t, "+", &["keyword.operator.arithmetic.python"]);
        // an escape in a SQL string takes the SQL string's scope too
        let t = lex(r#"q = "SELECT 'a\tb'""#);
        has(
            &t,
            r"\t",
            &[
                dq,
                "string.quoted.single.sql",
                "constant.character.escape.python",
            ],
        );
        // `create … view\s*` takes the line's `\n`; the next line's name is plain
        let t = lex("q = \"\"\"CREATE VIEW \nv AS SELECT 1\"\"\"");
        assert!(!t.iter().any(|(x, _)| x == "v"), "{t:?}");
        has(
            &t,
            "AS",
            &[
                "string.quoted.double.block.python",
                "keyword.operator.assignment.alias.sql",
            ],
        );
    }

    #[test]
    fn constant_names() {
        assert!(is_constant_name(b"ABC"));
        assert!(is_constant_name(b"_AB1"));
        assert!(!is_constant_name(b"A_B"));
        assert!(!is_constant_name(b"Abc"));
    }
}
