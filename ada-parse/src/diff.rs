//! Diffs and patches, in place of bat's `Diff`: each line is what its first bytes say, a file
//! header, a hunk's range, an added, removed or changed line, or context left plain. A line
//! doesn't depend on the ones before it.

use crate::emit::{Push, Runs, Sink};
use crate::{Out, State, same, skip_spaces, trim_end};

const SEPARATOR: u16 = 0;
const RANGE_NORMAL: u16 = 1;
const RANGE: u16 = 2;
const RANGE_MARK: u16 = 3;
const RANGE_LINES: u16 = 4;
const RANGE_SECTION: u16 = 5;
const RANGE_CONTEXT: u16 = 6;
const RANGE_CONTEXT_MARK: u16 = 7;
const FROM_FILE: u16 = 8;
const FROM_FILE_MARK: u16 = 9;
const TO_FILE: u16 = 10;
const TO_FILE_MARK: u16 = 11;
const INSERTED: u16 = 12;
const INSERTED_MARK: u16 = 13;
const CHANGED: u16 = 14;
const CHANGED_MARK: u16 = 15;
const DELETED: u16 = 16;
const DELETED_MARK: u16 = 17;
const INDEX: u16 = 18;
const INDEX_SEPARATOR: u16 = 19;
const INDEX_FILE: u16 = 20;

/// Each kind's scopes under `source.diff`, as bat's grammar gives them.
pub const KINDS: &[&str] = &[
    "meta.separator.diff punctuation.definition.separator.diff",
    "meta.diff.range.normal meta.range.normal.diff",
    "meta.diff.range.unified meta.range.unified.diff",
    "meta.diff.range.unified meta.range.unified.diff punctuation.definition.range.diff",
    "meta.diff.range.unified meta.range.unified.diff meta.toc-list.line-number.diff",
    "meta.diff.range.unified meta.range.unified.diff entity.name.section.diff",
    "meta.diff.range.context meta.range.context.diff",
    "meta.diff.range.context meta.range.context.diff punctuation.definition.range.diff",
    "meta.diff.header.from-file meta.header.from-file.diff",
    "meta.diff.header.from-file meta.header.from-file.diff punctuation.definition.from-file.diff",
    "meta.diff.header.to-file meta.header.to-file.diff",
    "meta.diff.header.to-file meta.header.to-file.diff punctuation.definition.to-file.diff",
    "markup.inserted.diff",
    "markup.inserted.diff punctuation.definition.inserted.diff",
    "markup.changed.diff",
    "markup.changed.diff punctuation.definition.changed.diff",
    "markup.deleted.diff",
    "markup.deleted.diff punctuation.definition.deleted.diff",
    "meta.diff.index meta.index.diff",
    "meta.diff.index meta.index.diff punctuation.separator.key-value.diff",
    "meta.diff.index meta.index.diff meta.toc-list.file-name.diff",
];

/// Bytes `.0...1` of a line.
type Stretch = (usize, usize);

/// One line, by the first of the grammar's patterns that matches it.
#[inline(never)]
pub fn line(b: &[u8], out: &mut Out) {
    scan(b, out, &mut Push);
}

/// [`line`]'s runs by theme rule written to `k` (X-163): inline in a block's loop, as the line
/// formats' are (X-050), [`crate::lex_runs`]' line too. Read from its spans, a line cost ada's
/// path ~118 instructions past its scan (X-074's run sink, a table call a line, ran 3–4% slower
/// in ada).
pub(crate) struct RunLine;

impl crate::LineScan for RunLine {
    #[inline(always)]
    fn scan(b: &[u8], _: &mut State, out: &mut Out, k: &mut Runs) {
        scan(b, out, k);
    }
}

