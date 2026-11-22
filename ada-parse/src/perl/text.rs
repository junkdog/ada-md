//! Perl's text: quote-like constructs (found to their ends first, then read: escapes,
//! interpolations, a regex's or SQL's text), here-documents, POD, comments, numbers, `format`'s
//! pictures.

use super::*;
use crate::heredoc::{Lead, Tail};
use crate::prim::find_in;
use crate::regex;
use crate::sql;

// A quote-like construct ([`Scan::qk`]'s low four bits).
/// `'…'`.
pub(super) const Q_SQ: u8 = 1;
/// `"…"`.
pub(super) const Q_DQ: u8 = 2;
/// `` `…` ``.
pub(super) const Q_BT: u8 = 3;
pub(super) const Q_Q: u8 = 4;
pub(super) const Q_QQ: u8 = 5;
pub(super) const Q_QW: u8 = 6;
pub(super) const Q_QX: u8 = 7;
/// `m…`, `/…/`.
pub(super) const Q_M: u8 = 8;
pub(super) const Q_QR: u8 = 9;
/// `s`'s pattern.
pub(super) const Q_S: u8 = 10;
/// `s`'s replacement.
pub(super) const Q_SR: u8 = 11;
/// `tr`'s or `y`'s search list.
pub(super) const Q_TR: u8 = 12;
/// `tr`'s or `y`'s replacement list.
pub(super) const Q_TRR: u8 = 13;
/// A bracketed `s`'s replacement's opener next, past blanks, comments and lines.
pub(super) const Q_SGAP: u8 = 14;
/// A bracketed `tr`'s likewise.
pub(super) const Q_TRGAP: u8 = 15;
const QMASK: u8 = 15;
/// The construct's text is SQL (a DBI call's first argument).
pub(super) const QF_SQL: u8 = 0x10;
/// A regex's extended syntax: `#` a comment (its `x` flag, or a body across lines).
const QF_X: u8 = 0x20;
/// `'` its delimiter: no interpolation (`m'…'`, `s'…'…'`).
const QF_LIT: u8 = 0x40;

// POD's paragraph in [`Scan::qk`] ([`M_POD`]), its open formatting codes' depth above.
/// A paragraph starts at the next line that isn't blank.
pub(super) const POD_PARA: u8 = 1;
/// In a verbatim paragraph: no formatting codes.
const POD_VERBATIM: u8 = 2;

// What a DBI call's argument is ([`Scan::sqls`]).
/// Not SQL.
pub(super) const S_DONE: u8 = 0;
/// SQL: the first argument, its strings each SQL (`"SELECT …" . "FROM …"`).
pub(super) const S_SQL: u8 = 1;

// A here-document's flags in the queue.
/// It interpolates (`<<"EOT"`, `<<EOT`).
pub(super) const H_INTERP: u8 = 1;
/// Its closer may be indented (`<<~`).
pub(super) const H_INDENT: u8 = 2;
/// Its body is SQL.
const H_SQL: u8 = 4;
/// Its body is HTML's, CSS's, JavaScript's, JSON's or XML's: its guest's (X-191), or text where
/// Perl's fields don't fit beside the guest's state.
pub(super) const H_EMBED: u8 = 8;

/// The string class a construct's text is written in.
#[inline(always)]
const fn class_of(q: u8) -> u8 {
    match q {
        Q_SQ => C_SQ,
        Q_DQ => C_DQ,
        Q_BT => C_BT,
        Q_M | Q_QR | Q_S => C_REGEX,
        _ => C_UNQ,
    }
}

/// The opener of a closing delimiter `c` that nests, 0 for one that doesn't.
#[inline(always)]
const fn opener_of(c: u8) -> u8 {
    match c {
        b')' => b'(',
        b']' => b'[',
        b'}' => b'{',
        b'>' => b'<',
        _ => 0,
    }
}

/// The closing delimiter of opener `c`.
#[inline(always)]
const fn closer_of(c: u8) -> u8 {
    match c {
        b'(' => b')',
        b'[' => b']',
        b'{' => b'}',
        b'<' => b'>',
        c => c,
    }
}

/// Where a construct's part from `p` ends on the line, its delimiter `close` (nesting with
/// `open`, at depth `nest`): the closing delimiter's place, whether it's there, and the depth at
/// the line's end. A `\` takes the byte after it.
#[inline(always)]
fn part_end(b: &[u8], mut p: usize, close: u8, open: u8, mut nest: u8) -> (usize, bool, u8) {
    let n = b.len();
    loop {
        let q = if open != 0 {
            find_in(b, p, n, [close, open, b'\\'])
        } else {
            find_in(b, p, n, [close, b'\\'])
        };
        if q >= n {
            return (n, false, nest);
        }
        let c = b[q];
        if c == b'\\' {
            p = q + 2;
            if p >= n {
                return (n, false, nest);
            }
        } else if c == close && (open == 0 || nest <= 1) {
            return (q, true, 0);
        } else if c == close {
            nest -= 1;
            p = q + 1;
        } else {
            nest = nest.saturating_add(1);
            p = q + 1;
        }
    }
}

/// The flags after a construct's closing delimiter at `e`: where they end.
#[inline(always)]
fn flags_end(b: &[u8], e: usize) -> usize {
    let mut f = e;
    while f < b.len() && b[f].is_ascii_alphabetic() {
        f += 1;
    }
    f
}

/// Whether flags `b[f..g]` hold `c`.
#[inline(always)]
fn has_flag(b: &[u8], f: usize, g: usize, c: u8) -> bool {
    b[f..g].contains(&c)
}

/// Where Perl's escape at `q` (a `\`) in an interpolating string ends, before `end`: `\x{…}`,
/// `\N{…}`, `\o{…}`, `\xHH`, `\cX`, octal, else the `\` and the character after it.
pub(super) fn escape_end(b: &[u8], q: usize, end: usize) -> usize {
    let Some(&c) = b.get(q + 1).filter(|_| q + 1 < end) else {
        return (q + 1).min(end.max(q + 1));
    };
    let braced = |from: usize| {
        if b.get(from) == Some(&b'{') {
            let r = find_in(b, from + 1, end, *b"}");
            if r < end {
                return Some(r + 1);
            }
        }
        None
    };
    let e = match c {
        b'x' => braced(q + 2).unwrap_or_else(|| {
            let mut e = q + 2;
            while e < end && e < q + 4 && b[e].is_ascii_hexdigit() {
                e += 1;
            }
            e
        }),
        b'N' | b'o' => braced(q + 2).unwrap_or(q + 2),
        b'c' if q + 2 < end => q + 2 + char_len(b[q + 2]),
        b'0'..=b'7' => {
            let mut e = q + 2;
            while e < end && e < q + 4 && matches!(b[e], b'0'..=b'7') {
                e += 1;
            }
            e
        }
        _ => q + 1 + char_len(c),
    };
    e.min(end)
}

/// Where a `printf` placeholder at `q` (a `%`) ends before `end`, as bat reads one, or `None`:
/// `%%`, or `%` [index] [flags] [width] [vector or precision] [size] conversion.
pub(super) fn placeholder_end(b: &[u8], q: usize, end: usize) -> Option<usize> {
    let at = |i: usize| if i < end { b[i] } else { 0 };
    let mut e = q + 1;
    if at(e) == b'%' {
        return Some(e + 1);
    }
    let digits = |mut e: usize| {
        while e < end && b[e].is_ascii_digit() {
            e += 1;
        }
        e
    };
    // index: `1$`
    let d = digits(e);
    if d > e && at(d) == b'$' {
        e = d + 1;
    }
    // flags
    let mut f = e;
    while matches!(at(f), b' ' | b'\t') {
        f += 1;
    }
    if matches!(at(f), b'-' | b'+') {
        f += 1;
        while matches!(at(f), b' ' | b'\t') {
            f += 1;
        }
        e = f;
    } else if matches!(at(e), b' ' | b'#' | b'0') {
        e += 1;
    }
    // width
    if at(e) == b'*' {
        e += 1;
        let d = digits(e);
        if d > e && at(d) == b'$' {
            e = d + 1;
        }
    } else {
        e = digits(e);
    }
    // vector, or precision
    if at(e) == b'v' {
        e = digits(e + 1);
    } else if at(e) == b'.' {
        e += 1;
        if at(e) == b'*' {
            e += 1;
            let d = digits(e);
            if d > e && at(d) == b'$' {
                e = d + 1;
            }
        } else {
            e = digits(e);
        }
    }
    // size
    if (at(e) == b'h' && at(e + 1) == b'h') || (at(e) == b'l' && at(e + 1) == b'l') {
        e += 2;
    } else if matches!(at(e), b'h' | b'j' | b'l' | b'L' | b'q' | b't' | b'z') {
        e += 1;
    }
    b"aAbBcdDeEfFgGinoOpsuUxX".contains(&at(e)).then_some(e + 1)
}

