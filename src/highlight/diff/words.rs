//! The words a change block's lines changed (FEAT-023). A block's removed lines are paired with
//! its added ones, in order: the pairing whose likenesses add up the most, a pair's likeness the
//! share of their words both lines have, whitespace aside, taken as bags (in any order), and
//! at least half. A pair's words are then diffed, and kept a pair if still half alike in order:
//! each line's words that its pair hasn't are what the painter draws on a stronger tint. A line
//! left unpaired has none: it's new, or gone, as a whole, or moved (the other side has it as it
//! stands, and it pairs with nothing).
//!
//! A word is a run of letters, digits and `_`, a run of whitespace, or any other character alone,
//! a character being a grapheme (a letter and its combining marks, an emoji's ZWJ sequence), so
//! a change never splits one. A letter of a script written without spaces (CJK, Thai) is a word
//! alone. A character of no width (a BOM, a zero-width space) is the word after it's, or the one
//! before it's at the line's end, so its change shows on something.
//!
//! Lines are compared as the diff has them (PGR-005), not as they're shown: a tab is not the
//! spaces it expands to, and a line's `\r` is a word, so a tab turned to spaces or CRLF to LF
//! is a change. Its bytes are then mapped onto the shown line, a `\r` past its end: the line's
//! end is marked ([`END`]), which the painters draw on the cell after the text.
//!
//! Every removed line is weighed against every added one, so the weighing is a merge of two
//! sorted bags of hashes; diffing every pair in order cost a 21 MB `git log -p` 1.8× its time.

use std::ops::Range;

use unicode_segmentation::GraphemeCursor;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::layout::TAB_STOP;

/// A changed range's end that marks the line's end, past its shown text: a `\r` the pair line
/// hasn't. The range starts at the shown line's end.
pub(super) const END: u32 = u32::MAX;

/// The most words two paired lines may differ by, so that comparing two lines is bounded.
const EDITS: usize = 64;

/// A line longer than this is never paired: its words aren't compared.
const LINE_BYTES: usize = 1024;

/// A line's word: its hash, and where it is in the line.
#[derive(Clone, Copy)]
struct Word {
    hash: u64,
    at: u32,
    end: u32,
}

/// A block's lines' changed words, and the vectors that find them, kept from block to block.
#[derive(Default)]
pub(super) struct Words {
    /// The block's lines' words end to end, line `n`'s `words[ends[n - 1]..ends[n]]`.
    words: Vec<Word>,
    ends: Vec<usize>,
    /// The lines' words but whitespace as hashes, each line's sorted, line `n`'s
    /// `bags[bag_ends[n - 1]..bag_ends[n]]`.
    bags: Vec<u64>,
    bag_ends: Vec<usize>,
    /// Whether each line has a twin, a line the other side has as it stands.
    twins: Vec<bool>,
    /// Each pair's likeness, removed line by added line: `None` where they aren't alike.
    likeness: Vec<Option<u32>>,
    /// The pairing's best totals, `(removed + 1) × (added + 1)`, and how each was reached.
    best: Vec<(u32, Step)>,
    pairs: Vec<(usize, usize)>,
    /// Myers' furthest reaches, and a row of them a step for the path back.
    reach: Vec<i32>,
    trace: Vec<i32>,
    /// Whether each of a pair's words is changed, the first line's then the second's.
    changed: Vec<bool>,
    /// Each line's changed bytes, line `n`'s `ranges[spans[n].clone()]`.
    ranges: Vec<(u32, u32)>,
    spans: Vec<Range<usize>>,
}

#[derive(Clone, Copy, Default)]
enum Step {
    #[default]
    Removed,
    Added,
    Paired,
}

