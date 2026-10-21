//! The width-independent half of the layout (ADR-35's layered layout), done once at parse.
//! Each subgraph is a [`Level`] of its own, laid out apart from its parent, where it is one
//! item; an edge crossing its frame meets a port item on the border. Per level: cycles broken
//! by DFS, longest-path layers, a dummy item per layer a long edge crosses (the first one its
//! label, if it has one), and a fixed number of barycenter sweeps against crossings, keeping
//! the best order found.

use std::collections::HashMap;

use super::{ClusterId, End, Graph, Line, Mark, NodeId};

/// Barycenter sweeps per level, alternately down and up.
const SWEEPS: usize = 16;
/// The most items (nodes, dummies, ports) all levels together may have.
const MAX_ITEMS: usize = 20_000;
/// A port's end of an edge: the frame, where nothing is drawn.
const BORDER: (End, Mark) = (End::Cluster(ClusterId::MAX), Mark::None);

#[derive(Debug, Default)]
pub(super) struct Topo {
    /// The root first; a child level always comes after its parent.
    pub levels: Vec<Level>,
}

#[derive(Debug, Default)]
pub(super) struct Level {
    pub cluster: Option<ClusterId>,
    pub items: Vec<Item>,
    /// Item indices per layer, in order across.
    pub layers: Vec<Vec<u32>>,
    pub paths: Vec<Path>,
    /// The port item of each port, in the order the parent asked for them.
    pub ports: Vec<u32>,
}

#[derive(Debug)]
pub(super) struct Item {
    pub kind: Kind,
    pub layer: u32,
    /// Edges from a node to itself, drawn beside it.
    pub loops: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Node(NodeId),
    /// A subgraph: the level it is laid out in.
    Child(u32),
    /// Where a long edge crosses a layer.
    Dummy,
    /// Where a long edge's label sits: the edge.
    Label(u32),
    /// Where an edge crosses this level's frame, on its first or last layer.
    Port,
    /// The frame's title, first on the layer along its top border.
    Title,
}

/// One edge's way through a level, an item per layer from the earlier to the later.
#[derive(Debug)]
pub(super) struct Path {
    pub edge: u32,
    pub items: Vec<u32>,
    /// The child's port at the first item, if that is a subgraph.
    pub first_port: u32,
    pub last_port: u32,
    /// The marks drawn at the first and last item, where those are what the edge connects.
    pub start: Mark,
    pub end: Mark,
}

/// A port a parent asks a child level for.
struct PortSpec {
    /// On the last layer (the edge leaves) rather than the first.
    exit: bool,
    /// What the edge connects to inside; `None` for the subgraph itself.
    inner: Option<End>,
    edge: u32,
    /// The mark at `inner`.
    mark: Mark,
}

#[derive(Clone, Copy)]
enum Member {
    Node(NodeId),
    Cluster(ClusterId),
    Port(u32),
    Other,
}

/// An edge between two items of a level.
struct LEdge {
    u: u32,
    v: u32,
    minlen: u32,
    kind: EKind,
    reversed: bool,
    /// The child's port at `u` or `v`, when that is a subgraph.
    u_port: u32,
    v_port: u32,
}

#[derive(Clone, Copy)]
enum EKind {
    Graph(u32),
    /// From an entry port to what it leads to.
    PortIn(u32),
    /// To an exit port.
    PortOut(u32),
}

pub(super) fn build(g: &Graph) -> Option<Topo> {
    let mut t = Topo::default();
    let mut count = 0;
    level(g, None, &[], &mut t, &mut count)?;
    Some(t)
}

