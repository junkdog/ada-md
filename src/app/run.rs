//! The interactive loop (ADR-04, ADR-05): an input thread and the workers feed one channel;
//! the main thread blocks in `recv()`, so idle costs nothing, then drains, updates and draws once.

use std::collections::VecDeque;
use std::io::{self, Stdout, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::{env, thread};

use ratatui::crossterm::event::{self, Event};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, enable_raw_mode};
use ratatui::layout::Size as Cells;

use super::{Bindings, Cmd, Effects, Keymap, Msg, Size, State, editor_command, osc52, update};
use crate::grid::Screen;
use crate::images;
use crate::record::Recorder;
use crate::ui;

/// The terminal: each frame diffed against the last and written to stdout in one write.
type Term = Screen<Stdout>;

/// A screen over the alternate screen just entered, which is blank.
pub(crate) fn new_screen() -> io::Result<Term> {
    let (w, h) = terminal::size()?;
    Ok(Screen::new(io::stdout(), Cells::new(w, h)))
}

fn terminal_size() -> Result<Size, String> {
    let (w, h) = terminal::size().map_err(|e| e.to_string())?;
    Ok(Size { w, h })
}

/// Draw a frame at the terminal's size, clearing it first if that changed, as ratatui's
/// `Terminal::draw` did.
pub(crate) fn draw_frame(terminal: &mut Term, state: &State) -> io::Result<()> {
    let (w, h) = terminal::size()?;
    if Cells::new(w, h) != terminal.size() {
        terminal.resize(Cells::new(w, h));
    }
    terminal.draw(|grid| ui::view(state, grid))
}

/// Run ada in the terminal until the user quits, for a process that exits next: the documents,
/// their layouts and the caches are left for the OS to reclaim instead of freed (O-31: 45 ms on
/// quit for a 20 MB document).
pub fn run_tui(
    mut state: State,
    path: PathBuf,
    mut effects: Effects,
    mut recorder: Option<Recorder>,
) -> Result<(), String> {
    // `try_init` sets up the screen and the panic hook; ada draws with its own grid, not ratatui's.
    ratatui::try_init().map_err(|e| format!("terminal: {e}"))?;
    let mut terminal = new_screen().map_err(|e| format!("terminal: {e}"))?;
    let result = event_loop(
        &mut terminal,
        &mut state,
        path,
        &mut effects,
        recorder.as_mut(),
    );
    ratatui::restore();
    // The recording is a `BufWriter`: its drop writes the tail.
    drop(recorder);
    std::mem::forget((terminal, state, effects));
    result
}

fn event_loop(
    terminal: &mut Term,
    state: &mut State,
    path: PathBuf,
    effects: &mut Effects,
    mut recorder: Option<&mut Recorder>,
) -> Result<(), String> {
    let size = terminal_size()?;
    // The query reads the terminal's answers from stdin, so it goes before the input thread.
    let picker = images::detect(state.config.images);
    let graphics = picker.as_ref().map(images::graphics_msg);
    if let Some(picker) = picker {
        effects.set_picker(picker);
    }
    let (tx, rx) = mpsc::channel();
    effects.spawn_workers(&tx);
    let (resume, resumed) = mpsc::channel();
    spawn_input(tx, state.config.keys.clone(), resumed);

    let init = Msg::Init { size };
    let mut queue: VecDeque<Msg> = [init].into_iter().chain(graphics).collect();
    queue.push_back(Msg::Open { path });
    loop {
        while let Some(msg) = queue.pop_front() {
            if let Some(rec) = recorder.as_deref_mut() {
                rec.log(&msg).map_err(|e| format!("recording: {e}"))?;
            }
            let edit = msg == Msg::Edit;
            // An effect's results apply before any input queued behind the message, so a quick
            // `Back Back` can't overtake the load the first one started.
            let mut results = Vec::new();
            for cmd in update(state, msg) {
                results.extend(run_cmd(terminal, effects, cmd)?);
            }
            if edit {
                // the editor may have left the terminal another size
                let now = terminal_size()?;
                if now != state.size {
                    results.push(Msg::Resize { size: now });
                }
                // the input thread waits after sending `Edit`, so it can't steal the editor's keys
                let _ = resume.send(());
            }
            for msg in results.into_iter().rev() {
                queue.push_front(msg);
            }
        }
        if state.quit {
            return Ok(());
        }
        draw_frame(terminal, state).map_err(|e| e.to_string())?;

        let Ok(msg) = rx.recv() else {
            return Ok(());
        };
        for msg in std::iter::once(msg).chain(rx.try_iter()) {
            // A resize drag produces a burst, and so does typing a search pattern quickly: only
            // the last one matters.
            let superseded = matches!(
                (&msg, queue.back()),
                (Msg::Resize { .. }, Some(Msg::Resize { .. }))
                    | (Msg::SearchInput { .. }, Some(Msg::SearchInput { .. }))
                    | (Msg::OutlineFilter { .. }, Some(Msg::OutlineFilter { .. }))
            );
            if superseded {
                queue.pop_back();
            }
            queue.push_back(msg);
        }
    }
}

/// Run one command: the terminal's own here, the rest through the effect runner.
fn run_cmd(terminal: &mut Term, effects: &mut Effects, cmd: Cmd) -> Result<Option<Msg>, String> {
    match cmd {
        Cmd::SpawnEditor { file, line } => edit(terminal, &file, line),
        Cmd::Osc52(text) => {
            let mut out = io::stdout();
            let seq = osc52(&text, env::var_os("TMUX").is_some());
            out.write_all(seq.as_bytes())
                .and_then(|()| out.flush())
                .map_err(|e| format!("clipboard: {e}"))?;
            Ok(None)
        }
        cmd => Ok(effects.run(cmd)),
    }
}

/// Hand the terminal to the editor (ADR-18): leave the alternate screen, run it in the
/// foreground, then take the screen back and redraw all of it.
fn edit(terminal: &mut Term, file: &Path, line: u32) -> Result<Option<Msg>, String> {
    let editor = ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(env::var_os)
        .map(|v| v.to_string_lossy().into_owned())
        .find(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "vi".to_owned());
    let Some(argv) = editor_command(&editor, file, line) else {
        return Ok(None);
    };
    ratatui::restore();
    let status = Command::new(&argv[0]).args(&argv[1..]).status();
    let back = || -> io::Result<Term> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Clear(ClearType::All))?;
        // A new screen has a blank last frame, so the next draw paints every cell.
        new_screen()
    };
    *terminal = back().map_err(|e| format!("terminal: {e}"))?;
    Ok(Some(match status {
        Ok(status) => Msg::EditorExited {
            status: status.code().unwrap_or(-1),
        },
        Err(e) => Msg::OpenFailed {
            target: editor,
            error: e.to_string(),
        },
    }))
}

fn spawn_input(tx: Sender<Msg>, keys: Bindings, resume: Receiver<()>) {
    thread::spawn(move || {
        let mut keymap = Keymap::new(keys);
        loop {
            let msg = match event::read() {
                Ok(Event::Key(key)) => keymap.map(key),
                Ok(Event::Resize(w, h)) => Some(Msg::Resize {
                    size: Size { w, h },
                }),
                Ok(_) => None,
                Err(_) => return,
            };
            let Some(msg) = msg else {
                continue;
            };
            let edit = msg == Msg::Edit;
            if tx.send(msg).is_err() {
                return;
            }
            // While the editor runs, the terminal's input is the editor's.
            if edit && resume.recv().is_err() {
                return;
            }
        }
    });
}
