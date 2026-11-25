//! The document IR (ADR-01, ADR-02): an owned, markup-free block tree, parsed once per source
//! version. Nothing downstream of [`parse`] sees pulldown-cmark types.

mod comments;
pub mod embed;
mod fence;
pub mod file;
pub mod flavor;
pub mod front;
mod lines;
mod parse;
pub mod props;
pub mod slug;
pub mod tex;

use std::hash::BuildHasher;
use std::ops::BitOr;
use std::sync::Arc;

use crate::diagram::Diagram;

pub use embed::{Embed, Embedded, Embeds};
pub use fence::language as fence_language;
pub use flavor::{Dialect, Flavor, Vault};
pub use parse::{options, parse, parse_as, parse_with};

pub type LinkId = u32;
pub type AnchorId = u32;
pub type FootnoteId = u32;
pub type TableId = u32;
pub type CodeId = u32;
/// Typeset math, indexed into [`Doc::math`] (OBS-003).
pub type MathId = u32;
/// A searchable stretch of visible text: a paragraph, heading, table cell, image alt text or
/// code line (ADR-12). Numbered `0..Doc::unit_count` in source order.
pub type UnitId = u32;

/// 1-based, inclusive source lines. `start == 0` means the thing has no source (decorative).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LineRange {
    pub start: u32,
    pub end: u32,
}

impl LineRange {
    pub const NONE: LineRange = LineRange { start: 0, end: 0 };

    pub fn new(start: u32, end: u32) -> Self {
        Self {
            start,
            end: end.max(start),
        }
    }

    pub fn line(line: u32) -> Self {
        Self::new(line, line)
    }

    pub fn is_none(self) -> bool {
        self.start == 0
    }

    pub fn union(self, other: Self) -> Self {
        match (self.is_none(), other.is_none()) {
            (true, _) => other,
            (_, true) => self,
            _ => Self::new(self.start.min(other.start), self.end.max(other.end)),
        }
    }

    pub fn contains(self, line: u32) -> bool {
        !self.is_none() && (self.start..=self.end).contains(&line)
    }
}

#[derive(Debug, Default)]
pub struct Doc {
    /// Top-level blocks; nested blocks live inside the variants.
    pub blocks: Vec<Block>,
    /// Every link, image and footnote reference, indexed by [`LinkId`].
    pub links: Vec<LinkRef>,
    /// Headings and footnote definitions, indexed by [`AnchorId`].
    pub anchors: Vec<Anchor>,
    pub footnotes: Vec<Footnote>,
    /// Byte offset of the start of each line, for offset → line lookups.
    pub line_starts: Vec<u32>,
    /// Tables are numbered `0..table_count` in source order, wherever they are nested.
    pub table_count: u32,
    /// Code blocks are numbered `0..code_count` the same way.
    pub code_count: u32,
    /// Search units are numbered `0..unit_count`.
    pub unit_count: u32,
    /// Every run's text, in source order: a run holds its range (PAR-004), one heap block for
    /// the document's inline text instead of one a run. Typeset math's TeX is here too.
    pub text: String,
    /// The TeX of each run of typeset math, indexed by [`MathId`]: ranges of [`Doc::text`],
    /// searched behind the math (OBS-003). The rows of one line of display math share one.
    pub math: Vec<TextRange>,
    /// Every paragraph's, heading's and cell's runs, end to end: an [`Inlines`] holds its range
    /// (PAR-008), one heap block for the document's runs instead of one a paragraph or cell.
    pub runs: Vec<Run>,
    /// The notes transcluded into it (OBS-007), indexed by [`Embed::note`]: their links and
    /// anchors are ranges of the document's.
    pub embedded: Vec<Embedded>,
}

impl Doc {
    /// The note's properties: its front matter, read.
    pub fn properties(&self) -> Option<&Properties> {
        match &self.blocks.first()?.kind {
            BlockKind::Properties(p) => Some(p),
            _ => None,
        }
    }

    /// The `title` of a note without an H1 of its own, from its properties (ADR-46): what heads
    /// the outline and the heading path.
    pub fn title(&self) -> Option<&str> {
        let h1 = (self.anchors.iter().enumerate())
            .any(|(i, a)| a.kind == AnchorKind::Heading(1) && self.anchor_note(i as u32).is_none());
        self.properties()?.title.as_deref().filter(|_| !h1)
    }

