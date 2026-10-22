//! The message architecture (ADR-19, architecture §10). Every input is a [`Msg`];
//! [`update`] is the only place state changes and performs no I/O. Effects are [`Cmd`]s that the
//! effect runner executes, feeding results back as messages.

mod effects;
mod find;
mod handoff;
mod imaging;
mod keymap;
mod load;
mod nav;
mod run;
mod save;
mod sort;
mod watch;

use std::ffi::OsString;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::doc::{CodeId, Doc, LinkId};
use crate::filetype::FileType;
use crate::git::LineMarks;
use crate::highlight::{self, CodeSpans, Epoch, Highlights, Job, SyntaxTheme};
use crate::images::{self, Encoded, Graphics, ImageCache, ImageKey, ImageMode, Probe, Proto};
use crate::layout::{
    self, ImageFit, Layout, LayoutParams, LineNumbers, Measure, RowKind, TableMode,
};
use crate::links::{LocalTarget, Resolved};
use crate::search::SearchMap;
use crate::theme::{Glyphs, Theme};

pub use effects::{Effects, Head, cut_to_head, editor_command, load_doc, load_doc_head, osc52};
pub use find::{Listing, Outline, OutlineEntry, Prompt, Search};
pub use imaging::{ImageFile, ImageView};
pub use keymap::{Action, Bindings, Key, KeySeq, Keymap};
pub use load::{Prepare, Prepared, prepare};
pub use nav::LinkState;
use nav::{Entry, Nav};
pub use run::{arenas, one_arena, run_tui};
pub(crate) use run::{draw_frame, init_terminal, restore_terminal};
pub use save::Saving;

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
    /// The terminal can show images: its protocol, and its font's cell size in pixels. Sent
    /// after `Init` when images are on.
    Graphics {
        proto: Proto,
        cell: Size,
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
    /// `s`: the outline, listing the columns of the first table on screen to sort it by.
    SortOpen,
    /// In the column list: the columns of the next or previous table on screen instead.
    SortTable {
        dir: Dir,
    },
    ToggleCodeWrap,
    /// `#`: show or hide line numbers for the kind of file shown.
    ToggleLineNumbers,
    /// `B`: the theme's background, or the terminal's.
    ToggleThemeBg,
    /// Shift every code block sideways, in no-wrap mode.
    ScrollCodeX {
        by: i16,
    },
    FocusLink {
        dir: Dir,
    },
    Follow,
    Back,
    Forward,
    OpenOutline,
    /// `l`: the outline, listing the links.
    OpenLinks,
    OutlineFilter {
        text: String,
    },
    OutlineMove {
        by: i32,
    },
    OutlinePick,
    OutlineClose,
    SearchOpen,
    /// The whole pattern typed so far.
    SearchInput {
        pattern: String,
    },
    SearchSubmit,
    SearchCancel,
    SearchNext {
        dir: Dir,
    },
    /// `]c` / `[c`: the next or previous run of changed rows.
    JumpChange {
        dir: Dir,
    },
    /// Read the file again, keeping the place, and refresh the gutter.
    Reload,
    /// `T`: the outline, listing the themes.
    OpenThemes,
    /// In the theme list: the dark themes (`Prev`, `←`) or the light ones (`Next`, `→`).
    ThemesTone {
        dir: Dir,
    },
    /// `?`: the help overlay, which lists the keys.
    Help,
    HelpScroll {
        by: i32,
    },
    HelpClose,
    /// `e`: hand the file to `$VISUAL` / `$EDITOR` at the current line (ADR-18).
    Edit,
    /// `y`: copy the focused link's target, or the code block on screen, through OSC 52.
    Yank,
    /// `S`: offer to save the settings in effect to the config file, listing what changes.
    SaveConfig,
    /// In the save box: write the file.
    SaveConfirm,
    /// In the save box: close it, writing nothing.
    SaveCancel,
    Quit,
    // world → app: the payload is recorded, because these read the outside world
    DocLoaded {
        path: PathBuf,
        /// Milliseconds since the epoch, when known.
        mtime: Option<u64>,
        text: Arc<str>,
        /// The document parsed and laid out by the loader (O-32); recorded as a marker only.
        doc: Computed<Prepared>,
    },
    DocFailed {
        path: PathBuf,
        error: String,
    },
    /// The local targets of a document's links, in the order `Cmd::ResolveLinks` asked.
    LinksResolved {
        doc_gen: u64,
        resolved: Vec<Resolved>,
    },
    /// Handing a file or URL to the system's opener failed.
    OpenFailed {
        target: String,
        error: String,
    },
    /// `git diff HEAD` and `git diff` of a document's file (ADR-06). Only sent inside a
    /// repository; `untracked` means the file isn't in the index, and the diffs are empty.
    GitDiff {
        doc_gen: u64,
        untracked: bool,
        total: Arc<str>,
        unstaged: Arc<str>,
    },
    /// The headers of a document's images, in the order `Cmd::ProbeImages` asked.
    ImagesProbed {
        doc_gen: u64,
        probes: Vec<Probe>,
    },
    /// The editor `Cmd::SpawnEditor` started has exited; `-1` when a signal ended it.
    EditorExited {
        status: i32,
    },
    /// The watched file's modification time changed (ADR-26).
    FileChanged {
        path: PathBuf,
        mtime: Option<u64>,
    },
    /// The config file, read for `S`: a missing one reads as the file ada would write.
    ConfigRead {
        path: PathBuf,
        text: Arc<str>,
    },
    /// `Cmd::WriteConfig` wrote the config file.
    ConfigWritten {
        path: PathBuf,
    },
    /// Reading or writing the config file failed.
    ConfigFailed {
        error: String,
    },
    // pure computation done off-thread: recorded as a marker only
    Highlighted {
        doc_gen: u64,
        block: CodeId,
        /// For the block's first lines only, while the worker goes on (O-05): how many.
        lines: Option<u32>,
        spans: Computed<Arc<CodeSpans>>,
    },
    /// An image decoded and encoded for the terminal, `size` cells big. Replay can't recompute
    /// it, as image bytes aren't recorded, so a replayed image stays a placeholder.
    ImageReady {
        path: PathBuf,
        mtime: Option<u64>,
        size: Size,
        image: Computed<Result<Encoded, String>>,
    },
}

impl Msg {
    /// Messages that carry results from the outside world. `--replay-live` skips them and
    /// re-executes the commands that produced them instead.
    pub fn is_world(&self) -> bool {
        matches!(
            self,
            Msg::DocLoaded { .. }
                | Msg::DocFailed { .. }
                | Msg::LinksResolved { .. }
                | Msg::OpenFailed { .. }
                | Msg::GitDiff { .. }
                | Msg::ImagesProbed { .. }
                | Msg::EditorExited { .. }
                | Msg::FileChanged { .. }
                | Msg::ConfigRead { .. }
                | Msg::ConfigWritten { .. }
                | Msg::ConfigFailed { .. }
        )
    }

    /// Messages that commands produce: world results, and computed ones. `--replay-live` skips
    /// them, since its re-executed commands produce them again.
    pub fn is_result(&self) -> bool {
        self.is_world() || matches!(self, Msg::Highlighted { .. } | Msg::ImageReady { .. })
    }

    /// Messages that would start a load, or that `update` turns away while a navigation waits
    /// for its document: the interactive loop holds them until it has arrived (O-32).
    pub fn waits_for_load(&self) -> bool {
        matches!(
            self,
            Msg::Open { .. }
                | Msg::Follow
                | Msg::Back
                | Msg::Forward
                | Msg::Reload
                | Msg::FileChanged { .. }
                | Msg::EditorExited { .. }
        )
    }

    /// Messages that leave the image view open: results, and the terminal's own.
    fn keeps_view(&self) -> bool {
        self.is_result()
            || matches!(
                self,
                Msg::Init { .. } | Msg::Graphics { .. } | Msg::Resize { .. } | Msg::Open { .. }
            )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    /// Read a document. A navigation's goes to the effect runner's loader, which also parses it
    /// and lays it out for `prepare`, off the UI thread, while the document on screen answers
    /// (O-32). The first, with nothing on screen to answer, is read at once: the UI thread `wait`s.
    LoadDoc {
        path: PathBuf,
        prepare: Box<Prepare>,
        wait: bool,
    },
    Highlight(Job),
    /// Highlighting is wanted only for this document and these rules from now on: the worker
    /// drops other jobs, and stops the one running if it's another's (O-05).
    HighlightOnly(Epoch),
    /// Find the files a document's links name, relative to `source`.
    ResolveLinks {
        doc_gen: u64,
        source: PathBuf,
        targets: Vec<LocalTarget>,
    },
    /// Hand a file or URL to the desktop's opener (`xdg-open`).
    OpenWithSystem(OsString),
    /// Diff a document's file against the index and `HEAD`, off the UI thread.
    RunGit {
        doc_gen: u64,
        path: PathBuf,
    },
    /// Read the headers of the images a document's links lead to.
    ProbeImages {
        doc_gen: u64,
        paths: Vec<PathBuf>,
    },
    DecodeImage(images::Job),
    /// Run the editor on `file` at `line`, in the foreground, with the terminal handed over.
    SpawnEditor {
        file: PathBuf,
        line: u32,
    },
    /// Put text on the system clipboard through the terminal (OSC 52).
    Osc52(String),
    /// Watch `path` for changes, through inotify or by polling its modification time (ADR-26).
    /// Replaces the file watched before.
    Watch {
        path: PathBuf,
        mtime: Option<u64>,
    },
    /// Read the config file, for `S` to compare the settings in effect with.
    ReadConfig(PathBuf),
    /// Write the config file. Only the interactive loop does: a replay never touches it.
    WriteConfig {
        path: PathBuf,
        text: String,
    },
    Quit,
}

/// Settings fixed for the session: the command line over the config file.
#[derive(Clone, Debug)]
pub struct Config {
    pub max_width: Option<u16>,
    /// Scroll to this source line on the first load.
    pub start_line: Option<u32>,
    /// Whether to query the terminal for image support. Only the TUI reads it.
    pub images: ImageMode,
    /// Reload when the file changes on disk.
    pub live_reload: bool,
    /// Leave the last frame on the screen on quit.
    pub keep_screen: bool,
    /// Whether the theme draws its own background, or leaves the terminal's.
    pub theme_bg: bool,
    /// The table layout, code wrapping and line numbers to start with.
    pub table_mode: TableMode,
    pub code_wrap: bool,
    pub line_numbers: LineNumbers,
    pub keys: Bindings,
    /// The config file, for the help to point at.
    pub file: Option<PathBuf>,
    /// The type `-l` gives the file ada opened, over what its name and contents say. Files
    /// reached by a link are typed as usual.
    pub language: Option<(PathBuf, FileType)>,
}

impl Config {
    /// The type `-l` forces on `path`, if any.
    pub fn forced_type(&self, path: &Path) -> Option<FileType> {
        self.language
            .as_ref()
            .filter(|(p, _)| p == path)
            .map(|&(_, t)| t)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_width: None,
            start_line: None,
            theme_bg: true,
            images: ImageMode::default(),
            live_reload: true,
            keep_screen: true,
            table_mode: TableMode::Auto,
            code_wrap: true,
            line_numbers: LineNumbers::Off,
            keys: Bindings::default(),
            file: None,
            language: None,
        }
    }
}

