//! pulldown-cmark events → [`Doc`] (ADR-01). A stack-based builder turns `Start`/`End` pairs into
//! owned blocks, resolves inline markup into styled runs, and converts byte offsets to lines.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::mem;
use std::ops::Range;
use std::sync::Arc;

use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, LinkType, Options, Parser, Tag, TagEnd,
};

use super::slug::Slugger;
use super::{
    Align, Anchor, AnchorKind, Block, BlockKind, Callout, CodeBlock, CodeId, Doc, Footnote,
    FootnoteId, InlineStyle, Inlines, LineRange, LinkId, LinkKind, LinkRef, List, ListItem, Run,
    RunKind, Table, TableId, TableRow, UnitId, fence,
};
use crate::{diagram, guard};

pub fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
        | Options::ENABLE_WIKILINKS
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
}

/// Parse markdown into the IR. Never panics, whatever the input: when pulldown-cmark panics, or
/// makes events out of all proportion to the text, the text shows as plain (F-05).
pub fn parse(text: &str) -> Doc {
    guard::catch(|| markdown(text))
        .flatten()
        .unwrap_or_else(|| super::file::code(text, None))
}

/// The most events a text of `len` bytes may make. Notes make up to 0.22 a byte, and dense markup
/// (an empty table cell per `|`) about 2; pulldown-cmark 0.13.4 doubles its events with each
/// `[[a|]]` (pulldown-cmark#1102).
fn max_events(len: usize) -> usize {
    8 * len + 4096
}

/// [`parse`], or `None` once the text makes more than [`max_events`].
fn markdown(text: &str) -> Option<Doc> {
    let max = max_events(text.len());
    let mut b = Builder::new(text);
    let events = Parser::new_ext(text, options()).into_offset_iter();
    for (n, (event, range)) in events.enumerate() {
        if n == max {
            return None;
        }
        b.event(event, range);
    }
    Some(b.finish())
}

/// The byte offset of the start of each line.
pub(super) fn line_starts(text: &str) -> Vec<u32> {
    std::iter::once(0)
        .chain(
            text.bytes()
                .enumerate()
                .filter(|&(_, b)| b == b'\n')
                .map(|(i, _)| i as u32 + 1),
        )
        .filter(|&s| s < text.len() as u32 || s == 0)
        .collect()
}

enum Container {
    Root,
    Quote(Option<Callout>),
    Item,
    Footnote(FootnoteId),
    /// Anything we flatten into its parent (definition lists, which are not enabled).
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InlineKind {
    Paragraph,
    /// Text directly inside a container: the content of a tight list item.
    Implicit,
    Heading(u8),
    Cell,
}

enum Frame {
    Blocks {
        kind: Container,
        blocks: Vec<Block>,
        src: LineRange,
        task: Option<bool>,
    },
    List {
        start: Option<u64>,
        loose: bool,
        items: Vec<ListItem>,
        src: LineRange,
    },
    Inline {
        kind: InlineKind,
        runs: Vec<Run>,
        src: LineRange,
    },
    Code {
        lang: Option<Box<str>>,
        text: String,
        src: LineRange,
        fenced: bool,
    },
    Html {
        text: String,
        src: LineRange,
    },
    Table {
        aligns: Vec<Align>,
        header: TableRow,
        rows: Vec<TableRow>,
        cells: Vec<Inlines>,
        row_src: LineRange,
        src: LineRange,
    },
    /// Front matter: parsed, not rendered.
    Skip,
}

struct ImageDraft {
    link: LinkId,
    alt: String,
    line: u32,
}

struct Builder {
    line_starts: Vec<u32>,
    stack: Vec<Frame>,
    links: Vec<LinkRef>,
    anchors: Vec<Anchor>,
    footnotes: Vec<Footnote>,
    footnote_ids: HashMap<Box<str>, FootnoteId>,
    ref_order: Vec<FootnoteId>,
    slugs: Slugger,
    style: InlineStyle,
    link: Option<LinkId>,
    image: Option<ImageDraft>,
    tables: TableId,
    code_blocks: CodeId,
    units: UnitId,
}

impl Builder {
    fn new(text: &str) -> Self {
        Self {
            line_starts: line_starts(text),
            stack: vec![Frame::Blocks {
                kind: Container::Root,
                blocks: Vec::new(),
                src: LineRange::NONE,
                task: None,
            }],
            links: Vec::new(),
            anchors: Vec::new(),
            footnotes: Vec::new(),
            footnote_ids: HashMap::new(),
            ref_order: Vec::new(),
            slugs: Slugger::default(),
            style: InlineStyle::NONE,
            link: None,
            image: None,
            tables: 0,
            code_blocks: 0,
            units: 0,
        }
    }