    pub fn line_count(&self) -> u32 {
        self.line_starts.len() as u32
    }

    /// `run`'s text: what is rendered and searched.
    pub fn run_text(&self, run: &Run) -> &str {
        run.text.of(&self.text)
    }

    /// `inlines`' runs.
    pub fn runs(&self, inlines: &Inlines) -> &[Run] {
        inlines.runs.of(&self.runs)
    }

    /// `inlines`' visible text, with breaks as spaces.
    pub fn plain_text(&self, inlines: &Inlines) -> String {
        plain_text(self.runs(inlines), &self.text)
    }
}

#[derive(Debug)]
pub struct Block {
    pub kind: BlockKind,
    pub src: LineRange,
}

// a variant held in place grows every block, and the parse with it (OBS-010's properties)
const _: () = assert!(size_of::<Block>() == 104);

#[derive(Debug)]
pub enum BlockKind {
    Heading {
        level: u8,
        inlines: Inlines,
        anchor: AnchorId,
    },
    Paragraph(Inlines),
    List(List),
    Quote {
        callout: Option<Callout>,
        blocks: Vec<Block>,
    },
    Code(CodeBlock),
    Table(Table),
    /// A paragraph that consists of a single image.
    Image {
        link: LinkId,
        alt: Box<str>,
        /// The alt text's search unit.
        unit: UnitId,
    },
    Rule,
    Html(Box<str>),
    /// Where the footnote section renders.
    FootnoteDefs,
    /// Display math typeset (OBS-003): its rows as runs of [`RunKind::Math`] with hard breaks
    /// between them, set centered.
    Math(Inlines),
    /// The note's front matter, read (ADR-46, OBS-010): always its first block. Boxed: in place,
    /// its 128 bytes grew every block past them, from 104, and every note's parse took 1.7% more
    /// instructions, front matter or not.
    Properties(Box<Properties>),
    /// A note embedded and transcluded (OBS-007): a link to it, then its blocks, or the blocks
    /// of the heading's section it names. Boxed, as properties are.
    Embed(Box<Embed>),
}

/// A note's front matter read into rows of a key and its value, shown above the note, hidden or
/// as written, as [`crate::layout::PropertiesMode`] says (ADR-46, OBS-010).
#[derive(Debug)]
pub struct Properties {
    /// What it holds, but for the keys of Obsidian's own ([`props::is_hidden`]).
    pub rows: Vec<Property>,
    /// The block as written, between its fences: shown as code in `source` mode, and in place of
    /// the rows when ada couldn't read it.
    pub source: CodeBlock,
    /// Whether ada read it: its rows are what it holds.
    pub read: bool,
    /// Its `title`, which titles a note without an H1 ([`Doc::title`]).
    pub title: Option<Box<str>>,
}

/// A row of [`Properties`]: a key, and its value as runs, its tags styled and its links links.
#[derive(Debug)]
pub struct Property {
    pub key: Box<str>,
    pub value: Inlines,
    pub src: LineRange,
}

#[derive(Debug)]
pub struct List {
    /// The first number of an ordered list; `None` for bullets.
    pub start: Option<u64>,
    pub tight: bool,
    pub items: Vec<ListItem>,
}

#[derive(Debug)]
pub struct ListItem {
    pub task: Option<Task>,
    pub blocks: Vec<Block>,
    pub src: LineRange,
}

/// A list item's task box: GFM's `[ ]` and `[x]`, or a status Obsidian's themes and the Tasks
/// plugin draw past them (OBS-012).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    Open,
    Done,
    /// `[/]` in progress, `[-]` cancelled, `[>]` forwarded, `[<]` scheduled, `[!]` important,
    /// `[?]` a question, `[*]` a star: the character in the brackets, one of [`Task::STATUSES`].
    Status(char),
}

