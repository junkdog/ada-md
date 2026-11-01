//! bat's shell grammar (`Bash.sublime-syntax` and its generated `commands-builtin-shell-bash`),
//! context for context: on its own from [`START`] (G-003), and as Makefile recipes, `!=` and
//! `$(shell …)` embed it, as `scope:source.shell` through `Shell-Unix-Generic` ([`SOURCE`]).
//! In a Makefile, make takes `$(`, `${` and `$x` before the shell sees them.

use super::grammar::{ahead, bol, byte, empty, eol, escape_any};
use super::scan::{Line, M, R, eol_at, find, find_str, non_ws, ws, ws_back, ws_end};
use super::{Ctx, Find, It, Pat, Sc, Sc::*};
use It::{I, P};

const fn p(find: Find) -> Pat {
    Pat::new(find)
}

/// A builtin pattern's argument is this plus its index in [`BUILTINS`]: the arguments it
/// pushes read which builtin they belong to.
pub const BUILTIN_ARG: u16 = 0x100;

// ---- matchers ----

/// Whether `c` is a `metachar`: `[\s\t\n|&;()<>]`.
fn metachar(c: u8) -> bool {
    ws(c) || b"|&;()<>".contains(&c)
}

/// `(?=\S)`
pub fn non_ws_ahead(l: &Line, pos: usize, _: u16) -> R {
    non_ws(l.b, pos).map(|i| M::at(i, i))
}

/// `(?=\s)`
fn ws_ahead(l: &Line, pos: usize, _: u16) -> R {
    (pos..l.len()).find(|&i| ws(l.b[i])).map(|i| M::at(i, i))
}

/// `(?:^\s*|\s+)(\#)`
fn comment(l: &Line, pos: usize, _: u16) -> R {
    let b = l.b;
    let mut at = pos;
    while let Some(c) = find(b, at, b'#') {
        if pos == 0 && ws_end(b, 0) >= c {
            return Some(M::at(0, c + 1).cap(1, c, c + 1));
        }
        let s = ws_back(b, pos, c);
        if s < c {
            return Some(M::at(s, c + 1).cap(1, c, c + 1));
        }
        at = c + 1;
    }
    None
}

/// `\\\n`
fn backslash_nl(l: &Line, pos: usize, _: u16) -> R {
    let n = l.len();
    (n >= 2 && n - 2 >= pos && l.b[n - 2] == b'\\' && l.b[n - 1] == b'\n').then(|| M::at(n - 2, n))
}

/// `\\(\s+)\n`
fn backslash_ws_nl(l: &Line, pos: usize, _: u16) -> R {
    let n = l.len();
    if n == 0 || l.b[n - 1] != b'\n' {
        return None;
    }
    let k = ws_back(l.b, 0, n - 1);
    (k > pos && k < n - 1 && l.b[k - 1] == b'\\').then(|| M::at(k - 1, n).cap(1, k, n - 1))
}

/// `\n`
fn nl(l: &Line, pos: usize, _: u16) -> R {
    find(l.b, pos, b'\n').map(|i| M::at(i, i + 1))
}

/// Whether `c` breaks a keyword: `(?![-=\w])` holds before it.
fn keyword_break(l: &Line, i: usize) -> bool {
    !matches!(l.at(i), b'-' | b'=') && !l.word_at(i)
}

const KEYWORDS: [&[u8]; 17] = [
    b"if",
    b"then",
    b"elif",
    b"fi",
    b"else",
    b"for",
    b"do",
    b"done",
    b"while",
    b"until",
    b"case",
    b"continue",
    b"break",
    b"esac",
    b"in",
    b"let",
    b"coproc",
];

/// `\bKEYWORD{{keyword_break}}`, the keyword [`KEYWORDS`]`[k]`
fn keyword(l: &Line, pos: usize, k: u16) -> R {
    let word = KEYWORDS[usize::from(k)];
    let mut at = pos;
    while let Some(i) = find_str(l.b, at, word) {
        let e = i + word.len();
        if !l.word_before(i) && keyword_break(l, e) {
            return Some(M::at(i, e));
        }
        at = i + 1;
    }
    None
}

/// `(?=\besac{{keyword_break}})`
fn esac_ahead(l: &Line, pos: usize, _: u16) -> R {
    keyword(l, pos, 13).map(|m| M::at(m.s(), m.s()))
}

/// `\blet\b`
fn let_kw(l: &Line, pos: usize, _: u16) -> R {
    let mut at = pos;
    while let Some(i) = find_str(l.b, at, b"let") {
        if !l.word_before(i) && !l.word_at(i + 3) {
            return Some(M::at(i, i + 3));
        }
        at = i + 1;
    }
    None
}

/// `;;&?|;&`
fn case_end(l: &Line, pos: usize, _: u16) -> R {
    let i = find_str(l.b, pos, b";;")
        .into_iter()
        .chain(find_str(l.b, pos, b";&"))
        .min()?;
    let e = if l.b[i + 1] == b';' && l.at(i + 2) == b'&' {
        i + 3
    } else {
        i + 2
    };
    Some(M::at(i, e))
}

/// `(?=;;&?|;&)`
fn case_end_ahead(l: &Line, pos: usize, _: u16) -> R {
    case_end(l, pos, 0).map(|m| M::at(m.s(), m.s()))
}

/// `;(?![;&])`
fn semicolon(l: &Line, pos: usize, _: u16) -> R {
    let mut at = pos;
    while let Some(i) = find(l.b, at, b';') {
        if !matches!(l.at(i + 1), b';' | b'&') {
            return Some(M::at(i, i + 1));
        }
        at = i + 1;
    }
    None
}

/// A literal: `\|\|`, `&&`, `\)\)`, …
fn lit(l: &Line, pos: usize, s: &[u8]) -> R {
    find_str(l.b, pos, s).map(|i| M::at(i, i + s.len()))
}

fn or_or(l: &Line, pos: usize, _: u16) -> R {
    lit(l, pos, b"||")
}

fn and_and(l: &Line, pos: usize, _: u16) -> R {
    lit(l, pos, b"&&")
}

fn close_close(l: &Line, pos: usize, _: u16) -> R {
    lit(l, pos, b"))")
}

/// `$|(?=\S)`
fn eol_or_non_ws(l: &Line, pos: usize, _: u16) -> R {
    let e = eol_at(l, pos);
    let at = non_ws(l.b, pos).map_or(e, |i| i.min(e));
    Some(M::at(at, at))
}

/// `$|(?=\`)`
fn eol_or_backtick(l: &Line, pos: usize, _: u16) -> R {
    let e = eol_at(l, pos);
    let at = find(l.b, pos, b'`').map_or(e, |i| i.min(e));
    Some(M::at(at, at))
}

/// `(?=[SET])` for the bytes of `set`.
fn ahead_any(l: &Line, pos: usize, set: &[u8]) -> R {
    (pos..l.len())
        .find(|&i| set.contains(&l.b[i]))
        .map(|i| M::at(i, i))
}

/// `(?=\)|\`)`
fn close_or_backtick_ahead(l: &Line, pos: usize, _: u16) -> R {
    ahead_any(l, pos, b")`")
}

/// `(?=\)|})`
fn close_or_brace_ahead(l: &Line, pos: usize, _: u16) -> R {
    ahead_any(l, pos, b")}")
}

/// `(?=[)};&|])`
fn end_of_options_end(l: &Line, pos: usize, _: u16) -> R {
    ahead_any(l, pos, b")};&|")
}

/// `(?={{metachar}})`
fn metachar_ahead(l: &Line, pos: usize, _: u16) -> R {
    (pos..l.len())
        .find(|&i| metachar(l.b[i]))
        .map(|i| M::at(i, i))
}

/// `(?={{metachar}}|\`)`
fn metachar_or_backtick_ahead(l: &Line, pos: usize, _: u16) -> R {
    (pos..l.len())
        .find(|&i| metachar(l.b[i]) || l.b[i] == b'`')
        .map(|i| M::at(i, i))
}

/// `(?=[&\`])`
fn amp_or_backtick_ahead(l: &Line, pos: usize, _: u16) -> R {
    ahead_any(l, pos, b"&`")
}

/// Where `\s+` before `i` starts, when there is some and it starts at or after `pos`.
fn ws_before(l: &Line, pos: usize, i: usize) -> Option<usize> {
    let s = ws_back(l.b, pos, i);
    (s < i).then_some(s)
}

/// `(?=}|\s+#)`
fn brace_or_comment_ahead(l: &Line, pos: usize, _: u16) -> R {
    let brace = find(l.b, pos, b'}');
    let mut at = pos;
    let mut hash = None;
    while let Some(c) = find(l.b, at, b'#') {
        if let Some(s) = ws_before(l, pos, c) {
            hash = Some(s);
            break;
        }
        at = c + 1;
    }
    let i = brace.into_iter().chain(hash).min()?;
    Some(M::at(i, i))
}

/// Where the first `[|;]` or `&(?!>)` at or after `pos` is.
fn separator(l: &Line, pos: usize) -> Option<usize> {
    (pos..l.len()).find(|&i| match l.b[i] {
        b'|' | b';' => true,
        b'&' => l.at(i + 1) != b'>',
        _ => false,
    })
}

/// `(?=\s*([|;]|&(?!>)))`
fn separator_ahead(l: &Line, pos: usize, _: u16) -> R {
    let c = separator(l, pos)?;
    let s = ws_back(l.b, pos, c);
    Some(M::at(s, s))
}

/// `(?=}|\s+#|\s*(?:[|;]|&(?!>)))`
fn name_common_end(l: &Line, pos: usize, _: u16) -> R {
    let a = brace_or_comment_ahead(l, pos, 0).map(|m| m.s());
    let b = separator_ahead(l, pos, 0).map(|m| m.s());
    let i = a.into_iter().chain(b).min()?;
    Some(M::at(i, i))
}

/// `(?={{is_start_of_arguments}}|{{is_end_of_interpolation}})`
fn name_end(l: &Line, pos: usize, _: u16) -> R {
    (pos..l.len())
        .find(|&i| ws(l.b[i]) || b"`=|&;()<>".contains(&l.b[i]))
        .map(|i| M::at(i, i))
}

/// `(?:\s+|^)` before `k`: where the match starts.
fn ws_or_bol(l: &Line, pos: usize, k: usize) -> Option<usize> {
    if k == 0 && pos == 0 {
        return Some(0);
    }
    ws_before(l, pos, k)
}

/// `(?:\s+|^)--(?=\s|$)`
fn end_of_options(l: &Line, pos: usize, _: u16) -> R {
    let mut at = pos;
    while let Some(k) = find_str(l.b, at, b"--") {
        let after = l.at(k + 2);
        if (k + 2 == l.len() || ws(after))
            && let Some(s) = ws_or_bol(l, pos, k)
        {
            return Some(M::at(s, k + 2));
        }
        at = k + 1;
    }
    None
}

/// `{{start_of_option}}`: `(?:\s+|^)--?(?=[\w$])`
fn option_start(l: &Line, pos: usize, _: u16) -> R {
    let opens = |i: usize| l.word_at(i) || l.at(i) == b'$';
    let mut at = pos;
    while let Some(k) = find(l.b, at, b'-') {
        if let Some(s) = ws_or_bol(l, pos, k) {
            if l.at(k + 1) == b'-' && opens(k + 2) {
                return Some(M::at(s, k + 2));
            }
            if opens(k + 1) {
                return Some(M::at(s, k + 1));
            }
        }
        at = k + 1;
    }
    None
}

/// `(?={{is_end_of_option}})`: `(?=[^\w$-]|$)`, and a backtick when `BT`
fn option_end<const BT: bool>(l: &Line, pos: usize, _: u16) -> R {
    let e = eol_at(l, pos);
    let mut i = pos;
    // a character at a time: a word character can be several bytes
    while i < e {
        let c = l.b[i];
        if (BT && c == b'`') || !(l.word_at(i) || matches!(c, b'$' | b'-')) {
            break;
        }
        i += l.char_len(i);
    }
    Some(M::at(i.min(e), i.min(e)))
}

/// `(BRACKET)(?=\s)`
fn bracket_ws(l: &Line, pos: usize, s: &[u8]) -> R {
    let mut at = pos;
    while let Some(i) = find_str(l.b, at, s) {
        let e = i + s.len();
        if ws(l.at(e)) {
            return Some(M::at(i, e).cap(1, i, e));
        }
        at = i + 1;
    }
    None
}

/// `(\[\[)(?=\s)`
fn double_bracket(l: &Line, pos: usize, _: u16) -> R {
    bracket_ws(l, pos, b"[[")
}

/// `(\[)(?=\s)`
fn bracket(l: &Line, pos: usize, _: u16) -> R {
    bracket_ws(l, pos, b"[")
}

/// `(\{)(?=\s)`
fn brace_open(l: &Line, pos: usize, _: u16) -> R {
    bracket_ws(l, pos, b"{")
}

/// `\s+(BRACKET)`
fn ws_bracket(l: &Line, pos: usize, s: &[u8]) -> R {
    let mut at = pos;
    while let Some(i) = find_str(l.b, at, s) {
        if let Some(start) = ws_before(l, pos, i) {
            return Some(M::at(start, i + s.len()).cap(1, i, i + s.len()));
        }
        at = i + 1;
    }
    None
}

