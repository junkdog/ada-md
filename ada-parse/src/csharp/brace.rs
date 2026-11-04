//! A line of blanks, or of one `{` or `}` among blanks, without the scan (X-025's sixth stage):
//! a third of C#'s lines, where building [`Scan`](super::Scan) and its stack, and saving it,
//! cost more than the turns. Each case is the turn the scan would take: a blank line in a
//! context that passes over blanks takes nothing, and a brace's turn in the commonest contexts
//! is one of a few shapes ([`Brace`]), read from a table of the contexts' own patterns. Any
//! other line, a context not in the table, or a level whose bits (a string's chain, an
//! annotation's scope) would give the blanks or the brace a scope of their own, is the scan's.

use super::c::*;
use super::*;

/// What a context's turn at a lone brace is, where the scan's turn there is one of these.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Brace {
    /// None of these: the line is the scan's.
    No,
    /// The brace is the leaf, and the context is replaced (`then`).
    Set(u8, u16),
    /// The brace is the leaf, and the context pushed (`tok_push`).
    Push(u8, u16),
    /// The brace is the leaf, and the context pops (`close`).
    Close(u16),
    /// A pop that takes nothing (`leave`, `(?=\S)`'s or the brace's), then the turn of the
    /// context under it.
    Leave,
    /// A set that takes nothing (`shift`), then the new context's turn.
    Shift(u8),
}

/// Each context's turn at a lone `{`.
static OPEN: [Brace; 256] = {
    let mut t = [Brace::No; 256];
    t[METHOD_BODY as usize] = Brace::Set(MB_BLOCK, BLOCK_BEGIN);
    t[IF_BLOCK as usize] = Brace::Set(MAIN_BRACE_M, M_BLOCK_BEGIN);
    t[ELSE_BODY as usize] = Brace::Set(MAIN_BRACE_M, M_BLOCK_BEGIN);
    t[NS_DECL as usize] = Brace::Set(MAIN_BRACE, BLOCK_BEGIN);
    t[TRY_BLOCK as usize] = Brace::Set(MAIN_BRACE, BLOCK_BEGIN);
    t[FOR_AFTER as usize] = Brace::Set(CODE_BRACE, BLOCK_BEGIN);
    t[WHILE_BLOCK as usize] = Brace::Set(CODE_BRACE, BLOCK_BEGIN);
    t[TRYCATCH_BLOCK as usize] = Brace::Set(CODE_BRACE, BLOCK_BEGIN);
    t[UB_AFTER as usize] = Brace::Set(CODE_BRACE, BLOCK_BEGIN);
    t[EB_AFTER as usize] = Brace::Set(EB_BRACE, M_BLOCK_BEGIN);
    t[SWITCH_BLOCK as usize] = Brace::Set(SW_BRACE, BLOCK_BEGIN);
    t[METHOD_ACCESSOR as usize] = Brace::Set(ACC_BLOCK, M_BLOCK_BEGIN);
    t[DATA_TYPE_BODY as usize] = Brace::Push(DTB_BLOCK, BLOCK_BEGIN);
    t[DATA_TYPE_SIG as usize] = Brace::Leave;
    t[TYPE_CONSTRAINT as usize] = Brace::Leave;
    t[ELSE_BLOCK as usize] = Brace::Leave;
    t[FINALLY_BLOCK as usize] = Brace::Leave;
    t[CATCH_EXPR as usize] = Brace::Leave;
    t[METHOD_NAME as usize] = Brace::Shift(METHOD_ACCESSOR);
    t[CTOR_PREBODY as usize] = Brace::Shift(METHOD_BODY);
    t[CATCH_WHEN as usize] = Brace::Shift(TRYCATCH_BLOCK);
    t[CW_OPEN as usize] = Brace::Shift(TRYCATCH_BLOCK);
    t
};

/// Each context's turn at a lone `}`.
static CLOSE: [Brace; 256] = {
    let mut t = [Brace::No; 256];
    t[MB_BLOCK as usize] = Brace::Close(BLOCK_END);
    t[CODE_BRACE as usize] = Brace::Close(BLOCK_END);
    t[DTB_BLOCK as usize] = Brace::Close(BLOCK_END);
    t[SW_BRACE as usize] = Brace::Close(BLOCK_END);
    t[MAIN_BRACE as usize] = Brace::Close(BLOCK_END);
    t[MAIN_BRACE_M as usize] = Brace::Close(M_BLOCK_END);
    t[EB_BRACE as usize] = Brace::Close(M_BLOCK_END);
    t[ACC_BLOCK as usize] = Brace::Set(ACC_AFTER, BLOCK_END);
    t[ELSE_BLOCK as usize] = Brace::Leave;
    t[FINALLY_BLOCK as usize] = Brace::Leave;
    t[CATCH_EXPR as usize] = Brace::Leave;
    t[ACC_AFTER as usize] = Brace::Leave;
    t
};

