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

/// A unified diff's hunk header, `@@ -a[,b] +c[,d] @@ section` (X-174), or a combined diff's,
/// `@@@ -a,b -c,d +e,f @@@ section` (X-176): how many of the old file's lines and the new file's
/// the hunk holds, and where its section text is, the bytes [`line`] gives the section's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hunk {
    /// The old file's lines; a combined diff's first parent's.
    pub old: u32,
    pub new: u32,
    /// Bytes `.0..1` of the line; empty when there is no section text.
    pub section: (usize, usize),
}

/// A range's count of lines at `at` in `r`, `sign` then `a[,b]` (a count left out being 1), and
/// where it ends.
fn count(r: &[u8], at: usize, sign: u8) -> Option<(u32, usize)> {
    if r.get(at) != Some(&sign) {
        return None;
    }
    let digits = |at: usize| {
        r[at.min(r.len())..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count()
    };
    let at = at + 1;
    let n = digits(at);
    if n == 0 {
        return None;
    }
    if r.get(at + n) != Some(&b',') {
        return Some((1, at + n));
    }
    let m = digits(at + n + 1);
    let c = std::str::from_utf8(&r[at + n + 1..at + n + 1 + m]).ok()?;
    Some((c.parse().ok()?, at + n + 1 + m))
}

/// The hunk `line` starts, if it is a unified diff's hunk header. A combined diff's (`@@@`) is
/// none: its lines carry a mark a parent ([`combined`]).
pub fn hunk(line: &str) -> Option<Hunk> {
    let b = line.as_bytes();
    let (lines, _, section) = unified_range(b)?;
    let range = &b[lines.0..lines.1];
    let (old, old_end) = count(range, 0, b'-')?;
    let gap = skip_spaces(range, old_end);
    let (new, new_end) = count(range, gap, b'+')?;
    if gap == old_end || new_end != range.len() {
        return None;
    }
    Some(Hunk { old, new, section })
}

/// The hunk `line` starts, if it is a combined diff's hunk header (X-176), a merge's in `git
/// diff --cc` or `git show`: an `@` for each parent and one more, a `-a,b` range for each parent,
/// then the result's `+c,d`, each parent's count of lines into `parents`. Its lines carry a mark
/// a parent, `-` where the line is that parent's alone, `+` where it is the result's alone.
pub fn combined(line: &str, parents: &mut Vec<u32>) -> Option<Hunk> {
    let b = line.as_bytes();
    let ats = b.iter().take_while(|&&c| c == b'@').count();
    if ats < 3 {
        return None;
    }
    parents.clear();
    // at least one space, then what `f` reads
    let spaced = |at: usize| (b.get(at) == Some(&b' ')).then(|| skip_spaces(b, at));
    let mut at = ats;
    for _ in 1..ats {
        let (n, end) = count(b, spaced(at)?, b'-')?;
        parents.push(n);
        at = end;
    }
    let (new, end) = count(b, spaced(at)?, b'+')?;
    let close = spaced(end)?;
    let after = close + ats;
    if b.get(close..after)?.iter().any(|&c| c != b'@') || b.get(after).is_some_and(|&c| c != b' ') {
        return None;
    }
    let section = skip_spaces(b, after);
    let section = (section, trim_end(b, section, b.len()).max(section));
    Some(Hunk {
        old: parents[0],
        new,
        section,
    })
}

/// A context diff's range line (X-176), `*** a,b ****` before a hunk's old file's lines or
/// `--- c,d ----` before its new file's: whether it is the new file's, and how many lines the
/// range holds. `a` alone is one line, `0` none (an empty file's).
pub fn context_section(line: &str) -> Option<(bool, u32)> {
    let (new, range) = if let Some(r) = line.strip_prefix("*** ") {
        (false, r.strip_suffix(" ****")?)
    } else {
        (true, line.strip_prefix("--- ")?.strip_suffix(" ----")?)
    };
    let number = |s: &str| -> Option<u32> {
        (!s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    let lines = match range.split_once(',') {
        Some((a, b)) => (number(b)? + 1).saturating_sub(number(a)?).max(1),
        None => u32::from(number(range)? > 0),
    };
    Some((new, lines))
}

/// What a diff's header line says of the file after it (X-174).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Header<'a> {
    /// A new file's section starts, `diff --git a/x b/y` (the new name), a merge's `diff --cc x`
    /// or `diff --combined x`, or another `diff …` line (its last word).
    Diff(Option<&'a str>),
    /// `--- path` and `+++ path`: the old file's and the new one's, `None` for `/dev/null`. A
    /// context diff's old file is its `*** path`, and its `--- path` after that the new one's
    /// (X-176), which only the line before tells.
    From(Option<&'a str>),
    To(Option<&'a str>),
    /// `Index: path`.
    Index(&'a str),
}

/// The file `line` names, if it is a header's that does. A path's timestamp after a tab and the
/// quotes around it come off; a context diff's range (`--- 1,5 ----`, `*** 1,5 ****`) is none.
pub fn header(line: &str) -> Option<Header<'_>> {
    fn path(p: &str) -> &str {
        let p = p.split('\t').next().unwrap_or(p).trim_end();
        p.strip_prefix('"')
            .and_then(|p| p.strip_suffix('"'))
            .unwrap_or(p)
    }
    fn file(p: &str) -> Option<&str> {
        Some(path(p)).filter(|&p| p != "/dev/null" && !p.is_empty())
    }
    if let Some(rest) = line.strip_prefix("diff ") {
        if let Some(p) = rest
            .strip_prefix("--cc ")
            .or_else(|| rest.strip_prefix("--combined "))
        {
            return Some(Header::Diff(Some(path(p)).filter(|p| !p.is_empty())));
        }
        let new = match rest.strip_prefix("--git ") {
            // `a/x b/y`, either quoted when its name needs it
            Some(paths) => paths
                .rfind(" \"b/")
                .or_else(|| paths.rfind(" b/"))
                .map_or_else(|| paths.rsplit(' ').next(), |i| Some(&paths[i + 1..])),
            None => rest.rsplit(' ').next(),
        };
        return Some(Header::Diff(new.map(path).filter(|p| !p.is_empty())));
    }
    if let Some(p) = line.strip_prefix("--- ") {
        return (!line.ends_with(" ----")).then(|| Header::From(file(p)));
    }
    if let Some(p) = line.strip_prefix("*** ") {
        return (!line.ends_with(" ****")).then(|| Header::From(file(p)));
    }
    if let Some(p) = line.strip_prefix("+++ ") {
        return Some(Header::To(file(p)));
    }
    let p = line.strip_prefix("Index: ")?;
    file(p).map(Header::Index)
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
    fn hunks() {
        use super::{Hunk, hunk};
        let h = |old, new, section| Some(Hunk { old, new, section });
        assert_eq!(hunk("@@ -1,3 +1,4 @@ fn main() {"), h(3, 4, (16, 27)));
        assert_eq!(hunk("@@ -5 +5 @@"), h(1, 1, (11, 11)));
        assert_eq!(hunk("@@ -0,0 +1,12 @@"), h(0, 12, (16, 16)));
        // a combined diff's, and what isn't a range
        assert_eq!(hunk("@@@ -1,2 -1,2 +1,3 @@@"), None);
        assert_eq!(hunk("@@ -1,2 @@"), None);
        assert_eq!(hunk("@@ -1,x +1 @@"), None);
        assert_eq!(hunk("@@ +1 -1 @@"), None);
    }

    #[test]
    fn combined_hunks() {
        use super::{Hunk, combined};
        let mut parents = Vec::new();
        let h = combined("@@@ -1,5 -1,4 +1,6 @@@ int main()", &mut parents);
        assert_eq!(
            h,
            Some(Hunk {
                old: 5,
                new: 6,
                section: (23, 33)
            })
        );
        assert_eq!(parents, [5, 4]);
        // an octopus merge's, a count left out
        let h = combined("@@@@ -3 -1,2 -0,0 +1,3 @@@@", &mut parents);
        assert_eq!(h.map(|h| (h.new, h.section)), Some((3, (27, 27))));
        assert_eq!(parents, [1, 2, 0]);
        // a parent's range short, a closer of another width, a unified hunk's
        assert_eq!(combined("@@@ -1,2 +1,3 @@@", &mut parents), None);
        assert_eq!(combined("@@@ -1,2 -1,2 +1,3 @@", &mut parents), None);
        assert_eq!(combined("@@@ -1,2 -1,2 +1,3 @@@@", &mut parents), None);
        assert_eq!(combined("@@ -1,2 +1,3 @@", &mut parents), None);
    }

    #[test]
    fn context_ranges() {
        use super::context_section as context_range;
        assert_eq!(context_range("*** 1,5 ****"), Some((false, 5)));
        assert_eq!(context_range("--- 12,13 ----"), Some((true, 2)));
        assert_eq!(context_range("--- 7 ----"), Some((true, 1)));
        assert_eq!(context_range("*** 0 ****"), Some((false, 0)));
        assert_eq!(context_range("*** a.c ****"), None);
        assert_eq!(context_range("--- 1,5"), None);
        assert_eq!(context_range("*** 1,5 ----"), None);
    }

    #[test]
    fn headers_name_files() {
        use super::{Header, header};
        assert_eq!(
            header("diff --git a/src/a.rs b/src/b.rs"),
            Some(Header::Diff(Some("b/src/b.rs")))
        );
        assert_eq!(
            header("diff --git \"a/x y.c\" \"b/x y.c\""),
            Some(Header::Diff(Some("b/x y.c")))
        );
        assert_eq!(
            header("diff -ru old/a.py new/a.py"),
            Some(Header::Diff(Some("new/a.py")))
        );
        assert_eq!(header("diff --cc a.c"), Some(Header::Diff(Some("a.c"))));
        assert_eq!(
            header("diff --combined \"x y.c\""),
            Some(Header::Diff(Some("x y.c")))
        );
        // a context diff's old file, its range none
        assert_eq!(
            header("*** a.c\t2024-01-01 10:00:00"),
            Some(Header::From(Some("a.c")))
        );
        assert_eq!(header("*** 1,5 ****"), None);
        assert_eq!(
            header("--- a/x.c\t2024-01-01 10:00:00"),
            Some(Header::From(Some("a/x.c")))
        );
        assert_eq!(header("--- /dev/null"), Some(Header::From(None)));
        assert_eq!(
            header("+++ b/Makefile"),
            Some(Header::To(Some("b/Makefile")))
        );
        assert_eq!(header("Index: src/a.c"), Some(Header::Index("src/a.c")));
        assert_eq!(header("--- 1,5 ----"), None);
        assert_eq!(header("---"), None);
        assert_eq!(header("+added"), None);
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
