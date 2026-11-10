//! Terraform (HCL) on a scanner of ada's own (X-102), from a floor probe: one loop over a line's
//! bytes, a byte's class one load from a table, a name read once and looked up once in one table
//! (the constants, the types, the named values, the block keywords, the builtin functions, the
//! for-expression's and the template directives' words), the byte after it deciding a call, and
//! frames, four bits a level ([`Scan::frames`]), for what nests or runs across lines: a block's
//! body, an object, a tuple, a for-expression in either, a parenthesis, a call's arguments, a
//! string under an interpolation, `${…}` and `%{…}`, a here-document's body, a `/* */` comment.
//! Kinds are bat's leaves; no theme keys on a meta scope, so none are written.
//!
//! **Where the language and bat's grammar part, the language's reading** (each a departure):
//! - an attribute's `=` takes the blank after it, not any character (bat's `=[^=|>]` takes
//!   `a="x"`'s quote, and its string runs the wrong way for the rest of the file);
//! - a quoted string ends with its line, as HCL's does (bat's runs on);
//! - a block's header is a name, labels (quoted or bare) and `{`, whatever its labels hold (bat's
//!   lookahead takes only word bytes, blanks, `-` and quotes before the `{`, so Vault's `path
//!   "secret/*" {` and a module named `"a.b"` read as an object);
//! - a name is a name whatever it starts with (bat's `(?!null|false|true)` turns away
//!   `nullable`, `true_x`), and may start with `_`;
//! - `$${` and `%%{` are HCL's escapes, not a `$` before an interpolation;
//! - an object's key may be followed by `:`, as HCL allows, and a traversal's names are a key's
//!   (`aws.west = aws.west` in a module's `providers`; bat colors the last name alone);
//! - Terraform's builtin functions as they are (`try`, `can`, `one`, `startswith`, …; bat's list
//!   predates them), and `moved`, `import`, `removed`, `check` and `ephemeral` at the top level
//!   are block keywords as `resource` is;
//! - comments where HCL allows them (bat's parenthesis and interpolation take none).

use crate::emit::{Rules, Run, Runs, Sink, Spans};
use crate::heredoc::{Label, Lead, Tail};
use crate::prim::{char_len, find_any, is_name_start, is_word};
use crate::words::{Words, text_len};
use crate::{Out, State};

/// The leaves: a token's scopes.
static LEAVES: [&str; 53] = [
    "",
    "comment.line.terraform",
    "comment.line.terraform punctuation.definition.comment.terraform",
    "comment.block.terraform",
    "comment.block.terraform punctuation.definition.comment.terraform",
    "string.quoted.double.terraform",
    "string.quoted.double.terraform punctuation.definition.string.begin.terraform",
    "string.quoted.double.terraform punctuation.definition.string.end.terraform",
    "string.quoted.double.terraform constant.character.escape.terraform",
    "keyword.other.interpolation.begin.terraform",
    "keyword.other.interpolation.end.terraform",
    "keyword.operator.template.left.trim.terraform",
    "keyword.operator.template.right.trim.terraform",
    "string.unquoted.heredoc.terraform",
    "keyword.operator.heredoc.terraform",
    "keyword.control.heredoc.terraform",
    "variable.declaration.terraform variable.other.readwrite.terraform",
    "variable.declaration.terraform",
    "variable.declaration.terraform keyword.operator.assignment.terraform",
    "meta.mapping.key.terraform string.unquoted.terraform",
    "keyword.operator.terraform",
    "keyword.operator.arithmetic.terraform",
    "keyword.operator.logical.terraform",
    "keyword.operator.accessor.terraform",
    "keyword.operator.splat.terraform",
    "punctuation.section.brackets.end.terraform keyword.operator.splat.terraform",
    "keyword.operator.word.terraform",
    "variable.other.member.terraform",
    "variable.other.readwrite.terraform",
    "variable.function.terraform",
    "support.function.builtin.terraform",
    "support.constant.terraform",
    "constant.language.terraform",
    "storage.type.terraform",
    "entity.name.type.terraform",
    "entity.name.label.terraform",
    "constant.numeric.integer.terraform",
    "constant.numeric.float.terraform",
    "constant.numeric.float.terraform punctuation.separator.decimal.terraform",
    "constant.numeric.float.terraform punctuation.separator.exponent.terraform",
    "keyword.control.terraform",
    "keyword.control.conditional.terraform",
    "storage.type.function.terraform",
    "punctuation.separator.terraform",
    "punctuation.section.block.begin.terraform",
    "punctuation.section.block.end.terraform",
    "punctuation.section.braces.begin.terraform",
    "punctuation.section.braces.end.terraform",
    "punctuation.section.parens.begin.terraform",
    "punctuation.section.parens.end.terraform",
    "punctuation.section.brackets.begin.terraform",
    "punctuation.section.brackets.end.terraform",
    "string.unquoted.heredoc.terraform constant.character.escape.terraform",
];

const COMMENT: u16 = 1;
const COMMENT_MARK: u16 = 2;
const BLOCK_COMMENT: u16 = 3;
const BLOCK_MARK: u16 = 4;
const STRING: u16 = 5;
const STRING_BEGIN: u16 = 6;
const STRING_END: u16 = 7;
const ESCAPE: u16 = 8;
const INTERP_BEGIN: u16 = 9;
const INTERP_END: u16 = 10;
const TRIM_LEFT: u16 = 11;
const TRIM_RIGHT: u16 = 12;
const HEREDOC: u16 = 13;
const HEREDOC_OP: u16 = 14;
const HEREDOC_LABEL: u16 = 15;
const ATTRIBUTE: u16 = 16;
const DECLARATION: u16 = 17;
const ASSIGN: u16 = 18;
const KEY: u16 = 19;
const OPERATOR: u16 = 20;
const ARITHMETIC: u16 = 21;
const LOGICAL: u16 = 22;
const ACCESSOR: u16 = 23;
const SPLAT: u16 = 24;
const SPLAT_END: u16 = 25;
const WORD_OP: u16 = 26;
const MEMBER: u16 = 27;
const READWRITE: u16 = 28;
const FUNCTION: u16 = 29;
const BUILTIN: u16 = 30;
const SUPPORT: u16 = 31;
const CONSTANT: u16 = 32;
const STORAGE: u16 = 33;
const TYPE_NAME: u16 = 34;
const LABEL: u16 = 35;
const INTEGER: u16 = 36;
const FLOAT: u16 = 37;
const DECIMAL: u16 = 38;
const EXPONENT: u16 = 39;
const CONTROL: u16 = 40;
const CONDITIONAL: u16 = 41;
const ARROW: u16 = 42;
const SEPARATOR: u16 = 43;
const BLOCK_BEGIN: u16 = 44;
const BLOCK_END: u16 = 45;
const BRACES_BEGIN: u16 = 46;
const BRACES_END: u16 = 47;
const PARENS_BEGIN: u16 = 48;
const PARENS_END: u16 = 49;
const BRACKETS_BEGIN: u16 = 50;
const BRACKETS_END: u16 = 51;
const HEREDOC_ESCAPE: u16 = 52;

