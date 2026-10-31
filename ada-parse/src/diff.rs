//! Diffs and patches, in place of bat's `Diff`: each line is what its first bytes say, a file
//! header, a hunk's range, an added, removed or changed line, or context left plain. A line
//! doesn't depend on the ones before it.

use crate::{Out, skip_spaces, trim_end};

const SEPARATOR: u8 = 0;
const RANGE_NORMAL: u8 = 1;
const RANGE: u8 = 2;
const RANGE_MARK: u8 = 3;
const RANGE_LINES: u8 = 4;
const RANGE_SECTION: u8 = 5;
const RANGE_CONTEXT: u8 = 6;
const RANGE_CONTEXT_MARK: u8 = 7;
const FROM_FILE: u8 = 8;
const FROM_FILE_MARK: u8 = 9;
const TO_FILE: u8 = 10;
const TO_FILE_MARK: u8 = 11;
const INSERTED: u8 = 12;
const INSERTED_MARK: u8 = 13;
const CHANGED: u8 = 14;
const CHANGED_MARK: u8 = 15;
const DELETED: u8 = 16;
const DELETED_MARK: u8 = 17;
const INDEX: u8 = 18;
const INDEX_SEPARATOR: u8 = 19;
const INDEX_FILE: u8 = 20;

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
    let len = b.len();
    let whole = |out: &mut Out, kind: u8, mark: usize, mark_kind: u8| {
        out.push(0, mark, mark_kind);
        out.push(mark, len, kind);
    };
    let starts = |s: &[u8]| b.starts_with(s);
    let after_mark = |c: u8| b.first() == Some(&c) && (len == 1 || b[1] == b' ');
    if b == b"---" || b == [b'*'; 15] || b == [b'='; 67] {
        out.push(0, len, SEPARATOR);
    } else if normal_range(b) {
        out.push(0, len, RANGE_NORMAL);
    } else if let Some((lines, at, section)) = unified_range(b) {
        out.push(0, 2, RANGE_MARK);
        out.push(2, lines.0, RANGE);
        out.push(lines.0, lines.1, RANGE_LINES);
        out.push(lines.1, at, RANGE);
        out.push(at, at + 2, RANGE_MARK);
        out.push(at + 2, section.0, RANGE);
        out.push(section.0, section.1, RANGE_SECTION);
        out.push(section.1, len, RANGE);
    } else if len >= 10
        && ((starts(b"--- ") && b.ends_with(b" ----"))
            || (starts(b"*** ") && b.ends_with(b" ****")))
    {
        out.push(0, 3, RANGE_CONTEXT_MARK);
        out.push(3, len - 4, RANGE_CONTEXT);
        out.push(len - 4, len, RANGE_CONTEXT_MARK);
    } else if len >= 5 && (starts(b"--- ") || starts(b"*** ")) {
        whole(out, FROM_FILE, 3, FROM_FILE_MARK);
    } else if let Some(dash) = perforce_from(b) {
        out.push(0, 4, FROM_FILE_MARK);
        out.push(4, dash, FROM_FILE);
        perforce_to(b, dash, out);
    } else if len >= 5 && starts(b"+++ ") {
        whole(out, TO_FILE, 3, TO_FILE_MARK);
    } else if starts(b"+") || after_mark(b'>') {
        whole(out, INSERTED, 1, INSERTED_MARK);
    } else if starts(b"!") {
        whole(out, CHANGED, 1, CHANGED_MARK);
    } else if starts(b"-") || after_mark(b'<') {
        whole(out, DELETED, 1, DELETED_MARK);
    } else if len >= 8 && starts(b"Index: ") {
        out.push(0, 5, INDEX);
        out.push(5, 6, INDEX_SEPARATOR);
        out.push(6, 7, INDEX);
        out.push(7, len, INDEX_FILE);
    } else {
        perforce_to(b, 0, out);
    }
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
fn perforce_to(b: &[u8], at: usize, out: &mut Out) {
    let len = b.len();
    if !b.ends_with(b" ====") {
        return;
    }
    let Some(dash) = find(&b[at..], b" - ").map(|i| at + i) else {
        return;
    };
    if dash + 3 > len - 5 {
        return;
    }
    out.push(dash, dash + 1, TO_FILE);
    out.push(dash + 1, dash + 2, TO_FILE_MARK);
    out.push(dash + 2, len - 4, TO_FILE);
    out.push(len - 4, len, TO_FILE_MARK);
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
