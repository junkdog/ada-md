//! The message architecture (ADR-19, architecture §10). Every input is a [`Msg`];
//! [`update`] is the only place state changes and performs no I/O. Effects are [`Cmd`]s that the
//! effect runner executes, feeding results back as messages.

mod effects;
mod keymap;
mod run;

use std::mem;
use std::path::PathBuf;
use std::sync::Arc;

use crate::doc::{self, CodeId, Doc};
use crate::highlight::{self, CodeSpans, Highlights, Job, SyntaxTheme};
use crate::layout::{self, Layout, LayoutParams, Measure, RowKind, TableMode};
use crate::theme::Theme;

pub use effects::{Effects, load_doc};
pub use keymap::Keymap;
pub use run::run_tui;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub w: u16,
    pub h: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Next,
    Prev,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollBy {
    Rows(i32),
    HalfPage(Dir),
    Page(Dir),
    Top,
    Bottom,
}

/// Columns per `←`/`→` press.
pub const CODE_X_STEP: i16 = 8;

/// The result of a pure computation done off-thread. Recordings keep only the marker, and replay
/// recomputes the value from state when it reaches `update` without one (ADR-19).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Computed<T>(pub Option<T>);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Msg {
    // terminal and session
    Init {
        size: Size,
    },
    Resize {
        size: Size,
    },
    Open {
        path: PathBuf,
    },
    // user intents, produced by the keymap; keys never reach update
    Scroll {
        by: ScrollBy,
    },
    JumpHeading {
        dir: Dir,
    },
    ToggleTableMode,
    ToggleCodeWrap,
    /// Shift every code block sideways, in no-wrap mode.
    ScrollCodeX {
        by: i16,
    },
    Quit,
    // world → app: the payload is recorded, because these read the outside world
    DocLoaded {
        path: PathBuf,
        /// Milliseconds since the epoch, when known.
        mtime: Option<u64>,
        text: Arc<str>,
    },
    DocFailed {
        path: PathBuf,
        error: String,
    },
    // pure computation done off-thread: recorded as a marker only
    Highlighted {
        doc_gen: u64,
        block: CodeId,
        spans: Computed<Arc<CodeSpans>>,
    },
}

impl Msg {
    /// Messages that carry results from the outside world. `--replay-live` skips them and
    /// re-executes the commands that produced them instead.
    pub fn is_world(&self) -> bool {
        matches!(self, Msg::DocLoaded { .. } | Msg::DocFailed { .. })
    }

