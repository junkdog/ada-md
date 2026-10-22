//! A gantt chart drawn to the text width: a column of task names, then a bar per task on a date
//! axis that takes the rest. The axis is a sticky header, its ticks run down the chart as dotted
//! lines, and a section is a heading row. Names too long for their column are cut and listed
//! under the chart, as a sequence diagram's labels are.

use super::time::{self, DAY, HOUR, MINUTE, Ms, SECOND, WEEK};
use super::{Gantt, Row, Step, Tag};
use crate::diagram::canvas::{Canvas, Pen};
use crate::diagram::{Drawn, cut, marker, width};
use crate::theme::StyleKey;

/// The fewest columns the bars get; below that the chart scrolls.
const MIN_AREA: usize = 16;

/// The name column takes at most this share of the width, in fifths, and at least
/// [`MIN_NAMES`] columns.
const NAMES_FIFTHS: usize = 2;
const MIN_NAMES: usize = 12;

/// The tick intervals tried when none is given, finest first.
const STEPS: &[Step] = &[
    Step::Fixed(SECOND),
    Step::Fixed(5 * SECOND),
    Step::Fixed(15 * SECOND),
    Step::Fixed(30 * SECOND),
    Step::Fixed(MINUTE),
    Step::Fixed(5 * MINUTE),
    Step::Fixed(15 * MINUTE),
    Step::Fixed(30 * MINUTE),
    Step::Fixed(HOUR),
    Step::Fixed(3 * HOUR),
    Step::Fixed(6 * HOUR),
    Step::Fixed(12 * HOUR),
    Step::Days(1),
    Step::Days(2),
    Step::Weeks(1),
    Step::Weeks(2),
    Step::Months(1),
    Step::Months(3),
    Step::Months(6),
    Step::Months(12),
    Step::Months(24),
    Step::Months(60),
    Step::Months(120),
    Step::Months(600),
    Step::Months(1200),
    Step::Months(6000),
];

/// Columns kept clear between two axis labels.
const LABEL_GAP: usize = 2;

pub fn draw(g: &Gantt, avail: usize, ascii: bool) -> Option<Drawn> {
    let tasks = || {
        g.rows.iter().filter_map(|r| match r {
            Row::Task(t) => Some(t),
            Row::Section(_) => None,
        })
    };
    let indent = |in_section: bool| if in_section { 2 } else { 0 };
    let names = tasks()
        .map(|t| indent(t.in_section) + width(&t.name))
        .max()?;
    let cap = MIN_NAMES.max(avail * NAMES_FIFTHS / 5);
    let names = names.min(cap);
    let left = names + 1;
    let area = avail.saturating_sub(left).max(MIN_AREA);
    let t0 = tasks().map(|t| t.start).min()?;
    let t1 = tasks().map(|t| t.end).max()?;
    // all at one instant: a day's span, the instant at its start
    let t1 = if t1 > t0 { t1 } else { t0 + DAY };
    let scale = Scale {
        t0,
        span: t1 - t0,
        left,
        area,
    };

    let mut canvas = Canvas::new(left + area, ascii);
    let mut row = 0;
    if !g.title.is_empty() {
        canvas.label(row, 0, &g.title, StyleKey::DiagramActor);
        row += 2;
    }
    let header = row..row + 2;
    let axis = row + 1;
    let bottom = axis + g.rows.len();
    canvas.grow(bottom + 1);
    canvas.hline(
        axis,
        left,
        left + area - 1,
        Pen::SOLID,
        StyleKey::DiagramLine,
    );
    let ticks = ticks(g, &scale);
    for (col, label) in &ticks {
        canvas.vline(*col, axis, bottom, Pen::DASHED, StyleKey::DiagramGrid);
        if let Some(label) = label {
            canvas.text(row, label.0, &label.1, StyleKey::DiagramNumber);
        }
    }

    let mut notes = Vec::new();
    let (bar, milestone) = if ascii { ('#', '*') } else { ('█', '◆') };
    for (i, r) in g.rows.iter().enumerate() {
        let row = axis + 1 + i;
        match r {
            Row::Section(name) => {
                canvas.label(row, 0, name, StyleKey::DiagramFrame);
            }
            Row::Task(t) => {
                let x = indent(t.in_section);
                let room = names.saturating_sub(x);
                if width(&t.name) <= room {
                    canvas.label(row, x, &t.name, StyleKey::DiagramLabel);
                } else {
                    let mark = marker(notes.len() + 1);
                    let head = cut(&t.name, room.saturating_sub(width(&mark))).trim_end();
                    let head = t.name.slice(0..head.len());
                    let after = canvas.label(row, x, &head, StyleKey::DiagramLabel);
                    canvas.text(row, after, &mark, StyleKey::DiagramNumber);
                    notes.push(t.name.clone());
                }
                let key = match t.tag {
                    Tag::Plain => StyleKey::DiagramBar,
                    Tag::Active => StyleKey::DiagramActive,
                    Tag::Done => StyleKey::DiagramDone,
                    Tag::Crit => StyleKey::DiagramCrit,
                };
                if t.milestone {
                    let mid = t.start + (t.end - t.start) / 2;
                    canvas.glyph(row, scale.col(mid), milestone, key);
                } else {
                    let (a, b) = (scale.col(t.start), scale.edge(t.end));
                    canvas.fill(row, a, b.max(a + 1), bar, key);
                }
            }
        }
    }
    Some(Drawn {
        canvas,
        header: Some(header),
        notes,
    })
}

