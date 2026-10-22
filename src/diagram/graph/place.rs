//! The width-dependent half of the layout: labels wrapped at a cap, items sized, placed across
//! their layers and routed, level by level from the innermost subgraph out.
//!
//! Coordinates are abstract: `r` runs along the edges (down in `TD`, right in `LR`), `c` across
//! the layers; [`paint`](super::paint) turns them into rows and columns. Across, each layer is
//! placed in turn, a few sweeps down and up, as close as the items' minimum gaps allow to where
//! their neighbors pull them (a weighted isotonic regression, solved exactly per layer by pooling
//! adjacent violators), so chains run straight. Between two layers, edges bend once, on a track
//! of their own in the channel; edges sharing a port share a track, and the tracks are ordered so
//! a line leaving or entering a port never runs in another's lane (a port moves when no order
//! can manage it), and doesn't cross another's track where it can help it.

use std::collections::HashMap;

use super::topo::{Kind, Level, Topo};
use super::{Dir, Graph, Mark, Shape};
use crate::diagram::{MAX_CELLS, Text, width, wrap};

/// The label widths tried in turn until the drawing fits.
pub(super) const CAPS: &[usize] = &[48, 36, 28, 22, 16, 12];
/// Sweeps placing the layers across.
const SWEEPS: usize = 10;

/// Everything drawn, at one cap.
#[derive(Debug)]
pub(super) struct Placed {
    pub levels: Vec<Geo>,
    /// Each node's label, wrapped.
    pub nodes: Vec<Vec<Text>>,
    pub notes: Vec<Vec<Text>>,
    /// Each edge's label, wrapped.
    pub edges: Vec<Vec<Text>>,
}

impl Placed {
    pub fn canvas_width(&self, dir: Dir) -> usize {
        let root = &self.levels[0];
        if dir.vertical() { root.cs } else { root.rs }
    }

    pub fn canvas_height(&self, dir: Dir) -> usize {
        let root = &self.levels[0];
        if dir.vertical() { root.rs } else { root.cs }
    }
}

/// A level's geometry, in its own coordinates.
#[derive(Debug, Default)]
pub(super) struct Geo {
    pub rs: usize,
    pub cs: usize,
    pub items: Vec<ItemGeo>,
    pub segs: Vec<Seg>,
    /// Across, where each port is.
    pub ports: Vec<usize>,
    /// Each layer's `(start, size)` along.
    pub bands: Vec<(usize, usize)>,
}

/// An item's footprint, and the box inside it that edges meet (a node's notes and loops are
/// beside its box).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ItemGeo {
    pub r: usize,
    pub c: usize,
    pub rs: usize,
    pub cs: usize,
    pub box_r: usize,
    pub box_c: usize,
    pub box_rs: usize,
    pub box_cs: usize,
}

/// One edge between two adjacent layers: along `cu` from `r0`, across on its track, along `cv`
/// to `r1`. A mark at either end is drawn on the end cell.
#[derive(Clone, Copy, Debug)]
pub(super) struct Seg {
    pub path: u32,
    /// Whether it leaves or enters a label, which its line must reach.
    pub from_label: bool,
    pub to_label: bool,
    pub r0: usize,
    pub cu: usize,
    pub track: Option<usize>,
    pub cv: usize,
    pub r1: usize,
    pub start: Mark,
    pub end: Mark,
}

/// One edge of a path between two adjacent layers, before routing: from item `a` to item `b`,
/// meeting each at an anchor across it (`oa`, `ob`), relative to the item.
#[derive(Clone, Copy, Debug)]
struct Link {
    a: u32,
    b: u32,
    oa: usize,
    ob: usize,
    path: u32,
}

pub(super) fn place(g: &Graph, t: &Topo, cap: usize) -> Option<Placed> {
    let vertical = g.dir.vertical();
    let nodes: Vec<Vec<Text>> = g.nodes.iter().map(|n| wrap_lines(&n.label, cap)).collect();
    let notes: Vec<Vec<Text>> = g
        .nodes
        .iter()
        .map(|n| {
            n.note
                .as_ref()
                .map_or_else(Vec::new, |n| wrap_lines(&n.text, cap))
        })
        .collect();
    let edges: Vec<Vec<Text>> = g.edges.iter().map(|e| wrap_lines(&e.label, cap)).collect();
    let mut p = Placed {
        levels: (0..t.levels.len()).map(|_| Geo::default()).collect(),
        nodes,
        notes,
        edges,
    };
    for l in (0..t.levels.len()).rev() {
        let geo = level(g, t, &p, l, vertical)?;
        p.levels[l] = geo;
    }
    let root = &p.levels[0];
    (root.rs.saturating_mul(root.cs) <= MAX_CELLS).then_some(p)
}

fn wrap_lines(lines: &[Text], cap: usize) -> Vec<Text> {
    let out: Vec<Text> = lines.iter().flat_map(|l| wrap(l, cap)).collect();
    if out.iter().all(|l| l.is_empty()) {
        Vec::new()
    } else {
        out
    }
}

fn widest(lines: &[Text]) -> usize {
    lines.iter().map(|l| width(l)).max().unwrap_or(0)
}

/// Is it a node's box or a subgraph, rather than a line, a label or a port?
fn solid(k: Kind) -> bool {
    matches!(k, Kind::Node(_) | Kind::Child(_))
}

