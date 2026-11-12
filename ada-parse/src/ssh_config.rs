//! OpenSSH's client and server configs, in place of bat's `SSH Config` and `SSHD Config`, on the
//! line-format engine (X-060): one scan for both, the dialect a run-time flag (one copy in ada:
//! a const parameter's two were +55 KB for 2–6% in ada, X-060's r1f against r1g). The design is
//! X-060's floor probe grown, not bat's 45 and 25 contexts: a line is blank, a comment, or a key
//! and its value; the key is read once and looked up once, folded, for what its value is (a
//! `Host` line's patterns, a `Match` line's conditions, a host name, a boolean, a shell command,
//! or a generic value); a value is read by one pass over its bytes. Nothing is carried between
//! lines (bat's shell contexts, which run on past a line's end where a quote is left open, are a
//! derailment not followed: a value ends with its line).
//!
//! **A generic value** is bat's: its first tokens (a boolean, a `%` escape, a number, a quoted
//! string) in the value's own context, then from the first other byte a string to the line's
//! end, whose tokens are found where bat finds them, leftmost first: a list's word (a key type, a
//! cipher, a key exchange, a MAC), a time, a size, an address only where a word starts (`\b`), an
//! IPv6 address anywhere (its pattern has no `\b`), punctuation at its byte. **Departure:** bat's
//! client config takes the string's first byte as a token of its own (`\S`), so the value's first
//! word never starts a word and a list's first entry (`Ciphers aes128-ctr,…`) or a first time
//! (`ServerAliveInterval 60s`) or address (`BindAddress 127.0.0.1`) stays plain; here the string
//! starts at that byte, as bat's server config has it. With it a client's `none` is a set's word,
//! as a server's is (`ProxyJump none`), not a deprecated cipher's.
//!
//! **The shell** (`ProxyCommand`, `LocalCommand`, `ForceCommand`, `Match exec`) is bash's scanner
//! on the value's text ([`crate::bash::line_spans`]), its spans laid back with the host's `%`
//! escapes over them (bat's `with_prototype`), from a fresh state each line.
//!
//! **Kinds are leaves**: no meta scope colors anything in the 14 built-in themes (X-060's
//! census), and a token inside a string takes its own leaf's color there, so a token's kind is
//! its leaf alone. The shell's kinds are bash's ([`crate::bash::scopes`]), below [`SSH`]; the
//! formats' own are from it.

use crate::emit::{Push, Runs, Sink};
use crate::line_format::{BLANK, Cursor, Format, leads};
use crate::prim::WORD;
use crate::ssh::{HEX, ipv4, ipv6};
use crate::words::{Words, text_len};
use crate::{Out, Span, State};

/// The formats' kinds are this plus their index in [`KINDS`]; bash's are below it, and bash's
/// twins ([`crate::bash::TWIN`]) above the formats'.
pub const SSH: u16 = 0x100;

macro_rules! kinds {
    ($($k:ident = $ssh:literal, $sshd:literal,)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms, dead_code)]
        #[derive(Clone, Copy)]
        #[repr(u16)]
        enum KindId { $($k,)* Count }
        /// The kinds, each a stack of scopes under `source.ssh_config` or `source.sshd_config`.
        #[allow(dead_code)]
        mod k {
            $(pub const $k: u16 = super::SSH + super::KindId::$k as u16;)*
        }
        /// Each kind's scopes in the client's config and the server's.
        static KINDS: [(&str, &str); KindId::Count as usize] = [$(($ssh, $sshd),)*];
    };
}

kinds! {
    COMMENT_MARK = "comment.line.number-sign.ssh.common punctuation.definition.comment.ssh.common",
        "comment.line.number-sign.sshd_config punctuation.definition.comment.sshd_config",
    COMMENT = "comment.line.number-sign.ssh.common", "comment.line.number-sign.sshd_config",
    KEY = "keyword.other.ssh_config", "keyword.other.sshd_config",
    HOST = "keyword.declaration.host-alias.ssh_config", "keyword.declaration.host-alias.ssh_config",
    HOSTNAME_KEY = "keyword.declaration.host.ssh_config", "keyword.declaration.host.ssh_config",
    MATCH = "keyword.control.conditional.ssh_config", "keyword.control.conditional.sshd_config",
    ASSIGN = "keyword.operator.assignment.ssh_config", "keyword.operator.assignment.sshd_config",
    LABEL = "entity.name.label.host-alias.ssh_config", "entity.name.label.host-alias.ssh_config",
    LABEL_WILDCARD = "entity.name.label.host-alias.ssh_config keyword.operator.wildcard.ssh_config",
        "entity.name.label.host-alias.ssh_config keyword.operator.wildcard.ssh_config",
    STRING = "string.unquoted.ssh_config", "string.unquoted.sshd_config",
    QUOTED = "string.quoted.double.ssh_config", "string.quoted.double.sshd_config",
    QUOTE_BEGIN = "string.quoted.double.ssh_config punctuation.definition.string.begin.ssh_config",
        "string.quoted.double.sshd_config punctuation.definition.string.begin.sshd_config",
    QUOTE_END = "string.quoted.double.ssh_config punctuation.definition.string.end.ssh_config",
        "string.quoted.double.sshd_config punctuation.definition.string.end.sshd_config",
    HOSTNAME = "string.unquoted.hostname.ssh_config", "string.unquoted.hostname.ssh_config",
    HOST_V4 = "constant.numeric.ip-address.v4.ssh_config", "constant.numeric.ip-address.v4.ssh_config",
    HOST_V6 = "constant.numeric.ip-address.v6.ssh_config", "constant.numeric.ip-address.v6.ssh_config",
    INVALID = "invalid.illegal.ssh_config", "invalid.illegal.sshd_config",
    BOOLEAN = "constant.language.boolean.ssh_config", "constant.language.boolean.sshd_config",
    SET = "constant.language.set.ssh_config", "constant.language.set.sshd_config",
    DEFAULT = "constant.language.default.ssh_config", "constant.language.default.sshd_config",
    NUMBER = "constant.numeric.ssh_config", "constant.numeric.sshd_config",
    DECIMAL = "constant.numeric.integer.decimal.ssh_config",
        "constant.numeric.integer.decimal.sshd_config",
    ESCAPE = "constant.character.escape.ssh_config", "constant.character.escape.sshd_config",
    WILDCARD = "keyword.operator.wildcard.ssh_config", "keyword.operator.wildcard.sshd_config",
    LOGICAL = "keyword.operator.logical.ssh_config", "keyword.operator.logical.sshd_config",
    SEPARATOR = "punctuation.separator.sequence.ssh_config",
        "punctuation.separator.sequence.sshd_config",
    KEY_TYPE = "support.type.key-type", "support.type.key-type",
    KEY_TYPE_DEPRECATED = "invalid.deprecated.key-type", "invalid.deprecated.key-type",
    CIPHER = "support.function.cipher", "support.function.cipher",
    CIPHER_DEPRECATED = "invalid.deprecated.cipher", "invalid.deprecated.cipher",
    KEX = "support.function.kex-algorithm", "support.function.kex-algorithm",
    KEX_DEPRECATED = "invalid.deprecated.kex-algorithm", "invalid.deprecated.kex-algorithm",
    MAC = "support.function.mac-algorithm", "support.function.mac-algorithm",
    MAC_DEPRECATED = "invalid.deprecated.mac-algorithm", "invalid.deprecated.mac-algorithm",
    V4 = "constant.numeric.ip-address.v4.ssh.common", "constant.numeric.ip-address.v4.ssh.common",
    V6 = "constant.numeric.ip-address.v6.ssh.common", "constant.numeric.ip-address.v6.ssh.common",
    V6_BEGIN = "constant.numeric.ip-address.v6.ssh.common punctuation.definition.constant.begin.ssh.common",
        "constant.numeric.ip-address.v6.ssh.common punctuation.definition.constant.begin.ssh.common",
    V6_END = "constant.numeric.ip-address.v6.ssh.common punctuation.definition.constant.end.ssh.common",
        "constant.numeric.ip-address.v6.ssh.common punctuation.definition.constant.end.ssh.common",
    CIDR = "punctuation.separator.sequence.ssh.common", "punctuation.separator.sequence.ssh.common",
    RANGE = "constant.other.range.ssh.common", "constant.other.range.ssh.common",
    AMOUNT = "constant.numeric.integer.decimal.ssh.common",
        "constant.numeric.integer.decimal.ssh.common",
    UNIT = "storage.modifier.unit.ssh.common", "storage.modifier.unit.ssh.common",
}

