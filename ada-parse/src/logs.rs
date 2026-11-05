//! Log lines (O-43): a one-pass scanner in place of bat's `Log` grammar, whose regexes took
//! ~0.5 ms a line on Oniguruma. A line is a lead (a timestamp; a level, maybe with a tracing
//! target after it; or syslog's and journalctl's `host unit[pid]:`), then a message scanned for
//! strings, numbers, hex, UUIDs, IP addresses, URLs, `key=value` pairs and level words.
//!
//! Bytes, not characters (X-044; anpa's combinators until then, the spans the same): one class
//! table for every byte test, a timestamp tried by the byte it starts with, a word of the
//! message read once with its shape (its length, all hex, all capitals) and the scanners that
//! might take it chosen from that and the byte after it, level words compared eight bytes at a
//! time, a string's body sixteen. Characters past ASCII are decoded only where one decides
//! something.
//!
//! The message's bytes sixty-four at a time (X-047): SSE2 classifies them into bit masks (word,
//! hex, digit, and where a gap stops), and a word's length and shape and a gap's end are bits
//! counted, not bytes read one at a time. A plain number is taken from its masks, a word is
//! told from the flags by its length and ends before it is folded, and a URL's scheme is read
//! only where a `://` follows on the line.
//!
//! A line depends on no other: a string left open isn't a string. Each [`Kind`] carries the
//! scopes bat's grammar gave the same text where it had one, so `[syntax]` rules written for it
//! still apply.

use crate::Span;
use crate::prim::{char_len, find_in, stops, wide_char, wide_char_at, wide_space};

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
    pub const fn scopes(self) -> &'static str {
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

/// Each kind's scopes, by its number, for [`crate::Lang::kinds`].
pub const KINDS: [&str; Kind::ALL.len()] = {
    let mut out = [""; Kind::ALL.len()];
    let mut i = 0;
    while i < out.len() {
        out[i] = Kind::ALL[i].scopes();
        i += 1;
    }
    out
};

/// The spans of one line, in order, into `spans` (cleared first).
pub fn tokens(line: &str, spans: &mut Vec<Span>) {
    spans.clear();
    let p = lead(line.as_bytes(), spans);
    message(line, spans, p);
}

/// `f`'s spans from `spans`, all or nothing: on failure, the spans it pushed are dropped.
#[inline(always)]
fn attempt(
    spans: &mut Vec<Span>,
    f: impl FnOnce(&mut Vec<Span>) -> Option<usize>,
) -> Option<usize> {
    let n = spans.len();
    let r = f(spans);
    if r.is_none() {
        spans.truncate(n);
    }
    r
}

/// Where the run of spaces from `i` ends.
fn spaces(b: &[u8], mut i: usize) -> usize {
    while b.get(i) == Some(&b' ') {
        i += 1;
    }
    i
}

/// The end of the `n` ASCII digits at `i`.
#[inline(always)]
fn digits(b: &[u8], i: usize, n: usize) -> Option<usize> {
    let d = b.get(i..i + n)?;
    d.iter().all(u8::is_ascii_digit).then_some(i + n)
}

// --- timestamps

/// `2026-10-18` or `2026/10/18`.
fn ymd(b: &[u8], p: usize) -> Option<usize> {
    let i = digits(b, p, 4)?;
    let sep = *b.get(i).filter(|&&c| c == b'-' || c == b'/')?;
    let i = digits(b, i + 1, 2)?;
    if b.get(i) != Some(&sep) {
        return None;
    }
    digits(b, i + 1, 2)
}

/// `09:12:01`, with a fraction (`.123`, `,123`) and a zone (`Z`, `+02:00`, `-0700`); the seconds
/// may be left out only when `secs` is false.
fn clock(b: &[u8], p: usize, secs: bool) -> Option<usize> {
    let i = digits(b, p, 2)?;
    if b.get(i) != Some(&b':') {
        return None;
    }
    let mut i = digits(b, i + 1, 2)?;
    match digits(b, i + 1, 2).filter(|_| b.get(i) == Some(&b':')) {
        Some(e) => i = e,
        None if secs => return None,
        None => {}
    }
    if matches!(b.get(i), Some(b'.' | b',')) && is(b, i + 1, DIGIT) {
        i = run_end(b, i + 1, DIGIT);
    }
    match b.get(i) {
        Some(b'Z') => i += 1,
        Some(b'+' | b'-') => {
            if let Some(h) = digits(b, i + 1, 2) {
                let m = if b.get(h) == Some(&b':') { h + 1 } else { h };
                i = digits(b, m, 2).unwrap_or(h);
            }
        }
        _ => {}
    }
    Some(i)
}

/// An ISO date, and the time after it.
fn datetime(b: &[u8], spans: &mut Vec<Span>, p: usize) -> Option<usize> {
    let date = ymd(b, p)?;
    put(spans, p, date, Kind::Date);
    let mut e = date;
    if let Some(&sep @ (b'T' | b' ')) = b.get(date)
        && let Some(t) = clock(b, date + 1, false)
    {
        if sep == b'T' {
            put(spans, date, date + 1, Kind::DateSep);
        }
        put(spans, date + 1, t, Kind::Time);
        e = t;
    }
    (!word_at(b, e)).then_some(e)
}

const MONTHS: [[u8; 3]; 12] = [
    *b"Jan", *b"Feb", *b"Mar", *b"Apr", *b"May", *b"Jun", *b"Jul", *b"Aug", *b"Sep", *b"Oct",
    *b"Nov", *b"Dec",
];

/// syslog's `Oct 18 09:12:01`, `Oct  8 09:12:01`.
fn syslog(b: &[u8], spans: &mut Vec<Span>, p: usize) -> Option<usize> {
    let month = b.get(p..p + 3)?;
    if run_end(b, p, UPPER | LOWER) != p + 3 || !MONTHS.iter().any(|m| m == month) {
        return None;
    }
    let ok = b.get(p + 3) == Some(&b' ')
        && matches!(b.get(p + 4), Some(b' ' | b'0'..=b'9'))
        && is(b, p + 5, DIGIT)
        && b.get(p + 6) == Some(&b' ');
    if !ok {
        return None;
    }
    let t = clock(b, p + 7, true)?;
    put(spans, p, p + 6, Kind::Date);
    put(spans, p + 7, t, Kind::Time);
    (!word_at(b, t)).then_some(t)
}

