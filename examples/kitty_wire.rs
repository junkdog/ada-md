//! IMG-002: what each kitty wire format costs a TUI image, end to end. For every image, at the
//! size a block image takes in a window of `--cells` (100×50 by default), each format's encode as
//! the worker runs it (the transmission, base64 and chunks included: median wall and CPU of
//! `--runs`), its bytes, and, inside kitty (`--kitty`), the time from the first byte written to
//! kitty's OK, which takes in the write and kitty's decode, with kitty's own CPU (`KITTY_PID`'s
//! `/proc` ticks over the rounds). The formats go round-robin, `--rounds` times. Outside kitty
//! the font is `--font WxH` (9x20 by default). The results go to stderr.
//!
//! ```zsh
//! target/release/examples/kitty_wire bench/b2/root/corpus/img/photo.jpg
//! kitty sh -c 'target/release/examples/kitty_wire --kitty bench/b2/root/corpus/img/*.{jpg,png} 2> out.txt'
//! ```

use std::env;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use ada::geom::Size;
use ada::images::{self, ImageMode, Probe};
use ada::kitty;
use image::DynamicImage;

mod common;

#[derive(Clone, Copy, Debug)]
enum Wire {
    /// PNG, fdeflate behind one filter for every row.
    Png(png::Filter),
    /// PNG, deflate's stored blocks.
    Stored,
    /// Raw pixels, `f=24` or `f=32`.
    Raw,
    /// Raw pixels in a POSIX shared memory object (`t=s`, Linux's /dev/shm here); kitty reads
    /// and unlinks it.
    Shm,
}

const WIRES: [Wire; 8] = [
    Wire::Png(png::Filter::Paeth),
    Wire::Png(png::Filter::Sub),
    Wire::Png(png::Filter::Up),
    Wire::Png(png::Filter::Avg),
    Wire::Png(png::Filter::NoFilter),
    Wire::Stored,
    Wire::Raw,
    Wire::Shm,
];

fn name(wire: Wire) -> String {
    match wire {
        Wire::Png(f) => format!("png {f:?}"),
        Wire::Stored => "png stored".into(),
        Wire::Raw => "raw".into(),
        Wire::Shm => "shm".into(),
    }
}

fn main() {
    let mut paths = Vec::new();
    let (mut cells, mut runs, mut rounds) = (Size::new(100, 50), 15, 20);
    let (mut font, mut in_kitty) = (Size::new(9, 20), false);
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--cells" => cells = common::cells_arg(args.next()),
            "--font" => font = common::cells_arg(args.next()),
            "--runs" => runs = args.next().and_then(|n| n.parse().ok()).expect("--runs N"),
            "--rounds" => {
                rounds = args
                    .next()
                    .and_then(|n| n.parse().ok())
                    .expect("--rounds N");
            }
            "--kitty" => in_kitty = true,
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    if in_kitty {
        let _ = crossterm::terminal::enable_raw_mode();
        font = images::detect(ImageMode::Auto).expect("images on").cell;
        let _ = crossterm::terminal::disable_raw_mode();
    }
    eprintln!(
        "font {}x{} px, window {}x{} cells",
        font.width, font.height, cells.width, cells.height
    );
    for path in &paths {
        let Probe::Image { w, h, .. } = images::probe(path) else {
            eprintln!("{}: not an image", path.display());
            continue;
        };
        let size = images::fit((w, h), font, cells);
        let (cols, rows) = (
            size.width.min(kitty::MAX_CELLS),
            size.height.min(kitty::MAX_CELLS),
        );
        let max = (
            u32::from(cols) * u32::from(font.width),
            u32::from(rows) * u32::from(font.height),
        );
        let (decode, pixels) = time(runs, || kitty::pixels(path, (w, h), max).unwrap());
        let Some(pixels) = pixels else {
            eprintln!("{}: sent as the file", path.display());
            continue;
        };
        eprintln!(
            "{} {w}x{h} -> {cols}x{rows} cells, {}x{} px {:?}: decode {:.2} ms (CPU {:.2})",
            file(path),
            pixels.width(),
            pixels.height(),
            pixels.color(),
            decode.0,
            decode.1,
        );
        for wire in WIRES {
            let (enc, seq) = time(runs, || transmit(&pixels, wire, 1, cols, rows, true));
            let ((raw, _), bytes) = time(runs, || payload(&pixels, wire).len());
            eprintln!(
                "  {:<12} encode {:6.2} ms (CPU {:6.2}; payload alone {:6.2}): {:>9} bytes on the wire, payload {bytes:>9}",
                name(wire),
                enc.0,
                enc.1,
                raw,
                seq.len(),
            );
        }
        if in_kitty {
            kitty_rounds(&pixels, cols, rows, rounds);
        }
    }
}

