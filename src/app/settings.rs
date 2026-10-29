//! The settings popup (`F2`, `,`): the options `S` saves, each with its value, changed in place
//! and shown behind the popup at once, as `T` previews themes. `a` keeps the changes for the
//! session, `s` saves them to the config file too, and `Esc` puts back the settings it opened
//! with. A row whose value isn't the config file's is marked, so the marks are what `s` saves.
//!
//! A change runs the setter the option's own key runs (`B`, `t`, `w`, `#`, `M`), so the popup
//! adds no second way to change one; the popup's own messages are what a recording keeps. Images,
//! their protocol and live reload are read at start: changed here, they are only saved, for the
//! next start.

use super::{Action, Cmd, Dir, Listing, State};
use crate::config::{LiveReload, Saved, Settings};
use crate::filetype::FileType;
use crate::images::{ImageMode, Proto};
use crate::layout::{LineNumbers, TableMode};
use crate::select::CopyOn;

/// A row of the popup: one option.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Theme,
    ThemeBg,
    Width,
    Tables,
    CodeWrap,
    LineNumbers,
    Images,
    Protocol,
    Mouse,
    CopyOn,
    LiveReload,
    KeepScreen,
}

impl Row {
    pub const ALL: [Row; 12] = [
        Row::Theme,
        Row::ThemeBg,
        Row::Width,
        Row::Tables,
        Row::CodeWrap,
        Row::LineNumbers,
        Row::Images,
        Row::Protocol,
        Row::Mouse,
        Row::CopyOn,
        Row::LiveReload,
        Row::KeepScreen,
    ];

    /// The group the row is listed under.
    pub fn group(self) -> &'static str {
        match self {
            Row::Theme | Row::ThemeBg | Row::Width => "Appearance",
            Row::Tables | Row::CodeWrap | Row::LineNumbers | Row::Images | Row::Protocol => {
                "Content"
            }
            Row::Mouse | Row::CopyOn | Row::LiveReload | Row::KeepScreen => "Behavior",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Row::Theme => "Theme",
            Row::ThemeBg => "Theme background",
            Row::Width => "Prose width",
            Row::Tables => "Tables",
            Row::CodeWrap => "Wrap code",
            Row::LineNumbers => "Line numbers",
            Row::Images => "Images",
            Row::Protocol => "Image protocol",
            Row::Mouse => "Mouse",
            Row::CopyOn => "Copy a selection",
            Row::LiveReload => "Live reload",
            Row::KeepScreen => "Keep screen on quit",
        }
    }

    /// Its option in the config file.
    pub fn option(self) -> &'static str {
        match self {
            Row::Theme => "theme",
            Row::ThemeBg => "theme_bg",
            Row::Width => "max_width",
            Row::Tables => "tables",
            Row::CodeWrap => "code_wrap",
            Row::LineNumbers => "line_numbers",
            Row::Images => "images",
            Row::Protocol => "image_protocol",
            Row::Mouse => "mouse",
            Row::CopyOn => "copy_on",
            Row::LiveReload => "live_reload",
            Row::KeepScreen => "keep_screen",
        }
    }

    /// The action whose key changes it outside the popup.
    pub fn action(self) -> Option<Action> {
        match self {
            Row::Theme => Some(Action::Theme),
            Row::ThemeBg => Some(Action::ThemeBg),
            Row::Tables => Some(Action::TableMode),
            Row::CodeWrap => Some(Action::CodeWrap),
            Row::LineNumbers => Some(Action::LineNumbers),
            Row::Mouse => Some(Action::Mouse),
            _ => None,
        }
    }

    /// Read only at start: a change counts from the next start, once saved.
    pub fn at_start(self) -> bool {
        matches!(self, Row::Images | Row::Protocol | Row::LiveReload)
    }
}

pub const TABLES: &[&str] = &["auto", "grid", "records"];
pub const LINE_NUMBERS: &[&str] = &["off", "code", "all"];
pub const PROTOCOLS: &[&str] = &["auto", "kitty", "sixel", "iterm2", "halfblocks"];
pub const COPY_ON: &[&str] = &["yank", "select"];

/// The narrowest prose width the popup sets: a typed number below it is a width on its way
/// (`1`, `10`, then `100`), and lays nothing out until it gets there.
pub const MIN_WIDTH: u16 = 20;
/// Digits the prose width takes: up to 999 columns.
const WIDTH_DIGITS: usize = 3;

/// What a row shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value<'a> {
    Switch(bool),
    /// Every choice, and the current one's index.
    Choice(&'static [&'static str], usize),
    /// The prose width as typed: empty for the terminal's.
    Width(&'a str),
    Theme(&'a str),
}

/// The settings popup, while it is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsPopup {
    /// The focused row, an index into [`Row::ALL`].
    pub row: usize,
    /// The prose width as typed: empty for the terminal's.
    pub width: String,
    /// The config file's values, once read, for the marks.
    pub file: Option<Saved>,
    /// The settings it opened with, which `Esc` puts back, and the theme's name among them.
    opened: Settings,
    theme: String,
    /// Images as they come back on: the protocol they had, or `auto`.
    images_on: ImageMode,
}

impl SettingsPopup {
    pub fn focused(&self) -> Row {
        Row::ALL[self.row.min(Row::ALL.len() - 1)]
    }
}

