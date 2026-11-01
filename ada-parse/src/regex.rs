//! Regular expressions in other languages' strings (ADR-40, phase 4): one scanner with a
//! dialect per family, embedded where a host's construct makes a string a regex though bat's
//! grammar leaves it plain (Kotlin's `Regex("…")`, Java's `Pattern.compile("…")`, …), and in
//! JavaScript's regex literals.
//!
//! The kinds carry the scopes of bat's `Regular Expressions (Javascript)` (anchors, back
//! references, quantifiers, groups and assertions, character classes and their sets, escapes),
//! with a few of its own for what JavaScript lacks (inline options, atomic groups, `\Q…\E`).
//! Five dialects ([`Dialect`]):
//! - **Java's** (`java.util.regex`, Kotlin's too) adds possessive quantifiers, `(?>…)`,
//!   `(?idmsux-idmsux)` and `(?idmsux-idmsux:…)`, nested classes and `&&`, `\Q…\E`, `\pL`,
//!   `\h\H\v\V\R\X`, `\A\G\Z\z` and its escapes.
//! - **ECMAScript's** is bat's grammar, but where it is wrong: a `.` in a class is a
//!   character, not a class escape, and `\u{…}` is one escape, not `\u` and a quantifier.
//! - **RE2's** (Go's `regexp`): `(?P<name>…)`, `(?imsU-imsU)`, `[[:alpha:]]`, `\pL`,
//!   `\p{^Greek}`, `\Q…\E`, `\A\z`, octal and `\x{…}` escapes; no back references, and a `]`
//!   first in a class is a character.
//! - **Rust's** (the `regex` crate): RE2's without `\Q…\E` or octal, with nested classes and
//!   `&&`, `--`, `~~`, `(?imsRUux)`, `\b{start}`, `\<`, `\>`, `\u{…}`, `\U{…}` and
//!   `\p{sc=Greek}`.
//! - **.NET's** (`System.Text.RegularExpressions`, C#'s and PowerShell's): `(?<name>…)` and
//!   `(?'name'…)`, balancing groups (`(?<a-b>…)`, `(?<-b>…)`), `(?>…)`, `(?imnsx-imnsx)`,
//!   `(?#…)` comments, `(?(name)…)` conditionals, class subtraction (`[a-z-[aeiou]]`), `\k'name'`,
//!   `\p{IsGreek}` (braces only); a `]` first in a class is a character, and no possessive
//!   quantifiers or `\Q…\E`.
//!
//! **A host's string spells the regex.** In a cooked string (Java's `"…"`, Kotlin's `"…"`)
//! `\\` is the regex's `\`, so `"\\d+"` is `\d+`, and any other `\` starts the host's escape, a
//! character to the regex that the host colors as its own; a raw string (Kotlin's `"""…"""`)
//! is the regex as it reads. The host's patterns (its string's end, its escapes, a template)
//! come first at each place they can start, as with SQL embedded ([`crate::sql::embedded`]):
//! the scan hands those places back, and a regex token that starts before one may take it
//! (`\\\"`, an escaped quote).
//!
//! The state between the host's turns, and between lines in a host's multi-line string, is a
//! byte, seven bits: the groups open, whether a class is and one nested in it, `\Q…\E`.

use crate::Out;

// The state, seven bits: the groups open (up to 15 kept), a class open and one nested in it,
// `\Q…\E`.
pub(crate) const DEPTH: u8 = 0x0f;
pub(crate) const SET: u8 = 0x10;
const NEST: u8 = 0x20;
const NESTS: u8 = NEST;
const QUOTE: u8 = 0x40;
/// The bits of the state: a host keeping it in a byte of its own has the top bit free.
pub(crate) const STATE: u8 = 0x7f;
/// The bits of an ECMAScript regex's state: it has no nested class and no `\Q…\E`.
pub(crate) const ECMA_STATE: u8 = DEPTH | SET;
/// The bits of a .NET regex's state: it has no `\Q…\E`.
pub(crate) const NET_STATE: u8 = DEPTH | SET | NEST;

/// The bytes a token of `main` (or a group in it) starts on, and of a class.
static MAIN_BYTES: [bool; 256] = table(b"\\^$?*+{|().[");
static SET_BYTES: [bool; 256] = table(b"\\[]&~");

pub(crate) const fn table(bytes: &[u8]) -> [bool; 256] {
    let mut t = [false; 256];
    let mut i = 0;
    while i < bytes.len() {
        t[bytes[i] as usize] = true;
        i += 1;
    }
    t
}

/// Which regexes a host's are.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Dialect {
    /// `java.util.regex`, Kotlin's too.
    Java,
    /// ECMAScript's, as bat's `Regular Expressions (Javascript)` reads it.
    Ecma,
    /// RE2's, Go's `regexp`.
    Re2,
    /// The Rust `regex` crate's.
    Rust,
    /// .NET's, C#'s and PowerShell's.
    DotNet,
}

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
    Anchor = "keyword.control.anchor.regexp",
    BackReference = "keyword.other.back-reference.regexp",
    NamedBegin = "punctuation.definition.group.named.begin.regexp",
    NamedEnd = "punctuation.definition.group.named.end.regexp",
    BackReferenceName = "variable.other.group.regexp",
    Quantifier = "keyword.operator.quantifier.regexp",
    QuantifierNumber = "keyword.operator.quantifier.regexp constant.literal.numeric.regexp",
    QuantifierComma = "keyword.operator.quantifier.regexp punctuation.separator.comma.regexp",
    Or = "keyword.operator.or.regexp",
    GroupBegin = "punctuation.definition.group.begin.regexp",
    GroupEnd = "punctuation.definition.group.end.regexp",
    LookAhead = "punctuation.definition.group.assertion.regexp meta.assertion.look-ahead.regexp",
    NegativeLookAhead = "punctuation.definition.group.assertion.regexp meta.assertion.negative-look-ahead.regexp",
    LookBehind = "punctuation.definition.group.assertion.regexp meta.assertion.look-behind.regexp",
    NegativeLookBehind = "punctuation.definition.group.assertion.regexp meta.assertion.negative-look-behind.regexp",
    NoCapture = "punctuation.definition.group.no-capture.regexp",
    Atomic = "punctuation.definition.group.atomic.regexp",
    GroupName = "entity.name.other.group.regexp",
    Options = "keyword.other.option-toggle.regexp",
    ClassEscape = "constant.other.character-class.escape.backslash.regexp",
    Property = "constant.other.character-class.escape.unicode-property.regexp",
    PropertyBegin = "punctuation.definition.unicode-property.begin.regexp",
    PropertyName = "support.constant.unicode-property.regexp",
    PropertyEquals = "punctuation.separator.key-value.unicode-property.regexp",
    PropertyEnd = "punctuation.definition.unicode-property.end.regexp",
    Escape = "constant.character.escape.backslash.regexp",
    SetBegin = "punctuation.definition.character-class.begin.regexp",
    Negation = "keyword.operator.negation.regexp",
    SetEnd = "punctuation.definition.character-class.end.regexp",
    Intersection = "keyword.operator.intersection.regexp",
    Range = "constant.other.character-class.range.regexp",
    RangeClassEscape = "constant.other.character-class.range.regexp constant.other.character-class.escape.backslash.regexp",
    RangeEscape = "constant.other.character-class.range.regexp constant.character.escape.backslash.regexp",
    RangeDash = "constant.other.character-class.range.regexp punctuation.definition.range.regexp",
    PosixClass = "constant.other.character-class.posix.regexp",
    Difference = "keyword.operator.difference.regexp",
    SymmetricDifference = "keyword.operator.symmetric-difference.regexp",
    Comment = "comment.block.regexp",
    CommentBegin = "comment.block.regexp punctuation.definition.comment.begin.regexp",
    CommentEnd = "comment.block.regexp punctuation.definition.comment.end.regexp",
    Conditional = "punctuation.definition.group.assertion.conditional.regexp",
    Balancing = "keyword.operator.balancing-group.regexp",
}

