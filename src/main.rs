use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::{env, fs, thread};

use ada::app::{self, Config, Effects, Head, Msg, State};
use ada::cli::{self, Args, Input};
use ada::filetype::{self, FileType};
use ada::git::{self, LineMarks};
use ada::images::{self, ImageMode};
use ada::layout::{ImageFit, LayoutParams, LineNumbers};
use ada::links::{Resolver, RootedResolver};
use ada::print::stream::Start;
use ada::record::{self, Recorder, ReplayOptions};
use ada::theme::Theme;
use ada::{completions, config, links, print, search, theme};

// libc named before std's libm, so `ldexp` (syntect's one libm call) binds to libc's copy and
// `--as-needed` drops libm: ld.so no longer maps it and runs its IFUNC resolvers (~0.3–0.5 MiB of
// every peak, MEM-04)
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[link(name = "c")]
unsafe extern "C" {}

// jemalloc instead of musl's malloc, for Rust and C alike (MEM-09)
#[cfg(all(target_env = "musl", feature = "jemalloc"))]
#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

// One arena, read by jemalloc before main: as fast as its default of four a CPU (each thread's
// cache keeps the lock off the fast path), with a peak up to 36 MiB lower (experiment 47).
// `MALLOC_CONF` still overrides it
#[cfg(all(target_env = "musl", feature = "jemalloc"))]
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
static malloc_conf: &[u8; 10] = b"narenas:1\0";

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
    app::one_arena();
    let args = cli::parse(env::args_os().skip(1))?;
    if args.help {
        print!("{}", cli::usage());
        return Ok(());
    }
    if args.version {
        let (commit, built) = (option_env!("ADA_GIT_COMMIT"), env!("ADA_BUILD_TIME"));
        println!("{}", cli::version(commit, built, option_env!("ADA_LAYOUT")));
        return Ok(());
    }
    if let Some(shell) = &args.completions {
        print!("{}", completions::script(shell).unwrap_or_default());
        return Ok(());
    }

    let printing = args.print || !io::stdout().is_terminal();
    if let (true, None, None, Some(Input::Path(path))) =
        (printing, &args.replay, &args.language, &args.input)
        && links::is_markdown(path)
    {
        print::start_file(path);
    }

    let forced = match &args.language {
        Some(name) => Some(FileType::named(name).ok_or_else(|| {
            format!("--language: `{name}` is no syntax ada knows (md, txt, rs, yaml, …)")
        })?),
        None => None,
    };
    // a print is no session: it reads the config but doesn't write the defaults (REL-15)
    let (settings, file, mut warnings) = config::load(!printing)?;
    let resolver = RootedResolver::new(&settings.links);
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
        theme_bg: settings.theme_bg,
        mouse: settings.mouse,
        copy_on: settings.copy_on,
        table_mode: settings.table_mode,
        code_wrap: args.code_wrap.unwrap_or(settings.code_wrap),
        line_numbers: args.line_numbers.unwrap_or(settings.line_numbers),
        keys: settings.keys,
        file,
        language: None,
    };

    if let Some(path) = &args.replay {
        let opts = ReplayOptions {
            live: args.replay_live,
            headless: args.headless,
            fast: args.replay_fast,
        };
        for w in &warnings {
            eprintln!("ada: warning: {w}");
        }
        let mut effects = Effects::new(None);
        effects.set_resolver(Box::new(resolver));
        return record::replay(path, State::new(theme, config), effects, opts);
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

    if printing {
        for w in &warnings {
            eprintln!("ada: warning: {w}");
        }
        // a print lands in pagers, fzf panes and scrollback, where a background stops at the
        // text: it keeps the terminal's, whatever `theme_bg` says
        return print_doc(&args, &config, &theme.with_bg(false), &resolver, &path);
    }

    let piped = (path == Path::new("-"))
        .then(|| read_stdin(Vec::new(), None, forced))
        .transpose()?;
    // a piped image is shown from a file of its own, which goes when ada quits
    let (stdin, image) = match piped {
        Some(Piped::Text(text)) => (Some(text), None),
        Some(Piped::Image(image)) => (None, Some(image)),
        None => (None, None),
    };
    let path = image.as_ref().map_or(path, |image| image.path.clone());
    if stdin.is_none() {
        let meta = fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.is_dir() {
            return Err(format!("{}: Is a directory", path.display()));
        }
    }
    let recorder = args
        .record
        .as_deref()
        .map(Recorder::create)
        .transpose()
        .map_err(|e| format!("--record: {e}"))?;
    let (themes, theme_warnings) = theme::load_user(&theme);
    warnings.extend(theme_warnings);
    // The alternate screen hides stderr until ada quits; the first warning shows meanwhile.
    for w in &warnings {
        eprintln!("ada: warning: {w}");
    }
    let mut state = State::new(theme, config);
    state.themes = themes;
    state.status = warnings.first().map(|w| format!("warning: {w}"));
    let mut effects = Effects::new(stdin);
    effects.set_resolver(Box::new(resolver));
    app::run_tui(state, path, effects, recorder)
}

