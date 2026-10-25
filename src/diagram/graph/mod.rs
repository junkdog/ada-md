//! Flowcharts, state and class diagrams (ada-mermaid, DG-3, DG-11): all parse into one graph IR, which
//! `topo` lays out as far as it can without knowing the width, once, at parse. Drawing
//! (`place`, then `paint`) sizes the labels, places and routes everything, and draws it, for
//! each width; labels wrap narrower until the drawing fits, then it scrolls.
//!
//! A subgraph (or composite state) is laid out on its own, as a block in its parent's layout,
//! with ports on its border where edges cross it, so frames never overlap anything.

mod class;
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

/// A flowchart, state or class diagram, with its width-independent layout.
#[derive(Debug)]
pub struct Graph {
    pub dir: Dir,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub clusters: Vec<Cluster>,
    /// Class members' types and parameters, as byte ranges of their text, each member's in
    /// order; one list for the diagram, so that it grows a few times rather than once a class.
    pub member_parts: Vec<(u32, u32, Part)>,
    /// A class diagram: its boxes, cardinalities and parts are worked out only then.
    classes: bool,
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
    /// A class diagram's annotation and members, for [`Shape::Class`].
    pub class: Option<Box<Class>>,
}

/// What a class box shows besides its name.
#[derive(Debug, Default)]
pub struct Class {
    /// `<<interface>>`, as `«interface»`.
    pub annotation: Vec<Text>,
    pub attributes: Vec<Member>,
    pub methods: Vec<Member>,
}

/// A class member as Mermaid shows it: generics as `<T>`, a return type after ` : `.
#[derive(Debug)]
pub struct Member {
    pub text: Text,
    pub classifier: Classifier,
    /// Its types and parameters: which of [`Graph::member_parts`] are its.
    pub parts: std::ops::Range<u32>,
    /// Its width, and that of the widest piece it can't break in, measured once; the piece
    /// only when the member is wider than the narrowest cap, so that it may wrap.
    pub width: usize,
    pub piece: usize,
}

impl Member {
    fn new(text: Text, classifier: Classifier) -> Member {
        let width = super::width(&text);
        Member {
            piece: if width > place::CLASS_CAP {
                longest_piece(&text)
            } else {
                width
            },
            width,
            text,
            classifier,
            parts: 0..0,
        }
    }
}

impl Graph {
    /// Member `m`'s types and parameters.
    pub fn parts_of(&self, m: &Member) -> &[(u32, u32, Part)] {
        &self.member_parts[m.parts.start as usize..m.parts.end as usize]
    }
}

impl Class {
    /// Member `i`, counting the attributes, then the methods.
    pub fn member(&self, i: usize) -> &Member {
        match self.attributes.get(i) {
            Some(m) => m,
            None => &self.methods[i - self.attributes.len()],
        }
    }
}

/// The widest piece (as [`pieces`](super::pieces) splits text) a member can't break in, its
/// visibility joined to the word after it. One pass over the characters.
fn longest_piece(text: &str) -> usize {
    use unicode_width::UnicodeWidthChar;
    let visibility = text.starts_with(['+', '-', '#', '~']);
    // the widest so far, the piece being read (its width and characters), how many ended,
    // and a lone visibility's width, carried onto the next piece
    let (mut best, mut width, mut chars, mut ended, mut carry) = (0, 0, 0, 0, 0);
    let mut end = |width: &mut usize, chars: &mut usize| {
        if *chars == 0 {
            return;
        }
        if ended == 0 && visibility && *width == 1 {
            carry = 1;
        } else {
            best = best.max(*width + carry);
            carry = 0;
        }
        ended += 1;
        (*width, *chars) = (0, 0);
    };
    for c in text.chars() {
        if c == ' ' {
            end(&mut width, &mut chars);
            continue;
        }
        width += c.width().unwrap_or(0);
        chars += 1;
        let breaks = match c {
            // an opening bracket first in a piece stays with what it opens
            '(' | '{' | '[' => chars > 1,
            '/' | '-' | ',' => true,
            _ => false,
        };
        if breaks {
            end(&mut width, &mut chars);
        }
    }
    end(&mut width, &mut chars);
    best.max(carry)
}