/// A time with no date.
fn time(b: &[u8], spans: &mut Vec<Span>, p: usize) -> Option<usize> {
    let t = clock(b, p, true).filter(|&t| !word_at(b, t))?;
    put(spans, p, t, Kind::Time);
    Some(t)
}

/// glog's `I1018 09:12:01.123456`: the level's letter, then month and day.
fn glog(b: &[u8], spans: &mut Vec<Span>, p: usize) -> Option<usize> {
    let level = match b.get(p)? {
        b'I' => Kind::Info,
        b'W' => Kind::Warning,
        b'E' | b'F' => Kind::Error,
        _ => return None,
    };
    let date = digits(b, p + 1, 4).filter(|&d| b.get(d) == Some(&b' '))?;
    put(spans, p, p + 1, level);
    put(spans, p + 1, date, Kind::Date);
    time(b, spans, date + 1)
}

/// dmesg's `[   12.345678]`.
fn uptime(b: &[u8], spans: &mut Vec<Span>, p: usize) -> Option<usize> {
    let s = spaces(b, p + 1);
    let whole = run_end(b, s, DIGIT);
    if whole == s || b.get(whole) != Some(&b'.') {
        return None;
    }
    let t = run_end(b, whole + 1, DIGIT);
    if t == whole + 1 || b.get(t) != Some(&b']') {
        return None;
    }
    put(spans, p, p + 1, Kind::Punct);
    put(spans, s, t, Kind::Time);
    put(spans, t, t + 1, Kind::Punct);
    Some(t + 1)
}

/// `[` and a date and time or a time, then `]`.
fn bracketed(b: &[u8], spans: &mut Vec<Span>, p: usize) -> Option<usize> {
    put(spans, p, p + 1, Kind::Punct);
    let e = attempt(spans, |s| datetime(b, s, p + 1)).or_else(|| time(b, spans, p + 1))?;
    if b.get(e) != Some(&b']') {
        return None;
    }
    put(spans, e, e + 1, Kind::Punct);
    Some(e + 1)
}

/// A timestamp, maybe in brackets, and whether syslog or journalctl writes it, which a
/// `host unit[pid]:` may follow; each kind tried by the byte it starts with.
fn stamp(b: &[u8], spans: &mut Vec<Span>, p: usize) -> Option<(usize, bool)> {
    match *b.get(p)? {
        b'0'..=b'9' => attempt(spans, |s| datetime(b, s, p))
            .map(|e| (e, true))
            .or_else(|| time(b, spans, p).map(|e| (e, false))),
        b'[' => attempt(spans, |s| bracketed(b, s, p))
            .or_else(|| uptime(b, spans, p))
            .map(|e| (e, false)),
        b'A'..=b'Z' => attempt(spans, |s| syslog(b, s, p))
            .map(|e| (e, true))
            .or_else(|| attempt(spans, |s| glog(b, s, p)).map(|e| (e, false))),
        _ => None,
    }
}

// --- the lead

/// A level word at `i` where a level is expected, in any case, then the end of a word.
fn level_word(b: &[u8], spans: &mut Vec<Span>, i: usize) -> Option<usize> {
    let e = run_end(b, i, UPPER | LOWER);
    if e == i || e - i > 8 || word_at(b, e) {
        return None;
    }
    let kind = level_of(fold_at(b, i, e - i))?;
    put(spans, i, e, kind);
    Some(e)
}

/// A level word, alone or in `[]`, `<>` or `()`.
fn level_tag(b: &[u8], spans: &mut Vec<Span>, i: usize) -> Option<usize> {
    let close = match *b.get(i)? {
        b'[' => b']',
        b'<' => b'>',
        b'(' => b')',
        _ => return level_word(b, spans, i),
    };
    attempt(spans, |spans| {
        put(spans, i, i + 1, Kind::Punct);
        let w = level_word(b, spans, spaces(b, i + 1))?;
        let c = spaces(b, w);
        if b.get(c) != Some(&close) {
            return None;
        }
        put(spans, c, c + 1, Kind::Punct);
        Some(c + 1)
    })
}

/// tracing's target at `i`, `ada::app:`, then a space or the end: a name of word characters,
/// `:`, `.`, `-` and `/`, not a level or error word.
fn target(b: &[u8], spans: &mut Vec<Span>, i: usize) -> Option<usize> {
    let mut j = i;
    while let Some(&c) = b.get(j) {
        if c < 0x80 {
            if CLASS[c as usize] & WORD == 0 && !matches!(c, b':' | b'.' | b'-' | b'/') {
                break;
            }
            j += 1;
        } else {
            if !wide_word(b, j) {
                break;
            }
            j += char_len(c);
        }
    }
    let ok = j > i
        && b.get(j).is_none_or(|&c| c == b' ')
        && b[j - 1] == b':'
        && word_at(b, i)
        && message_level(b, i, j - 1 - i).is_none();
    if !ok {
        return None;
    }
    put(spans, i, j - 1, Kind::Unit);
    put(spans, j - 1, j, Kind::Punct);
    Some(j)
}

/// Where a syslog unit's characters from `i` end: anything but blanks, `[` and `:`.
fn unit_end(b: &[u8], mut i: usize) -> usize {
    while let Some(&c) = b.get(i) {
        if c < 0x80 {
            if c.is_ascii_whitespace() || c == b'\x0b' || c == b'[' || c == b':' {
                break;
            }
            i += 1;
        } else {
            if char::from_u32(decode(b, i)).is_some_and(char::is_whitespace) {
                break;
            }
            i += char_len(c);
        }
    }
    i
}

/// syslog's and journalctl's `host unit[pid]: ` at `i`.
fn source(b: &[u8], spans: &mut Vec<Span>, i: usize) -> Option<usize> {
    let host = key_end(b, i);
    if host == i || b.get(host) != Some(&b' ') {
        return None;
    }
    let u = host + 1;
    let unit = unit_end(b, u);
    if unit == u {
        return None;
    }
    put(spans, i, host, Kind::Host);
    put(spans, u, unit, Kind::Unit);
    let mut e = unit;
    if b.get(e) == Some(&b'[') {
        let d = run_end(b, e + 1, DIGIT);
        if d > e + 1 && b.get(d) == Some(&b']') {
            put(spans, e, e + 1, Kind::Punct);
            put(spans, e + 1, d, Kind::Pid);
            put(spans, d, d + 1, Kind::Punct);
            e = d + 1;
        }
    }
    if b.get(e) != Some(&b':') || b.get(e + 1).is_some_and(|&c| c != b' ') {
        return None;
    }
    put(spans, e, e + 1, Kind::Punct);
    Some(e + 1)
}

