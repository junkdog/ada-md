//! HEEx, a `~H` sigil's text (ada's; bat's is a string): HTML on HTML's scanner, its `{…}` and
//! `<%= … %>` islands Elixir on this one. HTML's scanner lexes a copy of the line with what isn't
//! the template's (before its text starts, after the sigil ends) and each island blanked, so its
//! contexts run across an island as across blanks, as PHP's do (X-099); a function component's
//! or a slot's name (`<.link`, `<:item`, `</.form`) is a tag's to it. A small reader of the
//! template's own ([`R_TEXT`]…) finds the islands: a `{` in text or in a tag is one, in a quoted
//! attribute value or a comment it's text.
//!
//! **State.** A line that ends in the template's text keeps HTML's state, with this scanner's
//! frames below the sigil (four levels), the sigil's kind and the reader's in the last withs
//! ([`Scan::state`]); a line that ends inside an island keeps this scanner's state, and HTML's
//! below the island (three levels, no embed) and the reader's in withs 9 to 12.

use super::*;
use crate::Span;
use crate::prim::find_in;

/// The reader's states: the template's text, a tag's inside, a quoted attribute value's, a
/// comment's.
pub(super) const R_TEXT: u8 = 0;
pub(super) const R_TAG: u8 = 1;
const R_DQ: u8 = 2;
const R_SQ: u8 = 3;
const R_COMMENT: u8 = 4;
/// Inside an EEx comment, `<%#` to its `%>`, and HEEx's, `<%!--` to its `--%>`, across lines.
const R_EEXC: u8 = 5;
const R_EEXC_DASH: u8 = 6;
/// A `<style>`'s or a `<script>`'s text, to its closing tag.
const R_RAW: u8 = 7;
/// On [`R_TAG`], [`R_DQ`], [`R_SQ`]: the tag is a `<style>`'s or a `<script>`'s.
const RAW: u8 = 8;

/// What a template keeps while a line has one.
pub(super) struct Heex {
    /// HTML's state where the template's text stands (at the line's end once [`Heex::line`]).
    pub(super) html: State,
    /// The reader's state where the template's text stands.
    pub(super) r: u8,
    /// HTML's spans of the line's pass.
    spans: Vec<Span>,
    /// The line's copy HTML's scanner lexes, kept for the next line's.
    buf: Vec<u8>,
    /// The line's islands the pass found, each where it starts and the reader's state there;
    /// the reader's state where the pass ended.
    marks: Vec<(usize, u8)>,
    r_end: u8,
    /// The next of [`Heex::spans`] to write.
    next: usize,
    /// Whether the pass ran for this line.
    pub(super) line: bool,
}

impl Heex {
    pub(super) fn new() -> Self {
        Heex {
            html: State::default(),
            r: R_TEXT,
            spans: Vec::new(),
            buf: Vec::new(),
            marks: Vec::new(),
            r_end: R_TEXT,
            next: 0,
            line: false,
        }
    }
}

/// Where the island whose `{` is at `q - 1` ends on the line (past its `}`: braces balanced,
/// Elixir's strings and characters inside skipped), or `end`.
fn island_end(b: &[u8], mut q: usize, end: usize) -> usize {
    let mut depth = 1u32;
    while q < end {
        match b[q] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return q + 1;
                }
            }
            b'?' => q += 1,
            c @ (b'"' | b'\'') => {
                q += 1;
                while q < end && b[q] != c {
                    q += if b[q] == b'\\' { 2 } else { 1 };
                }
            }
            _ => {}
        }
        q += 1;
    }
    end
}

/// Where the EEx tag whose text starts at `q` ends on the line (past its `%>`), or `end`.
fn eex_end(b: &[u8], q: usize, end: usize) -> usize {
    find_end(b, q, end, b"%>").unwrap_or(end)
}

/// Where `mark` first ends in `b[q..end]`, if it's there.
fn find_end(b: &[u8], q: usize, end: usize, mark: &[u8]) -> Option<usize> {
    b.get(q..end)?
        .windows(mark.len())
        .position(|w| w == mark)
        .map(|i| q + i + mark.len())
}

/// Where the EEx tag at `s` (its `<%`) ends on the line: a HEEx comment's past its `--%>`,
/// another's past its `%>`; `end` where it doesn't.
fn tag_end(b: &[u8], s: usize, end: usize) -> usize {
    if b[s..].starts_with(b"<%!--") {
        return find_end(b, s + 5, end, b"--%>").unwrap_or(end);
    }
    eex_end(b, s + 2, end)
}

