//! Fuzz round 2 (plan P7): a whole session, scripted by the input. The input is a document
//! (`[len_lo, len_hi, ..len bytes]`), then bytes `arbitrary` turns into a terminal size, config,
//! and up to 64 intents. Commands run against a stub world: links resolve to fuzz-chosen files
//! in a scratch directory, git answers with fuzzed hunks, image probes report fuzzed sizes, and
//! every result waits in a queue until the script delivers it, so results can arrive late, out
//! of order, or after the document they were for is gone. Intents arrive directly or as keys
//! through the default keymap, and `T` lists two extra themes with wide, empty and ASCII
//! glyphs. The document is markdown, code, config, plain text or an image, as its name says,
//! and links lead to code files at a `#L42` line. Every step is painted. Nothing may panic.
//!
//! ```zsh
//! cargo +nightly fuzz run -s none -a session -- -dict=fuzz/markdown.dict -max_len=16384
//! ```

#![no_main]

use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use ada::app::{self, Cmd, Computed, Config, Dir, Effects, Keymap, Msg, ScrollBy, Size, State};
use ada::grid::{Grid, Screen};
use ada::highlight;
use ada::images::{Probe, Proto};
use ada::layout::{LineNumbers, TableMode};
use ada::links::Resolved;
use ada::theme::Theme;
use libfuzzer_sys::arbitrary::{self, Arbitrary, Result, Unstructured};
use libfuzzer_sys::fuzz_target;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Size as Cells;
use ratatui::style::{Color, Modifier};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
    SortOpen,
    SortTable(bool),
    ToggleCodeWrap,
    ToggleLineNumbers,
    ScrollCodeX(i16),
    FocusLink(bool),
    Follow,
    Back,
    Forward,
    OpenOutline,
    OpenLinks,
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
    OpenThemes,
    ThemesTone(bool),
    Help,
    HelpScroll(i16),
    HelpClose,
    Edit,
    Yank,
    Open(u8),
    /// A key press, through the default keymap.
    Key {
        code: u8,
        mods: u8,
    },
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
        let rust: String = (1..=40)
            .map(|i| format!("fn f{i}() {{ /* {} */ }}\n", "日x".repeat(i % 7 * 9)))
            .collect();
        let files = vec![
            write(
                "a.md",
                b"# A\n\nback to [fuzz](fuzz.md), [[b]] and [code](c.rs#L3-L5)\n",
            ),
            write("b.md", b"## B\n\n![i](img.png)\n\n| x |\n|---|\n| y |\n"),
            write("img.png", &png(3, 2)),
            write("tall.png", &png(1, 64)),
            write("wide.jpg", &jpeg),
            write("broken.png", b"\x89PNG\r\n\x1a\nnot really"),
            dir.join("missing.png"),
            dir.join("missing.md"),
            dir.join("fuzz.md"),
            write("c.rs", rust.as_bytes()),
            write(
                "conf.toml",
                b"[a]\nb = \"[x](a.md)\" # c\n\n[[d]]\ne = [1, 2]\n",
            ),
            write("notes.txt", b"plain\ttext, [not](a.md) a link\n"),
            write("run", b"#!/usr/bin/env python3\nprint('ada')\n"),
            write("blob.dat", b"ELF\0\0\x01"),
        ];
        World { dir, files }
    })
}

fn theme() -> Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(Theme::builtin).clone()
}

/// Themes whose glyphs and `[syntax]` differ from the built-ins', so `T` lays out and
/// highlights again: wide and multi-cell glyphs with a wide margin, and empty ones with none.
fn extra_themes() -> &'static [Theme] {
    const WIDE: &str = r#"
extends = "gruvbox-dark"
[document]
margin = 7
[heading]
1 = { prefix = "🦀🦀 ", suffix = " 日本" }
2 = { prefix = "" }
[glyphs]
bullets = ["★", "→→", "日"]
quote_bar = "┃日 "
rule = "═"
table_border = "ascii"
task = ["✅ ", "⬜⬜ "]
wrap_mark = "⏎"
code_clip = ["◀◀", "▶"]
sort_mark = ["⬆⬆", "日"]
image = "🖼🖼 "
[syntax]
"keyword" = { fg = "blue", bold = true }
"string" = { fg = "red", underline = true }
"keyword.control.flow" = { fg = "aqua" }   # a new selector: only selectors trigger a re-highlight
"#;
    const EMPTY: &str = r#"
