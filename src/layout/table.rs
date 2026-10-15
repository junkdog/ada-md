//! Tables (ADR-11, architecture §4). Columns are measured once per Doc ([`measure`]), then
//! distributed per width the way CSS automatic table layout does it ([`distribute`]). A grid that
//! can't fit without breaking words becomes one card per row in `auto` mode; `t` forces either.

use std::borrow::Cow;

use super::wrap::{self, Wrapped};
use super::{Builder, GridRows, RowKind, TableMode};
use crate::doc::{Align, Doc, LineRange, Table, TableRow};
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

fn column_count(t: &Table) -> usize {
    t.aligns
        .len()
        .max(t.header.cells.len())
        .max(t.rows.iter().map(|r| r.cells.len()).max().unwrap_or(0))
}

/// Measure every column of `t`. Inline styles don't change widths; inline code counts its padding.
pub fn measure(t: &Table, doc: &Doc, glyphs: &Glyphs) -> Vec<ColMeasure> {
    let mut cols = vec![ColMeasure::default(); column_count(t)];
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

pub(super) fn layout(b: &mut Builder<'_>, t: &Table) {
    let cols: Cow<'_, [ColMeasure]> = match b.measure.tables.get(t.id as usize) {
        Some(m) if m.len() == column_count(t) => Cow::Borrowed(m),
        _ => Cow::Owned(measure(t, b.doc, b.g)),
    };
    if cols.is_empty() {
        return;
    }
    let border = match b.g.table_border {
        TableBorder::Rounded => Some(&ROUNDED),
        TableBorder::Ascii => Some(&ASCII),
        TableBorder::None => None,
    };
    // Padding of one cell on each side of a column, plus the vertical borders.
    let chrome = 2 * cols.len() as u16 + border.map_or(0, |_| cols.len() as u16 + 1);
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
    match widths {
        Some(widths) => grid(b, t, &widths, border),
        None => records(b, t, &cols, border),
    }
}

fn cells(b: &Builder<'_>, row: &TableRow, widths: &[u16], key: StyleKey) -> Vec<Wrapped> {
    widths
        .iter()
        .enumerate()
        .map(|(c, &w)| {
            row.cells.get(c).map_or_else(Wrapped::default, |inl| {
                wrap::wrap(&inl.runs, key, w, b.doc, b.g, None)
            })
        })
        .collect()
}

fn grid(b: &mut Builder<'_>, t: &Table, widths: &[u16], border: Option<&Border>) {
    if let Some(bd) = border {
        border_row(b, widths, bd.top, bd.horiz);
    }
    let header_start = b.out.row_count();
    let header = cells(b, &t.header, widths, StyleKey::TableHeader);
    grid_row(b, &t.header, &header, widths, &t.aligns, border);
    if let Some(bd) = border
        && !t.rows.is_empty()
    {
        border_row(b, widths, bd.sep, bd.horiz);
    }
    let header = header_start..b.out.row_count();
    for row in &t.rows {
        let cells = cells(b, row, widths, b.text_key);
        grid_row(b, row, &cells, widths, &t.aligns, border);
    }
    if !t.rows.is_empty() {
        b.out.grids.push(GridRows {
            header,
            end: b.out.row_count(),
        });
    }
    if let Some(bd) = border {
        border_row(b, widths, bd.bottom, bd.horiz);
    }
}

fn border_row(b: &mut Builder<'_>, widths: &[u16], [left, mid, right]: [&str; 3], horiz: &str) {
    b.begin_row();
    b.text(left, StyleKey::TableBorder);
    for (i, &w) in widths.iter().enumerate() {
        if i > 0 {
            b.text(mid, StyleKey::TableBorder);
        }
        b.text(&horiz.repeat(w as usize + 2), StyleKey::TableBorder);
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
        for (c, cell) in cells.iter().enumerate() {
            if c > 0
                && let Some(bd) = border
            {
                b.text(bd.vert, StyleKey::TableBorder);
            }
            let line = cell.lines.get(k);
            let slack = widths[c].saturating_sub(line.map_or(0, |l| l.width));
            let (left, right) = match aligns.get(c) {
                Some(Align::Center) => (slack / 2, slack - slack / 2),
                Some(Align::Right) => (slack, 0),
                _ => (0, slack),
            };
            b.text(&" ".repeat(1 + left as usize), StyleKey::Text);
            if let Some(line) = line {
                b.emit_line(cell, line);
            }
            b.text(&" ".repeat(1 + right as usize), StyleKey::Text);
        }
        if let Some(bd) = border {
            b.text(bd.vert, StyleKey::TableBorder);
        }
        b.end_row(row.src, RowKind::Text);
    }
}

/// One card per body row: `header  value` pairs with a hanging indent. The key column is as wide
/// as the widest header, capped at a third of the width.
fn records(b: &mut Builder<'_>, t: &Table, cols: &[ColMeasure], border: Option<&Border>) {
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
    let keys = cells(
        b,
        &t.header,
        &vec![key_width; cols.len()],
        StyleKey::TableHeader,
    );
    let value_widths = vec![value_width; cols.len()];
    for (i, row) in t.rows.iter().enumerate() {
        b.begin_row();
        match border {
            Some(bd) => {
                b.text(&format!("{} {} ", bd.card, i + 1), StyleKey::TableBorder);
                b.fill_with(bd.horiz, avail, StyleKey::TableBorder);
            }
            None => b.text(&format!("{}.", i + 1), StyleKey::TableBorder),
        }
        b.end_row(row.src, RowKind::Decor);
        let values = cells(b, row, &value_widths, b.text_key);
        for (key, value) in keys.iter().zip(&values) {
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
                b.text(
                    &" ".repeat((key_width.saturating_sub(used) + 2) as usize),
                    StyleKey::Text,
                );
                if let Some(line) = value.lines.get(k) {
                    b.emit_line(value, line);
                }
                b.end_row(row.src, RowKind::Text);
            }
        }
    }
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

    #[test]
    fn shares_sum_exactly() {
        assert_eq!(share(10, &[1, 1, 1]), [4, 3, 3]);
        assert_eq!(share(0, &[5, 5]), [0, 0]);
        assert_eq!(share(7, &[0, 0]), [0, 0]);
        assert_eq!(share(5, &[0, 3, 7]), [0, 2, 3]);
    }
}
