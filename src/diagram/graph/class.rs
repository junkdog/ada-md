//! The class diagram parser (`classDiagram`, `classDiagram-v2`), onto the flowchart's graph:
//! classes are nodes with compartments of members, relations edges with UML's end marks and
//! cardinalities, namespaces square subgraphs, `note for` a note beside its class. Styling
//! statements are accepted and ignored: colors come from the theme.

use super::{
    Builder, Classifier, Dir, Edge, End, Frame, Line, Mark, Member, NodeId, Note, Part, Shape,
};
use crate::diagram::{Description, Source, Text, each_statement, first_word};

pub(super) fn parse<'a>(
    lines: impl Iterator<Item = (u32, &'a str)>,
    src: Source,
) -> Option<Builder> {
    let mut p = Parser::default();
    p.b.src = src;
    p.b.parts = true;
    each_statement(lines, |line| p.statement(line))?;
    Some(p.b)
}

#[derive(Default)]
struct Parser {
    b: Builder,
    /// The class whose `{ … }` is being read.
    body: Option<NodeId>,
    /// How many namespaces are open.
    namespaces: usize,
    description: Description,
}

impl Parser {
    fn statement(&mut self, line: &str) -> Option<()> {
        if let Some(n) = self.body {
            if line == "}" {
                self.body = None;
            } else {
                self.member(n, line);
            }
            return Some(());
        }
        if self.description.skips(line) {
            return Some(());
        }
        if line == "}" {
            self.namespaces = self.namespaces.checked_sub(1)?;
            return self.b.close().map(|_| ());
        }
        if let Some(rest) = line.strip_prefix("<<") {
            // `<<interface>> Shape`
            let (annotation, id) = rest.split_once(">>")?;
            let n = self.class(id)?;
            self.annotate(n, annotation);
            return Some(());
        }
        let (word, rest) = first_word(line);
        match word {
            "direction" => {
                self.b.dir = Dir::parse(rest)?;
                Some(())
            }
            "class" => self.declaration(rest),
            "namespace" => self.namespace(rest),
            "note" => self.note(rest),
            "style" | "classDef" | "cssClass" | "click" | "link" | "callback" | "accTitle" => {
                Some(())
            }
            "accDescr" => {
                self.description.start(rest);
                Some(())
            }
            _ => match relation(line) {
                Some(r) => self.relation(line, r),
                None => self.member_statement(line),
            },
        }
    }

    /// The class `id` names, added on first mention: `A`, `A~T~`, `` `A b` ``. A generic's
    /// type shows in its name.
    fn class(&mut self, id: &str) -> Option<NodeId> {
        let id = strip_css(id.trim());
        let (id, generic, quoted) = match id.strip_prefix('`') {
            Some(q) => (q.strip_suffix('`')?, None, true),
            None => match id.split_once('~') {
                Some((name, g)) => (name, Some(g.strip_suffix('~')?), false),
                None => (id, None, false),
            },
        };
        let odd = |c: char| c.is_whitespace() || "\"{}()[]:<>`".contains(c);
        if id.is_empty() || !quoted && id.contains(odd) {
            return None;
        }
        let n = self.b.node(id)?;
        let node = &mut self.b.nodes[n as usize];
        if node.shape != Shape::Class {
            node.shape = Shape::Class;
            node.class = Some(Box::default());
        }
        if let Some(g) = generic {
            node.label = vec![Text::from(format!("{id}<{}>", generics(g)).as_str())];
        }
        Some(n)
    }

