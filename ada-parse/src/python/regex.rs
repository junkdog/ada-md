//! `Regular Expressions (Python)`, embedded in raw strings: its `main`, its groups, its
//! comments and its character classes.

use super::*;

impl<K: Sink> Scan<'_, '_, K> {
    /// A regex context's own patterns at `p` (its string's prototype is tried before).
    pub(super) fn regex(&mut self, p: usize, c: u8) -> Option<usize> {
        match c {
            RE_COMMENT => {
                if self.is(p, b')') && p < self.b.len() {
                    let k = self.k(L::ReCommentEnd);
                    return self.close(p, p + 1, k);
                }
                None
            }
            RE_SET => {
                if self.is(p, b']') && p < self.b.len() {
                    let k = self.k(L::ReSetEnd);
                    return self.close(p, p + 1, k);
                }
                self.re_class(p).or_else(|| self.re_range(p))
            }
            _ => {
                if c == RE_GROUP && self.is(p, b')') && p < self.b.len() {
                    let k = self.k(L::ReGroupEnd);
                    return self.close(p, p + 1, k);
                }
                self.re_main(p)
            }
        }
    }

    /// The regex's `main`.
    fn re_main(&mut self, p: usize) -> Option<usize> {
        let b: &[u8] = self.b;
        let n = self.b.len();
        let c = self.ch(p).filter(|_| p < n)?;
        let at = |q: usize| b.get(q).copied().filter(|_| q < n);
        match c {
            b'#' => {
                // `(#)[^]\[(){}^$+*?\\|"']*$`
                if b[p + 1..n]
                    .iter()
                    .all(|c| !b"]\\[(){}^$+*?|\"'".contains(c))
                {
                    self.tok(p, p + 1, L::ReCommentMark);
                    self.tok(p + 1, n, L::ReComment);
                    return Some(n + 1);
                }
            }
            b'\\' => {
                // `\\[bBAZzG]`, `\\[1-9][0-9]?`
                match at(p + 1) {
                    Some(b'b' | b'B' | b'A' | b'Z' | b'z' | b'G') => {
                        return self.tok(p, p + 2, L::ReAnchor);
                    }
                    Some(b'1'..=b'9') => {
                        let e = if at(p + 2).is_some_and(|c| c.is_ascii_digit()) {
                            p + 3
                        } else {
                            p + 2
                        };
                        return self.tok(p, e, L::ReBackReference);
                    }
                    _ => {}
                }
            }
            b'^' | b'$' => return self.tok(p, p + 1, L::ReAnchor),
            b'?' | b'+' | b'*' => {
                let e = if matches!(at(p + 1), Some(b'?' | b'+')) {
                    p + 2
                } else {
                    p + 1
                };
                return self.tok(p, e, L::ReQuantifier);
            }
            b'{' => {
                // `\{(\d+,\d+|\d+,|,\d+|\d+)\}\??`
                let d = digits0(&b[..n], p + 1);
                let e = if at(d) == Some(b',') {
                    digits0(&b[..n], d + 1)
                } else {
                    d
                };
                if e > p + 1 && !(d == p + 1 && e == d + 1) && at(e) == Some(b'}') {
                    let e = if at(e + 1) == Some(b'?') {
                        e + 2
                    } else {
                        e + 1
                    };
                    return self.tok(p, e, L::ReQuantifier);
                }
            }
            b'|' => return self.tok(p, p + 1, L::ReOr),
            b'(' => return self.re_group(p),
            _ => {}
        }
        self.re_class(p)
    }

