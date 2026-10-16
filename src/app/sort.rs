//! Sorting tables: `s` picks a column of the table on screen, and the same column again sorts
//! the other way, then puts the source order back. The order lives in the document's
//! [`Measure`](crate::layout::Measure), so it survives resizes, table-mode toggles, going back
//! and forth, and reloads that keep the table.

use super::{Cmd, Document, State};
use crate::doc::TableId;
use crate::layout::{LayoutParams, Sort};
use crate::theme::Theme;

impl State {
    /// The first table on screen with more than one body row to sort.
    pub fn table_on_screen(&self) -> Option<TableId> {
        let doc = self.doc.as_ref()?;
        let tables = doc.doc.tables();
        let (top, bottom) = (self.scroll, self.scroll + self.viewport_height());
        doc.layout
            .tables
            .iter()
            .filter(|t| t.rows.start < bottom && t.rows.end > top)
            .map(|t| t.table)
            .find(|&id| tables.get(id as usize).is_some_and(|t| t.rows.len() > 1))
    }

    /// What the column picker offers: each header with how the table is sorted by it, if it is.
    /// The picker reaches the first nine columns.
    pub fn sort_columns(&self) -> Vec<(String, Option<Sort>)> {
        let (Some(id), Some(doc)) = (self.sorting, &self.doc) else {
            return Vec::new();
        };
        let tables = doc.doc.tables();
        let Some(table) = tables.get(id as usize) else {
            return Vec::new();
        };
        let sort = doc.measure.sort(id);
        (0..table.column_count().min(9))
            .map(|c| {
                let sort = sort.filter(|s| usize::from(s.col) == c);
                (header_name(table, c), sort)
            })
            .collect()
    }

    pub(super) fn sort_open(&mut self) {
        match self.table_on_screen() {
            Some(id) => self.sorting = Some(id),
            None => self.status = Some("no table on screen to sort".into()),
        }
    }

    pub(super) fn sort_column(&mut self, col: u16) -> Vec<Cmd> {
        let Some(id) = self.sorting.take() else {
            return Vec::new();
        };
        let params = self.params();
        let Some(doc) = &mut self.doc else {
            return Vec::new();
        };
        let tables = doc.doc.tables();
        let Some(&table) = tables.get(id as usize) else {
            return Vec::new();
        };
        let sort = match col.checked_sub(1) {
            None => None,
            Some(c) if usize::from(c) >= table.column_count() => {
                let n = table.column_count();
                self.status = Some(format!("no column {col}: the table has {n}"));
                return Vec::new();
            }
            Some(c) => match doc.measure.sort(id) {
                Some(s) if s.col == c && !s.desc => Some(Sort { col: c, desc: true }),
                Some(s) if s.col == c => None,
                _ => Some(Sort {
                    col: c,
                    desc: false,
                }),
            },
        };
        self.status = Some(match sort {
            Some(s) => {
                let [asc, desc] = &self.theme.glyphs.sort_mark;
                let mark = if s.desc { desc } else { asc };
                let name = header_name(table, usize::from(s.col));
                format!("sorted by {name} {mark}")
            }
            None => "table in source order".into(),
        });
        doc.measure.set_sort(table, sort);
        // only this table's rows change, so the rows above it stay where they are
        doc.relayout_now(&self.theme, params);
        self.scroll = self.scroll.min(self.max_scroll());
        self.reproject();
        self.visible_jobs()
    }
}

impl Document {
    /// Sort a reloaded document's tables the way the old one's were, where they still have
    /// the column.
    pub(super) fn keep_sorts(&mut self, old: &Document, theme: &Theme, params: LayoutParams) {
        let mut kept = false;
        for table in self.doc.tables() {
            if let Some(sort) = old.measure.sort(table.id)
                && usize::from(sort.col) < table.column_count()
            {
                self.measure.set_sort(table, Some(sort));
                kept = true;
            }
        }
        if kept {
            self.relayout_now(theme, params);
        }
    }
}