impl Words {
    /// Find the changed words of a block's `lines`, its `old` removed lines then the added ones,
    /// each with its mark, as the diff has it (tabs not expanded, a `\r` at its end kept).
    pub(super) fn block(&mut self, lines: &[&str], old: usize) {
        self.words.clear();
        self.ends.clear();
        self.bags.clear();
        self.bag_ends.clear();
        self.ranges.clear();
        self.spans.clear();
        let new = lines.len() - old;
        if old == 0 || new == 0 {
            return;
        }
        for line in lines {
            let (from, bag) = (self.words.len(), self.bags.len());
            if line.len() <= LINE_BYTES + 1 {
                split(line, &mut self.words);
            }
            let words = self.words[from..].iter().filter(|w| w.hash != SPACE);
            self.bags.extend(words.map(|w| w.hash));
            self.bags[bag..].sort_unstable();
            self.ends.push(self.words.len());
            self.bag_ends.push(self.bags.len());
            self.spans.push(0..0);
        }
        self.pair(old, new, lines);
    }

    /// Line `n`'s changed bytes, after [`Words::block`]: in the line as it's shown, its mark too
    /// (tabs expanded), and last the line's end's, `(len, END)`, when its `\r` changed.
    pub(super) fn line(&self, n: usize) -> &[(u32, u32)] {
        self.spans
            .get(n)
            .map_or(&[][..], |s| &self.ranges[s.clone()])
    }

    /// Line `n`'s words.
    fn of(&self, n: usize) -> Range<usize> {
        n.checked_sub(1).map_or(0, |p| self.ends[p])..self.ends[n]
    }

    /// Line `n`'s bag.
    fn bag(&self, n: usize) -> &[u64] {
        let from = n.checked_sub(1).map_or(0, |p| self.bag_ends[p]);
        &self.bags[from..self.bag_ends[n]]
    }

    /// Pair the `old` removed lines with the `new` added ones, and find each pair's changes. A
    /// line the other side has as it stands was moved, not edited (sorted imports): it pairs
    /// with none, so has no changes, and leaves the others to pair across it.
    fn pair(&mut self, old: usize, new: usize, lines: &[&str]) {
        self.twins.clear();
        self.twins.resize(lines.len(), false);
        for i in 0..old {
            for j in old..old + new {
                if lines[i][1..] == lines[j][1..] {
                    (self.twins[i], self.twins[j]) = (true, true);
                }
            }
        }
        self.likeness.clear();
        for i in 0..old {
            for j in old..old + new {
                let like = if self.twins[i] || self.twins[j] {
                    None
                } else {
                    (lines[i].len() <= LINE_BYTES + 1 && lines[j].len() <= LINE_BYTES + 1)
                        .then(|| likeness(self.bag(i), self.bag(j)))
                        .flatten()
                };
                self.likeness.push(like);
            }
        }
        // the pairing whose likenesses add up the most, its lines in order
        let width = new + 1;
        self.best.clear();
        self.best.resize(width * (old + 1), (0, Step::Removed));
        for i in 0..=old {
            for j in 0..=new {
                let mut cell = (0, Step::Removed);
                if i > 0 {
                    cell = (self.best[(i - 1) * width + j].0, Step::Removed);
                }
                if j > 0 {
                    let added = self.best[i * width + j - 1].0;
                    if added > cell.0 || i == 0 {
                        cell = (added, Step::Added);
                    }
                }
                if i > 0
                    && j > 0
                    && let Some(like) = self.likeness[(i - 1) * new + j - 1]
                {
                    let paired = self.best[(i - 1) * width + j - 1].0 + like;
                    if paired >= cell.0 {
                        cell = (paired, Step::Paired);
                    }
                }
                self.best[i * width + j] = cell;
            }
        }
        let (mut i, mut j) = (old, new);
        self.pairs.clear();
        while i > 0 || j > 0 {
            match self.best[i * width + j].1 {
                Step::Paired => {
                    self.pairs.push((i - 1, old + j - 1));
                    i -= 1;
                    j -= 1;
                }
                Step::Added => j -= 1,
                Step::Removed => i -= 1,
            }
        }
        for n in 0..self.pairs.len() {
            let (a, b) = self.pairs[n];
            self.changes(a, b, lines);
        }
    }

