//! Links found by name under a root (ADR-13, as amended 2026-10-21). A `[[wiki]]` name, or a
//! link that is a bare file name, which isn't beside its note is looked up in the whole tree of
//! the note's root: the nearest directory above it that the config lists in `[links] roots`, or
//! that holds one of `root_markers` (`.obsidian` by default). That is how Obsidian resolves
//! links, and it keeps links working when notes move between folders, PARA categories
//! included, without ada knowing about either.
//!
//! The tree is indexed by file name on the first lookup that needs it, and walked again on a
//! miss once the index is a little old, so a note created during the session is found.
//!
//! A `[[name]]` that no file has may be another name a note gives itself, in the `aliases` of its
//! front matter (OBS-010, ADR-46), as Obsidian resolves it. Those are read on the first lookup
//! that misses every file, from each note's front matter alone, and kept with the note's
//! modification time: when the index is walked again, only the notes changed since are read.
//!
//! The walk and the read run without the indexes' lock, held only to look up and to store: the
//! loader gathers a note's embeds while links resolve on threads of their own, and a cold read
//! of a large vault's front matter takes tens of milliseconds (RLD-001).

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use super::{BuiltinResolver, LocalTarget, Resolved, Resolver, is_markdown};
use crate::doc::{front, props};

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

/// `~` and `~/…` as under `$HOME`, and `~\…` too on Windows, where home is `%USERPROFILE%`;
/// other paths as given.
pub fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix('~') {
        Some("") => home().unwrap_or_else(|| path.into()),
        Some(rest) if rest.starts_with(std::path::is_separator) => match home() {
            Some(home) => home.join(&rest[1..]),
            None => path.into(),
        },
        _ => path.into(),
    }
}

fn home() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
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
    /// Behind a lock: the loader resolves a note's embeds while links resolve on threads of
    /// their own (OBS-007, RLD-001), and they share the indexes. Never held over a walk or a read.
    indexes: Mutex<Vec<Index>>,
}

#[derive(Debug)]
struct Index {
    root: PathBuf,
    built: Instant,
    /// Lowercased file name → the files of that name, sorted. Shared, so that the aliases are
    /// read from it without the lock.
    names: Arc<HashMap<String, Vec<PathBuf>>>,
    /// Lowercased alias → the notes that give it, sorted: made when a lookup first misses every
    /// file, from `read`.
    aliases: Option<HashMap<String, Vec<PathBuf>>>,
    /// Each note's aliases as last read.
    read: AliasesRead,
}

/// Each note's aliases as last read, beside its modification time then.
type AliasesRead = HashMap<PathBuf, (Option<SystemTime>, Vec<String>)>;

/// The most of a note read for its front matter: a block that runs on past it is no front matter
/// ada reads aliases from.
const MAX_FRONT: usize = 256 * 1024;

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
            indexes: Mutex::default(),
        }
    }

    fn indexes(&self) -> MutexGuard<'_, Vec<Index>> {
        self.indexes.lock().unwrap_or_else(PoisonError::into_inner)
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
        let name = name.to_lowercase();
        let named = |index: &Index| index.names.get(&name).cloned().unwrap_or_default();
        {
            let indexes = self.indexes();
            let index = indexes.iter().find(|i| i.root == root);
            if let Some(index) = index.filter(|i| !fresh || i.built.elapsed() < self.refresh) {
                return named(index);
            }
        }
        let index = Index::build(root);
        let found = named(&index);
        let mut indexes = self.indexes();
        match indexes.iter_mut().find(|i| i.root == root) {
            // another lookup walked it after this one did
            Some(old) if old.built > index.built => {}
            // the notes' aliases as read stay: a note unchanged since isn't read again
            Some(old) => {
                let read = std::mem::take(&mut old.read);
                *old = Index { read, ..index };
            }
            None => indexes.push(index),
        }
        found
    }

    /// The notes under `root`, which has an index, that give `name` as an alias, ignoring case.
    fn aliased(&self, root: &Path, name: &str) -> Vec<PathBuf> {
        let name = name.to_lowercase();
        let (built, names, read) = {
            let mut indexes = self.indexes();
            let Some(index) = indexes.iter_mut().find(|i| i.root == root) else {
                return Vec::new();
            };
            if let Some(aliases) = &index.aliases {
                return aliases.get(&name).cloned().unwrap_or_default();
            }
            let read = std::mem::take(&mut index.read);
            (index.built, Arc::clone(&index.names), read)
        };
        let (aliases, read) = read_all(&names, read);
        let found = aliases.get(&name).cloned().unwrap_or_default();
        let mut indexes = self.indexes();
        if let Some(index) = indexes.iter_mut().find(|i| i.root == root) {
            // what was read saves a later walk's index reading it again: moved in, as `read` was
            // taken from it (another read's notes added to the larger, a hash of each path
            // 1 ms for 5,536)
            if index.read.len() < read.len() {
                let other = std::mem::replace(&mut index.read, read);
                index.read.extend(other);
            } else {
                index.read.extend(read);
            }
            if index.built == built {
                index.aliases.get_or_insert(aliases);
            }
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
            LocalTarget::Path(p) if !p.contains('/') && !matches!(&**p, "" | "." | "..") => {
                ("", vec![p.clone()])
            }
            // a path with directories in it says where the file is: beside the note, else from
            // the root
            LocalTarget::Path(p) => return self.under_root(source, p),
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
                // an exact match wins over ones that differ in case, among those in the folders named
                if found.len() > 1 && found.iter().any(|p| file_name(p) == n) {
                    found.retain(|p| file_name(p) == n);
                }
                (!found.is_empty()).then_some((n, found))
            })
        };
        let (what, found) = match (find(false).or_else(|| find(true)), target) {
            (Some((name, found)), _) => (format!("files named {name}"), found),
            // no file has the name: a note's alias, a name without folders
            (None, LocalTarget::Wiki(name)) if dirs.as_os_str().is_empty() => {
                let found = self.aliased(&root, name);
                (format!("notes with the alias {name}"), found)
            }
            (None, _) => return None,
        };
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
                    _ => Resolved::Broken(ambiguous(&what, &root, many)),
                }
            }
        })
    }
}

