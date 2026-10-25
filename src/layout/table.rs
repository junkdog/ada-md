//! Tables (ADR-11, architecture §4). Columns are measured once per Doc ([`measure`]), then
//! distributed per width the way CSS automatic table layout does it ([`distribute`]). A grid that
//! can't fit without breaking words becomes one card per row in `auto` mode; `t` forces either.
//! `s` sorts the body rows by a column ([`order`]); the layout lays them out in that order.

use std::borrow::Cow;
use std::cmp::Ordering;

use unicode_width::UnicodeWidthStr;

use super::wrap::{self, Wrapped};
use super::{Builder, GridRows, RowKind, TableMode, TableRows};
use crate::doc::{Align, Doc, LineRange, Table, TableRow};
use crate::geom;
use crate::theme::{Glyphs, StyleKey, TableBorder};

struct Border {
    top: [&'static str; 3],
    sep: [&'static str; 3],
    bottom: [&'static str; 3],
    horiz: &'static str,
    vert: &'static str,
    card: &'static str,
}

const ROUNDED: Border = Border {
    top: ["╭", "┬", "╮"],
    sep: ["├", "┼", "┤"],
    bottom: ["╰", "┴", "╯"],
    horiz: "─",
    vert: "│",
    card: "╭─",
};

const ASCII: Border = Border {
    top: ["+", "+", "+"],
    sep: ["+", "+", "+"],
    bottom: ["+", "+", "+"],
    horiz: "-",
    vert: "|",
    card: "+-",
};

/// Words wider than this may be hard-broken rather than turn a whole table into records, so one
/// long URL or hash can't hold a table hostage (ADR-11).
pub const MIN_CAP: u16 = 24;

/// A column's width-independent measurements, in cells, without padding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ColMeasure {
    /// The widest unbreakable word.
    pub min: u16,
    /// The widest cell laid out on one line.
    pub max: u16,
    /// The header cell laid out on one line: the key column of the record cards.
    pub header: u16,
}

/// Measure every column of `t`. Inline styles don't change widths; inline code counts its padding.
pub fn measure(t: &Table, doc: &Doc, glyphs: &Glyphs) -> Vec<ColMeasure> {
    let mut cols = vec![ColMeasure::default(); t.column_count()];
    for (r, row) in std::iter::once(&t.header).chain(&t.rows).enumerate() {
        for (col, cell) in cols.iter_mut().zip(&row.cells) {
            let (min, max) = wrap::measure(&cell.runs, doc, glyphs);
            col.min = col.min.max(min);
            col.max = col.max.max(max);
            if r == 0 {
                col.header = max;
            }
        }
    }
    for col in &mut cols {
        col.max = col.max.max(1);
        col.min = col.min.clamp(1, col.max);
    }
    cols
}

/// Content widths for `cols` sharing `avail` cells, or `None` for records.
///
/// 1. Everything fits on one line: each column gets `max`.
/// 2. Every word fits: each column starts at `min`, and the rest is shared in proportion to
///    `max − min`, as in CSS automatic table layout.
/// 3. Every word up to [`MIN_CAP`] fits: the same from `min` capped, hard-breaking only the words
///    that are longer than what their column ends up with.
/// 4. Otherwise `auto` gives up on the grid, while a forced grid shrinks in proportion to the
///    capped `min`, down to two cells a column (one would split wide characters).
pub fn distribute(cols: &[ColMeasure], avail: u16, mode: TableMode) -> Option<Vec<u16>> {
    let avail = u32::from(avail);
    let total = |f: &dyn Fn(&ColMeasure) -> u16| cols.iter().map(|c| u32::from(f(c))).sum::<u32>();
    let max = |c: &ColMeasure| c.max;
    let min = |c: &ColMeasure| c.min;
    let capped = |c: &ColMeasure| c.min.min(MIN_CAP);
    let forced = |c: &ColMeasure| c.max.min(2);
    if total(&max) <= avail {
        return Some(cols.iter().map(max).collect());
    }
    if total(&min) <= avail {
        return Some(grow(cols, avail, min, max));
    }
    if total(&capped) <= avail {
        return Some(grow(cols, avail, capped, max));
    }
    (mode == TableMode::Grid && total(&forced) <= avail).then(|| grow(cols, avail, forced, capped))
}

/// Start every column at `from` and share what is left of `avail` in proportion to `to − from`.
fn grow(
    cols: &[ColMeasure],
    avail: u32,
    from: impl Fn(&ColMeasure) -> u16,
    to: impl Fn(&ColMeasure) -> u16,
) -> Vec<u16> {
    let base: u32 = cols.iter().map(|c| u32::from(from(c))).sum();
    let weights: Vec<u32> = cols
        .iter()
        .map(|c| u32::from(to(c).saturating_sub(from(c))))
        .collect();
    let extra = share(avail.saturating_sub(base), &weights);
    cols.iter().zip(extra).map(|(c, e)| from(c) + e).collect()
}

/// Split `total` in proportion to `weights` by largest remainder: the parts sum to `total`, and
/// none exceeds its weight while `total` is below the sum of the weights.
fn share(total: u32, weights: &[u32]) -> Vec<u16> {
    let sum: u64 = weights.iter().map(|&w| u64::from(w)).sum();
    if sum == 0 {
        return vec![0; weights.len()];
    }
    let total = u64::from(total);
    let mut parts: Vec<u64> = weights
        .iter()
        .map(|&w| total * u64::from(w) / sum)
        .collect();
    let mut left = total - parts.iter().sum::<u64>();
    let mut order: Vec<usize> = (0..weights.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(total * u64::from(weights[i]) % sum));
    for &i in order.iter().cycle() {
        if left == 0 {
            break;
        }
        parts[i] += 1;
        left -= 1;
    }
    parts
        .into_iter()
        .map(|p| p.min(u64::from(u16::MAX)) as u16)
        .collect()
}

/// How a table's body rows are sorted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    /// The column, from 0.
    pub col: u16,
    pub desc: bool,
}

/// The body rows of `t` sorted by `sort.col`, as indices into `t.rows`: by each cell's [`Key`],
/// with blank cells last either way, and rows that compare equal in source order.
pub fn order(t: &Table, sort: Sort) -> Vec<u32> {
    let cells: Vec<String> = t
        .rows
        .iter()
        .map(|r| {
            r.cells
                .get(usize::from(sort.col))
                .map(|c| c.plain_text().trim().to_owned())
                .unwrap_or_default()
        })
        .collect();
    let keys = keys(&cells);
    let mut order: Vec<u32> = (0..t.rows.len() as u32).collect();
    order.sort_by(|&a, &b| keys[a as usize].compare(&keys[b as usize], sort.desc));
    order
}

/// The [`Key`] of each of a column's cells, read in the column's [`Style`].
fn keys(cells: &[String]) -> Vec<Key<'_>> {
    let style = Style::of(cells);
    cells.iter().map(|c| Key::of(c, style)).collect()
}

