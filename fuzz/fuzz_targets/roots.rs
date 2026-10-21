//! Links looked up by name under a root (ADR-13, amended 2026-10-21): `RootedResolver` on a tree
//! the input builds in a directory of the process's own, under `/dev/shm` where there is one.
//!   - the tree: up to 16 files, directories and symlinks (to files, to directories, to an
//!     ancestor, outside the tree, dangling), named from a small alphabet that collides in case
//!     and across folders (`sift`, `Note`, `note.md`, `.obsidian`, `..`, …) or by the input
//!   - the config: roots (directories of the tree, the tree itself, missing ones) and markers
//!   - the note: a file of the tree, one reached through a symlink, or a missing one
//!   - up to 8 links, `[[wiki]]` names or paths, built from the same alphabet
//!
//! Each answer must match a reference written apart from ada's (a recursive walk, no index):
//! what the built-in rules find stands; otherwise the nearest root's unique match, the one in a
//! folder named like the note's, or the built-in reason, or an ambiguity listing the matches.
//! Asked again, by the same resolver (its index) or a fresh one, the answer is the same.
//!
//! Also `[links]` in the config: a parse that succeeds has absolute roots and plain markers (no
//! `/`, not `.` or `..`). Markers that exist above the tree are dropped, so a root stays inside it.
//!
//! ```zsh
//! cargo +nightly fuzz run -s none -a roots -- -max_len=1024
//! ```

#![no_main]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use ada::config;
use ada::links::{BuiltinResolver, LocalTarget, Resolved, Resolver, RootedResolver, Roots};
use libfuzzer_sys::arbitrary::{Result, Unstructured};
use libfuzzer_sys::fuzz_target;

const NAMES: &[&str] = &[
    "a",
    "A",
    "b",
    "sift",
    "Sift",
    "10-projects",
    "40-archive",
    "note",
    "Note",
    "note.md",
    "Note.md",
    "x.md",
    "pic.png",
    "PIC.png",
    ".obsidian",
    ".git",
    "VAULT.md",
    "..",
    ".",
    " ",
    "ä",
    "Ä",
    "n.MD",
    "deep",
];

/// The tree's directory, the same for every run of this process. The first call removes the
/// trees of processes that have exited: a fork job ends without cleaning up (round 9 left 514).
fn base() -> &'static Path {
    static BASE: OnceLock<PathBuf> = OnceLock::new();
    BASE.get_or_init(|| {
        let shm = Path::new("/dev/shm");
        let tmp = if shm.is_dir() {
            shm.to_path_buf()
        } else {
            std::env::temp_dir()
        };
        for entry in fs::read_dir(&tmp).into_iter().flatten().flatten() {
            let name = entry.file_name();
            let pid = name
                .to_str()
                .and_then(|n| n.strip_prefix("ada-fuzz-roots-"))
                .map(|n| n.trim_end_matches(".outside"))
                .filter(|pid| !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()));
            if let Some(pid) = pid
                && !Path::new("/proc").join(pid).exists()
            {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
        tmp.join(format!("ada-fuzz-roots-{}", std::process::id()))
    })
}

/// A directory outside the tree, holding a note, for symlinks to lead out to.
fn outside() -> PathBuf {
    let dir = base().with_extension("outside");
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(dir.join("note.md"), "x");
    dir
}

fn name(u: &mut Unstructured) -> Result<String> {
    if u.ratio(1, 8)? {
        let s: String = u.arbitrary()?;
        let s: String = s
            .chars()
            .filter(|&c| c != '/' && c != '\0')
            .take(12)
            .collect();
        if !s.is_empty() {
            return Ok(s);
        }
    }
    Ok(u.choose(NAMES)?.to_string())
}

/// A relative path of one to `max` names.
fn rel(u: &mut Unstructured, max: u32) -> Result<String> {
    let n = u.int_in_range(1..=max)?;
    let mut parts = Vec::new();
    for _ in 0..n {
        parts.push(name(u)?);
    }
    Ok(parts.join("/"))
}

