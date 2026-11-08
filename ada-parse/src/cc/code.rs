//! The loop ([`Scan::code`]): a blank run passed whole, a name read once and looked up once
//! ([`Scan::word`]), punctuation one `match` ([`Scan::punct`]); numbers, strings, characters and
//! comments each a reader of its own. `PP` is a `#define`'s body ([`super::pp`]): its plain
//! text is [`PP_TEXT`], filled before each token, and its braces open no level.

use super::words::{A_ACCESS, A_C99, A_CASE, A_CLASS, A_ENUM, A_LEAF, A_NS, ACT, WORDS};
use super::*;
use crate::prim;

impl<K: Sink> Scan<'_, '_, K> {
    /// The code from `p` to the line's end.
    #[inline(never)]
    pub(super) fn code<const PP: bool>(&mut self, mut p: usize) {
        let b = self.b;
        while p < b.len() {
            let c = b[p];
            let k = CC[usize::from(c)];
            if k & SP != 0 {
                p += 1;
                while p < b.len() && CC[usize::from(b[p])] & SP != 0 {
                    p += 1;
                }
                continue;
            }
            p = if k & NS0 != 0 {
                self.word::<PP>(p)
            } else {
                self.punct::<PP>(p, c)
            };
        }
    }

    /// The name at `p`, whose first byte is a name's: a keyword by its leaf, else what the token
    /// before and the byte after make it.
    #[inline(always)]
    fn word<const PP: bool>(&mut self, p: usize) -> usize {
        let b = self.b;
        let mut e = p + 1;
        let mut bits = CC[usize::from(b[p])];
        while e < b.len() {
            let k = CC[usize::from(b[e])];
            if k & NW == 0 {
                break;
            }
            bits |= k;
            e += 1;
        }
        let id = if bits & HI == 0 {
            WORDS.get(&b[p..e])
        } else {
            0
        };
        // SAFETY: an id is at most the table's words, the row's length less one
        let info = unsafe { *self.info.get_unchecked(usize::from(id)) };
        if info == 0 {
            return self.name::<PP>(p, e, false);
        }
        let leaf = info & ((1 << ACT) - 1);
        match info >> ACT {
            A_LEAF => {}
            A_C99 => return self.name::<PP>(p, e, true),
            A_CASE => self.case = true,
            A_ACCESS => {
                self.put::<PP>(p, e, leaf);
                self.prev = P_ACCESS;
                return e;
            }
            A_CLASS => {
                // `enum class` stays an enum's
                if self.pending != INIT + 1 {
                    self.pending = CLASS + 1;
                }
            }
            A_ENUM => self.pending = INIT + 1,
            A_NS => self.pending = NS + 1,
            _ => {}
        }
        self.put::<PP>(p, e, leaf);
        self.prev = P_OTHER;
        e
    }

    /// A name that isn't a keyword, `p..e` (`c99` one of C99's functions): a call before `(`, a
    /// member after `.` or `->`, a string's prefix before its quote, a label at a statement's
    /// start, else by its shape (`mFoo`, `kFoo`) or plain.
    #[inline(always)]
    fn name<const PP: bool>(&mut self, p: usize, e: usize, c99: bool) -> usize {
        let b = self.b;
        let prev = self.prev;
        self.prev = P_OTHER;
        let leaf = match b.get(e) {
            Some(b'(') => call_leaf(prev, c99),
            Some(b'"' | b'\'') => return self.prefixed::<PP>(p, e),
            Some(b':') if !PP && prev == P_STMT && b.get(e + 1) != Some(&b':') => {
                self.put::<PP>(p, e, LABEL);
                self.put::<PP>(e, e + 1, SEPARATOR);
                self.prev = P_STMT;
                return e + 1;
            }
            Some(&c) if CC[usize::from(c)] & SP != 0 => {
                let mut q = e + 1;
                while q < b.len() && CC[usize::from(b[q])] & SP != 0 {
                    q += 1;
                }
                if b.get(q) == Some(&b'(') {
                    call_leaf(prev, c99)
                } else {
                    self.value_leaf(p, e, prev)
                }
            }
            _ => self.value_leaf(p, e, prev),
        };
        if leaf != PLAIN {
            self.put::<PP>(p, e, leaf);
        }
        e
    }

