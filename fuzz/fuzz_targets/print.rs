//! `--print` as `main` runs it (ADR-15): a document read from stdin or a file, its type
//! sniffed or forced by `-l`, cut to a preview's head, laid out and written as SGR text, with
//! line numbers, code wrapped or cut, `--line` alignment, a bound on the rows, and block images
//! drawn with kitty placeholders or half-blocks. Links lead to fixed images (PNG of each color
//! type, JPEG, truncated, not an image at all, missing) or to one file whose bytes the input
//! holds, with sizes true or not, as a file changed since its probe. Nothing may panic.
//!
//! ```zsh
//! cargo +nightly fuzz run -s none -a print -- -dict=fuzz/markdown.dict -max_len=16384
//! ```

#![no_main]

mod skip;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use ada::app::{Head, cut_to_head};
use ada::filetype::{self, FileType};
use ada::geom::Size;
use ada::images::Proto;
use ada::layout::{ImageFit, LayoutParams, LineNumbers, TableMode};
use ada::print::{self, Options, Pictures};
use ada::theme::Theme;
use image::{DynamicImage, ImageFormat};
use libfuzzer_sys::arbitrary::{self, Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;

/// `main`'s `PREVIEW_ROWS`.
const PREVIEW_ROWS: usize = 2000;

#[derive(Arbitrary, Debug)]
struct Setup {
    width: u8,
    max_width: Option<u8>,
    /// `--line`.
    line: Option<u16>,
    /// `--height`, or fzf's pane.
    preview: Option<u8>,
    /// Where the text comes from: `-`, a name with no extension, or one with.
    name: u8,
    /// `-l`, as an index into [`FORCED`].
    forced: u8,
    /// The low bits pick the line numbers; the top bit turns `--code-wrap` off.
    line_numbers: u8,
    table_mode: u8,
    /// Images on, by protocol: kitty or half-blocks.
    images: Option<bool>,
    reuse_ids: bool,
    cell: (u8, u8),
    /// The fixture each link leads to, cycled.
    picks: Vec<u8>,
    /// Sizes each image claims, cycled; the true one where there's none.
    lies: Vec<(u16, u16)>,
    /// Bytes of the one image the input holds.
    image_len: u16,
}

const NAMES: [&str; 8] = [
    "-",
    "-",
    "fuzz.mmd",
    "NOTES",
    "fuzz.md",
    "fuzz.txt",
    "fuzz.rs",
    "fuzz.json",
];

const FORCED: [&str; 12] = [
    "", "", "", "mmd", "md", "txt", "rs", "yaml", "diff", "log", "toml", "html",
];

/// The scratch directory, and each fixture with its true size.
type Fixtures = (PathBuf, Vec<(PathBuf, (u32, u32))>);

/// The fixture images, written once a process, with their true sizes; the last is the input's.
fn fixtures() -> &'static Fixtures {
    static FIXTURES: OnceLock<Fixtures> = OnceLock::new();
    FIXTURES.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("ada-fuzz-print-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let mut files = Vec::new();
        let mut save = |name: &str, image: DynamicImage, format| {
            let path = dir.join(name);
            image.save_with_format(&path, format).expect("fixture");
            files.push((path, (image.width(), image.height())));
        };
        let gradient = |w, h| {
            image::RgbaImage::from_fn(w, h, |x, y| {
                image::Rgba([(x * 7) as u8, (y * 11) as u8, (x ^ y) as u8, (x * y) as u8])
            })
        };
        let rgb =
            |w, h| DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(gradient(w, h)).to_rgb8());
        let rgba = DynamicImage::ImageRgba8(gradient(7, 9));
        save("rgb.png", rgb(40, 20), ImageFormat::Png);
        save("rgba.png", rgba.clone(), ImageFormat::Png);
        save(
            "gray.png",
            DynamicImage::ImageLuma8(rgba.to_luma8()),
            ImageFormat::Png,
        );
        save(
            "graya.png",
            DynamicImage::ImageLumaA8(rgba.to_luma_alpha8()),
            ImageFormat::Png,
        );
        save(
            "rgb16.png",
            DynamicImage::ImageRgb16(rgba.to_rgb16()),
            ImageFormat::Png,
        );
        save("dot.png", rgb(1, 1), ImageFormat::Png);
        save("photo.jpg", rgb(33, 17), ImageFormat::Jpeg);
        save(
            "gray.jpg",
            DynamicImage::ImageLuma8(rgba.to_luma8()),
            ImageFormat::Jpeg,
        );
        let png = std::fs::read(&files[0].0).expect("fixture");
        let cut = dir.join("cut.png");
        std::fs::write(&cut, &png[..png.len() / 2]).expect("fixture");
        files.push((cut, (40, 20)));
        let text = dir.join("text.png");
        std::fs::write(&text, "not an image").expect("fixture");
        files.push((text, (3, 3)));
        files.push((dir.join("gone.png"), (8, 8)));
        files.push((dir.join("input.img"), (16, 16)));
        (dir, files)
    })
}

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);
    let Ok(setup) = Setup::arbitrary(&mut u) else {
        return;
    };
    let Ok(image) = u.bytes(usize::from(setup.image_len).min(u.len())) else {
        return;
    };
    let mut bytes = u.take_rest().to_vec();
    let (_, fixtures) = fixtures();
    let width = u16::from(setup.width).max(1);
    let path = Path::new(NAMES[usize::from(setup.name) % NAMES.len()]);
    let forced = Some(FORCED[usize::from(setup.forced) % FORCED.len()])
        .filter(|name| !name.is_empty())
        .and_then(FileType::named);
    let preview = setup.preview.map(|h| u16::from(h).max(1));
    let rows = preview.map(|h| usize::from(h).max(PREVIEW_ROWS));

    // `print_doc`'s read: the head of what isn't markdown, then no binary unless forced
    let head = rows
        .filter(|_| forced != Some(FileType::Markdown))
        .map(|rows| {
            let lines = setup.line.map_or(0, usize::from) + rows;
            Head {
                lines,
                bytes: (lines * (usize::from(width) + 1) * 4).max(1 << 16),
            }
        });
    if let Some(head) = head {
        bytes.truncate(head.bytes);
        let sniffed = || FileType::of(path, &String::from_utf8_lossy(&bytes));
        let markdown = if path == Path::new("-") {
            forced.unwrap_or_else(sniffed) == FileType::Markdown
        } else {
            FileType::by_path(path) == Some(FileType::Markdown)
        };
        if !markdown {
            cut_to_head(&mut bytes, head);
        }
    }
    if forced.is_none() && filetype::is_binary(&bytes) {
        return;
    }
    let text = String::from_utf8_lossy(&bytes);
    if skip::makefile(&text) {
        return;
    }

    let theme = Theme::builtin();
    let proto = setup.images.map(|kitty| {
        if kitty {
            Proto::Kitty
        } else {
            Proto::Halfblocks
        }
    });
    let cell = Size {
        width: u16::from(setup.cell.0 % 32).max(1),
        height: u16::from(setup.cell.1 % 64).max(1),
    };
    let height = preview.unwrap_or(24);
    let params = LayoutParams {
        width,
        max_width: setup.max_width.map(u16::from),
        table_mode: [TableMode::Auto, TableMode::Grid, TableMode::Records]
            [usize::from(setup.table_mode) % 3],
        code_wrap: setup.line_numbers & 0x80 == 0,
        line_numbers: [LineNumbers::Off, LineNumbers::Code, LineNumbers::All]
            [usize::from(setup.line_numbers) % 3],
        glyphs_gen: theme.glyphs.generation(),
        images: proto.map(|_| ImageFit {
            cell,
            max_rows: (height * 3 / 5).max(1),
            screen_rows: height,
        }),
    };
    let filetype = forced.unwrap_or_else(|| FileType::of(path, &text));
    let doc = filetype.parse(path, &text);

    let pictures = proto.map(|proto| {
        if image.is_empty() {
            let _ = std::fs::remove_file(&fixtures[fixtures.len() - 1].0);
        } else {
            std::fs::write(&fixtures[fixtures.len() - 1].0, image).expect("input image");
        }
        let files = (0..doc.links.len())
            .map(|i| {
                let pick = setup.picks.get(i % setup.picks.len().max(1)).copied()?;
                let (path, px) = fixtures.get(usize::from(pick) % (fixtures.len() + 1))?;
                let lie = setup.lies.get(i % setup.lies.len().max(1));
                let px = lie.map_or(*px, |&(w, h)| (u32::from(w), u32::from(h)));
                Some((path.clone(), px))
            })
            .collect();
        Pictures {
            proto,
            cell,
            files,
            reuse_ids: setup.reuse_ids,
        }
    });
    let opts = Options {
        align: setup.line.map(u32::from),
        rows,
        images: pictures.as_ref(),
    };
    let trace = std::env::var_os("ADA_FUZZ_TRACE").is_some();
    if trace {
        eprintln!(
            "{setup:#?}\n{filetype:?} at {path:?}, {} bytes of text, {} links, image {:02x?}",
            text.len(),
            doc.links.len(),
            &image[..image.len().min(32)]
        );
        eprintln!("{:#?}", pictures.as_ref().map(|p| &p.files));
    }
    if let Some(dump) = std::env::var_os("ADA_FUZZ_DUMP") {
        std::fs::write(dump, text.as_bytes()).expect("dump");
    }
    let start = std::time::Instant::now();
    let out = print::render(&doc, &theme, filetype.params(params), opts);
    if trace {
        eprintln!(
            "{} bytes out, {} lines, in {:?}",
            out.len(),
            out.lines().count(),
            start.elapsed()
        );
    }
    // the output is written as UTF-8 in pieces: none may split a character
    assert!(
        !out.contains('\u{FFFD}') || text.contains('\u{FFFD}'),
        "the output has bytes that aren't UTF-8"
    );
});
