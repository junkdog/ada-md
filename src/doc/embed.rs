//! Note embeds transcluded (OBS-007, ADR-48): `![[note]]`, `![[note#Heading]]` or a local
//! `![](note.md)` alone in its paragraph shows the note it names, or the heading's section of it,
//! in place, under a row that links to it.
//!
//! Reading the notes is the loader's: [`gather`] finds what a note may embed with a scan of its
//! text ([`targets`]), resolves each name as a link would be, reads the notes found, and the
//! notes they embed in turn, into [`Embeds`]. A load carries them beside the note's text, and a
//! recording keeps them (ADR-19), so the parse that transcludes them reads no files: it looks
//! each embed up in them ([`Embeds::find`]), parses the note it names, and grafts that note's
//! IR into its own (`Offsets`).

use std::collections::VecDeque;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{
    Anchor, AnchorKind, Block, BlockKind, Doc, Footnote, Inlines, LinkKind, LinkRef, Run, RunKind,
    Slots, TextRange,
};
use crate::links::{self, LocalTarget, Resolved, Resolver};

/// The most notes deep an embed is transcluded: a note embedding one that embeds another, and so
/// on. Deeper, an embed is its link. A note is never transcluded inside itself, however deep.
pub const MAX_DEPTH: u32 = 4;

/// The most notes a load reads for its embeds, and the most bytes of them.
const MAX_NOTES: usize = 64;
const MAX_BYTES: u64 = 16 << 20;

/// The most bytes of notes one parse transcludes, counted each time one is: a note that embeds
/// another many times, which embeds another many times, multiplies them.
pub const MAX_TRANSCLUDED: usize = 32 << 20;

/// A note an embed names, read: the embed is `target`, in the note at `from`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub from: PathBuf,
    pub target: LocalTarget,
    pub path: PathBuf,
    /// Milliseconds since the epoch, when known: live reload watches it.
    pub mtime: Option<u64>,
    pub text: Arc<str>,
}

/// The notes a note's embeds name, and theirs, as a load read them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Embeds(pub Arc<[Note]>);

impl Embeds {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The note `target` names, embedded in the note at `from`.
    pub fn find(&self, from: &Path, target: &LocalTarget) -> Option<&Note> {
        self.0
            .iter()
            .find(|n| n.from == from && n.target == *target)
    }

    /// Each note read once, with its modification time: what live reload watches.
    pub fn files(&self) -> Vec<(PathBuf, Option<u64>)> {
        let mut files: Vec<(PathBuf, Option<u64>)> = Vec::new();
        for n in self.0.iter() {
            if !files.iter().any(|(p, _)| *p == n.path) {
                files.push((n.path.clone(), n.mtime));
            }
        }
        files
    }
}

/// What the notes the note at `path`, holding `text`, embeds, and those they embed, read, up to
/// [`MAX_DEPTH`] notes deep; names resolve as `resolver` resolves links. Only notes ada reads as
/// markdown are read, and never the note itself.
pub fn gather(path: &Path, text: &str, resolver: &dyn Resolver) -> Embeds {
    let first = targets(text);
    if first.is_empty() {
        return Embeds::default();
    }
    let mut notes: Vec<Note> = Vec::new();
    let mut bytes = 0u64;
    let mut queue = VecDeque::from([(path.to_owned(), first, 0)]);
    'notes: while let Some((from, targets, depth)) = queue.pop_front() {
        for target in targets {
            if notes.len() == MAX_NOTES {
                break 'notes;
            }
            if notes.iter().any(|n| n.from == from && n.target == target) {
                continue;
            }
            let Resolved::Found(found) = resolver.resolve(&from, &target) else {
                continue;
            };
            if !links::is_markdown(&found) || found == path {
                continue;
            }
            let (text, mtime, new) = match notes.iter().find(|n| n.path == found) {
                Some(n) => (n.text.clone(), n.mtime, false),
                None => {
                    let mtime = crate::images::mtime_of(&found);
                    let Some(text) = read(&found, MAX_BYTES - bytes) else {
                        continue;
                    };
                    bytes += text.len() as u64;
                    (text, mtime, true)
                }
            };
            if new && depth + 1 < MAX_DEPTH {
                queue.push_back((found.clone(), self::targets(&text), depth + 1));
            }
            notes.push(Note {
                from: from.clone(),
                target,
                path: found,
                mtime,
                text,
            });
        }
    }
    Embeds(notes.into())
}

