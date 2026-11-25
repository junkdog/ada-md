//! Layout (architecture §3): `Doc` + [`LayoutParams`] → rows of [`StyleKey`]'d spans. Every row
//! carries the source lines it renders (ADR-02); colors are resolved later, at paint (ADR-03).

mod code;
mod diagram;
mod table;
pub mod wrap;

pub use code::{
    TAB_STOP, block_needs_expanding, expand_block, expand_tabs, expand_tabs_in, expand_tabs_onto,
};

#[cfg(test)]
mod tests;

use std::ops::Range;
use std::sync::OnceLock;

use crate::geom::Size;
use unicode_width::UnicodeWidthStr;

use crate::doc::{
    AnchorId, Block, BlockKind, Callout, CodeBlock, CodeId, Doc, Embed, InlineStyle, Inlines,
    LineRange, LinkId, List, Properties, Run, Table, TableId, UnitId,
};
use crate::images;
use crate::theme::{Glyphs, StyleKey};
use table::ColMeasure;
pub use table::Sort;
use wrap::{Line, NBSP, PAD, Seg, Wrapped, text_width};

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

/// Which files number their rows by source line, left of the margin; `#` toggles them for the
/// kind of file shown. Each mode covers the ones before it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum LineNumbers {
    #[default]
    Off,
    /// Code and text files, whose rows map to their lines one to one.
    Code,
    /// Markdown too.
    All,
}

impl LineNumbers {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "off" => Some(LineNumbers::Off),
            "code" => Some(LineNumbers::Code),
            "all" => Some(LineNumbers::All),
            _ => None,
        }
    }

    /// As the config file and `-n` spell it: the inverse of [`LineNumbers::parse`].
    pub fn spelling(self) -> &'static str {
        match self {
            LineNumbers::Off => "off",
            LineNumbers::Code => "code",
            LineNumbers::All => "all",
        }
    }

    /// Show or hide the numbers of the kind of file shown, keeping the other kind as it is.
    pub fn toggle(self, code: bool) -> Self {
        match (code, self) {
            (true, LineNumbers::Off) => LineNumbers::Code,
            (true, _) => LineNumbers::Off,
            (false, LineNumbers::All) => LineNumbers::Code,
            (false, _) => LineNumbers::All,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            LineNumbers::Off => "off",
            LineNumbers::Code => "code files",
            LineNumbers::All => "all files",
        }
    }
}

/// How a note's properties show (ADR-46): `p` hides them or shows them again for the session,
/// and `[markdown] properties` says how they start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PropertiesMode {
    /// A compact block of key and value rows.
    #[default]
    Show,
    Hide,
    /// The front matter as written, highlighted as YAML or TOML.
    Source,
}

impl PropertiesMode {
    pub const NAMES: &[&str] = &["show", "hide", "source"];

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "show" => Some(PropertiesMode::Show),
            "hide" => Some(PropertiesMode::Hide),
            "source" => Some(PropertiesMode::Source),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            PropertiesMode::Show => "show",
            PropertiesMode::Hide => "hide",
            PropertiesMode::Source => "source",
        }
    }

    /// `p`: shown properties hidden, hidden ones shown as `shown` (the config's) shows them, as
    /// rows when that hides them too.
    pub fn toggle(self, shown: Self) -> Self {
        match (self, shown) {
            (PropertiesMode::Hide, PropertiesMode::Hide) => PropertiesMode::Show,
            (PropertiesMode::Hide, shown) => shown,
            _ => PropertiesMode::Hide,
        }
    }

    /// Whether `p` shows as written, its source laid out as code: in `source` mode, and in
    /// `show` when ada couldn't read it. Only then is its YAML worth highlighting (PRN-006).
    pub fn as_written(self, p: &Properties) -> bool {
        match self {
            PropertiesMode::Source => true,
            PropertiesMode::Show => !p.read,
            PropertiesMode::Hide => false,
        }
    }
}

/// The most property rows shown before a row saying how many more there are; one more than this
/// shows whole, as the row saying so would take its place.
pub const MAX_PROPERTIES: usize = 6;

/// The most rows a property's value takes: past them, its first rows, then one saying how many
/// more lines it has. A list of hundreds of links (a playlist's tracks) took a thousand rows.
pub const MAX_VALUE_ROWS: usize = 3;

/// Everything a layout depends on besides the Doc. Colors are deliberately absent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutParams {
    /// The terminal width. Tables may use all of it.
    pub width: u16,
    /// Prose wraps at `min(width, max_width)`.
    pub max_width: Option<u16>,
    pub table_mode: TableMode,
    pub properties: PropertiesMode,
    pub code_wrap: bool,
    /// Code blocks name their language on their top row: the TUI's do, a print's don't. A
    /// [whole file](crate::doc::CodeBlock::whole_file) has no such row in either.
    pub code_labels: bool,
    /// A file of code printed on the terminal's background, which is its page: no margin, and
    /// its block takes no columns of its own but a wrapped line's mark. A man page is laid out
    /// so in the viewer too, as `man` filled it to the terminal's width (X-188).
    pub bare: bool,
    /// Rows are numbered unless `Off`: [`FileType::params`](crate::filetype::FileType::params)
    /// turns the mode off for the files it doesn't cover.
    pub line_numbers: LineNumbers,
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
    /// The rows the document has on screen.
    pub screen_rows: u16,
}

impl LayoutParams {
    pub fn prose_width(&self) -> u16 {
        self.max_width.map_or(self.width, |m| m.min(self.width))
    }
}

