//! Theme TOML → [`Theme`], walked by hand from `DeTable` without serde (ADR-08). Spans make errors
//! precise; unknown keys are warnings so newer themes still load in older ada.

use std::borrow::Cow;
use std::fs;
use std::ops::Range;
use std::path::PathBuf;

use ratatui::style::{Color, Modifier, Style};
use toml::de::{DeTable, DeValue};

use super::{StyleKey, TableBorder, Theme};
use crate::toml_walk::{self, Cx, Value, entry};

/// Built-in themes, `(name, toml)`. The first one is the default.
pub const BUILTIN: &[(&str, &str)] = &[
    ("gruvbox-dark", include_str!("builtin/gruvbox-dark.toml")),
    ("gruvbox-light", include_str!("builtin/gruvbox-light.toml")),
    (
        "catppuccin-mocha",
        include_str!("builtin/catppuccin-mocha.toml"),
    ),
];

const MAX_EXTENDS_DEPTH: u32 = 8;

/// Resolve `--theme NAME`: a path, then `$XDG_CONFIG_HOME/ada/themes/NAME.toml`, then the
/// built-ins. Returns the theme and any warnings.
/// The theme is named after how it was asked for: `T` and the status line show that name.
pub fn load(name: Option<&str>) -> Result<(Theme, Vec<String>), String> {
    let mut warnings = Vec::new();
    let name = name.unwrap_or(BUILTIN[0].0);
    let mut theme = resolve(name, &mut warnings, 0)?;
    theme.name = lookup_name(name);
    Ok((theme, warnings))
}

/// `--theme ~/x/mine.toml` is the theme `mine`.
fn lookup_name(name: &str) -> String {
    if name.ends_with(".toml") || name.contains('/') {
        let path = std::path::Path::new(name);
        return path
            .file_stem()
            .map_or_else(|| name.to_owned(), |s| s.to_string_lossy().into_owned());
    }
    name.to_owned()
}

/// Every theme `T` lists: the built-ins, then the user's themes directory by name,
/// with `current` among them. A user theme named like a built-in replaces it. Themes that
/// don't load are left out, with a warning.
pub fn load_all(current: &Theme) -> (Vec<Theme>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut names: Vec<String> = BUILTIN.iter().map(|(n, _)| (*n).to_owned()).collect();
    if let Some(dir) = themes_dir()
        && let Ok(entries) = fs::read_dir(dir)
    {
        let mut user: Vec<String> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .filter_map(|p| Some(p.file_stem()?.to_string_lossy().into_owned()))
            .filter(|n| !names.contains(n))
            .collect();
        user.sort();
        names.extend(user);
    }
    if !names.contains(&current.name) {
        names.insert(0, current.name.clone());
    }
    let mut themes = Vec::new();
    for name in names {
        if name == current.name {
            themes.push(current.clone());
            continue;
        }
        let mut w = Vec::new();
        match resolve(&name, &mut w, 0) {
            Ok(mut theme) => {
                theme.name = name;
                themes.push(theme);
            }
            Err(e) => warnings.push(e),
        }
    }
    (themes, warnings)
}

fn resolve(name: &str, warnings: &mut Vec<String>, depth: u32) -> Result<Theme, String> {
    if depth > MAX_EXTENDS_DEPTH {
        return Err(format!("theme \"{name}\": the `extends` chain is too deep"));
    }
    let (file, text) = find(name)?;
    load_str(&file, &text, warnings, depth)
}

fn find(name: &str) -> Result<(String, Cow<'static, str>), String> {
    let read = |path: PathBuf| {
        fs::read_to_string(&path)
            .map(|text| (path.display().to_string(), Cow::Owned(text)))
            .map_err(|e| format!("{}: {e}", path.display()))
    };
    if name.ends_with(".toml") || name.contains('/') {
        return read(PathBuf::from(name));
    }
    if let Some(dir) = themes_dir() {
        let path = dir.join(format!("{name}.toml"));
        if path.is_file() {
            return read(path);
        }
    }
    BUILTIN
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(n, text)| (format!("builtin:{n}"), Cow::Borrowed(*text)))
        .ok_or_else(|| format!("unknown theme \"{name}\""))
}

