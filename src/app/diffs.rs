//! A diff's files and commits (X-174): where `n`/`N` jump to before any search, and what `o`
//! lists in place of headings, each file with how it changed.

use crate::doc::code_lines;
use crate::layout::Layout;

/// A file in a diff, or a commit in `git log -p`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffFile {
    /// The line (from 1) it starts on: its `diff …` line (or `--- ` line, in a diff without
    /// them), or the commit's `commit <hash>` line.
    pub line: u32,
    /// 1 for a commit, and for a file when the diff has no commits; 2 for a commit's files.
    pub level: u8,
    /// The file's path, or a commit's short hash and subject.
    pub name: String,
    /// How a file changed: `added`, `renamed from …`, `binary`, and its lines `+12 -3`.
    pub note: Option<String>,
    pub commit: bool,
}

/// Every file and commit in a diff, in order. A file starts at each `diff …` line, or in a diff
/// without them at each `--- ` line with a `+++ ` line after it (a context diff's `*** ` and
/// `--- ` lines with its `***************` after them), outside a hunk. A commit starts at
/// each `commit <hash>` line, a `format-patch` mail's `From <hash>` and an annotated tag's
/// `tag <name>` (PGR-004). In a git diff with none of them (`git log -p --oneline`, a custom
/// `--format`), a commit's header is the text between files: its first line at the margin.
pub fn files(text: &str) -> Vec<DiffFile> {
    let lines: Vec<&str> = code_lines(text).collect();
    let git = lines.iter().any(|l| l.starts_with("diff "));
    // commits named by their header's first line, when no line names one as git's formats do
    let bare = !lines
        .iter()
        .enumerate()
        .any(|(i, l)| commit_header(l, lines.get(i + 1).copied()).is_some())
        && lines.iter().any(|l| is_git_file(l));
    let mut found = Vec::new();
    let mut file: Option<Parts> = None;
    // the commit still waiting for its subject, and where its header has it
    let mut subject: Option<(usize, Subject)> = None;
    let mut hunk = Hunk::None;
    // between files, where a bare commit's header is, and none named in it yet
    let mut gap = bare;
    for (i, &l) in lines.iter().enumerate() {
        let was = !matches!(hunk, Hunk::None);
        if hunk.takes(l, file.as_mut()) {
            continue;
        }
        let next = lines.get(i + 1).copied();
        if let Some((name, how)) = commit_header(l, next) {
            found.extend(file.take().map(Parts::finish));
            subject = Some((found.len(), how));
            found.push(DiffFile {
                line: i as u32 + 1,
                level: 1,
                name,
                note: None,
                commit: true,
            });
            continue;
        }
        // a hunk's end, or a file's header without hunks ending at a line that's no part of it
        gap |= bare
            && (was
                || file
                    .as_ref()
                    .is_some_and(|f| !f.hunks && !f.patch && !is_header(l)));
        let starts = if git {
            l.starts_with("diff ")
        } else {
            l.starts_with("--- ") && next.is_some_and(|n| n.starts_with("+++ "))
                || is_context_file(l, next, lines.get(i + 2).copied())
        };
        if gap && !starts && names_bare_commit(l) {
            found.extend(file.take().map(Parts::finish));
            gap = false;
            found.push(DiffFile {
                line: i as u32 + 1,
                level: 1,
                name: l.trim_end().to_owned(),
                note: None,
                commit: true,
            });
            continue;
        }
        if starts {
            found.extend(file.take().map(Parts::finish));
            subject = None;
            gap = false;
            let (header, prefixes) = match l.strip_prefix("diff --git ").and_then(one_path) {
                Some((a, b, path)) => (Some(path), [a, b]),
                None => {
                    let prefixes = if git { ["a/", "b/"] } else { ["", ""] };
                    (l.strip_prefix("diff ").map(header_path), prefixes)
                }
            };
            file = Some(Parts {
                line: i as u32 + 1,
                header,
                prefixes: prefixes.map(str::to_owned),
                ..Parts::default()
            });
        }
        let Some(f) = &mut file else {
            if let Some((at, how)) = &mut subject
                && let Some(text) = how.subject(l)
            {
                found[*at].name = format!("{} {text}", found[*at].name);
                subject = None;
            }
            continue;
        };
        // a context diff's file header: the old file's `*** ` line, then the new one's `--- `
        if is_context_file(l, next, lines.get(i + 2).copied()) {
            f.context = true;
        }
        if f.context
            && let Some(p) = l.strip_prefix("*** ")
        {
            f.old = Some(clean(p, &f.prefixes[0]));
        } else if f.context
            && let Some(p) = l.strip_prefix("--- ")
        {
            f.new = Some(clean(p, &f.prefixes[1]));
        } else if l == CONTEXT_HUNK && f.context {
            hunk = Hunk::Context { new: false };
            f.hunks = true;
        } else if let Some(p) = l.strip_prefix("--- ") {
            f.old = Some(clean(p, &f.prefixes[0]));
        } else if let Some(p) = l.strip_prefix("+++ ") {
            f.new = Some(clean(p, &f.prefixes[1]));
        } else if l.starts_with("@@") {
            hunk = Hunk::parse(l);
            f.hunks = true;
        } else if l.starts_with("new file mode") {
            f.added = true;
        } else if l.starts_with("deleted file mode") {
            f.deleted = true;
        } else if let Some(p) = l.strip_prefix("rename from ") {
            f.from = Some(("renamed", p.to_owned()));
        } else if let Some(p) = l.strip_prefix("copy from ") {
            f.from = Some(("copied", p.to_owned()));
        } else if let Some(p) = l
            .strip_prefix("rename to ")
            .or_else(|| l.strip_prefix("copy to "))
        {
            f.to = Some(p.to_owned());
        } else if let Some(mode) = l.strip_prefix("new mode ") {
            f.mode = Some(mode.to_owned());
        } else if l.starts_with("Binary files ") {
            f.binary = true;
        } else if l == "GIT binary patch" {
            (f.binary, f.patch) = (true, true);
        }
    }
    found.extend(file.map(Parts::finish));
    // a commit's files under it
    if found.iter().any(|f| f.commit) {
        for f in found.iter_mut().filter(|f| !f.commit) {
            f.level = 2;
        }
    }
    found
}

