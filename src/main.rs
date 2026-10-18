use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::{env, fs};

use ada::app::{self, Config, Effects, Head, Msg, State};
use ada::cli::{self, Args, Input};
use ada::filetype::{self, FileType};
use ada::images::{self, ImageMode};
use ada::layout::{ImageFit, LayoutParams};
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

    let forced = match &args.language {
        Some(name) => Some(FileType::named(name).ok_or_else(|| {
            format!("--language: `{name}` is no syntax ada knows (md, txt, rs, yaml, …)")
        })?),
        None => None,
    };
    let (settings, file, mut warnings) = config::load()?;
    let (theme, theme_warnings) = theme::load(args.theme.as_deref().or(settings.theme.as_deref()))?;
    warnings.extend(theme_warnings);
    let mut config = Config {
        max_width: args.max_width.or(settings.max_width),
        start_line: args.line,
        images: match args.images {
            ImageMode::Auto => settings.images,
            flag => flag,
        },
        live_reload: settings.live_reload && !args.no_reload,
        keep_screen: settings.keep_screen,
        table_mode: settings.table_mode,
        code_wrap: settings.code_wrap,
        line_numbers: args.line_numbers.unwrap_or(settings.line_numbers),
        keys: settings.keys,
        file,
        language: None,
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
    let path = match input {
        Input::Stdin => PathBuf::from("-"),
        Input::Path(path) => path,
    };
    config.language = forced.map(|t| (path.clone(), t));

    if args.print || !io::stdout().is_terminal() {
        for w in &warnings {
            eprintln!("ada: warning: {w}");
        }
        return print_doc(&args, &config, &theme, &path);
    }

    let stdin = (path == Path::new("-"))
        .then(|| read_stdin(None, forced))
        .transpose()?;
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

/// Rows a preview prints past the line it starts at, when its pane is shorter: room to scroll,
/// and a bound on a huge file, which isn't read past them (plan, "ada as the only fzf previewer").
const PREVIEW_ROWS: usize = 2000;

/// Read stdin, as [`app::load_doc_head`] reads a file: whole, or with a `head` only the head of
/// what isn't markdown, which the first bytes tell. Input with a NUL in it isn't shown, unless
/// `-l` names its type.
fn read_stdin(head: Option<Head>, forced: Option<FileType>) -> Result<Arc<str>, String> {
    let err = |e: io::Error| format!("stdin: {e}");
    let mut stdin = io::stdin().lock();
    let mut bytes = Vec::new();
    if let Some(head) = head {
        (&mut stdin)
            .take(head.bytes as u64)
            .read_to_end(&mut bytes)
            .map_err(err)?;
        let sniffed = || FileType::of(Path::new("-"), &String::from_utf8_lossy(&bytes));
        if forced.unwrap_or_else(sniffed) != FileType::Markdown {
            app::cut_to_head(&mut bytes, head);
            return text_of(bytes, forced);
        }
    }
    stdin.read_to_end(&mut bytes).map_err(err)?;
    text_of(bytes, forced)
}

fn text_of(bytes: Vec<u8>, forced: Option<FileType>) -> Result<Arc<str>, String> {
    if forced.is_none() && filetype::is_binary(&bytes) {
        return Err("stdin: binary input (-l names a type to show it anyway)".into());
    }
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    };
    Ok(text.into())
}

fn print_doc(args: &Args, config: &Config, theme: &Theme, path: &Path) -> Result<(), String> {
    // fzf exports the whole terminal as $COLUMNS, and the preview pane as $FZF_PREVIEW_COLUMNS.
    let env_number = |name| env::var(name).ok()?.trim().parse().ok();
    let tty = io::stdout().is_terminal();
    let screen = || {
        tty.then(|| ratatui::crossterm::terminal::size().ok())
            .flatten()
    };
    let width = args
        .width
        .or_else(|| env_number("FZF_PREVIEW_COLUMNS"))
        .or_else(|| env_number("COLUMNS"))
        .or_else(|| screen().map(|(w, _)| w))
        .unwrap_or(80)
        .max(1);
    // A preview pane's height bounds what's read and printed.
    let preview: Option<u16> = args
        .height
        .or_else(|| env_number("FZF_PREVIEW_LINES"))
        .map(|h| h.max(1));
    let rows = preview.map(|h| usize::from(h).max(PREVIEW_ROWS));
    let forced = config.forced_type(path);
    // a note is read whole, `-l md` or not
    let head = rows
        .filter(|_| forced != Some(FileType::Markdown))
        .map(|rows| {
            // each line takes a row at least, and a row's text is rarely over 4 bytes a column
            let lines = args.line.map_or(0, |l| l as usize) + rows;
            Head {
                lines,
                bytes: (lines * (usize::from(width) + 1) * 4).max(1 << 16),
            }
        });
    let text = if path == Path::new("-") {
        read_stdin(head, forced)?
    } else {
        match app::load_doc_head(path, None, head) {
            Msg::DocLoaded { text, .. } => text,
            Msg::DocFailed { path, error } => return Err(format!("{}: {error}", path.display())),
            other => return Err(format!("unexpected {other:?}")),
        }
    };
    // Images only where they reach a terminal, or when asked for by name.
    let to_terminal = tty || preview.is_some() || matches!(args.images, ImageMode::Force(_));
    let proto = to_terminal
        .then(|| images::print_proto(config.images, |name| env::var(name).ok()))
        .flatten();
    let height = preview.or_else(|| screen().map(|(_, h)| h)).unwrap_or(24);
    let cell = proto.map(|_| images::cell_size());
    // Printed code always wraps: there is no scrolling sideways on paper.
    let params = LayoutParams {
        width,
        max_width: config.max_width,
        table_mode: config.table_mode,
        code_wrap: true,
        line_numbers: config.line_numbers,
        glyphs_gen: theme.glyphs.generation(),
        images: cell.map(|cell| ImageFit {
            cell,
            max_rows: (height * 3 / 5).max(1),
            screen_rows: height,
        }),
    };
    let filetype = forced.unwrap_or_else(|| FileType::of(path, &text));
    let doc = filetype.parse(path, &text);
    // fzf redraws its preview pane for every file: its images can replace the last file's
    let reuse_ids = env::var_os("FZF_PREVIEW_LINES").is_some();
    let pictures = proto
        .zip(cell)
        .map(|(proto, cell)| print::Pictures::find(&doc, path, proto, cell, reuse_ids));
    let opts = print::Options {
        align: args.line,
        rows,
        images: pictures.as_ref(),
    };
    let result = print::run(&doc, theme, filetype.params(params), opts);
    // ada exits next: the document is left for the OS to reclaim instead of freed (O-31).
    std::mem::forget(doc);
    std::mem::forget(text);
    result.map_err(|e| e.to_string())
}
