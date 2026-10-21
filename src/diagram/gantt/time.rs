//! Gantt time, ada's own (no chrono): instants are milliseconds since 1970-01-01 on a calendar
//! without time zones, as Mermaid's are in the viewer's zone. Dates parse by a dayjs-style
//! `dateFormat` and print by a d3-style `axisFormat`, the two notations Mermaid uses.

pub type Ms = i64;

pub const SECOND: Ms = 1_000;
pub const MINUTE: Ms = 60 * SECOND;
pub const HOUR: Ms = 60 * MINUTE;
pub const DAY: Ms = 24 * HOUR;
pub const WEEK: Ms = 7 * DAY;

/// The first and last instants allowed: years 0 to 9999.
const FIRST: Ms = -719_528 * DAY;
const LAST: Ms = 2_932_897 * DAY - 1;

/// `t` if it is in years 0 to 9999.
pub fn valid(t: Ms) -> Option<Ms> {
    (FIRST..=LAST).contains(&t).then_some(t)
}

/// Days since 1970-01-01 of a civil date (Howard Hinnant's `days_from_civil`).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date of a day count: year, month 1–12, day 1–31.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (
        if m <= 2 {
            yoe + era * 400 + 1
        } else {
            yoe + era * 400
        },
        m,
        d,
    )
}

