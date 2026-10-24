//! Flowcharts and state diagrams (ada-mermaid, DG-3): both parse into one graph IR, which
//! `topo` lays out as far as it can without knowing the width, once, at parse. Drawing
//! (`place`, then `paint`) sizes the labels, places and routes everything, and draws it, for
//! each width; labels wrap narrower until the drawing fits, then it scrolls.
//!
//! A subgraph (or composite state) is laid out on its own, as a block in its parent's layout,
//! with ports on its border where edges cross it, so frames never overlap anything.

mod flowchart;
mod paint;
mod place;
mod state;
mod topo;

use std::collections::HashMap;

use super::{Drawn, Source, Text};

pub type NodeId = u32;
pub type ClusterId = u32;

/// The most nodes, edges and subgraphs a diagram may have; past them it shows as source.
const MAX_NODES: usize = 400;
const MAX_EDGES: usize = 800;
const MAX_CLUSTERS: usize = 100;

/// A flowchart or state diagram, with its width-independent layout.
#[derive(Debug)]
pub struct Graph {
    pub dir: Dir,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub clusters: Vec<Cluster>,
    topo: topo::Topo,
}

/// Where edges point: down (`TD`, `TB`), up (`BT`), right (`LR`) or left (`RL`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dir {
    #[default]
    Down,
    Up,
    Right,
    Left,
}

impl Dir {
    pub(super) fn parse(word: &str) -> Option<Dir> {
        match word {
            "TB" | "TD" => Some(Dir::Down),
            "BT" => Some(Dir::Up),
            "LR" => Some(Dir::Right),
            "RL" => Some(Dir::Left),
            _ => None,
        }
    }

    /// Layers stack along rows (`TD`, `BT`) rather than columns.
    fn vertical(self) -> bool {
        matches!(self, Dir::Down | Dir::Up)
    }
}

#[derive(Debug)]
pub struct Node {
    pub id: String,
    /// A line per `<br/>`.
    pub label: Vec<Text>,
    pub shape: Shape,
    pub parent: Option<ClusterId>,
    pub note: Option<Note>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Rect,
    Round,
    /// `([…])` and `((…))`: rounded, with `(` `)` sides.
    Stadium,
    /// `{…}` and `{{…}}`.
    Diamond,
    /// A state diagram's `[*]` before an arrow.
    Start,
    /// `[*]` after an arrow.
    End,
    /// `<<choice>>`.
    Choice,
    /// `<<fork>>` and `<<join>>`: a bar.
    Fork,
}

/// A state diagram's note, beside its state.
#[derive(Debug)]
pub struct Note {
    /// `left of` (before, on the cross axis) or `right of`.
    pub after: bool,
    pub text: Vec<Text>,
}

#[derive(Debug)]
pub struct Cluster {
    pub id: String,
    pub title: Text,
    pub parent: Option<ClusterId>,
    pub frame: Frame,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    /// A flowchart's subgraph.
    Square,
    /// A composite state.
    Round,
    /// One of a composite state's concurrent regions.
    Dashed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    Node(NodeId),
    /// An edge to a subgraph itself ends at its frame.
    Cluster(ClusterId),
}

#[derive(Debug)]
pub struct Edge {
    pub from: End,
    pub to: End,
    pub line: Line,
    /// The mark at `from` (`<-->`, `o--o`).
    pub head_from: Mark,
    pub head_to: Mark,
    pub label: Vec<Text>,
    /// Layers it spans at least: longer arrows (`--->`) span more.
    pub minlen: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Line {
    Solid,
    Dotted,
    Thick,
    /// `~~~`: placed like an edge, not drawn.
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    None,
    Arrow,
    /// `--o`.
    Circle,
    /// `--x`.
    Cross,
}

/// Parse a flowchart (`flowchart`, `graph`) or a state diagram. `header` is the first line.
pub(super) fn parse<'a>(
    kind: &str,
    header: &'a str,
    lines: impl Iterator<Item = (u32, &'a str)>,
    src: Source,
) -> Option<Graph> {
    let b = match kind {
        "stateDiagram" | "stateDiagram-v2" => state::parse(lines, src)?,
        _ => flowchart::parse(header, lines, src)?,
    };
    b.finish()
}

/// Draw `g` to fit `avail` columns where it can: labels wrap at each width in
/// [`place::CAPS`] in turn until one fits; past the last it scrolls.
pub(super) fn draw(g: &Graph, avail: usize, ascii: bool) -> Option<Drawn> {
    let mut best: Option<place::Placed> = None;
    // a cap no label reaches draws what the one before did
    let widest = g
        .nodes
        .iter()
        .flat_map(|n| n.label.iter().chain(n.note.iter().flat_map(|n| &n.text)))
        .chain(g.edges.iter().flat_map(|e| &e.label))
        .map(|l| super::width(l))
        .max()
        .unwrap_or(0);
    for (i, &cap) in place::CAPS.iter().enumerate() {
        if i > 0 && cap >= widest {
            continue;
        }
        let placed = place::place(g, &g.topo, cap)?;
        let fits = placed.canvas_width(g.dir) <= avail;
        let narrower = best
            .as_ref()
            .is_none_or(|b| placed.canvas_width(g.dir) < b.canvas_width(g.dir));
        if fits || narrower {
            best = Some(placed);
        }
        if fits {
            break;
        }
    }
    let placed = best?;
    Some(Drawn {
        canvas: paint::paint(g, &g.topo, &placed, ascii),
        header: None,
        notes: Vec::new(),
    })
}

/// What both parsers build the graph with.
#[derive(Default)]
struct Builder {
    src: Source,
    dir: Dir,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    clusters: Vec<Cluster>,
    ids: HashMap<String, NodeId>,
    /// The subgraphs open where the parser is, innermost last.
    open: Vec<ClusterId>,
}

impl Builder {
    /// The node `id` names, added on first mention. A mention inside a subgraph moves it
    /// there, as in Mermaid.
    fn node(&mut self, id: &str) -> Option<NodeId> {
        let id = id.trim();
        if id.is_empty() {
            return None;
        }
        let parent = self.open.last().copied();
        if let Some(&n) = self.ids.get(id) {
            if parent.is_some() {
                self.nodes[n as usize].parent = parent;
            }
            return Some(n);
        }
        if self.nodes.len() >= MAX_NODES {
            return None;
        }
        let n = self.nodes.len() as NodeId;
        self.nodes.push(Node {
            id: id.to_owned(),
            label: self.src.label(id),
            shape: Shape::Rect,
            parent,
            note: None,
        });
        self.ids.insert(id.to_owned(), n);
        Some(n)
    }

