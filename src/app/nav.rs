//! Links and navigation (ADR-13, architecture §7): link states, keyboard focus, following links,
//! and the back and forward stacks. Documents loaded for a navigation arrive as `DocLoaded`; the
//! history changes only once they have, so a failed load leaves it alone.

use std::path::PathBuf;
use std::sync::Arc;

use super::{Cmd, Dir, Document, ScrollAnchor, State};
use crate::doc::{AnchorId, Doc, LinkId, LinkKind};
use crate::links::{self, LocalTarget, Resolved, Target};

/// Recently visited documents kept parsed and laid out, besides the current one.
const CACHE: usize = 15;

/// Where a link leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkState {
    /// Waiting for the resolver.
    Pending,
    /// A heading or footnote in the same document.
    Anchor(AnchorId),
    /// A note preen opens itself, maybe at a heading.
    Doc {
        path: PathBuf,
        frag: Option<String>,
    },
    /// Any other local file: handed to the system.
    File(PathBuf),
    External(String),
    /// Why it can't be followed.
    Broken(String),
}

/// A place in the history: enough to come back to the same scroll position and focus, even if
/// the width changed meanwhile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Entry {
    path: PathBuf,
    anchor: Option<ScrollAnchor>,
    focus: Option<LinkId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NavOp {
    Follow,
    Back,
    Forward,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Landing {
    /// The top, or a heading.
    Frag(Option<String>),
    Entry(Entry),
}

/// A navigation waiting for `path` to load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Nav {
    pub path: PathBuf,
    op: NavOp,
    land: Landing,
}

/// The local targets of `doc`'s links, without duplicates, and each link's index among them.
fn local_targets(doc: &Doc) -> (Vec<LocalTarget>, Vec<Option<usize>>) {
    let mut targets: Vec<LocalTarget> = Vec::new();
    let index = doc
        .links
        .iter()
        .map(|link| match Target::of(link) {
            Target::Local { target, .. } => Some(match targets.iter().position(|t| *t == target) {
                Some(i) => i,
                None => {
                    targets.push(target);
                    targets.len() - 1
                }
            }),
            _ => None,
        })
        .collect();
    (targets, index)
}

/// Link states that need no filesystem: anchors, footnotes, external links.
pub(super) fn initial_links(doc: &Doc) -> Vec<LinkState> {
    doc.links
        .iter()
        .map(|link| match Target::of(link) {
            Target::Anchor(frag) => match links::find_anchor(doc, &frag) {
                Some(a) => LinkState::Anchor(a),
                None => LinkState::Broken(format!("no heading #{frag}")),
            },
            Target::Footnote(id) => match doc.footnotes.get(id as usize).and_then(|f| f.anchor) {
                Some(a) => LinkState::Anchor(a),
                None => LinkState::Broken("the footnote is never defined".into()),
            },
            Target::External(url) => LinkState::External(url),
            Target::Local { .. } => LinkState::Pending,
            Target::Empty => LinkState::Broken("empty link".into()),
        })
        .collect()
}

/// The command that resolves a new document's local links, if it has any.
pub(super) fn resolve_cmd(doc: &Document) -> Option<Cmd> {
    let (targets, _) = local_targets(&doc.doc);
    (!targets.is_empty()).then(|| Cmd::ResolveLinks {
        doc_gen: doc.generation,
        source: doc.path.clone(),
        targets,
    })
}

impl State {
    pub(super) fn links_resolved(&mut self, doc_gen: u64, resolved: &[Resolved]) {
        let doc = self
            .doc
            .iter_mut()
            .chain(self.cache.iter_mut())
            .find(|d| d.generation == doc_gen);
        let Some(doc) = doc else {
            return;
        };
        let (_, index) = local_targets(&doc.doc);
        for (id, i) in index.into_iter().enumerate() {
            let (Some(i), Some(link)) = (i, doc.doc.links.get(id)) else {
                continue;
            };
            let frag = match Target::of(link) {
                Target::Local { frag, .. } => frag,
                _ => None,
            };
            doc.links[id] = match resolved.get(i) {
                Some(Resolved::Found(path)) if links::is_markdown(path) => LinkState::Doc {
                    path: path.clone(),
                    frag,
                },
                Some(Resolved::Found(path)) => LinkState::File(path.clone()),
                Some(Resolved::Broken(why)) => LinkState::Broken(why.clone()),
                None => LinkState::Broken("not resolved".into()),
            };
        }
    }

    fn visible(&self, row: u32) -> bool {
        (self.scroll..self.scroll + self.viewport_height()).contains(&row)
    }