/// The rows `files` start on in `layout`, `None` past its end. Its rows run in source order, as
/// a diff's do, so one pass finds them all: each is what `Layout::row_for_line` would answer.
pub fn rows(layout: &Layout, files: &[DiffFile]) -> Vec<Option<u32>> {
    let rows = &layout.rows;
    let mut r = 0;
    files
        .iter()
        .map(|f| {
            while r < rows.len() && !(rows[r].src.contains(f.line) || rows[r].src.start > f.line) {
                r += 1;
            }
            (r < rows.len()).then_some(r as u32)
        })
        .collect()
}

/// The hash of a `commit <hash>` line, as `git log` writes it, decorations after it.
fn commit_hash(l: &str) -> Option<&str> {
    let hash = l.strip_prefix("commit ")?.split(' ').next()?;
    (hash.len() >= 7 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then_some(hash)
}

/// Where a commit's header has its subject.
#[derive(Clone, Copy)]
enum Subject {
    /// `git log`'s: its message's first line, indented.
    Indented,
    /// A mail's (`format-patch`): its `Subject:` line, without `[PATCH n/m]`.
    Mail,
    /// An annotated tag's: its message's first line, at the margin after the fields' blank
    /// line, once that has come.
    Tag { blank: bool },
}

impl Subject {
    /// The subject `l` holds, if it's the line of the header that has it.
    fn subject<'a>(&mut self, l: &'a str) -> Option<&'a str> {
        let text = match self {
            Subject::Indented => l.strip_prefix("    ")?.trim(),
            Subject::Mail => {
                let s = l.strip_prefix("Subject: ")?.trim();
                match s.strip_prefix('[').and_then(|s| s.split_once("] ")) {
                    Some((_, rest)) => rest.trim(),
                    None => s,
                }
            }
            Subject::Tag { blank: false } => {
                *self = Subject::Tag {
                    blank: l.is_empty(),
                };
                return None;
            }
            Subject::Tag { blank: true } => l.trim(),
        };
        (!text.is_empty()).then_some(text)
    }
}

