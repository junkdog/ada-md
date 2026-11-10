//! Swift on a scanner of ada's own (X-101), from a floor probe (`x101`'s `sol`): one loop over a
//! line's bytes, a byte's action one load from a table, a name read once and looked up once, an
//! operator's run read once and classed by its bytes and the bytes around it (as bat's grammar
//! classes it: by spacing, not by grammar), and frames only for what nests: parentheses, braces,
//! a string's interpolations ([`Scan::frames`], four bits a level). A multi-line string and a
//! block comment continued past a line's end are a mode ([`Scan::mode`]). Kinds are bat's leaves.
//! A regex literal's body is on [`crate::regex`] (Swift's dialect), a multi-line one's in its
//! extended syntax, where `#` starts a comment.

use crate::emit::{Rules, Run, Runs, Sink, Spans};
use crate::prim::{WORD, char_len, is_word, is_word_starting, wide_space, word_end};
use crate::regex;
use crate::words::{Words, text_len};
use crate::{Out, State};

/// The leaves: a token's scopes.
static LEAVES: [&str; 77] = [
    "",
    // comments: a line's text and its mark; a block's text, begin, end; a documentation
    // block's; a shebang's text and its mark
    "comment.line.double-slash.swift",
    "comment.line.double-slash.swift punctuation.definition.comment.line.double-slash.swift",
    "comment.block.swift",
    "comment.block.swift punctuation.definition.comment.block.begin.swift",
    "comment.block.swift punctuation.definition.comment.block.end.swift",
    "comment.block.documentation.swift",
    "comment.block.documentation.swift punctuation.definition.comment.block.documentation.begin.swift",
    "comment.block.documentation.swift punctuation.definition.comment.block.documentation.end.swift",
    "comment.line.shebang.swift",
    "comment.line.shebang.swift punctuation.definition.comment.line.shebang.swift",
    // strings
    "string.quoted.double.swift",
    "constant.character.escape.swift",
    "invalid.illegal.swift",
    "support.punctuation.expression.begin.swift",
    "support.punctuation.expression.end.swift",
    // words
    "keyword.control.if.swift",
    "keyword.control.switch.swift",
    "keyword.control.transfer.swift",
    "keyword.control.loop.swift",
    "kewyord.control.catch.swift",
    "keyword.other.operator.swift",
    "keyword.other.declaration-modifier.swift",
    "keyword.other.access-level-modifier.swift",
    "keyword.declaration.swift",
    "keyword.statement.swift",
    "keyword.other.statement.swift",
    "keyword.other.swift",
    "storage.type.swift",
    "storage.type.function.swift",
    "entity.type.function.swift",
    "keyword.other.import.swift",
    "storage.modifier.swift",
    "support.type.module.import.swift",
    "support.type.swift",
    "support.type.array.swift",
    "support.type.dictionary.swift",
    "punctuation.array.begin.swift",
    "punctuation.array.end.swift",
    "punctuation.dictionary.begin.swift",
    "punctuation.dictionary.end.swift",
    "keyword.constant.boolean.swift",
    "constant.nil.swift",
    "keyword.other.literal.swift",
    // numbers
    "constant.numeric.integer.binary.swift",
    "constant.numeric.integer.octal.swift",
    "constant.numeric.integer.decimal.swift",
    "constant.numeric.integer.hexadecimal.swift",
    // operators
    "keyword.operator.comparative.swift",
    "keyword.operator.assignment.swift",
    "keyword.operator.logical.swift",
    "keyword.operator.remainder.swift",
    "keyword.operator.increment-or-decrement.swift",
    "keyword.operator.overflow.swift",
    "keyword.operator.range.swift",
    "keyword.operator.bitwise.swift",
    "keyword.operator.arithmetic.swift",
    "keyword.operator.ternary.swift",
    "keyword.operator.type-casting.swift",
    "keyword.operator.custom.prefix.unary.swift",
    "keyword.operator.custom.postfix.unary.swift",
    "keyword.operator.custom.binary.swift",
    "keyword.operator.function-result.swift",
    // attributes
    "storage.modifier.attribute.swift",
    "storage.modifier.attribute.swift punctuation.definition.attribute.swift",
    "storage.modifier.attribute.swift punctuation.definition.attribute-arguments.begin.swift",
    "punctuation.definition.attribute-arguments.end.swift",
    // a function's declaration
    "punctuation.definition.generic-parameter-clause.begin.swift",
    "punctuation.definition.generic-parameter-clause.end.swift",
    "punctuation.definition.function-arguments.begin.swift",
    "punctuation.definition.function-arguments.end.swift",
    "punctuation.definition.code-block.begin.swift",
    "punctuation.definition.code-block.end.swift",
    // a regex literal (Swift 5.7), which bat's grammar reads as code: ada's own leaf
    "string.regexp.swift",
    "string.regexp.swift punctuation.definition.string.begin.swift",
    "string.regexp.swift punctuation.definition.string.end.swift",
    // a multi-line regex literal's comment (its extended syntax)
    "string.regexp.swift comment.line.number-sign.regexp",
];

const LINE_COMMENT: u16 = 1;
const LINE_MARK: u16 = 2;
const BLOCK_COMMENT: u16 = 3;
const BLOCK_BEGIN: u16 = 4;
const BLOCK_END: u16 = 5;
const DOC_COMMENT: u16 = 6;
const DOC_BEGIN: u16 = 7;
const DOC_END: u16 = 8;
const SHEBANG: u16 = 9;
const SHEBANG_MARK: u16 = 10;
const STRING: u16 = 11;
const ESCAPE: u16 = 12;
const INVALID: u16 = 13;
const INTERP_BEGIN: u16 = 14;
const INTERP_END: u16 = 15;
const K_IF: u16 = 16;
const K_SWITCH: u16 = 17;
const K_TRANSFER: u16 = 18;
const K_LOOP: u16 = 19;
const K_CATCH: u16 = 20;
const K_OPERATOR: u16 = 21;
const K_MODIFIER: u16 = 22;
const K_ACCESS: u16 = 23;
const K_DECLARATION: u16 = 24;
const K_STATEMENT: u16 = 25;
const K_OTHER_STATEMENT: u16 = 26;
const K_OTHER: u16 = 27;
const STORAGE: u16 = 28;
const STORAGE_FUNCTION: u16 = 29;
const FUNCTION_NAME: u16 = 30;
const IMPORT: u16 = 31;
const IMPORT_KIND: u16 = 32;
const MODULE: u16 = 33;
const SUPPORT_TYPE: u16 = 34;
const ARRAY: u16 = 35;
const DICTIONARY: u16 = 36;
const ARRAY_BEGIN: u16 = 37;
const ARRAY_END: u16 = 38;
const DICTIONARY_BEGIN: u16 = 39;
const DICTIONARY_END: u16 = 40;
const BOOLEAN: u16 = 41;
const NIL: u16 = 42;
const LITERAL: u16 = 43;
const BINARY: u16 = 44;
const OCTAL: u16 = 45;
const DECIMAL: u16 = 46;
const HEX: u16 = 47;
const COMPARATIVE: u16 = 48;
const ASSIGNMENT: u16 = 49;
const LOGICAL: u16 = 50;
const REMAINDER: u16 = 51;
const INCREMENT: u16 = 52;
const OVERFLOW: u16 = 53;
const RANGE: u16 = 54;
const BITWISE: u16 = 55;
const ARITHMETIC: u16 = 56;
const TERNARY: u16 = 57;
const CASTING: u16 = 58;
const PREFIX: u16 = 59;
const POSTFIX: u16 = 60;
const BINARY_OP: u16 = 61;
const RESULT_ARROW: u16 = 62;
const ATTRIBUTE: u16 = 63;
const ATTRIBUTE_MARK: u16 = 64;
const ATTRIBUTE_ARGS: u16 = 65;
const ATTRIBUTE_END: u16 = 66;
const GENERIC_BEGIN: u16 = 67;
const GENERIC_END: u16 = 68;
const PARAMS_BEGIN: u16 = 69;
const PARAMS_END: u16 = 70;
const BODY_BEGIN: u16 = 71;
const BODY_END: u16 = 72;
const REGEX: u16 = 73;
const RE_BEGIN: u16 = 74;
const RE_END: u16 = 75;
const RE_COMMENT: u16 = 76;

/// The kinds of a regex literal's tokens, from here: [`regex::KINDS`] of them.
const RE_KINDS: u16 = 128;
const _: () = assert!(LEAVES.len() <= RE_KINDS as usize);

/// A regex literal's body, the line cut at its closing `/`: the regex's text, no host's
/// patterns in it.
static RE_EMBED: regex::Embed<'static> = re_embed(&NO_STOPS);
/// A multi-line regex literal's: `#` outside a class starts a comment.
static RE_X_EMBED: regex::Embed<'static> = re_embed(&HASH_STOPS);
static NO_STOPS: [bool; 256] = regex::table(b"");
static HASH_STOPS: [bool; 256] = regex::table(b"#");

