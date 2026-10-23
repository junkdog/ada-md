//! Keys → semantic messages (plan §6). Pure: the only state is a pending prefix key for the
//! two-key bindings like `]]` and `]c`, and the mode with what has been typed into the search
//! prompt or the outline filter. Recordings log the messages, not the keys, so they survive keymap
//! changes.
//!
//! The keymap runs on the input thread, so it tracks the mode itself: `/` opens the prompt here
//! and now, and the keys typed right after it can't race the main loop. The one mode it can't
//! know is the confirm box `S` opens, which opens only when saving would change the file: the
//! input thread waits for the main loop's answer, and [`Keymap::no_confirm`] takes the box back.
//! Likewise, a change in the settings popup may open the theme list (its theme row): the input
//! thread waits to hear, and [`Keymap::theme_list`] hands the keys to the list.
//!
//! The mouse maps here too, by the same mode: the wheel scrolls what it shows, and a left click
//! in normal mode is a [`Msg::Click`] that `update` resolves against the layout.
//!
//! Normal-mode keys come from [`Bindings`]: one table of actions with their default keys, which
//! the config file's `[keys]` section overrides, and which the help overlay lists.

use std::fmt;
use std::mem;
use std::time::{Duration, Instant};

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use super::{CODE_X_STEP, Dir, Msg, ScrollBy};

/// Rows `PgUp` / `PgDn` move the outline selection.
const OUTLINE_PAGE: i32 = 10;
/// Rows `PgUp` / `PgDn` scroll the help.
const HELP_PAGE: i32 = 10;
/// Rows a notch of the wheel scrolls the document or the help; in a list it moves one entry.
const WHEEL_ROWS: i32 = 3;
/// Presses on one cell this close together count as a double or triple click.
pub const MULTI_CLICK: Duration = Duration::from_millis(500);

macro_rules! actions {
    ($($group:literal { $($variant:ident => $name:literal, $help:literal, [$($key:literal),*],)* })*) => {
        /// Something a normal-mode key does.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Action { $($($variant,)*)* }

        impl Action {
            pub const ALL: &[Action] = &[$($(Action::$variant,)*)*];

            /// The name in the config file's `[keys]` section.
            pub const fn name(self) -> &'static str {
                match self { $($(Action::$variant => $name,)*)* }
            }

            pub const fn help(self) -> &'static str {
                match self { $($(Action::$variant => $help,)*)* }
            }

            /// The help overlay's section.
            pub const fn group(self) -> &'static str {
                match self { $($(Action::$variant => $group,)*)* }
            }

            const fn default_keys(self) -> &'static [&'static str] {
                match self { $($(Action::$variant => &[$($key),*],)*)* }
            }
        }
    };
}

actions! {
    "Moving" {
        ScrollDown => "scroll_down", "scroll down a row", ["j", "down", "ctrl-e"],
        ScrollUp => "scroll_up", "scroll up a row", ["k", "up", "ctrl-y"],
        HalfPageDown => "half_page_down", "half a page down", ["d", "ctrl-d"],
        HalfPageUp => "half_page_up", "half a page up", ["u", "ctrl-u"],
        PageDown => "page_down", "a page down", ["space", "pagedown", "ctrl-f"],
        PageUp => "page_up", "a page up", ["b", "pageup", "ctrl-b"],
        Top => "top", "the top", ["g", "home"],
        Bottom => "bottom", "the bottom", ["G", "end"],
        NextHeading => "next_heading", "the next heading", ["]]"],
        PrevHeading => "prev_heading", "the previous heading", ["[["],
        NextChange => "next_change", "the next git change", ["]c"],
        PrevChange => "prev_change", "the previous git change", ["[c"],
    }
    "Links and history" {
        NextLink => "next_link", "focus the next link", ["tab"],
        PrevLink => "prev_link", "focus the previous link", ["shift-tab"],
        Follow => "follow", "follow the focused link", ["enter"],
        Back => "back", "go back", ["backspace", "H", "ctrl-o"],
        Forward => "forward", "go forward", ["L"],
    }
    "Search and outline" {
        Search => "search", "search with a regex", ["/"],
        SearchNext => "search_next", "the next match", ["n"],
        SearchPrev => "search_prev", "the previous match", ["N"],
        Outline => "outline", "the outline of headings", ["o"],
        Links => "links", "the list of links", ["l"],
    }
    "View" {
        TableMode => "table_mode", "tables: auto, grid, records", ["t"],
        Sort => "sort", "sort a table on screen by a column", ["s"],
        CodeWrap => "code_wrap", "wrap code, or scroll it sideways", ["w"],
        CodeLeft => "code_left", "scroll code left", ["left"],
        CodeRight => "code_right", "scroll code right", ["right"],
        LineNumbers => "line_numbers", "show or hide line numbers", ["#"],
        ThemeBg => "theme_bg", "theme background, or the terminal's", ["B"],
        Theme => "theme", "pick a theme, previewing each", ["T"],
    }
    "Other" {
        Reload => "reload", "read the file again", ["r"],
        Edit => "edit", "edit in $VISUAL or $EDITOR", ["e"],
        Yank => "yank", "copy a selection, link or code block", ["y"],
        Settings => "settings", "see, change and save the settings", ["f2", ","],
        SaveConfig => "save_config", "save the settings to the config file", ["S"],
        Mouse => "mouse", "the mouse to ada, or to the terminal", ["M"],
        Help => "help", "this help", ["?"],
        Quit => "quit", "quit; Esc clears a selection first", ["q", "esc"],
    }
}

impl Action {
    pub fn from_name(name: &str) -> Option<Action> {
        Action::ALL.iter().copied().find(|a| a.name() == name)
    }

    fn msg(self) -> Msg {
        let scroll = |by| Msg::Scroll { by };
        match self {
            Action::ScrollDown => scroll(ScrollBy::Rows(1)),
            Action::ScrollUp => scroll(ScrollBy::Rows(-1)),
            Action::HalfPageDown => scroll(ScrollBy::HalfPage(Dir::Next)),
            Action::HalfPageUp => scroll(ScrollBy::HalfPage(Dir::Prev)),
            Action::PageDown => scroll(ScrollBy::Page(Dir::Next)),
            Action::PageUp => scroll(ScrollBy::Page(Dir::Prev)),
            Action::Top => scroll(ScrollBy::Top),
            Action::Bottom => scroll(ScrollBy::Bottom),
            Action::NextHeading => Msg::JumpHeading { dir: Dir::Next },
            Action::PrevHeading => Msg::JumpHeading { dir: Dir::Prev },
            Action::NextChange => Msg::JumpChange { dir: Dir::Next },
            Action::PrevChange => Msg::JumpChange { dir: Dir::Prev },
            Action::NextLink => Msg::FocusLink { dir: Dir::Next },
            Action::PrevLink => Msg::FocusLink { dir: Dir::Prev },
            Action::Follow => Msg::Follow,
            Action::Back => Msg::Back,
            Action::Forward => Msg::Forward,
            Action::Search => Msg::SearchOpen,
            Action::SearchNext => Msg::SearchNext { dir: Dir::Next },
            Action::SearchPrev => Msg::SearchNext { dir: Dir::Prev },
            Action::Outline => Msg::OpenOutline,
            Action::Links => Msg::OpenLinks,
            Action::TableMode => Msg::ToggleTableMode,
            Action::Sort => Msg::SortOpen,
            Action::CodeWrap => Msg::ToggleCodeWrap,
            Action::CodeLeft => Msg::ScrollCodeX { by: -CODE_X_STEP },
            Action::CodeRight => Msg::ScrollCodeX { by: CODE_X_STEP },
            Action::LineNumbers => Msg::ToggleLineNumbers,
            Action::ThemeBg => Msg::ToggleThemeBg,
            Action::Theme => Msg::OpenThemes,
            Action::Reload => Msg::Reload,
            Action::Edit => Msg::Edit,
            Action::Yank => Msg::Yank,
            Action::Settings => Msg::OpenSettings,
            Action::SaveConfig => Msg::SaveConfig,
            Action::Mouse => Msg::ToggleMouse,
            Action::Help => Msg::Help,
            Action::Quit => Msg::Quit,
        }
    }
}

