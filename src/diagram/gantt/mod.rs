//! Gantt charts (DG-4): the IR and its line parser. Tasks are kept as written until the last
//! line, since Mermaid reads every date by the final `dateFormat` and lets `after` name a task
//! declared later; then their dates resolve to instants. Drawing is in [`draw`](mod@draw).

mod draw;
pub mod time;

use std::collections::HashMap;

pub use draw::draw;
use time::{DAY, DateFormat, Duration, Ms};

use super::{MAX_STATEMENTS, Source, Text};

/// The most days the `excludes` walk may step over in all, so a chart spanning millennia with
/// weekends excluded shows its source rather than stalling.
const MAX_EXCLUDE_STEPS: usize = 200_000;

#[derive(Debug, Default)]
pub struct Gantt {
    pub title: Text,
    /// The `axisFormat`, if one is given.
    pub axis: Option<String>,
    /// The `tickInterval`, if one is given.
    pub tick: Option<Step>,
    /// The day weekly ticks fall on, 0 for Sunday (`weekday`).
    pub week_start: u32,
    pub rows: Vec<Row>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Row {
    Section(Text),
    Task(Task),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Task {
    pub name: Text,
    pub start: Ms,
    /// Where its bar ends: a duration stretched over excluded days ends before those at its end.
    pub end: Ms,
    pub tag: Tag,
    pub milestone: bool,
    /// Under a `section`, so indented.
    pub in_section: bool,
}

/// What colors a task's bar. Of several tags, `done` wins, then `crit`, then `active`, as the
/// fill does in Mermaid (which marks `crit` with a red border beside the others).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    Plain,
    Active,
    Done,
    Crit,
}

/// A tick interval: fixed lengths up to hours, then calendar steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Fixed(Ms),
    Days(i64),
    Weeks(i64),
    Months(i64),
}

#[derive(Debug)]
struct RawTask {
    name: Text,
    tag: Tag,
    milestone: bool,
    id: Option<String>,
    /// `None`: after the task before it.
    start: Option<String>,
    end: String,
    in_section: bool,
}

#[derive(Debug)]
enum RawRow {
    Section(Text),
    Task(usize),
}

#[derive(Default)]
struct Parser {
    src: Source,
    gantt: Gantt,
    date_format: Option<String>,
    excludes: Vec<String>,
    includes: Vec<String>,
    /// `weekend friday`: the weekend is Friday and Saturday.
    weekend_friday: bool,
    inclusive_end: bool,
    tasks: Vec<RawTask>,
    rows: Vec<RawRow>,
    /// Inside a multi-line `accDescr { … }`.
    in_description: bool,
}

/// Parse the statements after the `gantt` line.
pub(super) fn parse<'a>(lines: impl Iterator<Item = (u32, &'a str)>, src: Source) -> Option<Gantt> {
    let mut p = Parser {
        src,
        ..Parser::default()
    };
    for (n, (_, line)) in lines.enumerate() {
        if n >= MAX_STATEMENTS {
            return None;
        }
        p.statement(line)?;
    }
    p.resolve()
}

impl Parser {
    fn statement(&mut self, line: &str) -> Option<()> {
        if self.in_description {
            self.in_description = !line.contains('}');
            return Some(());
        }
        let word = line
            .split(|c: char| c.is_whitespace() || c == ':')
            .next()
            .unwrap_or("");
        let rest = line[word.len()..].trim();
        match word.to_ascii_lowercase().as_str() {
            "dateformat" => self.date_format = Some(rest.to_owned()),
            "axisformat" => self.gantt.axis = Some(rest.to_owned()),
            "tickinterval" => self.gantt.tick = Some(tick_interval(rest)?),
            "excludes" => self.excludes.extend(list(&rest.to_lowercase())),
            "includes" => self.includes.extend(list(rest)),
            "inclusiveenddates" => self.inclusive_end = true,
            "weekday" => {
                let day = rest.to_ascii_lowercase();
                self.gantt.week_start = time::WEEKDAYS.iter().position(|d| *d == day)? as u32;
            }
            "weekend" => {
                self.weekend_friday = match rest.to_ascii_lowercase().as_str() {
                    "friday" => true,
                    "saturday" => false,
                    _ => return None,
                }
            }
            "todaymarker" | "topaxis" | "displaymode" | "click" | "acctitle" => {}
            "accdescr" => self.in_description = rest.starts_with('{') && !rest.contains('}'),
            "title" => {
                self.gantt.title = self.src.one_line(rest.strip_prefix(':').unwrap_or(rest));
            }
            "section" => self.rows.push(RawRow::Section(self.src.one_line(rest))),
            _ => self.task(line)?,
        }
        Some(())
    }