    /// The words lines `a` and `b`, a pair, don't share: each one's changed bytes, when they're
    /// at least half alike in order. Diffed from their first and last unlike words.
    fn changes(&mut self, a: usize, b: usize, lines: &[&str]) {
        let (wa, wb) = (self.of(a), self.of(b));
        let (x, y) = (&self.words[wa], &self.words[wb]);
        let same = |i: usize, j: usize| same(x[i], y[j], lines[a], lines[b]);
        let (head, tail) = ends(x.len(), y.len(), same);
        let (n, m) = (x.len() - head - tail, y.len() - head - tail);
        let inner = |i: usize, j: usize| same(head + i, head + j);
        // edits are words in one line and not the other; two at least, so that lines of only
        // whitespace, which pair, show theirs (a blank line to spaces, a tab to spaces)
        let most = ((x.len() + y.len()) / 2).clamp(2, EDITS);
        self.trace.clear();
        let trace = Some(&mut self.trace);
        let Some(d) = myers(n, m, most, inner, &mut self.reach, trace) else {
            return;
        };
        self.changed.clear();
        self.changed.resize(x.len() + y.len(), false);
        let (cx, cy) = self.changed.split_at_mut(x.len());
        let (mx, my) = (&mut cx[head..head + n], &mut cy[head..head + m]);
        mx.fill(true);
        my.fill(true);
        back(&self.trace, (n, m), d, most, (mx, my));
        let (cx, cy) = self.changed.split_at(x.len());
        for (line, words, changed) in [(a, x, cx), (b, y, cy)] {
            let from = self.ranges.len();
            ranges(words, changed, lines[line], &mut self.ranges);
            let len = shown(lines[line], &mut self.ranges[from..]);
            // a range reaching the line's end: what's in the line, then the end alone
            if let Some(last) = self.ranges[from..].last_mut()
                && last.1 == END
                && last.0 < len
            {
                last.1 = len;
                self.ranges.push((len, END));
            }
            self.spans[line] = from..self.ranges.len();
        }
    }
}

/// `line`'s bytes `ranges`, sorted, mapped from the line as the diff has it to the line as it's
/// shown: a `\r` at its end dropped, tabs to spaces at stops of [`TAB_STOP`] and other control
/// characters to a space ([`crate::layout::expand_tabs`]). A range reaching the dropped `\r` ends
/// at [`END`]. The shown line's length.
pub(super) fn shown(line: &str, ranges: &mut [(u32, u32)]) -> u32 {
    let body = line.strip_suffix('\r').unwrap_or(line);
    // bytes that may start a control character, 16 at a time with no exit inside, which
    // vectorizes (as `layout::code`'s check)
    let suspect = |b: u8| (b < 0x20) | (b == 0x7f) | (b == 0xc2);
    let (chunks, rest) = line.as_bytes().as_chunks::<16>();
    let controls = chunks
        .iter()
        .any(|c| c.iter().fold(false, |m, &b| m | suspect(b)))
        || rest.iter().any(|&b| suspect(b));
    if body.len() == line.len() && !controls {
        return line.len() as u32;
    }
    // each range's start and end in turn, in order
    let mut at = 0;
    let (mut shown, mut col) = (0, 0);
    let mut map = |raw: u32, shown: u32| {
        while at < 2 * ranges.len() {
            let (a, b) = &mut ranges[at / 2];
            let p = if at % 2 == 0 { a } else { b };
            if *p != raw {
                return;
            }
            *p = shown;
            at += 1;
        }
    };
    for (i, c) in body.char_indices() {
        map(i as u32, shown);
        let (bytes, cols) = if c == '\t' {
            let n = TAB_STOP - col % TAB_STOP;
            (n, n)
        } else if c.is_control() {
            (1, 1)
        } else {
            (c.len_utf8(), UnicodeWidthChar::width(c).unwrap_or(0))
        };
        shown += bytes as u32;
        col += cols;
    }
    map(body.len() as u32, shown);
    map(line.len() as u32, END);
    shown
}

