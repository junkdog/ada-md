//! Images (ADR-09, architecture §9). Everything that knows about ratatui-image lives here: sixel
//! and iTerm2. Kitty and half-blocks are ada's own, as print draws them ([`kitty`],
//! [`pixels::HalfBlock`]; CS-1 of the clean-slate plan), and so is the terminal's query
//! ([`probe`], CS-2).
//!
//! The terminal is queried once at startup, and answers on the input thread while the text paints
//! (O-06); its protocol and font cell size reach `update` as `Msg::Graphics`, and start the image
//! worker. Reading an image's header touches the filesystem, so it is a world effect
//! (`Cmd::ProbeImages`) whose result is recorded. Decoding and encoding run on a worker
//! (`Cmd::DecodeImage`) and are cached by `(path, mtime, cells)`. Hermetic replay has no image
//! bytes to recompute them from, so replayed images stay placeholders.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Instant, UNIX_EPOCH};

use image::imageops::FilterType;
use image::metadata::Orientation;
use image::{ImageDecoder, ImageReader};
use ratatui::layout::{Rect, Size};
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::sliced::{SignedPosition, SlicedImage, SlicedProtocol};
use ratatui_image::{FontSize, Resize};

use crate::grid::Grid;
use crate::kitty;
use crate::pixels::{self, HalfBlock};
use crate::probe;

use crate::app::{self, Computed, Msg};

/// Encoded images kept before the oldest are dropped.
const CACHE_CAP: usize = 48;

/// A terminal graphics protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proto {
    Kitty,
    Sixel,
    Iterm2,
    Halfblocks,
}

impl Proto {
    pub const ALL: [Proto; 4] = [Proto::Kitty, Proto::Sixel, Proto::Iterm2, Proto::Halfblocks];

    pub fn name(self) -> &'static str {
        match self {
            Proto::Kitty => "kitty",
            Proto::Sixel => "sixel",
            Proto::Iterm2 => "iterm2",
            Proto::Halfblocks => "halfblocks",
        }
    }

    pub fn parse(name: &str) -> Option<Proto> {
        Proto::ALL.into_iter().find(|p| p.name() == name)
    }

    fn protocol_type(self) -> ProtocolType {
        match self {
            Proto::Kitty => ProtocolType::Kitty,
            Proto::Sixel => ProtocolType::Sixel,
            Proto::Iterm2 => ProtocolType::Iterm2,
            Proto::Halfblocks => ProtocolType::Halfblocks,
        }
    }
}

/// Whether to show images: `--no-images`, `--image-protocol P`, or what the terminal supports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageMode {
    Off,
    #[default]
    Auto,
    Force(Proto),
}

/// What `update` knows about the terminal's images.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Graphics {
    pub proto: Proto,
    /// The font's cell size in pixels.
    pub cell: Size,
}

impl Graphics {
    /// The message that tells `update`.
    pub fn msg(self) -> Msg {
        Msg::Graphics {
            proto: self.proto,
            cell: app::Size {
                w: self.cell.width,
                h: self.cell.height,
            },
        }
    }
}

/// What the environment decides before the terminal is asked anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Write [`probe::QUERY`] and read the replies.
    pub ask: bool,
    /// The protocol, whatever the terminal says: the cell size is still asked for.
    pub forced: Option<Proto>,
}

/// How to find the terminal's image support: `None` when images are off. Inside tmux, half-blocks
/// without asking: kitty graphics don't work there (plan Q10), sixel and iTerm2 reach the terminal
/// only through tmux's passthrough, which misplaces them and loses them on tmux's redraws
/// (KNOWN_ISSUES.md), and a detached tmux answers no query (O-20).
pub fn plan(mode: ImageMode, env: impl Fn(&str) -> Option<String>) -> Option<Plan> {
    let forced = match mode {
        ImageMode::Off => return None,
        ImageMode::Auto => None,
        ImageMode::Force(p) => Some(p),
    };
    let tmux = env("TMUX").is_some_and(|v| !v.is_empty())
        || env("TERM").is_some_and(|t| t.starts_with("tmux"))
        || env("TERM_PROGRAM").is_some_and(|t| t == "tmux");
    Some(if tmux {
        Plan {
            ask: false,
            forced: Some(forced.unwrap_or(Proto::Halfblocks)),
        }
    } else {
        Plan {
            ask: cfg!(unix),
            forced,
        }
    })
}

