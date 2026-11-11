//! Nix on a scanner of ada's own (X-103), from a floor probe: one loop over a line's bytes, a
//! byte's class one load from a table, a name read once and looked up once in one table (the
//! keywords, the constants, the builtins in Nix's global scope), the byte after it deciding a
//! lambda's parameter, and frames, four bits a level ([`Scan::frames`]), for what nests or runs
//! across lines: an attribute set or a function's formals, `let`'s bindings, a list, a
//! parenthesis, `inherit`'s names, `with`'s and `assert`'s head, `${…}`, a `"…"` or `''…''`
//! string, a path around an interpolation, a `/* */` comment. Kinds are bat's leaves; no theme
//! keys on a meta scope, so none are written.
//!
//! The readings that need an item's start: a binding's attribute path (its names bat's attribute
//! names, its dots plain, its `=` bat's bind) where a name in an attribute set or `let` is
//! followed by an attribute path and `=` on its line, and a function's formals (a name before
//! `,`, `?` or `}`).
//!
//! **Where the language and bat's grammar part, the language's reading** (each a departure):
//! - no `markup.italic` over an interpolation's text (bat's meta scope on `${…}`, which every
//!   built-in theme sets in italics: its blanks and plain bytes italic, a markup scope in code);
//! - `$${` is text, as Nix reads it (`$$` is two dollars; bat opens an interpolation at the
//!   second);
//! - a float is a number (`1.5`, `1.5e3`, `.5`; bat reads `1`, an operator and `5`, or
//!   `invalid`);
//! - a path runs on through an interpolation (`./foo/${name}.nix`, Nix 2.4's; bat ends it
//!   before the `${` and reads the rest as an operator and names);
//! - `TODO`, `FIXME`, `BUG` and `!!!` in a comment as words (bat's pattern finds `BUG` inside
//!   `DEBUG`), and on a block comment's first line too (bat's opener takes the line's text whole);
//! - `fetchTarball`, `fetchGit`, `fetchTree`, `fetchMercurial`, `fromTOML` and `placeholder`
//!   are builtins in Nix's global scope as `import` is (bat's list predates them);
//! - a name selected after `.` is a member, a builtin only under `builtins` (`builtins.readFile`;
//!   bat colors a member by its word: `lib.map` and `cfg.placeholder` a builtin,
//!   `builtins.readFile` a name);
//! - a name is Nix's, `-` and `'` in it (bat's `\b` finds `import` in `import-cargo`).

use crate::emit::{Rules, Run, Runs, Sink, Spans};
use crate::prim::{char_len, find_any, find_in};
use crate::words::{Words, text_len};
use crate::{Out, State};

/// The leaves: a token's scopes.
static LEAVES: [&str; 39] = [
    "",
    "comment.line.number-sign.nix",
    "comment.line.number-sign.nix markup.bold.comment.nix",
    "comment.block.nix",
    "comment.block.nix markup.bold.comment.nix",
    "string.quoted.double.nix",
    "string.quoted.double.nix punctuation.definition.string.double.start.nix",
    "string.quoted.double.nix punctuation.definition.string.double.end.nix",
    "string.quoted.double.nix constant.character.escape.nix",
    "string.quoted.other.nix",
    "string.quoted.other.nix punctuation.definition.string.other.start.nix",
    "string.quoted.other.nix punctuation.definition.string.other.end.nix",
    "string.quoted.other.nix constant.character.escape.nix",
    "punctuation.section.embedded.begin.nix",
    "punctuation.section.embedded.end.nix",
    "string.unquoted.path.nix",
    "string.unquoted.spath.nix",
    "string.unquoted.url.nix",
    "variable.parameter.name.nix",
    "variable.parameter.function.4.nix",
    "variable.parameter.function.1.nix",
    "entity.other.attribute-name.multipart.nix",
    "entity.other.attribute-name.single.nix",
    "keyword.operator.bind.nix",
    "keyword.operator.nix",
    "punctuation.terminator.bind.nix",
    "punctuation.terminator.inherit.nix",
    "keyword.other.nix",
    "keyword.other.inherit.nix",
    "constant.language.nix",
    "support.function.nix",
    "constant.numeric.nix",
    "punctuation.definition.expression.nix",
    "punctuation.section.function.arguments.nix",
    "punctuation.definition.list.nix",
    "punctuation.definition.attrset-or-function.nix",
    "punctuation.definition.attrset.nix",
    "punctuation.definition.entity.function.nix",
    "punctuation.definition.function.nix",
];

const COMMENT: u16 = 1;
const COMMENT_REMARK: u16 = 2;
const BLOCK_COMMENT: u16 = 3;
const BLOCK_REMARK: u16 = 4;
const DQ: u16 = 5;
const DQ_BEGIN: u16 = 6;
const DQ_END: u16 = 7;
const DQ_ESCAPE: u16 = 8;
const IND: u16 = 9;
const IND_BEGIN: u16 = 10;
const IND_END: u16 = 11;
const IND_ESCAPE: u16 = 12;
const INTERP_BEGIN: u16 = 13;
const INTERP_END: u16 = 14;
const PATH: u16 = 15;
const SPATH: u16 = 16;
const URL: u16 = 17;
const NAME: u16 = 18;
const PARAM: u16 = 19;
const FORMAL: u16 = 20;
const ATTR: u16 = 21;
const ATTR_SINGLE: u16 = 22;
const BIND: u16 = 23;
const OPERATOR: u16 = 24;
const TERMINATOR: u16 = 25;
const INHERIT_END: u16 = 26;
const KEYWORD: u16 = 27;
const INHERIT: u16 = 28;
const CONSTANT: u16 = 29;
const SUPPORT: u16 = 30;
const NUMBER: u16 = 31;
const PARENS: u16 = 32;
const INHERIT_PARENS: u16 = 33;
const LIST: u16 = 34;
const BRACE: u16 = 35;
const ATTRSET: u16 = 36;
const FORMALS_END: u16 = 37;
const COLON: u16 = 38;

