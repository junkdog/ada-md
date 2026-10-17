//! The styled greedy word wrapper (architecture §3, "Wrapping").
//!
//! Runs are flattened into one buffer of styled pieces, split into words at whitespace and after
//! `-` and `/`, and packed greedily. A word longer than the line is hard-broken. Style changes
//! inside a word (`**foo**bar`) never create a break opportunity. Every line keeps the source
//! lines of the pieces on it.
//!
//! A piece may have a searchable *body* between decorations (inline code padding, an image icon).
//! The bodies of a [`Wrapped`], in order, are its search unit's text (ADR-12), so layout can map
//! every row back to the stretch of the unit it shows.

use std::borrow::Cow;
use std::ops::Range;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::doc::{Doc, InlineStyle, LineRange, LinkId, Run, RunKind, UnitId};
use crate::theme::{Glyphs, StyleKey};

/// Non-breaking padding; rendered as a plain space.
pub const NBSP: char = '\u{a0}';

/// A styled stretch of the flattened buffer.
#[derive(Clone, Copy, Debug)]
pub struct Piece {
    pub start: u32,
    pub end: u32,
    pub key: StyleKey,
    pub inline: InlineStyle,
    pub link: Option<LinkId>,
    pub src_line: u32,
    /// The searchable part of the piece, `body_start..body_end` in the buffer.
    pub body_start: u32,
    pub body_end: u32,
    /// Where the body starts in the unit's text.
    pub unit_off: u32,
}

/// Part of a line: a slice of the buffer inside one piece.
#[derive(Clone, Copy, Debug)]
pub struct Seg {
    pub start: u32,
    pub end: u32,
    pub width: u16,
    pub piece: u32,
}

#[derive(Debug, Default)]
pub struct Line {
    /// Indices into [`Wrapped::segs`].
    pub segs: Range<u32>,
    pub width: u16,
    pub src: LineRange,
}

/// Wrapped text. Its buffers can be reused for the next wrap ([`Wrapped::recycle`]), since
/// layout wraps one paragraph after another.
#[derive(Debug, Default)]
pub struct Wrapped {
    pub buf: String,
    pub pieces: Vec<Piece>,
    pub segs: Vec<Seg>,
    pub lines: Vec<Line>,
    /// The search unit the bodies belong to.
    pub unit: Option<UnitId>,
    /// The length of the unit's text so far.
    unit_len: u32,
    toks: Vec<Tok>,
}

impl Wrapped {
    /// Nothing yet, for search unit `unit`.
    pub fn new(unit: Option<UnitId>) -> Self {
        Self {
            unit,
            ..Self::default()
        }
    }

    /// Empty again, for search unit `unit`, keeping the buffers.
    pub fn recycle(mut self, unit: Option<UnitId>) -> Self {
        self.buf.clear();
        self.pieces.clear();
        self.segs.clear();
        self.lines.clear();
        self.toks.clear();
        self.unit = unit;
        self.unit_len = 0;
        self
    }

    pub fn line_segs(&self, line: &Line) -> &[Seg] {
        &self.segs[line.segs.start as usize..line.segs.end as usize]
    }

    pub fn seg_text(&self, seg: &Seg) -> &str {
        &self.buf[seg.start as usize..seg.end as usize]
    }

    pub fn width(&self) -> u16 {
        self.lines.iter().map(|l| l.width).max().unwrap_or(0)
    }
}

/// Wrap `runs` to `avail` columns, into `w` (empty, from [`Wrapped::new`] or
/// [`Wrapped::recycle`]). `key` is the base style of the text; `suffix` is glued to the last
/// word (a heading's trailing pad).
pub fn wrap(
    mut w: Wrapped,
    runs: &[Run],
    key: StyleKey,
    avail: u16,
    doc: &Doc,
    glyphs: &Glyphs,
    suffix: Option<&str>,
) -> Wrapped {
    flatten(runs, key, doc, glyphs, &mut w);
    if let Some(suffix) = suffix.filter(|s| !s.is_empty()) {
        let line = w.pieces.last().map_or(0, |p| p.src_line);
        let text: String = suffix
            .chars()
            .map(|c| if c == ' ' { NBSP } else { c })
            .collect();
        push_piece(&mut w, &text, key, InlineStyle::NONE, None, line);
    }
    pack(&mut w, avail.max(1));
    w
}