/// Git's whitespace errors in an added `line`, as the diff has it (git's default
/// `core.whitespace`, which `git diff` colors on added lines): blanks at its end (a `\r` aside,
/// as `cr-at-eol` has it), and spaces before a tab in its indentation. Onto `out`, sorted, in the
/// line as it's shown.
pub(super) fn whitespace_errors(line: &str, out: &mut Vec<(u32, u32)>) {
    out.clear();
    let blank = |b: &u8| matches!(b, b' ' | b'\t');
    let body = line.strip_suffix('\r').unwrap_or(line).as_bytes();
    let text = body.get(1..).unwrap_or_default();
    // most lines: no blank at the end, and no tab for a space to be before
    if !text.last().is_some_and(blank) && memchr::memchr(b'\t', text).is_none() {
        return;
    }
    let indent = text.iter().take_while(|b| blank(b)).count();
    // the spaces before each tab of the indentation
    let mut spaces = None;
    for (i, &b) in text[..indent].iter().enumerate() {
        match (b, spaces) {
            (b' ', None) => spaces = Some(i),
            (b'\t', Some(s)) => {
                out.push((s as u32 + 1, i as u32 + 1));
                spaces = None;
            }
            _ => {}
        }
    }
    let trail = text.iter().rev().take_while(|b| blank(b)).count();
    if trail > 0 {
        let at = (text.len() - trail) as u32 + 1;
        // a line of only blanks: its indentation's are in its end's
        while out.last().is_some_and(|l| l.0 >= at) {
            out.pop();
        }
        out.push((at, text.len() as u32 + 1));
    }
    if !out.is_empty() {
        shown(line, out);
    }
}

/// How alike two lines' bags `a` and `b` are, as a likeness for the pairing: the words they share,
/// both lines', over all their words, in 1024ths; `None` under half. Lines of only whitespace
/// are alike.
fn likeness(a: &[u64], b: &[u64]) -> Option<u32> {
    let all = a.len() + b.len();
    if all == 0 {
        return Some(1 << 10);
    }
    // half alike: both lines' shared words, twice those shared, at least half of all
    let least = all.div_ceil(4);
    let shared = 2 * shared(a, b, least)?;
    Some(((shared << 10) / all) as u32)
}

/// How many words two lines' sorted bags of hashes share, when at least `least`: `None` as soon
/// as the words left can't make it.
fn shared(a: &[u64], b: &[u64], least: usize) -> Option<usize> {
    let (mut i, mut j, mut n) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        if n + (a.len() - i).min(b.len() - j) < least {
            return None;
        }
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                n += 1;
                i += 1;
                j += 1;
            }
        }
    }
    (n >= least).then_some(n)
}

/// How many of `n` words and `m` are alike from the start, and then from the end.
fn ends(n: usize, m: usize, same: impl Fn(usize, usize) -> bool) -> (usize, usize) {
    let head = (0..n.min(m)).take_while(|&i| same(i, i)).count();
    let tail = (0..n.min(m) - head)
        .take_while(|&i| same(n - 1 - i, m - 1 - i))
        .count();
    (head, tail)
}

fn same(a: Word, b: Word, la: &str, lb: &str) -> bool {
    a.hash == b.hash && la[a.at as usize..a.end as usize] == lb[b.at as usize..b.end as usize]
}

/// A character's kind of word.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Word,
    Space,
    Other,
}

/// ASCII's kinds of word, by byte.
const ASCII: [Kind; 128] = {
    let mut t = [Kind::Other; 128];
    let mut b = 0;
    while b < 128 {
        let c = b as u8;
        t[b] = if c.is_ascii_alphanumeric() || c == b'_' {
            Kind::Word
        } else if c == b' ' || (c >= b'\t' && c <= b'\r') {
            Kind::Space
        } else {
            Kind::Other
        };
        b += 1;
    }
    t
};

