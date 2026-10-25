//! The state diagram parser (`stateDiagram`, `stateDiagram-v2`), onto the flowchart's graph:
//! states are rounded nodes, `[*]` a start or end dot per composite state, composite states
//! rounded subgraphs and their concurrent regions (`--`) dashed ones.

use std::collections::HashMap;

use super::{Builder, ClusterId, Dir, Edge, End, Frame, Line, Mark, NodeId, Note, Shape};
use crate::diagram::{Description, Source, Text, each_statement, first_word, label_lines};

pub(super) fn parse<'a>(
    lines: impl Iterator<Item = (u32, &'a str)>,
    src: Source,
) -> Option<Builder> {
    let mut p = Parser::default();
    p.b.src = src;
    each_statement(lines, |line| p.statement(line))?;
    // Mermaid wants composite states closed; ada closes them for it
    Some(p.b)
}

#[derive(Default)]
struct Parser {
    b: Builder,
    /// The open composite states, innermost last, and whether each is split into regions.
    composites: Vec<(ClusterId, bool)>,
    /// The start and end dots of each scope: the top level or a composite state.
    dots: HashMap<Option<ClusterId>, (Option<NodeId>, Option<NodeId>)>,
    /// A multi-line note being read: its state, its side and its lines so far.
    note: Option<(NodeId, bool, Vec<Text>)>,
    description: Description,
}

impl Parser {
    fn statement(&mut self, line: &str) -> Option<()> {
        if let Some((n, after, text)) = &mut self.note {
            if line.eq_ignore_ascii_case("end note") {
                let note = Note {
                    after: *after,
                    text: std::mem::take(text),
                };
                self.b.nodes[*n as usize].note = Some(note);
                self.note = None;
            } else {
                text.extend(self.b.src.label(line));
            }
            return Some(());
        }
        if self.description.skips(line) {
            return Some(());
        }
        if line == "}" {
            return self.close();
        }
        if line == "--" {
            return self.region();
        }
        let (word, rest) = first_word(line);
        match word {
            "direction" => {
                if self.composites.is_empty() {
                    self.b.dir = Dir::parse(rest)?;
                }
                Some(())
            }
            "state" => self.state(rest),
            "note" => self.note(rest),
            "classDef" | "class" | "style" | "scale" | "accTitle" | "hide" => Some(()),
            "accDescr" => {
                self.description.start(rest);
                Some(())
            }
            _ if line.contains("-->") => self.transition(line),
            _ => self.description(line),
        }
    }

    /// The state `id` names: a rounded node, or a scope's start or end dot for `[*]`.
    fn node(&mut self, id: &str, end: bool) -> Option<NodeId> {
        let id = strip_class(id.trim());
        if id == "[*]" {
            let scope = self.b.open.last().copied();
            let slot = self.dots.entry(scope).or_default();
            let (dot, shape) = if end {
                (&mut slot.1, Shape::End)
            } else {
                (&mut slot.0, Shape::Start)
            };
            if let Some(n) = *dot {
                return Some(n);
            }
            let n = self.b.anonymous(shape)?;
            *dot = Some(n);
            return Some(n);
        }
        if id.is_empty() || id.contains(char::is_whitespace) {
            return None;
        }
        let n = self.b.node(id)?;
        let node = &mut self.b.nodes[n as usize];
        if node.shape == Shape::Rect {
            node.shape = Shape::Round;
        }
        Some(n)
    }

    /// `A --> B`, `[*] --> A`, `A --> B : label`.
    fn transition(&mut self, line: &str) -> Option<()> {
        let (from, rest) = line.split_once("-->")?;
        let (to, label) = rest.split_once(':').unwrap_or((rest, ""));
        let from = self.node(from, false)?;
        let to = self.node(to, true)?;
        self.b.edge(Edge {
            from: End::Node(from),
            to: End::Node(to),
            line: Line::Solid,
            head_from: Mark::None,
            head_to: Mark::Arrow,
            label: self.b.src.label(label.trim()),
            minlen: 1,
            cards: Default::default(),
        })
    }

    /// `A : description`: the first replaces the name, more add lines.
    fn description(&mut self, line: &str) -> Option<()> {
        let (id, text) = line.split_once(':').unwrap_or((line, ""));
        self.describe(id, text)
    }

    /// State `id`'s description `text`.
    fn describe(&mut self, id: &str, text: &str) -> Option<()> {
        let n = self.node(id, false)?;
        let text = text.trim();
        if text.is_empty() {
            return Some(());
        }
        let node = &mut self.b.nodes[n as usize];
        if node.label == label_lines(&node.id) {
            node.label = self.b.src.label(text);
        } else {
            node.label.extend(self.b.src.label(text));
        }
        Some(())
    }

    /// `state A`, `state "name" as A`, `state A <<fork>>`, `state A {`.
    fn state(&mut self, rest: &str) -> Option<()> {
        let (body, composite) = match rest.strip_suffix('{') {
            Some(b) => (b.trim(), true),
            None => (rest, false),
        };
        let (id, name) = if let Some(q) = body.strip_prefix('"') {
            let (name, after) = q.split_once('"')?;
            let id = after.trim().strip_prefix("as")?.trim();
            (id, Some(name))
        } else {
            let (id, after) = body.split_once(char::is_whitespace).unwrap_or((body, ""));
            let after = after.trim();
            if let Some(text) = after.strip_prefix(':') {
                self.describe(id, text)?;
                return Some(());
            }
            let shape = match after {
                "<<fork>>" | "<<join>>" | "[[fork]]" | "[[join]]" => Some(Shape::Fork),
                "<<choice>>" | "[[choice]]" => Some(Shape::Choice),
                "" => None,
                _ => return None,
            };
            if let Some(shape) = shape {
                let n = self.node(id, false)?;
                let node = &mut self.b.nodes[n as usize];
                node.shape = shape;
                node.label.clear();
                return Some(());
            }
            (id, None)
        };
        let id = strip_class(id);
        if composite {
            let title = name.unwrap_or(id);
            let c = self.b.open(id, title, Frame::Round)?;
            self.composites.push((c, false));
            return Some(());
        }
        let n = self.node(id, false)?;
        if let Some(name) = name {
            self.b.nodes[n as usize].label = self.b.src.label(name);
        }
        Some(())
    }