    /// A node that is never looked up by id, like a state diagram's `[*]`.
    fn anonymous(&mut self, shape: Shape) -> Option<NodeId> {
        if self.nodes.len() >= MAX_NODES {
            return None;
        }
        self.nodes.push(Node {
            id: String::new(),
            label: Vec::new(),
            shape,
            parent: self.open.last().copied(),
            note: None,
        });
        Some((self.nodes.len() - 1) as NodeId)
    }

    /// Open a subgraph.
    fn open(&mut self, id: &str, title: &str, frame: Frame) -> Option<ClusterId> {
        if self.clusters.len() >= MAX_CLUSTERS {
            return None;
        }
        self.clusters.push(Cluster {
            id: id.trim().to_owned(),
            title: self.src.one_line(title),
            parent: self.open.last().copied(),
            frame,
        });
        let c = (self.clusters.len() - 1) as ClusterId;
        self.open.push(c);
        Some(c)
    }

    fn close(&mut self) -> Option<ClusterId> {
        self.open.pop()
    }

    fn edge(&mut self, edge: Edge) -> Option<()> {
        if self.edges.len() >= MAX_EDGES {
            return None;
        }
        self.edges.push(edge);
        Some(())
    }

    /// Resolve edges to subgraphs (a node named like a subgraph is that subgraph) and lay the
    /// graph out as far as the width allows.
    fn finish(mut self) -> Option<Graph> {
        if self.nodes.is_empty() && self.clusters.is_empty() {
            return None;
        }
        let by_id: HashMap<String, ClusterId> = self
            .clusters
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.id.is_empty())
            .map(|(i, c)| (c.id.clone(), i as ClusterId))
            .collect();
        // old node → its new index, or the subgraph it stands for
        let mut remap: Vec<End> = Vec::with_capacity(self.nodes.len());
        let mut kept = Vec::with_capacity(self.nodes.len());
        for node in self.nodes.drain(..) {
            match by_id.get(&node.id) {
                Some(&c) if !node.id.is_empty() => {
                    // a composite state's description is its title
                    let cluster = &mut self.clusters[c as usize];
                    if *cluster.title == cluster.id && node.label != [node.id.as_str()] {
                        cluster.title = Text::join(&node.label);
                    }
                    remap.push(End::Cluster(c));
                }
                _ => {
                    remap.push(End::Node(kept.len() as NodeId));
                    kept.push(node);
                }
            }
        }
        let fix = |e: End| match e {
            End::Node(n) => remap[n as usize],
            c => c,
        };
        for e in &mut self.edges {
            (e.from, e.to) = (fix(e.from), fix(e.to));
        }
        let mut g = Graph {
            dir: self.dir,
            nodes: kept,
            edges: self.edges,
            clusters: self.clusters,
            topo: topo::Topo::default(),
        };
        g.topo = topo::build(&g)?;
        Some(g)
    }
}

/// Split a line into statements at `;`, outside quotes and brackets.
fn split_statements(line: &str) -> impl Iterator<Item = &str> {
    let mut rest = Some(line);
    std::iter::from_fn(move || {
        let s = rest?;
        let (mut depth, mut quoted) = (0i32, false);
        for (i, c) in s.char_indices() {
            match c {
                '"' => quoted = !quoted,
                '[' | '(' | '{' if !quoted => depth += 1,
                ']' | ')' | '}' if !quoted => depth -= 1,
                ';' if !quoted && depth <= 0 => {
                    rest = Some(&s[i + 1..]);
                    return Some(s[..i].trim());
                }
                _ => {}
            }
        }
        rest = None;
        Some(s.trim())
    })
    .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests;