const fn re_embed(yields: &'static [bool; 256]) -> regex::Embed<'static> {
    regex::Embed {
        dialect: regex::Dialect::Swift,
        cooked: false,
        yields,
        stop: |_, _, _| false,
        arg: 0,
        escape: regex::java_escape,
        top_only: false,
        continues: false,
        base: RE_KINDS,
        text: REGEX,
    }
}

/// The scopes under `source.swift` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    if kind >= RE_KINDS {
        let mut out = vec!["string.regexp.swift"];
        out.extend(regex::scopes(kind - RE_KINDS));
        return out;
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
const A_QUOTE: u8 = 4;
const A_SLASH: u8 = 5;
/// `=-+!*%<>&|^~.`: an operator's run.
const A_OP: u8 = 6;
const A_LPAREN: u8 = 7;
const A_RPAREN: u8 = 8;
const A_LBRACE: u8 = 9;
const A_RBRACE: u8 = 10;
const A_AT: u8 = 11;
const A_HASH: u8 = 12;
/// `?` and `:`: the ternary's, where blanks or brackets are on both sides.
const A_TERNARY: u8 = 13;
/// A byte past ASCII: a word's character, or another.
const A_WIDE: u8 = 14;
/// `` ` ``: a quoted name, `` `default` `` or Swift 6.2's raw `` `a name` ``.
const A_BACKTICK: u8 = 15;

static ACT: [u8; 256] = {
    let mut t = [A_WIDE; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = match b {
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => A_SPACE,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => A_NAME,
            b'0'..=b'9' => A_DIGIT,
            b'"' => A_QUOTE,
            b'/' => A_SLASH,
            b'=' | b'-' | b'+' | b'!' | b'*' | b'%' | b'<' | b'>' | b'&' | b'|' | b'^' | b'~'
            | b'.' => A_OP,
            b'(' => A_LPAREN,
            b')' => A_RPAREN,
            b'{' => A_LBRACE,
            b'}' => A_RBRACE,
            b'@' => A_AT,
            b'#' => A_HASH,
            b'?' | b':' => A_TERNARY,
            b'`' => A_BACKTICK,
            _ => A_OTHER,
        };
        c += 1;
    }
    t
};

/// bat's operator characters, `[/=\-+!*%<>&|\^~.]`.
static OPCH: [bool; 256] = {
    let mut t = [false; 256];
    let s = b"/=-+!*%<>&|^~.";
    let mut i = 0;
    while i < s.len() {
        t[s[i] as usize] = true;
        i += 1;
    }
    t
};

/// What may stand before a custom operator that is prefix or binary, `[\s(\[{,;:]`.
static OPEN_SIDE: [bool; 256] = {
    let mut t = [false; 256];
    let s = b" \t\r\n\x0b\x0c([{,;:";
    let mut i = 0;
    while i < s.len() {
        t[s[i] as usize] = true;
        i += 1;
    }
    t
};

/// What may stand after a custom operator that is binary (and not after a prefix one),
/// `[\s)\]},;:]`.
static CLOSE_SIDE: [bool; 256] = {
    let mut t = [false; 256];
    let s = b" \t\r\n\x0b\x0c)]},;:";
    let mut i = 0;
    while i < s.len() {
        t[s[i] as usize] = true;
        i += 1;
    }
    t
};

// What a word is ([`WORDS`], [`INFO`]): its leaf in the low byte, an action in the high one.
/// `func`: a declaration's head where a name and `(` or `<` follow.
const W_FUNC: u16 = 1 << 8;
/// `import`: a module's path follows.
const W_IMPORT: u16 = 2 << 8;
/// An access level: `(set)` may follow.
const W_ACCESS: u16 = 3 << 8;
/// `as`: `as?` where `?` ends a word.
const W_AS: u16 = 4 << 8;
/// `Array`: its `<` opens an array's type.
const W_ARRAY: u16 = 5 << 8;
/// `Dictionary`: its `<` opens a dictionary's type.
const W_DICTIONARY: u16 = 6 << 8;
/// `Optional`: `Optional<` is plain, its `<` no operator.
const W_OPTIONAL: u16 = 7 << 8;

/// The words bat's grammar names, each with what its first pattern in `main`'s order makes
/// of it.
const WORD_LIST: &[(&[u8], u16)] = &[
    // literal
    (b"nil", NIL),
    (b"__FILE__", LITERAL),
    (b"__LINE__", LITERAL),
    (b"__COLUMN__", LITERAL),
    (b"__FUNCTION__", LITERAL),
    // operator: type-casting
    (b"is", CASTING),
    (b"as", W_AS | CASTING),
    // declaration
    (b"import", W_IMPORT | K_DECLARATION),
    (b"func", W_FUNC | STORAGE),
    // storage-type
    (b"var", STORAGE),
    (b"let", STORAGE),
    (b"class", STORAGE),
    (b"enum", STORAGE),
    (b"struct", STORAGE),
    (b"protocol", STORAGE),
    (b"extension", STORAGE),
    (b"typealias", STORAGE),
    // keyword: branch-statement-keyword
    (b"if", K_IF),
    (b"else", K_IF),
    (b"switch", K_SWITCH),
    (b"case", K_SWITCH),
    (b"default", K_SWITCH),
    (b"where", K_SWITCH),
    // control-transfer-statement-keyword
    (b"continue", K_TRANSFER),
    (b"break", K_TRANSFER),
    (b"fallthrough", K_TRANSFER),
    (b"return", K_TRANSFER),
    // loop-statement-keyword
    (b"while", K_LOOP),
    (b"repeat", K_LOOP),
    (b"for", K_LOOP),
    (b"in", K_LOOP),
    // catch-statement-keyword (bat's scope, spelled so)
    (b"catch", K_CATCH),
    (b"do", K_CATCH),
    // operator-declaration-modifier
    (b"operator", K_OPERATOR),
    (b"prefix", K_OPERATOR),
    (b"infix", K_OPERATOR),
    (b"postfix", K_OPERATOR),
    // declaration-modifier
    (b"convenience", K_MODIFIER),
    (b"dynamic", K_MODIFIER),
    (b"final", K_MODIFIER),
    (b"lazy", K_MODIFIER),
    (b"mutating", K_MODIFIER),
    (b"nonmutating", K_MODIFIER),
    (b"optional", K_MODIFIER),
    (b"override", K_MODIFIER),
    (b"required", K_MODIFIER),
    (b"static", K_MODIFIER),
    (b"unowned", K_MODIFIER),
    (b"unownedsafe", K_MODIFIER),
    (b"unownedunsafe", K_MODIFIER),
    (b"weak", K_MODIFIER),
    // access-level-modifier
    (b"open", W_ACCESS | K_ACCESS),
    (b"public", W_ACCESS | K_ACCESS),
    (b"internal", W_ACCESS | K_ACCESS),
    (b"fileprivate", W_ACCESS | K_ACCESS),
    (b"private", W_ACCESS | K_ACCESS),
    // keyword.declaration
    (b"deinit", K_DECLARATION),
    (b"init", K_DECLARATION),
    (b"subscript", K_DECLARATION),
    (b"throws", K_DECLARATION),
    (b"rethrows", K_DECLARATION),
    // keyword.statement
    (b"guard", K_STATEMENT),
    (b"defer", K_STATEMENT),
    (b"try", K_STATEMENT),
    (b"throw", K_STATEMENT),
    // keyword.other.statement
    (b"dynamicType", K_OTHER_STATEMENT),
    (b"new", K_OTHER_STATEMENT),
    (b"super", K_OTHER_STATEMENT),
    (b"self", K_OTHER_STATEMENT),
    (b"Self", K_OTHER_STATEMENT),
    (b"Type", K_OTHER_STATEMENT),
    // keyword.other
    (b"associativity", K_OTHER),
    (b"didSet", K_OTHER),
    (b"get", K_OTHER),
    (b"inout", K_OTHER),
    (b"left", K_OTHER),
    (b"none", K_OTHER),
    (b"precedence", K_OTHER),
    (b"right", K_OTHER),
    (b"set", K_OTHER),
    (b"willSet", K_OTHER),
    // type: primitive-type, integer-type, collection-type
    (b"Int", SUPPORT_TYPE),
    (b"Float", SUPPORT_TYPE),
    (b"Double", SUPPORT_TYPE),
    (b"String", SUPPORT_TYPE),
    (b"Bool", SUPPORT_TYPE),
    (b"Character", SUPPORT_TYPE),
    (b"Void", SUPPORT_TYPE),
    (b"UInt", SUPPORT_TYPE),
    (b"Int8", SUPPORT_TYPE),
    (b"Int16", SUPPORT_TYPE),
    (b"Int32", SUPPORT_TYPE),
    (b"Int64", SUPPORT_TYPE),
    (b"UInt8", SUPPORT_TYPE),
    (b"UInt16", SUPPORT_TYPE),
    (b"UInt32", SUPPORT_TYPE),
    (b"UInt64", SUPPORT_TYPE),
    (b"Array", W_ARRAY | SUPPORT_TYPE),
    (b"Dictionary", W_DICTIONARY | SUPPORT_TYPE),
    (b"Optional", W_OPTIONAL),
    // boolean
    (b"true", BOOLEAN),
    (b"false", BOOLEAN),
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

// The frames, four bits a level in [`Scan::frames`]; 0 is none (the top level, or a level lost
// past 32 levels).
/// `(` … `)`.
const F_PAREN: u8 = 1;
/// `{` … `}`.
const F_BRACE: u8 = 2;
/// A string's `\(` … `)`: the string a `"` one.
const F_INTERP: u8 = 3;
/// A multi-line string's `\(` … `)`.
const F_INTERP_MULTI: u8 = 4;
/// A raw string's `\#(` … `)`, its `#`s in [`Scan::raws`].
const F_INTERP_RAW: u8 = 5;
/// A raw multi-line string's.
const F_INTERP_RAW_MULTI: u8 = 6;
/// An attribute's arguments, `@name(` … `)`.
const F_ATTR: u8 = 7;
/// A function's parameters, `func name(` … `)`.
const F_PARAMS: u8 = 8;
/// A function's generic clause, `func name<` … `>`.
const F_GENERIC: u8 = 9;
/// A `<` … `>` inside an angle's frame ([`F_GENERIC`], an array's or a dictionary's type): a
/// generic type's arguments, Swift's reading (bat's frame ends at the first `>`).
const F_ANGLE: u8 = 10;
/// A function's body, `{` … `}`: its own `}` ends it (bat's, the first).
const F_BODY: u8 = 11;
/// A function's head, from its name to its body ([`Scan::head`]).
const F_HEAD: u8 = 12;
/// A function's result, from its `->` to its body ([`Scan::result`]).
const F_RESULT: u8 = 13;
/// `Array<` … `>`.
const F_ARRAY: u8 = 14;
/// `Dictionary<` … `>`.
const F_DICTIONARY: u8 = 15;
/// The regex literals a line may try that run to its end unclosed ([`Scan::misses`]).
const REGEX_TRIES: u8 = 8;

// What a line starts in ([`Scan::mode`]).
const M_CODE: u8 = 0;
/// A block comment, [`Scan::nest`] deep.
const M_COMMENT: u8 = 1;
/// A documentation comment, [`Scan::nest`] deep.
const M_DOC: u8 = 2;
/// A multi-line string, raw by [`Scan::hashes`].
const M_MULTI: u8 = 3;
/// A multi-line regex literal, `#/` at a line's end, closed by `/` and [`Scan::hashes`] `#`s.
const M_REGEX: u8 = 4;

/// Lex `b`, a line of Swift without its `\n`, from `state`.
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

/// A block of Swift on one scan opened from `state` ([`crate::Carry`]).
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
    /// What each open frame is ([`F_PAREN`]…), four bits a level, the innermost lowest; past
    /// 32 levels the outer ones are lost.
    frames: u128,
    /// How many frames are open, up to 255.
    depth: u8,
    /// The `#`s of the raw strings whose interpolations are open, a byte each, the innermost
    /// lowest.
    raws: u32,
    /// What the line starts in ([`M_CODE`]…).
    mode: u8,
    /// An open block comment's depth.
    nest: u8,
    /// An open multi-line string's `#`s.
    hashes: u8,
    /// An open multi-line regex literal's state on [`crate::regex`].
    re: u8,
    /// The regex literals tried on the line that ran to its end unclosed: past
    /// [`REGEX_TRIES`], `/` is an operator for the rest of the line, so that a line of
    /// unclosed ones stays linear.
    misses: u8,
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
            raws: 0,
            mode: M_CODE,
            nest: 0,
            hashes: 0,
            re: 0,
            misses: 0,
            k,
            out,
        }
    }

    /// The scan's fields from `state`: the frames in its levels and the first two withs, the
    /// raw strings' `#`s in the next four, the comment's depth and the string's `#`s in the next
    /// two, the mode in [`State::arg`]'s low three bits and a multi-line regex's state above.
    #[inline(always)]
    fn load(&mut self, state: &State) {
        let mut w = [0u8; 16];
        w[..14].copy_from_slice(&state.levels);
        w[14] = state.withs[0];
        w[15] = state.withs[1];
        self.frames = u128::from_le_bytes(w);
        self.depth = state.depth;
        let mut r = [0u8; 4];
        r.copy_from_slice(&state.withs[2..6]);
        self.raws = u32::from_le_bytes(r);
        self.nest = state.withs[6];
        self.hashes = state.withs[7];
        self.mode = (state.arg & 7) as u8;
        self.re = (state.arg >> 3) as u8 & regex::STATE;
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
        s.withs[2..6].copy_from_slice(&self.raws.to_le_bytes());
        s.withs[6] = self.nest;
        s.withs[7] = self.hashes;
        s.depth = self.depth;
        s.arg = u16::from(self.mode) | u16::from(self.re) << 3;
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

    /// A write: one call on the run sink ([`Sink::merge_call`]), whose writer inlined at each
    /// of the scan's writes made the run loop 37 KB; the common tokens' write inlined
    /// ([`Scan::put_hot`]).
    #[inline(always)]
    fn put(&mut self, s: usize, e: usize, kind: u16) {
        self.k.merge_call(&mut self.out, s, e, kind);
    }

    /// A write inlined, for the common tokens: a word's, an operator's, a number's, a line
    /// comment's, a string's.
    #[inline(always)]
    fn put_hot(&mut self, s: usize, e: usize, kind: u16) {
        self.k.merge(&mut self.out, s, e, kind);
    }

    #[inline(always)]
    fn top(&self) -> u8 {
        (self.frames & 15) as u8
    }

    #[inline(always)]
    fn push(&mut self, f: u8) {
        self.frames = self.frames << 4 | u128::from(f);
        self.depth = self.depth.saturating_add(1);
    }

    #[inline(always)]
    fn pop(&mut self) {
        if self.depth > 0 {
            self.frames >>= 4;
            self.depth -= 1;
        }
    }

    /// The line, from what the line before left: a mode's rest first; the loop out of line where
    /// `SHARED` ([`Scan::code_shared`]).
    #[inline(always)]
    fn lex_line<const SHARED: bool>(&mut self) {
        self.misses = 0;
        let p = match self.mode {
            M_CODE => 0,
            M_MULTI => self.string(0, self.hashes, true),
            M_REGEX => self.regex_lines(0),
            _ => self.comment(0),
        };
        let p = match self.top() {
            F_HEAD => self.head(p, true),
            F_RESULT => self.result(p, true),
            _ => p,
        };
        if SHARED {
            self.code_shared(p);
        } else {
            self.code(p);
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
            p = match ACT[usize::from(c)] {
                A_SPACE => {
                    let mut e = p + 1;
                    while e < n && ACT[usize::from(b[e])] == A_SPACE {
                        e += 1;
                    }
                    e
                }
                A_NAME => self.name(p),
                A_DIGIT => self.number(p),
                A_QUOTE => self.quote(p, p, 0),
                A_SLASH => match b.get(p + 1) {
                    Some(b'/') => {
                        self.put_hot(p, p + 2, LINE_MARK);
                        self.put_hot(p + 2, n, LINE_COMMENT);
                        n
                    }
                    Some(b'*') => self.comment_open(p),
                    _ => self.slash(p),
                },
                A_OP => self.op(p),
                A_LPAREN => {
                    self.push(F_PAREN);
                    p + 1
                }
                A_RPAREN => self.rparen(p),
                A_LBRACE => {
                    self.push(F_BRACE);
                    p + 1
                }
                A_RBRACE => {
                    match self.top() {
                        F_BRACE => self.pop(),
                        F_BODY => {
                            self.pop();
                            self.put(p, p + 1, BODY_END);
                        }
                        _ => {}
                    }
                    p + 1
                }
                A_AT => self.attribute(p),
                A_HASH => self.hash(p),
                A_TERNARY => {
                    if open_before(b, p) && close_at(b, p + 1) {
                        self.put(p, p + 1, TERNARY);
                    }
                    p + 1
                }
                // a name, plain: Swift's reading (bat's colors a keyword in it)
                A_BACKTICK => quoted_end(b, p).unwrap_or(p + 1),
                A_WIDE => {
                    if is_word_starting(b, p) {
                        word_end(b, p)
                    } else {
                        p + char_len(c)
                    }
                }
                _ => p + 1,
            };
        }
    }

    /// A `)` at `p`: a parenthesis's, or an interpolation's, whose string goes on.
    #[inline(always)]
    fn rparen(&mut self, p: usize) -> usize {
        match self.top() {
            F_PAREN => {
                self.pop();
                p + 1
            }
            F_ATTR => {
                self.pop();
                self.put(p, p + 1, ATTRIBUTE_END);
                p + 1
            }
            F_PARAMS => {
                self.pop();
                self.put(p, p + 1, PARAMS_END);
                if self.top() == F_HEAD {
                    self.head(p + 1, false)
                } else {
                    p + 1
                }
            }
            F_INTERP | F_INTERP_MULTI | F_INTERP_RAW | F_INTERP_RAW_MULTI => {
                let f = self.top();
                self.pop();
                self.put(p, p + 1, INTERP_END);
                let raw = if f >= F_INTERP_RAW {
                    let h = self.raws as u8;
                    self.raws >>= 8;
                    h
                } else {
                    0
                };
                self.string(p + 1, raw, f == F_INTERP_MULTI || f == F_INTERP_RAW_MULTI)
            }
            _ => p + 1,
        }
    }

    /// A word at `p`: a keyword, a type, or a name.
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
        if w >> 8 != 0 {
            return self.word_action(p, e, w);
        }
        if w != 0 {
            self.put_hot(p, e, w);
        }
        e
    }

    /// A word `b[p..e]` whose pattern reads past it (`w`'s action): `func`'s declaration,
    /// `import`'s path, `private(set)`, `as?`, `Array<`, `Dictionary<`, `Optional<`.
    #[inline(never)]
    fn word_action(&mut self, p: usize, e: usize, w: u16) -> usize {
        let b = self.b;
        let kind = w & 0xff;
        match w & 0xff00 {
            W_FUNC => {
                let Some((s, ne)) = func_name(b, e) else {
                    self.put(p, e, kind);
                    return e;
                };
                self.put(p, e, STORAGE_FUNCTION);
                self.put(s, ne, FUNCTION_NAME);
                self.push(F_HEAD);
                self.head(ne, false)
            }
            W_IMPORT => self.import(p, e),
            W_ACCESS => {
                let e = if b[e..].starts_with(b"(set)") {
                    e + 5
                } else {
                    e
                };
                self.put(p, e, kind);
                e
            }
            W_AS => {
                let e = if b.get(e) == Some(&b'?') && !is_word(b, e + 1) {
                    e + 1
                } else {
                    e
                };
                self.put(p, e, kind);
                e
            }
            W_ARRAY | W_DICTIONARY if b.get(e) == Some(&b'<') => self.collection(p, e, w),
            W_OPTIONAL => {
                if b.get(e) == Some(&b'<') {
                    e + 1
                } else {
                    e
                }
            }
            _ => {
                self.put(p, e, kind);
                e
            }
        }
    }

    /// `Array<` or `Dictionary<` at `p`, its `<` at `e`: the type's frame opened.
    #[inline(always)]
    fn collection(&mut self, p: usize, e: usize, w: u16) -> usize {
        if w & 0xff00 == W_ARRAY {
            self.put(p, e, ARRAY);
            self.put(e, e + 1, ARRAY_BEGIN);
            self.push(F_ARRAY);
        } else {
            self.put(p, e, DICTIONARY);
            self.put(e, e + 1, DICTIONARY_BEGIN);
            self.push(F_DICTIONARY);
        }
        e + 1
    }

    /// `import` at `p..e`: bat's declaration, `import [kind] path`, or the keyword alone.
    fn import(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let s = skip_spaces(b, e);
        if s == e {
            self.put(p, e, K_DECLARATION);
            return e;
        }
        let mut path = s;
        let mut kind = None;
        if WORD[usize::from(*b.get(s).unwrap_or(&0))] {
            let we = word_end(b, s);
            let t = skip_spaces(b, we);
            if t > we
                && matches!(
                    &b[s..we],
                    b"typealias" | b"struct" | b"class" | b"enum" | b"protocol" | b"var" | b"func"
                )
                && path_piece(b, t).is_some()
            {
                kind = Some(we);
                path = t;
            }
        }
        let Some(mut pe) = path_piece(b, path) else {
            self.put(p, e, K_DECLARATION);
            return e;
        };
        while b.get(pe) == Some(&b'.') {
            match path_piece(b, pe + 1) {
                Some(q) => pe = q,
                None => break,
            }
        }
        self.put(p, e, IMPORT);
        if let Some(we) = kind {
            self.put(s, we, IMPORT_KIND);
        }
        self.put(path, pe, MODULE);
        pe
    }

    /// A function's head from `p` (a line's start where `fresh`): bat's declaration context,
    /// which takes only `<` (the generic clause), `(` (the parameters), `->` (the result) and
    /// `{` (the body), every other byte plain. Comments are comments (bat's read as code). A
    /// line that starts with no word or byte that goes on with a head (`throws`, `where`, `->`,
    /// `{` …) ends it: a protocol's requirement has no body (bat's stays open until a later
    /// `{ … }`, its words plain). Where the code goes on from.
    #[inline(never)]
    fn head(&mut self, mut p: usize, mut fresh: bool) -> usize {
        let b = self.b;
        let n = b.len();
        while p < n {
            let c = b[p];
            if ACT[usize::from(c)] == A_SPACE {
                p += 1;
                continue;
            }
            if c == b'/' && matches!(b.get(p + 1), Some(b'/' | b'*')) {
                p = self.head_comment(p);
                if p == n {
                    return n;
                }
                continue;
            }
            if fresh && !continues_head(b, p) {
                self.pop();
                return p;
            }
            fresh = false;
            match c {
                b'<' => {
                    self.put(p, p + 1, GENERIC_BEGIN);
                    self.push(F_GENERIC);
                    return p + 1;
                }
                b'(' => {
                    self.put(p, p + 1, PARAMS_BEGIN);
                    self.push(F_PARAMS);
                    return p + 1;
                }
                b'-' if b.get(p + 1) == Some(&b'>')
                    && !(p > 0 && OPCH[usize::from(b[p - 1])])
                    && !b.get(p + 2).is_some_and(|&d| OPCH[usize::from(d)]) =>
                {
                    self.put(p, p + 2, RESULT_ARROW);
                    self.push(F_RESULT);
                    return self.result(p + 2, false);
                }
                b'{' => {
                    self.pop();
                    return self.body(p);
                }
                _ if WORD[usize::from(c)] || c >= 0x80 && is_word_starting(b, p) => {
                    p = word_end(b, p);
                }
                _ => p += char_len(c),
            }
        }
        n
    }

    /// A function's result from `p` (a line's start where `fresh`), after its `->`: bat's
    /// result context, which takes only types (`Int`…, `Array<`, `Dictionary<`), to the body's
    /// `{`; a line ends it as it ends a head ([`Scan::head`]).
    #[inline(never)]
    fn result(&mut self, mut p: usize, mut fresh: bool) -> usize {
        let b = self.b;
        let n = b.len();
        while p < n {
            let c = b[p];
            if ACT[usize::from(c)] == A_SPACE {
                p += 1;
                continue;
            }
            if c == b'/' && matches!(b.get(p + 1), Some(b'/' | b'*')) {
                p = self.head_comment(p);
                if p == n {
                    return n;
                }
                continue;
            }
            if fresh && !continues_head(b, p) {
                self.pop();
                self.pop();
                return p;
            }
            fresh = false;
            if c == b'{' {
                self.pop();
                self.pop();
                return self.body(p);
            }
            if WORD[usize::from(c)] || c >= 0x80 && is_word_starting(b, p) {
                let e = word_end(b, p);
                let w = WORDS.value(&b[p..e], &INFO);
                p = match w & 0xff00 {
                    W_ARRAY | W_DICTIONARY if b.get(e) == Some(&b'<') => {
                        return self.collection(p, e, w);
                    }
                    W_OPTIONAL if b.get(e) == Some(&b'<') => e + 1,
                    _ => {
                        if w & 0xff == SUPPORT_TYPE {
                            self.put(p, e, SUPPORT_TYPE);
                        }
                        e
                    }
                };
            } else {
                p += char_len(c);
            }
        }
        n
    }

    /// A comment at `p` in a head or a result: where the head goes on, the line's end where
    /// the comment does.
    #[inline(never)]
    fn head_comment(&mut self, p: usize) -> usize {
        let b = self.b;
        if b[p + 1] == b'/' {
            self.put(p, p + 2, LINE_MARK);
            self.put(p + 2, b.len(), LINE_COMMENT);
            b.len()
        } else {
            self.comment_open(p)
        }
    }

    /// A function's body's `{` at `p`.
    #[inline(always)]
    fn body(&mut self, p: usize) -> usize {
        self.put(p, p + 1, BODY_BEGIN);
        self.push(F_BODY);
        p + 1
    }

    /// `<` or `>` at `p` in an angle's frame: a nested type's `<`, a frame's `>`; where the code
    /// goes on (a head's or a result's, once its frame closes).
    #[inline(never)]
    fn angle(&mut self, p: usize) -> usize {
        if self.b[p] == b'<' {
            self.push(F_ANGLE);
            return p + 1;
        }
        let f = self.top();
        self.pop();
        let kind = match f {
            F_GENERIC => GENERIC_END,
            F_ARRAY => ARRAY_END,
            F_DICTIONARY => DICTIONARY_END,
            _ => return p + 1,
        };
        self.put(p, p + 1, kind);
        match self.top() {
            F_HEAD => self.head(p + 1, false),
            F_RESULT => self.result(p + 1, false),
            _ => p + 1,
        }
    }

    /// A number at `p`: bat's integer forms, else its float's, which colors nothing; a word
    /// that is neither is plain.
    #[inline(always)]
    fn number(&mut self, p: usize) -> usize {
        let b = self.b;
        let e = word_end(b, p);
        let kind = int_kind(&b[p..e]);
        if kind != 0 {
            self.put_hot(p, e, kind);
            return e;
        }
        float_end(b, p).unwrap_or(e)
    }

    /// An operator's run at `p` (to a comment's start), classed as bat's grammar classes it:
    /// a named operator where the run is one, else a custom one by the bytes around it.
    #[inline(always)]
    fn op(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        if matches!(b[p], b'<' | b'>')
            && matches!(self.top(), F_GENERIC | F_ANGLE | F_ARRAY | F_DICTIONARY)
        {
            return self.angle(p);
        }
        let mut e = p + 1;
        while e < n && OPCH[usize::from(b[e])] {
            if b[e] == b'/' && matches!(b.get(e + 1), Some(b'/' | b'*')) {
                break;
            }
            e += 1;
        }
        // bat's integer with its sign, `\B\-[0-9]…\b`, is a literal before it is an operator
        if e == p + 1 && b[p] == b'-' && e < n && b[e].is_ascii_digit() && !is_word_before(b, p) {
            let q = word_end(b, e);
            let kind = int_kind(&b[e..q]);
            if kind != 0 {
                self.put(p, q, kind);
                return q;
            }
        }
        let kind = match named_op(&b[p..e]) {
            0 => {
                // a comment after the run is a blank to it, as to Swift's lexer
                let after = b.get(e) == Some(&b'/') || close_at(b, e);
                if open_before(b, p) {
                    if after { BINARY_OP } else { PREFIX }
                } else if !after {
                    POSTFIX
                } else {
                    0
                }
            }
            k => k,
        };
        if kind != 0 {
            self.put_hot(p, e, kind);
        }
        e
    }

    /// `@` at `p`: an attribute, its name's `(` opening its arguments.
    #[inline(always)]
    fn attribute(&mut self, p: usize) -> usize {
        let b = self.b;
        if !is_word_starting(b, p + 1) {
            return p + 1;
        }
        let e = word_end(b, p + 1);
        self.put(p, p + 1, ATTRIBUTE_MARK);
        if b.get(e) == Some(&b'(') {
            self.put(p + 1, e, ATTRIBUTE_ARGS);
            self.push(F_ATTR);
            e + 1
        } else {
            self.put(p + 1, e, ATTRIBUTE);
            e
        }
    }

    /// `#` at `p`: a raw string's `#"`, a shebang at the line's start, else plain.
    #[inline(always)]
    fn hash(&mut self, p: usize) -> usize {
        let b = self.b;
        let mut q = p + 1;
        while q < b.len() && b[q] == b'#' {
            q += 1;
        }
        if b.get(q) == Some(&b'"') {
            return self.quote(p, q, (q - p).min(255) as u8);
        }
        if b.get(q) == Some(&b'/') {
            return self.extended_regex(p, q);
        }
        if p == 0 && b.get(1) == Some(&b'!') {
            self.put(0, 2, SHEBANG_MARK);
            self.put(2, b.len(), SHEBANG);
            return b.len();
        }
        p + 1
    }

    /// `/` at `p`, no comment's: a regex literal where an operand may start (SE-0354: not after
    /// an operand, not followed by a blank, closed on the line outside a character class), else
    /// an operator.
    #[inline(never)]
    fn slash(&mut self, p: usize) -> usize {
        let b = self.b;
        if self.misses < REGEX_TRIES
            && operand_may_start(b, p)
            && !matches!(b.get(p + 1), None | Some(b' ' | b'\t'))
        {
            match regex_end(b, p + 1, 0) {
                Ok(e) => return self.regex_literal(p, p + 1, e - 1, e),
                Err(true) => self.misses += 1,
                Err(false) => {}
            }
        }
        self.op(p)
    }

    /// An extended regex literal's `#`s at `p`, its `/` at `q`: closed by `/` and as many `#`s on
    /// the line, or, `#/` ending the line, on a later one ([`M_REGEX`]); else `#` is plain.
    #[inline(never)]
    fn extended_regex(&mut self, p: usize, q: usize) -> usize {
        let b = self.b;
        let n = (q - p).min(255) as u8;
        if q + 1 == b.len() {
            self.put(p, q + 1, RE_BEGIN);
            self.mode = M_REGEX;
            self.hashes = n;
            self.re = 0;
            return q + 1;
        }
        if self.misses >= REGEX_TRIES {
            return p + 1;
        }
        match regex_end(b, q + 1, n) {
            Ok(e) => self.regex_literal(p, q + 1, e - 1 - usize::from(n), e),
            Err(eol) => {
                self.misses += u8::from(eol);
                p + 1
            }
        }
    }

    /// A multi-line regex literal's line from `p`, to its `/` and `#`s or the line's end.
    #[inline(never)]
    fn regex_lines(&mut self, p: usize) -> usize {
        let b = self.b;
        let h = self.hashes;
        let mut q = p;
        while q < b.len() {
            if b[q] == b'/' && hashes_at(b, q + 1, h) {
                let e = q + 1 + usize::from(h);
                self.regex_body(p, q, &RE_X_EMBED);
                self.put(q, e, RE_END);
                self.mode = M_CODE;
                self.re = 0;
                return e;
            }
            q += 1;
        }
        self.regex_body(p, b.len(), &RE_X_EMBED);
        b.len()
    }

    /// A regex literal on one line from `p`: its `#`s and `/` to `s`, its body to `be`, its `/`
    /// and `#`s to `e`. Where it ends.
    #[inline(never)]
    fn regex_literal(&mut self, p: usize, s: usize, be: usize, e: usize) -> usize {
        self.put(p, s, RE_BEGIN);
        self.re = 0;
        self.regex_body(s, be, &RE_EMBED);
        self.re = 0;
        self.put(be, e, RE_END);
        e
    }

    /// A regex literal's body from `s` to `be` on [`crate::regex`], the line cut at `be`, from
    /// the state in [`Scan::re`] and left there. On [`RE_X_EMBED`] (a multi-line literal's
    /// extended syntax) a `#` outside a class starts a comment to `be`.
    fn regex_body(&mut self, s: usize, be: usize, embed: &regex::Embed) {
        if s >= be {
            return;
        }
        let b = &self.b[..be];
        self.k.close(self.out);
        let mut at = s;
        let mut q = s;
        while let Some((e, st)) = regex::embedded(b, q, self.re, embed, &mut at, self.out) {
            self.re = st;
            if e >= be {
                break;
            }
            // stopped at a `#`: a character in a class or `\Q…\E`, which the scan takes from
            // there, else a comment
            if st & regex::VERBATIM != 0 {
                q = e;
                continue;
            }
            self.out.push(e, be, RE_COMMENT);
            break;
        }
        self.k.reopen(self.out, 16);
    }

    /// A string's opening quote at `q`, its `#`s from `s`: `"""` a multi-line one.
    #[inline(always)]
    fn quote(&mut self, s: usize, q: usize, raw: u8) -> usize {
        let b = self.b;
        if b.get(q + 1..q + 3) == Some(b"\"\"") {
            self.put(s, q + 3, STRING);
            self.string(q + 3, raw, true)
        } else {
            self.put_hot(s, q + 1, STRING);
            self.string(q + 1, raw, false)
        }
    }

    /// A string's body from `p`: its escapes, its interpolations, to its closing quote, or the
    /// line's end (a multi-line string goes on: [`M_MULTI`]).
    #[inline(always)]
    fn string(&mut self, mut p: usize, raw: u8, multi: bool) -> usize {
        let b = self.b;
        let n = b.len();
        let mut t = p;
        while p < n {
            match b[p] {
                b'"' => {
                    let q = if multi {
                        if b.get(p + 1..p + 3) != Some(b"\"\"") {
                            p += 1;
                            continue;
                        }
                        p + 3
                    } else {
                        p + 1
                    };
                    if hashes_at(b, q, raw) {
                        let e = q + usize::from(raw);
                        self.put_hot(t, e, STRING);
                        self.mode = M_CODE;
                        return e;
                    }
                    p = q;
                }
                b'\\' => {
                    if !hashes_at(b, p + 1, raw) {
                        p += 1;
                        continue;
                    }
                    let q = p + 1 + usize::from(raw);
                    self.put(t, p, STRING);
                    match b.get(q) {
                        Some(b'(') => {
                            self.put(p, q + 1, INTERP_BEGIN);
                            self.push(match (raw > 0, multi) {
                                (false, false) => F_INTERP,
                                (false, true) => F_INTERP_MULTI,
                                (true, false) => F_INTERP_RAW,
                                (true, true) => F_INTERP_RAW_MULTI,
                            });
                            if raw > 0 {
                                self.raws = self.raws << 8 | u32::from(raw);
                            }
                            self.mode = M_CODE;
                            return q + 1;
                        }
                        Some(b'0' | b'\\' | b't' | b'n' | b'r' | b'"' | b'\'') => {
                            self.put(p, q + 1, ESCAPE);
                            p = q + 1;
                        }
                        Some(b'u') if b.get(q + 1) == Some(&b'{') => {
                            let mut e = q + 2;
                            while e < n && e < q + 10 && b[e].is_ascii_hexdigit() {
                                e += 1;
                            }
                            if e > q + 2 && b.get(e) == Some(&b'}') {
                                self.put(p, e + 1, ESCAPE);
                                p = e + 1;
                            } else {
                                self.put(p, q, INVALID);
                                p = q;
                            }
                        }
                        None if multi => {
                            self.put(p, q, ESCAPE);
                            p = q;
                        }
                        _ => {
                            self.put(p, q, INVALID);
                            p = q;
                        }
                    }
                    t = p;
                }
                _ => p += 1,
            }
        }
        self.put(t, n, STRING);
        if multi {
            self.mode = M_MULTI;
            self.hashes = raw;
        } else {
            self.mode = M_CODE;
        }
        n
    }

    /// `/*` at `p`: a block comment, `/**` a documentation one (`/**/` an empty block).
    #[inline(always)]
    fn comment_open(&mut self, p: usize) -> usize {
        let b = self.b;
        let doc = b.get(p + 2) == Some(&b'*') && b.get(p + 3) != Some(&b'/');
        self.nest = 1;
        if doc {
            self.mode = M_DOC;
            self.put(p, p + 3, DOC_BEGIN);
            self.comment(p + 3)
        } else {
            self.mode = M_COMMENT;
            self.put(p, p + 2, BLOCK_BEGIN);
            self.comment(p + 2)
        }
    }

    /// A block comment's body from `p`, nested ones counted, to its `*/` or the line's end.
    #[inline(always)]
    fn comment(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let (text, end) = if self.mode == M_DOC {
            (DOC_COMMENT, DOC_END)
        } else {
            (BLOCK_COMMENT, BLOCK_END)
        };
        let t = p;
        while p + 1 < n {
            match (b[p], b[p + 1]) {
                (b'*', b'/') => {
                    self.nest = self.nest.saturating_sub(1);
                    if self.nest == 0 {
                        self.put(t, p, text);
                        self.put(p, p + 2, end);
                        self.mode = M_CODE;
                        return p + 2;
                    }
                    p += 2;
                }
                (b'/', b'*') => {
                    self.nest = self.nest.saturating_add(1);
                    p += 2;
                }
                _ => p += 1,
            }
        }
        self.put(t, n, text);
        n
    }
}

