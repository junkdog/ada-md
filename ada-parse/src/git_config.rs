//! Git's config files (`.gitconfig`, `.git/config`, `.gitmodules`), in place of bat's `Git
//! Config`, on the line-format engine's cursor (X-177): ada's own reading, as git's `config.c`
//! reads a file (each corner checked with `git config -f … --list`, git 2.56).
//!
//! A line is blank, a comment (`#` or `;`), a section's header or a variable, and a header can
//! have more after it on its line (`[core] editor = vim`, `[a] [b]`). A header is `[name]`, its
//! name `[A-Za-z0-9.-]`, with a subsection as `[name "sub"]` (blanks, then a quoted name whose
//! `\` takes any one character) or the old `[name.sub]`. A variable is a name (a letter, then
//! letters, digits and `-`), then nothing (a boolean's true) or blanks, `=` and a value; anything
//! else is git's error, read as such to the line's end. A header's `[` or a quote left open at
//! the line's end is git's error too, shown on the mark that opened it.
//!
//! **A value** as `parse_value` reads it: `"` opens and closes a quote anywhere in it (`a"b c"d`),
//! `#` and `;` outside a quote start a comment wherever they stand (`x#y` is `x`), `\t`, `\b`,
//! `\n`, `\\` and `\"` are escapes and any other `\` git's error, and a `\` that ends a line joins
//! the next to the value, in a quote or out of one. What a value is read as is its key's: a
//! command git runs through the shell (a `!` alias, `core.pager`, `core.editor`, a diff driver's
//! `textconv`, a filter's `clean`, `credential.helper`, a trailer's `cmd`, …) is bash's scanner
//! on the text the shell gets (git's quotes and escapes taken out, a `!` that starts an alias's
//! or a credential helper's text the format's, a filter's or a merge driver's `%f`, `%A`, …
//! git's placeholders), its spans laid back over the line's bytes. A shell comment a `\`
//! continues goes on to the value's end, git joining the lines before the shell reads them. An
//! alias without a `!` is a git command line, which git splits itself: bash's scanner on it with
//! the shell's operators made its text. A key of a `[color "…"]` section is a list of color
//! words (`highlightRecent`'s a color and a date in turn, between commas); a remote's or a
//! submodule's `url` and a `[url "…"]` section's `insteadOf` a link; any other value is text, a
//! boolean or a number where it is that alone.
//!
//! **The state:** the section's reading is carried in `arg`'s low byte (which keys run
//! commands), what a line leaves open above it (a value continued by a `\`: its reading and
//! whether a quote is open). A command continued that way keeps bash's state in the state's
//! levels, `arg` being bash's unused field; any other line leaves them empty.
//!
//! **Departures from bat's:** bat reads a `#` or `;` after an unquoted value's text as the
//! value's, and one in a quote as a comment (`"#00ff00"`); text after a quote, and a variable
//! after a header on its line, as illegal; a bare key plain; a command only after a `!`. Keys
//! here take TOML's `entity.name.tag`, where bat's `variable.other.readwrite` is the plain text's
//! color in all 14 themes.
//!
//! **Kinds are leaves:** the shell's kinds are bash's ([`crate::bash::scopes`]), below
//! [`GITC`]; the format's own are from it.

use crate::emit::{Push, Runs, Sink};
use crate::line_format::{BLANK, Cursor, blanks, find};
use crate::{Out, Span, State};

/// The format's kinds are this plus their index in [`KINDS`]; bash's are below it, and bash's
/// twins ([`crate::bash::TWIN`]) above the format's.
pub const GITC: u16 = 0x100;

macro_rules! kinds {
    ($($k:ident = $s:literal,)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms, dead_code)]
        #[derive(Clone, Copy)]
        #[repr(u16)]
        enum KindId { $($k,)* Count }
        /// The kinds, each a stack of scopes under `text.git.config`.
        mod k {
            $(pub const $k: u16 = super::GITC + super::KindId::$k as u16;)*
        }
        /// Each kind's scopes.
        static KINDS: [&str; KindId::Count as usize] = [$($s,)*];
    };
}

kinds! {
    COMMENT_MARK = "comment.line.git.config punctuation.definition.comment.git.config",
    COMMENT = "comment.line.git.config",
    BRACKET_BEGIN = "meta.brackets.git.config punctuation.definition.brackets.begin.git.config",
    BRACKET_END = "meta.brackets.git.config punctuation.definition.brackets.end.git.config",
    SECTION = "meta.brackets.git.config entity.name.section.git.config",
    DOT = "meta.brackets.git.config punctuation.accessor.dot.git.config",
    SUBSECTION = "meta.brackets.git.config string.unquoted.git.config",
    SUB_BEGIN = "meta.brackets.git.config string.quoted.double.git.config punctuation.definition.string.begin.git.config",
    SUB_TEXT = "meta.brackets.git.config string.quoted.double.git.config",
    SUB_ESCAPE = "meta.brackets.git.config string.quoted.double.git.config constant.character.escape.git.config",
    SUB_END = "meta.brackets.git.config string.quoted.double.git.config punctuation.definition.string.end.git.config",
    KEY = "meta.mapping.key.git.config entity.name.tag.git.config",
    ASSIGN = "keyword.operator.assignment.git.config",
    TEXT = "string.unquoted.value.git.config",
    QUOTE_BEGIN = "string.quoted.double.git.config punctuation.definition.string.begin.git.config",
    QUOTED = "string.quoted.double.git.config",
    QUOTE_END = "string.quoted.double.git.config punctuation.definition.string.end.git.config",
    ESCAPE = "constant.character.escape.git.config",
    CONTINUATION = "punctuation.separator.continuation.line.git.config",
    BOOLEAN = "constant.language.boolean.git.config",
    NUMBER = "constant.numeric.integer.decimal.git.config",
    COLOR = "support.constant.color.git.config",
    COLOR_ATTRIBUTE = "support.constant.color-attribute.git.config",
    COLOR_VALUE = "constant.other.color.rgb-value.git.config",
    COLOR_RESET = "support.constant.color-reset.git.config",
    SHELL_MARK = "keyword.control.import.shell.git.config",
    PLACEHOLDER = "constant.other.placeholder.git.config",
    LINK = "markup.underline.link.git.config",
    INVALID = "invalid.illegal.git.config",
}

/// The scopes under `text.git.config` a kind carries, outermost first: the format's own, or
/// bash's for the shell's.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    match kind.checked_sub(GITC).map(usize::from) {
        Some(i) if i < KINDS.len() => KINDS[i].split(' ').collect(),
        _ => crate::bash::scopes(kind),
    }
}