/// The options read only at start, as the session sets them for the next one: they are saved,
/// while the session runs on [`Config`](super::Config)'s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NextStart {
    pub images: ImageMode,
    pub live_reload: LiveReload,
}

/// What the config file is read for: the save box (`S`), the settings popup's marks, or saving
/// what the popup set (`s`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigUse {
    SaveBox,
    Marks,
    Save,
}

/// A change to the focused row: `Space`/`Enter` flips it, `←`/`→` step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Flip,
    Step(Dir),
}

impl Change {
    fn switch(self, on: bool) -> bool {
        match self {
            Change::Flip => !on,
            Change::Step(dir) => dir == Dir::Next,
        }
    }

    /// The choice after `i` of `n`, wrapping.
    fn choice(self, i: usize, n: usize) -> usize {
        match self {
            Change::Flip | Change::Step(Dir::Next) => (i + 1) % n,
            Change::Step(Dir::Prev) => (i + n - 1) % n,
        }
    }
}

/// Where a click in the popup landed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// A row's label, or a value with nothing in particular under it.
    Row(usize),
    /// A row's switch, theme or width.
    Value(usize),
    /// One of a row's choices.
    Choice(usize, usize),
    Keep,
    Save,
    Cancel,
}

fn width_text(width: Option<u16>) -> String {
    width.map_or_else(String::new, |w| w.to_string())
}

