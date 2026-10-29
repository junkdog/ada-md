//! Handing things to the world outside ada (P7): the file to an editor (`e`, ADR-18), text to
//! the clipboard (`y`, OSC 52), and the file to a watcher that notices when it changes on disk
//! (live reload, ADR-26). Each is a command; what comes back is a message.

use std::path::Path;

use super::{Action, Cmd, CopyOn, LinkState, ScrollAnchor, State};
use crate::config::LiveReload;
use crate::doc::LinkKind;
use crate::filetype::FileType;
use crate::images::ImageCache;

impl State {
    /// Watch the current document's file, when live reload is on. Stdin has no file.
    pub(super) fn watch_cmd(&self) -> Option<Cmd> {
        let doc = self
            .doc
            .as_ref()
            .filter(|d| self.config.live_reload != LiveReload::Off && d.path.as_os_str() != "-")?;
        Some(Cmd::Watch {
            path: doc.path.clone(),
            mtime: doc.mtime,
        })
    }

    /// The watched file changed on disk: read it again, or, with `notify` (FEAT-014), mark it
    /// until `r` does. A change back to the time of the text on screen takes the mark away.
    pub(super) fn file_changed(&mut self, path: &Path, mtime: Option<u64>) -> Vec<Cmd> {
        let Some(doc) = self.doc.as_mut().filter(|d| d.path == path) else {
            return Vec::new();
        };
        let changed = doc.mtime != mtime;
        match self.config.live_reload {
            LiveReload::Auto if changed => self.reload(),
            LiveReload::Notify => {
                if changed && !doc.changed {
                    let key = self.config.keys.keys(Action::Reload).first();
                    self.status = Some(match key {
                        Some(key) => format!("changed on disk: {key} reloads"),
                        None => "changed on disk".to_owned(),
                    });
                }
                doc.changed = changed;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// `e`: edit the file at the first source line on screen.
    pub(super) fn edit(&mut self) -> Vec<Cmd> {
        let Some(doc) = &self.doc else {
            return Vec::new();
        };
        if doc.path.as_os_str() == "-" {
            self.status = Some("stdin can't be edited".into());
            return Vec::new();
        }
        if doc.filetype == FileType::Image {
            self.status = Some("an image can't be edited".into());
            return Vec::new();
        }
        let line = ScrollAnchor::capture(&doc.layout, self.scroll).map_or(1, |a| a.line);
        vec![Cmd::SpawnEditor {
            file: doc.path.clone(),
            line,
        }]
    }

    /// Back from the editor: the screen was handed over, so images are sent again, and the
    /// file is read again unless the editor failed.
    pub(super) fn editor_exited(&mut self, status: i32) -> Vec<Cmd> {
        // A terminal forgets the images it showed when the screen is cleared.
        self.images = ImageCache::default();
        if status != 0 {
            self.status = Some(format!("the editor exited with status {status}"));
            return self.visible_jobs();
        }
        self.reload()
    }

    /// `y`: the selection, which goes once copied when `copy_on` is `yank`; or else the focused
    /// link's target, or else the first code block on screen.
    pub(super) fn yank(&mut self) -> Vec<Cmd> {
        if let Some(cmds) = self.copy_selection() {
            if self.config.copy_on == CopyOn::Yank {
                self.selection = None;
            }
            return cmds;
        }
        let Some(doc) = &self.doc else {
            return Vec::new();
        };
        if let Some(link) = self.visible_focus() {
            let Some(link_ref) = doc.doc.links.get(link as usize) else {
                return Vec::new();
            };
            if matches!(link_ref.kind, LinkKind::Footnote(_)) {
                self.status = Some("a footnote reference has no target to copy".into());
                return Vec::new();
            }
            let text = match doc.links.get(link as usize) {
                Some(LinkState::External(url)) => url.clone(),
                _ => link_ref.dest.to_string(),
            };
            self.status = Some(format!("copied {text}"));
            return vec![Cmd::Osc52(text)];
        }
        let blocks = doc.doc.code_blocks();
        let Some(code) = self
            .visible_code()
            .first()
            .and_then(|&b| blocks.get(b as usize))
        else {
            self.status =
                Some("nothing to copy: Tab focuses a link, or bring a code block on screen".into());
            return Vec::new();
        };
        let lines = code.lines().count();
        let what = code
            .lang
            .as_deref()
            .map_or_else(String::new, |l| format!("{l} "));
        let plural = if lines == 1 { "" } else { "s" };
        self.status = Some(format!("copied the {what}code block, {lines} line{plural}"));
        vec![Cmd::Osc52(code.text.to_string())]
    }
}

#[cfg(test)]
mod tests {
    use crate::app::{Cmd, Computed, Config, Dir, Msg, ScrollBy, Size, State, only_load, update};
    use crate::config::LiveReload;
    use crate::theme::Theme;

    fn loaded(path: &str, text: &str) -> State {
        let mut state = State::new(Theme::builtin(), Config::default());
        update(
            &mut state,
            Msg::Init {
                size: Size { w: 80, h: 12 },
            },
        );
        update(
            &mut state,
            Msg::DocLoaded {
                path: path.into(),
                mtime: Some(5),
                text: text.into(),
                doc: Computed(None),
            },
        );
        state
    }

    fn note() -> String {
        let mut md = String::from("# Note\n\nsee <https://example.com> and [b](b.md)\n\n");
        for i in 0..30 {
            md.push_str(&format!("line {i}\n\n"));
        }
        md.push_str("```rust\nfn main() {}\nfn other() {}\n```\n");
        md
    }

    #[test]
    fn loads_watch_the_file() {
        let mut state = State::new(Theme::builtin(), Config::default());
        let cmds = update(
            &mut state,
            Msg::DocLoaded {
                path: "a.md".into(),
                mtime: Some(5),
                text: "# a\n".into(),
                doc: Computed(None),
            },
        );
        assert!(cmds.contains(&Cmd::Watch {
            path: "a.md".into(),
            mtime: Some(5)
        }));

        for (live_reload, watched) in [(LiveReload::Notify, true), (LiveReload::Off, false)] {
            let config = Config {
                live_reload,
                ..Config::default()
            };
            let mut state = State::new(Theme::builtin(), config);
            let cmds = update(
                &mut state,
                Msg::DocLoaded {
                    path: "a.md".into(),
                    mtime: Some(5),
                    text: "# a\n".into(),
                    doc: Computed(None),
                },
            );
            let watch = cmds.iter().any(|c| matches!(c, Cmd::Watch { .. }));
            assert_eq!(watch, watched, "{live_reload:?}");
        }
    }

    #[test]
    fn a_changed_file_reloads() {
        let mut state = loaded("a.md", &note());
        let changed = |mtime| Msg::FileChanged {
            path: "a.md".into(),
            mtime,
        };
        assert!(update(&mut state, changed(Some(5))).is_empty(), "the same");
        let other = Msg::FileChanged {
            path: "b.md".into(),
            mtime: Some(9),
        };
        assert!(update(&mut state, other).is_empty(), "not the current file");
        assert_eq!(
            only_load(&update(&mut state, changed(Some(9)))),
            Some("a.md")
        );
    }

    /// FEAT-014: with `notify`, a change marks the file and loads nothing until `r`; a change
    /// back to the time on screen takes the mark away, and so does reading the file again.
    #[test]
    fn a_changed_file_is_marked_until_r_with_notify() {
        let mut state = State::new(
            Theme::builtin(),
            Config {
                live_reload: LiveReload::Notify,
                ..Config::default()
            },
        );
        update(
            &mut state,
            Msg::Init {
                size: Size { w: 80, h: 12 },
            },
        );
        let loaded = |mtime, text: &str| Msg::DocLoaded {
            path: "a.md".into(),
            mtime: Some(mtime),
            text: text.into(),
            doc: Computed(None),
        };
        update(&mut state, loaded(5, &note()));
        let changed = |mtime| Msg::FileChanged {
            path: "a.md".into(),
            mtime: Some(mtime),
        };
        let marked = |state: &State| state.doc.as_ref().unwrap().changed;

        assert!(update(&mut state, changed(9)).is_empty(), "no load");
        assert!(marked(&state));
        assert_eq!(state.status.as_deref(), Some("changed on disk: r reloads"));
        // a key clears the message; the mark stays, and a second change says nothing new
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Rows(1),
            },
        );
        assert!(update(&mut state, changed(10)).is_empty());
        assert!(marked(&state) && state.status.is_none());
        let other = Msg::FileChanged {
            path: "b.md".into(),
            mtime: Some(5),
        };
        update(&mut state, other);
        assert!(marked(&state), "not the current file");

        assert!(update(&mut state, changed(5)).is_empty());
        assert!(!marked(&state), "back to the text on screen");

        // `r` reads it again, the same text or a new one
        update(&mut state, changed(11));
        assert_eq!(only_load(&update(&mut state, Msg::Reload)), Some("a.md"));
        update(&mut state, loaded(11, &note()));
        assert!(!marked(&state), "the same text");
        update(&mut state, changed(12));
        assert_eq!(only_load(&update(&mut state, Msg::Reload)), Some("a.md"));
        update(&mut state, loaded(12, "# a new note\n"));
        assert!(!marked(&state), "a new text");
        assert_eq!(state.status.as_deref(), Some("reloaded"));
    }

    #[test]
    fn edit_at_the_top_line_and_reload_after() {
        let mut state = loaded("a.md", &note());
        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Rows(6),
            },
        );
        let cmds = update(&mut state, Msg::Edit);
        let [Cmd::SpawnEditor { file, line }] = cmds.as_slice() else {
            panic!("{cmds:?}");
        };
        assert_eq!(file.to_str(), Some("a.md"));
        assert_eq!(*line, 7, "the first source line on screen");