fn thin(k: Kind) -> bool {
    matches!(k, Kind::Dummy | Kind::Label(_) | Kind::Port)
}

/// The least gap across between two neighbors in a layer.
fn gap(vertical: bool, a: Kind, b: Kind) -> usize {
    if !vertical {
        1
    } else if solid(a) && solid(b) {
        3
    } else if matches!((a, b), (Kind::Label(_), Kind::Label(_))) {
        // labels side by side mustn't read as one
        3
    } else if matches!(a, Kind::Label(_) | Kind::Title) || matches!(b, Kind::Label(_) | Kind::Title)
    {
        2
    } else {
        1
    }
}

/// How hard an edge between `a` and `b` pulls them into line: long edges hardest.
fn weight(a: Kind, b: Kind) -> f64 {
    match (thin(a), thin(b)) {
        (true, true) => 8.0,
        (true, _) | (_, true) => 2.0,
        _ => 1.0,
    }
}

/// The canvas size of a node's box: `(width, height)`.
fn box_size(
    shape: Shape,
    lines: &[Text],
    vertical: bool,
    degree: usize,
    loops: bool,
) -> (usize, usize) {
    match shape {
        Shape::Start | Shape::End | Shape::Choice => (1, 1),
        Shape::Fork => {
            let len = (2 * degree + 1).max(if vertical { 5 } else { 3 });
            if vertical { (len, 1) } else { (1, len) }
        }
        _ => {
            let h = lines.len().max(1) + 2;
            let h = if loops && vertical { h.max(4) } else { h };
            (widest(lines) + 4, h)
        }
    }
}

fn level(g: &Graph, t: &Topo, p: &Placed, l: usize, vertical: bool) -> Option<Geo> {
    let lv = &t.levels[l];
    let framed = lv.cluster.is_some();
    let n = lv.items.len();
    let kinds: Vec<Kind> = lv.items.iter().map(|i| i.kind).collect();
    // how many edges meet each item, on its busier side
    let mut degree = vec![(0usize, 0usize); n];
    for path in &lv.paths {
        if let (Some(&a), Some(&b)) = (path.items.first(), path.items.last()) {
            degree[a as usize].1 += 1;
            degree[b as usize].0 += 1;
        }
    }
    let mut geo: Vec<ItemGeo> = Vec::with_capacity(n);
    for (i, item) in lv.items.iter().enumerate() {
        geo.push(match item.kind {
            Kind::Node(nd) => {
                let loop_label: Vec<Text> = item
                    .loops
                    .iter()
                    .flat_map(|&e| p.edges[e as usize].iter().cloned())
                    .collect();
                node_geo(
                    g,
                    p,
                    nd as usize,
                    vertical,
                    degree[i].0.max(degree[i].1),
                    &item.loops,
                    &loop_label,
                )
            }
            Kind::Child(c) => {
                let child = &p.levels[c as usize];
                square(child.rs, child.cs)
            }
            Kind::Label(e) => {
                let lines = &p.edges[e as usize];
                let (w, h) = (widest(lines).max(1), lines.len().max(1));
                if vertical {
                    square(h, w)
                } else {
                    square(w + 2, h)
                }
            }
            Kind::Title => {
                let title = lv
                    .cluster
                    .map_or("", |c| g.clusters[c as usize].title.s.as_str());
                square(1, width(title) + 4)
            }
            Kind::Dummy | Kind::Port => square(1, 1),
        });
    }
    // where an edge meets an item, across, relative to the item
    let anchor = |item: u32, port: u32| -> usize {
        let it = &geo[item as usize];
        match kinds[item as usize] {
            Kind::Node(_) => it.box_c + it.box_cs / 2,
            Kind::Child(c) => p.levels[c as usize]
                .ports
                .get(port as usize)
                .copied()
                .unwrap_or(0),
            Kind::Label(e) if vertical => label_anchor(&p.edges[e as usize], it.cs),
            Kind::Label(_) => it.cs / 2,
            _ => 0,
        }
    };
    let mut segs: Vec<Link> = Vec::new();
    for (pi, path) in lv.paths.iter().enumerate() {
        let last = path.items.len() - 1;
        for (k, w) in path.items.windows(2).enumerate() {
            let pa = if k == 0 { path.first_port } else { 0 };
            let pb = if k + 1 == last { path.last_port } else { 0 };
            segs.push(Link {
                a: w[0],
                b: w[1],
                oa: anchor(w[0], pa),
                ob: anchor(w[1], pb),
                path: pi as u32,
            });
        }
    }
    // a box with several edges on one side spreads them along it, in the order of their
    // other ends, so those can line up with a port each
    let mut at = vec![0usize; n];
    for layer in &lv.layers {
        for (k, &it) in layer.iter().enumerate() {
            at[it as usize] = k;
        }
    }
    let mut sides: HashMap<(u32, bool), Vec<usize>> = HashMap::new();
    for (s, &Link { a, b, .. }) in segs.iter().enumerate() {
        if let Kind::Node(_) = kinds[a as usize] {
            sides.entry((a, true)).or_default().push(s);
        }
        if let Kind::Node(_) = kinds[b as usize] {
            sides.entry((b, false)).or_default().push(s);
        }
    }
    for ((item, out), mut list) in sides {
        if list.len() < 2 {
            continue;
        }
        let other = |s: usize| {
            let Link { a, b, oa, ob, .. } = segs[s];
            if out {
                (at[b as usize], ob)
            } else {
                (at[a as usize], oa)
            }
        };
        list.sort_by_key(|&s| other(s));
        let it = &geo[item as usize];
        let fork = matches!(kinds[item as usize], Kind::Node(nd) if g.nodes[nd as usize].shape == Shape::Fork);
        let (lo, room) = match it.box_cs {
            _ if fork => (it.box_c, it.box_cs),
            w if w >= 3 => (it.box_c + 1, w - 2),
            _ => continue,
        };
        let m = list.len();
        for (k, &s) in list.iter().enumerate() {
            let off = lo + (2 * k + 1) * room / (2 * m);
            if out {
                segs[s].oa = off;
            } else {
                segs[s].ob = off;
            }
        }
    }
    let mut pos = across(lv, &geo, &kinds, &segs, vertical);
    snap(g, lv, &geo, &kinds, &segs, vertical, &mut pos);
    // the title sits at the frame's left, ahead of the ports on its border
    if let Some(ti) = kinds.iter().position(|&k| k == Kind::Title) {
        let others = (0..n).filter(|&i| i != ti).map(|i| pos[i]).min();
        pos[ti] = others.unwrap_or(0);
        let layer = &lv.layers[lv.items[ti].layer as usize];
        for w in layer.windows(2) {
            let (a, b) = (w[0] as usize, w[1] as usize);
            let least = pos[a] + (geo[a].cs + gap(vertical, kinds[a], kinds[b])) as i64;
            pos[b] = pos[b].max(least);
        }
    }
    let pad = if !framed {
        0
    } else if vertical {
        2
    } else {
        1
    };
    let min = pos.iter().copied().min().unwrap_or(0);
    for (i, it) in geo.iter_mut().enumerate() {
        it.c = usize::try_from(pos[i] - min).ok()? + pad;
    }
    let cs = geo.iter().map(|it| it.c + it.cs).max().unwrap_or(0) + pad;
    route(g, lv, geo, &kinds, &segs, framed, vertical, cs)
}

