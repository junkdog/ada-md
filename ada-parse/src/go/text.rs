//! Go's text: strings (`"…"`, `` `…` ``) with their escapes, fmt's verbs and text/template's
//! actions, runes, comments with `//go:` directives, numbers, and the SQL and regexes in strings
//! ([`crate::sql`], [`crate::regex`]).

use super::*;
use crate::prim::find_in;

/// Where a `\` escape at `p` in a string or a rune quoted by `quote` ends, and whether Go has
/// it: `\a \b \f \n \r \t \v \\`, the quote, `\xHH`, `\uHHHH`, `\UHHHHHHHH`, `\OOO`. One that
/// Go rejects takes the `\` and the character after it.
pub(super) fn escape(b: &[u8], p: usize, quote: u8) -> (usize, bool) {
    let Some(&c) = b.get(p + 1) else {
        return (p + 1, false);
    };
    let run = |from: usize, k: usize, ok: fn(&u8) -> bool| {
        (b.len() >= from + k && b[from..from + k].iter().all(ok)).then_some(from + k)
    };
    let fixed = |r: Option<usize>| r.map_or((p + 2, false), |e| (e, true));
    match c {
        b'a' | b'b' | b'f' | b'n' | b'r' | b't' | b'v' | b'\\' => (p + 2, true),
        b'x' => fixed(run(p + 2, 2, u8::is_ascii_hexdigit)),
        b'u' => fixed(run(p + 2, 4, u8::is_ascii_hexdigit)),
        b'U' => fixed(run(p + 2, 8, u8::is_ascii_hexdigit)),
        b'0'..=b'7' => fixed(run(p + 1, 3, |c| matches!(c, b'0'..=b'7'))),
        _ if c == quote => (p + 2, true),
        _ => ((p + 1 + char_len(c)).min(b.len()), false),
    }
}

/// A fmt verb at `p`, a `%`, as fmt reads one: `%%`, or flags (`+-# 0`), an argument index
/// (`[n]`), a width and a precision (digits, `*`, `[n]*`), and a verb fmt has. Where it ends,
/// and whether it is `%%`. A verb after a space flag followed by a letter is prose ("50% off"),
/// not one.
pub(super) fn verb(b: &[u8], p: usize) -> Option<(usize, bool)> {
    let n = b.len();
    let mut q = p + 1;
    match b.get(q) {
        Some(b'%') => return Some((q + 1, true)),
        None => return None,
        _ => {}
    }
    let mut space = false;
    while let Some(&c) = b.get(q) {
        match c {
            b'+' | b'-' | b'#' | b'0' => q += 1,
            b' ' => {
                space = true;
                q += 1;
            }
            _ => break,
        }
    }
    let index = |q: usize| -> Option<usize> {
        if b.get(q) == Some(&b'[') {
            let mut e = q + 1;
            while e < n && b[e].is_ascii_digit() {
                e += 1;
            }
            if e > q + 1 && b.get(e) == Some(&b']') {
                return Some(e + 1);
            }
        }
        None
    };
    let amount = |mut q: usize| -> usize {
        if let Some(e) = index(q) {
            q = e;
        }
        if b.get(q) == Some(&b'*') {
            return q + 1;
        }
        while q < n && b[q].is_ascii_digit() {
            q += 1;
        }
        q
    };
    q = amount(q);
    if b.get(q) == Some(&b'.') {
        q = amount(q + 1);
    }
    if let Some(e) = index(q) {
        q = e;
    }
    let c = *b.get(q)?;
    if !b"vTtbcdoOqxXUeEfFgGspw".contains(&c) {
        return None;
    }
    if space && b.get(q + 1).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    Some((q + 1, false))
}

/// Where the Go string whose text runs from `p` ends on its line: its quote (an unescaped `"`,
/// or `` ` ``), or the line's end.
pub(super) fn string_end(b: &[u8], mut p: usize, raw: bool) -> usize {
    if raw {
        let s = p.min(b.len());
        return s + b[s..]
            .iter()
            .position(|&c| c == b'`')
            .unwrap_or(b.len() - s);
    }
    while p < b.len() {
        match b[p] {
            b'"' => return p,
            b'\\' => p += 2,
            _ => p += 1,
        }
    }
    b.len()
}

/// Where a template's action from `p` ends before `end`, the Go string's end: its `}}`, past the
/// template's own strings (`"…"` in a raw Go string, `\"…\"` and `` `…` `` in a `"…"` one).
fn template_end(b: &[u8], mut p: usize, end: usize, raw: bool) -> Option<usize> {
    let skip = |from: usize, quote: &[u8]| -> usize {
        let mut q = from;
        while q < end {
            if b[q..end].starts_with(quote) {
                return q + quote.len();
            }
            q += if b[q] == b'\\' && quote != b"`" { 2 } else { 1 };
        }
        end
    };
    while p + 1 < end {
        match b[p] {
            b'}' if b[p + 1] == b'}' => return Some(p),
            b'"' if raw => p = skip(p + 1, b"\""),
            b'\\' if !raw && b[p + 1] == b'"' => p = skip(p + 2, b"\\\""),
            b'`' if !raw => p = skip(p + 1, b"`"),
            _ => p += 1,
        }
    }
    None
}

