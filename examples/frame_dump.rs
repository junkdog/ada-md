//! Every frame of a scripted session as the encoder wrote it, for comparing two builds byte for
//! byte (O-25, O-26): line, page and half-page scrolls, links, table and code-wrap modes, a
//! search, the outline, help and a resize, on each file at 120×40 and 60×20.
//!
//! `--proto LIST` (comma-separated: kitty, sixel, iterm2, halfblocks) runs each file again for
//! each protocol, with cells of 10×20 px, and its images resolved, probed and decoded inline, as
//! the TUI's workers would. Without it, images stay frames with their alt text, as before.
//! Kitty's image ids, which differ from run to run, are renumbered (`id1`, `id2`, …).
//!
//! ```zsh
//! cargo run --release --example frame_dump -- bench/b2/root/corpus/*.md > new.bin
//! cmp base.bin new.bin
//! # from the repo's root, which image-scroll.md's paths start from
//! cargo run --release --example frame_dump -- --proto kitty,sixel,iterm2,halfblocks image-scroll.md
//! ```

use std::collections::HashMap;
use std::io::{self, Write};
use std::{env, fs};

use ada::app::{Cmd, Computed, Config, Cursor, Dir, Effects, Msg, ScrollBy, Size, State, update};
use ada::geom::Size as Cells;
use ada::grid::Screen;
use ada::images::{Encoder, Proto};
use ada::theme::Theme;
use ada::ui;
use regex::bytes::{Captures, Regex};

mod common;

use common::Shared;

/// The cell size images are drawn for: 10×20 px, as `image_scroll` draws them.
const CELL: Cells = Cells::new(10, 20);

/// Kitty's image ids differ from run to run (`kitty::image_id`), so each is renumbered from 1 in
/// the order the ids first show up, both in its transmission's `i=` and in its placeholders'
/// foreground color, so that two builds' dumps compare.
fn renumber_kitty(frames: &[u8]) -> Vec<u8> {
    let num = |b: &[u8]| std::str::from_utf8(b).ok()?.parse::<u32>().ok();
    let ids: HashMap<u32, u32> = Regex::new(r"\x1b_G[^;\x1b]*\bi=(\d+)")
        .unwrap()
        .captures_iter(frames)
        .filter_map(|c| num(&c[1]))
        .fold(HashMap::new(), |mut ids, id| {
            let next = ids.len() as u32 + 1;
            ids.entry(id).or_insert(next);
            ids
        });
    let ids = &ids;
    let frames = Regex::new(r"\b38;2;(\d+);(\d+);(\d+)")
        .unwrap()
        .replace_all(frames, |c: &Captures| {
            let rgb = [&c[1], &c[2], &c[3]].map(|v| num(v).unwrap_or(256));
            match ids.get(&(rgb[0] << 16 | rgb[1] << 8 | rgb[2])) {
                Some(n) if rgb.iter().all(|&v| v < 256) => format!("38;2;id{n}").into_bytes(),
                _ => c[0].to_vec(),
            }
        });
    Regex::new(r"(\x1b_G[^;\x1b]*\bi=)(\d+)")
        .unwrap()
        .replace_all(&frames, |c: &Captures| {
            match num(&c[2]).and_then(|id| ids.get(&id)) {
                Some(n) => [&c[1], format!("id{n}").as_bytes()].concat(),
                None => c[0].to_vec(),
            }
        })
        .into_owned()
}

/// Runs what `update` asks for that the frames show: highlighting, and with `effects` the
/// images, from resolving their links to decoding them.
fn step(state: &mut State, mut effects: Option<&mut Effects>, msg: Msg) {
    for cmd in update(state, msg) {
        let next = match (cmd, effects.as_deref_mut()) {
            (Cmd::Highlight(job), _) => Some(job.run()),
            (
                cmd @ (Cmd::ResolveLinks { .. } | Cmd::ProbeImages { .. } | Cmd::DecodeImage(_)),
                Some(effects),
            ) => effects.run(cmd),
            _ => None,
        };
        if let Some(next) = next {
            step(state, effects.as_deref_mut(), next);
        }
    }
}

