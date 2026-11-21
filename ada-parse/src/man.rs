//! Man pages as `man` shows them (X-188), in place of bat's `Manpage`: the text a pager gets,
//! its bold and underline taken off (`col -bx`, or ada's own reading of `man`'s output). The
//! page's layout says what a line is: the title, the footer, a section's heading at the margin,
//! a subsection's three columns in, and the body under them, indented as the page's first body
//! line is (five columns from groff 1.23 on, seven before; bat's grammar reads only seven).
//!
//! In the body: options (`-a`, `--all`, `--color[=WHEN]`) and their parameters, an option list's
//! tags read whole, `name(n)` references and `name()` functions, `$VARIABLE`s; a SYNOPSIS's
//! command, brackets, bars and placeholders; an ENVIRONMENT section's variables, a COMMANDS
//! section's commands; and C, from an `#include`, `#define`, `/*` or `struct … {` line to the
//! next sentence, on C's scanner. A line's reading depends on the lines before it only through
//! the word [`State`] keeps: the section, the body's indent, the last option tag's indent, a
//! synopsis's open brackets, and whether C is open (and a comment in it).

use crate::emit::{Push, Runs, Sink};
use crate::{Out, State};

const TITLE_NAME: u16 = 0;
const TITLE_PAREN: u16 = 1;
const TITLE_SECTION: u16 = 2;
const TITLE: u16 = 3;
const HEADING: u16 = 4;
const HEADING_SYNOPSIS: u16 = 5;
const HEADING_COMMANDS: u16 = 6;
const HEADING_ENV: u16 = 7;
const SUBHEADING: u16 = 8;
const OPTION: u16 = 9;
const PARAMETER: u16 = 10;
const ASSIGN: u16 = 11;
const BRACKET_BEGIN: u16 = 12;
const BRACKET_END: u16 = 13;
const PIPE: u16 = 14;
const ANGLE_BEGIN: u16 = 15;
const ANGLE_END: u16 = 16;
const SEPARATOR: u16 = 17;
const FUNCTION: u16 = 18;
const REF_PAREN: u16 = 19;
const REF_SECTION: u16 = 20;
const VAR_SIGIL: u16 = 21;
const VAR: u16 = 22;
const ENV_NAME: u16 = 23;
const COMMAND: u16 = 24;

/// Each kind's scopes under `source.man`: bat's where its grammar has the reading, ada's own
/// leaf beside them where it doesn't (a subsection's heading, a synopsis's command).
const KINDS: &[&str] = &[
    "meta.preprocessor.man",
    "keyword.operator.man",
    "string.quoted.other.man",
    "markup.heading.title.man",
    "markup.heading.other.man",
    "markup.heading.synopsis.man",
    "markup.heading.commands.man",
    "markup.heading.env.man",
    "markup.heading.subsection.man",
    "entity.name.command-line-option.man",
    "variable.parameter.man",
    "keyword.operator.man",
    "punctuation.section.brackets.begin.man",
    "punctuation.section.brackets.end.man",
    "keyword.operator.logical.man",
    "punctuation.definition.generic.begin.man",
    "punctuation.definition.generic.end.man",
    "punctuation.separator.man",
    "entity.name.function.man",
    "keyword.operator.man",
    "constant.numeric.man",
    "punctuation.definition.variable.man",
    "constant.other.man",
    "support.constant.environment-variable.man",
    "entity.name.command.man",
];

/// C's kinds ([`crate::cc::scopes_c`]) with this bit set, under `source.c`.
const C_KIND: u16 = 0x1000;

/// The scopes under `source.man` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    if kind & C_KIND != 0 {
        let mut v = vec!["source.c"];
        v.extend(crate::cc::scopes_c(kind & !C_KIND));
        return v;
    }
    KINDS
        .get(usize::from(kind))
        .map_or_else(Vec::new, |k| vec![*k])
}

// The sections whose bodies read apart, in [`State`]'s word.
const S_NONE: u64 = 0;
const S_SYNOPSIS: u64 = 1;
const S_COMMANDS: u64 = 2;
const S_ENV: u64 = 3;
const S_OTHER: u64 = 4;

