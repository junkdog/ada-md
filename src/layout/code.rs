//! Code blocks (ADR-20): a background-filled block with the language on its top row, but in a
//! print ([`LayoutParams::code_labels`](super::LayoutParams::code_labels)). With `code_wrap` on,
//! long lines wrap with the theme's `↪`. With it off, each source line is one row whose code span
//! is a fixed-width window, sliced at paint time by the shared horizontal offset, so scrolling
//! sideways never relayouts. Highlighting is overlaid at paint, too. A printed file of code
//! ([`LayoutParams::bare`](super::LayoutParams::bare)) has no columns of the block's own left of
//! its lines, nor right of them when they wrap: a wrapped line's mark takes its continuation's
//! first column.

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
    // a print never scrolls sideways, so a bare block's left clip mark never shows, and its
    // right one only where lines don't wrap
    let bare = b.params.bare;
    let lead = if bare {
        0
    } else {
        [&g.wrap_mark, &g.code_clip[0]]
            .iter()
            .map(|s| s.width())
            .max()
            .unwrap_or(0)
            .max(1) as u16
    };
    let trail = if bare && b.params.code_wrap {
        0
    } else {
        g.code_clip[1].width().max(1) as u16
    };
    let inner = avail.saturating_sub(lead + trail).max(1);
    // a bare block's continuations start with the mark, so they hold that much less
    let rest = if bare {
        inner.saturating_sub(g.wrap_mark.width() as u16).max(1)
    } else {
        inner
    };

    b.begin_row();
    if lead > 0 {
        b.spaces(lead, StyleKey::CodeBlock);
    }
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
            for (i, chunk) in chunks(&line, inner, rest).into_iter().enumerate() {
                b.begin_row();
                if i == 0 {
                    if lead > 0 {
                        b.spaces(lead, StyleKey::CodeBlock);
                    }
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
            if lead > 0 {
                b.spaces(lead, StyleKey::CodeBlock);
            }
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

/// Whether any line of `text`, as the highlighter splits it (`\n`, a `\r` before it or at the
/// end dropped), may hold a control character: when not, no line of it needs
/// [`expand_tabs`]. One pass over the block, 32 bytes a step (HL-004: checked a line at a time,
/// short lines spent most of it a byte at a time). A block with a byte that may start one is
/// looked at again (`keeps_controls`), so a CRLF block or one with `«` or `±` (0xc2, a C1
/// control's lead byte too) isn't checked again line by line (X-048: INI's corpus, 74
/// instructions a line).
pub fn block_needs_expanding(text: &str) -> bool {
    let suspect = |b: u8| ((b < 0x20) & (b != b'\n')) | (b == 0x7f) | (b == 0xc2);
    let b = text.as_bytes();
    let (chunks, rest) = b.as_chunks::<32>();
    match chunks
        .iter()
        .position(|c| c.iter().fold(false, |m, &x| m | suspect(x)))
    {
        Some(n) => keeps_controls(b, n),
        None => rest.iter().any(|&x| suspect(x)) && keeps_controls(b, chunks.len()),
    }
}

/// Whether a line of `b` keeps a control character, from its 32-byte step `n` on, the first
/// with a byte that may start one: each step's such bytes as a mask, and only those looked at.
/// Out of line, so the check every block makes stays as small as it was where it's inlined
/// (X-048: inlined whole, it cost fstab and Mailmap 2–4% at the same instructions).
#[inline(never)]
fn keeps_controls(b: &[u8], mut n: usize) -> bool {
    let suspect = |b: u8| ((b < 0x20) & (b != b'\n')) | (b == 0x7f) | (b == 0xc2);
    let (chunks, rest) = b.as_chunks::<32>();
    let has = |c: &[u8; 32]| c.iter().fold(false, |m, &x| m | suspect(x));
    // the test vectorizes; the mask, taken only for a step that has such a byte, doesn't
    while let Some(k) = chunks.get(n..).and_then(|c| c.iter().position(has)) {
        n += k;
        let mut mask = chunks[n]
            .iter()
            .enumerate()
            .fold(0u32, |m, (i, &x)| m | (u32::from(suspect(x)) << i));
        while mask != 0 {
            if kept(b, n * 32 + mask.trailing_zeros() as usize) {
                return true;
            }
            mask &= mask - 1;
        }
        n += 1;
    }
    let from = b.len() - rest.len();
    (from..b.len()).any(|i| suspect(b[i]) && kept(b, i))
}

/// Whether the byte at `i`, one that may start a control character, starts one a line keeps:
/// not a `\r` the line split drops (before `\n` or at the end), nor a `0xc2` that leads a
/// printable character.
fn kept(b: &[u8], i: usize) -> bool {
    match b[i] {
        b'\r' => b.get(i + 1).is_some_and(|&n| n != b'\n'),
        // C1 controls are U+0080..=U+009F: 0xc2 then 0x80..=0x9f
        0xc2 => b.get(i + 1).is_some_and(|n| (0x80..=0x9f).contains(n)),
        _ => true,
    }
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

/// Split a line into pieces, the first at most `first` columns wide and the rest at most `rest`.
/// An empty line is one empty chunk.
fn chunks(line: &str, first: u16, rest: u16) -> Vec<&str> {
    // ASCII past `expand_tabs` has no control characters: a column a byte (LAY-01)
    if line.is_ascii() {
        let len = line.len();
        let (first, rest) = (usize::from(first.max(1)), usize::from(rest.max(1)));
        if len <= first {
            return vec![line];
        }
        let starts = std::iter::once(0).chain((first..len).step_by(rest));
        return starts
            .map(|i| &line[i..(i + if i == 0 { first } else { rest }).min(len)])
            .collect();
    }
    let mut out = Vec::new();
    let mut start = 0;
    let mut used = 0u16;
    let mut width = first;
    for (i, c) in line.char_indices() {
        let cw = c.width().unwrap_or(0) as u16;
        if used + cw > width && i > start {
            out.push(&line[start..i]);
            start = i;
            used = 0;
            width = rest;
        }
        used += cw;
    }
    out.push(&line[start..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The block's answer is "some line, as the highlighter splits it, holds a control
    /// character", wherever in a 32-byte step the byte falls (X-048).
    #[test]
    fn block_check_matches_its_lines() {
        let lines_hold_one = |t: &str| {
            t.strip_suffix('\n')
                .unwrap_or(t)
                .split('\n')
                .any(|l| l.strip_suffix('\r').unwrap_or(l).contains(char::is_control))
        };
        let pieces = [
            "\r\n", "\r", "\r\r\n", "\t", "«", "±", "\u{85}", "\u{9f}", "\u{a0}", "\u{7f}", "\n",
            "é", "\u{1b}",
        ];
        for pad in [0, 5, 30, 31, 32, 33, 63, 64] {
            for p in pieces {
                for tail in ["", "x", "\n", "x\n"] {
                    let text = format!("{}{p}{tail}", "a".repeat(pad));
                    assert_eq!(
                        block_needs_expanding(&text),
                        lines_hold_one(&text),
                        "{text:?}"
                    );
                }
            }
        }
    }

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
        assert_eq!(chunks("abcdefg", 3, 3), ["abc", "def", "g"]);
        assert_eq!(chunks("abcdef", 3, 3), ["abc", "def"]);
        assert_eq!(chunks("", 3, 3), [""]);
        assert_eq!(chunks("日本語", 4, 4), ["日本", "語"]);
        // a bare block's continuations are a mark narrower
        assert_eq!(chunks("abcdefgh", 4, 3), ["abcd", "efg", "h"]);
        assert_eq!(chunks("日本語日本", 4, 3), ["日本", "語", "日", "本"]);
    }
}