/// The scopes under `source.terraform` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    LEAVES.get(usize::from(kind)).map_or_else(Vec::new, |l| {
        l.split(' ').filter(|s| !s.is_empty()).collect()
    })
}

/// The floor probe's build (X-102's `sol`, `ADA_PARSE_X102_SOL` set at build time): the loop, the
/// lookups and the frames without the readings that need a statement's or an item's start (an
/// attribute's name, a block's header, an object's key, a for-expression).
const SOL: bool = option_env!("ADA_PARSE_X102_SOL").is_some();

// A byte's class ([`CLASS`]).
const C_OTHER: u8 = 0;
/// ` `, `\t`, `\r`, `\x0b`, `\x0c`: Oniguruma's `\s` on a line.
const C_BLANK: u8 = 1;
/// `[A-Za-z_]`.
const C_NAME: u8 = 2;
const C_DIGIT: u8 = 3;
const C_QUOTE: u8 = 4;
const C_HASH: u8 = 5;
const C_SLASH: u8 = 6;
const C_LBRACE: u8 = 7;
const C_RBRACE: u8 = 8;
const C_LBRACKET: u8 = 9;
const C_RBRACKET: u8 = 10;
const C_LPAREN: u8 = 11;
const C_RPAREN: u8 = 12;
const C_COMMA: u8 = 13;
const C_DOT: u8 = 14;
/// `= ! < > & | + - * % ? : ~`.
const C_OP: u8 = 15;
/// A byte past ASCII.
const C_WIDE: u8 = 16;

static CLASS: [u8; 256] = {
    let mut t = [C_WIDE; 256];
    let mut c = 0;
    while c < 128 {
        t[c] = match c as u8 {
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => C_BLANK,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => C_NAME,
            b'0'..=b'9' => C_DIGIT,
            b'"' => C_QUOTE,
            b'#' => C_HASH,
            b'/' => C_SLASH,
            b'{' => C_LBRACE,
            b'}' => C_RBRACE,
            b'[' => C_LBRACKET,
            b']' => C_RBRACKET,
            b'(' => C_LPAREN,
            b')' => C_RPAREN,
            b',' => C_COMMA,
            b'.' => C_DOT,
            b'=' | b'!' | b'<' | b'>' | b'&' | b'|' | b'+' | b'-' | b'*' | b'%' | b'?' | b':'
            | b'~' => C_OP,
            _ => C_OTHER,
        };
        c += 1;
    }
    t
};

/// A name's ASCII bytes after its first: `[A-Za-z0-9_-]`.
static NAME: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 128 {
        t[c] = (c as u8).is_ascii_alphanumeric() || c == b'_' as usize || c == b'-' as usize;
        c += 1;
    }
    t
};

#[inline(always)]
fn is_blank(c: u8) -> bool {
    CLASS[usize::from(c)] == C_BLANK
}

/// Where the blanks at `p` end.
#[inline(always)]
fn blanks(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && is_blank(b[p]) {
        p += 1;
    }
    p
}

/// Where the name that starts at `p` ends: `[\p{L}_][\p{L}\p{N}_-]*`, a `-` at its end not its
/// (bat's `\b` after it).
#[inline(always)]
fn name_end(b: &[u8], p: usize) -> usize {
    let n = b.len();
    let mut e = p + 1;
    loop {
        while e < n && NAME[usize::from(b[e])] {
            e += 1;
        }
        if e < n && b[e] >= 0x80 && is_word(b, e) {
            e += char_len(b[e]);
        } else {
            break;
        }
    }
    let e = e.min(n);
    if b[e - 1] == b'-' {
        let mut t = e;
        while b[t - 1] == b'-' {
            t -= 1;
        }
        return t;
    }
    e
}

// A word's flags, above its kind's byte ([`INFO`]).
/// A builtin function: [`BUILTIN`] before `(`.
const W_BUILTIN: u16 = 1 << 8;
/// A block keyword (`resource`, …): [`STORAGE`] in a header.
const W_BLOCK: u16 = 1 << 9;
/// A block keyword at the top level only (`moved`, `import`, …).
const W_BLOCK_TOP: u16 = 1 << 10;
/// `for`: a for-expression at a tuple's or an object's start.
const W_FOR: u16 = 1 << 11;
/// `in` and `if` in a for-expression ([`WORD_OP`], [`CONDITIONAL`]).
const W_IN: u16 = 1 << 12;
const W_IF: u16 = 1 << 13;
/// A template directive's word: [`CONTROL`] in `${…}` and `%{…}`.
const W_DIRECTIVE: u16 = 1 << 14;

