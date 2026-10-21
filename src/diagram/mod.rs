//! Mermaid diagrams drawn as text (ADR-35): a ```` ```mermaid ```` block is parsed once with its
//! Doc and drawn for each layout onto a [`Canvas`], in the theme's `block.diagram_*` styles. A
//! kind ada doesn't draw, a diagram that doesn't parse or one past the limits shows as its
//! source, highlighted like any code block.

mod canvas;
pub mod graph;
pub mod sequence;

use unicode_width::UnicodeWidthChar;

pub use canvas::Canvas;

use crate::theme::Glyphs;

/// A parsed diagram, width-independent.
#[derive(Debug)]
pub enum Diagram {
    Sequence(sequence::Sequence),
    /// A flowchart or a state diagram.
    Graph(graph::Graph),
}

/// The most cells a drawn diagram may have; a bigger one shows as source.
const MAX_CELLS: usize = 2_000_000;

/// The most statements a diagram may have.
const MAX_STATEMENTS: usize = 2_000;

/// Parse a mermaid block's text. `None` for the kinds ada doesn't draw and for anything that
/// doesn't parse.
pub fn parse(src: &str) -> Option<Diagram> {
    let mut lines = statements(src);
    let (_, header) = lines.next()?;
    let kind = header.split_whitespace().next()?;
    match kind {
        "sequenceDiagram" => sequence::parse(lines).map(Diagram::Sequence),
        "flowchart" | "flowchart-elk" | "graph" | "stateDiagram" | "stateDiagram-v2" => {
            graph::parse(kind, header, lines).map(Diagram::Graph)
        }
        _ => None,
    }
}

/// A diagram drawn at one width.
#[derive(Debug)]
pub struct Drawn {
    pub canvas: Canvas,
    /// The rows that stay on screen while the rest scrolls under them (a sequence diagram's
    /// participants), if any.
    pub header: Option<std::ops::Range<usize>>,
    /// The full text of the labels cut to fit, in marker order: `…¹` is `notes[0]`.
    pub notes: Vec<String>,
}

/// Draw `diagram` to fit `width` columns where it can. `None` when it would be too big.
pub fn draw(diagram: &Diagram, width: u16, glyphs: &Glyphs) -> Option<Drawn> {
    let ascii = glyphs.table_border == crate::theme::TableBorder::Ascii;
    let drawn = match diagram {
        Diagram::Sequence(s) => sequence::draw(s, usize::from(width), ascii)?,
        Diagram::Graph(g) => graph::draw(g, usize::from(width), ascii)?,
    };
    (drawn.canvas.cells() <= MAX_CELLS).then_some(drawn)
}

/// A diagram's statements with their 0-based line in the block: trimmed, blank lines,
/// `%%` comments and directives and a front matter block left out, a trailing `;` dropped.
fn statements(src: &str) -> impl Iterator<Item = (u32, &str)> {
    let mut front = false;
    src.lines()
        .enumerate()
        .map(|(i, l)| (i as u32, l.trim()))
        .filter(move |&(i, l)| {
            if l == "---" && (i == 0 || front) {
                front = !front;
                return false;
            }
            !front && !l.is_empty() && !l.starts_with("%%")
        })
        .map(|(i, l)| (i, l.strip_suffix(';').unwrap_or(l).trim_end()))
        .take(MAX_STATEMENTS + 1)
}

/// A label's lines: split at `<br>`, `<br/>` and `<br />`, any case, with `#59;`-style
/// entities decoded and control characters as spaces.
pub fn label_lines(text: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(after) = br(rest) {
            out.push(String::new());
            rest = after;
            continue;
        }
        let (c, after) = entity(rest).unwrap_or_else(|| {
            let mut chars = rest.chars();
            (chars.next().unwrap_or(' '), chars.as_str())
        });
        if let Some(line) = out.last_mut() {
            push_char(line, c);
        }
        rest = after;
    }
    out.iter().map(|l| l.trim().to_owned()).collect()
}

fn push_char(line: &mut String, c: char) {
    line.push(if c.is_control() { ' ' } else { c });
}

/// What follows a `<br>` tag at the start of `s`.
fn br(s: &str) -> Option<&str> {
    let head = s.get(..3)?;
    if !head.eq_ignore_ascii_case("<br") {
        return None;
    }
    let rest = s[3..].trim_start_matches(' ');
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    rest.strip_prefix('>')
}

/// A `#59;` entity at the start of `s`: the character and what follows.
fn entity(s: &str) -> Option<(char, &str)> {
    let body = s.strip_prefix('#')?;
    let end = body.find(';')?;
    let digits = &body[..end];
    if digits.is_empty() || digits.len() > 7 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let c = char::from_u32(digits.parse().ok()?)?;
    Some((c, &body[end + 1..]))
}