// [`State`]'s word: the section, then flags, then the indents and the bracket depth a byte each.
const W_SECTION: u64 = 0x7;
/// A line with text in it has been read: the page's first is its title, if it has the shape.
const W_STARTED: u64 = 1 << 3;
/// The lines are C's, to the next sentence or heading.
const W_C: u64 = 1 << 4;
/// C's last line left a `/* … */` comment open.
const W_C_COMMENT: u64 = 1 << 5;
/// The line before was blank (or the page's first): an option list's tag may start here.
const W_BLANK: u64 = 1 << 6;
/// The body's indent, 0 until the first body line under a heading.
const W_INDENT: u32 = 8;
/// The last option tag's indent, 0 for none: a tag list without blank lines between its items.
const W_TAG: u32 = 16;
/// A synopsis's brackets open across lines, saturating at 15.
const W_DEPTH: u32 = 24;
/// The last line's indent, 0 after a blank one: a tag may stand deeper than the text above it.
const W_PREV: u32 = 32;
/// The title names a library's page (section 2 or 3, not Perl's): its synopsis is C.
const W_LIB: u64 = 1 << 7;

/// The floor probe of the scanner's shape (X-188, the guide's sol probe), built with
/// `ADA_PARSE_X188_SOL` set: a line's indent read, a heading at the margin, and the prose loop over
/// the rest of every line; no title, tags, synopsis, sections or C.
const SOL: bool = option_env!("ADA_PARSE_X188_SOL").is_some();

/// One line, after the lines before it left `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    scan(b, state, out, Push);
}

/// [`line`]'s runs by theme rule written to `k`: inline in a block's loop, as Diff's are
/// (X-163), [`crate::lex_runs`]' line too.
pub(crate) struct RunLine;

impl crate::LineScan for RunLine {
    #[inline(always)]
    fn scan(b: &[u8], state: &mut State, out: &mut Out, k: &mut Runs) {
        *k = scan(b, state, out, *k);
    }
}

/// A line's scan: the line, the page's word, and the sink.
struct Scan<'a, 'o, K: Sink> {
    b: &'a [u8],
    out: &'a mut Out<'o>,
    k: K,
    word: u64,
    /// Where the last write ended: a reference's name doesn't reach back into it.
    w: usize,
}

#[inline(always)]
fn scan<K: Sink>(b: &[u8], state: &mut State, out: &mut Out, k: K) -> K {
    let mut s = Scan {
        b,
        out,
        k,
        word: state.word(),
        w: 0,
    };
    s.k.open(s.out, 8);
    s.line();
    s.k.close(s.out);
    state.set_word(s.word);
    s.k
}

/// Whether `c` continues an option's name, or stands before one where it can't start.
#[inline(always)]
fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-'
}

/// Whether `c` is part of a reference's name (`systemd.unit`, `Archive::Tar`, `g++`).
#[inline(always)]
fn is_name(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b':' | b'+')
}

/// What a body byte asks of the prose loop: nothing, or a look at an option, a reference or a
/// variable.
const P_OPTION: u8 = 1;
const P_PAREN: u8 = 2;
const P_DOLLAR: u8 = 3;

static PROSE: [u8; 256] = {
    let mut t = [0u8; 256];
    t[b'-' as usize] = P_OPTION;
    t[b'(' as usize] = P_PAREN;
    t[b'$' as usize] = P_DOLLAR;
    t
};

impl<K: Sink> Scan<'_, '_, K> {
    #[inline(always)]
    fn put(&mut self, s: usize, e: usize, kind: u16) {
        self.k.push(&mut self.out, s, e, kind);
        self.w = e;
    }

    /// A rare token's write, one call on the run sink.
    #[inline(always)]
    fn put_call(&mut self, s: usize, e: usize, kind: u16) {
        self.k.push_call(&mut self.out, s, e, kind);
        self.w = e;
    }

    fn section(&self) -> u64 {
        self.word & W_SECTION
    }

    fn field(&self, at: u32) -> usize {
        (self.word >> at & 0xff) as usize
    }

    fn set_field(&mut self, at: u32, v: usize) {
        self.word = self.word & !(0xff << at) | (v.min(0xff) as u64) << at;
    }

