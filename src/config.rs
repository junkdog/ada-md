//! The config file, `$XDG_CONFIG_HOME/ada/config.toml` (P7). Optional: when it doesn't exist,
//! the TUI writes it with every option commented out at its default, so the options can be
//! found; `--print` leaves it unwritten (REL-15).
//! Walked by hand like themes (ADR-08): unknown keys are warnings, bad values are errors that
//! point at the value. Command-line flags win over it. `S` saves the session's settings back
//! into it, changing only the values ([`save`]).

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};

use toml::de::DeTable;

use crate::app::{Action, Bindings, KeySeq};
use crate::images::{ImageMode, Proto};
use crate::layout::{LineNumbers, TableMode};
use crate::links::{self, Roots};
use crate::theme::BUILTIN;
use crate::toml_walk::{self, Cx};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub theme: Option<String>,
    pub theme_bg: bool,
    pub mouse: bool,
    pub max_width: Option<u16>,
    pub images: ImageMode,
    pub live_reload: bool,
    pub keep_screen: bool,
    pub table_mode: TableMode,
    pub code_wrap: bool,
    pub line_numbers: LineNumbers,
    pub keys: Bindings,
    pub links: Roots,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: None,
            theme_bg: true,
            mouse: true,
            max_width: None,
            images: ImageMode::Auto,
            live_reload: true,
            keep_screen: true,
            table_mode: TableMode::Auto,
            code_wrap: true,
            line_numbers: LineNumbers::Off,
            keys: Bindings::default(),
            links: Roots::default(),
        }
    }
}

/// Where the config file lives.
pub fn path() -> Option<PathBuf> {
    Some(toml_walk::config_dir()?.join("config.toml"))
}

/// Read the config file, or, when there is none and `create` says so, write the commented
/// defaults. Returns the settings, the file, and warnings (a failed write is one).
pub fn load(create: bool) -> Result<(Settings, Option<PathBuf>, Vec<String>), String> {
    match path() {
        Some(path) => load_from(path, create),
        None => Ok((Settings::default(), None, Vec::new())),
    }
}

