//! Scala 2's XML literals: where Scala's scanner opens one (a `<` before a name at an operand's
//! place, after a blank, `(` or `{`; bat's opens one at any `<` before a letter, `a<b`, and stays
//! in XML to the next tag), its tags, attributes, text, comments, CDATA and entities, and its
//! `{…}` of Scala. An element's content, a start tag and a `{…}` are frames ([`F_XML`],
//! [`F_XTAG`], [`F_XEXPR`]), so a literal goes on across lines; it ends where its outermost
//! element does.

use super::*;
use crate::prim::find_in;

/// An XML name's bytes in ASCII, bat's `[\p{L}\p{N}_.-]` and `:`; a byte past ASCII is taken
/// too.
fn xml_name_end(b: &[u8], mut p: usize) -> usize {
    while p < b.len()
        && (b[p].is_ascii_alphanumeric()
            || matches!(b[p], b'_' | b'.' | b'-' | b':')
            || b[p] >= 0x80)
    {
        p += 1;
    }
    p
}

/// Whether an XML name can start at `p`: a letter or `_`.
fn xml_name_start(b: &[u8], p: usize) -> bool {
    match b.get(p) {
        Some(&c) if c < 0x80 => c.is_ascii_alphabetic() || c == b'_',
        Some(_) => is_word_starting(b, p),
        None => false,
    }
}

/// Where an entity at `p` (`&`) ends, bat's `&(?:name|#[0-9]+|#x\h+);`; `p` for none.
fn entity_end(b: &[u8], p: usize) -> usize {
    let s = p + 1;
    let e = match b.get(s) {
        Some(b'#') if b.get(s + 1) == Some(&b'x') => {
            let mut e = s + 2;
            while b.get(e).is_some_and(u8::is_ascii_hexdigit) {
                e += 1;
            }
            if e == s + 2 { return p } else { e }
        }
        Some(b'#') => {
            let mut e = s + 1;
            while b.get(e).is_some_and(u8::is_ascii_digit) {
                e += 1;
            }
            if e == s + 1 { return p } else { e }
        }
        _ if xml_name_start(b, s) || b.get(s) == Some(&b':') => xml_name_end(b, s),
        _ => return p,
    };
    if b.get(e) == Some(&b';') { e + 1 } else { p }
}

impl<K: Sink> Scan<'_, '_, K> {
    /// Whether the `<` at `p` opens an XML literal as Scala's scanner reads one: at an operand's
    /// place, after the line's start, a blank, `(` or `{`, before a name's start.
    #[inline(always)]
    pub(super) fn xml_starts(&self, p: usize) -> bool {
        let b = self.b;
        matches!(self.prev, P_START | P_OP)
            && (p == 0 || matches!(b[p - 1], b' ' | b'\t' | b'(' | b'{'))
            && xml_name_start(b, p + 1)
            && !self.typed()
    }

    /// An XML literal's first start tag at `p` (`<`).
    #[cold]
    #[inline(never)]
    pub(super) fn xml_open(&mut self, p: usize) -> usize {
        self.push(F_XTAG);
        let b = self.b;
        let e = xml_name_end(b, p + 1);
        self.put(p, p + 1, XML_TAG_BEGIN);
        self.put(p + 1, e, XML_TAG_NAME);
        self.xml_rest(e)
    }

    /// The `}` at `p` that closes an XML literal's `{…}`: the XML goes on.
    #[cold]
    #[inline(never)]
    pub(super) fn xml_expr_close(&mut self, p: usize) -> usize {
        self.pop();
        self.annot_closed(p);
        self.put(p, p + 1, XML_INLINE_END);
        self.xml_rest(p + 1)
    }