/// Where a line meets a label `cs` wide, across, in `TD` and `BT`: the column nearest its middle with
/// a character on both its first and last row (each centered), so the line never seems to run
/// through a gap between words.
fn label_anchor(lines: &[Text], cs: usize) -> usize {
    let mid = cs / 2;
    let (Some(first), Some(last)) = (lines.first(), lines.last()) else {
        return mid;
    };
    let inked = |line: &str, col: usize| {
        let x0 = cs.saturating_sub(width(line)) / 2;
        let mut x = x0;
        for c in line.chars() {
            let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            if col >= x && col < x + w.max(1) {
                return c != ' ';
            }
            x += w;
        }
        false
    };
    (0..=mid)
        .flat_map(|d| [mid - d, mid + d])
        .filter(|&c| c < cs)
        .find(|&c| inked(first, c) && inked(last, c))
        .unwrap_or(mid)
}

fn square(rs: usize, cs: usize) -> ItemGeo {
    ItemGeo {
        rs,
        cs,
        box_rs: rs,
        box_cs: cs,
        ..ItemGeo::default()
    }
}

/// A node's footprint: a note before or after its box, across, and its loops after it.
fn node_geo(
    g: &Graph,
    p: &Placed,
    n: usize,
    vertical: bool,
    degree: usize,
    loops: &[u32],
    loop_label: &[Text],
) -> ItemGeo {
    let node = &g.nodes[n];
    let lines = &p.nodes[n];
    let has_loops = !loops.is_empty();
    let (w, h) = box_size(node.shape, lines, vertical, degree, has_loops);
    let (w, h) = if has_loops && vertical {
        (w, h.max(loop_label.len() + 2))
    } else {
        (w, h)
    };
    let (box_rs, box_cs) = if vertical { (h, w) } else { (w, h) };
    let (mut before, mut after, mut rs) = (0, 0, box_rs);
    if has_loops {
        let lw = widest(loop_label);
        if vertical {
            after += 2 + if lw > 0 { 1 + lw } else { 0 };
        } else {
            after += 2;
            rs = rs.max(6 + lw);
        }
    }
    if let Some(note) = &node.note {
        let text = &p.notes[n];
        let (nw, nh) = (widest(text) + 4, text.len().max(1) + 2);
        let (n_rs, n_cs) = if vertical { (nh, nw + 2) } else { (nw, nh + 1) };
        rs = rs.max(n_rs);
        if note.after {
            after += n_cs;
        } else {
            before += n_cs;
        }
    }
    ItemGeo {
        r: 0,
        c: 0,
        rs,
        cs: before + box_cs + after,
        box_r: (rs - box_rs) / 2,
        box_c: before,
        box_rs,
        box_cs,
    }
}

