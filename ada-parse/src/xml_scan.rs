//! XML by a scanner (ADR-40, speed first): bat's `XML` grammar compiled by hand, as TOML's is.
//! Each of the port's contexts (`xml.rs`) is an arm of one `match` on the top of the stack, and
//! each decides at the byte where its next match can start: text runs to the next `<` or `&`, a
//! tag's attributes and strings to the next quote, `<`, `&` or `>`. Nearly every context ends
//! in a pattern that matches any byte that isn't a space (the grammar's `else-pop`s), so the
//! leftmost match is the first pattern that matches at the first byte that isn't a space; the
//! rest search ahead once, for a byte. The port's matchers are shared, anchored at a position.
//!
//! The stack holds the port's context ids and packs into the same [`State`], so the port is an
//! exact reference: the tests hold every byte's scopes and every line's state to it. A span's
//! kind is fixed ([`scopes`]): the meta scopes of the stack as a set (a doctype's, its internal
//! subset's, a conditional section's, then one of a tag's, a comment's, a declaration's, a
//! group's), a string's, and a leaf. bat's stack repeats a group's or a conditional section's
//! meta scopes as they nest, which none of the built-in themes tells apart; here each is kept
//! once, the only way the scanner's scopes differ from the port's.

use std::cell::Cell;

use crate::machine::{Atom, DEPTH, Line, M, MAX_PUSH};
use crate::xml::{
    ATOMS, ATTLIST_PARENS_0, CDATA_0, COMMENT_0, DOCTYPE_META, DOCTYPE_ROOT_NAME,
    DTD_ATTLIST_CONTENT, DTD_ATTLIST_META, DTD_CONTENT_QUOTED, DTD_CONTENT_TYPE,
    DTD_CONTENT_UNQUOTED, DTD_ELEMENT_CONTENT, DTD_ELEMENT_META, DTD_ELEMENT_NAME,
    DTD_ELEMENT_PARENS, DTD_ENTITY_CONTENT, DTD_ENTITY_META, DTD_ENTITY_NAME,
    DTD_ENTITY_PUNCTUATION, DTD_NOTATION_META, DTD_NOTATION_NAME, DTD_SUBSET_BRACKETS,
    DTD_SUBSET_META, DTD_SUBSET_NAME, ELEMENT_PARENS_0, KEY_VALUE_0, MAIN_0, PREPROCESSOR_0,
    PREPROCESSOR_1, START, STRING_DOUBLE_QUOTED_BODY, STRING_SINGLE_QUOTED_BODY, SUBSET_BRACKETS_0,
    TAG_0, TAG_1, TAG_ATTRIBUTE_SEPARATOR, UNKNOWN_0, attr_default_at, attr_type_at, attribute,
    attribute_at, boundary, char_ref_at, declaration_at, doctype_at, dtd_attr_name_at, dtd_name_at,
    is_name, is_name_start, is_name2, is_name2_start, marked_name_at, missing_tag_end,
    placeholder_at, preprocessor_other_at, preprocessor_xml_at, qname, question_gt, s, slash_gt,
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
    pub const TAG: usize = 1;
    pub const PREPROCESSOR: usize = 2;
    pub const COMMENT: usize = 3;
    pub const CDATA: usize = 4;
    pub const UNKNOWN: usize = 5;
    pub const ATTLIST: usize = 6;
    /// An enumeration in a declaration, or a group: one past the declaration's.
    pub const ENUMERATED: usize = 8;
    pub const ELEMENT: usize = 9;
    pub const GROUP: usize = 11;
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

/// The searches a tag or a processing instruction makes ahead of a byte that decides nothing
/// (bad input), each kept for the line while it lies ahead, as syntect keeps a regex's match.
const MEMOS: usize = 4;
const SLASH_GT: usize = 0;
const QUESTION_GT: usize = 1;
const MISSING: usize = 2;
const ATTRIBUTE: usize = 3;
const UNKNOWN: u32 = u32::MAX;
/// The most scopes the machine holds at once.
const SCOPES: usize = 64;
const NO_MATCH: u32 = u32::MAX - 1;

/// Lex `b`, a line without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let l = Line {
        b,
        arg: Cell::new(0),
    };
    let mut scan = Scan {
        nl: b.len(),
        stack: [0; DEPTH],
        depth: 0,
        dropped: 0,
        exact: state.is_exact(),
        base: 0,
        text: 0,
        metas: 0,
        deep: false,
        at: 0,
        memo: [UNKNOWN; MEMOS],
        out,
    };
    match state.levels() {
        [] => {
            scan.stack[0] = START;
            scan.depth = 1;
        }
        levels => {
            scan.stack[..levels.len()].copy_from_slice(levels);
            scan.depth = levels.len();
        }
    }
    scan.refresh();
    scan.run(&l);
    scan.fill(scan.nl);
    static NO_WITHS: [u8; DEPTH] = [0; DEPTH];
    *state = State::pack(
        &scan.stack[..scan.depth],
        &NO_WITHS[..scan.depth],
        0,
        scan.exact,
    );
}

struct Scan<'b, 'c> {
    /// Where the `\n` is.
    nl: usize,
    stack: [u8; DEPTH],
    depth: usize,
    /// Levels dropped from the bottom, past [`DEPTH`], still counted toward syntect's push limit.
    dropped: usize,
    exact: bool,
    /// The kind of the stack's scopes, with no leaf.
    base: u16,
    /// The kind of text no pattern matches: [`Scan::base`], in CDATA its text's.
    text: u16,
    /// How many meta scopes the machine would hold for the stack, repeats included: past
    /// [`SCOPES`] it drops some, and the line isn't exact.
    metas: usize,
    /// A leaf's scopes may not fit beside the stack's.
    deep: bool,
    /// Where the spans so far end.
    at: usize,
    memo: [u32; MEMOS],
    out: &'b mut Out<'c>,
}

/// What each context gives the stack's kind, by its id: how many scopes it gives its text
/// (its meta and meta content scopes, bits 0–1), a flag (bits 2–4), an inner markup (bits 5–8),
/// a group or an enumeration (bits 9–10), a string (bits 11–12). The kind changes only with the
/// contexts that give some scopes.
static ROLES: [u16; 68] = {
    let mut roles = [0; 68];
    let mut c = 0;
    while c < 68 {
        roles[c] = role(c as u8);
        c += 1;
    }
    roles
};

