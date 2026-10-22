//! Decoding an image straight to the size it's shown at (architecture §9). A photo is decoded
//! once and box-filtered down as its rows arrive, so memory goes to the size shown, not to the
//! file's: a PNG is shrunk a row at a time and never held whole, and a JPEG is held once, as
//! decoded. Both TUI images and printed ones go through here.

use std::fs::File;
use std::io::BufReader;
use std::panic::AssertUnwindSafe;
use std::path::Path;

use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbImage, RgbaImage};

use crate::guard;

/// The largest size, in pixels, that fits `max` keeping `px`'s aspect ratio, and no larger than
/// `px`: images are only ever shrunk.
pub fn fitted(px: (u32, u32), max: (u32, u32)) -> (u32, u32) {
    let scale = (f64::from(max.0) / f64::from(px.0.max(1)))
        .min(f64::from(max.1) / f64::from(px.1.max(1)))
        .min(1.0);
    let side = |n: u32| ((f64::from(n) * scale).round() as u32).clamp(1, n.max(1));
    (side(px.0), side(px.1))
}

/// Decode the image at `path`, upright, shrunk to fit `max` pixels. The result is RGB, or RGBA
/// when the image has alpha.
pub fn decode(path: &Path, max: (u32, u32)) -> Result<DynamicImage, String> {
    decode_to(path, |px| fitted(px, max))
}

/// Decode the image at `path`, upright, at the size `size` picks from its upright pixel size. A
/// size larger than the image's stays the image's. A decoder's panic on a malformed file (zune-jpeg
/// and png each had one, fuzz round 7) is an error like any other (F-05).
pub fn decode_to(
    path: &Path,
    size: impl FnOnce((u32, u32)) -> (u32, u32),
) -> Result<DynamicImage, String> {
    guard::catch(AssertUnwindSafe(|| decode_file(path, size)))
        .unwrap_or_else(|| Err("the image failed to decode".into()))
}

fn decode_file(
    path: &Path,
    size: impl FnOnce((u32, u32)) -> (u32, u32),
) -> Result<DynamicImage, String> {
    let reader = ImageReader::open(path)
        .and_then(ImageReader::with_guessed_format)
        .map_err(|e| e.to_string())?;
    if reader.format() == Some(ImageFormat::Png) {
        return png(path, size);
    }
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    // held whole as decoded: a header can claim any size, 65,498 × 65,256 in 384 bytes (fuzz round 7)
    if decoder.total_bytes() > MAX_WHOLE as u64 {
        let (w, h) = decoder.dimensions();
        return Err(format!("the image is too large to decode: {w} × {h}"));
    }
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let image = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    let (w, h) = (image.width(), image.height());
    // turning a size is its own inverse: the upright target, turned, is the stored one
    let target = upright(size(upright((w, h), orientation)), orientation);
    let image = match image {
        DynamicImage::ImageRgb8(_)
        | DynamicImage::ImageRgba8(_)
        | DynamicImage::ImageLuma8(_)
        | DynamicImage::ImageLumaA8(_) => image,
        other if other.color().has_alpha() => DynamicImage::ImageRgba8(other.to_rgba8()),
        other => DynamicImage::ImageRgb8(other.to_rgb8()),
    };
    let channels = usize::from(image.color().channel_count());
    let mut shrink = Shrink::new((w, h), target, channels);
    for row in image.as_bytes().chunks_exact(w as usize * channels) {
        shrink.push(row);
    }
    drop(image);
    Ok(turn(shrink.finish(), orientation))
}

/// One cell of an image drawn in half-blocks, two pixels a cell: `▀` with the top pixel in the
/// foreground and the bottom one in the background, `▄` when only the bottom one shows, a blank
/// when neither does. A pixel under half alpha doesn't show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HalfBlock {
    pub top: Option<Rgb>,
    pub bottom: Option<Rgb>,
}

/// A pixel's color.
pub type Rgb = (u8, u8, u8);

