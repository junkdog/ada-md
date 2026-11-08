//! Crontab, in place of bat's `Crontab`, on the line-format engine (X-059). The design is X-059's
//! floor probe grown, not bat's 28 contexts: a line's first byte past its blanks says what it
//! is (a comment, a `@` schedule, a cron expression, a variable, or anything else), a cron
//! expression's five fields are read a token at a time, each token decided by its first byte
//! and its field's patterns in bat's order, and the command is bash's scanner on the rest of the
//! line ([`crate::bash::line_spans`], from a fresh state). Nothing is carried between lines.
//!
//! **bat's corners, followed:** a field's token is the first of its patterns in bat's order to
//! match at its first byte (`\S+` illegal in the minute, hour and day-of-month fields, a byte
//! that isn't a letter illegal in the month and day-of-week fields, a letter left plain there);
//! a number is its whole digit run (`05` is illegal); the commas that start a field are
//! illegal; a variable is `^[A-Z]+=` (`CRON_TZ=` is an illegal word); a line starting with one
//! of bat's completion letters (`c r y a m w d h`) is never an illegal word, any other word
//! first is (with the blanks before it); `TODO` in a comment marks the rest of the line.
//!
//! **Departures:** a command ends with its line. bat's shell runs on into the next line after a
//! `\` that ends a line, and after a schedule whose blanks end its line (`* * * * * `, its `\s+`
//! taking the `\n`) the next line is all a command. A `#` that starts a command is a comment, as
//! sh reads it (bat's bash takes `#cd` for a command's name there); bash's command names are
//! X-093's.
//!
//! **Kinds are leaves**: no meta scope colors anything in the 14 built-in themes (X-059's
//! census), so a token's kind is its leaf alone. The shell's kinds are bash's
//! ([`crate::bash::scopes`]), below [`CRON`]; the format's own are from it.

use crate::emit::{Push, Runs, Sink};
use crate::line_format::{BLANK, Cursor, Format, leads};
use crate::prim::find_any;
use crate::{Out, Span, State};

/// The format's kinds are this plus their index in [`KINDS`]; bash's are below it, and bash's
/// twins ([`crate::bash::TWIN`]) above the format's.
pub const CRON: u16 = 0x100;

macro_rules! kinds {
    ($($k:ident = $s:literal,)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms, dead_code)]
        #[derive(Clone, Copy)]
        #[repr(u16)]
        enum KindId { $($k,)* Count }
        /// The kinds, each a stack of scopes under `source.crontab`.
        mod k {
            $(pub const $k: u16 = super::CRON + super::KindId::$k as u16;)*
        }
        /// Each kind's scopes.
        static KINDS: [&str; KindId::Count as usize] = [$($s,)*];
    };
}

kinds! {
    COMMENT_MARK = "comment.line.number-sign.crontab punctuation.definition.comment.crontab",
    COMMENT = "comment.line.number-sign.crontab",
    TODO = "comment.line.number-sign.crontab invalid.deprecated.todo.crontab",
    VARIABLE = "variable.other.readwrite.crontab",
    ASSIGN = "keyword.operator.assignment.crontab",
    STRING = "string.unquoted.crontab",
    SCHEDULE_AT = "constant.language.schedule.crontab punctuation.definition.variable.crontab",
    SCHEDULE = "constant.language.schedule.crontab",
    AT = "punctuation.definition.variable.crontab",
    INVALID = "invalid.illegal.crontab",
    ANY = "constant.language.any-value.crontab",
    STEP = "punctuation.separator.step.crontab",
    SEQUENCE = "punctuation.separator.sequence.crontab",
    NON_STANDARD = "invalid.deprecated.non-standard.crontab",
    RANGE = "constant.other.range.crontab",
    NUMBER = "constant.numeric.integer.decimal.crontab",
    MONTH = "support.constant.month-name.crontab",
    DAY = "support.constant.day-of-week-name.crontab",
}

/// The scopes under `source.crontab` a kind carries, outermost first: the format's own, or
/// bash's for the shell's.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    match kind.checked_sub(CRON).map(usize::from) {
        Some(i) if i < KINDS.len() => KINDS[i].split(' ').collect(),
        _ => crate::bash::scopes(kind),
    }
}

/// Lex `b`, a line of a crontab without its `\n`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    crate::line_format::line::<Crontab, _>(b, state, out, &mut Push);
}