/// What a line starts with: a timestamp, then a level and a target, or a journal's source.
/// Where the message starts.
fn lead(b: &[u8], spans: &mut Vec<Span>) -> usize {
    let (mut p, journal) = stamp(b, spans, spaces(b, 0)).unwrap_or((0, false));
    let i = spaces(b, p);
    let leveled = attempt(spans, |spans| {
        let e = level_tag(b, spans, i)?;
        let s = spaces(b, e);
        Some(
            (s > e)
                .then(|| attempt(spans, |spans| target(b, spans, s)))
                .flatten()
                .unwrap_or(e),
        )
    });
    if let Some(e) = leveled.or_else(|| {
        journal
            .then(|| attempt(spans, |spans| source(b, spans, i)))
            .flatten()
    }) {
        p = e;
    }
    p
}

// --- levels

/// A word of up to eight bytes as one number, its letters in lower case: `ERROR`, `Error` and
/// `error` are all `fold(b"error")`, so a word is told by a compare a candidate. The bytes past
/// the word are zero, which no byte of a word is, so `err` isn't `error`. Setting bit 5 of
/// every byte makes a letter lower case and leaves no other byte a letter, so it matches only
/// where a case-blind compare to a lower-case word would.
const fn fold(w: &[u8]) -> u64 {
    let mut k = 0;
    let mut i = 0;
    while i < w.len() && i < 8 {
        k |= ((w[i] | 0x20) as u64) << (8 * i);
        i += 1;
    }
    k
}

/// [`fold`] of the `n <= 8` bytes of `b` at `p`, in one load where eight bytes are left.
#[inline]
fn fold_at(b: &[u8], p: usize, n: usize) -> u64 {
    match b.get(p..p + 8) {
        Some(w) => {
            let k = u64::from_le_bytes(w.try_into().unwrap_or([0; 8])) | 0x2020_2020_2020_2020;
            if n >= 8 { k } else { k & ((1 << (8 * n)) - 1) }
        }
        None => fold(&b[p..p + n]),
    }
}

/// The level a [`fold`]ed word of up to eight bytes names.
fn level_of(k: u64) -> Option<Kind> {
    const LEVELS: [(u64, Kind); 22] = [
        (fold(b"error"), Kind::Error),
        (fold(b"err"), Kind::Error),
        (fold(b"eror"), Kind::Error),
        (fold(b"fatal"), Kind::Error),
        (fold(b"crit"), Kind::Error),
        (fold(b"critical"), Kind::Error),
        (fold(b"emerg"), Kind::Error),
        (fold(b"alert"), Kind::Error),
        (fold(b"panic"), Kind::Error),
        (fold(b"severe"), Kind::Error),
        (fold(b"warn"), Kind::Warning),
        (fold(b"warning"), Kind::Warning),
        (fold(b"wrn"), Kind::Warning),
        (fold(b"info"), Kind::Info),
        (fold(b"inf"), Kind::Info),
        (fold(b"notice"), Kind::Info),
        (fold(b"debug"), Kind::Debug),
        (fold(b"dbg"), Kind::Debug),
        (fold(b"trace"), Kind::Debug),
        (fold(b"trc"), Kind::Debug),
        (fold(b"verbose"), Kind::Debug),
        (fold(b"fine"), Kind::Debug),
    ];
    LEVELS.iter().find(|&&(l, _)| l == k).map(|&(_, kind)| kind)
}

/// A word in a message that flags a level: error and warning words in any case (`failed`,
/// `Exception`, `IOException`), the other levels in capitals (`INFO`, `DEBUG`). The word is
/// the `n` bytes of `b` at `p`. Most words are told apart before [`flagged`] by their ends and
/// length: a long one by its last byte (the suffixes' `n` and `r`), a lower-case one by the
/// lengths of the flags that start with its letter, a capitalized one (`Connection`) by its
/// last byte (`KeyError`'s `r`) unless its letter starts a flag, and one that starts with
/// anything but a letter never.
#[inline(always)]
fn message_level(b: &[u8], p: usize, n: usize) -> Option<Kind> {
    let w = b.get(p..p + n).filter(|w| !w.is_empty())?;
    let (first, last) = (w[0], w[n - 1]);
    let maybe = if n > 9 {
        matches!(last, b'n' | b'r')
    } else if first.is_ascii_lowercase() {
        FLAG_LENGTHS[usize::from(first - b'a')] >> n & 1 != 0
    } else if first.is_ascii_uppercase() {
        n > 2
            && (!w[1].is_ascii_lowercase()
                || matches!(first, b'E' | b'F' | b'C' | b'P' | b'W')
                || last == b'r')
    } else {
        false
    };
    if maybe { flagged(b, p, n) } else { None }
}

/// The lengths of the lower-case flags, a bit each, by their first letter: `error` and
/// `exception`; `fail`, `fatal`, `failed` and `failure`; `critical`; `panic` and `panicked`;
/// `warn` and `warning`.
const FLAG_LENGTHS: [u16; 26] = {
    let mut t = [0; 26];
    t[(b'e' - b'a') as usize] = 1 << 5 | 1 << 9;
    t[(b'f' - b'a') as usize] = 1 << 4 | 1 << 5 | 1 << 6 | 1 << 7;
    t[(b'c' - b'a') as usize] = 1 << 8;
    t[(b'p' - b'a') as usize] = 1 << 5 | 1 << 8;
    t[(b'w' - b'a') as usize] = 1 << 4 | 1 << 7;
    t
};

