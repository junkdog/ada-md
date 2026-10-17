//! The interactive loop (ADR-04, ADR-05): an input thread and the workers feed one channel;
//! the main thread blocks in `recv()`, so idle costs nothing, then drains, updates and draws once.

use std::collections::VecDeque;
use std::io::{self, BufWriter, Stdout, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::{env, thread};

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{Clear, ClearType, EnterAlternateScreen, enable_raw_mode};

use super::{Bindings, Cmd, Effects, Keymap, Msg, Size, State, editor_command, osc52, update};
use crate::images;
use crate::record::Recorder;
use crate::ui;

/// A terminal whose frames reach stdout in one write. Stdout itself is a 1 KB line buffer behind a
/// lock taken for every piece crossterm formats: about ten writes a frame, and a fifth of its CPU.
type Term = Terminal<CrosstermBackend<BufWriter<Stdout>>>;

fn new_terminal() -> io::Result<Term> {
    Terminal::new(CrosstermBackend::new(BufWriter::with_capacity(
        1 << 16,
        io::stdout(),
    )))
}

/// Run ada in the terminal until the user quits.
pub fn run_tui(
    mut state: State,
    path: PathBuf,
    mut effects: Effects,
    mut recorder: Option<Recorder>,
) -> Result<(), String> {
    // `try_init` sets up the screen and the panic hook; the terminal it returns writes unbuffered.
    ratatui::try_init().map_err(|e| format!("terminal: {e}"))?;
    let mut terminal = new_terminal().map_err(|e| format!("terminal: {e}"))?;
    let result = event_loop(
        &mut terminal,
        &mut state,
        path,
        &mut effects,
        recorder.as_mut(),
    );
    ratatui::restore();
    result
}

fn event_loop(
    terminal: &mut Term,
    state: &mut State,
    path: PathBuf,
    effects: &mut Effects,
    mut recorder: Option<&mut Recorder>,
) -> Result<(), String> {
    let size = terminal.size().map_err(|e| e.to_string())?;
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

    let init = Msg::Init {
        size: Size {
            w: size.width,
            h: size.height,
        },
    };
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
                let now = terminal.size().map_err(|e| e.to_string())?;
                let now = Size {
                    w: now.width,
                    h: now.height,
                };
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
        terminal
            .draw(|frame| ui::view(state, frame))
            .map_err(|e| e.to_string())?;

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
        // A new terminal has a blank back buffer, so the next draw paints every cell.
        // (`Terminal::clear` would ask for the cursor's position, which needs an answer.)
        new_terminal()
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
