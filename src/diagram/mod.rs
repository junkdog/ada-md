//! Mermaid diagrams drawn as text (ADR-35): a ```` ```mermaid ```` block is parsed once with its
//! Doc and drawn for each layout onto a [`Canvas`], in the theme's `block.diagram_*` styles. A
//! kind ada doesn't draw, a diagram that doesn't parse or one past the limits shows as its
//! source, highlighted like any code block.

mod canvas;
pub mod gantt;
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
    Gantt(gantt::Gantt),
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
    let source = Source::of(src);
    match kind {
        "sequenceDiagram" => sequence::parse(lines, source).map(Diagram::Sequence),
        "flowchart" | "flowchart-elk" | "graph" | "stateDiagram" | "stateDiagram-v2" => {
            graph::parse(kind, header, lines, source).map(Diagram::Graph)
        }
        "gantt" => gantt::parse(lines, source).map(Diagram::Gantt),
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
    pub notes: Vec<Text>,
}

/// Draw `diagram` to fit `width` columns where it can. `None` when it would be too big.
pub fn draw(diagram: &Diagram, width: u16, glyphs: &Glyphs) -> Option<Drawn> {
    let ascii = glyphs.table_border == crate::theme::TableBorder::Ascii;
    let drawn = match diagram {
        Diagram::Sequence(s) => sequence::draw(s, usize::from(width), ascii)?,
        Diagram::Graph(g) => graph::draw(g, usize::from(width), ascii)?,
        Diagram::Gantt(g) => gantt::draw(g, usize::from(width), ascii)?,
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

/// A line of label text, and where it came from in the block's source when it is a slice of it,
/// so search can find it in the drawing. Equal by text alone.
#[derive(Clone, Debug, Default)]
pub struct Text {
    pub s: String,
    pub origin: Option<Origin>,
}

/// Where a [`Text`]'s first `len` bytes are in the block's source: from byte `at` on. A cut
/// label's marker is past `len`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Origin {
    pub at: u32,
    pub len: u32,
}

impl Text {
    /// Bytes `range` of it, keeping the part of its origin they cover.
    fn slice(&self, range: std::ops::Range<usize>) -> Text {
        let origin = self.origin.and_then(|o| {
            let len = (o.len as usize).checked_sub(range.start)?.min(range.len());
            (len > 0).then_some(Origin {
                at: o.at + range.start as u32,
                len: len as u32,
            })
        });
        Text {
            s: self.s[range].to_owned(),
            origin,
        }
    }

    /// `lines` on one line, a space between them; one line keeps its origin.
    pub fn join(lines: &[Text]) -> Text {
        match lines {
            [one] => one.clone(),
            lines => Text {
                s: lines.iter().map(|l| &*l.s).collect::<Vec<_>>().join(" "),
                origin: None,
            },
        }
    }

    /// It, then `tail`, which isn't from the source.
    fn with(mut self, tail: &str) -> Text {
        self.s.push_str(tail);
        self
    }
}

impl std::ops::Deref for Text {
    type Target = str;

    fn deref(&self) -> &str {
        &self.s
    }
}

impl std::fmt::Display for Text {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.s)
    }
}

impl PartialEq for Text {
    fn eq(&self, other: &Text) -> bool {
        self.s == other.s
    }
}

impl Eq for Text {}

impl PartialEq<&str> for Text {
    fn eq(&self, other: &&str) -> bool {
        self.s == *other
    }
}

impl From<&str> for Text {
    fn from(s: &str) -> Text {
        Text {
            s: s.to_owned(),
            origin: None,
        }
    }
}

/// A diagram's source, to tell where in it a label sliced from it starts.
#[derive(Clone, Copy, Debug, Default)]
pub struct Source {
    start: usize,
    len: usize,
}

impl Source {
    fn of(src: &str) -> Source {
        Source {
            start: src.as_ptr() as usize,
            len: src.len(),
        }
    }

    /// Where `s` starts in the source, when it is a slice of it.
    fn offset(self, s: &str) -> Option<u32> {
        let at = (s.as_ptr() as usize).checked_sub(self.start)?;
        (at + s.len() <= self.len).then(|| u32::try_from(at).ok())?
    }

