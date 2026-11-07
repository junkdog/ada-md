//! Javadoc, and the part of HTML it includes: bat's contexts, as Java's scanner tries them (moved from
//! `java.rs` unchanged by X-015).

use super::*;

impl<'b, K: Sink> Scan<'b, '_, K> {
    /// Whether the line is `^\s*\*?\s*@`.
    pub(super) fn is_tag_line(&mut self) -> bool {
        let b = self.b;
        *self.tag_line.get_or_insert_with(|| {
            let mut q = skip_spaces(b, 0);
            if b.get(q) == Some(&b'*') {
                q = skip_spaces(b, q + 1);
            }
            b.get(q) == Some(&b'@')
        })
    }

    /// The `with_prototype`s of the levels below: `(?=\*/)` above `contents`, and
    /// `(?=^\s*\*?\s*@)` from the block tags up.
    pub(super) fn jd_withs(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        // a pop that would loop is passed over for the patterns after it
        if self.loop_at == (p, self.depth) {
            return None;
        }
        if b[p] == b'*' && b.get(p + 1) == Some(&b'/') {
            return Some(self.leave(p));
        }
        if p == 0 && self.tags > 0 && self.is_tag_line() {
            return Some(self.leave(p));
        }
        None
    }

    /// Javadoc's prototype at the line's start: `^\s*(\*)(?!/)\s*(?!\s*@)`.
    pub(super) fn jd_star(&mut self) -> Option<usize> {
        let b = self.b;
        let q = skip_spaces(b, 0);
        if b.get(q) != Some(&b'*') || b.get(q + 1) == Some(&b'/') {
            return None;
        }
        let e = skip_spaces(b, q + 1);
        if b.get(e) == Some(&b'@') {
            return None;
        }
        self.put(q, q + 1, W_JD | JD_STAR);
        self.fill(e);
        Some(e)
    }

    /// The Javadoc and HTML contexts at `p`.
    pub(super) fn javadoc(&mut self, p: usize, c: u8, top: u8) -> Option<usize> {
        let b = self.b;
        if top == JD_MAIN {
            return (c == b'*' && b.get(p + 1) == Some(&b'/')).then(|| self.close(p, 2, JD_END));
        }
        if let Some(r) = self.jd_withs(p) {
            return Some(r);
        }
        // the prototype, but in `contents`, below the tags and in HTML's contexts
        if p == 0
            && !matches!(top, JD_CONTENTS | JD_HOLD)
            && top < H_TAG
            && let Some(e) = self.jd_star()
        {
            return Some(e);
        }
        match top {
            JD_CONTENTS => {
                // `\s*(?=@)`, then `` pushing the text
                let q = skip_spaces(b, p);
                if b.get(q) == Some(&b'@') {
                    return Some(self.tags_at(p, q));
                }
                Some(self.enter(p, &[JD_TEXT]))
            }
            JD_TEXT => {
                if p == 0 && self.is_tag_line() {
                    // `^\s*(\*)?\s*(?=@)`
                    let mut q = skip_spaces(b, 0);
                    if b[q] == b'*' {
                        self.put(q, q + 1, W_JD | JD_STAR);
                        q = skip_spaces(b, q + 1);
                    }
                    return Some(self.tags_at(0, q));
                }
                self.inline_formatting(p, c)
            }
            JD_HOLD => (p == 0 && self.is_tag_line()).then(|| self.leave(p)),
            JD_TAGS => {
                if c != b'@' {
                    return None;
                }
                let tags: [(&[u8], u16, u16, &[u8]); 16] = [
                    (b"param", TAG_PARAM, TAG_PARAM_AT, &[JD_TAG, JD_PARAM_NAME]),
                    (b"see", TAG_SEE, TAG_SEE_AT, &[JD_TAG, JD_SEE]),
                    (b"throws", TAG_THROWS, TAG_THROWS_AT, &[JD_TAG, JD_REF]),
                    (b"exception", TAG_THROWS, TAG_THROWS_AT, &[JD_TAG, JD_REF]),
                    (b"uses", TAG_USES, TAG_USES_AT, &[JD_TAG, JD_REF]),
                    (b"provides", TAG_USES, TAG_USES_AT, &[JD_TAG, JD_REF]),
                    (b"return", TAG_DOC, TAG_DOC_AT, &[JD_TAG]),
                    (b"deprecated", TAG_DOC, TAG_DOC_AT, &[JD_TAG]),
                    (b"author", TAG_DOC, TAG_DOC_AT, &[JD_TAG]),
                    (b"version", TAG_DOC, TAG_DOC_AT, &[JD_TAG]),
                    (b"since", TAG_DOC, TAG_DOC_AT, &[JD_TAG]),
                    (b"apiNote", TAG_DOC, TAG_DOC_AT, &[JD_TAG]),
                    (b"implNote", TAG_DOC, TAG_DOC_AT, &[JD_TAG]),
                    (b"implSpec", TAG_DOC, TAG_DOC_AT, &[JD_TAG]),
                    (b"moduleGraph", TAG_DOC, TAG_DOC_AT, &[JD_TAG]),
                    (b"serial", TAG_DOC, TAG_DOC_AT, &[JD_TAG]),
                ];
                let w = &b[p + 1..word_end(b, p + 1)];
                let w = match w {
                    b"serialField" | b"serialData" => &b"serial"[..],
                    _ => w,
                };
                if let Some(&(_, leaf, at, cs)) = tags.iter().find(|t| t.0 == w) {
                    let e = word_end(b, p + 1);
                    for &cx in cs {
                        self.push(p, cx);
                    }
                    self.put(p, p + 1, W_JD | at);
                    self.put(p + 1, e, W_JD | leaf);
                    return Some(e);
                }
                Some(self.close(p, 1, W_JD))
            }
            JD_TAG => self.inline_formatting(p, c),
            JD_PARAM_NAME => (!is_space(c)).then(|| {
                let e = next_of(b, p, is_space);
                self.close(p, e - p, W_JD | JD_PARAM)
            }),
            JD_SEE => match c {
                b'\'' | b'<' => Some(self.leave(p)),
                _ if !is_space(c) => Some(self.shift(p, &[JD_REF])),
                _ => None,
            },
            JD_REF | JD_IREF => {
                if top == JD_IREF && c == b'}' {
                    return Some(self.leave(p));
                }
                let e = self.reference(p)?;
                let to = if top == JD_REF {
                    JD_REF_AFTER
                } else {
                    JD_IREF_AFTER
                };
                Some(self.then(p, e - p, &[to], W_JD | LINK))
            }
            JD_REF_AFTER | JD_IREF_AFTER => {
                if top == JD_IREF_AFTER && c == b'}' {
                    return Some(self.leave(p));
                }
                if c == b'(' {
                    let to = if top == JD_REF_AFTER {
                        JD_REF_PARENS
                    } else {
                        JD_IREF_PARENS
                    };
                    return Some(self.then(p, 1, &[to], W_JD | LINK));
                }
                Some(self.leave(p))
            }
            JD_REF_PARENS | JD_IREF_PARENS => {
                if top == JD_IREF_PARENS && c == b'}' {
                    return Some(self.leave(p));
                }
                if c == b')' {
                    return Some(self.close(p, 1, W_JD | LINK));
                }
                // `.`, to where another pattern could start
                let e = next_of(b, p + 1, |c| matches!(c, b')' | b'}' | b'*'));
                Some(self.token(p, e - p, W_JD | LINK))
            }
            JD_RAW | JD_BRACES => match c {
                b'}' => Some(self.close(p, 1, W_JD | if top == JD_RAW { INLINE_END } else { RAW })),
                b'{' => Some(self.open(p, 1, JD_BRACES, W_JD | RAW)),
                _ if p == 0 && top == JD_RAW && is_space(c) => {
                    // `^\s+`
                    let e = skip_spaces(b, 0);
                    self.fill(e);
                    Some(e)
                }
                _ => {
                    // `.`, to where another pattern could start
                    let e = next_of(b, p + 1, |c| matches!(c, b'{' | b'}' | b'*'));
                    Some(self.token(p, e - p, W_JD | RAW))
                }
            },
            JD_INLINE => (c == b'}').then(|| self.close(p, 1, W_JD | INLINE_END)),
            JD_LINK_LABEL => {
                let e = skip_spaces(b, p);
                self.fill(e);
                Some(self.shift(e, &[JD_LABEL]))
            }
            JD_LABEL => {
                if let Some(r) = self.html(p, c) {
                    return Some(r);
                }
                (c == b'}').then(|| self.leave(p))
            }
            _ => self.html_context(p, c, top),
        }
    }

    /// The block tags, pushed by a match from `p` to `q` (`contents_0` or `_3` under them).
    pub(super) fn tags_at(&mut self, p: usize, q: usize) -> usize {
        if q == p {
            return self.enter(p, &[JD_HOLD, JD_TAGS]);
        }
        self.push(q, JD_HOLD);
        self.push(q, JD_TAGS);
        q
    }

    /// `reference`'s `(?:(?:ident)\.)*(?:ident)(?:#(?:ident))?|#(?:ident)` at `p`.
    pub(super) fn reference(&self, p: usize) -> Option<usize> {
        let b = self.b;
        let hash = |q: usize| {
            if b.get(q) == Some(&b'#') {
                ident(b, q + 1)
            } else {
                None
            }
        };
        let Some(mut e) = ident(b, p) else {
            return hash(p);
        };
        while b.get(e) == Some(&b'.')
            && let Some(f) = ident(b, e + 1)
        {
            e = f;
        }
        Some(hash(e).unwrap_or(e))
    }

    /// `inline-formatting`: the inline tags, then HTML.
    pub(super) fn inline_formatting(&mut self, p: usize, c: u8) -> Option<usize> {
        if c == b'{'
            && let Some(r) = self.inline_tag(p)
        {
            return Some(r);
        }
        self.html(p, c)
    }

    /// `javadoc-inline-tags` at `p`, a `{`.
    pub(super) fn inline_tag(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if b.get(p + 1) != Some(&b'@') {
            return None;
        }
        let s = p + 2;
        let w = &b[s..word_end(b, s)];
        let e = s + w.len();
        // `({)((@)(?:code|literal))(?:\s+|(?=\}))`
        for kw in [&b"code"[..], b"literal"] {
            let k = s + kw.len();
            if b[s..].starts_with(kw) && (k == b.len() || is_space(b[k]) || b[k] == b'}') {
                let q = skip_spaces(b, k);
                return Some(self.inline_open(p, k, TAG_CODE, TAG_CODE_AT, &[JD_RAW], q));
            }
        }
        match w {
            b"link" | b"linkplain" => {
                let cs = [JD_INLINE, JD_LINK_LABEL, JD_IREF];
                Some(self.inline_open(p, e, TAG_LINK, TAG_LINK_AT, &cs, e))
            }
            b"value" => {
                Some(self.inline_open(p, e, TAG_VALUE, TAG_VALUE_AT, &[JD_INLINE, JD_IREF], e))
            }
            b"index" | b"extLink" => {
                Some(self.inline_open(p, e, TAG_DOC, TAG_DOC_AT, &[JD_INLINE], e))
            }
            b"inheritDoc" if b.get(e) == Some(&b'}') => {
                self.put(p, p + 1, W_JD | INLINE_BEGIN);
                self.put(p + 1, s, W_JD | TAG_DOC_AT);
                self.put(s, e, W_JD | TAG_DOC);
                Some(self.token(e, 1, W_JD | INLINE_END))
            }
            _ => None,
        }
    }

    /// An inline tag at `p`, its name ending at `e`, pushing `cs`; the match ends at `to`.
    pub(super) fn inline_open(
        &mut self,
        p: usize,
        e: usize,
        leaf: u16,
        at: u16,
        cs: &[u8],
        to: usize,
    ) -> usize {
        for &c in cs {
            self.push(p, c);
        }
        self.put(p, p + 1, W_JD | INLINE_BEGIN);
        self.put(p + 1, p + 2, W_JD | at);
        self.put(p + 2, e, W_JD | leaf);
        self.fill(to);
        to
    }

    /// HTML's `main` at `p`, in Javadoc: tags, comments, entities.
    pub(super) fn html(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if c == b'&' {
            return self.entity(p, W_JD);
        }
        if c != b'<' {
            return None;
        }
        let rest = &b[p + 1..];
        if rest.starts_with(b"!--") {
            // `(<!--)(-?>)?`
            self.push(p, H_COMMENT);
            self.put(p, p + 4, W_HCOMMENT | HCOMMENT_BEGIN);
            let mut e = p + 4;
            if b[e..].starts_with(b"->") {
                e += 2;
            } else if b[e..].starts_with(b">") {
                e += 1;
            }
            self.put(p + 4, e, W_HCOMMENT | HCOMMENT_BAD);
            return Some(e);
        }
        if rest.starts_with(b"![CDATA[") {
            self.push(p, H_CDATA);
            self.put(p, p + 3, W_JD | TAG_OPEN);
            self.put(p + 3, p + 8, W_JD | CDATA_KW);
            return Some(self.token(p + 8, 1, W_JD | TAG_OPEN));
        }
        if rest.starts_with(b"!DOCTYPE") {
            for cx in [DT_META, DT_CONTENT, DT_TYPE, DT_NAME] {
                self.push(p, cx);
            }
            self.put(p, p + 2, W_JD | TAG_OPEN);
            return Some(self.token(p + 2, 7, W_JD | DOCTYPE_KW));
        }
        if rest.starts_with(b"?xml") {
            self.push(p, H_PI);
            self.put(p, p + 2, W_JD | TAG_OPEN);
            return Some(self.token(p + 2, 3, W_JD | NAME_XML));
        }
        let close = rest.first() == Some(&b'/');
        let s = p + 1 + usize::from(close);
        let e = (s..b.len())
            .find(|&i| ends_tag_name(b, i))
            .unwrap_or(b.len());
        let name = &b[s..e];
        let lower = |w: &[u8]| name.eq_ignore_ascii_case(w);
        if !close && (lower(b"style") || lower(b"script")) {
            let leaf = if lower(b"style") {
                NAME_STYLE
            } else {
                NAME_SCRIPT
            };
            self.push(p, H_EMBED_TAG);
            self.put(p, s, W_JD | TAG_OPEN);
            return Some(self.token(s, e - s, W_JD | leaf));
        }
        if !name.is_empty() {
            let found = HTML_TAGS
                .iter()
                .find(|(names, _, _)| names.iter().any(|w| lower(w)));
            if let Some(&(_, leaf, cx)) = found {
                self.push(p, cx);
                self.put(p, s, W_JD | TAG_OPEN);
                return Some(self.token(s, e - s, W_JD | leaf));
            }
        }
        if b.get(s).is_some_and(u8::is_ascii_alphabetic) {
            let custom = name.contains(&b'-');
            self.push(p, H_TAG_SC);
            self.push(p, if custom { H_CUSTOM_NAME } else { H_OTHER_NAME });
            return Some(self.token(p, s - p, W_JD | TAG_OPEN));
        }
        if rest.first() == Some(&b'>') {
            return Some(self.token(p, 2, W_JD | INCOMPLETE));
        }
        None
    }

    /// `entities` at `p`, an `&`, in `wrap`.
    pub(super) fn entity(&mut self, p: usize, wrap: u16) -> Option<usize> {
        let b = self.b;
        let (mark, body, leaves) = if b[p + 1..].starts_with(b"#x") || b[p + 1..].starts_with(b"#X")
        {
            (
                3,
                next_of(b, p + 3, |c| !c.is_ascii_hexdigit()),
                [ENT_HEX, ENT_HEX_MARK, ENT_HEX_END],
            )
        } else if b.get(p + 1) == Some(&b'#') {
            (
                2,
                next_of(b, p + 2, |c| !c.is_ascii_digit()),
                [ENT_DEC, ENT_DEC_MARK, ENT_DEC_END],
            )
        } else {
            (
                1,
                next_of(b, p + 1, |c| !c.is_ascii_alphanumeric()),
                [ENT_NAMED, ENT_NAMED_MARK, ENT_NAMED_END],
            )
        };
        if body == p + mark || b.get(body) != Some(&b';') {
            return None;
        }
        self.put(p, p + mark, wrap | leaves[1]);
        self.put(p + mark, body, wrap | leaves[0]);
        Some(self.token(body, 1, wrap | leaves[2]))
    }

    /// HTML's contexts at `p`.
    pub(super) fn html_context(&mut self, p: usize, c: u8, top: u8) -> Option<usize> {
        let b = self.b;
        match top {
            H_TAG | H_TAG_SC | H_EMBED_TAG => {
                if c == b'>' {
                    if top == H_EMBED_TAG {
                        return Some(self.then(p, 1, &[H_EMBED_TEXT], W_JD | TAG_CLOSE));
                    }
                    return Some(self.close(p, 1, W_JD | TAG_CLOSE));
                }
                if c == b'/' && b.get(p + 1) == Some(&b'>') && top != H_TAG {
                    return Some(self.close(p, 2, W_JD | TAG_CLOSE));
                }
                self.attributes(p, c)
            }
            H_EMBED_TEXT => {
                let rest = &b[p..];
                let closes = |w: &[u8]| {
                    rest.len() >= 2 + w.len()
                        && rest.starts_with(b"</")
                        && rest[2..2 + w.len()].eq_ignore_ascii_case(w)
                };
                (closes(b"style") || closes(b"script")).then(|| self.leave(p))
            }
            H_PI => {
                if c == b'?' && b.get(p + 1) == Some(&b'>') {
                    return Some(self.close(p, 2, W_JD | TAG_CLOSE));
                }
                self.generic_attribute(p, c)
                    .or_else(|| self.html_string(p, c))
            }
            H_OTHER_NAME => ends_tag_name(b, p).then(|| self.leave(p)),
            H_CUSTOM_NAME => {
                if ends_tag_name(b, p) {
                    return Some(self.leave(p));
                }
                let ok = |i: usize| {
                    let c = b[i];
                    matches!(c, b'-' | b'_' | b'a'..=b'z' | b'0'..=b'9') || c >= 0x80
                };
                if c == b'\\' && p + 1 < b.len() {
                    let e = next(b, p + 1);
                    return Some(self.token(p, e - p, W_JD | NAME_CUSTOM));
                }
                if ok(p) {
                    let e = run_of(b, p, |_, i| ok(i));
                    return Some(self.token(p, e - p, W_JD | NAME_CUSTOM));
                }
                Some(self.token(p, char_len(c), W_JD | NAME_CUSTOM_BAD))
            }
            H_ATTR_NAME => {
                if is_html_space(b, p) || matches!(c, b'=' | b'/' | b'>') {
                    return Some(self.leave(p));
                }
                if matches!(c, b'"' | b'\'' | b'`' | b'<') {
                    return Some(self.token(p, 1, W_ATTR | BAD_NAME));
                }
                let e = next_of(b, p + 1, |c| {
                    matches!(
                        c,
                        b'\t'
                            | b'\n'
                            | b'\x0c'
                            | b' '
                            | b'='
                            | b'/'
                            | b'>'
                            | b'"'
                            | b'\''
                            | b'`'
                            | b'<'
                            | b'*'
                    )
                });
                self.fill(e);
                Some(e)
            }
            H_EQ | H_EMBED_EQ => match c {
                b'=' => {
                    let to = if top == H_EQ { H_VALUE } else { H_EMBED_VALUE };
                    Some(self.then(p, 1, &[to], W_JD | KEY_VALUE))
                }
                _ => self.any_pop(p),
            },
            H_VALUE => match c {
                b'"' => Some(self.then(p, 1, &[H_DQ], W_DQ | STR_BEGIN)),
                b'\'' => Some(self.then(p, 1, &[H_SQ], W_SQ | STR_BEGIN)),
                b'\t' | b'\n' | b'\x0c' | b' ' | b'=' | b'>' => self.any_pop(p),
                _ => Some(self.shift(p, &[H_UNQ])),
            },
            H_EMBED_VALUE => match c {
                b'"' => Some(self.then(p, 1, &[H_EMBED_DQ], W_JD | EMBED_DQ_BEGIN)),
                b'\'' => Some(self.then(p, 1, &[H_EMBED_SQ], W_JD | EMBED_SQ_BEGIN)),
                _ => self.any_pop(p),
            },
            H_EMBED_DQ | H_EMBED_SQ => {
                let (q, leaf) = if top == H_EMBED_DQ {
                    (b'"', EMBED_DQ_END)
                } else {
                    (b'\'', EMBED_SQ_END)
                };
                (c == q).then(|| self.close(p, 1, W_JD | leaf))
            }
            H_DQ | H_SQ => {
                let (q, wrap) = if top == H_DQ {
                    (b'"', W_DQ)
                } else {
                    (b'\'', W_SQ)
                };
                if c == q {
                    return Some(self.close(p, 1, wrap | STR_END));
                }
                if c == b'&' {
                    return self.entity(p, wrap);
                }
                None
            }
            H_UNQ => {
                if is_html_space(b, p) || c == b'>' || (c == b'/' && b.get(p + 1) == Some(&b'>')) {
                    return Some(self.leave(p));
                }
                if matches!(c, b'"' | b'\'' | b'`' | b'<') {
                    return Some(self.token(p, 1, W_UNQ | BAD_VALUE));
                }
                if c == b'&' {
                    return self.entity(p, W_UNQ);
                }
                None
            }
            H_COMMENT => {
                // `(<!-)?(--\s*>)`
                let dashes = |q: usize| {
                    b[q..].starts_with(b"--").then(|| {
                        let s = skip_spaces(b, q + 2);
                        (b.get(s) == Some(&b'>')).then_some(s + 1)
                    })?
                };
                if b[p..].starts_with(b"<!-")
                    && let Some(e) = dashes(p + 3)
                {
                    self.put(p, p + 3, W_HCOMMENT | HCOMMENT_BAD);
                    self.put(p + 3, e, W_HCOMMENT | HCOMMENT_END);
                    self.pop(e);
                    return Some(e);
                }
                if let Some(e) = dashes(p) {
                    return Some(self.close(p, e - p, W_HCOMMENT | HCOMMENT_END));
                }
                // `<!--(?!-?>)|--!>`
                if b[p..].starts_with(b"<!--")
                    && !b[p + 4..].starts_with(b">")
                    && !b[p + 4..].starts_with(b"->")
                {
                    return Some(self.token(p, 4, W_HCOMMENT | HCOMMENT_BAD));
                }
                if b[p..].starts_with(b"--!>") {
                    return Some(self.token(p, 4, W_HCOMMENT | HCOMMENT_BAD));
                }
                None
            }
            H_CDATA => b[p..]
                .starts_with(b"]]>")
                .then(|| self.close(p, 3, W_JD | TAG_CLOSE)),
            DT_NAME => {
                if c.is_ascii_alphabetic() {
                    let e = (p + 1..b.len())
                        .find(|&i| ends_tag_name(b, i))
                        .unwrap_or(b.len());
                    return Some(self.close(p, e - p, W_JD | DOCTYPE_NAME));
                }
                self.any_pop(p)
            }
            DT_TYPE => {
                for w in [&b"PUBLIC"[..], b"SYSTEM"] {
                    if b[p..].starts_with(w) && boundary(b, p + 6) {
                        return Some(self.close(p, 6, W_JD | DOCTYPE_EXTERNAL));
                    }
                }
                self.any_pop(p)
            }
            DT_CONTENT => {
                if c == b'[' {
                    return Some(self.then(p, 1, &[DT_SUBSET], W_JD | HBRACKET_BEGIN));
                }
                self.html_string(p, c).or_else(|| self.any_pop(p))
            }
            DT_META => (c == b'>').then(|| self.close(p, 1, W_JD | TAG_CLOSE)),
            DT_SUBSET => {
                if c == b']' {
                    return Some(self.close(p, 1, W_JD | HBRACKET_END));
                }
                if b[p..].starts_with(b"<!--") {
                    return self.html(p, c);
                }
                None
            }
            _ => None,
        }
    }

    /// `string-double-quoted`, `string-single-quoted` at `p`.
    pub(super) fn html_string(&mut self, p: usize, c: u8) -> Option<usize> {
        match c {
            b'"' => Some(self.open(p, 1, H_DQ, W_DQ | STR_BEGIN)),
            b'\'' => Some(self.open(p, 1, H_SQ, W_SQ | STR_BEGIN)),
            _ => None,
        }
    }

    /// `tag-attributes` at `p`.
    pub(super) fn attributes(&mut self, p: usize, c: u8) -> Option<usize> {
        let b = self.b;
        if boundary(b, p) && is_word(b, p) {
            let w = &b[p..word_end(b, p)];
            let found = match w {
                b"id" => Some((ATTR_ID, H_EQ)),
                b"class" => Some((ATTR_CLASS, H_EQ)),
                b"style" => Some((ATTR_STYLE, H_EMBED_EQ)),
                _ if w.starts_with(b"on") && EVENTS.contains(&&w[2..]) => {
                    Some((ATTR_EVENT, H_EMBED_EQ))
                }
                _ => None,
            };
            if let Some((leaf, cx)) = found {
                return Some(self.open(p, w.len(), cx, W_JD | leaf));
            }
        }
        self.generic_attribute(p, c)
    }

    /// `tag-generic-attribute`: `(?=[^[\t\n\f =/>]])`.
    pub(super) fn generic_attribute(&mut self, p: usize, c: u8) -> Option<usize> {
        if is_html_space(self.b, p) || matches!(c, b'=' | b'/' | b'>') {
            return None;
        }
        Some(self.enter(p, &[H_EQ, H_ATTR_NAME]))
    }
}

