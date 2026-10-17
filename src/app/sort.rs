//! Sorting tables: `s` lists the columns of the first table on screen, each ascending and
//! descending, `←`/`→` list another table's, and picking one sorts the table so. The order lives
//! in the document's
//! [`Measure`](crate::layout::Measure), so it survives resizes, table-mode toggles, going back
//! and forth, and reloads that keep the table.

use std::borrow::Cow;

use unicode_width::UnicodeWidthStr;

use super::{Cmd, Dir, Document, Listing, OutlineEntry, State};
use crate::doc::TableId;
use crate::layout::{LayoutParams, Sort};
use crate::theme::Glyphs;

impl State {
    /// The tables on screen with more than one body row to sort, in row order.
    pub fn tables_on_screen(&self) -> Vec<TableId> {
        let Some(doc) = self.doc.as_ref() else {
            return Vec::new();
        };
        let tables = doc.doc.tables();
        let (top, bottom) = (self.scroll, self.scroll + self.viewport_height());
        doc.layout
            .tables
            .iter()
            .filter(|t| t.rows.start < bottom && t.rows.end > top)
            .map(|t| t.table)
            .filter(|&id| tables.get(id as usize).is_some_and(|t| t.rows.len() > 1))
            .collect()
    }

    /// The table the column list is for, while it is open.
    pub fn sorting(&self) -> Option<TableId> {
        match self.outline.as_ref()?.listing {
            Listing::Columns(id) => id,
            _ => None,
        }
    }

    /// What the column list offers: each header ascending, then descending, the way the table
    /// is sorted marked, and the source order once it is sorted.
    pub(super) fn column_entries(&self, id: TableId) -> Vec<OutlineEntry<'_>> {
        let Some(doc) = &self.doc else {
            return Vec::new();
        };
        let tables = doc.doc.tables();
        let Some(table) = tables.get(id as usize) else {
            return Vec::new();
        };
        let current = doc.measure.sort(id);
        let marks = &self.theme.glyphs.sort_mark;
        let entry = |id: u32, text: String, dest: Option<String>| OutlineEntry {
            id,
            level: 0,
            row: 0,
            text: Cow::Owned(text),
            dest,
            broken: false,
        };
        let names: Vec<String> = (0..table.column_count())
            .map(|c| header_name(table, c))
            .collect();
        // padded, so the directions line up
        let width = names.iter().map(|n| n.width()).max().unwrap_or(0);
        let mut entries = Vec::new();
        for (c, name) in names.iter().enumerate() {
            let Ok(col) = u16::try_from(c) else {
                break;
            };
            for desc in [false, true] {
                let sort = Sort { col, desc };
                let mark = &marks[usize::from(desc)];
                let way = if desc { "descending" } else { "ascending" };
                let mut dest = if mark.is_empty() {
                    way.to_owned()
                } else {
                    format!("{mark} {way}")
                };
                if current == Some(sort) {
                    dest.push_str(" · current");
                }
                let pad = width - name.width();
                entries.push(entry(
                    entry_id(sort),
                    format!("{name}{:pad$}", ""),
                    Some(dest),
                ));
            }
        }
        if current.is_some() {
            entries.push(entry(0, "source order".into(), None));
        }
        entries
    }

    /// The entry the column list opens on: the other way round from how the table is sorted,
    /// so `Enter` turns it around.
    pub(super) fn column_to_open(&self, id: TableId) -> Option<u32> {
        let sort = self.doc.as_ref()?.measure.sort(id)?;
        Some(entry_id(Sort {
            desc: !sort.desc,
            ..sort
        }))
    }

    pub(super) fn sort_open(&mut self) -> Vec<Cmd> {
        let first = self.tables_on_screen().first().copied();
        self.open_outline(Listing::Columns(first))
    }

    /// List the columns of the next or previous table on screen, wrapping around.
    pub(super) fn sort_table(&mut self, dir: Dir) {
        let Some(current) = self.sorting() else {
            return;
        };
        let tables = self.tables_on_screen();
        let n = tables.len();
        if n == 0 {
            return;
        }
        let next = match (tables.iter().position(|&t| t == current), dir) {
            (Some(i), Dir::Next) => (i + 1) % n,
            (Some(i), Dir::Prev) => (i + n - 1) % n,
            (None, _) => 0,
        };
        if tables[next] != current {
            self.open_outline(Listing::Columns(Some(tables[next])));
        }
    }

    /// Sort table `id` the way column list entry `entry` says; 0 puts the source order back.
    pub(super) fn sort_column(&mut self, id: TableId, entry: u32) -> Vec<Cmd> {
        let params = self.params();
        let Some(doc) = &mut self.doc else {
            return Vec::new();
        };
        let tables = doc.doc.tables();
        let Some(&table) = tables.get(id as usize) else {
            return Vec::new();
        };
        let sort = match entry_sort(entry) {
            Some(Some(s)) if usize::from(s.col) < table.column_count() => Some(s),
            Some(None) => None,
            _ => return Vec::new(),
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
        doc.relayout_now(&self.theme.glyphs, params);
        self.scroll = self.scroll.min(self.max_scroll());
        self.reproject();
        self.visible_jobs()
    }
}

