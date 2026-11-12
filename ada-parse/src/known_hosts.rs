//! OpenSSH's `known_hosts`, in place of bat's `Known Hosts`, on the line-format engine (X-045): a
//! line is a comment, or a host line (an annotation, the hosts, a key type, a key and its
//! label).
//!
//! A host line is bat's `#anon_main_0`, which pops before the line's `\n`. Its patterns can all
//! match only where a token starts: blanks between them stay the line's, and at any other byte
//! one pattern matches right there (the host run `[^,\[\s*?]+` takes what the others leave), so
//! the token is the first pattern in the grammar's order that matches at the scan's place. A
//! fingerprint ends the line's tokens: its context takes the rest as the label. A `[` opens a
//! bracketed host, whose contexts don't pop at a line's end: past an address or a `!`, the rest
//! to its `]` is the host's text (`#anon_hostname-or-ip-value_1`), a `,` in it too, as syntect
//! steps past a pop that would loop after a push that took nothing. That bracket is all a line
//! leaves open.
//!
//! Its readings are held by `tests/readings.rs`' snapshots (X-116).

use crate::emit::{Push, Runs, Sink};
use crate::line_format::{BLANK, Cursor, Format, blanks, leads};
use crate::ssh::{BASE64, KEY_TYPES, Listed, fingerprint, ipv4, ipv6, label, listed};
use crate::{Out, State};

const COMMENT_MARK: u16 = 0;
const COMMENT: u16 = 1;
const LINE: u16 = 2;
const ANNOTATION_MARK: u16 = 3;
const ANNOTATION: u16 = 4;
const SEPARATOR: u16 = 5;
const FINGERPRINT: u16 = 6;
const LABEL: u16 = 7;
const KEY_TYPE: u16 = 8;
const KEY_TYPE_DEPRECATED: u16 = 9;
const BRACKET_BEGIN: u16 = 10;
const BRACKET_END: u16 = 11;
const PORT_SEPARATOR: u16 = 12;
const PORT: u16 = 13;
const BRACKET_LOGICAL: u16 = 14;
const BRACKET_V6: u16 = 15;
const BRACKET_V4: u16 = 16;
const BRACKET_HOST: u16 = 17;
const BRACKET_WILDCARD: u16 = 18;
const V6: u16 = 19;
const V4: u16 = 20;
const HASHED_MARK: u16 = 21;
const HASHED_ALGORITHM: u16 = 22;
const HASHED_SALT: u16 = 23;
const HASHED_HASH: u16 = 24;
const LOGICAL: u16 = 25;
const WILDCARD: u16 = 26;
const HOST: u16 = 27;

/// Each kind's scopes under `text.known_hosts`, as bat's grammar gives them.
pub const KINDS: &[&str] = &[
    "comment.line.number-sign.ssh.common punctuation.definition.comment.ssh.common",
    "comment.line.number-sign.ssh.common",
    "meta.line.known-host.known_hosts",
    "meta.line.known-host.known_hosts meta.annotation.known_hosts variable.annotation.known_hosts \
     punctuation.definition.annotation.known_hosts",
    "meta.line.known-host.known_hosts meta.annotation.known_hosts variable.annotation.known_hosts",
    "meta.line.known-host.known_hosts punctuation.separator.sequence.known_hosts",
    "meta.line.known-host.known_hosts variable.other.fingerprint.ssh.common",
    "meta.line.known-host.known_hosts meta.annotation.identifier.ssh.common \
     string.unquoted.ssh.common",
    "meta.line.known-host.known_hosts support.type.key-type",
    "meta.line.known-host.known_hosts invalid.deprecated.key-type",
    "meta.line.known-host.known_hosts meta.brackets.host.known_hosts \
     punctuation.definition.string.begin.known_hosts",
    "meta.line.known-host.known_hosts meta.brackets.host.known_hosts \
     punctuation.definition.string.end.known_hosts",
    "meta.line.known-host.known_hosts meta.brackets.host.known_hosts \
     punctuation.separator.sequence.known_hosts",
    "meta.line.known-host.known_hosts meta.brackets.host.known_hosts \
     constant.numeric.integer.decimal.known_hosts",
    "meta.line.known-host.known_hosts meta.brackets.host.known_hosts \
     keyword.operator.logical.known_hosts",
    "meta.line.known-host.known_hosts meta.brackets.host.known_hosts \
     constant.numeric.ip-address.v6.ssh.common",
    "meta.line.known-host.known_hosts meta.brackets.host.known_hosts \
     constant.numeric.ip-address.v4.ssh.common",
    "meta.line.known-host.known_hosts meta.brackets.host.known_hosts meta.string.host.known_hosts \
     string.quoted.other.known_hosts",
    "meta.line.known-host.known_hosts meta.brackets.host.known_hosts meta.string.host.known_hosts \
     string.quoted.other.known_hosts keyword.operator.wildcard.known_hosts",
    "meta.line.known-host.known_hosts constant.numeric.ip-address.v6.ssh.common",
    "meta.line.known-host.known_hosts constant.numeric.ip-address.v4.ssh.common",
    "meta.line.known-host.known_hosts meta.string.host.obfuscated.known_hosts \
     punctuation.definition.known_hosts",
    "meta.line.known-host.known_hosts meta.string.host.obfuscated.known_hosts \
     constant.numeric.integer.algorithm.known_hosts",
    "meta.line.known-host.known_hosts meta.string.host.obfuscated.known_hosts \
     string.unquoted.salt.known_hosts",
    "meta.line.known-host.known_hosts meta.string.host.obfuscated.known_hosts \
     string.unquoted.hash.known_hosts",
    "meta.line.known-host.known_hosts keyword.operator.logical.known_hosts",
    "meta.line.known-host.known_hosts meta.string.host.known_hosts string.unquoted.known_hosts \
     keyword.operator.wildcard.known_hosts",
    "meta.line.known-host.known_hosts meta.string.host.known_hosts string.unquoted.known_hosts",
];

