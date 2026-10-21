//! ada's sixel encoder (CS-6): pixels in, `sixel::Palette` and `sixel::Indexed` out, the output
//! decoded back. For any image, palette size, dither and cut:
//!   - the output is one DCS sixel sequence of the size asked for, which a strict decoder reads:
//!     colors defined before they're used, as the palette's colors, no sixel outside the image and
//!     no pixel drawn twice
//!   - a pixel is drawn exactly when it's opaque (alpha 128 and up), in a color of the palette
//!   - undithered, each pixel's color is within the nearest-color cache's error of the palette's
//!     nearest; an image of a few colors the percentages keep exactly comes back exactly
//!   - an `Indexed` image's cuts, any rows and columns, `prepare` called or not, are the whole
//!     image's pixels there, and undithered the same as `Palette::encode` of those rows
//!   - the output's length is bounded by the bands, the colors in them and the width
//!
//! The first bytes pick the size, alpha, palette size, dither and the kind of pixels (the rest of
//! the input as bytes, a few exact colors, a gradient); the cuts come after.
//!
//! ```zsh
//! cargo +nightly fuzz run -s none -a sixel -- -max_len=8192
//! ```

#![no_main]

use std::collections::HashMap;
use std::ops::Range;

use ada::sixel::{Dither, Indexed, MAX_COLORS, Palette};
use image::{DynamicImage, RgbImage, RgbaImage};
use libfuzzer_sys::arbitrary::{Result, Unstructured};
use libfuzzer_sys::fuzz_target;

/// Levels that survive the trip through percentages unchanged.
const EXACT: [u8; 6] = [0, 51, 102, 153, 204, 255];

/// A sixel image decoded: its size and each pixel's color, `None` where nothing was drawn. Panics
/// on anything ada shouldn't write.
fn decode(data: &[u8], palette: &[[u8; 3]]) -> (usize, usize, Vec<Option<[u8; 3]>>) {
    let text = || data.escape_ascii().to_string();
    let body = data
        .strip_prefix(b"\x1bP9;1;0q")
        .and_then(|d| d.strip_suffix(b"\x1b\\"))
        .unwrap_or_else(|| panic!("not a DCS sixel sequence: {}", text()));
    assert!(!body.contains(&0x1b), "an Esc inside: {}", text());
    let mut i = 0;
    let num = |i: &mut usize| {
        let start = *i;
        while *i < body.len() && body[*i].is_ascii_digit() {
            *i += 1;
        }
        assert!(*i > start, "a number expected at {start}: {}", text());
        std::str::from_utf8(&body[start..*i])
            .unwrap()
            .parse::<usize>()
            .unwrap()
    };
    assert_eq!(body.first(), Some(&b'"'), "raster attributes: {}", text());
    i += 1;
    let mut raster = [0; 4];
    for (k, r) in raster.iter_mut().enumerate() {
        if k > 0 {
            assert_eq!(body[i], b';');
            i += 1;
        }
        *r = num(&mut i);
    }
    let [1, 1, w, h] = raster else {
        panic!("raster {raster:?}");
    };
    let mut px = vec![None; w * h];
    let mut defs = HashMap::new();
    let (mut color, mut x, mut band) = (None, 0, 0);
    while i < body.len() {
        let b = body[i];
        i += 1;
        match b {
            b'#' => {
                let c = num(&mut i);
                assert!(c < palette.len(), "color {c} of {}", palette.len());
                if body.get(i) == Some(&b';') {
                    i += 1;
                    assert_eq!(num(&mut i), 2, "RGB");
                    let mut rgb = [0u8; 3];
                    for v in &mut rgb {
                        assert_eq!(body[i], b';');
                        i += 1;
                        let p = num(&mut i);
                        assert!(p <= 100, "{p}%");
                        *v = ((p * 255 + 50) / 100) as u8;
                    }
                    assert_eq!(rgb, palette[c], "color {c} defined as another");
                    assert!(defs.insert(c, rgb).is_none(), "color {c} defined twice");
                } else {
                    assert!(defs.contains_key(&c), "color {c} used before it's defined");
                }
                color = Some(c);
            }
            b'$' => x = 0,
            b'-' => (x, band) = (0, band + 1),
            b'!' | b'?'..=b'~' => {
                let (n, bits) = if b == b'!' {
                    let n = num(&mut i);
                    assert!(n > 0, "a repeat of 0");
                    let c = body[i];
                    i += 1;
                    assert!((b'?'..=b'~').contains(&c), "repeat of {c:#x}");
                    (n, c - b'?')
                } else {
                    (1, b - b'?')
                };
                let c = color.expect("a sixel before any color");
                for _ in 0..n {
                    for bit in 0..6 {
                        if bits & 1 << bit != 0 {
                            let y = band * 6 + bit;
                            assert!(x < w && y < h, "({x}, {y}) outside {w} × {h}");
                            let p = &mut px[y * w + x];
                            assert!(p.is_none(), "({x}, {y}) drawn twice");
                            *p = Some(defs[&c]);
                        }
                    }
                    x += 1;
                }
            }
            _ => panic!("unexpected byte {b:#x} at {i}: {}", text()),
        }
    }
    (w, h, px)
}

