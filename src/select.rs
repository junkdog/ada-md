//! Selecting text with the mouse (plan §4, ADR-38). A selection is two places in the document's
//! text, not on the screen: a search unit and a byte in it (ADR-12), so it survives scrolling,
//! resize and the table toggle, and what it copies is the text the document holds rather than
//! the cells that show it. A paragraph comes unwrapped, code from its source lines without
//! padding, line numbers or wrap marks, list items with their markers, links as their text.
//! Cells across a table copy as TSV, or as a GFM table once whole rows and the header are in,
//! and a record card's as `header: value` lines.
//!
//! [`View`] measures a selection against one layout: the segments of [`Layout::search`] it
//! covers, for paint, and the text, for the clipboard. It relies on those segments being in
//! screen order.

use std::fmt::Write as _;
use std::ops::Range;

use unicode_width::UnicodeWidthChar;

use crate::doc::{Align, Block, BlockKind, CodeBlock, CodeId, Doc, Table, UnitId};
use crate::layout::{Layout, RowKind, SearchSeg, TAB_STOP};
use crate::search::SearchMap;

/// Copies are cut to this many bytes. OSC 52 sends them in base64, a third longer, and tmux and
/// some terminals drop a sequence much past 100 KB.
pub const MAX_COPY: usize = 74 * 1024;

/// A place in the document's text: byte `off` of a search unit's text. A unit can show more than
/// once (a record card's key on every card), so `nth` says which of the segments that hold `off`
/// it is in, in screen order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pos {
    pub unit: UnitId,
    pub off: u32,
    pub nth: u16,
}

/// From where the selecting started to where it is now, either way round.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Pos,
    pub head: Pos,
}

/// What a selection is measured against: a document, its searchable text, one layout of it, and
/// the horizontal offset of no-wrap code and diagrams.
#[derive(Clone, Copy)]
pub struct View<'a> {
    pub doc: &'a Doc,
    pub map: &'a SearchMap,
    pub layout: &'a Layout,
    pub code_x: u16,
}

/// A [`Pos`] on the layout: a segment of [`Layout::search`], and a byte of its unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct At {
    seg: usize,
    off: u32,
}

/// What a selection covers on a layout.
#[derive(Debug, Default)]
pub struct Picked {
    /// Segments of [`Layout::search`] by index, with the bytes of their unit that are in, in
    /// screen order.
    pub segs: Vec<(usize, Range<u32>)>,
    tables: Vec<TablePick>,
    /// Within one unit: copied as its text alone, a diagram's label included.
    within: bool,
}

impl Picked {
    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }
}

/// Cells of a table: rows by their position in the order shown, columns by index.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TablePick {
    /// Index into [`Layout::tables`].
    at: usize,
    rows: Range<usize>,
    cols: Range<u16>,
}

/// How a copied table was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableFormat {
    Tsv,
    Markdown,
    Records,
}

/// What copying a selection puts on the clipboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Copied {
    pub text: String,
    /// When the selection was cells of one table: how many rows and columns, and how written.
    pub table: Option<(usize, usize, TableFormat)>,
}

