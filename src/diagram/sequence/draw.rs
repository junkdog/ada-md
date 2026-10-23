//! Drawing a sequence diagram (ada-mermaid, "Design"). The x layout puts the lifelines as close
//! as the boxes, labels and notes between them allow, solved left to right. When that is wider
//! than the text, a binary search finds the widest label cap that fits: labels over it wrap to
//! two rows, then are cut and listed under the diagram. The rows follow from the events, top to
//! bottom, with the participants as the sticky header.

use std::cell::OnceCell;

use super::{Event, Head, Message, NoteAt, Sequence, Who};
use crate::diagram::canvas::{Canvas, Pen};
use crate::diagram::{Drawn, Text, cut, marker, pieces, width, wrap, wrap_size};
use crate::theme::StyleKey;

/// Labels are never capped below this many columns; past it the diagram scrolls instead.
const MIN_CAP: usize = 16;
/// Columns between neighboring boxes.
const BOX_GAP: usize = 3;

const LINE: StyleKey = StyleKey::DiagramLine;

pub fn draw(s: &Sequence, avail: usize, ascii: bool) -> Option<Drawn> {
    // each event's label, measured once for all the plans the search tries
    let labels: Vec<Option<Label>> = s
        .events
        .iter()
        .map(|e| match e {
            Event::Message(m) => Some(Label::of(m)),
            Event::Note { text, .. } => Some(Label::note(text)),
            _ => None,
        })
        .collect();
    let natural = Plan::new(s, &labels, None);
    let cap = if natural.width <= avail {
        None
    } else {
        let widest = labels
            .iter()
            .flatten()
            .map(|l| l.natural)
            .max()
            .unwrap_or(0);
        let fits = |cap: usize| Plan::new(s, &labels, Some(cap)).width <= avail;
        let (mut lo, mut hi) = (MIN_CAP, widest.max(MIN_CAP));
        if fits(lo) {
            // the widest cap that fits: `lo` always fits, `hi` may not
            while lo < hi {
                let mid = lo + (hi - lo).div_ceil(2);
                if fits(mid) { lo = mid } else { hi = mid - 1 }
            }
        }
        Some(lo)
    };
    let plan = match cap {
        None => natural,
        Some(cap) => Plan::new(s, &labels, Some(cap)),
    };
    Some(Painter::new(s, plan, ascii).paint())
}

/// A message's label or a note's text: its lines, and the columns its number takes in front
/// of the first (`7 `), which never wrap away from it.
struct Label<'a> {
    lines: &'a [Text],
    number: usize,
    /// Columns it takes unfitted.
    natural: usize,
    /// Its pieces, measured the first time the search caps it.
    pieces: OnceCell<Pieces>,
}

/// A label's unbreakable pieces, measured once for every cap the search tries: each piece's
/// columns since the last one's end (spaces) and its own width.
struct Pieces {
    pieces: Vec<(usize, usize)>,
    /// Where each line's pieces end.
    ends: Vec<usize>,
    /// The widest piece: a cap narrower than it breaks a word, which [`wrap_size`] does.
    longest: usize,
}

impl Pieces {
    fn of(lines: &[Text]) -> Self {
        let mut out = Pieces {
            pieces: Vec::new(),
            ends: Vec::with_capacity(lines.len()),
            longest: 0,
        };
        for line in lines {
            let mut end = None;
            for (at, piece) in pieces(line) {
                let w = width(piece);
                out.pieces.push((end.map_or(0, |e| at - e), w));
                out.longest = out.longest.max(w);
                end = Some(at + piece.len());
            }
            out.ends.push(out.pieces.len());
        }
        out
    }

    /// [`wrap_size`] summed over the lines, when no piece is wider than `max`; it stops
    /// counting once the rows pass `limit`.
    fn wrap_size(&self, max: usize, limit: usize) -> (usize, usize) {
        let (mut rows, mut widest) = (0, 0);
        let mut from = 0;
        for &to in &self.ends {
            // the row being filled: its width, once a piece is on it
            let mut used = None;
            for &(gap, w) in &self.pieces[from..to] {
                used = Some(match used {
                    None => w,
                    Some(u) if u + gap + w <= max => u + gap + w,
                    Some(u) => {
                        rows += 1;
                        widest = widest.max(u);
                        if rows > limit {
                            return (rows, widest);
                        }
                        w
                    }
                });
            }
            from = to;
            rows += 1;
            widest = widest.max(used.unwrap_or(0));
            if rows > limit {
                break;
            }
        }
        (rows, widest)
    }
}