/// Rows a preview prints past the line it starts at, when its pane is shorter: room to scroll,
/// and a bound on a huge file, which isn't read past them (plan, "ada as the only fzf previewer").
const PREVIEW_ROWS: usize = 2000;

/// What stdin holds.
enum Piped {
    Text(Arc<str>),
    /// A PNG or JPEG, written to a file: ada's images are read from files.
    Image(PipedImage),
}

/// A piped image's file, alone in a directory only its owner reads, both removed on drop.
struct PipedImage {
    dir: PathBuf,
    path: PathBuf,
}

impl PipedImage {
    /// Write `bytes`, an image of extension `ext` by its magic bytes, once its header shows it
    /// is one: what isn't is an error, and nothing is written. The directory is under
    /// `$XDG_RUNTIME_DIR`, a tmpfs cleared at logout, else the temporary directory.
    fn write(bytes: &[u8], ext: &str) -> Result<PipedImage, String> {
        images::header_of(bytes)
            .map_err(|e| format!("stdin: starts as a {ext} but its header fails: {e}"))?;
        let base = env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute() && dir.is_dir())
            .unwrap_or_else(env::temp_dir);
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        let pid = std::process::id();
        // a directory left by an ada of the same pid that didn't get to remove it is passed over
        let dir = (0..100)
            .map(|n| base.join(format!("ada-{pid}-{n}")))
            .find_map(|dir| match builder.create(&dir) {
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => None,
                created => Some(created.map(|()| dir)),
            })
            .unwrap_or_else(|| Err(io::ErrorKind::AlreadyExists.into()))
            .map_err(|e| format!("stdin: {}: {e}", base.display()))?;
        let image = PipedImage {
            path: dir.join(format!("stdin.{ext}")),
            dir,
        };
        fs::write(&image.path, bytes)
            .map_err(|e| format!("stdin: {}: {e}", image.path.display()))?;
        Ok(image)
    }
}