/// One loaded document with its layouts: the current one and the previous one, so toggling back
/// and forth during a resize drag stays cheap.
pub struct Document {
    pub path: PathBuf,
    /// Markdown, an image, or code: what the file holds decides how it's parsed and laid out.
    pub filetype: FileType,
    pub mtime: Option<u64>,
    pub text: Arc<str>,
    pub doc: Doc,
    measure: Measure,
    pub layout: Layout,
    prev: Option<Layout>,
    pub generation: u64,
    /// Where each link leads, indexed by [`LinkId`].
    pub links: Vec<LinkState>,
    /// The searchable text, built on the first search.
    search_map: Option<SearchMap>,
    /// The git gutter; `None` outside a repository, or until git answers.
    pub marks: Option<LineMarks>,
    /// The image files links lead to, indexed by [`LinkId`], once their headers are read.
    pub images: Vec<Option<ImageFile>>,
}

impl Document {
    fn new(
        path: PathBuf,
        mtime: Option<u64>,
        text: Arc<str>,
        glyphs: &Glyphs,
        params: LayoutParams,
        generation: u64,
        forced: Option<FileType>,
    ) -> Self {
        let filetype = forced.unwrap_or_else(|| FileType::of(&path, &text));
        let doc = filetype.parse(&path, &text);
        let measure = Measure::new(&doc, glyphs);
        let params = filetype.params(params);
        let layout = layout::layout_measured(&doc, &measure, glyphs, params);
        let links = nav::initial_links(&doc);
        let images = vec![None; doc.links.len()];
        Self {
            path,
            filetype,
            mtime,
            text,
            doc,
            measure,
            layout,
            prev: None,
            generation,
            links,
            search_map: None,
            marks: None,
            images,
        }
    }

    pub fn search_map(&mut self, theme: &Theme) -> &SearchMap {
        self.search_view(theme).0
    }

    /// The searchable text and the layout to project matches onto.
    fn search_view(&mut self, theme: &Theme) -> (&SearchMap, &Layout) {
        let map = self
            .search_map
            .get_or_insert_with(|| SearchMap::build(&self.doc, &theme.glyphs));
        (map, &self.layout)
    }

    /// Whether following `link` can't work.
    pub fn is_broken(&self, link: LinkId) -> bool {
        matches!(self.links.get(link as usize), Some(LinkState::Broken(_)))
    }

    fn relayout(&mut self, glyphs: &Glyphs, params: LayoutParams) {
        let params = self.filetype.params(params);
        if self.layout.params == params {
            return;
        }
        // The images' fit shapes no row until an image is measured, so the probe's answer after the
        // first paint (O-06) keeps the rows: laying out again cost XL 38 ms and 16 MiB there.
        let images = LayoutParams {
            images: params.images,
            ..self.layout.params
        };
        if images == params && !self.measure.has_images() {
            self.layout.params = params;
            return;
        }
        if let Some(prev) = &mut self.prev
            && prev.params == params
        {
            mem::swap(&mut self.layout, prev);
            return;
        }
        if self.measure.glyphs_gen != params.glyphs_gen {
            self.measure.remeasure(&self.doc, glyphs);
        }
        let next = self.lay_out(glyphs, params);
        self.prev = Some(mem::replace(&mut self.layout, next));
    }

    /// Lay out again after the measurements changed, dropping the previous layout.
    fn relayout_now(&mut self, glyphs: &Glyphs, params: LayoutParams) {
        let params = self.filetype.params(params);
        self.layout = self.lay_out(glyphs, params);
    }

    /// A new layout, in the buffers of the previous one when there is one.
    fn lay_out(&mut self, glyphs: &Glyphs, params: LayoutParams) -> Layout {
        let (doc, measure) = (&self.doc, &self.measure);
        match self.prev.take() {
            Some(spare) => layout::relayout(doc, measure, glyphs, params, spare),
            None => layout::layout_measured(doc, measure, glyphs, params),
        }
    }
}

pub struct State {
    /// The theme in effect: [`designed`](Self::designed), on the terminal's background when
    /// [`theme_bg`](Self::theme_bg) is off.
    pub theme: Theme,
    /// The theme as it was loaded, with its own background.
    designed: Theme,
    /// Whether the theme draws its own background (`B`).
    pub theme_bg: bool,
    /// The themes `T` lists, the current one among them. Until `T` first opens, only the user's
    /// own: the built-ins are parsed then (O-42).
    pub themes: Vec<Theme>,
    themes_listed: bool,
    pub config: Config,
    pub size: Size,
    pub doc: Option<Document>,
    /// The first visible row.
    pub scroll: u32,
    pub table_mode: TableMode,
    pub code_wrap: bool,
    pub line_numbers: LineNumbers,
    /// The shared horizontal offset of code blocks in no-wrap mode (ADR-20).
    pub code_x: u16,
    pub highlights: Highlights,
    /// The theme's `[syntax]` table, for the highlighter.
    syntax: Arc<SyntaxTheme>,
    pub status: Option<String>,
    pub quit: bool,
    glyphs_gen: u64,
    generation: u64,
    /// The focused link, if any (architecture §7, "Focus").
    pub focus: Option<LinkId>,
    back: Vec<Entry>,
    forward: Vec<Entry>,
    /// Recently visited documents, most recent last, without the current one.
    cache: Vec<Document>,
    /// A navigation waiting for its document to load.
    nav: Option<Nav>,
    pub search: Option<Search>,
    /// The search prompt, while it is open.
    pub prompt: Option<Prompt>,
    pub outline: Option<Outline>,
    /// The terminal's image support; `None` shows images as text.
    pub graphics: Option<Graphics>,
    pub images: ImageCache,
    /// The full-screen image view, while it is open.
    pub view: Option<ImageView>,
    /// The help overlay's scroll offset, while it is open.
    pub help: Option<u16>,
    /// The box `S` opens, asking whether to save the config, while it is open.
    pub save: Option<Saving>,
}