/// `\s+(\])`
fn test_end(l: &Line, pos: usize, _: u16) -> R {
    ws_bracket(l, pos, b"]")
}

/// `\s+(\]\])`
fn double_test_end(l: &Line, pos: usize, _: u16) -> R {
    ws_bracket(l, pos, b"]]")
}

/// Whether `))` follows `from` on the line: `(?=.+\)\))` after a `((` ending at `from`.
fn closes_twice(l: &Line, from: usize) -> bool {
    find_str(l.b, from + 1, b"))").is_some()
}

/// `\(\((?=.+\)\))`
fn arithmetic_open(l: &Line, pos: usize, _: u16) -> R {
    let i = find_str(l.b, pos, b"((")?;
    closes_twice(l, i + 2).then(|| M::at(i, i + 2))
}

/// `(\$)(\(\()(?=.+\)\))`
fn dollar_arithmetic(l: &Line, pos: usize, _: u16) -> R {
    let mut at = pos;
    while let Some(i) = find_str(l.b, at, b"$((") {
        if closes_twice(l, i + 3) {
            return Some(M::at(i, i + 3).cap(1, i, i + 1).cap(2, i + 1, i + 3));
        }
        at = i + 1;
    }
    None
}

/// `(\$)(\()`
fn dollar_paren(l: &Line, pos: usize, _: u16) -> R {
    find_str(l.b, pos, b"$(").map(|i| M::at(i, i + 2).cap(1, i, i + 1).cap(2, i + 1, i + 2))
}

/// `\s*(\))`
fn ws_close(l: &Line, pos: usize, _: u16) -> R {
    let c = find(l.b, pos, b')')?;
    Some(M::at(ws_back(l.b, pos, c), c + 1).cap(1, c, c + 1))
}

/// `(\$)(\{)`
fn dollar_brace(l: &Line, pos: usize, _: u16) -> R {
    find_str(l.b, pos, b"${").map(|i| M::at(i, i + 2).cap(1, i, i + 1).cap(2, i + 1, i + 2))
}

/// `(\$)(X)` where `x` takes the byte after the `$` and says how long `X` is.
fn dollar_then(l: &Line, pos: usize, x: impl Fn(&Line, usize) -> Option<usize>) -> R {
    let mut at = pos;
    while let Some(i) = find(l.b, at, b'$') {
        if let Some(n) = x(l, i + 1) {
            return Some(
                M::at(i, i + 1 + n)
                    .cap(1, i, i + 1)
                    .cap(2, i + 1, i + 1 + n),
            );
        }
        at = i + 1;
    }
    None
}

/// `(\$)(\d)`
fn dollar_digit(l: &Line, pos: usize, _: u16) -> R {
    dollar_then(l, pos, |l, i| l.at(i).is_ascii_digit().then_some(1))
}

/// `(\$)([$#@!~*?_-])(?!\w)`
fn dollar_special(l: &Line, pos: usize, _: u16) -> R {
    dollar_then(l, pos, |l, i| {
        (b"$#@!~*?_-".contains(&l.at(i)) && !l.word_at(i + 1)).then_some(1)
    })
}

/// `{{identifier}}` at `i`: `[[:alpha:]_][[:alnum:]_]*`, how long.
fn identifier(l: &Line, i: usize) -> Option<usize> {
    let first = if l.at(i) == b'_' {
        Some(1)
    } else {
        l.alpha_at(i)
    }?;
    let mut e = i + first;
    while l.word_at(e) {
        e += l.char_len(e);
    }
    Some(e - i)
}

/// `(\$)({{identifier}})`
fn dollar_identifier(l: &Line, pos: usize, _: u16) -> R {
    dollar_then(l, pos, identifier)
}

/// `(?=\:?[-+=?])`
fn parameter_op_ahead(l: &Line, pos: usize, _: u16) -> R {
    parameter_op(l, pos, 0).map(|m| M::at(m.s(), m.s()))
}

/// `\:?[-+=?]`
fn parameter_op(l: &Line, pos: usize, _: u16) -> R {
    let i = (pos..l.len()).find(|&i| b"-+=?".contains(&l.b[i]))?;
    let s = if i > pos && l.b[i - 1] == b':' {
        i - 1
    } else {
        i
    };
    Some(M::at(s, i + 1))
}

/// `\#(?=})`
fn hash_before_brace(l: &Line, pos: usize, _: u16) -> R {
    find_str(l.b, pos, b"#}").map(|i| M::at(i, i + 1))
}

/// `([@*])?(\#\#?|%%?|\^\^?|,,?)`
fn parameter_expansion_op(l: &Line, pos: usize, _: u16) -> R {
    let i = (pos..l.len()).find(|&i| b"#%^,".contains(&l.b[i]))?;
    let c = l.b[i];
    let e = if l.at(i + 1) == c { i + 2 } else { i + 1 };
    let m = if i > pos && matches!(l.b[i - 1], b'@' | b'*') {
        M::at(i - 1, e).cap(1, i - 1, i)
    } else {
        M::at(i, e)
    };
    Some(m.cap(2, i, e))
}

/// `([@*]?)(@)([QEPAa])(?=})`
fn parameter_transform(l: &Line, pos: usize, _: u16) -> R {
    let mut at = pos;
    while let Some(i) = find(l.b, at, b'@') {
        if b"QEPAa".contains(&l.at(i + 1)) && l.at(i + 2) == b'}' {
            let s = if i > pos && matches!(l.b[i - 1], b'@' | b'*') {
                i - 1
            } else {
                i
            };
            return Some(
                M::at(s, i + 2)
                    .cap(1, s, i)
                    .cap(2, i, i + 1)
                    .cap(3, i + 1, i + 2),
            );
        }
        at = i + 1;
    }
    None
}

/// `[*@](?=})`
fn star_at_before_brace(l: &Line, pos: usize, _: u16) -> R {
    (pos..l.len())
        .find(|&i| matches!(l.b[i], b'*' | b'@') && l.at(i + 1) == b'}')
        .map(|i| M::at(i, i + 1))
}

/// `[*@]`
fn star_at(l: &Line, pos: usize, _: u16) -> R {
    (pos..l.len())
        .find(|&i| matches!(l.b[i], b'*' | b'@'))
        .map(|i| M::at(i, i + 1))
}

/// `([?*+@!])(\()`
fn extglob(l: &Line, pos: usize, _: u16) -> R {
    let mut at = pos;
    while let Some(i) = find(l.b, at, b'(') {
        if i > pos && b"?*+@!".contains(&l.b[i - 1]) {
            return Some(M::at(i - 1, i + 1).cap(1, i - 1, i).cap(2, i, i + 1));
        }
        at = i + 1;
    }
    None
}

/// `[*?]`
fn glob(l: &Line, pos: usize, _: u16) -> R {
    (pos..l.len())
        .find(|&i| matches!(l.b[i], b'*' | b'?'))
        .map(|i| M::at(i, i + 1))
}

/// `\[(?=.*])`
fn set_open(l: &Line, pos: usize, _: u16) -> R {
    let i = find(l.b, pos, b'[')?;
    find(l.b, i + 1, b']').map(|_| M::at(i, i + 1))
}

/// `[!^]`
fn set_not(l: &Line, pos: usize, _: u16) -> R {
    (pos..l.len())
        .find(|&i| matches!(l.b[i], b'!' | b'^'))
        .map(|i| M::at(i, i + 1))
}

/// `(?:-)?(\])`
fn set_close(l: &Line, pos: usize, _: u16) -> R {
    let c = find(l.b, pos, b']')?;
    let s = if c > pos && l.b[c - 1] == b'-' {
        c - 1
    } else {
        c
    };
    Some(M::at(s, c + 1).cap(1, c, c + 1))
}

/// `(D)[[:word:]](D)`: a collating symbol `.x.` or an equivalence class `=x=` in a set.
fn delimited_word<const D: u8>(l: &Line, pos: usize, _: u16) -> R {
    let mut at = pos;
    while let Some(i) = find(l.b, at, D) {
        if l.word_at(i + 1) {
            let e = i + 1 + l.char_len(i + 1);
            if l.at(e) == D {
                return Some(M::at(i, e + 1).cap(1, i, i + 1).cap(2, e, e + 1));
            }
        }
        at = i + 1;
    }
    None
}

/// `(:)[[:lower:]]+(:)`
fn char_class(l: &Line, pos: usize, _: u16) -> R {
    let mut at = pos;
    while let Some(i) = find(l.b, at, b':') {
        let mut e = i + 1;
        while l.at(e).is_ascii_lowercase() {
            e += 1;
        }
        if e > i + 1 && l.at(e) == b':' {
            return Some(M::at(i, e + 1).cap(1, i, i + 1).cap(2, e, e + 1));
        }
        at = i + 1;
    }
    None
}

/// `\[(?![\.=:])`
fn bracket_plain(l: &Line, pos: usize, _: u16) -> R {
    let mut at = pos;
    while let Some(i) = find(l.b, at, b'[') {
        if !matches!(l.at(i + 1), b'.' | b'=' | b':') {
            return Some(M::at(i, i + 1));
        }
        at = i + 1;
    }
    None
}

/// `(%)([%+-])`, `(%)(\d+)`, `(%)(\??)(\w+)` (by `form`), or a lone `%`
fn job(l: &Line, pos: usize, form: u16) -> R {
    let mut at = pos;
    while let Some(i) = find(l.b, at, b'%') {
        let j = i + 1;
        let m = M::at(i, j).cap(1, i, j);
        match form {
            0 if matches!(l.at(j), b'%' | b'+' | b'-') => {
                return Some(M::at(i, j + 1).cap(1, i, j).cap(2, j, j + 1));
            }
            1 if l.at(j).is_ascii_digit() => {
                let e = (j..l.len())
                    .find(|&k| !l.b[k].is_ascii_digit())
                    .unwrap_or(l.len());
                return Some(M::at(i, e).cap(1, i, j).cap(2, j, e));
            }
            2 => {
                let q = if l.at(j) == b'?' { j + 1 } else { j };
                let mut e = q;
                while l.word_at(e) {
                    e += l.char_len(e);
                }
                if e > q {
                    return Some(M::at(i, e).cap(1, i, j).cap(2, j, q).cap(3, q, e));
                }
            }
            3 => return Some(m),
            _ => {}
        }
        at = j;
    }
    None
}

/// `\\[$\`"\\]`
fn dq_escape(l: &Line, pos: usize, _: u16) -> R {
    let mut at = pos;
    while let Some(i) = find(l.b, at, b'\\') {
        if matches!(l.at(i + 1), b'$' | b'`' | b'"' | b'\\') {
            return Some(M::at(i, i + 2));
        }
        at = i + 1;
    }
    None
}

/// `\\([abfnrtv'"?]|[0-8]{1,3}|x\h{1,8}|c[a-z])`
fn ansi_escape(l: &Line, pos: usize, _: u16) -> R {
    let mut at = pos;
    while let Some(i) = find(l.b, at, b'\\') {
        let j = i + 1;
        let run = |from: usize, max: usize, f: fn(&u8) -> bool| {
            let mut e = from;
            while e < from + max && e < l.len() && f(&l.b[e]) {
                e += 1;
            }
            e
        };
        let e = match l.at(j) {
            c if b"abfnrtv'\"?".contains(&c) && c != 0 => Some(j + 1),
            b'0'..=b'8' => Some(run(j, 3, |c| (b'0'..=b'8').contains(c))),
            b'x' => Some(run(j + 1, 8, u8::is_ascii_hexdigit)).filter(|&e| e > j + 1),
            b'c' if l.at(j + 1).is_ascii_lowercase() => Some(j + 2),
            _ => None,
        };
        if let Some(e) = e {
            return Some(M::at(i, e).cap(1, j, e));
        }
        at = j;
    }
    None
}

/// `\$'`
fn dollar_quote(l: &Line, pos: usize, _: u16) -> R {
    lit(l, pos, b"$'")
}

/// `\$"`
fn dollar_dquote(l: &Line, pos: usize, _: u16) -> R {
    lit(l, pos, b"$\"")
}

// expressions

/// `\b` before `i`, which starts a word character.
fn starts_word(l: &Line, i: usize) -> bool {
    !l.word_before(i)
}

/// `\b0[xX]`
fn hex_start(l: &Line, pos: usize, _: u16) -> R {
    (pos..l.len())
        .find(|&i| l.b[i] == b'0' && matches!(l.at(i + 1), b'x' | b'X') && starts_word(l, i))
        .map(|i| M::at(i, i + 2))
}

/// `\b0(?=[0-7])`
fn octal_start(l: &Line, pos: usize, _: u16) -> R {
    (pos..l.len())
        .find(|&i| l.b[i] == b'0' && (b'0'..=b'7').contains(&l.at(i + 1)) && starts_word(l, i))
        .map(|i| M::at(i, i + 1))
}

