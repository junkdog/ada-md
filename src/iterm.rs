//! iTerm2's inline images, for iTerm2 and WezTerm (the clean-slate plan's CS-3): a PNG in an
//! `OSC 1337 ; File=` sequence, drawn over the cells at the cursor. Unlike kitty's placeholders,
//! the terminal keeps no image to move with the text, so each scroll that changes what's visible
//! sends the rows on screen again: the TUI keeps a PNG a row ([`rows`]).
//!
//! <https://iterm2.com/documentation-images.html>

use std::io::{self, Write};

use image::{DynamicImage, RgbaImage, imageops};

use crate::kitty;

/// Send `image` to fill `cols` × `rows` cells from the cursor, stretched to them: the caller
/// sizes it to those cells already. The cursor stays where it was (`doNotMoveCursor`, which
/// terminals that don't know it ignore, so the caller places the cursor after it either way).
pub fn transmit(w: &mut impl Write, image: &DynamicImage, cols: u16, rows: u16) -> io::Result<()> {
    let mut png = Vec::new();
    kitty::png(image, &mut png)?;
    let mut out = Vec::with_capacity(png.len() / 3 * 4 + 128);
    out.extend_from_slice(b"\x1b]1337;File=inline=1;size=");
    out.extend_from_slice(png.len().to_string().as_bytes());
    out.extend_from_slice(
        format!(";width={cols};height={rows};preserveAspectRatio=0;doNotMoveCursor=1:").as_bytes(),
    );
    kitty::base64(&png, &mut out);
    out.extend_from_slice(b"\x1b\\");
    w.write_all(&out)
}

/// `image` cut into its rows of cells, `cell` pixels each, as a sequence a row that draws it over
/// `cols` cells from the cursor, so that a scroll draws the rows it shows without encoding them
/// again (as ratatui-image slices them). Each row erases its cells first, as the image's
/// transparent pixels would show what was there, and an image that doesn't fill its last row or
/// column of cells is padded to them with transparent pixels, never stretched.
pub fn rows(
    image: &DynamicImage,
    cell: (u32, u32),
    cols: u16,
    rows: u16,
) -> io::Result<Vec<Box<str>>> {
    let (cw, ch) = (cell.0.max(1), cell.1.max(1));
    let width = u32::from(cols) * cw;
    (0..u32::from(rows))
        .map(|r| {
            let top = (r * ch).min(image.height());
            let h = ch.min(image.height() - top);
            let crop = image.crop_imm(0, top, width.min(image.width()), h);
            let row = if (crop.width(), crop.height()) == (width, ch) {
                crop
            } else {
                let mut padded = RgbaImage::new(width, ch);
                imageops::replace(&mut padded, &crop.to_rgba8(), 0, 0);
                DynamicImage::ImageRgba8(padded)
            };
            let mut out = format!("\x1b[{cols}X").into_bytes();
            transmit(&mut out, &row, cols, 1)?;
            String::from_utf8(out)
                .map(String::into_boxed_str)
                .map_err(io::Error::other)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use image::{Rgb, RgbImage};

    use super::*;

    #[test]
    fn an_image_is_a_png_in_one_sequence() {
        let image = DynamicImage::ImageRgb8(RgbImage::from_fn(30, 20, |x, y| {
            Rgb([(x * 8) as u8, (y * 12) as u8, 99])
        }));
        let mut out = Vec::new();
        transmit(&mut out, &image, 6, 2).unwrap();
        let text = String::from_utf8(out).unwrap();
        let body = text
            .strip_prefix("\x1b]1337;File=")
            .and_then(|t| t.strip_suffix("\x1b\\"))
            .expect("one OSC 1337 sequence");
        let (args, b64) = body.split_once(':').unwrap();
        let args: Vec<&str> = args.split(';').collect();
        for a in ["inline=1", "width=6", "height=2", "preserveAspectRatio=0"] {
            assert!(args.contains(&a), "{a} in {args:?}");
        }
        assert!(!b64.contains(['\x1b', '\x07', ';']));
        let mut png = Vec::new();
        kitty::png(&image, &mut png).unwrap();
        assert!(args.contains(&format!("size={}", png.len()).as_str()));
        let mut want = Vec::new();
        kitty::base64(&png, &mut want);
        assert_eq!(b64.as_bytes(), want);
    }

    /// The PNG in a row's sequence, decoded.
    fn row_png(seq: &str, cols: u16) -> DynamicImage {
        let seq = seq
            .strip_prefix(&format!("\x1b[{cols}X"))
            .expect("the cells erased first");
        let b64 = seq
            .strip_suffix("\x1b\\")
            .and_then(|s| s.split_once(':'))
            .unwrap()
            .1;
        let abc = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut bytes = Vec::new();
        let (mut acc, mut bits) = (0u32, 0);
        for c in b64.bytes().take_while(|&c| c != b'=') {
            acc = acc << 6 | abc.iter().position(|&a| a == c).unwrap() as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                bytes.push((acc >> bits) as u8);
            }
        }
        image::load_from_memory(&bytes).unwrap()
    }

    #[test]
    fn rows_are_whole_cells_padded_with_transparent_pixels() {
        // 25 × 47 pixels in cells of 10 × 20: 3 columns and 3 rows, the last row 7 pixels high
        let image = DynamicImage::ImageRgb8(RgbImage::from_fn(25, 47, |x, y| {
            Rgb([(x * 9) as u8, (y * 5) as u8, 7])
        }));
        let seqs = rows(&image, (10, 20), 3, 3).unwrap();
        assert_eq!(seqs.len(), 3);
        for (r, seq) in seqs.iter().enumerate() {
            assert!(seq.contains(";width=3;height=1;"), "{r}");
            let png = row_png(seq, 3).to_rgba8();
            assert_eq!(png.dimensions(), (30, 20));
            for (x, y, p) in png.enumerate_pixels() {
                let src = y + 20 * r as u32;
                let want = if x < 25 && src < 47 {
                    [(x * 9) as u8, (src * 5) as u8, 7, 255]
                } else {
                    [0; 4]
                };
                assert_eq!(p.0, want, "row {r} ({x}, {y})");
            }
        }
    }
}
