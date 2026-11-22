//! Markdown as source (X-189), in place of bat's `Markdown`: a `.md` file in a diff, a
//! ```` ```markdown ```` fence, `-l md` where a file isn't rendered. CommonMark's blocks and
//! GitHub's (tables, strikethrough, bare links), read a line at a time from what the lines before
//! left: headings (ATX, and a setext underline after a paragraph), thematic breaks, lists'
//! markers, block quotes' `>`, fenced and indented code, HTML blocks, link reference definitions,
//! table rows, and YAML's or TOML's front matter on a file's first line. In a line's text: code
//! spans, emphasis, strikethrough, links and images (inline and by reference), autolinks and bare
//! links, entities, escapes, HTML's tags and comments, a hard break's trailing blanks.
//!
//! **Embeds.** A fence's code is its info string's language (`Lang::from_token`: ```` ```rust ````,
//! ```` ```{r} ````), and front matter YAML's or TOML's: on the run sink each line is lexed by
//! that language's own scanner on its own rules ([`Sink::guest`], [`crate::Rules::guest`]), its
//! state carried through the fence; on the span sink, or where the rules keep no guest's, the
//! code is one raw kind (its state carried all the same, so both sinks leave the same state).
//! HTML's blocks and tags are HTML's scanner's, its kinds under [`HTML`] on either sink.
//!
//! **State.** Markdown's own fields are three bytes of [`State`] ([`B0`], [`B1`], [`B2`]: bytes
//! no language's lines leave set but Nix's and TSX's, under 1% of their lines, `x189/foot.txt`);
//! the rest is a fence's language's state, or zero. A language whose line leaves one of the three
//! set loses its state there: its lines are lexed each from a fresh state to the fence's end
//! ([`LOST`]). Markdown in a markdown fence keeps its three in `levels`' first three, and its own
//! fences' code is lexed from a fresh state each line.
//!
//! **Departures from bat**, decided from CommonMark and GitHub: fences indented, in a list item or
//! in a quote; `1)` lists; front matter; a table's pipes only in a row that starts with one or
//! after a delimiter row (bat colors every `|`); seven `#`s no heading; a markdown fence's code
//! markdown; bat's list item that runs on past its paragraph not followed.

use std::mem::MaybeUninit;

use crate::emit::{Push, Runs, Sink};
use crate::{Lang, Out, State};

// ---- the kinds: a leaf in the low six bits, the context it sits in above ----

const TEXT: u16 = 0;
const H_BEGIN: u16 = 1;
const H_END: u16 = 2;
const SETEXT_1: u16 = 3;
const SETEXT_2: u16 = 4;
const BREAK: u16 = 5;
const BULLET: u16 = 6;
const NUMBER: u16 = 7;
const NUMBER_P: u16 = 8;
const QUOTE: u16 = 9;
const FENCE_BEGIN: u16 = 10;
const FENCE_END: u16 = 11;
const LANG_NAME: u16 = 12;
const FENCE_BODY: u16 = 13;
const RAW_BLOCK: u16 = 14;
const RAW: u16 = 15;
const RAW_BEGIN: u16 = 16;
const RAW_END: u16 = 17;
const BOLD_BEGIN: u16 = 18;
const BOLD_END: u16 = 19;
const ITALIC_BEGIN: u16 = 20;
const ITALIC_END: u16 = 21;
const STRIKE_BEGIN: u16 = 22;
const STRIKE_END: u16 = 23;
const LINK_BEGIN: u16 = 24;
const LINK_END: u16 = 25;
const IMAGE_BEGIN: u16 = 26;
const IMAGE_END: u16 = 27;
const META_BEGIN: u16 = 28;
const META_END: u16 = 29;
const URL: u16 = 30;
const URL_IMAGE: u16 = 31;
const TITLE: u16 = 32;
const TITLE_BEGIN: u16 = 33;
const TITLE_END: u16 = 34;
const REF_BEGIN: u16 = 35;
const REF_END: u16 = 36;
const REF: u16 = 37;
const REF_DEF: u16 = 38;
const KEY_VALUE: u16 = 39;
const ESCAPE: u16 = 40;
/// An entity's three parts from here: named, decimal, hexadecimal ([`Scan::entity`]).
const ENTITY: u16 = 41;
const TABLE_SEP: u16 = 50;
const TABLE_HEADER: u16 = 51;
const TABLE_ALIGN: u16 = 52;
const HARD_BREAK: u16 = 53;
const COMMENT: u16 = 54;
const COMMENT_BEGIN: u16 = 55;
const COMMENT_END: u16 = 56;
const FRONT_BEGIN: u16 = 57;
const FRONT_END: u16 = 58;

/// Each leaf's scopes, bat's where its grammar has the token (front matter's are Sublime's
/// newer grammar's).
static LEAVES: [&str; 59] = [
    "",
    "punctuation.definition.heading.begin.markdown",
    "punctuation.definition.heading.end.markdown",
    "markup.heading.1.setext.markdown punctuation.definition.heading.setext.markdown",
    "markup.heading.2.setext.markdown punctuation.definition.heading.setext.markdown",
    "meta.separator.thematic-break.markdown punctuation.definition.thematic-break.markdown",
    "markup.list.unnumbered.bullet.markdown punctuation.definition.list_item.markdown",
    "markup.list.numbered.bullet.markdown",
    "markup.list.numbered.bullet.markdown punctuation.definition.list_item.markdown",
    "markup.quote.markdown punctuation.definition.blockquote.markdown",
    "punctuation.definition.raw.code-fence.begin.markdown",
    "punctuation.definition.raw.code-fence.end.markdown",
    "constant.other.language-name.markdown",
    "markup.raw.code-fence.markdown-gfm",
    "markup.raw.block.markdown",
    "markup.raw.inline.markdown",
    "markup.raw.inline.markdown punctuation.definition.raw.begin.markdown",
    "markup.raw.inline.markdown punctuation.definition.raw.end.markdown",
    "punctuation.definition.bold.begin.markdown",
    "punctuation.definition.bold.end.markdown",
    "punctuation.definition.italic.begin.markdown",
    "punctuation.definition.italic.end.markdown",
    "punctuation.definition.strikethrough.begin.markdown",
    "punctuation.definition.strikethrough.end.markdown",
    "punctuation.definition.link.begin.markdown",
    "punctuation.definition.link.end.markdown",
    "punctuation.definition.image.begin.markdown",
    "punctuation.definition.image.end.markdown",
    "punctuation.definition.metadata.begin.markdown",
    "punctuation.definition.metadata.end.markdown",
    "markup.underline.link.markdown",
    "markup.underline.link.image.markdown",
    "string.other.link.description.title.markdown",
    "string.other.link.description.title.markdown punctuation.definition.string.begin.markdown",
    "string.other.link.description.title.markdown punctuation.definition.string.end.markdown",
    "punctuation.definition.constant.begin.markdown",
    "punctuation.definition.constant.end.markdown",
    "constant.other.reference.link.markdown",
    "entity.name.reference.link.markdown",
    "punctuation.separator.key-value.markdown",
    "constant.character.escape.markdown",
    "constant.character.entity.named.html punctuation.definition.entity.html",
    "constant.character.entity.named.html",
    "constant.character.entity.named.html punctuation.terminator.entity.html",
    "constant.character.entity.decimal.html punctuation.definition.entity.html",
    "constant.character.entity.decimal.html",
    "constant.character.entity.decimal.html punctuation.terminator.entity.html",
    "constant.character.entity.hexadecimal.html punctuation.definition.entity.html",
    "constant.character.entity.hexadecimal.html",
    "constant.character.entity.hexadecimal.html punctuation.terminator.entity.html",
    "punctuation.separator.table-cell.markdown",
    "punctuation.section.table-header.markdown",
    "punctuation.definition.table-cell-alignment.markdown",
    "punctuation.definition.hard-line-break.markdown",
    "comment.block.html",
    "comment.block.html punctuation.definition.comment.begin.html",
    "comment.block.html punctuation.definition.comment.end.html",
    "punctuation.section.block.begin.frontmatter.markdown",
    "punctuation.section.block.end.frontmatter.markdown",
];

const LEAF: u16 = 0x3f;
/// A heading's level: 1, 2, or a lower one (bat's `markup.heading` for 3 to 6).
const H1: u16 = 1 << 6;
const H2: u16 = 2 << 6;
const HN: u16 = 3 << 6;
const H_MASK: u16 = 3 << 6;
/// A heading's text.
const SECTION: u16 = 1 << 8;
const BOLD: u16 = 1 << 9;
const ITALIC: u16 = 1 << 10;
const STRIKE: u16 = 1 << 11;
/// Italic opened before bold, so outside it (bold's outside otherwise, as `***` opens them).
const I_FIRST: u16 = 1 << 12;
/// HTML's kinds ([`crate::html::scopes`], all below `0x3000`) with this bit set, under
/// `text.html.basic`.
const HTML: u16 = 0x4000;

/// The scopes under `text.html.markdown` a kind carries, outermost first: a heading, its text,
/// bold, italic and strikethrough around the leaf, as bat nests them.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    if kind & HTML != 0 {
        let mut v = vec!["text.html.basic"];
        v.extend(crate::html::scopes(kind & !HTML));
        return v;
    }
    let mut v = Vec::new();
    match kind & H_MASK {
        H1 => v.push("markup.heading.1.markdown"),
        H2 => v.push("markup.heading.2.markdown"),
        HN => v.push("markup.heading.markdown"),
        _ => {}
    }
    if kind & SECTION != 0 {
        v.push("entity.name.section.markdown");
    }
    let (bold, italic) = (kind & BOLD != 0, kind & ITALIC != 0);
    if italic && kind & I_FIRST != 0 {
        v.push("markup.italic.markdown");
    }
    if bold {
        v.push("markup.bold.markdown");
    }
    if italic && kind & I_FIRST == 0 {
        v.push("markup.italic.markdown");
    }
    if kind & STRIKE != 0 {
        v.push("markup.strikethrough.markdown-gfm");
    }
    if let Some(leaf) = LEAVES.get(usize::from(kind & LEAF)) {
        v.extend(leaf.split(' ').filter(|s| !s.is_empty()));
    }
    v
}

