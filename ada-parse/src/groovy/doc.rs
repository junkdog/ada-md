//! Groovydoc: a `/** … */` comment's text as bat's Javadoc reads it, on a reader of ada's own: a
//! line's leading `*`, block tags at the text's first token (`@param name`, `@throws Type`,
//! `@return`…), inline tags (`{@code …}` raw to its balanced `}`, `{@link Type#member label}`,
//! `{@value #X}`, `{@inheritDoc}`), HTML's tags with their attributes, and entities. What spans
//! lines is the comment alone ([`M_DOC`]): an inline tag or an HTML tag ends with its line.

use super::*;
use crate::prim::find_in;

/// bat's block tags, by their leaf after `@`: `param` takes a name, `see`, `throws`,
/// `exception`, `uses`, `provides` a reference.
fn block_tag(w: &[u8]) -> Option<u8> {
    Some(match w {
        b"param" => 1,
        b"see" | b"throws" | b"exception" | b"uses" | b"provides" => 2,
        b"return" | b"deprecated" | b"author" | b"version" | b"since" | b"apiNote"
        | b"implNote" | b"implSpec" | b"moduleGraph" | b"serial" | b"serialField"
        | b"serialData" => 0,
        _ => return None,
    })
}

/// Where a word of ASCII letters from `p` ends.
fn letters_end(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && b[p].is_ascii_alphabetic() {
        p += 1;
    }
    p
}

/// Where a reference from `p` ends (`Type`, `pkg.Type#member`, `#member(int, String)`): its
/// name, and a parenthesized signature right after it.
fn reference_end(b: &[u8], p: usize) -> usize {
    let mut e = p;
    while e < b.len() && !matches!(b[e], b' ' | b'\t' | b'(' | b'}' | b'{') {
        e += 1;
    }
    if e > p && b.get(e) == Some(&b'(') {
        let c = find_in(b, e, b.len(), *b")");
        return if c < b.len() { c + 1 } else { b.len() };
    }
    e
}

impl<K: Sink> Scan<'_, '_, K> {
    /// A documentation comment's text from `p` (`first`: right after its `/**`, where a block tag
    /// may start the text), to its `*/` or the line's end.
    pub(super) fn doc(&mut self, mut p: usize, first: bool) -> usize {
        let b = self.b;
        let n = b.len();
        let mut lead = first;
        if !first {
            // bat's prototype `^\s*(\*)(?!/)`: the line's leading star
            let q = skip_spaces(b, 0);
            if b.get(q) == Some(&b'*') && b.get(q + 1) != Some(&b'/') {
                if q > 0 {
                    self.put(0, q, DOC);
                }
                self.put(q, q + 1, DOC_STAR);
                p = q + 1;
            }
            lead = true;
        }
        let mut at = p;
        loop {
            let q = find_in(b, p, n, *b"*@{<&");
            if q >= n {
                if n > at {
                    self.put(at, n, DOC);
                }
                return n;
            }
            match b[q] {
                b'*' => {
                    if b.get(q + 1) == Some(&b'/') {
                        if q > at {
                            self.put(at, q, DOC);
                        }
                        self.put(q, q + 2, DOC_END);
                        self.mode = M_CODE;
                        return q + 2;
                    }
                    p = q + 1;
                }
                b'@' => {
                    let e = letters_end(b, q + 1);
                    let tag = if lead && skip_spaces(b, at) == q {
                        block_tag(&b[q + 1..e])
                    } else {
                        None
                    };
                    let Some(t) = tag else {
                        p = q + 1;
                        continue;
                    };
                    if q > at {
                        self.put(at, q, DOC);
                    }
                    self.put(q, q + 1, DOC_TAG_MARK);
                    self.put(q + 1, e, DOC_TAG);
                    p = e;
                    let s = skip_spaces(b, e);
                    // `@see <a href…>` and `@see "text"`: no reference, HTML or text
                    if t > 0 && !matches!(b.get(s), None | Some(b'<' | b'"')) {
                        let f = if t == 1 {
                            find_in(b, s, n, *b" \t")
                        } else {
                            reference_end(b, s)
                        };
                        if f > s {
                            if s > e {
                                self.put(e, s, DOC);
                            }
                            self.put(s, f, if t == 1 { DOC_PARAM } else { DOC_LINK });
                            p = f;
                        }
                    }
                    at = p;
                }
                b'{' => match self.inline_tag(q) {
                    Some(e) => {
                        if q > at {
                            self.put(at, q, DOC);
                        }
                        self.inline_tag_write(q, e);
                        p = e;
                        at = e;
                    }
                    None => p = q + 1,
                },
                b'<' => match html_tag_end(b, q) {
                    Some(e) => {
                        if q > at {
                            self.put(at, q, DOC);
                        }
                        self.html_tag(q, e);
                        p = e;
                        at = e;
                    }
                    None => p = q + 1,
                },
                _ => {
                    // `&name;`
                    let e = {
                        let mut i = q + 1;
                        if b.get(i) == Some(&b'#') {
                            i += 1;
                        }
                        while i < n && b[i].is_ascii_alphanumeric() {
                            i += 1;
                        }
                        i
                    };
                    if e > q + 1 && b.get(e) == Some(&b';') {
                        if q > at {
                            self.put(at, q, DOC);
                        }
                        self.put(q, e + 1, DOC_ENTITY);
                        p = e + 1;
                        at = p;
                    } else {
                        p = q + 1;
                    }
                }
            }
            if p > q && b.get(q) != Some(&b'*') {
                lead = false;
            }
        }
    }

