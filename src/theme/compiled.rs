//! The dark built-in themes compiled into ada (PRN-008): each as its TOML loads, written out as
//! data by the [`compiled!`] macro, so a start with one of them parses no TOML. The toml crate's
//! parse and the walk over it took 1.67 M of the 2.76 M instructions `ada -p` ran on an empty
//! note. `compiled/dark.rs` is generated from the TOML by `compiled_themes_are_their_toml`
//! (`ADA_REGEN_THEMES=1` rewrites it), which fails while the two differ. The light built-ins,
//! a theme in the user's `themes/` and one by path load from TOML as before; a theme that
//! `extends` a compiled one starts from it.

use crate::doc::{CalloutKind, Task};
use crate::style::{Color, Modifier, Style};

use super::{Glyphs, StyleKey, TableBorder, Theme};

/// A style as the data packs it: foreground, background and underline colors as
/// [`Color::bits_of`] packs them (0: unset), then the modifiers added, and those taken away
/// shifted 16 bits up.
type Packed = [u32; 4];

/// A built-in theme as [`super::load::of_text`] makes it from the TOML: before the derived tints
/// and marks, which a load adds to the theme as a whole.
struct Data {
    name: &'static str,
    palette: &'static [(&'static str, u32)],
    /// The styles that aren't [`Theme::empty`]'s.
    styles: &'static [(StyleKey, Packed)],
    syntax: &'static [(&'static str, Packed)],
    glyphs: &'static GlyphData,
}