/// [`message_level`] past the words its tests tell apart.
#[inline(never)]
fn flagged(b: &[u8], p: usize, n: usize) -> Option<Kind> {
    let w = &b[p..p + n];
    let first = w[0];
    if n > 9 {
        // `IOException`, `ValueError`: the suffix by its last byte, then eight bytes at once
        let tail = u64::from_le_bytes(w[n - 8..].try_into().unwrap_or([0; 8]));
        let suffix = match w[n - 1] {
            b'n' if tail == u64::from_le_bytes(*b"xception") && w[n - 9] == b'E' => 9,
            b'r' if tail >> 24 == u64::from_le_bytes(*b"Error\0\0\0") => 5,
            _ => return None,
        };
        return ends_alphanumeric(&w[..n - suffix]).then_some(Kind::Error);
    }
    // a short CamelCase error type: `IOError`, `KeyError`
    let camel_error = n > 5 && first.is_ascii_uppercase() && w[n - 5..] == *b"Error";
    if n == 9 {
        let exception = fold_at(b, p, 8) == fold(b"exceptio") && w[8] | 0x20 == b'n';
        return (exception || camel_error).then_some(Kind::Error);
    }
    // the error and warning words in any case, by their first letter
    let k = fold_at(b, p, n);
    let error = match first | 0x20 {
        b'e' => k == const { fold(b"error") },
        b'f' => {
            k == const { fold(b"fail") }
                || k == const { fold(b"failed") }
                || k == const { fold(b"failure") }
                || k == const { fold(b"fatal") }
        }
        b'c' => k == const { fold(b"critical") },
        b'p' => k == const { fold(b"panic") } || k == const { fold(b"panicked") },
        b'w' if k == const { fold(b"warn") } || k == const { fold(b"warning") } => {
            return Some(Kind::Warning);
        }
        _ => false,
    };
    if error || camel_error {
        return Some(Kind::Error);
    }
    if n > 2
        && first.is_ascii_uppercase()
        && k != const { fold(b"fine") }
        && w.iter().all(u8::is_ascii_uppercase)
    {
        // maven's `BUILD SUCCESS`, a test runner's `PASSED`
        if k == const { fold(b"success") } || k == const { fold(b"passed") } {
            return Some(Kind::Info);
        }
        return level_of(k);
    }
    None
}

/// Whether `w`'s last character is a letter or a digit.
fn ends_alphanumeric(w: &[u8]) -> bool {
    match w.last() {
        None => false,
        Some(&c) if c < 0x80 => c.is_ascii_alphanumeric(),
        Some(_) => wide_char_at(w, w.len() - 1).is_some_and(char::is_alphanumeric),
    }
}

// --- the message

/// A byte's classes in a message, a bit each; a byte past ASCII has none (its character is
/// decoded where it counts).
const WORD: u8 = 1;
const HEX: u8 = 2;
const DIGIT: u8 = 4;
const UPPER: u8 = 8;
const LOWER: u8 = 16;
const QUOTE: u8 = 32;
/// A `key=value` key's: a word's, `.` and `-`.
const KEY: u8 = 64;
/// A URL scheme's: letters, digits, `+`, `.` and `-`.
const SCHEME: u8 = 128;

static CLASS: [u8; 256] = {
    let mut t = [0; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        let mut k = 0;
        if b.is_ascii_alphanumeric() || b == b'_' {
            k |= WORD | KEY;
        }
        if b.is_ascii_hexdigit() {
            k |= HEX;
        }
        if b.is_ascii_digit() {
            k |= DIGIT;
        }
        if b.is_ascii_uppercase() {
            k |= UPPER;
        }
        if b.is_ascii_lowercase() {
            k |= LOWER;
        }
        if matches!(b, b'"' | b'\'' | b'`') {
            k |= QUOTE;
        }
        if matches!(b, b'.' | b'-') {
            k |= KEY;
        }
        if b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-') {
            k |= SCHEME;
        }
        t[c] = k;
        c += 1;
    }
    t
};

/// Whether `b`'s byte `i` (a character's first) is in `class`.
#[inline(always)]
fn is(b: &[u8], i: usize, class: u8) -> bool {
    b.get(i).is_some_and(|&c| CLASS[c as usize] & class != 0)
}

/// Whether the character at byte `i` of `b` is a word character, `\w` as `char` reads it
/// (false at the end).
#[inline(always)]
fn word_at(b: &[u8], i: usize) -> bool {
    match b.get(i) {
        Some(&c) if c < 0x80 => CLASS[c as usize] & WORD != 0,
        Some(_) => wide_word(b, i),
        None => false,
    }
}

/// [`word_at`] past ASCII.
#[inline(never)]
fn wide_word(b: &[u8], i: usize) -> bool {
    let c = decode(b, i);
    !NOT_WORD.iter().any(|r| r.contains(&c)) && char::from_u32(c).is_some_and(char::is_alphanumeric)
}

/// Blocks with no letter or digit in them, where a log's characters past ASCII mostly are
/// (box drawing, `▶`, `—`, `→`), answered without Unicode's tables: General Punctuation;
/// Arrows, Mathematical Operators and Miscellaneous Technical; Box Drawing, Block Elements and
/// Geometric Shapes.
const NOT_WORD: [std::ops::RangeInclusive<u32>; 3] =
    [0x2000..=0x206f, 0x2190..=0x23ff, 0x2500..=0x25ff];

/// The code point of the character at byte `i` of `b`, past ASCII (a `str`'s bytes, so valid
/// UTF-8 and no check).
#[inline]
fn decode(b: &[u8], i: usize) -> u32 {
    let tail = |k: usize| u32::from(b.get(i + k).copied().unwrap_or(0x80) & 0x3f);
    let c = u32::from(b[i]);
    match c {
        ..=0xdf => (c & 0x1f) << 6 | tail(1),
        0xe0..=0xef => (c & 0x0f) << 12 | tail(1) << 6 | tail(2),
        _ => (c & 0x07) << 18 | tail(1) << 12 | tail(2) << 6 | tail(3),
    }
}

/// Where the run of bytes in `class` from `i` ends.
#[inline(always)]
fn run_end(b: &[u8], mut i: usize, class: u8) -> usize {
    while is(b, i, class) {
        i += 1;
    }
    i
}

/// Scanners that read a run of characters, then fail or not by what comes after it. A start
/// later in the same run reads it to the same end and fails the same way, so it fails at once
/// instead of reading it again: without this, a line of `a.a.a…` read the rest of the line
/// from each word (O-50). Each is the offset the last run that scanner failed on ends at.
#[derive(Default)]
struct Dead {
    /// [`Scan::url`]'s scheme.
    url: usize,
    /// [`Scan::pair`]'s key.
    pair: usize,
    /// [`Scan::number`]'s decimal: its digits and dotted parts.
    decimal: usize,
}

/// The message's scan: the line, the quotes known not to close (a bit each, `"`, `'` and
/// `` ` ``), and where the runs that scanners failed on end.
struct Scan<'a> {
    line: &'a str,
    b: &'a [u8],
    open: u8,
    dead: Dead,
    /// Where the next `://` from the last [`Scan::url`] is (the line's length when there is
    /// none), found when a URL's scheme might end there and not before.
    sep: usize,
    /// The 64 bytes' masks the loop reads words and gaps from, at or before its position.
    w: Window,
}

