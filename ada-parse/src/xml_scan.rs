//! XML by a scanner of ada's own (X-020, 2026-11-05): where the line is, as one mode and a few
//! counters, in place of a stack of bat's 68 contexts, and each span's kind held as the line
//! goes rather than worked out from the stack at each push and pop. Text runs to the next `<`
//! or `&`; a tag's plain names, its attributes and their strings take one step each; the
//! doctype, its internal subset and the declarations in it (`ENTITY`, `ELEMENT`, `ATTLIST`,
//! `NOTATION`, conditional sections) are modes of their own, read in the order bat's grammar
//! reads them, as are comments, CDATA and processing instructions. The matchers in `xml.rs`
//! read what isn't plain: a namespace, a bad name, a reference.
//!
//! A span's kind is fixed ([`scopes`]): a doctype's, an internal subset's and a conditional
//! section's meta scopes as a set, one of a tag's, a comment's, a declaration's or a group's,
//! a string's, and a leaf, as when the scanner was bat's grammar compiled by hand (2026-10-31),
//! so the themes color it as they did; its readings are held by `tests/readings.rs`' snapshots
//! (X-116). In a tag, a byte that starts
//! nothing (a stray `=`, `/` or `?`) is passed as the tag's own, where bat's grammar searched for
//! the leftmost of its patterns, which reads the same wherever it was tried (the hand-picked
//! blocks, the corpus, 36,000 lines of token soup). Nesting has no limit of syntect's.
//!
//! The state between lines is one word: the mode, where a string returns to, the string's
//! quote, the meta flags, the markup's meta scope, the conditional sections and an element's
//! groups open.

use crate::emit::{Push, Runs, Sink};
use crate::matcher::{Atom, Line, M, first};
use crate::prim::find_any;
use crate::xml::{
    ATOMS, attr_default_at, attr_type_at, attribute_at, boundary, char_ref_at, declaration_at,
    doctype_at, dtd_attr_name_at, dtd_name_at, is_name, is_name_start, is_name2, is_name2_start,
    marked_name_at, placeholder_at, preprocessor_other_at, preprocessor_xml_at, qname, s,
    unknown_at,
};
use crate::{Out, State, is_space};

/// The meta scopes a stack can hold below the innermost declaration's: a doctype's, an
/// internal subset's, a conditional section's. A kind's outer scopes are these flags and one
/// of [`INNERS`].
const FLAGS: [&[Atom]; 3] = [
    &[s::DOCTYPE_META_S],
    &[s::BRACKETS, s::INTERNAL_SUBSET],
    &[s::SUBSET_META],
];
const DOCTYPE: usize = 1;
const SUBSET: usize = 2;
const SECTION: usize = 4;

/// The meta scopes of what the innermost markup is.
const INNERS: [&[Atom]; 14] = [
    &[],
    &[s::TAG_META],
    &[s::PREPROCESSOR_META],
    &[s::COMMENT_BLOCK],
    &[s::CDATA_META],
    &[s::UNKNOWN_META],
    &[s::ATTLIST_META],
    &[s::ATTLIST_META, s::GROUP_ENUMERATED],
    &[s::GROUP_ENUMERATED],
    &[s::ELEMENT_META],
    &[s::ELEMENT_META, s::GROUP],
    &[s::GROUP],
    &[s::ENTITY_META],
    &[s::NOTATION_META],
];
mod inner {
    pub const NONE: usize = 0;
    pub const TAG: usize = 1;
    pub const PREPROCESSOR: usize = 2;
    pub const COMMENT: usize = 3;
    pub const CDATA: usize = 4;
    pub const UNKNOWN: usize = 5;
    /// An enumeration in an attribute list is one past it, [`super::ENUMERATION`]; 8 is an
    /// enumeration alone, which the scanner no longer gives.
    pub const ATTLIST: usize = 6;
    /// A group in an element's declaration is one past it, [`super::ELEMENT_GROUP`]; 11 is a
    /// group alone, which the scanner no longer gives.
    pub const ELEMENT: usize = 9;
    pub const ENTITY: usize = 12;
    pub const NOTATION: usize = 13;
}

/// A string's meta scope, if the innermost context is in one.
const STRINGS: [&[Atom]; 3] = [&[], &[s::DOUBLE], &[s::SINGLE]];

/// The leaves: a token's scopes under the outer ones.
static LEAVES: [&[Atom]; 70] = [
    &[],
    &[s::TAG_BEGIN],
    &[s::TAG_END],
    &[s::BAD_TAG_END],
    &[s::TAG_NS],
    &[s::TAG_NAME, s::NS_SEP],
    &[s::TAG_LOCAL],
    &[s::TAG_NAME],
    &[s::BAD_TAG_NAME],
    &[s::MISSING_TAG_END],
    &[s::UNEXPECTED_ATTR],
    &[s::ATTR_NS],
    &[s::ATTR_NAME, s::NS_SEP],
    &[s::ATTR_LOCAL],
    &[s::BAD_ATTR_NAME],
    &[s::KEY_VALUE],
    &[s::BAD_ATTR_VALUE],
    &[s::STR_BEGIN],
    &[s::STR_END],
    &[s::ENTITY_HEX],
    &[s::ENTITY_HEX, s::ENTITY_MARK],
    &[s::ENTITY_HEX, s::ENTITY_END],
    &[s::ENTITY_DEC],
    &[s::ENTITY_DEC, s::ENTITY_MARK],
    &[s::ENTITY_DEC, s::ENTITY_END],
    &[s::ENTITY_NAMED],
    &[s::ENTITY_NAMED, s::ENTITY_MARK],
    &[s::ENTITY_NAMED, s::ENTITY_END],
    &[s::BAD_AMPERSAND],
    &[s::MISSING_ENTITY],
    &[s::COMMENT_BEGIN],
    &[s::COMMENT_END],
    &[s::DOUBLE_HYPHEN],
    &[s::CDATA_KW],
    &[s::CDATA_TEXT],
    &[s::DOCTYPE_KW],
    &[s::ROOT_NS],
    &[s::ROOT, s::NS_SEP],
    &[s::ROOT_LOCAL],
    &[s::EXTERNAL],
    &[s::BRACKETS_BEGIN],
    &[s::BRACKETS_END],
    &[s::ENTITY_KW],
    &[s::ELEMENT_KW],
    &[s::ATTLIST_KW],
    &[s::NOTATION_KW],
    &[s::ENTITY_MARK],
    &[s::ENTITY_NAME],
    &[s::ELEMENT_NAME],
    &[s::NOTATION_NAME],
    &[s::SUBSET_NAME],
    &[s::PARAMETER],
    &[s::PARAMETER, s::PARAMETER_MARK],
    &[s::PARAMETER, s::PARAMETER_END],
    &[s::BAD_IDENTIFIER],
    &[s::NDATA],
    &[s::UNQUOTED],
    &[s::CONSTANT],
    &[s::GROUP_BEGIN],
    &[s::GROUP_END],
    &[s::OPERATOR],
    &[s::SEPARATOR],
    &[s::LOGICAL_SEP],
    &[s::PLACEHOLDER],
    &[s::PLACEHOLDER, s::CONSTANT_MARK],
    &[s::ATTR_TYPE],
    &[s::ATTR_DEFAULT],
    &[s::ATTR_DEFAULT, s::STORAGE_MARK],
    &[s::ATTR_NAME],
    &[s::UNEXPECTED],
];