    #[inline(always)]
    fn line(&mut self) {
        let b = self.b;
        let n = b.len();
        let ind = b.iter().take_while(|&&c| c == b' ').count();
        if SOL {
            match ind {
                _ if ind == n => {}
                0 => self.put(0, n, HEADING),
                _ => self.prose(ind),
            }
            return;
        }
        if ind == n {
            self.word |= W_BLANK;
            self.set_field(W_PREV, 0);
            return;
        }
        let blank = self.word & W_BLANK != 0;
        let prev = self.field(W_PREV);
        self.word &= !W_BLANK;
        self.set_field(W_PREV, ind);
        if self.word & W_STARTED == 0 {
            self.word |= W_STARTED | W_BLANK;
            if ind == 0
                && let Some(left) = title_left(b)
            {
                self.title(left);
                return;
            }
            self.word &= !W_BLANK;
        }
        if ind == 0 {
            self.margin();
            return;
        }
        if self.word & W_C != 0 {
            if c_ends(&b[ind..]) {
                self.word &= !(W_C | W_C_COMMENT);
            } else {
                return self.c();
            }
        }
        if c_starts(&b[ind..]) {
            self.word |= W_C;
            self.word &= !W_C_COMMENT;
            return self.c();
        }
        let indent = self.field(W_INDENT);
        if ind == 3 && (indent == 0 || indent > 3) && self.section() != S_NONE {
            // a subsection's heading; one that is an option's (go-md2man's pages, podman's) read
            // as its tag
            self.set_field(W_TAG, 0);
            if is_option_start(b, ind) {
                let p = self.tag(ind);
                return self.prose(p);
            }
            self.put_call(ind, trim_end(b), SUBHEADING);
            return;
        }
        // a library's synopsis is C, but for its sentences
        if self.section() == S_SYNOPSIS && self.word & W_LIB != 0 {
            if c_ends(&b[ind..]) {
                return self.prose(ind);
            }
            return self.c();
        }
        if indent == 0 && self.section() != S_NONE {
            self.set_field(W_INDENT, ind);
        }
        let tag = self.field(W_TAG);
        if tag != 0 && ind < tag {
            self.set_field(W_TAG, 0);
        }
        // where an option list's tag may stand: after a blank line, or at the last tag's indent;
        // an option's also deeper than the line before (a list right under its paragraph's line)
        let at_tag = blank || ind == tag;
        match self.section() {
            S_SYNOPSIS => self.synopsis(ind, ind == self.field(W_INDENT)),
            S_ENV if at_tag && is_env_start(b, ind) => {
                let e = ind + b[ind..].iter().take_while(|&&c| is_env(c)).count();
                if e == n || b[e] == b' ' {
                    self.set_field(W_TAG, ind);
                    self.put_call(ind, e, ENV_NAME);
                    self.prose(e);
                } else {
                    self.prose(ind);
                }
            }
            S_COMMANDS if at_tag && b[ind].is_ascii_lowercase() => {
                let e = ind
                    + b[ind..]
                        .iter()
                        .take_while(|&&c| c.is_ascii_lowercase() || c == b'_' || c == b'-')
                        .count();
                if e == n || b[e] == b' ' {
                    self.set_field(W_TAG, ind);
                    self.put_call(ind, e, COMMAND);
                    let p = self.tag_rest(e);
                    self.prose(p);
                } else {
                    self.prose(ind);
                }
            }
            _ if (at_tag || ind > prev) && is_option_start(b, ind) => {
                self.set_field(W_TAG, ind);
                let p = self.tag(ind);
                self.prose(p);
            }
            _ => self.prose(ind),
        }
    }

    /// The page's title, `LS(1)  User Commands  LS(1)`, its left part `left` bytes.
    #[cold]
    fn title(&mut self, left: usize) {
        let b = self.b;
        let n = trim_end(b);
        if is_library(&b[..left]) {
            self.word |= W_LIB;
        }
        self.name_section(0, left);
        let right = n - left;
        let mid = (left..right).find(|&i| b[i] != b' ').unwrap_or(right);
        let mid_end = (mid..right)
            .rev()
            .find(|&i| b[i] != b' ')
            .map_or(mid, |i| i + 1);
        self.put(mid, mid_end, TITLE);
        self.name_section(right, n);
    }

    /// A title's `NAME(SECTION)` in `s..e`, its last byte the `)`.
    fn name_section(&mut self, s: usize, e: usize) {
        let open = match self.b[s..e].iter().rposition(|&c| c == b'(') {
            Some(i) => s + i,
            None => return,
        };
        self.put(s, open, TITLE_NAME);
        self.put(open, open + 1, TITLE_PAREN);
        self.put(open + 1, e - 1, TITLE_SECTION);
        self.put(e - 1, e, TITLE_PAREN);
    }