/// A note's text, if it has at most `left` bytes.
fn read(path: &Path, left: u64) -> Option<Arc<str>> {
    let file = fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(left + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > left {
        return None;
    }
    Some(match String::from_utf8(bytes) {
        Ok(text) => text.into(),
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into(),
    })
}

/// The notes `text` may embed, found by a scan of its text rather than a parse: every wiki embed
/// of something other than a file embedded as itself (`is_attachment`: `![[note]]`,
/// `![[note#Heading|alias]]`, not `![[song.mp3]]`), and every
/// `![…](…)` whose path names a markdown file. More than the parse transcludes, as the scan
/// doesn't see code spans and blocks, or a paragraph holding more than the embed; never less.
/// Each target once, without its `#fragment`, in the order first found.
pub fn targets(text: &str) -> Vec<LocalTarget> {
    let mut out: Vec<LocalTarget> = Vec::new();
    let mut add = |t: LocalTarget| {
        if !out.contains(&t) {
            out.push(t);
        }
    };
    let bytes = text.as_bytes();
    let mut at = 0;
    while let Some(i) = text[at..].find("![").map(|i| at + i) {
        at = i + 2;
        if bytes.get(at) == Some(&b'[') {
            if let Some(t) = wiki(&text[at + 1..]) {
                add(t);
            }
            continue;
        }
        if let Some((t, end)) = inline(text, at) {
            add(t);
            at = end;
        }
    }
    out
}

/// The note a wiki embed names, `rest` being what follows its `![[`.
fn wiki(rest: &str) -> Option<LocalTarget> {
    let line = &rest[..rest.find('\n').unwrap_or(rest.len())];
    let inner = &line[..line.find("]]")?];
    let dest = inner.split('|').next().unwrap_or_default();
    let name = dest.split('#').next().unwrap_or_default().trim();
    (!name.is_empty() && !is_attachment(Path::new(name)))
        .then(|| LocalTarget::Wiki(name.to_owned()))
}

/// Whether `name` names a file embedded as itself, not a note: one with a file's extension (one to
/// five letters and digits, a letter among them) that isn't markdown's, as `pic.png`, `song.opus`
/// and `tracks.base`. `Chapter 1.1`, `Dr. Who` and `x.excalidraw` (a drawing's `x.excalidraw.md`)
/// are notes; a note named `notes.v2` is taken for a file unless the embed names it `notes.v2.md`.
/// Not looked for: a name no file has is looked for among every note's aliases, and a vault
/// embedding a song in each note printed in 69 ms instead of 3 (survey, 2026-11-22).
fn is_attachment(name: &Path) -> bool {
    let Some(ext) = name.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    let file_like = (1..=5).contains(&ext.len())
        && ext.bytes().all(|b| b.is_ascii_alphanumeric())
        && ext.bytes().any(|b| b.is_ascii_alphabetic());
    file_like && !links::is_markdown(name)
}

/// The note `![…](dest)` names, its text from `at` (past the `![`) on, and where the scan goes
/// on: the alt text's brackets balanced, the destination bare or in `<…>`, a markdown file's path
/// without a scheme.
fn inline(text: &str, at: usize) -> Option<(LocalTarget, usize)> {
    let bytes = text.as_bytes();
    let mut depth = 1;
    let mut i = at;
    while depth > 0 {
        match *bytes.get(i)? {
            b'\\' => i += 1,
            b'[' => depth += 1,
            b']' => depth -= 1,
            b'\n' if bytes.get(i + 1) == Some(&b'\n') => return None,
            _ => {}
        }
        i += 1;
    }
    if bytes.get(i) != Some(&b'(') {
        return None;
    }
    let rest = text[i + 1..].trim_start_matches([' ', '\t', '\n']);
    let dest = match rest.strip_prefix('<') {
        Some(r) => &r[..r.find(['>', '\n'])?],
        None => {
            let end = rest.find([' ', '\t', '\n', ')']).unwrap_or(rest.len());
            &rest[..end]
        }
    };
    let path = dest.split('#').next().unwrap_or_default();
    let ok = !path.is_empty() && links::scheme(dest).is_none();
    let target = links::percent_decode(path);
    (ok && links::is_markdown(Path::new(&target))).then_some((LocalTarget::Path(target), i + 1))
}

/// A note transcluded into the document: its path, and the ranges of the document's links and
/// anchors that are its own (a note it transcludes in turn has its own ranges, inside these).
#[derive(Debug)]
pub struct Embedded {
    pub path: PathBuf,
    pub links: Slots,
    pub anchors: Slots,
}

/// A note transcluded (OBS-007): the row linking to it, then its blocks.
#[derive(Debug)]
pub struct Embed {
    /// The embed as step 1 drew it, `↳ note#Heading`: the link to the note.
    pub title: Inlines,
    pub blocks: Vec<Block>,
    /// The note, in [`Doc::embedded`].
    pub note: u32,
}

/// How far a note's IR moves when it's grafted into another's: what each of its numberings
/// starts at in the document it joins.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Offsets {
    pub text: u32,
    pub runs: u32,
    pub links: u32,
    pub anchors: u32,
    pub footnotes: u32,
    pub math: u32,
    pub tables: u32,
    pub codes: u32,
    pub units: u32,
    pub notes: u32,
}