    /// A name `p..e` that isn't called: a member after an accessor, else by its shape.
    #[inline(always)]
    fn value_leaf(&self, p: usize, e: usize, prev: u8) -> u16 {
        if prev == P_ACCESSOR {
            return MEMBER_VAR;
        }
        if prev == P_ARROW {
            return PLAIN;
        }
        let b = self.b;
        let first = b[p];
        if !matches!(first, b'f' | b'm' | b'g' | b's' | b'k') || e - p < 2 {
            return PLAIN;
        }
        let second = b[p + 1];
        if CC[usize::from(second)] & UP != 0 {
            match first {
                b'f' | b'm' if !self.c => MEMBER_VAR,
                b'g' => GLOBAL_MAC,
                b's' => STATIC_MAC,
                b'k' => MAC_CONSTANT,
                _ => PLAIN,
            }
        } else if first == b'm' && second == b'_' && e - p > 2 && !self.c {
            MEMBER_VAR
        } else {
            PLAIN
        }
    }

    /// A name `p..e` before a quote: a string's or a character's prefix (`L`, `u`, `U`, `u8`,
    /// C++'s raw strings' `R`, `LR`…), else plain.
    #[cold]
    #[inline(never)]
    fn prefixed<const PP: bool>(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let w = &b[p..e];
        let plain = matches!(w, b"L" | b"u" | b"U" | b"u8");
        if b[e] == b'"' {
            if plain {
                self.put::<PP>(p, e, DQ_PREFIX);
                return self.dq::<PP>(e);
            }
            if !self.c
                && matches!(w, b"R" | b"LR" | b"uR" | b"UR" | b"u8R")
                && let Some(q) = self.raw::<PP>(p, e)
            {
                return q;
            }
        } else if plain {
            self.put::<PP>(p, e, SQ_PREFIX);
            return self.sq::<PP>(e);
        }
        let leaf = self.value_leaf(p, e, P_OTHER);
        if leaf != PLAIN {
            self.put::<PP>(p, e, leaf);
        }
        e
    }

    /// The token at `p` that isn't a blank nor a name: a number, a string, a comment, an
    /// operator, a bracket.
    #[inline(always)]
    fn punct<const PP: bool>(&mut self, p: usize, c: u8) -> usize {
        let b = self.b;
        let next = b.get(p + 1).copied().unwrap_or(0);
        let (n, leaf) = match c {
            b'0'..=b'9' => {
                self.prev = P_OTHER;
                return self.number::<PP>(p);
            }
            b'"' => {
                self.prev = P_OTHER;
                return self.dq::<PP>(p);
            }
            b'\'' => {
                self.prev = P_OTHER;
                return self.sq::<PP>(p);
            }
            b'/' => match next {
                b'/' => return self.line_comment::<PP>(p),
                b'*' => return self.block_comment::<PP>(p),
                b'=' => (2, ASSIGN_AUG),
                _ => (1, ARITH),
            },
            b'(' => {
                self.pending = NO_PENDING;
                (1, GROUP_BEGIN)
            }
            b')' => {
                self.put::<PP>(p, p + 1, GROUP_END);
                self.prev = P_CLOSE;
                return p + 1;
            }
            b';' => {
                self.put::<PP>(p, p + 1, TERMINATOR);
                (self.prev, self.pending, self.ternary, self.case) = (P_STMT, NO_PENDING, 0, false);
                return p + 1;
            }
            b',' => (1, SEPARATOR),
            b'{' => {
                if !PP {
                    self.open_brace();
                }
                self.put::<PP>(p, p + 1, BLOCK_BEGIN);
                (self.prev, self.ternary) = (P_STMT, 0);
                return p + 1;
            }
            b'}' => {
                if !PP {
                    self.close_brace();
                }
                self.put::<PP>(p, p + 1, BLOCK_END);
                (self.prev, self.ternary) = (P_STMT, 0);
                return p + 1;
            }
            b'[' => (1, BRACKETS_BEGIN),
            b']' => (1, BRACKETS_END),
            b'.' => match next {
                b'0'..=b'9' => {
                    self.prev = P_OTHER;
                    return self.number::<PP>(p);
                }
                b'.' if b.get(p + 2) == Some(&b'.') => (3, VARIADIC),
                b'*' => (2, OPERATOR),
                _ => {
                    self.put::<PP>(p, p + 1, DOT);
                    self.prev = P_ACCESSOR;
                    return p + 1;
                }
            },
            b'-' => match next {
                b'>' => {
                    let n = if b.get(p + 2) == Some(&b'*') { 3 } else { 2 };
                    self.put::<PP>(p, p + n, ARROW);
                    self.prev = if self.c { P_ARROW } else { P_ACCESSOR };
                    return p + n;
                }
                b'-' => (2, ARITH),
                b'=' => (2, ASSIGN_AUG),
                _ => (1, ARITH),
            },
            b'+' => match next {
                b'+' => (2, ARITH),
                b'=' => (2, ASSIGN_AUG),
                _ => (1, ARITH),
            },
            b'*' | b'%' | b'^' => match next {
                b'=' => (2, ASSIGN_AUG),
                _ if c == b'*' => (1, OPERATOR),
                _ => (1, ARITH),
            },
            b'&' | b'|' => match next {
                b'=' => (2, ASSIGN_AUG),
                _ if next == c => (2, OPERATOR),
                _ => (1, OPERATOR),
            },
            b'=' => match next {
                b'=' => (2, COMPARISON),
                _ => {
                    self.pending = NO_PENDING;
                    (1, ASSIGN)
                }
            },
            b'!' => match next {
                b'=' => (2, COMPARISON),
                _ => (1, OPERATOR),
            },
            b'<' | b'>' => {
                if next == c {
                    if b.get(p + 2) == Some(&b'=') {
                        (3, ASSIGN_AUG)
                    } else {
                        (2, ARITH)
                    }
                } else if next == b'=' {
                    if c == b'<' && b.get(p + 2) == Some(&b'>') {
                        (3, COMPARISON)
                    } else {
                        (2, COMPARISON)
                    }
                } else {
                    (1, COMPARISON)
                }
            }
            b'~' => (1, OPERATOR),
            b'?' => {
                self.ternary = self.ternary.saturating_add(1);
                (1, TERNARY)
            }
            b':' => return self.colon::<PP>(p, next),
            b'\\' if p + 1 == b.len() => (1, CONT),
            _ => {
                // a stray byte (`#`, `@`, a control character) is plain
                self.prev = P_OTHER;
                return p + 1;
            }
        };
        self.put::<PP>(p, p + n, leaf);
        self.prev = P_OTHER;
        p + n
    }