const GROUP: u16 = 1;
const ENUMERATION: u16 = 2;

const fn role(c: u8) -> u16 {
    let (flag, decl, group, string) = match c {
        DOCTYPE_META => (DOCTYPE, 0, 0, 0),
        SUBSET_BRACKETS_0 => (SUBSET, 0, 0, 0),
        DTD_SUBSET_META => (SECTION, 0, 0, 0),
        TAG_0 | TAG_1 => (0, inner::TAG, 0, 0),
        PREPROCESSOR_0 | PREPROCESSOR_1 => (0, inner::PREPROCESSOR, 0, 0),
        COMMENT_0 => (0, inner::COMMENT, 0, 0),
        CDATA_0 => (0, inner::CDATA, 0, 0),
        UNKNOWN_0 => (0, inner::UNKNOWN, 0, 0),
        DTD_ATTLIST_META => (0, inner::ATTLIST, 0, 0),
        DTD_ELEMENT_META => (0, inner::ELEMENT, 0, 0),
        DTD_ENTITY_META => (0, inner::ENTITY, 0, 0),
        DTD_NOTATION_META => (0, inner::NOTATION, 0, 0),
        DTD_ELEMENT_PARENS => (0, 0, GROUP, 0),
        ATTLIST_PARENS_0 => (0, 0, ENUMERATION, 0),
        STRING_DOUBLE_QUOTED_BODY => (0, 0, 0, 1),
        STRING_SINGLE_QUOTED_BODY => (0, 0, 0, 2),
        _ => (0, 0, 0, 0),
    };
    let metas = match c {
        SUBSET_BRACKETS_0 | CDATA_0 => 2,
        _ if flag != 0 || decl != 0 || group != 0 || string != 0 => 1,
        _ => 0,
    };
    metas | (flag as u16) << 2 | (decl as u16) << 5 | group << 9 | string << 11
}

/// Whether a context gives its text scopes, so the stack's kind changes with it.
fn metas_of(c: u8) -> usize {
    usize::from(ROLES[usize::from(c)] & 3)
}

/// `[*?+]`
fn is_quantifier(c: u8) -> bool {
    matches!(c, b'*' | b'?' | b'+')
}

/// Where the spaces from `p` end.
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

impl Scan<'_, '_> {
    /// Bytes `s..e` are `leaf` in the stack's scopes; the text before them, from where the
    /// spans end, is the stack's text.
    fn put(&mut self, s: usize, e: usize, leaf: u16) {
        self.fill(s);
        let e = e.min(self.nl);
        let kind = self.base + leaf;
        if self.deep && self.metas + LEAVES[usize::from(leaf)].len() > SCOPES {
            self.exact = false;
        }
        if kind != 0 && e > s {
            self.out.push_merged(s, e, kind);
        }
        self.at = self.at.max(e);
    }