/// Each format sent to kitty `rounds` times, round-robin: the median from the first byte to
/// kitty's OK, and kitty's CPU a send.
fn kitty_rounds(pixels: &DynamicImage, cols: u16, rows: u16, rounds: usize) {
    let kitty_pid = env::var("KITTY_PID").ok();
    let mut out = io::stdout().lock();
    let _ = crossterm::terminal::enable_raw_mode();
    let mut ms = vec![Vec::new(); WIRES.len()];
    let mut ticks = vec![0u64; WIRES.len()];
    let mut id = 0xada100;
    for _ in 0..rounds {
        for (i, &wire) in WIRES.iter().enumerate() {
            id += 1;
            let seq = transmit(pixels, wire, id, cols, rows, false);
            let t0 = kitty_pid.as_deref().map_or(0, cpu_ticks);
            let t = Instant::now();
            out.write_all(&seq).unwrap();
            out.flush().unwrap();
            let reply = read_reply();
            ms[i].push(t.elapsed().as_secs_f64() * 1e3);
            ticks[i] += kitty_pid.as_deref().map_or(0, cpu_ticks).saturating_sub(t0);
            assert!(reply.contains(";OK"), "{}: {reply:?}", name(wire));
            write!(out, "\x1b_Ga=d,d=I,i={id},q=2\x1b\\").unwrap();
            out.flush().unwrap();
        }
    }
    let _ = crossterm::terminal::disable_raw_mode();
    let hz = 100.0; // USER_HZ
    for (i, &wire) in WIRES.iter().enumerate() {
        let v = &mut ms[i];
        v.sort_by(f64::total_cmp);
        eprintln!(
            "  {:<12} kitty: write to OK median {:6.2} ms (min {:6.2}, max {:6.2}); kitty CPU {:6.2} ms a send",
            name(wire),
            v[v.len() / 2],
            v[0],
            v[v.len() - 1],
            ticks[i] as f64 / hz * 1e3 / rounds as f64,
        );
    }
}

/// kitty's reply, `ESC _ G … ESC \`.
fn read_reply() -> String {
    let mut stdin = io::stdin().lock();
    let mut got = Vec::new();
    let mut b = [0u8; 1];
    while stdin.read_exact(&mut b).is_ok() {
        got.push(b[0]);
        if got.ends_with(b"\x1b\\") && got.windows(3).any(|w| w == b"\x1b_G") {
            break;
        }
    }
    String::from_utf8_lossy(&got).into_owned()
}

fn cpu_ticks(pid: &str) -> u64 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    let rest = stat.rsplit_once(')').map_or("", |(_, r)| r);
    let f: Vec<&str> = rest.split_whitespace().collect();
    // fields 14 and 15 (utime, stime), 1-based, counted after the comm
    f.get(11).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0)
        + f.get(12).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0)
}