    /// Scroll as little as possible to bring `row` on screen.
    pub(super) fn reveal(&mut self, row: u32) {
        let height = self.viewport_height().max(1);
        if row < self.scroll {
            self.scroll = row;
        } else if row >= self.scroll + height {
            self.scroll = row + 1 - height;
        }
        self.scroll = self.scroll.min(self.max_scroll());
    }

    /// Each link once, at its first row, in row order.
    fn stops(&self) -> Vec<(u32, LinkId)> {
        let Some(doc) = &self.doc else {
            return Vec::new();
        };
        let mut seen = vec![false; doc.doc.links.len()];
        let mut stops = Vec::new();
        for hit in &doc.layout.hits {
            if let Some(s) = seen.get_mut(hit.link as usize)
                && !*s
            {
                *s = true;
                stops.push((hit.row, hit.link));
            }
        }
        stops
    }

    /// The focused link, if it's on screen.
    pub fn visible_focus(&self) -> Option<LinkId> {
        let focus = self.focus?;
        let doc = self.doc.as_ref()?;
        doc.layout
            .hits
            .iter()
            .any(|h| h.link == focus && self.visible(h.row))
            .then_some(focus)
    }

    /// `Tab` / `S-Tab`: the next link after the focused one, or the first one from the top of
    /// the viewport, scrolling when it runs past the edge. Wraps around.
    pub(super) fn focus_link(&mut self, dir: Dir) {
        let stops = self.stops();
        if stops.is_empty() {
            self.status = Some("no links".into());
            return;
        }
        let current = self
            .visible_focus()
            .and_then(|f| stops.iter().position(|&(_, l)| l == f));
        let bottom = self.scroll + self.viewport_height();
        let last = stops.len() - 1;
        let (next, wrapped) = match (current, dir) {
            (Some(i), Dir::Next) if i < last => (i + 1, false),
            (Some(_), Dir::Next) => (0, true),
            (Some(i), Dir::Prev) if i > 0 => (i - 1, false),
            (Some(_), Dir::Prev) => (last, true),
            (None, Dir::Next) => match stops.iter().position(|&(r, _)| r >= self.scroll) {
                Some(i) => (i, false),
                None => (0, true),
            },
            (None, Dir::Prev) => match stops.iter().rposition(|&(r, _)| r < bottom) {
                Some(i) => (i, false),
                None => (last, true),
            },
        };
        if wrapped {
            self.status = Some(match dir {
                Dir::Next => "links wrapped to the top".into(),
                Dir::Prev => "links wrapped to the bottom".into(),
            });
        }
        let (row, link) = stops[next];
        self.focus = Some(link);
        self.reveal(row);
    }

    fn entry(&self) -> Option<Entry> {
        let doc = self.doc.as_ref()?;
        Some(Entry {
            path: doc.path.clone(),
            anchor: ScrollAnchor::capture(&doc.layout, self.scroll),
            focus: self.focus,
        })
    }

    /// A jump within the document that `Back` can undo.
    pub(super) fn jump_local(&mut self, row: u32) {
        if let Some(entry) = self.entry() {
            self.back.push(entry);
            self.forward.clear();
        }
        self.focus = None;
        self.scroll = row.min(self.max_scroll());
    }

    fn anchor_row(&self, anchor: AnchorId) -> Option<u32> {
        let doc = self.doc.as_ref()?;
        doc.layout
            .anchor_rows
            .get(anchor as usize)
            .copied()
            .filter(|&r| r != u32::MAX)
    }

    /// `Enter`: follow the focused link.
    pub(super) fn follow(&mut self) -> Vec<Cmd> {
        if self.nav.is_some() {
            return Vec::new();
        }
        let (Some(link), Some(doc)) = (self.visible_focus(), &self.doc) else {
            self.status = Some("no link focused: Tab picks one".into());
            return Vec::new();
        };
        let state = doc.links.get(link as usize).cloned();
        let same_doc = |path: &PathBuf| *path == doc.path;
        match state {
            None | Some(LinkState::Pending) => {
                self.status = Some("still resolving links".into());
                Vec::new()
            }
            Some(LinkState::Anchor(a)) => {
                match self.anchor_row(a) {
                    Some(row) => self.jump_local(row),
                    None => self.status = Some("that heading isn't shown".into()),
                }
                Vec::new()
            }
            Some(LinkState::Doc { path, frag }) if same_doc(&path) => {
                let anchor = frag.as_deref().map(|f| links::find_anchor(&doc.doc, f));
                match anchor {
                    None => self.jump_local(0),
                    Some(Some(a)) => {
                        let row = self.anchor_row(a).unwrap_or(0);
                        self.jump_local(row);
                    }
                    Some(None) => {
                        self.status = Some(format!("no heading #{}", frag.unwrap_or_default()));
                    }
                }
                Vec::new()
            }
            Some(LinkState::Doc { path, frag }) => {
                self.nav = Some(Nav {
                    path: path.clone(),
                    op: NavOp::Follow,
                    land: Landing::Frag(frag),
                });
                vec![Cmd::LoadDoc(path)]
            }
            Some(LinkState::File(path)) => {
                let what = if links::is_image(&path) {
                    "image"
                } else {
                    "file"
                };
                self.status = Some(format!("opening the {what} {}", path.display()));
                vec![Cmd::OpenWithSystem(path.into_os_string())]
            }
            Some(LinkState::External(url)) if links::is_openable(&url) => {
                self.status = Some(format!("opening {url}"));
                vec![Cmd::OpenWithSystem(url.into())]
            }
            Some(LinkState::External(url)) => {
                self.status = Some(format!("can't open links like {url}"));
                Vec::new()
            }
            Some(LinkState::Broken(why)) => {
                self.status = Some(format!("broken link: {why}"));
                Vec::new()
            }
        }
    }

