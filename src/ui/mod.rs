//! The document viewport and the status bar. [`view`] is pure: it reads [`State`] and writes the
//! frame, touching only the rows on screen (ADR-05).

use std::ops::Range;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{Document, State};
use crate::layout::{Layout, Row, Span};
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
        let layout = &doc.layout;
        paint_rows(buf, doc_area, layout, state.scroll, &state.theme);
        if let Some(header) = sticky_header(layout, state.scroll, u32::from(doc_area.height)) {
            for (y, row) in (doc_area.y..).zip(header) {
                let line = Rect {
                    y,
                    height: 1,
                    ..doc_area
                };
                for x in line.left()..line.right() {
                    buf[(x, y)].reset();
                }
                buf.set_style(line, state.theme.style(StyleKey::Text));
                paint_row(buf, line, layout, row, &state.theme);
            }
        }
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
        paint_spans(buf, area.x, y, area.right(), layout, row, theme);
    }
}

/// Paint layout row `row` on the first line of `area`.
fn paint_row(buf: &mut Buffer, area: Rect, layout: &Layout, row: u32, theme: &Theme) {
    if let Some(row) = layout.rows.get(row as usize) {
        paint_spans(buf, area.x, area.y, area.right(), layout, row, theme);
    }
}

fn paint_spans(
    buf: &mut Buffer,
    mut x: u16,
    y: u16,
    right: u16,
    layout: &Layout,
    row: &Row,
    theme: &Theme,
) {
    for (text, span) in layout.spans(row) {
        if x >= right {
            break;
        }
        let room = usize::from(right - x);
        (x, _) = buf.set_stringn(x, y, text, room, span_style(theme, span));
    }
}

/// The header rows of the grid table straddling the top of a `height`-row viewport: its header
/// has scrolled off while body rows are still on screen below where the header will be drawn.
pub fn sticky_header(layout: &Layout, scroll: u32, height: u32) -> Option<Range<u32>> {
    let i = layout
        .grids
        .partition_point(|g| g.header.start < scroll)
        .checked_sub(1)?;
    let grid = &layout.grids[i];
    let len = grid.header.len() as u32;
    (scroll + len < grid.end && height > 2 * len).then(|| grid.header.clone())
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

    use crate::layout::{self, LayoutParams};

    #[test]
    fn sticky_header_while_the_body_is_on_screen() {
        let mut md = String::from("intro\n\n| a | b |\n|---|---|\n");
        for i in 0..20 {
            md.push_str(&format!("| {i} | x |\n"));
        }
        let doc = crate::doc::parse(&md);
        let params = LayoutParams {
            width: 40,
            max_width: None,
            table_mode: Default::default(),
            code_wrap: true,
            glyphs_gen: 0,
        };
        let l = layout::layout(&doc, &Theme::builtin().glyphs, params);
        // intro, blank, top border, header (row 3), separator, 20 body rows (5..25), bottom border.
        let g = &l.grids[0];
        assert_eq!((g.header.clone(), g.end), (3..5, 25));
        let sticky = |scroll| sticky_header(&l, scroll, 10);
        assert_eq!(sticky(0), None);
        assert_eq!(sticky(3), None, "the header is on screen");
        assert_eq!(sticky(4), Some(3..5));
        assert_eq!(sticky(22), Some(3..5), "one body row left under the header");
        assert_eq!(sticky(23), None);
        assert_eq!(sticky_header(&l, 10, 4), None, "the viewport is too short");
    }

    #[test]
    fn eliding() {
        assert_eq!(elide_start("abcdef", 6), "abcdef");
        assert_eq!(elide_start("abcdef", 4), "…def");
        assert_eq!(elide_start("abcdef", 0), "");
    }
}
