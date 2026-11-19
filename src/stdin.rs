//! Piped input's terminal escapes. `git` colors what it pages (`color.diff = auto` colors
//! whatever it pages, `git log --format` its `%C(…)`), and `ls --color`, `grep --color` and
//! loggers write colors too. Stdin is read as it comes, escapes and all; once its type is known
//! (FEAT-022), a diff's are dropped here and ada colors it as it colors a diff file, while other
//! colored text keeps its own colors ([`crate::highlight::ANSI`], drawn from its SGR sequences).
//!
//! Dropped: CSI sequences (SGR's `ESC [ … m` among them, `ESC [ K`), control strings with their
//! payloads (OSC's `ESC ] … BEL`, a hyperlink's or a title's; DCS's `ESC P`, a sixel image's;
//! APC's `ESC _`, a kitty image's; PM's `ESC ^` and SOS's `ESC X`), and other escapes (`ESC ( B`,
//! `ESC =`). A string ends at its terminator, `ESC \\`, BEL or a C1 ST (U+009C, or a lone 0x9C
//! byte); at CAN or SUB, which cancel it; or at the next `ESC`, which starts a sequence of its own,
//! as a terminal's parser reads them (PGR-007). A newline ends any sequence and stays, so the text
//! keeps the input's lines, each its own escapes gone; and a byte past ASCII is never taken for a
//! sequence's last, so the text stays UTF-8 where it was.

use std::borrow::Cow;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::sync::OnceLock;

/// A pipe named on the command line (`ada <(cmd)`, a FIFO), read in stdin's place.
static PIPE: OnceLock<File> = OnceLock::new();

/// Whether `path` is a pipe or another file that can be read only once, as stdin is (PGR-007):
/// not a regular file, and not a directory.
pub fn is_pipe(path: &Path) -> bool {
    path.as_os_str() != "-" && std::fs::metadata(path).is_ok_and(|m| !m.is_file() && !m.is_dir())
}

/// Read `pipe` wherever stdin would be read: piped text is typed, stripped and shown as stdin's
/// is, under the name `-`. Once only; a second pipe is refused.
pub fn read_from(pipe: File) -> Result<(), File> {
    PIPE.set(pipe)
}

/// What ada reads as its piped input: the pipe [`read_from`] gave, else stdin.
pub fn input() -> Box<dyn Read + Send> {
    match PIPE.get() {
        Some(pipe) => Box::new(pipe),
        None => Box::new(io::stdin()),
    }
}

/// Where the reading of escapes is: in text, or in a sequence that started in an earlier piece.
/// [`Strip::bytes`] also notes whether a sequence it dropped moved along the line or erased
/// it, which a terminal shows otherwise than the text without them (PGR-007).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Strip {
    at: At,
    /// The last byte read in a sequence: a CSI's last parameter byte, at its final one.
    last: u8,
    moved: bool,
    erased: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum At {
    #[default]
    Text,
    /// After an `ESC`.
    Escape,
    /// After an `ESC` and an intermediate byte or more (`ESC ( B`, a charset's).
    EscIntermediate,
    /// In a CSI sequence's parameters and intermediates, after `ESC [`.
    Csi,
    /// In a control string's payload (OSC, DCS, APC, PM, SOS): at a character's start, after
    /// a `0xC2` that may start U+009C (ST), or in a character past ASCII, 1 to 3 bytes left.
    Str,
    StrC2,
    StrChar1,
    StrChar2,
    StrChar3,
    /// After an `ESC` in a control string: `\\` ends it, anything else starts a new sequence.
    StrEscape,
}

impl Strip {
    /// Drop `buf`'s escapes in place, from where the reading is on: how many bytes are kept, at
    /// its start.
    pub fn bytes(&mut self, buf: &mut [u8]) -> usize {
        // most text has no escape at all
        if self.at == At::Text && memchr::memchr(0x1b, buf).is_none() {
            return buf.len();
        }
        let (mut r, mut w) = (0, 0);
        while r < buf.len() {
            // text up to the next escape stays whole: moved at once, not a byte at a time
            if self.at == At::Text {
                let next = memchr::memchr(0x1b, &buf[r..]).map_or(buf.len(), |n| r + n);
                buf.copy_within(r..next, w);
                w += next - r;
                r = next;
                if r == buf.len() {
                    break;
                }
            }
            let (b, was) = (buf[r], self.at);
            let (next, keep) = step(was, b);
            if was == At::Csi && next != At::Csi {
                self.note(b);
            }
            self.last = b;
            self.at = next;
            if keep {
                buf[w] = b;
                w += 1;
            }
            r += 1;
        }
        w
    }