/// The bytes a string's patterns start on: `"…"`'s (its end, an escape), then `` `…` ``'s.
static YIELDS: [[bool; 256]; 2] = {
    let mut t = [[false; 256]; 2];
    t[0][b'"' as usize] = true;
    t[0][b'\\' as usize] = true;
    t[1][b'`' as usize] = true;
    t
};

/// A regex in a `"…"` string, then in a `` `…` `` one.
static RE_EMBEDS: [regex::Embed<'static>; 2] = [
    regex::Embed {
        dialect: regex::Dialect::Re2,
        cooked: true,
        yields: &YIELDS[0],
        stop: |b, p, _| b[p] == b'"',
        arg: 0,
        escape: |b, p| escape(b, p, b'"').0,
        top_only: false,
        continues: false,
        base: RE_STRING,
        text: DOUBLE,
    },
    regex::Embed {
        dialect: regex::Dialect::Re2,
        cooked: false,
        yields: &YIELDS[1],
        stop: |b, p, _| b[p] == b'`',
        arg: 0,
        escape: |_, p| p + 1,
        top_only: false,
        continues: false,
        base: RE_RAW,
        text: OTHER,
    },
];

/// Whether the string whose quote opens at `p` is a regex: the first argument of
/// `regexp.MustCompile(`, `regexp.Compile(`, `regexp.MatchString(`, `regexp.Match(`,
/// `regexp.MatchReader(` or their `POSIX` twins, looked back on its line.
fn regex_string(b: &[u8], p: usize) -> bool {
    let q = crate::trim_end(b, 0, p);
    if q == 0 || b[q - 1] != b'(' {
        return false;
    }
    let w = crate::trim_end(b, 0, q - 1);
    // the name's last byte first: most calls end in none of these
    let names: &[&[u8]] = match w.checked_sub(1).map(|i| b[i]) {
        Some(b'e') => &[b"regexp.MustCompile", b"regexp.Compile"],
        Some(b'X') => &[b"regexp.MustCompilePOSIX", b"regexp.CompilePOSIX"],
        Some(b'g') => &[b"regexp.MatchString"],
        Some(b'h') => &[b"regexp.Match"],
        Some(b'r') => &[b"regexp.MatchReader"],
        _ => return false,
    };
    names.iter().any(|name| {
        b[..w].ends_with(name) && (w == name.len() || !crate::prim::is_word(b, w - name.len() - 1))
    })
}

/// The method `name` (after a `.`) whose arguments hold SQL, as GoLand reads them: the
/// state its first argument starts in. `database/sql`'s and pgx's `Exec`, `Query`, `QueryRow`,
/// `Prepare` (a context first, optionally) and their `…Context` forms, pgx's `Queue` and
/// `ExecParams`, sqlx's `MustExec`, `Queryx`, `NamedExec`, `Rebind`…, and `Get`, `Select` after
/// a destination.
pub(super) fn sql_call(name: &[u8]) -> Option<u8> {
    // the last byte first: most calls end in none of these
    match *name.last()? {
        b'c' => matches!(name, b"Exec" | b"MustExec" | b"NamedExec").then_some(S_LEAD),
        b'y' => matches!(name, b"Query" | b"NamedQuery").then_some(S_LEAD),
        b'w' => (name == b"QueryRow").then_some(S_LEAD),
        b'x' => matches!(name, b"Queryx" | b"QueryRowx" | b"Preparex").then_some(S_LEAD),
        b'e' => match name {
            b"Queue" => Some(S_LEAD),
            b"Prepare" => Some(S_LEAD_NAME),
            _ => None,
        },
        b'd' => matches!(name, b"Rebind" | b"PrepareNamed").then_some(S_LEAD),
        b's' => (name == b"ExecParams").then_some(S_LEAD),
        b't' => match name {
            b"Get" | b"Select" => Some(S_DEST),
            b"GetContext" | b"SelectContext" => Some(S_CTX_DEST),
            b"ExecContext"
            | b"QueryContext"
            | b"QueryRowContext"
            | b"PrepareContext"
            | b"MustExecContext"
            | b"NamedExecContext"
            | b"NamedQueryContext"
            | b"QueryxContext"
            | b"QueryRowxContext"
            | b"PreparexContext"
            | b"PrepareNamedContext" => Some(S_CTX),
            _ => None,
        },
        _ => None,
    }
}

/// Whether the method at `p` is on a receiver named as a prepared statement's (`stmt.Exec(`,
/// `insertStatement.Get(`, ASCII case ignored), whose arguments are the SQL's, not SQL.
fn stmt_receiver(b: &[u8], p: usize) -> bool {
    if p == 0 || b[p - 1] != b'.' {
        return false;
    }
    let e = p - 1;
    let mut s = e;
    while s > 0 && (b[s - 1].is_ascii_alphanumeric() || b[s - 1] == b'_') {
        s -= 1;
    }
    let r = b[s..e].to_ascii_lowercase();
    r.windows(4).any(|w| w == b"stmt") || r.windows(9).any(|w| w == b"statement")
}

/// Whether the argument at `p` names a context: a dotted name holding `ctx` or `context`
/// (ASCII case ignored), as `ctx`, `context.Background()`, `r.Context()`.
fn is_context(b: &[u8], p: usize) -> bool {
    let mut e = p;
    while e < b.len() && (b[e].is_ascii_alphanumeric() || matches!(b[e], b'_' | b'.')) {
        e += 1;
    }
    let w = b[p..e].to_ascii_lowercase();
    w.windows(3).any(|x| x == b"ctx") || w.windows(7).any(|x| x == b"context")
}

