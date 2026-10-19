//! Log lines (O-43): a one-pass anpa parser in place of bat's `Log` grammar, whose regexes took
//! ~0.5 ms a line on Oniguruma. A line is a lead (a timestamp; a level, maybe with a tracing
//! target after it; or syslog's and journalctl's `host unit[pid]:`), then a message scanned for
//! strings, numbers, hex, UUIDs, IP addresses, URLs, `key=value` pairs and level words.
//!
//! A line depends on no other: a string left open isn't a string. Each [`Kind`] carries the
//! scopes bat's grammar gave the same text where it had one, so `[syntax]` rules written for it
//! still apply.

use anpa::combinators::{
    bind, many, no_separator, not_empty, or, or_diff, peek, right, succeed, times,
};
use anpa::core::{AnpaState, ParserExt, StrParser};
use anpa::item_matches;
use anpa::parsers::{empty, item, item_if, item_while, skip};

/// What a stretch of a log line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Date,
    /// The `T` between a date and a time.
    DateSep,
    Time,
    Host,
    /// A syslog unit, or a tracing target.
    Unit,
    Pid,
    Punct,
    Error,
    Warning,
    Info,
    Debug,
    Number,
    /// `0x40`.
    Hex,
    /// A hash, a digest, an id: `4b7f62b`.
    Hash,
    Uuid,
    Ip,
    Str,
    Quote,
    Escape,
    Key,
    Assign,
    Url,
}

impl Kind {
    pub const ALL: [Kind; 22] = [
        Kind::Date,
        Kind::DateSep,
        Kind::Time,
        Kind::Host,
        Kind::Unit,
        Kind::Pid,
        Kind::Punct,
        Kind::Error,
        Kind::Warning,
        Kind::Info,
        Kind::Debug,
        Kind::Number,
        Kind::Hex,
        Kind::Hash,
        Kind::Uuid,
        Kind::Ip,
        Kind::Str,
        Kind::Quote,
        Kind::Escape,
        Kind::Key,
        Kind::Assign,
        Kind::Url,
    ];

    /// The scopes under `text.log`, innermost last.
    pub fn scopes(self) -> &'static str {
        match self {
            Kind::Date => {
                "meta.date.log meta.number.integer.decimal.log constant.numeric.value.log"
            }
            Kind::DateSep => "meta.date.log meta.time.log keyword.other.log",
            Kind::Time => {
                "meta.time.log meta.number.integer.decimal.log constant.numeric.value.log"
            }
            Kind::Host => "meta.host.log",
            Kind::Unit => "meta.unit.log entity.name.function.log",
            Kind::Pid => "meta.pid.log constant.numeric.value.log",
            Kind::Punct => "punctuation.separator.log",
            Kind::Error => "markup.error.log",
            Kind::Warning => "markup.warning.log",
            Kind::Info => "markup.info.log",
            Kind::Debug => "markup.debug.log",
            Kind::Number => "meta.number.log constant.numeric.value.log",
            Kind::Hex => "meta.number.hexadecimal.log constant.numeric.value.log",
            Kind::Hash => "constant.other.hash.log",
            Kind::Uuid => "constant.other.uuid.log",
            Kind::Ip => "meta.ipaddress.log constant.numeric.value.log",
            Kind::Str => "string.quoted.log",
            Kind::Quote => "string.quoted.log punctuation.definition.string.log",
            Kind::Escape => "string.quoted.log constant.character.escape.log",
            Kind::Key => "variable.parameter.log",
            Kind::Assign => "keyword.operator.assignment.log",
            Kind::Url => "markup.underline.link.log",
        }
    }
}

/// Bytes `start..end` of a line are a `kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub kind: Kind,
}

/// The parsers' state: the line, the spans found so far, the quotes known not to close, and
/// where the runs that scanners failed on end.
struct Out<'a> {
    line: &'a str,
    spans: Vec<Span>,
    /// One bit per [`QUOTES`] entry.
    open: u8,
    /// By [`Dead`]: the offset the last run that scanner failed on ends at.
    dead: [usize; 3],
}

/// Scanners that read a run of characters, then fail or not by what comes after it. A start
/// later in the same run reads it to the same end and fails the same way, so it fails at once
/// instead of reading it again: without this, a line of `a.a.a…` read the rest of the line
/// from each word (O-50).
#[derive(Clone, Copy)]
enum Dead {
    /// [`url`]'s scheme.
    Url,
    /// [`pair`]'s key.
    Pair,
    /// [`number`]'s decimal: its digits and dotted parts.
    Decimal,
}

const QUOTES: [char; 3] = ['"', '\'', '`'];

type State<'s, 'a> = AnpaState<'s, &'a str, Out<'a>>;

/// The spans of one line, in order, into `spans` (cleared first).
pub fn tokens(line: &str, spans: &mut Vec<Span>) {
    spans.clear();
    let mut out = Out {
        line,
        spans: std::mem::take(spans),
        open: 0,
        dead: [0; 3],
    };
    right(lead(), message()).parse_state(line, &mut out);
    *spans = out.spans;
}