// ---- the state: markdown's three bytes ----

/// [`State::byte`]'s places of markdown's own bytes: `withs` 6, 8 and 9.
const B0: usize = 20;
const B1: usize = 22;
const B2: usize = 23;

// B0: the mode, the quotes a fence or an HTML block opened in, and flags.
const MODE: u8 = 0x7;
/// Text: paragraphs, headings, lists, quotes, tables.
const M_TEXT: u8 = 0;
/// A fence's code: [`B1`] its closer, [`B2`] its language's slot.
const M_FENCE: u8 = 1;
/// An HTML block to a blank line.
const M_HTML: u8 = 2;
/// An HTML block to its end mark ([`B1`]: [`E_SCRIPT`] …).
const M_HTML_END: u8 = 3;
/// An HTML comment open across lines, in text ([`B1`], [`B2`] text's).
const M_COMMENT: u8 = 4;
/// Front matter: [`B2`] its language's slot, YAML's or TOML's.
const M_FRONT: u8 = 5;
const Q_SHIFT: u32 = 3;
const QUOTES: u8 = 0x3 << Q_SHIFT;
/// The fence's language's state is lost: each line lexed from a fresh one.
const LOST: u8 = 0x20;
/// The fence is `~`'s.
const TILDE: u8 = 0x40;
/// A line has been read: front matter only on the first.
const STARTED: u8 = 0x80;

// B1 in text.
/// The line before was a paragraph's: a setext underline may follow, indented code may not.
const PARA: u8 = 0x1;
const OPEN_BOLD: u8 = 0x2;
const OPEN_ITALIC: u8 = 0x4;
const OPEN_STRIKE: u8 = 0x8;
/// The open bold's and italic's delimiter is `_`.
const BOLD_US: u8 = 0x10;
const ITALIC_US: u8 = 0x20;
/// A code span open across lines: its backticks, 1 to 3 (0 for none).
const T_SHIFT: u32 = 6;
const TICKS: u8 = 0x3 << T_SHIFT;

// B1 in a fence: the opener's length past three (saturating at 10), its indent (to 15), and
// whether it stands in a list item (a line indented less ends it).
const F_LEN: u8 = 0x7;
const F_IN_LIST: u8 = 0x8;
const F_I_SHIFT: u32 = 4;

// B1 in an HTML block to its end mark: which mark.
const E_SCRIPT: u8 = 1;
const E_PRE: u8 = 2;
const E_STYLE: u8 = 3;
const E_TEXTAREA: u8 = 4;
const E_PI: u8 = 5;
const E_DECL: u8 = 6;
const E_CDATA: u8 = 7;

// B2 in text: the list item's content column (to 31), whether the line before was a table's,
// and whether italic open across lines is outside bold ([`I_FIRST`]).
const LIST: u8 = 0x1f;
const TABLE: u8 = 0x20;
const OPEN_I_FIRST: u8 = 0x40;

/// The floor probe of the scanner's shape (X-189, the guide's sol probe), built with
/// `ADA_PARSE_X189_SOL` set: a line's indent, its block by its first byte (headings, lists'
/// markers, quotes, fences, their code raw), and the inline loop with code spans and emphasis;
/// no links, HTML, entities, bare links, tables, front matter or fence languages.
const SOL: bool = option_env!("ADA_PARSE_X189_SOL").is_some();

/// A throwaway switch for parity's triage (the guide's mistake 21), `ADA_PARSE_X189_RAW`: every
/// fence's and front matter's code raw, so a report measures markdown's own readings apart from
/// the languages it embeds (bat's few and ada's many).
const RAW_FENCES: bool = option_env!("ADA_PARSE_X189_RAW").is_some();

/// A language's slot in [`B2`]: its place in [`Lang::ALL`] plus one, 0 for none.
fn slot(lang: Option<Lang>) -> u8 {
    lang.and_then(|l| Lang::ALL.iter().position(|&a| a == l))
        .map_or(0, |i| i as u8 + 1)
}

fn lang_of(slot: u8) -> Option<Lang> {
    Lang::ALL.get(usize::from(slot).wrapping_sub(1)).copied()
}

/// One line, after the lines before it left `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    scan(b, state, out, Push);
}

/// [`line`]'s runs by theme rule written to `k`: inline in a block's loop, as a man page's are,
/// [`crate::lex_runs`]' line too.
pub(crate) struct RunLine;

impl crate::LineScan for RunLine {
    #[inline(always)]
    fn scan(b: &[u8], state: &mut State, out: &mut Out, k: &mut Runs) {
        *k = scan(b, state, out, *k);
    }
}

#[inline(always)]
fn scan<K: Sink>(b: &[u8], state: &mut State, out: &mut Out, k: K) -> K {
    let mut s = Scan {
        b,
        out,
        k,
        st: *state,
        b0: state.byte(B0),
        b1: state.byte(B1),
        b2: state.byte(B2),
        ctx: 0,
        w: 0,
        brackets: Brackets::NONE,
        delims: Delims::NONE,
        memo: Memo::default(),
    };
    s.k.open(s.out, 8);
    s.line();
    s.k.close(s.out);
    s.st.set_byte(B0, s.b0);
    s.st.set_byte(B1, s.b1);
    s.st.set_byte(B2, s.b2);
    *state = s.st;
    s.k
}

/// A line's scan.
struct Scan<'a, 'o, K: Sink> {
    b: &'a [u8],
    out: &'a mut Out<'o>,
    k: K,
    /// The state: a fence's language's in all but markdown's bytes, else zero there.
    st: State,
    b0: u8,
    b1: u8,
    b2: u8,
    /// What the text written stands in: a heading, bold, italic, strikethrough.
    ctx: u16,
    /// Where the last write ended: the context fills the plain text from here.
    w: usize,
    /// The line's brackets paired, once a `[` is met.
    brackets: Brackets,
    /// The line's delimiter runs matched, once one is met.
    delims: Delims,
    memo: Memo,
}

/// Where searches along the line have failed, so that none runs again from a later place:
/// no `"`, `'` or `>` past these, no code span's closer of `n` backticks past `ticks[n - 1]`.
/// Each a place plus one, 0 for not looked ([`bound`], [`mark`]): zero to start, so a line's
/// setup clears it without a `memset` of `usize::MAX`es.
#[derive(Default)]
struct Memo {
    dq: u32,
    sq: u32,
    gt: u32,
    ticks: [u32; 8],
}

/// A [`Memo`] field's place, `usize::MAX` for not looked.
#[inline(always)]
fn bound(x: u32) -> usize {
    (x as usize).wrapping_sub(1)
}

#[inline(always)]
fn mark(p: usize) -> u32 {
    p as u32 + 1
}

/// A line's `[`s and the `]` each closes at (escaped ones left out), up to [`Brackets::MAX`]
/// pairs; past them a `[` has none.
struct Brackets {
    n: u8,
    done: bool,
    /// Written below `n` only: left unset, so that a line without a `[` doesn't clear them.
    pairs: [MaybeUninit<(u32, u32)>; Brackets::MAX],
}

impl Brackets {
    const MAX: usize = 32;
    const NONE: Brackets = Brackets {
        n: 0,
        done: false,
        pairs: [const { MaybeUninit::uninit() }; Brackets::MAX],
    };
}

/// A delimiter run of `*`, `_` or `~` ([`Scan::match_delims`]): where it starts (`u32::MAX` for
/// one the line before left open), its length left to match and as it was, its byte, its
/// flags, and what it closes and opens, by length, in the order matched (up to four each).
#[derive(Clone, Copy)]
struct Delim {
    at: u32,
    len: u8,
    orig: u8,
    c: u8,
    flags: u8,
    nc: u8,
    no: u8,
    close: [u8; 4],
    open: [u8; 4],
}

const D_OPEN: u8 = 1;
const D_CLOSE: u8 = 2;
const D_ACTIVE: u8 = 4;
const D_VIRTUAL: u8 = 8;
/// At a word's start before a letter or digit: it may open across lines.
const D_WORD: u8 = 16;

impl Delim {
    fn new(at: u32, len: u8, c: u8, flags: u8) -> Self {
        Delim {
            at,
            len,
            orig: len,
            c,
            flags: flags | D_ACTIVE,
            nc: 0,
            no: 0,
            close: [0; 4],
            open: [0; 4],
        }
    }
}

/// A line's delimiter runs, up to [`Delims::MAX`]; past them a run is text.
struct Delims {
    done: bool,
    n: u8,
    /// Written below `n` only.
    d: [MaybeUninit<Delim>; Delims::MAX],
}

impl Delims {
    const MAX: usize = 40;
    const NONE: Delims = Delims {
        done: false,
        n: 0,
        d: [const { MaybeUninit::uninit() }; Delims::MAX],
    };

    fn push(&mut self, d: Delim) {
        if usize::from(self.n) < Delims::MAX {
            self.d[usize::from(self.n)] = MaybeUninit::new(d);
            self.n += 1;
        }
    }

    fn all(&mut self) -> &mut [Delim] {
        let n = usize::from(self.n);
        // SAFETY: the runs below `n` are written, and `MaybeUninit<Delim>` is laid out as `Delim`
        unsafe { std::slice::from_raw_parts_mut(self.d.as_mut_ptr().cast::<Delim>(), n) }
    }

    fn find(&mut self, at: usize) -> Option<Delim> {
        self.all().iter().find(|d| d.at as usize == at).copied()
    }

