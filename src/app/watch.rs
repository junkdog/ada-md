//! Live reload (ADR-26): posts `FileChanged` when the open file changes on disk. It waits on
//! the system for the file's directory, inotify on Linux and `ReadDirectoryChangesW` on Windows,
//! so a save shows within the time a reload takes, and an idle ada doesn't wake at all.
//! Elsewhere, on a network filesystem, or when the system's watch fails, it `stat`s the file
//! every second (ADR-14).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use super::Msg;
use crate::images::mtime_of;

#[cfg(target_os = "linux")]
use inotify::Inotify as Native;
#[cfg(windows)]
use read_changes::ReadChanges as Native;

/// A file to watch, with the time of the text on screen; `None` stops watching.
type File = Option<(PathBuf, Option<u64>)>;

pub struct Watcher {
    tx: Sender<Msg>,
    /// The system's watch, started on the first file; `None` inside when it can't be had.
    #[cfg(any(target_os = "linux", windows))]
    native: Option<Option<Native>>,
    /// The polling thread, started on the first file the system's watch can't watch.
    poll: Option<Sender<File>>,
}

impl Watcher {
    pub fn new(tx: Sender<Msg>) -> Self {
        Self {
            tx,
            #[cfg(any(target_os = "linux", windows))]
            native: None,
            poll: None,
        }
    }

    /// Watch `path` instead of the file watched until now; `mtime` is the time of the text
    /// on screen. A file already past it is posted at once, as the poll's first look would:
    /// it changed while live reload was off, or between its read and the watch.
    pub fn watch(&mut self, path: PathBuf, mut mtime: Option<u64>) {
        if !post_if_changed(&path, &mut mtime, &self.tx) {
            return;
        }
        #[cfg(any(target_os = "linux", windows))]
        {
            let tx = &self.tx;
            let native = self
                .native
                .get_or_insert_with(|| Native::spawn(tx.clone()).ok());
            if let Some(native) = native {
                if native.watch(&path, mtime) {
                    if let Some(poll) = &self.poll {
                        let _ = poll.send(None);
                    }
                    return;
                }
                native.stop();
            }
        }
        let tx = &self.tx;
        let poll = self.poll.get_or_insert_with(|| spawn_poll(tx.clone()));
        let _ = poll.send(Some((path, mtime)));
    }