fn main() {
    let theme = Theme::builtin();
    let mut stdout = io::stdout().lock();
    let (mut paths, mut protos) = (Vec::new(), vec![None]);
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--proto" {
            let list = args.next().unwrap_or_default();
            protos = list
                .split(',')
                .map(|name| Some(Proto::parse(name).unwrap_or_else(|| panic!("--proto {name}?"))))
                .collect();
        } else {
            paths.push(arg);
        }
    }
    for path in paths {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for (proto, (w, h, w2, h2)) in protos
            .iter()
            .flat_map(|&p| [(120, 40, 80, 24), (60, 20, 135, 50)].map(|size| (p, size)))
        {
            let out = Shared::default();
            let mut screen = Screen::new(out.clone(), Cells::new(w, h));
            let mut state = State::new(theme.clone(), Config::default());
            let mut effects = proto.map(|proto| {
                let mut effects = Effects::new(None);
                effects.set_encoder(Encoder::new(proto, CELL));
                effects
            });
            let mut step = |state: &mut State, msg| step(state, effects.as_mut(), msg);
            step(
                &mut state,
                Msg::Init {
                    size: Size { w, h },
                },
            );
            step(
                &mut state,
                Msg::DocLoaded {
                    path: path.as_str().into(),
                    mtime: None,
                    text: text.as_str().into(),
                    doc: Computed(None),
                },
            );
            if let Some(proto) = proto {
                step(
                    &mut state,
                    Msg::Graphics {
                        proto,
                        cell: Size {
                            w: CELL.width,
                            h: CELL.height,
                        },
                    },
                );
            }
            let mut script: Vec<Msg> = Vec::new();
            let scroll = |by| Msg::Scroll { by };
            script.extend((0..30).map(|_| scroll(ScrollBy::Rows(1))));
            script.extend([
                scroll(ScrollBy::Page(Dir::Next)),
                scroll(ScrollBy::Page(Dir::Next)),
                scroll(ScrollBy::HalfPage(Dir::Next)),
                scroll(ScrollBy::Rows(-1)),
                scroll(ScrollBy::Rows(-1)),
                scroll(ScrollBy::Rows(3)),
                Msg::FocusLink { dir: Dir::Next },
                Msg::FocusLink { dir: Dir::Next },
                Msg::FocusLink { dir: Dir::Next },
                Msg::ToggleTableMode,
                scroll(ScrollBy::Rows(1)),
                Msg::ToggleTableMode,
                Msg::ToggleCodeWrap,
                Msg::ScrollCodeX { by: 8 },
                scroll(ScrollBy::Rows(1)),
                Msg::ToggleCodeWrap,
                Msg::SearchOpen,
                Msg::SearchInput {
                    pattern: "th".into(),
                    cursor: Cursor::END,
                },
                Msg::SearchInput {
                    pattern: "the".into(),
                    cursor: Cursor::END,
                },
                Msg::SearchSubmit,
                Msg::SearchNext { dir: Dir::Next },
                Msg::SearchNext { dir: Dir::Next },
                scroll(ScrollBy::Rows(1)),
                scroll(ScrollBy::Rows(1)),
                Msg::OpenOutline,
                Msg::OutlineMove { by: 3 },
                Msg::OutlineClose,
                Msg::Help,
                Msg::HelpClose,
                scroll(ScrollBy::Bottom),
                scroll(ScrollBy::Rows(-1)),
                scroll(ScrollBy::Page(Dir::Prev)),
                scroll(ScrollBy::Top),
                Msg::Resize {
                    size: Size { w: w2, h: h2 },
                },
            ]);
            script.extend((0..10).map(|_| scroll(ScrollBy::Rows(1))));
            script.push(scroll(ScrollBy::Page(Dir::Next)));
            script.extend((0..5).map(|_| scroll(ScrollBy::Rows(-1))));
            screen.draw(|g| ui::view(&state, g)).expect("draw");
            for msg in script {
                step(&mut state, msg);
                let size = Cells::new(state.size.w, state.size.h);
                if size != screen.size() {
                    screen.resize(size);
                }
                screen.draw(|g| ui::view(&state, g)).expect("draw");
            }
            match proto {
                Some(proto) => writeln!(stdout, "\n=== {path} {w}x{h} {}", proto.name()),
                None => writeln!(stdout, "\n=== {path} {w}x{h}"),
            }
            .unwrap();
            let frames = out.0.take();
            match proto {
                Some(Proto::Kitty) => stdout.write_all(&renumber_kitty(&frames)),
                _ => stdout.write_all(&frames),
            }
            .unwrap();
        }
    }
}