/// The kind of the character at byte `i` of `line`, by its first code point, and its length: a
/// grapheme's, so that a letter's combining marks or an emoji's sequence stay whole. A letter of
/// a script written without spaces is a word alone.
fn kind(line: &str, i: usize) -> (Kind, usize) {
    let b = line.as_bytes();
    if b[i] < 0x80 && b.get(i + 1).is_none_or(|&n| n < 0x80) {
        return (ASCII[usize::from(b[i])], 1);
    }
    let c = line[i..].chars().next().expect("a character at a boundary");
    let kind = if unspaced(c) {
        Kind::Other
    } else if c.is_alphanumeric() || c == '_' {
        Kind::Word
    } else if c.is_whitespace() {
        Kind::Space
    } else {
        Kind::Other
    };
    let end = GraphemeCursor::new(i, line.len(), true)
        .next_boundary(line, 0)
        .ok()
        .flatten()
        .unwrap_or(i + c.len_utf8());
    (kind, end - i)
}

/// Whether `c` is a letter of a script written without spaces between words, whose runs are
/// sentences: Thai, Lao, Myanmar, Khmer, kana and the CJK ideographs.
fn unspaced(c: char) -> bool {
    matches!(
        u32::from(c),
        0x0E00..=0x0EFF
            | 0x1000..=0x109F
            | 0x1780..=0x17FF
            | 0x3040..=0x30FF
            | 0x31F0..=0x31FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0xFF66..=0xFF9F
            | 0x20000..=0x3FFFF
    )
}

/// Whether `s`, a character other than ASCII, shows nothing: a BOM, a zero-width space, a
/// combining mark with no letter. A control character is shown as a space.
fn unseen(s: &str) -> bool {
    s.as_bytes()[0] >= 0x80 && !s.starts_with(char::is_control) && s.width() == 0
}

/// `line`'s words past its mark onto `out`, a byte at a time past ASCII's: a `\r` at its end one
/// of its own.
fn split(line: &str, out: &mut Vec<Word>) {
    let first = out.len();
    let body = line.strip_suffix('\r').unwrap_or(line);
    let word = |at: usize, end: usize, k: Kind| Word {
        hash: match k {
            Kind::Space => SPACE,
            _ => hash(&body.as_bytes()[at..end]),
        },
        at: at as u32,
        end: end as u32,
    };
    let mut at = 1.min(body.len());
    if body.is_ascii() {
        // a byte a character: the table alone, as most lines are (`kind` for every byte, out of
        // line, was 2.2% of a 29 MB `git log -p`'s `--color-only` profile)
        let b = body.as_bytes();
        while at < b.len() {
            let k = ASCII[usize::from(b[at])];
            let mut end = at + 1;
            if k != Kind::Other {
                end += b[end..]
                    .iter()
                    .take_while(|&&c| ASCII[usize::from(c)] == k)
                    .count();
            }
            out.push(word(at, end, k));
            at = end;
        }
        at = b.len();
    }
    // where a character that shows nothing starts, which the word after it takes
    let mut glue = None;
    while at < body.len() {
        let (k, len) = kind(body, at);
        let mut end = at + len;
        if k != Kind::Other {
            while end < body.len() {
                let (next, len) = kind(body, end);
                if next != k {
                    break;
                }
                end += len;
            }
        } else if unseen(&body[at..end]) {
            glue.get_or_insert(at);
            at = end;
            continue;
        }
        out.push(match glue.take() {
            Some(g) => word(g, end, Kind::Other),
            None => word(at, end, k),
        });
        at = end;
    }
    // at the end, the word before takes it
    if let Some(g) = glue {
        let from = match out.len() > first {
            true => out.pop().map_or(g, |w| w.at as usize),
            false => g,
        };
        out.push(word(from, body.len(), Kind::Other));
    }
    if body.len() < line.len() {
        out.push(Word {
            hash: SPACE,
            at: body.len() as u32,
            end: line.len() as u32,
        });
    }
}

/// A word's hash, FNV-1a's: words are short, and only told apart within a block, where
/// [`same`] compares their text too. Never [`SPACE`].
fn hash(bytes: &[u8]) -> u64 {
    let h = bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    if h == SPACE { h ^ 1 } else { h }
}