    /// CommonMark's matching ([`Scan::match_delims`]).
    fn resolve(&mut self) {
        let d = self.all();
        for ci in 0..d.len() {
            if d[ci].flags & D_CLOSE == 0 || d[ci].flags & D_ACTIVE == 0 {
                continue;
            }
            while d[ci].len > 0 {
                let c = d[ci].c;
                let Some(oi) = (0..ci).rev().find(|&j| {
                    let o = &d[j];
                    if o.flags & (D_ACTIVE | D_OPEN) != D_ACTIVE | D_OPEN || o.len == 0 || o.c != c
                    {
                        return false;
                    }
                    if c == b'~' {
                        return o.len == d[ci].len;
                    }
                    let either = o.flags & D_CLOSE != 0 || d[ci].flags & D_OPEN != 0;
                    let (a, z) = (u32::from(o.orig), u32::from(d[ci].orig));
                    !(either && (a + z) % 3 == 0 && !(a % 3 == 0 && z % 3 == 0))
                }) else {
                    break;
                };
                let take = if c == b'~' {
                    d[ci].len
                } else if d[oi].len >= 2 && d[ci].len >= 2 {
                    2
                } else {
                    1
                };
                if d[ci].nc < 4 {
                    let k = usize::from(d[ci].nc);
                    d[ci].close[k] = take;
                    d[ci].nc += 1;
                }
                if d[oi].no < 4 {
                    let k = usize::from(d[oi].no);
                    d[oi].open[k] = take;
                    d[oi].no += 1;
                }
                d[oi].len -= take;
                d[ci].len -= take;
                for x in &mut d[oi + 1..ci] {
                    x.flags &= !D_ACTIVE;
                }
            }
            if d[ci].len > 0 && d[ci].flags & D_OPEN == 0 {
                d[ci].flags &= !D_ACTIVE;
            }
        }
    }

    /// Openers left over that open across lines: at a word's start, by what is left of them.
    fn carry(&mut self) {
        for o in self.all() {
            let want = D_ACTIVE | D_OPEN | D_WORD;
            if o.flags & want != want || o.len == 0 || o.flags & D_VIRTUAL != 0 || o.no >= 4 {
                continue;
            }
            let take = o.len.min(2);
            let k = usize::from(o.no);
            o.open[k] = take;
            o.no += 1;
            o.len -= take;
        }
    }
}

#[inline(always)]
fn is_ws(c: u8) -> bool {
    c == b' ' || c == b'\t'
}

/// Bytes `p..` of `b` that are blank: their end.
#[inline(always)]
fn blanks(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && is_ws(b[p]) {
        p += 1;
    }
    p
}

/// Whether `b` from `p` holds only blanks.
#[inline(always)]
fn rest_blank(b: &[u8], p: usize) -> bool {
    blanks(b, p) == b.len()
}

/// The indent at `p`, its column `col`: where it ends and the column there (a tab to the next
/// multiple of four).
#[inline(always)]
fn indent(b: &[u8], mut p: usize, mut col: usize) -> (usize, usize) {
    while p < b.len() {
        match b[p] {
            b' ' => col += 1,
            b'\t' => col += 4 - col % 4,
            _ => break,
        }
        p += 1;
    }
    (p, col)
}

/// What a byte asks of the inline loop: nothing, or a look.
static INLINE: [bool; 256] = {
    let mut t = [false; 256];
    let mut i = 0;
    let set = if SOL {
        b"\\`*_".as_slice()
    } else {
        b"\\`*_~[!<&:@.|".as_slice()
    };
    while i < set.len() {
        t[set[i] as usize] = true;
        i += 1;
    }
    t
};

/// Where the plain text from `p` ends: the next byte [`INLINE`] asks a look at, or the end. A
/// function of its own, four bytes a step: inline in the loop that dispatches, LLVM folded the
/// plain bytes into the dispatch's `match` (a table and a jump a byte).
#[inline(never)]
fn skip_plain(t: &[u8], mut p: usize) -> usize {
    let tab = &INLINE;
    while p + 4 <= t.len() {
        let w = &t[p..p + 4];
        if tab[usize::from(w[0])] {
            return p;
        }
        if tab[usize::from(w[1])] {
            return p + 1;
        }
        if tab[usize::from(w[2])] {
            return p + 2;
        }
        if tab[usize::from(w[3])] {
            return p + 3;
        }
        p += 4;
    }
    while p < t.len() && !tab[usize::from(t[p])] {
        p += 1;
    }
    p
}

/// HTML's block-level tag names (CommonMark's sixth kind of HTML block), lowercase.
static BLOCK_TAGS: &[&[u8]] = &[
    b"address",
    b"article",
    b"aside",
    b"base",
    b"basefont",
    b"blockquote",
    b"body",
    b"caption",
    b"center",
    b"col",
    b"colgroup",
    b"dd",
    b"details",
    b"dialog",
    b"dir",
    b"div",
    b"dl",
    b"dt",
    b"fieldset",
    b"figcaption",
    b"figure",
    b"footer",
    b"form",
    b"frame",
    b"frameset",
    b"h1",
    b"h2",
    b"h3",
    b"h4",
    b"h5",
    b"h6",
    b"head",
    b"header",
    b"hr",
    b"html",
    b"iframe",
    b"legend",
    b"li",
    b"link",
    b"main",
    b"menu",
    b"menuitem",
    b"nav",
    b"noframes",
    b"ol",
    b"optgroup",
    b"option",
    b"p",
    b"param",
    b"search",
    b"section",
    b"summary",
    b"table",
    b"tbody",
    b"td",
    b"tfoot",
    b"th",
    b"thead",
    b"title",
    b"tr",
    b"track",
    b"ul",
];

/// The tag name at `p` (ASCII letters, digits and `-`, a letter first): its end.
#[inline(always)]
fn tag_name(b: &[u8], p: usize) -> Option<usize> {
    if !b.get(p)?.is_ascii_alphabetic() {
        return None;
    }
    let mut q = p + 1;
    while q < b.len() && (b[q].is_ascii_alphanumeric() || b[q] == b'-') {
        q += 1;
    }
    Some(q)
}

fn eq_ci(a: &[u8], lower: &[u8]) -> bool {
    a.len() == lower.len()
        && a.iter()
            .zip(lower)
            .all(|(x, y)| x.to_ascii_lowercase() == *y)
}

/// Whether `b` holds `lower` from `p`, ignoring ASCII case.
fn has_ci(b: &[u8], p: usize, lower: &[u8]) -> bool {
    b.get(p..p + lower.len()).is_some_and(|s| eq_ci(s, lower))
}

/// Whether `b` holds `needle` anywhere, ignoring ASCII case (an HTML block's end mark).
fn contains_ci(b: &[u8], needle: &[u8]) -> bool {
    b.len() >= needle.len()
        && (0..=b.len() - needle.len()).any(|i| eq_ci(&b[i..i + needle.len()], needle))
}

impl<K: Sink> Scan<'_, '_, K> {
    // ---- writing ----

    #[inline(always)]
    fn put(&mut self, s: usize, e: usize, kind: u16) {
        self.k.push(&mut self.out, s, e, kind);
        self.w = e;
    }

    /// The plain text from the last write to `s` in the context, where there is one.
    #[inline(always)]
    fn fill(&mut self, s: usize) {
        if self.ctx != 0 && self.w < s {
            let (w, c) = (self.w, self.ctx);
            self.k.push_call(&mut self.out, w, s, c);
        }
        self.w = self.w.max(s);
    }

    /// A token in the context: the text before it filled first.
    #[inline(always)]
    fn tok(&mut self, s: usize, e: usize, leaf: u16) {
        self.fill(s);
        let c = self.ctx | leaf;
        self.k.push_call(&mut self.out, s, e, c);
        self.w = e;
    }

    // ---- a line ----

    fn line(&mut self) {
        let started = self.b0 & STARTED != 0;
        self.b0 |= STARTED;
        match self.b0 & MODE {
            M_FENCE => return self.fence_line(),
            M_FRONT => return self.front_line(),
            M_HTML | M_HTML_END => {
                if self.html_line() {
                    return;
                }
            }
            M_COMMENT => {
                let Some(p) = self.comment_rest(0) else {
                    return;
                };
                self.b0 = self.b0 & !MODE | M_TEXT;
                let n = self.b.len();
                self.ctx = 0;
                self.inline(p, n);
                return;
            }
            _ => {}
        }
        if !started && !SOL && self.front_open() {
            return;
        }
        self.block(0, 0, 0);
    }

    /// A line of text from `p` at column `col`, inside `quotes` block quotes.
    fn block(&mut self, p0: usize, col0: usize, quotes: u8) {
        let b = self.b;
        let n = b.len();
        let (p, col) = indent(b, p0, col0);
        if p == n {
            // a blank line ends a paragraph and whatever it left open, not a list
            self.b1 = 0;
            self.b2 &= !TABLE;
            return;
        }
        let para = self.b1 & PARA != 0;
        let mut list = usize::from(self.b2 & LIST);
        if list > 0 && col < list && !para && !is_marker(b, p) {
            list = 0;
            self.b2 &= !LIST;
        }
        let base = if col0 > 0 { col0 } else { list };
        if col >= base + 4 && !para && quotes == 0 {
            self.put(p0, n, RAW_BLOCK);
            self.b1 = 0;
            return;
        }
        if col < base + 4 || quotes > 0 {
            match b[p] {
                b'>' => return self.quote(p, col, quotes),
                b'#' if self.heading(p) => return,
                b'-' | b'*' | b'_' | b'+' if self.dash(p, col, quotes, para) => return,
                b'0'..=b'9' if self.ordered(p, col, quotes) => return,
                b'`' | b'~' if self.fence_open(p, col, quotes, list > 0 && col >= list) => return,
                b'<' if !SOL && self.html_open(p, para) => return,
                b'=' if para && self.setext(p, b'=') => return,
                b'[' if !SOL && !para && self.definition(p) => return,
                _ => {}
            }
        }
        self.paragraph(p);
    }

    /// A paragraph's line, or a table's row, from `p`.
    fn paragraph(&mut self, p: usize) {
        let b = self.b;
        let n = b.len();
        if !SOL && self.table_line(p) {
            return;
        }
        self.b2 &= !TABLE;
        self.ctx = self.open_ctx();
        let p = self.open_code(p);
        self.inline(p, n);
        if !SOL && self.ctx == 0 && n >= p + 2 && b[n - 1] == b' ' && b[n - 2] == b' ' {
            let mut s = n - 2;
            while s > self.w && b[s - 1] == b' ' {
                s -= 1;
            }
            if s >= self.w && s > 0 {
                self.put(s, n, HARD_BREAK);
            }
        }
        self.fill(n);
        self.keep_ctx();
        self.b1 |= PARA;
    }