    /// `name : [tags,] [[id,] start,] end`, where the end is a date or a duration.
    fn task(&mut self, line: &str) -> Option<()> {
        let (name, data) = line.split_once(':')?;
        let name = self.src.one_line(name);
        if name.is_empty() {
            return None;
        }
        let mut data: Vec<&str> = data.split(',').map(str::trim).collect();
        let (mut tag, mut milestone) = (Tag::Plain, false);
        let (mut done, mut crit, mut active) = (false, false, false);
        while let Some(&first) = data.first() {
            match first {
                "done" => done = true,
                "crit" => crit = true,
                "active" => active = true,
                // `vert`, Mermaid's marker line, draws as a milestone
                "milestone" | "vert" => milestone = true,
                _ => break,
            }
            data.remove(0);
        }
        if done {
            tag = Tag::Done;
        } else if crit {
            tag = Tag::Crit;
        } else if active {
            tag = Tag::Active;
        }
        let own = |s: &str| (!s.is_empty()).then(|| s.to_owned());
        let (id, start, end) = match data[..] {
            [end] => (None, None, end),
            [start, end] => (None, Some(own(start)?), end),
            [id, start, end] => (own(id), Some(own(start)?), end),
            _ => return None,
        };
        let in_section = self.rows.iter().any(|r| matches!(r, RawRow::Section(_)));
        self.rows.push(RawRow::Task(self.tasks.len()));
        self.tasks.push(RawTask {
            name,
            tag,
            milestone,
            id,
            start,
            end: own(end)?,
            in_section,
        });
        Some(())
    }

    /// The tasks' dates, resolved in as many passes as `after` and `until` need.
    fn resolve(self) -> Option<Gantt> {
        let fmt = DateFormat::new(self.date_format.as_deref().unwrap_or("YYYY-MM-DD"));
        let date = |s: &str| fmt.parse(s).or_else(|| time::parse_iso(s));
        let calendar = Calendar::new(&self, &date);
        let ids: HashMap<&str, usize> = self
            .tasks
            .iter()
            .enumerate()
            .filter_map(|(i, t)| Some((t.id.as_deref()?, i)))
            .collect();
        let mut spans: Vec<Option<Span>> = vec![None; self.tasks.len()];
        let mut steps = 0;
        let mut left = self.tasks.len();
        while left > 0 {
            let before = left;
            for i in 0..self.tasks.len() {
                if spans[i].is_some() {
                    continue;
                }
                let resolver = Resolver {
                    tasks: &self.tasks,
                    spans: &spans,
                    ids: &ids,
                    date: &date,
                    inclusive_end: self.inclusive_end,
                };
                if let Some(span) = resolver.span(i)? {
                    let span = calendar.stretch(span, &mut steps)?;
                    spans[i] = Some(span);
                    left -= 1;
                }
            }
            if left == before {
                return None;
            }
        }
        let mut tasks: Vec<Option<RawTask>> = self.tasks.into_iter().map(Some).collect();
        let mut gantt = self.gantt;
        for row in self.rows {
            gantt.rows.push(match row {
                RawRow::Section(name) => Row::Section(name),
                RawRow::Task(i) => {
                    let t = tasks[i].take()?;
                    let span = spans[i]?;
                    Row::Task(Task {
                        name: t.name,
                        start: span.start,
                        end: span.drawn_end,
                        tag: t.tag,
                        milestone: t.milestone,
                        in_section: t.in_section,
                    })
                }
            });
        }
        (!tasks.is_empty()).then_some(gantt)
    }
}

#[derive(Clone, Copy, Debug)]
struct Span {
    start: Ms,
    /// Where the next task starts from.
    end: Ms,
    /// Where the bar ends.
    drawn_end: Ms,
    /// The end was a date, which excluded days don't move.
    manual: bool,
}

struct Resolver<'a, D> {
    tasks: &'a [RawTask],
    spans: &'a [Option<Span>],
    ids: &'a HashMap<&'a str, usize>,
    date: &'a D,
    inclusive_end: bool,
}

/// `Err` for a task that can't resolve, `Ok(None)` for one waiting on another.
type Resolved<T> = Option<Option<T>>;

