//! Which of C++'s and C's strings are regexes (X-115). C++'s regex is a library's, so the
//! dialect follows the call, not the language: a string is one where the call it is an argument
//! of names an engine, looked back on its line from the string ([`regex_at`]), and the engine's
//! dialect is the one its flags after the pattern pick, read ahead on the call's line.
//!
//! | engine | where | dialect |
//! |---|---|---|
//! | `<regex>` | `std::regex(`, `std::wregex(`, `regex(` (after `using namespace std`), a declaration `std::regex re(` or `{` | ECMAScript; ERE after `::extended`, `::egrep`, `::awk`; BRE after `::basic`, `::grep` |
//! | Boost.Regex | `boost::regex(`, `boost::wregex(`, a declaration; xpressive's `sregex::compile(` | Perl's (Java's); ERE and BRE as `<regex>`'s, `::sed` BRE; `::literal` none |
//! | Qt | `QRegularExpression(`, `QRegExp(`, a declaration, through `QStringLiteral(`, `QLatin1String(`, `QString(`…; `setFilterRegularExpression(` | PCRE2's (Java's); `QRegExp::Wildcard`, `WildcardUnix`, `FixedString` none |
//! | RE2 | `RE2(`, `re2::RE2(`, a declaration; the second argument of `RE2::FullMatch(`, `PartialMatch(`, `Consume(`, `FindAndConsume(`, `Replace(`, `GlobalReplace(`, `Extract(` | RE2's |
//! | PCRE | `pcre2_compile(`, `pcre_compile(`, a cast before the string passed over (`(PCRE2_SPTR)"…"`) | Java's; `PCRE2_LITERAL` none |
//! | POSIX (C too) | the second argument of `regcomp(` | BRE; ERE with `REG_EXTENDED`; `REG_NOSPEC` none |
//! | CTRE | `ctre::match<"…">`, any `ctre::` name's template argument; `ctll::fixed_string{"…"}` | Java's |
//!
//! Not followed: a pattern on a later line than its call, or in a variable passed later; a
//! member initializer (`: re_("…")`); `basic_regex<char>(`; CTRE's `"…"_ctre` (known only past
//! the string); a pattern built by `+`; a flag on a later line than a pattern (the engine's
//! default kept); a `"…"` regex continued by a `\` at its line's end (a string on the next).

use crate::regex::Dialect;

/// The dialects a raw string's state keeps ([`dialect_code`]).
const DIALECTS: [Dialect; 5] = [
    Dialect::Ecma,
    Dialect::Java,
    Dialect::Re2,
    Dialect::Ere,
    Dialect::Bre,
];

/// `d`'s code in a state's byte.
pub(super) fn dialect_code(d: Dialect) -> u8 {
    DIALECTS.iter().position(|&x| x == d).unwrap_or(0) as u8
}

/// The dialect of code `c` ([`dialect_code`]).
pub(super) fn dialect_of(c: u8) -> Dialect {
    DIALECTS
        .get(usize::from(c))
        .copied()
        .unwrap_or(Dialect::Ecma)
}

/// An engine, by the call a string is an argument of.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Engine {
    /// `<regex>`'s.
    Std,
    /// Boost.Regex's, xpressive's.
    Boost,
    /// `QRegularExpression`.
    Qt,
    /// `QRegExp`, whose syntax can be a wildcard's or a fixed string.
    QtOld,
    Re2,
    Pcre,
    /// POSIX's `regcomp(`.
    Posix,
    Ctre,
}

/// Whether `c` can be in a name.
fn word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

/// Where the blanks before `p` start.
fn blank_back(b: &[u8], p: usize) -> usize {
    let mut q = p;
    while q > 0 && matches!(b[q - 1], b' ' | b'\t') {
        q -= 1;
    }
    q
}

/// Where the name that ends at `w` starts (`w` if none does).
fn word_back(b: &[u8], w: usize) -> usize {
    let mut s = w;
    while s > 0 && word_byte(b[s - 1]) {
        s -= 1;
    }
    s
}

/// The name before `::` at `s` (`std` of `std::regex`), `None` without a `::`.
fn qualifier(b: &[u8], s: usize) -> Option<&[u8]> {
    (s >= 2 && &b[s - 2..s] == b"::").then(|| &b[word_back(b, s - 2)..s - 2])
}