/// The terminal's image support, from what it `found` to say, with ratatui-image 11.1's rules:
/// kitty, then sixel, then iTerm2 from the environment, else half-blocks. WezTerm and Konsole
/// answer kitty's query but draw no Unicode placeholders, and Konsole's sixel is buggy
/// (ratatui-image's compatibility matrix), so neither is taken from them. The cell size comes
/// from `CSI 16t`, else `CSI 14t` over the terminal's `cells`, else `TIOCGWINSZ` (`winsize`);
/// with none of them, half-blocks at 10×20.
pub fn decide(
    plan: Plan,
    found: &probe::Found,
    env: impl Fn(&str) -> Option<String>,
    cells: Size,
    winsize: Option<Size>,
) -> Graphics {
    let set = |name| env(name).is_some_and(|v| !v.is_empty());
    let area = |(w, h): (u16, u16)| {
        (cells.width > 0 && cells.height > 0).then(|| Size::new(w / cells.width, h / cells.height))
    };
    let cell = found
        .cell
        .map(|(w, h)| Size::new(w, h))
        .filter(|c| c.width > 0 && c.height > 0)
        .or_else(|| found.text_area.and_then(area))
        .filter(|c| c.width > 0 && c.height > 0)
        .or(winsize);
    let trusted = !set("WEZTERM_EXECUTABLE") && !set("KONSOLE_VERSION");
    let iterm2 = env("TERM_PROGRAM").is_some_and(|t| {
        [
            "iTerm",
            "WezTerm",
            "mintty",
            "vscode",
            "Tabby",
            "Hyper",
            "rio",
            "Bobcat",
            "WarpTerminal",
        ]
        .iter()
        .any(|name| t.contains(name))
    }) || env("LC_TERMINAL").is_some_and(|t| t.contains("iTerm"));
    let proto = plan.forced.unwrap_or(match cell {
        None => Proto::Halfblocks,
        Some(_) if trusted && found.kitty == Some(true) => Proto::Kitty,
        Some(_) if trusted && found.sixel() => Proto::Sixel,
        Some(_) if iterm2 => Proto::Iterm2,
        Some(_) => Proto::Halfblocks,
    });
    Graphics {
        proto,
        cell: cell.unwrap_or(Size::new(10, 20)),
    }
}

/// Ask the terminal and wait for its answer, for a tool that reads nothing else meanwhile (the TUI
/// reads the answer on its input thread instead). Call it in raw mode. `None` when images are off.
pub fn detect(mode: ImageMode) -> Option<Graphics> {
    let env = |name: &str| std::env::var(name).ok();
    let plan = plan(mode, env)?;
    let found = if plan.ask {
        let mut out = std::io::stdout();
        out.write_all(probe::QUERY)
            .and_then(|()| out.flush())
            .ok()?;
        probe::read(Instant::now() + probe::WAIT, |_| true)
    } else {
        probe::Found::default()
    };
    let (w, h) = ratatui::crossterm::terminal::size().unwrap_or((0, 0));
    Some(decide(plan, &found, env, Size::new(w, h), winsize_cell()))
}

/// What encodes images for the terminal: its protocol and the font's cell size.
#[derive(Clone)]
pub struct Encoder {
    proto: Proto,
    cell: Size,
    /// ratatui-image's, which draws sixel and iTerm2 until CS-3. Only for those: building one
    /// runs `tmux set -p allow-passthrough on` inside tmux, a change to the user's pane.
    picker: Option<Picker>,
}

impl Encoder {
    #[allow(deprecated)] // `from_fontsize`: ratatui-image would rather ask the terminal itself
    pub fn new(proto: Proto, cell: Size) -> Encoder {
        let cell = Size::new(cell.width.max(1), cell.height.max(1));
        let picker = matches!(proto, Proto::Sixel | Proto::Iterm2).then(|| {
            let mut picker = Picker::from_fontsize(FontSize::new(cell.width, cell.height));
            picker.set_protocol_type(proto.protocol_type());
            picker
        });
        Encoder {
            proto,
            cell,
            picker,
        }
    }

    /// Half-blocks at 10×20, for a terminal that wasn't asked.
    pub fn halfblocks() -> Encoder {
        Encoder::new(Proto::Halfblocks, Size::new(10, 20))
    }
}