/// `\b(\d+#)[a-zA-Z0-9@_]+`
fn based(l: &Line, pos: usize, _: u16) -> R {
    for i in pos..l.len() {
        if !l.b[i].is_ascii_digit() || !starts_word(l, i) {
            continue;
        }
        let d = (i..l.len())
            .find(|&k| !l.b[k].is_ascii_digit())
            .unwrap_or(l.len());
        if l.at(d) != b'#' {
            continue;
        }
        let e = (d + 1..l.len())
            .find(|&k| !(l.b[k].is_ascii_alphanumeric() || matches!(l.b[k], b'@' | b'_')))
            .unwrap_or(l.len());
        if e > d + 1 {
            return Some(M::at(i, e).cap(1, i, d + 1));
        }
    }
    None
}

/// `\b\d+`
fn decimal(l: &Line, pos: usize, _: u16) -> R {
    let i = (pos..l.len()).find(|&i| l.b[i].is_ascii_digit() && starts_word(l, i))?;
    let e = (i..l.len())
        .find(|&k| !l.b[k].is_ascii_digit())
        .unwrap_or(l.len());
    Some(M::at(i, e))
}

/// The first `i` at or after `pos` where `at` finds a match, and its end.
fn first(l: &Line, pos: usize, at: impl Fn(&Line, usize) -> Option<usize>) -> R {
    (pos..l.len()).find_map(|i| at(l, i).map(|e| M::at(i, e)))
}

/// `[*/%+\-&^|]?=|<<=|>>=`
fn expr_assign(l: &Line, pos: usize, _: u16) -> R {
    first(l, pos, |l, i| match l.b[i] {
        b'=' => Some(i + 1),
        b'*' | b'/' | b'%' | b'+' | b'-' | b'&' | b'^' | b'|' if l.at(i + 1) == b'=' => Some(i + 2),
        b'<' | b'>' if l.at(i + 1) == l.b[i] && l.at(i + 2) == b'=' => Some(i + 3),
        _ => None,
    })
}

/// `\+\+?|\-\-?|\*\*?|%|/`
fn expr_arith(l: &Line, pos: usize, _: u16) -> R {
    first(l, pos, |l, i| match l.b[i] {
        c @ (b'+' | b'-' | b'*') => Some(if l.at(i + 1) == c { i + 2 } else { i + 1 }),
        b'%' | b'/' => Some(i + 1),
        _ => None,
    })
}

/// `<[=<]?|>[=>]?|[=!]=|&&|\:|\|\||!`
fn expr_logical(l: &Line, pos: usize, _: u16) -> R {
    first(l, pos, |l, i| {
        let next = l.at(i + 1);
        match l.b[i] {
            b'<' => Some(if matches!(next, b'=' | b'<') {
                i + 2
            } else {
                i + 1
            }),
            b'>' => Some(if matches!(next, b'=' | b'>') {
                i + 2
            } else {
                i + 1
            }),
            b'=' if next == b'=' => Some(i + 2),
            b'!' if next == b'=' => Some(i + 2),
            b'&' if next == b'&' => Some(i + 2),
            b'|' if next == b'|' => Some(i + 2),
            b':' | b'!' => Some(i + 1),
            _ => None,
        }
    })
}

/// A byte of `set`: `[&|^~]`, `[,;]`, …
fn any_of(l: &Line, pos: usize, set: &[u8]) -> R {
    (pos..l.len())
        .find(|&i| set.contains(&l.b[i]))
        .map(|i| M::at(i, i + 1))
}

fn bitwise(l: &Line, pos: usize, _: u16) -> R {
    any_of(l, pos, b"&|^~")
}

fn comma_semicolon(l: &Line, pos: usize, _: u16) -> R {
    any_of(l, pos, b",;")
}

fn not_hex_letter(l: &Line, pos: usize, _: u16) -> R {
    (pos..l.len())
        .find(|&i| matches!(l.b[i], b'g'..=b'z' | b'G'..=b'Z'))
        .map(|i| M::at(i, i + 1))
}

/// `(?=\H)`
fn not_hex_ahead(l: &Line, pos: usize, _: u16) -> R {
    (pos..l.len())
        .find(|&i| !l.b[i].is_ascii_hexdigit())
        .map(|i| M::at(i, i))
}

fn eight_nine(l: &Line, pos: usize, _: u16) -> R {
    any_of(l, pos, b"89")
}

/// `(?=[^0-7])`
fn not_octal_ahead(l: &Line, pos: usize, _: u16) -> R {
    (pos..l.len())
        .find(|&i| !(b'0'..=b'7').contains(&l.b[i]))
        .map(|i| M::at(i, i))
}

/// `((-)[aobcdefghknoprstuvwxzGLNORS])(?=\s)`
fn test_unary(l: &Line, pos: usize, _: u16) -> R {
    first_dash(l, pos, |l, k| {
        (b"aobcdefghknoprstuvwxzGLNORS".contains(&l.at(k + 1)) && ws(l.at(k + 2))).then_some(k + 2)
    })
}

/// `((-)(?:ef|nt|ot|eq|ne|lt|le|gt|ge))(?=\s)`
fn test_binary(l: &Line, pos: usize, _: u16) -> R {
    first_dash(l, pos, |l, k| {
        let ops: [&[u8]; 9] = [
            b"ef", b"nt", b"ot", b"eq", b"ne", b"lt", b"le", b"gt", b"ge",
        ];
        (ops.iter().any(|o| l.b[k + 1..].starts_with(o)) && ws(l.at(k + 3))).then_some(k + 3)
    })
}

/// A `-` at or after `pos` where `rest` ends an option, as `((-)…)`.
fn first_dash(l: &Line, pos: usize, rest: impl Fn(&Line, usize) -> Option<usize>) -> R {
    let mut at = pos;
    while let Some(k) = find(l.b, at, b'-') {
        if let Some(e) = rest(l, k) {
            return Some(M::at(k, e).cap(1, k, e).cap(2, k, k + 1));
        }
        at = k + 1;
    }
    None
}

/// `(=~)\s*`
fn regex_match(l: &Line, pos: usize, _: u16) -> R {
    let i = find_str(l.b, pos, b"=~")?;
    Some(M::at(i, ws_end(l.b, i + 2)).cap(1, i, i + 2))
}

/// `==?|!=?|<|>|\|\||&&`
fn test_logical(l: &Line, pos: usize, _: u16) -> R {
    first(l, pos, |l, i| {
        let next = l.at(i + 1);
        match l.b[i] {
            b'=' | b'!' => Some(if next == b'=' { i + 2 } else { i + 1 }),
            b'<' | b'>' => Some(i + 1),
            b'|' if next == b'|' => Some(i + 2),
            b'&' if next == b'&' => Some(i + 2),
            _ => None,
        }
    })
}

// variables

/// Whether `(?:[({]{{nbc}}[)}])?{{nbc}}=` follows a run of `{{nbc}}` that ends at `i`.
fn variable_tail(l: &Line, mut i: usize) -> bool {
    let nbc = |c: u8| !(b"{}()=".contains(&c) || ws(c));
    let run = |mut i: usize| {
        while i < l.len() && nbc(l.b[i]) {
            i += 1;
        }
        i
    };
    if matches!(l.at(i), b'(' | b'{') {
        let j = run(i + 1);
        if matches!(l.at(j), b')' | b'}') {
            i = run(j + 1);
        }
    }
    l.at(i) == b'='
}

/// `{{is_variable}}`: `(?=\s*{{nbc}}(?:[({]{{nbc}}[)}])?{{nbc}}=)`. From any start in a run of
/// `{{nbc}}`, or the whitespace before it, the run ends at the same place: each run is tried
/// once. The contexts that try it push on any `\S` too, but at the stack's depth limit that
/// push is refused while a match found before is still taken: so the leftmost match counts.
fn is_variable(l: &Line, pos: usize, _: u16) -> R {
    let nbc = |c: u8| !(b"{}()=".contains(&c) || ws(c));
    let mut q = pos;
    while q < l.len() {
        let p = ws_end(l.b, q);
        if p >= l.len() {
            return None;
        }
        let mut end = p;
        while end < l.len() && nbc(l.b[end]) {
            end += 1;
        }
        if variable_tail(l, end) {
            return Some(M::at(q, q));
        }
        q = if end > p { end } else { p + 1 };
    }
    None
}

const MODIFIERS: [&[&[u8]]; 4] = [
    &[b"alias"],
    &[b"typeset", b"declare", b"local"],
    &[b"export"],
    &[b"readonly"],
];

/// `\s*\b(WORD){{keyword_break}}` for the words of [`MODIFIERS`]`[k]`
fn modifier(l: &Line, pos: usize, k: u16) -> R {
    MODIFIERS[usize::from(k)]
        .iter()
        .filter_map(|word| {
            let mut at = pos;
            while let Some(i) = find_str(l.b, at, word) {
                let e = i + word.len();
                if !l.word_before(i) && keyword_break(l, e) {
                    return Some(M::at(ws_back(l.b, pos, i), e).cap(1, i, e));
                }
                at = i + 1;
            }
            None
        })
        .min_by_key(M::s)
}

const MODIFIER_OPTIONS: [(&[u8], u8); 4] = [
    (b"aAf", b'p'),
    (b"fn", b'p'),
    (b"aAfFgilnrtux", b'p'),
    (b"", b'p'),
];

/// `\s*((-)(?:[SET]+|p))` for [`MODIFIER_OPTIONS`]`[k]`
fn modifier_option(l: &Line, pos: usize, k: u16) -> R {
    let (set, one) = MODIFIER_OPTIONS[usize::from(k)];
    let mut at = pos;
    while let Some(d) = find(l.b, at, b'-') {
        let mut e = d + 1;
        while e < l.len() && set.contains(&l.b[e]) {
            e += 1;
        }
        if e == d + 1 && l.at(e) == one {
            e += 1;
        }
        if e > d + 1 {
            return Some(M::at(ws_back(l.b, pos, d), e).cap(1, d, e).cap(2, d, d + 1));
        }
        at = d + 1;
    }
    None
}

/// `(?=\s*#)`
fn comment_ahead(l: &Line, pos: usize, _: u16) -> R {
    let c = find(l.b, pos, b'#')?;
    let s = ws_back(l.b, pos, c);
    Some(M::at(s, s))
}

/// `\s*`
fn spaces(l: &Line, pos: usize, _: u16) -> R {
    Some(M::at(pos, ws_end(l.b, pos)))
}

/// `\s*$`: whitespace to the line's end, its `\n` included.
fn ws_eol(l: &Line, pos: usize, _: u16) -> R {
    let s = ws_back(l.b, pos, l.len());
    Some(M::at(s, l.len()))
}

/// `[+\-?]?=` at `i`: where it ends.
fn varassign_at(l: &Line, i: usize) -> Option<usize> {
    match l.at(i) {
        b'=' => Some(i + 1),
        b'+' | b'-' | b'?' if l.at(i + 1) == b'=' => Some(i + 2),
        _ => None,
    }
}

/// `{{varassign}}`
fn varassign(l: &Line, pos: usize, _: u16) -> R {
    first(l, pos, varassign_at)
}

/// The end of a name: `(?={{varassign}}|\s)|$`, and `(?=[;&\`]|{{metachar}})` when `VAR`.
fn name_stop<const VAR: bool>(l: &Line, pos: usize, _: u16) -> R {
    let e = eol_at(l, pos);
    let at = (pos..e)
        .find(|&i| {
            let c = l.b[i];
            varassign_at(l, i).is_some() || ws(c) || (VAR && (metachar(c) || b";&`".contains(&c)))
        })
        .unwrap_or(e);
    Some(M::at(at, at))
}

// redirections

/// `(\d*)` before an operator at `k`: where the match starts.
fn digits_back(l: &Line, pos: usize, k: usize) -> usize {
    let mut s = k;
    while s > pos && l.b[s - 1].is_ascii_digit() {
        s -= 1;
    }
    s
}

/// The first operator at or after `pos` that `op` finds at its index, with `(\d*)` before.
fn redirection(l: &Line, pos: usize, op: impl Fn(&Line, usize) -> Option<usize>) -> R {
    let k = (pos..l.len()).find(|&k| op(l, k).is_some())?;
    let e = op(l, k)?;
    let s = digits_back(l, pos, k);
    Some(M::at(s, e).cap(1, s, k).cap(2, k, e))
}

/// `(\d*)(<<<)\s`
fn here_string(l: &Line, pos: usize, _: u16) -> R {
    let m = redirection(l, pos, |l, k| {
        (l.b[k..].starts_with(b"<<<") && ws(l.at(k + 3))).then_some(k + 3)
    })?;
    Some(m.end(m.e() + 1))
}

/// `(\d*)([<>])(\()`
fn process_substitution(l: &Line, pos: usize, _: u16) -> R {
    let m = redirection(l, pos, |l, k| {
        (matches!(l.b[k], b'<' | b'>') && l.at(k + 1) == b'(').then_some(k + 1)
    })?;
    let e = m.e();
    Some(m.end(e + 1).cap(3, e, e + 1))
}