fn build(u: &mut Unstructured, root: &Path) -> Result<Vec<PathBuf>> {
    let mut made = vec![root.to_path_buf()];
    for _ in 0..u.int_in_range(0..=16)? {
        let rel = rel(u, 4)?;
        // no `.` or `..`: after a symlink, `..` climbs out of where it leads (`outside/..` is
        // `/dev/shm`), and the tree would grow into other processes' (round 9's first launch)
        if rel.split('/').any(|c| c == "." || c == "..") {
            continue;
        }
        let path = root.join(rel);
        let parent = path.parent().unwrap_or(root);
        let _ = fs::create_dir_all(parent);
        match u.int_in_range(0..=9)? {
            0..=5 => {
                let _ = fs::write(&path, "x");
            }
            6 | 7 => {
                let _ = fs::create_dir_all(&path);
            }
            _ => {
                let to = match u.int_in_range(0..=3)? {
                    0 => u.choose(&made)?.clone(),
                    1 => root.to_path_buf(),
                    2 => PathBuf::from("/nonexistent-ada"),
                    _ => outside(),
                };
                let _ = std::os::unix::fs::symlink(to, &path);
            }
        }
        made.push(path);
    }
    Ok(made)
}

/// The reference: what the lookup under a root should answer, or `None` for the built-in reason.
fn reference(
    roots: &[PathBuf],
    markers: &[String],
    source: &Path,
    target: &LocalTarget,
) -> Option<Expect> {
    let (dirs, names) = match target {
        LocalTarget::Wiki(n) => {
            let (dirs, base) = match n.rfind('/') {
                Some(i) => (&n[..i], &n[i + 1..]),
                None => ("", &n[..]),
            };
            let ext = Path::new(base)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            let markdown = ["md", "markdown", "mdown", "mkd", "mkdn"];
            let names = if markdown.iter().any(|m| m.eq_ignore_ascii_case(ext)) {
                vec![base.to_string()]
            } else {
                vec![format!("{base}.md"), base.to_string()]
            };
            (dirs.to_string(), names)
        }
        LocalTarget::Path(p) if !p.contains('/') && !["", ".", ".."].contains(&&**p) => {
            (String::new(), vec![p.clone()])
        }
        LocalTarget::Path(_) => return None,
    };
    if names.iter().any(String::is_empty) {
        return None;
    }
    let dir = match source.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let canonical = fs::canonicalize(&dir).ok()?;
    let roots: Vec<PathBuf> = roots
        .iter()
        .filter_map(|r| fs::canonicalize(r).ok())
        .collect();
    let mut root = None;
    let mut at = Some(canonical.as_path());
    while let Some(a) = at {
        if roots.iter().any(|r| r == a) || markers.iter().any(|m| a.join(m).exists()) {
            root = Some(a.to_path_buf());
            break;
        }
        at = a.parent();
    }
    let root = root?;
    let mut files = Vec::new();
    walk(&root, &mut files);
    for n in &names {
        let mut found: Vec<PathBuf> = files
            .iter()
            .filter(|p| file_name(p).to_lowercase() == n.to_lowercase())
            .filter(|p| p.parent().is_some_and(|d| d.ends_with(&dirs)))
            .cloned()
            .collect();
        if found.is_empty() {
            continue;
        }
        if found.len() > 1 && found.iter().any(|p| file_name(p) == *n) {
            found.retain(|p| file_name(p) == *n);
        }
        let rebase = |p: &Path| match p.strip_prefix(&canonical) {
            Ok(rest) => dir.join(rest),
            Err(_) => p.to_path_buf(),
        };
        if let [one] = &found[..] {
            return Some(Expect::Found(rebase(one)));
        }
        let near: Vec<_> = found
            .iter()
            .filter(|p| p.parent().and_then(Path::file_name) == canonical.file_name())
            .collect();
        return Some(match near[..] {
            [one] => Expect::Found(rebase(one)),
            _ => Expect::Ambiguous(format!(
                "{} files named {n} under {}: ",
                found.len(),
                root.display()
            )),
        });
    }
    None
}

