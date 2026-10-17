//! Search (ADR-12) and the outline: the prompt, `n`/`N`, and the filterable list of headings,
//! links, themes or a table's columns. Both work on messages carrying what the user typed so far,
//! so recordings replay them exactly.

use std::borrow::Cow;

use regex::Regex;

use super::{Cmd, Dir, Document, State};
use crate::doc::{LinkId, LinkKind, TableId};
use crate::search::{self, Match, Projection, UnitKind};

#[derive(Debug)]
pub struct Search {
    pub pattern: String,
    regex: Regex,
    /// In search-text order; stable across relayouts.
    pub matches: Vec<Match>,
    /// The matches on the current layout.
    pub projection: Projection,
    /// The current match, an index into `matches`.
    pub current: Option<u32>,
}

impl Search {
    /// `3/17`: the current match's position among the ones on screen, and how many there are.
    pub fn counter(&self) -> String {
        let total = self.projection.order.len();
        match self.current.and_then(|c| self.projection.rank(c)) {
            Some(rank) => format!("{}/{total}", rank + 1),
            None => format!("{total}"),
        }
    }
}

/// The search prompt, while it is open.
#[derive(Debug)]
pub struct Prompt {
    pub input: String,
    /// Why the input doesn't compile; the previous matches stay meanwhile.
    pub error: Option<String>,
    /// The search before the prompt opened, for `Esc`.
    saved: Option<Search>,
    /// The scroll position when the prompt opened: incremental matches start from it.
    origin: u32,
}

/// What the outline lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Listing {
    /// `o`: the headings, indented by level.
    #[default]
    Headings,
    /// `l`: every link, in the order Tab visits them.
    Links,
    /// `T`: the themes, each previewed while it is selected.
    Themes,
    /// `s`: the columns to sort a table by, each ascending and descending, then the source order
    /// if it is sorted; `None` when
    /// no table on screen has rows to sort.
    Columns(Option<TableId>),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outline {
    pub listing: Listing,
    pub filter: String,
    /// An index into the filtered entries.
    pub selected: usize,
    /// The theme before the theme list opened, put back unless another one is picked.
    theme: Option<String>,
    /// The theme list shows the light themes, not the dark ones; `←`/`→` switch.
    pub light: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineEntry<'a> {
    /// The heading's anchor, the link, the theme's index, or how to sort a table (see
    /// `sort::entry_id`).
    pub id: u32,
    /// The heading level; 0 for links.
    pub level: u8,
    pub row: u32,
    pub text: Cow<'a, str>,
    /// Where a link leads, as written, unless the text already says so.
    pub dest: Option<String>,
    pub broken: bool,
}

impl OutlineEntry<'_> {
    fn matches(&self, filter: &str) -> bool {
        match &self.dest {
            Some(dest) => matches_filter(&format!("{} {dest}", self.text), filter),
            None => matches_filter(&self.text, filter),
        }
    }
}

/// Every word of `filter` occurs in `text`. Smart-case, as search is: ignoring case unless
/// `filter` has an uppercase letter.
fn matches_filter(text: &str, filter: &str) -> bool {
    if filter.chars().any(char::is_uppercase) {
        return filter.split_whitespace().all(|w| text.contains(w));
    }
    let text = text.to_lowercase();
    filter
        .split_whitespace()
        .all(|w| text.contains(&w.to_lowercase()))
}

