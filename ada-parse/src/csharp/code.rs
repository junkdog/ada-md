//! The turns of C#'s code contexts in a loop of their own (X-025's first stage): while the top
//! context is one that passes over blanks (code's), its turns run here a token at a time, blanks
//! passed inline and the hot contexts' patterns dispatched without [`Scan::context`]'s `match`.
//! bat's contexts and their stack stay as they are, so each turn is the one [`Scan::run`] would
//! take, and the state between lines is the same.

use super::c::*;
use super::*;
use crate::clike::{CLASS, SPACE};

impl Scan<'_> {
    /// The turns from `p` (past the line's start) while the top context is code's: blanks passed
    /// inline, each token's patterns tried as [`Scan::run`]'s turn would. Returns where the scan
    /// is when the top isn't code's, the line ends, or the turn at the place is one for
    /// [`Scan::run`] (a comment's start, a character past ASCII, no pattern matching).
    pub(super) fn code(&mut self, mut p: usize) -> usize {
        let (b, n) = (self.b, self.n);
        loop {
            let top = self.top();
            if CTX[usize::from(top)] & INERT == 0 {
                return p;
            }
            while p < n && CLASS[usize::from(b[p])] & SPACE != 0 {
                p += 1;
            }
            if p >= n {
                return p;
            }
            let c = b[p];
            if c >= 0x80 || (c == b'/' && p + 1 < n && matches!(b[p + 1], b'/' | b'*')) {
                return p;
            }
            self.looped = false;
            let Some(q) = self.frame(top, p) else {
                return p;
            };
            if self.halt {
                return q;
            }
            self.stuck = if q == p { self.stuck + 1 } else { 0 };
            p = q;
            self.step = q;
            if self.stuck > STUCK {
                self.stuck = 0;
                if p >= n {
                    return p;
                }
                p = self.next(p);
                self.step = p;
            }
        }
    }

    /// A frame's turn at `p` (not a blank, not a comment's start, ASCII): the hot ones' arms
    /// here, inlined into the loop, the rest [`Scan::context`]'s.
    #[inline(always)]
    fn frame(&mut self, top: u8, p: usize) -> Option<usize> {
        match top {
            LINE_IN => self.line_in(p),
            ARG_EXPR => {
                // `nosemi`'s pop at `,` or `)`, then `arguments`' turn there: its separator or
                // its end
                let c = self.b[p];
                if matches!(c, b',' | b')') && self.stuck < STUCK {
                    let q = self.leave(p)?;
                    if self.top() != ARGUMENTS {
                        return Some(q);
                    }
                    return if c == b',' {
                        self.tok(p, p + 1, ARGUMENT_SEP)
                    } else {
                        self.close(p, p + 1, GROUP_END)
                    };
                }
                if let Some(q) = self.nosemi(p) {
                    return Some(q);
                }
                if self.b[p] == b';' {
                    return self.leave(p);
                }
                None
            }
            ARGUMENTS => self.arguments(p),
            DTB_BLOCK => self.dtb_block(p),
            MB_BLOCK | CODE_BRACE | EB_BRACE => {
                if self.b[p] == b'}' {
                    let leaf = if top == EB_BRACE {
                        M_BLOCK_END
                    } else {
                        BLOCK_END
                    };
                    return self.close(p, p + 1, leaf);
                }
                if top == MB_BLOCK
                    && let Some(q) = self.stray(p)
                {
                    return Some(q);
                }
                // the push's turn, then the statement's at the same place, counted as the loop
                // counts a turn that takes nothing (and left to it where the guard is due)
                let q = self.enter(p, &[LINE])?;
                if self.halt || self.stuck + 1 >= STUCK {
                    return Some(q);
                }
                self.stuck += 1;
                self.line(p)
            }
            IF_GROUP => {
                if self.b[p] == b')' {
                    self.put(p, p + 1, M_GROUP_END);
                    self.pop();
                    return Some(p + 1);
                }
                self.line_in(p)
            }
            LINE => self.line(p),
            _ => self.context(top, p),
        }
    }
}