/// Bit masks of 64 bytes of a line from `base`, bit `i` for byte `base + i`: word bytes
/// (ASCII's `\w`), hex digits, digits, and the bytes a gap stops at (a word byte, a quote, a
/// byte past ASCII, and every bit past the line's end). A message's words and gaps are read
/// off them by counting bits, so a byte is classified once, sixteen a step, and not once a
/// test.
#[derive(Clone, Copy)]
struct Window {
    base: usize,
    word: u64,
    hex: u64,
    digit: u64,
    stop: u64,
}

impl Window {
    /// The masks of `b`'s 64 bytes from `base`: where fewer are left, the line's last 64
    /// shifted down, or a short line's bytes copied out first.
    #[inline(never)]
    fn at(b: &[u8], base: usize) -> Window {
        if let Some(c) = b.get(base..base + 64) {
            return Window::of(base, c, 0);
        }
        let left = b.len() - base;
        if left == 0 {
            return Window {
                base,
                word: 0,
                hex: 0,
                digit: 0,
                stop: !0,
            };
        }
        let mut w = match b.len().checked_sub(64) {
            Some(q) => Window::of(base, &b[q..], base - q),
            None => {
                let mut pad = [0; 64];
                pad[..left].copy_from_slice(&b[base..]);
                Window::of(base, &pad, 0)
            }
        };
        w.stop |= !0 << left;
        w
    }

    /// The masks of the 64 bytes `c`, shifted down `shift` bits.
    #[inline(always)]
    fn of(base: usize, c: &[u8], shift: usize) -> Window {
        let (mut word, mut hex, mut digit, mut stop) = (0, 0, 0, 0);
        for k in 0..4 {
            let [wm, hm, dm, sm] = classify16(&c[16 * k..16 * k + 16]).map(u64::from);
            word |= wm << (16 * k);
            hex |= hm << (16 * k);
            digit |= dm << (16 * k);
            stop |= sm << (16 * k);
        }
        Window {
            base,
            word: word >> shift,
            hex: hex >> shift,
            digit: digit >> shift,
            stop: stop >> shift,
        }
    }

    /// The window that holds `p`, at or past this one: the next on the grid when `p` is in it,
    /// so each byte is classified once.
    #[inline(always)]
    fn seek(&mut self, b: &[u8], p: usize) {
        let off = p - self.base;
        if off >= 64 {
            *self = Window::at(b, if off < 128 { self.base + 64 } else { p });
        }
    }
}

/// Word, hex, digit and stop masks of 16 bytes: SSE2, which every x86-64 has; the class table
/// elsewhere.
#[inline(always)]
fn classify16(c: &[u8]) -> [u32; 4] {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::*;
        // SAFETY: SSE2 is part of x86-64, and `c` has 16 bytes (each call), read unaligned
        unsafe {
            let v = _mm_loadu_si128(c.as_ptr().cast());
            // `lo..=hi` in one signed compare: the range moved to the bottom of i8's
            let range = |x: __m128i, lo: u8, hi: u8| {
                let t = _mm_add_epi8(x, _mm_set1_epi8(0x80u8.wrapping_sub(lo) as i8));
                _mm_cmplt_epi8(t, _mm_set1_epi8((0x80u8 + (hi - lo + 1)) as i8))
            };
            let folded = _mm_or_si128(v, _mm_set1_epi8(0x20));
            let digit = range(v, b'0', b'9');
            let alpha = range(folded, b'a', b'z');
            let under = _mm_cmpeq_epi8(v, _mm_set1_epi8(b'_' as i8));
            let word = _mm_or_si128(_mm_or_si128(digit, alpha), under);
            let hex = _mm_or_si128(digit, range(folded, b'a', b'f'));
            let quote = _mm_or_si128(
                _mm_or_si128(
                    _mm_cmpeq_epi8(v, _mm_set1_epi8(b'"' as i8)),
                    _mm_cmpeq_epi8(v, _mm_set1_epi8(b'\'' as i8)),
                ),
                _mm_cmpeq_epi8(v, _mm_set1_epi8(b'`' as i8)),
            );
            let word_m = _mm_movemask_epi8(word) as u32;
            [
                word_m,
                _mm_movemask_epi8(hex) as u32,
                _mm_movemask_epi8(digit) as u32,
                word_m | _mm_movemask_epi8(_mm_or_si128(quote, v)) as u32,
            ]
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let mut m = [0; 4];
        for (i, &c) in c.iter().enumerate() {
            let k = CLASS[c as usize];
            m[0] |= u32::from(k & WORD != 0) << i;
            m[1] |= u32::from(k & HEX != 0) << i;
            m[2] |= u32::from(k & DIGIT != 0) << i;
            m[3] |= u32::from(k & (WORD | QUOTE) != 0 || c >= 0x80) << i;
        }
        m
    }
}

fn put(spans: &mut Vec<Span>, start: usize, end: usize, kind: Kind) {
    spans.push(Span {
        start: start as u32,
        end: end as u32,
        kind: kind as u16,
    });
}

