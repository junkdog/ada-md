//! The sixel wire format, for terminals without kitty graphics (the clean-slate plan's CS-3): an
//! image cut to at most 255 colors, then drawn six pixel rows at a time, one pass per color.
//!
//! The palette is made once per image ([`Palette::new`]) and each encoding maps pixels to it
//! ([`Palette::encode`]), so a scroll that shows other rows of the image encodes those rows
//! again without choosing colors again. [`Indexed`] maps the whole image once, which leaves a
//! scroll only the bands to write: the TUI's images.
//!
//! <https://vt100.net/docs/vt3xx-gp/chapter14.html>

use std::borrow::Cow;
use std::ops::Range;
use std::sync::{Mutex, PoisonError};

use image::DynamicImage;

/// The most colors a palette holds: index 255 marks a pixel left out (transparent).
pub const MAX_COLORS: usize = 255;

const TRANSPARENT: u8 = 255;

/// Bits per channel of the histogram the palette is chosen from.
const HIST_BITS: u32 = 5;

/// Bits per channel of the cache of nearest colors that an encoding fills as it goes.
const CACHE_BITS: u32 = 6;

/// Rounds of k-means after the palette's boxes are cut.
const REFINE: usize = 2;

/// How pixels between two of the palette's colors are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dither {
    /// Each pixel takes its nearest color.
    None,
    /// Floyd–Steinberg, scanning rows in alternate directions, with 7/8 of the error passed
    /// on: icy_sixel's default, as ratatui-image encoded.
    FloydSteinberg,
}

/// The colors an image is drawn with, as the terminal shows them: sixel names a color by
/// percentages, so each is already rounded to the 101 levels that survive.
pub struct Palette {
    colors: Vec<[u8; 3]>,
}

impl Palette {
    /// A palette of at most `colors` colors (2 to [`MAX_COLORS`]) for `image`'s opaque pixels.
    ///
    /// Variance-based median cut on a 5-bit histogram: the box with the largest squared error is
    /// cut across its widest axis where the two halves' error is least, as Wu's quantizer does,
    /// then refined by k-means.
    pub fn new(image: &DynamicImage, colors: usize) -> Palette {
        let colors = colors.clamp(2, MAX_COLORS);
        let (bytes, ch) = rgb_or_rgba(image);
        let mut points = histogram(&bytes, ch);
        let boxes = median_cut(&mut points, colors);
        let mut palette: Vec<[f32; 3]> = boxes.iter().map(Moments::mean).collect();
        for _ in 0..REFINE {
            refine(&points, &mut palette);
        }
        Palette {
            colors: palette.iter().map(|&c| shown(c)).collect(),
        }
    }

    /// The colors, as the terminal shows them.
    pub fn colors(&self) -> &[[u8; 3]] {
        &self.colors
    }

    /// Rows `rows` of `image` as a sixel image, appended to `out`: the DCS sequence, from its
    /// introducer to its string terminator. Pixels with alpha below 128 are left out and show
    /// what was beneath them. Nothing is written when there are no pixels.
    pub fn encode(
        &self,
        image: &DynamicImage,
        rows: Range<u32>,
        dither: Dither,
        out: &mut Vec<u8>,
    ) {
        let rows = rows.start.min(image.height())..rows.end.min(image.height());
        let (w, h) = (image.width() as usize, rows.len());
        if w == 0 || h == 0 {
            return;
        }
        let (bytes, ch) = rgb_or_rgba(image);
        let bytes = &bytes[rows.start as usize * w * ch..rows.end as usize * w * ch];
        let indices = self.map(bytes, ch, w, dither);
        self.write(&indices, w, out);
    }

    /// `indices`, rows of `w` pixels, as a sixel image.
    fn write(&self, indices: &[u8], w: usize, out: &mut Vec<u8>) {
        self.header(w, indices.len() / w, &used(indices), out);
        bands(indices, w, self.colors.len(), out);
        out.extend_from_slice(b"\x1b\\");
    }

    /// A sixel image's start, up to its first band: its size, `w` × `h`, and the colors `used`.
    fn header(&self, w: usize, h: usize, used: &[bool; MAX_COLORS], out: &mut Vec<u8>) {
        // P1 = 9: square pixels, which Windows Terminal reads from P1 alone; P2 = 1: pixels not
        // drawn keep what's beneath them. The raster attributes say the same, and the size.
        out.extend_from_slice(b"\x1bP9;1;0q\"1;1;");
        push_num(out, w);
        out.push(b';');
        push_num(out, h);
        for (i, c) in self.colors.iter().enumerate().filter(|&(i, _)| used[i]) {
            out.push(b'#');
            push_num(out, i);
            out.extend_from_slice(b";2");
            for &v in c {
                out.push(b';');
                push_num(out, percent(v).into());
            }
        }
    }

