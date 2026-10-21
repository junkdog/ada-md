//! Links found by name under a root (ADR-13, as amended 2026-10-21). A `[[wiki]]` name, or a
//! link that is a bare file name, which isn't beside its note is looked up in the whole tree of
//! the note's root: the nearest directory above it that the config lists in `[links] roots`, or
//! that holds one of `root_markers` (`.obsidian` by default). That is how Obsidian resolves
//! links, and it keeps links working when notes move between folders, PARA categories
//! included, without ada knowing about either.
//!
//! The tree is indexed by file name on the first lookup that needs it, and walked again on a
//! miss once the index is a little old, so a note created during the session is found.

use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{BuiltinResolver, LocalTarget, Resolved, Resolver, is_markdown};

/// Where links may be looked up by name: `[links]` in the config.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Roots {
    /// Directories whose trees are searched, absolute.
    pub roots: Vec<PathBuf>,
    /// Names of files or directories that make the directory holding them a root.
    pub markers: Vec<String>,
}

impl Default for Roots {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            markers: vec![".obsidian".into()],
        }
    }
}

/// `~` and `~/…` as under `$HOME`; other paths as given.
pub fn expand_home(path: &str) -> PathBuf {
    let home = || env::var_os("HOME").map(PathBuf::from);
    match path.strip_prefix('~') {
        Some("") => home().unwrap_or_else(|| path.into()),
        Some(rest) if rest.starts_with('/') => match home() {
            Some(home) => home.join(&rest[1..]),
            None => path.into(),
        },
        _ => path.into(),
    }
}

/// Files a walk indexes at most: a root marker high up (`~/.obsidian`) shouldn't index a disk.
const MAX_FILES: usize = 200_000;

/// How old an index must be before a miss walks the tree again.
const REFRESH: Duration = Duration::from_secs(2);

/// The built-in rules, then a lookup by name under the note's root.
#[derive(Debug)]
pub struct RootedResolver {
    /// The configured roots, canonical.
    roots: Vec<PathBuf>,
    markers: Vec<String>,
    refresh: Duration,
    indexes: RefCell<Vec<Index>>,
}

#[derive(Debug)]
struct Index {
    root: PathBuf,
    built: Instant,
    /// Lowercased file name → the files of that name, sorted.
    names: HashMap<String, Vec<PathBuf>>,
}

impl RootedResolver {
    pub fn new(roots: &Roots) -> Self {
        Self {
            roots: roots
                .roots
                .iter()
                .filter_map(|r| fs::canonicalize(r).ok())
                .collect(),
            markers: roots.markers.clone(),
            refresh: REFRESH,
            indexes: RefCell::default(),
        }
    }

    /// The nearest directory at or above `dir` (canonical) that is a root.
    fn root_of(&self, dir: &Path) -> Option<PathBuf> {
        dir.ancestors()
            .find(|a| {
                self.roots.iter().any(|r| r == a) || self.markers.iter().any(|m| a.join(m).exists())
            })
            .map(Path::to_path_buf)
    }

    /// The files under `root` named `name` (ignoring case), walking the tree first if it has no
    /// index yet, or, when `fresh` is asked for, if its index is older than [`REFRESH`].
    fn named(&self, root: &Path, name: &str, fresh: bool) -> Vec<PathBuf> {
        let mut indexes = self.indexes.borrow_mut();
        let at = match indexes.iter().position(|i| i.root == root) {
            Some(at) => {
                if fresh && indexes[at].built.elapsed() >= self.refresh {
                    indexes[at] = Index::build(root);
                }
                at
            }
            None => {
                indexes.push(Index::build(root));
                indexes.len() - 1
            }
        };
        let index = &indexes[at];
        let mut found = index
            .names
            .get(&name.to_lowercase())
            .cloned()
            .unwrap_or_default();
        // an exact match wins over ones that differ in case
        if found.len() > 1 && found.iter().any(|p| file_name(p) == name) {
            found.retain(|p| file_name(p) == name);
        }
        found
    }

