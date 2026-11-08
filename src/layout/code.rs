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
    if expand_tabs_to(line, buf) { buf } else { line }
}

/// [`expand_tabs`] into `buf` when the line changes, and whether it did: `false` leaves `buf`
/// as it was, the line itself the answer.
fn expand_tabs_to(line: &str, buf: &mut String) -> bool {
    if !needs_expanding(line) {
        return false;
    }
    buf.clear();
    expand_into(line, buf);
    true
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
    expand_chars_at(line, 0, out);
}

/// [`expand_chars`] from column `col`.
fn expand_chars_at(line: &str, mut col: usize, out: &mut String) {
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

/// The room [`expand_block`] keeps past what it has written: a gap's copy writes 64 bytes
/// whatever its length.
const ROOM: usize = 64;

/// Up to `max` (one or more) of `text`'s lines, split as `ada_parse::lex_lines` splits them,
/// into `out`, each as [`expand_tabs`] expands it, joined by their `\n`s (a `\r` before one,
/// or at the end, kept for the split to drop): where the lines left start in `text`, `None`
/// once its last line is in, as `lex_lines` returns it.
///
/// One pass over the block, 64 bytes a step (X-088: a line at a time, `needs_expanding`'s and
/// `expand_into`'s loops each ran the line's length and missed their exits). A step's `\n`s,
/// tabs and other stops (control characters, bytes past ASCII) are masks; the loop goes from
/// a run of tabs to the next (an indentation is one run) and to each other stop, a `\n`
/// costing nothing but where the mask says its line starts. The text between is copied 64
/// bytes whatever its length, a run's spaces 32 at a time. A line past ASCII is expanded a
/// character at a time from its first such byte.
pub fn expand_block(text: &str, max: usize, out: &mut String) -> Option<usize> {
    debug_assert!(max > 0);
    let b = text.as_bytes();
    out.clear();
    // a tab adds up to three bytes: Go's blocks grow 13%, so an eighth more grew most of them
    out.reserve(b.len() + b.len() / 4 + ROOM);
    let mut o = 0; // bytes written, past `out`'s length until the end
    let mut line = 0; // where the line being written starts in `out`
    let mut lines = 0; // the `\n`s before the step
    let mut at = 0; // `b`'s first byte not written
    let mut w = 0; // the step's start
    'step: loop {
        let Stops {
            mut nl,
            mut tab,
            mut other,
            nls,
        } = stops64(b, w);
        // the `max`-th line's `\n` in the step: nothing past it
        let mut end = None;
        if lines + nls >= max {
            let mut m = nl;
            for _ in 0..max - lines - 1 {
                m &= m - 1;
            }
            let q = m.trailing_zeros() as usize;
            (nl, tab, other) = (nl & below(q), tab & below(q), other & below(q));
            end = Some(q);
        }
        let mut s = tab | other;
        while s != 0 {
            let p = s.trailing_zeros() as usize;
            let j = w + p;
            // SAFETY: `at <= j`, and `gap` makes its room
            unsafe { gap(b, at, j, out, o) };
            o += j - at;
            // the line starts past a `\n` since `at`, if there is one: the gap's bytes are
            // `o - j` on in `out`
            let starts = nl & below(p) & !below(at.saturating_sub(w));
            if starts != 0 {
                line = o - (j - (w + top(starts) + 1));
            }
            if other >> p & 1 != 0 {
                // rare: one stop, written by the String's own means
                // SAFETY: `out[..o]` written, whole characters
                unsafe { out.as_mut_vec().set_len(o) };
                let c = b[j];
                if c < 0x80 {
                    // a `\r` is dropped by the split before a `\n` or at the end; any other
                    // control character, or a `\r` elsewhere, is a space
                    let keep = c == b'\r' && b.get(j + 1).is_none_or(|&n| n == b'\n');
                    out.push(if keep { '\r' } else { ' ' });
                    (o, at) = (o + 1, j + 1);
                    s &= s - 1;
                    out.reserve(ROOM);
                    continue;
                }
                // `j` is a character's first byte (every byte past ASCII is a stop, and a
                // line past ASCII is written to its end here); the next step from its `\n`
                let e = memchr::memchr(b'\n', &b[j..]).map_or(b.len(), |k| j + k);
                let rest = &text[j..e];
                let (rest, cr) = rest.strip_suffix('\r').map_or((rest, false), |r| (r, true));
                expand_chars_at(rest, o - line, out);
                if cr {
                    out.push('\r');
                }
                if e == b.len() {
                    return None;
                }
                lines += (nl & below(p)).count_ones() as usize;
                (o, at, w) = (out.len(), e, e);
                out.reserve(ROOM);
                continue 'step;
            }
            // a run of tabs, the first to its stop and the rest a stop each
            let k = (!(tab >> p)).trailing_zeros() as usize;
            let n = TAB_STOP - (o - line) % TAB_STOP + TAB_STOP * (k - 1);
            // SAFETY: `out[..o]` written
            unsafe { spaces(out, o, n) };
            (o, at) = (o + n, j + k);
            s &= !below(p + k);
        }
        // a `\n` past the last stop starts the line
        let starts = nl & !below(at.saturating_sub(w));
        if starts != 0 {
            line = o + (w + top(starts) + 1 - at);
        }
        if let Some(q) = end {
            // the last line's `\n` isn't the text's: the lines left start past it
            // SAFETY: `at <= w + q`, `gap` makes its room; `out[..o]` written, whole characters
            unsafe {
                gap(b, at, w + q, out, o);
                out.as_mut_vec().set_len(o + w + q - at);
            }
            return Some(w + q + 1);
        }
        lines += nls;
        w += 64;
        if w >= b.len() {
            // SAFETY: as above
            unsafe {
                gap(b, at, b.len(), out, o);
                out.as_mut_vec().set_len(o + b.len() - at);
            }
            return None;
        }
    }
}

