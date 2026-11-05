//! The line-format engine (X-020's second part, the parser audit's item 5): the frame under the
//! formats read a line at a time, INI's, Java properties' and `.env`'s. A line is a comment, a
//! section's header or an entry (a key, its separator and a value), told apart by the first
//! byte past its blanks on the format's table ([`Format::LEAD`], the prefix dispatch); a format
//! reads each with the [`Cursor`]'s primitives. What a line leaves open (a value run on by a
//! `\`, a string not closed) is one word of the [`State`], 0 for nothing, and a line after one
//! that isn't 0 is the format's to read whole ([`Format::resume`]).
//!
//! What stays a format's own is what its grammar's corners need, as each is exact against bat's
//! grammar on syntect: where a key ends, what a separator is, which quotes close.

use crate::prim::{find_any, stops};
use crate::{Out, State};

/// What a line's first byte past its blanks starts: an entry (any byte the table doesn't
/// name), a comment or a section's header.
pub const ENTRY: u8 = 0;
pub const COMMENT: u8 = 1;
pub const SECTION: u8 = 2;

/// A [`Format::LEAD`] table: each of `comments` starts a comment, each of `sections` a
/// section's header, any other byte an entry.
pub const fn leads(comments: &[u8], sections: &[u8]) -> [u8; 256] {
    let mut t = [ENTRY; 256];
    let mut i = 0;
    while i < comments.len() {
        t[comments[i] as usize] = COMMENT;
        i += 1;
    }
    let mut i = 0;
    while i < sections.len() {
        t[sections[i] as usize] = SECTION;
        i += 1;
    }
    t
}

/// A format read a line at a time.
pub trait Format {
    /// What each byte starts as a line's first past its blanks ([`leads`]).
    const LEAD: [u8; 256];
    /// A comment line's kinds: its text, and its mark (the lead byte).
    const COMMENT_TEXT: u16;
    const COMMENT_MARK: u16;
    /// Whether the blanks before a comment's mark are its text (else plain).
    const COMMENT_BLANKS: bool;

    /// Whether a line can leave something open; a format whose lines never do reads each line
    /// as [`fresh`], whatever the state.
    const OPENS: bool = true;

    /// A line after one that left `open` (not 0): the whole line, and what it leaves open.
    /// Never called where [`Format::OPENS`] is false.
    fn resume(c: &mut Cursor, open: u64) -> u64;

    /// A line of blanks alone.
    #[inline(always)]
    fn blank(_c: &mut Cursor) {}

    /// A section's header whose lead byte is at `p`, or `None` where the line isn't one and is
    /// read as an entry.
    #[inline(always)]
    fn section(_c: &mut Cursor, _p: usize) -> Option<u64> {
        None
    }

    /// An entry whose first byte past the blanks is at `p` (the cursor still at the line's
    /// start): what the line leaves open.
    fn entry(c: &mut Cursor, p: usize) -> u64;
}

/// Lex `b`, a line without its `\n`, as format `F`, from `state`.
#[inline(always)]
pub fn line<F: Format>(b: &[u8], state: &mut State, out: &mut Out) {
    let mut c = Cursor { b, out, at: 0 };
    let open = match state.word() {
        open if open != 0 && F::OPENS => F::resume(&mut c, open),
        _ => fresh::<F>(&mut c),
    };
    state.set_word(open);
}

/// A line after one that left nothing open: a blank line, or by its first byte past the
/// blanks, a comment, a section's header or an entry. What it leaves open.
#[inline(always)]
pub fn fresh<F: Format>(c: &mut Cursor) -> u64 {
    let b = c.b;
    let p = blanks(b, 0);
    if p == b.len() {
        F::blank(c);
        return 0;
    }
    match F::LEAD[usize::from(b[p])] {
        COMMENT => {
            c.comment::<F>(p);
            0
        }
        SECTION => match F::section(c, p) {
            Some(open) => open,
            None => F::entry(c, p),
        },
        _ => F::entry(c, p),
    }
}

/// A line's spans as a format reads it: where the spans so far end, and what goes after.
pub struct Cursor<'l, 'o, 's> {
    pub b: &'l [u8],
    out: &'o mut Out<'s>,
    /// Where the spans so far end.
    pub at: usize,
}

impl Cursor<'_, '_, '_> {
    /// The line's length.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.b.len()
    }

    /// The spans so far go on to `end` with `kind` (none where it's empty).
    #[inline(always)]
    pub fn put(&mut self, end: usize, kind: u16) {
        self.out.push(self.at, end, kind);
        self.at = end;
    }

    /// As [`Cursor::put`], joined to the span before where it's the same kind and touches.
    #[inline(always)]
    pub fn put_merged(&mut self, end: usize, kind: u16) {
        self.out.push_merged(self.at, end, kind);
        self.at = end;
    }

    /// The bytes to `to` are plain: no span.
    #[inline(always)]
    pub fn skip(&mut self, to: usize) {
        self.at = to;
    }

    /// The text up to `s` is `text` (plain where `text` is `None`), then `s..e` is `kind`,
    /// each joined to a touching span of its kind before it.
    #[inline(always)]
    pub fn tok(&mut self, text: Option<u16>, s: usize, e: usize, kind: u16) {
        self.text(text, s);
        self.out.push_merged(s, e, kind);
        self.at = e;
    }

    /// The text up to `to` is `text` (plain where `None`), joined to a touching span of its
    /// kind.
    #[inline(always)]
    pub fn text(&mut self, text: Option<u16>, to: usize) {
        if let Some(kind) = text
            && to > self.at
        {
            self.out.push_merged(self.at, to, kind);
        }
        self.at = to;
    }

    /// A comment from its mark at `p` to the line's end, the blanks before it its text where
    /// `F` says so.
    #[inline(always)]
    pub fn comment<F: Format>(&mut self, p: usize) {
        if F::COMMENT_BLANKS {
            self.put(p, F::COMMENT_TEXT);
        } else {
            self.skip(p);
        }
        self.put(p + 1, F::COMMENT_MARK);
        self.put(self.b.len(), F::COMMENT_TEXT);
    }
}

/// ASCII's `\s`, the blanks.
pub static BLANK: [bool; 256] = stops(b" \t\n\x0b\x0c\r");

/// Where the blanks ([`BLANK`]) at `p` in `b` end.
#[inline(always)]
pub fn blanks(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && BLANK[usize::from(b[p])] {
        p += 1;
    }
    p
}

/// Where the blanks that end `b[..end]` start, not before `from`.
#[inline(always)]
pub fn trim(b: &[u8], from: usize, mut end: usize) -> usize {
    while end > from && BLANK[usize::from(b[end - 1])] {
        end -= 1;
    }
    end
}

/// Where the first of `set` from `p` in `b` is, or the line's end: eight bytes a step.
#[inline(always)]
pub fn find<const N: usize>(b: &[u8], p: usize, set: [u8; N]) -> usize {
    find_any(b, p, set)
}

/// Where the first byte from `p` in `b` that `stop` takes is, or the line's end.
#[inline(always)]
pub fn find_by(b: &[u8], mut p: usize, stop: &[bool; 256]) -> usize {
    while p < b.len() && !stop[usize::from(b[p])] {
        p += 1;
    }
    p
}
