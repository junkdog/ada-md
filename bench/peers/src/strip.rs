//! Corpus variants for pricing what's in comments and strings: `--strip comments` removes every
//! comment's text (`//` to the line's end, `/* … */` and Javadoc whole), `--strip
//! comments,strings` also a string's, a char's and a text block's contents, leaving the quotes.
//! Line ends are kept, so the lines are the same; done before the timing, for every engine. A
//! C-family reader (Java's and Rust's comments and strings; Rust's raw strings and nested block
//! comments aren't told apart, so Java only).

pub struct Strip {
    pub comments: bool,
    pub strings: bool,
}

pub fn strip(text: &str, s: &Strip) -> String {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    // keep only the line ends of `b[from..to]`
    let lines = |out: &mut Vec<u8>, from: usize, to: usize| {
        out.extend(b[from..to].iter().filter(|&&c| c == b'\n'));
    };
    while i < b.len() {
        let c = b[i];
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            let end = b[i..].iter().position(|&c| c == b'\n').map_or(b.len(), |n| i + n);
            if !s.comments {
                out.extend_from_slice(&b[i..end]);
            }
            i = end;
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let end = b[i + 2..].windows(2).position(|w| w == b"*/").map_or(b.len(), |n| i + 2 + n + 2);
            if s.comments { lines(&mut out, i, end) } else { out.extend_from_slice(&b[i..end]) }
            i = end;
        } else if b[i..].starts_with(b"\"\"\"") {
            let mut j = i + 3;
            while j < b.len() && !b[j..].starts_with(b"\"\"\"") {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            let end = (j + 3).min(b.len());
            if s.strings {
                out.extend_from_slice(b"\"\"\"");
                lines(&mut out, i + 3, j.min(b.len()));
                out.extend_from_slice(b"\"\"\"");
            } else {
                out.extend_from_slice(&b[i..end]);
            }
            i = end;
        } else if c == b'"' || c == b'\'' {
            let mut j = i + 1;
            while j < b.len() && b[j] != c && b[j] != b'\n' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            let end = if j < b.len() && b[j] == c { j + 1 } else { j.min(b.len()) };
            if s.strings {
                out.push(c);
                if end > i + 1 && b[end - 1] == c {
                    out.push(c);
                }
            } else {
                out.extend_from_slice(&b[i..end]);
            }
            i = end;
        } else {
            out.push(c);
            i += 1;
        }
    }
    // a cut inside a multi-byte character can't happen: every cut is at an ASCII byte
    String::from_utf8(out).expect("stripped text is UTF-8")
}
