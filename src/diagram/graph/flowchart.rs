//! The flowchart parser: `flowchart`/`graph` and a direction, then nodes in their shapes,
//! chains of edges (`A --> B -.-> C`, `A & B --> C`), labels on edges and subgraphs.
//! Styling statements are accepted and ignored: colors come from the theme.

use super::{
    Builder, Dir, Edge, End, Frame, Line, Mark, NodeId, Shape, label_lines, split_statements,
};
use crate::diagram::MAX_STATEMENTS;

/// Parse the statements after the `flowchart` line, `header`, which may hold statements of
/// its own after a `;`.
pub(super) fn parse<'a>(
    header: &'a str,
    lines: impl Iterator<Item = (u32, &'a str)>,
) -> Option<Builder> {
    let mut statements = split_statements(header);
    let mut words = statements.next()?.split_whitespace();
    words.next()?;
    let mut b = Builder {
        dir: words.next().map_or(Some(Dir::Down), Dir::parse)?,
        ..Builder::default()
    };
    let mut p = Parser {
        b: &mut b,
        in_description: false,
    };
    let rest = statements.map(|s| (0, s));
    for (n, (_, line)) in rest.chain(lines).enumerate() {
        if n >= MAX_STATEMENTS {
            return None;
        }
        for statement in split_statements(line) {
            p.statement(statement)?;
        }
    }
    Some(b)
}

struct Parser<'b> {
    b: &'b mut Builder,
    /// Inside a multi-line `accDescr { … }`.
    in_description: bool,
}

impl Parser<'_> {
    fn statement(&mut self, line: &str) -> Option<()> {
        if self.in_description {
            self.in_description = !line.contains('}');
            return Some(());
        }
        let word = line
            .split(|c: char| c.is_whitespace() || c == ':')
            .next()
            .unwrap_or("");
        let rest = line[word.len()..].trim();
        match word {
            "subgraph" => self.subgraph(rest),
            "end" => self.b.close().map(|_| ()),
            "direction" | "style" | "classDef" | "class" | "linkStyle" | "click" | "accTitle"
            | "callback" => Some(()),
            "accDescr" => {
                self.in_description = rest.starts_with('{') && !rest.contains('}');
                Some(())
            }
            _ => self.chain(line),
        }
    }

    /// `subgraph id`, `subgraph id [title]`, `subgraph "title"`, `subgraph some title`.
    fn subgraph(&mut self, rest: &str) -> Option<()> {
        let (id, title) = if let Some(q) = rest.strip_prefix('"') {
            let title = q.split('"').next().unwrap_or("");
            (title, title)
        } else {
            let id_len = rest.find(|c: char| !id_char(c)).unwrap_or(rest.len());
            let (id, after) = rest.split_at(id_len);
            let after = after.trim_start();
            match after.strip_prefix('[') {
                Some(t) => (id, unquote(t.strip_suffix(']').unwrap_or(t))),
                None if after.is_empty() => (id, id),
                None => (rest, rest),
            }
        };
        let id = if id.is_empty() { title } else { id };
        self.b.open(id, title, Frame::Square).map(|_| ())
    }

    /// `A --> B`, `A & B -- text --> C[shape] -.-> D`, or a lone node.
    fn chain(&mut self, line: &str) -> Option<()> {
        let mut s = line;
        let mut left = self.nodes(&mut s)?;
        loop {
            s = s.trim_start();
            if s.is_empty() {
                return Some(());
            }
            let arrow = arrow(&mut s)?;
            s = s.trim_start();
            let right = self.nodes(&mut s)?;
            for &a in &left {
                for &b in &right {
                    self.b.edge(Edge {
                        from: End::Node(a),
                        to: End::Node(b),
                        line: arrow.line,
                        head_from: arrow.from,
                        head_to: arrow.to,
                        label: arrow.label.clone(),
                        minlen: arrow.minlen,
                    })?;
                }
            }
            left = right;
        }
    }

    /// `A`, `A[label]`, `A & B:::cls & C`.
    fn nodes(&mut self, s: &mut &str) -> Option<Vec<NodeId>> {
        let mut out = vec![self.node(s)?];
        loop {
            let t = s.trim_start();
            let Some(after) = t.strip_prefix('&') else {
                return Some(out);
            };
            *s = after.trim_start();
            out.push(self.node(s)?);
        }
    }

    fn node(&mut self, s: &mut &str) -> Option<NodeId> {
        let len = id_len(s);
        if len == 0 {
            return None;
        }
        let id = &s[..len];
        *s = &s[len..];
        let n = self.b.node(id)?;
        if let Some((shape, label)) = shape(s)? {
            let node = &mut self.b.nodes[n as usize];
            node.shape = shape;
            if let Some(label) = label {
                node.label = label_lines(&label);
            }
        }
        // `:::class`
        if let Some(after) = s.strip_prefix(":::") {
            let len = after.find(|c: char| !id_char(c)).unwrap_or(after.len());
            *s = &after[len..];
        }
        Some(n)
    }
}