const WORD_LIST: &[(&[u8], u16)] = &[
    (b"true", CONSTANT),
    (b"false", CONSTANT),
    (b"null", CONSTANT),
    (b"any", STORAGE),
    (b"string", STORAGE),
    (b"number", STORAGE),
    (b"bool", STORAGE),
    (b"var", SUPPORT),
    (b"local", SUPPORT),
    (b"path", SUPPORT),
    (b"module", SUPPORT | W_BLOCK),
    (b"data", SUPPORT | W_BLOCK),
    (b"terraform", SUPPORT | W_BLOCK),
    (b"resource", W_BLOCK),
    (b"provider", W_BLOCK),
    (b"variable", W_BLOCK),
    (b"output", W_BLOCK),
    (b"locals", W_BLOCK),
    (b"moved", W_BLOCK_TOP),
    (b"import", W_BLOCK_TOP),
    (b"removed", W_BLOCK_TOP),
    (b"check", W_BLOCK_TOP),
    (b"ephemeral", W_BLOCK_TOP),
    (b"for", W_FOR | W_DIRECTIVE),
    (b"in", W_IN | W_DIRECTIVE),
    (b"if", W_IF | W_DIRECTIVE),
    (b"else", W_DIRECTIVE),
    (b"endif", W_DIRECTIVE),
    (b"endfor", W_DIRECTIVE),
    // Terraform's builtin functions (1.10)
    (b"abs", W_BUILTIN),
    (b"ceil", W_BUILTIN),
    (b"floor", W_BUILTIN),
    (b"log", W_BUILTIN),
    (b"max", W_BUILTIN),
    (b"min", W_BUILTIN),
    (b"parseint", W_BUILTIN),
    (b"pow", W_BUILTIN),
    (b"signum", W_BUILTIN),
    (b"chomp", W_BUILTIN),
    (b"endswith", W_BUILTIN),
    (b"format", W_BUILTIN),
    (b"formatlist", W_BUILTIN),
    (b"indent", W_BUILTIN),
    (b"join", W_BUILTIN),
    (b"lower", W_BUILTIN),
    (b"regex", W_BUILTIN),
    (b"regexall", W_BUILTIN),
    (b"replace", W_BUILTIN),
    (b"split", W_BUILTIN),
    (b"startswith", W_BUILTIN),
    (b"strcontains", W_BUILTIN),
    (b"strrev", W_BUILTIN),
    (b"substr", W_BUILTIN),
    (b"templatestring", W_BUILTIN),
    (b"title", W_BUILTIN),
    (b"trim", W_BUILTIN),
    (b"trimprefix", W_BUILTIN),
    (b"trimspace", W_BUILTIN),
    (b"trimsuffix", W_BUILTIN),
    (b"upper", W_BUILTIN),
    (b"alltrue", W_BUILTIN),
    (b"anytrue", W_BUILTIN),
    (b"chunklist", W_BUILTIN),
    (b"coalesce", W_BUILTIN),
    (b"coalescelist", W_BUILTIN),
    (b"compact", W_BUILTIN),
    (b"concat", W_BUILTIN),
    (b"contains", W_BUILTIN),
    (b"distinct", W_BUILTIN),
    (b"element", W_BUILTIN),
    (b"flatten", W_BUILTIN),
    (b"index", W_BUILTIN),
    (b"keys", W_BUILTIN),
    (b"length", W_BUILTIN),
    (b"list", W_BUILTIN),
    (b"lookup", W_BUILTIN),
    (b"map", W_BUILTIN),
    (b"matchkeys", W_BUILTIN),
    (b"merge", W_BUILTIN),
    (b"one", W_BUILTIN),
    (b"range", W_BUILTIN),
    (b"reverse", W_BUILTIN),
    (b"setintersection", W_BUILTIN),
    (b"setproduct", W_BUILTIN),
    (b"setsubtract", W_BUILTIN),
    (b"setunion", W_BUILTIN),
    (b"slice", W_BUILTIN),
    (b"sort", W_BUILTIN),
    (b"sum", W_BUILTIN),
    (b"transpose", W_BUILTIN),
    (b"values", W_BUILTIN),
    (b"zipmap", W_BUILTIN),
    (b"base64decode", W_BUILTIN),
    (b"base64encode", W_BUILTIN),
    (b"base64gzip", W_BUILTIN),
    (b"csvdecode", W_BUILTIN),
    (b"jsondecode", W_BUILTIN),
    (b"jsonencode", W_BUILTIN),
    (b"textdecodebase64", W_BUILTIN),
    (b"textencodebase64", W_BUILTIN),
    (b"urlencode", W_BUILTIN),
    (b"yamldecode", W_BUILTIN),
    (b"yamlencode", W_BUILTIN),
    (b"abspath", W_BUILTIN),
    (b"dirname", W_BUILTIN),
    (b"pathexpand", W_BUILTIN),
    (b"basename", W_BUILTIN),
    (b"file", W_BUILTIN),
    (b"fileexists", W_BUILTIN),
    (b"fileset", W_BUILTIN),
    (b"filebase64", W_BUILTIN),
    (b"templatefile", W_BUILTIN),
    (b"formatdate", W_BUILTIN),
    (b"plantimestamp", W_BUILTIN),
    (b"timeadd", W_BUILTIN),
    (b"timecmp", W_BUILTIN),
    (b"timestamp", W_BUILTIN),
    (b"base64sha256", W_BUILTIN),
    (b"base64sha512", W_BUILTIN),
    (b"bcrypt", W_BUILTIN),
    (b"filebase64sha256", W_BUILTIN),
    (b"filebase64sha512", W_BUILTIN),
    (b"filemd5", W_BUILTIN),
    (b"filesha1", W_BUILTIN),
    (b"filesha256", W_BUILTIN),
    (b"filesha512", W_BUILTIN),
    (b"md5", W_BUILTIN),
    (b"rsadecrypt", W_BUILTIN),
    (b"sha1", W_BUILTIN),
    (b"sha256", W_BUILTIN),
    (b"sha512", W_BUILTIN),
    (b"uuid", W_BUILTIN),
    (b"uuidv5", W_BUILTIN),
    (b"cidrhost", W_BUILTIN),
    (b"cidrnetmask", W_BUILTIN),
    (b"cidrsubnet", W_BUILTIN),
    (b"cidrsubnets", W_BUILTIN),
    (b"can", W_BUILTIN),
    (b"ephemeralasnull", W_BUILTIN),
    (b"issensitive", W_BUILTIN),
    (b"nonsensitive", W_BUILTIN),
    (b"sensitive", W_BUILTIN),
    (b"tobool", W_BUILTIN),
    (b"tolist", W_BUILTIN),
    (b"tomap", W_BUILTIN),
    (b"tonumber", W_BUILTIN),
    (b"toset", W_BUILTIN),
    (b"tostring", W_BUILTIN),
    (b"try", W_BUILTIN),
    (b"type", W_BUILTIN),
];

const NW: usize = WORD_LIST.len();
static WORD_TEXT: [&[u8]; NW] = {
    let mut t: [&[u8]; NW] = [b""; NW];
    let mut i = 0;
    while i < NW {
        t[i] = WORD_LIST[i].0;
        i += 1;
    }
    t
};
static WORDS: Words<512, NW, { text_len(&WORD_TEXT) }> = Words::new(&WORD_TEXT);
/// What each word is by id: its kind in the low byte, its flags above.
static INFO: [u16; NW + 1] = {
    let mut t = [0u16; NW + 1];
    let mut i = 0;
    while i < NW {
        t[i + 1] = WORD_LIST[i].1;
        i += 1;
    }
    t
};