/// Whether the character before `p` is a word's (`\b` would hold at `p` before a non-word).
#[inline(always)]
fn is_word_before(b: &[u8], p: usize) -> bool {
    p > 0 && is_word(b, p - 1)
}

/// Whether `[\s(\[{,;:]` holds before `p` (`\s` Unicode's spaces too): what stands before a
/// prefix or a binary custom operator, or the ternary's `?` and `:`.
#[inline(always)]
fn open_before(b: &[u8], p: usize) -> bool {
    match p.checked_sub(1).map(|i| b[i]) {
        None => false,
        Some(c) if c < 0x80 => OPEN_SIDE[usize::from(c)],
        Some(_) => wide_space_before(b, p),
    }
}

/// Whether `[\s)\]},;:]` holds at `q`, the line's `\n` at its end: what stands after a binary
/// custom operator, or the ternary's `?` and `:`.
#[inline(always)]
fn close_at(b: &[u8], q: usize) -> bool {
    match b.get(q) {
        None => true,
        Some(&c) if c < 0x80 => CLOSE_SIDE[usize::from(c)],
        Some(_) => wide_space(b, q).is_some(),
    }
}

/// Whether the character before `p`, past ASCII, is one of Unicode's spaces.
#[cold]
fn wide_space_before(b: &[u8], p: usize) -> bool {
    let mut s = p - 1;
    while s > 0 && p - s < 4 && b[s] & 0xc0 == 0x80 {
        s -= 1;
    }
    wide_space(b, s).is_some_and(|l| s + l == p)
}