    /// A line at the margin: the footer, or a section's heading.
    #[cold]
    fn margin(&mut self) {
        let b = self.b;
        let n = trim_end(b);
        self.word &= !(W_C | W_C_COMMENT);
        if let Some(right) = footer_right(&b[..n]) {
            self.name_section(right, n);
            return;
        }
        self.set_field(W_PREV, 0);
        let (section, kind) = heading(&b[..n]);
        self.word = self.word & !W_SECTION & !(0xff << W_TAG) & !(0xff << W_DEPTH) | section;
        self.word |= W_BLANK;
        self.put(0, n, kind);
    }

    /// A line of C, from the state the last one left.
    #[cold]
    fn c(&mut self) {
        let b = self.b;
        let mut st = crate::cc::embedded_state(self.word & W_C_COMMENT != 0);
        let out = &mut *self.out;
        let written = self
            .k
            .shifted(C_KIND, |k| crate::cc::embedded_runs(b, &mut st, out, k));
        if !written {
            self.k.close(self.out);
            let from = self.out.0.len();
            crate::cc::line_c(b, &mut st, self.out);
            for s in &mut self.out.0[from..] {
                s.kind |= C_KIND;
            }
            self.k.reopen(self.out, 8);
        }
        if crate::cc::embedded_comment(&st) {
            self.word |= W_C_COMMENT;
        } else {
            self.word &= !W_C_COMMENT;
        }
    }

    /// The body's text from `p`: options, references and variables among plain words.
    #[inline(always)]
    fn prose(&mut self, mut p: usize) {
        let b = self.b;
        let n = b.len();
        while p < n {
            match PROSE[usize::from(b[p])] {
                0 => p += 1,
                P_OPTION => p = self.prose_option(p),
                P_PAREN => p = self.reference(p),
                _ => p = self.variable(p),
            }
        }
    }

    /// At a `-` in prose: an option where one starts (bat's rule: not after a word's byte), with
    /// a value after its `=`; where the scan goes on.
    #[inline(never)]
    fn prose_option(&mut self, p: usize) -> usize {
        let b = self.b;
        if p > 0 && is_word(b[p - 1]) {
            return p + 1;
        }
        let Some(e) = option_end(b, p) else {
            return p + 1;
        };
        self.put(p, e, OPTION);
        if b.get(e) == Some(&b'=') {
            let v = e
                + 1
                + b[e + 1..]
                    .iter()
                    .take_while(|&&c| !matches!(c, b']' | b',' | b'.' | b'(' | b')' | b' '))
                    .count();
            self.put(e, e + 1, ASSIGN);
            self.put(e + 1, v, PARAMETER);
            return v;
        }
        e
    }

    /// At a `(`: a reference, `name(n)`, or a function, `name()` or `name(args)`, where its name
    /// stands right before it and its `)` closes it on the line, or another `(` comes first;
    /// where the scan goes on.
    #[inline(never)]
    fn reference(&mut self, p: usize) -> usize {
        let b = self.b;
        let floor = self.w;
        let mut s = p;
        while s > floor && is_name(b[s - 1]) {
            s -= 1;
        }
        // the name starts at a letter or digit
        while s < p && !b[s].is_ascii_alphanumeric() {
            s += 1;
        }
        if s == p {
            return p + 1;
        }
        let Some(close) = b[p + 1..].iter().position(|&c| c == b')' || c == b'(') else {
            return p + 1;
        };
        let close = p + 1 + close;
        if b[close] == b'(' {
            // a call with another inside, `f(a, sizeof(b))`: its name and `(`, the inner one
            // read next
            self.put(s, p, FUNCTION);
            self.put(p, p + 1, REF_PAREN);
            return p + 1;
        }
        let inner = &b[p + 1..close];
        // English plurals, `file(s)`, `entry(ies)`
        if matches!(inner, b"s" | b"es" | b"ies") {
            return close + 1;
        }
        self.put(s, p, FUNCTION);
        self.put(p, p + 1, REF_PAREN);
        if is_section(inner) {
            self.put(p + 1, close, REF_SECTION);
        }
        self.put(close, close + 1, REF_PAREN);
        close + 1
    }