/// What a cell sorts by. Cells sort in groups, in the order of the variants, and of [`Dim`]
/// within values; the direction only applies within a group, so text in a column of numbers
/// stays after them either way, as blanks do.
#[derive(Debug, PartialEq)]
enum Key<'a> {
    /// A value, then the text after it naturally (`12 cores`).
    Value(Dim, f64, &'a str),
    /// Anything else, compared naturally: runs of digits by value, the rest ignoring case,
    /// so `v1.9` < `v1.10`.
    Text(&'a str),
    /// Empty, or a placeholder such as `-`, `?` or `n/a`.
    Blank,
}

/// What a [`Key::Value`] measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Dim {
    /// A number, perhaps with a currency or percent sign, or `k`/`M`.
    Plain,
    /// A size, in bytes.
    Size,
    /// A duration, in seconds.
    Time,
    /// A time of day, in seconds from midnight.
    Clock,
    /// A date, perhaps with a time of day, in seconds.
    Date,
}

impl<'a> Key<'a> {
    fn of(s: &'a str, style: Style) -> Self {
        if blank(s) {
            return Key::Blank;
        }
        if let Some(t) = date(s, style) {
            return Key::Value(Dim::Date, t, "");
        }
        if let Some((t, "")) = clock(s).map(|(t, rest)| (t, rest.trim())) {
            return Key::Value(Dim::Clock, t, "");
        }
        match quantity(s, style) {
            Some((dim, n, rest)) => Key::Value(dim, n, rest),
            None => Key::Text(s),
        }
    }

    fn group(&self) -> (u8, Option<Dim>) {
        match self {
            Key::Value(dim, ..) => (0, Some(*dim)),
            Key::Text(_) => (1, None),
            Key::Blank => (2, None),
        }
    }

    fn compare(&self, other: &Self, desc: bool) -> Ordering {
        let within = match (self, other) {
            (Key::Value(da, x, ra), Key::Value(db, y, rb)) if da == db => {
                x.total_cmp(y).then_with(|| natural(ra, rb))
            }
            (Key::Text(a), Key::Text(b)) => natural(a, b),
            _ => return self.group().cmp(&other.group()),
        };
        if desc { within.reverse() } else { within }
    }
}

/// How a column writes its numbers and dates, going by all of its cells.
#[derive(Clone, Copy, Debug, Default)]
struct Style {
    /// `1.234,5` rather than `1,234.5`.
    decimal_comma: bool,
    /// `14/10/2026` rather than `10/14/2026`.
    day_first: bool,
}

impl Style {
    /// A decimal comma when some cell has one (`3,5`, `1.234,5`, `1.234.567`) and none has a
    /// decimal point; day first when some date can't be month first (`14/10/2026`) and none
    /// can't be day first.
    fn of(cells: &[String]) -> Self {
        let (mut comma, mut point, mut day, mut month) = (false, false, false, false);
        for c in cells {
            if let Some(([a, b, _], sep, _)) = triple(c)
                && sep != '.'
                && a.len() <= 2
            {
                let (a, b) = (a.parse().unwrap_or(0u32), b.parse().unwrap_or(0u32));
                day |= a > 12;
                month |= b > 12;
                continue;
            }
            let Some(i) = c.find(|ch: char| ch.is_ascii_digit()) else {
                continue;
            };
            let run = &c[i..];
            let end = run
                .find(|ch: char| !(ch.is_ascii_digit() || ch == '.' || ch == ','))
                .unwrap_or(run.len());
            let run = run[..end].trim_end_matches(['.', ',']);
            let after = |at: usize| run.len() - at - 1;
            match (run.rfind('.'), run.rfind(',')) {
                (Some(p), Some(q)) if q > p => comma = true,
                (Some(_), Some(_)) => point = true,
                (None, Some(q)) if run.matches(',').count() == 1 && after(q) != 3 => comma = true,
                (Some(p), None) if run.matches('.').count() == 1 => point |= after(p) != 3,
                (Some(_), None) => comma |= run.split('.').skip(1).all(|g| g.len() == 3),
                _ => {}
            }
        }
        Style {
            decimal_comma: comma && !point,
            day_first: day && !month,
        }
    }
}

/// Empty, dashes, or a placeholder such as `?`, `n/a` or `TBD`.
fn blank(s: &str) -> bool {
    s.chars()
        .all(|c| matches!(c, '-' | '–' | '—' | '?' | '…' | ' '))
        || ["n/a", "na", "tbd", "tba"]
            .iter()
            .any(|p| s.eq_ignore_ascii_case(p))
}

/// The value of a cell that is a number: perhaps signed, approximate (`~5`, `<1`), in money
/// (`$3`, `US$3`, `(1,234)` as a negative, `85 €`), a percentage, or `1.2k`; or a size (`1.5 GiB`);
/// or a duration, perhaps of several parts (`1m 30s`). A number followed by any other text is a
/// plain number, with that text after it.
fn quantity(s: &str, style: Style) -> Option<(Dim, f64, &str)> {
    if let Some(inner) = s.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        return match signed(inner, style)? {
            (Dim::Plain, n, "") => Some((Dim::Plain, -n, "")),
            _ => None,
        };
    }
    signed(s, style)
}

fn signed(s: &str, style: Style) -> Option<(Dim, f64, &str)> {
    let s = s
        .trim_start_matches(['~', '≈', '<', '>', '≤', '≥'])
        .trim_start();
    let (neg, s) = sign(s);
    let s = currency(s);
    let (neg, s) = match neg {
        false => sign(s),
        true => (true, s),
    };
    let (n, rest) = leading_number(s, style)?;
    // more of a number than it reads as: a version, an address
    if rest.starts_with(|c: char| c.is_ascii_digit() || matches!(c, '.' | ',' | ':')) {
        return None;
    }
    let (dim, n, rest) = match rest.trim_start() {
        "" | "%" => (Dim::Plain, n, ""),
        "‰" => (Dim::Plain, n / 10.0, ""),
        "k" | "K" => (Dim::Plain, n * 1e3, ""),
        "M" => (Dim::Plain, n * 1e6, ""),
        "bn" => (Dim::Plain, n * 1e9, ""),
        unit => match (bytes(unit), duration(n, unit, style)) {
            (Some(b), _) => (Dim::Size, n * b, ""),
            (None, Some(t)) => (Dim::Time, t, ""),
            (None, None) => (Dim::Plain, n, unit),
        },
    };
    Some((dim, if neg { -n } else { n }, rest))
}

