//! The kitty graphics protocol's wire format, for printed images (architecture §12) and the TUI's
//! ([`crate::images`]): an image sent as a virtual placement, then shown by unicode placeholder
//! cells. Placeholders are text, so a pager or an fzf preview pane clips, scrolls and clears them
//! with the rest of the output, and the TUI's frames move them with their rows.
//!
//! <https://sw.kovidgoyal.net/kitty/graphics-protocol/#unicode-placeholders>

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

use image::DynamicImage;

use crate::pixels;

/// The character kitty reserves for image placeholders.
pub const PLACEHOLDER: char = '\u{10EEEE}';

/// Raw bytes in one chunk: 4096 base64 characters, the most the protocol allows.
const CHUNK: usize = 4096 / 4 * 3;

/// What a transmission carries.
pub enum Payload<'a> {
    /// A PNG file, sent as it is: kitty decodes it.
    Png(&'a Path),
    /// Pixels, RGB or RGBA, encoded as a PNG.
    Pixels(&'a DynamicImage),
}

/// The id of the `n`th image printed. Below 2²⁴, so the foreground color carries all of it.
///
/// With `reuse`, it's the same for every run: an fzf preview pane is redrawn for each file, so
/// each preview's images take the ids of the one before, and kitty replaces their pixels instead
/// of holding every image previewed until its storage quota is full. Otherwise it differs from
/// run to run, from the pid and the time, so that output kept in the scrollback keeps its images.
pub fn image_id(n: u32, reuse: bool) -> u32 {
    if reuse {
        return 0xada000 | (n & 0xfff);
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let mut x = (u64::from(std::process::id()) << 32 | u64::from(nanos)) ^ u64::from(n);
    // splitmix64's finalizer
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    (x as u32 & 0xff_ffff).max(1)
}

/// The pixels to send for the image at `path`, `px` pixels upright, shown in `max` pixels, or
/// `None` when the file itself can go: a PNG small enough already, which kitty decodes.
pub fn pixels(
    path: &Path,
    px: (u32, u32),
    max: (u32, u32),
) -> Result<Option<DynamicImage>, String> {
    let fits = px.0 <= max.0 && px.1 <= max.1;
    if fits && pixels::is_plain_png(path) {
        return Ok(None);
    }
    pixels::decode(path, max).map(Some)
}

/// Send image `id` as a virtual placement of `cols` × `rows` cells, which kitty fits the image
/// into. Nothing is asked of the terminal and every reply is suppressed (`q=2`): under fzf, a
/// reply would arrive as keystrokes.
pub fn transmit(
    w: &mut impl Write,
    id: u32,
    payload: &Payload,
    cols: u16,
    rows: u16,
) -> io::Result<()> {
    let place = format!("a=T,U=1,q=2,i={id},c={cols},r={rows}");
    match payload {
        Payload::Png(path) => {
            let mut chunks = Chunks::new(w, format!("{place},f=100"));
            io::copy(&mut File::open(path)?, &mut chunks)?;
            chunks.finish()
        }
        Payload::Pixels(image) => {
            let mut chunks = Chunks::new(w, format!("{place},f=100"));
            png(image, &mut chunks)?;
            chunks.finish()
        }
    }
}

/// Send image `id`, `image`'s pixels, through a POSIX shared memory object (`t=s`), which the
/// terminal reads and unlinks: the escape carries only the object's name. The TUI's way when the
/// probe found the terminal can read one (it shares the machine). Each call makes a new object,
/// since the terminal unlinks each it reads. On a photo in kitty, 3.4 ms from the write to the
/// image stored, against 21.5 for the PNG and 0.25 ms against 4.0 to make it (IMG-002).
#[cfg(target_os = "linux")]
pub fn transmit_shm(
    out: &mut String,
    id: u32,
    image: &DynamicImage,
    cols: u16,
    rows: u16,
) -> io::Result<()> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static OBJECTS: AtomicU32 = AtomicU32::new(0);
    let name = format!(
        "/ada-{}-{}",
        std::process::id(),
        OBJECTS.fetch_add(1, Ordering::Relaxed)
    );
    shm_object(&name, image.as_bytes())?;
    let f = if image.color().has_alpha() { 32 } else { 24 };
    let (w, h) = (image.width(), image.height());
    let mut b64 = Vec::with_capacity(name.len() / 3 * 4 + 4);
    base64(name.as_bytes(), &mut b64);
    let b64 = String::from_utf8(b64).map_err(io::Error::other)?;
    out.push_str(&format!(
        "\x1b_Ga=T,U=1,q=2,i={id},c={cols},r={rows},f={f},s={w},v={h},t=s;{b64}\x1b\\"
    ));
    Ok(())
}

/// A shared memory object `name` holding `bytes`, readable by the user alone. One left from an
/// earlier process of the same pid is replaced.
#[cfg(target_os = "linux")]
fn shm_object(name: &str, bytes: &[u8]) -> io::Result<()> {
    use rustix::fs::Mode;
    use rustix::shm::{self, OFlags};
    let flags = OFlags::CREATE | OFlags::TRUNC | OFlags::RDWR;
    let fd = shm::open(name, flags, Mode::RUSR | Mode::WUSR)?;
    if let Err(e) = File::from(fd).write_all(bytes) {
        let _ = shm::unlink(name);
        return Err(e);
    }
    Ok(())
}

/// The shared memory query's image id: the probe's kitty query is 31.
pub const SHM_QUERY_ID: &str = "32";

/// Kitty's graphics query (id 32) with its 1×1 pixel in a shared memory object: an OK says the
/// terminal reads them, so it shares this machine. `None` where ada makes none, or when the object
/// can't be made. [`shm_query_done`] unlinks the object once the replies are in, since a terminal
/// that can't reach it leaves it.
pub fn shm_query() -> Option<Vec<u8>> {
    #[cfg(target_os = "linux")]
    {
        let name = shm_query_name();
        shm_object(&name, &[0; 3]).ok()?;
        let mut q = b"\x1b_Gi=32,s=1,v=1,a=q,t=s,f=24;".to_vec();
        base64(name.as_bytes(), &mut q);
        q.extend_from_slice(b"\x1b\\");
        Some(q)
    }
    #[cfg(not(target_os = "linux"))]
    None
}

/// Unlink [`shm_query`]'s object, if the terminal didn't.
pub fn shm_query_done() {
    #[cfg(target_os = "linux")]
    let _ = rustix::shm::unlink(shm_query_name());
}

#[cfg(target_os = "linux")]
fn shm_query_name() -> String {
    format!("/ada-{}-query", std::process::id())
}

/// `image`, RGB or RGBA, as a PNG streamed to `w`. png's fast level (fdeflate) behind the Paeth
/// filter: 3–4× faster than zlib's fast level on the pixels and, on photos, smaller (experiment
/// 24). An image whose every pixel is gray goes as gray: fdeflate, unlike zlib, doesn't fold
/// the gray's three copies back into one.
pub(crate) fn png(image: &DynamicImage, w: impl Write) -> io::Result<()> {
    use png::ColorType;
    let alpha = image.color().has_alpha();
    let ch = if alpha { 4 } else { 3 };
    let gray = image
        .as_bytes()
        .chunks_exact(ch)
        .all(|p| p[0] == p[1] && p[1] == p[2]);
    let color = match (gray, alpha) {
        (false, false) => ColorType::Rgb,
        (false, true) => ColorType::Rgba,
        (true, false) => ColorType::Grayscale,
        (true, true) => ColorType::GrayscaleAlpha,
    };
    let mut encoder = png::Encoder::new(w, image.width(), image.height());
    encoder.set_color(color);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    encoder.set_filter(png::Filter::Paeth);
    let mut png = encoder.write_header().map_err(io::Error::other)?;
    let mut stream = png
        .stream_writer_with_size(64 << 10)
        .map_err(io::Error::other)?;
    if gray {
        // a row at a time, so the gray copy is never whole
        let mut row = Vec::with_capacity(image.width() as usize * 2);
        for pixels in image.as_bytes().chunks(image.width() as usize * ch) {
            row.clear();
            for p in pixels.chunks_exact(ch) {
                row.push(p[0]);
                if alpha {
                    row.push(p[3]);
                }
            }
            stream.write_all(&row)?;
        }
    } else {
        stream.write_all(image.as_bytes())?;
    }
    stream.finish().map_err(io::Error::other)?;
    png.finish().map_err(io::Error::other)
}

/// Row `row` of image `id`, `cols` cells wide: each cell names its row and column, so the cells
/// need no neighbors to be placed (a pager may redraw any of them alone). The caller sets the
/// foreground to [`id_color`].
pub fn placeholders(row: u16, cols: u16, out: &mut String) {
    for col in 0..cols {
        out.extend(placeholder(row, col));
    }
}

/// The cell at row `row` and column `col` of an image: the placeholder, then the diacritics that
/// number them. Kitty can tell both from the cell to the left, when that's the same image's
/// previous column: the placeholder alone is that cell.
pub fn placeholder(row: u16, col: u16) -> [char; 3] {
    [PLACEHOLDER, diacritic(row), diacritic(col)]
}

/// The foreground color that names image `id` (below 2²⁴) in its placeholders.
pub fn id_color(id: u32) -> (u8, u8, u8) {
    let [_, r, g, b] = id.to_be_bytes();
    (r, g, b)
}

/// The most rows or columns placeholders can number.
pub const MAX_CELLS: u16 = DIACRITICS.len() as u16;

fn diacritic(n: u16) -> char {
    DIACRITICS[usize::from(n).min(DIACRITICS.len() - 1)]
}

/// An APC stream of base64 chunks: the first carries `header`, every one but the last says more
/// follow (`m=1`). A full chunk is held until the next byte arrives, so the last one is never
/// empty.
struct Chunks<'w, W: Write> {
    w: &'w mut W,
    header: Option<String>,
    buf: Vec<u8>,
    b64: Vec<u8>,
}