fn leap(y: i64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

pub fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        2 if leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The day `t` falls on.
pub fn day(t: Ms) -> i64 {
    t.div_euclid(DAY)
}

/// The weekday of a day count, 0 for Sunday.
pub fn weekday(day: i64) -> u32 {
    (day + 4).rem_euclid(7) as u32
}

/// `t` moved by `n` calendar months, the day kept or cut to the month's last, as dayjs does.
pub fn add_months(t: Ms, n: i64) -> Option<Ms> {
    let (y, m, d) = civil_from_days(day(t));
    let months = y
        .checked_mul(12)?
        .checked_add(i64::from(m) - 1)?
        .checked_add(n)?;
    let (y, m) = (months.div_euclid(12), months.rem_euclid(12) as u32 + 1);
    if !(0..=9999).contains(&y) {
        return None;
    }
    let d = d.min(days_in_month(y, m));
    valid(days_from_civil(y, m, d) * DAY + t.rem_euclid(DAY))
}

/// An instant split into its calendar fields.
struct Fields {
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    milli: u32,
    /// 0 for Sunday.
    weekday: u32,
    /// 0 for January 1st.
    yday: u32,
}

fn fields(t: Ms) -> Fields {
    let d = day(t);
    let (year, month, dom) = civil_from_days(d);
    let ms = t.rem_euclid(DAY);
    Fields {
        year,
        month,
        day: dom,
        hour: (ms / HOUR) as u32,
        minute: (ms % HOUR / MINUTE) as u32,
        second: (ms % MINUTE / SECOND) as u32,
        milli: (ms % SECOND) as u32,
        weekday: weekday(d),
        yday: (d - days_from_civil(year, 1, 1)) as u32,
    }
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

pub const WEEKDAYS: [&str; 7] = [
    "sunday",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
];

/// A piece of a `dateFormat`.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Lit(String),
    /// Digits, between `min` and `max` of them, into a field.
    Num(Field, usize, usize),
    /// `MMM`, `MMMM`: a month name, short or long.
    MonthName,
    /// `Do`: a day with its ordinal suffix.
    DayOrdinal,
    /// `A`, `a`: am or pm.
    Meridiem,
    /// `Z`, `ZZ`: an offset, parsed and dropped like the zone it names.
    Offset,
    /// `X`: Unix seconds.
    UnixSeconds,
    /// `x`: Unix milliseconds.
    UnixMillis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Year,
    /// `YY`: 2000–2068 for 00–68, 1969–1999 above, as dayjs reads them.
    ShortYear,
    Month,
    Day,
    Hour,
    Hour12,
    Minute,
    Second,
    /// Fractions of a second, scaled by the token's width.
    Fraction,
}

/// A parsed `dateFormat`, as dayjs's strict `customParseFormat` reads one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DateFormat(Vec<Tok>);

/// The dayjs tokens ada reads, longest first.
const TOKENS: &[(&str, Tok)] = &[
    ("YYYY", Tok::Num(Field::Year, 4, 4)),
    ("YY", Tok::Num(Field::ShortYear, 2, 2)),
    ("MMMM", Tok::MonthName),
    ("MMM", Tok::MonthName),
    ("MM", Tok::Num(Field::Month, 2, 2)),
    ("M", Tok::Num(Field::Month, 1, 2)),
    ("Do", Tok::DayOrdinal),
    ("DD", Tok::Num(Field::Day, 2, 2)),
    ("D", Tok::Num(Field::Day, 1, 2)),
    ("HH", Tok::Num(Field::Hour, 2, 2)),
    ("H", Tok::Num(Field::Hour, 1, 2)),
    ("hh", Tok::Num(Field::Hour12, 2, 2)),
    ("h", Tok::Num(Field::Hour12, 1, 2)),
    ("mm", Tok::Num(Field::Minute, 2, 2)),
    ("m", Tok::Num(Field::Minute, 1, 2)),
    ("ss", Tok::Num(Field::Second, 2, 2)),
    ("s", Tok::Num(Field::Second, 1, 2)),
    ("SSS", Tok::Num(Field::Fraction, 3, 3)),
    ("SS", Tok::Num(Field::Fraction, 2, 2)),
    ("S", Tok::Num(Field::Fraction, 1, 1)),
    ("A", Tok::Meridiem),
    ("a", Tok::Meridiem),
    ("ZZ", Tok::Offset),
    ("Z", Tok::Offset),
    ("X", Tok::UnixSeconds),
    ("x", Tok::UnixMillis),
];

impl Default for DateFormat {
    fn default() -> Self {
        DateFormat::new("YYYY-MM-DD")
    }
}

impl DateFormat {
    pub fn new(fmt: &str) -> Self {
        let mut toks = Vec::new();
        let mut rest = fmt;
        let lit = |toks: &mut Vec<Tok>, s: &str| match toks.last_mut() {
            Some(Tok::Lit(l)) => l.push_str(s),
            _ => toks.push(Tok::Lit(s.to_owned())),
        };
        while !rest.is_empty() {
            if let Some(body) = rest.strip_prefix('[')
                && let Some(end) = body.find(']')
            {
                lit(&mut toks, &body[..end]);
                rest = &body[end + 1..];
                continue;
            }
            if let Some((name, tok)) = TOKENS.iter().find(|(name, _)| rest.starts_with(name)) {
                toks.push(tok.clone());
                rest = &rest[name.len()..];
                continue;
            }
            let c = rest.chars().next().map_or(1, char::len_utf8);
            lit(&mut toks, &rest[..c]);
            rest = &rest[c..];
        }
        DateFormat(toks)
    }

    /// Parse `s`, all of it, by this format; `None` when it doesn't match or isn't a date.
    pub fn parse(&self, s: &str) -> Option<Ms> {
        let mut rest = s;
        let (mut year, mut month, mut dom) = (1970, 1, 1);
        let (mut hour, mut hour12, mut pm) = (0, None, None);
        let (mut minute, mut second, mut milli) = (0, 0, 0);
        let mut unix = None;
        for tok in &self.0 {
            match tok {
                Tok::Lit(l) => rest = rest.strip_prefix(l.as_str())?,
                &Tok::Num(field, min, max) => {
                    let (n, len) = digits(rest, min, max)?;
                    rest = &rest[len..];
                    match field {
                        Field::Year => year = n,
                        Field::ShortYear => year = n + if n <= 68 { 2000 } else { 1900 },
                        Field::Month => month = n,
                        Field::Day => dom = n,
                        Field::Hour => hour = n,
                        Field::Hour12 => hour12 = Some(n),
                        Field::Minute => minute = n,
                        Field::Second => second = n,
                        Field::Fraction => milli = n * 10_i64.pow(3 - len as u32),
                    }
                }
                Tok::MonthName => {
                    let (i, len) = month_name(rest)?;
                    month = i64::from(i);
                    rest = &rest[len..];
                }
                Tok::DayOrdinal => {
                    let (n, len) = digits(rest, 1, 2)?;
                    let after = &rest[len..];
                    let suffix = ["st", "nd", "rd", "th"]
                        .iter()
                        .find(|s| after.starts_with(*s))?;
                    dom = n;
                    rest = &after[suffix.len()..];
                }
                Tok::Meridiem => {
                    let head = rest.get(..2)?;
                    pm = Some(match head.to_ascii_lowercase().as_str() {
                        "am" => false,
                        "pm" => true,
                        _ => return None,
                    });
                    rest = &rest[2..];
                }
                Tok::Offset => rest = offset(rest)?,
                Tok::UnixSeconds | Tok::UnixMillis => {
                    let neg = rest.starts_with('-');
                    let body = if neg { &rest[1..] } else { rest };
                    let (n, len) = digits(body, 1, 15)?;
                    let n = if neg { -n } else { n };
                    let scale = if *tok == Tok::UnixSeconds { SECOND } else { 1 };
                    unix = Some(n.checked_mul(scale)?);
                    rest = &body[len..];
                }
            }
        }
        if !rest.is_empty() {
            return None;
        }
        if let Some(t) = unix {
            return valid(t);
        }
        if let Some(h) = hour12 {
            if !(1..=12).contains(&h) {
                return None;
            }
            hour = h % 12 + if pm == Some(true) { 12 } else { 0 };
        } else if pm == Some(true) && hour < 12 {
            hour += 12;
        }
        let month = u32::try_from(month).ok().filter(|m| (1..=12).contains(m))?;
        if !(0..=9999).contains(&year)
            || !(1..=i64::from(days_in_month(year, month))).contains(&dom)
            || hour > 23
            || minute > 59
            || second > 59
        {
            return None;
        }
        let d = days_from_civil(year, month, dom as u32);
        valid(d * DAY + hour * HOUR + minute * MINUTE + second * SECOND + milli)
    }
}

/// `min` to `max` ASCII digits at the start of `s`, as many as there are: the number and how
/// many.
fn digits(s: &str, min: usize, max: usize) -> Option<(i64, usize)> {
    let len = s.bytes().take(max).take_while(u8::is_ascii_digit).count();
    if len < min {
        return None;
    }
    Some((s[..len].parse().ok()?, len))
}

/// A month's name at the start of `s`, long or its first three letters, any case: 1–12 and its
/// length.
fn month_name(s: &str) -> Option<(u32, usize)> {
    MONTHS.iter().enumerate().find_map(|(i, m)| {
        let starts = |n: usize| s.get(..n).is_some_and(|h| h.eq_ignore_ascii_case(&m[..n]));
        let len = if starts(m.len()) {
            m.len()
        } else if starts(3) {
            3
        } else {
            return None;
        };
        Some((i as u32 + 1, len))
    })
}

/// What follows an offset (`Z`, `+02:00`, `-0130`) at the start of `s`.
fn offset(s: &str) -> Option<&str> {
    if let Some(rest) = s.strip_prefix('Z') {
        return Some(rest);
    }
    let rest = s.strip_prefix(['+', '-'])?;
    let (_, len) = digits(rest, 2, 2)?;
    let rest = &rest[len..];
    let rest = rest.strip_prefix(':').unwrap_or(rest);
    let (_, len) = digits(rest, 2, 2)?;
    Some(&rest[len..])
}

/// An ISO date, `YYYY-MM-DD` with an optional `THH:mm[:ss]` or ` HH:mm[:ss]`: what Mermaid
/// falls back to when a date doesn't match the `dateFormat`.
pub fn parse_iso(s: &str) -> Option<Ms> {
    // every end is tried as a date before a duration, so turn those away cheaply
    let b = s.as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let fmt = match (b.len(), b.get(10)) {
        (10, _) => "YYYY-MM-DD",
        (16, Some(b'T')) => "YYYY-MM-DDTHH:mm",
        (19, Some(b'T')) => "YYYY-MM-DDTHH:mm:ss",
        (16, Some(b' ')) => "YYYY-MM-DD HH:mm",
        (19, Some(b' ')) => "YYYY-MM-DD HH:mm:ss",
        _ => return None,
    };
    DateFormat::new(fmt).parse(s)
}

/// A duration: `1d`, `2.5h`, `3w`, `90m`, `30s`, `500ms`, `2M`, `1y`. Months and years are
/// calendar steps, the rest fixed lengths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Duration {
    Fixed(Ms),
    Months(i64),
}