fn offset(s: &State) -> usize {
    s.user_state.line.len() - s.input.len()
}

fn push(s: &mut State, start: usize, kind: Kind) {
    let end = offset(s);
    if end > start {
        s.user_state.spans.push(Span {
            start: start as u32,
            end: end as u32,
            kind,
        });
    }
}

/// `p`, its input marked as `kind`.
fn mark<'a, O>(kind: Kind, p: impl StrParser<'a, O, Out<'a>>) -> impl StrParser<'a, (), Out<'a>> {
    move |s: &mut State<'_, 'a>| {
        let start = offset(s);
        p(s)?;
        push(s, start, kind);
        Some(())
    }
}

/// `p`, its input marked as the kind it returns.
fn mark_as<'a>(p: impl StrParser<'a, Kind, Out<'a>>) -> impl StrParser<'a, (), Out<'a>> {
    move |s: &mut State<'_, 'a>| {
        let start = offset(s);
        let kind = p(s)?;
        push(s, start, kind);
        Some(())
    }
}

/// `p`, all or nothing: on failure, neither input nor spans move. Every alternative that marks
/// spans goes through this, since `or` only puts the input back.
fn atomic<'a, O>(p: impl StrParser<'a, O, Out<'a>>) -> impl StrParser<'a, O, Out<'a>> {
    move |s: &mut State<'_, 'a>| {
        let (input, n) = (s.input, s.user_state.spans.len());
        let o = p(s);
        if o.is_none() {
            s.input = input;
            s.user_state.spans.truncate(n);
        }
        o
    }
}

fn opt<'a, O>(p: impl StrParser<'a, O, Out<'a>>) -> impl StrParser<'a, Option<O>, Out<'a>> {
    succeed(atomic(p))
}

/// `p`, failing at once inside a run it failed on before: `run` is the length of the run at
/// the input, which decides how `p` ends.
fn dead_end<'a, O>(
    which: Dead,
    run: fn(&str) -> usize,
    p: impl StrParser<'a, O, Out<'a>>,
) -> impl StrParser<'a, O, Out<'a>> {
    move |s: &mut State<'_, 'a>| {
        let at = offset(s);
        if at < s.user_state.dead[which as usize] {
            return None;
        }
        let input = s.input;
        let o = p(s);
        if o.is_none() {
            s.user_state.dead[which as usize] = at + run(input);
        }
        o
    }
}

/// Succeeds, consuming nothing, when `ok`.
fn guard<'a>(ok: bool) -> impl StrParser<'a, (), Out<'a>> {
    move |_: &mut State<'_, 'a>| ok.then_some(())
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The end of a word: no word character comes next.
fn word_end<'a>() -> impl StrParser<'a, (), Out<'a>> {
    |s: &mut State<'_, 'a>| (!s.input.starts_with(is_word)).then_some(())
}

fn spaces<'a>() -> impl StrParser<'a, &'a str, Out<'a>> {
    item_while(|c: char| c == ' ')
}

fn digits<'a>(n: u32) -> impl StrParser<'a, &'a str, Out<'a>> {
    times(n, item_if(|c: char| c.is_ascii_digit()))
}

fn some_digits<'a>() -> impl StrParser<'a, &'a str, Out<'a>> {
    not_empty(item_while(|c: char| c.is_ascii_digit()))
}

fn hex_digits<'a>(n: u32) -> impl StrParser<'a, &'a str, Out<'a>> {
    times(n, item_if(|c: char| c.is_ascii_hexdigit()))
}

// --- timestamps

/// `2026-10-18` or `2026/10/18`.
fn ymd<'a>() -> impl StrParser<'a, (), Out<'a>> {
    right(
        digits(4),
        bind(item_matches!('-' | '/'), |sep: char| {
            right(digits(2), right(skip(sep), digits(2))).map(|_| ())
        }),
    )
}

/// `09:12:01`, with a fraction (`.123`, `,123`) and a zone (`Z`, `+02:00`, `-0700`); the seconds
/// may be left out only when `secs` is false.
fn clock<'a>(secs: bool) -> impl StrParser<'a, (), Out<'a>> {
    let seconds = opt(right(skip(':'), digits(2))).filter(move |s| s.is_some() || !secs);
    let fraction = opt(right(item_matches!('.' | ','), some_digits()));
    let offset = right(
        item_matches!('+' | '-'),
        right(digits(2), opt(right(succeed(skip(':')), digits(2)))),
    );
    let zone = opt(or_diff(skip('Z'), offset));
    right(
        right(digits(2), right(skip(':'), digits(2))),
        right(seconds, right(fraction, zone)),
    )
    .map(|_| ())
}

