//! The terminal probe's replies, read from the tty's raw bytes (the clean-slate plan's CS-2): what
//! the terminal says to kitty's graphics query, the primary device attributes (`CSI c`) and the
//! pixel sizes (`CSI 16t`, `CSI 14t`), until it answers the `CSI 5n` sent last.
//!
//! The probe reads the tty itself, not through crossterm: crossterm's parser turns a reply it
//! doesn't know into keys (kitty's `ESC _ G…` becomes Alt+`_`, then `G`, …). Keys typed while the
//! probe is out arrive mixed with the replies. Those of one byte go on as the key events crossterm
//! would have made of them (printable ASCII, Enter, Tab, Backspace, Ctrl-letters); escape
//! sequences, the arrows and function keys among them, are dropped with the replies.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// How long the probe waits for the replies (Q2 of the clean-slate plan): generous, since only
/// images and the keys typed meanwhile wait for it, and fixed, so keys don't extend it.
pub const WAIT: Duration = Duration::from_secs(2);

/// The queries, in order: kitty's graphics query (a 1×1 RGB image, id 31, `a=q`: checked, not
/// stored), the primary device attributes, the cell's size in pixels, the text area's, and the
/// device status, which every terminal answers, last.
pub const QUERY: &[u8] = b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c\x1b[16t\x1b[14t\x1b[5n";

/// The kitty query's image id.
const KITTY_ID: &str = "31";

/// The most bytes held of one unfinished sequence. A reply to the queries is far shorter; past
/// this, the bytes are noise and are dropped.
const MAX_PENDING: usize = 4096;

/// What the terminal said.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    /// The answer to kitty's graphics query: `Some(true)` for OK, `Some(false)` for an error.
    pub kitty: Option<bool>,
    /// The primary device attributes (`CSI ? Ps ; … c`).
    pub attributes: Option<Vec<u16>>,
    /// The cell's size in pixels, width and height (`CSI 6 ; h ; w t`).
    pub cell: Option<(u16, u16)>,
    /// The text area's size in pixels, width and height (`CSI 4 ; h ; w t`).
    pub text_area: Option<(u16, u16)>,
}

impl Found {
    /// The device attributes name sixel graphics (4).
    pub fn sixel(&self) -> bool {
        self.attributes.as_ref().is_some_and(|a| a.contains(&4))
    }
}

/// The probe's reader: bytes in as they arrive, keys out, until the device status reply ends it.
#[derive(Debug, Default)]
pub struct Replies {
    found: Found,
    done: bool,
    /// The start of a sequence whose end hasn't arrived.
    pending: Vec<u8>,
}

/// One thing read from the bytes.
enum Item {
    Key(KeyEvent),
    /// A reply, or a sequence that's dropped.
    Sequence,
    /// The device status reply: the last of the probe's.
    End,
}

impl Replies {
    pub fn new() -> Replies {
        Replies::default()
    }

    /// Read `bytes`, the next the tty gave, appending the keys among them to `keys`. Bytes after
    /// the device status reply are read too, by the same rule: the caller hands the tty to
    /// crossterm after this call, and what this one read crossterm never sees.
    pub fn feed(&mut self, bytes: &[u8], keys: &mut Vec<KeyEvent>) {
        let mut pending = std::mem::take(&mut self.pending);
        pending.extend_from_slice(bytes);
        let mut at = 0;
        while at < pending.len() {
            let Some((len, item)) = self.next(&pending[at..]) else {
                break;
            };
            match item {
                Some(Item::Key(key)) => keys.push(key),
                Some(Item::Sequence) | None => {}
                Some(Item::End) => self.done = true,
            }
            at += len;
        }
        pending.drain(..at);
        if pending.len() > MAX_PENDING {
            pending.clear();
        }
        self.pending = pending;
    }

    /// The device status reply has arrived: every reply there will be is in.
    pub fn done(&self) -> bool {
        self.done
    }

    /// What the terminal said. An unfinished sequence, if any, is dropped: at the deadline, the
    /// rest of it would reach crossterm as keys, which can't be helped.
    pub fn found(self) -> Found {
        self.found
    }

    /// The first item of `b` and its length in bytes, or `None` when `b` holds the start of a
    /// sequence but not its end. A byte that's neither a key nor the start of a sequence is
    /// read and dropped (`Some((1, None))`).
    fn next(&mut self, b: &[u8]) -> Option<(usize, Option<Item>)> {
        if b[0] != 0x1b {
            return Some((1, key(b[0]).map(Item::Key)));
        }
        match *b.get(1)? {
            b'[' => self.csi(b),
            // APC (kitty's reply), DCS, OSC, SOS, PM: strings that end at ST, or BEL for OSC
            b'_' | b'P' | b']' | b'X' | b'^' => {
                let end = string_end(&b[2..])?;
                if b[1] == b'_' {
                    self.apc(&b[2..2 + end.0]);
                }
                Some((2 + end.0 + end.1, Some(Item::Sequence)))
            }
            // SS3: F1–F4 and the arrows in application mode
            b'O' => b.get(2).map(|_| (3, Some(Item::Sequence))),
            // a lone Esc, then another sequence
            0x1b => Some((1, Some(Item::Sequence))),
            // Alt and a key
            _ => Some((2, Some(Item::Sequence))),
        }
    }

    /// A control sequence: parameters 0x30–0x3F, intermediates 0x20–0x2F, a final byte
    /// 0x40–0x7E. One cut short by another byte is dropped up to that byte.
    fn csi(&mut self, b: &[u8]) -> Option<(usize, Option<Item>)> {
        let mut i = 2;
        loop {
            match *b.get(i)? {
                0x20..=0x3f => i += 1,
                0x40..=0x7e => break,
                _ => return Some((i, Some(Item::Sequence))),
            }
        }
        let (params, last) = (&b[2..i], b[i]);
        let (private, params) = match params.first() {
            Some(&p @ b'<'..=b'?') => (Some(p), &params[1..]),
            _ => (None, params),
        };
        let nums: Vec<u16> = std::str::from_utf8(params)
            .unwrap_or("")
            .split(';')
            .map(|n| n.parse().unwrap_or(0))
            .collect();
        let item = match (private, last, nums.as_slice()) {
            (Some(b'?'), b'c', _) => {
                self.found.attributes = Some(nums);
                Item::Sequence
            }
            (None, b't', &[6, h, w]) => {
                self.found.cell = Some((w, h));
                Item::Sequence
            }
            (None, b't', &[4, h, w]) => {
                self.found.text_area = Some((w, h));
                Item::Sequence
            }
            // 0: ready; 3: a malfunction, but an answer
            (None, b'n', &[0 | 3]) => Item::End,
            _ => Item::Sequence,
        };
        Some((i + 1, Some(item)))
    }

    /// An application program command: kitty's reply is `G` with `i=<id>` among its keys, then
    /// `;OK` or `;` and an error.
    fn apc(&mut self, body: &[u8]) {
        let Some(body) = body.strip_prefix(b"G") else {
            return;
        };
        let body = String::from_utf8_lossy(body);
        let Some((keys, message)) = body.split_once(';') else {
            return;
        };
        if keys.split(',').any(|kv| kv == format!("i={KITTY_ID}")) {
            self.found.kitty = Some(message == "OK");
        }
    }
}

/// Where a string sequence's body ends, and the length of what ends it: ST (`ESC \`) or BEL.
fn string_end(b: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    loop {
        match *b.get(i)? {
            0x07 => return Some((i, 1)),
            0x1b => return (*b.get(i + 1)? == b'\\').then_some((i, 2)).or(Some((i, 0))),
            _ => i += 1,
        }
    }
}

/// Read the replies to [`QUERY`], written already, from the tty until the device status reply
/// arrives or `deadline` passes. Each key typed meanwhile goes to `key` as it's read, which says
/// whether to go on. Call it in raw mode, before crossterm reads anything.
#[cfg(unix)]
pub fn read(deadline: Instant, mut key: impl FnMut(KeyEvent) -> bool) -> Found {
    use std::fs::File;
    use std::io::{ErrorKind, Read};

    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    use rustix::io::Errno;

    let mut replies = Replies::new();
    // the terminal even when stdin is a pipe (`ada -`), and what crossterm reads (`use-dev-tty`)
    let Ok(mut tty) = File::open("/dev/tty") else {
        return replies.found();
    };
    let (mut buf, mut keys) = ([0; 1024], Vec::new());
    while !replies.done() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let timeout = Timespec::try_from(left).unwrap_or(Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        });
        match poll(&mut [PollFd::new(&tty, PollFlags::IN)], Some(&timeout)) {
            Ok(0) | Err(Errno::INTR) => continue,
            Ok(_) => {}
            Err(_) => break,
        }
        let n = match tty.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        replies.feed(&buf[..n], &mut keys);
        for k in keys.drain(..) {
            if !key(k) {
                return replies.found();
            }
        }
    }
    replies.found()
}