    /// An XML literal from `p` by the frame it's in (a start tag, an element's content), to the
    /// line's end, a `{` (code, [`F_XEXPR`]) or the literal's end: where the code goes on.
    #[cold]
    #[inline(never)]
    pub(super) fn xml_rest(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        loop {
            match self.top() {
                F_XTAG => {
                    // a start tag's attributes, to its `>` or `/>`
                    while p < n && matches!(b[p], b' ' | b'\t' | b'\r') {
                        p += 1;
                    }
                    if p >= n {
                        return n;
                    }
                    match b[p] {
                        b'/' if b.get(p + 1) == Some(&b'>') => {
                            self.put(p, p + 2, XML_TAG_END);
                            self.pop();
                            p += 2;
                        }
                        b'?' if b.get(p + 1) == Some(&b'>') => {
                            self.put(p, p + 2, XML_TAG_END);
                            self.pop();
                            p += 2;
                        }
                        b'>' => {
                            self.put(p, p + 1, XML_TAG_END);
                            self.set_top(F_XML);
                            p += 1;
                        }
                        b'{' => {
                            self.put(p, p + 1, XML_INLINE_BEGIN);
                            self.push(F_XEXPR);
                            self.prev = P_START;
                            return p + 1;
                        }
                        b'"' | b'\'' => {
                            let q = b[p];
                            let c = find_in(b, p + 1, n, [q]);
                            self.put(p, p + 1, XML_STR_BEGIN);
                            self.put(p + 1, c, XML_STR);
                            if c < n {
                                self.put(c, c + 1, XML_STR_END);
                                p = c + 1;
                            } else {
                                return n;
                            }
                        }
                        b'}' | b')' | b']' => {
                            // bat's tag ends before a closer of the code around it
                            self.pop();
                        }
                        _ if xml_name_start(b, p) => {
                            let e = xml_name_end(b, p);
                            let q = crate::prim::skip_spaces(b, e);
                            if b.get(q) == Some(&b'=') {
                                // `ns:name`: bat's namespace, its `:`, the local name
                                if let Some(c) = b[p..e].iter().position(|&c| c == b':') {
                                    self.put(p, p + c, XML_NS);
                                    self.put(p + c, p + c + 1, XML_NS_SEP);
                                    self.put(p + c + 1, e, XML_ATTR);
                                } else {
                                    self.put(p, e, XML_ATTR);
                                }
                                self.put(q, q + 1, XML_EQ);
                                p = q + 1;
                            } else {
                                p = e;
                            }
                        }
                        c => p += char_len(c),
                    }
                }
                F_XML => {
                    // an element's content: text to the next `<`, `{` or `&`
                    let q = find_in(b, p, n, *b"<{&");
                    self.put(p, q, XML_TEXT);
                    if q >= n {
                        return n;
                    }
                    p = q;
                    match b[p] {
                        b'{' => {
                            if b.get(p + 1) == Some(&b'{') {
                                self.put(p, p + 2, XML_TEXT);
                                p += 2;
                                continue;
                            }
                            self.put(p, p + 1, XML_INLINE_BEGIN);
                            self.push(F_XEXPR);
                            self.prev = P_START;
                            return p + 1;
                        }
                        b'&' => {
                            let e = entity_end(b, p);
                            let e = if e > p { e } else { p + 1 };
                            self.put(p, e, if e > p + 1 { XML_ENTITY } else { XML_TEXT });
                            p = e;
                        }
                        _ => p = self.xml_lt(p),
                    }
                }
                _ => {
                    // the literal's end
                    self.prev = P_VALUE;
                    return p;
                }
            }
        }
    }

    /// A `<` at `p` in an element's content: its end tag (the element closes), a child's start
    /// tag, a comment, CDATA; where the XML goes on.
    fn xml_lt(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        if b.get(p + 1) == Some(&b'/') {
            let e = xml_name_end(b, p + 2);
            self.put(p, p + 2, XML_TAG_BEGIN);
            self.put(p + 2, e, XML_TAG_NAME);
            let q = crate::prim::skip_spaces(b, e);
            if b.get(q) == Some(&b'>') {
                self.put(q, q + 1, XML_TAG_END);
                self.pop();
                return q + 1;
            }
            self.pop();
            return e;
        }
        if b[p..].starts_with(b"<!--") {
            self.put(p, p + 4, XML_COMMENT_BEGIN);
            let c = b[p + 4..]
                .windows(3)
                .position(|w| w == b"-->")
                .map(|i| p + 4 + i);
            return match c {
                Some(c) => {
                    self.put(p + 4, c, XML_COMMENT);
                    self.put(c, c + 3, XML_COMMENT_END);
                    c + 3
                }
                None => {
                    self.put(p + 4, n, XML_COMMENT);
                    n
                }
            };
        }
        if b[p..].starts_with(b"<![CDATA[") {
            let c = b[p + 9..]
                .windows(3)
                .position(|w| w == b"]]>")
                .map_or(n, |i| p + 12 + i);
            self.put(p, c, XML_CDATA);
            return c;
        }
        if xml_name_start(b, p + 1) {
            let e = xml_name_end(b, p + 1);
            self.put(p, p + 1, XML_TAG_BEGIN);
            self.put(p + 1, e, XML_TAG_NAME);
            self.push(F_XTAG);
            return e;
        }
        self.put(p, p + 1, XML_TEXT);
        p + 1
    }
}
