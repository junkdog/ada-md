//! The directive lexer: a line whose first token is `#`, its word, then what the word takes (an
//! include's path, `#define`'s name, parameters and body, `#error`'s message, `defined` in
//! `#if`), the rest of the line plain in [`PP_TEXT`] but for comments; a line ending in `\`
//! goes on on the next. `#if 0` opens a disabled branch, a comment to its `#else`, `#elif` or
//! `#endif` (the `#if`s inside counted). A directive doesn't move the code around it: what the
//! token before was, what a `{` would open, are kept across it.

use super::*;

impl<K: Sink> Scan<'_, '_, K> {
    /// The directive whose `#` is at `h`, the line's first token.
    #[inline(never)]
    pub(super) fn directive(&mut self, h: usize) {
        let b = self.b;
        let kept = self.kept();
        self.at = h;
        let mut s = h + 1;
        while s < b.len() && CC[usize::from(b[s])] & SP != 0 {
            s += 1;
        }
        let mut e = s;
        while e < b.len() && (b[e].is_ascii_alphabetic() || b[e] == b'_') {
            e += 1;
        }
        let w = &b[s..e];
        let e = e.max(h + 1);
        match w {
            b"if" | b"ifdef" | b"ifndef" => self.ifd = self.depth.saturating_add(1),
            b"else" | b"elif" | b"elifdef" | b"elifndef" => self.branch(),
            b"endif" => self.ifd = 0,
            _ => {}
        }
        match w {
            b"include" | b"include_next" | b"import" => {
                self.put::<true>(h, e, IMPORT_INCLUDE);
                self.include(e);
            }
            b"define" => {
                self.put::<true>(h, e, IMPORT_DEFINE);
                self.define(e);
            }
            b"error" | b"warning" => {
                self.put::<true>(h, e, IMPORT_ERROR);
                self.pp = PP_MESSAGE;
                self.pp_rest(e);
            }
            b"else" => {
                self.put::<true>(h, e, IMPORT_ELSE);
                self.pp = PP_REST;
                self.pp_rest(e);
            }
            b"if" => {
                self.put::<true>(h, e, IMPORT);
                self.pp = PP_REST;
                // a number right after `#if` is one (bat's); `#if 0` opens a disabled branch
                let t = skip_blanks(b, e);
                let mut d = t;
                while d < b.len() && b[d].is_ascii_digit() {
                    d += 1;
                }
                if d > t && b.get(d).is_none_or(|&c| CC[usize::from(c)] & NW == 0) {
                    self.put::<true>(t, d, DEC);
                    self.pp_rest(d);
                    if &b[t..d] == b"0" && self.pp == PP_NONE && self.mode == M_CODE {
                        (self.mode, self.aux) = (M_DISABLED, 0);
                    }
                } else {
                    self.pp_rest(e);
                }
            }
            _ => {
                self.put::<true>(h, e, IMPORT);
                self.pp = PP_REST;
                self.pp_rest(e);
            }
        }
        self.restore(kept);
    }

    /// An include's path after its word, at `e`: `<…>` or `"…"`, then the line's rest.
    fn include(&mut self, e: usize) {
        let b = self.b;
        let s = skip_blanks(b, e);
        let (close, begin, text, end) = match b.get(s) {
            Some(b'<') => (b'>', INC_LT_BEGIN, INC_LT, INC_LT_END),
            Some(b'"') => (b'"', INC_DQ_BEGIN, INC_DQ, INC_DQ_END),
            _ => {
                self.pp = PP_REST;
                return self.pp_rest(e);
            }
        };
        self.put::<true>(s, s + 1, begin);
        let q = prim_find(b, s + 1, close);
        if q > s + 1 {
            self.put::<true>(s + 1, q, text);
        }
        let mut r = q;
        if q < b.len() {
            self.put::<true>(q, q + 1, end);
            r = q + 1;
        }
        self.pp = PP_REST;
        self.pp_rest(r);
    }