/// [`line()`]'s lexemes written to `k`, a run sink.
#[inline(never)]
pub(crate) fn line_runs(b: &[u8], state: &mut State, out: &mut Out, k: &mut Runs) {
    crate::line_format::line::<Crontab, _>(b, state, out, k);
}

struct Crontab;

impl Format for Crontab {
    // `\s*(#)`: the blanks before the mark the comment's; its `TODO` read by `entry`
    const LEAD: [u8; 256] = leads(b"", b"");
    const COMMENT_TEXT: u16 = k::COMMENT;
    const COMMENT_MARK: u16 = k::COMMENT_MARK;
    const COMMENT_BLANKS: bool = true;
    const OPENS: bool = false;

    fn resume<K: Sink>(_: &mut Cursor<K>, _: u64) -> u64 {
        0
    }

    #[inline(always)]
    fn entry<K: Sink>(c: &mut Cursor<K>, p: usize) -> u64 {
        entry(c, p);
        0
    }
}

// what a line's first byte past its blanks starts
const L_OTHER: u8 = 0;
const L_COMMENT: u8 = 1;
const L_AT: u8 = 2;
const L_CRON: u8 = 3;
static LEAD: [u8; 256] = {
    let mut t = [L_OTHER; 256];
    t[b'#' as usize] = L_COMMENT;
    t[b'@' as usize] = L_AT;
    let s = b"*/,-0123456789";
    let mut i = 0;
    while i < s.len() {
        t[s[i] as usize] = L_CRON;
        i += 1;
    }
    t
};

/// The first bytes of bat's no-scope pattern in `main` (a schedule's word's prefixes, for
/// completion): a line that starts with one is never an illegal word.
static COMPLETION: [bool; 256] = {
    let mut t = [false; 256];
    let s = b"cryamwdh";
    let mut i = 0;
    while i < s.len() {
        t[s[i] as usize] = true;
        i += 1;
    }
    t
};

/// `\w` as Oniguruma reads it in UTF-8: ASCII's word bytes, and every byte past ASCII (a
/// letter's, nearly always: a CJK note first on a line is bat's illegal word).
static WORD: [bool; 256] = {
    let mut t = [false; 256];
    let mut i = 0;
    while i < 256 {
        let c = i as u8;
        t[i] = c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80;
        i += 1;
    }
    t
};

/// Whether `p` holds a word's byte (`\w`; past ASCII, a letter's).
#[inline(always)]
fn word_at(b: &[u8], p: usize) -> bool {
    p < b.len() && WORD[usize::from(b[p])]
}

/// Where the run of `\w` from `p` ends.
#[inline(always)]
fn word_end(b: &[u8], mut p: usize) -> usize {
    while word_at(b, p) {
        p += 1;
    }
    p
}

/// `\b` before `p`, where `p` holds a word's byte.
#[inline(always)]
fn starts(b: &[u8], p: usize) -> bool {
    p == 0 || !WORD[usize::from(b[p - 1])]
}

/// `[ \t]`.
#[inline(always)]
fn space_tab(x: u8) -> bool {
    x == b' ' || x == b'\t'
}

/// A line whose first byte past its blanks is at `p`.
#[inline(always)]
fn entry<K: Sink>(c: &mut Cursor<K>, p: usize) {
    match LEAD[usize::from(c.b[p])] {
        L_COMMENT => comment(c, 0, p),
        L_AT => at(c, p),
        L_CRON => cron(c, p),
        _ => other(c, p),
    }
}

/// A comment, its blanks from `s`, its mark at `p`: `TODO` to the line's end.
#[inline(never)]
fn comment<K: Sink>(c: &mut Cursor<K>, s: usize, p: usize) {
    let b = c.b;
    c.skip(s);
    c.put(p, k::COMMENT);
    c.put(p + 1, k::COMMENT_MARK);
    let todo = todo_at(b, p + 1);
    c.put(todo, k::COMMENT);
    c.put(b.len(), k::TODO);
}

/// Where the first `TODO` from `p` starts, or the line's end: its `T`s found sixteen bytes a
/// step (a comment's text is 40% of X-059's corpus; a byte loop was ~10% of its instructions).
#[inline(always)]
fn todo_at(b: &[u8], mut p: usize) -> usize {
    let n = b.len();
    while p + 4 <= n {
        p = find_any(b, p, *b"T");
        if p + 4 > n {
            break;
        }
        if &b[p..p + 4] == b"TODO" {
            return p;
        }
        p += 1;
    }
    n
}

