//! The regexes of bat's `Makefile.sublime-syntax` that Makefile's scanner (`makefile.rs`) keeps
//! syntect's rule for, each a matcher of its own: it finds the pattern's leftmost match at or
//! after a position, as Oniguruma's search would, in time linear in what it reads. Each
//! matcher's comment is the regex it stands for. From G-002's port of syntect's machine, whose
//! contexts and shell grammar the scanners replaced (ADR-40).

use super::scan::{
    Line, M, R, eol_at, find, find_str, find2, non_ws, spaces_back, ws, ws_back, ws_end,
};

/// `''`
pub fn empty(_: &Line, pos: usize, _: u16) -> R {
    Some(M::at(pos, pos))
}

/// `^`
pub fn bol(_: &Line, pos: usize, _: u16) -> R {
    (pos == 0).then(|| M::at(0, 0))
}

/// `$`
pub fn eol(l: &Line, pos: usize, _: u16) -> R {
    let at = eol_at(l, pos);
    Some(M::at(at, at))
}

/// `$\n?`
pub(crate) fn eol_nl(l: &Line, pos: usize, _: u16) -> R {
    let at = eol_at(l, pos);
    let end = if l.at(at) == b'\n' { at + 1 } else { at };
    Some(M::at(at, end))
}

/// A byte: `\#`, `\(`, `,`, …
pub fn byte<const C: u8>(l: &Line, pos: usize, _: u16) -> R {
    find(l.b, pos, C).map(|i| M::at(i, i + 1))
}

/// `(?=C)`
pub fn ahead<const C: u8>(l: &Line, pos: usize, _: u16) -> R {
    find(l.b, pos, C).map(|i| M::at(i, i))
}

/// `\s`
pub(crate) fn ws1(l: &Line, pos: usize, _: u16) -> R {
    (pos..l.len())
        .find(|&i| ws(l.b[i]))
        .map(|i| M::at(i, i + 1))
}

/// `\n`
pub(crate) fn nl(l: &Line, pos: usize, _: u16) -> R {
    byte::<b'\n'>(l, pos, 0)
}

/// `\s*`
pub(crate) fn eat_ws(l: &Line, pos: usize, _: u16) -> R {
    Some(M::at(pos, ws_end(l.b, pos)))
}

// ---- the line's start ----

/// `^\s*` then `kw`, at the start of the line: where `kw` starts and ends.
pub(crate) fn bol_kw(l: &Line, pos: usize, kw: &[u8]) -> Option<(usize, usize)> {
    if pos != 0 {
        return None;
    }
    let i = ws_end(l.b, 0);
    l.b[i..].starts_with(kw).then_some((i, i + kw.len()))
}

/// `^\s*(KW)\b`
pub(crate) fn bol_word(l: &Line, pos: usize, kw: &[u8]) -> R {
    let (i, e) = bol_kw(l, pos, kw)?;
    (!l.word_at(e)).then(|| M::at(0, e).cap(1, i, e))
}

/// `^\s*(endef)`
pub(crate) fn endef_stray(l: &Line, pos: usize, _: u16) -> R {
    let (i, e) = bol_kw(l, pos, b"endef")?;
    Some(M::at(0, e).cap(1, i, e))
}

/// `^\s*(endef)\b`
pub(crate) fn endef(l: &Line, pos: usize, _: u16) -> R {
    bol_word(l, pos, b"endef")
}

/// `^\s*define\b`
pub(crate) fn define_nested(l: &Line, pos: usize, _: u16) -> R {
    bol_word(l, pos, b"define")
}

/// `^\s*(else)\b`
pub(crate) fn else_kw(l: &Line, pos: usize, _: u16) -> R {
    bol_word(l, pos, b"else")
}

/// `^\s*(endif)\b`
pub(crate) fn endif_kw(l: &Line, pos: usize, _: u16) -> R {
    bol_word(l, pos, b"endif")
}

/// `^\s*(export)\b`
pub(crate) fn export_kw(l: &Line, pos: usize, _: u16) -> R {
    bol_word(l, pos, b"export")
}

/// `^\s*(vpath)\s`
pub(crate) fn vpath_ws(l: &Line, pos: usize, _: u16) -> R {
    let (i, e) = bol_kw(l, pos, b"vpath")?;
    ws(l.at(e)).then(|| M::at(0, e + 1).cap(1, i, e))
}

