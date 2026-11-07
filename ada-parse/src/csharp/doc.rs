//! A doc comment's turns in a run of their own (X-025's fourth stage): while the top is
//! `documentation` or a tag's attributes, each turn [`Scan::run`] would take is taken here,
//! without its dispatch, its spans emitted inline. The turns are bat's: the line's `///`, a
//! tag's start and end, an attribute's name, its value, the text between, and a tag's blanks
//! (no pattern of a tag's attributes starts at one, so each is a step of a character, passed
//! here as a run). Anything rarer (a line that ends the comment, a to-do or a link, a name
//! past ASCII) is left to the contexts' arms ([`Scan::doc`], [`Scan::doc_tag`]).

use super::c::*;
use super::*;
use crate::clike::{DIGIT, SPACE};

impl<K: Lex> Scan<'_, K> {
    /// The turns from `p` while the top is `documentation` or a tag's attributes, as
    /// [`Scan::run`]'s loop would take them (its bookkeeping too). Returns where the scan is,
    /// or `None` when it took no turn. It stops at a turn that takes nothing, a turn of the
    /// contexts' arms that matches nothing (left to [`Scan::run`], which takes it again), a
    /// top it doesn't run, or past the line's end.
    #[inline(never)]
    pub(super) fn doc_run(&mut self, mut p: usize) -> Option<usize> {
        let (b, n) = (self.b, self.b.len());
        let from = p;
        loop {
            let top = self.top();
            let q = if top == DOC {
                if p > n {
                    break;
                }
                self.looped = false;
                self.doc_turn(p)
            } else if top == DOC_TAG {
                // `#anon_documentation_0` has no pattern that starts at a blank: each is a
                // step of a character (`advance`), which sets neither `step` nor `stuck`
                let mut s = p;
                while s < n && matches!(b[s], b' ' | b'\t') {
                    s += 1;
                }
                if s > p {
                    self.looped = false;
                    p = s;
                    continue;
                }
                self.looped = false;
                self.tag_turn(p)
            } else {
                break;
            };
            let Some(q) = q else { break };
            if self.halt {
                return Some(q);
            }
            // `run`'s bookkeeping; a turn that takes nothing goes back to it
            self.step = q;
            if q == p {
                self.stuck += 1;
                return Some(q);
            }
            self.stuck = 0;
            p = q;
        }
        (p > from).then_some(p)
    }

    /// `documentation`'s turn at `p` (on the line or its `\n`): [`Scan::doc`]'s, its usual
    /// cases here.
    #[inline(always)]
    fn doc_turn(&mut self, p: usize) -> Option<usize> {
        let (b, n) = (self.b, self.b.len());
        let c = self.ch(p);
        if c == b'<' && p < n {
            // `(</)({{name}})(>)`, then `(<)({{name}})`: a name of ASCII has one end
            if self.ch(p + 1) == b'/' {
                if let Some(e) = self.ascii_name(p + 2)
                    && e < n
                    && b[e] == b'>'
                {
                    self.put_doc(p, p + 2, TAG_BEGIN);
                    self.put_doc(p + 2, e, TAG_NAME_END);
                    return Some(self.put_doc(e, e + 1, TAG_END));
                }
            } else if let Some(e) = self.ascii_name(p + 1) {
                if self.depth >= SYNTECT_DEPTH {
                    return self.doc(p);
                }
                self.put_doc(p, p + 1, TAG_BEGIN);
                self.put_doc(p + 1, e, TAG_NAME_BEGIN);
                self.push(DOC_TAG);
                return Some(e);
            }
            return self.doc(p);
        }
        if p == 0 {
            let s = self.ind;
            if self.lit(s, b"///") {
                return Some(self.put_doc(s, s + 3, DOC_PUNCT));
            }
            return self.doc(p);
        }
        if matches!(c, b't' | b'T' | b'h' | b'H' | b'x' | b'X' | b'/') && self.marks(p) {
            return self.doc(p);
        }
        // the text's turns, one after another while each would take text, as `doc`'s: a run of
        // word bytes and blanks (`[\w\s]+`, the line's `\n` in it when the run reaches it), or
        // a character of its own (`.`); a turn after a run starts at a byte that isn't a word's
        // or a blank, so a to-do's or a link's letters start one only after a character of its
        // own
        let mut e = p;
        loop {
            // the run: ASCII's word bytes and blanks on the core's classes, a character past
            // ASCII that is either by its own
            let s = e;
            loop {
                while e < n && CLASS[usize::from(b[e])] & (WORD | SPACE) != 0 {
                    e += 1;
                }
                if e < n && b[e] >= 0x80 && (self.word_at(e) || self.space(e)) {
                    e = self.next(e);
                    continue;
                }
                break;
            }
            if e >= n {
                e = n + 1;
                break;
            }
            let c = b[e];
            if e > s && (c == b'<' || (c == b'/' && self.marks(e))) {
                break;
            }
            // a character of its own
            e = self.next(e);
            if e >= n {
                break;
            }
            let c = b[e];
            if c == b'<'
                || (matches!(c, b't' | b'T' | b'h' | b'H' | b'x' | b'X' | b'/') && self.marks(e))
            {
                break;
            }
        }
        self.put_doc(p, e, DOC_TEXT);
        Some(e)
    }

    /// `#anon_documentation_0`'s turn at `p`, not a blank: [`Scan::doc_tag`]'s, its usual
    /// cases here.
    #[inline(always)]
    fn tag_turn(&mut self, p: usize) -> Option<usize> {
        let (b, n) = (self.b, self.b.len());
        if p >= n {
            return self.doc_tag(p);
        }
        match b[p] {
            // `({{name}})\s*(=)`: a name of ASCII has one end
            c if CLASS[usize::from(c)] & (WORD | DIGIT) == WORD => {
                if let Some(e) = self.ascii_name(p) {
                    let s = self.sp(e);
                    if s < n && b[s] == b'=' {
                        self.put_doc(p, e, ATTRIBUTE_NAME);
                        return Some(self.put_doc(s, s + 1, ARG_VALUE_SEP));
                    }
                }
                self.doc_tag(p)
            }
            b'/' if p + 1 < n && b[p + 1] == b'>' => {
                let q = self.put_doc(p, p + 2, TAG_END);
                self.pop();
                Some(q)
            }
            b'>' => {
                let q = self.put_doc(p, p + 1, TAG_END);
                self.pop();
                Some(q)
            }
            b'"' => match b[p + 1..n].iter().position(|&c| c == b'"') {
                Some(k) => Some(self.put_doc(p, p + k + 2, DQ)),
                None => self.doc_tag(p),
            },
            _ => self.doc_tag(p),
        }
    }

    /// Bytes `s..e` are `leaf` in the top context (`e` past the line is its end): [`Scan::put`]
    /// inline, the text before it (the doc comment's) too, for the usual case (`s..e` at or
    /// past where the spans end, room for two), the rest [`Scan::put_kind`]'s. Returns `e`.
    #[inline(always)]
    fn put_doc(&mut self, s: usize, e: usize, leaf: u16) -> usize {
        let k = self.cur() | leaf;
        let at = self.at;
        let end = e.min(self.b.len());
        if s < at || end < s || self.k.full(2) {
            self.put_kind(s, e, k);
            return e;
        }
        if s > at && self.text != PLAIN {
            self.emit(at, s, self.text);
        }
        if end > s && k != PLAIN {
            self.emit(s, end, k);
        }
        self.at = end;
        e
    }
}
