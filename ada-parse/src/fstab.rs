//! fstab, crypttab and mtab, in place of bat's `fstab`, on the line-format engine (X-034): a
//! line is a comment or an entry, its fields read in turn: the device, the mount point, the
//! file system, the options, the dump and pass numbers. A line doesn't depend on the ones
//! before it.
//!
//! Every context of bat's grammar ends with `.*(?m:$)`, which pops and takes the rest of the
//! line plain, and which matches wherever the scan is; so a field's pattern counts only where
//! it matches right at the scan's place, the first in the grammar's order that does, and
//! nothing past a field no pattern of its context takes is colored. A field's `\s*` takes the
//! blanks after it, its scope over them where the pattern has one; at a line's end it takes the
//! `\n`, and the next context pops there, so no line leaves anything open.
//!
//! Its readings are held by `tests/readings.rs`' snapshots (X-116).

use crate::emit::{Push, Runs, Sink};
use crate::line_format::{BLANK, Cursor, Format, blanks, leads};
use crate::{Out, State};

const COMMENT: u16 = 0;
const ACCESSOR: u16 = 1;
const DEVICE: u16 = 2;
const MOUNTPOINT: u16 = 3;
const INVALID: u16 = 4;
const FILESYSTEM: u16 = 5;
const OPTION: u16 = 6;
const OPTION_SEPARATOR: u16 = 7;
const OPTION_VALUE: u16 = 8;
const SEPARATOR: u16 = 9;
const NUMBER: u16 = 10;

/// Each kind's scopes under `source.fstab`, as bat's grammar gives them.
pub const KINDS: &[&str] = &[
    "comment.line",
    "punctuation.accessor",
    "keyword.other.device",
    "string.unquoted.mountpoint",
    "invalid.illegal",
    "variable.parameter.filesystem",
    "entity.name.option",
    "entity.other.separator",
    "variable.parameter",
    "punctuation.separator",
    "constant.numeric",
];

/// Lex `b`, a line without its `\n`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    crate::line_format::line::<Fstab, _>(b, state, out, &mut Push);
}

/// [`line`]'s lexemes written to `k`, a run sink (X-069).
/// Inline in a block's loop, [`crate::lex_runs`]' line too (X-050).
pub(crate) struct RunLine;

impl crate::LineScan for RunLine {
    #[inline(always)]
    fn scan(b: &[u8], state: &mut State, out: &mut Out, k: &mut Runs) {
        crate::line_format::line::<Fstab, _>(b, state, out, k);
    }
}

struct Fstab;

impl Format for Fstab {
    // `\s*#.*(?m:$)`, a span over the blanks, the `#` and the text alike: read by `entry`
    const LEAD: [u8; 256] = leads(b"", b"");
    const COMMENT_TEXT: u16 = COMMENT;
    const COMMENT_MARK: u16 = COMMENT;
    const COMMENT_BLANKS: bool = true;
    const OPENS: bool = false;

    fn resume<K: Sink>(_: &mut Cursor<K>, _: u64) -> u64 {
        unreachable!("fstab's lines leave nothing open")
    }

    /// The fields from the device at `q`, the first byte past the line's blanks.
    #[inline(always)]
    fn entry<K: Sink>(c: &mut Cursor<K>, q: usize) -> u64 {
        if comment(c, 0) {
            return 0;
        }
        let p = device(c, q);
        let p = field(c, p, |c, p, e| {
            let kind = if c.b[p] == b'/' { MOUNTPOINT } else { INVALID };
            put(c, p, e, kind);
            e
        });
        let p = field(c, p, |c, p, e| {
            put(c, p, e, FILESYSTEM);
            e
        });
        if let Some(p) = options(c, p) {
            let p = field(c, p, number);
            let p = field(c, p, number);
            // `expected_eol`: a comment, or the rest plain
            comment(c, p);
        }
        0
    }
}

/// `s..e` is `kind`, the text before it plain.
#[inline(always)]
fn put<K: Sink>(c: &mut Cursor<K>, s: usize, e: usize, kind: u16) {
    c.skip(s);
    c.put(e, kind);
}

/// `\s*#.*(?m:$)` at `p`: the comment, blanks and all, to the line's end, where there is one.
#[inline(always)]
fn comment<K: Sink>(c: &mut Cursor<K>, p: usize) -> bool {
    let q = blanks(c.b, p);
    if q < c.len() && c.b[q] == b'#' {
        put(c, p, c.len(), COMMENT);
        return true;
    }
    false
}

/// Where the run of bytes that aren't blanks from `p` ends.
#[inline(always)]
fn word_end(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && !BLANK[usize::from(b[p])] {
        p += 1;
    }
    p
}