/// Lex `b`, a line of a Git config file without its `\n`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    scan(b, state, out, &mut Push);
}

/// [`line()`]'s lexemes written to `k`, a run sink.
/// Inline in a block's loop, [`crate::lex_runs`]' line too (X-050).
pub(crate) struct RunLine;

impl crate::LineScan for RunLine {
    #[inline(always)]
    fn scan(b: &[u8], state: &mut State, out: &mut Out, k: &mut Runs) {
        scan(b, state, out, k);
    }
}

// The section's reading, `arg`'s low byte: which of these it is, and whether it has a
// subsection. Only the sections whose keys git runs as commands, or reads as colors, are told
// apart.
const S_OTHER: u8 = 0;
const S_ALIAS: u8 = 1;
const S_COLOR: u8 = 2;
const S_CORE: u8 = 3;
const S_SEQUENCE: u8 = 4;
const S_INTERACTIVE: u8 = 5;
const S_DIFF: u8 = 6;
const S_MERGE: u8 = 7;
const S_FILTER: u8 = 8;
const S_DIFFTOOL: u8 = 9;
const S_MERGETOOL: u8 = 10;
const S_PAGER: u8 = 11;
const S_CREDENTIAL: u8 = 12;
const S_GPG: u8 = 13;
const S_SENDEMAIL: u8 = 14;
const S_BROWSER: u8 = 15;
const S_MAN: u8 = 16;
const S_REMOTE: u8 = 17;
const S_HOOK: u8 = 18;
const S_UPLOADPACK: u8 = 19;
const S_SUBMODULE: u8 = 20;
const S_URL: u8 = 21;
const S_TRAILER: u8 = 22;
const S_GUITOOL: u8 = 23;
const S_GC: u8 = 24;
const S_MASK: u8 = 0x3f;
/// The section has a subsection (`[diff "x"]`, `[diff.x]`).
const SUB: u8 = 0x40;

static SECTIONS: &[(&[u8], u8)] = &[
    (b"alias", S_ALIAS),
    (b"color", S_COLOR),
    (b"core", S_CORE),
    (b"sequence", S_SEQUENCE),
    (b"interactive", S_INTERACTIVE),
    (b"diff", S_DIFF),
    (b"merge", S_MERGE),
    (b"filter", S_FILTER),
    (b"difftool", S_DIFFTOOL),
    (b"mergetool", S_MERGETOOL),
    (b"pager", S_PAGER),
    (b"credential", S_CREDENTIAL),
    (b"gpg", S_GPG),
    (b"sendemail", S_SENDEMAIL),
    (b"browser", S_BROWSER),
    (b"man", S_MAN),
    (b"remote", S_REMOTE),
    (b"hook", S_HOOK),
    (b"uploadpack", S_UPLOADPACK),
    (b"submodule", S_SUBMODULE),
    (b"url", S_URL),
    (b"trailer", S_TRAILER),
    (b"guitool", S_GUITOOL),
    (b"gc", S_GC),
];

/// A section's name's reading, ASCII case ignored, as git folds it.
fn section_id(name: &[u8]) -> u8 {
    SECTIONS
        .iter()
        .find(|(s, _)| s.eq_ignore_ascii_case(name))
        .map_or(S_OTHER, |&(_, id)| id)
}

// What a value is read as, an open value's reading in `arg`'s high byte
/// Text: a boolean or a number where the value is that alone.
const V_TEXT: u8 = 1;
/// Color words (a `[color "…"]` section's keys).
const V_COLOR: u8 = 2;
/// A link (a remote's or a submodule's `url`, a `[url "…"]` section's `insteadOf`).
const V_LINK: u8 = 3;
/// `color.blame.highlightRecent`'s colors and dates (X-179), items between commas, a color's
/// first and then a date's in turn: `blue,12 month ago,white`.
const V_RECENT: u8 = 4;
/// A command run through the shell: bash's scanner on the shell's text. This and the readings
/// after it are commands, [`shell`]'s.
const V_SHELL: u8 = 5;
/// A credential helper whose text hasn't started: a `!` first is the format's, and either way
/// the rest is [`V_SHELL`]'s (git runs a helper through the shell, `git credential-` before a
/// name).
const V_BANG: u8 = 6;
/// A filter's or a merge driver's command, whose `%` and a letter are git's placeholders
/// (`%f`, `%O`, `%A`, `%B`).
const V_DRIVER: u8 = 7;
/// An alias whose text hasn't started: a `!` first is the format's and the rest [`V_SHELL`]'s,
/// and without one the rest is [`V_GIT`]'s.
const V_ALIAS: u8 = 8;
/// A git command line, a non-`!` alias's (X-179): git splits it into its arguments itself
/// (`split_cmdline`, quotes and `\` as the shell has them) and runs no shell, so `|`, `;`,
/// `$` and `#` are its arguments' text, not the shell's.
const V_GIT: u8 = 9;
/// A shell's comment that a `\` continued (X-179): git joins the lines before the shell gets
/// them, so the comment goes on to the value's end.
const V_REM: u8 = 10;
const V_MASK: u8 = 0x0f;
/// The value continues in a quote.
const IN_QUOTE: u8 = 0x10;

/// What key `key` of a section read as `sect` holds, ASCII case ignored.
fn key_reading(sect: u8, key: &[u8]) -> u8 {
    let is = |w: &[u8]| key.eq_ignore_ascii_case(w);
    let sub = sect & SUB != 0;
    let shell = match (sect & S_MASK, sub) {
        (S_ALIAS, _) => return V_ALIAS,
        (S_CREDENTIAL, _) if is(b"helper") => return V_BANG,
        (S_COLOR, true) if is(b"highlightRecent") => return V_RECENT,
        (S_COLOR, true) => return V_COLOR,
        (S_REMOTE | S_SUBMODULE, true) if is(b"url") || is(b"pushurl") => return V_LINK,
        (S_URL, true) if is(b"insteadOf") || is(b"pushInsteadOf") => return V_LINK,
        (S_MERGE, true) if is(b"driver") => return V_DRIVER,
        (S_FILTER, true) if is(b"clean") || is(b"smudge") || is(b"process") => return V_DRIVER,
        (S_PAGER, _) => true,
        (S_CORE, false) => {
            is(b"editor")
                || is(b"pager")
                || is(b"sshCommand")
                || is(b"askPass")
                || is(b"gitProxy")
                || is(b"fsmonitor")
                || is(b"alternateRefsCommand")
        }
        (S_SEQUENCE, false) => is(b"editor"),
        (S_INTERACTIVE, false) => is(b"diffFilter"),
        (S_DIFF, false) => is(b"external"),
        (S_DIFF, true) => is(b"command") || is(b"textconv"),
        (S_DIFFTOOL | S_MERGETOOL | S_BROWSER | S_MAN, true) => is(b"cmd"),
        (S_GPG, _) => is(b"program") || sub && is(b"defaultKeyCommand"),
        (S_SENDEMAIL, _) => is(b"sendmailCmd") || is(b"toCmd") || is(b"ccCmd"),
        (S_REMOTE, true) => is(b"uploadpack") || is(b"receivepack"),
        (S_UPLOADPACK, false) => is(b"packObjectsHook"),
        (S_HOOK, true) => is(b"command"),
        (S_TRAILER, true) => is(b"cmd") || is(b"command"),
        (S_GUITOOL, true) => is(b"cmd"),
        (S_GC, false) => is(b"recentObjectsHook"),
        _ => false,
    };
    if shell { V_SHELL } else { V_TEXT }
}