fn id_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.')
}

/// The length of the node id at the start of `s`: a `-` belongs to it when a word character
/// follows, so `user-service-->db` is `user-service`, then an arrow.
fn id_len(s: &str) -> usize {
    let mut end = 0;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let next = chars.peek().map(|&(_, n)| n);
        let ok = id_char(c) || (c == '-' && end > 0 && next.is_some_and(char::is_alphanumeric));
        if !ok {
            break;
        }
        end = i + c.len_utf8();
    }
    // an id doesn't end in `.`, which starts a dotted arrow in `A.-B`
    s[..end].trim_end_matches('.').len()
}

/// A quoted label's text, or the text as it is; a markdown string's backticks go.
fn unquote(s: &str) -> &str {
    let s = s.trim();
    let s = s
        .strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .unwrap_or(s);
    s.strip_prefix('`')
        .and_then(|t| t.strip_suffix('`'))
        .unwrap_or(s)
}

/// The shapes, longest opener first: `(opener, closer, shape)`.
const SHAPES: [(&str, &str, Shape); 15] = [
    ("(((", ")))", Shape::Stadium),
    ("((", "))", Shape::Stadium),
    ("([", "])", Shape::Stadium),
    ("[[", "]]", Shape::Rect),
    ("[(", ")]", Shape::Round),
    ("[/", "/]", Shape::Rect),
    ("[/", "\\]", Shape::Rect),
    ("[\\", "\\]", Shape::Rect),
    ("[\\", "/]", Shape::Rect),
    ("[", "]", Shape::Rect),
    ("(", ")", Shape::Round),
    ("{{", "}}", Shape::Diamond),
    ("{", "}", Shape::Diamond),
    (">", "]", Shape::Rect),
    ("@{", "}", Shape::Rect),
];

/// A node's shape and label after its id, if it has one: `Some(None)` for no shape, `None`
/// for one that doesn't close.
#[allow(clippy::option_option)]
fn shape(s: &mut &str) -> Option<Option<(Shape, Option<String>)>> {
    for (open, close, shape) in SHAPES {
        let Some(body) = s.strip_prefix(open) else {
            continue;
        };
        let (text, after) = if body.trim_start().starts_with('"') {
            // a quoted label may hold the closer
            let q = body.trim_start();
            let end = q[1..].find('"')? + 2;
            let after = q[end..].trim_start();
            (&q[..end], after.strip_prefix(close))
        } else {
            match body.find(close) {
                Some(i) => (&body[..i], Some(&body[i + close.len()..])),
                None => (body, None),
            }
        };
        let Some(after) = after else {
            // `[/text\]` and its kin share an opener: try the next closer
            continue;
        };
        *s = after;
        if open == "@{" {
            return Some(Some(shape_spec(text)));
        }
        return Some(Some((shape, Some(unquote(text).to_owned()))));
    }
    match s.chars().next() {
        Some('[' | '(' | '{') => None,
        _ => Some(None),
    }
}

/// `@{ shape: rounded, label: "text" }`, Mermaid's newer shape syntax.
fn shape_spec(spec: &str) -> (Shape, Option<String>) {
    let (mut shape, mut label) = (Shape::Rect, None);
    for field in spec.split(',') {
        let Some((key, value)) = field.split_once(':') else {
            continue;
        };
        let value = unquote(value);
        match key.trim() {
            "shape" => {
                shape = match value {
                    "rounded" | "event" | "cyl" | "cylinder" | "das" | "db" | "database" => {
                        Shape::Round
                    }
                    "stadium" | "pill" | "terminal" | "circle" | "circ" | "dbl-circ"
                    | "sm-circ" => Shape::Stadium,
                    "diam" | "diamond" | "decision" | "question" | "hex" | "hexagon" => {
                        Shape::Diamond
                    }
                    "fork" | "join" => Shape::Fork,
                    _ => Shape::Rect,
                }
            }
            "label" => label = Some(value.to_owned()),
            _ => {}
        }
    }
    (shape, label)
}

/// An arrow between two nodes: its line, the marks at its ends, its label and length.
#[derive(Debug, PartialEq, Eq)]
struct Arrow {
    line: Line,
    from: Mark,
    to: Mark,
    label: Vec<String>,
    minlen: u8,
}