/// `(\d*)(>>!?|>&?|&>|&?>(?:\||>))`
fn redirect_output(l: &Line, pos: usize, _: u16) -> R {
    redirection(l, pos, |l, k| match l.b[k] {
        b'>' if l.at(k + 1) == b'>' => Some(if l.at(k + 2) == b'!' { k + 3 } else { k + 2 }),
        b'>' => Some(if l.at(k + 1) == b'&' { k + 2 } else { k + 1 }),
        b'&' if l.at(k + 1) == b'>' => Some(k + 2),
        _ => None,
    })
}

/// `(\d*)(<&?)`
fn redirect_input(l: &Line, pos: usize, _: u16) -> R {
    redirection(l, pos, |l, k| {
        (l.b[k] == b'<').then(|| if l.at(k + 1) == b'&' { k + 2 } else { k + 1 })
    })
}

/// `(\d*)(<>)`
fn redirect_inout(l: &Line, pos: usize, _: u16) -> R {
    redirection(l, pos, |l, k| l.b[k..].starts_with(b"<>").then_some(k + 2))
}

/// `\s*(?:(\d+)|(-))`
fn redirect_fd(l: &Line, pos: usize, _: u16) -> R {
    let c = (pos..l.len()).find(|&i| l.b[i].is_ascii_digit() || l.b[i] == b'-')?;
    let s = ws_back(l.b, pos, c);
    if l.b[c] == b'-' {
        return Some(M::at(s, c + 1).cap(2, c, c + 1));
    }
    let e = (c..l.len())
        .find(|&k| !l.b[k].is_ascii_digit())
        .unwrap_or(l.len());
    Some(M::at(s, e).cap(1, c, e))
}

/// `\s*(?=\S)`
fn ws_then_non_ws(l: &Line, pos: usize, _: u16) -> R {
    let c = non_ws(l.b, pos)?;
    Some(M::at(ws_back(l.b, pos, c), c))
}

/// `\!(?!\S)`
fn bang_alone(l: &Line, pos: usize, _: u16) -> R {
    let mut at = pos;
    while let Some(i) = find(l.b, at, b'!') {
        if i + 1 >= l.len() || ws(l.b[i + 1]) {
            return Some(M::at(i, i + 1));
        }
        at = i + 1;
    }
    None
}

/// `(\!)(-?\d+|!)`
fn history(l: &Line, pos: usize, _: u16) -> R {
    let mut at = pos;
    while let Some(i) = find(l.b, at, b'!') {
        let j = i + 1;
        if l.at(j) == b'!' {
            return Some(M::at(i, j + 1).cap(1, i, j));
        }
        let d = if l.at(j) == b'-' { j + 1 } else { j };
        let e = (d..l.len())
            .find(|&k| !l.b[k].is_ascii_digit())
            .unwrap_or(l.len());
        if e > d {
            return Some(M::at(i, e).cap(1, i, j));
        }
        at = j;
    }
    None
}

// builtins

/// A builtin: its name, its scope, whether it takes `--`, and its options as
/// `((PREFIX)(?:[ONE]|[MANY]+)|[BARE]+)`.
struct Builtin {
    name: &'static [u8],
    eoo: bool,
    prefix: &'static [u8],
    one: &'static [u8],
    many: &'static [u8],
    bare: &'static [u8],
}

const fn b(name: &'static [u8], eoo: bool) -> Builtin {
    Builtin {
        name,
        eoo,
        prefix: b"-",
        one: b"",
        many: b"",
        bare: b"",
    }
}

const fn opts(
    mut builtin: Builtin,
    one: &'static [u8],
    many: &'static [u8],
    bare: &'static [u8],
) -> Builtin {
    builtin.one = one;
    builtin.many = many;
    builtin.bare = bare;
    builtin
}

/// In the grammar's order, with `set`'s `+` prefix.
static BUILTINS: [Builtin; 40] = [
    Builtin {
        name: b"set",
        eoo: true,
        prefix: b"-+",
        one: b"o",
        many: b"abefhkmnptuvxBCEHPT",
        bare: b"",
    },
    opts(b(b"help", true), b"", b"dms", b""),
    opts(b(b"mapfile", true), b"dnOstuCc", b"", b""),
    b(b"getopts", true),
    opts(b(b"cd", true), b"LPe@", b"", b""),
    opts(b(b"suspend", true), b"f", b"", b""),
    b(b":", false),
    opts(b(b"printf", true), b"v", b"", b""),
    opts(b(b"hash", true), b"rpl", b"", b"dt"),
    opts(b(b"ulimit", true), b"", b"HSabcdefiklmnpqrstuvxPT", b""),
    opts(b(b"umask", true), b"pS", b"", b""),
    b(b"source", true),
    b(b"builtin", true),
    b(b"exit", false),
    opts(b(b"type", true), b"", b"afptP", b""),
    b(b"bg", true),
    b(b"return", false),
    opts(b(b"exec", true), b"a", b"", b"cl"),
    opts(b(b"read", true), b"adeinNprstu", b"", b"ers"),
    opts(b(b"enable", true), b"af", b"", b"dnps"),
    opts(b(b"pwd", true), b"", b"LP", b""),
    b(b"logout", true),
    b(b"eval", false),
    b(b"fg", true),
    opts(b(b"disown", true), b"h", b"", b"ar"),
    opts(b(b"echo", true), b"", b"neE", b""),
    b(b".", false),
    opts(b(b"wait", true), b"n", b"", b""),
    opts(b(b"jobs", true), b"x", b"", b"lnprs"),
    opts(b(b"bind", true), b"mqurfx", b"", b"lpsvPSVX"),
    b(b"caller", true),
    b(b"times", true),
    opts(b(b"unalias", true), b"a", b"", b""),
    opts(b(b"command", true), b"", b"pVv", b""),
    opts(b(b"trap", true), b"", b"lp", b""),
    b(b"shift", false),
    opts(b(b"kill", true), b"snlL", b"", b""),
    opts(b(b"readarray", true), b"adeinNprstu", b"", b"ers"),
    opts(b(b"shopt", true), b"o", b"", b"pqsu"),
    opts(b(b"unset", true), b"", b"fnv", b""),
];

/// `{{boundary_end}}`: `(?=\s|;|$|>|<|\|)`
fn boundary_end(l: &Line, i: usize) -> bool {
    i >= l.len() || ws(l.b[i]) || matches!(l.b[i], b';' | b'>' | b'<' | b'|')
}

/// A builtin's name, anywhere: the grammar's `boundary_begin` is empty.
fn builtin_name(l: &Line, pos: usize, arg: u16) -> R {
    let name = BUILTINS[usize::from(arg - BUILTIN_ARG)].name;
    let mut at = pos;
    while let Some(i) = find_str(l.b, at, name) {
        if boundary_end(l, i + name.len()) {
            return Some(M::at(i, i + name.len()));
        }
        at = i + 1;
    }
    None
}

/// The builtin whose arguments the level holds.
fn level_builtin(l: &Line) -> Option<&'static Builtin> {
    let arg = l.arg.get();
    (arg >= BUILTIN_ARG).then(|| &BUILTINS[usize::from(arg - BUILTIN_ARG)])
}

/// `(?:\s+|^)--(?=\s|$)`, for a builtin that takes it.
fn builtin_end_of_options(l: &Line, pos: usize, _: u16) -> R {
    level_builtin(l).filter(|b| b.eoo)?;
    end_of_options(l, pos, 0)
}

/// `(?:\s+|^)((PREFIX)(?:[ONE]|[MANY]+)|[BARE]+)(?=\s|;|$|\`|\))`, for the builtin's options.
fn builtin_option(l: &Line, pos: usize, _: u16) -> R {
    let bi = level_builtin(l)?;
    if bi.one.is_empty() && bi.many.is_empty() && bi.bare.is_empty() {
        return None;
    }
    let ends = |i: usize| i >= l.len() || ws(l.b[i]) || matches!(l.b[i], b';' | b'`' | b')');
    let run = |from: usize, set: &[u8]| {
        let mut e = from;
        while e < l.len() && set.contains(&l.b[e]) {
            e += 1;
        }
        e
    };
    for k in pos..l.len() {
        let Some(s) = ws_or_bol(l, pos, k) else {
            continue;
        };
        if ws(l.b[k]) {
            continue;
        }
        if bi.prefix.contains(&l.b[k]) {
            let j = k + 1;
            if bi.one.contains(&l.at(j)) && l.at(j) != 0 && ends(j + 1) {
                return Some(M::at(s, j + 1).cap(1, k, j + 1).cap(2, k, j));
            }
            let e = run(j, bi.many);
            if e > j && ends(e) {
                return Some(M::at(s, e).cap(1, k, e).cap(2, k, j));
            }
        }
        let e = run(k, bi.bare);
        if e > k && ends(e) {
            return Some(M::at(s, e).cap(1, k, e));
        }
    }
    None
}

// ---- contexts ----

pub static PROTOTYPE: Ctx = Ctx::new(&[I(&COMMENT), I(&LINE_CONTINUATION), I(&ANY_ESCAPE)]);

static COMMENT: Ctx = Ctx::new(&[P(&p(comment)
    .caps(&[(1, &[ShComment, ShCommentMark])])
    .push(&[&COMMENT_BODY]))]);

static COMMENT_BODY: Ctx = Ctx::new(&[P(&p(nl).sc(&[ShComment]).pop())]).content(&[ShComment]);

static LINE_CONTINUATION: Ctx = Ctx::new(&[
    P(&p(backslash_nl).sc(&[ShContinuation]).push(&[&CONTINUED])),
    P(&p(backslash_ws_nl).caps(&[(1, &[ShExtraSpaces])])),
]);

// what the prototype pushes has no prototype, as syntect links it
static CONTINUED: Ctx = Ctx::new(&[P(&p(bol).pop())]);

static ANY_ESCAPE: Ctx = Ctx::new(&[P(&p(escape_any).sc(&[ShEscape]))]);

static POP_AT_END: Ctx = Ctx::new(&[P(&p(eol).pop())]);

static CONTINUATION_OR_POP: Ctx = Ctx::new(&[I(&POP_AT_END), I(&LINE_CONTINUATION)]);

/// `Shell-Unix-Generic`'s `main`, which a reference by scope pushes.
pub static SOURCE: Ctx = Ctx::new(&[I(&MAIN)]).content(&[Source]).sh();

/// Bash on its own (G-003): syntect's `__start`, which pushes `__main`, a copy of `main`
/// without the scope a reference to it gives.
pub static START: Ctx = Ctx::new(&[P(&p(empty).push(&[&BASH_MAIN]))]);

static BASH_MAIN: Ctx = Ctx::new(&[I(&MAIN)]).sh();

static MAIN: Ctx = Ctx::new(&[
    I(&FUNCDEF),
    I(&VARDEF),
    I(&REDIRECTION),
    I(&OPERATOR_EXCLAMATION),
    P(&p(non_ws_ahead).push(&[&CMD])),
]);

static MAIN_BT: Ctx = Ctx::new(&[
    I(&FUNCDEF_BT),
    I(&VARDEF),
    I(&REDIRECTION),
    P(&p(non_ws_ahead).push(&[&CMD_BT])),
]);

const fn kw(k: u16, sc: &'static [Sc]) -> Pat {
    p(keyword).arg(k).sc(sc)
}

static CONTROL: Ctx = Ctx::new(&[
    P(&kw(0, &[If]).pop()),
    P(&kw(1, &[Then]).pop()),
    P(&kw(2, &[Elif]).pop()),
    P(&kw(3, &[CondEnd]).set(&[&CMD_POST, &CMD_ARGS])),
    P(&kw(4, &[Else]).pop()),
    P(&kw(5, &[For]).set(&[&CMD_POST, &FOR_ARGS])),
    P(&kw(6, &[Do]).pop()),
    P(&kw(7, &[Done]).set(&[&CMD_POST, &CMD_ARGS])),
    P(&kw(8, &[While])),
    P(&kw(9, &[Until])),
    P(&kw(10, &[Case]).set(&[&CASE_BODY, &CASE_WORD])),
    P(&kw(11, &[ContinueKw])),
    P(&kw(12, &[Break]).set(&[&CMD_POST, &CMD_ARGS])),
    P(&kw(13, &[CondEnd]).pop()),
]);

static CASE_WORD: Ctx = Ctx::new(&[
    P(&kw(14, &[In]).pop()),
    I(&CASE_END_AHEAD),
    I(&EXPANSION_AND_STRING),
])
.sh();

static CASE_BODY: Ctx = Ctx::new(&[
    P(&kw(13, &[CondEnd]).pop()),
    P(&p(ahead::<b'('>).push(&[&CASE_PAREN])),
    P(&p(non_ws_ahead).push(&[&CASE_PATTERNS])),
])
.meta(&[CaseMeta])
.sh();

static CASE_PAREN: Ctx = Ctx::new(&[P(&p(byte::<b'('>)
    .sc(&[PatternsBegin])
    .set(&[&CASE_PATTERNS]))])
.clear(1)
.sh();

static CASE_PATTERNS: Ctx = Ctx::new(&[
    P(&p(byte::<b')'>).sc(&[PatternsEnd]).set(&[&CASE_COMMANDS])),
    P(&p(case_end_ahead).set(&[&CASE_COMMANDS])),
    I(&CASE_END_AHEAD),
    I(&CASE_PATTERNS_BODY),
])
.clear(1)
.meta(&[CasePatterns])
.sh();