/// Build the level of `cluster` (the root for `None`) and, depth first, those inside it.
fn level(
    g: &Graph,
    cluster: Option<ClusterId>,
    specs: &[PortSpec],
    t: &mut Topo,
    count: &mut usize,
) -> Option<u32> {
    let me = t.levels.len() as u32;
    t.levels.push(Level {
        cluster,
        ..Level::default()
    });
    let framed = cluster.is_some();
    let mut members = Vec::new();
    let mut node_item = HashMap::new();
    let mut cluster_item = HashMap::new();
    for (i, n) in g.nodes.iter().enumerate() {
        if n.parent == cluster {
            node_item.insert(i as NodeId, members.len() as u32);
            members.push(Member::Node(i as NodeId));
        }
    }
    for (i, c) in g.clusters.iter().enumerate() {
        if c.parent == cluster && Some(i as ClusterId) != cluster {
            cluster_item.insert(i as ClusterId, members.len() as u32);
            members.push(Member::Cluster(i as ClusterId));
        }
    }
    // the item at this level holding `end`, if it is inside
    let item_of = |end: End| -> Option<u32> {
        let mut c = match end {
            End::Node(n) => {
                let p = g.nodes.get(n as usize)?.parent;
                if p == cluster {
                    return node_item.get(&n).copied();
                }
                p
            }
            End::Cluster(c) if Some(c) == cluster => return None,
            End::Cluster(c) => Some(c),
        };
        for _ in 0..=g.clusters.len() {
            let id = c?;
            let p = g.clusters.get(id as usize)?.parent;
            if p == cluster {
                return cluster_item.get(&id).copied();
            }
            c = p;
        }
        None
    };
    let mut edges = Vec::new();
    let mut loops: HashMap<u32, Vec<u32>> = HashMap::new();
    for (i, e) in g.edges.iter().enumerate() {
        let (Some(a), Some(b)) = (item_of(e.from), item_of(e.to)) else {
            continue;
        };
        if a != b {
            let labeled = e.line != Line::Hidden && e.label.iter().any(|l| !l.is_empty());
            edges.push(LEdge {
                u: a,
                v: b,
                minlen: u32::from(e.minlen.max(1)) + u32::from(labeled),
                kind: EKind::Graph(i as u32),
                reversed: false,
                u_port: 0,
                v_port: 0,
            });
        } else if e.from == e.to && matches!(e.from, End::Node(_)) && e.line != Line::Hidden {
            loops.entry(a).or_default().push(i as u32);
        }
    }
    let mut ports = Vec::with_capacity(specs.len());
    for (k, spec) in specs.iter().enumerate() {
        let port = members.len() as u32;
        members.push(Member::Port(k as u32));
        ports.push(port);
        if let Some(j) = spec.inner.and_then(item_of) {
            let (u, v, kind) = if spec.exit {
                (j, port, EKind::PortOut(k as u32))
            } else {
                (port, j, EKind::PortIn(k as u32))
            };
            edges.push(LEdge {
                u,
                v,
                minlen: 1,
                kind,
                reversed: false,
                u_port: 0,
                v_port: 0,
            });
        }
    }
    let title = framed
        && g.dir.vertical()
        && cluster.is_some_and(|c| !g.clusters[c as usize].title.is_empty());
    if title {
        members.push(Member::Other);
    }
    *count += members.len();
    if *count > MAX_ITEMS {
        return None;
    }
    break_cycles(members.len(), &mut edges);
    let layer = layers(g, &members, &edges, specs, framed, title);

    // what each child is asked for
    let mut child_specs: HashMap<u32, Vec<PortSpec>> = HashMap::new();
    for e in &mut edges {
        let (s, tgt) = if e.reversed { (e.v, e.u) } else { (e.u, e.v) };
        let hidden = matches!(e.kind, EKind::Graph(i) if g.edges[i as usize].line == Line::Hidden);
        if hidden {
            continue;
        }
        for (item, exit) in [(s, true), (tgt, false)] {
            if !matches!(members[item as usize], Member::Cluster(_)) {
                continue;
            }
            let spec = match e.kind {
                EKind::Graph(i) => {
                    let edge = &g.edges[i as usize];
                    let from_inside = item_of(edge.from) == Some(item);
                    let (inner, mark) = if from_inside {
                        (edge.from, edge.head_from)
                    } else {
                        (edge.to, edge.head_to)
                    };
                    PortSpec {
                        exit,
                        inner: Some(inner),
                        edge: i,
                        mark,
                    }
                }
                EKind::PortIn(k) | EKind::PortOut(k) => {
                    let outer = &specs[k as usize];
                    PortSpec {
                        exit,
                        inner: outer.inner,
                        edge: outer.edge,
                        mark: outer.mark,
                    }
                }
            };
            let list = child_specs.entry(item).or_default();
            let port = list.len() as u32;
            list.push(spec);
            if item == e.u {
                e.u_port = port;
            } else {
                e.v_port = port;
            }
        }
    }

    let mut items: Vec<Item> = members
        .iter()
        .enumerate()
        .map(|(i, m)| Item {
            kind: match *m {
                Member::Node(n) => Kind::Node(n),
                Member::Cluster(_) => Kind::Child(0),
                Member::Port(_) => Kind::Port,
                Member::Other => Kind::Title,
            },
            layer: layer[i],
            loops: loops.remove(&(i as u32)).unwrap_or_default(),
        })
        .collect();
    for (i, m) in members.iter().enumerate() {
        if let Member::Cluster(c) = *m {
            let specs = child_specs.remove(&(i as u32)).unwrap_or_default();
            items[i].kind = Kind::Child(level(g, Some(c), &specs, t, count)?);
        }
    }

    // an item per layer a long edge crosses, and each edge's path
    let mut paths = Vec::new();
    for e in &edges {
        let (edge, start, end) = match e.kind {
            EKind::Graph(i) => {
                let edge = &g.edges[i as usize];
                if edge.line == Line::Hidden {
                    continue;
                }
                let (s_end, t_end) = if e.reversed {
                    ((edge.to, edge.head_to), (edge.from, edge.head_from))
                } else {
                    ((edge.from, edge.head_from), (edge.to, edge.head_to))
                };
                (i, s_end, t_end)
            }
            EKind::PortIn(k) => {
                let spec = &specs[k as usize];
                (
                    spec.edge,
                    BORDER,
                    (spec.inner.unwrap_or(BORDER.0), spec.mark),
                )
            }
            EKind::PortOut(k) => {
                let spec = &specs[k as usize];
                (
                    spec.edge,
                    (spec.inner.unwrap_or(BORDER.0), spec.mark),
                    BORDER,
                )
            }
        };
        let (s, tgt, s_port, t_port) = if e.reversed {
            (e.v, e.u, e.v_port, e.u_port)
        } else {
            (e.u, e.v, e.u_port, e.v_port)
        };
        // a mark is drawn here when its end is this item: a node, or a subgraph itself
        let mark_at = |item: u32, (end, mark): (End, Mark)| match (members[item as usize], end) {
            (Member::Node(n), End::Node(m)) if n == m => mark,
            (Member::Cluster(c), End::Cluster(d)) if c == d => mark,
            _ => Mark::None,
        };
        let labeled = matches!(e.kind, EKind::Graph(i) if g.edges[i as usize].label.iter().any(|l| !l.is_empty()));
        let (ls, lt) = (items[s as usize].layer, items[tgt as usize].layer);
        let mut chain = vec![s];
        for (n, l) in (ls + 1..lt).enumerate() {
            let kind = if n == 0 && labeled {
                Kind::Label(edge)
            } else {
                Kind::Dummy
            };
            chain.push(items.len() as u32);
            items.push(Item {
                kind,
                layer: l,
                loops: Vec::new(),
            });
        }
        chain.push(tgt);
        *count += chain.len() - 2;
        if *count > MAX_ITEMS {
            return None;
        }
        paths.push(Path {
            edge,
            items: chain,
            first_port: s_port,
            last_port: t_port,
            start: mark_at(s, start),
            end: mark_at(tgt, end),
        });
    }

    let mut n_layers = items.iter().map(|i| i.layer + 1).max().unwrap_or(0) as usize;
    if framed {
        // the borders are layers even with no port on them
        let inner = items
            .iter()
            .filter(|i| matches!(i.kind, Kind::Node(_) | Kind::Child(_)))
            .map(|i| i.layer as usize)
            .max()
            .unwrap_or(0);
        n_layers = n_layers.max(inner + 2);
    }
    let mut by_layer = vec![Vec::new(); n_layers];
    for (i, it) in items.iter().enumerate() {
        by_layer[it.layer as usize].push(i as u32);
    }
    // the title leads its layer
    for layer in &mut by_layer {
        layer.sort_by_key(|&i| items[i as usize].kind != Kind::Title);
    }
    let lv = Level {
        cluster,
        items,
        layers: by_layer,
        paths,
        ports,
    };
    let lv = order(lv, &t.levels);
    t.levels[me as usize] = lv;
    Some(me)
}