/// Where the blanks at `p` end.
#[inline(always)]
fn skip_spaces(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && ACT[usize::from(b[p])] == A_SPACE {
        p += 1;
    }
    p
}

/// A function's name after `func` at `e`, as bat's declaration reads it: blanks, a name (a
/// word, a backquoted one, `$0`, or an operator's characters), blanks, then `(` or `<`.
fn func_name(b: &[u8], e: usize) -> Option<(usize, usize)> {
    let s = skip_spaces(b, e);
    if s == e {
        return None;
    }
    let c = *b.get(s)?;
    let opens = |q: usize| matches!(b.get(skip_spaces(b, q)), Some(b'(' | b'<'));
    let ne = if c == b'`' {
        // any quoted name, Swift 6.2's raw ones too (bat's takes a word only)
        quoted_end(b, s)?
    } else if c == b'$' {
        let mut q = s + 1;
        while q < b.len() && b[q].is_ascii_digit() {
            q += 1;
        }
        if q == s + 1 {
            return None;
        }
        q
    } else if is_word_starting(b, s) {
        word_end(b, s)
    } else if OPCH[usize::from(c)] {
        let mut q = s + 1;
        while q < b.len() && OPCH[usize::from(b[q])] {
            q += 1;
        }
        if !opens(q) {
            // the run backs off to its last `<`, which then opens the generic clause
            let lt = b[s + 1..q].iter().rposition(|&d| d == b'<')? + s + 1;
            return Some((s, lt));
        }
        q
    } else {
        return None;
    };
    opens(ne).then_some((s, ne))
}