    /// The context the text before left open: bold, italic, strikethrough.
    fn open_ctx(&self) -> u16 {
        let mut c = 0;
        if self.b1 & OPEN_BOLD != 0 {
            c |= BOLD;
        }
        if self.b1 & OPEN_ITALIC != 0 {
            c |= ITALIC;
        }
        if self.b1 & OPEN_STRIKE != 0 {
            c |= STRIKE;
        }
        if self.b2 & OPEN_I_FIRST != 0 && c & (BOLD | ITALIC) == BOLD | ITALIC {
            c |= I_FIRST;
        }
        c
    }

    /// The context left open at the line's end, for the next line of the paragraph.
    fn keep_ctx(&mut self) {
        self.b1 &= !(OPEN_BOLD | OPEN_ITALIC | OPEN_STRIKE);
        if self.ctx & BOLD != 0 {
            self.b1 |= OPEN_BOLD;
        }
        if self.ctx & ITALIC != 0 {
            self.b1 |= OPEN_ITALIC;
        }
        if self.ctx & STRIKE != 0 {
            self.b1 |= OPEN_STRIKE;
        }
        if self.ctx & BOLD == 0 {
            self.b1 &= !BOLD_US;
        }
        if self.ctx & ITALIC == 0 {
            self.b1 &= !ITALIC_US;
        }
        self.b2 &= !OPEN_I_FIRST;
        if self.ctx & I_FIRST != 0 {
            self.b2 |= OPEN_I_FIRST;
        }
        self.ctx = 0;
    }

    /// A code span the line before left open, from `p`: to its closer, or the whole line.
    fn open_code(&mut self, p: usize) -> usize {
        let ticks = usize::from((self.b1 & TICKS) >> T_SHIFT);
        if ticks == 0 {
            return p;
        }
        let n = self.b.len();
        match self.ticks_close(p, ticks) {
            Some(c) => {
                self.tok(p, c, RAW);
                self.tok(c, c + ticks, RAW_END);
                self.b1 &= !TICKS;
                c + ticks
            }
            None => {
                self.tok(p, n, RAW);
                n
            }
        }
    }

    /// `>` markers from `p` at column `col`: each one written, then the rest a block in them.
    fn quote(&mut self, mut p: usize, mut col: usize, mut quotes: u8) {
        let b = self.b;
        loop {
            self.put(p, p + 1, QUOTE);
            p += 1;
            col += 1;
            quotes = (quotes + 1).min(3);
            if p < b.len() && b[p] == b' ' {
                p += 1;
                col += 1;
            }
            let (q, c) = indent(b, p, col);
            if q < b.len() && b[q] == b'>' && c - col < 4 {
                (p, col) = (q, c);
                continue;
            }
            break;
        }
        self.block(p, col, quotes);
    }

    /// An ATX heading at `p`: 1 to 6 `#`s, then a blank or the line's end.
    fn heading(&mut self, p: usize) -> bool {
        let b = self.b;
        let n = b.len();
        let mut q = p;
        while q < n && b[q] == b'#' {
            q += 1;
        }
        let level = q - p;
        if level > 6 || (q < n && !is_ws(b[q])) {
            return false;
        }
        let h = match level {
            1 => H1,
            2 => H2,
            _ => HN,
        };
        self.b1 = 0;
        self.b2 &= !TABLE;
        self.ctx = h;
        self.put(p, q, h | H_BEGIN);
        // the closing sequence: `#`s after a blank, to the line's end but its blanks
        let mut end = n;
        while end > q && is_ws(b[end - 1]) {
            end -= 1;
        }
        let mut close = end;
        while close > q && b[close - 1] == b'#' {
            close -= 1;
        }
        let closer = close < end && (close == q || is_ws(b[close - 1]));
        let mut text_end = if closer { close } else { end };
        while text_end > q && is_ws(b[text_end - 1]) {
            text_end -= 1;
        }
        let text = blanks(b, q).min(text_end);
        self.fill(text);
        self.ctx = h | SECTION;
        self.inline(text, text_end);
        self.fill(text_end);
        self.ctx = h;
        if closer {
            self.tok(close, end, H_END);
        }
        self.fill(n);
        self.ctx = 0;
        true
    }

    /// A line at `p` starting `-`, `*`, `_` or `+`: a setext underline, a thematic break, or a
    /// list item.
    fn dash(&mut self, p: usize, col: usize, quotes: u8, para: bool) -> bool {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        if c == b'-' && para && self.setext(p, b'-') {
            return true;
        }
        if c != b'+' {
            // a thematic break: three or more of the byte, blanks between
            let (mut count, mut q) = (0, p);
            while q < n && (b[q] == c || is_ws(b[q])) {
                count += usize::from(b[q] == c);
                q += 1;
            }
            if q == n && count >= 3 {
                self.b1 = 0;
                self.b2 &= !(TABLE | LIST);
                let mut i = p;
                while i < n {
                    let s = i;
                    while i < n && b[i] == c {
                        i += 1;
                    }
                    if i > s {
                        self.put(s, i, BREAK);
                    }
                    while i < n && is_ws(b[i]) {
                        i += 1;
                    }
                }
                return true;
            }
        }
        if c == b'_' || (p + 1 < n && !is_ws(b[p + 1])) {
            return false;
        }
        self.put(p, p + 1, BULLET);
        self.item(p + 1, col + 1, quotes);
        true
    }

    /// An ordered list item's marker at `p`: 1 to 9 digits, `.` or `)`, a blank or the end.
    fn ordered(&mut self, p: usize, col: usize, quotes: u8) -> bool {
        let b = self.b;
        let n = b.len();
        let mut q = p;
        while q < n && q - p < 10 && b[q].is_ascii_digit() {
            q += 1;
        }
        if q - p > 9 || q >= n || !matches!(b[q], b'.' | b')') || (q + 1 < n && !is_ws(b[q + 1])) {
            return false;
        }
        self.put(p, q, NUMBER);
        self.put(q, q + 1, NUMBER_P);
        self.item(q + 1, col + (q + 1 - p), quotes);
        true
    }

    /// A list item's content from `p` (just past its marker, at column `col`): the content's
    /// column kept, then the rest a block of its own (a fence, a heading, a quote, a list).
    fn item(&mut self, p: usize, col: usize, quotes: u8) {
        let b = self.b;
        let (q, c) = indent(b, p, col);
        let content = if q == b.len() || c - col > 4 {
            col + 1
        } else {
            c
        };
        self.b1 = 0;
        self.b2 = self.b2 & !(LIST | TABLE) | (content.min(31) as u8);
        if q == b.len() {
            return;
        }
        match b[q] {
            b'`' | b'~' if self.fence_open(q, c, quotes, true) => {}
            b'#' if self.heading(q) => {}
            b'>' => self.quote(q, c, quotes),
            b'-' | b'*' | b'+' if self.dash(q, c, quotes, false) => {}
            b'0'..=b'9' if self.ordered(q, c, quotes) => {}
            _ => {
                self.ctx = 0;
                let s = self.open_code(q);
                self.inline(s, b.len());
                self.fill(b.len());
                self.keep_ctx();
            }
        }
    }

    /// A setext underline of `c` at `p` after a paragraph's line.
    fn setext(&mut self, p: usize, c: u8) -> bool {
        let b = self.b;
        let mut q = p;
        while q < b.len() && b[q] == c {
            q += 1;
        }
        if !rest_blank(b, q) {
            return false;
        }
        self.b1 = 0;
        self.put(p, q, if c == b'=' { SETEXT_1 } else { SETEXT_2 });
        true
    }

    // ---- fences ----

    /// A fence's opener at `p`, column `col`: three or more backticks or tildes, an info
    /// string (no backtick in a backtick fence's), its first word the language.
    fn fence_open(&mut self, p: usize, col: usize, quotes: u8, in_list: bool) -> bool {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        let mut q = p;
        while q < n && b[q] == c {
            q += 1;
        }
        let len = q - p;
        if len < 3 || (c == b'`' && b[q..].contains(&b'`')) {
            return false;
        }
        self.put(p, q, FENCE_BEGIN);
        let lang = if SOL { None } else { self.info(q) };
        self.b0 = self.b0 & !(MODE | QUOTES | LOST | TILDE)
            | M_FENCE
            | quotes << Q_SHIFT
            | if c == b'~' { TILDE } else { 0 };
        self.b1 = (len - 3).min(7) as u8
            | if in_list { F_IN_LIST } else { 0 }
            | (col.min(15) as u8) << F_I_SHIFT;
        self.b2 = slot(lang);
        self.st = State::default();
        true
    }

    /// A fence's info string from `p`: its word written, and the language it names.
    fn info(&mut self, p: usize) -> Option<Lang> {
        let b = self.b;
        let n = b.len();
        let mut s = blanks(b, p);
        if s == n {
            return None;
        }
        let braced = b[s] == b'{';
        if braced {
            // R Markdown's and Quarto's `{r}`, pandoc's `{.python}`
            s += 1;
            if s < n && b[s] == b'.' {
                s += 1;
            }
        }
        let mut e = s;
        while e < n && !is_ws(b[e]) && !matches!(b[e], b',' | b'}' | b'{' | b':') {
            e += 1;
        }
        if e == s {
            return None;
        }
        self.put(s, e, LANG_NAME);
        fence_lang(&b[s..e])
    }

    /// A line in a fence: its quotes' markers, its closer, or its code.
    fn fence_line(&mut self) {
        let b = self.b;
        let n = b.len();
        let Some((p, col, marks)) = self.quote_marks() else {
            return self.leave();
        };
        let open_len = usize::from(self.b1 & F_LEN) + 3;
        let f_ind = usize::from(self.b1 >> F_I_SHIFT);
        let (q, c) = indent(b, p, col);
        if self.b1 & F_IN_LIST != 0 && q < n && c < f_ind {
            // a line indented less than the item's fence: the item, and its fence, are over
            return self.leave();
        }
        self.marks(marks);
        let ch = if self.b0 & TILDE != 0 { b'~' } else { b'`' };
        if c < f_ind + 4 && q < n && b[q] == ch {
            let mut e = q;
            while e < n && b[e] == ch {
                e += 1;
            }
            if e - q >= open_len.min(10) && rest_blank(b, e) {
                self.put(q, e, FENCE_END);
                self.close_block();
                // the fence's column is the list item's content's where it stood in one
                if self.b1 & F_IN_LIST != 0 || f_ind > 0 {
                    self.b2 = (f_ind.min(31)) as u8;
                } else {
                    self.b2 = 0;
                }
                self.b1 = 0;
                return;
            }
        }
        // the code: up to the fence's indent stripped
        let (mut at, mut c2) = (p, col);
        while at < n && c2 < f_ind && is_ws(b[at]) {
            at += 1;
            c2 += 1;
        }
        let lang = lang_of(self.b2);
        self.guest(at, lang, FENCE_BODY);
    }

