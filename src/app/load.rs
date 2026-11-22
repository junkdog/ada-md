//! Loading a document off the UI thread (O-32): the loader reads the file, parses it and lays it
//! out at the width the screen had when the load began, then posts it with its text. Meanwhile
//! the document on screen keeps answering keys and resizes. A recording keeps only the text, so
//! a replayed load parses in `update`, as a load does without a loader (ADR-19).

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::thread;

use super::effects::{Stdin, stdin_of};
use super::{Cmd, Computed, Document, Msg, State, load_doc};
use crate::doc::{Embeds, Flavor};
use crate::filetype::FileType;
use crate::layout::{LayoutParams, Sort};
use crate::links::Resolver;
use crate::theme::Glyphs;

/// What the loader needs to prepare a document the way `update` would install it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepare {
    pub params: LayoutParams,
    pub glyphs: Glyphs,
    /// How the document on screen sorts its tables, by table, when it's the same file: a reload
    /// keeps them.
    pub sorts: Vec<Option<Sort>>,
    /// The text ada holds for the file already, on screen or cached, with the notes its embeds
    /// name. If the file and those still read the same, `update` has its document, and nothing is
    /// parsed.
    pub known: Option<(Arc<str>, Embeds)>,
    /// The type `-l` gives the file.
    pub forced: Option<FileType>,
    /// Which markdown a note is read as, with the vault the load finds (ADR-45).
    pub flavor: Flavor,
}

/// A document the loader parsed and laid out, moved into the state when its `DocLoaded` arrives.
#[derive(Clone)]
pub struct Prepared(Arc<Document>);

impl Prepared {
    /// The document, unless the message was cloned meanwhile.
    pub(super) fn take(self) -> Option<Document> {
        Arc::into_inner(self.0)
    }
}

impl fmt::Debug for Prepared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Prepared({})", self.0.path.display())
    }
}

impl PartialEq for Prepared {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Prepared {}

/// A loaded `msg` with its document prepared, unless ada has the text already.
pub fn prepare(msg: Msg, how: &Prepare) -> Msg {
    let Msg::DocLoaded {
        path,
        mtime,
        text,
        vault,
        embeds,
        ..
    } = msg
    else {
        return msg;
    };
    let known = (how.known.as_ref()).is_some_and(|(t, e)| *t == text && *e == embeds);
    let doc = (!known).then(|| {
        let (glyphs, params) = (&how.glyphs, how.params);
        let read = (how.forced, how.flavor.dialect(vault), embeds.clone());
        let (p, t) = (path.clone(), text.clone());
        let mut doc = Document::new(p, mtime, t, glyphs, params, 0, read);
        if doc.set_sorts(|t| how.sorts.get(t as usize).copied().flatten()) {
            doc.relayout_now(glyphs, params);
        }
        Prepared(Arc::new(doc))
    });
    Msg::DocLoaded {
        path,
        mtime,
        text,
        vault,
        embeds,
        doc: Computed(doc),
    }
}

/// A document to load, and how.
pub type Job = (PathBuf, Box<Prepare>);

/// Start the loader: it reads, parses and lays out each document it's sent, in order, and posts
/// its `DocLoaded` or `DocFailed` to `tx`. It finds the notes a note embeds with `resolver`.
pub fn spawn_worker(
    stdin: Stdin,
    resolver: Arc<dyn Resolver + Send + Sync>,
    tx: Sender<Msg>,
) -> Sender<Job> {
    let (jobs, rx) = mpsc::channel::<Job>();
    thread::spawn(move || {
        for (path, how) in rx {
            let loaded = load_doc(&path, stdin_of(&stdin).as_ref(), &*resolver);
            if tx.send(prepare(loaded, &how)).is_err() {
                return;
            }
        }
    });
    jobs
}

impl State {
    /// Load `path`, prepared for the screen as it is now; with nothing on screen, at once.
    pub(super) fn load_cmd(&self, path: PathBuf) -> Cmd {
        let known = self
            .doc
            .iter()
            .chain(&self.cache)
            .find(|d| d.path == path)
            .map(|d| (d.text.clone(), d.embeds.clone()));
        let sorts = self
            .doc
            .as_ref()
            .filter(|d| d.path == path)
            .map(Document::sorts)
            .unwrap_or_default();
        let prepare = Prepare {
            params: self.params(),
            glyphs: self.theme.glyphs.clone(),
            sorts,
            known,
            forced: self.config.forced_type(&path),
            flavor: self.config.flavor,
        };
        Cmd::LoadDoc {
            path,
            prepare: Box::new(prepare),
            wait: self.doc.is_none(),
        }
    }

    /// Whether a navigation waits for its document. In the interactive loop, what would start
    /// another load waits too (O-32).
    pub fn loading(&self) -> bool {
        self.nav.is_some()
    }
}
