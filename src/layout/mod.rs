//! Layout (architecture §3): `Doc` + [`LayoutParams`] → rows of [`StyleKey`]'d spans. Every row
//! carries the source lines it renders (ADR-02); colours are resolved later, at paint (ADR-03).

mod code;
mod table;
pub mod wrap;

pub use code::expand_tabs;

#[cfg(test)]
mod tests;

use std::ops::Range;

use ratatui::layout::Size;
use unicode_width::UnicodeWidthStr;

use crate::doc::{
    AnchorId, Block, BlockKind, Callout, CodeId, Doc, InlineStyle, Inlines, LineRange, LinkId,
    List, Run, Table, UnitId,
};
use crate::images;
use crate::theme::{Glyphs, StyleKey};
use table::ColMeasure;
use wrap::{Line, NBSP, Wrapped};

/// How tables render; `t` cycles it for the whole session (ADR-11, plan Q12).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TableMode {
    /// A grid when the words fit, otherwise record cards.
    #[default]
    Auto,
    /// Always a grid, breaking words if it must.
    Grid,
    /// Always record cards.
    Records,
}

impl TableMode {
    pub fn next(self) -> Self {
        match self {
            TableMode::Auto => TableMode::Grid,
            TableMode::Grid => TableMode::Records,
            TableMode::Records => TableMode::Auto,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            TableMode::Auto => "auto",
            TableMode::Grid => "grid",
            TableMode::Records => "records",
        }
    }
}

/// Everything a layout depends on besides the Doc. Colours are deliberately absent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutParams {
    /// The terminal width. Tables may use all of it.
    pub width: u16,
    /// Prose wraps at `min(width, max_width)`.
    pub max_width: Option<u16>,
    pub table_mode: TableMode,
    pub code_wrap: bool,
    pub glyphs_gen: u64,
    /// Block images render as images when set; otherwise as a `🖼 alt (path)` line.
    pub images: Option<ImageFit>,
}

/// How block images are sized (architecture §9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageFit {
    /// The font's cell size in pixels.
    pub cell: Size,
    /// The most rows one image may take.
    pub max_rows: u16,
}

impl LayoutParams {
    pub fn prose_width(&self) -> u16 {
        self.max_width.map_or(self.width, |m| m.min(self.width))
    }
}

/// A styled slice of [`Layout::text`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub width: u16,
    pub key: StyleKey,
    pub inline: InlineStyle,
    pub link: Option<LinkId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Text,
    Blank,
    /// Borders, rules, code-block padding.
    Decor,
    /// A row of code: the block's [`CodeId`], the line within the block, and the byte offset
    /// in the (tab-expanded) line where this row's piece of it starts.
    Code {
        block: CodeId,
        line: u32,
        from: u32,
    },
    /// A row reserved for [`Layout::images`]`[slot]`, which paint draws over it.
    Image {
        slot: u32,
    },
}

#[derive(Clone, Debug)]
pub struct Row {
    /// Indices into [`Layout::spans`].
    pub spans: Range<u32>,
    /// The source lines this row renders; `LineRange::NONE` for decoration.
    pub src: LineRange,
    pub kind: RowKind,
    /// Columns painted with the alternate table-row background.
    pub stripe: Option<Range<u16>>,
}

/// A link's cells on one row, for keyboard focus (and a future mouse).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub row: u32,
    pub cols: Range<u16>,
    pub link: LinkId,
}

/// Where a stretch of a search unit's text renders: bytes `start..end` of the unit, on `row`,
/// starting at column `col` (ADR-12). In no-wrap code, `col` is where the code's window starts,
/// and paint shifts by the horizontal offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchSeg {
    pub unit: UnitId,
    pub start: u32,
    pub end: u32,
    pub row: u32,
    pub col: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeadingRow {
    pub row: u32,
    pub level: u8,
    pub anchor: AnchorId,
}

/// What paint needs to know about a code block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CodeInfo {
    /// The highlight cache key; `None` for plain code.
    pub key: Option<u64>,
    /// The longest line, in columns.
    pub width: u16,
    /// Columns the code gets on each row.
    pub window: u16,
}

