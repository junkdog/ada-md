//! Obsidian's comments (OBS-004): `%%…%%` is hidden in its Reading view, inline or over many
//! lines, the markdown inside too, and an unclosed `%%` hides the rest of the note. Only in
//! Obsidian's dialect (ADR-45): `%%` is a literal percent sign in printf, SQL and batch files.
//!
//! The comments go before the note is parsed, so what's inside one makes no blocks: their bytes
//! are left out, but for their line ends, so every line keeps its number, and the `>`s of a quote
//! the comment opened in, so the quote goes on past it. A `%%` in code (a code span or block,
//! math, front matter) opens none: code as pulldown-cmark reads the note as written, with the
//! `$`s the parse masks before its first pass masked ([`super::parse`]). A note whose `%%`s all
//! look like code ([`look_like_code`]) is parsed as written, its first pass finding its code, and
//! again without its comments only if that finds one; any other note's code is found by a pass of
//! pulldown-cmark of its own ([`code`]) before the parse (PAR-021). Inside a comment, the next
//! `%%` closes it wherever it is, as Obsidian has it: a comment around a code block hides the
//! block.

use std::ops::Range;

use memchr::memmem;
use pulldown_cmark::{Event, Tag};

use super::parse::Probe;

/// Whether `text` holds a `%%`: one `memmem` for a note without.
pub fn any(text: &str) -> bool {
    memmem::find(text.as_bytes(), b"%%").is_some()
}

/// `text` without `comments` ([`find`]'s, of which there is one or more). Out of line: it runs
/// once a note, beside the parse's own loop.
#[inline(never)]
pub fn strip(text: &str, comments: &[Range<usize>]) -> Option<String> {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(text.len());
    let mut from = 0;
    // the start of the line the last comment opened on, how far it was looked for, and the
    // line's quote depth: each comment looks back only to the last one's start, not to its
    // line's, and reads the depth once a line (a line's first `%%` ends its `>`s, so later
    // comments on it find the same), which on a long line of comments was quadratic (PAR-013)
    let (mut line, mut seen, mut depth) = (0, 0, None);
    for c in comments.iter().cloned() {
        out.extend_from_slice(&b[from..c.start]);
        // a comment over lines of a quote keeps their `>`s, as deep as on its first line, so the
        // quote (a callout, an Annotator note's) doesn't end with it
        if let Some(nl) = memchr::memrchr(b'\n', &b[seen..c.start]) {
            line = seen + nl + 1;
            depth = None;
        }
        seen = c.start;
        let depth = *depth.get_or_insert_with(|| quote_prefix(&b[line..c.start], usize::MAX).1);
        let mut at = c.start;
        while let Some(nl) = memchr::memchr2(b'\n', b'\r', &b[at..c.end]) {
            let nl = at + nl;
            out.push(b[nl]);
            at = nl + 1;
            if b[nl] == b'\n' || b.get(at) != Some(&b'\n') {
                let keep = quote_prefix(&b[at..c.end], depth).0;
                out.extend_from_slice(&b[at..at + keep]);
                at += keep;
            }
        }
        from = c.end;
    }
    out.extend_from_slice(&b[from..]);
    // only whole ASCII `%%…%%` (or a comment running to the end), the spaces after one and the
    // text of lines in one are left out, each up to a newline or an ASCII `>` or space
    String::from_utf8(out).ok()
}

/// Whether every `%%` in `text` past `from` looks like it's in code, so that the note is likely
/// to hold no comment: after an odd number of backtick runs on its line (a code span), on a line
/// indented four columns or more (an indented block), or between fence lines (a fenced block).
/// A guess, which only picks how the note's code is found: a note that looks so is parsed as
/// written, its first pass finding its code and its comments, and parsed again without them if
/// it holds one; any other has its code found by [`code`] first, a pass of pulldown-cmark about
/// half a parse's cost (PAR-021). Linear in the text: the backticks and tildes before the last
/// `%%` are read once.
pub fn look_like_code(text: &str, from: usize) -> bool {
    let b = text.as_bytes();
    // the open fence's byte and length, if any, and how far fences are read
    let mut fence: Option<(u8, usize)> = None;
    let mut read = from;
    for at in memmem::find_iter(&b[from..], b"%%").map(|i| i + from) {
        let line = memchr::memrchr(b'\n', &b[..at]).map_or(0, |i| i + 1);
        // the lines before the `%%` with a backtick or tilde, each once: a fence's only at the
        // line's start, so a line is read up to its first (and the `%%`'s line, if that's before
        // it, whole)
        while read < at {
            match memchr::memchr2(b'`', b'~', &b[read..at]) {
                Some(i) => {
                    let start = read + i;
                    read = next_line(b, start);
                    fence = fence_line(b, start, fence);
                }
                None => read = at,
            }
        }
        let ticks = memchr::memchr_iter(b'`', &b[line..at])
            .filter(|&i| i == 0 || b[line + i - 1] != b'`')
            .count();
        let indent = b[line..at]
            .iter()
            .take_while(|&&c| matches!(c, b' ' | b'\t'))
            .map(|&c| if c == b'\t' { 4 } else { 1 })
            .sum::<usize>();
        if fence.is_none() && ticks % 2 == 0 && indent < 4 {
            return false;
        }
    }
    true
}