impl<'a> View<'a> {
    fn segs(&self) -> &'a [SearchSeg] {
        &self.layout.search
    }

    fn seg_text(&self, s: &SearchSeg) -> &'a str {
        let text = self.map.unit_text(s.unit);
        text.get(s.start as usize..s.end as usize).unwrap_or("")
    }

    /// For a segment in no-wrap code or a diagram: the screen column its block's window starts
    /// at, and how far the shared offset shifts the content left.
    fn window(&self, s: &SearchSeg) -> Option<(u16, u16)> {
        let row = self.layout.rows.get(s.row as usize)?;
        let (start, block) = match row.kind {
            RowKind::Code { block, .. } if !self.layout.params.code_wrap => (s.col, block),
            RowKind::Diagram { block, x } => (x, block),
            _ => return None,
        };
        let info = self.layout.code.get(block as usize).copied();
        Some((start, self.code_x.min(info.unwrap_or_default().max_x())))
    }

    /// The place under the layout's row `row`, column `col` (counted from where rows start,
    /// right of any line numbers), or when nothing is there, the nearest place with text: on
    /// the row, or the first row after it with any. `after` takes the place after the character
    /// under the column, for the far end of a selection, and the nearest before it.
    pub fn place(&self, row: u32, col: u16, after: bool) -> Option<Pos> {
        self.near(row, col, after).map(|at| self.pos(at))
    }

    /// The far end of a selection from `anchor`, at the layout's row `row`, column `col`: the
    /// character there is in, whichever side of the anchor it is.
    pub fn head(&self, anchor: Pos, row: u32, col: u16) -> Option<Pos> {
        let before = self.near(row, col, false)?;
        let forward = self.find(anchor).is_none_or(|a| before >= a);
        let at = if forward {
            self.near(row, col, true)?
        } else {
            before
        };
        Some(self.pos(at))
    }

    fn near(&self, row: u32, col: u16, after: bool) -> Option<At> {
        if let Some(at) = self.at_cell(row, col, after) {
            return Some(at);
        }
        let segs = self.segs();
        let i = segs.partition_point(|s| s.row < row);
        let last = |j: usize| At {
            seg: j,
            off: segs[j].end,
        };
        let first = |j: usize| At {
            seg: j,
            off: segs[j].start,
        };
        if after {
            i.checked_sub(1)
                .map(last)
                .or_else(|| (!segs.is_empty()).then(|| first(0)))
        } else {
            (i < segs.len())
                .then(|| first(i))
                .or_else(|| segs.len().checked_sub(1).map(last))
        }
    }

    /// The place at a cell of a row with text: in the segment under it, or past the end of the
    /// one before it, or at the start of the one after.
    fn at_cell(&self, row: u32, col: u16, after: bool) -> Option<At> {
        let segs = self.segs();
        let first = segs.partition_point(|s| s.row < row);
        let end = first + segs[first..].partition_point(|s| s.row <= row);
        if first == end {
            return None;
        }
        for i in first..end {
            let s = &segs[i];
            let c = match self.window(s) {
                Some((start, x)) if col >= start => col.saturating_add(x),
                _ => col,
            };
            if c < s.col {
                return Some(match i.checked_sub(1).filter(|&p| after && p >= first) {
                    Some(p) => At {
                        seg: p,
                        off: segs[p].end,
                    },
                    None => At {
                        seg: i,
                        off: s.start,
                    },
                });
            }
            let text = self.seg_text(s);
            let mut x = s.col;
            let mut chars = text.char_indices().peekable();
            while let Some((b, ch)) = chars.next() {
                let w = ch.width().unwrap_or(0) as u16;
                if w > 0 && c < x.saturating_add(w) {
                    let off = if after {
                        // past the character and the marks that combine with it
                        while chars.next_if(|&(_, n)| n.width() == Some(0)).is_some() {}
                        chars.peek().map_or(text.len(), |&(n, _)| n)
                    } else {
                        b
                    };
                    return Some(At {
                        seg: i,
                        off: s.start + off as u32,
                    });
                }
                x = x.saturating_add(w);
            }
        }
        let last = end - 1;
        Some(At {
            seg: last,
            off: segs[last].end,
        })
    }

    fn holds(s: &SearchSeg, unit: UnitId, off: u32) -> bool {
        s.unit == unit && s.start <= off && off <= s.end
    }

    fn pos(&self, at: At) -> Pos {
        let s = &self.segs()[at.seg];
        let nth = (self.segs()[..at.seg].iter())
            .filter(|t| Self::holds(t, s.unit, at.off))
            .count();
        Pos {
            unit: s.unit,
            off: at.off,
            nth: nth.min(usize::from(u16::MAX)) as u16,
        }
    }

    /// Where `pos` is on this layout: its `nth` segment holding it, or the last there is.
    fn find(&self, pos: Pos) -> Option<At> {
        let mut found = None;
        let holding = (self.segs().iter().enumerate())
            .filter(|(_, s)| Self::holds(s, pos.unit, pos.off))
            .map(|(i, _)| i);
        for (n, i) in holding.enumerate() {
            found = Some(i);
            if n >= usize::from(pos.nth) {
                break;
            }
        }
        found.map(|seg| At { seg, off: pos.off })
    }

    /// The place `off` in the unit of segment `near`, in the segment nearest it that holds it.
    fn pos_near(&self, near: usize, off: u32) -> Pos {
        let segs = self.segs();
        let unit = segs[near].unit;
        let after = (near..segs.len()).take_while(|&i| segs[i].unit == unit);
        let before = (0..near).rev().take_while(|&i| segs[i].unit == unit);
        let seg = after
            .chain(before)
            .find(|&i| Self::holds(&segs[i], unit, off))
            .unwrap_or(near);
        self.pos(At { seg, off })
    }

    /// Double-click: the word at the layout's row `row`, column `col`, or the run of spaces, or
    /// the one other character.
    pub fn word(&self, row: u32, col: u16) -> Option<Selection> {
        let at = self.near(row, col, false)?;
        let unit = self.segs()[at.seg].unit;
        let text = self.map.unit_text(unit);
        let off = (at.off as usize).min(text.len());
        let (i, c) = match text[off..].chars().next() {
            Some(c) => (off, c),
            None => text[..off].char_indices().next_back()?,
        };
        let class = |c: char| {
            if c.is_alphanumeric() || c == '_' {
                0
            } else if c == ' ' || c == '\t' {
                1
            } else {
                2
            }
        };
        let k = class(c);
        let (start, end) = if k == 2 {
            (i, i + c.len_utf8())
        } else {
            let same = |c: char| class(c) == k;
            let start = text[..i]
                .char_indices()
                .rev()
                .take_while(|&(_, c)| same(c))
                .last()
                .map_or(i, |(j, _)| j);
            let end = text[i..]
                .char_indices()
                .find(|&(_, c)| !same(c))
                .map_or(text.len(), |(j, _)| i + j);
            (start, end)
        };
        Some(Selection {
            anchor: self.pos_near(at.seg, start as u32),
            head: self.pos_near(at.seg, end as u32),
        })
    }

    /// Triple-click: the whole unit at the layout's row `row`, column `col`: a paragraph, a
    /// heading, a code line, a cell.
    pub fn unit(&self, row: u32, col: u16) -> Option<Selection> {
        let at = self.near(row, col, false)?;
        let len = self.map.unit_text(self.segs()[at.seg].unit).len() as u32;
        Some(Selection {
            anchor: self.pos_near(at.seg, 0),
            head: self.pos_near(at.seg, len),
        })
    }

    /// The end of the character at `at`, for an anchor the selection runs back from.
    fn past(&self, at: At) -> At {
        let s = &self.segs()[at.seg];
        let text = self.map.unit_text(s.unit);
        let next = text
            .get(at.off as usize..)
            .and_then(|t| t.chars().next())
            .map_or(0, |c| c.len_utf8() as u32);
        At {
            seg: at.seg,
            off: (at.off + next).min(s.end),
        }
    }

    /// The table drawn on layout row `row`, as an index into [`Layout::tables`].
    fn table_at(&self, row: u32) -> Option<usize> {
        let tables = &self.layout.tables;
        let i = tables.partition_point(|t| t.rows.end <= row);
        tables.get(i).filter(|t| t.rows.contains(&row)).map(|_| i)
    }

    /// What `sel` covers: the text between its ends within one unit; the cells between them
    /// when both are in one table; otherwise everything between, with whole rows of the tables
    /// it runs into.
    pub fn pick(&self, sel: &Selection) -> Picked {
        let (Some(a), Some(h)) = (self.find(sel.anchor), self.find(sel.head)) else {
            return Picked::default();
        };
        let (lo, hi) = if h < a { (h, self.past(a)) } else { (a, h) };
        if lo == hi {
            return Picked::default();
        }
        let segs = self.segs();
        let clip = |i: usize| {
            let s = &segs[i];
            let a = if i == lo.seg {
                lo.off.max(s.start)
            } else {
                s.start
            };
            let b = if i == hi.seg {
                hi.off.min(s.end)
            } else {
                s.end
            };
            (i, a..b.max(a))
        };
        let nonempty = |(_, r): &(usize, Range<u32>)| !r.is_empty();
        let unit = segs[lo.seg].unit;
        let (tl, th) = (
            self.table_at(segs[lo.seg].row),
            self.table_at(segs[hi.seg].row),
        );
        // one unit: on its own, or a cell that wraps, among its neighbors' rows (a record card's
        // key shows on every card, so both ends must be on one)
        let within = segs[hi.seg].unit == unit
            && (segs[lo.seg..=hi.seg].iter().all(|s| s.unit == unit)
                || tl
                    .filter(|_| tl == th)
                    .and_then(|t| Shown::of(self, t))
                    .is_some_and(|sh| {
                        sh.cell(self, &segs[lo.seg]).0 == sh.cell(self, &segs[hi.seg]).0
                    }));
        if within {
            return Picked {
                segs: (lo.seg..=hi.seg)
                    .filter(|&i| segs[i].unit == unit)
                    .map(clip)
                    .filter(nonempty)
                    .collect(),
                tables: Vec::new(),
                within: true,
            };
        }
        if let Some(t) = tl.filter(|_| tl == th)
            && let Some(shown) = Shown::of(self, t)
        {
            let (ra, ca) = shown.cell(self, &segs[lo.seg]);
            let (rb, cb) = shown.cell(self, &segs[hi.seg]);
            let pick = TablePick {
                at: t,
                rows: ra.min(rb)..ra.max(rb) + 1,
                cols: ca.min(cb)..ca.max(cb) + 1,
            };
            return Picked {
                segs: shown.segs(self, &pick),
                tables: vec![pick],
                within: false,
            };
        }
        let mut out = Vec::new();
        let mut tables = Vec::new();
        let mut i = lo.seg;
        while i <= hi.seg {
            let shown = self.table_at(segs[i].row).and_then(|t| Shown::of(self, t));
            let Some(shown) = shown else {
                out.push(clip(i));
                i += 1;
                continue;
            };
            let end = self.layout.tables[shown.at].rows.end;
            let last = i + segs[i..=hi.seg].partition_point(|s| s.row < end) - 1;
            let (ra, _) = shown.cell(self, &segs[i]);
            let (rb, _) = shown.cell(self, &segs[last]);
            let pick = TablePick {
                at: shown.at,
                rows: ra.min(rb)..ra.max(rb) + 1,
                cols: 0..shown.columns,
            };
            out.extend(shown.segs(self, &pick));
            tables.push(pick);
            i = last + 1;
        }
        out.retain(nonempty);
        Picked {
            segs: out,
            tables,
            within: false,
        }
    }

    /// The cells `picked` covers on layout rows `rows`: each row with its columns, shifted as
    /// no-wrap code and diagrams are, counted from where rows start.
    pub fn cells(&self, picked: &Picked, rows: Range<u32>) -> Vec<(u32, Range<u16>)> {
        let segs = self.segs();
        let first = picked
            .segs
            .partition_point(|(i, _)| segs[*i].row < rows.start);
        let mut out: Vec<(u32, Range<u16>, UnitId)> = Vec::new();
        for (i, r) in &picked.segs[first..] {
            let s = &segs[*i];
            if s.row >= rows.end {
                break;
            }
            let text = self.map.unit_text(s.unit);
            let width = |to: u32| {
                (text.get(s.start as usize..to as usize)).map_or(0, |t| {
                    t.chars()
                        .map(|c| c.width().unwrap_or(0) as u16)
                        .sum::<u16>()
                })
            };
            let mut cols = s.col + width(r.start)..s.col + width(r.end);
            if let Some((start, x)) = self.window(s) {
                let info = match self.layout.rows[s.row as usize].kind {
                    RowKind::Code { block, .. } | RowKind::Diagram { block, .. } => {
                        self.layout.code.get(block as usize).copied()
                    }
                    _ => None,
                };
                let end = start + info.unwrap_or_default().window;
                let shift = |c: u16| c.saturating_sub(x).clamp(start, end);
                cols = shift(cols.start)..shift(cols.end);
            }
            if cols.is_empty() {
                continue;
            }
            // what lies between two stretches of one unit on a row, such as inline code's
            // padding, is in too
            match out.last_mut() {
                Some((row, prev, unit)) if *row == s.row && *unit == s.unit => {
                    prev.end = prev.end.max(cols.end);
                }
                _ => out.push((s.row, cols, s.unit)),
            }
        }
        out.into_iter().map(|(row, cols, _)| (row, cols)).collect()
    }

    /// What copying `sel` puts on the clipboard, not yet cut to [`MAX_COPY`].
    pub fn copy(&self, sel: &Selection) -> Copied {
        let picked = self.pick(sel);
        let table = match (&picked.tables[..], picked.segs.first(), picked.segs.last()) {
            ([t], Some((a, _)), Some((b, _))) => {
                let rows = &self.layout.tables[t.at].rows;
                let only =
                    rows.contains(&self.segs()[*a].row) && rows.contains(&self.segs()[*b].row);
                only.then(|| Shown::of(self, t.at))
                    .flatten()
                    .map(|s| (t.rows.len(), t.cols.len(), s.format(t)))
            }
            _ => None,
        };
        Copied {
            text: self.text(&picked),
            table,
        }
    }

    fn text(&self, picked: &Picked) -> String {
        let segs = self.segs();
        let roles = Roles::of(self.doc);
        let codes = self.doc.code_blocks();
        let mut drawn = vec![false; codes.len()];
        for row in &self.layout.rows {
            if let RowKind::Diagram { block, .. } = row.kind
                && let Some(d) = drawn.get_mut(block as usize)
            {
                *d = true;
            }
        }
        let diagram_of = |s: &SearchSeg| match self.layout.rows[s.row as usize].kind {
            RowKind::Diagram { block, .. } => Some(block),
            _ => roles
                .code(s.unit)
                .map(|(block, _)| block)
                .filter(|&b| drawn.get(b as usize).copied().unwrap_or(false)),
        };

        let mut out = String::new();
        let mut prev: Option<Item> = None;
        let mut k = 0;
        while k < picked.segs.len() {
            let (i, r) = &picked.segs[k];
            let s = &segs[*i];
            let (item, text) = if let Some(t) =
                (picked.tables.iter()).find(|t| self.layout.tables[t.at].rows.contains(&s.row))
            {
                // a table, whole, as its cells are picked
                let rows = self.layout.tables[t.at].rows.clone();
                while k < picked.segs.len() && rows.contains(&segs[picked.segs[k].0].row) {
                    k += 1;
                }
                let text = Shown::of(self, t.at).map_or_else(String::new, |sh| sh.text(self, t));
                (Item::block(rows.start, rows.end - 1), text)
            } else if let Some(block) = diagram_of(s).filter(|_| !picked.within) {
                // a drawn diagram, as its source: the labels and the notes under it alike
                let (first, mut last) = (s.row, s.row);
                while k < picked.segs.len() && diagram_of(&segs[picked.segs[k].0]) == Some(block) {
                    last = segs[picked.segs[k].0].row;
                    k += 1;
                }
                let source = codes.get(block as usize).map_or("", |c| &c.text[..]);
                let source = source.strip_suffix('\n').unwrap_or(source);
                let text = format!("```mermaid\n{source}\n```");
                (Item::block(first, last), text)
            } else {
                // a stretch of one unit, over the rows it wraps across
                let (unit, first) = (s.unit, s.row);
                let (mut range, mut last) = (r.clone(), s.row);
                k += 1;
                while let Some((j, r)) = picked.segs.get(k)
                    && segs[*j].unit == unit
                {
                    range.start = range.start.min(r.start);
                    range.end = range.end.max(r.end);
                    last = segs[*j].row;
                    k += 1;
                }
                let text = self.unit_text(&roles, &codes, unit, range, !picked.within);
                let item = Item {
                    unit: Some(unit),
                    first,
                    last,
                };
                (item, text)
            };
            if let Some(p) = &prev {
                out.push_str(&self.separator(&roles, p, &item));
            }
            out.push_str(&text);
            prev = Some(item);
        }
        out
    }

    /// The text of bytes `range` of `unit`: code from its source line, and with `lists`, a list
    /// item's with its marker when it starts the item, and indented under it when it goes on.
    fn unit_text(
        &self,
        roles: &Roles,
        codes: &[&CodeBlock],
        unit: UnitId,
        range: Range<u32>,
        lists: bool,
    ) -> String {
        let text = self.map.unit_text(unit);
        let piece = text
            .get(range.start as usize..range.end as usize)
            .unwrap_or("");
        let source = roles.code(unit).and_then(|(block, line)| {
            let code = codes.get(block as usize)?;
            let line = code.lines().nth(line as usize)?;
            Some(&line[source_range(line, range.clone())])
        });
        let piece = source.unwrap_or(piece);
        let Some(list) = roles.list(unit).filter(|_| lists) else {
            return piece.to_owned();
        };
        let indent = " ".repeat(usize::from(list.indent));
        let mut out = String::with_capacity(piece.len() + indent.len());
        if range.start == 0 {
            match &list.marker {
                Some(marker) => {
                    let lead = usize::from(list.indent).saturating_sub(marker.len());
                    out.push_str(&indent[..lead]);
                    out.push_str(marker);
                }
                None => out.push_str(&indent),
            }
        }
        for (n, line) in piece.split('\n').enumerate() {
            if n > 0 {
                out.push('\n');
                out.push_str(&indent);
            }
            out.push_str(line);
        }
        out
    }

    /// What goes between two things copied: a line break, and a blank line where the screen
    /// shows one; lines of one code block as far apart as in its source.
    fn separator(&self, roles: &Roles, prev: &Item, next: &Item) -> String {
        let code = |u: Option<UnitId>| u.and_then(|u| roles.code(u));
        if let (Some((a, la)), Some((b, lb))) = (code(prev.unit), code(next.unit))
            && a == b
            && lb > la
        {
            return "\n".repeat((lb - la) as usize);
        }
        let between = (prev.last + 1)..next.first;
        let rows = self
            .layout
            .rows
            .get(between.start as usize..between.end as usize);
        let blank = rows.is_some_and(|rows| rows.iter().any(|r| r.kind == RowKind::Blank));
        if blank { "\n\n" } else { "\n" }.to_owned()
    }
}