impl HalfBlock {
    /// Cell `(x, y)` of `image` drawn in `cols` × `rows` cells. The image is no larger than its own
    /// pixels, so a small one repeats them.
    pub fn of(image: &DynamicImage, (cols, rows): (u16, u16), (x, y): (u32, u32)) -> HalfBlock {
        let (w, h) = (image.width(), image.height());
        let channels = usize::from(image.color().channel_count());
        let alpha = image.color().has_alpha();
        let at = |x: u32, y: u32| {
            let (x, y) = (x * w / u32::from(cols), y * h / (2 * u32::from(rows)));
            let p = &image.as_bytes()[(y * w + x) as usize * channels..];
            let a = if alpha { p[3] } else { 255 };
            (a >= 128).then_some((p[0], p[1], p[2]))
        };
        HalfBlock {
            top: at(x, 2 * y),
            bottom: at(x, 2 * y + 1),
        }
    }

    /// The cell's symbol, foreground and background.
    pub fn cell(self) -> (&'static str, Option<Rgb>, Option<Rgb>) {
        match (self.top, self.bottom) {
            (None, None) => (" ", None, None),
            (Some(t), b) => ("▀", Some(t), b),
            (None, b) => ("▄", b, None),
        }
    }
}

/// Whether `path` is a PNG that shows as it's stored, with no EXIF turn: a terminal that decodes
/// PNG can be sent the file itself. Only the header is read.
pub fn is_plain_png(path: &Path) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    let Ok(reader) = png::Decoder::new(BufReader::new(file)).read_info() else {
        return false;
    };
    let orientation = reader
        .info()
        .exif_metadata
        .as_deref()
        .and_then(Orientation::from_exif_chunk);
    matches!(orientation, None | Some(Orientation::NoTransforms))
}

/// The most bytes an image held whole may take decoded (a JPEG, an interlaced PNG), and the most a
/// PNG's row of column sums may take, as image's default limit would allow any image.
const MAX_WHOLE: usize = 512 << 20;

/// A PNG, a row at a time: only the rows being summed are held, never the whole image.
fn png(path: &Path, size: impl FnOnce((u32, u32)) -> (u32, u32)) -> Result<DynamicImage, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let info = reader.info();
    let (w, h, interlaced) = (info.width, info.height, info.interlaced);
    let orientation = info
        .exif_metadata
        .as_deref()
        .and_then(Orientation::from_exif_chunk)
        .unwrap_or(Orientation::NoTransforms);
    // turning a size is its own inverse: the upright target, turned, is the stored one
    let target = upright(size(upright((w, h), orientation)), orientation);
    let channels = match reader.output_color_type().0 {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err("an indexed PNG that didn't expand".into()),
    };
    // the column sums take two bytes a sample of a row: a header can claim a row of 2³¹ pixels
    if w as usize * channels * 2 > MAX_WHOLE {
        return Err(format!("the image is too wide to decode: {w} × {h}"));
    }
    let mut shrink = Shrink::new((w, h), target, channels);
    if interlaced {
        // Adam7 delivers its rows in seven passes: the image has to be whole before it's shrunk.
        let size = reader
            .output_buffer_size()
            .filter(|&n| n <= MAX_WHOLE)
            .ok_or("the interlaced PNG is too large")?;
        let mut buf = vec![0; size];
        let frame = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
        for row in buf.chunks_exact(frame.line_size).take(h as usize) {
            shrink.push(row);
        }
    } else {
        while let Some(row) = reader.next_row().map_err(|e| e.to_string())? {
            shrink.push(row.data());
        }
    }
    if shrink.rows < h {
        return Err("the PNG ends early".into());
    }
    Ok(turn(shrink.finish(), orientation))
}

fn turned(orientation: Orientation) -> bool {
    matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    )
}

/// The size of an image stored at `px`, once `orientation` is applied.
fn upright(px: (u32, u32), orientation: Orientation) -> (u32, u32) {
    if turned(orientation) {
        (px.1, px.0)
    } else {
        px
    }
}

fn turn(mut image: DynamicImage, orientation: Orientation) -> DynamicImage {
    image.apply_orientation(orientation);
    image
}

/// The most rows `Shrink::columns` takes before it's folded: 257 × 255 is `u16::MAX`.
const SPILL: u32 = 257;

