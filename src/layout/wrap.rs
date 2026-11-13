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

use crate::doc::{Doc, InlineStyle, LineRange, LinkId, Run, RunKind, TextRange, UnitId};
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
    /// Every byte is ASCII, so a slice of it is as wide as it is long (LAY-006: in the padding
    /// after `width`).
    pub ascii: bool,
    pub piece: u32,
}

const _: () = assert!(size_of::<Seg>() == 16);

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
    /// The pieces before this one belong to another cell: none grows ([`end_piece`]).
    sealed: u32,
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
        self.sealed = 0;
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

/// The 64-byte blocks of a document's [`Doc::text`] that hold a byte [`push_clean`] looks at when
/// it folds, a bit a block: a run in none is copied unscanned (LAY-006: `push_clean`, the scan of
/// every run and its copy, was 9–17% of layout). Width-independent, so a
/// [`Measure`](super::Measure) holds it.
#[derive(Debug, Default)]
pub struct Marks {
    bits: Vec<u64>,
}

impl Marks {
    pub fn new(doc: &Doc) -> Self {
        let (blocks, rest) = doc.text.as_bytes().as_chunks::<64>();
        let mut bits = vec![0u64; (blocks.len() + 1).div_ceil(64)];
        let marked = (blocks.iter().map(|b| any_suspect(b))).chain([any_suspect(rest)]);
        for (i, marked) in marked.enumerate() {
            bits[i / 64] |= u64::from(marked) << (i % 64);
        }
        Marks { bits }
    }

    /// Whether bytes `start..end` of the text may hold one: none past the text's end.
    fn any(&self, TextRange { start, end }: TextRange) -> bool {
        if start >= end {
            return false;
        }
        let (first, last) = ((start / 64) as usize, ((end - 1) / 64) as usize);
        (first..=last).any(|b| {
            self.bits
                .get(b / 64)
                .is_some_and(|w| w >> (b % 64) & 1 != 0)
        })
    }
}

/// Whether one of `bytes` is one [`suspect`] looks for when it folds, every byte looked at so the
/// test vectorizes.
fn any_suspect(bytes: &[u8]) -> bool {
    bytes.iter().fold(false, |acc, &b| {
        acc | (b < 0x20) | (b == 0x7f) | (b == 0xc2)
    })
}