/// The protocol for printed images, from the environment alone: under fzf, which owns the
/// terminal, there is no asking it. Kitty (and Ghostty) get unicode placeholders, everything else
/// half-blocks, and inside tmux half-blocks only, as in the TUI. Printing has no sixel or iTerm2:
/// forcing one prints half-blocks.
pub fn print_proto(mode: ImageMode, env: impl Fn(&str) -> Option<String>) -> Option<Proto> {
    let set = |name| env(name).is_some_and(|v| !v.is_empty());
    let proto = match mode {
        ImageMode::Off => return None,
        ImageMode::Force(p) => p,
        ImageMode::Auto if set("TMUX") => Proto::Halfblocks,
        ImageMode::Auto => {
            let term = env("TERM").unwrap_or_default();
            let kitty = set("KITTY_WINDOW_ID") || term == "xterm-kitty";
            let ghostty = set("GHOSTTY_RESOURCES_DIR") || term == "xterm-ghostty";
            if kitty || ghostty {
                Proto::Kitty
            } else {
                Proto::Halfblocks
            }
        }
    };
    Some(match proto {
        Proto::Kitty => Proto::Kitty,
        _ => Proto::Halfblocks,
    })
}

/// The font's cell size, from the terminal's size in cells and pixels (`TIOCGWINSZ` on
/// `/dev/tty`, as `kitten icat` reads it), or 10×20 when it reports no pixels.
pub fn cell_size() -> Size {
    winsize_cell().unwrap_or(Size::new(10, 20))
}

/// The font's cell size by `TIOCGWINSZ`, if the terminal reports pixels.
pub fn winsize_cell() -> Option<Size> {
    ratatui::crossterm::terminal::window_size()
        .ok()
        .filter(|s| s.columns > 0 && s.rows > 0 && s.width > 0 && s.height > 0)
        .map(|s| Size::new((s.width / s.columns).max(1), (s.height / s.rows).max(1)))
}

/// What reading an image's header found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Probe {
    /// The size in pixels, upright (EXIF orientation applied), and the file's mtime in
    /// milliseconds since the epoch.
    Image {
        w: u32,
        h: u32,
        mtime: Option<u64>,
    },
    Failed(String),
}

/// Read an image's header: cheap, as nothing is decoded.
pub fn probe(path: &Path) -> Probe {
    let header = || -> Result<(u32, u32), String> {
        let mut decoder = reader(path)?.into_decoder().map_err(|e| e.to_string())?;
        let (w, h) = decoder.dimensions();
        let turned = matches!(
            decoder.orientation().unwrap_or(Orientation::NoTransforms),
            Orientation::Rotate90
                | Orientation::Rotate270
                | Orientation::Rotate90FlipH
                | Orientation::Rotate270FlipH
        );
        Ok(if turned { (h, w) } else { (w, h) })
    };
    match header() {
        Ok((w, h)) if w > 0 && h > 0 => Probe::Image {
            w,
            h,
            mtime: mtime(path),
        },
        Ok(_) => Probe::Failed("the image is empty".into()),
        Err(e) => Probe::Failed(e),
    }
}

fn mtime(path: &Path) -> Option<u64> {
    let modified = fs::metadata(path).and_then(|m| m.modified()).ok()?;
    let since = modified.duration_since(UNIX_EPOCH).ok()?;
    Some(since.as_millis() as u64)
}

fn reader(path: &Path) -> Result<ImageReader<std::io::BufReader<fs::File>>, String> {
    ImageReader::open(path)
        .and_then(ImageReader::with_guessed_format)
        .map_err(|e| e.to_string())
}

/// The cells an image of `px` pixels takes: its natural size at the font's `cell` size, scaled
/// down to fit `max` cells if it must, keeping its aspect ratio.
pub fn fit(px: (u32, u32), cell: Size, max: Size) -> Size {
    let cols = f64::from(px.0) / f64::from(cell.width.max(1));
    let rows = f64::from(px.1) / f64::from(cell.height.max(1));
    let scale = (f64::from(max.width) / cols)
        .min(f64::from(max.height) / rows)
        .min(1.0);
    // a hair under, so that an exact fit doesn't round up past it
    let cells = |n: f64, max: u16| ((n * scale - 1e-9).ceil() as u16).clamp(1, max.max(1));
    Size::new(cells(cols, max.width), cells(rows, max.height))
}

/// An encoded image: the cache key, and what a decode job produces.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ImageKey {
    pub path: PathBuf,
    pub mtime: Option<u64>,
    /// Columns × rows.
    pub size: Size,
}

/// An image encoded for the terminal, ready to paint.
#[derive(Clone)]
pub struct Encoded(Arc<Image>);

enum Image {
    /// Sent to the terminal once, by the first frame that paints it; its cells are placeholders.
    Kitty {
        size: Size,
        transmit: Transmit,
        /// A frame has carried the transmission.
        sent: AtomicBool,
    },
    /// Row by row.
    Halfblocks { size: Size, cells: Box<[HalfBlock]> },
    /// Sixel and iTerm2, by ratatui-image.
    Sliced(SlicedProtocol),
}

