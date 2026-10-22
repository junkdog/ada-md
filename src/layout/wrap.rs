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

use std::ops::Range;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::doc::{Doc, InlineStyle, LineRange, LinkId, Run, RunKind, UnitId};
use crate::theme::{Glyphs, StyleKey};

/// Non-breaking padding; rendered as a plain space.
pub const NBSP: char = '\u{a0}';

/// What the buffer holds for ada's own non-breaking padding (code padding, a heading's suffix,
/// an image's icon): one byte, a column wide, shown as a space (O-24). Unlike an [`NBSP`], it
/// takes as many bytes as the space it shows as, so a line's text is copied into the layout in
/// one go. The source's control characters are spaces by then ([`clean_char`]), so none is a pad.
pub const PAD: char = '\u{1f}';

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
    /// Whether its decoration holds a [`PAD`].
    pub pads: bool,
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
    /// Whether `buf` holds a [`PAD`].
    pub pads: bool,
    /// Whether `buf` holds an [`NBSP`] of the source's, which shows as a space a byte shorter.
    pub nbsp: bool,
    /// The length of the unit's text so far.
    unit_len: u32,
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
        self.unit = unit;
        self.pads = false;
        self.nbsp = false;
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
    tokenize(&w.buf, |tok| match tok {
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
    });
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
    push_parts(&mut w, [pre, body, post], look, src_line, false);
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

/// `s` cut to at most `max` columns, counted a character at a time.
pub fn cut(s: &str, max: usize) -> &str {
    let mut used = 0;
    for (i, c) in s.char_indices() {
        used += c.width().unwrap_or(0);
        if used > max {
            return &s[..i];
        }
    }
    s
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
    push_clean(&mut out, text, false);
    out
}

/// Append `s` as the buffer holds it, with its newlines as spaces too when `fold`; copied whole
/// when that would change nothing, which one pass over its bytes tells (LAY-01). Returns whether
/// `s` holds an [`NBSP`].
fn push_clean(buf: &mut String, s: &str, fold: bool) -> bool {
    if !suspect(s, fold) {
        buf.push_str(s);
        return false;
    }
    let unclean = |c: char| c.is_control() && (fold || c != '\n');
    if s.chars().any(unclean) {
        buf.extend(s.chars().map(|c| if unclean(c) { ' ' } else { c }));
    } else {
        buf.push_str(s);
    }
    s.contains(NBSP)
}

/// Append decoration `s` as the buffer holds it: its [`NBSP`]s as [`PAD`]s, the rest as
/// [`push_clean`] would. Returns whether it pushed a pad.
fn push_deco(buf: &mut String, s: &str) -> bool {
    if !suspect(s, false) {
        buf.push_str(s);
        return false;
    }
    buf.extend(
        s.chars()
            .map(|c| if c == NBSP { PAD } else { clean_char(c) }),
    );
    s.contains(NBSP)
}

/// Whether `s` may have a control character other than `\n` (or, when `fold`, a `\n`) or an
/// [`NBSP`]. Every one starts with a byte below 0x20, 0x7f, or 0xc2 (C1 controls, but also
/// `NBSP`, `©`, `°`), so only those look closer.
fn suspect(s: &str, fold: bool) -> bool {
    // folded, a newline is suspect too: tested against 0xff, which no byte of a str is
    let nl = if fold { 0xff } else { b'\n' };
    let hit = |b: u8| ((b < 0x20) & (b != nl)) | (b == 0x7f) | (b == 0xc2);
    // a chunk at a time, not stopping inside one, so the test vectorizes (LAY-01)
    s.as_bytes()
        .chunks(32)
        .any(|c| c.iter().fold(false, |acc, &b| acc | hit(b)))
}

fn push_piece(
    w: &mut Wrapped,
    text: &str,
    key: StyleKey,
    inline: InlineStyle,
    link: Option<LinkId>,
    src_line: u32,
) {
    push_parts(w, [text, "", ""], (key, inline, link), src_line, false);
}

