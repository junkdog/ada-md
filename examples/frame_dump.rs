//! Every frame of a scripted session as the encoder wrote it, for comparing two builds byte for
//! byte (O-25, O-26): line, page and half-page scrolls, links, table and code-wrap modes, a
//! search, the outline, help and a resize, on each file at 120×40 and 60×20.
//!
//! ```zsh
//! cargo run --release --example frame_dump -- ~/b2/corpus/*.md > new.bin
//! cmp base.bin new.bin
//! ```

use std::cell::RefCell;
use std::io::{self, Write};
use std::rc::Rc;
use std::{env, fs};

use ada::app::{Cmd, Computed, Config, Dir, Msg, ScrollBy, Size, State, update};
use ada::grid::Screen;
use ada::theme::Theme;
use ada::ui;
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

fn step(state: &mut State, msg: Msg) {
    for cmd in update(state, msg) {
        if let Cmd::Highlight(job) = cmd {
            step(state, job.run());
        }
    }
}

fn main() {
    let theme = Theme::builtin();
    let mut stdout = io::stdout().lock();
    for path in env::args().skip(1) {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for (w, h, w2, h2) in [(120, 40, 80, 24), (60, 20, 135, 50)] {
            let out = Shared::default();
            let mut screen = Screen::new(out.clone(), Cells::new(w, h));
            let mut state = State::new(theme.clone(), Config::default());
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
                },
                Msg::SearchInput {
                    pattern: "the".into(),
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
            writeln!(stdout, "\n=== {path} {w}x{h}").unwrap();
            stdout.write_all(&out.0.take()).unwrap();
        }
    }
}