/// `^\s*(vpath)$`
pub(crate) fn vpath_eol(l: &Line, pos: usize, _: u16) -> R {
    let (_, e) = bol_kw(l, pos, b"vpath")?;
    (e == l.len() || l.at(e) == b'\n').then(|| M::at(0, e))
}

/// `^\s*([s-]?include)\s+`
pub(crate) fn include_kw(l: &Line, pos: usize, _: u16) -> R {
    if pos != 0 {
        return None;
    }
    let i = ws_end(l.b, 0);
    let j = if matches!(l.at(i), b's' | b'-') && l.b[i + 1..].starts_with(b"include") {
        i + 1
    } else {
        i
    };
    if !l.b[j..].starts_with(b"include") {
        return None;
    }
    let e = j + 7;
    let end = ws_end(l.b, e);
    (end > e).then(|| M::at(0, end).cap(1, i, e))
}

/// `ifn?(def|eq)` at `i`: where it ends.
pub(crate) fn if_directive(l: &Line, i: usize) -> Option<usize> {
    if !l.b[i..].starts_with(b"if") {
        return None;
    }
    let j = i + 2;
    (l.at(j) == b'n')
        .then(|| if_tail(l, j + 1))
        .flatten()
        .or_else(|| if_tail(l, j))
}

/// `\bifn?(def|eq)\b`
pub(crate) fn ifdef(l: &Line, pos: usize, _: u16) -> R {
    let mut at = pos;
    while let Some(i) = find_str(l.b, at, b"if") {
        if !l.word_before(i) {
            // `n?` backtracks when `ifn…` doesn't go on as a directive
            let j = i + 2;
            let ends = [
                (l.at(j) == b'n').then(|| if_tail(l, j + 1)).flatten(),
                if_tail(l, j),
            ];
            if let Some(e) = ends.into_iter().flatten().find(|&e| !l.word_at(e)) {
                return Some(M::at(i, e));
            }
        }
        at = i + 1;
    }
    None
}

/// `(def|eq)` at `j`: where it ends.
pub(crate) fn if_tail(l: &Line, j: usize) -> Option<usize> {
    if l.b[j..].starts_with(b"def") {
        Some(j + 3)
    } else if l.b[j..].starts_with(b"eq") {
        Some(j + 2)
    } else {
        None
    }
}

/// `^\s*(ifn?(def|eq))`
pub(crate) fn recipe_if(l: &Line, pos: usize, _: u16) -> R {
    if pos != 0 {
        return None;
    }
    let i = ws_end(l.b, 0);
    let e = if_directive(l, i)?;
    Some(M::at(0, e).cap(1, i, e))
}

/// `(?=^\s*#)`
pub(crate) fn comment_ahead(l: &Line, pos: usize, _: u16) -> R {
    (pos == 0 && l.at(ws_end(l.b, 0)) == b'#').then(|| M::at(0, 0))
}

// ---- continuations and escapes ----

/// `(\\)([ ]*)$\n?`: only the last `\` before the trailing spaces can match.
pub(crate) fn line_cont(l: &Line, pos: usize, _: u16) -> R {
    let b = l.b;
    let n = b.len();
    let end = if b.last() == Some(&b'\n') { n - 1 } else { n };
    let k = spaces_back(b, 0, end);
    if k == 0 || b[k - 1] != b'\\' || k - 1 < pos {
        return None;
    }
    Some(M::at(k - 1, n).cap(1, k - 1, k).cap(2, k, end))
}

/// `(?=\S|^\s*$)`
pub(crate) fn cont_end(l: &Line, pos: usize, _: u16) -> R {
    match non_ws(l.b, pos) {
        Some(i) => Some(M::at(i, i)),
        None if pos == 0 => Some(M::at(0, 0)),
        None => None,
    }
}

/// `\\.`
pub fn escape_any(l: &Line, pos: usize, _: u16) -> R {
    let mut at = pos;
    while let Some(i) = find(l.b, at, b'\\') {
        if i + 1 < l.len() && l.b[i + 1] != b'\n' {
            return Some(M::at(i, i + 1 + l.char_len(i + 1)));
        }
        at = i + 1;
    }
    None
}

