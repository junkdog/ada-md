//! `CMakeCache.txt`, in place of bat's `CMakeCache`, on the line-format engine (X-104): a line
//! is a comment (`#`, or `//`, a cache entry's documentation) or an entry, `KEY:TYPE=VALUE`.
//!
//! bat's grammar reads a key only at a line's start (`^(\w|-)+`), and every other byte its
//! patterns don't take is `invalid.illegal`, one character at a time (`.`), blanks too. After a
//! key, its type's context (`:`, the type's run, else an error a character) stays open across
//! lines until a type is read; the value's (`=`s, then the rest of the line) ends with the line.
//! So what a line leaves open is whether a key still waits for its type.
//! Its readings are held by `tests/readings.rs`' snapshots (X-116).

use crate::emit::{Push, Runs, Sink};
use crate::line_format::{Cursor, Format, leads};
use crate::prim::{WORD, char_len, is_word_starting};
use crate::{Out, State};

const COMMENT: u16 = 0;
const COMMENT_MARK: u16 = 1;
const DOC: u16 = 2;
const DOC_MARK: u16 = 3;
const KEY: u16 = 4;
const SEPARATOR: u16 = 5;
const TYPE: u16 = 6;
const ASSIGNMENT: u16 = 7;
const VALUE: u16 = 8;
const INVALID: u16 = 9;

/// Each kind's scopes under `source.cmakecache`, as bat's grammar gives them.
pub const KINDS: &[&str] = &[
    "comment.line.number-sign.cmakecache",
    "comment.line.number-sign.cmakecache punctuation.definition.comment.cmakecache",
    "comment.line.double-slash.doc.cmakecache",
    "comment.line.double-slash.doc.cmakecache punctuation.definition.comment.cmakecache",
    "variable.parameter.cmakecache",
    "punctuation.separator.cmakecache",
    "storage.type.cmakecache",
    "keyword.operator.assignment.cmakecache",
    "string.unquoted.cmakecache",
    "invalid.illegal.cmakecache",
];

/// A key waits for its type: what a line leaves open.
const TYPE_OPEN: u64 = 1;

/// A type's characters, `[a-zA-Z0-9\-_:\\/ .]`.
static TYPE_CHAR: [bool; 256] =
    crate::prim::stops(b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_:\\/ .");

/// Lex `b`, a line without its `\n`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    crate::line_format::line::<Cache, _>(b, state, out, &mut Push);
}

/// [`line`]'s lexemes written to `k`, a run sink.
#[inline(never)]
pub(crate) fn line_runs(b: &[u8], state: &mut State, out: &mut Out, k: &mut Runs) {
    crate::line_format::line::<Cache, _>(b, state, out, k);
}

struct Cache;

impl Format for Cache {
    // every line read whole by `entry`: blanks before a comment are errors, a key only at 0
    const LEAD: [u8; 256] = leads(b"", b"");
    const COMMENT_TEXT: u16 = COMMENT;
    const COMMENT_MARK: u16 = COMMENT_MARK;
    const COMMENT_BLANKS: bool = false;

    #[inline(always)]
    fn resume<K: Sink>(c: &mut Cursor<K>, _: u64) -> u64 {
        type_from(c, 0)
    }

    /// A line of blanks: each an error.
    #[inline(always)]
    fn blank<K: Sink>(c: &mut Cursor<K>) {
        if c.len() > 0 {
            c.put(c.len(), INVALID);
        }
    }

    #[inline(always)]
    fn entry<K: Sink>(c: &mut Cursor<K>, _: usize) -> u64 {
        let b = c.b;
        let n = b.len();
        let mut p = 0;
        while p < n {
            match b[p] {
                b'#' => return comment(c, p, 1, COMMENT_MARK, COMMENT),
                b'/' if b.get(p + 1) == Some(&b'/') => return comment(c, p, 2, DOC_MARK, DOC),
                _ if p == 0 && is_key(b, 0) => {
                    let mut e = 0;
                    while e < n && is_key(b, e) {
                        e += if b[e] < 0x80 { 1 } else { char_len(b[e]) };
                    }
                    c.put(e.min(n), KEY);
                    return type_from(c, e.min(n));
                }
                _ => p = invalid(c, p),
            }
        }
        0
    }
}

/// A comment whose mark (`len` bytes) is at `p`, to the line's end.
#[inline(always)]
fn comment<K: Sink>(c: &mut Cursor<K>, p: usize, len: usize, mark: u16, text: u16) -> u64 {
    c.skip(p);
    c.put(p + len, mark);
    c.put(c.len(), text);
    0
}

/// A character at `p` that no pattern takes: an error. Where it ends.
#[inline(always)]
fn invalid<K: Sink>(c: &mut Cursor<K>, p: usize) -> usize {
    let e = (p + char_len(c.b[p])).min(c.len());
    c.skip(p);
    c.put_merged(e, INVALID);
    e
}

/// Whether the character at `p` is a key's, `\w` or `-`.
#[inline(always)]
fn is_key(b: &[u8], p: usize) -> bool {
    let x = b[p];
    if x < 0x80 {
        WORD[usize::from(x)] || x == b'-'
    } else {
        is_word_starting(b, p)
    }
}

/// A key's type from `p`: `:`s, the type's run (then its value), errors; still open at the
/// line's end where no type came.
#[inline(always)]
fn type_from<K: Sink>(c: &mut Cursor<K>, mut p: usize) -> u64 {
    let b = c.b;
    let n = b.len();
    while p < n {
        if b[p] == b':' {
            c.skip(p);
            c.put_merged(p + 1, SEPARATOR);
            p += 1;
        } else if TYPE_CHAR[usize::from(b[p])] {
            let mut e = p + 1;
            while e < n && TYPE_CHAR[usize::from(b[e])] {
                e += 1;
            }
            c.skip(p);
            c.put(e, TYPE);
            return value_from(c, e);
        } else {
            p = invalid(c, p);
        }
    }
    TYPE_OPEN
}

/// A value from `p`: its `=`s, then the line's rest.
#[inline(always)]
fn value_from<K: Sink>(c: &mut Cursor<K>, mut p: usize) -> u64 {
    let b = c.b;
    while p < b.len() && b[p] == b'=' {
        c.put_merged(p + 1, ASSIGNMENT);
        p += 1;
    }
    if p < b.len() {
        c.put(b.len(), VALUE);
    }
    0
}