/// Where a quoted name at `p`'s `` ` `` ends, past its closing one on the line.
#[inline(always)]
fn quoted_end(b: &[u8], p: usize) -> Option<usize> {
    let e = p + 2 + b.get(p + 2..)?.iter().position(|&c| c == b'`')?;
    (b[p + 1] != b'`').then_some(e + 1)
}

/// A piece of an import's path at `p`: a word, or an operator's characters; where it ends.
fn path_piece(b: &[u8], p: usize) -> Option<usize> {
    let c = *b.get(p)?;
    if is_word_starting(b, p) {
        Some(word_end(b, p))
    } else if OPCH[usize::from(c)] {
        let mut q = p + 1;
        while q < b.len() && OPCH[usize::from(b[q])] {
            q += 1;
        }
        Some(q)
    } else {
        None
    }
}

/// Whether a line that starts at `p` in a function's head or result goes on with it: its
/// body's `{`, `->`, the effects and a `where` clause, or a list's `,` or `&`.
fn continues_head(b: &[u8], p: usize) -> bool {
    match b[p] {
        b'{' | b',' | b'&' => true,
        b'-' => b.get(p + 1) == Some(&b'>'),
        c if WORD[usize::from(c)] => matches!(
            &b[p..word_end(b, p)],
            b"throws" | b"rethrows" | b"async" | b"reasync" | b"where"
        ),
        _ => false,
    }
}