extends = "gruvbox-light"
[document]
margin = 0
[glyphs]
bullets = [""]
quote_bar = ""
table_border = "none"
task = ["", ""]
code_clip = ["", ""]
sort_mark = ["", ""]
image = ""
[syntax]
"comment" = { fg = "green" }
"#;
    static THEMES: OnceLock<Vec<Theme>> = OnceLock::new();
    THEMES.get_or_init(|| {
        [("fuzz-wide", WIDE), ("fuzz-empty", EMPTY)]
            .into_iter()
            .map(|(name, text)| {
                let mut theme = ada::theme::load_str(name, text, &mut Vec::new(), 0)
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
                theme.name = name.into();
                theme
            })
            .collect()
    })
}

/// A key the keymap might care about: printable ASCII, or a named key.
fn key(code: u8, mods: u8) -> KeyEvent {
    const NAMED: [KeyCode; 14] = [
        KeyCode::Enter,
        KeyCode::Esc,
        KeyCode::Tab,
        KeyCode::BackTab,
        KeyCode::Backspace,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::F(1),
    ];
    let code = match code {
        0x20..=0x7e => KeyCode::Char(char::from(code)),
        _ => NAMED[usize::from(code) % NAMED.len()],
    };
    // mostly plain keys; ctrl, alt and shift now and then
    let mods = match mods >> 5 {
        0 => KeyModifiers::from_bits_truncate(mods),
        _ => KeyModifiers::NONE,
    };
    KeyEvent::new(code, mods)
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
        Cmd::LoadDoc { path, prepare, .. } => {
            let msg = match u.int_in_range(0..=7)? {
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
                        doc: Computed(None),
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
                        doc: Computed(None),
                    }
                }
                _ => Msg::DocLoaded {
                    path,
                    mtime: u.arbitrary()?,
                    text: doc.clone(),
                    doc: Computed(None),
                },
            };
            // a navigation's, now and then as the loader posts it: parsed for the screen as it
            // was when the load began (O-32)
            Some(match u.arbitrary()? {
                true => app::prepare(msg, &prepare),
                false => msg,
            })
        }
        // now and then the marker a replay carries, which `update` asks for again
        Cmd::Highlight(job) if u.ratio(1, 8)? => Some(Msg::Highlighted {
            doc_gen: job.doc_gen,
            block: job.block,
            lines: match u.arbitrary()? {
                true => Some(u.int_in_range(0..=600)?),
                false => None,
            },
            spans: Computed(None),
        }),
        // now and then only the first lines, as the worker posts them for a long block, and
        // never the rest, as when it stops for another document (O-05)
        Cmd::Highlight(job) if u.ratio(1, 8)? => {
            let n = u.int_in_range(0..=600)?;
            let spans = highlight::highlight_first(job.key, &job.lang, &job.text, &job.theme, n);
            Some(Msg::Highlighted {
                doc_gen: job.doc_gen,
                block: job.block,
                lines: Some(n),
                spans: Computed(Some(Arc::new(spans))),
            })
        }
        // a replayed marker's job keeps its length
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
        Cmd::Watch { .. } | Cmd::HighlightOnly(_) | Cmd::Quit => None,
    })
}

/// `ADA_FUZZ_TRACE=1` prints every message and every frame's bytes, to follow a failing input.
fn trace() -> bool {
    static TRACE: OnceLock<bool> = OnceLock::new();
    *TRACE.get_or_init(|| std::env::var_os("ADA_FUZZ_TRACE").is_some())
}

/// The screen's writer: the terminal model reads what it wrote.
struct Tap(Rc<RefCell<Vec<u8>>>);

