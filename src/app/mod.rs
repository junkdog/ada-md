//! The message architecture (ADR-19, architecture §10). Every input is a [`Msg`];
//! [`update`] is the only place state changes and performs no I/O. Effects are [`Cmd`]s that the
//! effect runner executes, feeding results back as messages.

mod effects;
mod keymap;
mod run;

use std::mem;
use std::path::PathBuf;
use std::sync::Arc;

use crate::doc::{self, Doc};
use crate::layout::{self, Layout, LayoutParams, Measure, TableMode};
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
}

impl Msg {
    /// Messages that carry results from the outside world. `--replay-live` skips them and
    /// re-executes the commands that produced them instead.
    pub fn is_world(&self) -> bool {
        matches!(self, Msg::DocLoaded { .. } | Msg::DocFailed { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    LoadDoc(PathBuf),
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
    pub status: Option<String>,
    pub quit: bool,
    glyphs_gen: u64,
    generation: u64,
}

impl State {
    pub fn new(theme: Theme, config: Config) -> Self {
        let glyphs_gen = theme.glyphs.generation();
        Self {
            theme,
            config,
            size: Size { w: 80, h: 24 },
            doc: None,
            scroll: 0,
            table_mode: TableMode::Auto,
            code_wrap: true,
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
            Vec::new()
        }
        Msg::Open { path } => vec![Cmd::LoadDoc(path)],
        Msg::DocLoaded { path, mtime, text } => {
            state.load(path, mtime, &text);
            Vec::new()
        }
        Msg::DocFailed { path, error } => {
            state.status = Some(format!("{}: {error}", path.display()));
            Vec::new()
        }
        Msg::Scroll { by } => {
            state.status = None;
            state.scroll_by(by);
            Vec::new()
        }
        Msg::JumpHeading { dir } => {
            state.status = None;
            state.jump_heading(dir);
            Vec::new()
        }
        Msg::ToggleTableMode => {
            state.table_mode = state.table_mode.next();
            state.status = Some(format!("tables: {}", state.table_mode.name()));
            state.relayout();
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