    /// A `:` at `p`, or `::`: by the token before, a ternary's, a `case`'s, an access label's,
    /// an initializer list's, a base list's; else as bat has a bit-field's or a range `for`'s,
    /// the ternary's operator.
    #[inline(always)]
    fn colon<const PP: bool>(&mut self, p: usize, next: u8) -> usize {
        if next == b':' {
            self.put::<PP>(p, p + 2, DOUBLE_COLON);
            self.prev = P_OTHER;
            return p + 2;
        }
        let (leaf, prev) = if self.ternary > 0 {
            self.ternary -= 1;
            (TERNARY, P_OTHER)
        } else if self.case {
            self.case = false;
            (SEPARATOR, P_STMT)
        } else {
            match self.prev {
                P_ACCESS => (CLASS_SECTION, P_STMT),
                P_CLOSE => (INIT_LIST, P_OTHER),
                _ if self.pending == CLASS + 1 || self.pending == INIT + 1 => (SEPARATOR, P_OTHER),
                _ => (TERNARY, P_OTHER),
            }
        };
        self.put::<PP>(p, p + 1, leaf);
        self.prev = prev;
        p + 1
    }

    /// A number at `p` (a digit, or `.` before one): its base's prefix, digits, a float's `.`
    /// and exponent, a suffix.
    #[inline(never)]
    fn number<const PP: bool>(&mut self, p: usize) -> usize {
        let b = self.b;
        if b[p] == b'0' {
            match b.get(p + 1).map(|c| c | 0x20) {
                Some(b'x') => return self.hex::<PP>(p),
                Some(b'b') if matches!(b.get(p + 2), Some(b'0' | b'1')) => {
                    let e = digits(b, p + 2, |c| matches!(c, b'0' | b'1'));
                    self.put::<PP>(p, p + 2, BIN_BASE);
                    self.put::<PP>(p + 2, e, BIN);
                    return self.suffix::<PP>(e, BIN_SUFFIX, DEC_BAD_SUFFIX, false);
                }
                _ => {}
            }
        }
        let e = digits(b, p, |c| c.is_ascii_digit());
        if matches!(b.get(e), Some(b'.'))
            || matches!(b.get(e), Some(b'e' | b'E')) && exp_end(b, e).is_some()
        {
            return self.float::<PP>(p, e);
        }
        if b[p] == b'0' && e > p + 1 {
            self.put::<PP>(p, p + 1, OCT_BASE);
            self.put::<PP>(p + 1, e, OCT);
            return self.suffix::<PP>(e, OCT_SUFFIX, DEC_BAD_SUFFIX, false);
        }
        self.put::<PP>(p, e, DEC);
        self.suffix::<PP>(e, DEC_SUFFIX, DEC_BAD_SUFFIX, false)
    }