/// Whether the `$` or `@` at `q` interpolates in a regex: a name, `{`, `$name`, `::`; `$`
/// before anything else is an anchor.
#[inline(always)]
fn re_interp_at(b: &[u8], q: usize) -> bool {
    let at = |i: usize| b.get(q + i).copied().unwrap_or(b'\n');
    let c1 = at(1);
    is_name_start(c1)
        || c1 == b'{'
        || (c1 == b':' && at(2) == b':')
        || (c1 == b'$' && (is_name_start(at(2)) || at(2) == b'{'))
        || (b[q] == b'$' && c1.is_ascii_digit())
}

/// The bytes a regex's host patterns start on: an interpolation's sigil, an extended syntax's
/// `#`, by whether it interpolates and is extended.
static RE_YIELDS: [[bool; 256]; 4] = {
    let mut t = [[false; 256]; 4];
    t[1][b'#' as usize] = true;
    t[2][b'$' as usize] = true;
    t[2][b'@' as usize] = true;
    t[3][b'$' as usize] = true;
    t[3][b'@' as usize] = true;
    t[3][b'#' as usize] = true;
    t
};

/// Whether the byte at `p` is escaped: an odd run of `\`s before it.
#[inline(always)]
fn escaped(b: &[u8], p: usize) -> bool {
    let mut q = p;
    while q > 0 && b[q - 1] == b'\\' {
        q -= 1;
    }
    (p - q) % 2 == 1
}

/// Whether a regex's host has a pattern at `p` (a byte of [`RE_YIELDS`]): not past a `\`, the
/// regex's escape (`\$`, `\#`).
fn re_stop(b: &[u8], p: usize, _arg: usize) -> bool {
    (b[p] == b'#' || re_interp_at(b, p)) && !escaped(b, p)
}

macro_rules! re_embed {
    ($i:literal) => {
        regex::Embed {
            dialect: regex::Dialect::Perl,
            cooked: false,
            yields: &RE_YIELDS[$i],
            stop: re_stop,
            arg: 0,
            escape: |_, p| p,
            top_only: false,
            continues: false,
            base: RE_BASE,
            text: str_leaf(C_REGEX),
        }
    };
}

/// A regex: literal or interpolating, extended or not.
static RE_EMBEDS: [regex::Embed<'static>; 4] =
    [re_embed!(0), re_embed!(1), re_embed!(2), re_embed!(3)];

/// The bytes a SQL string's own patterns start on: a `\`, a placeholder's `%` (a `sprintf`
/// format's), an interpolating one's sigils.
static SQL_YIELDS: [[bool; 256]; 2] = {
    let mut t = [[false; 256]; 2];
    t[0][b'\\' as usize] = true;
    t[0][b'%' as usize] = true;
    t[1][b'\\' as usize] = true;
    t[1][b'%' as usize] = true;
    t[1][b'$' as usize] = true;
    t[1][b'@' as usize] = true;
    t
};

/// Whether a here-document's label names SQL: `SQL`, or a label that starts or ends with it.
fn sql_label(w: &[u8]) -> bool {
    w.len() >= 3
        && (w[..3].eq_ignore_ascii_case(b"SQL") || w[w.len() - 3..].eq_ignore_ascii_case(b"SQL"))
}

/// The language a here-document's label names, as bat's tags (an index into
/// [`super::HEREDOC_TAGS`]): 0 for none. bat's labels (`HTML`, `CSS`, `JAVASCRIPT`, `JSON`,
/// `XML`), SQL's ([`sql_label`]), and ada's (X-191, [`named_lang`]).
fn label_lang(w: &[u8]) -> u8 {
    match w {
        b"HTML" => 2,
        b"CSS" => 3,
        b"JAVASCRIPT" => 4,
        b"JSON" => 5,
        b"XML" => 6,
        _ if sql_label(w) => 1,
        _ => named_lang(w),
    }
}

/// The language a label names past bat's (X-191): one that starts or ends with a language's
/// name, ASCII case aside (`EOHTML`, `HTML_START`, `XHTML`, `ENDCSS`, `EOJSON`, `PPD_XML`), or
/// `JS` as a word of it (`JS`, `EOJS`, `END_JS`, `JS_CODE`); 0 for none. Every such label in
/// X-170's corpus held its language.
fn named_lang(w: &[u8]) -> u8 {
    let at = |name: &[u8]| {
        w.len() >= name.len()
            && (w[..name.len()].eq_ignore_ascii_case(name)
                || w[w.len() - name.len()..].eq_ignore_ascii_case(name))
    };
    let js = |w: &[u8]| {
        let n = w.len();
        (n >= 3 && w[..3].eq_ignore_ascii_case(b"JS_"))
            || (n >= 3 && w[n - 3..].eq_ignore_ascii_case(b"_JS"))
            || [&b"JS"[..], b"EOJS", b"ENDJS"]
                .iter()
                .any(|l| w.eq_ignore_ascii_case(l))
    };
    if at(b"JAVASCRIPT") {
        4
    } else if at(b"JSON") {
        5
    } else if at(b"HTML") {
        2
    } else if at(b"CSS") {
        3
    } else if at(b"XML") {
        6
    } else if js(w) {
        4
    } else {
        0
    }
}

/// The language a POD format's name names (`=begin html`, `=for json`), as [`label_lang`]'s;
/// 0 for none.
fn pod_lang(w: &[u8]) -> u8 {
    let is = |l: &[u8]| w.eq_ignore_ascii_case(l);
    if is(b"html") || is(b"xhtml") {
        2
    } else if is(b"css") {
        3
    } else if is(b"javascript") || is(b"js") {
        4
    } else if is(b"json") {
        5
    } else if is(b"xml") {
        6
    } else {
        0
    }
}

/// The language of [`Scan::glang`]'s code.
fn guest_lang(glang: u8) -> crate::Lang {
    match glang & super::GL_LANG {
        2 => crate::Lang::Html,
        3 => crate::Lang::Css,
        4 => crate::Lang::JavaScript,
        5 => crate::Lang::Json,
        _ => crate::Lang::Xml,
    }
}

/// `b` as a guest reads it (X-191), into `buf`: the bytes before `from` blank, and each of
/// `isl`'s, Perl's: an escape of a punctuation byte the byte it makes (`\"` a blank and `"`,
/// `\\` a blank and `\`), the rest blank, so the guest's contexts run across them.
fn mask(b: &[u8], from: usize, isl: &[(u32, u32)], buf: &mut Vec<u8>) {
    buf.clear();
    buf.extend_from_slice(b);
    buf[..from].fill(b' ');
    for &(q, e) in isl {
        let (q, e) = (q as usize, e as usize);
        if b[q] == b'\\' && e == q + 2 && b[q + 1].is_ascii_punctuation() {
            buf[q] = b' ';
        } else {
            buf[q..e].fill(b' ');
        }
    }
}

/// Where the bracket closing the one at `p` (`open`, `close`) is before `end`, past `\`s and
/// nesting, within [`MATCH_MAX`] bytes (a line of unclosed `"$x{` stays linear); `None` if not
/// there.
fn match_close(b: &[u8], p: usize, end: usize, open: u8, close: u8) -> Option<usize> {
    let end = end.min(p + MATCH_MAX);
    let mut depth = 0u32;
    let mut q = p;
    while q < end {
        let c = b[q];
        if c == b'\\' {
            q += 2;
            continue;
        }
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(q);
            }
        }
        q += 1;
    }
    None
}

/// How far an interpolation's subscript or `${…}` is looked for its close.
const MATCH_MAX: usize = 1024;

/// What a string's interpolated code is lexed with: the scan's fields that outlive a token,
/// saved and put back around it.
#[derive(Clone, Copy)]
struct Saved {
    frames: u128,
    depth: u8,
    mode: u8,
    prev: u8,
    forvar: bool,
    qk: u8,
    qdelim: u8,
    qnest: u8,
    ctx: u8,
    pp: u8,
    sqls: u8,
    queue: Queue<QUEUED>,
    call: u8,
}

