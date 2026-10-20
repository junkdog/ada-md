//! The image path (F-08): a file of any bytes decoded upright and box-filtered to the size it's
//! shown at (`pixels::decode_to`), then sent to kitty as a PNG of those pixels
//! (`kitty::transmit`). ada decodes whatever PNG or JPEG an fzf preview lands on, so:
//!   - nothing may panic, and a lying header (a huge IHDR or SOF, a truncated IDAT, an interlaced
//!     PNG near `MAX_WHOLE`) must cost bounded memory and time (`-rss_limit_mb`, `-timeout`)
//!   - the output is the size asked for, no larger than the image, turned by its EXIF orientation
//!   - for an 8-bit source, every output pixel is the rounded mean of its box over a whole decode
//!     by `image` (16-bit samples differ: `image` scales them, png's `normalize_to_color8` strips
//!     them), shrunk as stored and then turned, as ada does
//!   - kitty's payload, base64 and PNG decoded back, is the pixels
//!
//! The first three bytes pick the size: a box the image is fitted into (as print and the TUI ask),
//! or a size of its own, possibly past the image's; the rest is the file.
//!
//! ```zsh
//! cargo +nightly fuzz run -s none -a images -- -dict=fuzz/images.dict -max_len=65536
//! ```

#![no_main]

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::OnceLock;

use ada::kitty::{self, Payload};
use ada::pixels;
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use libfuzzer_sys::fuzz_target;

/// Where each input is written: `decode_to` reads a path. In `/dev/shm`, so it stays in memory.
fn scratch() -> &'static PathBuf {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let dir = if std::path::Path::new("/dev/shm").is_dir() {
            PathBuf::from("/dev/shm")
        } else {
            std::env::temp_dir()
        };
        dir.join(format!("ada-fuzz-images-{}", std::process::id()))
    })
}

fn upright(px: (u32, u32), o: Orientation) -> (u32, u32) {
    match o {
        Orientation::Rotate90
        | Orientation::Rotate270
        | Orientation::Rotate90FlipH
        | Orientation::Rotate270FlipH => (px.1, px.0),
        _ => px,
    }
}

/// `src` (`channels` of 1–4, 8-bit) shrunk to `dst` by the plain box mean `Shrink` computes: box
/// edges at `x * src / dst`, rounded halves up; gray spread over RGB, alpha kept.
fn naive(src: (u32, u32), dst: (u32, u32), channels: usize, px: &[u8]) -> Vec<u8> {
    let at = |x: u32, y: u32, c: usize| {
        u64::from(px[(y as usize * src.0 as usize + x as usize) * channels + c])
    };
    let mut out = Vec::new();
    for y in 0..dst.1 {
        let ys = (u64::from(y) * u64::from(src.1) / u64::from(dst.1)) as u32
            ..(u64::from(y + 1) * u64::from(src.1) / u64::from(dst.1)) as u32;
        for x in 0..dst.0 {
            let xs = (u64::from(x) * u64::from(src.0) / u64::from(dst.0)) as u32
                ..(u64::from(x + 1) * u64::from(src.0) / u64::from(dst.0)) as u32;
            let n = (xs.len() * ys.len()) as u64;
            let mean = |c| {
                let mut s = 0;
                for y in ys.clone() {
                    for x in xs.clone() {
                        s += at(x, y, c);
                    }
                }
                ((s + n / 2) / n) as u8
            };
            match channels {
                1 => out.extend([mean(0); 3]),
                2 => out.extend([mean(0), mean(0), mean(0), mean(1)]),
                _ => out.extend((0..channels).map(mean)),
            }
        }
    }
    out
}

/// What kitty would decode from a transmission: the base64 of every chunk, joined, as a PNG.
fn received(stream: &[u8]) -> DynamicImage {
    let text = std::str::from_utf8(stream).expect("kitty's commands are ASCII");
    let mut b64 = String::new();
    for cmd in text.split("\x1b_G").skip(1) {
        let body = cmd.strip_suffix("\x1b\\").expect("a command ends with ST");
        let (_, data) = body.split_once(';').expect("a chunk has a payload");
        b64.push_str(data);
    }
    let png = decode64(&b64);
    image::load_from_memory_with_format(&png, ImageFormat::Png).expect("kitty's payload is a PNG")
}

fn decode64(s: &str) -> Vec<u8> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => panic!("not base64: {c}"),
    };
    let mut out = Vec::new();
    for quad in s.as_bytes().chunks(4) {
        let n = quad.iter().take_while(|&&c| c != b'=').count();
        let mut acc = 0u32;
        for (i, &c) in quad[..n].iter().enumerate() {
            acc |= u32::from(val(c)) << (18 - 6 * i);
        }
        out.extend(&acc.to_be_bytes()[1..n.max(1)]);
    }
    out
}