/// HTML's tag names in its `main`, by the leaf of their name and the context their body is.
static HTML_TAGS: [(&[&[u8]], u16, u8); 8] = [
    (&[b"body", b"head", b"html"], NAME_STRUCTURE, H_TAG),
    (
        &[
            b"address",
            b"applet",
            b"article",
            b"aside",
            b"blockquote",
            b"center",
            b"dd",
            b"dir",
            b"div",
            b"dl",
            b"dt",
            b"figcaption",
            b"figure",
            b"footer",
            b"frame",
            b"frameset",
            b"h1",
            b"h2",
            b"h3",
            b"h4",
            b"h5",
            b"h6",
            b"header",
            b"iframe",
            b"menu",
            b"nav",
            b"noframes",
            b"object",
            b"ol",
            b"p",
            b"pre",
            b"section",
            b"ul",
        ],
        NAME_BLOCK,
        H_TAG,
    ),
    (&[b"hr"], NAME_BLOCK, H_TAG_SC),
    (&[b"form", b"fieldset"], NAME_FORM_BLOCK, H_TAG),
    (
        &[
            b"abbr",
            b"acronym",
            b"area",
            b"audio",
            b"b",
            b"base",
            b"basefont",
            b"bdi",
            b"bdo",
            b"big",
            b"br",
            b"canvas",
            b"caption",
            b"cite",
            b"code",
            b"del",
            b"details",
            b"dfn",
            b"dialog",
            b"em",
            b"font",
            b"head",
            b"html",
            b"i",
            b"img",
            b"ins",
            b"isindex",
            b"kbd",
            b"li",
            b"link",
            b"map",
            b"mark",
            b"menu",
            b"menuitem",
            b"meta",
            b"noscript",
            b"param",
            b"picture",
            b"q",
            b"rp",
            b"rt",
            b"rtc",
            b"ruby",
            b"s",
            b"samp",
            b"script",
            b"small",
            b"source",
            b"span",
            b"strike",
            b"strong",
            b"style",
            b"sub",
            b"summary",
            b"sup",
            b"time",
            b"title",
            b"track",
            b"tt",
            b"u",
            b"var",
            b"video",
            b"wbr",
        ],
        NAME_INLINE,
        H_TAG_SC,
    ),
    (
        &[
            b"button",
            b"datalist",
            b"input",
            b"label",
            b"legend",
            b"meter",
            b"optgroup",
            b"option",
            b"output",
            b"progress",
            b"select",
            b"template",
            b"textarea",
        ],
        NAME_FORM,
        H_TAG_SC,
    ),
    (&[b"a"], NAME_A, H_TAG_SC),
    (
        &[
            b"col",
            b"colgroup",
            b"table",
            b"tbody",
            b"td",
            b"tfoot",
            b"th",
            b"thead",
            b"tr",
        ],
        NAME_TABLE,
        H_TAG_SC,
    ),
];

