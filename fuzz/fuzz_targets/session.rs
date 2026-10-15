//! Fuzz round 2 (plan P7): a whole session, scripted by the input. The input is a document
//! (`[len_lo, len_hi, ..len bytes]`), then bytes `arbitrary` turns into a terminal size, config,
//! and up to 64 intents. Commands run against a stub world: links resolve to fuzz-chosen files
//! in a scratch directory, git answers with fuzzed hunks, image probes report fuzzed sizes, and
//! every result waits in a queue until the script delivers it, so results can arrive late, out
//! of order, or after the document they were for is gone. Every step is painted. Nothing may
//! panic.
//!
//! ```zsh
//! cargo +nightly fuzz run -s none -a session -- -dict=fuzz/markdown.dict -max_len=16384
//! ```

#![no_main]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use ada::app::{self, Cmd, Computed, Config, Dir, Effects, Msg, ScrollBy, Size, State};
use ada::images::{Probe, Proto};
use ada::layout::TableMode;
use ada::links::Resolved;
use ada::theme::Theme;
use libfuzzer_sys::arbitrary::{self, Arbitrary, Result, Unstructured};
use libfuzzer_sys::fuzz_target;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

const MAX_OPS: usize = 64;

#[derive(Arbitrary, Debug)]
enum Op {
    Resize {
        w: u16,
        h: u8,
    },
    Graphics {
        proto: u8,
        w: u8,
        h: u8,
    },
    Rows(i16),
    HalfPage(bool),
    Page(bool),
    Top,
    Bottom,
    JumpHeading(bool),
    ToggleTableMode,
    ToggleCodeWrap,
    ScrollCodeX(i16),
    FocusLink(bool),
    Follow,
    Back,
    Forward,
    OpenOutline,
    OutlineFilter(Short),
    OutlineMove(i8),
    OutlinePick,
    OutlineClose,
    SearchOpen,
    SearchInput(Short),
    SearchSubmit,
    SearchCancel,
    SearchNext(bool),
    JumpChange(bool),
    Reload,
    CycleTheme,
    Help,
    HelpScroll(i16),
    HelpClose,
    Edit,
    Yank,
    Open(u8),
    FileChanged {
        other: bool,
        mtime: Option<u8>,
    },
    /// Deliver the pending result at this index (mod the queue's length).
    Deliver(u8),
    DeliverAll,
    /// Drop the oldest pending result, as if the worker lost it.
    Drop,
}

/// A string of at most 24 chars, so search patterns and filters stay cheap.
#[derive(Debug)]
struct Short(String);

impl<'a> Arbitrary<'a> for Short {
    fn arbitrary(u: &mut Unstructured<'a>) -> Result<Self> {
        let s: &str = u.arbitrary()?;
        Ok(Short(s.chars().take(24).collect()))
    }
}

/// Files in the scratch directory that links can resolve to: notes, images, a corrupt image,
/// and paths that don't exist.
struct World {
    dir: PathBuf,
    files: Vec<PathBuf>,
}

fn world() -> &'static World {
    static WORLD: OnceLock<World> = OnceLock::new();
    WORLD.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("ada-fuzz-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("scratch dir");
        let write = |name: &str, bytes: &[u8]| {
            let path = dir.join(name);
            fs::write(&path, bytes).expect("scratch file");
            path
        };
        let png = |w, h| {
            let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([x as u8, y as u8, 99]));
            let mut out = std::io::Cursor::new(Vec::new());
            img.write_to(&mut out, image::ImageFormat::Png)
                .expect("png");
            out.into_inner()
        };
        let jpeg = {
            let img = image::RgbImage::from_fn(40, 3, |x, _| image::Rgb([x as u8, 0, 0]));
            let mut out = std::io::Cursor::new(Vec::new());
            img.write_to(&mut out, image::ImageFormat::Jpeg)
                .expect("jpeg");
            out.into_inner()
        };
        let files = vec![
            write("a.md", b"# A\n\nback to [fuzz](fuzz.md) and [[b]]\n"),
            write("b.md", b"## B\n\n![i](img.png)\n\n| x |\n|---|\n| y |\n"),
            write("img.png", &png(3, 2)),
            write("tall.png", &png(1, 64)),
            write("wide.jpg", &jpeg),
            write("broken.png", b"\x89PNG\r\n\x1a\nnot really"),
            dir.join("missing.png"),
            dir.join("missing.md"),
            dir.join("fuzz.md"),
        ];
        World { dir, files }
    })
}