    /// The patterns of `main` that start with `(`.
    fn re_group(&mut self, p: usize) -> Option<usize> {
        let b: &[u8] = self.b;
        let n = self.b.len();
        let at = |q: usize| b.get(q).copied().filter(|_| q < n);
        if at(p + 1) == Some(b'?') {
            match at(p + 2) {
                // `\(\?\#`
                Some(b'#') => {
                    let k = self.base_of(RE_COMMENT) | L::ReCommentBegin as u16;
                    return self.tok_push(p, p + 3, k, &[RE_COMMENT]);
                }
                // `\(\?[iLmsux]+\)`
                Some(b'i' | b'L' | b'm' | b's' | b'u' | b'x') => {
                    let e = (p + 2..n)
                        .find(|&q| !b"iLmsux".contains(&b[q]))
                        .unwrap_or(n);
                    if at(e) == Some(b')') {
                        return self.tok(p, e + 1, L::ReOptionToggle);
                    }
                }
                Some(b'P')
                    if at(p + 3) == Some(b'=')
                    // `(\()(\?P=([a-zA-Z_][a-zA-Z_0-9]*\w*))(\))`
                    && at(p + 4).is_some_and(|c| c.is_ascii_alphabetic() || c == b'_') =>
                {
                    let e = word_end(&b[..n], p + 4);
                    if at(e) == Some(b')') {
                        return self.tok(p, e + 1, L::ReNamedBackReference);
                    }
                }
                _ => {}
            }
            // `(\()((\?=)|(\?!)|(\?<=)|(\?<!))`
            let assertion = match (at(p + 2), at(p + 3)) {
                (Some(b'=' | b'!'), _) => Some(p + 3),
                (Some(b'<'), Some(b'=' | b'!')) => Some(p + 4),
                _ => None,
            };
            if let Some(e) = assertion {
                self.tok(p, p + 1, L::ReGroupBegin);
                return self.group_push(p + 1, e, L::ReAssertion);
            }
            // `(\()(\?\(([1-9][0-9]?|[a-zA-Z_][a-zA-Z_0-9]*)\))`
            if at(p + 2) == Some(b'(') {
                let s = p + 3;
                let e = match at(s) {
                    Some(b'1'..=b'9') => Some(if at(s + 1).is_some_and(|c| c.is_ascii_digit()) {
                        s + 2
                    } else {
                        s + 1
                    }),
                    Some(c) if c.is_ascii_alphabetic() || c == b'_' => Some(
                        (s..n)
                            .find(|&q| !(b[q].is_ascii_alphanumeric() || b[q] == b'_'))
                            .unwrap_or(n),
                    ),
                    _ => None,
                };
                if let Some(e) = e
                    && at(e) == Some(b')')
                {
                    self.tok(p, p + 1, L::ReGroupBegin);
                    self.tok(p + 1, s, L::ReConditional);
                    self.tok(s, e, L::ReConditionalReference);
                    return self.group_push(e, e + 1, L::ReConditional);
                }
            }
        }
        // `(\()((\?P<)([a-z]\w*)(>)|(\?:))?`
        self.tok(p, p + 1, L::ReGroupBegin);
        if at(p + 1) == Some(b'?') {
            if at(p + 2) == Some(b'P')
                && at(p + 3) == Some(b'<')
                && at(p + 4).is_some_and(|c| c.is_ascii_lowercase())
            {
                let e = word_end(&b[..n], p + 4);
                if at(e) == Some(b'>') {
                    self.tok(p + 1, p + 4, L::ReCapture);
                    self.tok(p + 4, e, L::ReGroupName);
                    return self.group_push(e, e + 1, L::ReCapture);
                }
            }
            if at(p + 2) == Some(b':') {
                return self.group_push(p + 1, p + 3, L::ReNoCapture);
            }
        }
        self.group_push(p + 1, p + 1, L::Plain)
    }

    /// The last bytes of a group's opening, `s..e` as `l`; then the group is open.
    fn group_push(&mut self, s: usize, e: usize, l: L) -> Option<usize> {
        let k = self.k(l);
        self.tok_push(s, e, k, &[RE_GROUP])
    }

    /// `character-class`.
    fn re_class(&mut self, p: usize) -> Option<usize> {
        let c = self.ch(p).filter(|_| p < self.b.len())?;
        match c {
            b'\\' => {
                if matches!(
                    self.ch(p + 1),
                    Some(b'w' | b'W' | b's' | b'S' | b'd' | b'D' | b'h' | b'H')
                ) && p + 1 < self.b.len()
                {
                    return self.tok(p, p + 2, L::ReClass);
                }
                let e = self.any(p + 1)?;
                self.tok(p, e, L::ReEscape)
            }
            b'.' => self.tok(p, p + 1, L::ReClass),
            b'[' => {
                let w = self.base_of(RE_SET);
                if self.is(p + 1, b'^') && p + 1 < self.b.len() {
                    self.put(p, p + 1, w | L::ReSetBegin as u16);
                    return self.tok_push(p + 1, p + 2, w | L::ReNegation as u16, &[RE_SET]);
                }
                self.tok_push(p, p + 1, w | L::ReSetBegin as u16, &[RE_SET])
            }
            _ => None,
        }
    }

    /// A set's range: `((\\.)|.)\-((\\.)|[^\]])`.
    fn re_range(&mut self, p: usize) -> Option<usize> {
        let first = |q: usize| -> Option<(usize, bool)> {
            if self.is(q, b'\\')
                && let Some(e) = self.any(q + 1)
            {
                return Some((e, true));
            }
            self.any(q).map(|e| (e, false))
        };
        let (d, esc1) = first(p)?;
        if !self.is(d, b'-') || d >= self.b.len() {
            return None;
        }
        let s = d + 1;
        let (e, esc2) = if self.is(s, b'\\')
            && let Some(e) = self.any(s + 1)
        {
            (e, true)
        } else if self.ch(s).is_some_and(|c| c != b']') {
            (self.next(s), false)
        } else {
            return None;
        };
        if esc1 {
            self.tok(p, d, L::ReRangeEscape);
        } else {
            self.tok(p, d, L::ReRange);
        }
        self.tok(d, s, L::ReRange);
        if esc2 {
            self.tok(s, e, L::ReRangeEscape)
        } else {
            self.tok(s, e, L::ReRange)
        }
    }
}
