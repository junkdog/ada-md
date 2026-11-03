//! YAML by a scanner of ada's own (ADR-40, speed first): one pass over a line's bytes,
//! dispatching on the byte at each token's start, in place of bat's 47 contexts. It reads YAML the way bat's grammar does where text is well formed:
//! keys (a plain scalar a `: ` follows), plain scalars to where the grammar ends them, the
//! implicit types (null, booleans, numbers, timestamps, `=`, `<<`), quoted strings, comments,
//! block scalars (their body runs to the first line indented less than its first), anchors,
//! tags, aliases, directives, document markers, and flow collections nested across lines.
//!
//! A kind is a leaf (bat's scopes for the token) under the innermost flow collection's meta
//! scope, a fixed set ([`scopes`]): bat's stack nests every enclosing collection's meta scope,
//! which none of the built-in themes tells apart, so the scanner keeps only the innermost.
//! What it doesn't follow: `? ` explicit keys' `meta.block-mapping` and their odd `:`, and the
//! grammar's paths through bad input (syntect's quirks); syntect is a guideline here
//! (`examples/parity.rs` reads how close it comes).
//!
//! The state between lines is one word: an open quoted string, a block scalar's header or its
//! indentation, and the flow collections open (the kinds and pair values of the innermost 16).

use crate::matcher::{Line, M};
use crate::yaml::{
    directive_reserved, directive_tag, directive_yaml, implicit_at, in_end_at, out_end_at,
    tag_handle,
};
use crate::{Out, State, is_space};

/// The meta scopes of the innermost flow collection: none, a sequence, a mapping, a pair's
/// value in either.
const OUTERS: [&[&str]; 5] = [
    &[],
    &["meta.flow-sequence.yaml"],
    &["meta.flow-mapping.yaml"],
    &["meta.flow-sequence.yaml", "meta.flow-pair.value.yaml"],
    &["meta.flow-mapping.yaml", "meta.flow-pair.value.yaml"],
];

/// The leaves: a token's scopes under its outer ones. A kind is `outer * LEAVES.len() + leaf`.
static LEAVES: [&str; 72] = [
    "",
    "string.unquoted.plain.out.yaml",
    "string.unquoted.plain.out.yaml entity.name.tag.yaml",
    "string.unquoted.plain.in.yaml",
    "string.unquoted.plain.in.yaml entity.name.tag.yaml",
    "string.quoted.double.yaml",
    "string.quoted.double.yaml punctuation.definition.string.begin.yaml",
    "string.quoted.double.yaml punctuation.definition.string.end.yaml",
    "string.quoted.double.yaml constant.character.escape.double-quoted.yaml",
    "string.quoted.double.yaml constant.character.escape.double-quoted.newline.yaml",
    "string.quoted.single.yaml",
    "string.quoted.single.yaml punctuation.definition.string.begin.yaml",
    "string.quoted.single.yaml punctuation.definition.string.end.yaml",
    "string.quoted.single.yaml constant.character.escape.single-quoted.yaml",
    "string.unquoted.block.yaml",
    "comment.line.number-sign.yaml",
    "comment.line.number-sign.yaml punctuation.definition.comment.line.number-sign.yaml",
    "punctuation.separator.key-value.mapping.yaml",
    "meta.flow-pair.yaml punctuation.separator.key-value.mapping.yaml",
    "punctuation.separator.sequence.yaml",
    "punctuation.separator.mapping.yaml",
    "punctuation.definition.block.sequence.item.yaml",
    "punctuation.definition.sequence.begin.yaml",
    "punctuation.definition.sequence.end.yaml",
    "punctuation.definition.mapping.begin.yaml",
    "punctuation.definition.mapping.end.yaml",
    "punctuation.definition.key-value.begin.yaml",
    "keyword.control.flow.block-scalar.literal.yaml",
    "keyword.control.flow.block-scalar.folded.yaml",
    "constant.numeric.integer.decimal.indentation-indicator.yaml",
    "storage.modifier.chomping-indicator.yaml",
    "invalid.illegal.expected-comment-or-newline.yaml",
    "entity.other.document.begin.yaml",
    "entity.other.document.end.yaml",
    "meta.directive.yaml",
    "meta.directive.yaml punctuation.definition.directive.begin.yaml",
    "meta.directive.yaml keyword.other.directive.yaml.yaml",
    "meta.directive.yaml constant.numeric.yaml-version.yaml",
    "meta.directive.yaml keyword.other.directive.tag.yaml",
    "meta.directive.yaml storage.type.tag-handle.yaml",
    "meta.directive.yaml support.type.tag-prefix.yaml",
    "meta.directive.yaml support.other.directive.reserved.yaml",
    "meta.directive.yaml string.unquoted.directive-name.yaml",
    "meta.directive.yaml string.unquoted.directive-parameter.yaml",
    "invalid.illegal.unrecognized.yaml",
    "meta.property.yaml keyword.control.property.anchor.yaml punctuation.definition.anchor.yaml",
    "meta.property.yaml entity.name.other.anchor.yaml",
    "meta.property.yaml invalid.illegal.character.anchor.yaml",
    "meta.property.yaml storage.type.tag-handle.yaml",
    "meta.property.yaml invalid.illegal.tag-handle.yaml",
    "keyword.control.flow.alias.yaml punctuation.definition.alias.yaml",
    "variable.other.alias.yaml",
    "invalid.illegal.character.anchor.yaml",
    "constant.language.null.yaml",
    "constant.language.boolean.yaml",
    "constant.numeric.integer.binary.yaml",
    "constant.numeric.integer.binary.yaml punctuation.definition.numeric.base.yaml",
    "constant.numeric.integer.octal.yaml",
    "constant.numeric.integer.octal.yaml punctuation.definition.numeric.base.yaml",
    "constant.numeric.integer.decimal.yaml",
    "constant.numeric.integer.hexadecimal.yaml",
    "constant.numeric.integer.hexadecimal.yaml punctuation.definition.numeric.base.yaml",
    "constant.numeric.integer.other.yaml",
    "constant.numeric.float.decimal.yaml",
    "constant.numeric.float.decimal.yaml punctuation.separator.decimal.yaml",
    "constant.numeric.float.other.yaml",
    "constant.numeric.float.other.yaml punctuation.separator.decimal.yaml",
    "constant.other.timestamp.yaml",
    "constant.other.timestamp.yaml punctuation.separator.date.yaml",
    "constant.other.timestamp.yaml punctuation.separator.time.yaml",
    "constant.language.value.yaml",
    "constant.language.merge.yaml",
];