/// What a line leaves open: a bracketed host, its `]` not yet met.
const BRACKET: u64 = 1;

/// Lex `b`, a line without its `\n`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    crate::line_format::line::<KnownHosts, _>(b, state, out, &mut Push);
}

/// [`line`]'s lexemes written to `k`, a run sink (X-069).
/// Inline in a block's loop, [`crate::lex_runs`]' line too (X-050).
pub(crate) struct RunLine;

impl crate::LineScan for RunLine {
    #[inline(always)]
    fn scan(b: &[u8], state: &mut State, out: &mut Out, k: &mut Runs) {
        crate::line_format::line::<KnownHosts, _>(b, state, out, k);
    }
}

struct KnownHosts;

impl Format for KnownHosts {
    // `^\s*(#)`, the blanks before the mark plain
    const LEAD: [u8; 256] = leads(b"#", b"");
    const COMMENT_TEXT: u16 = COMMENT;
    const COMMENT_MARK: u16 = COMMENT_MARK;
    const COMMENT_BLANKS: bool = false;

    /// A line inside a bracketed host: its text to the `]`, then the host line goes on.
    fn resume<K: Sink>(c: &mut Cursor<K>, _: u64) -> u64 {
        match bracket_text(c, 0) {
            Some(p) => tokens(c, p),
            None => BRACKET,
        }
    }

    /// A line of blanks: `^((@)…)?` matches nothing and pushes the host line, which takes them.
    #[inline(always)]
    fn blank<K: Sink>(c: &mut Cursor<K>) {
        c.put(c.len(), LINE);
    }

    #[inline(always)]
    fn entry<K: Sink>(c: &mut Cursor<K>, _: usize) -> u64 {
        let b = c.b;
        // `^((@)(?:revoked|cert-authority))?` at the line's start alone
        let annotation = [&b"@revoked"[..], b"@cert-authority"]
            .into_iter()
            .find(|a| b.starts_with(a));
        if let Some(a) = annotation {
            c.put(1, ANNOTATION_MARK);
            c.put(a.len(), ANNOTATION);
        }
        tokens(c, c.at)
    }
}

/// The host line's tokens from `p` to the line's end: what the line leaves open.
fn tokens<K: Sink>(c: &mut Cursor<K>, mut p: usize) -> u64 {
    let b = c.b;
    let n = b.len();
    loop {
        p = blanks(b, p);
        if p == n {
            line_to(c, n);
            return 0;
        }
        let x = b[p];
        let (e, kind) = if x == b',' {
            (p + 1, SEPARATOR)
        } else if let Some(e) = fingerprint(b, p) {
            line_to(c, p);
            c.put(e, FINGERPRINT);
            // `#anon_ssh-fingerprint-with-label_0`: the rest of the line
            if let Some((s, t)) = label(b, e) {
                line_to(c, s);
                c.put(t, LABEL);
            }
            line_to(c, n);
            return 0;
        } else if let Some((e, l)) = listed(b, p, &KEY_TYPES) {
            let kind = match l {
                Listed::Supported => KEY_TYPE,
                Listed::Deprecated => KEY_TYPE_DEPRECATED,
            };
            (e, kind)
        } else if x == b'[' {
            line_to(c, p);
            c.put(p + 1, BRACKET_BEGIN);
            match bracket(c, p + 1) {
                Some(e) => {
                    p = e;
                    continue;
                }
                None => return BRACKET,
            }
        } else if let Some(e) = ipv6(b, p) {
            (e, V6)
        } else if let Some(e) = ipv4(b, p) {
            (e, V4)
        } else if x == b'|'
            && let Some(e) = hashed(c, p)
        {
            p = e;
            continue;
        } else if x == b'!' {
            (p + 1, LOGICAL)
        } else if matches!(x, b'*' | b'?') {
            (p + 1, WILDCARD)
        } else {
            // `[^,\[\s*?]+`
            let mut e = p + 1;
            while e < n && !(BLANK[usize::from(b[e])] || matches!(b[e], b',' | b'[' | b'*' | b'?'))
            {
                e += 1;
            }
            (e, HOST)
        };
        line_to(c, p);
        c.put(e, kind);
        p = e;
    }
}