    /// Look `target` up under the root of `source`, if it names a file rather than a path.
    fn lookup(&self, source: &Path, target: &LocalTarget) -> Option<Resolved> {
        let (dirs, names): (&str, Vec<String>) = match target {
            LocalTarget::Wiki(name) => {
                let (dirs, base) = name.rsplit_once('/').unwrap_or(("", name));
                let names = if is_markdown(Path::new(base)) {
                    vec![base.to_owned()]
                } else {
                    vec![format!("{base}.md"), base.to_owned()]
                };
                (dirs, names)
            }
            // a path with directories in it says where the file is
            LocalTarget::Path(p) if !p.contains('/') && !matches!(&**p, "" | "." | "..") => {
                ("", vec![p.clone()])
            }
            LocalTarget::Path(_) => return None,
        };
        if names.iter().any(String::is_empty) {
            return None;
        }
        let dir = source.parent().filter(|d| !d.as_os_str().is_empty());
        let dir = dir.unwrap_or(Path::new("."));
        let canonical = fs::canonicalize(dir).ok()?;
        let root = self.root_of(&canonical)?;
        let dirs = Path::new(dirs);
        let under = |p: &PathBuf| p.parent().is_some_and(|d| d.ends_with(dirs));
        // the first name that matches anything, and what it matches
        let find = |fresh| {
            names.iter().find_map(|n| {
                let mut found = self.named(&root, n, fresh);
                found.retain(under);
                (!found.is_empty()).then_some((n, found))
            })
        };
        let (name, found) = find(false).or_else(|| find(true))?;
        Some(match &found[..] {
            [] => return None,
            [one] => Resolved::Found(rebase(one, dir, &canonical)),
            many => {
                // a project's folder under another category, as `10-projects/x` → `40-archive/x`
                let near: Vec<_> = many
                    .iter()
                    .filter(|p| p.parent().and_then(Path::file_name) == canonical.file_name())
                    .collect();
                match near[..] {
                    [one] => Resolved::Found(rebase(one, dir, &canonical)),
                    _ => Resolved::Broken(ambiguous(name, &root, many)),
                }
            }
        })
    }
}

impl Resolver for RootedResolver {
    fn resolve(&self, source: &Path, target: &LocalTarget) -> Resolved {
        match BuiltinResolver.resolve(source, target) {
            Resolved::Broken(why) => self.lookup(source, target).unwrap_or(Resolved::Broken(why)),
            found => found,
        }
    }
}

impl Index {
    fn build(root: &Path) -> Index {
        let mut names: HashMap<String, Vec<PathBuf>> = HashMap::new();
        let mut files = 0;
        let mut dirs = vec![root.to_path_buf()];
        'walk: while let Some(dir) = dirs.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                // `.git`, `.obsidian`, `.trash` and the like
                if name.starts_with('.') {
                    continue;
                }
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                let path = entry.path();
                // symlinked directories aren't followed: no loops, no trees outside the root
                if kind.is_dir() {
                    dirs.push(path);
                    continue;
                }
                if !(kind.is_file() || (kind.is_symlink() && path.is_file())) {
                    continue;
                }
                names.entry(name.to_lowercase()).or_default().push(path);
                files += 1;
                if files == MAX_FILES {
                    break 'walk;
                }
            }
        }
        for paths in names.values_mut() {
            paths.sort();
        }
        Index {
            root: root.to_path_buf(),
            built: Instant::now(),
            names,
        }
    }
}

fn file_name(path: &Path) -> &str {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("")
}

/// `found` under `dir` as the note was reached, when it is below the note's directory
/// (`canonical`): a note opened through a symlink keeps its neighbors behind the same link.
fn rebase(found: &Path, dir: &Path, canonical: &Path) -> PathBuf {
    match found.strip_prefix(canonical) {
        Ok(rest) => dir.join(rest),
        Err(_) => found.to_path_buf(),
    }
}