/// text/template's keywords.
fn template_keyword(w: &[u8]) -> bool {
    matches!(
        w,
        b"if"
            | b"else"
            | b"range"
            | b"template"
            | b"with"
            | b"end"
            | b"nil"
            | b"define"
            | b"block"
            | b"break"
            | b"continue"
    )
}

/// text/template's predeclared functions.
fn template_function(w: &[u8]) -> bool {
    matches!(
        w,
        b"and"
            | b"call"
            | b"html"
            | b"index"
            | b"slice"
            | b"js"
            | b"len"
            | b"not"
            | b"or"
            | b"print"
            | b"printf"
            | b"println"
            | b"urlquery"
            | b"eq"
            | b"ne"
            | b"lt"
            | b"le"
            | b"gt"
            | b"ge"
    )
}

/// Where an ASCII word (`[A-Za-z0-9_]`) from `p` ends.
fn ascii_word_end(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && (b[p].is_ascii_alphanumeric() || b[p] == b'_') {
        p += 1;
    }
    p
}

/// A run of `ok`'s digits at `p`, with `_` between them: `d+(?:_d+)*` (and with `lead`,
/// `_?d+(?:_d+)*`). Where it ends.
fn digits(b: &[u8], p: usize, lead: bool, ok: fn(u8) -> bool) -> Option<usize> {
    let is = |q: usize| b.get(q).is_some_and(|&c| ok(c));
    let mut q = p;
    if lead && b.get(q) == Some(&b'_') && is(q + 1) {
        q += 1;
    }
    if !is(q) {
        return None;
    }
    loop {
        while is(q) {
            q += 1;
        }
        if b.get(q) == Some(&b'_') && is(q + 1) {
            q += 1;
        } else {
            return Some(q);
        }
    }
}

fn dec_digits(b: &[u8], p: usize) -> Option<usize> {
    digits(b, p, false, |c| c.is_ascii_digit())
}

fn hex_digits(b: &[u8], p: usize) -> Option<usize> {
    digits(b, p, true, |c| c.is_ascii_hexdigit())
}

fn oct_digits(b: &[u8], p: usize) -> Option<usize> {
    digits(b, p, true, |c| matches!(c, b'0'..=b'7'))
}

fn bin_digits(b: &[u8], p: usize) -> Option<usize> {
    digits(b, p, true, |c| matches!(c, b'0' | b'1'))
}

/// `[eE][-+]?{{dec_digits}}` (or with `[pP]`) at `p`: where it ends.
fn exponent(b: &[u8], p: usize, marks: [u8; 2]) -> Option<usize> {
    if !b.get(p).is_some_and(|c| marks.contains(c)) {
        return None;
    }
    dec_digits(b, p + 1).or_else(|| {
        matches!(b.get(p + 1), Some(b'-' | b'+'))
            .then(|| dec_digits(b, p + 2))
            .flatten()
    })
}

/// `0[xX]` at `p` (or `[oO]`, `[bB]`).
fn base(b: &[u8], p: usize, mark: u8) -> bool {
    b.get(p) == Some(&b'0') && b.get(p + 1).is_some_and(|c| c.to_ascii_lowercase() == mark)
}

/// A number at `p`: its end, its leaf, and its captures as `(start, end, leaf)`, in order.
type Number = (usize, u16, [(usize, usize, u16); 3]);