// ---- `just_eat`: is the line a rule, or a variable? ----

/// The next paren at or after `q`, or the line's end.
pub(crate) fn next_paren(b: &[u8], q: usize) -> usize {
    find2(b, q, b'(', b')').unwrap_or(b.len())
}

/// A `:` not followed by `=` in `b[a..r]`.
pub(crate) fn colon_in(b: &[u8], a: usize, r: usize) -> bool {
    last_colon(b, a, r).is_some()
}

pub(crate) fn last_colon(b: &[u8], a: usize, r: usize) -> Option<usize> {
    (a..r)
        .rev()
        .find(|&c| b[c] == b':' && b.get(c + 1) != Some(&b'='))
}

pub(crate) fn first_eq(b: &[u8], a: usize, r: usize) -> Option<usize> {
    find(&b[..r], a, b'=')
}

/// The group opened at `r`, when `just_eat` takes it: where it closes, and where the text
/// after it runs to.
pub(crate) fn group_at(l: &Line, r: usize) -> Option<(usize, usize)> {
    if l.at(r) != b'(' {
        return None;
    }
    let close = l.parens().fitting(r)?;
    Some((close, next_paren(l.b, close + 1)))
}

/// `(?={{rule_lookahead}})`: `just_eat` then a `:` that isn't `:=`. `just_eat` takes text
/// without parens, or that, one group of the shapes it allows, and more text: so from `q` the
/// colon is in the run of text up to the next paren, or in the run after the group that paren
/// opens. Within a run a later start only loses text, so the first start of each run decides.
pub(crate) fn rule_ahead(l: &Line, pos: usize, _: u16) -> R {
    let b = l.b;
    let mut q = pos;
    while q < b.len() {
        let r = next_paren(b, q);
        let group = group_at(l, r);
        if colon_in(b, q, r) || group.is_some_and(|(c, r1)| colon_in(b, c + 1, r1)) {
            return Some(M::at(q, q));
        }
        q = r + 1;
    }
    None
}

/// `(?={{var_lookahead}}|{{first_assign_then_colon}})`: an `=` after `just_eat` where no rule
/// colon is. On Oniguruma the `(?x)` that starts `var_lookahead_base` takes the rest of the
/// group, the `|` included, so `(?!{{rule_lookahead}})` guards both branches; and where no rule
/// colon is, an `=` and then a colon is an `=` too.
pub(crate) fn var_ahead(l: &Line, pos: usize, _: u16) -> R {
    let b = l.b;
    let mut q = pos;
    while q < b.len() {
        let r = next_paren(b, q);
        let group = group_at(l, r);
        let colon_after = group.is_some_and(|(c, r1)| colon_in(b, c + 1, r1));
        let eq_after = group.is_some_and(|(c, r1)| first_eq(b, c + 1, r1).is_some());
        // no rule from here holds past the run's last colon
        if !colon_after {
            let v = last_colon(b, q, r).map_or(q, |c| c + 1);
            if v < r && (first_eq(b, v, r).is_some() || eq_after) {
                return Some(M::at(v, v));
            }
            if v == r && l.at(r) == b'(' && eq_after {
                return Some(M::at(r, r));
            }
        }
        q = r + 1;
    }
    None
}

// ---- rules ----

/// `(?= *::?[^=]+[!:?]?=)`
pub(crate) fn colon_then_eq_ahead(l: &Line, pos: usize, _: u16) -> R {
    let b = l.b;
    let last = l.last_eq()?;
    let mut at = pos;
    while let Some(c) = find(b, at, b':') {
        if last <= c + 1 {
            return None;
        }
        if b[c + 1] != b'=' {
            return Some(M::at(spaces_back(b, pos, c), spaces_back(b, pos, c)));
        }
        at = c + 1;
    }
    None
}

/// `(?= *::?)`
pub(crate) fn colon_ahead(l: &Line, pos: usize, _: u16) -> R {
    let c = find(l.b, pos, b':')?;
    let q = spaces_back(l.b, pos, c);
    Some(M::at(q, q))
}

/// `::?` at `c`: where it ends.
pub(crate) fn colons_end(l: &Line, c: usize) -> usize {
    if l.at(c + 1) == b':' { c + 2 } else { c + 1 }
}