        assert_eq!(
            only_load(&update(&mut state, Msg::EditorExited { status: 0 })),
            Some("a.md")
        );
        // the reload is still on its way; a failed editor doesn't start another
        update(
            &mut state,
            Msg::DocLoaded {
                path: "a.md".into(),
                mtime: Some(6),
                text: note().into(),
                doc: Computed(None),
            },
        );
        let cmds = update(&mut state, Msg::EditorExited { status: 1 });
        assert!(!cmds.iter().any(|c| matches!(c, Cmd::LoadDoc { .. })));
        assert_eq!(
            state.status.as_deref(),
            Some("the editor exited with status 1")
        );

        let mut stdin = loaded("-", "# piped\n");
        assert!(update(&mut stdin, Msg::Edit).is_empty());
        assert_eq!(stdin.status.as_deref(), Some("stdin can't be edited"));
    }

    #[test]
    fn yank_the_link_or_the_code() {
        let mut state = loaded("a.md", &note());
        assert!(update(&mut state, Msg::Yank).is_empty());
        assert!(
            state
                .status
                .as_deref()
                .unwrap()
                .starts_with("nothing to copy")
        );

        update(&mut state, Msg::FocusLink { dir: Dir::Next });
        assert_eq!(
            update(&mut state, Msg::Yank),
            [Cmd::Osc52("https://example.com".into())]
        );
        update(&mut state, Msg::FocusLink { dir: Dir::Next });
        assert_eq!(update(&mut state, Msg::Yank), [Cmd::Osc52("b.md".into())]);

        update(
            &mut state,
            Msg::Scroll {
                by: ScrollBy::Bottom,
            },
        );
        assert_eq!(
            update(&mut state, Msg::Yank),
            [Cmd::Osc52("fn main() {}\nfn other() {}\n".into())]
        );
        assert_eq!(
            state.status.as_deref(),
            Some("copied the rust code block, 2 lines")
        );
    }
}