impl Duration {
    pub fn parse(s: &str) -> Option<Duration> {
        let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
        let (num, unit) = s.split_at(split);
        if num.is_empty() || num.starts_with('.') || num.ends_with('.') {
            return None;
        }
        let n: f64 = num.parse().ok()?;
        let fixed = |unit: Ms| {
            let ms = (n * unit as f64).round();
            (ms < 1e15).then_some(Duration::Fixed(ms as Ms))
        };
        match unit {
            "ms" => fixed(1),
            "s" => fixed(SECOND),
            "m" => fixed(MINUTE),
            "h" => fixed(HOUR),
            "d" => fixed(DAY),
            "w" => fixed(WEEK),
            "M" if n < 1e6 => Some(Duration::Months(n.round() as i64)),
            "y" if n < 1e5 => Some(Duration::Months(n.round() as i64 * 12)),
            _ => None,
        }
    }

    pub fn after(self, t: Ms) -> Option<Ms> {
        match self {
            Duration::Fixed(ms) => valid(t.checked_add(ms)?),
            Duration::Months(n) => add_months(t, n),
        }
    }
}

/// Append `t` printed by the d3 `strftime`-style `fmt` to `out`: `%Y %y %m %d %e %b %B %a %A
/// %H %I %p %M %S %L %j %U %W %V %u %w %x %X %c %%`, each with an optional `-` (no padding),
/// `_` (spaces) or `0` modifier. Anything else is copied as written.
pub fn format(fmt: &str, t: Ms, out: &mut String) {
    use std::fmt::Write;
    let f = fields(t);
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let pad = match chars.peek() {
            Some(&p @ ('-' | '_' | '0')) => {
                chars.next();
                Some(p)
            }
            _ => None,
        };
        let Some(spec) = chars.next() else {
            out.push('%');
            break;
        };
        let num = |out: &mut String, n: i64, width: usize, default: char| {
            let _ = match pad.unwrap_or(default) {
                '-' => write!(out, "{n}"),
                '_' => write!(out, "{n:>width$}"),
                _ => write!(out, "{n:0width$}"),
            };
        };
        let month = MONTHS[(f.month - 1) as usize];
        let wday = WEEKDAYS[f.weekday as usize];
        match spec {
            'Y' => num(out, f.year, 4, '0'),
            'y' => num(out, f.year.rem_euclid(100), 2, '0'),
            'm' => num(out, f.month.into(), 2, '0'),
            'd' => num(out, f.day.into(), 2, '0'),
            'e' => num(out, f.day.into(), 2, '_'),
            'b' => out.push_str(&month[..3]),
            'B' => out.push_str(month),
            'a' => {
                out.push(wday.as_bytes()[0].to_ascii_uppercase().into());
                out.push_str(&wday[1..3]);
            }
            'A' => {
                out.push(wday.as_bytes()[0].to_ascii_uppercase().into());
                out.push_str(&wday[1..]);
            }
            'H' => num(out, f.hour.into(), 2, '0'),
            'I' => num(out, ((f.hour + 11) % 12 + 1).into(), 2, '0'),
            'p' => out.push_str(if f.hour < 12 { "AM" } else { "PM" }),
            'M' => num(out, f.minute.into(), 2, '0'),
            'S' => num(out, f.second.into(), 2, '0'),
            'L' => num(out, f.milli.into(), 3, '0'),
            'j' => num(out, i64::from(f.yday) + 1, 3, '0'),
            'U' => num(out, i64::from((f.yday + 7 - f.weekday) / 7), 2, '0'),
            'W' => num(
                out,
                i64::from((f.yday + 7 - (f.weekday + 6) % 7) / 7),
                2,
                '0',
            ),
            'V' => num(out, iso_week(t), 2, '0'),
            'u' => num(out, i64::from((f.weekday + 6) % 7 + 1), 1, '0'),
            'w' => num(out, f.weekday.into(), 1, '0'),
            'x' => format("%-m/%-d/%Y", t, out),
            'X' => format("%-I:%M:%S %p", t, out),
            'c' => format("%-m/%-d/%Y, %-I:%M:%S %p", t, out),
            '%' => out.push('%'),
            other => {
                out.push('%');
                if let Some(p) = pad {
                    out.push(p);
                }
                out.push(other);
            }
        }
    }
}

