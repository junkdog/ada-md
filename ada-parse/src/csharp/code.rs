//! The turns of C#'s code contexts in a loop of their own (X-025's first stage): while the top
//! context is one that passes over blanks (code's), its turns run here a token at a time, blanks
//! passed inline and the hot contexts' patterns dispatched without [`Scan::context`]'s `match`.
//! bat's contexts and their stack stay as they are, so each turn is the one [`Scan::run`] would
//! take, and the state between lines is the same. An expression's tokens go one step further
//! ([`Scan::expr`], X-025's third stage): a run of them in one call, each token's patterns
//! decided by its first byte and what follows it, the spans emitted inline.

use super::c::*;
use super::*;
use crate::clike::{self, CLASS, DIGIT, SPACE};

impl Scan<'_> {
    /// The turns from `p` (past the line's start) while the top context is code's: blanks passed
    /// inline, each token's patterns tried as [`Scan::run`]'s turn would. Returns where the scan
    /// is when the top isn't code's, the line ends, or the turn at the place is one for
    /// [`Scan::run`] (a comment's start, a character past ASCII, no pattern matching).
    pub(super) fn code(&mut self, mut p: usize) -> usize {
        let (b, n) = (self.b, self.b.len());
        loop {
            let top = self.top();
            if CTX[usize::from(top)] & INERT == 0 {
                return p;
            }
            p = blanks(b, n, p);
            if p >= n {
                return p;
            }
            let c = b[p];
            if c >= 0x80 || (c == b'/' && p + 1 < n && matches!(b[p + 1], b'/' | b'*')) {
                // a line comment with no to-do or link in it: the prototype's open, the
                // comment's step to the line's end, and its close there, as `run` takes them
                if c == b'/'
                    && b[p + 1] == b'/'
                    && CTX[usize::from(top)] & PROTO != 0
                    && self.depth < SYNTECT_DEPTH
                    && self.comment_skip::<false, false>(p + 2) >= n
                {
                    self.open(p, p + 2, LC_MARK, &[LINE_COMMENT]);
                    self.fill(n);
                    self.pop();
                    self.looped = false;
                    self.stuck = 0;
                    self.step = n + 1;
                    return n + 1;
                }
                return p;
            }
            self.looped = false;
            // an expression's tokens run in a loop of their own, where they can
            if CTX[usize::from(top)] & EXPR != 0 && self.stuck == 0 {
                let q = self.expr(p);
                if q > p {
                    p = q;
                    self.step = q;
                    continue;
                }
            }
            let Some(q) = self.frame(top, p) else {
                return p;
            };
            if self.halt {
                return q;
            }
            self.stuck = if q == p { self.stuck + 1 } else { 0 };
            p = q;
            self.step = q;
            if self.stuck > STUCK {
                self.stuck = 0;
                if p >= n {
                    return p;
                }
                p = self.next(p);
                self.step = p;
            }
        }
    }

    /// A frame's turn at `p` (not a blank, not a comment's start, ASCII): the hot ones' arms
    /// here, inlined into the loop, the rest [`Scan::context`]'s.
    #[inline(always)]
    fn frame(&mut self, top: u8, p: usize) -> Option<usize> {
        match top {
            LINE_IN => self.line_in(p),
            ARG_EXPR => {
                // `nosemi`'s pop at `,` or `)`, then `arguments`' turn there: its separator or
                // its end
                let c = self.b[p];
                if matches!(c, b',' | b')') && self.stuck < STUCK {
                    let q = self.leave(p)?;
                    if self.top() != ARGUMENTS {
                        return Some(q);
                    }
                    return if c == b',' {
                        self.tok(p, p + 1, ARGUMENT_SEP)
                    } else {
                        self.close(p, p + 1, GROUP_END)
                    };
                }
                if let Some(q) = self.nosemi(p) {
                    return Some(q);
                }
                if self.b[p] == b';' {
                    return self.leave(p);
                }
                None
            }
            ARGUMENTS => self.arguments(p),
            DTB_BLOCK => self.dtb_block(p),
            MB_BLOCK | CODE_BRACE | EB_BRACE => {
                if self.b[p] == b'}' {
                    let leaf = if top == EB_BRACE {
                        M_BLOCK_END
                    } else {
                        BLOCK_END
                    };
                    return self.close(p, p + 1, leaf);
                }
                if top == MB_BLOCK
                    && let Some(q) = self.stray(p)
                {
                    return Some(q);
                }
                // the push's turn, then the statement's at the same place, counted as the loop
                // counts a turn that takes nothing (and left to it where the guard is due)
                let q = self.enter(p, &[LINE])?;
                if self.halt || self.stuck + 1 >= STUCK {
                    return Some(q);
                }
                self.stuck += 1;
                self.line(p)
            }
            IF_GROUP => {
                if self.b[p] == b')' {
                    self.put(p, p + 1, M_GROUP_END);
                    self.pop();
                    return Some(p + 1);
                }
                self.line_in(p)
            }
            LINE => self.line(p),
            VARS_DECL => self.vars_decl(p),
            METHOD_PARAMS => {
                if self.b[p] == b')' {
                    return self.close(p, p + 1, PARAMS_END_M);
                }
                // `(?=\S)`'s push of a parameter and its type, then the type's `(?=\S)` set
                // (not at an attribute's `[`), neither taking anything, then the type's turn
                // at the same place: counted as the loop counts turns that take nothing, and
                // left to it where the guard is due
                let q = self.enter(p, &[METHOD_PARAM, METHOD_PARAM_TYPE])?;
                if self.halt || self.b[p] == b'[' || self.stuck + 2 >= STUCK {
                    return Some(q);
                }
                self.stuck += 1;
                self.step = p;
                self.shift(p, &[MPT_TYPE]);
                self.stuck += 1;
                self.mpt_type(p)
            }
            _ => self.context(top, p),
        }
    }
}

