//! The document viewport and the status bar. [`view`] is pure: it reads [`State`] and writes the
//! frame, touching only the rows on screen (ADR-05).

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{Document, State};
use crate::layout::{Layout, Span};
use crate::theme::{StyleKey, Theme};

pub fn view(state: &State, frame: &mut Frame<'_>) {
    let area = frame.area();
    let buf = frame.buffer_mut();
    buf.set_style(area, state.theme.style(StyleKey::Text));
    if area.height == 0 {
        return;
    }
    let doc_area = Rect {
        height: area.height - 1,
        ..area
    };
    let status_area = Rect {
        y: area.bottom() - 1,
        height: 1,
        ..area
    };
    if let Some(doc) = &state.doc {
        paint_rows(buf, doc_area, &doc.layout, state.scroll, &state.theme);
    }
    status_bar(buf, status_area, state);
}

/// A laid-out span's style.
pub fn span_style(theme: &Theme, span: &Span) -> Style {
    // Images and footnote references have their own link-like styles.
    let link = span.link.is_some() && !matches!(span.key, StyleKey::Image | StyleKey::FootnoteRef);
    theme.span_style(span.key, span.inline, link)
}

/// Paint `layout` rows starting at `scroll` into `area`.
pub fn paint_rows(buf: &mut Buffer, area: Rect, layout: &Layout, scroll: u32, theme: &Theme) {
    let rows = layout
        .rows
        .iter()
        .skip(scroll as usize)
        .take(area.height as usize);
    for (y, row) in (area.y..).zip(rows) {
        let mut x = area.x;
        for (text, span) in layout.spans(row) {
            if x >= area.right() {
                break;
            }
            let room = usize::from(area.right() - x);
            (x, _) = buf.set_stringn(x, y, text, room, span_style(theme, span));
        }
    }
}

fn status_bar(buf: &mut Buffer, area: Rect, state: &State) {
    let theme = &state.theme;
    buf.set_style(area, theme.style(StyleKey::StatusBar));
    let width = usize::from(area.width);

    let (name, position) = match &state.doc {
        Some(doc) => (display_name(doc), position(state)),
        None => (String::new(), String::new()),
    };
    let name = format!(" {} ", elide_start(&name, width / 2));
    let position = format!(" {position} ");
    let (x, _) = buf.set_stringn(
        area.x,
        area.y,
        &name,
        width,
        theme.style(StyleKey::StatusFile),
    );

    let pos_width = position.width() as u16;
    let pos_x = area.right().saturating_sub(pos_width).max(x);
    buf.set_stringn(
        pos_x,
        area.y,
        &position,
        usize::from(area.right() - pos_x),
        theme.style(StyleKey::StatusPos),
    );

    let room = usize::from(pos_x.saturating_sub(x)).saturating_sub(2);
    let (text, key) = match (&state.status, &state.doc) {
        (Some(msg), _) => (msg.clone(), StyleKey::StatusMessage),
        (None, Some(doc)) => (
            heading_path(doc, state.scroll).join(" › "),
            StyleKey::StatusHeading,
        ),
        (None, None) => (String::new(), StyleKey::StatusBar),
    };
    if room > 0 && !text.is_empty() {
        buf.set_stringn(
            x + 1,
            area.y,
            elide_start(&text, room),
            room,
            theme.style(key),
        );
    }
}

fn display_name(doc: &Document) -> String {
    if doc.path.as_os_str() == "-" {
        "stdin".to_owned()
    } else {
        doc.path.display().to_string()
    }
}

/// `Top`, `Bot`, `All` or a percentage, like less and vim.
fn position(state: &State) -> String {
    let rows = state.doc.as_ref().map_or(0, |d| d.layout.row_count());
    let max = state.max_scroll();
    if rows <= state.viewport_height() {
        "All".to_owned()
    } else if state.scroll == 0 {
        "Top".to_owned()
    } else if state.scroll >= max {
        "Bot".to_owned()
    } else {
        let seen = u64::from(state.scroll + state.viewport_height());
        format!("{}%", seen * 100 / u64::from(rows))
    }
}

/// The headings enclosing row `top`, outermost first.
pub fn heading_path(doc: &Document, top: u32) -> Vec<&str> {
    let mut stack: Vec<(u8, u32)> = Vec::new();
    for h in doc.layout.headings.iter().take_while(|h| h.row <= top) {
        while stack.last().is_some_and(|&(level, _)| level >= h.level) {
            stack.pop();
        }
        stack.push((h.level, h.anchor));
    }
    stack
        .iter()
        .filter_map(|&(_, a)| doc.doc.anchors.get(a as usize))
        .map(|a| &*a.text)
        .collect()
}

/// Keep the end of `s`, which is the interesting part of a path, within `max` columns.
fn elide_start(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_owned();
    }
    if max == 0 {
        return String::new();
    }
    let mut used = 1;
    let mut start = s.len();
    for (i, c) in s.char_indices().rev() {
        used += c.width().unwrap_or(0);
        if used > max {
            break;
        }
        start = i;
    }
    format!("…{}", &s[start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eliding() {
        assert_eq!(elide_start("abcdef", 6), "abcdef");
        assert_eq!(elide_start("abcdef", 4), "…def");
        assert_eq!(elide_start("abcdef", 0), "");
    }
}