/// Wrap `runs` to `avail` columns, into `w` (empty, from [`Wrapped::new`] or
/// [`Wrapped::recycle`]). `key` is the base style of the text; `suffix` is glued to the last
/// word (a heading's trailing pad). `marks`, the document's, spare its runs a scan.
pub fn wrap(
    mut w: Wrapped,
    runs: &[Run],
    key: StyleKey,
    avail: u16,
    (doc, marks): (&Doc, Option<&Marks>),
    glyphs: &Glyphs,
    suffix: Option<&str>,
) -> Wrapped {
    flatten(runs, key, (doc, marks), glyphs, &mut w);
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

/// The widths `runs` can wrap to: `(min, whole, max)`. `min` is the widest unbreakable word,
/// `whole` the widest word between spaces (which may also break after `-` and `/`), and `max`
/// the widest line when nothing wraps. Agrees with [`wrap`]: at `avail >= min` no word is
/// hard-broken, at `avail >= whole` every word fits a line of its own unbroken, and at
/// `avail >= max` no line wraps.
#[cfg(test)]
pub fn measure(runs: &[Run], doc: &Doc, glyphs: &Glyphs) -> (u16, u16, u16) {
    let mut w = Wrapped::default();
    flatten(runs, StyleKey::Text, (doc, None), glyphs, &mut w);
    measure_buf(&w.buf)
}

/// [`measure`] of a flattened buffer.
fn measure_buf(buf: &str) -> (u16, u16, u16) {
    let (mut min, mut whole, mut max, mut line, mut glued) = (0u16, 0u16, 0u16, 0u16, 0u16);
    let (mut started, mut pending) = (false, false);
    for tok in Tokens::new(buf) {
        match tok {
            Tok::Space { .. } => (pending, glued) = (started, 0),
            Tok::Newline => (line, glued, started, pending) = (0, 0, false, false),
            Tok::Word { width, .. } => {
                line = line
                    .saturating_add(u16::from(pending))
                    .saturating_add(width);
                glued = glued.saturating_add(width);
                (started, pending) = (true, false);
                min = min.max(width);
                whole = whole.max(glued);
                max = max.max(line);
            }
        }
    }
    (min, whole, max)
}

/// Cells flattened once, into one buffer, for both their measure and their wraps (LAY-003: a
/// table's cells were flattened to be measured, then again to be wrapped, each half of its
/// layout).
#[derive(Debug, Default)]
pub(crate) struct Flat {
    all: Wrapped,
    cells: Vec<FlatCell>,
}

/// One cell of a [`Flat`]: its bytes and pieces there.
#[derive(Clone, Copy, Debug)]
struct FlatCell {
    start: u32,
    end: u32,
    first: u32,
    last: u32,
    unit: UnitId,
    pads: bool,
    nbsp: bool,
}

impl Flat {
    /// Empty again, keeping the buffers.
    pub fn clear(&mut self) {
        self.all.buf.clear();
        self.all.pieces.clear();
        self.cells.clear();
    }

    /// The number of cells.
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Flatten a cell of `runs`, the text of search unit `unit`, in base style `key`.
    pub fn push(
        &mut self,
        runs: &[Run],
        unit: UnitId,
        key: StyleKey,
        doc: (&Doc, Option<&Marks>),
        glyphs: &Glyphs,
    ) {
        let w = &mut self.all;
        let (start, first) = (w.buf.len() as u32, w.pieces.len() as u32);
        (w.pads, w.nbsp, w.unit_len, w.sealed) = (false, false, 0, first);
        flatten(runs, key, doc, glyphs, w);
        self.cells.push(FlatCell {
            start,
            end: w.buf.len() as u32,
            first,
            last: w.pieces.len() as u32,
            unit,
            pads: w.pads,
            nbsp: w.nbsp,
        });
    }

    /// [`measure`] of cell `i`.
    pub fn measure(&self, i: usize) -> (u16, u16, u16) {
        let c = self.cells[i];
        measure_buf(&self.all.buf[c.start as usize..c.end as usize])
    }

    /// The lines [`wrap`] lays cell `i` in at its `max` and wider: one for each hard break, and
    /// one more after the last when a word follows it (or there is none). Apart from
    /// [`Flat::measure`], whose loop every cell goes through.
    pub fn hard_lines(&self, i: usize) -> u16 {
        let c = self.cells[i];
        let buf = &self.all.buf.as_bytes()[c.start as usize..c.end as usize];
        let breaks = buf
            .iter()
            .filter(|&&b| CLASS[b as usize] == NEWLINE)
            .count();
        let last = buf
            .rsplit(|&b| CLASS[b as usize] == NEWLINE)
            .next()
            .unwrap_or(&[]);
        let word = last.iter().any(|&b| CLASS[b as usize] != SPACE);
        u16::try_from(breaks + usize::from(word || breaks == 0)).unwrap_or(u16::MAX)
    }

    /// Cell `i`'s length in bytes: never less than its width, as no character is wider than its
    /// UTF-8 bytes (a [`PAD`] is one byte and one column).
    pub fn bytes(&self, i: usize) -> usize {
        let c = self.cells[i];
        (c.end - c.start) as usize
    }

    /// The lines [`Flat::wrap`] lays cell `i` in at `avail` columns, counted in place: no copy of
    /// its bytes and pieces, no segs (LAY-005: the table probe's wraps).
    pub fn lines(&self, i: usize, avail: u16) -> usize {
        let c = self.cells[i];
        let buf = &self.all.buf[c.start as usize..c.end as usize];
        let pieces = &self.all.pieces[c.first as usize..c.last as usize];
        count(buf, pieces, c.start, avail.max(1))
    }

    /// [`wrap`] of cell `i` to `avail` columns, into `into`'s buffers.
    pub fn wrap(&self, i: usize, into: Wrapped, avail: u16) -> Wrapped {
        let c = self.cells[i];
        let mut w = into.recycle(Some(c.unit));
        w.buf
            .push_str(&self.all.buf[c.start as usize..c.end as usize]);
        let pieces = &self.all.pieces[c.first as usize..c.last as usize];
        w.pieces.extend(pieces.iter().map(|p| Piece {
            start: p.start - c.start,
            end: p.end - c.start,
            body_start: p.body_start - c.start,
            body_end: p.body_end - c.start,
            ..*p
        }));
        (w.pads, w.nbsp) = (c.pads, c.nbsp);
        pack(&mut w, avail.max(1));
        w
    }
}

/// The text of a search unit made of `runs`: the bodies [`wrap`] gives them, in order.
pub fn unit_text(runs: &[Run], doc: &Doc, glyphs: &Glyphs) -> String {
    let mut w = Wrapped::default();
    flatten(runs, StyleKey::Text, (doc, None), glyphs, &mut w);
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

/// [`push_clean`] of a run's text, folded, unscanned when `marks` has none in its range.
fn push_run(buf: &mut String, doc: &Doc, run: &Run, marks: Option<&Marks>) -> bool {
    let text = doc.run_text(run);
    match marks {
        Some(m) if !m.any(run.text) => {
            buf.push_str(text);
            false
        }
        _ => push_clean(buf, text, true),
    }
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
    // a chunk at a time, not stopping inside one, so the test vectorizes (LAY-01); chunks of a
    // fixed length, so it does (LAY-002)
    let (chunks, rest) = s.as_bytes().as_chunks::<32>();
    chunks
        .iter()
        .any(|c| c.iter().fold(false, |acc, &b| acc | hit(b)))
        || rest.iter().any(|&b| hit(b))
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
    // most pieces are a body alone (LAY-006: two calls for nothing were 6% of emphasis' layout)
    let mut pads = !pre.is_empty() && push_deco(&mut w.buf, pre);
    let body_start = w.buf.len() as u32;
    w.nbsp |= push_clean(&mut w.buf, body, fold);
    let body_end = w.buf.len() as u32;
    pads |= !post.is_empty() && push_deco(&mut w.buf, post);
    let look = (key, inline, link);
    end_piece(w, [start, body_start, body_end], pads, look, src_line);
}

/// [`push_parts`] of inline code, its body folded between `pad`s already as the buffer holds
/// them, [`PAD`]s (LAY-006: an NBSP pad took `push_deco`'s character loop twice a span).
fn push_code(
    w: &mut Wrapped,
    pad: &str,
    (doc, run, marks): (&Doc, &Run, Option<&Marks>),
    look: Look,
) {
    debug_assert!(pad.bytes().all(|b| b == PAD as u8));
    let start = w.buf.len() as u32;
    w.buf.push_str(pad);
    let body_start = w.buf.len() as u32;
    w.nbsp |= push_run(&mut w.buf, doc, run, marks);
    let body_end = w.buf.len() as u32;
    w.buf.push_str(pad);
    let pads = !pad.is_empty();
    end_piece(w, [start, body_start, body_end], pads, look, run.src_line);
}

/// [`push_parts`] of a run of text, its body alone.
fn push_text(w: &mut Wrapped, (doc, run, marks): (&Doc, &Run, Option<&Marks>), look: Look) {
    let start = w.buf.len() as u32;
    w.nbsp |= push_run(&mut w.buf, doc, run, marks);
    let end = w.buf.len() as u32;
    end_piece(w, [start, start, end], false, look, run.src_line);
}

/// The piece from `start` to the buffer's end, its body `body_start..body_end`. A body alone that
/// follows a piece of the same look and source line ending in its body grows that piece instead
/// (LAY-006: a soft break was a piece of its own, a seg and a slow `Packer::add` on every row).
#[inline(always)]
fn end_piece(
    w: &mut Wrapped,
    [start, body_start, body_end]: [u32; 3],
    pads: bool,
    (key, inline, link): Look,
    src_line: u32,
) {
    let end = w.buf.len() as u32;
    if start == end {
        return;
    }
    if body_start == start
        && body_end == end
        && w.pieces.len() as u32 > w.sealed
        && let Some(last) = w.pieces.last_mut()
        && (last.end, last.body_end) == (start, start)
        && (last.key, last.inline, last.link, last.src_line) == (key, inline, link, src_line)
    {
        debug_assert_eq!(
            last.unit_off + (last.body_end - last.body_start),
            w.unit_len
        );
        (last.end, last.body_end) = (end, end);
        w.unit_len += end - start;
        return;
    }
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

/// A piece's style: key, inline markup, link.
type Look = (StyleKey, InlineStyle, Option<LinkId>);

fn flatten(
    runs: &[Run],
    key: StyleKey,
    (doc, marks): (&Doc, Option<&Marks>),
    glyphs: &Glyphs,
    w: &mut Wrapped,
) {
    // the pad as the buffer holds it: `push_deco` would make each NBSP a PAD
    let mut code_pad: Option<String> = None;
    for run in runs {
        let line = run.src_line;
        let look = (key, run.style, run.link);
        let text = doc.run_text(run);
        match run.kind {
            RunKind::Text if run.style.contains(InlineStyle::CODE) => {
                let pad = code_pad.get_or_insert_with(|| {
                    std::iter::repeat_n(PAD, glyphs.code_pad as usize).collect()
                });
                push_code(w, pad, (doc, run, marks), look);
            }
            RunKind::Text => push_text(w, (doc, run, marks), look),
            RunKind::SoftBreak => push_parts(w, ["", " ", ""], look, line, false),
            RunKind::HardBreak => push_parts(w, ["", "\n", ""], look, line, false),
            RunKind::InlineImage(link) => {
                let icon: String = glyphs
                    .image
                    .chars()
                    .map(|c| if c == ' ' { NBSP } else { c })
                    .collect();
                let icon = if text.is_empty() {
                    icon.trim_end_matches(NBSP)
                } else {
                    &icon
                };
                let look = (StyleKey::Image, run.style, Some(link));
                push_parts(w, [icon, text, ""], look, line, true);
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
    /// `ascii`: every byte is, so `width` is its length.
    Word {
        start: u32,
        end: u32,
        width: u16,
        ascii: bool,
    },
    Space {
        start: u32,
    },
    Newline,
}

/// How [`Tokens`] sees a byte: a word's byte unless one of these.
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

/// The bytes of `v`, eight of the buffer's in little-endian order, that [`word`]'s loop
/// stops at: a space, a newline, a `-` or `/`, and when `ascii` a byte past ASCII. Each one's top
/// bit is set, so the lowest is the first in the buffer. A byte matched for zero borrows into the
/// bytes after it, so a byte past the first may be set when it shouldn't; never one before it.
const fn stops(v: u64, ascii: bool) -> u64 {
    const LO: u64 = u64::from_le_bytes([0x01; 8]);
    const HI: u64 = u64::from_le_bytes([0x80; 8]);
    // the bytes of `x` that are zero
    const fn zero(x: u64) -> u64 {
        x.wrapping_sub(LO) & !x & HI
    }
    let delims = zero(v ^ (LO * b' ' as u64))
        | zero(v ^ (LO * b'\n' as u64))
        // `-` is 0x2d and `/` 0x2f: both 0x2f with bit 1 set
        | zero((v | (LO * 0x02)) ^ (LO * b'/' as u64));
    if ascii { delims | (v & HI) } else { delims }
}

/// `buf`'s tokens, in order: no vector of them is built, a word's bytes are read in a loop of
/// their own (LAY-01), and `next` inlines into its callers' loops (LAY-006: `pack`'s callback
/// was a call a token, 21% of prose's layout in itself).
struct Tokens<'a> {
    buf: &'a str,
    i: usize,
    /// The byte before `i` was a space: a run of spaces is one.
    spaced: bool,
}

impl<'a> Tokens<'a> {
    fn new(buf: &'a str) -> Self {
        Tokens {
            buf,
            i: 0,
            spaced: false,
        }
    }
}

impl Iterator for Tokens<'_> {
    type Item = Tok;

    #[inline(always)]
    fn next(&mut self) -> Option<Tok> {
        // Every delimiter is ASCII, and no byte of a multi-byte character is, so bytes will do.
        let bytes = self.buf.as_bytes();
        loop {
            let &b = bytes.get(self.i)?;
            match CLASS[b as usize] {
                SPACE => {
                    self.i += 1;
                    if !self.spaced {
                        self.spaced = true;
                        return Some(Tok::Space {
                            start: self.i as u32 - 1,
                        });
                    }
                }
                NEWLINE => {
                    self.i += 1;
                    self.spaced = false;
                    return Some(Tok::Newline);
                }
                _ => {
                    let (start, end, wide) = word(bytes, self.i);
                    self.i = end;
                    self.spaced = false;
                    // Without a byte past ASCII a word's width is its length, as in `text_width`
                    // (the buffer holds no `\r`: `clean_char`).
                    let width = match wide {
                        true => text_width(&self.buf[start..end]),
                        false => end - start,
                    };
                    return Some(Tok::Word {
                        start: start as u32,
                        end: end as u32,
                        width: width.min(u16::MAX as usize) as u16,
                        ascii: !wide,
                    });
                }
            }
        }
    }
}

/// The word that starts at `bytes[start]`, a word's byte: where it ends, and whether it has a
/// byte past ASCII.
#[inline(always)]
fn word(bytes: &[u8], start: usize) -> (usize, usize, bool) {
    let mut i = start;
    let mut wide = false;
    while i < bytes.len() {
        // the bytes up to the next one the loop below must look at, eight at a time (LAY-002):
        // most words end inside their first eight
        while let Some(chunk) = bytes.get(i..i + 8) {
            let v = u64::from_le_bytes(chunk.try_into().expect("eight bytes"));
            let stops = stops(v, !wide);
            if stops != 0 {
                i += (stops.trailing_zeros() / 8) as usize;
                break;
            }
            i += 8;
        }
        let Some(&b) = bytes.get(i) else { break };
        let class = CLASS[b as usize];
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
    (start, i, wide)
}

/// The line [`pack`] fills. Its last seg is held here, not yet in `segs`, so the words that grow
/// it are a few registers' work (LAY-006: they were a call and a `Vec`'s last element each).
struct Packer<'w> {
    pieces: &'w [Piece],
    buf: &'w str,
    cursor: usize,
    /// The end of `pieces[cursor]`.
    piece_end: u32,
    /// The line's last seg, when `open`.
    seg: Seg,
    /// Whether the line holds anything: `seg` with it.
    open: bool,
    /// The line's width and source lines so far, and where its segs start in `segs`.
    width: u16,
    src: LineRange,
    first: u32,
    segs: &'w mut Vec<Seg>,
    lines: &'w mut Vec<Line>,
}

impl<'w> Packer<'w> {
    fn new(w: &'w mut Wrapped) -> Self {
        Packer {
            piece_end: w.pieces.first().map_or(0, |p| p.end),
            pieces: &w.pieces,
            buf: &w.buf,
            cursor: 0,
            seg: Seg {
                start: 0,
                end: 0,
                width: 0,
                ascii: true,
                piece: 0,
            },
            open: false,
            width: 0,
            src: LineRange::NONE,
            first: w.segs.len() as u32,
            segs: &mut w.segs,
            lines: &mut w.lines,
        }
    }

    /// Append `[start, end)`, `width` columns wide and all ASCII when `ascii`, to the line, split
    /// at piece boundaries.
    #[inline(always)]
    fn add(&mut self, start: u32, end: u32, width: u16, ascii: bool) {
        // right after the line's last seg and inside its piece: that seg grows, and the line's
        // source lines already hold the piece's (LAY-002)
        if self.open && self.seg.end == start && end <= self.piece_end {
            debug_assert_eq!(self.seg.piece as usize, self.cursor);
            self.seg.end = end;
            self.seg.width = self.seg.width.saturating_add(width);
            self.seg.ascii &= ascii;
            self.width = self.width.saturating_add(width);
        } else {
            self.add_split(start, end, width, ascii);
        }
    }

    /// [`Packer::add`] past the end of the last seg's piece, or after a gap.
    #[inline(never)]
    fn add_split(&mut self, mut start: u32, end: u32, width: u16, ascii: bool) {
        let mut whole = true;
        while start < end {
            while self.cursor + 1 < self.pieces.len() && self.pieces[self.cursor].end <= start {
                self.cursor += 1;
            }
            let piece = &self.pieces[self.cursor];
            self.piece_end = piece.end;
            let stop = end.min(piece.end).max(start + 1);
            // a part of ASCII is as wide as it is long
            let width = match (whole && stop == end, ascii) {
                (true, _) => width,
                (false, true) => (stop - start) as u16,
                (false, false) => text_width(&self.buf[start as usize..stop as usize]) as u16,
            };
            whole = false;
            // a line's words of one piece are one seg (LAY-01)
            let piece_at = self.cursor as u32;
            if self.open && self.seg.piece == piece_at && self.seg.end == start {
                self.seg.end = stop;
                self.seg.width = self.seg.width.saturating_add(width);
                self.seg.ascii &= ascii;
            } else {
                if self.open {
                    self.segs.push(self.seg);
                }
                self.seg = Seg {
                    start,
                    end: stop,
                    width,
                    ascii,
                    piece: piece_at,
                };
                self.open = true;
            }
            self.width = self.width.saturating_add(width);
            self.src = self.src.union(LineRange::line(piece.src_line));
            start = stop;
        }
    }

    /// End the line, empty or not.
    fn flush(&mut self) {
        if self.open {
            self.segs.push(self.seg);
        }
        let end = self.segs.len() as u32;
        self.lines.push(Line {
            segs: self.first..end,
            width: self.width,
            src: self.src,
        });
        (self.open, self.width, self.src, self.first) = (false, 0, LineRange::NONE, end);
    }
}

fn pack(w: &mut Wrapped, avail: u16) {
    let mut p = Packer::new(w);
    // where the run of spaces before the next word starts, when it follows a word on the line
    let mut pending: Option<u32> = None;
    for tok in Tokens::new(p.buf) {
        match tok {
            Tok::Space { start } => {
                if p.open {
                    pending = Some(start);
                }
            }
            Tok::Newline => {
                p.flush();
                pending = None;
            }
            Tok::Word {
                start,
                end,
                width,
                ascii,
            } => {
                let space = u16::from(pending.is_some());
                if p.width.saturating_add(space).saturating_add(width) <= avail {
                    match pending {
                        // a space right before the word goes with it (LAY-002)
                        Some(s) if s + 1 == start => {
                            p.add(s, end, width.saturating_add(1), ascii);
                        }
                        Some(s) => {
                            p.add(s, s + 1, 1, true);
                            p.add(start, end, width, ascii);
                        }
                        None => p.add(start, end, width, ascii),
                    }
                } else if width <= avail {
                    p.flush();
                    p.add(start, end, width, ascii);
                } else {
                    hard_break(&mut p, start, end, avail);
                }
                pending = None;
            }
        }
    }
    if p.open || p.lines.is_empty() {
        p.flush();
    }
}

/// Bytes `start..end` of the buffer, a word longer than a whole line, broken across lines from a
/// fresh one.
#[inline(never)]
fn hard_break(p: &mut Packer<'_>, start: u32, end: u32, avail: u16) {
    if p.open {
        p.flush();
    }
    let buf = p.buf;
    let mut at = start;
    while at < end {
        let rest = &buf[at as usize..end as usize];
        let len = hard_cut(rest, avail);
        for (i, c) in rest[..len].char_indices() {
            let s = at + i as u32;
            let e = s + c.len_utf8() as u32;
            let width = text_width(&buf[s as usize..e as usize]) as u16;
            p.add(s, e, width, c.is_ascii());
        }
        at += len as u32;
        if at < end {
            p.flush();
        }
    }
}

/// The lines [`pack`] lays `buf` in at `avail` columns, counted: the same greedy pass, keeping
/// only the line's width and whether it holds anything (LAY-005). `pieces` are `buf`'s, their
/// offsets `base` past its start, for the widths [`Packer::add`] sums a piece at a time.
fn count(buf: &str, pieces: &[Piece], base: u32, avail: u16) -> usize {
    let (mut lines, mut width, mut open) = (0usize, 0u16, false);
    let mut pending: Option<u32> = None;
    let mut cursor = 0;
    // `start..end`'s columns as `Packer::add` counts them: `w` inside one piece, and otherwise
    // each piece's part measured on its own, the same sum unless a part is past ASCII
    let mut span = |start: u32, end: u32, w: u16, ascii: bool| -> u16 {
        let piece_end = |k: usize| pieces.get(k).map_or(u32::MAX, |p| p.end - base);
        while cursor + 1 < pieces.len() && piece_end(cursor) <= start {
            cursor += 1;
        }
        if end <= piece_end(cursor) || ascii {
            return w;
        }
        let (mut at, mut sum, mut k) = (start, 0u16, cursor);
        while at < end {
            while k + 1 < pieces.len() && piece_end(k) <= at {
                k += 1;
            }
            let stop = end.min(piece_end(k)).max(at + 1);
            sum = sum.saturating_add(text_width(&buf[at as usize..stop as usize]) as u16);
            at = stop;
        }
        cursor = k;
        sum
    };
    for tok in Tokens::new(buf) {
        match tok {
            Tok::Space { start } => {
                if open {
                    pending = Some(start);
                }
            }
            Tok::Newline => {
                (lines, width, open, pending) = (lines + 1, 0, false, None);
            }
            Tok::Word {
                start,
                end,
                width: w,
                ascii,
            } => {
                let space = u16::from(pending.is_some());
                if width.saturating_add(space).saturating_add(w) <= avail {
                    let add = match pending {
                        Some(s) if s + 1 == start => span(s, end, w.saturating_add(1), ascii),
                        Some(s) => {
                            let space = span(s, s + 1, 1, true);
                            space.saturating_add(span(start, end, w, ascii))
                        }
                        None => span(start, end, w, ascii),
                    };
                    width = width.saturating_add(add);
                } else if w <= avail {
                    lines += 1;
                    width = span(start, end, w, ascii);
                } else {
                    // hard-broken from a fresh line, as `pack` does
                    lines += usize::from(open);
                    let mut at = start;
                    while at < end {
                        let rest = &buf[at as usize..end as usize];
                        let len = hard_cut(rest, avail);
                        width = (rest[..len].char_indices())
                            .map(|(i, c)| text_width(&rest[i..i + c.len_utf8()]) as u16)
                            .fold(0, u16::saturating_add);
                        at += len as u32;
                        lines += usize::from(at < end);
                    }
                }
                (open, pending) = (true, None);
            }
        }
    }
    lines + usize::from(open || lines == 0)
}

/// The bytes of `word` that go on a line of `avail` columns: all of it when it fits, otherwise
/// as many characters as fit (one at least), cut back to the last point that reads as a break
/// when that keeps at least half the line (FEAT-015): after a `.`, `_` or `:`, and before a
/// capital that follows a lowercase letter or a digit. Scans no further than the line, so a
/// long word is cut in linear time.
fn hard_cut(word: &str, avail: u16) -> usize {
    let (mut used, mut soft, mut prev) = (0u16, None, None);
    for (i, c) in word.char_indices() {
        // `text_width`, not `c.width()`: a pad is a column wide
        let cw = text_width(&word[i..i + c.len_utf8()]) as u16;
        if used.saturating_add(cw) > avail && i > 0 {
            return soft.unwrap_or(i);
        }
        let breaks = prev.is_some_and(|p: char| {
            (matches!(p, '.' | '_' | ':') && !matches!(c, '.' | '_' | ':'))
                || ((p.is_lowercase() || p.is_ascii_digit()) && c.is_uppercase())
        });
        if breaks && used >= avail.div_ceil(2) {
            soft = Some(i);
        }
        used = used.saturating_add(cw);
        prev = Some(c);
    }
    word.len()
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
            doc.runs(inl),
            StyleKey::Text,
            avail,
            (&doc, None),
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

    /// FEAT-015: a hard break goes back to a `.`, `_`, `:` or a camelCase hump when that keeps at
    /// least half the line.
    #[test]
    fn hard_breaks_cut_where_a_word_reads_as_parts() {
        assert_eq!(
            lines("com.example.billing.InvoiceReconciliationService", 26),
            ["com.example.billing.", "InvoiceReconciliation", "Service"]
        );
        assert_eq!(
            lines("AbstractSingletonProxyFactoryBean", 26),
            ["AbstractSingletonProxy", "FactoryBean"]
        );
        assert_eq!(
            lines("MAX_POOL_SIZE_LIMIT", 12),
            ["MAX_POOL_", "SIZE_LIMIT"]
        );
        assert_eq!(
            lines("std::collections::HashMap", 20),
            ["std::collections::", "HashMap"]
        );
        assert_eq!(lines("v2Beta", 4), ["v2", "Beta"]);
        // a run of separators stays together
        assert_eq!(lines("aaaa..bbbb", 7), ["aaaa..", "bbbb"]);
        // the last point is under half the line: cut where it fills up
        assert_eq!(lines("a.bcdefghij", 6), ["a.bcde", "fghij"]);
        assert_eq!(lines("aBcdefghij", 6), ["aBcdef", "ghij"]);
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
            doc.runs(inl),
            StyleKey::Text,
            80,
            (&doc, None),
            &glyphs,
            None,
        );
        assert_eq!(w.buf, "a\u{a0}b \u{1f}c\u{1f}");
        assert!(w.nbsp && w.pads);
        assert_eq!(unit_text(doc.runs(inl), &doc, &glyphs), "a\u{a0}b c");
    }

    /// [`Tokens`]' tokens, as `(kind, start, end, width)`.
    fn tokens(buf: &str) -> Vec<(u8, u32, u32, u16)> {
        Tokens::new(buf)
            .map(|t| match t {
                Tok::Word {
                    start, end, width, ..
                } => (0, start, end, width),
                Tok::Space { start } => (1, start, start + 1, 0),
                Tok::Newline => (2, 0, 0, 0),
            })
            .collect()
    }

    /// The tokens of [`Tokens`] before LAY-002's eight bytes at a time, a byte at a time.
    fn tokens_bytewise(buf: &str) -> Vec<(u8, u32, u32, u16)> {
        let (bytes, mut out, mut spaced, mut i) = (buf.as_bytes(), Vec::new(), false, 0);
        while i < bytes.len() {
            match bytes[i] {
                b' ' => {
                    if !spaced {
                        out.push((1, i as u32, i as u32 + 1, 0));
                    }
                    (spaced, i) = (true, i + 1);
                }
                b'\n' => {
                    out.push((2, 0, 0, 0));
                    (spaced, i) = (false, i + 1);
                }
                _ => {
                    let start = i;
                    while i < bytes.len() && !matches!(bytes[i], b' ' | b'\n') {
                        i += 1;
                        let next = bytes.get(i).is_some_and(|&n| n != b' ' && n != b'\n');
                        if matches!(bytes[i - 1], b'-' | b'/') && i - 1 > start && next {
                            break;
                        }
                    }
                    let width = text_width(&buf[start..i]) as u16;
                    out.push((0, start as u32, i as u32, width));
                    spaced = false;
                }
            }
        }
        out
    }

    /// Every delimiter, and a byte past ASCII, at each offset of words around eight bytes long:
    /// the eight-byte scan stops where the byte loop did.
    #[test]
    fn tokens_eight_at_a_time_agree_with_a_byte_at_a_time() {
        let fills = ["a", "\u{1f}", ".", "\u{2e}\u{2c}", "é", "日", "0"];
        let marks = [
            " ", "\n", "-", "/", "--", "- ", "/\n", "é", "日本", "\u{a0}", ",", "+",
        ];
        for fill in fills {
            for len in 0..20 {
                for at in 0..=len {
                    for mark in marks {
                        let mut s = fill.repeat(len);
                        let cut = s.char_indices().nth(at).map_or(s.len(), |(i, _)| i);
                        s.insert_str(cut, mark);
                        for t in [
                            s.clone(),
                            format!("-{s}"),
                            format!("{s}-x"),
                            format!("x {s} y"),
                        ] {
                            assert_eq!(tokens(&t), tokens_bytewise(&t), "{t:?}");
                        }
                    }
                }
            }
        }
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
            // `Flat::bytes` bounds a cell's width (LAY-005)
            assert!(text_width(s) <= s.len(), "{s:?}");
        }
        for c in (0..=0x10_ffffu32).filter_map(char::from_u32) {
            let s = c.to_string();
            assert!(text_width(&s) <= s.len(), "{c:?}");
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

    /// [`suspect`] finds each byte it looks for, alone or among others, at every offset of runs
    /// up to past two chunks long, in their 32-byte chunks and their tails.
    #[test]
    fn suspect_finds_every_byte_at_every_offset() {
        let bad = [
            "\u{0}", "\u{1f}", "\t", "\r", "\u{7f}", "\u{a0}", "\u{85}", "©", "\n",
        ];
        let good = ["a", "é", "日", "\u{ff}", "\u{c0}", " ", "\u{2028}"];
        for len in 0..72 {
            for fill in good {
                let base = fill.repeat(len);
                assert!(!suspect(&base, false) && !suspect(&base, true), "{base:?}");
                for at in base.char_indices().map(|(i, _)| i).chain([base.len()]) {
                    for b in bad {
                        let mut s = base.clone();
                        s.insert_str(at, b);
                        // a control character, DEL, or one 0xc2 starts (U+0080 to U+00BF)
                        let want = |fold: bool| {
                            s.chars().any(|c| {
                                matches!(c as u32, 0x7f..=0xbf) || c < ' ' && (fold || c != '\n')
                            })
                        };
                        assert_eq!(suspect(&s, false), want(false), "{s:?}");
                        assert_eq!(suspect(&s, true), want(true), "{s:?} folded");
                    }
                }
            }
        }
    }

    /// [`Marks`] marks every run [`suspect`] would find something in, a byte to clean at each
    /// offset around its 64-byte blocks' edges past the first (a tab there is code), and a wrap
    /// with them is the wrap without.
    #[test]
    fn marks_cover_every_run_to_clean() {
        let glyphs = Glyphs {
            code_pad: 1,
            ..Glyphs::default()
        };
        for bad in ["\t", "\u{a0}", "\u{85}", "©", "\u{7f}"] {
            for at in (1..200).step_by(3) {
                let mut md = "word *em* `code` ".repeat(12);
                let cut = md.char_indices().nth(at).map_or(md.len(), |(i, _)| i);
                md.insert_str(cut, bad);
                let doc = parse(&md);
                let crate::doc::BlockKind::Paragraph(inl) = &doc.blocks[0].kind else {
                    panic!("not a paragraph")
                };
                let marks = Marks::new(&doc);
                for run in doc.runs(inl) {
                    let text = doc.run_text(run);
                    assert!(
                        marks.any(run.text) || !suspect(text, true),
                        "{text:?} in {md:?}"
                    );
                }
                let wrapped = |marks| {
                    let into = Wrapped::new(None);
                    let w = wrap(
                        into,
                        doc.runs(inl),
                        StyleKey::Text,
                        30,
                        (&doc, marks),
                        &glyphs,
                        None,
                    );
                    (
                        w.buf,
                        w.lines
                            .iter()
                            .map(|l| (l.segs.clone(), l.width))
                            .collect::<Vec<_>>(),
                    )
                };
                assert_eq!(wrapped(Some(&marks)), wrapped(None), "{md:?}");
            }
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
            doc.runs(inl),
            StyleKey::Text,
            12,
            (&doc, None),
            &Glyphs::default(),
            None,
        );
        let src: Vec<_> = w.lines.iter().map(|l| (l.src.start, l.src.end)).collect();
        assert_eq!(src, [(1, 1), (2, 2)]);
    }
}