/// Every run of whitespace's hash, which no other word has: it's left out of a line's bag, and
/// two runs are told apart by their text.
const SPACE: u64 = 0;

/// The edits between `n` words and `m`, `same` telling which are alike, when at most `most`:
/// Myers' O(ND) search, its furthest reaches in `reach`. With `trace`, each step's reaches, for
/// [`back`].
fn myers(
    n: usize,
    m: usize,
    most: usize,
    same: impl Fn(usize, usize) -> bool,
    reach: &mut Vec<i32>,
    mut trace: Option<&mut Vec<i32>>,
) -> Option<usize> {
    let most = most.min(n + m);
    let off = most as i32 + 1;
    reach.clear();
    reach.resize(2 * most + 3, 0);
    let v = reach;
    let (n, m) = (n as i32, m as i32);
    for d in 0..=most as i32 {
        for k in (-d..=d).step_by(2) {
            let at = (off + k) as usize;
            let mut x = if k == -d || (k != d && v[at - 1] < v[at + 1]) {
                v[at + 1]
            } else {
                v[at - 1] + 1
            };
            let mut y = x - k;
            while x < n && y < m && same(x as usize, y as usize) {
                x += 1;
                y += 1;
            }
            v[at] = x;
            if x >= n && y >= m {
                if let Some(t) = trace.as_mut() {
                    t.extend_from_slice(v);
                }
                return Some(d as usize);
            }
        }
        if let Some(t) = trace.as_mut() {
            t.extend_from_slice(v);
        }
    }
    None
}

/// Walk [`myers`]' `trace` back from the end, `(n, m)` words, `d` edits, its rows `most` wide,
/// clearing the words in common in `changed`, the first line's and the second's.
fn back(
    trace: &[i32],
    (n, m): (usize, usize),
    d: usize,
    most: usize,
    (cx, cy): (&mut [bool], &mut [bool]),
) {
    let most = most.min(n + m);
    let width = 2 * most + 3;
    let off = most as i32 + 1;
    let (mut x, mut y) = (n as i32, m as i32);
    let mut common = |x: i32, y: i32| {
        cx[x as usize] = false;
        cy[y as usize] = false;
    };
    for d in (1..=d as i32).rev() {
        let row = &trace[(d as usize - 1) * width..d as usize * width];
        let k = x - y;
        let down = k == -d || (k != d && row[(off + k - 1) as usize] < row[(off + k + 1) as usize]);
        let pk = if down { k + 1 } else { k - 1 };
        let px = row[(off + pk) as usize];
        let py = px - pk;
        // the snake after the edit, then the edit itself
        let (sx, sy) = if down { (px, py + 1) } else { (px + 1, py) };
        while x > sx && y > sy {
            x -= 1;
            y -= 1;
            common(x, y);
        }
        (x, y) = (px, py);
    }
    while x > 0 && y > 0 {
        x -= 1;
        y -= 1;
        common(x, y);
    }
}

