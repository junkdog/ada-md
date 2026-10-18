//! The interactive loop (ADR-04, ADR-05): an input thread and the workers feed one channel;
//! the main thread blocks in `recv()`, so idle costs nothing, then drains, updates and draws once.

use std::collections::{HashSet, VecDeque};
use std::io::{self, Stdout, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::{env, thread};

use ratatui::crossterm::cursor::Show;
use ratatui::crossterm::event::{self, Event};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, enable_raw_mode};
use ratatui::layout::{Rect, Size as Cells};
use ratatui::style::Color;

use super::{Bindings, Cmd, Effects, Keymap, Msg, Size, State, editor_command, osc52, update};
use crate::grid::{Grid, Screen};
use crate::guard;
use crate::images;
use crate::record::Recorder;
use crate::theme::StyleKey;
use crate::ui;

/// The terminal: each frame diffed against the last and written to stdout in one write.
type Term = Screen<Stdout>;

/// Take the terminal: raw mode, the alternate screen, and a panic hook that gives it back. ada
/// draws with its own grid, not ratatui's.
pub(crate) fn init_terminal() -> io::Result<Term> {
    ratatui::try_init()?;
    // ratatui's hook leaves the alternate screen; the frames hid the cursor, so show it after.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // a panic `guard::catch` handles leaves the terminal as it is
        if guard::catching() {
            return;
        }
        hook(info);
        let _ = execute!(io::stdout(), Show);
    }));
    new_screen()
}

/// Give the terminal back as it was: ratatui's `restore`, and the cursor every frame hid, which
/// ratatui's `Terminal` showed on drop but ada's `Screen` doesn't.
pub(crate) fn restore_terminal() {
    ratatui::restore();
    let _ = execute!(io::stdout(), Show);
}

/// A screen over the alternate screen just entered, which is blank.
fn new_screen() -> io::Result<Term> {
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
    let mut terminal = init_terminal().map_err(|e| format!("terminal: {e}"))?;
    let result = event_loop(
        &mut terminal,
        &mut state,
        path,
        &mut effects,
        recorder.as_mut(),
    );
    restore_terminal();
    if result.is_ok() && state.config.keep_screen {
        keep_screen(&mut terminal, &state);
    }
    // The recording is a `BufWriter`: its drop writes the tail.
    drop(recorder);
    std::mem::forget((terminal, state, effects));
    result
}

/// Leave the last frame on the screen ada just gave back, below the command that ran it: the
/// document's rows, without the status bar or the blank rows under a short document. Not when
/// the terminal changed size since the frame, which would wrap or cut its rows. Kitty images go to
/// the main screen's image store first: the alternate screen's held them.
fn keep_screen(terminal: &mut Term, state: &State) {
    let size = terminal.size();
    if terminal::size().ok() != Some((size.width, size.height)) {
        return;
    }
    let mut blank = Grid::empty(Rect::new(0, 0, 1, 1));
    blank.fill(state.theme.style(StyleKey::Text));
    let shown = terminal.shown();
    let mut rows = size.height.saturating_sub(1);
    while rows > 0 && (0..size.width).all(|x| shown[(x, rows - 1)] == blank[(0, 0)]) {
        rows -= 1;
    }
    // the kitty images in the kept rows, sent to the main screen's image store first, but for those
    // the frame sends itself: the frame that first painted an image holds its transmission
    let (mut colors, mut sent) = (HashSet::new(), HashSet::new());
    for y in 0..rows {
        for x in 0..size.width {
            if let (true, Color::Rgb(r, g, b)) = (shown[(x, y)].is_image(), shown[(x, y)].fg()) {
                let color = u32::from_be_bytes([0, r, g, b]);
                colors.insert(color);
                if shown.symbol((x, y)).starts_with("\x1b_G") {
                    sent.insert(color);
                }
            }
        }
    }
    colors.retain(|c| !sent.contains(c));
    let mut out = io::stdout();
    for t in state
        .images
        .transmits()
        .filter(|t| colors.contains(&t.color))
    {
        let _ = out.write_all(t.seq.as_bytes());
    }
    let _ = terminal.keep(rows);
}

/// One malloc arena for every thread (glibc), set before any thread starts. A document the loader
/// builds lands in its thread's arena, layouts built on the UI thread in the main one, and pages
/// freed in one arena don't serve the other: a session on a 2 MB document that reloaded and
/// resized peaked 6–27 MiB higher, and its first paint 4 MiB (O-32, ADR-27). Print mode too,
/// since highlighting runs on a watched thread (F-06): its arena put 2 MB's print 3.7 MiB higher,
/// and one arena is no slower with O-04's parallel blocks (experiment 20).
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn one_arena() {
    // SAFETY: `mallopt` only changes a setting of glibc's malloc; `M_ARENA_MAX` caps the arenas
    // created from now on, and no other thread runs yet.
    unsafe {
        libc::mallopt(libc::M_ARENA_MAX, 1);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn one_arena() {}

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
    // What waits for a navigation's document, which loads off this thread (O-32): keys and
    // resizes answer from the document on screen meanwhile, but `Back Back` still goes back
    // twice, and a save during a reload is read once it is in.
    let mut held: VecDeque<Msg> = VecDeque::new();
    loop {
        while let Some(msg) = queue.pop_front() {
            if state.loading() && msg.waits_for_load() {
                held.push_back(msg);
                continue;
            }
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
            if !state.loading() {
                results.extend(held.drain(..));
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
    restore_terminal();
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
