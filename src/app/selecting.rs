//! Selecting text with the mouse (ADR-38, plan §4): a drag, or a double or triple press, selects
//! in the document ([`crate::select`]), and the selection copies through OSC 52 on `y`, or as it's
//! made ([`CopyOn`]).

use super::{Cmd, ScrollBy, State};
use crate::select::{Copied, CopyOn, MAX_COPY, Pos, Selection, TableFormat, cut_copy};

/// The press a drag selects from, and the screen cell the pointer was last at, to select to again
/// when the document scrolls under it.
#[derive(Clone, Copy, Debug)]
pub(super) struct Drag {
    anchor: Pos,
    at: (u16, u16),
}

impl State {
    pub(super) fn clear_selection(&mut self) {
        self.selection = None;
        self.drag = None;
    }

    /// Whether the mouse selects now: not over a popup, the prompt or the image view, nor while
    /// a navigation loads (the screen is about to change under it).
    fn may_select(&self) -> bool {
        let popup = self.outline.is_some() || self.help.is_some() || self.save.is_some();
        self.nav.is_none() && !popup && self.prompt.is_none() && self.view.is_none()
    }

    /// The layout row and column the screen's cell `col`, `row` shows, as
    /// [`link_at`](Self::link_at) finds them, but kept to the document: the status bar counts as
    /// the last row on screen, and the margin left of the rows as their first column.
    fn layout_cell(&self, col: u16, row: u16) -> Option<(u32, u16)> {
        let doc = self.doc.as_ref()?;
        let (layout, height) = (&doc.layout, self.viewport_height());
        let last = layout.row_count().checked_sub(1)?;
        let row = u32::from(row).min(height.checked_sub(1)?);
        let r = match crate::ui::sticky_header(layout, self.scroll, height) {
            Some(header) if row < header.end - header.start => header.start + row,
            _ => self.scroll + row,
        };
        let col = col.saturating_sub(layout.numbers.min(self.size.w));
        Some((r.min(last), col))
    }

    /// A press: clears the selection, and starts a new one from the text under it; the second
    /// and third in quick succession select the word, or the whole unit.
    pub(super) fn press(&mut self, col: u16, row: u16, count: u8) {
        self.clear_selection();
        if !self.may_select() {
            return;
        }
        let Some((r, c)) = self.layout_cell(col, row) else {
            return;
        };
        let Some(doc) = self.doc.as_mut() else {
            return;
        };
        let view = doc.select_view(&self.theme, self.code_x);
        match count {
            1 => {
                let anchor = view.place(r, c, false);
                self.drag = anchor.map(|anchor| Drag {
                    anchor,
                    at: (col, row),
                });
            }
            2 => self.selection = view.word(r, c),
            _ => self.selection = view.unit(r, c),
        }
    }

    /// A drag: select from the press to the text under the pointer. On the top or bottom row,
    /// the document scrolls a row first, so a drag can reach past what's on screen.
    pub(super) fn drag_to(&mut self, col: u16, row: u16) -> Vec<Cmd> {
        let Some(drag) = &mut self.drag else {
            return Vec::new();
        };
        drag.at = (col, row);
        if !self.may_select() {
            return Vec::new();
        }
        let height = self.viewport_height();
        let by = if row == 0 && self.scroll > 0 {
            -1
        } else if u32::from(row) + 1 >= height && self.scroll < self.max_scroll() {
            1
        } else {
            0
        };
        if by != 0 {
            self.scroll_by(ScrollBy::Rows(by));
        }
        self.select_to(col, row);
        if by != 0 {
            self.visible_jobs()
        } else {
            Vec::new()
        }
    }

    fn select_to(&mut self, col: u16, row: u16) {
        let Some(drag) = self.drag else {
            return;
        };
        let Some((r, c)) = self.layout_cell(col, row) else {
            return;
        };
        let Some(doc) = self.doc.as_mut() else {
            return;
        };
        let head = doc
            .select_view(&self.theme, self.code_x)
            .head(drag.anchor, r, c);
        self.selection = head.map(|head| Selection {
            anchor: drag.anchor,
            head,
        });
    }

    /// The document scrolled while the button is down, by the wheel or sideways: select to
    /// what's under the pointer now.
    pub(super) fn follow_drag(&mut self) {
        if let Some(drag) = self.drag
            && self.selection.is_some()
        {
            self.select_to(drag.at.0, drag.at.1);
        }
    }

