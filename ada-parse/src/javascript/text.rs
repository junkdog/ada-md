//! The text frames of JavaScript's own scanner ([`super::Scan`]): strings, templates, comments,
//! regex literals and regexes in `RegExp(…)`'s strings, and JSX's tags, attributes and
//! children. Each opens from the code loop and, where it runs past the line, goes on as a frame
//! whose turn ([`Scan::text_turn`]) the loop hands the next line to.

use super::*;

/// The bytes a template's text stops at: its end, an escape, a substitution's `$`.
static TPL_STOPS: [bool; 256] = stops(b"`\\$");
/// The bytes a JSX element's children stop at.
static CHILD_STOPS: [bool; 256] = stops(b"<{&");

impl<K: Sink> Scan<'_, '_, K> {
    /// A text frame's turn at `p` (`top`): where the scan goes; the line's end handled here
    /// (a string not continued ends with its line).
    #[inline(never)]
    pub(super) fn text_turn(&mut self, p: usize, top: u8) -> usize {
        match top {
            T_STR_D | T_STR_S => self.string_text(p, if top == T_STR_D { b'"' } else { b'\'' }),
            T_TPL | T_TPL_PLAIN => self.template_text(p, top == T_TPL_PLAIN),
            T_BLOCK | T_DOC => self.comment_text(p, top == T_DOC),
            T_JSX_TAG => self.jsx_tag(p),
            T_JSX_CHILD => self.jsx_children(p),
            T_JSX_STR_D | T_JSX_STR_S => {
                self.jsx_string_text(p, if top == T_JSX_STR_D { b'"' } else { b'\'' })
            }
            T_RE_STR..=T_RE_STR_LAST => self.regex_string(p, top),
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
        self.prev = if matches!(self.prev, P_KEY | P_MEMBER)
            && matches!(self.top(), F_OBJECT | F_CLASS_BODY)
        {
            P_KEYED
        } else {
            P_VALUE
        };
        if p == self.re_arg {
            return self.regex_arg(p, q);
        }
        let i = crate::prim::find_in(b, p + 1, b.len(), [q, b'\\']);
        if i < b.len() && b[i] == q {
            // a quoted key before `:`: a function as its value names it
            let key = self.prev == P_KEYED && self.top() == F_OBJECT;
            let mark = if key {
                Some(self.k.mark(self.out))
            } else {
                None
            };
            self.tok(p, p + 1, STR_BEGIN);
            self.tok(p + 1, i, STR);
            self.tok(i, i + 1, STR_END);
            if let Some(mark) = mark {
                let s = skip_blanks(b, i + 1);
                if b.get(s) == Some(&b':') {
                    self.assigned_at(mark, p, i + 1, s, A_QUOTED);
                }
            }
            return i + 1;
        }
        self.tok(p, p + 1, STR_BEGIN);
        self.push(if q == b'"' { T_STR_D } else { T_STR_S });
        self.at = p + 1;
        p + 1
    }

    /// A string's text from `p`, its quote `q`: its escapes, its end, or the line's.
    fn string_text(&mut self, mut p: usize, q: u8) -> usize {
        let b = self.b;
        let n = b.len();
        loop {
            let i = crate::prim::find_in(b, p, n, [q, b'\\']);
            if i >= n {
                // `(\n)`: the string ends with its line
                self.fill_as(n, STR);
                self.pop();
                return n;
            }
            self.fill_as(i, STR);
            if b[i] == q {
                self.tok(i, i + 1, STR_END);
                self.at = i + 1;
                self.pop();
                return i + 1;
            }
            // `\\\s*\n`: the string goes on to the next line
            let s = spaces_to(b, i + 1);
            if s >= n {
                self.tok(i, n, STR_ESC_NL);
                self.at = n;
                return n;
            }
            match escape(b, i) {
                Some(e) => {
                    self.tok(i, e, STR_ESC);
                    self.at = e;
                    p = e;
                }
                None => p = i + 1,
            }
        }
    }

    /// The string or template opening at `p` as `RegExp(`'s argument: a regex in ECMAScript's
    /// dialect, above the string's frame.
    #[inline(never)]
    fn regex_arg(&mut self, p: usize, q: u8) -> usize {
        let (begin, frame) = match q {
            b'"' => (STR_BEGIN, T_STR_D),
            b'\'' => (STR_BEGIN, T_STR_S),
            _ => (TPL_BEGIN, T_TPL),
        };
        self.tok(p, p + 1, begin);
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
        let host = self.stack[self.depth - 2];
        let c = b.get(p).copied();
        let which = if host == T_TPL {
            match c {
                Some(b'`') => {
                    self.pop();
                    self.pop();
                    self.tok(p, p + 1, TPL_END);
                    self.at = p + 1;
                    return p + 1;
                }
                Some(b'\\') if b.get(p + 1) != Some(&b'\\') => {
                    if let Some(e) = escape(b, p) {
                        self.tok(p, e, ESC);
                        self.at = e;
                        return e;
                    }
                }
                Some(b'$') if b.get(p + 1) == Some(&b'{') => {
                    self.tok(p, p + 2, SUBST_BEGIN);
                    self.push(F_SUBST);
                    self.prev = P_OP;
                    return p + 2;
                }
                _ => {}
            }
            2
        } else {
            let q = if host == T_STR_D { b'"' } else { b'\'' };
            if c == Some(q) {
                self.pop();
                self.pop();
                self.tok(p, p + 1, STR_END);
                self.at = p + 1;
                return p + 1;
            }
            if p >= n {
                self.pop();
                self.pop();
                return n;
            }
            if c == Some(b'\\') && b.get(p + 1) != Some(&b'\\') {
                if spaces_to(b, p + 1) >= n {
                    self.tok(p, n, STR_ESC_NL);
                    self.at = n;
                    return n;
                }
                if let Some(e) = escape(b, p) {
                    self.tok(p, e, STR_ESC);
                    self.at = e;
                    return e;
                }
            }
            usize::from(host == T_STR_S)
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

    /// A template's `` ` `` at `p`; `plain`, styled-components' or `gql`'s, whose text is plain.
    #[inline(never)]
    pub(super) fn template(&mut self, p: usize, plain: bool) -> usize {
        if !plain && p == self.re_arg {
            return self.regex_arg(p, b'`');
        }
        if plain {
            self.tok(p, p + 1, TPL_MARK_BEGIN);
            self.push(T_TPL_PLAIN);
        } else {
            self.tok(p, p + 1, TPL_BEGIN);
            self.push(T_TPL);
        }
        self.at = p + 1;
        p + 1
    }

    /// A template's text from `p`: to its end, a substitution, or the line's end.
    fn template_text(&mut self, mut p: usize, plain: bool) -> usize {
        let b = self.b;
        let n = b.len();
        let text = if plain { PLAIN } else { TPL_TEXT };
        loop {
            let mut i = p;
            while i < n && !TPL_STOPS[usize::from(b[i])] {
                i += 1;
            }
            if i >= n {
                self.fill_as(n, text);
                return n;
            }
            match b[i] {
                b'`' => {
                    self.fill_as(i, text);
                    self.tok(i, i + 1, if plain { TPL_MARK_END } else { TPL_END });
                    self.at = i + 1;
                    self.pop();
                    self.prev = P_VALUE;
                    return i + 1;
                }
                b'$' if b.get(i + 1) == Some(&b'{') => {
                    self.fill_as(i, text);
                    self.tok(i, i + 2, SUBST_BEGIN);
                    self.at = i + 2;
                    self.push(if plain { F_SUBST_PLAIN } else { F_SUBST });
                    self.prev = P_OP;
                    return i + 2;
                }
                b'\\' => {
                    let s = spaces_to(b, i + 1);
                    let e = if s >= n {
                        Some((n, ESC_NL))
                    } else if let Some(e) = escape(b, i) {
                        Some((e, ESC))
                    } else if b.get(i + 1) == Some(&b'`') {
                        Some((i + 2, ESC))
                    } else {
                        None
                    };
                    match e {
                        Some((e, leaf)) => {
                            self.fill_as(i, text);
                            self.tok(i, e, leaf);
                            self.at = e;
                            p = e;
                        }
                        None => p = i + 1,
                    }
                }
                _ => p = i + 1,
            }
        }
    }

    // --- comments ---

    /// A `//` at `p`: the rest of the line.
    #[inline(always)]
    pub(super) fn line_comment(&mut self, p: usize) -> usize {
        let n = self.b.len();
        self.tok(p, p + 2, LINE_MARK);
        self.tok(p + 2, n, LINE);
        n
    }

    /// A `/*` at `p`: whole where it closes on the line, else its frame.
    #[inline(never)]
    pub(super) fn block_comment(&mut self, p: usize) -> usize {
        let b = self.b;
        let doc = b.get(p + 2) == Some(&b'*') && b.get(p + 3) != Some(&b'/');
        let (open, mark) = if doc { (3, DOC_MARK) } else { (2, BLOCK_BEGIN) };
        self.tok(p, p + open, mark);
        self.push(if doc { T_DOC } else { T_BLOCK });
        self.at = p + open;
        self.comment_text(p + open, doc)
    }

    /// A block comment's text from `p`: to its `*/` or the line's end.
    fn comment_text(&mut self, p: usize, doc: bool) -> usize {
        let b = self.b;
        let n = b.len();
        let text = if doc { DOC_TEXT } else { BLOCK_TEXT };
        let mut i = p;
        loop {
            i = crate::prim::find_in(b, i, n, *b"*");
            if i >= n {
                self.fill_as(n, text);
                return n;
            }
            if b.get(i + 1) == Some(&b'/') {
                self.fill_as(i, text);
                self.tok(i, i + 2, if doc { DOC_MARK } else { BLOCK_END });
                self.at = i + 2;
                self.pop();
                return i + 2;
            }
            i += 1;
        }
    }

    // --- regex literals ---

    /// A regex literal's `/` at `p`: its body on [`crate::regex`], its end and flags; it ends
    /// with its line where it doesn't close on it.
    #[inline(never)]
    pub(super) fn regex_literal(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        self.tok(p, p + 1, RE_BEGIN);
        self.at = p + 1;
        let mut q = p + 1;
        let mut st = 0;
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
            return n;
        }
        self.fill_as(q, RE);
        self.tok(q, q + 1, RE_END);
        let mut f = q + 1;
        while f < n && matches!(b[f], b'd' | b'g' | b'i' | b'm' | b's' | b'u' | b'v' | b'y') {
            f += 1;
        }
        self.tok(q + 1, f, RE_FLAGS);
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
            self.tok(p, p + 1, TAG_BEGIN);
            self.tok(p + 1, p + 2, TAG_END);
            self.push(T_JSX_CHILD);
            return p + 2;
        }
        let e = tag_name_end(b, p + 1);
        self.tok(p, p + 1, TAG_BEGIN);
        self.tok(p + 1, e, TAG_NAME);
        self.push(T_JSX_TAG);
        e
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
                self.tok(p, p + 2, TAG_END);
                self.pop();
                self.prev = P_VALUE;
                p + 2
            }
            b'/' if matches!(b.get(p + 1), Some(b'/')) => self.line_comment(p),
            b'/' if matches!(b.get(p + 1), Some(b'*')) => self.block_comment(p),
            b'>' => {
                self.tok(p, p + 1, TAG_END);
                self.set_top(T_JSX_CHILD);
                p + 1
            }
            b'=' => self.tok(p, p + 1, ATTR_EQ),
            q @ (b'"' | b'\'') => {
                self.tok(p, p + 1, JSTR_BEGIN);
                self.push(if q == b'"' { T_JSX_STR_D } else { T_JSX_STR_S });
                self.at = p + 1;
                self.jsx_string_text(p + 1, q)
            }
            b'{' => {
                self.tok(p, p + 1, EMBED_BEGIN);
                self.push(F_J_EXPR);
                self.prev = P_OP;
                p + 1
            }
            c if c.is_ascii_alphabetic() || c == b'_' || c == b'$' => {
                let mut e = p + 1;
                while e < n && (NAME[usize::from(b[e])] || b[e] == b'-' || b[e] == b':') {
                    e += 1;
                }
                self.tok(p, e, ATTR)
            }
            c => p + char_len(c),
        }
    }

    /// A JSX attribute string's text from `p`, its quote `q`.
    fn jsx_string_text(&mut self, mut p: usize, q: u8) -> usize {
        let b = self.b;
        let n = b.len();
        loop {
            let i = crate::prim::find_in(b, p, n, [q, b'&']);
            if i >= n {
                self.fill_as(n, JSTR);
                return n;
            }
            self.fill_as(i, JSTR);
            if b[i] == q {
                self.tok(i, i + 1, JSTR_END);
                self.at = i + 1;
                self.pop();
                return i + 1;
            }
            let e = self.entity(i, JSTR_ENTITY, JSTR_BAD);
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
                self.tok(i, i + 1, EMBED_BEGIN);
                self.push(F_J_EXPR);
                self.prev = P_OP;
                i + 1
            }
            b'&' => self.entity(i, ENTITY, BAD_AMP),
            _ => {
                if b.get(i + 1) == Some(&b'/') {
                    // a closing tag: the element ends
                    let s = i + 2;
                    let e = tag_name_end(b, s);
                    self.tok(i, s, TAG_BEGIN);
                    self.tok(s, e, TAG_NAME);
                    self.pop();
                    self.prev = P_VALUE;
                    let g = skip_blanks(b, e);
                    if b.get(g) == Some(&b'>') {
                        return self.tok(g, g + 1, TAG_END);
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

    /// `&…;` at `p`: an entity, else a bad `&` up to the last `;` before a space; else the `&`
    /// alone, plain.
    fn entity(&mut self, p: usize, ok: u16, bad: u16) -> usize {
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
            return self.tok(p, e + 1, ok);
        }
        let mut s = p + 1;
        while s < n && !matches!(b[s], b' ' | b'\t') {
            s += 1;
        }
        match b[p + 1..s].iter().rposition(|&c| c == b';') {
            Some(k) => self.tok(p, p + 1 + k + 1, bad),
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

/// Where `\s` from `p` ends, blanks and the rarer spaces of ASCII.
fn spaces_to(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && matches!(b[p], b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r') {
        p += 1;
    }
    p
}
