//! Clojure by a scanner of ada's own (ADR-40, speed first; X-006): bat's `Clojure` grammar, its
//! `#"…"` regex literals on ada-parse's regex scanner in Java's dialect (Clojure's regexes are
//! `java.util.regex`; bat embeds Sublime's generic `Regular Expression`, a difference by design).
//!
//! As Lisp's scanner, the grammar's contexts are frames of this scanner's design: one for each
//! open list, vector, map or declaration's body, and above them the one-shot frames a form's head
//! sets up (a `def`'s name, a `deftype`'s, a function's) or a reader macro does (`'`, `#`, `^`).
//! A frame that pops at its first non-space is replaced where the grammar sets the next one, so
//! a list's frame passes from its head to its name to its tail in place. Each token is classified
//! once, at the place the scan has reached: by its first byte, then its run of non-delimiters.
//! Kinds are bat's leaf scopes with the string or comment around them; the state between lines is
//! the stack, a byte a frame, runs of list tails as a count, and an open regex's state.

use super::DEPTHS;
use crate::emit::{Push, Rules, Run, Runs, Sink};
use crate::regex;
use crate::{Out, State};

macro_rules! leaves {
    ($($name:ident = $scope:literal,)*) => {
        #[allow(dead_code)]
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        #[repr(u16)]
        enum L { $($name,)* }
        static LEAVES: &[&str] = &[$($scope,)*];
    };
}

leaves! {
    Plain = "",
    CommentPunct = "comment.line.clojure punctuation.definition.comment",
    Comment = "comment.line.clojure",
    Comma = "punctuation.comma.clojure comment.punctuation.comma.clojure",
    Macro = "keyword.operator.macro.clojure",
    Illegal = "invalid.illegal.clojure",
    ParenBegin = "punctuation.section.parens.begin.clojure",
    ParenEnd = "punctuation.section.parens.end.clojure",
    BracketBegin = "punctuation.section.brackets.begin.clojure",
    BracketEnd = "punctuation.section.brackets.end.clojure",
    BraceBegin = "punctuation.section.braces.begin.clojure",
    BraceEnd = "punctuation.section.braces.end.clojure",
    StrBegin = "string.quoted.double.clojure punctuation.definition.string.begin.clojure",
    StrEnd = "string.quoted.double.clojure punctuation.definition.string.end.clojure",
    Str = "string.quoted.double.clojure",
    StrEscape = "string.quoted.double.clojure constant.character.escape.clojure",
    RegexBegin = "string.regexp.clojure punctuation.definition.string.begin.clojure",
    RegexEnd = "string.regexp.clojure punctuation.definition.string.end.clojure",
    Regex = "string.regexp.clojure",
    Char = "constant.character.clojure",
    ConstLang = "constant.language.clojure",
    KeywordPunct = "constant.other.keyword.clojure punctuation.definition.keyword.clojure",
    Keyword = "constant.other.keyword.clojure",
    Deprecated = "invalid.deprecated.clojure",
    Int = "constant.numeric.integer.decimal.clojure",
    IntSign = "constant.numeric.integer.decimal.clojure punctuation.definition.numeric.sign.clojure",
    IntSuffix = "constant.numeric.integer.decimal.clojure storage.type.numeric.clojure",
    Hex = "constant.numeric.integer.hexadecimal.clojure",
    HexSign = "constant.numeric.integer.hexadecimal.clojure punctuation.definition.numeric.sign.clojure",
    HexBase = "constant.numeric.integer.hexadecimal.clojure punctuation.definition.numeric.base.clojure",
    HexSuffix = "constant.numeric.integer.hexadecimal.clojure storage.type.numeric.clojure",
    Radix = "constant.numeric.integer.other.clojure",
    RadixSign = "constant.numeric.integer.other.clojure punctuation.definition.numeric.sign.clojure",
    RadixBase = "constant.numeric.integer.other.clojure punctuation.definition.numeric.base.clojure",
    Rational = "constant.numeric.rational.decimal.clojure",
    RationalSign = "constant.numeric.rational.decimal.clojure punctuation.definition.numeric.sign.clojure",
    RationalSep = "constant.numeric.rational.decimal.clojure punctuation.separator.rational.clojure",
    Float = "constant.numeric.float.decimal.clojure",
    FloatSign = "constant.numeric.float.decimal.clojure punctuation.definition.numeric.sign.clojure",
    FloatDot = "constant.numeric.float.decimal.clojure punctuation.separator.decimal.clojure",
    FloatSuffix = "constant.numeric.float.decimal.clojure storage.type.numeric.clojure",
    StorageType = "storage.type.clojure",
    Def = "storage.modifier.def.clojure",
    Fn = "storage.modifier.fn.clojure",
    Declare = "storage.modifier.declare.clojure",
    Function = "variable.function.clojure",
    FnName = "entity.name.function.clojure",
    TypeName = "entity.name.type.clojure",
    Forward = "entity.name.function.forward-decl.clojure",
    FnFn = "entity.name.function.fn.clojure",
    Inherited = "entity.other.inherited-class.clojure",
    Symbolic = "constant.other.symbolic.clojure",
    // a call's head by what `clojure.core` makes it, a design of ada's own ([`invoked`])
    Conditional = "keyword.control.conditional.clojure",
    Control = "keyword.control.clojure",
    Loop = "keyword.control.loop.clojure",
    Logical = "keyword.operator.logical.clojure",
    Import = "keyword.control.import.clojure",
    Operator = "keyword.operator.clojure",
    Binding = "keyword.declaration.variable.clojure",
    Builtin = "support.function.clojure",
}

/// The first kind of a regex literal's tokens: the regex scanner's kinds from here, under
/// `string.regexp.clojure`.
const REGEX: u16 = 128;