impl Task {
    /// The statuses read past GFM's: Obsidian takes any character for one, but a letter or a
    /// digit in brackets opens many a list of options or references (`- [a] first`, `- [1] Knuth`)
    /// in other markdown, and these have no other meaning there.
    pub const STATUSES: [char; 7] = ['/', '-', '>', '<', '!', '?', '*'];
}

/// A quote that opens with `[!kind]` (OBS-002): GitHub's alerts and Obsidian's callouts.
#[derive(Debug)]
pub struct Callout {
    pub kind: CalloutKind,
    /// What follows the marker on its line, else the kind as written, in title case.
    pub title: Inlines,
    /// Obsidian's `+` or `-` after the marker: a callout that folds. Shown open either way.
    pub fold: Option<Fold>,
}

/// A callout's kind: Obsidian's thirteen, then GitHub's two that Obsidian reads as aliases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalloutKind {
    Note,
    Abstract,
    Info,
    Todo,
    Tip,
    Success,
    Question,
    Warning,
    Failure,
    Danger,
    Bug,
    Example,
    Quote,
    Important,
    Caution,
}

impl CalloutKind {
    pub const ALL: [CalloutKind; 15] = [
        CalloutKind::Note,
        CalloutKind::Abstract,
        CalloutKind::Info,
        CalloutKind::Todo,
        CalloutKind::Tip,
        CalloutKind::Success,
        CalloutKind::Question,
        CalloutKind::Warning,
        CalloutKind::Failure,
        CalloutKind::Danger,
        CalloutKind::Bug,
        CalloutKind::Example,
        CalloutKind::Quote,
        CalloutKind::Important,
        CalloutKind::Caution,
    ];

    /// The kind a marker names, in any case: one of Obsidian's or an alias of one, and any
    /// other name a note, as in Obsidian. `important` and `caution` are GitHub's own kinds,
    /// except in an Obsidian note (`obsidian`), where they are tip and warning.
    pub fn of(name: &str, obsidian: bool) -> Self {
        let is = |s: &str| name.eq_ignore_ascii_case(s);
        let any = |names: &[&str]| names.iter().any(|s| is(s));
        match () {
            _ if any(&["abstract", "summary", "tldr"]) => CalloutKind::Abstract,
            _ if is("info") => CalloutKind::Info,
            _ if is("todo") => CalloutKind::Todo,
            _ if is("important") && !obsidian => CalloutKind::Important,
            _ if any(&["tip", "hint", "important"]) => CalloutKind::Tip,
            _ if any(&["success", "check", "done"]) => CalloutKind::Success,
            _ if any(&["question", "help", "faq"]) => CalloutKind::Question,
            _ if is("caution") && !obsidian => CalloutKind::Caution,
            _ if any(&["warning", "caution", "attention"]) => CalloutKind::Warning,
            _ if any(&["failure", "fail", "missing"]) => CalloutKind::Failure,
            _ if any(&["danger", "error"]) => CalloutKind::Danger,
            _ if is("bug") => CalloutKind::Bug,
            _ if is("example") => CalloutKind::Example,
            _ if any(&["quote", "cite"]) => CalloutKind::Quote,
            _ => CalloutKind::Note,
        }
    }

    /// Its name in a theme's `[glyphs.callout]`.
    pub fn name(self) -> &'static str {
        match self {
            CalloutKind::Note => "note",
            CalloutKind::Abstract => "abstract",
            CalloutKind::Info => "info",
            CalloutKind::Todo => "todo",
            CalloutKind::Tip => "tip",
            CalloutKind::Success => "success",
            CalloutKind::Question => "question",
            CalloutKind::Warning => "warning",
            CalloutKind::Failure => "failure",
            CalloutKind::Danger => "danger",
            CalloutKind::Bug => "bug",
            CalloutKind::Example => "example",
            CalloutKind::Quote => "quote",
            CalloutKind::Important => "important",
            CalloutKind::Caution => "caution",
        }
    }
}

/// How a foldable callout starts in Obsidian: `+` open, `-` folded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fold {
    Open,
    Folded,
}