/// Reverse the edges a DFS finds closing a cycle, in item order, so the rest is acyclic.
fn break_cycles(n: usize, edges: &mut [LEdge]) {
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, e) in edges.iter().enumerate() {
        if let EKind::Graph(_) = e.kind {
            out[e.u as usize].push(i);
        }
    }
    // 0 unseen, 1 on the stack, 2 done
    let mut state = vec![0u8; n];
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for root in 0..n {
        if state[root] != 0 {
            continue;
        }
        state[root] = 1;
        stack.push((root, 0));
        while let Some(&mut (v, ref mut next)) = stack.last_mut() {
            if let Some(&ei) = out[v].get(*next) {
                *next += 1;
                let w = edges[ei].v as usize;
                match state[w] {
                    0 => {
                        state[w] = 1;
                        stack.push((w, 0));
                    }
                    1 => edges[ei].reversed = true,
                    _ => {}
                }
            } else {
                state[v] = 2;
                stack.pop();
            }
        }
    }
}

/// Longest-path layers over the edges as oriented, each at least `minlen` apart; a framed
/// level keeps its first and last layer for the ports (and the title).
fn layers(
    g: &Graph,
    members: &[Member],
    edges: &[LEdge],
    specs: &[PortSpec],
    framed: bool,
    title: bool,
) -> Vec<u32> {
    let n = members.len();
    let base = u32::from(framed);
    let mut out: Vec<Vec<(usize, u32)>> = vec![Vec::new(); n];
    let mut indeg = vec![0u32; n];
    for e in edges {
        let (s, t) = if e.reversed { (e.v, e.u) } else { (e.u, e.v) };
        out[s as usize].push((t as usize, e.minlen));
        indeg[t as usize] += 1;
    }
    let is_port = |i: usize| matches!(members[i], Member::Port(_));
    let mut layer: Vec<u32> = (0..n).map(|i| if is_port(i) { 0 } else { base }).collect();
    // Kahn's order; what's left in a cycle can't be, as cycles were broken
    let mut order = Vec::with_capacity(n);
    let mut deg = indeg.clone();
    let mut queue: Vec<usize> = (0..n).filter(|&i| deg[i] == 0).collect();
    while let Some(v) = queue.pop() {
        order.push(v);
        for &(w, len) in &out[v] {
            layer[w] = layer[w].max(layer[v] + len);
            deg[w] -= 1;
            if deg[w] == 0 {
                queue.push(w);
            }
        }
    }
    // a source sits just above what it leads to, not at the top
    for &v in order.iter().rev() {
        if indeg[v] == 0 && !is_port(v) && !out[v].is_empty() {
            let below = out[v]
                .iter()
                .map(|&(w, len)| layer[w].saturating_sub(len))
                .min();
            if let Some(l) = below {
                layer[v] = layer[v].max(l).max(base);
            }
        }
    }
    if framed {
        let exit = (0..n)
            .filter(|&i| !is_port(i))
            .map(|i| layer[i] + 1)
            .max()
            .unwrap_or(1)
            .max(1);
        for (i, m) in members.iter().enumerate() {
            match *m {
                Member::Port(k) => {
                    layer[i] = if specs[k as usize].exit { exit } else { 0 };
                }
                Member::Other if title => {
                    layer[i] = if g.dir == super::Dir::Up { exit } else { 0 };
                }
                _ => {}
            }
        }
    }
    layer
}