mod leaf {
    pub const PLAIN_OUT: u16 = 1;
    pub const KEY_OUT: u16 = 2;
    pub const PLAIN_IN: u16 = 3;
    pub const KEY_IN: u16 = 4;
    pub const DOUBLE: u16 = 5;
    pub const DOUBLE_BEGIN: u16 = 6;
    pub const DOUBLE_END: u16 = 7;
    pub const DOUBLE_ESCAPE: u16 = 8;
    pub const DOUBLE_NEWLINE: u16 = 9;
    pub const SINGLE: u16 = 10;
    pub const SINGLE_BEGIN: u16 = 11;
    pub const SINGLE_END: u16 = 12;
    pub const SINGLE_ESCAPE: u16 = 13;
    pub const BLOCK_TEXT: u16 = 14;
    pub const COMMENT: u16 = 15;
    pub const COMMENT_MARK: u16 = 16;
    pub const KEY_SEP: u16 = 17;
    pub const PAIR_SEP: u16 = 18;
    pub const SEQ_SEP: u16 = 19;
    pub const MAP_SEP: u16 = 20;
    pub const SEQ_ITEM: u16 = 21;
    pub const SEQ_BEGIN: u16 = 22;
    pub const SEQ_END: u16 = 23;
    pub const MAP_BEGIN: u16 = 24;
    pub const MAP_END: u16 = 25;
    pub const KEY_BEGIN: u16 = 26;
    pub const LITERAL: u16 = 27;
    pub const FOLDED: u16 = 28;
    pub const INDENTATION: u16 = 29;
    pub const CHOMPING: u16 = 30;
    pub const EXPECTED_COMMENT: u16 = 31;
    pub const DOC_BEGIN: u16 = 32;
    pub const DOC_END: u16 = 33;
    pub const DIRECTIVE: u16 = 34;
    pub const DIRECTIVE_BEGIN: u16 = 35;
    /// The first of the directives' groups' leaves, in the order of `DIRECTIVE_GROUPS`.
    pub const DIRECTIVE_GROUP: u16 = 36;
    pub const UNRECOGNIZED: u16 = 44;
    pub const ANCHOR_MARK: u16 = 45;
    pub const ANCHOR_NAME: u16 = 46;
    pub const ANCHOR_BAD: u16 = 47;
    pub const TAG: u16 = 48;
    pub const TAG_BAD: u16 = 49;
    pub const ALIAS_MARK: u16 = 50;
    pub const ALIAS_NAME: u16 = 51;
    pub const ALIAS_BAD: u16 = 52;
    /// The first implicit type's leaf, in the order of `LEAVES`.
    pub const NULL: u16 = 53;
}

