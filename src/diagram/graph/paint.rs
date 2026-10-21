//! Drawing a placed graph onto the canvas: the abstract layout turned to rows and columns for
//! its direction, frames first, then edges, then the boxes and text over them.

use unicode_width::UnicodeWidthStr;

use super::place::{ItemGeo, Placed};
use super::topo::{Kind, Topo};
use super::{Dir, Frame, Graph, Line, Mark, Shape};
use crate::diagram::canvas::{Canvas, Pen};
use crate::diagram::{Text, width};
use crate::theme::StyleKey;

const LINE: StyleKey = StyleKey::DiagramLine;

pub(super) fn paint(g: &Graph, t: &Topo, p: &Placed, ascii: bool) -> Canvas {
    let mut canvas = Canvas::new(p.canvas_width(g.dir), ascii);
    canvas.grow(p.canvas_height(g.dir));
    let mut painter = Painter {
        g,
        t,
        p,
        canvas,
        dir: g.dir,
        total: p.levels[0].rs,
        ascii,
    };
    painter.level(0, 0, 0);
    painter.canvas
}

/// A way along an edge, in abstract terms: on along `r`, or back.
#[derive(Clone, Copy)]
enum Toward {
    Along,
    Back,
}

/// A rectangle on the canvas.
#[derive(Clone, Copy)]
struct Rect {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

struct Painter<'a> {
    g: &'a Graph,
    t: &'a Topo,
    p: &'a Placed,
    canvas: Canvas,
    dir: Dir,
    /// The root's size along, which `BT` and `RL` count back from.
    total: usize,
    ascii: bool,
}