    /// The button let go after a drag or a double or triple press: copy what's selected, or say
    /// how to.
    pub(super) fn release(&mut self) -> Vec<Cmd> {
        self.drag = None;
        if self.selection.is_none() || !self.may_select() {
            return Vec::new();
        }
        match self.config.copy_on {
            CopyOn::Select => self.copy_selection().unwrap_or_default(),
            CopyOn::Yank => {
                self.status = Some("y copies, Esc clears".into());
                Vec::new()
            }
        }
    }

    /// `Esc`: clear the selection, or quit when there's none.
    pub(super) fn cancel(&mut self) -> Vec<Cmd> {
        if self.selection.is_some() || self.drag.is_some() {
            self.clear_selection();
            self.status = None;
            return Vec::new();
        }
        self.quit = true;
        vec![Cmd::Quit]
    }

    /// Copy the selection and say what went; `None` when nothing is selected.
    pub(super) fn copy_selection(&mut self) -> Option<Vec<Cmd>> {
        let sel = self.selection?;
        let doc = self.doc.as_mut()?;
        let copied = doc.select_view(&self.theme, self.code_x).copy(&sel);
        if copied.text.is_empty() {
            self.status = Some("nothing selected to copy".into());
            return Some(Vec::new());
        }
        self.status = Some(describe(&copied));
        let (text, _) = cut_copy(copied.text);
        Some(vec![Cmd::Osc52(text)])
    }
}