    /// `class A`, `class A["label"]`, `class A~T~ {`, `class A { +member }`.
    fn declaration(&mut self, rest: &str) -> Option<()> {
        let (head, body) = match rest.split_once('{') {
            Some((head, body)) => (head.trim(), Some(body.trim())),
            None => (rest.trim(), None),
        };
        let (head, annotation) = match head.split_once("<<") {
            Some((h, a)) => (h.trim(), Some(a.strip_suffix(">>")?)),
            None => (head, None),
        };
        let (id, label) = match head.split_once('[') {
            Some((id, label)) => {
                let label = label.trim().strip_suffix(']')?.trim();
                (id, Some(label.strip_prefix('"')?.strip_suffix('"')?))
            }
            None => (head, None),
        };
        let n = self.class(id)?;
        if let Some(label) = label {
            self.b.nodes[n as usize].label = self.b.src.label(label);
        }
        if let Some(a) = annotation {
            self.annotate(n, a);
        }
        match body {
            None => {}
            Some(b) => match b.strip_suffix('}') {
                // on one line
                Some(member) => self.member(n, member.trim()),
                None => {
                    self.body = Some(n);
                    if !b.is_empty() {
                        self.member(n, b);
                    }
                }
            },
        }
        Some(())
    }

    /// `namespace N {`.
    fn namespace(&mut self, rest: &str) -> Option<()> {
        let name = rest.strip_suffix('{')?.trim();
        if name.is_empty() {
            return None;
        }
        self.b.open(name, name, Frame::Square)?;
        self.namespaces += 1;
        Some(())
    }

    fn annotate(&mut self, n: NodeId, annotation: &str) {
        let text = Text::from(format!("«{}»", annotation.trim()).as_str());
        if let Some(c) = &mut self.b.nodes[n as usize].class {
            c.annotation.push(text);
        }
    }

    /// A line of a class's body: a member, or an annotation.
    fn member(&mut self, n: NodeId, line: &str) {
        if line.is_empty() {
            return;
        }
        if let Some(a) = line.strip_prefix("<<").and_then(|a| a.strip_suffix(">>")) {
            self.annotate(n, a);
            return;
        }
        let (mut member, method) = member(line, self.b.src);
        let parts = &mut self.b.member_parts;
        if let Some(c) = &mut self.b.nodes[n as usize].class {
            let start = parts.len() as u32;
            member_parts(&member.text, method, parts);
            member.parts = start..parts.len() as u32;
            if method {
                c.methods.push(member);
            } else {
                c.attributes.push(member);
            }
        }
    }

    /// `A : +member`, or a lone class.
    fn member_statement(&mut self, line: &str) -> Option<()> {
        match line.split_once(':') {
            Some((id, member)) => {
                let n = self.class(id)?;
                self.member(n, member.trim());
                Some(())
            }
            None => self.class(line).map(|_| ()),
        }
    }

    /// `note for A "text"`; a floating `note "text"` has nothing to stand beside.
    fn note(&mut self, rest: &str) -> Option<()> {
        let Some(rest) = rest.strip_prefix("for ") else {
            return Some(());
        };
        let (id, text) = rest.trim().split_once(char::is_whitespace)?;
        let text = text.trim().strip_prefix('"')?.strip_suffix('"')?;
        let n = self.class(id)?;
        // Mermaid breaks a note at `\n` too
        let lines = text
            .split("\\n")
            .flat_map(|l| self.b.src.label(l))
            .collect();
        self.b.nodes[n as usize].note = Some(Note {
            after: true,
            text: lines,
        });
        Some(())
    }

    /// `A "1" *-- "many" B : label`.
    fn relation(&mut self, line: &str, r: Relation) -> Option<()> {
        let (left, right) = (line[..r.start].trim(), &line[r.end..]);
        let (right, label) = match split_outside_quotes(right, ':') {
            Some((r, l)) => (r.trim(), l.trim()),
            None => (right.trim(), ""),
        };
        let (left, card_from) = match left.strip_suffix('"') {
            Some(l) => {
                let (id, card) = l.rsplit_once('"')?;
                (id, card)
            }
            None => (left, ""),
        };
        let (right, card_to) = match right.strip_prefix('"') {
            Some(r) => {
                let (card, id) = r.split_once('"')?;
                (id, card)
            }
            None => (right, ""),
        };
        let from = self.class(left)?;
        let to = self.class(right)?;
        let src = self.b.src;
        self.b.edge(Edge {
            from: End::Node(from),
            to: End::Node(to),
            line: r.line,
            head_from: r.from,
            head_to: r.to,
            label: if label.is_empty() {
                Vec::new()
            } else {
                src.label(label)
            },
            minlen: 1,
            cards: [src.one_line(card_from), src.one_line(card_to)],
        })
    }
}

