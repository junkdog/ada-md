//! Perl's variables (`$x`, `@x`, `%x`, `&x`, `*x`, their package paths, dereferences and the
//! special ones), and a sub's head: its name, signature or prototype, attributes.

use super::text::special_name as special;
use super::*;

/// Where a special variable's punctuation name may stand after `$` in code: bat's
/// `[".:,;\]~?!/\\|()<>]`, with `@`, `%`, `=`, `[` and `0`.
static PUNCT_VAR: [bool; 256] = crate::regex::table(b"\".:,;]~?!/\\|()<>@%=[0");

impl<K: Sink> Scan<'_, '_, K> {
    /// `$` at `p` in code: a scalar, an element, `$#array`, a dereference, a special variable.
    #[inline(always)]
    pub(super) fn dollar(&mut self, p: usize) -> usize {
        let b = self.b;
        if self.prev == P_PARAM {
            return self.param(p);
        }
        let c1 = b.get(p + 1).copied().unwrap_or(b'\n');
        if is_name_start(c1) {
            let e = name_run(b, p + 2);
            if b.get(e) == Some(&b':') && b.get(e + 1) == Some(&b':') {
                return self.var_path(p, p + 1);
            }
            let special = special(b, p + 1, e, b'$');
            self.put_hot(p, p + 1, if special { VAR_LANG_SIGIL } else { VAR_SIGIL });
            self.put_hot(p + 1, e, if special { VAR_LANG } else { VAR });
            self.prev = P_VALUE;
            return e;
        }
        self.dollar_rare(p, c1)
    }

    /// `$` at `p` before `c1`, not a name: `${…}`, `$$x`, `$#x`, `$1`, `$&`, `$^W`, `$::x`.
    #[inline(never)]
    fn dollar_rare(&mut self, p: usize, c1: u8) -> usize {
        let b = self.b;
        let at = |i: usize| b.get(p + i).copied().unwrap_or(b'\n');
        self.prev = P_VALUE;
        match c1 {
            b':' if at(2) == b':' => self.var_path(p, p + 1),
            b'{' => self.brace_var(p, p + 1),
            b'$' if is_name_start(at(2)) || matches!(at(2), b'{' | b'$' | b':') => {
                // `$$ref`: a dereference of `$ref`
                self.put(p, p + 1, DEREF);
                self.prev = P_TERM;
                p + 1
            }
            b'#' => {
                let c2 = at(2);
                if is_name_start(c2) {
                    // `$#array`
                    let e = name_run(b, p + 3);
                    self.put(p, p + 2, VAR_SIGIL);
                    self.put(p + 2, e, VAR);
                    e
                } else if matches!(c2, b'{' | b'$') {
                    // `$#{…}`, `$#$ref`
                    if c2 == b'{' {
                        return self.brace_var(p, p + 2);
                    }
                    self.put(p, p + 2, DEREF);
                    self.prev = P_TERM;
                    p + 2
                } else {
                    self.put(p, p + 2, VAR_LANG);
                    p + 2
                }
            }
            b'0'..=b'9' => {
                let mut e = p + 2;
                while e < b.len() && b[e].is_ascii_digit() {
                    e += 1;
                }
                if c1 == b'0' && e == p + 2 {
                    self.put(p, p + 1, VAR_LANG_SIGIL);
                    self.put(p + 1, e, VAR_LANG);
                } else {
                    self.put(p, p + 1, GROUP_SIGIL);
                    self.put(p + 1, e, GROUP_VAR);
                }
                e
            }
            b'&' | b'`' | b'\'' => {
                self.put(p, p + 1, MATCH_SIGIL);
                self.put(p + 1, p + 2, MATCH_VAR);
                p + 2
            }
            b'+' | b'-' => {
                self.put(p, p + 1, GROUP_SIGIL);
                self.put(p + 1, p + 2, GROUP_VAR);
                p + 2
            }
            b'^' if at(2).is_ascii_uppercase() || matches!(at(2), b'[' | b']' | b'_' | b'?') => {
                self.put(p, p + 1, VAR_LANG_SIGIL);
                self.put(p + 1, p + 3, VAR_LANG);
                p + 3
            }
            b'$' => {
                // `$$`: the process's id
                self.put(p, p + 1, VAR_LANG_SIGIL);
                self.put(p + 1, p + 2, VAR_LANG);
                p + 2
            }
            c if PUNCT_VAR[usize::from(c)] => {
                self.put(p, p + 1, VAR_LANG_SIGIL);
                self.put(p + 1, p + 2, VAR_LANG);
                p + 2
            }
            _ => {
                self.put(p, p + 1, VAR_SIGIL);
                self.prev = P_TERM;
                p + 1
            }
        }
    }