/// A number at `p` (a digit, or `.` before one), as Go's spec reads one, in bat's leaves:
/// imaginary, float and integer literals of each base, an old octal's `0`, an invalid `08`.
fn number_at(b: &[u8], p: usize) -> Option<Number> {
    const NONE: (usize, usize, u16) = (0, 0, PLAIN);
    let is = |q: usize, c: u8| b.get(q) == Some(&c);
    // `{{dec_digits}}(?:(\.){{dec_digits}}?)?{{dec_exponent}}?(i)`
    if let Some(d) = dec_digits(b, p) {
        let (f, dot) = if is(d, b'.') {
            (dec_digits(b, d + 1).unwrap_or(d + 1), Some(d))
        } else {
            (d, None)
        };
        let x = exponent(b, f, *b"eE").unwrap_or(f);
        if is(x, b'i') {
            let dot = dot.map_or(NONE, |d| (d, d + 1, IMAG_DEC + 1));
            return Some((x + 1, IMAG_DEC, [dot, (x, x + 1, IMAG_DEC + 2), NONE]));
        }
    }
    let hex = base(b, p, b'x');
    // `(0[xX]){{hex_digits}}?(?:(\.){{hex_digits}}?)?{{hex_exponent}}?(i)`
    if hex {
        let h = hex_digits(b, p + 2).unwrap_or(p + 2);
        let (f, dot) = if is(h, b'.') {
            (hex_digits(b, h + 1).unwrap_or(h + 1), Some(h))
        } else {
            (h, None)
        };
        let e = exponent(b, f, *b"pP").unwrap_or(f);
        if is(e, b'i') {
            let dot = dot.map_or(NONE, |d| (d, d + 1, IMAG_HEX + 2));
            return Some((
                e + 1,
                IMAG_HEX,
                [(p, p + 2, IMAG_HEX + 1), dot, (e, e + 1, IMAG_HEX + 3)],
            ));
        }
    }
    // `(0[oO]){{oct_digits}}(i)`, `(0[bB]){{bin_digits}}(i)`
    for (mark, leaf, run) in [
        (
            b'o',
            IMAG_OCT,
            oct_digits as fn(&[u8], usize) -> Option<usize>,
        ),
        (b'b', IMAG_BIN, bin_digits),
    ] {
        if base(b, p, mark)
            && let Some(e) = run(b, p + 2)
            && is(e, b'i')
        {
            return Some((
                e + 1,
                leaf,
                [(p, p + 2, leaf + 1), (e, e + 1, leaf + 2), NONE],
            ));
        }
    }
    // decimal floats: `{{dec_digits}}(?:(\.){{dec_digits}}?{{dec_exponent}}?|{{dec_exponent}})`,
    // `(\.){{dec_digits}}{{dec_exponent}}?`
    if let Some(d) = dec_digits(b, p) {
        if is(d, b'.') {
            let f = dec_digits(b, d + 1).unwrap_or(d + 1);
            let x = exponent(b, f, *b"eE").unwrap_or(f);
            return Some((x, FLOAT_DEC, [(d, d + 1, FLOAT_DEC + 1), NONE, NONE]));
        }
        if let Some(x) = exponent(b, d, *b"eE") {
            return Some((x, FLOAT_DEC, [NONE; 3]));
        }
    } else if is(p, b'.')
        && let Some(f) = dec_digits(b, p + 1)
    {
        let x = exponent(b, f, *b"eE").unwrap_or(f);
        return Some((x, FLOAT_DEC, [(p, p + 1, FLOAT_DEC + 1), NONE, NONE]));
    }
    // `(0[xX]){{hex_digits}}?(?:(\.){{hex_digits}}?)?{{hex_exponent}}`
    if hex {
        let h = hex_digits(b, p + 2).unwrap_or(p + 2);
        let (f, dot) = if is(h, b'.') {
            (hex_digits(b, h + 1).unwrap_or(h + 1), Some(h))
        } else {
            (h, None)
        };
        if let Some(x) = exponent(b, f, *b"pP") {
            let dot = dot.map_or(NONE, |d| (d, d + 1, FLOAT_HEX + 2));
            return Some((x, FLOAT_HEX, [(p, p + 2, FLOAT_HEX + 1), dot, NONE]));
        }
    }
    // `(0){{oct_digits}}(?=\D)`, backtracking to a `_` where the run goes on in a digit
    if is(p, b'0')
        && let Some(e) = oct_digits(b, p + 1)
    {
        let not_digit = |q: usize| !b.get(q).is_some_and(u8::is_ascii_digit);
        let e = if not_digit(e) {
            Some(e)
        } else {
            (p + 2..e).rev().find(|&q| b[q] == b'_' && b[q - 1] != b'_')
        };
        if let Some(e) = e {
            return Some((e, INT_OCT, [(p, p + 1, INT_OCT + 1), NONE, NONE]));
        }
    }
    // `0[0-7]*[8-9]+`
    if is(p, b'0') {
        let o = (p + 1..=b.len())
            .find(|&q| !matches!(b.get(q), Some(b'0'..=b'7')))
            .unwrap_or(b.len());
        if matches!(b.get(o), Some(b'8' | b'9')) {
            let e = (o..=b.len())
                .find(|&q| !matches!(b.get(q), Some(b'8' | b'9')))
                .unwrap_or(b.len());
            return Some((e, INVALID, [NONE; 3]));
        }
    }
    // `(0[oO]){{oct_digits}}`, `(0[xX]){{hex_digits}}`, `(0[bB]){{bin_digits}}`
    for (mark, leaf, run) in [
        (
            b'o',
            INT_OCT,
            oct_digits as fn(&[u8], usize) -> Option<usize>,
        ),
        (b'x', INT_HEX, hex_digits),
        (b'b', INT_BIN, bin_digits),
    ] {
        if base(b, p, mark)
            && let Some(e) = run(b, p + 2)
        {
            return Some((e, leaf, [(p, p + 2, leaf + 1), NONE, NONE]));
        }
    }
    dec_digits(b, p).map(|e| (e, INT_DEC, [NONE; 3]))
}

impl<K: Sink> Scan<'_, '_, K> {
    /// A number at `p`: a decimal integer at once, else [`number_at`]'s reading.
    #[inline(always)]
    pub(super) fn number(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        self.prev = P_VALUE;
        self.role = R_CODE;
        let mut e = p;
        while e < n && b[e].is_ascii_digit() {
            e += 1;
        }
        if e > p
            && (b[p] != b'0' || e == p + 1)
            && !matches!(
                b.get(e),
                Some(
                    b'.' | b'e'
                        | b'E'
                        | b'i'
                        | b'_'
                        | b'x'
                        | b'X'
                        | b'o'
                        | b'O'
                        | b'b'
                        | b'B'
                        | b'p'
                        | b'P'
                )
            )
        {
            self.put_hot(p, e, INT_DEC);
            return e;
        }
        self.number_rest(p)
    }

    /// [`Scan::number`] past its decimal integer's: any of Go's literals, its captures.
    #[inline(never)]
    fn number_rest(&mut self, p: usize) -> usize {
        let Some((end, kind, caps)) = number_at(self.b, p) else {
            // `.` before a digit always reads; a digit always does
            self.put(p, p + 1, INT_DEC);
            return p + 1;
        };
        let mut at = p;
        for (s, e, leaf) in caps {
            if s < e {
                self.put(at, s, kind);
                self.put(s, e, leaf);
                at = e;
            }
        }
        self.put(at, end, kind);
        end
    }

