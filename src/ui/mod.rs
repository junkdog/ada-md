//! The document viewport and the status bar. [`view`] is pure: it reads [`State`] and writes the
//! frame, touching only the rows on screen (ADR-05).

mod outline;
mod paint;

use std::ops::Range;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{Document, State};
use crate::layout::{Layout, RowKind};
use crate::theme::StyleKey;

pub use paint::{Painter, span_style};

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
        let painter = Painter {
            theme: &state.theme,
            layout,
            highlights: &state.highlights,
            code_x: state.code_x,
            links: &doc.links,
        };
        let lines = screen_rows(layout, state.scroll, doc_area);
        for &(y, r) in &lines {
            let Some(row) = layout.rows.get(r as usize) else {
                continue;
            };
            let line = Rect {
                y,
                height: 1,
                ..doc_area
            };
            for x in line.left()..line.right() {
                buf[(x, y)].reset();
            }
            buf.set_style(line, state.theme.style(StyleKey::Text));
            painter.row_into(buf, line.x, y, line.right(), row);
        }
        overlays(buf, doc_area, state, doc, &lines);
    }
    if state.outline.is_some() {
        outline::draw(buf, doc_area, state);
    }
    status_bar(buf, status_area, state);
}

/// Which layout row each screen line of `area` shows: the rows from `scroll`, with the sticky
/// header of a table straddling the top drawn over the first lines.
fn screen_rows(layout: &Layout, scroll: u32, area: Rect) -> Vec<(u16, u32)> {
    let mut lines: Vec<(u16, u32)> = (area.y..area.bottom())
        .zip(scroll..layout.row_count())
        .collect();
    if let Some(header) = sticky_header(layout, scroll, u32::from(area.height)) {
        for (line, r) in lines.iter_mut().zip(header) {
            line.1 = r;
        }
    }
    lines
}