static CASE_PATTERNS_BODY: Ctx = Ctx::new(&[
    I(&EXPANSION_PATTERN),
    I(&EXPANSION_TILDE),
    I(&EXPANSION_PARAMETER),
    I(&EXPANSION_COMMAND),
    I(&EXPANSION_ARITHMETIC),
    I(&STRING),
    P(&p(byte::<b'|'>).sc(&[Logical])),
    P(&p(byte::<b'('>)
        .sc(&[ParensBegin])
        .push(&[&CASE_PATTERNS_PAREN])),
]);

static CASE_PATTERNS_PAREN: Ctx = Ctx::new(&[
    P(&p(byte::<b')'>).sc(&[ParensEnd]).pop()),
    I(&CASE_PATTERNS_BODY),
])
.sh();

static CASE_COMMANDS: Ctx = Ctx::new(&[
    P(&p(case_end).sc(&[CaseCommands, CaseEnd]).pop()),
    I(&CASE_END_AHEAD),
    I(&MAIN),
])
.clear(1)
.content(&[CaseCommands])
.sh();

static CASE_END_AHEAD: Ctx = Ctx::new(&[P(&p(esac_ahead).pop())]);

static FOR_ARGS: Ctx = Ctx::new(&[P(&p(empty).set(&[&FOR_BODY]))]).sh();

static FOR_BODY: Ctx = Ctx::new(&[I(&CMD_ARGS_BOILERPLATE), I(&ARITHMETIC), P(&kw(14, &[In]))])
    .meta(&[GroupFor])
    .sh();

static CMD_POST: Ctx = Ctx::new(&[
    P(&p(semicolon).sc(&[Continue]).pop()),
    P(&p(or_or).sc(&[Or]).pop()),
    P(&p(byte::<b'|'>).sc(&[Pipe]).pop()),
    P(&p(and_and).sc(&[And]).pop()),
    P(&p(byte::<b'&'>).sc(&[Job]).pop()),
    P(&p(eol_or_non_ws).pop()),
])
.sh();

static CMD_ARGS_BOILERPLATE: Ctx = Ctx::new(&[
    P(&p(ahead::<b')'>).pop()),
    I(&CMD_ARGS_COMMON),
    P(&p(end_of_options)
        .sc(&[EndOfOptions])
        .set(&[&END_OF_OPTIONS])),
]);

static CMD_ARGS_BOILERPLATE_BT: Ctx = Ctx::new(&[
    P(&p(close_or_backtick_ahead).pop()),
    I(&CMD_ARGS_COMMON),
    P(&p(end_of_options)
        .sc(&[EndOfOptions])
        .set(&[&END_OF_OPTIONS_BT])),
]);

static END_OF_OPTIONS: Ctx = Ctx::new(&[I(&END_OF_OPTIONS_COMMON)])
    .content(&[FnArgsSh])
    .sh();

static END_OF_OPTIONS_BT: Ctx = Ctx::new(&[P(&p(ahead::<b'`'>).pop()), I(&END_OF_OPTIONS_COMMON)])
    .content(&[FnArgsSh])
    .sh();

static END_OF_OPTIONS_COMMON: Ctx = Ctx::new(&[
    I(&REDIRECTION),
    P(&p(end_of_options_end).pop()),
    I(&EXPANSION_AND_STRING),
    I(&CONTINUATION_OR_POP),
]);

static CMD_ARGS_COMMON: Ctx = Ctx::new(&[
    P(&p(brace_or_comment_ahead).pop()),
    I(&REDIRECTION),
    P(&p(separator_ahead).pop()),
    I(&EXPANSION_AND_STRING),
    I(&CONTINUATION_OR_POP),
]);

static CMD_ARGS: Ctx = Ctx::new(&[P(&p(empty).set(&[&CMD_ARGS_BODY]))]).sh();

static CMD_ARGS_BODY: Ctx = Ctx::new(&[
    I(&CMD_ARGS_BOILERPLATE),
    P(&p(option_start).sc(&[OptionMark]).push(&[&OPTION])),
])
.meta(&[FnArgsSh])
.sh();

static OPTION: Ctx = Ctx::new(&[
    P(&p(ahead::<b'='>).set(&[&OPTION_ASSIGN])),
    P(&p(option_end::<false>).pop()),
    I(&EXPANSION_AND_STRING),
])
.meta(&[ShOption])
.sh();

static OPTION_ASSIGN: Ctx = Ctx::new(&[P(&p(byte::<b'='>).sc(&[OptionAssign]).pop())]).sh();

static CMD_ARGS_BT: Ctx = Ctx::new(&[P(&p(empty).set(&[&CMD_ARGS_BODY_BT]))]).sh();

static CMD_ARGS_BODY_BT: Ctx = Ctx::new(&[
    I(&CMD_ARGS_BOILERPLATE_BT),
    P(&p(option_start).sc(&[OptionMark]).push(&[&OPTION_BT])),
])
.meta(&[FnArgsSh])
.sh();

static OPTION_BT: Ctx = Ctx::new(&[
    P(&p(ahead::<b'='>).set(&[&OPTION_ASSIGN])),
    P(&p(option_end::<true>).pop()),
    I(&EXPANSION_AND_STRING),
])
.meta(&[ShOption])
.sh();

static CMD: Ctx = Ctx::new(&[
    I(&CMD_COMMON),
    P(&p(byte::<b'('>).sc(&[CompoundBegin]).push(&[&CMD_PAREN])),
    I(&BUILTIN_COMMANDS),
    P(&p(let_kw).sc(&[Let]).push(&[&LET])),
    P(&p(double_bracket)
        .caps(&[(1, &[DoubleBraceBegin])])
        .set(&[&CMD_POST, &TEST_DOUBLE])),
    P(&p(bracket)
        .caps(&[(1, &[TestBegin])])
        .set(&[&CMD_POST, &TEST])),
    P(&p(brace_open)
        .caps(&[(1, &[BracesBegin])])
        .push(&[&CMD_BRACE])),
    P(&p(non_ws_ahead).set(&[&CMD_POST, &CMD_ARGS, &CMD_NAME])),
])
.sh();

static CMD_PAREN: Ctx = Ctx::new(&[
    P(&p(byte::<b')'>)
        .sc(&[CompoundEnd])
        .set(&[&CMD_POST, &CMD_ARGS])),
    I(&MAIN),
])
.sh();

static CMD_BRACE: Ctx = Ctx::new(&[
    P(&p(byte::<b'}'>)
        .sc(&[BracesEnd])
        .set(&[&CMD_POST, &CMD_ARGS])),
    I(&MAIN),
])
.sh();

static LET: Ctx = Ctx::new(&[P(&p(eol).pop()), I(&EXPRESSION)])
    .meta(&[FnCall])
    .sh();

static CMD_BT: Ctx = Ctx::new(&[
    I(&CMD_COMMON),
    P(&p(byte::<b'('>).sc(&[CompoundBegin]).push(&[&CMD_PAREN_BT])),
    I(&BUILTIN_COMMANDS_BT),
    P(&p(let_kw).sc(&[Let]).push(&[&LET_BT])),
    P(&p(double_bracket)
        .caps(&[(1, &[DoubleBraceBegin])])
        .set(&[&CMD_POST, &TEST_DOUBLE])),
    P(&p(bracket)
        .caps(&[(1, &[TestBegin])])
        .set(&[&CMD_POST, &TEST_BT])),
    P(&p(brace_open)
        .caps(&[(1, &[BracesBegin])])
        .push(&[&CMD_BRACE_BT])),
    P(&p(non_ws_ahead).set(&[&CMD_POST, &CMD_ARGS_BT, &CMD_NAME])),
])
.sh();

static CMD_PAREN_BT: Ctx = Ctx::new(&[
    P(&p(byte::<b')'>)
        .sc(&[CompoundEnd])
        .set(&[&CMD_POST, &CMD_ARGS_BT])),
    I(&MAIN),
])
.sh();

static CMD_BRACE_BT: Ctx = Ctx::new(&[
    P(&p(byte::<b'}'>)
        .sc(&[BracesEnd])
        .set(&[&CMD_POST, &CMD_ARGS_BT])),
    I(&MAIN_BT),
])
.sh();

static LET_BT: Ctx = Ctx::new(&[P(&p(eol_or_backtick).pop()), I(&EXPRESSION)])
    .meta(&[FnCall])
    .sh();

static TEST: Ctx = Ctx::new(&[P(&p(empty).set(&[&TEST_BODY]))]).sh();

static TEST_BODY: Ctx = Ctx::new(&[
    I(&CMD_ARGS_BOILERPLATE),
    P(&p(test_end).caps(&[(1, &[TestEnd])]).pop()),
    I(&EXPRESSION_TEST),
])
.meta(&[FnArgsSh])
.sh();

static TEST_BT: Ctx = Ctx::new(&[P(&p(empty).set(&[&TEST_BODY_BT]))]).sh();

static TEST_BODY_BT: Ctx = Ctx::new(&[
    I(&CMD_ARGS_BOILERPLATE_BT),
    P(&p(test_end).caps(&[(1, &[TestEnd])]).pop()),
    I(&EXPRESSION_TEST),
])
.meta(&[FnArgsSh])
.sh();

static TEST_DOUBLE: Ctx = Ctx::new(&[
    P(&p(double_test_end).caps(&[(1, &[DoubleBraceEnd])]).pop()),
    I(&EXPRESSION_TEST),
])
.meta(&[FnArgsSh])
.sh();

static CMD_NAME: Ctx = Ctx::new(&[P(&p(empty).set(&[&CMD_NAME_BODY]))]).sh();

static CMD_NAME_BODY: Ctx = Ctx::new(&[P(&p(name_end).pop()), I(&CMD_NAME_COMMON)])
    .meta(&[FnCall, FnName])
    .sh();

static CMD_NAME_COMMON: Ctx = Ctx::new(&[
    P(&p(name_common_end).pop()),
    I(&STRING),
    I(&EXPANSION_PARAMETER),
    I(&EXPANSION_ARITHMETIC),
    I(&EXPANSION_COMMAND),
    I(&EXPANSION_TILDE),
    I(&EXPANSION_JOB),
    I(&CONTINUATION_OR_POP),
]);

static CMD_COMMON: Ctx = Ctx::new(&[
    I(&CONTROL),
    I(&ARITHMETIC),
    P(&p(close_or_brace_ahead).pop()),
    I(&CONTINUATION_OR_POP),
]);

static ARITHMETIC: Ctx = Ctx::new(&[P(&p(arithmetic_open)
    .sc(&[ArithmeticBegin])
    .push(&[&ARITHMETIC_BODY]))]);

static ARITHMETIC_BODY: Ctx = Ctx::new(&[
    P(&p(close_close).sc(&[ArithmeticEnd]).pop()),
    I(&EXPRESSION),
])
.meta(&[Arithmetic])
.sh();

static EXPRESSION: Ctx = Ctx::new(&[
    P(&p(hex_start).sc(&[Base]).push(&[&HEX])),
    P(&p(octal_start).sc(&[Base]).push(&[&OCTAL])),
    P(&p(based).sc(&[OtherBase]).caps(&[(1, &[Base])])),
    P(&p(decimal).sc(&[Decimal])),
    P(&p(expr_assign).sc(&[ShAssign])),
    P(&p(expr_arith).sc(&[ArithOp])),
    P(&p(expr_logical).sc(&[Logical])),
    P(&p(bitwise).sc(&[Bitwise])),
    P(&p(comma_semicolon).sc(&[ShSeparator])),
    P(&p(byte::<b'?'>).sc(&[Ternary])),
    P(&p(byte::<b'('>)
        .sc(&[ParensBegin])
        .push(&[&EXPRESSION_PAREN])),
    I(&STRING),
    I(&EXPANSION_PARAMETER),
    I(&EXPANSION_ARITHMETIC),
    I(&EXPANSION_COMMAND),
]);

static HEX: Ctx = Ctx::new(&[
    P(&p(not_hex_letter).sc(&[NotHex]).pop()),
    P(&p(not_hex_ahead).pop()),
])
.meta(&[Hex])
.sh();

static OCTAL: Ctx = Ctx::new(&[
    P(&p(eight_nine).sc(&[NotOctal]).pop()),
    P(&p(not_octal_ahead).pop()),
])
.meta(&[Octal])
.sh();

static EXPRESSION_PAREN: Ctx =
    Ctx::new(&[P(&p(byte::<b')'>).sc(&[ParensEnd]).pop()), I(&EXPRESSION)])
        .meta(&[ParensGroup])
        .sh();

static EXPRESSION_TEST: Ctx = Ctx::new(&[
    I(&EXPANSION_AND_STRING),
    P(&p(test_unary).caps(&[(2, &[OptionMark]), (1, &[ShOption])])),
    P(&p(test_binary).caps(&[(2, &[OptionMark]), (1, &[ShOption])])),
    P(&p(regex_match).caps(&[(1, &[Logical])]).push(&[&REGEXP])),
    P(&p(test_logical).sc(&[Logical])),
]);