/// An ISO date, and the time after it.
fn datetime<'a>() -> impl StrParser<'a, (), Out<'a>> {
    let sep = or_diff(mark(Kind::DateSep, skip('T')), skip(' '));
    right(
        mark(Kind::Date, ymd()),
        right(opt(right(sep, mark(Kind::Time, clock(false)))), word_end()),
    )
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// syslog's `Oct 18 09:12:01`, `Oct  8 09:12:01`.
fn syslog<'a>() -> impl StrParser<'a, (), Out<'a>> {
    let month = item_while(|c: char| c.is_ascii_alphabetic()).filter(|m| MONTHS.contains(m));
    let day = right(item_matches!(' ' | '0'..='9'), digits(1));
    right(
        mark(Kind::Date, right(month, right(skip(' '), day))),
        right(skip(' '), right(mark(Kind::Time, clock(true)), word_end())),
    )
}

/// A time with no date.
fn time<'a>() -> impl StrParser<'a, (), Out<'a>> {
    right(mark(Kind::Time, clock(true)), word_end())
}

/// glog's `I1018 09:12:01.123456`: the level's letter, then month and day.
fn glog<'a>() -> impl StrParser<'a, (), Out<'a>> {
    let level = item_matches!('I' | 'W' | 'E' | 'F').map(|c| match c {
        'I' => Kind::Info,
        'W' => Kind::Warning,
        _ => Kind::Error,
    });
    right(
        right(mark_as(level), mark(Kind::Date, digits(4))),
        right(skip(' '), time()),
    )
}

/// dmesg's `[   12.345678]`.
fn uptime<'a>() -> impl StrParser<'a, (), Out<'a>> {
    let seconds = right(some_digits(), right(skip('.'), some_digits()));
    right(
        mark(Kind::Punct, skip('[')),
        right(
            spaces(),
            right(mark(Kind::Time, seconds), mark(Kind::Punct, skip(']'))),
        ),
    )
}

/// A timestamp, maybe in brackets; true for one syslog or journalctl writes, which a
/// `host unit[pid]:` may follow.
fn stamp<'a>() -> impl StrParser<'a, bool, Out<'a>> {
    let journal = or(atomic(datetime()), atomic(syslog()));
    let bare = or(
        journal.map(|_| true),
        or(atomic(glog()), atomic(time())).map(|_| false),
    );
    let bracketed = right(
        mark(Kind::Punct, skip('[')),
        right(
            or(atomic(datetime()), atomic(time())),
            mark(Kind::Punct, skip(']')),
        ),
    );
    or(bare, or(atomic(bracketed), atomic(uptime())).map(|_| false))
}

// --- levels

/// A level word where a level is expected, in any case.
fn level(word: &str) -> Option<Kind> {
    const LEVELS: [(&str, Kind); 22] = [
        ("error", Kind::Error),
        ("err", Kind::Error),
        ("eror", Kind::Error),
        ("fatal", Kind::Error),
        ("crit", Kind::Error),
        ("critical", Kind::Error),
        ("emerg", Kind::Error),
        ("alert", Kind::Error),
        ("panic", Kind::Error),
        ("severe", Kind::Error),
        ("warn", Kind::Warning),
        ("warning", Kind::Warning),
        ("wrn", Kind::Warning),
        ("info", Kind::Info),
        ("inf", Kind::Info),
        ("notice", Kind::Info),
        ("debug", Kind::Debug),
        ("dbg", Kind::Debug),
        ("trace", Kind::Debug),
        ("trc", Kind::Debug),
        ("verbose", Kind::Debug),
        ("fine", Kind::Debug),
    ];
    if word.len() > 8 {
        return None;
    }
    LEVELS
        .iter()
        .find(|(l, _)| word.eq_ignore_ascii_case(l))
        .map(|&(_, k)| k)
}

/// A word in a message that flags a level: error and warning words in any case (`failed`,
/// `Exception`, `IOException`), the other levels in capitals (`INFO`, `DEBUG`).
fn message_level(word: &str) -> Option<Kind> {
    const ERRORS: [&str; 9] = [
        "error",
        "fail",
        "failed",
        "failure",
        "fatal",
        "critical",
        "exception",
        "panic",
        "panicked",
    ];
    if word.len() > 9 {
        // `IOException`, `ValueError`
        let camel = |suffix: &str| {
            word.strip_suffix(suffix)
                .is_some_and(|w| w.ends_with(|c: char| c.is_alphanumeric()))
        };
        return (camel("Exception") || camel("Error")).then_some(Kind::Error);
    }
    if word
        .starts_with(|c: char| c.is_ascii_lowercase() && !matches!(c, 'e' | 'f' | 'c' | 'p' | 'w'))
    {
        // most words, at once
        return None;
    }
    if ERRORS.iter().any(|e| word.eq_ignore_ascii_case(e)) || camel_error(word) {
        return Some(Kind::Error);
    }
    if word.eq_ignore_ascii_case("warn") || word.eq_ignore_ascii_case("warning") {
        return Some(Kind::Warning);
    }
    if word.len() > 2 && word.bytes().all(|b| b.is_ascii_uppercase()) && word != "FINE" {
        // maven's `BUILD SUCCESS`, a test runner's `PASSED`
        if matches!(word, "SUCCESS" | "PASSED") {
            return Some(Kind::Info);
        }
        return level(word);
    }
    None
}