/// Place each layer's items across: packed first, then swept down and up, each layer as near
/// as its gaps allow to where its edges pull it.
fn across(lv: &Level, geo: &[ItemGeo], kinds: &[Kind], segs: &[Link], vertical: bool) -> Vec<i64> {
    let n = lv.items.len();
    // (neighbor, its anchor, mine, weight)
    let mut up: Vec<Vec<(u32, i64, i64, f64)>> = vec![Vec::new(); n];
    let mut down: Vec<Vec<(u32, i64, i64, f64)>> = vec![Vec::new(); n];
    for &Link { a, b, oa, ob, .. } in segs {
        let w = weight(kinds[a as usize], kinds[b as usize]);
        down[a as usize].push((b, ob as i64, oa as i64, w));
        up[b as usize].push((a, oa as i64, ob as i64, w));
    }
    let mut pos = vec![0i64; n];
    let seps: Vec<Vec<i64>> = lv
        .layers
        .iter()
        .map(|layer| {
            layer
                .windows(2)
                .map(|w| {
                    let (a, b) = (w[0] as usize, w[1] as usize);
                    (geo[a].cs + gap(vertical, kinds[a], kinds[b])) as i64
                })
                .collect()
        })
        .collect();
    for (layer, sep) in lv.layers.iter().zip(&seps) {
        let mut x = 0;
        for (k, &it) in layer.iter().enumerate() {
            pos[it as usize] = x;
            x += sep.get(k).copied().unwrap_or(0);
        }
    }
    let n_layers = lv.layers.len();
    let mut desired = Vec::new();
    for sweep in 0..SWEEPS {
        let both = sweep + 2 >= SWEEPS;
        let downward = sweep % 2 == 0;
        let order: Vec<usize> = if downward || both {
            (0..n_layers).collect()
        } else {
            (0..n_layers).rev().collect()
        };
        for l in order {
            let layer = &lv.layers[l];
            desired.clear();
            for &it in layer {
                let (mut sw, mut swd) = (0.0, 0.0);
                let mut pull = |list: &[(u32, i64, i64, f64)]| {
                    for &(o, oo, om, w) in list {
                        sw += w;
                        swd += w * (pos[o as usize] + oo - om) as f64;
                    }
                };
                if downward || both {
                    pull(&up[it as usize]);
                }
                if !downward || both {
                    pull(&down[it as usize]);
                }
                if sw == 0.0 {
                    desired.push((pos[it as usize] as f64, 1e-3));
                } else {
                    desired.push((swd / sw, sw));
                }
            }
            let placed = pava(&seps[l], &desired);
            for (k, &it) in layer.iter().enumerate() {
                pos[it as usize] = placed[k];
            }
        }
    }
    pos
}

/// The columns an item offers an edge, relative to the item: a box's span of ports, or the
/// one place a line, label, port or subgraph's port is.
fn offers(g: &Graph, kind: Kind, it: &ItemGeo, anchor: usize) -> (i64, i64) {
    match kind {
        Kind::Node(n) => {
            let lo = it.box_c as i64;
            let hi = lo + it.box_cs as i64 - 1;
            if g.nodes[n as usize].shape == Shape::Fork {
                (lo, hi)
            } else if it.box_cs >= 3 {
                (lo + 1, hi - 1)
            } else {
                (lo + it.box_cs as i64 / 2, lo + it.box_cs as i64 / 2)
            }
        }
        _ => (anchor as i64, anchor as i64),
    }
}

/// Move items, within the room their layer leaves them, to where more of their edges can run
/// straight: a box straight over a line when the line meets its span of ports, two boxes when
/// their spans overlap. Two passes, down then up.
fn snap(
    g: &Graph,
    lv: &Level,
    geo: &[ItemGeo],
    kinds: &[Kind],
    segs: &[Link],
    vertical: bool,
    pos: &mut [i64],
) {
    let n = lv.items.len();
    // (neighbor, its anchor, mine, weight)
    let mut nbrs: Vec<Vec<(u32, usize, usize, f64)>> = vec![Vec::new(); n];
    for &Link { a, b, oa, ob, .. } in segs {
        let w = weight(kinds[a as usize], kinds[b as usize]);
        nbrs[a as usize].push((b, ob, oa, w));
        nbrs[b as usize].push((a, oa, ob, w));
    }
    let score = |x: usize, p: i64, pos: &[i64]| -> f64 {
        nbrs[x]
            .iter()
            .filter(|&&(o, oo, om, _)| {
                let (xl, xh) = offers(g, kinds[x], &geo[x], om);
                let (ol, oh) = offers(g, kinds[o as usize], &geo[o as usize], oo);
                let o = pos[o as usize];
                p + xl <= o + oh && o + ol <= p + xh
            })
            .map(|e| e.3)
            .sum()
    };
    let n_layers = lv.layers.len();
    let order: Vec<usize> = (0..n_layers).chain((0..n_layers).rev()).collect();
    for l in order.into_iter().chain(0..n_layers) {
        let layer = &lv.layers[l];
        for (k, &it) in layer.iter().enumerate() {
            let x = it as usize;
            let lo = k.checked_sub(1).map(|j| {
                let a = layer[j] as usize;
                pos[a] + (geo[a].cs + gap(vertical, kinds[a], kinds[x])) as i64
            });
            let hi = layer.get(k + 1).map(|&b| {
                let b = b as usize;
                pos[b] - (geo[x].cs + gap(vertical, kinds[x], kinds[b])) as i64
            });
            let bound = |p: i64| {
                let p = lo.map_or(p, |lo| p.max(lo));
                hi.map_or(p, |hi| p.min(hi))
            };
            let now = pos[x];
            let (mut best, mut best_score) = (now, score(x, now, pos));
            for &(o, oo, om, _) in &nbrs[x] {
                let (xl, xh) = offers(g, kinds[x], &geo[x], om);
                let (ol, oh) = offers(g, kinds[o as usize], &geo[o as usize], oo);
                let op = pos[o as usize];
                // the positions keeping this edge straight, nearest the current one
                let p = bound(now.clamp(op + ol - xh, op + oh - xl));
                let s = score(x, p, pos);
                if s > best_score || (s == best_score && (p - now).abs() < (best - now).abs()) {
                    (best, best_score) = (p, s);
                }
            }
            pos[x] = best;
        }
    }
}