    /// A decimal float whose integer part is `p..e`: its `.`, its fraction, its exponent, a
    /// suffix.
    fn float<const PP: bool>(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        if e > p {
            self.put::<PP>(p, e, FLOAT);
        }
        let mut q = e;
        if b.get(q) == Some(&b'.') {
            self.put::<PP>(q, q + 1, FLOAT_DOT);
            let f = digits(b, q + 1, |c| c.is_ascii_digit());
            if f > q + 1 {
                self.put::<PP>(q + 1, f, FLOAT);
            }
            q = f;
        }
        if let Some(x) = exp_end(b, q) {
            self.put::<PP>(q, x, FLOAT);
            q = x;
        }
        self.suffix::<PP>(q, FLOAT_SUFFIX, FLOAT_BAD_SUFFIX, true)
    }

    /// A hexadecimal number at `p`, its `0x` there.
    fn hex<const PP: bool>(&mut self, p: usize) -> usize {
        let b = self.b;
        let e = digits(b, p + 2, |c| c.is_ascii_hexdigit());
        if matches!(b.get(e), Some(b'.' | b'p' | b'P')) {
            // a hexadecimal float
            self.put::<PP>(p, p + 2, HEX_FLOAT_BASE);
            if e > p + 2 {
                self.put::<PP>(p + 2, e, HEX_FLOAT);
            }
            let mut q = e;
            if b.get(q) == Some(&b'.') {
                self.put::<PP>(q, q + 1, HEX_FLOAT_DOT);
                let f = digits(b, q + 1, |c| c.is_ascii_hexdigit());
                if f > q + 1 {
                    self.put::<PP>(q + 1, f, HEX_FLOAT);
                }
                q = f;
            }
            if matches!(b.get(q), Some(b'p' | b'P')) {
                let mut x = q + 1;
                if matches!(b.get(x), Some(b'+' | b'-')) {
                    x += 1;
                }
                let d = digits(b, x, |c| c.is_ascii_digit());
                if d > x {
                    self.put::<PP>(q, d, HEX_FLOAT);
                    q = d;
                }
            }
            return self.suffix::<PP>(q, HEX_FLOAT_SUFFIX, FLOAT_BAD_SUFFIX, true);
        }
        self.put::<PP>(p, p + 2, HEX_BASE);
        if e > p + 2 {
            self.put::<PP>(p + 2, e, HEX);
        }
        self.suffix::<PP>(e, HEX_SUFFIX, HEX_BAD_SUFFIX, false)
    }

    /// A number's suffix at `e`, a name's bytes: `ok` where it is one (`u`, `ll`, `f`…), else
    /// `bad`.
    #[inline(always)]
    fn suffix<const PP: bool>(&mut self, e: usize, ok: u16, bad: u16, float: bool) -> usize {
        let b = self.b;
        let mut s = e;
        while s < b.len() && CC[usize::from(b[s])] & NW != 0 {
            s += 1;
        }
        if s > e {
            let w = &b[e..s];
            let good = if float {
                float_suffix(w)
            } else {
                int_suffix(w)
            };
            self.put::<PP>(e, s, if good { ok } else { bad });
        }
        s
    }

    /// A `"…"` string at `p`, its quote there.
    pub(super) fn dq<const PP: bool>(&mut self, p: usize) -> usize {
        self.put::<PP>(p, p + 1, DQ_BEGIN);
        self.dq_body::<PP>(p + 1)
    }