#[derive(Debug)]
pub struct CodeBlock {
    /// Indexes per-document code data such as line widths and highlighting.
    pub id: CodeId,
    pub lang: Option<Box<str>>,
    pub text: Arc<str>,
    /// Content hash, the key of the highlight cache (P3).
    pub hash: u64,
    /// The source line of the first line of code.
    pub first_line: u32,
    pub fenced: bool,
    /// The whole of a file of code or text, or a piece of one, not a block in a note: it is
    /// drawn and printed as its lines alone, without the top row naming its language or the
    /// row closing it.
    pub whole_file: bool,
    /// The search unit of the first line; line `n` is unit `first_unit + n`.
    pub first_unit: UnitId,
    /// A ```` ```mermaid ```` block ada draws (ADR-35); `None` shows the source.
    pub diagram: Option<Arc<Diagram>>,
    /// The input as it came, escapes and all, of text in its own colors (FEAT-022,
    /// [`crate::highlight::ANSI`]): `text` is without them, and the highlighting reads them.
    pub colors: Option<Arc<str>>,
}

impl CodeBlock {
    /// The code's lines, without the final newline.
    pub fn lines(&self) -> impl Iterator<Item = &str> {
        code_lines(&self.text)
    }

    /// The text its highlighting reads: the input with its escapes, for text in its own colors.
    pub fn highlight_text(&self) -> &Arc<str> {
        self.colors.as_ref().unwrap_or(&self.text)
    }
}

/// The diagram a code block in `lang` draws: mermaid of a kind ada draws (ADR-35), parsed.
fn diagram(lang: Option<&str>, text: &str) -> Option<Arc<Diagram>> {
    (lang == Some(crate::diagram::LANG))
        .then(|| crate::guard::catch(|| crate::diagram::parse(text)).flatten())
        .flatten()
        .map(Arc::new)
}

/// A code block's content hash, the highlight cache's key: foldhash's quality hasher, about 6×
/// SipHash on code (experiment 65). The key needs no defense against crafted collisions, only
/// to tell texts apart, and a fixed seed keeps it the same from one run to the next.
pub fn content_hash(text: &str) -> u64 {
    foldhash::quality::FixedState::with_seed(0).hash_one(text)
}

/// The lines of a code block's text, without the final newline.
pub fn code_lines(text: &str) -> impl Iterator<Item = &str> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    Some(text)
        .filter(|t| !t.is_empty())
        .into_iter()
        .flat_map(|t| t.split('\n'))
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
}

#[derive(Debug)]
pub struct Table {
    /// Indexes per-document table data such as column measurements.
    pub id: TableId,
    pub aligns: Vec<Align>,
    pub header: TableRow,
    pub rows: Vec<TableRow>,
    /// Every row's cells, the header's first, end to end: a [`TableRow`] holds its range
    /// (PAR-008), one heap block for the table's cells instead of one a row.
    pub cells: Vec<Inlines>,
}

impl Table {
    /// The widest of the alignment row, the header and the body rows.
    pub fn column_count(&self) -> usize {
        self.aligns
            .len()
            .max(self.header.cells.len())
            .max(self.rows.iter().map(|r| r.cells.len()).max().unwrap_or(0))
    }

    /// `row`'s cells.
    pub fn cells(&self, row: &TableRow) -> &[Inlines] {
        row.cells.of(&self.cells)
    }
}

#[derive(Debug, Default)]
pub struct TableRow {
    /// A range of its table's [`Table::cells`] (read it with [`Table::cells`]).
    pub cells: Slots,
    pub src: LineRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

#[derive(Debug, Default)]
pub struct Inlines {
    /// A range of [`Doc::runs`] (read it with [`Doc::runs`]).
    pub runs: Slots,
    pub unit: UnitId,
}

/// Items `start..end` of a list the IR keeps end to end: a document's runs or a table's cells
/// (PAR-008).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Slots {
    pub start: u32,
    pub end: u32,
}

impl Slots {
    /// This range of `items`.
    pub fn of<T>(self, items: &[T]) -> &[T] {
        &items[self.start as usize..self.end as usize]
    }