/// One key with the modifiers that matter: Ctrl and Alt always, Shift only on keys that aren't
/// characters (a character's case already says it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    code: KeyCode,
    mods: KeyModifiers,
}

/// Named keys, as the config file spells them, and as the help shows them.
const NAMES: &[(&str, KeyCode, &str)] = &[
    ("enter", KeyCode::Enter, "Enter"),
    ("return", KeyCode::Enter, "Enter"),
    ("esc", KeyCode::Esc, "Esc"),
    ("escape", KeyCode::Esc, "Esc"),
    ("tab", KeyCode::Tab, "Tab"),
    ("backtab", KeyCode::BackTab, "S-Tab"),
    ("backspace", KeyCode::Backspace, "BS"),
    ("bs", KeyCode::Backspace, "BS"),
    ("space", KeyCode::Char(' '), "Space"),
    ("up", KeyCode::Up, "↑"),
    ("down", KeyCode::Down, "↓"),
    ("left", KeyCode::Left, "←"),
    ("right", KeyCode::Right, "→"),
    ("home", KeyCode::Home, "Home"),
    ("end", KeyCode::End, "End"),
    ("pageup", KeyCode::PageUp, "PgUp"),
    ("pgup", KeyCode::PageUp, "PgUp"),
    ("pagedown", KeyCode::PageDown, "PgDn"),
    ("pgdn", KeyCode::PageDown, "PgDn"),
    ("delete", KeyCode::Delete, "Del"),
    ("del", KeyCode::Delete, "Del"),
    ("insert", KeyCode::Insert, "Ins"),
];

impl Key {
    fn new(code: KeyCode, mods: KeyModifiers) -> Self {
        let mut mods = mods & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        if matches!(code, KeyCode::Char(_) | KeyCode::BackTab) {
            mods.remove(KeyModifiers::SHIFT);
        }
        // Shift-Tab arrives as BackTab
        let code = match code {
            KeyCode::Tab if mods.contains(KeyModifiers::SHIFT) => {
                mods.remove(KeyModifiers::SHIFT);
                KeyCode::BackTab
            }
            code => code,
        };
        Self { code, mods }
    }

    fn of(event: &KeyEvent) -> Self {
        Self::new(event.code, event.modifiers)
    }

    fn is_ctrl_c(self) -> bool {
        self == Key::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
    }

    /// `ctrl-d`, `C-d`, `alt-x`, `shift-up`, `S-Tab`, `pagedown`, `f5`, `G`, `/`.
    pub fn parse(spec: &str) -> Option<Self> {
        let mut mods = KeyModifiers::NONE;
        let mut rest = spec;
        while let Some((prefix, tail)) = rest.split_once('-').filter(|(_, t)| !t.is_empty()) {
            mods |= match prefix.to_ascii_lowercase().as_str() {
                "ctrl" | "c" => KeyModifiers::CONTROL,
                "alt" | "a" | "meta" | "m" => KeyModifiers::ALT,
                "shift" | "s" => KeyModifiers::SHIFT,
                _ => break,
            };
            rest = tail;
        }
        let lower = rest.to_ascii_lowercase();
        let code = if let Some(&(_, code, _)) = NAMES.iter().find(|(n, ..)| *n == lower) {
            code
        } else if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
            (1..=12).contains(&n).then_some(KeyCode::F(n))?
        } else {
            let mut chars = rest.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => {
                    // ctrl and alt combine with the base key, whatever the terminal sends
                    let c = if mods.is_empty() {
                        c
                    } else {
                        c.to_ascii_lowercase()
                    };
                    KeyCode::Char(c)
                }
                _ => return None,
            }
        };
        Some(Key::new(code, mods))
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.contains(KeyModifiers::CONTROL) {
            f.write_str("C-")?;
        }
        if self.mods.contains(KeyModifiers::ALT) {
            f.write_str("M-")?;
        }
        if self.mods.contains(KeyModifiers::SHIFT) {
            f.write_str("S-")?;
        }
        match NAMES.iter().find(|(_, code, _)| *code == self.code) {
            Some((_, _, shown)) => f.write_str(shown),
            None => match self.code {
                KeyCode::Char(c) => write!(f, "{c}"),
                KeyCode::F(n) => write!(f, "F{n}"),
                code => write!(f, "{code:?}"),
            },
        }
    }
}

/// One key, or two pressed in turn.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeySeq(Vec<Key>);

impl KeySeq {
    /// A key, two keys separated by a space (`g g`), or two plain characters (`]]`).
    pub fn parse(spec: &str) -> Option<Self> {
        let parts: Vec<&str> = spec.split_whitespace().collect();
        let keys = match parts.as_slice() {
            [one] => match Key::parse(one) {
                Some(key) => vec![key],
                None => {
                    let chars: Vec<char> = one.chars().collect();
                    match chars.as_slice() {
                        &[a, b] => vec![Key::new(KeyCode::Char(a), KeyModifiers::NONE), {
                            Key::new(KeyCode::Char(b), KeyModifiers::NONE)
                        }],
                        _ => return None,
                    }
                }
            },
            [a, b] => vec![Key::parse(a)?, Key::parse(b)?],
            _ => return None,
        };
        Some(Self(keys))
    }
}

impl fmt::Display for KeySeq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let plain = self
            .0
            .iter()
            .all(|k| k.mods.is_empty() && matches!(k.code, KeyCode::Char(c) if c != ' '));
        for (i, key) in self.0.iter().enumerate() {
            if i > 0 && !plain {
                f.write_str(" ")?;
            }
            write!(f, "{key}")?;
        }
        Ok(())
    }
}

/// Every normal-mode action with its keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bindings(Vec<(Action, Vec<KeySeq>)>);