/// Why a name that matches several files isn't followed: the first few, relative to the root.
fn ambiguous(name: &str, root: &Path, many: &[PathBuf]) -> String {
    const SHOWN: usize = 3;
    let mut list = many
        .iter()
        .take(SHOWN)
        .map(|p| p.strip_prefix(root).unwrap_or(p).display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    if many.len() > SHOWN {
        list.push_str(", …");
    }
    format!(
        "{} files named {name} under {}: {list}",
        many.len(),
        root.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tree of files in a fresh directory; `.keep` entries make empty directories.
    fn tree(name: &str, files: &[&str]) -> PathBuf {
        let dir = env::temp_dir().join(format!("ada-roots-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for f in files {
            let path = dir.join(f);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            if !f.ends_with(".keep") {
                fs::write(&path, "x").unwrap();
            }
        }
        fs::canonicalize(dir).unwrap()
    }

    fn rooted(roots: &[&Path]) -> RootedResolver {
        RootedResolver::new(&Roots {
            roots: roots.iter().map(|r| r.to_path_buf()).collect(),
            ..Roots::default()
        })
    }

    fn wiki(s: &str) -> LocalTarget {
        LocalTarget::Wiki(s.into())
    }

    fn path(s: &str) -> LocalTarget {
        LocalTarget::Path(s.into())
    }

    #[test]
    fn a_note_moved_to_another_category_is_found() {
        let dir = tree(
            "para",
            &[
                "10-projects/ada/plan.md",
                "40-archive/ada/old-plan.md",
                "10-projects/oxasm/oxasm-port.md",
                "30-resources/pics/cat.png",
            ],
        );
        let r = rooted(&[&dir]);
        let source = dir.join("10-projects/ada/plan.md");
        assert_eq!(
            r.resolve(&source, &wiki("old-plan")),
            Resolved::Found(dir.join("40-archive/ada/old-plan.md"))
        );
        assert_eq!(
            r.resolve(&source, &wiki("oxasm-port")),
            Resolved::Found(dir.join("10-projects/oxasm/oxasm-port.md"))
        );
        // `![[cat.png]]` and `![](cat.png)` name a file, so they're looked up too
        assert_eq!(
            r.resolve(&source, &path("cat.png")),
            Resolved::Found(dir.join("30-resources/pics/cat.png"))
        );
        // a path with directories in it is where it says
        let Resolved::Broken(why) = r.resolve(&source, &path("pics/cat.png")) else {
            panic!("followed a path that isn't there");
        };
        assert!(why.starts_with("no such file"), "{why}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn outside_a_root_only_the_builtin_rules_apply() {
        let dir = tree("outside", &["a/note.md", "b/other.md"]);
        let source = dir.join("a/note.md");
        for r in [rooted(&[]), rooted(&[&dir.join("a")])] {
            assert_eq!(
                r.resolve(&source, &wiki("other")),
                BuiltinResolver.resolve(&source, &wiki("other"))
            );
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_marker_makes_a_root() {
        let dir = tree(
            "marker",
            &[
                "vault/.obsidian/.keep",
                "vault/a/note.md",
                "vault/b/other.md",
            ],
        );
        let r = rooted(&[]);
        assert_eq!(
            r.resolve(&dir.join("vault/a/note.md"), &wiki("other")),
            Resolved::Found(dir.join("vault/b/other.md"))
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_nearest_root_wins() {
        let dir = tree(
            "nearest",
            &[
                "outer/inner/.obsidian/.keep",
                "outer/inner/a/note.md",
                "outer/b/other.md",
            ],
        );
        let r = rooted(&[&dir.join("outer")]);
        let Resolved::Broken(_) = r.resolve(&dir.join("outer/inner/a/note.md"), &wiki("other"))
        else {
            panic!("looked past the nearest root");
        };
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn several_matches() {
        let dir = tree(
            "ambiguous",
            &[
                "10-projects/sift/plan.md",
                "30-resources/sift/notes.md",
                "40-archive/oxasm/notes.md",
                "10-projects/x/todo.md",
                "20-areas/y/todo.md",
                "20-areas/y/sub/todo.md",
                "10-projects/x/arch.puml",
                "20-areas/y/arch.puml",
            ],
        );
        let r = rooted(&[&dir]);
        let source = dir.join("10-projects/sift/plan.md");
        // one in a folder named like the note's
        assert_eq!(
            r.resolve(&source, &wiki("notes")),
            Resolved::Found(dir.join("30-resources/sift/notes.md"))
        );
        let Resolved::Broken(why) = r.resolve(&source, &wiki("todo")) else {
            panic!("picked one of three");
        };
        assert_eq!(
            why,
            format!(
                "3 files named todo.md under {}: 10-projects/x/todo.md, 20-areas/y/sub/todo.md, 20-areas/y/todo.md",
                dir.display()
            )
        );
        // the message names what matched: `arch.puml`, not `arch.puml.md`
        let Resolved::Broken(why) = r.resolve(&source, &wiki("arch.puml")) else {
            panic!("picked one of two");
        };
        assert!(why.starts_with("2 files named arch.puml under"), "{why}");
        // the end of a path tells them apart
        assert_eq!(
            r.resolve(&source, &wiki("y/todo")),
            Resolved::Found(dir.join("20-areas/y/todo.md"))
        );
        assert_eq!(
            r.resolve(&source, &wiki("y/sub/todo.md")),
            Resolved::Found(dir.join("20-areas/y/sub/todo.md"))
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn names_match_ignoring_case_and_hidden_folders_are_skipped() {
        let dir = tree("case", &["a/note.md", "b/Other Note.md", ".trash/gone.md"]);
        let r = rooted(&[&dir]);
        let source = dir.join("a/note.md");
        assert_eq!(
            r.resolve(&source, &wiki("other note")),
            Resolved::Found(dir.join("b/Other Note.md"))
        );
        let Resolved::Broken(_) = r.resolve(&source, &wiki("gone")) else {
            panic!("found a note in a hidden folder");
        };
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_new_note_is_found_once_the_index_is_old() {
        let dir = tree("fresh", &["a/note.md"]);
        let mut r = rooted(&[&dir]);
        let source = dir.join("a/note.md");
        let Resolved::Broken(_) = r.resolve(&source, &wiki("later")) else {
            panic!("found a note before it was written");
        };
        fs::create_dir_all(dir.join("b")).unwrap();
        fs::write(dir.join("b/later.md"), "x").unwrap();
        // within REFRESH of the last walk, a miss doesn't walk again
        let Resolved::Broken(_) = r.resolve(&source, &wiki("later")) else {
            panic!("walked the tree again at once");
        };
        r.refresh = Duration::ZERO;
        assert_eq!(
            r.resolve(&source, &wiki("later")),
            Resolved::Found(dir.join("b/later.md"))
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_note_reached_through_a_symlink_keeps_its_neighbors_behind_it() {
        let dir = tree(
            "symlink",
            &[
                "vault/p/ada/plan.md",
                "vault/p/ada/sub/deep.md",
                "vault/q/far.md",
            ],
        );
        std::os::unix::fs::symlink(dir.join("vault/p/ada"), dir.join("docs")).unwrap();
        let r = rooted(&[&dir.join("vault")]);
        let source = dir.join("docs/plan.md");
        assert_eq!(
            r.resolve(&source, &wiki("deep")),
            Resolved::Found(dir.join("docs/sub/deep.md"))
        );
        assert_eq!(
            r.resolve(&source, &wiki("far")),
            Resolved::Found(dir.join("vault/q/far.md"))
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn home_expands() {
        let Some(home) = env::var_os("HOME").map(PathBuf::from) else {
            return;
        };
        assert_eq!(expand_home("~"), home);
        assert_eq!(expand_home("~/vault"), home.join("vault"));
        assert_eq!(expand_home("~other/vault"), PathBuf::from("~other/vault"));
        assert_eq!(expand_home("/abs"), PathBuf::from("/abs"));
    }
}