/// A styled slice of [`Layout::text`], from `start` to where the next span starts (the text's end
/// for the last): spans cover the text in order, without gaps, so they don't keep their ends
/// (MEM-010: 20 bytes to 12, XL's TUI held 212k of them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    /// The [`LinkId`], or [`NO_LINK`].
    link: u32,
    pub width: u16,
    pub key: StyleKey,
    pub inline: InlineStyle,
}

/// [`Span`]'s link for none: an `Option<LinkId>` took 8 of its bytes.
const NO_LINK: u32 = u32::MAX;

// MEM-010: XL's layout holds 212k spans and 39k rows, so a field added here is a regression to
// measure first.
const _: () = assert!(size_of::<Span>() == 12 && size_of::<Row>() == 32);

impl Span {
    pub fn link(&self) -> Option<LinkId> {
        (self.link != NO_LINK).then_some(self.link)
    }
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
    /// A row of a drawn mermaid diagram (ADR-35): its code block's [`CodeId`], and the column
    /// the diagram starts at. The spans from there on are the whole line, which paint shows
    /// through the block's window.
    Diagram {
        block: CodeId,
        x: u16,
    },
    /// A [`RowKind::Text`] row of a table whose columns `from..to` take the alternate table-row
    /// background: in the kind, which had the room, rather than a field of [`Row`]'s (MEM-010).
    Striped {
        from: u16,
        to: u16,
    },
}

#[derive(Clone, Debug)]
pub struct Row {
    /// Indices into [`Layout::spans`].
    pub spans: Range<u32>,
    /// The source lines this row renders; `LineRange::NONE` for decoration.
    pub src: LineRange,
    pub kind: RowKind,
}

impl Row {
    /// Columns painted with the alternate table-row background.
    pub fn stripe(&self) -> Option<Range<u16>> {
        match self.kind {
            RowKind::Striped { from, to } => Some(from..to),
            _ => None,
        }
    }
}

/// A link's cells on one row, for keyboard focus (and a future mouse).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub row: u32,
    pub cols: Range<u16>,
    pub link: LinkId,
    /// The row before ends inside the word this one starts with (a break after `-` or `/`, or
    /// in a word too long for the line), so the link's text has no space there.
    pub joined: bool,
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

/// Where a table renders, grid or cards.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableRows {
    pub table: TableId,
    pub rows: Range<u32>,
}

/// A grid table's rows, for the sticky header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridRows {
    /// The header row(s) and the separator under them.
    pub header: Range<u32>,
    /// One past the last body row.
    pub end: u32,
}

/// A table's rows in their sorted order, as indices into [`Table::rows`].
#[derive(Clone, Debug, PartialEq, Eq)]
struct Sorted {
    sort: Sort,
    order: Vec<u32>,
}

/// The width-independent work behind a layout, done once per Doc and glyph set (architecture §1,
/// "Measure"), so that a resize or a table-mode toggle only redistributes.
#[derive(Debug, Default)]
pub struct Measure {
    pub glyphs_gen: u64,
    /// Column measurements, indexed by [`Table::id`]: taken by the first layout that reaches the
    /// table, from the cells it flattens to wrap them (LAY-003).
    tables: Vec<OnceLock<Box<[ColMeasure]>>>,
    /// Image sizes in pixels, indexed by [`LinkId`]; known once their headers have been read.
    images: Vec<Option<(u32, u32)>>,
    /// The order of sorted tables' rows, indexed by [`Table::id`]; `None` keeps the source order.
    sorts: Vec<Option<Sorted>>,
    /// Where the inline text holds a byte to clean (LAY-006).
    marks: wrap::Marks,
}

impl Measure {
    pub fn new(doc: &Doc, glyphs: &Glyphs) -> Self {
        Self {
            glyphs_gen: glyphs.generation(),
            tables: (0..doc.table_count).map(|_| OnceLock::new()).collect(),
            images: vec![None; doc.links.len()],
            sorts: vec![None; doc.table_count as usize],
            marks: wrap::Marks::new(doc),
        }
    }

    /// Some image's size is known.
    pub fn has_images(&self) -> bool {
        self.images.iter().any(Option::is_some)
    }

    /// Measure again for new glyphs, keeping the image sizes and the sorted tables.
    pub fn remeasure(&mut self, doc: &Doc, glyphs: &Glyphs) {
        let images = std::mem::take(&mut self.images);
        let sorts = std::mem::take(&mut self.sorts);
        *self = Self {
            images,
            sorts,
            ..Self::new(doc, glyphs)
        };
    }

    /// How table `t` is sorted, if it is.
    pub fn sort(&self, t: TableId) -> Option<Sort> {
        self.sorted(t).map(|s| s.sort)
    }

    fn sorted(&self, t: TableId) -> Option<&Sorted> {
        self.sorts.get(t as usize)?.as_ref()
    }

    /// Sort `table`'s rows, or put them back in source order. A column the table doesn't have
    /// leaves it in source order.
    pub fn set_sort(&mut self, doc: &Doc, table: &Table, sort: Option<Sort>) {
        let Some(slot) = self.sorts.get_mut(table.id as usize) else {
            return;
        };
        *slot = sort
            .filter(|s| usize::from(s.col) < table.column_count())
            .map(|sort| Sorted {
                sort,
                order: table::order(doc, table, sort),
            });
    }

    /// Record the pixel size of the image `link` shows.
    pub fn set_image(&mut self, link: LinkId, px: Option<(u32, u32)>) {
        if let Some(slot) = self.images.get_mut(link as usize) {
            *slot = px;
        }
    }
}