/// Positions as near the desired ones `(position, weight)` as the separations `sep` between
/// neighbors allow, in the least-squares sense: pool adjacent violators.
fn pava(sep: &[i64], desired: &[(f64, f64)]) -> Vec<i64> {
    let m = desired.len();
    let mut offset = vec![0i64; m];
    for k in 1..m {
        offset[k] = offset[k - 1] + sep[k - 1];
    }
    // (total weight, weighted sum, first index)
    let mut blocks: Vec<(f64, f64, usize)> = Vec::with_capacity(m);
    for (k, &(d, w)) in desired.iter().enumerate() {
        let y = d - offset[k] as f64;
        blocks.push((w, w * y, k));
        while blocks.len() >= 2 {
            let last = blocks[blocks.len() - 1];
            let prev = blocks[blocks.len() - 2];
            if prev.1 / prev.0 <= last.1 / last.0 {
                break;
            }
            blocks.pop();
            let top = blocks.len() - 1;
            blocks[top] = (prev.0 + last.0, prev.1 + last.1, prev.2);
        }
    }
    let mut out = vec![0i64; m];
    for (bi, &(w, s, start)) in blocks.iter().enumerate() {
        let end = blocks.get(bi + 1).map_or(m, |b| b.2);
        let v = (s / w).round() as i64;
        for k in start..end {
            out[k] = v + offset[k];
        }
    }
    out
}

/// The span of columns a node's box offers edges on one side.
fn port_range(kind: Kind, g: &Graph, it: &ItemGeo) -> (usize, usize) {
    let lo = it.c + it.box_c;
    let hi = lo + it.box_cs - 1;
    match kind {
        Kind::Node(n) if matches!(g.nodes[n as usize].shape, Shape::Fork) => (lo, hi),
        _ if it.box_cs >= 3 => (lo + 1, hi - 1),
        _ => (lo + it.box_cs / 2, lo + it.box_cs / 2),
    }
}

/// Columns for `m` edges on one side of a box, as near their ideals (sorted) as distinct
/// columns in `lo..=hi` allow; they share when there are more edges than columns.
fn spread(ideal: &[usize], lo: usize, hi: usize) -> Vec<usize> {
    let m = ideal.len();
    let room = hi - lo + 1;
    if m > room {
        return (0..m).map(|k| lo + k * room / m).collect();
    }
    // as near the ideals as one column apart allows, then pushed inside the range
    let wanted: Vec<(f64, f64)> = ideal.iter().map(|&i| (i as f64, 1.0)).collect();
    let mut out: Vec<usize> = pava(&vec![1; m.saturating_sub(1)], &wanted)
        .into_iter()
        .map(|c| usize::try_from(c).unwrap_or(0))
        .collect();
    for k in 0..m {
        let floor = if k == 0 { lo } else { out[k - 1] + 1 };
        out[k] = out[k].max(floor);
    }
    let mut ceil = hi + 1;
    for k in (0..m).rev() {
        out[k] = out[k].min(ceil - 1);
        ceil = out[k];
    }
    out
}