impl RootedResolver {
    /// `path` from the root of `source`, as Obsidian writes a markdown link in its `absolute`
    /// link format, and in its `shortest` one for a name that isn't unique (OBS-013):
    /// `[x](Projects/Note.md)` in any folder of the vault. Never a file outside the root.
    fn under_root(&self, source: &Path, path: &str) -> Option<Resolved> {
        if path.is_empty() || Path::new(path).has_root() {
            return None;
        }
        let dir = source.parent().filter(|d| !d.as_os_str().is_empty());
        let dir = dir.unwrap_or(Path::new("."));
        let canonical = fs::canonicalize(dir).ok()?;
        let root = self.root_of(&canonical)?;
        let found = super::normalize(&root.join(path));
        (found.starts_with(&root) && found.exists())
            .then(|| Resolved::Found(rebase(&found, dir, &canonical)))
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
            names: Arc::new(names),
            aliases: None,
            read: HashMap::new(),
        }
    }
}

/// Lowercased alias → the notes of `names` that give it, from their front matter, beside each
/// note's aliases as read now: a note unchanged since `read` keeps the aliases read then.
fn read_all(
    names: &HashMap<String, Vec<PathBuf>>,
    mut read: AliasesRead,
) -> (HashMap<String, Vec<PathBuf>>, AliasesRead) {
    let notes: Vec<&PathBuf> = names
        .values()
        .flatten()
        .filter(|p| is_markdown(p))
        .collect();
    // on threads: from a cold cache, a note's read is ~0.1 ms of waiting on the disk
    // (5,536 notes, 0.53 s on one), a warm one's 8 µs
    let fresh = on_threads(&notes, |path| {
        let modified = fs::metadata(path).and_then(|m| m.modified()).ok();
        let same = read
            .get(*path)
            .is_some_and(|(then, _)| then.is_some() && *then == modified);
        (modified, (!same).then(|| read_aliases(path)))
    });
    let mut map: HashMap<String, Vec<PathBuf>> = HashMap::new();
    let mut now = HashMap::new();
    for (path, (modified, fresh)) in notes.into_iter().zip(fresh) {
        let given = match fresh {
            Some(given) => given,
            None => read
                .remove(path)
                .map(|(_, given)| given)
                .unwrap_or_default(),
        };
        for alias in &given {
            map.entry(alias.to_lowercase())
                .or_default()
                .push(path.clone());
        }
        now.insert(path.clone(), (modified, given));
    }
    for paths in map.values_mut() {
        paths.sort();
        paths.dedup();
    }
    (map, now)
}

/// `f` of each of `items`, in order, on up to 8 threads; on this one for a few.
fn on_threads<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = crate::cores();
    if threads == 1 || items.len() < 64 {
        return items.iter().map(f).collect();
    }
    let f = &f;
    std::thread::scope(|s| {
        let chunks: Vec<_> = (items.chunks(items.len().div_ceil(threads)))
            .map(|chunk| s.spawn(move || chunk.iter().map(f).collect::<Vec<R>>()))
            .collect();
        (chunks.into_iter())
            .flat_map(|c| c.join().unwrap_or_else(|p| std::panic::resume_unwind(p)))
            .collect()
    })
}