/// Search matches and the focused link, patched over the painted rows (ADR-03).
fn overlays(buf: &mut Buffer, area: Rect, state: &State, doc: &Document, lines: &[(u16, u32)]) {
    let theme = &state.theme;
    let layout = &doc.layout;
    let patch = |buf: &mut Buffer, y: u16, cols: Range<u16>, key: StyleKey| {
        let x = area.x.saturating_add(cols.start);
        let right = area.x.saturating_add(cols.end).min(area.right());
        if x < right {
            let rect = Rect::new(x, y, right - x, 1);
            buf.set_style(rect, theme.styles[key as usize]);
        }
    };
    if let Some(search) = &state.search {
        for &(y, r) in lines {
            for cells in search.projection.on_rows(r..r + 1) {
                let key = if Some(cells.at) == search.current {
                    StyleKey::SearchCurrent
                } else if cells.hidden {
                    StyleKey::SearchMatchHidden
                } else {
                    StyleKey::SearchMatch
                };
                let cols = match (cells.window, layout.rows[r as usize].kind) {
                    (Some(start), RowKind::Code { block, .. }) => {
                        let info = layout.code.get(block as usize).copied().unwrap_or_default();
                        let x = state.code_x.min(info.max_x());
                        let end = start + info.window;
                        let shift = |c: u16| c.saturating_sub(x).clamp(start, end);
                        shift(cells.cols.start)..shift(cells.cols.end)
                    }
                    _ => cells.cols.clone(),
                };
                patch(buf, y, cols, key);
            }
        }
    }
    if let Some(focus) = state.visible_focus() {
        for &(y, r) in lines {
            for hit in layout.hits.iter().filter(|h| h.row == r && h.link == focus) {
                patch(buf, y, hit.cols.clone(), StyleKey::LinkFocused);
            }
        }
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

/// Styled pieces of text, written left to right.
type Pieces = Vec<(String, Style)>;

/// Write `pieces` from `x`, within `max` columns. Returns where they end.
fn put(buf: &mut Buffer, x: u16, y: u16, max: usize, pieces: &Pieces) -> u16 {
    let mut x = x;
    let end = x.saturating_add(max as u16);
    for (text, style) in pieces {
        if x >= end {
            break;
        }
        (x, _) = buf.set_stringn(x, y, text, usize::from(end - x), *style);
    }
    x
}

fn status_bar(buf: &mut Buffer, area: Rect, state: &State) {
    let theme = &state.theme;
    buf.set_style(area, theme.style(StyleKey::StatusBar));
    let width = usize::from(area.width);

    let mut right = String::new();
    if let Some(search) = &state.search {
        right.push_str(&format!(" {} ", search.counter()));
    }
    let position = state
        .doc
        .as_ref()
        .map_or_else(String::new, |_| position(state));
    let pos_text = format!(" {position} ");
    let pos_width = (right.width() + pos_text.width()).min(width) as u16;
    let pos_x = area.right().saturating_sub(pos_width);
    put(
        buf,
        pos_x,
        area.y,
        usize::from(pos_width),
        &vec![
            (right, theme.style(StyleKey::StatusMessage)),
            (pos_text, theme.style(StyleKey::StatusPos)),
        ],
    );

    if let Some(prompt) = &state.prompt {
        let room = usize::from(pos_x - area.x);
        let mut pieces = vec![(format!("/{}", prompt.input), theme.style(StyleKey::Prompt))];
        pieces.push(("▏".to_owned(), theme.style(StyleKey::Prompt)));
        if let Some(e) = &prompt.error {
            pieces.push((format!("  {e}"), theme.style(StyleKey::PromptError)));
        }
        put(buf, area.x, area.y, room, &pieces);
        return;
    }

    let name = match &state.doc {
        Some(doc) => display_name(doc),
        None => String::new(),
    };
    let name = format!(" {} ", elide_start(&name, width / 2));
    let (x, _) = buf.set_stringn(
        area.x,
        area.y,
        &name,
        usize::from(pos_x - area.x),
        theme.style(StyleKey::StatusFile),
    );
    let room = usize::from(pos_x.saturating_sub(x)).saturating_sub(2);
    if room > 0 {
        put(buf, x + 1, area.y, room, &status_text(state, room));
    }
}

/// The middle of the status line: a message, a match in hidden text, the focused link, or the
/// heading path, in that order of precedence.
fn status_text(state: &State, room: usize) -> Pieces {
    let theme = &state.theme;
    if let Some(msg) = &state.status {
        return vec![(elide_start(msg, room), theme.style(StyleKey::StatusMessage))];
    }
    if let Some((kind, text, range)) = state.hidden_match() {
        let label = format!("{kind} → ");
        let base = theme.style(StyleKey::StatusLink);
        let hit = base.patch(theme.styles[StyleKey::SearchCurrent as usize]);
        let [before, matched, after] =
            elide_around(text, range, room.saturating_sub(label.width()));
        return vec![(label, base), (before, base), (matched, hit), (after, base)];
    }
    if let Some(target) = state.focus_text() {
        let text = format!("→ {target}");
        return vec![(elide_start(&text, room), theme.style(StyleKey::StatusLink))];
    }
    match &state.doc {
        Some(doc) => {
            let path = heading_path(doc, state.scroll).join(" › ");
            vec![(
                elide_start(&path, room),
                theme.style(StyleKey::StatusHeading),
            )]
        }
        None => Vec::new(),
    }
}

/// `text` split around `range` and elided to `max` columns, keeping the match in view.
fn elide_around(text: &str, range: Range<usize>, max: usize) -> [String; 3] {
    let (Some(before), Some(matched), Some(after)) = (
        text.get(..range.start),
        text.get(range.clone()),
        text.get(range.end..),
    ) else {
        return [elide_start(text, max), String::new(), String::new()];
    };
    if text.width() <= max {
        return [before.to_owned(), matched.to_owned(), after.to_owned()];
    }
    let matched = elide_end(matched, max);
    let left = max.saturating_sub(matched.width());
    let before_room = (left / 2).max(left.saturating_sub(after.width()));
    let before = elide_start(before, before_room);
    let after = elide_end(after, left.saturating_sub(before.width()));
    [before, matched, after]
}

/// Keep the start of `s` within `max` columns.
fn elide_end(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_owned();
    }
    if max == 0 {
        return String::new();
    }
    let mut used = 1;
    let mut out = String::new();
    for c in s.chars() {
        used += c.width().unwrap_or(0);
        if used > max {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
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
    use crate::theme::Theme;

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
        assert_eq!(elide_end("abcdef", 4), "abc…");
        let around = |max| elide_around("../30-resources/ratatui/README.md", 24..30, max).join("|");
        assert_eq!(around(80), "../30-resources/ratatui/|README|.md");
        assert_eq!(around(16), "…tatui/|README|.md");
        assert_eq!(around(4), "|REA…|");
    }
}