/// The bytes of a line's changed `words`, onto `out`: two changes with nothing or only
/// whitespace between them made one.
fn ranges(words: &[Word], changed: &[bool], line: &str, out: &mut Vec<(u32, u32)>) {
    let from = out.len();
    for (w, _) in words.iter().zip(changed).filter(|&(_, &c)| c) {
        if out.len() > from {
            let last = out.last_mut().expect("a range after `from`");
            if line[last.1 as usize..w.at as usize].trim().is_empty() {
                last.1 = w.end;
                continue;
            }
        }
        out.push((w.at, w.end));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each line's changed text as it's shown, for a block of `old` removed lines and the added
    /// ones, each given without its mark; a line's end's mark as `$`.
    fn changed(lines: &[&str], old: usize) -> Vec<Vec<String>> {
        let marked: Vec<String> = lines
            .iter()
            .enumerate()
            .map(|(n, l)| format!("{}{l}", if n < old { '-' } else { '+' }))
            .collect();
        let marked: Vec<&str> = marked.iter().map(String::as_str).collect();
        let mut w = Words::default();
        w.block(&marked, old);
        (0..lines.len())
            .map(|n| {
                let raw = marked[n];
                let line = crate::layout::expand_tabs(raw.strip_suffix('\r').unwrap_or(raw));
                w.line(n)
                    .iter()
                    .map(|&(a, b)| match b {
                        END => "$".to_owned(),
                        b => line[a as usize..b as usize].to_owned(),
                    })
                    .collect()
            })
            .collect()
    }

    /// PGR-005: changes that show no text of their own show on what's beside them, or at the
    /// line's end.
    #[test]
    fn invisible_changes_show() {
        // CRLF to LF, and back: the line's end
        let got = changed(&["let x = 1;\r", "let x = 1;"], 1);
        assert_eq!(got, [vec!["$"], vec![]]);
        let got = changed(&["a b  ", "a b  \r"], 1);
        assert_eq!(got, [vec![], vec!["$"]]);
        // a word and the `\r`: both
        let got = changed(&["a b\r", "a c"], 1);
        assert_eq!(got, [vec!["b", "$"], vec!["c"]]);
        // a tab to spaces, its width or not
        let got = changed(&["\tx = 1;", "   x = 1;"], 1);
        assert_eq!(got, [vec!["   "], vec!["   "]]);
        // a blank line to whitespace, a tab alone to spaces
        assert_eq!(changed(&["", "  "], 1), [vec![], vec!["  "]]);
        assert_eq!(changed(&["\t", "    "], 1), [vec!["   "], vec!["    "]]);
        // a BOM, a zero-width space and a combining mark show on the word they're with
        let got = changed(&["\u{feff}use x;", "use x;"], 1);
        assert_eq!(got, [vec!["\u{feff}use"], vec!["use"]]);
        let got = changed(&["foo\u{200b}bar = 1", "foobar = 1"], 1);
        assert_eq!(got, [vec!["foo\u{200b}bar"], vec!["foobar"]]);
        let got = changed(&["let cafe\u{301} = 1;", "let cafe\u{300} = 1;"], 1);
        assert_eq!(got, [vec!["cafe\u{301}"], vec!["cafe\u{300}"]]);
        // at the end, the word before it
        let got = changed(&["x = y\u{200b}", "x = y"], 1);
        assert_eq!(got, [vec!["y\u{200b}"], vec!["y"]]);
    }

    /// PGR-005: a tab's gap, which its column sets, isn't a change of its own beside one.
    #[test]
    fn a_tabs_gap_is_no_change() {
        let got = changed(&["ab\tc", "abc\tc"], 1);
        assert_eq!(got, [vec!["ab"], vec!["abc"]]);
    }

    /// PGR-005: a grapheme stays whole, and a script without spaces is compared a letter at a
    /// time.
    #[test]
    fn graphemes_and_unspaced_scripts() {
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
        let couple = "\u{1f468}\u{200d}\u{1f469}";
        let got = changed(&[&format!("say({family});"), &format!("say({couple});")], 1);
        assert_eq!(got, [vec![family], vec![couple]]);
        let got = changed(&["日本語の文章です", "日本語の文書です"], 1);
        assert_eq!(got, [vec!["章"], vec!["書"]]);
    }

    /// PGR-005: git's whitespace errors in an added line, as it's shown.
    #[test]
    fn whitespace_errors_are_found() {
        let errors = |line: &str| {
            let mut out = Vec::new();
            whitespace_errors(line, &mut out);
            out
        };
        assert_eq!(errors("+x = 1;"), []);
        assert_eq!(errors("+x = 1;  "), [(7, 9)]);
        // a `\r` is no blank, as `cr-at-eol` has it
        assert_eq!(errors("+x = 1;\r"), []);
        assert_eq!(errors("+x = 1; \r"), [(7, 8)]);
        // a trailing tab, as shown
        assert_eq!(errors("+x\t"), [(2, 4)]);
        // spaces before a tab in the indentation, as shown: `+` `  ` to column 4, then `x`
        assert_eq!(errors("+  \tx"), [(1, 3)]);
        assert_eq!(errors("+\t \tx"), [(4, 5)]);
        // a line of only blanks, all of it
        assert_eq!(errors("+ \t "), [(1, 5)]);
        assert_eq!(errors("+"), []);
    }

    #[test]
    fn a_pairs_changed_words_stand_out() {
        let got = changed(&["let x = foo(a, b);", "let y = foo(a, c);"], 1);
        assert_eq!(got, [vec!["x", "b"], vec!["y", "c"]]);
        // whitespace between two changes joins them; an unchanged word doesn't
        let got = changed(&["call(one two)", "call(three four)"], 1);
        assert_eq!(got, [vec!["one two"], vec!["three four"]]);
        // a word only added, at either end
        let got = changed(&["a b", "x y z", "a b c", "w x y z"], 2);
        assert_eq!(got, [vec![], vec![], vec![" c"], vec!["w "]]);
    }

    #[test]
    fn lines_unalike_are_not_paired() {
        let got = changed(&["return a;", "if b { c }"], 1);
        assert_eq!(got, [Vec::<String>::new(), vec![]]);
        // a line only removed, or only added: nothing to compare it with
        assert_eq!(changed(&["x = 1"], 1), [Vec::<String>::new()]);
        assert_eq!(changed(&["x = 1"], 0), [Vec::<String>::new()]);
    }

    /// Lines pair with the line most like them, in order, not by their place in the block.
    #[test]
    fn lines_pair_with_their_likes() {
        let lines = [
            "fn alpha(x: u32) -> u32 {",
            "    x + 1",
            "/// a new comment, all of it",
            "fn alpha(x: u64) -> u32 {",
            "    x + 2",
        ];
        let got = changed(&lines, 2);
        assert_eq!(
            got,
            [vec!["u32"], vec!["1"], vec![], vec!["u64"], vec!["2"]]
        );
    }

    /// Sorted imports: each line moved, none edited, though `io` and `fmt` pair up alike.
    #[test]
    fn moved_lines_have_no_changes() {
        let lines = [
            "use std::io;",
            "use std::fmt;",
            "use std::fmt;",
            "use std::io;",
        ];
        assert!(changed(&lines, 2).iter().all(Vec::is_empty));
        // a moved line beside an edited one: the edit keeps its words
        let lines = ["use std::io;", "let x = 1;", "let x = 2;", "use std::io;"];
        let got = changed(&lines, 2);
        assert_eq!(got, [vec![], vec!["1"], vec!["2"], vec![]]);
    }

    #[test]
    fn identical_and_empty_lines_have_no_changes() {
        let got = changed(&["}", "", "}", ""], 2);
        assert!(got.iter().all(Vec::is_empty));
    }

    /// Myers' path back marks exactly the words outside a longest common run.
    #[test]
    fn myers_finds_the_fewest_edits() {
        let (a, b) = (b"abcabba", b"cbabac");
        let same = |i: usize, j: usize| a[i] == b[j];
        let (mut reach, mut trace) = (Vec::new(), Vec::new());
        let d = myers(a.len(), b.len(), 64, same, &mut reach, Some(&mut trace)).unwrap();
        assert_eq!(d, 5);
        let (mut ca, mut cb) = (vec![true; a.len()], vec![true; b.len()]);
        back(&trace, (a.len(), b.len()), d, 64, (&mut ca, &mut cb));
        let kept = |s: &[u8], c: &[bool]| -> String {
            s.iter()
                .zip(c)
                .filter(|&(_, &c)| !c)
                .map(|(&b, _)| b as char)
                .collect()
        };
        assert_eq!(kept(a, &ca), kept(b, &cb));
        assert_eq!(kept(a, &ca).len(), 4);
        assert_eq!(myers(a.len(), b.len(), 4, same, &mut reach, None), None);
    }
}