impl<D: Fn(&str) -> Option<Ms>> Resolver<'_, D> {
    fn span(&self, i: usize) -> Resolved<Span> {
        let t = &self.tasks[i];
        let start = match &t.start {
            // the first task has nothing to follow
            None => match self.spans[i.checked_sub(1)?] {
                Some(prev) => prev.end,
                None => return Some(None),
            },
            Some(s) => match s.strip_prefix("after ") {
                Some(ids) => match self.refer(ids, |s| s.end, Iterator::max)? {
                    Some(t) => t,
                    None => return Some(None),
                },
                None => (self.date)(s)?,
            },
        };
        let (end, manual) = if let Some(ids) = t.end.strip_prefix("until ") {
            match self.refer(ids, |s| s.start, Iterator::min)? {
                // it names its end, as a date does, so excluded days leave it be
                Some(t) => (t, true),
                None => return Some(None),
            }
        } else if let Some(end) = (self.date)(&t.end) {
            let end = if self.inclusive_end {
                time::valid(end + DAY)?
            } else {
                end
            };
            (end, true)
        } else {
            (Duration::parse(&t.end)?.after(start)?, false)
        };
        let end = end.max(start);
        Some(Some(Span {
            start,
            end,
            drawn_end: end,
            manual,
        }))
    }

    /// The tasks `ids` names, those that exist, combined by `pick` over `field`; `Ok(None)`
    /// while one is unresolved.
    fn refer(
        &self,
        ids: &str,
        field: fn(&Span) -> Ms,
        pick: fn(std::vec::IntoIter<Ms>) -> Option<Ms>,
    ) -> Resolved<Ms> {
        let mut found = Vec::new();
        for id in ids.split_whitespace() {
            let Some(&j) = self.ids.get(id) else {
                continue;
            };
            match &self.spans[j] {
                Some(span) => found.push(field(span)),
                None => return Some(None),
            }
        }
        Some(Some(pick(found.into_iter())?))
    }
}

/// The days `excludes` leaves out, which a task given by its duration is stretched over.
struct Calendar {
    weekends: bool,
    weekend_friday: bool,
    weekdays: [bool; 7],
    dates: Vec<i64>,
    included: Vec<i64>,
}

impl Calendar {
    fn new(p: &Parser, date: &impl Fn(&str) -> Option<Ms>) -> Self {
        let mut c = Calendar {
            weekends: false,
            weekend_friday: p.weekend_friday,
            weekdays: [false; 7],
            dates: Vec::new(),
            included: p
                .includes
                .iter()
                .filter_map(|s| date(s))
                .map(time::day)
                .collect(),
        };
        for e in &p.excludes {
            if e == "weekends" {
                c.weekends = true;
            } else if let Some(d) = time::WEEKDAYS.iter().position(|d| d == e) {
                c.weekdays[d] = true;
            } else if let Some(t) = date(e) {
                c.dates.push(time::day(t));
            }
        }
        c
    }

    fn excluded(&self, day: i64) -> bool {
        if self.included.contains(&day) {
            return false;
        }
        let wd = time::weekday(day) as usize;
        let weekend = if self.weekend_friday {
            wd == 5 || wd == 6
        } else {
            wd == 6 || wd == 0
        };
        (self.weekends && weekend) || self.weekdays[wd] || self.dates.contains(&day)
    }

    fn is_empty(&self) -> bool {
        !self.weekends && !self.weekdays.contains(&true) && self.dates.is_empty()
    }

    /// Mermaid's `fixTaskDates`: walking a day at a time from the start, each excluded day
    /// pushes the end a day later, and the bar stops before the excluded days at the end.
    fn stretch(&self, span: Span, steps: &mut usize) -> Option<Span> {
        if span.manual || self.is_empty() {
            return Some(span);
        }
        let (mut t, mut end, mut drawn) = (span.start, span.end, span.end);
        let mut invalid = false;
        while t <= end {
            *steps += 1;
            if *steps > MAX_EXCLUDE_STEPS {
                return None;
            }
            if !invalid {
                drawn = end;
            }
            invalid = self.excluded(time::day(t));
            if invalid {
                end = time::valid(end + DAY)?;
            }
            t += DAY;
        }
        Some(Span {
            end,
            drawn_end: drawn,
            ..span
        })
    }
}

/// `excludes` and `includes` take words split at spaces and commas.
fn list(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split([' ', ',', '\t'])
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
}