pub(crate) fn themes_dir() -> Option<PathBuf> {
    Some(toml_walk::config_dir()?.join("themes"))
}

/// Load a theme from TOML text. `file` names it in messages; `depth` guards `extends` cycles.
pub fn load_str(
    file: &str,
    text: &str,
    warnings: &mut Vec<String>,
    depth: u32,
) -> Result<Theme, String> {
    let root = toml_walk::parse(file, text)?;
    let root = root.get_ref();
    let mut cx = Cx {
        file,
        text,
        warnings,
    };
    let mut theme = match entry(root, "extends") {
        Some(base) => {
            let base = cx.string(base)?;
            resolve(base, cx.warnings, depth + 1)?
        }
        None => Theme::empty(),
    };
    apply(&mut theme, root, &mut cx)?;
    Ok(theme)
}

fn apply(theme: &mut Theme, root: &DeTable<'_>, cx: &mut Cx<'_>) -> Result<(), String> {
    // The palette comes first: every colour below may refer to it.
    if let Some(palette) = entry(root, "palette") {
        for (k, v) in cx.table(palette)? {
            let color = color(cx, &theme.palette, v)?;
            let name = k.get_ref().as_ref();
            match theme.palette.iter_mut().find(|(n, _)| n == name) {
                Some(slot) => slot.1 = color,
                None => theme.palette.push((name.to_owned(), color)),
            }
        }
    }
    for (k, v) in root {
        match k.get_ref().as_ref() {
            "name" => theme.name = cx.string(v)?.to_owned(),
            "extends" | "palette" => {}
            "glyphs" => glyphs(theme, v, cx)?,
            "syntax" => syntax(theme, v, cx)?,
            section @ ("document" | "heading" | "inline" | "block" | "gutter" | "ui") => {
                styles(theme, section, v, cx)?;
            }
            other => cx.warn(k.span(), format_args!("unknown key \"{other}\"")),
        }
    }
    Ok(())
}

fn styles(theme: &mut Theme, path: &str, v: &Value<'_>, cx: &mut Cx<'_>) -> Result<(), String> {
    if let Some(&key) = StyleKey::ALL.iter().find(|k| k.path() == path) {
        return style_entry(theme, key, v, cx);
    }
    for (k, v) in cx.table(v)? {
        let sub = format!("{path}.{}", k.get_ref());
        let nested = format!("{sub}.");
        if StyleKey::ALL
            .iter()
            .any(|key| key.path() == sub || key.path().starts_with(&nested))
        {
            styles(theme, &sub, v, cx)?;
        } else {
            cx.warn(k.span(), format_args!("unknown style \"{sub}\""));
        }
    }
    Ok(())
}

fn style_entry(
    theme: &mut Theme,
    key: StyleKey,
    v: &Value<'_>,
    cx: &mut Cx<'_>,
) -> Result<(), String> {
    let (style, extras) = parse_style(&theme.palette, v, cx)?;
    let slot = &mut theme.styles[key as usize];
    *slot = slot.patch(style);
    for (name, span, value) in extras {
        let g = &mut theme.glyphs;
        match (name, key.heading_level(), key.gutter_index()) {
            ("margin", ..) if key == StyleKey::Text => g.margin = cx.u16(value)?,
            ("prefix", Some(level), _) => g.heading[level].0 = cx.string(value)?.to_owned(),
            ("suffix", Some(level), _) => g.heading[level].1 = cx.string(value)?.to_owned(),
            ("pad", ..) if key == StyleKey::CodeInline => g.code_pad = cx.u16(value)?,
            ("glyph", _, Some(i)) => g.gutter[i] = cx.glyph(value)?,
            _ => cx.warn(
                span,
                format_args!("unknown field \"{name}\" in {}", key.path()),
            ),
        }
    }
    Ok(())
}