fn sign(s: &str) -> (bool, &str) {
    match s.strip_prefix(['-', '−']) {
        Some(rest) => (true, rest.trim_start()),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    }
}

/// `s` past a currency sign it starts with, perhaps after up to three capitals (`US$`, `R$`).
fn currency(s: &str) -> &str {
    let caps = s.find(|c: char| !c.is_ascii_uppercase()).unwrap_or(s.len());
    let rest = &s[caps..];
    match rest.chars().next() {
        Some(c) if caps <= 3 && is_currency(c) => rest[c.len_utf8()..].trim_start(),
        _ => s,
    }
}

fn is_currency(c: char) -> bool {
    matches!(c, '$' | '¢' | '£' | '¤' | '¥' | '฿') || ('\u{20a0}'..='\u{20cf}').contains(&c)
}

/// `n` of the unit `rest` starts with, plus any further parts, in seconds.
fn duration(mut n: f64, mut rest: &str, style: Style) -> Option<f64> {
    let mut total = 0.0;
    loop {
        let end = rest
            .find(|c: char| !c.is_alphabetic())
            .unwrap_or(rest.len());
        let (unit, tail) = rest.split_at(end);
        total += n * seconds(unit)?;
        rest = tail.trim_start();
        if rest.is_empty() {
            return Some(total);
        }
        (n, rest) = leading_number(rest, style)?;
        rest = rest.trim_start();
    }
}

/// Seconds in a time unit.
fn seconds(unit: &str) -> Option<f64> {
    Some(match unit {
        "ns" => 1e-9,
        "us" | "µs" | "μs" => 1e-6,
        "ms" => 1e-3,
        "s" | "sec" | "secs" | "second" | "seconds" => 1.0,
        "m" | "min" | "mins" | "minute" | "minutes" => 60.0,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3600.0,
        "d" | "day" | "days" => 86400.0,
        "w" | "wk" | "wks" | "week" | "weeks" => 7.0 * 86400.0,
        "mo" | "month" | "months" => 30.436875 * 86400.0,
        "y" | "yr" | "yrs" | "year" | "years" => 365.2425 * 86400.0,
        _ => return None,
    })
}

/// Bytes in a size unit: `B`, `kB`, `MB` and on in thousands, `KiB`, `MiB` and on in 1024s.
/// Bits are taken for bytes (`Mb`), which only matters when a column mixes the two.
fn bytes(unit: &str) -> Option<f64> {
    if matches!(unit, "B" | "byte" | "bytes") {
        return Some(1.0);
    }
    let prefix = unit.strip_suffix(['B', 'b'])?;
    let (prefix, base) = match prefix.strip_suffix('i') {
        Some(p) => (p, 1024f64),
        None => (prefix, 1000f64),
    };
    let power = match prefix {
        "k" | "K" => 1,
        "m" | "M" => 2,
        "g" | "G" => 3,
        "t" | "T" => 4,
        "p" | "P" => 5,
        "e" | "E" => 6,
        _ => return None,
    };
    Some(base.powi(power))
}

/// The number `s` starts with, and what follows it: digits, perhaps in groups (`1,024`,
/// `1'024`, `1 024`, `1_024`, `1.024` with a decimal comma, `1,00,000`), a fraction after the
/// column's decimal mark, and an exponent.
fn leading_number(s: &str, style: Style) -> Option<(f64, &str)> {
    let (mark, group) = match style.decimal_comma {
        true => (',', '.'),
        false => ('.', ','),
    };
    let (int, mut rest) = split_digits(s);
    let mut digits = int.to_owned();
    while !int.is_empty()
        && let Some(sep) = rest.chars().next()
    {
        let (run, tail) = split_digits(&rest[sep.len_utf8()..]);
        let grouped = match sep {
            '_' => !run.is_empty(),
            ',' if sep == group => matches!(run.len(), 2 | 3),
            '\'' | '’' | ' ' | '\u{a0}' | '\u{2009}' | '\u{202f}' => run.len() == 3,
            _ => sep == group && run.len() == 3,
        };
        if !grouped {
            break;
        }
        digits.push_str(run);
        rest = tail;
    }
    if let Some((frac, tail)) = rest.strip_prefix(mark).map(split_digits)
        && !frac.is_empty()
    {
        digits.push('.');
        digits.push_str(frac);
        rest = tail;
    }
    if digits.is_empty() {
        return None;
    }
    if let Some(e) = rest.strip_prefix(['e', 'E']) {
        let (sign, e) = match e.strip_prefix(['+', '-']) {
            Some(tail) => (&e[..1], tail),
            None => ("", e),
        };
        let (exp, tail) = split_digits(e);
        if !exp.is_empty() {
            digits.push('e');
            digits.push_str(sign);
            digits.push_str(exp);
            rest = tail;
        }
    }
    let n: f64 = digits.parse().ok().filter(|n: &f64| n.is_finite())?;
    Some((n, rest))
}

/// A date, perhaps with a time of day and `Z`/`UTC`, as seconds: `2026-10-14`, `2026/10/14`,
/// `14.10.2026`, `10/14/2026` or `14/10/26` as `style` says, `14 Oct 2026`, `14-Oct-2026`,
/// `Oct 14, 2026`, `October 14th`, `Oct 2026`, each perhaps after a weekday (`Tue, `).
/// Without a year, the date sorts before any that has one.
fn date(s: &str, style: Style) -> Option<f64> {
    let s = weekday(s);
    let (y, m, d, rest) = numeric_date(s, style)
        .or_else(|| day_month(s))
        .or_else(|| month_day(s))?;
    let rest = rest.trim_start_matches([',', ' ']);
    let rest = rest.strip_prefix('T').unwrap_or(rest);
    let (secs, rest) = match rest {
        "" => (0.0, ""),
        rest => clock(rest)?,
    };
    if !matches!(rest.trim(), "" | "Z" | "UTC" | "GMT") {
        return None;
    }
    Some(days(y, m, d) as f64 * 86400.0 + secs)
}

type Ymd<'a> = (i64, u32, u32, &'a str);

