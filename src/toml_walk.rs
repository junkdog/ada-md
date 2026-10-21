//! Walking `toml`'s spanned `DeTable` by hand, without serde (ADR-08): shared by the theme
//! loader and the config file. Spans make errors precise, e.g. `config.toml:3:9: expected an
//! integer, found string`.

use std::env;
use std::fmt::Display;
use std::ops::Range;
use std::path::PathBuf;

use toml::Spanned;
use toml::de::{DeTable, DeValue};

pub type Value<'i> = Spanned<DeValue<'i>>;

/// `$XDG_CONFIG_HOME/ada`, falling back to `~/.config/ada`.
pub fn config_dir() -> Option<PathBuf> {
    let config = env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("ada"))
}

/// Parse `text`, naming `file` and the position in a syntax error.
pub fn parse<'i>(file: &str, text: &'i str) -> Result<Spanned<DeTable<'i>>, String> {
    DeTable::parse(text).map_err(|e| {
        let at = e.span().map_or_else(String::new, |s| {
            let (line, col) = position(text, s.start);
            format!(":{line}:{col}")
        });
        format!("{file}{at}: {}", e.message().trim())
    })
}

pub fn entry<'v, 'i>(table: &'v DeTable<'i>, key: &str) -> Option<&'v Value<'i>> {
    table
        .iter()
        .find(|(k, _)| k.get_ref().as_ref() == key)
        .map(|(_, v)| v)
}

/// 1-based line and column of a byte offset.
pub fn position(text: &str, offset: usize) -> (usize, usize) {
    // `str::floor_char_boundary`, stable only since 1.91.
    let mut end = offset.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let before = &text[..end];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, col)
}

/// The file being walked, for messages, and where warnings go.
pub struct Cx<'a> {
    pub file: &'a str,
    pub text: &'a str,
    pub warnings: &'a mut Vec<String>,
}

impl Cx<'_> {
    pub fn at(&self, span: Range<usize>, msg: impl Display) -> String {
        let (line, col) = position(self.text, span.start);
        format!("{}:{line}:{col}: {msg}", self.file)
    }

    pub fn warn(&mut self, span: Range<usize>, msg: impl Display) {
        let w = self.at(span, msg);
        self.warnings.push(w);
    }

    pub fn expected(&self, what: &str, v: &Value<'_>) -> String {
        self.at(
            v.span(),
            format_args!("expected {what}, found {}", v.get_ref().type_str()),
        )
    }

    pub fn table<'v, 'i>(&self, v: &'v Value<'i>) -> Result<&'v DeTable<'i>, String> {
        v.get_ref()
            .as_table()
            .ok_or_else(|| self.expected("a table", v))
    }

    pub fn string<'v>(&self, v: &'v Value<'_>) -> Result<&'v str, String> {
        v.get_ref()
            .as_str()
            .ok_or_else(|| self.expected("a string", v))
    }

    /// A non-empty string, for glyphs that are repeated or must take up space.
    pub fn glyph(&self, v: &Value<'_>) -> Result<String, String> {
        let s = self.string(v)?;
        if unicode_width::UnicodeWidthStr::width(s) == 0 {
            return Err(self.at(v.span(), "glyph must be at least one column wide"));
        }
        Ok(s.to_owned())
    }

    pub fn strings(&self, v: &Value<'_>) -> Result<Vec<String>, String> {
        let array = v
            .get_ref()
            .as_array()
            .ok_or_else(|| self.expected("an array of strings", v))?;
        array
            .into_iter()
            .map(|item| self.string(item).map(str::to_owned))
            .collect()
    }

    /// A string, or an array of them.
    pub fn one_or_more(&self, v: &Value<'_>) -> Result<Vec<String>, String> {
        match v.get_ref() {
            DeValue::String(s) => Ok(vec![s.to_string()]),
            DeValue::Array(_) => self.strings(v),
            _ => Err(self.expected("a string or an array of strings", v)),
        }
    }

    pub fn bool(&self, v: &Value<'_>) -> Result<bool, String> {
        v.get_ref()
            .as_bool()
            .ok_or_else(|| self.expected("true or false", v))
    }

    pub fn int(&self, v: &Value<'_>) -> Result<i64, String> {
        let i = v
            .get_ref()
            .as_integer()
            .ok_or_else(|| self.expected("an integer", v))?;
        i64::from_str_radix(i.as_str(), i.radix())
            .map_err(|e| self.at(v.span(), format_args!("bad integer: {e}")))
    }

    pub fn u16(&self, v: &Value<'_>) -> Result<u16, String> {
        u16::try_from(self.int(v)?)
            .map_err(|_| self.at(v.span(), "expected a number from 0 to 65535"))
    }
}