    /// At a `$`: `$NAME`, its name not starting with a digit.
    #[inline(never)]
    fn variable(&mut self, p: usize) -> usize {
        let b = self.b;
        let e = p
            + 1
            + b[p + 1..]
                .iter()
                .take_while(|&&c| c.is_ascii_alphanumeric() || c == b'_')
                .count();
        if e == p + 1 || b[p + 1].is_ascii_digit() {
            return p + 1;
        }
        self.put(p, p + 1, VAR_SIGIL);
        self.put(p + 1, e, VAR);
        e
    }

    /// An option list's tag from `p`, `-a, --all`, `--color[=WHEN]`, `-o file`: its options,
    /// their values and the separators, to its end (two blanks, or the line's); where the
    /// description after it starts.
    #[inline(never)]
    fn tag(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        loop {
            let Some(e) = option_end(b, p) else {
                return p;
            };
            self.put(p, e, OPTION);
            p = self.value(e, n, true);
            // a separator, then the next option
            match b.get(p) {
                Some(b',') => {
                    let q = skip_blank(b, p + 1);
                    if q < n && is_option_start(b, q) && q - p <= 2 {
                        self.put(p, p + 1, SEPARATOR);
                        p = q;
                        continue;
                    }
                    return p;
                }
                Some(b' ') if b.get(p + 1) == Some(&b'|') => {
                    let q = skip_blank(b, p + 2);
                    if q < n && is_option_start(b, q) {
                        self.put(p + 1, p + 2, PIPE);
                        p = q;
                        continue;
                    }
                    return p;
                }
                Some(b'|') => {
                    if is_option_start(b, p + 1) {
                        self.put(p, p + 1, PIPE);
                        p += 1;
                        continue;
                    }
                    return p;
                }
                _ => return p,
            }
        }
    }

    /// A COMMANDS section's tag after its command's name, at `p`: its arguments as a synopsis
    /// reads them, to the tag's end.
    #[inline(never)]
    fn tag_rest(&mut self, p: usize) -> usize {
        let b = self.b;
        let end = gap(b, p);
        self.args(p, end).max(end)
    }

    /// An option's value after it at `p`, before `end`: `=VALUE`, `[=VALUE]`, `<value>`, or a
    /// word after one blank: an uppercase one always; another where a word, a `,` or a `]`
    /// ends it, but in an option list's `tag` only where the tag ends after it, so that a
    /// description after one blank isn't taken. Where the scan goes on.
    fn value(&mut self, p: usize, end: usize, tag: bool) -> usize {
        let b = &self.b[..end];
        let n = b.len();
        match b.get(p) {
            Some(b'=') => {
                self.put(p, p + 1, ASSIGN);
                self.param(p + 1, end)
            }
            Some(b'[') if b.get(p + 1) == Some(&b'=') => {
                // `[=WHEN]`, its value up to the `]`
                self.put(p, p + 1, BRACKET_BEGIN);
                self.put(p + 1, p + 2, ASSIGN);
                let v = self.param(p + 2, end);
                if b.get(v) != Some(&b']') {
                    return v;
                }
                self.put(v, v + 1, BRACKET_END);
                v + 1
            }
            Some(b'<') => self.param(p, end),
            Some(b' ') if p + 1 < n => {
                let q = p + 1;
                let c = b[q];
                if c == b'<' {
                    return self.param(q, end);
                }
                if !(c.is_ascii_alphanumeric() || c == b'_') {
                    return p;
                }
                let e = q + b[q..].iter().take_while(|&&c| is_param(c)).count();
                let upper =
                    b[q..e].iter().all(|c| !c.is_ascii_lowercase()) && b[q].is_ascii_alphabetic();
                let ends = e == n
                    || b[e] == b','
                    || b[e] == b']'
                    || b[e] == b' ' && (b.get(e + 1) == Some(&b' ') || !tag);
                if upper && (ends || b[e] == b' ' || b[e] == b'[') || ends {
                    self.put(q, e, PARAMETER);
                    return e;
                }
                p
            }
            _ => p,
        }
    }

    /// A parameter at `p`, before `end`: `<…>` with its brackets, or a run of a value's bytes.
    fn param(&mut self, p: usize, end: usize) -> usize {
        let b = &self.b[..end];
        if b.get(p) == Some(&b'<') {
            let Some(close) = b[p..].iter().position(|&c| c == b'>').map(|i| p + i) else {
                return p;
            };
            self.put(p, p + 1, ANGLE_BEGIN);
            self.put(p + 1, close, PARAMETER);
            self.put(close, close + 1, ANGLE_END);
            return close + 1;
        }
        let e = p + b[p..].iter().take_while(|&&c| is_value(c)).count();
        self.put(p, e, PARAMETER);
        e
    }