/// The scopes under the format's base a kind carries, outermost first: the formats' own, or
/// bash's for the shell's.
pub fn scopes(kind: u16, sshd: bool) -> Vec<&'static str> {
    match kind.checked_sub(SSH).map(usize::from) {
        Some(i) if i < KINDS.len() => {
            let (ssh, d) = KINDS[i];
            if sshd { d } else { ssh }.split(' ').collect()
        }
        _ => crate::bash::scopes(kind),
    }
}

/// Lex `b`, a line of a client's config without its `\n`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    crate::line_format::line::<Config<false>, _>(b, state, out, &mut Push);
}

/// [`line()`]'s lexemes written to `k`, a run sink.
/// Inline in a block's loop, [`crate::lex_runs`]' line too (X-050).
pub(crate) struct RunLine;

impl crate::LineScan for RunLine {
    #[inline(always)]
    fn scan(b: &[u8], state: &mut State, out: &mut Out, k: &mut Runs) {
        crate::line_format::line::<Config<false>, _>(b, state, out, k);
    }
}

/// Lex `b`, a line of a server's config without its `\n`.
#[inline(never)]
pub fn sshd_line(b: &[u8], state: &mut State, out: &mut Out) {
    crate::line_format::line::<Config<true>, _>(b, state, out, &mut Push);
}

/// [`sshd_line`]'s lexemes written to `k`, a run sink.
/// Inline in a block's loop, [`crate::lex_runs`]' line too (X-050).
pub(crate) struct SshdRunLine;

impl crate::LineScan for SshdRunLine {
    #[inline(always)]
    fn scan(b: &[u8], state: &mut State, out: &mut Out, k: &mut Runs) {
        crate::line_format::line::<Config<true>, _>(b, state, out, k);
    }
}

/// A config, the server's where `SSHD`: the flag [`entry`] reads, its scan one copy for both.
struct Config<const SSHD: bool>;

impl<const SSHD: bool> Format for Config<SSHD> {
    // `^\s*(#)`, `^\s*(;)`: the blanks before the mark plain
    const LEAD: [u8; 256] = leads(b"#;", b"");
    const COMMENT_TEXT: u16 = k::COMMENT;
    const COMMENT_MARK: u16 = k::COMMENT_MARK;
    const COMMENT_BLANKS: bool = false;
    const OPENS: bool = false;

    fn resume<K: Sink>(_: &mut Cursor<K>, _: u64) -> u64 {
        0
    }

    #[inline(always)]
    fn entry<K: Sink>(c: &mut Cursor<K>, p: usize) -> u64 {
        entry(c, p, SSHD);
        0
    }
}

// what a key's value is
const GENERIC: u8 = 0;
const HOST_LINE: u8 = 1;
const MATCH_LINE: u8 = 2;
const HOSTNAME_VALUE: u8 = 3;
const PROXY: u8 = 4;
const LOCAL: u8 = 5;
const FORCE: u8 = 6;
const BOOL: u8 = 7;
const BOOL_PLUS: u8 = 8;
/// A word that isn't a key: the line is plain.
const NOT_KEY: u8 = 9;

/// A key: what its value is, `fallback` for a key not listed.
#[inline(always)]
fn action(w: &[u8], fallback: u8, sshd: bool) -> u8 {
    let a = if sshd {
        SSHD_KEYS.value(w, &SSHD_ACTIONS)
    } else {
        SSH_KEYS.value(w, &SSH_ACTIONS)
    };
    if a == NONE { fallback } else { a }
}

/// Not a listed key ([`action`]'s table at 0).
const NONE: u8 = u8::MAX;

/// A client's keys whose values aren't generic: `options`' own rules, `host`, `match`.
const SSH_KEY_ENTRIES: [(&[u8], u8); 40] = [
    (b"host", HOST_LINE),
    (b"match", MATCH_LINE),
    (b"hostname", HOSTNAME_VALUE),
    (b"proxycommand", PROXY),
    (b"localcommand", LOCAL),
    (b"stricthostkeychecking", BOOL_PLUS),
    (b"verifyhostkeydns", BOOL_PLUS),
    (b"pubkeyauthentication", BOOL),
    (b"hostbasedauthentication", BOOL),
    (b"passwordauthentication", BOOL),
    (b"challengeresponseauthentication", BOOL),
    (b"kbdinteractiveauthentication", BOOL),
    (b"rsaauthentication", BOOL),
    (b"rhostsrsaauthentication", BOOL),
    (b"forwardagent", BOOL),
    (b"forwardx11", BOOL),
    (b"forwardx11trusted", BOOL),
    (b"clearallforwardings", BOOL),
    (b"exitonforwardfailure", BOOL),
    (b"batchmode", BOOL),
    (b"canonicalizefallbacklocal", BOOL),
    (b"checkhostip", BOOL),
    (b"compression", BOOL),
    (b"enablesshkeysign", BOOL),
    (b"gatewayports", BOOL),
    (b"hashknownhosts", BOOL),
    (b"identitiesonly", BOOL),
    (b"nohostauthenticationforlocalhost", BOOL),
    (b"permitlocalcommand", BOOL),
    (b"proxyusefdpass", BOOL),
    (b"streamlocalbindunlink", BOOL),
    (b"tcpkeepalive", BOOL),
    (b"usekeychain", BOOL),
    (b"useprivilegedport", BOOL),
    (b"visualhostkey", BOOL),
    (b"gssapiauthentication", BOOL),
    (b"gssapikeyexchange", BOOL),
    (b"gssapidelegatecredentials", BOOL),
    (b"gssapirenewalforcesrekey", BOOL),
    (b"gssapitrustdns", BOOL),
];

/// A server's: `match`, and `forcecommand` (bat's only in a `Match` block, everywhere here).
const SSHD_KEY_ENTRIES: [(&[u8], u8); 2] = [(b"match", MATCH_LINE), (b"forcecommand", FORCE)];

