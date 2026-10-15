use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::{env, fs};

use preen::app::{self, Config, Effects, Msg, State};
use preen::cli::{self, Args, Input};
use preen::layout::{LayoutParams, TableMode};
use preen::record::{self, Recorder, ReplayOptions};
use preen::theme::Theme;
use preen::{doc, print, theme};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("preen: {e}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), String> {
    let args = cli::parse(env::args_os().skip(1))?;
    if args.help {
        print!("{}", cli::USAGE);
        return Ok(());
    }
    if args.version {
        println!("preen {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let (theme, warnings) = theme::load(args.theme.as_deref())?;
    for w in &warnings {
        eprintln!("preen: warning: {w}");
    }
    let config = Config {
        max_width: args.max_width,
        start_line: args.line,
    };

    if let Some(path) = &args.replay {
        let opts = ReplayOptions {
            live: args.replay_live,
            headless: args.headless,
        };
        return record::replay(path, State::new(theme, config), Effects::new(None), opts);
    }

    let input = match &args.input {
        Some(input) => input.clone(),
        None if !io::stdin().is_terminal() => Input::Stdin,
        None => return Err(format!("no file given\n\n{}", cli::USAGE)),
    };
    let (path, stdin) = match input {
        Input::Stdin => {
            let mut bytes = Vec::new();
            io::stdin()
                .read_to_end(&mut bytes)
                .map_err(|e| format!("stdin: {e}"))?;
            let text = String::from_utf8_lossy(&bytes).into_owned();
            (PathBuf::from("-"), Some(Arc::from(text)))
        }
        Input::Path(path) => (path, None),
    };

    if args.print || !io::stdout().is_terminal() {
        return print_doc(&args, &theme, &path, stdin.as_ref());
    }

    if stdin.is_none() {
        fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let recorder = args
        .record
        .as_deref()
        .map(Recorder::create)
        .transpose()
        .map_err(|e| format!("--record: {e}"))?;
    let mut state = State::new(theme, config);
    state.status = warnings.first().map(|w| format!("warning: {w}"));
    app::run_tui(state, path, Effects::new(stdin), recorder)
}

fn print_doc(
    args: &Args,
    theme: &Theme,
    path: &Path,
    stdin: Option<&Arc<str>>,
) -> Result<(), String> {
    let text = match app::load_doc(path, stdin) {
        Msg::DocLoaded { text, .. } => text,
        Msg::DocFailed { path, error } => return Err(format!("{}: {error}", path.display())),
        other => return Err(format!("unexpected {other:?}")),
    };
    // fzf exports the whole terminal as $COLUMNS, and the preview pane as $FZF_PREVIEW_COLUMNS.
    let env_width = |name| env::var(name).ok()?.trim().parse().ok();
    let width = args
        .width
        .or_else(|| env_width("FZF_PREVIEW_COLUMNS"))
        .or_else(|| env_width("COLUMNS"))
        .or_else(|| {
            io::stdout()
                .is_terminal()
                .then(|| ratatui::crossterm::terminal::size().ok().map(|(w, _)| w))
                .flatten()
        })
        .unwrap_or(80)
        .max(1);
    let params = LayoutParams {
        width,
        max_width: args.max_width,
        table_mode: TableMode::Auto,
        code_wrap: true,
        glyphs_gen: theme.glyphs.generation(),
    };
    print::run(&doc::parse(&text), theme, params, args.line).map_err(|e| e.to_string())
}