/// `(::?)\s*`
pub(crate) fn colons_ws(l: &Line, pos: usize, _: u16) -> R {
    let c = find(l.b, pos, b':')?;
    let e = colons_end(l, c);
    Some(M::at(c, ws_end(l.b, e)).cap(1, c, e))
}

/// `::?`
pub(crate) fn colons(l: &Line, pos: usize, _: u16) -> R {
    let c = find(l.b, pos, b':')?;
    Some(M::at(c, colons_end(l, c)))
}

/// `(?=\s*;)`
pub(crate) fn semicolon_ahead(l: &Line, pos: usize, _: u16) -> R {
    let c = find(l.b, pos, b';')?;
    let q = ws_back(l.b, pos, c);
    Some(M::at(q, q))
}

// ---- recipes ----

/// `^(?=[ ]+([-@]{1,2})?)`
pub(crate) fn spaces_ahead(l: &Line, pos: usize, _: u16) -> R {
    (pos == 0 && l.at(0) == b' ').then(|| M::at(0, 0))
}

/// `^(?=\t([-@]{1,2})?)`
pub(crate) fn tab_ahead(l: &Line, pos: usize, _: u16) -> R {
    (pos == 0 && l.at(0) == b'\t').then(|| M::at(0, 0))
}

/// `([-@]{1,2})?` from `i`: where it ends.
pub(crate) fn prefix_end(l: &Line, i: usize) -> usize {
    let mut e = i;
    while e < i + 2 && matches!(l.at(e), b'-' | b'@') {
        e += 1;
    }
    e
}

/// `^([ ]+)([-@]{1,2})?`
pub(crate) fn recipe_spaces(l: &Line, pos: usize, _: u16) -> R {
    if pos != 0 || l.at(0) != b' ' {
        return None;
    }
    let s = (0..l.len()).find(|&i| l.b[i] != b' ').unwrap_or(l.len());
    let e = prefix_end(l, s);
    Some(M::at(0, e).cap(1, 0, s).cap(2, s, e))
}

/// `^(\t)([-@]{1,2})?`; group 1 is the prefix when `ONE` is false, `^\t([-@]{1,2})?`
pub(crate) fn recipe_tab<const ONE: bool>(l: &Line, pos: usize, _: u16) -> R {
    if pos != 0 || l.at(0) != b'\t' {
        return None;
    }
    let e = prefix_end(l, 1);
    Some(if ONE {
        M::at(0, e).cap(1, 1, e)
    } else {
        M::at(0, e).cap(1, 0, 1).cap(2, 1, e)
    })
}

/// `^(?![ ]+)`
pub(crate) fn not_spaces(l: &Line, pos: usize, _: u16) -> R {
    (pos == 0 && l.at(0) != b' ').then(|| M::at(0, 0))
}

/// `^(?!\t)`
pub(crate) fn not_tab(l: &Line, pos: usize, _: u16) -> R {
    (pos == 0 && l.at(0) != b'\t').then(|| M::at(0, 0))
}

/// `^\n`
pub(crate) fn blank(l: &Line, pos: usize, _: u16) -> R {
    (pos == 0 && l.b == b"\n").then(|| M::at(0, 1))
}

/// `\s*(#)`
pub(crate) fn hash_ws(l: &Line, pos: usize, _: u16) -> R {
    let c = find(l.b, pos, b'#')?;
    Some(M::at(ws_back(l.b, pos, c), c + 1).cap(1, c, c + 1))
}

// ---- functions and variables ----

pub(crate) const CALL_NAMES: [&[&[u8]]; 6] = [
    &[b"call"],
    &[b"patsubst", b"filter"],
    &[b"wildcard"],
    &[b"info", b"warning", b"error"],
    &[
        b"subst",
        b"strip",
        b"findstring",
        b"filter-out",
        b"sort",
        b"word",
        b"wordlist",
        b"words",
        b"firstword",
        b"lastword",
        b"dir",
        b"notdir",
        b"suffix",
        b"basename",
        b"addsuffix",
        b"addprefix",
        b"join",
        b"realpath",
        b"abspath",
        b"if",
        b"or",
        b"and",
        b"foreach",
        b"file",
        b"value",
        b"eval",
        b"origin",
        b"flavor",
        b"guile",
    ],
    &[b"shell"],
];