/// Where a relation's arrow is in a statement, and what it draws.
#[derive(Clone, Copy)]
struct Relation {
    start: usize,
    end: usize,
    line: Line,
    from: Mark,
    to: Mark,
}

/// The relation arrow in `line` (before any `:` label), if it has one: `--` or `..`, with a
/// mark at either end: `<|` / `|>`, `*`, `o`, `<` / `>`, `()`.
fn relation(line: &str) -> Option<Relation> {
    let head = match split_outside_quotes(line, ':') {
        Some((h, _)) => h,
        None => line,
    };
    let bytes = head.as_bytes();
    let mut quoted = false;
    let at = (0..bytes.len().saturating_sub(1)).find(|&i| {
        if bytes[i] == b'"' {
            quoted = !quoted;
        }
        !quoted && matches!(&bytes[i..i + 2], b"--" | b"..")
    })?;
    let line_kind = if bytes[at] == b'-' {
        Line::Solid
    } else {
        Line::Dotted
    };
    let before = &head[..at];
    // `o` is a mark only apart from the id before it
    let apart = |s: &str| s.is_empty() || s.ends_with([' ', '"', '\t']);
    let (from, start) = if before.ends_with("<|") {
        (Mark::Triangle, at - 2)
    } else if before.ends_with("()") {
        (Mark::Circle, at - 2)
    } else if before.ends_with('*') {
        (Mark::Diamond, at - 1)
    } else if before.ends_with('<') {
        (Mark::Arrow, at - 1)
    } else if before.ends_with('o') && apart(&before[..before.len() - 1]) {
        (Mark::HollowDiamond, at - 1)
    } else {
        (Mark::None, at)
    };
    let after = &head[at + 2..];
    let apart_after = |s: &str| s.is_empty() || s.starts_with([' ', '"', '\t']);
    let (to, end) = if after.starts_with("|>") {
        (Mark::Triangle, at + 4)
    } else if after.starts_with("()") {
        (Mark::Circle, at + 4)
    } else if after.starts_with('*') {
        (Mark::Diamond, at + 3)
    } else if after.starts_with('>') {
        (Mark::Arrow, at + 3)
    } else if after.starts_with('o') && apart_after(&after[1..]) {
        (Mark::HollowDiamond, at + 3)
    } else {
        (Mark::None, at + 2)
    };
    Some(Relation {
        start,
        end,
        line: line_kind,
        from,
        to,
    })
}

/// `s` split at the first `sep` outside double quotes.
fn split_outside_quotes(s: &str, sep: char) -> Option<(&str, &str)> {
    let mut quoted = false;
    for (i, c) in s.char_indices() {
        if c == '"' {
            quoted = !quoted;
        } else if c == sep && !quoted {
            return Some((&s[..i], &s[i + c.len_utf8()..]));
        }
    }
    None
}

/// An id without its `:::class`.
fn strip_css(id: &str) -> &str {
    id.split_once(":::").map_or(id, |(id, _)| id).trim()
}