/// The most turns a brace's line takes here: zero-width pops and sets, the brace's, and the
/// pops at the line's end.
const TURNS: usize = 8;

/// Where the line's leading ` ` and `\t` end: eight bytes at a time while they're all ` `.
#[inline(always)]
pub(super) fn lead(b: &[u8]) -> usize {
    let mut p = 0;
    while let Some(chunk) = b.get(p..p + 8) {
        let x = u64::from_le_bytes(chunk.try_into().unwrap()) ^ 0x2020_2020_2020_2020;
        if x != 0 {
            p += (x.trailing_zeros() / 8) as usize;
            break;
        }
        p += 8;
    }
    while p < b.len() && matches!(b[p], b' ' | b'\t') {
        p += 1;
    }
    p
}

/// `b`, a line whose leading blanks end at `i`, lexed here if it is a blank line or a lone
/// brace this module takes: its span pushed and `state` moved on as [`line`](super::line)
/// would. `false`, and nothing touched, when the line is the scan's.
#[inline(always)]
pub(super) fn line(b: &[u8], i: usize, state: &mut State, out: &mut Out) -> bool {
    let c = match b.get(i) {
        None => 0,
        Some(&c @ (b'{' | b'}')) => {
            if !b[i + 1..].iter().all(|&t| matches!(t, b' ' | b'\t')) {
                return false;
            }
            c
        }
        Some(_) => return false,
    };
    take(i, c, state, out)
}

/// Whether context `c` gives its level bits of its own (or by rule, [`level_bits`]): where no
/// level of the stack does, every level's bits are none, so its text is plain and a brace's
/// kind is its leaf.
static SETS: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 256 {
        let (keep, set) = LEVELS[c];
        t[c] = keep == BY_RULE || set != 0;
        c += 1;
    }
    t
};

/// [`line`]'s case `c` (`{`, `}`, or 0 for a blank line) at `i`.
#[inline(never)]
fn take(i: usize, c: u8, state: &mut State, out: &mut Out) -> bool {
    let mut d = usize::from(state.depth).min(KEPT);
    let mut st = [0; KEPT];
    st[..HALF].copy_from_slice(&state.levels);
    st[HALF..].copy_from_slice(&state.withs);
    let top = |st: &[u8; KEPT], d: usize| if d == 0 { MAIN } else { st[d - 1] };
    let t = top(&st, d);
    // the blanks before the brace (or the line's) are passed over, and its text is plain
    if (i > 0 || c == 0) && CTX[usize::from(t)] & INERT == 0 {
        return false;
    }
    if c == 0 && i == 0 {
        state.arg = 0;
        return true;
    }
    if st[..d].iter().any(|&x| SETS[usize::from(x)]) {
        return false;
    }
    if c == 0 {
        state.arg = 0;
        return true;
    }
    let table = if c == b'{' { &OPEN } else { &CLOSE };
    let mut kind = PLAIN;
    let mut turns = 0;
    // the brace's turn, after any that take nothing
    while kind == PLAIN {
        turns += 1;
        if turns > TURNS || d == 0 {
            return false;
        }
        match table[usize::from(st[d - 1])] {
            Brace::No => return false,
            Brace::Set(x, leaf) => {
                kind = leaf;
                st[d - 1] = x;
            }
            Brace::Push(x, leaf) => {
                if d + 1 >= KEPT {
                    return false;
                }
                kind = leaf;
                st[d] = x;
                d += 1;
            }
            Brace::Close(leaf) => {
                kind = leaf;
                d -= 1;
            }
            Brace::Leave => d -= 1,
            Brace::Shift(x) => st[d - 1] = x,
        }
        if SETS[usize::from(top(&st, d))] {
            return false;
        }
    }
    // the line's end: a context that passes over blanks takes nothing more, a type's body pops
    // there (`data_type_body`'s `leave`), anything else is the scan's
    while CTX[usize::from(top(&st, d))] & INERT == 0 {
        turns += 1;
        if top(&st, d) != DATA_TYPE_BODY || turns > TURNS {
            return false;
        }
        d -= 1;
    }
    let spans = &mut *out.0;
    let (s, e) = (i as u32, i as u32 + 1);
    match spans.last_mut() {
        Some(last) if last.end == s && last.kind == kind && out.1 => last.end = e,
        _ => spans.push(Span {
            start: s,
            end: e,
            kind,
        }),
    }
    st[d..].fill(0);
    state.levels.copy_from_slice(&st[..HALF]);
    state.withs.copy_from_slice(&st[HALF..]);
    state.depth = d as u8;
    state.arg = 0;
    true
}