/// The events of `tag-event-attribute`, after `on`.
static EVENTS: [&[u8]; 64] = [
    b"abort",
    b"autocomplete",
    b"autocompleteerror",
    b"auxclick",
    b"blur",
    b"cancel",
    b"canplay",
    b"canplaythrough",
    b"change",
    b"click",
    b"close",
    b"contextmenu",
    b"cuechange",
    b"dblclick",
    b"drag",
    b"dragend",
    b"dragenter",
    b"dragexit",
    b"dragleave",
    b"dragover",
    b"dragstart",
    b"drop",
    b"durationchange",
    b"emptied",
    b"ended",
    b"error",
    b"focus",
    b"input",
    b"invalid",
    b"keydown",
    b"keypress",
    b"keyup",
    b"load",
    b"loadeddata",
    b"loadedmetadata",
    b"loadstart",
    b"mousedown",
    b"mouseenter",
    b"mouseleave",
    b"mousemove",
    b"mouseout",
    b"mouseover",
    b"mouseup",
    b"mousewheel",
    b"pause",
    b"play",
    b"playing",
    b"progress",
    b"ratechange",
    b"reset",
    b"resize",
    b"scroll",
    b"seeked",
    b"seeking",
    b"select",
    b"show",
    b"sort",
    b"stalled",
    b"submit",
    b"suspend",
    b"timeupdate",
    b"toggle",
    b"volumechange",
    b"waiting",
];