static REGEXP: Ctx = Ctx::new(&[P(&p(ws_ahead).pop()), I(&EXPANSION_AND_STRING)])
    .content(&[Regexp])
    .sh();

static EXPANSION_TILDE: Ctx = Ctx::new(&[P(&p(byte::<b'~'>).sc(&[TildeGroup, Tilde]))]);

static EXPANSION_BRACE: Ctx =
    Ctx::new(&[P(&p(byte::<b'{'>).sc(&[BraceBegin]).push(&[&BRACE_BODY]))]);

static BRACE_BODY: Ctx = Ctx::new(&[
    P(&p(byte::<b'}'>).sc(&[BraceEnd]).pop()),
    P(&p(byte::<b','>).sc(&[ShSeparator])),
    I(&EXPANSION_AND_STRING),
])
.meta(&[BraceGroup])
.sh();

static EXPANSION_PARAMETER: Ctx = Ctx::new(&[
    P(&p(dollar_brace)
        .caps(&[(0, &[Parameter]), (1, &[VarMark]), (2, &[ParameterBegin])])
        .push(&[&PARAMETER_START])),
    P(&p(dollar_digit).caps(&[(0, &[Parameter]), (1, &[VarMark]), (2, &[ReadWrite])])),
    P(&p(dollar_special).caps(&[(0, &[Parameter]), (1, &[VarMark]), (2, &[LangVar])])),
    P(&p(dollar_identifier).caps(&[(0, &[Parameter]), (1, &[VarMark]), (2, &[ReadWrite])])),
]);

static PARAMETER_START: Ctx = Ctx::new(&[
    P(&p(byte::<b'!'>).sc(&[Indirection]).set(&[&PARAMETER_POST])),
    P(&p(byte::<b'#'>).sc(&[ArithOp]).set(&[&PARAMETER_POST])),
    P(&p(empty).set(&[&PARAMETER_POST])),
])
.content(&[Parameter]);

static PARAMETER_POST: Ctx = Ctx::new(&[
    I(&PARAMETER_COMMON),
    P(&p(slash_ahead).set(&[&PARAMETER_SLASH])),
    P(&p(parameter_op_ahead).set(&[&PARAMETER_OP])),
    P(&p(substring_ahead).set(&[&PARAMETER_SUBSTRING])),
    P(&p(hash_before_brace)),
    P(&p(parameter_expansion_op)
        .caps(&[(1, &[LangVar]), (2, &[ExpansionOp])])
        .set(&[&PARAMETER_PATTERN])),
    P(&p(parameter_transform).caps(&[(1, &[LangVar]), (2, &[ExpansionOp]), (3, &[Switch])])),
    I(&ARRAY),
    P(&p(star_at_before_brace).sc(&[LangVar])),
])
.content(&[Parameter, ReadWrite])
.sh();

static PARAMETER_OP: Ctx = Ctx::new(&[P(&p(parameter_op)
    .sc(&[ShAssign])
    .set(&[&PARAMETER_COMMON]))])
.content(&[Parameter])
.sh();

static PARAMETER_PATTERN: Ctx =
    Ctx::new(&[I(&PARAMETER_COMMON), I(&EXPANSION_PATTERN)]).content(&[Parameter]);

static PARAMETER_COMMON: Ctx = Ctx::new(&[
    P(&p(byte::<b'}'>).sc(&[Parameter, ParameterEnd]).pop()),
    I(&STRING),
    I(&EXPANSION_PARAMETER),
    I(&EXPANSION_ARITHMETIC),
    I(&EXPANSION_COMMAND),
    I(&EXPANSION_TILDE),
    I(&ANY_ESCAPE),
])
.content(&[Parameter])
.sh();

static EXPANSION_ARITHMETIC: Ctx = Ctx::new(&[P(&p(dollar_arithmetic)
    .caps(&[(1, &[VarMark]), (2, &[ParensBegin])])
    .push(&[&ARITHMETIC_EXPANSION]))]);

static ARITHMETIC_EXPANSION: Ctx =
    Ctx::new(&[P(&p(close_close).sc(&[ParensEnd]).pop()), I(&EXPRESSION)])
        .meta(&[ArithExpansion])
        .sh();

static EXPANSION_COMMAND: Ctx = Ctx::new(&[
    P(&p(dollar_paren)
        .caps(&[(1, &[VarMark]), (2, &[ParensBegin])])
        .push(&[&COMMAND_PARENS])),
    P(&p(byte::<b'`'>).sc(&[ShGroupBegin]).push(&[&BACKTICKS])),
]);

static COMMAND_PARENS: Ctx =
    Ctx::new(&[P(&p(ws_close).caps(&[(1, &[ParensEnd])]).pop()), I(&MAIN)])
        .meta(&[CommandParens])
        .sh();

static BACKTICKS: Ctx = Ctx::new(&[P(&p(byte::<b'`'>).sc(&[ShGroupEnd]).pop()), I(&MAIN_BT)])
    .meta(&[Backticks])
    .sh();

static EXPANSION_JOB: Ctx = Ctx::new(&[
    P(&p(job)
        .arg(0)
        .caps(&[(0, &[JobGroup]), (1, &[JobMark]), (2, &[JobVar])])),
    P(&p(job)
        .arg(1)
        .caps(&[(0, &[JobGroup]), (1, &[JobMark]), (2, &[JobNumber])])),
    P(&p(job).arg(2).caps(&[
        (0, &[JobGroup]),
        (1, &[JobMark]),
        (2, &[Quantifier]),
        (3, &[ReadWrite]),
    ])),
    P(&p(job).arg(3).sc(&[JobGroup, JobMark])),
]);

static EXPANSION_PATTERN: Ctx = Ctx::new(&[
    P(&p(extglob)
        .caps(&[(1, &[Quantifier]), (2, &[ParensBegin])])
        .push(&[&EXTGLOB])),
    P(&p(glob).sc(&[Quantifier])),
    P(&p(set_open).sc(&[SetBegin]).push(&[&SET_START])),
]);

static EXTGLOB: Ctx = Ctx::new(&[
    P(&p(byte::<b')'>).sc(&[ParensEnd]).pop()),
    P(&p(byte::<b'|'>).sc(&[Or])),
    I(&EXPANSION_AND_STRING),
])
.sh();

static SET_START: Ctx = Ctx::new(&[
    P(&p(ahead::<b']'>).set(&[&SET_REST])),
    P(&p(set_not).sc(&[LogicalNot]).set(&[&SET_REST])),
    P(&p(byte::<b'-'>).set(&[&SET_REST])),
    P(&p(empty).set(&[&SET_REST])),
])
.sh();

static SET_REST: Ctx = Ctx::new(&[
    P(&p(set_close).caps(&[(1, &[SetEnd])]).pop()),
    P(&p(byte::<b'-'>).sc(&[WordOp])),
    P(&p(delimited_word::<b'.'>).caps(&[(1, &[CollateBegin]), (2, &[CollateEnd])])),
    P(&p(delimited_word::<b'='>).caps(&[(1, &[EquivalenceBegin]), (2, &[EquivalenceEnd])])),
    P(&p(char_class).caps(&[(1, &[ClassBegin]), (2, &[ClassEnd])])),
    P(&p(bracket_plain)),
    I(&EXPANSION_AND_STRING),
])
.sh();

static EXPANSION: Ctx = Ctx::new(&[
    I(&EXPANSION_PATTERN),
    I(&EXPANSION_PARAMETER),
    I(&EXPANSION_BRACE),
    I(&EXPANSION_ARITHMETIC),
    I(&EXPANSION_COMMAND),
    I(&EXPANSION_TILDE),
    I(&EXPANSION_JOB),
]);

static EXPANSION_AND_STRING: Ctx = Ctx::new(&[I(&STRING), I(&EXPANSION)]);

static STRING: Ctx = Ctx::new(&[
    P(&p(byte::<b'"'>).sc(&[ShStrBegin]).push(&[&STRING_DOUBLE])),
    P(&p(byte::<b'\''>).sc(&[ShStrBegin]).push(&[&STRING_SINGLE])),
    P(&p(dollar_quote).sc(&[ShStrBegin]).push(&[&STRING_ANSI_C])),
    P(&p(dollar_dquote).sc(&[ShStrBegin]).push(&[&STRING_LOCALE])),
]);

static STRING_SINGLE: Ctx =
    Ctx::new(&[P(&p(byte::<b'\''>).sc(&[ShStrEnd]).pop())]).meta(&[ShStrSingle]);

static STRING_DOUBLE: Ctx = Ctx::new(&[I(&STRING_DOUBLE_COMMON)]).meta(&[ShStrDouble]);

static STRING_DOUBLE_ESCAPE: Ctx = Ctx::new(&[
    P(&p(dq_escape).sc(&[ShEscape])),
    P(&p(backslash_nl).sc(&[ShEscape]).push(&[&ESCAPED_NEWLINE])),
]);

static ESCAPED_NEWLINE: Ctx = Ctx::new(&[P(&p(non_ws_ahead).pop())]);

static STRING_ANSI_C: Ctx = Ctx::new(&[
    P(&p(byte::<b'\''>).sc(&[ShStrEnd]).pop()),
    I(&STRING_DOUBLE_ESCAPE),
    P(&p(ansi_escape).sc(&[ShEscape])),
])
.meta(&[ShStrAnsiC]);

static STRING_LOCALE: Ctx = Ctx::new(&[I(&STRING_DOUBLE_COMMON)]).meta(&[ShStrLocale]);

static STRING_DOUBLE_COMMON: Ctx = Ctx::new(&[
    P(&p(byte::<b'"'>).sc(&[ShStrEnd]).pop()),
    I(&STRING_DOUBLE_ESCAPE),
    I(&EXPANSION_PARAMETER),
    I(&EXPANSION_ARITHMETIC),
    I(&EXPANSION_COMMAND),
]);

static ARRAY: Ctx = Ctx::new(&[P(&p(byte::<b'['>)
    .sc(&[SectionBracesBegin])
    .push(&[&ARRAY_BODY]))]);

static ARRAY_BODY: Ctx = Ctx::new(&[
    P(&p(byte::<b']'>).sc(&[SectionBracesEnd]).pop()),
    P(&p(star_at).sc(&[ArrayVar])),
    I(&EXPRESSION),
])
.sh();

static VARDEF: Ctx = Ctx::new(&[
    P(&p(modifier).arg(0).caps(&[(1, &[Alias])]).push(&[
        &VARDEF_FUNCTION_CALL,
        &VARDEF_MORE,
        &VARDEF_VALUE,
        &VARDEF_ASSIGN,
        &VARDEF_ALIAS_NAME,
        &VARDEF_ALIAS_OPTIONS,
    ])),
    P(&p(modifier).arg(1).caps(&[(1, &[Modifier])]).push(&[
        &VARDEF_FUNCTION_CALL,
        &VARDEF_MORE,
        &VARDEF_VALUE,
        &VARDEF_ASSIGN,
        &VARDEF_NAME,
        &VARDEF_DECLARE_OPTIONS,
    ])),
    P(&p(modifier).arg(2).caps(&[(1, &[Modifier])]).push(&[
        &VARDEF_FUNCTION_CALL,
        &VARDEF_MORE,
        &VARDEF_VALUE,
        &VARDEF_ASSIGN,
        &VARDEF_NAME,
        &VARDEF_EXPORT_OPTIONS,
    ])),
    P(&p(modifier).arg(3).caps(&[(1, &[Modifier])]).push(&[
        &VARDEF_FUNCTION_CALL,
        &VARDEF_MORE,
        &VARDEF_VALUE,
        &VARDEF_ASSIGN,
        &VARDEF_NAME,
        &VARDEF_READONLY_OPTIONS,
    ])),
    P(&p(is_variable).push(&[&VARDEF_VALUE, &VARDEF_ASSIGN, &VARDEF_NAME])),
]);

static VARDEF_READONLY_OPTIONS: Ctx = Ctx::new(&[
    P(&p(modifier_option)
        .arg(0)
        .caps(&[(2, &[OptionMark]), (1, &[ShOption])])),
    P(&p(spaces).pop()),
])
.sh();

static VARDEF_EXPORT_OPTIONS: Ctx = Ctx::new(&[
    P(&p(modifier_option)
        .arg(1)
        .caps(&[(2, &[OptionMark]), (1, &[ShOption])])),
    P(&p(spaces).pop()),
])
.sh();

static VARDEF_DECLARE_OPTIONS: Ctx = Ctx::new(&[
    P(&p(modifier_option)
        .arg(2)
        .caps(&[(2, &[OptionMark]), (1, &[ShOption])])),
    P(&p(spaces).pop()),
])
.sh();

static VARDEF_ALIAS_OPTIONS: Ctx = Ctx::new(&[
    P(&p(modifier_option)
        .arg(3)
        .caps(&[(2, &[OptionMark]), (1, &[ShOption])])),
    P(&p(spaces).pop()),
])
.sh();

static VARDEF_FUNCTION_CALL: Ctx = Ctx::new(&[P(&p(empty).pop())]).meta(&[FnCall]);