/// A box filter fed one row at a time: every source pixel adds to the one output pixel it falls
/// in, and each output pixel is the mean of its box. For shrinking, it's as good as a resampling
/// filter, and it needs only one output row of sums.
struct Shrink {
    /// Source width and height.
    src: (u32, u32),
    /// Output width and height, no larger than the source's.
    dst: (u32, u32),
    /// Source channels: gray, gray and alpha, RGB or RGBA.
    channels: usize,
    /// Where each output column starts, in source columns, and one past the last.
    cols: Vec<u32>,
    /// The source rows not yet folded into `sums`, added up byte by byte: a flat add the
    /// compiler vectorizes, where summing each box a row at a time went a pixel at a time.
    columns: Vec<u16>,
    /// Source rows in `columns`, at most `SPILL`.
    pending: u32,
    /// One output row's sums, by output column and source channel.
    sums: Vec<u64>,
    /// Source rows taken.
    rows: u32,
    /// The output row being summed.
    y: u32,
    out: Vec<u8>,
}

impl Shrink {
    fn new(src: (u32, u32), dst: (u32, u32), channels: usize) -> Shrink {
        let dst = (dst.0.clamp(1, src.0.max(1)), dst.1.clamp(1, src.1.max(1)));
        let cols = (0..=dst.0)
            .map(|x| (u64::from(x) * u64::from(src.0) / u64::from(dst.0)) as u32)
            .collect();
        let out_channels = if channels.is_multiple_of(2) { 4 } else { 3 };
        Shrink {
            src,
            dst,
            channels,
            cols,
            columns: vec![0; src.0 as usize * channels],
            pending: 0,
            sums: vec![0; dst.0 as usize * channels],
            rows: 0,
            y: 0,
            out: Vec::with_capacity(dst.0 as usize * dst.1 as usize * out_channels),
        }
    }

    /// The first source row of output row `y`.
    fn row_start(&self, y: u32) -> u32 {
        (u64::from(y) * u64::from(self.src.1) / u64::from(self.dst.1)) as u32
    }

    /// Add the next source row.
    fn push(&mut self, row: &[u8]) {
        if self.rows >= self.src.1 {
            return;
        }
        // the first row is copied rather than added, so `columns` needs no clearing
        if self.pending == 0 {
            for (s, &v) in self.columns.iter_mut().zip(row) {
                *s = u16::from(v);
            }
            if let Some(rest) = self.columns.get_mut(row.len()..) {
                rest.fill(0);
            }
        } else {
            for (s, &v) in self.columns.iter_mut().zip(row) {
                *s += u16::from(v);
            }
        }
        self.pending += 1;
        self.rows += 1;
        if self.rows == self.row_start(self.y + 1) {
            self.flush();
        } else if self.pending == SPILL {
            self.fold();
        }
    }

    /// Add `columns` to the sums, box by box, and empty it.
    fn fold(&mut self) {
        match self.channels {
            1 => self.fold_by::<1>(),
            2 => self.fold_by::<2>(),
            3 => self.fold_by::<3>(),
            _ => self.fold_by::<4>(),
        }
        self.pending = 0;
    }

    /// `fold` for `CH` channels, a number the compiler can unroll a pixel by.
    fn fold_by<const CH: usize>(&mut self) {
        let columns = self.columns.as_chunks::<CH>().0;
        let sums = self.sums.as_chunks_mut::<CH>().0;
        for (sums, cols) in sums.iter_mut().zip(self.cols.windows(2)) {
            let mut acc = [0u64; CH];
            for px in &columns[cols[0] as usize..cols[1] as usize] {
                for (a, &v) in acc.iter_mut().zip(px) {
                    *a += u64::from(v);
                }
            }
            for (s, a) in sums.iter_mut().zip(acc) {
                *s += a;
            }
        }
    }

    /// Write the output row summed so far, and start the next.
    fn flush(&mut self) {
        if self.pending > 0 {
            self.fold();
        }
        match self.channels {
            1 => self.write::<1, 3>(),
            2 => self.write::<2, 4>(),
            3 => self.write::<3, 3>(),
            _ => self.write::<4, 4>(),
        }
        self.sums.fill(0);
        self.y += 1;
    }

