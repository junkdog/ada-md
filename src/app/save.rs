//! `S`: saving the settings in effect to the config file. The file is read when `S` is pressed,
//! so edits made to it meanwhile count; a box lists what saving changes, and only an answer
//! writes it. Whatever is in effect counts, a key's toggle or a flag.

use std::path::PathBuf;

use super::{Cmd, State};
use crate::config::{self, Save, Settings};

/// The save box, while it is open: the file, and what saving changes in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saving {
    pub path: PathBuf,
    pub save: Save,
}

impl State {
    /// The settings in effect, as the config file sets them. A theme from a path is that path,
    /// which loads again where its name wouldn't.
    pub fn active_settings(&self) -> Settings {
        let theme = self.theme.path.as_ref().unwrap_or(&self.theme.name);
        Settings {
            theme: Some(theme.clone()),
            max_width: self.config.max_width,
            images: self.config.images,
            live_reload: self.config.live_reload,
            keep_screen: self.config.keep_screen,
            table_mode: self.table_mode,
            code_wrap: self.code_wrap,
            line_numbers: self.line_numbers,
            keys: self.config.keys.clone(),
            // `S` saves neither keys nor links, and a session can't change the links
            links: Default::default(),
        }
    }

    pub(super) fn save_config(&mut self) -> Vec<Cmd> {
        match &self.config.file {
            Some(path) => vec![Cmd::ReadConfig(path.clone())],
            None => {
                self.status = Some("no config file to save to: $HOME isn't set".into());
                Vec::new()
            }
        }
    }

    /// Open the save box on what saving changes in the file's `text`; when it changes nothing,
    /// or the file doesn't read, the status line says so instead.
    pub(super) fn config_read(&mut self, path: PathBuf, text: &str) {
        match config::save(&path.display().to_string(), text, &self.active_settings()) {
            Err(e) => self.status = Some(format!("can't save: {e}")),
            Ok(save) if save.changes.is_empty() => {
                self.status = Some(format!(
                    "nothing to save: {} has these settings",
                    path.display()
                ));
            }
            Ok(save) => {
                self.status = None;
                self.save = Some(Saving { path, save });
            }
        }
    }

    pub(super) fn save_confirm(&mut self) -> Vec<Cmd> {
        let Some(Saving { path, save }) = self.save.take() else {
            return Vec::new();
        };
        let mut cmds = vec![Cmd::WriteConfig {
            path,
            text: save.text,
        }];
        cmds.extend(self.visible_jobs());
        cmds
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Config, Msg, update};
    use crate::images::{ImageMode, Proto};
    use crate::theme::Theme;

    fn fresh(config: Config) -> State {
        let mut state = State::new(Theme::builtin(), config);
        update(
            &mut state,
            Msg::Init {
                size: super::super::Size { w: 80, h: 24 },
            },
        );
        state
    }

    fn with_file() -> Config {
        Config {
            file: Some("c.toml".into()),
            ..Config::default()
        }
    }

    /// `S`, with the file reading `text`: the commands, and the box's rows.
    fn press_s(state: &mut State, text: &str) -> Vec<String> {
        assert_eq!(
            update(state, Msg::SaveConfig),
            [Cmd::ReadConfig("c.toml".into())]
        );
        update(
            state,
            Msg::ConfigRead {
                path: "c.toml".into(),
                text: text.into(),
            },
        );
        state.save.as_ref().map_or_else(Vec::new, |s| {
            s.save
                .changes
                .iter()
                .map(|(key, from, to)| format!("{key}: {from} → {to}"))
                .collect()
        })
    }

    #[test]
    fn toggles_list_what_they_change() {
        let mut state = fresh(with_file());
        let text = config::default_text();
        assert!(press_s(&mut state, &text).is_empty());
        assert_eq!(
            state.status.as_deref(),
            Some("nothing to save: c.toml has these settings")
        );

        update(&mut state, Msg::ToggleTableMode);
        update(&mut state, Msg::ToggleCodeWrap);
        update(&mut state, Msg::ToggleLineNumbers);
        assert_eq!(
            press_s(&mut state, &text),
            [
                "tables: \"auto\" → \"grid\"",
                "code_wrap: true → false",
                "line_numbers: \"off\" → \"all\"",
            ]
        );
        // `Esc` writes nothing
        update(&mut state, Msg::SaveCancel);
        assert!(state.save.is_none());

        press_s(&mut state, &text);
        let cmds = update(&mut state, Msg::SaveConfirm);
        let Some(Cmd::WriteConfig { path, text: saved }) = cmds.first() else {
            panic!("{cmds:?}");
        };
        assert_eq!(path.to_str(), Some("c.toml"));
        assert!(saved.contains("\ntables = \"grid\"\n"), "{saved}");
        assert!(state.save.is_none());
        update(
            &mut state,
            Msg::ConfigWritten {
                path: "c.toml".into(),
            },
        );
        assert_eq!(state.status.as_deref(), Some("saved c.toml"));

        // the file has them now
        assert!(press_s(&mut state, saved).is_empty());
    }

    #[test]
    fn flags_count_before_any_key() {
        let (theme, _) = crate::theme::load(Some("dracula")).unwrap();
        let config = Config {
            max_width: Some(90),
            images: ImageMode::Force(Proto::Kitty),
            live_reload: false,
            ..with_file()
        };
        let mut state = State::new(theme, config);
        assert_eq!(
            press_s(&mut state, "theme = \"gruvbox-light\"\n"),
            [
                "theme: \"gruvbox-light\" → \"dracula\"",
                "max_width: 0 → 90",
                "image_protocol: \"auto\" → \"kitty\"",
                "live_reload: true → false",
            ]
        );
    }

    #[test]
    fn a_theme_from_a_path_is_saved_as_the_path() {
        let mut state = fresh(with_file());
        state.theme.name = "mine".into();
        state.theme.path = Some("~/x/mine.toml".into());
        assert_eq!(
            press_s(&mut state, ""),
            ["theme: \"gruvbox-dark\" → \"~/x/mine.toml\""]
        );
    }

    #[test]
    fn failures_go_to_the_status_line() {
        let mut state = fresh(with_file());
        state.code_wrap = false;
        assert!(press_s(&mut state, "code_wrap = 1\n").is_empty());
        assert_eq!(
            state.status.as_deref(),
            Some("can't save: c.toml:1:13: expected true or false, found integer")
        );
        let error = "can't write c.toml: Permission denied".to_owned();
        update(
            &mut state,
            Msg::ConfigFailed {
                error: error.clone(),
            },
        );
        assert_eq!(state.status, Some(error));

        let mut state = fresh(Config::default());
        assert!(update(&mut state, Msg::SaveConfig).is_empty());
        assert!(state.status.is_some());
        // an answer with no box open does nothing
        assert!(update(&mut state, Msg::SaveConfirm).is_empty());
    }
}