impl Default for Bindings {
    fn default() -> Self {
        Self(
            Action::ALL
                .iter()
                .map(|&a| {
                    let keys = a.default_keys().iter().filter_map(|k| KeySeq::parse(k));
                    (a, keys.collect())
                })
                .collect(),
        )
    }
}

impl Bindings {
    pub fn keys(&self, action: Action) -> &[KeySeq] {
        self.0
            .iter()
            .find(|(a, _)| *a == action)
            .map_or(&[], |(_, k)| k)
    }

    /// The default keys as the config file writes them.
    pub fn default_specs(action: Action) -> &'static [&'static str] {
        action.default_keys()
    }

    /// Replace the keys of the actions in `set`, taking those keys away from the actions it
    /// leaves at their defaults. Returns warnings about keys that can't work.
    pub fn rebind(&mut self, set: Vec<(Action, Vec<KeySeq>)>) -> Vec<String> {
        let mut warnings = Vec::new();
        let chosen: Vec<Action> = set.iter().map(|(a, _)| *a).collect();
        for (action, keys) in set {
            for (other, theirs) in &mut self.0 {
                if *other == action {
                    continue;
                }
                if chosen.contains(other) {
                    for k in theirs.iter().filter(|k| keys.contains(k)) {
                        warnings.push(format!(
                            "keys: {k} is bound to both {} and {}",
                            other.name(),
                            action.name()
                        ));
                    }
                } else {
                    theirs.retain(|k| !keys.contains(k));
                }
            }
            if let Some(slot) = self.0.iter_mut().find(|(a, _)| *a == action) {
                slot.1 = keys;
            }
        }
        for (action, keys) in &self.0 {
            for k in keys
                .iter()
                .filter(|k| k.0.len() == 1 && self.is_prefix(k.0[0]))
            {
                warnings.push(format!(
                    "keys: {k} ({}) also starts a two-key binding, so it can't work alone",
                    action.name()
                ));
            }
        }
        warnings
    }

    fn find(&self, keys: &[Key]) -> Option<Action> {
        self.0
            .iter()
            .find(|(_, seqs)| seqs.iter().any(|s| s.0 == keys))
            .map(|(a, _)| *a)
    }

    fn is_prefix(&self, key: Key) -> bool {
        self.0
            .iter()
            .flat_map(|(_, seqs)| seqs)
            .any(|s| s.0.len() == 2 && s.0[0] == key)
    }

    /// What the help shows: each group with its actions, and their keys.
    pub fn help(&self) -> Vec<(&'static str, Vec<(String, &'static str)>)> {
        let mut groups: Vec<(&'static str, Vec<(String, &'static str)>)> = Vec::new();
        for (action, keys) in &self.0 {
            let shown = keys
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ");
            let shown = if shown.is_empty() {
                "(unbound)".to_owned()
            } else {
                shown
            };
            match groups.last_mut() {
                Some((g, entries)) if *g == action.group() => entries.push((shown, action.help())),
                _ => groups.push((action.group(), vec![(shown, action.help())])),
            }
        }
        groups
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Normal,
    /// The search prompt, with the pattern typed so far.
    Search(String),
    /// The outline popup, of headings, links or themes, with its filter.
    Outline(String),
    Help,
    /// The outline popup listing a table's columns, which `←`/`→` trade for another table's.
    Sort(String),
    /// The outline popup listing the themes, which `←`/`→` trade for the dark or light ones.
    Themes(String),
    /// The box asking whether to save the config: `Enter`/`y` or `Esc`/`n`/`q`.
    Confirm,
    /// The settings popup: moving through its rows, changing the focused one, and the ways out.
    Settings,
}

#[derive(Debug, Default)]
pub struct Keymap {
    bindings: Bindings,
    pending: Option<Key>,
    mode: Mode,
    /// The cell the left button went down on, until it's released or dragged off.
    pressed: Option<(u16, u16)>,
    /// The theme list is open over the settings popup, which takes the keys back when it closes.
    over_settings: bool,
    /// The left button went down in normal mode, so a drag selects, until it's released.
    selecting: bool,
    /// The button has been dragged since it went down.
    dragged: bool,
    /// The last press: its cell, when, and its count, which presses on the same cell in quick
    /// succession raise.
    last_press: Option<((u16, u16), Instant, u8)>,
}

/// Edit `text` with a prompt key. `None` when the key isn't an edit.
fn edit(text: &mut String, key: &KeyEvent) -> Option<()> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('u') if ctrl => text.clear(),
        KeyCode::Char('w') if ctrl => {
            let trimmed = text.trim_end().len();
            let start = text[..trimmed]
                .rfind(char::is_whitespace)
                .map_or(0, |i| i + 1);
            text.truncate(start);
        }
        KeyCode::Char(c) if !ctrl => text.push(c),
        KeyCode::Backspace => {
            text.pop();
        }
        _ => return None,
    }
    Some(())
}

impl Keymap {
    pub fn new(bindings: Bindings) -> Self {
        Self {
            bindings,
            ..Self::default()
        }
    }