// What a name is to the calls ([`NAMES`]): the reader is cold, so a table searched in turn,
// small, in place of `match`es on slices compiled to chains of compares.
/// `<regex>`'s or Boost's type, by the qualifier (`regex`, `wregex`).
const N_REGEX: u8 = 1;
const N_QT: u8 = 2;
const N_QT_OLD: u8 = 3;
const N_RE2: u8 = 4;
/// PCRE's compile, its pattern first (C too).
const N_PCRE: u8 = 5;
/// xpressive's `sregex::compile`.
const N_COMPILE: u8 = 6;
/// Qt's setters that take a pattern.
const N_QT_SETTER: u8 = 7;
/// `ctll::fixed_string`.
const N_FIXED: u8 = 8;
/// A Qt string's wrapper, the call around it the engine's.
const N_WRAPPER: u8 = 9;
/// POSIX's `regcomp(`, its pattern second (C too).
const N_REGCOMP: u8 = 10;
/// RE2's matchers, their pattern second.
const N_RE2_MATCHER: u8 = 11;

static NAMES: [(&[u8], u8); 35] = [
    (b"regex", N_REGEX),
    (b"wregex", N_REGEX),
    (b"QRegularExpression", N_QT),
    (b"QRegExp", N_QT_OLD),
    (b"RE2", N_RE2),
    (b"pcre2_compile", N_PCRE),
    (b"pcre2_compile_8", N_PCRE),
    (b"pcre2_compile_16", N_PCRE),
    (b"pcre2_compile_32", N_PCRE),
    (b"pcre_compile", N_PCRE),
    (b"pcre_compile2", N_PCRE),
    (b"compile", N_COMPILE),
    (b"setFilterRegularExpression", N_QT_SETTER),
    (b"setFilterRegExp", N_QT_SETTER),
    (b"fixed_string", N_FIXED),
    (b"QStringLiteral", N_WRAPPER),
    (b"QLatin1String", N_WRAPPER),
    (b"QLatin1StringView", N_WRAPPER),
    (b"QString", N_WRAPPER),
    (b"QStringView", N_WRAPPER),
    (b"QLatin1Literal", N_WRAPPER),
    (b"fromLatin1", N_WRAPPER),
    (b"fromUtf8", N_WRAPPER),
    (b"regcomp", N_REGCOMP),
    (b"FullMatch", N_RE2_MATCHER),
    (b"PartialMatch", N_RE2_MATCHER),
    (b"Consume", N_RE2_MATCHER),
    (b"FindAndConsume", N_RE2_MATCHER),
    (b"Replace", N_RE2_MATCHER),
    (b"GlobalReplace", N_RE2_MATCHER),
    (b"Extract", N_RE2_MATCHER),
    (b"FullMatchN", N_RE2_MATCHER),
    (b"PartialMatchN", N_RE2_MATCHER),
    (b"ConsumeN", N_RE2_MATCHER),
    (b"FindAndConsumeN", N_RE2_MATCHER),
];

/// What name `w` is to the calls ([`N_REGEX`]…), 0 for none.
fn name_of(w: &[u8]) -> u8 {
    NAMES.iter().find(|(n, _)| *n == w).map_or(0, |&(_, r)| r)
}

/// Whether the qualifier `q` of a name is one of `of`.
fn qualified(q: Option<&[u8]>, of: &[&[u8]]) -> bool {
    q.is_some_and(|q| of.contains(&q))
}

/// The engine a type's name `b[s..w]` is: `std::regex`, `boost::regex`, `QRegularExpression`…
fn engine_type(b: &[u8], s: usize, w: usize) -> Option<Engine> {
    let q = qualifier(b, s);
    match name_of(&b[s..w]) {
        N_REGEX if q.is_none() || qualified(q, &[b"std", b"tr1", b""]) => Some(Engine::Std),
        N_REGEX if qualified(q, &[b"boost"]) => Some(Engine::Boost),
        N_QT => Some(Engine::Qt),
        N_QT_OLD => Some(Engine::QtOld),
        N_RE2 if q.is_none() || qualified(q, &[b"re2", b""]) => Some(Engine::Re2),
        _ => None,
    }
}