#[derive(Debug)]
pub struct Layout {
    pub params: LayoutParams,
    /// Columns left of everything else for line numbers, a space after them included; 0 without.
    /// Row columns start after them, as if the terminal were that much narrower.
    pub numbers: u16,
    /// Blank columns on each side; the git gutter draws in the left one.
    pub margin: u16,
    /// The arena all spans point into.
    pub text: String,
    pub spans: Vec<Span>,
    /// Where the last span ends: the text's length once laid out. While a line is laid out, the
    /// text can run ahead of it ([`Builder::emit_line`] copies a run of segs before their spans).
    spans_end: u32,
    pub rows: Vec<Row>,
    /// Top-level block index → its rows.
    pub block_rows: Vec<Range<u32>>,
    /// Anchor → first row (`u32::MAX` if it didn't render, or is a block id's, which
    /// [`Layout::block_row`] finds when a link is followed).
    pub anchor_rows: Vec<u32>,
    pub headings: Vec<HeadingRow>,
    pub hits: Vec<Hit>,
    /// Search units → rows and columns, in screen order: by row, then column.
    pub search: Vec<SearchSeg>,
    /// Grid tables with body rows, in row order.
    pub grids: Vec<GridRows>,
    /// Every table, in row order.
    pub tables: Vec<TableRows>,
    /// Code blocks, indexed by [`CodeId`].
    pub code: Vec<CodeInfo>,
    /// Block images shown as images, in row order.
    pub images: Vec<ImageSlot>,
}

impl Layout {
    /// A layout with nothing in it, as a spare for [`relayout`].
    pub fn empty(params: LayoutParams) -> Self {
        Self {
            params,
            numbers: 0,
            margin: 0,
            text: String::new(),
            spans: Vec::new(),
            spans_end: 0,
            rows: Vec::new(),
            block_rows: Vec::new(),
            anchor_rows: Vec::new(),
            headings: Vec::new(),
            hits: Vec::new(),
            search: Vec::new(),
            grids: Vec::new(),
            tables: Vec::new(),
            code: Vec::new(),
            images: Vec::new(),
        }
    }

    pub fn row_count(&self) -> u32 {
        self.rows.len() as u32
    }

    pub fn spans<'a>(&'a self, row: &Row) -> impl Iterator<Item = (&'a str, &'a Span)> + 'a {
        let (first, last) = (row.spans.start as usize, row.spans.end as usize);
        self.spans[first..last]
            .iter()
            .enumerate()
            .map(move |(k, s)| {
                let end = self
                    .spans
                    .get(first + k + 1)
                    .map_or(self.text.len(), |next| next.start as usize);
                (&self.text[s.start as usize..end], s)
            })
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

    /// The first row of the block whose first search unit is `unit`, which an Obsidian `^id`
    /// names (OBS-009); failing that (a table whose first cell is empty), the first row showing a
    /// later unit. Sought when a link to it is followed, not at every layout: from
    /// [`Layout::search`], so on screen only.
    pub fn block_row(&self, unit: UnitId) -> Option<u32> {
        let exact = self.search.iter().find(|s| s.unit == unit);
        exact
            .or_else(|| self.search.iter().find(|s| s.unit > unit))
            .map(|s| s.row)
    }

    /// The line number `row` shows: the first source line it renders, unless the row before
    /// starts on the same line (a wrapped line, a paragraph's later rows).
    pub fn line_number(&self, row: u32) -> Option<u32> {
        let start = self.rows.get(row as usize)?.src.start;
        let prev = row.checked_sub(1).map(|r| self.rows[r as usize].src.start);
        (start != 0 && prev != Some(start)).then_some(start)
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
    relayout(doc, measure, glyphs, params, Layout::empty(params))
}

/// Lay out a document measured with the same glyphs, reusing the buffers of `spare`, a layout
/// that is no longer needed.
pub fn relayout(
    doc: &Doc,
    measure: &Measure,
    glyphs: &Glyphs,
    params: LayoutParams,
    spare: Layout,
) -> Layout {
    relayout_lines(doc, measure, glyphs, params, spare, doc.line_count(), true)
}

/// [`layout_measured`] for print, which reads only the rows: without [`Layout::search`] and
/// [`Layout::hits`], which only the TUI's search and link focus read (MEM-05: 2–3% of a print's
/// instructions; XL's map is 1.6 MiB, though it sat in heap the peak held anyway).
pub fn layout_print(doc: &Doc, measure: &Measure, glyphs: &Glyphs, params: LayoutParams) -> Layout {
    let spare = Layout::empty(params);
    relayout_lines(doc, measure, glyphs, params, spare, doc.line_count(), false)
}

/// [`layout_print`] for a piece of a longer file printed a piece at a time (O-52): its line
/// numbers take the width the whole file's `lines` need, as they do in the other pieces.
pub fn relayout_piece(
    doc: &Doc,
    glyphs: &Glyphs,
    params: LayoutParams,
    lines: u32,
    spare: Layout,
) -> Layout {
    relayout_lines(
        doc,
        &Measure::new(doc, glyphs),
        glyphs,
        params,
        spare,
        lines,
        false,
    )
}