/// Read the replies to [`QUERY`], as on unix, from the console's input: `CONIN$`, the console's
/// even when stdin is a pipe, as crossterm opens it. The console hands input over as records,
/// not bytes; with VT input on, its key records carry the terminal's replies and the keys typed
/// meanwhile as the bytes a unix tty would give, Backspace as 0x7f among them. VT input goes off
/// again at the end, a bit at a time: the mouse's capture (crossterm's) sets the whole mode,
/// and may meanwhile.
#[cfg(windows)]
pub fn read(deadline: Instant, mut key: impl FnMut(KeyEvent) -> bool) -> Found {
    use windows_sys::Win32::Foundation::{
        CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{
        ENABLE_VIRTUAL_TERMINAL_INPUT, GetConsoleMode, INPUT_RECORD, KEY_EVENT, ReadConsoleInputW,
        SetConsoleMode,
    };
    use windows_sys::Win32::System::Threading::WaitForSingleObject;

    /// The console's input, with VT input on while it's held.
    struct Input(HANDLE);
    impl Drop for Input {
        fn drop(&mut self) {
            let mut mode = 0;
            // SAFETY: the handle is open until `CloseHandle` here; `mode` outlives the call
            unsafe {
                if GetConsoleMode(self.0, &mut mode) != 0 {
                    SetConsoleMode(self.0, mode & !ENABLE_VIRTUAL_TERMINAL_INPUT);
                }
                CloseHandle(self.0);
            }
        }
    }

    let mut replies = Replies::new();
    let name: Vec<u16> = "CONIN$\0".encode_utf16().collect();
    // SAFETY: `name` is NUL-terminated and outlives the call; no security attributes or template
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return replies.found();
    }
    let input = Input(handle);
    let mut mode = 0;
    // SAFETY: the handle is open while `input` lives; `mode` outlives the call. Should VT input
    // not go on (a console older than Windows 10's), the replies' characters come as keys still.
    unsafe {
        if GetConsoleMode(input.0, &mut mode) != 0 {
            SetConsoleMode(input.0, mode | ENABLE_VIRTUAL_TERMINAL_INPUT);
        }
    }
    let mut records = [INPUT_RECORD::default(); 64];
    let (mut bytes, mut keys) = (Vec::new(), Vec::new());
    while !replies.done() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let ms = u32::try_from(left.as_millis()).unwrap_or(u32::MAX).max(1);
        // SAFETY: the handle is open while `input` lives
        match unsafe { WaitForSingleObject(input.0, ms) } {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => continue,
            _ => break,
        }
        let mut n = 0;
        // SAFETY: `records` holds `len` records and outlives the call, as does `n`; input is
        // waiting, so the call doesn't block
        let read = unsafe {
            ReadConsoleInputW(input.0, records.as_mut_ptr(), records.len() as u32, &mut n)
        };
        if read == 0 {
            break;
        }
        bytes.clear();
        for record in &records[..n as usize] {
            if u32::from(record.EventType) != KEY_EVENT {
                continue;
            }
            // SAFETY: a `KEY_EVENT` record's event is a key event, whose character is UTF-16
            let (down, repeat, unit) = unsafe {
                let k = record.Event.KeyEvent;
                (k.bKeyDown != 0, k.wRepeatCount, k.uChar.UnicodeChar)
            };
            if !down || unit == 0 {
                continue;
            }
            // a lone surrogate half is dropped, as the bytes of anything but ASCII are
            let Some(Ok(c)) = char::decode_utf16([unit]).next() else {
                continue;
            };
            let mut utf8 = [0; 4];
            for _ in 0..repeat.max(1) {
                bytes.extend_from_slice(c.encode_utf8(&mut utf8).as_bytes());
            }
        }
        replies.feed(&bytes, &mut keys);
        for k in keys.drain(..) {
            if !key(k) {
                return replies.found();
            }
        }
    }
    replies.found()
}