/// A line from `state`: the section's reading and what the line before left open in `arg`,
/// bash's levels under an open command.
#[inline(always)]
fn scan<K: Sink>(b: &[u8], state: &mut State, out: &mut Out, k: &mut K) {
    let mut sect = state.arg as u8;
    let open = (state.arg >> 8) as u8;
    let mut c = Cursor::new(b, out, k);
    let mut sh = State::default();
    let open = match open & V_MASK {
        0 => fresh(&mut c, &mut sect, &mut sh),
        v if v >= V_SHELL => {
            sh = State { arg: 0, ..*state };
            shell(&mut c, 0, v, open & IN_QUOTE != 0, &mut sh)
        }
        v => text(&mut c, 0, v, open & IN_QUOTE != 0),
    };
    if open & V_MASK < V_SHELL {
        sh = State::default();
    }
    sh.arg = u16::from(sect) | u16::from(open) << 8;
    *state = sh;
}

/// A line with nothing open before it: what it leaves open.
#[inline(always)]
fn fresh<K: Sink>(c: &mut Cursor<K>, sect: &mut u8, sh: &mut State) -> u8 {
    let b = c.b;
    let n = b.len();
    // the byte-order mark git skips at a file's start
    let mut p = if b.starts_with(b"\xEF\xBB\xBF") { 3 } else { 0 };
    loop {
        p = blanks(b, p);
        if p == n {
            return 0;
        }
        match b[p] {
            b'#' | b';' => {
                comment(c, p);
                return 0;
            }
            b'[' => match header(c, p, sect) {
                Some(q) => p = q,
                None => return 0,
            },
            x if x.is_ascii_alphabetic() => return variable(c, p, *sect, sh),
            _ => {
                invalid(c, p);
                return 0;
            }
        }
    }
}

/// A comment from its mark at `p` to the line's end.
#[inline(never)]
fn comment<K: Sink>(c: &mut Cursor<K>, p: usize) {
    c.skip(p);
    c.put(p + 1, k::COMMENT_MARK);
    c.put(c.b.len(), k::COMMENT);
}

/// git's error from `p` to the line's end (it stops reading the file there).
#[inline(never)]
fn invalid<K: Sink>(c: &mut Cursor<K>, p: usize) {
    c.skip(p);
    c.put(c.b.len(), k::INVALID);
}

/// How many bytes the character whose first byte is `x` takes.
#[inline(always)]
fn char_len(x: u8) -> usize {
    match x {
        0xF0.. => 4,
        0xE0.. => 3,
        0xC0.. => 2,
        _ => 1,
    }
}

/// A section's header from its `[` at `p`: the section's reading into `sect`, and where the
/// line goes on after its `]` (`None` where git's error ends it). What the line's end leaves
/// open is git's error too, and shows where it opened (X-179): the `[` of a header with no `]`
/// (`[core`, `[remote "origin"`), the `"` of a subsection with no closing one (`[remote "o`).
#[inline(never)]
fn header<K: Sink>(c: &mut Cursor<K>, p: usize, sect: &mut u8) -> Option<usize> {
    let b = c.b;
    let n = b.len();
    let s = p + 1;
    let mut q = s;
    while q < n && (b[q].is_ascii_alphanumeric() || b[q] == b'-' || b[q] == b'.') {
        q += 1;
    }
    // a subsection's quote at `r`, and where its closing quote leaves it; what follows the
    // name, read ahead so that a `[` or a `"` left open shows as git's error
    let r = blanks(b, q);
    let quote = (q > s && r > q && b.get(r) == Some(&b'"')).then(|| (r, quote_end(b, r + 1)));
    let open = match (b.get(q), quote) {
        (None, _) => true,
        (Some(b' ' | b'\t'), None) => r == n,
        (_, Some((_, Some(t)))) => t == n,
        _ => false,
    };
    c.skip(p);
    c.put(s, if open { k::INVALID } else { k::BRACKET_BEGIN });
    // the old `[name.sub]`: the subsection from the first `.`
    let dot = b[s..q].iter().position(|&y| y == b'.').map(|i| s + i);
    let name = dot.unwrap_or(q);
    c.put(name, k::SECTION);
    if let Some(d) = dot {
        c.put(d + 1, k::DOT);
        c.put(q, k::SUBSECTION);
    }
    *sect = section_id(&b[s..name]) | if dot.is_some() { SUB } else { 0 };
    match b.get(q) {
        Some(b']') if q > s => {
            c.put(q + 1, k::BRACKET_END);
            Some(q + 1)
        }
        Some(b' ' | b'\t') if q > s => {
            let Some((r, end)) = quote else {
                invalid(c, r);
                return None;
            };
            *sect |= SUB;
            c.skip(r);
            c.put(
                r + 1,
                if end.is_some() {
                    k::SUB_BEGIN
                } else {
                    k::INVALID
                },
            );
            let mut t = r + 1;
            loop {
                let e = find(b, t, *b"\"\\");
                c.put(e, k::SUB_TEXT);
                if e == n {
                    // no closing quote: git's error, at the line's end
                    return None;
                }
                if b[e] == b'"' {
                    c.put(e + 1, k::SUB_END);
                    t = e + 1;
                    break;
                }
                // `\` and the character after it, whatever it is
                if e + 1 == n {
                    invalid(c, e);
                    return None;
                }
                t = e + 1 + char_len(b[e + 1]);
                c.put(t, k::SUB_ESCAPE);
            }
            if b.get(t) == Some(&b']') {
                c.put(t + 1, k::BRACKET_END);
                Some(t + 1)
            } else {
                invalid(c, t);
                None
            }
        }
        _ => {
            *sect = S_OTHER;
            invalid(c, q);
            None
        }
    }
}