    /// A variable's package path from `p` (its sigil), the path from `q` (`$Foo::Bar::x`,
    /// `@::list`): its packages, then its name.
    #[inline(never)]
    fn var_path(&mut self, p: usize, q: usize) -> usize {
        let b = self.b;
        let end = if b[q] == b':' {
            qualified_end(b, q)
        } else {
            qualified_end(b, name_run(b, q + 1))
        };
        let mut last = end;
        while last > q && is_name_byte(b[last - 1]) {
            last -= 1;
        }
        self.put(p, q, VAR_SIGIL);
        let mut r = q;
        while r < last {
            if b[r] == b':' {
                self.put(r, r + 2, DCOLON);
                r += 2;
            } else {
                let w = name_run(b, r + 1);
                self.put(r, w, NS);
                r = w;
            }
        }
        if last < end {
            self.put(last, end, VAR);
        }
        self.prev = P_VALUE;
        end
    }

    /// A sigil's `{` at `q` (`${…}`, `@{…}`, `%{…}`, `$#{…}`), the sigil from `p`: `${^NAME}`
    /// a special variable, `${name}` a name, else a dereference's block.
    #[inline(never)]
    fn brace_var(&mut self, p: usize, q: usize) -> usize {
        let b = self.b;
        if !SOL {
            let r = skip_blanks(b, q + 1);
            if b.get(r) == Some(&b'^') {
                // `${^MATCH}`
                let e = name_run(b, r + 1);
                if b.get(e) == Some(&b'}') {
                    self.put(p, q, VAR_LANG_SIGIL);
                    self.put(q, e + 1, VAR_LANG);
                    self.prev = P_VALUE;
                    return e + 1;
                }
            } else if b.get(r).is_some_and(|&c| is_name_start(c)) {
                // `${name}`: the name a string
                let e = name_run(b, r + 1);
                let t = skip_blanks(b, e);
                if b.get(t) == Some(&b'}') {
                    self.put(p, q, DEREF);
                    self.put(q, q + 1, VBEGIN);
                    self.put(r, e, str_leaf(C_UNQ));
                    self.put(t, t + 1, VEND);
                    self.prev = P_VALUE;
                    return t + 1;
                }
            }
        }
        self.put(p, q, DEREF);
        self.put(q, q + 1, VBEGIN);
        self.push(F_DEREF);
        self.prev = P_TERM;
        q + 1
    }

    /// `@` at `p`; `%`, `&` and `*` where a term starts: an array, a hash, a sub's call, a glob,
    /// or their dereferences.
    #[inline(always)]
    pub(super) fn at(&mut self, p: usize) -> usize {
        self.sigil(p)
    }

    #[inline(always)]
    pub(super) fn sigil(&mut self, p: usize) -> usize {
        let b = self.b;
        if self.prev == P_PARAM {
            return self.param(p);
        }
        let s = b[p];
        let c1 = b.get(p + 1).copied().unwrap_or(b'\n');
        if is_name_start(c1) {
            let e = name_run(b, p + 2);
            if b.get(e) == Some(&b':') && b.get(e + 1) == Some(&b':') {
                if s == b'&' {
                    self.put(p, p + 1, DEREF);
                    self.prev = P_TERM;
                    return self.name(p + 1);
                }
                return self.var_path(p, p + 1);
            }
            if s == b'&' {
                // `&foo`: a sub's call
                self.put_hot(p, p + 1, DEREF);
                let w = WORDS.value(&b[p + 1..e], &INFO) & W_ACTION;
                self.put_hot(
                    p + 1,
                    e,
                    if matches!(w, W_FN | W_LIST | W_DO) {
                        SUPPORT
                    } else {
                        FUNC
                    },
                );
                self.prev = P_VALUE;
                return e;
            }
            let special = s != b'*' && special(b, p + 1, e, s);
            self.put_hot(p, p + 1, if special { VAR_LANG_SIGIL } else { VAR_SIGIL });
            self.put_hot(p + 1, e, if special { VAR_LANG } else { VAR });
            self.prev = P_VALUE;
            return e;
        }
        self.sigil_rare(p, s, c1)
    }