    /// The quotes' markers a fence or an HTML block opened in, on this line: where its text
    /// starts, the column there, and the markers' places; `None` where one is missing (the
    /// quote, and the block, are over). Nothing written: [`Scan::marks`] writes them.
    fn quote_marks(&self) -> Option<(usize, usize, [usize; 3])> {
        let b = self.b;
        let quotes = (self.b0 & QUOTES) >> Q_SHIFT;
        let (mut p, mut col) = (0, 0);
        let mut at = [usize::MAX; 3];
        for m in at.iter_mut().take(usize::from(quotes)) {
            let (q, c) = indent(b, p, col);
            if q >= b.len() || b[q] != b'>' || c - col > 3 {
                return None;
            }
            *m = q;
            (p, col) = (q + 1, c + 1);
            if p < b.len() && b[p] == b' ' {
                p += 1;
                col += 1;
            }
        }
        Some((p, col, at))
    }

    /// The quotes' markers [`Scan::quote_marks`] found, written.
    fn marks(&mut self, at: [usize; 3]) {
        for q in at.into_iter().filter(|&q| q != usize::MAX) {
            self.put(q, q + 1, QUOTE);
        }
    }

    /// A fence or an HTML block over: text again, its fields fresh.
    fn close_block(&mut self) {
        self.b0 &= !(MODE | QUOTES | LOST | TILDE);
        self.b1 = 0;
        self.b2 = 0;
        self.st = State::default();
    }

    /// The block over before this line, which is text.
    fn leave(&mut self) {
        self.close_block();
        self.block(0, 0, 0);
    }

    /// Code in `lang` from `at` to the line's end: lexed on its own scanner from the state the
    /// fence carries, on the run sink by its own rules; elsewhere `raw`, its state carried all
    /// the same. Without a language, `raw`.
    fn guest(&mut self, at: usize, lang: Option<Lang>, raw: u16) {
        let n = self.b.len();
        let Some(lang) = lang.filter(|_| !RAW_FENCES) else {
            if raw != TEXT {
                self.put(at, n, raw);
            }
            return;
        };
        if at > n {
            return;
        }
        let lost = self.b0 & LOST != 0;
        // markdown in markdown keeps its own bytes in `levels`' first three, its fences' code
        // lexed from a fresh state each line
        let nested = lang == Lang::Markdown;
        let mut g = if lost || nested {
            State::default()
        } else {
            self.st
        };
        for (i, at) in [B0, B1, B2].into_iter().enumerate() {
            g.set_byte(at, if nested { self.st.byte(i) } else { 0 });
        }
        // SAFETY: `at` is past ASCII bytes only (blanks and `>`), so on a character of the
        // line, which came from a `&str`
        let text = unsafe { std::str::from_utf8_unchecked(&self.b[at..]) };
        let mut kept = false;
        if K::RUNS {
            let spans = &mut *self.out.0;
            kept = self.k.guest(at, lang, &mut |rules, runs| {
                crate::lex_runs(lang, text, &mut g, spans, rules, runs);
            });
            if kept {
                self.w = n;
            }
        }
        if !kept {
            crate::lex_state(lang, text, &mut g);
            if raw != TEXT {
                self.put(at, n, raw);
            }
        }
        if nested {
            self.st = State::default();
            for (i, at) in [B0, B1, B2].into_iter().enumerate() {
                self.st.set_byte(i, g.byte(at));
            }
            return;
        }
        let set = g.byte(B0) | g.byte(B1) | g.byte(B2);
        if lost || set != 0 {
            self.b0 |= LOST;
            self.st = State::default();
        } else {
            self.st = g;
        }
    }

    // ---- front matter ----

    /// Front matter on the file's first line: `---` (YAML's) or `+++` (TOML's).
    fn front_open(&mut self) -> bool {
        let b = self.b;
        let lang = match b.get(..3) {
            Some(b"---") => Lang::Yaml,
            Some(b"+++") => Lang::Toml,
            _ => return false,
        };
        if !rest_blank(b, 3) {
            return false;
        }
        self.put(0, 3, FRONT_BEGIN);
        self.b0 = self.b0 & !MODE | M_FRONT;
        self.b1 = 0;
        self.b2 = slot(Some(lang));
        true
    }

    /// A line of front matter: its closer, or its language's.
    fn front_line(&mut self) {
        let b = self.b;
        let lang = lang_of(self.b2);
        let closer: &[&[u8]] = if lang == Some(Lang::Toml) {
            &[b"+++"]
        } else {
            &[b"---", b"..."]
        };
        if closer.iter().any(|c| b.starts_with(c)) && rest_blank(b, 3) {
            self.put(0, 3, FRONT_END);
            self.close_block();
            return;
        }
        self.guest(0, lang, TEXT);
    }

    // ---- HTML ----

    /// HTML's scanner on `at..e`, from `st`, its kinds under [`HTML`].
    fn html(&mut self, at: usize, e: usize, st: &mut State) {
        self.fill(at);
        let b = &self.b[at..e];
        let out = &mut *self.out;
        if K::RUNS {
            self.k.host(at, HTML, &mut |rules, runs| {
                crate::html::line_runs(b, st, out, rules, runs);
            });
        } else {
            self.k.close(out);
            let from = out.0.len();
            crate::html::line(b, st, out);
            for s in &mut out.0[from..] {
                s.start += at as u32;
                s.end += at as u32;
                s.kind |= HTML;
            }
            self.k.reopen(out, 8);
        }
        self.w = e;
    }

    /// An HTML block opening at `p`: its first line HTML's, and the mode to its end.
    fn html_open(&mut self, p: usize, para: bool) -> bool {
        let b = self.b;
        let n = b.len();
        let q = p + 1;
        if b[q..].starts_with(b"!--") {
            // a comment: read as markdown reads one in text
            return false;
        }
        let end = if has_ci(b, q, b"![cdata[") {
            Some(E_CDATA)
        } else if q < n && b[q] == b'?' {
            Some(E_PI)
        } else if q < n && b[q] == b'!' && b.get(q + 1).is_some_and(u8::is_ascii_alphabetic) {
            Some(E_DECL)
        } else {
            let name_at = if b.get(q) == Some(&b'/') { q + 1 } else { q };
            let Some(ne) = tag_name(b, name_at) else {
                return false;
            };
            let name = &b[name_at..ne];
            let after_ok =
                ne == n || matches!(b[ne], b' ' | b'\t' | b'>') || b[ne..].starts_with(b"/>");
            if !after_ok {
                return false;
            }
            let raw = [
                (b"script".as_slice(), E_SCRIPT),
                (b"pre", E_PRE),
                (b"style", E_STYLE),
                (b"textarea", E_TEXTAREA),
            ];
            match raw.iter().find(|(t, _)| name_at == q && eq_ci(name, t)) {
                Some(&(_, e)) => Some(e),
                None if BLOCK_TAGS.iter().any(|t| eq_ci(name, t)) => None,
                None => {
                    // the seventh kind: a whole tag alone on its line, not in a paragraph
                    let Some(te) = self.tag_end(p) else {
                        return false;
                    };
                    if para || !rest_blank(b, te) {
                        return false;
                    }
                    None
                }
            }
        };
        let quotes = self.b0 & QUOTES;
        self.b0 = self.b0 & !(MODE | QUOTES | LOST) | quotes;
        match end {
            Some(e) => {
                self.b0 |= M_HTML_END;
                self.b1 = e;
            }
            None => {
                self.b0 |= M_HTML;
                self.b1 = 0;
            }
        }
        self.b2 = 0;
        self.st = State::default();
        self.html_body(p);
        true
    }

    /// An HTML block's line from `p`: HTML's, the block closed where its end mark is.
    fn html_body(&mut self, p: usize) {
        let n = self.b.len();
        let mut st = if self.b0 & LOST != 0 {
            State::default()
        } else {
            self.st
        };
        self.html(p, n, &mut st);
        let set = st.byte(B0) | st.byte(B1) | st.byte(B2);
        if set != 0 {
            self.b0 |= LOST;
            self.st = State::default();
        } else {
            self.st = st;
        }
        if self.b0 & MODE == M_HTML_END {
            let mark: &[u8] = match self.b1 {
                E_SCRIPT => b"</script>",
                E_PRE => b"</pre>",
                E_STYLE => b"</style>",
                E_TEXTAREA => b"</textarea>",
                E_PI => b"?>",
                E_DECL => b">",
                _ => b"]]>",
            };
            if contains_ci(&self.b[p..], mark) {
                self.close_block();
            }
        }
    }

    /// A line in an HTML block: `true` where it was the block's; `false` for a blank line that
    /// ended it, or a missing quote marker, which is text again.
    fn html_line(&mut self) -> bool {
        let Some((p, _, marks)) = self.quote_marks() else {
            self.close_block();
            return false;
        };
        if self.b0 & MODE == M_HTML && rest_blank(self.b, p) {
            self.close_block();
            return false;
        }
        self.marks(marks);
        self.html_body(p);
        true
    }