/// A member as Mermaid shows it, and whether it is a method (it has a `(`): the visibility
/// (`+ - # ~`) kept, a trailing `$` (static) or `*` (abstract) taken off as its classifier, a
/// method's return type after ` : `, generics as `<T>`. As written, it keeps its origin.
fn member(line: &str, src: Source) -> (Member, bool) {
    let (body, mut classifier) = match line.strip_suffix('$') {
        Some(b) => (b.trim_end(), Classifier::Static),
        None => match line.strip_suffix('*') {
            Some(b) => (b.trim_end(), Classifier::Abstract),
            None => (line, Classifier::None),
        },
    };
    let (vis, rest) = match body.chars().next() {
        Some(c @ ('+' | '-' | '#' | '~')) => (Some(c), &body[1..]),
        _ => (None, body),
    };
    let method = rest.contains('(');
    // what follows the parameters is a return type when it starts like one; Mermaid takes
    // `a() / b()`'s `/ b()` for one too
    let ret = rest.rfind(')').map(|close| {
        let mut ret = rest[close + 1..].trim_end();
        // or the classifier right after the `)`: `count()$ int`
        if let Some(c) = ret.chars().next().filter(|c| matches!(c, '$' | '*')) {
            classifier = if c == '$' {
                Classifier::Static
            } else {
                Classifier::Abstract
            };
            ret = &ret[1..];
        }
        let ret = ret.trim();
        (close, ret.strip_prefix(':').map_or(ret, str::trim))
    });
    let typed = |r: &str| r.starts_with(|c: char| c.is_alphanumeric() || "_&[('".contains(c));
    // written into one string: a member is parsed for every class box
    let vis = vis.map_or("", |_| &body[..1]);
    let shown = match ret {
        Some((close, ret)) if method && typed(ret) => {
            let mut s = String::with_capacity(body.len() + 3);
            s.push_str(vis);
            generics_into(&mut s, &rest[..=close]);
            s.push_str(" : ");
            generics_into(&mut s, ret);
            Some(s)
        }
        _ if rest.contains('~') => {
            let mut s = String::with_capacity(body.len());
            s.push_str(vis);
            generics_into(&mut s, rest);
            Some(s)
        }
        _ => None,
    };
    let text = match shown {
        Some(s) => Text { s, origin: None },
        None => src.one_line(body),
    };
    (Member::new(text, classifier), method)
}

/// The types and parameters in member text `s` as [`member`] shows it, as byte ranges onto `out`:
/// each parameter of a method, `name: Type` or `Type name` (a lone word is a name), and its
/// return type after ` : `; an attribute's type, after a `:` or before its name.
fn member_parts(s: &str, method: bool, out: &mut Vec<(u32, u32, Part)>) {
    let b = s.as_bytes();
    let body = usize::from(s.starts_with(['+', '-', '#', '~']));
    if !method {
        declaration(b, body, None, |_| false, out);
        return;
    }
    let (mut at, mut last) = (body, None);
    'groups: while let Some(open) = s[at..].find('(').map(|i| at + i) {
        // its parameters, each up to a `,` or the `)`; a group left open has none
        let (mut from, kept) = (open + 1, out.len());
        loop {
            let i = declaration(
                b,
                from,
                Some(Part::Param),
                |c| matches!(c, b',' | b')'),
                out,
            );
            if i == b.len() {
                out.truncate(kept);
                break 'groups;
            }
            from = i + 1;
            if b[i] == b')' {
                break;
            }
        }
        (at, last) = (from, Some(from - 1));
    }
    if let Some(ret) = last.and_then(|close| s[close + 1..].strip_prefix(" : ")) {
        push(out, b.len() - ret.len(), b.len(), Part::Type, b);
    }
}

/// A declaration from `b[from]` up to the first byte outside brackets that `stop` takes, or
/// the end: `name: Type` or `Type name`, a default after `=` left out. Its type, and its name
/// as `name` when that is a part; returns where it stopped. One pass over its bytes, since
/// every mark it looks for is ASCII.
fn declaration(
    b: &[u8],
    from: usize,
    name: Option<Part>,
    stop: impl Fn(u8) -> bool,
    out: &mut Vec<(u32, u32, Part)>,
) -> usize {
    // outside brackets: the first `:` that isn't a path's `::`, the last space before a word,
    // and where a default starts
    let (mut colon, mut space, mut default) = (None, None, None);
    let (mut depth, mut prev, mut to) = (0usize, b' ', b.len());
    for (i, &c) in b.iter().enumerate().skip(from) {
        if c.is_ascii_alphanumeric() || c == b'_' {
            // most of it: no mark, no bracket
            prev = c;
            continue;
        }
        if depth == 0 {
            if stop(c) {
                to = i;
                break;
            }
            if default.is_none() {
                let next = b.get(i + 1).copied();
                match c {
                    b'=' => default = Some(i),
                    b':' if colon.is_none() && prev != b':' && next != Some(b':') => {
                        colon = Some(i);
                    }
                    b' ' if next.is_some_and(|n| !matches!(n, b' ' | b'=') && !stop(n)) => {
                        space = Some(i);
                    }
                    _ => {}
                }
            }
        }
        depth = bracket(depth, prev, c);
        prev = c;
    }
    let end = default.unwrap_or(to);
    match colon {
        Some(i) => {
            if let Some(part) = name {
                push(out, from, i, part, b);
            }
            push(out, i + 1, end, Part::Type, b);
        }
        None => {
            // `Type name`: the type comes first
            let named = match space {
                Some(i) => {
                    push(out, from, i, Part::Type, b);
                    i + 1
                }
                None => from,
            };
            if let Some(part) = name {
                push(out, named, end, part, b);
            }
        }
    }
    to
}

