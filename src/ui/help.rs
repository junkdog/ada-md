//! The help overlay (`?`): every action with its keys, from the bindings in use, so rebound keys
//! show as they are. It scrolls when it doesn't fit.

use crate::grid::Grid;
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::app::State;
use crate::theme::StyleKey;

const MAX_WIDTH: u16 = 64;
/// The keys column is at most this wide; longer key lists push their description along.
const KEYS_WIDTH: usize = 16;

/// The help's lines, and whether each is a heading.
fn lines(state: &State) -> Vec<(String, bool)> {
    let help = state.config.keys.help();
    let keys_width = help
        .iter()
        .flat_map(|(_, entries)| entries)
        .map(|(keys, _)| keys.width())
        .max()
        .unwrap_or(0)
        .min(KEYS_WIDTH);
    let mut lines = Vec::new();
    for (group, entries) in help {
        if !lines.is_empty() {
            lines.push((String::new(), false));
        }
        lines.push((format!(" {group}"), true));
        for (keys, what) in entries {
            let pad = keys_width.saturating_sub(keys.width());
            lines.push((format!("   {keys}{}  {what}", " ".repeat(pad)), false));
        }
    }
    if let Some(file) = &state.config.file {
        lines.push((String::new(), false));
        lines.push((format!(" keys and options: {}", file.display()), false));
    }
    lines
}

/// Where the popup goes in `area`, for `lines` lines.
fn rect(area: Rect, lines: &[(String, bool)]) -> Option<Rect> {
    let widest = lines.iter().map(|(l, _)| l.width()).max().unwrap_or(0);
    let width = (widest as u16 + 3)
        .clamp(32, MAX_WIDTH)
        .min(area.width.saturating_sub(2));
    let height = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
    (width >= 8 && height >= 3).then(|| Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + 1,
        width,
        height,
    })
}

/// How far the help scrolls in the document's area.
pub fn max_scroll(state: &State) -> u16 {
    let area = Rect::new(0, 0, state.size.w, state.size.h.saturating_sub(1));
    let lines = lines(state);
    let shown = rect(area, &lines).map_or(0, |r| usize::from(r.height - 2));
    lines.len().saturating_sub(shown) as u16
}

pub fn draw(buf: &mut Grid, area: Rect, state: &State) {
    let Some(scroll) = state.help else {
        return;
    };
    let theme = &state.theme;
    let lines = lines(state);
    let Some(rect) = rect(area, &lines) else {
        return;
    };
    for y in rect.top()..rect.bottom() {
        for x in rect.left()..rect.right() {
            buf[(x, y)].reset();
        }
    }
    buf.set_style(rect, theme.style(StyleKey::Popup));

    let border = theme.style(StyleKey::PopupBorder);
    let inner = usize::from(rect.width - 2);
    let (top, bottom) = (rect.top(), rect.bottom() - 1);
    buf.set_string(rect.x, top, format!("╭{}╮", "─".repeat(inner)), border);
    buf.set_string(rect.x, bottom, format!("╰{}╯", "─".repeat(inner)), border);
    for y in top + 1..bottom {
        buf.set_string(rect.x, y, "│", border);
        buf.set_string(rect.right() - 1, y, "│", border);
    }
    let title = theme.style(StyleKey::PopupTitle);
    buf.set_stringn(rect.x + 2, top, " keys ", inner - 1, title);

    let rows = usize::from(bottom - top - 1);
    let first = usize::from(scroll).min(lines.len().saturating_sub(rows));
    for (y, (text, heading)) in (top + 1..bottom).zip(lines.iter().skip(first)) {
        let style = if *heading {
            title
        } else {
            theme.style(StyleKey::Popup)
        };
        buf.set_stringn(rect.x + 1, y, text, inner, style);
    }
    if first + rows < lines.len() {
        let more = " j/k scroll ";
        let x = rect.right().saturating_sub(more.width() as u16 + 2);
        buf.set_stringn(x, bottom, more, inner, border);
    }
}