/// The rest of the line, a token at a time by its first byte: a quote, a run of bytes that
/// start nothing, or a word. A word's length and shape (all hex digits, all digits) are read
/// off the [`Window`]'s masks, and the byte after it and that shape decide which scanners may
/// take it, each reading from the word's start only when it might.
fn message(line: &str, spans: &mut Vec<Span>, mut p: usize) {
    let mut m = Scan {
        line,
        b: line.as_bytes(),
        open: 0,
        dead: Dead::default(),
        sep: 0,
        w: Window::at(line.as_bytes(), p),
    };
    let b = m.b;
    while let Some(&c) = b.get(p) {
        let class = CLASS[c as usize];
        if class & QUOTE != 0 {
            // a quote that doesn't close is a character like another
            p = m.quoted(spans, p).unwrap_or(p + 1);
            m.w.seek(b, p);
            continue;
        }
        if class & WORD == 0 && (c < 0x80 || !wide_word(b, p)) {
            p = m.gap(p);
            continue;
        }
        let (run, hex, digits) = m.word_run(p, class & DIGIT != 0);
        let next = b.get(p + run).copied().unwrap_or(0);
        if digits && !matches!(next, b'.' | b'-' | b'/' | b':') {
            // a plain number, which nothing before `number` takes and `number` reads whole
            put(spans, p, p + run, Kind::Number);
            p += run;
            continue;
        }
        let uuid_ahead = hex && run == 8 && next == b'-';
        let found = 'found: {
            if class & DIGIT != 0 {
                if uuid_ahead && let Some(e) = m.uuid(spans, p) {
                    break 'found Some(e);
                }
                if next == b'.'
                    && let Some(e) = m.ipv4(spans, p)
                {
                    break 'found Some(e);
                }
                if run == 4
                    && matches!(next, b'-' | b'/')
                    && let Some(e) = attempt(spans, |s| datetime(b, s, p))
                {
                    break 'found Some(e);
                }
                if run == 2
                    && next == b':'
                    && let Some(e) = time(b, spans, p)
                {
                    break 'found Some(e);
                }
                if hex
                    && run >= 7
                    && let Some(e) = m.hash(spans, p, run)
                {
                    break 'found Some(e);
                }
                if let Some(e) = m.number(spans, p) {
                    break 'found Some(e);
                }
            } else {
                if matches!(next, b':' | b'+' | b'.' | b'-')
                    && let Some(e) = m.url(spans, p)
                {
                    break 'found Some(e);
                }
                if uuid_ahead && let Some(e) = m.uuid(spans, p) {
                    break 'found Some(e);
                }
                if hex
                    && run >= 7
                    && let Some(e) = m.hash(spans, p, run)
                {
                    break 'found Some(e);
                }
                if matches!(next, b'=' | b'.' | b'-')
                    && let Some(e) = m.pair(spans, p)
                {
                    break 'found Some(e);
                }
            }
            // two colons at least: a hex digit or a colon after the first
            if hex
                && next == b':'
                && b.get(p + run + 1)
                    .is_some_and(|&c| c == b':' || c.is_ascii_hexdigit())
            {
                break 'found m.ipv6(spans, p);
            }
            None
        };
        p = match found {
            Some(e) => e,
            None => m.word(spans, p, run),
        };
        // a scanner may end past the window; a gap and a word's run end in it
        m.w.seek(b, p);
    }
}

/// The length of the word characters from `p`, and the classes all their bytes share (none
/// once a character past ASCII is among them).
#[inline(always)]
fn word_shape(b: &[u8], p: usize) -> (usize, u8) {
    let mut i = p;
    let mut shape = !0;
    loop {
        while let Some(&c) = b.get(i) {
            let k = CLASS[c as usize];
            if k & WORD == 0 {
                break;
            }
            shape &= k;
            i += 1;
        }
        match b.get(i) {
            Some(&c) if c >= 0x80 && wide_word(b, i) => {
                shape = 0;
                i += char_len(c);
            }
            _ => return (i - p, shape),
        }
    }
}

