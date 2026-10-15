//! The git gutter (ADR-06, architecture §8). Two diffs whose `+` side is the working tree, so
//! their line numbers agree: `git diff HEAD` (every change) and `git diff` (the unstaged ones).
//! A line marked by the unstaged diff is unstaged; one only the total diff marks is staged.
//!
//! [`run`] reads the outside world and runs in the effect runner; its raw diff text is the
//! recorded payload (ADR-19). Turning it into [`LineMarks`] is pure and happens in `update`.

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

use anpa::combinators::{map, right, succeed};
use anpa::core::{ParserExtNoState, StrParser};
use anpa::number::integer;
use anpa::parsers::skip;
use anpa::tuplify;

use crate::doc::LineRange;
use crate::layout::Layout;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// Lines were deleted after this one.
    Deleted,
    Added,
    Modified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// The index differs from `HEAD`, and the working tree matches the index.
    Staged,
    /// The working tree differs from the index.
    Unstaged,
}

/// Ordered by strength: unstaged beats staged, then modified beats added beats deleted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Mark {
    pub stage: Stage,
    pub kind: Kind,
}

impl Mark {
    /// The index of the mark's glyph and style, in the order of the `Gutter*` style keys.
    pub fn index(self) -> usize {
        let kind = match self.kind {
            Kind::Added => 0,
            Kind::Modified => 1,
            Kind::Deleted => 2,
        };
        match self.stage {
            Stage::Unstaged => kind,
            Stage::Staged => 3 + kind,
        }
    }
}

/// `@@ -a,b +c,d @@`: `b` old lines at `a` became `d` new lines at `c`. A missing count is 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Hunk {
    old: u32,
    new_start: u32,
    new: u32,
}

fn range<'a>() -> impl StrParser<'a, (u32, u32)> {
    tuplify!(
        integer(),
        map(succeed(right(skip(','), integer())), |n: Option<u32>| n
            .unwrap_or(1))
    )
}

fn hunk_header<'a>() -> impl StrParser<'a, Hunk> {
    map(
        tuplify!(right(skip("@@ -"), range()), right(skip(" +"), range())),
        |((_, old), (new_start, new))| Hunk {
            old,
            new_start,
            new,
        },
    )
}

/// The working-tree lines a `-U0` diff marks, by line, unsorted.
fn diff_marks(diff: &str) -> Vec<(u32, Kind)> {
    let mut out = Vec::new();
    for line in diff.lines().filter(|l| l.starts_with("@@ ")) {
        let Some(h) = hunk_header().parse(line).result else {
            continue;
        };
        let lines = |from: u32, n: u32| (from..from.saturating_add(n)).map(move |l| (l, from));
        if h.old == 0 {
            out.extend(lines(h.new_start, h.new).map(|(l, _)| (l, Kind::Added)));
        } else if h.new == 0 {
            // deleted after line `new_start`; before the first line when that is 0
            out.push((h.new_start.max(1), Kind::Deleted));
        } else {
            let modified = h.old.min(h.new);
            out.extend(lines(h.new_start, modified).map(|(l, _)| (l, Kind::Modified)));
            let extra = h.new - modified;
            out.extend(lines(h.new_start + modified, extra).map(|(l, _)| (l, Kind::Added)));
        }
    }
    out
}

/// Which source lines changed, and how.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineMarks {
    /// One mark per line, sorted by line.
    marks: Vec<(u32, Mark)>,
    /// The file isn't tracked: every line is new.
    untracked: bool,
}

impl LineMarks {
    /// Combine the total diff (`git diff HEAD`) with the unstaged one (`git diff`).
    pub fn new(total: &str, unstaged: &str, untracked: bool) -> Self {
        let mut marks: Vec<(u32, Mark)> = Vec::new();
        let mut add = |diff: &str, stage| {
            for (line, kind) in diff_marks(diff) {
                marks.push((line, Mark { stage, kind }));
            }
        };
        add(total, Stage::Staged);
        add(unstaged, Stage::Unstaged);
        // the strongest mark of each line: unstaged wins, as the newest edit
        marks.sort_unstable_by_key(|&(line, mark)| (line, std::cmp::Reverse(mark)));
        marks.dedup_by_key(|&mut (line, _)| line);
        Self { marks, untracked }
    }

    pub fn is_empty(&self) -> bool {
        self.marks.is_empty() && !self.untracked
    }

    /// The strongest mark on `lines`.
    fn strongest(&self, lines: LineRange) -> Option<Mark> {
        if lines.is_none() {
            return None;
        }
        if self.untracked {
            return Some(Mark {
                stage: Stage::Unstaged,
                kind: Kind::Added,
            });
        }
        let from = self.marks.partition_point(|&(l, _)| l < lines.start);
        self.marks[from..]
            .iter()
            .take_while(|&&(l, _)| l <= lines.end)
            .map(|&(_, m)| m)
            .max()
    }

