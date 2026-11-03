//! Feeds recorded chunks through vt100 in order, to answer "when did the screen first show X after t".

use std::hash::{DefaultHasher, Hash, Hasher};
use std::num::NonZeroU16;

use crate::pty::{COLS, Chunk, ROWS};

pub struct Replay {
    parser: vt100::Parser,
    /// The next chunk to feed.
    pub i: usize,
    /// Inside a synchronized update (DEC 2026): begun and not yet ended. vt100 draws as the bytes come, where a
    /// terminal that knows the mode shows nothing of the update until it ends.
    synced: bool,
    /// The last bytes fed, for a mode marker split across two chunks.
    tail: Vec<u8>,
}

const BEGIN_SYNC: &[u8] = b"\x1b[?2026h";
const END_SYNC: &[u8] = b"\x1b[?2026l";

/// What a span of output did to the screen; times in ms after the span's start.
pub struct Scan {
    pub first: Option<f64>,
    /// The last chunk after which the screen (text and attributes) changed.
    pub done: Option<f64>,
    /// When the needle first showed at a frame's end, where the pty went quiet for [`FRAME_GAP`]:
    /// a first frame followed within 2 ms by more writes reads as the end of the burst.
    pub shown: Option<f64>,
    /// When the needle first showed after any chunk that leaves no synchronized update open, the frame's own
    /// end (BEN-003).
    pub shown_chunk: Option<f64>,
    pub bytes: usize,
    pub changed: bool,
}

/// vt100 takes a terminal size as nonzero rows and columns.
fn dim(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n).expect("terminal size")
}

/// Chunks can split a frame; a frame boundary is where the next chunk is more than 2 ms away.
const FRAME_GAP: f64 = 0.002;

impl Replay {
    pub fn new() -> Replay {
        Replay {
            parser: vt100::Parser::new(dim(ROWS), dim(COLS), 0),
            i: 0,
            synced: false,
            tail: Vec::new(),
        }
    }

    pub fn text(&self) -> String {
        let sc = self.parser.screen();
        sc.rows(0, sc.size().1.get()).collect::<Vec<_>>().join("\n")
    }

    pub fn sig(&self) -> u64 {
        let sc = self.parser.screen();
        let mut h = DefaultHasher::new();
        sc.rows_formatted(0, sc.size().1.get())
            .for_each(|r| r.hash(&mut h));
        h.finish()
    }

    pub fn advance_to(&mut self, chunks: &[Chunk], t: f64) {
        while let Some((_, c)) = chunks.get(self.i).filter(|(tc, _)| *tc < t) {
            self.feed(c);
            self.i += 1;
        }
    }

    /// Feed one chunk, keeping track of whether a synchronized update is open after it: the last marker wins.
    fn feed(&mut self, c: &[u8]) {
        self.parser.process(c);
        let mut seen = std::mem::take(&mut self.tail);
        seen.extend_from_slice(c);
        let last = |m: &[u8]| seen.windows(m.len()).rposition(|w| w == m);
        match (last(BEGIN_SYNC), last(END_SYNC)) {
            (Some(b), Some(e)) => self.synced = b > e,
            (Some(_), None) => self.synced = true,
            (None, Some(_)) => self.synced = false,
            (None, None) => {}
        }
        self.tail = seen[seen.len().saturating_sub(BEGIN_SYNC.len() - 1)..].to_vec();
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.screen_mut().set_size(dim(rows), dim(cols));
    }

    /// Feeds the chunks in [t_from, t_to).
    pub fn scan(&mut self, chunks: &[Chunk], t_from: f64, t_to: f64, needle: Option<&str>) -> Scan {
        self.advance_to(chunks, t_from);
        let sig0 = self.sig();
        let mut sig = sig0;
        let mut shown = needle.filter(|n| self.text().contains(n)).map(|_| t_from);
        let mut shown_chunk = shown;
        let (mut first, mut last) = (None, None);
        let mut bytes = 0;
        while let Some((t, c)) = chunks.get(self.i).filter(|(t, _)| *t < t_to) {
            self.feed(c);
            self.i += 1;
            bytes += c.len();
            first = first.or(Some(*t));
            // as a terminal that knows synchronized output shows it: not inside an update
            if shown_chunk.is_none()
                && !self.synced
                && needle.is_some_and(|n| self.text().contains(n))
            {
                shown_chunk = Some(*t);
            }
            let next = chunks.get(self.i).map_or(f64::INFINITY, |(t, _)| *t);
            if next - t > FRAME_GAP {
                let s2 = self.sig();
                if s2 != sig {
                    (sig, last) = (s2, Some(*t));
                }
                if shown.is_none() && needle.is_some_and(|n| self.text().contains(n)) {
                    shown = Some(*t);
                }
            }
        }
        let ms = |x: Option<f64>| x.map(|x| (x - t_from) * 1e3);
        Scan {
            first: ms(first),
            done: ms(last),
            shown: ms(shown),
            shown_chunk: ms(shown_chunk),
            bytes,
            changed: sig != sig0,
        }
    }
}