    /// A synopsis's line from its indent `ind`: at the body's indent, its command first; then
    /// its arguments.
    #[inline(never)]
    fn synopsis(&mut self, ind: usize, at_body: bool) {
        let b = self.b;
        let n = b.len();
        let mut p = ind;
        if at_body && self.field(W_DEPTH) == 0 && is_command_start(b[p]) {
            let e = p + b[p..].iter().take_while(|&&c| c != b' ').count();
            self.put(p, e, FUNCTION);
            p = e;
        }
        self.args(p, n);
    }

    /// A synopsis's arguments in `p..end`: options and their values, brackets (open across
    /// lines), bars, placeholders (`FILE`, `<file>`), literal words plain. Where the last read
    /// ends: past `end` where a reference closes after it.
    fn args(&mut self, mut p: usize, end: usize) -> usize {
        let b = self.b;
        let mut depth = self.field(W_DEPTH);
        while p < end {
            let c = b[p];
            match c {
                b' ' => p += 1,
                b'[' => {
                    self.put(p, p + 1, BRACKET_BEGIN);
                    depth = (depth + 1).min(15);
                    p += 1;
                }
                b']' => {
                    self.put(p, p + 1, BRACKET_END);
                    depth = depth.saturating_sub(1);
                    p += 1;
                }
                b'|' => {
                    self.put(p, p + 1, PIPE);
                    p += 1;
                }
                b'<' => {
                    let q = self.param(p, end);
                    p = if q == p { p + 1 } else { q };
                }
                b'-' | b'+' if (p == 0 || !is_word(b[p - 1])) && option_end(b, p).is_some() => {
                    let e = option_end(b, p).unwrap_or(p + 1).min(end);
                    self.put(p, e, OPTION);
                    let q = self.value(e, end, false);
                    p = q.min(end).max(e);
                }
                _ => {
                    let e = p + b[p..end]
                        .iter()
                        .take_while(|&&c| !matches!(c, b' ' | b'[' | b']' | b'|' | b'<'))
                        .count();
                    let w = &b[p..e];
                    // a placeholder: an uppercase word, its `...` after it plain
                    let core = w.iter().take_while(|&&c| is_param(c) && c != b'.').count();
                    if core >= 1
                        && w[0].is_ascii_uppercase()
                        && w[..core].iter().all(|c| !c.is_ascii_lowercase())
                    {
                        self.put(p, p + core, PARAMETER);
                        p = e.max(p + 1);
                    } else {
                        // a reference read here may close past the word, `f('a', B)`
                        p = self.prose_in(p, e).max(e).max(p + 1);
                    }
                }
            }
        }
        self.set_field(W_DEPTH, depth);
        p
    }

    /// Prose's references and variables starting in `p..e`: where the last one read ends.
    fn prose_in(&mut self, mut p: usize, e: usize) -> usize {
        let b = self.b;
        while p < e {
            match PROSE[usize::from(b[p])] {
                P_PAREN => p = self.reference(p),
                P_DOLLAR => p = self.variable(p),
                _ => p += 1,
            }
        }
        p
    }
}

/// Whether a title's `NAME(SECTION)` names a library's page: section 2 or 3, but Perl's modules'
/// (`3perl`, `3pm`), whose synopses are Perl.
fn is_library(name: &[u8]) -> bool {
    let Some(open) = name.iter().rposition(|&c| c == b'(') else {
        return false;
    };
    let sect = &name[open + 1..name.len() - 1];
    matches!(sect.first(), Some(b'2' | b'3')) && !sect.ends_with(b"perl") && !sect.ends_with(b"pm")
}

/// Where the line's text ends, its blanks after it off.
fn trim_end(b: &[u8]) -> usize {
    b.iter().rposition(|&c| c != b' ').map_or(0, |i| i + 1)
}

fn skip_blank(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && b[p] == b' ' {
        p += 1;
    }
    p
}

/// Where the text from `p` reaches two blanks (a tag's end), or the line's end.
fn gap(b: &[u8], p: usize) -> usize {
    let mut i = p;
    while i < b.len() {
        if b[i] == b' ' && b.get(i + 1) == Some(&b' ') {
            return i;
        }
        i += 1;
    }
    b.len()
}