/// A field at `p`, past its context's comment: `read` colors the field `p..e`, the run of
/// bytes that aren't blanks and the blanks after it, and gives where the next field starts.
/// The line's end where nothing goes on.
#[inline(always)]
fn field<K: Sink>(
    c: &mut Cursor<K>,
    p: usize,
    read: impl Fn(&mut Cursor<K>, usize, usize) -> usize,
) -> usize {
    if p >= c.len() || comment(c, p) {
        return c.len();
    }
    let e = blanks(c.b, word_end(c.b, p));
    read(c, p, e)
}

/// `fstab_dump`'s and `fstab_pass`'s `\s*[012]\s*`, else `\s*[^\s]+\s*` invalid: a number
/// is the field's first byte alone (and the blanks after it), the rest the next field's.
#[inline(always)]
fn number<K: Sink>(c: &mut Cursor<K>, p: usize, e: usize) -> usize {
    if matches!(c.b[p], b'0'..=b'2') {
        let e = blanks(c.b, p + 1);
        put(c, p, e, NUMBER);
        e
    } else {
        put(c, p, e, INVALID);
        e
    }
}

/// `fstab_device` at `q`, the line's first byte past its blanks, in the grammar's order: where
/// the mount point starts.
#[inline(always)]
fn device<K: Sink>(c: &mut Cursor<K>, q: usize) -> usize {
    let b = c.b;
    let te = word_end(b, q);
    let next = blanks(b, te);
    let token = &b[q..te];
    // `\s*([^=\s]+=)([^\s]+)\s*`: an `=` with a byte before and after it
    let equals = token.iter().position(|&x| x == b'=');
    let accessor = if let Some(i) = equals.filter(|&i| i > 0 && i + 1 < token.len()) {
        Some((q, q + i + 1))
    // `\s*(//|\\\\)([^\s]+)\s*`
    } else if token.len() > 2 && (token.starts_with(b"//") || token.starts_with(b"\\\\")) {
        Some((q, q + 2))
    } else if let Some(i) = token
        .get(1..token.len().saturating_sub(1))
        .and_then(|inner| inner.iter().rposition(|&x| x == b':'))
    {
        // `\s*([^\s]+)(:)([^\s]+)\s*`: the last `:` with a byte before and after it
        let i = q + 1 + i;
        put(c, q, i, DEVICE);
        Some((i, i + 1))
    } else if !matches!(b[q], b'/' | b'\\') {
        // `\s*[^\s/\\]+\s*`: to the first `/` or `\`, its blanks and all
        let e = token
            .iter()
            .position(|&x| x == b'/' || x == b'\\')
            .map_or(next, |i| q + i);
        c.put(e, DEVICE);
        return e;
    } else if token.len() > 5 && token.starts_with(b"/dev/") {
        // `\s*(/dev/)([^\s]+)\s*`
        Some((q, q + 5))
    } else {
        // `\s*([^\s/]*/)*([^\s]+)\s*`: as many segments as leave a byte after them, the
        // last the accessor
        let last = match token.iter().rposition(|&x| x == b'/') {
            Some(s) if s + 1 < token.len() => Some(s),
            Some(s) => token[..s].iter().rposition(|&x| x == b'/'),
            None => None,
        };
        last.map(|s| {
            let from = token[..s]
                .iter()
                .rposition(|&x| x == b'/')
                .map_or(0, |i| i + 1);
            (q + from, q + s + 1)
        })
    };
    if let Some((s, e)) = accessor {
        put(c, s, e, ACCESSOR);
    }
    put(c, c.at.max(q), te, DEVICE);
    c.skip(next);
    next
}

/// `fstab_options` from `p`: the options; `None` where a comment or the line's end ends them,
/// else where the dump number starts.
#[inline(always)]
fn options<K: Sink>(c: &mut Cursor<K>, mut p: usize) -> Option<usize> {
    let b = c.b;
    let n = b.len();
    let ends = |x: u8| BLANK[usize::from(x)] || x == b',';
    loop {
        if p >= n || comment(c, p) {
            return None;
        }
        let x = b[p];
        if x == b',' {
            put(c, p, p + 1, SEPARATOR);
            p += 1;
        } else if BLANK[usize::from(x)] {
            // `\s+`: on to the dump number
            return Some(blanks(b, p));
        } else {
            // `([^\s,=]+)(=)([^\s,]+)`, else `[^\s,]+`
            let mut r = p;
            while r < n && !ends(b[r]) && b[r] != b'=' {
                r += 1;
            }
            let mut e = r;
            while e < n && !ends(b[e]) {
                e += 1;
            }
            if r > p && r < n && b[r] == b'=' && r + 1 < e {
                put(c, p, r, OPTION);
                put(c, r, r + 1, OPTION_SEPARATOR);
                put(c, r + 1, e, OPTION_VALUE);
            } else {
                put(c, p, e, OPTION);
            }
            p = e;
        }
    }
}