type Extras<'v, 'i> = Vec<(&'v str, Range<usize>, &'v Value<'i>)>;

/// The style fields of a table, plus the fields that aren't style fields.
fn parse_style<'v, 'i>(
    palette: &[(String, Color)],
    v: &'v Value<'i>,
    cx: &Cx<'_>,
) -> Result<(Style, Extras<'v, 'i>), String> {
    let mut style = Style::new();
    let mut extras = Vec::new();
    for (k, v) in cx.table(v)? {
        let name: &str = k.get_ref();
        style = match name {
            "fg" => style.fg(color(cx, palette, v)?),
            "bg" => style.bg(color(cx, palette, v)?),
            "underline_color" => style.underline_color(color(cx, palette, v)?),
            "bold" => modifier(style, Modifier::BOLD, cx.bool(v)?),
            "italic" => modifier(style, Modifier::ITALIC, cx.bool(v)?),
            "underline" => modifier(style, Modifier::UNDERLINED, cx.bool(v)?),
            "crossed_out" => modifier(style, Modifier::CROSSED_OUT, cx.bool(v)?),
            "dim" => modifier(style, Modifier::DIM, cx.bool(v)?),
            "reversed" => modifier(style, Modifier::REVERSED, cx.bool(v)?),
            _ => {
                extras.push((name, k.span(), v));
                style
            }
        };
    }
    Ok((style, extras))
}

fn modifier(style: Style, m: Modifier, on: bool) -> Style {
    if on {
        style.add_modifier(m)
    } else {
        style.remove_modifier(m)
    }
}

fn glyphs(theme: &mut Theme, v: &Value<'_>, cx: &mut Cx<'_>) -> Result<(), String> {
    for (k, v) in cx.table(v)? {
        let g = &mut theme.glyphs;
        match k.get_ref().as_ref() {
            "bullets" => {
                let bullets = cx.strings(v)?;
                if bullets.is_empty() {
                    return Err(cx.at(v.span(), "bullets must not be empty"));
                }
                g.bullets = bullets;
            }
            "quote_bar" => g.quote_bar = cx.string(v)?.to_owned(),
            "rule" => g.rule = cx.glyph(v)?,
            "table_border" => {
                g.table_border = match cx.string(v)? {
                    "rounded" => TableBorder::Rounded,
                    "ascii" => TableBorder::Ascii,
                    "none" => TableBorder::None,
                    other => {
                        return Err(cx.at(
                            v.span(),
                            format_args!(
                                "unknown table border \"{other}\" (rounded, ascii or none)"
                            ),
                        ));
                    }
                };
            }
            "task" => match <[String; 2]>::try_from(cx.strings(v)?) {
                Ok(task) => g.task = task,
                Err(_) => return Err(cx.at(v.span(), "task needs two glyphs: [done, open]")),
            },
            "wrap_mark" => g.wrap_mark = cx.glyph(v)?,
            "code_clip" => match <[String; 2]>::try_from(cx.strings(v)?) {
                Ok(clip) => g.code_clip = clip,
                Err(_) => return Err(cx.at(v.span(), "code_clip needs two glyphs: [left, right]")),
            },
            "sort_mark" => match <[String; 2]>::try_from(cx.strings(v)?) {
                Ok(mark) => g.sort_mark = mark,
                Err(_) => return Err(cx.at(v.span(), "sort_mark needs two glyphs: [asc, desc]")),
            },
            "image" => g.image = cx.string(v)?.to_owned(),
            other => cx.warn(k.span(), format_args!("unknown glyph \"{other}\"")),
        }
    }
    Ok(())
}

fn syntax(theme: &mut Theme, v: &Value<'_>, cx: &mut Cx<'_>) -> Result<(), String> {
    for (k, v) in cx.table(v)? {
        let (style, extras) = parse_style(&theme.palette, v, cx)?;
        for (name, span, _) in extras {
            cx.warn(span, format_args!("unknown field \"{name}\" in syntax"));
        }
        let scope = k.get_ref().as_ref();
        match theme.syntax.iter_mut().find(|(s, _)| s == scope) {
            Some(slot) => slot.1 = slot.1.patch(style),
            None => theme.syntax.push((scope.to_owned(), style)),
        }
    }
    Ok(())
}

fn parse_color(s: &str, palette: &[(String, Color)]) -> Option<Color> {
    if let Some(hex) = s.strip_prefix('#') {
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let n = u32::from_str_radix(hex, 16).ok()?;
        return match hex.len() {
            6 => Some(Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8)),
            3 => {
                let expand = |d: u32| (d * 17) as u8;
                Some(Color::Rgb(
                    expand(n >> 8),
                    expand((n >> 4) & 0xf),
                    expand(n & 0xf),
                ))
            }
            _ => None,
        };
    }
    if let Some((_, c)) = palette.iter().find(|(name, _)| name == s) {
        return Some(*c);
    }
    match s {
        "default" | "reset" => Some(Color::Reset),
        _ => s.parse().ok(),
    }
}