/// `b[from..to]` without the spaces around it, as a part, unless it is blank.
fn push(out: &mut Vec<(u32, u32, Part)>, from: usize, to: usize, part: Part, b: &[u8]) {
    let (mut start, mut end) = (from, to);
    while start < end && b[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && b[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    if start < end {
        out.push((start as u32, end as u32, part));
    }
}

/// How deep in brackets after byte `c`, which follows `prev`.
fn bracket(depth: usize, prev: u8, c: u8) -> usize {
    match c {
        b'(' | b'[' | b'{' | b'<' => depth + 1,
        // not an arrow's `>`: `Fn() -> T`
        b'>' if matches!(prev, b'-' | b'=') => depth,
        b')' | b']' | b'}' | b'>' => depth.saturating_sub(1),
        _ => depth,
    }
}

/// Mermaid's generics, `~T~`, as `<T>`: a `~` before a word opens one, any other closes the
/// innermost open one (`Rc~RefCell~T~~` is `Rc<RefCell<T>>`). Unbalanced, they pair up in
/// turn instead.
fn generics(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    generics_into(&mut out, s);
    out
}

/// [`generics`] onto the end of `out`.
fn generics_into(out: &mut String, s: &str) {
    if !s.contains('~') {
        out.push_str(s);
        return;
    }
    let start = out.len();
    let mut depth = 0usize;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '~' {
            out.push(c);
            continue;
        }
        let opens = chars
            .peek()
            .is_some_and(|&n| n.is_alphanumeric() || n == '_' || n == '(' || n == '[');
        if opens {
            depth += 1;
            out.push('<');
        } else if depth > 0 {
            depth -= 1;
            out.push('>');
        } else {
            depth = usize::MAX;
            break;
        }
    }
    if depth == 0 {
        return;
    }
    // unbalanced: alternate
    out.truncate(start);
    let mut open = true;
    out.extend(s.chars().map(|c| match c {
        '~' => {
            let g = if open { '<' } else { '>' };
            open = !open;
            g
        }
        c => c,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(src: &str) -> Builder {
        let lines = crate::diagram::statements(src).skip(1);
        parse(lines, Source::default()).expect("parses")
    }

    fn class(b: &Builder, id: &str) -> usize {
        b.nodes.iter().position(|n| n.id == id).expect("a class")
    }

    #[test]
    fn generics_open_before_a_word_and_close_otherwise() {
        assert_eq!(generics("Vec~Line~"), "Vec<Line>");
        assert_eq!(
            generics("Rc~RefCell~TerminalGrid~~"),
            "Rc<RefCell<TerminalGrid>>"
        );
        assert_eq!(generics("HashMap~str, u16~"), "HashMap<str, u16>");
        assert_eq!(generics("Box~dyn Atlas~"), "Box<dyn Atlas>");
        assert_eq!(generics("List~int~ id"), "List<int> id");
        assert_eq!(generics("a~ b~"), "a< b>");
        assert_eq!(generics("plain"), "plain");
    }

    #[test]
    fn members_as_mermaid_shows_them() {
        let m = |s: &str| {
            let (m, method) = member(s, Source::default());
            (m.text.s, m.classifier, method)
        };
        assert_eq!(
            m("+String path"),
            ("+String path".into(), Classifier::None, false)
        );
        assert_eq!(m("+reload()"), ("+reload()".into(), Classifier::None, true));
        assert_eq!(
            m("+resolve(key, bits) Option~Slot~"),
            (
                "+resolve(key, bits) : Option<Slot>".into(),
                Classifier::None,
                true
            )
        );
        assert_eq!(
            m("someMethod()$"),
            ("someMethod()".into(), Classifier::Static, true)
        );
        assert_eq!(
            m("+area() double*"),
            ("+area() : double".into(), Classifier::Abstract, true)
        );
        assert_eq!(
            m("~int count$"),
            ("~int count".into(), Classifier::Static, false)
        );
        assert_eq!(
            m("-grid: Rc~T~"),
            ("-grid: Rc<T>".into(), Classifier::None, false)
        );
        assert_eq!(
            m("Normal(u16)"),
            ("Normal(u16)".into(), Classifier::None, true)
        );
        assert_eq!(
            m("+a() / b()"),
            ("+a() / b()".into(), Classifier::None, true)
        );
        assert_eq!(
            m("+count()$ int"),
            ("+count() : int".into(), Classifier::Static, true)
        );
    }

    #[test]
    fn members_parts_are_their_types_and_parameters() {
        let parts = |s: &str| {
            let (m, method) = member(s, Source::default());
            let mut out = Vec::new();
            member_parts(&m.text, method, &mut out);
            out.iter()
                .map(|&(a, b, part)| (m.text.s[a as usize..b as usize].to_owned(), part))
                .collect::<Vec<_>>()
        };
        let (ty, param) = (Part::Type, Part::Param);
        let p = |s: &str, part| (s.to_owned(), part);
        assert_eq!(
            parts("+resolve(key, bits) Option~Slot~"),
            [p("key", param), p("bits", param), p("Option<Slot>", ty)]
        );
        assert_eq!(
            parts("+put(key: &str, v: HashMap~u16, str~)$"),
            [
                p("key", param),
                p("&str", ty),
                p("v", param),
                p("HashMap<u16, str>", ty)
            ]
        );
        assert_eq!(
            parts("+deposit(int amount, List~int~ ids) bool"),
            [
                p("int", ty),
                p("amount", param),
                p("List<int>", ty),
                p("ids", param),
                p("bool", ty)
            ]
        );
        assert_eq!(
            parts("+bind(gl) / delete(gl)"),
            [p("gl", param), p("gl", param)]
        );
        assert_eq!(
            parts("+run(f: Fn() -> u8, n = 3)"),
            [p("f", param), p("Fn() -> u8", ty), p("n", param)]
        );
        assert_eq!(parts("+String path"), [p("String", ty)]);
        assert_eq!(
            parts("-glyphs: HashMap~str, u16~"),
            [p("HashMap<str, u16>", ty)]
        );
        assert_eq!(parts("-io: std::io::Error"), [p("std::io::Error", ty)]);
        assert_eq!(parts("+count$"), []);
        assert_eq!(parts("+reload()"), []);
    }

    #[test]
    fn relations_marks_and_lines() {
        fn rel(s: &str) -> (Mark, Line, Mark, &str) {
            let r = relation(s).expect("a relation");
            (r.from, r.line, r.to, &s[r.start..r.end])
        }
        assert_eq!(
            rel("A <|-- B"),
            (Mark::Triangle, Line::Solid, Mark::None, "<|--")
        );
        assert_eq!(
            rel("A *-- B"),
            (Mark::Diamond, Line::Solid, Mark::None, "*--")
        );
        assert_eq!(
            rel("A o-- B"),
            (Mark::HollowDiamond, Line::Solid, Mark::None, "o--")
        );
        assert_eq!(rel("Foo--B"), (Mark::None, Line::Solid, Mark::None, "--"));
        assert_eq!(
            rel("A --> B"),
            (Mark::None, Line::Solid, Mark::Arrow, "-->")
        );
        assert_eq!(
            rel("A ..> B"),
            (Mark::None, Line::Dotted, Mark::Arrow, "..>")
        );
        assert_eq!(
            rel("A ..|> B"),
            (Mark::None, Line::Dotted, Mark::Triangle, "..|>")
        );
        assert_eq!(rel("A .. B"), (Mark::None, Line::Dotted, Mark::None, ".."));
        assert_eq!(
            rel("A ()-- B"),
            (Mark::Circle, Line::Solid, Mark::None, "()--")
        );
        assert_eq!(
            rel("A <|--|> B"),
            (Mark::Triangle, Line::Solid, Mark::Triangle, "<|--|>")
        );
        assert_eq!(
            rel("A --o B"),
            (Mark::None, Line::Solid, Mark::HollowDiamond, "--o")
        );
        assert_eq!(rel("A --oB"), (Mark::None, Line::Solid, Mark::None, "--"));
        assert!(relation("A : +int x").is_none());
        assert!(relation("A : +f(a.. b)").is_none());
        assert!(relation("A \"x--y\" B").is_none());
    }

    #[test]
    fn bodies_members_and_annotations() {
        let b = parsed(
            "classDiagram\nclass Atlas {\n<<sealed trait>>\n+flush(gl)\n+String name\n}\n\
             <<enum>> Slot\nSlot : Normal(u16)\nclass Box~T~\nclass L[\"A label\"]\nclass E { +x }",
        );
        let atlas = b.nodes[class(&b, "Atlas")].class.as_ref().unwrap();
        assert_eq!(atlas.annotation, ["«sealed trait»"]);
        assert_eq!(atlas.methods[0].text, "+flush(gl)");
        assert_eq!(atlas.attributes[0].text, "+String name");
        let slot = b.nodes[class(&b, "Slot")].class.as_ref().unwrap();
        assert_eq!(slot.annotation, ["«enum»"]);
        assert_eq!(slot.methods[0].text, "Normal(u16)");
        assert_eq!(b.nodes[class(&b, "Box")].label, ["Box<T>"]);
        assert_eq!(b.nodes[class(&b, "L")].label, ["A label"]);
        let e = b.nodes[class(&b, "E")].class.as_ref().unwrap();
        assert_eq!(e.attributes[0].text, "+x");
        assert!(b.nodes.iter().all(|n| n.shape == Shape::Class));
    }

    #[test]
    fn relations_with_cardinality_and_labels() {
        let b = parsed(
            "classDiagram\nDoc \"1\" --> \"*\" Line : has\nLine <|-- CodeLine\nA ..> B : uses: a lot",
        );
        assert_eq!(b.edges[0].cards, ["1", "*"]);
        assert_eq!(b.edges[0].label, ["has"]);
        assert_eq!(b.edges[0].head_to, Mark::Arrow);
        assert_eq!(b.edges[1].head_from, Mark::Triangle);
        assert_eq!(b.edges[2].label, ["uses: a lot"]);
        assert_eq!(b.edges[2].line, Line::Dotted);
        assert_eq!(b.nodes.len(), 5);
    }

    #[test]
    fn namespaces_notes_direction_and_what_is_ignored() {
        let b = parsed(
            "classDiagram\ndirection LR\nnamespace Shapes {\nclass Square\nclass Circle\n}\n\
             note for Square \"one\\ntwo<br>three\"\nnote \"floating\"\nstyle Square fill:#f00\n\
             cssClass \"Square\" hot\nclick Square call cb()\nclass Circle:::hot",
        );
        assert_eq!(b.dir, Dir::Right);
        assert_eq!(b.clusters.len(), 1);
        assert!(b.nodes.iter().all(|n| n.parent == Some(0)));
        let note = b.nodes[class(&b, "Square")].note.as_ref().unwrap();
        assert_eq!(note.text, ["one", "two", "three"]);
    }

    #[test]
    fn what_doesnt_parse() {
        for src in [
            "classDiagram\n}",
            "classDiagram\nthis is not a class",
            "classDiagram\nclass A[label]",
            "classDiagram\ndirection sideways",
        ] {
            let lines = crate::diagram::statements(src).skip(1);
            assert!(parse(lines, Source::default()).is_none(), "{src:?}");
        }
    }
}
