//! A line of text being typed, with a cursor (FEAT-018): what the search prompt and the lists'
//! filters share. The cursor is a byte offset on a grapheme boundary, so `←`/`→`, `Backspace` and
//! `Del` take a character as it is drawn: an emoji or a letter with its accents goes whole.

use std::mem;

use unicode_segmentation::UnicodeSegmentation;

/// Where the cursor stands in a message's text, as a byte offset. A recording from before the
/// cursor leaves it out, which puts it at the end, where it always was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor(pub u32);

impl Cursor {
    pub const END: Cursor = Cursor(u32::MAX);
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Input {
    text: String,
    cursor: usize,
}

impl Input {
    /// `text` with the cursor at `cursor`, or at the end when that is past it; inside a
    /// character, it moves back to the character's start.
    pub fn new(text: String, cursor: Cursor) -> Self {
        let mut at = (cursor.0 as usize).min(text.len());
        while !text.is_char_boundary(at) {
            at -= 1;
        }
        Self { text, cursor: at }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> Cursor {
        Cursor(u32::try_from(self.cursor).unwrap_or(u32::MAX))
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The text before the cursor, and from it on.
    pub fn split(&self) -> (&str, &str) {
        self.text.split_at(self.cursor)
    }

    /// Replace the text, the cursor at its end.
    pub fn set(&mut self, text: &str) {
        text.clone_into(&mut self.text);
        self.cursor = self.text.len();
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    pub fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Where the character before the cursor starts.
    fn prev(&self) -> Option<usize> {
        self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map(|(i, _)| i)
    }

    /// Where the character after the cursor ends.
    fn next(&self) -> Option<usize> {
        self.text[self.cursor..]
            .graphemes(true)
            .next()
            .map(|g| self.cursor + g.len())
    }

    /// Each of these returns whether anything changed.
    pub fn left(&mut self) -> bool {
        self.prev().map(|i| self.cursor = i).is_some()
    }

    pub fn right(&mut self) -> bool {
        self.next().map(|i| self.cursor = i).is_some()
    }

    pub fn home(&mut self) -> bool {
        mem::replace(&mut self.cursor, 0) != 0
    }

    pub fn end(&mut self) -> bool {
        let end = self.text.len();
        mem::replace(&mut self.cursor, end) != end
    }

    /// `Backspace`: the character before the cursor.
    pub fn backspace(&mut self) -> bool {
        let Some(start) = self.prev() else {
            return false;
        };
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
        true
    }

    /// `Del`: the character under the cursor; nothing at the end.
    pub fn delete(&mut self) -> bool {
        let Some(end) = self.next() else {
            return false;
        };
        self.text.replace_range(self.cursor..end, "");
        true
    }

    /// `Ctrl-U`: everything before the cursor, as readline does.
    pub fn kill_start(&mut self) -> bool {
        let changed = self.cursor > 0;
        self.text.replace_range(..self.cursor, "");
        self.cursor = 0;
        changed
    }

    /// `Ctrl-W`: the word before the cursor, and the blanks after it.
    pub fn kill_word(&mut self) -> bool {
        let before = &self.text[..self.cursor];
        let start = before
            .trim_end()
            .rfind(char::is_whitespace)
            .map_or(0, |i| i + 1);
        let changed = start < self.cursor;
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text` with the cursor at the `|`.
    fn at(text: &str) -> Input {
        let cursor = text.find('|').expect("a cursor");
        Input::new(text.replacen('|', "", 1), Cursor(cursor as u32))
    }

    fn show(input: &Input) -> String {
        let (a, b) = input.split();
        format!("{a}|{b}")
    }

    #[test]
    fn typing_inserts_at_the_cursor() {
        let mut input = at("|");
        input.insert('a');
        input.insert('c');
        assert_eq!(show(&input), "ac|");
        input.left();
        input.insert('b');
        assert_eq!(show(&input), "ab|c");
        input.home();
        input.insert('_');
        assert_eq!(show(&input), "_|abc");
    }

    #[test]
    fn moves_stop_at_either_end() {
        let mut input = at("a|b");
        assert!(input.left());
        assert!(!input.left());
        assert_eq!(show(&input), "|ab");
        assert!(input.end());
        assert!(!input.end());
        assert!(!input.right());
        assert!(input.home());
        assert!(!input.home());
    }

    #[test]
    fn backspace_and_delete_take_one_character() {
        let mut input = at("ab|cd");
        assert!(input.backspace());
        assert_eq!(show(&input), "a|cd");
        assert!(input.delete());
        assert_eq!(show(&input), "a|d");

        let mut input = at("|ab");
        assert!(!input.backspace(), "nothing before the start");
        let mut input = at("ab|");
        assert!(!input.delete(), "nothing under the end");
        assert_eq!(show(&input), "ab|");
    }

    #[test]
    fn wide_and_combining_characters_go_whole() {
        // `e` and a combining acute, then a wide CJK character, then a flag
        let mut input = at("e\u{301}漢🇸🇪|");
        input.left();
        assert_eq!(show(&input), "e\u{301}漢|🇸🇪");
        input.left();
        input.left();
        assert_eq!(show(&input), "|e\u{301}漢🇸🇪");
        assert!(input.delete());
        assert_eq!(show(&input), "|漢🇸🇪");
        input.end();
        assert!(input.backspace());
        assert_eq!(show(&input), "漢|");
    }

    #[test]
    fn a_cursor_out_of_bounds_lands_inside() {
        assert_eq!(show(&Input::new("ab".into(), Cursor::END)), "ab|");
        // inside `é`'s two bytes: to its start
        assert_eq!(show(&Input::new("aé".into(), Cursor(2))), "a|é");
    }

    #[test]
    fn ctrl_u_and_ctrl_w_delete_before_the_cursor() {
        let mut input = at("foo bar|baz");
        assert!(input.kill_word());
        assert_eq!(show(&input), "foo |baz");
        assert!(input.kill_word(), "the blank, then the word before it");
        assert_eq!(show(&input), "|baz");
        assert!(!input.kill_word());

        let mut input = at("foo  |bar");
        input.kill_word();
        assert_eq!(show(&input), "|bar");

        let mut input = at("foo|bar");
        assert!(input.kill_start());
        assert_eq!(show(&input), "|bar");
        assert!(!input.kill_start());
    }

    #[test]
    fn set_puts_the_cursor_at_the_end() {
        let mut input = at("x|y");
        input.set("hello");
        assert_eq!(show(&input), "hello|");
        input.clear();
        assert_eq!(show(&input), "|");
    }
}
