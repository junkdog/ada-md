//! Mermaid diagrams (ADR-35): the drawn canvas, a row per line, in the diagram style keys at
//! their real widths. Paint shows each row through a window as wide as the text, shifted by the
//! shared horizontal offset like no-wrap code, so a diagram wider than the text scrolls sideways
//! without a relayout. A sequence diagram's participants are a sticky header, like a table's.
//! Labels cut to fit are listed in full under the diagram.
//!
//! Search runs over the block's source lines, as for code; the labels the drawing shows from
//! them register where they went, so a match in a label lights up in the drawing.

use std::panic::AssertUnwindSafe;

use super::{Builder, CodeInfo, GridRows, RowKind, SearchSeg, expand_tabs, push_span, wrap};
use crate::diagram::{self, Canvas, Diagram};
use crate::doc::{CodeBlock, InlineStyle, LineRange, UnitId, code_lines};
use crate::guard;
use crate::theme::StyleKey;

/// Lay out `diagram`, the drawing of `code`. `false` when it can't be drawn, and the block is
/// laid out as its source instead.
pub(super) fn layout(
    b: &mut Builder<'_>,
    code: &CodeBlock,
    diagram: &Diagram,
    src: LineRange,
) -> bool {
    let avail = b.avail();
    let g = b.g;
    let drawn = guard::catch(AssertUnwindSafe(|| diagram::draw(diagram, avail, g))).flatten();
    let Some(drawn) = drawn else {
        return false;
    };
    let canvas = &drawn.canvas;
    let first = b.out.row_count();
    let mut x = 0;
    for row in 0..canvas.height() {
        b.begin_row();
        x = b.col;
        let (out, col, row_start) = (&mut b.out, &mut b.col, b.row_start);
        canvas.pieces(row, StyleKey::DiagramLine, &mut |text, width, key| {
            let width = width.min(usize::from(u16::MAX)) as u16;
            push_span(
                out,
                row_start,
                col,
                text,
                width,
                (key, InlineStyle::NONE, None),
            );
        });
        b.end_row(src, RowKind::Diagram { block: code.id, x });
    }
    let end = b.out.row_count();
    if let Some(header) = drawn.header {
        b.out.grids.push(GridRows {
            header: first + header.start as u32..first + header.end as u32,
            end,
        });
    }
    if let Some(info) = b.out.code.get_mut(code.id as usize) {
        *info = CodeInfo {
            key: None,
            width: canvas.width().min(usize::from(u16::MAX)) as u16,
            window: avail,
        };
    }
    let lines = Lines::of(code);
    if b.screen {
        found_labels(b, &lines, canvas, first, x);
    }
    let marks = drawn.notes.len().to_string().len() + 1;
    for (i, note) in drawn.notes.iter().enumerate() {
        let mark = diagram::note_mark(i + 1);
        b.push_prefix(&format!("{mark:<marks$}"), StyleKey::DiagramNumber);
        let unit = note
            .origin
            .filter(|o| o.len as usize == note.len())
            .and_then(|o| lines.unit(o.at, o.len));
        let w = match unit {
            Some((unit, at, _)) => {
                let mut w = wrap::wrap_unit(
                    ["", note, ""],
                    unit,
                    StyleKey::DiagramNote,
                    src.start,
                    b.avail(),
                );
                for piece in &mut w.pieces {
                    piece.unit_off += at;
                }
                w
            }
            None => wrap::wrap_plain(note, StyleKey::DiagramNote, src.start, b.avail()),
        };
        b.emit_lines(&w);
        b.pop_prefix();
    }
    true
}

/// A block's source lines, to find the search unit a label came from.
struct Lines<'a> {
    /// Each line, and the byte of the block's text it starts at.
    lines: Vec<(u32, &'a str)>,
    first_unit: UnitId,
}

impl<'a> Lines<'a> {
    fn of(code: &'a CodeBlock) -> Self {
        let text = &*code.text;
        let lines = code_lines(text)
            .map(|l| ((l.as_ptr() as usize - text.as_ptr() as usize) as u32, l))
            .collect();
        Self {
            lines,
            first_unit: code.first_unit,
        }
    }

    /// Bytes `at..at + len` of the block's text: their line's unit, where they are in its
    /// search text, and the text itself. `None` when they run past their line.
    fn unit(&self, at: u32, len: u32) -> Option<(UnitId, u32, &'a str)> {
        let i = self
            .lines
            .partition_point(|&(s, _)| s <= at)
            .checked_sub(1)?;
        let (start, line) = self.lines[i];
        let from = (at - start) as usize;
        let text = line.get(from..from + len as usize)?;
        // the unit's text has its tabs expanded; a label has none of its own
        let before = &line[..from];
        let off = if before.contains(|c: char| c.is_control()) {
            expand_tabs(before).len()
        } else {
            from
        };
        Some((self.first_unit + i as UnitId, off as u32, text))
    }
}

/// Register a search segment for each label `canvas` still shows: rows from `first`, columns
/// from `x`.
fn found_labels(b: &mut Builder<'_>, lines: &Lines<'_>, canvas: &Canvas, first: u32, x: u16) {
    for f in &canvas.found {
        let Some((unit, at, text)) = lines.unit(f.at, f.len) else {
            continue;
        };
        let col = u16::try_from(f.col).ok().and_then(|c| x.checked_add(c));
        if let Some(col) = col
            && canvas.holds(f.row as usize, f.col as usize, text)
        {
            b.out.search.push(SearchSeg {
                unit,
                start: at,
                end: at + f.len,
                row: first + f.row,
                col,
            });
        }
    }
}