/// Where a header's subsection's quote that opens before `t` closes, past its `"`: `None`
/// when the line ends first, or in a `\` (git takes any character after one).
fn quote_end(b: &[u8], mut t: usize) -> Option<usize> {
    loop {
        let e = find(b, t, *b"\"\\");
        match b.get(e)? {
            b'"' => return Some(e + 1),
            _ if e + 1 == b.len() => return None,
            _ => t = e + 1 + char_len(b[e + 1]),
        }
    }
}

/// Whether a value's quote that opens before `t` closes on its line, or goes on to the next
/// (a `\` at the line's end); a quote left open at a value's end is git's error (X-179). And
/// the first `"` or `\` from `t`, where the quote's reading stops next.
fn value_quote_closes(b: &[u8], t: usize) -> (bool, usize) {
    let first = find(b, t, *b"\"\\");
    let mut e = first;
    loop {
        match b.get(e) {
            None => return (false, first),
            Some(b'"') => return (true, first),
            _ => match escape(b, e) {
                Escape::Continuation => return (true, first),
                Escape::Valid(_) => e = find(b, e + 2, *b"\"\\"),
                Escape::Invalid(len) => e = find(b, e + len, *b"\"\\"),
            },
        }
    }
}

/// A variable from its name's first letter at `p`: what the line leaves open.
#[inline(never)]
fn variable<K: Sink>(c: &mut Cursor<K>, p: usize, sect: u8, sh: &mut State) -> u8 {
    let b = c.b;
    let n = b.len();
    let mut q = p + 1;
    while q < n && (b[q].is_ascii_alphanumeric() || b[q] == b'-') {
        q += 1;
    }
    c.skip(p);
    c.put(q, k::KEY);
    let r = blanks(b, q);
    if r == n {
        // a key alone: a boolean's true
        return 0;
    }
    if b[r] != b'=' {
        invalid(c, r);
        return 0;
    }
    c.skip(r);
    c.put(r + 1, k::ASSIGN);
    let v = blanks(b, r + 1);
    c.skip(v);
    let reading = key_reading(sect, &b[p..q]);
    if (reading == V_TEXT || reading == V_SHELL)
        && let Some((e, kind)) = word(b, v)
    {
        c.put(e, kind);
        let r = blanks(b, e);
        if r < n {
            comment(c, r);
        }
        return 0;
    }
    if reading >= V_SHELL {
        shell(c, v, reading, false, sh)
    } else {
        text(c, v, reading, false)
    }
}

/// A value at `v` that is a boolean or a number alone (then blanks, a comment or the line's
/// end): its end and its kind.
#[inline(always)]
fn word(b: &[u8], v: usize) -> Option<(usize, u16)> {
    let n = b.len();
    let minus = b.get(v) == Some(&b'-');
    let mut e = v + usize::from(minus);
    while e < n && b[e].is_ascii_alphanumeric() {
        e += 1;
    }
    let r = blanks(b, e);
    if e == v + usize::from(minus) || r < n && b[r] != b'#' && b[r] != b';' {
        return None;
    }
    let w = &b[v..e];
    if !minus && is_boolean(w) {
        Some((e, k::BOOLEAN))
    } else if is_number(&w[usize::from(minus)..]) {
        Some((e, k::NUMBER))
    } else {
        None
    }
}

/// git's booleans (`git_parse_maybe_bool`), and the `auto`, `always` and `never` of its
/// tristate settings (`color.ui`, `column.ui`).
fn is_boolean(w: &[u8]) -> bool {
    [
        &b"true"[..],
        b"false",
        b"yes",
        b"no",
        b"on",
        b"off",
        b"auto",
        b"always",
        b"never",
    ]
    .iter()
    .any(|x| x.eq_ignore_ascii_case(w))
}

/// Digits with a unit (`k`, `m`, `g`) or without, as `git_parse_signed` reads them.
fn is_number(w: &[u8]) -> bool {
    let digits = match w.last() {
        Some(b'k' | b'K' | b'm' | b'M' | b'g' | b'G') => &w[..w.len() - 1],
        _ => w,
    };
    !digits.is_empty() && digits.iter().all(u8::is_ascii_digit)
}

/// What a `\` at `p` in a value is.
enum Escape {
    /// It ends the line (a `\r` after it aside): the next line is the value's.
    Continuation,
    /// One of git's: the byte it stands for.
    Valid(u8),
    /// Any other: git's error, `\` and the character after it, so many bytes.
    Invalid(usize),
}

#[inline(always)]
fn escape(b: &[u8], p: usize) -> Escape {
    match b.get(p + 1) {
        None => Escape::Continuation,
        Some(b'\r') if p + 2 == b.len() => Escape::Continuation,
        Some(&y) if matches!(y, b'"' | b'\\' | b't' | b'b' | b'n') => Escape::Valid(y),
        Some(&y) => Escape::Invalid(1 + char_len(y)),
    }
}

/// A value read as text, colors or a link (`reading`) from `p`, in a quote where `quoted`: what
/// the line leaves open.
#[inline(never)]
fn text<K: Sink>(c: &mut Cursor<K>, mut p: usize, reading: u8, mut quoted: bool) -> u8 {
    let b = c.b;
    let n = b.len();
    let link = reading == V_LINK;
    // whether `highlightRecent`'s item is a date's, a color's first
    let mut date = false;
    // where a quote's reading stops next, found when it opened
    let mut next = None;
    loop {
        let e = if let Some(e) = next.take() {
            e
        } else if quoted {
            find(b, p, *b"\"\\")
        } else {
            find(b, p, *b"\"\\#;")
        };
        if reading == V_COLOR {
            colors(c, p, e, quoted);
        } else if reading == V_RECENT {
            recent(c, p, e, quoted, &mut date);
        } else {
            c.put(
                e,
                if link {
                    k::LINK
                } else if quoted {
                    k::QUOTED
                } else {
                    k::TEXT
                },
            );
        }
        if e == n {
            // a quote open here is git's error; the line is read as it stands
            return 0;
        }
        match b[e] {
            b'"' => {
                let kind = match quoted {
                    true => k::QUOTE_END,
                    false => {
                        let (closes, stop) = value_quote_closes(b, e + 1);
                        next = Some(stop);
                        if closes { k::QUOTE_BEGIN } else { k::INVALID }
                    }
                };
                c.put(e + 1, kind);
                quoted = !quoted;
                p = e + 1;
            }
            b'\\' => match escape(b, e) {
                Escape::Continuation => {
                    c.put(e + 1, k::CONTINUATION);
                    return reading | if quoted { IN_QUOTE } else { 0 };
                }
                Escape::Valid(_) => {
                    c.put(e + 2, k::ESCAPE);
                    p = e + 2;
                }
                Escape::Invalid(len) => {
                    c.put(e + len, k::INVALID);
                    p = e + len;
                }
            },
            _ => {
                comment(c, e);
                return 0;
            }
        }
    }
}