    /// Each pixel's color index, or [`TRANSPARENT`].
    fn map(&self, bytes: &[u8], ch: usize, w: usize, dither: Dither) -> Vec<u8> {
        let mut indices = vec![TRANSPARENT; bytes.len() / ch];
        if self.colors.is_empty() {
            return indices;
        }
        let mut nearest = Nearest::new(&self.colors);
        let opaque = |p: &[u8]| ch == 3 || p[3] >= 128;
        match dither {
            Dither::None => {
                for (p, i) in bytes.chunks_exact(ch).zip(&mut indices) {
                    if opaque(p) {
                        *i = nearest.cached([p[0], p[1], p[2]]);
                    }
                }
            }
            Dither::FloydSteinberg => {
                // errors in 1/128ths of a level, for this row and the one below, with a cell of
                // margin each side: 7/16, 3/16, 5/16 and 1/16 of 7/8 of a pixel's error
                let mut cur = vec![[0i32; 3]; w + 2];
                let mut below = vec![[0i32; 3]; w + 2];
                let rows = bytes.chunks_exact(w * ch).zip(indices.chunks_exact_mut(w));
                for (y, (row, out)) in rows.enumerate() {
                    let back = y % 2 == 1;
                    for step in 0..w {
                        let x = if back { w - 1 - step } else { step };
                        let p = &row[x * ch..x * ch + ch];
                        if !opaque(p) {
                            continue;
                        }
                        let e = cur[x + 1];
                        let v = [0, 1, 2]
                            .map(|c| (i32::from(p[c]) + ((e[c] + 64) >> 7)).clamp(0, 255) as u8);
                        let i = nearest.cached(v);
                        out[x] = i;
                        let shown = self.colors[usize::from(i)];
                        let (ahead, behind) = if back { (x, x + 2) } else { (x + 2, x) };
                        for c in 0..3 {
                            let d = i32::from(v[c]) - i32::from(shown[c]);
                            cur[ahead][c] += d * 49;
                            below[behind][c] += d * 21;
                            below[x + 1][c] += d * 35;
                            below[ahead][c] += d * 7;
                        }
                    }
                    std::mem::swap(&mut cur, &mut below);
                    below.fill([0; 3]);
                }
            }
        }
        indices
    }
}

/// An image mapped to its palette once, so that any part of it is drawn as sixels without
/// mapping it again. Dithered, the rows are the same pixels wherever the image is cut, so a
/// scroll doesn't make them shimmer.
///
/// A scroll by whole cells starts the image at a row a cell's height further on, so its bands
/// start at one of a few offsets from the top: a cell's height steps through at most six. Each
/// offset's bands are written once, by [`Indexed::prepare`] or the first cut that needs them,
/// and a cut joins those it shows, writing only its last band cut short.
pub struct Indexed {
    palette: Palette,
    width: u32,
    height: u32,
    indices: Vec<u8>,
    /// The colors the image uses.
    used: [bool; MAX_COLORS],
    /// The bands that start at each offset from the top.
    banded: Mutex<[Option<Banded>; 6]>,
}

/// The image's bands from a row, each one followed by `-` in `data`, where `ends` has the end
/// of each.
struct Banded {
    data: Vec<u8>,
    ends: Vec<usize>,
}

impl Indexed {
    /// `image` in a palette of at most `colors` colors of its own.
    pub fn new(image: &DynamicImage, colors: usize, dither: Dither) -> Indexed {
        let palette = Palette::new(image, colors);
        let (bytes, ch) = rgb_or_rgba(image);
        let indices = palette.map(&bytes, ch, image.width() as usize, dither);
        Indexed {
            palette,
            width: image.width(),
            height: image.height(),
            used: used(&indices),
            indices,
            banded: Mutex::default(),
        }
    }