/// Where the decimal integer at `p` ends, where `literals` reads it as one and only one
/// (`DECIMAL`, no suffix): ASCII digits that no letter, `_`, `.` or byte past ASCII follows,
/// and no `0` before more digits.
#[inline(always)]
fn plain_int(b: &[u8], p: usize) -> Option<usize> {
    let mut d = p + 1;
    while d < b.len() && b[d].is_ascii_digit() {
        d += 1;
    }
    let ok = (b[p] != b'0' || d == p + 1)
        && b.get(d)
            .is_none_or(|&c| c < 0x80 && !c.is_ascii_alphanumeric() && !matches!(c, b'_' | b'.'));
    ok.then_some(d)
}

/// Where the blanks at `p` end, on a line of `n` bytes: one or two checked here, a longer run
/// out of line (a loop here keeps the run's place in several registers at every token).
#[inline(always)]
fn blanks(b: &[u8], n: usize, p: usize) -> usize {
    if p < n && CLASS[usize::from(b[p])] & SPACE != 0 {
        if p + 1 < n && CLASS[usize::from(b[p + 1])] & SPACE != 0 {
            return blank_run(b, n, p + 2);
        }
        return p + 1;
    }
    p
}

#[inline(never)]
fn blank_run(b: &[u8], n: usize, p: usize) -> usize {
    clike::skip_blanks(&b[..n], p)
}

/// The bytes an expression's context has a pattern of its own at, those under 64 (`,`, `)`,
/// `:`, `;`); `]` is tested by itself.
const OWN: u64 = 1 << b',' | 1 << b')' | 1 << b':' | 1 << b';';

/// What an expression's turn did at the place, `p`.
enum Turn {
    /// It took `p..` to here.
    Took(usize),
    /// The name `p..` to here is the expression's next turn ([`Scan::expr_name`]); anything
    /// before it (an argument's push) is done.
    Name(usize),
    /// `arguments`' push of an expression (its turn takes nothing); the expression's turn at
    /// the same place is next, and takes it.
    Pushed,
    /// Nothing; the turn is [`Scan::frame`]'s.
    Not,
}