/// An entry list's words, and what each is by id (none at 0).
const fn words_of<const N: usize>(e: &[(&'static [u8], u8); N]) -> [&'static [u8]; N] {
    let mut l: [&[u8]; N] = [b""; N];
    let mut i = 0;
    while i < N {
        l[i] = e[i].0;
        i += 1;
    }
    l
}

const fn values_of<const N: usize, const M: usize>(
    e: &[(&'static [u8], u8); N],
    none: u8,
) -> [u8; M] {
    let mut t = [none; M];
    let mut i = 0;
    while i < N {
        t[i + 1] = e[i].1;
        i += 1;
    }
    t
}

const SSH_KEY_LIST: [&[u8]; 40] = words_of(&SSH_KEY_ENTRIES);
static SSH_KEYS: Words<128, 40, { text_len(&SSH_KEY_LIST) }, true> = Words::new(&SSH_KEY_LIST);
static SSH_ACTIONS: [u8; 41] = values_of(&SSH_KEY_ENTRIES, NONE);
const SSHD_KEY_LIST: [&[u8]; 2] = words_of(&SSHD_KEY_ENTRIES);
static SSHD_KEYS: Words<8, 2, { text_len(&SSHD_KEY_LIST) }, true> = Words::new(&SSHD_KEY_LIST);
static SSHD_ACTIONS: [u8; 3] = values_of(&SSHD_KEY_ENTRIES, NONE);

/// `[A-Za-z1]`, a generic key's byte.
static KEY_BYTE: [bool; 256] = {
    let mut t = [false; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = (i as u8).is_ascii_alphabetic();
        i += 1;
    }
    t[b'1' as usize] = true;
    t
};

/// `[ \t]`.
#[inline(always)]
fn space_tab(x: u8) -> bool {
    x == b' ' || x == b'\t'
}

/// Where the run of `[ \t]` from `p` ends.
#[inline(always)]
fn spaces(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && space_tab(b[p]) {
        p += 1;
    }
    p
}

/// Where the run of `\w` from `p` ends.
#[inline(always)]
fn word_end(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && WORD[usize::from(b[p])] {
        p += 1;
    }
    p
}

/// Whether a word starts at `p`: `\b` before a word byte.
#[inline(always)]
fn word_start(b: &[u8], p: usize) -> bool {
    WORD[usize::from(b[p])] && (p == 0 || !WORD[usize::from(b[p - 1])])
}

/// A line whose first byte past its blanks, at `p`, isn't a comment's mark.
#[inline(never)]
fn entry<K: Sink>(c: &mut Cursor<K>, p: usize, sshd: bool) {
    let b = c.b;
    let n = b.len();
    let e = word_end(b, p);
    if e == p {
        return;
    }
    let generic = b[p..e].iter().all(|&x| KEY_BYTE[usize::from(x)]);
    let fallback = if generic { GENERIC } else { NOT_KEY };
    let mut a = action(&b[p..e], fallback, sshd);
    // `^[ \t]*(host|match)\b`: their lines want the blanks before them spaces and tabs
    if matches!(a, HOST_LINE | MATCH_LINE) && !b[..p].iter().all(|&x| space_tab(x)) {
        a = fallback;
    }
    let kind = match a {
        NOT_KEY => return,
        HOST_LINE => k::HOST,
        MATCH_LINE => k::MATCH,
        HOSTNAME_VALUE => k::HOSTNAME_KEY,
        _ => k::KEY,
    };
    c.tok(None, p, e, kind);
    let mut q = e;
    match a {
        HOST_LINE => return hosts(c, q),
        MATCH_LINE if sshd => return sshd_conditions(c, q),
        MATCH_LINE => return conditions(c, q),
        _ => {}
    }
    // `[ \t]*(=)?`
    q = spaces(b, q);
    if q < n && b[q] == b'=' {
        c.tok(None, q, q + 1, k::ASSIGN);
        q += 1;
    }
    match a {
        HOSTNAME_VALUE => hostname(c, q),
        PROXY => shell(c, q, n, ESC_PROXY, true),
        LOCAL => shell(c, q, n, ESC_LOCAL, true),
        FORCE => shell(c, q, n, 0, true),
        BOOL => boolean(c, q, false),
        BOOL_PLUS => boolean(c, q, true),
        _ => generic_value(c, q, sshd),
    }
}

// --- the `%` escapes each place takes, a bit a byte -------------------------------------------

const fn mask(s: &[u8]) -> u128 {
    let mut m = 0u128;
    let mut i = 0;
    while i < s.len() {
        m |= 1 << s[i];
        i += 1;
    }
    m
}

/// A client's value (`tokens`).
const ESC_SSH: u128 = mask(b"%CdhiLlnprTu");
/// A server's value.
const ESC_SSHD: u128 = mask(b"%DFfhiKksTtUu");
const ESC_PROXY: u128 = mask(b"%hpr");
const ESC_LOCAL: u128 = mask(b"%CdhilnprTu");
/// `Match exec` without quotes.
const ESC_EXEC: u128 = mask(b"%hiLlnpru");

/// Whether an escape of `set` is at `p`.
#[inline(always)]
fn escape(b: &[u8], p: usize, set: u128) -> bool {
    b[p] == b'%' && p + 1 < b.len() && b[p + 1] < 128 && set >> b[p + 1] & 1 != 0
}

/// Whether a list's word, a number or a size may end before `e`: `(?=[\s,"])`, the line's end
/// its `\n`.
#[inline(always)]
fn ends_value(b: &[u8], e: usize) -> bool {
    e == b.len() || BLANK[usize::from(b[e])] || b[e] == b',' || b[e] == b'"'
}

/// `\d+` from `p`: where it ends.
#[inline(always)]
fn digits(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && b[p].is_ascii_digit() {
        p += 1;
    }
    p
}

// --- a `Host` line -----------------------------------------------------------------------------

/// `host-patterns` from `q`: `!`, a label (`[^\s*?,]+`), a wildcard in one; the rest plain.
#[inline(always)]
fn hosts<K: Sink>(c: &mut Cursor<K>, mut q: usize) {
    let b = c.b;
    let n = b.len();
    while q < n {
        let x = b[q];
        match x {
            b'!' => c.tok(None, q, q + 1, k::LOGICAL),
            b'*' | b'?' => c.tok(None, q, q + 1, k::LABEL_WILDCARD),
            b',' => {}
            _ if BLANK[usize::from(x)] => {}
            _ => {
                let mut e = q + 1;
                while e < n && !(BLANK[usize::from(b[e])] || matches!(b[e], b'*' | b'?' | b',')) {
                    e += 1;
                }
                c.tok(None, q, e, k::LABEL);
                q = e;
                continue;
            }
        }
        q += 1;
    }
}

// --- `HostName` ---------------------------------------------------------------------------------

/// `hostname-or-ip-value` from `q`, as often as it matches: an address or a name, the blanks
/// after it, and the next word invalid.
#[inline(always)]
fn hostname<K: Sink>(c: &mut Cursor<K>, mut q: usize) {
    let b = c.b;
    let n = b.len();
    loop {
        while q < n && BLANK[usize::from(b[q])] {
            q += 1;
        }
        if q == n {
            return;
        }
        let (e, kind) = match simple_v4(b, q) {
            Some(e) => (e, k::HOST_V4),
            None => match simple_v6(b, q) {
                Some(e) => (e, k::HOST_V6),
                None => {
                    let mut e = q;
                    while e < n && !BLANK[usize::from(b[e])] {
                        e += 1;
                    }
                    (e, k::HOSTNAME)
                }
            },
        };
        c.tok(None, q, e, kind);
        // `[ \t]*(\S*)`
        let s = spaces(b, e);
        let mut t = s;
        while t < n && !BLANK[usize::from(b[t])] {
            t += 1;
        }
        c.tok(None, s, t, k::INVALID);
        q = t;
    }
}

/// `(?:\d{1,3}\.){3}\d{1,3}` at `p`: each group's digits are its whole run, the last's up to
/// three.
fn simple_v4(b: &[u8], mut p: usize) -> Option<usize> {
    for _ in 0..3 {
        let e = digits(b, p);
        if !(1..=3).contains(&(e - p)) || b.get(e) != Some(&b'.') {
            return None;
        }
        p = e + 1;
    }
    let e = digits(b, p);
    (e > p).then(|| e.min(p + 3))
}

/// `(?i:[a-f0-9:]+:+)+[a-f0-9]+` at `p`: within the run of hex digits and colons, the hex digits
/// after its last colon that one follows (not the run's first byte).
fn simple_v6(b: &[u8], p: usize) -> Option<usize> {
    let mut r = p;
    while r < b.len() && (HEX[usize::from(b[r])] || b[r] == b':') {
        r += 1;
    }
    let i = (p + 1..r.saturating_sub(1))
        .rev()
        .find(|&i| b[i] == b':' && HEX[usize::from(b[i + 1])])?;
    let mut e = i + 1;
    while e < r && HEX[usize::from(b[e])] {
        e += 1;
    }
    Some(e)
}

// --- booleans -------------------------------------------------------------------------------------

/// `boolean-option`'s value from `q`: `yes` or `no`, else `y`, `ye`, `n` plain (and with `plus`,
/// `ask`, else `a`, `ak`, `ask` plain), else the word invalid; past the first, every word with
/// the blanks before it invalid.
#[inline(always)]
fn boolean<K: Sink>(c: &mut Cursor<K>, mut q: usize, plus: bool) {
    let b = c.b;
    let n = b.len();
    while q < n {
        let x = b[q];
        if space_tab(x) {
            let s = spaces(b, q);
            let mut t = s;
            while t < n && !BLANK[usize::from(b[t])] {
                t += 1;
            }
            if t > s {
                c.tok(None, q, t, k::INVALID);
            }
            q = t.max(q + 1);
            continue;
        }
        if BLANK[usize::from(x)] {
            q += 1;
            continue;
        }
        let e = word_end(b, q);
        if word_start(b, q) {
            let w = &b[q..e];
            let lower = |s: &[u8]| w.eq_ignore_ascii_case(s);
            if lower(b"yes") || lower(b"no") {
                c.tok(None, q, e, k::BOOLEAN);
                q = e;
                continue;
            }
            if lower(b"y") || lower(b"ye") || lower(b"n") {
                c.skip(e);
                q = e;
                continue;
            }
            if plus && lower(b"ask") {
                c.tok(None, q, e, k::SET);
                q = e;
                continue;
            }
            // `\b(?i:a(?:s?k)?)\b`
            if plus && (lower(b"a") || lower(b"ak")) {
                c.skip(e);
                q = e;
                continue;
            }
        }
        let mut t = q;
        while t < n && !BLANK[usize::from(b[t])] {
            t += 1;
        }
        c.tok(None, q, t, k::INVALID);
        q = t;
    }
}

// --- a generic value --------------------------------------------------------------------------

/// A generic option's value from `q`: its first tokens in the value's context (a client's: a
/// boolean, an escape, a number, a quoted string; a server's: an escape, a quoted string), then a
/// string to the line's end.
#[inline(always)]
fn generic_value<K: Sink>(c: &mut Cursor<K>, mut q: usize, sshd: bool) {
    let b = c.b;
    let n = b.len();
    while q < n {
        let x = b[q];
        if BLANK[usize::from(x)] {
            q += 1;
            continue;
        }
        if x == b'"' {
            q = quoted(c, q, sshd);
            continue;
        }
        if escape(b, q, if sshd { ESC_SSHD } else { ESC_SSH }) {
            c.tok(None, q, q + 2, k::ESCAPE);
            q += 2;
            continue;
        }
        if !sshd && word_start(b, q) {
            let e = word_end(b, q);
            let w = &b[q..e];
            if w.eq_ignore_ascii_case(b"yes") || w.eq_ignore_ascii_case(b"no") {
                c.tok(None, q, e, k::BOOLEAN);
                q = e;
                continue;
            }
            let d = digits(b, q);
            if d > q && ends_value(b, d) {
                c.tok(None, q, d, k::NUMBER);
                q = d;
                continue;
            }
        }
        // bat's client takes this byte alone as the string's (`\S`), the server looks ahead
        // (`(?=\S)`): both start the string here (the module's departure)
        tokens(c, q, n, k::STRING, sshd);
        return;
    }
}

/// A quoted string whose `"` is at `q`: its tokens to the next `"` or the line's end. Where it
/// ends.
#[inline(never)]
fn quoted<K: Sink>(c: &mut Cursor<K>, q: usize, sshd: bool) -> usize {
    let b = c.b;
    let n = b.len();
    c.tok(None, q, q + 1, k::QUOTE_BEGIN);
    let end = b[q + 1..]
        .iter()
        .position(|&x| x == b'"')
        .map_or(n, |i| q + 1 + i);
    tokens(c, q + 1, end, k::QUOTED, sshd);
    if end < n {
        c.tok(None, end, end + 1, k::QUOTE_END);
        end + 1
    } else {
        n
    }
}

/// A string's text from `q` to `end` as `text`, its tokens where bat's patterns find them, the
/// leftmost first and at one place the first in bat's order (`generic-option-includes`,
/// `generic-option-value`).
#[inline(always)]
fn tokens<K: Sink>(c: &mut Cursor<K>, mut q: usize, end: usize, text: u16, sshd: bool) {
    let b = c.b;
    let set = if sshd { ESC_SSHD } else { ESC_SSH };
    // the blanks before the string are the value's, plain
    c.text(None, q);
    // the run a list's word would end with (to a `,`, a blank or a `"`), read once a run, and
    // where its last byte no list word has is past
    let (mut run, mut clean) = (q, q);
    while q < end {
        let x = b[q];
        let (e, kind) = match x {
            b'%' if escape(b, q, set) => (q + 2, k::ESCAPE),
            b'*' | b'?' => (q + 1, k::WILDCARD),
            b'!' => (q + 1, k::LOGICAL),
            b',' => (q + 1, k::SEPARATOR),
            b'[' => match ipv6(b, q + 1) {
                Some(v) if b.get(v) == Some(&b']') => {
                    c.tok(Some(text), q, q + 1, k::V6_BEGIN);
                    c.tok(None, q + 1, v, k::V6);
                    c.tok(None, v, v + 1, k::V6_END);
                    q = v + 1;
                    continue;
                }
                _ => {
                    q += 1;
                    continue;
                }
            },
            _ if word_start(b, q) => {
                let we = word_end(b, q);
                if q >= run {
                    (run, clean) = (q, q);
                    while !ends_value(b, run) {
                        if !LIST_BYTE[usize::from(b[run])] {
                            clean = run + 1;
                        }
                        run += 1;
                    }
                }
                let list = if q >= clean { run } else { 0 };
                match word(c, q, we, list, text, sshd) {
                    Some(e) => q = e,
                    // past the word's first byte only an IPv6 address can start
                    None => q = v6_in_word(c, q + 1, we, text),
                }
                continue;
            }
            b':' => match v6_cidr(c, q, Some(text)) {
                Some(e) => {
                    q = e;
                    continue;
                }
                None => {
                    q += 1;
                    continue;
                }
            },
            _ => {
                q += 1;
                continue;
            }
        };
        c.tok(Some(text), q, e, kind);
        q = e;
    }
    c.text(Some(text), end);
}

/// The patterns that start where a word does, at `q` (the word to `we`), in bat's order: where
/// the token ends, its spans written; `None` where none matches. A list's word would end at
/// `list`, 0 where a byte no list word has comes first.
#[inline(always)]
fn word<K: Sink>(
    c: &mut Cursor<K>,
    q: usize,
    we: usize,
    list: usize,
    text: u16,
    sshd: bool,
) -> Option<usize> {
    let b = c.b;
    let x = b[q];
    if sshd && x.is_ascii_alphabetic() {
        let e = we;
        let w = &b[q..e];
        let is = |s: &[u8]| w.eq_ignore_ascii_case(s);
        let kind = if is(b"yes") || is(b"no") {
            Some(k::BOOLEAN)
        } else if is(b"any") || is(b"none") {
            Some(k::SET)
        } else if is(b"default") {
            Some(k::DEFAULT)
        } else {
            None
        };
        if let Some(kind) = kind {
            c.tok(Some(text), q, e, kind);
            return Some(e);
        }
    }
    // `none` a set's word in a client's value too, as a server's has it (`IdentityAgent none`,
    // `ProxyJump none`), where bat's client reads a deprecated cipher's (after a `,`) or a string
    if !sshd && (x == b'n' || x == b'N') && b[q..we].eq_ignore_ascii_case(b"none") {
        c.tok(Some(text), q, q + 4, k::SET);
        return Some(q + 4);
    }
    if list > q && CRYPTO_FIRST[usize::from(x)] {
        let kind = crypto(&b[q..list]);
        if kind != 0 {
            c.tok(Some(text), q, list, kind);
            return Some(list);
        }
    }
    if HEX[usize::from(x)]
        && let Some(e) = v6_cidr(c, q, Some(text))
    {
        return Some(e);
    }
    if !x.is_ascii_digit() {
        return None;
    }
    if let Some(v) = ipv4(b, q) {
        // `(?:(/)(3[0-2]|[12]\d|\d))?\b`
        c.tok(Some(text), q, v, k::V4);
        if let Some(r) = cidr(b, v, false) {
            c.tok(None, v, v + 1, k::CIDR);
            c.tok(None, v + 1, r, k::RANGE);
            return Some(r);
        }
        return Some(v);
    }
    let d = digits(b, q);
    if d < b.len() {
        let u = b[d];
        // `time-values`, `(?=[\d\s,"])` after the unit; `bytes-values`
        let time = matches!(u.to_ascii_lowercase(), b's' | b'm' | b'h' | b'd' | b'w')
            && (ends_value(b, d + 1) || b.get(d + 1).is_some_and(u8::is_ascii_digit));
        let size = matches!(u, b'K' | b'M' | b'G') && ends_value(b, d + 1);
        if time || size {
            c.tok(Some(text), q, d, k::AMOUNT);
            c.tok(None, d, d + 1, k::UNIT);
            return Some(d + 1);
        }
    }
    if sshd && ends_value(b, d) {
        c.tok(Some(text), q, d, k::NUMBER);
        return Some(d);
    }
    None
}

/// `(/)(3[0-2]|[12]\d|\d)` then `\b` after an IPv4 address ending at `v`, or `(/)(12[0-8]|
/// 1[01]\d|[1-9]\d|\d)\b` after an IPv6 one (`v6`): the first alternative followed by `\b`.
/// Where the length ends.
fn cidr(b: &[u8], v: usize, v6: bool) -> Option<usize> {
    if b.get(v) != Some(&b'/') {
        return None;
    }
    let s = v + 1;
    let at = |i: usize| b.get(s + i).copied().unwrap_or(0);
    let d = |i: usize| at(i).is_ascii_digit();
    let lens: [usize; 4] = if v6 {
        [
            if at(0) == b'1' && at(1) == b'2' && (b'0'..=b'8').contains(&at(2)) {
                3
            } else {
                0
            },
            if at(0) == b'1' && matches!(at(1), b'0' | b'1') && d(2) {
                3
            } else {
                0
            },
            if (b'1'..=b'9').contains(&at(0)) && d(1) {
                2
            } else {
                0
            },
            if d(0) { 1 } else { 0 },
        ]
    } else {
        [
            if at(0) == b'3' && (b'0'..=b'2').contains(&at(1)) {
                2
            } else {
                0
            },
            if matches!(at(0), b'1' | b'2') && d(1) {
                2
            } else {
                0
            },
            if d(0) { 1 } else { 0 },
            0,
        ]
    };
    lens.into_iter()
        .filter(|&l| l > 0)
        .map(|l| s + l)
        .find(|&e| e == b.len() || !WORD[usize::from(b[e])])
}

/// `ipv6-with-cidr` at `q`: an IPv6 address and an optional `/` and prefix length.
#[inline(never)]
fn v6_cidr<K: Sink>(c: &mut Cursor<K>, q: usize, text: Option<u16>) -> Option<usize> {
    let b = c.b;
    let v = ipv6(b, q)?;
    c.tok(text, q, v, k::V6);
    if let Some(r) = cidr(b, v, true) {
        c.tok(None, v, v + 1, k::CIDR);
        c.tok(None, v + 1, r, k::RANGE);
        return Some(r);
    }
    Some(v)
}

/// A word's bytes from `q` to `e` past its first: an IPv6 address may start at a hex digit with
/// a `:` within five bytes. Where the scan goes on.
#[inline(always)]
fn v6_in_word<K: Sink>(c: &mut Cursor<K>, mut q: usize, e: usize, text: u16) -> usize {
    let b = c.b;
    // a word without a `:` after it can't hold one
    if e >= b.len() || b[e] != b':' {
        return e;
    }
    while q < e {
        if e - q <= 4
            && let Some(v) = v6_cidr(c, q, Some(text))
        {
            return v;
        }
        q += 1;
    }
    e
}

// --- a `Match` line -------------------------------------------------------------------------------

/// `pop-before-match-option`'s words: a client's, or a server's (`sshd`).
fn is_condition(w: &[u8], sshd: bool) -> bool {
    let is = |s: &[u8]| w.eq_ignore_ascii_case(s);
    if sshd {
        is(b"all")
            || is(b"user")
            || is(b"group")
            || is(b"host")
            || is(b"address")
            || is(b"localaddress")
            || is(b"localport")
    } else {
        is(b"all")
            || is(b"canonical")
            || is(b"exec")
            || is(b"host")
            || is(b"originalhost")
            || is(b"user")
            || is(b"localuser")
    }
}

/// `(?=[ \t]*(?i:all|…)\b)` at `q`: a condition's word ahead, past spaces and tabs.
#[inline(always)]
fn condition_ahead(b: &[u8], q: usize, sshd: bool) -> bool {
    let s = spaces(b, q);
    s < b.len() && b[s].is_ascii_alphabetic() && is_condition(&b[s..word_end(b, s)], sshd)
}

// what the words after a condition's keyword are
const C_NONE: u8 = 0;
const C_HOSTS: u8 = 1;
const C_USERS: u8 = 2;
const C_ADDRESSES: u8 = 3;
const C_PORT: u8 = 4;

/// A client's `Match` conditions from `q` (`match-conditions`): its keywords where a word
/// starts, `!`, the rest plain; after `host` its patterns, after `user` a string, each to the
/// next condition's word; `exec`'s command on the shell.
#[inline(never)]
fn conditions<K: Sink>(c: &mut Cursor<K>, mut q: usize) {
    let b = c.b;
    let n = b.len();
    let mut mode = C_NONE;
    while q < n {
        let x = b[q];
        if mode != C_NONE && condition_ahead(b, q, false) {
            mode = C_NONE;
        }
        match mode {
            C_HOSTS => match x {
                b',' => c.tok(None, q, q + 1, k::SEPARATOR),
                b'!' => c.tok(None, q, q + 1, k::LOGICAL),
                b'*' | b'?' => c.tok(None, q, q + 1, k::LABEL_WILDCARD),
                _ if BLANK[usize::from(x)] => {}
                _ => {
                    let mut e = q + 1;
                    while e < n && !(BLANK[usize::from(b[e])] || matches!(b[e], b'*' | b'?' | b','))
                    {
                        e += 1;
                    }
                    c.tok(None, q, e, k::LABEL);
                    q = e;
                    continue;
                }
            },
            // `\S` a byte at a time, a condition's word looked for at each
            C_USERS => {
                if !BLANK[usize::from(x)] {
                    c.tok(None, q, q + 1, k::STRING);
                }
            }
            _ => {
                if x == b'!' {
                    c.tok(None, q, q + 1, k::LOGICAL);
                } else if word_start(b, q) {
                    let e = word_end(b, q);
                    let w = &b[q..e];
                    let is = |s: &[u8]| w.eq_ignore_ascii_case(s);
                    if is(b"all") {
                        c.tok(None, q, e, k::BOOLEAN);
                    } else if is(b"canonical") || is(b"final") {
                        c.tok(None, q, e, k::KEY);
                    } else if is(b"host") || is(b"originalhost") {
                        c.tok(None, q, e, k::KEY);
                        mode = C_HOSTS;
                    } else if is(b"user") || is(b"localuser") {
                        c.tok(None, q, e, k::KEY);
                        mode = C_USERS;
                    } else if is(b"exec") {
                        q = exec(c, q, e);
                        continue;
                    }
                    q = e;
                    continue;
                }
            }
        }
        q += 1;
    }
}

/// `exec` from `q` to `e`, its command after it: in quotes to the first `"` without a `\`
/// before it, the quotes a string's; else after spaces to the next condition's word, `%`
/// escapes in it. Where the conditions go on.
fn exec<K: Sink>(c: &mut Cursor<K>, q: usize, e: usize) -> usize {
    let b = c.b;
    let n = b.len();
    let s = spaces(b, e);
    if s < n && b[s] == b'"' {
        c.tok(None, q, e, k::KEY);
        c.tok(None, s, s + 1, k::QUOTE_BEGIN);
        let mut end = s + 1;
        while end < n && !(b[end] == b'"' && b[end - 1] != b'\\') {
            end += 1;
        }
        shell(c, s + 1, end, 0, false);
        if end < n {
            c.tok(None, end, end + 1, k::QUOTE_END);
            return end + 1;
        }
        return n;
    }
    if s == e {
        // neither a quote nor a blank after it: plain
        return e;
    }
    c.tok(None, q, e, k::KEY);
    let mut end = s;
    while end < n && !(space_tab(b[end]) && condition_ahead(b, end, false)) {
        end += 1;
    }
    shell(c, s, end, ESC_EXEC, false);
    end
}

/// A server's `Match` conditions from `q`: its keywords where a word starts, `!`, the rest
/// plain; after `user`, `group` and `host` patterns as strings, after `address` and
/// `localaddress` addresses, after `localport` numbers, each to the next condition's word.
#[inline(never)]
fn sshd_conditions<K: Sink>(c: &mut Cursor<K>, mut q: usize) {
    let b = c.b;
    let n = b.len();
    let mut mode = C_NONE;
    while q < n {
        let x = b[q];
        match mode {
            C_USERS => {
                if condition_ahead(b, q, true) {
                    mode = C_NONE;
                    continue;
                }
                match x {
                    b'!' => c.tok(None, q, q + 1, k::LOGICAL),
                    b',' => c.tok(None, q, q + 1, k::SEPARATOR),
                    b'*' | b'?' => c.tok(None, q, q + 1, k::WILDCARD),
                    _ if BLANK[usize::from(x)] => {}
                    _ => {
                        let mut e = q + 1;
                        while e < n
                            && !(BLANK[usize::from(b[e])]
                                || matches!(b[e], b'*' | b'?' | b'!' | b','))
                        {
                            e += 1;
                        }
                        c.tok(None, q, e, k::STRING);
                        q = e;
                        continue;
                    }
                }
            }
            C_ADDRESSES => {
                let kind = match x {
                    b'!' => Some(k::LOGICAL),
                    b',' => Some(k::SEPARATOR),
                    b'*' | b'?' => Some(k::WILDCARD),
                    _ => None,
                };
                if let Some(kind) = kind {
                    c.tok(None, q, q + 1, kind);
                } else if (HEX[usize::from(x)] || x == b':')
                    && let Some(e) = v6_cidr(c, q, None)
                {
                    q = e;
                    continue;
                } else if let Some(v) = ipv4(b, q) {
                    c.tok(None, q, v, k::V4);
                    q = v;
                    if let Some(r) = cidr(b, v, false) {
                        c.tok(None, v, v + 1, k::CIDR);
                        c.tok(None, v + 1, r, k::RANGE);
                        q = r;
                    }
                    continue;
                } else if condition_ahead(b, q, true) {
                    mode = C_NONE;
                    continue;
                }
            }
            C_PORT => {
                if x.is_ascii_digit() {
                    let e = digits(b, q);
                    c.tok(None, q, e, k::DECIMAL);
                    q = e;
                    continue;
                }
                if condition_ahead(b, q, true) {
                    mode = C_NONE;
                    continue;
                }
            }
            _ => {
                if x == b'!' {
                    c.tok(None, q, q + 1, k::LOGICAL);
                } else if word_start(b, q) {
                    let e = word_end(b, q);
                    let w = &b[q..e];
                    let is = |s: &[u8]| w.eq_ignore_ascii_case(s);
                    let next = if is(b"all") {
                        c.tok(None, q, e, k::BOOLEAN);
                        C_NONE
                    } else if is(b"user") || is(b"group") || is(b"host") {
                        C_USERS
                    } else if is(b"address") || is(b"localaddress") {
                        C_ADDRESSES
                    } else if is(b"localport") {
                        C_PORT
                    } else {
                        q = e;
                        continue;
                    };
                    if next != C_NONE {
                        c.tok(None, q, e, k::KEY);
                    }
                    mode = next;
                    q = e;
                    continue;
                }
            }
        }
        q += 1;
    }
}

// --- the shell --------------------------------------------------------------------------------------

std::thread_local! {
    static SHELL: std::cell::RefCell<Vec<Span>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Where the next escape of `set` from `p` before `end` is.
#[inline(always)]
fn next_escape(b: &[u8], mut p: usize, end: usize, set: u128) -> Option<usize> {
    if set == 0 {
        return None;
    }
    while p + 1 < end {
        if escape(b, p, set) {
            return Some(p);
        }
        p += 1;
    }
    None
}

/// bash's scanner on `b[q..end]` from a fresh state, its spans laid back with the escapes of
/// `set` over them; where `none` may stand for the command (bat's `/none/` ahead of the shell),
/// `none` alone, the rest plain.
#[inline(never)]
fn shell<K: Sink>(c: &mut Cursor<K>, q: usize, end: usize, set: u128, none: bool) {
    let b = c.b;
    if none {
        let s = spaces(b, q);
        if b[s..end].starts_with(b"none") {
            c.tok(None, s, s + 4, k::SET);
            return;
        }
    }
    if q >= end {
        return;
    }
    SHELL.with_borrow_mut(|spans| {
        spans.clear();
        let mut state = State::default();
        crate::bash::line_spans(&b[q..end], &mut state, &mut Out::new(spans));
        let mut esc = next_escape(b, q, end, set);
        for sp in spans.iter() {
            let (mut s, e) = (q + sp.start as usize, q + sp.end as usize);
            while let Some(x) = esc
                && x < e
            {
                let from = s.max(c.at);
                if x > from {
                    c.tok(None, from, x, sp.kind);
                }
                c.tok(None, x, x + 2, k::ESCAPE);
                s = x + 2;
                esc = next_escape(b, x + 2, end, set);
            }
            let from = s.max(c.at);
            if e > from {
                c.tok(None, from, e, sp.kind);
            }
        }
        while let Some(x) = esc {
            c.tok(None, x, x + 2, k::ESCAPE);
            esc = next_escape(b, x + 2, end, set);
        }
    });
}

// --- SSH Crypto's lists ------------------------------------------------------------------------

/// A word of `SSH Crypto`'s lists: its kind, the first list's that has it (key types, ciphers,
/// key exchanges, MACs, as `generic-option-includes` orders them); 0 for none.
#[inline(always)]
fn crypto(w: &[u8]) -> u16 {
    CRYPTO.value(w, &CRYPTO_KINDS)
}

const CRYPTO_LIST: [&[u8]; CRYPTO_WORDS.len()] = {
    let mut l: [&[u8]; CRYPTO_WORDS.len()] = [b""; CRYPTO_WORDS.len()];
    let mut i = 0;
    while i < l.len() {
        l[i] = CRYPTO_WORDS[i].0;
        i += 1;
    }
    l
};

static CRYPTO: Words<512, { CRYPTO_WORDS.len() }, { text_len(&CRYPTO_LIST) }> =
    Words::new(&CRYPTO_LIST);

/// The bytes a list's word starts with.
static CRYPTO_FIRST: [bool; 256] = {
    let mut t = [false; 256];
    let mut i = 0;
    while i < CRYPTO_WORDS.len() {
        t[CRYPTO_WORDS[i].0[0] as usize] = true;
        i += 1;
    }
    t
};

/// The bytes list words are made of: `[A-Za-z0-9@._+=-]`.
static LIST_BYTE: [bool; 256] = {
    let mut t = WORD;
    t[b'@' as usize] = true;
    t[b'.' as usize] = true;
    t[b'+' as usize] = true;
    t[b'=' as usize] = true;
    t[b'-' as usize] = true;
    t
};

/// Each list word's kind by its id, none at 0.
static CRYPTO_KINDS: [u16; CRYPTO_WORDS.len() + 1] = {
    let mut t = [0; CRYPTO_WORDS.len() + 1];
    let mut i = 0;
    while i < CRYPTO_WORDS.len() {
        t[i + 1] = CRYPTO_WORDS[i].1;
        i += 1;
    }
    t
};

/// bat's lists, case kept (the patterns don't ignore it).
static CRYPTO_WORDS: [(&[u8], u16); 189] = [
    (b"des", k::CIPHER_DEPRECATED),
    (b"3des", k::CIPHER_DEPRECATED),
    (b"none", k::CIPHER_DEPRECATED),
    (b"arcfour", k::CIPHER_DEPRECATED),
    (b"des-cbc", k::CIPHER_DEPRECATED),
    (b"ssh-dss", k::KEY_TYPE_DEPRECATED),
    (b"ssh-rsa", k::KEY_TYPE),
    (b"3des-cbc", k::CIPHER_DEPRECATED),
    (b"3des-ctr", k::CIPHER_DEPRECATED),
    (b"hmac-md5", k::MAC_DEPRECATED),
    (b"idea-cbc", k::CIPHER_DEPRECATED),
    (b"idea-ctr", k::CIPHER_DEPRECATED),
    (b"ssh-rsa1", k::KEY_TYPE_DEPRECATED),
    (b"hmac-sha1", k::MAC),
    (b"ssh-ed448", k::KEY_TYPE),
    (b"aes128-cbc", k::CIPHER_DEPRECATED),
    (b"aes128-ctr", k::CIPHER),
    (b"aes128-gcm", k::CIPHER),
    (b"aes192-cbc", k::CIPHER_DEPRECATED),
    (b"aes192-ctr", k::CIPHER),
    (b"aes256-cbc", k::CIPHER_DEPRECATED),
    (b"aes256-ctr", k::CIPHER),
    (b"aes256-gcm", k::CIPHER),
    (b"arcfour128", k::CIPHER_DEPRECATED),
    (b"arcfour256", k::CIPHER_DEPRECATED),
    (b"ext-info-c", k::KEX),
    (b"ext-info-s", k::KEX),
    (b"cast128-cbc", k::CIPHER_DEPRECATED),
    (b"cast128-ctr", k::CIPHER_DEPRECATED),
    (b"hmac-md5-96", k::MAC_DEPRECATED),
    (b"hmac-ripemd", k::MAC_DEPRECATED),
    (b"hmac-sha256", k::MAC),
    (b"hmac-sha512", k::MAC),
    (b"ssh-ed25519", k::KEY_TYPE),
    (b"twofish-ctr", k::CIPHER),
    (b"blowfish-cbc", k::CIPHER_DEPRECATED),
    (b"blowfish-ctr", k::CIPHER_DEPRECATED),
    (b"des-cbc-ssh1", k::CIPHER_DEPRECATED),
    (b"hmac-sha1-96", k::MAC_DEPRECATED),
    (b"hmac-sha2-56", k::MAC),
    (b"rsa-sha2-256", k::KEY_TYPE),
    (b"rsa-sha2-512", k::KEY_TYPE),
    (b"rsa1024-sha1", k::KEX),
    (b"spi-sign-rsa", k::KEY_TYPE),
    (b"gss-gex-sha1-", k::KEX_DEPRECATED),
    (b"hmac-sha2-224", k::MAC),
    (b"hmac-sha2-256", k::MAC),
    (b"hmac-sha2-384", k::MAC),
    (b"hmac-sha2-512", k::MAC),
    (b"hmac-sha3-224", k::MAC),
    (b"hmac-sha3-256", k::MAC),
    (b"hmac-sha3-384", k::MAC),
    (b"hmac-sha3-512", k::MAC),
    (b"ssh-gost-2001", k::KEY_TYPE),
    (b"x509v3-ssh-rs", k::KEY_TYPE),
    (b"hmac-ripemd160", k::MAC_DEPRECATED),
    (b"rsa2048-sha256", k::KEX),
    (b"serpent128-cbc", k::CIPHER_DEPRECATED),
    (b"serpent128-ctr", k::CIPHER_DEPRECATED),
    (b"serpent192-cbc", k::CIPHER_DEPRECATED),
    (b"serpent192-ctr", k::CIPHER_DEPRECATED),
    (b"serpent256-cbc", k::CIPHER_DEPRECATED),
    (b"serpent256-ctr", k::CIPHER_DEPRECATED),
    (b"twofish128-cbc", k::CIPHER_DEPRECATED),
    (b"twofish128-ctr", k::CIPHER),
    (b"twofish192-cbc", k::CIPHER_DEPRECATED),
    (b"twofish192-ctr", k::CIPHER),
    (b"twofish256-cbc", k::CIPHER_DEPRECATED),
    (b"twofish256-ctr", k::CIPHER),
    (b"camellia128-cbc", k::CIPHER),
    (b"camellia128-ctr", k::CIPHER),
    (b"camellia192-cbc", k::CIPHER),
    (b"camellia192-ctr", k::CIPHER),
    (b"camellia256-cbc", k::CIPHER),
    (b"camellia256-ctr", k::CIPHER),
    (b"curve448-sha512", k::KEX),
    (b"rijndael128-cbc", k::CIPHER_DEPRECATED),
    (b"rijndael192-cbc", k::CIPHER_DEPRECATED),
    (b"rijndael256-cbc", k::CIPHER_DEPRECATED),
    (b"x509v3-sign-dss", k::KEY_TYPE_DEPRECATED),
    (b"x509v3-sign-rsa", k::KEY_TYPE),
    (b"AEAD_AES_128_GCM", k::CIPHER),
    (b"AEAD_AES_256_GCM", k::CIPHER),
    (b"Curve25519SHA256", k::KEX),
    (b"gss-group1-sha1-", k::KEX_DEPRECATED),
    (b"hmac-sha2-256-96", k::MAC_DEPRECATED),
    (b"hmac-sha2-512-96", k::MAC_DEPRECATED),
    (b"seed-cbc@ssh.com", k::CIPHER),
    (b"chacha20-poly1305", k::CIPHER),
    (b"curve25519-sha256", k::KEX),
    (b"gss-group14-sha1-", k::KEX_DEPRECATED),
    (b"ssh-gost-2012-256", k::KEY_TYPE),
    (b"ssh-gost-2012-512", k::KEY_TYPE),
    (b"ecdh-sha2-nistb233", k::KEX),
    (b"ecdh-sha2-nistb409", k::KEX),
    (b"ecdh-sha2-nistk163", k::KEX),
    (b"ecdh-sha2-nistk233", k::KEX),
    (b"ecdh-sha2-nistk283", k::KEX),
    (b"ecdh-sha2-nistk409", k::KEX),
    (b"ecdh-sha2-nistp192", k::KEX),
    (b"ecdh-sha2-nistp224", k::KEX),
    (b"ecdh-sha2-nistp256", k::KEX),
    (b"ecdh-sha2-nistp384", k::KEX),
    (b"ecdh-sha2-nistp521", k::KEX),
    (b"ecdh-sha2-nistt571", k::KEX),
    (b"ecdsa-sha2-nistp256", k::KEY_TYPE),
    (b"ecdsa-sha2-nistp384", k::KEY_TYPE),
    (b"ecdsa-sha2-nistp521", k::KEY_TYPE),
    (b"gss-group14-sha256-", k::KEX),
    (b"gss-group15-sha512-", k::KEX),
    (b"gss-group16-sha512-", k::KEX),
    (b"hmac-sha256@ssh.com", k::MAC),
    (b"hmac-sha512@ssh.com", k::MAC),
    (b"umac-32@openssh.com", k::MAC),
    (b"umac-64@openssh.com", k::MAC),
    (b"umac-96@openssh.com", k::MAC),
    (b"ecdh-sha2-curve25519", k::KEX),
    (b"gss-nistp256-sha256-", k::KEX),
    (b"umac-128@openssh.com", k::MAC),
    (b"crypticore128@ssh.com", k::CIPHER),
    (b"x509v3-rsa2408-sha256", k::KEY_TYPE),
    (b"aes128-gcm@openssh.com", k::CIPHER),
    (b"aes256-gcm@openssh.com", k::CIPHER),
    (b"crypticore-mac@ssh.com", k::MAC),
    (b"ecdh-sha2-1.3.132.0.10", k::KEX),
    (b"gss-curve25519-sha256-", k::KEX),
    (b"hmac-sha256-96@ssh.com", k::MAC),
    (b"ssh-dss-sha256@ssh.com", k::KEY_TYPE),
    (b"ssh-rsa-sha256@ssh.com", k::KEY_TYPE),
    (b"ecdsa-sha2-1.3.132.0.10", k::KEY_TYPE),
    (b"kexAlgoCurve25519SHA256", k::KEX),
    (b"umac-64-etm@openssh.com", k::MAC),
    (b"hmac-md5-etm@openssh.com", k::MAC_DEPRECATED),
    (b"umac-128-etm@openssh.com", k::MAC),
    (b"hmac-sha1-etm@openssh.com", k::MAC),
    (b"kexguess2@matt.ucc.asn.au", k::KEX),
    (b"diffie-hellman-group1-sha1", k::KEX_DEPRECATED),
    (b"hmac-ripemd160@openssh.com", k::MAC_DEPRECATED),
    (b"x509v3-ecdsa-sha2-nistp256", k::KEY_TYPE),
    (b"x509v3-ecdsa-sha2-nistp384", k::KEY_TYPE),
    (b"x509v3-ecdsa-sha2-nistp521", k::KEY_TYPE),
    (b"diffie-hellman-group14-sha1", k::KEX_DEPRECATED),
    (b"hmac-md5-96-etm@openssh.com", k::MAC_DEPRECATED),
    (b"rijndael-cbc@lysator.liu.se", k::CIPHER_DEPRECATED),
    (b"curve25519-sha256@libssh.org", k::KEX),
    (b"diffie-hellman-group1-sha256", k::KEX),
    (b"hmac-sha1-96-etm@openssh.com", k::MAC_DEPRECATED),
    (b"ssh-dss-cert-v00@openssh.com", k::KEY_TYPE_DEPRECATED),
    (b"ssh-dss-cert-v01@openssh.com", k::KEY_TYPE_DEPRECATED),
    (b"ssh-rsa-cert-v00@openssh.com", k::KEY_TYPE_DEPRECATED),
    (b"ssh-rsa-cert-v01@openssh.com", k::KEY_TYPE),
    (b"chacha20-poly1305@openssh.com", k::CIPHER),
    (b"diffie-hellman-group14-sha256", k::KEX),
    (b"diffie-hellman-group15-sha256", k::KEX),
    (b"diffie-hellman-group15-sha512", k::KEX),
    (b"diffie-hellman-group16-sha256", k::KEX),
    (b"diffie-hellman-group16-sha512", k::KEX),
    (b"diffie-hellman-group17-sha512", k::KEX),
    (b"diffie-hellman-group18-sha512", k::KEX),
    (b"hmac-sha2-256-etm@openssh.com", k::MAC),
    (b"hmac-sha2-512-etm@openssh.com", k::MAC),
    (b"hmac-ripemd160-etm@openssh.com", k::MAC_DEPRECATED),
    (b"x509v3-sign-rsa-sha256@ssh.com", k::KEY_TYPE),
    (b"hmac-sha2-256-96-etm@openssh.com", k::MAC),
    (b"hmac-sha2-512-96-etm@openssh.com", k::MAC),
    (b"ssh-ed25519-cert-v01@openssh.com", k::KEY_TYPE),
    (b"rsa-sha2-256-cert-v01@openssh.com", k::KEY_TYPE),
    (b"rsa-sha2-512-cert-v01@openssh.com", k::KEY_TYPE),
    (b"diffie-hellman-group-exchange-sha1", k::KEX_DEPRECATED),
    (b"sk-ecdsa-sha2-nistp256@openssh.com", k::KEY_TYPE),
    (b"diffie-hellman-group-exchange-sha256", k::KEX),
    (b"diffie-hellman-group14-sha256@ssh.com", k::KEX),
    (b"diffie-hellman-group15-sha256@ssh.com", k::KEX),
    (b"diffie-hellman-group15-sha384@ssh.com", k::KEX),
    (b"diffie-hellman-group16-sha384@ssh.com", k::KEX),
    (b"diffie-hellman-group16-sha512@ssh.com", k::KEX),
    (b"diffie-hellman-group18-sha512@ssh.com", k::KEX),
    (b"gss-gex-sha1-toWM5Slw5Ew8Mqkay+al2g==", k::KEX_DEPRECATED),
    (b"sntrup4591761x25519-sha512@tinyssh.org", k::KEX),
    (b"ecdsa-sha2-nistp256-cert-v01@openssh.com", k::KEY_TYPE),
    (b"ecdsa-sha2-nistp384-cert-v01@openssh.com", k::KEY_TYPE),
    (b"ecdsa-sha2-nistp521-cert-v01@openssh.com", k::KEY_TYPE),
    (
        b"gss-group1-sha1-toWM5Slw5Ew8Mqkay+al2g==",
        k::KEX_DEPRECATED,
    ),
    (
        b"gss-group14-sha1-toWM5Slw5Ew8Mqkay+al2g==",
        k::KEX_DEPRECATED,
    ),
    (b"gss-group14-sha256-toWM5Slw5Ew8Mqkay+al2g==", k::KEX),
    (b"gss-group15-sha512-toWM5Slw5Ew8Mqkay+al2g==", k::KEX),
    (b"sk-ecdsa-sha2-nistp256-cert-v01@openssh.com", k::KEY_TYPE),
    (b"diffie-hellman-group-exchange-sha256@ssh.com", k::KEX),
    (b"diffie-hellman-group-exchange-sha512@ssh.com", k::KEX),
];

#[cfg(test)]
mod tests {
    use crate::{Lang, State, lex};

    /// Each line's tokens as `text·leaf`, the leaf a kind's last scope without its suffix, plain
    /// text left out.
    fn toks(lang: Lang, text: &str) -> Vec<String> {
        let mut state = State::default();
        let mut spans = Vec::new();
        text.lines()
            .map(|l| {
                lex(lang, l, &mut state, &mut spans);
                spans
                    .iter()
                    .map(|s| {
                        let scopes = lang.scopes(s.kind);
                        let leaf = scopes.last().map_or("?", |s| {
                            [".ssh_config", ".sshd_config", ".ssh.common", ".shell"]
                                .iter()
                                .find_map(|x| s.strip_suffix(x))
                                .unwrap_or(s)
                        });
                        format!("{}·{leaf}", &l[s.start as usize..s.end as usize])
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }

    #[test]
    fn a_host_block() {
        let t = toks(
            Lang::SshConfig,
            "Host a b* !c\n  HostName 10.0.0.1 x\n  Port 22\n  ForwardAgent yes no\n",
        );
        assert_eq!(
            t[0],
            "Host·keyword.declaration.host-alias a·entity.name.label.host-alias \
            b·entity.name.label.host-alias *·keyword.operator.wildcard !·keyword.operator.logical \
            c·entity.name.label.host-alias"
        );
        assert_eq!(
            t[1],
            "HostName·keyword.declaration.host 10.0.0.1·constant.numeric.ip-address.v4 \
            x·invalid.illegal"
        );
        assert_eq!(t[2], "Port·keyword.other 22·constant.numeric");
        assert_eq!(
            t[3],
            "ForwardAgent·keyword.other yes·constant.language.boolean  no·invalid.illegal"
        );
    }

    /// bat leaves a client's first list word plain (its string's first byte a token alone): ada
    /// colors it, as bat's server config does.
    #[test]
    fn a_lists_first_word_is_colored() {
        let t = toks(
            Lang::SshConfig,
            "Ciphers aes128-ctr,3des-cbc\nServerAliveInterval 60s\nProxyJump none\n",
        );
        assert_eq!(
            t[0],
            "Ciphers·keyword.other aes128-ctr·support.function.cipher \
            ,·punctuation.separator.sequence 3des-cbc·invalid.deprecated.cipher"
        );
        assert_eq!(
            t[1],
            "ServerAliveInterval·keyword.other 60·constant.numeric.integer.decimal \
            s·storage.modifier.unit"
        );
        assert_eq!(t[2], "ProxyJump·keyword.other none·constant.language.set");
    }

    #[test]
    fn the_shell_with_the_hosts_escapes() {
        let t = toks(
            Lang::SshConfig,
            "ProxyCommand ssh -W %h:%p bastion\nProxyCommand none\n",
        );
        assert!(t[0].contains("%h·constant.character.escape"), "{}", t[0]);
        assert!(t[0].contains("%p·constant.character.escape"), "{}", t[0]);
        assert!(
            t[0].contains("-·punctuation.definition.parameter"),
            "{}",
            t[0]
        );
        assert_eq!(
            t[1],
            "ProxyCommand·keyword.other none·constant.language.set"
        );
    }

    /// bat's shell runs on past an open quote into the lines after it; here a value ends with its
    /// line.
    #[test]
    fn an_open_quote_ends_with_its_line() {
        let t = toks(Lang::SshConfig, "LocalCommand echo \"%h\n  Port 22\n");
        assert_eq!(t[1], "Port·keyword.other 22·constant.numeric");
    }

    #[test]
    fn match_conditions() {
        let t = toks(
            Lang::SshConfig,
            "Match host a,b user bob exec \"test -f x\" all\n",
        );
        assert!(
            t[0].starts_with(
                "Match·keyword.control.conditional host·keyword.other \
            a·entity.name.label.host-alias ,·punctuation.separator.sequence \
            b·entity.name.label.host-alias user·keyword.other bob·string.unquoted \
            exec·keyword.other \"·punctuation.definition.string.begin"
            ),
            "{}",
            t[0]
        );
        assert!(
            t[0].ends_with("\"·punctuation.definition.string.end all·constant.language.boolean"),
            "{}",
            t[0]
        );
        let t = toks(
            Lang::SshdConfig,
            "Match Address 10.0.0.0/8,::1 LocalPort 22 User *bob\n",
        );
        assert_eq!(
            t[0],
            "Match·keyword.control.conditional Address·keyword.other \
            10.0.0.0·constant.numeric.ip-address.v4 /·punctuation.separator.sequence \
            8·constant.other.range ,·punctuation.separator.sequence \
            ::1·constant.numeric.ip-address.v6 LocalPort·keyword.other \
            22·constant.numeric.integer.decimal User·keyword.other *·keyword.operator.wildcard \
            bob·string.unquoted"
        );
    }

    #[test]
    fn a_servers_values() {
        let t = toks(
            Lang::SshdConfig,
            "# a comment\nPermitRootLogin no\nMaxStartups 10:30:100\nRekeyLimit 1G 1h\nX none,default\n",
        );
        assert_eq!(
            t[0],
            "#·punctuation.definition.comment  a comment·comment.line.number-sign"
        );
        assert_eq!(
            t[1],
            "PermitRootLogin·keyword.other no·constant.language.boolean"
        );
        assert_eq!(
            t[3],
            "RekeyLimit·keyword.other 1·constant.numeric.integer.decimal \
            G·storage.modifier.unit  ·string.unquoted 1·constant.numeric.integer.decimal \
            h·storage.modifier.unit"
        );
        assert_eq!(
            t[4],
            "X·keyword.other none·constant.language.set ,·punctuation.separator.sequence \
            default·constant.language.default"
        );
    }

    #[test]
    fn long_lines_lex_in_linear_time() {
        for lang in [Lang::SshConfig, Lang::SshdConfig] {
            for unit in [
                "a:",
                "%h",
                "1.",
                "Match user a",
                "\"x",
                "!*,",
                "aes128-ctr ",
                "ab:cd::",
            ] {
                let line = format!("Ciphers {}", unit.repeat(20_000));
                let t = std::time::Instant::now();
                lex(lang, &line, &mut State::default(), &mut Vec::new());
                let line = format!("Match exec {}", unit.repeat(20_000));
                lex(lang, &line, &mut State::default(), &mut Vec::new());
                assert!(t.elapsed().as_secs_f64() < 2.0, "{lang:?} {unit:?}");
            }
        }
    }
}