    /// The size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Rows `rows` of the image, cut to its first `width` columns, as a sixel image appended to
    /// `out`: the pixels [`Palette::encode`] writes for them. Nothing is written when there are
    /// no pixels.
    pub fn encode(&self, rows: Range<u32>, width: u32, out: &mut Vec<u8>) {
        let rows = rows.start.min(self.height) as usize..rows.end.min(self.height) as usize;
        let (full, w) = (self.width as usize, width.min(self.width) as usize);
        if w == 0 || rows.is_empty() {
            return;
        }
        let indices = &self.indices[rows.start * full..rows.end * full];
        // cut on the right, which only a window narrower than the image does: written whole
        if w < full {
            let cut: Vec<u8> = indices
                .chunks_exact(full)
                .flat_map(|row| &row[..w])
                .copied()
                .collect();
            self.palette.write(&cut, w, out);
            return;
        }
        self.palette.header(w, rows.len(), &self.used, out);
        let offset = rows.start % 6;
        let mut banded = self.banded.lock().unwrap_or_else(PoisonError::into_inner);
        let banded = banded[offset].get_or_insert_with(|| self.bands_from(offset));
        // the bands that end inside the rows, the image's last however short; then the rest,
        // cut short at the rows' end
        let first = rows.start / 6;
        let (last, rest) = if rows.end == self.height as usize {
            (banded.ends.len(), rows.end..rows.end)
        } else {
            let last = (rows.end - offset) / 6;
            (last, offset + last * 6..rows.end)
        };
        let start = if first == 0 { 0 } else { banded.ends[first - 1] };
        if last > first {
            out.extend_from_slice(&banded.data[start..banded.ends[last - 1]]);
        }
        if rest.is_empty() {
            // the last band's `-`
            out.pop_if(|b| *b == b'-');
        } else {
            let mut sixels = Sixels::new(w, self.palette.colors.len());
            sixels.band(&self.indices[rest.start * full..rest.end * full], out);
        }
        out.extend_from_slice(b"\x1b\\");
    }

    /// Write the bands of every cut that starts on a multiple of `every` rows (a cell's height),
    /// so that a scroll by cells writes none: each offset of `every`'s multiples from a band.
    pub fn prepare(&self, every: u32) {
        let every = every.max(1) as usize;
        let mut banded = self.banded.lock().unwrap_or_else(PoisonError::into_inner);
        let mut offset = 0;
        loop {
            banded[offset].get_or_insert_with(|| self.bands_from(offset));
            offset = (offset + every) % 6;
            if offset == 0 || every >= self.height as usize {
                break;
            }
        }
    }

    /// The bands that start at row `offset`, each followed by `-`.
    fn bands_from(&self, offset: usize) -> Banded {
        let w = self.width as usize;
        let mut sixels = Sixels::new(w, self.palette.colors.len());
        let (mut data, mut ends) = (Vec::new(), Vec::new());
        for rows in self.indices[(offset * w).min(self.indices.len())..].chunks(w * 6) {
            sixels.band(rows, &mut data);
            data.push(b'-');
            ends.push(data.len());
        }
        Banded { data, ends }
    }
}

/// The colors `indices` uses.
fn used(indices: &[u8]) -> [bool; MAX_COLORS] {
    let mut used = [false; MAX_COLORS];
    for &i in indices {
        if i != TRANSPARENT {
            used[usize::from(i)] = true;
        }
    }
    used
}

/// `image`'s bytes and channels: RGB or RGBA as they are, anything else as RGBA.
fn rgb_or_rgba(image: &DynamicImage) -> (Cow<'_, [u8]>, usize) {
    match image {
        DynamicImage::ImageRgb8(i) => (Cow::Borrowed(i.as_raw()), 3),
        DynamicImage::ImageRgba8(i) => (Cow::Borrowed(i.as_raw()), 4),
        _ => (Cow::Owned(image.to_rgba8().into_raw()), 4),
    }
}

/// Six pixel rows at a time, each color used in them as one pass of sixels from the first column
/// it's in to the last, the passes joined by `$` (back to the band's first column) and the bands
/// by `-`.
fn bands(indices: &[u8], w: usize, colors: usize, out: &mut Vec<u8>) {
    let mut sixels = Sixels::new(w, colors);
    for (band, rows) in indices.chunks(w * 6).enumerate() {
        if band > 0 {
            out.push(b'-');
        }
        sixels.band(rows, out);
    }
}

/// Bands of `w` pixels, one at a time: each color's six bits per column in the band at hand,
/// and the columns it spans.
struct Sixels {
    w: usize,
    sixels: Vec<u8>,
    span: Vec<(usize, usize)>,
    used: Vec<usize>,
}

impl Sixels {
    fn new(w: usize, colors: usize) -> Sixels {
        Sixels {
            w,
            sixels: vec![0; colors * w],
            span: vec![(usize::MAX, 0); colors],
            used: Vec::with_capacity(colors),
        }
    }