/// Whether an option starts at `p`: `-` or `--` (a tag's `+` too), then a letter or digit.
fn is_option_start(b: &[u8], p: usize) -> bool {
    option_end(b, p).is_some()
}

/// Where the option at `p` ends: `--?` or `+`, a letter or digit, then letters, digits, `_` and
/// `-` (bat's `--?[A-Za-z0-9][_A-Za-z0-9-]*`).
fn option_end(b: &[u8], p: usize) -> Option<usize> {
    let mut q = p;
    match b.get(q)? {
        b'-' => {
            q += 1;
            if b.get(q) == Some(&b'-') {
                q += 1;
            }
        }
        b'+' => q += 1,
        _ => return None,
    }
    if !b.get(q)?.is_ascii_alphanumeric() {
        return None;
    }
    Some(q + b[q..].iter().take_while(|&&c| is_word(c)).count())
}

/// A parameter word's bytes.
fn is_param(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'/' | b':')
}

/// A value's bytes after `=`: to a blank, a bracket, a comma or a parenthesis (bat's
/// `[^],.() ]+`, a `.` inside kept: `--name=a.b`).
fn is_value(c: u8) -> bool {
    !matches!(c, b' ' | b']' | b'[' | b',' | b'(' | b')')
}

fn is_env(c: u8) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_'
}

/// An ENVIRONMENT section's variable at `p`: an uppercase letter or `_`, and another of the
/// name's bytes (`LS_COLORS`, `TZ`; not a sentence's `A`).
fn is_env_start(b: &[u8], p: usize) -> bool {
    matches!(b[p], b'A'..=b'Z' | b'_') && b.get(p + 1).copied().is_some_and(is_env)
}

/// Whether a synopsis's line at the body's indent starts with a command's name.
fn is_command_start(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'/')
}

/// A section, as a reference's parentheses hold it: `1`, `3p`, `3perl`, `8`, `n`, `l`.
fn is_section(s: &[u8]) -> bool {
    match s {
        [b'n' | b'l'] => true,
        [d, rest @ ..] => {
            d.is_ascii_digit() && rest.len() <= 8 && rest.iter().all(u8::is_ascii_alphanumeric)
        }
        [] => false,
    }
}

/// The title's left part, `NAME(SECTION)`, where `b` is a title: the line ends with the same
/// text, a gap between them. Its length.
fn title_left(b: &[u8]) -> Option<usize> {
    let n = trim_end(b);
    let close = b[..n].iter().position(|&c| c == b')')? + 1;
    let open = b[..close].iter().position(|&c| c == b'(')?;
    if open == 0 || 2 * close + 1 > n || b[..close].contains(&b'\t') {
        return None;
    }
    (b[n - close..n] == b[..close] && b[close] == b' ' && b[n - close - 1] == b' ').then_some(close)
}

/// Where a footer's `NAME(SECTION)` starts, where `b` (trimmed) is one: text, a gap of two
/// blanks or more, and `NAME(SECTION)` at the end, its section short.
fn footer_right(b: &[u8]) -> Option<usize> {
    if b.last() != Some(&b')') {
        return None;
    }
    let start = b.iter().rposition(|&c| c == b' ')? + 1;
    let open = b[start..].iter().position(|&c| c == b'(')? + start;
    let sect = &b[open + 1..b.len() - 1];
    let gapped = start >= 2 && b[start - 2] == b' ';
    (open > start && !sect.is_empty() && sect.len() <= 10 && gapped).then_some(start)
}

/// A heading's section and kind, by its text: a SYNOPSIS (bat's translations, with German's,
/// Spanish's and Portuguese's), COMMANDS, ENVIRONMENT (bat's needs `ENVIRONMENT VARIABLES`), or
/// any other.
fn heading(b: &[u8]) -> (u64, u16) {
    const SYNOPSES: &[&[u8]] = &[
        b"SYNOPSIS",
        b"SYNTAX",
        b"USAGE",
        b"SINTASSI",
        "SKŁADNIA".as_bytes(),
        "СИНТАКСИС".as_bytes(),
        "書式".as_bytes(),
        "ÜBERSICHT".as_bytes(),
        b"SINOPSIS",
        b"SINOPSE",
    ];
    const ENVS: &[&[u8]] = &[b"ENVIRONMENT", b"UMGEBUNG", b"ENVIRONNEMENT", b"ENTORNO"];
    if SYNOPSES.iter().any(|s| b.starts_with(s)) {
        return (S_SYNOPSIS, HEADING_SYNOPSIS);
    }
    if b.starts_with(b"COMMANDS") && b.get(8).is_none_or(|c| !c.is_ascii_alphanumeric()) {
        return (S_COMMANDS, HEADING_COMMANDS);
    }
    if ENVS.iter().any(|s| b.starts_with(s)) {
        return (S_ENV, HEADING_ENV);
    }
    (S_OTHER, HEADING)
}

