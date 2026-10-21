//! Code blocks (ADR-20): a background-filled block with the language on its top row. With
//! `code_wrap` on, long lines wrap with the theme's `↪`. With it off, each source line is one
//! row whose code span is a fixed-width window, sliced at paint time by the shared horizontal
//! offset, so scrolling sideways never relayouts. Highlighting is overlaid at paint, too.

use std::borrow::Cow;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{Builder, CodeInfo, RowKind, push_span};
use crate::doc::{CodeBlock, InlineStyle, LineRange};
use crate::highlight;
use crate::theme::StyleKey;

const TAB_STOP: usize = 4;

pub(super) fn layout(b: &mut Builder<'_>, code: &CodeBlock, src: LineRange) {
    if let Some(d) = &code.diagram
        && super::diagram::layout(b, code, d, src)
    {
        return;
    }
    let block = code.id;
    let g = b.g;
    let avail = b.avail();
    let lead = [&g.wrap_mark, &g.code_clip[0]]
        .iter()
        .map(|s| s.width())
        .max()
        .unwrap_or(0)
        .max(1) as u16;
    let trail = g.code_clip[1].width().max(1) as u16;
    let inner = avail.saturating_sub(lead + trail).max(1);
    let pad = " ".repeat(lead as usize);

    b.begin_row();
    b.text(&pad, StyleKey::CodeBlock);
    if let Some(lang) = &code.lang {
        b.text(truncate(lang, inner), StyleKey::CodeLabel);
    }
    b.fill(avail, StyleKey::CodeBlock);
    let fence = if code.fenced {
        LineRange::line(src.start)
    } else {
        LineRange::NONE
    };
    b.end_row(fence, RowKind::Decor);

    let mut count = 0;
    let mut widest = 0;
    for (n, line) in code.lines().enumerate() {
        count += 1;
        let line = expand_tabs(line);
        let unit = code.first_unit + n as u32;
        let line_src = LineRange::line(code.first_line + n as u32);
        if b.params.code_wrap {
            let mut from = 0;
            for (i, chunk) in chunks(&line, inner).into_iter().enumerate() {
                b.begin_row();
                if i == 0 {
                    b.text(&pad, StyleKey::CodeBlock);
                } else {
                    b.text(&g.wrap_mark, StyleKey::CodeWrapMark);
                    b.fill(lead, StyleKey::CodeBlock);
                }
                if !chunk.is_empty() {
                    let col = b.col;
                    let range = from..from + chunk.len() as u32;
                    b.search_seg(unit, range, col..col + chunk.width() as u16);
                }
                b.text(chunk, StyleKey::CodeText);
                b.fill(avail, StyleKey::CodeBlock);
                let kind = RowKind::Code {
                    block,
                    line: n as u32,
                    from,
                };
                b.end_row(line_src, kind);
                from += chunk.len() as u32;
            }
        } else {
            b.begin_row();
            b.text(&pad, StyleKey::CodeBlock);
            if !line.is_empty() {
                let col = b.col;
                b.search_seg(unit, 0..line.len() as u32, col..col + inner);
            }
            // The whole line, in a span `inner` columns wide that paint slices.
            let look = (StyleKey::CodeText, InlineStyle::NONE, None);
            push_span(&mut b.out, b.row_start, &mut b.col, &line, inner, look);
            b.fill(avail, StyleKey::CodeBlock);
            let kind = RowKind::Code {
                block,
                line: n as u32,
                from: 0,
            };
            b.end_row(line_src, kind);
        }
        widest = widest.max(line.width());
    }
    if let Some(info) = b.out.code.get_mut(block as usize) {
        *info = CodeInfo {
            key: highlight::key(code),
            width: widest.min(usize::from(u16::MAX)) as u16,
            window: inner,
        };
    }

    b.begin_row();
    b.fill(avail, StyleKey::CodeBlock);
    let last_code = code.first_line + count;
    let close = if code.fenced && src.end >= last_code {
        LineRange::line(src.end)
    } else {
        LineRange::NONE
    };
    b.end_row(close, RowKind::Decor);
}

/// Tabs to spaces at stops of four, and other control characters to spaces. Layout and the
/// highlighter both work on the expanded line, so their byte offsets agree.
pub fn expand_tabs(line: &str) -> Cow<'_, str> {
    if !line.contains(|c: char| c.is_control()) {
        return Cow::Borrowed(line);
    }
    let mut out = String::with_capacity(line.len() + TAB_STOP);
    let mut col = 0;
    for c in line.chars() {
        if c == '\t' {
            let n = TAB_STOP - col % TAB_STOP;
            out.extend(std::iter::repeat_n(' ', n));
            col += n;
        } else if c.is_control() {
            out.push(' ');
            col += 1;
        } else {
            out.push(c);
            col += c.width().unwrap_or(0);
        }
    }
    Cow::Owned(out)
}

/// Split a line into pieces at most `width` columns wide. An empty line is one empty chunk.
fn chunks(line: &str, width: u16) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut used = 0u16;
    for (i, c) in line.char_indices() {
        let cw = c.width().unwrap_or(0) as u16;
        if used + cw > width && i > start {
            out.push(&line[start..i]);
            start = i;
            used = 0;
        }
        used += cw;
    }
    out.push(&line[start..]);
    out
}

fn truncate(s: &str, width: u16) -> &str {
    let mut used = 0;
    for (i, c) in s.char_indices() {
        used += c.width().unwrap_or(0);
        if used > width as usize {
            return &s[..i];
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_expand_to_stops() {
        assert_eq!(expand_tabs("a\tb\t\tc"), "a   b       c");
    }

    #[test]
    fn chunking() {
        assert_eq!(chunks("abcdefg", 3), ["abc", "def", "g"]);
        assert_eq!(chunks("", 3), [""]);
        assert_eq!(chunks("日本語", 4), ["日本", "語"]);
    }
}