/// Each link once, in row order, with the text it renders as: the text of its spans on every
/// row it spans, whitespace collapsed.
fn link_entries(doc: &Document) -> Vec<OutlineEntry<'_>> {
    let layout = &doc.layout;
    let mut entries: Vec<OutlineEntry<'_>> = Vec::new();
    // per link: its entry, and the last row its text was taken from
    let mut seen: Vec<Option<(usize, u32)>> = vec![None; doc.doc.links.len()];
    for hit in &layout.hits {
        let Some(slot) = seen.get_mut(hit.link as usize) else {
            continue;
        };
        let at = match *slot {
            Some((_, row)) if row == hit.row => continue,
            Some((at, _)) => at,
            None => {
                entries.push(OutlineEntry {
                    id: hit.link,
                    level: 0,
                    row: hit.row,
                    text: Cow::Owned(String::new()),
                    dest: None,
                    broken: doc.is_broken(hit.link),
                });
                entries.len() - 1
            }
        };
        *slot = Some((at, hit.row));
        let Some(row) = layout.rows.get(hit.row as usize) else {
            continue;
        };
        let text = entries[at].text.to_mut();
        text.push(' ');
        for (t, _) in layout.spans(row).filter(|(_, s)| s.link == Some(hit.link)) {
            text.push_str(t);
        }
    }
    for entry in &mut entries {
        let text = entry.text.split_whitespace().collect::<Vec<_>>().join(" ");
        let Some(link) = doc.doc.links.get(entry.id as usize) else {
            continue;
        };
        let written = super::nav::written_dest(&doc.doc, entry.id);
        if text.is_empty() {
            entry.text = Cow::Owned(written);
        } else {
            entry.dest = (text != *link.dest).then_some(written);
            entry.text = Cow::Owned(text);
        }
    }
    entries
}

impl State {
    /// Run the search pattern again: the document changed.
    pub(super) fn rematch(&mut self) {
        let (Some(search), Some(doc)) = (&mut self.search, &mut self.doc) else {
            return;
        };
        search.matches = doc.search_map(&self.theme).find(&search.regex);
        search.current = None;
        self.reproject();
    }

    /// Project the matches onto the current layout.
    pub(super) fn reproject(&mut self) {
        let (Some(search), Some(doc)) = (&mut self.search, &mut self.doc) else {
            return;
        };
        let (map, layout) = doc.search_view(&self.theme);
        search.projection = Projection::new(map, &search.matches, layout);
    }

    fn new_search(&mut self, pattern: String, regex: Regex) -> Option<Search> {
        let doc = self.doc.as_mut()?;
        let (map, layout) = doc.search_view(&self.theme);
        let matches = map.find(&regex);
        let projection = Projection::new(map, &matches, layout);
        Some(Search {
            pattern,
            regex,
            matches,
            projection,
            current: None,
        })
    }

    /// Bring match `at` on screen, a third of the way down, unless it already is.
    fn reveal_match(&mut self, at: u32) {
        let Some(row) = self.search.as_ref().and_then(|s| s.projection.row_of(at)) else {
            return;
        };
        let height = self.viewport_height();
        if !(self.scroll..self.scroll + height).contains(&row) {
            self.scroll = row.saturating_sub(height / 3).min(self.max_scroll());
        }
    }

    pub(super) fn search_open(&mut self) -> Vec<Cmd> {
        let cmds = self.outline_close();
        self.prompt = Some(Prompt {
            input: String::new(),
            error: None,
            saved: self.search.take(),
            origin: self.scroll,
        });
        cmds
    }

    /// Incremental search: match as the pattern is typed, from where the prompt opened.
    pub(super) fn search_input(&mut self, pattern: String) {
        let Some(prompt) = &mut self.prompt else {
            return;
        };
        prompt.input.clone_from(&pattern);
        let origin = prompt.origin;
        if pattern.is_empty() {
            prompt.error = None;
            self.search = None;
            self.scroll = origin;
            return;
        }
        let regex = match search::compile(&pattern) {
            Ok(regex) => regex,
            Err(e) => {
                prompt.error = Some(e);
                return;
            }
        };
        prompt.error = None;
        self.search = self.new_search(pattern, regex);
        self.scroll = origin;
        self.step_match(Dir::Next, false);
    }