    /// A `"…"` string's text from `q`: escapes, `printf`'s placeholders, its end. A string
    /// whose line ends in `\` goes on on the next ([`M_DQ`]); one that doesn't ends at the
    /// line's end (ada's: bat's runs on into the lines after).
    #[inline(never)]
    pub(super) fn dq_body<const PP: bool>(&mut self, mut q: usize) -> usize {
        let b = self.b;
        let mut s = q;
        loop {
            q = prim::find_in(b, q, b.len(), *b"\"\\%");
            if q >= b.len() {
                if b.len() > s {
                    self.put::<PP>(s, b.len(), DQ);
                }
                return b.len();
            }
            match b[q] {
                b'"' => {
                    if q > s {
                        self.put::<PP>(s, q, DQ);
                    }
                    self.put::<PP>(q, q + 1, DQ_END);
                    return q + 1;
                }
                b'\\' => {
                    if q > s {
                        self.put::<PP>(s, q, DQ);
                    }
                    if q + 1 == b.len() {
                        self.put::<PP>(q, q + 1, DQ_CONT);
                        self.mode = M_DQ;
                        return b.len();
                    }
                    let e = escape_end(b, q);
                    self.put::<PP>(q, e, DQ_ESCAPE);
                    (s, q) = (e, e);
                }
                _ => match placeholder(b, q) {
                    Some(e) => {
                        if q > s {
                            self.put::<PP>(s, q, DQ);
                        }
                        self.put::<PP>(q, e, DQ_PLACEHOLDER);
                        (s, q) = (e, e);
                    }
                    None => q += 1,
                },
            }
        }
    }

    /// A character `'…'` at `p`, its quote there.
    pub(super) fn sq<const PP: bool>(&mut self, p: usize) -> usize {
        self.put::<PP>(p, p + 1, SQ_BEGIN);
        self.sq_body::<PP>(p + 1)
    }

    /// A character's text from `q`, as [`Scan::dq_body`]'s (no placeholders).
    #[inline(never)]
    pub(super) fn sq_body<const PP: bool>(&mut self, mut q: usize) -> usize {
        let b = self.b;
        let mut s = q;
        loop {
            q = prim::find_swar(b, q, *b"'\\");
            if q >= b.len() {
                if b.len() > s {
                    self.put::<PP>(s, b.len(), SQ);
                }
                return b.len();
            }
            if q > s {
                self.put::<PP>(s, q, SQ);
            }
            if b[q] == b'\'' {
                self.put::<PP>(q, q + 1, SQ_END);
                return q + 1;
            }
            if q + 1 == b.len() {
                self.put::<PP>(q, q + 1, SQ_CONT);
                self.mode = M_SQ;
                return b.len();
            }
            let e = escape_end(b, q);
            self.put::<PP>(q, e, SQ_ESCAPE);
            (s, q) = (e, e);
        }
    }

    /// A raw string at `p` (its prefix to `q`, its quote): `R"delim(…)delim"`, on to the lines
    /// after till its end ([`M_RAW`]); `None` where no `(` ends a delimiter.
    fn raw<const PP: bool>(&mut self, p: usize, q: usize) -> Option<usize> {
        let b = self.b;
        let mut d = q + 1;
        while d < b.len() && d - q <= 17 && !matches!(b[d], b'(' | b')' | b'\\' | b'"' | b' ') {
            d += 1;
        }
        if b.get(d) != Some(&b'(') {
            return None;
        }
        let n = d - q - 1;
        self.delim = [0; DELIM_KEPT];
        let kept = n.min(DELIM_KEPT);
        self.delim[..kept].copy_from_slice(&b[q + 1..q + 1 + kept]);
        self.aux = n as u8;
        self.put::<PP>(p, q, DQ_PREFIX);
        self.put::<PP>(q, d + 1, DQ_BEGIN);
        Some(self.raw_body::<PP>(d + 1))
    }

    /// A raw string's text from `q` to its `)delim"`, or the line's end.
    pub(super) fn raw_body<const PP: bool>(&mut self, q: usize) -> usize {
        let b = self.b;
        let n = usize::from(self.aux);
        let kept = n.min(DELIM_KEPT);
        let mut i = q;
        loop {
            i = prim::find_swar(b, i, *b")");
            if i >= b.len() {
                if b.len() > q {
                    self.put::<PP>(q, b.len(), DQ);
                }
                self.mode = M_RAW;
                return b.len();
            }
            let e = i + 1 + n;
            if e < b.len() && b[e] == b'"' && b[i + 1..i + 1 + kept] == self.delim[..kept] {
                if i > q {
                    self.put::<PP>(q, i, DQ);
                }
                self.put::<PP>(i, e + 1, DQ_END);
                self.mode = M_CODE;
                self.aux = 0;
                self.delim = [0; DELIM_KEPT];
                return e + 1;
            }
            i += 1;
        }
    }