/// The bits below bit `q` (up to 64).
#[inline(always)]
fn below(q: usize) -> u64 {
    if q >= 64 { u64::MAX } else { (1 << q) - 1 }
}

/// The highest set bit of `m`, not 0.
#[inline(always)]
fn top(m: u64) -> usize {
    63 - m.leading_zeros() as usize
}

/// `b[from..to]` written at `o` into `out`'s spare room, with [`ROOM`] left past it: 64 bytes
/// copied whatever the length where `b` has them, so a gap's length decides no branch.
///
/// # Safety
///
/// `out[..o]` is written, `out`'s length at most `o`, and `from <= to <= b.len()`.
#[inline(always)]
unsafe fn gap(b: &[u8], from: usize, to: usize, out: &mut String, o: usize) {
    let n = to - from;
    // SAFETY: the caller's
    let v = unsafe { out.as_mut_vec() };
    if v.capacity() - o < n + ROOM {
        // SAFETY: the caller's
        unsafe { v.set_len(o) };
        v.reserve(n + ROOM);
    }
    // SAFETY: `b` holds `from..to`, and 64 bytes from `from` where the first branch reads them;
    // `v` has room for `o + n + ROOM`
    unsafe {
        let (src, dst) = (b.as_ptr().add(from), v.as_mut_ptr().add(o));
        if from + 64 <= b.len() {
            std::ptr::copy_nonoverlapping(src, dst, 64);
            if n > 64 {
                std::ptr::copy_nonoverlapping(src.add(64), dst.add(64), n - 64);
            }
        } else {
            std::ptr::copy_nonoverlapping(src, dst, n);
        }
    }
}

/// `n` spaces written at `o` into `out`'s spare room, 32 a store.
///
/// # Safety
///
/// `out[..o]` is written and `out`'s length at most `o`.
#[inline(always)]
unsafe fn spaces(out: &mut String, o: usize, n: usize) {
    // SAFETY: the caller's
    let v = unsafe { out.as_mut_vec() };
    if v.capacity() - o < n + 32 {
        // SAFETY: the caller's
        unsafe { v.set_len(o) };
        v.reserve(n + 32);
    }
    let mut i = 0;
    while i < n {
        // SAFETY: `v` has room for `o + n + 32`
        unsafe {
            v.as_mut_ptr()
                .add(o + i)
                .cast::<[u8; 32]>()
                .write_unaligned([b' '; 32]);
        }
        i += 32;
    }
}

/// A step's stops, byte `i` at bit `i`: its `\n`s, its tabs, the rest of its control
/// characters and bytes past ASCII, and how many `\n`s.
struct Stops {
    nl: u64,
    tab: u64,
    other: u64,
    nls: usize,
}

/// [`expand_block`]'s stops in `b[w..]`'s next 64 bytes; past `b`'s end none.
#[inline(always)]
fn stops64(b: &[u8], w: usize) -> Stops {
    match b.get(w..).and_then(<[u8]>::first_chunk::<64>) {
        Some(c) => stops(c),
        None => {
            let mut c = [b' '; 64];
            let rest = &b[w.min(b.len())..];
            c[..rest.len()].copy_from_slice(rest);
            stops(&c)
        }
    }
}

