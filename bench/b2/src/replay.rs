//! Feeds recorded chunks through vt100 in order, to answer "when did the screen first show X after t".

use std::hash::{DefaultHasher, Hash, Hasher};

use crate::pty::{COLS, Chunk, ROWS};

pub struct Replay {
    parser: vt100::Parser,
    /// The next chunk to feed.
    pub i: usize,
}

/// What a span of output did to the screen; times in ms after the span's start.
pub struct Scan {
    pub first: Option<f64>,
    /// The last chunk after which the screen (text and attributes) changed.
    pub done: Option<f64>,
    pub shown: Option<f64>,
    pub bytes: usize,
    pub changed: bool,
}

/// Chunks can split a frame; a frame boundary is where the next chunk is more than 2 ms away.
const FRAME_GAP: f64 = 0.002;

impl Replay {
    pub fn new() -> Replay {
        Replay {
            parser: vt100::Parser::new(ROWS, COLS, 0),
            i: 0,
        }
    }

    pub fn text(&self) -> String {
        let sc = self.parser.screen();
        sc.rows(0, sc.size().1).collect::<Vec<_>>().join("\n")
    }

    pub fn sig(&self) -> u64 {
        let sc = self.parser.screen();
        let mut h = DefaultHasher::new();
        sc.rows_formatted(0, sc.size().1)
            .for_each(|r| r.hash(&mut h));
        h.finish()
    }

    pub fn advance_to(&mut self, chunks: &[Chunk], t: f64) {
        while let Some((_, c)) = chunks.get(self.i).filter(|(tc, _)| *tc < t) {
            self.parser.process(c);
            self.i += 1;
        }
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.screen_mut().set_size(rows, cols);
    }

    /// Feeds the chunks in [t_from, t_to).
    pub fn scan(&mut self, chunks: &[Chunk], t_from: f64, t_to: f64, needle: Option<&str>) -> Scan {
        self.advance_to(chunks, t_from);
        let sig0 = self.sig();
        let mut sig = sig0;
        let mut shown = needle.filter(|n| self.text().contains(n)).map(|_| t_from);
        let (mut first, mut last) = (None, None);
        let mut bytes = 0;
        while let Some((t, c)) = chunks.get(self.i).filter(|(t, _)| *t < t_to) {
            self.parser.process(c);
            self.i += 1;
            bytes += c.len();
            first = first.or(Some(*t));
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
            bytes,
            changed: sig != sig0,
        }
    }
}