/// `screen`: also record what only the TUI reads ([`Builder::screen`]).
fn relayout_lines(
    doc: &Doc,
    measure: &Measure,
    glyphs: &Glyphs,
    params: LayoutParams,
    spare: Layout,
    lines: u32,
    screen: bool,
) -> Layout {
    let numbers = number_width(lines, params);
    // the rows lay out beside the numbers; `out.params` keeps the width asked for, which is
    // what relayouts compare
    let inner = LayoutParams {
        width: params.width - numbers,
        ..params
    };
    let margin = if inner.prose_width() >= 24 && !params.bare {
        glyphs.margin
    } else {
        0
    };
    let Layout {
        mut text,
        mut spans,
        mut rows,
        mut block_rows,
        mut anchor_rows,
        mut headings,
        mut hits,
        mut search,
        mut grids,
        mut tables,
        mut code,
        mut images,
        ..
    } = spare;
    text.clear();
    // A layout's text is 1.0–3.2× its source (median 1.4 over the vault's notes past 20 KB): room
    // for twice that up front, so it seldom grows. Grown by doubling, its capacity landed anywhere
    // up to twice its length, set by how the first bytes came, and a large text held both copies
    // while it moved (XL's print +2.7 MiB at O-24, which copies a line at a time). The room it
    // doesn't fill is never touched, so it isn't resident. The source is the last line's start, or
    // a whole file's code when that's longer: a one-line file's last line starts at 0, and its
    // text grew by doubling from a row's 100 bytes, each copy left resident (huge.json's print
    // +3.6 MiB, MEM-014).
    let source = doc.line_starts.last().map_or(0, |&s| s as usize).max(
        match doc.blocks.first().map(|b| &b.kind) {
            Some(BlockKind::Code(code)) if code.whole_file => code.text.len(),
            _ => 0,
        },
    );
    text.reserve(2 * source);
    spans.clear();
    rows.clear();
    block_rows.clear();
    block_rows.reserve(doc.blocks.len());
    anchor_rows.clear();
    anchor_rows.resize(doc.anchors.len(), u32::MAX);
    headings.clear();
    hits.clear();
    search.clear();
    grids.clear();
    tables.clear();
    code.clear();
    code.resize(doc.code_count as usize, CodeInfo::default());
    images.clear();
    let mut b = Builder {
        doc,
        measure,
        g: glyphs,
        params: inner,
        out: Layout {
            params,
            numbers,
            margin,
            text,
            spans,
            spans_end: 0,
            rows,
            block_rows,
            anchor_rows,
            headings,
            hits,
            search,
            grids,
            tables,
            code,
            images,
        },
        screen,
        prefixes: Vec::new(),
        spare: Vec::new(),
        prefix_width: 0,
        right_margin: margin,
        text_key: StyleKey::Text,
        list_depth: 0,
        pin: None,
        row_start: 0,
        col: 0,
        search_col: 0,
        stripe: None,
        striped: false,
        scratch: Wrapped::default(),
        wraps: wrap::Scratch::default(),
        cells: Vec::new(),
        flat: table::Cells::default(),
    };
    if margin > 0 {
        b.push_prefix(&" ".repeat(margin as usize), StyleKey::Text);
    }
    // properties that took no rows (hidden, or none to show) leave no gap
    let gap = |i: usize, b: &Builder<'_>| match i {
        0 => false,
        1 => {
            let rows = b.out.block_rows[0].clone();
            !(rows.is_empty() && matches!(doc.blocks[0].kind, BlockKind::Properties(_)))
        }
        _ => true,
    };
    for (i, block) in doc.blocks.iter().enumerate() {
        if gap(i, &b) {
            b.blank_row();
        }
        let first = b.out.row_count();
        b.block(block);
        b.out.block_rows.push(first..b.out.row_count());
    }
    debug_assert_eq!(
        b.out.spans_end as usize,
        b.out.text.len(),
        "text past the last span"
    );
    b.out
}

/// The columns line numbers take: the last line's digits and a space, when they're on and leave
/// the document as much room as the margin needs.
fn number_width(lines: u32, params: LayoutParams) -> u16 {
    if params.line_numbers == LineNumbers::Off {
        return 0;
    }
    let width = lines.max(1).ilog10() as u16 + 2;
    if params.width >= width + 24 { width } else { 0 }
}

/// `text` in at most `width` columns (but for a `…` in fewer than one), cut with `…` when it's
/// wider. Measured as [`text_width`] measures the whole: a character's own width can be less
/// than it takes beside the next (an emoji, then U+FE0F), and a key cut by those widths came
/// out wider than its column, which underflowed its padding (fuzz, 2026-11-22).
fn fit(text: &str, width: usize) -> String {
    if text_width(text) <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    for c in text.chars() {
        out.push(c);
        if text_width(&out) + 1 > width {
            out.pop();
            break;
        }
    }
    out.push('…');
    out
}

/// The part of an image block's alt text that renders: its search unit's text.
pub fn image_alt(alt: &str) -> &str {
    alt.trim()
}

/// Indentation pushed by a container: a bullet, a quote bar, a heading prefix. Its texts hold
/// what [`push_span`] would write, NBSPs as spaces, so each row copies them unchecked (LAY-002).
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
    /// Whether to record [`Layout::search`] and [`Layout::hits`]: not for print (MEM-05).
    screen: bool,
    prefixes: Vec<Prefix>,
    /// Prefixes popped, kept for [`Builder::push_prefix_of`] to write over: each quote's bar,
    /// item's bullet and callout's icon took three allocations, and quotes' layout spent 7% of
    /// its instructions in `str::replace` (LAY-010).
    spare: Vec<Prefix>,
    prefix_width: u16,
    right_margin: u16,
    /// The base style of prose: `Text`, or `Quote` inside a block quote.
    text_key: StyleKey,
    list_depth: usize,
    /// The source lines of the embed being laid out (OBS-007): every row of a note transcluded
    /// is its embed's line, for the gutter, line numbers, `e` and a reload's anchor alike.
    pin: Option<LineRange>,
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
    /// The last paragraph's wrap, whose buffers the next one reuses.
    scratch: Wrapped,
    /// What the last paragraph's [`wrap::wrap`] kept between flattening and packing (its source
    /// lines, and the masks of its fast path, LAY-007), their buffers the next one's.
    wraps: wrap::Scratch,
    /// Table cells' wraps done with, whose buffers the next row's reuse.
    cells: Vec<Wrapped>,
    /// The table being laid out's cells, flattened (LAY-003); its buffers are the next table's.
    flat: table::Cells,
}

