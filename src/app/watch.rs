//! Live reload (ADR-26): posts `FileChanged` when the open file changes on disk. On Linux it
//! waits on inotify for the file's directory, so a save shows within the time a reload takes,
//! and an idle ada doesn't wake at all. Elsewhere, on a network filesystem, or when inotify
//! fails, it `stat`s the file every second (ADR-14).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

use super::Msg;

/// A file's modification time in milliseconds since the epoch.
pub fn mtime_of(path: &Path) -> Option<u64> {
    let modified = fs::metadata(path).and_then(|m| m.modified()).ok()?;
    let since = modified.duration_since(UNIX_EPOCH).ok()?;
    Some(since.as_millis() as u64)
}

/// A file to watch, with the time of the text on screen; `None` stops watching.
type File = Option<(PathBuf, Option<u64>)>;

pub struct Watcher {
    tx: Sender<Msg>,
    /// Started on the first file; `None` inside when inotify can't be had.
    #[cfg(target_os = "linux")]
    inotify: Option<Option<inotify::Inotify>>,
    /// The polling thread, started on the first file inotify can't watch.
    poll: Option<Sender<File>>,
}

impl Watcher {
    pub fn new(tx: Sender<Msg>) -> Self {
        Self {
            tx,
            #[cfg(target_os = "linux")]
            inotify: None,
            poll: None,
        }
    }

    /// Watch `path` instead of the file watched until now; `mtime` is the time of the text
    /// on screen.
    pub fn watch(&mut self, path: PathBuf, mtime: Option<u64>) {
        #[cfg(target_os = "linux")]
        {
            let tx = &self.tx;
            let inotify = self
                .inotify
                .get_or_insert_with(|| inotify::Inotify::spawn(tx.clone()).ok());
            if let Some(inotify) = inotify {
                if inotify.watch(&path, mtime) {
                    if let Some(poll) = &self.poll {
                        let _ = poll.send(None);
                    }
                    return;
                }
                inotify.stop();
            }
        }
        let tx = &self.tx;
        let poll = self.poll.get_or_insert_with(|| spawn_poll(tx.clone()));
        let _ = poll.send(Some((path, mtime)));
    }
}

/// How often the polling thread looks at the file (ADR-14).
const POLL_EVERY: Duration = Duration::from_secs(1);

/// Start the polling thread: once it has a file, it `stat`s it every second and posts
/// `FileChanged` when its modification time moves. Without one, it sleeps in `recv()`.
fn spawn_poll(tx: Sender<Msg>) -> Sender<File> {
    let (files, rx) = mpsc::channel::<File>();
    thread::spawn(move || {
        let mut watched: File = None;
        loop {
            let next = match watched {
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
                Some(_) => rx.recv_timeout(POLL_EVERY),
            };
            match next {
                Ok(file) => watched = file,
                Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {
                    let Some((path, known)) = &mut watched else {
                        continue;
                    };
                    if !post_if_changed(path, known, &tx) {
                        return;
                    }
                }
            }
        }
    });
    files
}

/// Post `FileChanged` when `path`'s modification time is no longer `known`. False when the
/// app is gone.
fn post_if_changed(path: &Path, known: &mut Option<u64>, tx: &Sender<Msg>) -> bool {
    // a file being replaced may be missing for a moment
    let Some(mtime) = mtime_of(path) else {
        return true;
    };
    if Some(mtime) == *known {
        return true;
    }
    *known = Some(mtime);
    tx.send(Msg::FileChanged {
        path: path.to_owned(),
        mtime: Some(mtime),
    })
    .is_ok()
}

#[cfg(target_os = "linux")]
mod inotify {
    use std::ffi::{CStr, OsString};
    use std::fs;
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::Sender;
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
    use std::thread;
    use std::time::{Duration, Instant};

    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    use rustix::fd::OwnedFd;
    use rustix::fs::inotify::{self, CreateFlags, ReadFlags, Reader, WatchFlags};
    use rustix::io::Errno;

    use super::{Msg, post_if_changed};

    /// How long a file has to stay quiet after a write that didn't close it. A writer that
    /// closes the file, or renames a new one into place, is done at once.
    const SETTLE: Duration = Duration::from_millis(50);