/// A short CamelCase error type: `IOError`, `KeyError`.
fn camel_error(word: &str) -> bool {
    word.strip_suffix("Error")
        .is_some_and(|w| w.starts_with(|c: char| c.is_ascii_uppercase()))
}

/// A level word, alone or in `[]`, `<>` or `()`.
fn level_tag<'a>() -> impl StrParser<'a, (), Out<'a>> {
    let word = mark_as(left_end(
        item_while(|c: char| c.is_ascii_alphabetic()).map_if(level),
    ));
    let enclosed = |open: char, close: char| {
        right(
            mark(Kind::Punct, skip(open)),
            right(
                right(spaces(), word),
                right(spaces(), mark(Kind::Punct, skip(close))),
            ),
        )
    };
    or(
        atomic(word),
        or(
            atomic(enclosed('[', ']')),
            or(atomic(enclosed('<', '>')), atomic(enclosed('(', ')'))),
        ),
    )
}

/// `p`, then the end of a word.
fn left_end<'a, O>(p: impl StrParser<'a, O, Out<'a>>) -> impl StrParser<'a, O, Out<'a>> {
    move |s: &mut State<'_, 'a>| {
        let o = p(s)?;
        word_end()(s)?;
        Some(o)
    }
}

/// tracing's target, `ada::app:`, then a space or the end: a name of word characters, `:`,
/// `.`, `-` and `/`, not a level or error word.
fn target<'a>() -> impl StrParser<'a, (), Out<'a>> {
    |s: &mut State<'_, 'a>| {
        let run = s.input.split(' ').next()?;
        let name = run.strip_suffix(':')?;
        let ok = name.starts_with(is_word)
            && name
                .chars()
                .all(|c| is_word(c) || matches!(c, ':' | '.' | '-' | '/'))
            && message_level(name).is_none();
        if !ok {
            return None;
        }
        let start = offset(s);
        s.input = &s.input[name.len()..];
        push(s, start, Kind::Unit);
        s.input = &s.input[1..];
        push(s, start + name.len(), Kind::Punct);
        Some(())
    }
}

/// syslog's and journalctl's `host unit[pid]: `.
fn source<'a>() -> impl StrParser<'a, (), Out<'a>> {
    let host = mark(
        Kind::Host,
        not_empty(item_while(|c: char| is_word(c) || matches!(c, '.' | '-'))),
    );
    let unit = mark(
        Kind::Unit,
        not_empty(item_while(|c: char| {
            !c.is_whitespace() && !matches!(c, '[' | ':')
        })),
    );
    let pid = right(
        mark(Kind::Punct, skip('[')),
        right(mark(Kind::Pid, some_digits()), mark(Kind::Punct, skip(']'))),
    );
    let colon = right(
        mark(Kind::Punct, skip(':')),
        peek(or_diff(skip(' '), empty())),
    );
    right(right(host, skip(' ')), right(unit, right(opt(pid), colon)))
}

/// What a line starts with: a timestamp, then a level and a target, or a journal's source.
fn lead<'a>() -> impl StrParser<'a, (), Out<'a>> {
    let stamped = opt(right(spaces(), stamp())).map(|j| j.unwrap_or(false));
    let after = |journal: bool| {
        let leveled = right(level_tag(), opt(right(not_empty(spaces()), target()))).map(|_| ());
        opt(right(
            spaces(),
            or(atomic(leveled), atomic(right(guard(journal), source()))),
        ))
        .map(|_| ())
    };
    bind(stamped, after)
}

// --- the message

/// A run of characters that start nothing: spaces, punctuation.
fn gap<'a>() -> impl StrParser<'a, (), Out<'a>> {
    not_empty(item_while(|c: char| !is_word(c) && !QUOTES.contains(&c))).map(|_| ())
}

/// The length of the word characters `s` starts with.
fn word_len(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b.is_ascii() {
            if !(b.is_ascii_alphanumeric() || b == b'_') {
                break;
            }
            i += 1;
        } else {
            match s[i..].chars().next() {
                Some(c) if is_word(c) => i += c.len_utf8(),
                _ => break,
            }
        }
    }
    i
}

/// The word at the input, `len` bytes of word characters and then any apostrophes between word
/// characters (`don't`), which start no string: marked when it's a level or error word.
fn word(s: &mut State, mut len: usize) {
    while s.input[len..].starts_with('\'') {
        match word_len(&s.input[len + 1..]) {
            0 => break,
            n => len += 1 + n,
        }
    }
    let start = offset(s);
    let kind = message_level(&s.input[..len]);
    s.input = &s.input[len..];
    if let Some(kind) = kind {
        push(s, start, kind);
    }
}

