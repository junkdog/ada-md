//! Link resolution (ADR-13, architecture §7). ada knows nothing about vaults: paths and
//! `[[wiki]]` names resolve relative to the note's own directory, and failing that, by name under
//! the note's root, a directory the config names or marks ([`Roots`]).
//!
//! Classifying a link is pure. Finding the file it names reads the filesystem, so it runs in the
//! effect runner through the [`Resolver`] trait, and the results come back as a message (ADR-19).
//! [`RootedResolver`] chains after [`BuiltinResolver`] without touching callers.

mod roots;

use std::path::{Component, Path, PathBuf};

pub use roots::{RootedResolver, Roots, expand_home};

use crate::doc::{Anchor, AnchorId, AnchorKind, Doc, FootnoteId, LinkKind, LinkRef, slug};

/// What a link points at, before touching the filesystem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// `#frag`: a heading in the same note.
    Anchor(String),
    Footnote(FootnoteId),
    /// A URL with a scheme, such as `https:` or `mailto:`.
    External(String),
    Local {
        target: LocalTarget,
        frag: Option<String>,
    },
    Empty,
}

/// A file named by a link, relative to the note it's in.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LocalTarget {
    /// A path, percent-decoded.
    Path(String),
    /// A `[[wiki]]` name: `name.md` or `name`.
    Wiki(String),
}

impl Target {
    pub fn of(link: &LinkRef) -> Target {
        let dest = &*link.dest;
        match link.kind {
            LinkKind::Footnote(id) => return Target::Footnote(id),
            LinkKind::Email => return Target::External(format!("mailto:{dest}")),
            _ if dest.trim().is_empty() => return Target::Empty,
            LinkKind::Wiki { .. } => {
                let (name, frag) = split_frag(dest);
                return match name.trim() {
                    "" => Target::Anchor(frag.unwrap_or_default().to_owned()),
                    name => Target::Local {
                        target: LocalTarget::Wiki(name.to_owned()),
                        frag: frag.map(str::to_owned),
                    },
                };
            }
            _ => {}
        }
        if let Some(frag) = dest.strip_prefix('#') {
            return Target::Anchor(percent_decode(frag));
        }
        let path = match scheme(dest) {
            Some(s) if s.eq_ignore_ascii_case("file") => {
                dest[s.len() + 1..].trim_start_matches("//")
            }
            Some(_) => return Target::External(dest.to_owned()),
            None => dest,
        };
        let (path, frag) = split_frag(path);
        Target::Local {
            target: LocalTarget::Path(percent_decode(path)),
            frag: frag.map(percent_decode),
        }
    }
}

fn split_frag(dest: &str) -> (&str, Option<&str>) {
    match dest.split_once('#') {
        Some((path, frag)) => (path, Some(frag)),
        None => (dest, None),
    }
}

/// A URL scheme: a letter, then letters, digits, `+`, `-` or `.`, then `:`. Single letters are
/// left alone, as they are more likely drive letters than schemes.
pub(crate) fn scheme(dest: &str) -> Option<&str> {
    let (scheme, _) = dest.split_once(':')?;
    let mut chars = scheme.chars();
    let ok = scheme.len() > 1
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    ok.then_some(scheme)
}

/// `%20` → ` `. Invalid escapes and invalid UTF-8 are kept as they are.
pub fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_owned();
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(h), Some(l)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            )
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_owned())
}

/// Where a local target leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    Found(PathBuf),
    /// Why not, for the status line.
    Broken(String),
}

/// Finds the file a link names. Runs in the effect runner, so it may touch the filesystem.
pub trait Resolver {
    fn resolve(&self, source: &Path, target: &LocalTarget) -> Resolved;
}

/// The built-in rules (iteration 1, plan Q4): paths relative to the note's directory, and
/// `[[name]]` as `name.md`, then `name`, in that directory.
#[derive(Debug, Default)]
pub struct BuiltinResolver;

impl Resolver for BuiltinResolver {
    fn resolve(&self, source: &Path, target: &LocalTarget) -> Resolved {
        let dir = source.parent().unwrap_or(Path::new(""));
        let candidates = match target {
            LocalTarget::Path(p) => vec![PathBuf::from(p)],
            LocalTarget::Wiki(name) if is_markdown(Path::new(name)) => vec![PathBuf::from(name)],
            LocalTarget::Wiki(name) => vec![PathBuf::from(format!("{name}.md")), name.into()],
        };
        let candidates: Vec<PathBuf> = candidates.iter().map(|p| normalize(&dir.join(p))).collect();
        match candidates.iter().find(|p| p.exists()) {
            Some(found) => Resolved::Found(found.clone()),
            None => Resolved::Broken(format!("no such file: {}", candidates[0].display())),
        }
    }
}

