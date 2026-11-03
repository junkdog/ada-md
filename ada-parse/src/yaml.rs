//! What YAML's scanner (`yaml_scan.rs`) shares from the port of bat's `YAML` it replaced: the
//! matchers of the directives', tags' and implicit types' regexes. `\s`, `\S`, `\d` and `\w`
//! are ASCII here, and a byte past ASCII counts as a character of a plain scalar or a tag.

use crate::matcher::{Line, M, first, is_blank, is_space};

// the matchers: each finds its regex's leftmost match at or after `from`

/// `[\[\]{},]`
fn is_flow(c: u8) -> bool {
    b"[]{},".contains(&c)
}

/// Where the spaces from `p` end, the `\n` among them.
fn ws(l: &Line, p: usize) -> usize {
    l.run(p, is_space)
}

/// Where a plain scalar ends, out of flow collections: `\s*$`, `\s+\#`, or `\s*:(\s|$)`
/// after `p`.
pub(crate) fn out_end_at(l: &Line, p: usize) -> bool {
    let q = ws(l, p);
    q == l.end()
        || (q > p && l.at(q) == Some(b'#'))
        || (l.at(q) == Some(b':') && (l.is(q + 1, is_space) || l.eol(q + 1)))
}

/// And in them: also `\s*:[\[\]{},]` and `\s*[\[\]{},]`.
pub(crate) fn in_end_at(l: &Line, p: usize) -> bool {
    out_end_at(l, p) || {
        let q = ws(l, p);
        l.is(q, is_flow) || (l.at(q) == Some(b':') && l.is(q + 1, is_flow))
    }
}

/// `(YAML)[ \t]+(\d+\.\d+)`
pub(crate) fn directive_yaml(l: &Line, from: usize) -> Option<M> {
    let digits = |p: usize| l.run(p, |c| c.is_ascii_digit());
    let mut at = from;
    while let Some(m) = l.lit(at, b"YAML") {
        let k = m.s as usize;
        let v = l.run(k + 4, is_blank);
        let d = digits(v);
        if v > k + 4 && d > v && l.at(d) == Some(b'.') && digits(d + 1) > d + 1 {
            return Some(
                M::new(k, digits(d + 1))
                    .g(1, k, k + 4)
                    .g(2, v, digits(d + 1)),
            );
        }
        at = k + 1;
    }
    None
}

/// `%[0-9A-Fa-f]{2}` at `p`.
fn percent_hex(l: &Line, p: usize) -> bool {
    l.at(p) == Some(b'%')
        && l.is(p + 1, |c| c.is_ascii_hexdigit())
        && l.is(p + 2, |c| c.is_ascii_hexdigit())
}

/// The end of a run of `%HH` or bytes in `set` from `p`.
fn uri_run(l: &Line, mut p: usize, set: &[u8]) -> usize {
    loop {
        if percent_hex(l, p) {
            p += 3;
        } else if l.is(p, |c| c.is_ascii_alphanumeric() || set.contains(&c)) {
            p += 1;
        } else {
            return p;
        }
    }
}

/// The URI characters of a tag prefix, beside letters and digits.
const URI: &[u8] = b"-#;/?:@&=+$,_.!~*'()[]";

/// A tag handle at `p`, `!(?:[0-9A-Za-z\-]*!)?`, its longest first: its ends.
fn handles(l: &Line, p: usize) -> Option<(usize, Option<usize>)> {
    if l.at(p) != Some(b'!') {
        return None;
    }
    let r = l.run(p + 1, |c| c.is_ascii_alphanumeric() || c == b'-');
    let long = (l.at(r) == Some(b'!')).then_some(r + 1);
    Some((p + 1, long))
}

