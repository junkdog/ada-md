//! Themes (ADR-03, ADR-08). Layout emits semantic [`StyleKey`]s and paint resolves them here, so a
//! color-only theme switch needs no relayout. Glyphs change widths, so they are a layout input.

mod load;

use std::hash::{DefaultHasher, Hash, Hasher};

use crate::style::{Color, Kind, Modifier, Style};

use crate::doc::{Callout, InlineStyle};

pub use load::{BUILTIN, load, load_str, load_user};

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
    DiagramLine => "block.diagram_line",
    DiagramActor => "block.diagram_actor",
    DiagramLabel => "block.diagram_label",
    DiagramNote => "block.diagram_note",
    DiagramFrame => "block.diagram_frame",
    DiagramNumber => "block.diagram_number",
    DiagramBar => "block.diagram_bar",
    DiagramActive => "block.diagram_active",
    DiagramDone => "block.diagram_done",
    DiagramCrit => "block.diagram_crit",
    DiagramGrid => "block.diagram_grid",
    DiagramType => "block.diagram_type",
    DiagramParam => "block.diagram_param",
    DiagramStatic => "block.diagram_static",
    DiagramAbstract => "block.diagram_abstract",
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
    StatusChanged => "ui.status_changed",
    SearchMatch => "ui.search_match",
    SearchMatchHidden => "ui.search_match_hidden",
    SearchCurrent => "ui.search_current",
    Prompt => "ui.prompt",
    PromptError => "ui.prompt_error",
    Popup => "ui.popup",
    PopupBorder => "ui.popup_border",
    PopupTitle => "ui.popup_title",
    PopupSelected => "ui.popup_selected",
    SettingsLabel => "ui.settings.label",
    SettingsFocus => "ui.settings.focus",
    SettingsField => "ui.settings.field",
    SettingsSwitchOn => "ui.settings.switch_on",
    SettingsSwitchOff => "ui.settings.switch_off",
    SettingsChoice => "ui.settings.choice",
    SettingsChanged => "ui.settings.changed",
    SortTable => "ui.sort_table",
    Selection => "ui.selection",
    Landed => "ui.landed",
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
            StyleKey::DiagramLine => Some(StyleKey::TableBorder),
            StyleKey::DiagramActor => Some(StyleKey::TableHeader),
            StyleKey::DiagramLabel => Some(StyleKey::Text),
            StyleKey::DiagramNote => Some(StyleKey::Quote),
            StyleKey::DiagramFrame => Some(StyleKey::CodeLabel),
            StyleKey::DiagramNumber => Some(StyleKey::ListNumber),
            StyleKey::DiagramBar => Some(StyleKey::CalloutNote),
            StyleKey::DiagramActive => Some(StyleKey::CalloutWarning),
            StyleKey::DiagramDone => Some(StyleKey::CalloutTip),
            StyleKey::DiagramCrit => Some(StyleKey::CalloutCaution),
            StyleKey::DiagramGrid => Some(StyleKey::TableBorder),
            StyleKey::DiagramType
            | StyleKey::DiagramParam
            | StyleKey::DiagramStatic
            | StyleKey::DiagramAbstract => Some(StyleKey::DiagramLabel),
            StyleKey::SettingsLabel
            | StyleKey::SettingsFocus
            | StyleKey::SettingsField
            | StyleKey::SettingsChanged => Some(StyleKey::Popup),
            StyleKey::SettingsSwitchOn | StyleKey::SettingsSwitchOff | StyleKey::SettingsChoice => {
                Some(StyleKey::SettingsField)
            }
            // the mark sits on the file name
            StyleKey::StatusChanged => Some(StyleKey::StatusFile),
            // only its background is used, so a theme without it tints with this one's
            StyleKey::Landed => Some(StyleKey::SearchMatchHidden),
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
    /// The path it was asked for by, as given (`--theme ~/x/mine.toml`), so saving the config
    /// writes that and not the name, which wouldn't load; `None` for a built-in or a theme in
    /// `themes/`, asked for by name.
    pub path: Option<String>,
    pub palette: Vec<(String, Color)>,
    pub styles: [Style; StyleKey::COUNT],
    pub glyphs: Glyphs,
    /// TextMate scope selector → style; [`crate::highlight::SyntaxTheme`] turns it into a
    /// syntect theme (ADR-07).
    pub syntax: Vec<(String, Style)>,
}

