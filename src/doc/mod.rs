//! The document IR (ADR-01, ADR-02): an owned, markup-free block tree, parsed once per source
//! version. Nothing downstream of [`parse`] sees pulldown-cmark types.

mod fence;
mod parse;
pub mod slug;

use std::ops::BitOr;
use std::sync::Arc;

pub use parse::parse;

pub type LinkId = u32;
pub type AnchorId = u32;
pub type FootnoteId = u32;
pub type TableId = u32;
pub type CodeId = u32;
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
}

impl Doc {
    pub fn line_count(&self) -> u32 {
        self.line_starts.len() as u32
    }
}

#[derive(Debug)]
pub struct Block {
    pub kind: BlockKind,
    pub src: LineRange,
}

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
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
    pub src: LineRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Callout {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

impl Callout {
    pub fn title(self) -> &'static str {
        match self {
            Callout::Note => "Note",
            Callout::Tip => "Tip",
            Callout::Important => "Important",
            Callout::Warning => "Warning",
            Callout::Caution => "Caution",
        }
    }
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
    /// The search unit of the first line; line `n` is unit `first_unit + n`.
    pub first_unit: UnitId,
}

impl CodeBlock {
    /// The code's lines, without the final newline.
    pub fn lines(&self) -> impl Iterator<Item = &str> {
        code_lines(&self.text)
    }
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
}

impl Table {
    /// The widest of the alignment row, the header and the body rows.
    pub fn column_count(&self) -> usize {
        self.aligns
            .len()
            .max(self.header.cells.len())
            .max(self.rows.iter().map(|r| r.cells.len()).max().unwrap_or(0))
    }
}

#[derive(Debug, Default)]
pub struct TableRow {
    pub cells: Vec<Inlines>,
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
    pub runs: Vec<Run>,
    pub unit: UnitId,
}

impl Inlines {
    /// The visible text, with breaks as spaces.
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for run in &self.runs {
            match run.kind {
                RunKind::Text | RunKind::InlineImage(_) => out.push_str(&run.text),
                RunKind::SoftBreak | RunKind::HardBreak => out.push(' '),
                RunKind::FootnoteRef(_) => {}
            }
        }
        out
    }
}

#[derive(Debug)]
pub struct Run {
    /// Unescaped text: what is rendered and searched.
    pub text: Box<str>,
    pub style: InlineStyle,
    pub link: Option<LinkId>,
    /// The source line this run starts on.
    pub src_line: u32,
    pub kind: RunKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunKind {
    Text,
    SoftBreak,
    HardBreak,
    InlineImage(LinkId),
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
    pub const SUP: Self = Self(1 << 4);
    pub const SUB: Self = Self(1 << 5);

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
            if let BlockKind::Code(code) = &block.kind
                && let Some(slot) = out.get_mut(code.id as usize)
            {
                *slot = Some(code);
            }
        });
        out.into_iter().flatten().collect()
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
