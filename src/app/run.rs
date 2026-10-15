//! The interactive loop (ADR-04, ADR-05): an input thread and (later) workers feed one channel;
//! the main thread blocks in `recv()`, so idle costs nothing, then drains, updates and draws once.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event};

use super::{Effects, Keymap, Msg, Size, State, update};
use crate::record::Recorder;
use crate::ui;

/// Run preen in the terminal until the user quits.
pub fn run_tui(
    mut state: State,
    path: PathBuf,
    mut effects: Effects,
    mut recorder: Option<Recorder>,
) -> Result<(), String> {
    let mut terminal = ratatui::try_init().map_err(|e| format!("terminal: {e}"))?;
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
    terminal: &mut DefaultTerminal,
    state: &mut State,
    path: PathBuf,
    effects: &mut Effects,
    mut recorder: Option<&mut Recorder>,
) -> Result<(), String> {
    let size = terminal.size().map_err(|e| e.to_string())?;
    let (tx, rx) = mpsc::channel();
    spawn_input(tx);

    let mut queue = VecDeque::from([
        Msg::Init {
            size: Size {
                w: size.width,
                h: size.height,
            },
        },
        Msg::Open { path },
    ]);
    loop {
        while let Some(msg) = queue.pop_front() {
            if let Some(rec) = recorder.as_deref_mut() {
                rec.log(&msg).map_err(|e| format!("recording: {e}"))?;
            }
            for cmd in update(state, msg) {
                queue.extend(effects.run(&cmd));
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
            // A resize drag produces a burst; only the last size matters.
            if matches!(msg, Msg::Resize { .. }) && matches!(queue.back(), Some(Msg::Resize { .. }))
            {
                queue.pop_back();
            }
            queue.push_back(msg);
        }
    }
}

fn spawn_input(tx: Sender<Msg>) {
    thread::spawn(move || {
        let mut keymap = Keymap::default();
        loop {
            let msg = match event::read() {
                Ok(Event::Key(key)) => keymap.map(key),
                Ok(Event::Resize(w, h)) => Some(Msg::Resize {
                    size: Size { w, h },
                }),
                Ok(_) => None,
                Err(_) => return,
            };
            if let Some(msg) = msg
                && tx.send(msg).is_err()
            {
                return;
            }
        }
    });
}