impl Theme {
    pub fn empty() -> Self {
        let mut styles = [Style::new(); StyleKey::COUNT];
        // UML's: static members underlined, abstract ones italic, whatever a theme's colors
        styles[StyleKey::DiagramStatic as usize] = Style::new().add_modifier(Modifier::UNDERLINED);
        styles[StyleKey::DiagramAbstract as usize] = Style::new().add_modifier(Modifier::ITALIC);
        Self {
            name: String::new(),
            path: None,
            palette: Vec::new(),
            styles,
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

    /// Every theme `T` lists: the built-ins, then `others` (the user's, from [`load_user`]),
    /// with `current` among them. A theme named like a built-in takes its place.
    pub fn with_builtins(current: &Theme, others: Vec<Theme>) -> Vec<Theme> {
        let mut themes = Theme::builtins();
        let mut rest = Vec::new();
        for theme in others {
            match themes.iter_mut().find(|t| t.name == theme.name) {
                Some(slot) => *slot = theme,
                None => rest.push(theme),
            }
        }
        themes.extend(rest);
        match themes.iter_mut().find(|t| t.name == current.name) {
            Some(slot) => *slot = current.clone(),
            None => themes.insert(0, current.clone()),
        }
        themes
    }

    /// A theme for a light background: its document background is light or, when it leaves
    /// the terminal's, its text is dark. A theme with neither in RGB counts as dark.
    pub fn is_light(&self) -> bool {
        // relative luminance, roughly; light above the middle gray
        let light = |c: Option<Color>| match c.map(Color::kind) {
            Some(Kind::Rgb(r, g, b)) => {
                Some(299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b) > 128_000)
            }
            _ => None,
        };
        let text = self.styles[StyleKey::Text as usize];
        light(text.bg)
            .or_else(|| light(text.fg).map(|fg| !fg))
            .unwrap_or(false)
    }

    /// The theme with its own background, or (`theme_bg = false`) on the terminal's: the
    /// document background goes, and with it every style's background that repeats it, so the
    /// margins, the gutter and the blanks show the terminal's. Backgrounds that set something
    /// apart (code, heading bars, stripes, the status bar, popups, matches) stay. Resolved
    /// when the theme loads, so paint is as ever.
    pub fn with_bg(&self, own: bool) -> Theme {
        let mut theme = self.clone();
        if own {
            return theme;
        }
        let text = StyleKey::Text as usize;
        let Some(bg) = theme.styles[text].bg.take() else {
            return theme;
        };
        for &key in StyleKey::ALL {
            if theme.styles[key as usize].bg != Some(bg) {
                continue;
            }
            // a parent with a background of its own would show through: reset to the terminal's
            let parent = key.parent().and_then(|p| self.styles[p as usize].bg);
            theme.styles[key as usize].bg = parent.filter(|&p| p != bg).map(|_| Color::RESET);
        }
        theme
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
            (InlineStyle::STATIC, StyleKey::DiagramStatic),
            (InlineStyle::ABSTRACT, StyleKey::DiagramAbstract),
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
    fn with_builtins_lists_the_built_ins_then_the_users_with_the_current_one_in_its_place() {
        let named = |name: &str| Theme {
            name: name.into(),
            ..Theme::builtin()
        };
        let names = |themes: &[Theme]| themes.iter().map(|t| t.name.clone()).collect::<Vec<_>>();
        let builtins = names(&Theme::builtins());

        // a user theme named like a built-in takes its place; the others follow the built-ins
        let mine = Theme {
            palette: vec![("mine".into(), Color::RESET)],
            ..named("dracula")
        };
        let themes = Theme::with_builtins(&Theme::builtin(), vec![mine, named("zzz")]);
        assert_eq!(
            names(&themes),
            [&builtins[..], &["zzz".to_owned()]].concat()
        );
        let dracula = themes.iter().find(|t| t.name == "dracula").unwrap();
        assert_eq!(dracula.palette, [("mine".to_owned(), Color::RESET)]);

        // a current theme from a path is listed first
        let themes = Theme::with_builtins(&named("mine"), Vec::new());
        assert_eq!(themes[0].name, "mine");
        assert_eq!(themes.len(), builtins.len() + 1);
    }

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
    fn every_builtin_has_a_background_the_toggle_takes_away() {
        for theme in Theme::builtins() {
            let bg = theme.styles[StyleKey::Text as usize].bg;
            assert!(bg.is_some(), "{} has no background", theme.name);
            assert_eq!(theme.with_bg(true).styles, theme.styles, "{}", theme.name);

            let off = theme.with_bg(false);
            assert_eq!(off.is_light(), theme.is_light(), "{}", theme.name);
            for &key in StyleKey::ALL {
                let (was, now) = (theme.style(key), off.style(key));
                assert_ne!(
                    now.bg,
                    bg,
                    "{}: {} keeps the background",
                    theme.name,
                    key.path()
                );
                // only the document's background goes
                if was.bg != bg {
                    assert_eq!(now, was, "{}: {}", theme.name, key.path());
                } else {
                    assert_eq!(
                        now,
                        Style { bg: None, ..was },
                        "{}: {}",
                        theme.name,
                        key.path()
                    );
                }
            }
        }
    }

    #[test]
    fn a_style_repeating_the_background_shows_the_terminals() {
        let bg = Color::rgb(1, 2, 3);
        let mut theme = Theme::empty();
        theme.styles[StyleKey::Text as usize].bg = Some(bg);
        theme.styles[StyleKey::Emph as usize].bg = Some(bg);
        theme.styles[StyleKey::CodeBlock as usize].bg = Some(Color::BLUE);
        theme.styles[StyleKey::CodeText as usize].bg = Some(bg);
        let off = theme.with_bg(false);
        assert_eq!(off.style(StyleKey::Text).bg, None);
        assert_eq!(off.style(StyleKey::Emph).bg, None);
        // over a parent of another color, the terminal's background is set outright
        assert_eq!(off.style(StyleKey::CodeText).bg, Some(Color::RESET));
        assert_eq!(off.style(StyleKey::CodeBlock).bg, Some(Color::BLUE));

        // a theme without a background is left as it is
        let plain = Theme::empty();
        assert_eq!(plain.with_bg(false).styles, plain.styles);
    }

    /// WCAG's contrast ratio between two colors, 1 to 21.
    fn contrast(a: Color, b: Color) -> f64 {
        let luminance = |c: Color| {
            let Kind::Rgb(r, g, b) = c.kind() else {
                panic!("{c:?} isn't RGB");
            };
            let lin = |v: u8| {
                let v = f64::from(v) / 255.0;
                if v <= 0.039_28 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// The settings popup's slots, in every built-in with and without its background: values
    /// read on their field, labels and marks on the popup and on the focused row, and the field,
    /// the focused row and the popup tell apart.
    #[test]
    fn settings_slots_read_in_every_builtin() {
        for designed in Theme::builtins() {
            for theme in [designed.clone(), designed.with_bg(false)] {
                let name = &theme.name;
                let fg = |k| theme.style(k).fg.unwrap();
                let bg = |k| theme.style(k).bg.unwrap();
                let (popup, focus, field) = (
                    bg(StyleKey::Popup),
                    bg(StyleKey::SettingsFocus),
                    bg(StyleKey::SettingsField),
                );
                assert_eq!(bg(StyleKey::SettingsSwitchOn), field, "{name}");
                assert_eq!(bg(StyleKey::SettingsSwitchOff), field, "{name}");
                for (what, a, b) in [
                    ("field and popup", field, popup),
                    ("field and focus", field, focus),
                    ("focus and popup", focus, popup),
                ] {
                    assert!(contrast(a, b) > 1.05, "{name}: {what} {a:?} {b:?}");
                }
                for (what, key, on, min) in [
                    ("value", StyleKey::SettingsField, field, 4.5),
                    ("switch on", StyleKey::SettingsSwitchOn, field, 3.0),
                    ("switch off", StyleKey::SettingsSwitchOff, field, 1.8),
                    (
                        "choice",
                        StyleKey::SettingsChoice,
                        bg(StyleKey::SettingsChoice),
                        3.0,
                    ),
                    ("label", StyleKey::SettingsLabel, popup, 3.0),
                    // bold: WCAG asks 3 of bold text; tokyonight-day's own popup text is at 3.99
                    ("focused label", StyleKey::SettingsFocus, focus, 3.5),
                    ("mark", StyleKey::SettingsChanged, popup, 2.5),
                    ("focused mark", StyleKey::SettingsChanged, focus, 2.0),
                ] {
                    let ratio = contrast(fg(key), on);
                    assert!(ratio >= min, "{name}: {what} {ratio:.2} < {min}");
                }
                // on and off differ in lightness, not only in hue; the knob's side and the word
                // beside it say which, for eyes that tell neither
                let (on, off) = (
                    fg(StyleKey::SettingsSwitchOn),
                    fg(StyleKey::SettingsSwitchOff),
                );
                assert!(contrast(on, off) > 1.2, "{name}: on and off look alike");
            }
        }
    }

    /// The mark on a file changed on disk (FEAT-014) reads on the file name's background, and
    /// isn't the name's own color, in every built-in with and without its background.
    #[test]
    fn changed_mark_reads_on_the_file_name_in_every_builtin() {
        for designed in Theme::builtins() {
            for theme in [designed.clone(), designed.with_bg(false)] {
                let name = &theme.name;
                let (file, mark) = (
                    theme.style(StyleKey::StatusFile),
                    theme.style(StyleKey::StatusChanged),
                );
                assert_eq!(mark.bg, file.bg, "{name}: the mark sits on the file name");
                let ratio = contrast(mark.fg.unwrap(), file.bg.unwrap());
                assert!(ratio >= 3.0, "{name}: mark {ratio:.2} < 3");
                assert_ne!(mark.fg, file.fg, "{name}: the mark looks like the name");
            }
        }
    }

    /// Where a jump landed shows against the page and a code block's background, and what lands
    /// there still reads on it: text, headings, a footnote's label, a link. Each keeps WCAG's 4.5
    /// (text) or 3 (the rest), or, where the theme's own page gives it less (the light themes'
    /// headings), three quarters of that. The first heading level paints its own background,
    /// which the tint leaves alone.
    #[test]
    fn landing_tint_reads_in_every_builtin() {
        for theme in Theme::builtins() {
            let name = &theme.name;
            let page = theme.style(StyleKey::Text).bg.unwrap();
            let tint = theme.style(StyleKey::Landed).bg.unwrap();
            let shows = contrast(tint, page);
            assert!(shows >= 1.15, "{name}: the tint barely shows, {shows:.2}");
            // a code line is tinted on its block's background
            let block = theme.style(StyleKey::CodeBlock).bg.unwrap();
            let on_code = contrast(tint, block);
            assert!(
                on_code >= 1.15,
                "{name}: on code the tint barely shows, {on_code:.2}"
            );
            for (key, min) in [
                (StyleKey::Text, 4.5),
                (StyleKey::H2, 3.0),
                (StyleKey::H3, 3.0),
                (StyleKey::FootnoteLabel, 3.0),
                (StyleKey::Link, 3.0),
            ] {
                let fg = theme.style(key).fg.unwrap();
                let (on_page, on_tint) = (contrast(fg, page), contrast(fg, tint));
                let min = f64::min(min, 0.75 * on_page);
                assert!(
                    on_tint >= min,
                    "{name}: {} {on_tint:.2} < {min:.2}",
                    key.path()
                );
            }
        }
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
