//! The effect runner: executes [`Cmd`]s and turns their results into messages. This and the
//! input thread are the only places that touch the outside world.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

use ratatui_image::picker::Picker;

use super::{Cmd, Msg};
use crate::filetype::{self, FileType};
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
    /// The live-reload watcher; without it, nothing is watched.
    watcher: Option<Sender<(PathBuf, Option<u64>)>>,
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
            watcher: None,
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
        self.watcher = Some(spawn_watcher(tx.clone()));
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
            Cmd::Watch { path, mtime } => {
                if let Some(watcher) = &self.watcher {
                    let _ = watcher.send((path, mtime));
                }
                None
            }
            // The terminal's: the interactive loop runs them, and a replay never does.
            Cmd::SpawnEditor { .. } | Cmd::Osc52(_) | Cmd::Quit => None,
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

/// Read a document: `DocLoaded` with its text, or `DocFailed`. An image has no text, and a
/// binary file fails.
pub fn load_doc(path: &Path, stdin: Option<&Arc<str>>) -> Msg {
    if path == Path::new("-") {
        return Msg::DocLoaded {
            path: path.to_owned(),
            mtime: None,
            text: stdin.cloned().unwrap_or_default(),
        };
    }
    // The time first: a write landing while we read leaves a newer time for the watcher.
    let mtime = mtime_of(path);
    let filetype = FileType::by_path(path);
    // An image is read by the image worker; its document is only its name.
    let read = match filetype {
        Some(FileType::Image) => fs::metadata(path).map(|_| Vec::new()),
        _ => fs::read(path),
    };
    match read {
        Ok(bytes) if filetype != Some(FileType::Markdown) && filetype::is_binary(&bytes) => {
            Msg::DocFailed {
                path: path.to_owned(),
                error: "binary file".into(),
            }
        }
        Ok(bytes) => {
            let text = match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
            };
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

/// A file's modification time in milliseconds since the epoch.
fn mtime_of(path: &Path) -> Option<u64> {
    let modified = fs::metadata(path).and_then(|m| m.modified()).ok()?;
    let since = modified.duration_since(UNIX_EPOCH).ok()?;
    Some(since.as_millis() as u64)
}

/// How often the watcher looks at the file (ADR-14).
const WATCH_EVERY: Duration = Duration::from_secs(1);

/// Start the live-reload watcher: once it has a file, it `stat`s it every second and posts
/// `FileChanged` when its modification time moves. Until then it sleeps in `recv()`.
fn spawn_watcher(tx: Sender<Msg>) -> Sender<(PathBuf, Option<u64>)> {
    let (files, rx) = mpsc::channel::<(PathBuf, Option<u64>)>();
    thread::spawn(move || {
        let mut watched: Option<(PathBuf, Option<u64>)> = None;
        loop {
            let next = match watched {
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
                Some(_) => rx.recv_timeout(WATCH_EVERY),
            };
            match next {
                Ok(file) => watched = Some(file),
                Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {
                    let Some((path, known)) = &mut watched else {
                        continue;
                    };
                    // a file being replaced may be missing for a moment
                    let Some(mtime) = mtime_of(path) else {
                        continue;
                    };
                    if Some(mtime) != *known {
                        *known = Some(mtime);
                        let msg = Msg::FileChanged {
                            path: path.clone(),
                            mtime: Some(mtime),
                        };
                        if tx.send(msg).is_err() {
                            return;
                        }
                    }
                }
            }
        }
    });
    files
}

/// Split an editor setting the way a shell would: words, `'single'` and `"double"` quotes,
/// and backslash escapes, so `EDITOR="code --wait"` works (ADR-18).
fn shell_words(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut chars = s.chars();
    let mut quote = None;
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, c) if c.is_whitespace() => words.extend(word.take()),
            (None, '\'' | '"') => {
                quote = Some(c);
                word.get_or_insert_default();
            }
            (Some(q), c) if c == q => quote = None,
            (None | Some('"'), '\\') => {
                if let Some(next) = chars.next() {
                    word.get_or_insert_default().push(next);
                }
            }
            (_, c) => word.get_or_insert_default().push(c),
        }
    }
    words.extend(word);
    words
}