    pub(super) fn go_back(&mut self) -> Vec<Cmd> {
        self.go(NavOp::Back)
    }

    pub(super) fn go_forward(&mut self) -> Vec<Cmd> {
        self.go(NavOp::Forward)
    }

    fn go(&mut self, op: NavOp) -> Vec<Cmd> {
        if self.nav.is_some() {
            return Vec::new();
        }
        let stack = if op == NavOp::Back {
            &self.back
        } else {
            &self.forward
        };
        let Some(entry) = stack.last().cloned() else {
            self.status = Some(match op {
                NavOp::Back => "nothing to go back to".into(),
                _ => "nothing to go forward to".into(),
            });
            return Vec::new();
        };
        if self.doc.as_ref().is_some_and(|d| d.path == entry.path) {
            self.shift_history(op);
            self.restore(&entry);
            return Vec::new();
        }
        self.nav = Some(Nav {
            path: entry.path.clone(),
            op,
            land: Landing::Entry(entry.clone()),
        });
        vec![Cmd::LoadDoc(entry.path)]
    }

    /// Move the current place onto the stack opposite the one `op` takes from.
    fn shift_history(&mut self, op: NavOp) {
        let current = self.entry();
        match op {
            NavOp::Follow => {
                self.back.extend(current);
                self.forward.clear();
            }
            NavOp::Back => {
                self.back.pop();
                self.forward.extend(current);
            }
            NavOp::Forward => {
                self.forward.pop();
                self.back.extend(current);
            }
        }
    }

    fn restore(&mut self, entry: &Entry) {
        let Some(doc) = &self.doc else {
            return;
        };
        self.scroll = entry
            .anchor
            .as_ref()
            .map_or(0, |a| a.restore(&doc.layout))
            .min(self.max_scroll());
        self.focus = entry.focus.filter(|&f| (f as usize) < doc.doc.links.len());
    }

    /// The document a navigation waited for has loaded.
    pub(super) fn arrive(&mut self, nav: Nav, mtime: Option<u64>, text: Arc<str>) -> Vec<Cmd> {
        self.shift_history(nav.op);
        let cmds = self.install(nav.path, mtime, text);
        match nav.land {
            Landing::Entry(entry) => self.restore(&entry),
            Landing::Frag(None) => {}
            Landing::Frag(Some(frag)) => {
                let anchor = self
                    .doc
                    .as_ref()
                    .and_then(|d| links::find_anchor(&d.doc, &frag));
                match anchor.and_then(|a| self.anchor_row(a)) {
                    Some(row) => self.scroll = row.min(self.max_scroll()),
                    None => self.status = Some(format!("no heading #{frag}")),
                }
            }
        }
        cmds
    }

    /// Keep a document we navigated away from, dropping the least recent beyond the cap.
    pub(super) fn stash(&mut self, doc: Document) {
        self.cache.retain(|d| d.path != doc.path);
        self.cache.push(doc);
        if self.cache.len() > CACHE {
            self.cache.remove(0);
        }
    }

    /// What the status line shows for the focused link: its destination as written, and why
    /// it's broken if it is.
    pub fn focus_text(&self) -> Option<String> {
        let link = self.visible_focus()?;
        let doc = self.doc.as_ref()?;
        let link_ref = doc.doc.links.get(link as usize)?;
        let shown = match link_ref.kind {
            LinkKind::Footnote(id) => {
                let n = doc.doc.footnotes.get(id as usize).map_or(0, |f| f.number);
                format!("footnote [{n}]")
            }
            LinkKind::Wiki { .. } => format!("[[{}]]", link_ref.dest),
            _ => link_ref.dest.to_string(),
        };
        Some(match doc.links.get(link as usize) {
            Some(LinkState::Broken(why)) => format!("{shown}  ✗ {why}"),
            _ => shown,
        })
    }
}