/// Whether the call whose `(` or `{` is at `o` takes a regex first: the engine and where the
/// call's name starts.
fn first_argument(b: &[u8], o: usize, c: bool) -> Option<(Engine, usize)> {
    let w = blank_back(b, o);
    let s = word_back(b, w);
    if s == w {
        return None;
    }
    let q = qualifier(b, s);
    match name_of(&b[s..w]) {
        N_PCRE => return Some((Engine::Pcre, s)),
        _ if c => return None,
        N_REGEX | N_QT | N_QT_OLD | N_RE2 => {
            if let Some(e) = engine_type(b, s, w) {
                return Some((e, s));
            }
        }
        N_COMPILE if qualified(q, &[b"sregex", b"cregex", b"wsregex", b"wcregex"]) => {
            return Some((Engine::Boost, s));
        }
        N_QT_SETTER => return Some((Engine::Qt, s)),
        N_FIXED if qualified(q, &[b"ctll"]) => return Some((Engine::Ctre, s)),
        // a string's wrapper in Qt: the call around it
        N_WRAPPER => {
            let s = match q {
                Some(q) if q == b"QString" => s - 2 - q.len(),
                _ => s,
            };
            let t = blank_back(b, s);
            if t > 0
                && matches!(b[t - 1], b'(' | b'{')
                && let Some((e @ (Engine::Qt | Engine::QtOld), at)) = first_argument(b, t - 1, c)
            {
                return Some((e, at));
            }
            return None;
        }
        _ => {}
    }
    // a declaration: `std::regex re(`, `static const QRegularExpression re{`
    let t = blank_back(b, s);
    if t == s {
        return None;
    }
    let ts = word_back(b, t);
    engine_type(b, ts, t).map(|e| (e, s))
}

/// Whether the call whose argument starts after the `,` at `comma` takes a regex there, its
/// second: `regcomp(`, `RE2::FullMatch(`…
fn second_argument(b: &[u8], comma: usize) -> Option<Engine> {
    // back to the call's `(`, over the first argument's brackets and strings
    let mut depth = 0usize;
    let mut i = comma;
    let open = loop {
        if i == 0 {
            return None;
        }
        i -= 1;
        match b[i] {
            b')' | b']' | b'}' => depth += 1,
            b'(' | b'[' | b'{' if depth == 0 => break i,
            b'(' | b'[' | b'{' => depth -= 1,
            b',' | b';' if depth == 0 => return None,
            q @ (b'"' | b'\'') => {
                // to the quote that opens it: one not escaped
                loop {
                    let j = b[..i].iter().rposition(|&x| x == q)?;
                    let escapes = b[..j].iter().rev().take_while(|&&x| x == b'\\').count();
                    i = j;
                    if escapes % 2 == 0 {
                        break;
                    }
                }
            }
            _ => {}
        }
    };
    if b[open] != b'(' {
        return None;
    }
    let w = blank_back(b, open);
    let s = word_back(b, w);
    match name_of(&b[s..w]) {
        N_REGCOMP => Some(Engine::Posix),
        N_RE2_MATCHER if qualified(qualifier(b, s), &[b"RE2"]) => Some(Engine::Re2),
        _ => None,
    }
}

/// Where the string at `s` (its prefix, or its quote) ends on its line: past its closing quote,
/// `None` where it goes on past the line.
fn string_end(b: &[u8], s: usize) -> Option<usize> {
    let q = s + b[s..].iter().position(|&c| c == b'"')?;
    if b[s..q].ends_with(b"R") {
        // `)delim"`
        let d = q + 1 + b[q + 1..].iter().position(|&c| c == b'(')?;
        let delim = &b[q + 1..d];
        let mut i = d + 1;
        loop {
            let c = i + b[i..].iter().position(|&c| c == b')')?;
            let e = c + 1 + delim.len();
            if e < b.len() && &b[c + 1..e] == delim && b[e] == b'"' {
                return Some(e + 1);
            }
            i = c + 1;
        }
    }
    let mut i = q + 1;
    while i < b.len() {
        match b[i] {
            b'"' => return Some(i + 1),
            b'\\' => i += 2,
            _ => i += 1,
        }
    }
    None
}

// The flags read ahead in a call ([`flags_after`]).
const F_ERE: u8 = 1;
const F_BRE: u8 = 2;
/// A pattern taken as it reads, not a regex: Boost's `::literal`, `QRegExp::FixedString` and
/// its wildcards, `PCRE2_LITERAL`, BSD's `REG_NOSPEC`.
const F_LITERAL: u8 = 4;
/// POSIX's `REG_EXTENDED`.
const F_REG_EXTENDED: u8 = 8;