/// The most bytes the encoding of `rows` × `w` pixels in a palette of `colors` may take: the
/// header and color definitions, then per band, each color in it as one pass no wider than the
/// image, with its `$`, `#n` and the run up to its first column.
fn bound(w: usize, rows: usize, colors: usize) -> usize {
    let bands = rows.div_ceil(6);
    let passes = colors.min(6 * w);
    32 + 18 * colors + bands * (1 + passes * (w + 12)) + 2
}

struct Case {
    image: DynamicImage,
    colors: usize,
    dither: Dither,
    /// Every opaque pixel is one of a few exact colors, no more than the palette holds.
    exact: bool,
}

fn case(u: &mut Unstructured) -> Result<Case> {
    let w = u.int_in_range(1..=400u32)?;
    let h = u.int_in_range(1..=300u32)?;
    let alpha = u.arbitrary::<bool>()?;
    let colors = match u.int_in_range(0..=3)? {
        0 => MAX_COLORS,
        1 => usize::from(u.arbitrary::<u8>()?),
        2 => u.int_in_range(0..=16)?,
        _ => u.int_in_range(256..=1000)?,
    };
    let dither = if u.arbitrary()? {
        Dither::None
    } else {
        Dither::FloydSteinberg
    };
    let ch = if alpha { 4 } else { 3 };
    let mut exact = false;
    let px: Vec<u8> = match u.int_in_range(0..=2)? {
        // the input's bytes, over and over
        0 => {
            let bytes = u.peek_bytes(u.len()).unwrap_or(&[]).to_vec();
            if bytes.is_empty() {
                vec![0; (w * h) as usize * ch]
            } else {
                bytes
                    .iter()
                    .copied()
                    .cycle()
                    .take((w * h) as usize * ch)
                    .collect()
            }
        }
        // a few exact colors, as many as the palette holds at most
        1 => {
            let n = u.int_in_range(1..=colors.clamp(2, MAX_COLORS).min(216))?;
            let picks: Vec<[u8; 3]> = (0..n)
                .map(|k| {
                    let k = (k * 97 + 13) % 216;
                    [EXACT[k / 36], EXACT[k / 6 % 6], EXACT[k % 6]]
                })
                .collect();
            let (a, b) = (u.int_in_range(1..=31u32)?, u.int_in_range(1..=31u32)?);
            exact = true;
            (0..w * h)
                .flat_map(|i| {
                    let (x, y) = (i % w, i / w);
                    let c = picks[((x * a + y * b) as usize) % n];
                    let opaque = !alpha || (x + y) % 5 != 0;
                    c.into_iter()
                        .chain(alpha.then_some(if opaque { 255 } else { 0 }))
                })
                .collect()
        }
        // a gradient
        _ => {
            let (a, b, c) = (
                u.arbitrary::<u8>()?,
                u.arbitrary::<u8>()?,
                u.arbitrary::<u8>()?,
            );
            (0..w * h)
                .flat_map(|i| {
                    let (x, y) = (i % w, i / w);
                    let v = [
                        (x * u32::from(a) / w) as u8,
                        (y * u32::from(b) / h) as u8,
                        ((x + y) * u32::from(c) / (w + h)) as u8,
                    ];
                    let alpha = ((x * 7 + y * 3) % 256) as u8;
                    v.into_iter().chain((ch == 4).then_some(alpha))
                })
                .collect()
        }
    };
    let image = if alpha {
        DynamicImage::ImageRgba8(RgbaImage::from_raw(w, h, px).expect("size"))
    } else {
        DynamicImage::ImageRgb8(RgbImage::from_raw(w, h, px).expect("size"))
    };
    Ok(Case {
        image,
        colors,
        dither,
        exact,
    })
}

/// The squared distance of two colors.
fn dist(a: [u8; 3], b: [u8; 3]) -> i32 {
    (0..3)
        .map(|k| (i32::from(a[k]) - i32::from(b[k])).pow(2))
        .sum()
}

/// Rows `rows` of the source as `Option<[u8; 3]>`: `None` where a pixel isn't opaque.
fn source(image: &DynamicImage, rows: Range<u32>, cols: u32) -> Vec<Option<[u8; 3]>> {
    let rgba = image.to_rgba8();
    rows.flat_map(|y| (0..cols).map(move |x| (x, y)))
        .map(|(x, y)| {
            let p = rgba.get_pixel(x, y).0;
            (p[3] >= 128).then_some([p[0], p[1], p[2]])
        })
        .collect()
}