    /// Filesystems whose files can change on another machine, which inotify never hears of:
    /// NFS, SMB (three kinds), FUSE, 9p, Ceph and AFS. Their files are polled.
    const REMOTE: [u32; 9] = [
        0x6969,
        0x517b,
        0xff53_4d42,
        0xfe53_4d42,
        0x6573_5546,
        0x0102_1997,
        0x00c3_6400,
        0x5346_414f,
        0x6b41_4653,
    ];

    /// The file being watched: its directory's watch, its name in it, and the time of the
    /// text on screen.
    struct Target {
        wd: i32,
        name: OsString,
        path: PathBuf,
        known: Option<u64>,
    }

    /// The inotify instance, shared with the thread that reads it.
    pub struct Inotify {
        fd: Arc<OwnedFd>,
        target: Arc<Mutex<Option<Target>>>,
    }

    impl Inotify {
        pub fn spawn(tx: Sender<Msg>) -> rustix::io::Result<Self> {
            let fd = Arc::new(inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK)?);
            let target = Arc::new(Mutex::new(None));
            let (reader, shared) = (fd.clone(), target.clone());
            thread::spawn(move || read(&reader, &shared, &tx));
            Ok(Self { fd, target })
        }

        /// Watch `path` through its directory, which sees editors that save by writing a new
        /// file and renaming it over the old one. False when inotify can't do it.
        pub fn watch(&self, path: &Path, mtime: Option<u64>) -> bool {
            // through symlinks, to the directory the writes land in
            let real = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
            let Some(name) = real.file_name() else {
                return false;
            };
            let dir = match real.parent() {
                Some(dir) if !dir.as_os_str().is_empty() => dir,
                _ => Path::new("."),
            };
            if rustix::fs::statfs(dir).is_ok_and(|s| REMOTE.contains(&(s.f_type as u32))) {
                return false;
            }
            let events = WatchFlags::CLOSE_WRITE
                | WatchFlags::MOVED_TO
                | WatchFlags::MODIFY
                | WatchFlags::ATTRIB
                | WatchFlags::CREATE
                | WatchFlags::ONLYDIR;
            let Ok(wd) = inotify::add_watch(&*self.fd, dir, events) else {
                return false;
            };
            let old = lock(&self.target).replace(Target {
                wd,
                name: name.to_owned(),
                path: path.to_owned(),
                known: mtime,
            });
            // the same directory keeps its watch
            if let Some(old) = old.filter(|old| old.wd != wd) {
                let _ = inotify::remove_watch(&*self.fd, old.wd);
            }
            true
        }

