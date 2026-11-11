//! CSS in another language's template (X-126): styled-components', emotion's and Lit's
//! ``css`…` ``, ``styled.div`…` ``, ``createGlobalStyle`…` ``, ``keyframes`…` `` in JavaScript
//! and TypeScript. The host finds the template's text and its stops (the closing `` ` ``, a
//! `${`, the line's end) and hands each stretch between them to [`hosted`], which lexes it on
//! the host's sink as CSS lexes a line (a stretch's end a line's end, but that a string open at
//! a `${` goes on after it), each kind with [`HOST_KIND`] set. The template's CSS state rides in
//! the host's state as a [`Record`] of [`RECORD`] bytes, which the host keeps beside its frame
//! for the template.

use super::*;

/// An embedded CSS token's kind in its host: CSS's kind with this bit set.
pub(crate) const HOST_KIND: u16 = 0x0800;
const _: () = assert!(KF_VENDOR < HOST_KIND);

/// How many bytes a template's CSS state takes in its host's state.
pub(crate) const RECORD: usize = 6;
/// A template's CSS state ([`start`], [`hosted`]): the innermost six frames, the depth (up to
/// six), the item's mode and value flags, the at-rule, what the stretch ended inside.
pub(crate) type Record = [u8; RECORD];

/// How many frames a record keeps.
const KEPT: u8 = 6;

/// A template's CSS state at its start: a block of declarations and nested rules (a styled
/// component's, ``css`…` ``, a global style's), or a keyframes block (``keyframes`…` ``).
pub(crate) fn start(keyframes: bool) -> Record {
    let f = if keyframes { F_KEYFRAMES } else { F_DECLS };
    [f, 0, 0, 1, 0, O_NONE]
}

/// The CSS of `b[from..]` (the template's text, `b` cut at its next stop), from `rec` and left
/// in it, on the host's sink `k` (copied in, and back as the result). `cut`: `b` ends at a `${`
/// or the template's end, not the line's, so a string open there goes on after it.
pub(crate) fn hosted<K: Sink>(
    b: &[u8],
    from: usize,
    out: &mut Out,
    k: K,
    rec: &mut Record,
    cut: bool,
) -> K {
    let mut s = Scan::new(b, out, k);
    s.host = HOST_KIND;
    s.frames = u128::from(rec[0]) | u128::from(rec[1]) << 8 | u128::from(rec[2]) << 16;
    s.depth = rec[3] & 7;
    s.mode = rec[3] >> 3 & 3;
    s.vflags = rec[3] >> 5 & 3;
    s.at = rec[4];
    s.open = rec[5] & 3;
    s.cut = cut;
    s.lex_line::<true>(from);
    let f = s.frames.to_le_bytes();
    *rec = [
        f[0],
        f[1],
        f[2],
        s.depth.min(KEPT) | s.mode << 3 | s.vflags << 5,
        s.at,
        s.open,
    ];
    s.k
}