    /// A label's lines: split at `<br>`, `<br/>` and `<br />`, any case, with `#59;`-style
    /// entities decoded and control characters as spaces. A line left as it was in `text`
    /// keeps its origin.
    pub fn label(self, text: &str) -> Vec<Text> {
        let mut out = Vec::new();
        let mut line = String::new();
        // where the line being read starts in `text`, and whether it is still as written
        let (mut from, mut verbatim) = (0, true);
        let mut rest = text;
        loop {
            let at = text.len() - rest.len();
            let after = br(rest);
            if after.is_some() || rest.is_empty() {
                out.push(self.line(&line, &text[from..at], verbatim));
                let Some(after) = after else {
                    return out;
                };
                line.clear();
                rest = after;
                (from, verbatim) = (text.len() - rest.len(), true);
                continue;
            }
            let (c, after) = match entity(rest) {
                Some(e) => {
                    verbatim = false;
                    e
                }
                None => {
                    let mut chars = rest.chars();
                    (chars.next().unwrap_or(' '), chars.as_str())
                }
            };
            verbatim &= !c.is_control();
            push_char(&mut line, c);
            rest = after;
        }
    }

    /// One line of a label, trimmed; `raw` is what it was read from.
    fn line(self, line: &str, raw: &str, verbatim: bool) -> Text {
        let s = line.trim();
        let lead = line.len() - line.trim_start().len();
        let origin = if verbatim {
            self.offset(raw).map(|at| Origin {
                at: at + lead as u32,
                len: s.len() as u32,
            })
        } else {
            None
        };
        Text {
            s: s.to_owned(),
            origin,
        }
    }

    /// A label on one line, its breaks as spaces.
    pub fn one_line(self, text: &str) -> Text {
        let s = text.trim();
        if !s.contains(['<', '#']) && !s.contains(char::is_control) {
            // no break or entity: as written (a gantt chart has a name per task)
            return Text {
                s: s.to_owned(),
                origin: self.offset(s).map(|at| Origin {
                    at,
                    len: s.len() as u32,
                }),
            };
        }
        Text::join(&self.label(s))
    }
}

/// A label's lines, as [`Source::label`] reads them, with no origin.
pub fn label_lines(text: &str) -> Vec<Text> {
    Source::default().label(text)
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
fn wrap(text: &Text, max: usize) -> Vec<Text> {
    let mut out = Vec::new();
    wrap_each(text, max, &mut |at, line, _| {
        out.push(text.slice(at..at + line.len()));
    });
    out
}

/// The rows `text` wraps to at `max` columns, and the widest of them, for the layout's search,
/// which measures every label several times.
fn wrap_size(text: &str, max: usize) -> (usize, usize) {
    let (mut rows, mut widest) = (0, 0);
    wrap_each(text, max, &mut |_, _, w| {
        rows += 1;
        widest = widest.max(w);
    });
    (rows, widest)
}

/// [`wrap`], a line at a time: where it starts in `text`, the slice, and its width. Lines break
/// at spaces, which go, and after `(`, `{`, `[`, `/`, `-` and `,`, which stay; a piece longer
/// than a line breaks where it must.
fn wrap_each<'a>(text: &'a str, max: usize, emit: &mut impl FnMut(usize, &'a str, usize)) {
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
                emit(s, &text[s..end], used);
                (emitted, used) = (true, 0);
                continue;
            }
            let head = break_word(piece, max);
            emit(at, head, width(head));
            emitted = true;
            (at, piece) = (at + head.len(), &piece[head.len()..]);
            if piece.is_empty() {
                break;
            }
        }
    }
    match start {
        Some(s) => emit(s, &text[s..end], used),
        None if !emitted => emit(0, "", 0),
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
        assert_eq!(wrap(&"one two three".into(), 7), ["one two", "three"]);
        assert_eq!(wrap(&"abcdefghij".into(), 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap(&"".into(), 4), [""]);
        assert_eq!(wrap(&"日本語".into(), 1), ["日", "本", "語"]);
    }

    #[test]
    fn wrapping_after_brackets_and_separators() {
        assert_eq!(
            wrap(&"PayloadRequest(public key)".into(), 16),
            ["PayloadRequest(", "public key)"]
        );
        assert_eq!(wrap(&"a/b/c-d".into(), 4), ["a/b/", "c-d"]);
        assert_eq!(wrap(&"x,  y".into(), 3), ["x,", "y"]);
        assert_eq!(wrap(&"payload (mTLS)".into(), 10), ["payload", "(mTLS)"]);
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