/// A span's style: key, inline markup, link.
type Look = (StyleKey, InlineStyle, Option<LinkId>);

fn push_span(out: &mut Layout, row_start: u32, col: &mut u16, text: &str, width: u16, look: Look) {
    if text.is_empty() {
        return;
    }
    let start = out.text.len() as u32;
    push_spaced(&mut out.text, text);
    let end = out.text.len() as u32;
    push_range(out, row_start, col, start..end, width, look);
}

/// `text` written to the end of `out`, its NBSPs as spaces: a span's text, a prefix's.
fn push_spaced(out: &mut String, text: &str) {
    // 0xa0 ends NBSP's encoding (and others'); most texts don't have it, and memchr is quick
    if text.as_bytes().contains(&0xa0) {
        out.extend(text.chars().map(|c| if c == NBSP { ' ' } else { c }));
    } else {
        out.push_str(text);
    }
}

/// [`push_span`] of `text` in style `key`, which holds no [`NBSP`]: copied unchecked.
fn push_text(
    out: &mut Layout,
    row_start: u32,
    col: &mut u16,
    text: &str,
    width: u16,
    key: StyleKey,
) {
    if text.is_empty() {
        return;
    }
    debug_assert!(!text.contains(NBSP));
    let start = out.text.len() as u32;
    out.text.push_str(text);
    let end = out.text.len() as u32;
    push_range(
        out,
        row_start,
        col,
        start..end,
        width,
        (key, InlineStyle::NONE, None),
    );
}

/// Append the bytes `segs` of `w` cover, one run of them that follow each other in its buffer,
/// with their [`PAD`]s as spaces: as many bytes, so their offsets hold. Copied whole, then every
/// pad in the copy turned into a space in place, a loop that vectorizes (LAY-006; the copy was
/// split at each pad of the pieces' decorations before, and at each `memchr` hit before LAY-002).
/// Only ada's decorations hold a pad: [`clean_char`](wrap::clean_char) makes the source's control
/// characters spaces.
fn push_padded(out: &mut String, w: &Wrapped, segs: &[Seg]) {
    let (Some(first), Some(last)) = (segs.first(), segs.last()) else {
        return;
    };
    let at = out.len();
    out.push_str(&w.buf[first.start as usize..last.end as usize]);
    // SAFETY: a PAD and a space are each one byte of ASCII, so the text stays UTF-8
    let copy = unsafe { &mut out.as_mut_vec()[at..] };
    for b in copy {
        *b = if *b == PAD as u8 { b' ' } else { *b };
    }
}