impl CodeInfo {
    /// The furthest the shared horizontal offset can shift this block.
    pub fn max_x(&self) -> u16 {
        self.width.saturating_sub(self.window)
    }
}

/// Where a block image goes: `size` cells from column `x` of `row` on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageSlot {
    pub link: LinkId,
    pub row: u32,
    pub x: u16,
    /// Columns × rows.
    pub size: Size,
}

impl ImageSlot {
    /// Whether any of its rows is in `rows`.
    pub fn overlaps(&self, rows: Range<u32>) -> bool {
        self.row < rows.end && self.row + u32::from(self.size.height) > rows.start
    }
}

/// A grid table's rows, for the sticky header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridRows {
    /// The header row(s) and the separator under them.
    pub header: Range<u32>,
    /// One past the last body row.
    pub end: u32,
}

/// The width-independent work behind a layout, done once per Doc and glyph set (architecture §1,
/// "Measure"), so that a resize or a table-mode toggle only redistributes.
#[derive(Debug, Default)]
pub struct Measure {
    pub glyphs_gen: u64,
    /// Column measurements, indexed by [`Table::id`].
    tables: Vec<Vec<ColMeasure>>,
    /// Image sizes in pixels, indexed by [`LinkId`]; known once their headers have been read.
    images: Vec<Option<(u32, u32)>>,
}

impl Measure {
    pub fn new(doc: &Doc, glyphs: &Glyphs) -> Self {
        let mut tables = vec![Vec::new(); doc.table_count as usize];
        let footnotes = doc.footnotes.iter().map(|f| &f.blocks[..]);
        for blocks in std::iter::once(&doc.blocks[..]).chain(footnotes) {
            each_table(blocks, &mut |t| {
                if let Some(slot) = tables.get_mut(t.id as usize) {
                    *slot = table::measure(t, doc, glyphs);
                }
            });
        }
        Self {
            glyphs_gen: glyphs.generation(),
            tables,
            images: vec![None; doc.links.len()],
        }
    }

    /// Measure again for new glyphs, keeping the image sizes.
    pub fn remeasure(&mut self, doc: &Doc, glyphs: &Glyphs) {
        let images = std::mem::take(&mut self.images);
        *self = Self {
            images,
            ..Self::new(doc, glyphs)
        };
    }

    /// Record the pixel size of the image `link` shows.
    pub fn set_image(&mut self, link: LinkId, px: Option<(u32, u32)>) {
        if let Some(slot) = self.images.get_mut(link as usize) {
            *slot = px;
        }
    }
}

fn each_table<'a>(blocks: &'a [Block], f: &mut impl FnMut(&'a Table)) {
    for block in blocks {
        match &block.kind {
            BlockKind::Table(t) => f(t),
            BlockKind::List(list) => {
                for item in &list.items {
                    each_table(&item.blocks, f);
                }
            }
            BlockKind::Quote { blocks, .. } => each_table(blocks, f),
            _ => {}
        }
    }
}

#[derive(Debug)]
pub struct Layout {
    pub params: LayoutParams,
    /// Blank columns on each side; the git gutter draws in the left one.
    pub margin: u16,
    /// The arena all spans point into.
    pub text: String,
    pub spans: Vec<Span>,
    pub rows: Vec<Row>,
    /// Top-level block index → its rows.
    pub block_rows: Vec<Range<u32>>,
    /// Anchor → first row (`u32::MAX` if it didn't render).
    pub anchor_rows: Vec<u32>,
    pub headings: Vec<HeadingRow>,
    pub hits: Vec<Hit>,
    /// Search units → rows and columns, in row order.
    pub search: Vec<SearchSeg>,
    /// Grid tables with body rows, in row order.
    pub grids: Vec<GridRows>,
    /// Code blocks, indexed by [`CodeId`].
    pub code: Vec<CodeInfo>,
    /// Block images shown as images, in row order.
    pub images: Vec<ImageSlot>,
}

impl Layout {
    pub fn row_count(&self) -> u32 {
        self.rows.len() as u32
    }