    /// One band: up to six rows of indices, each color in them as one pass.
    fn band(&mut self, rows: &[u8], out: &mut Vec<u8>) {
        let w = self.w;
        for (bit, row) in rows.chunks_exact(w).enumerate() {
            for (x, &i) in row.iter().enumerate() {
                if i == TRANSPARENT {
                    continue;
                }
                let i = usize::from(i);
                self.sixels[i * w + x] |= 1 << bit;
                let s = &mut self.span[i];
                if s.0 == usize::MAX {
                    self.used.push(i);
                }
                *s = (s.0.min(x), s.1.max(x));
            }
        }
        for (n, &i) in self.used.iter().enumerate() {
            if n > 0 {
                out.push(b'$');
            }
            out.push(b'#');
            push_num(out, i);
            let (first, last) = self.span[i];
            run(out, 0, first);
            let row = &mut self.sixels[i * w + first..=i * w + last];
            let mut x = 0;
            while x < row.len() {
                let bits = row[x];
                let n = row[x..].iter().take_while(|&&b| b == bits).count();
                run(out, bits, n);
                x += n;
            }
            row.fill(0);
            self.span[i] = (usize::MAX, 0);
        }
        self.used.clear();
    }
}

/// `n` sixels of `bits`, as a repeat when that's shorter.
fn run(out: &mut Vec<u8>, bits: u8, n: usize) {
    let c = b'?' + bits;
    if n > 3 {
        out.push(b'!');
        push_num(out, n);
        out.push(c);
    } else {
        out.extend(std::iter::repeat_n(c, n));
    }
}

fn push_num(out: &mut Vec<u8>, mut n: usize) {
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    out.extend_from_slice(&buf[i..]);
}

/// A level as the percentage sixel names it by.
fn percent(v: u8) -> u8 {
    ((u32::from(v) * 100 + 127) / 255) as u8
}

/// A color as the terminal shows it, after the round trip through percentages.
fn shown(c: [f32; 3]) -> [u8; 3] {
    c.map(|v| {
        let p = percent(v.round().clamp(0.0, 255.0) as u8);
        ((u32::from(p) * 255 + 50) / 100) as u8
    })
}

/// A histogram bin: how many pixels fell in it, and their mean color.
#[derive(Clone, Copy)]
struct Point {
    n: f32,
    c: [f32; 3],
}

/// The opaque pixels' colors, binned to [`HIST_BITS`] bits per channel: each populated bin with
/// the mean of the pixels in it, so a color with a bin to itself keeps its exact value.
fn histogram(bytes: &[u8], ch: usize) -> Vec<Point> {
    let shift = 8 - HIST_BITS;
    let mut bins = vec![(0u32, [0u64; 3]); 1 << (3 * HIST_BITS)];
    for p in bytes.chunks_exact(ch) {
        if ch == 4 && p[3] < 128 {
            continue;
        }
        let key = (usize::from(p[0] >> shift) << (2 * HIST_BITS))
            | (usize::from(p[1] >> shift) << HIST_BITS)
            | usize::from(p[2] >> shift);
        let bin = &mut bins[key];
        bin.0 += 1;
        for (sum, &v) in bin.1.iter_mut().zip(p) {
            *sum += u64::from(v);
        }
    }
    bins.iter()
        .filter(|b| b.0 > 0)
        .map(|&(n, sum)| Point {
            n: n as f32,
            c: sum.map(|s| (s as f64 / f64::from(n)) as f32),
        })
        .collect()
}

/// A set of points' weight, sums and sums of squares per channel, in f64: a 24 MP photo's sums
/// of squares are past an f32's precision.
#[derive(Clone, Copy, Default)]
struct Moments {
    n: f64,
    s: [f64; 3],
    q: [f64; 3],
}

impl Moments {
    fn of(points: &[Point]) -> Moments {
        let mut m = Moments::default();
        points.iter().for_each(|p| m.add(p));
        m
    }

    fn add(&mut self, p: &Point) {
        let n = f64::from(p.n);
        self.n += n;
        for c in 0..3 {
            let v = f64::from(p.c[c]);
            self.s[c] += n * v;
            self.q[c] += n * v * v;
        }
    }

    fn minus(&self, o: &Moments) -> Moments {
        Moments {
            n: self.n - o.n,
            s: [0, 1, 2].map(|c| self.s[c] - o.s[c]),
            q: [0, 1, 2].map(|c| self.q[c] - o.q[c]),
        }
    }

