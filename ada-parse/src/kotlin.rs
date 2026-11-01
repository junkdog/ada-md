//! Kotlin by a scanner of ada's own (ADR-40, speed first): bat's `Kotlin` grammar compiled by
//! hand, as Rust's was (`rust.rs`). Each of its contexts that can be open is an arm of one
//! `match` on the top of a stack, trying its patterns in the grammar's order at the position the
//! scan has reached, which is syntect's leftmost match; where no pattern can start (a word,
//! spaces, a string's text), the run is skipped whole.
//!
//! The grammar is small (62 contexts, no meta scopes but its strings' and comments'), so a kind
//! is a leaf, bat's scopes for a token ([`scopes`]), and the contexts that differ only in name
//! are one here: the bodies `namespace`, a class and a lambda open are [`BRACE`], `( … )` in an
//! expression and among a class's supertypes is [`PAREN`], a getter's and a setter's `= …` is
//! [`ACCESSOR_EXPR`]. It follows the grammar where it colors, odd corners included: `type`
//! anywhere in a statement opens a typedef to the line's end (`prototype` too, and its `<` then
//! opens generics); a word right before `<` opens a type's arguments (`i<n`); a function's `}`
//! closes the class around it; a class header colors every word before `<`, `{`, `(` or `:` as
//! its name, and a function's every word before `(`. Names past ASCII are approximated: every
//! byte past ASCII is a letter, and `\s` is ASCII's.
//!
//! **Where Kotlin reads it otherwise, Kotlin wins** (ADR-40): a `"""…"""` string has no escapes,
//! and ends at the last three quotes of a run; a string a construct makes a regex
//! (`Regex("…")`, `"…".toRegex()`) is colored as one ([`crate::regex`]).
//!
//! The stack between lines is the contexts open above the top level, up to 28 (the bytes of
//! [`State`]'s levels).

use crate::machine::KEPT as HALF;
use crate::regex;
use crate::{Out, State, is_space, skip_spaces, trim_end};

// The contexts, by bat's names where one is one of them.
/// The top level (`main`).
const MAIN: u8 = 0;
/// `{ … }` of statements: `namespaces`', a class's body, a lambda's (`types`').
const BRACE: u8 = 1;
/// A function's body, which its `}` doesn't close: the context under it does.
const FN_BODY: u8 = 2;
/// `( … )` of expressions.
const PAREN: u8 = 3;
/// A getter's or setter's `{ … }`, of expressions.
const EXPR_BRACE: u8 = 4;
/// A function's `= …`, to the line's end.
const FN_EXPR: u8 = 5;
/// A getter's or setter's `= …`, to the line's end or a `set`.
const ACCESSOR_EXPR: u8 = 6;
/// From `type` to the line's end.
const TYPEDEF: u8 = 7;
/// A parameter's default value.
const PARAM_DEFAULT: u8 = 8;
/// A type's arguments, `Name<` to `>`.
const ANGLE: u8 = 9;
/// `( … )` in a type.
const TYPE_PAREN: u8 = 10;
/// A parameter's type.
const PARAM_TYPE: u8 = 11;
/// A type parameter's bound, after `:`.
const BOUND: u8 = 12;
/// A function's return type.
const FN_RETURN: u8 = 13;
/// `< … >` of type parameters.
const GENERICS: u8 = 14;
/// `( … )` of parameters: a function's, a constructor's, a setter's.
const PARAMS: u8 = 15;
/// A class's, an object's or an interface's header, to the line's end.
const CLASS: u8 = 16;
const CLASS_NAME: u8 = 17;
/// After a class header's `:`.
const SUPERS: u8 = 18;
/// From `fun` to the line's end.
const FUNCTION: u8 = 19;
/// After `fun`, to `(`, across lines.
const FN_NAME: u8 = 20;
const GETTER: u8 = 21;
const SETTER: u8 = 22;
/// From `val` or `var` to `:`, `=`, `by` or the line's end.
const VARIABLE: u8 = 23;
const VAR_NAME: u8 = 24;
// text last
/// `"""…"""`.
const RAW: u8 = 25;
const STRING: u8 = 26;
const CHAR: u8 = 27;
/// `` `name` ``.
const BACKTICK: u8 = 28;
const COMMENT: u8 = 29;
/// A regex in the string below (ADR-40, phase 4): this bit, and the regex's state.
const REGEX: u8 = 0x80;

/// The leaves: a token's scopes.
static LEAVES: [&str; 49] = [
    "",
    "comment.line.double-slash.kotlin",
    "comment.line.double-slash.kotlin punctuation.definition.comment.kotlin",
    "comment.block.kotlin",
    "comment.block.kotlin punctuation.definition.comment.kotlin",
    "string.quoted.third.kotlin",
    "string.quoted.third.kotlin punctuation.definition.string.begin.kotlin",
    "string.quoted.third.kotlin punctuation.definition.string.end.kotlin",
    "string.quoted.third.kotlin constant.character.escape.kotlin",
    "string.quoted.third.kotlin variable.parameter.template.kotlin",
    "string.quoted.double.kotlin",
    "string.quoted.double.kotlin punctuation.definition.string.begin.kotlin",
    "string.quoted.double.kotlin punctuation.definition.string.end.kotlin",
    "string.quoted.double.kotlin constant.character.escape.kotlin",
    "string.quoted.double.kotlin variable.parameter.template.kotlin",
    "string.quoted.single.kotlin",
    "string.quoted.single.kotlin punctuation.definition.string.begin.kotlin",
    "string.quoted.single.kotlin punctuation.definition.string.end.kotlin",
    "string.quoted.single.kotlin constant.character.escape.kotlin",
    "storage.modifier.kotlin",
    "keyword.operator.declaration.kotlin",
    "keyword.operator.assignment.kotlin",
    "entity.name.type.class.kotlin",
    "entity.other.inherited-class.kotlin",
    "keyword.other.kotlin",
    "entity.name.function.kotlin",
    "storage.type.generic.kotlin",
    "entity.name.package.kotlin",
    "variable.parameter.function.kotlin",
    "constant.language.kotlin",
    "constant.numeric.kotlin",
    "constant.other.kotlin",
    "keyword.control.catch-exception.kotlin",
    "keyword.control.kotlin",
    "keyword.operator.kotlin",
    "keyword.operator.comparison.kotlin",
    "keyword.other.by.kotlin",
    "keyword.operator.safenav.kotlin",
    "keyword.operator.dot.kotlin",
    "keyword.operator.elvis.kotlin",
    "keyword.operator.increment-decrement.kotlin",
    "keyword.operator.arithmetic.assign.kotlin",
    "keyword.operator.arithmetic.kotlin",
    "keyword.operator.logical.kotlin",
    "punctuation.terminator.kotlin",
    "storage.type.buildin.kotlin",
    "storage.type.buildin.array.kotlin",
    "storage.type.buildin.collection.kotlin",
    "entity.name.variable.kotlin",
];

