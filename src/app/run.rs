//! The interactive loop (ADR-04, ADR-05): an input thread and the workers feed one channel;
//! the main thread blocks in `recv()`, so idle costs nothing, then drains, updates and draws once.

use std::collections::{HashSet, VecDeque};
use std::io::{self, Stdout, Write};
use std::mem;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;
use std::{env, thread};

use crate::geom::{Rect, Size as Cells};
use crate::style::Color;
use crossterm::cursor::Show;
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
    enable_raw_mode,
};

use super::{
    Bindings, Cmd, Effects, Keymap, Msg, ScrollBy, Size, State, editor_command, osc52, update,
};
use crate::config;
use crate::grid::{Grid, Screen};
use crate::guard;
use crate::images;
use crate::probe::{self, Found};
use crate::record::Recorder;
use crate::theme::StyleKey;
use crate::ui;

/// The terminal: each frame diffed against the last and written to stdout in one write.
type Term = Screen<Stdout>;

/// Take the terminal: a panic hook that gives it back, raw mode and the alternate screen, as
/// ratatui's `try_init` took it.
pub(crate) fn init_terminal() -> io::Result<Term> {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // a panic `guard::catch` handles leaves the terminal as it is
        if guard::catching() {
            return;
        }
        restore_terminal();
        hook(info);
    }));
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    new_screen()
}

/// Give the terminal back as it was: raw mode off first (it does more than the screen), the
/// main screen, the mouse, and the cursor every frame hid.
pub(crate) fn restore_terminal() {
    let restored = disable_raw_mode().and_then(|()| execute!(io::stdout(), LeaveAlternateScreen));
    if let Err(e) = restored {
        eprintln!("Failed to restore terminal: {e}");
    }
    let _ = capture_mouse(false);
    let _ = execute!(io::stdout(), Show);
}

/// Capture the mouse, or give it back to the terminal: presses, releases and drags, in SGR's
/// encoding. crossterm's own capture also reports every motion with no button held, which
/// ada has no use for.
#[cfg(unix)]
fn capture_mouse(on: bool) -> io::Result<()> {
    let seq = if on {
        "\x1b[?1000h\x1b[?1002h\x1b[?1006h"
    } else {
        "\x1b[?1006l\x1b[?1002l\x1b[?1000l"
    };
    let mut out = io::stdout();
    out.write_all(seq.as_bytes()).and_then(|()| out.flush())
}