/// A kind's bit for a token in a class: under `constant.other.character-class.set.regexp`.
const IN_SET: u16 = 64;
/// How many kinds there are: each leaf, then each in a class.
pub(crate) const KINDS: usize = 2 * IN_SET as usize;

/// The scopes kind `kind` carries, outermost first.
pub(crate) fn scopes(kind: u16) -> Vec<&'static str> {
    let Some(leaf) = LEAVES.get(usize::from(kind & (IN_SET - 1))) else {
        return Vec::new();
    };
    let set = if kind & IN_SET != 0 {
        "constant.other.character-class.set.regexp"
    } else {
        ""
    };
    [set, leaf]
        .into_iter()
        .flat_map(|s| s.split(' '))
        .filter(|s| !s.is_empty())
        .collect()
}

/// A regex in a host's string: what the host gives the scan.
pub(crate) struct Embed<'t> {
    pub dialect: Dialect,
    /// `\\` is the regex's `\`, and another `\` the host's escape (Java's and Kotlin's `"…"`);
    /// false where the string's text is the regex's.
    pub cooked: bool,
    /// The bytes a pattern of the host's can start on (its end, an escape, a template); the
    /// line's end is always one.
    pub yields: &'t [bool; 256],
    /// Whether a pattern of the host's but its escape matches at a byte of `yields`, given
    /// [`Embed::arg`]: a token of the regex doesn't run over it.
    pub stop: fn(&[u8], usize, usize) -> bool,
    /// What the host's [`Embed::stop`] needs beside the line (Rust's raw string's `#`s).
    pub arg: usize,
    /// Where the host's escape at `p` (a `\` that isn't a cooked `\\`) ends.
    pub escape: fn(&[u8], usize) -> usize,
    /// The host's patterns are tried only where no group or class is open: a regex literal's,
    /// whose contexts a group or a class sits above (`/[/]/` is one literal).
    pub top_only: bool,
    /// A `\` ending the line is the host's: its string goes on to the next line (JavaScript's),
    /// so no token of the regex takes it.
    pub continues: bool,
    /// Added to each of the regex's kinds: where they sit among the host's.
    pub base: u16,
    /// The kind of the regex's literal text, the host string's.
    pub text: u16,
}

/// Lex a regex embedded in a host's string, in state `state`, from `p`, where the host's
/// patterns didn't match: up to the next place they could (a byte of [`Embed::yields`] but a
/// cooked `\\`, or the line's end), or past the line. `at` is where the spans so far end. Where
/// it stopped and the state there; `None` from past the line.
pub(crate) fn embedded(
    b: &[u8],
    p: usize,
    state: u8,
    host: &Embed,
    at: &mut usize,
    out: &mut Out,
) -> Option<(usize, u8)> {
    let n = b.len();
    if p > n {
        return None;
    }
    if p == n {
        return Some((n + 1, state));
    }
    let mut s = Scan {
        b,
        n,
        out,
        from: p,
        host,
        at: *at,
        st: state,
    };
    let mut p = p;
    while p < n && !(p > s.from && s.host_at(p)) {
        p = if s.st & QUOTE != 0 {
            s.quoted(p)
        } else if s.st & SET != 0 {
            s.set(p)
        } else {
            s.main(p)
        };
    }
    // the text to where the host takes over is the regex's: a class's, in one
    s.fill(p.min(n));
    *at = s.at;
    Some((p.min(n), s.st))
}

struct Scan<'b, 'o, 's, 'e> {
    b: &'b [u8],
    n: usize,
    out: &'o mut Out<'s>,
    /// Where the scan took over from the host, whose patterns had their turn there.
    from: usize,
    host: &'e Embed<'e>,
    /// Where the spans so far end.
    at: usize,
    st: u8,
}

/// A character of the regex: its byte as the regex reads it (`\\` in a cooked string is one
/// `\`), where it ends, and whether it is the host's escape.
#[derive(Clone, Copy)]
struct Unit {
    c: u8,
    end: usize,
    host: bool,
}

impl Scan<'_, '_, '_, '_> {
    /// Bytes `s..e` are leaf `l`, in a class if one is open.
    fn tok(&mut self, s: usize, e: usize, l: L) -> usize {
        let set = if self.st & SET != 0 { IN_SET } else { 0 };
        self.put(s, e, l as u16 | set)
    }

    fn put(&mut self, s: usize, e: usize, kind: u16) -> usize {
        let e = e.min(self.n);
        self.fill(s);
        if e > s {
            self.out.push(s, e, self.host.base + kind);
            self.at = e;
        }
        e
    }

    /// The text up to `s`, after the spans so far: a class's if one is open, else the host
    /// string's.
    fn fill(&mut self, s: usize) {
        if s > self.at {
            let kind = if self.st & SET != 0 {
                self.host.base + IN_SET
            } else {
                self.host.text
            };
            if kind != 0 {
                self.out.push_merged(self.at, s, kind);
            }
            self.at = s;
        }
    }

    /// Whether the host's patterns get place `p`: a byte they start on, but a cooked `\\`.
    fn host_at(&self, p: usize) -> bool {
        let c = self.b[p];
        self.host.yields[usize::from(c)]
            && !(c == b'\\' && self.host.cooked && self.b.get(p + 1) == Some(&b'\\'))
            && !(self.host.top_only && self.st & STATE != 0)
    }

    /// Whether a pattern of the host's but its escape matches at `p`, past where the scan took
    /// over: a token of the regex doesn't run over it.
    fn host_stop(&self, p: usize) -> bool {
        let c = self.b[p];
        p > self.from
            && c != b'\\'
            && self.host.yields[usize::from(c)]
            && (self.host.stop)(self.b, p, self.host.arg)
    }

    /// Whether byte `p` is one of `bytes` or one a pattern of the host's can start on.
    fn special(&self, bytes: &[bool; 256], p: usize) -> bool {
        let c = usize::from(self.b[p]);
        bytes[c] || self.host.yields[c]
    }

    fn java(&self) -> bool {
        self.host.dialect == Dialect::Java
    }

    fn ecma(&self) -> bool {
        self.host.dialect == Dialect::Ecma
    }

    /// RE2's or Rust's.
    fn re2(&self) -> bool {
        matches!(self.host.dialect, Dialect::Re2 | Dialect::Rust)
    }

    fn rust(&self) -> bool {
        self.host.dialect == Dialect::Rust
    }