impl Offsets {
    pub fn run(self, run: &Run) -> Run {
        Run {
            text: TextRange {
                start: run.text.start + self.text,
                end: run.text.end + self.text,
            },
            link: run.link.map(|l| l + self.links),
            kind: match run.kind {
                RunKind::Math(id) => RunKind::Math(id + self.math),
                RunKind::InlineImage(l) => RunKind::InlineImage(l + self.links),
                RunKind::Embed(l) => RunKind::Embed(l + self.links),
                RunKind::FootnoteRef(f) => RunKind::FootnoteRef(f + self.footnotes),
                kind => kind,
            },
            ..*run
        }
    }

    pub fn link(self, mut link: LinkRef) -> LinkRef {
        if let LinkKind::Footnote(f) = &mut link.kind {
            *f += self.footnotes;
        }
        link
    }

    pub fn anchor(self, mut anchor: Anchor) -> Anchor {
        match &mut anchor.kind {
            AnchorKind::Footnote(f) => *f += self.footnotes,
            AnchorKind::Block { unit, .. } => *unit += self.units,
            AnchorKind::Heading(_) => {}
        }
        anchor
    }

    pub fn math(self, r: TextRange) -> TextRange {
        TextRange {
            start: r.start + self.text,
            end: r.end + self.text,
        }
    }

    pub fn footnote(self, mut note: Footnote) -> Footnote {
        note.anchor = note.anchor.map(|a| a + self.anchors);
        self.blocks(&mut note.blocks);
        note
    }

    pub fn embedded(self, mut e: Embedded) -> Embedded {
        e.links = slots(e.links, self.links);
        e.anchors = slots(e.anchors, self.anchors);
        e
    }

    fn inlines(self, i: &mut Inlines) {
        i.runs = slots(i.runs, self.runs);
        i.unit += self.units;
    }

    pub fn blocks(self, blocks: &mut [Block]) {
        for block in blocks {
            self.block(&mut block.kind);
        }
    }

    fn block(self, kind: &mut BlockKind) {
        match kind {
            BlockKind::Heading {
                inlines, anchor, ..
            } => {
                self.inlines(inlines);
                *anchor += self.anchors;
            }
            BlockKind::Paragraph(inlines) | BlockKind::Math(inlines) => self.inlines(inlines),
            BlockKind::List(list) => {
                for item in &mut list.items {
                    self.blocks(&mut item.blocks);
                }
            }
            BlockKind::Quote { callout, blocks } => {
                if let Some(c) = callout {
                    self.inlines(&mut c.title);
                }
                self.blocks(blocks);
            }
            BlockKind::Code(code) => {
                code.id += self.codes;
                code.first_unit += self.units;
            }
            BlockKind::Table(table) => {
                table.id += self.tables;
                for cell in &mut table.cells {
                    self.inlines(cell);
                }
            }
            BlockKind::Image { link, unit, .. } => {
                *link += self.links;
                *unit += self.units;
            }
            BlockKind::Properties(p) => {
                for row in &mut p.rows {
                    self.inlines(&mut row.value);
                }
                p.source.id += self.codes;
                p.source.first_unit += self.units;
            }
            BlockKind::Embed(e) => {
                self.inlines(&mut e.title);
                self.blocks(&mut e.blocks);
                e.note += self.notes;
            }
            BlockKind::Rule | BlockKind::Html(_) | BlockKind::FootnoteDefs => {}
        }
    }
}

impl Doc {
    /// The transcluded note link `id` is in, the innermost, as an index of [`Doc::embedded`];
    /// `None` for the document's own.
    pub fn link_note(&self, id: u32) -> Option<usize> {
        innermost(&self.embedded, |e| e.links, id)
    }

    /// The transcluded note anchor `id` is in, as [`Doc::link_note`] says for a link.
    pub fn anchor_note(&self, id: u32) -> Option<usize> {
        innermost(&self.embedded, |e| e.anchors, id)
    }

    /// The file link `id` is resolved from: the transcluded note it's in, else `own`, the
    /// document's.
    pub fn link_source<'a>(&'a self, id: u32, own: &'a Path) -> &'a Path {
        match self.link_note(id) {
            Some(n) => &self.embedded[n].path,
            None => own,
        }
    }
}