/// A commit's or a tag's header line, as git's formats write it: what the outline names it
/// before its subject, and where the subject is. `next` is the line after it.
fn commit_header(l: &str, next: Option<&str>) -> Option<(String, Subject)> {
    if let Some(hash) = commit_hash(l) {
        return Some((hash[..7].to_owned(), Subject::Indented));
    }
    // `format-patch`'s `From <hash> Mon Sep 17 00:00:00 2001`, an mbox's first line
    if let Some(rest) = l.strip_prefix("From ") {
        let hash = rest.split(' ').next()?;
        let full = hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit());
        return full.then(|| (hash[..7].to_owned(), Subject::Mail));
    }
    let name = l.strip_prefix("tag ")?;
    let tagged = next.is_some_and(|n| n.starts_with("Tagger: ")) && !name.contains(' ');
    tagged.then(|| (format!("tag {name}"), Subject::Tag { blank: false }))
}

/// A git diff's file header: `diff --git`, or a merge's `diff --cc`, `diff --combined`.
fn is_git_file(l: &str) -> bool {
    ["diff --git ", "diff --cc ", "diff --combined "]
        .iter()
        .any(|d| l.starts_with(d))
}

/// A line that can name a bare commit, between files: one at the margin, not one of the
/// lines a diff has there (`--stat`'s indented, a mail signature's `-- `).
fn names_bare_commit(l: &str) -> bool {
    l.bytes()
        .next()
        .is_some_and(|b| !b.is_ascii_whitespace() && !b"-+@\\".contains(&b))
}

/// A line of a git file's extended header, after its `diff` line and before its hunks.
fn is_header(l: &str) -> bool {
    const HEADERS: [&str; 15] = [
        "index ",
        "mode ",
        "old mode ",
        "new mode ",
        "new file mode ",
        "deleted file mode ",
        "similarity index ",
        "dissimilarity index ",
        "rename from ",
        "rename to ",
        "copy from ",
        "copy to ",
        "--- ",
        "+++ ",
        "Binary files ",
    ];
    l.starts_with("@@") || HEADERS.iter().any(|h| l.starts_with(h))
}

/// The line that opens each of a context diff's (`diff -c`) hunks.
const CONTEXT_HUNK: &str = "***************";

/// A context diff's file header: its `*** old` line, `--- new` after it, then a hunk.
fn is_context_file(l: &str, next: Option<&str>, after: Option<&str>) -> bool {
    l.starts_with("*** ")
        && !l.ends_with(" ****")
        && next.is_some_and(|n| n.starts_with("--- "))
        && after == Some(CONTEXT_HUNK)
}

/// A path from a `--- ` or `+++ ` line: before a tab (a plain diff's timestamp), and without
/// the file's `prefix` (a git diff's `a/` or `b/`), inside quotes too.
fn clean(p: &str, prefix: &str) -> String {
    let p = p.split('\t').next().unwrap_or(p);
    if p == "/dev/null" {
        return p.to_owned();
    }
    match p.strip_prefix('"') {
        Some(q) => format!("\"{}", q.strip_prefix(prefix).unwrap_or(q)),
        None => p.strip_prefix(prefix).unwrap_or(p).to_owned(),
    }
}

/// A `diff --git` line's paths when they name one file, as git writes them for any change but
/// a rename or a copy: the prefixes its `--- ` and `+++ ` paths carry, and the path. Halves
/// alike as they stand are `diff.noprefix`'s; alike past a character and `/` each, git's `a/`
/// and `b/` or `diff.mnemonicPrefix`'s `c/`, `i/`, `w/`, `o/`. Quotes (`core.quotePath`) stay.
/// Split this way as a path with ` b/` in it can't be split at it.
fn one_path(paths: &str) -> Option<(&str, &str, String)> {
    let half = paths.len() / 2;
    if paths.len().is_multiple_of(2)
        || !paths.is_char_boundary(half)
        || paths.as_bytes()[half] != b' '
    {
        return None;
    }
    let (a, b) = (&paths[..half], &paths[half + 1..]);
    if a == b {
        return Some(("", "", a.to_owned()));
    }
    let (qa, qb) = (a.strip_prefix('"'), b.strip_prefix('"'));
    if qa.is_some() != qb.is_some() {
        return None;
    }
    let quote = if qa.is_some() { "\"" } else { "" };
    // a `/` second means the first character is one byte
    fn split(p: &str) -> Option<(&str, &str)> {
        (p.as_bytes().get(1) == Some(&b'/')).then(|| p.split_at(2))
    }
    let ((pa, ra), (pb, rb)) = (split(qa.unwrap_or(a))?, split(qb.unwrap_or(b))?);
    (ra == rb).then(|| (pa, pb, format!("{quote}{ra}")))
}