/// Order each layer by barycenter sweeps, down then up, keeping the order with the fewest
/// crossings. A subgraph's ports count at their place along its border.
fn order(mut lv: Level, levels: &[Level]) -> Level {
    let n = lv.items.len();
    // where along a child's border each of its ports is, -0.5..0.5
    let port_frac = |item: u32, port: u32| -> f64 {
        let Kind::Child(l) = lv.items[item as usize].kind else {
            return 0.0;
        };
        let Some(child) = levels.get(l as usize) else {
            return 0.0;
        };
        let Some(&p) = child.ports.get(port as usize) else {
            return 0.0;
        };
        let layer = &child.layers[child.items[p as usize].layer as usize];
        let at = layer.iter().position(|&x| x == p).unwrap_or(0);
        (at as f64 + 0.5) / layer.len() as f64 - 0.5
    };
    // (neighbor, its port's place, mine)
    let mut up: Vec<Vec<(u32, f64, f64)>> = vec![Vec::new(); n];
    let mut down: Vec<Vec<(u32, f64, f64)>> = vec![Vec::new(); n];
    for p in &lv.paths {
        let last = p.items.len() - 1;
        for (k, w) in p.items.windows(2).enumerate() {
            let fa = if k == 0 {
                port_frac(w[0], p.first_port)
            } else {
                0.0
            };
            let fb = if k + 1 == last {
                port_frac(w[1], p.last_port)
            } else {
                0.0
            };
            down[w[0] as usize].push((w[1], fb, fa));
            up[w[1] as usize].push((w[0], fa, fb));
        }
    }
    let mut pos = vec![0f64; n];
    let set_pos = |layers: &[Vec<u32>], pos: &mut [f64]| {
        for layer in layers {
            for (i, &x) in layer.iter().enumerate() {
                pos[x as usize] = i as f64;
            }
        }
    };
    set_pos(&lv.layers, &mut pos);
    let mut best = lv.layers.clone();
    let mut best_crossings = crossings(&lv, &pos, &down);
    for sweep in 0..SWEEPS {
        if best_crossings == 0 {
            break;
        }
        let downward = sweep % 2 == 0;
        let n_layers = lv.layers.len();
        let range: Vec<usize> = if downward {
            (1..n_layers).collect()
        } else {
            (0..n_layers.saturating_sub(1)).rev().collect()
        };
        for l in range {
            let nbrs = if downward { &up } else { &down };
            let mut keyed: Vec<(f64, u32)> = lv.layers[l]
                .iter()
                .map(|&x| {
                    if lv.items[x as usize].kind == Kind::Title {
                        return (f64::NEG_INFINITY, x);
                    }
                    let ns = &nbrs[x as usize];
                    if ns.is_empty() {
                        return (pos[x as usize], x);
                    }
                    let sum: f64 = ns
                        .iter()
                        .map(|&(o, fo, fm)| pos[o as usize] + fo - fm)
                        .sum();
                    (sum / ns.len() as f64, x)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (i, &(_, x)) in keyed.iter().enumerate() {
                lv.layers[l][i] = x;
                pos[x as usize] = i as f64;
            }
        }
        let c = crossings(&lv, &pos, &down);
        if c < best_crossings {
            best_crossings = c;
            best.clone_from(&lv.layers);
        }
    }
    lv.layers = best;
    lv
}

/// Edge crossings between all adjacent layers, counted as inversions.
fn crossings(lv: &Level, pos: &[f64], down: &[Vec<(u32, f64, f64)>]) -> usize {
    let mut total = 0;
    let mut pairs: Vec<(f64, f64)> = Vec::new();
    for layer in &lv.layers {
        pairs.clear();
        for &x in layer {
            for &(o, fo, fm) in &down[x as usize] {
                pairs.push((pos[x as usize] + fm, pos[o as usize] + fo));
            }
        }
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        let mut lower: Vec<f64> = pairs.iter().map(|p| p.1).collect();
        total += inversions(&mut lower);
    }
    total
}

/// Pairs `i < j` with `v[i] > v[j]`, by merge sort.
fn inversions(v: &mut [f64]) -> usize {
    if v.len() < 2 {
        return 0;
    }
    let mid = v.len() / 2;
    let mut count = inversions(&mut v[..mid]) + inversions(&mut v[mid..]);
    let mut merged = Vec::with_capacity(v.len());
    let (mut i, mut j) = (0, mid);
    while i < mid && j < v.len() {
        if v[j] < v[i] {
            count += mid - i;
            merged.push(v[j]);
            j += 1;
        } else {
            merged.push(v[i]);
            i += 1;
        }
    }
    merged.extend_from_slice(&v[i..mid]);
    merged.extend_from_slice(&v[j..]);
    v.copy_from_slice(&merged);
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inversions_count_crossings() {
        assert_eq!(inversions(&mut [0.0, 1.0, 2.0]), 0);
        assert_eq!(inversions(&mut [2.0, 1.0, 0.0]), 3);
        assert_eq!(inversions(&mut [1.0, 1.0]), 0);
    }
}