/// Check one encoding of rows `rows` and the first `cols` columns, decoded as `px`.
fn check(case: &Case, palette: &[[u8; 3]], rows: Range<u32>, cols: u32, px: &[Option<[u8; 3]>]) {
    let want = source(&case.image, rows, cols);
    assert_eq!(px.len(), want.len());
    for (k, (got, src)) in px.iter().zip(&want).enumerate() {
        let (x, y) = (k as u32 % cols, k as u32 / cols);
        assert_eq!(
            got.is_some(),
            src.is_some(),
            "({x}, {y}): drawn {got:?}, source {src:?}"
        );
        let (Some(got), Some(src)) = (got, src) else {
            continue;
        };
        if case.exact {
            assert_eq!(got, src, "({x}, {y}) of {} colors", palette.len());
        } else if case.dither == Dither::None {
            // the cache maps a cell 4 levels wide to the color nearest its center: at most
            // 2 × √12 ≈ 7 farther than the nearest
            let best = palette.iter().map(|&c| dist(c, *src)).min().unwrap();
            let d = f64::from(dist(*got, *src)).sqrt();
            assert!(
                d <= f64::from(best).sqrt() + 7.0,
                "({x}, {y}): {src:?} drawn as {got:?}, {d} away, the nearest {}",
                f64::from(best).sqrt()
            );
        }
    }
}

fn run(u: &mut Unstructured) -> Result<()> {
    let case = case(u)?;
    let (w, h) = (case.image.width(), case.image.height());
    let palette = Palette::new(&case.image, case.colors);
    let colors = palette.colors();
    assert!(colors.len() <= case.colors.clamp(2, MAX_COLORS));
    let opaque = source(&case.image, 0..h, w).iter().any(Option::is_some);
    assert_eq!(colors.is_empty(), !opaque, "{} colors", colors.len());

    // whole rows, through the palette
    let mut out = Vec::new();
    let cuts = u.int_in_range(1..=3)?;
    for n in 0..cuts {
        let rows = if n == 0 {
            0..h
        } else {
            let a = u.int_in_range(0..=h + 4)?;
            a..u.int_in_range(a..=h + 8)?
        };
        out.clear();
        palette.encode(&case.image, rows.clone(), case.dither, &mut out);
        let rows = rows.start.min(h)..rows.end.min(h);
        if rows.is_empty() {
            assert!(out.is_empty());
            continue;
        }
        assert!(
            out.len() <= bound(w as usize, rows.len(), colors.len()),
            "{} bytes",
            out.len()
        );
        let (dw, dh, px) = decode(&out, colors);
        assert_eq!((dw, dh), (w as usize, rows.len()));
        check(&case, colors, rows, w, &px);
    }

    // an indexed image, cut anywhere
    let indexed = Indexed::new(&case.image, case.colors, case.dither);
    assert_eq!(indexed.size(), (w, h));
    if u.arbitrary()? {
        indexed.prepare(u32::from(u.arbitrary::<u8>()?));
    }
    out.clear();
    indexed.encode(0..h, w, &mut out);
    let whole = if opaque || !out.is_empty() {
        let (_, _, px) = decode(&out, colors);
        check(&case, colors, 0..h, w, &px);
        px
    } else {
        Vec::new()
    };
    for _ in 0..u.int_in_range(0..=8)? {
        let a = u.int_in_range(0..=h + 4)?;
        let rows = a..u.int_in_range(a..=h + 8)?;
        let cols = u.int_in_range(0..=w + 4)?;
        out.clear();
        indexed.encode(rows.clone(), cols, &mut out);
        let (rows, cols) = (rows.start.min(h)..rows.end.min(h), cols.min(w));
        if rows.is_empty() || cols == 0 {
            assert!(out.is_empty());
            continue;
        }
        assert!(
            out.len() <= bound(cols as usize, rows.len(), colors.len()),
            "{} bytes",
            out.len()
        );
        let (dw, dh, px) = decode(&out, colors);
        assert_eq!((dw, dh), (cols as usize, rows.len()));
        for (k, p) in px.iter().enumerate() {
            let (x, y) = (k % dw, k / dw + rows.start as usize);
            assert_eq!(
                *p,
                whole[y * w as usize + x],
                "({x}, {y}) cut at {rows:?} × {cols}"
            );
        }
        if case.dither == Dither::None && cols == w {
            let mut again = Vec::new();
            palette.encode(&case.image, rows.clone(), Dither::None, &mut again);
            assert_eq!(decode(&again, colors).2, px, "rows {rows:?}");
        }
    }
    Ok(())
}

fuzz_target!(|data: &[u8]| {
    let _ = run(&mut Unstructured::new(data));
});