    fn dotnet(&self) -> bool {
        self.host.dialect == Dialect::DotNet
    }

    /// Java's and ECMAScript's have possessive quantifiers (ECMAScript's as bat colors them).
    fn possessive(&self) -> bool {
        matches!(self.host.dialect, Dialect::Java | Dialect::Ecma)
    }

    /// The regex's character at `p`. Past where the scan took over, a place where the host's
    /// end or template matches is theirs: none, so no token of the regex runs over it (`"\\"` is
    /// a lone `\` and the string's end, `"\\$"` an escaped `$`).
    fn unit(&self, p: usize) -> Option<Unit> {
        let &c = self.b.get(p).filter(|_| p < self.n)?;
        if self.host_stop(p) {
            return None;
        }
        if c != b'\\' || !self.host.cooked {
            return Some(Unit {
                c,
                end: p + char_len(c),
                host: false,
            });
        }
        let Some(&d) = self.b.get(p + 1) else {
            if self.host.continues {
                return None;
            }
            return Some(Unit {
                c,
                end: p + 1,
                host: true,
            });
        };
        if d == b'\\' {
            return Some(Unit {
                c: b'\\',
                end: p + 2,
                host: false,
            });
        }
        // the host's escape: a character to the regex, at least the `\` and the character after it,
        // but where the host's escape is the `\` alone and its end or `${` follows (Babel's
        // template, where `\${` is a `\` and a substitution): the regex's `\`
        let end = (self.host.escape)(self.b, p);
        if end == p + 1 && self.host_stop(p + 1) {
            return Some(Unit {
                c: b'\\',
                end,
                host: false,
            });
        }
        let end = end.max(p + 1 + char_len(d));
        // what the regex reads: a quote, `$` and the like as themselves, the rest a
        // character with no meaning to it
        let c = match d {
            b'"' | b'\'' | b'$' | b'`' => d,
            _ => 0x01,
        };
        Some(Unit {
            c,
            end: end.min(self.n),
            host: true,
        })
    }

    /// The regex's character at `p`, if it is `c` (not the host's escape): where it ends.
    fn is(&self, p: usize, c: u8) -> Option<usize> {
        self.unit(p).filter(|u| u.c == c && !u.host).map(|u| u.end)
    }

    /// One token or one character of `main` (or a group in it) at `p`: where the scan goes on.
    fn main(&mut self, p: usize) -> usize {
        // a run of characters with no meaning to the regex or the host: text
        if !self.special(&MAIN_BYTES, p) {
            let mut q = p + 1;
            while q < self.n && !self.special(&MAIN_BYTES, q) {
                q += 1;
            }
            return q;
        }
        let Some(u) = self.unit(p) else {
            return p + 1;
        };
        if u.host {
            return u.end;
        }
        let e = u.end;
        match u.c {
            b'\\' => self.backslash(p, e),
            b'^' | b'$' => self.tok(p, e, L::Anchor),
            b'?' | b'*' | b'+' => {
                let e = self.lazy(e);
                self.tok(p, e, L::Quantifier)
            }
            b'{' => self.braces(p, e).unwrap_or(e),
            b'|' => self.tok(p, e, L::Or),
            b'(' => self.group(p, e),
            b')' if self.st & DEPTH != 0 => {
                self.st -= 1;
                self.tok(p, e, L::GroupEnd)
            }
            b'.' => self.tok(p, e, L::ClassEscape),
            b'[' => self.open_set(p, e),
            _ => e,
        }
    }

    /// A quantifier's `?` (lazy) or `+` (possessive; one token in ECMAScript too, the same
    /// colors as bat's two) after it ends at `e`: where it ends. RE2, Rust and .NET have no
    /// possessive.
    fn lazy(&self, e: usize) -> usize {
        self.is(e, b'?')
            .or_else(|| self.is(e, b'+').filter(|_| self.possessive()))
            .unwrap_or(e)
    }

    /// `\{(\d+)(?:(,)(\d+)?)?\}` or `\{(,)(\d+)\}`, `{` at `p` to `e`.
    fn braces(&mut self, p: usize, e: usize) -> Option<usize> {
        let digits = |q: usize| {
            let mut r = q;
            while r < self.n && self.b[r].is_ascii_digit() {
                r += 1;
            }
            r
        };
        let d1 = digits(e);
        let (comma, d2) = match self.is(d1, b',') {
            Some(c) => (Some(d1), digits(c)),
            None => (None, d1),
        };
        // .NET's `{,n}` is text
        if d1 == e && (comma.is_none() || d2 == d1 + 1 || self.dotnet()) {
            return None;
        }
        let close = self.is(d2, b'}')?;
        let end = self.lazy(close);
        self.tok(p, e, L::Quantifier);
        self.tok(e, d1, L::QuantifierNumber);
        if let Some(c) = comma {
            self.tok(c, c + 1, L::QuantifierComma);
            self.tok(c + 1, d2, L::QuantifierNumber);
        }
        Some(self.tok(d2, end, L::Quantifier))
    }

