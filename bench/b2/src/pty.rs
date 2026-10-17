//! A viewer in a pty. A reader thread owns the master's read side: it timestamps every chunk the moment
//! `read` returns, answers terminal queries, and hands `(t, chunk)` to the main thread over a channel. The
//! main thread only writes input, marks when, and keeps an emulator current to know when to go on.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use std::{fs, io, ptr};

use crate::replay::Replay;

pub const COLS: u16 = 120;
pub const ROWS: u16 = 40;

/// A non-kitty truecolor terminal, as in B1b: (query, reply).
const REPLIES: &[(&[u8], &[u8])] = &[
    (b"\x1b[c", b"\x1b[?62;22c"),
    (b"\x1b[0c", b"\x1b[?62;22c"),
    (b"\x1b[6n", b"\x1b[40;1R"),
    (b"\x1b[5n", b"\x1b[0n"),
    (b"\x1b]11;?", b"\x1b]11;rgb:2828/2828/2828\x1b\\"),
    (b"\x1b]10;?", b"\x1b]10;rgb:ebeb/dbdb/b2b2\x1b\\"),
    (b"\x1b[14t", b"\x1b[4;800;1200t"),
    (b"\x1b[16t", b"\x1b[6;20;10t"),
    (b"\x1b[18t", b"\x1b[8;40;120t"),
];

static EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Seconds on CLOCK_MONOTONIC (what `Instant` reads on Linux), shared by both threads.
pub fn now() -> f64 {
    EPOCH.elapsed().as_secs_f64()
}

pub type Chunk = (f64, Vec<u8>);

/// Opens a pty of `rows` x `cols`; returns (master, slave), both close-on-exec.
pub fn open_pty(rows: u16, cols: u16) -> io::Result<(File, File)> {
    let (mut m, mut s) = (-1, -1);
    let ws = winsize(rows, cols);
    if unsafe { libc::openpty(&mut m, &mut s, ptr::null_mut(), ptr::null(), &ws) } != 0 {
        return Err(io::Error::last_os_error());
    }
    for fd in [m, s] {
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    Ok(unsafe { (File::from_raw_fd(m), File::from_raw_fd(s)) })
}

fn winsize(rows: u16, cols: u16) -> libc::winsize {
    libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: cols * 10,
        ws_ypixel: rows * 20,
    }
}

/// Every query that ends inside `chunk` (with `tail`, the previous chunk's last bytes, in front), answered
/// in the order it was asked. An OSC is answered with the terminator it used, as xterm does.
pub fn replies(tail: &[u8], chunk: &[u8]) -> Vec<u8> {
    let win = [tail, chunk].concat();
    let mut found: Vec<(usize, Vec<u8>)> = vec![];
    for &(q, a) in REPLIES {
        let mut from = 0;
        while let Some(j) = find(&win[from..], q).map(|j| j + from) {
            if j + q.len() > tail.len() {
                let bel = q.starts_with(b"\x1b]") && win.get(j + q.len()) == Some(&0x07);
                let reply = if bel {
                    [&a[..a.len() - 2], b"\x07"].concat()
                } else {
                    a.to_vec()
                };
                found.push((j, reply));
            }
            from = j + 1;
        }
    }
    found.sort();
    found.into_iter().flat_map(|(_, a)| a).collect()
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

pub fn reader(mut master: File, tx: std::sync::mpsc::Sender<Chunk>) {
    let mut buf = vec![0u8; 65536];
    let mut tail: Vec<u8> = vec![];
    loop {
        let n = match master.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => break, // EIO once every slave fd is closed
        };
        let t = now();
        let chunk = &buf[..n];
        let answer = replies(&tail, chunk);
        if !answer.is_empty() {
            let _ = master.write_all(&answer);
        }
        let win = [&tail[..], chunk].concat();
        tail = win[win.len().saturating_sub(16)..].to_vec();
        if tx.send((t, chunk.to_vec())).is_err() {
            break;
        }
    }
}

pub struct Session {
    pub pid: i32,
    master: File,
    rx: Receiver<Chunk>,
    reader: Option<JoinHandle<()>>,
    pub t0: f64,
    pub chunks: Vec<Chunk>,
    pub marks: Vec<(f64, String)>,
    /// The viewer's CPU ms (all live threads) when each mark was taken.
    pub mark_cpu: Vec<Option<f64>>,
    pub eof: bool,
    /// An emulator kept current between actions, to know when to go on.
    pub live: Replay,
    pub total_cpu_ms: f64,
    pub total_wall_ms: f64,
}