/// [`line`] on the sink `k`. The grammar's patterns in its order, but only those that can
/// match a line starting with its first byte (X-163): a context line, the commonest, is the
/// Perforce to-file's test alone.
#[inline(always)]
fn scan<K: Sink>(b: &[u8], mut out: &mut Out, k: &mut K) {
    // a header's or a range's writes, rare, through the sink's one call; a changed line's, the
    // commonest, inline ([`Sink::push_call`]: every write through it ran ada's path 13% slower,
    // every write inline cost ada 9.4 KB)
    let push = &mut |s: usize, e: usize, kind: u16| k.push_call(&mut out, s, e, kind);
    let len = b.len();
    // the line its mark's and the rest its kind's
    macro_rules! whole {
        ($kind:expr, $mark:expr, $mark_kind:expr) => {{
            push(0, $mark, $mark_kind);
            push($mark, len, $kind);
        }};
    }
    // as `whole!`, inline
    macro_rules! changed {
        ($kind:expr, $mark_kind:expr) => {{
            k.push(&mut out, 0, 1, $mark_kind);
            k.push(&mut out, 1, len, $kind);
        }};
    }
    let starts = |s: &[u8]| len >= s.len() && same(&b[..s.len()], s);
    let ends = |s: &[u8]| len >= s.len() && same(&b[len - s.len()..], s);
    let Some(&first) = b.first() else {
        return;
    };
    // `>` and `<` a mark before a blank or the line's end
    let after_mark = len == 1 || b[1] == b' ';
    match first {
        b'+' if len >= 5 && starts(b"+++ ") => whole!(TO_FILE, 3, TO_FILE_MARK),
        b'+' => changed!(INSERTED, INSERTED_MARK),
        b'-' if len == 3 && starts(b"---") => push(0, len, SEPARATOR),
        b'-' if len >= 10 && starts(b"--- ") && ends(b" ----") => context_range(len, push),
        b'-' if len >= 5 && starts(b"--- ") => whole!(FROM_FILE, 3, FROM_FILE_MARK),
        b'-' => changed!(DELETED, DELETED_MARK),
        b'*' if b == [b'*'; 15] => push(0, len, SEPARATOR),
        b'*' if len >= 10 && starts(b"*** ") && ends(b" ****") => context_range(len, push),
        b'*' if len >= 5 && starts(b"*** ") => whole!(FROM_FILE, 3, FROM_FILE_MARK),
        b'=' if b == [b'='; 67] => push(0, len, SEPARATOR),
        b'=' if let Some(dash) = perforce_from(b) => {
            push(0, 4, FROM_FILE_MARK);
            push(4, dash, FROM_FILE);
            perforce_to(b, dash, push);
        }
        b'0'..=b'9' if normal_range(b) => push(0, len, RANGE_NORMAL),
        b'@' if let Some((lines, at, section)) = unified_range(b) => {
            push(0, 2, RANGE_MARK);
            push(2, lines.0, RANGE);
            push(lines.0, lines.1, RANGE_LINES);
            push(lines.1, at, RANGE);
            push(at, at + 2, RANGE_MARK);
            push(at + 2, section.0, RANGE);
            push(section.0, section.1, RANGE_SECTION);
            push(section.1, len, RANGE);
        }
        b'>' if after_mark => changed!(INSERTED, INSERTED_MARK),
        b'!' => changed!(CHANGED, CHANGED_MARK),
        b'<' if after_mark => changed!(DELETED, DELETED_MARK),
        b'I' if len >= 8 && starts(b"Index: ") => {
            push(0, 5, INDEX);
            push(5, 6, INDEX_SEPARATOR);
            push(6, 7, INDEX);
            push(7, len, INDEX_FILE);
        }
        _ => perforce_to(b, 0, push),
    }
}

/// A context diff's range, `--- 1,5 ----` or `*** 1,5 ****`: its marks and the range.
#[inline(always)]
fn context_range(len: usize, push: &mut impl FnMut(usize, usize, u16)) {
    push(0, 3, RANGE_CONTEXT_MARK);
    push(3, len - 4, RANGE_CONTEXT);
    push(len - 4, len, RANGE_CONTEXT_MARK);
}

/// `^\d+(,\d+)*(a|d|c)\d+(,\d+)*$`: a normal diff's `12,15c12`.
fn normal_range(b: &[u8]) -> bool {
    let at = numbers(b, 0);
    at > 0
        && matches!(b.get(at), Some(b'a' | b'd' | b'c'))
        && numbers(b, at + 1) > at + 1
        && numbers(b, at + 1) == b.len()
}

/// Where `\d+(,\d+)*` at `at` ends, or `at` when there's no digit there.
fn numbers(b: &[u8], at: usize) -> usize {
    let digits = |i: usize| i + b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
    let mut end = digits(at);
    if end == at {
        return at;
    }
    while b.get(end) == Some(&b',') && b.get(end + 1).is_some_and(u8::is_ascii_digit) {
        end = digits(end + 1);
    }
    end
}

/// `^(@@)\s*(.+?)\s*(@@)\s*(.*?)\s*$`: the line numbers, where the second `@@` is, and the
/// section heading after it.
fn unified_range(b: &[u8]) -> Option<(Stretch, usize, Stretch)> {
    if !b.starts_with(b"@@") {
        return None;
    }
    let spaces = skip_spaces(b, 2);
    // `.+?` takes a byte at least, then stops at the first `@@` it can; with none after that
    // byte, `\s*` gives it back a space, before an `@@` right after the spaces
    let after = |from: usize| Some(from + 1 + find(b.get(from + 1..)?, b"@@")?);
    let (from, at) = match after(spaces) {
        Some(at) => (spaces, at),
        None if spaces > 2 && b[spaces..].starts_with(b"@@") => (spaces - 1, spaces),
        None => return None,
    };
    let lines = (from, trim_end(b, from + 1, at));
    let section = skip_spaces(b, at + 2);
    Some((lines, at, (section, trim_end(b, section, b.len()))))
}

/// A Perforce header's from-file, `^(={4}) .+(?= - )`: where its last ` - ` is.
fn perforce_from(b: &[u8]) -> Option<usize> {
    if !b.starts_with(b"==== ") {
        return None;
    }
    let dash = b.windows(3).rposition(|w| w == b" - ")?;
    (dash >= 6).then_some(dash)
}