    /// A group's opening, `(` at `p` to `e`.
    fn group(&mut self, p: usize, e: usize) -> usize {
        let open = |s: &mut Self| {
            if s.st & DEPTH < DEPTH {
                s.st += 1;
            }
        };
        if let Some(q) = self.is(e, b'?') {
            let next = self.unit(q).filter(|u| !u.host).map(|u| u.c);
            let assertion = match next {
                Some(b'=') => Some((q + 1, L::LookAhead)),
                Some(b'!') => Some((q + 1, L::NegativeLookAhead)),
                Some(b'<') => match self.unit(q + 1).filter(|u| !u.host).map(|u| u.c) {
                    Some(b'=') => Some((q + 2, L::LookBehind)),
                    Some(b'!') => Some((q + 2, L::NegativeLookBehind)),
                    _ => None,
                },
                _ => None,
            };
            if let Some((end, l)) = assertion {
                self.tok(p, e, L::GroupBegin);
                open(self);
                return self.tok(e, end, l);
            }
            match next {
                Some(b':') => {
                    self.tok(p, e, L::GroupBegin);
                    open(self);
                    return self.tok(e, q + 1, L::NoCapture);
                }
                Some(b'>') if self.java() || self.dotnet() => {
                    self.tok(p, e, L::GroupBegin);
                    open(self);
                    return self.tok(e, q + 1, L::Atomic);
                }
                Some(b'<' | b'P') => {
                    // `(?<name>`, and RE2's and Rust's `(?P<name>`
                    let lt = if next == Some(b'P') {
                        self.is(q, b'P').filter(|_| self.re2())
                    } else {
                        Some(q)
                    };
                    if let Some(lt) = lt
                        && let Some(s) = self.is(lt, b'<')
                    {
                        if self.dotnet() {
                            if let Some(r) = self.net_group(p, e, lt, s, b'>') {
                                return r;
                            }
                        } else if let Some(t) = self.group_name(s)
                            && let Some(g) = self.is(t, b'>')
                        {
                            self.tok(p, e, L::GroupBegin);
                            open(self);
                            self.tok(e, lt, L::GroupBegin);
                            self.tok(lt, s, L::NamedBegin);
                            self.tok(s, t, L::GroupName);
                            return self.tok(t, g, L::NamedEnd);
                        }
                    }
                }
                // .NET's `(?'name'`
                Some(b'\'') if self.dotnet() => {
                    if let Some(s) = self.is(q, b'\'')
                        && let Some(r) = self.net_group(p, e, q, s, b'\'')
                    {
                        return r;
                    }
                }
                // .NET's `(?#…)`, a comment to the next `)`
                Some(b'#') if self.dotnet() => return self.net_comment(p, q),
                // .NET's `(?(name)` or `(?(1)`; else `(?` before a group, the condition
                Some(b'(') if self.dotnet() => {
                    self.tok(p, e, L::GroupBegin);
                    open(self);
                    if let Some(r) = self.is(q, b'(')
                        && let Some(t) = self.group_name(r)
                        && let Some(c) = self.is(t, b')')
                    {
                        self.tok(e, r, L::Conditional);
                        self.tok(r, t, L::BackReferenceName);
                        return self.tok(t, c, L::Conditional);
                    }
                    return self.tok(e, q, L::Conditional);
                }
                _ => {}
            }
            if !self.ecma() {
                // `(?idmsuxU-idmsuxU)` and `(?idmsuxU-idmsuxU:`, RE2's `imsU`, Rust's `imsRUux`
                {
                    let flags: &[u8] = match self.host.dialect {
                        Dialect::Re2 => b"imsU",
                        Dialect::Rust => b"imsRUux",
                        Dialect::DotNet => b"imnsx",
                        _ => b"idmsuxU",
                    };
                    let flag = |c: u8| flags.contains(&c);
                    let mut t = q;
                    while t < self.n && flag(self.b[t]) {
                        t += 1;
                    }
                    if self.b.get(t) == Some(&b'-') {
                        t += 1;
                        while t < self.n && flag(self.b[t]) {
                            t += 1;
                        }
                    }
                    if t > q {
                        if let Some(c) = self.is(t, b')') {
                            self.tok(p, e, L::GroupBegin);
                            self.tok(e, t, L::Options);
                            return self.tok(t, c, L::GroupEnd);
                        }
                        if let Some(c) = self.is(t, b':') {
                            self.tok(p, e, L::GroupBegin);
                            open(self);
                            self.tok(e, t, L::Options);
                            return self.tok(t, c, L::NoCapture);
                        }
                    }
                }
            }
        }
        open(self);
        self.tok(p, e, L::GroupBegin)
    }