/// Ports, tracks, then positions along; the level's geometry.
#[allow(clippy::too_many_arguments)]
fn route(
    g: &Graph,
    lv: &Level,
    mut geo: Vec<ItemGeo>,
    kinds: &[Kind],
    segs: &[Link],
    framed: bool,
    vertical: bool,
    cs: usize,
) -> Option<Geo> {
    let ns = segs.len();
    // the column each segment leaves and enters by: fixed for lines, ports and subgraphs,
    // chosen on boxes
    let fixed = |item: u32, off: usize, geo: &[ItemGeo]| -> Option<usize> {
        match kinds[item as usize] {
            Kind::Node(_) => None,
            _ => Some(geo[item as usize].c + off),
        }
    };
    let center = |item: u32, off: usize, geo: &[ItemGeo]| geo[item as usize].c + off;
    let mut cu = vec![0usize; ns];
    let mut cv = vec![0usize; ns];
    let mut exits: HashMap<u32, Vec<usize>> = HashMap::new();
    let mut entries: HashMap<u32, Vec<usize>> = HashMap::new();
    for (s, &Link { a, b, oa, ob, .. }) in segs.iter().enumerate() {
        match fixed(a, oa, &geo) {
            Some(c) => cu[s] = c,
            None => exits.entry(a).or_default().push(s),
        }
        match fixed(b, ob, &geo) {
            Some(c) => cv[s] = c,
            None => entries.entry(b).or_default().push(s),
        }
    }
    let assign =
        |list: &HashMap<u32, Vec<usize>>, out: &mut [usize], other: &dyn Fn(usize) -> usize| {
            for (&item, ss) in list {
                let mut ss = ss.clone();
                ss.sort_by_key(|&s| other(s));
                let (lo, hi) = port_range(kinds[item as usize], g, &geo[item as usize]);
                let ideal: Vec<usize> = ss.iter().map(|&s| other(s)).collect();
                for (k, c) in spread(&ideal, lo, hi).into_iter().enumerate() {
                    out[ss[k]] = c;
                }
            }
        };
    let target = |s: usize| {
        let Link { b, ob, .. } = segs[s];
        fixed(b, ob, &geo).unwrap_or_else(|| center(b, ob, &geo))
    };
    assign(&exits, &mut cu, &target);
    let cu_now = cu.clone();
    assign(&entries, &mut cv, &|s: usize| cu_now[s]);

    // tracks, channel by channel
    let n_layers = lv.layers.len();
    let mut by_channel: Vec<Vec<usize>> = vec![Vec::new(); n_layers];
    for (s, &Link { a, .. }) in segs.iter().enumerate() {
        by_channel[lv.items[a as usize].layer as usize].push(s);
    }
    let mut track = vec![None; ns];
    // per channel: (rows, where tracks start)
    let mut channels = vec![(0usize, 0usize); n_layers.saturating_sub(1)];
    for (ch, list) in by_channel
        .iter()
        .enumerate()
        .take(n_layers.saturating_sub(1))
    {
        let (mut tracks, mut lanes) = assign_tracks(list, segs, &cu, &cv, &mut track);
        // a lane two legs would share whatever the order: a box's port there moves to a free
        // column of its side, and the channel is routed again
        for _ in 0..4 {
            if lanes.is_empty() || !free_lanes(g, kinds, &geo, list, segs, &mut cu, &mut cv, &lanes)
            {
                break;
            }
            (tracks, lanes) = assign_tracks(list, segs, &cu, &cv, &mut track);
        }
        let path_of = |s: usize| &lv.paths[segs[s].path as usize];
        let ends = |s: usize| {
            let path = path_of(s);
            let first = path.items[0] == segs[s].a;
            let last = *path.items.last().unwrap_or(&0) == segs[s].b;
            (
                first && path.start != Mark::None,
                last && path.end != Mark::None,
            )
        };
        let has_start = list.iter().any(|&s| ends(s).0);
        let has_end = list.iter().any(|&s| ends(s).1);
        let border = framed && (ch == 0 || ch + 2 == n_layers);
        let min = match (border, vertical) {
            (true, true) => 0,
            (true, false) => 1,
            (false, true) => 2,
            (false, false) => 3,
        };
        let need = usize::from(has_start) + tracks + usize::from(has_end);
        let rows = min.max(need);
        channels[ch] = (rows, usize::from(has_start) + (rows - need) / 2);
    }
    // a frame across `LR` needs room for its title along
    if framed && !vertical && n_layers >= 2 {
        let title = lv
            .cluster
            .map_or(0, |c| width(&g.clusters[c as usize].title));
        let along: usize = channels.iter().map(|c| c.0).sum::<usize>()
            + (0..n_layers).map(|l| band_size(lv, &geo, l)).sum::<usize>();
        let need = if title > 0 { title + 6 } else { 0 };
        if along < need {
            channels[n_layers - 2].0 += need - along;
        }
    }
    let mut bands = Vec::with_capacity(n_layers);
    let mut r = 0usize;
    for l in 0..n_layers {
        let size = band_size(lv, &geo, l);
        bands.push((r, size));
        for &it in &lv.layers[l] {
            let g = &mut geo[it as usize];
            g.r = r + (size - g.rs) / 2;
        }
        r += size;
        if let Some(&(rows, _)) = channels.get(l) {
            r += rows;
        }
    }
    let rs = r;
    // the cells each segment runs between
    let exit = |item: u32, geo: &[ItemGeo]| -> usize {
        let it = &geo[item as usize];
        let (start, size) = bands[lv.items[item as usize].layer as usize];
        match kinds[item as usize] {
            Kind::Node(_) => it.r + it.box_r + it.box_rs - 1,
            Kind::Child(_) => it.r + it.rs - 1,
            Kind::Label(_) => it.r + it.rs,
            Kind::Dummy => start + size - 1,
            _ => it.r,
        }
    };
    let entry = |item: u32, geo: &[ItemGeo]| -> usize {
        let it = &geo[item as usize];
        let (start, _) = bands[lv.items[item as usize].layer as usize];
        match kinds[item as usize] {
            Kind::Node(_) => it.r + it.box_r,
            Kind::Label(_) => it.r.saturating_sub(1),
            Kind::Dummy => start,
            _ => it.r,
        }
    };
    let mut out = Vec::with_capacity(ns);
    for (s, &Link { a, b, path: pi, .. }) in segs.iter().enumerate() {
        let path = &lv.paths[pi as usize];
        let start = if path.items[0] == a {
            path.start
        } else {
            Mark::None
        };
        let end = if *path.items.last()? == b {
            path.end
        } else {
            Mark::None
        };
        let r0 = exit(a, &geo) + usize::from(start != Mark::None);
        let r1 = entry(b, &geo).saturating_sub(usize::from(end != Mark::None));
        let ch = lv.items[a as usize].layer as usize;
        let (band_start, band_size) = bands[ch];
        let ch_start = band_start + band_size;
        let track = track[s].map(|t: usize| ch_start + channels[ch].1 + t);
        out.push(Seg {
            path: pi,
            from_label: matches!(kinds[a as usize], Kind::Label(_)),
            to_label: matches!(kinds[b as usize], Kind::Label(_)),
            r0,
            cu: cu[s],
            track,
            cv: cv[s],
            r1: r1.max(r0),
            start,
            end,
        });
    }
    let ports = lv.ports.iter().map(|&it| geo[it as usize].c).collect();
    Some(Geo {
        rs,
        cs,
        items: geo,
        segs: out,
        ports,
        bands,
    })
}

