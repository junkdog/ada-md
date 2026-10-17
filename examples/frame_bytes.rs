//! Where a frame's bytes go (O-10, O-14): one page-down and one line scroll through the TUI's
//! encoder at B2's 120×40, and `--print` at 100 columns, split into cursor moves, SGR and text.
//!
//! ```zsh
//! cargo run --release --example frame_bytes -- ~/b2/corpus/{s,m,l}.md
//! ```

use std::cell::RefCell;
use std::io::{self, Write};
use std::rc::Rc;
use std::{env, fs};

use ada::app::{Cmd, Computed, Config, Dir, Msg, ScrollBy, Size, State, update};
use ada::grid::Screen;
use ada::layout::{LayoutParams, TableMode};
use ada::theme::Theme;
use ada::{doc, print, ui};
use ratatui::layout::Size as Cells;

#[derive(Clone, Default)]
struct Shared(Rc<RefCell<Vec<u8>>>);

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn main() {
    let theme = Theme::builtin();
    for path in env::args().skip(1) {
        let text = fs::read_to_string(&path).expect("read");
        println!("{path}");

        let size = Size { w: 120, h: 40 };
        let out = Shared::default();
        let mut screen = Screen::new(out.clone(), Cells::new(size.w, size.h));
        let mut state = State::new(theme.clone(), Config::default());
        update(&mut state, Msg::Init { size });
        let cmds = update(
            &mut state,
            Msg::DocLoaded {
                path: path.as_str().into(),
                mtime: None,
                text: text.as_str().into(),
                doc: Computed(None),
            },
        );
        for cmd in cmds {
            if let Cmd::Highlight(job) = cmd {
                update(&mut state, job.run());
            }
        }
        screen.draw(|g| ui::view(&state, g)).expect("draw");
        report("first paint", &out.0.take());
        for (name, by) in [
            ("page down", ScrollBy::Page(Dir::Next)),
            ("line down", ScrollBy::Rows(1)),
        ] {
            update(&mut state, Msg::Scroll { by });
            screen.draw(|g| ui::view(&state, g)).expect("draw");
            let bytes = out.0.take();
            if let Ok(dir) = env::var("FRAME_DUMP") {
                let file = std::path::Path::new(&path)
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                fs::write(
                    format!("{dir}/{file}-{}.bin", name.replace(' ', "-")),
                    &bytes,
                )
                .expect("dump");
            }
            report(name, &bytes);
        }
        // time per frame, scrolling as `b2 flood` does: 300 rows down, then 300 up, repeating
        for (name, by) in [("line", 1), ("page", 40)] {
            update(&mut state, Msg::Scroll { by: ScrollBy::Top });
            let frames = 6000;
            let start = std::time::Instant::now();
            for f in 0..frames {
                let dir = if f / 300 % 2 == 0 { by } else { -by };
                update(
                    &mut state,
                    Msg::Scroll {
                        by: ScrollBy::Rows(dir),
                    },
                );
                screen.draw(|g| ui::view(&state, g)).expect("draw");
                out.0.borrow_mut().clear();
            }
            let us = start.elapsed().as_secs_f64() * 1e6 / f64::from(frames);
            println!("  {name} scroll: {us:.1} µs per frame");
        }

        let params = LayoutParams {
            width: 100,
            max_width: None,
            table_mode: TableMode::default(),
            code_wrap: true,
            glyphs_gen: 0,
            images: None,
        };
        let printed = print::render(&doc::parse(&text), &theme, params, None);
        report("print -w 100", printed.as_bytes());
        println!();
    }
}

/// Bytes and counts per kind of output.
#[derive(Default)]
struct Tally {
    cup: (usize, usize),
    sgr: (usize, usize),
    resets: usize,
    other: (usize, usize),
    text: usize,
    spaces: usize,
    newlines: usize,
    fg_rgb: usize,
    bg_rgb: usize,
    color_bytes: usize,
}

fn report(name: &str, bytes: &[u8]) {
    let mut t = Tally::default();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'[') {
            let end = i
                + 2
                + bytes[i + 2..]
                    .iter()
                    .position(|b| (0x40..=0x7e).contains(b))
                    .expect("terminated");
            let len = end + 1 - i;
            let params = &bytes[i + 2..end];
            match bytes[end] {
                b'H' => t.cup = (t.cup.0 + 1, t.cup.1 + len),
                b'm' => {
                    t.sgr = (t.sgr.0 + 1, t.sgr.1 + len);
                    color_params(params, &mut t);
                }
                _ => t.other = (t.other.0 + 1, t.other.1 + len),
            }
            i = end + 1;
        } else {
            match bytes[i] {
                b' ' => t.spaces += 1,
                b'\n' => t.newlines += 1,
                _ => {}
            }
            t.text += 1;
            i += 1;
        }
    }
    let pct = |n: usize| 100.0 * n as f64 / bytes.len().max(1) as f64;
    println!(
        "  {name:<12} {:>7} B | cup {:>4} × = {:>6} B ({:>4.1}%) | sgr {:>4} × = {:>6} B ({:>4.1}%; \
         colors {:>5} B, fg rgb {:>4}, bg rgb {:>4}, resets {:>3}) | text {:>6} B ({:>4.1}%; spaces {:>5}, \
         newlines {:>4}) | other {} B",
        bytes.len(),
        t.cup.0,
        t.cup.1,
        pct(t.cup.1),
        t.sgr.0,
        t.sgr.1,
        pct(t.sgr.1),
        t.color_bytes,
        t.fg_rgb,
        t.bg_rgb,
        t.resets,
        t.text,
        pct(t.text),
        t.spaces,
        t.newlines,
        t.other.1,
    );
}

/// Count the colors in one SGR's parameters, and the bytes they take with their separators.
fn color_params(params: &[u8], t: &mut Tally) {
    let p: Vec<&[u8]> = params.split(|&b| b == b';').collect();
    let mut k = 0;
    while k < p.len() {
        let len = |n: usize| {
            p[k..(k + n).min(p.len())]
                .iter()
                .map(|s| s.len() + 1)
                .sum::<usize>()
        };
        match p[k] {
            b"0" | b"" => {
                t.resets += 1;
                k += 1;
            }
            b"38" | b"48" if p.get(k + 1) == Some(&&b"2"[..]) => {
                if p[k] == b"38" {
                    t.fg_rgb += 1;
                } else {
                    t.bg_rgb += 1;
                }
                t.color_bytes += len(5);
                k += 5;
            }
            b"38" | b"48" => {
                t.color_bytes += len(3);
                k += 3;
            }
            _ => k += 1,
        }
    }
}