const NLEAVES: u16 = LEAVES.len() as u16;

/// The scopes under `source.yaml` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    let outer = OUTERS
        .get(usize::from(kind / NLEAVES))
        .copied()
        .unwrap_or(&[]);
    let leaf = LEAVES[usize::from(kind % NLEAVES)];
    outer
        .iter()
        .copied()
        .chain(leaf.split(' ').filter(|s| !s.is_empty()))
        .collect()
}

/// What a line starts in.
const BLOCK: u64 = 0;
const DOUBLE: u64 = 1;
const SINGLE: u64 = 2;
/// After a block scalar's `|` or `>`, before its first line sets the indentation.
const HEADER: u64 = 3;
/// In a block scalar's body.
const BODY: u64 = 4;

/// How many flow collections' kinds and pair values the state keeps; one nested deeper reads
/// as a sequence's, outside a pair's value.
const KEPT: usize = 16;

/// Lex `b`, a line without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let w = state.word();
    let mut s = Scan {
        b,
        mode: w & 7,
        indent: (w >> 8) as u16 as usize,
        depth: (w >> 24) as u8 as usize,
        maps: (w >> 32) as u16,
        values: (w >> 48) as u16,
        out,
    };
    s.lex();
    let w = s.mode
        | (s.indent.min(usize::from(u16::MAX)) as u64) << 8
        | (s.depth.min(255) as u64) << 24
        | u64::from(s.maps) << 32
        | u64::from(s.values) << 48;
    state.set_word(w);
}

struct Scan<'a, 'b, 'c> {
    b: &'a [u8],
    mode: u64,
    /// A block scalar body's indentation.
    indent: usize,
    /// Flow collections open.
    depth: usize,
    /// By level, from the outermost: a mapping (1) or a sequence (0).
    maps: u16,
    /// By level: in a pair's value, after its `:`.
    values: u16,
    out: &'b mut Out<'c>,
}

/// The bytes a plain scalar can't start with (or only before a character, for `?`, `:`, `-`).
fn is_indicator(c: u8) -> bool {
    matches!(
        c,
        b'-' | b'?'
            | b':'
            | b','
            | b'['
            | b']'
            | b'{'
            | b'}'
            | b'#'
            | b'&'
            | b'*'
            | b'!'
            | b'|'
            | b'>'
            | b'\''
            | b'"'
            | b'%'
            | b'@'
            | b'`'
    )
}

fn is_flow(c: u8) -> bool {
    matches!(c, b'[' | b']' | b'{' | b'}' | b',')
}

fn is_blank(c: u8) -> bool {
    c == b' ' || c == b'\t'
}

/// The first bytes of an implicit type: null, a boolean, a number, a timestamp, `=`, `<<`.
fn may_be_implicit(c: u8) -> bool {
    matches!(
        c,
        b'~' | b'n'
            | b'N'
            | b'y'
            | b'Y'
            | b't'
            | b'T'
            | b'f'
            | b'F'
            | b'o'
            | b'O'
            | b'+'
            | b'-'
            | b'.'
            | b'0'..=b'9' | b'=' | b'<'
    )
}