/// `b[s..e]` of a color value: its words, each a color's or an attribute's where git's color
/// parser reads it so, the others text.
#[inline(never)]
fn colors<K: Sink>(c: &mut Cursor<K>, s: usize, e: usize, quoted: bool) {
    let b = c.b;
    let mut q = s;
    while q < e {
        if BLANK[usize::from(b[q])] {
            q += 1;
            continue;
        }
        let mut w = q;
        while w < e && !BLANK[usize::from(b[w])] {
            w += 1;
        }
        let kind = color_word(&b[q..w]).unwrap_or(if quoted { k::QUOTED } else { k::TEXT });
        c.skip(q);
        c.put(w, kind);
        q = w;
    }
    c.skip(e);
}

/// `b[s..e]` of `color.blame.highlightRecent`'s value (X-179), as `parse_color_fields` reads
/// it: items between commas, a color's words and a date's text in turn, `date` whether the item
/// `s` is in is a date's.
#[inline(never)]
fn recent<K: Sink>(c: &mut Cursor<K>, s: usize, e: usize, quoted: bool, date: &mut bool) {
    let b = c.b;
    let mut q = s;
    loop {
        let end = q + b[q..e].iter().position(|&y| y == b',').unwrap_or(e - q);
        if *date {
            c.put(end, if quoted { k::QUOTED } else { k::TEXT });
        } else {
            colors(c, q, end, quoted);
        }
        if end == e {
            return;
        }
        c.skip(end + 1);
        *date = !*date;
        q = end + 1;
    }
}

/// A word of a color value as git's `color_parse_mem` reads it, ASCII case ignored: a color's
/// name (`bright` before it too), a number from -1 to 255 or `#` and three or six hex digits, an
/// attribute (`no` or `no-` before it too), `reset`.
fn color_word(w: &[u8]) -> Option<u16> {
    const NAMES: [&[u8]; 10] = [
        b"normal", b"default", b"black", b"red", b"green", b"yellow", b"blue", b"magenta", b"cyan",
        b"white",
    ];
    const ATTRIBUTES: [&[u8]; 7] = [
        b"bold", b"dim", b"italic", b"ul", b"blink", b"reverse", b"strike",
    ];
    let is = |list: &[&[u8]], w: &[u8]| list.iter().any(|x| x.eq_ignore_ascii_case(w));
    let bright = w.len() > 6 && w[..6].eq_ignore_ascii_case(b"bright");
    if is(&NAMES, w) || bright && is(&NAMES, &w[6..]) {
        return Some(k::COLOR);
    }
    let no = if w.len() > 3 && w[..3].eq_ignore_ascii_case(b"no-") {
        3
    } else if w.len() > 2 && w[..2].eq_ignore_ascii_case(b"no") {
        2
    } else {
        0
    };
    if is(&ATTRIBUTES, &w[no..]) {
        return Some(k::COLOR_ATTRIBUTE);
    }
    if w.eq_ignore_ascii_case(b"reset") {
        return Some(k::COLOR_RESET);
    }
    let number = match w {
        [b'-', b'1'] => true,
        [..] if !w.is_empty() && w.len() <= 3 && w.iter().all(u8::is_ascii_digit) => {
            w.iter().fold(0u32, |a, &d| a * 10 + u32::from(d - b'0')) <= 255
        }
        [b'#', hex @ ..] => {
            (hex.len() == 3 || hex.len() == 6) && hex.iter().all(u8::is_ascii_hexdigit)
        }
        _ => false,
    };
    number.then_some(k::COLOR_VALUE)
}

/// A command's text, as git reads its value and as the shell gets it.
#[derive(Default)]
struct ShellText {
    /// The text the shell gets: git's quotes and escapes taken out.
    cooked: Vec<u8>,
    /// Where each of `cooked`'s bytes is in the line: its first byte and the one past its last.
    at: Vec<(u32, u32)>,
    /// git's own tokens in the value (quotes, its escapes that aren't the shell's text, `!`),
    /// laid over bash's spans: start, end, kind.
    marks: Vec<(u32, u32, u16)>,
    /// bash's spans of `cooked`.
    spans: Vec<Span>,
    /// Each of the value's bytes' kind.
    kinds: Vec<u16>,
}

impl ShellText {
    #[inline(always)]
    fn push(&mut self, y: u8, s: usize, e: usize) {
        self.cooked.push(y);
        self.at.push((s as u32, e as u32));
    }
}

std::thread_local! {
    static SHELL: std::cell::RefCell<ShellText> = std::cell::RefCell::new(ShellText::default());
}

/// A byte no kind is: plain.
const NONE: u16 = u16::MAX;