/// Past the line end after `at`, or the text's end.
fn next_line(b: &[u8], at: usize) -> usize {
    memchr::memchr(b'\n', &b[at..]).map_or(b.len(), |i| at + i + 1)
}

/// The fence open after the line holding the backtick or tilde at `at`, given the one open
/// before (`open`): a run of three or more at the line's start (past spaces and quotes' `>`s)
/// opens one when none is, and a run of its byte as long or longer, alone on its line, closes it.
fn fence_line(b: &[u8], at: usize, open: Option<(u8, usize)>) -> Option<(u8, usize)> {
    let line = memchr::memrchr(b'\n', &b[..at]).map_or(0, |i| i + 1);
    if !b[line..at]
        .iter()
        .all(|&c| matches!(c, b' ' | b'\t' | b'>'))
    {
        return open;
    }
    let c = b[at];
    let run = b[at..].iter().take_while(|&&x| x == c).count();
    if run < 3 {
        return open;
    }
    match open {
        None => Some((c, run)),
        Some((o, len)) if o == c && run >= len => {
            let end = next_line(b, at);
            match b[at + run..end].iter().all(u8::is_ascii_whitespace) {
                true => None,
                false => open,
            }
        }
        Some(_) => open,
    }
}

/// The quote markers `line` starts with, up to `depth` of them: their length (the spaces before
/// each and the `>`) and how many there are.
fn quote_prefix(line: &[u8], depth: usize) -> (usize, usize) {
    let (mut len, mut n, mut at) = (0, 0, 0);
    while n < depth {
        while matches!(line.get(at), Some(b' ' | b'\t')) {
            at += 1;
        }
        if line.get(at) != Some(&b'>') {
            break;
        }
        at += 1;
        n += 1;
        len = at;
    }
    (len, n)
}

/// Where `source` (a note as written, its first pass's `$`s masked) has code that a `%%` can't
/// open a comment in, in source order, or `None` past `max` events: its front matter, up to
/// `front` (as the parse finds it, a blank line after the `---` too, OBS-011), then what a pass
/// of pulldown-cmark over the rest finds, read by the parse's own loop into a builder that builds
/// nothing ([`super::parse::probe`]: the loop stays the one reader of pulldown-cmark's events,
/// PAR-023). Out of line: it runs once a note, beside the parse.
#[inline(never)]
pub fn code(source: &str, front: usize, max: usize) -> Option<Vec<Range<usize>>> {
    // the front matter first
    let start = Probe::Code(std::iter::once(0..front).collect());
    let (Probe::Code(mut code), true) = super::parse::probe(source, front, max, start) else {
        return None;
    };
    code.sort_unstable_by_key(|r| r.start);
    Some(code)
}

/// Whether `event` is code a `%%` opens no comment in, for [`code`] and the parse's first pass
/// of a note it finds the code of (PAR-021).
pub fn is_code(event: &Event<'_>) -> bool {
    matches!(
        event,
        Event::Start(Tag::CodeBlock(_))
            | Event::Code(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_)
    )
}