impl Drop for PipedImage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Read stdin, after the bytes `read` of it already, as [`app::load_doc_head`] reads a file:
/// whole, or with a `head` only the head of what isn't markdown, which the first bytes tell.
/// An image is told by its first bytes and read whole. Other input with a NUL in it isn't shown,
/// unless `-l` names its type.
fn read_stdin(
    read: Vec<u8>,
    head: Option<Head>,
    forced: Option<FileType>,
) -> Result<Piped, String> {
    let err = |e: io::Error| format!("stdin: {e}");
    let mut stdin = io::stdin().lock();
    let mut bytes = read;
    if forced.is_none() {
        (&mut stdin)
            .take(filetype::IMAGE_MAGIC.saturating_sub(bytes.len()) as u64)
            .read_to_end(&mut bytes)
            .map_err(err)?;
        if let Some(ext) = filetype::image_ext(&bytes) {
            stdin.read_to_end(&mut bytes).map_err(err)?;
            return PipedImage::write(&bytes, ext).map(Piped::Image);
        }
    }
    if let Some(head) = head {
        (&mut stdin)
            .take(head.bytes.saturating_sub(bytes.len()) as u64)
            .read_to_end(&mut bytes)
            .map_err(err)?;
        let sniffed = || FileType::of(Path::new("-"), &String::from_utf8_lossy(&bytes));
        if forced.unwrap_or_else(sniffed) != FileType::Markdown {
            app::cut_to_head(&mut bytes, head);
            return text_of(bytes, forced).map(Piped::Text);
        }
    }
    stdin.read_to_end(&mut bytes).map_err(err)?;
    text_of(bytes, forced).map(Piped::Text)
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

fn print_doc(
    args: &Args,
    config: &Config,
    theme: &Theme,
    resolver: &dyn Resolver,
    path: &Path,
) -> Result<(), String> {
    // fzf exports the whole terminal as $COLUMNS, and the preview pane as $FZF_PREVIEW_COLUMNS.
    let env_number = |name| env::var(name).ok()?.trim().parse().ok();
    let tty = io::stdout().is_terminal();
    // Windows' console shows escapes as text until VT processing is on; the TUI gets it from
    // crossterm's first command, and a print calls crossterm for nothing else.
    #[cfg(windows)]
    if tty {
        crossterm::ansi_support::supports_ansi();
    }
    let screen = || tty.then(|| crossterm::terminal::size().ok()).flatten();
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
    // Images only where they reach a terminal, or when asked for by name.
    let to_terminal = tty || preview.is_some() || matches!(args.images, ImageMode::Force(_));
    let proto = to_terminal
        .then(|| images::print_proto(config.images, |name| env::var(name).ok()))
        .flatten();
    let height = preview.or_else(|| screen().map(|(_, h)| h)).unwrap_or(24);
    let cell = proto.map(|_| images::cell_size());
    // Printed code wraps, as there is no scrolling sideways on paper; a preview pane clips it
    // instead when code_wrap is off, and --code-wrap off clips any print.
    let params = LayoutParams {
        width,
        max_width: config.max_width,
        table_mode: config.table_mode,
        code_wrap: args
            .code_wrap
            .unwrap_or(config.code_wrap || preview.is_none()),
        line_numbers: config.line_numbers,
        glyphs_gen: theme.glyphs.generation(),
        images: cell.map(|cell| ImageFit {
            cell,
            max_rows: (height * 3 / 5).max(1),
            screen_rows: height,
        }),
    };
    // an empty pattern (fzf's query before a key) marks nothing; a bad one, typed halfway, warns
    let matches = match args.matching.as_deref().filter(|p| !p.is_empty()) {
        Some(pattern) => search::compile(pattern)
            .inspect_err(|e| eprintln!("ada: warning: --match: {e}"))
            .ok(),
        None => None,
    };
    // A whole log, JSON or text file prints a piece at a time (O-52); a preview reads only its
    // head, and `--line`, `--match` and `--gutter` need the whole layout.
    let mut read = Vec::new();
    if rows.is_none()
        && args.line.is_none()
        && matches.is_none()
        && !args.gutter
        && forced != Some(FileType::Markdown)
    {
        let numbered = FileType::Text { lang: None }.params(params).line_numbers;
        match print::stream::start(path, forced, numbered != LineNumbers::Off) {
            Start::Stream(stream) => return stream.run(theme, params).map_err(|e| e.to_string()),
            Start::Whole(bytes) => read = bytes,
        }
    }
    // a piped image prints from a file of its own, removed once it's printed
    let mut image = None;
    let piped = path == Path::new("-");
    let (path, text) = if piped {
        match read_stdin(read, head, forced)? {
            Piped::Text(text) => (path, text),
            Piped::Image(piped) => (&*image.insert(piped).path, Arc::from("")),
        }
    } else {
        match app::load_doc_head(path, None, head) {
            Msg::DocLoaded { text, .. } => (path, text),
            Msg::DocFailed { path, error } => return Err(format!("{}: {error}", path.display())),
            other => return Err(format!("unexpected {other:?}")),
        }
    };
    let filetype = forced.unwrap_or_else(|| FileType::of(path, &text));
    print::start((filetype == FileType::Markdown).then_some(&*text));
    // git runs while the document is parsed and laid out; never for stdin or an image
    // (architecture §8)
    let diffs = (args.gutter && !piped && filetype != FileType::Image).then(|| {
        thread::spawn({
            let path = path.to_owned();
            move || git::run(&path)
        })
    });
    let doc = filetype.parse(path, &text);
    // fzf redraws its preview pane for every file: its images can replace the last file's
    let reuse_ids = env::var_os("FZF_PREVIEW_LINES").is_some();
    let pictures = proto
        .zip(cell)
        .map(|(proto, cell)| print::Pictures::find(&doc, path, resolver, proto, cell, reuse_ids));
    let gutter = diffs
        .and_then(|git| git.join().ok().flatten())
        .map(|d| LineMarks::new(&d.total, &d.unstaged, d.untracked));
    let opts = print::Options {
        align: args.line,
        rows,
        images: pictures.as_ref(),
        matches: matches.as_ref(),
        gutter: gutter.as_ref(),
    };
    let result = print::run(&doc, theme, filetype.params(params), opts);
    // ada exits next: the document is left for the OS to reclaim instead of freed (O-31).
    std::mem::forget(doc);
    std::mem::forget(text);
    result.map_err(|e| e.to_string())
}