/// Something copied, and the layout rows it came from.
struct Item {
    unit: Option<UnitId>,
    first: u32,
    last: u32,
}

impl Item {
    fn block(first: u32, last: u32) -> Self {
        Self {
            unit: None,
            first,
            last,
        }
    }
}

/// The bytes of `line` that show as bytes `range` of it with its tabs expanded
/// ([`expand_tabs`](crate::layout::expand_tabs)): a tab whole, if any of its spaces are in.
fn source_range(line: &str, range: Range<u32>) -> Range<usize> {
    let (start, end) = (range.start as usize, range.end as usize);
    let (mut from, mut to) = (None, line.len());
    let (mut exp, mut col) = (0, 0);
    for (i, c) in line.char_indices() {
        if exp >= end {
            to = i;
            break;
        }
        let (len, width) = match c {
            '\t' => {
                let n = TAB_STOP - col % TAB_STOP;
                (n, n)
            }
            c if c.is_control() => (1, 1),
            c => (c.len_utf8(), c.width().unwrap_or(0)),
        };
        if from.is_none() && exp + len > start {
            from = Some(i);
        }
        exp += len;
        col += width;
    }
    let from = from.unwrap_or(to).min(to);
    from..to
}

/// Where a unit sits in the document, for what its copy needs: the code line it is, and the
/// list item it's in.
#[derive(Clone, Debug, Default)]
struct Role {
    code: Option<(CodeId, u32)>,
    list: Option<ListAt>,
}