/// Parse the arrow at the start of `s`: `-->`, `---`, `-.->`, `==>`, `~~~`, `<-->`, `--o`,
/// `--x`, longer ones, and a label in it (`-- text -->`) or after it (`-->|text|`).
fn arrow(s: &mut &str) -> Option<Arrow> {
    let (from, body) = match s.as_bytes() {
        [b'<', ..] => (Mark::Arrow, &s[1..]),
        [b'x', b'-' | b'=' | b'.', ..] => (Mark::Cross, &s[1..]),
        [b'o', b'-' | b'=' | b'.', ..] => (Mark::Circle, &s[1..]),
        _ => (Mark::None, *s),
    };
    let line = match body.as_bytes() {
        [b'~', b'~', b'~', ..] => Line::Hidden,
        [b'-', b'.', ..] => Line::Dotted,
        [b'=', b'=', ..] => Line::Thick,
        [b'-', b'-', ..] => Line::Solid,
        _ => return None,
    };
    let (mut arrow, mut rest) = match line {
        Line::Hidden => {
            let n = body.len() - body.trim_start_matches('~').len();
            let a = Arrow {
                line,
                from: Mark::None,
                to: Mark::None,
                label: Vec::new(),
                minlen: (n - 2).min(4) as u8,
            };
            (a, &body[n..])
        }
        _ => line_arrow(body, line)?,
    };
    arrow.from = if line == Line::Hidden {
        Mark::None
    } else {
        from
    };
    // `-->|label|`
    let t = rest.trim_start();
    if let Some(after) = t.strip_prefix('|') {
        let end = after.find('|')?;
        arrow.label = label_lines(unquote(&after[..end]));
        rest = &after[end + 1..];
    }
    *s = rest;
    Some(arrow)
}

/// A solid, dotted or thick arrow, which may hold its label: `-- text -->`.
fn line_arrow(body: &str, line: Line) -> Option<(Arrow, &str)> {
    let (dash, inner) = match line {
        Line::Dotted => ('-', '.'),
        Line::Thick => ('=', '='),
        _ => ('-', '-'),
    };
    // the run of line characters: `-`, then `.`s and a closing `-` for dotted ones
    let run = |s: &str| -> usize {
        let mut n = 0;
        for (i, c) in s.char_indices() {
            if c == dash || c == inner {
                n = i + 1;
            } else {
                break;
            }
        }
        n
    };
    let n = run(body);
    let (end, after) = (&body[..n], &body[n..]);
    let (to, rest) = head(after, n, line);
    if to != Mark::None || closed(end, line) {
        let minlen = length(end, line, to != Mark::None);
        let a = Arrow {
            line,
            from: Mark::None,
            to,
            label: Vec::new(),
            minlen,
        };
        return Some((a, rest));
    }
    // `-- text -->`: the text runs to the closing part
    let opener = match line {
        Line::Dotted => "-.",
        Line::Thick => "==",
        _ => "--",
    };
    let text_and_close = body.strip_prefix(opener)?;
    let closer = match line {
        Line::Dotted => ".-",
        Line::Thick => "==",
        _ => "--",
    };
    let at = text_and_close.find(closer)?;
    let text = &text_and_close[..at];
    let close = &text_and_close[at..];
    let n = run(close);
    let (to, rest) = head(&close[n..], n, line);
    if to == Mark::None && !closed_after_text(&close[..n], line) {
        return None;
    }
    let a = Arrow {
        line,
        from: Mark::None,
        to,
        label: label_lines(unquote(text)),
        minlen: length(&close[..n], line, to != Mark::None),
    };
    Some((a, rest))
}

/// The mark ending an arrow, and what follows it: `>` always, `x` and `o` only when no word
/// goes on after them (`--xB` is still a cross, after a bare `--`).
fn head(after: &str, run: usize, line: Line) -> (Mark, &str) {
    let mut chars = after.chars();
    let mark = match chars.next() {
        Some('>') => return (Mark::Arrow, &after[1..]),
        Some('x') => Mark::Cross,
        Some('o') => Mark::Circle,
        _ => return (Mark::None, after),
    };
    let next = chars.next();
    let bare = run <= 2 && line != Line::Dotted;
    if next.is_none_or(|c| !id_char(c)) || bare {
        (mark, &after[1..])
    } else {
        (Mark::None, after)
    }
}

/// An arrow without a head is whole when long enough: `---`, `-.-`, `===`.
fn closed(run: &str, line: Line) -> bool {
    match line {
        Line::Dotted => run.len() >= 3 && run.ends_with('-'),
        _ => run.len() >= 3,
    }
}

/// The closing part after a label, without a head: `---`, `.-`, `===`.
fn closed_after_text(run: &str, line: Line) -> bool {
    match line {
        Line::Dotted => run.ends_with('-'),
        _ => run.len() >= 3,
    }
}