/// What transmits a kitty image to the terminal. The first frame that paints the image sends it,
/// but the main screen has an image store of its own, which a frame kept on quit has to send it
/// to again.
#[derive(Debug)]
pub struct Transmit {
    /// The image's id: its placeholders' foreground color.
    pub color: u32,
    pub seq: Box<str>,
}

impl Encoded {
    pub fn size(&self) -> Size {
        match &*self.0 {
            Image::Kitty { size, .. } | Image::Halfblocks { size, .. } => *size,
            Image::Sliced(proto) => proto.size(),
        }
    }

    pub fn transmit(&self) -> Option<&Transmit> {
        match &*self.0 {
            Image::Kitty { transmit, .. } => Some(transmit),
            _ => None,
        }
    }
}

impl fmt::Debug for Encoded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let size = self.size();
        write!(f, "Encoded({}x{})", size.width, size.height)
    }
}

impl PartialEq for Encoded {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Encoded {}

/// Kitty images encoded so far, which number the next one's id.
static KITTY_IMAGES: AtomicU32 = AtomicU32::new(0);

/// Decode an image and encode it for the terminal, fitted to `key.size` cells. It's decoded
/// straight to the pixels those cells hold ([`pixels::decode`]), so there is nothing left to
/// resize. Kitty and half-blocks are drawn as print draws them.
pub fn encode(encoder: &Encoder, key: &ImageKey) -> Result<Encoded, String> {
    let font = encoder.cell;
    let (cols, rows) = (key.size.width, key.size.height);
    let image = match encoder.proto {
        Proto::Kitty => {
            let (cols, rows) = (cols.min(kitty::MAX_CELLS), rows.min(kitty::MAX_CELLS));
            let max = (
                u32::from(cols) * u32::from(font.width),
                u32::from(rows) * u32::from(font.height),
            );
            let px = match probe(&key.path) {
                Probe::Image { w, h, .. } => (w, h),
                Probe::Failed(why) => return Err(why),
            };
            let pixels = kitty::pixels(&key.path, px, max)?;
            let payload = match &pixels {
                Some(image) => kitty::Payload::Pixels(image),
                None => kitty::Payload::Png(&key.path),
            };
            let id = kitty::image_id(KITTY_IMAGES.fetch_add(1, Ordering::Relaxed), false);
            let mut seq = Vec::new();
            kitty::transmit(&mut seq, id, &payload, cols, rows).map_err(|e| e.to_string())?;
            let seq = String::from_utf8(seq).map_err(|e| e.to_string())?;
            Image::Kitty {
                size: Size::new(cols, rows),
                transmit: Transmit {
                    color: id,
                    seq: seq.into(),
                },
                sent: AtomicBool::new(false),
            }
        }
        Proto::Halfblocks => {
            let image = pixels::decode_to(&key.path, |_| (u32::from(cols), 2 * u32::from(rows)))?;
            let cells = (0..u32::from(rows))
                .flat_map(|y| (0..u32::from(cols)).map(move |x| (x, y)))
                .map(|at| HalfBlock::of(&image, (cols, rows), at))
                .collect();
            Image::Halfblocks {
                size: key.size,
                cells,
            }
        }
        Proto::Sixel | Proto::Iterm2 => {
            let picker = encoder.picker.as_ref().ok_or("no picker")?;
            let max = (
                u32::from(cols) * u32::from(font.width),
                u32::from(rows) * u32::from(font.height),
            );
            let image = pixels::decode(&key.path, max)?;
            let resize = Resize::Fit(Some(FilterType::Triangle));
            let proto = SlicedProtocol::new_with_resize(picker, image, key.size, resize)
                .map_err(|e| e.to_string())?;
            Image::Sliced(proto)
        }
    };
    Ok(Encoded(Arc::new(image)))
}

/// A decode job for the image worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub key: ImageKey,
    /// On screen: goes to the front of the queue.
    pub urgent: bool,
}

impl Job {
    pub fn run(&self, encoder: &Encoder) -> Msg {
        let ImageKey { path, mtime, size } = self.key.clone();
        Msg::ImageReady {
            path,
            mtime,
            size: app::Size {
                w: size.width,
                h: size.height,
            },
            image: Computed(Some(encode(encoder, &self.key))),
        }
    }
}