    /// Where the tag at `p` (`<` there) ends: an open tag with its attributes, or a closing
    /// one, on this line.
    fn tag_end(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        let mut q = p + 1;
        if b.get(q) == Some(&b'/') {
            let e = tag_name(b, q + 1)?;
            let e = blanks(b, e);
            return (b.get(e) == Some(&b'>')).then_some(e + 1);
        }
        q = tag_name(b, q)?;
        if p >= bound(self.memo.gt) {
            return None;
        }
        loop {
            let s = blanks(b, q);
            match b.get(s)? {
                b'>' => return Some(s + 1),
                b'/' => return (b.get(s + 1) == Some(&b'>')).then_some(s + 2),
                _ => {}
            }
            if s == q {
                return None;
            }
            // an attribute: a name, then a value after `=`
            let c = *b.get(s)?;
            if !(c.is_ascii_alphabetic() || matches!(c, b'_' | b':' | b'@')) {
                return None;
            }
            q = s + 1;
            while q < n
                && (b[q].is_ascii_alphanumeric()
                    || matches!(b[q], b'_' | b'.' | b':' | b'-' | b'@'))
            {
                q += 1;
            }
            let v = blanks(b, q);
            if b.get(v) != Some(&b'=') {
                continue;
            }
            let v = blanks(b, v + 1);
            match *b.get(v)? {
                quote @ (b'"' | b'\'') => {
                    let memo = bound(if quote == b'"' {
                        self.memo.dq
                    } else {
                        self.memo.sq
                    });
                    if v >= memo {
                        return None;
                    }
                    match b[v + 1..].iter().position(|&x| x == quote) {
                        Some(i) => q = v + 1 + i + 1,
                        None => {
                            if quote == b'"' {
                                self.memo.dq = mark(v);
                            } else {
                                self.memo.sq = mark(v);
                            }
                            return None;
                        }
                    }
                }
                _ => {
                    q = v;
                    while q < n
                        && !is_ws(b[q])
                        && !matches!(b[q], b'"' | b'\'' | b'=' | b'<' | b'>' | b'`')
                    {
                        q += 1;
                    }
                    if q == v {
                        return None;
                    }
                }
            }
        }
    }

    // ---- comments ----

    /// A comment's text from `p`, open from the line before or an opener's: written to its
    /// `-->`, and where the text goes on after it; `None` where it runs on past the line.
    fn comment_rest(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        match find_close(b, p) {
            Some(c) => {
                self.put(p, c, COMMENT);
                self.put(c, c + 3, COMMENT_END);
                Some(c + 3)
            }
            None => {
                self.put(p, n, COMMENT);
                None
            }
        }
    }

    // ---- tables ----

    /// A table's row at `p`: a delimiter row, a row starting with `|`, or a row after one.
    fn table_line(&mut self, p: usize) -> bool {
        let b = self.b;
        let n = b.len();
        let mut q = p;
        let (mut pipe, mut dash) = (false, false);
        while q < n && matches!(b[q], b'|' | b'-' | b':' | b' ' | b'\t') {
            pipe |= b[q] == b'|';
            dash |= b[q] == b'-';
            q += 1;
        }
        if q == n && pipe && dash {
            for i in p..n {
                match b[i] {
                    b'|' => self.put(i, i + 1, TABLE_SEP),
                    b':' => self.put(i, i + 1, TABLE_ALIGN),
                    b'-' if i == p || b[i - 1] != b'-' => {
                        let mut e = i;
                        while e < n && b[e] == b'-' {
                            e += 1;
                        }
                        self.put(i, e, TABLE_HEADER);
                    }
                    _ => {}
                }
            }
            self.b1 = 0;
            self.b2 |= TABLE;
            return true;
        }
        let row = b[p] == b'|' || (self.b2 & TABLE != 0 && b[p..].contains(&b'|'));
        if !row {
            return false;
        }
        self.b1 = 0;
        self.b2 |= TABLE;
        self.ctx = 0;
        self.inline(p, n);
        self.fill(n);
        self.ctx = 0;
        true
    }

    // ---- link reference definitions ----

    /// `[label]: destination "title"` at `p`, the line's whole text.
    fn definition(&mut self, p: usize) -> bool {
        let b = self.b;
        let n = b.len();
        let mut c = p + 1;
        while c < n && b[c] != b']' && b[c] != b'[' {
            c += if b[c] == b'\\' { 2 } else { 1 };
        }
        if c >= n || b[c] != b']' || c == p + 1 || b.get(c + 1) != Some(&b':') {
            return false;
        }
        let d = blanks(b, c + 2);
        let footnote = b[p + 1] == b'^';
        if !footnote {
            // a destination, then perhaps a title, then nothing
            if d == n {
                return false;
            }
            let (ds, de, next) = if b[d] == b'<' {
                let Some(i) = b[d + 1..].iter().position(|&x| x == b'>') else {
                    return false;
                };
                (d + 1, d + 1 + i, d + 1 + i + 1)
            } else {
                let mut e = d;
                while e < n && !is_ws(b[e]) {
                    e += 1;
                }
                (d, e, e)
            };
            let t = blanks(b, next);
            let title = if t < n && matches!(b[t], b'"' | b'\'' | b'(') && t > next {
                let close = if b[t] == b'(' { b')' } else { b[t] };
                let Some(i) = b[t + 1..].iter().position(|&x| x == close) else {
                    return false;
                };
                if !rest_blank(b, t + 1 + i + 1) {
                    return false;
                }
                Some((t, t + 1 + i))
            } else if t < n {
                return false;
            } else {
                None
            };
            self.put(p, p + 1, REF_BEGIN);
            self.put(p + 1, c, REF_DEF);
            self.put(c, c + 1, REF_END);
            self.put(c + 1, c + 2, KEY_VALUE);
            if b[d] == b'<' {
                self.put(d, d + 1, LINK_BEGIN);
                self.put(ds, de, URL);
                self.put(de, de + 1, LINK_END);
            } else {
                self.put(ds, de, URL);
            }
            if let Some((ts, te)) = title {
                self.put(ts, ts + 1, TITLE_BEGIN);
                self.put(ts + 1, te, TITLE);
                self.put(te, te + 1, TITLE_END);
            }
            self.b1 = 0;
            return true;
        }
        self.put(p, p + 1, REF_BEGIN);
        self.put(p + 1, c, REF_DEF);
        self.put(c, c + 1, REF_END);
        self.put(c + 1, c + 2, KEY_VALUE);
        self.ctx = 0;
        self.inline(c + 2, n);
        self.fill(n);
        self.keep_ctx();
        self.b1 |= PARA;
        true
    }

    // ---- inline ----

    /// The text `p..e` of a line, in the scan's context.
    fn inline(&mut self, mut p: usize, e: usize) {
        let b = self.b;
        let text = &b[..e];
        loop {
            p = skip_plain(text, p);
            if p >= text.len() {
                break;
            }
            let c = text[p];
            let q = match c {
                b'\\' => self.escape(p, e),
                b'`' => self.code(p, e),
                b'*' | b'_' => self.emphasis(p, e),
                b'~' => self.emphasis(p, e),
                b'[' => self.link(p, e, false),
                b'!' => {
                    if p + 1 < e && b[p + 1] == b'[' {
                        self.link(p + 1, e, true)
                    } else {
                        p + 1
                    }
                }
                b'<' => self.angle(p, e),
                b'&' => self.entity(p, e),
                b':' => self.bare(p, e),
                b'@' => self.email(p, e),
                b'.' => self.www(p, e),
                b'|' => {
                    if self.b2 & TABLE != 0 {
                        self.tok(p, p + 1, TABLE_SEP);
                    }
                    p + 1
                }
                _ => p + 1,
            };
            debug_assert!(q > p, "the inline loop moves on");
            p = q;
        }
    }

    fn escape(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        if p + 1 == e && e == b.len() {
            // a hard break
            self.tok(p, p + 1, ESCAPE);
            return e;
        }
        if p + 1 < e && b[p + 1].is_ascii_punctuation() {
            self.tok(p, p + 2, ESCAPE);
            return p + 2;
        }
        p + 1
    }

    /// The closer of a code span of `n` backticks after `p`: where it starts.
    fn ticks_close(&mut self, p: usize, n: usize) -> Option<usize> {
        let b = self.b;
        let slot = n.min(8) - 1;
        if n > 8 || p >= bound(self.memo.ticks[slot]) {
            return None;
        }
        let mut q = p;
        while q < b.len() {
            match b[q..].iter().position(|&x| x == b'`') {
                None => break,
                Some(i) => {
                    let s = q + i;
                    let mut t = s;
                    while t < b.len() && b[t] == b'`' {
                        t += 1;
                    }
                    if t - s == n {
                        return Some(s);
                    }
                    q = t;
                }
            }
        }
        self.memo.ticks[slot] = mark(p);
        None
    }

    fn code(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let mut q = p;
        while q < e && b[q] == b'`' {
            q += 1;
        }
        let n = q - p;
        match self.ticks_close(q, n) {
            Some(c) if c + n <= e => {
                self.tok(p, q, RAW_BEGIN);
                self.tok(q, c, RAW);
                self.tok(c, c + n, RAW_END);
                c + n
            }
            Some(_) => q,
            None if n <= 3 && e == b.len() => {
                // open to the paragraph's next lines, as bat reads one
                self.tok(p, q, RAW_BEGIN);
                self.tok(q, e, RAW);
                self.b1 = self.b1 & !TICKS | (n as u8) << T_SHIFT;
                e
            }
            None => q,
        }
    }

    /// A delimiter run of `*`, `_` or `~` at `p`: what the line's matching made of it
    /// ([`Scan::match_delims`]), written: its closers, then text, then its openers.
    fn emphasis(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let c = b[p];
        let mut q = p;
        while q < e && b[q] == c {
            q += 1;
        }
        if !self.delims.done {
            self.match_delims(p);
        }
        let Some(d) = self.delims.find(p) else {
            return q;
        };
        let mut at = p;
        for &u in &d.close[..usize::from(d.nc)] {
            let n = usize::from(u);
            let (bit, end) = match (c, n) {
                (b'~', _) => (STRIKE, STRIKE_END),
                (_, 2) => (BOLD, BOLD_END),
                _ => (ITALIC, ITALIC_END),
            };
            if at + n <= q && self.ctx & bit != 0 {
                self.tok(at, at + n, end);
                self.ctx &= !(bit | I_FIRST);
            }
            at += n;
        }
        // the openers from the run's end: the last one matched (the outermost) first
        let opens: usize = d.open[..usize::from(d.no)]
            .iter()
            .map(|&u| usize::from(u))
            .sum();
        let mut o = q.saturating_sub(opens).max(at);
        for &u in d.open[..usize::from(d.no)].iter().rev() {
            let n = usize::from(u);
            if o + n > q {
                break;
            }
            let (bit, begin) = match (c, n) {
                (b'~', _) => (STRIKE, STRIKE_BEGIN),
                (_, 2) => (BOLD, BOLD_BEGIN),
                _ => (ITALIC, ITALIC_BEGIN),
            };
            self.fill(o);
            if bit == BOLD && self.ctx & ITALIC != 0 && self.ctx & BOLD == 0 {
                self.ctx |= I_FIRST;
            }
            self.ctx |= bit;
            let us = if c == b'_' { 0xff } else { 0 };
            match bit {
                BOLD => self.b1 = self.b1 & !BOLD_US | (BOLD_US & us),
                ITALIC => self.b1 = self.b1 & !ITALIC_US | (ITALIC_US & us),
                _ => {}
            }
            self.tok(o, o + n, begin);
            o += n;
        }
        q
    }