/// Columns `s` takes, as the canvas counts them.
pub fn width(s: &str) -> usize {
    s.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// `s` cut to at most `max` columns.
fn cut(s: &str, max: usize) -> &str {
    let mut used = 0;
    for (i, c) in s.char_indices() {
        used += c.width().unwrap_or(0);
        if used > max {
            return &s[..i];
        }
    }
    s
}

/// Wrap `text` into lines of at most `max` columns, at spaces where it can.
fn wrap(text: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    wrap_each(text, max, &mut |line, _| out.push(line.to_owned()));
    out
}

/// The rows `text` wraps to at `max` columns, and the widest of them, for the layout's search,
/// which measures every label several times.
fn wrap_size(text: &str, max: usize) -> (usize, usize) {
    let (mut rows, mut widest) = (0, 0);
    wrap_each(text, max, &mut |_, w| {
        rows += 1;
        widest = widest.max(w);
    });
    (rows, widest)
}

/// [`wrap`], a line at a time: each a slice of `text`, and its width. Lines break at spaces, which go, and
/// after `(`, `{`, `[`, `/`, `-` and `,`, which stay; a piece longer than a line breaks where
/// it must.
fn wrap_each<'a>(text: &'a str, max: usize, emit: &mut impl FnMut(&'a str, usize)) {
    let max = max.max(1);
    // the line being filled: where it starts, ends and how wide it is
    let (mut start, mut end, mut used) = (None, 0, 0);
    let mut emitted = false;
    for (at, piece) in pieces(text) {
        let (mut at, mut piece) = (at, piece);
        loop {
            let gap = start.map_or(0, |_| at - end);
            let w = width(piece);
            if used + gap + w <= max {
                start = start.or(Some(at));
                (end, used) = (at + piece.len(), used + gap + w);
                break;
            }
            if let Some(s) = start.take() {
                emit(&text[s..end], used);
                (emitted, used) = (true, 0);
                continue;
            }
            let head = break_word(piece, max);
            emit(head, width(head));
            emitted = true;
            (at, piece) = (at + head.len(), &piece[head.len()..]);
            if piece.is_empty() {
                break;
            }
        }
    }
    match start {
        Some(s) => emit(&text[s..end], used),
        None if !emitted => emit("", 0),
        None => {}
    }
}

/// The unbreakable pieces of `text`, with their byte offsets: runs between spaces, split
/// again after the characters a line may break after.
fn pieces(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut pos = 0;
    std::iter::from_fn(move || {
        let rest = &text[pos..];
        let start = pos + (rest.len() - rest.trim_start_matches(' ').len());
        let body = text.get(start..).filter(|b| !b.is_empty())?;
        let len = body
            .char_indices()
            .find_map(|(i, c)| match c {
                ' ' => Some(i),
                // an opening bracket after a space stays with what it opens
                '(' | '{' | '[' if i == 0 => None,
                '(' | '{' | '[' | '/' | '-' | ',' => Some(i + c.len_utf8()),
                _ => None,
            })
            .unwrap_or(body.len());
        pos = start + len;
        Some((start, &body[..len]))
    })
}

/// The first line's worth of a piece too long for a line: at least one character.
fn break_word(word: &str, max: usize) -> &str {
    let head = cut(word, max);
    if head.is_empty() {
        word.chars().next().map_or(word, |c| &word[..c.len_utf8()])
    } else {
        head
    }
}

/// A cut label's marker: `…` and [`note_mark`].
fn marker(n: usize) -> String {
    format!("…{}", note_mark(n))
}

/// The number of the note that lists a cut label, in superscript digits.
pub fn note_mark(n: usize) -> String {
    const SUP: [char; 10] = ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];
    n.to_string()
        .bytes()
        .map(|b| SUP[usize::from(b - b'0')])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_break_at_br_tags_and_decode_entities() {
        assert_eq!(label_lines("a<br/>b<BR>c<br />d"), ["a", "b", "c", "d"]);
        assert_eq!(label_lines("x #59; y"), ["x ; y"]);
        assert_eq!(label_lines("List<WrappingKey>"), ["List<WrappingKey>"]);
        assert_eq!(label_lines("#notanentity;"), ["#notanentity;"]);
        assert_eq!(label_lines(""), [""]);
    }

    #[test]
    fn statements_skip_comments_and_front_matter() {
        let src = "---\ntitle: x\n---\nsequenceDiagram\n  %% note\n\n  A->>B: hi;\n";
        let got: Vec<_> = statements(src).collect();
        assert_eq!(got, [(3, "sequenceDiagram"), (6, "A->>B: hi")]);
    }

    #[test]
    fn wrapping_at_spaces_then_inside_words() {
        assert_eq!(wrap("one two three", 7), ["one two", "three"]);
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("", 4), [""]);
        assert_eq!(wrap("日本語", 1), ["日", "本", "語"]);
    }

    #[test]
    fn wrapping_after_brackets_and_separators() {
        assert_eq!(
            wrap("PayloadRequest(public key)", 16),
            ["PayloadRequest(", "public key)"]
        );
        assert_eq!(wrap("a/b/c-d", 4), ["a/b/", "c-d"]);
        assert_eq!(wrap("x,  y", 3), ["x,", "y"]);
        assert_eq!(wrap("payload (mTLS)", 10), ["payload", "(mTLS)"]);
        assert_eq!(wrap_size("PayloadRequest(public key)", 16), (2, 15));
    }

    #[test]
    fn markers_count_in_superscript() {
        assert_eq!(marker(1), "…¹");
        assert_eq!(marker(12), "…¹²");
    }

    #[test]
    fn unknown_kinds_are_left_as_source() {
        assert!(parse("pie\n \"a\": 1").is_none());
        assert!(parse("classDiagram\n class A").is_none());
        assert!(parse("").is_none());
        assert!(parse("%% only a comment").is_none());
    }
}