/// Start the image worker. Jobs on screen jump the queue; the rest run in order.
pub fn spawn_worker(encoder: Encoder, tx: Sender<Msg>) -> Sender<Job> {
    let (jobs_tx, jobs) = mpsc::channel::<Job>();
    thread::spawn(move || {
        let mut queue: VecDeque<Job> = VecDeque::new();
        loop {
            if queue.is_empty() {
                let Ok(job) = jobs.recv() else {
                    return;
                };
                queue.push_back(job);
            }
            for job in jobs.try_iter() {
                queue.retain(|j| j.key != job.key);
                if job.urgent {
                    queue.push_front(job);
                } else {
                    queue.push_back(job);
                }
            }
            let Some(job) = queue.pop_front() else {
                continue;
            };
            if tx.send(job.run(&encoder)).is_err() {
                return;
            }
        }
    });
    jobs_tx
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// A job is on its way.
    Pending,
    Ready(Encoded),
    Failed(String),
}

/// Encoded images, by key. The least recently requested go first once it holds `CACHE_CAP`.
#[derive(Debug, Default)]
pub struct ImageCache {
    map: HashMap<ImageKey, Entry>,
    /// Least recently requested first.
    order: VecDeque<ImageKey>,
}

impl ImageCache {
    pub fn get(&self, key: &ImageKey) -> Option<&Entry> {
        self.map.get(key)
    }

    /// The kitty images' transmissions.
    pub fn transmits(&self) -> impl Iterator<Item = &Transmit> {
        self.map.values().filter_map(|e| match e {
            Entry::Ready(image) => image.transmit(),
            _ => None,
        })
    }

    /// Whether `key` needs a decode job, marking it pending if so. Images on screen are
    /// requested again as the view changes, which keeps them from being dropped.
    pub fn request(&mut self, key: &ImageKey) -> bool {
        if self.map.contains_key(key) {
            if let Some(i) = self.order.iter().position(|k| k == key)
                && let Some(k) = self.order.remove(i)
            {
                self.order.push_back(k);
            }
            return false;
        }
        self.put(key.clone(), Entry::Pending);
        true
    }

    pub fn insert(&mut self, key: ImageKey, result: Result<Encoded, String>) {
        let entry = match result {
            Ok(image) => Entry::Ready(image),
            Err(e) => Entry::Failed(e),
        };
        self.put(key, entry);
    }