/// [`Glyphs`], as static text.
struct GlyphData {
    margin: u16,
    heading: [(&'static str, &'static str); 6],
    code_pad: u16,
    bullets: &'static [&'static str],
    quote_bar: &'static str,
    rule: &'static str,
    table_border: TableBorder,
    task: [&'static str; 2],
    task_status: [&'static str; Task::STATUSES.len()],
    wrap_mark: &'static str,
    code_clip: [&'static str; 2],
    sort_mark: [&'static str; 2],
    image: &'static str,
    embed: &'static str,
    callout: [&'static str; CalloutKind::ALL.len()],
    fold: [&'static str; 2],
    gutter: [&'static str; 6],
}

/// The compiled themes: glyph sets the themes share by name, then each theme as
/// `"name" (GLYPHS) { palette: [...], styles: [...], syntax: [...] }`, its styles keyed by
/// [`StyleKey`] variant. Every field of [`GlyphData`] is spelled out, so a glyph ada adds fails
/// the build until the data is generated again.
macro_rules! compiled {
    (
        $(glyphs $set:ident { $($field:ident: $value:expr,)* })*
        $(theme $name:literal ($glyphs:ident) {
            palette: [$(($color:literal, $bits:literal),)*],
            styles: [$(($key:ident, $style:expr),)*],
            syntax: [$(($scope:literal, $rule:expr),)*],
        })*
    ) => {
        $(static $set: GlyphData = GlyphData { $($field: $value,)* };)*
        pub(super) static THEMES: &[Data] = &[$(Data {
            name: $name,
            palette: &[$(($color, $bits),)*],
            styles: &[$((StyleKey::$key, $style),)*],
            syntax: &[$(($scope, $rule),)*],
            glyphs: &$glyphs,
        },)*];
    };
}

mod dark;
use dark::THEMES;

/// The compiled built-in `name`, as [`super::load::of_text`] makes it from its TOML; `None` for a
/// name that isn't one.
pub(super) fn theme(name: &str) -> Option<Theme> {
    THEMES.iter().find(|d| d.name == name).map(build)
}

fn build(data: &Data) -> Theme {
    let mut theme = Theme::empty();
    theme.name = data.name.to_owned();
    theme.palette = (data.palette.iter())
        .map(|&(name, bits)| (name.to_owned(), Color::from_bits(bits)))
        .collect();
    for &(key, packed) in data.styles {
        theme.styles[key as usize] = unpack(packed);
    }
    theme.syntax = (data.syntax.iter())
        .map(|&(scope, packed)| (scope.to_owned(), unpack(packed)))
        .collect();
    let g = data.glyphs;
    let own = |s: &str| s.to_owned();
    theme.glyphs = Glyphs {
        margin: g.margin,
        heading: g.heading.map(|(a, b)| (own(a), own(b))),
        code_pad: g.code_pad,
        bullets: g.bullets.iter().copied().map(own).collect(),
        quote_bar: own(g.quote_bar),
        rule: own(g.rule),
        table_border: g.table_border,
        task: g.task.map(own),
        task_status: g.task_status.map(own),
        wrap_mark: own(g.wrap_mark),
        code_clip: g.code_clip.map(own),
        sort_mark: g.sort_mark.map(own),
        image: own(g.image),
        embed: own(g.embed),
        callout: g.callout.map(own),
        fold: g.fold.map(own),
        gutter: g.gutter.map(own),
    };
    theme
}

fn unpack([fg, bg, underline, modifiers]: Packed) -> Style {
    let color = |bits: u32| (bits != 0).then(|| Color::from_bits(bits));
    Style {
        fg: color(fg),
        bg: color(bg),
        underline_color: color(underline),
        add_modifier: Modifier::from_bits_truncate(modifiers as u16),
        sub_modifier: Modifier::from_bits_truncate((modifiers >> 16) as u16),
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;

    use super::*;
    use crate::theme::BUILTIN;
    use crate::theme::load::of_text;

    /// The dark built-ins as their TOML loads them, before the derived passes.
    fn from_toml() -> Vec<Theme> {
        BUILTIN
            .iter()
            .map(|(name, text)| {
                let mut warnings = Vec::new();
                let theme = of_text(&format!("builtin:{name}"), text, &mut warnings, 0)
                    .expect("the built-in themes are valid");
                assert!(warnings.is_empty(), "{name}: {warnings:?}");
                assert_eq!(theme.name, *name, "a built-in's `name` is its own");
                theme
            })
            .filter(|t| !t.is_light())
            .collect()
    }

    fn pack(s: Style) -> String {
        let modifiers = u32::from(s.add_modifier.bits()) | u32::from(s.sub_modifier.bits()) << 16;
        let hex = |c: Option<Color>| match Color::bits_of(c) {
            0 => "0".to_owned(),
            bits => format!("{bits:#010x}"),
        };
        let m = match modifiers {
            0 => "0".to_owned(),
            m => format!("{m:#x}"),
        };
        format!(
            "[{}, {}, {}, {m}]",
            hex(s.fg),
            hex(s.bg),
            hex(s.underline_color)
        )
    }

    /// `compiled/dark.rs` as it should read.
    fn source(themes: &[Theme]) -> String {
        let mut out = String::from(
            "//! @generated from the dark built-ins' TOML by `compiled_themes_are_their_toml`; \
             don't edit:\n//! `ADA_REGEN_THEMES=1` and that test write it again.\n\n\
             use super::*;\n\ncompiled! {\n",
        );
        let mut sets: Vec<&Glyphs> = Vec::new();
        for t in themes {
            if !sets.contains(&&t.glyphs) {
                sets.push(&t.glyphs);
            }
        }
        let q = |s: &str| format!("{s:?}");
        let list = |items: &[String]| format!("[{}]", items.join(", "));
        for (i, g) in sets.iter().enumerate() {
            let pairs = |a: &[(String, String)]| {
                list(
                    &a.iter()
                        .map(|(x, y)| format!("({}, {})", q(x), q(y)))
                        .collect::<Vec<_>>(),
                )
            };
            let strs = |a: &[String]| list(&a.iter().map(|s| q(s)).collect::<Vec<_>>());
            let _ = writeln!(out, "    glyphs GLYPHS_{i} {{");
            for (field, value) in [
                ("margin", g.margin.to_string()),
                ("heading", pairs(&g.heading)),
                ("code_pad", g.code_pad.to_string()),
                ("bullets", format!("&{}", strs(&g.bullets))),
                ("quote_bar", q(&g.quote_bar)),
                ("rule", q(&g.rule)),
                ("table_border", format!("TableBorder::{:?}", g.table_border)),
                ("task", strs(&g.task)),
                ("task_status", strs(&g.task_status)),
                ("wrap_mark", q(&g.wrap_mark)),
                ("code_clip", strs(&g.code_clip)),
                ("sort_mark", strs(&g.sort_mark)),
                ("image", q(&g.image)),
                ("embed", q(&g.embed)),
                ("callout", strs(&g.callout)),
                ("fold", strs(&g.fold)),
                ("gutter", strs(&g.gutter)),
            ] {
                let _ = writeln!(out, "        {field}: {value},");
            }
            out.push_str("    }\n");
        }
        let empty = Theme::empty();
        for t in themes {
            let set = sets.iter().position(|g| *g == &t.glyphs).unwrap();
            let _ = writeln!(out, "    theme {} (GLYPHS_{set}) {{", q(&t.name));
            out.push_str("        palette: [\n");
            for (name, color) in &t.palette {
                let _ = writeln!(out, "            ({}, {:#010x}),", q(name), color.bits());
            }
            out.push_str("        ],\n        styles: [\n");
            for (i, key) in StyleKey::ALL.iter().enumerate() {
                if t.styles[i] != empty.styles[i] {
                    let _ = writeln!(out, "            ({key:?}, {}),", pack(t.styles[i]));
                }
            }
            out.push_str("        ],\n        syntax: [\n");
            for (scope, style) in &t.syntax {
                let _ = writeln!(out, "            ({}, {}),", q(scope), pack(*style));
            }
            out.push_str("        ],\n    }\n");
        }
        out.push_str("}\n");
        out
    }

    /// The compiled themes are the dark built-ins' TOML, each as it loads; with
    /// `ADA_REGEN_THEMES=1`, writes `compiled/dark.rs` from the TOML instead.
    #[test]
    fn compiled_themes_are_their_toml() {
        let themes = from_toml();
        let want = source(&themes);
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/theme/compiled/dark.rs");
        if std::env::var_os("ADA_REGEN_THEMES").is_some() {
            std::fs::write(path, &want).unwrap();
            return;
        }
        assert!(
            include_str!("compiled/dark.rs") == want,
            "src/theme/compiled/dark.rs is stale: ADA_REGEN_THEMES=1 and this test write it again"
        );
        assert_eq!(THEMES.len(), themes.len());
        for t in &themes {
            let compiled = theme(&t.name).expect("compiled");
            assert_eq!(format!("{compiled:?}"), format!("{t:?}"), "{}", t.name);
        }
    }

    /// The light built-ins stay TOML, and a name that isn't a built-in has no compiled theme.
    #[test]
    fn only_the_dark_built_ins_are_compiled() {
        assert!(theme("gruvbox-dark").is_some());
        assert!(theme("gruvbox-light").is_none());
        assert!(theme("catppuccin-latte").is_none());
        assert!(theme("nope").is_none());
    }
}
