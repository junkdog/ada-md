//! The plumbing the stack scanners share (X-017's third part, the parser audit's item 3): a
//! line's stack loaded from the [`State`] the line before left and saved into the one it
//! leaves, the push that drops the bottom level at the stack's limit, and the emitter that
//! fills a context's text up to a token and puts the token after it. Each was a copy in up to
//! sixteen scanners; the scanners whose contract differs (a span pushed over one already out,
//! a kind built from the stack's wrap, spans never joined) keep their own. No behavior of
//! their own: what each did before, the digests hold.

use crate::{State, matcher::KEPT as HALF};

/// How many levels a state holds: its `levels`, then its `withs` as more of them.
pub const HELD: usize = 2 * HALF;

impl State {
    /// How many levels the state holds.
    #[inline(always)]
    pub fn held(&self) -> usize {
        usize::from(self.depth)
    }

    /// Level `i` held, bottom first.
    #[inline(always)]
    pub fn level(&self, i: usize) -> u8 {
        if i < HALF {
            self.levels[i]
        } else {
            self.withs[i - HALF]
        }
    }

    /// Sets level `i` held, bottom first.
    #[inline(always)]
    pub fn set_level(&mut self, i: usize, c: u8) {
        if i < HALF {
            self.levels[i] = c;
        } else {
            self.withs[i - HALF] = c;
        }
    }

    /// The levels held, copied whole into `stack[1..=HELD]` above the stack's base, and the
    /// stack's depth with them: the bytes copied above it are the state's zeros, never read
    /// before a push writes them.
    #[inline(always)]
    pub fn load(&self, stack: &mut [u8]) -> usize {
        stack[1..=HALF].copy_from_slice(&self.levels);
        stack[1 + HALF..=HELD].copy_from_slice(&self.withs);
        1 + self.held()
    }

    /// The state that holds the top [`HELD`] levels of `stack[1..depth]` (the base below them
    /// isn't kept), the bytes past the top zero, and `arg` beside them; inexact where `exact`
    /// is false or a level below them is dropped. `stack` is at least `1 + HELD` long.
    #[inline(always)]
    pub fn keep(stack: &[u8], depth: usize, exact: bool, arg: u16) -> State {
        let n = depth - 1;
        let from = n.saturating_sub(HELD);
        let kept = n - from;
        // the top levels in one copy (`1 + from + HELD` is at most `depth` when any are
        // dropped, else `1 + HELD`), those past the top masked to zeros
        let mut held = [0; HELD];
        held.copy_from_slice(&stack[1 + from..1 + from + HELD]);
        for (h, m) in held.iter_mut().zip(&MASKS[kept]) {
            *h &= m;
        }
        let mut state = State {
            depth: kept as u8,
            inexact: !exact || from > 0,
            arg,
            ..State::default()
        };
        state.levels.copy_from_slice(&held[..HALF]);
        state.withs.copy_from_slice(&held[HALF..]);
        state
    }
}

/// For `m` levels held, the mask of the bytes that are theirs: a save clears the rest.
static MASKS: [[u8; HELD]; HELD + 1] = {
    let mut t = [[0; HELD]; HELD + 1];
    let mut m = 0;
    while m <= HELD {
        let mut k = 0;
        while k < m {
            t[m][k] = 0xff;
            k += 1;
        }
        m += 1;
    }
    t
};

/// Room for a level in a full stack: the lowest above the base (`stack[0]`) dropped, those
/// above it a level down, and the lines after no longer syntect's.
#[inline(always)]
pub fn drop_bottom(stack: &mut [u8], depth: &mut usize, exact: &mut bool) {
    stack.copy_within(2.., 1);
    *depth -= 1;
    *exact = false;
}

/// A line's end as a state's save and the next line's load leave its stack, for a scan kept
/// across a block's lines (X-067): past the levels a state holds, the bottom ones dropped, and
/// the lines after no longer syntect's.
#[inline(always)]
pub fn hold(stack: &mut [u8], depth: &mut usize, exact: &mut bool) {
    if *depth - 1 > HELD {
        hold_top(stack, depth, exact);
    }
}

#[cold]
#[inline(never)]
fn hold_top(stack: &mut [u8], depth: &mut usize, exact: &mut bool) {
    let from = *depth - 1 - HELD;
    stack.copy_within(1 + from..*depth, 1);
    *depth -= from;
    *exact = false;
}