/// The rest of a line in bat's `main` from `p`: plain, but a comment at its first `#`, the
/// blanks before the mark the comment's.
#[inline(always)]
fn rest<K: Sink>(c: &mut Cursor<K>, p: usize) {
    let b = c.b;
    if let Some(h) = b[p..].iter().position(|&x| x == b'#') {
        let h = p + h;
        let mut s = h;
        while s > p && BLANK[usize::from(b[s - 1])] {
            s -= 1;
        }
        comment(c, s, h);
    }
}

/// A line no lead takes: a variable (`^([A-Z]+)(=)`, its value a comment or a string), or bat's
/// `main` (an illegal first word with the blanks before it, the rest plain but a comment).
#[inline(never)]
fn other<K: Sink>(c: &mut Cursor<K>, p: usize) {
    let b = c.b;
    let n = b.len();
    if p == 0 && b[0].is_ascii_uppercase() {
        let mut e = 1;
        while e < n && b[e].is_ascii_uppercase() {
            e += 1;
        }
        if e < n && b[e] == b'=' {
            c.put(e, k::VARIABLE);
            c.put(e + 1, k::ASSIGN);
            let v = e + 1;
            let q = crate::line_format::blanks(b, v);
            if q < n && b[q] == b'#' {
                comment(c, v, q);
            } else {
                c.put(n, k::STRING);
            }
            return;
        }
    }
    if COMPLETION[usize::from(b[0])] {
        rest(c, 0);
    } else {
        // `^\s*\w+`
        let e = unicode_word_end(b, p);
        if e > p {
            c.put(e, k::INVALID);
        }
        rest(c, e);
    }
}

/// Where the run of Unicode's `\w` from `p` ends: ASCII's word bytes, and past ASCII a letter
/// or a digit (not punctuation, `：`), as Oniguruma reads it.
#[inline(always)]
fn unicode_word_end(b: &[u8], p: usize) -> usize {
    let e = word_end_ascii(b, p);
    if e == b.len() || b[e] < 0x80 {
        return e;
    }
    let Ok(rest) = std::str::from_utf8(&b[e..]) else {
        return e;
    };
    for (i, x) in rest.char_indices() {
        if !(x == '_' || x.is_alphanumeric()) {
            return e + i;
        }
    }
    b.len()
}

/// Where the run of ASCII's word bytes from `p` ends.
#[inline(always)]
fn word_end_ascii(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && (b[p].is_ascii_alphanumeric() || b[p] == b'_') {
        p += 1;
    }
    p
}

/// `@` at `p`: a schedule's word and its command (bash to the line's end, after `[ \t]+`), a
/// schedule's word alone (the rest `main`'s), or another word for completion (plain).
#[inline(never)]
fn at<K: Sink>(c: &mut Cursor<K>, p: usize) {
    let b = c.b;
    let n = b.len();
    let e = word_end(b, p + 1);
    if matches!(
        &b[p + 1..e],
        b"reboot"
            | b"yearly"
            | b"annually"
            | b"monthly"
            | b"weekly"
            | b"daily"
            | b"midnight"
            | b"hourly"
    ) {
        c.skip(p);
        c.put(p + 1, k::SCHEDULE_AT);
        c.put(e, k::SCHEDULE);
        let mut q = e;
        while q < n && space_tab(b[q]) {
            q += 1;
        }
        if q > e {
            shell(c, q);
        } else {
            rest(c, e);
        }
    } else if e == n || BLANK[usize::from(b[e])] {
        // `(@)(\w*(?:[\n\s]|$))`
        c.skip(p);
        c.put(p + 1, k::AT);
        rest(c, (e + 1).min(n));
    } else {
        rest(c, p);
    }
}