impl Session {
    pub fn spawn(cmd: &[String], env: &BTreeMap<String, String>, cwd: &str) -> io::Result<Session> {
        let (master, slave) = open_pty(ROWS, COLS)?;
        let mut c = Command::new(&cmd[0]);
        c.args(&cmd[1..])
            .env_clear()
            .envs(env)
            .current_dir(cwd)
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave));
        unsafe {
            c.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let t0 = now();
        let child = c.spawn()?; // the parent's slave fds close when `c` drops
        drop(c);
        let pid = child.id() as i32; // reaped with wait4 in close(), for its rusage
        let (tx, rx) = channel();
        let rm = master.try_clone()?;
        let reader = std::thread::Builder::new()
            .name("pty-reader".into())
            .spawn(move || reader(rm, tx))?;
        Ok(Session {
            pid,
            master,
            rx,
            reader: Some(reader),
            t0,
            chunks: vec![],
            marks: vec![],
            mark_cpu: vec![],
            eof: false,
            live: Replay::new(),
            total_cpu_ms: 0.0,
            total_wall_ms: 0.0,
        })
    }

    pub fn set_size(&self, rows: u16, cols: u16) {
        let ws = winsize(rows, cols);
        unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &ws) };
    }

    pub fn bytes(&self) -> usize {
        self.chunks.iter().map(|(_, c)| c.len()).sum()
    }

    /// Collects chunks for up to `timeout` s; true if any arrived.
    pub fn pump(&mut self, timeout: f64) -> bool {
        if self.eof {
            return false;
        }
        match self
            .rx
            .recv_timeout(Duration::from_secs_f64(timeout.max(0.0)))
        {
            Ok(c) => self.chunks.push(c),
            Err(RecvTimeoutError::Timeout) => return false,
            Err(RecvTimeoutError::Disconnected) => {
                self.eof = true;
                return false;
            }
        }
        loop {
            match self.rx.try_recv() {
                Ok(c) => self.chunks.push(c),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.eof = true;
                    break;
                }
            }
        }
        true
    }

    /// Pumps until `quiet` s pass with no output, or `cap` s in all.
    pub fn wait_quiet(&mut self, quiet: f64, cap: f64) -> bool {
        let end = now() + cap;
        let mut last = now();
        while now() < end && !self.eof {
            if self.pump(quiet.min(end - now())) {
                last = now();
            } else if now() - last >= quiet {
                return true;
            }
        }
        false
    }

    /// Pumps until the emulated screen (text and attributes) hasn't changed for `quiet` s and shows `need`,
    /// or `cap` s pass. Output that repaints the same screen (a periodic redraw) doesn't count.
    pub fn wait_stable(&mut self, quiet: f64, cap: f64, need: Option<&str>) -> bool {
        let end = now() + cap;
        let mut last = now();
        let mut sig = self.live.sig();
        while now() < end && !self.eof {
            if self.pump(0.01_f64.min(end - now())) {
                continue; // drain the burst before paying for the emulator
            }
            if self.live.i < self.chunks.len() {
                self.live.advance_to(&self.chunks, f64::INFINITY);
                let s2 = self.live.sig();
                if s2 != sig {
                    (sig, last) = (s2, now());
                }
            }
            if now() - last >= quiet && need.is_none_or(|n| self.live.text().contains(n)) {
                return true;
            }
        }
        false
    }

    fn mark_inner(&mut self, label: String) -> f64 {
        let cpu = self.cpu_ms().ok();
        self.mark_cpu.push(cpu);
        let t = now();
        self.marks.push((t, label));
        t
    }

    /// Writes input without a mark: in a flood, a mark's /proc reads would cost more than the key.
    pub fn write(&self, data: &[u8]) -> io::Result<()> {
        (&self.master).write_all(data)
    }

    pub fn send(&mut self, data: &str, label: impl Into<String>) -> f64 {
        let t = self.mark_inner(label.into());
        let _ = (&self.master).write_all(data.as_bytes());
        t
    }

    pub fn resize(&mut self, rows: u16, cols: u16, label: impl Into<String>) -> f64 {
        let t = self.mark_inner(label.into());
        self.live.advance_to(&self.chunks, f64::INFINITY);
        self.live.resize(rows, cols);
        self.set_size(rows, cols); // the kernel sends SIGWINCH to the foreground group
        t
    }

    pub fn mark(&mut self, label: impl Into<String>) -> f64 {
        self.mark_inner(label.into())
    }

    /// CPU ms of the viewer's live threads, from schedstat (ns).
    pub fn cpu_ms(&self) -> io::Result<f64> {
        let mut ns = 0u64;
        for e in fs::read_dir(format!("/proc/{}/task", self.pid))? {
            let p = e?.path().join("schedstat");
            if let Ok(s) = fs::read_to_string(p) {
                ns += s
                    .split_whitespace()
                    .next()
                    .and_then(|x| x.parse::<u64>().ok())
                    .unwrap_or(0);
            }
        }
        Ok(ns as f64 / 1e6)
    }

    pub fn ctxsw(&self) -> io::Result<u64> {
        let mut n = 0;
        for e in fs::read_dir(format!("/proc/{}/task", self.pid))? {
            if let Ok(s) = fs::read_to_string(e?.path().join("status")) {
                n += s
                    .lines()
                    .filter(|l| l.contains("ctxt_switches"))
                    .filter_map(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
                    .sum::<u64>();
            }
        }
        Ok(n)
    }

    pub fn hwm_kib(&self) -> io::Result<u64> {
        let s = fs::read_to_string(format!("/proc/{}/status", self.pid))?;
        s.lines()
            .find(|l| l.starts_with("VmHWM"))
            .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
            .ok_or_else(|| io::Error::other("no VmHWM"))
    }

    pub fn close(&mut self, quit: &str) {
        let _ = (&self.master).write_all(quit.as_bytes());
        self.wait_quiet(0.1, 0.5);
        unsafe { libc::kill(self.pid, libc::SIGTERM) };
        self.wait_quiet(0.05, 0.3);
        unsafe { libc::kill(self.pid, libc::SIGKILL) };
        let mut status = 0;
        let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
        unsafe { libc::wait4(self.pid, &mut status, 0, &mut ru) };
        let tv = |t: libc::timeval| t.tv_sec as f64 * 1e3 + t.tv_usec as f64 / 1e3;
        self.total_cpu_ms = tv(ru.ru_utime) + tv(ru.ru_stime);
        self.total_wall_ms = (now() - self.t0) * 1e3;
        // anything the viewer left behind in its session still holds the slave open; the reader ends on EIO
        unsafe { libc::kill(-self.pid, libc::SIGKILL) };
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
    }
}