impl State {
    pub fn new(theme: Theme, config: Config) -> Self {
        let glyphs_gen = theme.glyphs.generation();
        let syntax = Arc::new(SyntaxTheme::new(&theme.syntax));
        Self {
            // `main` adds the user's themes, and the first `T` the built-ins
            themes: Vec::new(),
            themes_listed: false,
            theme: theme.with_bg(config.theme_bg),
            designed: theme,
            theme_bg: config.theme_bg,
            size: Size { w: 80, h: 24 },
            doc: None,
            scroll: 0,
            table_mode: config.table_mode,
            code_wrap: config.code_wrap,
            line_numbers: config.line_numbers,
            config,
            code_x: 0,
            highlights: Highlights::default(),
            syntax,
            status: None,
            quit: false,
            glyphs_gen,
            generation: 0,
            focus: None,
            back: Vec::new(),
            forward: Vec::new(),
            cache: Vec::new(),
            nav: None,
            search: None,
            prompt: None,
            outline: None,
            graphics: None,
            images: ImageCache::default(),
            view: None,
            help: None,
            save: None,
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
            line_numbers: self.line_numbers,
            glyphs_gen: self.glyphs_gen,
            // an image may take 60 % of the viewport
            images: self.graphics.map(|g| ImageFit {
                cell: g.cell,
                max_rows: (self.viewport_height() * 3 / 5).max(1) as u16,
                screen_rows: self.viewport_height() as u16,
            }),
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
        doc.relayout(&self.theme.glyphs, params);
        self.scroll = anchor.map_or(0, |a| a.restore(&doc.layout));
        self.scroll = self.scroll.min(self.max_scroll());
        self.reproject();
    }

    /// Make a loaded document current, reusing the cached one when its text is unchanged, or
    /// the one the loader prepared, and stash the previous one. Returns the commands a fresh
    /// document needs.
    fn install(
        &mut self,
        path: PathBuf,
        mtime: Option<u64>,
        text: Arc<str>,
        prepared: Option<Prepared>,
    ) -> Vec<Cmd> {
        let params = self.params();
        let cached = self
            .cache
            .iter()
            .position(|d| d.path == path && d.text == text)
            .map(|i| self.cache.remove(i));
        let mut cmds = Vec::new();
        let doc = match cached {
            Some(mut doc) => {
                doc.mtime = mtime;
                doc.relayout(&self.theme.glyphs, params);
                doc
            }
            None => {
                self.generation += 1;
                let glyphs = &self.theme.glyphs;
                let mut doc = match prepared.and_then(Prepared::take) {
                    // laid out for the screen as it was: the width or the glyphs may have
                    // changed since
                    Some(mut doc) => {
                        doc.generation = self.generation;
                        doc.relayout(glyphs, params);
                        doc
                    }
                    None => {
                        let forced = self.config.forced_type(&path);
                        Document::new(path, mtime, text, glyphs, params, self.generation, forced)
                    }
                };
                if let Some(old) = self.doc.as_ref().filter(|d| d.path == doc.path) {
                    doc.keep_sorts(old, glyphs, params);
                }
                cmds.extend(nav::resolve_cmd(&doc));
                doc
            }
        };
        if let Some(old) = self.doc.replace(doc)
            && self.doc.as_ref().is_some_and(|d| d.path != old.path)
        {
            self.stash(old);
        }
        self.focus = None;
        self.scroll = 0;
        self.rematch();
        cmds.extend(self.highlight_all());
        cmds.extend(self.image_jobs());
        cmds.extend(self.git_cmd());
        cmds.extend(self.watch_cmd());
        cmds
    }

    /// Refresh the current document's gutter. Stdin has no file to diff, and an image no lines.
    fn git_cmd(&self) -> Option<Cmd> {
        let doc = self
            .doc
            .as_ref()
            .filter(|d| d.path.as_os_str() != "-" && d.filetype != FileType::Image)?;
        Some(Cmd::RunGit {
            doc_gen: doc.generation,
            path: doc.path.clone(),
        })
    }

    /// The current or a cached document, by its generation.
    fn doc_by_gen(&self, doc_gen: u64) -> Option<&Document> {
        self.doc
            .iter()
            .chain(self.cache.iter())
            .find(|d| d.generation == doc_gen)
    }

    fn doc_by_gen_mut(&mut self, doc_gen: u64) -> Option<&mut Document> {
        self.doc
            .iter_mut()
            .chain(self.cache.iter_mut())
            .find(|d| d.generation == doc_gen)
    }

    fn git_diff(&mut self, doc_gen: u64, total: &str, unstaged: &str, untracked: bool) {
        if let Some(doc) = self.doc_by_gen_mut(doc_gen) {
            doc.marks = Some(LineMarks::new(total, unstaged, untracked));
        }
    }

    fn jump_change(&mut self, dir: Dir) {
        let Some(doc) = &self.doc else {
            return;
        };
        let Some(marks) = &doc.marks else {
            self.status = Some("no git changes to jump to".into());
            return;
        };
        let starts = marks.change_starts(&doc.layout);
        let target = match dir {
            Dir::Next => starts.iter().find(|&&r| r > self.scroll),
            Dir::Prev => starts.iter().rev().find(|&&r| r < self.scroll),
        };
        match target {
            Some(&row) => self.scroll = row.min(self.max_scroll()),
            None if starts.is_empty() => self.status = Some("no changes".into()),
            None => {
                self.status = Some(match dir {
                    Dir::Next => "no more changes below".into(),
                    Dir::Prev => "no more changes above".into(),
                });
            }
        }
    }

    /// The first load, or one that isn't a navigation: no history.
    fn load(
        &mut self,
        path: PathBuf,
        mtime: Option<u64>,
        text: Arc<str>,
        prepared: Option<Prepared>,
    ) -> Vec<Cmd> {
        let cmds = self.install(path, mtime, text, prepared);
        if let (Some(line), Some(doc)) = (self.config.start_line.take(), &self.doc) {
            self.scroll = doc.layout.row_for_line(line).unwrap_or(0);
        }
        self.scroll = self.scroll.min(self.max_scroll());
        cmds
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
            first: None,
        }))
    }

    /// Jobs for every block not highlighted yet: the visible ones first, at the front of the
    /// worker's queue. Jobs for another document or theme are dropped.
    fn highlight_all(&mut self) -> Vec<Cmd> {
        let Some(doc) = &self.doc else {
            return Vec::new();
        };
        self.highlights.forget_pending();
        let mut cmds = vec![Cmd::HighlightOnly(Epoch {
            doc_gen: doc.generation,
            rules: self.syntax.id,
        })];
        let visible = self.visible_code();
        let rest = (0..doc.doc.code_count).filter(|b| !visible.contains(b));
        let blocks = doc.doc.code_blocks();
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
                first: None,
            }));
        }
        cmds
    }

    /// Work for what is on screen now: highlighting, and decoding images.
    fn visible_jobs(&mut self) -> Vec<Cmd> {
        let mut cmds = self.highlight_visible();
        cmds.extend(self.image_jobs());
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

    /// Keep a block's highlighting. A replay's marker carries none: it asks for the block
    /// again, as it was then, and the replay runs that job inline (ADR-19).
    fn highlighted(
        &mut self,
        doc_gen: u64,
        block: CodeId,
        lines: Option<u32>,
        spans: Option<Arc<CodeSpans>>,
    ) -> Option<Cmd> {
        match spans {
            // a result for the theme before the last switch has the wrong rules
            Some(spans) if spans.rules != self.syntax.id => None,
            Some(spans) => {
                self.highlights.insert(spans);
                None
            }
            // a replay: highlight again, if the block is still around
            None => {
                let doc = self.doc.as_ref().filter(|d| d.generation == doc_gen)?;
                let blocks = doc.doc.code_blocks();
                let code = blocks.get(block as usize)?;
                Some(Cmd::Highlight(Job {
                    doc_gen,
                    block,
                    key: highlight::key(code)?,
                    lang: code.lang.clone()?,
                    text: code.text.clone(),
                    theme: self.syntax.clone(),
                    urgent: true,
                    first: lines,
                }))
            }
        }
    }

    /// Shift no-wrap code and diagrams sideways. Diagrams can't wrap, so they scroll with `w`
    /// on too (ADR-35).
    fn scroll_code_x(&mut self, by: i16) {
        let mut blocks = self.visible_diagrams();
        if !self.code_wrap {
            blocks.extend(self.visible_code());
        }
        let Some(doc) = &self.doc else {
            return;
        };
        let max = blocks
            .iter()
            .filter_map(|&b| doc.layout.code.get(b as usize))
            .map(layout::CodeInfo::max_x)
            .max();
        match max {
            None | Some(0) if self.code_wrap => {
                self.status = Some("code wraps: press w to scroll it sideways".to_owned());
            }
            Some(max) => {
                let x = i32::from(self.code_x.min(max)) + i32::from(by);
                self.code_x = x.clamp(0, i32::from(max)) as u16;
            }
            None => {}
        }
    }

    /// The diagrams with a row on screen.
    fn visible_diagrams(&self) -> Vec<CodeId> {
        let Some(doc) = &self.doc else {
            return Vec::new();
        };
        let rows = doc.layout.rows.iter().skip(self.scroll as usize);
        let mut ids: Vec<CodeId> = rows
            .take(self.viewport_height() as usize)
            .filter_map(|row| match row.kind {
                RowKind::Diagram { block, .. } => Some(block),
                _ => None,
            })
            .collect();
        ids.dedup();
        ids
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

    /// Add the built-ins to the themes `T` lists, the first time it opens: parsed at every start,
    /// twice over, they cost it ~1.8 ms (O-42).
    fn list_themes(&mut self) {
        if !self.themes_listed {
            self.themes = Theme::with_builtins(&self.designed, mem::take(&mut self.themes));
            self.themes_listed = true;
        }
    }

    /// Switch to the theme named `name`, unless it's on already.
    fn switch_theme(&mut self, name: &str) -> Vec<Cmd> {
        if self.theme.name == name {
            return Vec::new();
        }
        match self.themes.iter().find(|t| t.name == name) {
            Some(theme) => self.set_theme(theme.clone()),
            None => Vec::new(),
        }
    }

    /// Switch themes. Colors resolve at paint, so only new glyphs lay the documents out again,
    /// and only new `[syntax]` rules highlight the code again (architecture §5).
    /// `theme` is as designed; `theme_bg` decides its background here.
    fn set_theme(&mut self, theme: Theme) -> Vec<Cmd> {
        let same_rules = self
            .theme
            .syntax
            .iter()
            .map(|(scope, _)| scope)
            .eq(theme.syntax.iter().map(|(scope, _)| scope));
        let glyphs_gen = theme.glyphs.generation();
        self.theme = theme.with_bg(self.theme_bg);
        self.designed = theme;
        let mut cmds = Vec::new();
        if !same_rules {
            self.syntax = Arc::new(SyntaxTheme::new(&self.theme.syntax));
            self.highlights = Highlights::default();
            cmds.extend(self.highlight_all());
        }
        if glyphs_gen != self.glyphs_gen {
            self.glyphs_gen = glyphs_gen;
            // the searchable text holds glyphs too
            for doc in self.doc.iter_mut().chain(self.cache.iter_mut()) {
                doc.search_map = None;
            }
            self.relayout();
            self.rematch();
        }
        cmds.extend(self.visible_jobs());
        cmds
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
#[derive(Clone, Debug, PartialEq, Eq)]
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
    // The image view takes the keys that close it, and `Enter`; any other intent closes it and
    // goes ahead, so the keymap's idea of the mode stays right.
    if state.view.is_some() {
        match &msg {
            Msg::Quit | Msg::Back => {
                state.view = None;
                state.status = None;
                return state.visible_jobs();
            }
            Msg::Follow => return state.view_external(),
            m if m.keeps_view() => {}
            _ => state.view = None,
        }
    }
    match msg {
        Msg::Init { size } | Msg::Resize { size } => {
            state.size = size;
            state.relayout();
            state.visible_jobs()
        }
        Msg::Graphics { proto, cell } => {
            let cell = crate::geom::Size::new(cell.w.max(1), cell.h.max(1));
            let first = state.graphics.replace(Graphics { proto, cell }).is_none();
            state.relayout();
            let mut cmds = state.visible_jobs();
            if first {
                cmds.extend(state.probe_cmds());
            }
            cmds
        }
        Msg::Open { path } => vec![state.load_cmd(crate::links::normalize(&path))],
        Msg::DocLoaded {
            path,
            mtime,
            text,
            doc,
        } => match state.nav.take() {
            Some(nav) if nav.path == path => state.arrive(nav, mtime, text, doc.0),
            _ => state.load(path, mtime, text, doc.0),
        },
        Msg::DocFailed { path, error } => {
            if state.nav.as_ref().is_some_and(|n| n.path == path) {
                state.nav = None;
            }
            state.status = Some(format!("{}: {error}", path.display()));
            Vec::new()
        }
        Msg::LinksResolved { doc_gen, resolved } => {
            state.links_resolved(doc_gen, &resolved);
            state.probe_cmd(doc_gen).into_iter().collect()
        }
        Msg::ImagesProbed { doc_gen, probes } => {
            state.images_probed(doc_gen, &probes);
            state.visible_jobs()
        }
        Msg::ImageReady {
            path,
            mtime,
            size,
            image,
        } => {
            let size = crate::geom::Size::new(size.w, size.h);
            state.image_ready(ImageKey { path, mtime, size }, image.0);
            Vec::new()
        }
        Msg::GitDiff {
            doc_gen,
            untracked,
            total,
            unstaged,
        } => {
            state.git_diff(doc_gen, &total, &unstaged, untracked);
            Vec::new()
        }
        Msg::JumpChange { dir } => {
            state.status = None;
            state.jump_change(dir);
            state.visible_jobs()
        }
        Msg::Reload => {
            state.status = None;
            state.reload()
        }
        Msg::FileChanged { path, mtime } => match &state.doc {
            Some(doc) if doc.path == path && doc.mtime != mtime => state.reload(),
            _ => Vec::new(),
        },
        Msg::Edit => {
            state.status = None;
            state.edit()
        }
        Msg::EditorExited { status } => state.editor_exited(status),
        Msg::Yank => {
            state.status = None;
            state.yank()
        }
        Msg::Help => {
            state.status = None;
            let cmds = state.outline_close();
            state.help = Some(0);
            cmds
        }
        Msg::HelpScroll { by } => {
            let max = i64::from(crate::ui::help_max_scroll(state));
            if let Some(help) = &mut state.help {
                *help = (i64::from(*help) + i64::from(by)).clamp(0, max) as u16;
            }
            Vec::new()
        }
        Msg::HelpClose => {
            state.help = None;
            state.visible_jobs()
        }
        Msg::SaveConfig => {
            state.status = None;
            state.save_config()
        }
        Msg::ConfigRead { path, text } => {
            state.config_read(path, &text);
            Vec::new()
        }
        Msg::SaveConfirm => state.save_confirm(),
        Msg::SaveCancel => {
            state.save = None;
            state.visible_jobs()
        }
        Msg::ConfigWritten { path } => {
            state.status = Some(format!("saved {}", path.display()));
            Vec::new()
        }
        Msg::ConfigFailed { error } => {
            state.status = Some(error);
            Vec::new()
        }
        Msg::OpenFailed { target, error } => {
            state.status = Some(format!("can't open {target}: {error}"));
            Vec::new()
        }
        Msg::FocusLink { dir } => {
            state.status = None;
            state.focus_link(dir);
            state.visible_jobs()
        }
        Msg::Follow => {
            state.status = None;
            let mut cmds = state.follow();
            cmds.extend(state.visible_jobs());
            cmds
        }
        Msg::Back => {
            state.status = None;
            let mut cmds = state.go_back();
            cmds.extend(state.visible_jobs());
            cmds
        }
        Msg::Forward => {
            state.status = None;
            let mut cmds = state.go_forward();
            cmds.extend(state.visible_jobs());
            cmds
        }
        Msg::OpenOutline => {
            state.status = None;
            state.open_outline(Listing::Headings)
        }
        Msg::OpenLinks => {
            state.status = None;
            state.open_outline(Listing::Links)
        }
        Msg::OpenThemes => {
            state.status = None;
            state.list_themes();
            state.open_outline(Listing::Themes)
        }
        Msg::ThemesTone { dir } => state.themes_tone(dir),
        Msg::OutlineFilter { text } => {
            state.outline_filter(text);
            state.preview_theme()
        }
        Msg::OutlineMove { by } => {
            state.outline_move(by);
            state.preview_theme()
        }
        Msg::OutlinePick => state.outline_pick(),
        Msg::OutlineClose => state.outline_close(),
        Msg::SearchOpen => {
            state.status = None;
            state.search_open()
        }
        Msg::SearchInput { pattern } => {
            state.search_input(pattern);
            state.visible_jobs()
        }
        Msg::SearchSubmit => {
            state.search_submit();
            state.visible_jobs()
        }
        Msg::SearchCancel => {
            state.search_cancel();
            state.visible_jobs()
        }
        Msg::SearchNext { dir } => {
            state.status = None;
            state.search_next(dir);
            state.visible_jobs()
        }
        Msg::Scroll { by } => {
            state.status = None;
            state.scroll_by(by);
            state.visible_jobs()
        }
        Msg::JumpHeading { dir } => {
            state.status = None;
            state.jump_heading(dir);
            state.visible_jobs()
        }
        Msg::ToggleTableMode => {
            state.table_mode = state.table_mode.next();
            state.status = Some(format!("tables: {}", state.table_mode.name()));
            state.relayout();
            state.visible_jobs()
        }
        Msg::SortOpen => {
            state.status = None;
            state.sort_open()
        }
        Msg::SortTable { dir } => {
            state.sort_table(dir);
            Vec::new()
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
            state.visible_jobs()
        }
        Msg::ToggleThemeBg => {
            state.theme_bg = !state.theme_bg;
            let bg = if state.theme_bg {
                "the theme's"
            } else {
                "the terminal's"
            };
            state.status = Some(format!("background: {bg}"));
            state.set_theme(state.designed.clone())
        }
        Msg::ToggleLineNumbers => {
            let code = state
                .doc
                .as_ref()
                .is_some_and(|d| matches!(d.filetype, FileType::Text { .. }));
            state.line_numbers = state.line_numbers.toggle(code);
            state.status = Some(format!("line numbers: {}", state.line_numbers.name()));
            state.relayout();
            state.visible_jobs()
        }
        Msg::ScrollCodeX { by } => {
            state.status = None;
            state.scroll_code_x(by);
            Vec::new()
        }
        Msg::Highlighted {
            doc_gen,
            block,
            lines,
            spans,
        } => state
            .highlighted(doc_gen, block, lines, spans.0)
            .into_iter()
            .collect(),
        Msg::Quit => {
            state.quit = true;
            vec![Cmd::Quit]
        }
    }
}

/// The file a lone `LoadDoc` among `cmds` reads.
#[cfg(test)]
fn only_load(cmds: &[Cmd]) -> Option<&str> {
    match cmds {
        [Cmd::LoadDoc { path, .. }] => path.to_str(),
        _ => None,
    }
}

/// What the loader posts for a navigation's lone `LoadDoc` among `cmds`, when the file holds
/// `text`.
#[cfg(test)]
fn loader_answer(cmds: &[Cmd], text: &str) -> Msg {
    let [
        Cmd::LoadDoc {
            path, prepare: how, ..
        },
    ] = cmds
    else {
        panic!("no navigation's load in {cmds:?}");
    };
    let loaded = Msg::DocLoaded {
        path: path.clone(),
        mtime: None,
        text: text.into(),
        doc: Computed(None),
    };
    prepare(loaded, how)
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
                doc: Computed(None),
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
                doc: Computed(None),
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

    /// Apply `msg`, then run the jobs it asks for inline, as a replay does.
    fn replay_step(state: &mut State, msg: Msg) {
        for cmd in update(state, msg) {
            let Cmd::Highlight(job) = cmd else {
                panic!("a marker asks only for highlighting: {cmd:?}");
            };
            assert!(update(state, job.run()).is_empty());
        }
    }

    #[test]
    fn replayed_highlights_are_recomputed() {
        let mut state = loaded("", 80, 10);
        load_cmds(&mut state, code_doc());
        let key = state.doc.as_ref().unwrap().layout.code[1].key.unwrap();
        let replayed = |doc_gen| Msg::Highlighted {
            doc_gen,
            block: 1,
            lines: None,
            spans: Computed(None),
        };
        assert!(
            update(&mut state, replayed(7)).is_empty(),
            "another document's block"
        );
        let doc_gen = state.generation;
        let cmds = update(&mut state, replayed(doc_gen));
        assert!(
            state.highlights.get(key).is_none(),
            "update leaves the work to the job"
        );
        assert!(
            matches!(&cmds[..], [Cmd::Highlight(job)] if job.key == key && job.first.is_none())
        );
        replay_step(&mut state, replayed(doc_gen));
        let spans = state.highlights.get(key).expect("recomputed");
        assert!(!spans.partial && !spans.line(0).is_empty());
    }

    /// A partial result's marker replays as that many lines (O-05).
    #[test]
    fn replayed_partial_highlights_keep_their_length() {
        let mut state = loaded("", 80, 10);
        load_cmds(&mut state, code_doc());
        let key = state.doc.as_ref().unwrap().layout.code[1].key.unwrap();
        let doc_gen = state.generation;
        replay_step(
            &mut state,
            Msg::Highlighted {
                doc_gen,
                block: 1,
                lines: Some(1),
                spans: Computed(None),
            },
        );
        let spans = state.highlights.get(key).expect("recomputed");
        assert!(spans.partial && spans.line_count() == 1);
    }

    /// Every switch of document or theme tells the worker which jobs are still wanted.
    #[test]
    fn loads_and_theme_switches_name_the_wanted_jobs() {
        let mut state = loaded("", 80, 10);
        let cmds = load_cmds(&mut state, code_doc());
        let only = |state: &State| {
            Cmd::HighlightOnly(Epoch {
                doc_gen: state.generation,
                rules: state.syntax.id,
            })
        };
        assert_eq!(cmds.first(), Some(&only(&state)));
        let other = Theme::builtins()
            .into_iter()
            .find(|t| SyntaxTheme::new(&t.syntax).id != state.syntax.id)
            .expect("a theme with other [syntax] rules");
        let cmds = state.set_theme(other);
        assert_eq!(cmds.first(), Some(&only(&state)));
    }

    #[test]
    fn b_trades_the_themes_background_for_the_terminals_across_switches() {
        let mut state = loaded(&code_doc(), 80, 10);
        let bg = |s: &State| s.theme.style(crate::theme::StyleKey::Text).bg;
        let code = |s: &State| s.theme.style(crate::theme::StyleKey::CodeBlock).bg;
        let (designed, code_bg) = (bg(&state), code(&state));
        assert!(state.theme_bg && designed.is_some());
        let (layout, rules) = (
            state.doc.as_ref().unwrap().layout.rows.len(),
            state.syntax.id,
        );

        // only a repaint: the same layout and highlighting, and code keeps its background
        update(&mut state, Msg::ToggleThemeBg);
        assert!(!state.theme_bg);
        assert_eq!(bg(&state), None);
        assert_eq!(code(&state), code_bg);
        assert_eq!(state.status.as_deref(), Some("background: the terminal's"));
        assert_eq!(state.doc.as_ref().unwrap().layout.rows.len(), layout);
        assert_eq!(state.syntax.id, rules);

        // a theme picked meanwhile comes without its background, and still counts as light
        update(&mut state, Msg::OpenThemes);
        update(&mut state, Msg::ThemesTone { dir: Dir::Next });
        update(&mut state, Msg::OutlinePick);
        assert_eq!(state.theme.name, "gruvbox-light");
        assert_eq!(bg(&state), None);
        update(&mut state, Msg::OpenThemes);
        assert!(state.outline.as_ref().unwrap().light);
        update(&mut state, Msg::OutlineClose);

        update(&mut state, Msg::ToggleThemeBg);
        assert!(state.theme_bg);
        assert!(bg(&state).is_some_and(|b| Some(b) != designed));
        assert!(state.active_settings().theme_bg);
    }

    /// Diagrams can't wrap, so they scroll sideways with code wrap on (ADR-35).
    #[test]
    fn diagrams_scroll_sideways_with_wrap_on() {
        let labels: String = (0..6)
            .map(|i| format!("P{i}->>P{}: message {i}\n", i + 1))
            .collect();
        let md = format!("```mermaid\nsequenceDiagram\n{labels}```\n");
        let mut state = loaded(&md, 40, 20);
        assert!(state.code_wrap);
        let max = state.doc.as_ref().unwrap().layout.code[0].max_x();
        assert!(max > 8, "the diagram is wider than the text: {max}");
        update(&mut state, Msg::ScrollCodeX { by: 8 });
        assert_eq!(state.code_x, 8);
        assert!(state.status.is_none());
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
            doc: Computed(None),
        };
        update(&mut state, msg.clone());
        assert_eq!(top_line(&state), 41);
        update(&mut state, msg);
        assert_eq!(state.scroll, 0);
    }

    // --- P4: links, history, search, outline -------------------------------------------------

    fn load_at(state: &mut State, path: &str, text: &str) -> Vec<Cmd> {
        update(
            state,
            Msg::DocLoaded {
                path: path.into(),
                mtime: None,
                text: text.into(),
                doc: Computed(None),
            },
        )
    }

    fn linked() -> String {
        let mut md = String::from("# A\n\nsee [b](b.md#part-two) and [[gone]] and [x](#later)\n\n");
        md.push_str(&sample());
        md.push_str("\n## Later\n\nthe end, with <https://example.com>\n");
        md
    }

    fn b_doc() -> String {
        let mut md = String::from("# B\n\n");
        md.push_str(&sample());
        md.push_str("\n## Part two\n\nback to [a](a.md)\n");
        md
    }

    /// `a.md` loaded and its links resolved: `b.md` exists, `gone.md` doesn't.
    fn walker() -> State {
        let mut state = loaded("", 80, 20);
        let cmds = load_at(&mut state, "a.md", &linked());
        let Some(Cmd::ResolveLinks {
            doc_gen, targets, ..
        }) = cmds
            .iter()
            .find(|c| matches!(c, Cmd::ResolveLinks { .. }))
            .cloned()
        else {
            panic!("no ResolveLinks in {cmds:?}");
        };
        assert_eq!(
            targets,
            [
                LocalTarget::Path("b.md".into()),
                LocalTarget::Wiki("gone".into())
            ]
        );
        update(
            &mut state,
            Msg::LinksResolved {
                doc_gen,
                resolved: vec![
                    Resolved::Found("b.md".into()),
                    Resolved::Broken("no such file: gone.md".into()),
                ],
            },
        );
        state
    }

    fn focused_dest(state: &State) -> Option<String> {
        let doc = state.doc.as_ref()?;
        Some(doc.doc.links[state.focus? as usize].dest.to_string())
    }

    fn path(state: &State) -> &str {
        state.doc.as_ref().unwrap().path.to_str().unwrap()
    }

    #[test]
    fn tab_cycles_links_from_the_top_of_the_viewport() {
        let mut state = walker();
        let tab = |s: &mut State, dir| update(s, Msg::FocusLink { dir });
        tab(&mut state, Dir::Next);
        assert_eq!(focused_dest(&state).as_deref(), Some("b.md#part-two"));
        assert_eq!(state.focus_text().as_deref(), Some("b.md#part-two"));
        tab(&mut state, Dir::Next);
        assert_eq!(focused_dest(&state).as_deref(), Some("gone"));
        assert!(state.focus_text().unwrap().contains("✗ no such file"));
        tab(&mut state, Dir::Next);
        tab(&mut state, Dir::Next);
        // the autolink at the end: scrolled into view
        assert_eq!(focused_dest(&state).as_deref(), Some("https://example.com"));
        assert_eq!(state.scroll, state.max_scroll());
        tab(&mut state, Dir::Next);
        assert_eq!(focused_dest(&state).as_deref(), Some("b.md#part-two"));
        assert_eq!(state.scroll, 2, "scrolled just enough to show it");
        assert!(state.status.as_deref().unwrap().contains("wrapped"));
        tab(&mut state, Dir::Prev);
        assert_eq!(focused_dest(&state).as_deref(), Some("https://example.com"));

        // with the focus scrolled away, Tab starts again from the top of the viewport
        update(&mut state, Msg::Scroll { by: ScrollBy::Top });
        tab(&mut state, Dir::Next);
        assert_eq!(focused_dest(&state).as_deref(), Some("b.md#part-two"));
    }

    #[test]
    fn follow_and_come_back() {
        let mut state = walker();
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Rows(1),
            },
        );
        update(&mut state, Msg::FocusLink { dir: Dir::Next });
        let a_gen = state.doc.as_ref().unwrap().generation;
        let cmds = update(&mut state, Msg::Follow);
        assert_eq!(only_load(&cmds), Some("b.md"));
        assert_eq!(path(&state), "a.md", "nothing changes until b.md arrives");

        load_at(&mut state, "b.md", &b_doc());
        assert_eq!(path(&state), "b.md");
        assert!(top_line(&state) >= 120, "landed on #part-two");
        assert_eq!(state.focus, None);

        assert_eq!(only_load(&update(&mut state, Msg::Back)), Some("a.md"));
        let cmds = load_at(&mut state, "a.md", &linked());
        assert!(
            !cmds.iter().any(|c| matches!(c, Cmd::ResolveLinks { .. })),
            "the cached document is reused"
        );
        let doc = state.doc.as_ref().unwrap();
        assert_eq!(doc.generation, a_gen);
        assert_eq!(state.scroll, 1);
        assert_eq!(focused_dest(&state).as_deref(), Some("b.md#part-two"));

        assert_eq!(only_load(&update(&mut state, Msg::Forward)), Some("b.md"));
        load_at(&mut state, "b.md", &b_doc());
        assert!(top_line(&state) >= 120);
        update(&mut state, Msg::Forward);
        assert_eq!(state.status.as_deref(), Some("nothing to go forward to"));
    }

    #[test]
    fn a_changed_document_is_parsed_again() {
        let mut state = walker();
        update(&mut state, Msg::FocusLink { dir: Dir::Next });
        update(&mut state, Msg::Follow);
        load_at(&mut state, "b.md", &b_doc());
        update(&mut state, Msg::Back);
        let cmds = load_at(&mut state, "a.md", &format!("{}\nmore\n", linked()));
        assert!(cmds.iter().any(|c| matches!(c, Cmd::ResolveLinks { .. })));
    }

    #[test]
    fn failed_navigation_leaves_history_alone() {
        let mut state = walker();
        update(&mut state, Msg::FocusLink { dir: Dir::Next });
        update(&mut state, Msg::Follow);
        update(
            &mut state,
            Msg::DocFailed {
                path: "b.md".into(),
                error: "gone".into(),
            },
        );
        assert_eq!(path(&state), "a.md");
        update(&mut state, Msg::Back);
        assert_eq!(state.status.as_deref(), Some("nothing to go back to"));
    }

    #[test]
    fn anchors_broken_links_and_external_ones() {
        let mut state = walker();
        for _ in 0..3 {
            update(&mut state, Msg::FocusLink { dir: Dir::Next });
        }
        assert_eq!(focused_dest(&state).as_deref(), Some("#later"));
        assert!(update(&mut state, Msg::Follow).is_empty());
        let row = state.scroll;
        assert!(row > 0, "jumped to ## Later");
        update(&mut state, Msg::Back);
        assert_eq!(state.scroll, 0);
        assert_eq!(focused_dest(&state).as_deref(), Some("#later"));

        update(&mut state, Msg::FocusLink { dir: Dir::Prev });
        update(&mut state, Msg::Follow);
        assert_eq!(
            state.status.as_deref(),
            Some("broken link: no such file: gone.md")
        );

        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Bottom,
            },
        );
        update(&mut state, Msg::FocusLink { dir: Dir::Prev });
        assert_eq!(
            update(&mut state, Msg::Follow),
            [Cmd::OpenWithSystem("https://example.com".into())]
        );

        update(&mut state, Msg::Scroll { by: ScrollBy::Top });
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Rows(30),
            },
        );
        update(&mut state, Msg::Follow);
        assert!(
            state
                .status
                .as_deref()
                .unwrap()
                .starts_with("no link focused")
        );
    }

    fn search(state: &mut State, pattern: &str) {
        update(state, Msg::SearchOpen);
        update(
            state,
            Msg::SearchInput {
                pattern: pattern.into(),
            },
        );
    }

    fn counter(state: &State) -> String {
        state
            .search
            .as_ref()
            .map(Search::counter)
            .unwrap_or_default()
    }

    #[test]
    fn incremental_search_and_cancel() {
        let mut state = loaded(&sample(), 80, 20);
        search(&mut state, "paragraph 4");
        // paragraph 4, then 40..=49
        assert_eq!(counter(&state), "1/11");
        search(&mut state, "paragraph 5");
        update(
            &mut state,
            Msg::SearchInput {
                pattern: "paragraph 50".into(),
            },
        );
        assert_eq!(counter(&state), "1/1");
        assert!(state.scroll > 0, "scrolled to the match");
        update(&mut state, Msg::SearchCancel);
        assert_eq!(state.scroll, 0);
        assert_eq!(counter(&state), "1/11", "the previous search is back");

        search(&mut state, "paragraph (");
        assert!(state.prompt.as_ref().unwrap().error.is_some());
        assert_eq!(
            counter(&state),
            "",
            "no search while the prompt shows an error"
        );
    }

    #[test]
    fn matches_in_a_wide_diagram_scroll_it_sideways() {
        let names = [
            "Alpha", "Bravo", "Charlie", "Delta", "Echo", "Foxtrot", "Golf", "Omega",
        ];
        let mut md = String::from("# Wide\n\n```mermaid\nsequenceDiagram\n");
        for pair in names.windows(2) {
            md += &format!("    {} ->> {}: hello\n", pair[0], pair[1]);
        }
        md += "```\n";
        let mut state = loaded(&md, 40, 20);
        let info = state.doc.as_ref().unwrap().layout.code[0];
        assert!(info.max_x() > 0, "it scrolls: {info:?}");
        search(&mut state, "Omega");
        assert_eq!(counter(&state), "1/1");
        assert!(state.code_x > 0, "Omega is off to the right");
        let x = state.code_x;
        search(&mut state, "Alpha");
        assert_eq!(state.code_x, 0, "back from {x}");
    }

    #[test]
    fn n_steps_through_matches_and_survives_resize() {
        let mut state = loaded(&sample(), 80, 20);
        search(&mut state, "heading 1\\b");
        update(&mut state, Msg::SearchSubmit);
        assert_eq!(counter(&state), "1/1");
        search(&mut state, "Heading 1");
        update(&mut state, Msg::SearchSubmit);
        // 1, 10..=19
        assert_eq!(counter(&state), "1/11");
        update(&mut state, Msg::SearchNext { dir: Dir::Next });
        update(&mut state, Msg::SearchNext { dir: Dir::Next });
        assert_eq!(counter(&state), "3/11");
        update(
            &mut state,
            Msg::Resize {
                size: Size { w: 41, h: 20 },
            },
        );
        assert_eq!(counter(&state), "3/11");
        update(&mut state, Msg::SearchNext { dir: Dir::Prev });
        update(&mut state, Msg::SearchNext { dir: Dir::Prev });
        update(&mut state, Msg::SearchNext { dir: Dir::Prev });
        assert_eq!(counter(&state), "11/11");
        assert!(state.status.as_deref().unwrap().contains("wrapped"));

        // an empty pattern repeats the last search
        update(&mut state, Msg::Scroll { by: ScrollBy::Top });
        update(&mut state, Msg::SearchOpen);
        update(&mut state, Msg::SearchSubmit);
        assert_eq!(counter(&state), "1/11");
    }

    #[test]
    fn hidden_matches_explain_themselves() {
        let mut state = walker();
        search(&mut state, "part-t");
        assert_eq!(counter(&state), "1/1");
        let (kind, text, range) = state.hidden_match().expect("a hidden match");
        assert_eq!(
            (kind, text, &text[range]),
            ("link", "b.md#part-two", "part-t")
        );
        search(&mut state, "see");
        assert_eq!(state.hidden_match(), None);
    }

    #[test]
    fn search_follows_navigation() {
        let mut state = walker();
        search(&mut state, "part two");
        update(&mut state, Msg::SearchSubmit);
        assert_eq!(counter(&state), "0");
        update(&mut state, Msg::FocusLink { dir: Dir::Next });
        update(&mut state, Msg::Follow);
        load_at(&mut state, "b.md", &b_doc());
        assert_eq!(state.search.as_ref().unwrap().projection.order.len(), 1);
    }

    #[test]
    fn outline_filters_and_jumps() {
        let mut state = loaded(&sample(), 80, 20);
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Rows(30),
            },
        );
        update(&mut state, Msg::OpenOutline);
        let selected = |s: &State| {
            s.outline_entries()[s.outline.as_ref().unwrap().selected]
                .text
                .to_string()
        };
        let current = selected(&state);
        assert!(current.starts_with("Heading "), "{current}");
        // every word, anywhere: 4, 14, 24, 34, 40..=49, 54
        update(
            &mut state,
            Msg::OutlineFilter {
                text: "heading 4".into(),
            },
        );
        assert_eq!(state.outline_entries().len(), 15);
        update(&mut state, Msg::OutlineMove { by: 3 });
        assert_eq!(selected(&state), "Heading 34");
        update(
            &mut state,
            Msg::OutlineFilter {
                text: "heading 34".into(),
            },
        );
        assert_eq!(
            selected(&state),
            "Heading 34",
            "the selection survives filtering"
        );
        update(&mut state, Msg::OutlinePick);
        assert!(state.outline.is_none());
        let heading = state.doc.as_ref().unwrap().layout.headings[33].row;
        assert_eq!(state.scroll, heading.min(state.max_scroll()));
        update(&mut state, Msg::Back);
        assert_eq!(state.scroll, 30);
    }

    #[test]
    fn the_built_in_themes_are_parsed_when_the_theme_list_first_opens() {
        let mut state = loaded("# Themes\n", 80, 20);
        assert!(state.themes.is_empty());
        update(&mut state, Msg::OpenThemes);
        let names: Vec<&str> = state.themes.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names.len(), crate::theme::BUILTIN.len());
        assert_eq!(names[0], "gruvbox-dark");
        update(&mut state, Msg::OutlineClose);
        update(&mut state, Msg::OpenThemes);
        assert_eq!(state.themes.len(), crate::theme::BUILTIN.len());
    }

    #[test]
    fn theme_list_previews_and_keeps_only_the_pick() {
        let mut state = loaded("# Themes\n", 80, 20);
        let name = |s: &State| s.theme.name.clone();
        let (dark, light) = ("gruvbox-dark".to_owned(), "gruvbox-light".to_owned());
        assert_eq!(name(&state), dark);

        // the selection is on show, and `Esc` puts the old theme back
        update(&mut state, Msg::OpenThemes);
        assert_eq!(state.outline_entries()[0].dest.as_deref(), Some("current"));
        update(&mut state, Msg::ThemesTone { dir: Dir::Next });
        assert_eq!(name(&state), light);
        update(&mut state, Msg::OutlineClose);
        assert_eq!(name(&state), dark);

        // a filter matching nothing shows the old theme, and so does picking nothing
        update(&mut state, Msg::OpenThemes);
        update(
            &mut state,
            Msg::OutlineFilter {
                text: "mocha".into(),
            },
        );
        assert_eq!(name(&state), "catppuccin-mocha");
        update(
            &mut state,
            Msg::OutlineFilter {
                text: "nope".into(),
            },
        );
        assert_eq!(name(&state), dark);
        update(&mut state, Msg::OutlinePick);
        assert_eq!(name(&state), dark);

        // the help closes the list without picking
        update(&mut state, Msg::OpenThemes);
        update(&mut state, Msg::OutlineMove { by: 1 });
        update(&mut state, Msg::Help);
        assert_eq!(name(&state), dark);
        update(&mut state, Msg::HelpClose);

        update(&mut state, Msg::OpenThemes);
        update(&mut state, Msg::ThemesTone { dir: Dir::Next });
        update(&mut state, Msg::OutlinePick);
        assert_eq!(name(&state), light);
        assert_eq!(state.status.as_deref(), Some("theme: gruvbox-light"));
        update(&mut state, Msg::OpenThemes);
        let current = &state.outline_entries()[state.outline.as_ref().unwrap().selected];
        assert_eq!(
            current.text, "gruvbox-light",
            "the list opens on the current theme"
        );
        assert!(
            state.outline.as_ref().unwrap().light,
            "among the light themes"
        );
    }

    #[test]
    fn theme_list_trades_dark_for_light_at_the_counterpart() {
        let mut state = loaded("# Themes\n", 80, 20);
        let name = |s: &State| s.theme.name.clone();
        let tone = |state: &mut State, dir| {
            update(state, Msg::ThemesTone { dir });
            name(state)
        };
        update(&mut state, Msg::OpenThemes);
        let listed = |s: &State| {
            s.outline_entries()
                .iter()
                .map(|e| e.text.to_string())
                .collect::<Vec<_>>()
        };
        assert!(!listed(&state).contains(&"gruvbox-light".to_owned()));
        assert_eq!(tone(&mut state, Dir::Prev), "gruvbox-dark", "already dark");

        // the counterpart shares the most words of the name; the first of equals wins
        update(
            &mut state,
            Msg::OutlineFilter {
                text: "moon".into(),
            },
        );
        assert_eq!(name(&state), "tokyonight-moon");
        update(
            &mut state,
            Msg::OutlineFilter {
                text: String::new(),
            },
        );
        update(&mut state, Msg::OutlineMove { by: 2 });
        assert_eq!(name(&state), "rose-pine-moon");
        assert_eq!(tone(&mut state, Dir::Next), "rose-pine-dawn");
        assert_eq!(
            listed(&state),
            [
                "gruvbox-light",
                "catppuccin-latte",
                "tokyonight-day",
                "rose-pine-dawn"
            ]
        );
        assert_eq!(tone(&mut state, Dir::Prev), "rose-pine");
        update(&mut state, Msg::OutlineMove { by: 2 });
        assert_eq!(name(&state), "dracula");
        assert_eq!(
            tone(&mut state, Dir::Next),
            "gruvbox-light",
            "no counterpart: the first"
        );

        // the filter stays, and `Esc` still puts the old theme back
        update(&mut state, Msg::OutlineFilter { text: "cat".into() });
        assert_eq!(name(&state), "catppuccin-latte");
        assert_eq!(tone(&mut state, Dir::Prev), "catppuccin-mocha");
        assert_eq!(listed(&state).len(), 3);
        update(&mut state, Msg::OutlineClose);
        assert_eq!(name(&state), "gruvbox-dark");
    }

    #[test]
    fn link_list_filters_and_focuses() {
        let mut state = walker();
        update(&mut state, Msg::OpenLinks);
        let shown: Vec<_> = state
            .outline_entries()
            .iter()
            .map(|e| (e.text.to_string(), e.dest.clone(), e.broken))
            .collect();
        assert_eq!(
            shown,
            [
                ("b".into(), Some("b.md#part-two".into()), false),
                ("gone".into(), None, true),
                ("x".into(), Some("#later".into()), false),
                ("https://example.com".into(), None, false),
            ]
        );
        // the destination counts for the filter
        update(
            &mut state,
            Msg::OutlineFilter {
                text: "later".into(),
            },
        );
        assert_eq!(state.outline_entries().len(), 1);
        update(
            &mut state,
            Msg::OutlineFilter {
                text: String::new(),
            },
        );
        update(&mut state, Msg::OutlineMove { by: 3 });
        update(&mut state, Msg::OutlinePick);
        assert!(state.outline.is_none());
        assert_eq!(focused_dest(&state).as_deref(), Some("https://example.com"));
        assert!(state.scroll > 0, "scrolled to the last link");
        // reopening starts at the focused link; picking one on screen doesn't scroll
        update(&mut state, Msg::OpenLinks);
        assert_eq!(state.outline.as_ref().unwrap().selected, 3);
        let scroll = state.scroll;
        update(&mut state, Msg::OutlinePick);
        assert_eq!(state.scroll, scroll);
        update(&mut state, Msg::Back);
        assert_eq!(state.scroll, 0);
    }

    // --- P5: the git gutter and reload --------------------------------------------------------

    fn git_diff(state: &mut State, total: &str, unstaged: &str) {
        let doc_gen = state.doc.as_ref().unwrap().generation;
        update(
            state,
            Msg::GitDiff {
                doc_gen,
                untracked: false,
                total: total.into(),
                unstaged: unstaged.into(),
            },
        );
    }

    #[test]
    fn loads_ask_git_and_stdin_does_not() {
        let mut state = loaded("", 80, 20);
        let cmds = load_at(&mut state, "notes/a.md", "# a\n");
        assert!(cmds.contains(&Cmd::RunGit {
            doc_gen: state.generation,
            path: "notes/a.md".into()
        }));
        let cmds = load_at(&mut state, "-", "# a\n");
        assert!(!cmds.iter().any(|c| matches!(c, Cmd::RunGit { .. })));
    }

    #[test]
    fn change_jumps() {
        let mut state = loaded(&sample(), 80, 20);
        update(&mut state, Msg::JumpChange { dir: Dir::Next });
        assert_eq!(state.status.as_deref(), Some("no git changes to jump to"));
        // a staged heading near the top, unstaged edits further down
        git_diff(
            &mut state,
            "@@ -1 +1 @@\n",
            "@@ -41 +41 @@\n@@ -101,0 +102,2 @@\n",
        );
        let doc = state.doc.as_ref().unwrap();
        let rows = |line| doc.layout.row_for_line(line).unwrap();
        let (a, b) = (rows(41), rows(102));
        update(&mut state, Msg::JumpChange { dir: Dir::Next });
        assert_eq!(state.scroll, a);
        update(&mut state, Msg::JumpChange { dir: Dir::Next });
        assert_eq!(state.scroll, b.min(state.max_scroll()));
        update(&mut state, Msg::JumpChange { dir: Dir::Prev });
        assert_eq!(state.scroll, a);
        update(&mut state, Msg::JumpChange { dir: Dir::Prev });
        assert_eq!(state.scroll, 0);
        update(&mut state, Msg::JumpChange { dir: Dir::Prev });
        assert_eq!(state.status.as_deref(), Some("no more changes above"));
    }

    #[test]
    fn reload_keeps_the_place_and_asks_git_again() {
        let mut state = loaded("", 80, 20);
        load_at(&mut state, "a.md", &sample());
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Rows(40),
            },
        );
        let line = top_line(&state);
        let doc_gen = state.generation;

        // unchanged text: the same document, and only git runs
        assert_eq!(only_load(&update(&mut state, Msg::Reload)), Some("a.md"));
        let cmds = load_at(&mut state, "a.md", &sample());
        assert_eq!(
            cmds,
            [
                Cmd::RunGit {
                    doc_gen,
                    path: "a.md".into()
                },
                Cmd::Watch {
                    path: "a.md".into(),
                    mtime: None
                }
            ]
        );
        assert_eq!(top_line(&state), line);
        assert_eq!(state.status.as_deref(), Some("reloaded"));

        // edited above the viewport: parsed again, anchored to the same source line number
        update(&mut state, Msg::Reload);
        let edited = format!("# New title\n\nNew intro.\n\n{}", sample());
        let cmds = load_at(&mut state, "a.md", &edited);
        assert!(cmds.iter().any(|c| matches!(c, Cmd::RunGit { .. })));
        assert!(state.generation > doc_gen);
        assert_eq!(top_line(&state), line);
        update(&mut state, Msg::Back);
        assert_eq!(
            state.status.as_deref(),
            Some("nothing to go back to"),
            "no history"
        );
    }

    // --- O-32: loading off the UI thread -------------------------------------------------------

    fn is_prepared(msg: &Msg) -> bool {
        matches!(
            msg,
            Msg::DocLoaded {
                doc: Computed(Some(_)),
                ..
            }
        )
    }

    #[test]
    fn a_prepared_document_is_the_one_update_would_parse() {
        let follow = || {
            let mut state = walker();
            update(&mut state, Msg::FocusLink { dir: Dir::Next });
            let cmds = update(&mut state, Msg::Follow);
            // the screen narrows while b.md loads
            update(
                &mut state,
                Msg::Resize {
                    size: Size { w: 50, h: 20 },
                },
            );
            (state, cmds)
        };
        let (mut inline, _) = follow();
        load_at(&mut inline, "b.md", &b_doc());
        let (mut state, cmds) = follow();
        let msg = loader_answer(&cmds, &b_doc());
        assert!(is_prepared(&msg));
        update(&mut state, msg);

        let (doc, want) = (state.doc.as_ref().unwrap(), inline.doc.as_ref().unwrap());
        assert_eq!(doc.layout.params.width, 50);
        assert_eq!(format!("{:?}", doc.layout), format!("{:?}", want.layout));
        assert_eq!(doc.generation, want.generation);
        assert_eq!(state.scroll, inline.scroll);
        assert!(top_line(&state) >= 120, "landed on #part-two");
        assert_eq!(
            update(&mut state, Msg::Back),
            update(&mut inline, Msg::Back)
        );
    }

    #[test]
    fn a_reload_lands_where_the_document_is_when_it_arrives() {
        let mut state = loaded("", 80, 20);
        load_at(&mut state, "a.md", &sample());
        let cmds = update(&mut state, Msg::Reload);
        // still on screen while it loads, and still scrolling
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Rows(40),
            },
        );
        let line = top_line(&state);
        let edited = format!("# New title\n\nNew intro.\n\n{}", sample());
        update(&mut state, loader_answer(&cmds, &edited));
        assert_eq!(top_line(&state), line);
        assert_eq!(state.status.as_deref(), Some("reloaded"));
    }

    #[test]
    fn the_loader_leaves_text_ada_has_to_update() {
        let mut state = walker();
        let cmds = update(&mut state, Msg::Reload);
        assert!(!is_prepared(&loader_answer(&cmds, &linked())), "on screen");
        load_at(&mut state, "a.md", &linked());

        update(&mut state, Msg::FocusLink { dir: Dir::Next });
        let cmds = update(&mut state, Msg::Follow);
        update(&mut state, loader_answer(&cmds, &b_doc()));
        let cmds = update(&mut state, Msg::Back);
        assert!(!is_prepared(&loader_answer(&cmds, &linked())), "cached");
        assert!(is_prepared(&loader_answer(&cmds, &b_doc())), "changed");
    }

    // --- P6: images ---------------------------------------------------------------------------

    use crate::images::{Entry, Job};

    fn pets_md() -> String {
        let mut md = String::from("# Pets\n\n![Ada](ada.png)\n\n");
        md.push_str(&sample());
        md.push_str("\nsee [the photo](neumann.jpeg) and [notes](n.md)\n");
        md
    }

    /// `p.md` loaded and its links resolved, in an 80×20 terminal that shows images when
    /// `graphics` is set. Returns the commands the links' resolution produced.
    fn pets(graphics: bool) -> (State, Vec<Cmd>) {
        let mut state = loaded("", 80, 20);
        if graphics {
            update(
                &mut state,
                Msg::Graphics {
                    proto: Proto::Halfblocks,
                    cell: Size { w: 10, h: 20 },
                },
            );
        }
        load_at(&mut state, "p.md", &pets_md());
        let doc_gen = state.generation;
        let resolved = ["ada.png", "neumann.jpeg", "n.md"]
            .map(|p| Resolved::Found(p.into()))
            .to_vec();
        let cmds = update(&mut state, Msg::LinksResolved { doc_gen, resolved });
        (state, cmds)
    }

    fn decodes(cmds: &[Cmd]) -> Vec<(&str, (u16, u16), bool)> {
        cmds.iter()
            .filter_map(|c| match c {
                Cmd::DecodeImage(Job { key, urgent }) => Some((
                    key.path.to_str().unwrap(),
                    (key.size.width, key.size.height),
                    *urgent,
                )),
                _ => None,
            })
            .collect()
    }

    /// [`pets`] with the image headers read: Ada is 300×200, the photo 2000×3000.
    fn pets_probed() -> (State, Vec<Cmd>) {
        let (mut state, cmds) = pets(true);
        let doc_gen = state.generation;
        assert_eq!(
            cmds,
            [Cmd::ProbeImages {
                doc_gen,
                paths: vec!["ada.png".into(), "neumann.jpeg".into()]
            }]
        );
        let probes = vec![
            Probe::Image {
                w: 300,
                h: 200,
                mtime: Some(7),
            },
            Probe::Image {
                w: 2000,
                h: 3000,
                mtime: Some(8),
            },
        ];
        let cmds = update(&mut state, Msg::ImagesProbed { doc_gen, probes });
        (state, cmds)
    }

    #[test]
    fn block_images_are_sized_and_decoded_near_the_viewport() {
        let (mut state, cmds) = pets_probed();
        let slot = state.doc.as_ref().unwrap().layout.images[0];
        assert_eq!((slot.size.width, slot.size.height), (30, 10));
        assert_eq!(decodes(&cmds), [("ada.png", (30, 10), true)]);
        // asked once: scrolling doesn't ask again
        assert!(
            decodes(&update(
                &mut state,
                Msg::Scroll {
                    by: ScrollBy::Rows(1)
                }
            ))
            .is_empty()
        );

        let key = ImageKey {
            path: "ada.png".into(),
            mtime: Some(7),
            size: crate::geom::Size::new(30, 10),
        };
        assert_eq!(state.images.get(&key), Some(&Entry::Pending));
        let ready = |image| Msg::ImageReady {
            path: "ada.png".into(),
            mtime: Some(7),
            size: Size { w: 30, h: 10 },
            image: Computed(image),
        };
        // a replay has no image: the placeholder stays
        update(&mut state, ready(None));
        assert_eq!(state.images.get(&key), Some(&Entry::Pending));
        update(&mut state, ready(Some(Err("corrupt".into()))));
        assert_eq!(
            state.images.get(&key),
            Some(&Entry::Failed("corrupt".into()))
        );
    }

    #[test]
    fn the_image_view() {
        let (mut state, _) = pets_probed();
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Bottom,
            },
        );
        update(&mut state, Msg::FocusLink { dir: Dir::Next });
        assert_eq!(focused_dest(&state).as_deref(), Some("neumann.jpeg"));
        let cmds = update(&mut state, Msg::Follow);
        assert!(state.view.is_some());
        // 200×150 cells, cut to the 19 rows above the status bar
        assert_eq!(decodes(&cmds), [("neumann.jpeg", (26, 19), true)]);

        // Enter again hands it to the system's viewer
        assert_eq!(
            update(&mut state, Msg::Follow),
            [Cmd::OpenWithSystem("neumann.jpeg".into())]
        );
        assert!(state.view.is_some());
        // q and Esc close the view, not ada
        update(&mut state, Msg::Quit);
        assert!(state.view.is_none() && !state.quit);

        // a resize keeps it open and fits it again; any other intent closes it and goes ahead
        update(&mut state, Msg::Follow);
        let cmds = update(
            &mut state,
            Msg::Resize {
                size: Size { w: 80, h: 41 },
            },
        );
        assert!(state.view.is_some());
        assert_eq!(decodes(&cmds), [("neumann.jpeg", (54, 40), true)]);
        let scroll = state.scroll;
        update(&mut state, Msg::Scroll { by: ScrollBy::Top });
        assert!(state.view.is_none());
        assert!(state.scroll < scroll);
    }

    #[test]
    fn without_graphics_images_are_text_and_open_externally() {
        let (mut state, cmds) = pets(false);
        assert!(cmds.is_empty(), "no headers to read");
        assert!(state.doc.as_ref().unwrap().layout.images.is_empty());
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Bottom,
            },
        );
        update(&mut state, Msg::FocusLink { dir: Dir::Next });
        assert_eq!(
            update(&mut state, Msg::Follow),
            [Cmd::OpenWithSystem("neumann.jpeg".into())]
        );
        assert!(state.view.is_none());
    }

    /// The probe answers after the first paint (O-06), often after the document's links are
    /// resolved: its headers are read then, once.
    #[test]
    fn graphics_after_the_links_read_the_headers_then() {
        let (mut state, cmds) = pets(false);
        assert!(cmds.is_empty());
        let graphics = Msg::Graphics {
            proto: Proto::Halfblocks,
            cell: Size { w: 10, h: 20 },
        };
        let cmds = update(&mut state, graphics.clone());
        assert_eq!(
            cmds,
            [Cmd::ProbeImages {
                doc_gen: state.generation,
                paths: vec!["ada.png".into(), "neumann.jpeg".into()]
            }]
        );
        assert!(update(&mut state, graphics).is_empty());
    }

    /// Until an image is measured, the images' fit changes no row: the probe's answer keeps the
    /// layout it finds, and the headers' sizes lay it out again.
    #[test]
    fn graphics_keep_the_layout_until_an_image_is_measured() {
        let (mut state, _) = pets(false);
        let rows = state.doc.as_ref().unwrap().layout.row_count();
        let graphics = Msg::Graphics {
            proto: Proto::Halfblocks,
            cell: Size { w: 10, h: 20 },
        };
        update(&mut state, graphics);
        let doc = state.doc.as_ref().unwrap();
        assert!(doc.prev.is_none(), "laid out again");
        assert_eq!(doc.layout.params, state.params());
        assert_eq!(doc.layout.row_count(), rows);
        let doc_gen = state.generation;
        let probes = vec![
            Probe::Image {
                w: 300,
                h: 200,
                mtime: Some(7),
            },
            Probe::Failed("gone".into()),
        ];
        update(&mut state, Msg::ImagesProbed { doc_gen, probes });
        let doc = state.doc.as_ref().unwrap();
        assert_eq!(doc.layout.images.len(), 1);
        assert!(doc.layout.row_count() > rows);
        // measured: a height-only resize lays out again, for the images' fit
        update(
            &mut state,
            Msg::Resize {
                size: Size { w: 80, h: 30 },
            },
        );
        assert!(state.doc.as_ref().unwrap().prev.is_some());
    }

    // --- files that aren't markdown ------------------------------------------------------------

    fn rust_file() -> String {
        (1..=80)
            .map(|i| format!("fn f{i}() {{ /* {} */ }}\n", "x".repeat(i % 7 * 20)))
            .collect()
    }

    #[test]
    fn a_code_file_is_one_highlighted_block_as_wide_as_the_terminal() {
        let mut state = loaded("", 140, 20);
        state.config.max_width = Some(60);
        let cmds = load_at(&mut state, "src/lib.rs", &rust_file());
        let doc = state.doc.as_ref().unwrap();
        assert_eq!(doc.filetype, FileType::Text { lang: Some("Rust") });
        assert_eq!(doc.layout.params.max_width, None, "code isn't prose");
        let width = |r| {
            doc.layout
                .spans(r)
                .map(|(_, s)| u32::from(s.width))
                .sum::<u32>()
        };
        let widest = doc.layout.rows.iter().map(width).max();
        assert!(widest.unwrap() > 60, "{widest:?}");
        let langs: Vec<&str> = cmds
            .iter()
            .filter_map(|c| match c {
                Cmd::Highlight(job) => Some(&*job.lang),
                _ => None,
            })
            .collect();
        assert_eq!(langs, ["Rust"]);
        assert!(cmds.iter().any(|c| matches!(c, Cmd::RunGit { .. })));

        // the source lines are the file's own, for the gutter, `e` and `--line`
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Rows(30),
            },
        );
        let line = top_line(&state);
        assert!(line > 1);
        assert_eq!(
            update(&mut state, Msg::Edit),
            [Cmd::SpawnEditor {
                file: "src/lib.rs".into(),
                line
            }]
        );
    }

    #[test]
    fn hash_numbers_the_lines_of_the_kind_of_file_shown() {
        let md = "# A\n\ntext\n";
        let mut state = loaded(md, 80, 20);
        let numbers = |s: &State| s.doc.as_ref().unwrap().layout.numbers;
        load_at(&mut state, "src/lib.rs", &rust_file());
        assert_eq!(numbers(&state), 0);

        update(&mut state, Msg::ToggleLineNumbers);
        assert_eq!(state.line_numbers, LineNumbers::Code);
        assert_eq!(state.status.as_deref(), Some("line numbers: code files"));
        assert_eq!(numbers(&state), 3, "80 lines: two digits and a space");

        // markdown isn't numbered in this mode; `#` numbers it too, then only code again
        load_at(&mut state, "a.md", md);
        assert_eq!(numbers(&state), 0);
        update(&mut state, Msg::ToggleLineNumbers);
        assert_eq!(state.line_numbers, LineNumbers::All);
        assert_eq!(numbers(&state), 2);
        update(&mut state, Msg::ToggleLineNumbers);
        assert_eq!(state.line_numbers, LineNumbers::Code);
        assert_eq!(numbers(&state), 0);

        // the cached code file comes back numbered, and `#` there turns them off
        load_at(&mut state, "src/lib.rs", &rust_file());
        assert_eq!(numbers(&state), 3);
        update(&mut state, Msg::ToggleLineNumbers);
        assert_eq!(state.line_numbers, LineNumbers::Off);
        assert_eq!(numbers(&state), 0);
    }

    #[test]
    fn links_to_code_open_in_ada_at_a_line() {
        let mut state = loaded("", 80, 20);
        let md = "# A\n\nsee [lib](src/lib.rs#L40) and [the pdf](x.pdf)\n";
        let cmds = load_at(&mut state, "a.md", md);
        let doc_gen = state.generation;
        assert!(cmds.iter().any(|c| matches!(c, Cmd::ResolveLinks { .. })));
        let resolved = ["src/lib.rs", "x.pdf"]
            .map(|p| Resolved::Found(p.into()))
            .to_vec();
        update(&mut state, Msg::LinksResolved { doc_gen, resolved });
        let links = &state.doc.as_ref().unwrap().links;
        assert!(matches!(&links[0], LinkState::Doc { frag: Some(f), .. } if f == "L40"));
        assert_eq!(links[1], LinkState::File("x.pdf".into()));

        update(&mut state, Msg::FocusLink { dir: Dir::Next });
        assert_eq!(
            only_load(&update(&mut state, Msg::Follow)),
            Some("src/lib.rs")
        );
        load_at(&mut state, "src/lib.rs", &rust_file());
        assert_eq!(path(&state), "src/lib.rs");
        assert_eq!(top_line(&state), 40);
        assert_eq!(state.status, None);
    }

    #[test]
    fn a_missing_line_says_so() {
        assert_eq!(nav::line_frag("L12-L20"), Some(12));
        assert_eq!(nav::line_frag("L0"), None);
        assert_eq!(nav::line_frag("Lists"), None);
        let mut state = loaded("", 80, 20);
        load_at(&mut state, "a.md", "[far](b.rs#L999)\n");
        let doc_gen = state.generation;
        let resolved = vec![Resolved::Found("b.rs".into())];
        update(&mut state, Msg::LinksResolved { doc_gen, resolved });
        update(&mut state, Msg::FocusLink { dir: Dir::Next });
        update(&mut state, Msg::Follow);
        load_at(&mut state, "b.rs", &rust_file());
        assert_eq!(path(&state), "b.rs");
        assert_eq!(
            (state.scroll, state.status.as_deref()),
            (0, Some("no line 999"))
        );
    }

    #[test]
    fn an_image_file_fills_the_screen() {
        let mut state = loaded("", 80, 20);
        update(
            &mut state,
            Msg::Graphics {
                proto: Proto::Halfblocks,
                cell: Size { w: 10, h: 20 },
            },
        );
        let cmds = load_at(&mut state, "pics/cat.png", "");
        let doc_gen = state.generation;
        assert_eq!(state.doc.as_ref().unwrap().filetype, FileType::Image);
        assert!(!cmds.iter().any(|c| matches!(c, Cmd::RunGit { .. })));
        assert!(cmds.contains(&Cmd::ResolveLinks {
            doc_gen,
            source: "pics/cat.png".into(),
            targets: vec![LocalTarget::Path("cat.png".into())],
        }));
        let resolved = vec![Resolved::Found("pics/cat.png".into())];
        let cmds = update(&mut state, Msg::LinksResolved { doc_gen, resolved });
        assert_eq!(
            cmds,
            [Cmd::ProbeImages {
                doc_gen,
                paths: vec!["pics/cat.png".into()]
            }]
        );
        let probes = vec![Probe::Image {
            w: 4000,
            h: 4000,
            mtime: Some(1),
        }];
        update(&mut state, Msg::ImagesProbed { doc_gen, probes });
        let slot = state.doc.as_ref().unwrap().layout.images[0];
        // 19 rows for the document, one of them the caption
        assert_eq!(slot.size.height, 18);
        update(&mut state, Msg::Edit);
        assert_eq!(state.status.as_deref(), Some("an image can't be edited"));
    }

    #[test]
    fn binary_files_aren_t_loaded() {
        let dir = std::env::temp_dir().join(format!("ada-binary-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("blob.dat");
        std::fs::write(&path, b"ELF\0\0\x01").unwrap();
        let msg = load_doc(&path, None);
        assert!(
            matches!(&msg, Msg::DocFailed { error, .. } if error == "binary file"),
            "{msg:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