/// The ISO 8601 week number of `t`'s day: weeks start on Monday, and week 1 holds the year's
/// first Thursday.
fn iso_week(t: Ms) -> i64 {
    let d = day(t);
    // the Thursday of this week decides its year
    let thursday = d - i64::from((weekday(d) + 6) % 7) + 3;
    let (y, _, _) = civil_from_days(thursday);
    (thursday - days_from_civil(y, 1, 1)) / 7 + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(s: &str) -> Ms {
        DateFormat::default().parse(s).expect(s)
    }

    fn fmt(f: &str, t: Ms) -> String {
        let mut out = String::new();
        format(f, t, &mut out);
        out
    }

    #[test]
    fn civil_dates_round_trip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        for d in [-719_528, -1, 0, 59, 10_000, 2_932_896] {
            let (y, m, dd) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
        assert_eq!(civil_from_days(-719_528), (0, 1, 1));
        assert_eq!(civil_from_days(2_932_896), (9999, 12, 31));
        assert_eq!(weekday(0), 4, "1970-01-01 was a Thursday");
    }

    #[test]
    fn dates_parse_by_the_date_format() {
        assert_eq!(date("2014-01-01"), days_from_civil(2014, 1, 1) * DAY);
        let f = DateFormat::new("DD.MM.YYYY HH:mm");
        assert_eq!(
            f.parse("03.02.2024 13:30"),
            Some(days_from_civil(2024, 2, 3) * DAY + 13 * HOUR + 30 * MINUTE)
        );
        assert_eq!(f.parse("3.02.2024 13:30"), None, "DD wants two digits");
        assert_eq!(f.parse("03.02.2024"), None, "the whole format must match");
        let f = DateFormat::new("MMM Do, YY h:mm a");
        assert_eq!(
            f.parse("Feb 3rd, 24 1:05 pm"),
            Some(days_from_civil(2024, 2, 3) * DAY + 13 * HOUR + 5 * MINUTE)
        );
        assert_eq!(DateFormat::new("X").parse("86400"), Some(DAY));
        assert_eq!(DateFormat::new("x").parse("-1000"), Some(-SECOND));
        assert_eq!(DateFormat::new("[week] W").parse("week W"), Some(0));
        assert_eq!(
            DateFormat::new("HH:mm").parse("09:15"),
            Some(9 * HOUR + 15 * MINUTE)
        );
        assert_eq!(
            DateFormat::new("YYYY-MM-DDTHH:mm:ssZ").parse("2024-01-01T00:00:00+02:00"),
            Some(date("2024-01-01"))
        );
    }

    #[test]
    fn impossible_dates_do_not_parse() {
        let f = DateFormat::default();
        assert_eq!(f.parse("2023-02-29"), None);
        assert_eq!(f.parse("2024-13-01"), None);
        assert!(f.parse("2024-02-29").is_some());
        assert_eq!(f.parse("１２３４-01-01"), None);
        assert_eq!(DateFormat::new("X").parse("999999999999999"), None);
        assert_eq!(
            parse_iso("2024-05-06T07:08"),
            Some(date("2024-05-06") + 7 * HOUR + 8 * MINUTE)
        );
    }

    #[test]
    fn durations_and_calendar_months() {
        assert_eq!(Duration::parse("3d"), Some(Duration::Fixed(3 * DAY)));
        assert_eq!(Duration::parse("1.5h"), Some(Duration::Fixed(90 * MINUTE)));
        assert_eq!(Duration::parse("250ms"), Some(Duration::Fixed(250)));
        assert_eq!(Duration::parse("2y"), Some(Duration::Months(24)));
        for bad in ["d", "1", "1.d", ".5d", "1x", "-1d", "1e3d", "1dd"] {
            assert_eq!(Duration::parse(bad), None, "{bad}");
        }
        let jan31 = date("2024-01-31");
        assert_eq!(Duration::Months(1).after(jan31), Some(date("2024-02-29")));
        assert_eq!(
            Duration::Months(-1).after(date("2024-03-31")),
            Some(date("2024-02-29"))
        );
        assert_eq!(Duration::Months(12 * 9000).after(jan31), None);
    }

    #[test]
    fn axis_formats_print_like_d3() {
        let t = date("2024-03-05") + 14 * HOUR + 7 * MINUTE;
        assert_eq!(fmt("%Y-%m-%d", t), "2024-03-05");
        assert_eq!(fmt("%b %e", t), "Mar  5");
        assert_eq!(fmt("%-d %B", t), "5 March");
        assert_eq!(fmt("%a %A", t), "Tue Tuesday");
        assert_eq!(fmt("%H:%M %I%p", t), "14:07 02PM");
        assert_eq!(fmt("%j %U %W %V %u %w", t), "065 09 10 10 2 2");
        assert_eq!(fmt("%x", t), "3/5/2024");
        assert_eq!(fmt("100%% %q", t), "100% %q");
        assert_eq!(fmt("%V", date("2021-01-03")), "53");
        assert_eq!(fmt("%V", date("2024-12-30")), "01");
    }
}