impl<K: Sink> Scan<'_, '_, K> {
    /// Whether a string opened here is a DBI call's SQL.
    #[inline(always)]
    fn sql_arg(&self) -> bool {
        !SOL && self.top() == F_SQL && self.sqls == S_SQL
    }

    /// `'`, `"` or `` ` `` at `p`: a string.
    #[inline(always)]
    pub(super) fn quote(&mut self, p: usize) -> usize {
        let c = self.b[p];
        let mut q = match c {
            b'\'' => Q_SQ,
            b'"' => Q_DQ,
            _ => Q_BT,
        };
        if q != Q_BT && self.top() == F_SQL && self.sql_arg() {
            q |= QF_SQL;
        }
        self.open_at(q, p)
    }

    /// A quote-like operator's word `b[p..e]` (`q`, `qq`, `qw`, `qx`, `m`, `qr`, `s`, `tr`,
    /// `y`): its construct from its delimiter, past blanks; a comment or the line's end before
    /// it waits for the next line; no delimiter, a bareword.
    #[inline(never)]
    pub(super) fn quote_op(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut q = match &b[p..e] {
            b"q" => Q_Q,
            b"qq" => Q_QQ,
            b"qw" => Q_QW,
            b"qx" => Q_QX,
            b"m" => Q_M,
            b"qr" => Q_QR,
            b"s" => Q_S,
            _ => Q_TR,
        };
        let r = skip_blanks(b, e);
        if r < n {
            let c = b[r];
            // bat's delimiters: not a word's byte, a blank or a closer (`$h{s}`, `(y)`)
            if is_name_byte(c) || matches!(c, b')' | b']' | b'}' | b'>') || (c == b'#' && r > e) {
                if c != b'#' {
                    return self.bareword_out(p, e);
                }
            } else {
                if matches!(q, Q_Q | Q_QQ) && self.sql_arg() {
                    q |= QF_SQL;
                }
                self.put_hot(p, e, SUPPORT);
                return self.open_at(q, r);
            }
        }
        // the delimiter on a later line
        self.put_hot(p, e, SUPPORT);
        if r < n {
            self.comment(r);
        }
        self.qk = q;
        self.mode = M_QPEND;
        self.prev = P_VALUE;
        n
    }

    /// A quote-like operator waiting for its delimiter, at a line's start: its construct from the
    /// line's first byte that isn't a blank, or a comment.
    #[cold]
    pub(super) fn quote_pending(&mut self) -> usize {
        let b = self.b;
        let n = b.len();
        let r = skip_blanks(b, 0);
        if r >= n {
            return n;
        }
        let c = b[r];
        if c == b'#' {
            return self.comment(r);
        }
        if is_name_byte(c) || matches!(c, b')' | b']' | b'}' | b'>') {
            // no construct after all
            self.qk = 0;
            self.mode = M_CODE;
            return r;
        }
        let q = self.qk;
        self.open_at(q, r)
    }

    /// A construct `q` opened by its delimiter at `d`: the delimiter written, its text read.
    #[inline(always)]
    pub(super) fn open_at(&mut self, q: u8, d: usize) -> usize {
        let b = self.b;
        let c = b[d];
        let mut q = q;
        if c == b'\'' && matches!(q & QMASK, Q_M | Q_QR | Q_S) {
            q |= QF_LIT;
        }
        self.qk = q;
        self.qdelim = closer_of(c);
        self.qnest = u8::from(opener_of(self.qdelim) != 0);
        self.ctx = if q & QF_SQL != 0 { sql::MAIN } else { 0 };
        let kind = match q & QMASK {
            Q_SQ | Q_DQ | Q_BT => str_leaf(class_of(q & QMASK)) + BEGIN,
            _ => delim_kind(c, false),
        };
        self.put_hot(d, d + 1, kind);
        self.prev = P_VALUE;
        if !SOL && matches!(q & QMASK, Q_M | Q_QR | Q_S) {
            self.regex_flags_ahead(d + 1);
        }
        self.quote_body(d + 1)
    }

    /// A regex opened at `p`: its `x` flag read ahead where the construct closes on its line,
    /// assumed where it doesn't (a body across lines is an extended one).
    #[inline(never)]
    fn regex_flags_ahead(&mut self, p: usize) {
        let b = self.b;
        let close = self.qdelim;
        let open = opener_of(close);
        let (e, closed, _) = part_end(b, p, close, open, self.qnest);
        let x = if !closed {
            true
        } else if self.qk & QMASK == Q_S {
            // past the replacement
            let mut r = e + 1;
            let (mut c2, mut o2) = (close, open);
            if open != 0 {
                r = skip_blanks(b, r);
                let Some(&c) = b.get(r) else {
                    return;
                };
                c2 = closer_of(c);
                o2 = opener_of(c2);
                r += 1;
            }
            let (e2, closed2, _) = part_end(b, r, c2, o2, u8::from(o2 != 0));
            if !closed2 {
                return;
            }
            let f = flags_end(b, e2 + 1);
            has_flag(b, e2 + 1, f, b'x')
        } else {
            let f = flags_end(b, e + 1);
            has_flag(b, e + 1, f, b'x')
        };
        if x {
            self.qk |= QF_X;
        }
    }

    /// The open construct's text from `p`: each part to its delimiter, the next part's opener,
    /// the flags; where the code goes on, or the line's end with the construct still open
    /// ([`M_QUOTE`]).
    pub(super) fn quote_body(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        loop {
            let q = self.qk & QMASK;
            if q == Q_SGAP || q == Q_TRGAP {
                // a bracketed `s{…}`'s replacement: past blanks, comments, lines
                let r = skip_blanks(b, p);
                if r >= n {
                    self.mode = M_QUOTE;
                    return n;
                }
                if b[r] == b'#' {
                    self.comment(r);
                    self.mode = M_QUOTE;
                    return n;
                }
                let c = b[r];
                if is_name_byte(c) || matches!(c, b')' | b']' | b'}' | b'>') {
                    // no replacement's opener (a word's byte, one past ASCII, a closer): the
                    // construct ends, the code goes on
                    self.qk = 0;
                    self.ctx = 0;
                    self.mode = M_CODE;
                    self.prev = P_VALUE;
                    return r;
                }
                let mut k = if q == Q_SGAP { Q_SR } else { Q_TRR };
                if c == b'\'' && q == Q_SGAP {
                    k |= QF_LIT;
                }
                self.qk = k | (self.qk & QF_X);
                self.qdelim = closer_of(c);
                self.qnest = u8::from(opener_of(self.qdelim) != 0);
                self.put(r, r + 1, delim_kind(c, false));
                p = r + 1;
                continue;
            }
            let close = self.qdelim;
            let open = opener_of(close);
            let (end, closed, nest) = part_end(b, p, close, open, self.qnest);
            self.qnest = nest;
            self.part_text(p, end, closed);
            if !closed {
                if matches!(q, Q_M | Q_QR | Q_S) {
                    // a regex across lines: its extended syntax
                    self.qk |= QF_X;
                }
                if self.qk & QF_SQL != 0 && matches!(self.ctx, sql::DASHES | sql::HASHES) {
                    // SQL's line comments end with the line
                    self.ctx = sql::MAIN;
                }
                self.mode = M_QUOTE;
                return n;
            }
            // the part's closing delimiter at `end`
            match q {
                Q_S | Q_TR if open != 0 => {
                    self.put(end, end + 1, delim_kind(open, true));
                    self.qk = if q == Q_S { Q_SGAP } else { Q_TRGAP } | (self.qk & QF_X);
                    self.ctx = 0;
                    p = end + 1;
                    continue;
                }
                Q_S | Q_TR => {
                    // one delimiter between the parts: bat's separator
                    self.put(end, end + 1, COMMA);
                    let lit = self.qk & QF_LIT;
                    self.qk = if q == Q_S { Q_SR } else { Q_TRR } | lit | (self.qk & QF_X);
                    self.qnest = 0;
                    self.ctx = 0;
                    p = end + 1;
                    continue;
                }
                Q_SQ | Q_DQ | Q_BT => {
                    self.put_hot(end, end + 1, str_leaf(class_of(q)) + END);
                }
                _ => {
                    self.put(end, end + 1, delim_kind(open, true));
                }
            }
            let mut e = end + 1;
            if matches!(q, Q_M | Q_QR | Q_SR | Q_TRR) {
                let f = flags_end(b, e);
                if f > e {
                    self.put(e, f, FLAGS);
                    e = f;
                }
            }
            self.qk = 0;
            self.ctx = 0;
            self.mode = M_CODE;
            self.prev = P_VALUE;
            return e;
        }
    }