    /// A `"` at `p`: a string, its text SQL's or a regex's where the call or marker around it
    /// says so.
    #[inline(always)]
    pub(super) fn dquote(&mut self, p: usize) -> usize {
        self.put_hot(p, p + 1, DOUBLE + BEGIN);
        self.prev = P_VALUE;
        self.role = R_CODE;
        if !SOL {
            if self.embedded(p) {
                return self.sql_text(p + 1, false);
            }
            if p > 0 && self.b[p - 1] == b'(' && regex_string(self.b, p) {
                self.ctx = 0;
                return self.regex_text(p + 1, false);
            }
        }
        self.string(p + 1)
    }

    /// A `` ` `` at `p`: a raw string, SQL's or a regex's as [`Scan::dquote`]'s.
    #[inline(always)]
    pub(super) fn bquote(&mut self, p: usize) -> usize {
        self.put_hot(p, p + 1, OTHER + BEGIN);
        self.prev = P_VALUE;
        self.role = R_CODE;
        if !SOL {
            if self.embedded(p) {
                return self.sql_text(p + 1, true);
            }
            if p > 0 && self.b[p - 1] == b'(' && regex_string(self.b, p) {
                self.ctx = 0;
                return self.regex_text(p + 1, true);
            }
        }
        self.raw(p + 1)
    }

    /// Whether the string at `p` is SQL: a SQL call's argument, or after a marker. SQL's
    /// context starts afresh.
    #[inline(always)]
    fn embedded(&mut self, p: usize) -> bool {
        if (self.sqls == S_SQL && self.top() == F_SQLCALL) || (p >= self.mark && self.marked_at(p))
        {
            self.mark = usize::MAX;
            self.ctx = sql::MAIN;
            return true;
        }
        false
    }

    /// Whether the SQL marker still holds at `p`; it's dropped if not.
    #[cold]
    fn marked_at(&mut self, p: usize) -> bool {
        let holds = sql::marker_holds(self.b, self.mark, p);
        if !holds {
            self.mark = usize::MAX;
        }
        holds
    }

    /// A raw string's rest at a line's start: plain, SQL's or a regex's by the mode.
    #[cold]
    pub(super) fn raw_rest(&mut self) -> usize {
        match self.mode {
            M_RAW_SQL => self.sql_text(0, true),
            M_RAW_RE => self.regex_text(0, true),
            _ => self.raw(0),
        }
    }

    /// A `"…"` string's text from `p`, its escapes, fmt's verbs and templates, to its quote or
    /// the line's end, where Go's `"…"` strings end.
    #[inline(always)]
    pub(super) fn string(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        loop {
            let q = find_in(b, p, n, *b"\"\\%{");
            if q >= n {
                self.put_hot(p, n, DOUBLE);
                return n;
            }
            match b[q] {
                b'"' => {
                    self.put_hot(p, q, DOUBLE);
                    self.put_hot(q, q + 1, DOUBLE + END);
                    return q + 1;
                }
                b'\\' => {
                    let (e, ok) = escape(b, q, b'"');
                    self.put_hot(p, q, DOUBLE);
                    self.put(q, e, if ok { DOUBLE + ESCAPE } else { STRING_INVALID });
                    p = e;
                }
                b'%' => p = self.percent(p, q, DOUBLE),
                _ => p = self.brace(p, q, false),
            }
        }
    }

    /// A raw string's text from `p`, fmt's verbs and templates, to its `` ` `` or the line's end,
    /// where it goes on ([`M_RAW`]).
    pub(super) fn raw(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        loop {
            let q = find_in(b, p, n, *b"`%{");
            if q >= n {
                self.put_hot(p, n, OTHER);
                self.mode = M_RAW;
                return n;
            }
            match b[q] {
                b'`' => {
                    self.put_hot(p, q, OTHER);
                    self.put_hot(q, q + 1, OTHER + END);
                    self.mode = M_CODE;
                    return q + 1;
                }
                b'%' => p = self.percent(p, q, OTHER),
                _ => p = self.brace(p, q, true),
            }
        }
    }

    /// A `%` at `q` in a string of `text` from `p`: a verb, `%%`, or the string's text. Where the
    /// text goes on.
    #[inline(always)]
    fn percent(&mut self, p: usize, q: usize, text: u16) -> usize {
        match verb(self.b, q) {
            Some((e, esc)) => {
                self.put_hot(p, q, text);
                self.put(q, e, text + if esc { ESCAPE } else { PLACEHOLDER });
                e
            }
            None => {
                self.put_hot(p, q + 1, text);
                q + 1
            }
        }
    }

    /// A `{` at `q` in a string from `p`: a template's `{{ … }}` where it closes in the string,
    /// else the string's text. Where the text goes on.
    #[inline(always)]
    fn brace(&mut self, p: usize, q: usize, raw: bool) -> usize {
        if !SOL
            && self.b.get(q + 1) == Some(&b'{')
            && let Some(e) = self.template(p, q, raw)
        {
            return e;
        }
        self.put_hot(p, q + 1, if raw { OTHER } else { DOUBLE });
        q + 1
    }