/// Move the first box's port found on one of `lanes` (columns where legs of two nets in one
/// channel share a lane) to the nearest column of its side no leg in the channel uses. Whether
/// one moved.
#[allow(clippy::too_many_arguments)]
fn free_lanes(
    g: &Graph,
    kinds: &[Kind],
    geo: &[ItemGeo],
    list: &[usize],
    segs: &[Link],
    cu: &mut [usize],
    cv: &mut [usize],
    lanes: &[usize],
) -> bool {
    for &c in lanes {
        // the box whose port it is, and whether its legs leave or enter there
        let port = list.iter().find_map(|&s| {
            let Link { a, b, .. } = segs[s];
            if cu[s] == c && matches!(kinds[a as usize], Kind::Node(_)) {
                Some((a, true))
            } else if cv[s] == c && matches!(kinds[b as usize], Kind::Node(_)) {
                Some((b, false))
            } else {
                None
            }
        });
        let Some((item, leaving)) = port else {
            continue;
        };
        let (lo, hi) = port_range(kinds[item as usize], g, &geo[item as usize]);
        let used = |x: usize| list.iter().any(|&s| cu[s] == x || cv[s] == x);
        let Some(to) = (1..=hi - lo)
            .flat_map(|d| [c.checked_sub(d), Some(c + d)])
            .flatten()
            .find(|&x| (lo..=hi).contains(&x) && !used(x))
        else {
            continue;
        };
        for &s in list {
            if leaving && segs[s].a == item && cu[s] == c {
                cu[s] = to;
            } else if !leaving && segs[s].b == item && cv[s] == c {
                cv[s] = to;
            }
        }
        return true;
    }
    false
}

fn band_size(lv: &Level, geo: &[ItemGeo], l: usize) -> usize {
    lv.layers[l]
        .iter()
        .map(|&it| geo[it as usize].rs)
        .max()
        .unwrap_or(1)
        .max(1)
}