impl Painter<'_> {
    /// Abstract `(r, c)` → canvas `(row, col)`.
    fn cell(&self, r: usize, c: usize) -> (usize, usize) {
        let back = |r: usize| self.total.saturating_sub(r + 1);
        match self.dir {
            Dir::Down => (r, c),
            Dir::Up => (back(r), c),
            Dir::Right => (c, r),
            Dir::Left => (c, back(r)),
        }
    }

    fn rect(&self, r: usize, c: usize, rs: usize, cs: usize) -> Rect {
        let back = |r: usize| self.total.saturating_sub(r + rs);
        match self.dir {
            Dir::Down => Rect {
                x: c,
                y: r,
                w: cs,
                h: rs,
            },
            Dir::Up => Rect {
                x: c,
                y: back(r),
                w: cs,
                h: rs,
            },
            Dir::Right => Rect {
                x: r,
                y: c,
                w: rs,
                h: cs,
            },
            Dir::Left => Rect {
                x: back(r),
                y: c,
                w: rs,
                h: cs,
            },
        }
    }

    /// A straight line between two abstract cells.
    fn line(&mut self, a: (usize, usize), b: (usize, usize), pen: Pen) {
        let (ra, ca) = self.cell(a.0, a.1);
        let (rb, cb) = self.cell(b.0, b.1);
        if ra == rb {
            self.canvas.hline(ra, ca, cb, pen, LINE);
        } else {
            self.canvas.vline(ca, ra, rb, pen, LINE);
        }
    }

    /// Extend the line at abstract `at` one cell's worth `toward`.
    fn extend(&mut self, at: (usize, usize), toward: Toward) {
        let (row, col) = self.cell(at.0, at.1);
        let way = match (self.dir, toward) {
            (Dir::Down, Toward::Back) | (Dir::Up, Toward::Along) => (true, false, false, false),
            (Dir::Up, Toward::Back) | (Dir::Down, Toward::Along) => (false, true, false, false),
            (Dir::Right, Toward::Back) | (Dir::Left, Toward::Along) => (false, false, true, false),
            (Dir::Left, Toward::Back) | (Dir::Right, Toward::Along) => (false, false, false, true),
        };
        self.canvas.extend(row, col, way);
    }

    fn mark(&mut self, r: usize, c: usize, mark: Mark, toward: Toward) {
        let glyph = match (mark, self.ascii) {
            (Mark::None, _) => return,
            (Mark::Circle, false) => '○',
            (Mark::Circle, true) => 'o',
            (Mark::Cross, false) => '×',
            (Mark::Cross, true) => 'x',
            (Mark::Arrow, ascii) => {
                // the canvas direction it points in: down, up, right, left
                let d = match (self.dir, toward) {
                    (Dir::Down, Toward::Along) | (Dir::Up, Toward::Back) => 0,
                    (Dir::Up, Toward::Along) | (Dir::Down, Toward::Back) => 1,
                    (Dir::Right, Toward::Along) | (Dir::Left, Toward::Back) => 2,
                    (Dir::Left, Toward::Along) | (Dir::Right, Toward::Back) => 3,
                };
                let set = if ascii {
                    ['v', '^', '>', '<']
                } else {
                    ['▼', '▲', '▶', '◀']
                };
                set[d]
            }
        };
        let (row, col) = self.cell(r, c);
        self.canvas.glyph(row, col, glyph, LINE);
    }

    fn pen(line: Line) -> Pen {
        match line {
            Line::Dotted => Pen::DASHED,
            Line::Thick => Pen::THICK,
            _ => Pen::SOLID,
        }
    }

    /// Draw level `l` with its origin at abstract `(r0, c0)`.
    fn level(&mut self, l: usize, r0: usize, c0: usize) {
        let (t, p, g) = (self.t, self.p, self.g);
        let lv = &t.levels[l];
        let geo = &p.levels[l];
        if let Some(c) = lv.cluster {
            let cluster = &g.clusters[c as usize];
            let pen = match cluster.frame {
                Frame::Square => Pen::SOLID,
                Frame::Round => Pen::ROUND,
                Frame::Dashed => Pen::DASHED,
            };
            let f = self.rect(r0, c0, geo.rs, geo.cs);
            if f.w > 0 && f.h > 0 {
                self.canvas
                    .rect(f.y, f.x, f.y + f.h - 1, f.x + f.w - 1, pen, LINE);
            }
            if !cluster.title.is_empty() {
                let x = match lv.items.iter().position(|i| i.kind == Kind::Title) {
                    Some(i) if self.dir.vertical() => c0 + geo.items[i].c,
                    _ => f.x + 2,
                };
                self.title(f.y, x, &cluster.title);
            }
        }
        // edges
        for s in &geo.segs {
            let path = &lv.paths[s.path as usize];
            let pen = Self::pen(g.edges[path.edge as usize].line);
            let (a, b) = ((r0 + s.r0, c0 + s.cu), (r0 + s.r1, c0 + s.cv));
            match s.track {
                Some(tr) if s.cu != s.cv => {
                    let tr = r0 + tr;
                    self.line(a, (tr, a.1), pen);
                    self.line((tr, a.1), (tr, b.1), pen);
                    self.line((tr, b.1), b, pen);
                }
                _ => self.line(a, (b.0, a.1), pen),
            }
            // a line at a label reaches it, even where its first cell is a corner or a crossing
            if s.from_label {
                self.extend(a, Toward::Back);
            }
            if s.to_label {
                self.extend(b, Toward::Along);
            }
        }
        // long edges through the layers between
        for path in &lv.paths {
            let pen = Self::pen(g.edges[path.edge as usize].line);
            for &it in &path.items[1..path.items.len() - 1] {
                if lv.items[it as usize].kind != Kind::Dummy {
                    continue;
                }
                let (start, size) = geo.bands[lv.items[it as usize].layer as usize];
                let c = c0 + geo.items[it as usize].c;
                self.line((r0 + start, c), (r0 + start + size - 1, c), pen);
            }
        }
        for s in &geo.segs {
            self.mark(r0 + s.r0, c0 + s.cu, s.start, Toward::Back);
            self.mark(r0 + s.r1, c0 + s.cv, s.end, Toward::Along);
        }
        // what the edges meet
        for (i, item) in lv.items.iter().enumerate() {
            let it = geo.items[i];
            let (r, c) = (r0 + it.r, c0 + it.c);
            match item.kind {
                Kind::Node(n) => self.node(n as usize, r, c, &it, &item.loops),
                Kind::Label(e) => {
                    let f = self.rect(r, c, it.rs, it.cs);
                    let lines = &p.edges[e as usize];
                    let pad = usize::from(!self.dir.vertical());
                    self.text_block(f, pad, lines, StyleKey::DiagramLabel);
                }
                Kind::Child(child) => self.level(child as usize, r, c),
                _ => {}
            }
        }
    }

    fn title(&mut self, row: usize, x: usize, title: &Text) {
        let x = self.canvas.text(row, x, " ", LINE);
        let x = self.canvas.label(row, x, title, StyleKey::DiagramFrame);
        self.canvas.text(row, x, " ", LINE);
    }

    /// Lines centered across `f`, `pad` columns in from each side, from its top.
    fn text_block(&mut self, f: Rect, pad: usize, lines: &[Text], key: StyleKey) {
        let inner = f.w.saturating_sub(2 * pad);
        for (k, line) in lines.iter().enumerate() {
            let x = f.x + pad + inner.saturating_sub(width(line)) / 2;
            self.canvas.label(f.y + k, x, line, key);
        }
    }

    fn node(&mut self, n: usize, r: usize, c: usize, it: &ItemGeo, loops: &[u32]) {
        let (g, p) = (self.g, self.p);
        let node = &g.nodes[n];
        let b = self.rect(r + it.box_r, c + it.box_c, it.box_rs, it.box_cs);
        let (right, bottom) = (b.x + b.w - 1, b.y + b.h - 1);
        let ascii = self.ascii;
        match node.shape {
            Shape::Start => self
                .canvas
                .glyph(b.y, b.x, if ascii { '*' } else { '●' }, LINE),
            Shape::End => self
                .canvas
                .glyph(b.y, b.x, if ascii { '@' } else { '◉' }, LINE),
            Shape::Choice => self
                .canvas
                .glyph(b.y, b.x, if ascii { '?' } else { '◆' }, LINE),
            Shape::Fork => {
                let bar = if b.h == 1 {
                    if ascii { '=' } else { '━' }
                } else if ascii {
                    '#'
                } else {
                    '┃'
                };
                for y in b.y..=bottom {
                    for x in b.x..=right {
                        self.canvas.glyph(y, x, bar, LINE);
                    }
                }
            }
            shape => {
                let pen = match shape {
                    Shape::Round | Shape::Stadium => Pen::ROUND,
                    _ => Pen::SOLID,
                };
                // blank inside, so nothing shows through
                for y in b.y + 1..bottom {
                    for x in b.x + 1..right {
                        self.canvas.glyph(y, x, ' ', StyleKey::DiagramActor);
                    }
                }
                self.canvas.rect(b.y, b.x, bottom, right, pen, LINE);
                match shape {
                    Shape::Stadium => {
                        for y in b.y + 1..bottom {
                            self.canvas.glyph(y, b.x, '(', LINE);
                            self.canvas.glyph(y, right, ')', LINE);
                        }
                    }
                    Shape::Diamond => {
                        let [tl, tr] = if ascii { ['/', '\\'] } else { ['╱', '╲'] };
                        self.canvas.glyph(b.y, b.x, tl, LINE);
                        self.canvas.glyph(b.y, right, tr, LINE);
                        self.canvas.glyph(bottom, b.x, tr, LINE);
                        self.canvas.glyph(bottom, right, tl, LINE);
                    }
                    _ => {}
                }
                let inner = Rect {
                    x: b.x,
                    y: b.y + 1,
                    w: b.w,
                    h: b.h.saturating_sub(2),
                };
                let lines = &p.nodes[n];
                // centered down the box, too, when it's taller for a loop
                let top = inner.h.saturating_sub(lines.len()) / 2;
                let inner = Rect {
                    y: inner.y + top,
                    ..inner
                };
                self.text_block(inner, 2, lines, StyleKey::DiagramActor);
            }
        }
        if let Some(&e) = loops.first() {
            self.self_loop(b, e, loops);
        }
        if let Some(note) = &node.note {
            let fp = self.rect(r, c, it.rs, it.cs);
            self.note(fp, b, note.after, &p.notes[n]);
        }
    }

    /// A loop from a box back to itself: out of its right side in `TD` and `BT`, its bottom
    /// in `LR` and `RL`, the labels beside it.
    fn self_loop(&mut self, b: Rect, e: u32, loops: &[u32]) {
        let (g, p) = (self.g, self.p);
        let edge = &g.edges[e as usize];
        let pen = Self::pen(edge.line);
        let ascii = self.ascii;
        let labels: Vec<&Text> = loops
            .iter()
            .flat_map(|&e| p.edges[e as usize].iter())
            .collect();
        let (right, bottom) = (b.x + b.w - 1, b.y + b.h - 1);
        if self.dir.vertical() {
            let (y0, y1) = (b.y + 1, b.y + 2);
            self.canvas.hline(y0, right, right + 2, pen, LINE);
            self.canvas.vline(right + 2, y0, y1, pen, LINE);
            self.canvas.hline(y1, right + 1, right + 2, pen, LINE);
            if edge.head_to != Mark::None {
                let head = if ascii { '<' } else { '◀' };
                self.canvas.glyph(y1, right + 1, head, LINE);
            }
            for (k, l) in labels.iter().enumerate() {
                self.canvas
                    .label(y0 + k, right + 4, l, StyleKey::DiagramLabel);
            }
        } else {
            let (x0, x1) = (b.x + 1, b.x + 4);
            self.canvas.vline(x0, bottom, bottom + 2, pen, LINE);
            self.canvas.hline(bottom + 2, x0, x1, pen, LINE);
            self.canvas.vline(x1, bottom + 1, bottom + 2, pen, LINE);
            if edge.head_to != Mark::None {
                let head = if ascii { '^' } else { '▲' };
                self.canvas.glyph(bottom + 1, x1, head, LINE);
            }
            for (k, l) in labels.iter().enumerate() {
                self.canvas
                    .label(bottom + 2 + k, x1 + 2, l, StyleKey::DiagramLabel);
            }
        }
    }

    /// A note beside a box, within the node's footprint `fp`, joined to it by a dotted line.
    fn note(&mut self, fp: Rect, b: Rect, after: bool, text: &[Text]) {
        let w = text.iter().map(|l| l.width()).max().unwrap_or(0) + 4;
        let h = text.len().max(1) + 2;
        let n = if self.dir.vertical() {
            let x = if after { fp.x + fp.w - w } else { fp.x };
            Rect {
                x,
                y: b.y + b.h / 2 - (h / 2).min(b.y + b.h / 2 - fp.y),
                w,
                h,
            }
        } else {
            let y = if after { fp.y + fp.h - h } else { fp.y };
            Rect {
                x: b.x + b.w / 2 - (w / 2).min(b.x + b.w / 2 - fp.x),
                y,
                w,
                h,
            }
        };
        for y in n.y + 1..n.y + n.h - 1 {
            for x in n.x + 1..n.x + n.w - 1 {
                self.canvas.glyph(y, x, ' ', StyleKey::DiagramNote);
            }
        }
        self.canvas
            .rect(n.y, n.x, n.y + n.h - 1, n.x + n.w - 1, Pen::ROUND, LINE);
        let inner = Rect {
            x: n.x,
            y: n.y + 1,
            w: n.w,
            h: n.h - 2,
        };
        self.text_block(inner, 2, text, StyleKey::DiagramNote);
        if self.dir.vertical() {
            let row = b.y + b.h / 2;
            let (from, to) = if after {
                (b.x + b.w, n.x.saturating_sub(1))
            } else {
                (n.x + n.w, b.x.saturating_sub(1))
            };
            if from <= to {
                self.canvas.hline(row, from, to, Pen::DASHED, LINE);
            }
        } else {
            let col = b.x + b.w / 2;
            let (from, to) = if after {
                (b.y + b.h, n.y.saturating_sub(1))
            } else {
                (n.y + n.h, b.y.saturating_sub(1))
            };
            if from <= to {
                self.canvas.vline(col, from, to, Pen::DASHED, LINE);
            }
        }
    }
}