/// The host line's own bytes (blanks, a label's) up to `to`.
#[inline(always)]
fn line_to<K: Sink>(c: &mut Cursor<K>, to: usize) {
    if to > c.at {
        c.put_merged(to, LINE);
    }
}

/// `(\|)(\d+)(\|)([\w+/]{27}=)(\|)([\w+/]{27}=)` at `p`, a hashed host: where it ends.
fn hashed<K: Sink>(c: &mut Cursor<K>, p: usize) -> Option<usize> {
    let b = c.b;
    let mut d = p + 1;
    while d < b.len() && b[d].is_ascii_digit() {
        d += 1;
    }
    let part = |s: usize| {
        (b.get(s..s + 27)?.iter().all(|&x| BASE64[usize::from(x)]) && b.get(s + 27) == Some(&b'='))
            .then_some(s + 28)
    };
    if d == p + 1 || b.get(d) != Some(&b'|') {
        return None;
    }
    let salt = part(d + 1)?;
    if b.get(salt) != Some(&b'|') {
        return None;
    }
    let hash = part(salt + 1)?;
    line_to(c, p);
    c.put(p + 1, HASHED_MARK);
    c.put(d, HASHED_ALGORITHM);
    c.put(d + 1, HASHED_MARK);
    c.put(salt, HASHED_SALT);
    c.put(salt + 1, HASHED_MARK);
    c.put(hash, HASHED_HASH);
    Some(hash)
}

/// `#anon_hostname-or-ip-value_0` from `p`, past a `[`: `!`s and addresses, then the host's
/// text to the `]`. Where the host line goes on past the `]` and its port, or `None` where the
/// line ends first.
fn bracket<K: Sink>(c: &mut Cursor<K>, mut p: usize) -> Option<usize> {
    let b = c.b;
    loop {
        if p == b.len() {
            return None;
        }
        let (e, kind) = if b[p] == b']' {
            return Some(close(c, p));
        } else if b[p] == b'!' {
            (p + 1, BRACKET_LOGICAL)
        } else if let Some(e) = ipv6(b, p) {
            (e, BRACKET_V6)
        } else if let Some(e) = ipv4(b, p) {
            (e, BRACKET_V4)
        } else {
            return bracket_text(c, p);
        };
        c.put(e, kind);
        p = e;
    }
}

/// `#anon_hostname-or-ip-value_1` from `p`: the host's text and wildcards to the `]`, which
/// closes the bracket. Where the host line goes on, or `None` where the line ends first.
fn bracket_text<K: Sink>(c: &mut Cursor<K>, mut p: usize) -> Option<usize> {
    let b = c.b;
    loop {
        let e = crate::line_format::find(b, p, *b"]*?");
        if e > p {
            c.put_merged(e, BRACKET_HOST);
        }
        if e == b.len() {
            return None;
        }
        if b[e] == b']' {
            return Some(close(c, e));
        }
        c.put(e + 1, BRACKET_WILDCARD);
        p = e + 1;
    }
}

/// `(\])(?:(:)(\d{1,5}))?` at `p`: where the host line goes on.
fn close<K: Sink>(c: &mut Cursor<K>, p: usize) -> usize {
    let b = c.b;
    c.put(p + 1, BRACKET_END);
    if b.get(p + 1) == Some(&b':') && b.get(p + 2).is_some_and(u8::is_ascii_digit) {
        let mut e = p + 3;
        while e < b.len() && e < p + 7 && b[e].is_ascii_digit() {
            e += 1;
        }
        c.put(p + 2, PORT_SEPARATOR);
        c.put(e, PORT);
        return e;
    }
    p + 1
}