/// The aliases the note at `path` gives itself: its front matter read, and nothing past it but
/// the rest of the first 4 KiB.
fn read_aliases(path: &Path) -> Vec<String> {
    let Ok(mut file) = fs::File::open(path) else {
        return Vec::new();
    };
    let mut head = Vec::new();
    let mut limit = 4096;
    loop {
        let want = (limit - head.len()) as u64;
        if (&mut file).take(want).read_to_end(&mut head).is_err()
            || !(head.starts_with(b"---") || head.starts_with(b"+++"))
        {
            return Vec::new();
        }
        let text = String::from_utf8_lossy(&head);
        if head.len() < limit || limit >= MAX_FRONT || front::find(&text, true).is_some() {
            return crate::guard::catch(|| props::aliases(&text)).unwrap_or_default();
        }
        limit *= 4;
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

/// Why a name that matches several files isn't followed: `what` they are, and the first few,
/// relative to the root.
fn ambiguous(what: &str, root: &Path, many: &[PathBuf]) -> String {
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
    format!("{} {what} under {}: {list}", many.len(), root.display())
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
        // a path with directories in it is where it says: beside the note, or from the root
        let Resolved::Broken(why) = r.resolve(&source, &path("pics/cat.png")) else {
            panic!("followed a path that isn't there");
        };
        assert!(why.starts_with("no such file"), "{why}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_path_resolves_from_the_root_after_the_note() {
        let dir = tree(
            "from-root",
            &[
                "vault/.obsidian/.keep",
                "vault/Projects/Note.md",
                "vault/Daily/2026/today.md",
                "vault/Daily/2026/Projects/Note.md",
                "vault/pics/cat.png",
                "outside.md",
            ],
        );
        let r = rooted(&[]);
        let source = dir.join("vault/Daily/2026/today.md");
        // the note's folder first
        assert_eq!(
            r.resolve(&source, &path("Projects/Note.md")),
            Resolved::Found(dir.join("vault/Daily/2026/Projects/Note.md"))
        );
        let source = dir.join("vault/Daily/today.md");
        assert_eq!(
            r.resolve(&source, &path("Projects/Note.md")),
            Resolved::Found(dir.join("vault/Projects/Note.md"))
        );
        assert_eq!(
            r.resolve(&source, &path("pics/cat.png")),
            Resolved::Found(dir.join("vault/pics/cat.png"))
        );
        // never out of the root, nor a path the root doesn't have
        for p in ["../outside.md", "pics/../../outside.md", "Projects/Gone.md"] {
            let Resolved::Broken(why) = r.resolve(&source, &path(p)) else {
                panic!("followed {p}");
            };
            assert!(why.starts_with("no such file"), "{why}");
        }
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
        // in the system's separators, as the root before them
        let list = "10-projects/x/todo.md, 20-areas/y/sub/todo.md, 20-areas/y/todo.md"
            .replace('/', std::path::MAIN_SEPARATOR_STR);
        assert_eq!(
            why,
            format!("3 files named todo.md under {}: {list}", dir.display())
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
    fn case_decides_only_among_the_folders_named() {
        // fuzz `roots`, 2026-10-21: the exact-case `y/Note.md` hid `x/sub/note.md` before the
        // folder was checked, and `[[sub/Note]]` came out broken
        let dir = tree("case-folder", &["a/src.md", "x/sub/note.md", "y/Note.md"]);
        let r = rooted(&[&dir]);
        assert_eq!(
            r.resolve(&dir.join("a/src.md"), &wiki("sub/Note")),
            Resolved::Found(dir.join("x/sub/note.md"))
        );
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
    fn an_alias_in_front_matter_names_its_note() {
        let dir = tree("aliases", &["vault/.obsidian/.keep", "vault/a/src.md"]);
        let note = |path: &str, text: &str| {
            let path = dir.join("vault").join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        note(
            "b/Robert.md",
            "---\naliases: [Bob, \"Rob B\"]\n---\n# Robert\n",
        );
        note("c/Bob.md", "a note named like the alias wins\n");
        note("d/one.md", "---\nalias: Twin\n---\n");
        note("e/two.md", "---\naliases:\n  - twin\n---\n");
        // past the first 4 KiB of front matter too
        let long = format!("---\nnote: {}\naliases: [Far]\n---\n", "x".repeat(6000));
        note("f/long.md", &long);
        let r = rooted(&[]);
        let source = dir.join("vault/a/src.md");
        assert_eq!(
            r.resolve(&source, &wiki("rob b")),
            Resolved::Found(dir.join("vault/b/Robert.md"))
        );
        assert_eq!(
            r.resolve(&source, &wiki("Bob")),
            Resolved::Found(dir.join("vault/c/Bob.md"))
        );
        assert_eq!(
            r.resolve(&source, &wiki("Far")),
            Resolved::Found(dir.join("vault/f/long.md"))
        );
        let Resolved::Broken(why) = r.resolve(&source, &wiki("Twin")) else {
            panic!("picked one of two");
        };
        assert!(
            why.starts_with("2 notes with the alias Twin under"),
            "{why}"
        );
        // a folder in the link names a file, never an alias
        let Resolved::Broken(_) = r.resolve(&source, &wiki("b/Rob B")) else {
            panic!("took an alias for a path");
        };
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn home_expands() {
        let Some(home) = home() else {
            return;
        };
        assert_eq!(expand_home("~"), home);
        assert_eq!(expand_home("~/vault"), home.join("vault"));
        assert_eq!(expand_home("~other/vault"), PathBuf::from("~other/vault"));
        assert_eq!(expand_home("/abs"), PathBuf::from("/abs"));
    }
}
