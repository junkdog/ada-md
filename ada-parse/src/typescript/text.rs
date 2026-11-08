//! The text frames of TypeScript's own scanner ([`super::Scan`]): strings, templates, comments
//! (JSDoc's tags, types and names in a `/** … */`), regex literals and regexes in `RegExp(…)`'s
//! strings, and TSX's tags, attributes and children. Each opens from the code loop and, where
//! it runs past the line, goes on as a frame whose turn ([`Scan::text_turn`]) the loop hands
//! the next line to.

use super::*;
use crate::prim::stops;

/// The bytes a template's text stops at: its end, an escape, a substitution's `$`.
static TPL_STOPS: [bool; 256] = stops(b"`\\$");
/// The bytes a JSX element's children stop at.
static CHILD_STOPS: [bool; 256] = stops(b"<{&");
/// Where a regex in a `"…"` or a `'…'` yields to its string (its quote, an escape), and where a
/// regex literal's body can end.
static DQ_STOPS: [bool; 256] = stops(b"\"\\");
static SQ_STOPS: [bool; 256] = stops(b"'\\");
static REGEX_STOPS: [bool; 256] = stops(b"/");
/// A template's, with its escapes: where its patterns start in a regex template.
static TEMPLATE_RE_STOPS: [bool; 256] = stops(b"`$\\");

/// A regex literal's body: its `/` ends it where no group or class is open.
static RE_EMBED: regex::Embed<'static> = regex::Embed {
    dialect: regex::Dialect::Ecma,
    cooked: false,
    yields: &REGEX_STOPS,
    stop: |_, _, _| false,
    arg: 0,
    escape: regex::java_escape,
    top_only: true,
    continues: false,
    base: RE_KINDS,
    text: RE,
};

/// A regex in a `'…'`, a `"…"` and a template: the host's end, its escapes and a template's
/// `${` come first ([`RE_HOSTS`]' order past the literal's).
static RE_STR_EMBED: [regex::Embed<'static>; 3] = [
    re_str_embed(&SQ_STOPS, 1, STR_S),
    re_str_embed(&DQ_STOPS, 2, STR_D),
    regex::Embed {
        stop: |b, p, _| b[p] == b'`' || b.get(p + 1) == Some(&b'{'),
        escape: |b, p| escape(b, p),
        ..re_str_embed(&TEMPLATE_RE_STOPS, 3, TPL)
    },
];