/// Whether an operand may start at `p`: the line's first token, or after an operator, an
/// opening bracket, a separator or a keyword that an expression follows; not after a name, a
/// literal or a closing bracket (an operand: `/` divides).
fn operand_may_start(b: &[u8], p: usize) -> bool {
    let mut q = p;
    while q > 0 && matches!(b[q - 1], b' ' | b'\t') {
        q -= 1;
    }
    let Some(&c) = q.checked_sub(1).map(|i| &b[i]) else {
        return true;
    };
    match c {
        b'(' | b'[' | b'{' | b',' | b';' | b':' | b'=' | b'&' | b'|' | b'^' | b'~' | b'<'
        | b'>' | b'+' | b'-' | b'*' | b'%' => true,
        b'!' | b'?' => q < 2 || !is_word(b, q - 2) && !matches!(b[q - 2], b')' | b']'),
        _ if WORD[usize::from(c)] => {
            let mut s = q - 1;
            while s > 0 && WORD[usize::from(b[s - 1])] {
                s -= 1;
            }
            matches!(
                &b[s..q],
                b"return"
                    | b"case"
                    | b"in"
                    | b"where"
                    | b"if"
                    | b"guard"
                    | b"while"
                    | b"try"
                    | b"await"
                    | b"throw"
                    | b"else"
                    | b"switch"
                    | b"for"
                    | b"repeat"
                    | b"is"
                    | b"as"
                    | b"let"
                    | b"var"
                    | b"yield"
            )
        }
        _ => false,
    }
}

