//! Images (ADR-09, architecture §9). Everything that knows about ratatui-image lives here.
//!
//! The terminal is queried once at startup; its protocol and font cell size reach `update` as
//! `Msg::Graphics`. Reading an image's header touches the filesystem, so it is a world effect
//! (`Cmd::ProbeImages`) whose result is recorded. Decoding and encoding run on a worker
//! (`Cmd::DecodeImage`) and are cached by `(path, mtime, cells)`. Hermetic replay has no image
//! bytes to recompute them from, so replayed images stay placeholders.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::UNIX_EPOCH;

use image::imageops::FilterType;
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader};
use ratatui::buffer::Buffer;
use ratatui::layout::{Rect, Size};
use ratatui::widgets::Widget;
use ratatui_image::Resize;
use ratatui_image::picker::{Capability, Picker, ProtocolType};
use ratatui_image::sliced::{SignedPosition, SlicedImage, SlicedProtocol};

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

    fn of(t: ProtocolType) -> Proto {
        match t {
            ProtocolType::Kitty => Proto::Kitty,
            ProtocolType::Sixel => Proto::Sixel,
            ProtocolType::Iterm2 => Proto::Iterm2,
            ProtocolType::Halfblocks => Proto::Halfblocks,
        }
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

/// Query the terminal for its graphics protocol and font size. Call it in raw mode, before
/// anything else reads stdin. `None` when images are off.
pub fn detect(mode: ImageMode) -> Option<Picker> {
    let forced = match mode {
        ImageMode::Off => return None,
        ImageMode::Auto => None,
        ImageMode::Force(p) => Some(p),
    };
    let mut picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
    let tmux = picker.tmux_detected() || std::env::var_os("TMUX").is_some_and(|v| !v.is_empty());
    let sixel = picker.capabilities().contains(&Capability::Sixel);
    let proto = forced.unwrap_or_else(|| choose(Proto::of(picker.protocol_type()), tmux, sixel));
    picker.set_protocol_type(proto.protocol_type());
    Some(picker)
}

/// The protocol to use, given what the query found. Kitty graphics don't work inside tmux, and
/// won't (plan Q10): there, sixel if tmux passes it through, otherwise half-blocks.
pub fn choose(found: Proto, tmux: bool, sixel: bool) -> Proto {
    match found {
        Proto::Kitty if tmux && sixel => Proto::Sixel,
        Proto::Kitty if tmux => Proto::Halfblocks,
        p => p,
    }
}

/// The message that tells `update` what the picker found.
pub fn graphics_msg(picker: &Picker) -> Msg {
    let font = picker.font_size();
    Msg::Graphics {
        proto: Proto::of(picker.protocol_type()),
        cell: app::Size {
            w: font.width,
            h: font.height,
        },
    }
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

fn decode(path: &Path) -> Result<DynamicImage, String> {
    let mut decoder = reader(path)?.into_decoder().map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    image.apply_orientation(orientation);
    Ok(image)
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
pub struct Encoded(Arc<SlicedProtocol>);

impl Encoded {
    pub fn size(&self) -> Size {
        self.0.size()
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

/// Decode an image and encode it for the terminal, fitted to `key.size` cells.
pub fn encode(picker: &Picker, key: &ImageKey) -> Result<Encoded, String> {
    let image = decode(&key.path)?;
    let resize = Resize::Fit(Some(FilterType::Triangle));
    SlicedProtocol::new_with_resize(picker, image, key.size, resize)
        .map(|p| Encoded(Arc::new(p)))
        .map_err(|e| e.to_string())
}

/// A decode job for the image worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub key: ImageKey,
    /// On screen: goes to the front of the queue.
    pub urgent: bool,
}

impl Job {
    pub fn run(&self, picker: &Picker) -> Msg {
        let ImageKey { path, mtime, size } = self.key.clone();
        Msg::ImageReady {
            path,
            mtime,
            size: app::Size {
                w: size.width,
                h: size.height,
            },
            image: Computed(Some(encode(picker, &self.key))),
        }
    }
}

/// Start the image worker. Jobs on screen jump the queue; the rest run in order.
pub fn spawn_worker(picker: Picker, tx: Sender<Msg>) -> Sender<Job> {
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
            if tx.send(job.run(&picker)).is_err() {
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
pub fn render(image: &Encoded, buf: &mut Buffer, area: Rect, x: u16, y: i32) {
    if x >= area.width {
        return;
    }
    let y = y.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
    let position = SignedPosition::from((x as i16, y));
    SlicedImage::new(&image.0, position).render(area, buf);
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

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

    #[test]
    fn kitty_is_never_picked_inside_tmux() {
        assert_eq!(choose(Proto::Kitty, false, true), Proto::Kitty);
        assert_eq!(choose(Proto::Kitty, true, true), Proto::Sixel);
        assert_eq!(choose(Proto::Kitty, true, false), Proto::Halfblocks);
        assert_eq!(choose(Proto::Sixel, true, true), Proto::Sixel);
        assert_eq!(choose(Proto::Halfblocks, true, false), Proto::Halfblocks);
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

        let mut picker = Picker::halfblocks();
        picker.set_protocol_type(ProtocolType::Halfblocks);
        let key = ImageKey {
            path: path.clone(),
            mtime: None,
            size: fit((40, 60), picker.font_size().into_size(), Size::new(80, 10)),
        };
        let Msg::ImageReady {
            image: Computed(Some(Ok(image))),
            ..
        } = (Job { key, urgent: true }).run(&picker)
        else {
            panic!("no image");
        };
        assert_eq!(image.size(), Size::new(4, 3));

        // clipped at the top: only the bottom row lands in the area
        let area = Rect::new(0, 0, 10, 4);
        let mut buf = Buffer::empty(area);
        render(&image, &mut buf, area, 2, -2);
        let painted = |y| (0..10).filter(|&x| buf[(x, y)].symbol() != " ").count();
        assert_eq!((painted(0), painted(1)), (4, 0));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    trait IntoSize {
        fn into_size(self) -> Size;
    }

    impl IntoSize for ratatui_image::FontSize {
        fn into_size(self) -> Size {
            Size::new(self.width, self.height)
        }
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