impl<'a> Label<'a> {
    fn of(m: &'a Message) -> Self {
        let number = m
            .number
            .map_or(0, |n| n.checked_ilog10().unwrap_or(0) as usize + 2);
        Self::new(&m.label, number)
    }

    fn note(lines: &'a [Text]) -> Self {
        Self::new(lines, 0)
    }

    fn new(lines: &'a [Text], number: usize) -> Self {
        let natural = lines
            .iter()
            .enumerate()
            .map(|(i, l)| width(l) + if i == 0 { number } else { 0 })
            .max()
            .unwrap_or(number);
        Self {
            lines,
            number,
            natural,
            pieces: OnceCell::new(),
        }
    }

    /// The rows it may take before the last is cut: two, or its own line count.
    fn rows(&self) -> usize {
        self.lines.len().max(2)
    }

    /// Columns for the text when capped at `cap`, the number aside.
    fn inner(&self, cap: usize) -> usize {
        cap.saturating_sub(self.number).max(1)
    }

    /// The columns it takes fitted to `cap`, a cut marker included.
    fn width(&self, cap: Option<usize>) -> usize {
        let Some(cap) = cap.filter(|&c| self.natural > c) else {
            return self.natural;
        };
        let inner = self.inner(cap);
        let pieces = self.pieces.get_or_init(|| Pieces::of(self.lines));
        let (rows, widest) = if pieces.longest <= inner {
            pieces.wrap_size(inner, self.rows())
        } else {
            self.lines
                .iter()
                .map(|l| wrap_size(l, inner))
                .fold((0, 0), |(r, w), (r2, w2)| (r + r2, w.max(w2)))
        };
        if rows > self.rows() {
            cap
        } else {
            self.number + widest
        }
    }

    /// Its lines fitted to `cap` (the number left out), and whether the last was cut.
    fn fit(&self, cap: Option<usize>) -> (Vec<Text>, bool) {
        let Some(cap) = cap.filter(|&c| self.natural > c) else {
            return (self.lines.to_vec(), false);
        };
        let inner = self.inner(cap);
        let mut out: Vec<Text> = self.lines.iter().flat_map(|l| wrap(l, inner)).collect();
        let cut = out.len() > self.rows();
        out.truncate(self.rows());
        (out, cut)
    }
}

/// Where everything goes across: the lifelines and the frames.
struct Plan {
    cap: Option<usize>,
    /// Lifeline columns.
    centers: Vec<usize>,
    /// Box widths.
    boxes: Vec<usize>,
    /// `(left, right)` of each frame, in order of their `Begin`.
    frames: Vec<(usize, usize)>,
    width: usize,
}