/// The widths `runs` can wrap to: `(min, max)`. `min` is the widest unbreakable word and `max`
/// the widest line when nothing wraps. Agrees with [`wrap`]: at `avail >= min` no word is
/// hard-broken, and at `avail >= max` no line wraps.
pub fn measure(runs: &[Run], doc: &Doc, glyphs: &Glyphs) -> (u16, u16) {
    let mut w = Wrapped::default();
    flatten(runs, StyleKey::Text, doc, glyphs, &mut w);
    let (mut min, mut max, mut line) = (0u16, 0u16, 0u16);
    let (mut started, mut pending) = (false, false);
    tokenize(&w.buf, &mut w.toks);
    for &tok in &w.toks {
        match tok {
            Tok::Space { .. } => pending = started,
            Tok::Newline => (line, started, pending) = (0, false, false),
            Tok::Word { width, .. } => {
                line = line
                    .saturating_add(u16::from(pending))
                    .saturating_add(width);
                (started, pending) = (true, false);
                min = min.max(width);
                max = max.max(line);
            }
        }
    }
    (min, max)
}

/// The text of a search unit made of `runs`: the bodies [`wrap`] gives them, in order.
pub fn unit_text(runs: &[Run], doc: &Doc, glyphs: &Glyphs) -> String {
    let mut w = Wrapped::default();
    flatten(runs, StyleKey::Text, doc, glyphs, &mut w);
    w.pieces
        .iter()
        .map(|p| &w.buf[p.body_start as usize..p.body_end as usize])
        .collect()
}

/// Wrap plain text that has a single style.
pub fn wrap_plain(text: &str, key: StyleKey, src_line: u32, avail: u16) -> Wrapped {
    let mut w = Wrapped::default();
    push_piece(&mut w, text, key, InlineStyle::NONE, None, src_line);
    pack(&mut w, avail.max(1));
    w
}

/// Wrap single-style text whose middle part, `body`, is the text of search unit `unit`.
pub fn wrap_unit(
    [pre, body, post]: [&str; 3],
    unit: UnitId,
    key: StyleKey,
    src_line: u32,
    avail: u16,
) -> Wrapped {
    let mut w = Wrapped::new(Some(unit));
    let look = (key, InlineStyle::NONE, None);
    push_parts(&mut w, [pre, body, post], look, src_line);
    pack(&mut w, avail.max(1));
    w
}

/// The columns `s` takes: its length when it's ASCII, otherwise [`UnicodeWidthStr::width`].
/// A `\r` before a `\n` takes no column of its own there, so it takes the slow path.
pub fn text_width(s: &str) -> usize {
    if s.bytes().all(|b| b < 0x80 && b != b'\r') {
        s.len()
    } else {
        s.width()
    }
}

/// What the buffer holds for `c`: tabs, carriage returns and other control characters, except
/// newlines, become spaces.
pub fn clean_char(c: char) -> char {
    match c {
        '\t' | '\r' => ' ',
        c if c.is_control() && c != '\n' => ' ',
        c => c,
    }
}

/// `text` as the buffer holds it.
pub fn clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    push_clean(&mut out, text);
    out
}

/// Append `s` as the buffer holds it, copied whole when [`clean_char`] would change nothing.
fn push_clean(buf: &mut String, s: &str) {
    if needs_clean(s) {
        buf.extend(s.chars().map(clean_char));
    } else {
        buf.push_str(s);
    }
}