/// How many layers an arrow spans: one, and one more per extra `-`, `.` or `=` (`-->` and
/// `---` span one, `--->` and `----` two).
fn length(run: &str, line: Line, headed: bool) -> u8 {
    let n = match line {
        Line::Dotted => run.matches('.').count(),
        _ => run.len().saturating_sub(if headed { 1 } else { 2 }),
    };
    n.clamp(1, 4) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arrow_of(s: &str) -> Option<(Arrow, String)> {
        let mut t = s;
        let a = arrow(&mut t)?;
        Some((a, t.to_owned()))
    }

    #[test]
    fn arrows_and_their_marks() {
        let cases = [
            ("-->B", Line::Solid, Mark::None, Mark::Arrow, 1),
            ("--- B", Line::Solid, Mark::None, Mark::None, 1),
            ("--->B", Line::Solid, Mark::None, Mark::Arrow, 2),
            ("-.->B", Line::Dotted, Mark::None, Mark::Arrow, 1),
            ("-..->B", Line::Dotted, Mark::None, Mark::Arrow, 2),
            ("-.-B", Line::Dotted, Mark::None, Mark::None, 1),
            ("==>B", Line::Thick, Mark::None, Mark::Arrow, 1),
            ("===B", Line::Thick, Mark::None, Mark::None, 1),
            ("<-->B", Line::Solid, Mark::Arrow, Mark::Arrow, 1),
            ("--o B", Line::Solid, Mark::None, Mark::Circle, 1),
            ("x--x B", Line::Solid, Mark::Cross, Mark::Cross, 1),
            ("--xB", Line::Solid, Mark::None, Mark::Cross, 1),
            ("~~~B", Line::Hidden, Mark::None, Mark::None, 1),
        ];
        for (src, line, from, to, minlen) in cases {
            let (a, _) = arrow_of(src).unwrap_or_else(|| panic!("{src}"));
            assert_eq!(
                (a.line, a.from, a.to, a.minlen),
                (line, from, to, minlen),
                "{src}"
            );
        }
        assert!(arrow_of("->B").is_none());
        assert!(arrow_of("--B").is_none());
    }

    #[test]
    fn labels_in_and_after_arrows() {
        let (a, rest) = arrow_of("-->|yes| B").unwrap();
        assert_eq!((a.label, rest.as_str()), (vec!["yes".to_owned()], " B"));
        let (a, rest) = arrow_of("-- some text -->B").unwrap();
        assert_eq!(
            (a.label, a.to, rest.as_str()),
            (vec!["some text".to_owned()], Mark::Arrow, "B")
        );
        let (a, _) = arrow_of("-. maybe .-> B").unwrap();
        assert_eq!((a.label, a.line), (vec!["maybe".to_owned()], Line::Dotted));
        let (a, _) = arrow_of("== sure ==> B").unwrap();
        assert_eq!((a.label, a.line), (vec!["sure".to_owned()], Line::Thick));
        let (a, _) = arrow_of("---|\"quoted\"| B").unwrap();
        assert_eq!(a.label, ["quoted"]);
    }

    #[test]
    fn ids_take_inner_hyphens() {
        assert_eq!(id_len("user-service-->db"), "user-service".len());
        assert_eq!(id_len("A-->B"), 1);
        assert_eq!(id_len("A-.->B"), 1);
        assert_eq!(id_len("n1[x]"), 2);
        assert_eq!(id_len("日本-->x"), "日本".len());
    }

    #[test]
    fn shapes() {
        let cases = [
            ("[a]", Shape::Rect, "a"),
            ("(a)", Shape::Round, "a"),
            ("([a])", Shape::Stadium, "a"),
            ("((a))", Shape::Stadium, "a"),
            ("{a}", Shape::Diamond, "a"),
            ("{{a}}", Shape::Diamond, "a"),
            ("[(a)]", Shape::Round, "a"),
            ("[/a\\]", Shape::Rect, "a"),
            (">a]", Shape::Rect, "a"),
            ("[\"a [b]\"]", Shape::Rect, "a [b]"),
            ("[\"`md`\"]", Shape::Rect, "md"),
            ("@{ shape: diamond, label: \"q\" }", Shape::Diamond, "q"),
        ];
        for (src, want, label) in cases {
            let mut s = src;
            let got = shape(&mut s).flatten().unwrap_or_else(|| panic!("{src}"));
            assert_eq!((got.0, got.1.as_deref()), (want, Some(label)), "{src}");
            assert!(s.is_empty(), "{src}: {s:?} left");
        }
        let mut s = "[unclosed";
        assert!(shape(&mut s).is_none());
    }
}
