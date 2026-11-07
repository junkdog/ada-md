//! What bat's hidden `SSH Common` and `SSH Crypto` grammars give the SSH formats (X-045): the
//! addresses, the key types and the other algorithm lists, a key's fingerprint and the label
//! after it. Known Hosts reads them; SSH Config and SSHD Config include the same contexts.
//!
//! The IPv6 pattern is twelve ordered alternatives of counted repeats, without a `\b` at its
//! start, and where a `]` must follow it (`ipv6-square-bracket`) a shorter alternative can be
//! the one that matches; so the addresses go through a small backtracking matcher ([`Re`]) that
//! takes the alternatives and repeats in the order Oniguruma does, the first path that
//! matches being the match. A pattern is tried only where its first byte can start it.

use crate::prim::WORD;

/// A pattern for [`matches`]: a byte of a class, a run of bytes (ASCII's case ignored), a
/// sequence, ordered alternatives, a greedy counted repeat, `\b`; and a class's whole run, one
/// byte or more, never given back (`[0-9a-z]{1,}` where no byte of its class can follow).
#[derive(Clone, Copy)]
pub enum Re {
    Class(&'static [bool; 256]),
    Lit(&'static [u8]),
    Seq(&'static [Re]),
    Alt(&'static [Re]),
    Rep(&'static Re, u8, u8),
    #[cfg(test)]
    Boundary,
    Run(&'static [bool; 256]),
}

/// Whether `re` matches `b` at `p` and `then` takes where it ends, trying the paths in the
/// order a backtracking matcher does: alternatives in order, repeats greedy.
pub fn matches(re: &Re, b: &[u8], p: usize, then: &mut dyn FnMut(usize) -> bool) -> bool {
    match *re {
        Re::Class(t) => p < b.len() && t[usize::from(b[p])] && then(p + 1),
        Re::Lit(s) => {
            p + s.len() <= b.len() && b[p..p + s.len()].eq_ignore_ascii_case(s) && then(p + s.len())
        }
        Re::Seq(items) => seq(items, b, p, then),
        Re::Alt(alts) => alts.iter().any(|a| matches(a, b, p, then)),
        Re::Rep(r, min, max) => rep(r, min, max, 0, b, p, then),
        #[cfg(test)]
        Re::Boundary => {
            let before = p > 0 && WORD[usize::from(b[p - 1])];
            let after = p < b.len() && WORD[usize::from(b[p])];
            before != after && then(p)
        }
        Re::Run(t) => {
            let mut e = p;
            while e < b.len() && t[usize::from(b[e])] {
                e += 1;
            }
            e > p && then(e)
        }
    }
}

fn seq(items: &[Re], b: &[u8], p: usize, then: &mut dyn FnMut(usize) -> bool) -> bool {
    match items.split_first() {
        None => then(p),
        Some((first, rest)) => matches(first, b, p, &mut |q| seq(rest, b, q, then)),
    }
}

fn rep(
    r: &Re,
    min: u8,
    max: u8,
    n: u8,
    b: &[u8],
    p: usize,
    then: &mut dyn FnMut(usize) -> bool,
) -> bool {
    // one more first (greedy), each taking at least a byte; then as few as are enough
    (n < max
        && matches(r, b, p, &mut |q| {
            q > p && rep(r, min, max, n + 1, b, q, then)
        }))
        || (n >= min && then(p))
}

/// Where `re`'s first path from `p` ends, the match taken as a whole.
pub fn match_end(re: &Re, b: &[u8], p: usize) -> Option<usize> {
    let mut end = None;
    matches(re, b, p, &mut |e| {
        end = Some(e);
        true
    });
    end
}

/// A class of bytes from ranges, each inclusive.
const fn class(ranges: &[(u8, u8)]) -> [bool; 256] {
    let mut t = [false; 256];
    let mut i = 0;
    while i < ranges.len() {
        let (lo, hi) = ranges[i];
        let mut c = lo as usize;
        while c <= hi as usize {
            t[c] = true;
            c += 1;
        }
        i += 1;
    }
    t
}

static DIGIT: [bool; 256] = class(&[(b'0', b'9')]);
static NONZERO: [bool; 256] = class(&[(b'1', b'9')]);
static TO4: [bool; 256] = class(&[(b'0', b'4')]);
static TO5: [bool; 256] = class(&[(b'0', b'5')]);
/// `[0-9a-f]` under `(?i)`.
pub static HEX: [bool; 256] = class(&[(b'0', b'9'), (b'a', b'f'), (b'A', b'F')]);
/// `[0-9a-z]` under `(?i)`.
static ALNUM: [bool; 256] = class(&[(b'0', b'9'), (b'a', b'z'), (b'A', b'Z')]);

const D: Re = Re::Class(&DIGIT);
const X: Re = Re::Class(&HEX);
/// `[0-9a-f]{1,4}`.
const H: Re = Re::Rep(&X, 1, 4);
/// `(?:[0-9a-f]{1,4}:)`.
const HC: Re = Re::Seq(&[H, Re::Lit(b":")]);
/// `(?::[0-9a-f]{1,4})`.
const CH: Re = Re::Seq(&[Re::Lit(b":"), H]);

/// An octet, `(?:25[0-5])|(?:2[0-4][0-9])|(?:1[0-9][0-9])|(?:[1-9][0-9])|[0-9]`.
const OCTET: Re = Re::Alt(&[
    Re::Seq(&[Re::Lit(b"25"), Re::Class(&TO5)]),
    Re::Seq(&[Re::Lit(b"2"), Re::Class(&TO4), D]),
    Re::Seq(&[Re::Lit(b"1"), D, D]),
    Re::Seq(&[Re::Class(&NONZERO), D]),
    D,
]);

/// Four octets, `(?:OCTET\.){3}OCTET`.
const QUAD: Re = Re::Seq(&[Re::Rep(&Re::Seq(&[OCTET, Re::Lit(b".")]), 3, 3), OCTET]);

/// `ipv4`'s pattern: `\b QUAD \b`, [`ipv4`]'s reference in the tests.
#[cfg(test)]
static IPV4: Re = Re::Seq(&[Re::Boundary, QUAD, Re::Boundary]);

/// `ipv6`'s pattern, its twelve alternatives in order, case ignored.
pub static IPV6: Re = Re::Alt(&[
    // ::255.255.255.255  ::ffff:255.255.255.255  ::ffff:0:255.255.255.255
    Re::Seq(&[
        Re::Lit(b"::"),
        Re::Rep(
            &Re::Seq(&[
                Re::Lit(b"ffff"),
                Re::Rep(
                    &Re::Seq(&[Re::Lit(b":"), Re::Rep(&Re::Lit(b"0"), 1, 4)]),
                    0,
                    1,
                ),
                Re::Lit(b":"),
            ]),
            0,
            1,
        ),
        QUAD,
    ]),
    // 2001:db8:3:4::192.0.2.33  64:ff9b::192.0.2.33
    Re::Seq(&[Re::Rep(&HC, 1, 4), Re::Lit(b":"), QUAD]),
    // fe80::7:8%eth0
    Re::Seq(&[
        Re::Lit(b"fe80:"),
        Re::Rep(&CH, 0, 4),
        Re::Lit(b"%"),
        // `[0-9a-z]{1,}`: what follows the address (`]`, `/`, nothing) is never one of its
        // bytes, so the whole run is the only length that can match
        Re::Run(&ALNUM),
    ]),
    // 1:2:3:4:5:6:7:8
    Re::Seq(&[Re::Rep(&HC, 7, 7), H]),
    // 1::3:4:5:6:7:8
    Re::Seq(&[HC, Re::Rep(&CH, 1, 6)]),
    Re::Seq(&[Re::Rep(&HC, 1, 2), Re::Rep(&CH, 1, 5)]),
    Re::Seq(&[Re::Rep(&HC, 1, 3), Re::Rep(&CH, 1, 4)]),
    Re::Seq(&[Re::Rep(&HC, 1, 4), Re::Rep(&CH, 1, 3)]),
    Re::Seq(&[Re::Rep(&HC, 1, 5), Re::Rep(&CH, 1, 2)]),
    // 1::8
    Re::Seq(&[Re::Rep(&HC, 1, 6), Re::Lit(b":"), H]),
    // 1::
    Re::Seq(&[Re::Rep(&HC, 1, 7), Re::Lit(b":")]),
    // ::2:3:4:5:6:7:8  ::
    Re::Seq(&[Re::Lit(b":"), Re::Alt(&[Re::Rep(&CH, 1, 7), Re::Lit(b":")])]),
]);

/// Where an IPv6 address from `p` ends (`ipv6`), if one starts there.
#[inline(never)]
pub fn ipv6(b: &[u8], p: usize) -> Option<usize> {
    // every alternative starts with a `:`, or with one to four hex digits and a `:` (`fe80:`
    // too): a name that starts with hex letters is passed without the matcher
    if *b.get(p)? != b':' {
        let run = b[p..]
            .iter()
            .take(5)
            .take_while(|&&x| HEX[usize::from(x)])
            .count();
        if !(1..=4).contains(&run) || b.get(p + run) != Some(&b':') {
            return None;
        }
    }
    match_end(&IPV6, b, p)
}

/// Where an IPv4 address from `p` ends (`ipv4`), if one starts there: the pattern (`IPV4`, the tests') read without
/// the matcher: each octet must be its whole run of digits, as a shorter one leaves a digit
/// where the `.` or the `\b` after it must be, so no octet has a choice to backtrack to.
#[inline]
pub fn ipv4(b: &[u8], p: usize) -> Option<usize> {
    if !DIGIT[usize::from(*b.get(p)?)] || (p > 0 && WORD[usize::from(b[p - 1])]) {
        return None;
    }
    let mut q = p;
    for i in 0..4 {
        let run = b[q..]
            .iter()
            .take(4)
            .take_while(|x| x.is_ascii_digit())
            .count();
        let ok = match b.get(q..q + run)? {
            [_] => true,
            [x, _] => *x != b'0',
            [b'1', _, _] => true,
            [b'2', y, z] => *y < b'5' || (*y == b'5' && *z <= b'5'),
            _ => false,
        };
        q += run;
        if !ok {
            return None;
        }
        if i < 3 {
            if b.get(q) != Some(&b'.') {
                return None;
            }
            q += 1;
        }
    }
    (q == b.len() || !WORD[usize::from(b[q])]).then_some(q)
}

/// `[\w+/]`, a fingerprint's or a hash's byte.
pub static BASE64: [bool; 256] = {
    let mut t = WORD;
    t[b'+' as usize] = true;
    t[b'/' as usize] = true;
    t
};

/// `ssh-fingerprint`'s `AAAA(?:E2V|[BC]3N)[\w+/]+={0,3}` at `p`: where it ends.
#[inline(never)]
pub fn fingerprint(b: &[u8], p: usize) -> Option<usize> {
    let head = b.get(p..p + 7)?;
    if !(head.starts_with(b"AAAA")
        && matches!(&head[4..], b"E2V" | b"B3N" | b"C3N")
        && b.get(p + 7).is_some_and(|&c| BASE64[usize::from(c)]))
    {
        return None;
    }
    let mut e = p + 8;
    // a key is most of its line: eight bytes a step while all are the key's
    while let Some(c) = b.get(e..e + 8)
        && c.iter().fold(true, |all, &x| all & BASE64[usize::from(x)])
    {
        e += 8;
    }
    while e < b.len() && BASE64[usize::from(b[e])] {
        e += 1;
    }
    let mut n = 0;
    while n < 3 && e < b.len() && b[e] == b'=' {
        e += 1;
        n += 1;
    }
    Some(e)
}

/// `\s` as ASCII reads it.
fn space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// `ssh-fingerprint-with-label`'s label after a fingerprint ending at `p`,
/// `[ \t]*(\S(?:.*\S)?)[ \t]*(?m:$)`: the first byte that isn't `\s` to the last, where only
/// spaces and tabs follow that one; else none.
#[inline]
pub fn label(b: &[u8], p: usize) -> Option<(usize, usize)> {
    let mut t = b.len();
    while t > p && matches!(b[t - 1], b' ' | b'\t') {
        t -= 1;
    }
    if t == p || space(b[t - 1]) {
        return None;
    }
    let s = (p..t).find(|&i| !space(b[i]))?;
    Some((s, t))
}

/// What a word from `SSH Crypto`'s lists is: listed, or listed as deprecated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Listed {
    Supported,
    Deprecated,
}

/// A list's word at `p`: `\b(?:…)(?=[,\s"])`, the run to the first `,`, `\s` or `"` (the
/// line's end being its `\n`), where it's one of `list`'s and a word doesn't go on before `p`.
/// Where it ends, and which of the two lists has it. Only a byte some word starts with goes on
/// to the run and the search.
#[inline]
pub fn listed(b: &[u8], p: usize, list: &List) -> Option<(usize, Listed)> {
    if !list.first[usize::from(b[p])] || (p > 0 && WORD[usize::from(b[p - 1])]) {
        return None;
    }
    listed_run(b, p, list.words)
}

#[inline(never)]
fn listed_run(b: &[u8], p: usize, words: &Words) -> Option<(usize, Listed)> {
    let mut e = p;
    while e < b.len() && !(space(b[e]) || b[e] == b',' || b[e] == b'"') {
        e += 1;
    }
    lookup(words, &b[p..e]).map(|l| (e, l))
}

/// A list's words with what each is, by length, then bytes: [`lookup`]'s order.
type Words = [(&'static [u8], Listed)];

/// A list: its words, and the bytes they start with.
pub struct List {
    words: &'static Words,
    first: [bool; 256],
}

impl List {
    const fn new(words: &'static Words) -> List {
        let mut first = [false; 256];
        let mut i = 0;
        while i < words.len() {
            first[words[i].0[0] as usize] = true;
            i += 1;
        }
        List { words, first }
    }
}

/// `w` in `words` (sorted as [`Words`] says), by a binary search: a table, where a `match` on
/// the words compiled to ~6 KB of compares for the key types alone.
fn lookup(words: &Words, w: &[u8]) -> Option<Listed> {
    words
        .binary_search_by(|(k, _)| k.len().cmp(&w.len()).then_with(|| k.cmp(&w)))
        .ok()
        .map(|i| words[i].1)
}

/// `ssh-key-types`: `support.type.key-type`, else `invalid.deprecated.key-type`.
pub static KEY_TYPES: List = List::new(&KEY_TYPE_WORDS);

static KEY_TYPE_WORDS: [(&[u8], Listed); 37] = [
    (b"ssh-dss", Listed::Deprecated),
    (b"ssh-rsa", Listed::Supported),
    (b"ssh-rsa1", Listed::Deprecated),
    (b"ssh-ed448", Listed::Supported),
    (b"ssh-ed25519", Listed::Supported),
    (b"rsa-sha2-256", Listed::Supported),
    (b"rsa-sha2-512", Listed::Supported),
    (b"spi-sign-rsa", Listed::Supported),
    (b"ssh-gost-2001", Listed::Supported),
    (b"x509v3-ssh-rs", Listed::Supported),
    (b"x509v3-sign-dss", Listed::Deprecated),
    (b"x509v3-sign-rsa", Listed::Supported),
    (b"ssh-gost-2012-256", Listed::Supported),
    (b"ssh-gost-2012-512", Listed::Supported),
    (b"ecdsa-sha2-nistp256", Listed::Supported),
    (b"ecdsa-sha2-nistp384", Listed::Supported),
    (b"ecdsa-sha2-nistp521", Listed::Supported),
    (b"x509v3-rsa2408-sha256", Listed::Supported),
    (b"ssh-dss-sha256@ssh.com", Listed::Supported),
    (b"ssh-rsa-sha256@ssh.com", Listed::Supported),
    (b"ecdsa-sha2-1.3.132.0.10", Listed::Supported),
    (b"x509v3-ecdsa-sha2-nistp256", Listed::Supported),
    (b"x509v3-ecdsa-sha2-nistp384", Listed::Supported),
    (b"x509v3-ecdsa-sha2-nistp521", Listed::Supported),
    (b"ssh-dss-cert-v00@openssh.com", Listed::Deprecated),
    (b"ssh-dss-cert-v01@openssh.com", Listed::Deprecated),
    (b"ssh-rsa-cert-v00@openssh.com", Listed::Deprecated),
    (b"ssh-rsa-cert-v01@openssh.com", Listed::Supported),
    (b"x509v3-sign-rsa-sha256@ssh.com", Listed::Supported),
    (b"ssh-ed25519-cert-v01@openssh.com", Listed::Supported),
    (b"rsa-sha2-256-cert-v01@openssh.com", Listed::Supported),
    (b"rsa-sha2-512-cert-v01@openssh.com", Listed::Supported),
    (b"sk-ecdsa-sha2-nistp256@openssh.com", Listed::Supported),
    (
        b"ecdsa-sha2-nistp256-cert-v01@openssh.com",
        Listed::Supported,
    ),
    (
        b"ecdsa-sha2-nistp384-cert-v01@openssh.com",
        Listed::Supported,
    ),
    (
        b"ecdsa-sha2-nistp521-cert-v01@openssh.com",
        Listed::Supported,
    ),
    (
        b"sk-ecdsa-sha2-nistp256-cert-v01@openssh.com",
        Listed::Supported,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv6_takes_the_first_alternative_that_matches() {
        let end = |s: &str| ipv6(s.as_bytes(), 0);
        assert_eq!(end("::1"), Some(3));
        assert_eq!(end("::"), Some(2));
        assert_eq!(end("::ffff:1.2.3.4"), Some(14));
        assert_eq!(end("1:2:3:4:5:6:7:8"), Some(15));
        assert_eq!(end("fe80::7:8%eth0 x"), Some(14));
        assert_eq!(end("2001:db8::1"), Some(11));
        assert_eq!(end("1::"), Some(3));
        assert_eq!(end("g::1"), None);
        assert_eq!(end("abc"), None);
        // the third alternative takes `fe80:` and its zone; the fourth on would take less
        assert_eq!(end("FE80::1%A"), Some(9));
    }

    #[test]
    fn ipv4_wants_a_boundary_on_each_side() {
        let end = |s: &str, p| ipv4(s.as_bytes(), p);
        assert_eq!(end("1.2.3.4", 0), Some(7));
        assert_eq!(end("255.255.255.255,", 0), Some(15));
        assert_eq!(end("1.2.3.456", 0), None);
        assert_eq!(end("1.2.3.4a", 0), None);
        assert_eq!(end("x1.2.3.4", 1), None);
    }

    /// The hand-read IPv4 against its pattern on the matcher, at every place of strings of
    /// digits, dots and a letter.
    #[test]
    fn ipv4_reads_as_its_pattern_matches() {
        let mut x = 0x2545_f491_4f6c_dd1d_u64;
        for _ in 0..200_000 {
            let mut s = Vec::new();
            for _ in 0..(x % 19) {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                s.push(b"0123456789.....a_"[(x % 17) as usize]);
            }
            for p in 0..s.len() {
                let want = (DIGIT[usize::from(s[p])])
                    .then(|| match_end(&IPV4, &s, p))
                    .flatten();
                assert_eq!(
                    ipv4(&s, p),
                    want,
                    "{:?} at {p}",
                    String::from_utf8_lossy(&s)
                );
            }
        }
    }

    #[test]
    fn the_word_tables_are_sorted() {
        let words = KEY_TYPES.words;
        assert!(
            words
                .windows(2)
                .all(|w| (w[0].0.len(), w[0].0) < (w[1].0.len(), w[1].0))
        );
        assert!(words.iter().all(|&(k, l)| lookup(words, k) == Some(l)));
    }

    #[test]
    fn a_label_runs_to_the_last_byte_that_isnt_a_space() {
        let l = |s: &str| label(s.as_bytes(), 0);
        assert_eq!(l(" a b  "), Some((1, 4)));
        assert_eq!(l("   "), None);
        assert_eq!(l("a\x0b"), None);
        assert_eq!(l("\x0ba"), Some((1, 2)));
    }
}