fn numeric_date(s: &str, style: Style) -> Option<Ymd<'_>> {
    let ([a, b, c], sep, rest) = triple(s)?;
    let (y, m, d) = match (a.len(), c.len()) {
        (4, _) => (a, b, c),
        (1 | 2, 2 | 4) if c.len() == 4 || sep == '/' => match sep == '.' || style.day_first {
            true => (c, b, a),
            false => (c, a, b),
        },
        _ => return None,
    };
    let (m, d) = (day_or_month(m, 12)?, day_or_month(d, 31)?);
    let y = match (y.len(), y.parse::<i64>().ok()?) {
        (2, y) if y < 70 => 2000 + y,
        (2, y) => 1900 + y,
        (_, y) => y,
    };
    Some((y, m, d, rest))
}

/// `a-b-c`, `a/b/c` or `a.b.c`: three runs of digits between the same separator, and what
/// follows them.
fn triple(s: &str) -> Option<([&str; 3], char, &str)> {
    let (a, rest) = split_digits(s);
    let sep = rest
        .chars()
        .next()
        .filter(|c| matches!(c, '-' | '/' | '.'))?;
    let (b, rest) = split_digits(&rest[1..]);
    let (c, rest) = split_digits(rest.strip_prefix(sep)?);
    [a, b, c]
        .iter()
        .all(|p| !p.is_empty())
        .then_some(([a, b, c], sep, rest))
}

fn day_or_month(s: &str, max: u32) -> Option<u32> {
    (s.len() <= 2)
        .then(|| s.parse().ok())
        .flatten()
        .filter(|n| (1..=max).contains(n))
}

/// `14 Oct 2026`, `14th October`, `14-Oct-2026`, `14. Oct`.
fn day_month(s: &str) -> Option<Ymd<'_>> {
    let (d, rest) = day(s)?;
    let (m, rest) = month(rest.trim_start_matches([' ', '-']))?;
    let rest = rest.trim_start_matches([' ', ',', '-']);
    Some(match year(rest) {
        Some((y, rest)) => (y, m, d, rest),
        None => (0, m, d, rest),
    })
}

/// `Oct 14, 2026`, `October 14th`, `Oct 2026`.
fn month_day(s: &str) -> Option<Ymd<'_>> {
    let (m, rest) = month(s)?;
    let rest = rest.trim_start_matches([' ', ',', '-']);
    if let Some((y, rest)) = year(rest) {
        return Some((y, m, 1, rest));
    }
    let (d, rest) = day(rest)?;
    let rest = rest.trim_start_matches([' ', ',']);
    Some(match year(rest) {
        Some((y, rest)) => (y, m, d, rest),
        None => (0, m, d, rest),
    })
}

/// A day of the month, perhaps as an ordinal (`14th`) or with a dot (`14.`).
fn day(s: &str) -> Option<(u32, &str)> {
    let (d, rest) = split_digits(s);
    let d = day_or_month(d, 31)?;
    let rest = match rest.get(..2) {
        Some(o)
            if ["st", "nd", "rd", "th"]
                .iter()
                .any(|x| o.eq_ignore_ascii_case(x)) =>
        {
            &rest[2..]
        }
        _ => rest,
    };
    Some((d, rest.strip_prefix('.').unwrap_or(rest)))
}

fn year(s: &str) -> Option<(i64, &str)> {
    let (y, rest) = split_digits(s);
    Some((y.parse().ok().filter(|_| y.len() == 4)?, rest))
}

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

const WEEKDAYS: [&str; 7] = [
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
];

/// A month from 1, by its English name or the first three or more letters of it (`Sept.`).
fn month(s: &str) -> Option<(u32, &str)> {
    let (i, rest) = name(s, &MONTHS)?;
    Some((i as u32 + 1, rest.strip_prefix('.').unwrap_or(rest)))
}

/// `s` past a weekday it starts with (`Tue, `, `Tuesday `), when something follows.
fn weekday(s: &str) -> &str {
    match name(s, &WEEKDAYS) {
        Some((_, rest)) => match rest.trim_start_matches(['.', ',']).trim_start() {
            "" => s,
            rest => rest,
        },
        None => s,
    }
}

/// Which of `names` the word `s` starts with is, or the first three or more letters of.
fn name<'a>(s: &'a str, names: &[&str]) -> Option<(usize, &'a str)> {
    let end = s
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(s.len());
    let (word, rest) = s.split_at(end);
    let i = names.iter().position(|n| {
        word.len() >= 3 && n.len() >= word.len() && n[..word.len()].eq_ignore_ascii_case(word)
    })?;
    Some((i, rest))
}

/// A time of day, in seconds from midnight, and what follows it: `9:30`, `21:05:10`,
/// `9:30 pm`, `9 a.m.`. Without minutes it needs the `am`/`pm`.
fn clock(s: &str) -> Option<(f64, &str)> {
    let (h, mut rest) = split_digits(s);
    let h: u32 = (1..=2).contains(&h.len()).then(|| h.parse().ok())??;
    let mut secs = 0;
    let mut parts = 0;
    for scale in [60, 1] {
        let Some((n, tail)) = rest.strip_prefix(':').map(split_digits) else {
            break;
        };
        let n: u32 = (n.len() == 2).then(|| n.parse().ok())??;
        if n >= 60 {
            return None;
        }
        secs += n * scale;
        parts += 1;
        rest = tail;
    }
    let h = match meridiem(rest.trim_start()) {
        Some((pm, tail)) if (1..=12).contains(&h) => {
            rest = tail;
            h % 12 + if pm { 12 } else { 0 }
        }
        Some(_) => return None,
        None if parts == 0 || h > 24 => return None,
        None => h,
    };
    Some((f64::from(h * 3600 + secs), rest))
}

/// Whether `s` starts with `pm` rather than `am` (or `p.m.`, any case), and what follows.
fn meridiem(s: &str) -> Option<(bool, &str)> {
    ["am", "a.m.", "pm", "p.m."].iter().find_map(|m| {
        let rest = s.get(m.len()..)?;
        (s[..m.len()].eq_ignore_ascii_case(m) && !rest.starts_with(char::is_alphabetic))
            .then(|| (m.starts_with('p'), rest))
    })
}