    /// `#define`'s name at `e` (a function-like macro's parameters after it), then its body.
    fn define(&mut self, e: usize) {
        let b = self.b;
        let s = skip_blanks(b, e);
        let mut n = s;
        if n < b.len() && CC[usize::from(b[n])] & NS0 != 0 {
            n += 1;
            while n < b.len() && CC[usize::from(b[n])] & NW != 0 {
                n += 1;
            }
        }
        let mut p = n;
        if n > s {
            if b.get(n) == Some(&b'(') {
                self.put::<true>(s, n, PP_FUNCTION);
                p = self.macro_params(n);
            } else {
                self.put::<true>(s, n, PP_CONSTANT);
            }
        }
        self.pp = PP_BODY;
        self.body_start();
        self.code::<true>(p);
        self.end_pp();
    }

    /// A function-like macro's parameters, their `(` at `p`: where they end.
    fn macro_params(&mut self, p: usize) -> usize {
        let b = self.b;
        self.put::<true>(p, p + 1, GROUP_BEGIN);
        let mut q = p + 1;
        while q < b.len() {
            let c = b[q];
            let k = CC[usize::from(c)];
            if k & SP != 0 {
                q += 1;
            } else if k & NS0 != 0 {
                let mut e = q + 1;
                while e < b.len() && CC[usize::from(b[e])] & NW != 0 {
                    e += 1;
                }
                self.put::<true>(q, e, PARAMETER);
                q = e;
            } else {
                match c {
                    b',' => self.put::<true>(q, q + 1, SEPARATOR),
                    b')' => {
                        self.put::<true>(q, q + 1, GROUP_END);
                        return q + 1;
                    }
                    b'.' if b.get(q + 1) == Some(&b'.') && b.get(q + 2) == Some(&b'.') => {
                        self.put::<true>(q, q + 3, VARIADIC);
                        q += 2;
                    }
                    // anything else ends them, the body's
                    _ => return q,
                }
                q += 1;
            }
        }
        q
    }

    /// A directive's rest from `p`, [`Scan::pp`] what it is: plain but for comments, `defined`,
    /// and in a message (`#error`'s) its strings and words; a `\` at the line's end continues it.
    #[inline(never)]
    pub(super) fn pp_rest(&mut self, mut p: usize) {
        let b = self.b;
        let message = self.pp == PP_MESSAGE;
        while p < b.len() {
            let c = b[p];
            let k = CC[usize::from(c)];
            if k & SP != 0 {
                p += 1;
                continue;
            }
            match c {
                b'/' if b.get(p + 1) == Some(&b'/') => {
                    self.put::<true>(p, p + 2, LINE_MARK);
                    self.comment_line::<true>(p + 2);
                    p = b.len();
                }
                b'/' if b.get(p + 1) == Some(&b'*') => {
                    self.put::<true>(p, p + 2, COMMENT_MARK);
                    p = self.comment_block::<true>(p + 2);
                }
                b'\\' if p + 1 == b.len() => {
                    self.put::<true>(p, p + 1, CONT);
                    p += 1;
                }
                b'"' if message => p = self.dq::<true>(p),
                _ if message => {
                    // a word of the message, to a blank, a quote or a comment
                    let mut e = p + 1;
                    while e < b.len()
                        && CC[usize::from(b[e])] & SP == 0
                        && b[e] != b'"'
                        && !(b[e] == b'/' && matches!(b.get(e + 1), Some(b'/' | b'*')))
                        && !(b[e] == b'\\' && e + 1 == b.len())
                    {
                        e += 1;
                    }
                    self.put::<true>(p, e, UNQUOTED);
                    p = e;
                }
                _ if k & NS0 != 0 => {
                    let mut e = p + 1;
                    while e < b.len() && CC[usize::from(b[e])] & NW != 0 {
                        e += 1;
                    }
                    if &b[p..e] == b"defined" {
                        self.put::<true>(p, e, DEFINED);
                    }
                    p = e;
                }
                _ if k & NW != 0 => {
                    while p < b.len() && CC[usize::from(b[p])] & NW != 0 {
                        p += 1;
                    }
                }
                _ => p += 1,
            }
        }
        self.end_pp();
    }