// The leaves by name, in the order of `LEAVES`.
const PLAIN: u16 = 0;
const LINE_COMMENT: u16 = 1;
const LINE_MARK: u16 = 2;
const BLOCK_COMMENT: u16 = 3;
const BLOCK_MARK: u16 = 4;
/// A string's leaves: its text, then these after it.
const RAW_TEXT: u16 = 5;
const STRING_TEXT: u16 = 10;
const CHAR_TEXT: u16 = 15;
const BEGIN: u16 = 1;
const END: u16 = 2;
const ESCAPE: u16 = 3;
const TEMPLATE: u16 = 4;
const MODIFIER: u16 = 19;
const DECLARATION: u16 = 20;
const ASSIGNMENT: u16 = 21;
const CLASS_LEAF: u16 = 22;
const INHERITED: u16 = 23;
const KEYWORD: u16 = 24;
const FUNCTION_LEAF: u16 = 25;
const GENERIC: u16 = 26;
const PACKAGE: u16 = 27;
const PARAMETER: u16 = 28;
const LANGUAGE: u16 = 29;
const NUMBER: u16 = 30;
const CONSTANT: u16 = 31;
const CATCH: u16 = 32;
const CONTROL: u16 = 33;
const OPERATOR: u16 = 34;
const COMPARISON: u16 = 35;
const BY: u16 = 36;
const SAFENAV: u16 = 37;
const DOT: u16 = 38;
const ELVIS: u16 = 39;
const INCREMENT: u16 = 40;
const ARITHMETIC_ASSIGN: u16 = 41;
const ARITHMETIC: u16 = 42;
const LOGICAL: u16 = 43;
const TERMINATOR: u16 = 44;
const BUILTIN: u16 = 45;
const ARRAY: u16 = 46;
const COLLECTION: u16 = 47;
const VARIABLE_LEAF: u16 = 48;

/// The kinds of a regex in a `"…"` string and in a `"""…"""` one: the regex's, from each.
const RE_STRING: u16 = 64;
const RE_RAW: u16 = 192;
const _: () = assert!(LEAVES.len() <= RE_STRING as usize);
const _: () = assert!(RE_STRING as usize + regex::KINDS <= RE_RAW as usize);

/// The scopes under `source.Kotlin` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    if kind >= RE_STRING {
        let (text, base) = if kind >= RE_RAW {
            (RAW_TEXT, RE_RAW)
        } else {
            (STRING_TEXT, RE_STRING)
        };
        let mut out = scopes(text);
        out.extend(regex::scopes(kind - base));
        return out;
    }
    LEAVES.get(usize::from(kind)).map_or_else(Vec::new, |l| {
        l.split(' ').filter(|s| !s.is_empty()).collect()
    })
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
        exact: state.is_exact(),
        text: PLAIN,
        at: 0,
        nodot: 0,
        opened: None,
        out,
    };
    let n = usize::from(state.depth);
    for i in 0..n {
        s.stack[1 + i] = if i < HALF {
            state.levels[i]
        } else {
            state.withs[i - HALF]
        };
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
    exact: bool,
    /// The kind of text no pattern matches.
    text: u16,
    /// Where the spans so far end.
    at: usize,
    /// Below it, no dotted prefix of a name ends in the run of name bytes there
    /// ([`Scan::dotted`]).
    nodot: usize,
    /// The last string opened on the line, not as a regex: where its text starts, and how many
    /// spans there were then ([`Scan::string_end`]).
    opened: Option<(usize, usize)>,
    out: &'b mut Out<'c>,
}

/// Whether the character that byte `i` of `b` is in is a word's, `\w`: past ASCII, a letter or
/// a number (Kotlin's test names in backticks hold dashes and arrows, which aren't).
fn is_word(b: &[u8], i: usize) -> bool {
    match b.get(i) {
        Some(&c) if c < 0x80 => c.is_ascii_alphanumeric() || c == b'_',
        Some(_) => wide_word(b, i),
        None => false,
    }
}

