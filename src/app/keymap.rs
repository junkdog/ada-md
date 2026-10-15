//! Keys → semantic messages (plan §6). Pure: the only state is a pending prefix key for the
//! two-key bindings `]]` and `[[`. Recordings log the messages, not the keys, so they survive
//! keymap changes.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::{Dir, Msg, ScrollBy};

#[derive(Debug, Default)]
pub struct Keymap {
    pending: Option<char>,
}

impl Keymap {
    pub fn map(&mut self, key: KeyEvent) -> Option<Msg> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
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
            _ if ctrl => None,
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Enter => scroll(ScrollBy::Rows(1)),
            KeyCode::Char('k') | KeyCode::Up => scroll(ScrollBy::Rows(-1)),
            KeyCode::Char('d') => scroll(ScrollBy::HalfPage(Dir::Next)),
            KeyCode::Char('u') => scroll(ScrollBy::HalfPage(Dir::Prev)),
            KeyCode::Char(' ') | KeyCode::PageDown => scroll(ScrollBy::Page(Dir::Next)),
            KeyCode::Char('b') | KeyCode::PageUp => scroll(ScrollBy::Page(Dir::Prev)),
            KeyCode::Char('g') | KeyCode::Home => scroll(ScrollBy::Top),
            KeyCode::Char('G') | KeyCode::End => scroll(ScrollBy::Bottom),
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
}