mod leaf {
    pub const NONE: u16 = 0;
    pub const TAG_BEGIN: u16 = 1;
    pub const TAG_END: u16 = 2;
    pub const BAD_TAG_END: u16 = 3;
    /// A tag's name: its namespace, the `:`, its local name, then a bad name (as [`qname`]'s
    /// groups 2 to 6 have them, the `:`'s first).
    pub const TAG_NS: u16 = 4;
    pub const TAG_NAME: u16 = 7;
    pub const BAD_TAG_NAME: u16 = 8;
    pub const MISSING_TAG_END: u16 = 9;
    pub const UNEXPECTED_ATTR: u16 = 10;
    /// An attribute's name, as [`TAG_NS`] (its bad names one leaf).
    pub const ATTR_NS: u16 = 11;
    pub const BAD_ATTR_NAME: u16 = 14;
    pub const KEY_VALUE: u16 = 15;
    pub const BAD_ATTR_VALUE: u16 = 16;
    pub const STR_BEGIN: u16 = 17;
    pub const STR_END: u16 = 18;
    /// A reference's text, its `&` or `&#` and its `;`: hexadecimal, decimal, named.
    pub const ENTITY_HEX: u16 = 19;
    pub const ENTITY_DEC: u16 = 22;
    pub const ENTITY_NAMED: u16 = 25;
    pub const BAD_AMPERSAND: u16 = 28;
    pub const MISSING_ENTITY: u16 = 29;
    pub const COMMENT_BEGIN: u16 = 30;
    pub const COMMENT_END: u16 = 31;
    pub const DOUBLE_HYPHEN: u16 = 32;
    pub const CDATA_KW: u16 = 33;
    pub const CDATA_TEXT: u16 = 34;
    pub const DOCTYPE_KW: u16 = 35;
    pub const ROOT_NS: u16 = 36;
    pub const EXTERNAL: u16 = 39;
    pub const BRACKETS_BEGIN: u16 = 40;
    pub const BRACKETS_END: u16 = 41;
    pub const ENTITY_KW: u16 = 42;
    pub const ELEMENT_KW: u16 = 43;
    pub const ATTLIST_KW: u16 = 44;
    pub const NOTATION_KW: u16 = 45;
    pub const ENTITY_MARK: u16 = 46;
    pub const ENTITY_NAME: u16 = 47;
    pub const ELEMENT_NAME: u16 = 48;
    pub const NOTATION_NAME: u16 = 49;
    pub const SUBSET_NAME: u16 = 50;
    /// A parameter entity's reference, as [`ENTITY_HEX`].
    pub const PARAMETER: u16 = 51;
    pub const BAD_IDENTIFIER: u16 = 54;
    pub const NDATA: u16 = 55;
    pub const UNQUOTED: u16 = 56;
    pub const CONSTANT: u16 = 57;
    pub const GROUP_BEGIN: u16 = 58;
    pub const GROUP_END: u16 = 59;
    pub const OPERATOR: u16 = 60;
    pub const SEPARATOR: u16 = 61;
    pub const LOGICAL_SEP: u16 = 62;
    /// `#PCDATA` and its `#`.
    pub const PLACEHOLDER: u16 = 63;
    pub const ATTR_TYPE: u16 = 65;
    /// `#REQUIRED` and its `#`.
    pub const ATTR_DEFAULT: u16 = 66;
    pub const ATTR_NAME: u16 = 68;
    pub const UNEXPECTED: u16 = 69;
}

const NLEAVES: u16 = LEAVES.len() as u16;

/// A kind: `(outer * 3 + string) * NLEAVES + leaf`, with `outer` the flags times
/// `INNERS.len()` plus the inner markup, so that the kinds most text has are small numbers.
fn kind(flags: usize, inner: usize, string: usize, leaf: u16) -> u16 {
    let outer = flags * INNERS.len() + inner;
    (outer * 3 + string) as u16 * NLEAVES + leaf
}

/// The scopes under `text.xml` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    let leaf = kind % NLEAVES;
    let rest = usize::from(kind / NLEAVES);
    let (outer, string) = (rest / 3, rest % 3);
    let (flags, inner) = (outer / INNERS.len(), outer % INNERS.len());
    let flags = (0..3).filter(|i| flags >> i & 1 == 1).map(|i| FLAGS[i]);
    flags
        .chain(INNERS.get(inner).copied())
        .chain([STRINGS[string], LEAVES[usize::from(leaf)]])
        .flatten()
        .map(|&a| ATOMS[usize::from(a)])
        .collect()
}