    /// A CSI sequence's final byte `b`: a move along the line (`C`, `D`, `G`, `` ` ``, `a`), an
    /// erase (`K`), of the line's start or all of it (`1`, `2`) a move too.
    fn note(&mut self, b: u8) {
        match b {
            b'C' | b'D' | b'G' | b'`' | b'a' => self.moved = true,
            b'K' => {
                self.erased = true;
                self.moved |= matches!(self.last, b'1' | b'2');
            }
            _ => {}
        }
    }

    /// Whether [`Strip::bytes`] dropped a move along the line, or an erase of its start or all
    /// of it, so far.
    pub(crate) fn moved(self) -> bool {
        self.moved
    }

    /// Whether [`Strip::bytes`] dropped an erase (`ESC [ K`) so far.
    pub(crate) fn erased(self) -> bool {
        self.erased
    }

    /// Read one byte: whether it is text that stays.
    #[inline(always)]
    pub(crate) fn byte(&mut self, b: u8) -> bool {
        let (next, keep) = step(self.at, b);
        self.at = next;
        keep
    }

    /// Whether the reading is in a CSI sequence's parameters (after `ESC [`).
    pub(crate) fn in_csi(self) -> bool {
        self.at == At::Csi
    }

    /// Whether the reading is in text, where every byte up to the next `ESC` stays.
    pub(crate) fn in_text(self) -> bool {
        self.at == At::Text
    }

    /// Whether a lone `0x9C` byte, never UTF-8, would end a control string here: in a payload,
    /// at a character's start.
    fn at_c1_st(self) -> bool {
        self.at == At::Str
    }
}

/// The reading after byte `b` from `at`, and whether `b` is text that stays. No call of its own,
/// so that it inlines into the loops that read a byte at a time.
#[inline(always)]
fn step(at: At, b: u8) -> (At, bool) {
    use At::*;
    match (at, b) {
        // a sequence never runs past its line
        (_, b'\n') => (Text, true),
        (Str | StrC2 | StrChar1 | StrChar2 | StrChar3, 0x1b) => (StrEscape, false),
        (_, 0x1b) => (Escape, false),
        (Text, _) => (Text, true),
        // CAN and SUB cancel a sequence, and show nothing
        (Escape | EscIntermediate | Csi | StrEscape, 0x18 | 0x1a) => (Text, false),
        // ST ends a string; any other escape in one ends it and starts its own sequence
        (StrEscape, b'\\') => (Text, false),
        (Escape | StrEscape, b'[') => (Csi, false),
        // OSC, DCS, APC, PM, SOS: a payload, to its terminator
        (Escape | StrEscape, b']' | b'P' | b'_' | b'^' | b'X') => (Str, false),
        // another escape: intermediates (`ESC ( B`'s `(`, a charset's), then a final byte
        (Escape | StrEscape | EscIntermediate, 0x20..=0x2f) => (EscIntermediate, false),
        (Escape | StrEscape | EscIntermediate, 0x30..=0x7e) => (Text, false),
        // parameters and intermediates, then the final byte
        (Csi, 0x20..=0x3f) => (Csi, false),
        (Csi, 0x40..=0x7e) => (Text, false),
        // no sequence's: the text goes on with it (a lone `ESC`, a CSI cut short)
        (Escape | StrEscape | EscIntermediate | Csi, _) => (Text, true),
        // in a string's payload from here on: BEL, CAN and SUB end it
        (_, 0x07 | 0x18 | 0x1a) => (Text, false),
        // U+009C, ST, as UTF-8
        (StrC2, 0x9c) => (Text, false),
        (StrC2 | StrChar1, 0x80..=0xbf) => (Str, false),
        (StrChar2, 0x80..=0xbf) => (StrChar1, false),
        (StrChar3, 0x80..=0xbf) => (StrChar2, false),
        // at a character's start, or one cut short: a lone 0x9C, no UTF-8's, is a C1 ST
        (_, 0x9c) => (Text, false),
        (_, 0xc2) => (StrC2, false),
        (_, 0xc3..=0xdf) => (StrChar1, false),
        (_, 0xe0..=0xef) => (StrChar2, false),
        (_, 0xf0..=0xf7) => (StrChar3, false),
        _ => (Str, false),
    }
}