    /// A `#define`'s body's line starts as an expression's: no declaration, no entity.
    fn body_start(&mut self) {
        (self.prev, self.decl, self.paren, self.angle) = (P_OTHER, D_DONE, 0, 0);
    }

    /// A directive's line ended: its text filled to the end; the directive goes on on the next
    /// line where this one ends in `\` or in a comment or string that does.
    pub(super) fn end_pp(&mut self) {
        let n = self.b.len();
        if n > self.at {
            self.k.merge(&mut self.out, self.at, n, PP_TEXT);
            self.at = n;
        }
        if self.mode == M_CODE && self.b.last() != Some(&b'\\') {
            self.pp = PP_NONE;
        }
    }

    /// A line that starts in a mode the line before left ([`Scan::mode`]) or in a directive
    /// ([`Scan::pp`]): the mode's rest, then the directive's or the code.
    #[inline(never)]
    pub(super) fn lex_resumed(&mut self) {
        if self.mode == M_DISABLED {
            return self.disabled();
        }
        let kept = self.kept();
        let p = if self.pp != PP_NONE {
            self.resume::<true>()
        } else {
            self.resume::<false>()
        };
        match self.pp {
            PP_NONE => self.code_line(p),
            PP_BODY => {
                self.body_start();
                self.code::<true>(p);
                self.end_pp();
                self.restore(kept);
            }
            _ => {
                if p < self.b.len() {
                    self.pp_rest(p);
                } else {
                    self.end_pp();
                }
                self.restore(kept);
            }
        }
    }

    /// The mode's rest at the line's start: where the code after it starts.
    #[inline(always)]
    fn resume<const PP: bool>(&mut self) -> usize {
        let b = self.b;
        match self.mode {
            M_BLOCK_COMMENT => self.comment_lines::<PP>(),
            M_LINE_COMMENT => {
                self.comment_line::<PP>(0);
                b.len()
            }
            M_DQ => {
                self.mode = M_CODE;
                self.dq_body::<PP>(0)
            }
            M_SQ => {
                self.mode = M_CODE;
                self.sq_body::<PP>(0)
            }
            M_RAW => self.raw_body::<PP>(0),
            _ => 0,
        }
    }

    /// A line in an `#if 0`'s branch: a comment, but where its `#else`, `#elif` or `#endif`
    /// (none inside an `#if` in it) ends it, a directive.
    fn disabled(&mut self) {
        let b = self.b;
        let q = skip_blanks(b, 0);
        if b.get(q) == Some(&b'#') {
            let s = skip_blanks(b, q + 1);
            let mut e = s;
            while e < b.len() && b[e].is_ascii_alphabetic() {
                e += 1;
            }
            match &b[s..e] {
                b"if" | b"ifdef" | b"ifndef" => self.aux = self.aux.saturating_add(1).min(31),
                b"endif" | b"else" | b"elif" | b"elifdef" | b"elifndef" if self.aux == 0 => {
                    self.mode = M_CODE;
                    self.directive(q);
                    return;
                }
                b"endif" => self.aux -= 1,
                _ => {}
            }
        }
        if !b.is_empty() {
            self.k.merge(&mut self.out, 0, b.len(), IF_BRANCH);
        }
    }
}

/// Where the blanks at `p` end.
#[inline(always)]
pub(super) fn skip_blanks(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && CC[usize::from(b[p])] & SP != 0 {
        p += 1;
    }
    p
}

/// Where the next `c` from `p` is, or the line's end.
#[inline(always)]
fn prim_find(b: &[u8], p: usize, c: u8) -> usize {
    crate::prim::find_swar(b, p, [c])
}
