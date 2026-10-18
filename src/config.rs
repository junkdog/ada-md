//! The config file, `$XDG_CONFIG_HOME/ada/config.toml` (P7). Optional: when it doesn't exist,
//! ada writes it with every option commented out at its default, so the options can be found.
//! Walked by hand like themes (ADR-08): unknown keys are warnings, bad values are errors that
//! point at the value. Command-line flags win over it.

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use crate::app::{Action, Bindings, KeySeq};
use crate::images::{ImageMode, Proto};
use crate::layout::{LineNumbers, TableMode};
use crate::toml_walk::{self, Cx};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub theme: Option<String>,
    pub max_width: Option<u16>,
    pub images: ImageMode,
    pub live_reload: bool,
    pub keep_screen: bool,
    pub table_mode: TableMode,
    pub code_wrap: bool,
    pub line_numbers: LineNumbers,
    pub keys: Bindings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: None,
            max_width: None,
            images: ImageMode::Auto,
            live_reload: true,
            keep_screen: true,
            table_mode: TableMode::Auto,
            code_wrap: true,
            line_numbers: LineNumbers::Off,
            keys: Bindings::default(),
        }
    }
}

/// Where the config file lives.
pub fn path() -> Option<PathBuf> {
    Some(toml_walk::config_dir()?.join("config.toml"))
}

/// Read the config file, or write the commented defaults when there is none. Returns the
/// settings, the file, and warnings (a failed write is one).
pub fn load() -> Result<(Settings, Option<PathBuf>, Vec<String>), String> {
    let mut warnings = Vec::new();
    let Some(path) = path() else {
        return Ok((Settings::default(), None, warnings));
    };
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let written = path
                .parent()
                .map_or(Ok(()), fs::create_dir_all)
                .and_then(|()| fs::write(&path, default_text()));
            if let Err(e) = written {
                warnings.push(format!("can't write {}: {e}", path.display()));
            }
            return Ok((Settings::default(), Some(path), warnings));
        }
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let settings = parse(&path.display().to_string(), &text, &mut warnings)?;
    Ok((settings, Some(path), warnings))
}

pub fn parse(file: &str, text: &str, warnings: &mut Vec<String>) -> Result<Settings, String> {
    let root = toml_walk::parse(file, text)?;
    let mut cx = Cx {
        file,
        text,
        warnings,
    };
    let mut s = Settings::default();
    let mut protocol = None;
    let mut images = true;
    for (k, v) in root.get_ref() {
        match k.get_ref().as_ref() {
            "theme" => s.theme = Some(cx.string(v)?.to_owned()),
            "max_width" => s.max_width = Some(cx.u16(v)?).filter(|&w| w > 0),
            "images" => images = cx.bool(v)?,
            "image_protocol" => match cx.string(v)? {
                "auto" => protocol = None,
                name => {
                    let proto = Proto::parse(name).ok_or_else(|| {
                        cx.at(
                            v.span(),
                            format_args!(
                                "unknown image protocol \"{name}\" (auto, kitty, sixel, iterm2 or halfblocks)"
                            ),
                        )
                    })?;
                    protocol = Some(proto);
                }
            },
            "live_reload" => s.live_reload = cx.bool(v)?,
            "keep_screen" => s.keep_screen = cx.bool(v)?,
            "tables" => {
                s.table_mode = match cx.string(v)? {
                    "auto" => TableMode::Auto,
                    "grid" => TableMode::Grid,
                    "records" => TableMode::Records,
                    other => {
                        return Err(cx.at(
                            v.span(),
                            format_args!(
                                "unknown table layout \"{other}\" (auto, grid or records)"
                            ),
                        ));
                    }
                };
            }
            "code_wrap" => s.code_wrap = cx.bool(v)?,
            "line_numbers" => {
                let name = cx.string(v)?;
                s.line_numbers = LineNumbers::parse(name).ok_or_else(|| {
                    cx.at(
                        v.span(),
                        format_args!("unknown line numbers \"{name}\" (off, code or all)"),
                    )
                })?;
            }
            "keys" => {
                let mut set = Vec::new();
                for (k, v) in cx.table(v)? {
                    let name = k.get_ref().as_ref();
                    let Some(action) = Action::from_name(name) else {
                        cx.warn(k.span(), format_args!("unknown action \"{name}\""));
                        continue;
                    };
                    let mut seqs = Vec::new();
                    for spec in cx.one_or_more(v)? {
                        let seq = KeySeq::parse(&spec).ok_or_else(|| {
                            cx.at(v.span(), format_args!("can't read the key \"{spec}\""))
                        })?;
                        seqs.push(seq);
                    }
                    set.push((action, seqs));
                }
                for w in s.keys.rebind(set) {
                    cx.warnings.push(format!("{file}: {w}"));
                }
            }
            other => cx.warn(k.span(), format_args!("unknown key \"{other}\"")),
        }
    }
    s.images = match (images, protocol) {
        (false, _) => ImageMode::Off,
        (true, Some(proto)) => ImageMode::Force(proto),
        (true, None) => ImageMode::Auto,
    };
    Ok(s)
}