/// Days from 1970-01-01 to a date in the proleptic Gregorian calendar.
fn days(y: i64, m: u32, d: u32) -> i64 {
    let (m, d) = (i64::from(m), i64::from(d));
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn natural(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a, b);
    loop {
        match (a.chars().next(), b.chars().next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (da, ra) = split_digits(a);
                let (db, rb) = split_digits(b);
                let (za, zb) = (da.trim_start_matches('0'), db.trim_start_matches('0'));
                let by_value = za.len().cmp(&zb.len()).then_with(|| za.cmp(zb));
                if by_value != Ordering::Equal {
                    return by_value;
                }
                (a, b) = (ra, rb);
            }
            (Some(x), Some(y)) => {
                let by_char = x.to_lowercase().cmp(y.to_lowercase());
                if by_char != Ordering::Equal {
                    return by_char;
                }
                (a, b) = (&a[x.len_utf8()..], &b[y.len_utf8()..]);
            }
        }
    }
}

fn split_digits(s: &str) -> (&str, &str) {
    s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()))
}

pub(super) fn layout(b: &mut Builder<'_>, t: &Table) {
    let mut cols: Cow<'_, [ColMeasure]> = match b.measure.tables.get(t.id as usize) {
        Some(m) if m.len() == t.column_count() => Cow::Borrowed(m),
        _ => Cow::Owned(measure(t, b.doc, b.g)),
    };
    if cols.is_empty() {
        return;
    }
    let sorted = b.measure.sorted(t.id);
    let glyph = sorted
        .map(|s| b.g.sort_mark[usize::from(s.sort.desc)].clone())
        .filter(|m| !m.is_empty());
    let mark = sorted.zip(glyph.as_deref()).map(|(s, glyph)| Mark {
        col: usize::from(s.sort.col),
        glyph,
        extra: (glyph.width() as u16).saturating_sub(1),
    });
    let border = match b.g.table_border {
        TableBorder::Rounded => Some(&ROUNDED),
        TableBorder::Ascii => Some(&ASCII),
        TableBorder::None => None,
    };
    // a mark wider than the padding it sits in widens its column by the rest; a bordered grid
    // draws it on the rule under the header instead, and cards make room as they wrap the key
    if let Some(m) = mark.filter(|m| m.extra > 0)
        && border.is_none()
        && let Some(col) = cols.to_mut().get_mut(m.col)
    {
        col.header = col.header.saturating_add(m.extra);
        col.max = col.max.max(col.header);
        col.min = col.min.saturating_add(m.extra).min(col.max);
    }
    let rows: Vec<&TableRow> = match sorted {
        Some(s) => s
            .order
            .iter()
            .filter_map(|&i| t.rows.get(i as usize))
            .collect(),
        None => t.rows.iter().collect(),
    };
    let header = Header {
        row: &t.header,
        mark,
    };
    // Padding of one cell on each side of a column, plus the vertical borders; saturating, as
    // 32,768 columns pass u16::MAX (F-012).
    let n = geom::cells(cols.len());
    let chrome = n
        .saturating_mul(2)
        .saturating_add(border.map_or(0, |_| n.saturating_add(1)));
    let avail = b.table_avail().saturating_sub(chrome);
    let mode = match b.params.table_mode {
        // There is nothing to put on a card without body rows.
        _ if t.rows.is_empty() => TableMode::Grid,
        mode => mode,
    };
    let widths = match mode {
        TableMode::Records => None,
        mode => distribute(&cols, avail, mode),
    };
    let start = b.out.row_count();
    match widths {
        Some(widths) => grid(b, t, &header, &rows, &widths, border),
        None => records(b, &header, &rows, &cols, border),
    }
    b.out.tables.push(TableRows {
        table: t.id,
        rows: start..b.out.row_count(),
    });
}

/// The header row, and the sort mark of the column the table is sorted by.
struct Header<'a> {
    row: &'a TableRow,
    mark: Option<Mark<'a>>,
}

impl Header<'_> {
    /// The marked cell wraps short of its width by the mark's `extra`.
    fn cells(&self, b: &mut Builder<'_>, widths: &[u16]) -> Vec<Wrapped> {
        let mut widths = widths.to_vec();
        if let Some(m) = self.mark
            && let Some(w) = widths.get_mut(m.col)
        {
            *w = w.saturating_sub(m.extra).max(1);
        }
        cells(b, self.row, &widths, StyleKey::TableHeader)
    }
}

/// The sort mark sits on the rule under the header of a bordered grid, and otherwise in the
/// padding after the last line of its header, so a mark one cell wide leaves the column as wide
/// as it was; what's wider than that is `extra`.
#[derive(Clone, Copy)]
struct Mark<'a> {
    col: usize,
    glyph: &'a str,
    extra: u16,
}

impl Mark<'_> {
    /// `pad` cells of padding after the header's text, the mark in them: a space off the text
    /// when there's room for one, and `keep` cells clear after it.
    fn pad(self, b: &mut Builder<'_>, pad: u16, keep: u16) {
        let width = self.extra + 1;
        let gap = u16::from(pad.saturating_sub(keep) > width);
        b.spaces(gap, StyleKey::Text);
        b.text(self.glyph, StyleKey::TableHeader);
        b.spaces(pad.saturating_sub(gap + width), StyleKey::Text);
    }
}

/// A row's cells wrapped, in buffers from [`Builder::cells`]; [`done`] hands them back.
fn cells(b: &mut Builder<'_>, row: &TableRow, widths: &[u16], key: StyleKey) -> Vec<Wrapped> {
    let mut out = Vec::with_capacity(widths.len());
    for (c, &w) in widths.iter().enumerate() {
        let into = b.cells.pop().unwrap_or_default();
        out.push(match row.cells.get(c) {
            Some(inl) => {
                let into = into.recycle(Some(inl.unit));
                wrap::wrap(into, &inl.runs, key, w, b.doc, b.g, None)
            }
            None => into.recycle(None),
        });
    }
    out
}

/// Keep `cells`' buffers for the next row's (LAY-01: a fresh one per cell was 15% of XL's layout).
fn done(b: &mut Builder<'_>, cells: Vec<Wrapped>) {
    b.cells.extend(cells);
}