/// The comments in `text`, whose code is `code` (in order of their starts), each from its
/// opening `%%` to past its closing one, or to the end. An inline comment with a space on both
/// sides takes the one after with it, so that `an %%inline%% comment` reads with one space, as
/// Obsidian's HTML does.
pub fn find(text: &str, code: &[Range<usize>]) -> Vec<Range<usize>> {
    let finder = memmem::Finder::new(b"%%");
    let b = text.as_bytes();
    let mut comments = Vec::new();
    let mut code = code.iter().peekable();
    let mut at = 0;
    while let Some(found) = finder.find(&b[at..]) {
        let open = at + found;
        while code.next_if(|r| r.end <= open).is_some() {}
        if let Some(r) = code.peek()
            && r.start <= open
        {
            at = r.end;
            continue;
        }
        let mut end = finder
            .find(&b[open + 2..])
            .map_or(b.len(), |close| open + 2 + close + 2);
        let spaced = |c: Option<&u8>| matches!(c, Some(b' ' | b'\t'));
        if spaced(open.checked_sub(1).map(|i| &b[i])) && spaced(b.get(end)) {
            end += 1;
        }
        comments.push(open..end);
        at = end;
    }
    comments
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text`'s comments, found as a note's that doesn't look like code is: its code by a pass
    /// of its own, its `$`s as written. `None` past `max` events.
    fn comments(text: &str, max: usize) -> Option<Vec<Range<usize>>> {
        let front = super::super::front::find(text, true).unwrap_or(0);
        Some(find(text, &code(text, front, max)?))
    }

    fn stripped(text: &str) -> String {
        let comments = comments(text, usize::MAX).unwrap();
        match comments.is_empty() {
            true => text.to_string(),
            false => strip(text, &comments).unwrap(),
        }
    }

    #[test]
    fn comments_go_and_their_lines_stay() {
        assert_eq!(stripped("an %%inline%% comment"), "an comment");
        assert_eq!(stripped("a%%b%%c %%d%%"), "ac ");
        assert_eq!(
            stripped("a\n%%\nhidden\n\n# too\n%%\nb\n"),
            "a\n\n\n\n\n\nb\n"
        );
        assert_eq!(stripped("a %%over\r\nlines%% b"), "a \r\nb");
        // unclosed: the rest of the note
        assert_eq!(stripped("a\n%%\nb\nc\n"), "a\n\n\n\n");
        assert_eq!(stripped("100%% sure"), "100");
        assert_eq!(stripped("%%%%x"), "x");
        assert_eq!(stripped("%%%x%%y"), "y");
        assert!(!any("no comments, 50%"));
    }

    #[test]
    fn a_comment_in_a_quote_keeps_its_lines_quoted() {
        // an Annotator note's: one quote, its first lines empty
        assert_eq!(
            stripped(">%%\n>json\n> >x\n>%%\n>*a*\n"),
            ">\n>\n>\n>\n>*a*\n"
        );
        assert_eq!(
            stripped("> [!note]\n> %%a\r\n> b%% c"),
            "> [!note]\n> \r\n>c"
        );
        assert_eq!(stripped("> > %%\n> > a\n> %%\n"), "> > \n> >\n>\n");
        // a quote commented out goes whole
        assert_eq!(stripped("%%\n> q\n%%\nx"), "\n\n\nx");
    }

    #[test]
    fn code_opens_no_comment() {
        let text = "`printf(\"%%d\")` and $50\\%%$\n\n```sql\nLIKE '%%a'\n```\n\n    %%i\n";
        assert_eq!(comments(text, 100), Some(Vec::new()));
        assert_eq!(
            stripped("---\nx: '%%'\n---\n%%hidden%%\n"),
            "---\nx: '%%'\n---\n\n"
        );
        // inside a comment, code hides with it, and its `%%` closes it
        assert_eq!(stripped("%%\n```\n%%\n```\n"), "\n\n\n```\n");
        assert_eq!(stripped("%% `a` %% b `%%` c"), " b `%%` c");
    }

    /// A long line of comments, behind a deep quote's markers, strips in linear time (PAR-013:
    /// each comment looked back to its line's start, 3.8 s for 80,000 on one line).
    #[test]
    fn a_long_line_of_comments_is_linear() {
        let n = 100_000;
        let text = ">".repeat(1000) + &" %%a%%".repeat(n) + "\n";
        let start = std::time::Instant::now();
        let out = stripped(&text);
        assert_eq!(out, ">".repeat(1000) + " \n");
        assert!(start.elapsed().as_secs() < 5, "{:?}", start.elapsed());
    }

    #[test]
    fn too_many_events_is_none() {
        assert!(comments("%% *a* *b*", 3).is_none());
    }
}