impl Plan {
    fn new(s: &Sequence, labels: &[Option<Label>], cap: Option<usize>) -> Self {
        let n = s.participants.len();
        let boxes: Vec<usize> = s
            .participants
            .iter()
            .map(|p| Label::note(&p.label).natural + 4)
            .collect();
        // each event's label fitted to the cap, measured once
        let widths: Vec<usize> = labels
            .iter()
            .map(|l| l.as_ref().map_or(0, |l| l.width(cap)))
            .collect();
        let half_l = |i: usize| boxes[i] / 2;
        let half_r = |i: usize| boxes[i] - 1 - boxes[i] / 2;
        // `needs[j]`: (i, d) for c[j] - c[i] >= d, i < j; `left`: c[j] >= left[j]; `right`:
        // the diagram reaches c[j] + right[j]
        let mut needs: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
        let mut left: Vec<usize> = (0..n).map(half_l).collect();
        let mut right: Vec<usize> = (0..n).map(half_r).collect();
        let mut need = |i: usize, j: usize, d: usize| {
            let (i, j) = (i.min(j), i.max(j));
            if i != j {
                needs[j].push((i, d));
            }
        };
        for j in 1..n {
            need(j - 1, j, half_r(j - 1) + BOX_GAP + half_l(j));
        }
        for (e, &w) in s.events.iter().zip(&widths) {
            match e {
                Event::Message(m) => {
                    let (a, b) = (usize::from(m.from), usize::from(m.to));
                    if a == b {
                        // a loop out to the right, the label beside it
                        let reach = 5 + w;
                        if a + 1 < n {
                            // room for a frame around it before the next lifeline
                            need(a, a + 1, reach + 4);
                        } else {
                            right[a] = right[a].max(reach);
                        }
                    } else {
                        need(a, b, w + 4);
                    }
                }
                Event::Note { at, .. } => {
                    let w = w + 4;
                    match *at {
                        NoteAt::RightOf(a) => {
                            let a = usize::from(a);
                            if a + 1 < n {
                                need(a, a + 1, w + 3);
                            } else {
                                right[a] = right[a].max(w + 2);
                            }
                        }
                        NoteAt::LeftOf(a) => {
                            let a = usize::from(a);
                            if a > 0 {
                                need(a - 1, a, w + 3);
                            } else {
                                left[a] = left[a].max(w + 2);
                            }
                        }
                        NoteAt::Over(a, b) => {
                            let (a, b) = (usize::from(a), usize::from(b));
                            if a == b {
                                left[a] = left[a].max(w / 2);
                                right[a] = right[a].max(w - w / 2);
                                if a > 0 {
                                    need(a - 1, a, w / 2 + 2);
                                }
                                if a + 1 < n {
                                    need(a, a + 1, w - w / 2 + 2);
                                }
                            } else {
                                need(a, b, w.saturating_sub(5));
                                left[a] = left[a].max(2);
                                right[b] = right[b].max(2);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        let mut centers = vec![0; n];
        for j in 0..n {
            let mut c = left[j];
            for &(i, d) in &needs[j] {
                c = c.max(centers[i] + d);
            }
            centers[j] = c;
        }
        let mut plan = Plan {
            cap,
            centers,
            boxes,
            frames: Vec::new(),
            width: 0,
        };
        plan.place_frames(s, &widths);
        plan.width = (0..n)
            .map(|j| plan.centers[j] + right[j] + 1)
            .chain(plan.frames.iter().map(|f| f.1 + 1))
            .max()
            .unwrap_or(0);
        plan
    }

    /// Frames go around what happens inside them, two columns out, each enclosing frame two
    /// more; the diagram moves right when one would start left of it.
    fn place_frames(&mut self, s: &Sequence, widths: &[usize]) {
        type Span = Option<(isize, isize)>;
        fn grow(span: &mut Span, (a, b): (isize, isize)) {
            *span = Some(span.map_or((a, b), |(l, r)| (l.min(a), r.max(b))));
        }
        let c = |w: Who| self.centers[usize::from(w)] as isize;
        // (index into `frames`, what's inside so far, the width its label needs)
        let mut open: Vec<(usize, Span, isize)> = Vec::new();
        let mut frames: Vec<(isize, isize)> = Vec::new();
        for (e, &w) in s.events.iter().zip(widths) {
            let span = match e {
                Event::Message(m) if m.from == m.to => {
                    Some((c(m.from), c(m.from) + 5 + w as isize))
                }
                Event::Message(m) => Some((c(m.from).min(c(m.to)), c(m.from).max(c(m.to)))),
                Event::Note { at, .. } => Some(note_span(*at, w + 4, &c)),
                Event::Begin { kind, label } => {
                    let label = (width(kind) + width(label) + 7) as isize;
                    open.push((frames.len(), None, label));
                    frames.push((0, 0));
                    None
                }
                Event::End => {
                    let Some((i, inside, label)) = open.pop() else {
                        continue;
                    };
                    let (l, r) = inside.unwrap_or((c(0), c(0)));
                    let (l, r) = (l - 2, (r + 2).max(l - 2 + label));
                    // a side on a lifeline would read as a junction
                    let on_lifeline = |x: isize| self.centers.iter().any(|&c| c as isize == x);
                    let l = if on_lifeline(l) { l - 1 } else { l };
                    let r = if on_lifeline(r) { r + 1 } else { r };
                    frames[i] = (l, r);
                    Some((l, r))
                }
                _ => None,
            };
            if let (Some(span), Some(top)) = (span, open.last_mut()) {
                grow(&mut top.1, span);
            }
        }
        let shift = frames.iter().map(|f| -f.0).max().unwrap_or(0).max(0);
        self.frames = frames
            .into_iter()
            .map(|(l, r)| ((l + shift) as usize, (r + shift) as usize))
            .collect();
        for c in &mut self.centers {
            *c += shift as usize;
        }
    }
}

/// The columns a note `w` wide takes.
fn note_span(at: NoteAt, w: usize, c: &impl Fn(Who) -> isize) -> (isize, isize) {
    let w = w as isize;
    match at {
        NoteAt::RightOf(a) => (c(a) + 2, c(a) + 1 + w),
        NoteAt::LeftOf(a) => (c(a) - 1 - w, c(a) - 2),
        NoteAt::Over(a, b) if a == b => (c(a) - w / 2, c(a) - w / 2 + w - 1),
        NoteAt::Over(a, b) => {
            let (l, r) = (c(a) - 2, c(b) + 2);
            let extra = (w - (r - l + 1)).max(0);
            (l - extra / 2, r + extra - extra / 2)
        }
    }
}

struct Painter<'a> {
    s: &'a Sequence,
    plan: Plan,
    canvas: Canvas,
    notes: Vec<Text>,
    ascii: bool,
    row: usize,
}

impl<'a> Painter<'a> {
    fn new(s: &'a Sequence, plan: Plan, ascii: bool) -> Self {
        let canvas = Canvas::new(plan.width, ascii);
        Self {
            s,
            plan,
            canvas,
            notes: Vec::new(),
            ascii,
            row: 0,
        }
    }

    fn c(&self, w: Who) -> usize {
        self.plan.centers[usize::from(w)]
    }

    fn paint(mut self) -> Drawn {
        let s = self.s;
        for line in s.title.iter().filter(|l| !l.is_empty()) {
            self.canvas.label(self.row, 0, line, StyleKey::DiagramActor);
            self.row += 1;
        }
        if self.row > 0 {
            self.row += 1;
        }
        let header_top = self.row;
        let lines = s
            .participants
            .iter()
            .map(|p| p.label.len())
            .max()
            .unwrap_or(1);
        let bottom = header_top + lines + 1;
        for (i, p) in s.participants.iter().enumerate() {
            let (c, w) = (self.plan.centers[i], self.plan.boxes[i]);
            let l = c - w / 2;
            self.canvas
                .rect(header_top, l, bottom, l + w - 1, Pen::ROUND, LINE);
            for (k, line) in p.label.iter().enumerate() {
                let x = l + (w - width(line)) / 2;
                self.canvas
                    .label(header_top + 1 + k, x, line, StyleKey::DiagramActor);
            }
        }
        self.row = bottom + 2;
        let mut frames = self.plan.frames.clone().into_iter();
        let mut open: Vec<((usize, usize), usize)> = Vec::new();
        // per participant: how deep its activations nest, and the row the outermost began on
        let mut active = vec![(0u32, 0usize); s.participants.len()];
        // (participant, first row, last row) of each outermost activation
        let mut bars: Vec<(usize, usize, usize)> = Vec::new();
        for (i, e) in s.events.iter().enumerate() {
            match e {
                Event::Message(m) if m.from == m.to => self.self_message(m),
                Event::Message(m) => self.message(m),
                Event::Note { at, text } => self.note(*at, text),
                Event::Begin { kind, label } => {
                    let f = frames.next().unwrap_or_default();
                    self.frame_label(f, kind, label, false);
                    open.push((f, self.row));
                    self.row += 1;
                }
                Event::Divider { kind, label } => {
                    if let Some(&(f, _)) = open.last() {
                        self.frame_label(f, kind, label, true);
                        self.row += 1;
                    }
                }
                Event::End => {
                    if let Some(((l, r), top)) = open.pop() {
                        self.canvas.rect(top, l, self.row, r, Pen::SOLID, LINE);
                        self.row += 1;
                    }
                }
                Event::Activation { who, on } => {
                    // an arrow's `+` starts on the arrow's row; `activate` on the next one
                    let after_arrow = i > 0 && matches!(s.events[i - 1], Event::Message(_));
                    let (depth, since) = &mut active[usize::from(*who)];
                    if *on {
                        if *depth == 0 {
                            *since = if after_arrow { self.row - 1 } else { self.row };
                        }
                        *depth += 1;
                    } else if *depth > 0 {
                        *depth -= 1;
                        if *depth == 0 {
                            bars.push((usize::from(*who), *since, self.row.saturating_sub(1)));
                        }
                    }
                }
            }
        }
        let last = self.row;
        for (who, &(depth, since)) in active.iter().enumerate() {
            if depth > 0 {
                bars.push((who, since, last));
            }
        }
        self.canvas.grow(last + 1);
        for i in 0..s.participants.len() {
            let c = self.plan.centers[i];
            self.canvas.vline(c, bottom, last, Pen::SOLID, LINE);
        }
        // an active participant's lifeline is heavy (ada-mermaid Q-1)
        for (who, from, to) in bars {
            self.canvas
                .thicken(self.plan.centers[who], from.max(bottom + 1), to.max(from));
        }
        Drawn {
            canvas: self.canvas,
            header: Some(header_top..bottom + 1),
            notes: self.notes,
        }
    }

    /// Fit a message's label to `cap`, registering what was cut, with the marker that ends the
    /// cut row (empty when nothing was). The number stays out: [`Painter::label_line`] puts it
    /// in front of row 0.
    fn fitted(&mut self, m: &Message, cap: Option<usize>) -> (Vec<Text>, String) {
        let label = Label::of(m);
        let (mut out, was_cut) = label.fit(cap);
        let mut mark = String::new();
        if was_cut && let (Some(cap), Some(last)) = (cap, out.pop()) {
            self.notes.push(Text::join(&m.label));
            mark = marker(self.notes.len());
            let room = label.inner(cap).saturating_sub(width(&mark));
            let keep = cut(&last, room).trim_end_matches([' ', '(', '{', '[', '…']);
            out.push(last.slice(0..keep.len()));
        }
        (out, mark)
    }

    /// Write row `k` of a message's label from `x`, the number first on row 0 and `mark`, in
    /// the number's style like the note it points to, after it.
    fn label_line(&mut self, row: usize, x: usize, m: &Message, k: usize, line: &Text, mark: &str) {
        let mut x = x;
        if let (0, Some(n)) = (k, m.number) {
            x = self
                .canvas
                .text(row, x, &n.to_string(), StyleKey::DiagramNumber);
            // written, so a lifeline under it doesn't show through
            x = self.canvas.text(row, x, " ", StyleKey::DiagramLabel);
        }
        x = self.canvas.label(row, x, line, StyleKey::DiagramLabel);
        self.canvas.text(row, x, mark, StyleKey::DiagramNumber);
    }

    /// The columns row `k` of a message's label takes, its marker included.
    fn label_width(m: &Message, k: usize, line: &str, mark: &str) -> usize {
        let number = if k == 0 { Label::of(m).number } else { 0 };
        number + width(line) + width(mark)
    }

    fn message(&mut self, m: &Message) {
        let (a, b) = (self.c(m.from), self.c(m.to));
        let (lo, hi) = (a.min(b), a.max(b));
        let span = hi - lo - 1;
        // the cap made the layout fit; between these two lifelines there may be more room
        let cap = self.plan.cap.map(|c| c.max(span.saturating_sub(2)));
        let (lines, mark) = self.fitted(m, cap);
        for (k, line) in lines.iter().enumerate() {
            let mark = if k + 1 == lines.len() { &*mark } else { "" };
            let w = Self::label_width(m, k, line, mark);
            if w == 0 {
                continue;
            }
            let x = lo + 1 + span.saturating_sub(w) / 2;
            self.label_line(self.row, x, m, k, line, mark);
            self.row += 1;
        }
        let row = self.row;
        let pen = if m.arrow.dotted {
            Pen::DASHED
        } else {
            Pen::SOLID
        };
        let right = b > a;
        let (head_r, head_l) = self.heads(m.arrow.head);
        // the line leaves the sender's lifeline and stops short of the receiver's, where the
        // head is; without a head it meets the lifeline
        let to = match (m.arrow.head, right) {
            (Head::None, _) => b,
            (_, true) => b - 1,
            (_, false) => b + 1,
        };
        let from = if m.arrow.both {
            if right { a + 1 } else { a - 1 }
        } else {
            a
        };
        self.canvas.hline(row, from, to, pen, LINE);
        if m.arrow.head != Head::None {
            let head = if right { head_r } else { head_l };
            self.canvas.glyph(row, to, head, LINE);
            if m.arrow.both {
                self.canvas
                    .glyph(row, from, if right { head_l } else { head_r }, LINE);
            }
        }
        self.row += 1;
    }

    /// `[right, left]` heads.
    fn heads(&self, head: Head) -> (char, char) {
        match (head, self.ascii) {
            (Head::Filled, false) => ('▶', '◀'),
            (Head::Open, false) => ('▷', '◁'),
            (Head::Cross, false) => ('×', '×'),
            (Head::Cross, true) => ('x', 'x'),
            _ => ('>', '<'),
        }
    }

    fn self_message(&mut self, m: &Message) {
        let (lines, mark) = self.fitted(m, self.plan.cap);
        let c = self.c(m.from);
        let (top, pen) = (
            self.row,
            if m.arrow.dotted {
                Pen::DASHED
            } else {
                Pen::SOLID
            },
        );
        self.canvas.hline(top, c, c + 3, pen, LINE);
        self.canvas.vline(c + 3, top, top + 1, pen, LINE);
        self.canvas.hline(top + 1, c + 1, c + 3, pen, LINE);
        if m.arrow.head != Head::None {
            self.canvas
                .glyph(top + 1, c + 1, self.heads(m.arrow.head).1, LINE);
        }
        for (k, line) in lines.iter().enumerate() {
            let mark = if k + 1 == lines.len() { &*mark } else { "" };
            self.label_line(top + k, c + 5, m, k, line, mark);
        }
        self.row += lines.len().max(2);
    }

    fn note(&mut self, at: NoteAt, text: &[Text]) {
        let lines = self.fitted_note(text);
        let w = lines.iter().map(|l| width(l)).max().unwrap_or(0) + 4;
        let c = |who: Who| self.plan.centers[usize::from(who)] as isize;
        let (l, r) = note_span(at, w, &c);
        let (l, r) = (l.max(0) as usize, r.max(0) as usize);
        let (top, bottom) = (self.row, self.row + lines.len() + 1);
        // blank text inside, so lifelines pass behind the note
        for row in top + 1..bottom {
            for x in l + 1..r {
                self.canvas.glyph(row, x, ' ', StyleKey::DiagramNote);
            }
        }
        self.canvas.rect(top, l, bottom, r, Pen::ROUND, LINE);
        for (k, line) in lines.iter().enumerate() {
            let x = l + 1 + (r - l - 1).saturating_sub(width(line)) / 2;
            self.canvas
                .label(top + 1 + k, x, line, StyleKey::DiagramNote);
        }
        self.row = bottom + 1;
    }

    /// Notes wrap at the cap but are never cut.
    fn fitted_note(&self, text: &[Text]) -> Vec<Text> {
        match self.plan.cap {
            Some(cap) if Label::note(text).natural > cap => {
                text.iter().flat_map(|l| wrap(l, cap)).collect()
            }
            _ => text.to_vec(),
        }
    }

    /// A frame's top edge with its kind and label, or (`divider`) a dashed line across it.
    fn frame_label(&mut self, (l, r): (usize, usize), kind: &str, label: &Text, divider: bool) {
        let row = self.row;
        if divider {
            self.canvas.hline(row, l, r, Pen::DASHED, LINE);
        }
        let x = self.canvas.text(row, l + 2, " ", LINE);
        let x = self.canvas.text(row, x, kind, StyleKey::DiagramFrame);
        let x = self.canvas.text(row, x, " ", LINE);
        if !label.is_empty() {
            let x = self.canvas.label(row, x, label, StyleKey::DiagramLabel);
            self.canvas.text(row, x, " ", LINE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagram::{Diagram, parse};

    fn drawn(src: &str, avail: usize) -> (Vec<String>, Drawn) {
        let Some(Diagram::Sequence(s)) = parse(src) else {
            panic!("didn't parse");
        };
        let d = draw(&s, avail, false).expect("draws");
        let rows = (0..d.canvas.height())
            .map(|r| d.canvas.row_text(r))
            .collect();
        (rows, d)
    }

    #[test]
    fn measured_pieces_wrap_as_wrap_size_does() {
        let labels: &[&[&str]] = &[
            &["Fetch the order (with its lines), then ask the warehouse"],
            &["one", "", "two   spaced  out", "   "],
            &["a/b/c-d,e [f] {g} (h)", "日本語のラベル と 幅"],
            &["", "x"],
            &["supercalifragilistic word"],
        ];
        for lines in labels {
            let lines: Vec<Text> = lines.iter().map(|&l| Text::from(l)).collect();
            let pieces = Pieces::of(&lines);
            for max in pieces.longest.max(1)..80 {
                let want = lines
                    .iter()
                    .map(|l| wrap_size(l, max))
                    .fold((0, 0), |(r, w), (r2, w2)| (r + r2, w.max(w2)));
                assert_eq!(
                    pieces.wrap_size(max, usize::MAX),
                    want,
                    "{lines:?} at {max}"
                );
                // stopped early, it still counts past the limit
                let (rows, _) = pieces.wrap_size(max, 1);
                assert_eq!(rows > 1, want.0 > 1, "{lines:?} at {max}");
            }
        }
    }

    #[test]
    fn two_participants_and_a_reply() {
        let (rows, d) = drawn("sequenceDiagram\nA->>B: hi\nB-->>A: yo", 80);
        insta::assert_snapshot!(rows.join("\n"), @r"
        ╭───╮  ╭───╮
        │ A │  │ B │
        ╰─┬─╯  ╰─┬─╯
          │      │
          │  hi  │
          ├─────▶│
          │  yo  │
          │◀┄┄┄┄┄┤
          │      │
        ");
        assert_eq!(d.header, Some(0..3));
    }

    #[test]
    fn self_messages_notes_and_frames() {
        let src = "sequenceDiagram\nautonumber\nloop retry\nA->>A: cache it\nend\n\
                   Note over A,B: shared\nalt ok\nA-xB: go\nelse\nA-)B: later\nend";
        let (rows, _) = drawn(src, 80);
        insta::assert_snapshot!(rows.join("\n"));
    }

    #[test]
    fn active_lifelines_are_heavy() {
        let src = "sequenceDiagram\nA->>+B: call\nB->>+B: recurse\nB-->>-B: done\nB-->>-A: reply\nactivate A\nA->>B: open";
        let (rows, _) = drawn(src, 80);
        insta::assert_snapshot!(rows.join("\n"));
    }

    #[test]
    fn long_labels_wrap_then_cut_to_fit() {
        let src = "sequenceDiagram\nA->>B: POST /internal/fetch-payloads (… + wrappingKey{id, publicKey, alg}) \
                   and a lot more words than two rows can hold at all\nB-->>A: short";
        let (rows, d) = drawn(src, 40);
        assert!(rows.iter().all(|r| width(r) <= 40), "{rows:#?}");
        assert_eq!(d.notes.len(), 1);
        assert!(d.notes[0].starts_with("POST /internal"));
        assert!(rows.iter().any(|r| r.contains("…¹")), "{rows:#?}");
        // the marker takes the number's style, as the note it points to does
        let mut marks = Vec::new();
        for row in 0..d.canvas.height() {
            d.canvas.pieces(row, LINE, &mut |t, _, k| {
                if t.contains('¹') {
                    marks.push((t.to_owned(), k));
                }
            });
        }
        assert_eq!(marks, [("…¹".to_owned(), StyleKey::DiagramNumber)]);
        // fits at its natural width: nothing cut
        let (_, d) = drawn(src, 400);
        assert!(d.notes.is_empty());
    }

    #[test]
    fn too_many_participants_for_the_width_scroll_instead() {
        let src: String = (0..12)
            .map(|i| format!("P{i}->>P{}: message {i}\n", i + 1))
            .collect();
        let (rows, _) = drawn(&format!("sequenceDiagram\n{src}"), 30);
        assert!(rows.iter().any(|r| width(r) > 30));
    }

    #[test]
    fn ascii_glyphs() {
        let Some(Diagram::Sequence(s)) = parse("sequenceDiagram\nA->>B: hi") else {
            panic!();
        };
        let d = draw(&s, 80, true).unwrap();
        let rows: Vec<_> = (0..d.canvas.height())
            .map(|r| d.canvas.row_text(r))
            .collect();
        insta::assert_snapshot!(rows.join("\n"), @r"
        +---+  +---+
        | A |  | B |
        +-+-+  +-+-+
          |      |
          |  hi  |
          +----->|
          |      |
        ");
    }
}
