//! Obsidian's comments (OBS-004): `%%…%%` is hidden in its Reading view, inline or over many
//! lines, the markdown inside too, and an unclosed `%%` hides the rest of the note. Only in
//! Obsidian's dialect (ADR-45): `%%` is a literal percent sign in printf, SQL and batch files.
//!
//! The comments go before the note is parsed, so what's inside one makes no blocks: their bytes
//! are left out, but for their line ends, so every line keeps its number, and the `>`s of a quote
//! the comment opened in, so the quote goes on past it. A `%%` in code (a
//! code span or block, math, front matter) opens none: those are found by a pass of
//! pulldown-cmark over the note as written. Inside a comment, the next `%%` closes it wherever it
//! is, as Obsidian has it: a comment around a code block hides the block.

use std::borrow::Cow;
use std::ops::Range;

use memchr::memmem;
use pulldown_cmark::{Event, Parser, Tag};

/// `text` without its comments, or `None` once its code pass makes more than `max` events. Out
/// of line: it runs once a note, beside the parse's own loop.
#[inline(never)]
pub fn strip(text: &str, max: usize) -> Option<Cow<'_, str>> {
    let finder = memmem::Finder::new(b"%%");
    if finder.find(text.as_bytes()).is_none() {
        return Some(Cow::Borrowed(text));
    }
    let code = code(text, max)?;
    let comments = comments(text, &code, &finder);
    if comments.is_empty() {
        return Some(Cow::Borrowed(text));
    }
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(text.len());
    let mut from = 0;
    // the start of the line the last comment opened on, how far it was looked for, and the
    // line's quote depth: each comment looks back only to the last one's start, not to its
    // line's, and reads the depth once a line (a line's first `%%` ends its `>`s, so later
    // comments on it find the same), which on a long line of comments was quadratic (PAR-013)
    let (mut line, mut seen, mut depth) = (0, 0, None);
    for c in comments {
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
    String::from_utf8(out).ok().map(Cow::Owned)
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

/// Where `text` has code that a `%%` can't open a comment in, in source order: its front matter
/// (as the parse finds it, a blank line after the `---` too, OBS-011), then what a pass of
/// pulldown-cmark over the rest finds.
fn code(text: &str, max: usize) -> Option<Vec<Range<usize>>> {
    let front = super::front::find(text, true).unwrap_or(0);
    let mut code = Vec::new();
    code.push(0..front);
    for (n, (event, range)) in Parser::new_ext(&text[front..], super::parse::options())
        .into_offset_iter()
        .enumerate()
    {
        if n == max {
            return None;
        }
        let range = range.start + front..range.end + front;
        match event {
            Event::Start(Tag::CodeBlock(_))
            | Event::Code(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_) => code.push(range),
            _ => {}
        }
    }
    code.sort_unstable_by_key(|r| r.start);
    Some(code)
}

/// The comments in `text`, each from its opening `%%` to past its closing one, or to the end.
/// An inline comment with a space on both sides takes the one after with it, so that `an
/// %%inline%% comment` reads with one space, as Obsidian's HTML does.
fn comments(text: &str, code: &[Range<usize>], finder: &memmem::Finder<'_>) -> Vec<Range<usize>> {
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

    fn stripped(text: &str) -> String {
        strip(text, usize::MAX).unwrap().into_owned()
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
        assert!(matches!(
            strip("no comments, 50%", 100),
            Some(Cow::Borrowed(_))
        ));
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
        assert!(matches!(strip(text, 100), Some(Cow::Borrowed(_))));
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
        assert!(strip("%% *a* *b*", 3).is_none());
    }
}
