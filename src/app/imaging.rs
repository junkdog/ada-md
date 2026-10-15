//! Images in the document, and the full-screen image view (P6, architecture §9). Once a
//! document's links are resolved, the headers of the images they lead to are read, so layout
//! can reserve rows for block images. Those are decoded as they come near the viewport, and
//! `Enter` on any link to an image shows it full screen.

use std::path::PathBuf;

use ratatui::layout::Size as Cells;

use super::{Cmd, Document, LinkState, ScrollAnchor, State};
use crate::doc::LinkId;
use crate::images::{self, Encoded, ImageKey, Job, Probe};
use crate::links;

/// An image file a link leads to, with its header read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageFile {
    pub path: PathBuf,
    pub mtime: Option<u64>,
    /// Pixels, upright.
    pub px: (u32, u32),
}

impl ImageFile {
    pub fn key(&self, size: Cells) -> ImageKey {
        ImageKey {
            path: self.path.clone(),
            mtime: self.mtime,
            size,
        }
    }
}

/// The full-screen view of one image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageView {
    pub link: LinkId,
    pub file: ImageFile,
}

/// The image files a document's links lead to, without duplicates, in link order.
fn image_paths(doc: &Document) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for state in &doc.links {
        if let LinkState::File(path) = state
            && links::is_image(path)
            && !paths.contains(path)
        {
            paths.push(path.clone());
        }
    }
    paths
}

impl State {
    fn doc_by_gen(&self, doc_gen: u64) -> Option<&Document> {
        self.doc
            .iter()
            .chain(self.cache.iter())
            .find(|d| d.generation == doc_gen)
    }

    /// Read the headers of the images a document's links lead to, once they are resolved.
    pub(super) fn probe_cmd(&self, doc_gen: u64) -> Option<Cmd> {
        self.graphics?;
        let paths = image_paths(self.doc_by_gen(doc_gen)?);
        (!paths.is_empty()).then_some(Cmd::ProbeImages { doc_gen, paths })
    }

    /// The headers have been read, in the order [`State::probe_cmd`] asked: lay out again with
    /// the block images' sizes.
    pub(super) fn images_probed(&mut self, doc_gen: u64, probes: &[Probe]) {
        let params = self.params();
        let current = self.doc.as_ref().is_some_and(|d| d.generation == doc_gen);
        let anchor = self
            .doc
            .as_ref()
            .filter(|_| current)
            .and_then(|d| ScrollAnchor::capture(&d.layout, self.scroll));
        let doc = self
            .doc
            .iter_mut()
            .chain(self.cache.iter_mut())
            .find(|d| d.generation == doc_gen);
        let Some(doc) = doc else {
            return;
        };
        let paths = image_paths(doc);
        for (id, state) in doc.links.iter().enumerate() {
            let LinkState::File(path) = state else {
                continue;
            };
            let probe = paths
                .iter()
                .position(|p| p == path)
                .and_then(|i| probes.get(i));
            let file = match probe {
                Some(&Probe::Image { w, h, mtime }) => Some(ImageFile {
                    path: path.clone(),
                    mtime,
                    px: (w, h),
                }),
                _ => None,
            };
            doc.measure
                .set_image(id as LinkId, file.as_ref().map(|f| f.px));
            doc.images[id] = file;
        }
        doc.relayout_now(&self.theme, params);
        if current {
            let layout = &doc.layout;
            self.scroll = anchor.map_or(0, |a| a.restore(layout));
            self.scroll = self.scroll.min(self.max_scroll());
            self.reproject();
        }
    }

    /// The key of the image the view shows, fitted to the screen.
    pub fn view_key(&self) -> Option<ImageKey> {
        let view = self.view.as_ref()?;
        let graphics = self.graphics?;
        let max = Cells::new(self.size.w, self.viewport_height() as u16);
        Some(view.file.key(images::fit(view.file.px, graphics.cell, max)))
    }

    /// Decode jobs for the view, the block images on screen, and those within a page of it.
    pub(super) fn image_jobs(&mut self) -> Vec<Cmd> {
        if self.graphics.is_none() {
            return Vec::new();
        }
        let mut wanted: Vec<(ImageKey, bool)> =
            self.view_key().map(|k| (k, true)).into_iter().collect();
        if let Some(doc) = &self.doc {
            let height = self.viewport_height();
            let screen = self.scroll..self.scroll + height;
            let near = self.scroll.saturating_sub(height)..self.scroll + 2 * height;
            for slot in doc
                .layout
                .images
                .iter()
                .filter(|s| s.overlaps(near.clone()))
            {
                if let Some(Some(file)) = doc.images.get(slot.link as usize) {
                    wanted.push((file.key(slot.size), slot.overlaps(screen.clone())));
                }
            }
        }
        wanted
            .into_iter()
            .filter(|(key, _)| self.images.request(key))
            .map(|(key, urgent)| Cmd::DecodeImage(Job { key, urgent }))
            .collect()
    }

    pub(super) fn image_ready(&mut self, key: ImageKey, image: Option<Result<Encoded, String>>) {
        // A replay has no image to show: the placeholder stays.
        if let Some(result) = image {
            self.images.insert(key, result);
        }
    }

    /// `Enter` on a link to an image: show it full screen, if the terminal can.
    pub(super) fn open_view(&mut self, link: LinkId) -> Option<Vec<Cmd>> {
        self.graphics?;
        let file = self.doc.as_ref()?.images.get(link as usize)?.clone()?;
        self.view = Some(ImageView { link, file });
        Some(self.image_jobs())
    }

    /// `Enter` in the view: hand the image to the system's viewer.
    pub(super) fn view_external(&mut self) -> Vec<Cmd> {
        let Some(view) = &self.view else {
            return Vec::new();
        };
        let path = view.file.path.clone();
        self.status = Some("opening it externally".into());
        vec![Cmd::OpenWithSystem(path.into_os_string())]
    }
}