    fn line_of(&self, offset: usize) -> u32 {
        self.line_starts
            .partition_point(|&s| s as usize <= offset)
            .max(1) as u32
    }

    fn event(&mut self, event: Event<'_>, range: Range<usize>) {
        let line = self.line_of(range.start);
        let src = LineRange::new(
            line,
            self.line_of(range.end.saturating_sub(1).max(range.start)),
        );
        match event {
            Event::Start(tag) => self.start(tag, src),
            Event::End(end) => self.end(end),
            Event::Text(t) => self.text(&t, line),
            Event::Code(t) | Event::InlineMath(t) | Event::DisplayMath(t) => {
                self.push_text(&t, self.style | InlineStyle::CODE, line);
            }
            Event::Html(t) => match self.stack.last_mut() {
                Some(Frame::Html { text, .. }) => text.push_str(&t),
                _ => self.inline_html(&t, line),
            },
            Event::InlineHtml(t) => self.inline_html(&t, line),
            Event::FootnoteReference(label) => self.footnote_ref(&label, line),
            Event::SoftBreak => self.push_break(RunKind::SoftBreak, line),
            Event::HardBreak => self.push_break(RunKind::HardBreak, line),
            Event::Rule => self.push_block(Block {
                kind: BlockKind::Rule,
                src,
            }),
            Event::TaskListMarker(done) => {
                let item = self.stack.iter_mut().rev().find_map(|f| match f {
                    Frame::Blocks {
                        kind: Container::Item,
                        task,
                        ..
                    } => Some(task),
                    _ => None,
                });
                if let Some(task) = item {
                    *task = Some(done);
                }
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>, src: LineRange) {
        match tag {
            Tag::Paragraph => {
                self.close_implicit();
                self.mark_loose();
                self.push_inline(InlineKind::Paragraph, src);
            }
            Tag::Heading { level, .. } => {
                self.close_implicit();
                self.push_inline(InlineKind::Heading(level as u8), src);
            }
            Tag::BlockQuote(kind) => self.push_blocks(Container::Quote(kind.map(callout)), src),
            Tag::CodeBlock(kind) => {
                self.close_implicit();
                let (lang, fenced) = match kind {
                    CodeBlockKind::Fenced(info) => (fence::language(&info), true),
                    CodeBlockKind::Indented => (None, false),
                };
                self.stack.push(Frame::Code {
                    lang,
                    text: String::new(),
                    src,
                    fenced,
                });
            }
            Tag::HtmlBlock => {
                self.close_implicit();
                self.stack.push(Frame::Html {
                    text: String::new(),
                    src,
                });
            }
            Tag::List(start) => {
                self.close_implicit();
                self.stack.push(Frame::List {
                    start,
                    loose: false,
                    items: Vec::new(),
                    src,
                });
            }
            Tag::Item => self.push_blocks(Container::Item, src),
            Tag::FootnoteDefinition(label) => {
                let id = self.footnote_id(&label);
                self.push_blocks(Container::Footnote(id), src);
            }
            Tag::Table(aligns) => {
                self.close_implicit();
                self.stack.push(Frame::Table {
                    aligns: aligns.into_iter().map(align).collect(),
                    header: TableRow::default(),
                    rows: Vec::new(),
                    cells: Vec::new(),
                    row_src: src,
                    src,
                });
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(Frame::Table { row_src, .. }) = self.stack.last_mut() {
                    *row_src = src;
                }
            }
            Tag::TableCell => self.push_inline(InlineKind::Cell, src),
            Tag::Emphasis => self.style = self.style | InlineStyle::EMPH,
            Tag::Strong => self.style = self.style | InlineStyle::STRONG,
            Tag::Strikethrough => self.style = self.style | InlineStyle::STRIKE,
            Tag::Superscript => self.style = self.style | InlineStyle::SUP,
            Tag::Subscript => self.style = self.style | InlineStyle::SUB,
            Tag::Link {
                link_type,
                dest_url,
                title,
                ..
            } => {
                let kind = match link_type {
                    LinkType::Inline => LinkKind::Inline,
                    LinkType::Autolink => LinkKind::Autolink,
                    LinkType::Email => LinkKind::Email,
                    LinkType::WikiLink { has_pothole } => LinkKind::Wiki {
                        aliased: has_pothole,
                    },
                    _ => LinkKind::Reference,
                };
                self.link = Some(self.add_link(&dest_url, &title, kind, src.start));
            }
            Tag::Image {
                dest_url, title, ..
            } => {
                let link = self.add_link(&dest_url, &title, LinkKind::Image, src.start);
                self.image = Some(ImageDraft {
                    link,
                    alt: String::new(),
                    line: src.start,
                });
            }
            Tag::MetadataBlock(_) => {
                self.close_implicit();
                self.stack.push(Frame::Skip);
            }
            Tag::DefinitionList | Tag::DefinitionListTitle | Tag::DefinitionListDefinition => {
                self.push_blocks(Container::Other, src);
            }
        }
    }

    fn end(&mut self, end: TagEnd) {
        match end {
            TagEnd::Emphasis => self.style = self.style.without(InlineStyle::EMPH),
            TagEnd::Strong => self.style = self.style.without(InlineStyle::STRONG),
            TagEnd::Strikethrough => self.style = self.style.without(InlineStyle::STRIKE),
            TagEnd::Superscript => self.style = self.style.without(InlineStyle::SUP),
            TagEnd::Subscript => self.style = self.style.without(InlineStyle::SUB),
            TagEnd::Link => self.link = None,
            TagEnd::Image => {
                if let Some(img) = self.image.take() {
                    self.push_run(Run {
                        text: img.alt.into(),
                        style: self.style,
                        link: Some(img.link),
                        src_line: img.line,
                        kind: RunKind::InlineImage(img.link),
                    });
                }
            }
            TagEnd::TableHead => {
                if let Some(Frame::Table {
                    header,
                    cells,
                    row_src,
                    ..
                }) = self.stack.last_mut()
                {
                    *header = TableRow {
                        cells: mem::take(cells),
                        src: *row_src,
                    };
                }
            }
            TagEnd::TableRow => {
                if let Some(Frame::Table {
                    rows,
                    cells,
                    row_src,
                    ..
                }) = self.stack.last_mut()
                {
                    rows.push(TableRow {
                        cells: mem::take(cells),
                        src: *row_src,
                    });
                }
            }
            TagEnd::TableCell => {
                if let Some(Frame::Inline {
                    kind: InlineKind::Cell,
                    ..
                }) = self.stack.last()
                    && let Some(Frame::Inline { runs, .. }) = self.stack.pop()
                    && let Some(Frame::Table { cells, .. }) = self.stack.last_mut()
                {
                    cells.push(Inlines {
                        runs,
                        unit: self.units,
                    });
                    self.units += 1;
                }
            }
            _ => {
                self.close_implicit();
                self.pop_block();
            }
        }
    }

    fn text(&mut self, text: &str, line: u32) {
        if let Some(img) = &mut self.image {
            img.alt.push_str(text);
            return;
        }
        match self.stack.last_mut() {
            Some(Frame::Code { text: code, .. } | Frame::Html { text: code, .. }) => {
                code.push_str(text);
            }
            Some(Frame::Skip) => {}
            _ => self.push_text(text, self.style, line),
        }
    }

    fn push_text(&mut self, text: &str, style: InlineStyle, line: u32) {
        if let Some(img) = &mut self.image {
            img.alt.push_str(text);
            return;
        }
        self.push_run(Run {
            text: text.into(),
            style,
            link: self.link,
            src_line: line,
            kind: RunKind::Text,
        });
    }

    fn push_break(&mut self, kind: RunKind, line: u32) {
        if let Some(img) = &mut self.image {
            img.alt.push(' ');
            return;
        }
        self.push_run(Run {
            text: "".into(),
            style: self.style,
            link: self.link,
            src_line: line,
            kind,
        });
    }

    fn inline_html(&mut self, html: &str, line: u32) {
        let tag = html.trim().to_ascii_lowercase();
        let is_br = tag
            .strip_prefix("<br")
            .is_some_and(|rest| !rest.starts_with(|c: char| c.is_ascii_alphanumeric()));
        if is_br {
            self.push_break(RunKind::HardBreak, line);
        }
        // Other inline tags are dropped; the text between them renders as ordinary text.
    }

    fn footnote_ref(&mut self, label: &str, line: u32) {
        let id = self.footnote_id(label);
        self.ref_order.push(id);
        let link = self.add_link(label, "", LinkKind::Footnote(id), line);
        self.push_run(Run {
            text: "".into(),
            style: self.style,
            link: Some(link),
            src_line: line,
            kind: RunKind::FootnoteRef(id),
        });
    }

    fn push_run(&mut self, run: Run) {
        if !matches!(self.stack.last(), Some(Frame::Inline { .. })) {
            if !matches!(self.stack.last(), Some(Frame::Blocks { .. })) {
                return;
            }
            self.push_inline(InlineKind::Implicit, LineRange::line(run.src_line));
        }
        if let Some(Frame::Inline { runs, .. }) = self.stack.last_mut() {
            runs.push(run);
        }
    }

    fn push_inline(&mut self, kind: InlineKind, src: LineRange) {
        self.stack.push(Frame::Inline {
            kind,
            runs: Vec::new(),
            src,
        });
    }

    fn push_blocks(&mut self, kind: Container, src: LineRange) {
        self.close_implicit();
        self.stack.push(Frame::Blocks {
            kind,
            blocks: Vec::new(),
            src,
            task: None,
        });
    }

    fn close_implicit(&mut self) {
        if let Some(Frame::Inline {
            kind: InlineKind::Implicit,
            ..
        }) = self.stack.last()
        {
            self.pop_block();
        }
    }

    /// A `Paragraph` directly inside a list item makes the list loose.
    fn mark_loose(&mut self) {
        let n = self.stack.len();
        if n >= 2
            && let Frame::Blocks {
                kind: Container::Item,
                ..
            } = &self.stack[n - 1]
            && let Frame::List { loose, .. } = &mut self.stack[n - 2]
        {
            *loose = true;
        }
    }

    fn add_link(&mut self, dest: &str, title: &str, kind: LinkKind, line: u32) -> LinkId {
        self.links.push(LinkRef {
            dest: dest.into(),
            title: title.into(),
            kind,
            src_line: line,
        });
        self.links.len() as LinkId - 1
    }

    fn footnote_id(&mut self, label: &str) -> FootnoteId {
        if let Some(&id) = self.footnote_ids.get(label) {
            return id;
        }
        let id = self.footnotes.len() as FootnoteId;
        self.footnotes.push(Footnote {
            label: label.into(),
            number: 0,
            anchor: None,
            blocks: Vec::new(),
            src: LineRange::NONE,
        });
        self.footnote_ids.insert(label.into(), id);
        id
    }

    fn pop_block(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        let Some(frame) = self.stack.pop() else {
            return;
        };
        let block = match frame {
            Frame::Inline { kind, runs, src } => match kind {
                InlineKind::Paragraph | InlineKind::Implicit | InlineKind::Cell
                    if runs.is_empty() =>
                {
                    return;
                }
                InlineKind::Heading(level) => {
                    let inlines = Inlines {
                        runs,
                        unit: self.next_unit(),
                    };
                    let text = inlines.plain_text();
                    let slug = self.slugs.slug(&text);
                    self.anchors.push(Anchor {
                        slug: slug.into(),
                        text: text.into(),
                        kind: AnchorKind::Heading(level),
                    });
                    Block {
                        kind: BlockKind::Heading {
                            level,
                            inlines,
                            anchor: self.anchors.len() as u32 - 1,
                        },
                        src,
                    }
                }
                InlineKind::Implicit => {
                    let first = runs.first().map_or(src.start, |r| r.src_line);
                    let last = runs.last().map_or(src.end, |r| r.src_line);
                    paragraph(runs, LineRange::new(first, last), self.next_unit())
                }
                InlineKind::Paragraph | InlineKind::Cell => paragraph(runs, src, self.next_unit()),
            },
            Frame::Blocks {
                kind,
                blocks,
                src,
                task,
            } => match kind {
                Container::Item => {
                    if let Some(Frame::List { items, .. }) = self.stack.last_mut() {
                        items.push(ListItem { task, blocks, src });
                    }
                    return;
                }
                Container::Footnote(id) => {
                    let label = self.footnotes[id as usize].label.clone();
                    let slug = self.slugs.slug(&format!("fn-{label}"));
                    self.anchors.push(Anchor {
                        slug: slug.into(),
                        text: label,
                        kind: AnchorKind::Footnote(id),
                    });
                    let note = &mut self.footnotes[id as usize];
                    note.anchor = Some(self.anchors.len() as u32 - 1);
                    note.blocks = blocks;
                    note.src = src;
                    return;
                }
                Container::Quote(callout) => Block {
                    kind: BlockKind::Quote { callout, blocks },
                    src,
                },
                Container::Other => {
                    for b in blocks {
                        self.push_block(b);
                    }
                    return;
                }
                Container::Root => return,
            },
            Frame::List {
                start,
                loose,
                items,
                src,
            } => Block {
                kind: BlockKind::List(List {
                    start,
                    tight: !loose,
                    items,
                }),
                src,
            },
            Frame::Code {
                lang,
                text,
                src,
                fenced,
            } => {
                let mut h = DefaultHasher::new();
                text.hash(&mut h);
                self.code_blocks += 1;
                let first_unit = self.units;
                self.units += super::code_lines(&text).count() as UnitId;
                let diagram = (lang.as_deref() == Some("mermaid"))
                    .then(|| guard::catch(|| diagram::parse(&text)).flatten())
                    .flatten()
                    .map(Arc::new);
                Block {
                    kind: BlockKind::Code(CodeBlock {
                        id: self.code_blocks - 1,
                        lang,
                        hash: h.finish(),
                        text: text.into(),
                        first_line: src.start + u32::from(fenced),
                        fenced,
                        first_unit,
                        diagram,
                    }),
                    src,
                }
            }
            Frame::Html { text, src } => {
                let trimmed = text.trim();
                if trimmed.is_empty() || (trimmed.starts_with("<!--") && trimmed.ends_with("-->")) {
                    return;
                }
                Block {
                    kind: BlockKind::Html(trimmed.into()),
                    src,
                }
            }
            Frame::Table {
                aligns,
                header,
                rows,
                src,
                ..
            } => {
                self.tables += 1;
                Block {
                    kind: BlockKind::Table(Table {
                        id: self.tables - 1,
                        aligns,
                        header,
                        rows,
                    }),
                    src,
                }
            }
            Frame::Skip => return,
        };
        self.push_block(block);
    }

    fn next_unit(&mut self) -> UnitId {
        self.units += 1;
        self.units - 1
    }

    fn push_block(&mut self, block: Block) {
        self.close_implicit();
        let target = self.stack.iter_mut().rev().find_map(|f| match f {
            Frame::Blocks { blocks, .. } => Some(blocks),
            _ => None,
        });
        if let Some(blocks) = target {
            blocks.push(block);
        }
    }

    fn finish(mut self) -> Doc {
        while self.stack.len() > 1 {
            self.close_implicit();
            self.pop_block();
        }
        let mut blocks = match self.stack.pop() {
            Some(Frame::Blocks { blocks, .. }) => blocks,
            _ => Vec::new(),
        };

        // Number footnotes in order of first reference, then any that are never referenced.
        let mut next = 0;
        let mut numbers = vec![0; self.footnotes.len()];
        for id in self
            .ref_order
            .iter()
            .copied()
            .chain(0..self.footnotes.len() as u32)
        {
            let n = &mut numbers[id as usize];
            if *n == 0 {
                next += 1;
                *n = next;
            }
        }
        for (note, n) in self.footnotes.iter_mut().zip(numbers) {
            note.number = n;
        }
        if self.footnotes.iter().any(|f| f.anchor.is_some()) {
            blocks.push(Block {
                kind: BlockKind::FootnoteDefs,
                src: LineRange::NONE,
            });
        }

        Doc {
            blocks,
            links: self.links,
            anchors: self.anchors,
            footnotes: self.footnotes,
            line_starts: self.line_starts,
            table_count: self.tables,
            code_count: self.code_blocks,
            unit_count: self.units,
        }
    }
}

/// A paragraph that holds nothing but one image becomes an image block.
fn paragraph(runs: Vec<Run>, src: LineRange, unit: UnitId) -> Block {
    let mut images = runs.iter().filter(|r| !is_blank(r));
    if let (Some(run), None) = (images.next(), images.next())
        && let RunKind::InlineImage(link) = run.kind
    {
        return Block {
            kind: BlockKind::Image {
                link,
                alt: run.text.clone(),
                unit,
            },
            src,
        };
    }
    Block {
        kind: BlockKind::Paragraph(Inlines { runs, unit }),
        src,
    }
}

fn is_blank(run: &Run) -> bool {
    match run.kind {
        RunKind::Text => run.text.trim().is_empty(),
        RunKind::SoftBreak | RunKind::HardBreak => true,
        RunKind::InlineImage(_) | RunKind::FootnoteRef(_) => false,
    }
}

fn callout(kind: BlockQuoteKind) -> Callout {
    match kind {
        BlockQuoteKind::Note => Callout::Note,
        BlockQuoteKind::Tip => Callout::Tip,
        BlockQuoteKind::Important => Callout::Important,
        BlockQuoteKind::Warning => Callout::Warning,
        BlockQuoteKind::Caution => Callout::Caution,
    }
}

fn align(a: Alignment) -> Align {
    match a {
        Alignment::None => Align::None,
        Alignment::Left => Align::Left,
        Alignment::Center => Align::Center,
        Alignment::Right => Align::Right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(blocks: &[Block]) -> Vec<&'static str> {
        blocks
            .iter()
            .map(|b| match &b.kind {
                BlockKind::Heading { .. } => "heading",
                BlockKind::Paragraph(_) => "paragraph",
                BlockKind::List(_) => "list",
                BlockKind::Quote { .. } => "quote",
                BlockKind::Code(_) => "code",
                BlockKind::Table(_) => "table",
                BlockKind::Image { .. } => "image",
                BlockKind::Rule => "rule",
                BlockKind::Html(_) => "html",
                BlockKind::FootnoteDefs => "footnotes",
            })
            .collect()
    }

    fn list(b: &Block) -> &List {
        match &b.kind {
            BlockKind::List(l) => l,
            _ => panic!("not a list"),
        }
    }

    #[test]
    fn markup_that_blows_up_in_the_parser_shows_as_plain_text() {
        // pulldown-cmark#1102: each `[[a|]]` doubles the events; 40 of them would be 2⁴⁰
        let text = "[[a|]] ".repeat(40);
        let doc = parse(&text);
        assert_eq!(kinds(&doc.blocks), ["code"]);
        assert!(doc.links.is_empty());
        // a few stay markdown
        assert_eq!(kinds(&parse("[[a|]] [[b|]]\n").blocks), ["paragraph"]);
    }

    #[test]
    fn a_parser_panic_shows_the_text_as_plain() {
        // pulldown-cmark#1129: `into_offset_iter` unwraps an empty tight paragraph
        for text in ["- [o]:u\n    \t", "> - [o]:u\n>     \t", "+\t[\0]:d\n\u{b}"] {
            let doc = parse(text);
            assert_eq!(kinds(&doc.blocks), ["code"], "{text:?}");
        }
    }

    #[test]
    fn inline_markup_is_resolved_into_runs() {
        let doc = parse("some **bold** and _emph_ and `code`\n");
        let BlockKind::Paragraph(inl) = &doc.blocks[0].kind else {
            panic!()
        };
        let runs: Vec<_> = inl.runs.iter().map(|r| (&*r.text, r.style)).collect();
        assert_eq!(
            runs,
            [
                ("some ", InlineStyle::NONE),
                ("bold", InlineStyle::STRONG),
                (" and ", InlineStyle::NONE),
                ("emph", InlineStyle::EMPH),
                (" and ", InlineStyle::NONE),
                ("code", InlineStyle::CODE),
            ]
        );
    }

    #[test]
    fn line_ranges() {
        let doc = parse("# Title\n\npara one\npara two\n\n```rust\nfn x() {}\n```\n");
        assert_eq!(kinds(&doc.blocks), ["heading", "paragraph", "code"]);
        assert_eq!(doc.blocks[0].src, LineRange::new(1, 1));
        assert_eq!(doc.blocks[1].src, LineRange::new(3, 4));
        assert_eq!(doc.blocks[2].src, LineRange::new(6, 8));
        let BlockKind::Code(code) = &doc.blocks[2].kind else {
            panic!()
        };
        assert_eq!(code.first_line, 7);
        assert_eq!(code.lang.as_deref(), Some("rust"));
        assert_eq!(code.lines().collect::<Vec<_>>(), ["fn x() {}"]);
        let BlockKind::Paragraph(inl) = &doc.blocks[1].kind else {
            panic!()
        };
        assert_eq!(inl.runs.last().map(|r| r.src_line), Some(4));
    }

    #[test]
    fn tight_and_loose_lists() {
        let doc = parse("- a\n- b\n  - nested\n\n1. x\n\n2. y\n");
        assert_eq!(kinds(&doc.blocks), ["list", "list"]);
        let tight = list(&doc.blocks[0]);
        assert!(tight.tight);
        assert_eq!(kinds(&tight.items[1].blocks), ["paragraph", "list"]);
        let loose = list(&doc.blocks[1]);
        assert!(!loose.tight);
        assert_eq!(loose.start, Some(1));
    }

    #[test]
    fn task_items() {
        let doc = parse("- [x] done\n- [ ] open\n- plain\n");
        let tasks: Vec<_> = list(&doc.blocks[0]).items.iter().map(|i| i.task).collect();
        assert_eq!(tasks, [Some(true), Some(false), None]);
    }

    #[test]
    fn footnotes_are_numbered_by_first_reference() {
        let doc = parse("b[^b] a[^a] b again[^b]\n\n[^a]: first def\n[^b]: second def\n");
        assert_eq!(kinds(&doc.blocks), ["paragraph", "footnotes"]);
        let numbers: Vec<_> = doc
            .footnotes
            .iter()
            .map(|f| (&*f.label, f.number))
            .collect();
        assert_eq!(numbers, [("b", 1), ("a", 2)]);
        assert!(doc.footnotes.iter().all(|f| f.anchor.is_some()));
    }

    #[test]
    fn lone_image_becomes_an_image_block() {
        let doc = parse("![a cat](cat.png)\n\ntext ![inline](x.png) text\n");
        assert_eq!(kinds(&doc.blocks), ["image", "paragraph"]);
        assert_eq!(&*doc.links[0].dest, "cat.png");
    }

    #[test]
    fn wiki_links() {
        let doc = parse("see [[other-note#Some Heading|alias]] and [[plain]]\n");
        let kinds: Vec<_> = doc.links.iter().map(|l| (&*l.dest, l.kind)).collect();
        assert_eq!(
            kinds,
            [
                ("other-note#Some Heading", LinkKind::Wiki { aliased: true }),
                ("plain", LinkKind::Wiki { aliased: false }),
            ]
        );
    }

    #[test]
    fn headings_get_unique_anchors() {
        let doc = parse("# Intro\n## Intro\n## `code` *x*\n");
        let slugs: Vec<_> = doc.anchors.iter().map(|a| &*a.slug).collect();
        assert_eq!(slugs, ["intro", "intro-1", "code-x"]);
    }

    #[test]
    fn tables() {
        let doc = parse("| a | b |\n|---|--:|\n| 1 | 2 |\n| 3 | 4 |\n");
        let BlockKind::Table(t) = &doc.blocks[0].kind else {
            panic!()
        };
        assert_eq!(t.aligns, [Align::None, Align::Right]);
        assert_eq!(t.header.cells.len(), 2);
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.header.src, LineRange::line(1));
        assert_eq!(t.rows[1].src, LineRange::line(4));
    }

    #[test]
    fn callouts_and_comments() {
        let doc = parse("> [!WARNING]\n> careful\n\n<!-- hidden -->\n\n<div>shown</div>\n");
        assert_eq!(kinds(&doc.blocks), ["quote", "html"]);
        assert!(matches!(
            doc.blocks[0].kind,
            BlockKind::Quote {
                callout: Some(Callout::Warning),
                ..
            }
        ));
    }

    #[test]
    fn front_matter_is_skipped() {
        let doc = parse("---\ntitle: x\n---\n\n# Body\n");
        assert_eq!(kinds(&doc.blocks), ["heading"]);
        assert_eq!(doc.blocks[0].src, LineRange::line(5));
    }

    #[test]
    fn hostile_input_does_not_panic() {
        for text in [
            "",
            "\n\n\n",
            "- [ ]",
            "[^x]",
            "> - > - > ```\n",
            "|a|\n|-|\n|![x](y)|",
            "<br>",
            "***\n---\n___",
            "[[]]",
            "\u{0}\u{feff}#",
        ] {
            parse(text);
        }
    }
}