impl Document {
    /// Sort a reloaded document's tables the way the old one's were, where they still have
    /// the column.
    pub(super) fn keep_sorts(&mut self, old: &Document, glyphs: &Glyphs, params: LayoutParams) {
        if self.set_sorts(|t| old.measure.sort(t)) {
            self.relayout_now(glyphs, params);
        }
    }

    /// Sort each table the way `sort_of` says, where it still has the column; whether any
    /// changed. The layout is left to the caller.
    pub(super) fn set_sorts(&mut self, sort_of: impl Fn(TableId) -> Option<Sort>) -> bool {
        let mut changed = false;
        for table in self.doc.tables() {
            let sort = sort_of(table.id).filter(|s| usize::from(s.col) < table.column_count());
            if sort != self.measure.sort(table.id) {
                self.measure.set_sort(table, sort);
                changed = true;
            }
        }
        changed
    }

    /// How each table is sorted, by table.
    pub(super) fn sorts(&self) -> Vec<Option<Sort>> {
        (0..self.doc.table_count)
            .map(|t| self.measure.sort(t))
            .collect()
    }
}

/// The column list's entry for sorting by `sort`; 0 is the source order.
fn entry_id(sort: Sort) -> u32 {
    2 * u32::from(sort.col) + 1 + u32::from(sort.desc)
}