    /// A `//` comment at `p`, to the line's end; one that ends in `\` goes on on the next
    /// ([`M_LINE_COMMENT`]).
    #[inline(never)]
    fn line_comment<const PP: bool>(&mut self, p: usize) -> usize {
        self.put::<PP>(p, p + 2, LINE_MARK);
        self.comment_line::<PP>(p + 2);
        self.b.len()
    }

    /// A `//` comment's text from `q` to the line's end.
    pub(super) fn comment_line<const PP: bool>(&mut self, q: usize) {
        let b = self.b;
        let n = b.len();
        if b.last() == Some(&b'\\') && n > q {
            if n - 1 > q {
                self.put::<PP>(q, n - 1, LINE_TEXT);
            }
            self.put::<PP>(n - 1, n, LINE_CONT);
            self.mode = M_LINE_COMMENT;
        } else {
            if n > q {
                self.put::<PP>(q, n, LINE_TEXT);
            }
            self.mode = M_CODE;
        }
    }

    /// A `/* … */` comment at `p`, on to the lines after till its end ([`M_BLOCK_COMMENT`]).
    #[inline(never)]
    fn block_comment<const PP: bool>(&mut self, p: usize) -> usize {
        self.put::<PP>(p, p + 2, COMMENT_MARK);
        self.comment_block::<PP>(p + 2)
    }

    /// A block comment's line that the line before left open: a leading `*` (not `*/`) its
    /// punctuation, as bat has it, then its text.
    pub(super) fn comment_lines<const PP: bool>(&mut self) -> usize {
        let b = self.b;
        let mut q = 0;
        while q < b.len() && CC[usize::from(b[q])] & SP != 0 {
            q += 1;
        }
        if b.get(q) == Some(&b'*') && b.get(q + 1) != Some(&b'/') {
            if q > 0 {
                self.put::<PP>(0, q, BLOCK_TEXT);
            }
            self.put::<PP>(q, q + 1, COMMENT_MARK);
            return self.comment_block::<PP>(q + 1);
        }
        self.comment_block::<PP>(0)
    }

    /// A block comment's text from `q` to its `*/`, or the line's end.
    pub(super) fn comment_block<const PP: bool>(&mut self, q: usize) -> usize {
        let b = self.b;
        let mut i = q;
        loop {
            i = prim::find_in(b, i, b.len(), *b"*");
            if i + 1 >= b.len() {
                if b.len() > q {
                    self.put::<PP>(q, b.len(), BLOCK_TEXT);
                }
                self.mode = M_BLOCK_COMMENT;
                return b.len();
            }
            if b[i + 1] == b'/' {
                if i > q {
                    self.put::<PP>(q, i, BLOCK_TEXT);
                }
                self.put::<PP>(i, i + 2, COMMENT_MARK);
                self.mode = M_CODE;
                return i + 2;
            }
            i += 1;
        }
    }
}

/// A call's leaf, by the token before: a member's after an accessor, C99's for its functions.
#[inline(always)]
fn call_leaf(prev: u8, c99: bool) -> u16 {
    if prev == P_ACCESSOR {
        MEMBER_FUNCTION
    } else if c99 {
        C99
    } else {
        FUNCTION
    }
}

/// Where the digits `digit` takes from `p` end, a `'` between two of them taken (C++14's
/// separator).
#[inline(always)]
fn digits(b: &[u8], mut p: usize, digit: impl Fn(u8) -> bool) -> usize {
    while p < b.len() {
        let c = b[p];
        if digit(c)
            || (c == b'\'' && p + 1 < b.len() && digit(b[p + 1]) && p > 0 && digit(b[p - 1]))
        {
            p += 1;
        } else {
            break;
        }
    }
    p
}

/// An exponent at `e` (`e`, a sign, digits): where it ends.
#[inline(always)]
fn exp_end(b: &[u8], e: usize) -> Option<usize> {
    if !matches!(b.get(e), Some(b'e' | b'E')) {
        return None;
    }
    let mut x = e + 1;
    if matches!(b.get(x), Some(b'+' | b'-')) {
        x += 1;
    }
    let d = digits(b, x, |c| c.is_ascii_digit());
    (d > x).then_some(d)
}

