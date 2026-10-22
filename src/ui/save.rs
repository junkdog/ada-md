//! The save box (`S`): the file it saves to, one row per option saving changes, `key  from →
//! to`, and the answers it takes. Static, like the help: there is nothing to pick.

use crate::geom::Rect;
use crate::grid::Grid;
use unicode_width::UnicodeWidthStr;

use super::{elide_start, popup_frame};
use crate::app::{Saving, State};
use crate::theme::StyleKey;

const FOOTER: &str = "Enter save · Esc cancel";

/// The rows: the options, then a blank one and the footer.
fn lines(saving: &Saving) -> Vec<String> {
    let changes = &saving.save.changes;
    let keys = changes.iter().map(|(key, ..)| key.len()).max().unwrap_or(0);
    let mut lines: Vec<String> = changes
        .iter()
        .map(|(key, from, to)| format!(" {key:<keys$}  {from} → {to} "))
        .collect();
    lines.push(String::new());
    lines.push(FOOTER.to_owned());
    lines
}

pub fn draw(buf: &mut Grid, area: Rect, state: &State) {
    let Some(saving) = &state.save else {
        return;
    };
    let theme = &state.theme;
    let title = format!(" save to {} ", saving.path.display());
    let lines = lines(saving);
    let widest = lines
        .iter()
        .map(|l| l.width())
        .chain([title.width() + 2])
        .max()
        .unwrap_or(0);
    let width = (widest as u16 + 2).min(area.width.saturating_sub(2));
    let height = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
    if width < 8 || height < 3 {
        return;
    }
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + 1,
        width,
        height,
    };
    let border = popup_frame(buf, rect, theme);
    let text = theme.style(StyleKey::Popup);
    let inner = usize::from(rect.width - 2);
    let (top, bottom) = (rect.top(), rect.bottom() - 1);
    let title_style = theme.style(StyleKey::PopupTitle);
    // a long path loses its start, and the title keeps saying what it is
    let path = saving.path.display().to_string();
    let room = inner.saturating_sub(2 + " save to  ".width());
    let title = format!(" save to {} ", elide_start(&path, room));
    buf.set_stringn(rect.x + 2, top, &title, inner - 1, title_style);

    // the footer stays when the rows don't fit
    let rows = usize::from(bottom - top - 1);
    let shown = lines.len().min(rows);
    let (options, footer) = lines.split_at(lines.len() - 2);
    for (y, line) in (top + 1..).zip(options.iter().take(shown.saturating_sub(2))) {
        buf.set_stringn(rect.x + 1, y, line, inner, text);
    }
    let pad = inner.saturating_sub(FOOTER.width()) / 2;
    let y = bottom - 1;
    buf.set_stringn(rect.x + 1 + pad as u16, y, &footer[1], inner, border);
}