/// `(TAG)(?:[ \t]+(!(?:[0-9A-Za-z\-]*!)?)(?:[ \t]+(URI))?)?`
pub(crate) fn directive_tag(l: &Line, from: usize) -> Option<M> {
    let k = l.lit(from, b"TAG")?.s as usize;
    let mut m = M::new(k, k + 3).g(1, k, k + 3);
    let h = l.run(k + 3, is_blank);
    if h == k + 3 {
        return Some(m);
    }
    let Some((short, long)) = handles(l, h) else {
        return Some(m);
    };
    let he = long.unwrap_or(short);
    m = m.g(2, h, he);
    m.e = he as u32;
    let u = l.run(he, is_blank);
    if u > he {
        // `!` and any URI characters, or a URI that doesn't start with `,![]{}`
        let e = if l.at(u) == Some(b'!') {
            Some(uri_run(l, u + 1, URI))
        } else if !l.is(u, |c| b",![]{}".contains(&c)) {
            Some(uri_run(l, u, URI)).filter(|&e| e > u)
        } else {
            None
        };
        if let Some(e) = e {
            m = m.g(3, u, e);
            m.e = e as u32;
        }
    }
    Some(m)
}

/// `(\w+)(?:[ \t]+(\w+)(?:[ \t]+(\w+))?)?`
pub(crate) fn directive_reserved(l: &Line, from: usize) -> Option<M> {
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80;
    let hay = l.b.get(from..)?;
    let k = from + hay.iter().position(|&c| word(c))?;
    let e1 = l.run(k, word);
    let mut m = M::new(k, e1).g(1, k, e1);
    let s2 = l.run(e1, is_blank);
    let e2 = l.run(s2, word);
    if s2 > e1 && e2 > s2 {
        m = m.g(2, s2, e2);
        m.e = e2 as u32;
        let s3 = l.run(e2, is_blank);
        let e3 = l.run(s3, word);
        if s3 > e2 && e3 > s3 {
            m = m.g(3, s3, e3);
            m.e = e3 as u32;
        }
    }
    Some(m)
}

/// The tag characters after a handle, beside letters and digits.
const TAG: &[u8] = b"-#;/?:@&=+$_.~*'()";

/// `(?:!<URI+>|HANDLE(?:%HH|TAG)+|!)(?=\ |\t|$)`
pub(crate) fn tag_handle(l: &Line, from: usize) -> Option<M> {
    let ahead = |e: usize| l.is(e, is_blank) || l.eol(e);
    let mut at = from;
    while let Some(p) = first(l, at, b'!') {
        if l.at(p + 1) == Some(b'<') {
            let e = uri_run(l, p + 2, URI);
            if e > p + 2 && l.at(e) == Some(b'>') && ahead(e + 1) {
                return Some(M::new(p, e + 1));
            }
        }
        if let Some((short, long)) = handles(l, p) {
            for h in long.into_iter().chain([short]) {
                let e = uri_run(l, h, TAG);
                if e > h && ahead(e) {
                    return Some(M::new(p, e));
                }
            }
        }
        if ahead(p + 1) {
            return Some(M::new(p, p + 1));
        }
        at = p + 1;
    }
    None
}

/// `\d` at `i`.
fn digit(l: &Line, i: usize) -> bool {
    l.is(i, |c| c.is_ascii_digit())
}

/// The end of a run of bytes in `set` from `p`, at least `min` long.
fn run_of(l: &Line, p: usize, min: usize, set: impl Fn(u8) -> bool) -> Option<usize> {
    let e = l.run(p, set);
    (e >= p + min).then_some(e)
}