/// Give each net of segments in one channel (those sharing a port) a track, if it bends: the
/// left-edge algorithm under the vertical constraints, so a leg into or out of a port keeps out
/// of the lane of another net's leg the other way, and stays off other tracks where it can. Returns the number of tracks, and the columns where legs of two
/// nets still share a lane.
fn assign_tracks(
    list: &[usize],
    segs: &[Link],
    cu: &[usize],
    cv: &[usize],
    track: &mut [Option<usize>],
) -> (usize, Vec<usize>) {
    let m = list.len();
    let mut parent: Vec<usize> = (0..m).collect();
    fn find(parent: &mut [usize], x: usize) -> usize {
        let mut x = x;
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    let mut by_out: HashMap<(u32, usize), usize> = HashMap::new();
    let mut by_in: HashMap<(u32, usize), usize> = HashMap::new();
    for (k, &s) in list.iter().enumerate() {
        for (map, key) in [
            (&mut by_out, (segs[s].a, cu[s])),
            (&mut by_in, (segs[s].b, cv[s])),
        ] {
            if let Some(&o) = map.get(&key) {
                let (x, y) = (find(&mut parent, k), find(&mut parent, o));
                parent[x] = y;
            } else {
                map.insert(key, k);
            }
        }
    }
    // nets: (legs from above, legs down, lo, hi, members)
    struct Net {
        up: Vec<usize>,
        down: Vec<usize>,
        lo: usize,
        hi: usize,
        members: Vec<usize>,
    }
    let mut nets: Vec<Net> = Vec::new();
    let mut net_of: HashMap<usize, usize> = HashMap::new();
    for (k, &s) in list.iter().enumerate() {
        let root = find(&mut parent, k);
        let ni = *net_of.entry(root).or_insert_with(|| {
            nets.push(Net {
                up: Vec::new(),
                down: Vec::new(),
                lo: usize::MAX,
                hi: 0,
                members: Vec::new(),
            });
            nets.len() - 1
        });
        let net = &mut nets[ni];
        net.up.push(cu[s]);
        net.down.push(cv[s]);
        net.lo = net.lo.min(cu[s]).min(cv[s]);
        net.hi = net.hi.max(cu[s]).max(cv[s]);
        net.members.push(s);
    }
    let bending: Vec<usize> = (0..nets.len())
        .filter(|&i| nets[i].lo != nets[i].hi)
        .collect();
    let nb = bending.len();
    if nb == 0 {
        return (0, Vec::new());
    }
    let overlap = |a: &Net, b: &Net| a.lo <= b.hi + 1 && b.lo <= a.hi + 1;
    let within = |cols: &[usize], n: &Net| cols.iter().any(|&c| n.lo <= c && c <= n.hi);
    let shared = |a: &[usize], b: &[usize]| a.iter().any(|c| b.contains(c));
    // where a leg of one runs in the lane of a leg of the other the other way
    fn lanes_of(x: &Net, y: &Net, out: &mut Vec<usize>) {
        out.extend(x.up.iter().filter(|c| y.down.contains(c)));
        out.extend(x.down.iter().filter(|c| y.up.contains(c)));
    }
    // preds[i]: nets that should be above bending[i], and whether they must; indeg counts
    // (must, should) of those not yet placed
    let mut preds: Vec<Vec<(usize, bool)>> = vec![Vec::new(); nb];
    let mut indeg = vec![(0usize, 0usize); nb];
    // the columns where two legs share a lane whatever the order: two nets trading columns, or
    // a cycle of musts broken below
    let mut lanes = Vec::new();
    for i in 0..nb {
        for j in i + 1..nb {
            let (x, y) = (&nets[bending[i]], &nets[bending[j]]);
            if !overlap(x, y) {
                continue;
            }
            // a leg in the column of the other's leg the other way must stay clear of it, or
            // the two run in one lane and read as a junction; a leg across the other's track
            // only crosses it, which is better avoided
            let lane_above = shared(&x.up, &y.down);
            let lane_below = shared(&x.down, &y.up);
            let must = lane_above || lane_below;
            let (x_above, x_below) = if must {
                (lane_above, lane_below)
            } else {
                (
                    within(&x.up, y) || within(&y.down, x),
                    within(&x.down, y) || within(&y.up, x),
                )
            };
            let (above, below) = match (x_above, x_below) {
                (true, false) => (i, j),
                (false, true) => (j, i),
                (true, true) if must => {
                    lanes_of(x, y, &mut lanes);
                    continue;
                }
                _ => continue,
            };
            preds[below].push((above, must));
            if must {
                indeg[below].0 += 1;
            } else {
                indeg[below].1 += 1;
            }
        }
    }
    let mut succs: Vec<Vec<(usize, bool)>> = vec![Vec::new(); nb];
    for (j, ps) in preds.iter().enumerate() {
        for &(i, must) in ps {
            succs[i].push((j, must));
        }
    }
    let mut done = vec![false; nb];
    let mut tr: Vec<Option<usize>> = vec![None; nb];
    let mut cycle = Vec::new();
    let mut on_track: Vec<Vec<usize>> = Vec::new();
    for _ in 0..nb {
        // the leftmost ready net, or, in a cycle, the one breaking the fewest constraints,
        // musts last
        let next = (0..nb)
            .filter(|&i| !done[i])
            .min_by_key(|&i| (indeg[i], nets[bending[i]].lo))
            .unwrap_or(0);
        if indeg[next].0 > 0 {
            // a cycle of musts: the lanes along it, where a port moved would break it
            for i in (0..nb).filter(|&i| !done[i]) {
                for &(j, must) in &preds[i] {
                    if must && !done[j] {
                        lanes_of(&nets[bending[j]], &nets[bending[i]], &mut cycle);
                    }
                }
            }
        }
        done[next] = true;
        for &(j, must) in &succs[next] {
            let d = &mut indeg[j];
            if must {
                d.0 = d.0.saturating_sub(1);
            } else {
                d.1 = d.1.saturating_sub(1);
            }
        }
        let floor = preds[next]
            .iter()
            .filter_map(|&(i, _)| tr[i])
            .map(|t| t + 1)
            .max()
            .unwrap_or(0);
        let net = &nets[bending[next]];
        let mut t = floor;
        while on_track
            .get(t)
            .is_some_and(|ons| ons.iter().any(|&o| overlap(&nets[bending[o]], net)))
        {
            t += 1;
        }
        if on_track.len() <= t {
            on_track.resize(t + 1, Vec::new());
        }
        on_track[t].push(next);
        tr[next] = Some(t);
    }
    for (i, &ni) in bending.iter().enumerate() {
        for &s in &nets[ni].members {
            track[s] = tr[i];
        }
    }
    for i in 0..nb {
        for &(j, must) in &preds[i] {
            if must && tr[j] >= tr[i] {
                lanes_of(&nets[bending[j]], &nets[bending[i]], &mut lanes);
            }
        }
    }
    lanes.extend(cycle);
    lanes.dedup();
    (on_track.len(), lanes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pava_keeps_gaps_and_pulls_to_targets() {
        // two items wanting the same spot, 5 apart at least
        assert_eq!(pava(&[5], &[(10.0, 1.0), (10.0, 1.0)]), [8, 13]);
        // already apart: unmoved
        assert_eq!(pava(&[2], &[(0.0, 1.0), (9.0, 1.0)]), [0, 9]);
        // a heavy item barely moves
        assert_eq!(pava(&[4], &[(0.0, 100.0), (0.0, 1.0)]), [0, 4]);
    }

    #[test]
    fn ports_spread_and_share() {
        assert_eq!(spread(&[5, 5, 5], 0, 10), [4, 5, 6]);
        assert_eq!(spread(&[0, 20], 3, 8), [3, 8]);
        assert_eq!(spread(&[1, 1, 1], 1, 2), [1, 1, 2]);
    }
}