/// Where a regex literal's body from `p` ends: past its closing `/` and `n` `#`s, outside a
/// character class and an escape, on the line; else whether the line's end was reached. A
/// bare one (`n` 0) doesn't end with a blank nor hold an unbalanced `)` (SE-0354: `/` is a
/// prefix operator then, `foo(/a, b: /c)`).
fn regex_end(b: &[u8], mut p: usize, n: u8) -> Result<usize, bool> {
    let mut class = 0u32;
    let mut group = 0u32;
    while p < b.len() {
        match b[p] {
            b'\\' => p += 1,
            b'[' => class += 1,
            b']' => class = class.saturating_sub(1),
            b'(' if class == 0 => group += 1,
            b')' if class == 0 => {
                if group == 0 && n == 0 {
                    return Err(false);
                }
                group = group.saturating_sub(1);
            }
            b'/' if class == 0 && hashes_at(b, p + 1, n) => {
                if n == 0 && matches!(b[p - 1], b' ' | b'\t') {
                    return Err(false);
                }
                return Ok(p + 1 + usize::from(n));
            }
            _ => {}
        }
        p += 1;
    }
    Err(true)
}

/// Whether `n` `#`s stand at `p`.
#[inline(always)]
fn hashes_at(b: &[u8], p: usize, n: u8) -> bool {
    n == 0
        || b.get(p..p + usize::from(n))
            .is_some_and(|h| h.iter().all(|&c| c == b'#'))
}

/// A word that starts with a digit: bat's integer forms' kind, or 0.
#[inline(always)]
fn int_kind(w: &[u8]) -> u16 {
    fn all(w: &[u8], f: impl Fn(u8) -> bool) -> bool {
        w.iter().all(|&c| f(c) || c == b'_')
    }
    if w.len() > 2 && w[0] == b'0' {
        let body = &w[2..];
        let first = body[0];
        match w[1] {
            b'b' if (first == b'0' || first == b'1') && all(body, |c| c == b'0' || c == b'1') => {
                return BINARY;
            }
            b'o' if (b'0'..=b'7').contains(&first) && all(body, |c| (b'0'..=b'7').contains(&c)) => {
                return OCTAL;
            }
            b'x' if first.is_ascii_hexdigit() && all(body, |c| c.is_ascii_hexdigit()) => {
                return HEX;
            }
            _ => {}
        }
    }
    if all(w, |c| c.is_ascii_digit()) {
        DECIMAL
    } else {
        0
    }
}

/// bat's float, `\b[0-9][0-9_]*(\.[0-9][0-9_]*)?([eE][+-]?[0-9][0-9_]*)?\b`, at `p`: where it
/// ends, a word's end there.
#[inline(always)]
fn float_end(b: &[u8], p: usize) -> Option<usize> {
    let digits = |mut q: usize| {
        if !b.get(q).is_some_and(u8::is_ascii_digit) {
            return None;
        }
        while q < b.len() && (b[q].is_ascii_digit() || b[q] == b'_') {
            q += 1;
        }
        Some(q)
    };
    let ends = |q: usize| !is_word(b, q);
    let mut q = digits(p)?;
    if b.get(q) == Some(&b'.')
        && let Some(r) = digits(q + 1)
    {
        q = r;
    }
    if matches!(b.get(q), Some(b'e' | b'E')) {
        let mut r = q + 1;
        if matches!(b.get(r), Some(b'+' | b'-')) {
            r += 1;
        }
        if let Some(r) = digits(r)
            && ends(r)
        {
            return Some(r);
        }
    }
    ends(q).then_some(q)
}