#[derive(Clone, Debug)]
struct ListAt {
    /// The columns of the markers it's under.
    indent: u16,
    /// The item's marker, on the first unit in the item: `- `, `3. `, `- [x] `.
    marker: Option<String>,
}

/// Every visible unit's [`Role`].
struct Roles(Vec<Role>);

impl Roles {
    fn of(doc: &Doc) -> Self {
        let mut roles = Roles(vec![Role::default(); doc.unit_count as usize]);
        let mut marker = None;
        roles.walk(&doc.blocks, 0, &mut marker);
        for note in &doc.footnotes {
            roles.walk(&note.blocks, 0, &mut None);
        }
        roles
    }

    fn set(&mut self, unit: UnitId, indent: u16, marker: &mut Option<String>) {
        if let Some(role) = self.0.get_mut(unit as usize)
            && (indent > 0 || marker.is_some())
        {
            role.list = Some(ListAt {
                indent,
                marker: marker.take(),
            });
        }
    }

    fn walk(&mut self, blocks: &[Block], indent: u16, marker: &mut Option<String>) {
        for block in blocks {
            match &block.kind {
                BlockKind::Heading { inlines, .. } | BlockKind::Paragraph(inlines) => {
                    self.set(inlines.unit, indent, marker);
                }
                BlockKind::Table(t) => {
                    marker.take();
                    for row in std::iter::once(&t.header).chain(&t.rows) {
                        for cell in &row.cells {
                            self.set(cell.unit, 0, &mut None);
                        }
                    }
                }
                BlockKind::Code(code) => {
                    for n in 0..code.lines().count() as u32 {
                        let unit = code.first_unit + n;
                        self.set(unit, indent, marker);
                        if let Some(role) = self.0.get_mut(unit as usize) {
                            role.code = Some((code.id, n));
                        }
                    }
                }
                BlockKind::Image { unit, .. } => self.set(*unit, indent, marker),
                BlockKind::List(list) => {
                    for (i, item) in list.items.iter().enumerate() {
                        let mark = match (item.task, list.start) {
                            (Some(true), _) => "- [x] ".to_owned(),
                            (Some(false), _) => "- [ ] ".to_owned(),
                            (None, Some(n)) => format!("{}. ", n.saturating_add(i as u64)),
                            (None, None) => "- ".to_owned(),
                        };
                        // a task's text lines up after its bullet, as markdown nests it
                        let width = if item.task.is_some() { 2 } else { mark.len() };
                        let inner = indent.saturating_add(width as u16);
                        let mut mark = Some(mark);
                        self.walk(&item.blocks, inner, &mut mark);
                    }
                }
                BlockKind::Quote { blocks, .. } => self.walk(blocks, indent, marker),
                BlockKind::Rule | BlockKind::Html(_) | BlockKind::FootnoteDefs => {}
            }
        }
    }