/// Without a tty to poll, there is no asking: [`crate::images::plan`] doesn't.
#[cfg(not(any(unix, windows)))]
pub fn read(_deadline: Instant, _key: impl FnMut(KeyEvent) -> bool) -> Found {
    Found::default()
}

/// The key a byte typed on its own is, as crossterm reads it in raw mode, if it's one the probe
/// passes on.
fn key(b: u8) -> Option<KeyEvent> {
    let (code, mods) = match b {
        b'\r' => (KeyCode::Enter, KeyModifiers::NONE),
        b'\t' => (KeyCode::Tab, KeyModifiers::NONE),
        0x7f => (KeyCode::Backspace, KeyModifiers::NONE),
        // Ctrl-A to Ctrl-Z, Ctrl-J among them: in raw mode `\n` is only that
        0x01..=0x1a => (KeyCode::Char((b - 1 + b'a') as char), KeyModifiers::CONTROL),
        b' '..=b'~' => {
            let c = b as char;
            let mods = if c.is_ascii_uppercase() {
                KeyModifiers::SHIFT
            } else {
                KeyModifiers::NONE
            };
            (KeyCode::Char(c), mods)
        }
        _ => return None,
    };
    Some(KeyEvent::new(code, mods))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(chunks: &[&[u8]]) -> (Found, bool, Vec<KeyEvent>) {
        let mut r = Replies::new();
        let mut keys = Vec::new();
        for c in chunks {
            r.feed(c, &mut keys);
        }
        let done = r.done();
        (r.found(), done, keys)
    }

    fn char(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn kittys_replies() {
        let (found, done, keys) =
            read(&[b"\x1b_Gi=31;OK\x1b\\\x1b[?62;22c\x1b[6;20;10t\x1b[4;800;1000t\x1b[0n"]);
        assert!(done && keys.is_empty());
        assert_eq!(
            found,
            Found {
                kitty: Some(true),
                attributes: Some(vec![62, 22]),
                cell: Some((10, 20)),
                text_area: Some((1000, 800)),
            }
        );
        assert!(!found.sixel());
    }

    #[test]
    fn a_sixel_terminal_without_kitty_graphics() {
        // no APC reply, sixel in the attributes, no CSI 16t answer
        let (found, done, _) = read(&[b"\x1b[?65;1;4;6;9;15;22c\x1b[4;480;640t\x1b[0n"]);
        assert!(done && found.sixel());
        assert_eq!((found.kitty, found.cell), (None, None));
        assert_eq!(found.text_area, Some((640, 480)));
    }

    #[test]
    fn a_kitty_error_is_an_answer() {
        let (found, ..) = read(&[b"\x1b_Gi=31;ENOTSUPPORTED:no\x1b\\\x1b[0n"]);
        assert_eq!(found.kitty, Some(false));
        // another image's reply isn't the probe's
        let (found, ..) = read(&[b"\x1b_Gi=7;OK\x1b\\\x1b[0n"]);
        assert_eq!(found.kitty, None);
    }

    #[test]
    fn replies_split_anywhere() {
        let all: &[u8] = b"\x1b_Gi=31;OK\x1b\\\x1b[?62;4c\x1b[6;16;8t\x1b[0n";
        let (whole, ..) = read(&[all]);
        for cut in 1..all.len() {
            let (found, done, keys) = read(&[&all[..cut], &all[cut..]]);
            assert!(done && keys.is_empty(), "cut at {cut}");
            assert_eq!(found, whole, "cut at {cut}");
        }
        // a byte at a time
        let bytes: Vec<&[u8]> = all.chunks(1).collect();
        assert_eq!(read(&bytes).0, whole);
    }

    #[test]
    fn plain_keys_go_on_and_escape_sequences_dont() {
        // j, an arrow, q, Alt-x, Enter, F1 (SS3), Ctrl-C, Tab, an arrow in SS3, Backspace, Shift-G
        let typed = b"j\x1b[Aq\x1bx\r\x1bOP\x03\t\x1bOA\x7fG";
        let (_, done, keys) = read(&[b"\x1b_Gi=31;OK\x1b\\", typed, b"\x1b[0n"]);
        assert!(done);
        assert_eq!(
            keys,
            [
                char('j'),
                char('q'),
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
                KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
                KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT),
            ]
        );
    }

    #[test]
    fn keys_between_replies_and_after_the_end() {
        let (found, done, keys) = read(&[b"\x1b[?62;4ck\x1b[6;16;8t", b"\nj\x1b[0nq"]);
        assert!(done && found.sixel());
        assert_eq!(found.cell, Some((8, 16)));
        assert_eq!(
            keys,
            [
                char('k'),
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL),
                char('j'),
                char('q')
            ]
        );
    }

    #[test]
    fn noise_is_dropped() {
        // a lone Esc before a reply, an unknown CSI, a DCS, an OSC ended by BEL, UTF-8, a NUL, a
        // CSI cut short by the next sequence
        let noise: &[u8] =
            "\x1b\x1b[0;5R\x1bP1$r0m\x1b\\\x1b]11;rgb:0/0/0\x07éx\0\x1b[12\x1b[0n".as_bytes();
        let (found, done, keys) = read(&[noise]);
        assert!(done);
        assert_eq!(found, Found::default());
        assert_eq!(keys, [char('x')]);
    }

    #[test]
    fn an_unfinished_sequence_is_held_then_dropped() {
        let mut r = Replies::new();
        let mut keys = Vec::new();
        r.feed(b"a\x1b_Gi=31;O", &mut keys);
        assert!(!r.done());
        assert_eq!(keys, [char('a')]);
        // a lone Esc at the end waits for what follows it
        r.feed(b"K\x1b\\\x1b", &mut keys);
        assert_eq!(r.found.kitty, Some(true));
        assert_eq!(r.pending, b"\x1b");
        // a string that never ends is let go of
        r.feed(b"P", &mut keys);
        r.feed(&[b'z'; MAX_PENDING], &mut keys);
        assert!(r.pending.is_empty());
        r.feed(b"b", &mut keys);
        assert_eq!(keys, [char('a'), char('b')]);
    }
}