impl Scan<'_> {
    /// Where a run of characters that start nothing (spaces, punctuation) from `p` ends: the
    /// next stop in the window, a byte past ASCII decoded to tell whether it is one.
    #[inline(always)]
    fn gap(&mut self, mut p: usize) -> usize {
        let b = self.b;
        loop {
            let off = p - self.w.base;
            let z = (self.w.stop >> off).trailing_zeros() as usize;
            if off + z >= 64 {
                p = self.w.base + 64;
                self.w = Window::at(b, p);
                continue;
            }
            p += z;
            match b.get(p) {
                Some(&c) if c >= 0x80 && !wide_word(b, p) => {
                    p += char_len(c);
                    self.w.seek(b, p);
                }
                _ => return p,
            }
        }
    }

    /// The length of the word characters from `p` (one at least), and whether they are all
    /// hex digits, and all digits (only when `digit`, its first is one): the window's run of
    /// word bits, [`word_shape`] where a character past ASCII is among them. The run ends in
    /// the window, which moves on when the word reaches past it.
    #[inline(always)]
    fn word_run(&mut self, p: usize, digit: bool) -> (usize, bool, bool) {
        let b = self.b;
        let off = p - self.w.base;
        let ones = |m: u64| (!m).trailing_zeros() as usize;
        let mut run = ones(self.w.word >> off);
        let mut hex = ones(self.w.hex >> off) >= run;
        let mut digits = digit && ones(self.w.digit >> off) >= run;
        if off + run >= 64 {
            // the word reaches into the next window
            self.w = Window::at(b, self.w.base + 64);
            let more = ones(self.w.word);
            hex &= ones(self.w.hex) >= more;
            digits &= ones(self.w.digit) >= more;
            run += more;
            if more == 64 {
                run = usize::MAX;
            }
        }
        if run != usize::MAX && b.get(p + run).is_none_or(|&c| c < 0x80) {
            return (run, hex, digits);
        }
        // a long word, or one with a character past ASCII, may end past the window
        let (run, shape) = word_shape(b, p);
        self.w.seek(b, p + run);
        (run, shape & HEX != 0, shape & DIGIT != 0)
    }

    /// The word at `p`, its `run` of word characters and then any apostrophes between word
    /// characters (`don't`), which start no string: marked when it's a level or error word.
    fn word(&self, spans: &mut Vec<Span>, p: usize, run: usize) -> usize {
        let b = self.b;
        let mut len = run;
        while b.get(p + len) == Some(&b'\'') {
            match word_shape(b, p + len + 1).0 {
                0 => break,
                n => len += 1 + n,
            }
        }
        if let Some(kind) = message_level(b, p, len) {
            put(spans, p, p + len, kind);
        }
        p + len
    }

    /// A quoted string at `p`, closed on its line: `"` with `\` escapes, `'` and `` ` `` only
    /// after a non-word character and before one. A quote that doesn't close makes every later
    /// one of its kind fail at once.
    fn quoted(&mut self, spans: &mut Vec<Span>, p: usize) -> Option<usize> {
        let b = self.b;
        let q = b[p];
        let bit = match q {
            b'"' => 1,
            b'\'' => 2,
            _ => 4,
        };
        if self.open & bit != 0 {
            return None;
        }
        if q != b'"' && p > 0 && word_before(b, p) {
            return None;
        }
        let n = spans.len();
        let closed = string(b, spans, p, q);
        if closed.is_none() {
            spans.truncate(n);
            self.open |= bit;
        }
        closed
    }

    /// `8-4-4-4-12` hex digits, the first eight read.
    fn uuid(&self, spans: &mut Vec<Span>, p: usize) -> Option<usize> {
        let b = self.b;
        let mut i = p + 8;
        for n in [4, 4, 4, 12] {
            if b.get(i) != Some(&b'-') || run_end(b, i + 1, HEX).min(i + 1 + n) != i + 1 + n {
                return None;
            }
            i += 1 + n;
        }
        if word_at(b, i) {
            return None;
        }
        put(spans, p, i, Kind::Uuid);
        Some(i)
    }

    /// `192.168.1.1`, `10.0.0.1:8080`, `10.0.0.0/8`.
    fn ipv4(&self, spans: &mut Vec<Span>, p: usize) -> Option<usize> {
        let b = self.b;
        let octet = |i: usize| {
            let e = run_end(b, i, DIGIT);
            let n = match e - i {
                1 => u16::from(b[i] - b'0'),
                2 => u16::from(b[i] - b'0') * 10 + u16::from(b[i + 1] - b'0'),
                3 => {
                    u16::from(b[i] - b'0') * 100
                        + u16::from(b[i + 1] - b'0') * 10
                        + u16::from(b[i + 2] - b'0')
                }
                _ => return None,
            };
            (n < 256).then_some(e)
        };
        let mut i = octet(p)?;
        for _ in 0..3 {
            if b.get(i) != Some(&b'.') {
                return None;
            }
            i = octet(i + 1)?;
        }
        // not a dotted version with more parts
        if b.get(i) == Some(&b'.') && is(b, i + 1, DIGIT) {
            return None;
        }
        if matches!(b.get(i), Some(b':' | b'/')) && is(b, i + 1, DIGIT) {
            i = run_end(b, i + 1, DIGIT);
        }
        if word_at(b, i) {
            return None;
        }
        put(spans, p, i, Kind::Ip);
        Some(i)
    }

    /// A hash, a digest or an id: the `run` of 7 or more hex digits at `p`, with a digit among
    /// them and letters all in one case (`4b7f62b`, `4B7F62B`, not `4b7F62b`).
    fn hash(&self, spans: &mut Vec<Span>, p: usize, run: usize) -> Option<usize> {
        let any = self.b[p..p + run]
            .iter()
            .fold(0, |k, &c| k | CLASS[c as usize]);
        let letters = any & (UPPER | LOWER);
        let ok = any & DIGIT != 0 && letters != 0 && letters != (UPPER | LOWER);
        if !ok {
            return None;
        }
        put(spans, p, p + run, Kind::Hash);
        Some(p + run)
    }

    /// `42`, `3.14`, `1.2.3`, `12ms`, `6.2KiB`, `0x40`, at `p`, a digit.
    fn number(&mut self, spans: &mut Vec<Span>, p: usize) -> Option<usize> {
        let b = self.b;
        if b[p] == b'0' && matches!(b.get(p + 1), Some(b'x' | b'X')) {
            let e = run_end(b, p + 2, HEX);
            if e > p + 2 && !word_at(b, e) {
                put(spans, p, e, Kind::Hex);
                return Some(e);
            }
        }
        if p < self.dead.decimal {
            return None;
        }
        let mut i = run_end(b, p, DIGIT);
        while b.get(i) == Some(&b'.') && is(b, i + 1, DIGIT) {
            i = run_end(b, i + 1, DIGIT);
        }
        let u = alphabetic_end(b, i);
        let e = if is_unit(&b[i..u]) { u } else { i };
        if word_at(b, e) {
            self.dead.decimal = i;
            return None;
        }
        put(spans, p, e, Kind::Number);
        Some(e)
    }

    /// `fe80::1`, `2001:db8:0:0:0:0:0:1`: hex groups of up to four, with `::` or eight groups,
    /// and a digit somewhere, so neither a time nor `abc::def` is one.
    fn ipv6(&self, spans: &mut Vec<Span>, p: usize) -> Option<usize> {
        // groups and colons, read no further than a valid address could reach (7 colons and 8
        // groups of 4: 39 bytes), so a long run of them isn't read again from each word (O-50)
        let n = self.b[p..]
            .iter()
            .take(40)
            .take_while(|&&c| c.is_ascii_hexdigit() || c == b':')
            .count();
        if !(1..40).contains(&n) {
            return None;
        }
        let a = &self.line[p..p + n];
        let colons = a.bytes().filter(|&c| c == b':').count();
        let valid = (a.contains("::") || colons == 7)
            && (2..=7).contains(&colons)
            && a.split(':').all(|g| g.len() <= 4)
            && a.bytes().any(|c| c.is_ascii_digit())
            && !a.contains(":::");
        if !valid || word_at(self.b, p + n) {
            return None;
        }
        put(spans, p, p + n, Kind::Ip);
        Some(p + n)
    }

    /// `https://example.com/x?y`, short of trailing punctuation and of a `)` it doesn't open.
    fn url(&mut self, spans: &mut Vec<Span>, p: usize) -> Option<usize> {
        let b = self.b;
        if p < self.dead.url {
            return None;
        }
        if self.sep <= p {
            // a `://` can't be at `p`, a word's start: the one before it is behind
            self.sep = scheme_sep(b, p);
        }
        if self.sep == b.len() {
            // no `://` on the rest of the line
            self.dead.url = b.len();
            return None;
        }
        let scheme = run_end(b, p, SCHEME);
        if scheme == p || scheme != self.sep {
            self.dead.url = scheme;
            return None;
        }
        let mut i = scheme + 3;
        while let Some(&c) = b.get(i) {
            if c < 0x80 {
                if URL_END[usize::from(c)] {
                    break;
                }
                i += 1;
            } else {
                if wide_space(b, i).is_some() {
                    break;
                }
                i += char_len(c);
            }
        }
        let mut url = &self.line[scheme + 3..i];
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
            self.dead.url = scheme;
            return None;
        }
        let e = scheme + 3 + url.len();
        put(spans, p, e, Kind::Url);
        Some(e)
    }

    /// `key=value`'s key and `=`; a level's value (`level=warn`) as its level.
    fn pair(&mut self, spans: &mut Vec<Span>, p: usize) -> Option<usize> {
        if p < self.dead.pair {
            return None;
        }
        let b = self.b;
        let k = key_end(b, p);
        // a `=` that isn't `==`
        if b.get(k) != Some(&b'=') || b.get(k + 1) == Some(&b'=') {
            self.dead.pair = k;
            return None;
        }
        put(spans, p, k, Kind::Key);
        put(spans, k, k + 1, Kind::Assign);
        let v = k + 1;
        const LEVELED: [u64; 4] = [
            fold(b"level"),
            fold(b"lvl"),
            fold(b"severity"),
            fold(b"loglevel"),
        ];
        let leveled = k - p <= 8 && LEVELED.contains(&fold_at(b, p, k - p));
        if leveled {
            let e = run_end(b, v, UPPER | LOWER);
            if e - v <= 8
                && let Some(kind) = level_of(fold_at(b, v, e - v)).filter(|_| e > v)
                && !word_at(b, e)
            {
                put(spans, v, e, kind);
                return Some(e);
            }
        }
        Some(v)
    }
}