    fn code(&self, unit: UnitId) -> Option<(CodeId, u32)> {
        self.0.get(unit as usize)?.code
    }

    fn list(&self, unit: UnitId) -> Option<&ListAt> {
        self.0.get(unit as usize)?.list.as_ref()
    }
}

/// A table as one layout shows it.
struct Shown<'a> {
    /// Index into [`Layout::tables`].
    at: usize,
    table: &'a Table,
    /// Its rows in the order shown: `None` for the header, else the index of a body row.
    order: Vec<Option<u32>>,
    /// Each row's first source line, with its position in `order`, by line.
    lines: Vec<(u32, usize)>,
    /// Each cell's unit, with its row and column, by unit.
    cells: Vec<(UnitId, Option<u32>, u16)>,
    columns: u16,
    records: bool,
}

impl<'a> Shown<'a> {
    fn of(view: &View<'a>, at: usize) -> Option<Self> {
        let layout = view.layout;
        let rows = layout.tables.get(at)?;
        let tables = view.doc.tables();
        let table: &'a Table = tables.get(rows.table as usize).copied()?;
        let mut cells = Vec::new();
        let mut by_line = Vec::new();
        let all = std::iter::once((None, &table.header));
        for (row, r) in all.chain(
            table
                .rows
                .iter()
                .enumerate()
                .map(|(i, r)| (Some(i as u32), r)),
        ) {
            by_line.push((r.src.start, row));
            for (col, cell) in r.cells.iter().enumerate() {
                cells.push((cell.unit, row, col as u16));
            }
        }
        cells.sort_unstable_by_key(|c| c.0);
        by_line.sort_unstable_by_key(|l| l.0);
        let mut order: Vec<Option<u32>> = Vec::new();
        let mut lines: Vec<(u32, usize)> = Vec::new();
        for r in &layout.rows[rows.rows.start as usize..rows.rows.end as usize] {
            let line = r.src.start;
            if r.src.is_none() || lines.last().is_some_and(|l| l.0 == line) {
                continue;
            }
            let Ok(i) = by_line.binary_search_by_key(&line, |l| l.0) else {
                continue;
            };
            lines.push((line, order.len()));
            order.push(by_line[i].1);
        }
        lines.sort_unstable_by_key(|l| l.0);
        lines.dedup_by_key(|l| l.0);
        let records = !table.rows.is_empty()
            && !(layout.grids.iter()).any(|g| rows.rows.contains(&g.header.start));
        Some(Self {
            at,
            table,
            order,
            lines,
            cells,
            columns: table.column_count().min(usize::from(u16::MAX)) as u16,
            records,
        })
    }

    /// The position in the order shown of the row a segment is in, and its column.
    fn cell(&self, view: &View<'_>, s: &SearchSeg) -> (usize, u16) {
        let line = view.layout.rows[s.row as usize].src.start;
        let row = (self.lines.binary_search_by_key(&line, |l| l.0)).map_or(0, |i| self.lines[i].1);
        let col =
            (self.cells.binary_search_by_key(&s.unit, |c| c.0)).map_or(0, |i| self.cells[i].2);
        (row, col)
    }

    /// The segments of the cells `pick` covers, whole: a card's keys with its values.
    fn segs(&self, view: &View<'_>, pick: &TablePick) -> Vec<(usize, Range<u32>)> {
        let segs = view.segs();
        let rows = &view.layout.tables[self.at].rows;
        let first = segs.partition_point(|s| s.row < rows.start);
        let end = first + segs[first..].partition_point(|s| s.row < rows.end);
        (first..end)
            .filter(|&i| {
                let (row, col) = self.cell(view, &segs[i]);
                pick.rows.contains(&row) && pick.cols.contains(&col)
            })
            .map(|i| (i, segs[i].start..segs[i].end))
            .collect()
    }