/// A Perforce header's to-file, ` (-) .* (={4})$`, searched for from `at`; nothing when there
/// is none.
#[inline(always)]
fn perforce_to(b: &[u8], at: usize, push: &mut impl FnMut(usize, usize, u16)) {
    let len = b.len();
    if len < 5 || !same(&b[len - 5..], b" ====") {
        return;
    }
    let Some(dash) = find(&b[at..], b" - ").map(|i| at + i) else {
        return;
    };
    if dash + 3 > len - 5 {
        return;
    }
    push(dash, dash + 1, TO_FILE);
    push(dash + 1, dash + 2, TO_FILE_MARK);
    push(dash + 2, len - 4, TO_FILE);
    push(len - 4, len, TO_FILE_MARK);
}

/// Where `needle` first starts in `b`.
fn find(b: &[u8], needle: &[u8]) -> Option<usize> {
    b.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use crate::Lang;
    use crate::tests::marked;

    fn kinds(line: &str) -> Vec<(&str, &'static str)> {
        marked(Lang::Diff, &[line])
            .into_iter()
            .map(|(t, s)| (t, s.rsplit(' ').next().unwrap()))
            .collect()
    }

    #[test]
    fn headers() {
        assert_eq!(kinds("diff --git a/x b/x"), []);
        assert_eq!(
            kinds("--- a/x"),
            [
                ("---", "punctuation.definition.from-file.diff"),
                (" a/x", "meta.header.from-file.diff")
            ]
        );
        assert_eq!(
            kinds("+++ b/x"),
            [
                ("+++", "punctuation.definition.to-file.diff"),
                (" b/x", "meta.header.to-file.diff")
            ]
        );
        assert_eq!(
            kinds("--- 1,5 ----"),
            [
                ("---", "punctuation.definition.range.diff"),
                (" 1,5 ", "meta.range.context.diff"),
                ("----", "punctuation.definition.range.diff"),
            ]
        );
        assert_eq!(
            kinds("Index: src/a.c"),
            [
                ("Index", "meta.index.diff"),
                (":", "punctuation.separator.key-value.diff"),
                (" ", "meta.index.diff"),
                ("src/a.c", "meta.toc-list.file-name.diff"),
            ]
        );
        assert_eq!(
            kinds("---"),
            [("---", "punctuation.definition.separator.diff")]
        );
    }

    #[test]
    fn ranges() {
        assert_eq!(
            kinds("@@ -1,3 +1,4 @@  fn main() {  "),
            [
                ("@@", "punctuation.definition.range.diff"),
                (" ", "meta.range.unified.diff"),
                ("-1,3 +1,4", "meta.toc-list.line-number.diff"),
                (" ", "meta.range.unified.diff"),
                ("@@", "punctuation.definition.range.diff"),
                ("  ", "meta.range.unified.diff"),
                ("fn main() {", "entity.name.section.diff"),
                ("  ", "meta.range.unified.diff"),
            ]
        );
        assert_eq!(kinds("@@ -1 @@@"), kinds("@@ -1 @@@"));
        assert_eq!(kinds("@@@@"), []);
        // `\s*` gives the space back to the line numbers
        assert_eq!(
            kinds("@@  @@"),
            [
                ("@@", "punctuation.definition.range.diff"),
                (" ", "meta.range.unified.diff"),
                (" ", "meta.toc-list.line-number.diff"),
                ("@@", "punctuation.definition.range.diff"),
            ]
        );
        assert_eq!(kinds("12,15c12"), [("12,15c12", "meta.range.normal.diff")]);
        assert_eq!(kinds("12,c12"), []);
    }

    #[test]
    fn changes() {
        assert_eq!(
            kinds("+ added"),
            [
                ("+", "punctuation.definition.inserted.diff"),
                (" added", "markup.inserted.diff")
            ]
        );
        assert_eq!(
            kinds("-removed"),
            [
                ("-", "punctuation.definition.deleted.diff"),
                ("removed", "markup.deleted.diff")
            ]
        );
        assert_eq!(kinds("> x").len(), 2);
        assert_eq!(kinds(">x"), []);
        assert_eq!(kinds("! changed").len(), 2);
        assert_eq!(kinds(" context"), []);
        assert_eq!(kinds(""), []);
    }

    #[test]
    fn perforce() {
        assert_eq!(
            kinds("==== //depot/a.c#3 - /home/a.c ===="),
            [
                ("====", "punctuation.definition.from-file.diff"),
                (" //depot/a.c#3", "meta.header.from-file.diff"),
                (" ", "meta.header.to-file.diff"),
                ("-", "punctuation.definition.to-file.diff"),
                (" /home/a.c ", "meta.header.to-file.diff"),
                ("====", "punctuation.definition.to-file.diff"),
            ]
        );
    }
}
