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

/// The scopes kind `kind` carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
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
const KEPT: usize = 2 * crate::machine::KEPT;
/// A byte of the packed state with this bit is a run of list tails, its count in the rest.
const RUN: u8 = 0x80;

/// `[\s,;\(\)\[\]{}\"`~@\^\\]`: the delimiters (ASCII; Unicode's spaces are too).
static DELIM: [bool; 256] = crate::regex::table(b" \t\n\x0b\x0c\r,;()[]{}\"`~@^\\");

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
    let mut s = Scan {
        b,
        n: b.len(),
        out,
        stack: [TOP; MAX],
        depth: 0,
        base: 0,
        exact: state.is_exact(),
        regex: state.arg as u8,
    };
    s.unpack(state);
    s.run();
    *state = s.pack();
}

struct Scan<'b, 'o, 's> {
    b: &'b [u8],
    n: usize,
    out: &'o mut Out<'s>,
    stack: [u8; MAX],
    depth: usize,
    /// List tails below the stack.
    base: usize,
    exact: bool,
    /// An open regex literal's state.
    regex: u8,
}

impl Scan<'_, '_, '_> {
    fn unpack(&mut self, state: &State) {
        let byte = |i: usize| {
            if i < crate::machine::KEPT {
                state.levels[i]
            } else {
                state.withs[i - crate::machine::KEPT]
            }
        };
        let depth = usize::from(state.depth);
        let count = |c: u8| {
            if c & RUN != 0 {
                usize::from(c & !RUN)
            } else {
                1
            }
        };
        let total: usize = (0..depth).map(|i| count(byte(i))).sum();
        let mut skip = total.saturating_sub(MAX / 2);
        for i in 0..depth {
            let c = byte(i);
            let k = count(c);
            let below = skip.min(k);
            skip -= below;
            if c & RUN != 0 {
                self.base += below;
                for _ in below..k {
                    self.push(LIST);
                }
            } else if below > 0 {
                self.exact = false;
            } else {
                self.push(c);
            }
        }
    }

    fn pack(&self) -> State {
        let mut codes = [0u8; 2 * MAX + 64];
        let mut k = 0;
        let mut run = self.base.min(0x7f * KEPT);
        let flush = |run: &mut usize, codes: &mut [u8], k: &mut usize| {
            while *run > 0 {
                let n = (*run).min(0x7f);
                codes[*k] = RUN | n as u8;
                *k += 1;
                *run -= n;
            }
        };
        for &f in &self.stack[..self.depth] {
            if f == LIST {
                run += 1;
                continue;
            }
            flush(&mut run, &mut codes, &mut k);
            codes[k] = f;
            k += 1;
        }
        flush(&mut run, &mut codes, &mut k);
        let from = k.saturating_sub(KEPT);
        let mut state = State {
            depth: (k - from) as u8,
            inexact: !self.exact || from > 0 || self.base > 0x7f * KEPT,
            arg: if self.top() == REGEX_FRAME {
                u16::from(self.regex)
            } else {
                0
            },
            ..State::default()
        };
        for (i, &c) in codes[from..k].iter().enumerate() {
            if i < crate::machine::KEPT {
                state.levels[i] = c;
            } else {
                state.withs[i - crate::machine::KEPT] = c;
            }
        }
        state
    }

    fn top(&self) -> u8 {
        match self.depth {
            0 if self.base > 0 => LIST,
            0 => TOP,
            d => self.stack[d - 1],
        }
    }

    fn push(&mut self, f: u8) {
        if self.depth == MAX {
            if self.stack[0] == LIST {
                self.base += 1;
            } else {
                self.exact = false;
            }
            self.stack.copy_within(1.., 0);
            self.depth -= 1;
        }
        self.stack[self.depth] = f;
        self.depth += 1;
    }

    fn pop(&mut self) {
        if self.depth > 0 {
            self.depth -= 1;
        } else if self.base > 0 {
            self.base -= 1;
        }
    }

    /// The top frame replaced by `f`.
    fn set(&mut self, f: u8) {
        self.pop();
        self.push(f);
    }

    /// `f` pushed in a body, set in place of a one-shot frame.
    fn open(&mut self, f: u8, once: bool) {
        if once {
            self.set(f);
        } else {
            self.push(f);
        }
    }

    fn put(&mut self, from: usize, to: usize, kind: L) {
        self.out.push(from, to, kind as u16);
    }

    fn at(&self, p: usize) -> u8 {
        self.b.get(p).copied().unwrap_or(b'\n')
    }

    /// Whether `p` is a delimiter or the line's end (`(?=[\s,;…])`, the `\n` a space).
    fn at_delim(&self, p: usize) -> bool {
        p >= self.n
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

    /// `[^\s,;…]*` at `p`: where the run of non-delimiters ends.
    fn run_end(&self, mut p: usize) -> usize {
        while p < self.n {
            let c = self.b[p];
            if DELIM[usize::from(c)] || (c >= 0x80 && self.wide_space(p) > 0) {
                break;
            }
            p += 1;
        }
        p
    }

    fn skip_spaces(&self, mut p: usize) -> usize {
        while p < self.n {
            let c = self.b[p];
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
        (p + super::char_len(self.b[p])).min(self.n)
    }

    fn run(&mut self) {
        let mut p = 0;
        loop {
            let top = self.top();
            p = match top {
                STRING => self.string(p),
                REGEX_FRAME => self.regex(p),
                DISPATCH => self.dispatch(p),
                _ => {
                    p = self.skip_spaces(p);
                    if p >= self.n {
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
            if p >= self.n && !(self.top() == DISPATCH && p == self.n) {
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
            self.put(p, p + 1, kind);
            self.pop();
            return p + 1;
        }
        if c == b'(' && matches!(top, INTERFACE | PROTOCOL | TYPE) {
            self.put(p, p + 1, L::ParenBegin);
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

    /// `match-noise` at `p`: a comment, a `,`, or `^` (its expression pushed).
    fn noise(&mut self, p: usize) -> Option<usize> {
        match self.b[p] {
            b';' => {
                let mut e = p + 1;
                while self.at(e) == b';' && e < self.n {
                    e += 1;
                }
                self.put(p, e, L::CommentPunct);
                self.put(e, self.n, L::Comment);
                Some(self.n)
            }
            b'#' if self.at(p + 1) == b'!' && p + 1 < self.n => {
                self.put(p, p + 2, L::CommentPunct);
                self.put(p + 2, self.n, L::Comment);
                Some(self.n)
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
                self.put(p, p + 1, L::ParenBegin);
                self.open(HEAD, once);
                return Some(p + 1);
            }
            b'[' => {
                self.put(p, p + 1, L::BracketBegin);
                self.open(VECTOR, once);
                return Some(p + 1);
            }
            b'{' => {
                self.put(p, p + 1, L::BraceBegin);
                self.open(MAP, once);
                return Some(p + 1);
            }
            b'#' if self.at(p + 1) == b'{' && p + 1 < self.n => {
                self.put(p, p + 2, L::BraceBegin);
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
        if q >= self.n || self.skip_spaces(q) > q {
            return None;
        }
        let mut e = q + super::char_len(self.b[q]);
        while e < self.n && !self.at_delim(e) && !matches!(self.b[e], b'#' | b'\'') {
            e += 1;
        }
        Some(e.min(self.n))
    }

    /// `(?:nil|true|false)(?=[\s,;…])` at `p`.
    fn constant(&self, p: usize) -> Option<usize> {
        let w: &[u8] = match self.b[p] {
            b'n' => b"nil",
            b't' => b"true",
            b'f' => b"false",
            _ => return None,
        };
        let e = p + w.len();
        (self.b[p..].starts_with(w) && self.at_delim(e)).then_some(e)
    }

    /// `(:):?[^:\s,;…][^\s,;…]*` at `p`: where the keyword ends.
    fn keyword(&self, p: usize) -> Option<usize> {
        let q = p + 1 + usize::from(self.at(p + 1) == b':' && p + 1 < self.n);
        if q >= self.n || self.b[q] == b':' || self.at_delim(q) {
            return None;
        }
        Some(self.run_end(q))
    }

    /// The tails' symbol, `(?:/|[^\s,;…\d#':][^\s,;…]*)`, at `p`: where it ends.
    fn symbol(&self, p: usize) -> Option<usize> {
        let c = self.b[p];
        if c == b'/' {
            return Some(p + 1);
        }
        if self.at_delim(p) || matches!(c, b'#' | b'\'' | b':') || super::digit(self.b, p).is_some()
        {
            return None;
        }
        Some(self.run_end(p))
    }

    /// The numbers at `p`, in the grammar's order: where the number ends, its spans pushed.
    fn number(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let q = p + usize::from(matches!(b[p], b'+' | b'-'));
        super::digit(b, q)?;
        let d = super::digits(b, q);
        let n_after = |s: &Self, i: usize| i + usize::from(s.at(i) == b'N' && i < s.n);
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
        if b[q] == b'0' && d == q + 1 && matches!(self.at(d), b'x' | b'X') && d < self.n {
            let mut h = d + 1;
            while h < self.n && b[h].is_ascii_hexdigit() {
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
            && d < self.n
            && (b'1'..=b'9').contains(&b[q])
            && (d > q + 1 || b[q] >= b'2')
        {
            let mut v = d + 1;
            while v < self.n && b[v].is_ascii_alphanumeric() {
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
        if self.at(d) == b'/' && d < self.n && super::digit(b, d + 1).is_some() {
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
            if !matches!(s.at(i), b'e' | b'E') || i >= s.n {
                return None;
            }
            let j = i + 1 + usize::from(matches!(s.at(i + 1), b'+' | b'-'));
            super::digit(s.b, j)?;
            Some(super::digits(s.b, j))
        };
        let (dot, f) = if self.at(d) == b'.' && d < self.n && super::digit(b, d + 1).is_some() {
            let f = super::digits(b, d + 1);
            (true, exponent(self, f).unwrap_or(f))
        } else if let Some(f) = exponent(self, d) {
            (false, f)
        } else {
            (false, d)
        };
        let m = f + usize::from(self.at(f) == b'M' && f < self.n);
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
        let found = if w.starts_with(b"defprotocol") {
            Some((L::StorageType, PROTOCOL_NAME, e))
        } else if w.starts_with(b"definterface") {
            Some((L::StorageType, INTERFACE_NAME, e))
        } else if w.starts_with(b"deftype") || w.starts_with(b"defrecord") {
            Some((L::StorageType, TYPE_NAME, e))
        } else if w.starts_with(b"defmethod") {
            Some((L::Def, FN_NAME, e))
        } else if w.starts_with(b"def") {
            Some((L::Def, DEF_NAME, e))
        } else if w.starts_with(b"reify")
            || w.starts_with(b"proxy")
            || w.starts_with(b"extend-protocol")
            || w.starts_with(b"extend-type")
        {
            Some((L::Function, TYPE, e))
        } else if w.starts_with(b"fn")
            && (self.at_delim(p + 2) || (self.at(p + 2) == b'*' && self.at_delim(p + 3)))
        {
            let e = p + 2 + usize::from(self.at(p + 2) == b'*');
            Some((L::Fn, FN_NAME, e))
        } else if w.starts_with(b"declare") {
            Some((L::Declare, FORWARD, e))
        } else {
            None
        };
        match found {
            Some((kind, next, e)) => {
                self.put(p, e, kind);
                self.set(next);
                e
            }
            None => {
                self.set(INVOKE);
                p
            }
        }
    }

    /// One turn of a name's frame (`pop-invoke-list-tail`, `pop-declare-*-list-tail`,
    /// `pop-fn-list-tail`): a constant or the name, then the list's tail in its place.
    fn name(&mut self, p: usize, top: u8) -> usize {
        if let Some(q) = self.noise(p) {
            return q;
        }
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
        if let Some(e) = self.symbol(p) {
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
        if p >= self.n || self.skip_spaces(p) > p {
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
        let (b, n) = (self.b, self.n);
        let mut from = p;
        let mut q = p;
        while q < n {
            match b[q] {
                b'"' => {
                    self.out.push_merged(from, q, L::Str as u16);
                    self.put(q, q + 1, L::StrEnd);
                    self.pop();
                    return q + 1;
                }
                b'\\' if q + 1 < n => {
                    let e = q + 1 + super::char_len(b[q + 1]);
                    self.out.push_merged(from, q, L::Str as u16);
                    self.put(q, e, L::StrEscape);
                    (from, q) = (e, e);
                }
                _ => q += 1,
            }
        }
        self.out.push_merged(from, n, L::Str as u16);
        n
    }

    /// A regex literal from `p`: its `"`, else the regex to its next `"` or the line's end.
    fn regex(&mut self, p: usize) -> usize {
        if p < self.n && self.b[p] == b'"' {
            self.put(p, p + 1, L::RegexEnd);
            self.pop();
            return p + 1;
        }
        let mut at = p;
        match regex::embedded(self.b, p, self.regex, &REGEX_EMBED, &mut at, self.out) {
            Some((q, st)) => {
                self.regex = st;
                q.min(self.n).max(if q > self.n {
                    self.n
                } else {
                    p + usize::from(q == p)
                })
            }
            None => self.n,
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