impl io::Write for Tap {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Styles that show on a blank, as ada's encoder counts them: on any other blank, only the
/// background shows, and the encoder may leave the rest of the pen as it was.
const VISIBLE_ON_BLANK: Modifier = Modifier::REVERSED
    .union(Modifier::UNDERLINED)
    .union(Modifier::SLOW_BLINK)
    .union(Modifier::RAPID_BLINK)
    .union(Modifier::CROSSED_OUT);

/// The terminal's color for one of ada's: named colors are the first 16 indexes.
fn term_color(c: Color) -> vt100::Color {
    const NAMED: [Color; 16] = [
        Color::Black,
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::Gray,
        Color::DarkGray,
        Color::LightRed,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
        Color::White,
    ];
    match c {
        Color::Reset => vt100::Color::Default,
        Color::Indexed(i) => vt100::Color::Idx(i),
        Color::Rgb(r, g, b) => vt100::Color::Rgb(r, g, b),
        named => vt100::Color::Idx(NAMED.iter().position(|&n| n == named).unwrap_or(0) as u8),
    }
}

/// The columns ada gives a grapheme (`grid::cell_width`): its width, plus one for each
/// halfwidth voiced mark.
fn cell_width(g: &str) -> usize {
    g.width()
        + g.chars()
            .filter(|c| matches!(c, '\u{FF9E}' | '\u{FF9F}'))
            .count()
}

/// What vt100 is given in place of grapheme `g`, so that it lays the grapheme out as ada does.
/// vt100 measures a code point at a time, and attaches a zero-width one to the cell before,
/// so a grapheme of several code points (an emoji sequence, a prepended mark) takes the wrong
/// columns; and it drops U+FFFD, which terminals draw (and fuzzed documents are full of). Such
/// a grapheme becomes one private-use character, or one wide CJK one, of ada's width.
fn stand_in(g: &str) -> &str {
    let mut chars = g.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c != '\u{fffd}' && c.width() == Some(cell_width(g)) => g,
        _ => match cell_width(g) {
            0 => "",
            1 => "\u{e000}",
            _ => "\u{4e00}",
        },
    }
}

/// A frame's bytes as vt100 is given them: escape sequences and control characters as they
/// are, and each symbol written by its [`stand_in`]. The symbols are the new frame's, at the
/// cursor, which follows ada's moves (`CUP`, `CUF`, `CHA`, CR, LF, and the margins and line
/// moves of a scroll), as the text alone can't say where one cell ends: `"` and `🏽` from two
/// cells read as one grapheme. Text that isn't what the grid holds there goes a grapheme at a
/// time, and vt100 then shows the difference. `rows` gets the rows the bytes touch, all of
/// them when lines move or the screen clears.
fn for_vt100(bytes: &[u8], grid: &Grid, rows: &mut [bool]) -> Vec<u8> {
    let touch = |rows: &mut [bool], y: u16| {
        if let Some(r) = rows.get_mut(usize::from(y)) {
            *r = true;
        }
    };
    let area = grid.area();
    let mut out = Vec::with_capacity(bytes.len());
    let (mut x, mut y) = (0u16, 0u16);
    let mut i = 0;
    while i < bytes.len() {
        let end = match bytes[i] {
            0x1b => match bytes.get(i + 1) {
                // CSI: parameters up to a final byte
                Some(b'[') => {
                    let end = (i + 2..bytes.len())
                        .find(|&j| (0x40..=0x7e).contains(&bytes[j]))
                        .map_or(bytes.len(), |j| j + 1);
                    let params = std::str::from_utf8(&bytes[i + 2..end - 1]).unwrap_or("");
                    let mut n = params
                        .split(';')
                        .map(|p| p.parse::<u16>().unwrap_or(1).max(1));
                    match bytes[end - 1] {
                        b'H' => {
                            y = n.next().unwrap_or(1) - 1;
                            x = n.next().unwrap_or(1) - 1;
                        }
                        b'C' => x = x.saturating_add(n.next().unwrap_or(1)),
                        b'G' => x = n.next().unwrap_or(1) - 1,
                        b'r' => (x, y) = (0, 0),
                        b'L' | b'M' => {
                            x = 0;
                            rows.fill(true);
                        }
                        b'J' => rows.fill(true),
                        b'K' => touch(rows, y),
                        _ => {}
                    }
                    end
                }
                // a string (DCS, APC, OSC, PM, SOS): up to BEL or ST
                Some(b'P' | b'_' | b']' | b'^' | b'X') => (i + 2..bytes.len())
                    .find(|&j| bytes[j] == 0x07 || bytes[j..].starts_with(b"\x1b\\"))
                    .map_or(bytes.len(), |j| j + if bytes[j] == 0x07 { 1 } else { 2 }),
                _ => (i + 2).min(bytes.len()),
            },
            b'\r' => {
                x = 0;
                i + 1
            }
            b'\n' => {
                y = y.saturating_add(1);
                i + 1
            }
            b if b < 0x20 || b == 0x7f => i + 1,
            _ => {
                let end = (i..bytes.len())
                    .find(|&j| bytes[j] < 0x20 || bytes[j] == 0x7f)
                    .unwrap_or(bytes.len());
                let mut text = std::str::from_utf8(&bytes[i..end]).expect("ada writes UTF-8");
                touch(rows, y);
                while !text.is_empty() {
                    let here = (x < area.width && y < area.height)
                        .then(|| (grid.symbol((x, y)), grid[(x, y)].width()))
                        .filter(|(s, _)| !s.is_empty() && text.starts_with(s));
                    let (symbol, width) = here.unwrap_or_else(|| {
                        let g = text.graphemes(true).next().unwrap_or(text);
                        (g, cell_width(g) as u16)
                    });
                    out.extend_from_slice(stand_in(symbol).as_bytes());
                    text = &text[symbol.len()..];
                    x = x.saturating_add(width.max(1));
                }
                i = end;
                continue;
            }
        };
        out.extend_from_slice(&bytes[i..end]);
        i = end;
    }
    out
}