/// What a piece of a member is, for its style; the rest is its name, visibility and punctuation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Type,
    Param,
}

/// UML's static (underlined) and abstract (italic) members: `$` and `*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Classifier {
    None,
    Static,
    Abstract,
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
    /// A class diagram's class: its name over compartments of members.
    Class,
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
    /// A class diagram's part unrelated to the rest: laid out on its own, not drawn, so the
    /// parts can be set in rows.
    Hidden,
}

impl Frame {
    /// Whether a level in this frame (`None` the root) has a border: room around it, and its
    /// first and last layers for ports and a title.
    fn drawn(frame: Option<Frame>) -> bool {
        frame.is_some_and(|f| f != Frame::Hidden)
    }
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
    /// A class relation's cardinality at `from` and at `to` (`A "1" --> "*" B`); empty for none.
    pub cards: [Text; 2],
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
    /// A class diagram's hollow triangle: inheritance (`<|--`) and realization (`<|..`).
    Triangle,
    /// Composition (`*--`).
    Diamond,
    /// Aggregation (`o--`).
    HollowDiamond,
}

/// Parse a flowchart (`flowchart`, `graph`), a state or a class diagram. `header` is the first
/// line.
pub(super) fn parse<'a>(
    kind: &str,
    header: &'a str,
    lines: impl Iterator<Item = (u32, &'a str)>,
    src: Source,
) -> Option<Graph> {
    let b = match kind {
        "stateDiagram" | "stateDiagram-v2" => state::parse(lines, src)?,
        "classDiagram" | "classDiagram-v2" => class::parse(lines, src)?,
        _ => flowchart::parse(header, lines, src)?,
    };
    b.finish()
}