    fn format(&self, pick: &TablePick) -> TableFormat {
        let header = self
            .order
            .get(pick.rows.clone())
            .is_some_and(|r| r.contains(&None));
        if self.records {
            TableFormat::Records
        } else if header && pick.cols == (0..self.columns) && pick.rows.len() > 1 {
            TableFormat::Markdown
        } else {
            TableFormat::Tsv
        }
    }

    /// A cell's text, `""` for one the row doesn't have.
    fn cell_text(&self, view: &View<'a>, row: Option<u32>, col: u16) -> &'a str {
        let r = match row {
            None => Some(&self.table.header),
            Some(i) => self.table.rows.get(i as usize),
        };
        r.and_then(|r| r.cells.get(usize::from(col)))
            .map_or("", |c| view.map.unit_text(c.unit))
    }

    fn text(&self, view: &View<'a>, pick: &TablePick) -> String {
        let rows = self.order.get(pick.rows.clone()).unwrap_or_default();
        let cols = pick.cols.clone();
        let flat = |s: &str| s.replace(['\t', '\n'], " ");
        let mut out = String::new();
        match self.format(pick) {
            TableFormat::Records => {
                for (n, &row) in rows.iter().enumerate() {
                    if n > 0 {
                        out.push_str("\n\n");
                    }
                    for (k, col) in cols.clone().enumerate() {
                        if k > 0 {
                            out.push('\n');
                        }
                        let key = flat(self.cell_text(view, None, col));
                        let value = flat(self.cell_text(view, row, col));
                        let _ = write!(out, "{key}: {value}");
                    }
                }
            }
            TableFormat::Markdown => {
                let line = |out: &mut String, cells: &mut dyn Iterator<Item = String>| {
                    out.push('|');
                    for cell in cells {
                        let _ = write!(out, " {cell} |");
                    }
                };
                let gfm = |s: &str| s.replace('|', "\\|").replace('\n', "<br>");
                let header = cols.clone().map(|c| gfm(self.cell_text(view, None, c)));
                line(&mut out, &mut header.into_iter());
                out.push('\n');
                let rule = cols.clone().map(|c| {
                    match self.table.aligns.get(usize::from(c)) {
                        Some(Align::Left) => ":---",
                        Some(Align::Center) => ":---:",
                        Some(Align::Right) => "---:",
                        _ => "---",
                    }
                    .to_owned()
                });
                line(&mut out, &mut rule.into_iter());
                for &row in rows.iter().filter(|r| r.is_some()) {
                    out.push('\n');
                    let cells = cols.clone().map(|c| gfm(self.cell_text(view, row, c)));
                    line(&mut out, &mut cells.into_iter());
                }
            }
            TableFormat::Tsv => {
                for (n, &row) in rows.iter().enumerate() {
                    if n > 0 {
                        out.push('\n');
                    }
                    for (k, col) in cols.clone().enumerate() {
                        if k > 0 {
                            out.push('\t');
                        }
                        out.push_str(&flat(self.cell_text(view, row, col)));
                    }
                }
            }
        }
        out
    }
}

/// `text` cut to at most [`MAX_COPY`] bytes, at a character boundary; and whether it was cut.
pub fn cut_copy(mut text: String) -> (String, bool) {
    if text.len() <= MAX_COPY {
        return (text, false);
    }
    let mut end = MAX_COPY;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    (text, true)
}

/// When a selection is copied (`copy_on` in the config).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CopyOn {
    /// On `y`, which also clears it; until then it stays on screen.
    #[default]
    Yank,
    /// As the drag, or the double or triple click, ends, as X's primary selection does.
    Select,
}

impl CopyOn {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "yank" => Some(CopyOn::Yank),
            "select" => Some(CopyOn::Select),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            CopyOn::Yank => "yank",
            CopyOn::Select => "select",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::parse;
    use crate::layout::{LayoutParams, LineNumbers, Measure, Sort, TableMode, layout_measured};
    use crate::theme::Theme;

    fn params(width: u16) -> LayoutParams {
        LayoutParams {
            width,
            max_width: None,
            table_mode: TableMode::Auto,
            code_wrap: true,
            line_numbers: LineNumbers::Off,
            glyphs_gen: 0,
            images: None,
        }
    }

    struct Fixture {
        doc: Doc,
        map: SearchMap,
        layout: Layout,
    }

    impl Fixture {
        fn new(md: &str, p: LayoutParams) -> Self {
            Self::sorted(md, p, None)
        }

        /// With the first table sorted by `sort`.
        fn sorted(md: &str, p: LayoutParams, sort: Option<Sort>) -> Self {
            let theme = Theme::builtin();
            let doc = parse(md);
            let mut measure = Measure::new(&doc, &theme.glyphs);
            if let Some(t) = doc.tables().first() {
                measure.set_sort(t, sort);
            }
            let map = SearchMap::build(&doc, &theme.glyphs);
            let layout = layout_measured(&doc, &measure, &theme.glyphs, p);
            Self { doc, map, layout }
        }

        fn view(&self) -> View<'_> {
            View {
                doc: &self.doc,
                map: &self.map,
                layout: &self.layout,
                code_x: 0,
            }
        }

        /// The cell showing the first character of the first `needle` on screen, or its last.
        fn cell(&self, needle: &str, last: bool) -> (u32, u16) {
            for r in 0..self.layout.row_count() {
                let text = self.layout.row_text(r as usize);
                if let Some(i) = text.find(needle) {
                    let at = if last {
                        i + needle.char_indices().last().map_or(0, |(j, _)| j)
                    } else {
                        i
                    };
                    let col: usize = text[..at].chars().map(|c| c.width().unwrap_or(0)).sum();
                    return (r, col as u16);
                }
            }
            panic!("{needle:?} isn't on screen:\n{}", self.screen());
        }

        fn screen(&self) -> String {
            (0..self.layout.row_count() as usize)
                .map(|r| self.layout.row_text(r))
                .collect::<Vec<_>>()
                .join("\n")
        }

        /// A drag from the first character of `from` to the last of `to`.
        fn select(&self, from: &str, to: &str) -> Selection {
            let v = self.view();
            let (a, b) = (self.cell(from, false), self.cell(to, true));
            let anchor = v.place(a.0, a.1, false).expect("an anchor");
            let head = v.head(anchor, b.0, b.1).expect("a head");
            Selection { anchor, head }
        }