    /// text/template's `{{ … }}` at `q` in a string from `p`, where its `}}` comes before the
    /// string's end on the line: its actions as bat's grammar colors them, the string's scope
    /// cleared. In a `"…"` string a template's own strings are quoted `\"`; neither runs past the
    /// Go string. Where the string's text goes on; `None` (nothing written) for a `{{` that is
    /// text.
    #[cold]
    #[inline(never)]
    fn template(&mut self, p: usize, q: usize, raw: bool) -> Option<usize> {
        let b = self.b;
        let te = template_end(b, q + 2, string_end(b, q, raw), raw)?;
        self.put(p, q, if raw { OTHER } else { DOUBLE });
        self.put(q, q + 2, INTERP_BEGIN);
        let mut r = q + 2;
        while r < te {
            r = self.template_token(r, te, raw);
        }
        self.put(te, te + 2, INTERP_END);
        Some(te + 2)
    }

    /// A template's token at `r`, before its `}}` at `te`: where it ends.
    fn template_token(&mut self, r: usize, te: usize, raw: bool) -> usize {
        let b = self.b;
        let c = b[r];
        let space = |c: u8| matches!(c, b' ' | b'\t' | b'\r' | b'\n');
        if space(c) {
            if b.get(r + 1) == Some(&b'-') && r + 2 == te {
                self.put(r, r + 2, RIGHT_TRIM);
                return r + 2;
            }
            self.put(r, r + 1, INTERP_TEXT);
            return r + 1;
        }
        let one = |s: &mut Self, k: u16| {
            s.put(r, r + 1, k);
            r + 1
        };
        match c {
            b'-' if r + 1 < te && space(b[r + 1]) => {
                self.put(r, r + 2, LEFT_TRIM);
                r + 2
            }
            b':' if b.get(r + 1) == Some(&b'=') && r + 2 <= te => {
                self.put(r, r + 2, INTERP_ASSIGN);
                r + 2
            }
            b'=' => one(self, INTERP_ASSIGN),
            b'|' => one(self, PIPE),
            b'.' | b'$' => {
                let e = ascii_word_end(b, r + 1).min(te);
                if e == r + 1 {
                    return one(self, TEMPLATE_VAR);
                }
                if c == b'.' {
                    self.put(r, r + 1, INTERP_DOT);
                    self.put(r + 1, e, INTERP_MEMBER);
                } else {
                    self.put(r, r + 1, TEMPLATE_VAR_MARK);
                    self.put(r + 1, e, TEMPLATE_VAR);
                }
                e
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let e = ascii_word_end(b, r).min(te);
                let w = &b[r..e];
                let kind = if template_keyword(w) {
                    INTERP_KEYWORD
                } else if template_function(w) {
                    INTERP_FUNCTION
                } else {
                    INTERP_TEXT
                };
                self.put(r, e, kind);
                e
            }
            b'"' if raw => self.template_string(r, 1, te, b"\""),
            b'\\' if !raw && b.get(r + 1) == Some(&b'"') => self.template_string(r, 2, te, b"\\\""),
            b'`' if !raw => self.template_string(r, 1, te, b"`"),
            b'/' if b.get(r + 1) == Some(&b'*') => {
                let e = b[r + 2..te]
                    .windows(2)
                    .position(|w| w == b"*/")
                    .map_or(te, |i| r + 2 + i + 2);
                self.put(r, r + 2, COMMENT_BEGIN);
                if e >= r + 4 && b[e - 2..e] == *b"*/" {
                    self.put(r + 2, e - 2, COMMENT);
                    self.put(e - 2, e, COMMENT_END);
                } else {
                    self.put(r + 2, e, COMMENT);
                }
                e
            }
            _ => {
                let e = (r + char_len(c)).min(te);
                self.put(r, e, INTERP_TEXT);
                e
            }
        }
    }

    /// A template's string at `r`, quoted by `quote` (`k` bytes), to its closing quote before
    /// `te` (else to `te`).
    fn template_string(&mut self, r: usize, k: usize, te: usize, quote: &[u8]) -> usize {
        let b = self.b;
        self.put(r, r + k, DOUBLE + BEGIN);
        let mut q = r + k;
        let mut at = q;
        while q < te {
            if b[q..te].starts_with(quote) {
                self.put(at, q, DOUBLE);
                self.put(q, q + k, DOUBLE + END);
                return q + k;
            }
            if b[q] == b'%'
                && let Some((e, esc)) = verb(&b[..te], q)
            {
                self.put(at, q, DOUBLE);
                self.put(q, e, DOUBLE + if esc { ESCAPE } else { PLACEHOLDER });
                q = e;
                at = e;
                continue;
            }
            if b[q] == b'\\' && quote != b"`" {
                // the `\` and the character after it, whole
                let e = b.get(q + 1).map_or(q + 1, |&c| q + 1 + char_len(c)).min(te);
                self.put(at, q, DOUBLE);
                self.put(q, e, DOUBLE + ESCAPE);
                q = e;
                at = e;
                continue;
            }
            q += 1;
        }
        self.put(at, te, DOUBLE);
        te
    }