/// Push one piece: decoration, a searchable body, decoration. The decorations' [`NBSP`]s are
/// ada's own and become [`PAD`]s; the body's are the source's and stay. `fold`: the body's
/// newlines become spaces.
fn push_parts(
    w: &mut Wrapped,
    [pre, body, post]: [&str; 3],
    (key, inline, link): (StyleKey, InlineStyle, Option<LinkId>),
    src_line: u32,
    fold: bool,
) {
    if pre.is_empty() && body.is_empty() && post.is_empty() {
        return;
    }
    let start = w.buf.len() as u32;
    let mut pads = push_deco(&mut w.buf, pre);
    let body_start = w.buf.len() as u32;
    w.nbsp |= push_clean(&mut w.buf, body, fold);
    let body_end = w.buf.len() as u32;
    pads |= push_deco(&mut w.buf, post);
    w.pads |= pads;
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
        pads,
    });
    w.unit_len += body_end - body_start;
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
                push_parts(w, [pad, &run.text, pad], look, line, true);
            }
            RunKind::Text => push_parts(w, ["", &run.text, ""], look, line, true),
            RunKind::SoftBreak => push_parts(w, ["", " ", ""], look, line, false),
            RunKind::HardBreak => push_parts(w, ["", "\n", ""], look, line, false),
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
                push_parts(w, [icon, &run.text, ""], look, line, true);
            }
            RunKind::FootnoteRef(id) => {
                let n = doc.footnotes.get(id as usize).map_or(0, |f| f.number);
                let look = (StyleKey::FootnoteRef, InlineStyle::NONE, run.link);
                push_parts(w, [&format!("[{n}]"), "", ""], look, line, false);
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

/// How [`tokenize`] sees a byte: a word's byte unless one of these.
const SPACE: u8 = 1;
const NEWLINE: u8 = 2;
/// `-` and `/`, which a word may break after.
const BREAK: u8 = 4;
/// A byte of a character past ASCII.
const WIDE: u8 = 8;

const CLASS: [u8; 256] = {
    let mut t = [0; 256];
    t[b' ' as usize] = SPACE;
    t[b'\n' as usize] = NEWLINE;
    t[b'-' as usize] = BREAK;
    t[b'/' as usize] = BREAK;
    let mut b = 0x80;
    while b < 256 {
        t[b] = WIDE;
        b += 1;
    }
    t
};

/// Split `buf` into tokens, handed to `f` in order: no vector of them is built, and a word's
/// bytes are read in a loop of their own (LAY-01).
fn tokenize(buf: &str, mut f: impl FnMut(Tok)) {
    // Every delimiter is ASCII, and no byte of a multi-byte character is, so bytes will do.
    let bytes = buf.as_bytes();
    let mut spaced = false;
    let mut i = 0;
    while i < bytes.len() {
        match CLASS[bytes[i] as usize] {
            SPACE => {
                // a run of spaces is one
                if !spaced {
                    f(Tok::Space {
                        start: i as u32,
                        end: i as u32 + 1,
                    });
                    spaced = true;
                }
                i += 1;
            }
            NEWLINE => {
                f(Tok::Newline);
                spaced = false;
                i += 1;
            }
            _ => {
                let start = i;
                // Whether the word has a byte past ASCII. Without one its width is its length,
                // as in `text_width` (the buffer holds no `\r`: `clean_char`).
                let mut wide = false;
                while i < bytes.len() {
                    let class = CLASS[bytes[i] as usize];
                    if class & (SPACE | NEWLINE) != 0 {
                        break;
                    }
                    wide |= class == WIDE;
                    i += 1;
                    // after a `-` or `/` inside a word that goes on
                    let breaks_after = class == BREAK
                        && i - 1 > start
                        && bytes.get(i).is_some_and(|&n| n != b' ' && n != b'\n');
                    if breaks_after {
                        break;
                    }
                }
                let width = if wide {
                    text_width(&buf[start..i])
                } else {
                    i - start
                };
                f(Tok::Word {
                    start: start as u32,
                    end: i as u32,
                    width: width.min(u16::MAX as usize) as u16,
                });
                spaced = false;
            }
        }
    }
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
            // a line's words of one piece are one seg (LAY-01)
            let (piece_at, open) = (self.cursor as u32, !self.is_empty());
            match self.segs.last_mut() {
                Some(last) if open && last.piece == piece_at && last.end == start => {
                    last.end = stop;
                    last.width = last.width.saturating_add(width);
                }
                _ => self.segs.push(Seg {
                    start,
                    end: stop,
                    width,
                    piece: piece_at,
                }),
            }
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
    let mut p = Packer {
        pieces: &w.pieces,
        buf: &w.buf,
        cursor: 0,
        line: Line::default(),
        segs: &mut w.segs,
        lines: &mut w.lines,
    };
    let mut pending: Option<(u32, u32)> = None;
    tokenize(&w.buf, |tok| {
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
                        let s = start + i as u32;
                        let e = s + c.len_utf8() as u32;
                        // `text_width`, not `c.width()`: a pad is a column wide
                        let cw = text_width(&w.buf[s as usize..e as usize]) as u16;
                        if p.line.width + cw > avail && !p.is_empty() {
                            p.flush();
                        }
                        p.add(s, e, cw);
                    }
                }
                pending = None;
            }
        }
    });
    if !p.is_empty() || p.lines.is_empty() {
        p.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::parse;

    fn lines(md: &str, avail: u16) -> Vec<String> {
        lines_with(md, avail, None)
    }

    /// The lines of `md`'s first paragraph at `avail` columns, a heading's `suffix` after it,
    /// with pads as `·`.
    fn lines_with(md: &str, avail: u16, suffix: Option<&str>) -> Vec<String> {
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
            suffix,
        );
        w.lines
            .iter()
            .map(|l| {
                w.line_segs(l)
                    .iter()
                    .map(|s| w.seg_text(s).replace(PAD, "·"))
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
    fn a_pad_is_a_column_in_a_hard_break() {
        assert_eq!(lines("`abcdefgh`", 4), ["·abc", "defg", "h·"]);
        assert_eq!(lines("`日本語`", 4), ["·日", "本語", "·"]);
    }

    /// A theme's glyph: its spaces are ada's own padding and hold, a tab is a space and breaks.
    #[test]
    fn a_glyphs_spaces_hold_and_its_tabs_break() {
        assert_eq!(lines_with("aaaa", 5, Some(" x")), ["aaaa·", "x"]);
        assert_eq!(lines_with("aaaa", 5, Some("\tx")), ["aaaa", "x"]);
    }

    /// The source's NBSPs stay what they are, and set [`Wrapped::nbsp`]; ada's pads are [`PAD`]s.
    #[test]
    fn the_sources_nbsps_are_not_pads() {
        let doc = parse("a\u{a0}b `c`");
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
            80,
            &doc,
            &glyphs,
            None,
        );
        assert_eq!(w.buf, "a\u{a0}b \u{1f}c\u{1f}");
        assert!(w.nbsp && w.pads);
        assert_eq!(unit_text(&inl.runs, &doc, &glyphs), "a\u{a0}b c");
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
    fn push_clean_agrees_with_clean_char() {
        for c in (0..0x800).filter_map(char::from_u32) {
            let s = format!("a{c}b");
            let (mut kept, mut folded) = (String::new(), String::new());
            assert_eq!(push_clean(&mut kept, &s, false), c == NBSP, "{c:?}");
            assert_eq!(kept, s.chars().map(clean_char).collect::<String>(), "{c:?}");
            push_clean(&mut folded, &s, true);
            assert_eq!(folded, kept.replace('\n', " "), "{c:?}");
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