#[cold]
fn wide_word(b: &[u8], i: usize) -> bool {
    let s = (i.saturating_sub(3)..=i)
        .rev()
        .find(|&s| b[s] & 0xc0 != 0x80)
        .unwrap_or(i);
    let e = (s + char_len(b[s])).min(b.len());
    std::str::from_utf8(&b[s..e])
        .ok()
        .and_then(|t| t.chars().next())
        .is_some_and(char::is_alphanumeric)
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

fn word_end(b: &[u8], mut p: usize) -> usize {
    while is_word(b, p) {
        p += char_len(b[p]);
    }
    p
}

/// Whether `\b` holds before `p`, at a word's start.
fn starts_word(b: &[u8], p: usize) -> bool {
    p == 0 || !is_word(b, p - 1)
}

/// Whether `\b` holds at `e`, after a word.
fn ends_word(b: &[u8], e: usize) -> bool {
    !is_word(b, e)
}

/// Whether `\b` holds at `e`, after at least one byte.
fn boundary(b: &[u8], e: usize) -> bool {
    is_word(b, e - 1) != is_word(b, e)
}

/// Where the first byte from `from` that `stop` takes is, or the end.
fn next_of(b: &[u8], from: usize, stop: impl Fn(u8) -> bool) -> usize {
    b.get(from..)
        .and_then(|rest| rest.iter().position(|&c| stop(c)))
        .map_or(b.len(), |i| from + i)
}

/// A string's template at `p`, `(\$\w+|\$\{[^\}]+\})`: where it ends.
fn template(b: &[u8], p: usize) -> Option<usize> {
    match b.get(p + 1) {
        Some(_) if is_word(b, p + 1) => Some(word_end(b, p + 1)),
        Some(b'{') => {
            let close = next_of(b, p + 2, |c| c == b'}');
            (close < b.len() && close > p + 2).then_some(close + 1)
        }
        _ => None,
    }
}

/// Whether a `"""…"""` string ends at `p`: the last three quotes of a run, as Kotlin reads
/// them. bat's grammar ends it at the first three, and the quote left over opens a string that
/// colors the rest of the file (`""""a":1""""`).
fn raw_end(b: &[u8], p: usize) -> bool {
    b[p..].starts_with(b"\"\"\"") && b.get(p + 3) != Some(&b'"')
}

/// The bytes a string's patterns start on, around a regex: `"…"`'s, then `"""…"""`'s (no
/// escape: the regex reads its `\`).
static RE_YIELDS: [[bool; 256]; 2] = {
    let mut t = [[false; 256]; 2];
    t[0][b'"' as usize] = true;
    t[0][b'$' as usize] = true;
    t[0][b'\\' as usize] = true;
    t[1][b'"' as usize] = true;
    t[1][b'$' as usize] = true;
    t
};

/// A regex in a `"…"` string, then in a `"""…"""` one.
static RE_EMBEDS: [regex::Embed<'static>; 2] = [
    regex::Embed {
        cooked: true,
        yields: &RE_YIELDS[0],
        stop: |b, p| b[p] == b'"' || template(b, p).is_some(),
        base: RE_STRING,
        text: STRING_TEXT,
    },
    regex::Embed {
        cooked: false,
        yields: &RE_YIELDS[1],
        stop: |b, p| raw_end(b, p) || (b[p] == b'$' && template(b, p).is_some()),
        base: RE_RAW,
        text: RAW_TEXT,
    },
];

/// Whether the string whose quote opens at `p` is a regex: right after `Regex(`,
/// `Pattern.compile(` or `Pattern.matches(`. One right before `.toRegex(` or `.toPattern(` is
/// found at its end ([`Scan::string_end`]).
fn regex_string(b: &[u8], p: usize) -> bool {
    let q = trim_end(b, 0, p);
    if q == 0 || b[q - 1] != b'(' {
        return false;
    }
    let w = trim_end(b, 0, q - 1);
    // the name's last byte first: most calls end in none of the three
    let name: &[u8] = match w.checked_sub(1).map(|i| b[i]) {
        Some(b'x') => b"Regex",
        Some(b'e') => b"Pattern.compile",
        Some(b's') => b"Pattern.matches",
        _ => return false,
    };
    b[..w].ends_with(name) && (w == name.len() || !is_word(b, w - name.len() - 1))
}

/// `[A-Z][A-Z0-9_]+`, a whole word: a constant's name.
fn is_constant(w: &[u8]) -> bool {
    w.len() >= 2
        && w[0].is_ascii_uppercase()
        && w[1..]
            .iter()
            .all(|&c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
}

/// What a whole word is to bat's grammar.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Name {
    Other,
    Namespace,
    /// `class`, `object`, `interface`.
    Class,
    Companion,
    Fun,
    /// `var`, `val`: a statement's, and among the keywords a modifier.
    Binding,
    Get,
    Set,
    /// `storage.type.buildin`.
    Builtin,
    Array,
    /// A collection's name, before `<`.
    Collection,
    Language,
    Keyword(u16),
}

/// The word `w` among bat's keywords and type names, dispatching on its first byte.
fn lookup(w: &[u8]) -> Name {
    use Name::{
        Array as A, Binding, Builtin as T, Class, Collection as C, Companion, Fun, Get,
        Keyword as K, Language as L, Namespace, Other, Set,
    };
    let Some(&c) = w.first() else {
        return Other;
    };
    match c {
        b'a' => match w {
            b"abstract" | b"attribute" | b"annotation" => K(MODIFIER),
            b"as" | b"assert" => K(OPERATOR),
            _ => Other,
        },
        b'b' => match w {
            b"break" => K(CONTROL),
            b"by" => K(BY),
            _ => Other,
        },
        b'c' => match w {
            b"class" => Class,
            b"companion" => Companion,
            b"const" => K(MODIFIER),
            b"catch" => K(CATCH),
            b"continue" => K(CONTROL),
            _ => Other,
        },
        b'd' => match w {
            b"data" => K(MODIFIER),
            b"do" => K(CONTROL),
            b"dynamic" => T,
            _ => Other,
        },
        b'e' => match w {
            b"else" => K(CONTROL),
            b"enum" => K(MODIFIER),
            _ => Other,
        },
        b'f' => match w {
            b"fun" => Fun,
            b"for" => K(CONTROL),
            b"final" => K(MODIFIER),
            b"finally" => K(CATCH),
            b"false" => L,
            _ => Other,
        },
        b'g' => match w {
            b"get" => Get,
            _ => Other,
        },
        b'i' => match w {
            b"if" => K(CONTROL),
            b"in" | b"inline" | b"internal" | b"infix" => K(MODIFIER),
            b"is" => K(OPERATOR),
            b"interface" => Class,
            _ => Other,
        },
        b'n' => match w {
            b"null" => L,
            b"namespace" => Namespace,
            _ => Other,
        },
        b'o' => match w {
            b"object" => Class,
            b"override" | b"open" | b"out" | b"operator" => K(MODIFIER),
            _ => Other,
        },
        b'p' => match w {
            b"private" | b"public" | b"protected" => K(MODIFIER),
            _ => Other,
        },
        b'r' => match w {
            b"return" => K(CONTROL),
            b"reified" => K(MODIFIER),
            _ => Other,
        },
        b's' => match w {
            b"set" => Set,
            b"super" => L,
            b"sealed" | b"suspend" => K(MODIFIER),
            _ => Other,
        },
        b't' => match w {
            b"this" | b"true" => L,
            b"try" | b"throw" => K(CATCH),
            b"tailrec" | b"typealias" | b"typeof" => K(MODIFIER),
            _ => Other,
        },
        b'v' => match w {
            b"val" | b"var" => Binding,
            b"vararg" => K(MODIFIER),
            _ => Other,
        },
        b'w' => match w {
            b"when" | b"while" | b"where" => K(CONTROL),
            _ => Other,
        },
        b'y' => match w {
            b"yield" => K(MODIFIER),
            _ => Other,
        },
        b'A' => match w {
            b"Any" => T,
            b"Array" => C,
            _ => Other,
        },
        b'B' => match w {
            b"Boolean" | b"Byte" => T,
            b"BooleanArray" | b"ByteArray" => A,
            _ => Other,
        },
        b'C' => match w {
            b"Char" | b"CharSequence" => T,
            b"CharArray" => A,
            b"Collection" => C,
            _ => Other,
        },
        b'D' => match w {
            b"Double" => T,
            b"DoubleArray" => A,
            _ => Other,
        },
        b'F' => match w {
            b"Float" => T,
            b"FloatArray" => A,
            _ => Other,
        },
        b'I' => match w {
            b"Int" => T,
            b"IntArray" => A,
            _ => Other,
        },
        b'L' => match w {
            b"Long" => T,
            b"LongArray" => A,
            b"List" => C,
            _ => Other,
        },
        b'M' => match w {
            b"Map" | b"MutableList" | b"MutableMap" | b"MutableSet" => C,
            _ => Other,
        },
        b'N' => match w {
            b"Nothing" => T,
            _ => Other,
        },
        b'S' => match w {
            b"String" | b"Short" => T,
            b"ShortArray" => A,
            b"Set" | b"Sequence" => C,
            _ => Other,
        },
        b'U' => match w {
            b"Unit" => T,
            _ => Other,
        },
        _ => Other,
    }
}

/// The leaf `keywords` gives a word, if any.
fn keyword(name: Name) -> Option<u16> {
    match name {
        Name::Keyword(leaf) => Some(leaf),
        Name::Binding => Some(MODIFIER),
        _ => None,
    }
}

fn digits_end(b: &[u8], from: usize, digit: impl Fn(u8) -> bool) -> usize {
    next_of(b, from, |c| !digit(c))
}

/// bat's number at `p`, where `\b` holds:
/// `((0(x|X)\h*)|(([0-9]+\.?[0-9]*)|(\.[0-9]+))((e|E)(\+|-)?[0-9]+)?)([LlFf])?\b`, tried in
/// the order a backtracking engine tries it. Where it ends.
fn number(b: &[u8], p: usize) -> Option<usize> {
    // `([LlFf])?\b` at `e`
    let suffixed = |e: usize| {
        if matches!(b.get(e), Some(b'L' | b'l' | b'F' | b'f')) && boundary(b, e + 1) {
            return Some(e + 1);
        }
        boundary(b, e).then_some(e)
    };
    // the exponent, then the suffix, at `j`
    let after = |j: usize| {
        if matches!(b.get(j), Some(b'e' | b'E')) {
            let mut s = j + 1;
            if matches!(b.get(s), Some(b'+' | b'-')) {
                s += 1;
            }
            let d = digits_end(b, s, |c| c.is_ascii_digit());
            if let Some(e) = (s + 1..=d).rev().find_map(suffixed) {
                return Some(e);
            }
        }
        suffixed(j)
    };
    if b[p] == b'0' && matches!(b.get(p + 1), Some(b'x' | b'X')) {
        let h = digits_end(b, p + 2, |c| c.is_ascii_hexdigit());
        if let Some(e) = (p + 2..=h).rev().find_map(suffixed) {
            return Some(e);
        }
    }
    if b[p] == b'.' {
        let f = digits_end(b, p + 1, |c| c.is_ascii_digit());
        return (p + 2..=f).rev().find_map(after);
    }
    let d = digits_end(b, p, |c| c.is_ascii_digit());
    if d == p {
        return None;
    }
    if b.get(d) == Some(&b'.') {
        let f = digits_end(b, d + 1, |c| c.is_ascii_digit());
        if let Some(e) = (d + 1..=f).rev().find_map(after) {
            return Some(e);
        }
    }
    // a shorter run of digits ends inside it, where `\b` can't hold
    after(d)
}

impl Scan<'_, '_> {
    fn top(&self) -> u8 {
        self.stack[self.depth - 1]
    }

    /// Bytes `s..e` are `leaf`, after the text before them.
    fn put(&mut self, s: usize, e: usize, leaf: u16) {
        self.fill(s);
        if leaf != PLAIN {
            self.out.push_merged(s, e, leaf);
        }
        self.at = e;
    }

    /// The text up to `p` is the context's.
    fn fill(&mut self, p: usize) {
        if p > self.at {
            if self.text != PLAIN {
                self.out.push_merged(self.at, p, self.text);
            }
            self.at = p;
        }
    }

    fn refresh(&mut self) {
        let top = self.top();
        if top >= REGEX {
            return self.refresh_regex();
        }
        self.text = match top {
            RAW => RAW_TEXT,
            STRING => STRING_TEXT,
            CHAR | BACKTICK => CHAR_TEXT,
            COMMENT => BLOCK_COMMENT,
            _ => PLAIN,
        };
    }

    /// [`Scan::refresh`] in a regex: its string's text.
    #[cold]
    fn refresh_regex(&mut self) {
        self.text = if self.stack[self.depth - 2] == RAW {
            RAW_TEXT
        } else {
            STRING_TEXT
        };
    }

    fn push(&mut self, at: usize, c: u8) {
        self.fill(at);
        if self.depth == MAX {
            self.stack.copy_within(2.., 1);
            self.depth -= 1;
            self.exact = false;
        }
        self.stack[self.depth] = c;
        self.depth += 1;
        self.refresh();
    }

    fn pop(&mut self, at: usize) {
        self.fill(at);
        if self.depth > 1 {
            self.depth -= 1;
        }
        self.refresh();
    }

    /// A lookahead's pop at `p`: the context below goes on there.
    fn leave(&mut self, p: usize) -> usize {
        self.pop(p);
        p
    }

    /// A lookahead's push at `p`.
    fn enter(&mut self, p: usize, c: u8) -> usize {
        self.push(p, c);
        p
    }

    /// `n` bytes at `p` are `leaf`: where they end.
    fn token(&mut self, p: usize, n: usize, leaf: u16) -> usize {
        self.put(p, p + n, leaf);
        p + n
    }

    /// `n` bytes at `p`, `leaf`, that open `c`.
    fn open(&mut self, p: usize, n: usize, c: u8, leaf: u16) -> usize {
        self.push(p, c);
        self.token(p, n, leaf)
    }

    /// `n` bytes at `p`, `leaf`, that close the context.
    fn close(&mut self, p: usize, n: usize, leaf: u16) -> usize {
        self.put(p, p + n, leaf);
        self.pop(p + n);
        p + n
    }

    /// Whether `w` is the whole word at `p`.
    fn is_word_at(&self, p: usize, w: &[u8]) -> bool {
        self.b[p..].starts_with(w) && starts_word(self.b, p) && ends_word(self.b, p + w.len())
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
            // a raw string has no escapes
            RAW => next_of(b, p + 1, |c| c == b'"' || c == b'$'),
            STRING => next_of(b, p + 1, |c| matches!(c, b'"' | b'$' | b'\\')),
            CHAR => next_of(b, p + 1, |c| c == b'\'' || c == b'\\'),
            BACKTICK => next_of(b, p + 1, |c| c == b'`'),
            COMMENT => next_of(b, p + 1, |c| c == b'*'),
            top => {
                let c = b[p];
                if is_space(c) {
                    skip_spaces(b, p)
                } else if is_word(b, p) {
                    let e = word_end(b, p);
                    // `typedefs`' `(?=\s*(?:type))` matches inside a word too
                    if matches!(top, MAIN | BRACE | FN_BODY)
                        && let Some(i) = b[p + 1..e].windows(4).position(|w| w == b"type")
                    {
                        return p + 1 + i;
                    }
                    e
                } else {
                    p + char_len(c)
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
        match top {
            MAIN => {
                if p == 0
                    && let Some(e) = self.header()
                {
                    return Some(e);
                }
                self.statements(p, c)
            }
            BRACE => match c {
                b'}' => Some(self.close(p, 1, PLAIN)),
                _ => self.statements(p, c),
            },
            FN_BODY => match c {
                b'}' => Some(self.leave(p)),
                _ => self.statements(p, c),
            },
            PAREN => match c {
                b')' => Some(self.close(p, 1, PLAIN)),
                _ => self.expressions(p, c),
            },
            EXPR_BRACE => match c {
                b'}' => Some(self.close(p, 1, PLAIN)),
                _ => self.expressions(p, c),
            },
            FN_EXPR => self.expressions(p, c),
            ACCESSOR_EXPR => {
                if c == b's' && self.is_word_at(p, b"set") {
                    return Some(self.leave(p));
                }
                self.expressions(p, c)
            }
            TYPEDEF => {
                if c == b't' && self.is_word_at(p, b"type") {
                    return Some(self.token(p, 4, KEYWORD));
                }
                if c == b'<' {
                    return Some(self.open(p, 1, GENERICS, PLAIN));
                }
                self.expressions(p, c)
            }
            PARAM_DEFAULT => match c {
                b',' | b')' => Some(self.leave(p)),
                _ => self.expressions(p, c),
            },
            ANGLE => match c {
                b'>' => Some(self.close(p, 1, PLAIN)),
                _ => self.types(p, c).or_else(|| self.keywords(p, c)),
            },
            TYPE_PAREN => match c {
                b')' => Some(self.close(p, 1, PLAIN)),
                _ => self.types(p, c),
            },
            PARAM_TYPE => match c {
                b',' | b')' | b'=' => Some(self.leave(p)),
                _ => self.types(p, c),
            },
            BOUND => match c {
                b',' | b'>' => Some(self.leave(p)),
                _ => self.types(p, c),
            },
            FN_RETURN => match c {
                b'{' | b'=' => Some(self.leave(p)),
                _ => self.types(p, c),
            },
            GENERICS => match c {
                b'>' => Some(self.close(p, 1, PLAIN)),
                b':' => Some(self.open(p, 1, BOUND, DECLARATION)),
                _ => self.keywords(p, c).or_else(|| self.word(p, GENERIC)),
            },
            PARAMS => match c {
                b')' => Some(self.close(p, 1, PLAIN)),
                b':' => Some(self.open(p, 1, PARAM_TYPE, DECLARATION)),
                b'=' => Some(self.open(p, 1, PARAM_DEFAULT, DECLARATION)),
                _ => self.keywords(p, c).or_else(|| self.word(p, PARAMETER)),
            },
            CLASS => match c {
                b'}' => Some(self.leave(p)),
                b'<' => Some(self.open(p, 1, GENERICS, PLAIN)),
                b'(' => Some(self.open(p, 1, PARAMS, PLAIN)),
                b':' => Some(self.open(p, 1, SUPERS, DECLARATION)),
                b'{' => Some(self.open(p, 1, BRACE, PLAIN)),
                _ => self.comment(p, c).or_else(|| self.class_keyword(p)),
            },
            CLASS_NAME => match c {
                b'<' | b'{' | b'(' | b':' => Some(self.leave(p)),
                _ => self.comment(p, c).or_else(|| self.word(p, CLASS_LEAF)),
            },
            SUPERS => match c {
                b'{' => Some(self.leave(p)),
                b'(' => Some(self.open(p, 1, PAREN, PLAIN)),
                _ => self.word(p, INHERITED),
            },
            FUNCTION => match c {
                b'}' => Some(self.leave(p)),
                b'(' => Some(self.open(p, 1, PARAMS, PLAIN)),
                b':' => Some(self.open(p, 1, FN_RETURN, DECLARATION)),
                b'{' => Some(self.open(p, 1, FN_BODY, PLAIN)),
                b'=' => Some(self.open(p, 1, FN_EXPR, ASSIGNMENT)),
                b'f' if self.is_word_at(p, b"fun") => Some(self.open(p, 3, FN_NAME, KEYWORD)),
                _ => None,
            },
            FN_NAME => match c {
                b'(' => Some(self.leave(p)),
                b'<' => Some(self.open(p, 1, GENERICS, PLAIN)),
                _ => self.name(p, FUNCTION_LEAF),
            },
            GETTER | SETTER => match c {
                b'}' => Some(self.close(p, 1, PLAIN)),
                b's' if top == GETTER && self.is_word_at(p, b"set") => Some(self.leave(p)),
                b'g' if top == SETTER && self.is_word_at(p, b"get") => Some(self.leave(p)),
                b'(' if top == SETTER => Some(self.open(p, 1, PARAMS, PLAIN)),
                b'=' => Some(self.open(p, 1, ACCESSOR_EXPR, ASSIGNMENT)),
                b'{' => Some(self.open(p, 1, EXPR_BRACE, PLAIN)),
                _ => None,
            },
            VARIABLE | VAR_NAME => match c {
                b':' | b'=' => Some(self.leave(p)),
                b'b' if self.is_word_at(p, b"by") => Some(self.leave(p)),
                b'<' if top == VAR_NAME => Some(self.open(p, 1, GENERICS, PLAIN)),
                b'v' if top == VARIABLE
                    && (self.is_word_at(p, b"val") || self.is_word_at(p, b"var")) =>
                {
                    Some(self.open(p, 3, VAR_NAME, KEYWORD))
                }
                _ if top == VAR_NAME => self.name(p, VARIABLE_LEAF),
                _ => None,
            },
            RAW => match c {
                b'"' if raw_end(b, p) => Some(self.string_end(p, 3, RAW_TEXT)),
                // a template; no escape, which Kotlin's raw strings don't have (bat's grammar has
                // `\\.`, so `\"` before the end took one of its quotes)
                b'$' => self.in_string(p, c, RAW_TEXT),
                _ => None,
            },
            STRING => match c {
                b'"' => Some(self.string_end(p, 1, STRING_TEXT)),
                _ => self.in_string(p, c, STRING_TEXT),
            },
            CHAR => match c {
                b'\'' => Some(self.close(p, 1, CHAR_TEXT + END)),
                b'\\' => self.escape(p, CHAR_TEXT),
                _ => None,
            },
            BACKTICK => (c == b'`').then(|| self.close(p, 1, CHAR_TEXT + END)),
            REGEX.. => self.in_regex(p, c, top),
            _ => (c == b'*' && b.get(p + 1) == Some(&b'/')).then(|| self.close(p, 2, BLOCK_MARK)),
        }
    }

    /// At the line's end, after the last match: the patterns that match the `\n`, each a pop.
    fn eol(&mut self, top: u8) -> Option<usize> {
        match top {
            CLASS | CLASS_NAME | SUPERS | FUNCTION | FN_RETURN | FN_EXPR | GETTER | SETTER
            | ACCESSOR_EXPR | TYPEDEF | VARIABLE | VAR_NAME => Some(self.leave(self.b.len())),
            _ => None,
        }
    }

    /// `package` or `import` at the line's start, `^\s*`.
    fn header(&mut self) -> Option<usize> {
        let b = self.b;
        let q = skip_spaces(b, 0);
        // `^\s*(package)\b(?:\s*([^ ;$]+)\s*)?`
        if self.is_word_at(q, b"package") {
            self.put(q, q + 7, KEYWORD);
            let s = skip_spaces(b, q + 7);
            let e = next_of(b, s, |c| matches!(c, b' ' | b';' | b'$'));
            if e == s {
                return Some(q + 7);
            }
            self.put(s, e, PACKAGE);
            return Some(skip_spaces(b, e));
        }
        // `^\s*(import)\s+[^ $]+\s+(as)?`: the line's `\n` is a space
        if !b[q..].starts_with(b"import") || !b.get(q + 6).is_some_and(|&c| is_space(c)) {
            return None;
        }
        let w = skip_spaces(b, q + 6);
        if w == b.len() || b[w] == b'$' {
            return None;
        }
        let r = next_of(b, w, |c| c == b' ' || c == b'$');
        let k = if r == b.len() || b[r] == b' ' {
            r
        } else {
            (w + 1..r).rev().find(|&k| is_space(b[k]))?
        };
        self.put(q, q + 6, KEYWORD);
        let s = skip_spaces(b, k);
        if b[s..].starts_with(b"as") {
            return Some(self.token(s, 2, KEYWORD));
        }
        Some(s)
    }

    /// `statements`, at `p`: their own patterns, then `expressions`.
    fn statements(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if is_word(b, p) {
            if !starts_word(b, p) {
                // inside a word, only `(?=\s*(?:type))`
                return b[p..].starts_with(b"type").then(|| self.enter(p, TYPEDEF));
            }
            let e = word_end(b, p);
            let name = lookup(&b[p..e]);
            if name == Name::Namespace {
                return Some(self.token(p, e - p, KEYWORD));
            }
            if b[p..].starts_with(b"type") {
                return Some(self.enter(p, TYPEDEF));
            }
            match name {
                // `(?<!::)(?=\b(?:companion|class|object|interface)\b)`
                Name::Class | Name::Companion if !b[..p].ends_with(b"::") => {
                    return Some(self.enter(p, CLASS));
                }
                Name::Fun => return Some(self.enter(p, FUNCTION)),
                Name::Binding => return Some(self.enter(p, VARIABLE)),
                // `\b(get)\b\s*\(\s*\)`
                Name::Get => {
                    let q = skip_spaces(b, e);
                    if b.get(q) == Some(&b'(') {
                        let r = skip_spaces(b, q + 1);
                        if b.get(r) == Some(&b')') {
                            self.put(p, e, FUNCTION_LEAF);
                            return Some(self.open(r + 1, 0, GETTER, PLAIN));
                        }
                    }
                }
                // `\b(set)\b\s*(?=\()`
                Name::Set => {
                    let q = skip_spaces(b, e);
                    if b.get(q) == Some(&b'(') {
                        self.put(p, e, FUNCTION_LEAF);
                        return Some(self.open(q, 0, SETTER, PLAIN));
                    }
                }
                _ => {}
            }
            return self.expression_word(p, e, name);
        }
        if is_space(c) {
            // `(?=\s*(?:type))`, `(?=\s*\b(?:fun)\b)`, `(?=\s*\b(?:var|val)\b)`
            let q = skip_spaces(b, p);
            if b[q..].starts_with(b"type") {
                return Some(self.enter(p, TYPEDEF));
            }
            match lookup(&b[q..word_end(b, q)]) {
                Name::Fun => return Some(self.enter(p, FUNCTION)),
                Name::Binding => return Some(self.enter(p, VARIABLE)),
                _ => {}
            }
        }
        // `namespaces`' `\{` comes first, and opens what `types`' does
        self.expressions(p, c)
    }

    /// `expressions`, at `p`: `(`, then `types`, `strings`, `constants`, `comments` and
    /// `keywords`.
    fn expressions(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        match c {
            b'(' => Some(self.open(p, 1, PAREN, PLAIN)),
            b'{' => Some(self.open(p, 1, BRACE, PLAIN)),
            b'-' if b.get(p + 1) == Some(&b'>') => Some(self.token(p, 2, DECLARATION)),
            b'"' if b[p..].starts_with(b"\"\"\"") => Some(self.string(p, 3, RAW, RAW_TEXT)),
            b'"' => Some(self.string(p, 1, STRING, STRING_TEXT)),
            b'\'' => Some(self.open(p, 1, CHAR, CHAR_TEXT + BEGIN)),
            b'`' => Some(self.open(p, 1, BACKTICK, CHAR_TEXT + BEGIN)),
            // `\b\.[0-9]+…`, after a word
            b'.' if p > 0 && is_word(b, p - 1) => match number(b, p) {
                Some(e) => Some(self.token(p, e - p, NUMBER)),
                None => Some(self.token(p, 1, DOT)),
            },
            b'/' => self.comment(p, c).or_else(|| self.keywords(p, c)),
            _ if is_word(b, p) => {
                if !starts_word(b, p) {
                    return None;
                }
                let e = word_end(b, p);
                self.expression_word(p, e, lookup(&b[p..e]))
            }
            _ if is_space(c) => self.comment(p, c),
            _ => self.keywords(p, c),
        }
    }

    /// The word `b[p..e]` in `expressions`: `types`, `constants`, `keywords`.
    fn expression_word(&mut self, p: usize, e: usize, name: Name) -> Option<usize> {
        if let Some(r) = self.type_word(p, e, name) {
            return Some(r);
        }
        let b = self.b;
        let leaf = if name == Name::Language {
            LANGUAGE
        } else if b[p].is_ascii_digit()
            && let Some(n) = number(b, p)
        {
            return Some(self.token(p, n - p, NUMBER));
        } else if is_constant(&b[p..e]) {
            CONSTANT
        } else {
            keyword(name)?
        };
        Some(self.token(p, e - p, leaf))
    }

    /// `types`, at `p`.
    fn types(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        match c {
            b'{' => Some(self.open(p, 1, BRACE, PLAIN)),
            b'(' => Some(self.open(p, 1, TYPE_PAREN, PLAIN)),
            b'-' if b.get(p + 1) == Some(&b'>') => Some(self.token(p, 2, DECLARATION)),
            _ if is_word(b, p) && starts_word(b, p) => {
                let e = word_end(b, p);
                self.type_word(p, e, lookup(&b[p..e]))
            }
            _ => None,
        }
    }

    /// The word `b[p..e]` among `types`' patterns.
    fn type_word(&mut self, p: usize, e: usize, name: Name) -> Option<usize> {
        let b = self.b;
        match name {
            Name::Builtin => return Some(self.token(p, e - p, BUILTIN)),
            Name::Array => return Some(self.token(p, e - p, ARRAY)),
            _ => {}
        }
        if b.get(e) != Some(&b'<') {
            return None;
        }
        // `\b(Array|…)<\b`, else `\w+<`
        if name == Name::Collection && is_word(b, e + 1) {
            self.put(p, e, COLLECTION);
        }
        Some(self.open(e, 1, ANGLE, PLAIN))
    }

    /// `keywords`, at `p`.
    fn keywords(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        let next = b.get(p + 1).copied();
        let (n, leaf) = match c {
            b'!' => {
                // `\b(…|!in|!is|…)\b`: `\b` before `!` holds after a word
                if p > 0
                    && is_word(b, p - 1)
                    && (b[p + 1..].starts_with(b"in") || b[p + 1..].starts_with(b"is"))
                    && ends_word(b, p + 3)
                {
                    (3, OPERATOR)
                } else if next == Some(b'=') {
                    (2, COMPARISON)
                } else {
                    (1, LOGICAL)
                }
            }
            b'=' if next == Some(b'=') => (2, COMPARISON),
            b'=' => (1, ASSIGNMENT),
            b'<' | b'>' if next == Some(b'=') => (2, COMPARISON),
            b'<' | b'>' => (1, COMPARISON),
            b':' if next == Some(b':') => (2, OPERATOR),
            b':' => (1, DECLARATION),
            b'?' if next == Some(b'.') => (2, SAFENAV),
            b'?' if next == Some(b':') => (2, ELVIS),
            b'.' => (1, DOT),
            b'-' | b'+' if next == Some(c) => (2, INCREMENT),
            b'-' | b'+' | b'*' | b'/' if next == Some(b'=') => (2, ARITHMETIC_ASSIGN),
            b'-' | b'+' | b'*' | b'/' | b'%' => (1, ARITHMETIC),
            b'&' if next == Some(b'&') => (2, LOGICAL),
            b'|' if next == Some(b'|') => (2, LOGICAL),
            b';' => (1, TERMINATOR),
            _ if is_word(b, p) && starts_word(b, p) => {
                let e = word_end(b, p);
                (e - p, keyword(lookup(&b[p..e]))?)
            }
            _ => return None,
        };
        Some(self.token(p, n, leaf))
    }

    /// `comments`, at `p`: `/*`, or `\s*((//).*$)`.
    fn comment(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if c == b'/' && b.get(p + 1) == Some(&b'*') {
            return Some(self.open(p, 2, COMMENT, BLOCK_MARK));
        }
        let q = if is_space(c) { skip_spaces(b, p) } else { p };
        if !b[q..].starts_with(b"//") {
            return None;
        }
        self.put(q, q + 2, LINE_MARK);
        self.put(q + 2, b.len(), LINE_COMMENT);
        Some(b.len())
    }

    /// `\w+` at `p`, as `leaf`.
    fn word(&mut self, p: usize, leaf: u16) -> Option<usize> {
        is_word(self.b, p).then(|| {
            let e = word_end(self.b, p);
            self.token(p, e - p, leaf)
        })
    }

    /// A class header's keyword at `p`: `\b(companion\s*)?(class|object|interface)\b`.
    fn class_keyword(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if !is_word(b, p) || !starts_word(b, p) {
            return None;
        }
        let kind = |s: usize| {
            [&b"class"[..], b"object", b"interface"]
                .into_iter()
                .find(|w| b[s..].starts_with(w) && ends_word(b, s + w.len()))
                .map(|w| s + w.len())
        };
        let e = b[p..]
            .starts_with(b"companion")
            .then(|| kind(skip_spaces(b, p + 9)))
            .flatten()
            .or_else(|| kind(p))?;
        self.put(p, e, MODIFIER);
        Some(self.open(e, 0, CLASS_NAME, PLAIN))
    }

    /// A function's or a variable's name at `p`, as `leaf`: `([\.<\?>\w]+\.)?(\w+)`, its
    /// dotted prefix plain.
    fn name(&mut self, p: usize, leaf: u16) -> Option<usize> {
        let (s, e) = self.dotted(p)?;
        self.put(p, s, PLAIN);
        Some(self.token(s, e - s, leaf))
    }

    /// `([\.<\?>\w]+\.)?(\w+)` at `p`: where its `\w+` starts and ends. The prefix takes the
    /// last `.` before a word in the run of the bytes it takes.
    fn dotted(&mut self, p: usize) -> Option<(usize, usize)> {
        let b = self.b;
        if p >= self.nodot {
            let mut r = p;
            while is_word(b, r) || matches!(b.get(r), Some(b'.' | b'<' | b'?' | b'>')) {
                r += char_len(b[r]);
            }
            if let Some(k) = (p + 1..r)
                .rev()
                .find(|&k| b[k] == b'.' && is_word(b, k + 1))
            {
                return Some((k + 1, word_end(b, k + 1)));
            }
            // and none from anywhere later in the run
            self.nodot = r;
        }
        is_word(b, p).then(|| (p, word_end(b, p)))
    }

    /// A string, its quote `n` bytes at `p`, that opens `c`, with a regex in it where it is
    /// one (phase 4): where the scan goes on.
    // out of line, so that `expressions` keeps its words' paths inlined
    #[inline(never)]
    fn string(&mut self, p: usize, n: usize, c: u8, text: u16) -> usize {
        let e = self.open(p, n, c, text + BEGIN);
        if regex_string(self.b, p) {
            self.push(e, REGEX);
        } else {
            self.opened = Some((e, self.out.len()));
        }
        e
    }

    /// A string's end, its quote `n` bytes at `p`. Right before `.toRegex(` or `.toPattern(`, a
    /// string opened on the line is a regex: its spans are dropped and it is lexed again as one
    /// (rare, so a string's open costs no lookahead).
    fn string_end(&mut self, p: usize, n: usize, text: u16) -> usize {
        let rest = &self.b[p + n..];
        if rest.first() == Some(&b'.')
            && (rest.starts_with(b".toRegex(") || rest.starts_with(b".toPattern("))
            && let Some((s, len)) = self.opened.take()
        {
            return self.relex(s, len);
        }
        self.close(p, n, text + END)
    }

    /// [`Scan::string_end`]'s regex: the string from `s`, its spans from the `len`th, again.
    #[cold]
    #[inline(never)]
    fn relex(&mut self, s: usize, len: usize) -> usize {
        self.out.truncate(len);
        self.at = s;
        self.push(s, REGEX);
        s
    }

    /// In a regex in a string, at `p`: the string's patterns first (its end, a template, an
    /// escape but `\\`, which is the regex's), else the regex's to the next place they can
    /// start.
    // rare, and kept out of `at`'s inlined body
    #[cold]
    #[inline(never)]
    fn in_regex(&mut self, p: usize, c: u8, top: u8) -> Option<usize> {
        let raw = self.stack[self.depth - 2] == RAW;
        let text = if raw { RAW_TEXT } else { STRING_TEXT };
        let r = match c {
            b'"' if !raw => {
                self.pop(p);
                return Some(self.close(p, 1, text + END));
            }
            b'"' if raw_end(self.b, p) => {
                self.pop(p);
                return Some(self.close(p, 3, text + END));
            }
            b'$' => self.in_string(p, c, text),
            b'\\' if !raw && self.b.get(p + 1) != Some(&b'\\') => self.escape(p, text),
            _ => None,
        };
        if r.is_some() {
            return r;
        }
        let (q, st) = regex::embedded(
            self.b,
            p,
            top & regex::STATE,
            &RE_EMBEDS[usize::from(raw)],
            &mut self.at,
            self.out,
        )?;
        self.stack[self.depth - 1] = REGEX | st;
        Some(q)
    }

    /// A string's patterns but its end, at `p`: a template, then an escape.
    fn in_string(&mut self, p: usize, c: u8, text: u16) -> Option<usize> {
        let b = self.b;
        match c {
            b'$' => {
                let e = template(b, p)?;
                Some(self.token(p, e - p, text + TEMPLATE))
            }
            b'\\' => self.escape(p, text),
            _ => None,
        }
    }

    /// `\\.` at `p`.
    fn escape(&mut self, p: usize, text: u16) -> Option<usize> {
        let &d = self.b.get(p + 1)?;
        let n = match d {
            0xf0.. => 4,
            0xe0.. => 3,
            0xc0.. => 2,
            _ => 1,
        };
        Some(self.token(p, 1 + n, text + ESCAPE))
    }
}

#[cfg(test)]
mod tests {
    use crate::Lang;
    use crate::tests::marked;

    fn of(lines: &[&str]) -> Vec<(String, String)> {
        marked(Lang::Kotlin, lines)
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
    fn declarations() {
        let l = [
            "package sift.core.api",
            "import kotlin.reflect.KClass as K",
            "data class Foo<T : Any>(val x: Int, y: List<String> = listOf()) : Bar(x), Baz {",
            "    fun <R> List<T>.map(f: (T) -> R): List<R> = TODO()",
            "    val name: String get() = \"x\"",
            "}",
        ];
        has(&l, "package", "keyword.other.kotlin");
        has(&l, "sift.core.api", "entity.name.package.kotlin");
        has(&l, "import", "keyword.other.kotlin");
        has(&l, "as", "keyword.other.kotlin");
        has(&l, "data", "storage.modifier.kotlin");
        has(&l, "class", "storage.modifier.kotlin");
        has(&l, "Foo", "entity.name.type.class.kotlin");
        has(&l, "T", "storage.type.generic.kotlin");
        has(&l, "Any", "storage.type.buildin.kotlin");
        has(&l, "val", "storage.modifier.kotlin");
        has(&l, "x", "variable.parameter.function.kotlin");
        has(&l, "List", "storage.type.buildin.collection.kotlin");
        has(&l, "=", "keyword.operator.declaration.kotlin");
        has(&l, "Bar", "entity.other.inherited-class.kotlin");
        has(&l, "fun", "keyword.other.kotlin");
        has(&l, "map", "entity.name.function.kotlin");
        has(&l, "f", "variable.parameter.function.kotlin");
        has(&l, "->", "keyword.operator.declaration.kotlin");
        has(&l, "name", "entity.name.variable.kotlin");
        has(&l, "get", "entity.name.function.kotlin");
    }

    #[test]
    fn expressions_strings_and_comments() {
        let l = [
            "val s = \"a $x ${y.z} \\n\" + \"\"\"raw $w",
            "still\"\"\" // done",
            "if (a?.b ?: 0x1FL >= 1.5e3f && c !is D) return null /* block",
            "end */ x++; y += 2; z::class; MAX_VALUE; it.toString()",
        ];
        has(
            &l,
            "$x",
            "string.quoted.double.kotlin variable.parameter.template.kotlin",
        );
        has(
            &l,
            "${y.z}",
            "string.quoted.double.kotlin variable.parameter.template.kotlin",
        );
        has(
            &l,
            "\\n",
            "string.quoted.double.kotlin constant.character.escape.kotlin",
        );
        has(&l, "still", "string.quoted.third.kotlin");
        has(
            &l,
            "//",
            "comment.line.double-slash.kotlin punctuation.definition.comment.kotlin",
        );
        has(&l, "if", "keyword.control.kotlin");
        has(&l, "?.", "keyword.operator.safenav.kotlin");
        has(&l, "?:", "keyword.operator.elvis.kotlin");
        has(&l, "0x1FL", "constant.numeric.kotlin");
        has(&l, "1.5e3f", "constant.numeric.kotlin");
        // `\b` before `!` holds only after a word: here `!` and `is`
        has(&l, "!", "keyword.operator.logical.kotlin");
        has(&l, "is", "keyword.operator.kotlin");
        has(&l, "null", "constant.language.kotlin");
        has(&l, "end ", "comment.block.kotlin");
        has(&l, "++", "keyword.operator.increment-decrement.kotlin");
        has(&l, "+=", "keyword.operator.arithmetic.assign.kotlin");
        has(&l, "::", "keyword.operator.kotlin");
        has(&l, "MAX_VALUE", "constant.other.kotlin");
        has(&l, ".", "keyword.operator.dot.kotlin");
    }

    #[test]
    fn the_grammars_odd_corners() {
        // `type` inside a word opens a typedef, whose `<` opens generics
        has(
            &["x = isPrototype && a < b"],
            "b",
            "storage.type.generic.kotlin",
        );
        // a word before `<` opens a type's arguments, where numbers aren't
        assert!(!of(&["if (i<n) 1"]).iter().any(|(t, _)| t == "1"));
        // `1.` before a word is a number
        has(&["1.toString()"], "1.", "constant.numeric.kotlin");
        // every word of a header before `(` names the class
        has(
            &["class Foo private constructor(x: Int)"],
            "constructor",
            "entity.name.type.class.kotlin",
        );
    }

    #[test]
    fn regexes_in_strings() {
        const RE: &str = "string.quoted.double.kotlin";
        const RAW: &str = "string.quoted.third.kotlin";
        let l = [
            r#"val a = Regex("^(?<y>\\d{4})[a-z&&[^q]]$x\\$")"#,
            r#"val b = "\\w+\"\\".toRegex() + Pattern.compile("""(\d+)\Q*\E""")"#,
            r#"val c = "(\\d+".toPattern(); val d = Regex.fromLiteral("a+"); f("a+")"#,
            r#"val e = Regex(""""#,
            r#"  (a|b)+ ${x}$"""+"x*")"#,
            r#"val f = """q\"""".toRegex()"#,
        ];
        has(&l, "^", &format!("{RE} keyword.control.anchor.regexp"));
        has(&l, "y", &format!("{RE} entity.name.other.group.regexp"));
        has(
            &l,
            r"\\d",
            &format!("{RE} constant.other.character-class.escape.backslash.regexp"),
        );
        has(
            &l,
            "&&",
            &format!(
                "{RE} constant.other.character-class.set.regexp keyword.operator.intersection.regexp"
            ),
        );
        // a template, then an escaped `$`
        has(
            &l,
            "$x",
            &format!("{RE} variable.parameter.template.kotlin"),
        );
        has(
            &l,
            r"\\$",
            &format!("{RE} constant.character.escape.backslash.regexp"),
        );
        // the host's escape, then the regex's lone `\` before the string's end
        has(
            &l,
            r#"\""#,
            &format!("{RE} constant.character.escape.kotlin"),
        );
        has(
            &l,
            r"\\",
            &format!("{RE} constant.character.escape.backslash.regexp"),
        );
        has(&l, "+", &format!("{RE} keyword.operator.quantifier.regexp"));
        has(
            &l,
            r#"""""#,
            &format!("{RAW} punctuation.definition.string.end.kotlin"),
        );
        // raw: `\d` as it reads, `\Q…\E`
        has(
            &l,
            r"\d",
            &format!("{RAW} constant.other.character-class.escape.backslash.regexp"),
        );
        has(
            &l,
            r"\Q",
            &format!("{RAW} constant.character.escape.backslash.regexp"),
        );
        // `.toPattern()`
        has(
            &l,
            "(",
            &format!("{RE} punctuation.definition.group.begin.regexp"),
        );
        // a `"""` regex across lines, its template and anchor
        has(&l, "|", &format!("{RAW} keyword.operator.or.regexp"));
        has(
            &l,
            "${x}",
            &format!("{RAW} variable.parameter.template.kotlin"),
        );
        has(&l, "$", &format!("{RAW} keyword.control.anchor.regexp"));
        // not a regex: `Regex.fromLiteral`, any other call, a string after a regex
        let got = of(&[&l[2][36..]]);
        assert!(!got.iter().any(|(_, s)| s.contains("regexp")), "{got:?}");
        has(&l[3..], "x*", RE);
        // a run of quotes ends a raw regex at its last three, as Kotlin reads it
        has(
            &l[5..],
            r#"\""#,
            &format!("{RAW} constant.character.escape.backslash.regexp"),
        );
        has(
            &l[5..],
            r#"""""#,
            &format!("{RAW} punctuation.definition.string.end.kotlin"),
        );
    }

    #[test]
    fn raw_strings_as_kotlin_reads_them() {
        const RAW: &str = "string.quoted.third.kotlin";
        // no escapes; a run of quotes ends at its last three, and nothing after it is a string
        let l = [r#"val j = """{"a":"C:\new\"""" + x"#];
        has(&l, r#"{"a":"C:\new\""#, RAW);
        has(
            &l,
            r#"""""#,
            &format!("{RAW} punctuation.definition.string.end.kotlin"),
        );
        has(&l, "+", "keyword.operator.arithmetic.kotlin");
        // across lines, with a template
        let l = [r#"val s = """a $x \"#, r#"""""""#];
        has(
            &l,
            "$x",
            &format!("{RAW} variable.parameter.template.kotlin"),
        );
        // five quotes: two of text, then the end
        has(&l, r#""""#, RAW);
    }
}
