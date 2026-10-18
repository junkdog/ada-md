use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::{env, fs};

use ada::app::{self, Config, Effects, Msg, State};
use ada::cli::{self, Args, Input};
use ada::filetype::FileType;
use ada::images::ImageMode;
use ada::layout::LayoutParams;
use ada::record::{self, Recorder, ReplayOptions};
use ada::theme::Theme;
use ada::{completions, config, print, theme};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ada: {e}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), String> {
    let args = cli::parse(env::args_os().skip(1))?;
    if args.help {
        print!("{}", cli::usage());
        return Ok(());
    }
    if args.version {
        println!("ada {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if let Some(shell) = &args.completions {
        print!("{}", completions::script(shell).unwrap_or_default());
        return Ok(());
    }

    let (settings, file, mut warnings) = config::load()?;
    let (theme, theme_warnings) = theme::load(args.theme.as_deref().or(settings.theme.as_deref()))?;
    warnings.extend(theme_warnings);
    let config = Config {
        max_width: args.max_width.or(settings.max_width),
        start_line: args.line,
        images: match args.images {
            ImageMode::Auto => settings.images,
            flag => flag,
        },
        live_reload: settings.live_reload && !args.no_reload,
        table_mode: settings.table_mode,
        code_wrap: settings.code_wrap,
        line_numbers: args.line_numbers.unwrap_or(settings.line_numbers),
        keys: settings.keys,
        file,
    };

    if let Some(path) = &args.replay {
        let opts = ReplayOptions {
            live: args.replay_live,
            headless: args.headless,
        };
        for w in &warnings {
            eprintln!("ada: warning: {w}");
        }
        return record::replay(path, State::new(theme, config), Effects::new(None), opts);
    }

    let input = match &args.input {
        Some(input) => input.clone(),
        None if !io::stdin().is_terminal() => Input::Stdin,
        None => return Err(format!("no file given\n\n{}", cli::usage())),
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
        for w in &warnings {
            eprintln!("ada: warning: {w}");
        }
        return print_doc(&args, &config, &theme, &path, stdin.as_ref());
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
    let (themes, theme_warnings) = theme::load_all(&theme);
    warnings.extend(theme_warnings);
    // The alternate screen hides stderr until ada quits; the first warning shows meanwhile.
    for w in &warnings {
        eprintln!("ada: warning: {w}");
    }
    let mut state = State::new(theme, config);
    state.themes = themes;
    state.status = warnings.first().map(|w| format!("warning: {w}"));
    app::run_tui(state, path, Effects::new(stdin), recorder)
}

fn print_doc(
    args: &Args,
    config: &Config,
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
    // Printed code always wraps: there is no scrolling sideways on paper.
    let params = LayoutParams {
        width,
        max_width: config.max_width,
        table_mode: config.table_mode,
        code_wrap: true,
        line_numbers: config.line_numbers,
        glyphs_gen: theme.glyphs.generation(),
        images: None,
    };
    let filetype = FileType::of(path, &text);
    let doc = filetype.parse(path, &text);
    let result = print::run(&doc, theme, filetype.params(params), args.line);
    // ada exits next: the document is left for the OS to reclaim instead of freed (O-31).
    std::mem::forget(doc);
    std::mem::forget(text);
    result.map_err(|e| e.to_string())
}