fn theme() -> Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(Theme::builtin).clone()
}

fn size(w: u16, h: u8) -> Size {
    Size {
        w: w % 400 + 1,
        h: u16::from(h % 96) + 1,
    }
}

/// Unified diff text: mostly `-U0` hunk headers, sometimes junk. Line numbers can be extreme;
/// counts stay within what a real file could produce, as each counted line becomes a mark.
fn diff(u: &mut Unstructured) -> Result<Arc<str>> {
    let mut out = String::new();
    for _ in 0..u.int_in_range(0..=6)? {
        let start = |u: &mut Unstructured| -> Result<u32> {
            Ok(match u.int_in_range(0..=7)? {
                0 => u32::MAX - u32::from(u.arbitrary::<u8>()?),
                1 => 0,
                _ => u32::from(u.arbitrary::<u16>()?),
            })
        };
        let count = |u: &mut Unstructured| -> Result<u32> { Ok(u32::from(u.arbitrary::<u8>()?)) };
        let (a, b, c, d) = (start(u)?, count(u)?, start(u)?, count(u)?);
        match u.int_in_range(0..=5)? {
            0 => out.push_str(&format!("@@ -{a} +{c} @@\n")),
            1 => out.push_str(&format!("@@ -{a},{b} +{c} @@ ctx\n")),
            2 => out.push_str(&u.arbitrary::<Short>()?.0),
            _ => out.push_str(&format!("@@ -{a},{b} +{c},{d} @@\n")),
        }
    }
    Ok(out.into())
}

/// The stub world's answer to a command, if it has one.
fn answer(
    cmd: Cmd,
    u: &mut Unstructured,
    doc: &Arc<str>,
    effects: &mut Effects,
) -> Result<Option<Msg>> {
    let world = world();
    Ok(match cmd {
        Cmd::LoadDoc(path) => Some(match u.int_in_range(0..=7)? {
            0 => Msg::DocFailed {
                path,
                error: "fuzz".into(),
            },
            1 => {
                let text = fs::read_to_string(&path).unwrap_or_default();
                Msg::DocLoaded {
                    path,
                    mtime: u.arbitrary()?,
                    text: text.into(),
                }
            }
            2 => {
                let cut = u.int_in_range(0..=doc.len())?;
                let cut = (0..=cut)
                    .rev()
                    .find(|&i| doc.is_char_boundary(i))
                    .unwrap_or(0);
                Msg::DocLoaded {
                    path,
                    mtime: u.arbitrary()?,
                    text: doc[cut..].into(),
                }
            }
            _ => Msg::DocLoaded {
                path,
                mtime: u.arbitrary()?,
                text: doc.clone(),
            },
        }),
        Cmd::Highlight(job) => Some(job.run()),
        Cmd::ResolveLinks {
            doc_gen, targets, ..
        } => {
            let mut resolved = Vec::with_capacity(targets.len());
            for _ in &targets {
                resolved.push(match u.int_in_range(0..=world.files.len())? {
                    0 => Resolved::Broken("fuzz".into()),
                    i => Resolved::Found(world.files[i - 1].clone()),
                });
            }
            // now and then, the wrong number of answers
            match u.int_in_range(0..=15)? {
                0 => {
                    resolved.pop();
                }
                1 => resolved.push(Resolved::Found(world.files[2].clone())),
                _ => {}
            }
            Some(Msg::LinksResolved { doc_gen, resolved })
        }
        Cmd::OpenWithSystem(target) => u.arbitrary::<bool>()?.then(|| Msg::OpenFailed {
            target: target.to_string_lossy().into_owned(),
            error: "fuzz".into(),
        }),
        Cmd::RunGit { doc_gen, .. } => Some(Msg::GitDiff {
            doc_gen,
            untracked: u.ratio(1, 8)?,
            total: diff(u)?,
            unstaged: diff(u)?,
        }),
        Cmd::ProbeImages { doc_gen, paths } => {
            let mut probes = Vec::with_capacity(paths.len());
            for path in &paths {
                probes.push(match u.int_in_range(0..=5)? {
                    0 => Probe::Failed("fuzz".into()),
                    1 => Probe::Image {
                        w: u.arbitrary()?,
                        h: u.arbitrary()?,
                        mtime: u.arbitrary()?,
                    },
                    2 => Probe::Image {
                        w: u32::from(u.arbitrary::<u16>()?),
                        h: u32::from(u.arbitrary::<u8>()?),
                        mtime: None,
                    },
                    _ => ada::images::probe(path),
                });
            }
            Some(Msg::ImagesProbed { doc_gen, probes })
        }
        Cmd::DecodeImage(job) => {
            // real decodes only for the small files; others fail as the worker would report
            let key = &job.key;
            let cells = u32::from(key.size.width) * u32::from(key.size.height);
            if key.path.starts_with(&world.dir) && cells <= 4096 && u.arbitrary()? {
                effects.run(Cmd::DecodeImage(job))
            } else {
                Some(Msg::ImageReady {
                    path: key.path.clone(),
                    mtime: key.mtime,
                    size: Size {
                        w: key.size.width,
                        h: key.size.height,
                    },
                    image: Computed(Some(Err("fuzz".into()))),
                })
            }
        }
        Cmd::SpawnEditor { .. } => Some(Msg::EditorExited {
            status: i32::from(u.arbitrary::<i8>()?),
        }),
        Cmd::Osc52(text) => {
            let _ = app::osc52(&text, u.arbitrary()?);
            None
        }
        Cmd::Watch { .. } | Cmd::Quit => None,
    })
}

