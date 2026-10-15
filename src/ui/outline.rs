//! The outline popup (`o`): the headings, filtered by what's typed, indented by level.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use unicode_width::UnicodeWidthStr;

use crate::app::State;
use crate::theme::StyleKey;

const MAX_WIDTH: u16 = 72;

pub fn draw(buf: &mut Buffer, area: Rect, state: &State) {
    let Some(outline) = &state.outline else {
        return;
    };
    let theme = &state.theme;
    let entries = state.outline_entries();
    let indent = |level: u8| 2 * usize::from(level.saturating_sub(1));
    // Sized for every heading, not just the filtered ones, so the frame holds still while typing.
    let Some(doc) = &state.doc else {
        return;
    };
    let all = &doc.layout.headings;
    let widest = all
        .iter()
        .filter_map(|h| {
            let text = &doc.doc.anchors.get(h.anchor as usize)?.text;
            Some(indent(h.level) + text.width())
        })
        .max()
        .unwrap_or(0);
    let width = (widest as u16 + 4)
        .clamp(32, MAX_WIDTH)
        .min(area.width.saturating_sub(2));
    let height = (all.len().max(1) as u16 + 3).min(area.height.saturating_sub(2));
    if width < 8 || height < 4 {
        return;
    }
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + 1,
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
        " outline {}/{} ",
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

    // the filter, then as many entries as fit, keeping the selection in view
    let x = rect.x + 1;
    let filter = format!(" › {}▏", outline.filter);
    buf.set_stringn(x, top + 1, &filter, inner, theme.style(StyleKey::Prompt));
    let rows = usize::from(bottom - top - 2);
    if entries.is_empty() {
        let none = if outline.filter.is_empty() {
            "   no headings"
        } else {
            "   no match"
        };
        buf.set_stringn(x, top + 2, none, inner, theme.style(StyleKey::PopupBorder));
        return;
    }
    let first = outline
        .selected
        .saturating_sub(rows / 2)
        .min(entries.len().saturating_sub(rows));
    for (y, (i, entry)) in (top + 2..bottom).zip(entries.iter().enumerate().skip(first)) {
        let mut style = theme.style(StyleKey::Popup);
        if i == outline.selected {
            style = theme.style(StyleKey::PopupSelected);
        } else if entry.level == 1 {
            style = style.add_modifier(Modifier::BOLD);
        }
        let text = format!(" {}{}", " ".repeat(indent(entry.level)), entry.text);
        buf.set_stringn(x, y, format!("{text:<inner$}"), inner, style);
    }
}
