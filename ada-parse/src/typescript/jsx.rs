//! TSX's JSX (`TypeScriptReact`'s `jsx`): elements in an expression, their tags, attributes and
//! children. A closing tag closes the open element whatever its name, as the grammar has it.

use super::*;

/// A tag's name at `i`: `\s*(?:(«alpha»[-_$[:alpha:][:digit:].]*)(?<!\.|-)(:))?((?:[a-z][a-z0-9]*|(«alpha»[-_$[:alpha:][:digit:].]*))(?<!\.|-))`
/// with what may follow it: the namespace's end and `:`, and the name's end and kind.
struct TagName {
    ns: Option<(usize, usize)>,
    name: (usize, usize),
    component: bool,
}

impl Scan<'_, '_, '_> {
    /// `[-_$[:alpha:][:digit:].]*` from `i`, backed off a trailing `.` or `-`: its end.
    fn tag_run(&self, i: usize) -> usize {
        let mut e = i;
        loop {
            let a = look::alnum(self.b, e);
            if a > 0 {
                e += a;
            } else if matches!(self.c(e), b'-' | b'.') {
                e += 1;
            } else {
                break;
            }
        }
        e
    }

    /// The tag's name from `i` (after `<` and its blanks), ending where `ok` holds: the regex's
    /// alternatives in order, `[a-z][a-z0-9]*` first.
    fn tag_name(&self, i: usize, ok: &dyn Fn(&Self, usize) -> bool, optional: bool) -> Option<Option<TagName>> {
        // an optional namespace: «alpha»[-_$[:alpha:][:digit:].]*(?<!\.|-)(:)
        let mut starts = Vec::with_capacity(2);
        if look::alpha(self.b, i) > 0 {
            let mut e = self.tag_run(i + look::alpha(self.b, i));
            while e > i + 1 && matches!(self.b[e - 1], b'.' | b'-') {
                e -= 1;
            }
            if self.c(e) == b':' {
                starts.push((Some((i, e)), e + 1));
            }
        }
        starts.push((None, i));
        for (ns, s) in starts {
            // [a-z][a-z0-9]*, backing off to shorter runs
            if self.c(s).is_ascii_lowercase() {
                let mut e = s + 1;
                while self.c(e).is_ascii_lowercase() || self.c(e).is_ascii_digit() {
                    e += 1;
                }
                let mut k = e;
                while k > s {
                    if ok(self, k) {
                        return Some(Some(TagName { ns, name: (s, k), component: false }));
                    }
                    k -= 1;
                }
            }
            // («alpha»[-_$[:alpha:][:digit:].]*)(?<!\.|-), backing off
            let a = look::alpha(self.b, s);
            if a > 0 {
                let mut k = self.tag_run(s + a);
                while k > s {
                    if !matches!(self.b[k - 1], b'.' | b'-') && ok(self, k) {
                        return Some(Some(TagName { ns, name: (s, k), component: true }));
                    }
                    k -= 1;
                    while k > s && self.b[k - 1] & 0xc0 == 0x80 {
                        k -= 1;
                    }
                }
            }
            if optional && ns.is_none() && ok(self, s) {
                return Some(None);
            }
        }
        None
    }

    /// After a tag's name: `(?=((<\s*)|(\s+))(?!\?)|\/?>)`.
    fn tag_follow(&self, k: usize) -> bool {
        match self.c(k) {
            b'<' => self.c(self.ws(k + 1)) != b'?',
            b'>' => true,
            b'/' => self.c(k + 1) == b'>',
            c if is_space(c) => self.c(self.ws(k)) != b'?',
            _ => false,
        }
    }

    /// The tag's tokens: `<`, the namespace and its `:`, the name.
    fn tag_toks(&mut self, lt: usize, lt_end: usize, t: &Option<TagName>) {
        self.tok(lt, lt_end, JSX_TAG_BEGIN);
        if let Some(t) = t {
            if let Some((s, e)) = t.ns {
                self.tok(s, e, JSX_TAG_NS);
                self.tok(e, e + 1, JSX_NS_SEP);
            }
            self.tok(t.name.0, t.name.1, if t.component { JSX_COMPONENT } else { JSX_TAG_NAME });
        }
    }

    /// The lookbehind of `jsx`'s two: `(?<!\+\+|--)(?<=[({\[,?=>:*]|&&|\|\||\?|\*\/|^await|…|^return|…|^default|…|^yield|…|^)`.
    fn jsx_before(&self, i: usize) -> bool {
        if i == 0 {
            return true;
        }
        let b = self.b;
        if i >= 2 && (&b[i - 2..i] == b"++" || &b[i - 2..i] == b"--") {
            return false;
        }
        matches!(b[i - 1], b'(' | b'{' | b'[' | b',' | b'?' | b'=' | b'>' | b':' | b'*')
            || (i >= 2 && matches!(&b[i - 2..i], b"&&" | b"||" | b"*/"))
            || self.word_ends_at(i, &[b"await", b"return", b"default", b"yield"])
    }

    /// `jsx` in an expression: an element pushed without taking anything but blanks.
    pub(super) fn jsx(&mut self) -> bool {
        let i = self.i;
        if !self.jsx_before(i) {
            return false;
        }
        let k = self.ws(i);
        if self.c(k) != b'<' {
            return false;
        }
        let m = self.ws(k + 1);
        // without attributes: <name?\s*>
        if self.tag_name(m, &|me, e| me.c(me.ws(e)) == b'>', true).is_some() {
            self.i = k.min(self.n);
            return if k == i { self.push_zero(JSX_WA_EXPR) } else {
                self.push(JSX_WA_EXPR);
                true
            };
        }
        // (?!<\s*name((\s+extends\s+[^=>])|,)): not type parameters
        let ne = self.name(m);
        if ne > m {
            let g = self.ws(ne);
            if self.c(g) == b',' {
                return false;
            }
            if g > ne && let Some(x) = self.word_b(g, b"extends") {
                let h = self.ws(x);
                if h > x && !matches!(self.c(h), b'=' | b'>') && h < self.n {
                    return false;
                }
            }
        }
        if self.tag_name(m, &|me, e| me.tag_follow(e), false).is_some() {
            self.i = k.min(self.n);
            return if k == i { self.push_zero(JSX_EXPR) } else {
                self.push(JSX_EXPR);
                true
            };
        }
        false
    }

    /// A JSX frame's turn.
    pub(super) fn jsx_step(&mut self, f: u8) -> bool {
        let i = self.i;
        let c = self.c(i);
        match f {
            JSX_WA_EXPR | JSX_EXPR => {
                // pops unless another element starts here
                let m = self.ws(i + 1);
                let another = c == b'<'
                    && if f == JSX_WA_EXPR {
                        self.tag_name(m, &|me, e| me.c(me.ws(e)) == b'>', true).is_some()
                    } else {
                        self.tag_name(m, &|me, e| me.tag_follow(e), false).is_some()
                    };
                if !another {
                    return self.pop_zero();
                }
                if f == JSX_WA_EXPR {
                    self.jsx_without_attributes()
                } else {
                    self.push_zero(JSX_TAG_F)
                }
            }
            JSX_WA => {
                // (</)\s*name?\s*(>)
                if c == b'<' && self.c(i + 1) == b'/' {
                    let m = self.ws(i + 2);
                    if let Some(t) = self.tag_name(m, &|me, e| me.c(me.ws(e)) == b'>', true) {
                        let e = t.as_ref().map_or(m, |t| t.name.1);
                        let g = self.ws(e);
                        self.tag_toks(i, i + 2, &t);
                        self.tok(g, g + 1, JSX_TAG_END);
                        self.i = g + 1;
                        return self.pop();
                    }
                }
                self.jsx_children()
            }
            JSX_TAG_F => {
                // (/>)|(?:(</)\s*name?\s*(>)): the element's end
                if c == b'/' && self.c(i + 1) == b'>' {
                    self.tok(i, i + 2, JSX_TAG_END);
                    self.i = i + 2;
                    return self.pop();
                }
                if c == b'<' && self.c(i + 1) == b'/' {
                    let m = self.ws(i + 2);
                    if let Some(t) = self.tag_name(m, &|me, e| me.c(me.ws(e)) == b'>', true) {
                        let e = t.as_ref().map_or(m, |t| t.name.1);
                        let g = self.ws(e);
                        self.tag_toks(i, i + 2, &t);
                        self.tok(g, g + 1, JSX_TAG_END);
                        self.i = g + 1;
                        return self.pop();
                    }
                }
                if c == b'<' {
                    let m = self.ws(i + 1);
                    if let Some(Some(t)) = self.tag_name(m, &|me, e| me.tag_follow(e), false) {
                        let t = Some(t);
                        self.tag_toks(i, i + 1, &t);
                        self.i = t.as_ref().map_or(m, |t| t.name.1);
                        self.push(JSX_ATTRS);
                        return true;
                    }
                }
                if c == b'>' {
                    self.tok(i, i + 1, JSX_TAG_END);
                    self.i = i + 1;
                    self.push(JSX_CHILDREN);
                    return true;
                }
                false
            }
            JSX_ATTRS | JSX_ATTRS2 => {
                if (c == b'>' || (c == b'/' && self.c(i + 1) == b'>')) && self.pop_zero() {
                    return true;
                }
                if self.comment(false) {
                    return true;
                }
                if f == JSX_ATTRS {
                    if c == b'<' && self.type_arguments() {
                        return true;
                    }
                    // jsx-tag-attributes: \s+
                    if is_space(c) {
                        self.push(JSX_ATTRS2);
                        self.i = self.ws(i).min(self.n + 1);
                        return true;
                    }
                    return false;
                }
                self.jsx_attribute()
            }
            JSX_CHILDREN => {
                if c == b'<' && self.c(i + 1) == b'/' && self.pop_zero() {
                    return true;
                }
                self.jsx_children()
            }
            JSX_EMBED => {
                if c == b'}' {
                    self.tok(i, i + 1, JSX_EMBED_END);
                    self.i = i + 1;
                    return self.pop();
                }
                self.expression()
            }
            JSX_STR_D | JSX_STR_S => {
                let q = if f == JSX_STR_D { b'"' } else { b'\'' };
                if c == q {
                    self.tok(i, i + 1, if f == JSX_STR_D { JSX_STR_D_END } else { JSX_STR_S_END });
                    self.i = i + 1;
                    return self.pop();
                }
                if c == b'&' {
                    return self.jsx_entity();
                }
                false
            }
            _ => false,
        }
    }

    /// `jsx-tag-without-attributes`: `(<)\s*name?\s*(>)`, then its children.
    fn jsx_without_attributes(&mut self) -> bool {
        let i = self.i;
        if self.c(i) != b'<' {
            return false;
        }
        let m = self.ws(i + 1);
        let Some(t) = self.tag_name(m, &|me, e| me.c(me.ws(e)) == b'>', true) else { return false };
        let e = t.as_ref().map_or(m, |t| t.name.1);
        let g = self.ws(e);
        self.tag_toks(i, i + 1, &t);
        self.tok(g, g + 1, JSX_TAG_END);
        self.i = g + 1;
        self.push(JSX_WA);
        true
    }

    /// `jsx-children`.
    fn jsx_children(&mut self) -> bool {
        let i = self.i;
        match self.c(i) {
            b'<' => {
                if self.jsx_without_attributes() {
                    return true;
                }
                let m = self.ws(i + 1);
                if self.tag_name(m, &|me, e| me.tag_follow(e), false).is_some() {
                    return self.push_zero(JSX_TAG_F);
                }
                false
            }
            b'{' => {
                self.tok(i, i + 1, JSX_EMBED_BEGIN);
                self.i = i + 1;
                self.push(JSX_EMBED);
                true
            }
            b'&' => self.jsx_entity(),
            _ => false,
        }
    }

    /// `jsx-entities`.
    fn jsx_entity(&mut self) -> bool {
        let i = self.i;
        let mut e = i + 1;
        let ok = if self.c(e) == b'#' {
            e += 1;
            if self.c(e) == b'x' {
                let s = e + 1;
                e = s;
                while self.c(e).is_ascii_hexdigit() {
                    e += 1;
                }
                e > s
            } else {
                let s = e;
                while self.c(e).is_ascii_digit() {
                    e += 1;
                }
                e > s
            }
        } else {
            let s = e;
            while self.c(e).is_ascii_alphanumeric() {
                e += 1;
            }
            e > s
        };
        if ok && self.c(e) == b';' {
            return self.take(i, e + 1, JSX_ENTITY);
        }
        // `#x` can back off to `#` and digits: `&#x;` isn't one; `&#1x;` neither
        self.take(i, i + 1, JSX_BAD_AMP)
    }

    /// An attribute's patterns.
    fn jsx_attribute(&mut self) -> bool {
        let i = self.i;
        let c = self.c(i);
        // \s*(?:(«alpha»[-_$[:alpha:][:digit:].]*)(:))?(«alpha»[-_$[:alpha:][:digit:]]*)(?=\s|=|/?>|/\*|//)
        let s = self.ws(i);
        if look::alpha(self.b, s) > 0 {
            let run = self.tag_run(s + look::alpha(self.b, s));
            let mut ns = None;
            let mut n0 = s;
            if self.c(run) == b':' && look::alpha(self.b, run + 1) > 0 {
                ns = Some((s, run));
                n0 = run + 1;
            }
            let mut e = n0 + look::alpha(self.b, n0);
            if e > n0 {
                loop {
                    let a = look::alnum(self.b, e);
                    if a > 0 {
                        e += a;
                    } else if self.c(e) == b'-' {
                        e += 1;
                    } else {
                        break;
                    }
                }
                let d = self.c(e);
                let follows = is_space(d) || d == b'=' || d == b'>' || (d == b'/' && matches!(self.c(e + 1), b'>' | b'*' | b'/'));
                if follows {
                    if let Some((a, b)) = ns {
                        self.tok(a, b, JSX_ATTR_NS);
                        self.tok(b, b + 1, JSX_NS_SEP);
                    }
                    return self.take(n0, e, JSX_ATTR);
                }
            }
        }
        match c {
            b'=' => {
                let k = self.ws(i + 1);
                if matches!(self.c(k), b'\'' | b'"' | b'{') || (self.c(k) == b'/' && matches!(self.c(k + 1), b'*' | b'/')) || k >= self.n {
                    return self.take(i, i + 1, ASSIGN);
                }
            }
            b'"' => {
                self.tok(i, i + 1, JSX_STR_D_BEGIN);
                self.i = i + 1;
                self.push(JSX_STR_D);
                return true;
            }
            b'\'' => {
                self.tok(i, i + 1, JSX_STR_S_BEGIN);
                self.i = i + 1;
                self.push(JSX_STR_S);
                return true;
            }
            b'{' => {
                self.tok(i, i + 1, JSX_EMBED_BEGIN);
                self.i = i + 1;
                self.push(JSX_EMBED);
                return true;
            }
            _ => {}
        }
        // jsx-tag-attributes-illegal: \S+
        if i < self.n && !is_space(c) {
            let mut e = i;
            while e < self.n && !is_space(self.b[e]) {
                e += 1;
            }
            return self.take(i, e, JSX_ILLEGAL);
        }
        false
    }
}