    /// A construct's part's text `b[p..end]` by what the part is.
    #[inline(always)]
    fn part_text(&mut self, p: usize, end: usize, closed: bool) {
        let qk = self.qk;
        let q = qk & QMASK;
        let class = class_of(q);
        if SOL {
            match q {
                Q_SQ | Q_Q | Q_QW | Q_TR | Q_TRR => self.put_hot(p, end, str_leaf(class)),
                _ => self.interp_text(p, end, class, true),
            }
            return;
        }
        match q {
            Q_M | Q_QR | Q_S => self.regex_text(p, end, qk & QF_LIT != 0, qk & QF_X != 0),
            Q_SQ | Q_Q | Q_QW if qk & QF_SQL == 0 => self.literal_text(p, end, class),
            Q_SQ | Q_Q => self.sql_text(p, end, class, false),
            Q_TR | Q_TRR => self.interp_text(p, end, class, false),
            Q_SR if qk & QF_LIT != 0 => self.literal_text(p, end, class),
            Q_SR if closed && self.replacement_is_code(end) => self.code_bounded(p, end),
            _ if qk & QF_SQL != 0 => self.sql_text(p, end, class, true),
            _ => self.interp_text(p, end, class, true),
        }
    }

    /// Whether `s///`'s replacement closing at `end` is code: its flags hold `e`.
    #[inline(never)]
    fn replacement_is_code(&self, end: usize) -> bool {
        let f = flags_end(self.b, end + 1);
        has_flag(self.b, end + 1, f, b'e')
    }

    /// A literal string's text `b[p..end]` of class `class`: `\\` and a `\` before its
    /// delimiter its escapes, bat's placeholders.
    fn literal_text(&mut self, p: usize, end: usize, class: u8) {
        let b = self.b;
        let text = str_leaf(class);
        let close = self.qdelim;
        let open = opener_of(close);
        let mut at = p;
        let mut q = p;
        loop {
            q = find_in(b, q, end, *b"\\%");
            if q >= end {
                break;
            }
            if b[q] == b'\\' {
                let c = b.get(q + 1).copied().unwrap_or(0);
                if q + 1 < end && (c == b'\\' || c == close || (open != 0 && c == open)) {
                    self.put_hot(at, q, text);
                    self.put(q, q + 2, text + ESC);
                    at = q + 2;
                    q += 2;
                } else {
                    q += 1;
                }
            } else if let Some(e) = placeholder_end(b, q, end) {
                self.put_hot(at, q, text);
                self.placeholder(q, e, text);
                at = e;
                q = e;
            } else {
                q += 1;
            }
        }
        self.put_hot(at, end, text);
    }

    /// An interpolating string's text `b[p..end]` of class `class`: escapes, placeholders and,
    /// where `vars`, interpolated variables.
    pub(super) fn interp_text(&mut self, p: usize, end: usize, class: u8, vars: bool) {
        let b = self.b;
        let text = str_leaf(class);
        let mut at = p;
        let mut q = p;
        loop {
            q = if vars {
                find_in(b, q, end, *b"\\$@%")
            } else {
                find_in(b, q, end, *b"\\%")
            };
            if q >= end {
                break;
            }
            match b[q] {
                b'\\' => {
                    let e = escape_end(b, q, end).max(q + 1);
                    self.put_hot(at, q, text);
                    self.put(q, e, text + ESC);
                    at = e;
                    q = e;
                }
                b'%' => {
                    if let Some(e) = (!SOL).then(|| placeholder_end(b, q, end)).flatten() {
                        self.put_hot(at, q, text);
                        self.placeholder(q, e, text);
                        at = e;
                        q = e;
                    } else {
                        q += 1;
                    }
                }
                _ => match self.interp_var(at, q, end, text, false) {
                    Some(e) => {
                        at = e;
                        q = e;
                    }
                    None => q += 1,
                },
            }
        }
        self.put_hot(at, end, text);
    }

    /// A placeholder `b[q..e]` in a string whose text is `text`: `%%` an escape; else bat's
    /// `%` mark, its precision's `.`, the rest the placeholder's.
    fn placeholder(&mut self, q: usize, e: usize, text: u16) {
        let b = self.b;
        if b[q + 1] == b'%' {
            self.put(q, e, text + ESC);
            return;
        }
        self.put(q, q + 1, text + PH_MARK);
        let mut at = q + 1;
        if let Some(d) = b[q + 1..e].iter().position(|&c| c == b'.') {
            let d = q + 1 + d;
            self.put(at, d, text + PLACEHOLDER);
            self.put(d, d + 1, text + PH_DOT);
            at = d + 1;
        }
        self.put(at, e, text + PLACEHOLDER);
    }

    /// An interpolated variable at `q` (`$` or `@`) in a string's text before `end`, the text
    /// from `at` written before it as `text`: its sigil, name and subscripts written, where it
    /// ends; `None`, nothing written, where none starts there. `re` in a regex: `$` before
    /// anything but a name or `{` is an anchor, and a `[` or `{` after the variable a
    /// subscript only where it reads as one.
    pub(super) fn interp_var(
        &mut self,
        at: usize,
        q: usize,
        end: usize,
        text: u16,
        re: bool,
    ) -> Option<usize> {
        let b = self.b;
        let get = |i: usize| if i < end { b[i] } else { 0 };
        let s = b[q];
        let c1 = get(q + 1);
        let mut e;
        if is_name_start(c1) || (c1 == b':' && get(q + 2) == b':') {
            e = if c1 == b':' {
                qualified_end(&b[..end], q + 1)
            } else {
                qualified_end(&b[..end], name_run(&b[..end], q + 2))
            };
            if e == q + 1 {
                return None;
            }
            self.put_hot(at, q, text);
            if b[q + 1..e].contains(&b':') {
                let save = self.prev;
                let r = self.var_path_in(q, e);
                self.prev = save;
                e = r;
            } else {
                let special = special_name(b, q + 1, e, s);
                self.put_hot(q, q + 1, if special { VAR_LANG_SIGIL } else { VAR_SIGIL });
                self.put_hot(q + 1, e, if special { VAR_LANG } else { VAR });
            }
        } else if SOL {
            return None;
        } else {
            match c1 {
                b'{' => {
                    let r = match_close(b, q + 1, end, b'{', b'}')?;
                    self.put_hot(at, q, text);
                    let t = skip_blanks(b, q + 2);
                    if b.get(t) == Some(&b'^') {
                        self.put(q, q + 1, VAR_LANG_SIGIL);
                        self.put(q + 1, r + 1, VAR_LANG);
                    } else {
                        let w = if t < r && is_name_start(b[t]) {
                            name_run(&b[..r], t + 1)
                        } else {
                            t
                        };
                        self.put(q, q + 1, DEREF);
                        self.put(q + 1, q + 2, VBEGIN);
                        if w > t && skip_blanks(&b[..r], w) == r {
                            self.put(t, w, str_leaf(C_UNQ));
                        } else {
                            self.code_bounded(q + 2, r);
                        }
                        self.put(r, r + 1, VEND);
                    }
                    e = r + 1;
                }
                b'$' if is_name_start(get(q + 2)) || get(q + 2) == b'{' => {
                    // `$$ref`, `@$ref`
                    self.put_hot(at, q, text);
                    self.put(q, q + 1, DEREF);
                    return Some(
                        self.interp_var(q + 1, q + 1, end, text, re)
                            .unwrap_or(q + 1),
                    );
                }
                b'$' if s == b'$' => {
                    self.put_hot(at, q, text);
                    self.put(q, q + 1, VAR_LANG_SIGIL);
                    self.put(q + 1, q + 2, VAR_LANG);
                    e = q + 2;
                }
                b'0'..=b'9' if s == b'$' => {
                    let mut r = q + 2;
                    while r < end && b[r].is_ascii_digit() {
                        r += 1;
                    }
                    self.put_hot(at, q, text);
                    if c1 == b'0' && r == q + 2 {
                        self.put(q, q + 1, VAR_LANG_SIGIL);
                        self.put(q + 1, r, VAR_LANG);
                    } else {
                        self.put(q, q + 1, GROUP_SIGIL);
                        self.put(q + 1, r, GROUP_VAR);
                    }
                    e = r;
                }
                _ if re || s != b'$' => return None,
                b'&' | b'`' | b'\'' => {
                    self.put_hot(at, q, text);
                    self.put(q, q + 1, MATCH_SIGIL);
                    self.put(q + 1, q + 2, MATCH_VAR);
                    return Some(q + 2);
                }
                b'^' if get(q + 2).is_ascii_uppercase() => {
                    self.put_hot(at, q, text);
                    self.put(q, q + 1, VAR_LANG_SIGIL);
                    self.put(q + 1, q + 3, VAR_LANG);
                    return Some(q + 3);
                }
                b'.' | b',' | b';' | b'!' | b'/' | b'\\' | b'|' | b'(' | b')' | b'<' | b'>'
                | b':' | b'?' | b'~' | b']' | b'"' | b'@'
                    if !is_name_byte(get(q + 2)) && !matches!(get(q + 2), b'$' | b'@' | b'%') =>
                {
                    self.put_hot(at, q, text);
                    self.put(q, q + 1, VAR_LANG_SIGIL);
                    self.put(q + 1, q + 2, VAR_LANG);
                    return Some(q + 2);
                }
                _ => return None,
            }
        }
        if SOL {
            return Some(e);
        }
        Some(self.interp_subscripts(e, end, re))
    }