    pub fn map(&mut self, key: KeyEvent) -> Option<Msg> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        let sorting = matches!(self.mode, Mode::Sort(_));
        let themes = matches!(self.mode, Mode::Themes(_));
        match &mut self.mode {
            Mode::Normal => self.normal(&key),
            Mode::Search(text) => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Enter => {
                        self.mode = Mode::Normal;
                        Some(Msg::SearchSubmit)
                    }
                    KeyCode::Esc => {
                        self.mode = Mode::Normal;
                        Some(Msg::SearchCancel)
                    }
                    KeyCode::Char('c') if ctrl => {
                        self.mode = Mode::Normal;
                        Some(Msg::SearchCancel)
                    }
                    // Backspace on an empty prompt closes it, like vim.
                    KeyCode::Backspace if text.is_empty() => {
                        self.mode = Mode::Normal;
                        Some(Msg::SearchCancel)
                    }
                    _ => {
                        edit(text, &key)?;
                        Some(Msg::SearchInput {
                            pattern: text.clone(),
                        })
                    }
                }
            }
            Mode::Outline(text) | Mode::Sort(text) | Mode::Themes(text) => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let by = |by| Some(Msg::OutlineMove { by });
                // another table's columns: the filter starts over
                let table = |text: &mut String, dir| {
                    text.clear();
                    Some(Msg::SortTable { dir })
                };
                match key.code {
                    KeyCode::Left if sorting => table(text, Dir::Prev),
                    KeyCode::Right if sorting => table(text, Dir::Next),
                    KeyCode::Left if themes => Some(Msg::ThemesTone { dir: Dir::Prev }),
                    KeyCode::Right if themes => Some(Msg::ThemesTone { dir: Dir::Next }),
                    KeyCode::Enter => {
                        self.close_list();
                        Some(Msg::OutlinePick)
                    }
                    KeyCode::Esc => {
                        self.close_list();
                        Some(Msg::OutlineClose)
                    }
                    KeyCode::Char('c') if ctrl => {
                        self.close_list();
                        Some(Msg::OutlineClose)
                    }
                    KeyCode::Up | KeyCode::BackTab => by(-1),
                    KeyCode::Char('p' | 'k') if ctrl => by(-1),
                    KeyCode::Down | KeyCode::Tab => by(1),
                    KeyCode::Char('n' | 'j') if ctrl => by(1),
                    KeyCode::PageUp => by(-OUTLINE_PAGE),
                    KeyCode::PageDown => by(OUTLINE_PAGE),
                    KeyCode::Home => by(i32::MIN / 2),
                    KeyCode::End => by(i32::MAX / 2),
                    _ => {
                        edit(text, &key)?;
                        Some(Msg::OutlineFilter { text: text.clone() })
                    }
                }
            }
            // The help scrolls with the usual keys; any other key closes it.
            Mode::Help => {
                let by = |by| Some(Msg::HelpScroll { by });
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Char('j') | KeyCode::Down => by(1),
                    KeyCode::Char('k') | KeyCode::Up => by(-1),
                    KeyCode::Char('d') if ctrl => by(HELP_PAGE),
                    KeyCode::Char('u') if ctrl => by(-HELP_PAGE),
                    KeyCode::Char(' ') | KeyCode::PageDown => by(HELP_PAGE),
                    KeyCode::Char('b') | KeyCode::PageUp => by(-HELP_PAGE),
                    KeyCode::Char('g') | KeyCode::Home => by(i32::MIN / 2),
                    KeyCode::Char('G') | KeyCode::End => by(i32::MAX / 2),
                    _ => {
                        self.mode = Mode::Normal;
                        Some(Msg::HelpClose)
                    }
                }
            }
            // The box waits for an answer; other keys do nothing.
            Mode::Confirm => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let msg = match key.code {
                    KeyCode::Char('c') if ctrl => Msg::SaveCancel,
                    KeyCode::Enter | KeyCode::Char('y') => Msg::SaveConfirm,
                    KeyCode::Esc | KeyCode::Char('n' | 'q') => Msg::SaveCancel,
                    _ => return None,
                };
                self.mode = Mode::Normal;
                Some(msg)
            }
            Mode::Settings => self.settings(&key),
        }
    }

    /// The settings popup's keys: `↑`/`↓` move, `Space`/`Enter` change, `←`/`→` choose, digits
    /// type a width; `a` keeps, `s` saves, `Esc` undoes. Its own keys close it as `a` does.
    fn settings(&mut self, event: &KeyEvent) -> Option<Msg> {
        let key = Key::of(event);
        let by = |by| Some(Msg::SettingsMove { by });
        let step = |dir| Some(Msg::SettingsStep { dir });
        let close = |keymap: &mut Self, msg| {
            keymap.mode = Mode::Normal;
            Some(msg)
        };
        if key.is_ctrl_c() {
            return close(self, Msg::SettingsCancel);
        }
        if self.bindings.find(&[key]) == Some(Action::Settings) {
            return close(self, Msg::SettingsKeep);
        }
        if !key.mods.difference(KeyModifiers::SHIFT).is_empty() {
            return None;
        }
        match key.code {
            KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k') => by(-1),
            KeyCode::Down | KeyCode::Tab | KeyCode::Char('j') => by(1),
            KeyCode::PageUp | KeyCode::Home => by(i32::MIN / 2),
            KeyCode::PageDown | KeyCode::End => by(i32::MAX / 2),
            KeyCode::Char(' ') | KeyCode::Enter => Some(Msg::SettingsFlip),
            KeyCode::Left | KeyCode::Char('h') => step(Dir::Prev),
            KeyCode::Right | KeyCode::Char('l') => step(Dir::Next),
            KeyCode::Char(c) if c.is_ascii_digit() => Some(Msg::SettingsDigit {
                digit: c as u8 - b'0',
            }),
            KeyCode::Backspace => Some(Msg::SettingsErase),
            KeyCode::Char('a') => close(self, Msg::SettingsKeep),
            KeyCode::Char('s') => close(self, Msg::SettingsSave),
            KeyCode::Esc | KeyCode::Char('q') => close(self, Msg::SettingsCancel),
            _ => None,
        }
    }

    /// A list closes: back to the settings popup when it opened over it.
    fn close_list(&mut self) {
        self.mode = if mem::take(&mut self.over_settings) {
            Mode::Settings
        } else {
            Mode::Normal
        };
    }

    /// The main loop's answer to a change in the settings popup: whether it opened the theme
    /// list, which then takes the keys until it closes.
    pub fn theme_list(&mut self, open: bool) {
        if open && self.mode == Mode::Settings {
            self.mode = Mode::Themes(String::new());
            self.over_settings = true;
        }
    }

    /// The wheel scrolls the document (under the search prompt too), moves through a list or the
    /// settings, or scrolls the help; sideways, it scrolls code. A left button released on the
    /// cell it went down on clicks there in normal mode and in the settings; a drag isn't a click.
    /// The save box takes no mouse.
    ///
    /// In normal mode, the left button selects text too (ADR-38): each press is a
    /// [`Msg::Press`], counted 2 and 3 when it comes within `MULTI_CLICK` of the one before on
    /// the same cell; a drag from it is [`Msg::Drag`], and letting go after a drag or a double
    /// or triple press is [`Msg::Release`] rather than a click. `now` is when the event came.
    pub fn mouse(&mut self, event: MouseEvent, now: Instant) -> Option<Msg> {
        let at = (event.column, event.row);
        let wheel = |by: i32| match self.mode {
            Mode::Normal | Mode::Search(_) => Some(Msg::Scroll {
                by: ScrollBy::Rows(by * WHEEL_ROWS),
            }),
            Mode::Outline(_) | Mode::Sort(_) | Mode::Themes(_) => Some(Msg::OutlineMove { by }),
            Mode::Help => Some(Msg::HelpScroll {
                by: by * WHEEL_ROWS,
            }),
            Mode::Settings => Some(Msg::SettingsMove { by }),
            Mode::Confirm => None,
        };
        let sideways = |by: i16| {
            matches!(self.mode, Mode::Normal | Mode::Search(_)).then_some(Msg::ScrollCodeX { by })
        };
        let msg = match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.pressed = Some(at);
                self.dragged = false;
                self.selecting = self.mode == Mode::Normal;
                if !self.selecting {
                    self.last_press = None;
                    return None;
                }
                let count = match self.last_press {
                    Some((cell, then, n))
                        if cell == at && now.saturating_duration_since(then) <= MULTI_CLICK =>
                    {
                        n % 3 + 1
                    }
                    _ => 1,
                };
                self.last_press = Some((at, now, count));
                Some(Msg::Press {
                    col: at.0,
                    row: at.1,
                    count,
                })
            }
            MouseEventKind::Drag(MouseButton::Left) if self.selecting => {
                self.pressed = None;
                self.dragged = true;
                self.last_press = None;
                // the prompt or a popup may have opened meanwhile
                (self.mode == Mode::Normal).then_some(Msg::Drag {
                    col: at.0,
                    row: at.1,
                })
            }
            MouseEventKind::Drag(_) => {
                self.pressed = None;
                return None;
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let selecting = mem::take(&mut self.selecting);
                let dragged = mem::take(&mut self.dragged);
                let count = self.last_press.map_or(1, |(_, _, n)| n);
                let clicked = self.pressed.take() == Some(at);
                let (col, row) = at;
                if selecting && (dragged || count > 1) {
                    Some(Msg::Release)
                } else {
                    match (clicked, &self.mode) {
                        (true, Mode::Normal) => Some(Msg::Click { col, row }),
                        (true, Mode::Settings) => Some(Msg::SettingsClick { col, row }),
                        _ => None,
                    }
                }
            }
            MouseEventKind::ScrollDown => wheel(1),
            MouseEventKind::ScrollUp => wheel(-1),
            MouseEventKind::ScrollRight => sideways(CODE_X_STEP),
            MouseEventKind::ScrollLeft => sideways(-CODE_X_STEP),
            _ => return None,
        };
        // as a key that completes no binding would, it drops a pending `]` or `[`
        if msg.is_some() {
            self.pending = None;
        }
        msg
    }

    /// The confirm box `S` asks for didn't open, since saving would change nothing (or
    /// couldn't): back to the usual keys.
    pub fn no_confirm(&mut self) {
        if self.mode == Mode::Confirm {
            self.mode = Mode::Normal;
        }
    }

    fn normal(&mut self, event: &KeyEvent) -> Option<Msg> {
        let key = Key::of(event);
        let pending = self.pending.take();
        if key.is_ctrl_c() {
            return Some(Msg::Quit);
        }
        // a key that doesn't complete a two-key binding counts on its own
        let action = match pending.and_then(|p| self.bindings.find(&[p, key])) {
            Some(action) => action,
            None if self.bindings.is_prefix(key) => {
                self.pending = Some(key);
                return None;
            }
            None => self.bindings.find(&[key])?,
        };
        match action {
            Action::Outline | Action::Links => {
                self.mode = Mode::Outline(String::new());
            }
            Action::Search => self.mode = Mode::Search(String::new()),
            Action::Help => self.mode = Mode::Help,
            Action::Sort => self.mode = Mode::Sort(String::new()),
            Action::Theme => self.mode = Mode::Themes(String::new()),
            Action::SaveConfig => self.mode = Mode::Confirm,
            Action::Settings => self.mode = Mode::Settings,
            // `Esc` clears a selection before it quits
            Action::Quit if key.code == KeyCode::Esc => return Some(Msg::Cancel),
            _ => {}
        }
        Some(action.msg())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(km: &mut Keymap, code: KeyCode) -> Option<Msg> {
        km.map(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn typed(km: &mut Keymap, text: &str) -> Vec<Msg> {
        text.chars()
            .filter_map(|c| press(km, KeyCode::Char(c)))
            .collect()
    }

    #[test]
    fn two_key_heading_jumps() {
        let mut km = Keymap::default();
        assert_eq!(press(&mut km, KeyCode::Char(']')), None);
        assert_eq!(
            press(&mut km, KeyCode::Char(']')),
            Some(Msg::JumpHeading { dir: Dir::Next })
        );
        assert_eq!(press(&mut km, KeyCode::Char('[')), None);
        assert_eq!(
            press(&mut km, KeyCode::Char('j')),
            Some(Msg::Scroll {
                by: ScrollBy::Rows(1)
            })
        );
        // the pending `[` was cancelled by `j`
        assert_eq!(press(&mut km, KeyCode::Char('[')), None);
        assert_eq!(
            press(&mut km, KeyCode::Char('[')),
            Some(Msg::JumpHeading { dir: Dir::Prev })
        );
    }

    #[test]
    fn change_jumps_and_reload() {
        let mut km = Keymap::default();
        assert_eq!(press(&mut km, KeyCode::Char(']')), None);
        assert_eq!(
            press(&mut km, KeyCode::Char('c')),
            Some(Msg::JumpChange { dir: Dir::Next })
        );
        press(&mut km, KeyCode::Char('['));
        assert_eq!(
            press(&mut km, KeyCode::Char('c')),
            Some(Msg::JumpChange { dir: Dir::Prev })
        );
        assert_eq!(
            press(&mut km, KeyCode::Char('c')),
            None,
            "c alone does nothing"
        );
        assert_eq!(press(&mut km, KeyCode::Char('r')), Some(Msg::Reload));
    }

    #[test]
    fn ctrl_keys() {
        let mut km = Keymap::default();
        let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        assert_eq!(km.map(ctrl('c')), Some(Msg::Quit));
        assert_eq!(
            km.map(ctrl('d')),
            Some(Msg::Scroll {
                by: ScrollBy::HalfPage(Dir::Next)
            })
        );
        assert_eq!(km.map(ctrl('q')), None);
        assert_eq!(km.map(ctrl('o')), Some(Msg::Back));
    }

    #[test]
    fn shift_is_in_the_character() {
        let mut km = Keymap::default();
        let shifted = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT);
        assert_eq!(
            km.map(shifted('G')),
            Some(Msg::Scroll {
                by: ScrollBy::Bottom
            })
        );
        let backtab = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(km.map(backtab), Some(Msg::FocusLink { dir: Dir::Prev }));
        assert_eq!(km.map(shifted('T')), Some(Msg::OpenThemes));
    }

    #[test]
    fn code_keys() {
        let mut km = Keymap::default();
        assert_eq!(
            press(&mut km, KeyCode::Char('w')),
            Some(Msg::ToggleCodeWrap)
        );
        assert_eq!(
            press(&mut km, KeyCode::Left),
            Some(Msg::ScrollCodeX { by: -CODE_X_STEP })
        );
        assert_eq!(
            press(&mut km, KeyCode::Right),
            Some(Msg::ScrollCodeX { by: CODE_X_STEP })
        );
        let mut hash = |mods| km.map(KeyEvent::new(KeyCode::Char('#'), mods));
        assert_eq!(hash(KeyModifiers::NONE), Some(Msg::ToggleLineNumbers));
        assert_eq!(hash(KeyModifiers::SHIFT), Some(Msg::ToggleLineNumbers));
    }

    #[test]
    fn link_keys() {
        let mut km = Keymap::default();
        assert_eq!(
            press(&mut km, KeyCode::BackTab),
            Some(Msg::FocusLink { dir: Dir::Prev })
        );
        assert_eq!(press(&mut km, KeyCode::Enter), Some(Msg::Follow));
        assert_eq!(press(&mut km, KeyCode::Backspace), Some(Msg::Back));
        assert_eq!(press(&mut km, KeyCode::Char('L')), Some(Msg::Forward));
    }

    #[test]
    fn p7_keys() {
        let mut km = Keymap::default();
        assert_eq!(press(&mut km, KeyCode::Char('e')), Some(Msg::Edit));
        assert_eq!(press(&mut km, KeyCode::Char('y')), Some(Msg::Yank));
        assert_eq!(press(&mut km, KeyCode::Char('?')), Some(Msg::Help));
        // the help scrolls, and closes on anything else
        assert_eq!(
            press(&mut km, KeyCode::Char('j')),
            Some(Msg::HelpScroll { by: 1 })
        );
        assert_eq!(press(&mut km, KeyCode::Char('q')), Some(Msg::HelpClose));
        assert_eq!(press(&mut km, KeyCode::Char('q')), Some(Msg::Quit));
    }

    fn mouse(km: &mut Keymap, kind: MouseEventKind, column: u16, row: u16) -> Option<Msg> {
        // a second apart: never a double click
        let then = km.last_press.map(|(_, t, _)| t + Duration::from_secs(1));
        mouse_at(km, kind, column, row, then.unwrap_or_else(Instant::now))
    }

    fn mouse_at(
        km: &mut Keymap,
        kind: MouseEventKind,
        column: u16,
        row: u16,
        now: Instant,
    ) -> Option<Msg> {
        let event = MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        km.mouse(event, now)
    }

    #[test]
    fn the_wheel_scrolls_what_the_mode_shows() {
        let mut km = Keymap::default();
        let down = |km: &mut Keymap| mouse(km, MouseEventKind::ScrollDown, 5, 5);
        let up = |km: &mut Keymap| mouse(km, MouseEventKind::ScrollUp, 5, 5);
        let rows = |n| {
            Some(Msg::Scroll {
                by: ScrollBy::Rows(n),
            })
        };
        assert_eq!(down(&mut km), rows(3));
        assert_eq!(up(&mut km), rows(-3));
        assert_eq!(
            mouse(&mut km, MouseEventKind::ScrollRight, 5, 5),
            Some(Msg::ScrollCodeX { by: CODE_X_STEP })
        );
        // a pending `]` is dropped, as another key would drop it
        press(&mut km, KeyCode::Char(']'));
        down(&mut km);
        assert_eq!(press(&mut km, KeyCode::Char(']')), None, "pending again");
        press(&mut km, KeyCode::Esc);

        // the search prompt stays open over the document's scroll
        press(&mut km, KeyCode::Char('/'));
        assert_eq!(down(&mut km), rows(3));
        assert_eq!(
            typed(&mut km, "a"),
            [Msg::SearchInput {
                pattern: "a".into()
            }]
        );
        press(&mut km, KeyCode::Esc);

        press(&mut km, KeyCode::Char('o'));
        assert_eq!(down(&mut km), Some(Msg::OutlineMove { by: 1 }));
        assert_eq!(up(&mut km), Some(Msg::OutlineMove { by: -1 }));
        assert_eq!(mouse(&mut km, MouseEventKind::ScrollLeft, 5, 5), None);
        press(&mut km, KeyCode::Esc);

        press(&mut km, KeyCode::Char('?'));
        assert_eq!(down(&mut km), Some(Msg::HelpScroll { by: 3 }));
        press(&mut km, KeyCode::Char('q'));

        press(&mut km, KeyCode::Char('S'));
        assert_eq!(down(&mut km), None, "the save box waits for an answer");
        assert_eq!(press(&mut km, KeyCode::Char('n')), Some(Msg::SaveCancel));
    }

    #[test]
    fn a_click_is_a_press_and_a_release_on_one_cell() {
        let mut km = Keymap::default();
        let left = MouseButton::Left;
        let pressed = |col, row| Some(Msg::Press { col, row, count: 1 });
        assert_eq!(
            mouse(&mut km, MouseEventKind::Down(left), 7, 2),
            pressed(7, 2)
        );
        assert_eq!(
            mouse(&mut km, MouseEventKind::Up(left), 7, 2),
            Some(Msg::Click { col: 7, row: 2 })
        );
        // released elsewhere, dragged, released with no press, or another button: no click
        mouse(&mut km, MouseEventKind::Down(left), 7, 2);
        assert_eq!(mouse(&mut km, MouseEventKind::Up(left), 8, 2), None);
        mouse(&mut km, MouseEventKind::Down(left), 7, 2);
        mouse(&mut km, MouseEventKind::Drag(left), 8, 2);
        mouse(&mut km, MouseEventKind::Drag(left), 7, 2);
        assert_eq!(
            mouse(&mut km, MouseEventKind::Up(left), 7, 2),
            Some(Msg::Release),
            "a drag selects"
        );
        assert_eq!(mouse(&mut km, MouseEventKind::Up(left), 7, 2), None);
        let right = MouseButton::Right;
        assert_eq!(mouse(&mut km, MouseEventKind::Down(right), 7, 2), None);
        assert_eq!(mouse(&mut km, MouseEventKind::Up(right), 7, 2), None);
        // only in normal mode: a popup takes no clicks, and the left button selects nothing
        press(&mut km, KeyCode::Char('o'));
        assert_eq!(mouse(&mut km, MouseEventKind::Down(left), 7, 2), None);
        assert_eq!(mouse(&mut km, MouseEventKind::Drag(left), 8, 2), None);
        assert_eq!(mouse(&mut km, MouseEventKind::Up(left), 7, 2), None);
        // and `M` toggles the capture
        press(&mut km, KeyCode::Esc);
        assert_eq!(press(&mut km, KeyCode::Char('M')), Some(Msg::ToggleMouse));
    }

    #[test]
    fn presses_in_quick_succession_on_one_cell_count_up() {
        let mut km = Keymap::default();
        let left = MouseButton::Left;
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let down = |km: &mut Keymap, col, t| mouse_at(km, MouseEventKind::Down(left), col, 3, t);
        let up = |km: &mut Keymap, col, t| mouse_at(km, MouseEventKind::Up(left), col, 3, t);
        let count = |msg: Option<Msg>| match msg {
            Some(Msg::Press { count, .. }) => count,
            other => panic!("not a press: {other:?}"),
        };
        assert_eq!(count(down(&mut km, 4, ms(0))), 1);
        assert_eq!(up(&mut km, 4, ms(50)), Some(Msg::Click { col: 4, row: 3 }));
        assert_eq!(count(down(&mut km, 4, ms(200))), 2);
        assert_eq!(up(&mut km, 4, ms(250)), Some(Msg::Release), "not a click");
        assert_eq!(count(down(&mut km, 4, ms(400))), 3);
        up(&mut km, 4, ms(450));
        assert_eq!(count(down(&mut km, 4, ms(600))), 1, "a fourth starts over");
        up(&mut km, 4, ms(650));
        // too late, or on another cell
        assert_eq!(count(down(&mut km, 4, ms(1200))), 1);
        up(&mut km, 4, ms(1250));
        assert_eq!(count(down(&mut km, 5, ms(1300))), 1);
        // a drag in between starts over
        mouse_at(&mut km, MouseEventKind::Drag(left), 6, 3, ms(1350));
        up(&mut km, 6, ms(1400));
        assert_eq!(count(down(&mut km, 6, ms(1450))), 1);
    }

    #[test]
    fn esc_cancels_where_q_quits() {
        let mut km = Keymap::default();
        assert_eq!(press(&mut km, KeyCode::Esc), Some(Msg::Cancel));
        assert_eq!(press(&mut km, KeyCode::Char('q')), Some(Msg::Quit));
    }

    #[test]
    fn the_save_box_takes_an_answer() {
        let mut km = Keymap::default();
        for (key, answer) in [
            (KeyCode::Enter, Msg::SaveConfirm),
            (KeyCode::Char('y'), Msg::SaveConfirm),
            (KeyCode::Esc, Msg::SaveCancel),
            (KeyCode::Char('n'), Msg::SaveCancel),
            (KeyCode::Char('q'), Msg::SaveCancel),
        ] {
            assert_eq!(press(&mut km, KeyCode::Char('S')), Some(Msg::SaveConfig));
            // other keys wait for the answer
            assert_eq!(press(&mut km, KeyCode::Char('j')), None);
            assert_eq!(press(&mut km, key), Some(answer));
        }
        assert_eq!(
            km.map(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(Msg::Quit)
        );
        // no box opened: the next key is a usual one
        assert_eq!(press(&mut km, KeyCode::Char('S')), Some(Msg::SaveConfig));
        km.no_confirm();
        assert_eq!(press(&mut km, KeyCode::Char('q')), Some(Msg::Quit));
    }

    #[test]
    fn the_settings_take_their_keys() {
        let mut km = Keymap::default();
        assert_eq!(press(&mut km, KeyCode::F(2)), Some(Msg::OpenSettings));
        let mv = |by| Some(Msg::SettingsMove { by });
        assert_eq!(press(&mut km, KeyCode::Down), mv(1));
        assert_eq!(press(&mut km, KeyCode::Char('k')), mv(-1));
        assert_eq!(press(&mut km, KeyCode::Tab), mv(1));
        assert_eq!(press(&mut km, KeyCode::Char(' ')), Some(Msg::SettingsFlip));
        assert_eq!(press(&mut km, KeyCode::Enter), Some(Msg::SettingsFlip));
        assert_eq!(
            press(&mut km, KeyCode::Left),
            Some(Msg::SettingsStep { dir: Dir::Prev })
        );
        assert_eq!(
            typed(&mut km, "l90x"),
            [
                Msg::SettingsStep { dir: Dir::Next },
                Msg::SettingsDigit { digit: 9 },
                Msg::SettingsDigit { digit: 0 },
            ]
        );
        assert_eq!(press(&mut km, KeyCode::Backspace), Some(Msg::SettingsErase));
        // the document's keys mean nothing here
        assert_eq!(press(&mut km, KeyCode::Char('t')), None);
        assert_eq!(press(&mut km, KeyCode::Char('a')), Some(Msg::SettingsKeep));
        assert_eq!(
            press(&mut km, KeyCode::Char('j')),
            Some(Msg::Scroll {
                by: ScrollBy::Rows(1)
            })
        );

        // the ways out, and `,` opens it too
        for (key, out) in [
            (KeyCode::Char('s'), Msg::SettingsSave),
            (KeyCode::Esc, Msg::SettingsCancel),
            (KeyCode::Char('q'), Msg::SettingsCancel),
            (KeyCode::F(2), Msg::SettingsKeep),
            (KeyCode::Char(','), Msg::SettingsKeep),
        ] {
            assert_eq!(press(&mut km, KeyCode::Char(',')), Some(Msg::OpenSettings));
            assert_eq!(press(&mut km, key), Some(out));
            assert_eq!(press(&mut km, KeyCode::Char('q')), Some(Msg::Quit));
        }
        press(&mut km, KeyCode::F(2));
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(km.map(ctrl_c), Some(Msg::SettingsCancel));
    }

    #[test]
    fn the_theme_list_hands_the_keys_back_to_the_settings() {
        let mut km = Keymap::default();
        press(&mut km, KeyCode::F(2));
        // a change that opened no list: still the settings
        press(&mut km, KeyCode::Enter);
        km.theme_list(false);
        assert_eq!(
            press(&mut km, KeyCode::Down),
            Some(Msg::SettingsMove { by: 1 })
        );
        // one that did: the list's keys, then the settings' again, picked or not
        for out in [KeyCode::Enter, KeyCode::Esc] {
            press(&mut km, KeyCode::Enter);
            km.theme_list(true);
            assert_eq!(
                typed(&mut km, "s"),
                [Msg::OutlineFilter { text: "s".into() }]
            );
            assert_eq!(
                press(&mut km, KeyCode::Right),
                Some(Msg::ThemesTone { dir: Dir::Next })
            );
            press(&mut km, out);
            assert_eq!(press(&mut km, KeyCode::Char('s')), Some(Msg::SettingsSave));
            press(&mut km, KeyCode::F(2));
        }
        // `T` from the document goes back to it
        press(&mut km, KeyCode::Esc);
        press(&mut km, KeyCode::Char('T'));
        press(&mut km, KeyCode::Esc);
        assert_eq!(press(&mut km, KeyCode::Char('q')), Some(Msg::Quit));
        // and an answer outside the settings changes nothing
        km.theme_list(true);
        assert_eq!(press(&mut km, KeyCode::Char('q')), Some(Msg::Quit));
    }

    #[test]
    fn the_settings_take_the_wheel_and_clicks() {
        let mut km = Keymap::default();
        press(&mut km, KeyCode::Char(','));
        assert_eq!(
            mouse(&mut km, MouseEventKind::ScrollDown, 5, 5),
            Some(Msg::SettingsMove { by: 1 })
        );
        assert_eq!(mouse(&mut km, MouseEventKind::ScrollRight, 5, 5), None);
        let left = MouseButton::Left;
        mouse(&mut km, MouseEventKind::Down(left), 30, 4);
        assert_eq!(
            mouse(&mut km, MouseEventKind::Up(left), 30, 4),
            Some(Msg::SettingsClick { col: 30, row: 4 })
        );
    }

    #[test]
    fn the_search_prompt_takes_every_key() {
        let mut km = Keymap::default();
        assert_eq!(press(&mut km, KeyCode::Char('/')), Some(Msg::SearchOpen));
        let input = |p: &str| Msg::SearchInput { pattern: p.into() };
        assert_eq!(
            typed(&mut km, "q n"),
            [input("q"), input("q "), input("q n")]
        );
        let ctrl_w = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
        assert_eq!(km.map(ctrl_w), Some(input("q ")));
        assert_eq!(press(&mut km, KeyCode::Backspace), Some(input("q")));
        assert_eq!(press(&mut km, KeyCode::Enter), Some(Msg::SearchSubmit));
        // back to normal
        assert_eq!(
            press(&mut km, KeyCode::Char('n')),
            Some(Msg::SearchNext { dir: Dir::Next })
        );

        press(&mut km, KeyCode::Char('/'));
        assert_eq!(press(&mut km, KeyCode::Backspace), Some(Msg::SearchCancel));
        assert_eq!(press(&mut km, KeyCode::Char('q')), Some(Msg::Quit));
    }

    #[test]
    fn the_outline_filters_and_moves() {
        let mut km = Keymap::default();
        assert_eq!(press(&mut km, KeyCode::Char('o')), Some(Msg::OpenOutline));
        assert_eq!(
            typed(&mut km, "jk"),
            [
                Msg::OutlineFilter { text: "j".into() },
                Msg::OutlineFilter { text: "jk".into() }
            ]
        );
        assert_eq!(
            press(&mut km, KeyCode::Down),
            Some(Msg::OutlineMove { by: 1 })
        );
        assert_eq!(press(&mut km, KeyCode::Esc), Some(Msg::OutlineClose));
        assert_eq!(press(&mut km, KeyCode::Esc), Some(Msg::Cancel));

        // the link list shares the outline's keys
        assert_eq!(press(&mut km, KeyCode::Char('l')), Some(Msg::OpenLinks));
        assert_eq!(
            typed(&mut km, "q"),
            [Msg::OutlineFilter { text: "q".into() }]
        );
        assert_eq!(press(&mut km, KeyCode::Enter), Some(Msg::OutlinePick));

        // and so does the theme list, whose `←`/`→` list the dark or light themes
        assert_eq!(press(&mut km, KeyCode::Char('T')), Some(Msg::OpenThemes));
        assert_eq!(
            press(&mut km, KeyCode::Down),
            Some(Msg::OutlineMove { by: 1 })
        );
        assert_eq!(
            press(&mut km, KeyCode::Right),
            Some(Msg::ThemesTone { dir: Dir::Next })
        );
        assert_eq!(
            press(&mut km, KeyCode::Left),
            Some(Msg::ThemesTone { dir: Dir::Prev })
        );
        assert_eq!(press(&mut km, KeyCode::Esc), Some(Msg::OutlineClose));

        // the outline has no use for them
        assert_eq!(press(&mut km, KeyCode::Char('o')), Some(Msg::OpenOutline));
        assert_eq!(press(&mut km, KeyCode::Right), None);
        assert_eq!(press(&mut km, KeyCode::Esc), Some(Msg::OutlineClose));
    }

    #[test]
    fn key_specs() {
        let key = |s| Key::parse(s).map(|k| k.to_string());
        assert_eq!(key("ctrl-d").as_deref(), Some("C-d"));
        assert_eq!(key("C-D").as_deref(), Some("C-d"));
        assert_eq!(key("alt-x").as_deref(), Some("M-x"));
        assert_eq!(key("shift-tab").as_deref(), Some("S-Tab"));
        assert_eq!(key("S-up").as_deref(), Some("S-↑"));
        assert_eq!(key("PageDown").as_deref(), Some("PgDn"));
        assert_eq!(key("f5").as_deref(), Some("F5"));
        assert_eq!(key("-").as_deref(), Some("-"));
        assert_eq!(key("G").as_deref(), Some("G"));
        assert_eq!(key("f13"), None);
        assert_eq!(key("bogus"), None);
        let seq = |s| KeySeq::parse(s).map(|k| k.to_string());
        assert_eq!(seq("]]").as_deref(), Some("]]"));
        assert_eq!(seq("g g").as_deref(), Some("gg"));
        assert_eq!(seq("ctrl-w j").as_deref(), Some("C-w j"));
        assert_eq!(seq("a b c"), None);
        assert_eq!(seq("abc"), None);
    }

    #[test]
    fn every_default_key_parses() {
        let bindings = Bindings::default();
        for &action in Action::ALL {
            assert_eq!(
                bindings.keys(action).len(),
                action.default_keys().len(),
                "{}",
                action.name()
            );
        }
        assert!(Bindings::default().rebind(Vec::new()).is_empty());
    }

    #[test]
    fn rebinding_takes_keys_from_the_defaults() {
        let mut bindings = Bindings::default();
        let seqs = |specs: &[&str]| specs.iter().map(|s| KeySeq::parse(s).unwrap()).collect();
        let warnings = bindings.rebind(vec![
            (Action::NextLink, seqs(&["j"])),
            (Action::Top, seqs(&["g g"])),
        ]);
        assert!(warnings.is_empty(), "top was rebound, so g alone is gone");
        let mut km = Keymap::new(bindings);
        assert_eq!(
            press(&mut km, KeyCode::Char('j')),
            Some(Msg::FocusLink { dir: Dir::Next })
        );
        assert_eq!(
            press(&mut km, KeyCode::Down),
            Some(Msg::Scroll {
                by: ScrollBy::Rows(1)
            })
        );
        assert_eq!(press(&mut km, KeyCode::Char('g')), None);
        assert_eq!(
            press(&mut km, KeyCode::Char('g')),
            Some(Msg::Scroll { by: ScrollBy::Top })
        );

        let mut both = Bindings::default();
        let warnings = both.rebind(vec![
            (Action::Quit, seqs(&["x"])),
            (Action::Yank, seqs(&["x", "q"])),
        ]);
        assert_eq!(warnings, ["keys: x is bound to both quit and yank"]);

        let mut alone = Bindings::default();
        let warnings = alone.rebind(vec![(Action::Theme, seqs(&["]"]))]);
        assert_eq!(
            warnings,
            ["keys: ] (theme) also starts a two-key binding, so it can't work alone"]
        );
    }

    #[test]
    fn sort_lists_columns_and_trades_tables_with_left_and_right() {
        let mut km = Keymap::default();
        assert_eq!(
            typed(&mut km, "sj"),
            [Msg::SortOpen, Msg::OutlineFilter { text: "j".into() }]
        );
        assert_eq!(
            press(&mut km, KeyCode::Right),
            Some(Msg::SortTable { dir: Dir::Next })
        );
        // the filter started over with the other table
        assert_eq!(
            typed(&mut km, "x"),
            [Msg::OutlineFilter { text: "x".into() }]
        );
        assert_eq!(
            press(&mut km, KeyCode::Left),
            Some(Msg::SortTable { dir: Dir::Prev })
        );
        assert_eq!(
            press(&mut km, KeyCode::Tab),
            Some(Msg::OutlineMove { by: 1 })
        );
        assert_eq!(press(&mut km, KeyCode::Enter), Some(Msg::OutlinePick));
        typed(&mut km, "s");
        assert_eq!(press(&mut km, KeyCode::Esc), Some(Msg::OutlineClose));
        assert_eq!(press(&mut km, KeyCode::Esc), Some(Msg::Cancel));
        // the other lists leave the arrows alone
        typed(&mut km, "o");
        assert_eq!(press(&mut km, KeyCode::Right), None);
    }

    #[test]
    fn help_groups_follow_the_table() {
        let help = Bindings::default().help();
        assert_eq!(help[0].0, "Moving");
        assert_eq!(help[0].1[0], ("j ↓ C-e".to_owned(), "scroll down a row"));
        let groups: Vec<_> = help.iter().map(|(g, _)| *g).collect();
        assert_eq!(
            groups,
            [
                "Moving",
                "Links and history",
                "Search and outline",
                "View",
                "Other"
            ]
        );
    }
}