struct Session<'a> {
    state: State,
    effects: Effects,
    pending: Vec<Msg>,
    doc: Arc<str>,
    terminal: Terminal<TestBackend>,
    u: Unstructured<'a>,
}

impl Session<'_> {
    /// Apply `msg`; queue the stub world's answers to its commands.
    fn send(&mut self, msg: Msg) {
        for cmd in app::update(&mut self.state, msg) {
            match answer(cmd, &mut self.u, &self.doc, &mut self.effects) {
                Ok(Some(msg)) => self.pending.push(msg),
                Ok(None) => {}
                // out of input: the world goes quiet
                Err(_) => {}
            }
        }
        self.paint();
    }

    /// Paint into the frame's buffer and throw it away: diffing against the last frame and
    /// flushing to the backend is ratatui's work, not ada's, and was 40% of the time.
    fn paint(&mut self) {
        let Size { w, h } = self.state.size;
        let area = self.terminal.backend().buffer().area;
        if (area.width, area.height) != (w, h) {
            self.terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
        }
        let mut frame = self.terminal.get_frame();
        ada::ui::view(&self.state, &mut frame);
        self.terminal.current_buffer_mut().reset();
    }

    fn deliver(&mut self, i: usize) {
        if !self.pending.is_empty() {
            let msg = self.pending.remove(i % self.pending.len());
            self.send(msg);
        }
    }

    /// Deliver everything, including what the deliveries ask for, a bounded number of times.
    fn settle(&mut self) {
        for _ in 0..256 {
            if self.pending.is_empty() {
                break;
            }
            self.deliver(0);
        }
    }

    fn intent(&mut self, op: Op) -> Option<Msg> {
        let dir = |next| if next { Dir::Next } else { Dir::Prev };
        Some(match op {
            Op::Resize { w, h } => Msg::Resize { size: size(w, h) },
            Op::Graphics { proto, w, h } => Msg::Graphics {
                proto: Proto::ALL[usize::from(proto) % Proto::ALL.len()],
                cell: Size {
                    w: u16::from(w),
                    h: u16::from(h),
                },
            },
            Op::Rows(n) => Msg::Scroll {
                by: ScrollBy::Rows(i32::from(n)),
            },
            Op::HalfPage(next) => Msg::Scroll {
                by: ScrollBy::HalfPage(dir(next)),
            },
            Op::Page(next) => Msg::Scroll {
                by: ScrollBy::Page(dir(next)),
            },
            Op::Top => Msg::Scroll { by: ScrollBy::Top },
            Op::Bottom => Msg::Scroll {
                by: ScrollBy::Bottom,
            },
            Op::JumpHeading(next) => Msg::JumpHeading { dir: dir(next) },
            Op::ToggleTableMode => Msg::ToggleTableMode,
            Op::ToggleCodeWrap => Msg::ToggleCodeWrap,
            Op::ScrollCodeX(by) => Msg::ScrollCodeX { by },
            Op::FocusLink(next) => Msg::FocusLink { dir: dir(next) },
            Op::Follow => Msg::Follow,
            Op::Back => Msg::Back,
            Op::Forward => Msg::Forward,
            Op::OpenOutline => Msg::OpenOutline,
            Op::OutlineFilter(text) => Msg::OutlineFilter { text: text.0 },
            Op::OutlineMove(by) => Msg::OutlineMove { by: i32::from(by) },
            Op::OutlinePick => Msg::OutlinePick,
            Op::OutlineClose => Msg::OutlineClose,
            Op::SearchOpen => Msg::SearchOpen,
            Op::SearchInput(pattern) => Msg::SearchInput { pattern: pattern.0 },
            Op::SearchSubmit => Msg::SearchSubmit,
            Op::SearchCancel => Msg::SearchCancel,
            Op::SearchNext(next) => Msg::SearchNext { dir: dir(next) },
            Op::JumpChange(next) => Msg::JumpChange { dir: dir(next) },
            Op::Reload => Msg::Reload,
            Op::CycleTheme => Msg::CycleTheme,
            Op::Help => Msg::Help,
            Op::HelpScroll(by) => Msg::HelpScroll { by: i32::from(by) },
            Op::HelpClose => Msg::HelpClose,
            Op::Edit => Msg::Edit,
            Op::Yank => Msg::Yank,
            Op::Open(i) => {
                let files = &world().files;
                Msg::Open {
                    path: files[usize::from(i) % files.len()].clone(),
                }
            }
            Op::FileChanged { other, mtime } => {
                let path = match (&self.state.doc, other) {
                    (Some(doc), false) => doc.path.clone(),
                    _ => world().files[0].clone(),
                };
                Msg::FileChanged {
                    path,
                    mtime: mtime.map(u64::from),
                }
            }
            Op::Deliver(i) => {
                self.deliver(usize::from(i));
                return None;
            }
            Op::DeliverAll => {
                self.settle();
                return None;
            }
            Op::Drop => {
                if !self.pending.is_empty() {
                    self.pending.remove(0);
                }
                return None;
            }
        })
    }
}