    /// A variable's path `$Foo::x` from `q` to `e` in a string: its packages and name.
    fn var_path_in(&mut self, q: usize, e: usize) -> usize {
        let b = self.b;
        let mut last = e;
        while last > q + 1 && is_name_byte(b[last - 1]) {
            last -= 1;
        }
        self.put(q, q + 1, VAR_SIGIL);
        let mut r = q + 1;
        while r < last {
            if b[r] == b':' {
                self.put(r, r + 2, DCOLON);
                r += 2;
            } else {
                let w = name_run(&b[..last], r + 1);
                self.put(r, w, NS);
                r = w;
            }
        }
        if last < e {
            self.put(last, e, VAR);
        }
        e
    }

    /// An interpolated variable's subscripts from `e` before `end`: `[…]`, `{…}`, `->[…]`,
    /// `->{…}`, chained; where they end.
    fn interp_subscripts(&mut self, mut e: usize, end: usize, re: bool) -> usize {
        let b = self.b;
        let get = |i: usize| if i < end { b[i] } else { 0 };
        loop {
            match get(e) {
                b'[' => {
                    if re && !index_like(b, e + 1, end) {
                        return e;
                    }
                    let Some(r) = match_close(b, e, end, b'[', b']') else {
                        return e;
                    };
                    self.put(e, e + 1, ITEM_B);
                    self.code_bounded(e + 1, r);
                    self.put(r, r + 1, ITEM_E);
                    e = r + 1;
                }
                b'{' => {
                    if re && quantifier_like(b, e + 1, end) {
                        return e;
                    }
                    let Some(r) = match_close(b, e, end, b'{', b'}') else {
                        return e;
                    };
                    self.put(e, e + 1, ITEM_B);
                    let t = skip_blanks(&b[..r], e + 1);
                    let w = if t < r && b[t] == b'-' { t + 1 } else { t };
                    let we = if w < r && is_name_start(b[w]) {
                        name_run(&b[..r], w + 1)
                    } else {
                        w
                    };
                    if we > w && skip_blanks(&b[..r], we) == r {
                        if w > t {
                            self.put(t, w, ARITH);
                        }
                        self.put(w, we, KEY);
                    } else {
                        self.code_bounded(e + 1, r);
                    }
                    self.put(r, r + 1, ITEM_E);
                    e = r + 1;
                }
                b'-' if get(e + 1) == b'>' && matches!(get(e + 2), b'[' | b'{') => {
                    self.put(e, e + 2, ARROW);
                    e += 2;
                }
                _ => return e,
            }
        }
    }

    /// `b[from..to]`, a string's interpolated code, lexed by the code loop on that piece of
    /// the line, the scan's fields put back after; past [`NEST`] strings deep, left plain.
    pub(super) fn code_bounded(&mut self, from: usize, to: usize) {
        if from >= to || self.nest >= NEST {
            return;
        }
        let saved = Saved {
            frames: self.frames,
            depth: self.depth,
            mode: self.mode,
            prev: self.prev,
            forvar: self.forvar,
            qk: self.qk,
            qdelim: self.qdelim,
            qnest: self.qnest,
            ctx: self.ctx,
            pp: self.pp,
            sqls: self.sqls,
            queue: self.queue,
            call: self.call,
        };
        let b = self.b;
        self.nest += 1;
        self.b = &b[..to];
        self.mode = M_CODE;
        self.prev = P_TERM;
        self.qk = 0;
        self.call = 0;
        self.code_shared(from);
        self.b = b;
        self.nest -= 1;
        self.frames = saved.frames;
        self.depth = saved.depth;
        self.mode = saved.mode;
        self.prev = saved.prev;
        self.forvar = saved.forvar;
        self.qk = saved.qk;
        self.qdelim = saved.qdelim;
        self.qnest = saved.qnest;
        self.ctx = saved.ctx;
        self.pp = saved.pp;
        self.sqls = saved.sqls;
        self.queue = saved.queue;
        self.call = saved.call;
        // the piece's writes, made with the `use` statement's fill off where it popped below it
        self.wrote(to);
    }

    /// A regex's text `b[p..end]` on [`crate::regex`] (Perl's dialect), its
    /// interpolated variables Perl's, from the state in [`Scan::ctx`] and left there; where `x`,
    /// a `#` outside a class starts a comment to `end`.
    fn regex_text(&mut self, p: usize, end: usize, lit: bool, x: bool) {
        let b = self.b;
        let line = &b[..end];
        let text = str_leaf(C_REGEX);
        let embed = &RE_EMBEDS[usize::from(!lit) * 2 + usize::from(x)];
        let mut at = p;
        let mut q = p;
        while q < end {
            let c = b[q];
            if !lit
                && (c == b'$' || c == b'@')
                && re_interp_at(line, q)
                && !escaped(line, q)
                && let Some(e) = self.interp_var(at, q, end, text, true)
            {
                at = e;
                q = e;
                continue;
            }
            if x && c == b'#' && self.ctx & regex::VERBATIM == 0 && !escaped(line, q) {
                self.put(at, q, text);
                self.put(q, end, RE_COMMENT);
                at = end;
                break;
            }
            self.k.close(self.out);
            let r = regex::embedded(line, q, self.ctx, embed, &mut at, self.out);
            self.k.reopen(self.out, 16);
            self.wrote(at);
            let Some((e, st)) = r else {
                break;
            };
            self.ctx = st;
            q = if e > q { e } else { q + char_len(c) };
        }
        self.put(at, end, text);
    }