/// The template's reader from `q` in state `r` to the next island's start (`{` or `<%`), or
/// `end`; the state there. A `<style>`'s or a `<script>`'s text has no islands (HEEx's): its tag
/// carries [`RAW`], and its text is [`R_RAW`] to its closing tag. Each state looks only for the
/// bytes that can end it, sixteen a step.
fn reader(b: &[u8], mut q: usize, end: usize, mut r: u8) -> (usize, u8) {
    while q < end {
        let raw = r & RAW;
        match r & !RAW {
            R_TEXT => {
                q = find_in(b, q, end, *b"{<");
                if q >= end {
                    break;
                }
                if b[q] == b'{' || b.get(q + 1) == Some(&b'%') {
                    return (q, r);
                }
                if b[q..].starts_with(b"<!--") {
                    r = R_COMMENT;
                    q += 4;
                    continue;
                }
                if b.get(q + 1)
                    .is_some_and(|&d| d.is_ascii_alphabetic() || matches!(d, b'/' | b'.' | b':'))
                {
                    r = if raw_tag(b, q + 1) {
                        R_TAG | RAW
                    } else {
                        R_TAG
                    };
                }
            }
            R_TAG => {
                q = find_in(b, q, end, *b"{<\"'>");
                if q >= end {
                    break;
                }
                match b[q] {
                    b'{' => return (q, r),
                    b'<' if b.get(q + 1) == Some(&b'%') => return (q, r),
                    b'"' => r = R_DQ | raw,
                    b'\'' => r = R_SQ | raw,
                    b'>' => {
                        r = if raw != 0 && (q == 0 || b[q - 1] != b'/') {
                            R_RAW
                        } else {
                            R_TEXT
                        };
                    }
                    _ => {}
                }
            }
            R_DQ | R_SQ => {
                let c = if r & !RAW == R_DQ { b'"' } else { b'\'' };
                q = find_in(b, q, end, [c]);
                if q >= end {
                    break;
                }
                r = R_TAG | raw;
            }
            R_COMMENT => {
                q = find_in(b, q, end, *b"-");
                if q >= end {
                    break;
                }
                if b[q..].starts_with(b"-->") {
                    r = R_TEXT;
                    q += 3;
                    continue;
                }
            }
            R_RAW => {
                q = find_in(b, q, end, *b"<");
                if q >= end {
                    break;
                }
                if b.get(q + 1) == Some(&b'/') && raw_tag(b, q + 2) {
                    r = R_TAG;
                }
            }
            _ => return (end, r),
        }
        q += 1;
    }
    (end, r)
}

/// Whether a tag's name at `q` is `style` or `script` (ASCII case aside), a raw text's.
fn raw_tag(b: &[u8], q: usize) -> bool {
    let w = |name: &[u8]| {
        b.len() >= q + name.len()
            && b[q..q + name.len()].eq_ignore_ascii_case(name)
            && !b
                .get(q + name.len())
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'-')
    };
    w(b"style") || w(b"script")
}

/// Where a single-line `~H` sigil's text from `p` ends: its closing delimiter (a `\` taking the
/// character after it), or the line's end.
fn sigil_end(b: &[u8], mut p: usize, closer: u8) -> usize {
    while p < b.len() {
        if b[p] == b'\\' {
            p += 2;
            continue;
        }
        if b[p] == closer {
            return p;
        }
        p += 1;
    }
    b.len()
}

thread_local! {
    /// A template's box a line API's scan left, for the next line's: a box and its vectors
    /// made once, not a line.
    static SPARE: std::cell::Cell<Option<Box<Heex>>> = const { std::cell::Cell::new(None) };
}

impl<K: Sink> Scan<'_, '_, K> {
    /// The template's box, made where a line first needs it (or the spare one).
    #[inline(always)]
    pub(super) fn heex(&mut self) -> &mut Heex {
        self.heex.get_or_insert_with(|| {
            SPARE
                .with(std::cell::Cell::take)
                .unwrap_or_else(|| Box::new(Heex::new()))
        })
    }

    /// The template's box kept for the next line's scan.
    #[cold]
    #[inline(never)]
    pub(super) fn spare_heex(&mut self) {
        if let Some(h) = self.heex.take() {
            SPARE.with(|s| s.set(Some(h)));
        }
    }

    /// A `~H` sigil opened: a template, HTML's and the reader's state fresh.
    #[cold]
    #[inline(never)]
    pub(super) fn heex_open(&mut self) {
        let h = self.heex();
        h.html = State::default();
        h.r = R_TEXT;
        h.line = false;
    }