    /// The text from where the spans end to `pos` is the stack's text.
    fn fill(&mut self, pos: usize) {
        let pos = pos.min(self.nl);
        if pos > self.at {
            if self.text != 0 {
                self.out.push_merged(self.at, pos, self.text);
            }
            self.at = pos;
        }
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
                debug_assert!(gs >= at);
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

    /// The stack's kinds, after it changed.
    fn refresh(&mut self) {
        let (mut flags, mut decl, mut group, mut string) = (0, 0, 0, 0);
        let mut metas = 0;
        for &c in &self.stack[..self.depth] {
            let r = ROLES[usize::from(c)];
            metas += usize::from(r & 3);
            flags |= usize::from(r >> 2 & 7);
            if r >> 5 & 15 != 0 {
                decl = usize::from(r >> 5 & 15);
            }
            group |= r >> 9 & 3;
            if r >> 11 != 0 {
                string = usize::from(r >> 11);
            }
        }
        self.metas = metas;
        if metas > SCOPES {
            self.exact = false;
        }
        self.deep = metas + 2 > SCOPES;
        let inner = match (decl, group) {
            (d, 0) => d,
            (inner::ATTLIST | inner::ELEMENT, _) => decl + 1,
            // the group's declaration dropped with the bottom levels
            (_, ENUMERATION) => inner::ENUMERATED,
            _ => inner::GROUP,
        };
        self.base = kind(flags, inner, string, 0);
        self.text = match self.depth > 0 && self.top() == CDATA_0 {
            true => self.base + leaf::CDATA_TEXT,
            false => self.base,
        };
    }

    fn top(&self) -> u8 {
        self.stack[self.depth - 1]
    }

    /// Whether syntect would push here (it doesn't past 100 levels; a `set` it always does).
    fn can_push(&self) -> bool {
        self.depth + self.dropped < MAX_PUSH
    }

    /// Push `refs` at `at`, the text before it in the scopes so far.
    fn push(&mut self, at: usize, refs: &[u8]) {
        self.fill(at);
        for &r in refs {
            if self.depth == DEPTH {
                self.stack.copy_within(1.., 0);
                self.depth -= 1;
                self.dropped += 1;
                self.exact = false;
            }
            self.stack[self.depth] = r;
            self.depth += 1;
        }
        if refs.iter().any(|&r| metas_of(r) > 0) || self.dropped > 0 {
            self.refresh();
        }
    }

    fn set(&mut self, at: usize, refs: &[u8]) {
        self.fill(at);
        self.depth -= 1;
        let gone = self.stack[self.depth];
        self.push(at, refs);
        if metas_of(gone) > 0 {
            self.refresh();
        }
    }

    /// Pop at `at`, the text before it in the scopes so far.
    fn pop(&mut self, at: usize) {
        self.fill(at);
        self.depth -= 1;
        if metas_of(self.stack[self.depth]) > 0 || self.depth == 0 {
            self.refresh();
        }
    }

    fn run(&mut self, l: &Line) {
        let mut p = 0;
        while self.depth > 0 {
            match self.step(l, p) {
                Some(q) => p = q,
                None => return,
            }
        }
    }

    /// One match of the top context at `p`: where the next starts, or `None` when nothing
    /// matches before the line ends.
    fn step(&mut self, l: &Line, p: usize) -> Option<usize> {
        let b = l.b;
        let nl = self.nl;
        match self.top() {
            START => {
                if !self.can_push() {
                    return None;
                }
                self.push(p, &[MAIN_0]);
                Some(p)
            }
            MAIN_0 => self.main(l, p),
            COMMENT_0 => {
                let mut i = p;
                loop {
                    let k = find(b, i, |c| c == b'-')?;
                    if b.get(k + 1) == Some(&b'-') {
                        if b.get(k + 2) == Some(&b'>') {
                            self.put(k, k + 3, leaf::COMMENT_END);
                            self.pop(k + 3);
                            return Some(k + 3);
                        }
                        let e = l.run(k, |c| c == b'-');
                        self.put(k, e, leaf::DOUBLE_HYPHEN);
                        return Some(e);
                    }
                    i = k + 1;
                }
            }
            CDATA_0 => self.closer(l, p, b"]]>", leaf::TAG_END),
            PREPROCESSOR_1 => self.closer(l, p, b"?>", leaf::TAG_END),
            TAG_0 | DOCTYPE_META | DTD_ENTITY_META | DTD_ELEMENT_META | DTD_ATTLIST_META
            | DTD_NOTATION_META | UNKNOWN_0 => {
                // a close tag's end, or `dtd-end`; the same but for what a missing end is
                // before, and the scope of what else they take
                let q = spaces(b, p);
                if q >= nl {
                    return None;
                }
                let tag = self.top() == TAG_0;
                let missing = |c: u8| c == b'<' || (!tag && c == b']');
                let c = b[q];
                if missing(c) {
                    let s = if q > p { q - 1 } else { q };
                    self.put(s, q, leaf::MISSING_TAG_END);
                    self.pop(q);
                    return Some(q);
                }
                if c == b'>' {
                    self.put(q, q + 1, leaf::TAG_END);
                    self.pop(q + 1);
                    return Some(q + 1);
                }
                if matches!(c, b'/' | b'?') && b.get(q + 1) == Some(&b'>') {
                    self.put(q, q + 2, leaf::BAD_TAG_END);
                    self.pop(q + 2);
                    return Some(q + 2);
                }
                let e = q + l.char_len(q).unwrap_or(1);
                let unexpected = match tag {
                    true => leaf::UNEXPECTED_ATTR,
                    false => leaf::UNEXPECTED,
                };
                self.put(q, e, unexpected);
                Some(e)
            }
            TAG_1 => self.tag(l, p, true),
            PREPROCESSOR_0 => self.tag(l, p, false),
            TAG_ATTRIBUTE_SEPARATOR => {
                let q = spaces(b, p);
                if q >= nl {
                    return None;
                }
                if b[q] == b'=' {
                    self.put(q, q + 1, leaf::KEY_VALUE);
                    self.set(q + 1, &[KEY_VALUE_0]);
                    return Some(q + 1);
                }
                let at = self.tag_else(b, p, q);
                self.pop(at);
                Some(at)
            }
            KEY_VALUE_0 => {
                let q = spaces(b, p);
                if q >= nl {
                    return None;
                }
                let c = b[q];
                if q > p && c == b'<' {
                    self.pop(q - 1);
                    return Some(q - 1);
                }
                if c == b'"' || c == b'\'' {
                    let body = match c {
                        b'"' => STRING_DOUBLE_QUOTED_BODY,
                        _ => STRING_SINGLE_QUOTED_BODY,
                    };
                    self.set(q, &[body]);
                    self.put(q, q + 1, leaf::STR_BEGIN);
                    return Some(q + 1);
                }
                if !matches!(c, b'?' | b'/' | b'<' | b'>') {
                    let e = l.run(q, |c| {
                        !matches!(c, b'?' | b'/' | b'<' | b'>') && !is_space(c)
                    });
                    self.put(q, e, leaf::BAD_ATTR_VALUE);
                    self.pop(e);
                    return Some(e);
                }
                self.pop(q);
                Some(q)
            }
            body @ (STRING_DOUBLE_QUOTED_BODY | STRING_SINGLE_QUOTED_BODY) => {
                let quote = match body {
                    STRING_DOUBLE_QUOTED_BODY => b'"',
                    _ => b'\'',
                };
                let k = find(b, p, |c| c == quote || c == b'<' || c == b'&')?;
                match b[k] {
                    b'<' => {
                        let s = if k > p && is_space(b[k - 1]) {
                            k - 1
                        } else {
                            k
                        };
                        self.put(s, k, leaf::MISSING_TAG_END);
                        self.pop(k);
                        Some(k)
                    }
                    b'&' => Some(self.ampersand(l, k)),
                    _ => {
                        self.put(k, k + 1, leaf::STR_END);
                        self.pop(k + 1);
                        Some(k + 1)
                    }
                }
            }
            DOCTYPE_ROOT_NAME => {
                let q = spaces(b, p);
                if q >= nl {
                    return None;
                }
                if q > p && matches!(b[q], b']' | b'<') {
                    self.pop(q - 1);
                    return Some(q - 1);
                }
                if let Some(m) = qname(l, q, M::new(q, q), 1) {
                    self.emit(
                        &m,
                        leaf::NONE,
                        &[
                            (1, leaf::ROOT_NS),
                            (2, leaf::BAD_TAG_NAME),
                            (3, leaf::ROOT_NS + 1),
                            (4, leaf::ROOT_NS + 2),
                            (5, leaf::BAD_TAG_NAME),
                        ],
                    );
                    self.pop(m.e as usize);
                    return Some(m.e as usize);
                }
                self.pop(q);
                Some(q)
            }
            DTD_CONTENT_TYPE => {
                let q = spaces(b, p);
                if q >= nl {
                    return None;
                }
                let at = self.else_pop(b, p, q);
                let rest = &b[q..];
                if at == q
                    && (rest.starts_with(b"PUBLIC") || rest.starts_with(b"SYSTEM"))
                    && boundary(l, q + 6)
                {
                    self.put(q, q + 6, leaf::EXTERNAL);
                    self.pop(q + 6);
                    return Some(q + 6);
                }
                self.pop(at);
                Some(at)
            }
            DTD_CONTENT_QUOTED => {
                let q = spaces(b, p);
                if q >= nl {
                    return None;
                }
                self.quoted_or_pop(b, p, q)
            }
            DTD_SUBSET_BRACKETS => {
                let q = spaces(b, p);
                if q >= nl {
                    return None;
                }
                let at = self.else_pop(b, p, q);
                if at == q && b[q] == b'[' {
                    self.set(q, &[SUBSET_BRACKETS_0]);
                    self.put(q, q + 1, leaf::BRACKETS_BEGIN);
                    return Some(q + 1);
                }
                self.pop(at);
                Some(at)
            }
            SUBSET_BRACKETS_0 => self.dtd(l, p),
            DTD_SUBSET_META => {
                let k = find(b, p, |c| c == b'>' || c == b'<')?;
                if b[k] == b'<' {
                    let s = if k > p && is_space(b[k - 1]) {
                        k - 1
                    } else {
                        k
                    };
                    self.put(s, k, leaf::MISSING_TAG_END);
                    self.pop(k);
                    return Some(k);
                }
                let before = (k > p).then(|| b[k - 1]);
                let (s, end) = match before {
                    Some(b']') => (k - 1, leaf::TAG_END),
                    Some(b'/' | b'?') => (k - 1, leaf::BAD_TAG_END),
                    _ => (k, leaf::BAD_TAG_END),
                };
                self.put(s, k + 1, end);
                self.pop(k + 1);
                Some(k + 1)
            }
            name @ (DTD_ENTITY_NAME | DTD_ELEMENT_NAME | DTD_NOTATION_NAME | DTD_SUBSET_NAME) => {
                let q = spaces(b, p);
                if q >= nl {
                    return None;
                }
                if q > p && matches!(b[q], b']' | b'<') {
                    self.pop(q - 1);
                    return Some(q - 1);
                }
                if let Some(m) = dtd_name_at(l, q) {
                    let leaf = match name {
                        DTD_ENTITY_NAME => leaf::ENTITY_NAME,
                        DTD_ELEMENT_NAME => leaf::ELEMENT_NAME,
                        DTD_NOTATION_NAME => leaf::NOTATION_NAME,
                        _ => leaf::SUBSET_NAME,
                    };
                    self.put(q, m.e as usize, leaf);
                    self.pop(m.e as usize);
                    return Some(m.e as usize);
                }
                // `dtd-common-name`
                if b[q] == b'%'
                    && let Some(m) = marked_name_at(l, q)
                {
                    let e = self.reference(&m, leaf::PARAMETER);
                    self.pop(e);
                    return Some(e);
                }
                let ident = |c: u8| !b"'\"[]()<>".contains(&c) && !is_space(c);
                if ident(b[q]) {
                    let e = l.run(q, ident);
                    self.put(q, e, leaf::BAD_IDENTIFIER);
                    self.pop(e);
                    return Some(e);
                }
                self.pop(q);
                Some(q)
            }
            DTD_ENTITY_PUNCTUATION => {
                let q = spaces(b, p);
                if q >= nl {
                    return None;
                }
                let at = self.else_pop(b, p, q);
                if at == q && b[q] == b'%' {
                    self.put(q, q + 1, leaf::ENTITY_MARK);
                    self.pop(q + 1);
                    return Some(q + 1);
                }
                self.pop(at);
                Some(at)
            }
            DTD_ENTITY_CONTENT => {
                let q = spaces(b, p);
                if q >= nl {
                    return None;
                }
                if self.else_pop(b, p, q) == q && b[q..].starts_with(b"NDATA") && boundary(l, q + 5)
                {
                    self.put(q, q + 5, leaf::NDATA);
                    self.set(q + 5, &[DTD_CONTENT_UNQUOTED]);
                    return Some(q + 5);
                }
                self.quoted_or_pop(b, p, q)
            }
            DTD_CONTENT_UNQUOTED => {
                let q = spaces(b, p);
                if q >= nl {
                    return None;
                }
                let at = self.else_pop(b, p, q);
                if at == q && is_name2_start(b[q]) {
                    let e = l.run(q + 1, is_name2);
                    self.put(q, e, leaf::UNQUOTED);
                    return Some(e);
                }
                self.pop(at);
                Some(at)
            }
            DTD_ELEMENT_CONTENT => {
                let q = spaces(b, p);
                if q >= nl {
                    return None;
                }
                if self.else_pop(b, p, q) == q {
                    let rest = &b[q..];
                    let word = [&b"EMPTY"[..], b"ANY"]
                        .into_iter()
                        .find(|w| rest.starts_with(w) && boundary(l, q + w.len()));
                    if let Some(w) = word
                        && boundary(l, q)
                    {
                        self.put(q, q + w.len(), leaf::CONSTANT);
                        self.pop(q + w.len());
                        return Some(q + w.len());
                    }
                    if b[q] == b'(' {
                        self.set(q, &[DTD_ELEMENT_PARENS]);
                        self.put(q, q + 1, leaf::GROUP_BEGIN);
                        return Some(q + 1);
                    }
                    if let Some(e) = self.entity_parameter(l, q) {
                        return Some(e);
                    }
                }
                self.quoted_or_pop(b, p, q)
            }
            ctx @ (DTD_ELEMENT_PARENS | ATTLIST_PARENS_0) => self.group(l, p, ctx),
            ELEMENT_PARENS_0 => {
                let q = spaces(b, p);
                if q >= nl {
                    return None;
                }
                let at = self.else_pop(b, p, q);
                if at == q && is_quantifier(b[q]) {
                    self.put(q, q + 1, leaf::OPERATOR);
                    self.pop(q + 1);
                    return Some(q + 1);
                }
                self.pop(at);
                Some(at)
            }
            DTD_ATTLIST_CONTENT => {
                let q = spaces(b, p);
                if q >= nl {
                    return None;
                }
                if self.else_pop(b, p, q) == q {
                    if let Some(e) = self.entity_parameter(l, q) {
                        return Some(e);
                    }
                    if b[q] == b'(' && self.can_push() {
                        self.push(q, &[ATTLIST_PARENS_0]);
                        self.put(q, q + 1, leaf::GROUP_BEGIN);
                        return Some(q + 1);
                    }
                    if let Some(m) = attr_type_at(l, q) {
                        self.put(q, m.e as usize, leaf::ATTR_TYPE);
                        return Some(m.e as usize);
                    }
                    if b[q] == b'#'
                        && let Some(m) = attr_default_at(l, q)
                    {
                        self.emit(&m, leaf::ATTR_DEFAULT, &[(1, leaf::ATTR_DEFAULT + 1)]);
                        return Some(m.e as usize);
                    }
                    if let Some(m) = dtd_attr_name_at(l, q) {
                        self.put(q, m.e as usize, leaf::ATTR_NAME);
                        return Some(m.e as usize);
                    }
                }
                self.quoted_or_pop(b, p, q)
            }
            // the rest are only ever included
            _ => None,
        }
    }

    /// `\s?(?=<)` and the empty match of `tag-else-pop`: where it pops, `q` the first byte
    /// from `p` that isn't a space.
    fn tag_else(&self, b: &[u8], p: usize, q: usize) -> usize {
        if q > p && b[q] == b'<' { q - 1 } else { q }
    }

    /// `dtd-else-pop`: where it pops, as [`Scan::tag_else`], a `]` too.
    fn else_pop(&self, b: &[u8], p: usize, q: usize) -> usize {
        if q > p && matches!(b[q], b']' | b'<') {
            q - 1
        } else {
            q
        }
    }

    /// `dtd-content-quoted` at `q`, the first byte from `p` that isn't a space: a quoted
    /// string, else pop.
    fn quoted_or_pop(&mut self, b: &[u8], p: usize, q: usize) -> Option<usize> {
        let at = self.else_pop(b, p, q);
        if at == q && matches!(b[q], b'"' | b'\'') && self.can_push() {
            let body = match b[q] {
                b'"' => STRING_DOUBLE_QUOTED_BODY,
                _ => STRING_SINGLE_QUOTED_BODY,
            };
            self.push(q, &[body]);
            self.put(q, q + 1, leaf::STR_BEGIN);
            return Some(q + 1);
        }
        self.pop(at);
        Some(at)
    }

    /// A context that only ends at `close`, its scope `end`.
    fn closer(&mut self, l: &Line, p: usize, close: &[u8], end: u16) -> Option<usize> {
        let k = l.lit(p, close)?.s as usize;
        let e = k + close.len();
        self.put(k, e, end);
        self.pop(e);
        Some(e)
    }

    /// `entity-parameter` at `q`: `#PCDATA` or a `%name;`.
    fn entity_parameter(&mut self, l: &Line, q: usize) -> Option<usize> {
        match l.b[q] {
            b'#' => {
                let m = placeholder_at(l, q)?;
                self.emit(&m, leaf::PLACEHOLDER, &[(1, leaf::PLACEHOLDER + 1)]);
                Some(m.e as usize)
            }
            b'%' => {
                let m = marked_name_at(l, q)?;
                Some(self.reference(&m, leaf::PARAMETER))
            }
            _ => None,
        }
    }

    /// `&` at `k`: a reference (`entity`), else a bad one (`should-be-entity`).
    fn ampersand(&mut self, l: &Line, k: usize) -> usize {
        if l.b.get(k + 1) == Some(&b'#') {
            if let Some(m) = char_ref_at(l, k, true) {
                return self.reference(&m, leaf::ENTITY_HEX);
            }
            if let Some(m) = char_ref_at(l, k, false) {
                return self.reference(&m, leaf::ENTITY_DEC);
            }
        }
        if let Some(m) = marked_name_at(l, k) {
            return self.reference(&m, leaf::ENTITY_NAMED);
        }
        self.put(k, k + 1, leaf::BAD_AMPERSAND);
        k + 1
    }

    /// The document's text, up to the next `<` or `&`.
    fn main(&mut self, l: &Line, p: usize) -> Option<usize> {
        let b = l.b;
        let k = find(b, p, |c| c == b'<' || c == b'&')?;
        if b[k] == b'&' {
            return Some(self.ampersand(l, k));
        }
        let push = self.can_push();
        match b.get(k + 1) {
            Some(b'?') if push => {
                if let Some(e) = self.preprocessor(l, k) {
                    return Some(e);
                }
            }
            Some(b'!') if push => {
                if let Some(m) = doctype_at(l, k) {
                    self.push(
                        k,
                        &[
                            DOCTYPE_META,
                            DTD_SUBSET_BRACKETS,
                            DTD_CONTENT_QUOTED,
                            DTD_CONTENT_TYPE,
                            DOCTYPE_ROOT_NAME,
                        ],
                    );
                    self.emit(
                        &m,
                        leaf::NONE,
                        &[
                            (1, leaf::TAG_BEGIN),
                            (2, leaf::DOCTYPE_KW),
                            (3, leaf::BAD_TAG_NAME),
                        ],
                    );
                    return Some(m.e as usize);
                }
                if let Some(e) = self.comment_or_cdata(b, k) {
                    return Some(e);
                }
            }
            Some(b'/') if push => {
                if let Some(e) = self.plain_tag(l, k, 2, TAG_0) {
                    return Some(e);
                }
                let m = qname(l, k + 2, M::new(k, k).g(1, k, k + 2), 2);
                if let Some(m) = m {
                    self.push(k, &[TAG_0]);
                    return Some(self.tag_name(&m));
                }
            }
            _ if push => {
                if let Some(e) = self.plain_tag(l, k, 1, TAG_1) {
                    return Some(e);
                }
                let m = qname(l, k + 1, M::new(k, k).g(1, k, k + 1), 2);
                if let Some(m) = m {
                    self.push(k, &[TAG_1]);
                    return Some(self.tag_name(&m));
                }
            }
            _ => {}
        }
        self.put(k, k + 1, leaf::MISSING_ENTITY);
        Some(k + 1)
    }

    /// A tag at `k` whose `<` or `</` (`open` bytes) a plain local name follows, as [`qname`]
    /// reads it: no namespace, and one of `[/<>\s]` after it.
    fn plain_tag(&mut self, l: &Line, k: usize, open: usize, ctx: u8) -> Option<usize> {
        let q = k + open;
        if !l.is(q, is_name_start) {
            return None;
        }
        let e = l.run(q + 1, is_name);
        if !l.is(e, |c| matches!(c, b'/' | b'<' | b'>') || is_space(c)) {
            return None;
        }
        self.push(k, &[ctx]);
        self.put(k, q, leaf::TAG_BEGIN);
        self.put(q, e, leaf::TAG_NS + 2);
        Some(e)
    }

    /// A tag's `<` or `</` and name, from [`qname`]'s groups.
    fn tag_name(&mut self, m: &M) -> usize {
        self.emit(
            m,
            leaf::NONE,
            &[
                (1, leaf::TAG_BEGIN),
                (2, leaf::TAG_NS),
                (3, leaf::BAD_TAG_NAME),
                (4, leaf::TAG_NS + 1),
                (5, leaf::TAG_NS + 2),
                (6, leaf::BAD_TAG_NAME),
            ],
        );
        m.e as usize
    }

    /// `<!--` or `<![CDATA[` at `k`.
    fn comment_or_cdata(&mut self, b: &[u8], k: usize) -> Option<usize> {
        let rest = &b[k..];
        if rest.starts_with(b"<!--") {
            self.push(k, &[COMMENT_0]);
            self.put(k, k + 4, leaf::COMMENT_BEGIN);
            return Some(k + 4);
        }
        if rest.starts_with(b"<![CDATA[") {
            self.push(k, &[CDATA_0]);
            self.put(k, k + 3, leaf::TAG_BEGIN);
            self.put(k + 3, k + 8, leaf::CDATA_KW);
            self.put(k + 8, k + 9, leaf::TAG_BEGIN);
            return Some(k + 9);
        }
        None
    }

    /// A processing instruction at `k`, where a `<?` is.
    fn preprocessor(&mut self, l: &Line, k: usize) -> Option<usize> {
        if let Some(m) = preprocessor_xml_at(l, k) {
            self.push(k, &[PREPROCESSOR_0]);
            let caps = [
                (1, leaf::TAG_BEGIN),
                (2, leaf::TAG_NAME),
                (3, leaf::BAD_TAG_NAME),
            ];
            self.emit(&m, leaf::NONE, &caps);
            return Some(m.e as usize);
        }
        let m = preprocessor_other_at(l, k)?;
        self.push(k, &[PREPROCESSOR_1]);
        self.emit(&m, leaf::NONE, &[(1, leaf::TAG_BEGIN), (2, leaf::TAG_NAME)]);
        Some(m.e as usize)
    }

    /// Inside a tag (`tag`) or an `<?xml …?>`: its end, an attribute, or a missing end.
    fn tag(&mut self, l: &Line, p: usize, tag: bool) -> Option<usize> {
        let b = l.b;
        let q = spaces(b, p);
        if q >= self.nl {
            return None;
        }
        let c = b[q];
        let next = b.get(q + 1).copied();
        if c == b'<' {
            let s = if q > p { q - 1 } else { q };
            self.put(s, q, leaf::MISSING_TAG_END);
            self.pop(q);
            return Some(q);
        }
        if tag && (c == b'>' || (c == b'/' && next == Some(b'>'))) {
            let e = if c == b'>' { q + 1 } else { q + 2 };
            self.put(q, e, leaf::TAG_END);
            self.pop(e);
            return Some(e);
        }
        if c == b'?' && next == Some(b'>') {
            let end = if tag {
                leaf::BAD_TAG_END
            } else {
                leaf::TAG_END
            };
            self.put(q, q + 2, end);
            self.pop(q + 2);
            return Some(q + 2);
        }
        if !self.can_push() {
            return self.tag_slow(l, p, tag);
        }
        // a plain local name, as `attribute_at` reads it: no namespace, its look-ahead after it
        if is_name_start(c) {
            let e = l.run(q + 1, is_name);
            let ahead = l.is(e, |c| matches!(c, b'=' | b'<' | b'>') || is_space(c))
                || (l.is(e, |c| c == b'/' || c == b'?') && l.at(e + 1) == Some(b'>'));
            if ahead {
                self.push(q, &[TAG_ATTRIBUTE_SEPARATOR]);
                self.put(q, e, leaf::ATTR_NS + 2);
                return Some(e);
            }
        }
        match attribute_at(l, q) {
            Some(m) => Some(self.attribute(&m)),
            None => self.tag_slow(l, p, tag),
        }
    }

    /// An attribute's name: its groups, then its `=` and value to come.
    fn attribute(&mut self, m: &M) -> usize {
        self.push(m.s as usize, &[TAG_ATTRIBUTE_SEPARATOR]);
        self.emit(
            m,
            leaf::NONE,
            &[
                (1, leaf::ATTR_NS),
                (2, leaf::BAD_ATTR_NAME),
                (3, leaf::ATTR_NS + 1),
                (4, leaf::ATTR_NS + 2),
                (5, leaf::BAD_ATTR_NAME),
            ],
        );
        m.e as usize
    }

    /// A pattern's leftmost match from `p`, kept for the line while it lies ahead.
    fn memo(
        &mut self,
        slot: usize,
        l: &Line,
        p: usize,
        find: fn(&Line, usize) -> Option<M>,
    ) -> Option<M> {
        match self.memo[slot] {
            NO_MATCH => None,
            at if at != UNKNOWN && at as usize >= p => find(l, at as usize),
            _ => {
                let m = find(l, p);
                self.memo[slot] = m.map_or(NO_MATCH, |m| m.s);
                m
            }
        }
    }

    /// [`Scan::tag`] where the first byte that isn't a space decides nothing: the leftmost of
    /// its patterns, ties to the first, as the machine finds it.
    fn tag_slow(&mut self, l: &Line, p: usize, tag: bool) -> Option<usize> {
        let gt = match tag {
            true => self.memo(SLASH_GT, l, p, slash_gt),
            false => None,
        };
        let question = self.memo(QUESTION_GT, l, p, question_gt);
        let missing = self.memo(MISSING, l, p, missing_tag_end);
        let attribute = match self.can_push() {
            true => self.memo(ATTRIBUTE, l, p, attribute),
            false => None,
        };
        let best = [gt, question, missing, attribute]
            .into_iter()
            .enumerate()
            .filter_map(|(i, m)| m.map(|m| (m.s, i, m)))
            .min_by_key(|&(s, i, _)| (s, i))?;
        let (s, e) = (best.2.s as usize, best.2.e as usize);
        match best.1 {
            0 => {
                self.put(s, e, leaf::TAG_END);
                self.pop(e);
            }
            1 => {
                let end = if tag {
                    leaf::BAD_TAG_END
                } else {
                    leaf::TAG_END
                };
                self.put(s, e, end);
                self.pop(e);
            }
            2 => {
                self.put(s, e, leaf::MISSING_TAG_END);
                self.pop(e);
            }
            _ => return Some(self.attribute(&best.2)),
        }
        Some(e)
    }

    /// The internal subset: a declaration, a comment, a reference, its `]`.
    fn dtd(&mut self, l: &Line, p: usize) -> Option<usize> {
        let b = l.b;
        let mut i = p;
        loop {
            let k = find(b, i, |c| matches!(c, b']' | b'<' | b'#' | b'%'))?;
            match b[k] {
                b']' => {
                    self.put(k, k + 1, leaf::BRACKETS_END);
                    self.pop(k + 1);
                    return Some(k + 1);
                }
                b'<' if self.can_push() => {
                    if let Some(e) = self.declaration(l, k) {
                        return Some(e);
                    }
                }
                b'#' | b'%' => {
                    if let Some(e) = self.entity_parameter(l, k) {
                        return Some(e);
                    }
                }
                _ => {}
            }
            i = k + 1;
        }
    }

    /// A declaration, a comment, CDATA or a processing instruction at `k`, where a `<` is.
    fn declaration(&mut self, l: &Line, k: usize) -> Option<usize> {
        let b = l.b;
        if b.get(k + 1) == Some(&b'?') {
            return self.preprocessor(l, k);
        }
        if b.get(k + 1) != Some(&b'!') {
            return None;
        }
        // CDATA before the comment, as `dtd` includes them; neither starts as the other
        if let Some(e) = self.comment_or_cdata(b, k) {
            return Some(e);
        }
        const DECLS: [(&[u8], u16, &[u8]); 4] = [
            (
                b"ENTITY",
                leaf::ENTITY_KW,
                &[
                    DTD_ENTITY_META,
                    DTD_ENTITY_CONTENT,
                    DTD_CONTENT_TYPE,
                    DTD_ENTITY_NAME,
                    DTD_ENTITY_PUNCTUATION,
                ],
            ),
            (
                b"ELEMENT",
                leaf::ELEMENT_KW,
                &[DTD_ELEMENT_META, DTD_ELEMENT_CONTENT, DTD_ELEMENT_NAME],
            ),
            (
                b"ATTLIST",
                leaf::ATTLIST_KW,
                &[DTD_ATTLIST_META, DTD_ATTLIST_CONTENT, DTD_ELEMENT_NAME],
            ),
            (
                b"NOTATION",
                leaf::NOTATION_KW,
                &[
                    DTD_NOTATION_META,
                    DTD_CONTENT_QUOTED,
                    DTD_CONTENT_TYPE,
                    DTD_NOTATION_NAME,
                ],
            ),
        ];
        for (kw, kw_leaf, refs) in DECLS {
            if let Some(m) = declaration_at(l, k, kw) {
                self.push(k, refs);
                self.emit(&m, leaf::NONE, &[(1, leaf::TAG_BEGIN), (2, kw_leaf)]);
                return Some(m.e as usize);
            }
        }
        if b.get(k + 2) == Some(&b'[') {
            self.push(k, &[DTD_SUBSET_META, DTD_SUBSET_BRACKETS, DTD_SUBSET_NAME]);
            self.put(k, k + 3, leaf::TAG_BEGIN);
            return Some(k + 3);
        }
        let m = unknown_at(l, k);
        self.push(k, &[UNKNOWN_0]);
        self.emit(
            &m,
            leaf::NONE,
            &[(1, leaf::TAG_BEGIN), (2, leaf::BAD_TAG_NAME)],
        );
        Some(m.e as usize)
    }

    /// A group in an element's declaration or an attribute's enumeration: its names,
    /// separators, references and `)`.
    fn group(&mut self, l: &Line, p: usize, ctx: u8) -> Option<usize> {
        let b = l.b;
        let element = ctx == DTD_ELEMENT_PARENS;
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
                    self.put(k, k + 1, leaf::GROUP_END);
                    match element {
                        true => self.set(k + 1, &[ELEMENT_PARENS_0]),
                        false => self.pop(k + 1),
                    }
                    return Some(k + 1);
                }
                b'(' => {
                    if self.can_push() {
                        self.push(k, &[DTD_ELEMENT_PARENS]);
                        self.put(k, k + 1, leaf::GROUP_BEGIN);
                        return Some(k + 1);
                    }
                }
                b'|' if !element => {
                    self.put(k, k + 1, leaf::LOGICAL_SEP);
                    return Some(k + 1);
                }
                b'|' | b',' => {
                    self.put(k, k + 1, leaf::SEPARATOR);
                    return Some(k + 1);
                }
                b'#' | b'%' => {
                    if let Some(e) = self.entity_parameter(l, k) {
                        return Some(e);
                    }
                }
                b'&' => return Some(self.ampersand(l, k)),
                b'<' => {
                    self.put(k, k + 1, leaf::MISSING_ENTITY);
                    return Some(k + 1);
                }
                _ if is_quantifier(c) => {
                    self.put(k, k + 1, leaf::OPERATOR);
                    return Some(k + 1);
                }
                _ => {
                    let e = l.run(k + 1, is_name2);
                    self.put(k, e, leaf::UNQUOTED);
                    return Some(e);
                }
            }
            i = k + 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Span, machine, xml};

    /// Each byte's scopes, as `kind_scopes` names a span's kind, each scope once.
    fn bytes(
        len: usize,
        spans: &[Span],
        kind_scopes: impl Fn(u16) -> Vec<&'static str>,
    ) -> Vec<String> {
        let mut out = vec![String::new(); len];
        let mut last = 0;
        for s in spans {
            assert!(
                s.start >= last && s.end > s.start && s.end as usize <= len,
                "{spans:?}"
            );
            last = s.end;
            let mut scopes = kind_scopes(s.kind);
            let mut seen = Vec::new();
            scopes.retain(|s| {
                let new = !seen.contains(s);
                seen.push(*s);
                new
            });
            let scopes = scopes.join(" ");
            for b in &mut out[s.start as usize..s.end as usize] {
                b.clone_from(&scopes);
            }
        }
        out
    }

    /// The scanner gives each line of `block` the port's scopes (each once) and leaves it the
    /// port's state.
    fn agree(block: &str) -> Result<(), String> {
        let (mut port, mut scan) = (State::default(), State::default());
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for (n, text) in block.split('\n').enumerate() {
            a.clear();
            b.clear();
            machine::line(
                &xml::GRAMMAR,
                text.as_bytes(),
                &mut port,
                &mut Out::new(&mut a),
            );
            line(text.as_bytes(), &mut scan, &mut Out::new(&mut b));
            let want = bytes(text.len(), &a, |k| xml::GRAMMAR.scopes(k));
            let mut got = bytes(text.len(), &b, scopes);
            // where the machine's scopes are full it drops the innermost; the scanner keeps them
            for s in &a {
                if xml::GRAMMAR.scopes(s.kind).len() >= SCOPES {
                    for i in s.start..s.end {
                        got[i as usize].clone_from(&want[i as usize]);
                    }
                }
            }
            if want != got {
                let i = (0..text.len()).find(|&i| want[i] != got[i]).unwrap_or(0);
                return Err(format!(
                    "line {n} {text:?}, byte {i}: port {:?}, scanner {:?}",
                    want[i], got[i]
                ));
            }
            if port != scan {
                return Err(format!(
                    "line {n} {text:?}: state port {port:?}, scanner {scan:?}"
                ));
            }
        }
        Ok(())
    }

    #[test]
    fn agrees_with_the_port_on_examples() {
        let blocks = [
            "<?xml version=\"1.0\" encoding='UTF-8'?>\n<root a=\"1\">text &amp; more</root>\n",
            "<?xml-stylesheet href=\"a.xsl\"?>\n<?php echo 1; ?>\n<?XmL x?>\n<?123?>\n",
            "<!DOCTYPE note SYSTEM \"note.dtd\" [\n  <!ELEMENT note (to,from?,(a|b)*)>\n  <!ELEMENT br EMPTY>\n  <!ATTLIST note id ID #REQUIRED kind (x|y) \"x\" ref IDREFS #IMPLIED>\n  <!ENTITY % param \"value\">\n  <!ENTITY pic SYSTEM \"p.gif\" NDATA gif>\n  <!NOTATION gif PUBLIC \"image/gif\">\n  %param;\n  <!BOGUS x>\n]>\n",
            "<!doctype html>\n<!DOCTYPE>\n<!DOCTYPE a:b PUBLIC \"x\" 'y'>\n",
            "<ns:root xmlns:ns=\"urn:x\" a='1' b=bare c = \"&amp;&#65;&#x41;&bad\">\n",
            "  <!-- a comment -- with -- dashes --->\n  <![CDATA[ raw <text> ]]>\n",
            "  <item key=\"v\"/>text &lt; & more < x\n  <a:b:c d:e:f=\"1\" ?>\n  <bad$name 1x=\"2\" / >\n",
            "  <multi\n    attr=\"v\"\n  >\n</ns:root>\n<!-- open\ncomment -->\n",
            "<a b=\"x <c>\n<a b='open\nstill' c\n=\n'v'>\n</a >\n</ a>\n</a x>\n",
            "<a =\"x\" b/c=1 d=/>\n<a ?x=1 /y>\n<?xml a/b=1 >x?>\n<a b==\"c\">\n",
            "<!DOCTYPE x [\n<![ INCLUDE [\n<!ELEMENT a ((b|c)+, d)*>\n<![%p;[<!ATTLIST a b CDATA #FIXED \"1\">]]>\n]]>\n]>\n",
            "<!DOCTYPE x [ <!ENTITY a \"&#38;&amp;\"> <!-- c --> <?pi x?> <![CDATA[x]]> ]>\n",
            "<!DOCTYPE x [<!ELEMENT a (#PCDATA|b)* > <!ATTLIST a b (c|d|&e;|<) #IMPLIED>]>\n",
            "\u{e9}<\u{e9}l\u{e9} \u{e9}=\"\u{e9}\">&\u{e9};</\u{e9}l\u{e9}>\n",
        ];
        for block in blocks {
            agree(block).unwrap();
        }
        // past the depth a line holds, and past syntect's push limit
        agree(&format!("<!DOCTYPE x [<!ELEMENT a {}b", "(".repeat(200))).unwrap();
        agree(&format!("<!DOCTYPE x [{}", "<![[".repeat(60))).unwrap();
    }

    /// SplitMix64, so the test needs no crate: any seed, zero included.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }

        fn pick<'a>(&mut self, of: &[&'a str]) -> &'a str {
            of[(self.next() % of.len() as u64) as usize]
        }
    }

    /// Blocks of XML's tokens in random order, against the port.
    #[test]
    fn agrees_with_the_port_on_token_soup() {
        let tokens = [
            "<",
            ">",
            "</",
            "/>",
            "<?",
            "?>",
            "<?xml",
            "<?xml-s",
            "<?php",
            "<!",
            "<!--",
            "-->",
            "--",
            "-",
            "<![CDATA[",
            "]]>",
            "<!DOCTYPE",
            "<!doctype",
            "<!ELEMENT",
            "<!ATTLIST",
            "<!ENTITY",
            "<!NOTATION",
            "<![",
            "[",
            "]",
            "]>",
            "(",
            ")",
            "|",
            ",",
            "*",
            "?",
            "+",
            "#PCDATA",
            "#CDATA",
            "#REQUIRED",
            "#FIXED",
            "#",
            "%",
            "%p;",
            "&",
            "&amp;",
            "&#65;",
            "&#x41;",
            "&#x;",
            ";",
            "=",
            "\"",
            "'",
            " ",
            "  ",
            "\t",
            "\n",
            "\n",
            "\n",
            "a",
            "b:c",
            "ns:",
            ":",
            "x1",
            "1",
            "_",
            ".",
            "/",
            "PUBLIC",
            "SYSTEM",
            "NDATA",
            "EMPTY",
            "ANY",
            "CDATA",
            "ID",
            "IDREFS",
            "NMTOKEN",
            "é",
            "$",
        ];
        let rounds: u64 = std::env::var("XML_SOUP")
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(20_000);
        let mut rng = Rng(0);
        for _ in 0..rounds {
            let n = 1 + rng.next() % 40;
            let block: String = (0..n).map(|_| rng.pick(&tokens)).collect();
            if let Err(e) = agree(&block) {
                panic!("{e}\nblock: {block:?}");
            }
        }
    }

    /// Every file listed in `$XML_CORPUS` (a file of paths), against the port.
    #[test]
    fn agrees_with_the_port_on_a_corpus() {
        let Ok(list) = std::env::var("XML_CORPUS") else {
            return;
        };
        let list = std::fs::read_to_string(list).expect("a list of paths");
        for path in list.lines() {
            let text = std::fs::read_to_string(path).unwrap_or_default();
            if let Err(e) = agree(&text) {
                panic!("{path}: {e}");
            }
        }
    }
}
