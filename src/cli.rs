//! Hand-rolled flag parsing (ADR-10).

use std::ffi::OsString;
use std::path::PathBuf;

use crate::images::{ImageMode, Proto};
use crate::layout::LineNumbers;

/// What a flag's value completes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value {
    /// No value: a switch.
    None,
    Number,
    Theme,
    /// A language: what [`language_names`](crate::filetype::language_names) lists.
    Language,
    File,
    /// A recording.
    Rec,
    Choice(&'static [&'static str]),
}

pub struct Flag {
    pub short: Option<char>,
    pub long: &'static str,
    /// The value's name in the usage, and what it completes to.
    pub value: (&'static str, Value),
    /// The usage's description; `\n` continues it on the next line.
    pub help: &'static str,
}

const fn flag(
    short: Option<char>,
    long: &'static str,
    value: (&'static str, Value),
    help: &'static str,
) -> Flag {
    Flag {
        short,
        long,
        value,
        help,
    }
}

const SWITCH: (&str, Value) = ("", Value::None);

pub const SHELLS: &[&str] = &["zsh", "bash", "fish"];
/// Every flag, in the order the usage lists them. The parser, the usage and the completion
/// scripts all follow this table.
pub const FLAGS: &[Flag] = &[
    flag(
        Some('p'),
        "print",
        SWITCH,
        "write the rendered document to stdout and exit",
    ),
    flag(
        Some('w'),
        "width",
        ("N", Value::Number),
        "print width (default: $FZF_PREVIEW_COLUMNS, $COLUMNS, then 80)",
    ),
    flag(
        None,
        "height",
        ("N", Value::Number),
        "print for a preview N rows high: images fit it, and a long\nfile is cut (default: $FZF_PREVIEW_LINES)",
    ),
    flag(
        None,
        "max-width",
        ("N", Value::Number),
        "wrap prose at most N columns wide",
    ),
    flag(
        Some('l'),
        "language",
        ("NAME", Value::Language),
        "show the file as language NAME instead of what ada detects\n(a syntax name or extension: md for markdown, txt for plain text)",
    ),
    flag(
        Some('L'),
        "line",
        ("N", Value::Number),
        "start at source line N; when printing, shift the output so\nthat line N renders on output line N (for fzf previews)",
    ),
    flag(
        Some('t'),
        "theme",
        ("NAME", Value::Theme),
        "a theme name or a path to a .toml theme",
    ),
    flag(
        Some('n'),
        "line-numbers",
        ("WHEN", Value::Choice(&["off", "code", "all"])),
        "number rows by source line: off, code (code and other text\nfiles) or all (markdown too)",
    ),
    flag(
        None,
        "no-images",
        SWITCH,
        "show images as text, without asking the terminal what it supports",
    ),
    flag(
        None,
        "image-protocol",
        (
            "P",
            Value::Choice(&["kitty", "sixel", "iterm2", "halfblocks"]),
        ),
        "kitty, sixel, iterm2 or halfblocks, not what the terminal reports",
    ),
    flag(
        None,
        "no-reload",
        SWITCH,
        "don't reload when the file changes on disk",
    ),
    flag(
        None,
        "record",
        ("FILE", Value::File),
        "record the session to FILE",
    ),
    flag(
        None,
        "replay",
        ("FILE", Value::Rec),
        "replay a recorded session",
    ),
    flag(
        None,
        "replay-live",
        SWITCH,
        "with --replay: re-run effects instead of using recorded results",
    ),
    flag(
        None,
        "replay-fast",
        SWITCH,
        "with --replay: don't wait between messages; the last frame stays until a key",
    ),
    flag(
        None,
        "headless",
        SWITCH,
        "with --replay: render off-screen and print the final frame",
    ),
    flag(
        None,
        "completions",
        ("SHELL", Value::Choice(SHELLS)),
        "print a completion script for zsh, bash or fish",
    ),
    flag(Some('h'), "help", SWITCH, "show this help"),
    flag(Some('V'), "version", SWITCH, "show the version"),
];

/// Where the usage's descriptions start.
const HELP_COLUMN: usize = 24;

pub fn usage() -> String {
    let mut out = String::from(
        "\
usage: ada [options] [FILE | -]

A terminal markdown viewer, which also shows code, config and other text files highlighted,
and images. Reads stdin when FILE is `-`, or is omitted and stdin is a pipe, and tells
markdown, code, data, diffs and logs apart by what it holds (else plain text; -l names it).
Prints instead of paging when stdout is not a terminal. Options are also read from
~/.config/ada/config.toml (written with the defaults on first run); flags win.

options:
",
    );
    for f in FLAGS {
        let short = f
            .short
            .map_or_else(|| "    ".to_owned(), |c| format!("-{c}, "));
        let value = if f.value.0.is_empty() {
            String::new()
        } else {
            format!(" {}", f.value.0)
        };
        let head = format!("  {short}--{}{value}", f.long);
        let mut lines = f.help.split('\n');
        let first = lines.next().unwrap_or_default();
        if head.len() < HELP_COLUMN {
            out.push_str(&format!("{head:<HELP_COLUMN$}{first}\n"));
        } else {
            out.push_str(&format!("{head}\n{:HELP_COLUMN$}{first}\n", ""));
        }
        for line in lines {
            out.push_str(&format!("{:HELP_COLUMN$}{line}\n", ""));
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Stdin,
    Path(PathBuf),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    pub input: Option<Input>,
    pub print: bool,
    pub width: Option<u16>,
    pub height: Option<u16>,
    pub max_width: Option<u16>,
    /// A language name, checked when it's used: the syntaxes load then.
    pub language: Option<String>,
    pub line: Option<u32>,
    pub theme: Option<String>,
    pub line_numbers: Option<LineNumbers>,
    pub images: ImageMode,
    pub no_reload: bool,
    pub record: Option<PathBuf>,
    pub replay: Option<PathBuf>,
    pub replay_live: bool,
    pub replay_fast: bool,
    pub headless: bool,
    pub completions: Option<String>,
    pub help: bool,
    pub version: bool,
}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Args, String> {
    let mut out = Args::default();
    let mut args = args.into_iter();
    let mut only_files = false;
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy();
        if only_files || text == "-" || !text.starts_with('-') {
            if out.input.is_some() {
                return Err(format!("unexpected argument `{text}`: ada shows one file"));
            }
            out.input = Some(if text == "-" {
                Input::Stdin
            } else {
                Input::Path(PathBuf::from(arg))
            });
            continue;
        }
        if text == "--" {
            only_files = true;
            continue;
        }
        let (flag, inline) = match text.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => {
                (flag.to_owned(), Some(value.to_owned()))
            }
            _ => (text.into_owned(), None),
        };
        let mut value = |name: &str| -> Result<String, String> {
            inline
                .clone()
                .or_else(|| args.next().map(|v| v.to_string_lossy().into_owned()))
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match flag.as_str() {
            "-p" | "--print" => out.print = true,
            "-w" | "--width" => out.width = Some(number(&flag, &value(&flag)?)?),
            "--height" => out.height = Some(number(&flag, &value(&flag)?)?),
            "--max-width" => out.max_width = Some(number(&flag, &value(&flag)?)?),
            "-l" | "--language" => out.language = Some(value(&flag)?),
            "-L" | "--line" => out.line = Some(number(&flag, &value(&flag)?)?),
            "-t" | "--theme" => out.theme = Some(value(&flag)?),
            "-n" | "--line-numbers" => {
                let when = value(&flag)?;
                let numbers = LineNumbers::parse(&when)
                    .ok_or_else(|| format!("{flag}: `{when}` is not off, code or all"))?;
                out.line_numbers = Some(numbers);
            }
            "--no-images" => out.images = ImageMode::Off,
            "--image-protocol" => {
                let name = value(&flag)?;
                let proto = Proto::parse(&name).ok_or_else(|| {
                    format!("{flag}: `{name}` is not kitty, sixel, iterm2 or halfblocks")
                })?;
                out.images = ImageMode::Force(proto);
            }
            "--no-reload" => out.no_reload = true,
            "--record" => out.record = Some(value(&flag)?.into()),
            "--replay" => out.replay = Some(value(&flag)?.into()),
            "--replay-live" => out.replay_live = true,
            "--replay-fast" => out.replay_fast = true,
            "--headless" => out.headless = true,
            "--completions" => {
                let shell = value(&flag)?;
                if !SHELLS.contains(&shell.as_str()) {
                    return Err(format!("{flag}: `{shell}` is not zsh, bash or fish"));
                }
                out.completions = Some(shell);
            }
            "-h" | "--help" => out.help = true,
            "-V" | "--version" => out.version = true,
            _ => return Err(format!("unknown option `{flag}` (try --help)")),
        }
        let switch = FLAGS
            .iter()
            .any(|f| f.value.1 == Value::None && flag == format!("--{}", f.long));
        if inline.is_some() && switch {
            return Err(format!("{flag} takes no value"));
        }
    }
    if (out.replay_live || out.replay_fast || out.headless) && out.replay.is_none() {
        return Err("--replay-live, --replay-fast and --headless need --replay FILE".into());
    }
    if out.replay.is_some() && out.record.is_some() {
        return Err("--record and --replay can't be combined".into());
    }
    Ok(out)
}

fn number<T: std::str::FromStr>(flag: &str, value: &str) -> Result<T, String> {
    value
        .parse()
        .ok()
        .filter(|_| !value.starts_with('-'))
        .ok_or_else(|| format!("{flag}: `{value}` is not a positive number"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Args, String> {
        parse(list.iter().map(OsString::from))
    }

    #[test]
    fn files_and_flags() {
        let a = args(&[
            "--width=100",
            "-t",
            "mine",
            "notes/a.md",
            "--max-width",
            "90",
            "--height",
            "30",
        ])
        .unwrap();
        assert_eq!(a.input, Some(Input::Path("notes/a.md".into())));
        assert_eq!(a.width, Some(100));
        assert_eq!(a.height, Some(30));
        assert_eq!(a.max_width, Some(90));
        assert_eq!(a.theme.as_deref(), Some("mine"));
        assert_eq!(args(&["-"]).unwrap().input, Some(Input::Stdin));
        assert_eq!(
            args(&["--", "-dash.md"]).unwrap().input,
            Some(Input::Path("-dash.md".into()))
        );
    }

    #[test]
    fn errors() {
        assert!(args(&["--width"]).is_err());
        assert!(args(&["--width", "-3"]).is_err());
        assert!(args(&["--frobnicate"]).is_err());
        assert!(args(&["a.md", "b.md"]).is_err());
        assert!(args(&["--headless"]).is_err());
        assert!(args(&["--print=yes"]).is_err());
        assert!(args(&["--image-protocol", "chafa"]).is_err());
        assert!(args(&["-n", "yes"]).is_err());
    }

    #[test]
    fn the_usage_lists_every_flag_the_parser_takes() {
        let usage = usage();
        for f in FLAGS {
            assert!(usage.contains(&format!("--{}", f.long)), "{}", f.long);
            let value = (f.value.1 != Value::None).then_some("zsh");
            let parsed = match f.long {
                "replay-live" | "replay-fast" | "headless" => {
                    args(&["--replay", "x.rec", &format!("--{}", f.long)])
                }
                long => args(
                    &[&format!("--{long}")]
                        .into_iter()
                        .map(String::as_str)
                        .chain(value)
                        .collect::<Vec<_>>(),
                ),
            };
            let err = parsed.err().unwrap_or_default();
            assert!(!err.contains("unknown option"), "--{}: {err}", f.long);
        }
        assert!(usage.contains(
            "  -L, --line N          start at source line N; when printing, shift the output so\n"
        ));
        assert!(usage.contains("      --image-protocol P\n                        kitty, sixel"));
    }

    #[test]
    fn language_and_line() {
        let a = args(&["-l", "json", "-L", "12", "-"]).unwrap();
        assert_eq!(a.language.as_deref(), Some("json"));
        assert_eq!(a.line, Some(12));
        let a = args(&["--language=md", "--line", "3"]).unwrap();
        assert_eq!(a.language.as_deref(), Some("md"));
        assert_eq!(a.line, Some(3));
        assert!(args(&["-l"]).is_err());
    }

    #[test]
    fn line_numbers() {
        assert_eq!(args(&[]).unwrap().line_numbers, None);
        assert_eq!(
            args(&["-n", "all", "a.md"]).unwrap().line_numbers,
            Some(LineNumbers::All)
        );
        assert_eq!(
            args(&["--line-numbers=off"]).unwrap().line_numbers,
            Some(LineNumbers::Off)
        );
    }

    #[test]
    fn image_flags() {
        assert_eq!(args(&[]).unwrap().images, ImageMode::Auto);
        assert_eq!(args(&["--no-images"]).unwrap().images, ImageMode::Off);
        assert_eq!(
            args(&["--image-protocol=sixel"]).unwrap().images,
            ImageMode::Force(Proto::Sixel)
        );
    }
}