    /// Stop watching, with live reload turned off; the threads stay for the next watch.
    pub fn unwatch(&mut self) {
        #[cfg(any(target_os = "linux", windows))]
        if let Some(Some(native)) = &self.native {
            native.stop();
        }
        if let Some(poll) = &self.poll {
            let _ = poll.send(None);
        }
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

/// What a `ReadDirectoryChangesW` read's records (`FILE_NOTIFY_INFORMATION`s) say about the
/// file `name`, in lowercase: whether it was renamed into place, as editors save, and whether
/// it's being written. No records means more changed than the buffer held, the file perhaps.
#[cfg(any(windows, test))]
fn notified(records: &[u8], name: &str) -> (bool, bool) {
    // `FILE_ACTION_ADDED`, `FILE_ACTION_MODIFIED` and `FILE_ACTION_RENAMED_NEW_NAME`
    const ADDED: u32 = 1;
    const MODIFIED: u32 = 3;
    const RENAMED_NEW_NAME: u32 = 5;
    let word = |at: usize| {
        records
            .get(at..at + 4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_le_bytes)
    };
    let (mut done, mut writing) = (false, records.is_empty());
    let mut at = 0;
    // each record: the next one's offset, the action, the name's length in bytes, the name
    while let (Some(next), Some(action), Some(len)) = (word(at), word(at + 4), word(at + 8)) {
        let Some(bytes) = records.get(at + 12..at + 12 + len as usize) else {
            break;
        };
        let units = bytes.as_chunks().0.iter().copied().map(u16::from_le_bytes);
        let file: String = char::decode_utf16(units)
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect();
        // NTFS compares names without case; a name matched wrongly costs one `stat`
        if file.to_lowercase() == name {
            match action {
                RENAMED_NEW_NAME => (done, writing) = (true, false),
                ADDED | MODIFIED => writing = true,
                _ => {}
            }
        }
        if next == 0 {
            break;
        }
        at += next as usize;
    }
    (done, writing)
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

/// Windows' inotify: `ReadDirectoryChangesW` on the file's directory, read overlapped by a
/// thread that also waits for the next file to watch. The system has no `CLOSE_WRITE`: every
/// write settles first, and only a rename into place is done at once.
#[cfg(windows)]
mod read_changes {
    use std::fs;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Component, Path, PathBuf, Prefix};
    use std::ptr::{null, null_mut};
    use std::sync::Arc;
    use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{
        CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY,
        FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetDriveTypeW, OPEN_EXISTING,
        ReadDirectoryChangesW,
    };
    use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
    use windows_sys::Win32::System::Threading::{
        CreateEventW, INFINITE, ResetEvent, SetEvent, WaitForMultipleObjects,
    };

    use super::{Msg, notified, post_if_changed};

    /// How long a file has to stay quiet after a write. A file renamed into place is done at
    /// once.
    const SETTLE: Duration = Duration::from_millis(50);

    /// The file being watched: its directory, its name there in lowercase, and the time of
    /// the text on screen.
    struct Target {
        dir: PathBuf,
        name: String,
        path: PathBuf,
        known: Option<u64>,
    }

    /// What the reading thread is told; each `Watch` is answered, true when it's watched.
    enum Cmd {
        Watch(Target),
        Stop,
        Quit,
    }

    /// The reading thread, and the way to tell it things.
    pub struct ReadChanges {
        cmds: Sender<Cmd>,
        acks: Receiver<bool>,
        /// Set after each `Cmd`, to end the thread's wait.
        wake: Arc<Event>,
        thread: Option<JoinHandle<()>>,
    }

    impl ReadChanges {
        pub fn spawn(tx: Sender<Msg>) -> io::Result<Self> {
            let wake = Arc::new(Event::new(false)?);
            let (cmds, rx) = mpsc::channel();
            let (ack, acks) = mpsc::channel();
            let woken = wake.clone();
            let thread = thread::Builder::new()
                .name("watch".into())
                .spawn(move || read(&rx, &ack, &woken, &tx))?;
            Ok(Self {
                cmds,
                acks,
                wake,
                thread: Some(thread),
            })
        }

        /// Watch `path` through its directory, which sees editors that save by writing a new
        /// file and renaming it over the old one. False when the system can't do it.
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
            if remote(dir) {
                return false;
            }
            let target = Target {
                dir: dir.to_owned(),
                name: name.to_string_lossy().to_lowercase(),
                path: path.to_owned(),
                known: mtime,
            };
            self.send(Cmd::Watch(target)) && self.acks.recv().unwrap_or(false)
        }

        /// Stop watching, for a file that's polled instead.
        pub fn stop(&self) {
            self.send(Cmd::Stop);
        }

        fn send(&self, cmd: Cmd) -> bool {
            // SAFETY: the event is open while `self.wake` lives
            self.cmds.send(cmd).is_ok() && unsafe { SetEvent(self.wake.0) } != 0
        }
    }

    impl Drop for ReadChanges {
        /// Wait for the thread to close the directory, so it can go at once.
        fn drop(&mut self) {
            if self.send(Cmd::Quit)
                && let Some(thread) = self.thread.take()
            {
                let _ = thread.join();
            }
        }
    }

    /// Whether `dir` is on another machine, whose writes the system may not hear of: a UNC
    /// path or a mapped drive. Their files are polled, as on Linux.
    fn remote(dir: &Path) -> bool {
        // `GetDriveTypeW`'s answer for a network drive, from `Win32_System_WindowsProgramming`
        const DRIVE_REMOTE: u32 = 4;
        let Some(Component::Prefix(prefix)) = dir.components().next() else {
            return false;
        };
        match prefix.kind() {
            Prefix::UNC(..) | Prefix::VerbatimUNC(..) => true,
            Prefix::Disk(d) | Prefix::VerbatimDisk(d) => {
                let root = [u16::from(d), u16::from(b':'), u16::from(b'\\'), 0];
                // SAFETY: `root` is NUL-terminated and outlives the call
                unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_REMOTE }
            }
            _ => false,
        }
    }