/// A span over `start..end` of [`Layout::text`], already there, which starts where the last span
/// ends; merged into the row's last span when that one has the same look.
fn push_range(
    out: &mut Layout,
    row_start: u32,
    col: &mut u16,
    Range { start, end }: Range<u32>,
    width: u16,
    look: Look,
) {
    let (key, inline, link) = look;
    let link = link.unwrap_or(NO_LINK);
    *col = col.saturating_add(width);
    // the spans keep only their starts (MEM-010), so a gap would show in the span before it
    debug_assert_eq!(start, out.spans_end, "a gap between spans");
    out.spans_end = end;
    if out.spans.len() as u32 > row_start
        && let Some(last) = out.spans.last_mut()
        && (last.key, last.inline, last.link) == (key, inline, link)
    {
        last.width = last.width.saturating_add(width);
        return;
    }
    out.spans.push(Span {
        start,
        link,
        width,
        key,
        inline,
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
        self.push_prefix_of(&[first], key);
    }

    /// A prefix of `parts` written one after the other, in a prefix popped before if there is
    /// one: its strings are written over, not allocated again (LAY-010).
    fn push_prefix_of(&mut self, parts: &[&str], key: StyleKey) {
        let mut p = self.spare.pop().unwrap_or_else(|| Prefix {
            first: String::new(),
            rest: String::new(),
            key,
            width: 0,
            used: false,
        });
        p.first.clear();
        let mut width = 0;
        for part in parts {
            width += part.width() as u16;
            push_spaced(&mut p.first, part);
        }
        p.rest.clear();
        p.rest.extend(std::iter::repeat_n(' ', usize::from(width)));
        (p.key, p.width, p.used) = (key, width, false);
        self.prefixes.push(p);
        self.prefix_width += width;
    }

    /// A prefix that repeats on every row, like a quote bar.
    fn push_bar(&mut self, bar: &str, key: StyleKey) {
        self.push_prefix(bar, key);
        if let Some(p) = self.prefixes.last_mut() {
            p.rest.clear();
            push_spaced(&mut p.rest, bar);
        }
    }

    fn pop_prefix(&mut self) {
        if let Some(p) = self.prefixes.pop() {
            self.prefix_width -= p.width;
            self.spare.push(p);
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
            push_text(out, *row_start, col, text, p.width, p.key);
            p.used = true;
        }
    }

    fn end_row(&mut self, src: LineRange, kind: RowKind) {
        let kind = match (self.stripe.take(), kind) {
            (Some(cols), RowKind::Text) => RowKind::Striped {
                from: cols.start,
                to: cols.end,
            },
            (stripe, kind) => {
                debug_assert!(stripe.is_none(), "a stripe on a {kind:?} row");
                kind
            }
        };
        self.out.rows.push(Row {
            spans: self.row_start..self.out.spans.len() as u32,
            src: self.pin.unwrap_or(src),
            kind,
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
            push_text(out, *row_start, col, &p.rest, p.width, p.key);
        }
        self.end_row(LineRange::NONE, RowKind::Blank);
    }

    fn text(&mut self, text: &str, key: StyleKey) {
        let width = text_width(text) as u16;
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
        self.spaces(target.saturating_sub(self.col), key);
    }

    /// `n` spaces, without building them (LAY-01: a table's cells are padded on every row), or
    /// looking for an NBSP in them (LAY-004).
    fn spaces(&mut self, n: u16, key: StyleKey) {
        const SPACES: &str = "                                                                ";
        if n == 0 {
            return;
        }
        let start = self.out.text.len() as u32;
        let mut left = usize::from(n);
        while left > 0 {
            let k = left.min(SPACES.len());
            self.out.text.push_str(&SPACES[..k]);
            left -= k;
        }
        let end = self.out.text.len() as u32;
        let look = (key, InlineStyle::NONE, None);
        push_range(
            &mut self.out,
            self.row_start,
            &mut self.col,
            start..end,
            n,
            look,
        );
    }

    /// `text`, `width` columns wide and without an [`NBSP`]: a glyph of ada's own, whose width is
    /// known (LAY-004: a table measured every border it drew).
    fn glyph(&mut self, text: &str, width: u16, key: StyleKey) {
        debug_assert!(!text.contains(NBSP) && text_width(text) == usize::from(width));
        let start = self.out.text.len() as u32;
        self.out.text.push_str(text);
        let end = self.out.text.len() as u32;
        let look = (key, InlineStyle::NONE, None);
        push_range(
            &mut self.out,
            self.row_start,
            &mut self.col,
            start..end,
            width,
            look,
        );
    }

    /// `glyph`, `width` columns wide, `n` times, written into the text as it goes (LAY-004).
    fn repeat(&mut self, glyph: &str, width: u16, n: u16, key: StyleKey) {
        if n == 0 || glyph.is_empty() {
            return;
        }
        // 0xa0 ends NBSP's encoding, as in `push_span`
        if glyph.as_bytes().contains(&0xa0) {
            self.text(&glyph.repeat(usize::from(n)), key);
            return;
        }
        let text = &mut self.out.text;
        let (start, len, count) = (text.len(), glyph.len(), usize::from(n));
        text.reserve(len * count);
        text.push_str(glyph);
        // doubling: a copy a power of two, not a copy a glyph
        let mut have = 1;
        while have < count {
            let k = have.min(count - have);
            text.extend_from_within(start..start + k * len);
            have += k;
        }
        let (start, end) = (start as u32, text.len() as u32);
        let look = (key, InlineStyle::NONE, None);
        let width = width.saturating_mul(n);
        push_range(
            &mut self.out,
            self.row_start,
            &mut self.col,
            start..end,
            width,
            look,
        );
    }

    /// Repeat `glyph` up to `width` columns past the prefixes.
    fn fill_with(&mut self, glyph: &str, width: u16, key: StyleKey) {
        let target = self.prefix_width.saturating_add(width);
        let gw = text_width(glyph) as u16;
        if self.col < target {
            self.repeat(glyph, gw, (target - self.col) / gw.max(1), key);
        }
    }

    fn emit_line(&mut self, w: &Wrapped, line: &Line) {
        let segs = w.line_segs(line);
        // Segs that follow each other in the buffer are copied into the text in one go, and
        // their spans are ranges of that copy (O-24): `copied` segs so far, and the last copy's
        // start in the text and in the buffer. A run of spaces between words keeps only its
        // first, so a line can take more than one copy. A buffer with an NBSP of the source's,
        // shorter as a space, takes the copy a seg at a time.
        let (mut copied, mut to, mut from_buf) = (0, 0, 0);
        for (i, seg) in segs.iter().enumerate() {
            let piece = &w.pieces[seg.piece as usize];
            let from = self.col;
            if let Some(unit) = w.unit.filter(|_| self.screen) {
                let (a, b) = (seg.start.max(piece.body_start), seg.end.min(piece.body_end));
                if a < b {
                    // the body's columns: an ASCII seg's are its bytes' (LAY-006)
                    let width = |x: u32, y: u32| match seg.ascii {
                        true => (y - x) as u16,
                        false => text_width(&w.buf[x as usize..y as usize]) as u16,
                    };
                    let (col, end) = if (a, b) == (seg.start, seg.end) {
                        (from, from + seg.width)
                    } else {
                        let col = from + width(seg.start, a);
                        (col, col + width(a, b))
                    };
                    let at = |x: u32| piece.unit_off + (x - piece.body_start);
                    self.search_seg(unit, at(a)..at(b), col..end);
                }
            }
            let look = (piece.key, piece.inline, piece.link);
            if w.nbsp {
                let text = w.seg_text(seg);
                let text = match w.pads {
                    true => &text.replace(PAD, " "),
                    false => text,
                };
                push_span(
                    &mut self.out,
                    self.row_start,
                    &mut self.col,
                    text,
                    seg.width,
                    look,
                );
            } else {
                if i == copied {
                    let mut end = i + 1;
                    while end < segs.len() && segs[end].start == segs[end - 1].end {
                        end += 1;
                    }
                    (to, from_buf) = (self.out.text.len() as u32, seg.start);
                    match w.pads {
                        true => push_padded(&mut self.out.text, w, &segs[i..end]),
                        false => {
                            let text = &w.buf[seg.start as usize..segs[end - 1].end as usize];
                            self.out.text.push_str(text);
                        }
                    }
                    copied = end;
                }
                let start = to + (seg.start - from_buf);
                let range = start..start + (seg.end - seg.start);
                push_range(
                    &mut self.out,
                    self.row_start,
                    &mut self.col,
                    range,
                    seg.width,
                    look,
                );
            }
            if let Some(link) = piece.link {
                // no bytes between the line before and this one: the break was inside a word
                let joined = i == 0
                    && (line.segs.start as usize)
                        .checked_sub(1)
                        .is_some_and(|before| w.segs[before].end == seg.start);
                self.hit(from, self.col, link, joined);
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
        if !self.screen {
            return;
        }
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

    fn hit(&mut self, from: u16, to: u16, link: LinkId, joined: bool) {
        if !self.screen {
            return;
        }
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
            joined,
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
        // one call of `code::layout`, a properties block's source through it too: a second took it
        // out of line, and fenced code's layout +3.4% in instructions
        let code = match &block.kind {
            BlockKind::Code(code) => code,
            BlockKind::Properties(p) => match self.properties(p) {
                Some(source) => source,
                None => return,
            },
            _ => return self.other_block(block),
        };
        code::layout(self, code, block.src);
    }

    fn other_block(&mut self, block: &Block) {
        match &block.kind {
            BlockKind::Heading {
                level,
                inlines,
                anchor,
            } => self.heading(*level, inlines, *anchor),
            BlockKind::Paragraph(inlines) => {
                self.paragraph(self.doc.runs(inlines), inlines.unit, self.text_key, None);
            }
            BlockKind::Math(inlines) => self.math(inlines),
            BlockKind::List(list) => self.list(list),
            BlockKind::Quote { callout, blocks } => {
                self.quote(callout.as_ref(), blocks, block.src);
            }
            BlockKind::Code(_) | BlockKind::Properties(_) => {}
            BlockKind::Table(table) => table::layout(self, table),
            BlockKind::Image { link, alt, unit } => self.image(*link, alt, *unit, block.src),
            BlockKind::Rule => self.rule(block.src),
            BlockKind::Html(html) => self.html(html, block.src),
            BlockKind::FootnoteDefs => self.footnotes(),
            BlockKind::Embed(e) => self.embed(e, block.src),
        }
    }

    /// A note's properties (ADR-46): a compact block of dim `key  value` rows, without borders,
    /// the keys in a column as wide as the widest (at most a third of the prose width, longer
    /// ones cut), each value wrapped under itself; past [`MAX_PROPERTIES`], a row says how many
    /// more. `source` mode, and a block ada couldn't read, show the front matter as written: the
    /// code block returned, which [`Builder::block`] lays out.
    fn properties<'p>(&mut self, p: &'p Properties) -> Option<&'p CodeBlock> {
        match self.params.properties {
            PropertiesMode::Hide => return None,
            mode if mode.as_written(p) => return Some(&p.source),
            _ => {}
        }
        let shown = match p.rows.len() {
            n if n <= MAX_PROPERTIES + 1 => n,
            _ => MAX_PROPERTIES,
        };
        let keys: Vec<String> = p.rows[..shown]
            .iter()
            .map(|r| wrap::clean(&r.key))
            .collect();
        let cap = usize::from(self.avail() / 3).max(4);
        let width = keys
            .iter()
            .map(|k| text_width(k))
            .max()
            .unwrap_or(0)
            .min(cap);
        for (row, key) in p.rows[..shown].iter().zip(keys) {
            let mut key = fit(&key, width);
            let pad = width.saturating_sub(text_width(&key));
            key.extend(std::iter::repeat_n(' ', pad + 2));
            self.push_prefix(&key, StyleKey::PropertyKey);
            let runs = self.doc.runs(&row.value);
            if runs.is_empty() {
                self.begin_row();
                self.end_row(row.src, RowKind::Text);
            } else {
                self.property_value(runs, row.value.unit);
            }
            self.pop_prefix();
        }
        if let [first, .., last] | [first @ last] = &p.rows[shown..] {
            let more = format!("+{} more", p.rows.len() - shown);
            let more = fit(&more, usize::from(self.avail()));
            self.begin_row();
            self.text(&more, StyleKey::PropertyKey);
            self.end_row(LineRange::new(first.src.start, last.src.end), RowKind::Text);
        }
        None
    }

    /// A property's value, wrapped under itself in at most [`MAX_VALUE_ROWS`] rows.
    fn property_value(&mut self, runs: &[Run], unit: UnitId) {
        let avail = self.avail();
        let into = std::mem::take(&mut self.scratch).recycle(Some(unit));
        let doc = (self.doc, Some(&self.measure.marks));
        let key = StyleKey::PropertyValue;
        let mut w = wrap::wrap(into, &mut self.wraps, runs, key, avail, doc, self.g, None);
        let all = w.lines.len();
        let shown = match all {
            n if n <= MAX_VALUE_ROWS => n,
            _ => MAX_VALUE_ROWS - 1,
        };
        let cut = (w.lines[shown..].iter()).fold(LineRange::NONE, |r, l| r.union(l.src));
        // through `emit_lines`, as a paragraph's: a second caller of `emit_line` took it out of
        // line, and fenced code's layout +3.4% in instructions
        w.lines.truncate(shown);
        self.emit_lines(&w);
        if shown < all {
            let more = format!("+{} more lines", all - shown);
            let more = fit(&more, usize::from(avail));
            self.begin_row();
            self.text(&more, StyleKey::PropertyKey);
            self.end_row(cut, RowKind::Text);
        }
        self.scratch = w;
    }

    fn paragraph(&mut self, runs: &[Run], unit: UnitId, key: StyleKey, suffix: Option<&str>) {
        let avail = self.avail();
        let into = std::mem::take(&mut self.scratch).recycle(Some(unit));
        let doc = (self.doc, Some(&self.measure.marks));
        let w = wrap::wrap(into, &mut self.wraps, runs, key, avail, doc, self.g, suffix);
        self.emit_lines(&w);
        self.scratch = w;
    }

    /// Display math typeset (OBS-003): its rows set as a block in the middle of the prose width,
    /// their alignment kept; a row wider than that wraps as a paragraph's line does.
    fn math(&mut self, inlines: &Inlines) {
        let runs = self.doc.runs(inlines);
        let widest = (runs.iter())
            .map(|r| text_width(self.doc.run_text(r)))
            .max()
            .unwrap_or(0);
        let indent = usize::from(self.avail()).saturating_sub(widest) / 2;
        if indent > 0 {
            self.push_prefix(&" ".repeat(indent), self.text_key);
        }
        self.paragraph(runs, inlines.unit, self.text_key, None);
        if indent > 0 {
            self.pop_prefix();
        }
    }

    fn heading(&mut self, level: u8, inlines: &Inlines, anchor: AnchorId) {
        let row = self.out.row_count();
        if let Some(slot) = self.out.anchor_rows.get_mut(anchor as usize) {
            *slot = row;
        }
        // a transcluded note's headings are its own: not the outline's, nor the heading path's
        if self.pin.is_none() {
            self.out.headings.push(HeadingRow { row, level, anchor });
        }
        let key = StyleKey::heading(level);
        let g = self.g;
        let (prefix, suffix) = &g.heading[usize::from(level.clamp(1, 6) - 1)];
        self.push_prefix(prefix, key);
        self.paragraph(self.doc.runs(inlines), inlines.unit, key, Some(suffix));
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
                (Some(task), _) => {
                    let (glyph, key) = g.task_box(task);
                    (glyph.to_owned(), key)
                }
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

    fn quote(&mut self, callout: Option<&Callout>, blocks: &[Block], src: LineRange) {
        let g = self.g;
        self.push_bar(&g.quote_bar, StyleKey::QuoteBar);
        let saved = std::mem::replace(&mut self.text_key, StyleKey::Quote);
        if let Some(callout) = callout {
            self.callout_title(callout);
        }
        self.blocks(blocks, false);
        if blocks.is_empty() && callout.is_none() {
            self.begin_row();
            self.end_row(src, RowKind::Text);
        }
        self.text_key = saved;
        self.pop_prefix();
    }

    /// A note transcluded (OBS-007): behind a quote's bar, the embed's link to it (`↳ note`),
    /// then its blocks, without its properties, in the prose's style around it. Its rows are the
    /// embed's source lines ([`Builder::pin`]), the outermost embed's when they nest.
    fn embed(&mut self, e: &Embed, src: LineRange) {
        let g = self.g;
        let outer = self.pin.is_none();
        if outer {
            self.pin = Some(src);
        }
        self.push_bar(&g.quote_bar, StyleKey::QuoteBar);
        let runs = self.doc.runs(&e.title);
        self.paragraph(runs, e.title.unit, self.text_key, None);
        let blocks = match e.blocks.first().map(|b| &b.kind) {
            Some(BlockKind::Properties(_)) => &e.blocks[1..],
            _ => &e.blocks[..],
        };
        self.blocks(blocks, false);
        self.pop_prefix();
        if outer {
            self.pin = None;
        }
    }

    /// A callout's title (OBS-002): its kind's icon, then the title in the kind's color, wrapped
    /// under itself, and the fold mark of one that folds, open as the callout is shown.
    fn callout_title(&mut self, callout: &Callout) {
        let g = self.g;
        let key = StyleKey::callout(callout.kind);
        let icon = &g.callout[callout.kind as usize];
        let fold = callout.fold.map(|_| format!(" {}", g.fold[0]));
        match icon.is_empty() {
            true => self.push_prefix("", key),
            false => self.push_prefix_of(&[icon, " "], key),
        }
        let runs = self.doc.runs(&callout.title);
        self.paragraph(runs, callout.title.unit, key, fold.as_deref());
        self.pop_prefix();
    }

    /// A block image: reserved rows and a caption when its size is known and the terminal can
    /// show images, otherwise a `🖼 alt (path)` line. Obsidian's size for it (`|100`, OBS-008)
    /// caps its pixels, as the terminal's cells measure them.
    fn image(&mut self, link: LinkId, alt: &str, unit: UnitId, src: LineRange) {
        let link_ref = self.doc.links.get(link as usize);
        let dest = link_ref.map_or("", |l| &l.dest);
        let alt = image_alt(alt);
        let px = self.measure.images.get(link as usize).copied().flatten();
        let px = px.map(|px| link_ref.map_or(px, |l| l.size.cap(px)));
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