    /// `flush` from `CH` source channels to `OUT` output ones: gray is spread over RGB.
    fn write<const CH: usize, const OUT: usize>(&mut self) {
        let rows = u64::from(self.rows - self.row_start(self.y));
        // a box is as wide as the source over the output, rounded down or up
        let narrow = self.src.0 / self.dst.0;
        let means = [narrow, narrow + 1].map(|w| Mean::new(rows * u64::from(w)));
        let start = self.out.len();
        self.out.resize(start + self.dst.0 as usize * OUT, 0);
        let out = self.out[start..].as_chunks_mut::<OUT>().0;
        let sums = self.sums.as_chunks::<CH>().0;
        for ((px, sums), cols) in out.iter_mut().zip(sums).zip(self.cols.windows(2)) {
            let mean = means[(cols[1] - cols[0] - narrow) as usize];
            for (i, v) in px.iter_mut().enumerate() {
                // gray and alpha: the gray three times, then the alpha
                let from = if CH > 2 { i } else { usize::from(i == 3) };
                *v = mean.of(sums[from]);
            }
        }
    }

    fn finish(mut self) -> DynamicImage {
        // a short source leaves the rest of the output to its last rows' sums, then black
        while self.y < self.dst.1 && self.rows > self.row_start(self.y) {
            self.flush();
        }
        let (w, h) = self.dst;
        let alpha = self.channels.is_multiple_of(2);
        self.out
            .resize(w as usize * h as usize * if alpha { 4 } else { 3 }, 0);
        if alpha {
            DynamicImage::ImageRgba8(RgbaImage::from_raw(w, h, self.out).expect("sized"))
        } else {
            DynamicImage::ImageRgb8(RgbImage::from_raw(w, h, self.out).expect("sized"))
        }
    }
}

/// The rounded mean of `n` bytes from their sum, by a multiply rather than a divide: `flush`
/// takes one for every output channel.
#[derive(Clone, Copy)]
struct Mean {
    n: u64,
    /// ⌈2⁴⁰ / n⌉, or 0 when `n` is too large for it to be exact.
    m: u64,
}

impl Mean {
    fn new(n: u64) -> Mean {
        let n = n.max(1);
        // t = s + n/2 is below 256n, so t·n < 2⁴⁰ while n ≤ 2¹⁶: then t·m >> 40 is ⌊t / n⌋
        let m = if n <= 1 << 16 {
            (1u64 << 40).div_ceil(n)
        } else {
            0
        };
        Mean { n, m }
    }

    fn of(self, sum: u64) -> u8 {
        let t = sum + self.n / 2;
        (if self.m == 0 {
            t / self.n
        } else {
            (t * self.m) >> 40
        }) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitting_only_shrinks() {
        assert_eq!(fitted((400, 300), (1000, 1000)), (400, 300));
        assert_eq!(fitted((4000, 3000), (1000, 1000)), (1000, 750));
        assert_eq!(fitted((3000, 4000), (1000, 1000)), (750, 1000));
        assert_eq!(fitted((5000, 1), (100, 100)), (100, 1));
        assert_eq!(fitted((0, 0), (10, 10)), (1, 1));
    }

    fn shrink(src: (u32, u32), dst: (u32, u32), channels: usize, px: &[u8]) -> Vec<u8> {
        let mut s = Shrink::new(src, dst, channels);
        for row in px.chunks_exact(src.0 as usize * channels) {
            s.push(row);
        }
        s.finish().into_bytes()
    }

    #[test]
    fn each_output_pixel_is_its_boxs_mean() {
        // 4×2 gray, halved both ways: two boxes of four
        let gray = [0, 10, 100, 100, 20, 30, 200, 201];
        assert_eq!(
            shrink((4, 2), (2, 1), 1, &gray),
            [15, 15, 15, 150, 150, 150]
        );
        // RGBA keeps its alpha
        let rgba = [10, 20, 30, 0, 30, 40, 50, 255];
        assert_eq!(shrink((2, 1), (1, 1), 4, &rgba), [20, 30, 40, 128]);
        // the same size is the same image, gray turned RGB
        assert_eq!(shrink((2, 1), (2, 1), 1, &[7, 9]), [7, 7, 7, 9, 9, 9]);
        // uneven boxes: 3 columns into 2 take one and two
        assert_eq!(shrink((3, 1), (2, 1), 3, &[9; 9]), [9; 6]);
        // never larger than the source
        assert_eq!(shrink((1, 1), (4, 4), 3, &[1, 2, 3]), [1, 2, 3]);
    }

    #[test]
    fn shrinking_is_a_naive_box_mean() {
        // uneven boxes both ways, and boxes over SPILL rows tall, at every channel count
        let sizes = [
            ((7, 5), (3, 2)),
            ((13, 600), (4, 2)),
            ((5, 1000), (2, 3)),
            ((9, 9), (9, 9)),
        ];
        let mut seed = 0x2545_f491_u32;
        for ((src, dst), channels) in sizes.into_iter().flat_map(|s| (1..=4).map(move |c| (s, c))) {
            let px: Vec<u8> = (0..src.0 * src.1 * channels as u32)
                .map(|_| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (seed >> 24) as u8
                })
                .collect();
            let at =
                |x: u32, y: u32, c: usize| u64::from(px[(y * src.0 + x) as usize * channels + c]);
            let mut want = vec![];
            for y in 0..dst.1 {
                let ys = y * src.1 / dst.1..(y + 1) * src.1 / dst.1;
                for x in 0..dst.0 {
                    let xs = x * src.0 / dst.0..(x + 1) * src.0 / dst.0;
                    let n = u64::from(xs.len() as u32 * ys.len() as u32);
                    let mean = |c| {
                        let s: u64 = ys
                            .clone()
                            .flat_map(|y| xs.clone().map(move |x| (x, y)))
                            .map(|(x, y)| at(x, y, c))
                            .sum();
                        ((s + n / 2) / n) as u8
                    };
                    match channels {
                        1 => want.extend([mean(0); 3]),
                        2 => want.extend([mean(0), mean(0), mean(0), mean(1)]),
                        _ => want.extend((0..channels).map(mean)),
                    }
                }
            }
            assert_eq!(
                shrink(src, dst, channels, &px),
                want,
                "{src:?} → {dst:?}, {channels} channels"
            );
        }
    }