// The frames, four bits a level in [`Scan::frames`]; 0 is the file's top (a body), or a level
// lost past 32 levels.
const F_TOP: u8 = 0;
/// A block's body, `{ … }` after a header.
const F_BODY: u8 = 1;
/// An object, `{ … }` in an expression.
const F_OBJECT: u8 = 2;
/// An object's for-expression, `{ for … }`.
const F_OBJECT_FOR: u8 = 3;
/// A tuple, `[ … ]`.
const F_TUPLE: u8 = 4;
/// A tuple's for-expression, `[ for … ]`.
const F_TUPLE_FOR: u8 = 5;
/// A parenthesis, `( … )`, not a call's.
const F_PARENS: u8 = 6;
/// A call's arguments, `name( … )`.
const F_CALL: u8 = 7;
/// A quoted string: across a line's end only under an interpolation.
const F_STRING: u8 = 8;
/// `${ … }` or `%{ … }` in a string or a here-document.
const F_INTERP: u8 = 9;
/// A here-document's body, its label in [`Scan::label`].
const F_HEREDOC: u8 = 10;
/// A `/* … */` comment.
const F_COMMENT: u8 = 11;

/// Lex `b`, a line of Terraform without its `\n`, from `state`.
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

/// A block of Terraform on one scan opened from `state` ([`crate::Carry`]).
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
    /// What each open frame is ([`F_BODY`]…), four bits a level, the innermost lowest; past 32
    /// levels the outer ones are lost.
    frames: u128,
    /// How many frames are open, up to 255.
    depth: u8,
    /// The open here-document's [`Label`] bits, 0 for none: one at a time (HCL's opener ends its
    /// line, so a second can only open in an interpolation inside the first's body, where it is
    /// read as two `<`s).
    label: u32,
    /// At a statement's or an item's start: a line's start, a `{`, a `[`, an item's `,`. Not
    /// kept between lines.
    start: bool,
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
            label: 0,
            start: true,
            k,
            out,
        }
    }

    /// The scan's fields from `state`: the frames in its levels and the first two withs, the
    /// here-document's label in the next four.
    #[inline(always)]
    fn load(&mut self, state: &State) {
        let mut w = [0u8; 16];
        w[..14].copy_from_slice(&state.levels);
        w[14] = state.withs[0];
        w[15] = state.withs[1];
        self.frames = u128::from_le_bytes(w);
        self.depth = state.depth;
        let mut l = [0u8; 4];
        l.copy_from_slice(&state.withs[2..6]);
        self.label = u32::from_le_bytes(l);
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
        s.withs[2..6].copy_from_slice(&self.label.to_le_bytes());
        s.depth = self.depth;
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

    /// A write: one call on the run sink ([`Sink::merge_call`]).
    #[inline(always)]
    fn put(&mut self, s: usize, e: usize, kind: u16) {
        self.k.merge_call(&mut self.out, s, e, kind);
    }

    /// A write inlined, for the common tokens.
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

    #[inline(always)]
    fn set_top(&mut self, f: u8) {
        self.frames = self.frames & !15 | u128::from(f);
    }

    /// The line, from what the line before left: a here-document's body or a comment's rest
    /// first; the loop out of line where `SHARED` ([`Scan::code_shared`]).
    #[inline(always)]
    fn lex_line<const SHARED: bool>(&mut self) {
        self.start = true;
        let p = match self.top() {
            F_HEREDOC => self.heredoc_line(),
            F_COMMENT => self.comment_rest(0),
            _ => 0,
        };
        if SHARED {
            self.code_shared(p);
        } else {
            self.code(p);
        }
        // a quoted string ends with its line (one under an interpolation goes on)
        if self.top() == F_STRING {
            self.pop();
        }
    }

    /// [`Scan::code`] out of line: on the run sink, one copy that the line API and a block's
    /// lines share; on the span sink, which ada doesn't hold, inlined.
    #[inline(never)]
    fn code_shared(&mut self, p: usize) {
        self.code(p);
    }

    /// The line's code from `p`. A string, a here-document's text or a comment is read where it
    /// opens and runs to its close or the line's end, so the loop's frame is always code's.
    #[inline(always)]
    fn code(&mut self, mut p: usize) {
        let b = self.b;
        let n = b.len();
        while p < n {
            let c = b[p];
            p = match CLASS[usize::from(c)] {
                C_BLANK => blanks(b, p + 1),
                C_NAME => self.name(p, name_end(b, p)),
                C_DIGIT => self.number(p),
                C_QUOTE => {
                    self.put_hot(p, p + 1, STRING_BEGIN);
                    self.push(F_STRING);
                    self.string(p + 1)
                }
                C_HASH => self.line_comment(p, p + 1),
                C_SLASH => match b.get(p + 1) {
                    Some(b'/') => self.line_comment(p, p + 2),
                    Some(b'*') => self.block_comment(p),
                    _ => {
                        self.put(p, p + 1, ARITHMETIC);
                        p + 1
                    }
                },
                C_LBRACE => {
                    self.put_hot(p, p + 1, BRACES_BEGIN);
                    self.push(F_OBJECT);
                    self.start = true;
                    p + 1
                }
                C_RBRACE => self.rbrace(p),
                C_LBRACKET => {
                    self.put_hot(p, p + 1, BRACKETS_BEGIN);
                    self.push(F_TUPLE);
                    self.start = true;
                    p + 1
                }
                C_RBRACKET => {
                    if matches!(self.top(), F_TUPLE | F_TUPLE_FOR) {
                        self.put_hot(p, p + 1, BRACKETS_END);
                        self.pop();
                    }
                    p + 1
                }
                C_LPAREN => {
                    self.put_hot(p, p + 1, PARENS_BEGIN);
                    self.push(F_PARENS);
                    p + 1
                }
                C_RPAREN => {
                    if matches!(self.top(), F_PARENS | F_CALL) {
                        self.put_hot(p, p + 1, PARENS_END);
                        self.pop();
                    }
                    p + 1
                }
                C_COMMA => {
                    self.put_hot(p, p + 1, SEPARATOR);
                    if matches!(self.top(), F_OBJECT | F_TUPLE) {
                        self.start = true;
                    }
                    p + 1
                }
                C_DOT => self.dot(p),
                C_OP => self.op(p),
                C_WIDE => {
                    if is_name_start(b, p) {
                        self.name(p, name_end(b, p))
                    } else {
                        p + char_len(c)
                    }
                }
                _ => p + 1,
            };
        }
    }

    /// A name `b[p..e]`: by the frame it stands in and the byte after it.
    #[inline(always)]
    fn name(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let w = WORDS.value(&b[p..e], &INFO);
        let top = self.top();
        if !SOL && self.start {
            self.start = false;
            if let Some(r) = self.at_start(p, e, w, top) {
                return r;
            }
        }
        self.start = false;
        let kind = w & 0xff;
        let kind = match top {
            F_INTERP if w & W_DIRECTIVE != 0 => CONTROL,
            F_TUPLE_FOR | F_OBJECT_FOR if !SOL && w & W_IN != 0 => WORD_OP,
            F_TUPLE_FOR | F_OBJECT_FOR if !SOL && w & W_IF != 0 => CONDITIONAL,
            _ if kind != 0 => kind,
            _ if b.get(e) == Some(&b'(') => {
                let kind = if w & W_BUILTIN != 0 {
                    BUILTIN
                } else {
                    FUNCTION
                };
                self.put_hot(p, e, kind);
                self.put_hot(e, e + 1, PARENS_BEGIN);
                self.push(F_CALL);
                return e + 1;
            }
            F_PARENS | F_TUPLE_FOR | F_OBJECT_FOR => READWRITE,
            _ => return e,
        };
        self.put_hot(p, e, kind);
        e
    }

    /// A name `b[p..e]` at a statement's or an item's start: an attribute, a block's header, an
    /// object's key, a for-expression; `None` for an expression's name.
    #[inline(always)]
    fn at_start(&mut self, p: usize, e: usize, w: u16, top: u8) -> Option<usize> {
        let b = self.b;
        match top {
            F_TOP | F_BODY => {
                let q = blanks(b, e);
                if b.get(q) == Some(&b'=') && !matches!(b.get(q + 1), Some(b'=' | b'>')) {
                    return Some(self.attribute(p, e, q));
                }
                self.header(p, e, w, q)
            }
            F_OBJECT | F_TUPLE if w & W_FOR != 0 => {
                self.set_top(if top == F_OBJECT {
                    F_OBJECT_FOR
                } else {
                    F_TUPLE_FOR
                });
                self.put(p, e, CONTROL);
                Some(e)
            }
            F_OBJECT => {
                if b.get(e) == Some(&b'.') {
                    return self.traversal_key(p, e);
                }
                is_key(b, blanks(b, e)).then(|| {
                    self.put_hot(p, e, KEY);
                    e
                })
            }
            _ => None,
        }
    }

    /// A name `b[p..e]` at an object item's start with a `.` after it: a key where the names
    /// and dots run to its `=` or `:` (a module's `providers = { aws.west = aws.west }`), each
    /// name a key's, each dot an accessor.
    #[inline(never)]
    fn traversal_key(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        let mut r = e;
        while b.get(r) == Some(&b'.')
            && b.get(r + 1)
                .is_some_and(|&c| CLASS[usize::from(c)] == C_NAME || is_name_start(b, r + 1))
        {
            r = name_end(b, r + 1);
        }
        if !is_key(b, blanks(b, r)) {
            return None;
        }
        let mut s = p;
        while s < r {
            let t = name_end(b, s);
            self.put(s, t, KEY);
            if t < r {
                self.put(t, t + 1, ACCESSOR);
            }
            s = t + 1;
        }
        Some(r)
    }

    /// An attribute's name `b[p..e]`, its `=` at `q`: the name, the blanks, the `=` with the
    /// blank after it, and the blanks after that, bat's declaration.
    #[inline(always)]
    fn attribute(&mut self, p: usize, e: usize, q: usize) -> usize {
        let b = self.b;
        self.put_hot(p, e, ATTRIBUTE);
        self.put_hot(e, q, DECLARATION);
        let a = if b.get(q + 1).is_some_and(|&c| is_blank(c)) {
            q + 2
        } else {
            q + 1
        };
        self.put_hot(q, a, ASSIGN);
        let r = blanks(b, a);
        self.put_hot(a, r, DECLARATION);
        r
    }

    /// A block's header from its type `b[p..e]`, the blanks after it to `q`: labels, quoted or
    /// bare, then `{`; `None` where the line holds something else before a `{`.
    #[inline(never)]
    fn header(&mut self, p: usize, e: usize, w: u16, q: usize) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        // the labels read once to the `{`, written after
        let mut r = q;
        loop {
            match b.get(r) {
                Some(b'{') => break,
                Some(b'"') => {
                    let mut s = r + 1;
                    loop {
                        s = find_any(b, s, *b"\"\\");
                        if s >= n {
                            return None;
                        }
                        if b[s] == b'"' {
                            break;
                        }
                        s += 2;
                    }
                    r = blanks(b, s + 1);
                }
                Some(&c) if CLASS[usize::from(c)] == C_NAME || is_name_start(b, r) => {
                    r = blanks(b, name_end(b, r));
                }
                _ => return None,
            }
        }
        let keyword = w & W_BLOCK != 0 || (w & W_BLOCK_TOP != 0 && self.depth == 0);
        self.put(p, e, if keyword { STORAGE } else { TYPE_NAME });
        let mut s = q;
        while s < r {
            if b[s] == b'"' {
                let mut t = s + 1;
                loop {
                    t = find_any(b, t, *b"\"\\");
                    if b[t] == b'"' {
                        break;
                    }
                    t += 2;
                }
                self.put(s, s + 1, STRING_BEGIN);
                self.put(s + 1, t, STRING);
                self.put(t, t + 1, STRING_END);
                s = blanks(b, t + 1);
            } else {
                let t = name_end(b, s);
                self.put(s, t, LABEL);
                s = blanks(b, t);
            }
        }
        self.put_hot(r, r + 1, BLOCK_BEGIN);
        self.push(F_BODY);
        self.start = true;
        Some(r + 1)
    }

    /// A `}` at `p`: a body's, an object's, an interpolation's (whose string or here-document
    /// goes on), or none.
    #[inline(always)]
    fn rbrace(&mut self, p: usize) -> usize {
        self.start = false;
        match self.top() {
            F_BODY => {
                self.put_hot(p, p + 1, BLOCK_END);
                self.pop();
                p + 1
            }
            F_OBJECT | F_OBJECT_FOR => {
                self.put_hot(p, p + 1, BRACES_END);
                self.pop();
                p + 1
            }
            F_INTERP => {
                self.put_hot(p, p + 1, INTERP_END);
                self.pop();
                match self.top() {
                    F_STRING => self.string(p + 1),
                    F_HEREDOC => self.heredoc_text(p + 1),
                    _ => p + 1,
                }
            }
            _ => p + 1,
        }
    }

    /// A number at `p`: bat's integer, its float with a fraction or an exponent; digits that run
    /// into a name's bytes are none (bat's `\b`).
    #[inline(always)]
    fn number(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut e = p + 1;
        while e < n && b[e].is_ascii_digit() {
            e += 1;
        }
        let mut frac = 0;
        if e + 1 < n && b[e] == b'.' && b[e + 1].is_ascii_digit() {
            frac = e;
            e += 2;
            while e < n && b[e].is_ascii_digit() {
                e += 1;
            }
        }
        let mut exp = (0, 0);
        if e < n && matches!(b[e], b'e' | b'E') {
            let s = if matches!(b.get(e + 1), Some(b'+' | b'-')) {
                e + 2
            } else {
                e + 1
            };
            if s < n && b[s].is_ascii_digit() {
                exp = (e, s);
                e = s + 1;
                while e < n && b[e].is_ascii_digit() {
                    e += 1;
                }
            }
        }
        if e < n && is_word(b, e) {
            self.start = false;
            let mut t = e;
            while t < n && is_word(b, t) {
                t += char_len(b[t]);
            }
            return t.min(n);
        }
        self.start = false;
        if frac == 0 && exp.0 == 0 {
            self.put_hot(p, e, INTEGER);
            return e;
        }
        self.number_parts(p, e, frac, exp);
        e
    }

    /// A float's spans: its digits, its `.` and its exponent's mark.
    #[inline(never)]
    fn number_parts(&mut self, p: usize, e: usize, frac: usize, exp: (usize, usize)) {
        let mut s = p;
        if frac != 0 {
            self.put(s, frac, FLOAT);
            self.put(frac, frac + 1, DECIMAL);
            s = frac + 1;
        }
        if exp.0 != 0 {
            self.put(s, exp.0, FLOAT);
            self.put(exp.0, exp.1, EXPONENT);
            s = exp.1;
        }
        self.put(s, e, FLOAT);
    }

    /// A `.` at `p`: `...`, or an accessor and what it reaches (a name, an index, `*`).
    #[inline(always)]
    fn dot(&mut self, p: usize) -> usize {
        let b = self.b;
        self.start = false;
        if b.get(p + 1) == Some(&b'.') && b.get(p + 2) == Some(&b'.') {
            self.put(p, p + 3, OPERATOR);
            return p + 3;
        }
        self.put_hot(p, p + 1, ACCESSOR);
        let q = p + 1;
        match b.get(q) {
            Some(&c) if CLASS[usize::from(c)] == C_NAME => {
                let e = name_end(b, q);
                self.put_hot(q, e, MEMBER);
                e
            }
            Some(c) if c.is_ascii_digit() => {
                let mut e = q + 1;
                while e < b.len() && b[e].is_ascii_digit() {
                    e += 1;
                }
                self.put(q, e, INTEGER);
                e
            }
            Some(b'*') => {
                self.put(q, q + 1, SPLAT);
                q + 1
            }
            Some(&c) if c >= 0x80 && is_name_start(b, q) => {
                let e = name_end(b, q);
                self.put(q, e, MEMBER);
                e
            }
            _ => q,
        }
    }

    /// An operator's byte at `p`.
    #[inline(always)]
    fn op(&mut self, p: usize) -> usize {
        let b = self.b;
        self.start = false;
        let next = b.get(p + 1).copied().unwrap_or(0);
        let (e, kind) = match b[p] {
            b'=' => match next {
                b'=' => (p + 2, OPERATOR),
                b'>' if self.top() == F_OBJECT_FOR => (p + 2, ARROW),
                _ if self.top() == F_OBJECT => (p + 1, OPERATOR),
                _ => return p + 1,
            },
            b'!' => match next {
                b'=' => (p + 2, OPERATOR),
                _ => (p + 1, LOGICAL),
            },
            b'<' => match next {
                b'=' => (p + 2, OPERATOR),
                b'<' => return self.heredoc_open(p),
                _ => (p + 1, OPERATOR),
            },
            b'>' => match next {
                b'=' => (p + 2, OPERATOR),
                _ => (p + 1, OPERATOR),
            },
            b'&' if next == b'&' => (p + 2, LOGICAL),
            b'|' if next == b'|' => (p + 2, LOGICAL),
            b'+' | b'-' | b'%' => (p + 1, ARITHMETIC),
            b'*' => {
                if next == b']' && self.top() == F_TUPLE {
                    self.put(p, p + 1, SPLAT_END);
                    self.put(p + 1, p + 2, BRACKETS_END);
                    self.pop();
                    return p + 2;
                }
                (p + 1, ARITHMETIC)
            }
            b'?' | b':' => (p + 1, OPERATOR),
            b'~' => return self.trim(p),
            _ => return p + 1,
        };
        self.put_hot(p, e, kind);
        e
    }

    /// A `~` at `p`: in an interpolation, a strip marker with the blank before or after it.
    #[inline(never)]
    fn trim(&mut self, p: usize) -> usize {
        let b = self.b;
        if self.top() != F_INTERP {
            return p + 1;
        }
        if p > 0 && is_blank(b[p - 1]) {
            self.put(p - 1, p + 1, TRIM_RIGHT);
            p + 1
        } else if b.get(p + 1).is_some_and(|&c| is_blank(c)) {
            self.put(p, p + 2, TRIM_LEFT);
            p + 2
        } else {
            p + 1
        }
    }

    /// `<<` at `p`: a here-document's opener where a label ends the line (`<<-` too), and none
    /// is open; else two `<`s.
    #[inline(never)]
    fn heredoc_open(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let op = if b.get(p + 2) == Some(&b'-') {
            p + 3
        } else {
            p + 2
        };
        let s = blanks(b, op);
        let opens =
            self.label == 0 && s < n && (CLASS[usize::from(b[s])] == C_NAME || is_name_start(b, s));
        if opens {
            let e = name_end(b, s);
            if blanks(b, e) == n {
                self.put(p, op, HEREDOC_OP);
                self.put(s, e, HEREDOC_LABEL);
                self.label = Label::of(&b[s..e]).bits();
                self.push(F_HEREDOC);
                return n;
            }
        }
        self.put(p, p + 1, OPERATOR);
        p + 1
    }

    /// A here-document's line from its start: its closer, or its text.
    #[inline(never)]
    fn heredoc_line(&mut self) -> usize {
        let b = self.b;
        if Label::from_bits(self.label)
            .close(b, Lead::Blanks, Tail::Blanks)
            .is_some()
        {
            self.put(0, b.len(), HEREDOC_LABEL);
            self.pop();
            self.label = 0;
            return b.len();
        }
        self.heredoc_text(0)
    }

    /// A here-document's text from `p`: to the line's end, or an interpolation's opener.
    #[inline(always)]
    fn heredoc_text(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut s = p;
        loop {
            p = find_any(b, p, *b"$%");
            if p + 1 >= n {
                self.put_hot(s, n, HEREDOC);
                return n;
            }
            match (b[p + 1], b.get(p + 2)) {
                (b'{', _) => {
                    self.put_hot(s, p, HEREDOC);
                    return self.interp(p);
                }
                (c, Some(b'{')) if c == b[p] => {
                    self.put_hot(s, p, HEREDOC);
                    self.put(p, p + 2, HEREDOC_ESCAPE);
                    s = p + 2;
                    p += 3;
                }
                _ => p += 1,
            }
        }
    }

    /// A quoted string's text from `p`: to its close, the line's end, or an interpolation's
    /// opener.
    #[inline(always)]
    fn string(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut s = p;
        loop {
            p = find_any(b, p, *b"\"\\$%");
            if p >= n {
                self.put_hot(s, n, STRING);
                return n;
            }
            match b[p] {
                b'"' => {
                    self.put_hot(s, p, STRING);
                    self.put_hot(p, p + 1, STRING_END);
                    self.pop();
                    return p + 1;
                }
                b'\\' => {
                    let e = escape_end(b, p);
                    if e > p {
                        self.put_hot(s, p, STRING);
                        self.put(p, e, ESCAPE);
                        s = e;
                        p = e;
                    } else {
                        p += 1;
                    }
                }
                c => match (b.get(p + 1), b.get(p + 2)) {
                    (Some(b'{'), _) => {
                        self.put_hot(s, p, STRING);
                        return self.interp(p);
                    }
                    (Some(&d), Some(b'{')) if d == c => {
                        self.put_hot(s, p, STRING);
                        self.put(p, p + 2, ESCAPE);
                        s = p + 2;
                        p += 3;
                    }
                    _ => p += 1,
                },
            }
        }
    }

    /// `${` or `%{` at `p`: an interpolation opened.
    #[inline(always)]
    fn interp(&mut self, p: usize) -> usize {
        self.put_hot(p, p + 2, INTERP_BEGIN);
        self.push(F_INTERP);
        p + 2
    }

    /// A `#` or `//` comment, its mark `b[p..m]`.
    #[inline(always)]
    fn line_comment(&mut self, p: usize, m: usize) -> usize {
        let n = self.b.len();
        self.put_hot(p, m, COMMENT_MARK);
        self.put_hot(m, n, COMMENT);
        n
    }

    /// `/*` at `p`.
    #[inline(never)]
    fn block_comment(&mut self, p: usize) -> usize {
        self.put(p, p + 2, BLOCK_MARK);
        self.push(F_COMMENT);
        self.comment_rest(p + 2)
    }

    /// An open `/* */` comment from `p`: to its `*/`, or the line's end.
    #[inline(never)]
    fn comment_rest(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut q = p;
        loop {
            q = find_any(b, q, *b"*");
            if q + 1 >= n {
                self.put(p, n, BLOCK_COMMENT);
                return n;
            }
            if b[q + 1] == b'/' {
                self.put(p, q, BLOCK_COMMENT);
                self.put(q, q + 2, BLOCK_MARK);
                self.pop();
                return q + 2;
            }
            q += 1;
        }
    }
}

