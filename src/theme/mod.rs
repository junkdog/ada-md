//! Themes (ADR-03, ADR-08). Layout emits semantic [`StyleKey`]s and paint resolves them here, so a
//! color-only theme switch needs no relayout. Glyphs change widths, so they are a layout input.

mod load;

use std::hash::{DefaultHasher, Hash, Hasher};

use ratatui::style::{Color, Style};

use crate::doc::{Callout, InlineStyle};

pub use load::{BUILTIN, load, load_all, load_str};

macro_rules! style_keys {
    ($($variant:ident => $path:literal,)*) => {
        /// A semantic style slot. Resolving one is an array index.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum StyleKey { $($variant,)* }

        impl StyleKey {
            pub const ALL: &[StyleKey] = &[$(StyleKey::$variant,)*];
            pub const COUNT: usize = StyleKey::ALL.len();

            /// Where the style lives in the theme TOML, e.g. `inline.emph`.
            pub const fn path(self) -> &'static str {
                match self { $(StyleKey::$variant => $path,)* }
            }
        }
    };
}

style_keys! {
    Text => "document",
    H1 => "heading.1",
    H2 => "heading.2",
    H3 => "heading.3",
    H4 => "heading.4",
    H5 => "heading.5",
    H6 => "heading.6",
    Emph => "inline.emph",
    Strong => "inline.strong",
    Strike => "inline.strike",
    CodeInline => "inline.code",
    Link => "inline.link",
    LinkFocused => "inline.link_focused",
    LinkBroken => "inline.link_broken",
    Image => "inline.image",
    FootnoteRef => "inline.footnote_ref",
    Quote => "block.quote",
    QuoteBar => "block.quote_bar",
    CalloutNote => "block.callout_note",
    CalloutTip => "block.callout_tip",
    CalloutImportant => "block.callout_important",
    CalloutWarning => "block.callout_warning",
    CalloutCaution => "block.callout_caution",
    CodeBlock => "block.code",
    CodeText => "block.code_text",
    CodeLabel => "block.code_label",
    CodeWrapMark => "block.code_wrap_mark",
    ListBullet => "block.list_bullet",
    ListNumber => "block.list_number",
    TaskDone => "block.task_done",
    TaskOpen => "block.task_open",
    Rule => "block.rule",
    TableBorder => "block.table_border",
    TableHeader => "block.table_header",
    TableRowAlt => "block.table_row_alt",
    FootnoteLabel => "block.footnote_label",
    Html => "block.html",
    ImageFrame => "block.image_frame",
    GutterUnstagedAdded => "gutter.unstaged.added",
    GutterUnstagedModified => "gutter.unstaged.modified",
    GutterUnstagedDeleted => "gutter.unstaged.deleted",
    GutterStagedAdded => "gutter.staged.added",
    GutterStagedModified => "gutter.staged.modified",
    GutterStagedDeleted => "gutter.staged.deleted",
    LineNumber => "ui.line_number",
    StatusBar => "ui.status_bar",
    StatusFile => "ui.status_file",
    StatusHeading => "ui.status_heading",
    StatusPos => "ui.status_pos",
    StatusMessage => "ui.status_message",
    StatusLink => "ui.status_link",
    SearchMatch => "ui.search_match",
    SearchMatchHidden => "ui.search_match_hidden",
    SearchCurrent => "ui.search_current",
    Prompt => "ui.prompt",
    PromptError => "ui.prompt_error",
    Popup => "ui.popup",
    PopupBorder => "ui.popup_border",
    PopupTitle => "ui.popup_title",
    PopupSelected => "ui.popup_selected",
    SortTable => "ui.sort_table",
}

impl StyleKey {
    pub fn heading(level: u8) -> StyleKey {
        match level {
            1 => StyleKey::H1,
            2 => StyleKey::H2,
            3 => StyleKey::H3,
            4 => StyleKey::H4,
            5 => StyleKey::H5,
            _ => StyleKey::H6,
        }
    }

    pub fn callout(callout: Callout) -> StyleKey {
        match callout {
            Callout::Note => StyleKey::CalloutNote,
            Callout::Tip => StyleKey::CalloutTip,
            Callout::Important => StyleKey::CalloutImportant,
            Callout::Warning => StyleKey::CalloutWarning,
            Callout::Caution => StyleKey::CalloutCaution,
        }
    }

    /// The key this one refines, so that themes may leave it out.
    fn parent(self) -> Option<StyleKey> {
        match self {
            StyleKey::CodeText => Some(StyleKey::CodeBlock),
            StyleKey::ImageFrame => Some(StyleKey::TableBorder),
            _ => None,
        }
    }

    /// The gutter style for a mark index (see [`crate::git::Mark::index`]).
    pub fn gutter(index: usize) -> StyleKey {
        StyleKey::ALL[StyleKey::GutterUnstagedAdded as usize + index.min(5)]
    }

    fn heading_level(self) -> Option<usize> {
        let level = self as usize;
        (StyleKey::H1 as usize..=StyleKey::H6 as usize)
            .contains(&level)
            .then(|| level - StyleKey::H1 as usize)
    }