    pub fn spans<'a>(&'a self, row: &Row) -> impl Iterator<Item = (&'a str, &'a Span)> + 'a {
        self.spans[row.spans.start as usize..row.spans.end as usize]
            .iter()
            .map(|s| (&self.text[s.start as usize..s.end as usize], s))
    }

    /// A row's text, without styles.
    pub fn row_text(&self, row: usize) -> String {
        self.spans(&self.rows[row]).map(|(t, _)| t).collect()
    }

    /// The first row that renders `line`, or failing that the first row after it.
    pub fn row_for_line(&self, line: u32) -> Option<u32> {
        self.rows
            .iter()
            .position(|r| r.src.contains(line))
            .or_else(|| self.rows.iter().position(|r| r.src.start > line))
            .map(|r| r as u32)
    }
}

/// Lay out a document, measuring it first. Use [`layout_measured`] to reuse a [`Measure`].
pub fn layout(doc: &Doc, glyphs: &Glyphs, params: LayoutParams) -> Layout {
    layout_measured(doc, &Measure::new(doc, glyphs), glyphs, params)
}

/// Lay out a document measured with the same glyphs.
pub fn layout_measured(
    doc: &Doc,
    measure: &Measure,
    glyphs: &Glyphs,
    params: LayoutParams,
) -> Layout {
    let margin = if params.prose_width() >= 24 {
        glyphs.margin
    } else {
        0
    };
    let mut b = Builder {
        doc,
        measure,
        g: glyphs,
        params,
        out: Layout {
            params,
            margin,
            text: String::new(),
            spans: Vec::new(),
            rows: Vec::new(),
            block_rows: Vec::with_capacity(doc.blocks.len()),
            anchor_rows: vec![u32::MAX; doc.anchors.len()],
            headings: Vec::new(),
            hits: Vec::new(),
            search: Vec::new(),
            grids: Vec::new(),
            code: vec![CodeInfo::default(); doc.code_count as usize],
            images: Vec::new(),
        },
        prefixes: Vec::new(),
        prefix_width: 0,
        right_margin: margin,
        text_key: StyleKey::Text,
        list_depth: 0,
        row_start: 0,
        col: 0,
        search_col: 0,
        stripe: None,
        striped: false,
    };
    if margin > 0 {
        b.push_prefix(&" ".repeat(margin as usize), StyleKey::Text);
    }
    for (i, block) in doc.blocks.iter().enumerate() {
        if i > 0 {
            b.blank_row();
        }
        let first = b.out.row_count();
        b.block(block);
        b.out.block_rows.push(first..b.out.row_count());
    }
    b.out
}

/// The part of an image block's alt text that renders: its search unit's text.
pub fn image_alt(alt: &str) -> &str {
    alt.trim()
}

/// Indentation pushed by a container: a bullet, a quote bar, a heading prefix.
struct Prefix {
    /// Shown on the container's first row.
    first: String,
    /// Shown on every later row; as wide as `first`.
    rest: String,
    key: StyleKey,
    width: u16,
    used: bool,
}

struct Builder<'a> {
    doc: &'a Doc,
    measure: &'a Measure,
    g: &'a Glyphs,
    params: LayoutParams,
    out: Layout,
    prefixes: Vec<Prefix>,
    prefix_width: u16,
    right_margin: u16,
    /// The base style of prose: `Text`, or `Quote` inside a block quote.
    text_key: StyleKey,
    list_depth: usize,
    /// Where the row being built starts in `out.spans`.
    row_start: u32,
    /// The column the next span lands in.
    col: u16,
    /// The column where the last search segment ends.
    search_col: u16,
    /// The stripe of the row being built.
    stripe: Option<Range<u16>>,
    /// Whether the table row being built is striped.
    striped: bool,
}

/// A span's style: key, inline markup, link.
type Look = (StyleKey, InlineStyle, Option<LinkId>);

fn push_span(out: &mut Layout, row_start: u32, col: &mut u16, text: &str, width: u16, look: Look) {
    let (key, inline, link) = look;
    if text.is_empty() {
        return;
    }
    let start = out.text.len() as u32;
    if text.contains(NBSP) {
        out.text
            .extend(text.chars().map(|c| if c == NBSP { ' ' } else { c }));
    } else {
        out.text.push_str(text);
    }
    let end = out.text.len() as u32;
    *col = col.saturating_add(width);
    if out.spans.len() as u32 > row_start
        && let Some(last) = out.spans.last_mut()
        && (last.key, last.inline, last.link) == (key, inline, link)
        && last.end == start
    {
        last.end = end;
        last.width = last.width.saturating_add(width);
        return;
    }
    out.spans.push(Span {
        start,
        end,
        width,
        key,
        inline,
        link,
    });
}