    /// A template's text from `p` of kind `sk`: HTML's spans to the next island (its opener
    /// written, its frame pushed: code to its end), the sigil's end, or the line's end (the
    /// template goes on).
    #[cold]
    #[inline(never)]
    pub(super) fn heex_text(&mut self, p: usize, sk: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let heredoc = sk & HEREDOC != 0;
        let end = if heredoc {
            n
        } else {
            sigil_end(b, p, CLOSER[usize::from(sk & 7)])
        };
        let mut p = p;
        let r = self.heex().r;
        if matches!(r, R_EEXC | R_EEXC_DASH) {
            // an EEx comment from a line before
            let mark: &[u8] = if r == R_EEXC { b"%>" } else { b"--%>" };
            let Some(e) = find_end(b, p, end, mark) else {
                self.put(p, end, COMMENT);
                if end < n {
                    return self.str_close(end, sk);
                }
                self.mode = M_STRING;
                self.suspend(sk);
                return n;
            };
            self.put(p, e, COMMENT);
            self.heex().r = R_TEXT;
            p = e;
        }
        if !self.heex().line {
            self.html_pass(p, end);
        }
        // the pass's next island from `p`, and the reader's state there
        let h = self.heex();
        let (q, r) = match h.marks.iter().find(|m| m.0 >= p) {
            Some(&m) => m,
            None => (end, h.r_end),
        };
        h.r = r;
        self.write_html(p, q);
        if q < end {
            return self.island_open(q, sk);
        }
        if end < n {
            return self.str_close(end, sk);
        }
        self.mode = M_STRING;
        self.suspend(sk);
        n
    }

    /// An island at `q` (`{`, `<%=`, `<%`, `<%#`): its opener, and code to its end; an EEx
    /// comment to its `%>` here.
    fn island_open(&mut self, q: usize, sk: u8) -> usize {
        let b = self.b;
        if b[q] == b'{' {
            self.put(q, q + 1, EMBED_BEGIN);
            self.suspend(sk);
            self.push(F_ISLAND);
            return q + 1;
        }
        // `<%`: an EEx or a HEEx comment to its end, across lines
        let dash = b[q..].starts_with(b"<%!--");
        if dash || b.get(q + 2) == Some(&b'#') {
            let (from, mark): (usize, &[u8]) = if dash {
                (q + 5, b"--%>")
            } else {
                (q + 3, b"%>")
            };
            let Some(e) = find_end(b, from, b.len(), mark) else {
                self.put(q, b.len(), COMMENT);
                self.heex().r = if dash { R_EEXC_DASH } else { R_EEXC };
                self.mode = M_STRING;
                self.suspend(sk);
                return b.len();
            };
            self.put(q, e, COMMENT);
            return self.heex_text(e, sk);
        }
        let e = if b.get(q + 2) == Some(&b'=') {
            q + 3
        } else {
            q + 2
        };
        self.put(q, e, EMBED_BEGIN);
        self.suspend(sk);
        self.push(F_EEX);
        e
    }

    /// The `}` at `p` closing an island: the template goes on.
    #[cold]
    #[inline(never)]
    pub(super) fn island_close(&mut self, p: usize, len: usize) -> usize {
        self.pop();
        self.put(p, p + len, EMBED_END);
        let sk = self.resume();
        if sk & KIND != K_HEEX {
            // a template lost past the strings kept: code goes on
            return p + len;
        }
        self.heex_text(p + len, sk)
    }

    /// `%` at `p`: an EEx tag's `%>` closing its island, the frames left open in it dropped; a
    /// plain `%` elsewhere.
    #[inline(always)]
    pub(super) fn percent(&mut self, p: usize) -> usize {
        if self.b.get(p + 1) == Some(&b'>') && self.depth > 0 {
            return self.percent_close(p);
        }
        p + 1
    }

    #[cold]
    #[inline(never)]
    fn percent_close(&mut self, p: usize) -> usize {
        let levels = self.depth.min(LEVELS);
        let mut i = 0;
        while i < levels.min(6) {
            if self.frame(i) == F_EEX {
                for _ in 0..i {
                    self.pop();
                }
                return self.island_close(p, 2);
            }
            i += 1;
        }
        p + 1
    }

    /// The line from `p` to `end` through HTML's scanner, from [`Heex::html`], on a copy with
    /// what's before `p`, each island and what's from `end` blanked, and a component's or a
    /// slot's name a tag's. The spans into [`Heex::spans`], HTML's state to the line's end.
    fn html_pass(&mut self, p: usize, end: usize) {
        let b = self.b;
        let mut buf = std::mem::take(&mut self.heex().buf);
        buf.clear();
        buf.extend_from_slice(b);
        buf[..p].fill(b' ');
        buf[end..].fill(b' ');
        let mut marks = std::mem::take(&mut self.heex().marks);
        marks.clear();
        let mut r = self.heex().r;
        let mut q = p;
        while q < end {
            let (s, r2) = reader(b, q, end, r);
            // `<.link`, `</.link`, `<:item`: a name HTML's scanner takes for a tag's
            let mut i = find_in(b, q, s, *b"<");
            while i < s {
                let j = if b.get(i + 1) == Some(&b'/') {
                    i + 2
                } else {
                    i + 1
                };
                if matches!(b.get(j), Some(b'.' | b':')) {
                    buf[j] = b'x';
                }
                i = find_in(b, i + 1, s, *b"<");
            }
            r = r2;
            if s >= end {
                break;
            }
            marks.push((s, r));
            let e = if b[s] == b'{' {
                island_end(b, s + 1, end)
            } else {
                tag_end(b, s, end)
            };
            buf[s..e].fill(b' ');
            if r & !RAW == R_TAG && b[..s].trim_ascii_end().ends_with(b"=") {
                // an attribute's value: a quoted one of the island's width to HTML's scanner
                // (its last byte a character's own: an island left open to the line's end may
                // end inside one)
                if e - s >= 2 && b[e - 1] < 0x80 {
                    buf[s] = b'"';
                    buf[e - 1] = b'"';
                } else {
                    buf[s] = b'x';
                }
            }
            q = e;
        }
        let h = self.heex();
        h.marks = marks;
        h.r_end = r;
        h.spans.clear();
        crate::html::line(&buf, &mut h.html, &mut Out::new(&mut h.spans));
        h.buf = buf;
        h.line = true;
        h.next = 0;
    }