/// The implicit types' regex at `p`, its alternatives in order, each end tried against the
/// look-ahead `end` (bat's two variants differ only there). Only the timestamp's trailing
/// blanks and zone can end a match early to meet it; elsewhere a shorter end is followed by
/// a digit, a letter, `_` or `.`, which the look-ahead never takes, so the greedy end is the
/// only one.
pub(crate) fn implicit_at(l: &Line, p: usize, end: fn(&Line, usize) -> bool) -> Option<M> {
    let ok = |e: usize| end(l, e);
    let words = |words: &[&[u8]]| {
        words
            .iter()
            .map(|w| p + w.len())
            .zip(words)
            .find(|&(e, w)| l.b.get(p..e) == Some(*w) && ok(e))
            .map(|(e, _)| e)
    };
    if let Some(e) = words(&[b"null", b"Null", b"NULL", b"~"]) {
        return Some(M::new(p, e).g(1, p, e));
    }
    let bools: [&[u8]; 22] = [
        b"y", b"Y", b"yes", b"Yes", b"YES", b"n", b"N", b"no", b"No", b"NO", b"true", b"True",
        b"TRUE", b"false", b"False", b"FALSE", b"on", b"On", b"ON", b"off", b"Off", b"OFF",
    ];
    if let Some(e) = words(&bools) {
        return Some(M::new(p, e).g(2, p, e));
    }
    let q = p + usize::from(l.is(p, |c| c == b'+' || c == b'-'));
    let int = |m: M| Some(m.g(3, p, m.e as usize));
    // `(0b)[0-1_]+`, `(0)[0-7_]+`
    if l.b.get(q..q + 2) == Some(b"0b")
        && let Some(e) = run_of(l, q + 2, 1, |c| matches!(c, b'0' | b'1' | b'_')).filter(|&e| ok(e))
    {
        return int(M::new(p, e).g(4, p, e).g(5, q, q + 2));
    }
    if l.at(q) == Some(b'0')
        && let Some(e) = run_of(l, q + 1, 1, |c| matches!(c, b'0'..=b'7' | b'_')).filter(|&e| ok(e))
    {
        return int(M::new(p, e).g(6, p, e).g(7, q, q + 1));
    }
    // `(?:0|[1-9][0-9_]*)`
    let decimal = match l.at(q) {
        Some(b'0') => Some(q + 1),
        Some(b'1'..=b'9') => Some(l.run(q + 1, |c| c.is_ascii_digit() || c == b'_')),
        _ => None,
    };
    if let Some(e) = decimal.filter(|&e| ok(e)) {
        return int(M::new(p, e).g(8, p, e));
    }
    if l.b.get(q..q + 2) == Some(b"0x")
        && let Some(e) =
            run_of(l, q + 2, 1, |c| c.is_ascii_hexdigit() || c == b'_').filter(|&e| ok(e))
    {
        return int(M::new(p, e).g(9, p, e).g(10, q, q + 2));
    }
    // `[1-9][0-9_]*(?::[0-5]?[0-9])+`
    let sexagesimal = |from: usize| {
        let mut e = from;
        while l.at(e) == Some(b':') {
            let n = match (l.is(e + 1, |c| (b'0'..=b'5').contains(&c)), digit(l, e + 2)) {
                (true, true) => 3,
                _ if digit(l, e + 1) => 2,
                _ => break,
            };
            e += n;
        }
        (e > from).then_some(e)
    };
    if l.is(q, |c| (b'1'..=b'9').contains(&c))
        && let Some(e) = sexagesimal(l.run(q + 1, |c| c.is_ascii_digit() || c == b'_'))
        && ok(e)
    {
        return int(M::new(p, e).g(11, p, e));
    }
    let float = |m: M| Some(m.g(12, p, m.e as usize));
    // `(?:[0-9][0-9_]*)?(\.)[0-9.]*(?:[eE][-+][0-9]+)?`
    let dot = match digit(l, q) {
        true => l.run(q, |c| c.is_ascii_digit() || c == b'_'),
        false => q,
    };
    if l.at(dot) == Some(b'.') {
        let mut e = l.run(dot + 1, |c| c.is_ascii_digit() || c == b'.');
        if l.is(e, |c| c == b'e' || c == b'E')
            && l.is(e + 1, |c| c == b'-' || c == b'+')
            && digit(l, e + 2)
        {
            e = l.run(e + 2, |c| c.is_ascii_digit());
        }
        if ok(e) {
            return float(M::new(p, e).g(13, p, e).g(14, dot, dot + 1));
        }
    }
    // `[0-9][0-9_]*(?::[0-5]?[0-9])+(\.)[0-9_]*`
    if digit(l, q)
        && let Some(d) = sexagesimal(l.run(q + 1, |c| c.is_ascii_digit() || c == b'_'))
        && l.at(d) == Some(b'.')
    {
        let e = l.run(d + 1, |c| c.is_ascii_digit() || c == b'_');
        if ok(e) {
            return float(M::new(p, e).g(15, p, e).g(16, d, d + 1));
        }
    }
    // `(\.)(?:inf|Inf|INF)`, then `(\.)(?:nan|NaN|NAN)` without a sign
    if l.at(q) == Some(b'.')
        && [&b"inf"[..], b"Inf", b"INF"].contains(&l.b.get(q + 1..q + 4).unwrap_or_default())
        && ok(q + 4)
    {
        return float(M::new(p, q + 4).g(17, p, q + 4).g(18, q, q + 1));
    }
    if l.at(p) == Some(b'.')
        && [&b"nan"[..], b"NaN", b"NAN"].contains(&l.b.get(p + 1..p + 4).unwrap_or_default())
        && ok(p + 4)
    {
        return float(M::new(p, p + 4).g(19, p, p + 4).g(20, p, p + 1));
    }
    if let Some(m) = timestamp(l, p, ok) {
        return Some(m);
    }
    if l.at(p) == Some(b'=') && ok(p + 1) {
        return Some(M::new(p, p + 1).g(30, p, p + 1));
    }
    if l.b.get(p..p + 2) == Some(b"<<") && ok(p + 2) {
        return Some(M::new(p, p + 2).g(31, p, p + 2));
    }
    None
}