    /// Messages that commands produce: world results, and computed ones. `--replay-live` skips
    /// them, since its re-executed commands produce them again.
    pub fn is_result(&self) -> bool {
        self.is_world() || matches!(self, Msg::Highlighted { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    LoadDoc(PathBuf),
    Highlight(Job),
    Quit,
}

/// Settings fixed for the session.
#[derive(Clone, Debug, Default)]
pub struct Config {
    pub max_width: Option<u16>,
    /// Scroll to this source line on the first load.
    pub start_line: Option<u32>,
}

/// One loaded document with its layouts: the current one and the previous one, so toggling back
/// and forth during a resize drag stays cheap.
pub struct Document {
    pub path: PathBuf,
    pub mtime: Option<u64>,
    pub doc: Doc,
    measure: Measure,
    pub layout: Layout,
    prev: Option<Layout>,
    pub generation: u64,
}

impl Document {
    fn relayout(&mut self, theme: &Theme, params: LayoutParams) {
        if self.layout.params == params {
            return;
        }
        if let Some(prev) = &mut self.prev
            && prev.params == params
        {
            mem::swap(&mut self.layout, prev);
            return;
        }
        if self.measure.glyphs_gen != params.glyphs_gen {
            self.measure = Measure::new(&self.doc, &theme.glyphs);
        }
        let next = layout::layout_measured(&self.doc, &self.measure, &theme.glyphs, params);
        self.prev = Some(mem::replace(&mut self.layout, next));
    }
}

pub struct State {
    pub theme: Theme,
    pub config: Config,
    pub size: Size,
    pub doc: Option<Document>,
    /// The first visible row.
    pub scroll: u32,
    pub table_mode: TableMode,
    pub code_wrap: bool,
    /// The shared horizontal offset of code blocks in no-wrap mode (ADR-20).
    pub code_x: u16,
    pub highlights: Highlights,
    /// The theme's `[syntax]` table, for the highlighter.
    syntax: Arc<SyntaxTheme>,
    pub status: Option<String>,
    pub quit: bool,
    glyphs_gen: u64,
    generation: u64,
}

impl State {
    pub fn new(theme: Theme, config: Config) -> Self {
        let glyphs_gen = theme.glyphs.generation();
        let syntax = Arc::new(SyntaxTheme::new(&theme.syntax));
        Self {
            theme,
            config,
            size: Size { w: 80, h: 24 },
            doc: None,
            scroll: 0,
            table_mode: TableMode::Auto,
            code_wrap: true,
            code_x: 0,
            highlights: Highlights::default(),
            syntax,
            status: None,
            quit: false,
            glyphs_gen,
            generation: 0,
        }
    }

    /// Rows available to the document; the bottom row is the status bar.
    pub fn viewport_height(&self) -> u32 {
        u32::from(self.size.h.saturating_sub(1))
    }

    pub fn params(&self) -> LayoutParams {
        LayoutParams {
            width: self.size.w,
            max_width: self.config.max_width,
            table_mode: self.table_mode,
            code_wrap: self.code_wrap,
            glyphs_gen: self.glyphs_gen,
        }
    }

    pub fn max_scroll(&self) -> u32 {
        self.doc.as_ref().map_or(0, |d| {
            d.layout.row_count().saturating_sub(self.viewport_height())
        })
    }

    fn relayout(&mut self) {
        let params = self.params();
        let Some(doc) = &mut self.doc else {
            return;
        };
        let anchor = ScrollAnchor::capture(&doc.layout, self.scroll);
        doc.relayout(&self.theme, params);
        self.scroll = anchor.map_or(0, |a| a.restore(&doc.layout));
        self.scroll = self.scroll.min(self.max_scroll());
    }

    fn load(&mut self, path: PathBuf, mtime: Option<u64>, text: &str) {
        self.generation += 1;
        let doc = doc::parse(text);
        let measure = Measure::new(&doc, &self.theme.glyphs);
        let layout = layout::layout_measured(&doc, &measure, &self.theme.glyphs, self.params());
        let line = self.config.start_line.take();
        self.scroll = line.and_then(|l| layout.row_for_line(l)).unwrap_or(0);
        self.doc = Some(Document {
            path,
            mtime,
            doc,
            measure,
            layout,
            prev: None,
            generation: self.generation,
        });
        self.scroll = self.scroll.min(self.max_scroll());
    }

    /// The code blocks on screen, in row order.
    fn visible_code(&self) -> Vec<CodeId> {
        let Some(doc) = &self.doc else {
            return Vec::new();
        };
        let mut ids: Vec<CodeId> = Vec::new();
        let rows = doc.layout.rows.iter().skip(self.scroll as usize);
        for row in rows.take(self.viewport_height() as usize) {
            if let RowKind::Code { block, .. } = row.kind
                && ids.last() != Some(&block)
                && !ids.contains(&block)
            {
                ids.push(block);
            }
        }
        ids
    }

    fn job(&self, doc: &Document, block: CodeId, key: u64, urgent: bool) -> Option<Cmd> {
        let code = doc.doc.code_blocks().into_iter().nth(block as usize)?;
        Some(Cmd::Highlight(Job {
            doc_gen: doc.generation,
            block,
            key,
            lang: code.lang.clone()?,
            text: code.text.clone(),
            theme: self.syntax.clone(),
            urgent,
        }))
    }

    /// Jobs for every block not highlighted yet: the visible ones first, at the front of the
    /// worker's queue.
    fn highlight_all(&mut self) -> Vec<Cmd> {
        let Some(doc) = &self.doc else {
            return Vec::new();
        };
        let visible = self.visible_code();
        let rest = (0..doc.doc.code_count).filter(|b| !visible.contains(b));
        let blocks = doc.doc.code_blocks();
        let mut cmds = Vec::new();
        for (block, urgent) in visible
            .iter()
            .map(|&b| (b, true))
            .chain(rest.map(|b| (b, false)))
        {
            let Some(code) = blocks.get(block as usize) else {
                continue;
            };
            let (Some(key), Some(lang)) = (highlight::key(code), &code.lang) else {
                continue;
            };
            if !self.highlights.wants(key) {
                continue;
            }
            self.highlights.request(key);
            if urgent {
                self.highlights.urge(key);
            }
            cmds.push(Cmd::Highlight(Job {
                doc_gen: doc.generation,
                block,
                key,
                lang: lang.clone(),
                text: code.text.clone(),
                theme: self.syntax.clone(),
                urgent,
            }));
        }
        cmds
    }

    /// Move blocks that scrolled into view to the front of the worker's queue.
    fn highlight_visible(&mut self) -> Vec<Cmd> {
        let Some(doc) = &self.doc else {
            return Vec::new();
        };
        let mut cmds = Vec::new();
        for block in self.visible_code() {
            let Some(key) = doc.layout.code.get(block as usize).and_then(|c| c.key) else {
                continue;
            };
            if self.highlights.urge(key) {
                cmds.extend(self.job(doc, block, key, true));
            }
        }
        cmds
    }

    fn highlighted(&mut self, doc_gen: u64, block: CodeId, spans: Option<Arc<CodeSpans>>) {
        let spans = match spans {
            Some(spans) => spans,
            // a replay: recompute, if the block is still around
            None => {
                let Some(doc) = self.doc.as_ref().filter(|d| d.generation == doc_gen) else {
                    return;
                };
                let blocks = doc.doc.code_blocks();
                let Some(code) = blocks.get(block as usize) else {
                    return;
                };
                let (Some(key), Some(lang)) = (highlight::key(code), &code.lang) else {
                    return;
                };
                Arc::new(highlight::highlight(key, lang, &code.text, &self.syntax))
            }
        };
        self.highlights.insert(spans);
    }

    fn scroll_code_x(&mut self, by: i16) {
        if self.code_wrap {
            self.status = Some("code wraps: press w to scroll it sideways".to_owned());
            return;
        }
        let Some(doc) = &self.doc else {
            return;
        };
        let max = self
            .visible_code()
            .iter()
            .filter_map(|&b| doc.layout.code.get(b as usize))
            .map(layout::CodeInfo::max_x)
            .max();
        if let Some(max) = max {
            let x = i32::from(self.code_x.min(max)) + i32::from(by);
            self.code_x = x.clamp(0, i32::from(max)) as u16;
        }
    }

    fn scroll_by(&mut self, by: ScrollBy) {
        let page = self.viewport_height().max(1) as i64;
        let delta = match by {
            ScrollBy::Rows(n) => i64::from(n),
            ScrollBy::HalfPage(dir) => dir.sign() * (page / 2).max(1),
            ScrollBy::Page(dir) => dir.sign() * page,
            ScrollBy::Top => i64::MIN / 2,
            ScrollBy::Bottom => i64::MAX / 2,
        };
        let target = (i64::from(self.scroll) + delta).clamp(0, i64::from(self.max_scroll()));
        self.scroll = target as u32;
    }

    fn jump_heading(&mut self, dir: Dir) {
        let Some(doc) = &self.doc else {
            return;
        };
        let rows = doc.layout.headings.iter().map(|h| h.row);
        let target = match dir {
            Dir::Next => rows.filter(|&r| r > self.scroll).min(),
            Dir::Prev => rows.filter(|&r| r < self.scroll).max(),
        };
        if let Some(row) = target {
            self.scroll = row.min(self.max_scroll());
        }
    }
}

impl Dir {
    fn sign(self) -> i64 {
        match self {
            Dir::Next => 1,
            Dir::Prev => -1,
        }
    }
}

/// Scroll position in terms of the source (architecture §3, "Resize"): the source line of the
/// first visible row, and how far into that line's rows the viewport starts.
#[derive(Debug, PartialEq, Eq)]
struct ScrollAnchor {
    line: u32,
    /// Rows into the group of rows that start at `line`.
    nth: u32,
    of: u32,
    /// Decorative rows between the viewport top and the group.
    lead: u32,
}

impl ScrollAnchor {
    fn capture(layout: &Layout, scroll: u32) -> Option<Self> {
        let rows = &layout.rows;
        let first = (scroll as usize..rows.len()).find(|&r| !rows[r].src.is_none())?;
        let line = rows[first].src.start;
        let group = group_of(layout, first, line);
        Some(Self {
            line,
            nth: (scroll as usize).saturating_sub(group.start) as u32,
            of: group.len() as u32,
            lead: (first - scroll as usize) as u32,
        })
    }

    fn restore(&self, layout: &Layout) -> u32 {
        let Some(row) = layout.row_for_line(self.line) else {
            return layout.row_count();
        };
        let line = layout.rows[row as usize].src.start;
        let group = group_of(layout, row as usize, line);
        let nth = if line == self.line && self.of > 0 {
            (u64::from(self.nth) * group.len() as u64 / u64::from(self.of)) as u32
        } else {
            0
        };
        (group.start as u32 + nth).saturating_sub(self.lead)
    }
}

/// The contiguous rows around `row` whose source starts at `line`.
fn group_of(layout: &Layout, row: usize, line: u32) -> std::ops::Range<usize> {
    let rows = &layout.rows;
    let same = |r: usize| rows[r].src.start == line;
    let mut start = row;
    while start > 0 && same(start - 1) {
        start -= 1;
    }
    let mut end = row + 1;
    while end < rows.len() && same(end) {
        end += 1;
    }
    start..end
}

/// The only place state changes. Deterministic: no clock, environment or filesystem.
pub fn update(state: &mut State, msg: Msg) -> Vec<Cmd> {
    match msg {
        Msg::Init { size } | Msg::Resize { size } => {
            state.size = size;
            state.relayout();
            state.highlight_visible()
        }
        Msg::Open { path } => vec![Cmd::LoadDoc(path)],
        Msg::DocLoaded { path, mtime, text } => {
            state.load(path, mtime, &text);
            state.highlight_all()
        }
        Msg::DocFailed { path, error } => {
            state.status = Some(format!("{}: {error}", path.display()));
            Vec::new()
        }
        Msg::Scroll { by } => {
            state.status = None;
            state.scroll_by(by);
            state.highlight_visible()
        }
        Msg::JumpHeading { dir } => {
            state.status = None;
            state.jump_heading(dir);
            state.highlight_visible()
        }
        Msg::ToggleTableMode => {
            state.table_mode = state.table_mode.next();
            state.status = Some(format!("tables: {}", state.table_mode.name()));
            state.relayout();
            state.highlight_visible()
        }
        Msg::ToggleCodeWrap => {
            state.code_wrap = !state.code_wrap;
            let mode = if state.code_wrap {
                "wrap"
            } else {
                "no-wrap, ←/→ scroll"
            };
            state.status = Some(format!("code: {mode}"));
            state.relayout();
            state.highlight_visible()
        }
        Msg::ScrollCodeX { by } => {
            state.status = None;
            state.scroll_code_x(by);
            Vec::new()
        }
        Msg::Highlighted {
            doc_gen,
            block,
            spans,
        } => {
            state.highlighted(doc_gen, block, spans.0);
            Vec::new()
        }
        Msg::Quit => {
            state.quit = true;
            vec![Cmd::Quit]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded(text: &str, w: u16, h: u16) -> State {
        let mut state = State::new(Theme::builtin(), Config::default());
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
            },
        );
        state
    }

    fn top_line(state: &State) -> u32 {
        let layout = &state.doc.as_ref().unwrap().layout;
        layout.rows[state.scroll as usize..]
            .iter()
            .find(|r| !r.src.is_none())
            .unwrap()
            .src
            .start
    }

    fn sample() -> String {
        (1..=60)
            .map(|i| format!("## Heading {i}\n\nparagraph {i} with enough words to wrap at narrow widths, and then some more words.\n"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn scrolling_clamps() {
        let mut state = loaded(&sample(), 80, 20);
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Rows(-5),
            },
        );
        assert_eq!(state.scroll, 0);
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Bottom,
            },
        );
        assert_eq!(state.scroll, state.max_scroll());
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Page(Dir::Prev),
            },
        );
        assert_eq!(state.scroll, state.max_scroll() - 19);
        update(&mut state, Msg::Scroll { by: ScrollBy::Top });
        assert_eq!(state.scroll, 0);
    }

