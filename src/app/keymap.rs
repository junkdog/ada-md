//! Keys → semantic messages (plan §6). Pure: the only state is a pending prefix key for the
//! two-key bindings `]]` and `[[`, and the mode with what has been typed into the search prompt
//! or the outline filter. Recordings log the messages, not the keys, so they survive keymap
//! changes.
//!
//! The keymap runs on the input thread, so it tracks the mode itself: `/` opens the prompt here
//! and now, and the keys typed right after it can't race the main loop.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::{CODE_X_STEP, Dir, Msg, ScrollBy};

/// Rows `PgUp` / `PgDn` move the outline selection.
const OUTLINE_PAGE: i32 = 10;

#[derive(Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Normal,
    /// The search prompt, with the pattern typed so far.
    Search(String),
    /// The outline popup, with its filter.
    Outline(String),
}

#[derive(Debug, Default)]
pub struct Keymap {
    pending: Option<char>,
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
    pub fn map(&mut self, key: KeyEvent) -> Option<Msg> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        match &mut self.mode {
            Mode::Normal => self.normal(key),
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
            Mode::Outline(text) => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let by = |by| Some(Msg::OutlineMove { by });
                match key.code {
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
        }
    }

    fn normal(&mut self, key: KeyEvent) -> Option<Msg> {
        let pending = self.pending.take();
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let scroll = |by| Some(Msg::Scroll { by });
        match key.code {
            KeyCode::Char(c @ (']' | '[')) if !ctrl => {
                if pending == Some(c) {
                    let dir = if c == ']' { Dir::Next } else { Dir::Prev };
                    return Some(Msg::JumpHeading { dir });
                }
                self.pending = Some(c);
                None
            }
            KeyCode::Char('c') if ctrl => Some(Msg::Quit),
            KeyCode::Char('d') if ctrl => scroll(ScrollBy::HalfPage(Dir::Next)),
            KeyCode::Char('u') if ctrl => scroll(ScrollBy::HalfPage(Dir::Prev)),
            KeyCode::Char('f') if ctrl => scroll(ScrollBy::Page(Dir::Next)),
            KeyCode::Char('b') if ctrl => scroll(ScrollBy::Page(Dir::Prev)),
            KeyCode::Char('e') if ctrl => scroll(ScrollBy::Rows(1)),
            KeyCode::Char('y') if ctrl => scroll(ScrollBy::Rows(-1)),
            KeyCode::Char('o') if ctrl => Some(Msg::Back),
            _ if ctrl => None,
            KeyCode::Char('j') | KeyCode::Down => scroll(ScrollBy::Rows(1)),
            KeyCode::Char('k') | KeyCode::Up => scroll(ScrollBy::Rows(-1)),
            KeyCode::Char('d') => scroll(ScrollBy::HalfPage(Dir::Next)),
            KeyCode::Char('u') => scroll(ScrollBy::HalfPage(Dir::Prev)),
            KeyCode::Char(' ') | KeyCode::PageDown => scroll(ScrollBy::Page(Dir::Next)),
            KeyCode::Char('b') | KeyCode::PageUp => scroll(ScrollBy::Page(Dir::Prev)),
            KeyCode::Char('g') | KeyCode::Home => scroll(ScrollBy::Top),
            KeyCode::Char('G') | KeyCode::End => scroll(ScrollBy::Bottom),
            KeyCode::Char('t') => Some(Msg::ToggleTableMode),
            KeyCode::Char('w') => Some(Msg::ToggleCodeWrap),
            KeyCode::Left => Some(Msg::ScrollCodeX { by: -CODE_X_STEP }),
            KeyCode::Right => Some(Msg::ScrollCodeX { by: CODE_X_STEP }),
            KeyCode::Tab => Some(Msg::FocusLink { dir: Dir::Next }),
            KeyCode::BackTab => Some(Msg::FocusLink { dir: Dir::Prev }),
            KeyCode::Enter => Some(Msg::Follow),
            KeyCode::Backspace | KeyCode::Char('H') => Some(Msg::Back),
            KeyCode::Char('L') => Some(Msg::Forward),
            KeyCode::Char('o') => {
                self.mode = Mode::Outline(String::new());
                Some(Msg::OpenOutline)
            }
            KeyCode::Char('/') => {
                self.mode = Mode::Search(String::new());
                Some(Msg::SearchOpen)
            }
            KeyCode::Char('n') => Some(Msg::SearchNext { dir: Dir::Next }),
            KeyCode::Char('N') => Some(Msg::SearchNext { dir: Dir::Prev }),
            KeyCode::Char('q') | KeyCode::Esc => Some(Msg::Quit),
            _ => None,
        }
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
    }
}