    /// Where an inline tag at `q` (`{@`) ends: past its `}`, or the line's end; `None` for a
    /// `{` that opens none of bat's.
    fn inline_tag(&self, q: usize) -> Option<usize> {
        let b = self.b;
        if b.get(q + 1) != Some(&b'@') {
            return None;
        }
        let e = letters_end(b, q + 2);
        let ok = matches!(
            &b[q + 2..e],
            b"code"
                | b"literal"
                | b"link"
                | b"linkplain"
                | b"value"
                | b"index"
                | b"extLink"
                | b"inheritDoc"
        );
        if !ok {
            return None;
        }
        // to the `}` that closes it, `{…}` balanced
        let mut depth = 0u32;
        let mut i = e;
        while i < b.len() {
            match b[i] {
                b'{' => depth += 1,
                b'}' => {
                    if depth == 0 {
                        return Some(i + 1);
                    }
                    depth -= 1;
                }
                _ => {}
            }
            i += 1;
        }
        Some(b.len())
    }

    /// The inline tag `b[q..e]`: its `{`, `@` and name, then its content (raw, a reference and
    /// a label, or plain), its `}`.
    fn inline_tag_write(&mut self, q: usize, e: usize) {
        let b = self.b;
        let ne = letters_end(b, q + 2);
        let closed = e > q && b.get(e - 1) == Some(&b'}') && e > ne;
        let end = if closed { e - 1 } else { e };
        self.put(q, q + 1, DOC_INLINE_BEGIN);
        self.put(q + 1, q + 2, DOC_TAG_MARK);
        self.put(q + 2, ne, DOC_TAG);
        let s = skip_spaces(b, ne);
        if s < end {
            match &b[q + 2..ne] {
                b"code" | b"literal" => {
                    self.put(ne, s, DOC);
                    self.put(s, end, DOC_RAW);
                }
                b"link" | b"linkplain" | b"value" => {
                    let r = reference_end(b, s).min(end);
                    self.put(ne, s, DOC);
                    if r > s {
                        self.put(s, r, DOC_LINK);
                    }
                    if end > r {
                        self.put(r, end, DOC);
                    }
                }
                _ => self.put(ne, end, DOC),
            }
        } else if end > ne {
            self.put(ne, end, DOC);
        }
        if closed {
            self.put(e - 1, e, DOC_INLINE_END);
        }
    }

    /// The HTML tag `b[q..e]` (`<name attr="v">`, `</name>`): its marks, name, attributes'
    /// names and values.
    fn html_tag(&mut self, q: usize, e: usize) {
        let b = self.b;
        let s = if b.get(q + 1) == Some(&b'/') {
            q + 2
        } else {
            q + 1
        };
        self.put(q, s, DOC_HTML_BEGIN);
        let mut i = s;
        while i < e && !matches!(b[i], b' ' | b'\t' | b'>' | b'/') {
            i += 1;
        }
        self.put(s, i, DOC_HTML_NAME);
        // a self-closing tag's `/` is the text's (bat's)
        let close = if b.get(e - 1) == Some(&b'>') {
            e - 1
        } else {
            e
        };
        while i < close {
            let c = b[i];
            if c.is_ascii_alphabetic() {
                let mut j = i;
                while j < close
                    && (b[j].is_ascii_alphanumeric() || matches!(b[j], b'-' | b':' | b'_'))
                {
                    j += 1;
                }
                self.put(i, j, DOC_ATTR);
                i = j;
            } else if c == b'"' || c == b'\'' {
                let mut j = i + 1;
                while j < close && b[j] != c {
                    j += 1;
                }
                let j = (j + 1).min(close);
                self.put(i, j, DOC_ATTR_VALUE);
                i = j;
            } else {
                if c == b'=' {
                    self.put(i, i + 1, DOC_ATTR_SEP);
                } else {
                    self.put(i, i + 1, DOC);
                }
                i += 1;
            }
        }
        if close < e {
            self.put(close, e, DOC_HTML_END);
        }
    }
}

/// Where an HTML tag at `q` (`<`) ends, past its `>`: a name after `<` or `</` (bat's
/// `[^\s>/]` run: `<paolo@gmail.com>` one too), then anything but `<` to `>` on the line;
/// `None` for a `<` that opens none.
fn html_tag_end(b: &[u8], q: usize) -> Option<usize> {
    let s = if b.get(q + 1) == Some(&b'/') {
        q + 2
    } else {
        q + 1
    };
    if !b.get(s)?.is_ascii_alphabetic() {
        return None;
    }
    let mut i = s;
    while i < b.len() {
        match b[i] {
            b'>' => return Some(i + 1),
            b'<' => return None,
            _ => i += 1,
        }
    }
    None
}