    /// Sigil `s` at `p` before `c1`, not a name: `@{…}`, `@$x`, `@::x`, `@-`, `%+`, `%^H`.
    #[inline(never)]
    fn sigil_rare(&mut self, p: usize, s: u8, c1: u8) -> usize {
        let b = self.b;
        let at = |i: usize| b.get(p + i).copied().unwrap_or(b'\n');
        match c1 {
            b':' if at(2) == b':' => self.var_path(p, p + 1),
            b'{' => self.brace_var(p, p + 1),
            b'$' => {
                self.put(p, p + 1, DEREF);
                self.prev = P_TERM;
                p + 1
            }
            b'-' | b'+' if s != b'&' && s != b'*' => {
                self.put(p, p + 1, GROUP_SIGIL);
                self.put(p + 1, p + 2, GROUP_VAR);
                self.prev = P_VALUE;
                p + 2
            }
            b'^' if s == b'%' && at(2).is_ascii_uppercase() => {
                self.put(p, p + 1, VAR_LANG_SIGIL);
                self.put(p + 1, p + 3, VAR_LANG);
                self.prev = P_VALUE;
                p + 3
            }
            _ => {
                // a lone sigil: an operator where one is (`% 2` after a lost value)
                let kind = match s {
                    b'@' => VAR_SIGIL,
                    b'&' => BITWISE,
                    _ => ARITH,
                };
                self.put(p, p + 1, kind);
                self.prev = P_TERM;
                p + 1
            }
        }
    }

    /// A signature's or prototype's parameter at `p` (`$x`, `@rest`, `%opts`, `$$;@`, `\@`):
    /// the sigils and the name a parameter's, `;` a separator.
    #[cold]
    #[inline(never)]
    fn param(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut e = p;
        loop {
            let s = e;
            while e < n
                && matches!(
                    b[e],
                    b'$' | b'@' | b'%' | b'&' | b'*' | b'\\' | b'[' | b']' | b'+'
                )
            {
                e += 1;
            }
            if e > s && e < n && is_name_start(b[e]) {
                e = name_run(b, e + 1);
            } else if e < n && b[e] == b'_' {
                e += 1;
            }
            if e > s {
                self.put(s, e, PARAM);
            }
            if e < n && b[e] == b';' {
                self.put(e, e + 1, COMMA);
                e += 1;
                continue;
            }
            break;
        }
        self.prev = P_VALUE;
        e.max(p + 1)
    }

    /// The name after `sub` from `e`: a named sub's (its package's path too), a callback's
    /// (`DESTROY`), a phase's (`BEGIN`); none for an anonymous sub.
    #[inline(never)]
    pub(super) fn sub_name(&mut self, e: usize) -> usize {
        let b = self.b;
        let q = skip_blanks(b, e);
        if !b.get(q).is_some_and(|&c| is_name_start(c)) {
            return e;
        }
        let end = qualified_end(b, name_run(b, q + 1));
        let mut last = end;
        while last > q && is_name_byte(b[last - 1]) {
            last -= 1;
        }
        let mut r = q;
        while r < last {
            if b[r] == b':' {
                self.put(r, r + 2, DCOLON);
                r += 2;
            } else {
                let w = name_run(b, r + 1);
                self.put(r, w, NS);
                r = w;
            }
        }
        if last < end {
            let kind = match WORDS.value(&b[last..end], &INFO) & W_ACTION {
                W_CALLBACK => CALLBACK,
                W_PHASE => PREPROC,
                _ => ENTITY,
            };
            self.put(last, end, kind);
        }
        self.prev = P_SUBNAME;
        end
    }

    /// A signature's `(` written, its parameters from `p`.
    #[inline(always)]
    pub(super) fn signature(&mut self, p: usize) -> usize {
        self.prev = P_PARAM;
        p
    }

    /// `:` at `p` after a sub's name: its attributes (`:lvalue`, `:prototype($)`, `: method`).
    #[cold]
    #[inline(never)]
    pub(super) fn attributes(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        self.put(p, p + 1, ATTR_MARK);
        let mut q = skip_blanks(b, p + 1);
        while q < n && is_name_start(b[q]) {
            let e = name_run(b, q + 1);
            self.put(q, e, ATTR);
            q = e;
            if b.get(q) == Some(&b'(') {
                // its arguments, to their `)` on the line
                let mut depth = 0usize;
                let mut r = q;
                while r < n {
                    match b[r] {
                        b'(' => depth += 1,
                        b')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        b'\\' => r += 1,
                        _ => {}
                    }
                    r += 1;
                }
                self.put(q, q + 1, PARAMS_B);
                if r >= n {
                    return n;
                }
                if &b[q - 9.min(q)..q] == b"prototype" {
                    // `:prototype($;@)`: a prototype's characters
                    self.put(q + 1, r, PARAM);
                } else {
                    // its arguments as code, as bat reads them (`:Path('a')`, `:Args(0)`)
                    self.code_bounded(q + 1, r);
                }
                self.put(r, r + 1, PARAMS_E);
                q = r + 1;
            }
            let t = skip_blanks(b, q);
            if b.get(t) == Some(&b':') && b.get(t + 1) != Some(&b':') {
                self.put(t, t + 1, ATTR_MARK);
                q = skip_blanks(b, t + 1);
            } else {
                q = t;
            }
        }
        self.prev = P_SUBNAME;
        q.max(p + 1)
    }
}