fn file(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

fn time<T>(runs: usize, mut f: impl FnMut() -> T) -> ((f64, f64), T) {
    let (mut ms, mut cpu) = (Vec::new(), Vec::new());
    let mut last = None;
    for _ in 0..runs {
        let (t, c) = (Instant::now(), common::cpu());
        last = Some(std::hint::black_box(f()));
        ms.push(t.elapsed().as_secs_f64() * 1e3);
        cpu.push(common::ms(common::cpu().saturating_sub(c)));
    }
    ms.sort_by(f64::total_cmp);
    cpu.sort_by(f64::total_cmp);
    ((ms[runs / 2], cpu[runs / 2]), last.unwrap())
}

/// The payload alone: the PNG, or the pixels.
fn payload(image: &DynamicImage, wire: Wire) -> Vec<u8> {
    let mut v = Vec::new();
    match wire {
        Wire::Raw | Wire::Shm => v.extend_from_slice(image.as_bytes()),
        Wire::Png(filter) => png(image, filter, png::Compression::Fast, &mut v),
        Wire::Stored => png(
            image,
            png::Filter::NoFilter,
            png::Compression::NoCompression,
            &mut v,
        ),
    }
    v
}

/// The transmission, as `kitty::transmit` writes it; `quiet` asks for no reply (`q=2`).
fn transmit(
    image: &DynamicImage,
    wire: Wire,
    id: u32,
    cols: u16,
    rows: u16,
    quiet: bool,
) -> Vec<u8> {
    let q = if quiet { "q=2," } else { "" };
    let format = match wire {
        Wire::Raw | Wire::Shm => format!(
            "f={},s={},v={}{}",
            if image.color().has_alpha() { 32 } else { 24 },
            image.width(),
            image.height(),
            if matches!(wire, Wire::Shm) {
                ",t=s"
            } else {
                ""
            },
        ),
        _ => "f=100".into(),
    };
    let mut seq = Vec::new();
    let mut chunks = Chunks::new(
        &mut seq,
        format!("a=T,U=1,{q}i={id},c={cols},r={rows},{format}"),
        q,
    );
    match wire {
        Wire::Raw => chunks.write_all(image.as_bytes()).unwrap(),
        Wire::Shm => {
            let name = format!("ada-img002-{}-{id}", std::process::id());
            let path = format!("/dev/shm/{name}");
            std::fs::write(&path, image.as_bytes()).unwrap();
            if quiet {
                // nothing reads it
                std::fs::remove_file(&path).unwrap();
            }
            chunks.write_all(format!("/{name}").as_bytes()).unwrap();
        }
        Wire::Png(filter) => png(image, filter, png::Compression::Fast, &mut chunks),
        Wire::Stored => png(
            image,
            png::Filter::NoFilter,
            png::Compression::NoCompression,
            &mut chunks,
        ),
    }
    chunks.finish().unwrap();
    seq
}

/// `kitty::png` with the filter and level given.
fn png(image: &DynamicImage, filter: png::Filter, level: png::Compression, w: impl Write) {
    let color = if image.color().has_alpha() {
        png::ColorType::Rgba
    } else {
        png::ColorType::Rgb
    };
    let mut encoder = png::Encoder::new(w, image.width(), image.height());
    encoder.set_color(color);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(level);
    encoder.set_filter(filter);
    let mut png = encoder.write_header().unwrap();
    let mut stream = png.stream_writer_with_size(64 << 10).unwrap();
    stream.write_all(image.as_bytes()).unwrap();
    stream.finish().unwrap();
    png.finish().unwrap();
}

/// kitty.rs's chunks, copied: 4096 base64 characters a chunk.
const CHUNK: usize = 4096 / 4 * 3;

struct Chunks<'w, W: Write> {
    w: &'w mut W,
    header: Option<String>,
    q: &'static str,
    buf: Vec<u8>,
    b64: Vec<u8>,
}

impl<'w, W: Write> Chunks<'w, W> {
    fn new(w: &'w mut W, header: String, q: &'static str) -> Self {
        Chunks {
            w,
            header: Some(header),
            q,
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
            self.b64.extend_from_slice(self.q.as_bytes());
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

fn base64(data: &[u8], out: &mut Vec<u8>) {
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