/// The scopes under `source.nix` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    LEAVES.get(usize::from(kind)).map_or_else(Vec::new, |l| {
        l.split(' ').filter(|s| !s.is_empty()).collect()
    })
}

/// The floor probe's build (X-103's `sol`, `ADA_PARSE_X103_SOL` set at build time): the loop,
/// the lookups and the frames without the readings that need an item's start (a binding's
/// attribute path, a function's formals) or `inherit`'s names.
const SOL: bool = option_env!("ADA_PARSE_X103_SOL").is_some();

// A byte's class ([`CLASS`]).
const C_OTHER: u8 = 0;
/// ` `, `\t`, `\r`, `\x0b`, `\x0c`.
const C_BLANK: u8 = 1;
/// `[A-Za-z_]`.
const C_NAME: u8 = 2;
const C_DIGIT: u8 = 3;
const C_DQUOTE: u8 = 4;
const C_SQUOTE: u8 = 5;
const C_HASH: u8 = 6;
const C_SLASH: u8 = 7;
const C_LBRACE: u8 = 8;
const C_RBRACE: u8 = 9;
const C_LBRACKET: u8 = 10;
const C_RBRACKET: u8 = 11;
const C_LPAREN: u8 = 12;
const C_RPAREN: u8 = 13;
const C_SEMI: u8 = 14;
const C_DOT: u8 = 15;
/// `= ! < > & | + - * ? : , @ ~`.
const C_OP: u8 = 16;
const C_DOLLAR: u8 = 17;
/// A byte past ASCII: never a token's in Nix's code.
const C_WIDE: u8 = 18;

static CLASS: [u8; 256] = {
    let mut t = [C_WIDE; 256];
    let mut c = 0;
    while c < 128 {
        t[c] = match c as u8 {
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => C_BLANK,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => C_NAME,
            b'0'..=b'9' => C_DIGIT,
            b'"' => C_DQUOTE,
            b'\'' => C_SQUOTE,
            b'#' => C_HASH,
            b'/' => C_SLASH,
            b'{' => C_LBRACE,
            b'}' => C_RBRACE,
            b'[' => C_LBRACKET,
            b']' => C_RBRACKET,
            b'(' => C_LPAREN,
            b')' => C_RPAREN,
            b';' => C_SEMI,
            b'.' => C_DOT,
            b'=' | b'!' | b'<' | b'>' | b'&' | b'|' | b'+' | b'-' | b'*' | b'?' | b':' | b','
            | b'@' | b'~' => C_OP,
            b'$' => C_DOLLAR,
            _ => C_OTHER,
        };
        c += 1;
    }
    t
};

/// A name's bytes after its first, Nix's `[a-zA-Z0-9_'-]`.
static NAME_BYTE: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = b.is_ascii_alphanumeric() || b == b'_' || b == b'\'' || b == b'-';
        c += 1;
    }
    t
};

/// A path's bytes between its `/`s, Nix's `[a-zA-Z0-9._+-]`.
static PATH_BYTE: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-');
        c += 1;
    }
    t
};

/// A URI's bytes after its scheme's `:`, Nix's `[a-zA-Z0-9%/?:@&=+$,_.!~*'-]`.
static URI_BYTE: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'%' | b'/'
                    | b'?'
                    | b':'
                    | b'@'
                    | b'&'
                    | b'='
                    | b'+'
                    | b'$'
                    | b','
                    | b'-'
                    | b'_'
                    | b'.'
                    | b'!'
                    | b'~'
                    | b'*'
                    | b'\''
            );
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

/// Where the name that starts at `p` (a `[A-Za-z_]`) ends.
#[inline(always)]
fn name_end(b: &[u8], p: usize) -> usize {
    let mut e = p + 1;
    while e < b.len() && NAME_BYTE[usize::from(b[e])] {
        e += 1;
    }
    e
}

/// Where the run of path bytes at `p` ends.
#[inline(always)]
fn path_run(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && PATH_BYTE[usize::from(b[p])] {
        p += 1;
    }
    p
}

/// Whether `b[q..]` is `${`.
#[inline(always)]
fn interp_at(b: &[u8], q: usize) -> bool {
    b.get(q) == Some(&b'$') && b.get(q + 1) == Some(&b'{')
}

/// Where a path whose first segment's bytes end at `q` ends, Nix's `{PATH_CHAR}*(/{PATH_CHAR}+)+/?`:
/// `None` where no `/` and a path byte follow (or a `/` and `${`, a path's interpolation).
#[inline(always)]
fn path_end(b: &[u8], mut q: usize) -> Option<usize> {
    if b.get(q) != Some(&b'/') {
        return None;
    }
    match b.get(q + 1) {
        Some(&c) if PATH_BYTE[usize::from(c)] => {}
        _ if interp_at(b, q + 1) => return Some(q + 1),
        _ => return None,
    }
    while b.get(q) == Some(&b'/') && b.get(q + 1).is_some_and(|&c| PATH_BYTE[usize::from(c)]) {
        q = path_run(b, q + 1);
    }
    // a `/` at its end, or before an interpolation
    if b.get(q) == Some(&b'/') {
        q += 1;
    }
    Some(q)
}

