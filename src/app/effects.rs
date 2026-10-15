//! The effect runner: executes [`Cmd`]s and turns their results into messages. This and the
//! input thread are the only places that touch the outside world.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use super::{Cmd, Msg};

pub struct Effects {
    /// stdin, read once at startup, served for the path `-`.
    stdin: Option<Arc<str>>,
}

impl Effects {
    pub fn new(stdin: Option<Arc<str>>) -> Self {
        Self { stdin }
    }

    /// Run one command. P1's commands are all quick and synchronous; slow ones (git, highlight,
    /// images) will post their results to the message channel from worker threads instead.
    pub fn run(&mut self, cmd: &Cmd) -> Option<Msg> {
        match cmd {
            Cmd::LoadDoc(path) => Some(load_doc(path, self.stdin.as_ref())),
            Cmd::Quit => None,
        }
    }
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