    /// An event object.
    struct Event(HANDLE);

    // SAFETY: an event can be set and waited on from any thread
    unsafe impl Send for Event {}
    unsafe impl Sync for Event {}

    impl Event {
        /// A manual reset event stays set until reset, as an overlapped read wants.
        fn new(manual: bool) -> io::Result<Self> {
            // SAFETY: no security attributes or name
            let handle = unsafe { CreateEventW(null(), manual.into(), 0, null()) };
            if handle.is_null() {
                Err(io::Error::last_os_error())
            } else {
                Ok(Self(handle))
            }
        }
    }

    impl Drop for Event {
        fn drop(&mut self) {
            // SAFETY: the handle is open until here
            unsafe { CloseHandle(self.0) };
        }
    }

    /// A directory open for its changes, and the read pending on it. The buffer and the
    /// `OVERLAPPED` are boxed, to stay where the system writes them while the read is pending.
    struct Dir {
        path: PathBuf,
        handle: HANDLE,
        /// Declared after `handle`, to close after it: the pending read sets it.
        event: Event,
        /// 16 KiB, DWORD-aligned as the call wants; over a network, 64 KiB at most.
        buf: Box<[u32; 4096]>,
        overlapped: Box<OVERLAPPED>,
        pending: bool,
    }

    impl Dir {
        /// Open `path` and start reading its changes; `None` when its filesystem has none to
        /// give.
        fn open(path: &Path) -> Option<Self> {
            let event = Event::new(true).ok()?;
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
            // SAFETY: `wide` is NUL-terminated and outlives the call; no security attributes
            // or template. Shared for deletes too, so the directory and its files can go.
            let handle = unsafe {
                CreateFileW(
                    wide.as_ptr(),
                    FILE_LIST_DIRECTORY,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    null(),
                    OPEN_EXISTING,
                    FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                    null_mut(),
                )
            };
            if handle == INVALID_HANDLE_VALUE {
                return None;
            }
            let overlapped = Box::new(OVERLAPPED {
                hEvent: event.0,
                ..OVERLAPPED::default()
            });
            let mut dir = Self {
                path: path.to_owned(),
                handle,
                event,
                buf: Box::new([0; 4096]),
                overlapped,
                pending: false,
            };
            dir.read().then_some(dir)
        }

        /// Ask for the next changes: names (a creation, a removal, a rename), sizes and
        /// write times. False when the filesystem doesn't give them.
        fn read(&mut self) -> bool {
            let len = size_of_val(&*self.buf) as u32;
            let changes = FILE_NOTIFY_CHANGE_FILE_NAME
                | FILE_NOTIFY_CHANGE_SIZE
                | FILE_NOTIFY_CHANGE_LAST_WRITE;
            // SAFETY: the event is open while `self` lives. The buffer holds `len` bytes, and it
            // and the `OVERLAPPED` are boxed: they stay put until `Drop` has cancelled the read
            // and waited for it to end.
            let started = unsafe {
                // the system clears it too, as `ReadFile` does
                ResetEvent(self.event.0);
                ReadDirectoryChangesW(
                    self.handle,
                    self.buf.as_mut_ptr().cast(),
                    len,
                    0,
                    changes,
                    null_mut(),
                    &raw mut *self.overlapped,
                    None,
                )
            };
            self.pending = started != 0;
            self.pending
        }