    /// Row `row`'s mark: the strongest mark on its lines. A deletion shows on the last row
    /// above the gap it left, where the glyph's low bar sits, even when the line it follows
    /// renders nothing (a blank line between blocks).
    pub fn row_mark(&self, layout: &Layout, row: u32) -> Option<Mark> {
        let rows = &layout.rows;
        let src = rows.get(row as usize)?.src;
        if self.untracked || src.is_none() {
            return self.strongest(src);
        }
        // This row owns the deletions after its first line, up to the next row's first line.
        let next = rows[row as usize + 1..]
            .iter()
            .map(|r| r.src)
            .find(|s| !s.is_none())
            .map_or(u32::MAX, |s| s.start);
        let owned = src.start..next.max(src.start);
        let last = src.end.max(owned.end.saturating_sub(1));
        let from = self.marks.partition_point(|&(l, _)| l < src.start);
        self.marks[from..]
            .iter()
            .take_while(|&&(l, _)| l <= last)
            .filter(|&&(l, m)| match m.kind {
                Kind::Deleted => owned.contains(&l),
                _ => l <= src.end,
            })
            .map(|&(_, m)| m)
            .max()
    }

    /// The rows where a run of changed rows starts, in order.
    pub fn change_starts(&self, layout: &Layout) -> Vec<u32> {
        let mut starts = Vec::new();
        let mut prev = None;
        for r in 0..layout.row_count() {
            let mark = self.row_mark(layout, r);
            if mark.is_some() && mark != prev {
                starts.push(r);
            }
            prev = mark;
        }
        starts
    }
}

/// What `git` says about a file. `None` from [`run`] means no gutter: not in a repository, or
/// git isn't there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diffs {
    pub total: String,
    pub unstaged: String,
    pub untracked: bool,
}