    pub(super) fn search_submit(&mut self) {
        let Some(prompt) = self.prompt.take() else {
            return;
        };
        if prompt.input.is_empty() || prompt.error.is_some() {
            // An empty pattern repeats the previous search, like vim.
            if let Some(e) = &prompt.error {
                self.status = Some(format!("bad pattern: {e}"));
            }
            self.search = prompt.saved;
            self.scroll = prompt.origin;
            self.reproject();
            if prompt.error.is_none() {
                self.search_next(Dir::Next);
            }
            return;
        }
        if let Some(search) = &self.search
            && search.projection.order.is_empty()
        {
            self.status = Some(format!("pattern not found: {}", search.pattern));
        }
    }

    pub(super) fn search_cancel(&mut self) {
        let Some(prompt) = self.prompt.take() else {
            return;
        };
        self.search = prompt.saved;
        self.scroll = prompt.origin;
        self.reproject();
    }

    /// `n` / `N`.
    pub(super) fn search_next(&mut self, dir: Dir) {
        match &self.search {
            None => self.status = Some("no previous search: / starts one".into()),
            Some(s) if s.projection.order.is_empty() => {
                self.status = Some(format!("pattern not found: {}", s.pattern));
            }
            Some(_) => self.step_match(dir, true),
        }
    }

    /// Make the next match current: after the current one if it's on screen, otherwise the
    /// first one from the top of the viewport (or the last one from its bottom, going back).
    fn step_match(&mut self, dir: Dir, from_current: bool) {
        let Some(search) = &self.search else {
            return;
        };
        let p = &search.projection;
        if p.order.is_empty() {
            return;
        }
        let (top, bottom) = (self.scroll, self.scroll + self.viewport_height());
        let row = |rank: usize| p.row_of(p.order[rank]).unwrap_or(0);
        let current = search
            .current
            .filter(|_| from_current)
            .and_then(|c| p.rank(c))
            .map(|r| r as usize)
            .filter(|&r| (top..bottom).contains(&row(r)));
        let last = p.order.len() - 1;
        let (next, wrapped) = match (current, dir) {
            (Some(r), Dir::Next) if r < last => (r + 1, false),
            (Some(_), Dir::Next) => (0, true),
            (Some(r), Dir::Prev) if r > 0 => (r - 1, false),
            (Some(_), Dir::Prev) => (last, true),
            (None, Dir::Next) => (0..=last)
                .find(|&r| row(r) >= top)
                .map_or((0, true), |r| (r, false)),
            (None, Dir::Prev) => (0..=last)
                .rev()
                .find(|&r| row(r) < bottom)
                .map_or((last, true), |r| (r, false)),
        };
        let at = p.order[next];
        if wrapped && from_current {
            self.status = Some(match dir {
                Dir::Next => "search wrapped to the top".into(),
                Dir::Prev => "search wrapped to the bottom".into(),
            });
        }
        if let Some(search) = &mut self.search {
            search.current = Some(at);
        }
        self.reveal_match(at);
    }

    /// The current match, when it is in hidden text: what kind of text, the text, and the
    /// matched bytes in it.
    pub fn hidden_match(&self) -> Option<(&'static str, &str, std::ops::Range<usize>)> {
        let search = self.search.as_ref()?;
        let m = search.matches.get(search.current? as usize)?;
        let doc = self.doc.as_ref()?;
        let map = doc.search_map.as_ref()?;
        let UnitKind::Hidden { owner } = map.units.get(m.unit as usize)?.kind else {
            return None;
        };
        let kind = match doc.doc.links.get(owner as usize)?.kind {
            LinkKind::Image => "image",
            LinkKind::Wiki { .. } => "wiki",
            _ => "link",
        };
        Some((
            kind,
            map.unit_text(m.unit),
            m.start as usize..m.end as usize,
        ))
    }