impl Builder<'_> {
    /// Prose columns available after the prefixes.
    fn avail(&self) -> u16 {
        self.params
            .prose_width()
            .saturating_sub(self.prefix_width + self.right_margin)
            .max(1)
    }

    /// Table columns: tables may grow past the prose width up to the terminal (plan Q3).
    fn table_avail(&self) -> u16 {
        self.params
            .width
            .saturating_sub(self.prefix_width + self.right_margin)
            .max(1)
    }

    fn push_prefix(&mut self, first: &str, key: StyleKey) {
        let width = first.width() as u16;
        self.prefixes.push(Prefix {
            first: first.to_owned(),
            rest: " ".repeat(width as usize),
            key,
            width,
            used: false,
        });
        self.prefix_width += width;
    }

    /// A prefix that repeats on every row, like a quote bar.
    fn push_bar(&mut self, bar: &str, key: StyleKey) {
        self.push_prefix(bar, key);
        if let Some(p) = self.prefixes.last_mut() {
            p.rest = bar.to_owned();
        }
    }

    fn pop_prefix(&mut self) {
        if let Some(p) = self.prefixes.pop() {
            self.prefix_width -= p.width;
        }
    }

    fn begin_row(&mut self) {
        self.row_start = self.out.spans.len() as u32;
        self.col = 0;
        let Builder {
            prefixes,
            out,
            col,
            row_start,
            ..
        } = self;
        for p in prefixes.iter_mut() {
            let text = if p.used { &p.rest } else { &p.first };
            push_span(
                out,
                *row_start,
                col,
                text,
                p.width,
                (p.key, InlineStyle::NONE, None),
            );
            p.used = true;
        }
    }

    fn end_row(&mut self, src: LineRange, kind: RowKind) {
        self.out.rows.push(Row {
            spans: self.row_start..self.out.spans.len() as u32,
            src,
            kind,
            stripe: self.stripe.take(),
        });
    }

    fn blank_row(&mut self) {
        self.row_start = self.out.spans.len() as u32;
        self.col = 0;
        let Builder {
            prefixes,
            out,
            col,
            row_start,
            ..
        } = self;
        for p in prefixes.iter() {
            push_span(
                out,
                *row_start,
                col,
                &p.rest,
                p.width,
                (p.key, InlineStyle::NONE, None),
            );
        }
        self.end_row(LineRange::NONE, RowKind::Blank);
    }

    fn text(&mut self, text: &str, key: StyleKey) {
        let width = text.width() as u16;
        push_span(
            &mut self.out,
            self.row_start,
            &mut self.col,
            text,
            width,
            (key, InlineStyle::NONE, None),
        );
    }

    /// Pad the row with spaces up to `width` columns past the prefixes.
    fn fill(&mut self, width: u16, key: StyleKey) {
        let target = self.prefix_width.saturating_add(width);
        if self.col < target {
            self.text(&" ".repeat((target - self.col) as usize), key);
        }
    }

    fn fill_with(&mut self, glyph: &str, width: u16, key: StyleKey) {
        let target = self.prefix_width.saturating_add(width);
        let gw = glyph.width().max(1) as u16;
        if self.col < target {
            self.text(&glyph.repeat(((target - self.col) / gw) as usize), key);
        }
    }

    fn emit_line(&mut self, w: &Wrapped, line: &Line) {
        for seg in &line.segs {
            let piece = &w.pieces[seg.piece as usize];
            let from = self.col;
            if let Some(unit) = w.unit {
                let (a, b) = (seg.start.max(piece.body_start), seg.end.min(piece.body_end));
                if a < b {
                    let col = from + w.buf[seg.start as usize..a as usize].width() as u16;
                    let end = col + w.buf[a as usize..b as usize].width() as u16;
                    let at = |x: u32| piece.unit_off + (x - piece.body_start);
                    self.search_seg(unit, at(a)..at(b), col..end);
                }
            }
            push_span(
                &mut self.out,
                self.row_start,
                &mut self.col,
                w.seg_text(seg),
                seg.width,
                (piece.key, piece.inline, piece.link),
            );
            if let Some(link) = piece.link {
                self.hit(from, self.col, link);
            }
        }
    }

    fn emit_lines(&mut self, w: &Wrapped) {
        for line in &w.lines {
            self.begin_row();
            self.emit_line(w, line);
            self.end_row(line.src, RowKind::Text);
        }
    }

    /// Record that bytes `range` of `unit` render on the current row in columns `cols`.
    fn search_seg(&mut self, unit: UnitId, range: Range<u32>, cols: Range<u16>) {
        let row = self.out.row_count();
        if let Some(last) = self.out.search.last_mut()
            && (last.unit, last.row, last.end) == (unit, row, range.start)
            && self.search_col == cols.start
        {
            last.end = range.end;
        } else {
            self.out.search.push(SearchSeg {
                unit,
                start: range.start,
                end: range.end,
                row,
                col: cols.start,
            });
        }
        self.search_col = cols.end;
    }

    fn hit(&mut self, from: u16, to: u16, link: LinkId) {
        let row = self.out.row_count();
        if let Some(h) = self.out.hits.last_mut()
            && h.row == row
            && h.link == link
            && h.cols.end == from
        {
            h.cols.end = to;
            return;
        }
        self.out.hits.push(Hit {
            row,
            cols: from..to,
            link,
        });
    }

    fn blocks(&mut self, blocks: &[Block], tight: bool) {
        for (i, block) in blocks.iter().enumerate() {
            if i > 0 && !tight {
                self.blank_row();
            }
            self.block(block);
        }
    }

    fn block(&mut self, block: &Block) {
        match &block.kind {
            BlockKind::Heading {
                level,
                inlines,
                anchor,
            } => self.heading(*level, inlines, *anchor),
            BlockKind::Paragraph(inlines) => {
                self.paragraph(&inlines.runs, inlines.unit, self.text_key, None);
            }
            BlockKind::List(list) => self.list(list),
            BlockKind::Quote { callout, blocks } => self.quote(*callout, blocks, block.src),
            BlockKind::Code(code) => code::layout(self, code, block.src),
            BlockKind::Table(table) => table::layout(self, table),
            BlockKind::Image { link, alt, unit } => self.image(*link, alt, *unit, block.src),
            BlockKind::Rule => self.rule(block.src),
            BlockKind::Html(html) => self.html(html, block.src),
            BlockKind::FootnoteDefs => self.footnotes(),
        }
    }

    fn paragraph(&mut self, runs: &[Run], unit: UnitId, key: StyleKey, suffix: Option<&str>) {
        let avail = self.avail();
        let w = wrap::wrap(runs, Some(unit), key, avail, self.doc, self.g, suffix);
        self.emit_lines(&w);
    }

    fn heading(&mut self, level: u8, inlines: &Inlines, anchor: AnchorId) {
        let row = self.out.row_count();
        if let Some(slot) = self.out.anchor_rows.get_mut(anchor as usize) {
            *slot = row;
        }
        self.out.headings.push(HeadingRow { row, level, anchor });
        let key = StyleKey::heading(level);
        let g = self.g;
        let (prefix, suffix) = &g.heading[usize::from(level.clamp(1, 6) - 1)];
        self.push_prefix(prefix, key);
        self.paragraph(&inlines.runs, inlines.unit, key, Some(suffix));
        self.pop_prefix();
    }

    fn list(&mut self, list: &List) {
        let g = self.g;
        let depth = self.list_depth;
        self.list_depth += 1;
        let last = list
            .start
            .map(|s| s.saturating_add(list.items.len().saturating_sub(1) as u64));
        let num_width = last.map_or(0, |n| n.to_string().len());
        for (i, item) in list.items.iter().enumerate() {
            if i > 0 && !list.tight {
                self.blank_row();
            }
            let (marker, key) = match (item.task, list.start) {
                (Some(done), _) => (
                    g.task[usize::from(!done)].clone(),
                    if done {
                        StyleKey::TaskDone
                    } else {
                        StyleKey::TaskOpen
                    },
                ),
                (None, Some(start)) => (
                    format!("{:>num_width$}. ", start.saturating_add(i as u64)),
                    StyleKey::ListNumber,
                ),
                (None, None) => (
                    format!("{} ", g.bullets[depth % g.bullets.len()]),
                    StyleKey::ListBullet,
                ),
            };
            self.push_prefix(&marker, key);
            if item.blocks.is_empty() {
                self.begin_row();
                self.end_row(item.src, RowKind::Text);
            } else {
                self.blocks(&item.blocks, list.tight);
            }
            self.pop_prefix();
        }
        self.list_depth -= 1;
    }

    fn quote(&mut self, callout: Option<Callout>, blocks: &[Block], src: LineRange) {
        let g = self.g;
        self.push_bar(&g.quote_bar, StyleKey::QuoteBar);
        let saved = std::mem::replace(&mut self.text_key, StyleKey::Quote);
        if let Some(callout) = callout {
            self.begin_row();
            self.text(callout.title(), StyleKey::callout(callout));
            self.end_row(LineRange::line(src.start), RowKind::Text);
        }
        self.blocks(blocks, false);
        if blocks.is_empty() && callout.is_none() {
            self.begin_row();
            self.end_row(src, RowKind::Text);
        }
        self.text_key = saved;
        self.pop_prefix();
    }

    /// A block image: reserved rows and a caption when its size is known and the terminal can
    /// show images, otherwise a `🖼 alt (path)` line.
    fn image(&mut self, link: LinkId, alt: &str, unit: UnitId, src: LineRange) {
        let dest = self.doc.links.get(link as usize).map_or("", |l| &l.dest);
        let alt = image_alt(alt);
        let px = self.measure.images.get(link as usize).copied().flatten();
        let shown = match (self.params.images, px) {
            (Some(fit), Some(px)) => {
                let max = Size::new(self.avail(), fit.max_rows);
                let size = images::fit(px, fit.cell, max);
                let slot = self.out.images.len() as u32;
                self.out.images.push(ImageSlot {
                    link,
                    row: self.out.row_count(),
                    x: self.prefix_width,
                    size,
                });
                for _ in 0..size.height {
                    self.begin_row();
                    self.end_row(src, RowKind::Image { slot });
                }
                true
            }
            _ => false,
        };
        // Under the image, the caption names it: the alt text, or the path when there is none.
        let dest = match alt {
            "" => format!("({dest})"),
            _ if shown => String::new(),
            _ => format!(" ({dest})"),
        };
        let parts = [&self.g.image[..], alt, &dest];
        let mut w = wrap::wrap_unit(parts, unit, StyleKey::Image, src.start, self.avail());
        for piece in &mut w.pieces {
            piece.link = Some(link);
        }
        self.emit_lines(&w);
    }

    fn rule(&mut self, src: LineRange) {
        let g = self.g;
        let avail = self.avail();
        self.begin_row();
        self.fill_with(&g.rule, avail, StyleKey::Rule);
        self.end_row(src, RowKind::Decor);
    }

    fn html(&mut self, html: &str, src: LineRange) {
        for (i, line) in html.lines().enumerate() {
            let w = wrap::wrap_plain(line, StyleKey::Html, src.start + i as u32, self.avail());
            self.emit_lines(&w);
        }
    }

    fn footnotes(&mut self) {
        let doc = self.doc;
        self.rule(LineRange::NONE);
        let mut notes: Vec<_> = doc
            .footnotes
            .iter()
            .filter(|f| f.anchor.is_some())
            .collect();
        notes.sort_by_key(|f| f.number);
        let label_width = notes
            .iter()
            .map(|f| format!("[{}] ", f.number).len())
            .max()
            .unwrap_or(0);
        for note in notes {
            if let Some(slot) = note
                .anchor
                .and_then(|a| self.out.anchor_rows.get_mut(a as usize))
            {
                *slot = self.out.rows.len() as u32;
            }
            let label = format!("{:<label_width$}", format!("[{}] ", note.number));
            self.push_prefix(&label, StyleKey::FootnoteLabel);
            if note.blocks.is_empty() {
                self.begin_row();
                self.end_row(note.src, RowKind::Text);
            } else {
                self.blocks(&note.blocks, true);
            }
            self.pop_prefix();
        }
    }
}