    /// Each channel's squared error about the mean.
    fn spread(&self) -> [f64; 3] {
        if self.n == 0.0 {
            return [0.0; 3];
        }
        [0, 1, 2].map(|c| self.q[c] - self.s[c] * self.s[c] / self.n)
    }

    /// The squared error of the points about their mean.
    fn error(&self) -> f64 {
        self.spread().iter().sum()
    }

    fn mean(&self) -> [f32; 3] {
        self.s.map(|s| (s / self.n) as f32)
    }
}

/// A box of the median cut: a range of the points, and their moments.
struct Cut {
    range: Range<usize>,
    m: Moments,
}

/// Cut `points` into at most `colors` boxes, reordering them so each box is a range, and return
/// each box's moments.
fn median_cut(points: &mut [Point], colors: usize) -> Vec<Moments> {
    if points.is_empty() {
        return Vec::new();
    }
    let mut boxes = vec![Cut {
        range: 0..points.len(),
        m: Moments::of(points),
    }];
    // boxes that can't be cut further
    let mut done = Vec::new();
    while boxes.len() + done.len() < colors {
        let Some(i) =
            (0..boxes.len()).max_by(|&a, &b| boxes[a].m.error().total_cmp(&boxes[b].m.error()))
        else {
            break;
        };
        let Cut { range, m: total } = boxes.swap_remove(i);
        let part = &mut points[range.clone()];
        let spread = total.spread();
        let axis = (0..3)
            .max_by(|&a, &b| spread[a].total_cmp(&spread[b]))
            .unwrap_or(0);
        part.sort_unstable_by(|a, b| a.c[axis].total_cmp(&b.c[axis]));
        let mut left = Moments::default();
        let mut best: Option<(usize, f64, Moments)> = None;
        for (k, p) in part[..part.len() - 1].iter().enumerate() {
            left.add(p);
            // points equal on the axis stay on one side
            if p.c[axis] == part[k + 1].c[axis] {
                continue;
            }
            let e = left.error() + total.minus(&left).error();
            if best.is_none_or(|(_, b, _)| e < b) {
                best = Some((k + 1, e, left));
            }
        }
        let Some((k, _, left)) = best else {
            // a single color
            done.push(total);
            continue;
        };
        let mid = range.start + k;
        boxes.push(Cut {
            range: range.start..mid,
            m: left,
        });
        boxes.push(Cut {
            range: mid..range.end,
            m: total.minus(&left),
        });
    }
    done.extend(boxes.into_iter().map(|b| b.m));
    done
}

/// One round of k-means: each point to its nearest color, each color to its points' mean. A
/// color no point is nearest to stays as it was.
fn refine(points: &[Point], palette: &mut [[f32; 3]]) {
    let colors: Vec<[u8; 3]> = palette.iter().map(|c| c.map(|v| v.round() as u8)).collect();
    let nearest = Nearest::new(&colors);
    let mut sums = vec![Moments::default(); palette.len()];
    for p in points {
        let i = nearest.find(p.c.map(|v| v.round() as i32));
        sums[usize::from(i)].add(p);
    }
    for (c, m) in palette.iter_mut().zip(&sums) {
        if m.n > 0.0 {
            *c = m.mean();
        }
    }
}

/// The nearest color of a palette, by squared distance in RGB: the palette sorted by red, and
/// searched outward from the color's red until red alone is farther than the best so far.
struct Nearest {
    sorted: Vec<([i32; 3], u8)>,
    /// The nearest color to each [`CACHE_BITS`] cell, or [`TRANSPARENT`] until it's asked for.
    cache: Vec<u8>,
}

impl Nearest {
    fn new(colors: &[[u8; 3]]) -> Nearest {
        let mut sorted: Vec<_> = colors
            .iter()
            .enumerate()
            .map(|(i, c)| (c.map(i32::from), i as u8))
            .collect();
        sorted.sort_unstable_by_key(|(c, _)| c[0]);
        Nearest {
            sorted,
            cache: Vec::new(),
        }
    }

    fn find(&self, c: [i32; 3]) -> u8 {
        let start = self.sorted.partition_point(|(p, _)| p[0] < c[0]);
        let (mut best, mut at) = (i32::MAX, 0);
        let mut visit = |(p, i): &([i32; 3], u8)| {
            if (p[0] - c[0]).pow(2) >= best {
                return false;
            }
            let d = (0..3).map(|k| (p[k] - c[k]).pow(2)).sum::<i32>();
            if d < best {
                (best, at) = (d, *i);
            }
            true
        };
        for p in &self.sorted[start..] {
            if !visit(p) {
                break;
            }
        }
        for p in self.sorted[..start].iter().rev() {
            if !visit(p) {
                break;
            }
        }
        at
    }

