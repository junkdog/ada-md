//! bat's `Makefile.sublime-syntax` (sublimehq/Packages at bat 0.26.1's commit, with bat's
//! patch to `just_eat`), context for context. Each matcher's comment is the regex it stands
//! for.

use super::scan::{
    Line, M, R, eol_at, find, find_str, find2, non_ws, spaces_back, ws, ws_back, ws_end,
};
use super::shell;
use super::{Ctx, Find, It, Pat, Sc::*};
use It::{I, P};

const fn p(find: Find) -> Pat {
    Pat::new(find)
}

// ---- matchers shared with the shell ----

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
fn eol_nl(l: &Line, pos: usize, _: u16) -> R {
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
fn ws1(l: &Line, pos: usize, _: u16) -> R {
    (pos..l.len())
        .find(|&i| ws(l.b[i]))
        .map(|i| M::at(i, i + 1))
}

/// `\n`
fn nl(l: &Line, pos: usize, _: u16) -> R {
    byte::<b'\n'>(l, pos, 0)
}

/// `\s*`
fn eat_ws(l: &Line, pos: usize, _: u16) -> R {
    Some(M::at(pos, ws_end(l.b, pos)))
}

// ---- the line's start ----

/// `^\s*` then `kw`, at the start of the line: where `kw` starts and ends.
fn bol_kw(l: &Line, pos: usize, kw: &[u8]) -> Option<(usize, usize)> {
    if pos != 0 {
        return None;
    }
    let i = ws_end(l.b, 0);
    l.b[i..].starts_with(kw).then_some((i, i + kw.len()))
}

/// `^\s*(KW)\b`
fn bol_word(l: &Line, pos: usize, kw: &[u8]) -> R {
    let (i, e) = bol_kw(l, pos, kw)?;
    (!l.word_at(e)).then(|| M::at(0, e).cap(1, i, e))
}

/// `^\s*(endef)`
fn endef_stray(l: &Line, pos: usize, _: u16) -> R {
    let (i, e) = bol_kw(l, pos, b"endef")?;
    Some(M::at(0, e).cap(1, i, e))
}

/// `^\s*(endef)\b`
fn endef(l: &Line, pos: usize, _: u16) -> R {
    bol_word(l, pos, b"endef")
}

/// `^\s*define\b`
fn define_nested(l: &Line, pos: usize, _: u16) -> R {
    bol_word(l, pos, b"define")
}

/// `^\s*(else)\b`
fn else_kw(l: &Line, pos: usize, _: u16) -> R {
    bol_word(l, pos, b"else")
}

/// `^\s*(endif)\b`
fn endif_kw(l: &Line, pos: usize, _: u16) -> R {
    bol_word(l, pos, b"endif")
}

/// `^\s*(export)\b`
fn export_kw(l: &Line, pos: usize, _: u16) -> R {
    bol_word(l, pos, b"export")
}

/// `^\s*(vpath)\s`
fn vpath_ws(l: &Line, pos: usize, _: u16) -> R {
    let (i, e) = bol_kw(l, pos, b"vpath")?;
    ws(l.at(e)).then(|| M::at(0, e + 1).cap(1, i, e))
}

/// `^\s*(vpath)$`
fn vpath_eol(l: &Line, pos: usize, _: u16) -> R {
    let (_, e) = bol_kw(l, pos, b"vpath")?;
    (e == l.len() || l.at(e) == b'\n').then(|| M::at(0, e))
}

/// `^\s*([s-]?include)\s+`
fn include_kw(l: &Line, pos: usize, _: u16) -> R {
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
fn if_directive(l: &Line, i: usize) -> Option<usize> {
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
fn ifdef(l: &Line, pos: usize, _: u16) -> R {
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
fn if_tail(l: &Line, j: usize) -> Option<usize> {
    if l.b[j..].starts_with(b"def") {
        Some(j + 3)
    } else if l.b[j..].starts_with(b"eq") {
        Some(j + 2)
    } else {
        None
    }
}

/// `^\s*(ifn?(def|eq))`
fn recipe_if(l: &Line, pos: usize, _: u16) -> R {
    if pos != 0 {
        return None;
    }
    let i = ws_end(l.b, 0);
    let e = if_directive(l, i)?;
    Some(M::at(0, e).cap(1, i, e))
}

/// `(?=^\s*#)`
fn comment_ahead(l: &Line, pos: usize, _: u16) -> R {
    (pos == 0 && l.at(ws_end(l.b, 0)) == b'#').then(|| M::at(0, 0))
}

// ---- continuations and escapes ----

/// `(\\)([ ]*)$\n?`: only the last `\` before the trailing spaces can match.
fn line_cont(l: &Line, pos: usize, _: u16) -> R {
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
fn cont_end(l: &Line, pos: usize, _: u16) -> R {
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
fn next_paren(b: &[u8], q: usize) -> usize {
    find2(b, q, b'(', b')').unwrap_or(b.len())
}

/// A `:` not followed by `=` in `b[a..r]`.
fn colon_in(b: &[u8], a: usize, r: usize) -> bool {
    last_colon(b, a, r).is_some()
}

fn last_colon(b: &[u8], a: usize, r: usize) -> Option<usize> {
    (a..r)
        .rev()
        .find(|&c| b[c] == b':' && b.get(c + 1) != Some(&b'='))
}

fn first_eq(b: &[u8], a: usize, r: usize) -> Option<usize> {
    find(&b[..r], a, b'=')
}

/// The group opened at `r`, when `just_eat` takes it: where it closes, and where the text
/// after it runs to.
fn group_at(l: &Line, r: usize) -> Option<(usize, usize)> {
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
fn rule_ahead(l: &Line, pos: usize, _: u16) -> R {
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
fn var_ahead(l: &Line, pos: usize, _: u16) -> R {
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
fn colon_then_eq_ahead(l: &Line, pos: usize, _: u16) -> R {
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
fn colon_ahead(l: &Line, pos: usize, _: u16) -> R {
    let c = find(l.b, pos, b':')?;
    let q = spaces_back(l.b, pos, c);
    Some(M::at(q, q))
}

/// `::?` at `c`: where it ends.
fn colons_end(l: &Line, c: usize) -> usize {
    if l.at(c + 1) == b':' { c + 2 } else { c + 1 }
}

/// `(::?)\s*`
fn colons_ws(l: &Line, pos: usize, _: u16) -> R {
    let c = find(l.b, pos, b':')?;
    let e = colons_end(l, c);
    Some(M::at(c, ws_end(l.b, e)).cap(1, c, e))
}

/// `::?`
fn colons(l: &Line, pos: usize, _: u16) -> R {
    let c = find(l.b, pos, b':')?;
    Some(M::at(c, colons_end(l, c)))
}

/// `(?=\s*;)`
fn semicolon_ahead(l: &Line, pos: usize, _: u16) -> R {
    let c = find(l.b, pos, b';')?;
    let q = ws_back(l.b, pos, c);
    Some(M::at(q, q))
}

// ---- recipes ----

/// `^(?=[ ]+([-@]{1,2})?)`
fn spaces_ahead(l: &Line, pos: usize, _: u16) -> R {
    (pos == 0 && l.at(0) == b' ').then(|| M::at(0, 0))
}

/// `^(?=\t([-@]{1,2})?)`
fn tab_ahead(l: &Line, pos: usize, _: u16) -> R {
    (pos == 0 && l.at(0) == b'\t').then(|| M::at(0, 0))
}

/// `([-@]{1,2})?` from `i`: where it ends.
fn prefix_end(l: &Line, i: usize) -> usize {
    let mut e = i;
    while e < i + 2 && matches!(l.at(e), b'-' | b'@') {
        e += 1;
    }
    e
}

/// `^([ ]+)([-@]{1,2})?`
fn recipe_spaces(l: &Line, pos: usize, _: u16) -> R {
    if pos != 0 || l.at(0) != b' ' {
        return None;
    }
    let s = (0..l.len()).find(|&i| l.b[i] != b' ').unwrap_or(l.len());
    let e = prefix_end(l, s);
    Some(M::at(0, e).cap(1, 0, s).cap(2, s, e))
}

/// `^(\t)([-@]{1,2})?`; group 1 is the prefix when `ONE` is false, `^\t([-@]{1,2})?`
fn recipe_tab<const ONE: bool>(l: &Line, pos: usize, _: u16) -> R {
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
fn not_spaces(l: &Line, pos: usize, _: u16) -> R {
    (pos == 0 && l.at(0) != b' ').then(|| M::at(0, 0))
}

/// `^(?!\t)`
fn not_tab(l: &Line, pos: usize, _: u16) -> R {
    (pos == 0 && l.at(0) != b'\t').then(|| M::at(0, 0))
}

/// `^\n`
fn blank(l: &Line, pos: usize, _: u16) -> R {
    (pos == 0 && l.b == b"\n").then(|| M::at(0, 1))
}

/// `\s*(#)`
fn hash_ws(l: &Line, pos: usize, _: u16) -> R {
    let c = find(l.b, pos, b'#')?;
    Some(M::at(ws_back(l.b, pos, c), c + 1).cap(1, c, c + 1))
}

// ---- functions and variables ----

const CALL_NAMES: [&[&[u8]]; 6] = [
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

/// `(\$\$?\()(NAME)\s`, for the names of group `g` of [`CALL_NAMES`].
fn call(l: &Line, pos: usize, g: u16) -> R {
    let b = l.b;
    let mut at = pos;
    while let Some(i) = find(b, at, b'$') {
        let o = if l.at(i + 1) == b'$' { i + 2 } else { i + 1 };
        if l.at(o) == b'(' {
            let name = o + 1;
            for word in CALL_NAMES[usize::from(g)] {
                let e = name + word.len();
                if b[name..].starts_with(word) && ws(l.at(e)) {
                    return Some(M::at(i, e + 1).cap(1, i, name).cap(2, name, e));
                }
            }
        }
        at = i + 1;
    }
    None
}

/// `\\['"]|['"](?=\s*,)`
fn call_quote(l: &Line, pos: usize, _: u16) -> R {
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

/// `\$\$?` then a byte `pred` takes.
fn dollar(l: &Line, pos: usize, pred: fn(u8) -> bool) -> R {
    let mut at = pos;
    while let Some(i) = find(l.b, at, b'$') {
        if l.at(i + 1) == b'$' && pred(l.at(i + 2)) {
            return Some(M::at(i, i + 3));
        }
        if pred(l.at(i + 1)) {
            return Some(M::at(i, i + 2));
        }
        at = i + 1;
    }
    None
}

/// `\$\$?\(`
fn dollar_paren(l: &Line, pos: usize, _: u16) -> R {
    dollar(l, pos, |c| c == b'(')
}

/// `\$\$?{`
fn dollar_brace(l: &Line, pos: usize, _: u16) -> R {
    dollar(l, pos, |c| c == b'{')
}

/// `\$\$?[@%<?^+|*]`
fn automatic(l: &Line, pos: usize, _: u16) -> R {
    dollar(l, pos, |c| b"@%<?^+|*".contains(&c))
}

/// `\$\$`
fn dollars(l: &Line, pos: usize, _: u16) -> R {
    find_str(l.b, pos, b"$$").map(|i| M::at(i, i + 2))
}

/// `(\$)[[:alpha:]]`
fn single_char(l: &Line, pos: usize, _: u16) -> R {
    let mut at = pos;
    while let Some(i) = find(l.b, at, b'$') {
        if let Some(n) = l.alpha_at(i + 1) {
            return Some(M::at(i, i + 1 + n).cap(1, i, i + 1));
        }
        at = i + 1;
    }
    None
}

/// `\s*(WORD)\b`
fn ws_word(l: &Line, pos: usize, word: &[u8]) -> R {
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
fn override_kw(l: &Line, pos: usize, _: u16) -> R {
    ws_word(l, pos, b"override")
}

/// `\s*(define)\b`
fn define_ws(l: &Line, pos: usize, _: u16) -> R {
    ws_word(l, pos, b"define")
}

/// `\bdefine\b`
fn define_word(l: &Line, pos: usize, _: u16) -> R {
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
fn op_start<const BANG: bool>(b: &[u8], pos: usize, f: usize) -> usize {
    if f >= pos + 2 && b[f - 2] == b':' && b[f - 1] == b':' {
        f - 2
    } else if f > pos && (matches!(b[f - 1], b'?' | b'+' | b':') || (BANG && b[f - 1] == b'!')) {
        f - 1
    } else {
        f
    }
}

/// `(\?|\+|::?)?=`, or `(!|\?|\+|::?)?=` when `BANG`
fn assign_op<const BANG: bool>(l: &Line, pos: usize, _: u16) -> R {
    let f = find(l.b, pos, b'=')?;
    Some(M::at(op_start::<BANG>(l.b, pos, f), f + 1))
}

/// `(?=\s*(!|\?|\+|::?)?=)`
fn assign_ahead(l: &Line, pos: usize, _: u16) -> R {
    let s = assign_op::<true>(l, pos, 0)?.s();
    let q = ws_back(l.b, pos, s);
    Some(M::at(q, q))
}

/// `(?=\s*!=)`
fn shell_assign_ahead(l: &Line, pos: usize, _: u16) -> R {
    let k = find_str(l.b, pos, b"!=")?;
    let q = ws_back(l.b, pos, k);
    Some(M::at(q, q))
}

/// `!=`
fn bang_eq(l: &Line, pos: usize, _: u16) -> R {
    find_str(l.b, pos, b"!=").map(|k| M::at(k, k + 2))
}

/// `((?:bash|sh|zsh)\s+-c\s+)(['"`])`
fn shell_c(l: &Line, pos: usize, _: u16) -> R {
    let b = l.b;
    for i in pos..b.len() {
        let Some(word) = [&b"bash"[..], b"sh", b"zsh"]
            .into_iter()
            .find(|w| b[i..].starts_with(w))
        else {
            continue;
        };
        let j = i + word.len();
        let k = ws_end(b, j);
        if k == j || !b[k..].starts_with(b"-c") {
            continue;
        }
        let q = ws_end(b, k + 2);
        if q > k + 2 && matches!(l.at(q), b'\'' | b'"' | b'`') {
            return Some(M::at(i, q + 1).cap(1, i, q).cap(2, q, q + 1));
        }
    }
    None
}

/// `(?!<\\)\2`: the quote the script started with.
fn quote_back(l: &Line, pos: usize, _: u16) -> R {
    let &q = l.cap.borrow().first()?;
    find(l.b, pos, q).map(|i| M::at(i, i + 1))
}

/// `(?=(?!<\\)\2)`
fn quote_back_ahead(l: &Line, pos: usize, _: u16) -> R {
    quote_back(l, pos, 0).map(|m| M::at(m.s(), m.s()))
}

/// The line's last character but whitespace, when it is an `=`: the assignment that ends a
/// `define` line, `(!|\?|\+|::?)?=\s*\n`.
fn define_op(l: &Line, pos: usize) -> Option<(usize, usize)> {
    let t = (0..l.len()).rev().find(|&i| !ws(l.b[i]))?;
    (t >= pos && l.b[t] == b'=').then(|| (op_start::<true>(l.b, pos, t), t + 1))
}

/// `(?=(!|\?|\+|::?)?=\s*\n)`
fn define_op_ahead(l: &Line, pos: usize, _: u16) -> R {
    let (s, _) = define_op(l, pos)?;
    Some(M::at(s, s))
}

/// `((!|\?|\+|::?)?=)\s*\n`
fn define_assign(l: &Line, pos: usize, _: u16) -> R {
    let (s, e) = define_op(l, pos)?;
    Some(M::at(s, l.len()).cap(1, s, e))
}

// ---- contexts ----

pub static START: Ctx = Ctx::new(&[P(&p(empty).push(&[&MAIN]))]);

static MAIN: Ctx = Ctx::new(&[
    I(&COMMENTS),
    I(&VARIABLE_DEFINITIONS),
    P(&p(rule_ahead).push(&[&EXPECT_RULE])),
    I(&VARIABLE_SUBSTITUTIONS),
    I(&CONTROL_FLOW),
    P(&p(endef_stray).caps(&[(1, &[StrayEndef])])),
]);

static INSIDE_CONTROL_FLOW: Ctx = Ctx::new(&[
    P(&p(byte::<b'\''>).sc(&[StrBegin]).push(&[&STRING_SINGLE])),
    P(&p(byte::<b'"'>).sc(&[StrBegin]).push(&[&STRING_DOUBLE])),
    P(&p(byte::<b'('>).sc(&[GroupBegin]).push(&[&CONTROL_GROUP])),
    P(&p(byte::<b')'>).sc(&[StrayParen])),
    I(&CONTINUATION_OR_POP),
])
.meta(&[MetaGroup]);

static STRING_SINGLE: Ctx = Ctx::new(&[
    P(&p(byte::<b'\''>).sc(&[StrEnd]).pop()),
    I(&ESCAPE_LITERALS),
    I(&LINE_CONTINUATION),
    I(&VARIABLE_SUBSTITUTIONS),
])
.meta(&[MetaString, StrSingle]);

static STRING_DOUBLE: Ctx = Ctx::new(&[
    P(&p(byte::<b'"'>).sc(&[StrEnd]).pop()),
    I(&ESCAPE_LITERALS),
    I(&LINE_CONTINUATION),
    I(&VARIABLE_SUBSTITUTIONS),
])
.meta(&[MetaString, StrDouble]);

static CONTROL_GROUP: Ctx = Ctx::new(&[
    P(&p(byte::<b')'>).sc(&[GroupEnd]).pop()),
    I(&VARIABLE_SUBSTITUTIONS),
    P(&p(byte::<b','>).sc(&[Separator])),
]);

static CONTROL_FLOW: Ctx = Ctx::new(&[
    P(&p(vpath_ws).caps(&[(1, &[Vpath])]).push(&[&VPATH])),
    P(&p(vpath_eol)),
    P(&p(include_kw).caps(&[(1, &[Import])]).push(&[&INCLUDE])),
    P(&p(ifdef).sc(&[Conditional]).push(&[&INSIDE_CONTROL_FLOW])),
    P(&p(else_kw).caps(&[(1, &[Conditional])]).push(&[&ELSE])),
    P(&p(endif_kw).caps(&[(1, &[Conditional])]).push(&[&ENDIF])),
]);

static VPATH: Ctx = Ctx::new(&[
    I(&HIGHLIGHT_WILDCARD),
    P(&p(ws1).set(&[&VPATH_DIRS])),
    I(&POP_ON_LINE_END),
]);

static VPATH_DIRS: Ctx = Ctx::new(&[I(&POP_ON_LINE_END)]).content(&[MetaString, Unquoted]);

static INCLUDE: Ctx = Ctx::new(&[
    I(&CONTINUATION_OR_POP),
    I(&VARIABLE_SUBSTITUTIONS),
    I(&COMMENTS),
])
.content(&[MetaString, Unquoted]);

static ELSE: Ctx = Ctx::new(&[I(&CONTROL_FLOW), I(&COMMENTS), I(&POP_ON_LINE_END)]);

static ENDIF: Ctx = Ctx::new(&[I(&COMMENTS), I(&POP_ON_LINE_END)]);

static HIGHLIGHT_PERCENT: Ctx = Ctx::new(&[P(&p(byte::<b'%'>).sc(&[Percent]))]);

static HIGHLIGHT_WILDCARD: Ctx = Ctx::new(&[P(&p(byte::<b'*'>).sc(&[Wildcard]))]);

static EXPECT_RULE: Ctx = Ctx::new(&[
    I(&LINE_CONTINUATION),
    I(&VARIABLE_SUBSTITUTIONS),
    I(&HIGHLIGHT_PERCENT),
    P(&p(colon_then_eq_ahead).set(&[&TARGET_VARIABLE])),
    P(&p(colon_ahead).set(&[&RULE_COLON])),
])
.meta(&[MetaFunction, EntityFunction]);

static TARGET_VARIABLE: Ctx =
    Ctx::new(&[P(&p(colons).sc(&[Assign]).set(&[&TARGET_VARIABLE_VALUE]))]);

static TARGET_VARIABLE_VALUE: Ctx = Ctx::new(&[
    I(&VARIABLE_DEFINITIONS),
    I(&VARIABLE_SUBSTITUTIONS),
    I(&CONTINUATION_OR_POP),
]);

static RULE_COLON: Ctx = Ctx::new(&[P(&p(colons_ws)
    .caps(&[(1, &[Assign])])
    .set(&[&PREREQUISITES]))]);

static PREREQUISITES: Ctx = Ctx::new(&[
    I(&LINE_CONTINUATION),
    I(&VARIABLE_SUBSTITUTIONS),
    I(&HIGHLIGHT_PERCENT),
    P(&p(ahead::<b'#'>).set(&[&RULE_COMMENT_START])),
    P(&p(eol).set(&[&RECIPE_JUNCTION])),
    P(&p(semicolon_ahead).set(&[&RULE_SEMICOLON])),
])
.content(&[FnArgs, Unquoted]);

static RULE_COMMENT_START: Ctx =
    Ctx::new(&[P(&p(byte::<b'#'>).sc(&[CommentMark]).set(&[&RULE_COMMENT]))]);

static RULE_COMMENT: Ctx = Ctx::new(&[P(&p(eol).set(&[&RECIPE_JUNCTION]))]).meta(&[Comment]);

static RULE_SEMICOLON: Ctx =
    Ctx::new(&[P(&p(byte::<b';'>).sc(&[Terminator]).set(&[&RECIPE_INLINE]))]);

static RECIPE_JUNCTION: Ctx = Ctx::new(&[
    I(&COMMENTS),
    P(&p(recipe_if)
        .caps(&[(1, &[Conditional])])
        .push(&[&RECIPE_IF])),
    P(&p(spaces_ahead).set(&[&RECIPE_WITH_SPACES])),
    P(&p(tab_ahead).set(&[&RECIPE_WITH_TABS])),
    P(&p(bol).pop()),
])
.content(&[FnBody]);

static RECIPE_IF: Ctx = Ctx::new(&[I(&INSIDE_CONTROL_FLOW), I(&RECIPE_JUNCTION)]);

static RECIPE_INLINE: Ctx = Ctx::new(&[
    I(&RECIPE_JUNCTION),
    I(&CONTROL_FLOW),
    P(&p(eol).set(&[&RECIPE_JUNCTION])),
    P(&p(empty).push(&[&SHELL])),
])
.content(&[FnBody]);

static RECIPE_WITH_SPACES: Ctx = Ctx::new(&[
    P(&p(recipe_spaces).caps(&[(2, &[ConstLang])]).push(&[&SHELL])),
    P(&p(recipe_tab::<false>).caps(&[(1, &[ExpectedSpaces]), (2, &[ConstLang])])),
    I(&RECIPE_COMMON),
    P(&p(not_spaces).pop()),
])
.content(&[FnBody]);

static RECIPE_WITH_TABS: Ctx = Ctx::new(&[
    P(&p(recipe_tab::<true>)
        .caps(&[(1, &[ConstLang])])
        .push(&[&SHELL])),
    P(&p(recipe_spaces).caps(&[(1, &[ExpectedTab]), (2, &[ConstLang])])),
    I(&RECIPE_COMMON),
    P(&p(not_tab).pop()),
])
.content(&[FnBody]);

static RECIPE_COMMON: Ctx = Ctx::new(&[I(&CONTROL_FLOW), P(&p(blank))]);

static SHELL: Ctx = Ctx::new(&[
    P(&p(hash_ws)
        .caps(&[(1, &[ShCommentMark])])
        .set(&[&SHELL_COMMENT])),
    P(&p(empty).set(&[&shell::SOURCE]).with(&RECIPE_PROTOTYPE)),
]);

static SHELL_COMMENT: Ctx = Ctx::new(&[I(&COMMENTS_POP_ON_LINE_END)]).meta(&[Source, ShComment]);

static RECIPE_PROTOTYPE: Ctx = Ctx::new(&[
    I(&POP_ON_LINE_END),
    I(&shell::PROTOTYPE),
    I(&LINE_CONTINUATION),
    I(&VARIABLE_SUBSTITUTIONS),
]);

static LINE_CONTINUATION: Ctx = Ctx::new(&[P(&p(line_cont)
    .caps(&[(1, &[Continuation])])
    .push(&[&CONTINUED]))]);

static CONTINUED: Ctx = Ctx::new(&[P(&p(cont_end).pop())]);

pub static POP_ON_LINE_END: Ctx = Ctx::new(&[P(&p(eol).pop())]);

static CONTINUATION_OR_POP: Ctx = Ctx::new(&[I(&LINE_CONTINUATION), I(&POP_ON_LINE_END)]);

static QUOTED_STRING: Ctx = Ctx::new(&[
    P(&p(byte::<b'\''>).sc(&[StrBegin]).push(&[&STRING_SINGLE])),
    P(&p(byte::<b'"'>).sc(&[StrBegin]).push(&[&STRING_DOUBLE])),
]);

static ESCAPE_LITERALS: Ctx = Ctx::new(&[P(&p(escape_any).sc(&[Escape]))]);

static COMMENTS_POP_ON_LINE_END: Ctx = Ctx::new(&[I(&LINE_CONTINUATION), P(&p(eol_nl).pop())]);

static COMMENTS: Ctx = Ctx::new(&[
    P(&p(comment_ahead).push(&[&COMMENT_START])),
    P(&p(byte::<b'#'>).sc(&[CommentMark]).push(&[&COMMENT])),
]);

static COMMENT_START: Ctx = Ctx::new(&[P(&p(byte::<b'#'>).sc(&[CommentMark]).set(&[&COMMENT]))]);

static COMMENT: Ctx = Ctx::new(&[I(&COMMENTS_POP_ON_LINE_END)]).meta(&[Comment]);

static INSIDE_FUNCTION_CALL: Ctx = Ctx::new(&[
    P(&p(byte::<b')'>).sc(&[BlockEnd]).pop()),
    P(&p(byte::<b'('>).push(&[&PAREN_BALANCER])),
    P(&p(byte::<b','>).sc(&[Separator])),
    P(&p(call_quote)),
    I(&VARIABLE_SUBSTITUTIONS),
    I(&QUOTED_STRING),
])
.content(&[CallArgs]);

static FUNCTION_INVOCATIONS: Ctx = Ctx::new(&[
    P(&p(call)
        .arg(0)
        .caps(&[(0, &[Call]), (1, &[BlockBegin]), (2, &[CallLang])])
        .push(&[&CALL])),
    P(&p(call)
        .arg(1)
        .caps(&[(0, &[Call]), (1, &[BlockBegin]), (2, &[Builtin])])
        .push(&[&CALL_PATSUBST])),
    P(&p(call)
        .arg(2)
        .caps(&[(0, &[Call]), (1, &[BlockBegin]), (2, &[Builtin])])
        .push(&[&CALL_WILDCARD])),
    P(&p(call)
        .arg(3)
        .caps(&[(0, &[Call]), (1, &[BlockBegin]), (2, &[Builtin])])
        .push(&[&CALL_INFO])),
    P(&p(call)
        .arg(4)
        .caps(&[(0, &[Call]), (1, &[BlockBegin]), (2, &[Builtin])])
        .push(&[&INSIDE_FUNCTION_CALL])),
    P(&p(call)
        .arg(5)
        .caps(&[(1, &[BlockBegin]), (2, &[Builtin])])
        .push(&[&CALL_SHELL])),
]);

static CALL: Ctx = Ctx::new(&[
    P(&p(ahead::<b')'>).set(&[&INSIDE_FUNCTION_CALL])),
    P(&p(ahead::<b','>).set(&[&CALL_COMMA])),
    I(&VARIABLE_SUBSTITUTIONS),
])
.content(&[Call, VarFunction]);

static CALL_COMMA: Ctx = Ctx::new(&[P(&p(byte::<b','>)
    .sc(&[Separator])
    .set(&[&INSIDE_FUNCTION_CALL]))])
.content(&[Call]);

static CALL_PATSUBST: Ctx =
    Ctx::new(&[I(&HIGHLIGHT_PERCENT), I(&INSIDE_FUNCTION_CALL)]).content(&[CallArgs]);

static CALL_WILDCARD: Ctx =
    Ctx::new(&[I(&INSIDE_FUNCTION_CALL), I(&HIGHLIGHT_WILDCARD)]).content(&[CallArgs]);

static CALL_INFO: Ctx = Ctx::new(&[
    P(&p(byte::<b')'>).sc(&[BlockEnd]).pop()),
    P(&p(byte::<b'('>).push(&[&PAREN_BALANCER])),
    P(&p(byte::<b','>).sc(&[Separator])),
    I(&VARIABLE_SUBSTITUTIONS),
])
.content(&[CallArgs]);

static CALL_SHELL: Ctx = Ctx::new(&[
    P(&p(byte::<b')'>).sc(&[BlockEnd]).pop()),
    P(&p(empty).push(&[&shell::SOURCE]).with(&CALL_SHELL_PROTOTYPE)),
]);

static CALL_SHELL_PROTOTYPE: Ctx = Ctx::new(&[
    P(&p(ahead::<b')'>).pop()),
    I(&VARIABLE_SUBSTITUTIONS),
    I(&QUOTED_STRING),
    P(&p(byte::<b'('>).push(&[&PAREN_BALANCER])),
]);

static VARIABLE_DEFINITIONS: Ctx = Ctx::new(&[
    P(&p(override_kw).caps(&[(1, &[Control])]).set(&[&OVERRIDE])),
    P(&p(define_ws).caps(&[(1, &[Control])]).push(&[&DEFINE])),
    P(&p(export_kw).caps(&[(1, &[Control])]).push(&[&EXPORT])),
    P(&p(var_ahead).push(&[&VARIABLE])),
]);

static OVERRIDE: Ctx = Ctx::new(&[
    P(&p(define_word).sc(&[Control]).push(&[&DEFINE])),
    I(&VARIABLE_DEFINITIONS),
    I(&CONTINUATION_OR_POP),
]);

static EXPORT: Ctx = Ctx::new(&[
    I(&CONTINUATION_OR_POP),
    I(&VARIABLE_SUBSTITUTIONS),
    P(&p(assign_op::<false>)
        .sc(&[Assign])
        .set(&[&VALUE, &EAT_WHITESPACE])),
])
.content(&[VarOther]);

static VARIABLE: Ctx = Ctx::new(&[
    P(&p(shell_assign_ahead).set(&[&SHELL_ASSIGN])),
    P(&p(assign_ahead).set(&[&ASSIGN])),
    I(&VARIABLE_SUBSTITUTIONS),
])
.content(&[VarOther]);

static SHELL_ASSIGN: Ctx = Ctx::new(&[P(&p(bang_eq)
    .sc(&[Assign])
    .set(&[&shell::SOURCE])
    .with(&SHELL_ASSIGN_PROTOTYPE))]);

static SHELL_ASSIGN_PROTOTYPE: Ctx = Ctx::new(&[
    I(&VARIABLE_SUBSTITUTIONS),
    I(&POP_ON_LINE_END),
    I(&PAREN_BALANCER),
]);

static ASSIGN: Ctx = Ctx::new(&[
    P(&p(assign_op::<true>)
        .sc(&[Assign])
        .set(&[&VALUE_MAYBE_SHELL, &EAT_WHITESPACE])),
    I(&VARIABLE_SUBSTITUTIONS),
    I(&CONTINUATION_OR_POP),
]);

static PAREN_BALANCER: Ctx = Ctx::new(&[P(&p(byte::<b')'>).pop()), I(&VARIABLE_SUBSTITUTIONS)]);

static EAT_WHITESPACE: Ctx = Ctx::new(&[P(&p(eat_ws).pop())]).clear(1);

static VALUE_MAYBE_SHELL: Ctx = Ctx::new(&[
    P(&p(shell_c)
        .caps(&[
            (1, &[MetaString, Unquoted]),
            (2, &[MetaString, Interpolation, InterpBegin]),
        ])
        .push(&[&EMBEDDED_SCRIPT, &shell::SOURCE])
        .with(&EMBEDDED_SCRIPT_END)),
    P(&p(empty).set(&[&VALUE])),
]);

/// What `embed` turns into: a context that ends at the escape, under the embedded syntax.
static EMBEDDED_SCRIPT: Ctx = Ctx::new(&[P(&p(quote_back)
    .caps(&[(0, &[MetaString, Interpolation, InterpEnd])])
    .pop()
    .nocache())])
.content(&[MetaString, Interpolation])
.backref(2);

static EMBEDDED_SCRIPT_END: Ctx = Ctx::new(&[P(&p(quote_back_ahead).pop().nocache())]).backref(2);

static VALUE: Ctx = Ctx::new(&[
    I(&ESCAPE_LITERALS),
    P(&p(ahead::<b'#'>).set(&[&VALUE_COMMENT_START])),
    I(&VARIABLE_SUBSTITUTIONS),
    I(&CONTINUATION_OR_POP),
])
.content(&[MetaString, Unquoted]);

static VALUE_COMMENT_START: Ctx =
    Ctx::new(&[P(&p(byte::<b'#'>).sc(&[CommentMark]).set(&[&COMMENT]))]);

static DEFINE: Ctx = Ctx::new(&[
    P(&p(define_op_ahead).set(&[&DEFINE_ASSIGN])),
    P(&p(nl).set(&[&DEFINE_VALUE])),
    I(&VARIABLE_SUBSTITUTIONS),
])
.content(&[VarOther]);

static DEFINE_ASSIGN: Ctx = Ctx::new(&[P(&p(define_assign)
    .caps(&[(1, &[Assign])])
    .set(&[&DEFINE_VALUE]))]);

static DEFINE_VALUE: Ctx = Ctx::new(&[
    P(&p(endef).caps(&[(1, &[Control])]).pop()),
    P(&p(define_nested).push(&[&DEFINE_NESTED])),
])
.content(&[MetaString, Unquoted]);

static DEFINE_NESTED: Ctx = Ctx::new(&[P(&p(endef).pop())]);

static SUBSTITUTION_COMMON: Ctx = Ctx::new(&[
    P(&p(byte::<b':'>).sc(&[SubstMark])),
    P(&p(byte::<b'='>).sc(&[AssignMark])),
    I(&HIGHLIGHT_PERCENT),
    I(&VARIABLE_SUBSTITUTIONS),
]);

pub static VARIABLE_SUBSTITUTIONS: Ctx = Ctx::new(&[
    I(&FUNCTION_INVOCATIONS),
    P(&p(dollar_paren)
        .sc(&[BlockBegin])
        .push(&[&SUBSTITUTION_PAREN])),
    P(&p(dollar_brace)
        .sc(&[BlockBegin])
        .push(&[&SUBSTITUTION_BRACE])),
    P(&p(automatic).sc(&[Automatic])),
    P(&p(dollars).sc(&[Escape])),
    P(&p(single_char).caps(&[(0, &[VarParam]), (1, &[SingleChar])])),
]);

static SUBSTITUTION_PAREN: Ctx = Ctx::new(&[
    P(&p(byte::<b')'>).sc(&[BlockEnd]).pop()),
    I(&SUBSTITUTION_COMMON),
])
.meta(&[VarParam]);

static SUBSTITUTION_BRACE: Ctx = Ctx::new(&[
    P(&p(byte::<b'}'>).sc(&[BlockEnd]).pop()),
    I(&SUBSTITUTION_COMMON),
])
.meta(&[VarParam]);