/// The first kind of a list's, a vector's or a map's delimiters by depth, past the regex
/// scanner's: `DEPTH_KINDS + 6 * depth + i`, `i` the delimiter's place from [`L::ParenBegin`]
/// (an opener, then its closer: parens, brackets, braces), its leaf under `meta.depth-N.clojure`.
const DEPTH_KINDS: u16 = REGEX + regex::KINDS as u16;

static DEPTH_SCOPES: [&str; DEPTHS as usize] = [
    "meta.depth-1.clojure",
    "meta.depth-2.clojure",
    "meta.depth-3.clojure",
    "meta.depth-4.clojure",
    "meta.depth-5.clojure",
    "meta.depth-6.clojure",
];

/// The scopes kind `kind` carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    if kind >= DEPTH_KINDS {
        let (depth, i) = ((kind - DEPTH_KINDS) / 6, (kind - DEPTH_KINDS) % 6);
        let Some(&outer) = DEPTH_SCOPES.get(usize::from(depth)) else {
            return Vec::new();
        };
        return vec![outer, LEAVES[usize::from(L::ParenBegin as u16 + i)]];
    }
    if kind >= REGEX {
        let mut s = vec!["string.regexp.clojure"];
        s.extend(regex::scopes(kind - REGEX));
        return s;
    }
    LEAVES.get(usize::from(kind)).map_or_else(Vec::new, |l| {
        l.split(' ').filter(|s| !s.is_empty()).collect()
    })
}

// The frames; below the stack is the top level (`main`).
const TOP: u8 = 0;
/// `pop-list-tail`: a list's elements and its `)`.
const LIST: u8 = 1;
/// A vector's elements and its `]`.
const VECTOR: u8 = 2;
/// A map's or a set's elements and its `}`.
const MAP: u8 = 3;
/// `pop-interface-list-tail`: a list in it is a method.
const INTERFACE: u8 = 4;
/// `pop-protocol-list-tail`: a list in it is a method's declaration.
const PROTOCOL: u8 = 5;
/// `pop-type-list-tail`: its symbols are the classes and protocols it implements.
const TYPE: u8 = 6;
// The one-shot frames, each in place of the list's frame until it sets the next.
/// `pop-list-head`.
const HEAD: u8 = 16;
/// `pop-invoke-list-tail`: the function a list calls.
const INVOKE: u8 = 17;
const DEF_NAME: u8 = 18;
const INTERFACE_NAME: u8 = 19;
const FORWARD: u8 = 20;
const PROTOCOL_NAME: u8 = 21;
const TYPE_NAME: u8 = 22;
const FN_NAME: u8 = 23;
/// `pop-expr`: one expression, after a reader macro.
const EXPR: u8 = 24;
/// `pop-dispatch-expr`: right after `#`.
const DISPATCH: u8 = 25;
/// `pop-dispatch-tag-expr`: a tagged literal's tag.
const TAG: u8 = 26;
/// `pop-symbolic-value`: after `##`.
const SYMBOLIC: u8 = 27;
const STRING: u8 = 48;
/// `#"…"`, its regex's state in [`State`]'s `arg`.
const REGEX_FRAME: u8 = 49;

const MAX: usize = 128;
// the engine's stack's size: [`super::expand`] writes this one
const _: () = assert!(MAX == super::MAX);
/// A byte of the packed state with this bit is a run of list tails, its count in the rest.
const RUN: u8 = 0x80;

/// `[\s,;\(\)\[\]{}\"`~@\^\\]`: the delimiters (ASCII; Unicode's spaces are too).
static DELIM: [bool; 256] = crate::regex::table(b" \t\n\x0b\x0c\r,;()[]{}\"`~@^\\");
/// [`DELIM`] and the bytes past ASCII: where [`Scan::run_end`] looks closer.
static RUN_STOP: [bool; 256] = {
    let mut t = DELIM;
    let mut i = 0x80;
    while i < 256 {
        t[i] = true;
        i += 1;
    }
    t
};
/// Of the sixteen bytes of `b` at `q` (`q + 16 <= b.len()`), a bit for each a step of
/// [`Scan::run_end`] stops at: [`DELIM`]'s bytes, the bytes past ASCII and the control bytes
/// (the last two the run's but Unicode's spaces, looked at past the step).
#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn stop_mask(b: &[u8], q: usize) -> u32 {
    use std::arch::x86_64::*;
    assert!(q + 16 <= b.len());
    // SAFETY: SSE2 is part of x86-64; the load reads `b[q..q + 16]`, inside `b` (above), and
    // needs no alignment
    unsafe {
        let w = _mm_loadu_si128(b.as_ptr().add(q).cast());
        let eq = |x: __m128i, c: u8| _mm_cmpeq_epi8(x, _mm_set1_epi8(c as i8));
        let or = |a, b| _mm_or_si128(a, b);
        // below `!`, signed: the spaces, the control bytes and the bytes past ASCII
        let low = _mm_cmplt_epi8(w, _mm_set1_epi8(b'!' as i8));
        // `& !1` takes `(` `)` to `(`; `| 0x20` takes `[` to `{` and `]` to `}`, and no other
        // byte to those
        let pair = _mm_and_si128(w, _mm_set1_epi8(!1));
        let up = _mm_or_si128(w, _mm_set1_epi8(0x20));
        let brackets = or(eq(pair, b'('), or(eq(up, b'{'), eq(up, b'}')));
        let marks = or(
            or(or(eq(w, b','), eq(w, b';')), or(eq(w, b'"'), eq(w, b'\\'))),
            or(or(eq(w, b'`'), eq(w, b'~')), or(eq(w, b'@'), eq(w, b'^'))),
        );
        _mm_movemask_epi8(or(low, or(brackets, marks))) as u32
    }
}

/// The first bytes of `match-noise`'s patterns ([`Scan::noise`]).
static NOISE: [bool; 256] = crate::regex::table(b";#,^");