    /// The nearest color to the center of the [`CACHE_BITS`] cell `c` is in, found once per
    /// cell.
    fn cached(&mut self, c: [u8; 3]) -> u8 {
        if self.cache.is_empty() {
            self.cache = vec![TRANSPARENT; 1 << (3 * CACHE_BITS)];
        }
        let shift = 8 - CACHE_BITS;
        let key = (usize::from(c[0] >> shift) << (2 * CACHE_BITS))
            | (usize::from(c[1] >> shift) << CACHE_BITS)
            | usize::from(c[2] >> shift);
        if self.cache[key] == TRANSPARENT {
            let half = 1 << (shift - 1);
            self.cache[key] = self.find(c.map(|v| i32::from(v >> shift << shift) + half));
        }
        self.cache[key]
    }
}

#[cfg(test)]
mod tests {
    use image::{Rgb, RgbImage, Rgba, RgbaImage};

    use super::*;

    /// A sixel image decoded: its size and each pixel's color, `None` where nothing was drawn.
    fn decode(data: &[u8]) -> (usize, usize, Vec<Option<[u8; 3]>>) {
        let body = data
            .strip_prefix(b"\x1bP9;1;0q")
            .and_then(|d| d.strip_suffix(b"\x1b\\"))
            .expect("a DCS sixel sequence");
        let mut i = 0;
        let num = |i: &mut usize| {
            let start = *i;
            while *i < body.len() && body[*i].is_ascii_digit() {
                *i += 1;
            }
            std::str::from_utf8(&body[start..*i])
                .unwrap()
                .parse::<usize>()
                .unwrap()
        };
        assert_eq!(body[i], b'"');
        i += 1;
        let mut raster = [0; 4];
        for (k, r) in raster.iter_mut().enumerate() {
            if k > 0 {
                assert_eq!(body[i], b';');
                i += 1;
            }
            *r = num(&mut i);
        }
        let [_, _, w, h] = raster;
        let mut px = vec![None; w * h];
        let mut defs = std::collections::HashMap::new();
        let (mut color, mut x, mut band) = (0, 0, 0);
        while i < body.len() {
            let b = body[i];
            i += 1;
            match b {
                b'#' => {
                    color = num(&mut i);
                    if body.get(i) == Some(&b';') {
                        i += 1;
                        assert_eq!(num(&mut i), 2, "RGB");
                        let mut rgb = [0u8; 3];
                        for v in &mut rgb {
                            i += 1;
                            let p = num(&mut i);
                            assert!(p <= 100);
                            *v = ((p * 255 + 50) / 100) as u8;
                        }
                        defs.insert(color, rgb);
                    }
                }
                b'$' => x = 0,
                b'-' => (x, band) = (0, band + 1),
                b'!' | b'?'..=b'~' => {
                    let (n, bits) = if b == b'!' {
                        let n = num(&mut i);
                        i += 1;
                        (n, body[i - 1] - b'?')
                    } else {
                        (1, b - b'?')
                    };
                    for _ in 0..n {
                        for bit in 0..6 {
                            if bits & 1 << bit != 0 {
                                let y = band * 6 + bit;
                                assert!(x < w && y < h, "({x}, {y}) outside {w} × {h}");
                                px[y * w + x] = Some(defs[&color]);
                            }
                        }
                        x += 1;
                    }
                }
                _ => panic!("unexpected byte {b:#x} at {i}"),
            }
        }
        (w, h, px)
    }

    fn encode(image: &DynamicImage, colors: usize, dither: Dither) -> Vec<u8> {
        let mut out = Vec::new();
        Palette::new(image, colors).encode(image, 0..image.height(), dither, &mut out);
        out
    }

    /// Levels that survive the trip through percentages unchanged.
    const EXACT: [u8; 6] = [0, 51, 102, 153, 204, 255];

    #[test]
    fn few_colors_come_back_exactly() {
        // 6 × 6 × 6 = 216 colors, sizes not a multiple of 6
        let image = DynamicImage::ImageRgb8(RgbImage::from_fn(37, 23, |x, y| {
            let k = (x * 7 + y * 13) as usize % 216;
            Rgb([EXACT[k / 36], EXACT[k / 6 % 6], EXACT[k % 6]])
        }));
        for dither in [Dither::None, Dither::FloydSteinberg] {
            let (w, h, px) = decode(&encode(&image, MAX_COLORS, dither));
            assert_eq!((w, h), (37, 23));
            let want: Vec<_> = image
                .as_bytes()
                .chunks(3)
                .map(|p| Some([p[0], p[1], p[2]]))
                .collect();
            assert!(px == want, "{dither:?}");
        }
    }