/// Where instants land across the bars.
struct Scale {
    t0: Ms,
    span: Ms,
    left: usize,
    area: usize,
}

impl Scale {
    /// The column `t` falls in.
    fn col(&self, t: Ms) -> usize {
        self.edge(t).min(self.left + self.area - 1)
    }

    /// The column boundary at `t`: a bar from `a` to `b` covers `edge(a)..edge(b)`, so one
    /// ending at the chart's end fills its last column.
    fn edge(&self, t: Ms) -> usize {
        self.left + self.offset(t).clamp(0, self.area as i128) as usize
    }

    /// `t`'s offset in columns, unclamped, for spacing ticks.
    fn offset(&self, t: Ms) -> i128 {
        self.cols(t - self.t0)
    }

    /// How many columns `ms` spans: in `i64` when the product fits, as it does for any chart
    /// short of millennia, since `i128` division is a library call (DIA-02).
    fn cols(&self, ms: Ms) -> i128 {
        match ms.checked_mul(self.area as i64) {
            Some(p) => i128::from(p / self.span),
            None => i128::from(ms) * self.area as i128 / i128::from(self.span),
        }
    }

    fn end(&self) -> Ms {
        self.t0 + self.span
    }
}

/// A tick: its column, and its label's column and text if it has room.
type Tick = (usize, Option<(usize, String)>);

/// The axis ticks: at the `tickInterval` if one is given and fits, otherwise at the finest
/// interval whose labels all fit side by side.
fn ticks(g: &Gantt, scale: &Scale) -> Vec<Tick> {
    let try_step = |step: Step, all_labels: bool| {
        let instants = instants(step, g.week_start, scale)?;
        label(&instants, axis_format(g, step), scale, all_labels)
    };
    if let Some(step) = g.tick
        && let Some(ticks) = try_step(step, false)
    {
        return ticks;
    }
    // most steps are far too fine: skip those by their spacing, without making their ticks, when
    // even the first tick's label wouldn't fit between two
    // (the steps share a few formats, so the label is measured once per format)
    let mut first = String::new();
    let mut measured: Option<(&str, usize)> = None;
    STEPS
        .iter()
        .filter(|&&step| {
            let spacing = scale.cols(step.shortest());
            // no label fits in less than a column past the gap: skip it before measuring one
            if spacing <= LABEL_GAP as i128 {
                return false;
            }
            let fmt = axis_format(g, step);
            let w = match measured {
                Some((f, w)) if f == fmt => w,
                _ => {
                    first.clear();
                    time::format(fmt, scale.t0, &mut first);
                    let w = width(&first);
                    measured = Some((fmt, w));
                    w
                }
            };
            spacing >= (w + LABEL_GAP) as i128
        })
        .find_map(|&step| try_step(step, true))
        .unwrap_or_default()
}