impl State {
    /// What `row` shows.
    pub fn setting(&self, row: Row) -> Value<'_> {
        let index = |names: &[&str], name: &str| names.iter().position(|n| *n == name);
        match row {
            Row::Theme => Value::Theme(&self.theme.name),
            Row::ThemeBg => Value::Switch(self.theme_bg),
            Row::Width => Value::Width(self.settings.as_ref().map_or("", |p| &p.width)),
            Row::Tables => {
                Value::Choice(TABLES, index(TABLES, self.table_mode.name()).unwrap_or(0))
            }
            Row::CodeWrap => Value::Switch(self.code_wrap),
            Row::LineNumbers => Value::Choice(
                LINE_NUMBERS,
                index(LINE_NUMBERS, self.line_numbers.spelling()).unwrap_or(0),
            ),
            Row::Images => Value::Switch(self.next_start.images != ImageMode::Off),
            Row::Protocol => {
                let proto = match self.images_on() {
                    ImageMode::Force(proto) => proto.name(),
                    _ => "auto",
                };
                Value::Choice(PROTOCOLS, index(PROTOCOLS, proto).unwrap_or(0))
            }
            Row::Mouse => Value::Switch(self.mouse),
            Row::CopyOn => Value::Choice(
                COPY_ON,
                index(COPY_ON, self.config.copy_on.name()).unwrap_or(0),
            ),
            Row::LiveReload => Value::Choice(
                LiveReload::NAMES,
                index(LiveReload::NAMES, self.next_start.live_reload.name()).unwrap_or(0),
            ),
            Row::KeepScreen => Value::Switch(self.config.keep_screen),
        }
    }

    /// Whether `row` takes changes: the protocol only with images on.
    pub fn setting_enabled(&self, row: Row) -> bool {
        row != Row::Protocol || self.next_start.images != ImageMode::Off
    }

    /// Whether `row`'s value isn't the config file's, as far as it has been read.
    pub fn setting_changed(&self, row: Row) -> bool {
        let Some(file) = self.settings.as_ref().and_then(|p| p.file.as_ref()) else {
            return false;
        };
        file.changes(&self.active_settings())
            .iter()
            .any(|(key, ..)| *key == row.option())
    }

    /// Images as they are, or as they'll come back on.
    fn images_on(&self) -> ImageMode {
        match (self.next_start.images, &self.settings) {
            (ImageMode::Off, Some(popup)) => popup.images_on,
            (ImageMode::Off, None) => ImageMode::Auto,
            (mode, _) => mode,
        }
    }

    /// Open the popup on the first row, and read the config file for its marks.
    pub(super) fn open_settings(&mut self) -> Vec<Cmd> {
        let mut cmds = self.outline_close();
        (self.help, self.save, self.prompt) = (None, None, None);
        self.status = None;
        self.settings = Some(SettingsPopup {
            row: 0,
            width: width_text(self.config.max_width),
            file: None,
            opened: self.active_settings(),
            theme: self.theme.name.clone(),
            images_on: self.images_on(),
        });
        // what the read is for, file or not: a replay without one gets the recorded read
        self.config_for = Some(ConfigUse::Marks);
        if let Some(path) = &self.config.file {
            cmds.push(Cmd::ReadConfig(path.clone()));
        }
        cmds
    }

    /// The config file, read for the marks; one that doesn't read leaves them off.
    pub(super) fn settings_file(&mut self, file: &str, text: &str) {
        let Some(popup) = &mut self.settings else {
            return;
        };
        match Saved::read(file, text) {
            Ok(saved) => popup.file = Some(saved),
            Err(e) => self.status = Some(format!("can't mark the changes: {e}")),
        }
    }

    pub(super) fn settings_move(&mut self, by: i32) {
        if let Some(popup) = &mut self.settings {
            let row = popup.row as i64 + i64::from(by);
            popup.row = row.clamp(0, Row::ALL.len() as i64 - 1) as usize;
        }
    }

    /// Change the focused row: flip or step it, or, on the theme, open the theme list.
    pub(super) fn settings_change(&mut self, change: Change) -> Vec<Cmd> {
        let Some(row) = self.settings.as_ref().map(SettingsPopup::focused) else {
            return Vec::new();
        };
        self.change_setting(row, change)
    }

    fn change_setting(&mut self, row: Row, change: Change) -> Vec<Cmd> {
        if !self.setting_enabled(row) {
            self.status = Some("image protocol: images are off".into());
            return Vec::new();
        }
        match (row, self.setting(row)) {
            (Row::Theme, _) if change != Change::Step(Dir::Prev) => {
                self.list_themes();
                self.open_outline(Listing::Themes)
            }
            (_, Value::Choice(names, i)) => self.set_choice(row, change.choice(i, names.len())),
            (Row::ThemeBg, Value::Switch(on)) => self.set_theme_bg(change.switch(on)),
            (Row::CodeWrap, Value::Switch(on)) => self.set_code_wrap(change.switch(on)),
            (Row::Images, Value::Switch(on)) => {
                let mode = if change.switch(on) {
                    self.images_on()
                } else {
                    ImageMode::Off
                };
                self.set_images(mode);
                Vec::new()
            }
            (Row::Mouse, Value::Switch(on)) => self.set_mouse(change.switch(on)),
            (Row::KeepScreen, Value::Switch(on)) => {
                self.set_keep_screen(change.switch(on));
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// A digit typed into the prose width, or (`None`) one taken off. A width below
    /// [`MIN_WIDTH`] is on its way, and lays nothing out; none is the terminal's.
    pub(super) fn settings_width(&mut self, digit: Option<u8>) -> Vec<Cmd> {
        let Some(popup) = self.settings.as_mut().filter(|p| p.focused() == Row::Width) else {
            return Vec::new();
        };
        match digit {
            Some(d) if d > 9 => return Vec::new(),
            // `0` alone is the terminal's width
            Some(0) if popup.width.is_empty() => {}
            Some(d) if popup.width.len() < WIDTH_DIGITS => popup.width.push(char::from(b'0' + d)),
            Some(_) => return Vec::new(),
            None => {
                popup.width.pop();
            }
        }
        match popup.width.parse::<u16>().ok() {
            None => self.set_max_width(None),
            Some(w) if w >= MIN_WIDTH => self.set_max_width(Some(w)),
            Some(_) => {
                self.status = Some(format!("prose width: at least {MIN_WIDTH} columns"));
                Vec::new()
            }
        }
    }

    /// A click in the popup: focus the row, and change what's under it.
    pub(super) fn settings_click(&mut self, col: u16, row: u16) -> Vec<Cmd> {
        let Some(hit) = crate::ui::settings_hit(self, col, row) else {
            return Vec::new();
        };
        let mut focus = |i: usize| {
            if let Some(popup) = &mut self.settings {
                popup.row = i;
            }
            Row::ALL[i]
        };
        match hit {
            Hit::Row(i) => {
                focus(i);
                Vec::new()
            }
            Hit::Value(i) => {
                let row = focus(i);
                self.change_setting(row, Change::Flip)
            }
            Hit::Choice(i, choice) => {
                let row = focus(i);
                if self.setting_enabled(row) {
                    self.set_choice(row, choice)
                } else {
                    self.change_setting(row, Change::Flip)
                }
            }
            Hit::Keep => self.settings_keep(),
            Hit::Save => self.settings_save(),
            Hit::Cancel => self.settings_cancel(),
        }
    }

    /// Set a row of choices to its `i`th.
    fn set_choice(&mut self, row: Row, i: usize) -> Vec<Cmd> {
        match row {
            Row::Tables => {
                let modes = [TableMode::Auto, TableMode::Grid, TableMode::Records];
                modes
                    .get(i)
                    .map_or_else(Vec::new, |&mode| self.set_table_mode(mode))
            }
            Row::LineNumbers => LINE_NUMBERS
                .get(i)
                .and_then(|name| LineNumbers::parse(name))
                .map_or_else(Vec::new, |n| self.set_line_numbers(n)),
            Row::Protocol => {
                if let Some(name) = PROTOCOLS.get(i) {
                    self.set_images(Proto::parse(name).map_or(ImageMode::Auto, ImageMode::Force));
                }
                Vec::new()
            }
            Row::CopyOn => {
                if let Some(copy_on) = COPY_ON.get(i).and_then(|name| CopyOn::parse(name)) {
                    self.set_copy_on(copy_on);
                }
                Vec::new()
            }
            Row::LiveReload => {
                if let Some(mode) = LiveReload::NAMES
                    .get(i)
                    .and_then(|name| LiveReload::parse(name))
                {
                    self.set_live_reload(mode);
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// `a`: close, keeping the changes for the session.
    pub(super) fn settings_keep(&mut self) -> Vec<Cmd> {
        let Some(popup) = self.settings.take() else {
            return Vec::new();
        };
        let now = self.active_settings();
        let start: Vec<&str> = Row::ALL
            .iter()
            .filter(|r| r.at_start() && differs(&popup.opened, &now, **r))
            .map(|r| r.option())
            .collect();
        self.status = Some(if start.is_empty() {
            "settings kept for this session".to_owned()
        } else {
            format!(
                "settings kept for this session; {} once saved and restarted",
                start.join(", ")
            )
        });
        self.visible_jobs()
    }

    /// `s`: close, keeping the changes, and save them to the config file: no box asks first, as
    /// the popup's marks said what changes.
    pub(super) fn settings_save(&mut self) -> Vec<Cmd> {
        if self.settings.take().is_none() {
            return Vec::new();
        }
        let mut cmds = self.save_config();
        self.config_for = Some(ConfigUse::Save);
        cmds.extend(self.visible_jobs());
        cmds
    }

    /// `Esc`: close, putting back the settings the popup opened with.
    pub(super) fn settings_cancel(&mut self) -> Vec<Cmd> {
        let Some(popup) = self.settings.take() else {
            return Vec::new();
        };
        let was = popup.opened;
        let mut cmds = self.switch_theme(&popup.theme);
        if self.theme_bg != was.theme_bg {
            cmds.extend(self.set_theme_bg(was.theme_bg));
        }
        if self.mouse != was.mouse {
            cmds.extend(self.set_mouse(was.mouse));
        }
        self.config.copy_on = was.copy_on;
        self.config.max_width = was.max_width;
        self.table_mode = was.table_mode;
        self.code_wrap = was.code_wrap;
        self.line_numbers = was.line_numbers;
        self.set_images(was.images);
        self.set_live_reload(was.live_reload);
        self.set_keep_screen(was.keep_screen);
        self.relayout();
        self.status = None;
        cmds.extend(self.visible_jobs());
        cmds
    }

    // --- the setters the keys and the popup share -------------------------------------------

    pub(super) fn set_table_mode(&mut self, mode: TableMode) -> Vec<Cmd> {
        self.table_mode = mode;
        self.status = Some(format!("tables: {}", mode.name()));
        self.relayout();
        self.visible_jobs()
    }

    pub(super) fn set_code_wrap(&mut self, on: bool) -> Vec<Cmd> {
        self.code_wrap = on;
        let mode = if on {
            "wrap"
        } else {
            "no-wrap, ←/→ scroll"
        };
        self.status = Some(format!("code: {mode}"));
        self.relayout();
        self.visible_jobs()
    }

    pub(super) fn set_line_numbers(&mut self, numbers: LineNumbers) -> Vec<Cmd> {
        self.line_numbers = numbers;
        self.status = Some(format!("line numbers: {}", numbers.name()));
        self.relayout();
        self.visible_jobs()
    }

    /// `#`'s next step for the kind of file shown.
    pub(super) fn toggle_line_numbers(&mut self) -> Vec<Cmd> {
        let code = self
            .doc
            .as_ref()
            .is_some_and(|d| matches!(d.filetype, FileType::Text { .. }));
        self.set_line_numbers(self.line_numbers.toggle(code))
    }

    pub(super) fn set_theme_bg(&mut self, on: bool) -> Vec<Cmd> {
        self.theme_bg = on;
        let bg = if on { "the theme's" } else { "the terminal's" };
        self.status = Some(format!("background: {bg}"));
        self.set_theme(self.designed.clone())
    }

    pub(super) fn set_mouse(&mut self, on: bool) -> Vec<Cmd> {
        self.mouse = on;
        self.status = Some(if on {
            "mouse: on, a drag selects in ada, Shift-drag in the terminal".to_owned()
        } else {
            "mouse: off, the terminal selects text".to_owned()
        });
        self.clear_selection();
        vec![Cmd::Mouse(on)]
    }

    fn set_copy_on(&mut self, copy_on: CopyOn) {
        self.config.copy_on = copy_on;
        self.status = Some(match copy_on {
            CopyOn::Yank => "copy a selection: on y".to_owned(),
            CopyOn::Select => "copy a selection: as it's made".to_owned(),
        });
    }

    pub(super) fn set_max_width(&mut self, width: Option<u16>) -> Vec<Cmd> {
        self.config.max_width = width;
        self.status = Some(match width {
            Some(w) => format!("prose width: at most {w} columns"),
            None => "prose width: the terminal's".to_owned(),
        });
        self.relayout();
        self.visible_jobs()
    }

    /// Images on or off, or their protocol, for the next start.
    fn set_images(&mut self, mode: ImageMode) {
        if mode != ImageMode::Off
            && let Some(popup) = &mut self.settings
        {
            popup.images_on = mode;
        }
        self.next_start.images = mode;
        let what = match mode {
            ImageMode::Off => "off",
            ImageMode::Auto => "on",
            ImageMode::Force(proto) => proto.name(),
        };
        self.status = Some(format!("images: {what}, once saved and restarted"));
    }

    fn set_live_reload(&mut self, mode: LiveReload) {
        self.next_start.live_reload = mode;
        self.status = Some(format!(
            "live reload: {}, once saved and restarted",
            mode.name()
        ));
    }

    /// Read at quit, so it counts for this session too.
    fn set_keep_screen(&mut self, on: bool) {
        self.config.keep_screen = on;
        self.status = Some(if on {
            "on quit: the screen stays in the terminal".to_owned()
        } else {
            "on quit: the screen is cleared".to_owned()
        });
    }
}

/// Whether `row`'s option differs between two settings, for any row.
#[cfg(test)]
fn differs_any(a: &Settings, b: &Settings, row: Row) -> bool {
    match row {
        Row::Theme => a.theme != b.theme,
        Row::ThemeBg => a.theme_bg != b.theme_bg,
        Row::Width => a.max_width != b.max_width,
        Row::Tables => a.table_mode != b.table_mode,
        Row::CodeWrap => a.code_wrap != b.code_wrap,
        Row::LineNumbers => a.line_numbers != b.line_numbers,
        Row::Mouse => a.mouse != b.mouse,
        Row::CopyOn => a.copy_on != b.copy_on,
        Row::KeepScreen => a.keep_screen != b.keep_screen,
        _ => differs(a, b, row),
    }
}

/// Whether a start-only `row`'s option differs between two settings.
fn differs(a: &Settings, b: &Settings, row: Row) -> bool {
    match row {
        Row::Images => (a.images == ImageMode::Off) != (b.images == ImageMode::Off),
        Row::Protocol => {
            a.images != b.images && a.images != ImageMode::Off && b.images != ImageMode::Off
        }
        Row::LiveReload => a.live_reload != b.live_reload,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Computed, Config, Msg, Size, update};
    use crate::config;
    use crate::theme::Theme;

    fn sample() -> String {
        let mut md = String::from("# Settings\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n");
        for i in 0..40 {
            md.push_str(&format!(
                "paragraph {i} with words enough to wrap at any width.\n\n"
            ));
        }
        md.push_str("```rust\nfn main() { println!(\"a long line of code, longer than the screen is wide\"); }\n```\n");
        md
    }

    /// A document on a 100×30 screen, with a config file.
    fn opened() -> State {
        let config = Config {
            file: Some("c.toml".into()),
            ..Config::default()
        };
        let mut state = State::new(Theme::builtin(), config);
        update(
            &mut state,
            Msg::Init {
                size: Size { w: 100, h: 30 },
            },
        );
        update(
            &mut state,
            Msg::DocLoaded {
                path: "t.md".into(),
                mtime: None,
                text: sample().into(),
                doc: Computed(None),
            },
        );
        state
    }

    /// `F2`, the file reading `text`.
    fn open(state: &mut State, text: &str) {
        let cmds = update(state, Msg::OpenSettings);
        assert!(cmds.contains(&Cmd::ReadConfig("c.toml".into())), "{cmds:?}");
        update(
            state,
            Msg::ConfigRead {
                path: "c.toml".into(),
                text: text.into(),
            },
        );
        assert!(state.settings.is_some() && state.save.is_none());
    }

    fn focus(state: &mut State, row: Row) {
        let i = Row::ALL.iter().position(|&r| r == row).unwrap();
        update(state, Msg::SettingsMove { by: -99 });
        update(state, Msg::SettingsMove { by: i as i32 });
    }

    fn marked(state: &State) -> Vec<&'static str> {
        Row::ALL
            .iter()
            .filter(|&&r| state.setting_changed(r))
            .map(|r| r.option())
            .collect()
    }

    #[test]
    fn switches_flip_and_choices_wrap() {
        let mut state = opened();
        open(&mut state, "");
        focus(&mut state, Row::CodeWrap);
        update(&mut state, Msg::SettingsFlip);
        assert!(!state.code_wrap);
        // `→` turns a switch on, `←` off, whatever it was
        update(&mut state, Msg::SettingsStep { dir: Dir::Next });
        update(&mut state, Msg::SettingsStep { dir: Dir::Next });
        assert!(state.code_wrap);
        update(&mut state, Msg::SettingsStep { dir: Dir::Prev });
        assert!(!state.code_wrap);

        focus(&mut state, Row::Tables);
        let mut seen = Vec::new();
        for _ in 0..4 {
            update(&mut state, Msg::SettingsStep { dir: Dir::Prev });
            seen.push(state.table_mode.name());
        }
        assert_eq!(seen, ["records", "grid", "auto", "records"]);
        update(&mut state, Msg::SettingsFlip);
        assert_eq!(state.table_mode, TableMode::Auto, "Space takes the next");

        // every choice of line numbers, markdown or not
        focus(&mut state, Row::LineNumbers);
        let mut seen = Vec::new();
        for _ in 0..3 {
            update(&mut state, Msg::SettingsStep { dir: Dir::Next });
            seen.push(state.line_numbers.spelling());
        }
        assert_eq!(seen, ["code", "all", "off"]);

        // the moves stop at the ends
        update(&mut state, Msg::SettingsMove { by: 99 });
        assert_eq!(state.settings.as_ref().unwrap().focused(), Row::KeepScreen);
        update(&mut state, Msg::SettingsMove { by: -99 });
        assert_eq!(state.settings.as_ref().unwrap().focused(), Row::Theme);
    }

    #[test]
    fn the_width_takes_digits_within_bounds() {
        let mut state = opened();
        open(&mut state, "");
        let width = |s: &State| s.config.max_width;
        let typed = |s: &State| s.settings.as_ref().unwrap().width.clone();
        // only on its own row
        update(&mut state, Msg::SettingsDigit { digit: 8 });
        assert_eq!(typed(&state), "");
        focus(&mut state, Row::Width);
        // `0` alone is the terminal's, and a width on its way lays nothing out
        update(&mut state, Msg::SettingsDigit { digit: 0 });
        assert_eq!((typed(&state), width(&state)), (String::new(), None));
        update(&mut state, Msg::SettingsDigit { digit: 1 });
        assert_eq!((typed(&state), width(&state)), ("1".into(), None));
        assert!(state.status.as_deref().unwrap().contains("at least 20"));
        let rows = state.doc.as_ref().unwrap().layout.row_count();
        update(&mut state, Msg::SettingsDigit { digit: 0 });
        update(&mut state, Msg::SettingsDigit { digit: 0 });
        assert_eq!(width(&state), Some(100));
        update(&mut state, Msg::SettingsErase);
        assert_eq!(width(&state), Some(100), "10 is on its way");
        update(&mut state, Msg::SettingsDigit { digit: 6 });
        assert_eq!(width(&state), Some(106));
        for _ in 0..3 {
            update(&mut state, Msg::SettingsErase);
        }
        assert_eq!(width(&state), None, "none typed: the terminal's");
        update(&mut state, Msg::SettingsDigit { digit: 4 });
        update(&mut state, Msg::SettingsDigit { digit: 0 });
        assert_eq!(width(&state), Some(40));
        assert!(
            state.doc.as_ref().unwrap().layout.row_count() > rows,
            "laid out narrower"
        );
        // three digits at most
        update(&mut state, Msg::SettingsDigit { digit: 5 });
        update(&mut state, Msg::SettingsDigit { digit: 5 });
        assert_eq!((typed(&state), width(&state)), ("405".into(), Some(405)));
        for _ in 0..4 {
            update(&mut state, Msg::SettingsErase);
        }
        assert_eq!((typed(&state), width(&state)), (String::new(), None));
        assert!(update(&mut state, Msg::SettingsDigit { digit: 12 }).is_empty());
    }

    /// Everything the keys change, changed in the popup instead, lands where the keys do.
    #[test]
    fn changes_reach_the_state_the_keys_do() {
        let mut keys = opened();
        for msg in [
            Msg::ToggleThemeBg,
            Msg::ToggleTableMode,
            Msg::ToggleCodeWrap,
            Msg::ToggleLineNumbers,
            Msg::ToggleMouse,
        ] {
            update(&mut keys, msg);
        }
        let mut popup = opened();
        open(&mut popup, "");
        for row in [
            Row::ThemeBg,
            Row::Tables,
            Row::CodeWrap,
            Row::LineNumbers,
            Row::Mouse,
        ] {
            focus(&mut popup, row);
            let cmds = update(&mut popup, Msg::SettingsFlip);
            if row == Row::Mouse {
                assert_eq!(cmds, [Cmd::Mouse(false)]);
            }
        }
        // the markdown is numbered either way: `#` numbers it as `all`
        assert_eq!(popup.line_numbers, LineNumbers::Code);
        focus(&mut popup, Row::LineNumbers);
        update(&mut popup, Msg::SettingsFlip);
        update(&mut popup, Msg::SettingsKeep);
        assert!(popup.settings.is_none());
        assert_eq!(popup.active_settings(), keys.active_settings());
        let (a, b) = (popup.doc.as_ref().unwrap(), keys.doc.as_ref().unwrap());
        assert_eq!(format!("{:?}", a.layout), format!("{:?}", b.layout));
        assert_eq!(popup.theme.styles, keys.theme.styles);
        assert_eq!(popup.scroll, keys.scroll);
    }

    #[test]
    fn marks_follow_the_file_and_s_writes_what_capital_s_would() {
        let file = "tables = \"grid\"\nkeep_screen = false\n";
        let mut state = opened();
        open(&mut state, file);
        assert_eq!(marked(&state), ["tables", "keep_screen"]);
        focus(&mut state, Row::Tables);
        update(&mut state, Msg::SettingsStep { dir: Dir::Next });
        assert_eq!(state.table_mode, TableMode::Grid);
        assert_eq!(marked(&state), ["keep_screen"], "the file's value again");
        focus(&mut state, Row::KeepScreen);
        update(&mut state, Msg::SettingsFlip);
        focus(&mut state, Row::LiveReload);
        update(&mut state, Msg::SettingsFlip);
        assert_eq!(marked(&state), ["live_reload"]);

        // `S` with the same settings: the box's text is what `s` writes
        let mut boxed = State::new(Theme::builtin(), state.config.clone());
        boxed.table_mode = state.table_mode;
        boxed.next_start = state.next_start;
        update(&mut boxed, Msg::SaveConfig);
        update(
            &mut boxed,
            Msg::ConfigRead {
                path: "c.toml".into(),
                text: file.into(),
            },
        );
        let Some(Cmd::WriteConfig { text: want, .. }) =
            update(&mut boxed, Msg::SaveConfirm).first().cloned()
        else {
            panic!("no write");
        };

        let cmds = update(&mut state, Msg::SettingsSave);
        assert_eq!(cmds.first(), Some(&Cmd::ReadConfig("c.toml".into())));
        assert!(state.settings.is_none());
        let cmds = update(
            &mut state,
            Msg::ConfigRead {
                path: "c.toml".into(),
                text: file.into(),
            },
        );
        assert_eq!(
            cmds,
            [Cmd::WriteConfig {
                path: "c.toml".into(),
                text: want.clone()
            }],
            "no box: the marks said what changes"
        );
        assert!(state.save.is_none());
        assert_eq!(
            want,
            "tables = \"grid\"\nkeep_screen = false\nlive_reload = \"notify\"\n"
        );
        let back = config::parse("c.toml", &want, &mut Vec::new()).unwrap();
        assert!(!back.keep_screen);
        assert_eq!(back.live_reload, LiveReload::Notify);
    }

    #[test]
    fn copy_on_has_no_key_but_the_popup_sets_and_saves_it() {
        let mut state = opened();
        open(&mut state, "");
        focus(&mut state, Row::CopyOn);
        assert_eq!(state.setting(Row::CopyOn), Value::Choice(COPY_ON, 0));
        update(&mut state, Msg::SettingsStep { dir: Dir::Next });
        assert_eq!(state.config.copy_on, CopyOn::Select);
        assert_eq!(marked(&state), ["copy_on"]);
        update(&mut state, Msg::SettingsFlip);
        assert_eq!(state.config.copy_on, CopyOn::Yank, "wraps");
        update(&mut state, Msg::SettingsFlip);
        let save = config::save("c.toml", "", &state.active_settings()).unwrap();
        assert_eq!(save.text, "copy_on = \"select\"\n");
        let back = config::parse("c.toml", &save.text, &mut Vec::new()).unwrap();
        assert_eq!(back.copy_on, CopyOn::Select);
    }

    #[test]
    fn nothing_to_save_says_so() {
        let mut state = opened();
        open(&mut state, "");
        update(&mut state, Msg::SettingsSave);
        let cmds = update(
            &mut state,
            Msg::ConfigRead {
                path: "c.toml".into(),
                text: "".into(),
            },
        );
        assert!(cmds.is_empty());
        assert!(
            state
                .status
                .as_deref()
                .unwrap()
                .starts_with("nothing to save")
        );

        // no config file: kept, and the status line says why it isn't saved
        let mut state = opened();
        state.config.file = None;
        assert!(update(&mut state, Msg::OpenSettings).is_empty());
        focus(&mut state, Row::CodeWrap);
        update(&mut state, Msg::SettingsFlip);
        assert!(
            update(&mut state, Msg::SettingsSave)
                .iter()
                .all(|c| !matches!(c, Cmd::ReadConfig(_)))
        );
        assert!(!state.code_wrap);
        assert!(state.status.as_deref().unwrap().contains("no config file"));
    }

    #[test]
    fn esc_puts_back_what_it_opened_with() {
        let mut state = opened();
        update(
            &mut state,
            Msg::Scroll {
                by: crate::app::ScrollBy::Rows(20),
            },
        );
        let (before, layout, styles) = (
            state.active_settings(),
            format!("{:?}", state.doc.as_ref().unwrap().layout),
            state.theme.styles,
        );
        open(&mut state, "");
        for (row, steps) in [
            (Row::ThemeBg, 1),
            (Row::Tables, 2),
            (Row::CodeWrap, 1),
            (Row::LineNumbers, 1),
            (Row::Images, 1),
            (Row::Mouse, 1),
            (Row::CopyOn, 1),
            (Row::LiveReload, 1),
            (Row::KeepScreen, 1),
        ] {
            focus(&mut state, row);
            for _ in 0..steps {
                update(&mut state, Msg::SettingsFlip);
            }
        }
        focus(&mut state, Row::Width);
        for digit in [6, 0] {
            update(&mut state, Msg::SettingsDigit { digit });
        }
        // and a theme, picked from the list the theme row opens
        focus(&mut state, Row::Theme);
        update(&mut state, Msg::SettingsFlip);
        assert!(state.outline.is_some() && state.settings.is_some());
        update(&mut state, Msg::ThemesTone { dir: Dir::Next });
        update(&mut state, Msg::OutlinePick);
        assert_eq!(state.theme.name, "gruvbox-light");
        assert!(state.settings.is_some(), "back to the settings");

        let changed = state.active_settings();
        for row in Row::ALL {
            if row != Row::Protocol {
                assert!(
                    super::differs_any(&before, &changed, row),
                    "{} unchanged",
                    row.option()
                );
            }
        }
        let cmds = update(&mut state, Msg::SettingsCancel);
        assert!(cmds.contains(&Cmd::Mouse(true)), "{cmds:?}");
        assert!(state.settings.is_none() && state.status.is_none());
        assert_eq!(state.active_settings(), before);
        assert_eq!(format!("{:?}", state.doc.as_ref().unwrap().layout), layout);
        assert_eq!(state.theme.styles, styles);
    }

    #[test]
    fn the_theme_list_returns_to_the_settings() {
        let mut state = opened();
        open(&mut state, "");
        // `←` on the theme does nothing; `→` and `Enter` list the themes
        assert!(update(&mut state, Msg::SettingsStep { dir: Dir::Prev }).is_empty());
        assert!(state.outline.is_none());
        update(&mut state, Msg::SettingsStep { dir: Dir::Next });
        assert!(state.outline.is_some());
        update(&mut state, Msg::OutlineMove { by: 1 });
        assert_ne!(state.theme.name, "gruvbox-dark", "previewed");
        // `Esc` in the list puts its theme back, and the settings stay open
        update(&mut state, Msg::OutlineClose);
        assert_eq!(state.theme.name, "gruvbox-dark");
        assert!(state.settings.is_some());
        assert_eq!(marked(&state), Vec::<&str>::new());
    }

    #[test]
    fn start_only_options_are_saved_not_applied() {
        let mut state = opened();
        open(&mut state, "");
        let session = (state.config.images, state.config.live_reload);
        focus(&mut state, Row::Protocol);
        update(&mut state, Msg::SettingsStep { dir: Dir::Next });
        assert_eq!(state.next_start.images, ImageMode::Force(Proto::Kitty));
        assert_eq!(
            state.status.as_deref(),
            Some("images: kitty, once saved and restarted")
        );
        // with images off, the protocol stays and can't be changed; back on, it's the one it was
        focus(&mut state, Row::Images);
        update(&mut state, Msg::SettingsFlip);
        assert_eq!(state.next_start.images, ImageMode::Off);
        assert!(!state.setting_enabled(Row::Protocol));
        assert_eq!(state.setting(Row::Protocol), Value::Choice(PROTOCOLS, 1));
        focus(&mut state, Row::Protocol);
        update(&mut state, Msg::SettingsFlip);
        assert_eq!(state.next_start.images, ImageMode::Off);
        focus(&mut state, Row::Images);
        update(&mut state, Msg::SettingsFlip);
        assert_eq!(state.next_start.images, ImageMode::Force(Proto::Kitty));
        focus(&mut state, Row::LiveReload);
        update(&mut state, Msg::SettingsFlip);
        assert_eq!(state.next_start.live_reload, LiveReload::Notify);
        assert_eq!(
            state.status.as_deref(),
            Some("live reload: notify, once saved and restarted")
        );
        update(&mut state, Msg::SettingsKeep);
        assert_eq!((state.config.images, state.config.live_reload), session);
        let status = state.status.clone().unwrap();
        assert!(
            status.ends_with("; image_protocol, live_reload once saved and restarted"),
            "{status}"
        );
        // a reload watches the file as the session started
        let cmds = update(&mut state, Msg::Reload);
        update(&mut state, crate::app::loader_answer(&cmds, &sample()));
        assert!(state.watch_cmd().is_some());
        // and `S` saves them
        let save = config::save("c.toml", "", &state.active_settings()).unwrap();
        let keys: Vec<_> = save.changes.iter().map(|(k, ..)| *k).collect();
        assert_eq!(keys, ["image_protocol", "live_reload"]);
    }

    #[test]
    fn clicks_change_what_they_land_on() {
        let mut state = opened();
        open(&mut state, "");
        let grid = |state: &State| {
            let mut grid = crate::grid::Grid::empty(crate::geom::Rect::new(0, 0, 100, 30));
            crate::ui::view(state, &mut grid);
            grid
        };
        // the cell showing `text` on the row labeled `label`
        let find = |state: &State, label: &str, text: &str| {
            let frame = crate::record::frame_text(&grid(state));
            let (y, line) = frame
                .lines()
                .enumerate()
                .find(|(_, l)| l.contains(label))
                .unwrap();
            let at = line.find(text).unwrap();
            ((line[..at].chars().count()) as u16, y as u16)
        };
        let click = |state: &mut State, (col, row): (u16, u16)| {
            update(state, Msg::SettingsClick { col, row })
        };
        let at = find(&state, "Tables", "records");
        click(&mut state, at);
        assert_eq!(state.table_mode, TableMode::Records);
        assert_eq!(state.settings.as_ref().unwrap().focused(), Row::Tables);
        let at = find(&state, "Wrap code", "on");
        click(&mut state, at);
        assert!(!state.code_wrap);
        // a label only focuses
        let at = find(&state, "Mouse", "Mouse");
        click(&mut state, at);
        assert!(state.mouse);
        assert_eq!(state.settings.as_ref().unwrap().focused(), Row::Mouse);
        // the theme lists the themes
        let at = find(&state, "Theme ", "gruvbox-dark");
        click(&mut state, at);
        assert!(state.outline.is_some());
        update(&mut state, Msg::OutlineClose);
        // outside the popup: nothing
        assert!(click(&mut state, (0, 0)).is_empty());
        assert!(state.settings.is_some());
        // the footer's ways out
        let at = find(&state, "keep for this session", "esc");
        click(&mut state, at);
        assert!(state.settings.is_none());
        assert!(
            state.code_wrap && state.table_mode == TableMode::Auto,
            "undone"
        );
    }

    #[test]
    fn the_popup_takes_the_place_of_other_popups() {
        let mut state = opened();
        update(&mut state, Msg::OpenThemes);
        update(&mut state, Msg::OutlineMove { by: 1 });
        update(&mut state, Msg::OpenSettings);
        assert!(state.outline.is_none());
        assert_eq!(
            state.theme.name, "gruvbox-dark",
            "the list's preview went back"
        );
        // a click on the document behind it follows nothing
        update(&mut state, Msg::SettingsKeep);
        update(&mut state, Msg::Help);
        update(&mut state, Msg::OpenSettings);
        assert!(state.help.is_none());
    }
}