    /// A group's name from `s`: Java's `[a-zA-Z][a-zA-Z0-9]*`, ECMAScript's
    /// `[_$\p{L}][_$\p{L}\p{N}]*` (any character past ASCII taken as a letter), RE2's
    /// `\w+`, Rust's `[_a-zA-Z][_a-zA-Z0-9.\[\]]*`, .NET's `\w+` (past ASCII a letter). Where
    /// it ends.
    fn group_name(&self, s: usize) -> Option<usize> {
        type Class = fn(u8) -> bool;
        let (first, rest): (Class, Class) = match self.host.dialect {
            Dialect::Java => (|c| c.is_ascii_alphabetic(), |c| c.is_ascii_alphanumeric()),
            Dialect::Ecma => (
                |c| c.is_ascii_alphabetic() || matches!(c, b'_' | b'$' | 0x80..),
                |c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'$' | 0x80..),
            ),
            Dialect::Re2 => (
                |c| c.is_ascii_alphanumeric() || c == b'_',
                |c| c.is_ascii_alphanumeric() || c == b'_',
            ),
            Dialect::Rust => (
                |c| c.is_ascii_alphabetic() || c == b'_',
                |c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'[' | b']'),
            ),
            Dialect::DotNet => (
                |c| c.is_ascii_alphanumeric() || matches!(c, b'_' | 0x80..),
                |c| c.is_ascii_alphanumeric() || matches!(c, b'_' | 0x80..),
            ),
        };
        // a name doesn't run over the host's end (PowerShell's `’`, a byte past ASCII)
        if s >= self.n || !first(self.b[s]) || self.host_stop(s) {
            return None;
        }
        let mut t = s + 1;
        while t < self.n && rest(self.b[t]) && !self.host_stop(t) {
            t += 1;
        }
        Some(t)
    }

    /// `[` at `p` to `e`, or `[^`.
    fn open_set(&mut self, p: usize, e: usize) -> usize {
        // the text before it isn't the class's
        self.fill(p);
        self.st |= SET;
        self.tok(p, e, L::SetBegin);
        let e = match self.is(e, b'^') {
            Some(n) => self.tok(e, n, L::Negation),
            None => e,
        };
        // RE2's, Rust's and .NET's `[]a]`: a `]` first is a character
        match self.is(e, b']').filter(|_| self.re2() || self.dotnet()) {
            Some(n) => n,
            None => e,
        }
    }

    /// One token or one character in a class at `p`.
    fn set(&mut self, p: usize) -> usize {
        // a character with no meaning to the class or the host, and no range's `-` after it
        // (nor .NET's `-[`, a subtraction)
        if !self.special(&SET_BYTES, p) {
            let e = p + char_len(self.b[p]);
            match self.b.get(e) {
                Some(b'-') => {}
                Some(b'[') if self.b[p] == b'-' => {}
                _ => return e,
            }
        }
        let Some(u) = self.unit(p) else {
            return p + 1;
        };
        if u.host {
            return self.range(p, u).unwrap_or(u.end);
        }
        let e = u.end;
        match u.c {
            b']' => {
                let r = self.tok(p, e, L::SetEnd);
                if self.st & NESTS != 0 {
                    self.st -= NEST;
                } else {
                    self.st &= !SET;
                }
                r
            }
            b'[' if self.re2()
                && let Some(q) = self.is(e, b':').and_then(|c| self.posix(c)) =>
            {
                self.tok(p, q, L::PosixClass)
            }
            b'[' if self.java() || self.rust() => {
                if self.st & NESTS != NESTS {
                    self.st += NEST;
                }
                self.tok(p, e, L::SetBegin);
                match self.is(e, b'^') {
                    Some(n) => self.tok(e, n, L::Negation),
                    None => e,
                }
            }
            b'&' if (self.java() || self.rust()) && self.is(e, b'&').is_some() => {
                self.tok(p, e + 1, L::Intersection)
            }
            b'-' if self.rust() && self.is(e, b'-').is_some() => self.tok(p, e + 1, L::Difference),
            b'~' if self.rust() && self.is(e, b'~').is_some() => {
                self.tok(p, e + 1, L::SymmetricDifference)
            }
            // .NET's `-[…]`, a class subtracted
            b'-' if self.dotnet()
                && let Some(n) = self.is(e, b'[') =>
            {
                self.tok(p, e, L::Difference);
                if self.st & NESTS != NESTS {
                    self.st += NEST;
                }
                self.tok(e, n, L::SetBegin);
                match self.is(n, b'^') {
                    Some(m) => self.tok(n, m, L::Negation),
                    None => n,
                }
            }
            _ => {
                if let Some(r) = self.range(p, u) {
                    return r;
                }
                if u.c == b'\\' {
                    return self.backslash(p, e);
                }
                e
            }
        }
    }

    /// A range in a class, its first character `u` at `p`: where it ends.
    fn range(&mut self, p: usize, u: Unit) -> Option<usize> {
        // an end: a class escape, an escape, or a character but `]`
        let end = |s: &Self, q: usize, first: bool| -> Option<(usize, Option<L>)> {
            let v = s.unit(q)?;
            if !v.host && v.c == b'\\' {
                let Some(w) = s.unit(v.end) else {
                    // ECMAScript: a `\` at the line's end is a character
                    return (!s.java()).then_some((v.end, None));
                };
                let classes: &[u8] = if s.java() { b"wWsSdDhHvV" } else { b"wWsSdD" };
                if !w.host && classes.contains(&w.c) {
                    return Some((w.end, Some(L::RangeClassEscape)));
                }
                let e = if s.java() || w.host {
                    w.end
                } else {
                    s.ecma_escape(w.end, w.c)
                };
                return Some((e, Some(L::RangeEscape)));
            }
            if v.host {
                return Some((v.end, None));
            }
            if !first && v.c == b']' {
                return None;
            }
            Some((v.end, None))
        };
        let (d, l1) = if u.host {
            (u.end, None)
        } else {
            end(self, p, true)?
        };
        let dash = self.is(d, b'-')?;
        // Rust's `--` is a difference, not a range's dash, and .NET's `-[` a subtraction
        if (self.rust() && self.is(dash, b'-').is_some())
            || (self.dotnet() && self.is(dash, b'[').is_some())
        {
            return None;
        }
        let (e, l2) = end(self, dash, false)?;
        self.tok(p, d, l1.unwrap_or(L::Range));
        self.tok(d, dash, L::RangeDash);
        Some(self.tok(dash, e, l2.unwrap_or(L::Range)))
    }

    /// `[:alpha:]` or `[:^alpha:]` in a class, from `s` past its `[:`: where it ends.
    fn posix(&self, s: usize) -> Option<usize> {
        let w = if self.b.get(s) == Some(&b'^') {
            s + 1
        } else {
            s
        };
        let mut t = w;
        while t < self.n && self.b[t].is_ascii_lowercase() {
            t += 1;
        }
        (t > w && self.b[t..self.n].starts_with(b":]")).then_some(t + 2)
    }

    /// `\…`, its `\` at `p` to `e`: where it ends.
    fn backslash(&mut self, p: usize, e: usize) -> usize {
        if self.re2() {
            return self.backslash_re2(p, e);
        }
        if self.dotnet() {
            return self.backslash_net(p, e);
        }
        if !self.java() {
            return self.backslash_ecma(p, e);
        }
        let Some(u) = self.unit(e) else {
            // at the line's end, or before the host's end or template
            return self.tok(p, e, L::Escape);
        };
        if u.host {
            return self.tok(p, u.end, L::Escape);
        }
        let n = u.end;
        let in_set = self.st & SET != 0;
        match u.c {
            b'b' | b'B' | b'A' | b'G' | b'Z' | b'z' if !in_set => self.tok(p, n, L::Anchor),
            b'1'..=b'9' if !in_set => {
                let mut q = n;
                while q < self.n && self.b[q].is_ascii_digit() {
                    q += 1;
                }
                self.tok(p, q, L::BackReference)
            }
            b'k' if !in_set => {
                if let Some(s) = self.is(n, b'<')
                    && self.b.get(s).is_some_and(u8::is_ascii_alphabetic)
                {
                    let mut t = s + 1;
                    while t < self.n && self.b[t].is_ascii_alphanumeric() {
                        t += 1;
                    }
                    if let Some(g) = self.is(t, b'>') {
                        self.tok(p, n, L::BackReference);
                        self.tok(n, s, L::NamedBegin);
                        self.tok(s, t, L::BackReferenceName);
                        return self.tok(t, g, L::NamedEnd);
                    }
                }
                self.tok(p, n, L::Escape)
            }
            b'w' | b'W' | b's' | b'S' | b'd' | b'D' | b'h' | b'H' | b'v' | b'V' => {
                self.tok(p, n, L::ClassEscape)
            }
            b'R' | b'X' if !in_set => self.tok(p, n, L::ClassEscape),
            b'p' | b'P' => self.property(p, n),
            b'Q' => {
                self.st |= QUOTE;
                self.tok(p, n, L::Escape)
            }
            b'0' => {
                let mut q = n;
                while q < n + 3 && q < self.n && matches!(self.b[q], b'0'..=b'7') {
                    q += 1;
                }
                self.tok(p, q, L::Escape)
            }
            b'x' => {
                let hex = |q: usize| self.b.get(q).is_some_and(u8::is_ascii_hexdigit);
                if self.b.get(n) == Some(&b'{')
                    && let Some(c) = (n + 1..self.n).find(|&q| !hex(q))
                    && c > n + 1
                    && self.b[c] == b'}'
                {
                    return self.tok(p, c + 1, L::Escape);
                }
                let q = if hex(n) && hex(n + 1) { n + 2 } else { n };
                self.tok(p, q, L::Escape)
            }
            b'u' => {
                let hex = |q: usize| self.b.get(q).is_some_and(u8::is_ascii_hexdigit);
                let q = if (n..n + 4).all(hex) { n + 4 } else { n };
                self.tok(p, q, L::Escape)
            }
            b'c' => {
                let q = self.unit(n).map_or(n, |v| v.end);
                self.tok(p, q, L::Escape)
            }
            b'N' if self.b.get(n) == Some(&b'{') => {
                let q = (n + 1..self.n)
                    .take_while(|&q| self.unit(q).is_some_and(|u| !u.host))
                    .find(|&q| self.b[q] == b'}')
                    .map_or(n, |c| c + 1);
                self.tok(p, q, L::Escape)
            }
            _ => self.tok(p, n, L::Escape),
        }
    }

    /// `\…` in ECMAScript, its `\` at `p` to `e`: where it ends.
    #[inline(never)]
    fn backslash_ecma(&mut self, p: usize, e: usize) -> usize {
        let Some(u) = self.unit(e) else {
            // at the line's end no pattern takes a lone `\` in a literal; in a string, and before
            // the host's end or `${`, it is an escape (of what the substitution starts with)
            if e < self.n || self.host.cooked {
                return self.tok(p, e, L::Escape);
            }
            return e;
        };
        if u.host {
            return self.tok(p, u.end, L::Escape);
        }
        let n = u.end;
        let in_set = self.st & SET != 0;
        match u.c {
            b'b' | b'B' if !in_set => self.tok(p, n, L::Anchor),
            b'1'..=b'9' if !in_set => {
                let mut q = n;
                while q < self.n && self.b[q].is_ascii_digit() {
                    q += 1;
                }
                self.tok(p, q, L::BackReference)
            }
            b'k' if !in_set => {
                if let Some(s) = self.is(n, b'<')
                    && let Some(t) = self.group_name(s)
                    && let Some(g) = self.is(t, b'>')
                {
                    self.tok(p, n, L::BackReference);
                    self.tok(n, s, L::NamedBegin);
                    self.tok(s, t, L::BackReferenceName);
                    return self.tok(t, g, L::NamedEnd);
                }
                self.tok(p, n, L::Escape)
            }
            b'w' | b'W' | b's' | b'S' | b'd' | b'D' => self.tok(p, n, L::ClassEscape),
            b'p' | b'P' => self.property(p, n),
            c => {
                let q = self.ecma_escape(n, c);
                self.tok(p, q, L::Escape)
            }
        }
    }

    /// `\…` in RE2 or Rust, its `\` at `p` to `e`: where it ends.
    #[inline(never)]
    fn backslash_re2(&mut self, p: usize, e: usize) -> usize {
        let Some(u) = self.unit(e) else {
            return self.tok(p, e, L::Escape);
        };
        if u.host {
            return self.tok(p, u.end, L::Escape);
        }
        let n = u.end;
        let in_set = self.st & SET != 0;
        let rust = self.rust();
        let hex = |q: usize| q < self.n && self.b[q].is_ascii_hexdigit();
        // `{H…}`, from `n`: where it ends
        let braced = |s: &Self, n: usize| {
            if s.b.get(n) != Some(&b'{') {
                return None;
            }
            let c = (n + 1..s.n).find(|&q| !hex(q))?;
            (c > n + 1 && s.b[c] == b'}').then_some(c + 1)
        };
        match u.c {
            b'A' | b'z' | b'b' | b'B' if !in_set => {
                // Rust's `\b{start}`, `\b{end}`, `\b{start-half}`, `\b{end-half}`
                if rust && u.c == b'b' && self.b.get(n) == Some(&b'{') {
                    for w in [&b"{start}"[..], b"{end}", b"{start-half}", b"{end-half}"] {
                        if self.b[n..self.n].starts_with(w) {
                            return self.tok(p, n + w.len(), L::Anchor);
                        }
                    }
                }
                self.tok(p, n, L::Anchor)
            }
            b'<' | b'>' if rust && !in_set => self.tok(p, n, L::Anchor),
            b'w' | b'W' | b's' | b'S' | b'd' | b'D' => self.tok(p, n, L::ClassEscape),
            b'p' | b'P' => self.property(p, n),
            b'Q' if !rust => {
                self.st |= QUOTE;
                self.tok(p, n, L::Escape)
            }
            b'0'..=b'7' if !rust => {
                let mut q = n;
                while q < n + 2 && q < self.n && matches!(self.b[q], b'0'..=b'7') {
                    q += 1;
                }
                self.tok(p, q, L::Escape)
            }
            b'x' => {
                let q = braced(self, n).unwrap_or(if hex(n) && hex(n + 1) { n + 2 } else { n });
                self.tok(p, q, L::Escape)
            }
            b'u' | b'U' if rust => {
                let len = if u.c == b'u' { 4 } else { 8 };
                let q = braced(self, n).unwrap_or(if (n..n + len).all(hex) { n + len } else { n });
                self.tok(p, q, L::Escape)
            }
            _ => self.tok(p, n, L::Escape),
        }
    }

    /// `\…` in .NET, its `\` at `p` to `e`: where it ends.
    #[inline(never)]
    fn backslash_net(&mut self, p: usize, e: usize) -> usize {
        let Some(u) = self.unit(e) else {
            return self.tok(p, e, L::Escape);
        };
        if u.host {
            return self.tok(p, u.end, L::Escape);
        }
        let n = u.end;
        let in_set = self.st & SET != 0;
        let hex = |q: usize| q < self.n && self.b[q].is_ascii_hexdigit();
        match u.c {
            b'b' | b'B' | b'A' | b'G' | b'Z' | b'z' if !in_set => self.tok(p, n, L::Anchor),
            b'1'..=b'9' if !in_set => {
                let mut q = n;
                while q < self.n && self.b[q].is_ascii_digit() {
                    q += 1;
                }
                self.tok(p, q, L::BackReference)
            }
            // `\k<name>`, `\k'name'`, `\k<1>`
            b'k' if !in_set => {
                let close = match self.unit(n).filter(|v| !v.host).map(|v| v.c) {
                    Some(b'<') => b'>',
                    Some(b'\'') => b'\'',
                    _ => return self.tok(p, n, L::Escape),
                };
                if let Some(t) = self.group_name(n + 1)
                    && let Some(g) = self.is(t, close)
                {
                    self.tok(p, n, L::BackReference);
                    self.tok(n, n + 1, L::NamedBegin);
                    self.tok(n + 1, t, L::BackReferenceName);
                    return self.tok(t, g, L::NamedEnd);
                }
                self.tok(p, n, L::Escape)
            }
            b'w' | b'W' | b's' | b'S' | b'd' | b'D' => self.tok(p, n, L::ClassEscape),
            b'p' | b'P' => self.property(p, n),
            b'0' => {
                let mut q = n;
                while q < n + 2 && q < self.n && matches!(self.b[q], b'0'..=b'7') {
                    q += 1;
                }
                self.tok(p, q, L::Escape)
            }
            b'x' => {
                let q = if hex(n) && hex(n + 1) { n + 2 } else { n };
                self.tok(p, q, L::Escape)
            }
            b'u' => {
                let q = if (n..n + 4).all(hex) { n + 4 } else { n };
                self.tok(p, q, L::Escape)
            }
            b'c' if n < self.n && self.b[n].is_ascii_alphabetic() => self.tok(p, n + 1, L::Escape),
            _ => self.tok(p, n, L::Escape),
        }
    }

    /// .NET's named group, `(` at `p` to `e`, its `<` or `'` at `lt` to `s`: a name, or a
    /// balancing group's (`<a-b>`, `<-b>`), then `close`. Where it ends, if it is one.
    fn net_group(&mut self, p: usize, e: usize, lt: usize, s: usize, close: u8) -> Option<usize> {
        let t = self.group_name(s).unwrap_or(s);
        let bal = self
            .is(t, b'-')
            .and_then(|d| self.group_name(d).map(|t2| (d, t2)));
        let end = bal.map_or(t, |(_, t2)| t2);
        if t == s && bal.is_none() {
            return None;
        }
        let g = self.is(end, close)?;
        self.tok(p, e, L::GroupBegin);
        if self.st & DEPTH < DEPTH {
            self.st += 1;
        }
        self.tok(e, lt, L::GroupBegin);
        self.tok(lt, s, L::NamedBegin);
        self.tok(s, t, L::GroupName);
        if let Some((d, t2)) = bal {
            self.tok(t, d, L::Balancing);
            self.tok(d, t2, L::BackReferenceName);
        }
        Some(self.tok(end, g, L::NamedEnd))
    }

    /// .NET's `(?#…)`, `(` at `p`, its `#` at `q`: a comment to the next `)`, or to where the
    /// host takes over. Where it ends.
    fn net_comment(&mut self, p: usize, q: usize) -> usize {
        let mut t = q + 1;
        while let Some(u) = self.unit(t) {
            if !u.host && u.c == b')' {
                self.tok(p, q + 1, L::CommentBegin);
                self.tok(q + 1, t, L::Comment);
                return self.tok(t, u.end, L::CommentEnd);
            }
            t = u.end;
        }
        self.tok(p, q + 1, L::CommentBegin);
        self.tok(q + 1, t, L::Comment)
    }

    /// Where ECMAScript's escape `\c…` ends, `c` the character after its `\`, ending at `n`:
    /// `\cX`, `\xHH`, `\uHHHH` and `\u{H…}` take more.
    fn ecma_escape(&self, n: usize, c: u8) -> usize {
        let hex = |q: usize| q < self.n && self.b[q].is_ascii_hexdigit();
        match c {
            b'c' if n < self.n && self.b[n].is_ascii_uppercase() => n + 1,
            b'x' if hex(n) && hex(n + 1) => n + 2,
            b'u' => {
                if self.b.get(n) == Some(&b'{')
                    && let Some(c) = (n + 1..self.n).find(|&q| !hex(q))
                    && c > n + 1
                    && self.b[c] == b'}'
                {
                    return c + 1;
                }
                if (n..n + 4).all(hex) { n + 4 } else { n }
            }
            _ => n,
        }
    }

    /// `\p` or `\P` at `p` to `e`: `{name}`, `{key=value}`, or (but ECMAScript's) a letter.
    fn property(&mut self, p: usize, e: usize) -> usize {
        // Java's and RE2's `[a-zA-Z0-9_]+`, ECMAScript's `[\p{L}_]+`, Rust's with ` `, `-`, `.`,
        // .NET's with `-`
        let dialect = self.host.dialect;
        let name = |s: &Self, q: usize| {
            let mut t = q;
            while t < s.n
                && (s.b[t].is_ascii_alphabetic()
                    || s.b[t] == b'_'
                    || match dialect {
                        Dialect::Ecma => s.b[t] >= 0x80,
                        Dialect::Rust => {
                            s.b[t].is_ascii_digit() || matches!(s.b[t], b' ' | b'-' | b'.')
                        }
                        // `IsLatin-1Supplement`
                        Dialect::DotNet => s.b[t].is_ascii_digit() || s.b[t] == b'-',
                        _ => s.b[t].is_ascii_digit(),
                    })
            {
                t += 1;
            }
            t
        };
        if let Some(o) = self.is(e, b'{') {
            // RE2's `\p{^Greek}`
            let neg = self.is(o, b'^').filter(|_| self.re2());
            let k0 = neg.unwrap_or(o);
            let k = name(self, k0);
            if k > k0 {
                // `=`, and Rust's `:` and `!=`
                let sep = match self.b.get(k) {
                    Some(b'=') => Some(k + 1),
                    Some(b':') if self.rust() => Some(k + 1),
                    Some(b'!') if self.rust() && self.b.get(k + 1) == Some(&b'=') => Some(k + 2),
                    _ => None,
                };
                let (eq, v) = match sep {
                    Some(s) => (Some((k, s)), name(self, s)),
                    None => (None, k),
                };
                if let Some(c) = self.is(v, b'}') {
                    self.tok(p, e, L::Property);
                    self.tok(e, o, L::PropertyBegin);
                    self.tok(o, k0, L::Negation);
                    self.tok(k0, k, L::PropertyName);
                    if let Some((eq, s)) = eq {
                        self.tok(eq, s, L::PropertyEquals);
                        self.tok(s, v, L::PropertyName);
                    }
                    return self.tok(v, c, L::PropertyEnd);
                }
            }
            return self.tok(p, e, L::Escape);
        }
        if !self.ecma() && !self.dotnet() && self.b.get(e).is_some_and(u8::is_ascii_alphabetic) {
            self.tok(p, e, L::Property);
            return self.tok(e, e + 1, L::PropertyName);
        }
        self.tok(p, e, L::Escape)
    }

    /// In `\Q…\E` at `p`: to `\E`, which ends it, or a character on.
    fn quoted(&mut self, p: usize) -> usize {
        let Some(u) = self.unit(p) else {
            return p + 1;
        };
        if !u.host
            && u.c == b'\\'
            && let Some(e) = self.is(u.end, b'E')
        {
            self.st &= !QUOTE;
            return self.tok(p, e, L::Escape);
        }
        u.end
    }
}