    /// A regex's string from `p` ([`crate::regex`], RE2): the string's own patterns first (its
    /// end, an escape but `\\`, which is the regex's), the regex's between, to the string's end
    /// or the line's ([`M_RAW_RE`] for a raw one).
    #[cold]
    #[inline(never)]
    fn regex_text(&mut self, p: usize, raw: bool) -> usize {
        let b = self.b;
        let n = b.len();
        let text = if raw { OTHER } else { DOUBLE };
        let embed = &RE_EMBEDS[usize::from(raw)];
        let mut at = p;
        let mut q = p;
        while q < n {
            let c = b[q];
            if (c == b'"' && !raw) || (c == b'`' && raw) {
                self.put(at, q, text);
                self.put(q, q + 1, text + END);
                self.mode = M_CODE;
                self.ctx = 0;
                return q + 1;
            }
            if c == b'\\' && !raw && b.get(q + 1) != Some(&b'\\') {
                let (e, ok) = escape(b, q, b'"');
                self.put(at, q, text);
                self.put(q, e, if ok { text + ESCAPE } else { STRING_INVALID });
                (q, at) = (e, e);
                continue;
            }
            self.k.close(self.out);
            let r = regex::embedded(b, q, self.ctx, embed, &mut at, self.out);
            self.k.reopen(self.out, 16);
            let Some((e, st)) = r else {
                break;
            };
            self.ctx = st;
            q = if e > q { e } else { q + char_len(c) };
        }
        self.put(at, n, text);
        if raw {
            self.mode = M_RAW_RE;
        } else {
            self.mode = M_CODE;
            self.ctx = 0;
        }
        n
    }

    /// A SQL string's text from `p` ([`crate::sql`], in context [`Scan::ctx`]): the string's own
    /// patterns first at each place they can start (its end, an escape), SQL's between, the line
    /// cut at the string's end so no SQL token runs past it; at the line's end a raw one goes on
    /// ([`M_RAW_SQL`]).
    #[cold]
    #[inline(never)]
    fn sql_text(&mut self, p: usize, raw: bool) -> usize {
        let b = self.b;
        let n = b.len();
        let (text, base) = if raw {
            (OTHER, SQL_RAW)
        } else {
            (DOUBLE, SQL_STRING)
        };
        let host = sql::Embed {
            yields: &YIELDS[usize::from(raw)],
            base,
            text,
        };
        let end = string_end(b, p, raw);
        let line = &b[..end];
        self.sql.is_end = usize::MAX;
        let mut q = p;
        while q < end {
            let c = b[q];
            if c == b'\\' && !raw {
                let (e, ok) = escape(b, q, b'"');
                let e = e.min(end);
                self.put(q, e, if ok { text + ESCAPE } else { STRING_INVALID });
                q = e;
                continue;
            }
            self.sql.at = q;
            self.k.close(self.out);
            let r = sql::embedded(line, q, self.ctx, &host, &mut self.sql, self.out);
            self.k.reopen(self.out, 16);
            let Some((e, ctx)) = r else {
                break;
            };
            self.ctx = ctx;
            let e = e.min(end);
            if self.sql.at < e {
                self.put(self.sql.at, e, text);
            }
            if e > q {
                q = e;
            } else {
                // no progress: the character is the string's text
                let e = (q + char_len(c)).min(end);
                self.put(q, e, text);
                q = e;
            }
        }
        if end < n {
            self.put(end, end + 1, text + END);
            self.mode = M_CODE;
            self.ctx = 0;
            self.sql_done();
            return end + 1;
        }
        if raw {
            // SQL's turn at the line's end, where its line comments end
            self.sql.at = n;
            self.k.close(self.out);
            let r = sql::embedded(b, n, self.ctx, &host, &mut self.sql, self.out);
            self.k.reopen(self.out, 16);
            if let Some((_, ctx)) = r {
                self.ctx = ctx;
            }
            if matches!(self.ctx, sql::DASHES | sql::HASHES) {
                self.ctx = sql::MAIN;
            }
            self.mode = M_RAW_SQL;
        } else {
            self.mode = M_CODE;
            self.ctx = 0;
            self.sql_done();
        }
        n
    }

    /// A SQL call's SQL string closed: a `+` after it goes on with SQL.
    fn sql_done(&mut self) {
        if self.sqls == S_SQL && self.top() == F_SQLCALL {
            self.sqls = S_AFTER;
        }
    }

    /// A method's name `b[p..e]` before its call's `(`: that `(` opens a SQL call's arguments
    /// if the name is one of [`sql_call`]'s, not on a prepared statement.
    #[cold]
    #[inline(never)]
    pub(super) fn sql_name(&mut self, p: usize, e: usize) {
        if let Some(st) = sql_call(&self.b[p..e])
            && !stmt_receiver(self.b, p)
        {
            self.sqlcall = st;
        }
    }

    /// A SQL call's `(` at `p`: its first argument's state from its first token.
    #[cold]
    #[inline(never)]
    pub(super) fn sql_paren(&mut self, p: usize) -> usize {
        self.push(F_SQLCALL);
        self.sqls = self.sqlcall;
        self.sqlcall = S_DONE;
        self.arg_at(p + 1);
        p + 1
    }

    /// A SQL call's `,` before `p`: the next argument's state, from its first token.
    #[cold]
    #[inline(never)]
    pub(super) fn sql_comma(&mut self, p: usize) {
        self.sqls = match self.sqls & !S_PENDING {
            S_SKIP_SQL => S_SQL,
            S_SKIP_DEST => S_DEST,
            S_SKIP_NAME => S_NAME,
            _ => S_DONE,
        };
        if self.sqls != S_DONE {
            self.arg_at(p);
        }
    }