/// The stops of `c`: SSE2's compares on x86-64 (a byte past ASCII is below 0x20 as a signed
/// one), the `\n`s counted from theirs (`psadbw`: x86-64's floor has no `popcnt`), a byte at a
/// time elsewhere.
#[inline(always)]
fn stops(c: &[u8; 64]) -> Stops {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::*;
        // SAFETY: SSE2 is part of x86-64; each load reads 16 of `c`'s 64 bytes, unaligned
        unsafe {
            let (lo, del) = (_mm_set1_epi8(0x20), _mm_set1_epi8(0x7f));
            let (lf, ht) = (_mm_set1_epi8(b'\n' as i8), _mm_set1_epi8(b'\t' as i8));
            let bits = |v| u64::from(_mm_movemask_epi8(v) as u16);
            let (mut nl, mut tab, mut stop) = (0, 0, 0);
            let mut sum = _mm_setzero_si128();
            for q in [0, 16, 32, 48] {
                let v = _mm_loadu_si128(c.as_ptr().add(q).cast());
                let n = _mm_cmpeq_epi8(v, lf);
                nl |= bits(n) << q;
                tab |= bits(_mm_cmpeq_epi8(v, ht)) << q;
                stop |= bits(_mm_or_si128(_mm_cmplt_epi8(v, lo), _mm_cmpeq_epi8(v, del))) << q;
                // each `\n` a -1 in its lane: the lanes' negated sums are the counts
                sum = _mm_sub_epi8(sum, n);
            }
            let s = _mm_sad_epu8(sum, _mm_setzero_si128());
            let nls = _mm_cvtsi128_si32(s) + _mm_cvtsi128_si32(_mm_unpackhi_epi64(s, s));
            Stops {
                nl,
                tab,
                other: stop & !(nl | tab),
                nls: nls as usize,
            }
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    stops_bytes(c)
}

/// [`stops`] a byte at a time, where x86-64's SSE2 isn't.
#[cfg(any(test, not(target_arch = "x86_64")))]
fn stops_bytes(c: &[u8; 64]) -> Stops {
    let mask = |f: &dyn Fn(u8) -> bool| {
        c.iter()
            .enumerate()
            .fold(0, |m, (i, &b)| m | u64::from(f(b)) << i)
    };
    let nl = mask(&|b| b == b'\n');
    let tab = mask(&|b| b == b'\t');
    let stop = mask(&|b| !(0x20..0x7f).contains(&b));
    Stops {
        nl,
        tab,
        other: stop & !(nl | tab),
        nls: nl.count_ones() as usize,
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

    /// Up to `max` of `text`'s lines, split and expanded a line at a time as ada-parse's
    /// `split_lines` and `prep` did before X-088, and where the lines left start.
    fn lines_expanded(text: &str, max: usize) -> (Vec<String>, Option<usize>) {
        let (mut lines, mut p) = (Vec::new(), 0);
        for _ in 0..max {
            let e = text[p..].find('\n').map_or(text.len(), |i| p + i);
            let line = &text[p..e];
            lines.push(expand_tabs(line.strip_suffix('\r').unwrap_or(line)).into_owned());
            if e == text.len() {
                return (lines, None);
            }
            p = e + 1;
        }
        (lines, Some(p))
    }

    /// A block expanded at once splits into the lines a line at a time gives, and stops where
    /// it does, whatever the stops and wherever they fall against the 64-byte steps (X-088).
    #[test]
    fn block_expands_as_its_lines_do() {
        let pieces = [
            "\t", "\t", "\t", "\n", "\n", "\r\n", "\r", "\u{7f}", "\u{1b}", "\0", "é", "日",
            "\u{85}", "\u{9f}", "ñ\t", "ab", "abc", "abcdefgh", "    ",
        ];
        let mut seed = 0x2545_f491_u32;
        let mut out = String::new();
        for len in (0..40).chain([60, 90, 140]) {
            for _ in 0..60 {
                let text: String = (0..len)
                    .map(|_| {
                        seed ^= seed << 13;
                        seed ^= seed >> 17;
                        seed ^= seed << 5;
                        pieces[seed as usize % pieces.len()]
                    })
                    .collect();
                for max in [1, 2, 3, 7, usize::MAX] {
                    let (want, next) = lines_expanded(&text, max);
                    let got = expand_block(&text, max, &mut out);
                    assert_eq!(got, next, "{text:?} max {max}");
                    assert_eq!(
                        lines_expanded(&out, usize::MAX).0,
                        want,
                        "{text:?} max {max}"
                    );
                }
            }
        }
        assert_eq!(expand_block("a\tb\r\n\tc\r", 9, &mut out), None);
        assert_eq!(out, "a   b\r\n    c\r");
        assert_eq!(expand_block("\tx\ny\nz", 2, &mut out), Some(5));
        assert_eq!(out, "    x\ny");
    }

    /// A step's stops on SSE2 are the bytes' own (X-088).
    #[test]
    fn stops_are_each_bytes() {
        let mut c = [0u8; 64];
        for k in 0..=255u8 {
            for (i, b) in c.iter_mut().enumerate() {
                *b = k.wrapping_add((i as u8).wrapping_mul(37));
            }
            let (a, b) = (stops(&c), stops_bytes(&c));
            assert_eq!(
                (a.nl, a.tab, a.other, a.nls),
                (b.nl, b.tab, b.other, b.nls),
                "{k}"
            );
        }
        let c = [b'\n'; 64];
        assert_eq!(stops(&c).nls, 64);
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