    pub fn len(self) -> usize {
        (self.end - self.start) as usize
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// The visible text of `runs`, with breaks as spaces; `text` is the document's [`Doc::text`].
pub fn plain_text(runs: &[Run], text: &str) -> String {
    let mut out = String::new();
    for run in runs {
        match run.kind {
            RunKind::Text | RunKind::Math(_) | RunKind::InlineImage(_) | RunKind::Embed(_) => {
                out.push_str(run.text.of(text));
            }
            RunKind::SoftBreak | RunKind::HardBreak => out.push(' '),
            RunKind::FootnoteRef(_) => {}
        }
    }
    out
}

#[derive(Clone, Copy, Debug)]
pub struct Run {
    /// Unescaped text: what is rendered and searched, a range of [`Doc::text`] (read it with
    /// [`Doc::run_text`]).
    pub text: TextRange,
    pub style: InlineStyle,
    pub link: Option<LinkId>,
    /// The source line this run starts on.
    pub src_line: u32,
    pub kind: RunKind,
}

/// A run's text: bytes `start..end` of its document's [`Doc::text`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextRange {
    pub start: u32,
    pub end: u32,
}

impl TextRange {
    /// This range of `text`, the document's [`Doc::text`].
    pub fn of(self, text: &str) -> &str {
        &text[self.start as usize..self.end as usize]
    }

