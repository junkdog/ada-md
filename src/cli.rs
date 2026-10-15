//! Hand-rolled flag parsing (ADR-10).

use std::ffi::OsString;
use std::path::PathBuf;

use crate::images::{ImageMode, Proto};

pub const USAGE: &str = "\
usage: preen [options] [FILE | -]

A terminal markdown viewer. Reads stdin when FILE is `-`, or is omitted and stdin is a pipe.
Prints instead of paging when stdout is not a terminal.

options:
  -p, --print           write the rendered document to stdout and exit
  -w, --width N         print width (default: $FZF_PREVIEW_COLUMNS, $COLUMNS, then 80)
      --max-width N     wrap prose at most N columns wide
  -l, --line N          start at source line N; when printing, shift the output so
                        that line N renders on output line N (for fzf previews)
  -t, --theme NAME      a theme name or a path to a .toml theme
      --no-images       show images as text, without asking the terminal what it supports
      --image-protocol P
                        kitty, sixel, iterm2 or halfblocks, instead of what the terminal
                        reports (kitty is otherwise never used inside tmux)
      --record FILE     record the session to FILE
      --replay FILE     replay a recorded session
      --replay-live     with --replay: re-run effects instead of using recorded results
      --headless        with --replay: render off-screen and print the final frame
  -h, --help            show this help
  -V, --version         show the version
";

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
    pub max_width: Option<u16>,
    pub line: Option<u32>,
    pub theme: Option<String>,
    pub images: ImageMode,
    pub record: Option<PathBuf>,
    pub replay: Option<PathBuf>,
    pub replay_live: bool,
    pub headless: bool,
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
                return Err(format!(
                    "unexpected argument `{text}`: preen shows one file"
                ));
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
            "--max-width" => out.max_width = Some(number(&flag, &value(&flag)?)?),
            "-l" | "--line" => out.line = Some(number(&flag, &value(&flag)?)?),
            "-t" | "--theme" => out.theme = Some(value(&flag)?),
            "--no-images" => out.images = ImageMode::Off,
            "--image-protocol" => {
                let name = value(&flag)?;
                let proto = Proto::parse(&name).ok_or_else(|| {
                    format!("{flag}: `{name}` is not kitty, sixel, iterm2 or halfblocks")
                })?;
                out.images = ImageMode::Force(proto);
            }
            "--record" => out.record = Some(value(&flag)?.into()),
            "--replay" => out.replay = Some(value(&flag)?.into()),
            "--replay-live" => out.replay_live = true,
            "--headless" => out.headless = true,
            "-h" | "--help" => out.help = true,
            "-V" | "--version" => out.version = true,
            _ => return Err(format!("unknown option `{flag}` (try --help)")),
        }
        if inline.is_some()
            && matches!(
                flag.as_str(),
                "--print" | "--replay-live" | "--headless" | "--no-images" | "--help" | "--version"
            )
        {
            return Err(format!("{flag} takes no value"));
        }
    }
    if (out.replay_live || out.headless) && out.replay.is_none() {
        return Err("--replay-live and --headless need --replay FILE".into());
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
        ])
        .unwrap();
        assert_eq!(a.input, Some(Input::Path("notes/a.md".into())));
        assert_eq!(a.width, Some(100));
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