fn color(cx: &Cx<'_>, palette: &[(String, Color)], v: &Value<'_>) -> Result<Color, String> {
    match v.get_ref() {
        DeValue::String(s) => parse_color(s, palette)
            .ok_or_else(|| cx.at(v.span(), format_args!("unknown palette colour \"{s}\""))),
        DeValue::Integer(_) => u8::try_from(cx.int(v)?)
            .map(Color::Indexed)
            .map_err(|_| cx.at(v.span(), "colour index must be 0–255")),
        _ => Err(cx.expected("a colour", v)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_load_without_warnings() {
        for (name, text) in BUILTIN {
            let mut warnings = Vec::new();
            let theme = load_str("builtin", text, &mut warnings, 0).unwrap();
            assert_eq!(warnings, Vec::<String>::new());
            assert_eq!(theme.name, *name);
        }
        let mut warnings = Vec::new();
        let theme = load_str("builtin", BUILTIN[0].1, &mut warnings, 0).unwrap();
        assert_eq!(theme.name, "gruvbox-dark");
        assert_eq!(theme.glyphs.heading[1].0, "## ");
        assert_eq!(theme.glyphs.code_pad, 1);
        assert_eq!(
            theme.styles[StyleKey::Emph as usize].fg,
            Some(Color::Rgb(0xd5, 0xc4, 0xa1))
        );
        assert!(!theme.syntax.is_empty());
    }

    #[test]
    fn errors_point_at_the_value() {
        let text = "[palette]\nfg = \"#ebdbb2\"\n\n[inline]\nemph = { fg = \"orang\" }\n";
        let err = load_str("t.toml", text, &mut Vec::new(), 0).unwrap_err();
        assert_eq!(err, "t.toml:5:15: unknown palette colour \"orang\"");
    }

    #[test]
    fn unknown_keys_are_warnings() {
        let text = "shiny = true\n[inline]\nemph = { fg = \"#fff\", sparkle = 1 }\nnope = {}\n";
        let mut warnings = Vec::new();
        load_str("t.toml", text, &mut warnings, 0).unwrap();
        assert_eq!(
            warnings,
            [
                "t.toml:3:23: unknown field \"sparkle\" in inline.emph",
                "t.toml:4:1: unknown style \"inline.nope\"",
                "t.toml:1:1: unknown key \"shiny\"",
            ]
        );
    }

    #[test]
    fn extends_overrides_only_some_keys() {
        let text =
            "extends = \"gruvbox-dark\"\nname = \"mine\"\n[inline]\nemph = { fg = \"red\" }\n";
        let theme = load_str("t.toml", text, &mut Vec::new(), 0).unwrap();
        let emph = theme.styles[StyleKey::Emph as usize];
        assert_eq!(theme.name, "mine");
        assert_eq!(emph.fg, Some(Color::Rgb(0xfb, 0x49, 0x34)));
        assert!(emph.add_modifier.contains(Modifier::ITALIC));
        assert_eq!(theme.glyphs.heading[1].0, "## ");
    }

    #[test]
    fn colours() {
        let palette = [("x".to_owned(), Color::Rgb(1, 2, 3))];
        assert_eq!(parse_color("x", &palette), Some(Color::Rgb(1, 2, 3)));
        assert_eq!(
            parse_color("#abc", &palette),
            Some(Color::Rgb(0xaa, 0xbb, 0xcc))
        );
        assert_eq!(parse_color("#+12345", &palette), None);
        assert_eq!(parse_color("default", &palette), Some(Color::Reset));
        assert_eq!(parse_color("blue", &palette), Some(Color::Blue));
    }
}