/// Whether C starts on a line whose text is `t`: bat's `#include`, `#define`, `/*` and
/// `struct …{`.
fn c_starts(t: &[u8]) -> bool {
    match t.first() {
        Some(b'#') => t.starts_with(b"#include") || t.starts_with(b"#define"),
        Some(b'/') => t.starts_with(b"/*"),
        Some(b's') if t.starts_with(b"struct") => {
            // `struct`, a blank or the `{`, an optional name, then `{`
            let r = &t[6..];
            if !matches!(r.first(), Some(b' ' | b'{')) {
                return false;
            }
            let name = r.iter().position(|&c| c != b' ').unwrap_or(r.len());
            let after = name
                + r[name..]
                    .iter()
                    .take_while(|&&c| c.is_ascii_alphanumeric() || c == b'_')
                    .count();
            r[after..].iter().find(|&&c| c != b' ') == Some(&b'{')
        }
        _ => false,
    }
}

/// Whether C ends at a line whose text is `t`: a sentence (an uppercase letter, then not
/// another) or a parenthesized note, `(… .)`, bat's escape.
fn c_ends(t: &[u8]) -> bool {
    match t {
        [c, d, ..] if c.is_ascii_uppercase() => !d.is_ascii_uppercase(),
        [c] if c.is_ascii_uppercase() => true,
        [b'(', ..] => {
            let e = trim_end(t);
            e >= 2 && t[e - 1] == b')' && t[e - 2] == b'.'
        }
        _ => false,
    }
}

/// A heading of a man page's outline (X-188): its line (from 0), its level (1 a section's, 2 a
/// subsection's) and its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Section<'t> {
    pub line: u32,
    pub level: u8,
    pub text: &'t str,
}

/// The headings of the man page `text`, in order, as the scanner reads them: sections at the
/// margin (not the title or the footer), subsections three columns in where the body is
/// deeper.
pub fn sections(text: &str) -> Vec<Section<'_>> {
    let mut found = Vec::new();
    let mut started = false;
    let mut indent = 0;
    let mut in_section = false;
    for (i, l) in text.split('\n').enumerate() {
        let l = l.strip_suffix('\r').unwrap_or(l);
        let b = l.as_bytes();
        let ind = b.iter().take_while(|&&c| c == b' ').count();
        if ind == b.len() {
            continue;
        }
        if !started {
            started = true;
            if ind == 0 && title_left(b).is_some() {
                continue;
            }
        }
        let line = u32::try_from(i).unwrap_or(u32::MAX);
        let n = trim_end(b);
        if ind == 0 {
            if footer_right(&b[..n]).is_some() {
                continue;
            }
            in_section = true;
            found.push(Section {
                line,
                level: 1,
                text: &l[..n],
            });
        } else if ind == 3 && (indent == 0 || indent > 3) && in_section {
            found.push(Section {
                line,
                level: 2,
                text: &l[3..n],
            });
        } else if indent == 0 && in_section {
            indent = ind;
        }
    }
    found
}

/// Whether `text` reads as a man page as `man` shows it: its first line a title
/// (`LS(1)  User Commands  LS(1)`) and a section's heading at the margin among the next few.
pub fn is_page(text: &str) -> bool {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let Some(first) = lines.next() else {
        return false;
    };
    if title_left(first.trim_end_matches('\r').as_bytes()).is_none() {
        return false;
    }
    lines.take(3).any(|l| {
        let b = l.as_bytes();
        b.first().is_some_and(u8::is_ascii_uppercase)
            && footer_right(b).is_none()
            && b.iter().take_while(|c| !c.is_ascii_whitespace()).count() >= 2
    })
}

#[cfg(test)]
mod tests;