/// A quoted string, closed on its line: `"` with `\` escapes, `'` and `` ` `` only after a
/// non-word character and before one. A quote that doesn't close makes every later one of its
/// kind fail at once.
fn quoted<'a>(n: usize) -> impl StrParser<'a, (), Out<'a>> {
    let q = QUOTES[n];
    let body = mark(
        Kind::Str,
        not_empty(item_while(move |c: char| c != q && c != '\\')),
    );
    let escape = mark(Kind::Escape, right(skip('\\'), item()));
    // `'don't'`
    let apostrophe = mark(Kind::Str, right(skip('\''), peek(item_if(is_word))));
    let inside = many(
        or_diff(body, or_diff(atomic(escape), atomic(apostrophe))),
        true,
        no_separator(),
    );
    let string = right(
        mark(Kind::Quote, skip(q)),
        right(inside, mark(Kind::Quote, skip(q))),
    );
    move |s: &mut State<'_, 'a>| {
        let bit = 1 << n;
        if !s.input.starts_with(q) || s.user_state.open & bit != 0 {
            return None;
        }
        let start = offset(s);
        let after_word = s.user_state.line[..start]
            .chars()
            .next_back()
            .is_some_and(is_word);
        if q != '"' && after_word {
            return None;
        }
        let closed = atomic(right(string, guard_end(q)))(s);
        if closed.is_none() {
            s.user_state.open |= bit;
        }
        closed
    }
}

/// After a closing `'` or `` ` ``, the end of a word.
fn guard_end<'a>(q: char) -> impl StrParser<'a, (), Out<'a>> {
    move |s: &mut State<'_, 'a>| (q == '"' || !s.input.starts_with(is_word)).then_some(())
}

/// `42`, `3.14`, `1.2.3`, `12ms`, `6.2KiB`, `0x40`.
fn number<'a>() -> impl StrParser<'a, (), Out<'a>> {
    const UNITS: [&str; 22] = [
        "ns", "us", "µs", "ms", "s", "m", "h", "d", "b", "B", "k", "K", "M", "G", "KB", "kB",
        "KiB", "MB", "MiB", "GB", "GiB", "TB",
    ];
    let hex = mark(
        Kind::Hex,
        right(
            or_diff(skip("0x"), skip("0X")),
            not_empty(item_while(|c: char| c.is_ascii_hexdigit())),
        ),
    );
    let dotted = many(
        atomic(right(skip('.'), some_digits())),
        true,
        no_separator(),
    );
    let unit = item_while(|c: char| c.is_alphabetic()).filter(|u| UNITS.contains(u));
    let decimal = mark(Kind::Number, right(some_digits(), right(dotted, opt(unit))));
    // a decimal can't follow `0x`, so the hex's end of a word goes with it alone
    let decimal = dead_end(Dead::Decimal, decimal_len, left_end(decimal));
    or(atomic(left_end(hex)).map(|_| ()), decimal)
}

/// The length of the digits `s` starts with and the dotted parts after them, `1.2.3`.
fn decimal_len(s: &str) -> usize {
    let b = s.as_bytes();
    let digits = |i: usize| b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
    let mut i = digits(0);
    while b.get(i) == Some(&b'.') && b.get(i + 1).is_some_and(u8::is_ascii_digit) {
        i += 1 + digits(i + 1);
    }
    i
}

/// A hash, a digest or an id: 7 or more hex digits, with a digit among them and letters all in
/// one case (`4b7f62b`, `4B7F62B`, not `4b7F62b`).
fn hash<'a>() -> impl StrParser<'a, (), Out<'a>> {
    mark(
        Kind::Hash,
        left_end(
            not_empty(item_while(|c: char| c.is_ascii_hexdigit())).filter(|h: &&str| {
                h.len() >= 7
                    && h.bytes().any(|b| b.is_ascii_digit())
                    && h.bytes().any(|b| b.is_ascii_alphabetic())
                    && (!h.bytes().any(|b| b.is_ascii_uppercase())
                        || !h.bytes().any(|b| b.is_ascii_lowercase()))
            }),
        ),
    )
}

fn uuid<'a>() -> impl StrParser<'a, (), Out<'a>> {
    let part = |n| right(skip('-'), hex_digits(n));
    mark(
        Kind::Uuid,
        left_end(right(
            hex_digits(8),
            right(part(4), right(part(4), right(part(4), part(12)))),
        )),
    )
}

/// `192.168.1.1`, `10.0.0.1:8080`, `10.0.0.0/8`.
fn ipv4<'a>() -> impl StrParser<'a, (), Out<'a>> {
    let octet =
        || some_digits().filter(|d: &&str| d.len() <= 3 && d.parse::<u16>().is_ok_and(|n| n < 256));
    let dot_octet = || right(skip('.'), octet());
    let port = opt(right(item_matches!(':' | '/'), some_digits()));
    // not a dotted version with more parts
    let last = |s: &mut State<'_, 'a>| {
        let more = s
            .input
            .strip_prefix('.')
            .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()));
        (!more).then_some(())
    };
    mark(
        Kind::Ip,
        left_end(right(
            right(octet(), right(dot_octet(), right(dot_octet(), dot_octet()))),
            right(last, port),
        )),
    )
}