const fn re_str_embed(yields: &'static [bool; 256], host: u16, text: u16) -> regex::Embed<'static> {
    regex::Embed {
        dialect: regex::Dialect::Ecma,
        cooked: true,
        yields,
        stop: |_, _, _| true,
        arg: 0,
        escape: |b, p| escape(b, p),
        top_only: false,
        continues: true,
        base: RE_KINDS + host * regex::KINDS as u16,
        text,
    }
}

/// A string escape at `p`, a `\` (bat's `string-character-escape`): `\xHH`, `\uHHHH`,
/// `\u{H…}`, an octal's digits, else the `\` and the character after it. Where it ends.
fn escape(b: &[u8], p: usize) -> usize {
    let hex = |i: usize| b.get(i).is_some_and(u8::is_ascii_hexdigit);
    let q = p + 2;
    match b.get(p + 1) {
        None => p + 1,
        Some(b'x') if hex(q) && hex(q + 1) => q + 2,
        Some(b'u') if b.get(q) == Some(&b'{') && hex(q + 1) => {
            let mut e = q + 1;
            while hex(e) {
                e += 1;
            }
            if b.get(e) == Some(&b'}') { e + 1 } else { q }
        }
        Some(b'u') if (q..q + 4).all(hex) => q + 4,
        Some(b'0'..=b'7') => {
            let mut e = p + 2;
            while e < p + 4 && b.get(e).is_some_and(|c| matches!(c, b'0'..=b'7')) {
                e += 1;
            }
            e
        }
        Some(&c) => p + 1 + char_len(c),
    }
}

impl<K: Sink> Scan<'_, '_, K> {
    /// A text frame's turn at `p` (`top`): where the scan goes; the line's end handled here
    /// (a string not continued ends with its line).
    #[inline(never)]
    pub(super) fn text_turn(&mut self, p: usize, top: u8) -> usize {
        match top {
            T_STR_D | T_STR_S => self.string_text(p, top),
            T_TPL => self.template_text(p),
            T_BLOCK | T_DOC => self.comment_text(p, top == T_DOC),
            T_DOC_INLINE => self.doc_inline_text(p),
            T_JSX_TAG => self.jsx_tag(p),
            T_JSX_CHILD => self.jsx_children(p),
            T_JSX_STR_D | T_JSX_STR_S => self.jsx_string_text(p, top),
            T_RE_STR..=T_RE_STR_LAST => self.regex_string(p, top),
            T_RE_LIT..=T_RE_LIT_LAST => {
                // a literal whose group or class the line before left open
                self.pop();
                self.at = p;
                self.regex_body(p, top & regex::ECMA_STATE)
            }
            _ => {
                // a frame no turn reads: dropped
                self.pop();
                p
            }
        }
    }

    // --- strings ---

    /// A `"` or `'` at `p` (`q`) in code: the string whole where it closes on the line before
    /// any `\`, else its frame.
    #[inline(always)]
    pub(super) fn string(&mut self, p: usize, q: u8) -> usize {
        let b = self.b;
        let keyed =
            matches!(self.prev, P_KEY | P_MEMBER) && matches!(self.top(), F_OBJECT | F_CLASS_BODY);
        self.prev = if keyed { P_KEYED } else { P_VALUE };
        if p == self.re_arg {
            return self.regex_arg(p, q);
        }
        let (begin, text, end) = if q == b'"' {
            (STR_D_BEGIN, STR_D, STR_D_END)
        } else {
            (STR_S_BEGIN, STR_S, STR_S_END)
        };
        let i = crate::prim::find_in(b, p + 1, b.len(), [q, b'\\']);
        if i < b.len() && b[i] == q {
            // a quoted key before `:`: a function as its value names it
            let key = keyed && self.top() == F_OBJECT;
            let mark = if key {
                Some(self.k.mark(self.out))
            } else {
                None
            };
            self.tok(p, p + 1, begin);
            self.tok(p + 1, i, text);
            self.tok(i, i + 1, end);
            if let Some(mark) = mark {
                let s = skip_blanks(b, i + 1);
                if b.get(s) == Some(&b':') {
                    self.assigned_at(mark, p, i + 1, s, A_QUOTED);
                    self.tok_far(s, s + 1, KEY_VALUE);
                    self.prev = P_OP;
                    return s + 1;
                }
            }
            return i + 1;
        }
        self.tok_far(p, p + 1, begin);
        self.push(if q == b'"' { T_STR_D } else { T_STR_S });
        self.at = p + 1;
        p + 1
    }

    /// A string's text from `p` (`top` its frame): its escapes, its end, or the line's.
    fn string_text(&mut self, mut p: usize, top: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let (q, text, end, esc) = if top == T_STR_D {
            (b'"', STR_D, STR_D_END, STR_D_ESC)
        } else {
            (b'\'', STR_S, STR_S_END, STR_S_ESC)
        };
        loop {
            let i = crate::prim::find_in(b, p, n, [q, b'\\']);
            if i >= n {
                // the string ends with its line
                self.fill_as(n, text);
                self.pop();
                return n;
            }
            self.fill_as(i, text);
            if b[i] == q {
                self.tok_far(i, i + 1, end);
                self.at = i + 1;
                self.pop();
                return i + 1;
            }
            if i + 1 >= n {
                // `\` at the line's end: the string goes on
                self.tok_far(i, n, esc);
                self.at = n;
                return n;
            }
            let e = escape(b, i);
            self.tok_far(i, e, esc);
            self.at = e;
            p = e;
        }
    }

    /// The string or template opening at `p` as `RegExp(`'s argument: a regex in ECMAScript's
    /// dialect, above the string's frame.
    #[inline(never)]
    fn regex_arg(&mut self, p: usize, q: u8) -> usize {
        let (begin, frame) = match q {
            b'"' => (STR_D_BEGIN, T_STR_D),
            b'\'' => (STR_S_BEGIN, T_STR_S),
            _ => (TPL_BEGIN, T_TPL),
        };
        self.tok_far(p, p + 1, begin);
        self.push(frame);
        self.push(T_RE_STR);
        self.at = p + 1;
        p + 1
    }

    /// A regex string's turn at `p` (`top`): the host string's or template's end, escapes
    /// (but `\\`, the regex's `\`) and `${` first, then the regex.
    fn regex_string(&mut self, p: usize, top: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let host = self.under();
        let c = b.get(p).copied();
        let which = if host == T_TPL {
            match c {
                Some(b'`') => {
                    self.pop();
                    self.pop();
                    self.tok_far(p, p + 1, TPL_END);
                    self.at = p + 1;
                    self.prev = P_VALUE;
                    return p + 1;
                }
                Some(b'$') if b.get(p + 1) == Some(&b'{') => {
                    self.tok_far(p, p + 2, SUBST_BEGIN);
                    self.push(F_SUBST);
                    self.substs = (self.substs + 1).min(SUBSTS_MAX);
                    self.prev = P_OP;
                    return p + 2;
                }
                Some(b'\\') if b.get(p + 1) != Some(&b'\\') => {
                    let e = escape(b, p).min(n);
                    self.tok_far(p, e, TPL_ESC);
                    self.at = e;
                    return e;
                }
                _ => {}
            }
            2
        } else {
            let (q, end, esc) = if host == T_STR_D {
                (b'"', STR_D_END, STR_D_ESC)
            } else {
                (b'\'', STR_S_END, STR_S_ESC)
            };
            if c == Some(q) {
                self.pop();
                self.pop();
                self.tok_far(p, p + 1, end);
                self.at = p + 1;
                self.prev = P_VALUE;
                return p + 1;
            }
            if p >= n {
                self.pop();
                self.pop();
                return n;
            }
            if c == Some(b'\\') && b.get(p + 1) != Some(&b'\\') {
                // the string's escapes first (a `\\` is the regex's)
                let e = escape(b, p).min(n);
                self.tok_far(p, e, esc);
                self.at = e;
                return e;
            }
            usize::from(host == T_STR_D)
        };
        if p >= n {
            return n;
        }
        self.k.close(self.out);
        let r = regex::embedded(
            b,
            p,
            top & regex::ECMA_STATE,
            &RE_STR_EMBED[which],
            &mut self.at,
            self.out,
        );
        self.k.reopen(self.out, 16);
        match r {
            Some((q, st)) => {
                self.set_top(T_RE_STR | st);
                q.max(p + 1).min(n)
            }
            None => n,
        }
    }

    // --- templates ---

    /// A template's `` ` `` at `p`.
    #[inline(never)]
    pub(super) fn template(&mut self, p: usize) -> usize {
        if p == self.re_arg {
            return self.regex_arg(p, b'`');
        }
        self.tok_far(p, p + 1, TPL_BEGIN);
        self.push(T_TPL);
        self.at = p + 1;
        p + 1
    }

    /// A template's text from `p`: to its end, a substitution, or the line's end.
    fn template_text(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        loop {
            let mut i = p;
            while i < n && !TPL_STOPS[usize::from(b[i])] {
                i += 1;
            }
            if i >= n {
                self.fill_as(n, TPL);
                return n;
            }
            match b[i] {
                b'`' => {
                    self.fill_as(i, TPL);
                    self.tok_far(i, i + 1, TPL_END);
                    self.at = i + 1;
                    self.pop();
                    self.prev = P_VALUE;
                    return i + 1;
                }
                b'$' if b.get(i + 1) == Some(&b'{') => {
                    self.fill_as(i, TPL);
                    self.tok(i, i + 2, SUBST_BEGIN);
                    self.at = i + 2;
                    if (F_TYPE..TEXT).contains(&self.under()) {
                        // a template literal type's substitution holds a type
                        self.push(F_T_SUBST);
                        self.prev = P_OP;
                        return i + 2;
                    }
                    self.push(F_SUBST);
                    self.substs = (self.substs + 1).min(SUBSTS_MAX);
                    self.prev = P_OP;
                    return i + 2;
                }
                b'\\' => {
                    let e = escape(b, i).min(n);
                    self.fill_as(i, TPL);
                    self.tok_far(i, e, TPL_ESC);
                    self.at = e;
                    p = e;
                }
                _ => p = i + 1,
            }
        }
    }

    // --- comments ---

    /// A `//` at `p`: the rest of the line; the blanks before it the comment's where it starts
    /// the line.
    #[inline(always)]
    pub(super) fn line_comment(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        if p > 0 && b[..p].iter().all(|&c| c == b' ' || c == b'\t') {
            self.tok(0, p, LEADING);
        }
        if b.get(p + 2) == Some(&b'/') && has(b, skip_blanks(b, p + 3), b"<") {
            return self.directive(p);
        }
        // a CRLF line's `\r` stays plain, as bat's `$` leaves it
        let end = if b[n - 1] == b'\r' { n - 1 } else { n };
        self.tok(p, p + 2, LINE_MARK);
        self.tok(p + 2, end, LINE);
        n
    }

    /// A `///` directive's line at `p` (`/// <reference path="…" />`).
    #[inline(never)]
    fn directive(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        self.tok_far(p, p + 3, DIRECTIVE_MARK);
        let mut i = p + 3;
        while i < n {
            let c = b[i];
            match c {
                b'<' => {
                    self.tok_far(i, i + 1, DIRECTIVE_TAG_MARK);
                    let mut e = i + 1;
                    while e < n && NAME[usize::from(b[e])] {
                        e += 1;
                    }
                    self.tok_far(i + 1, e, DIRECTIVE_TAG);
                    i = e;
                }
                b'/' if b.get(i + 1) == Some(&b'>') => {
                    self.tok_far(i, i + 2, DIRECTIVE_TAG_MARK);
                    i += 2;
                }
                b'>' => {
                    self.tok_far(i, i + 1, DIRECTIVE_TAG_MARK);
                    i += 1;
                }
                b'=' => {
                    self.tok_far(i, i + 1, DIRECTIVE_EQ);
                    i += 1;
                }
                b'"' | b'\'' => {
                    let (bg, t, en) = if c == b'"' {
                        (DIRECTIVE_STR_D_BEGIN, DIRECTIVE_STR_D, DIRECTIVE_STR_D_END)
                    } else {
                        (DIRECTIVE_STR_S_BEGIN, DIRECTIVE_STR_S, DIRECTIVE_STR_S_END)
                    };
                    let mut e = i + 1;
                    while e < n && b[e] != c {
                        e += 1;
                    }
                    self.tok_far(i, i + 1, bg);
                    self.tok_far(i + 1, e, t);
                    if e < n {
                        self.tok_far(e, e + 1, en);
                        e += 1;
                    }
                    i = e;
                }
                _ if NAME[usize::from(c)] => {
                    let mut e = i + 1;
                    while e < n && NAME[usize::from(b[e])] {
                        e += 1;
                    }
                    self.tok_far(i, e, DIRECTIVE_ATTR);
                    i = e;
                }
                _ => {
                    self.tok_far(i, i + 1, DIRECTIVE);
                    i += 1;
                }
            }
        }
        n
    }

    /// A `/*` at `p`: whole where it closes on the line, else its frame.
    #[inline(never)]
    pub(super) fn block_comment(&mut self, p: usize) -> usize {
        let b = self.b;
        let doc = b.get(p + 2) == Some(&b'*') && b.get(p + 3) != Some(&b'/');
        let (open, mark) = if doc { (3, DOC_MARK) } else { (2, BLOCK_MARK) };
        self.tok_far(p, p + open, mark);
        self.push(if doc { T_DOC } else { T_BLOCK });
        self.at = p + open;
        self.comment_text(p + open, doc)
    }

    /// A block comment's text from `p`: to its `*/` or the line's end; a doc comment's JSDoc.
    fn comment_text(&mut self, p: usize, doc: bool) -> usize {
        let b = self.b;
        let n = b.len();
        let text = if doc { DOC } else { BLOCK_TEXT };
        let mark = if doc { DOC_MARK } else { BLOCK_MARK };
        let mut i = p;
        loop {
            i = if doc {
                crate::prim::find_in(b, i, n, *b"*@{")
            } else {
                crate::prim::find_in(b, i, n, *b"*")
            };
            if i >= n {
                self.fill_as(n, text);
                return n;
            }
            match b[i] {
                b'*' if b.get(i + 1) == Some(&b'/') => {
                    self.fill_as(i, text);
                    self.tok_far(i, i + 2, mark);
                    self.at = i + 2;
                    self.pop();
                    return i + 2;
                }
                b'@' => {
                    i = self.jsdoc_tag(i);
                }
                b'{' if b.get(i + 1) == Some(&b'@') => {
                    i = self.jsdoc_inline(i);
                }
                _ => i += 1,
            }
        }
    }

    /// A JSDoc block tag's `@` at `p` (bat's `docblock`): the tag,
    /// and what the tag takes: a parameter's name, a `{type}` and a name, a `{type}`, a
    /// namepath, `@see`'s link. Where the text goes on.
    #[inline(never)]
    fn jsdoc_tag(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let s = p + 1;
        let mut e = s;
        while e < n && (b[e].is_ascii_alphanumeric() || b[e] == b'_' || b[e] == b'$') {
            e += 1;
        }
        if e == s {
            return p + 1;
        }
        let tag = &b[s..e];
        let after = skip_blanks(b, e);
        let gap = after > e && after < n;
        let next = if gap { b[after] } else { 0 };
        let tagged = |me: &mut Self, end: usize| {
            me.fill_as(p, DOC);
            me.tok_far(p, p + 1, JD_TAG_AT);
            me.tok_far(p + 1, end, JD_TAG);
            me.at = end;
        };
        let word = |c: u8| c.is_ascii_alphabetic() || c == b'_' || c == b'$';
        // a parameter's name, no type
        if gap
            && word(next)
            && matches!(
                tag,
                b"arg"
                    | b"argument"
                    | b"const"
                    | b"constant"
                    | b"member"
                    | b"namespace"
                    | b"param"
                    | b"var"
            )
        {
            let mut k = after + 1;
            while k < n
                && (b[k].is_ascii_alphanumeric()
                    || matches!(b[k], b'_' | b'$' | b'.' | b'[' | b']'))
            {
                k += 1;
            }
            tagged(self, e);
            self.fill_as(after, DOC);
            self.tok_far(after, k, JD_VAR);
            self.at = k;
            return k;
        }
        let param = matches!(
            tag,
            b"arg"
                | b"argument"
                | b"const"
                | b"constant"
                | b"member"
                | b"namespace"
                | b"param"
                | b"prop"
                | b"property"
                | b"var"
        );
        let typed = param
            || matches!(
                tag,
                b"define"
                    | b"enum"
                    | b"exception"
                    | b"export"
                    | b"extends"
                    | b"lends"
                    | b"implements"
                    | b"modifies"
                    | b"private"
                    | b"protected"
                    | b"returns"
                    | b"return"
                    | b"suppress"
                    | b"this"
                    | b"throws"
                    | b"type"
                    | b"yields"
                    | b"yield"
                    | b"typedef"
            );
        if gap && next == b'{' && typed {
            tagged(self, e);
            return self.jsdoc_typed(after, param);
        }
        if gap
            && matches!(
                tag,
                b"alias"
                    | b"augments"
                    | b"callback"
                    | b"constructs"
                    | b"emits"
                    | b"event"
                    | b"fires"
                    | b"exports"
                    | b"export"
                    | b"extends"
                    | b"external"
                    | b"function"
                    | b"func"
                    | b"host"
                    | b"lends"
                    | b"listens"
                    | b"interface"
                    | b"memberof"
                    | b"method"
                    | b"module"
                    | b"mixes"
                    | b"mixin"
                    | b"name"
                    | b"requires"
                    | b"this"
                    | b"typedef"
                    | b"uses"
            )
            && !matches!(next, b'{' | b'}' | b'@')
        {
            let k = doc_run(b, after);
            if k > after {
                tagged(self, e);
                self.fill_as(after, DOC);
                self.tok_far(after, k, JD_TYPE);
                self.at = k;
                return k;
            }
        }
        if gap && tag == b"see" {
            let k = doc_run(b, after);
            if has(b, after, b"http://") || has(b, after, b"https://") {
                tagged(self, e);
                self.fill_as(after, DOC);
                self.tok_far(after, k, JD_LINK);
                self.at = k;
                return k;
            }
            if k > after && next != b'{' && next != b'[' && next != b'@' {
                tagged(self, e);
                self.fill_as(after, DOC);
                self.tok_far(after, k, JD_TYPE);
                self.at = k;
                return k;
            }
        }
        if doc_tag(tag) || (e < n && matches!(b[e], b' ' | b'\t')) || e == n {
            tagged(self, e);
            return e;
        }
        p + 1
    }

    /// A `{type}` at `p` after a typed tag, and after a parameter's (`param`) its name,
    /// `[name=default]` an optional one's. Where the text goes on.
    fn jsdoc_typed(&mut self, p: usize, param: bool) -> usize {
        let b = self.b;
        let n = b.len();
        let mut d = 0usize;
        let mut q = p;
        while q < n {
            match b[q] {
                b'{' => d += 1,
                b'}' => {
                    d -= 1;
                    if d == 0 {
                        break;
                    }
                }
                _ => {}
            }
            q += 1;
        }
        if q >= n {
            return p;
        }
        self.fill_as(p, DOC);
        self.tok_far(p, p + 1, JD_TYPE_BEGIN);
        self.tok_far(p + 1, q, JD_TYPE);
        self.tok_far(q, q + 1, JD_TYPE_END);
        self.at = q + 1;
        if !param {
            return q + 1;
        }
        let s = skip_blanks(b, q + 1);
        if b.get(s) == Some(&b'[') {
            let mut r = s + 1;
            while r < n && b[r] != b']' {
                r += 1;
            }
            if r < n {
                self.fill_as(s, DOC);
                self.tok_far(s, s + 1, JD_OPT_BEGIN);
                let eq = b[s + 1..r]
                    .iter()
                    .position(|&c| c == b'=')
                    .map(|k| s + 1 + k);
                match eq {
                    Some(k) => {
                        self.tok_far(s + 1, k, JD_VAR);
                        self.tok_far(k, k + 1, JD_OPT_EQ);
                        self.tok_far(k + 1, r, JD_OPT_VALUE);
                    }
                    None => {
                        self.tok_far(s + 1, r, JD_VAR);
                    }
                }
                self.tok_far(r, r + 1, JD_OPT_END);
                self.at = r + 1;
                return r + 1;
            }
            return s;
        }
        let mut r = s;
        while r < n
            && (NAME[usize::from(b[r])] || matches!(b[r], b'.' | b'[' | b']') || b[r] >= 0x80)
        {
            r += 1;
        }
        if r > s {
            self.fill_as(s, DOC);
            self.tok_far(s, r, JD_VAR);
            self.at = r;
        }
        r.max(self.at)
    }

    /// An inline tag's `{@` at `p` (`{@link Foo}`): where the text goes on.
    #[inline(never)]
    fn jsdoc_inline(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut e = p + 2;
        while e < n && b[e].is_ascii_alphabetic() {
            e += 1;
        }
        if e == p + 2 {
            return p + 1;
        }
        let Some(close) = b[e..].iter().position(|&c| c == b'}').map(|k| e + k) else {
            return self.doc_inline_open(p, e);
        };
        self.fill_as(p, DOC);
        self.tok_far(p, p + 1, JD_INLINE_BEGIN);
        self.tok_far(p + 1, p + 2, JD_INLINE_TAG_AT);
        self.tok_far(p + 2, e, JD_INLINE_TAG);
        let s = skip_blanks(b, e);
        if s > e {
            self.tok_far(e, s, JD_INLINE);
        }
        // the target, then a description after a `|` or a blank
        let mut t = s;
        while t < close && !matches!(b[t], b' ' | b'\t' | b'|') {
            t += 1;
        }
        if t > s {
            self.tok_far(s, t, JD_INLINE_DESC);
        }
        if t < close {
            self.tok_far(t, close, JD_INLINE);
        }
        self.tok_far(close, close + 1, JD_INLINE_END);
        self.at = close + 1;
        close + 1
    }

    /// An inline tag `{@tag` at `p..e` that doesn't close on its line: to the line's end, its
    /// target a description's as on one line, its frame kept (bat's); where a `*/` follows,
    /// the `{` the text's.
    #[cold]
    #[inline(never)]
    fn doc_inline_open(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let n = b.len();
        if b[e..].windows(2).any(|w| w == b"*/") {
            return p + 1;
        }
        self.fill_as(p, DOC);
        self.tok_far(p, p + 1, JD_INLINE_BEGIN);
        self.tok_far(p + 1, p + 2, JD_INLINE_TAG_AT);
        self.tok_far(p + 2, e, JD_INLINE_TAG);
        let s = skip_blanks(b, e);
        if s > e {
            self.tok_far(e, s, JD_INLINE);
        }
        let mut t = s;
        while t < n && !matches!(b[t], b' ' | b'\t' | b'|') {
            t += 1;
        }
        if t > s {
            self.tok_far(s, t, JD_INLINE_DESC);
        }
        if t < n {
            self.tok_far(t, n, JD_INLINE);
        }
        self.push(T_DOC_INLINE);
        self.at = n;
        n
    }

    /// A line in an inline tag the line before left open, from `p`: the tag's to its `}`, or
    /// to a `*/` that closes the comment, else the whole line.
    #[inline(never)]
    fn doc_inline_text(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut i = p;
        while i < n {
            match b[i] {
                b'}' => {
                    self.fill_as(p, DOC);
                    if i > p {
                        self.tok_far(p, i, JD_INLINE);
                    }
                    self.tok_far(i, i + 1, JD_INLINE_END);
                    self.pop();
                    self.at = i + 1;
                    return i + 1;
                }
                b'*' if b.get(i + 1) == Some(&b'/') => {
                    self.fill_as(p, DOC);
                    if i > p {
                        self.tok_far(p, i, JD_INLINE);
                    }
                    self.pop();
                    self.at = i;
                    return i;
                }
                _ => i += 1,
            }
        }
        self.fill_as(p, DOC);
        if n > p {
            self.tok_far(p, n, JD_INLINE);
        }
        self.at = n;
        n
    }

    // --- regex literals ---

    /// A regex literal's `/` at `p`: its body on [`crate::regex`], its end and flags; it ends
    /// with its line where it doesn't close on it.
    #[inline(never)]
    pub(super) fn regex_literal(&mut self, p: usize) -> usize {
        self.tok_far(p, p + 1, RE_BEGIN);
        self.at = p + 1;
        self.regex_body(p + 1, 0)
    }

    /// A regex literal's body from `p` in regex state `st`: to its `/` and flags, or to the
    /// line's end, its frame kept where a group or a class is open there.
    fn regex_body(&mut self, p: usize, mut st: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let mut q = p;
        while q < n {
            if b[q] == b'/' && st == 0 {
                break;
            }
            self.k.close(self.out);
            let r = regex::embedded(b, q, st, &RE_EMBED, &mut self.at, self.out);
            self.k.reopen(self.out, 16);
            match r {
                Some((e, s)) => {
                    if e <= q && !(b.get(e) == Some(&b'/') && s == 0) {
                        q += 1;
                    } else {
                        q = e;
                    }
                    st = s;
                    if b.get(q) == Some(&b'/') && st == 0 {
                        break;
                    }
                }
                None => {
                    q = n;
                    break;
                }
            }
        }
        self.prev = P_VALUE;
        if q >= n {
            self.fill_as(n, RE);
            if st != 0 {
                self.push(T_RE_LIT | st);
            }
            return n;
        }
        self.fill_as(q, RE);
        self.tok_far(q, q + 1, RE_END);
        let mut f = q + 1;
        while f < n && matches!(b[f], b'd' | b'g' | b'i' | b'm' | b's' | b'u' | b'v' | b'y') {
            f += 1;
        }
        self.tok_far(q + 1, f, RE_FLAGS);
        self.at = f;
        f
    }

    // --- JSX ---

    /// A JSX tag's `<` at `p` (a name or `>` after it): its name, and the tag's frame.
    #[inline(never)]
    pub(super) fn jsx_open(&mut self, p: usize) -> usize {
        let b = self.b;
        if b.get(p + 1) == Some(&b'>') {
            // a fragment
            self.tok_far(p, p + 1, JSX_TAG_BEGIN);
            self.tok_far(p + 1, p + 2, JSX_TAG_END);
            self.push(T_JSX_CHILD);
            return p + 2;
        }
        let e = tag_name_end(b, p + 1);
        self.tok(p, p + 1, JSX_TAG_BEGIN);
        self.tag_name(p + 1, e);
        self.push(T_JSX_TAG);
        e
    }

    /// A tag's name `s..e`: a component's (capitalized, or a path), else an element's.
    fn tag_name(&mut self, mut s: usize, e: usize) {
        if let Some(k) = self.b[s..e].iter().position(|&c| c == b':') {
            // a namespace's, `svg:rect`
            self.tok_far(s, s + k, JSX_TAG_NS);
            self.tok_far(s + k, s + k + 1, JSX_NS_SEP);
            s += k + 1;
            if s >= e {
                return;
            }
        }
        let w = &self.b[s..e];
        let leaf = if w.first().is_some_and(u8::is_ascii_uppercase) || w.contains(&b'.') {
            JSX_COMPONENT
        } else {
            JSX_TAG_NAME
        };
        self.tok(s, e, leaf);
    }

    /// An opening tag's turn at `p`: its attributes, their values, its end.
    fn jsx_tag(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let p = skip_blanks(b, p);
        if p >= n {
            return n;
        }
        match b[p] {
            b'/' if b.get(p + 1) == Some(&b'>') => {
                self.tok_far(p, p + 2, JSX_TAG_END);
                self.pop();
                self.prev = P_VALUE;
                p + 2
            }
            b'/' if matches!(b.get(p + 1), Some(b'/')) => self.line_comment(p),
            b'/' if matches!(b.get(p + 1), Some(b'*')) => self.block_comment(p),
            b'>' => {
                self.tok(p, p + 1, JSX_TAG_END);
                self.set_top(T_JSX_CHILD);
                p + 1
            }
            b'=' => self.tok(p, p + 1, ASSIGN),
            q @ (b'"' | b'\'') => {
                self.tok(p, p + 1, if q == b'"' { STR_D_BEGIN } else { STR_S_BEGIN });
                self.push(if q == b'"' { T_JSX_STR_D } else { T_JSX_STR_S });
                self.at = p + 1;
                self.jsx_string_text(p + 1, self.top())
            }
            b'{' => {
                self.tok(p, p + 1, JSX_EMBED_BEGIN);
                self.push(F_J_EXPR);
                self.prev = P_OP;
                p + 1
            }
            b'<' => {
                // a tag's type arguments (`<Foo<T> …>`)
                self.type_args(p)
            }
            c if c.is_ascii_alphabetic() || c == b'_' || c == b'$' => {
                let mut e = p + 1;
                while e < n && (NAME[usize::from(b[e])] || b[e] == b'-' || b[e] == b':') {
                    e += 1;
                }
                self.tok(p, e, JSX_ATTR)
            }
            c => p + char_len(c),
        }
    }

    /// A JSX attribute string's text from `p`, its frame `top`.
    fn jsx_string_text(&mut self, mut p: usize, top: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let (q, text, end, ent, bad) = if top == T_JSX_STR_D {
            (b'"', STR_D, STR_D_END, JSX_STR_D_ENTITY, JSX_STR_D_BAD_AMP)
        } else {
            (b'\'', STR_S, STR_S_END, JSX_STR_S_ENTITY, JSX_STR_S_BAD_AMP)
        };
        loop {
            let i = crate::prim::find_in(b, p, n, [q, b'&']);
            if i >= n {
                self.fill_as(n, text);
                return n;
            }
            self.fill_as(i, text);
            if b[i] == q {
                self.tok(i, i + 1, end);
                self.at = i + 1;
                self.pop();
                return i + 1;
            }
            let e = self.entity(i, ent, bad, top == T_JSX_STR_D);
            self.at = e;
            p = e;
        }
    }

    /// A JSX element's children from `p`: their text plain, entities, `{…}`, a nested element,
    /// and the closing tag that ends them.
    fn jsx_children(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut i = p;
        while i < n && !CHILD_STOPS[usize::from(b[i])] {
            i += 1;
        }
        if i >= n {
            return n;
        }
        match b[i] {
            b'{' => {
                self.tok_far(i, i + 1, JSX_EMBED_BEGIN);
                self.push(F_J_EXPR);
                self.prev = P_OP;
                i + 1
            }
            b'&' => self.entity(i, JSX_ENTITY, JSX_BAD_AMP, false),
            _ => {
                if b.get(i + 1) == Some(&b'/') {
                    // a closing tag: the element ends
                    let s = i + 2;
                    let e = tag_name_end(b, s);
                    self.tok(i, s, JSX_TAG_BEGIN);
                    self.tag_name(s, e);
                    self.pop();
                    self.prev = P_VALUE;
                    let g = skip_blanks(b, e);
                    if b.get(g) == Some(&b'>') {
                        return self.tok(g, g + 1, JSX_TAG_END);
                    }
                    return e;
                }
                if matches!(
                    b.get(i + 1),
                    Some(b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' | b'>')
                ) {
                    return self.jsx_open(i);
                }
                i + 1
            }
        }
    }

    /// `&…;` at `p`: an entity (its `&` and `;` marks in children), else a bad `&` up to the
    /// last `;` before a space; else the `&` alone, plain.
    fn entity(&mut self, p: usize, ok: u16, bad: u16, dq: bool) -> usize {
        let b = self.b;
        let n = b.len();
        let run = |from: usize, f: fn(&u8) -> bool| {
            from + b[from.min(n)..].iter().take_while(|c| f(c)).count()
        };
        let alnum = run(p + 1, u8::is_ascii_alphanumeric);
        let e = if alnum > p + 1 {
            Some(alnum)
        } else if b.get(p + 1) == Some(&b'#') {
            if b.get(p + 2) == Some(&b'x') && run(p + 3, u8::is_ascii_hexdigit) > p + 3 {
                Some(run(p + 3, u8::is_ascii_hexdigit))
            } else {
                let d = run(p + 2, u8::is_ascii_digit);
                (d > p + 2).then_some(d)
            }
        } else {
            None
        };
        if let Some(e) = e
            && b.get(e) == Some(&b';')
        {
            let m = match ok {
                JSX_ENTITY => JSX_ENTITY_MARK,
                _ if dq => JSX_STR_D_ENTITY_MARK,
                _ => JSX_STR_S_ENTITY_MARK,
            };
            self.tok_far(p, p + 1, m);
            self.tok_far(p + 1, e, ok);
            return self.tok_far(e, e + 1, m);
        }
        let mut s = p + 1;
        while s < n && !matches!(b[s], b' ' | b'\t') {
            s += 1;
        }
        match b[p + 1..s].iter().rposition(|&c| c == b';') {
            Some(k) => self.tok_far(p, p + 1 + k + 1, bad),
            // a `&` before a blank or the end: bat's bad ampersand
            None if s == p + 1 => self.tok_far(p, p + 1, bad),
            None => p + 1,
        }
    }
}

/// Where a JSX tag's name from `p` ends: `[-$\w:.]`, and letters past ASCII.
fn tag_name_end(b: &[u8], mut p: usize) -> usize {
    while p < b.len() {
        let c = b[p];
        if c < 0x80 {
            if NAME[usize::from(c)] || matches!(c, b'-' | b':' | b'.') {
                p += 1;
            } else {
                break;
            }
        } else if is_word(b, p) {
            p += char_len(c);
        } else {
            break;
        }
    }
    p
}

/// `(?:[^{}@\s*]|\*[^/])+` from `i`: a JSDoc namepath's end.
fn doc_run(b: &[u8], mut i: usize) -> usize {
    while i < b.len() {
        let c = b[i];
        if matches!(c, b' ' | b'\t' | b'{' | b'}' | b'@') {
            break;
        }
        if c == b'*' {
            if i + 1 < b.len() && b[i + 1] != b'/' && !matches!(b[i + 1], b' ' | b'\t') {
                i += 2;
                continue;
            }
            break;
        }
        i += 1;
    }
    i
}

/// The JSDoc tags bat's `docblock` names as tags wherever they stand.
fn doc_tag(t: &[u8]) -> bool {
    matches!(
        t,
        b"abstract"
            | b"access"
            | b"alias"
            | b"api"
            | b"arg"
            | b"argument"
            | b"async"
            | b"attribute"
            | b"augments"
            | b"author"
            | b"beta"
            | b"borrows"
            | b"bubbles"
            | b"callback"
            | b"chainable"
            | b"class"
            | b"classdesc"
            | b"code"
            | b"config"
            | b"const"
            | b"constant"
            | b"constructor"
            | b"constructs"
            | b"copyright"
            | b"default"
            | b"defaultvalue"
            | b"define"
            | b"deprecated"
            | b"desc"
            | b"description"
            | b"dict"
            | b"emits"
            | b"enum"
            | b"event"
            | b"example"
            | b"exception"
            | b"exports"
            | b"export"
            | b"extends"
            | b"extension"
            | b"extension_for"
            | b"extensionfor"
            | b"external"
            | b"externs"
            | b"file"
            | b"fileoverview"
            | b"final"
            | b"fires"
            | b"for"
            | b"func"
            | b"function"
            | b"generator"
            | b"global"
            | b"hideconstructor"
            | b"host"
            | b"ignore"
            | b"implements"
            | b"implicitCast"
            | b"inheritDoc"
            | b"inheritdoc"
            | b"inner"
            | b"instance"
            | b"interface"
            | b"internal"
            | b"kind"
            | b"lends"
            | b"license"
            | b"listens"
            | b"main"
            | b"member"
            | b"memberof"
            | b"method"
            | b"mixes"
            | b"mixin"
            | b"mixins"
            | b"modifies"
            | b"module"
            | b"name"
            | b"namespace"
            | b"noalias"
            | b"nocollapse"
            | b"nocompile"
            | b"nosideeffects"
            | b"override"
            | b"overview"
            | b"package"
            | b"param"
            | b"polymer"
            | b"polymerBehavior"
            | b"preserve"
            | b"private"
            | b"prop"
            | b"property"
            | b"protected"
            | b"public"
            | b"readOnly"
            | b"readonly"
            | b"record"
            | b"required"
            | b"requires"
            | b"returns"
            | b"return"
            | b"see"
            | b"since"
            | b"static"
            | b"struct"
            | b"submodule"
            | b"summary"
            | b"suppress"
            | b"template"
            | b"this"
            | b"throws"
            | b"todo"
            | b"tutorial"
            | b"type"
            | b"typedef"
            | b"unrestricted"
            | b"uses"
            | b"var"
            | b"variation"
            | b"version"
            | b"virtual"
            | b"writeOnce"
            | b"yields"
            | b"yield"
    )
}