/// `s` moved on by `by`.
fn slots(s: Slots, by: u32) -> Slots {
    Slots {
        start: s.start + by,
        end: s.end + by,
    }
}

/// The narrowest of `embedded`'s ranges holding `id`.
fn innermost(embedded: &[Embedded], range: impl Fn(&Embedded) -> Slots, id: u32) -> Option<usize> {
    (embedded.iter().enumerate())
        .filter(|(_, e)| (range(e).start..range(e).end).contains(&id))
        .min_by_key(|(_, e)| range(e).len())
        .map(|(i, _)| i)
}

/// Lines `first..=last` (from 1) of `text`, or from `first` to its end.
pub(super) fn lines(text: &str, first: u32, last: Option<u32>) -> &str {
    let start = |n: u32| {
        if n <= 1 {
            return Some(0);
        }
        (text.match_indices('\n').nth(n as usize - 2)).map(|(i, _)| i + 1)
    };
    let Some(from) = start(first) else {
        return "";
    };
    let to = last.and_then(|l| start(l + 1)).unwrap_or(text.len());
    &text[from..to.max(from)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wiki(s: &str) -> LocalTarget {
        LocalTarget::Wiki(s.into())
    }

    fn path(s: &str) -> LocalTarget {
        LocalTarget::Path(s.into())
    }

    #[test]
    fn the_scan_finds_note_embeds() {
        let text = "![[a]] and ![[b#Head|alias]], ![[pic.png]], ![[doc.pdf]], ![[a#x]] ![[s.mp3]]\n\
                    ![alt [nested]](sub/c.md#sec) ![x](d%20e.md) ![x](<f g.md>) ![x](h.png)\n\
                    ![x](https://e.org/i.md) [link](j.md) ![[unclosed\n]]";
        assert_eq!(
            targets(text),
            [
                wiki("a"),
                wiki("b"),
                path("sub/c.md"),
                path("d e.md"),
                path("f g.md"),
            ]
        );
        assert!(targets("no embeds here, ![ alone").is_empty());
        let notes =
            "![[s.opus]] ![[t.base]] ![[Chapter 1.1]] ![[Dr. Who]] ![[x.excalidraw]] ![[v.2.md]]";
        assert_eq!(
            targets(notes),
            [
                wiki("Chapter 1.1"),
                wiki("Dr. Who"),
                wiki("x.excalidraw"),
                wiki("v.2.md")
            ]
        );
    }

    #[test]
    fn a_load_gathers_the_notes_embedded() {
        let dir = std::env::temp_dir().join(format!("ada-embeds-{}", std::process::id()));
        fs::create_dir_all(dir.join("sub")).unwrap();
        let write = |name: &str, text: &str| fs::write(dir.join(name), text).unwrap();
        write(
            "host.md",
            "![[a]] ![[a#Part]] ![](sub/b.md) ![[gone]] ![[pic.png]]\n",
        );
        write("a.md", "![[host]] ![[sub/b]]\n");
        write("sub/b.md", "![[c]]\n");
        write("sub/c.md", "deep\n");
        let host = dir.join("host.md");
        let text = fs::read_to_string(&host).unwrap();
        let embeds = gather(&host, &text, &crate::links::BuiltinResolver);
        let read: Vec<_> = (embeds.0.iter())
            .map(|n| {
                let rel = |p: &Path| p.strip_prefix(&dir).unwrap().to_string_lossy().into_owned();
                (rel(&n.from), n.target.clone(), rel(&n.path))
            })
            .collect();
        let s = String::from;
        assert_eq!(
            read,
            [
                (s("host.md"), wiki("a"), s("a.md")),
                (s("host.md"), path("sub/b.md"), s("sub/b.md")),
                // the host isn't read again; b is, once, from where a names it
                (s("a.md"), wiki("sub/b"), s("sub/b.md")),
                (s("sub/b.md"), wiki("c"), s("sub/c.md")),
            ]
        );
        assert_eq!(&*embeds.0[3].text, "deep\n");
        assert!(embeds.0.iter().all(|n| n.mtime.is_some()));
        assert_eq!(embeds.files().len(), 3);
        assert!(gather(&host, "no embeds\n", &crate::links::BuiltinResolver).is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lines_of_a_text() {
        let text = "one\ntwo\nthree\nfour";
        assert_eq!(lines(text, 2, Some(3)), "two\nthree\n");
        assert_eq!(lines(text, 3, None), "three\nfour");
        assert_eq!(lines(text, 1, Some(1)), "one\n");
        assert_eq!(lines(text, 9, None), "");
    }
}