    fn gutter_index(self) -> Option<usize> {
        let i = self as usize;
        (StyleKey::GutterUnstagedAdded as usize..=StyleKey::GutterStagedDeleted as usize)
            .contains(&i)
            .then(|| i - StyleKey::GutterUnstagedAdded as usize)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TableBorder {
    Rounded,
    Ascii,
    None,
}

/// Everything in a theme that changes widths, and therefore layout.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Glyphs {
    /// Columns of blank space left and right of the document.
    pub margin: u16,
    /// `(prefix, suffix)` per heading level.
    pub heading: [(String, String); 6],
    /// Padding on each side of inline code.
    pub code_pad: u16,
    pub bullets: Vec<String>,
    pub quote_bar: String,
    pub rule: String,
    pub table_border: TableBorder,
    /// `[done, open]` task boxes.
    pub task: [String; 2],
    pub wrap_mark: String,
    /// `[left, right]` marks for code clipped at the edges in no-wrap mode.
    pub code_clip: [String; 2],
    /// `[ascending, descending]` marks on the header of the column a table is sorted by.
    pub sort_mark: [String; 2],
    pub image: String,
    /// Gutter marks, in the order of the `Gutter*` style keys.
    pub gutter: [String; 6],
}

impl Default for Glyphs {
    fn default() -> Self {
        Self {
            margin: 2,
            heading: Default::default(),
            code_pad: 0,
            bullets: vec!["•".into()],
            quote_bar: "│ ".into(),
            rule: "─".into(),
            table_border: TableBorder::Rounded,
            task: ["[x] ".into(), "[ ] ".into()],
            wrap_mark: "↪".into(),
            code_clip: ["‹".into(), "›".into()],
            sort_mark: ["▲".into(), "▼".into()],
            image: "🖼 ".into(),
            gutter: Default::default(),
        }
    }
}

impl Glyphs {
    /// Changes only when glyphs differ; part of `LayoutParams`.
    pub fn generation(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub name: String,
    pub palette: Vec<(String, Color)>,
    pub styles: [Style; StyleKey::COUNT],
    pub glyphs: Glyphs,
    /// TextMate scope selector → style; [`crate::highlight::SyntaxTheme`] turns it into a
    /// syntect theme (ADR-07).
    pub syntax: Vec<(String, Style)>,
}

impl Theme {
    pub fn empty() -> Self {
        Self {
            name: String::new(),
            palette: Vec::new(),
            styles: [Style::new(); StyleKey::COUNT],
            glyphs: Glyphs::default(),
            syntax: Vec::new(),
        }
    }

    /// The built-in default theme.
    pub fn builtin() -> Self {
        let (name, text) = BUILTIN[0];
        load_str(&format!("builtin:{name}"), text, &mut Vec::new(), 0)
            .expect("the built-in theme is valid")
    }

    /// Every built-in theme, the default first.
    pub fn builtins() -> Vec<Theme> {
        BUILTIN
            .iter()
            .map(|(name, text)| {
                load_str(&format!("builtin:{name}"), text, &mut Vec::new(), 0)
                    .expect("the built-in themes are valid")
            })
            .collect()
    }

    /// A theme for a light background: its document background is light or, when it leaves
    /// the terminal's, its text is dark. A theme with neither in RGB counts as dark.
    pub fn is_light(&self) -> bool {
        // relative luminance, roughly; light above the middle gray
        let light = |c| match c {
            Some(Color::Rgb(r, g, b)) => {
                Some(299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b) > 128_000)
            }
            _ => None,
        };
        let text = self.styles[StyleKey::Text as usize];
        light(text.bg)
            .or_else(|| light(text.fg).map(|fg| !fg))
            .unwrap_or(false)
    }

    /// A key's style on top of the document style, and of its parent's.
    pub fn style(&self, key: StyleKey) -> Style {
        let base = self.styles[StyleKey::Text as usize];
        let base = match key.parent() {
            Some(parent) => base.patch(self.styles[parent as usize]),
            None => base,
        };
        base.patch(self.styles[key as usize])
    }

    /// The style of a laid-out span: its key, then inline markup, then link styling.
    pub fn span_style(&self, key: StyleKey, inline: InlineStyle, link: bool) -> Style {
        let mut style = self.style(key);
        for (flag, key) in [
            (InlineStyle::CODE, StyleKey::CodeInline),
            (InlineStyle::EMPH, StyleKey::Emph),
            (InlineStyle::STRONG, StyleKey::Strong),
            (InlineStyle::STRIKE, StyleKey::Strike),
        ] {
            if inline.contains(flag) {
                style = style.patch(self.styles[key as usize]);
            }
        }
        if link {
            style = style.patch(self.styles[StyleKey::Link as usize]);
        }
        style
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_light_or_dark_by_their_colors() {
        let light: Vec<String> = Theme::builtins()
            .into_iter()
            .filter(Theme::is_light)
            .map(|t| t.name)
            .collect();
        assert_eq!(
            light,
            [
                "gruvbox-light",
                "catppuccin-latte",
                "tokyonight-day",
                "rose-pine-dawn"
            ]
        );
    }

    #[test]
    fn style_paths_are_unique() {
        for (i, a) in StyleKey::ALL.iter().enumerate() {
            for b in &StyleKey::ALL[i + 1..] {
                assert_ne!(a.path(), b.path());
            }
        }
    }

    #[test]
    fn index_helpers() {
        assert_eq!(StyleKey::H3.heading_level(), Some(2));
        assert_eq!(StyleKey::Emph.heading_level(), None);
        assert_eq!(StyleKey::GutterStagedDeleted.gutter_index(), Some(5));
    }
}