/// The terminal shows what the grid holds, cell by cell: what the diff left out was already
/// there, and what the encoder wrote, scroll moves included, lands where it should (F-04). A
/// blank is compared by what shows of it, a grapheme by its [`stand_in`]. Image cells, and
/// graphemes no or several columns wide, are left out. With `rows`, only the rows it marks.
fn shows(term: &vt100::Screen, grid: &Grid, rows: Option<&[bool]>) {
    let area = grid.area();
    assert_eq!(term.size(), (area.height, area.width));
    for y in 0..area.height {
        if rows.is_some_and(|r| !r[usize::from(y)]) {
            continue;
        }
        let mut x = 0;
        while x < area.width {
            let cell = &grid[(x, y)];
            let width = cell.width().max(1);
            let symbol = match grid.symbol((x, y)) {
                // ASCII
                s if s.len() == 1 => s,
                s => stand_in(s),
            };
            if !symbol.is_empty() && width <= 2 && !cell.is_image() {
                let m = cell.modifier();
                let blank = symbol == " " && !m.intersects(VISIBLE_ON_BLANK);
                let styles = [
                    Modifier::BOLD,
                    Modifier::DIM,
                    Modifier::ITALIC,
                    Modifier::UNDERLINED,
                    Modifier::REVERSED,
                ]
                .map(|s| m.contains(s));
                let want = (
                    symbol,
                    term_color(cell.bg()),
                    (!blank).then(|| (term_color(cell.fg()), styles)),
                );
                let seen = term.cell(y, x).expect("cell in the terminal");
                let styles = [
                    seen.bold(),
                    seen.dim(),
                    seen.italic(),
                    seen.underline(),
                    seen.inverse(),
                ];
                let got = (
                    if seen.has_contents() {
                        seen.contents()
                    } else {
                        " "
                    },
                    seen.bgcolor(),
                    (!blank).then(|| (seen.fgcolor(), styles)),
                );
                assert_eq!(
                    want,
                    got,
                    "cell ({x}, {y}) of {}×{}: the grid's, then the terminal's; their rows:\n{:?}\n{:?}",
                    area.width,
                    area.height,
                    (0..area.width)
                        .map(|x| stand_in(grid.symbol((x, y))))
                        .collect::<String>(),
                    term.contents_between(y, 0, y, area.width),
                );
            }
            x += width;
        }
    }
}

struct Session<'a> {
    state: State,
    keymap: Keymap,
    effects: Effects,
    pending: Vec<Msg>,
    doc: Arc<str>,
    screen: Screen<Tap>,
    /// What the screen wrote since the last frame was checked.
    out: Rc<RefCell<Vec<u8>>>,
    term: vt100::Parser,
    paints: u32,
    u: Unstructured<'a>,
}