/// `fe80::1`, `2001:db8:0:0:0:0:0:1`: hex groups of up to four, with `::` or eight groups, and a
/// digit somewhere, so neither a time nor `abc::def` is one.
fn ipv6<'a>() -> impl StrParser<'a, (), Out<'a>> {
    let valid = |a: &&str| {
        let colons = a.bytes().filter(|&b| b == b':').count();
        (a.contains("::") || colons == 7)
            && (2..=7).contains(&colons)
            && a.split(':').all(|g| g.len() <= 4)
            && a.bytes().any(|b| b.is_ascii_digit())
            && !a.contains(":::")
    };
    // groups and colons, read no further than a valid address could reach (7 colons and 8
    // groups of 4: 39 bytes), so a long run of them isn't read again from each word (O-50)
    let groups = |s: &mut State<'_, 'a>| {
        let n = s
            .input
            .bytes()
            .take(40)
            .take_while(|&b| b.is_ascii_hexdigit() || b == b':')
            .count();
        let a = &s.input[..n];
        s.input = &s.input[n..];
        (1..40).contains(&n).then_some(a)
    };
    mark(Kind::Ip, left_end(groups.filter(valid)))
}

/// `https://example.com/x?y`, short of trailing punctuation and of a `)` it doesn't open.
fn url<'a>() -> impl StrParser<'a, (), Out<'a>> {
    let url = |s: &mut State<'_, 'a>| {
        let input = s.input;
        let scheme = scheme_len(input);
        if scheme == 0 || !input[scheme..].starts_with("://") {
            return None;
        }
        let rest = &input[scheme + 3..];
        let end = rest
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '<' | '>'))
            .unwrap_or(rest.len());
        let mut url = &rest[..end];
        loop {
            let trimmed = url.trim_end_matches(['.', ',', ';', ':', '!', '?']);
            let opens = trimmed.matches('(').count();
            let trimmed = match trimmed.strip_suffix(')') {
                Some(t) if t.matches(')').count() >= opens => t,
                _ => trimmed,
            };
            if trimmed.len() == url.len() {
                break;
            }
            url = trimmed;
        }
        if url.is_empty() {
            return None;
        }
        let start = offset(s);
        s.input = &input[scheme + 3 + url.len()..];
        push(s, start, Kind::Url);
        Some(())
    };
    dead_end(Dead::Url, scheme_len, url)
}

/// The length of the URL scheme's characters `s` starts with.
fn scheme_len(s: &str) -> usize {
    s.find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')))
        .unwrap_or(s.len())
}

fn is_key(c: char) -> bool {
    is_word(c) || matches!(c, '.' | '-')
}

/// The length of the key characters `s` starts with.
fn key_len(s: &str) -> usize {
    s.find(|c: char| !is_key(c)).unwrap_or(s.len())
}

/// `key=value`'s key and `=`; a level's value (`level=warn`) as its level.
fn pair<'a>() -> impl StrParser<'a, (), Out<'a>> {
    let name = not_empty(item_while(is_key));
    let key = move |s: &mut State<'_, 'a>| {
        let start = offset(s);
        let key = name(s)?;
        push(s, start, Kind::Key);
        Some(key)
    };
    let assign = mark(Kind::Assign, left_end_not_eq());
    let pair = bind(key, move |k: &str| {
        let leveled = ["level", "lvl", "severity", "loglevel"]
            .iter()
            .any(|l| k.eq_ignore_ascii_case(l));
        let value = mark_as(left_end(
            item_while(|c: char| c.is_ascii_alphabetic()).map_if(level),
        ));
        right(assign, opt(right(guard(leveled), value)).map(|_| ()))
    });
    dead_end(Dead::Pair, key_len, pair)
}

/// A `=` that isn't `==`.
fn left_end_not_eq<'a>() -> impl StrParser<'a, (), Out<'a>> {
    |s: &mut State<'_, 'a>| {
        let rest = s.input.strip_prefix('=')?;
        if rest.starts_with('=') {
            return None;
        }
        s.input = rest;
        Some(())
    }
}