    #[test]
    fn transparent_pixels_are_left_out() {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_fn(10, 9, |x, y| {
            let a = if (x + y) % 3 == 0 { 0 } else { 255 };
            Rgba([255, 0, 0, a])
        }));
        let (_, _, px) = decode(&encode(&image, 16, Dither::FloydSteinberg));
        for (k, p) in px.iter().enumerate() {
            let (x, y) = (k % 10, k / 10);
            let want = ((x + y) % 3 != 0).then_some([255, 0, 0]);
            assert_eq!(*p, want, "({x}, {y})");
        }
        // nothing opaque: no colors, no sixels
        let clear = DynamicImage::ImageRgba8(RgbaImage::new(4, 4));
        let out = encode(&clear, 16, Dither::None);
        let (_, _, px) = decode(&out);
        assert!(px.iter().all(Option::is_none));
        assert!(!out.contains(&b'#'));
    }

    #[test]
    fn rows_are_encoded_as_their_own_image() {
        let image = DynamicImage::ImageRgb8(RgbImage::from_fn(8, 30, |x, y| {
            Rgb([EXACT[(x % 6) as usize], EXACT[(y % 6) as usize], 0])
        }));
        let palette = Palette::new(&image, 64);
        let mut out = Vec::new();
        palette.encode(&image, 7..19, Dither::None, &mut out);
        let (w, h, px) = decode(&out);
        assert_eq!((w, h), (8, 12));
        for (k, p) in px.iter().enumerate() {
            let (x, y) = (k % 8, k / 8 + 7);
            assert_eq!(*p, Some([EXACT[x % 6], EXACT[y % 6], 0]));
        }
        // rows past the end are cut, and no rows is nothing
        out.clear();
        palette.encode(&image, 25..40, Dither::None, &mut out);
        assert_eq!(decode(&out).1, 5);
        out.clear();
        palette.encode(&image, 40..50, Dither::None, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn an_indexed_image_draws_any_rows_and_columns_of_it() {
        let image = DynamicImage::ImageRgb8(RgbImage::from_fn(9, 30, |x, y| {
            Rgb([EXACT[(x % 6) as usize], EXACT[(y % 6) as usize], 0])
        }));
        let indexed = Indexed::new(&image, 64, Dither::FloydSteinberg);
        assert_eq!(indexed.size(), (9, 30));
        // a cell 4 rows high starts the bands at offsets 0, 4 and 2; the rest come when asked for
        indexed.prepare(4);
        let ready = |i: &Indexed| i.banded.lock().unwrap().each_ref().map(Option::is_some);
        assert_eq!(ready(&indexed), [true, false, true, false, true, false]);
        // the same pixels as the whole image's, wherever it's cut
        let mut out = Vec::new();
        indexed.encode(0..30, 9, &mut out);
        let (_, _, whole) = decode(&out);
        // every cut: each offset of the bands, ends on a band and inside one, the image's end
        let all = (0..30).flat_map(|s| (s + 1..=30).map(move |e| (s..e, 9)));
        for (rows, width) in all.chain([(7..19, 9), (0..5, 4), (25..40, 20), (7..19, 9)]) {
            out.clear();
            indexed.encode(rows.clone(), width, &mut out);
            let (w, h, px) = decode(&out);
            let (rows, width) = (rows.start as usize..rows.end.min(30) as usize, width.min(9) as usize);
            assert_eq!((w, h), (width, rows.len()));
            for (k, p) in px.iter().enumerate() {
                let (x, y) = (k % w, k / w + rows.start);
                assert_eq!(*p, whole[y * 9 + x], "({x}, {y})");
                assert_eq!(*p, Some([EXACT[x % 6], EXACT[y % 6], 0]));
            }
        }
        for (rows, width) in [(30..40, 9), (3..3, 9), (0..6, 0)] {
            out.clear();
            indexed.encode(rows, width, &mut out);
            assert!(out.is_empty());
        }
    }

    #[test]
    fn runs_are_repeats() {
        let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(100, 12, Rgb([0, 0, 255])));
        let out = encode(&image, 4, Dither::None);
        let text = String::from_utf8(out).unwrap();
        // one color, two full bands
        assert!(text.ends_with("#0!100~-#0!100~\x1b\\"), "{text:?}");
    }

    #[test]
    fn a_pass_starts_at_its_colors_first_column() {
        // a blue column at x = 5 on the band's last row, a dot at x = 2 on its first: the pass
        // for blue starts at 2
        let image = DynamicImage::ImageRgba8(RgbaImage::from_fn(8, 6, |x, y| match (x, y) {
            (5, _) | (2, 0) => Rgba([0, 0, 255, 255]),
            _ => Rgba([0, 0, 0, 0]),
        }));
        let (_, _, px) = decode(&encode(&image, 4, Dither::None));
        for (k, p) in px.iter().enumerate() {
            let (x, y) = (k % 8, k / 8);
            assert_eq!(p.is_some(), x == 5 || (x, y) == (2, 0), "({x}, {y})");
        }
    }

    #[test]
    fn the_palette_fits_the_colors_asked_for() {
        let image = DynamicImage::ImageRgb8(RgbImage::from_fn(64, 64, |x, y| {
            Rgb([(x * 4) as u8, (y * 4) as u8, ((x + y) * 2) as u8])
        }));
        for colors in [2, 16, 255, 1000] {
            let palette = Palette::new(&image, colors);
            let n = palette.colors().len();
            assert!(n > 1 && n <= colors.min(MAX_COLORS), "{colors}: {n}");
        }
        // and close to a smooth gradient with all of them
        assert!(rms(&image, &encode(&image, 255, Dither::None)) < 6.0);
    }

    #[test]
    fn dithering_keeps_a_gradients_mean() {
        let image = DynamicImage::ImageRgb8(RgbImage::from_fn(120, 60, |x, _| {
            let v = (x * 2) as u8;
            Rgb([v, v, v])
        }));
        let mean = |px: &[Option<[u8; 3]>]| {
            px.iter().map(|p| f64::from(p.unwrap()[0])).sum::<f64>() / px.len() as f64
        };
        let (_, _, px) = decode(&encode(&image, 4, Dither::FloydSteinberg));
        let source = image
            .as_bytes()
            .chunks(3)
            .map(|p| f64::from(p[0]))
            .sum::<f64>()
            / 7200.0;
        assert!(
            (mean(&px) - source).abs() < 2.0,
            "{} vs {source}",
            mean(&px)
        );
        // and each column's mean, inside the palette's range, where four flat colors miss by up to 30
        let col = |x: usize| {
            (0..60)
                .map(|y| f64::from(px[y * 120 + x].unwrap()[0]))
                .sum::<f64>()
                / 60.0
        };
        for x in (20..100).step_by(10) {
            assert!((col(x) - (x * 2) as f64).abs() < 12.0, "{x}: {}", col(x));
        }
    }

    /// The root mean square error of `out`, decoded, against `image`.
    fn rms(image: &DynamicImage, out: &[u8]) -> f64 {
        let (_, _, px) = decode(out);
        let rgb = image.to_rgb8();
        let sum: f64 = rgb
            .pixels()
            .zip(&px)
            .flat_map(|(a, b)| {
                let b = b.unwrap();
                (0..3).map(move |c| (f64::from(a[c]) - f64::from(b[c])).powi(2))
            })
            .sum();
        (sum / (px.len() * 3) as f64).sqrt()
    }

    #[test]
    fn nearest_is_the_brute_force_nearest() {
        let colors: Vec<[u8; 3]> = (0..200u32)
            .map(|i| {
                [
                    (i * 37 % 256) as u8,
                    (i * 91 % 256) as u8,
                    (i * 53 % 256) as u8,
                ]
            })
            .collect();
        let nearest = Nearest::new(&colors);
        let dist =
            |a: [u8; 3], b: [i32; 3]| (0..3).map(|k| (i32::from(a[k]) - b[k]).pow(2)).sum::<i32>();
        for k in 0..5000u32 {
            let c = [
                (k * 13 % 256) as i32,
                (k * 7 % 256) as i32,
                (k * 29 % 256) as i32,
            ];
            let best = colors.iter().map(|&p| dist(p, c)).min().unwrap();
            assert_eq!(dist(colors[usize::from(nearest.find(c))], c), best, "{c:?}");
        }
    }

    #[test]
    fn percentages_round_trip_to_the_nearest_level() {
        for v in 0..=255u8 {
            let s = shown([f32::from(v); 3])[0];
            assert!((i32::from(s) - i32::from(v)).abs() <= 2, "{v} → {s}");
        }
        for v in EXACT {
            assert_eq!(shown([f32::from(v); 3]), [v; 3]);
        }
    }
}