/// `tickInterval 1day`: a count and a unit, `millisecond` to `month`.
fn tick_interval(s: &str) -> Option<Step> {
    let split = s.find(|c: char| !c.is_ascii_digit())?;
    let n: i64 = s[..split]
        .parse()
        .ok()
        .filter(|n| (1..=100_000).contains(n))?;
    Some(match &s[split..] {
        "millisecond" => Step::Fixed(n),
        "second" => Step::Fixed(n * time::SECOND),
        "minute" => Step::Fixed(n * time::MINUTE),
        "hour" => Step::Fixed(n * time::HOUR),
        "day" => Step::Days(n),
        "week" => Step::Weeks(n),
        "month" => Step::Months(n),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagram::{Diagram, parse as parse_diagram};

    fn gantt(src: &str) -> Gantt {
        match parse_diagram(src) {
            Some(Diagram::Gantt(g)) => g,
            _ => panic!("didn't parse: {src}"),
        }
    }

    fn date(s: &str) -> Ms {
        DateFormat::default().parse(s).expect(s)
    }

    fn tasks(g: &Gantt) -> Vec<(&str, Ms, Ms)> {
        g.rows
            .iter()
            .filter_map(|r| match r {
                Row::Task(t) => Some((&*t.name, t.start, t.end)),
                Row::Section(_) => None,
            })
            .collect()
    }

    #[test]
    fn tasks_follow_one_another_and_their_ids() {
        let g = gantt(
            "gantt\ntitle Plan\nsection One\nA : a1, 2024-01-01, 3d\nB : 2d\n\
             section Two\nC : after a1, 1w\nD : 2024-01-20, 2024-01-25",
        );
        assert_eq!(g.title, "Plan");
        assert_eq!(
            tasks(&g),
            [
                ("A", date("2024-01-01"), date("2024-01-04")),
                ("B", date("2024-01-04"), date("2024-01-06")),
                ("C", date("2024-01-04"), date("2024-01-11")),
                ("D", date("2024-01-20"), date("2024-01-25")),
            ]
        );
        assert_eq!(g.rows[0], Row::Section("One".into()));
    }

    #[test]
    fn after_and_until_may_name_later_tasks() {
        let g = gantt(
            "gantt\nA : a, after b c, 1d\nB : b, 2024-03-01, 2d\nC : c, 2024-03-02, 4d\n\
             D : 2024-02-20, until b",
        );
        let t = tasks(&g);
        assert_eq!(t[0].1, date("2024-03-06"), "after the later of b and c");
        assert_eq!(t[3].2, date("2024-03-01"), "until b starts");
    }

    #[test]
    fn tags_pick_the_color() {
        let g = gantt(
            "gantt\nA : done, crit, a, 2024-01-01, 1d\nB : active, crit, 1d\nC : active, 1d\n\
             M : milestone, 2024-01-05, 0d",
        );
        let tags: Vec<_> = g
            .rows
            .iter()
            .filter_map(|r| match r {
                Row::Task(t) => Some((t.tag, t.milestone)),
                Row::Section(_) => None,
            })
            .collect();
        assert_eq!(
            tags,
            [
                (Tag::Done, false),
                (Tag::Crit, false),
                (Tag::Active, false),
                (Tag::Plain, true)
            ]
        );
    }

    #[test]
    fn durations_stretch_over_excluded_days() {
        // 2024-01-05 is a Friday
        let g = gantt(
            "gantt\nexcludes weekends, 2024-01-09\nA : a, 2024-01-05, 1d\nB : 1d\n\
             C : 2024-01-05, 2024-01-08\nD : 2024-01-05, 2d",
        );
        let t = tasks(&g);
        assert_eq!(
            t[0].2,
            date("2024-01-06"),
            "the bar stops before the weekend"
        );
        assert_eq!(
            t[1].1,
            date("2024-01-08"),
            "but the next task starts after it"
        );
        assert_eq!(t[1].2, date("2024-01-09"));
        assert_eq!(t[2].2, date("2024-01-08"), "an end date doesn't move");
        assert_eq!(
            t[3].2,
            date("2024-01-09"),
            "Friday and Monday, the bar across the weekend"
        );
        // every day excluded: Mermaid would never stop, ada gives up
        let all = "gantt\nexcludes monday tuesday wednesday thursday friday saturday sunday\nA : 2024-01-01, 1d";
        assert!(parse_diagram(all).is_none());
    }

    #[test]
    fn date_formats_and_ticks() {
        let g = gantt(
            "gantt\ndateFormat HH:mm\naxisFormat %H:%M\ntickInterval 15minute\nweekday monday\n\
             A : 09:00, 30m\nB : 1h",
        );
        assert_eq!(tasks(&g)[1].2, 10 * time::HOUR + 30 * time::MINUTE);
        assert_eq!(g.tick, Some(Step::Fixed(15 * time::MINUTE)));
        assert_eq!(g.week_start, 1);
        let g = gantt("gantt\ndateFormat DD/MM/YYYY\nA : 01/02/2024, 1M\nB : 2024-03-01, 1d");
        assert_eq!(tasks(&g)[0].2, date("2024-03-01"));
        assert_eq!(tasks(&g)[1].1, date("2024-03-01"), "ISO dates still read");
    }

    #[test]
    fn what_does_not_resolve_shows_the_source() {
        for src in [
            "gantt\nA : 1d",                                 // nothing to follow
            "gantt\nA : a, after b, 1d\nB : b, after a, 1d", // a cycle
            "gantt\nA : after nobody, 1d",
            "gantt\nA : 2024-01-01, 1 day",
            "gantt\nA : 2024-02-30, 1d",
            "gantt\nnot a task",
            "gantt\ntitle only",
            "gantt\nA : a, b, c, d, 1d",
            "gantt\ntickInterval 1fortnight\nA : 2024-01-01, 1d",
        ] {
            assert!(parse_diagram(src).is_none(), "{src}");
        }
    }
}