    /// The line's delimiter runs from `p` (its first) on, matched as CommonMark's emphasis is:
    /// each closer, left to right, with the nearest opener of its byte before it (the rule of
    /// three kept; `~` by the same length), two of each at a time where both have them, the runs
    /// between dropped. What the line before left open is an opener below the line's. An opener
    /// left over opens across lines only at a word's start before a letter or digit, in a
    /// paragraph. The runs are found as the inline loop finds them: past escapes, code spans,
    /// tags, autolinks, bare links and links' destinations.
    fn match_delims(&mut self, p: usize) {
        let b = self.b;
        let n = b.len();
        let mut d = Delims::NONE;
        d.done = true;
        // what the line before left open, outermost first
        let ctx = self.ctx;
        let mut virt = |len: u8, c: u8| {
            d.push(Delim::new(u32::MAX, len, c, D_OPEN | D_VIRTUAL));
        };
        let us = |bit: u8| if self.b1 & bit != 0 { b'_' } else { b'*' };
        if ctx & STRIKE != 0 {
            virt(2, b'~');
        }
        let (bold, italic) = (ctx & BOLD != 0, ctx & ITALIC != 0);
        if italic && ctx & I_FIRST != 0 {
            virt(1, us(ITALIC_US));
        }
        if bold {
            virt(2, us(BOLD_US));
        }
        if italic && ctx & I_FIRST == 0 {
            virt(1, us(ITALIC_US));
        }
        let mut i = p;
        while i < n {
            match b[i] {
                b'\\' => i += 2,
                b'`' => {
                    let mut q = i;
                    while q < n && b[q] == b'`' {
                        q += 1;
                    }
                    match self.ticks_close(q, q - i) {
                        Some(c) => i = c + (q - i),
                        // the rest is a code span open across lines
                        None if q - i <= 3 => break,
                        None => i = q,
                    }
                }
                b'<' => {
                    i = if let Some(q) = autolink_end(b, i, n) {
                        q
                    } else if b[i..].starts_with(b"<!--") {
                        find_close(b, i + 4).map_or(n, |c| c + 3)
                    } else if b
                        .get(i + 1)
                        .is_some_and(|&x| x.is_ascii_alphabetic() || x == b'/')
                        && let Some(te) = self.tag_end(i)
                    {
                        te
                    } else {
                        i + 1
                    };
                }
                b']' if b.get(i + 1) == Some(&b'(') => {
                    i = match self.destination(i + 2, n) {
                        Some((.., close)) => close + 1,
                        None => i + 1,
                    };
                }
                b':' => i = bare_end(b, i, n).unwrap_or(i + 1),
                c @ (b'*' | b'_' | b'~') => {
                    let s = i;
                    while i < n && b[i] == c {
                        i += 1;
                    }
                    let len = i - s;
                    if c == b'~' && len > 2 {
                        continue;
                    }
                    let prev = if s > 0 { b[s - 1] } else { b' ' };
                    let next = if i < n { b[i] } else { b' ' };
                    let (left, right) = flanks(prev, next);
                    let (open, close) = if c == b'_' {
                        (
                            left && (!right || prev.is_ascii_punctuation()),
                            right && (!left || next.is_ascii_punctuation()),
                        )
                    } else {
                        (left, right)
                    };
                    let word = !(prev.is_ascii_alphanumeric() || prev == b'_')
                        && next.is_ascii_alphanumeric();
                    let flags = if open { D_OPEN } else { 0 }
                        | if close { D_CLOSE } else { 0 }
                        | if word { D_WORD } else { 0 };
                    if flags & (D_OPEN | D_CLOSE) != 0 {
                        d.push(Delim::new(s as u32, len.min(255) as u8, c, flags));
                    }
                }
                _ => i += 1,
            }
        }
        d.resolve();
        if self.ctx & H_MASK == 0 && self.b2 & TABLE == 0 {
            d.carry();
        }
        self.delims = d;
    }

