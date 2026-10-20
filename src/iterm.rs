//! iTerm2's inline images, for iTerm2 and WezTerm (the clean-slate plan's CS-3): a PNG of the
//! part of the image on screen, sent in one `OSC 1337 ; File=` sequence and drawn over the cells
//! at the cursor. Unlike kitty's placeholders, the terminal keeps no image to move with the text,
//! so each scroll that changes what's visible sends the image again.
//!
//! <https://iterm2.com/documentation-images.html>

use std::io::{self, Write};

use image::DynamicImage;

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
}