std::thread_local! {
    static SHELL: std::cell::RefCell<Vec<Span>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// bash's scanner on `b[q..]` from a fresh state.
#[inline(never)]
fn shell<K: Sink>(c: &mut Cursor<K>, q: usize) {
    let b = c.b;
    if q >= b.len() {
        return;
    }
    SHELL.with_borrow_mut(|spans| {
        spans.clear();
        let mut state = State::default();
        crate::bash::line_spans(&b[q..], &mut state, &mut Out::new(spans));
        for sp in spans.iter() {
            c.tok(None, q + sp.start as usize, q + sp.end as usize, sp.kind);
        }
    });
}

// the fields
const MINUTE: u8 = 0;
const HOUR: u8 = 1;
const DAY_OF_MONTH: u8 = 2;
const MONTH_F: u8 = 3;
const DAY_OF_WEEK: u8 = 4;

/// Whether `d`, a whole run of digits, is one of field `f`'s numbers.
#[inline(always)]
fn valid(f: u8, d: &[u8]) -> bool {
    matches!(
        (f, d),
        (MINUTE | HOUR, [_])
            | (MINUTE, [b'1'..=b'5', _])
            | (HOUR, [b'1', _] | [b'2', b'0'..=b'3'])
            | (DAY_OF_MONTH | MONTH_F, [b'1'..=b'9'])
            | (DAY_OF_MONTH, [b'1' | b'2', _] | [b'3', b'0' | b'1'])
            | (MONTH_F, [b'1', b'0'..=b'2'])
            | (DAY_OF_WEEK, [b'0'..=b'7'])
    )
}

/// Where the run of ASCII digits from `p` ends.
#[inline(always)]
fn digits(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && b[p].is_ascii_digit() {
        p += 1;
    }
    p
}

/// Whether a three-letter name of `names` (folded) is at `p`, a word on its own.
#[inline(always)]
fn name3(b: &[u8], p: usize, names: &[[u8; 3]]) -> bool {
    if p + 3 > b.len() || word_at(b, p + 3) {
        return false;
    }
    let w = [b[p] | 0x20, b[p + 1] | 0x20, b[p + 2] | 0x20];
    names.contains(&w)
}

static MONTHS: [[u8; 3]; 12] = [
    *b"jan", *b"feb", *b"mar", *b"apr", *b"may", *b"jun", *b"jul", *b"aug", *b"sep", *b"oct",
    *b"nov", *b"dec",
];
static DAYS: [[u8; 3]; 7] = [
    *b"sun", *b"mon", *b"tue", *b"wed", *b"thu", *b"fri", *b"sat",
];

/// `[/,?-]`.
#[inline(always)]
fn punct(x: u8) -> bool {
    matches!(x, b'/' | b',' | b'?' | b'-')
}

/// One token of field `f` at `p` (not a space or a tab), the first of the field's patterns in
/// bat's order to match there: where it ends.
#[inline(always)]
fn field_token<K: Sink>(c: &mut Cursor<K>, p: usize, f: u8) -> usize {
    let b = c.b;
    let n = b.len();
    let x = b[p];
    // `(?:\*{2,}|[/,?-]{2,}|,(?=[ \t]))`
    if x == b'*' && p + 1 < n && b[p + 1] == b'*' {
        let mut e = p + 2;
        while e < n && b[e] == b'*' {
            e += 1;
        }
        c.tok(None, p, e, k::INVALID);
        return e;
    }
    if punct(x) && p + 1 < n && punct(b[p + 1]) {
        let mut e = p + 2;
        while e < n && punct(b[e]) {
            e += 1;
        }
        c.tok(None, p, e, k::INVALID);
        return e;
    }
    let (kind, e) = match x {
        b',' if p + 1 < n && space_tab(b[p + 1]) => (k::INVALID, p + 1),
        b'*' => (k::ANY, p + 1),
        b'/' if p + 1 < n && b[p + 1].is_ascii_digit() => (k::STEP, p + 1),
        b',' => (k::SEQUENCE, p + 1),
        b'H' if starts(b, p) && !word_at(b, p + 1) => (k::NON_STANDARD, p + 1),
        b'0'..=b'9' if starts(b, p) => match number(c, p, f) {
            Some(e) => return e,
            None => return fallback(c, p, f),
        },
        b'?' if f == DAY_OF_MONTH || f == DAY_OF_WEEK => (k::NON_STANDARD, p + 1),
        b'#' if f == DAY_OF_WEEK => (k::NON_STANDARD, p + 1),
        b'A'..=b'Z' | b'a'..=b'z' if f == MONTH_F && starts(b, p) && name3(b, p, &MONTHS) => {
            (k::MONTH, p + 3)
        }
        b'A'..=b'Z' | b'a'..=b'z' if f == DAY_OF_WEEK && starts(b, p) && name3(b, p, &DAYS) => {
            (k::DAY, p + 3)
        }
        _ => return fallback(c, p, f),
    };
    c.tok(None, p, e, kind);
    e
}

/// A range or a number of field `f` at `p`, a digit after no word's byte: where it ends, or
/// `None` where neither matches.
#[inline(always)]
fn number<K: Sink>(c: &mut Cursor<K>, p: usize, f: u8) -> Option<usize> {
    let b = c.b;
    let n = b.len();
    let d = digits(b, p);
    if !valid(f, &b[p..d]) {
        return None;
    }
    if d < n && b[d] == b'-' {
        let d2 = digits(b, d + 1);
        if d2 > d + 1 && valid(f, &b[d + 1..d2]) && !word_at(b, d2) {
            c.tok(None, p, d2, k::RANGE);
            return Some(d2);
        }
    }
    if !word_at(b, d) {
        c.tok(None, p, d, k::NUMBER);
        return Some(d);
    }
    // `([LW])?\b` after a day of the month, `(L)?\b` after a day of the week
    let suffix = match f {
        DAY_OF_MONTH => b[d] == b'L' || b[d] == b'W',
        DAY_OF_WEEK => b[d] == b'L',
        _ => false,
    };
    if suffix && !word_at(b, d + 1) {
        c.tok(None, p, d, k::NUMBER);
        c.tok(None, d, d + 1, k::NON_STANDARD);
        return Some(d + 1);
    }
    None
}

/// Field `f`'s last pattern at `p`: `[^a-zA-Z]` illegal in the month and day-of-week fields (a
/// letter plain), `\S+` in the others (a blank no pattern takes plain).
#[inline(always)]
fn fallback<K: Sink>(c: &mut Cursor<K>, p: usize, f: u8) -> usize {
    let b = c.b;
    let x = b[p];
    if f >= MONTH_F {
        if !x.is_ascii_alphabetic() {
            c.tok(None, p, p + 1, k::INVALID);
        }
        return p + 1;
    }
    if BLANK[usize::from(x)] {
        return p + 1;
    }
    let mut e = p + 1;
    while e < b.len() && !BLANK[usize::from(b[e])] {
        e += 1;
    }
    c.tok(None, p, e, k::INVALID);
    e
}

/// A cron expression from `p`: five fields, each to a space or a tab, the blanks and commas
/// before the next field's first byte (the commas illegal), then the command from the first
/// byte past the blanks.
#[inline(never)]
fn cron<K: Sink>(c: &mut Cursor<K>, mut p: usize) {
    let b = c.b;
    let n = b.len();
    let mut f = MINUTE;
    loop {
        while p < n && !space_tab(b[p]) {
            p = field_token(c, p, f);
        }
        if p == n {
            return;
        }
        if f == DAY_OF_WEEK {
            let q = crate::line_format::blanks(b, p);
            shell(c, q);
            return;
        }
        f += 1;
        while p < n && (BLANK[usize::from(b[p])] || b[p] == b',') {
            if b[p] == b',' {
                c.tok(None, p, p + 1, k::INVALID);
            }
            p += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Lang, State, lex};

    /// Each line's tokens as `text·leaf`, the leaf a kind's last scope without its suffix, plain
    /// text left out.
    fn toks(text: &str) -> Vec<String> {
        let mut state = State::default();
        let mut spans = Vec::new();
        text.lines()
            .map(|l| {
                lex(Lang::Crontab, l, &mut state, &mut spans);
                spans
                    .iter()
                    .map(|s| {
                        let scopes = Lang::Crontab.scopes(s.kind);
                        let leaf = scopes.last().map_or("?", |s| {
                            [".crontab", ".shell"]
                                .iter()
                                .find_map(|x| s.strip_suffix(x))
                                .unwrap_or(s)
                        });
                        format!("{}·{leaf}", &l[s.start as usize..s.end as usize])
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }

    #[test]
    fn fields() {
        let t = toks(concat!(
            "*/5 1-5 1-31/2 Jan,feb mon-fri x\n",
            "60 24 0 13 8 x\n",
            "0 0 15W * 5L x\n",
            "0 0 ? x5 MON#2 x\n",
            "5, ,5 ** */ Sunday x\n",
        ));
        assert_eq!(
            t[0],
            "*·constant.language.any-value /·punctuation.separator.step 5·constant.numeric.integer.decimal \
             1-5·constant.other.range 1-31·constant.other.range /·punctuation.separator.step \
             2·constant.numeric.integer.decimal Jan·support.constant.month-name \
             ,·punctuation.separator.sequence feb·support.constant.month-name \
             mon·support.constant.day-of-week-name -·invalid.illegal fri·support.constant.day-of-week-name \
             x·support.function.command"
        );
        assert_eq!(
            t[1],
            "60·invalid.illegal 24·invalid.illegal 0·invalid.illegal 13·invalid.illegal \
             8·invalid.illegal x·support.function.command"
        );
        assert_eq!(
            t[2],
            "0·constant.numeric.integer.decimal 0·constant.numeric.integer.decimal \
             15·constant.numeric.integer.decimal W·invalid.deprecated.non-standard \
             *·constant.language.any-value 5·constant.numeric.integer.decimal \
             L·invalid.deprecated.non-standard x·support.function.command"
        );
        assert_eq!(
            t[3],
            "0·constant.numeric.integer.decimal 0·constant.numeric.integer.decimal \
             ?·invalid.deprecated.non-standard 5·invalid.illegal MON·support.constant.day-of-week-name \
             #·invalid.deprecated.non-standard 2·constant.numeric.integer.decimal x·support.function.command"
        );
        assert_eq!(
            t[4],
            "5·constant.numeric.integer.decimal ,·invalid.illegal ,·invalid.illegal \
             5·constant.numeric.integer.decimal **·invalid.illegal *·constant.language.any-value \
             /·invalid.illegal x·support.function.command"
        );
    }

    #[test]
    fn lines() {
        let t = toks(concat!(
            "SHELL=/bin/bash\n",
            "MAILTO=  # none\n",
            "CRON_TZ=UTC\n",
            "  abc = 1 # c\n",
            "cron x\n",
            "@reboot  /x.sh\n",
            "@daily#c\n",
            "@foo bar\n",
            "  # TODO: x\n",
        ));
        assert_eq!(
            t[0],
            "SHELL·variable.other.readwrite =·keyword.operator.assignment /bin/bash·string.unquoted"
        );
        assert_eq!(
            t[1],
            "MAILTO·variable.other.readwrite =·keyword.operator.assignment   ·comment.line.number-sign \
             #·punctuation.definition.comment  none·comment.line.number-sign"
        );
        assert_eq!(t[2], "CRON_TZ·invalid.illegal");
        assert_eq!(
            t[3],
            "  abc·invalid.illegal  ·comment.line.number-sign #·punctuation.definition.comment  c·comment.line.number-sign"
        );
        assert_eq!(t[4], "");
        assert_eq!(
            t[5],
            "@·punctuation.definition.variable reboot·constant.language.schedule /x.sh·support.function.command"
        );
        assert_eq!(
            t[6],
            "@·punctuation.definition.variable daily·constant.language.schedule \
             #·punctuation.definition.comment c·comment.line.number-sign"
        );
        assert_eq!(t[7], "@·punctuation.definition.variable");
        assert_eq!(
            t[8],
            "  ·comment.line.number-sign #·punctuation.definition.comment  ·comment.line.number-sign \
             TODO: x·invalid.deprecated.todo"
        );
    }

    #[test]
    fn a_command_ends_with_its_line() {
        let t = toks("* * * * * echo \"open\nnext\n* * * * * x \\\nnext\n");
        assert_eq!(t[1], "next·invalid.illegal");
        assert_eq!(t[3], "next·invalid.illegal");
    }

    #[test]
    fn a_word_past_ascii_and_a_commented_command() {
        let t = toks("可以 x\n* * * * * #cd /x\n可以：x\n");
        assert_eq!(t[0], "可以·invalid.illegal");
        assert_eq!(t[2], "可以·invalid.illegal");
        // sh's reading: bat's bash takes `#cd` for a command's name
        assert!(
            t[1].ends_with("#·punctuation.definition.comment.begin cd /x·comment.line.number-sign"),
            "{}",
            t[1]
        );
    }

    #[test]
    fn long_lines_lex_in_linear_time() {
        for unit in ["1-5,", "*/", "x5", "# TODO", "@", "A", ", "] {
            let line = format!("{}{}", "* * * ", unit.repeat(200_000));
            let t = std::time::Instant::now();
            toks(&line);
            assert!(t.elapsed().as_secs() < 2, "{unit:?}");
        }
    }
}