    /// The line's `[`s paired with their `]`s from `p` on, once.
    fn pair_brackets(&mut self, p: usize) {
        if self.brackets.done {
            return;
        }
        self.brackets.done = true;
        let b = self.b;
        let mut stack = [0u32; Brackets::MAX];
        let mut depth = 0;
        let mut i = p;
        while i < b.len() {
            match b[i] {
                b'\\' => i += 1,
                b'[' => {
                    if depth < Brackets::MAX {
                        stack[depth] = i as u32;
                    }
                    depth += 1;
                }
                b']' if depth > 0 => {
                    depth -= 1;
                    if depth < Brackets::MAX && usize::from(self.brackets.n) < Brackets::MAX {
                        let k = usize::from(self.brackets.n);
                        self.brackets.pairs[k] = MaybeUninit::new((stack[depth], i as u32));
                        self.brackets.n += 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    fn closing(&self, p: usize) -> Option<usize> {
        self.brackets.pairs[..usize::from(self.brackets.n)]
            .iter()
            // SAFETY: the pairs below `n` are written
            .map(|x| unsafe { x.assume_init() })
            .find(|&(o, _)| o as usize == p)
            .map(|(_, c)| c as usize)
    }

    /// A link's or (`image`, `!` before `p`) an image's text from `p`'s `[`: its destination and
    /// title in `(…)`, or its reference in `[…]`.
    fn link(&mut self, p: usize, e: usize, image: bool) -> usize {
        let b = self.b;
        self.pair_brackets(p);
        let Some(c) = self.closing(p).filter(|&c| c < e) else {
            return p + 1;
        };
        let start = if image { p - 1 } else { p };
        let (begin, end) = if image {
            (IMAGE_BEGIN, IMAGE_END)
        } else {
            (LINK_BEGIN, LINK_END)
        };
        match b.get(c + 1) {
            Some(b'(') if c + 1 < e => {
                let Some((ds, de, title, close)) = self.destination(c + 2, e) else {
                    return p + 1;
                };
                self.tok(start, p + 1, begin);
                self.inline(p + 1, c);
                self.tok(c, c + 1, end);
                self.tok(c + 1, c + 2, META_BEGIN);
                let url = if image { URL_IMAGE } else { URL };
                if ds < de && b[ds] == b'<' {
                    self.tok(ds, ds + 1, LINK_BEGIN);
                    self.tok(ds + 1, de - 1, url);
                    self.tok(de - 1, de, LINK_END);
                } else {
                    self.tok(ds, de, url);
                }
                if let Some((ts, te)) = title {
                    self.tok(ts, ts + 1, TITLE_BEGIN);
                    self.tok(ts + 1, te, TITLE);
                    self.tok(te, te + 1, TITLE_END);
                }
                self.tok(close, close + 1, META_END);
                close + 1
            }
            Some(b'[') if c + 1 < e => {
                let r = c + 1;
                self.pair_brackets(r);
                let Some(rc) = self.closing(r).filter(|&x| x < e) else {
                    return p + 1;
                };
                self.tok(start, p + 1, begin);
                self.inline(p + 1, c);
                self.tok(c, c + 1, end);
                self.tok(r, r + 1, REF_BEGIN);
                self.tok(r + 1, rc, REF);
                self.tok(rc, rc + 1, REF_END);
                rc + 1
            }
            _ if !image && b.get(p + 1) == Some(&b'^') => {
                // a footnote's reference, as bat reads it: a link's brackets around plain text
                self.tok(p, p + 1, LINK_BEGIN);
                self.tok(c, c + 1, LINK_END);
                c + 1
            }
            _ => p + 1,
        }
    }

    /// An inline link's `(…)` from `p` (past its `(`): the destination, the title's quotes, and
    /// the `)`.
    #[allow(clippy::type_complexity)]
    fn destination(
        &mut self,
        p: usize,
        e: usize,
    ) -> Option<(usize, usize, Option<(usize, usize)>, usize)> {
        let b = self.b;
        let ds = blanks(b, p);
        let de = if b.get(ds) == Some(&b'<') {
            let i = b[ds + 1..e].iter().position(|&x| x == b'>' || x == b'<')?;
            if b[ds + 1 + i] != b'>' {
                return None;
            }
            ds + 1 + i + 1
        } else {
            let mut q = ds;
            let mut depth = 0usize;
            while q < e {
                match b[q] {
                    b' ' | b'\t' => break,
                    b'\\' => q += 1,
                    b'(' => depth += 1,
                    b')' if depth == 0 => break,
                    b')' => depth -= 1,
                    c if c < 0x20 => return None,
                    _ => {}
                }
                q += 1;
            }
            q.min(e)
        };
        let t = blanks(b, de);
        if t >= e {
            return None;
        }
        if b[t] == b')' {
            return Some((ds, de, None, t));
        }
        if t == de || !matches!(b[t], b'"' | b'\'' | b'(') {
            return None;
        }
        let close = if b[t] == b'(' { b')' } else { b[t] };
        let memo = match close {
            b'"' => bound(self.memo.dq),
            b'\'' => bound(self.memo.sq),
            _ => usize::MAX,
        };
        if t >= memo {
            return None;
        }
        let Some(i) = b[t + 1..e].iter().position(|&x| x == close) else {
            match close {
                b'"' => self.memo.dq = mark(t),
                b'\'' => self.memo.sq = mark(t),
                _ => {}
            }
            return None;
        };
        let te = t + 1 + i;
        let r = blanks(b, te + 1);
        (r < e && b[r] == b')').then_some((ds, de, Some((t, te)), r))
    }

    /// `<` at `p`: an autolink, a comment, or an HTML tag.
    fn angle(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        if b[p..e].starts_with(b"<!--") {
            // closed before `e`, or open to the next lines from a line's text
            let close = find_close(&b[..e], p + 4);
            if close.is_some() || e == b.len() {
                self.tok(p, p + 4, COMMENT_BEGIN);
                if self.comment_rest(p + 4).is_none() {
                    self.b0 = self.b0 & !MODE | M_COMMENT;
                }
                return close.map_or(e, |c| c + 3);
            }
        }
        if let Some(q) = self.autolink(p, e) {
            return q;
        }
        let next = b.get(p + 1).copied().unwrap_or(0);
        if next.is_ascii_alphabetic() || next == b'/' {
            if self.memo.gt == 0 {
                self.memo.gt = mark(
                    b[p..]
                        .iter()
                        .rposition(|&x| x == b'>')
                        .map_or(p, |i| p + i + 1),
                );
            }
            if let Some(te) = self.tag_end(p).filter(|&te| te <= e) {
                let mut st = State::default();
                self.html(p, te, &mut st);
                return te;
            }
        }
        p + 1
    }

    /// `<scheme:…>` or `<address@host>` at `p`.
    fn autolink(&mut self, p: usize, e: usize) -> Option<usize> {
        let q = autolink_end(self.b, p, e)? - 1;
        self.tok(p, p + 1, LINK_BEGIN);
        self.tok(p + 1, q, URL);
        self.tok(q, q + 1, LINK_END);
        Some(q + 1)
    }

    /// `&` at `p`: an entity, named, decimal or hexadecimal.
    fn entity(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let (body, base) = match b.get(p + 1) {
            Some(b'#') if matches!(b.get(p + 2), Some(b'x' | b'X')) => (p + 3, ENTITY + 6),
            Some(b'#') => (p + 2, ENTITY + 3),
            _ => (p + 1, ENTITY),
        };
        let mut q = body;
        while q < e && q - body < 32 && b[q].is_ascii_alphanumeric() {
            q += 1;
        }
        let ok = q > body
            && q < e
            && b[q] == b';'
            && match base - ENTITY {
                3 => b[body..q].iter().all(u8::is_ascii_digit),
                6 => b[body..q].iter().all(u8::is_ascii_hexdigit),
                _ => b[body].is_ascii_alphabetic(),
            };
        if !ok {
            return p + 1;
        }
        self.tok(p, body, base);
        self.tok(body, q, base + 1);
        self.tok(q, q + 1, base + 2);
        q + 1
    }

    /// `:` at `p`: a bare link's `://`, its scheme before it (`http`, `https`, `ftp`).
    fn bare(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        if !(b.get(p + 1) == Some(&b'/') && b.get(p + 2) == Some(&b'/')) {
            return p + 1;
        }
        match scheme_start(b, p) {
            Some(s) if s >= self.w => self.url_to(s, p + 3, e),
            _ => p + 1,
        }
    }

    /// `.` at `p`: a bare link's `www.`.
    fn www(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let Some(s) = p.checked_sub(3) else {
            return p + 1;
        };
        if b[p - 1] | 0x20 != b'w' {
            return p + 1;
        }
        if !(eq_ci(&b[s..p], b"www")
            && (s == 0 || is_ws(b[s - 1]) || matches!(b[s - 1], b'(' | b'*' | b'_' | b'~')))
            || s < self.w
            || !b.get(p + 1).is_some_and(u8::is_ascii_alphanumeric)
        {
            return p + 1;
        }
        self.url_to(s, p + 1, e)
    }

    /// A bare link from `s`, its text past `from`: to a blank or `<`, its trailing punctuation
    /// left out.
    fn url_to(&mut self, s: usize, from: usize, e: usize) -> usize {
        let q = url_end(self.b, from, e);
        if q <= from {
            return from;
        }
        self.tok(s, q, URL);
        q
    }

    /// `@` at `p`: an email address, its local part before it and a dotted domain after.
    fn email(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let mut s = p;
        while s > self.w
            && (b[s - 1].is_ascii_alphanumeric() || matches!(b[s - 1], b'.' | b'-' | b'_' | b'+'))
        {
            s -= 1;
        }
        let mut q = p + 1;
        let mut dot = false;
        while q < e && (b[q].is_ascii_alphanumeric() || matches!(b[q], b'.' | b'-' | b'_')) {
            dot |= b[q] == b'.';
            q += 1;
        }
        while q > p + 1 && matches!(b[q - 1], b'.' | b'-' | b'_') {
            q -= 1;
        }
        if s == p || q == p + 1 || !dot || !b[q - 1].is_ascii_alphanumeric() {
            return p + 1;
        }
        self.tok(s, q, URL);
        q
    }
}

/// `<scheme:…>` or `<address@host>` at `p`, within `e`: where it ends, past its `>`.
fn autolink_end(b: &[u8], p: usize, e: usize) -> Option<usize> {
    let mut q = p + 1;
    while q < e && (b[q].is_ascii_alphanumeric() || matches!(b[q], b'+' | b'.' | b'-')) {
        q += 1;
    }
    let scheme = q - (p + 1);
    let at_email = q < e && b[q] == b'@';
    let at_scheme =
        q < e && b[q] == b':' && (2..=32).contains(&scheme) && b[p + 1].is_ascii_alphabetic();
    if !(at_scheme || at_email) {
        return None;
    }
    while q < e && b[q] != b'>' && b[q] != b'<' && !is_ws(b[q]) {
        q += 1;
    }
    (q < e && b[q] == b'>').then_some(q + 1)
}

/// A bare link's scheme before the `:` at `p` (`http`, `https`, `ftp`, at a word's start).
fn scheme_start(b: &[u8], p: usize) -> Option<usize> {
    ["https", "http", "ftp"].iter().find_map(|scheme| {
        let s = p.checked_sub(scheme.len())?;
        (eq_ci(&b[s..p], scheme.as_bytes()) && (s == 0 || !b[s - 1].is_ascii_alphanumeric()))
            .then_some(s)
    })
}

/// A bare link's end, its text past `from`: to a blank or `<`, its trailing punctuation left out.
fn url_end(b: &[u8], from: usize, e: usize) -> usize {
    let mut q = from;
    while q < e && !is_ws(b[q]) && b[q] != b'<' {
        q += 1;
    }
    while q > from
        && matches!(
            b[q - 1],
            b'.' | b',' | b':' | b';' | b'!' | b'?' | b'"' | b'\'' | b'*' | b'_' | b'~' | b')'
        )
    {
        q -= 1;
    }
    q
}

/// A bare link at the `:` at `p` (its `://`): where it ends.
fn bare_end(b: &[u8], p: usize, e: usize) -> Option<usize> {
    if !(b.get(p + 1) == Some(&b'/') && b.get(p + 2) == Some(&b'/')) {
        return None;
    }
    scheme_start(b, p)?;
    let q = url_end(b, p + 3, e);
    (q > p + 3).then_some(q)
}

/// The language a fence's info word names ([`Lang::from_token`]), through a few words a thread
/// met last: a file's fences name a few languages, and the lookup reads every language's names.
fn fence_lang(word: &[u8]) -> Option<Lang> {
    const N: usize = 4;
    const W: usize = 16;
    std::thread_local! {
        static LAST: std::cell::Cell<[([u8; W], u8, u8); N]> =
            const { std::cell::Cell::new([([0; W], 0, 0); N]) };
    }
    if word.is_empty() || word.len() > W {
        return std::str::from_utf8(word).ok().and_then(Lang::from_token);
    }
    let mut key = [0u8; W];
    key[..word.len()].copy_from_slice(word);
    let len = word.len() as u8;
    let mut last = LAST.with(std::cell::Cell::get);
    if let Some(i) = last.iter().position(|&(k, l, _)| l == len && k == key) {
        return lang_of(last[i].2);
    }
    let lang = std::str::from_utf8(word).ok().and_then(Lang::from_token);
    last.rotate_right(1);
    last[0] = (key, len, slot(lang));
    LAST.with(|c| c.set(last));
    lang
}

/// Whether a list's marker stands at `p`: a bullet or an ordered item's number, then a blank or
/// the line's end.
fn is_marker(b: &[u8], p: usize) -> bool {
    let n = b.len();
    match b[p] {
        b'-' | b'*' | b'+' => p + 1 == n || is_ws(b[p + 1]),
        b'0'..=b'9' => {
            let mut q = p;
            while q < n && b[q].is_ascii_digit() {
                q += 1;
            }
            q < n && matches!(b[q], b'.' | b')') && (q + 1 == n || is_ws(b[q + 1]))
        }
        _ => false,
    }
}

/// A delimiter run's flanks, from the bytes around it (a line's end a blank): left-flanking,
/// right-flanking.
#[inline(always)]
fn flanks(prev: u8, next: u8) -> (bool, bool) {
    let ws = |c: u8| is_ws(c);
    let punct = |c: u8| c.is_ascii_punctuation();
    let left = !ws(next) && (!punct(next) || ws(prev) || punct(prev));
    let right = !ws(prev) && (!punct(prev) || ws(next) || punct(next));
    (left, right)
}

/// The first place at or past `p` where `pair` stands.
fn find2(b: &[u8], p: usize, pair: [u8; 2]) -> Option<usize> {
    let mut i = p;
    while i + 1 < b.len() {
        let j = i + b[i..b.len() - 1].iter().position(|&x| x == pair[0])?;
        if b[j + 1] == pair[1] {
            return Some(j);
        }
        i = j + 1;
    }
    None
}

/// A comment's `-->` at or past `p`.
fn find_close(b: &[u8], p: usize) -> Option<usize> {
    let mut i = p;
    while let Some(j) = find2(b, i, *b"--") {
        if b.get(j + 2) == Some(&b'>') {
            return Some(j);
        }
        i = j + 1;
    }
    None
}

#[cfg(test)]
mod tests;
