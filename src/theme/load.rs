//! Theme TOML → [`Theme`], walked by hand from `DeTable` without serde (ADR-08). Spans make errors
//! precise; unknown keys are warnings so newer themes still load in older preen.

use std::borrow::Cow;
use std::fmt::Display;
use std::ops::Range;
use std::path::PathBuf;
use std::{env, fs};

use ratatui::style::{Color, Modifier, Style};
use toml::Spanned;
use toml::de::{DeTable, DeValue};

use super::{StyleKey, TableBorder, Theme};

/// Built-in themes, `(name, toml)`. The first one is the default.
pub const BUILTIN: &[(&str, &str)] = &[("gruvbox-dark", include_str!("builtin/gruvbox-dark.toml"))];

const MAX_EXTENDS_DEPTH: u32 = 8;

type Value<'i> = Spanned<DeValue<'i>>;

/// Resolve `--theme NAME`: a path, then `$XDG_CONFIG_HOME/preen/themes/NAME.toml`, then the
/// built-ins. Returns the theme and any warnings.
pub fn load(name: Option<&str>) -> Result<(Theme, Vec<String>), String> {
    let mut warnings = Vec::new();
    let theme = resolve(name.unwrap_or(BUILTIN[0].0), &mut warnings, 0)?;
    Ok((theme, warnings))
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

fn themes_dir() -> Option<PathBuf> {
    let config = env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("preen").join("themes"))
}

/// Load a theme from TOML text. `file` names it in messages; `depth` guards `extends` cycles.
pub fn load_str(
    file: &str,
    text: &str,
    warnings: &mut Vec<String>,
    depth: u32,
) -> Result<Theme, String> {
    let root = DeTable::parse(text).map_err(|e| {
        let at = e.span().map_or_else(String::new, |s| {
            let (line, col) = position(text, s.start);
            format!(":{line}:{col}")
        });
        format!("{file}{at}: {}", e.message().trim())
    })?;
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

fn entry<'v, 'i>(table: &'v DeTable<'i>, key: &str) -> Option<&'v Value<'i>> {
    table
        .iter()
        .find(|(k, _)| k.get_ref().as_ref() == key)
        .map(|(_, v)| v)
}

fn apply(theme: &mut Theme, root: &DeTable<'_>, cx: &mut Cx<'_>) -> Result<(), String> {
    // The palette comes first: every colour below may refer to it.
    if let Some(palette) = entry(root, "palette") {
        for (k, v) in cx.table(palette)? {
            let color = cx.color(&theme.palette, v)?;
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
            "fg" => style.fg(cx.color(palette, v)?),
            "bg" => style.bg(cx.color(palette, v)?),
            "underline_color" => style.underline_color(cx.color(palette, v)?),
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

/// 1-based line and column of a byte offset.
fn position(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..text.floor_char_boundary(offset)];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, col)
}

struct Cx<'a> {
    file: &'a str,
    text: &'a str,
    warnings: &'a mut Vec<String>,
}

impl Cx<'_> {
    fn at(&self, span: Range<usize>, msg: impl Display) -> String {
        let (line, col) = position(self.text, span.start);
        format!("{}:{line}:{col}: {msg}", self.file)
    }

    fn warn(&mut self, span: Range<usize>, msg: impl Display) {
        let w = self.at(span, msg);
        self.warnings.push(w);
    }

    fn expected(&self, what: &str, v: &Value<'_>) -> String {
        self.at(
            v.span(),
            format_args!("expected {what}, found {}", v.get_ref().type_str()),
        )
    }

    fn table<'v, 'i>(&self, v: &'v Value<'i>) -> Result<&'v DeTable<'i>, String> {
        v.get_ref()
            .as_table()
            .ok_or_else(|| self.expected("a table", v))
    }

    fn string<'v>(&self, v: &'v Value<'_>) -> Result<&'v str, String> {
        v.get_ref()
            .as_str()
            .ok_or_else(|| self.expected("a string", v))
    }

    /// A non-empty string, for glyphs that are repeated or must take up space.
    fn glyph(&self, v: &Value<'_>) -> Result<String, String> {
        let s = self.string(v)?;
        if unicode_width::UnicodeWidthStr::width(s) == 0 {
            return Err(self.at(v.span(), "glyph must be at least one column wide"));
        }
        Ok(s.to_owned())
    }

    fn strings(&self, v: &Value<'_>) -> Result<Vec<String>, String> {
        let array = v
            .get_ref()
            .as_array()
            .ok_or_else(|| self.expected("an array of strings", v))?;
        array
            .into_iter()
            .map(|item| self.string(item).map(str::to_owned))
            .collect()
    }

    fn bool(&self, v: &Value<'_>) -> Result<bool, String> {
        v.get_ref()
            .as_bool()
            .ok_or_else(|| self.expected("true or false", v))
    }

    fn int(&self, v: &Value<'_>) -> Result<i64, String> {
        let i = v
            .get_ref()
            .as_integer()
            .ok_or_else(|| self.expected("an integer", v))?;
        i64::from_str_radix(i.as_str(), i.radix())
            .map_err(|e| self.at(v.span(), format_args!("bad integer: {e}")))
    }

    fn u16(&self, v: &Value<'_>) -> Result<u16, String> {
        u16::try_from(self.int(v)?)
            .map_err(|_| self.at(v.span(), "expected a number from 0 to 65535"))
    }

    fn color(&self, palette: &[(String, Color)], v: &Value<'_>) -> Result<Color, String> {
        match v.get_ref() {
            DeValue::String(s) => parse_color(s, palette)
                .ok_or_else(|| self.at(v.span(), format_args!("unknown palette colour \"{s}\""))),
            DeValue::Integer(_) => u8::try_from(self.int(v)?)
                .map(Color::Indexed)
                .map_err(|_| self.at(v.span(), "colour index must be 0–255")),
            _ => Err(self.expected("a colour", v)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_loads_without_warnings() {
        let mut warnings = Vec::new();
        let theme = load_str("builtin", BUILTIN[0].1, &mut warnings, 0).unwrap();
        assert_eq!(warnings, Vec::<String>::new());
        assert_eq!(theme.name, "gruvbox-dark");
        assert_eq!(theme.glyphs.heading[1].0, "## ");
        assert_eq!(theme.glyphs.code_pad, 1);
        assert_eq!(
            theme.styles[StyleKey::Emph as usize].fg,
            Some(Color::Rgb(0xfa, 0xbd, 0x2f))
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