static VARDEF_MORE: Ctx = Ctx::new(&[
    P(&p(ahead::<b'`'>).pop()),
    P(&p(comment_ahead).pop()),
    I(&CMD_ARGS_BOILERPLATE),
    P(&p(non_ws_ahead).push(&[&VARDEF_VALUE, &VARDEF_ASSIGN, &VARDEF_NAME])),
]);

static VARDEF_ALIAS_NAME: Ctx = Ctx::new(&[P(&p(spaces).set(&[&VARDEF_ALIAS_NAME_BODY]))]).sh();

static VARDEF_ALIAS_NAME_BODY: Ctx = Ctx::new(&[
    I(&CONTINUATION_OR_POP),
    I(&ANY_ESCAPE),
    P(&p(name_stop::<false>).pop()),
    I(&ARRAY),
    P(&p(ws_eol).pop()),
    I(&STRING),
])
.content(&[AliasName]);

static VARDEF_NAME: Ctx = Ctx::new(&[P(&p(spaces).set(&[&VARDEF_NAME_BODY]))]).sh();

static VARDEF_NAME_BODY: Ctx = Ctx::new(&[
    I(&CONTINUATION_OR_POP),
    I(&ANY_ESCAPE),
    P(&p(name_stop::<true>).pop()),
    I(&ARRAY),
    P(&p(ws_eol).pop()),
    I(&STRING),
])
.content(&[VarAssign]);

static VARDEF_ASSIGN: Ctx = Ctx::new(&[
    I(&CONTINUATION_OR_POP),
    I(&ANY_ESCAPE),
    P(&p(varassign).sc(&[ShAssign]).pop()),
    P(&p(empty).pop()),
]);

static VARDEF_VALUE: Ctx = Ctx::new(&[
    P(&p(byte::<b'('>).sc(&[ParensBegin]).set(&[&VARDEF_ARRAY])),
    P(&p(amp_or_backtick_ahead).pop()),
    P(&p(empty).set(&[&VARDEF_VALUE_WORD])),
]);

static VARDEF_ARRAY: Ctx = Ctx::new(&[
    P(&p(byte::<b')'>).sc(&[ParensEnd]).pop()),
    P(&p(byte::<b'['>)
        .sc(&[BracketsBegin])
        .push(&[&VARDEF_ARRAY_INDEX])),
    I(&EXPANSION_AND_STRING),
])
.sh();

static VARDEF_ARRAY_INDEX: Ctx = Ctx::new(&[
    P(&p(byte::<b']'>).sc(&[BracketsEnd]).set(&[&VARDEF_ARRAY_EQ])),
    I(&EXPANSION_AND_STRING),
])
.sh();

static VARDEF_ARRAY_EQ: Ctx = Ctx::new(&[
    P(&p(byte::<b'='>).sc(&[ShAssign]).pop()),
    P(&p(empty).pop()),
])
.sh();

static VARDEF_VALUE_WORD: Ctx = Ctx::new(&[
    P(&p(ahead::<b'`'>).pop()),
    I(&EXPANSION_AND_STRING),
    I(&CONTINUATION_OR_POP),
    I(&ANY_ESCAPE),
    P(&p(metachar_ahead).pop()),
])
.meta(&[ShUnquoted]);

static REDIRECTION: Ctx = Ctx::new(&[
    P(&p(here_string).caps(&[(1, &[Fd]), (2, &[HereString])])),
    P(&p(heredoc)
        .arg(0)
        .caps(QUOTED_HEREDOC)
        .push(&[&HEREDOC_TABS_LITERAL, &HEREDOC_PREAMBLE])),
    P(&p(heredoc)
        .arg(1)
        .caps(QUOTED_HEREDOC)
        .push(&[&HEREDOC_TABS_LITERAL, &HEREDOC_PREAMBLE])),
    P(&p(heredoc)
        .arg(2)
        .caps(ESCAPED_HEREDOC)
        .push(&[&HEREDOC_TABS_LITERAL, &HEREDOC_PREAMBLE])),
    P(&p(heredoc)
        .arg(3)
        .caps(BARE_HEREDOC)
        .push(&[&HEREDOC_TABS, &HEREDOC_PREAMBLE])),
    P(&p(heredoc)
        .arg(4)
        .caps(QUOTED_HEREDOC)
        .push(&[&HEREDOC_LITERAL, &HEREDOC_PREAMBLE])),
    P(&p(heredoc)
        .arg(5)
        .caps(QUOTED_HEREDOC)
        .push(&[&HEREDOC_LITERAL, &HEREDOC_PREAMBLE])),
    P(&p(heredoc)
        .arg(6)
        .caps(ESCAPED_HEREDOC)
        .push(&[&HEREDOC_LITERAL, &HEREDOC_PREAMBLE])),
    P(&p(heredoc)
        .arg(7)
        .caps(BARE_HEREDOC)
        .push(&[&HEREDOC, &HEREDOC_PREAMBLE])),
    P(&p(process_substitution)
        .caps(&[(1, &[Fd]), (2, &[RedirectProcess]), (3, &[ParensBegin])])
        .push(&[&PROCESS_SUBSTITUTION])),
    P(&p(redirect_input)
        .caps(&[(1, &[Fd]), (2, &[Redirect])])
        .push(&[&REDIRECTION_POST])),
    P(&p(redirect_output)
        .caps(&[(1, &[Fd]), (2, &[Redirect])])
        .push(&[&REDIRECTION_POST])),
    P(&p(redirect_inout).caps(&[(1, &[Fd]), (2, &[Redirect])])),
]);

static PROCESS_SUBSTITUTION: Ctx =
    Ctx::new(&[P(&p(byte::<b')'>).sc(&[ParensEnd]).pop()), I(&MAIN)]).sh();

static REDIRECTION_POST: Ctx = Ctx::new(&[
    P(&p(redirect_fd).caps(&[(1, &[Fd]), (2, &[FdEnd])]).pop()),
    P(&p(ws_then_non_ws).set(&[&REDIRECTION_TARGET])),
    P(&p(spaces).pop()),
])
.sh();

static REDIRECTION_TARGET: Ctx = Ctx::new(&[
    P(&p(metachar_or_backtick_ahead).pop()),
    I(&EXPANSION_AND_STRING),
])
.sh();

static OPERATOR_EXCLAMATION: Ctx = Ctx::new(&[
    P(&p(bang_alone).sc(&[Logical])),
    P(&p(history).sc(&[History]).caps(&[(1, &[HistoryMark])])),
    P(&p(byte::<b'!'>).sc(&[HistoryMark])),
]);

/// A builtin command: its name's scope, then its arguments.
macro_rules! builtin_commands {
    ($args:ident, $($i:literal $sc:ident,)*) => {
        Ctx::new(&[$(
            P(&p(builtin_name)
                .arg(BUILTIN_ARG + $i)
                .sc(&[FnCall, $sc])
                .set(&[&CMD_POST, &$args])),
        )*])
    };
}

macro_rules! builtins {
    ($($i:literal $sc:ident,)*) => {
        static BUILTIN_COMMANDS: Ctx = builtin_commands!(BUILTIN_ARGS, $($i $sc,)*);
        static BUILTIN_COMMANDS_BT: Ctx = builtin_commands!(BUILTIN_ARGS_BT, $($i $sc,)*);
    };
}

builtins! {
    0 BuiltinSet, 1 BuiltinHelp, 2 BuiltinMapfile, 3 BuiltinGetopts, 4 BuiltinCd,
    5 BuiltinSuspend, 6 BuiltinColon, 7 BuiltinPrintf, 8 BuiltinHash, 9 BuiltinUlimit,
    10 BuiltinUmask, 11 BuiltinSource, 12 BuiltinBuiltin, 13 BuiltinExit, 14 BuiltinType,
    15 BuiltinBg, 16 Return, 17 BuiltinExec, 18 BuiltinRead, 19 BuiltinEnable,
    20 BuiltinPwd, 21 BuiltinLogout, 22 BuiltinEval, 23 BuiltinFg, 24 BuiltinDisown,
    25 BuiltinEcho, 26 BuiltinDot, 27 BuiltinWait, 28 BuiltinJobs, 29 BuiltinBind,
    30 BuiltinCaller, 31 BuiltinTimes, 32 BuiltinUnalias, 33 BuiltinCommand, 34 BuiltinTrap,
    35 BuiltinShift, 36 BuiltinKill, 37 BuiltinReadarray, 38 BuiltinShopt, 39 BuiltinUnset,
}

// `commands-builtin-shell-bash`, whose prototype is the shell's
static BUILTIN_ARGS: Ctx = Ctx::new(&[P(&p(empty).set(&[&BUILTIN_ARGS_BODY]))]).sh();

static BUILTIN_ARGS_BODY: Ctx = Ctx::new(&[I(&CMD_ARGS_BOILERPLATE), I(&BUILTIN_BASE)])
    .meta(&[FnArgsSh])
    .sh();

static BUILTIN_ARGS_BT: Ctx = Ctx::new(&[P(&p(empty).set(&[&BUILTIN_ARGS_BODY_BT]))]).sh();

static BUILTIN_ARGS_BODY_BT: Ctx = Ctx::new(&[I(&CMD_ARGS_BOILERPLATE_BT), I(&BUILTIN_BASE)])
    .meta(&[FnArgsSh])
    .sh();

static BUILTIN_BASE: Ctx = Ctx::new(&[
    P(&p(builtin_end_of_options)
        .nocache()
        .sc(&[EndOfOptions])
        .set(&[&BUILTIN_END_OF_OPTIONS])),
    P(&p(builtin_option)
        .nocache()
        .caps(&[(1, &[ShOption]), (2, &[OptionMark])])),
]);

static BUILTIN_END_OF_OPTIONS: Ctx = Ctx::new(&[I(&EXPANSION_AND_STRING), I(&CONTINUATION_OR_POP)])
    .content(&[FnArgsSh])
    .sh();

/// `{{is_function}}`: `\s*\b(function)\s+|(?=\s*{{identifier_non_posix}}\s*\(\s*\))`, its
/// leftmost match, as [`is_variable`]'s.
fn is_function(l: &Line, pos: usize, _: u16) -> R {
    let keyword = {
        let mut at = pos;
        let mut found = None;
        while let Some(k) = find_str(l.b, at, b"function") {
            let e = k + 8;
            let end = ws_end(l.b, e);
            if !l.word_before(k) && end > e {
                found = Some(M::at(ws_back(l.b, pos, k), end).cap(1, k, e));
                break;
            }
            at = k + 1;
        }
        found
    };
    let name = function_name(l, pos).map(|q| M::at(q, q));
    match (keyword, name) {
        (Some(k), Some(n)) => Some(if n.s() < k.s() { n } else { k }),
        (k, n) => k.or(n),
    }
}

/// Where `(?=\s*{{identifier_non_posix}}\s*\(\s*\))` first holds: a name, `[^{{metachar}}\d]`
/// then `[^{{metachar}}=]*`, and `()`. Every start in a run of `[^{{metachar}}=]` but a digit
/// gives the same name's end, and an `=` may start a name too.
fn function_name(l: &Line, pos: usize) -> Option<usize> {
    let run_end = |mut i: usize| {
        while i < l.len() && !metachar(l.b[i]) && l.b[i] != b'=' {
            i += 1;
        }
        i
    };
    let parens = |e: usize| {
        let open = ws_end(l.b, e);
        l.at(open) == b'(' && l.at(ws_end(l.b, open + 1)) == b')'
    };
    let mut i = pos;
    while i < l.len() {
        let c = l.b[i];
        if metachar(c) {
            i += 1;
            continue;
        }
        if c == b'=' {
            if parens(run_end(i + 1)) {
                return Some(ws_back(l.b, pos, i));
            }
            i += 1;
            continue;
        }
        let e = run_end(i);
        if parens(e)
            && let Some(q) = (i..e).find(|&q| !l.b[q].is_ascii_digit())
        {
            return Some(if q == i { ws_back(l.b, pos, i) } else { q });
        }
        i = e;
    }
    None
}

/// `(?=\s*[({]|$)`
fn function_name_end(l: &Line, pos: usize, _: u16) -> R {
    let e = eol_at(l, pos);
    let at = (pos..e)
        .find(|&i| matches!(l.b[ws_end(l.b, i).min(e)], b'(' | b'{') && ws_end(l.b, i) < e)
        .unwrap_or(e);
    Some(M::at(at, at))
}

/// `(\()\s*(\))`
fn empty_parens(l: &Line, pos: usize, _: u16) -> R {
    let mut at = pos;
    while let Some(i) = find(l.b, at, b'(') {
        let close = ws_end(l.b, i + 1);
        if l.at(close) == b')' {
            return Some(
                M::at(i, close + 1)
                    .cap(1, i, i + 1)
                    .cap(2, close, close + 1),
            );
        }
        at = i + 1;
    }
    None
}

static FUNCDEF: Ctx = Ctx::new(&[
    P(&p(is_function).caps(&[(1, &[FunctionKw])]).push(&[
        &FUNCDEF_BODY,
        &FUNCDEF_PARENS,
        &FUNCDEF_NAME,
    ])),
    P(&kw(16, &[Coproc]).push(&[&CMD_POST, &CMD_ARGS, &COPROC_BODY])),
]);