impl Scan<'_> {
    /// An expression's tokens in a run of their own (X-025's third stage): while the top is
    /// `line_of_code_in`, an argument's expression or `arguments`, each token whose turn there
    /// is decided by its first byte and what follows it on the line (a plain name, a call's
    /// name, `.`, an operator, a number, a character or a string's start, an argument's start,
    /// `,` and `)`, a statement's `;`) is taken here, the turn [`Scan::frame`]'s arm would take,
    /// its spans emitted inline. Returns where the run stops: the line's end, a token it leaves
    /// to [`Scan::frame`], or a top it doesn't run (`p` itself when it took nothing).
    #[inline(never)]
    pub(super) fn expr(&mut self, mut p: usize) -> usize {
        let (b, n) = (self.b, self.b.len());
        loop {
            let top = self.top();
            if CTX[usize::from(top)] & EXPR == 0 || self.depth + 2 >= SYNTECT_DEPTH {
                return p;
            }
            let q = blanks(b, n, p);
            if q >= n {
                return q;
            }
            let c = b[q];
            if c >= 0x80 || (c == b'/' && q + 1 < n && matches!(b[q + 1], b'/' | b'*')) {
                return q;
            }
            self.looped = false;
            let turn = if top == ARGUMENTS {
                self.expr_arg(q)
            } else {
                self.expr_tok(top, q)
            };
            // a name's patterns in one place, whichever context reached them
            let r = match turn {
                Turn::Took(r) => r,
                Turn::Name(e) => match self.expr_name(q, e) {
                    Some(r) => r,
                    None => return q,
                },
                Turn::Pushed => {
                    p = q;
                    continue;
                }
                Turn::Not => return q,
            };
            debug_assert!(r > q);
            p = r;
            self.step = r;
        }
    }

    /// The turn at `p` of an expression's context `top` ([`EXPR`]'s but `arguments`), where it
    /// is one [`Scan::expr`] takes (`None` before anything is done, where it isn't): the
    /// context's own patterns first, then `line_of_code_in`'s (`line_of_code_in_no_semicolon`'s
    /// alone for a group's and an argument's expression).
    #[inline(always)]
    fn expr_tok(&mut self, top: u8, p: usize) -> Turn {
        let (b, n) = (self.b, self.b.len());
        let c = b[p];
        if (c < 64 && OWN >> c & 1 != 0) || c == b']' {
            match (top, c) {
                // `nosemi`'s pop, then `arguments`' separator or end
                (ARG_EXPR, b',' | b')') => {
                    let d = self.depth;
                    if d < 3 || self.ctx_at(d - 2) != ARGUMENTS || self.loop_at == (p, d) {
                        return Turn::Not;
                    }
                    self.fill(p);
                    self.pop();
                    return Turn::Took(if c == b',' {
                        self.put_code(p, p + 1, ARGUMENT_SEP)
                    } else {
                        self.close_code(p, GROUP_END)
                    });
                }
                // `VD_VALUE`'s pop at `;` or `,`, then `vars_decl`'s end or separator there
                (VD_VALUE, b';' | b',') => {
                    let d = self.depth;
                    if d < 3 || self.ctx_at(d - 2) != VARS_DECL || self.loop_at == (p, d) {
                        return Turn::Not;
                    }
                    self.fill(p);
                    self.pop();
                    return Turn::Took(if c == b',' {
                        self.put_code(p, p + 1, VARIABLES_SEP)
                    } else {
                        self.close_code(p, TERM_STMT)
                    });
                }
                (IF_GROUP, b')') => return Turn::Took(self.close_code(p, M_GROUP_END)),
                (GROUP | GROUP_IN, b')') => return Turn::Took(self.close_code(p, GROUP_END)),
                (TERNARY, b':') => return Turn::Took(self.close_code(p, TERNARY_OP)),
                (ACCESSOR_ARGS, b',') => return Turn::Took(self.put_code(p, p + 1, ACCESSOR_SEP)),
                (ACCESSOR_ARGS, b']') => return Turn::Took(self.close_code(p, M_BRACKETS_END)),
                // their pops, and `nosemi`'s `;`: none takes it
                (ARG_EXPR | GROUP | NOSEMI | LAMBDA_BODY | MV_VALUE, b';') => {
                    return Turn::Not;
                }
                (MV_VALUE, b',') => return Turn::Not,
                _ => {}
            }
        }
        if CLASS[usize::from(c)] & (WORD | DIGIT) == WORD {
            let Some(e) = self.name_here(p) else {
                return Turn::Not;
            };
            if top == ACCESSOR_ARGS {
                // `({{name}})\s*(:)` first
                let s = clike::skip_blanks(&b[..n], e);
                if s < n && matches!(b[s], b':' | 0x80..) {
                    return Turn::Not;
                }
            }
            return Turn::Name(e);
        }
        let next = if p + 1 < n { b[p + 1] } else { 0 };
        let q = match c {
            b'.' => self.put_code(p, p + 1, DOT),
            b';' => self.close_code(p, TERM_STMT),
            // a decimal integer that no letter, `_`, `.` or digit past ASCII goes on from (and
            // no `0` before more digits): `literals`' number, `(?:0|[1-9]{{dec_digits}}?)`
            b'0'..=b'9' if let Some(d) = plain_int(&b[..n], p) => self.put_code(p, d, DECIMAL),
            b'0'..=b'9' | b'"' | b'\'' => match self.literals(p) {
                Some(q) => q,
                None => return Turn::Not,
            },
            b'-' if next == b'>' => self.put_code(p, p + 2, ARROW),
            b'?' if next == b'.' => self.put_code(p, p + 2, NULL_COALESCING),
            b'+' | b'-' | b'*' | b'/' | b'%' | b'^' | b'|' | b'&' | b'<' | b'>' | b'=' | b'!'
            | b'~' | b'?' => match self.operator(p) {
                Some(e) => self.put_code(p, e, OPERATOR),
                None if c == b'=' => self.put_code(p, p + 1, ASSIGNMENT),
                None => return Turn::Not,
            },
            _ => return Turn::Not,
        };
        Turn::Took(q)
    }

    /// `line_of_code_in_no_semicolon` at a name of ASCII `p..e` ([`Scan::nosemi_name`]), a
    /// plain one decided here: no keyword, not `_…`, no `<` after it, and after its blanks
    /// nothing that starts another of the name's patterns but a call's `(`.
    #[inline(always)]
    pub(super) fn expr_name(&mut self, p: usize, e: usize) -> Option<usize> {
        let id = if self.b[p].is_ascii_lowercase() {
            words::get(&self.b[p..e])
        } else {
            0
        };
        self.expr_name_id(p, e, id)
    }

    /// [`Scan::expr_name`] with the name's word id looked up ([`Scan::nosemi_name_id`]'s).
    #[inline(always)]
    pub(super) fn expr_name_id(&mut self, p: usize, e: usize, id: u8) -> Option<usize> {
        let (b, n) = (self.b, self.b.len());
        if id != 0 {
            // `true`, `false`, `null`: no other of the name's patterns comes first
            if words::flags(id) & (words::KWS | words::LINQ | words::LANG) == words::LANG {
                return Some(self.put_code(p, e, LANGUAGE));
            }
            return self.nosemi_name_id(p, e, id);
        }
        if b[p] == b'_' || (e < n && b[e] == b'<') {
            return self.nosemi_name_id(p, e, id);
        }
        let s = blanks(b, n, e);
        if s < n {
            let after = if s + 1 < n { b[s + 1] } else { 0 };
            match b[s] {
                // `((?!{{reserved}}\b){{name}})\s*(\()`
                b'(' => {
                    self.put_code(p, e, FC_FUNCTION);
                    self.put_code(e, s, FUNCTION_CALL);
                    let q = self.put_code(s, s + 1, FC_GROUP_BEGIN);
                    self.push(ARGUMENTS);
                    return Some(q);
                }
                b'[' | 0x80.. => return self.nosemi_name_id(p, e, 0),
                b'?' if after == b'[' => return self.nosemi_name_id(p, e, 0),
                b'=' if s > e && after == b'>' => return self.nosemi_name_id(p, e, 0),
                b':' if after == b':' => return self.nosemi_name_id(p, e, 0),
                _ => {}
            }
        }
        Some(self.put_code(p, e, VAR_OTHER))
    }

    /// `arguments`' turn at `p`, where it is one [`Scan::expr`] takes: its `,` and `)`, or a
    /// name that starts an argument's expression (not `out`, `ref`, a parameter's `name =` or
    /// `name:`), the push and the expression's turn at the name.
    #[inline(always)]
    fn expr_arg(&mut self, p: usize) -> Turn {
        let (b, n) = (self.b, self.b.len());
        match b[p] {
            b',' => Turn::Took(self.put_code(p, p + 1, ARGUMENT_SEP)),
            b')' => Turn::Took(self.close_code(p, GROUP_END)),
            c if CLASS[usize::from(c)] & (WORD | DIGIT) == WORD => {
                let Some(e) = self.name_here(p) else {
                    return Turn::Not;
                };
                if e - p == 3 && (crate::same(&b[p..e], b"out") || crate::same(&b[p..e], b"ref")) {
                    return Turn::Not;
                }
                let s = blanks(b, n, e);
                if s < n && matches!(b[s], b'=' | b':' | 0x80..) {
                    return Turn::Not;
                }
                // `enter`'s push of the expression, which takes nothing, then the expression's
                // turn at the name
                self.fill(p);
                self.push(ARG_EXPR);
                self.loop_at = (p, self.depth);
                Turn::Name(e)
            }
            // a string's or a number's start: no pattern of `arguments` but `(?=\S)`'s push of
            // the expression, whose turn there (`literals`') takes the string's start or the
            // number; room left for the run to go on
            b'"' | b'0'..=b'9' if self.depth + 3 < SYNTECT_DEPTH => {
                self.fill(p);
                self.push(ARG_EXPR);
                self.loop_at = (p, self.depth);
                Turn::Pushed
            }
            _ => Turn::Not,
        }
    }

    /// The byte at `p` is `leaf`, then the top context pops ([`Scan::close`]). Returns `p + 1`.
    #[inline(always)]
    fn close_code(&mut self, p: usize, leaf: u16) -> usize {
        let q = self.put_code(p, p + 1, leaf);
        self.pop();
        q
    }

    /// Bytes `s..e` are `leaf` in the top context: [`Scan::put`] inline, its usual case here
    /// (`s..e` on the line, at or past where the spans end, the text before plain, the spans
    /// with room), the rest [`Scan::put_kind`]'s. Returns `e`.
    #[inline(always)]
    pub(super) fn put_code(&mut self, s: usize, e: usize, leaf: u16) -> usize {
        let k = self.cur() | leaf;
        let at = self.at;
        if s < at
            || e < s
            || e > self.b.len()
            || (s > at && self.text != PLAIN)
            || self.spans.len() == self.spans.capacity()
        {
            self.put_kind(s, e, k);
            return e;
        }
        if e > s && k != PLAIN {
            if let Some(last) = self.spans.last_mut()
                && last.end as usize == s
                && last.kind == k
                && self.merge
            {
                last.end = e as u32;
            } else {
                self.spans.push(Span {
                    start: s as u32,
                    end: e as u32,
                    kind: k,
                });
            }
        }
        self.at = e;
        e
    }
}