        fn drag(&self, from: &str, to: &str) -> String {
            self.view().copy(&self.select(from, to)).text
        }

        fn copied(&self, from: &str, to: &str) -> Copied {
            self.view().copy(&self.select(from, to))
        }
    }

    fn drag(md: &str, width: u16, from: &str, to: &str) -> String {
        Fixture::new(md, params(width)).drag(from, to)
    }

    #[test]
    fn a_wrapped_paragraph_copies_unwrapped() {
        let md = "alpha beta gamma delta epsilon zeta\n";
        assert_eq!(drag(md, 16, "beta", "delta"), "beta gamma delta");
        assert_eq!(
            drag(md, 16, "alpha", "zeta"),
            "alpha beta gamma delta epsilon zeta"
        );
        // one character: the cell under the press and the release
        assert_eq!(drag(md, 16, "g", "g"), "g");
    }

    #[test]
    fn a_drag_back_takes_the_character_it_started_on() {
        let f = Fixture::new("alpha beta gamma delta\n", params(14));
        let v = f.view();
        let (a, b) = (f.cell("delta", true), f.cell("beta", false));
        let anchor = v.place(a.0, a.1, false).unwrap();
        let head = v.head(anchor, b.0, b.1).unwrap();
        assert_eq!(v.copy(&Selection { anchor, head }).text, "beta gamma delta");
    }

    #[test]
    fn hard_breaks_stay_and_blocks_keep_their_blank_line() {
        let md = "one  \ntwo **bold** [link](x.md)\n\n# Head\n\nlast\n";
        assert_eq!(drag(md, 40, "one", "link"), "one\ntwo bold link");
        assert_eq!(drag(md, 40, "two", "last"), "two bold link\n\nHead\n\nlast");
    }

    #[test]
    fn list_items_come_with_their_markers() {
        let md = "- first\n- second\n  - nested\n- [x] done\n\n3. three\n4. four\n";
        assert_eq!(
            drag(md, 40, "first", "done"),
            "- first\n- second\n  - nested\n- [x] done"
        );
        assert_eq!(drag(md, 40, "irst", "nested"), "irst\n- second\n  - nested");
        assert_eq!(drag(md, 40, "three", "four"), "3. three\n4. four");
        // within one item: its text alone
        assert_eq!(drag(md, 40, "second", "second"), "second");
        // a wrapped item's later rows are one line
        let md = "- an item that wraps over rows\n- next\n";
        assert_eq!(
            drag(md, 14, "an", "next"),
            "- an item that wraps over rows\n- next"
        );
    }

    #[test]
    fn code_copies_from_its_source_lines() {
        let md = "```\nfn a() {\n\tb\t= 1;\n\n}\n```\n";
        assert_eq!(drag(md, 40, "fn", "}"), "fn a() {\n\tb\t= 1;\n\n}");
        // part of a line: the tab whole when the selection starts in its spaces
        assert_eq!(drag(md, 40, "b", "1"), "b\t= 1");
        // wrapped code: no wrap marks
        let long = format!("```\n{}\n```\n", "x".repeat(30));
        let f = Fixture::new(&long, params(20));
        let (first, last) = (f.cell("x", false), f.cell("x", true));
        let v = f.view();
        let anchor = v.place(first.0, first.1, false).unwrap();
        let end = f.layout.row_count() - 2;
        let head = v.head(anchor, end, last.1 + 40).unwrap();
        assert_eq!(v.copy(&Selection { anchor, head }).text, "x".repeat(30));
    }

    #[test]
    fn no_wrap_code_follows_the_offset() {
        let md = "```\nabcdefghijklmnopqrstuvwxyz0123456789\n```\n";
        let p = LayoutParams {
            code_wrap: false,
            ..params(24)
        };
        let f = Fixture::new(md, p);
        let (row, col) = f.cell("a", false);
        let v = View {
            code_x: 8,
            ..f.view()
        };
        // the cell that showed `a` shows `i` once the code is shifted 8 columns
        let anchor = v.place(row, col, false).unwrap();
        let head = v.head(anchor, row, col + 2).unwrap();
        assert_eq!(v.copy(&Selection { anchor, head }).text, "ijk");
    }

    const TABLE: &str =
        "| key | value |\n|:----|------:|\n| alpha | 1 |\n| beta \\| b | 22 |\n|  | 333 |\n";

    #[test]
    fn inside_one_cell_a_drag_selects_its_text() {
        let md = "| a | b |\n|---|---|\n| hello there world | x |\n";
        let f = Fixture::new(md, params(14));
        // the cell wraps; the selection is the cell's text, unwrapped
        assert_eq!(f.drag("hello", "world"), "hello there world");
        assert_eq!(f.copied("hello", "world").table, None);
        assert_eq!(f.drag("ello", "worl"), "ello there worl");
        // a card's key is a cell of its own, shown on every card
        let p = LayoutParams {
            table_mode: TableMode::Records,
            ..params(40)
        };
        let f = Fixture::new(TABLE, p);
        let c = f.copied("valu", "value");
        assert_eq!((c.text.as_str(), c.table), ("value", None));
    }

    #[test]
    fn cells_across_a_table_copy_as_tsv() {
        let f = Fixture::new(TABLE, params(40));
        let c = f.copied("alpha", "22");
        assert_eq!(c.text, "alpha\t1\nbeta | b\t22");
        assert_eq!(c.table, Some((2, 2, TableFormat::Tsv)));
        // one column, down
        assert_eq!(f.drag("1", "333"), "1\n22\n333");
        // the header in, but not every column
        assert_eq!(f.drag("value", "22"), "value\n1\n22");
    }

    #[test]
    fn whole_rows_with_the_header_copy_as_markdown() {
        let f = Fixture::new(TABLE, params(40));
        let c = f.copied("key", "22");
        assert_eq!(
            c.text,
            "| key | value |\n| :--- | ---: |\n| alpha | 1 |\n| beta \\| b | 22 |"
        );
        assert_eq!(c.table, Some((3, 2, TableFormat::Markdown)));
        // the empty cell stays
        assert!(f.drag("key", "333").ends_with("\n|  | 333 |"));
    }

    #[test]
    fn a_sorted_table_copies_in_the_order_shown() {
        let sort = Sort { col: 1, desc: true };
        let f = Fixture::sorted(TABLE, params(40), Some(sort));
        assert_eq!(f.drag("333", "1"), "333\n22\n1");
        assert_eq!(f.drag("333", "alpha"), "\t333\nbeta | b\t22\nalpha\t1");
    }

    #[test]
    fn record_cards_copy_as_header_value_lines() {
        let p = LayoutParams {
            table_mode: TableMode::Records,
            ..params(40)
        };
        let f = Fixture::new(TABLE, p);
        let c = f.copied("alpha", "22");
        assert_eq!(c.text, "key: alpha\nvalue: 1\n\nkey: beta | b\nvalue: 22");
        assert_eq!(c.table, Some((2, 2, TableFormat::Records)));
    }

    #[test]
    fn a_drag_through_a_table_takes_its_rows_whole() {
        let md = format!("before\n\n{TABLE}\nafter\n");
        let f = Fixture::new(&md, params(40));
        assert_eq!(
            f.drag("before", "after"),
            format!(
                "before\n\n{}\n\nafter",
                "| key | value |\n| :--- | ---: |\n| alpha | 1 |\n| beta \\| b | 22 |\n|  | 333 |"
            )
        );
        // from mid-table on: whole rows from there, without the header, as TSV
        assert_eq!(f.drag("beta", "after"), "beta | b\t22\n\t333\n\nafter");
    }

    #[test]
    fn a_drawn_diagram_copies_as_its_source() {
        let md =
            "before\n\n```mermaid\nflowchart LR\n    A[start here] --> B[done]\n```\n\nafter\n";
        let f = Fixture::new(md, params(60));
        assert_eq!(
            f.drag("before", "after"),
            "before\n\n```mermaid\nflowchart LR\n    A[start here] --> B[done]\n```\n\nafter"
        );
        // inside one label, its text
        assert_eq!(f.drag("start", "here"), "start here");
    }

    #[test]
    fn double_and_triple_clicks() {
        let f = Fixture::new("alpha beta_2 gamma, delta\n\nnext\n", params(14));
        let v = f.view();
        let text = |sel: Option<Selection>| v.copy(&sel.unwrap()).text;
        let (r, c) = f.cell("beta", false);
        assert_eq!(text(v.word(r, c + 2)), "beta_2");
        let (r, c) = f.cell(",", false);
        assert_eq!(text(v.word(r, c)), ",");
        assert_eq!(text(v.unit(r, c)), "alpha beta_2 gamma, delta");
        let (r, c) = f.cell("next", false);
        assert_eq!(text(v.unit(r, c)), "next");
    }

    #[test]
    fn a_selection_survives_a_relayout() {
        let md = "alpha beta gamma delta epsilon zeta\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        let wide = Fixture::new(md, params(80));
        let sel = wide.select("beta", "epsilon");
        let narrow = Fixture::new(md, params(14));
        assert_eq!(narrow.view().copy(&sel).text, "beta gamma delta epsilon");
        // and table cells across table modes
        let sel = wide.select("1", "2");
        let p = LayoutParams {
            table_mode: TableMode::Records,
            ..params(80)
        };
        let records = Fixture::new(md, p);
        assert_eq!(records.view().copy(&sel).text, "a: 1\nb: 2");
    }

    #[test]
    fn the_far_end_snaps_to_text() {
        let f = Fixture::new("first para\n\nsecond para\n", params(40));
        let v = f.view();
        let (r, c) = f.cell("first", false);
        let anchor = v.place(r, c, false).unwrap();
        // the blank row between, then far past the end of a row
        let head = v.head(anchor, r + 1, 0).unwrap();
        assert_eq!(v.copy(&Selection { anchor, head }).text, "first para");
        let head = v.head(anchor, r + 2, 200).unwrap();
        assert_eq!(
            v.copy(&Selection { anchor, head }).text,
            "first para\n\nsecond para"
        );
    }

    #[test]
    fn cells_cover_what_the_selection_shows() {
        let f = Fixture::new("alpha `code` beta\n", params(40));
        let v = f.view();
        let sel = f.select("alpha", "beta");
        let picked = v.pick(&sel);
        let cells = v.cells(&picked, 0..f.layout.row_count());
        let row = f.layout.row_text(cells[0].0 as usize);
        let (start, end) = (f.cell("alpha", false).1, f.cell("beta", true).1 + 1);
        assert_eq!(cells, [(f.cell("alpha", false).0, start..end)], "{row}");
    }

    #[test]
    fn source_ranges_take_whole_tabs() {
        assert_eq!(source_range("a\tb", 0..5), 0..3);
        assert_eq!(source_range("a\tb", 2..5), 1..3, "in the tab's spaces");
        assert_eq!(source_range("a\tb", 4..5), 2..3);
        assert_eq!(source_range("ab", 1..1), 1..1);
    }

    #[test]
    fn copies_are_cut_at_a_character() {
        let text = "é".repeat(MAX_COPY);
        let (cut, was) = cut_copy(text);
        assert!(was && cut.len() <= MAX_COPY && cut.len() >= MAX_COPY - 1);
        assert_eq!(cut_copy("short".into()), ("short".into(), false));
    }

    /// Everything here leans on [`Layout::search`] being in screen order.
    #[test]
    fn search_segments_are_in_screen_order() {
        let theme = Theme::builtin();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "md") {
                continue;
            }
            let doc = parse(&std::fs::read_to_string(&path).unwrap());
            for width in [1, 24, 80] {
                for code_wrap in [true, false] {
                    let p = LayoutParams {
                        code_wrap,
                        ..params(width)
                    };
                    let layout = crate::layout::layout(&doc, &theme.glyphs, p);
                    let keys: Vec<_> = layout.search.iter().map(|s| (s.row, s.col)).collect();
                    assert!(
                        keys.is_sorted(),
                        "{} at {width}, wrap {code_wrap}",
                        path.display()
                    );
                }
            }
        }
    }
}