static FUNCDEF_BT: Ctx = Ctx::new(&[
    P(&p(is_function).caps(&[(1, &[FunctionKw])]).push(&[
        &FUNCDEF_BODY_BT,
        &FUNCDEF_PARENS,
        &FUNCDEF_NAME,
    ])),
    P(&kw(16, &[Coproc]).push(&[&CMD_POST, &CMD_ARGS_BT, &COPROC_BODY])),
]);

/// `\s*(?=\S+\s*\{)`: from a run of whitespace, or none, before a word that a `{` ends or
/// follows. Every start in a word but its first gives a shorter word, so a word is tried once.
fn coproc_name_ahead(l: &Line, pos: usize, _: u16) -> R {
    let b = l.b;
    let mut s = pos;
    while s < l.len() {
        let j = ws_end(b, s);
        let k = (j..l.len()).find(|&i| ws(b[i])).unwrap_or(l.len());
        if k > j && (find(&b[..k], j + 1, b'{').is_some() || l.at(ws_end(b, k)) == b'{') {
            return Some(M::at(s, j));
        }
        s = k.max(s + 1);
    }
    None
}

/// `(?=\s*\{)`
fn brace_ahead(l: &Line, pos: usize, _: u16) -> R {
    find(l.b, pos, b'{').map(|i| {
        let s = ws_back(l.b, pos, i);
        M::at(s, s)
    })
}

static COPROC_BODY: Ctx = Ctx::new(&[
    P(&p(coproc_name_ahead).set(&[&COPROC_NAME])),
    P(&p(empty).set(&[&MAIN_WITH_POP_AT_END])),
])
.sh();

static COPROC_NAME: Ctx = Ctx::new(&[P(&p(brace_ahead).set(&[&COPROC_BRACE]))])
    .content(&[CoprocName])
    .sh();

static COPROC_BRACE: Ctx = Ctx::new(&[P(&p(byte::<b'{'>)
    .sc(&[SectionBracesBegin])
    .set(&[&COPROC_BRACES]))])
.sh();

static COPROC_BRACES: Ctx =
    Ctx::new(&[P(&p(byte::<b'}'>).sc(&[SectionBracesEnd]).pop()), I(&MAIN)])
        .meta(&[CoprocMeta])
        .sh();

static FUNCDEF_NAME: Ctx = Ctx::new(&[P(&p(spaces).set(&[&FUNCDEF_NAME_BODY]))]).sh();

static FUNCDEF_NAME_BODY: Ctx = Ctx::new(&[P(&p(function_name_end).pop())])
    .content(&[FunctionName])
    .sh();

static FUNCDEF_PARENS: Ctx = Ctx::new(&[
    P(&p(empty_parens).caps(&[(1, &[ParensBegin]), (2, &[ParensEnd])])),
    P(&p(byte::<b'{'>).sc(&[SectionBracesBegin]).pop()),
    P(&p(byte::<b'('>).sc(&[CompoundBegin]).pop()),
])
.sh();

static FUNCDEF_BODY: Ctx = Ctx::new(&[
    P(&p(byte::<b'}'>).sc(&[SectionBracesEnd]).pop()),
    P(&p(byte::<b')'>).sc(&[CompoundEnd]).pop()),
    I(&MAIN),
])
.meta(&[Function])
.sh();

static FUNCDEF_BODY_BT: Ctx = Ctx::new(&[
    P(&p(byte::<b'}'>).sc(&[SectionBracesEnd]).pop()),
    P(&p(byte::<b')'>).sc(&[CompoundEnd]).pop()),
    I(&MAIN_BT),
])
.meta(&[Function])
.sh();

// here-documents

/// `(\d*)(<<-?)\s*` and the end word, as form `form` of the grammar's eight: `<<-` for 0–3,
/// `<<` for 4–7; the word in `'…'`, in `"…"`, after a `\`, or bare.
fn heredoc(l: &Line, pos: usize, form: u16) -> R {
    let dash = form < 4;
    let op: &[u8] = if dash { b"<<-" } else { b"<<" };
    let mut at = pos;
    while let Some(k) = find_str(l.b, at, op) {
        at = k + 1;
        let o = k + op.len();
        let j = ws_end(l.b, o);
        let s = digits_back(l, pos, k);
        let m = M::at(s, s).cap(1, s, k).cap(2, k, o);
        let word = |i: usize| identifier(l, i).map(|n| i + n);
        match form % 4 {
            q @ (0 | 1) => {
                let quote = if q == 0 { b'\'' } else { b'"' };
                if l.at(j) == quote
                    && let Some(e) = word(j + 1)
                    && l.at(e) == quote
                {
                    return Some(
                        m.end(e + 1)
                            .cap(3, j, j + 1)
                            .cap(4, j + 1, e)
                            .cap(5, e, e + 1),
                    );
                }
            }
            2 => {
                if l.at(j) == b'\\'
                    && let Some(e) = word(j + 1)
                {
                    return Some(m.end(e).cap(3, j, j + 1).cap(4, j + 1, e));
                }
            }
            _ => {
                if let Some(e) = word(j) {
                    return Some(m.end(e).cap(3, j, e));
                }
            }
        }
    }
    None
}

const QUOTED_HEREDOC: &[(u8, &[Sc])] = &[
    (1, &[Fd]),
    (2, &[Redirect]),
    (3, &[ShStrBegin]),
    (4, &[HeredocToken]),
    (5, &[ShStrEnd]),
];

const ESCAPED_HEREDOC: &[(u8, &[Sc])] = &[
    (1, &[Fd]),
    (2, &[Redirect]),
    (3, &[StringMark]),
    (4, &[HeredocToken]),
];

const BARE_HEREDOC: &[(u8, &[Sc])] = &[(1, &[Fd]), (2, &[Redirect]), (3, &[HeredocToken])];

/// The end word at the start of the line, after whitespace when `TABS`: where it starts and
/// ends.
fn end_word<const TABS: bool>(l: &Line, pos: usize) -> Option<(usize, usize)> {
    let word = l.cap.borrow();
    let i = if TABS { ws_end(l.b, 0) } else { 0 };
    (pos == 0 && !word.is_empty() && l.b[i..].starts_with(&word)).then(|| (i, i + word.len()))
}

/// `^\3(\s+)\n`, or `^\s*\3(\s+)\n` when `TABS`
fn heredoc_end_spaces<const TABS: bool>(l: &Line, pos: usize, _: u16) -> R {
    let (_, e) = end_word::<TABS>(l, pos)?;
    let n = l.len();
    (n > e + 1 && l.b[n - 1] == b'\n' && l.b[e..n - 1].iter().all(|&c| ws(c)))
        .then(|| M::at(0, n).cap(1, e, n - 1))
}

/// `^\3$`, or `^\s*(\3)$` when `TABS`
fn heredoc_end<const TABS: bool>(l: &Line, pos: usize, _: u16) -> R {
    let (i, e) = end_word::<TABS>(l, pos)?;
    (e == l.len() || (l.b[e] == b'\n' && e + 1 == l.len())).then(|| M::at(0, e).cap(1, i, e))
}

static HEREDOC: Ctx = Ctx::new(&[
    I(&HEREDOC_EXPANSION),
    P(&p(heredoc_end_spaces::<false>)
        .nocache()
        .caps(&[(1, &[HeredocSpaces])])
        .pop()),
    P(&p(heredoc_end::<false>).nocache().sc(&[HeredocToken]).pop()),
])
.meta(&[Heredoc])
.backref(3);

static HEREDOC_TABS: Ctx = Ctx::new(&[
    I(&HEREDOC_EXPANSION),
    P(&p(heredoc_end_spaces::<true>)
        .nocache()
        .caps(&[(1, &[HeredocSpaces])])
        .pop()),
    P(&p(heredoc_end::<true>)
        .nocache()
        .caps(&[(1, &[HeredocToken])])
        .pop()),
])
.meta(&[Heredoc])
.backref(3);

static HEREDOC_LITERAL: Ctx = Ctx::new(&[
    P(&p(heredoc_end_spaces::<false>)
        .nocache()
        .caps(&[(1, &[HeredocSpaces])])
        .pop()),
    P(&p(heredoc_end::<false>).nocache().sc(&[HeredocToken]).pop()),
])
.meta(&[Heredoc])
.backref(4);

static HEREDOC_TABS_LITERAL: Ctx = Ctx::new(&[
    P(&p(heredoc_end_spaces::<true>)
        .nocache()
        .caps(&[(1, &[HeredocSpaces])])
        .pop()),
    P(&p(heredoc_end::<true>)
        .nocache()
        .caps(&[(1, &[HeredocToken])])
        .pop()),
])
.meta(&[Heredoc])
.backref(4);

static HEREDOC_EXPANSION: Ctx = Ctx::new(&[
    P(&p(dq_escape).sc(&[BacktickEscape])),
    I(&EXPANSION_PARAMETER),
    I(&EXPANSION_ARITHMETIC),
    I(&EXPANSION_COMMAND),
]);

/// The rest of the line a here-document starts on, before its body.
static HEREDOC_PREAMBLE: Ctx = Ctx::new(&[P(&p(empty).set(&[&HEREDOC_LINE]))]).sh();

static HEREDOC_LINE: Ctx = Ctx::new(&[
    P(&p(eol).pop()),
    P(&p(ws_then_non_ws).push(&[&MAIN_WITH_POP_AT_END, &CMD_POST, &CMD_ARGS])),
])
.clear(1)
.sh();

static MAIN_WITH_POP_AT_END: Ctx = Ctx::new(&[I(&CONTINUATION_OR_POP), I(&MAIN)]).sh();

// `${…}`'s substitution and substring forms

/// `(?=[@*]?/)`
fn slash_ahead(l: &Line, pos: usize, _: u16) -> R {
    at_slash(l, pos, 0).map(|m| M::at(m.s(), m.s()))
}

/// `([@*])?(/)`
fn at_slash(l: &Line, pos: usize, _: u16) -> R {
    let k = find(l.b, pos, b'/')?;
    Some(if k > pos && matches!(l.b[k - 1], b'@' | b'*') {
        M::at(k - 1, k + 1).cap(1, k - 1, k).cap(2, k, k + 1)
    } else {
        M::at(k, k + 1).cap(2, k, k + 1)
    })
}

/// `[/#%]`
fn switch(l: &Line, pos: usize, _: u16) -> R {
    any_of(l, pos, b"/#%")
}

/// `(?=@?:)`
fn substring_ahead(l: &Line, pos: usize, _: u16) -> R {
    at_colon(l, pos, 0).map(|m| M::at(m.s(), m.s()))
}

/// `(@)?(:)`
fn at_colon(l: &Line, pos: usize, _: u16) -> R {
    let k = find(l.b, pos, b':')?;
    Some(if k > pos && l.b[k - 1] == b'@' {
        M::at(k - 1, k + 1).cap(1, k - 1, k).cap(2, k, k + 1)
    } else {
        M::at(k, k + 1).cap(2, k, k + 1)
    })
}

static PARAMETER_SLASH: Ctx = Ctx::new(&[P(&p(at_slash)
    .caps(&[(1, &[LangVar]), (2, &[Substitution])])
    .set(&[&PARAMETER_SLASH_FIRST]))])
.content(&[Parameter])
.sh();

static PARAMETER_SLASH_FIRST: Ctx = Ctx::new(&[
    P(&p(switch).sc(&[Switch]).set(&[&PARAMETER_SUBSTITUTE])),
    P(&p(empty).set(&[&PARAMETER_SUBSTITUTE])),
])
.content(&[Parameter]);

/// `expansion-parameter-pattern`
static PARAMETER_SUBSTITUTE: Ctx = Ctx::new(&[
    P(&p(byte::<b'/'>)
        .sc(&[Substitution])
        .set(&[&PARAMETER_COMMON])),
    I(&PARAMETER_COMMON),
    I(&EXPANSION_PATTERN),
])
.content(&[Parameter])
.sh();

static PARAMETER_SUBSTRING: Ctx = Ctx::new(&[P(&p(at_colon)
    .caps(&[(1, &[LangVar]), (2, &[SubstringBegin])])
    .set(&[&PARAMETER_OFFSET]))])
.content(&[Parameter])
.sh();

static PARAMETER_OFFSET: Ctx = Ctx::new(&[
    P(&p(ahead::<b':'>).set(&[&PARAMETER_LENGTH_COLON])),
    I(&EXPRESSION),
    I(&PARAMETER_COMMON),
])
.content(&[Parameter])
.sh();

static PARAMETER_LENGTH_COLON: Ctx = Ctx::new(&[P(&p(byte::<b':'>)
    .sc(&[SubstringEnd])
    .set(&[&PARAMETER_LENGTH]))])
.content(&[Parameter])
.sh();

static PARAMETER_LENGTH: Ctx = Ctx::new(&[I(&EXPRESSION), I(&PARAMETER_COMMON)])
    .content(&[Parameter])
    .sh();