// A word's flags, above its kind's byte ([`INFO`]).
/// `let`: [`F_LET`] pushed.
const W_LET: u16 = 1 << 8;
/// `in`: [`F_LET`] popped.
const W_IN: u16 = 1 << 9;
/// `with` and `assert`: [`F_WITH`] pushed, to their `;`.
const W_WITH: u16 = 1 << 10;
/// `inherit`: [`F_INHERIT`] pushed, to its `;`.
const W_INHERIT: u16 = 1 << 11;
/// A keyword: never an attribute's name.
const W_KEYWORD: u16 = 1 << 12;
/// What a word does past its kind.
const W_ACTS: u16 = W_LET | W_IN | W_WITH | W_INHERIT;

const WORD_LIST: &[(&[u8], u16)] = &[
    (b"let", KEYWORD | W_LET | W_KEYWORD),
    (b"in", KEYWORD | W_IN | W_KEYWORD),
    (b"with", KEYWORD | W_WITH | W_KEYWORD),
    (b"assert", KEYWORD | W_WITH | W_KEYWORD),
    (b"inherit", INHERIT | W_INHERIT | W_KEYWORD),
    (b"rec", KEYWORD | W_KEYWORD),
    (b"if", KEYWORD | W_KEYWORD),
    (b"then", KEYWORD | W_KEYWORD),
    (b"else", KEYWORD | W_KEYWORD),
    (b"or", OPERATOR | W_KEYWORD),
    (b"true", CONSTANT),
    (b"false", CONSTANT),
    (b"null", CONSTANT),
    (b"builtins", CONSTANT),
    // the builtins in Nix's global scope
    (b"scopedImport", SUPPORT),
    (b"import", SUPPORT),
    (b"isNull", SUPPORT),
    (b"abort", SUPPORT),
    (b"throw", SUPPORT),
    (b"baseNameOf", SUPPORT),
    (b"dirOf", SUPPORT),
    (b"removeAttrs", SUPPORT),
    (b"map", SUPPORT),
    (b"toString", SUPPORT),
    (b"derivationStrict", SUPPORT),
    (b"derivation", SUPPORT),
    (b"fetchTarball", SUPPORT),
    (b"fetchGit", SUPPORT),
    (b"fetchTree", SUPPORT),
    (b"fetchMercurial", SUPPORT),
    (b"fromTOML", SUPPORT),
    (b"placeholder", SUPPORT),
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
static WORDS: Words<128, NW, { text_len(&WORD_TEXT) }> = Words::new(&WORD_TEXT);
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

// The frames, four bits a level in [`Scan::frames`]; 0 is the file's top, or a level lost past
// 32 levels.
/// `{ … }`: an attribute set or a function's formals, not yet told apart.
const F_BRACE: u8 = 1;
/// `{ … }` known to be a function's formals (a `,` or `...` in it).
const F_FORMALS: u8 = 2;
/// `let`'s bindings, to `in`.
const F_LET: u8 = 3;
/// `[ … ]`.
const F_LIST: u8 = 4;
/// `( … )`.
const F_PARENS: u8 = 5;
/// `inherit`'s names, to `;`.
const F_INHERIT: u8 = 6;
/// `inherit ( … )`'s source.
const F_INHERIT_PARENS: u8 = 7;
/// `with`'s or `assert`'s head, to `;`.
const F_WITH: u8 = 8;
/// `${ … }`.
const F_INTERP: u8 = 9;
/// A `"…"` string.
const F_DQ: u8 = 10;
/// A `''…''` string.
const F_IND: u8 = 11;
/// A `/* … */` comment.
const F_COMMENT: u8 = 12;
/// A path under its interpolation: it goes on after the `}`.
const F_PATH: u8 = 13;
/// `...` at a line's end in formals: the next line's blanks before its `}` or `,` are the
/// ellipsis's, as bat's meta scope over them.
const F_ELLIPSIS: u8 = 14;

/// Lex `b`, a line of Nix without its `\n`, from `state`.
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

/// A block of Nix on one scan opened from `state` ([`crate::Carry`]).
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
    /// What each open frame is ([`F_BRACE`]…), four bits a level, the innermost lowest; past 32
    /// levels the outer ones are lost.
    frames: u128,
    /// How many frames are open, up to 255.
    depth: u8,
    /// At an item's start: a line's start, a `{`, a `;`, a `,`, `let`. Not kept between lines.
    start: bool,
    /// Where a binding's `=` is, while its attribute path is read past a part that isn't a
    /// name (a string, an interpolation), 0 for none; at [`Scan::bind_depth`]. Not kept
    /// between lines.
    bind_at: usize,
    bind_depth: u8,
    /// Where the last run of path bytes that started neither a path nor a URI ended: a start
    /// before it has the same answer (one search a run, not one a name in it). Not kept between
    /// lines.
    no_path: usize,
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
            start: true,
            bind_at: 0,
            bind_depth: 0,
            no_path: 0,
            k,
            out,
        }
    }

    /// The scan's fields from `state`: the frames in its levels and the first two withs.
    #[inline(always)]
    fn load(&mut self, state: &State) {
        let mut w = [0u8; 16];
        w[..14].copy_from_slice(&state.levels);
        w[14] = state.withs[0];
        w[15] = state.withs[1];
        self.frames = u128::from_le_bytes(w);
        self.depth = state.depth;
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

    /// The line, from what the line before left: a string's or a comment's rest first; the
    /// loop out of line where `SHARED` ([`Scan::code_shared`]).
    #[inline(always)]
    fn lex_line<const SHARED: bool>(&mut self) {
        self.start = true;
        self.bind_at = 0;
        self.no_path = 0;
        let p = match self.top() {
            F_DQ => self.dq(0),
            F_IND => self.ind(0),
            F_COMMENT => self.comment_rest(0),
            F_ELLIPSIS => self.ellipsis_rest(),
            _ => 0,
        };
        if SHARED {
            self.code_shared(p);
        } else {
            self.code(p);
        }
        // a path ends with its line
        if self.top() == F_PATH {
            self.pop();
        }
    }

    /// [`Scan::code`] out of line: on the run sink, one copy that the line API and a block's
    /// lines share; on the span sink, which ada doesn't hold, inlined.
    #[inline(never)]
    fn code_shared(&mut self, p: usize) {
        self.code(p);
    }

    /// The line's code from `p`. A string or a comment is read where it opens and runs to its
    /// close or the line's end, so the loop's frame is always code's.
    #[inline(always)]
    fn code(&mut self, mut p: usize) {
        let b = self.b;
        let n = b.len();
        while p < n {
            let c = b[p];
            p = match CLASS[usize::from(c)] {
                C_BLANK => blanks(b, p + 1),
                C_NAME => self.name(p),
                C_DIGIT => self.number(p),
                C_DQUOTE => {
                    if !SOL && self.start {
                        self.lead(p);
                    }
                    self.put_hot(p, p + 1, DQ_BEGIN);
                    self.push(F_DQ);
                    self.dq(p + 1)
                }
                C_SQUOTE if b.get(p + 1) == Some(&b'\'') => {
                    self.put_hot(p, p + 2, IND_BEGIN);
                    self.push(F_IND);
                    self.ind(p + 2)
                }
                C_HASH => self.line_comment(p),
                C_SLASH => self.slash(p),
                C_LBRACE => {
                    self.put_hot(p, p + 1, BRACE);
                    self.push(F_BRACE);
                    self.start = true;
                    p + 1
                }
                C_RBRACE => self.rbrace(p),
                C_LBRACKET => {
                    self.put_hot(p, p + 1, LIST);
                    self.push(F_LIST);
                    self.start = false;
                    p + 1
                }
                C_RBRACKET => {
                    self.unwind();
                    if self.top() == F_LIST {
                        self.put_hot(p, p + 1, LIST);
                        self.pop();
                    }
                    p + 1
                }
                C_LPAREN => {
                    self.start = false;
                    if self.top() == F_INHERIT {
                        self.put(p, p + 1, INHERIT_PARENS);
                        self.push(F_INHERIT_PARENS);
                    } else {
                        self.put_hot(p, p + 1, PARENS);
                        self.push(F_PARENS);
                    }
                    p + 1
                }
                C_RPAREN => {
                    self.unwind();
                    match self.top() {
                        F_PARENS => {
                            self.put_hot(p, p + 1, PARENS);
                            self.pop();
                        }
                        F_INHERIT_PARENS => {
                            self.put(p, p + 1, INHERIT_PARENS);
                            self.pop();
                        }
                        _ => {}
                    }
                    p + 1
                }
                C_SEMI => self.semi(p),
                C_DOT => self.dot(p),
                C_OP => self.op(p),
                C_DOLLAR if b.get(p + 1) == Some(&b'{') => {
                    if !SOL && self.start {
                        self.lead(p);
                    }
                    self.interp(p)
                }
                C_WIDE => p + char_len(c),
                _ => p + 1,
            };
        }
    }

    /// Before a bracket's closer: the frames of a head left open in it (`with`'s, `inherit`'s,
    /// `let`'s) closed with it.
    #[inline(always)]
    fn unwind(&mut self) {
        while matches!(self.top(), F_WITH | F_INHERIT | F_LET) {
            self.pop();
        }
    }

    /// A name at `p`: a path or a URI it starts, a word, an attribute's name, a parameter, or a
    /// name.
    #[inline(always)]
    fn name(&mut self, p: usize) -> usize {
        let b = self.b;
        let e = name_end(b, p);
        if let Some(&c) = b.get(e)
            && matches!(c, b'/' | b'.' | b'+' | b':')
            && e > self.no_path
            && let Some(r) = self.path_or_uri(p, e)
        {
            return r;
        }
        let w = WORDS.value(&b[p..e], &INFO);
        let top = self.top();
        if !SOL && self.start {
            self.start = false;
            if w & W_KEYWORD == 0
                && matches!(top, F_BRACE | F_FORMALS | F_LET)
                && let Some(r) = self.at_start(p, e, w, top)
            {
                return r;
            }
        }
        self.start = false;
        if !SOL && top == F_INHERIT && w & W_KEYWORD == 0 {
            self.put(p, e, ATTR_SINGLE);
            return e;
        }
        let kind = w & 0xff;
        if kind != 0 {
            self.put_hot(p, e, kind);
            if w & W_ACTS != 0 {
                self.keyword(w);
            }
            return e;
        }
        let kind = match b.get(e) {
            Some(b':' | b'@') => PARAM,
            Some(b' ') if b.get(e + 1) == Some(&b':') => PARAM,
            _ => NAME,
        };
        self.put_hot(p, e, kind);
        e
    }

    /// A keyword's frame: `let`'s pushed, `in`'s popped, `with`'s, `assert`'s and `inherit`'s
    /// pushed to their `;`.
    #[inline(never)]
    fn keyword(&mut self, w: u16) {
        if w & W_LET != 0 {
            self.push(F_LET);
            self.start = true;
        } else if w & W_IN != 0 {
            if self.top() == F_LET {
                self.pop();
            }
        } else if w & W_WITH != 0 {
            self.push(F_WITH);
        } else {
            self.push(F_INHERIT);
        }
    }

    /// A name `b[p..e]` at an item's start in an attribute set, a function's formals or `let`:
    /// a binding's attribute path, or a formal; `None` for an expression's name.
    #[inline(always)]
    fn at_start(&mut self, p: usize, e: usize, w: u16, top: u8) -> Option<usize> {
        let b = self.b;
        let q = blanks(b, e);
        // a formal is a name; at a line's end, in formals or a brace's first
        let formal = w == 0 && top != F_LET;
        match b.get(q) {
            Some(b'=') if b.get(q + 1) != Some(&b'=') => {
                self.put_hot(p, e, ATTR);
                self.put_hot(q, q + 1, BIND);
                Some(q + 1)
            }
            Some(b'.') => self.binding(p, e),
            Some(b',' | b'?' | b'}') if formal => {
                self.put_hot(p, e, FORMAL);
                Some(e)
            }
            None if formal && (top == F_FORMALS || b[..p].trim_ascii_end().ends_with(b"{")) => {
                self.put_hot(p, e, FORMAL);
                Some(e)
            }
            _ => None,
        }
    }

    /// A name `b[p..e]` at an item's start with a `.` after it: a binding where the attribute
    /// path runs to an `=` (not `==`) on the line, its names attributes', its dots plain; a part
    /// that isn't a name (a string, an interpolation) read by the loop, the dots and names after
    /// it the path's still ([`Scan::bind_at`]).
    #[inline(never)]
    fn binding(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        let (eq, other) = attr_path(b, e)?;
        let to = if other == 0 { eq } else { other };
        let mut s = p;
        while s < to {
            let t = name_end(b, s);
            self.put(s, t, ATTR);
            s = blanks(b, t);
            if s < to {
                // the dot
                s = blanks(b, s + 1);
            }
        }
        if other == 0 {
            self.put_hot(eq, eq + 1, BIND);
            return Some(eq + 1);
        }
        self.bind_at = eq;
        self.bind_depth = self.depth;
        Some(other)
    }

    /// A string or an interpolation at `p`, at an item's start in an attribute set or `let`:
    /// where it starts a binding's attribute path, its dots and names after it the path's
    /// ([`Scan::bind_at`]); the loop reads it.
    #[inline(never)]
    fn lead(&mut self, p: usize) {
        self.start = false;
        if !matches!(self.top(), F_BRACE | F_LET) {
            return;
        }
        let b = self.b;
        let r = if b[p] == b'"' {
            dq_end(b, p + 1)
        } else {
            braces_end(b, p + 2)
        };
        if let Some((eq, _)) = r.and_then(|r| attr_path(b, r)) {
            self.bind_at = eq;
            self.bind_depth = self.depth;
        }
    }

    /// A name `b[p..e]` followed by a path's or a URI's bytes: the path or URI it starts, or
    /// `None`.
    #[inline(never)]
    fn path_or_uri(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        let q = path_run(b, p);
        // a `'` in the name: no path's byte
        if q < e {
            return None;
        }
        if let Some(r) = path_end(b, q) {
            return Some(self.path(p, r));
        }
        self.no_path = q;
        // a URI: a scheme, `[a-zA-Z][a-zA-Z0-9+.-]*`, its `:` and a URI's byte
        if b.get(q) == Some(&b':')
            && b.get(q + 1).is_some_and(|&c| URI_BYTE[usize::from(c)])
            && !b[p..q].contains(&b'_')
        {
            let mut r = q + 1;
            while r < b.len() && URI_BYTE[usize::from(b[r])] {
                r += 1;
            }
            self.start = false;
            self.put(p, r, URL);
            return Some(r);
        }
        None
    }

    /// A path `b[p..r]`: written, and an interpolation that follows it opened, the path going
    /// on after its `}` ([`F_PATH`]).
    #[inline(never)]
    fn path(&mut self, p: usize, r: usize) -> usize {
        self.start = false;
        self.put(p, r, PATH);
        if interp_at(self.b, r) {
            self.push(F_PATH);
            return self.interp(r);
        }
        r
    }

    /// A path's rest from `p`, after an interpolation in it: its bytes and `/`s, another
    /// interpolation, or its end.
    #[inline(never)]
    fn path_rest(&mut self, p: usize) -> usize {
        let b = self.b;
        let mut r = p;
        while r < b.len() && (PATH_BYTE[usize::from(b[r])] || b[r] == b'/') {
            r += 1;
        }
        self.put(p, r, PATH);
        if interp_at(b, r) {
            return self.interp(r);
        }
        self.pop();
        r
    }

    /// A number at `p`: digits, a fraction and an exponent (Nix's float); a path it starts.
    #[inline(always)]
    fn number(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        self.start = false;
        let mut e = p + 1;
        while e < n && b[e].is_ascii_digit() {
            e += 1;
        }
        if e < n
            && e > self.no_path
            && matches!(b[e], b'/' | b'.' | b'+' | b'-' | b'_' | b'a'..=b'z' | b'A'..=b'Z')
        {
            let q = path_run(b, p);
            if let Some(r) = path_end(b, q) {
                return self.path(p, r);
            }
            self.no_path = q;
            // Nix's float has a fraction, its exponent after it
            if b[e] == b'.' {
                e += 1;
                while e < n && b[e].is_ascii_digit() {
                    e += 1;
                }
                e = exponent(b, e);
            }
        }
        self.put_hot(p, e, NUMBER);
        e
    }

    /// A `.` at `p`: `...`, a path (`./`, `../`), a float (`.5`), an attribute path's dot, or a
    /// selection's operator.
    #[inline(always)]
    fn dot(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        self.start = false;
        match b.get(p + 1) {
            Some(b'.') if b.get(p + 2) == Some(&b'.') => return self.ellipsis(p),
            Some(b'.' | b'/') if p >= self.no_path => {
                let q = path_run(b, p);
                if let Some(r) = path_end(b, q) {
                    return self.path(p, r);
                }
                self.no_path = q;
            }
            Some(c) if c.is_ascii_digit() => {
                let mut e = p + 2;
                while e < n && b[e].is_ascii_digit() {
                    e += 1;
                }
                let e = exponent(b, e);
                self.put(p, e, NUMBER);
                return e;
            }
            _ => {}
        }
        if p < self.bind_at && self.depth == self.bind_depth {
            // an attribute path's dot, and its name
            let t = blanks(b, p + 1);
            if b.get(t).is_some_and(|&c| CLASS[usize::from(c)] == C_NAME) {
                let e = name_end(b, t);
                self.put(t, e, ATTR);
                return e;
            }
            return p + 1;
        }
        self.put_hot(p, p + 1, OPERATOR);
        // the member it selects: a builtin's name under `builtins`, else a name, whatever
        // word it is (`cfg.placeholder`, `lib.map`)
        let q = p + 1;
        if b.get(q).is_some_and(|&c| CLASS[usize::from(c)] == C_NAME) {
            let e = name_end(b, q);
            let kind = if after_builtins(b, p) { SUPPORT } else { NAME };
            self.put_hot(q, e, kind);
            return e;
        }
        q
    }

    /// `...` at `p`: the formals' rest, with the blanks after it to a `}` or `,`, or to the
    /// line's end and on the next line's start ([`F_ELLIPSIS`]), as bat's meta scope over them.
    #[inline(never)]
    fn ellipsis(&mut self, p: usize) -> usize {
        let b = self.b;
        if self.top() == F_BRACE {
            self.set_top(F_FORMALS);
        }
        let q = blanks(b, p + 3);
        match b.get(q) {
            Some(b'}' | b',') => {
                self.put(p, q, OPERATOR);
                q
            }
            None if self.top() == F_FORMALS => {
                self.put(p, q, OPERATOR);
                self.push(F_ELLIPSIS);
                q
            }
            _ => {
                self.put(p, p + 3, OPERATOR);
                p + 3
            }
        }
    }

    /// A line after `...` at a line's end: its blanks before a `}` or `,` the ellipsis's.
    #[inline(never)]
    fn ellipsis_rest(&mut self) -> usize {
        let b = self.b;
        self.pop();
        let q = blanks(b, 0);
        match b.get(q) {
            Some(b'}' | b',') if q > 0 => {
                self.put(0, q, OPERATOR);
                q
            }
            None => {
                // a blank line: the ellipsis's still
                self.push(F_ELLIPSIS);
                q
            }
            _ => 0,
        }
    }

    /// A `/` at `p`: `//`, a comment, an absolute path, or the operator.
    #[inline(always)]
    fn slash(&mut self, p: usize) -> usize {
        let b = self.b;
        self.start = false;
        match b.get(p + 1) {
            Some(b'/') => {
                self.put_hot(p, p + 2, OPERATOR);
                p + 2
            }
            Some(b'*') => self.block_comment(p),
            _ => {
                if let Some(r) = path_end(b, p) {
                    return self.path(p, r);
                }
                self.put_hot(p, p + 1, OPERATOR);
                p + 1
            }
        }
    }

    /// An operator's byte at `p`.
    #[inline(always)]
    fn op(&mut self, p: usize) -> usize {
        let b = self.b;
        let next = b.get(p + 1).copied().unwrap_or(0);
        self.start = false;
        let (e, kind) = match b[p] {
            b'=' => match next {
                b'=' => (p + 2, OPERATOR),
                _ => {
                    self.bind_at = 0;
                    (p + 1, BIND)
                }
            },
            b'!' | b'<' | b'>' if next == b'=' => (p + 2, OPERATOR),
            b'<' => return self.spath(p),
            b'&' if next == b'&' => (p + 2, OPERATOR),
            b'|' if next == b'|' => (p + 2, OPERATOR),
            b'+' if next == b'+' => (p + 2, OPERATOR),
            b'-' if next == b'>' => (p + 2, OPERATOR),
            b'!' | b'>' | b'+' | b'-' | b'*' | b'?' => (p + 1, OPERATOR),
            b':' => (p + 1, COLON),
            b',' => {
                if self.top() == F_BRACE {
                    self.set_top(F_FORMALS);
                }
                self.start = !SOL;
                (p + 1, OPERATOR)
            }
            b'~' => {
                if let Some(r) = path_end(b, p + 1) {
                    return self.path(p, r);
                }
                return p + 1;
            }
            _ => return p + 1,
        };
        self.put_hot(p, e, kind);
        e
    }

    /// `<` at `p`: a search path (`<nixpkgs/lib>`), or the operator.
    #[inline(never)]
    fn spath(&mut self, p: usize) -> usize {
        let b = self.b;
        let mut q = path_run(b, p + 1);
        if q > p + 1 {
            while b.get(q) == Some(&b'/')
                && b.get(q + 1).is_some_and(|&c| PATH_BYTE[usize::from(c)])
            {
                q = path_run(b, q + 1);
            }
            if b.get(q) == Some(&b'>') {
                self.put(p, q + 1, SPATH);
                return q + 1;
            }
        }
        self.put_hot(p, p + 1, OPERATOR);
        p + 1
    }

    /// A `;` at `p`: `with`'s or `assert`'s (plain, as bat's), `inherit`'s, or a binding's.
    #[inline(always)]
    fn semi(&mut self, p: usize) -> usize {
        match self.top() {
            F_WITH => {
                self.pop();
                self.start = false;
            }
            F_INHERIT => {
                self.put(p, p + 1, INHERIT_END);
                self.pop();
                self.start = true;
            }
            _ => {
                self.put_hot(p, p + 1, TERMINATOR);
                self.start = true;
            }
        }
        p + 1
    }

    /// A `}` at `p`: an attribute set's or formals' (the frames of a head left open in it closed
    /// with it), an interpolation's (whose string or path goes on), or none.
    #[inline(always)]
    fn rbrace(&mut self, p: usize) -> usize {
        self.start = false;
        self.unwind();
        match self.top() {
            F_INTERP => {
                self.put_hot(p, p + 1, INTERP_END);
                self.pop();
                return match self.top() {
                    F_DQ => self.dq(p + 1),
                    F_IND => self.ind(p + 1),
                    F_PATH => self.path_rest(p + 1),
                    _ => p + 1,
                };
            }
            F_BRACE => {
                let b = self.b;
                let q = blanks(b, p + 1);
                let kind = if matches!(b.get(q), Some(b':' | b'@')) {
                    FORMALS_END
                } else {
                    ATTRSET
                };
                self.put_hot(p, p + 1, kind);
                self.pop();
            }
            F_FORMALS => {
                self.put_hot(p, p + 1, FORMALS_END);
                self.pop();
            }
            _ => {}
        }
        p + 1
    }

    /// `${` at `p`: an interpolation opened.
    #[inline(always)]
    fn interp(&mut self, p: usize) -> usize {
        self.start = false;
        self.put_hot(p, p + 2, INTERP_BEGIN);
        self.push(F_INTERP);
        p + 2
    }

    /// A `"…"` string's text from `p`: to its close, the line's end, or an interpolation's
    /// opener. `\` escapes the character after it; `$$` is text, so `$${` is too.
    #[inline(always)]
    fn dq(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut s = p;
        loop {
            p = find_any(b, p, *b"\"\\$");
            if p >= n {
                if s < n {
                    self.put_hot(s, n, DQ);
                }
                return n;
            }
            match b[p] {
                b'"' => {
                    self.put_hot(s, p, DQ);
                    self.put_hot(p, p + 1, DQ_END);
                    self.pop();
                    return p + 1;
                }
                b'\\' => {
                    if p + 1 < n {
                        let e = p + 1 + char_len(b[p + 1]);
                        self.put_hot(s, p, DQ);
                        self.put(p, e.min(n), DQ_ESCAPE);
                        s = e.min(n);
                        p = s;
                    } else {
                        p += 1;
                    }
                }
                _ => match b.get(p + 1) {
                    Some(b'{') => {
                        self.put_hot(s, p, DQ);
                        return self.interp(p);
                    }
                    Some(b'$') => p += 2,
                    _ => p += 1,
                },
            }
        }
    }

    /// A `''…''` string's text from `p`: to its close, the line's end, or an interpolation's
    /// opener. `''$`, `'''` and `''\` with the character after it are escapes; `$$` is text.
    #[inline(always)]
    fn ind(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut s = p;
        loop {
            p = find_any(b, p, *b"'$");
            if p >= n {
                if s < n {
                    self.put_hot(s, n, IND);
                }
                return n;
            }
            if b[p] == b'\'' {
                if b.get(p + 1) != Some(&b'\'') {
                    p += 1;
                    continue;
                }
                let e = match b.get(p + 2) {
                    Some(b'$' | b'\'') => p + 3,
                    Some(b'\\') => (p + 3 + b.get(p + 3).map_or(0, |&c| char_len(c))).min(n),
                    _ => {
                        self.put_hot(s, p, IND);
                        self.put_hot(p, p + 2, IND_END);
                        self.pop();
                        return p + 2;
                    }
                };
                self.put_hot(s, p, IND);
                self.put(p, e, IND_ESCAPE);
                s = e;
                p = e;
                continue;
            }
            match b.get(p + 1) {
                Some(b'{') => {
                    self.put_hot(s, p, IND);
                    return self.interp(p);
                }
                Some(b'$') => p += 2,
                _ => p += 1,
            }
        }
    }

    /// A `#` comment at `p`.
    #[inline(always)]
    fn line_comment(&mut self, p: usize) -> usize {
        let n = self.b.len();
        self.remarks(p, n, COMMENT, COMMENT_REMARK);
        n
    }

    /// A comment's text `b[s..e]` as `kind`, its remarks (`TODO`, `FIXME`, `BUG`, `!!!`) as
    /// `remark`.
    #[inline(always)]
    fn remarks(&mut self, mut s: usize, e: usize, kind: u16, remark: u16) {
        let b = self.b;
        let mut q = s;
        loop {
            q = find_in(b, q, e, *b"TFB!");
            if q >= e {
                break;
            }
            let r = remark_end(b, q, e);
            if r > q {
                self.put_hot(s, q, kind);
                self.put(q, r, remark);
                s = r;
                q = r;
            } else {
                q += 1;
            }
        }
        if s < e {
            self.put_hot(s, e, kind);
        }
    }

    /// `/*` at `p`.
    #[inline(never)]
    fn block_comment(&mut self, p: usize) -> usize {
        self.push(F_COMMENT);
        self.put(p, p + 2, BLOCK_COMMENT);
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
                self.remarks(p, n, BLOCK_COMMENT, BLOCK_REMARK);
                return n;
            }
            if b[q + 1] == b'/' {
                self.remarks(p, q + 2, BLOCK_COMMENT, BLOCK_REMARK);
                self.pop();
                return q + 2;
            }
            q += 1;
        }
    }
}