/// Whether `s` has a control character other than `\n`. Every one starts with a byte below
/// 0x20, 0x7f, or 0xc2 (C1 controls, but also `NBSP`, `©`, `°`), so only those look closer.
fn needs_clean(s: &str) -> bool {
    let suspect = |b: u8| (b < 0x20 && b != b'\n') || b == 0x7f || b == 0xc2;
    s.bytes().any(suspect) && s.chars().any(|c| c != '\n' && c.is_control())
}

fn push_piece(
    w: &mut Wrapped,
    text: &str,
    key: StyleKey,
    inline: InlineStyle,
    link: Option<LinkId>,
    src_line: u32,
) {
    push_parts(w, [text, "", ""], (key, inline, link), src_line);
}

/// Push one piece: decoration, a searchable body, decoration.
fn push_parts(
    w: &mut Wrapped,
    [pre, body, post]: [&str; 3],
    (key, inline, link): (StyleKey, InlineStyle, Option<LinkId>),
    src_line: u32,
) {
    if pre.is_empty() && body.is_empty() && post.is_empty() {
        return;
    }
    let start = w.buf.len() as u32;
    push_clean(&mut w.buf, pre);
    let body_start = w.buf.len() as u32;
    push_clean(&mut w.buf, body);
    let body_end = w.buf.len() as u32;
    push_clean(&mut w.buf, post);
    w.pieces.push(Piece {
        start,
        end: w.buf.len() as u32,
        key,
        inline,
        link,
        src_line,
        body_start,
        body_end,
        unit_off: w.unit_len,
    });
    w.unit_len += body_end - body_start;
}

/// `text` with its newlines as spaces, borrowed when it has none.
fn unfold(text: &str) -> Cow<'_, str> {
    if text.contains('\n') {
        Cow::Owned(text.replace('\n', " "))
    } else {
        Cow::Borrowed(text)
    }
}