    fn put(&mut self, key: ImageKey, entry: Entry) {
        if self.map.insert(key.clone(), entry).is_none() {
            self.order.push_back(key);
        }
        while self.order.len() > CACHE_CAP {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }
}

/// Paint `image` with its top-left corner at column `x` and row `y` of `area`. Rows outside
/// `area` are clipped, so `y` may be negative.
pub fn render(image: &Encoded, grid: &mut Grid, area: Rect, x: u16, y: i32) {
    if x >= area.width {
        return;
    }
    let size = image.size();
    // the image's rows and columns inside the area, and its cell (0, 0) on the grid
    let rows = (-y).clamp(0, i32::from(size.height))
        ..(i32::from(area.height) - y).clamp(0, i32::from(size.height));
    let (rows, cols) = (
        rows.start as u16..rows.end as u16,
        size.width.min(area.width - x),
    );
    let at = |row: u16, col: u16| {
        let y = i32::from(area.y) + y + i32::from(row);
        (area.x + x + col, y as u16)
    };
    match &*image.0 {
        Image::Kitty { transmit, sent, .. } => {
            let [_, r, g, b] = transmit.color.to_be_bytes();
            let color = Color::Rgb(r, g, b);
            for row in rows.clone() {
                for col in 0..cols {
                    grid.placeholder(at(row, col), (row, col), color);
                }
            }
            // the first cell painted carries the transmission, the first time
            if !rows.is_empty() && cols > 0 && !sent.swap(true, Ordering::Relaxed) {
                let mut first = String::from(&*transmit.seq);
                first.extend(kitty::placeholder(rows.start, 0));
                grid.image_cell(at(rows.start, 0), &first, color);
            }
        }
        Image::Halfblocks { size, cells } => {
            let rgb = |c: Option<pixels::Rgb>| c.map(|(r, g, b)| Color::Rgb(r, g, b));
            for row in rows.clone() {
                let line = &cells[usize::from(row) * usize::from(size.width)..];
                for (col, cell) in line.iter().take(usize::from(cols)).enumerate() {
                    let (text, fg, bg) = cell.cell();
                    let (x, y) = at(row, col as u16);
                    let style = Style {
                        fg: rgb(fg),
                        bg: rgb(bg),
                        ..Style::new()
                    };
                    grid.set_stringn(x, y, text, 1, style);
                }
            }
        }
        Image::Sliced(proto) => {
            let y = y.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
            let position = SignedPosition::from((x as i16, y));
            let top = area.y.saturating_add(y.max(0) as u16);
            let covered = Rect::new(area.x + x, top, size.width, size.height).intersection(area);
            grid.lend(covered, |buf| {
                SlicedImage::new(proto, position).render(area, buf);
            });
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::grid::Screen;

    /// A `w`×`h` PNG in a fresh temporary directory.
    pub(crate) fn png(name: &str, w: u32, h: u32) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ada-images-{}-{name}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.png"));
        let img =
            image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x * 7) as u8, (y * 5) as u8, 90]));
        img.save(&path).unwrap();
        path
    }

    /// An environment of `vars`.
    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|&(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        move |name| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
    }

    #[test]
    fn inside_tmux_half_blocks_without_asking() {
        let auto = ImageMode::Auto;
        let ask = Plan {
            ask: true,
            forced: None,
        };
        assert_eq!(plan(auto, env(&[("TERM", "xterm-kitty")])), Some(ask));
        assert_eq!(plan(auto, env(&[("TMUX", "")])), Some(ask));
        let half = Some(Plan {
            ask: false,
            forced: Some(Proto::Halfblocks),
        });
        assert_eq!(plan(auto, env(&[("TMUX", "/tmp/tmux-1000/d,1,0")])), half);
        assert_eq!(plan(auto, env(&[("TERM", "tmux-256color")])), half);
        assert_eq!(plan(auto, env(&[("TERM_PROGRAM", "tmux")])), half);
        // forced: asked for the cell size, but not inside tmux
        let force = ImageMode::Force(Proto::Sixel);
        assert_eq!(
            plan(force, env(&[])),
            Some(Plan {
                ask: true,
                forced: Some(Proto::Sixel)
            })
        );
        assert_eq!(
            plan(force, env(&[("TMUX", "x")])),
            Some(Plan {
                ask: false,
                forced: Some(Proto::Sixel)
            })
        );
        assert_eq!(plan(ImageMode::Off, env(&[])), None);
    }

    #[test]
    fn deciding_from_the_replies() {
        let ask = Plan {
            ask: true,
            forced: None,
        };
        let cells = Size::new(100, 40);
        let found = |kitty, attributes: &[u16], cell, text_area| probe::Found {
            kitty,
            attributes: Some(attributes.to_vec()),
            cell,
            text_area,
        };
        let decide = |found: &probe::Found, vars: &[(&str, &str)], winsize| {
            let g = decide(ask, found, env(vars), cells, winsize);
            (g.proto, (g.cell.width, g.cell.height))
        };
        let kitty = found(Some(true), &[62, 4], Some((9, 18)), None);
        let sixel = found(None, &[65, 4], None, Some((1000, 800)));
        let neither = found(Some(false), &[62], Some((8, 16)), None);
        // kitty over sixel; the cell by 16t, else 14t over the cells, else TIOCGWINSZ
        assert_eq!(decide(&kitty, &[], None), (Proto::Kitty, (9, 18)));
        assert_eq!(decide(&sixel, &[], None), (Proto::Sixel, (10, 20)));
        let ioctl = Some(Size::new(7, 14));
        assert_eq!(
            decide(&found(None, &[4], None, None), &[], ioctl),
            (Proto::Sixel, (7, 14))
        );
        assert_eq!(decide(&neither, &[], None), (Proto::Halfblocks, (8, 16)));
        // no pixel size anywhere: half-blocks at 10×20, whatever was found
        assert_eq!(
            decide(&found(Some(true), &[4], None, None), &[], None),
            (Proto::Halfblocks, (10, 20))
        );
        // iTerm2 from the environment, after what the terminal said
        let wez = [("TERM_PROGRAM", "WezTerm")];
        assert_eq!(decide(&neither, &wez, None), (Proto::Iterm2, (8, 16)));
        assert_eq!(
            decide(&kitty, &[("LC_TERMINAL", "iTerm2")], None).0,
            Proto::Kitty
        );
        // WezTerm and Konsole: kitty and sixel aren't taken from them
        let wez = [("WEZTERM_EXECUTABLE", "/usr/bin/wezterm-gui"), wez[0]];
        assert_eq!(decide(&kitty, &wez, None).0, Proto::Iterm2);
        let konsole = [("KONSOLE_VERSION", "260802")];
        assert_eq!(decide(&kitty, &konsole, None).0, Proto::Halfblocks);
        assert_eq!(decide(&sixel, &konsole, None).0, Proto::Halfblocks);
        // no answer at all, as after the deadline
        assert_eq!(
            decide(&probe::Found::default(), &[], ioctl),
            (Proto::Halfblocks, (7, 14))
        );
        // forced: the protocol as given, the cell as found
        let forced = Plan {
            ask: true,
            forced: Some(Proto::Iterm2),
        };
        let g = super::decide(forced, &kitty, env(&[]), cells, None);
        assert_eq!((g.proto, g.cell), (Proto::Iterm2, Size::new(9, 18)));
    }

    #[test]
    fn printed_images_take_their_protocol_from_the_environment() {
        let print = |mode, vars: &[(&str, &str)]| print_proto(mode, env(vars));
        let auto = ImageMode::Auto;
        assert_eq!(print(auto, &[("KITTY_WINDOW_ID", "3")]), Some(Proto::Kitty));
        assert_eq!(print(auto, &[("TERM", "xterm-kitty")]), Some(Proto::Kitty));
        assert_eq!(
            print(auto, &[("TERM", "xterm-ghostty")]),
            Some(Proto::Kitty)
        );
        assert_eq!(
            print(auto, &[("TERM", "xterm-256color")]),
            Some(Proto::Halfblocks)
        );
        let tmux = [
            ("KITTY_WINDOW_ID", "3"),
            ("TMUX", "/tmp/tmux-1000/default,1,0"),
        ];
        assert_eq!(print(auto, &tmux), Some(Proto::Halfblocks));
        assert_eq!(
            print(auto, &[("TMUX", ""), ("TERM", "xterm-kitty")]),
            Some(Proto::Kitty)
        );
        // forced, but print has only these two
        let force = ImageMode::Force;
        assert_eq!(print(force(Proto::Kitty), &tmux), Some(Proto::Kitty));
        assert_eq!(print(force(Proto::Sixel), &[]), Some(Proto::Halfblocks));
        assert_eq!(print(ImageMode::Off, &[("TERM", "xterm-kitty")]), None);
    }

    #[test]
    fn protocol_names_round_trip() {
        for p in Proto::ALL {
            assert_eq!(Proto::parse(p.name()), Some(p));
        }
        assert_eq!(Proto::parse("chafa"), None);
    }

    #[test]
    fn fitting() {
        let cell = Size::new(10, 20);
        let roomy = Size::new(200, 100);
        // natural size, rounded up to whole cells
        assert_eq!(fit((400, 300), cell, roomy), Size::new(40, 15));
        assert_eq!(fit((401, 301), cell, roomy), Size::new(41, 16));
        // too wide: scaled to the width, keeping the aspect ratio
        assert_eq!(
            fit((4000, 1000), cell, Size::new(100, 100)),
            Size::new(100, 13)
        );
        // too tall: scaled to the height
        assert_eq!(fit((300, 4000), cell, Size::new(100, 20)), Size::new(3, 20));
        // never less than a cell
        assert_eq!(fit((1, 1), cell, roomy), Size::new(1, 1));
        assert_eq!(fit((5000, 5000), cell, Size::new(0, 0)), Size::new(1, 1));
    }

    #[test]
    fn probe_decode_and_paint() {
        let path = png("probe", 40, 60);
        assert_eq!(
            probe(&path),
            Probe::Image {
                w: 40,
                h: 60,
                mtime: mtime(&path)
            }
        );
        let Probe::Failed(why) = probe(&path.with_extension("gone")) else {
            panic!("probed a missing file");
        };
        assert!(!why.is_empty());
        let text = path.with_extension("txt");
        fs::write(&text, "not an image").unwrap();
        assert!(matches!(probe(&text), Probe::Failed(_)));

        let encoder = Encoder::halfblocks();
        let key = ImageKey {
            path: path.clone(),
            mtime: None,
            size: fit((40, 60), Size::new(10, 20), Size::new(80, 10)),
        };
        let Msg::ImageReady {
            image: Computed(Some(Ok(image))),
            ..
        } = (Job { key, urgent: true }).run(&encoder)
        else {
            panic!("no image");
        };
        assert_eq!(image.size(), Size::new(4, 3));

        // clipped at the top: only the bottom row lands in the area
        let area = Rect::new(0, 0, 10, 4);
        let mut grid = Grid::empty(area);
        render(&image, &mut grid, area, 2, -2);
        let painted = |y| (0..10).filter(|&x| grid.symbol((x, y)) != " ").count();
        assert_eq!((painted(0), painted(1)), (4, 0));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    /// The next frame `paint` draws, as written to the terminal.
    fn frame(screen: &mut Screen<Shared>, out: &Shared, paint: impl FnOnce(&mut Grid)) -> String {
        screen.draw(paint).unwrap();
        String::from_utf8(std::mem::take(&mut *out.0.borrow_mut())).unwrap()
    }

    #[derive(Clone, Default)]
    struct Shared(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

    impl std::io::Write for Shared {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn placeholder(row: u16, col: u16) -> String {
        kitty::placeholder(row, col).iter().collect()
    }

    /// A kitty image is sent once, by the first frame that paints it, as print sends it: a PNG
    /// that fits goes as the file. Its cells are placeholders in the color that names it, numbered
    /// where kitty can't tell their row and column from the cell to the left, and clipped with
    /// their rows.
    #[test]
    fn kitty_images_are_sent_once_and_drawn_by_placeholders() {
        let path = png("kitty", 30, 40);
        let encoder = Encoder::new(Proto::Kitty, Size::new(10, 20));
        let key = ImageKey {
            path: path.clone(),
            mtime: None,
            size: Size::new(3, 2),
        };
        let image = encode(&encoder, &key).unwrap();
        let transmit = image.transmit().expect("kitty");
        let params = transmit
            .seq
            .strip_prefix("\x1b_G")
            .unwrap()
            .split(';')
            .next()
            .unwrap();
        for p in ["a=T", "U=1", "q=2", "c=3", "r=2", "f=100"] {
            assert!(params.split(',').any(|kv| kv == p), "{p} in {params}");
        }
        assert!(params.contains(&format!("i={}", transmit.color)));
        let [_, r, g, b] = transmit.color.to_be_bytes();
        let color = format!("\x1b[38;2;{r};{g};{b}m");

        let (p, (r0, r1)) = ('\u{10EEEE}', (placeholder(0, 0), placeholder(1, 0)));
        let area = Rect::new(0, 0, 6, 2);
        let out = Shared::default();
        let mut screen = Screen::new(out.clone(), Size::new(6, 2));
        // the first frame: row 1 clipped off the bottom; the transmission in the first cell, which
        // moves the cursor its own way, then the next placed explicitly and numbered
        let first = frame(&mut screen, &out, |g| render(&image, g, area, 1, 1));
        assert_eq!(
            first.matches("\x1b_G").count(),
            transmit.seq.matches("\x1b_G").count()
        );
        let cells = format!("{}{r0}\x1b[2;3H{}{p}", transmit.seq, placeholder(0, 1));
        assert!(
            first.contains(&format!("\x1b[2;2H{color}{cells}\x1b[0m")),
            "{first:?}"
        );
        // scrolled up a row: row 0 clipped off the top, row 1 moves up, and nothing is sent again
        let next = frame(&mut screen, &out, |g| render(&image, g, area, 1, -1));
        assert!(!next.contains("\x1b_G"), "{next:?}");
        assert!(
            next.contains(&format!("\x1b[1;2H{color}{r1}{p}{p}")),
            "{next:?}"
        );
        assert_eq!(cells_of(&next), 3);
        assert!(image.clone().transmit().is_some());
        let mut cache = ImageCache::default();
        cache.insert(key, Ok(image));
        assert_eq!(cache.transmits().count(), 1);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    fn cells_of(frame: &str) -> usize {
        frame.matches('\u{10EEEE}').count()
    }

    #[test]
    fn the_cache_drops_the_least_recently_requested() {
        let key = |n: u16| ImageKey {
            path: "a.png".into(),
            mtime: None,
            size: Size::new(n, 1),
        };
        let mut cache = ImageCache::default();
        assert!(cache.request(&key(0)));
        assert!(!cache.request(&key(0)));
        cache.insert(key(0), Err("bad".into()));
        assert_eq!(cache.get(&key(0)), Some(&Entry::Failed("bad".into())));
        for n in 1..CACHE_CAP as u16 {
            cache.request(&key(n));
        }
        // asked for again, so 1 is now the least recent
        assert!(!cache.request(&key(0)));
        cache.request(&key(CACHE_CAP as u16));
        assert_eq!(cache.get(&key(1)), None);
        assert_eq!(cache.get(&key(0)), Some(&Entry::Failed("bad".into())));
        assert_eq!(cache.get(&key(2)), Some(&Entry::Pending));
    }
}