    /// An argument starting at its first token past `p`, or at the next line's.
    fn arg_at(&mut self, p: usize) {
        let b = self.b;
        let q = skip_blanks(b, p);
        if q >= b.len() || b[q..].starts_with(b"//") {
            self.sqls |= S_PENDING;
        } else {
            self.sql_arg(q);
        }
    }

    /// A SQL call's argument whose first token is at `q`: what it is by the call's shape (a
    /// string where SQL stands, a context, a destination), else the call's SQL is over.
    pub(super) fn sql_arg(&mut self, q: usize) {
        let b = self.b;
        let c = b[q];
        let quote = matches!(c, b'"' | b'`');
        self.sqls = match self.sqls {
            S_LEAD | S_LEAD_NAME | S_SQL if quote => S_SQL,
            S_LEAD if is_context(b, q) => S_SKIP_SQL,
            S_LEAD_NAME if is_context(b, q) => S_SKIP_NAME,
            S_CTX if !quote => S_SKIP_SQL,
            S_CTX_DEST if !quote => S_SKIP_DEST,
            S_DEST if c == b'&' => S_SKIP_SQL,
            S_NAME => S_SKIP_SQL,
            _ => S_DONE,
        };
    }

    /// A rune at `p` (`'`): one character or an escape, else bat's `'[^']*'` to the next quote on
    /// the line; a lone quote is plain.
    pub(super) fn rune(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        self.prev = P_VALUE;
        self.role = R_CODE;
        let q = p + 1;
        if q < n && b[q] == b'\\' {
            let (e, ok) = escape(b, q, b'\'');
            if b.get(e) == Some(&b'\'') {
                self.put(p, q, RUNE);
                self.put(q, e, if ok { RUNE_ESCAPE } else { RUNE_INVALID });
                self.put(e, e + 1, RUNE);
                return e + 1;
            }
        } else if q < n && b[q] != b'\'' {
            let e = (q + char_len(b[q])).min(n);
            if b.get(e) == Some(&b'\'') {
                self.put_hot(p, e + 1, RUNE);
                return e + 1;
            }
        }
        match b[q.min(n)..].iter().position(|&c| c == b'\'') {
            Some(i) => {
                self.put(p, q + i + 1, RUNE);
                q + i + 1
            }
            None => p + 1,
        }
    }

    /// `//` at `p`: a line comment to the line's end, a `//go:` directive's parts; `//
    /// language=SQL` marks the next string.
    #[inline(always)]
    pub(super) fn line_comment(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        if !SOL && b.get(p + 2) == Some(&b'g') && b[p + 2..].starts_with(b"go:") {
            self.annotation(p);
            return n;
        }
        self.put_hot(p, p + 2, LINE_MARK);
        self.put_hot(p + 2, n, LINE_COMMENT);
        if !SOL && sql::marker_comment(b, p + 2) {
            self.mark = n;
        }
        n
    }

    /// `//go:` at `p`: bat's `(//)(go)(:)({{ident}})?`, then each `\S+` a parameter.
    #[cold]
    #[inline(never)]
    fn annotation(&mut self, p: usize) {
        let b = self.b;
        let n = b.len();
        self.put(p, p + 2, ANN_MARK);
        self.put(p + 2, p + 4, ANN_KEYWORD);
        self.put(p + 4, p + 5, ANN_COLON);
        let mut q = p + 5;
        if q < n && (b[q].is_ascii_alphabetic() || b[q] == b'_') {
            let e = ascii_word_end(b, q);
            self.put(q, e, ANN_FUNCTION);
            q = e;
        }
        while q < n {
            let s = skip_blanks(b, q);
            self.put(q, s, ANNOTATION);
            let mut e = s;
            while e < n && !matches!(b[e], b' ' | b'\t') {
                e += 1;
            }
            self.put(s, e, ANN_PARAMETER);
            q = e;
        }
    }

    /// `/*` at `p`: a block comment; `/* language=SQL */` marks the next string.
    #[inline(never)]
    pub(super) fn comment_open(&mut self, p: usize) -> usize {
        let b = self.b;
        if !SOL && sql::marker_comment(b, p + 2) {
            self.mark = b[p + 2..]
                .windows(2)
                .position(|w| w == b"*/")
                .map_or(b.len(), |i| p + 4 + i);
        }
        self.put(p, p + 2, COMMENT_BEGIN);
        self.mode = M_COMMENT;
        self.comment_text(p + 2)
    }

    /// A block comment's line from its start: a leading `*` its mark (bat's `^\s*(\*)(?!/)`),
    /// then its text.
    pub(super) fn comment(&mut self, p: usize) -> usize {
        let b = self.b;
        let s = skip_blanks(b, p);
        if b.get(s) == Some(&b'*') && b.get(s + 1) != Some(&b'/') {
            self.put(p, s, COMMENT);
            self.put(s, s + 1, COMMENT_STAR);
            return self.comment_text(s + 1);
        }
        self.comment_text(p)
    }

    /// A block comment's text from `p`: where the code goes on, or the line's end
    /// ([`M_COMMENT`]).
    fn comment_text(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        loop {
            let q = find_in(b, p, n, *b"*");
            if q >= n {
                self.put(p, n, COMMENT);
                return n;
            }
            if b.get(q + 1) == Some(&b'/') {
                self.put(p, q, COMMENT);
                self.put(q, q + 2, COMMENT_END);
                self.mode = M_CODE;
                return q + 2;
            }
            self.put(p, q + 1, COMMENT);
            p = q + 1;
        }
    }
}
