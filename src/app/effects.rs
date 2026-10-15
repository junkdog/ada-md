//! The effect runner: executes [`Cmd`]s and turns their results into messages. This and the
//! input thread are the only places that touch the outside world.

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::UNIX_EPOCH;

use ratatui_image::picker::Picker;

use super::{Cmd, Msg};
use crate::git;
use crate::highlight::{self, Job};
use crate::images;
use crate::links::{BuiltinResolver, Resolver};

pub struct Effects {
    /// stdin, read once at startup, served for the path `-`.
    stdin: Option<Arc<str>>,
    /// The highlight worker's queue; without a worker, highlighting runs inline.
    highlight: Option<Sender<Job>>,
    resolver: Box<dyn Resolver>,
    /// Where git runs post their results; without it, git runs inline.
    git: Option<Sender<Msg>>,
    /// The terminal's image support, as queried at startup. Without it, images are encoded as
    /// half-blocks, as `--replay-live` does.
    picker: Option<Picker>,
    /// The image worker's queue; without a worker, images are decoded inline.
    images: Option<Sender<images::Job>>,
}

impl Effects {
    pub fn new(stdin: Option<Arc<str>>) -> Self {
        Self {
            stdin,
            highlight: None,
            resolver: Box::new(BuiltinResolver),
            git: None,
            picker: None,
            images: None,
        }
    }

    /// Encode images for the terminal `picker` describes.
    pub fn set_picker(&mut self, picker: Picker) {
        self.picker = Some(picker);
    }

    fn picker(&mut self) -> &Picker {
        self.picker.get_or_insert_with(Picker::halfblocks)
    }

    /// Start the worker threads, which post their results to `tx`.
    pub fn spawn_workers(&mut self, tx: &Sender<Msg>) {
        self.highlight = Some(highlight::spawn_worker(tx.clone()));
        self.git = Some(tx.clone());
        if let Some(picker) = &self.picker {
            self.images = Some(images::spawn_worker(picker.clone(), tx.clone()));
        }
    }

    /// Run one command. Quick ones answer right away; slow ones go to a worker, which posts
    /// its result to the message channel.
    pub fn run(&mut self, cmd: Cmd) -> Option<Msg> {
        match cmd {
            Cmd::LoadDoc(path) => Some(load_doc(&path, self.stdin.as_ref())),
            Cmd::Highlight(job) => match &self.highlight {
                Some(worker) => worker.send(job).err().map(|e| e.0.run()),
                None => Some(job.run()),
            },
            Cmd::ResolveLinks {
                doc_gen,
                source,
                targets,
            } => Some(Msg::LinksResolved {
                doc_gen,
                resolved: targets
                    .iter()
                    .map(|t| self.resolver.resolve(&source, t))
                    .collect(),
            }),
            Cmd::OpenWithSystem(target) => {
                open_with_system(&target).err().map(|e| Msg::OpenFailed {
                    target: target.to_string_lossy().into_owned(),
                    error: e.to_string(),
                })
            }
            Cmd::RunGit { doc_gen, path } => {
                let diff = move || {
                    git::run(&path).map(|d| Msg::GitDiff {
                        doc_gen,
                        untracked: d.untracked,
                        total: d.total.into(),
                        unstaged: d.unstaged.into(),
                    })
                };
                match &self.git {
                    Some(tx) => {
                        let tx = tx.clone();
                        thread::spawn(move || diff().map(|msg| tx.send(msg)));
                        None
                    }
                    None => diff(),
                }
            }
            Cmd::ProbeImages { doc_gen, paths } => Some(Msg::ImagesProbed {
                doc_gen,
                probes: paths.iter().map(|p| images::probe(p)).collect(),
            }),
            Cmd::DecodeImage(job) => {
                let job = match &self.images {
                    Some(worker) => worker.send(job).err()?.0,
                    None => job,
                };
                Some(job.run(self.picker()))
            }
            Cmd::Quit => None,
        }
    }
}

/// Hand a file or URL to the desktop's opener, detached from the terminal.
fn open_with_system(target: &OsStr) -> std::io::Result<()> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let mut child = Command::new(opener)
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    // Reap it whenever it exits, so it doesn't linger as a zombie.
    thread::spawn(move || child.wait());
    Ok(())
}

/// Read a document: `DocLoaded` with its text, or `DocFailed`.
pub fn load_doc(path: &Path, stdin: Option<&Arc<str>>) -> Msg {
    if path == Path::new("-") {
        return Msg::DocLoaded {
            path: path.to_owned(),
            mtime: None,
            text: stdin.cloned().unwrap_or_default(),
        };
    }
    match fs::read(path) {
        Ok(bytes) => {
            let text = match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
            };
            let mtime = fs::metadata(path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64);
            Msg::DocLoaded {
                path: path.to_owned(),
                mtime,
                text: text.into(),
            }
        }
        Err(e) => Msg::DocFailed {
            path: path.to_owned(),
            error: e.to_string(),
        },
    }
}