    /// Everything the outline lists, before filtering.
    pub fn outline_all(&self) -> Vec<OutlineEntry<'_>> {
        let Some(outline) = &self.outline else {
            return Vec::new();
        };
        match (outline.listing, &self.doc) {
            (Listing::Themes, _) => self
                .themes
                .iter()
                .enumerate()
                .filter(|(_, t)| t.is_light() == outline.light)
                .map(|(i, t)| OutlineEntry {
                    id: i as u32,
                    level: 0,
                    row: 0,
                    text: Cow::Borrowed(&t.name),
                    dest: (outline.theme.as_ref() == Some(&t.name)).then(|| "current".to_owned()),
                    broken: false,
                })
                .collect(),
            (_, None) => Vec::new(),
            (Listing::Headings, Some(doc)) => doc
                .layout
                .headings
                .iter()
                .filter_map(|h| {
                    Some(OutlineEntry {
                        id: h.anchor,
                        level: h.level,
                        row: h.row,
                        text: Cow::Borrowed(&doc.doc.anchors.get(h.anchor as usize)?.text),
                        dest: None,
                        broken: false,
                    })
                })
                .collect(),
            (Listing::Links, Some(doc)) => link_entries(doc),
            (Listing::Columns(None), Some(_)) => Vec::new(),
            (Listing::Columns(Some(id)), Some(_)) => self.column_entries(id),
        }
    }

    /// What the outline lists, filtered.
    pub fn outline_entries(&self) -> Vec<OutlineEntry<'_>> {
        let filter = self.outline.as_ref().map_or("", |o| &o.filter);
        let mut entries = self.outline_all();
        entries.retain(|e| e.matches(filter));
        entries
    }

    /// Open the outline at the heading the view is in, at the focused link or the first one on
    /// screen, at the current theme, or at the other way round from how the table is sorted.
    pub(super) fn open_outline(&mut self, listing: Listing) -> Vec<Cmd> {
        let cmds = self.outline_close();
        self.prompt = None;
        self.outline = Some(Outline {
            listing,
            theme: (listing == Listing::Themes).then(|| self.theme.name.clone()),
            light: self.theme.is_light(),
            ..Outline::default()
        });
        let (scroll, focus) = (self.scroll, self.visible_focus());
        let entries = self.outline_all();
        let selected = match listing {
            Listing::Headings => entries.iter().rposition(|e| e.row <= scroll),
            Listing::Links => entries
                .iter()
                .position(|e| Some(e.id) == focus)
                .or_else(|| entries.iter().position(|e| e.row >= scroll))
                .or(entries.len().checked_sub(1)),
            Listing::Themes => entries.iter().position(|e| e.dest.is_some()),
            Listing::Columns(table) => {
                let open = table.and_then(|t| self.column_to_open(t));
                entries.iter().position(|e| Some(e.id) == open)
            }
        };
        let selected = selected.unwrap_or(0);
        if let Some(outline) = &mut self.outline {
            outline.selected = selected;
        }
        cmds
    }

    /// Close the outline; the theme list puts back the theme it opened on.
    pub(super) fn outline_close(&mut self) -> Vec<Cmd> {
        match self.outline.take().and_then(|o| o.theme) {
            Some(name) => self.switch_theme(&name),
            None => Vec::new(),
        }
    }

    /// In the theme list, show the selected theme; or the one it opened on, when the filter
    /// leaves none.
    pub(super) fn preview_theme(&mut self) -> Vec<Cmd> {
        let Some(outline) = self
            .outline
            .as_ref()
            .filter(|o| o.listing == Listing::Themes)
        else {
            return Vec::new();
        };
        let selected = self.outline_entries().get(outline.selected).map(|e| e.id);
        let name = match selected.and_then(|i| self.themes.get(i as usize)) {
            Some(theme) => theme.name.clone(),
            None => outline.theme.clone().unwrap_or_default(),
        };
        self.switch_theme(&name)
    }

    /// In the theme list, `←` lists the dark themes and `→` the light ones, at the selected
    /// theme's counterpart: the first one sharing the most `-`-separated words of its name, as
    /// `gruvbox-light` does `gruvbox-dark`'s. The filter stays.
    pub(super) fn themes_tone(&mut self, dir: Dir) -> Vec<Cmd> {
        let light = dir == Dir::Next;
        let Some(outline) = self
            .outline
            .as_ref()
            .filter(|o| o.listing == Listing::Themes && o.light != light)
        else {
            return Vec::new();
        };
        let selected = self
            .outline_entries()
            .get(outline.selected)
            .map(|e| e.text.clone().into_owned());
        if let Some(outline) = &mut self.outline {
            outline.light = light;
        }
        let shared = |name: &str| {
            let words = selected.as_deref().unwrap_or("").split('-');
            words
                .zip(name.split('-'))
                .take_while(|(a, b)| a == b)
                .count()
        };
        let entries = self.outline_entries();
        // the first of the most alike, as `max_by_key` would take the last
        let best = entries
            .iter()
            .enumerate()
            .rev()
            .max_by_key(|(_, e)| shared(&e.text))
            .map_or(0, |(i, _)| i);
        if let Some(outline) = &mut self.outline {
            outline.selected = best;
        }
        self.preview_theme()
    }

    pub(super) fn outline_filter(&mut self, text: String) {
        let selected = self
            .outline_entries()
            .get(self.outline.as_ref().map_or(0, |o| o.selected))
            .map(|e| e.id);
        let Some(outline) = &mut self.outline else {
            return;
        };
        outline.filter = text;
        let keep = self
            .outline_entries()
            .iter()
            .position(|e| Some(e.id) == selected);
        if let Some(outline) = &mut self.outline {
            outline.selected = keep.unwrap_or(0);
        }
    }

    pub(super) fn outline_move(&mut self, by: i32) {
        let len = self.outline_entries().len();
        if let Some(outline) = &mut self.outline {
            let target = outline.selected as i64 + i64::from(by);
            outline.selected = target.clamp(0, len.saturating_sub(1) as i64) as usize;
        }
    }

    /// Jump to the heading; bring the link on screen, a third of the way down, and focus it, so
    /// `Enter` follows it; keep the theme on show; or sort the table by the column.
    pub(super) fn outline_pick(&mut self) -> Vec<Cmd> {
        let picked = self
            .outline_entries()
            .get(self.outline.as_ref().map_or(0, |o| o.selected))
            .map(|e| (e.id, e.row));
        let (Some(outline), Some((id, row))) = (self.outline.take(), picked) else {
            return Vec::new();
        };
        match outline.listing {
            Listing::Headings => self.jump_local(row),
            Listing::Links => {
                if !self.visible(row) {
                    self.jump_local(row.saturating_sub(self.viewport_height() / 3));
                }
                self.focus = Some(id as LinkId);
            }
            Listing::Themes => self.status = Some(format!("theme: {}", self.theme.name)),
            Listing::Columns(None) => {}
            Listing::Columns(Some(table)) => return self.sort_column(table, id),
        }
        self.visible_jobs()
    }
}

#[cfg(test)]
mod tests {
    use super::matches_filter;

    #[test]
    fn outline_filters_by_words() {
        assert!(matches_filter("P4 · Links, history", "hist p4"));
        assert!(matches_filter("anything", ""));
        assert!(!matches_filter("P4 · Links", "search"));
    }

    #[test]
    fn outline_filters_are_smart_case() {
        assert!(matches_filter("P4 · Links, history", "links"));
        assert!(matches_filter("P4 · Links, history", "Links"));
        assert!(!matches_filter("P4 · links, history", "Links"));
        assert!(
            !matches_filter("P4 · Links, history", "History links"),
            "one uppercase letter makes every word case-sensitive"
        );
        assert!(matches_filter("Ärger", "ärger"));
        assert!(!matches_filter("ärger", "Ärger"));
    }
}