/// Where a line is.
mod mode {
    /// The document's text.
    pub const TEXT: u8 = 0;
    /// In a tag or an `<?xml …?>` (the markup tells): an attribute's name, or the tag's end.
    pub const TAG: u8 = 1;
    /// After an attribute's name: its `=`.
    pub const TAG_EQ: u8 = 2;
    /// After an attribute's `=`: its value.
    pub const TAG_VALUE: u8 = 3;
    /// In a quoted string, which returns to [`super::Scan::ret`].
    pub const STRING: u8 = 4;
    /// A close tag's, the doctype's or a declaration's end, after what it holds.
    pub const END: u8 = 5;
    pub const COMMENT: u8 = 6;
    pub const CDATA: u8 = 7;
    /// A processing instruction but `<?xml`, to its `?>`.
    pub const PI: u8 = 8;
    /// The doctype's root element.
    pub const ROOT: u8 = 9;
    /// `PUBLIC` or `SYSTEM`, in the doctype, an entity's or a notation's declaration.
    pub const TYPE: u8 = 10;
    /// The doctype's or a notation's quoted strings.
    pub const QUOTED: u8 = 11;
    /// The doctype's internal subset's `[`, or a conditional section's.
    pub const BRACKETS: u8 = 12;
    /// The internal subset, or a conditional section's.
    pub const SUBSET: u8 = 13;
    /// A conditional section's end, after its `]`.
    pub const SECTION_END: u8 = 14;
    /// A declaration's name, or a conditional section's keyword.
    pub const NAME: u8 = 15;
    /// An entity's declaration: its `%`.
    pub const PERCENT: u8 = 16;
    /// An entity's value, or `NDATA`.
    pub const ENTITY_VALUE: u8 = 17;
    /// After `NDATA`: the notation's name.
    pub const NDATA_NAME: u8 = 18;
    /// An element's content: `EMPTY`, `ANY`, its group.
    pub const ELEMENT_CONTENT: u8 = 19;
    /// In an element's group or an attribute's enumeration (the markup tells).
    pub const GROUP: u8 = 20;
    /// After a group's `)`: its quantifier.
    pub const GROUP_AFTER: u8 = 21;
    /// An attribute list's attributes: their names, types, defaults.
    pub const ATTRIBUTES: u8 = 22;
}

/// What a group's markup is: an element's declaration and a group, an attribute list's and an
/// enumeration.
const ELEMENT_GROUP: usize = inner::ELEMENT + 1;
const ENUMERATION: usize = inner::ATTLIST + 1;

/// `[*?+]`
fn is_quantifier(c: u8) -> bool {
    matches!(c, b'*' | b'?' | b'+')
}

/// Where the spaces from `p` end.
#[inline(always)]
fn spaces(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && is_space(b[p]) {
        p += 1;
    }
    p
}

/// The first byte at or after `from` that `set` takes.
fn find(b: &[u8], from: usize, set: impl Fn(u8) -> bool) -> Option<usize> {
    b.get(from..)?
        .iter()
        .position(|&c| set(c))
        .map(|i| from + i)
}

/// Lex `b`, a line without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let w = lex(b, state.word(), out, &mut Push);
    state.set_word(w);
}

/// As [`line`], the line's runs written to `k` (X-072): a block's lines on one sink, as the
/// line-format engine's, since all a line hands the next is the word.
#[inline(never)]
pub(crate) fn line_runs(b: &[u8], state: &mut State, out: &mut Out, k: &mut Runs) {
    let w = lex(b, state.word(), out, k);
    state.set_word(w);
}

/// `b` lexed from `w`, the word the line before left: the word it leaves. The scan a local, its fields in registers.
// one copy a sink, for `line` and the block: with two, `run`'s callees were no longer inlined
// into it (XML +2.6% instructions). The sink by reference, as the doctype's paths are out of
// line (X-069's lesson).
#[inline(never)]
fn lex<K: Sink>(b: &[u8], w: u64, out: &mut Out, k: &mut K) -> u64 {
    let mut x = Scan {
        b,
        mode: 0,
        ret: 0,
        string: 0,
        flags: 0,
        inner: 0,
        sections: 0,
        depth: 0,
        base: 0,
        text: 0,
        at: 0,
        out,
        k,
    };
    x.unpack(w);
    x.rebase();
    x.run(0);
    x.fill(b.len());
    x.pack()
}

/// A block of XML ([`crate::Carry`], X-067): each line on a scan of its own from the word the
/// one before left, `state` left as the last line leaves it. A local scan keeps its fields in
/// registers, which one kept across the lines behind a reference lost (+3% instructions).
pub fn block(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    let mut out = Out::new(spans);
    let mut c = Carried {
        out: &mut out,
        w: state.word(),
    };
    let rest = pass(&mut c);
    state.set_word(c.w);
    rest
}

/// What XML's lines hand on across a block: the word.
struct Carried<'o, 'c> {
    out: &'o mut Out<'c>,
    w: u64,
}

impl crate::Carry for Carried<'_, '_> {
    fn next(&mut self, b: &[u8]) -> &[crate::Span] {
        self.out.0.clear();
        self.w = lex(b, self.w, self.out, &mut Push);
        self.out.0
    }
}

impl<K: Sink> Scan<'_, '_, '_, K> {
    /// The scan's state from a state's word.
    #[inline(always)]
    fn unpack(&mut self, w: u64) {
        self.mode = (w & 31) as u8;
        self.ret = (w >> 5 & 31) as u8;
        self.string = (w >> 10 & 3) as usize;
        self.flags = (w >> 12 & 7) as usize;
        self.inner = (w >> 15 & 15) as usize;
        self.sections = (w >> 19) as u8;
        self.depth = (w >> 27) as u8;
    }

    /// The state's word the scan's line leaves.
    #[inline(always)]
    fn pack(&self) -> u64 {
        u64::from(self.mode)
            | u64::from(self.ret) << 5
            | (self.string as u64) << 10
            | (self.flags as u64) << 12
            | (self.inner as u64) << 15
            | u64::from(self.sections) << 19
            | u64::from(self.depth) << 27
    }
}

struct Scan<'b, 'o, 'c, K: Sink> {
    b: &'b [u8],
    mode: u8,
    /// Where a string returns to.
    ret: u8,
    /// The string open: none, double-quoted, single-quoted.
    string: usize,
    /// The meta flags: in the doctype, its internal subset, a conditional section.
    flags: usize,
    /// The markup's meta scope ([`INNERS`]).
    inner: usize,
    /// The conditional sections open.
    sections: u8,
    /// An element's groups open.
    depth: u8,
    /// The kind of the scopes so far, with no leaf.
    base: u16,
    /// The kind of text no token takes: [`Scan::base`], in CDATA its text's.
    text: u16,
    /// Where the spans so far end.
    at: usize,
    out: &'o mut Out<'c>,
    k: &'o mut K,
}