/// A regex literal's host: its `"` ends it wherever the regex is (`with_prototype: (?=")`).
static REGEX_YIELDS: [bool; 256] = crate::regex::table(b"\"");
static REGEX_EMBED: regex::Embed<'static> = regex::Embed {
    dialect: regex::Dialect::Java,
    cooked: false,
    yields: &REGEX_YIELDS,
    stop: |b, p, _| b[p] == b'"' && !escaped(b, p),
    arg: 0,
    escape: regex::java_escape,
    top_only: false,
    continues: false,
    base: REGEX,
    text: L::Regex as u16,
};

/// What a call's head `w` is: a special form, a macro or a function of `clojure.core` by what
/// it does, else a function. bat's grammar calls them all functions (`variable.function`, which
/// no theme colors); this reading is ada's own, as Lisp's and Scheme's heads are.
fn invoked(w: &[u8]) -> L {
    use super::words::*;
    let info = clojure_lookup(w);
    match clojure_head(info) {
        C_CONDITIONAL => L::Conditional,
        C_CONTROL => L::Control,
        C_LOOP => L::Loop,
        C_LOGICAL => L::Logical,
        C_MODULE => L::Import,
        C_OPERATOR => L::Operator,
        C_BINDING => L::Binding,
        _ if info & C_BUILTIN != 0 => L::Builtin,
        _ => L::Function,
    }
}

/// Whether the byte at `p` follows an odd run of `\`: a regex's escape takes it (`\"`).
fn escaped(b: &[u8], p: usize) -> bool {
    b[..p].iter().rev().take_while(|&&c| c == b'\\').count() % 2 == 1
}

/// Lex `b`, a line of Clojure without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let mut s = Scan::fresh(b, state, out, Push);
    s.unpack(state);
    s.run();
    *state = s.pack();
}

/// [`line`]'s runs by theme rule pushed to `runs` (X-074, X-068's sink): [`crate::lex_runs`] for
/// Clojure.
#[inline(never)]
pub fn line_runs(
    b: &[u8],
    state: &mut State,
    out: &mut Out,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
) {
    let mut s = Scan::fresh(b, state, out, Runs::new(rules, runs));
    s.unpack(state);
    s.run();
    *state = s.pack();
    s.k.end_line(b.len());
}

/// A block of Clojure on one scan opened from `state` ([`crate::Carry`], X-070), `state` left
/// as the last line leaves it.
pub fn block(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    let mut out = Out::new(spans);
    let mut c = Carried {
        s: Scan::fresh(b"", state, &mut out, Push),
        deep: None,
    };
    c.s.unpack(state);
    let rest = pass(&mut c);
    *state = c.deep.unwrap_or_else(|| c.s.pack());
    rest
}

/// [`block`]'s runs by theme rule pushed to `runs` ([`crate::CarryRuns`], X-074).
pub fn block_runs(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    lines: &mut crate::RunLines,
) -> Option<usize> {
    let mut out = Out::new(spans);
    out.0.clear();
    let mut c = Carried {
        s: Scan::fresh(b"", state, &mut out, Runs::new(rules, runs)),
        deep: None,
    };
    c.s.unpack(state);
    let rest = lines.pass(&mut c);
    *state = c.deep.unwrap_or_else(|| c.s.pack());
    rest
}

/// A scan kept across a block's lines, and the state the last line left where a save and load
/// don't leave its frames as they are (lists below the stack, or more frames than a state's
/// codes): a block ending there hands on that state, as `line` would.
struct Carried<'b, 'o, 's, K: Sink> {
    s: Scan<'b, 'o, 's, K>,
    deep: Option<State>,
}

impl crate::Carry for Carried<'_, '_, '_, Push> {
    fn next(&mut self, b: &[u8]) -> &[crate::Span] {
        self.s.out.0.clear();
        // SAFETY: the line is set back to an empty one before `b`'s borrow ends
        self.carry(unsafe { crate::line_for(b) });
        self.s.out.0
    }
}

impl crate::CarryRuns for Carried<'_, '_, '_, Runs<'_>> {
    fn next(&mut self, b: &[u8]) -> usize {
        self.s.k.start_line();
        // SAFETY: as above
        self.carry(unsafe { crate::line_for(b) });
        self.s.k.end_line(b.len())
    }
}

impl<'b, K: Sink> Carried<'b, '_, '_, K> {
    /// `b`, the block's next line, lexed on the scan; the scan's line an empty one again after.
    #[inline(always)]
    fn carry(&mut self, b: &'b [u8]) {
        let s = &mut self.s;
        s.b = b;
        s.run();
        s.b = b"";
        // a regex's state is kept only while it is the top frame (`pack`)
        if s.top() != REGEX_FRAME {
            s.regex = 0;
        }
        // `keep`'s codes in order and `expand`'s frames from them: the frames as they are
        self.deep = (s.base > 0 || s.depth > super::KEPT).then(|| s.reloaded());
    }
}

impl<'b, 'o, 's, K: Sink> Scan<'b, 'o, 's, K> {
    /// A scan of `b`, returned as a literal so that it is built in place; its frames are
    /// [`Scan::unpack`]'s.
    #[inline(always)]
    fn fresh(b: &'b [u8], state: &State, out: &'o mut Out<'s>, k: K) -> Self {
        Scan {
            b,
            k,
            out,
            stack: [TOP; MAX + 1],
            depth: 0,
            base: 0,
            regex: state.arg as u8,
            level: DEPTH_KINDS + 6 * (state.arg >> 8).min(u16::from(DEPTHS) - 1),
        }
    }

    /// The scan as a state's save and the next line's load leave it: that state.
    #[cold]
    #[inline(never)]
    fn reloaded(&mut self) -> State {
        let state = self.pack();
        self.regex = state.arg as u8;
        self.level = DEPTH_KINDS + 6 * (state.arg >> 8).min(u16::from(DEPTHS) - 1);
        self.stack = [TOP; MAX + 1];
        self.depth = 0;
        self.base = 0;
        self.unpack(&state);
        state
    }
}