fn grid(
    b: &mut Builder<'_>,
    t: &Table,
    header: &Header<'_>,
    rows: &[&TableRow],
    widths: &[u16],
    border: Option<&Border>,
) {
    if let Some(bd) = border {
        border_row(b, widths, bd.top, bd.horiz, None);
    }
    let header_start = b.out.row_count();
    // a border draws the sort mark on the rule under the header, the header's padding otherwise
    let (padded, ruled) = match border {
        Some(_) => (None, header.mark),
        None => (header.mark, None),
    };
    let head = Header {
        row: header.row,
        mark: padded,
    }
    .cells(b, widths);
    grid_row(b, header.row, &head, widths, &t.aligns, border, padded);
    done(b, head);
    if let Some(bd) = border
        && !rows.is_empty()
    {
        border_row(b, widths, bd.sep, bd.horiz, ruled);
    }
    let header = header_start..b.out.row_count();
    for (i, row) in rows.iter().enumerate() {
        let key = b.text_key;
        let cells = cells(b, row, widths, key);
        b.striped = i % 2 == 1;
        grid_row(b, row, &cells, widths, &t.aligns, border, None);
        done(b, cells);
        b.striped = false;
    }
    if !rows.is_empty() {
        b.out.grids.push(GridRows {
            header,
            end: b.out.row_count(),
        });
    }
    if let Some(bd) = border {
        // a header alone has its sort mark on the bottom edge
        let mark = ruled.filter(|_| rows.is_empty());
        border_row(b, widths, bd.bottom, bd.horiz, mark);
    }
}

/// `mark` sits at the right end of its column's stretch of the rule, a `horiz` clear of the
/// junction when there's room for one; the rule goes without it when it's wider than the stretch.
fn border_row(
    b: &mut Builder<'_>,
    widths: &[u16],
    [left, mid, right]: [&str; 3],
    horiz: &str,
    mark: Option<Mark<'_>>,
) {
    b.begin_row();
    b.text(left, StyleKey::TableBorder);
    for (i, &w) in widths.iter().enumerate() {
        if i > 0 {
            b.text(mid, StyleKey::TableBorder);
        }
        let span = w + 2;
        match mark.filter(|m| m.col == i && m.extra < span) {
            Some(m) => {
                let width = m.extra + 1;
                let tail = u16::from(width < span);
                let lead = span - width - tail;
                b.text(&horiz.repeat(lead as usize), StyleKey::TableBorder);
                b.text(m.glyph, StyleKey::TableHeader);
                b.text(&horiz.repeat(tail as usize), StyleKey::TableBorder);
            }
            None => b.text(&horiz.repeat(span as usize), StyleKey::TableBorder),
        }
    }
    b.text(right, StyleKey::TableBorder);
    b.end_row(LineRange::NONE, RowKind::Decor);
}

fn grid_row(
    b: &mut Builder<'_>,
    row: &TableRow,
    cells: &[Wrapped],
    widths: &[u16],
    aligns: &[Align],
    border: Option<&Border>,
    mark: Option<Mark<'_>>,
) {
    let height = cells
        .iter()
        .map(|c| c.lines.len())
        .max()
        .unwrap_or(0)
        .max(1);
    for k in 0..height {
        b.begin_row();
        if let Some(bd) = border {
            b.text(bd.vert, StyleKey::TableBorder);
        }
        let start = b.col;
        for (c, cell) in cells.iter().enumerate() {
            if c > 0
                && let Some(bd) = border
            {
                b.text(bd.vert, StyleKey::TableBorder);
            }
            let line = cell.lines.get(k);
            let mark = mark.filter(|m| m.col == c);
            let extra = mark.map_or(0, |m| m.extra);
            let slack = widths[c]
                .saturating_sub(extra)
                .saturating_sub(line.map_or(0, |l| l.width));
            let (left, right) = match aligns.get(c) {
                Some(Align::Center) => (slack / 2, slack - slack / 2),
                Some(Align::Right) => (slack, 0),
                _ => (0, slack),
            };
            b.spaces(1 + left, StyleKey::Text);
            if let Some(line) = line {
                b.emit_line(cell, line);
            }
            let pad = 1 + right + extra;
            match mark {
                Some(m) if k == cell.lines.len().saturating_sub(1) => m.pad(b, pad, 0),
                _ => b.spaces(pad, StyleKey::Text),
            }
        }
        if b.striped {
            b.stripe = Some(start..b.col);
        }
        if let Some(bd) = border {
            b.text(bd.vert, StyleKey::TableBorder);
        }
        b.end_row(row.src, RowKind::Text);
    }
}