/// Clean up a path without touching the filesystem: `.` goes, `..` cancels the component
/// before it. Symlinks are not followed, so `a/link/..` becomes `a`.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else if !out.has_root() {
                    out.push("..");
                }
            }
            c => out.push(c),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

pub(crate) fn has_extension(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// URLs handed to the system's opener: web pages and mail. Other schemes could launch anything
/// that registered for them, so they aren't followed.
pub fn is_openable(url: &str) -> bool {
    scheme(url).is_some_and(|s| {
        ["http", "https", "mailto"]
            .iter()
            .any(|x| s.eq_ignore_ascii_case(x))
    })
}

/// Notes ada opens itself.
pub fn is_markdown(path: &Path) -> bool {
    has_extension(path, &["md", "markdown", "mdown", "mkd", "mkdn"])
}

pub fn is_image(path: &Path) -> bool {
    has_extension(path, &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"])
}

/// The anchor `frag` names in `doc`: a slug, then (for wiki links) the heading text,
/// case-insensitive with whitespace collapsed, then the slug of `frag`, then Obsidian's path of
/// headings, `A#B` naming a heading B under A (OBS-013). A `^id` names a block by its id alone
/// (OBS-009), then ignoring case. Only the document's own anchors, not those of the notes
/// transcluded into it (OBS-007).
pub fn find_anchor(doc: &Doc, frag: &str) -> Option<AnchorId> {
    find_anchor_in(doc, frag, None)
}

/// [`find_anchor`] among the anchors of `note`, a note transcluded into `doc` (an index of
/// [`Doc::embedded`]), or `None` for the document's own.
pub fn find_anchor_in(doc: &Doc, frag: &str, note: Option<usize>) -> Option<AnchorId> {
    let all = doc.embedded.is_empty();
    let anchors = (doc.anchors.iter().enumerate())
        .filter(|&(i, _)| all || doc.anchor_note(i as AnchorId) == note);
    if let Some(id) = frag.strip_prefix('^') {
        let mut blocks = anchors.filter(|(_, a)| matches!(a.kind, AnchorKind::Block { .. }));
        let found = (blocks.clone().find(|(_, a)| *a.text == *id))
            .or_else(|| blocks.find(|(_, a)| a.text.eq_ignore_ascii_case(id)));
        return found.map(|(i, _)| i as AnchorId);
    }
    let mut found = anchors.clone().find(|(_, a)| *a.slug == *frag);
    if found.is_none() {
        let key = slug::heading_key(frag);
        found = anchors.clone().find(|(_, a)| {
            matches!(a.kind, AnchorKind::Heading(_)) && slug::heading_key(&a.text) == key
        });
    }
    if found.is_none() {
        let slug = slug::github(frag);
        found = anchors.clone().find(|(_, a)| *a.slug == slug);
    }
    if found.is_none() && frag.contains('#') {
        found = nested(anchors, frag);
    }
    found.map(|(i, _)| i as AnchorId)
}

/// The heading Obsidian's `A#B#C` names: a heading C whose headings above it, those of higher
/// levels it falls under, include A and then B, each by its text as [`find_anchor`] matches it.
fn nested<'a>(
    anchors: impl Iterator<Item = (usize, &'a Anchor)>,
    frag: &str,
) -> Option<(usize, &'a Anchor)> {
    let path: Vec<String> = (frag.split('#'))
        .filter(|p| !p.trim().is_empty())
        .map(slug::heading_key)
        .collect();
    let (last, above) = path.split_last()?;
    // the headings the one being read falls under, by level
    let mut open: Vec<(u8, String)> = Vec::new();
    for (i, a) in anchors {
        let AnchorKind::Heading(level) = a.kind else {
            continue;
        };
        while open.last().is_some_and(|&(l, _)| l >= level) {
            open.pop();
        }
        let key = slug::heading_key(&a.text);
        if key == *last {
            let mut keys = open.iter().map(|(_, k)| k);
            if above.iter().all(|want| keys.any(|k| k == want)) {
                return Some((i, a));
            }
        }
        open.push((level, key));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::parse;

    fn target(md: &str) -> Target {
        let doc = parse(md);
        Target::of(&doc.links[0])
    }

    fn local(path: &str, frag: Option<&str>) -> Target {
        Target::Local {
            target: LocalTarget::Path(path.into()),
            frag: frag.map(Into::into),
        }
    }

    #[test]
    fn classification() {
        assert_eq!(
            target("[a](#Some-Part)"),
            Target::Anchor("Some-Part".into())
        );
        assert_eq!(
            target("[a](../x/y.md#sec)"),
            local("../x/y.md", Some("sec"))
        );
        assert_eq!(target("[a](my%20note.md)"), local("my note.md", None));
        assert_eq!(target("[a](<my note.md>)"), local("my note.md", None));
        assert_eq!(
            target("[a](https://example.com/#x)"),
            Target::External("https://example.com/#x".into())
        );
        assert_eq!(
            target("<https://e.org>"),
            Target::External("https://e.org".into())
        );
        assert_eq!(
            target("<me@e.org>"),
            Target::External("mailto:me@e.org".into())
        );
        assert_eq!(target("[a](file:///tmp/x.md)"), local("/tmp/x.md", None));
        assert_eq!(target("[a]()"), Target::Empty);
        assert_eq!(
            target("[[other note#Known  Bits|alias]]"),
            Target::Local {
                target: LocalTarget::Wiki("other note".into()),
                frag: Some("Known  Bits".into()),
            }
        );
        assert_eq!(target("[[#Local]]"), Target::Anchor("Local".into()));
        assert!(matches!(target("x[^n]\n\n[^n]: y"), Target::Footnote(0)));
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("a%20b%2Fc"), "a b/c");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz%c3%a9"), "%zzé");
        assert_eq!(percent_decode("%ff"), "%ff");
    }

    #[test]
    fn normalizing() {
        let n = |p: &str| normalize(Path::new(p));
        assert_eq!(n("a/./b/../c.md"), Path::new("a/c.md"));
        assert_eq!(n("a/../../c.md"), Path::new("../c.md"));
        assert_eq!(n("/a/../../c.md"), Path::new("/c.md"));
        assert_eq!(n("a/.."), Path::new("."));
    }

    #[test]
    fn anchors_by_slug_and_heading_text() {
        let doc = parse("# Intro\n\n## Known  nondeterminism\n\n## Intro\n");
        assert_eq!(find_anchor(&doc, "intro-1"), Some(2));
        assert_eq!(find_anchor(&doc, "known--nondeterminism"), Some(1));
        assert_eq!(find_anchor(&doc, "Known Nondeterminism"), Some(1));
        assert_eq!(find_anchor(&doc, "Intro"), Some(0));
        assert_eq!(find_anchor(&doc, "missing"), None);
    }

    #[test]
    fn a_path_of_headings_names_the_last_under_the_others() {
        let doc = parse(
            "# Setup\n\n## Linux\n\n### Notes\n\n## Windows\n\n### Notes\n\n# Usage\n\n## Notes\n",
        );
        let text =
            |frag: &str| find_anchor(&doc, frag).map(|a| (a, &*doc.anchors[a as usize].text));
        assert_eq!(text("Setup#Windows#Notes"), Some((4, "Notes")));
        assert_eq!(text("Setup#Linux#Notes"), Some((2, "Notes")));
        // a heading further up, skipping a level
        assert_eq!(text("Setup#Notes"), Some((2, "Notes")));
        assert_eq!(text("Usage#Notes"), Some((6, "Notes")));
        assert_eq!(text("windows#notes"), Some((4, "Notes")));
        // in the order written, and each under the one before
        assert_eq!(text("Windows#Setup#Notes"), None);
        assert_eq!(text("Usage#Windows#Notes"), None);
        assert_eq!(text("Setup#Gone"), None);
    }

    #[test]
    fn builtin_resolution() {
        let dir = std::env::temp_dir().join(format!("ada-links-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for f in ["note.md", "sub/other.md", "plain", "pic.png"] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        let source = dir.join("sub/../note.md");
        let r = |t: LocalTarget| BuiltinResolver.resolve(&source, &t);
        let wiki = |s: &str| LocalTarget::Wiki(s.into());
        let path = |s: &str| LocalTarget::Path(s.into());
        assert_eq!(
            r(path("sub/other.md")),
            Resolved::Found(normalize(&dir.join("sub/other.md")))
        );
        assert_eq!(
            r(wiki("note")),
            Resolved::Found(normalize(&dir.join("note.md")))
        );
        assert_eq!(
            r(wiki("plain")),
            Resolved::Found(normalize(&dir.join("plain")))
        );
        assert_eq!(
            r(wiki("sub/other")),
            Resolved::Found(normalize(&dir.join("sub/other.md")))
        );
        assert_eq!(
            r(path("pic.png")),
            Resolved::Found(normalize(&dir.join("pic.png")))
        );
        let Resolved::Broken(why) = r(wiki("gone")) else {
            panic!("found a missing note");
        };
        assert!(why.ends_with("gone.md"), "{why}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn openable_urls() {
        assert!(is_openable("https://example.com"));
        assert!(is_openable("HTTP://example.com"));
        assert!(is_openable("mailto:me@e.org"));
        assert!(!is_openable("javascript:alert(1)"));
        assert!(!is_openable("ssh://host"));
    }

    #[test]
    fn kinds_of_file() {
        assert!(is_markdown(Path::new("a/B.MD")));
        assert!(!is_markdown(Path::new("a/b")));
        assert!(is_image(Path::new("x.JPG")));
    }
}