/// How a column list entry sorts: `Some(None)` for the source order, `None` for no entry.
fn entry_sort(entry: u32) -> Option<Option<Sort>> {
    let Some(i) = entry.checked_sub(1) else {
        return Some(None);
    };
    let col = u16::try_from(i / 2).ok()?;
    Some(Some(Sort {
        col,
        desc: i % 2 == 1,
    }))
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
    use super::super::{Computed, Config, Msg, ScrollBy, Size, loader_answer, update};
    use super::*;
    use crate::layout::TableMode;
    use crate::theme::Theme;

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
                doc: Computed(None),
            },
        );
    }

    /// Open the column list and pick the first entry `filter` leaves, e.g. `size desc`.
    fn sort(state: &mut State, filter: &str) {
        update(state, Msg::SortOpen);
        update(
            state,
            Msg::OutlineFilter {
                text: filter.into(),
            },
        );
        update(state, Msg::OutlinePick);
    }

    /// The column list's entries, as `name  direction`.
    fn columns(state: &State) -> Vec<String> {
        state.outline_entries().iter().map(entry_text).collect()
    }

    fn entry_text(e: &OutlineEntry) -> String {
        match &e.dest {
            Some(dest) => format!("{}  {dest}", e.text),
            None => e.text.to_string(),
        }
    }

    fn selected(state: &State) -> String {
        let entries = state.outline_entries();
        entry_text(&entries[state.outline.as_ref().unwrap().selected])
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
    fn each_column_sorts_either_way_in_one_pick() {
        let mut state = loaded(TABLE);
        assert_eq!(names(&state), ["beta", "alpha", "gamma"]);
        update(&mut state, Msg::SortOpen);
        assert_eq!(state.sorting(), Some(0));
        assert_eq!(
            columns(&state),
            [
                "Name  ▲ ascending",
                "Name  ▼ descending",
                "Size  ▲ ascending",
                "Size  ▼ descending",
            ]
        );
        assert_eq!(selected(&state), "Name  ▲ ascending");
        update(&mut state, Msg::OutlineMove { by: 3 });
        update(&mut state, Msg::OutlinePick);
        assert_eq!(state.outline, None);
        assert_eq!(names(&state), ["gamma", "beta", "alpha"]);
        assert_eq!(state.status.as_deref(), Some("sorted by Size ▼"));
        assert!(header(&state).contains("Size ▼"), "{}", header(&state));

        // the list marks the current order and opens on its reverse, so `Enter` turns it around
        update(&mut state, Msg::SortOpen);
        assert_eq!(
            columns(&state)[2..],
            [
                "Size  ▲ ascending",
                "Size  ▼ descending · current",
                "source order",
            ]
        );
        assert_eq!(selected(&state), "Size  ▲ ascending");
        update(&mut state, Msg::OutlinePick);
        assert_eq!(names(&state), ["alpha", "beta", "gamma"]);
        assert!(header(&state).contains("Size ▲"));

        // picking the current order keeps it
        sort(&mut state, "size asc");
        assert_eq!(names(&state), ["alpha", "beta", "gamma"]);

        sort(&mut state, "name desc");
        assert_eq!(names(&state), ["gamma", "beta", "alpha"]);
        sort(&mut state, "source");
        assert_eq!(names(&state), ["beta", "alpha", "gamma"]);
        assert_eq!(state.status.as_deref(), Some("table in source order"));
        assert!(!header(&state).contains('▲') && !header(&state).contains('▼'));
    }

    #[test]
    fn the_order_survives_toggles_resizes_and_reloads() {
        let mut state = loaded(TABLE);
        sort(&mut state, "size desc");
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
    fn a_reload_the_loader_prepares_keeps_the_order_the_table_has_when_it_arrives() {
        let mut state = loaded(TABLE);
        sort(&mut state, "size desc");
        let cmds = update(&mut state, Msg::Reload);
        let grown = format!("{TABLE}| alpha | 1000 |\n");
        update(&mut state, loader_answer(&cmds, &grown));
        assert_eq!(names(&state), ["alpha", "gamma", "beta", "alpha"]);
        // sorted the other way while it loaded
        let cmds = update(&mut state, Msg::Reload);
        sort(&mut state, "size asc");
        update(
            &mut state,
            loader_answer(&cmds, &format!("{grown}| beta | 3 |\n")),
        );
        assert_eq!(names(&state), ["beta", "alpha", "beta", "gamma", "alpha"]);
    }

    #[test]
    fn the_list_needs_a_table_and_closes_without_sorting() {
        let mut state = loaded("# Title\n\nno tables here\n");
        update(&mut state, Msg::SortOpen);
        assert!(
            state.outline.is_some(),
            "open, saying there's nothing to sort"
        );
        assert_eq!(state.sorting(), None);
        assert!(columns(&state).is_empty());
        update(&mut state, Msg::OutlinePick);
        assert_eq!(state.outline, None);

        let mut state = loaded(TABLE);
        update(&mut state, Msg::SortOpen);
        update(&mut state, Msg::OutlineClose);
        assert_eq!(state.outline, None);
        assert_eq!(names(&state), ["beta", "alpha", "gamma"]);

        // a filter matching no column picks nothing
        sort(&mut state, "nope");
        assert_eq!(names(&state), ["beta", "alpha", "gamma"]);
    }

    #[test]
    fn every_column_is_listed_and_empty_headers_get_a_name() {
        let header = (1..=12).map(|i| format!("c{i}")).collect::<Vec<_>>();
        let table = format!(
            "| {} | |\n|{}\n| a |\n| b |\n",
            header.join(" | "),
            "---|".repeat(13)
        );
        let mut state = loaded(&table);
        update(&mut state, Msg::SortOpen);
        let listed = columns(&state);
        assert_eq!(listed.len(), 26);
        assert_eq!(listed[22], "c12        ▲ ascending");
        assert_eq!(listed[25], "column 13  ▼ descending");
    }

    #[test]
    fn left_and_right_trade_for_another_table_on_screen() {
        let filler = "words\n\n".repeat(30);
        let one_row = "| Lone |\n|---|\n| only |\n";
        let other = "| Pet | Legs |\n|---|--:|\n| cat | 4 |\n| bird | 2 |\n";
        let mut state = loaded(&format!("{one_row}\n{TABLE}\n{other}\n{filler}{TABLE}"));
        // the first table has one row: nothing to sort
        assert_eq!(state.tables_on_screen(), [1, 2]);
        update(&mut state, Msg::SortOpen);
        assert_eq!(state.sorting(), Some(1));
        update(&mut state, Msg::OutlineFilter { text: "z".into() });
        update(&mut state, Msg::SortTable { dir: Dir::Next });
        assert_eq!(state.sorting(), Some(2));
        assert_eq!(columns(&state).len(), 4, "the filter starts over");
        assert_eq!(columns(&state)[0], "Pet   ▲ ascending");
        update(&mut state, Msg::SortTable { dir: Dir::Next });
        assert_eq!(state.sorting(), Some(1), "and wraps around");
        update(&mut state, Msg::SortTable { dir: Dir::Prev });
        update(&mut state, Msg::OutlineMove { by: 2 });
        update(&mut state, Msg::OutlinePick);
        assert_eq!(state.status.as_deref(), Some("sorted by Legs ▲"));
        assert_eq!(
            names(&state),
            ["beta", "alpha", "gamma", "beta", "alpha", "gamma"]
        );

        // a table alone on screen has no other to trade for
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Bottom,
            },
        );
        assert_eq!(state.tables_on_screen(), [3]);
        update(&mut state, Msg::SortOpen);
        update(&mut state, Msg::SortTable { dir: Dir::Next });
        assert_eq!(state.sorting(), Some(3));
    }
}