fn load_from(
    path: PathBuf,
    create: bool,
) -> Result<(Settings, Option<PathBuf>, Vec<String>), String> {
    let mut warnings = Vec::new();
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if create && let Err(e) = write(&path, &default_text()) {
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
            "theme_bg" => s.theme_bg = cx.bool(v)?,
            "mouse" => s.mouse = cx.bool(v)?,
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
            "links" => s.links = parse_links(&mut cx, v)?,
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

/// `[links]`: the roots links are looked up under ([`Roots`]).
fn parse_links(cx: &mut Cx<'_>, v: &toml_walk::Value<'_>) -> Result<Roots, String> {
    let mut roots = Roots::default();
    for (k, v) in cx.table(v)? {
        match k.get_ref().as_ref() {
            "roots" => {
                roots.roots = Vec::new();
                for root in cx.strings(v)? {
                    let path = links::expand_home(&root);
                    if !path.is_absolute() {
                        return Err(cx.at(
                            v.span(),
                            format_args!(
                                "a root must be an absolute path or start with ~/: \"{root}\""
                            ),
                        ));
                    }
                    roots.roots.push(path);
                }
            }
            "root_markers" => {
                roots.markers = cx.strings(v)?;
                if let Some(bad) = roots
                    .markers
                    .iter()
                    .find(|m| matches!(m.as_str(), "" | "." | "..") || m.contains('/'))
                {
                    return Err(cx.at(
                        v.span(),
                        format_args!(
                            "a root marker is a file or directory name, without /, not . or ..: \"{bad}\""
                        ),
                    ));
                }
            }
            other => cx.warn(k.span(), format_args!("unknown key \"links.{other}\"")),
        }
    }
    Ok(roots)
}

/// The file ada writes when there is none: every option, commented out at its default.
pub fn default_text() -> String {
    let mut text = String::from(
        "\
# ada's configuration. Every option is shown commented out, at its default value;
# command-line flags win over what is set here. `S` saves the settings in effect to this file,
# changing only the values.

# The theme: a built-in name (gruvbox-dark, gruvbox-light, catppuccin-mocha, catppuccin-macchiato,
# catppuccin-frappe, catppuccin-latte, tokyonight-night, tokyonight-storm, tokyonight-moon,
# tokyonight-day, rose-pine, rose-pine-moon, rose-pine-dawn, dracula), the name of a theme in
# ~/.config/ada/themes/, or a path to a .toml theme. `T` picks among them all.
# theme = \"gruvbox-dark\"

# Draw the theme's background; false keeps the terminal's behind the text (`B` toggles it).
# Code blocks, heading bars, table stripes, the status bar and popups keep theirs either way.
# theme_bg = true

# Take the mouse (`M` toggles it): the wheel scrolls, and a click follows a link. Most
# terminals still select text with Shift held; false leaves the mouse to the terminal.
# mouse = true

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

# Where links are looked up by name. A [[wiki]] link, or a link that is just a file name, that
# isn't beside its note is searched for in the whole tree of the note's root: the nearest
# directory above the note that is listed in roots or holds one of root_markers. Folders whose
# names start with a dot are skipped. When several files match, the one in a folder named like
# the note's wins, else the link shows as broken; [[folder/name]] tells them apart.
[links]
# roots = []                     # e.g. [\"~/notes\"]
# root_markers = [\".obsidian\"]

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

/// Read the config file for saving: a missing one reads as the file ada would write.
pub fn read(path: &Path) -> io::Result<String> {
    match fs::read_to_string(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(default_text()),
        read => read,
    }
}

/// Write the config file, and the directory it goes in when that's missing.
pub fn write(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, text)
}

/// The options `S` saves, in the order its box lists them. `keep_screen` has no key or flag to
/// change it in a session, and `[keys]` none at all.
const SAVED: [&str; 10] = [
    "theme",
    "theme_bg",
    "mouse",
    "tables",
    "code_wrap",
    "line_numbers",
    "max_width",
    "images",
    "image_protocol",
    "live_reload",
];

/// What saving the session's settings does: each option whose active value differs from the
/// file's, as `(key, from, to)` TOML values, and the file's text with the new values in place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Save {
    pub changes: Vec<(&'static str, String, String)>,
    pub text: String,
}

/// Compare the `active` settings with the config file's `text`, which `file` names in errors.
/// An option the file doesn't set counts as its default. Only the values change: every comment
/// and line around them stays as the user wrote it.
pub fn save(file: &str, text: &str, active: &Settings) -> Result<Save, String> {
    let saved = parse(file, text, &mut Vec::new())?;
    let parsed = toml_walk::parse(file, text)?;
    let root = parsed.get_ref();
    // with images off, the protocol stays as the file names it
    let protocol = toml_walk::entry(root, "image_protocol")
        .and_then(|v| v.get_ref().as_str())
        .map_or_else(|| toml_string("auto"), toml_string);
    let to = values(active, &protocol);
    let changes: Vec<_> = SAVED
        .into_iter()
        .zip(values(&saved, &protocol))
        .zip(to.clone())
        .filter(|((_, from), to)| from != to)
        .map(|((key, from), to)| (key, from, to))
        .collect();
    if changes.is_empty() {
        return Ok(Save {
            changes,
            text: text.to_owned(),
        });
    }
    let text = rewrite(text, root, &changes);
    // what's written must read back as the settings saved
    let back = parse(file, &text, &mut Vec::new()).map(|s| values(&s, &protocol));
    if back != Ok(to) {
        return Err(format!(
            "{file}: can't save the settings without breaking the file"
        ));
    }
    Ok(Save { changes, text })
}

/// The options [`SAVED`] names, as `s` sets them, in TOML. With images off, `image_protocol` is
/// `protocol`.
fn values(s: &Settings, protocol: &str) -> [String; 10] {
    [
        toml_string(s.theme.as_deref().unwrap_or(BUILTIN[0].0)),
        s.theme_bg.to_string(),
        s.mouse.to_string(),
        toml_string(s.table_mode.name()),
        s.code_wrap.to_string(),
        toml_string(s.line_numbers.spelling()),
        s.max_width.unwrap_or(0).to_string(),
        (s.images != ImageMode::Off).to_string(),
        match s.images {
            ImageMode::Off => protocol.to_owned(),
            ImageMode::Auto => toml_string("auto"),
            ImageMode::Force(proto) => toml_string(proto.name()),
        },
        s.live_reload.to_string(),
    ]
}

/// `s` as a TOML basic string.
fn toml_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `text` with each change's new value in place. A key the file sets has its value replaced.
/// One it doesn't takes its commented default line, `# key = …`, uncommented; failing that, it
/// goes before the first table and the comments over it, so it stays top-level.
fn rewrite(text: &str, root: &DeTable<'_>, changes: &[(&str, String, String)]) -> String {
    let tables = first_table(text, root);
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let mut added = String::new();
    for (key, _, value) in changes {
        if let Some(v) = toml_walk::entry(root, key) {
            edits.push((v.span(), value.clone()));
        } else if let Some(line) = commented(&text[..tables], key) {
            edits.push((line, format!("{key} = {value}")));
        } else {
            let _ = writeln!(added, "{key} = {value}");
        }
    }
    if !added.is_empty() {
        let at = if tables < text.len() {
            comments_above(text, tables)
        } else {
            tables
        };
        if !text[..at].is_empty() && !text[..at].ends_with('\n') {
            added.insert(0, '\n');
        }
        if at < text.len() {
            added.push('\n');
        }
        edits.push((at..at, added));
    }
    // an insertion goes before a line edit that starts where it does
    edits.sort_by_key(|(span, _)| (span.start, span.end));
    let mut out = String::with_capacity(text.len() + 64);
    let mut from = 0;
    for (span, value) in edits {
        out.push_str(&text[from..span.start]);
        out.push_str(&value);
        from = span.end;
    }
    out.push_str(&text[from..]);
    out
}

/// Where the line of the first table header (`[keys]`) starts, or the end of `text`.
fn first_table(text: &str, root: &DeTable<'_>) -> usize {
    root.iter()
        .filter_map(|(k, _)| {
            let line = text[..k.span().start].rfind('\n').map_or(0, |i| i + 1);
            text[line..].trim_start().starts_with('[').then_some(line)
        })
        .min()
        .unwrap_or(text.len())
}

/// The start of the comment lines right above the line starting at `at`.
fn comments_above(text: &str, at: usize) -> usize {
    let mut start = at;
    while start > 0 {
        let line = text[..start - 1].rfind('\n').map_or(0, |i| i + 1);
        if !text[line..start].trim_start().starts_with('#') {
            break;
        }
        start = line;
    }
    start
}

/// The line of `text` that sets `key` commented out, `# key = …`, without its line break.
fn commented(text: &str, key: &str) -> Option<Range<usize>> {
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let sets = body
            .trim_start()
            .strip_prefix('#')
            .and_then(|rest| rest.trim_start().strip_prefix(key))
            .is_some_and(|rest| rest.trim_start().starts_with('='));
        if sets {
            return Some(start..start + body.len());
        }
        start += line.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Keymap;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
        let text = "max_width = 100\nimages = true\nimage_protocol = \"sixel\"\nlive_reload = false\nkeep_screen = false\ntables = \"records\"\ncode_wrap = false\nline_numbers = \"all\"\ntheme_bg = false\nmouse = false\n";
        let s = parse("c.toml", text, &mut Vec::new()).unwrap();
        assert_eq!(s.max_width, Some(100));
        assert!(!s.theme_bg && !s.mouse);
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
    fn link_roots() {
        let text = "[links]\nroots = [\"/notes\", \"~/vault\"]\nroot_markers = [\"VAULT.md\"]\n";
        let s = parse("c.toml", text, &mut Vec::new()).unwrap();
        assert_eq!(s.links.roots[0], PathBuf::from("/notes"));
        assert_eq!(s.links.roots[1], links::expand_home("~/vault"));
        assert_eq!(s.links.markers, ["VAULT.md"]);
        // the markers keep their default when only roots are set
        let s = parse("c.toml", "[links]\nroots = []\n", &mut Vec::new()).unwrap();
        assert_eq!(s.links, Roots::default());

        let err = parse("c.toml", "[links]\nroots = [\"notes\"]\n", &mut Vec::new()).unwrap_err();
        assert!(
            err.starts_with("c.toml:2:9: a root must be an absolute path"),
            "{err}"
        );
        let err = parse(
            "c.toml",
            "[links]\nroot_markers = [\"a/b\"]\n",
            &mut Vec::new(),
        );
        assert!(err.unwrap_err().contains("without /"));
        // either would make every directory a root
        for bad in [".", ".."] {
            let text = format!("[links]\nroot_markers = [\"{bad}\"]\n");
            assert!(parse("c.toml", &text, &mut Vec::new()).is_err(), "{bad}");
        }
        let mut warnings = Vec::new();
        parse("c.toml", "[links]\nroot = []\n", &mut warnings).unwrap();
        assert_eq!(warnings, ["c.toml:2:1: unknown key \"links.root\""]);
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

    fn changes(save: &Save) -> Vec<String> {
        save.changes
            .iter()
            .map(|(key, from, to)| format!("{key}: {from} → {to}"))
            .collect()
    }

    /// The lines of `after` that aren't in `before`, and those of `before` that went.
    fn diff(before: &str, after: &str) -> (Vec<String>, Vec<String>) {
        let lines = |t: &str| t.lines().map(str::to_owned).collect::<Vec<_>>();
        let (b, a) = (lines(before), lines(after));
        let added = a.iter().filter(|l| !b.contains(l)).cloned().collect();
        let gone = b.iter().filter(|l| !a.contains(l)).cloned().collect();
        (added, gone)
    }

    #[test]
    fn saving_the_defaults_changes_nothing() {
        let text = default_text();
        let out = save("c.toml", &text, &Settings::default()).unwrap();
        assert!(out.changes.is_empty());
        assert_eq!(out.text, text);
        // an option set to its default counts as the default
        let text = "theme = \"gruvbox-dark\"\nmax_width = 0\n";
        assert!(
            save("c.toml", text, &Settings::default())
                .unwrap()
                .changes
                .is_empty()
        );
    }

    #[test]
    fn unset_options_take_their_commented_default_line() {
        let text = default_text();
        let active = Settings {
            theme: Some("dracula".into()),
            theme_bg: false,
            max_width: Some(100),
            images: ImageMode::Force(Proto::Sixel),
            live_reload: false,
            table_mode: TableMode::Records,
            code_wrap: false,
            line_numbers: LineNumbers::All,
            ..Settings::default()
        };
        let out = save("c.toml", &text, &active).unwrap();
        assert_eq!(
            changes(&out),
            [
                "theme: \"gruvbox-dark\" → \"dracula\"",
                "theme_bg: true → false",
                "tables: \"auto\" → \"records\"",
                "code_wrap: true → false",
                "line_numbers: \"off\" → \"all\"",
                "max_width: 0 → 100",
                "image_protocol: \"auto\" → \"sixel\"",
                "live_reload: true → false",
            ]
        );
        let (added, gone) = diff(&text, &out.text);
        assert_eq!(
            added,
            [
                "theme = \"dracula\"",
                "theme_bg = false",
                "max_width = 100",
                "image_protocol = \"sixel\"",
                "live_reload = false",
                "tables = \"records\"",
                "code_wrap = false",
                "line_numbers = \"all\"",
            ]
        );
        assert_eq!(
            gone,
            [
                "# theme = \"gruvbox-dark\"",
                "# theme_bg = true",
                "# max_width = 0",
                "# image_protocol = \"auto\"",
                "# live_reload = true",
                "# tables = \"auto\"",
                "# code_wrap = true",
                "# line_numbers = \"off\"",
            ]
        );
        // the action of the same name in [keys] stays commented
        assert!(out.text.contains("# code_wrap      = [\"w\"]"));
        assert!(out.text.contains("# theme_bg       = [\"B\"]"));
        let back = parse("c.toml", &out.text, &mut Vec::new()).unwrap();
        assert_eq!(back, active);
    }

    #[test]
    fn a_set_value_is_replaced_in_place() {
        let text = "\
# mine
theme = \"gruvbox-light\"   # light, for the day
tables=\"grid\"
code_wrap = false
shade = 1

[keys]
quit = \"x\"
";
        let mut warnings = Vec::new();
        let mut active = parse("c.toml", text, &mut warnings).unwrap();
        assert_eq!(warnings, ["c.toml:5:1: unknown key \"shade\""]);
        active.theme = Some("~/themes/my \"own\".toml".into());
        active.table_mode = TableMode::Auto;
        let out = save("c.toml", text, &active).unwrap();
        assert_eq!(
            changes(&out),
            [
                "theme: \"gruvbox-light\" → \"~/themes/my \\\"own\\\".toml\"",
                "tables: \"grid\" → \"auto\""
            ]
        );
        assert_eq!(
            out.text,
            "\
# mine
theme = \"~/themes/my \\\"own\\\".toml\"   # light, for the day
tables=\"auto\"
code_wrap = false
shade = 1

[keys]
quit = \"x\"
"
        );
        let back = parse("c.toml", &out.text, &mut Vec::new()).unwrap();
        assert_eq!(back.theme.as_deref(), Some("~/themes/my \"own\".toml"));
        assert_eq!(back.keys, active.keys);
    }

    #[test]
    fn a_missing_line_goes_before_the_first_table_and_its_comments() {
        let text = "code_wrap = false\n\n# my keys\n[keys]\nquit = \"x\"\n";
        let active = Settings {
            max_width: Some(90),
            live_reload: false,
            ..parse("c.toml", text, &mut Vec::new()).unwrap()
        };
        let out = save("c.toml", text, &active).unwrap();
        assert_eq!(
            out.text,
            "code_wrap = false\n\nmax_width = 90\nlive_reload = false\n\n# my keys\n[keys]\nquit = \"x\"\n"
        );
        // with no table, at the end
        for text in ["", "code_wrap = false", "code_wrap = false\n"] {
            let active = Settings {
                code_wrap: false,
                max_width: Some(90),
                ..Settings::default()
            };
            let saved = super::save("c.toml", text, &active).unwrap().text;
            assert_eq!(
                parse("c.toml", &saved, &mut Vec::new()).unwrap(),
                active,
                "{saved:?}"
            );
            assert!(saved.ends_with("max_width = 90\n"), "{saved:?}");
        }
    }

    #[test]
    fn images_off_keeps_the_protocol() {
        let text = "images = true\nimage_protocol = \"kitty\"\n";
        let off = Settings {
            images: ImageMode::Off,
            ..Settings::default()
        };
        let out = save("c.toml", text, &off).unwrap();
        assert_eq!(changes(&out), ["images: true → false"]);
        assert_eq!(out.text, "images = false\nimage_protocol = \"kitty\"\n");

        // a protocol asked for with images off in the file turns them on
        let text = "images = false\nimage_protocol = \"kitty\"\n";
        let sixel = Settings {
            images: ImageMode::Force(Proto::Sixel),
            ..Settings::default()
        };
        let out = save("c.toml", text, &sixel).unwrap();
        assert_eq!(
            changes(&out),
            [
                "images: false → true",
                "image_protocol: \"kitty\" → \"sixel\""
            ]
        );
        let kitty = Settings {
            images: ImageMode::Force(Proto::Kitty),
            ..Settings::default()
        };
        assert_eq!(
            changes(&super::save("c.toml", text, &kitty).unwrap()),
            ["images: false → true"]
        );
    }

    #[test]
    fn a_file_that_does_not_read_is_not_saved() {
        let err = save("c.toml", "tables = 1\n", &Settings::default()).unwrap_err();
        assert_eq!(err, "c.toml:1:10: expected a string, found integer");
    }

    #[test]
    fn a_missing_file_reads_as_the_defaults() {
        let dir = std::env::temp_dir().join(format!("ada-config-{}", std::process::id()));
        let path = dir.join("ada/config.toml");
        assert_eq!(read(&path).unwrap(), default_text());
        write(&path, "max_width = 1\n").unwrap();
        assert_eq!(read(&path).unwrap(), "max_width = 1\n");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_file_is_written_only_when_asked() {
        let dir = std::env::temp_dir().join(format!("ada-config-load-{}", std::process::id()));
        let path = dir.join("ada/config.toml");
        let (settings, file, warnings) = load_from(path.clone(), false).unwrap();
        assert_eq!(settings, Settings::default());
        assert_eq!(file.as_deref(), Some(path.as_path()));
        assert!(warnings.is_empty());
        assert!(!path.exists());
        load_from(path.clone(), true).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), default_text());
        fs::remove_dir_all(&dir).unwrap();
    }
}