/// The file ada writes when there is none: every option, commented out at its default.
pub fn default_text() -> String {
    let mut text = String::from(
        "\
# ada's configuration. Every option is shown commented out, at its default value;
# command-line flags win over what is set here.

# The theme: a built-in name (gruvbox-dark, gruvbox-light, catppuccin-mocha, catppuccin-macchiato,
# catppuccin-frappe, catppuccin-latte, tokyonight-night, tokyonight-storm, tokyonight-moon,
# tokyonight-day, rose-pine, rose-pine-moon, rose-pine-dawn, dracula), the name of a theme in
# ~/.config/ada/themes/, or a path to a .toml theme. `T` picks among them all.
# theme = \"gruvbox-dark\"

# Wrap prose at most this many columns wide; 0 uses the terminal's width. Tables that need
# more room still grow up to the terminal's width.
# max_width = 0

# Show images in terminals that can (kitty, sixel, iTerm2), or false for text only.
# images = true
# Which protocol: auto asks the terminal, or kitty, sixel, iterm2, halfblocks.
# image_protocol = \"auto\"

# Read the file again when it changes on disk.
# live_reload = true

# Leave what ada showed in the terminal when it quits, below the command that ran it, so it
# can still be read (and scrolled back to); false clears it, as most full-screen programs do.
# keep_screen = true

# The table layout to start with (`t` cycles it): auto, grid or records.
# tables = \"auto\"

# Wrap long lines of code (`w` toggles it); false scrolls them sideways with ←/→.
# code_wrap = true

# Number rows by source line (`#` toggles it for the kind of file shown): off, code for code
# and other text files, or all for markdown too.
# line_numbers = \"off\"

# Keys: each action takes a key or a list of keys, and setting one replaces its defaults.
# Keys look like j, G, ctrl-d, alt-x, shift-up, space, enter, esc, tab, shift-tab,
# backspace, up, down, left, right, home, end, pageup, pagedown, f1–f12; two keys pressed in
# turn are written \"]]\" or \"g g\". Ctrl-C always quits.
[keys]
",
    );
    for &action in Action::ALL {
        let keys = Bindings::default_specs(action)
            .iter()
            .map(|k| format!("{k:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            text,
            "# {:<15}= [{keys}]{:pad$}# {}",
            action.name(),
            "",
            action.help(),
            pad = 28usize.saturating_sub(keys.len() + 2)
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Keymap;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn the_default_file_reads_as_the_defaults() {
        let mut warnings = Vec::new();
        let text = default_text();
        assert_eq!(
            parse("c.toml", &text, &mut warnings).unwrap(),
            Settings::default()
        );
        assert!(warnings.is_empty(), "{warnings:?}");

        // and uncommented, too
        let uncommented: String = text
            .lines()
            .map(|l| match l.strip_prefix("# ") {
                Some(rest) if rest.contains(" = ") || rest.contains("= [") => rest,
                _ => l,
            })
            .map(|l| format!("{l}\n"))
            .collect();
        let s = parse("c.toml", &uncommented, &mut warnings).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(s.theme.as_deref(), Some("gruvbox-dark"));
        assert_eq!(Settings { theme: None, ..s }, Settings::default());
    }

    #[test]
    fn options() {
        let text = "max_width = 100\nimages = true\nimage_protocol = \"sixel\"\nlive_reload = false\nkeep_screen = false\ntables = \"records\"\ncode_wrap = false\nline_numbers = \"all\"\n";
        let s = parse("c.toml", text, &mut Vec::new()).unwrap();
        assert_eq!(s.max_width, Some(100));
        assert_eq!(s.images, ImageMode::Force(Proto::Sixel));
        assert!(!s.live_reload && !s.keep_screen && !s.code_wrap);
        assert_eq!(s.table_mode, TableMode::Records);
        assert_eq!(s.line_numbers, LineNumbers::All);
        let off = parse(
            "c.toml",
            "images = false\nimage_protocol = \"kitty\"\n",
            &mut Vec::new(),
        );
        assert_eq!(off.unwrap().images, ImageMode::Off);
    }

    #[test]
    fn errors_and_warnings_point_at_the_value() {
        let err = parse("c.toml", "\ntables = \"wide\"\n", &mut Vec::new()).unwrap_err();
        assert_eq!(
            err,
            "c.toml:2:10: unknown table layout \"wide\" (auto, grid or records)"
        );
        let err = parse("c.toml", "max_width = \"80\"\n", &mut Vec::new()).unwrap_err();
        assert_eq!(err, "c.toml:1:13: expected an integer, found string");
        let err = parse("c.toml", "[keys]\nquit = \"ctrl-\"\n", &mut Vec::new()).unwrap_err();
        assert_eq!(err, "c.toml:2:8: can't read the key \"ctrl-\"");

        let mut warnings = Vec::new();
        parse("c.toml", "color = 1\n[keys]\nfly = \"f\"\n", &mut warnings).unwrap();
        assert_eq!(
            warnings,
            [
                "c.toml:1:1: unknown key \"color\"",
                "c.toml:3:1: unknown action \"fly\""
            ]
        );
    }

    #[test]
    fn keys_rebind() {
        let text = "[keys]\nquit = \"x\"\nnext_link = [\"j\", \"ctrl-n\"]\n";
        let s = parse("c.toml", text, &mut Vec::new()).unwrap();
        let mut km = Keymap::new(s.keys);
        let press = |km: &mut Keymap, code, mods| km.map(KeyEvent::new(code, mods));
        assert_eq!(
            press(&mut km, KeyCode::Char('x'), KeyModifiers::NONE),
            Some(crate::app::Msg::Quit)
        );
        assert_eq!(press(&mut km, KeyCode::Char('q'), KeyModifiers::NONE), None);
        assert_eq!(
            press(&mut km, KeyCode::Char('n'), KeyModifiers::CONTROL),
            Some(crate::app::Msg::FocusLink {
                dir: crate::app::Dir::Next
            })
        );
    }
}