fn flatten(runs: &[Run], key: StyleKey, doc: &Doc, glyphs: &Glyphs, w: &mut Wrapped) {
    let mut code_pad: Option<String> = None;
    for run in runs {
        let line = run.src_line;
        let look = (key, run.style, run.link);
        match run.kind {
            RunKind::Text if run.style.contains(InlineStyle::CODE) => {
                let pad = code_pad.get_or_insert_with(|| {
                    std::iter::repeat_n(NBSP, glyphs.code_pad as usize).collect()
                });
                push_parts(w, [pad, &unfold(&run.text), pad], look, line);
            }
            RunKind::Text => push_parts(w, ["", &unfold(&run.text), ""], look, line),
            RunKind::SoftBreak => push_parts(w, ["", " ", ""], look, line),
            RunKind::HardBreak => push_parts(w, ["", "\n", ""], look, line),
            RunKind::InlineImage(link) => {
                let icon: String = glyphs
                    .image
                    .chars()
                    .map(|c| if c == ' ' { NBSP } else { c })
                    .collect();
                let icon = if run.text.is_empty() {
                    icon.trim_end_matches(NBSP)
                } else {
                    &icon
                };
                let look = (StyleKey::Image, run.style, Some(link));
                push_parts(w, [icon, &unfold(&run.text), ""], look, line);
            }
            RunKind::FootnoteRef(id) => {
                let n = doc.footnotes.get(id as usize).map_or(0, |f| f.number);
                let look = (StyleKey::FootnoteRef, InlineStyle::NONE, run.link);
                push_parts(w, [&format!("[{n}]"), "", ""], look, line);
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Tok {
    Word { start: u32, end: u32, width: u16 },
    Space { start: u32, end: u32 },
    Newline,
}

/// Split `buf` into `toks`.
fn tokenize(buf: &str, toks: &mut Vec<Tok>) {
    toks.clear();
    toks.reserve(buf.len() / 3 + 1);
    let mut word: Option<u32> = None;
    let close = |toks: &mut Vec<Tok>, word: &mut Option<u32>, end: usize| {
        if let Some(start) = word.take() {
            let width = text_width(&buf[start as usize..end]).min(u16::MAX as usize) as u16;
            toks.push(Tok::Word {
                start,
                end: end as u32,
                width,
            });
        }
    };
    // Every delimiter is ASCII, and no byte of a multi-byte character is, so bytes will do.
    let bytes = buf.as_bytes();
    for (i, &c) in bytes.iter().enumerate() {
        match c {
            b' ' => {
                close(toks, &mut word, i);
                if !matches!(toks.last(), Some(Tok::Space { .. })) {
                    toks.push(Tok::Space {
                        start: i as u32,
                        end: i as u32 + 1,
                    });
                }
            }
            b'\n' => {
                close(toks, &mut word, i);
                toks.push(Tok::Newline);
            }
            _ => {
                let start = *word.get_or_insert(i as u32);
                let breaks_after = (c == b'-' || c == b'/')
                    && start < i as u32
                    && bytes.get(i + 1).is_some_and(|&n| n != b' ' && n != b'\n');
                if breaks_after {
                    close(toks, &mut word, i + 1);
                }
            }
        }
    }
    close(toks, &mut word, buf.len());
}

struct Packer<'w> {
    pieces: &'w [Piece],
    buf: &'w str,
    cursor: usize,
    /// The line being packed; its `segs` start where `segs` ends.
    line: Line,
    segs: &'w mut Vec<Seg>,
    lines: &'w mut Vec<Line>,
}

impl Packer<'_> {
    /// Append `[start, end)`, `width` columns wide, to the current line, split at piece
    /// boundaries.
    fn add(&mut self, mut start: u32, end: u32, width: u16) {
        let mut whole = true;
        while start < end {
            while self.cursor + 1 < self.pieces.len() && self.pieces[self.cursor].end <= start {
                self.cursor += 1;
            }
            let piece = self.pieces[self.cursor];
            let stop = end.min(piece.end).max(start + 1);
            let width = if whole && stop == end {
                width
            } else {
                text_width(&self.buf[start as usize..stop as usize]) as u16
            };
            whole = false;
            self.segs.push(Seg {
                start,
                end: stop,
                width,
                piece: self.cursor as u32,
            });
            self.line.width = self.line.width.saturating_add(width);
            self.line.src = self.line.src.union(LineRange::line(piece.src_line));
            start = stop;
        }
    }

    /// Whether the line being packed is empty.
    fn is_empty(&self) -> bool {
        self.segs.len() as u32 == self.line.segs.start
    }

    fn flush(&mut self) {
        let end = self.segs.len() as u32;
        self.line.segs.end = end;
        let line = std::mem::replace(
            &mut self.line,
            Line {
                segs: end..end,
                ..Line::default()
            },
        );
        self.lines.push(line);
    }
}

fn pack(w: &mut Wrapped, avail: u16) {
    let mut toks = std::mem::take(&mut w.toks);
    tokenize(&w.buf, &mut toks);
    let mut p = Packer {
        pieces: &w.pieces,
        buf: &w.buf,
        cursor: 0,
        line: Line::default(),
        segs: &mut w.segs,
        lines: &mut w.lines,
    };
    let mut pending: Option<(u32, u32)> = None;
    for &tok in &toks {
        match tok {
            Tok::Space { start, end } => {
                if !p.is_empty() {
                    pending = Some((start, end));
                }
            }
            Tok::Newline => {
                p.flush();
                pending = None;
            }
            Tok::Word { start, end, width } => {
                let space = u16::from(pending.is_some());
                if p.line.width.saturating_add(space).saturating_add(width) <= avail {
                    if let Some((s, e)) = pending {
                        p.add(s, e, 1);
                    }
                    p.add(start, end, width);
                } else if width <= avail {
                    p.flush();
                    p.add(start, end, width);
                } else {
                    // Longer than a whole line: hard-break it, starting on a fresh line.
                    if !p.is_empty() {
                        p.flush();
                    }
                    let word = &w.buf[start as usize..end as usize];
                    for (i, c) in word.char_indices() {
                        let cw = c.width().unwrap_or(0) as u16;
                        if p.line.width + cw > avail && !p.is_empty() {
                            p.flush();
                        }
                        let s = start + i as u32;
                        let e = s + c.len_utf8() as u32;
                        p.add(s, e, text_width(&w.buf[s as usize..e as usize]) as u16);
                    }
                }
                pending = None;
            }
        }
    }
    if !p.is_empty() || p.lines.is_empty() {
        p.flush();
    }
    w.toks = toks;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::parse;

    fn lines(md: &str, avail: u16) -> Vec<String> {
        let doc = parse(md);
        let crate::doc::BlockKind::Paragraph(inl) = &doc.blocks[0].kind else {
            panic!("not a paragraph")
        };
        let glyphs = Glyphs {
            code_pad: 1,
            ..Glyphs::default()
        };
        let w = wrap(
            Wrapped::new(None),
            &inl.runs,
            StyleKey::Text,
            avail,
            &doc,
            &glyphs,
            None,
        );
        w.lines
            .iter()
            .map(|l| {
                w.line_segs(l)
                    .iter()
                    .map(|s| w.seg_text(s).replace(NBSP, "·"))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn greedy_fill() {
        assert_eq!(
            lines("the quick brown fox jumps over the lazy dog", 15),
            ["the quick brown", "fox jumps over", "the lazy dog"]
        );
    }

    #[test]
    fn style_changes_do_not_break_words() {
        assert_eq!(lines("aaaa **bb**cc", 6), ["aaaa", "bbcc"]);
    }

    #[test]
    fn breaks_after_slash_and_hyphen() {
        assert_eq!(
            lines("see ../30-resources/ratatui/README.md now", 18),
            ["see ../30-", "resources/ratatui/", "README.md now"]
        );
    }

    #[test]
    fn long_words_are_hard_broken() {
        assert_eq!(lines("x abcdefghij y", 4), ["x", "abcd", "efgh", "ij y"]);
    }

    #[test]
    fn inline_code_keeps_its_padding_attached() {
        assert_eq!(lines("call `f()` now", 8), ["call", "·f()·", "now"]);
    }

    #[test]
    fn hard_breaks_and_soft_breaks() {
        assert_eq!(lines("one\ntwo  \nthree", 80), ["one two", "three"]);
    }

    #[test]
    fn wide_characters_count_double() {
        assert_eq!(lines("日本語 テキスト", 7), ["日本語", "テキス", "ト"]);
    }

    #[test]
    fn text_width_agrees_with_unicode_width() {
        let ascii = (0u8..0x80).map(char::from);
        for a in ascii.clone() {
            for b in ascii.clone() {
                let s = format!("x{a}{b}y");
                assert_eq!(text_width(&s), s.width(), "{s:?}");
            }
        }
        for s in [
            "日本語",
            "a\u{a0}b",
            "e\u{301}",
            "👍🏽 ok",
            "\u{2764}\u{fe0f}",
        ] {
            assert_eq!(text_width(s), s.width(), "{s:?}");
        }
    }

    #[test]
    fn needs_clean_agrees_with_clean_char() {
        for c in (0..0x800).filter_map(char::from_u32) {
            let s = format!("a{c}b");
            let changed = s.chars().map(clean_char).ne(s.chars());
            assert_eq!(needs_clean(&s), changed, "{c:?}");
        }
    }

    #[test]
    fn source_lines_per_row() {
        let doc = parse("alpha beta\ngamma delta\n");
        let crate::doc::BlockKind::Paragraph(inl) = &doc.blocks[0].kind else {
            panic!()
        };
        let w = wrap(
            Wrapped::new(None),
            &inl.runs,
            StyleKey::Text,
            12,
            &doc,
            &Glyphs::default(),
            None,
        );
        let src: Vec<_> = w.lines.iter().map(|l| (l.src.start, l.src.end)).collect();
        assert_eq!(src, [(1, 1), (2, 2)]);
    }
}
