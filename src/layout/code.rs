//! Code blocks (ADR-20): a background-filled block with the language on its top row. With
//! `code_wrap` on, long lines wrap with the theme's `↪`; with it off, each source line is one row,
//! sliced at paint time by the shared horizontal offset (P3). Highlighting fills in at paint (P3).

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{Builder, RowKind};
use crate::doc::{CodeBlock, LineRange};
use crate::theme::StyleKey;

const TAB_STOP: usize = 4;

pub(super) fn layout(b: &mut Builder<'_>, code: &CodeBlock, src: LineRange) {
    let block = b.code_blocks;
    b.code_blocks += 1;
    let g = b.g;
    let avail = b.avail();
    let lead = g.wrap_mark.width().max(1) as u16;
    let inner = avail.saturating_sub(lead + 1).max(1);
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
    for (n, line) in code.lines().enumerate() {
        count += 1;
        let line = expand_tabs(line);
        let line_src = LineRange::line(code.first_line + n as u32);
        let kind = RowKind::Code {
            block,
            line: n as u32,
        };
        if b.params.code_wrap {
            for (i, chunk) in chunks(&line, inner).into_iter().enumerate() {
                b.begin_row();
                if i == 0 {
                    b.text(&pad, StyleKey::CodeBlock);
                } else {
                    b.text(&g.wrap_mark, StyleKey::CodeWrapMark);
                    b.fill(lead, StyleKey::CodeBlock);
                }
                b.text(chunk, StyleKey::CodeBlock);
                b.fill(avail, StyleKey::CodeBlock);
                b.end_row(line_src, kind);
            }
        } else {
            b.begin_row();
            b.text(&pad, StyleKey::CodeBlock);
            b.text(&line, StyleKey::CodeBlock);
            b.fill(avail, StyleKey::CodeBlock);
            b.end_row(line_src, kind);
        }
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

fn expand_tabs(line: &str) -> String {
    if !line.contains(['\t', '\u{b}', '\u{c}']) {
        return line.to_owned();
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
    out
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