/// The command line that opens `file` at `line` in `editor` (a `$VISUAL` / `$EDITOR` value):
/// `+{line} {file}` for the editors that take it, `{file}:{line}` for helix, and just `{file}`
/// for the rest. `None` when the setting is blank.
pub fn editor_command(editor: &str, file: &Path, line: u32) -> Option<Vec<OsString>> {
    let mut argv: Vec<OsString> = shell_words(editor)
        .into_iter()
        .map(OsString::from)
        .collect();
    let program = Path::new(argv.first()?).file_name()?.to_string_lossy();
    let plus = [
        "vi",
        "vim",
        "nvim",
        "gvim",
        "mvim",
        "view",
        "nano",
        "pico",
        "emacs",
        "emacsclient",
        "kak",
        "micro",
        "joe",
        "jed",
        "ne",
        "mg",
        "vis",
    ];
    if plus.contains(&&*program) {
        argv.push(format!("+{line}").into());
        argv.push(file.into());
    } else if program == "hx" || program == "helix" {
        let mut at = file.as_os_str().to_owned();
        at.push(format!(":{line}"));
        argv.push(at);
    } else {
        argv.push(file.into());
    }
    Some(argv)
}

/// The escape sequence that puts `text` on the clipboard (OSC 52). Inside tmux, the same again
/// wrapped for passthrough, for when tmux doesn't take OSC 52 itself.
pub fn osc52(text: &str, tmux: bool) -> String {
    let seq = format!("\x1b]52;c;{}\x07", base64(text.as_bytes()));
    if tmux {
        let wrapped = seq.replace('\x1b', "\x1b\x1b");
        format!("{seq}\x1bPtmux;{wrapped}\x1b\\")
    } else {
        seq
    }
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editors_get_the_line_the_way_they_take_it() {
        let cmd = |editor| {
            editor_command(editor, Path::new("notes/a b.md"), 42).map(|argv| {
                argv.iter()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(cmd("nvim").unwrap(), ["nvim", "+42", "notes/a b.md"]);
        assert_eq!(
            cmd("/usr/bin/vim -u NONE").unwrap(),
            ["/usr/bin/vim", "-u", "NONE", "+42", "notes/a b.md"]
        );
        assert_eq!(cmd("hx").unwrap(), ["hx", "notes/a b.md:42"]);
        assert_eq!(
            cmd("code --wait").unwrap(),
            ["code", "--wait", "notes/a b.md"]
        );
        assert_eq!(
            cmd("'/opt/My Editor/bin/edit' -w").unwrap(),
            ["/opt/My Editor/bin/edit", "-w", "notes/a b.md"]
        );
        assert_eq!(cmd("   "), None);
    }

    #[test]
    fn shell_words_quote_and_escape() {
        assert_eq!(
            shell_words(r#"a "b c" 'd "e"' f\ g "h\"i" '' x"#),
            ["a", "b c", "d \"e\"", "f g", "h\"i", "", "x"]
        );
    }

    #[test]
    fn base64_pads() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64("ada ✓".as_bytes()), "YWRhIOKckw==");
    }

    #[test]
    fn osc52_inside_tmux() {
        assert_eq!(osc52("hi", false), "\x1b]52;c;aGk=\x07");
        assert_eq!(
            osc52("hi", true),
            "\x1b]52;c;aGk=\x07\x1bPtmux;\x1b\x1b]52;c;aGk=\x07\x1b\\"
        );
    }

    #[test]
    fn the_watcher_reports_changes() {
        let dir = std::env::temp_dir().join(format!("ada-watch-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("w.md");
        fs::write(&path, "a").unwrap();
        let (tx, rx) = mpsc::channel();
        let watcher = spawn_watcher(tx);
        // told an old time: the next look reports the real one
        watcher.send((path.clone(), Some(1))).unwrap();
        let msg = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            msg,
            Msg::FileChanged {
                path: path.clone(),
                mtime: mtime_of(&path)
            }
        );
        // and then stays quiet while nothing changes
        assert!(rx.recv_timeout(Duration::from_millis(1500)).is_err());
        drop(watcher);
        fs::remove_dir_all(&dir).unwrap();
    }
}