    #[test]
    fn resize_keeps_the_top_source_line() {
        let mut state = loaded(&sample(), 100, 20);
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Rows(57),
            },
        );
        let line = top_line(&state);
        for w in [40, 63, 120, 100] {
            update(
                &mut state,
                Msg::Resize {
                    size: Size { w, h: 20 },
                },
            );
            assert_eq!(top_line(&state), line, "width {w}");
        }
    }

    #[test]
    fn previous_layout_is_reused() {
        let mut state = loaded(&sample(), 100, 20);
        update(
            &mut state,
            Msg::Resize {
                size: Size { w: 50, h: 20 },
            },
        );
        let ptr = |s: &State| s.doc.as_ref().unwrap().layout.rows.as_ptr();
        let narrow = ptr(&state);
        update(
            &mut state,
            Msg::Resize {
                size: Size { w: 100, h: 20 },
            },
        );
        update(
            &mut state,
            Msg::Resize {
                size: Size { w: 50, h: 20 },
            },
        );
        assert_eq!(ptr(&state), narrow);
    }

    #[test]
    fn table_mode_cycles_and_keeps_the_top_line() {
        let mut md = sample();
        md.push_str("\n| Module | Description |\n|---|---|\n");
        for i in 0..30 {
            md.push_str(&format!(
                "| module-{i} | a description long enough to need wrapping in a narrow column {i} |\n"
            ));
        }
        md.push_str("\nafter the table\n");
        let mut state = loaded(&md, 60, 20);
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Bottom,
            },
        );
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Rows(-12),
            },
        );
        let line = top_line(&state);
        let rows = |s: &State| s.doc.as_ref().unwrap().layout.row_count();
        let mut counts = vec![rows(&state)];
        for expected in ["grid", "records", "auto"] {
            update(&mut state, Msg::ToggleTableMode);
            assert_eq!(
                state.status.as_deref(),
                Some(&*format!("tables: {expected}"))
            );
            assert_eq!(top_line(&state), line, "after switching to {expected}");
            counts.push(rows(&state));
        }
        assert_eq!(state.table_mode, TableMode::Auto);
        assert_eq!(counts[0], counts[3]);
        assert!(
            counts[2] > counts[1],
            "records take more rows than a grid: {counts:?}"
        );
    }

    #[test]
    fn heading_jumps() {
        let mut state = loaded(&sample(), 100, 20);
        update(&mut state, Msg::JumpHeading { dir: Dir::Next });
        let first = state.scroll;
        assert!(first > 0);
        update(&mut state, Msg::JumpHeading { dir: Dir::Next });
        assert!(state.scroll > first);
        update(&mut state, Msg::JumpHeading { dir: Dir::Prev });
        assert_eq!(state.scroll, first);
    }

    fn code_doc() -> String {
        let mut md = String::from("```rust\nfn main() {}\n```\n\n");
        md.push_str(&sample());
        md.push_str("\n```ts\nconst x = 1;\n```\n\n```text\nplain\n```\n\n");
        md.push_str("```rust\nfn main() {}\n```\n\n```kotlin\nval long = \"");
        md.push_str(&"x".repeat(120));
        md.push_str("\"\n```\n");
        md
    }

    fn jobs(cmds: &[Cmd]) -> Vec<(CodeId, bool)> {
        cmds.iter()
            .filter_map(|c| match c {
                Cmd::Highlight(job) => Some((job.block, job.urgent)),
                _ => None,
            })
            .collect()
    }

    fn load_cmds(state: &mut State, text: String) -> Vec<Cmd> {
        update(
            state,
            Msg::DocLoaded {
                path: "t.md".into(),
                mtime: None,
                text: text.into(),
            },
        )
    }

    #[test]
    fn highlighting_starts_with_the_visible_blocks() {
        let mut state = loaded("", 80, 10);
        let cmds = load_cmds(&mut state, code_doc());
        // block 2 is `text`; block 3 has the same key as block 0
        assert_eq!(jobs(&cmds), [(0, true), (1, false), (4, false)]);

        // scrolling blocks into view moves them to the front, once each
        let mut urged = jobs(&update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Bottom,
            },
        ));
        for _ in 0..30 {
            urged.extend(jobs(&update(
                &mut state,
                Msg::Scroll {
                    by: ScrollBy::Rows(-1),
                },
            )));
        }
        urged.sort_unstable();
        assert_eq!(urged, [(1, true), (4, true)]);

        // results clear the pending state
        let Some(Cmd::Highlight(job)) = cmds_for(&state, 1) else {
            panic!("block 1 is still pending");
        };
        update(&mut state, job.run());
        assert!(state.highlights.get(job.key).is_some());
        assert!(!state.highlights.is_pending(job.key));
    }

    /// The pending job for `block`, recreated the way a fresh load would.
    fn cmds_for(state: &State, block: CodeId) -> Option<Cmd> {
        let doc = state.doc.as_ref()?;
        let key = doc.layout.code[block as usize].key?;
        state.job(doc, block, key, false)
    }

    #[test]
    fn replayed_highlights_are_recomputed() {
        let mut state = loaded("", 80, 10);
        load_cmds(&mut state, code_doc());
        let key = state.doc.as_ref().unwrap().layout.code[1].key.unwrap();
        let replayed = |doc_gen| Msg::Highlighted {
            doc_gen,
            block: 1,
            spans: Computed(None),
        };
        update(&mut state, replayed(7));
        assert!(
            state.highlights.get(key).is_none(),
            "another document's block"
        );
        let doc_gen = state.generation;
        update(&mut state, replayed(doc_gen));
        let spans = state.highlights.get(key).expect("recomputed");
        assert!(!spans.line(0).is_empty());
    }

    #[test]
    fn code_scrolls_sideways_only_without_wrapping() {
        let mut state = loaded(&code_doc(), 60, 10);
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Bottom,
            },
        );
        update(&mut state, Msg::ScrollCodeX { by: 8 });
        assert_eq!(state.code_x, 0);
        assert!(state.status.as_deref().unwrap().contains("press w"));

        update(&mut state, Msg::Scroll { by: ScrollBy::Top });
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Rows(57),
            },
        );
        let line = top_line(&state);
        update(&mut state, Msg::ToggleCodeWrap);
        assert!(!state.code_wrap);
        assert_eq!(top_line(&state), line);
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Bottom,
            },
        );
        let max = state.doc.as_ref().unwrap().layout.code[4].max_x();
        assert!(max > 8);
        update(&mut state, Msg::ScrollCodeX { by: 8 });
        assert_eq!(state.code_x, 8);
        for _ in 0..40 {
            update(&mut state, Msg::ScrollCodeX { by: 8 });
        }
        assert_eq!(state.code_x, max);
        update(&mut state, Msg::ScrollCodeX { by: -100 });
        assert_eq!(state.code_x, 0);

        // with no code on screen, the offset stays put
        update(&mut state, Msg::ScrollCodeX { by: 8 });
        update(&mut state, Msg::Scroll { by: ScrollBy::Top });
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Rows(5),
            },
        );
        update(&mut state, Msg::ScrollCodeX { by: 8 });
        assert_eq!(state.code_x, 8);
    }

    #[test]
    fn start_line_applies_to_the_first_load() {
        let mut state = State::new(
            Theme::builtin(),
            Config {
                start_line: Some(41),
                ..Config::default()
            },
        );
        update(
            &mut state,
            Msg::Init {
                size: Size { w: 80, h: 20 },
            },
        );
        let msg = Msg::DocLoaded {
            path: "t.md".into(),
            mtime: None,
            text: sample().into(),
        };
        update(&mut state, msg.clone());
        assert_eq!(top_line(&state), 41);
        update(&mut state, msg);
        assert_eq!(state.scroll, 0);
    }
}