/// The flags' names: the flag, whether it counts only after `::` (a library's option, not C's
/// macro).
static FLAGS: [(&[u8], u8, bool); 13] = [
    (b"extended", F_ERE, true),
    (b"egrep", F_ERE, true),
    (b"awk", F_ERE, true),
    (b"basic", F_BRE, true),
    (b"grep", F_BRE, true),
    (b"sed", F_BRE, true),
    (b"literal", F_LITERAL, true),
    (b"FixedString", F_LITERAL, true),
    (b"Wildcard", F_LITERAL, true),
    (b"WildcardUnix", F_LITERAL, true),
    (b"PCRE2_LITERAL", F_LITERAL, false),
    (b"REG_NOSPEC", F_LITERAL, false),
    (b"REG_EXTENDED", F_REG_EXTENDED, false),
];

/// The flags in the call's arguments after the string at `s`, to the call's end on the line:
/// a library's syntax option after `::` (`std::regex::extended`, `QRegExp::Wildcard`), C's as
/// bare names (`REG_EXTENDED`).
fn flags_after(b: &[u8], s: usize) -> u8 {
    let Some(mut i) = string_end(b, s) else {
        return 0;
    };
    let mut flags = 0;
    let mut depth = 0usize;
    while i < b.len() {
        let c = b[i];
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' if depth == 0 => break,
            b')' | b']' | b'}' => depth -= 1,
            b';' => break,
            b'"' | b'\'' => {
                // a string or a character passed over
                i += 1;
                while i < b.len() && b[i] != c {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
            }
            _ if word_byte(c) => {
                let mut e = i + 1;
                while e < b.len() && word_byte(b[e]) {
                    e += 1;
                }
                let scoped = i >= 2 && &b[i - 2..i] == b"::";
                flags |= FLAGS
                    .iter()
                    .find(|(w, _, needs)| *w == &b[i..e] && (!needs || scoped))
                    .map_or(0, |&(_, f, _)| f);
                i = e;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    flags
}

/// Whether the string at `s` (its prefix, or its quote) is a regex, the byte before it (past
/// blanks) at `o`: its dialect.
#[cold]
#[inline(never)]
pub(super) fn regex_at(b: &[u8], o: usize, s: usize, c: bool) -> Option<Dialect> {
    let engine = match b[o] {
        b'(' | b'{' => first_argument(b, o, c)?.0,
        b',' => second_argument(b, o)?,
        // CTRE's `ctre::match<"…">`
        b'<' if !c => {
            let w = blank_back(b, o);
            let ws = word_back(b, w);
            (ws < w && qualifier(b, ws) == Some(b"ctre")).then_some(Engine::Ctre)?
        }
        // a cast before the string: `pcre2_compile((PCRE2_SPTR)"…"`
        b')' => {
            let open = b[..o]
                .iter()
                .rposition(|&x| matches!(x, b'(' | b')' | b'"' | b';'))?;
            if b[open] != b'(' {
                return None;
            }
            let q = blank_back(b, open);
            if q == 0 || b[q - 1] != b'(' {
                return None;
            }
            match first_argument(b, q - 1, c)?.0 {
                Engine::Pcre => Engine::Pcre,
                _ => return None,
            }
        }
        _ => return None,
    };
    let flags = flags_after(b, s);
    let posix = |default: Dialect| {
        if flags & F_ERE != 0 {
            Dialect::Ere
        } else if flags & F_BRE != 0 {
            Dialect::Bre
        } else {
            default
        }
    };
    Some(match engine {
        Engine::Std => posix(Dialect::Ecma),
        Engine::Boost if flags & F_LITERAL != 0 => return None,
        Engine::Boost => posix(Dialect::Java),
        Engine::QtOld if flags & F_LITERAL != 0 => return None,
        Engine::Qt | Engine::QtOld | Engine::Ctre => Dialect::Java,
        Engine::Re2 => Dialect::Re2,
        Engine::Pcre if flags & F_LITERAL != 0 => return None,
        Engine::Pcre => Dialect::Java,
        Engine::Posix if flags & F_LITERAL != 0 => return None,
        Engine::Posix if flags & F_REG_EXTENDED != 0 => Dialect::Ere,
        Engine::Posix => Dialect::Bre,
    })
}