    /// `note left of A : text`, or `note right of A` and lines up to `end note`.
    fn note(&mut self, rest: &str) -> Option<()> {
        let (place, text) = match rest.split_once(':') {
            Some((p, t)) => (p.trim(), Some(t.trim())),
            None => (rest.trim(), None),
        };
        let (after, id) = if let Some(id) = place.strip_prefix("right of") {
            (true, id)
        } else if let Some(id) = place.strip_prefix("left of") {
            (false, id)
        } else {
            // a floating note (`note "text" as N`) has nothing to stand beside
            return Some(());
        };
        let n = self.node(id, false)?;
        match text {
            Some(text) => {
                self.b.nodes[n as usize].note = Some(Note {
                    after,
                    text: self.b.src.label(text),
                });
            }
            None => self.note = Some((n, after, Vec::new())),
        }
        Some(())
    }

    /// `--` splits the innermost composite state into concurrent regions, each a dashed frame.
    fn region(&mut self) -> Option<()> {
        let (comp, split) = *self.composites.last()?;
        if split {
            self.b.close()?;
        } else {
            // what came before the first `--` is the first region
            let first = self.b.open("", "", Frame::Dashed)?;
            self.b.close();
            for n in &mut self.b.nodes {
                if n.parent == Some(comp) {
                    n.parent = Some(first);
                }
            }
            for (i, c) in self.b.clusters.iter_mut().enumerate() {
                if c.parent == Some(comp) && i as ClusterId != first {
                    c.parent = Some(first);
                }
            }
            if let Some(dots) = self.dots.remove(&Some(comp)) {
                self.dots.insert(Some(first), dots);
            }
            if let Some(top) = self.composites.last_mut() {
                top.1 = true;
            }
        }
        self.b.open("", "", Frame::Dashed)?;
        Some(())
    }

    fn close(&mut self) -> Option<()> {
        let (_, split) = self.composites.pop()?;
        if split {
            self.b.close()?;
        }
        self.b.close().map(|_| ())
    }
}

/// An id without its `:::class`.
fn strip_class(id: &str) -> &str {
    id.split_once(":::").map_or(id, |(id, _)| id).trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(src: &str) -> Builder {
        let lines = crate::diagram::statements(src).skip(1);
        parse(lines, crate::diagram::Source::default()).expect("parses")
    }

    #[test]
    fn transitions_dots_and_descriptions() {
        let b = parsed(
            "stateDiagram-v2\n[*] --> Idle\nIdle --> Busy : work\nBusy --> [*]\nIdle : waiting<br/>for work",
        );
        let shapes: Vec<_> = b.nodes.iter().map(|n| n.shape).collect();
        assert_eq!(
            shapes,
            [Shape::Start, Shape::Round, Shape::Round, Shape::End]
        );
        assert_eq!(b.nodes[1].label, ["waiting", "for work"]);
        assert_eq!(b.edges[1].label, ["work"]);
        assert_eq!(b.edges.len(), 3);
    }

    #[test]
    fn composites_have_their_own_dots() {
        let b = parsed("stateDiagram\n[*] --> A\nstate A {\n[*] --> a1\na1 --> [*]\n}\nA --> [*]");
        let starts = b.nodes.iter().filter(|n| n.shape == Shape::Start).count();
        assert_eq!(starts, 2);
        assert_eq!(b.clusters.len(), 1);
        let a1 = b.nodes.iter().find(|n| n.id == "a1").unwrap();
        assert_eq!(a1.parent, Some(0));
    }

    #[test]
    fn regions_split_a_composite() {
        let b = parsed("stateDiagram\nstate Active {\nx --> y\n--\np --> q\n}");
        assert_eq!(b.clusters.len(), 3);
        let parent = |id: &str| b.nodes.iter().find(|n| n.id == id).unwrap().parent;
        assert_eq!(parent("x"), Some(1));
        assert_eq!(parent("p"), Some(2));
        assert!(b.clusters[1..].iter().all(|c| c.parent == Some(0)));
    }

    #[test]
    fn named_states_forks_and_notes() {
        let b = parsed(
            "stateDiagram\nstate \"Waiting for input\" as W\nstate F <<fork>>\nnote right of W : a note\n\
             note left of F\nline one\nline two\nend note",
        );
        assert_eq!(b.nodes[0].label, ["Waiting for input"]);
        assert_eq!(b.nodes[1].shape, Shape::Fork);
        assert!(b.nodes[0].note.as_ref().is_some_and(|n| n.after));
        assert_eq!(
            b.nodes[1].note.as_ref().unwrap().text,
            ["line one", "line two"]
        );
    }

    #[test]
    fn what_doesnt_parse() {
        let lines = crate::diagram::statements("stateDiagram\n}").skip(1);
        assert!(parse(lines, crate::diagram::Source::default()).is_none());
        let lines = crate::diagram::statements("stateDiagram\nstate X <<bogus>>").skip(1);
        assert!(parse(lines, crate::diagram::Source::default()).is_none());
    }
}
