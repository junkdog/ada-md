//! Which markdown a note is read as (ADR-45): CommonMark with GFM, or Obsidian's, which gives
//! some of CommonMark's syntax another meaning. `auto` reads a note as Obsidian's when a
//! `.obsidian` folder is at or above it; `[markdown] flavor` and `--flavor` choose instead.
//!
//! The setting is [`Flavor`]; what the files around a note say is a [`Vault`], found by the effect
//! runner as it reads the note and recorded with it (ADR-19), so a replay reads it the same; the
//! two give the [`Dialect`] the note is parsed in.

use std::fs;
use std::path::Path;

/// The `flavor` setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Flavor {
    /// Obsidian's in an Obsidian vault, else CommonMark.
    #[default]
    Auto,
    CommonMark,
    Obsidian,
}

impl Flavor {
    pub const NAMES: &[&str] = &["auto", "commonmark", "obsidian"];

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "auto" => Some(Flavor::Auto),
            "commonmark" => Some(Flavor::CommonMark),
            "obsidian" => Some(Flavor::Obsidian),
            _ => None,
        }
    }

    /// How a note in `vault` is read under this setting. Obsidian's line breaks follow the
    /// vault's `strictLineBreaks`; outside a vault (stdin, `--flavor obsidian` elsewhere), they
    /// are Obsidian's default: shown.
    pub fn dialect(self, vault: Vault) -> Dialect {
        let strict = vault == Vault::ObsidianStrict;
        match (self, vault) {
            (Flavor::CommonMark, _) | (Flavor::Auto, Vault::None) => Dialect::CommonMark,
            _ => Dialect::Obsidian {
                line_breaks: !strict,
            },
        }
    }
}

/// What the files around a note say of how Obsidian reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Vault {
    /// No `.obsidian` folder at or above it, or no file at all (stdin).
    #[default]
    None,
    /// In a vault that shows single line breaks, Obsidian's default.
    Obsidian,
    /// In a vault whose `.obsidian/app.json` sets `strictLineBreaks`.
    ObsidianStrict,
}

impl Vault {
    pub fn name(self) -> &'static str {
        match self {
            Vault::None => "none",
            Vault::Obsidian => "obsidian",
            Vault::ObsidianStrict => "obsidian-strict",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "none" => Some(Vault::None),
            "obsidian" => Some(Vault::Obsidian),
            "obsidian-strict" => Some(Vault::ObsidianStrict),
            _ => None,
        }
    }

    /// The vault `path` is in: the nearest `.obsidian` folder at or above its canonical path,
    /// apart from where links are looked up (a link root needn't be a vault). `-` is stdin.
    pub fn of(path: &Path) -> Vault {
        if path == Path::new("-") {
            return Vault::None;
        }
        let Ok(real) = fs::canonicalize(path) else {
            return Vault::None;
        };
        for dir in real.ancestors().skip(1) {
            let config = dir.join(".obsidian");
            if config.is_dir() {
                let app = fs::read_to_string(config.join("app.json")).unwrap_or_default();
                return match strict_line_breaks(&app) {
                    true => Vault::ObsidianStrict,
                    false => Vault::Obsidian,
                };
            }
        }
        Vault::None
    }
}

/// Whether Obsidian's `app.json` turns strict line breaks on: `"strictLineBreaks": true`, as
/// Obsidian writes it. Anything else, a missing or unreadable file too, is its default, off.
fn strict_line_breaks(app: &str) -> bool {
    let key = "\"strictLineBreaks\"";
    app.match_indices(key).any(|(at, _)| {
        let rest = app[at + key.len()..].trim_start();
        rest.strip_prefix(':')
            .is_some_and(|v| v.trim_start().starts_with("true"))
    })
}

/// How a note's markdown is parsed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dialect {
    #[default]
    CommonMark,
    /// Obsidian's: `line_breaks` shows each single newline as a line break (OBS-001).
    Obsidian { line_breaks: bool },
}

impl Dialect {
    /// Whether a single newline in a paragraph breaks the line.
    pub fn line_breaks(self) -> bool {
        matches!(self, Dialect::Obsidian { line_breaks: true })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_and_the_vault_give_the_dialect() {
        let shown = Dialect::Obsidian { line_breaks: true };
        let strict = Dialect::Obsidian { line_breaks: false };
        assert_eq!(Flavor::Auto.dialect(Vault::None), Dialect::CommonMark);
        assert_eq!(Flavor::Auto.dialect(Vault::Obsidian), shown);
        assert_eq!(Flavor::Auto.dialect(Vault::ObsidianStrict), strict);
        assert_eq!(
            Flavor::CommonMark.dialect(Vault::Obsidian),
            Dialect::CommonMark
        );
        // outside a vault, Obsidian's default
        assert_eq!(Flavor::Obsidian.dialect(Vault::None), shown);
        assert_eq!(Flavor::Obsidian.dialect(Vault::ObsidianStrict), strict);
    }

    #[test]
    fn app_json_sets_strict_line_breaks() {
        assert!(strict_line_breaks("{\n  \"strictLineBreaks\": true\n}"));
        assert!(strict_line_breaks("{\"a\":1,\"strictLineBreaks\" :true}"));
        assert!(!strict_line_breaks("{\n  \"strictLineBreaks\": false\n}"));
        assert!(!strict_line_breaks("{\n  \"alwaysUpdateLinks\": true\n}"));
        assert!(!strict_line_breaks(""));
    }

    #[test]
    fn a_vault_is_found_above_the_note() {
        let dir = std::env::temp_dir().join(format!("ada-vault-{}", std::process::id()));
        let note = dir.join("notes/sub/a.md");
        fs::create_dir_all(note.parent().unwrap()).unwrap();
        fs::write(&note, "a\nb\n").unwrap();
        assert_eq!(Vault::of(&note), Vault::None);
        fs::create_dir_all(dir.join("notes/.obsidian")).unwrap();
        assert_eq!(Vault::of(&note), Vault::Obsidian);
        fs::write(
            dir.join("notes/.obsidian/app.json"),
            "{\n  \"strictLineBreaks\": true\n}",
        )
        .unwrap();
        assert_eq!(Vault::of(&note), Vault::ObsidianStrict);
        assert_eq!(Vault::of(Path::new("-")), Vault::None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn vault_names_read_back() {
        for v in [Vault::None, Vault::Obsidian, Vault::ObsidianStrict] {
            assert_eq!(Vault::parse(v.name()), Some(v));
        }
    }
}