/// The path a `diff …` line names, for a file with no `--- `/`+++ ` lines (a mode change, a
/// binary file), when [`one_path`] can't tell it: `--git a/P b/Q`'s `Q`, `--cc P`, or a plain
/// diff's last word.
fn header_path(rest: &str) -> String {
    if let Some(paths) = rest.strip_prefix("--git ") {
        return paths
            .rsplit_once(" b/")
            .map_or(paths, |(_, b)| b)
            .to_owned();
    }
    for combined in ["--cc ", "--combined "] {
        if let Some(p) = rest.strip_prefix(combined) {
            return p.to_owned();
        }
    }
    rest.split_whitespace().last().unwrap_or(rest).to_owned()
}

/// What a file's header lines said, and its hunks' lines counted.
#[derive(Default)]
struct Parts {
    line: u32,
    header: Option<String>,
    /// What its `--- ` and `+++ ` paths start with: [`one_path`]'s, else `a/` and `b/` in a
    /// git diff.
    prefixes: [String; 2],
    old: Option<String>,
    new: Option<String>,
    added: bool,
    deleted: bool,
    /// `renamed` or `copied`, and from where.
    from: Option<(&'static str, String)>,
    to: Option<String>,
    mode: Option<String>,
    binary: bool,
    /// A `GIT binary patch`, whose data lines are no header's: a bare commit after it isn't told.
    patch: bool,
    /// Whether a hunk has started: a file without one ends at a line that's no header's.
    hunks: bool,
    /// A context diff's (`diff -c`): its old path on the `*** ` line, the new one on `--- `.
    context: bool,
    plus: u32,
    minus: u32,
}

impl Parts {
    fn finish(self) -> DiffFile {
        let null = |p: &Option<String>| p.as_deref() == Some("/dev/null");
        let (added, deleted) = (
            self.added || null(&self.old),
            self.deleted || null(&self.new),
        );
        let path = |p: Option<String>| p.filter(|p| p != "/dev/null");
        let name = self
            .to
            .or_else(|| path(self.new))
            .or_else(|| path(self.old))
            .or(self.header)
            .unwrap_or_default();
        let mut note = Vec::new();
        if added {
            note.push("added".to_owned());
        } else if deleted {
            note.push("deleted".to_owned());
        }
        if let Some((how, from)) = self.from {
            note.push(format!("{how} from {from}"));
        }
        if let Some(mode) = self.mode.filter(|_| !added && !deleted) {
            note.push(format!("mode {mode}"));
        }
        if self.binary {
            note.push("binary".to_owned());
        }
        let counts = match (self.plus, self.minus) {
            (0, 0) => String::new(),
            (p, 0) => format!("+{p}"),
            (0, m) => format!("-{m}"),
            (p, m) => format!("+{p} -{m}"),
        };
        if !counts.is_empty() {
            note.push(counts);
        }
        DiffFile {
            line: self.line,
            level: 1,
            name,
            note: (!note.is_empty()).then(|| note.join(" · ")),
            commit: false,
        }
    }
}

/// Where a hunk's lines are being read.
enum Hunk {
    None,
    /// The old and new lines it has left.
    Two {
        old: u32,
        new: u32,
    },
    /// A merge's combined hunk: a column of `+`, `-` or space for each parent.
    Combined {
        parents: usize,
    },
    /// A context diff's hunk, in its old file's lines or, past its `--- a,b ----`, the new
    /// one's: `- ` removed, `+ ` added, `! ` changed, two spaces of context.
    Context {
        new: bool,
    },
}

impl Hunk {
    /// A hunk's `@@ -a,b +c,d @@` line, or a combined one's `@@@ -a,b -c,d +e,f @@@`.
    fn parse(l: &str) -> Self {
        let ats = l.bytes().take_while(|&b| b == b'@').count();
        if ats > 2 {
            return Hunk::Combined { parents: ats - 1 };
        }
        match ada_parse::diff_hunk(l) {
            Some(h) => Hunk::Two {
                old: h.old,
                new: h.new,
            },
            None => Hunk::None,
        }
    }