fn doc_path() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| world().dir.join("fuzz.md"))
}

fuzz_target!(|data: &[u8]| {
    let [lo, hi, rest @ ..] = data else {
        return;
    };
    let len = usize::from(u16::from_le_bytes([*lo, *hi])).min(rest.len());
    let (doc, script) = rest.split_at(len);
    let doc: Arc<str> = String::from_utf8_lossy(doc).into();
    let mut u = Unstructured::new(script);

    let (w, h): (u16, u8) = u.arbitrary().unwrap_or((79, 23));
    let config = Config {
        max_width: u
            .arbitrary::<Option<u8>>()
            .unwrap_or_default()
            .map(u16::from),
        table_mode: match u.int_in_range(0..=2).unwrap_or(0) {
            0 => TableMode::Auto,
            1 => TableMode::Grid,
            _ => TableMode::Records,
        },
        code_wrap: u.arbitrary().unwrap_or(true),
        start_line: u
            .arbitrary::<Option<u16>>()
            .unwrap_or_default()
            .map(u32::from),
        ..Config::default()
    };
    let graphics: Option<(u8, u8, u8)> = u.arbitrary().unwrap_or_default();

    let state = State::new(theme(), config);
    let terminal = Terminal::new(TestBackend::new(state.size.w, state.size.h)).expect("terminal");
    let mut s = Session {
        state,
        effects: Effects::new(None),
        pending: Vec::new(),
        doc: doc.clone(),
        terminal,
        u,
    };

    s.send(Msg::Init { size: size(w, h) });
    if let Some((proto, w, h)) = graphics {
        if let Some(msg) = s.intent(Op::Graphics { proto, w, h }) {
            s.send(msg);
        }
    }
    s.send(Msg::DocLoaded {
        path: doc_path().to_owned(),
        mtime: Some(1),
        text: doc,
    });
    for _ in 0..MAX_OPS {
        if s.state.quit {
            return;
        }
        // an exhausted `Unstructured` yields defaults forever, not errors
        if s.u.is_empty() {
            break;
        }
        let Ok(op) = s.u.arbitrary::<Op>() else {
            break;
        };
        if let Some(msg) = s.intent(op) {
            s.send(msg);
        }
    }
    s.settle();
});