/// Whether an object's key ends before `q`: an `=` (not `==` or `=>`) or a `:` (not `::`).
#[inline(always)]
fn is_key(b: &[u8], q: usize) -> bool {
    match b.get(q) {
        Some(b'=') => !matches!(b.get(q + 1), Some(b'=' | b'>')),
        Some(b':') => b.get(q + 1) != Some(&b':'),
        _ => false,
    }
}

/// Where the escape at `p` (a `\`) ends: bat's `\\[nrt"\\]|\\u(\h{8}|\h{4})`, and HCL's
/// `\U\h{8}`; `p` for none.
#[inline(always)]
fn escape_end(b: &[u8], p: usize) -> usize {
    let hex = |from: usize, k: usize| {
        b.get(from..from + k)
            .is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit))
    };
    match b.get(p + 1) {
        Some(b'n' | b'r' | b't' | b'"' | b'\\') => p + 2,
        Some(b'u') if hex(p + 2, 8) => p + 10,
        Some(b'u') if hex(p + 2, 4) => p + 6,
        Some(b'U') if hex(p + 2, 8) => p + 10,
        _ => p,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Lang;
    use crate::tests::marked;

    fn of(lines: &[&str]) -> Vec<(String, String)> {
        marked(Lang::Terraform, lines)
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
    fn blocks_attributes_and_keys() {
        let l = [
            "resource \"aws_instance\" \"web\" {",
            "  ami = \"x\"",
            "  nullable=false",
            "  tags = {",
            "    Name = \"web\"",
            "    eventName : [\"INSERT\"]",
            "    aws.west = aws.west",
            "  }",
            "  dynamic \"ingress\" {",
            "    content {",
            "    }",
            "  }",
            "}",
            "path \"secret/data/*\" {",
            "}",
            "moved {",
            "  check {",
            "  }",
            "}",
            "block_type bare-label {",
            "}",
        ];
        has(&l, "resource", "storage.type.terraform");
        has(&l, "aws_instance", "string.quoted.double.terraform");
        has(
            &l,
            "ami",
            "variable.declaration.terraform variable.other.readwrite.terraform",
        );
        has(
            &l,
            "= ",
            "variable.declaration.terraform keyword.operator.assignment.terraform",
        );
        // `=` takes a blank after it, not the value's first byte
        has(
            &l,
            "nullable",
            "variable.declaration.terraform variable.other.readwrite.terraform",
        );
        has(
            &l,
            "=",
            "variable.declaration.terraform keyword.operator.assignment.terraform",
        );
        has(&l, "false", "constant.language.terraform");
        has(
            &l,
            "Name",
            "meta.mapping.key.terraform string.unquoted.terraform",
        );
        has(
            &l,
            "eventName",
            "meta.mapping.key.terraform string.unquoted.terraform",
        );
        has(
            &l,
            "aws",
            "meta.mapping.key.terraform string.unquoted.terraform",
        );
        has(
            &l,
            "west",
            "meta.mapping.key.terraform string.unquoted.terraform",
        );
        has(&l, "west", "variable.other.member.terraform");
        has(&l, "dynamic", "entity.name.type.terraform");
        has(&l, "content", "entity.name.type.terraform");
        has(&l, "path", "entity.name.type.terraform");
        has(&l, "secret/data/*", "string.quoted.double.terraform");
        // a top-level keyword at the top only
        has(&l, "moved", "storage.type.terraform");
        has(&l, "check", "entity.name.type.terraform");
        has(&l, "bare-label", "entity.name.label.terraform");
        has(&l, "{", "punctuation.section.block.begin.terraform");
        has(&l, "}", "punctuation.section.block.end.terraform");
    }

    #[test]
    fn expressions() {
        let l = [
            "x = [for s in var.l : upper(s) if s != \"\"]",
            "y = { for k, v in local.m : k => v... }",
            "z = try(aws_instance.web[*].id, a.0, (b), my_fn(c), -1.5e3, d.*.e)",
            "w = list(string)",
            "v = [*]",
        ];
        has(&l, "for", "keyword.control.terraform");
        has(&l, "in", "keyword.operator.word.terraform");
        has(&l, "if", "keyword.control.conditional.terraform");
        has(&l, "s", "variable.other.readwrite.terraform");
        has(&l, "=>", "storage.type.function.terraform");
        has(&l, "...", "keyword.operator.terraform");
        has(&l, "var", "support.constant.terraform");
        has(&l, "l", "variable.other.member.terraform");
        has(&l, "upper", "support.function.builtin.terraform");
        has(&l, "try", "support.function.builtin.terraform");
        has(&l, "my_fn", "variable.function.terraform");
        has(&l, "*", "keyword.operator.splat.terraform");
        has(&l, "0", "constant.numeric.integer.terraform");
        has(&l, "b", "variable.other.readwrite.terraform");
        has(
            &l,
            ".",
            "constant.numeric.float.terraform punctuation.separator.decimal.terraform",
        );
        has(
            &l,
            "e",
            "constant.numeric.float.terraform punctuation.separator.exponent.terraform",
        );
        has(&l, "-", "keyword.operator.arithmetic.terraform");
        has(&l, "list", "support.function.builtin.terraform");
        has(&l, "string", "storage.type.terraform");
        has(
            &l,
            "*",
            "punctuation.section.brackets.end.terraform keyword.operator.splat.terraform",
        );
        // a name in a call's arguments or a tuple is plain
        lacks(&l, "aws_instance");
        lacks(&l, "c");
    }

    #[test]
    fn strings_templates_and_heredocs() {
        let l = [
            "a = \"x ${var.y} %{ if z ~}q%{ endif } $${no} %%{no} \\n \\u00e9\"",
            "b = \"open",
            "c = 1",
            "d = <<-EOT",
            "  text ${x}",
            "  EOT",
            "e = jsonencode(<<EOF",
            "${y}",
            "EOF",
            ")",
            "f = \"${",
            "  g",
            "} h\"",
        ];
        has(&l, "${", "keyword.other.interpolation.begin.terraform");
        has(&l, "}", "keyword.other.interpolation.end.terraform");
        has(&l, "if", "keyword.control.terraform");
        has(&l, "endif", "keyword.control.terraform");
        has(&l, " ~", "keyword.operator.template.right.trim.terraform");
        has(
            &l,
            "$$",
            "string.quoted.double.terraform constant.character.escape.terraform",
        );
        has(
            &l,
            "%%",
            "string.quoted.double.terraform constant.character.escape.terraform",
        );
        has(
            &l,
            "\\n",
            "string.quoted.double.terraform constant.character.escape.terraform",
        );
        has(
            &l,
            "\\u00e9",
            "string.quoted.double.terraform constant.character.escape.terraform",
        );
        // a quoted string ends with its line
        has(
            &l,
            "c",
            "variable.declaration.terraform variable.other.readwrite.terraform",
        );
        has(&l, "<<-", "keyword.operator.heredoc.terraform");
        has(&l, "EOT", "keyword.control.heredoc.terraform");
        has(&l, "  text ", "string.unquoted.heredoc.terraform");
        has(&l, "  EOT", "keyword.control.heredoc.terraform");
        has(&l, "EOF", "keyword.control.heredoc.terraform");
        has(&l, ")", "punctuation.section.parens.end.terraform");
        // an interpolation across lines, its string going on after it
        has(&l, " h", "string.quoted.double.terraform");
    }

    #[test]
    fn comments() {
        let l = ["# a", "// b", "x = 1 /* c */ + 2", "/* d", "e */ y = 3"];
        has(
            &l,
            "#",
            "comment.line.terraform punctuation.definition.comment.terraform",
        );
        has(&l, " a", "comment.line.terraform");
        has(
            &l,
            "//",
            "comment.line.terraform punctuation.definition.comment.terraform",
        );
        has(&l, " c ", "comment.block.terraform");
        has(
            &l,
            "*/",
            "comment.block.terraform punctuation.definition.comment.terraform",
        );
        has(&l, "e ", "comment.block.terraform");
        has(
            &l,
            "y",
            "variable.declaration.terraform variable.other.readwrite.terraform",
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
        // every field a line hands the next: frames past the 32 kept levels, of every kind, a
        // here-document's label, a comment, an interpolation across lines
        let deep = "a {\n".repeat(12)
            + "x = [ ( f( {\n"
            + &"[for y in z : (\n".repeat(8)
            + &")]\n".repeat(10)
            + "\"${ {\n}\" ] ) ) }\n";
        let text = format!(
            "{deep}{}\nb = <<-EOT\n  ${{c\n}} d\n  EOT\n/* e\nf */ g = h(<<X\nX\n)\n",
            "}\n".repeat(14)
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
    }

    #[test]
    fn long_lines_lex_in_linear_time() {
        for unit in [
            "a ", "a \"b\" ", "a, ", "{ a ", "[for ", "x = ", "\"${", "<<", "/*", "1.", "a.", "~ ",
            "a {", "{ a.b, ", "x = { a.",
        ] {
            let l = unit.repeat(20_000);
            let t = std::time::Instant::now();
            lines(&l);
            assert!(t.elapsed().as_millis() < 200, "{unit:?}: {:?}", t.elapsed());
        }
    }
}
