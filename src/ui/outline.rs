//! The outline popup: the headings (`o`), indented by level, the links (`l`), each with where it
//! leads, the themes (`T`), or the columns to sort a table by (`s`); filtered by what's typed.

use crate::grid::Grid;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use unicode_width::UnicodeWidthStr;

use crate::app::{Listing, OutlineEntry, State};
use crate::theme::StyleKey;

const MAX_WIDTH: u16 = 72;
/// Links carry their destinations too.
const MAX_LINKS_WIDTH: u16 = 100;
/// Between a link's text and its destination.
const GAP: usize = 2;
const BROKEN: &str = " ✗";

fn indent(level: u8) -> usize {
    2 * usize::from(level.saturating_sub(1))
}

/// The columns an entry wants, its destination included.
fn entry_width(entry: &OutlineEntry) -> usize {
    let dest = entry.dest.as_deref().map_or(0, |d| GAP + d.width());
    let broken = if entry.broken { BROKEN.width() } else { 0 };
    indent(entry.level) + entry.text.width() + dest + broken
}

pub fn draw(buf: &mut Grid, area: Rect, state: &State) {
    let Some(outline) = &state.outline else {
        return;
    };
    let theme = &state.theme;
    let entries = state.outline_entries();
    // Sized for every entry, not just the filtered ones, so the frame holds still while typing.
    let all = state.outline_all();
    let widest = all.iter().map(entry_width).max().unwrap_or(0);
    let (max_width, what) = match outline.listing {
        Listing::Headings => (MAX_WIDTH, "outline"),
        Listing::Links => (MAX_LINKS_WIDTH, "links"),
        Listing::Themes => (MAX_WIDTH, "themes"),
        Listing::Columns(_) => (MAX_WIDTH, "sort by"),
    };
    let width = (widest as u16 + 4)
        .clamp(32, max_width)
        .min(area.width.saturating_sub(2));
    let height = (all.len().max(1) as u16 + 3).min(area.height.saturating_sub(2));
    if width < 8 || height < 4 {
        return;
    }
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + popup_y(state, area, height),
        width,
        height,
    };
    for y in rect.top()..rect.bottom() {
        for x in rect.left()..rect.right() {
            buf[(x, y)].reset();
        }
    }
    buf.set_style(rect, theme.style(StyleKey::Popup));

    // the border, with the title on top
    let border = theme.style(StyleKey::PopupBorder);
    let inner = usize::from(width - 2);
    let (top, bottom) = (rect.top(), rect.bottom() - 1);
    buf.set_string(rect.x, top, format!("╭{}╮", "─".repeat(inner)), border);
    buf.set_string(rect.x, bottom, format!("╰{}╯", "─".repeat(inner)), border);
    for y in top + 1..bottom {
        buf.set_string(rect.x, y, "│", border);
        buf.set_string(rect.right() - 1, y, "│", border);
    }
    let title = format!(
        " {what} {}/{} ",
        entries.len().min(outline.selected + 1),
        entries.len()
    );
    buf.set_stringn(
        rect.x + 2,
        top,
        &title,
        inner - 1,
        theme.style(StyleKey::PopupTitle),
    );
    // which of the tables on screen it sorts, when there's a choice
    let tables = state.tables_on_screen();
    if let (Some(i), true) = (
        state
            .sorting()
            .and_then(|t| tables.iter().position(|&s| s == t)),
        tables.len() > 1,
    ) {
        let hint = format!(" ← table {}/{} → ", i + 1, tables.len());
        let x = rect.right().saturating_sub(hint.width() as u16 + 2);
        if x > rect.x + 1 {
            buf.set_string(x, bottom, &hint, theme.style(StyleKey::PopupTitle));
        }
    }

    // the filter, then as many entries as fit, keeping the selection in view
    let x = rect.x + 1;
    let filter = format!(" › {}▏", outline.filter);
    buf.set_stringn(x, top + 1, &filter, inner, theme.style(StyleKey::Prompt));
    let rows = usize::from(bottom - top - 2);
    if entries.is_empty() {
        let none = match (outline.filter.is_empty(), outline.listing) {
            (true, Listing::Headings) => "   no headings",
            (true, Listing::Links) => "   no links",
            (true, Listing::Themes) => "   no themes",
            (true, Listing::Columns(_)) => "   no table on screen to sort",
            (false, _) => "   no match",
        };
        buf.set_stringn(x, top + 2, none, inner, theme.style(StyleKey::PopupBorder));
        return;
    }
    let first = outline
        .selected
        .saturating_sub(rows / 2)
        .min(entries.len().saturating_sub(rows));
    for (y, (i, entry)) in (top + 2..bottom).zip(entries.iter().enumerate().skip(first)) {
        let selected = i == outline.selected;
        let mut style = theme.style(StyleKey::Popup);
        if selected {
            style = theme.style(StyleKey::PopupSelected);
        } else if entry.level == 1 {
            style = style.add_modifier(Modifier::BOLD);
        }
        let text = format!(" {}{}", " ".repeat(indent(entry.level)), entry.text);
        buf.set_stringn(x, y, format!("{text:<inner$}"), inner, style);
        // the destination, dimmed, and a mark on broken links; the selection keeps its colors
        let mut col = text.width();
        let mut tail = |s: &str, key: StyleKey| {
            if col < inner {
                let style = if selected { style } else { theme.style(key) };
                let (end, _) = buf.set_stringn(x + col as u16, y, s, inner - col, style);
                col = usize::from(end - x);
            }
        };
        if let Some(dest) = &entry.dest {
            tail(&format!("{:GAP$}{dest}", ""), StyleKey::PopupBorder);
        }
        if entry.broken {
            tail(BROKEN, StyleKey::LinkBroken);
        }
    }
}

/// The popup's row in `area`: one below the top, unless the table the column list sorts shows
/// more of itself with the popup at the bottom.
fn popup_y(state: &State, area: Rect, height: u16) -> u16 {
    let top = 1;
    let bottom = area.height.saturating_sub(height + 1).max(top);
    let rows = state
        .sorting()
        .zip(state.doc.as_ref())
        .and_then(|(t, doc)| {
            let table = doc.layout.tables.iter().find(|r| r.table == t)?;
            let at = |r: u32| r.saturating_sub(state.scroll).min(u32::from(area.height));
            Some(at(table.rows.start)..at(table.rows.end))
        });
    let Some(rows) = rows else {
        return top;
    };
    let hidden = |y: u16| {
        let popup = u32::from(y)..u32::from(y + height);
        popup
            .end
            .min(rows.end)
            .saturating_sub(popup.start.max(rows.start))
    };
    if hidden(bottom) < hidden(top) {
        bottom
    } else {
        top
    }
}