    /// The line's HTML spans in `p..q` written.
    fn write_html(&mut self, p: usize, q: usize) {
        let mut i = self.heex.as_ref().map_or(0, |h| h.next);
        loop {
            let sp = match self.heex.as_ref() {
                Some(h) if i < h.spans.len() => h.spans[i],
                _ => break,
            };
            let (s, e) = (sp.start as usize, sp.end as usize);
            if s >= q {
                break;
            }
            if e > p {
                self.put(s.max(p), e.min(q), sp.kind | HTML_KIND);
            }
            if e > q {
                break;
            }
            i += 1;
        }
        if let Some(h) = self.heex.as_mut() {
            h.next = i;
        }
    }

    /// The state of a line that ends in a template's text: HTML's, with this scanner's frames
    /// below the sigil (four levels), its depth and the reader's state in with 12, the sigil's
    /// kind in with 13 (never 0: the layout's mark).
    #[cold]
    #[inline(never)]
    pub(super) fn state_heex(&self, sk: u8) -> State {
        let h = self.heex.as_ref();
        let mut s = h.map_or_else(State::default, |h| h.html);
        if s.withs[9..14] != [0; 5] {
            // HTML's embed reached the withs kept here: its state lost
            s = State::default();
        }
        let d = self.depth.min(4);
        let f = (self.frames & 0xffff) as u16 & ((1u32 << (4 * u32::from(d))) - 1) as u16;
        s.withs[10..12].copy_from_slice(&f.to_le_bytes());
        s.withs[12] = d | h.map_or(R_TEXT, |h| h.r) << 3;
        s.withs[13] = sk;
        s
    }

    /// [`Scan::load`] of a line that starts in a template's text ([`Scan::state_heex`]).
    #[cold]
    #[inline(never)]
    pub(super) fn load_heex(&mut self, state: &State) {
        let sk = state.withs[13];
        let d = state.withs[12] & 7;
        self.frames = u128::from(u16::from_le_bytes([state.withs[10], state.withs[11]]));
        self.depth = d;
        self.strs = u64::from(sk);
        self.ctx = 0;
        self.mode = M_STRING;
        self.carried = 0;
        let mut html = *state;
        html.withs[9..14].fill(0);
        let h = self.heex();
        h.html = html;
        h.r = state.withs[12] >> 3;
        h.line = false;
    }

    /// HTML's state below an island a line ends in, kept in withs 9 to 12 where it can be (its
    /// stack's top three levels, no embed): with 9 its mark, depth and the reader's state.
    #[cold]
    #[inline(never)]
    pub(super) fn html_below(&self, s: &mut State) {
        let Some(h) = self.heex.as_ref() else {
            return;
        };
        let html = &h.html;
        let k = usize::from(html.depth);
        // the reader's state below an island: a tag's (a raw element's too) or the text's
        let mut tag = 0x80 | (h.r & 1) << 3 | (h.r & RAW) << 1;
        if k <= 3 && html.arg == 0 && html.withs.iter().all(|&w| w == 0) {
            s.withs[10..10 + k].copy_from_slice(&html.levels[..k]);
            tag |= k as u8;
        }
        s.withs[9] = tag;
    }

    /// [`Scan::load`]'s HTML below an island ([`Scan::html_below`]).
    #[cold]
    #[inline(never)]
    pub(super) fn load_below(&mut self, state: &State) {
        let tag = state.withs[9];
        let k = usize::from(tag & 7);
        let mut html = State::default();
        html.levels[..k].copy_from_slice(&state.withs[10..10 + k]);
        html.depth = k as u8;
        let h = self.heex();
        h.html = html;
        h.r = (tag >> 3) & 1 | (tag >> 1) & RAW;
        h.line = false;
    }
}
