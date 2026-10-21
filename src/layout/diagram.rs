//! Mermaid diagrams (ADR-35): the drawn canvas, a row per line, in the diagram style keys at
//! their real widths. Paint shows each row through a window as wide as the text, shifted by the
//! shared horizontal offset like no-wrap code, so a diagram wider than the text scrolls sideways
//! without a relayout. A sequence diagram's participants are a sticky header, like a table's.
//! Labels cut to fit are listed in full under the diagram.

use std::panic::AssertUnwindSafe;

use super::{Builder, CodeInfo, GridRows, RowKind, push_span, wrap};
use crate::diagram::{self, Diagram};
use crate::doc::{CodeBlock, InlineStyle, LineRange};
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
    for row in 0..canvas.height() {
        b.begin_row();
        let x = b.col;
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
    let marks = drawn.notes.len().to_string().len() + 1;
    for (i, note) in drawn.notes.iter().enumerate() {
        let mark = diagram::note_mark(i + 1);
        b.push_prefix(&format!("{mark:<marks$}"), StyleKey::DiagramNumber);
        let w = wrap::wrap_plain(note, StyleKey::DiagramNote, src.start, b.avail());
        b.emit_lines(&w);
        b.pop_prefix();
    }
    true
}