/// An attribute path's rest from `r`, past its first part: `(.` part`)*` and an `=` (not `==`)
/// on the line, a part a name, a `"…"` string or `${…}`. Where its `=` is, and where its first
/// part that isn't a name starts (its `.`), 0 for none; `None` for no binding's.
fn attr_path(b: &[u8], mut r: usize) -> Option<(usize, usize)> {
    let n = b.len();
    let mut other = 0;
    loop {
        let q = blanks(b, r);
        match b.get(q) {
            Some(b'.') => {
                let t = blanks(b, q + 1);
                match b.get(t) {
                    Some(&c) if CLASS[usize::from(c)] == C_NAME => r = name_end(b, t),
                    Some(b'"') => {
                        if other == 0 {
                            other = q;
                        }
                        r = dq_end(b, t + 1)?;
                    }
                    Some(b'$') if b.get(t + 1) == Some(&b'{') => {
                        if other == 0 {
                            other = q;
                        }
                        r = braces_end(b, t + 2)?;
                    }
                    _ => return None,
                }
            }
            Some(b'=') if b.get(q + 1) != Some(&b'=') => return Some((q, other)),
            _ => return None,
        }
        if r >= n {
            return None;
        }
    }
}

/// Whether the `.` at `p` follows the name `builtins`.
#[inline(always)]
fn after_builtins(b: &[u8], p: usize) -> bool {
    p >= 8
        && b[p - 1] == b's'
        && u64::from_le_bytes(b[p - 8..p].try_into().unwrap_or([0; 8]))
            == u64::from_le_bytes(*b"builtins")
        && (p == 8 || !NAME_BYTE[usize::from(b[p - 9])] && b[p - 9] != b'.')
}