/// `bytes` as text, what isn't UTF-8 in them replaced (U+FFFD), but for a lone `0x9C` that ends a
/// control string, a C1 ST of an 8-bit program's: it ends it as U+009C, which [`strip`] drops,
/// so that text read as UTF-8 reads as the bytes did (PGR-007).
pub fn text(bytes: Vec<u8>) -> String {
    let bytes = match String::from_utf8(bytes) {
        Ok(text) => return text,
        Err(e) => e.into_bytes(),
    };
    let mut out = String::with_capacity(bytes.len() + 16);
    let mut strip = Strip::default();
    for chunk in bytes.utf8_chunks() {
        let valid = chunk.valid();
        out.push_str(valid);
        // text up to its first escape leaves the reading where it is
        let from = match strip.in_text() {
            true => memchr::memchr(0x1b, valid.as_bytes()).unwrap_or(valid.len()),
            false => 0,
        };
        for &b in &valid.as_bytes()[from..] {
            strip.byte(b);
        }
        let invalid = chunk.invalid();
        if invalid == [0x9c] && strip.at_c1_st() {
            out.push('\u{9c}');
            strip = Strip::default();
        } else if !invalid.is_empty() {
            // one replacement a run, as `String::from_utf8_lossy` has it
            out.push(char::REPLACEMENT_CHARACTER);
            for &r in "\u{fffd}".as_bytes() {
                strip.byte(r);
            }
        }
    }
    out
}

/// Whether `bytes` hold an escape.
pub fn has_escape(bytes: &[u8]) -> bool {
    memchr::memchr(0x1b, bytes).is_some()
}

/// Whether `bytes` strike a character over another with a backspace, as `man` writes bold
/// (`X\bX`) and underline (`_\bX`) to a pager when it writes no colors (`GROFF_NO_SGR`).
pub fn has_overstrike(bytes: &[u8]) -> bool {
    memchr::memchr_iter(0x08, bytes).any(|i| {
        let (Some(&a), Some(&b)) = (i.checked_sub(1).map(|p| &bytes[p]), bytes.get(i + 1)) else {
            return false;
        };
        a.is_ascii_graphic() && b.is_ascii_graphic() && (a == b || a == b'_' || b == b'_')
    })
}

/// `text` without its escapes.
pub fn strip(text: &str) -> Cow<'_, str> {
    if !has_escape(text.as_bytes()) {
        return Cow::Borrowed(text);
    }
    let mut bytes = text.as_bytes().to_vec();
    let n = Strip::default().bytes(&mut bytes);
    bytes.truncate(n);
    // whole characters only go: a sequence starts and ends on ASCII bytes
    Cow::Owned(String::from_utf8(bytes).expect("stripped text is UTF-8"))
}

/// A reader's bytes without their escapes, as [`strip`] drops them, wherever the reads cut.
pub struct Stripped<R> {
    inner: R,
    strip: Strip,
}

impl<R> Stripped<R> {
    /// `inner`'s bytes from where `strip` left the reading of the ones before them.
    pub fn new(inner: R, strip: Strip) -> Self {
        Self { inner, strip }
    }
}