/// Where a Java or Kotlin string's escape at `p` ends: `\uXXXX` is six bytes, the rest a
/// character after the `\`.
pub(crate) fn java_escape(b: &[u8], p: usize) -> usize {
    let hex = |q: usize| b.get(q).is_some_and(u8::is_ascii_hexdigit);
    match b.get(p + 1) {
        Some(b'u') if (2..6).all(|i| hex(p + i)) => p + 6,
        Some(&d) => p + 1 + char_len(d),
        None => p + 1,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `text`'s tokens as a cooked (or raw) Java string's regex, `(text, scopes)`.
    fn lex(text: &str, cooked: bool) -> Vec<(&str, String)> {
        lex_in(text, cooked, Dialect::Java)
    }

    /// `text`'s tokens as a regex of `dialect` in a cooked (or raw) string, `(text, scopes)`.
    fn lex_in(text: &str, cooked: bool, dialect: Dialect) -> Vec<(&str, String)> {
        // a Java string's end and escapes; a JavaScript literal's `/`
        let java = dialect == Dialect::Java;
        let yields = {
            let mut t = [false; 256];
            if java {
                t[usize::from(b'"')] = true;
                t[usize::from(b'\\')] = true;
            } else {
                t[usize::from(b'/')] = true;
            }
            t
        };
        let host = Embed {
            dialect,
            cooked,
            yields: &yields,
            stop: |b, p, _| b[p] == b'"',
            arg: 0,
            escape: java_escape,
            top_only: !java,
            continues: false,
            base: 0,
            text: 0,
        };
        let b = text.as_bytes();
        let mut spans = Vec::new();
        let mut out = Out(&mut spans);
        let (mut p, mut st, mut at) = (0, 0, 0);
        while let Some((q, s)) = embedded(b, p, st, &host, &mut at, &mut out) {
            // the host's turn: its escape is two bytes, here unmarked
            (p, st) = (q, s);
            if p < b.len() && b[p] == b'\\' {
                p += 2;
                at = p;
            } else {
                p += 1;
            }
        }
        spans
            .iter()
            .map(|s| {
                (
                    &text[s.start as usize..s.end as usize],
                    scopes(s.kind).join(" "),
                )
            })
            .collect()
    }

    fn has(t: &[(&str, String)], text: &str, scope: &str) {
        assert!(
            t.iter().any(|(x, s)| *x == text && s == scope),
            "{text:?} {scope:?} in {t:?}"
        );
    }

    #[test]
    fn a_cooked_strings_regex() {
        let t = lex(
            r#"^(?<year>\\d{4})-(?:\\w+|[a-z&&[^q]])*+\\.\\p{Lu}\\k<year>$"#,
            true,
        );
        has(&t, "^", "keyword.control.anchor.regexp");
        has(&t, "year", "entity.name.other.group.regexp");
        has(
            &t,
            r"\\d",
            "constant.other.character-class.escape.backslash.regexp",
        );
        has(
            &t,
            "4",
            "keyword.operator.quantifier.regexp constant.literal.numeric.regexp",
        );
        has(&t, "?:", "punctuation.definition.group.no-capture.regexp");
        has(&t, "|", "keyword.operator.or.regexp");
        has(
            &t,
            "a",
            "constant.other.character-class.set.regexp constant.other.character-class.range.regexp",
        );
        has(
            &t,
            "&&",
            "constant.other.character-class.set.regexp keyword.operator.intersection.regexp",
        );
        has(&t, "*+", "keyword.operator.quantifier.regexp");
        has(&t, r"\\.", "constant.character.escape.backslash.regexp");
        has(&t, "Lu", "support.constant.unicode-property.regexp");
        has(&t, r"\\k", "keyword.other.back-reference.regexp");
        has(&t, "$", "keyword.control.anchor.regexp");
    }

    #[test]
    fn the_hosts_escapes_and_quotes() {
        // `\"` and `\n` are the host's; `\\\"` is the regex's escaped quote
        let t = lex(r#"a\"b\n(\\\")\\Q*+\\E{2,}"#, true);
        assert!(!t.iter().any(|(x, _)| *x == r#"\""#), "{t:?}");
        has(&t, r#"\\\""#, "constant.character.escape.backslash.regexp");
        has(&t, r"\\Q", "constant.character.escape.backslash.regexp");
        assert!(!t.iter().any(|(x, _)| *x == "*+"), "{t:?}");
        has(&t, "{", "keyword.operator.quantifier.regexp");
        // raw: `\d` as it reads
        let t = lex(r"\d+(?i)x", false);
        has(
            &t,
            r"\d",
            "constant.other.character-class.escape.backslash.regexp",
        );
        has(&t, "?i", "keyword.other.option-toggle.regexp");
    }

    #[test]
    fn re2s_and_rusts_dialects() {
        fn lex_raw(text: &str, dialect: Dialect) -> Vec<(&str, String)> {
            lex_in(text, false, dialect)
        }
        let t = lex_raw(
            r"(?P<n>a)(?<m>b)(?imsU-i:c)[[:^digit:]][]a]*+\Q*\E\101\pN\p{^L}",
            Dialect::Re2,
        );
        has(&t, "?P", "punctuation.definition.group.begin.regexp");
        has(&t, "n", "entity.name.other.group.regexp");
        has(&t, "?imsU-i", "keyword.other.option-toggle.regexp");
        has(
            &t,
            "[:^digit:]",
            "constant.other.character-class.set.regexp constant.other.character-class.posix.regexp",
        );
        // `]` first is a character; no possessive quantifier
        has(&t, "]a", "constant.other.character-class.set.regexp");
        has(&t, "*", "keyword.operator.quantifier.regexp");
        has(&t, r"\Q", "constant.character.escape.backslash.regexp");
        has(&t, r"\101", "constant.character.escape.backslash.regexp");
        has(&t, "N", "support.constant.unicode-property.regexp");
        has(&t, "^", "keyword.operator.negation.regexp");
        let t = lex_raw(
            r"[a-y&&xyz--[^b]~~c][a--b]\b{end}\<\U{41}\u0041\Q(?R)",
            Dialect::Rust,
        );
        has(
            &t,
            "[",
            "constant.other.character-class.set.regexp punctuation.definition.character-class.begin.regexp",
        );
        has(
            &t,
            "--",
            "constant.other.character-class.set.regexp keyword.operator.difference.regexp",
        );
        has(
            &t,
            "~~",
            "constant.other.character-class.set.regexp keyword.operator.symmetric-difference.regexp",
        );
        // `a--b` is a difference, not a range
        assert!(!t.iter().any(|(x, _)| *x == "a-"), "{t:?}");
        has(&t, r"\b{end}", "keyword.control.anchor.regexp");
        has(&t, r"\<", "keyword.control.anchor.regexp");
        has(&t, r"\U{41}", "constant.character.escape.backslash.regexp");
        has(&t, r"\u0041", "constant.character.escape.backslash.regexp");
        // no `\Q…\E` in Rust's; its `R` option
        has(&t, r"\Q", "constant.character.escape.backslash.regexp");
        has(&t, "?R", "keyword.other.option-toggle.regexp");
    }

    #[test]
    fn dotnets_dialect() {
        let t = lex_in(
            r"(?<a-b>x)(?'-b'y)(?#c)(?(a)z|w)(?(?=q)r)[]a-z-[^aeiou]]{,3}*+\k'a'\k<1>\pL\p{IsGreek}\Q",
            false,
            Dialect::DotNet,
        );
        let set = "constant.other.character-class.set.regexp";
        has(&t, "a", "entity.name.other.group.regexp");
        has(&t, "-", "keyword.operator.balancing-group.regexp");
        has(&t, "b", "variable.other.group.regexp");
        has(&t, "'", "punctuation.definition.group.named.begin.regexp");
        has(&t, "c", "comment.block.regexp");
        has(
            &t,
            "?(",
            "punctuation.definition.group.assertion.conditional.regexp",
        );
        has(
            &t,
            "?",
            "punctuation.definition.group.assertion.conditional.regexp",
        );
        // `]` first is a character; `-[` a subtraction, its class negated
        has(&t, "]", set);
        has(
            &t,
            "-",
            &format!("{set} keyword.operator.difference.regexp"),
        );
        has(&t, "^", &format!("{set} keyword.operator.negation.regexp"));
        // `{,3}` is text, `*+` two quantifiers; `\pL` and `\Q` are escapes
        assert!(!t.iter().any(|(x, _)| *x == "*+" || *x == "3"), "{t:?}");
        has(&t, "+", "keyword.operator.quantifier.regexp");
        has(&t, r"\k", "keyword.other.back-reference.regexp");
        has(&t, "1", "variable.other.group.regexp");
        for e in [r"\p", r"\Q"] {
            has(&t, e, "constant.character.escape.backslash.regexp");
        }
        has(&t, "IsGreek", "support.constant.unicode-property.regexp");
    }

    #[test]
    fn ecmascripts_dialect() {
        let t = lex_in(
            r"\cA\cz\x41\x4\0123\N[\x41-\u005A.\w-][-\d]\h\R\Q",
            false,
            Dialect::Ecma,
        );
        let esc = "constant.character.escape.backslash.regexp";
        for e in [
            r"\cA", r"\c", r"\x41", r"\x", r"\0", r"\N", r"\h", r"\R", r"\Q",
        ] {
            has(&t, e, esc);
        }
        has(
            &t,
            r"\x41",
            "constant.other.character-class.set.regexp constant.other.character-class.range.regexp constant.character.escape.backslash.regexp",
        );
        has(
            &t,
            r"\u005A",
            "constant.other.character-class.set.regexp constant.other.character-class.range.regexp constant.character.escape.backslash.regexp",
        );
        has(
            &t,
            r"\w",
            "constant.other.character-class.set.regexp constant.other.character-class.escape.backslash.regexp",
        );
        has(&t, ".", "constant.other.character-class.set.regexp");
        // `\pL` is Java's; a property's name has no digit; a lone `\` at the end is a character
        let t = lex_in(r"\pL\p{L1}\p{Script=}x\", false, Dialect::Ecma);
        has(&t, r"\p", esc);
        assert!(!t.iter().any(|(x, _)| *x == "L" || *x == "\\"), "{t:?}");
        has(&t, "Script", "support.constant.unicode-property.regexp");
        // `(?<名>…)`: a name past ASCII
        let t = lex_in("(?<名>a)", false, Dialect::Ecma);
        has(&t, "名", "entity.name.other.group.regexp");
    }
}