        /// What the finished read says about the file `name`, as [`notified`] tells it, once
        /// the next read has started. `None` when either fails, as when the directory is
        /// removed: the watch ends until the next load.
        fn changes(&mut self, name: &str) -> Option<(bool, bool)> {
            let mut len = 0;
            // SAFETY: the read was started on this handle and `OVERLAPPED`; its event is set,
            // so it's over and the call doesn't wait
            let over = unsafe {
                GetOverlappedResult(self.handle, &raw const *self.overlapped, &mut len, 0)
            };
            if over == 0 {
                return None;
            }
            self.pending = false;
            let all = self.buf.as_ptr().cast::<u8>();
            let len = (len as usize).min(size_of_val(&*self.buf));
            // SAFETY: the read is over, and wrote `len` bytes of the buffer
            let records = unsafe { std::slice::from_raw_parts(all, len) };
            let seen = notified(records, name);
            self.read().then_some(seen)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let mut len = 0;
            // SAFETY: a pending read is cancelled and waited for before the handle closes,
            // and before the buffer and `OVERLAPPED` it writes to go
            unsafe {
                if self.pending {
                    CancelIoEx(self.handle, &raw const *self.overlapped);
                    GetOverlappedResult(self.handle, &raw const *self.overlapped, &mut len, 1);
                }
                CloseHandle(self.handle);
            }
        }
    }

