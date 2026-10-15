//! Search (ADR-12) and the outline: the prompt, `n`/`N`, and the heading list. Both work on
//! messages carrying what the user typed so far, so recordings replay them exactly.

use regex::Regex;

use super::{Dir, State};
use crate::doc::{AnchorId, LinkKind};
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

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outline {
    pub filter: String,
    /// An index into the filtered entries.
    pub selected: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutlineEntry<'a> {
    pub anchor: AnchorId,
    pub level: u8,
    pub row: u32,
    pub text: &'a str,
}

/// Every word of `filter` occurs in `text`, ignoring case.
fn matches_filter(text: &str, filter: &str) -> bool {
    let text = text.to_lowercase();
    filter
        .split_whitespace()
        .all(|w| text.contains(&w.to_lowercase()))
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

    pub(super) fn search_open(&mut self) {
        self.outline = None;
        self.prompt = Some(Prompt {
            input: String::new(),
            error: None,
            saved: self.search.take(),
            origin: self.scroll,
        });
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

    /// The headings the outline lists, filtered.
    pub fn outline_entries(&self) -> Vec<OutlineEntry<'_>> {
        let (Some(doc), Some(outline)) = (&self.doc, &self.outline) else {
            return Vec::new();
        };
        doc.layout
            .headings
            .iter()
            .filter_map(|h| {
                let text = &doc.doc.anchors.get(h.anchor as usize)?.text;
                matches_filter(text, &outline.filter).then_some(OutlineEntry {
                    anchor: h.anchor,
                    level: h.level,
                    row: h.row,
                    text,
                })
            })
            .collect()
    }

    pub(super) fn open_outline(&mut self) {
        self.prompt = None;
        self.outline = Some(Outline::default());
        let scroll = self.scroll;
        let selected = self
            .outline_entries()
            .iter()
            .rposition(|e| e.row <= scroll)
            .unwrap_or(0);
        if let Some(outline) = &mut self.outline {
            outline.selected = selected;
        }
    }

    pub(super) fn outline_filter(&mut self, text: String) {
        let selected = self
            .outline_entries()
            .get(self.outline.as_ref().map_or(0, |o| o.selected))
            .map(|e| e.anchor);
        let Some(outline) = &mut self.outline else {
            return;
        };
        outline.filter = text;
        let entries = self.outline_entries();
        let keep = entries.iter().position(|e| Some(e.anchor) == selected);
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

    pub(super) fn outline_pick(&mut self) {
        let row = self
            .outline_entries()
            .get(self.outline.as_ref().map_or(0, |o| o.selected))
            .map(|e| e.row);
        self.outline = None;
        if let Some(row) = row {
            self.jump_local(row);
        }
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
}