/// Where a remark at `q` ends (before `e`): `TODO`, `FIXME`, `BUG` as words, `!!!`; `q` for
/// none.
#[inline(never)]
fn remark_end(b: &[u8], q: usize, e: usize) -> usize {
    let word = |w: &[u8]| {
        let r = q + w.len();
        r <= e
            && &b[q..r] == w
            && (q == 0 || !NAME_BYTE[usize::from(b[q - 1])])
            && (r == b.len() || !NAME_BYTE[usize::from(b[r])])
    };
    let w: &[u8] = match b[q] {
        b'T' => b"TODO",
        b'F' => b"FIXME",
        b'B' => b"BUG",
        _ => {
            return if q + 3 <= e && &b[q..q + 3] == b"!!!" {
                q + 3
            } else {
                q
            };
        }
    };
    if word(w) { q + w.len() } else { q }
}

/// Where a float's exponent at `p` ends (`e`, an optional sign, digits); `p` for none.
#[inline(always)]
fn exponent(b: &[u8], p: usize) -> usize {
    if !matches!(b.get(p), Some(b'e' | b'E')) {
        return p;
    }
    let s = if matches!(b.get(p + 1), Some(b'+' | b'-')) {
        p + 2
    } else {
        p + 1
    };
    if !b.get(s).is_some_and(u8::is_ascii_digit) {
        return p;
    }
    let mut e = s + 1;
    while e < b.len() && b[e].is_ascii_digit() {
        e += 1;
    }
    e
}

/// How far an attribute path's string or interpolation is looked for its end.
const PART: usize = 256;

/// Where a `"…"` string whose text starts at `p` ends, past its `"`, within [`PART`] bytes;
/// `None` for one that runs on.
fn dq_end(b: &[u8], mut p: usize) -> Option<usize> {
    let end = (p + PART).min(b.len());
    loop {
        p = find_in(b, p, end, *b"\"\\");
        if p >= end {
            return None;
        }
        match b[p] {
            b'"' => return Some(p + 1),
            _ => p += 2,
        }
    }
}

/// Where `${…}`'s braces whose inside starts at `p` close, past the `}`, within [`PART`] bytes
/// (braces counted, strings' braces too); `None` for ones that run on.
fn braces_end(b: &[u8], mut p: usize) -> Option<usize> {
    let end = (p + PART).min(b.len());
    let mut depth = 1u32;
    loop {
        p = find_in(b, p, end, *b"{}");
        if p >= end {
            return None;
        }
        match b[p] {
            b'{' => depth += 1,
            _ => {
                depth -= 1;
                if depth == 0 {
                    return Some(p + 1);
                }
            }
        }
        p += 1;
    }
}

#[cfg(test)]
mod tests;