/// The named operator a whole run is, in `operator`'s order, or 0.
#[inline(always)]
fn named_op(r: &[u8]) -> u16 {
    match r {
        b"." => 0,
        b"=" => ASSIGNMENT,
        b"==" | b"===" | b"!=" | b"!==" | b"<" | b"<=" | b">" | b">=" | b"~=" => COMPARATIVE,
        b"+=" | b"-=" | b"*=" | b"/=" | b"%=" | b"<<=" | b">>=" | b"&=" | b"^=" | b"|="
        | b"&&=" | b"||=" => ASSIGNMENT,
        b"!" | b"&&" | b"||" => LOGICAL,
        b"%" => REMAINDER,
        b"++" | b"--" => INCREMENT,
        b"&+" | b"&-" | b"&*" | b"&/" | b"&%" => OVERFLOW,
        b".." | b"..." => RANGE,
        b"&" | b"|" | b"^" | b"<<" | b">>" => BITWISE,
        b"+" | b"-" | b"*" | b"/" => ARITHMETIC,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Lang;
    use crate::tests::marked;

    fn of(lines: &[&str]) -> Vec<(String, String)> {
        marked(Lang::Swift, lines)
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

    fn lacks(lines: &[&str], text: &str) {
        let got = of(lines);
        assert!(!got.iter().any(|(t, _)| t == text), "{text:?} in {got:#?}");
    }

    #[test]
    fn declarations() {
        let l = [
            "import struct Foundation.Date",
            "@testable import Kit",
            "@available(iOS 13, *) public final class A: B {",
            "    private(set) var v: Int = 0",
            "    static func == <T>(lhs: (Int) -> Void, rhs: A) async throws -> [String: Any] {",
            "        if v { return [:] }",
            "    }",
            "}",
        ];
        has(&l, "import", "keyword.other.import.swift");
        has(&l, "struct", "storage.modifier.swift");
        has(&l, "Foundation.Date", "support.type.module.import.swift");
        has(&l, "Kit", "support.type.module.import.swift");
        has(&l, "testable", "storage.modifier.attribute.swift");
        has(
            &l,
            "available",
            "storage.modifier.attribute.swift punctuation.definition.attribute-arguments.begin.swift",
        );
        has(
            &l,
            ")",
            "punctuation.definition.attribute-arguments.end.swift",
        );
        has(
            &l,
            "private(set)",
            "keyword.other.access-level-modifier.swift",
        );
        has(&l, "func", "storage.type.function.swift");
        has(&l, "==", "entity.type.function.swift");
        has(
            &l,
            "<",
            "punctuation.definition.generic-parameter-clause.begin.swift",
        );
        has(
            &l,
            "(",
            "punctuation.definition.function-arguments.begin.swift",
        );
        // the parameters nest (bat's end at the first `)`): the function type's `->` is an operator
        has(&l, "->", "keyword.operator.custom.binary.swift");
        has(
            &l,
            ")",
            "punctuation.definition.function-arguments.end.swift",
        );
        has(&l, "->", "keyword.operator.function-result.swift");
        has(&l, "String", "support.type.swift");
        has(&l, "{", "punctuation.definition.code-block.begin.swift");
        // the body's own `}` ends it, not the `if`'s
        let got = of(&l);
        let ends: Vec<_> = got
            .iter()
            .filter(|(_, s)| s == "punctuation.definition.code-block.end.swift")
            .collect();
        assert_eq!(ends.len(), 1, "{got:#?}");
        // `throws` and `async` are plain in a head, as in bat's
        lacks(&l, "throws");
    }

    #[test]
    fn a_requirement_without_a_body_ends_its_head() {
        let l = [
            "protocol P {",
            "    func foo() -> Int",
            "    func bar(a: Int)",
            "    var name: String { get }",
            "}",
            "func baz()",
            "    where T: Equatable",
            "{",
            "}",
        ];
        let got = of(&l);
        let funcs = got
            .iter()
            .filter(|(t, s)| t == "func" && s == "storage.type.function.swift")
            .count();
        assert_eq!(funcs, 3, "{got:#?}");
        has(&l, "var", "storage.type.swift");
        has(&l, "get", "keyword.other.swift");
        // a head's `where` goes on to the next line's `{`
        has(&l, "{", "punctuation.definition.code-block.begin.swift");
        has(&l, "}", "punctuation.definition.code-block.end.swift");
    }

    #[test]
    fn strings_and_comments() {
        let l = [
            r##"let s = "a\(f(1)) \u{1F600} \x41" + #"raw \(x) \#(y) "q""#"##,
            r#"let t = """"#,
            r#"  He said "hi" \(name)"#,
            r#"  """"#,
            "/* outer /* inner */ still */ let z = 1 // c",
            "/**/ let w = 2 /** doc */",
        ];
        has(&l, "\\(", "support.punctuation.expression.begin.swift");
        has(&l, ")", "support.punctuation.expression.end.swift");
        has(&l, "\\u{1F600}", "constant.character.escape.swift");
        has(&l, "\\", "invalid.illegal.swift");
        has(&l, "\\#(", "support.punctuation.expression.begin.swift");
        has(&l, " \"q\"\"#", "string.quoted.double.swift");
        has(&l, "  He said \"hi\" ", "string.quoted.double.swift");
        has(&l, " outer /* inner */ still ", "comment.block.swift");
        has(
            &l,
            "*/",
            "comment.block.swift punctuation.definition.comment.block.end.swift",
        );
        has(
            &l,
            "/**",
            "comment.block.documentation.swift punctuation.definition.comment.block.documentation.begin.swift",
        );
    }

    #[test]
    fn operators_numbers_and_regexes() {
        let l = [
            "let y = x-1 + -2 * 0x1F + 1e5 + 1_000 + 0b101 + a.b(c ?? d)!.e",
            "let r = cond ? /^(\\d+) calls/ : #/a/b/#",
            "let q = a / b / c, s = x as? Int",
            ".ifLet(\\.child, action: /Action.child) // not a regex",
        ];
        has(&l, "-", "keyword.operator.arithmetic.swift");
        has(&l, "-2", "constant.numeric.integer.decimal.swift");
        has(&l, "0x1F", "constant.numeric.integer.hexadecimal.swift");
        has(&l, "0b101", "constant.numeric.integer.binary.swift");
        lacks(&l, "1e5");
        has(&l, ".", "keyword.operator.custom.postfix.unary.swift");
        has(&l, "!.", "keyword.operator.custom.postfix.unary.swift");
        has(&l, "?", "keyword.operator.ternary.swift");
        const BEGIN: &str = "string.regexp.swift punctuation.definition.string.begin.swift";
        const END: &str = "string.regexp.swift punctuation.definition.string.end.swift";
        has(&l, "/", BEGIN);
        has(&l, "^", "string.regexp.swift keyword.control.anchor.regexp");
        has(
            &l,
            "\\d",
            "string.regexp.swift constant.other.character-class.escape.backslash.regexp",
        );
        has(&l, " calls", "string.regexp.swift");
        has(&l, "#/", BEGIN);
        has(&l, "a/b", "string.regexp.swift");
        has(&l, "/#", END);
        has(&l, "/", "keyword.operator.arithmetic.swift");
        has(&l, "as?", "keyword.operator.type-casting.swift");
        let got = of(&l);
        assert_eq!(
            got.iter().filter(|(_, s)| s == BEGIN).count(),
            2,
            "{got:#?}"
        );
    }

    /// A regex literal's body is on ada's regex scanner, in Swift's dialect.
    #[test]
    fn regex_literals_have_parts() {
        const RE: &str = "string.regexp.swift";
        let l = [
            "static let pattern = #/^(?<key>[a-z]+)-(\\d+)$/#",
            "let r = /(?P<n>a)(?'m'b)\\k'm'(?#note)[[:alpha:]--[a-c]]\\u{1F600}\\y*+/",
        ];
        has(
            &l,
            "#/",
            &format!("{RE} punctuation.definition.string.begin.swift"),
        );
        has(&l, "^", &format!("{RE} keyword.control.anchor.regexp"));
        has(&l, "key", &format!("{RE} entity.name.other.group.regexp"));
        has(
            &l,
            "a",
            &format!(
                "{RE} constant.other.character-class.set.regexp constant.other.character-class.range.regexp"
            ),
        );
        has(&l, "+", &format!("{RE} keyword.operator.quantifier.regexp"));
        has(&l, "-", RE);
        has(&l, "$", &format!("{RE} keyword.control.anchor.regexp"));
        has(
            &l,
            "/#",
            &format!("{RE} punctuation.definition.string.end.swift"),
        );
        has(&l, "n", &format!("{RE} entity.name.other.group.regexp"));
        has(&l, "m", &format!("{RE} entity.name.other.group.regexp"));
        has(&l, "m", &format!("{RE} variable.other.group.regexp"));
        has(&l, "note", &format!("{RE} comment.block.regexp"));
        has(
            &l,
            "[:alpha:]",
            &format!(
                "{RE} constant.other.character-class.set.regexp constant.other.character-class.posix.regexp"
            ),
        );
        has(
            &l,
            "--",
            &format!(
                "{RE} constant.other.character-class.set.regexp keyword.operator.difference.regexp"
            ),
        );
        has(
            &l,
            "\\u{1F600}",
            &format!("{RE} constant.character.escape.backslash.regexp"),
        );
        has(&l, "\\y", &format!("{RE} keyword.control.anchor.regexp"));
        has(
            &l,
            "*+",
            &format!("{RE} keyword.operator.quantifier.regexp"),
        );
    }

    /// A multi-line regex literal is in extended syntax: `#` outside a class starts a comment,
    /// and a group or a class stays open across its lines.
    #[test]
    fn multi_line_regex_literals_are_extended() {
        const RE: &str = "string.regexp.swift";
        let l = ["let r = #/", "  ( [#a] # a comment", "  \\d+ )", "/#.x"];
        has(
            &l,
            "#/",
            &format!("{RE} punctuation.definition.string.begin.swift"),
        );
        has(
            &l,
            "(",
            &format!("{RE} punctuation.definition.group.begin.regexp"),
        );
        has(
            &l,
            "#a",
            &format!("{RE} constant.other.character-class.set.regexp"),
        );
        has(
            &l,
            "# a comment",
            &format!("{RE} comment.line.number-sign.regexp"),
        );
        has(&l, "+", &format!("{RE} keyword.operator.quantifier.regexp"));
        has(
            &l,
            ")",
            &format!("{RE} punctuation.definition.group.end.regexp"),
        );
        has(
            &l,
            "/#",
            &format!("{RE} punctuation.definition.string.end.swift"),
        );
        lacks(&l, ".x");
    }

    #[test]
    fn quoted_names_are_names() {
        let l = [
            "case `default`, `for`",
            "@Test func `GET /{scope}/{name} returns releases`() async throws {}",
        ];
        lacks(&l, "default");
        lacks(&l, "for");
        has(
            &l,
            "`GET /{scope}/{name} returns releases`",
            "entity.type.function.swift",
        );
    }

    /// `text` a line at a time on the line API: each line's spans and the state after it.
    fn lines(text: &str) -> Vec<(Vec<crate::Span>, State)> {
        let mut state = State::default();
        let mut out = Vec::new();
        for l in text.lines() {
            let mut spans = Vec::new();
            line(l.as_bytes(), &mut state, &mut Out::new(&mut spans));
            out.push((spans, state));
        }
        out
    }

    #[test]
    fn a_block_lexes_as_its_lines() {
        // every field a line hands the next: frames past the levels, raw interpolations past
        // their four, a comment's depth, a multi-line string's and regex's `#`s, the regex's
        // open group and class, a head across lines
        let deep = "( ".repeat(40) + &") ".repeat(38);
        let raws = "#\"\\#(".repeat(6);
        let text = format!(
            "let a = {deep}\n))\nlet r = {raws}\n) \"# ) \"# ) \"# ) \"# ) \"# ) \"#\n\
             /* a /* b /* c\n*/ */ d */ x\n##\"\"\"\n  \"\"\" \\##(f(\n  1)) \n\"\"\"## y\n\
             let e = ##/\n  (a /# b\n  [x\n ]) /## z\nfunc f<T>(\n  a: T\n) -> Int\n  where T: P\n{{\n}}\n\
             func g()\nlet h = 1\n"
        );
        let one = lines(&text);
        let mut state = State::default();
        let mut all = Vec::new();
        let mut v = Vec::new();
        block(&mut state, &mut v, &mut |c| {
            for l in text.lines() {
                all.push(c.next(l.as_bytes()).to_vec());
            }
            None
        });
        assert_eq!(all, one.iter().map(|(s, _)| s.clone()).collect::<Vec<_>>());
        assert_eq!(state, one.last().unwrap().1);
        // the multi-line string and regex end, the head ends at `let`
        assert_eq!(one.last().unwrap().1.arg & 7, u16::from(M_CODE));
    }

    #[test]
    fn long_lines_lex_in_linear_time() {
        for unit in [
            "a(",
            "{ ",
            "\"\\(",
            "/* ",
            "func f(",
            "func f<",
            "#\"",
            "=/[a",
            "#/a",
            "(/a )",
            "x / ",
            "`a",
            "@a(",
            "Array<",
            "a.b ?? ",
            "import a.",
        ] {
            let l = unit.repeat(20_000);
            let t = std::time::Instant::now();
            lines(&l);
            assert!(t.elapsed().as_millis() < 200, "{unit:?}: {:?}", t.elapsed());
        }
    }
}