/// A command's value from `p` (`reading` [`V_SHELL`] or one after it), in a quote where
/// `quoted`, bash's state in `sh`: what the line leaves open. git's reading of the line first
/// (its quotes, escapes, comment and continuation), then bash's scanner on the text that leaves,
/// its spans mapped back to the line's bytes and git's tokens laid over them. A git command line
/// ([`V_GIT`]) is bash's scanner on its text with the shell's operators made words; a comment
/// continued ([`V_REM`]) is a comment to the value's end.
#[inline(never)]
fn shell<K: Sink>(
    c: &mut Cursor<K>,
    p: usize,
    mut reading: u8,
    mut quoted: bool,
    sh: &mut State,
) -> u8 {
    let b = c.b;
    let n = b.len();
    let continued = SHELL.with_borrow_mut(|v| {
        v.cooked.clear();
        v.at.clear();
        v.marks.clear();
        v.spans.clear();
        let mut q = p;
        // where the value's text ends, and whether a comment or a continuation follows
        let mut end = n;
        let mut continued = false;
        // the mark of a quote opened on this line and open still
        let mut opened = None;
        while q < n {
            match b[q] {
                b'"' => {
                    let kind = if quoted { k::QUOTE_END } else { k::QUOTE_BEGIN };
                    opened = (!quoted).then_some(v.marks.len());
                    v.marks.push((q as u32, q as u32 + 1, kind));
                    quoted = !quoted;
                    q += 1;
                }
                b'\\' => match escape(b, q) {
                    Escape::Continuation => {
                        end = q;
                        continued = true;
                        break;
                    }
                    Escape::Valid(y @ (b'"' | b'\\')) => {
                        v.push(y, q, q + 2);
                        q += 2;
                    }
                    Escape::Valid(y) => {
                        // a tab, a backspace or a line break in the command: a blank or a `;` to
                        // the shell, git's escape to the eye
                        v.push(if y == b'n' { b';' } else { b' ' }, q, q + 2);
                        v.marks.push((q as u32, q as u32 + 2, k::ESCAPE));
                        q += 2;
                    }
                    Escape::Invalid(len) => {
                        let e = (q + len).min(n);
                        v.push(b' ', q, e);
                        v.marks.push((q as u32, e as u32, k::INVALID));
                        q = e;
                    }
                },
                b'#' | b';' if !quoted => {
                    end = q;
                    break;
                }
                b'%' if reading == V_DRIVER
                    && b.get(q + 1).is_some_and(u8::is_ascii_alphabetic) =>
                {
                    // a word to the shell
                    v.push(b'x', q, q + 1);
                    v.push(b'x', q + 1, q + 2);
                    v.marks.push((q as u32, q as u32 + 2, k::PLACEHOLDER));
                    q += 2;
                }
                y => {
                    v.push(y, q, q + 1);
                    q += 1;
                }
            }
        }
        // a quote left open at the value's end: git's error, where it opened (X-179)
        if quoted
            && !continued
            && let Some(i) = opened
        {
            v.marks[i].2 = k::INVALID;
        }
        // an alias's or a credential helper's text: a `!` first makes it a shell's, and an
        // alias without one a git command line
        let mut from = 0;
        if matches!(reading, V_BANG | V_ALIAS)
            && let Some(i) = v.cooked.iter().position(|&y| !BLANK[usize::from(y)])
        {
            let bang = v.cooked[i] == b'!';
            if bang {
                let (s, e) = v.at[i];
                v.marks.push((s, e, k::SHELL_MARK));
                from = i + 1;
            }
            reading = if bang || reading == V_BANG {
                V_SHELL
            } else {
                V_GIT
            };
        }
        if reading == V_GIT {
            // the shell's operators, expansions and comments: an argument's text to git
            for y in &mut v.cooked[from..] {
                if matches!(
                    y,
                    b'|' | b'&' | b';' | b'<' | b'>' | b'(' | b')' | b'$' | b'`'
                ) || matches!(y, b'#' | b'{' | b'}' | b'~' | b'!' | b'*' | b'?' | b'[')
                {
                    *y = b'x';
                }
            }
        }
        if continued {
            // the shell's own continuation, so that bash reads the next line as this one's
            v.push(b'\\', end, end + 1);
        }
        let (_, rem) = crate::bash::PLAIN_COMMENT;
        if reading == V_REM {
            let len = v.cooked.len() as u32;
            v.spans.push(Span {
                start: 0,
                end: len,
                kind: rem,
            });
        } else if from < v.cooked.len() {
            crate::bash::line_spans(&v.cooked[from..], sh, &mut Out::new(&mut v.spans));
            // a comment the `\` continues: git joins the next line to it
            let to = (v.cooked.len() - from) as u32;
            if continued && v.spans.last().is_some_and(|l| l.kind == rem && l.end == to) {
                reading = V_REM;
            }
        }
        let to = if continued { end + 1 } else { end };
        v.kinds.clear();
        v.kinds.resize(to - p, NONE);
        for sp in &v.spans {
            for i in sp.start as usize..sp.end as usize {
                let (s, e) = v.at[from + i];
                v.kinds[s as usize - p..e as usize - p].fill(sp.kind);
            }
        }
        for &(s, e, kind) in &v.marks {
            v.kinds[s as usize - p..e as usize - p].fill(kind);
        }
        if continued {
            v.kinds[end - p] = k::CONTINUATION;
        }
        c.skip(p);
        let mut i = 0;
        while i < v.kinds.len() {
            let kind = v.kinds[i];
            let mut j = i + 1;
            while j < v.kinds.len() && v.kinds[j] == kind {
                j += 1;
            }
            if kind == NONE {
                c.skip(p + j);
            } else {
                c.put(p + j, kind);
            }
            i = j;
        }
        if end < n && !continued {
            comment(c, end);
        }
        continued
    });
    if continued {
        reading | if quoted { IN_QUOTE } else { 0 }
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use crate::Lang;
    use crate::tests::marked;

    fn has(lines: &[&str], text: &str, scope: &str) {
        let got = marked(Lang::GitConfig, lines);
        assert!(
            got.iter().any(|&(t, s)| t == text && s == scope),
            "{text:?} as {scope:?} in {got:#?}"
        );
    }

    fn lacks(lines: &[&str], text: &str, scope: &str) {
        let got = marked(Lang::GitConfig, lines);
        assert!(
            !got.iter().any(|&(t, s)| t == text && s == scope),
            "{text:?} not as {scope:?} in {got:#?}"
        );
    }

    const KEY: &str = "meta.mapping.key.git.config entity.name.tag.git.config";
    const SECTION: &str = "meta.brackets.git.config entity.name.section.git.config";
    const TEXT: &str = "string.unquoted.value.git.config";
    const QUOTED: &str = "string.quoted.double.git.config";
    const COMMENT: &str = "comment.line.git.config";
    const INVALID: &str = "invalid.illegal.git.config";

    #[test]
    fn sections_keys_and_comments() {
        let l = [
            "# a comment",
            "; another",
            "[user]",
            "    name = Ada Lovelace",
            "[remote \"origin\"]",
            "\turl = https://example.com/x.git",
            "[section.Sub]",
            "[a \"s\\\"x\\\\y\"]",
            "[init] defaultBranch = main",
            "[a] [b] k = v",
            "\tquotepath",
        ];
        has(&l, " a comment", COMMENT);
        has(&l, " another", COMMENT);
        has(&l, "user", SECTION);
        has(&l, "name", KEY);
        has(&l, "=", "keyword.operator.assignment.git.config");
        has(&l, "Ada Lovelace", TEXT);
        has(&l, "remote", SECTION);
        has(
            &l,
            "origin",
            "meta.brackets.git.config string.quoted.double.git.config",
        );
        has(
            &l,
            "https://example.com/x.git",
            "markup.underline.link.git.config",
        );
        has(
            &l,
            ".",
            "meta.brackets.git.config punctuation.accessor.dot.git.config",
        );
        has(
            &l,
            "Sub",
            "meta.brackets.git.config string.unquoted.git.config",
        );
        has(
            &l,
            "\\\"",
            "meta.brackets.git.config string.quoted.double.git.config constant.character.escape.git.config",
        );
        // a variable after a header on its line, as git reads it
        has(&l, "defaultBranch", KEY);
        has(&l, "main", TEXT);
        has(&l, "b", SECTION);
        has(&l, "k", KEY);
        // a key alone is a boolean's true
        has(&l, "quotepath", KEY);
    }

    #[test]
    fn values_as_git_reads_them() {
        let l = [
            "[a]",
            "k = x#y",
            "k = x;y",
            "k = \"x;y\" z",
            "k = \"a \\\"q\\\" \\\\ b\\t\" and more ; c",
            "k = \"#00ff00\"",
            "k = \\q",
        ];
        // `#` and `;` outside a quote start a comment wherever they stand
        has(&l, "x", TEXT);
        has(&l, "y", COMMENT);
        has(&l, "x;y", QUOTED);
        has(&l, " z", TEXT);
        has(&l, "\\\"", "constant.character.escape.git.config");
        has(&l, "\\t", "constant.character.escape.git.config");
        has(&l, " and more ", TEXT);
        has(&l, " c", COMMENT);
        // in a quote, a `#` is the value's
        has(&l, "#00ff00", QUOTED);
        // an escape git doesn't know is its error
        has(&l, "\\q", INVALID);
    }

    #[test]
    fn booleans_and_numbers_alone() {
        let l = [
            "[core]",
            "autocrlf = false",
            "bare = TRUE # c",
            "bigFileThreshold = 512m",
            "compression = -1",
            "x = true story",
            "y = 1.5",
        ];
        has(&l, "false", "constant.language.boolean.git.config");
        has(&l, "TRUE", "constant.language.boolean.git.config");
        has(&l, "512m", "constant.numeric.integer.decimal.git.config");
        has(&l, "-1", "constant.numeric.integer.decimal.git.config");
        has(&l, "true story", TEXT);
        has(&l, "1.5", TEXT);
    }

    #[test]
    fn errors() {
        let l = [
            "[a]",
            "k # c",
            "1k = v",
            "k_1 = v",
            "[ a]",
            "[a \"s\"x]",
            "[]",
        ];
        has(&l, "# c", INVALID);
        has(&l, "1k = v", INVALID);
        has(&l, "_1 = v", INVALID);
        has(&l, " a]", INVALID);
        has(&l, "x]", INVALID);
        has(&l, "]", INVALID);
    }

    #[test]
    fn continuations() {
        let l = [
            "[a]",
            "k = one \\",
            "  two # c",
            "q = \"in \\",
            "quote # still\"",
            "after = x",
        ];
        has(
            &l,
            "\\",
            "punctuation.separator.continuation.line.git.config",
        );
        has(&l, "  two ", TEXT);
        has(&l, " c", COMMENT);
        // a quote goes on across the line break
        has(&l, "quote # still", QUOTED);
        has(&l, "after", KEY);
    }

    #[test]
    fn colors() {
        let l = [
            "[color \"diff\"]",
            "meta = 11",
            "frag = magenta bold",
            "old = brightred nobold no-ul",
            "new = \"#00ff00\" reverse",
            "whitespace = red reset fuzzy",
            "[color]",
            "ui = auto",
        ];
        has(&l, "11", "constant.other.color.rgb-value.git.config");
        has(&l, "magenta", "support.constant.color.git.config");
        has(&l, "bold", "support.constant.color-attribute.git.config");
        has(&l, "brightred", "support.constant.color.git.config");
        has(&l, "nobold", "support.constant.color-attribute.git.config");
        has(&l, "no-ul", "support.constant.color-attribute.git.config");
        has(&l, "#00ff00", "constant.other.color.rgb-value.git.config");
        has(&l, "reset", "support.constant.color-reset.git.config");
        has(&l, "fuzzy", TEXT);
        // `[color]`'s own keys aren't colors
        has(&l, "auto", "constant.language.boolean.git.config");
    }

    #[test]
    fn commands_are_bash() {
        let l = [
            "[alias]",
            "co = checkout",
            "up = !git fetch && git rebase",
            "recent = \"!f() { git log --format=\\\"%h\\\"; }; f | head\"",
            "semi = !git fetch; git rebase",
            "[core]",
            "pager = ada -F --theme x",
            "editor = nvim",
            "[pager]",
            "branch = false",
            "[credential]",
            "helper = store",
            "[filter \"lfs\"]",
            "clean = git-lfs clean -- %f",
            "required = true",
            "[user]",
            "name = git log",
        ];
        let got = marked(Lang::GitConfig, &l);
        let shell = |t: &str| got.iter().any(|&(x, s)| x == t && s.ends_with(".shell"));
        assert!(shell("checkout"), "{got:#?}");
        has(&l, "!", "keyword.control.import.shell.git.config");
        assert!(shell("git"), "{got:#?}");
        assert!(shell("&&"), "{got:#?}");
        // the shell gets `"%h"` from `\"%h\"`: its string, over git's escapes
        has(&l, "\\\"", "punctuation.definition.string.begin.shell");
        has(&l, "%h", "string.quoted.double.shell");
        has(
            &l,
            "\"",
            "string.quoted.double.git.config punctuation.definition.string.begin.git.config",
        );
        // an unquoted `;` is git's comment, not the shell's
        has(&l, " git rebase", COMMENT);
        assert!(shell("ada"), "{got:#?}");
        has(&l, "F", "variable.parameter.option.shell");
        assert!(shell("nvim"), "{got:#?}");
        has(&l, "false", "constant.language.boolean.git.config");
        assert!(shell("store"), "{got:#?}");
        assert!(shell("git-lfs"), "{got:#?}");
        has(&l, "true", "constant.language.boolean.git.config");
        // other keys are text
        has(&l, "git log", TEXT);
    }

    #[test]
    fn a_command_continued_keeps_bash_state() {
        let l = [
            "[alias]",
            "lg = \"!git log \\",
            "    --graph \\",
            "    --oneline\"",
            "x = \"!f() { \\",
            "  echo 'a \\",
            "b'; }; f\"",
            "next = y",
        ];
        let got = marked(Lang::GitConfig, &l);
        // the next line goes on with the command: options, not a command's name
        assert!(
            got.iter()
                .any(|&(x, s)| x == "graph" && s.contains("option")),
            "{got:#?}"
        );
        // a shell string open across a line break
        assert!(
            got.iter()
                .any(|&(x, s)| x.contains("echo") && s.ends_with(".shell")),
            "{got:#?}"
        );
        assert!(
            got.iter()
                .any(|&(x, s)| x.starts_with("b") && s.contains("string.quoted.single.shell")),
            "{got:#?}"
        );
        has(&l, "next", KEY);
        lacks(&l, "y", "keyword.control.import.shell.git.config");
    }

    #[test]
    fn links_and_driver_placeholders() {
        let l = [
            "[remote \"origin\"]",
            "url = https://example.com/x.git",
            "fetch = +refs/heads/*:refs/remotes/origin/*",
            "[url \"git@github.com:\"]",
            "insteadOf = https://github.com/",
            "[filter \"lfs\"]",
            "clean = git-lfs clean -- %f",
            "[merge \"ours\"]",
            "driver = merge-it %O %A %B",
            "[alias]",
            "d = !date +%Y",
        ];
        has(
            &l,
            "https://example.com/x.git",
            "markup.underline.link.git.config",
        );
        has(&l, "+refs/heads/*:refs/remotes/origin/*", TEXT);
        has(
            &l,
            "https://github.com/",
            "markup.underline.link.git.config",
        );
        has(&l, "%f", "constant.other.placeholder.git.config");
        has(&l, "%O", "constant.other.placeholder.git.config");
        has(&l, "%B", "constant.other.placeholder.git.config");
        // only a driver's: an alias's `%` is the shell's text
        lacks(&l, "%Y", "constant.other.placeholder.git.config");
    }

    /// X-179: a header's `[` or a quote left open at a line's end is git's error, on the mark
    /// that opened it; a quote a `\` continues isn't open.
    #[test]
    fn what_a_line_leaves_open() {
        has(&["[core"], "[", INVALID);
        has(&["[core"], "core", SECTION);
        has(&["[remote \"origin\""], "[", INVALID);
        has(&["[remote \"origin"], "\"", INVALID);
        lacks(&["[remote \"origin"], "[", INVALID);
        has(&["[a]", "k = \"abc"], "\"", INVALID);
        has(&["[a]", "k = \"abc"], "abc", QUOTED);
        lacks(&["[a]", "k = \"a \\", "b\""], "\"", INVALID);
        has(&["[alias]", "lg = !echo \"x"], "\"", INVALID);
        lacks(&["[alias]", "lg = \"!echo x \\", "y\""], "\"", INVALID);
        lacks(&["[a]", "k = \"x\"", "[b]"], "[", INVALID);
    }

    /// X-179: a shell comment in a quoted command, continued: git joins the lines, so the
    /// comment swallows the rest of the value.
    #[test]
    fn a_continued_comment_goes_on() {
        let l = [
            "[alias]",
            "x = \"!f() { echo hi; # note \\",
            "    echo there; }; f\"",
            "next = y",
        ];
        has(&l, "    echo there; }; f", "comment.line.number-sign.shell");
        has(
            &l,
            "\"",
            "string.quoted.double.git.config punctuation.definition.string.end.git.config",
        );
        has(&l, "next", KEY);
    }

    /// X-179: an alias without a `!` is a git command line, which git splits itself: the
    /// shell's operators are its text; with a `!`, the shell's.
    #[test]
    fn an_alias_without_a_bang_is_gits_command_line() {
        let l = [
            "[alias]",
            "l = log --oneline | head",
            "q = log \"$x\" && y",
            "p = !git log | head",
        ];
        let got = marked(Lang::GitConfig, &l);
        let pipes = got
            .iter()
            .filter(|&&(t, s)| t == "|" && s.contains("pipe"))
            .count();
        assert_eq!(pipes, 1, "{got:#?}");
        // the git command's name a command's, as bash reads one
        assert!(
            got.iter()
                .any(|&(t, s)| t == "log" && s.starts_with("meta.function-call.shell")),
            "{got:#?}"
        );
        lacks(&l, "&&", "keyword.operator.logical.and.shell");
        lacks(&l, "$", "punctuation.definition.variable.shell");
    }

    /// X-179: the commands git runs through the shell that X-177 read as text.
    #[test]
    fn more_commands_are_bash() {
        let l = [
            "[gpg \"ssh\"]",
            "defaultKeyCommand = ssh-add -L",
            "[trailer \"sign\"]",
            "cmd = echo x",
            "[guitool \"lint\"]",
            "cmd = make lint",
            "[gc]",
            "recentObjectsHook = hook-it --all",
        ];
        let got = marked(Lang::GitConfig, &l);
        for word in ["ssh-add", "echo", "make", "hook-it"] {
            assert!(
                got.iter().any(|&(x, s)| x == word && s.ends_with(".shell")),
                "{word}: {got:#?}"
            );
        }
    }

    /// X-179: `color.blame.highlightRecent`, a color and a date in turn between commas.
    #[test]
    fn highlight_recent_colors_and_dates() {
        let l = [
            "[color \"blame\"]",
            "highlightRecent = blue,12 month ago,white bold,1 month ago,red",
        ];
        has(&l, "blue", "support.constant.color.git.config");
        has(&l, "white", "support.constant.color.git.config");
        has(&l, "bold", "support.constant.color-attribute.git.config");
        has(&l, "red", "support.constant.color.git.config");
        has(&l, "12 month ago", TEXT);
        has(&l, "1 month ago", TEXT);
        lacks(&l, "12", "constant.other.color.rgb-value.git.config");
    }

    #[test]
    fn long_lines_lex_in_linear_time() {
        for unit in [
            "[a \"\\\"]",
            "k = \"x\\t\" ",
            "[alias]\na = !f \"\\\\\" ",
            "[color \"x\"]\nk = red bold ",
            "[filter \"x\"]\nclean = %f%",
            "#;\\",
        ] {
            let lines: Vec<String> = unit
                .split('\n')
                .map(|l| {
                    if l.ends_with(' ') {
                        l.repeat(20_000)
                    } else {
                        l.to_string()
                    }
                })
                .collect();
            let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
            let start = std::time::Instant::now();
            marked(Lang::GitConfig, &lines);
            assert!(start.elapsed().as_secs() < 2, "{unit:?}");
        }
    }
}