impl<R: Read> Read for Stripped<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let n = self.inner.read(buf)?;
            if n == 0 {
                return Ok(0);
            }
            let kept = self.strip.bytes(&mut buf[..n]);
            // a read that was all escapes isn't the end of the input
            if kept > 0 {
                return Ok(kept);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stripped(parts: &[&[u8]]) -> String {
        let mut strip = Strip::default();
        let mut out = Vec::new();
        for part in parts {
            let mut buf = part.to_vec();
            let n = strip.bytes(&mut buf);
            out.extend_from_slice(&buf[..n]);
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn colors_go() {
        assert_eq!(
            stripped(&[b"\x1b[33mcommit abc\x1b[m\n\x1b[32m+new\x1b[m\n"]),
            "commit abc\n+new\n"
        );
        assert_eq!(stripped(&[b"\x1b[1;38;2;1;2;3mx\x1b[0m y"]), "x y");
        assert_eq!(stripped(&[b"plain, untouched\n"]), "plain, untouched\n");
        assert_eq!(stripped(&[b"a\x1b(Bb\x1b[Kc"]), "abc");
    }

    #[test]
    fn a_sequence_split_between_reads() {
        assert_eq!(stripped(&[b"a\x1b", b"[3", b"2mb\x1b[m"]), "ab");
        assert_eq!(
            stripped(&[b"a\x1b]8;;https://x", b"\x1b\\link\x1b]8;;\x07z"]),
            "alinkz"
        );
    }

    #[test]
    fn a_newline_ends_any_sequence() {
        assert_eq!(stripped(&[b"a\x1b]8;;x\nb\x1b[3\nc\x1b\nd"]), "a\nb\nc\nd");
        assert_eq!(strip("one\x1b]2;title\ntwo").lines().count(), 2);
    }

    /// PGR-007: every control string's payload goes, a kitty image's and a sixel's too, and a
    /// string ends wherever a terminal ends it.
    #[test]
    fn control_strings_go_whole() {
        // APC (kitty graphics), DCS (sixel), PM, SOS, ended by ST or BEL
        assert_eq!(strip("a\x1b_Gf=100,a=T;iVBORw0KGgo=\x1b\\b"), "ab");
        assert_eq!(strip("a\x1bPq#0;2;0;0;0#0~~@@vv@@~~\x1b\\b"), "ab");
        assert_eq!(strip("a\x1b^private\x07b\x1bXstring\x1b\\c"), "abc");
        // U+009C ends one, and so does a lone 0x9C read as bytes
        assert_eq!(strip("a\x1b]0;title\u{9c}b"), "ab");
        assert_eq!(
            text(b"a\x1b]0;t\x9cb\xff".to_vec()),
            "a\x1b]0;t\u{9c}b\u{fffd}"
        );
        assert_eq!(strip(&text(b"a\x1b]0;t\x9cb".to_vec())), "ab");
        assert_eq!(stripped(&[b"a\x1b]0;t\x9cb"]).as_bytes(), b"ab");
        // ... but not a 0x9C inside a character of the payload: `✜` is E2 9C 9C
        assert_eq!(strip("a\x1b]2;✜ title\x07b"), "ab");
        // an escape in a payload ends it and starts its own sequence, CAN and SUB cancel one
        assert_eq!(strip("a\x1b]8;;http://x\x1b[31mb\x1b[m"), "ab");
        assert_eq!(strip("a\x1b]8;;x\x18b\x1b[3\x1ac"), "abc");
        // an unterminated string still ends at its line
        assert_eq!(strip("a\x1b]8;;x text\nb"), "a\nb");
    }

    #[test]
    fn characters_past_ascii_stay_whole() {
        assert_eq!(strip("\x1b[é\x1bü\x1b]ö\x07x"), "éüx");
        assert_eq!(strip("\x1b[31mgrün\x1b[m"), "grün");
    }

    #[test]
    fn a_reader_drops_them_wherever_reads_cut() {
        let text = b"\x1b[31mred\x1b[m\n\x1b]8;;u\x1b\\l\x1b]8;;\x1b\\\n";
        for cut in 0..text.len() {
            let mut first = text[..cut].to_vec();
            let mut strip = Strip::default();
            let kept = strip.bytes(&mut first);
            first.truncate(kept);
            let mut rest = Stripped::new(&text[cut..], strip);
            rest.read_to_end(&mut first).unwrap();
            assert_eq!(first, b"red\nl\n", "cut at {cut}");
        }
    }
}
