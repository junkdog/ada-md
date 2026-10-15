//! Tables, P1 edition: a grid when every column fits at its natural width, otherwise one card per
//! row. P2 replaces this with CSS-auto-like column distribution, the `t` toggle and sticky
//! headers (ADR-11); the record layout stays.

use super::wrap::{self, Wrapped};
use super::{Builder, RowKind};
use crate::doc::{Align, LineRange, Table, TableRow};
use crate::theme::{StyleKey, TableBorder};

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

/// Wrapping width for a cell laid out on one line.
const UNBOUNDED: u16 = u16::MAX / 2;

pub(super) fn layout(b: &mut Builder<'_>, t: &Table) {
    let cols = t
        .aligns
        .len()
        .max(t.header.cells.len())
        .max(t.rows.iter().map(|r| r.cells.len()).max().unwrap_or(0));
    if cols == 0 {
        return;
    }
    let border = match b.g.table_border {
        TableBorder::Rounded => Some(&ROUNDED),
        TableBorder::Ascii => Some(&ASCII),
        TableBorder::None => None,
    };
    let header = cells(b, &t.header, cols, StyleKey::TableHeader, UNBOUNDED);
    let body: Vec<_> = t
        .rows
        .iter()
        .map(|r| cells(b, r, cols, b.text_key, UNBOUNDED))
        .collect();
    let widths: Vec<u16> = (0..cols)
        .map(|c| {
            body.iter()
                .map(|r| r[c].width())
                .fold(header[c].width(), u16::max)
                .max(1)
        })
        .collect();
    let total = widths.iter().map(|&w| u32::from(w) + 2).sum::<u32>()
        + border.map_or(2 * (cols as u32 - 1), |_| cols as u32 + 1);
    if total <= u32::from(b.table_avail()) {
        grid(b, t, &widths, &header, &body, border);
    } else {
        records(b, t, cols, border);
    }
}

fn cells(b: &Builder<'_>, row: &TableRow, cols: usize, key: StyleKey, avail: u16) -> Vec<Wrapped> {
    (0..cols)
        .map(|c| {
            row.cells.get(c).map_or_else(Wrapped::default, |inl| {
                wrap::wrap(&inl.runs, key, avail, b.doc, b.g, None)
            })
        })
        .collect()
}

fn grid(
    b: &mut Builder<'_>,
    t: &Table,
    widths: &[u16],
    header: &[Wrapped],
    body: &[Vec<Wrapped>],
    border: Option<&Border>,
) {
    if let Some(bd) = border {
        border_row(b, widths, bd.top, bd.horiz);
    }
    grid_row(b, &t.header, header, widths, &t.aligns, border);
    if let Some(bd) = border {
        border_row(b, widths, bd.sep, bd.horiz);
    }
    for (row, cells) in t.rows.iter().zip(body) {
        grid_row(b, row, cells, widths, &t.aligns, border);
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
            if c > 0 {
                match border {
                    Some(bd) => b.text(bd.vert, StyleKey::TableBorder),
                    None => b.text("  ", StyleKey::Text),
                }
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

/// One card per body row: `header  value` pairs with a hanging indent.
fn records(b: &mut Builder<'_>, t: &Table, cols: usize, border: Option<&Border>) {
    let avail = b.avail();
    let bar = border.map_or("", |bd| bd.vert);
    let bar_width = if bar.is_empty() { 0 } else { 2 };
    let natural = cells(b, &t.header, cols, StyleKey::TableHeader, UNBOUNDED);
    let key_width = natural
        .iter()
        .map(Wrapped::width)
        .max()
        .unwrap_or(0)
        .min(avail / 3)
        .max(1);
    let value_width = avail.saturating_sub(bar_width + key_width + 2).max(1);
    let keys = cells(b, &t.header, cols, StyleKey::TableHeader, key_width);
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
        let values = cells(b, row, cols, b.text_key, value_width);
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