impl Step {
    /// The shortest this step can be.
    fn shortest(self) -> Ms {
        match self {
            Step::Fixed(ms) => ms,
            Step::Days(n) => n * DAY,
            Step::Weeks(n) => n * WEEK,
            Step::Months(n) => n * 28 * DAY,
        }
    }
}

/// The `axisFormat`, or Mermaid's default, `%Y-%m-%d`; when the ticks are less than a day apart,
/// `%H:%M`, since the date alone would repeat.
fn axis_format(g: &Gantt, step: Step) -> &str {
    match (&g.axis, step) {
        (Some(f), _) => f,
        (None, Step::Fixed(ms)) if ms < MINUTE => "%H:%M:%S",
        (None, Step::Fixed(_)) => "%H:%M",
        (None, _) => "%Y-%m-%d",
    }
}

/// The instants `step` puts ticks at between the chart's ends, aligned to the calendar: `None`
/// when there would be more than one per column.
fn instants(step: Step, week_start: u32, scale: &Scale) -> Option<Vec<Ms>> {
    let (t0, t1) = (scale.t0, scale.end());
    let midnight = |t: Ms| time::day(t) * DAY;
    let ceil_day = |t: Ms| {
        let m = midnight(t);
        if m < t { m + DAY } else { m }
    };
    let (first, next): (Ms, Box<dyn Fn(Ms) -> Option<Ms>>) = match step {
        Step::Fixed(ms) => {
            let base = midnight(t0);
            let first = base + (t0 - base + ms - 1) / ms * ms;
            (first, Box::new(move |t| t.checked_add(ms)))
        }
        Step::Days(n) => (ceil_day(t0), Box::new(move |t| t.checked_add(n * DAY))),
        Step::Weeks(n) => {
            let d = ceil_day(t0);
            let behind =
                (i64::from(week_start) - i64::from(time::weekday(time::day(d)))).rem_euclid(7);
            (d + behind * DAY, Box::new(move |t| t.checked_add(n * WEEK)))
        }
        Step::Months(n) => {
            let (y, m, d) = time::civil_from_days(time::day(t0));
            let mut months = y * 12 + i64::from(m) - 1;
            if d > 1 || t0 % DAY != 0 {
                months += 1;
            }
            let months = (months + n - 1).div_euclid(n) * n;
            let first =
                time::days_from_civil(months.div_euclid(12), months.rem_euclid(12) as u32 + 1, 1)
                    * DAY;
            (first, Box::new(move |t| time::add_months(t, n)))
        }
    };
    let mut out = Vec::new();
    let mut t = first;
    while t < t1 {
        if out.len() >= scale.area {
            return None;
        }
        out.push(t);
        t = next(t)?;
    }
    Some(out)
}