struct Scan<'b, 'o, 's, K: Sink> {
    b: &'b [u8],
    /// Where the lexemes go, a line's spans or its runs by theme rule (X-074, X-068's sink).
    k: K,
    /// The spans' vector: the span API's, and where an embedded regex pushes its spans for the
    /// run sink to take.
    out: &'o mut Out<'s>,
    /// The frames at `1..=depth`, bottom first; at 0 what is below them: a list body (`LIST`)
    /// while `base` counts any, else `TOP`. So the top is one load, `stack[depth]`.
    stack: [u8; MAX + 1],
    depth: usize,
    /// List tails below the stack.
    base: usize,
    /// An open regex literal's state.
    regex: u8,
    /// The lists, vectors and maps open, modulo [`DEPTHS`], as that depth's first kind
    /// ([`DEPTH_KINDS`]): the depth the next opener colors as.
    level: u16,
}

impl<K: Sink> Scan<'_, '_, '_, K> {
    fn unpack(&mut self, state: &State) {
        // most lines: every frame fits, none below the stack
        if let Some(d) = super::expand(state, LIST, &mut self.stack[1..]) {
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
        let mut skip = total.saturating_sub(MAX / 2);
        for i in 0..depth {
            let c = state.level(i);
            let k = count(c);
            let below = skip.min(k);
            skip -= below;
            if c & RUN != 0 {
                self.base += below;
                for _ in below..k {
                    self.push(LIST);
                }
            } else if below == 0 {
                self.push(c);
            }
        }
        if self.base > 0 {
            self.stack[0] = LIST;
        }
    }

    fn pack(&self) -> State {
        let arg = ((self.level - DEPTH_KINDS) / 6) << 8
            | if self.top() == REGEX_FRAME {
                u16::from(self.regex)
            } else {
                0
            };
        super::keep(&self.stack[1..=self.depth], LIST, self.base, arg)
    }

    fn top(&self) -> u8 {
        self.stack[self.depth]
    }

    fn push(&mut self, f: u8) {
        if self.depth == MAX {
            // the bottom frame dropped: a body into the count below
            if self.stack[1] == LIST {
                self.base += 1;
                self.stack[0] = LIST;
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

    /// `f` pushed in a body, set in place of a one-shot frame. Called: inlined it took more
    /// instructions out than cycles (X-038: cycles −2.4% called, its misses fewer).
    fn open(&mut self, f: u8, once: bool) {
        if once {
            self.set(f);
        } else {
            self.push(f);
        }
    }

    fn put(&mut self, from: usize, to: usize, kind: L) {
        self.k.push_call(&mut self.out, from, to, kind as u16);
    }

    /// A list's, a vector's or a map's delimiter `kind` at `from..to`, the kind of its depth
    /// ([`DEPTH_KINDS`]): an opener at the depth it opens, a closer at the depth it closes.
    #[inline(always)]
    fn delim(&mut self, from: usize, to: usize, kind: L) {
        let i = kind as u16 - L::ParenBegin as u16;
        debug_assert!(i < 6, "{kind:?} is no delimiter");
        // `level` is the depth's first kind: an opener's is the depth's, then the next one's
        const FIRST: u16 = DEPTH_KINDS;
        const LAST: u16 = DEPTH_KINDS + 6 * (DEPTHS as u16 - 1);
        if i & 1 != 0 {
            self.level = if self.level == FIRST {
                LAST
            } else {
                self.level - 6
            };
        }
        self.k.push(&mut self.out, from, to, self.level + i);
        if i & 1 == 0 {
            self.level = if self.level == LAST {
                FIRST
            } else {
                self.level + 6
            };
        }
    }

    fn at(&self, p: usize) -> u8 {
        self.b.get(p).copied().unwrap_or(b'\n')
    }

    /// Whether `p` is a delimiter or the line's end (`(?=[\s,;…])`, the `\n` a space).
    fn at_delim(&self, p: usize) -> bool {
        p >= self.b.len()
            || DELIM[usize::from(self.b[p])]
            || (self.b[p] >= 0x80 && self.wide_space(p) > 0)
    }

    /// The same with `#` and `'`: the numbers' lookahead.
    fn at_number_end(&self, p: usize) -> bool {
        self.at_delim(p) || matches!(self.b[p], b'#' | b'\'')
    }

    #[cold]
    fn wide_space(&self, p: usize) -> usize {
        match super::wide_char(self.b, p) {
            Some(c) if c.is_whitespace() => c.len_utf8(),
            _ => 0,
        }
    }

    /// `[^\s,;…]*` at `p`: where the run of non-delimiters ends. Sixteen bytes a step on a line
    /// of sixteen or more ([`stop_mask`]): a step's end is a bit's place, not a branch a byte,
    /// whose exit was the largest site of Clojure's branch misses (X-039: misses −18%, cycles
    /// −6.6%).
    fn run_end(&self, mut p: usize) -> usize {
        let (b, n) = (self.b, self.b.len());
        #[cfg(target_arch = "x86_64")]
        if n >= 16 {
            while p < n {
                // the sixteen bytes from `p`, the line's last sixteen where fewer are left; a
                // stop bit past them where none stops
                let c = p.min(n - 16);
                let s = p - c;
                let stops = (stop_mask(b, c) >> s) | (1 << (16 - s));
                let len = stops.trailing_zeros();
                p += len as usize;
                if len == (16 - s) as u32 {
                    continue;
                }
                // a step stops at the bytes below `!` and past ASCII too: a control byte, or
                // a byte past ASCII but Unicode's space, is the run's
                if DELIM[usize::from(b[p])] || (b[p] >= 0x80 && self.wide_space(p) > 0) {
                    return p;
                }
                p += 1;
            }
            return p;
        }
        self.run_end_bytes(p)
    }

    /// [`Scan::run_end`] a byte a step.
    fn run_end_bytes(&self, mut p: usize) -> usize {
        while p < self.b.len() {
            // one test a byte: a delimiter, or a byte past ASCII (a space of Unicode's, rare)
            if RUN_STOP[usize::from(self.b[p])] && (self.b[p] < 0x80 || self.wide_space(p) > 0) {
                break;
            }
            p += 1;
        }
        p
    }

    fn skip_spaces(&self, mut p: usize) -> usize {
        let (b, n) = (self.b, self.b.len());
        while p < n {
            let c = b[p];
            // ASCII but a space or a control byte: most turns start on a token
            if c.wrapping_sub(b'!') < 0x80 - b'!' {
                break;
            }
            if matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r') {
                p += 1;
            } else if c >= 0x80 {
                match self.wide_space(p) {
                    0 => break,
                    k => p += k,
                }
            } else {
                break;
            }
        }
        p
    }

    fn step(&self, p: usize) -> usize {
        (p + super::char_len(self.b[p])).min(self.b.len())
    }

    fn run(&mut self) {
        // a line's indentation, eight bytes a step, where a code frame's first turn would skip
        // it; a string's or a regex's spaces are its text, and a `#` before them is a tag's
        let top = self.top();
        let mut p = if top < STRING && top != DISPATCH {
            super::blanks(self.b, 0)
        } else {
            0
        };
        loop {
            let top = self.top();
            p = match top {
                STRING => self.string(p),
                REGEX_FRAME => self.regex(p),
                DISPATCH => self.dispatch(p),
                _ => {
                    p = self.skip_spaces(p);
                    if p >= self.b.len() {
                        return;
                    }
                    match top {
                        TOP | LIST | VECTOR | MAP | INTERFACE | PROTOCOL | TYPE => {
                            self.body(p, top)
                        }
                        HEAD => self.head(p),
                        EXPR => self.expression(p, true).unwrap_or_else(|| self.step(p)),
                        TAG | SYMBOLIC => self.tag(p, top),
                        _ => self.name(p, top),
                    }
                }
            };
            if p >= self.b.len() && !(self.top() == DISPATCH && p == self.b.len()) {
                return;
            }
        }
    }

    /// One turn of a body: its closer, what it adds, then `match-expr`.
    fn body(&mut self, p: usize, top: u8) -> usize {
        let c = self.b[p];
        // the top level has no closer: its `)` is `match-expr`'s, stray
        let (closer, kind) = match top {
            VECTOR => (b']', L::BracketEnd),
            MAP => (b'}', L::BraceEnd),
            _ => (b')', L::ParenEnd),
        };
        if c == closer && top != TOP {
            self.delim(p, p + 1, kind);
            self.pop();
            return p + 1;
        }
        if c == b'(' && matches!(top, INTERFACE | PROTOCOL | TYPE) {
            self.delim(p, p + 1, L::ParenBegin);
            self.push(if top == PROTOCOL { DEF_NAME } else { FN_NAME });
            return p + 1;
        }
        if top == TYPE
            && let Some(e) = self.implemented(p)
        {
            return e;
        }
        self.expression(p, false).unwrap_or_else(|| self.step(p))
    }

    /// `match-symbol-implemented-or-extended` at `p`.
    fn implemented(&mut self, p: usize) -> Option<usize> {
        if let Some(e) = self.constant(p) {
            self.put(p, e, L::ConstLang);
            return Some(e);
        }
        let e = self.symbol(p)?;
        self.put(p, e, L::Inherited);
        Some(e)
    }

    /// `match-noise` at `p`: a comment, a `,`, or `^` (its expression pushed). Tried at every
    /// turn, so its first byte is tested in place ([`NOISE`]) and the rest called.
    #[inline(always)]
    fn noise(&mut self, p: usize) -> Option<usize> {
        if !NOISE[usize::from(self.b[p])] {
            return None;
        }
        self.noise_at(p)
    }

    /// [`Scan::noise`] past its first byte's test.
    fn noise_at(&mut self, p: usize) -> Option<usize> {
        match self.b[p] {
            b';' => {
                let mut e = p + 1;
                while self.at(e) == b';' && e < self.b.len() {
                    e += 1;
                }
                self.put(p, e, L::CommentPunct);
                self.put(e, self.b.len(), L::Comment);
                Some(self.b.len())
            }
            b'#' if self.at(p + 1) == b'!' && p + 1 < self.b.len() => {
                self.put(p, p + 2, L::CommentPunct);
                self.put(p + 2, self.b.len(), L::Comment);
                Some(self.b.len())
            }
            b',' => {
                self.put(p, p + 1, L::Comma);
                Some(p + 1)
            }
            b'^' => {
                self.put(p, p + 1, L::Macro);
                self.push(EXPR);
                Some(p + 1)
            }
            _ => None,
        }
    }

    /// `match-expr` at `p` (`pop-expr` with `once`: what it opens is set in its place, and a
    /// token pops it). `None` where nothing matches.
    fn expression(&mut self, p: usize, once: bool) -> Option<usize> {
        if let Some(q) = self.noise(p) {
            return Some(q);
        }
        let c = self.b[p];
        let (e, kind) = match c {
            b')' | b']' | b'}' => (p + 1, L::Illegal),
            b'(' => {
                self.delim(p, p + 1, L::ParenBegin);
                self.open(HEAD, once);
                return Some(p + 1);
            }
            b'[' => {
                self.delim(p, p + 1, L::BracketBegin);
                self.open(VECTOR, once);
                return Some(p + 1);
            }
            b'{' => {
                self.delim(p, p + 1, L::BraceBegin);
                self.open(MAP, once);
                return Some(p + 1);
            }
            b'#' if self.at(p + 1) == b'{' && p + 1 < self.b.len() => {
                self.delim(p, p + 2, L::BraceBegin);
                self.open(MAP, once);
                return Some(p + 2);
            }
            b'"' => {
                self.put(p, p + 1, L::StrBegin);
                self.open(STRING, once);
                return Some(p + 1);
            }
            b'#' => {
                self.put(p, p + 1, L::Macro);
                self.open(DISPATCH, once);
                return Some(p + 1);
            }
            b'\'' | b'`' | b'~' | b'@' => {
                self.put(p, p + 1, L::Macro);
                self.open(EXPR, once);
                return Some(p + 1);
            }
            b'\\' => (self.character(p)?, L::Char),
            b':' if let Some(e) = self.keyword(p) => {
                self.put(p, p + 1, L::KeywordPunct);
                (e, L::Keyword)
            }
            _ => {
                if let Some(e) = self.constant(p) {
                    (e, L::ConstLang)
                } else if let Some(e) = self.number(p) {
                    if once {
                        self.pop();
                    }
                    return Some(e);
                } else {
                    // `[^\s,;…]+`: a symbol, plain
                    let e = self.run_end(p);
                    if e == p {
                        return None;
                    }
                    (e, L::Plain)
                }
            }
        };
        match kind {
            L::Plain => {}
            L::Keyword => self.put(p + 1, e, kind),
            _ => self.put(p, e, kind),
        }
        if once {
            self.pop();
        }
        Some(e)
    }

    /// `\\\S[^\s,;…#']*` at `p`: where the character ends.
    fn character(&self, p: usize) -> Option<usize> {
        let q = p + 1;
        if q >= self.b.len() || self.skip_spaces(q) > q {
            return None;
        }
        let mut e = q + super::char_len(self.b[q]);
        while e < self.b.len() && !self.at_delim(e) && !matches!(self.b[e], b'#' | b'\'') {
            e += 1;
        }
        Some(e.min(self.b.len()))
    }

    /// `(?:nil|true|false)(?=[\s,;…])` at `p`. In place: tried at most tokens, a byte's
    /// compare where a call cost more.
    #[inline(always)]
    fn constant(&self, p: usize) -> Option<usize> {
        let w: &[u8] = match self.b[p] {
            b'n' => b"nil",
            b't' => b"true",
            b'f' => b"false",
            _ => return None,
        };
        let e = p + w.len();
        (crate::starts(&self.b[p..], w) && self.at_delim(e)).then_some(e)
    }

    /// `(:):?[^:\s,;…][^\s,;…]*` at `p`: where the keyword ends.
    fn keyword(&self, p: usize) -> Option<usize> {
        let q = p + 1 + usize::from(self.at(p + 1) == b':' && p + 1 < self.b.len());
        if q >= self.b.len() || self.b[q] == b':' || self.at_delim(q) {
            return None;
        }
        Some(self.run_end(q))
    }

    /// The tails' symbol, `(?:/|[^\s,;…\d#':][^\s,;…]*)`, at `p`: where it ends.
    fn symbol(&self, p: usize) -> Option<usize> {
        self.symbol_run(p, None)
    }

    /// [`Scan::symbol`], `run` the end of the run at `p` where it's read ([`Scan::run_end`]).
    #[inline(always)]
    fn symbol_run(&self, p: usize, run: Option<usize>) -> Option<usize> {
        let c = self.b[p];
        if c == b'/' {
            return Some(p + 1);
        }
        if self.at_delim(p) || matches!(c, b'#' | b'\'' | b':') || super::digit(self.b, p).is_some()
        {
            return None;
        }
        Some(run.unwrap_or_else(|| self.run_end(p)))
    }

    /// The numbers at `p`, in the grammar's order: where the number ends, its spans pushed.
    fn number(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let q = p + usize::from(matches!(b[p], b'+' | b'-'));
        super::digit(b, q)?;
        let d = super::digits(b, q);
        let n_after = |s: &Self, i: usize| i + usize::from(s.at(i) == b'N' && i < s.b.len());
        // `[-+]?0\d+N?`, deprecated
        if b[q] == b'0' && d > q + 1 {
            let e = n_after(self, d);
            if self.at_delim(e) {
                self.put(p, e, L::Deprecated);
                return Some(e);
            }
        }
        let sign = |s: &mut Self, sign: L| {
            if q > p {
                s.put(p, q, sign);
            }
        };
        // `([-+]?)\d+(N?)`
        let e = n_after(self, d);
        if self.at_number_end(e) {
            sign(self, L::IntSign);
            self.put(q, d, L::Int);
            if e > d {
                self.put(d, e, L::IntSuffix);
            }
            return Some(e);
        }
        // `([-+]?)(0[Xx])\h+(N?)`
        if b[q] == b'0' && d == q + 1 && matches!(self.at(d), b'x' | b'X') && d < self.b.len() {
            let mut h = d + 1;
            while h < self.b.len() && b[h].is_ascii_hexdigit() {
                h += 1;
            }
            let e = n_after(self, h);
            if h > d + 1 && self.at_number_end(e) {
                sign(self, L::HexSign);
                self.put(q, d + 1, L::HexBase);
                self.put(d + 1, h, L::Hex);
                if e > h {
                    self.put(h, e, L::HexSuffix);
                }
                return Some(e);
            }
        }
        // `([-+]?)((?:[2-9]|[1-9]\d+)[Rr])[0-9A-Za-z]+`
        if matches!(self.at(d), b'r' | b'R')
            && d < self.b.len()
            && (b'1'..=b'9').contains(&b[q])
            && (d > q + 1 || b[q] >= b'2')
        {
            let mut v = d + 1;
            while v < self.b.len() && b[v].is_ascii_alphanumeric() {
                v += 1;
            }
            if v > d + 1 && self.at_number_end(v) {
                sign(self, L::RadixSign);
                self.put(q, d + 1, L::RadixBase);
                self.put(d + 1, v, L::Radix);
                return Some(v);
            }
        }
        // `([-+]?)\d+(/)\d+`
        if self.at(d) == b'/' && d < self.b.len() && super::digit(b, d + 1).is_some() {
            let r = super::digits(b, d + 1);
            if self.at_number_end(r) {
                sign(self, L::RationalSign);
                self.put(q, d, L::Rational);
                self.put(d, d + 1, L::RationalSep);
                self.put(d + 1, r, L::Rational);
                return Some(r);
            }
        }
        // `([-+]?)\d+(?:(?:(\.)\d+(?:[eE][-+]?\d+)?|(?:[eE][-+]?\d+))(M)?|(M))`
        let exponent = |s: &Self, i: usize| -> Option<usize> {
            if !matches!(s.at(i), b'e' | b'E') || i >= s.b.len() {
                return None;
            }
            let j = i + 1 + usize::from(matches!(s.at(i + 1), b'+' | b'-'));
            super::digit(s.b, j)?;
            Some(super::digits(s.b, j))
        };
        let (dot, f) = if self.at(d) == b'.' && d < self.b.len() && super::digit(b, d + 1).is_some()
        {
            let f = super::digits(b, d + 1);
            (true, exponent(self, f).unwrap_or(f))
        } else if let Some(f) = exponent(self, d) {
            (false, f)
        } else {
            (false, d)
        };
        let m = f + usize::from(self.at(f) == b'M' && f < self.b.len());
        if (f > d || m > f) && self.at_number_end(m) {
            sign(self, L::FloatSign);
            if dot {
                self.put(q, d, L::Float);
                self.put(d, d + 1, L::FloatDot);
                self.put(d + 1, f, L::Float);
            } else {
                self.put(q, f, L::Float);
            }
            if m > f {
                self.put(f, m, L::FloatSuffix);
            }
            return Some(m);
        }
        None
    }

    /// `pop-list-head`: a list's head at `p`.
    fn head(&mut self, p: usize) -> usize {
        if let Some(q) = self.noise(p) {
            return q;
        }
        let e = self.run_end(p);
        let w = &self.b[p..e];
        let starts = |s: &[u8]| crate::starts(w, s);
        // the patterns by their first byte, in the grammar's order within one: most heads are
        // none of them, and a compare is two loads a side, no `memcmp`
        let found = match w.first() {
            Some(b'd') => {
                if starts(b"defprotocol") {
                    Some((L::StorageType, PROTOCOL_NAME, e))
                } else if starts(b"definterface") {
                    Some((L::StorageType, INTERFACE_NAME, e))
                } else if starts(b"deftype") || starts(b"defrecord") {
                    Some((L::StorageType, TYPE_NAME, e))
                } else if starts(b"defmethod") {
                    Some((L::Def, FN_NAME, e))
                } else if starts(b"def") {
                    Some((L::Def, DEF_NAME, e))
                } else if starts(b"declare") {
                    Some((L::Declare, FORWARD, e))
                } else {
                    None
                }
            }
            Some(b'r') if starts(b"reify") => Some((L::Function, TYPE, e)),
            Some(b'p') if starts(b"proxy") => Some((L::Function, TYPE, e)),
            Some(b'e') if starts(b"extend-protocol") || starts(b"extend-type") => {
                Some((L::Function, TYPE, e))
            }
            Some(b'f')
                if starts(b"fn")
                    && (self.at_delim(p + 2)
                        || (self.at(p + 2) == b'*' && self.at_delim(p + 3))) =>
            {
                let e = p + 2 + usize::from(self.at(p + 2) == b'*');
                Some((L::Fn, FN_NAME, e))
            }
            _ => None,
        };
        match found {
            Some((kind, next, e)) => {
                self.put(p, e, kind);
                self.set(next);
                e
            }
            // `pop-invoke-list-tail` at once, where the next turn would start (no spaces, no
            // noise), on the run read above
            None => self.name_at(p, INVOKE, Some(e)),
        }
    }

    /// One turn of a name's frame (`pop-invoke-list-tail`, `pop-declare-*-list-tail`,
    /// `pop-fn-list-tail`): a constant or the name, then the list's tail in its place.
    fn name(&mut self, p: usize, top: u8) -> usize {
        if let Some(q) = self.noise(p) {
            return q;
        }
        self.name_at(p, top, None)
    }

    /// [`Scan::name`] past `match-noise`, `run` the end of the run at `p` where it's read
    /// ([`Scan::run_end`]).
    #[inline(always)]
    fn name_at(&mut self, p: usize, top: u8, run: Option<usize>) -> usize {
        if let Some(e) = self.constant(p) {
            self.put(p, e, L::ConstLang);
            self.set(LIST);
            return e;
        }
        let (kind, next) = match top {
            INVOKE => (L::Function, LIST),
            DEF_NAME => (L::FnName, LIST),
            INTERFACE_NAME => (L::TypeName, INTERFACE),
            FORWARD => (L::Forward, LIST),
            PROTOCOL_NAME => (L::TypeName, PROTOCOL),
            TYPE_NAME => (L::TypeName, TYPE),
            _ => (L::FnFn, LIST),
        };
        if let Some(e) = self.symbol_run(p, run) {
            let kind = if top == INVOKE {
                invoked(&self.b[p..e])
            } else {
                kind
            };
            self.put(p, e, kind);
            self.set(next);
            return e;
        }
        // `set-normal-list-tail`
        self.set(LIST);
        p
    }

    /// `pop-dispatch-expr`, right after `#`, at `p`.
    fn dispatch(&mut self, p: usize) -> usize {
        if p >= self.b.len() || self.skip_spaces(p) > p {
            self.set(TAG);
            return p;
        }
        match self.b[p] {
            b'_' => {
                self.put(p, p + 1, L::Macro);
                self.set(EXPR);
                p + 1
            }
            b'#' => {
                self.put(p, p + 1, L::Macro);
                self.set(SYMBOLIC);
                p + 1
            }
            b'\'' => {
                self.put(p, p + 1, L::Macro);
                self.pop();
                p + 1
            }
            b'"' => {
                self.put(p, p + 1, L::RegexBegin);
                self.set(REGEX_FRAME);
                self.regex = 0;
                p + 1
            }
            b'^' | b':' => {
                self.pop();
                p
            }
            _ => {
                self.set(TAG);
                p
            }
        }
    }

    /// `pop-dispatch-tag-expr` and `pop-symbolic-value` at `p`.
    fn tag(&mut self, p: usize, top: u8) -> usize {
        if let Some(q) = self.noise(p) {
            return q;
        }
        let found = if top == TAG {
            self.symbol(p).map(|e| (e, L::Macro))
        } else {
            let e = self.run_end(p);
            (e > p).then_some((e, L::Symbolic))
        };
        match found {
            Some((e, kind)) => {
                self.put(p, e, kind);
                self.pop();
                e
            }
            None => {
                self.set(EXPR);
                p
            }
        }
    }

    /// A string's text from `p`: to its end or the line's.
    fn string(&mut self, p: usize) -> usize {
        let (b, n) = (self.b, self.b.len());
        let mut from = p;
        let mut q = p;
        while q < n {
            match b[q] {
                b'"' => {
                    self.k.merge_call(&mut self.out, from, q, L::Str as u16);
                    self.put(q, q + 1, L::StrEnd);
                    self.pop();
                    return q + 1;
                }
                b'\\' if q + 1 < n => {
                    let e = q + 1 + super::char_len(b[q + 1]);
                    self.k.merge_call(&mut self.out, from, q, L::Str as u16);
                    self.put(q, e, L::StrEscape);
                    (from, q) = (e, e);
                }
                _ => q += 1,
            }
        }
        self.k.merge_call(&mut self.out, from, n, L::Str as u16);
        n
    }

    /// A regex literal from `p`: its `"`, else the regex to its next `"` or the line's end.
    fn regex(&mut self, p: usize) -> usize {
        if p < self.b.len() && self.b[p] == b'"' {
            self.put(p, p + 1, L::RegexEnd);
            self.pop();
            return p + 1;
        }
        let mut at = p;
        self.k.close(self.out);
        let r = regex::embedded(self.b, p, self.regex, &REGEX_EMBED, &mut at, self.out);
        self.k.reopen(self.out, 16);
        match r {
            Some((q, st)) => {
                self.regex = st;
                q.min(self.b.len()).max(if q > self.b.len() {
                    self.b.len()
                } else {
                    p + usize::from(q == p)
                })
            }
            None => self.b.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::marked;
    use crate::{Lang, State, lex};

    fn scope_of(lines: &[&'static str], text: &str) -> &'static str {
        marked(Lang::Clojure, lines)
            .into_iter()
            .find(|(t, _)| *t == text)
            .map_or("(none)", |(_, s)| s)
    }

    /// A run of non-delimiters sixteen bytes a step (X-039) ends where a byte a step ends it,
    /// from every place of lines holding what a step stops at but the run takes (control bytes,
    /// Unicode's letters and digits) and what ends it (Unicode's spaces, delimiters).
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn runs_step_as_bytes() {
        use super::Scan;
        use crate::emit::Push;
        let pieces = [
            "abc",
            "x1",
            "12",
            ".",
            "-",
            "*v*",
            "#",
            "'",
            ":kw",
            "\u{1}",
            "\u{1f}",
            "\u{7f}",
            "é",
            "٣",
            "\u{3000}",
            "\u{a0}",
            "\u{2028}",
            "\\",
            "|",
            "(",
            ")",
            "[",
            "]",
            "{",
            "}",
            "\t",
            " ",
            ",",
            ";",
            "\"",
            "`",
            "~",
            "@",
            "^",
            "clojure.core/some->>",
            "0123456789.0123",
        ];
        let mut spans = Vec::new();
        for line in super::super::tests::step_lines(&pieces) {
            let mut out = crate::Out::new(&mut spans);
            let s = Scan::fresh(line.as_bytes(), &State::default(), &mut out, Push);
            for p in 0..=line.len() {
                assert_eq!(s.run_end(p), s.run_end_bytes(p), "{line:?} at {p}");
            }
        }
    }

    #[test]
    fn a_defn() {
        let l = ["(defn foo [x] (str \"a\\n\" x :k ::b 1N 0x1F 2r101 1/2 1.5M nil #\"a\\d+\"))"];
        assert_eq!(scope_of(&l, "defn"), "storage.modifier.def.clojure");
        assert_eq!(scope_of(&l, "foo"), "entity.name.function.clojure");
        assert_eq!(
            scope_of(&l, "str"),
            "support.function.clojure",
            "clojure.core's"
        );
        assert_eq!(scope_of(&l, "k"), "constant.other.keyword.clojure");
        assert_eq!(scope_of(&l, "nil"), "constant.language.clojure");
        assert_eq!(scope_of(&l, "1/2"), "(none)");
        assert_eq!(
            scope_of(&l, "/"),
            "constant.numeric.rational.decimal.clojure punctuation.separator.rational.clojure"
        );
        assert_eq!(
            scope_of(&l, "\\d"),
            "string.regexp.clojure constant.other.character-class.escape.backslash.regexp"
        );
    }

    /// The `tokens` smoke's finding: a NUL byte at the top level took the top level's closer
    /// (none), a span of no scopes.
    #[test]
    fn nul_bytes_at_the_top_level() {
        let (mut spans, mut state) = (Vec::new(), State::default());
        lex(Lang::Clojure, "\0\t\0\0\0diff", &mut state, &mut spans);
        assert!(spans.is_empty(), "{spans:?}");
        assert_eq!(state, State::default());
    }

    #[test]
    fn state_across_lines() {
        let l = ["(defprotocol P", "  (m [x]))", "#\"a(", "b)\" \"s", "t\""];
        assert_eq!(scope_of(&l, "P"), "entity.name.type.clojure");
        assert_eq!(scope_of(&l, "m"), "entity.name.function.clojure");
        assert_eq!(scope_of(&l, "t"), "string.quoted.double.clojure");
        let (mut spans, mut state) = (Vec::new(), State::default());
        for line in l {
            lex(Lang::Clojure, line, &mut state, &mut spans);
        }
        assert_eq!(state, State::default());
    }
}