impl Session<'_> {
    /// Apply `msg`; queue the stub world's answers to its commands.
    fn send(&mut self, msg: Msg) {
        if trace() {
            eprintln!("msg {msg:?}");
        }
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

    /// Paint, diff against the last frame and encode: since the grid replaced ratatui's buffer,
    /// the diff and the encoder are ada's code too. The bytes go to a vt100 terminal, which
    /// must then show the frame (F-04).
    fn paint(&mut self) {
        let Size { w, h } = self.state.size;
        if self.screen.size() != Cells::new(w, h) {
            self.screen.resize(Cells::new(w, h));
            self.term.screen_mut().set_size(h, w);
        }
        let state = &self.state;
        self.screen
            .draw(|grid| ada::ui::view(state, grid))
            .expect("tap");
        let bytes = std::mem::take(&mut *self.out.borrow_mut());
        if trace() {
            eprintln!("frame {}", bytes.escape_ascii());
        }
        // the rows this frame touched, and every row now and then: a changed row the diff left
        // out entirely stays wrong until it's checked
        let mut rows = vec![false; usize::from(h)];
        self.term
            .process(&for_vt100(&bytes, self.screen.shown(), &mut rows));
        self.paints += 1;
        let all = self.paints.is_multiple_of(8);
        shows(
            self.term.screen(),
            self.screen.shown(),
            (!all).then_some(&rows[..]),
        );
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
            Op::SortOpen => Msg::SortOpen,
            Op::SortTable(next) => Msg::SortTable { dir: dir(next) },
            Op::ToggleCodeWrap => Msg::ToggleCodeWrap,
            Op::ToggleLineNumbers => Msg::ToggleLineNumbers,
            Op::ScrollCodeX(by) => Msg::ScrollCodeX { by },
            Op::FocusLink(next) => Msg::FocusLink { dir: dir(next) },
            Op::Follow => Msg::Follow,
            Op::Back => Msg::Back,
            Op::Forward => Msg::Forward,
            Op::OpenOutline => Msg::OpenOutline,
            Op::OpenLinks => Msg::OpenLinks,
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
            Op::OpenThemes => Msg::OpenThemes,
            Op::ThemesTone(next) => Msg::ThemesTone { dir: dir(next) },
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
            Op::Key { code, mods } => return self.keymap.map(key(code, mods)),
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

/// Names the document can have. Its file type decides how it's parsed and laid out: markdown
/// mostly, then code and config in a few syntaxes, plain text, an image, a name only the first
/// line explains (markdown unless it's a `#!` line), and an extension nothing claims.
const DOC_NAMES: [&str; 12] = [
    "fuzz.md",
    "fuzz.md",
    "fuzz.md",
    "fuzz.md",
    "fuzz.rs",
    "Cargo.toml",
    "ci.yml",
    "fuzz.py",
    "fuzz.txt",
    "fuzz",
    "fuzz.png",
    "fuzz.xyz",
];

fn doc_path(name: usize) -> PathBuf {
    world().dir.join(DOC_NAMES[name % DOC_NAMES.len()])
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
        line_numbers: match u.int_in_range(0..=2).unwrap_or(0) {
            0 => LineNumbers::Off,
            1 => LineNumbers::Code,
            _ => LineNumbers::All,
        },
        start_line: u
            .arbitrary::<Option<u16>>()
            .unwrap_or_default()
            .map(u32::from),
        ..Config::default()
    };
    let graphics: Option<(u8, u8, u8)> = u.arbitrary().unwrap_or_default();
    let stdin = u.ratio(1, 16).unwrap_or(false);
    let name: usize = u.int_in_range(0..=DOC_NAMES.len() - 1).unwrap_or(0);

    let mut state = State::new(theme(), config);
    state.themes.extend_from_slice(extra_themes());
    let out = Rc::new(RefCell::new(Vec::new()));
    let screen = Screen::new(Tap(out.clone()), Cells::new(state.size.w, state.size.h));
    let term = vt100::Parser::new(state.size.h, state.size.w, 0);
    let mut s = Session {
        state,
        keymap: Keymap::default(),
        effects: Effects::new(None),
        pending: Vec::new(),
        doc: doc.clone(),
        screen,
        out,
        term,
        paints: 0,
        u,
    };

    s.send(Msg::Init { size: size(w, h) });
    if let Some((proto, w, h)) = graphics
        && let Some(msg) = s.intent(Op::Graphics { proto, w, h })
    {
        s.send(msg);
    }
    s.send(Msg::DocLoaded {
        path: if stdin { "-".into() } else { doc_path(name) },
        mtime: Some(1),
        text: doc,
        doc: Computed(None),
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
    shows(s.term.screen(), s.screen.shown(), None);
});