/// A header cell's text, or `column N` for an empty one.
fn header_name(table: &crate::doc::Table, col: usize) -> String {
    let name = table
        .header
        .cells
        .get(col)
        .map(|c| c.plain_text().trim().to_owned())
        .unwrap_or_default();
    if name.is_empty() {
        format!("column {}", col + 1)
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Config, Msg, ScrollBy, Size, update};
    use super::*;
    use crate::layout::TableMode;

    const TABLE: &str =
        "| Name | Size |\n|---|--:|\n| beta | 10 |\n| alpha | 9 |\n| gamma | 200 |\n";

    fn loaded(text: &str) -> State {
        let mut state = State::new(Theme::builtin(), Config::default());
        update(
            &mut state,
            Msg::Init {
                size: Size { w: 60, h: 20 },
            },
        );
        load(&mut state, text);
        state
    }

    fn load(state: &mut State, text: &str) {
        update(
            state,
            Msg::DocLoaded {
                path: "t.md".into(),
                mtime: None,
                text: text.into(),
            },
        );
    }

    fn sort(state: &mut State, col: u16) {
        update(state, Msg::SortOpen);
        update(state, Msg::SortColumn { col });
    }

    /// The name on each body row, grid or card, in the order they render.
    fn names(state: &State) -> Vec<String> {
        let layout = &state.doc.as_ref().unwrap().layout;
        (0..layout.rows.len())
            .map(|r| layout.row_text(r))
            .filter_map(|t| {
                t.split(['│', ' '])
                    .find(|w| ["alpha", "beta", "gamma"].contains(w))
                    .map(str::to_owned)
            })
            .collect()
    }

    fn header(state: &State) -> String {
        let layout = &state.doc.as_ref().unwrap().layout;
        (0..layout.rows.len())
            .map(|r| layout.row_text(r))
            .find(|t| t.contains("Name"))
            .unwrap()
    }

    #[test]
    fn same_column_cycles_up_down_and_back() {
        let mut state = loaded(TABLE);
        assert_eq!(names(&state), ["beta", "alpha", "gamma"]);
        update(&mut state, Msg::SortOpen);
        assert_eq!(
            state.sort_columns(),
            [("Name".to_owned(), None), ("Size".to_owned(), None)]
        );
        update(&mut state, Msg::SortColumn { col: 2 });
        assert_eq!(state.sorting, None);
        assert_eq!(names(&state), ["alpha", "beta", "gamma"]);
        assert_eq!(state.status.as_deref(), Some("sorted by Size ▲"));
        assert!(header(&state).contains("Size ▲"), "{}", header(&state));

        sort(&mut state, 2);
        assert_eq!(names(&state), ["gamma", "beta", "alpha"]);
        assert!(header(&state).contains("Size ▼"));

        sort(&mut state, 2);
        assert_eq!(names(&state), ["beta", "alpha", "gamma"]);
        assert_eq!(state.status.as_deref(), Some("table in source order"));
        assert!(!header(&state).contains('▲') && !header(&state).contains('▼'));

        // another column starts ascending; 0 puts the source order back
        sort(&mut state, 2);
        sort(&mut state, 1);
        assert_eq!(names(&state), ["alpha", "beta", "gamma"]);
        sort(&mut state, 0);
        assert_eq!(names(&state), ["beta", "alpha", "gamma"]);
    }

    #[test]
    fn the_order_survives_toggles_resizes_and_reloads() {
        let mut state = loaded(TABLE);
        sort(&mut state, 2);
        sort(&mut state, 2);
        update(&mut state, Msg::ToggleTableMode);
        update(&mut state, Msg::ToggleTableMode);
        assert_eq!(state.table_mode, TableMode::Records);
        assert_eq!(names(&state), ["gamma", "beta", "alpha"]);
        update(
            &mut state,
            Msg::Resize {
                size: Size { w: 30, h: 20 },
            },
        );
        assert_eq!(names(&state), ["gamma", "beta", "alpha"]);
        load(&mut state, &format!("{TABLE}| alpha | 1000 |\n"));
        assert_eq!(names(&state), ["alpha", "gamma", "beta", "alpha"]);
        // a reload that loses the column loses the order
        load(&mut state, "| Name |\n|---|\n| beta |\n| alpha |\n");
        assert_eq!(names(&state), ["beta", "alpha"]);
    }

    #[test]
    fn the_picker_needs_a_table_and_closes_on_other_keys() {
        let mut state = loaded("# Title\n\nno tables here\n");
        update(&mut state, Msg::SortOpen);
        assert_eq!(state.sorting, None);
        assert_eq!(state.status.as_deref(), Some("no table on screen to sort"));

        let mut state = loaded(TABLE);
        update(&mut state, Msg::SortOpen);
        assert!(state.sorting.is_some());
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Rows(1),
            },
        );
        assert_eq!(state.sorting, None);
        update(&mut state, Msg::SortColumn { col: 1 });
        assert_eq!(
            names(&state),
            ["beta", "alpha", "gamma"],
            "no picker, no sort"
        );

        sort(&mut state, 3);
        assert_eq!(
            state.status.as_deref(),
            Some("no column 3: the table has 2")
        );
        assert_eq!(names(&state), ["beta", "alpha", "gamma"]);
    }

    #[test]
    fn picks_the_first_table_on_screen() {
        let filler = "words\n\n".repeat(30);
        let one_row = "| Lone |\n|---|\n| only |\n";
        let mut state = loaded(&format!("{one_row}\n{TABLE}\n{filler}{TABLE}"));
        // the first table has one row: nothing to sort
        assert_eq!(state.table_on_screen(), Some(1));
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Bottom,
            },
        );
        assert_eq!(state.table_on_screen(), Some(2));
    }
}