enum Expect {
    Found(PathBuf),
    /// The start of the reason.
    Ambiguous(String),
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let path = e.path();
        if file_name(&path).starts_with('.') {
            continue;
        }
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            walk(&path, out);
        } else if meta.is_file() || (meta.file_type().is_symlink() && path.is_file()) {
            out.push(path);
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn run(u: &mut Unstructured) -> Result<()> {
    // `[links]` as the config holds it
    let text = format!("[links]\n{}", u.arbitrary::<&str>()?);
    if let Ok(s) = config::parse("c.toml", &text, &mut Vec::new()) {
        assert!(
            s.links.roots.iter().all(|r| r.is_absolute()),
            "{:?}",
            s.links
        );
        assert!(
            s.links
                .markers
                .iter()
                .all(|m| !matches!(m.as_str(), "" | "." | "..") && !m.contains('/')),
            "{:?}",
            s.links
        );
    }

    let base = base();
    let _ = fs::remove_dir_all(base);
    // and what the last run wrote through a symlink to outside the tree
    let _ = fs::remove_dir_all(base.with_extension("outside"));
    fs::create_dir_all(base).unwrap();
    let tree = fs::canonicalize(base).unwrap();
    let made = build(u, &tree)?;

    let mut roots = Vec::new();
    for _ in 0..u.int_in_range(0..=2)? {
        roots.push(match u.int_in_range(0..=3)? {
            0 => tree.clone(),
            1 => tree.join("missing"),
            _ => u.choose(&made)?.clone(),
        });
    }
    let mut markers = Vec::new();
    for _ in 0..u.int_in_range(0..=2)? {
        markers.push(name(u)?);
    }
    // as the config allows them, and none that exists above the tree (`dev`, `usr`), where the
    // walk would meet other processes' trees
    let above: Vec<&Path> = tree.ancestors().skip(1).collect();
    let markers: Vec<String> = markers
        .into_iter()
        .filter(|m| !matches!(m.as_str(), "" | "." | "..") && !m.contains('/'))
        .filter(|m| !above.iter().any(|a| a.join(m).exists()))
        .collect();
    let config = Roots {
        roots: roots.clone(),
        markers: markers.clone(),
    };

    let source = match u.int_in_range(0..=3)? {
        0 => tree.join(rel(u, 3)?),
        _ => u
            .choose(&made)?
            .join(if u.arbitrary()? { "note.md" } else { "" }),
    };
    let mut targets = Vec::new();
    for _ in 0..u.int_in_range(1..=8)? {
        let s = if u.ratio(1, 4)? { name(u)? } else { rel(u, 3)? };
        targets.push(if u.arbitrary()? {
            LocalTarget::Wiki(s)
        } else {
            LocalTarget::Path(s)
        });
    }

    let rooted = RootedResolver::new(&config);
    for target in &targets {
        let got = rooted.resolve(&source, target);
        let builtin = BuiltinResolver.resolve(&source, target);
        if std::env::var_os("ADA_FUZZ_TRACE").is_some() {
            eprintln!(
                "roots {roots:?} markers {markers:?} source {source:?} target {target:?}: {got:?}"
            );
        }
        match (&builtin, reference(&roots, &markers, &source, target)) {
            (Resolved::Found(_), _) | (Resolved::Broken(_), None) => {
                assert_eq!(got, builtin, "{source:?} {target:?}");
            }
            (Resolved::Broken(_), Some(Expect::Found(p))) => {
                assert_eq!(got, Resolved::Found(p), "{source:?} {target:?}");
            }
            (Resolved::Broken(_), Some(Expect::Ambiguous(start))) => match &got {
                Resolved::Broken(why) if why.starts_with(&start) => {}
                _ => panic!("{source:?} {target:?}: {got:?}, not {start}…"),
            },
        }
        // the index answers as a walk does, now and from a fresh resolver
        assert_eq!(rooted.resolve(&source, target), got);
        assert_eq!(RootedResolver::new(&config).resolve(&source, target), got);
    }
    Ok(())
}

fuzz_target!(|data: &[u8]| {
    let _ = run(&mut Unstructured::new(data));
});
