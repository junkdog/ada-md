//! Code blocks (ADR-20): a background-filled block with the language on its top row, but in a
//! print ([`LayoutParams::code_labels`](super::LayoutParams::code_labels)). With `code_wrap` on, long lines wrap with the theme's `↪`. With it off, each source line is one
//! row whose code span is a fixed-width window, sliced at paint time by the shared horizontal
//! offset, so scrolling sideways never relayouts. Highlighting is overlaid at paint, too.

use std::borrow::Cow;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::wrap::{cut, text_width};
use super::{Builder, CodeInfo, RowKind, push_span};
use crate::doc::{CodeBlock, InlineStyle, LineRange};
use crate::highlight;
use crate::theme::StyleKey;

pub const TAB_STOP: usize = 4;

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
    if let Some(lang) = code.lang.as_ref().filter(|_| b.params.code_labels) {
        b.text(cut(lang, usize::from(inner)), StyleKey::CodeLabel);
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
                if b.screen && !chunk.is_empty() {
                    let col = b.col;
                    let range = from..from + chunk.len() as u32;
                    b.search_seg(unit, range, col..col + text_width(chunk) as u16);
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
        widest = widest.max(text_width(&line));
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
    if !needs_expanding(line) {
        return Cow::Borrowed(line);
    }
    let mut out = String::with_capacity(line.len() + TAB_STOP);
    expand_into(line, &mut out);
    Cow::Owned(out)
}

/// [`expand_tabs`] into `buf`, reused from line to line: the line itself when nothing changes,
/// else `buf` holding the expanded line (HL-004: no allocation a line with a tab).
pub fn expand_tabs_in<'a>(line: &'a str, buf: &'a mut String) -> &'a str {
    if !needs_expanding(line) {
        return line;
    }
    buf.clear();
    expand_into(line, buf);
    buf
}

/// [`expand_tabs`] onto the end of `out`.
pub fn expand_tabs_onto(line: &str, out: &mut String) {
    if needs_expanding(line) {
        expand_into(line, out);
    } else {
        out.push_str(line);
    }
}

/// Whether any line of `text` may hold a control character, `\n` aside: when not, no line of
/// it needs [`expand_tabs`]. One pass over the block, 32 bytes a step (HL-004: checked a line
/// at a time, short lines spent most of it a byte at a time).
pub fn block_needs_expanding(text: &str) -> bool {
    let suspect = |b: u8| ((b < 0x20) & (b != b'\n')) | (b == 0x7f) | (b == 0xc2);
    let (chunks, rest) = text.as_bytes().as_chunks::<32>();
    chunks
        .iter()
        .any(|c| c.iter().fold(false, |m, &b| m | suspect(b)))
        || rest.iter().any(|&b| suspect(b))
}

/// Whether `line` holds a control character.
fn needs_expanding(line: &str) -> bool {
    // every control character starts with a byte below 0x20, 0x7f, or 0xc2 (C1): bytes first,
    // 16 at a time with no exit inside, which vectorizes (HL-004: the byte-at-a-time `any` was
    // a third of `expand_tabs`)
    let suspect = |b: u8| (b < 0x20) | (b == 0x7f) | (b == 0xc2);
    let (chunks, rest) = line.as_bytes().as_chunks::<16>();
    let found = chunks
        .iter()
        .any(|c| c.iter().fold(false, |m, &b| m | suspect(b)))
        || rest.iter().any(|&b| suspect(b));
    found && line.contains(|c: char| c.is_control())
}

/// `line`'s characters onto `out`, tabs to spaces at stops of four and other control
/// characters to a space.
fn expand_into(line: &str, out: &mut String) {
    if line.is_ascii() {
        // a column a byte, and every control character one byte
        const SPACES: &str = "    ";
        const _: () = assert!(SPACES.len() == TAB_STOP);
        let (b, start) = (line.as_bytes(), out.len());
        // indentation first, a stop a tab: most lines expanded are tabs, then text with no
        // control character, copied whole (HL-004)
        let lead = b.iter().take_while(|&&c| c == b'\t').count();
        for _ in 0..lead {
            out.push_str(SPACES);
        }
        let control = |c: u8| (c < 0x20) | (c == 0x7f);
        let (chunks, tail) = b[lead..].as_chunks::<16>();
        if !chunks
            .iter()
            .any(|c| c.iter().fold(false, |m, &x| m | control(x)))
            && !tail.iter().any(|&x| control(x))
        {
            out.push_str(&line[lead..]);
            return;
        }
        let mut from = lead;
        for (i, &b) in b.iter().enumerate().skip(lead) {
            if !control(b) {
                continue;
            }
            out.push_str(&line[from..i]);
            let n = if b == b'\t' {
                TAB_STOP - (out.len() - start) % TAB_STOP
            } else {
                1
            };
            out.push_str(&SPACES[..n]);
            from = i + 1;
        }
        out.push_str(&line[from..]);
        return;
    }
    expand_chars(line, out);
}

/// [`expand_into`] a character at a time, for a line past ASCII.
fn expand_chars(line: &str, out: &mut String) {
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
}

/// Split a line into pieces at most `width` columns wide. An empty line is one empty chunk.
fn chunks(line: &str, width: u16) -> Vec<&str> {
    // ASCII past `expand_tabs` has no control characters: a column a byte (LAY-01)
    if line.is_ascii() {
        let (len, width) = (line.len(), usize::from(width.max(1)));
        let starts = (0..len.max(1)).step_by(width);
        return starts
            .map(|i| &line[i.min(len)..(i + width).min(len)])
            .collect();
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_expand_to_stops() {
        assert_eq!(expand_tabs("a\tb\t\tc"), "a   b       c");
    }

    /// ASCII's byte path expands as the character path does, and `expand_tabs_in` as
    /// `expand_tabs`, whatever the control bytes and wherever they fall against the 16-byte
    /// check (HL-004).
    #[test]
    fn ascii_expands_as_chars_do() {
        let mut seed = 0x9e37_79b9_u32;
        let mut buf = String::new();
        for len in 0..80 {
            for _ in 0..40 {
                let line: String = (0..len)
                    .map(|_| {
                        seed ^= seed << 13;
                        seed ^= seed >> 17;
                        seed ^= seed << 5;
                        match seed % 8 {
                            0 => '\t',
                            1 => char::from((seed >> 8) as u8 % 0x20),
                            2 => '\x7f',
                            _ => char::from(b'a' + (seed >> 8) as u8 % 26),
                        }
                    })
                    .collect();
                let (mut fast, mut slow) = (String::new(), String::new());
                expand_into(&line, &mut fast);
                expand_chars(&line, &mut slow);
                assert_eq!(fast, slow, "{line:?}");
                assert_eq!(
                    expand_tabs_in(&line, &mut buf),
                    expand_tabs(&line),
                    "{line:?}"
                );
            }
        }
        assert_eq!(expand_tabs("\u{85}x\tñ\t"), " x  ñ   ");
        assert_eq!(expand_tabs_in("ñ\u{9f}", &mut buf), "ñ ");
    }

    #[test]
    fn chunking() {
        assert_eq!(chunks("abcdefg", 3), ["abc", "def", "g"]);
        assert_eq!(chunks("", 3), [""]);
        assert_eq!(chunks("日本語", 4), ["日本", "語"]);
    }
}
