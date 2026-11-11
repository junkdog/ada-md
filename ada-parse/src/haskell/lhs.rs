//! Literate Haskell (`.lhs`, X-136): prose around Haskell's code, which GHC's `unlit` takes from
//! Bird tracks (a line starting `>`), from `\begin{code}` … `\end{code}`, and markdown-unlit
//! from ```` ```haskell ```` fences. The code is lexed by Haskell's scan, its state carried
//! across the prose; the prose is plain (bat reads it as LaTeX, Bird tracks too; ada has no
//! LaTeX), `\begin{code}` and `\end{code}` take bat's LaTeX scopes.

use super::*;

// Where an `.lhs`'s line stands ([`Scan::lit`]).
/// Prose, or a Bird track's code, decided a line at a time.
pub(super) const L_PROSE: u8 = 0;
/// Code after `\begin{code}`, to `\end{code}`.
pub(super) const L_LATEX: u8 = 1;
/// Code in a ```` ```haskell ```` fence, to the fence's close.
pub(super) const L_FENCE: u8 = 2;

/// Lex `b`, a line of Literate Haskell without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    line_on::<_, false>(b, state, out, Spans::new());
}

/// [`line`]'s runs by theme rule pushed to `runs`.
#[inline(never)]
pub fn line_runs(
    b: &[u8],
    state: &mut State,
    out: &mut Out,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
) {
    let mut k = line_on::<_, true>(b, state, out, Runs::new(rules, runs));
    k.end_line(b.len());
}

#[inline(always)]
fn line_on<K: Sink, const SHARED: bool>(b: &[u8], state: &mut State, out: &mut Out, k: K) -> K {
    let mut s = Scan::new(b, out, k);
    s.load(state);
    s.literate = true;
    s.k.open(s.out, b.len() / 4 + 16);
    s.literate_line::<SHARED>();
    s.k.close(s.out);
    *state = s.state();
    s.k
}

/// A block of Literate Haskell on one scan opened from `state` ([`crate::Carry`]).
pub fn block(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    let mut out = Out::new(spans);
    let mut s = Scan::new(b"", &mut out, Spans::new());
    s.load(state);
    s.literate = true;
    s.out.0.clear();
    s.k.open(s.out, 64);
    let rest = pass(&mut s);
    *state = s.state();
    rest
}

/// [`block`]'s runs by theme rule pushed to `runs` ([`crate::CarryRuns`]).
pub fn block_runs(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    lines: &mut crate::RunLines,
) -> Option<usize> {
    let mut out = Out::new(spans);
    let mut s = Scan::new(b"", &mut out, Runs::new(rules, runs));
    s.load(state);
    s.literate = true;
    s.out.0.clear();
    s.k.open(s.out, 64);
    let rest = lines.pass(&mut s);
    *state = s.state();
    rest
}

/// `\begin{code}` or `\end{code}` (`word`) at the line's start after blanks: where its `\` is.
fn latex_at(b: &[u8], word: &[u8]) -> Option<usize> {
    let p = blanks(b, 0);
    let rest = &b[p..];
    (rest.first() == Some(&b'\\')
        && rest[1..].starts_with(word)
        && rest[1 + word.len()..].starts_with(b"{code}"))
    .then_some(p)
}

/// A fence's line: its backticks or tildes (three or more) after up to three blanks, and what
/// follows them.
fn fence(b: &[u8]) -> Option<&[u8]> {
    let p = blanks(b, 0);
    if p > 3 {
        return None;
    }
    let c = *b.get(p)?;
    if c != b'`' && c != b'~' {
        return None;
    }
    let mut e = p;
    while e < b.len() && b[e] == c {
        e += 1;
    }
    (e - p >= 3).then(|| &b[e..])
}

/// Whether a fence's info string names Haskell: `haskell`, `{.haskell}`, `hs`.
fn haskell_info(info: &[u8]) -> bool {
    let t = info.trim_ascii();
    let t = t.strip_prefix(b"{").unwrap_or(t).trim_ascii_start();
    let t = t.strip_prefix(b".").unwrap_or(t);
    let word = t
        .split(|c| !c.is_ascii_alphanumeric())
        .next()
        .unwrap_or(b"");
    word.eq_ignore_ascii_case(b"haskell") || word.eq_ignore_ascii_case(b"hs")
}

impl<K: Sink> Scan<'_, '_, K> {
    /// A line of Literate Haskell: code by where it stands ([`Scan::lit`]) or its Bird track;
    /// prose plain.
    #[inline(always)]
    pub(super) fn literate_line<const SHARED: bool>(&mut self) {
        let b = self.b;
        match self.lit {
            L_LATEX => {
                if let Some(p) = latex_at(b, b"end") {
                    self.latex(p, 3);
                    self.lit = L_PROSE;
                } else {
                    self.lex_line::<SHARED>(0);
                }
            }
            L_FENCE => {
                if fence(b).is_some_and(|rest| rest.trim_ascii().is_empty()) {
                    self.lit = L_PROSE;
                } else {
                    self.lex_line::<SHARED>(0);
                }
            }
            _ => {
                if b.first() == Some(&b'>') {
                    self.put(0, 1, BIRD);
                    self.lex_line::<SHARED>(1);
                } else if let Some(p) = latex_at(b, b"begin") {
                    self.latex(p, 5);
                    self.lit = L_LATEX;
                } else if fence(b).is_some_and(haskell_info) {
                    self.lit = L_FENCE;
                }
            }
        }
    }

    /// `\begin{code}` or `\end{code}` at `p`, its command `k` bytes after the `\`: bat's LaTeX
    /// scopes.
    fn latex(&mut self, p: usize, k: usize) {
        let e = p + 1 + k;
        self.put(p, p + 1, LATEX_BACKSLASH);
        self.put(p + 1, e, LATEX_BE);
        self.put(e, e + 1, LATEX_BRACE_BEGIN);
        self.put(e + 1, e + 5, LATEX_ENV);
        self.put(e + 5, e + 6, LATEX_BRACE_END);
    }
}