/// The timestamp alternative at `p`: a date, or a date and a time with a fraction, blanks
/// and a zone, whose blanks and zone give back what the look-ahead `ok` needs.
fn timestamp(l: &Line, p: usize, ok: impl Fn(usize) -> bool) -> Option<M> {
    if !(p..p + 4).all(|i| digit(l, i)) || l.at(p + 4) != Some(b'-') {
        return None;
    }
    let stamp = |m: M| Some(m.g(21, p, m.e as usize));
    // `\d{4}(-)\d{2}(-)\d{2}`
    if digit(l, p + 5)
        && digit(l, p + 6)
        && l.at(p + 7) == Some(b'-')
        && digit(l, p + 8)
        && digit(l, p + 9)
        && ok(p + 10)
    {
        return stamp(M::new(p, p + 10).g(22, p + 4, p + 5).g(23, p + 7, p + 8));
    }
    // `\d{1,2}` at `at`, as many as there are: its end
    let one_two = |at: usize| digit(l, at).then(|| at + 1 + usize::from(digit(l, at + 1)));
    let month = one_two(p + 5)?;
    if l.at(month) != Some(b'-') {
        return None;
    }
    let day = one_two(month + 1)?;
    let sep = match l.at(day)? {
        b'T' | b't' => day + 1,
        c if is_blank(c) => l.run(day, is_blank),
        _ => return None,
    };
    let hour = one_two(sep)?;
    let two = |at: usize| digit(l, at) && digit(l, at + 1);
    if !(l.at(hour) == Some(b':') && two(hour + 1) && l.at(hour + 3) == Some(b':') && two(hour + 4))
    {
        return None;
    }
    let mut m = M::new(p, p)
        .g(24, p + 4, p + 5)
        .g(25, month, month + 1)
        .g(26, hour, hour + 1)
        .g(27, hour + 3, hour + 4);
    let mut e = hour + 6;
    if l.at(e) == Some(b'.') {
        m = m.g(28, e, e + 1);
        e = l.run(e + 1, |c| c.is_ascii_digit());
    }
    // the blanks, most first, and the zone's choices after them, as they backtrack
    let blanks = l.run(e, is_blank);
    for b in (e..=blanks).rev() {
        let mut zones: Vec<(usize, Option<usize>)> = Vec::new();
        match l.at(b) {
            Some(b'Z') => zones.push((b + 1, None)),
            Some(b'+' | b'-') => {
                for h in [2, 1] {
                    if (1..=h).all(|i| digit(l, b + i)) {
                        let c = b + 1 + h;
                        if l.at(c) == Some(b':') {
                            for mm in [2, 1] {
                                if (1..=mm).all(|i| digit(l, c + i)) {
                                    zones.push((c + 1 + mm, Some(c)));
                                }
                            }
                        }
                        zones.push((c, None));
                    }
                }
            }
            _ => {}
        }
        zones.push((b, None));
        for (end, colon) in zones {
            if ok(end) {
                let mut m = match colon {
                    Some(c) => m.g(29, c, c + 1),
                    None => m,
                };
                m.e = end as u32;
                return stamp(m);
            }
        }
    }
    None
}