/// Windows' console takes the mouse by its input mode, which crossterm sets.
#[cfg(not(unix))]
fn capture_mouse(on: bool) -> io::Result<()> {
    use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
    if on {
        execute!(io::stdout(), EnableMouseCapture)
    } else {
        execute!(io::stdout(), DisableMouseCapture)
    }
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
    if state.mouse {
        capture_mouse(true).map_err(|e| format!("terminal: {e}"))?;
    }
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

/// Up to `n` malloc arenas (glibc), set before the threads that use them start. A print streamed
/// a piece at a time (O-52) gives its workers an arena each: on one, they waited on its lock and
/// on the kernel's for the blocks it maps, ~1.7× the CPU of a whole print on a JSON file, for
/// 1–2 MiB of peak.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn arenas(n: usize) {
    // SAFETY: as in `one_arena`; only threads started after this get the new arenas.
    unsafe {
        libc::mallopt(libc::M_ARENA_MAX, n.try_into().unwrap_or(i32::MAX));
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn arenas(_: usize) {}

fn event_loop(
    terminal: &mut Term,
    state: &mut State,
    path: PathBuf,
    effects: &mut Effects,
    mut recorder: Option<&mut Recorder>,
) -> Result<(), String> {
    let size = terminal_size()?;
    // The terminal is asked for its image support before the first frame, and its answer read on
    // the input thread, so the text paints without waiting for it (O-06).
    let plan = images::plan(state.config.images, |name| env::var(name).ok());
    if plan.is_some_and(|p| p.ask) {
        let mut out = io::stdout();
        out.write_all(probe::QUERY)
            .and_then(|()| out.flush())
            .map_err(|e| format!("terminal: {e}"))?;
    }
    let deadline = Instant::now() + probe::WAIT;
    let (tx, rx) = mpsc::channel();
    effects.spawn_workers(&tx);
    let (resume, resumed) = mpsc::channel();
    let keys = state.config.keys.clone();
    spawn_input(tx, keys, resumed, plan.map(|p| (p, deadline)));

    let mut queue = VecDeque::from([Msg::Init { size }, Msg::Open { path }]);
    // What waits for a navigation's document, which loads off this thread (O-32): keys and
    // resizes answer from the document on screen meanwhile, but `Back Back` still goes back
    // twice, and a save during a reload is read once it is in.
    let mut held: VecDeque<Msg> = VecDeque::new();
    // `S` was pressed, and the input thread waits to hear whether the save box opened
    let mut saving = false;
    loop {
        while let Some(msg) = queue.pop_front() {
            if state.loading() && msg.waits_for_load() {
                held.push_back(msg);
                continue;
            }
            if let Some(rec) = recorder.as_deref_mut() {
                rec.log(&msg).map_err(|e| format!("recording: {e}"))?;
            }
            if let Msg::Graphics { proto, cell } = msg {
                effects.set_encoder(images::Encoder::new(proto, Cells::new(cell.w, cell.h)));
            }
            let edit = msg == Msg::Edit;
            let listing = msg.may_list_themes();
            saving |= msg == Msg::SaveConfig;
            // An effect's results apply before any input queued behind the message, so a quick
            // `Back Back` can't overtake the load the first one started.
            let mut results = Vec::new();
            for cmd in update(state, msg) {
                results.extend(run_cmd(terminal, effects, cmd, state.mouse)?);
            }
            if edit {
                // the editor may have left the terminal another size
                let now = terminal_size()?;
                if now != state.size {
                    results.push(Msg::Resize { size: now });
                }
                // the input thread waits after sending `Edit`, so it can't steal the editor's keys
                let _ = resume.send(true);
            }
            if listing {
                // and after a change in the settings, to hear whether the theme list opened
                let _ = resume.send(state.outline.is_some());
            }
            if !state.loading() {
                results.extend(held.drain(..));
            }
            for msg in results.into_iter().rev() {
                queue.push_front(msg);
            }
        }
        // The file was read inline, and its text applied before the queue ran dry.
        if mem::take(&mut saving) {
            let _ = resume.send(state.save.is_some());
        }
        if state.quit {
            return Ok(());
        }
        draw_frame(terminal, state).map_err(|e| e.to_string())?;

        let Ok(msg) = rx.recv() else {
            return Ok(());
        };
        for msg in std::iter::once(msg).chain(rx.try_iter()) {
            // A resize drag produces a burst, and so do typing a search pattern quickly and
            // dragging a selection: only the last one matters.
            let superseded = matches!(
                (&msg, queue.back()),
                (Msg::Resize { .. }, Some(Msg::Resize { .. }))
                    | (Msg::SearchInput { .. }, Some(Msg::SearchInput { .. }))
                    | (Msg::OutlineFilter { .. }, Some(Msg::OutlineFilter { .. }))
                    | (Msg::Drag { .. }, Some(Msg::Drag { .. }))
            );
            if superseded {
                queue.pop_back();
            }
            match queue
                .back_mut()
                .and_then(|last| coalesce(last, &msg).map(|m| (last, m)))
            {
                Some((last, merged)) => *last = merged,
                None => queue.push_back(msg),
            }
        }
    }
}

/// One message for two in a row that move the same way, so a burst of wheel notches scrolls
/// once a frame. Every move clamps at its ends, so two the same way land where their sum does.
fn coalesce(last: &Msg, next: &Msg) -> Option<Msg> {
    let same = |a: i32, b: i32| a.signum() == b.signum();
    Some(match (last, next) {
        (
            Msg::Scroll {
                by: ScrollBy::Rows(a),
            },
            Msg::Scroll {
                by: ScrollBy::Rows(b),
            },
        ) if same(*a, *b) => Msg::Scroll {
            by: ScrollBy::Rows(a.saturating_add(*b)),
        },
        (Msg::ScrollCodeX { by: a }, Msg::ScrollCodeX { by: b })
            if same(i32::from(*a), i32::from(*b)) =>
        {
            Msg::ScrollCodeX {
                by: a.saturating_add(*b),
            }
        }
        (Msg::OutlineMove { by: a }, Msg::OutlineMove { by: b }) if same(*a, *b) => {
            Msg::OutlineMove {
                by: a.saturating_add(*b),
            }
        }
        (Msg::HelpScroll { by: a }, Msg::HelpScroll { by: b }) if same(*a, *b) => Msg::HelpScroll {
            by: a.saturating_add(*b),
        },
        (Msg::SettingsMove { by: a }, Msg::SettingsMove { by: b }) if same(*a, *b) => {
            Msg::SettingsMove {
                by: a.saturating_add(*b),
            }
        }
        _ => return None,
    })
}

/// Run one command: the terminal's own here, the rest through the effect runner. `mouse` says
/// whether the mouse is captured, to take it again after the editor.
fn run_cmd(
    terminal: &mut Term,
    effects: &mut Effects,
    cmd: Cmd,
    mouse: bool,
) -> Result<Option<Msg>, String> {
    match cmd {
        Cmd::SpawnEditor { file, line } => edit(terminal, &file, line, mouse),
        Cmd::Mouse(on) => {
            capture_mouse(on).map_err(|e| format!("terminal: {e}"))?;
            Ok(None)
        }
        Cmd::Osc52(text) => {
            let mut out = io::stdout();
            let seq = osc52(&text, env::var_os("TMUX").is_some());
            out.write_all(seq.as_bytes())
                .and_then(|()| out.flush())
                .map_err(|e| format!("clipboard: {e}"))?;
            Ok(None)
        }
        Cmd::WriteConfig { path, text } => Ok(Some(match config::write(&path, &text) {
            Ok(()) => Msg::ConfigWritten { path },
            Err(e) => Msg::ConfigFailed {
                error: format!("can't write {}: {e}", path.display()),
            },
        })),
        cmd => Ok(effects.run(cmd)),
    }
}

/// Hand the terminal to the editor (ADR-18): leave the alternate screen, run it in the
/// foreground, then take the screen back, and the mouse when `mouse` says so, and redraw all
/// of it.
fn edit(terminal: &mut Term, file: &Path, line: u32, mouse: bool) -> Result<Option<Msg>, String> {
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
        if mouse {
            capture_mouse(true)?;
        }
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

/// Read the terminal's events into messages: first, when images are on, its answer to the image
/// probe, by [`ask_terminal`] and until `deadline`; then crossterm's events.
fn spawn_input(
    tx: Sender<Msg>,
    keys: Bindings,
    resume: Receiver<bool>,
    images: Option<(images::Plan, Instant)>,
) {
    thread::spawn(move || {
        let mut keymap = Keymap::new(keys);
        if let Some((plan, deadline)) = images
            && !ask_terminal(plan, deadline, &tx, &resume, &mut keymap)
        {
            return;
        }
        loop {
            let msg = match event::read() {
                Ok(Event::Key(key)) => keymap.map(key),
                Ok(Event::Mouse(mouse)) => keymap.mouse(mouse, Instant::now()),
                Ok(Event::Resize(w, h)) => Some(Msg::Resize {
                    size: Size { w, h },
                }),
                Ok(_) => None,
                Err(_) => return,
            };
            if let Some(msg) = msg
                && !send(msg, &tx, &resume, &mut keymap)
            {
                return;
            }
        }
    });
}

/// Read the terminal's answer to the probe's query, written before the first frame, and send what
/// it can show as `Graphics` (CS-2). It reads the tty itself, since crossterm would take the
/// replies for keys; plain keys typed meanwhile go on as they're read. crossterm hasn't started
/// listening for resizes yet, so a resize meanwhile is sent after. False when the main loop is
/// gone.
fn ask_terminal(
    plan: images::Plan,
    deadline: Instant,
    tx: &Sender<Msg>,
    resume: &Receiver<bool>,
    keymap: &mut Keymap,
) -> bool {
    let before = terminal::size().ok();
    let mut gone = false;
    let found = if plan.ask {
        probe::read(deadline, |key| {
            gone = keymap
                .map(key)
                .is_some_and(|msg| !send(msg, tx, resume, keymap));
            !gone
        })
    } else {
        Found::default()
    };
    if gone {
        return false;
    }
    let now = terminal::size().unwrap_or((0, 0));
    let cells = Cells::new(now.0, now.1);
    let env = |name: &str| env::var(name).ok();
    let graphics = images::decide(plan, &found, env, cells, images::winsize_cell());
    if tx.send(graphics.msg()).is_err() {
        return false;
    }
    if before.is_none_or(|b| b == now) || now == (0, 0) {
        return true;
    }
    let size = Size { w: now.0, h: now.1 };
    send(Msg::Resize { size }, tx, resume, keymap)
}

/// Send `msg` to the main loop. After `Edit`, `SaveConfig` and a change in the settings it waits
/// on `resume` for the main loop: until the editor quits, to hear whether the save box opened, and
/// whether the theme list did. False when the main loop is gone.
fn send(msg: Msg, tx: &Sender<Msg>, resume: &Receiver<bool>, keymap: &mut Keymap) -> bool {
    let (edit, save) = (msg == Msg::Edit, msg == Msg::SaveConfig);
    let listing = msg.may_list_themes();
    if tx.send(msg).is_err() {
        return false;
    }
    // While the editor runs, the terminal's input is the editor's.
    if edit && resume.recv().is_err() {
        return false;
    }
    if save {
        match resume.recv() {
            Ok(true) => {}
            Ok(false) => keymap.no_confirm(),
            Err(_) => return false,
        }
    }
    if listing {
        match resume.recv() {
            Ok(open) => keymap.theme_list(open),
            Err(_) => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Dir;

    #[test]
    fn moves_the_same_way_coalesce() {
        let rows = |n| Msg::Scroll {
            by: ScrollBy::Rows(n),
        };
        assert_eq!(coalesce(&rows(3), &rows(3)), Some(rows(6)));
        assert_eq!(coalesce(&rows(-3), &rows(-1)), Some(rows(-4)));
        // back and forth at an end isn't their sum: each clamps
        assert_eq!(coalesce(&rows(3), &rows(-3)), None);
        assert_eq!(coalesce(&rows(i32::MAX), &rows(3)), Some(rows(i32::MAX)));
        let page = Msg::Scroll {
            by: ScrollBy::Page(Dir::Next),
        };
        assert_eq!(coalesce(&page, &page), None);
        assert_eq!(coalesce(&rows(3), &page), None);
        assert_eq!(
            coalesce(&Msg::ScrollCodeX { by: 8 }, &Msg::ScrollCodeX { by: 8 }),
            Some(Msg::ScrollCodeX { by: 16 })
        );
        assert_eq!(
            coalesce(&Msg::OutlineMove { by: 1 }, &Msg::OutlineMove { by: 1 }),
            Some(Msg::OutlineMove { by: 2 })
        );
        assert_eq!(
            coalesce(&Msg::HelpScroll { by: -3 }, &Msg::HelpScroll { by: -3 }),
            Some(Msg::HelpScroll { by: -6 })
        );
        assert_eq!(coalesce(&Msg::Click { col: 1, row: 1 }, &rows(3)), None);
    }
}