/// `\\['"]|['"](?=\s*,)`
pub(crate) fn call_quote(l: &Line, pos: usize, _: u16) -> R {
    let b = l.b;
    for i in pos..b.len() {
        match b[i] {
            b'\\' if matches!(l.at(i + 1), b'\'' | b'"') => return Some(M::at(i, i + 2)),
            b'\'' | b'"' if l.at(ws_end(b, i + 1)) == b',' => return Some(M::at(i, i + 1)),
            _ => {}
        }
    }
    None
}

/// `\s*(WORD)\b`
pub(crate) fn ws_word(l: &Line, pos: usize, word: &[u8]) -> R {
    let mut at = pos;
    while let Some(k) = find_str(l.b, at, word) {
        let e = k + word.len();
        if !l.word_at(e) {
            return Some(M::at(ws_back(l.b, pos, k), e).cap(1, k, e));
        }
        at = k + 1;
    }
    None
}

/// `\s*(override)\b`
pub(crate) fn override_kw(l: &Line, pos: usize, _: u16) -> R {
    ws_word(l, pos, b"override")
}

/// `\s*(define)\b`
pub(crate) fn define_ws(l: &Line, pos: usize, _: u16) -> R {
    ws_word(l, pos, b"define")
}

/// `\bdefine\b`
pub(crate) fn define_word(l: &Line, pos: usize, _: u16) -> R {
    let mut at = pos;
    while let Some(k) = find_str(l.b, at, b"define") {
        if !l.word_before(k) && !l.word_at(k + 6) {
            return Some(M::at(k, k + 6));
        }
        at = k + 1;
    }
    None
}

/// Where `(\?|\+|::?)?=` (with `!` too when `BANG`) ending at the `=` at `f` starts, not
/// before `pos`.
pub(crate) fn op_start<const BANG: bool>(b: &[u8], pos: usize, f: usize) -> usize {
    if f >= pos + 2 && b[f - 2] == b':' && b[f - 1] == b':' {
        f - 2
    } else if f > pos && (matches!(b[f - 1], b'?' | b'+' | b':') || (BANG && b[f - 1] == b'!')) {
        f - 1
    } else {
        f
    }
}

/// `(\?|\+|::?)?=`, or `(!|\?|\+|::?)?=` when `BANG`
pub(crate) fn assign_op<const BANG: bool>(l: &Line, pos: usize, _: u16) -> R {
    let f = find(l.b, pos, b'=')?;
    Some(M::at(op_start::<BANG>(l.b, pos, f), f + 1))
}

/// `(?=\s*(!|\?|\+|::?)?=)`
pub(crate) fn assign_ahead(l: &Line, pos: usize, _: u16) -> R {
    let s = assign_op::<true>(l, pos, 0)?.s();
    let q = ws_back(l.b, pos, s);
    Some(M::at(q, q))
}

/// `(?=\s*!=)`
pub(crate) fn shell_assign_ahead(l: &Line, pos: usize, _: u16) -> R {
    let k = find_str(l.b, pos, b"!=")?;
    let q = ws_back(l.b, pos, k);
    Some(M::at(q, q))
}

/// `!=`
pub(crate) fn bang_eq(l: &Line, pos: usize, _: u16) -> R {
    find_str(l.b, pos, b"!=").map(|k| M::at(k, k + 2))
}

/// The line's last character but whitespace, when it is an `=`: the assignment that ends a
/// `define` line, `(!|\?|\+|::?)?=\s*\n`.
pub(crate) fn define_op(l: &Line, pos: usize) -> Option<(usize, usize)> {
    let t = (0..l.len()).rev().find(|&i| !ws(l.b[i]))?;
    (t >= pos && l.b[t] == b'=').then(|| (op_start::<true>(l.b, pos, t), t + 1))
}

/// `(?=(!|\?|\+|::?)?=\s*\n)`
pub(crate) fn define_op_ahead(l: &Line, pos: usize, _: u16) -> R {
    let (s, _) = define_op(l, pos)?;
    Some(M::at(s, s))
}

/// `((!|\?|\+|::?)?=)\s*\n`
pub(crate) fn define_assign(l: &Line, pos: usize, _: u16) -> R {
    let (s, e) = define_op(l, pos)?;
    Some(M::at(s, l.len()).cap(1, s, e))
}
