//! Keys → semantic messages (plan §6). Pure: the only state is a pending prefix key for the
//! two-key bindings like `]]` and `]c`, and the mode with what has been typed into the search
//! prompt or the outline filter. Recordings log the messages, not the keys, so they survive keymap
//! changes.
//!
//! The keymap runs on the input thread, so it tracks the mode itself: `/` opens the prompt here
//! and now, and the keys typed right after it can't race the main loop. The one mode it can't
//! know is the confirm box `S` opens, which opens only when saving would change the file: the
//! input thread waits for the main loop's answer, and [`Keymap::no_confirm`] takes the box back.
//!
//! Normal-mode keys come from [`Bindings`]: one table of actions with their default keys, which
//! the config file's `[keys]` section overrides, and which the help overlay lists.

use std::fmt;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::{CODE_X_STEP, Dir, Msg, ScrollBy};

/// Rows `PgUp` / `PgDn` move the outline selection.
const OUTLINE_PAGE: i32 = 10;
/// Rows `PgUp` / `PgDn` scroll the help.
const HELP_PAGE: i32 = 10;

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
        Yank => "yank", "copy the link or the code block", ["y"],
        SaveConfig => "save_config", "save the settings to the config file", ["S"],
        Help => "help", "this help", ["?"],
        Quit => "quit", "quit (so does C-c)", ["q", "esc"],
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
            Action::SaveConfig => Msg::SaveConfig,
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
}

#[derive(Debug, Default)]
pub struct Keymap {
    bindings: Bindings,
    pending: Option<Key>,
    mode: Mode,
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
                        self.mode = Mode::Normal;
                        Some(Msg::OutlinePick)
                    }
                    KeyCode::Esc => {
                        self.mode = Mode::Normal;
                        Some(Msg::OutlineClose)
                    }
                    KeyCode::Char('c') if ctrl => {
                        self.mode = Mode::Normal;
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
        }
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
        assert_eq!(press(&mut km, KeyCode::Esc), Some(Msg::Quit));

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
        assert_eq!(press(&mut km, KeyCode::Esc), Some(Msg::Quit));
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