/// Draw `g` to fit `avail` columns where it can: labels wrap at each width in
/// [`place::CAPS`] in turn until one fits (a class diagram's parts set in rows as wide as
/// `avail`); past the last it scrolls.
pub(super) fn draw(g: &Graph, avail: usize, ascii: bool) -> Option<Drawn> {
    let mut best: Option<place::Placed> = None;
    // a cap no label reaches draws what the one before did
    let widest = g
        .nodes
        .iter()
        .flat_map(|n| {
            let class = n.class.iter().flat_map(|c| {
                let members = c.attributes.iter().chain(&c.methods).map(|m| &m.text);
                c.annotation.iter().chain(members)
            });
            n.label
                .iter()
                .chain(n.note.iter().flat_map(|n| &n.text))
                .chain(class)
        })
        .chain(g.edges.iter().flat_map(|e| &e.label))
        .map(|l| super::width(l))
        .max()
        .unwrap_or(0);
    let floor = if g.classes { place::CLASS_CAP } else { 0 };
    // the caps worth trying: the first, then those some label reaches, down to the floor
    let caps: Vec<usize> = (place::CAPS.iter().enumerate())
        .filter(|&(i, &cap)| i == 0 || (cap < widest && cap >= floor))
        .map(|(_, &cap)| cap)
        .collect();
    let mut try_cap = |cap: usize, bail: bool| -> Option<bool> {
        let placed = place::place(g, &g.topo, cap, avail, bail)?;
        if placed.over {
            return Some(false);
        }
        let fits = placed.canvas_width(g.dir) <= avail;
        let narrower = best
            .as_ref()
            .is_none_or(|b| placed.canvas_width(g.dir) < b.canvas_width(g.dir));
        if fits || narrower {
            best = Some(placed);
        }
        Some(fits)
    };
    // widest first; each but the last gives up as soon as its boxes alone are too wide, so a
    // diagram that scrolls is sized at every cap but placed only at the last
    for (k, &cap) in caps.iter().enumerate() {
        if try_cap(cap, k + 1 < caps.len())? {
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

/// The node ids' hasher: short keys, looked up once a mention, so a multiply and a rotate a
/// word (FxHash's) beat SipHash. No defense against crafted collisions is needed: a diagram has
/// at most [`MAX_NODES`] ids.
#[derive(Clone, Copy, Default)]
struct Ids;

impl std::hash::BuildHasher for Ids {
    type Hasher = IdHasher;

    fn build_hasher(&self) -> IdHasher {
        IdHasher(0)
    }
}

struct IdHasher(u64);

impl std::hash::Hasher for IdHasher {
    fn write(&mut self, bytes: &[u8]) {
        const K: u64 = 0x51_7c_c1_b7_27_22_0a_95;
        let (words, rest) = bytes.as_chunks::<8>();
        for &word in words {
            self.0 = (self.0.rotate_left(5) ^ u64::from_le_bytes(word)).wrapping_mul(K);
        }
        let mut tail = [0u8; 8];
        tail[..rest.len()].copy_from_slice(rest);
        self.0 =
            (self.0.rotate_left(5) ^ u64::from_le_bytes(tail) ^ rest.len() as u64).wrapping_mul(K);
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// What both parsers build the graph with.
#[derive(Default)]
struct Builder {
    src: Source,
    dir: Dir,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    clusters: Vec<Cluster>,
    ids: HashMap<String, NodeId, Ids>,
    /// The subgraphs open where the parser is, innermost last.
    open: Vec<ClusterId>,
    /// Set the unrelated parts apart, to be drawn in rows (class diagrams).
    parts: bool,
    /// Class members' types and parameters, for [`Graph::member_parts`].
    member_parts: Vec<(u32, u32, Part)>,
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
            class: None,
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
            class: None,
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
            classes: self.parts,
            member_parts: self.member_parts,
            topo: topo::Topo::default(),
        };
        if self.parts {
            parts(&mut g);
        }
        g.topo = topo::build(&g)?;
        Some(g)
    }
}

/// Put each part of `g` unrelated to the rest (what no edge joins: nodes and subgraphs at the
/// top level, with what they hold) in a hidden frame of its own, in the order of their first
/// nodes, so that the drawing can set them in rows rather than all in one.
fn parts(g: &mut Graph) {
    let (n, k) = (g.nodes.len(), g.clusters.len());
    // the top-level subgraph `c` is in
    let top = |mut c: ClusterId| {
        for _ in 0..=k {
            match g.clusters[c as usize].parent {
                Some(p) => c = p,
                None => break,
            }
        }
        c
    };
    // items: nodes 0..n, subgraphs n..n + k
    let item = |e: End| match e {
        End::Node(x) => match g.nodes[x as usize].parent {
            None => x as usize,
            Some(c) => n + top(c) as usize,
        },
        End::Cluster(c) => n + top(c) as usize,
    };
    let mut parent: Vec<usize> = (0..n + k).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    for e in &g.edges {
        let (a, b) = (item(e.from), item(e.to));
        let (a, b) = (find(&mut parent, a), find(&mut parent, b));
        parent[a] = b;
    }
    // the top-level items, by their first node, then subgraphs holding none
    let mut tops: Vec<usize> = Vec::new();
    for x in (0..n).map(|x| item(End::Node(x as NodeId))) {
        if !tops.contains(&x) {
            tops.push(x);
        }
    }
    for c in (0..k).filter(|&c| g.clusters[c].parent.is_none()) {
        if !tops.contains(&(n + c)) {
            tops.push(n + c);
        }
    }
    let mut groups: Vec<(usize, Vec<usize>)> = Vec::new();
    for &t in &tops {
        let root = find(&mut parent, t);
        match groups.iter_mut().find(|(r, _)| *r == root) {
            Some((_, list)) => list.push(t),
            None => groups.push((root, vec![t])),
        }
    }
    if groups.len() < 2 {
        return;
    }
    for (_, list) in groups {
        let c = g.clusters.len() as ClusterId;
        g.clusters.push(Cluster {
            id: String::new(),
            title: Text::default(),
            parent: None,
            frame: Frame::Hidden,
        });
        for t in list {
            if t < n {
                g.nodes[t].parent = Some(c);
            } else {
                g.clusters[t - n].parent = Some(c);
            }
        }
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