    /// The reading thread: sleeps until it's told something or the directory changes, and
    /// posts `FileChanged` for the target's changes once they settle. Returns when told to
    /// quit, or when the app is gone.
    fn read(cmds: &Receiver<Cmd>, acks: &Sender<bool>, wake: &Event, tx: &Sender<Msg>) {
        let mut open: Option<(Dir, Target)> = None;
        // when a write has settled
        let mut due: Option<Instant> = None;
        loop {
            let ms = due.map_or(INFINITE, |d| {
                let left = d.saturating_duration_since(Instant::now());
                u32::try_from(left.as_millis()).unwrap_or(INFINITE - 1)
            });
            let (handles, count) = match &open {
                Some((dir, _)) => ([wake.0, dir.event.0], 2),
                None => ([wake.0, wake.0], 1),
            };
            // SAFETY: the handles are open while `wake` and `open` live, and `count` of them
            // are given
            let woke = unsafe { WaitForMultipleObjects(count, handles.as_ptr(), 0, ms) };
            let (mut done, mut writing) = (false, false);
            match woke {
                WAIT_OBJECT_0 => loop {
                    match cmds.try_recv() {
                        Ok(Cmd::Watch(target)) => {
                            due = None;
                            // the same directory keeps its read
                            let dir = match open.take() {
                                Some((dir, _)) if dir.path == target.dir => Some(dir),
                                _ => Dir::open(&target.dir),
                            };
                            let watched = dir.is_some();
                            open = dir.map(|dir| (dir, target));
                            if acks.send(watched).is_err() {
                                return;
                            }
                        }
                        Ok(Cmd::Stop) => (open, due) = (None, None),
                        Ok(Cmd::Quit) | Err(TryRecvError::Disconnected) => return,
                        Err(TryRecvError::Empty) => break,
                    }
                },
                WAIT_TIMEOUT => (done, due) = (true, None),
                _ if woke == WAIT_OBJECT_0 + 1 => {
                    let Some((dir, target)) = open.as_mut() else {
                        continue;
                    };
                    match dir.changes(&target.name) {
                        Some(seen) => (done, writing) = seen,
                        None => (open, due) = (None, None),
                    }
                }
                _ => return,
            }
            if writing {
                due = Some(Instant::now() + SETTLE);
            } else if done {
                due = None;
            }
            if done
                && let Some((_, t)) = open.as_mut()
                && !post_if_changed(&t.path, &mut t.known, tx)
            {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
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
    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn the_system_reports_each_kind_of_save() {
        use std::io::Write;

        let dir = temp_dir("native");
        let path = dir.join("w.md");
        let other = dir.join("other.md");
        fs::write(&path, "a").unwrap();
        let (tx, rx) = mpsc::channel();
        let mut watcher = Watcher::new(tx);
        watcher.watch(path.clone(), mtime_of(&path));
        assert!(watcher.poll.is_none(), "the system watches it");
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

        // written and left open: posted once the writes stop. Windows may tell of the write
        // only when the file's directory entry catches up, on its close at the latest.
        thread::sleep(Duration::from_millis(5));
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"d").unwrap();
        #[cfg(windows)]
        drop(file);
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

        // a file in another directory: the first one's then says nothing
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).unwrap();
        let deeper = sub.join("z.md");
        fs::write(&deeper, "a").unwrap();
        watcher.watch(deeper.clone(), mtime_of(&deeper));
        thread::sleep(Duration::from_millis(5));
        fs::write(&other, "z").unwrap();
        assert_eq!(next(&rx, Duration::from_millis(200)), None);
        fs::write(&deeper, "b").unwrap();
        assert_eq!(next(&rx, soon), Some(changed(&deeper)));

        #[cfg(target_os = "linux")]
        drop(file);
        // the watch ends with it, and the directories can go
        drop(watcher);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A file already past the time on screen is posted as the watch begins, as live reload
    /// turned back on finds it; `unwatch` stops the posts until the next watch.
    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn a_watch_catches_up_and_unwatch_stops_it() {
        let dir = temp_dir("catch-up");
        let path = dir.join("w.md");
        fs::write(&path, "a").unwrap();
        let (tx, rx) = mpsc::channel();
        let mut watcher = Watcher::new(tx);
        watcher.watch(path.clone(), Some(1));
        assert_eq!(rx.try_recv().ok(), Some(changed(&path)), "posted at once");
        // the time it posted is the one known: the next write is posted, once
        thread::sleep(Duration::from_millis(5));
        fs::write(&path, "b").unwrap();
        assert_eq!(next(&rx, Duration::from_millis(400)), Some(changed(&path)));
        assert_eq!(next(&rx, Duration::from_millis(200)), None);

        watcher.unwatch();
        thread::sleep(Duration::from_millis(5));
        fs::write(&path, "c").unwrap();
        assert_eq!(next(&rx, Duration::from_millis(400)), None, "not watched");
        // watched again, from the time on screen: the write meanwhile is posted
        watcher.watch(path.clone(), Some(1));
        assert_eq!(rx.try_recv().ok(), Some(changed(&path)));
        drop(watcher);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// `FILE_NOTIFY_INFORMATION` records, as a `ReadDirectoryChangesW` read leaves them.
    fn records(changes: &[(u32, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, (action, name)) in changes.iter().enumerate() {
            let name: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
            // DWORD-aligned, the last one's next offset 0
            let len = (12 + name.len()).next_multiple_of(4);
            let next = if i + 1 == changes.len() { 0 } else { len };
            out.extend((next as u32).to_le_bytes());
            out.extend(action.to_le_bytes());
            out.extend((name.len() as u32).to_le_bytes());
            out.extend(name);
            out.resize(out.len().next_multiple_of(4), 0);
        }
        out
    }

    #[test]
    fn notifications_name_the_file() {
        let (added, removed, modified, old, new) = (1, 2, 3, 4, 5);
        // written: settles first
        let written = records(&[(modified, "W.md")]);
        assert_eq!(notified(&written, "w.md"), (false, true));
        // renamed into place, as editors save, after its other name was written
        let saved = records(&[
            (added, ".w.md.swp"),
            (modified, ".w.md.swp"),
            (removed, "w.md"),
            (old, ".w.md.swp"),
            (new, "w.md"),
        ]);
        assert_eq!(notified(&saved, "w.md"), (true, false));
        // another file, or the file going away, says nothing
        let others = records(&[(modified, "other.md"), (removed, "w.md"), (old, "w.md")]);
        assert_eq!(notified(&others, "w.md"), (false, false));
        // a name that isn't ASCII, and a cut-off record ends the walk
        let mut wide = records(&[(modified, "notes"), (new, "Ünïcødé.md")]);
        assert_eq!(notified(&wide, "ünïcødé.md"), (true, false));
        wide.truncate(wide.len() - 4);
        assert_eq!(notified(&wide, "ünïcødé.md"), (false, false));
        // an overflowed buffer: anything may have changed
        assert_eq!(notified(&[], "w.md"), (false, true));
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