impl<K: Sink> Scan<'_, '_, '_, K> {
    /// The kinds, after the flags, the markup, the string or the mode changed.
    fn rebase(&mut self) {
        self.base = kind(self.flags, self.inner, self.string, 0);
        self.text = match self.mode {
            mode::CDATA => self.base + leaf::CDATA_TEXT,
            _ => self.base,
        };
    }

    /// Bytes `s..e` are `leaf` in the scopes so far; the text before them, from where the
    /// spans end, is the scopes' text.
    #[inline(always)]
    fn put(&mut self, s: usize, e: usize, leaf: u16) {
        self.fill(s);
        let e = e.min(self.b.len());
        let kind = self.base + leaf;
        if kind != 0 && e > s {
            self.k.merge(&mut self.out, s, e, kind);
        }
        self.at = self.at.max(e);
    }

    /// [`Scan::put`] out of line, for the doctype's and its subset's rare paths: inlined there,
    /// they took 17 KB.
    #[inline(never)]
    fn put_rare(&mut self, s: usize, e: usize, leaf: u16) {
        self.put(s, e, leaf);
    }

    /// The text from where the spans end to `pos` is the scopes' text.
    #[inline(always)]
    fn fill(&mut self, pos: usize) {
        let pos = pos.min(self.b.len());
        if pos > self.at {
            if self.text != 0 {
                self.k.merge(&mut self.out, self.at, pos, self.text);
            }
            self.at = pos;
        }
    }

    /// From `at`, markup whose meta scope is `inner`, in `mode`.
    fn markup(&mut self, at: usize, inner: usize, mode: u8) {
        self.fill(at);
        self.inner = inner;
        self.mode = mode;
        self.rebase();
    }

    /// Where markup that ends returns: the internal subset, or the document's text.
    fn back(&self) -> u8 {
        match self.flags & SUBSET {
            0 => mode::TEXT,
            _ => mode::SUBSET,
        }
    }

    /// The markup ends at `at`.
    fn leave(&mut self, at: usize) {
        let back = self.back();
        self.markup(at, inner::NONE, back);
    }

    /// A match `m`: its groups in `caps`, as `(group, leaf)`, the rest of it `sc`.
    fn emit(&mut self, m: &M, sc: u16, caps: &[(u8, u16)]) {
        // the matchers set a match's groups from left to right, and none nests in another
        let mut at = m.s as usize;
        for (g, gs, ge) in m.groups() {
            let Some(&(_, leaf)) = caps.iter().find(|c| c.0 == g) else {
                continue;
            };
            if ge > gs {
                self.put(at, gs, sc);
                self.put(gs, ge, leaf);
                at = ge;
            }
        }
        self.put(at, m.e as usize, sc);
    }

    /// A reference (`&…;`, `%…;`): its text `leaf`, its mark and its `;` the two after.
    fn reference(&mut self, m: &M, leaf: u16) -> usize {
        self.emit(m, leaf, &[(1, leaf + 1), (2, leaf + 2)]);
        m.e as usize
    }

    /// A quoted string at `q`, its quote: it returns to `ret`.
    fn string_at(&mut self, q: usize, ret: u8) {
        self.fill(q);
        self.string = match self.b[q] {
            b'"' => 1,
            _ => 2,
        };
        self.ret = ret;
        self.mode = mode::STRING;
        self.rebase();
        self.put(q, q + 1, leaf::STR_BEGIN);
    }

    /// `\s?(?=[\]<])`'s pop at `q`, the first byte from `p` that isn't a space: where the
    /// next mode starts.
    fn else_pop(&self, p: usize, q: usize) -> usize {
        match q > p && matches!(self.b[q], b']' | b'<') {
            true => q - 1,
            false => q,
        }
    }

    /// A quoted string at `q`, the first byte from `p` that isn't a space, returning to `ret`;
    /// else `next` from where it pops.
    fn quoted_or_pop(&mut self, p: usize, q: usize, ret: u8, next: u8) -> usize {
        let at = self.else_pop(p, q);
        if at == q && matches!(self.b[q], b'"' | b'\'') {
            self.string_at(q, ret);
            return q + 1;
        }
        self.mode = next;
        at
    }

    /// The line from `p`, in the mode it is in.
    #[inline(always)]
    fn run(&mut self, mut p: usize) {
        let b = self.b;
        let nl = b.len();
        loop {
            match self.mode {
                mode::TEXT => {
                    let k = find_any(b, p, *b"<&");
                    if k >= nl {
                        return;
                    }
                    p = match b[k] {
                        b'&' => self.ampersand(k),
                        _ => self.open(k),
                    };
                }
                mode::TAG => match self.tag(p) {
                    Some(e) => p = e,
                    None => return,
                },
                mode::TAG_EQ => {
                    let q = spaces(b, p);
                    if q >= nl {
                        return;
                    }
                    if b[q] == b'=' {
                        self.put(q, q + 1, leaf::KEY_VALUE);
                        self.mode = mode::TAG_VALUE;
                        p = q + 1;
                    } else {
                        self.mode = mode::TAG;
                        p = if q > p && b[q] == b'<' { q - 1 } else { q };
                    }
                }
                mode::TAG_VALUE => {
                    let q = spaces(b, p);
                    if q >= nl {
                        return;
                    }
                    let c = b[q];
                    let stop = |c: u8| matches!(c, b'?' | b'/' | b'<' | b'>');
                    if q > p && c == b'<' {
                        self.mode = mode::TAG;
                        p = q - 1;
                    } else if c == b'"' || c == b'\'' {
                        self.string_at(q, mode::TAG);
                        p = q + 1;
                    } else if !stop(c) {
                        let e = q + b[q..]
                            .iter()
                            .take_while(|&&c| !stop(c) && !is_space(c))
                            .count();
                        self.put(q, e, leaf::BAD_ATTR_VALUE);
                        self.mode = mode::TAG;
                        p = e;
                    } else {
                        self.mode = mode::TAG;
                        p = q;
                    }
                }
                mode::STRING => {
                    let quote = if self.string == 1 { b'"' } else { b'\'' };
                    let k = find_any(b, p, [quote, b'<', b'&']);
                    if k >= nl {
                        return;
                    }
                    p = match b[k] {
                        b'<' => {
                            let s = if k > p && is_space(b[k - 1]) {
                                k - 1
                            } else {
                                k
                            };
                            self.put(s, k, leaf::MISSING_TAG_END);
                            self.end_string(k);
                            k
                        }
                        b'&' => self.ampersand(k),
                        _ => {
                            self.put(k, k + 1, leaf::STR_END);
                            self.end_string(k + 1);
                            k + 1
                        }
                    };
                }
                mode::END => match self.end(p) {
                    Some(e) => p = e,
                    None => return,
                },
                mode::COMMENT => {
                    let mut i = p;
                    loop {
                        let Some(k) = first(&Line { b }, i, b'-') else {
                            return;
                        };
                        if b.get(k + 1) == Some(&b'-') {
                            if b.get(k + 2) == Some(&b'>') {
                                self.put(k, k + 3, leaf::COMMENT_END);
                                self.leave(k + 3);
                                p = k + 3;
                            } else {
                                let e = k + b[k..].iter().take_while(|&&c| c == b'-').count();
                                self.put(k, e, leaf::DOUBLE_HYPHEN);
                                p = e;
                            }
                            break;
                        }
                        i = k + 1;
                    }
                }
                mode::CDATA | mode::PI => {
                    let close: &[u8] = match self.mode {
                        mode::CDATA => b"]]>",
                        _ => b"?>",
                    };
                    let Some(m) = Line { b }.lit(p, close) else {
                        return;
                    };
                    let e = m.s as usize + close.len();
                    self.put(m.s as usize, e, leaf::TAG_END);
                    self.leave(e);
                    p = e;
                }
                _ => match self.dtd(p) {
                    Some(e) => p = e,
                    None => return,
                },
            }
        }
    }

    /// The doctype and its internal subset, a step: where the line goes on, or none at its end.
    /// Out of line, as they are rare: in `line`, they took XML's code from 31 to 54 KB.
    #[inline(never)]
    fn dtd(&mut self, mut p: usize) -> Option<usize> {
        let b = self.b;
        let nl = b.len();
        loop {
            match self.mode {
                mode::ROOT => {
                    let q = spaces(b, p);
                    if q >= nl {
                        return None;
                    }
                    self.mode = mode::TYPE;
                    if q > p && matches!(b[q], b']' | b'<') {
                        p = q - 1;
                        continue;
                    }
                    p = match qname(&Line { b }, q, M::new(q, q), 1) {
                        Some(m) => {
                            let caps = [
                                (1, leaf::ROOT_NS),
                                (2, leaf::BAD_TAG_NAME),
                                (3, leaf::ROOT_NS + 1),
                                (4, leaf::ROOT_NS + 2),
                                (5, leaf::BAD_TAG_NAME),
                            ];
                            self.emit(&m, leaf::NONE, &caps);
                            m.e as usize
                        }
                        None => q,
                    };
                }
                mode::TYPE => {
                    let q = spaces(b, p);
                    if q >= nl {
                        return None;
                    }
                    let at = self.else_pop(p, q);
                    let rest = &b[q..];
                    p = if at == q
                        && (rest.starts_with(b"PUBLIC") || rest.starts_with(b"SYSTEM"))
                        && boundary(&Line { b }, q + 6)
                    {
                        self.put_rare(q, q + 6, leaf::EXTERNAL);
                        q + 6
                    } else {
                        at
                    };
                    self.mode = match self.inner {
                        inner::ENTITY => mode::ENTITY_VALUE,
                        _ => mode::QUOTED,
                    };
                }
                mode::QUOTED => {
                    let q = spaces(b, p);
                    if q >= nl {
                        return None;
                    }
                    let next = match self.inner {
                        inner::NONE => mode::BRACKETS,
                        _ => mode::END,
                    };
                    p = self.quoted_or_pop(p, q, mode::QUOTED, next);
                }
                mode::BRACKETS => {
                    let q = spaces(b, p);
                    if q >= nl {
                        return None;
                    }
                    let at = self.else_pop(p, q);
                    if at == q && b[q] == b'[' {
                        self.fill(q);
                        self.flags |= SUBSET;
                        self.mode = mode::SUBSET;
                        self.rebase();
                        self.put_rare(q, q + 1, leaf::BRACKETS_BEGIN);
                        p = q + 1;
                    } else {
                        self.mode = match self.flags & SUBSET {
                            0 => mode::END,
                            _ => mode::SECTION_END,
                        };
                        p = at;
                    }
                }
                mode::SUBSET => p = self.subset(p)?,
                mode::SECTION_END => {
                    let k = find_any(b, p, *b"><");
                    if k >= nl {
                        return None;
                    }
                    if b[k] == b'<' {
                        let s = if k > p && is_space(b[k - 1]) {
                            k - 1
                        } else {
                            k
                        };
                        self.put_rare(s, k, leaf::MISSING_TAG_END);
                        self.end_section(k);
                        p = k;
                    } else {
                        let (s, end) = match (k > p).then(|| b[k - 1]) {
                            Some(b']') => (k - 1, leaf::TAG_END),
                            Some(b'/' | b'?') => (k - 1, leaf::BAD_TAG_END),
                            _ => (k, leaf::BAD_TAG_END),
                        };
                        self.put_rare(s, k + 1, end);
                        self.end_section(k + 1);
                        p = k + 1;
                    }
                }
                mode::NAME => {
                    let q = spaces(b, p);
                    if q >= nl {
                        return None;
                    }
                    p = self.name(p, q);
                }
                mode::PERCENT => {
                    let q = spaces(b, p);
                    if q >= nl {
                        return None;
                    }
                    let at = self.else_pop(p, q);
                    p = if at == q && b[q] == b'%' {
                        self.put_rare(q, q + 1, leaf::ENTITY_MARK);
                        q + 1
                    } else {
                        at
                    };
                    self.mode = mode::NAME;
                }
                mode::ENTITY_VALUE => {
                    let q = spaces(b, p);
                    if q >= nl {
                        return None;
                    }
                    if self.else_pop(p, q) == q
                        && b[q..].starts_with(b"NDATA")
                        && boundary(&Line { b }, q + 5)
                    {
                        self.put_rare(q, q + 5, leaf::NDATA);
                        self.mode = mode::NDATA_NAME;
                        p = q + 5;
                        continue;
                    }
                    p = self.quoted_or_pop(p, q, mode::ENTITY_VALUE, mode::END);
                }
                mode::NDATA_NAME => {
                    let q = spaces(b, p);
                    if q >= nl {
                        return None;
                    }
                    let at = self.else_pop(p, q);
                    if at == q && is_name2_start(b[q]) {
                        let e = q + 1 + b[q + 1..].iter().take_while(|&&c| is_name2(c)).count();
                        self.put_rare(q, e, leaf::UNQUOTED);
                        p = e;
                    } else {
                        self.mode = mode::END;
                        p = at;
                    }
                }
                mode::ELEMENT_CONTENT => {
                    let q = spaces(b, p);
                    if q >= nl {
                        return None;
                    }
                    p = self.element_content(p, q);
                }
                mode::GROUP => p = self.group(p)?,
                mode::GROUP_AFTER => {
                    let q = spaces(b, p);
                    if q >= nl {
                        return None;
                    }
                    let at = self.else_pop(p, q);
                    p = if at == q && is_quantifier(b[q]) {
                        self.put_rare(q, q + 1, leaf::OPERATOR);
                        q + 1
                    } else {
                        at
                    };
                    self.mode = match self.depth {
                        0 => mode::END,
                        _ => mode::GROUP,
                    };
                }
                mode::ATTRIBUTES => {
                    let q = spaces(b, p);
                    if q >= nl {
                        return None;
                    }
                    p = self.attributes(p, q);
                }
                _ => {}
            }
            return Some(p);
        }
    }

    /// The string ends at `at`.
    fn end_string(&mut self, at: usize) {
        self.fill(at);
        self.string = 0;
        self.mode = self.ret;
        self.rebase();
    }

    /// A conditional section ends at `at`.
    fn end_section(&mut self, at: usize) {
        self.fill(at);
        self.sections = self.sections.saturating_sub(1);
        if self.sections == 0 {
            self.flags &= !SECTION;
        }
        self.mode = mode::SUBSET;
        self.rebase();
    }

    /// A `<` at `k` in the document's text: the markup it opens, else a bad `<`.
    fn open(&mut self, k: usize) -> usize {
        let b = self.b;
        let l = Line { b };
        match b.get(k + 1) {
            Some(b'?') => {
                if let Some(e) = self.preprocessor(k) {
                    return e;
                }
            }
            Some(b'!') => {
                if let Some(m) = doctype_at(&l, k) {
                    self.fill(k);
                    self.flags |= DOCTYPE;
                    self.markup(k, inner::NONE, mode::ROOT);
                    let caps = [
                        (1, leaf::TAG_BEGIN),
                        (2, leaf::DOCTYPE_KW),
                        (3, leaf::BAD_TAG_NAME),
                    ];
                    self.emit(&m, leaf::NONE, &caps);
                    return m.e as usize;
                }
                if let Some(e) = self.comment_or_cdata(k) {
                    return e;
                }
            }
            Some(b'/') => {
                if let Some(e) = self.plain_tag(k, 2, mode::END) {
                    return e;
                }
                if let Some(m) = qname(&l, k + 2, M::new(k, k).g(1, k, k + 2), 2) {
                    self.markup(k, inner::TAG, mode::END);
                    return self.tag_name(&m);
                }
            }
            _ => {
                if let Some(e) = self.plain_tag(k, 1, mode::TAG) {
                    return e;
                }
                if let Some(m) = qname(&l, k + 1, M::new(k, k).g(1, k, k + 1), 2) {
                    self.markup(k, inner::TAG, mode::TAG);
                    return self.tag_name(&m);
                }
            }
        }
        self.put(k, k + 1, leaf::MISSING_ENTITY);
        k + 1
    }

    /// A tag at `k` whose `<` or `</` (`open` bytes) a plain local name follows, as [`qname`]
    /// reads it: no namespace, and one of `[/<>\s]` after it.
    #[inline(always)]
    fn plain_tag(&mut self, k: usize, open: usize, mode: u8) -> Option<usize> {
        let b = self.b;
        let q = k + open;
        if !b.get(q).is_some_and(|&c| is_name_start(c)) {
            return None;
        }
        let e = q + 1 + b[q + 1..].iter().take_while(|&&c| is_name(c)).count();
        if !b
            .get(e)
            .is_none_or(|&c| matches!(c, b'/' | b'<' | b'>') || is_space(c))
        {
            return None;
        }
        self.markup(k, inner::TAG, mode);
        self.put(k, q, leaf::TAG_BEGIN);
        self.put(q, e, leaf::TAG_NS + 2);
        Some(e)
    }

    /// A tag's `<` or `</` and name, from [`qname`]'s groups.
    fn tag_name(&mut self, m: &M) -> usize {
        let caps = [
            (1, leaf::TAG_BEGIN),
            (2, leaf::TAG_NS),
            (3, leaf::BAD_TAG_NAME),
            (4, leaf::TAG_NS + 1),
            (5, leaf::TAG_NS + 2),
            (6, leaf::BAD_TAG_NAME),
        ];
        self.emit(m, leaf::NONE, &caps);
        m.e as usize
    }

    /// `<!--` or `<![CDATA[` at `k`.
    fn comment_or_cdata(&mut self, k: usize) -> Option<usize> {
        let rest = &self.b[k..];
        if rest.starts_with(b"<!--") {
            self.markup(k, inner::COMMENT, mode::COMMENT);
            self.put(k, k + 4, leaf::COMMENT_BEGIN);
            return Some(k + 4);
        }
        if rest.starts_with(b"<![CDATA[") {
            self.markup(k, inner::CDATA, mode::CDATA);
            self.put(k, k + 3, leaf::TAG_BEGIN);
            self.put(k + 3, k + 8, leaf::CDATA_KW);
            self.put(k + 8, k + 9, leaf::TAG_BEGIN);
            return Some(k + 9);
        }
        None
    }

    /// A processing instruction at `k`, where a `<?` is.
    fn preprocessor(&mut self, k: usize) -> Option<usize> {
        let l = Line { b: self.b };
        if let Some(m) = preprocessor_xml_at(&l, k) {
            self.markup(k, inner::PREPROCESSOR, mode::TAG);
            let caps = [
                (1, leaf::TAG_BEGIN),
                (2, leaf::TAG_NAME),
                (3, leaf::BAD_TAG_NAME),
            ];
            self.emit(&m, leaf::NONE, &caps);
            return Some(m.e as usize);
        }
        let m = preprocessor_other_at(&l, k)?;
        self.markup(k, inner::PREPROCESSOR, mode::PI);
        self.emit(&m, leaf::NONE, &[(1, leaf::TAG_BEGIN), (2, leaf::TAG_NAME)]);
        Some(m.e as usize)
    }

    /// In a tag or an `<?xml …?>`: its end, an attribute, or a missing end.
    #[inline(always)]
    fn tag(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let q = spaces(b, p);
        let &c = b.get(q)?;
        let tag = self.inner == inner::TAG;
        let next = b.get(q + 1).copied();
        if c == b'<' {
            let s = if q > p { q - 1 } else { q };
            self.put(s, q, leaf::MISSING_TAG_END);
            self.leave(q);
            return Some(q);
        }
        if tag && (c == b'>' || (c == b'/' && next == Some(b'>'))) {
            let e = if c == b'>' { q + 1 } else { q + 2 };
            self.put(q, e, leaf::TAG_END);
            self.leave(e);
            return Some(e);
        }
        if c == b'?' && next == Some(b'>') {
            let end = if tag {
                leaf::BAD_TAG_END
            } else {
                leaf::TAG_END
            };
            self.put(q, q + 2, end);
            self.leave(q + 2);
            return Some(q + 2);
        }
        // a plain local name, as `attribute_at` reads it: no namespace, its look-ahead after it
        if is_name_start(c) {
            let e = q + 1 + b[q + 1..].iter().take_while(|&&c| is_name(c)).count();
            let ahead = match b.get(e) {
                None => true,
                Some(&c) => {
                    matches!(c, b'=' | b'<' | b'>')
                        || is_space(c)
                        || (matches!(c, b'/' | b'?') && b.get(e + 1) == Some(&b'>'))
                }
            };
            if ahead {
                self.put(q, e, leaf::ATTR_NS + 2);
                self.mode = mode::TAG_EQ;
                return Some(e);
            }
        }
        if let Some(m) = attribute_at(&Line { b }, q) {
            let caps = [
                (1, leaf::ATTR_NS),
                (2, leaf::BAD_ATTR_NAME),
                (3, leaf::ATTR_NS + 1),
                (4, leaf::ATTR_NS + 2),
                (5, leaf::BAD_ATTR_NAME),
            ];
            self.emit(&m, leaf::NONE, &caps);
            self.mode = mode::TAG_EQ;
            return Some(m.e as usize);
        }
        // a byte that starts nothing: the tag's own
        Some(q + Line { b }.char_len(q).unwrap_or(1))
    }

    /// A close tag's, the doctype's or a declaration's end at the first byte from `p` that
    /// isn't a space; else a byte it doesn't expect.
    fn end(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let q = spaces(b, p);
        let &c = b.get(q)?;
        let tag = self.inner == inner::TAG;
        if c == b'<' || (!tag && c == b']') {
            let s = if q > p { q - 1 } else { q };
            self.put(s, q, leaf::MISSING_TAG_END);
            self.end_markup(q);
            return Some(q);
        }
        if c == b'>' {
            self.put(q, q + 1, leaf::TAG_END);
            self.end_markup(q + 1);
            return Some(q + 1);
        }
        if matches!(c, b'/' | b'?') && b.get(q + 1) == Some(&b'>') {
            self.put(q, q + 2, leaf::BAD_TAG_END);
            self.end_markup(q + 2);
            return Some(q + 2);
        }
        let e = q + Line { b }.char_len(q).unwrap_or(1);
        let unexpected = match tag {
            true => leaf::UNEXPECTED_ATTR,
            false => leaf::UNEXPECTED,
        };
        self.put(q, e, unexpected);
        Some(e)
    }

    /// What [`Scan::end`] ends, ends at `at`: a close tag, the doctype, or a declaration.
    fn end_markup(&mut self, at: usize) {
        if self.inner == inner::NONE {
            self.fill(at);
            self.flags &= !DOCTYPE;
        }
        self.leave(at);
    }

    /// The internal subset or a conditional section's from `p`: a declaration, a comment, a
    /// reference, its `]`.
    fn subset(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let mut i = p;
        loop {
            let k = find_any(b, i, *b"]<#%");
            if k >= b.len() {
                return None;
            }
            match b[k] {
                b']' => {
                    self.put_rare(k, k + 1, leaf::BRACKETS_END);
                    self.fill(k + 1);
                    if self.sections > 0 {
                        self.mode = mode::SECTION_END;
                    } else {
                        self.flags &= !SUBSET;
                        self.mode = mode::END;
                    }
                    self.rebase();
                    return Some(k + 1);
                }
                b'<' => {
                    if let Some(e) = self.declaration(k) {
                        return Some(e);
                    }
                }
                _ => {
                    if let Some(e) = self.entity_parameter(k) {
                        return Some(e);
                    }
                }
            }
            i = k + 1;
        }
    }

    /// A declaration, a comment, CDATA or a processing instruction at `k`, where a `<` is.
    fn declaration(&mut self, k: usize) -> Option<usize> {
        let b = self.b;
        let l = Line { b };
        if b.get(k + 1) == Some(&b'?') {
            return self.preprocessor(k);
        }
        if b.get(k + 1) != Some(&b'!') {
            return None;
        }
        if let Some(e) = self.comment_or_cdata(k) {
            return Some(e);
        }
        const DECLS: [(&[u8], u16, usize, u8); 4] = [
            (b"ENTITY", leaf::ENTITY_KW, inner::ENTITY, mode::PERCENT),
            (b"ELEMENT", leaf::ELEMENT_KW, inner::ELEMENT, mode::NAME),
            (b"ATTLIST", leaf::ATTLIST_KW, inner::ATTLIST, mode::NAME),
            (b"NOTATION", leaf::NOTATION_KW, inner::NOTATION, mode::NAME),
        ];
        for (kw, kw_leaf, inner, mode) in DECLS {
            if let Some(m) = declaration_at(&l, k, kw) {
                self.markup(k, inner, mode);
                self.emit(&m, leaf::NONE, &[(1, leaf::TAG_BEGIN), (2, kw_leaf)]);
                return Some(m.e as usize);
            }
        }
        if b.get(k + 2) == Some(&b'[') {
            self.fill(k);
            self.sections = self.sections.saturating_add(1);
            self.flags |= SECTION;
            self.markup(k, inner::NONE, mode::NAME);
            self.put_rare(k, k + 3, leaf::TAG_BEGIN);
            return Some(k + 3);
        }
        let m = unknown_at(&l, k);
        self.markup(k, inner::UNKNOWN, mode::END);
        self.emit(
            &m,
            leaf::NONE,
            &[(1, leaf::TAG_BEGIN), (2, leaf::BAD_TAG_NAME)],
        );
        Some(m.e as usize)
    }

    /// A declaration's name or a conditional section's keyword at `q`, the first byte from `p`
    /// that isn't a space: where what comes after it starts.
    fn name(&mut self, p: usize, q: usize) -> usize {
        let b = self.b;
        let l = Line { b };
        let (name, next) = match self.inner {
            inner::ENTITY => (leaf::ENTITY_NAME, mode::TYPE),
            inner::ELEMENT => (leaf::ELEMENT_NAME, mode::ELEMENT_CONTENT),
            inner::ATTLIST => (leaf::ELEMENT_NAME, mode::ATTRIBUTES),
            inner::NOTATION => (leaf::NOTATION_NAME, mode::TYPE),
            _ => (leaf::SUBSET_NAME, mode::BRACKETS),
        };
        self.mode = next;
        if q > p && matches!(b[q], b']' | b'<') {
            return q - 1;
        }
        if let Some(m) = dtd_name_at(&l, q) {
            self.put_rare(q, m.e as usize, name);
            return m.e as usize;
        }
        // `dtd-common-name`
        if b[q] == b'%'
            && let Some(m) = marked_name_at(&l, q)
        {
            return self.reference(&m, leaf::PARAMETER);
        }
        let ident = |c: u8| !b"'\"[]()<>".contains(&c) && !is_space(c);
        if ident(b[q]) {
            let e = q + b[q..].iter().take_while(|&&c| ident(c)).count();
            self.put_rare(q, e, leaf::BAD_IDENTIFIER);
            return e;
        }
        q
    }

    /// An element's content at `q`, the first byte from `p` that isn't a space.
    fn element_content(&mut self, p: usize, q: usize) -> usize {
        let b = self.b;
        let l = Line { b };
        if self.else_pop(p, q) == q {
            let rest = &b[q..];
            let word = [&b"EMPTY"[..], b"ANY"]
                .into_iter()
                .find(|w| rest.starts_with(w) && boundary(&l, q + w.len()));
            if let Some(w) = word
                && boundary(&l, q)
            {
                self.put_rare(q, q + w.len(), leaf::CONSTANT);
                self.mode = mode::END;
                return q + w.len();
            }
            if b[q] == b'(' {
                self.depth = 1;
                self.markup(q, ELEMENT_GROUP, mode::GROUP);
                self.put_rare(q, q + 1, leaf::GROUP_BEGIN);
                return q + 1;
            }
            if let Some(e) = self.entity_parameter(q) {
                return e;
            }
        }
        self.quoted_or_pop(p, q, mode::ELEMENT_CONTENT, mode::END)
    }

    /// An attribute list's attributes at `q`, the first byte from `p` that isn't a space.
    fn attributes(&mut self, p: usize, q: usize) -> usize {
        let b = self.b;
        let l = Line { b };
        if self.else_pop(p, q) == q {
            if let Some(e) = self.entity_parameter(q) {
                return e;
            }
            if b[q] == b'(' {
                self.markup(q, ENUMERATION, mode::GROUP);
                self.put_rare(q, q + 1, leaf::GROUP_BEGIN);
                return q + 1;
            }
            if let Some(m) = attr_type_at(&l, q) {
                self.put_rare(q, m.e as usize, leaf::ATTR_TYPE);
                return m.e as usize;
            }
            if b[q] == b'#'
                && let Some(m) = attr_default_at(&l, q)
            {
                self.emit(&m, leaf::ATTR_DEFAULT, &[(1, leaf::ATTR_DEFAULT + 1)]);
                return m.e as usize;
            }
            if let Some(m) = dtd_attr_name_at(&l, q) {
                self.put_rare(q, m.e as usize, leaf::ATTR_NAME);
                return m.e as usize;
            }
        }
        self.quoted_or_pop(p, q, mode::ATTRIBUTES, mode::END)
    }

    /// `entity-parameter` at `q`: `#PCDATA` or a `%name;`.
    fn entity_parameter(&mut self, q: usize) -> Option<usize> {
        let l = Line { b: self.b };
        match l.b[q] {
            b'#' => {
                let m = placeholder_at(&l, q)?;
                self.emit(&m, leaf::PLACEHOLDER, &[(1, leaf::PLACEHOLDER + 1)]);
                Some(m.e as usize)
            }
            b'%' => {
                let m = marked_name_at(&l, q)?;
                Some(self.reference(&m, leaf::PARAMETER))
            }
            _ => None,
        }
    }

    /// `&` at `k`: a reference, else a bad one.
    fn ampersand(&mut self, k: usize) -> usize {
        let l = Line { b: self.b };
        if l.b.get(k + 1) == Some(&b'#') {
            if let Some(m) = char_ref_at(&l, k, true) {
                return self.reference(&m, leaf::ENTITY_HEX);
            }
            if let Some(m) = char_ref_at(&l, k, false) {
                return self.reference(&m, leaf::ENTITY_DEC);
            }
        }
        if let Some(m) = marked_name_at(&l, k) {
            return self.reference(&m, leaf::ENTITY_NAMED);
        }
        self.put(k, k + 1, leaf::BAD_AMPERSAND);
        k + 1
    }

    /// An element's group or an attribute's enumeration from `p`: its names, separators,
    /// references and `)`.
    fn group(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let element = self.inner == ELEMENT_GROUP;
        let mut i = p;
        loop {
            let k = find(b, i, |c| {
                matches!(c, b')' | b'|' | b'#' | b'%' | b'&' | b'<')
                    || is_name2_start(c)
                    || (element && matches!(c, b'(' | b'*' | b'?' | b'+' | b','))
            })?;
            let c = b[k];
            match c {
                b')' => {
                    self.put_rare(k, k + 1, leaf::GROUP_END);
                    if element {
                        self.depth -= 1;
                        let inner = match self.depth {
                            0 => inner::ELEMENT,
                            _ => ELEMENT_GROUP,
                        };
                        self.markup(k + 1, inner, mode::GROUP_AFTER);
                    } else {
                        self.markup(k + 1, inner::ATTLIST, mode::ATTRIBUTES);
                    }
                    return Some(k + 1);
                }
                b'(' => {
                    self.depth = self.depth.saturating_add(1);
                    self.put_rare(k, k + 1, leaf::GROUP_BEGIN);
                    return Some(k + 1);
                }
                b'|' if !element => {
                    self.put_rare(k, k + 1, leaf::LOGICAL_SEP);
                    return Some(k + 1);
                }
                b'|' | b',' => {
                    self.put_rare(k, k + 1, leaf::SEPARATOR);
                    return Some(k + 1);
                }
                b'#' | b'%' => {
                    if let Some(e) = self.entity_parameter(k) {
                        return Some(e);
                    }
                }
                b'&' => return Some(self.ampersand(k)),
                b'<' => {
                    self.put_rare(k, k + 1, leaf::MISSING_ENTITY);
                    return Some(k + 1);
                }
                _ if is_quantifier(c) => {
                    self.put_rare(k, k + 1, leaf::OPERATOR);
                    return Some(k + 1);
                }
                _ => {
                    let e = k + 1 + b[k + 1..].iter().take_while(|&&c| is_name2(c)).count();
                    self.put_rare(k, e, leaf::UNQUOTED);
                    return Some(e);
                }
            }
            i = k + 1;
        }
    }
}