    /// Whether `l` is one of the hunk's lines, counted into `file`; a line that can't be ends it.
    fn takes(&mut self, l: &str, file: Option<&mut Parts>) -> bool {
        let (plus, minus) = match self {
            Hunk::None => return false,
            Hunk::Two { old: 0, new: 0 } => {
                *self = Hunk::None;
                return false;
            }
            Hunk::Two { old, new } => match l.as_bytes().first() {
                Some(b'+') => {
                    *new = new.saturating_sub(1);
                    (true, false)
                }
                Some(b'-') => {
                    *old = old.saturating_sub(1);
                    (false, true)
                }
                // an empty line is an empty context line, as `diff.suppressBlankEmpty` writes it
                Some(b' ') | None => {
                    *old = old.saturating_sub(1);
                    *new = new.saturating_sub(1);
                    (false, false)
                }
                Some(b'\\') => (false, false),
                Some(_) => {
                    *self = Hunk::None;
                    return false;
                }
            },
            Hunk::Combined { parents } => match l.as_bytes().get(..*parents) {
                Some(cols) if cols.iter().all(|b| b" +-".contains(b)) => {
                    (cols.contains(&b'+'), cols.contains(&b'-'))
                }
                _ => {
                    *self = Hunk::None;
                    return false;
                }
            },
            Hunk::Context { new } => {
                let b = l.as_bytes();
                if l == CONTEXT_HUNK || l.starts_with("*** ") && l.ends_with(" ****") {
                    *new = false;
                    (false, false)
                } else if l.starts_with("--- ") && l.ends_with(" ----") {
                    *new = true;
                    (false, false)
                } else {
                    match (b.first(), b.get(1)) {
                        (None, _) | (Some(b' '), Some(b' ')) => (false, false),
                        (Some(b'-'), Some(b' ')) if !*new => (false, true),
                        (Some(b'+'), Some(b' ')) if *new => (true, false),
                        (Some(b'!'), Some(b' ')) => (*new, !*new),
                        _ => {
                            *self = Hunk::None;
                            return false;
                        }
                    }
                }
            }
        };
        if let Some(f) = file {
            f.plus += u32::from(plus);
            f.minus += u32::from(minus);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<u32> {
        files(text).iter().map(|f| f.line).collect()
    }

    fn listed(text: &str) -> Vec<(u8, String, Option<String>)> {
        files(text)
            .into_iter()
            .map(|f| (f.level, f.name, f.note))
            .collect()
    }

    #[test]
    fn starts_by_kind_of_diff() {
        let git =
            "commit 0123abc\n\ndiff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n--- a\n+++ b\n";
        assert_eq!(lines(git), [1, 3]);
        let plain = "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-x\n+y\n--- a/y\t2026\n+++ b/y\t2026\n";
        assert_eq!(lines(plain), [1, 6]);
        assert_eq!(lines("commit message\n"), Vec::<u32>::new());
    }

    /// A removed `-- comment` line reads `--- comment`: inside a hunk, it starts no file.
    #[test]
    fn a_hunks_lines_start_no_file() {
        let plain = "--- x.lua\n+++ x.lua\n@@ -1,2 +1,2 @@\n--- old\n++++ new\n x\n--- y.lua\n+++ y.lua\n@@ -1 +1 @@\n-a\n+b\n";
        assert_eq!(lines(plain), [1, 7]);
        assert_eq!(
            listed(plain),
            [
                (1, "x.lua".into(), Some("+1 -1".into())),
                (1, "y.lua".into(), Some("+1 -1".into()))
            ]
        );
    }

    #[test]
    fn files_say_how_they_changed() {
        let git = "\
commit 0123abcdef0123abcdef (HEAD -> main)
Author: A <a@b>
Date:   today

    Fix the thing

    More words.

diff --git a/src/a.rs b/src/a.rs
index 1..2 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,3 +1,4 @@
 a
-b
+c
+d

diff --git a/new.txt b/new.txt
new file mode 100644
--- /dev/null
+++ b/new.txt
@@ -0,0 +1 @@
+hi
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
--- a/gone.txt
+++ /dev/null
@@ -1 +0,0 @@
-bye
diff --git a/old name.rs b/new name.rs
similarity index 100%
rename from old name.rs
rename to new name.rs
diff --git a/run.sh b/run.sh
old mode 100644
new mode 100755
diff --git a/logo.png b/logo.png
index 1..2 100644
Binary files a/logo.png and b/logo.png differ
commit 4567abc
Author: A <a@b>
Date:   yesterday

diff --git a/b c/d b c/d
--- a/b c/d
+++ b/b c/d
@@ -1 +1 @@
-x
+y
";
        assert_eq!(
            listed(git),
            [
                (1, "0123abc Fix the thing".into(), None),
                (2, "src/a.rs".into(), Some("+2 -1".into())),
                (2, "new.txt".into(), Some("added · +1".into())),
                (2, "gone.txt".into(), Some("deleted · -1".into())),
                (
                    2,
                    "new name.rs".into(),
                    Some("renamed from old name.rs".into())
                ),
                (2, "run.sh".into(), Some("mode 100755".into())),
                (2, "logo.png".into(), Some("binary".into())),
                (1, "4567abc".into(), None),
                (2, "b c/d".into(), Some("+1 -1".into())),
            ]
        );
    }

    /// PGR-004: `git log -p --oneline` or a custom `--format`: a commit is the text between
    /// files, named by its first line; after a file without hunks too. A `--stat` without
    /// commits names none.
    #[test]
    fn bare_commits_are_the_text_between_files() {
        let oneline = "\
32450bc (HEAD -> x) fix: one
diff --git a/a b/a
--- a/a
+++ b/a
@@ -1,2 +1,2 @@
-x
+y
 z
d9843f2 feat: two
diff --git a/b b/b
new file mode 100644
index 0000000..e69de29
abc1234 docs: empty
---
 c | 2 +-

diff --git a/c b/c
--- a/c
+++ b/c
@@ -1 +1 @@
-x
+y
@@ -9 +9 @@
-x
+y
";
        assert_eq!(
            listed(oneline),
            [
                (1, "32450bc (HEAD -> x) fix: one".into(), None),
                (2, "a".into(), Some("+1 -1".into())),
                (1, "d9843f2 feat: two".into(), None),
                (2, "b".into(), Some("added".into())),
                (1, "abc1234 docs: empty".into(), None),
                (2, "c".into(), Some("+2 -2".into())),
            ]
        );
        let stat = " a | 2 +-\n 1 file changed\n\ndiff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-x\n+y\n";
        assert_eq!(listed(stat), [(1, "a".into(), Some("+1 -1".into()))]);
    }

    /// PGR-004: a `format-patch` series' mails by their subjects, a signature between them; an
    /// annotated tag's header by its name and message, its commit after it.
    #[test]
    fn mails_and_tags_are_commits() {
        let mails = "\
From 0123456789abcdef0123456789abcdef01234567 Mon Sep 17 00:00:00 2001
From: A <a@b>
Date: Thu, 19 Nov 2026 18:06:31 +0100
Subject: [PATCH 1/2] fix: the thing

Body.
---
 x | 1 +

diff --git a/x b/x
--- a/x
+++ b/x
@@ -0,0 +1 @@
+x
--
2.56.1

From 89abcdef0123456789abcdef0123456789abcdef Mon Sep 17 00:00:00 2001
From: A <a@b>
Subject: no prefix

diff --git a/y b/y
--- a/y
+++ b/y
@@ -1 +0,0 @@
-y
";
        assert_eq!(
            listed(mails),
            [
                (1, "0123456 fix: the thing".into(), None),
                (2, "x".into(), Some("+1".into())),
                (1, "89abcde no prefix".into(), None),
                (2, "y".into(), Some("-1".into())),
            ]
        );
        let tag = "\
tag v1.0
Tagger: A <a@b>
Date:   today

Release one

-----BEGIN PGP SIGNATURE-----
commit 0123abcdef
Author: A <a@b>

    Last fix

diff --git a/x b/x
--- a/x
+++ b/x
@@ -1 +1 @@
-a
+b
";
        assert_eq!(
            listed(tag),
            [
                (1, "tag v1.0 Release one".into(), None),
                (1, "0123abc Last fix".into(), None),
                (2, "x".into(), Some("+1 -1".into())),
            ]
        );
    }

    /// PGR-004: a context diff's (`diff -c`) files, `!` lines counted on both sides.
    #[test]
    fn context_diffs_list_their_files() {
        let context = "\
*** a/x.txt\t2026-11-19 18:05:33
--- b/x.txt\t2026-11-19 18:05:34
***************
*** 1,3 ****
  one
! two
- gone
--- 1,3 ----
  one
! TWO
+ new
***************
*** 9 ****
--- 9,10 ----
+ more
*** a/y.txt\t2026-11-19
--- b/y.txt\t2026-11-19
***************
*** 1 ****
! y
--- 1 ----
! Y
";
        assert_eq!(
            listed(context),
            [
                (1, "b/x.txt".into(), Some("+3 -2".into())),
                (1, "b/y.txt".into(), Some("+1 -1".into())),
            ]
        );
        assert_eq!(lines(context), [1, 16]);
    }

    #[test]
    fn a_merges_combined_hunks_are_counted() {
        let cc = "diff --cc x.rs\nindex 1,2..3\n--- a/x.rs\n+++ b/x.rs\n@@@ -1,2 -1,2 +1,2 @@@\n  a\n- b\n +c\n++d\n";
        assert_eq!(listed(cc), [(1, "x.rs".into(), Some("+2 -1".into()))]);
    }

    #[test]
    fn header_paths() {
        let one = one_path;
        assert_eq!(one("a/x y b/x y"), Some(("a/", "b/", "x y".into())));
        assert_eq!(one("a/x b/y"), None);
        // diff.noprefix: no characters lost
        assert_eq!(one("build.sh build.sh"), Some(("", "", "build.sh".into())));
        assert_eq!(
            one("src/lib.rs src/lib.rs"),
            Some(("", "", "src/lib.rs".into()))
        );
        // diff.mnemonicPrefix
        assert_eq!(one("i/x.rs w/x.rs"), Some(("i/", "w/", "x.rs".into())));
        // core.quotePath
        let quoted = r#""a/caf\303\251" "b/caf\303\251""#;
        assert_eq!(one(quoted), Some(("a/", "b/", r#""caf\303\251""#.into())));
        assert_eq!(header_path("--git a/x b/y"), "y");
        assert_eq!(header_path("--cc x.rs"), "x.rs");
        assert_eq!(header_path("-ruN old/x new/x"), "new/x");
    }

    /// `diff.noprefix` and `diff.mnemonicPrefix`: a file's paths lose the prefixes its
    /// `diff --git` line shows, header-only files and others alike.
    #[test]
    fn other_prefixes_name_their_files() {
        let noprefix = "\
diff --git src/lib.rs src/lib.rs
new file mode 100644
index 0000000..e69de29
diff --git build.sh build.sh
old mode 100644
new mode 100755
diff --git a/x.rs a/x.rs
--- a/x.rs
+++ a/x.rs
@@ -1 +1 @@
-a
+b
";
        let names: Vec<String> = listed(noprefix).into_iter().map(|f| f.1).collect();
        assert_eq!(names, ["src/lib.rs", "build.sh", "a/x.rs"]);
        let mnemonic = "\
diff --git i/main.rs w/main.rs
--- i/main.rs
+++ w/main.rs
@@ -1 +1 @@
-a
+b
diff --git c/img.png w/img.png
Binary files c/img.png and w/img.png differ
";
        let names: Vec<String> = listed(mnemonic).into_iter().map(|f| f.1).collect();
        assert_eq!(names, ["main.rs", "img.png"]);
    }
}