    /// A SQL string's text `b[p..end]` of class `class` on [`crate::sql`] (in context
    /// [`Scan::ctx`]): its own escapes and, where `interp`, interpolations first at each place
    /// they can start, SQL's between.
    pub(super) fn sql_text(&mut self, p: usize, end: usize, class: u8, interp: bool) {
        let b = self.b;
        // a here-document's SQL text is plain, as bat's `source.sql`; its escapes still the
        // here-document's
        let esc = str_leaf(class) + ESC;
        let text = if class == C_HEREDOC {
            0
        } else {
            str_leaf(class)
        };
        let host = sql::Embed {
            yields: &SQL_YIELDS[usize::from(interp)],
            base: SQL_BASE + sql_class(class) * sql::KINDS as u16,
            text,
        };
        let line = &b[..end];
        self.sql.is_end = usize::MAX;
        let mut q = p;
        while q < end {
            let c = b[q];
            if c == b'\\' {
                let e = if interp {
                    escape_end(b, q, end).max(q + 1)
                } else {
                    // the `\` and the character after it, whole
                    (q + 1 + b.get(q + 1).map_or(0, |&d| char_len(d))).min(end)
                };
                self.put(q, e, esc);
                q = e;
                continue;
            }
            if interp
                && (c == b'$' || c == b'@')
                && let Some(e) = self.interp_var(q, q, end, text, false)
            {
                q = e;
                continue;
            }
            if c == b'%'
                && let Some(e) = placeholder_end(b, q, end)
            {
                self.placeholder(q, e, str_leaf(class));
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
            self.wrote(self.sql.at);
            if self.sql.at < e && text != 0 {
                self.put(self.sql.at, e, text);
            }
            if e > q {
                q = e;
            } else {
                // no progress: the character is the string's text
                let e = (q + char_len(c)).min(end);
                if text != 0 {
                    self.put(q, e, text);
                }
                q = e;
            }
        }
    }

    /// `/` at `p` where a term starts: a regex.
    #[inline(never)]
    pub(super) fn slash_regex(&mut self, p: usize) -> usize {
        self.open_at(Q_M, p)
    }

    /// `<` at `p` where a term starts: `<FH>`, `<$fh>`, `<>`, `<<>>`, a glob (`<*.c>`), to its
    /// `>` on the line; `None` for none.
    #[inline(never)]
    pub(super) fn angle(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        let open = if b.get(p + 1) == Some(&b'<') {
            if b.get(p + 2) == Some(&b'>') && b.get(p + 3) == Some(&b'>') {
                let k = str_leaf(C_ANGLE);
                self.put(p, p + 2, k + BEGIN);
                self.put(p + 2, p + 4, k + END);
                self.prev = P_VALUE;
                return Some(p + 4);
            }
            return None;
        } else {
            p + 1
        };
        let r = find_in(b, open, n, *b">;<=");
        if r >= n || b[r] != b'>' {
            return None;
        }
        let k = str_leaf(C_ANGLE);
        self.put(p, open, k + BEGIN);
        if b.get(open) == Some(&b'$') && b.get(open + 1).is_some_and(|&c| is_name_start(c)) {
            let e = name_run(b, open + 2);
            if e == r {
                self.put(open, open + 1, VAR_SIGIL);
                self.put(open + 1, e, VAR);
            } else {
                self.put(open, r, k);
            }
        } else {
            self.put(open, r, k);
        }
        self.put(r, r + 1, k + END);
        self.prev = P_VALUE;
        Some(r + 1)
    }

    /// `<<` at `p` where a term starts: a here-document's opener (`<<"EOT"`, `<<'EOT'`, `<<EOT`,
    /// `<<\EOT`, `<<~EOT`, `<< "EOT"`), its label queued for its body; `None` for none.
    #[inline(never)]
    pub(super) fn heredoc_open(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        let mut q = p + 2;
        let indent = b.get(q) == Some(&b'~');
        if indent {
            q += 1;
        }
        let c = b.get(q).copied().unwrap_or(b'\n');
        let (ws, we, e, quote) = match c {
            b'"' | b'\'' | b'`' => {
                // `<<""`: an empty label, its body ended by an empty line
                let r = find_in(b, q + 1, n, [c]);
                if r >= n {
                    return None;
                }
                (q + 1, r, r + 1, c)
            }
            b'\\' if b.get(q + 1).is_some_and(|&c| is_name_start(c)) => {
                let r = name_run(b, q + 2);
                (q + 1, r, r, b'\\')
            }
            _ if is_name_start(c) => {
                let r = name_run(b, q + 1);
                (q, r, r, 0)
            }
            b' ' | b'\t' => {
                // `<< "EOT"`: only a quoted label after blanks
                let t = skip_blanks(b, q);
                let c = b.get(t).copied().unwrap_or(b'\n');
                if !matches!(c, b'"' | b'\'') {
                    return None;
                }
                let r = find_in(b, t + 1, n, [c]);
                if r >= n {
                    return None;
                }
                (t + 1, r, r + 1, c)
            }
            _ => return None,
        };
        let word = &b[ws..we];
        let mut lang = label_lang(word);
        if lang == 0 && self.sql_arg() {
            lang = 1;
        }
        let mut flags = 0;
        if quote != b'\'' && quote != b'\\' {
            flags |= H_INTERP;
        }
        if indent {
            flags |= H_INDENT;
        }
        match lang {
            0 => {}
            1 => flags |= H_SQL,
            _ => {
                flags |= H_EMBED;
                if self.queue.is_empty() {
                    self.qlang = lang;
                }
            }
        }
        let tag = TAGS + u16::from(if SOL { 0 } else { lang });
        self.put(p, q, HEREDOC_OP);
        if quote == b'\\' {
            self.put(q, ws, HEREDOC_OP);
        } else if quote != 0 {
            self.put(ws - 1, ws, TAG_QUOTE);
        }
        self.put(ws, we, tag);
        if quote != 0 && quote != b'\\' {
            self.put(we, e, TAG_QUOTE_END);
        }
        self.queue.push(Label::of(word), flags);
        self.prev = P_VALUE;
        Some(e)
    }

    /// A here-document's line ([`M_HEREDOC`]): the queue's first label's closer, or its body's
    /// text.
    pub(super) fn heredoc_line(&mut self) {
        let b = self.b;
        let n = b.len();
        let Some((label, flags)) = self.queue.front() else {
            self.mode = M_CODE;
            return;
        };
        let lead = if flags & H_INDENT != 0 {
            Lead::Blanks
        } else {
            Lead::None
        };
        if label == Label::of(b"") {
            // `<<""`'s body ends at an empty line
            if n == 0 {
                self.queue.pop();
                self.ctx = 0;
                if self.queue.is_empty() {
                    self.mode = M_CODE;
                }
                return;
            }
        } else if let Some(c) = label.close(b, lead, Tail::Blanks) {
            let lang = if SOL {
                0
            } else if flags & H_SQL != 0 {
                1
            } else if flags & H_EMBED != 0 {
                label_lang(&b[c.start..c.end]).max(2)
            } else {
                0
            };
            self.put(c.start, c.end, TAGS + u16::from(lang));
            self.queue.pop();
            self.glang = 0;
            self.ctx = 0;
            if self.queue.is_empty() {
                self.mode = M_CODE;
            }
            return;
        }
        let interp = flags & H_INTERP != 0;
        if self.glang != 0 {
            return self.guest_line(0, interp, str_leaf(C_EMBED));
        }
        if !SOL && flags & H_SQL != 0 {
            self.sql_text(0, n, C_HEREDOC, interp);
            // SQL's line comments end with the line
            if matches!(self.ctx, sql::DASHES | sql::HASHES) {
                self.ctx = sql::MAIN;
            }
            return;
        }
        let class = if !SOL && flags & H_EMBED != 0 {
            C_EMBED
        } else {
            C_HEREDOC
        };
        if interp {
            self.interp_text(0, n, class, true);
        } else {
            self.put(0, n, str_leaf(class));
        }
    }

    /// A line of a guest's body from `from` ([`Scan::glang`], X-191; Perl's own lexemes before
    /// it written): on the run sink the guest's runs by its own rules, from the line with what
    /// isn't its text blanked ([`mask`]: the line before `from`, an interpolating body's escapes
    /// and interpolated variables), those written Perl's among them; on the span sink, or where
    /// the rules keep no guest's, Perl's reading (`text`, those escapes and variables), the guest
    /// lexed for its state alone. A guest's line that sets a byte of Perl's word in the state
    /// loses its state: the body's lines lexed each from a fresh one to its end.
    #[cold]
    #[inline(never)]
    pub(super) fn guest_line(&mut self, from: usize, interp: bool, text: u16) {
        let b = self.b;
        let n = b.len();
        let lang = guest_lang(self.glang);
        let mut g = GUEST.with(|c| std::mem::take(&mut *c.borrow_mut()));
        g.isl.clear();
        // Perl's reading, its escapes and variables found
        let m = self.k.mark(self.out);
        if interp {
            self.embed_text(from, n, text, &mut g.isl);
        } else {
            self.put(from, n, text);
        }
        let line: &[u8] = if from == 0 && g.isl.is_empty() {
            b
        } else {
            mask(b, from, &g.isl, &mut g.buf);
            &g.buf
        };
        // SAFETY: the line came from a `&str`, and only whole characters are blanked
        let line = unsafe { std::str::from_utf8_unchecked(line) };
        let mut st = if self.glost {
            State::default()
        } else {
            self.gst
        };
        let mut kept = false;
        if K::RUNS {
            let spans = &mut *self.out.0;
            kept = self.k.side_guest(&mut g.side, lang, &mut |rules, runs| {
                crate::lex_runs(lang, line, &mut st, spans, rules, runs);
            });
        }
        if kept {
            // the guest's runs, Perl's escapes and variables written again among them
            self.k.undo(self.out, m);
            let mut at = from;
            for i in 0..g.isl.len() {
                let (q, e) = (g.isl[i].0 as usize, g.isl[i].1 as usize);
                self.k
                    .side_write(&mut self.out, &mut g.side, at, q, 0, Side::NO_GAP);
                if b[q] == b'\\' {
                    self.put(q, e, text + ESC);
                } else {
                    self.interp_var(q, q, n, text, false);
                }
                at = e;
            }
            self.k
                .side_write(&mut self.out, &mut g.side, at, n, 0, Side::NO_GAP);
        } else {
            crate::lex_state(lang, line, &mut st);
        }
        if st.withs[6..14].iter().any(|&c| c != 0) {
            self.glost = true;
        }
        self.gst = if self.glost { State::default() } else { st };
        GUEST.with(|c| *c.borrow_mut() = g);
    }

    /// An interpolating body's text `b[p..end]` as Perl reads it under a guest (X-191): its
    /// escapes and interpolated variables, each pushed to `isl`, the text between `text`; no
    /// placeholders (a guest's `%` is its own).
    fn embed_text(&mut self, p: usize, end: usize, text: u16, isl: &mut Vec<(u32, u32)>) {
        let b = self.b;
        let (mut at, mut q) = (p, p);
        loop {
            q = find_in(b, q, end, *b"\\$@");
            if q >= end {
                break;
            }
            if b[q] == b'\\' {
                let e = escape_end(b, q, end).max(q + 1);
                self.put_hot(at, q, text);
                self.put(q, e, text + ESC);
                isl.push((q as u32, e as u32));
                (at, q) = (e, e);
            } else if let Some(e) = self.interp_var(at, q, end, text, false) {
                isl.push((q as u32, e as u32));
                (at, q) = (e, e);
            } else {
                q += 1;
            }
        }
        self.put_hot(at, end, text);
    }

    /// `#` at `p`: a comment to the line's end, its `#`s its mark.
    #[inline(always)]
    pub(super) fn comment(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut e = p + 1;
        while e < n && b[e] == b'#' {
            e += 1;
        }
        self.put(p, e, COMMENT_MARK);
        self.put(e, n, COMMENT);
        n
    }

    /// A number at `p` (a digit, or the `.` before one): `0x`, `0b`, `0o` and `0`'s octal,
    /// decimals with `_`s, a fraction, an exponent; a version with two dots or more.
    #[inline(always)]
    pub(super) fn number(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        self.prev = P_VALUE;
        let digits = |mut e: usize, ok: fn(u8) -> bool| {
            while e < n && (ok(b[e]) || b[e] == b'_') {
                e += 1;
            }
            e
        };
        let dec = |c: u8| c.is_ascii_digit();
        let c1 = b.get(p + 1).copied().unwrap_or(0);
        if b[p] == b'0' && c1 != 0 && !SOL {
            let (prefix, kind, ok): (usize, u16, fn(u8) -> bool) = match c1 {
                b'x' | b'X' => (2, HEX, |c| c.is_ascii_hexdigit()),
                b'b' | b'B' => (2, BIN, |c| c == b'0' || c == b'1'),
                b'o' | b'O' => (2, OCT, |c| matches!(c, b'0'..=b'7')),
                b'0'..=b'7' | b'_' => (1, OCT, |c| matches!(c, b'0'..=b'7')),
                _ => (0, 0, dec),
            };
            if prefix != 0 {
                let e = digits(p + prefix, ok);
                if e > p + prefix {
                    self.put(p, p + prefix, kind + 1);
                    self.put_hot(p + prefix, e, kind);
                    return e;
                }
            }
        }
        let e = if b[p] == b'.' { p } else { digits(p, dec) };
        if b.get(e) == Some(&b'.') && b.get(e + 1) != Some(&b'.') {
            let f = digits(e + 1, dec);
            if !SOL
                && f > e + 1
                && b.get(f) == Some(&b'.')
                && b.get(f + 1).is_some_and(u8::is_ascii_digit)
            {
                // `1.2.3`: a version
                return self.version(p);
            }
            let x = self.exponent(f);
            if e > p {
                self.put_hot(p, e, FLOAT);
            }
            self.put_hot(e, e + 1, FLOAT_DOT);
            if x > e + 1 {
                self.put_hot(e + 1, x, FLOAT);
            }
            return x.max(e + 1);
        }
        let x = self.exponent(e);
        if x > e {
            self.put_hot(p, x, FLOAT);
            return x;
        }
        self.put_hot(p, e, INT);
        e
    }

    /// An exponent at `e` (`e10`, `E-3`): where it ends, `e` itself for none.
    #[inline(always)]
    fn exponent(&self, e: usize) -> usize {
        let b = self.b;
        if !matches!(b.get(e), Some(b'e' | b'E')) {
            return e;
        }
        let mut q = e + 1;
        if matches!(b.get(q), Some(b'+' | b'-')) {
            q += 1;
        }
        if !b.get(q).is_some_and(u8::is_ascii_digit) {
            return e;
        }
        while q < b.len() && (b[q].is_ascii_digit() || b[q] == b'_') {
            q += 1;
        }
        q
    }

    /// A POD command at a code line's start (`=head1`, `=pod`, `=cut`): POD to `=cut`.
    #[cold]
    #[inline(never)]
    pub(super) fn pod_open(&mut self) {
        let b = self.b;
        let n = b.len();
        let e = name_run(b, 1);
        self.put(0, e, POD_TAG);
        if &b[1..e] == b"cut" {
            self.put(e, n, POD);
            return;
        }
        self.mode = M_POD;
        self.pod_para();
        if SOL {
            self.put(e, n, POD_HEAD);
        } else {
            self.pod_command(e);
        }
    }

    /// A POD command's text from `e`, past its word, its paragraph's state set: `=begin`, `=for`
    /// and `=end` before a guest's name (`html`, `json`) name it, and `=begin`'s region and
    /// `=for`'s paragraph (from the name on) are the guest's where Perl's fields fit beside its
    /// state (X-191); the rest a heading's.
    fn pod_command(&mut self, e: usize) {
        let b = self.b;
        let n = b.len();
        let cmd = &b[1..e];
        let (begin, for_) = (cmd == b"begin", cmd == b"for");
        if begin || for_ || cmd == b"end" {
            let t = skip_blanks(b, e);
            if t < n && b[t].is_ascii_alphabetic() {
                let w = name_run(b, t + 1);
                let lang = pod_lang(&b[t..w]);
                if lang != 0 {
                    self.put(e, t, POD_HEAD);
                    self.put(t, w, POD_LANG);
                    if (begin || for_) && self.queue.is_empty() && self.guest_fits() {
                        self.guest_open(lang | GL_POD | if for_ { GL_FOR } else { 0 });
                        if for_ {
                            return self.guest_line(w, false, POD);
                        }
                    }
                    return self.pod_text(w, n, 1);
                }
            }
        }
        self.pod_text(e, n, 1);
    }

    /// A paragraph starts at the next line: no formatting code open, its kind (ordinary or
    /// verbatim) by its first line.
    #[inline(always)]
    fn pod_para(&mut self) {
        (self.qk, self.qdelim, self.qnest, self.ctx) = (POD_PARA, 0, 0, 0);
    }

    /// A line of POD ([`M_POD`]): a command (`=cut` its end), a blank line (a paragraph's end), a
    /// verbatim paragraph's line (its first indented: no formatting codes), or an ordinary one's
    /// text, its codes carried from the line before ([`Scan::qk`]'s depth, the three outermost in
    /// [`Scan::qdelim`], [`Scan::qnest`] and [`Scan::ctx`], which POD leaves free).
    #[cold]
    #[inline(never)]
    pub(super) fn pod_line(&mut self) {
        let b = self.b;
        let n = b.len();
        if self.glang != 0 {
            // a guest's region: to a command, or `=for`'s paragraph to a blank line
            let command = b.first() == Some(&b'=') && b.get(1).is_some_and(u8::is_ascii_alphabetic);
            if !command && (self.glang & GL_FOR == 0 || skip_blanks(b, 0) < n) {
                return self.guest_line(0, false, POD);
            }
            self.glang = 0;
        }
        if skip_blanks(b, 0) == n {
            self.pod_para();
            return;
        }
        if b[0] == b'=' && b.get(1).is_some_and(u8::is_ascii_alphabetic) {
            let e = name_run(b, 1);
            self.put(0, e, POD_TAG);
            if &b[1..e] == b"cut" {
                self.put(e, n, POD);
                self.mode = if self.poddata { M_DATA } else { M_CODE };
                self.poddata = false;
                (self.qk, self.qdelim, self.qnest, self.ctx) = (0, 0, 0, 0);
                return;
            }
            self.pod_para();
            if SOL {
                self.put(e, n, POD_HEAD);
            } else {
                self.pod_command(e);
            }
            return;
        }
        if self.qk & POD_PARA != 0 {
            self.qk = if matches!(b[0], b' ' | b'\t') {
                POD_VERBATIM
            } else {
                0
            };
        }
        if SOL || self.qk & POD_VERBATIM != 0 {
            self.put(0, n, POD);
            return;
        }
        self.pod_text(0, n, 0);
    }

    /// POD's text `b[from..to]` in context `ctx` (0 an ordinary paragraph's line, its open codes
    /// carried across lines; 1 a command's line): its formatting codes (`B<…>`, `C<< … >>`,
    /// nested).
    fn pod_text(&mut self, from: usize, to: usize, ctx: u8) {
        let b = self.b;
        let base = if ctx == 1 { POD_HEAD } else { POD };
        // the open codes: a letter's index, its angles
        let mut stack = [(0u8, 0u8); 8];
        let mut sp = 0usize;
        if ctx == 0 {
            sp = usize::from(self.qk >> 2).min(stack.len());
            let kept = [self.qdelim, self.qnest, self.ctx];
            for (i, slot) in stack.iter_mut().enumerate().take(sp) {
                let e = kept[i.min(2)];
                *slot = (e & 7, if i < 3 { (e >> 3).max(1) } else { 1 });
            }
        }
        let mut at = from;
        let mut p = from;
        let text = |stack: &[(u8, u8)], sp: usize| {
            if sp == 0 {
                base
            } else {
                pod_kind(ctx, stack[sp - 1].0, true)
            }
        };
        while p < to {
            let q = if sp == 0 {
                find_in(b, p, to, *b"<")
            } else {
                find_in(b, p, to, *b"<>")
            };
            if q >= to {
                break;
            }
            if b[q] == b'<' {
                let letter = if q > from {
                    POD_LETTERS.iter().position(|&c| c == b[q - 1])
                } else {
                    None
                };
                let Some(letter) = letter.filter(|_| sp < stack.len()) else {
                    p = q + 1;
                    continue;
                };
                let mut k = 1;
                while q + k < to && b[q + k] == b'<' {
                    k += 1;
                }
                if k > 1 && !(q + k < to && matches!(b[q + k], b' ' | b'\t')) {
                    k = 1;
                }
                let k = k.min(31);
                self.put(at, q - 1, text(&stack, sp));
                self.put(q - 1, q, pod_kind(ctx, letter as u8, false));
                self.put(q, q + k, pod_angle(ctx, false));
                stack[sp] = (letter as u8, k as u8);
                sp += 1;
                at = q + k;
                p = q + k;
                continue;
            }
            // `>`: the innermost code's close, its angles' count of them after a blank
            let k = usize::from(stack[sp - 1].1);
            let mut r = q;
            while r < to && b[r] == b'>' {
                r += 1;
            }
            if k == 1 || (r - q >= k && q > from && matches!(b[q - 1], b' ' | b'\t')) {
                self.put(at, q, text(&stack, sp));
                self.put(q, q + k, pod_angle(ctx, true));
                sp -= 1;
                at = q + k;
                p = q + k;
            } else {
                p = r;
            }
        }
        self.put(at, to, text(&stack, sp));
        if ctx == 0 {
            let kept = |i: usize| {
                if i < sp {
                    stack[i].0 | stack[i].1.min(31) << 3
                } else {
                    0
                }
            };
            self.qk = (sp.min(15) as u8) << 2;
            (self.qdelim, self.qnest, self.ctx) = (kept(0), kept(1), kept(2));
        }
    }

    /// `format` at `p` to `e` at a statement's start, before `NAME =` and the line's end: its
    /// pictures to a `.` line follow ([`M_FORMAT`]); else a bareword.
    #[inline(never)]
    pub(super) fn format_head(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let n = b.len();
        if self.prev != P_START || SOL {
            return self.bareword_out(p, e);
        }
        let q = skip_blanks(b, e);
        let r = if b.get(q).is_some_and(|&c| is_name_start(c)) {
            qualified_end(b, name_run(b, q + 1))
        } else {
            q
        };
        let t = skip_blanks(b, r);
        if b.get(t) != Some(&b'=') || skip_blanks(b, t + 1) != n {
            return self.bareword_out(p, e);
        }
        self.put(p, e, FORMAT_KW);
        if r > q {
            self.put(q, r, VAR);
        }
        self.put(t, t + 1, ASSIGN);
        self.mode = M_FORMAT;
        n
    }

    /// A line of `format`'s pictures ([`M_FORMAT`]): `.` alone ends them; fields (`@<<<`,
    /// `^>>>`, `@##.##`, `@*`) and variables in text.
    #[cold]
    pub(super) fn format_line(&mut self) {
        let b = self.b;
        let n = b.len();
        if b.first() == Some(&b'.') && skip_blanks(b, 1) == n {
            self.put(0, 1, FORMAT_END);
            self.mode = M_CODE;
            self.prev = P_START;
            return;
        }
        let text = str_leaf(C_UNQ);
        let mut at = 0;
        let mut q = 0;
        loop {
            q = find_in(b, q, n, *b"@^$");
            if q >= n {
                break;
            }
            let c1 = b.get(q + 1).copied().unwrap_or(0);
            if b[q] != b'$' && matches!(c1, b'<' | b'>' | b'|' | b'#' | b'*') {
                let mut e = q + 2;
                while e < n && matches!(b[e], b'<' | b'>' | b'|' | b'#' | b'.') {
                    e += 1;
                }
                self.put(at, q, text);
                self.put(q, q + 1, FORMAT_MARK);
                self.put(q + 1, e, FORMAT_FIELD);
                at = e;
                q = e;
                continue;
            }
            if b[q] != b'^'
                && let Some(e) = self.interp_var(at, q, n, text, false)
            {
                at = e;
                q = e;
                continue;
            }
            q += 1;
        }
        self.put(at, n, text);
    }
}

/// Whether a variable's name `b[p..e]` after sigil `s` is a special one (`$_`, `@ARGV`,
/// `%ENV`, `$a`): looked up only where it can be (one byte, or a capital first).
#[inline(always)]
pub(super) fn special_name(b: &[u8], p: usize, e: usize, s: u8) -> bool {
    let c = b[p];
    if e - p == 1 {
        return match c {
            b'_' | b'F' => true,
            b'a' | b'b' => s == b'$',
            _ => false,
        };
    }
    c.is_ascii_uppercase() && WORDS.value(&b[p..e], &INFO) & W_VARLANG != 0
}

/// Whether `b[p..]` reads as an array's index in a regex (`$x[1]`, `$x[-1]`, `$x[$i]`), not a
/// class.
fn index_like(b: &[u8], p: usize, end: usize) -> bool {
    let mut q = p;
    if q < end && b[q] == b'-' {
        q += 1;
    }
    if q < end && b[q] == b'$' {
        q += 1;
        if !(q < end && is_name_start(b[q])) {
            return false;
        }
        q = name_run(&b[..end], q + 1);
    } else {
        let s = q;
        while q < end && b[q].is_ascii_digit() {
            q += 1;
        }
        if q == s {
            return false;
        }
    }
    q < end && b[q] == b']'
}

/// Whether `b[p..]` reads as a quantifier's braces' contents (`2}`, `2,}`, `2,5}`).
fn quantifier_like(b: &[u8], p: usize, end: usize) -> bool {
    let mut q = p;
    while q < end && (b[q].is_ascii_digit() || b[q] == b',') {
        q += 1;
    }
    q > p && q < end && b[q] == b'}'
}