/// An integer's suffix: `u`, `l`, `ll`, `z` and their pairs, either case.
fn int_suffix(w: &[u8]) -> bool {
    let mut u = false;
    let mut rest = w;
    if let [c, r @ ..] = rest
        && (*c | 0x20) == b'u'
    {
        u = true;
        rest = r;
    }
    rest = match rest {
        [b'l', b'l', r @ ..] | [b'L', b'L', r @ ..] => r,
        [b'l' | b'L' | b'z' | b'Z', r @ ..] => r,
        r => r,
    };
    if !u
        && let [c, r @ ..] = rest
        && (*c | 0x20) == b'u'
    {
        rest = r;
    }
    rest.is_empty()
}

/// A float's suffix: `f`, `l`, `f16`…`bf16`, either case.
fn float_suffix(w: &[u8]) -> bool {
    matches!(
        w,
        b"f" | b"F"
            | b"l"
            | b"L"
            | b"f16"
            | b"f32"
            | b"f64"
            | b"f128"
            | b"F16"
            | b"F32"
            | b"F64"
            | b"F128"
            | b"bf16"
            | b"BF16"
            | b"df"
            | b"dd"
            | b"dl"
            | b"DF"
            | b"DD"
            | b"DL"
    )
}

/// An escape at `q` (its `\`, not the line's last byte): where it ends.
#[inline(always)]
fn escape_end(b: &[u8], q: usize) -> usize {
    let c = b[q + 1];
    match c {
        b'x' => {
            let e = digits(b, q + 2, |c| c.is_ascii_hexdigit());
            e.max(q + 2)
        }
        b'u' | b'U' => {
            let n = if c == b'u' { 4 } else { 8 };
            let mut e = q + 2;
            while e < b.len() && e < q + 2 + n && b[e].is_ascii_hexdigit() {
                e += 1;
            }
            e
        }
        b'0'..=b'7' => {
            let mut e = q + 2;
            while e < b.len() && e < q + 4 && matches!(b[e], b'0'..=b'7') {
                e += 1;
            }
            e
        }
        _ => q + 1 + prim::char_len(c),
    }
}

/// `printf`'s placeholder at `q`, its `%`: where it ends (bat's
/// `%(\d+\$)?[#0\- +']*[,;:_]?((-?\d+)|\*(-?\d+\$)?)?(\.((-?\d+)|\*(-?\d+\$)?)?)?(hh|h|ll|l|j|t|z|q|L|vh|vl|v|hv|hl)?[diouxXDOUeEfFgGaACcSspn%]`).
fn placeholder(b: &[u8], q: usize) -> Option<usize> {
    let at = |i: usize| b.get(i).copied().unwrap_or(0);
    let mut i = q + 1;
    // `\d+\$`
    let d = digits(b, i, |c| c.is_ascii_digit());
    if d > i && at(d) == b'$' {
        i = d + 1;
    }
    while matches!(at(i), b'#' | b'0' | b'-' | b' ' | b'+' | b'\'') {
        i += 1;
    }
    if matches!(at(i), b',' | b';' | b':' | b'_') {
        i += 1;
    }
    // a width: `-?\d+` or `\*(-?\d+\$)?`
    let width = |i: usize| -> usize {
        if at(i) == b'*' {
            let mut j = i + 1;
            let s = if at(j) == b'-' { j + 1 } else { j };
            let d = digits(b, s, |c| c.is_ascii_digit());
            if d > s && at(d) == b'$' {
                j = d + 1;
            }
            j
        } else {
            let s = if at(i) == b'-' { i + 1 } else { i };
            let d = digits(b, s, |c| c.is_ascii_digit());
            if d > s { d } else { i }
        }
    };
    i = width(i);
    if at(i) == b'.' {
        i = width(i + 1);
    }
    i += match (at(i), at(i + 1)) {
        (b'h', b'h') | (b'l', b'l') | (b'v', b'h') | (b'v', b'l') | (b'h', b'v') | (b'h', b'l') => {
            2
        }
        (b'h' | b'l' | b'j' | b't' | b'z' | b'q' | b'L' | b'v', _) => 1,
        _ => 0,
    };
    matches!(
        at(i),
        b'd' | b'i'
            | b'o'
            | b'u'
            | b'x'
            | b'X'
            | b'D'
            | b'O'
            | b'U'
            | b'e'
            | b'E'
            | b'f'
            | b'F'
            | b'g'
            | b'G'
            | b'a'
            | b'A'
            | b'C'
            | b'c'
            | b'S'
            | b's'
            | b'p'
            | b'n'
            | b'%'
    )
    .then_some(i + 1)
}