fuzz_target!(|data: &[u8]| {
    let [a, b, flags, file @ ..] = data else {
        return;
    };
    let fit = flags & 1 == 0;
    let (a, b) = (u32::from(*a), u32::from(*b));
    std::fs::write(scratch(), file).expect("scratch file");
    let _ = pixels::is_plain_png(scratch());
    let mut asked = None;
    let decoded = pixels::decode_to(scratch(), |px| {
        // fitted into a box, as print and the TUI ask; or a size of its own, past the image's too
        let size = if fit {
            pixels::fitted(px, (1 + a * 8, 1 + b * 8))
        } else {
            (1 + a * 2, 1 + b * 2)
        };
        asked = Some((px, size));
        size
    });
    let Ok(out) = decoded else {
        return;
    };
    let (px, size) = asked.expect("decode_to asks for a size before it decodes");
    let want = (size.0.clamp(1, px.0.max(1)), size.1.clamp(1, px.1.max(1)));
    assert_eq!((out.width(), out.height()), want, "{px:?} asked {size:?}");
    assert!(matches!(
        out,
        DynamicImage::ImageRgb8(_) | DynamicImage::ImageRgba8(_)
    ));

    // the same file decoded whole by `image`, shrunk naively as stored, then turned. A whole decode and a naive
    // mean take ~10 s on a lying 65,498-wide header, so the reference runs up to 4 MP, where it costs little and
    // the fuzzer keeps its pace; ada's own decode above is still run, and bounded, at any size
    let reader = || {
        ImageReader::new(Cursor::new(file))
            .with_guessed_format()
            .ok()
    };
    let small = reader()
        .and_then(|r| r.into_dimensions().ok())
        .is_some_and(|(w, h)| u64::from(w) * u64::from(h) <= 4 << 20);
    let whole = reader()
        .filter(|_| small)
        .and_then(|r| r.into_decoder().ok())
        .and_then(|mut d| {
            let o = d.orientation().unwrap_or(Orientation::NoTransforms);
            DynamicImage::from_decoder(d).ok().map(|img| (img, o))
        });
    if let Some((img, o)) = whole {
        let img = match img {
            DynamicImage::ImageLuma8(_)
            | DynamicImage::ImageLumaA8(_)
            | DynamicImage::ImageRgb8(_)
            | DynamicImage::ImageRgba8(_) => Some(img),
            // ada's JPEG path turns the rest to 8-bit RGB(A) as `image` does; its PNG path strips
            // 16-bit samples where `image` scales them, so those don't compare
            other if image::guess_format(file).ok() != Some(ImageFormat::Png) => {
                Some(if other.color().has_alpha() {
                    DynamicImage::ImageRgba8(other.to_rgba8())
                } else {
                    DynamicImage::ImageRgb8(other.to_rgb8())
                })
            }
            _ => None,
        };
        if let Some(img) = img {
            let stored = (img.width(), img.height());
            assert_eq!(upright(stored, o), px, "ada and image disagree on the size");
            let dst = upright(want, o);
            let channels = usize::from(img.color().channel_count());
            let shrunk = naive(stored, dst, channels, img.as_bytes());
            let mut reference = if channels.is_multiple_of(2) {
                DynamicImage::ImageRgba8(
                    image::RgbaImage::from_raw(dst.0, dst.1, shrunk).expect("sized"),
                )
            } else {
                DynamicImage::ImageRgb8(
                    image::RgbImage::from_raw(dst.0, dst.1, shrunk).expect("sized"),
                )
            };
            reference.apply_orientation(o);
            assert!(
                reference.as_bytes() == out.as_bytes(),
                "{stored:?} {o:?} → {want:?}, {channels} channels: pixels differ from the naive box mean"
            );
        }
    }

    // kitty's PNG, decoded back, is the pixels
    let mut stream = Vec::new();
    kitty::transmit(
        &mut stream,
        1 + u32::from(*flags),
        &Payload::Pixels(&out),
        10,
        5,
    )
    .expect("transmit");
    let back = received(&stream);
    let back = if out.color().has_alpha() {
        DynamicImage::ImageRgba8(back.to_rgba8())
    } else {
        DynamicImage::ImageRgb8(back.to_rgb8())
    };
    assert!(
        back.as_bytes() == out.as_bytes(),
        "kitty's PNG doesn't decode to the pixels sent"
    );
});