impl<'w, W: Write> Chunks<'w, W> {
    fn new(w: &'w mut W, header: String) -> Self {
        Chunks {
            w,
            header: Some(header),
            buf: Vec::with_capacity(CHUNK),
            b64: Vec::with_capacity(CHUNK / 3 * 4 + 64),
        }
    }

    fn send(&mut self, more: bool) -> io::Result<()> {
        self.b64.clear();
        self.b64.extend_from_slice(b"\x1b_G");
        if let Some(header) = self.header.take() {
            self.b64.extend_from_slice(header.as_bytes());
            self.b64.push(b',');
        } else {
            self.b64.extend_from_slice(b"q=2,");
        }
        self.b64
            .extend_from_slice(if more { b"m=1;" } else { b"m=0;" });
        base64(&self.buf, &mut self.b64);
        self.b64.extend_from_slice(b"\x1b\\");
        self.buf.clear();
        self.w.write_all(&self.b64)
    }

    fn finish(mut self) -> io::Result<()> {
        self.send(false)
    }
}

impl<W: Write> Write for Chunks<'_, W> {
    fn write(&mut self, mut data: &[u8]) -> io::Result<usize> {
        let n = data.len();
        while !data.is_empty() {
            if self.buf.len() == CHUNK {
                self.send(true)?;
            }
            let take = (CHUNK - self.buf.len()).min(data.len());
            self.buf.extend_from_slice(&data[..take]);
            data = &data[take..];
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn base64(data: &[u8], out: &mut Vec<u8>) {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let (triples, rest) = data.as_chunks::<3>();
    for &[a, b, c] in triples {
        let n = u32::from(a) << 16 | u32::from(b) << 8 | u32::from(c);
        out.extend([18, 12, 6, 0].map(|s| ABC[(n >> s & 63) as usize]));
    }
    match *rest {
        [a] => {
            let n = u32::from(a) << 16;
            out.extend([
                ABC[(n >> 18) as usize],
                ABC[(n >> 12 & 63) as usize],
                b'=',
                b'=',
            ]);
        }
        [a, b] => {
            let n = u32::from(a) << 16 | u32::from(b) << 8;
            out.extend([18, 12, 6].map(|s| ABC[(n >> s & 63) as usize]));
            out.push(b'=');
        }
        _ => {}
    }
}

/// Row and column numbers, from kitty's rowcolumn-diacritics.txt.
static DIACRITICS: [char; 297] = [
    '\u{305}',
    '\u{30D}',
    '\u{30E}',
    '\u{310}',
    '\u{312}',
    '\u{33D}',
    '\u{33E}',
    '\u{33F}',
    '\u{346}',
    '\u{34A}',
    '\u{34B}',
    '\u{34C}',
    '\u{350}',
    '\u{351}',
    '\u{352}',
    '\u{357}',
    '\u{35B}',
    '\u{363}',
    '\u{364}',
    '\u{365}',
    '\u{366}',
    '\u{367}',
    '\u{368}',
    '\u{369}',
    '\u{36A}',
    '\u{36B}',
    '\u{36C}',
    '\u{36D}',
    '\u{36E}',
    '\u{36F}',
    '\u{483}',
    '\u{484}',
    '\u{485}',
    '\u{486}',
    '\u{487}',
    '\u{592}',
    '\u{593}',
    '\u{594}',
    '\u{595}',
    '\u{597}',
    '\u{598}',
    '\u{599}',
    '\u{59C}',
    '\u{59D}',
    '\u{59E}',
    '\u{59F}',
    '\u{5A0}',
    '\u{5A1}',
    '\u{5A8}',
    '\u{5A9}',
    '\u{5AB}',
    '\u{5AC}',
    '\u{5AF}',
    '\u{5C4}',
    '\u{610}',
    '\u{611}',
    '\u{612}',
    '\u{613}',
    '\u{614}',
    '\u{615}',
    '\u{616}',
    '\u{617}',
    '\u{657}',
    '\u{658}',
    '\u{659}',
    '\u{65A}',
    '\u{65B}',
    '\u{65D}',
    '\u{65E}',
    '\u{6D6}',
    '\u{6D7}',
    '\u{6D8}',
    '\u{6D9}',
    '\u{6DA}',
    '\u{6DB}',
    '\u{6DC}',
    '\u{6DF}',
    '\u{6E0}',
    '\u{6E1}',
    '\u{6E2}',
    '\u{6E4}',
    '\u{6E7}',
    '\u{6E8}',
    '\u{6EB}',
    '\u{6EC}',
    '\u{730}',
    '\u{732}',
    '\u{733}',
    '\u{735}',
    '\u{736}',
    '\u{73A}',
    '\u{73D}',
    '\u{73F}',
    '\u{740}',
    '\u{741}',
    '\u{743}',
    '\u{745}',
    '\u{747}',
    '\u{749}',
    '\u{74A}',
    '\u{7EB}',
    '\u{7EC}',
    '\u{7ED}',
    '\u{7EE}',
    '\u{7EF}',
    '\u{7F0}',
    '\u{7F1}',
    '\u{7F3}',
    '\u{816}',
    '\u{817}',
    '\u{818}',
    '\u{819}',
    '\u{81B}',
    '\u{81C}',
    '\u{81D}',
    '\u{81E}',
    '\u{81F}',
    '\u{820}',
    '\u{821}',
    '\u{822}',
    '\u{823}',
    '\u{825}',
    '\u{826}',
    '\u{827}',
    '\u{829}',
    '\u{82A}',
    '\u{82B}',
    '\u{82C}',
    '\u{82D}',
    '\u{951}',
    '\u{953}',
    '\u{954}',
    '\u{F82}',
    '\u{F83}',
    '\u{F86}',
    '\u{F87}',
    '\u{135D}',
    '\u{135E}',
    '\u{135F}',
    '\u{17DD}',
    '\u{193A}',
    '\u{1A17}',
    '\u{1A75}',
    '\u{1A76}',
    '\u{1A77}',
    '\u{1A78}',
    '\u{1A79}',
    '\u{1A7A}',
    '\u{1A7B}',
    '\u{1A7C}',
    '\u{1B6B}',
    '\u{1B6D}',
    '\u{1B6E}',
    '\u{1B6F}',
    '\u{1B70}',
    '\u{1B71}',
    '\u{1B72}',
    '\u{1B73}',
    '\u{1CD0}',
    '\u{1CD1}',
    '\u{1CD2}',
    '\u{1CDA}',
    '\u{1CDB}',
    '\u{1CE0}',
    '\u{1DC0}',
    '\u{1DC1}',
    '\u{1DC3}',
    '\u{1DC4}',
    '\u{1DC5}',
    '\u{1DC6}',
    '\u{1DC7}',
    '\u{1DC8}',
    '\u{1DC9}',
    '\u{1DCB}',
    '\u{1DCC}',
    '\u{1DD1}',
    '\u{1DD2}',
    '\u{1DD3}',
    '\u{1DD4}',
    '\u{1DD5}',
    '\u{1DD6}',
    '\u{1DD7}',
    '\u{1DD8}',
    '\u{1DD9}',
    '\u{1DDA}',
    '\u{1DDB}',
    '\u{1DDC}',
    '\u{1DDD}',
    '\u{1DDE}',
    '\u{1DDF}',
    '\u{1DE0}',
    '\u{1DE1}',
    '\u{1DE2}',
    '\u{1DE3}',
    '\u{1DE4}',
    '\u{1DE5}',
    '\u{1DE6}',
    '\u{1DFE}',
    '\u{20D0}',
    '\u{20D1}',
    '\u{20D4}',
    '\u{20D5}',
    '\u{20D6}',
    '\u{20D7}',
    '\u{20DB}',
    '\u{20DC}',
    '\u{20E1}',
    '\u{20E7}',
    '\u{20E9}',
    '\u{20F0}',
    '\u{2CEF}',
    '\u{2CF0}',
    '\u{2CF1}',
    '\u{2DE0}',
    '\u{2DE1}',
    '\u{2DE2}',
    '\u{2DE3}',
    '\u{2DE4}',
    '\u{2DE5}',
    '\u{2DE6}',
    '\u{2DE7}',
    '\u{2DE8}',
    '\u{2DE9}',
    '\u{2DEA}',
    '\u{2DEB}',
    '\u{2DEC}',
    '\u{2DED}',
    '\u{2DEE}',
    '\u{2DEF}',
    '\u{2DF0}',
    '\u{2DF1}',
    '\u{2DF2}',
    '\u{2DF3}',
    '\u{2DF4}',
    '\u{2DF5}',
    '\u{2DF6}',
    '\u{2DF7}',
    '\u{2DF8}',
    '\u{2DF9}',
    '\u{2DFA}',
    '\u{2DFB}',
    '\u{2DFC}',
    '\u{2DFD}',
    '\u{2DFE}',
    '\u{2DFF}',
    '\u{A66F}',
    '\u{A67C}',
    '\u{A67D}',
    '\u{A6F0}',
    '\u{A6F1}',
    '\u{A8E0}',
    '\u{A8E1}',
    '\u{A8E2}',
    '\u{A8E3}',
    '\u{A8E4}',
    '\u{A8E5}',
    '\u{A8E6}',
    '\u{A8E7}',
    '\u{A8E8}',
    '\u{A8E9}',
    '\u{A8EA}',
    '\u{A8EB}',
    '\u{A8EC}',
    '\u{A8ED}',
    '\u{A8EE}',
    '\u{A8EF}',
    '\u{A8F0}',
    '\u{A8F1}',
    '\u{AAB0}',
    '\u{AAB2}',
    '\u{AAB3}',
    '\u{AAB7}',
    '\u{AAB8}',
    '\u{AABE}',
    '\u{AABF}',
    '\u{AAC1}',
    '\u{FE20}',
    '\u{FE21}',
    '\u{FE22}',
    '\u{FE23}',
    '\u{FE24}',
    '\u{FE25}',
    '\u{FE26}',
    '\u{10A0F}',
    '\u{10A38}',
    '\u{1D185}',
    '\u{1D186}',
    '\u{1D187}',
    '\u{1D188}',
    '\u{1D189}',
    '\u{1D1AA}',
    '\u{1D1AB}',
    '\u{1D1AC}',
    '\u{1D1AD}',
    '\u{1D242}',
    '\u{1D243}',
    '\u{1D244}',
];

#[cfg(test)]
mod tests {

    use super::*;

    /// The shared memory query names a 1×1 pixel's object, which `shm_query_done` unlinks.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_shared_memory_query() {
        let q = shm_query().unwrap();
        let q = std::str::from_utf8(&q).unwrap();
        let name = format!("/dev/shm{}", shm_query_name());
        let mut b64 = Vec::new();
        base64(shm_query_name().as_bytes(), &mut b64);
        let b64 = String::from_utf8(b64).unwrap();
        assert_eq!(q, format!("\x1b_Gi=32,s=1,v=1,a=q,t=s,f=24;{b64}\x1b\\"));
        assert_eq!(std::fs::read(&name).unwrap(), [0; 3]);
        shm_query_done();
        assert!(!Path::new(&name).exists());
        shm_query_done();
    }

    #[test]
    fn base64_matches_the_rfc() {
        for (raw, b64) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            let mut out = Vec::new();
            base64(raw.as_bytes(), &mut out);
            assert_eq!(out, b64.as_bytes());
        }
    }

    /// The commands of a transmission: their parameters and their payload.
    fn commands(stream: &[u8]) -> Vec<(String, String)> {
        let text = std::str::from_utf8(stream).unwrap();
        text.split("\x1b_G")
            .skip(1)
            .map(|c| {
                let c = c.strip_suffix("\x1b\\").expect("ends with ST");
                let (p, data) = c.split_once(';').unwrap();
                (p.to_owned(), data.to_owned())
            })
            .collect()
    }

    #[test]
    fn a_transmission_is_chunked_and_says_where_it_ends() {
        let image = DynamicImage::ImageRgb8(image::RgbImage::from_fn(300, 200, |x, y| {
            image::Rgb([(x * 31 % 256) as u8, (y * 17 % 256) as u8, (x ^ y) as u8])
        }));
        let mut out = Vec::new();
        transmit(&mut out, 42, &Payload::Pixels(&image), 30, 10).unwrap();
        let cmds = commands(&out);
        assert!(cmds.len() > 1, "{} commands", cmds.len());
        let first = &cmds[0].0;
        for p in ["a=T", "U=1", "q=2", "i=42", "c=30", "r=10", "f=100"] {
            assert!(first.split(',').any(|kv| kv == p), "{p} in {first}");
        }
        for (i, (p, data)) in cmds.iter().enumerate() {
            let last = i == cmds.len() - 1;
            assert!(p.ends_with(if last { "m=0" } else { "m=1" }), "{p}");
            assert!(i == 0 || p == "q=2,m=1" || p == "q=2,m=0", "{p}");
            assert!(!data.is_empty() && data.len() <= 4096);
        }
        // the payloads, joined and decoded, are a PNG of the pixels
        let b64: String = cmds.iter().map(|c| c.1.as_str()).collect();
        let png = image::load_from_memory_with_format(&decode64(&b64), image::ImageFormat::Png);
        assert_eq!(png.unwrap(), image);
    }

    #[test]
    fn gray_pixels_go_as_a_gray_png() {
        let sent = |image: &DynamicImage| {
            let mut out = Vec::new();
            transmit(&mut out, 1, &Payload::Pixels(image), 2, 1).unwrap();
            let b64: String = commands(&out).iter().map(|c| c.1.clone()).collect();
            image::load_from_memory_with_format(&decode64(&b64), image::ImageFormat::Png).unwrap()
        };
        let rgb = DynamicImage::ImageRgb8(image::RgbImage::from_fn(20, 10, |x, y| {
            let g = (x * 12 + y) as u8;
            image::Rgb([g, g, g])
        }));
        let png = sent(&rgb);
        assert_eq!(png.color(), image::ColorType::L8);
        assert_eq!(png.to_rgb8(), *rgb.as_rgb8().unwrap());
        let rgba = DynamicImage::ImageRgba8(image::RgbaImage::from_fn(20, 10, |x, y| {
            let g = (x * 12) as u8;
            image::Rgba([g, g, g, (y * 25) as u8])
        }));
        let png = sent(&rgba);
        assert_eq!(png.color(), image::ColorType::La8);
        assert_eq!(png.to_rgba8(), *rgba.as_rgba8().unwrap());
        // one pixel of color, and it's color
        let mut color = rgb.to_rgb8();
        color.put_pixel(19, 9, image::Rgb([1, 2, 3]));
        let color = DynamicImage::ImageRgb8(color);
        assert_eq!(sent(&color), color);
    }

    #[test]
    fn a_png_goes_as_it_is() {
        let dir = std::env::temp_dir().join(format!("ada-kitty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.png");
        image::RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 4]))
            .save(&path)
            .unwrap();
        let mut out = Vec::new();
        transmit(&mut out, 7, &Payload::Png(&path), 1, 1).unwrap();
        let cmds = commands(&out);
        assert_eq!(cmds.len(), 1);
        assert!(cmds[0].0.contains(",f=100,") && !cmds[0].0.contains("o=z"));
        assert_eq!(decode64(&cmds[0].1), std::fs::read(&path).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn placeholders_name_their_row_and_column() {
        let mut out = String::new();
        placeholders(2, 3, &mut out);
        let cells: Vec<Vec<char>> = out
            .split(PLACEHOLDER)
            .skip(1)
            .map(|c| c.chars().collect())
            .collect();
        assert_eq!(
            cells,
            [0, 1, 2].map(|col| vec![DIACRITICS[2], DIACRITICS[col]])
        );
        assert_eq!(id_color(0x12_3456), (0x12, 0x34, 0x56));
        assert!((1..1 << 24).contains(&image_id(3, false)));
        assert_eq!(image_id(3, true), image_id(3, true));
    }

    fn decode64(s: &str) -> Vec<u8> {
        let val = |c: u8| match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            _ => 63,
        };
        let mut out = Vec::new();
        for quad in s.as_bytes().chunks(4) {
            let pad = quad.iter().filter(|&&c| c == b'=').count();
            let n = quad.iter().fold(0u32, |n, &c| {
                n << 6 | if c == b'=' { 0 } else { u32::from(val(c)) }
            });
            out.extend(&n.to_be_bytes()[1..4 - pad]);
        }
        out
    }
}