        /// Stop watching, for a file that's polled instead.
        pub fn stop(&self) {
            if let Some(old) = lock(&self.target).take() {
                let _ = inotify::remove_watch(&*self.fd, old.wd);
            }
        }
    }

    fn lock(target: &Mutex<Option<Target>>) -> MutexGuard<'_, Option<Target>> {
        target.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The reading thread: sleeps until the directory has events, and posts `FileChanged` for
    /// the target's once it's done being written. Returns when the app is gone or inotify fails.
    fn read(fd: &OwnedFd, target: &Mutex<Option<Target>>, tx: &Sender<Msg>) {
        let mut buf = [MaybeUninit::uninit(); 4096];
        // when a write that left the file open has settled
        let mut due: Option<Instant> = None;
        loop {
            let timeout = due.map(|d| {
                let left = d.saturating_duration_since(Instant::now());
                Timespec::try_from(left).unwrap_or(Timespec {
                    tv_sec: 0,
                    tv_nsec: 0,
                })
            });
            match poll(&mut [PollFd::new(fd, PollFlags::IN)], timeout.as_ref()) {
                Ok(_) | Err(Errno::INTR) => {}
                Err(_) => return,
            }
            let (mut done, mut writing) = (false, false);
            {
                let target = lock(target);
                let mut events = Reader::new(fd, &mut buf);
                loop {
                    let event = match events.next() {
                        Ok(event) => event,
                        Err(Errno::AGAIN) => break,
                        Err(Errno::INTR) => continue,
                        Err(_) => return,
                    };
                    let Some(t) = target.as_ref() else {
                        continue;
                    };
                    if event.wd() != t.wd
                        || event.file_name().map(CStr::to_bytes) != Some(t.name.as_bytes())
                    {
                        continue;
                    }
                    if event
                        .events()
                        .intersects(ReadFlags::CLOSE_WRITE | ReadFlags::MOVED_TO)
                    {
                        (done, writing) = (true, false);
                    } else {
                        writing = true;
                    }
                }
            }
            if writing {
                due = Some(Instant::now() + SETTLE);
            } else if due.is_some_and(|d| Instant::now() >= d) {
                (done, due) = (true, None);
            } else if done {
                due = None;
            }
            if done {
                let mut target = lock(target);
                if let Some(t) = target.as_mut()
                    && !post_if_changed(&t.path, &mut t.known, tx)
                {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::Receiver;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ada-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn changed(path: &Path) -> Msg {
        Msg::FileChanged {
            path: path.to_owned(),
            mtime: mtime_of(path),
        }
    }

    /// Wait for the next message; the time a write takes to show, at most `within`.
    fn next(rx: &Receiver<Msg>, within: Duration) -> Option<Msg> {
        rx.recv_timeout(within).ok()
    }

    #[test]
    fn the_poll_reports_changes() {
        let dir = temp_dir("poll");
        let path = dir.join("w.md");
        fs::write(&path, "a").unwrap();
        let (tx, rx) = mpsc::channel();
        let poll = spawn_poll(tx);
        // told an old time: the next look reports the real one
        poll.send(Some((path.clone(), Some(1)))).unwrap();
        assert_eq!(next(&rx, Duration::from_secs(5)), Some(changed(&path)));
        // and then stays quiet while nothing changes
        assert_eq!(next(&rx, Duration::from_millis(1500)), None);
        // until told to stop watching
        poll.send(None).unwrap();
        fs::write(&path, "ab").unwrap();
        assert_eq!(next(&rx, Duration::from_millis(1500)), None);
        drop(poll);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// The writes, each seen in well under the poll's second.
    #[cfg(target_os = "linux")]
    #[test]
    fn inotify_reports_each_kind_of_save() {
        use std::io::Write;

        let dir = temp_dir("inotify");
        let path = dir.join("w.md");
        let other = dir.join("other.md");
        fs::write(&path, "a").unwrap();
        let (tx, rx) = mpsc::channel();
        let mut watcher = Watcher::new(tx);
        watcher.watch(path.clone(), mtime_of(&path));
        assert!(watcher.poll.is_none(), "inotify watches it");
        let soon = Duration::from_millis(400);
        // nothing happened yet
        assert_eq!(next(&rx, Duration::from_millis(200)), None);

        // written in place, like `>`; the time may need to move past the last one's millisecond
        thread::sleep(Duration::from_millis(5));
        fs::write(&path, "b").unwrap();
        assert_eq!(next(&rx, soon), Some(changed(&path)));

        // a new file renamed over it, as editors save
        thread::sleep(Duration::from_millis(5));
        let tmp = dir.join(".w.md.swp");
        fs::write(&tmp, "c").unwrap();
        fs::rename(&tmp, &path).unwrap();
        assert_eq!(next(&rx, soon), Some(changed(&path)));

        // written and left open: posted once the writes stop
        thread::sleep(Duration::from_millis(5));
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"d").unwrap();
        assert_eq!(next(&rx, soon), Some(changed(&path)));

        // another file in the same directory says nothing
        fs::write(&other, "x").unwrap();
        assert_eq!(next(&rx, Duration::from_millis(200)), None);

        // until it's the one watched; the old one then says nothing
        watcher.watch(other.clone(), mtime_of(&other));
        thread::sleep(Duration::from_millis(5));
        fs::write(&path, "e").unwrap();
        assert_eq!(next(&rx, Duration::from_millis(200)), None);
        fs::write(&other, "y").unwrap();
        assert_eq!(next(&rx, soon), Some(changed(&other)));

        drop(file);
        drop(watcher);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A symlink is watched where its target is written, and reported under its own path.
    #[cfg(target_os = "linux")]
    #[test]
    fn inotify_watches_through_a_symlink() {
        let dir = temp_dir("inotify-link");
        let real = dir.join("real");
        fs::create_dir_all(&real).unwrap();
        let file = real.join("w.md");
        fs::write(&file, "a").unwrap();
        let link = dir.join("link.md");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        let (tx, rx) = mpsc::channel();
        let mut watcher = Watcher::new(tx);
        watcher.watch(link.clone(), mtime_of(&link));
        thread::sleep(Duration::from_millis(5));
        fs::write(&file, "b").unwrap();
        assert_eq!(
            next(&rx, Duration::from_millis(400)),
            Some(changed(&link)),
            "posted under the path opened"
        );
        drop(watcher);
        fs::remove_dir_all(&dir).unwrap();
    }
}