    #[test]
    fn a_mean_rounds_as_dividing_would() {
        // where the rounding turns, for every mean, and either side of the multiply's limit
        for n in (1..=2048).chain(65_000..=66_000) {
            let mean = Mean::new(n);
            for q in 0..=255 {
                for s in [q * n, q * n + n / 2, (q + 1) * n - 1] {
                    let s = s.saturating_sub(n / 2);
                    for s in s.saturating_sub(1)..=(s + 1).min(255 * n) {
                        assert_eq!(u64::from(mean.of(s)), (s + n / 2) / n, "{s} / {n}");
                    }
                }
            }
        }
    }

    #[test]
    fn malformed_files_are_errors_not_panics_or_huge_allocations() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/images");
        // 384 bytes of JPEG whose header claims 65,498 × 65,256 pixels: 12.8 GB held whole (round 7)
        let err = decode(&dir.join("huge-sof.jpg"), (100, 100)).unwrap_err();
        assert!(err.contains("too large"), "{err}");
        // zune-jpeg 0.5.15's IDCT and png 0.18.1's palette expansion panicked on these (round 7);
        // the PNG's CRCs are recomputed, or png refuses it at IHDR in a build with checksums on (F-10)
        for file in ["zune-idct-panic.jpg", "png-palette-panic.png"] {
            let err = decode(&dir.join(file), (100, 100)).unwrap_err();
            assert_eq!(err, "the image failed to decode", "{file}");
        }
    }

    #[test]
    fn decodes_png_and_jpeg_shrunk_and_upright() {
        let dir = std::env::temp_dir().join(format!("ada-pixels-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = RgbImage::from_fn(40, 20, |x, _| {
            image::Rgb([if x < 20 { 0 } else { 250 }, 0, 0])
        });
        let png = dir.join("a.png");
        img.save(&png).unwrap();
        let out = decode(&png, (10, 10)).unwrap();
        assert_eq!((out.width(), out.height()), (10, 5));
        let rgb = out.to_rgb8();
        assert_eq!((rgb.get_pixel(0, 0)[0], rgb.get_pixel(9, 4)[0]), (0, 250));
        // a transparent PNG keeps its alpha
        let rgba = RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 4]));
        rgba.save(dir.join("b.png")).unwrap();
        let out = decode(&dir.join("b.png"), (4, 4)).unwrap();
        assert_eq!(out.as_bytes()[..4], [1, 2, 3, 4]);
        let jpg = dir.join("c.jpg");
        img.save(&jpg).unwrap();
        let out = decode(&jpg, (20, 20)).unwrap();
        assert_eq!(
            (out.width(), out.height(), out.color().has_alpha()),
            (20, 10, false)
        );
        assert!(decode(&dir.join("gone.png"), (1, 1)).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