/// The ticks at `instants`, labeled by `fmt` centered on each: `None` when `all` and two
/// labels would be closer than [`LABEL_GAP`]; otherwise a label without room is left out.
fn label(instants: &[Ms], fmt: &str, scale: &Scale, all: bool) -> Option<Vec<Tick>> {
    let right = scale.left + scale.area;
    let mut out = Vec::with_capacity(instants.len());
    // where the last label ended
    let mut clear = 0;
    let mut text = String::new();
    for &t in instants {
        if scale.offset(t) >= scale.area as i128 {
            break;
        }
        let col = scale.col(t);
        text.clear();
        time::format(fmt, t, &mut text);
        let w = width(&text);
        let at = col.saturating_sub(w / 2).min(right.saturating_sub(w));
        let fits = at >= clear && at + w <= right;
        if !fits && all {
            return None;
        }
        out.push((col, fits.then(|| (at, text.clone()))));
        if fits {
            clear = at + w + LABEL_GAP;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagram::{Diagram, parse};

    fn rows_at(src: &str, avail: usize, ascii: bool) -> Vec<String> {
        let Some(Diagram::Gantt(g)) = parse(src) else {
            panic!("didn't parse: {src}");
        };
        let d = draw(&g, avail, ascii).expect("draws");
        let mut rows: Vec<String> = (0..d.canvas.height())
            .map(|r| d.canvas.row_text(r))
            .collect();
        rows.extend(d.notes.iter().map(|n| format!("note: {n}")));
        rows
    }

    const PLAN: &str = "gantt\n    title Release plan\n    dateFormat YYYY-MM-DD\n    \
        section Build\n    Parser :done, p, 2024-01-01, 5d\n    Layout :active, l, after p, 6d\n    \
        Paint : after l, 4d\n    section Ship\n    Fuzz the whole thing for a week : crit, f, after l, 1w\n    \
        Release : milestone, after f, 0d";

    #[test]
    fn a_chart_fits_the_width() {
        let rows = rows_at(PLAN, 72, false);
        assert!(rows.iter().all(|r| width(r) <= 72));
        insta::assert_snapshot!(rows.join("\n"));
    }

    #[test]
    fn narrow_charts_cut_names_and_list_them() {
        insta::assert_snapshot!(rows_at(PLAN, 40, false).join("\n"));
    }

    #[test]
    fn ascii_glyphs() {
        insta::assert_snapshot!(rows_at(PLAN, 60, true).join("\n"));
    }

    #[test]
    fn hours_and_a_given_tick_interval() {
        let src = "gantt\ndateFormat HH:mm\naxisFormat %H:%M\ntickInterval 30minute\n\
                   Standup : 09:00, 15m\nReview : 30m\nLunch : 12:00, 1h";
        insta::assert_snapshot!(rows_at(src, 70, false).join("\n"));
    }

    #[test]
    fn random_charts_draw_without_panicking() {
        const LINES: &[&str] = &[
            "title A plan",
            "dateFormat YYYY-MM-DD",
            "axisFormat %a %e",
            "axisFormat %H:%M:%S.%L %j %V",
            "tickInterval 1day",
            "tickInterval 1millisecond",
            "tickInterval 7month",
            "weekday wednesday",
            "weekend friday",
            "excludes weekends, 2024-01-03, monday",
            "includes 2024-01-06",
            "inclusiveEndDates",
            "section Build",
            "section 日本語のセクション",
            "Parser : done, p, 2024-01-01, 3d",
            "Layout : active, l, after p, 2d",
            "Paint : crit, after l q, 1w",
            "Quick : q, 2024-01-02T09:30, 90m",
            "Release : milestone, until p",
            "Long : 2024-01-01, 2034-01-01",
            "A task with a name long enough to be cut short : 1d",
            "日本語のタスク : 1ms",
            "Ancient : 0000-01-01, 1d",
            "Far : 9999-12-30, 1d",
        ];
        let mut seed = 0x00ad_a5ee_d000_0006u64;
        let mut below = |n: usize| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize % n
        };
        let mut drawn = 0;
        for _ in 0..500 {
            let mut src = String::from("gantt\n");
            for _ in 0..below(20) {
                src.push_str(LINES[below(LINES.len())]);
                src.push('\n');
            }
            let Some(Diagram::Gantt(g)) = parse(&src) else {
                continue;
            };
            drawn += 1;
            for avail in [1, 20, 60, 200] {
                for ascii in [false, true] {
                    let d = draw(&g, avail, ascii).unwrap_or_else(|| panic!("{src}"));
                    for r in 0..d.canvas.height() {
                        assert!(width(&d.canvas.row_text(r)) <= d.canvas.width(), "{src}");
                    }
                }
            }
        }
        assert!(drawn > 100, "only {drawn} parsed");
    }

    #[test]
    fn ticks_align_to_the_calendar() {
        let scale = Scale {
            t0: time::DateFormat::default().parse("2024-01-15").unwrap(),
            span: 80 * DAY,
            left: 0,
            area: 80,
        };
        let months = instants(Step::Months(1), 0, &scale).unwrap();
        let mut got = Vec::new();
        for t in months {
            let mut s = String::new();
            time::format("%Y-%m-%d", t, &mut s);
            got.push(s);
        }
        assert_eq!(got, ["2024-02-01", "2024-03-01", "2024-04-01"]);
        let mondays = instants(Step::Weeks(1), 1, &scale).unwrap();
        assert!(mondays.iter().all(|&t| time::weekday(time::day(t)) == 1));
        assert!(
            instants(Step::Fixed(SECOND), 0, &scale).is_none(),
            "too many"
        );
    }
}