    pub fn len(self) -> usize {
        (self.end - self.start) as usize
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunKind {
    Text,
    SoftBreak,
    HardBreak,
    /// Math typeset as Unicode (OBS-003): its text what shows, [`Doc::math`] its TeX.
    Math(MathId),
    InlineImage(LinkId),
    /// A note or another file that isn't an image, embedded (`![[note]]`): shown as a link to it,
    /// its text what the embed names (OBS-007).
    Embed(LinkId),
    FootnoteRef(FootnoteId),
}

/// Inline markup, resolved during parsing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct InlineStyle(u8);

impl InlineStyle {
    pub const NONE: Self = Self(0);
    pub const EMPH: Self = Self(1);
    pub const STRONG: Self = Self(1 << 1);
    pub const STRIKE: Self = Self(1 << 2);
    pub const CODE: Self = Self(1 << 3);
    /// Obsidian's `==highlight==` (OBS-005), and its `#tag` (OBS-006): superscript and subscript
    /// had their bits, never set, as pulldown-cmark's options for them are off.
    pub const HIGHLIGHT: Self = Self(1 << 4);
    pub const TAG: Self = Self(1 << 5);
    /// A class diagram member's UML classifier, static (`$`) or abstract (`*`), over whatever
    /// colors its parts.
    pub const STATIC: Self = Self(1 << 6);
    pub const ABSTRACT: Self = Self(1 << 7);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

impl BitOr for InlineStyle {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

#[derive(Debug)]
pub struct LinkRef {
    /// The raw destination, e.g. `../x.md#frag`, `note#Heading` (wiki) or `https://…`.
    pub dest: Box<str>,
    pub title: Box<str>,
    pub kind: LinkKind,
    pub src_line: u32,
    /// An image's size in Obsidian's `|100` or `|100x145` (OBS-008); in the padding, so a link
    /// stays 48 bytes.
    pub size: ImageSize,
}

/// The size Obsidian gives an image after a `|` in its alt text, `![[pic.png|100]]` or
/// `![alt|100x145](pic.png)` (OBS-008): the most pixels it takes across and down, 0 for no
/// bound. The image keeps its shape and is never enlarged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageSize {
    pub width: u16,
    pub height: u16,
}

impl ImageSize {
    pub const NONE: Self = Self {
        width: 0,
        height: 0,
    };

    /// `100` or `100x145`, spaces around it allowed; a width of 0 is none.
    pub fn parse(s: &str) -> Option<Self> {
        let num = |s: &str| {
            let s = s.trim();
            (!s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()))
                .then(|| s.parse::<u64>().map_or(u16::MAX, |n| n.min(65535) as u16))
        };
        let size = match s.split_once('x') {
            Some((w, h)) => Self {
                width: num(w)?,
                height: num(h)?,
            },
            None => Self {
                width: num(s)?,
                height: 0,
            },
        };
        (size.width > 0).then_some(size)
    }

    /// An image of `px` pixels scaled down, its shape kept, to fit this size.
    pub fn cap(self, px: (u32, u32)) -> (u32, u32) {
        let bound = |max: u16, n: u32| match max {
            0 => 1.0,
            _ => f64::from(max) / f64::from(n.max(1)),
        };
        let scale = bound(self.width, px.0).min(bound(self.height, px.1));
        if scale >= 1.0 {
            return px;
        }
        let of = |n: u32| ((f64::from(n) * scale).round() as u32).max(1);
        (of(px.0), of(px.1))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    Inline,
    Reference,
    Autolink,
    Email,
    Wiki { aliased: bool },
    Image,
    Footnote(FootnoteId),
}

#[derive(Debug)]
pub struct Anchor {
    /// The GitHub-style slug, unique within the document.
    pub slug: Box<str>,
    pub text: Box<str>,
    pub kind: AnchorKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorKind {
    Heading(u8),
    Footnote(FootnoteId),
    /// A block named by Obsidian's `^id` (OBS-009), its slug `^id` and its text the id: a
    /// paragraph or list item it ends, or the list, quote or table before it on a line alone.
    Block {
        /// The block's first search unit, which layout finds its row by.
        unit: UnitId,
        /// The block's lines in the note it was parsed from (not moved when grafted): what
        /// `![[note#^id]]` transcludes.
        lines: LineRange,
    },
}

#[derive(Debug)]
pub struct Footnote {
    pub label: Box<str>,
    /// Display number: footnotes are numbered in order of first reference.
    pub number: u32,
    /// `None` when the footnote is referenced but never defined.
    pub anchor: Option<AnchorId>,
    pub blocks: Vec<Block>,
    pub src: LineRange,
}

impl Doc {
    /// Every block, nested ones and footnotes' included, in source order.
    fn each_block<'a>(&'a self, f: &mut impl FnMut(&'a Block)) {
        fn walk<'a>(blocks: &'a [Block], f: &mut impl FnMut(&'a Block)) {
            for block in blocks {
                f(block);
                match &block.kind {
                    BlockKind::List(list) => {
                        for item in &list.items {
                            walk(&item.blocks, f);
                        }
                    }
                    BlockKind::Quote { blocks, .. } => walk(blocks, f),
                    BlockKind::Embed(e) => walk(&e.blocks, f),
                    _ => {}
                }
            }
        }
        walk(&self.blocks, f);
        for note in &self.footnotes {
            walk(&note.blocks, f);
        }
    }

    /// The code blocks, indexed by [`CodeBlock::id`].
    pub fn code_blocks(&self) -> Vec<&CodeBlock> {
        let mut out = vec![None; self.code_count as usize];
        self.each_block(&mut |block| {
            let code = match &block.kind {
                BlockKind::Code(code) => code,
                BlockKind::Properties(p) => &p.source,
                _ => return,
            };
            if let Some(slot) = out.get_mut(code.id as usize) {
                *slot = Some(code);
            }
        });
        out.into_iter().flatten().collect()
    }

    /// The code blocks a layout shows as code, the ones print highlights: every fenced block,
    /// and the note's own front matter when `as_written` says it shows as written (as
    /// [`crate::layout::PropertiesMode::as_written`] does). An embedded note's never shows.
    /// Before, any note with front matter cost print a highlight thread and a YAML highlight
    /// (PRN-006). In source order, not indexed by id as [`Doc::code_blocks`] is.
    pub fn shown_code(&self, as_written: impl Fn(&Properties) -> bool) -> Vec<&CodeBlock> {
        let own = self
            .properties()
            .filter(|&p| as_written(p))
            .map(|p| p.source.id);
        let mut out = Vec::new();
        self.each_block(&mut |block| match &block.kind {
            BlockKind::Code(code) => out.push(code),
            BlockKind::Properties(p) if Some(p.source.id) == own => out.push(&p.source),
            _ => {}
        });
        out
    }

    /// The tables, indexed by [`Table::id`].
    pub fn tables(&self) -> Vec<&Table> {
        let mut out = vec![None; self.table_count as usize];
        self.each_block(&mut |block| {
            if let BlockKind::Table(t) = &block.kind
                && let Some(slot) = out.get_mut(t.id as usize)
            {
                *slot = Some(t);
            }
        });
        out.into_iter().flatten().collect()
    }
}