/// The rest of the line. A token goes by its first character and the shape of the word it
/// starts, so a plain word is scanned for what it is once, not once for each thing it might
/// have been.
fn message<'a>() -> impl StrParser<'a, (), Out<'a>> {
    let (uuid, ipv4, datetime, time) = (
        atomic(uuid()),
        atomic(ipv4()),
        atomic(datetime()),
        atomic(time()),
    );
    let (hash, number, ipv6, url, pair) = (
        atomic(hash()),
        atomic(number()),
        atomic(ipv6()),
        atomic(url()),
        atomic(pair()),
    );
    let (quotes, gap) = ([quoted(0), quoted(1), quoted(2)], gap());
    let token = move |s: &mut State<'_, 'a>| {
        let c = s.input.chars().next()?;
        if let Some(n) = QUOTES.iter().position(|&q| q == c) {
            // a quote that doesn't close is a character like another
            return quotes[n](s).or_else(|| {
                s.input = &s.input[1..];
                Some(())
            });
        }
        if !is_word(c) {
            return gap(s);
        }
        let run = word_len(s.input);
        let next = s.input[run..].chars().next();
        let hex = s.input[..run].bytes().all(|b| b.is_ascii_hexdigit());
        let uuid_ahead = hex && run == 8 && next == Some('-');
        let hash_ahead = hex && run >= 7;
        let ipv6_ahead = hex && next == Some(':');
        let found = if c.is_ascii_digit() {
            uuid_ahead && uuid(s).is_some()
                || next == Some('.') && ipv4(s).is_some()
                || run == 4 && matches!(next, Some('-' | '/')) && datetime(s).is_some()
                || run == 2 && next == Some(':') && time(s).is_some()
                || hash_ahead && hash(s).is_some()
                || number(s).is_some()
                || ipv6_ahead && ipv6(s).is_some()
        } else {
            matches!(next, Some(':' | '+' | '.' | '-')) && url(s).is_some()
                || uuid_ahead && uuid(s).is_some()
                || hash_ahead && hash(s).is_some()
                || matches!(next, Some('=' | '.' | '-')) && pair(s).is_some()
                || ipv6_ahead && ipv6(s).is_some()
        };
        if !found {
            word(s, run);
        }
        Some(())
    };
    many(token, true, no_separator()).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The marked pieces of `line`, as `(text, kind)`.
    fn marked(line: &str) -> Vec<(&str, Kind)> {
        let mut spans = Vec::new();
        tokens(line, &mut spans);
        let mut last = 0;
        for s in &spans {
            assert!(s.start >= last && s.end > s.start, "{line}: {spans:?}");
            last = s.end;
        }
        assert!(last as usize <= line.len());
        spans
            .iter()
            .map(|s| (&line[s.start as usize..s.end as usize], s.kind))
            .collect()
    }

    use Kind::*;

    #[test]
    fn journalctl_short_iso() {
        assert_eq!(
            marked(
                "2026-09-26T19:46:46+02:00 smol llama-server[1104]: 765.29.141.124 I srv proxying to model on port 35717"
            ),
            [
                ("2026-09-26", Date),
                ("T", DateSep),
                ("19:46:46+02:00", Time),
                ("smol", Host),
                ("llama-server", Unit),
                ("[", Punct),
                ("1104", Pid),
                ("]", Punct),
                (":", Punct),
                ("765.29.141.124", Number),
                ("35717", Number),
            ]
        );
    }

    #[test]
    fn syslog() {
        assert_eq!(
            marked("Oct  8 09:12:01 birb kernel: usb 1-1: new device"),
            [
                ("Oct  8", Date),
                ("09:12:01", Time),
                ("birb", Host),
                ("kernel", Unit),
                (":", Punct),
                ("1", Number),
                ("1", Number),
            ]
        );
    }

    #[test]
    fn tracing() {
        assert_eq!(
            marked("2026-10-18T09:12:01.123456Z  WARN ada::app: slow frame took=12ms"),
            [
                ("2026-10-18", Date),
                ("T", DateSep),
                ("09:12:01.123456Z", Time),
                ("WARN", Warning),
                ("ada::app", Unit),
                (":", Punct),
                ("took", Key),
                ("=", Assign),
                ("12ms", Number),
            ]
        );
    }

    #[test]
    fn levels_in_brackets_and_alone() {
        assert_eq!(
            marked("[2026-10-18 09:12:01] [ERROR] gave up"),
            [
                ("[", Punct),
                ("2026-10-18", Date),
                ("09:12:01", Time),
                ("]", Punct),
                ("[", Punct),
                ("ERROR", Error),
                ("]", Punct),
            ]
        );
        assert_eq!(marked("INFO starting"), [("INFO", Info)]);
        assert_eq!(
            marked("<debug> x"),
            [("<", Punct), ("debug", Debug), (">", Punct)]
        );
        // python's default
        assert_eq!(
            marked("2026-10-18 09:12:01,123 - root - WARNING - low disk"),
            [
                ("2026-10-18", Date),
                ("09:12:01,123", Time),
                ("WARNING", Warning),
            ]
        );
    }

    #[test]
    fn glog_and_dmesg() {
        assert_eq!(
            marked("E1018 09:12:01.123456  1234 main.cc:12] bad"),
            [
                ("E", Error),
                ("1018", Date),
                ("09:12:01.123456", Time),
                ("1234", Number),
                ("12", Number),
            ]
        );
        assert_eq!(
            marked("[    0.000000] Linux version 6.1"),
            [
                ("[", Punct),
                ("0.000000", Time),
                ("]", Punct),
                ("6.1", Number)
            ]
        );
    }

    #[test]
    fn message_words() {
        assert_eq!(
            marked("12:00:01 Failed to start: java.io.IOException at Foo, see warning"),
            [
                ("12:00:01", Time),
                ("Failed", Error),
                ("IOException", Error),
                ("warning", Warning),
            ]
        );
        // lower-case info and debug are words, capitals a level
        assert_eq!(
            marked("a debug build, info only, DEBUG on"),
            [("DEBUG", Debug)]
        );
        assert_eq!(
            marked("msg level=warn lvl=x"),
            [
                ("level", Key),
                ("=", Assign),
                ("warn", Warning),
                ("lvl", Key),
                ("=", Assign),
            ]
        );
    }

    #[test]
    fn strings() {
        assert_eq!(
            marked(r#"said "hi \"you\"" and 'it's' don't `x`"#),
            [
                ("\"", Quote),
                ("hi ", Str),
                ("\\\"", Escape),
                ("you", Str),
                ("\\\"", Escape),
                ("\"", Quote),
                ("'", Quote),
                ("it", Str),
                ("'", Str),
                ("s", Str),
                ("'", Quote),
                ("`", Quote),
                ("x", Str),
                ("`", Quote),
            ]
        );
        // open quotes aren't strings
        assert_eq!(marked(r#"a "b 'c d 5"#), [("5", Number)]);
        assert_eq!(marked("the dogs' bowls 'x"), []);
    }

    #[test]
    fn numbers_and_hex() {
        assert_eq!(
            marked("status 0x40 at 0XdeadBEEF, 3.14 v15 12th 6.2KiB 40%"),
            [
                ("0x40", Hex),
                ("0XdeadBEEF", Hex),
                ("3.14", Number),
                ("6.2KiB", Number),
                ("40", Number),
            ]
        );
        assert_eq!(
            marked("commit 4b7f62b sha256:9f86d081884c7d65 cafe added deadbeef"),
            [("4b7f62b", Hash), ("9f86d081884c7d65", Hash)]
        );
        assert_eq!(
            marked("req a1b2c3d4-e5f6-1234-5678-9abcdef01234 done"),
            [("a1b2c3d4-e5f6-1234-5678-9abcdef01234", Uuid)]
        );
    }

    #[test]
    fn addresses() {
        assert_eq!(
            marked("from 192.168.1.1:8080 and fe80::1 to 10.0.0.0/8, not 1.2.3.4.5 or ada::app"),
            [
                ("192.168.1.1:8080", Ip),
                ("fe80::1", Ip),
                ("10.0.0.0/8", Ip),
                ("1.2.3.4.5", Number),
            ]
        );
        assert_eq!(
            marked("see (https://example.com/a_(b)?q=1). or http://x.org:80/p, done"),
            [
                ("https://example.com/a_(b)?q=1", Url),
                ("http://x.org:80/p", Url),
            ]
        );
    }

    #[test]
    fn maven() {
        assert_eq!(
            marked("[ERROR] Failed to execute goal on project ada: 2 tests failed"),
            [
                ("[", Punct),
                ("ERROR", Error),
                ("]", Punct),
                ("Failed", Error),
                ("2", Number),
                ("failed", Error),
            ]
        );
        assert_eq!(
            marked("[INFO] BUILD SUCCESS"),
            [
                ("[", Punct),
                ("INFO", Info),
                ("]", Punct),
                ("SUCCESS", Info)
            ]
        );
        assert_eq!(
            marked("[WARNING] /src/A.java:[12,5] unchecked"),
            [
                ("[", Punct),
                ("WARNING", Warning),
                ("]", Punct),
                ("12", Number),
                ("5", Number),
            ]
        );
    }

    /// A long run of word characters and `.`, `-` or `:` with no space in it is read once, not
    /// once from each word (O-50: 16 KiB of `a.` took 217 ms, and `1.` up to a `1x` 437). What
    /// the scanners mark inside such runs stays: an address from a later group, hex after a
    /// dotted number, a key after one that failed.
    #[test]
    fn long_runs_are_read_once() {
        assert_eq!(marked("f:f:f:f:f:f:f:f:1"), [("f:f:f:f:f:f:f:1", Ip)]);
        assert_eq!(marked("1.0x1f"), [("0x1f", Hex)]);
        assert_eq!(
            marked("a.b.c==d.e=1"),
            [("d.e", Key), ("=", Assign), ("1", Number)]
        );
        for run in ["a.", "a-", "a:", "f:", "a::", "a1b2c3d4-", "1."] {
            let line = run.repeat((1 << 16) / run.len()) + "1x";
            marked(&line);
        }
    }

    #[test]
    fn anything_goes() {
        for line in [
            "",
            " ",
            "\"",
            "'",
            "[",
            "2026-",
            "Oct",
            "12:",
            "0x",
            "a=",
            "é'ü \"\\",
            "ünïcödé wörds 12µs",
            "level=",
            "INFO",
            "x::",
            "http://",
        ] {
            marked(line);
        }
    }
}