fn git(dir: &Path, args: &[&OsStr]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Run the two diffs for `path`, in parallel.
pub fn run(path: &Path) -> Option<Diffs> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let name = path.file_name()?;
    let os = |s: &'static str| OsStr::new(s);
    git(dir, &[os("rev-parse"), os("--is-inside-work-tree")])?;
    if git(
        dir,
        &[os("ls-files"), os("--error-unmatch"), os("--"), name],
    )
    .is_none()
    {
        return Some(Diffs {
            total: String::new(),
            unstaged: String::new(),
            untracked: true,
        });
    }
    let diff = [
        os("diff"),
        os("--no-color"),
        os("--no-ext-diff"),
        os("--no-textconv"),
        os("-U0"),
    ];
    fn with<'a>(diff: &[&'a OsStr], base: &'a OsStr, name: &'a OsStr) -> Vec<&'a OsStr> {
        let mut args = diff.to_vec();
        args.extend([base, OsStr::new("--"), name]);
        args
    }
    let (total, unstaged) = thread::scope(|s| {
        let unstaged = s.spawn(|| {
            let mut args = diff.to_vec();
            args.extend([os("--"), name]);
            git(dir, &args)
        });
        let total = git(dir, &with(&diff, os("HEAD"), name)).or_else(|| {
            // no commits yet: everything is new relative to the empty tree
            let empty = git(
                dir,
                &[os("hash-object"), os("-t"), os("tree"), os("/dev/null")],
            )?;
            git(dir, &with(&diff, OsStr::new(empty.trim()), name))
        });
        (total, unstaged.join().ok().flatten())
    });
    Some(Diffs {
        total: total.unwrap_or_default(),
        unstaged: unstaged.unwrap_or_default(),
        untracked: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(stage: Stage, kind: Kind) -> Mark {
        Mark { stage, kind }
    }

    #[test]
    fn hunk_headers() {
        let h = |s: &str| hunk_header().parse(s).result;
        assert_eq!(
            h("@@ -12,3 +12,0 @@ fn foo"),
            Some(Hunk {
                old: 3,
                new_start: 12,
                new: 0
            })
        );
        assert_eq!(
            h("@@ -5 +7 @@"),
            Some(Hunk {
                old: 1,
                new_start: 7,
                new: 1
            })
        );
        assert_eq!(h("@@@ -1 +1 @@@"), None);
    }

    #[test]
    fn hunks_become_marks() {
        let diff = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -0,0 +1,2 @@\n+a\n+b\n@@ -5,1 +7,3 @@\n-x\n+y\n+z\n+w\n@@ -9,2 +11,0 @@\n-p\n-q\n@@ -20,2 +20 @@\n-1\n-2\n+3\n@@ -30,1 +0,0 @@\n";
        let mut marks = diff_marks(diff);
        marks.sort_unstable();
        use Kind::*;
        assert_eq!(
            marks,
            [
                (1, Deleted),
                (1, Added),
                (2, Added),
                (7, Modified),
                (8, Added),
                (9, Added),
                (11, Deleted),
                (20, Modified)
            ]
        );
    }

    #[test]
    fn unstaged_beats_staged() {
        let total = "@@ -1 +1 @@\n@@ -0,0 +5,2 @@\n";
        let unstaged = "@@ -0,0 +6 @@\n@@ -9 +8,0 @@\n";
        let lm = LineMarks::new(total, unstaged, false);
        use Kind::*;
        use Stage::*;
        assert_eq!(
            lm.marks,
            [
                (1, m(Staged, Modified)),
                (5, m(Staged, Added)),
                (6, m(Unstaged, Added)),
                (8, m(Unstaged, Deleted)),
            ]
        );
        assert_eq!(lm.strongest(LineRange::new(5, 6)), Some(m(Unstaged, Added)));
        assert_eq!(lm.strongest(LineRange::new(2, 4)), None);
        assert_eq!(lm.strongest(LineRange::NONE), None);
        assert_eq!(m(Staged, Deleted).index(), 5);
        assert_eq!(m(Unstaged, Modified).index(), 1);
    }

    #[test]
    fn rows_get_marks_and_deletions_sit_on_the_last_row() {
        let theme = crate::theme::Theme::builtin();
        let doc = crate::doc::parse("alpha beta gamma delta\n\nnext\n");
        let params = crate::layout::LayoutParams {
            width: 20,
            max_width: None,
            table_mode: Default::default(),
            code_wrap: true,
            glyphs_gen: 0,
            images: None,
        };
        let layout = crate::layout::layout(&doc, &theme.glyphs, params);
        let rows: Vec<_> = layout.rows.iter().map(|r| r.src).collect();
        // alpha beta gamma / delta / (blank) / next
        let line = LineRange::line;
        assert_eq!(rows, [line(1), line(1), LineRange::NONE, line(3)]);
        let lm = LineMarks::new("@@ -2 +1,0 @@\n@@ -0,0 +3 @@\n", "", false);
        let marks: Vec<_> = (0..layout.row_count())
            .map(|r| lm.row_mark(&layout, r).map(|m| m.kind))
            .collect();
        assert_eq!(marks, [None, Some(Kind::Deleted), None, Some(Kind::Added)]);
        assert_eq!(lm.change_starts(&layout), [1, 3]);

        // a deletion after the blank line 2, which no row renders, lands above the gap
        let lm = LineMarks::new("@@ -3 +2,0 @@\n", "", false);
        let marks: Vec<_> = (0..layout.row_count())
            .map(|r| lm.row_mark(&layout, r).map(|m| m.kind))
            .collect();
        assert_eq!(marks, [None, Some(Kind::Deleted), None, None]);
    }

    #[test]
    fn untracked_marks_everything() {
        let lm = LineMarks::new("", "", true);
        assert!(!lm.is_empty());
        assert_eq!(
            lm.strongest(LineRange::line(3)).map(|m| m.kind),
            Some(Kind::Added)
        );
        assert!(LineMarks::new("", "", false).is_empty());
    }

    /// A real repository: a staged edit, then an unstaged one on top.
    #[test]
    fn staged_and_unstaged_in_a_real_repo() {
        let dir = std::env::temp_dir().join(format!("preen-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sh = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args([
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .output()
                .is_ok_and(|o| o.status.success())
        };
        if !sh(&["init", "-q"]) {
            return; // no git here
        }
        let file = dir.join("note.md");
        std::fs::write(dir.join("other.md"), "x\n").unwrap();
        assert_eq!(
            run(&dir.join("other.md")),
            Some(Diffs {
                total: String::new(),
                unstaged: String::new(),
                untracked: true
            })
        );
        std::fs::write(&file, "a\nb\nc\nd\n").unwrap();
        assert!(sh(&["add", "note.md"]));
        assert!(sh(&["commit", "-q", "-m", "init"]));
        std::fs::write(&file, "a\nB\nc\nd\n").unwrap();
        assert!(sh(&["add", "note.md"]));
        std::fs::write(&file, "a\nB\nc\nD\ne\n").unwrap();
        let diffs = run(&file).expect("in a repo");
        let lm = LineMarks::new(&diffs.total, &diffs.unstaged, diffs.untracked);
        use Kind::*;
        use Stage::*;
        assert_eq!(
            lm.marks,
            [
                (2, m(Staged, Modified)),
                (4, m(Unstaged, Modified)),
                (5, m(Unstaged, Added))
            ]
        );
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(run(Path::new("/")), None, "not a repository");
    }
}