/// Where the first `://` after `p` is, or `b`'s length.
fn scheme_sep(b: &[u8], p: usize) -> usize {
    let mut i = p + 1;
    loop {
        let j = find_in(b, i, b.len(), *b":");
        if j == b.len() || b.get(j + 1..j + 3) == Some(b"//") {
            return j;
        }
        i = j + 1;
    }
}

/// The ASCII bytes a URL ends before: spaces (`char::is_whitespace`'s), quotes, `<` and `>`.
static URL_END: [bool; 256] = stops(b" \t\n\x0b\x0c\r\"'`<>");

/// Whether the character before byte `p` of `b` is a word character.
fn word_before(b: &[u8], p: usize) -> bool {
    match b[p - 1] {
        c if c < 0x80 => CLASS[c as usize] & WORD != 0,
        _ => wide_char_at(b, p - 1).is_some_and(char::is_alphanumeric),
    }
}

/// The string the quote `q` at `p` opens, its spans pushed: bodies sixteen bytes a step, `\`
/// and the character after it an escape, and in `'…'` an apostrophe before a word character
/// part of it (`'it's'`). Its end, or `None` when it doesn't close on the line.
fn string(b: &[u8], spans: &mut Vec<Span>, p: usize, q: u8) -> Option<usize> {
    put(spans, p, p + 1, Kind::Quote);
    let mut i = p + 1;
    loop {
        let j = find_in(b, i, b.len(), [q, b'\\']);
        if j > i {
            put(spans, i, j, Kind::Str);
        }
        if *b.get(j)? == b'\\' {
            let e = j + 1 + char_len(*b.get(j + 1)?);
            put(spans, j, e, Kind::Escape);
            i = e;
        } else if q == b'\'' && word_at(b, j + 1) {
            put(spans, j, j + 1, Kind::Str);
            i = j + 1;
        } else {
            put(spans, j, j + 1, Kind::Quote);
            return (q == b'"' || !word_at(b, j + 1)).then_some(j + 1);
        }
    }
}

/// Where the run of alphabetic characters from `i` ends.
fn alphabetic_end(b: &[u8], mut i: usize) -> usize {
    while let Some(&c) = b.get(i) {
        if c < 0x80 {
            if CLASS[c as usize] & (UPPER | LOWER) == 0 {
                break;
            }
            i += 1;
        } else {
            match wide_char(b, i) {
                Some(c) if c.is_alphabetic() => i += c.len_utf8(),
                _ => break,
            }
        }
    }
    i
}

/// A number's unit.
fn is_unit(u: &[u8]) -> bool {
    matches!(
        u,
        [b's' | b'm' | b'h' | b'd' | b'b' | b'B' | b'k' | b'K' | b'M' | b'G']
            | [b'n' | b'u' | b'm', b's']
            | [0xc2, 0xb5, b's']
            | [b'K' | b'k' | b'M' | b'G' | b'T', b'B']
            | [b'K' | b'M' | b'G', b'i', b'B']
    )
}

/// Where the key characters from `p` end: word characters, `.` and `-`.
fn key_end(b: &[u8], mut i: usize) -> usize {
    loop {
        i = run_end(b, i, KEY);
        match b.get(i) {
            Some(&c) if c >= 0x80 && wide_word(b, i) => i += char_len(c),
            _ => return i,
        }
    }
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
            .map(|s| {
                (
                    &line[s.start as usize..s.end as usize],
                    Kind::ALL[usize::from(s.kind)],
                )
            })
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

    /// The blocks `wide_word` answers without Unicode's tables hold no letter or digit, and its
    /// decoding is `char`'s.
    #[test]
    fn blocks_without_words() {
        for r in NOT_WORD {
            for c in r.filter_map(char::from_u32) {
                assert!(!c.is_alphanumeric(), "U+{:04X}", u32::from(c));
            }
        }
        for c in [
            'é', 'ü', '²', '½', 'µ', '─', '▶', '日', '🦀', '\u{2071}', '\u{24b6}',
        ] {
            let s = format!("a{c}");
            assert_eq!(decode(s.as_bytes(), 1), u32::from(c));
            assert_eq!(wide_word(s.as_bytes(), 1), c.is_alphanumeric(), "{c}");
        }
    }

    /// A window's masks are the class table's, from every start, lines short and long ending
    /// anywhere on the grid.
    #[test]
    fn windows_are_the_class_table() {
        let piece = "ab_9F \"x'`.é─日-Z0\t";
        for len in (0..20).chain(60..70).chain(124..132) {
            let line: String = piece.chars().cycle().take(len).collect();
            let b = line.as_bytes();
            for base in 0..=b.len() {
                let w = Window::at(b, base);
                for i in 0..64 {
                    let (word, hex, digit, stop) = match b.get(base + i) {
                        Some(&c) => {
                            let k = CLASS[c as usize];
                            let word = k & WORD != 0;
                            (
                                word,
                                k & HEX != 0,
                                k & DIGIT != 0,
                                word || k & QUOTE != 0 || c >= 0x80,
                            )
                        }
                        None => (false, false, false, true),
                    };
                    let bit = |m: u64| m >> i & 1 == 1;
                    let at = format!("{line:?} from {base}, byte {i}");
                    assert_eq!(bit(w.word), word, "word: {at}");
                    assert_eq!(bit(w.hex), hex, "hex: {at}");
                    assert_eq!(bit(w.digit), digit, "digit: {at}");
                    assert_eq!(bit(w.stop), stop, "stop: {at}");
                }
            }
            marked(&line);
            marked(&line.replace(' ', "a"));
        }
        // words that end past the next window: a long one, one with a character past ASCII
        for n in [60, 64, 127, 128, 129, 200] {
            let digits = "9".repeat(n);
            let line = format!("x {digits} 1");
            assert_eq!(marked(&line), [(&digits[..], Number), ("1", Number)], "{n}");
            let line = format!("é{} 1", "a".repeat(n));
            assert_eq!(marked(&line), [("1", Number)], "{n}");
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