impl Scan<'_, '_, '_> {
    fn nl(&self) -> usize {
        self.b.len()
    }

    /// Where a run of bytes in `set` from `i` ends.
    fn run(&self, mut i: usize, set: impl Fn(u8) -> bool) -> usize {
        while i < self.b.len() && set(self.b[i]) {
            i += 1;
        }
        i
    }

    /// `(?=\s|$)` at `i`.
    fn space_or_eol(&self, i: usize) -> bool {
        self.b.get(i).is_none_or(|&c| is_space(c))
    }

    fn is_map(&self) -> bool {
        let i = self.depth - 1;
        i < KEPT && self.maps >> i & 1 == 1
    }

    fn in_value(&self) -> bool {
        let i = self.depth - 1;
        i < KEPT && self.values >> i & 1 == 1
    }

    fn set_value(&mut self, on: bool) {
        let i = self.depth - 1;
        if i < KEPT {
            self.values = self.values & !(1 << i) | u16::from(on) << i;
        }
    }

    /// The innermost collection's outer scopes, as an index into [`OUTERS`].
    fn outer(&self) -> u16 {
        match self.depth {
            0 => 0,
            _ => 1 + u16::from(self.is_map()) + 2 * u16::from(self.in_value()),
        }
    }

    /// Bytes `s..e` are `leaf` in the innermost collection.
    fn put(&mut self, s: usize, e: usize, leaf: u16) {
        let kind = self.outer() * NLEAVES + leaf;
        self.out.push_merged(s, e, kind);
    }

    fn lex(&mut self) {
        let nl = self.nl();
        let p = match self.mode {
            DOUBLE => match self.double_body(0) {
                Some(e) => e,
                None => return,
            },
            SINGLE => match self.single_body(0) {
                Some(e) => e,
                None => return,
            },
            HEADER => match self.b.first() {
                None => return,
                // the first line indented sets the block's indentation
                Some(b' ') => {
                    self.indent = self.run(0, |c| c == b' ');
                    self.mode = BODY;
                    self.put(0, nl, leaf::BLOCK_TEXT);
                    return;
                }
                Some(&c) if !is_space(c) => {
                    self.mode = BLOCK;
                    0
                }
                // a tab or another space first: a comment, or not what a block scalar takes
                Some(_) => {
                    let q = self.run(0, is_blank);
                    match self.b.get(q) {
                        Some(b'#') => self.comment(q),
                        _ => self.put(0, nl, leaf::EXPECTED_COMMENT),
                    }
                    return;
                }
            },
            BODY => {
                let indented =
                    self.b.len() >= self.indent && self.b[..self.indent].iter().all(|&c| c == b' ');
                if indented || self.b.iter().all(|&c| is_space(c)) {
                    self.put(0, nl, leaf::BLOCK_TEXT);
                    return;
                }
                self.mode = BLOCK;
                0
            }
            _ => 0,
        };
        self.tokens(p);
    }

    /// The tokens from `p` to the line's end, or to a string or block scalar left open.
    fn tokens(&mut self, mut p: usize) {
        let nl = self.nl();
        while p < nl {
            let c = self.b[p];
            if is_blank(c) {
                let q = self.run(p, is_blank);
                if self.b.get(q) == Some(&b'#') {
                    return self.comment(q);
                }
                p = q;
                continue;
            }
            if c == b'#' && p == 0 {
                return self.comment(0);
            }
            let next = match self.depth {
                0 => self.block_at(p),
                _ => self.flow_at(p),
            };
            match next {
                Some(q) => p = q,
                // a string or block scalar runs on to the next line
                None => return,
            }
        }
    }

    /// `#` at `q` and the rest of the line.
    fn comment(&mut self, q: usize) {
        self.put(q, q + 1, leaf::COMMENT_MARK);
        self.put(q + 1, self.nl(), leaf::COMMENT);
    }

    /// One token at `p` out of flow collections: where the next starts.
    fn block_at(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        match b[p] {
            b'!' | b'&' => return Some(self.property(p)),
            b'%' if p == 0 => return Some(self.directive()),
            b'-' if p == 0 && b.starts_with(b"---") => {
                self.put(0, 3, leaf::DOC_BEGIN);
                return Some(3);
            }
            b'.' if p == 0 && b.starts_with(b"...") => {
                self.put(0, 3, leaf::DOC_END);
                return Some(3);
            }
            c @ (b'|' | b'>') => {
                let mark = if c == b'|' {
                    leaf::LITERAL
                } else {
                    leaf::FOLDED
                };
                self.block_scalar(p, mark);
                return None;
            }
            b'-' if self.space_or_eol(p + 1) => {
                self.put(p, p + 1, leaf::SEQ_ITEM);
                return Some(p + 1);
            }
            b'?' => {
                self.put(p, p + 1, leaf::KEY_BEGIN);
                return Some(p + 1);
            }
            b':' if self.space_or_eol(p + 1) => {
                self.put(p, p + 1, leaf::KEY_SEP);
                return Some(p + 1);
            }
            _ => {}
        }
        if let Some(e) = self.plain(p, false, true) {
            return Some(e);
        }
        self.flow_node(p)
    }

    /// One token at `p` in a flow collection.
    fn flow_at(&mut self, p: usize) -> Option<usize> {
        let c = self.b[p];
        if c == b'!' || c == b'&' {
            return Some(self.property(p));
        }
        if self.in_value() {
            // a pair's value ends before `,`, `]` or `}`, which the collection then takes
            if matches!(c, b',' | b']' | b'}') {
                self.set_value(false);
                return Some(p);
            }
            if let Some(e) = self.plain(p, true, false) {
                return Some(e);
            }
            return self.flow_node(p);
        }
        match c {
            b'}' if self.is_map() => return Some(self.close(p, leaf::MAP_END)),
            b']' if !self.is_map() => return Some(self.close(p, leaf::SEQ_END)),
            b',' => {
                let sep = if self.is_map() {
                    leaf::MAP_SEP
                } else {
                    leaf::SEQ_SEP
                };
                self.put(p, p + 1, sep);
                return Some(p + 1);
            }
            b'?' => {
                self.put(p, p + 1, leaf::KEY_BEGIN);
                return Some(p + 1);
            }
            _ => {}
        }
        if let Some(e) = self.plain(p, true, true) {
            return Some(e);
        }
        if c == b':' && self.b.get(p + 1).is_none_or(|&n| is_space(n) || is_flow(n)) {
            self.put(p, p + 1, leaf::PAIR_SEP);
            self.set_value(true);
            return Some(p + 1);
        }
        self.flow_node(p)
    }

    /// An alias, a collection opened or a quoted string at `p`; else nothing matches here and
    /// the next character is tried.
    fn flow_node(&mut self, p: usize) -> Option<usize> {
        match self.b[p] {
            b'*' => Some(self.alias(p).unwrap_or(p + 1)),
            b'[' => Some(self.open(p, false)),
            b'{' => Some(self.open(p, true)),
            b'"' => {
                self.put(p, p + 1, leaf::DOUBLE_BEGIN);
                self.double_body(p + 1)
            }
            b'\'' => {
                self.put(p, p + 1, leaf::SINGLE_BEGIN);
                self.single_body(p + 1)
            }
            // what nothing takes stays plain; a character at a time keeps to their boundaries
            _ => Some(
                p + 1
                    + self.b[p + 1..]
                        .iter()
                        .take_while(|&&c| c & 0xc0 == 0x80)
                        .count(),
            ),
        }
    }

    fn open(&mut self, p: usize, map: bool) -> usize {
        if self.depth < KEPT {
            let bit = 1 << self.depth;
            self.maps = if map {
                self.maps | bit
            } else {
                self.maps & !bit
            };
            self.values &= !bit;
        }
        self.depth = (self.depth + 1).min(255);
        self.put(
            p,
            p + 1,
            if map {
                leaf::MAP_BEGIN
            } else {
                leaf::SEQ_BEGIN
            },
        );
        p + 1
    }

    fn close(&mut self, p: usize, end: u16) -> usize {
        self.put(p, p + 1, end);
        self.depth -= 1;
        p + 1
    }

    /// A plain scalar at `p`, or an implicit type, `key` saying whether one can be a key here:
    /// its end, or `None` when none starts at `p`.
    fn plain(&mut self, p: usize, flow: bool, key: bool) -> Option<usize> {
        let b = self.b;
        let c = b[p];
        let start = if !is_space(c) && !is_indicator(c) {
            p + 1
        } else if matches!(c, b'?' | b':' | b'-')
            && b.get(p + 1)
                .is_some_and(|&n| !is_space(n) && !(flow && is_flow(n)))
        {
            p + 2
        } else {
            return None;
        };
        let (e, colon) = self.plain_end(start, flow);
        // an implicit type ends where its look-ahead holds, at the scalar's end, but for a
        // timestamp's blanks and zone; one that starts with a letter is a word of at most five
        if may_be_implicit(c) && (!c.is_ascii_alphabetic() || e - p <= 5) {
            let l = Line { b };
            let end: fn(&Line, usize) -> bool = if flow { in_end_at } else { out_end_at };
            if let Some(m) = implicit_at(&l, p, end) {
                self.implicit(&m);
                return Some(m.e as usize);
            }
        }
        let leaf = match (flow, key && colon) {
            (false, false) => leaf::PLAIN_OUT,
            (false, true) => leaf::KEY_OUT,
            (true, false) => leaf::PLAIN_IN,
            (true, true) => leaf::KEY_IN,
        };
        self.put(p, e, leaf);
        Some(e)
    }

    /// Where a plain scalar from `i` ends: before `\s*$`, `\s+#` or `\s*:(\s|$)`, and in flow
    /// collections also `\s*:[,[]{}]` and `\s*[,[]{}]`; and whether that was a `: `, which
    /// makes it a key.
    fn plain_end(&self, mut i: usize, flow: bool) -> (usize, bool) {
        let b = self.b;
        let nl = self.nl();
        // where a plain scalar can end: `\s`, `:`, and in flow collections `,[]{}`
        static STOPS: [u8; 256] = {
            let mut t = [0; 256];
            let mut c = 0;
            while c < 256 {
                let b = c as u8;
                let space = matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r');
                if space || b == b':' {
                    t[c] = 3;
                } else if matches!(b, b'[' | b']' | b'{' | b'}' | b',') {
                    t[c] = 2;
                }
                c += 1;
            }
            t
        };
        let mask = if flow { 2 } else { 1 };
        loop {
            let Some(k) = b
                .get(i..)
                .and_then(|r| r.iter().position(|&c| STOPS[usize::from(c)] & mask != 0))
            else {
                return (nl, false);
            };
            let k = i + k;
            let c = b[k];
            if is_space(c) {
                let j = self.run(k, is_space);
                match b.get(j) {
                    None | Some(b'#') => return (k, false),
                    Some(b':') if self.space_or_eol(j + 1) => return (k, true),
                    Some(b':') if flow && b.get(j + 1).is_some_and(|&n| is_flow(n)) => {
                        return (k, false);
                    }
                    Some(&n) if flow && is_flow(n) => return (k, false),
                    _ => i = j,
                }
            } else if c == b':' {
                if self.space_or_eol(k + 1) {
                    return (k, true);
                }
                if flow && b.get(k + 1).is_some_and(|&n| is_flow(n)) {
                    return (k, false);
                }
                i = k + 1;
            } else {
                return (k, false);
            }
        }
    }

    /// An implicit type's match: the type over it, its separators within.
    fn implicit(&mut self, m: &M) {
        // the type's group and the separators', by their number in bat's regex
        let leaf = |g: u8| -> Option<(u16, bool)> {
            let n = leaf::NULL;
            Some(match g {
                1 => (n, true),
                2 => (n + 1, true),
                4 => (n + 2, true),
                5 => (n + 3, false),
                6 => (n + 4, true),
                7 => (n + 5, false),
                8 => (n + 6, true),
                9 => (n + 7, true),
                10 => (n + 8, false),
                11 => (n + 9, true),
                13 => (n + 10, true),
                14 => (n + 11, false),
                15 | 17 | 19 => (n + 12, true),
                16 | 18 | 20 => (n + 13, false),
                21 => (n + 14, true),
                22..=25 => (n + 15, false),
                26..=29 => (n + 16, false),
                30 => (n + 17, true),
                31 => (n + 18, true),
                _ => return None,
            })
        };
        let mut main = leaf::NULL;
        let mut seps = [(0usize, 0usize, 0u16); 8];
        let mut n = 0;
        for (g, s, e) in m.groups() {
            match leaf(g) {
                Some((l, true)) => main = l,
                Some((l, false)) if n < seps.len() => {
                    seps[n] = (s, e, l);
                    n += 1;
                }
                _ => {}
            }
        }
        let seps = &mut seps[..n];
        seps.sort_unstable_by_key(|s| s.0);
        let mut at = m.s as usize;
        for &(s, e, l) in seps.iter() {
            self.put(at, s, main);
            self.put(s, e, l);
            at = e;
        }
        self.put(at, m.e as usize, main);
    }

    /// A double-quoted string's text from `i`: where its closing quote ends, or `None` when it
    /// runs on past the line.
    fn double_body(&mut self, mut i: usize) -> Option<usize> {
        let b = self.b;
        let nl = self.nl();
        let mut text = i;
        while let Some(k) = b[i..].iter().position(|&c| c == b'"' || c == b'\\') {
            let k = i + k;
            if b[k] == b'"' {
                self.put(text, k, leaf::DOUBLE);
                self.put(k, k + 1, leaf::DOUBLE_END);
                self.mode = BLOCK;
                return Some(k + 1);
            }
            let hex = |at: usize, n: usize| {
                b.get(at..at + n)
                    .is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit))
            };
            let n = match b.get(k + 1) {
                None => {
                    // `\` at the end: the newline escaped
                    self.put(text, k, leaf::DOUBLE);
                    self.put(k, nl, leaf::DOUBLE_NEWLINE);
                    self.mode = DOUBLE;
                    return None;
                }
                Some(c) if b"0abtnvfre \"/\\N_Lp".contains(c) => 2,
                Some(b'x') if hex(k + 2, 2) => 4,
                Some(b'u') if hex(k + 2, 4) => 6,
                Some(b'U') if hex(k + 2, 8) => 10,
                _ => 0,
            };
            if n > 0 {
                self.put(text, k, leaf::DOUBLE);
                self.put(k, k + n, leaf::DOUBLE_ESCAPE);
                text = k + n;
                i = k + n;
            } else {
                i = k + 1;
            }
        }
        self.put(text, nl, leaf::DOUBLE);
        self.mode = DOUBLE;
        None
    }

    /// A single-quoted string's text from `i`, as [`Scan::double_body`].
    fn single_body(&mut self, mut i: usize) -> Option<usize> {
        let b = self.b;
        let mut text = i;
        while let Some(k) = b[i..].iter().position(|&c| c == b'\'') {
            let k = i + k;
            if b.get(k + 1) == Some(&b'\'') {
                self.put(text, k, leaf::SINGLE);
                self.put(k, k + 2, leaf::SINGLE_ESCAPE);
                text = k + 2;
                i = k + 2;
                continue;
            }
            self.put(text, k, leaf::SINGLE);
            self.put(k, k + 1, leaf::SINGLE_END);
            self.mode = BLOCK;
            return Some(k + 1);
        }
        self.put(text, self.nl(), leaf::SINGLE);
        self.mode = SINGLE;
        None
    }

    /// A block scalar's header at `p`: `|` or `>`, its indicators, and the rest of the line,
    /// which takes only a comment.
    fn block_scalar(&mut self, p: usize, mark: u16) {
        let b = self.b;
        self.put(p, p + 1, mark);
        let mut e = p + 1;
        if b.get(e).is_some_and(|c| (b'1'..=b'9').contains(c)) {
            self.put(e, e + 1, leaf::INDENTATION);
            e += 1;
        }
        if b.get(e).is_some_and(|&c| c == b'-' || c == b'+') {
            self.put(e, e + 1, leaf::CHOMPING);
            e += 1;
        }
        self.mode = HEADER;
        let q = self.run(e, is_blank);
        if q > e && b.get(q) == Some(&b'#') {
            self.comment(q);
        } else if e < self.nl() {
            self.put(e, self.nl(), leaf::EXPECTED_COMMENT);
        }
    }

    /// An anchor or a tag at `p`: where it ends.
    fn property(&mut self, p: usize) -> usize {
        let b = self.b;
        let non_space = |s: &Self, i: usize| s.run(i, |c| !is_space(c));
        if b[p] == b'&' {
            let name = self.run(p + 1, |c| !is_space(c) && !b"[]/{},".contains(&c));
            if name > p + 1 {
                let rest = non_space(self, name);
                self.put(p, p + 1, leaf::ANCHOR_MARK);
                self.put(p + 1, name, leaf::ANCHOR_NAME);
                self.put(name, rest, leaf::ANCHOR_BAD);
                return rest;
            }
        } else {
            let l = Line { b };
            if let Some(m) = tag_handle(&l, p).filter(|m| m.s as usize == p) {
                self.put(p, m.e as usize, leaf::TAG);
                return m.e as usize;
            }
        }
        let e = non_space(self, p);
        self.put(p, e, leaf::TAG_BAD);
        e
    }

    /// `(\*)([^\s\[\]/{},]+)([^\s\]},]\S*)?` at `p`.
    fn alias(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let name = self.run(p + 1, |c| !is_space(c) && !b"[]/{},".contains(&c));
        if name == p + 1 {
            return None;
        }
        self.put(p, p + 1, leaf::ALIAS_MARK);
        self.put(p + 1, name, leaf::ALIAS_NAME);
        if b.get(name)
            .is_some_and(|&c| !is_space(c) && !b"]},".contains(&c))
        {
            let rest = self.run(name, |c| !is_space(c));
            self.put(name, rest, leaf::ALIAS_BAD);
            return Some(rest);
        }
        Some(name)
    }

    /// A directive, `%` at the line's start: its name and parameters, then anything else on
    /// the line but a comment, unrecognized.
    fn directive(&mut self) -> usize {
        let b = self.b;
        self.put(0, 1, leaf::DIRECTIVE_BEGIN);
        let l = Line { b };
        let found = [directive_yaml, directive_tag, directive_reserved]
            .iter()
            .enumerate()
            .find_map(|(n, f)| f(&l, 1).filter(|m| m.s == 1).map(|m| (n, m)));
        let mut p = 1;
        if let Some((n, m)) = found {
            let mut at = 1;
            let mut groups: Vec<_> = m.groups().collect();
            groups.sort_unstable_by_key(|g| g.1);
            for (g, s, e) in groups {
                self.put(at, s, leaf::DIRECTIVE);
                // `YAML` takes two groups, `TAG` and the reserved three
                let base = [0, 2, 5][n];
                self.put(s, e, leaf::DIRECTIVE_GROUP + base + u16::from(g) - 1);
                at = e;
            }
            p = m.e as usize;
            self.put(at, p, leaf::DIRECTIVE);
        }
        // `(?=$|[ \t]+($|#))` ends it; before that, `\S+` is unrecognized
        loop {
            let q = self.run(p, is_blank);
            if q == self.nl() || (q > p && b[q] == b'#') {
                return p;
            }
            let k = self.run(p, is_space);
            if k == self.nl() {
                return p;
            }
            let e = self.run(k, |c| !is_space(c));
            self.put(k, e, leaf::UNRECOGNIZED);
            p = e;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Lang;
    use crate::tests::marked;

    fn yaml(lines: &[&'static str]) -> Vec<(&'static str, &'static str)> {
        marked(Lang::Yaml, lines)
    }

    #[test]
    fn keys_values_and_comments() {
        assert_eq!(
            yaml(&["name: ada # why", "- on: yes", "  k: 12"]),
            [
                (
                    "name",
                    "string.unquoted.plain.out.yaml entity.name.tag.yaml"
                ),
                (":", "punctuation.separator.key-value.mapping.yaml"),
                ("ada", "string.unquoted.plain.out.yaml"),
                (
                    "#",
                    "comment.line.number-sign.yaml punctuation.definition.comment.line.number-sign.yaml"
                ),
                (" why", "comment.line.number-sign.yaml"),
                ("-", "punctuation.definition.block.sequence.item.yaml"),
                ("on", "constant.language.boolean.yaml"),
                (":", "punctuation.separator.key-value.mapping.yaml"),
                ("yes", "constant.language.boolean.yaml"),
                ("k", "string.unquoted.plain.out.yaml entity.name.tag.yaml"),
                (":", "punctuation.separator.key-value.mapping.yaml"),
                ("12", "constant.numeric.integer.decimal.yaml"),
            ]
        );
    }

    #[test]
    fn flow_collections_run_across_lines() {
        assert_eq!(
            yaml(&["a: {b: [1,", " 'x']}"]),
            [
                ("a", "string.unquoted.plain.out.yaml entity.name.tag.yaml"),
                (":", "punctuation.separator.key-value.mapping.yaml"),
                (
                    "{",
                    "meta.flow-mapping.yaml punctuation.definition.mapping.begin.yaml"
                ),
                (
                    "b",
                    "meta.flow-mapping.yaml string.unquoted.plain.in.yaml entity.name.tag.yaml"
                ),
                (
                    ":",
                    "meta.flow-mapping.yaml meta.flow-pair.yaml punctuation.separator.key-value.mapping.yaml"
                ),
                (
                    "[",
                    "meta.flow-sequence.yaml punctuation.definition.sequence.begin.yaml"
                ),
                (
                    "1",
                    "meta.flow-sequence.yaml constant.numeric.integer.decimal.yaml"
                ),
                (
                    ",",
                    "meta.flow-sequence.yaml punctuation.separator.sequence.yaml"
                ),
                (
                    "'",
                    "meta.flow-sequence.yaml string.quoted.single.yaml punctuation.definition.string.begin.yaml"
                ),
                ("x", "meta.flow-sequence.yaml string.quoted.single.yaml"),
                (
                    "'",
                    "meta.flow-sequence.yaml string.quoted.single.yaml punctuation.definition.string.end.yaml"
                ),
                (
                    "]",
                    "meta.flow-sequence.yaml punctuation.definition.sequence.end.yaml"
                ),
                (
                    "}",
                    "meta.flow-mapping.yaml punctuation.definition.mapping.end.yaml"
                ),
            ]
        );
    }

    #[test]
    fn block_scalars_end_at_a_line_indented_less() {
        assert_eq!(
            yaml(&["run: |", "  cargo test", "", "    more", "next: x"]),
            [
                ("run", "string.unquoted.plain.out.yaml entity.name.tag.yaml"),
                (":", "punctuation.separator.key-value.mapping.yaml"),
                ("|", "keyword.control.flow.block-scalar.literal.yaml"),
                ("  cargo test", "string.unquoted.block.yaml"),
                ("    more", "string.unquoted.block.yaml"),
                (
                    "next",
                    "string.unquoted.plain.out.yaml entity.name.tag.yaml"
                ),
                (":", "punctuation.separator.key-value.mapping.yaml"),
                ("x", "string.unquoted.plain.out.yaml"),
            ]
        );
    }
}