/// The status line's word on a copy: what went, and how much, and when it was cut.
fn describe(copied: &Copied) -> String {
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    let text = &copied.text;
    let what = match copied.table {
        Some((rows, cols, TableFormat::Tsv)) => format!("{rows}×{cols} cells as TSV"),
        Some((rows, _, TableFormat::Markdown)) => {
            format!("a markdown table, {}", plural(rows - 1, "row"))
        }
        Some((rows, _, TableFormat::Records)) => plural(rows, "record"),
        None => {
            let (lines, chars) = (text.lines().count().max(1), text.chars().count());
            match lines {
                1 if chars <= 40 => format!("\"{text}\""),
                1 => plural(chars, "character"),
                _ => format!("{}, {}", plural(lines, "line"), plural(chars, "character")),
            }
        }
    };
    if text.len() > MAX_COPY {
        let kb = |n: usize| n.div_ceil(1024);
        format!(
            "copied {what}, cut to {} KB of {} KB",
            kb(MAX_COPY),
            kb(text.len())
        )
    } else {
        format!("copied {what}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Computed, Config, Msg, Size, update};
    use crate::theme::Theme;

    fn loaded(text: &str, config: Config, w: u16, h: u16) -> State {
        let mut state = State::new(Theme::builtin(), config);
        update(
            &mut state,
            Msg::Init {
                size: Size { w, h },
            },
        );
        update(
            &mut state,
            Msg::DocLoaded {
                path: "t.md".into(),
                mtime: None,
                text: text.into(),
                doc: Computed(None),
            },
        );
        state
    }

    /// The screen cell of the first character of `needle`, or of its last.
    fn cell(state: &State, needle: &str, last: bool) -> (u16, u16) {
        let layout = &state.doc.as_ref().unwrap().layout;
        for r in state.scroll..layout.row_count() {
            let text = layout.row_text(r as usize);
            if let Some(i) = text.find(needle) {
                let at = if last {
                    i + needle.char_indices().last().map_or(0, |(j, _)| j)
                } else {
                    i
                };
                let col = unicode_width::UnicodeWidthStr::width(&text[..at]) as u16;
                return (layout.numbers + col, (r - state.scroll) as u16);
            }
        }
        panic!("{needle:?} isn't on screen");
    }

    fn drag(state: &mut State, from: &str, to: &str) -> Vec<Cmd> {
        let (a, b) = (cell(state, from, false), cell(state, to, true));
        update(
            state,
            Msg::Press {
                col: a.0,
                row: a.1,
                count: 1,
            },
        );
        update(state, Msg::Drag { col: b.0, row: b.1 });
        update(state, Msg::Release)
    }

    fn copied(cmds: &[Cmd]) -> Option<&str> {
        cmds.iter().find_map(|c| match c {
            Cmd::Osc52(text) => Some(text.as_str()),
            _ => None,
        })
    }

    const DOC: &str =
        "# Title\n\nalpha beta gamma delta epsilon\n\n```\nlet x = 1;\nlet y = 2;\n```\n";

    #[test]
    fn y_copies_the_selection_and_clears_it() {
        let mut state = loaded(DOC, Config::default(), 40, 12);
        assert!(
            drag(&mut state, "beta", "delta").is_empty(),
            "nothing copied yet"
        );
        assert!(state.selection.is_some());
        assert_eq!(state.status.as_deref(), Some("y copies, Esc clears"));
        let cmds = update(&mut state, Msg::Yank);
        assert_eq!(copied(&cmds), Some("beta gamma delta"));
        assert_eq!(state.status.as_deref(), Some("copied \"beta gamma delta\""));
        assert!(state.selection.is_none());
        // without a selection, `y` copies the code block on screen, as before
        let cmds = update(&mut state, Msg::Yank);
        assert_eq!(copied(&cmds), Some("let x = 1;\nlet y = 2;\n"));
    }

    #[test]
    fn with_copy_on_select_a_drag_copies_as_it_ends() {
        let config = Config {
            copy_on: CopyOn::Select,
            ..Config::default()
        };
        let mut state = loaded(DOC, config, 40, 12);
        let cmds = drag(&mut state, "let x", "2;");
        assert_eq!(copied(&cmds), Some("let x = 1;\nlet y = 2;"));
        assert_eq!(
            state.status.as_deref(),
            Some("copied 2 lines, 21 characters")
        );
        assert!(state.selection.is_some(), "it stays on screen");
        // a double and a triple press, released
        let (col, row) = cell(&state, "gamma", false);
        update(&mut state, Msg::Press { col, row, count: 2 });
        assert_eq!(copied(&update(&mut state, Msg::Release)), Some("gamma"));
        update(&mut state, Msg::Press { col, row, count: 3 });
        assert_eq!(
            copied(&update(&mut state, Msg::Release)),
            Some("alpha beta gamma delta epsilon")
        );
        // a press alone clears it
        update(&mut state, Msg::Press { col, row, count: 1 });
        assert!(state.selection.is_none());
    }

    #[test]
    fn esc_clears_the_selection_before_it_quits() {
        let mut state = loaded(DOC, Config::default(), 40, 12);
        drag(&mut state, "beta", "delta");
        assert!(update(&mut state, Msg::Cancel).is_empty());
        assert!(state.selection.is_none() && !state.quit);
        assert_eq!(update(&mut state, Msg::Cancel), [Cmd::Quit]);
        assert!(state.quit);
    }

    #[test]
    fn line_numbers_are_left_of_the_text() {
        let mut state = loaded(DOC, Config::default(), 40, 12);
        update(&mut state, Msg::ToggleLineNumbers);
        assert!(state.doc.as_ref().unwrap().layout.numbers > 0);
        drag(&mut state, "let x", "2;");
        // a drag from over the numbers takes the line from its start
        let (_, row) = cell(&state, "let y", false);
        update(
            &mut state,
            Msg::Press {
                col: 0,
                row,
                count: 1,
            },
        );
        let (col, _) = cell(&state, "2;", true);
        update(&mut state, Msg::Drag { col, row });
        update(&mut state, Msg::Release);
        assert_eq!(copied(&update(&mut state, Msg::Yank)), Some("let y = 2;"));
    }

    #[test]
    fn a_drag_at_the_edge_scrolls_and_the_wheel_carries_it_on() {
        let md: String = (1..=40).map(|i| format!("line {i}\n\n")).collect();
        let mut state = loaded(&md, Config::default(), 40, 10);
        let (col, row) = cell(&state, "line 1", false);
        update(&mut state, Msg::Press { col, row, count: 1 });
        // the bottom row of the document: a row further each time
        for _ in 0..4 {
            update(&mut state, Msg::Drag { col: 20, row: 8 });
        }
        assert_eq!(state.scroll, 4);
        // the wheel scrolls the document under the pointer, which still holds the button
        update(
            &mut state,
            Msg::Scroll {
                by: crate::app::ScrollBy::Rows(6),
            },
        );
        update(&mut state, Msg::Release);
        let text = copied(&update(&mut state, Msg::Yank)).unwrap().to_owned();
        assert!(text.starts_with("line 1\n\nline 2"), "{text}");
        assert!(text.ends_with("line 10"), "{text}");
    }

    #[test]
    fn popups_and_the_prompt_take_no_selection() {
        let mut state = loaded(DOC, Config::default(), 40, 12);
        update(&mut state, Msg::OpenOutline);
        drag(&mut state, "beta", "delta");
        assert!(state.selection.is_none());
        update(&mut state, Msg::OutlineClose);
        // a new document, or `M`, drops one
        drag(&mut state, "beta", "delta");
        assert!(state.selection.is_some());
        update(&mut state, Msg::ToggleMouse);
        assert!(state.selection.is_none());
    }
}