/// One card per body row: `header  value` pairs with a hanging indent. The key column is as wide
/// as the widest header, capped at a third of the width.
fn records(
    b: &mut Builder<'_>,
    header: &Header<'_>,
    rows: &[&TableRow],
    cols: &[ColMeasure],
    border: Option<&Border>,
) {
    let avail = b.avail();
    let bar = border.map_or("", |bd| bd.vert);
    let bar_width = if bar.is_empty() { 0 } else { 2 };
    let key_width = cols
        .iter()
        .map(|c| c.header)
        .max()
        .unwrap_or(0)
        .min(avail / 3)
        .max(1);
    let value_width = avail.saturating_sub(bar_width + key_width + 2).max(1);
    let keys = header.cells(b, &vec![key_width; cols.len()]);
    let value_widths = vec![value_width; cols.len()];
    for (i, row) in rows.iter().enumerate() {
        b.begin_row();
        match border {
            Some(bd) => {
                b.text(&format!("{} {} ", bd.card, i + 1), StyleKey::TableBorder);
                b.fill_with(bd.horiz, avail, StyleKey::TableBorder);
            }
            None => b.text(&format!("{}.", i + 1), StyleKey::TableBorder),
        }
        b.end_row(row.src, RowKind::Decor);
        let key = b.text_key;
        let values = cells(b, row, &value_widths, key);
        for (c, (key, value)) in keys.iter().zip(&values).enumerate() {
            let mark = header.mark.filter(|m| m.col == c);
            let height = key.lines.len().max(value.lines.len()).max(1);
            for k in 0..height {
                b.begin_row();
                if !bar.is_empty() {
                    b.text(bar, StyleKey::TableBorder);
                    b.text(" ", StyleKey::Text);
                }
                let used = match key.lines.get(k) {
                    Some(line) => {
                        b.emit_line(key, line);
                        line.width
                    }
                    None => 0,
                };
                let pad = key_width.saturating_sub(used) + 2;
                match mark {
                    // one space kept before the value
                    Some(m) if k == key.lines.len().saturating_sub(1) => m.pad(b, pad, 1),
                    _ => b.spaces(pad, StyleKey::Text),
                }
                if let Some(line) = value.lines.get(k) {
                    b.emit_line(value, line);
                }
                if i % 2 == 1 {
                    let start = b.prefix_width + bar_width.min(1);
                    b.stripe = Some(start..b.prefix_width.saturating_add(avail));
                }
                b.end_row(row.src, RowKind::Text);
            }
        }
        done(b, values);
    }
    done(b, keys);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{BlockKind, parse};

    fn col(min: u16, max: u16) -> ColMeasure {
        ColMeasure {
            min,
            max,
            header: 0,
        }
    }

    fn measured(md: &str) -> Vec<ColMeasure> {
        let doc = parse(md);
        let BlockKind::Table(t) = &doc.blocks[0].kind else {
            panic!("not a table")
        };
        let glyphs = Glyphs {
            code_pad: 1,
            ..Glyphs::default()
        };
        measure(t, &doc, &glyphs)
    }

    #[test]
    fn measures_words_lines_and_headers() {
        let cols = measured(
            "| Name | Notes |\n|---|---|\n| `x` | see ../a-b/c.md now |\n| y | line<br>longer line |\n",
        );
        let pairs: Vec<_> = cols.iter().map(|c| (c.min, c.max, c.header)).collect();
        // `x` is 3 wide with its padding, and the path can break after every `/` and `-`.
        assert_eq!(pairs, [(4, 4, 4), (6, 19, 5)]);
    }

    #[test]
    fn long_words_break_only_when_they_must() {
        let url = "https://example.com/".to_owned() + &"x".repeat(40);
        let cols = measured(&format!(
            "| link | notes |\n|---|---|\n| {url} | a few words of text |\n"
        ));
        assert_eq!((cols[0].min, cols[0].max), (40, 60));
        // Room for the whole URL: it isn't broken.
        assert_eq!(distribute(&cols, 50, TableMode::Auto), Some(vec![43, 7]));
        // No room: the URL breaks rather than turning the table into records.
        assert_eq!(distribute(&cols, 30, TableMode::Auto), Some(vec![25, 5]));
        assert_eq!(distribute(&cols, 28, TableMode::Auto), None);
    }

    #[test]
    fn natural_widths_when_everything_fits() {
        let cols = [col(3, 10), col(5, 20)];
        assert_eq!(distribute(&cols, 30, TableMode::Auto), Some(vec![10, 20]));
        assert_eq!(distribute(&cols, 99, TableMode::Auto), Some(vec![10, 20]));
    }

    #[test]
    fn remainder_shared_by_max_minus_min() {
        let cols = [col(4, 4), col(2, 12), col(2, 32)];
        // Σmin = 8, 12 left over, shared 10 : 30 → 3 and 9.
        assert_eq!(distribute(&cols, 20, TableMode::Auto), Some(vec![4, 5, 11]));
    }

    #[test]
    fn auto_gives_up_and_grid_shrinks() {
        let cols = [col(10, 20), col(20, 40)];
        assert_eq!(distribute(&cols, 24, TableMode::Auto), None);
        assert_eq!(distribute(&cols, 24, TableMode::Grid), Some(vec![8, 16]));
        assert_eq!(distribute(&cols, 3, TableMode::Grid), None);
    }

    /// Architecture §13: widths sum to at most `avail`, and no column falls below `min` (capped,
    /// when the words don't all fit) unless the grid is forced.
    #[test]
    fn distribution_properties() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        for _ in 0..5000 {
            let cols: Vec<_> = (0..1 + next(8))
                .map(|_| {
                    let min = 1 + next(40) as u16;
                    col(min, min + next(60) as u16)
                })
                .collect();
            let avail = next(200) as u16;
            for mode in [TableMode::Auto, TableMode::Grid] {
                let sum = |f: fn(&ColMeasure) -> u16| -> u32 {
                    cols.iter().map(|c| u32::from(f(c))).sum()
                };
                let fits = sum(|c| c.min) <= u32::from(avail);
                let capped_fits = sum(|c| c.min.min(MIN_CAP)) <= u32::from(avail);
                let Some(widths) = distribute(&cols, avail, mode) else {
                    assert!(!capped_fits, "{cols:?} gave up at {avail}");
                    continue;
                };
                let total: u32 = widths.iter().map(|&w| u32::from(w)).sum();
                assert!(
                    total <= u32::from(avail),
                    "{cols:?} → {widths:?} at {avail}"
                );
                for (&w, c) in widths.iter().zip(&cols) {
                    let what = format!("{cols:?} → {widths:?} at {avail}");
                    assert!(w <= c.max, "{what}");
                    assert!(!fits || w >= c.min, "{what}");
                    assert!(!capped_fits || w >= c.min.min(MIN_CAP), "{what}");
                }
            }
        }
    }

    /// `a` against `b`, ascending, in a column of the two.
    fn compare_cells(a: &str, b: &str) -> Ordering {
        let cells = [a.to_owned(), b.to_owned()];
        let keys = keys(&cells);
        keys[0].compare(&keys[1], false)
    }

    /// A column's cells, sorted ascending.
    fn sorted(cells: &[&str]) -> Vec<String> {
        let cells: Vec<String> = cells.iter().map(|&c| c.to_owned()).collect();
        let keys = keys(&cells);
        let mut order: Vec<usize> = (0..cells.len()).collect();
        order.sort_by(|&a, &b| keys[a].compare(&keys[b], false));
        order.into_iter().map(|i| cells[i].clone()).collect()
    }

    #[test]
    fn cells_compare_as_numbers_or_naturally() {
        use Ordering::{Equal, Greater, Less};
        let cases = [
            ("2", "10", Less),
            ("1,024", "999", Greater),
            ("-1.5", "0.25", Less),
            ("−3", "-2", Less),
            ("$30", "$4", Greater),
            ("12 %", "9%", Greater),
            ("v1.9", "v1.10", Less),
            ("item 007", "item 7", Equal),
            ("apple", "Banana", Less),
            ("Äpfel", "äpfel", Equal),
            ("abc", "ab", Greater),
            ("10 MB", "9 MB", Greater),
            ("1.2.3", "1.10.0", Less),
            ("12345678901234567890123", "99", Greater),
            ("1e3", "999", Greater),
            ("2024-01-15", "2024-01-09", Greater),
            // durations, in any unit, with or without a space
            ("12ms", "12 ms", Equal),
            ("900 ms", "1.2s", Less),
            ("1.5 s", "1500ms", Equal),
            ("250us", "1 ms", Less),
            ("800 µs", "900μs", Less),
            ("999 ns", "1us", Less),
            ("1m 30s", "90 s", Equal),
            ("1h2m", "61 min", Greater),
            ("2 days", "47 h", Greater),
            ("-5ms", "1ns", Less),
            // a plain number before a duration; an unknown unit by its number, then as text
            ("12ms", "12", Greater),
            ("5 parsecs", "4 parsecs", Greater),
            ("3 ms ago", "20 ms ago", Less),
            ("5 parsecs", "5 lightyears", Greater),
            // thousands in groups, money, approximations, accounting negatives
            ("1'024", "999", Greater),
            ("1’000’000", "999 999", Greater),
            ("1 000 000", "2 000", Greater),
            ("1\u{202f}000", "999", Greater),
            ("(1,234)", "-5", Less),
            ("~5", "<4", Greater),
            ("US$ 30", "€4", Greater),
            ("85 €", "310 €", Less),
            ("1.5k", "999", Greater),
            ("2M", "1500k", Greater),
            ("3‰", "1%", Less),
            // sizes
            ("1.5 GB", "300 MB", Greater),
            ("1 KiB", "1000 B", Greater),
            ("1 MiB", "1.05 MB", Less),
            ("1 GiB", "1000 MiB", Greater),
            ("2 TiB", "2048 GiB", Equal),
            ("512 MiB", "1.5 GiB", Less),
            ("2 bytes", "1kB", Less),
            // durations in weeks, months, years
            ("2 weeks", "13 days", Greater),
            ("1 mo", "4 w", Greater),
            ("1y", "11 months", Greater),
            // dates, in any of their formats
            ("14 Oct 2026", "3 Nov 2025", Greater),
            ("Oct 14, 2026", "Nov 3, 2025", Greater),
            ("October 14th 2026", "2026-10-14", Equal),
            ("14-Oct-2026", "Tue, 14 Oct 2026", Equal),
            ("Sept 2026", "2026-09-01", Equal),
            ("14.10.2026", "2026/10/13", Greater),
            ("10/14/2026", "2026-10-14", Equal),
            ("2026-10-14T09:30Z", "2026-10-14 13:05", Less),
            ("Oct 14, 2026 9:30 PM", "2026-10-14 21:30", Equal),
            ("Oct 14", "Jan 2, 1999", Less),
            // times of day
            ("9:30 PM", "10:15 AM", Greater),
            ("12am", "1 a.m.", Less),
            ("21:05:10", "9:30 pm", Less),
            // text that only looks like a date or a number
            ("May 5 tasks", "April showers", Greater),
            ("10.0.0.2", "10.0.0.10", Less),
            ("3.12.10", "3.9.18", Greater),
        ];
        for (a, b, expected) in cases {
            assert_eq!(compare_cells(a, b), expected, "{a} vs {b}");
            assert_eq!(compare_cells(b, a), expected.reverse(), "{b} vs {a}");
        }
    }

    #[test]
    fn a_column_says_how_it_writes_numbers_and_dates() {
        // a decimal comma, once a cell can't be read any other way
        assert_eq!(
            sorted(&["1.234,5", "3,5", "1.000"]),
            ["3,5", "1.000", "1.234,5"]
        );
        assert_eq!(sorted(&["0,25", "0,3"]), ["0,25", "0,3"]);
        // or a point, and grouping commas, by default
        assert_eq!(
            sorted(&["1,234", "3.5", "1,000"]),
            ["3.5", "1,000", "1,234"]
        );
        // day first once a date can't be month first, month first by default
        assert_eq!(
            sorted(&["01/02/2026", "13/01/2026"]),
            ["13/01/2026", "01/02/2026"]
        );
        assert_eq!(
            sorted(&["01/13/2026", "01/02/2026"]),
            ["01/02/2026", "01/13/2026"]
        );
    }

    #[test]
    fn mixed_columns_sort_in_groups_with_text_and_blanks_last() {
        let cells = [
            "n/a", "b", "2 s", "", "3", "500 ms", "a", "-", "Oct 2026", "10",
        ];
        assert_eq!(
            sorted(&cells),
            [
                "3", "10", "500 ms", "2 s", "Oct 2026", "a", "b", "n/a", "", "-"
            ]
        );
        let cells: Vec<String> = cells.iter().map(|&c| c.to_owned()).collect();
        let keys = keys(&cells);
        let mut order: Vec<usize> = (0..cells.len()).collect();
        order.sort_by(|&a, &b| keys[a].compare(&keys[b], true));
        let desc: Vec<&str> = order.iter().map(|&i| cells[i].as_str()).collect();
        assert_eq!(
            desc,
            [
                "10", "3", "2 s", "500 ms", "Oct 2026", "b", "a", "n/a", "", "-"
            ]
        );
    }

    #[test]
    fn keys_compare_in_a_total_order() {
        let cells: Vec<String> = [
            "2 s",
            "3",
            "500 ms",
            "10",
            "1 min",
            "n/a",
            "45",
            "7 ms",
            "1 MB",
            "x",
            "9:30",
            "2026-10-14",
            "1.2.3",
            "3 cores",
            "-",
            "1e3",
            "Oct 14",
        ]
        .iter()
        .map(|&c| c.to_owned())
        .collect();
        let keys = keys(&cells);
        for desc in [false, true] {
            let cmp = |a: &Key<'_>, b: &Key<'_>| a.compare(b, desc);
            for a in &keys {
                for b in &keys {
                    assert_eq!(cmp(a, b), cmp(b, a).reverse(), "{a:?} vs {b:?}");
                    for c in &keys {
                        if cmp(a, b).is_le() && cmp(b, c).is_le() {
                            assert!(cmp(a, c).is_le(), "{a:?} <= {b:?} <= {c:?}");
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn rows_sort_both_ways_with_blanks_last() {
        let doc = parse(
            "| k | n |\n|---|---|\n| a | 10 |\n| b |  |\n| c | 2 |\n| d | 2 |\n| e |\n| f | 33 |\n",
        );
        let BlockKind::Table(t) = &doc.blocks[0].kind else {
            panic!("not a table")
        };
        let asc = order(
            t,
            Sort {
                col: 1,
                desc: false,
            },
        );
        let desc = order(t, Sort { col: 1, desc: true });
        // ties (c, d) and blanks (b, e) keep their source order either way
        assert_eq!(asc, [2, 3, 0, 5, 1, 4]);
        assert_eq!(desc, [5, 0, 2, 3, 1, 4]);
        // a column past the row's cells is blank
        assert_eq!(order(t, Sort { col: 7, desc: true }), [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn shares_sum_exactly() {
        assert_eq!(share(10, &[1, 1, 1]), [4, 3, 3]);
        assert_eq!(share(0, &[5, 5]), [0, 0]);
        assert_eq!(share(7, &[0, 0]), [0, 0]);
        assert_eq!(share(5, &[0, 3, 7]), [0, 2, 3]);
    }
}
