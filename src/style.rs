//! Colors, text modifiers and styles, in place of ratatui's (CS-4 of the clean-slate plan). They
//! keep ratatui 0.30's semantics: a style sets only what it names, patching one style over another
//! or over a cell keeps the rest, and a color parses from the same names.

use std::fmt;
use std::num::NonZeroU32;
use std::ops::{BitAnd, BitOr, BitOrAssign, Sub};
use std::str::FromStr;

/// A terminal color: the default, a 256-color index or RGB, packed as a grid's cell holds it so
/// that patching a style over a cell copies it as it is (TRM-05). The kind is in the top byte
/// (1 the default, 2 RGB, 3 an index) and the value below; no color is 0, which leaves
/// `Option<Color>` four bytes with `None` as 0. The 16 named colors are indexes 0 to 15.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color(NonZeroU32);

/// What a color is, to match on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The terminal's own foreground or background.
    Reset,
    Rgb(u8, u8, u8),
    Indexed(u8),
}

impl Color {
    pub const RESET: Color = Color::packed(1 << 24);
    pub const BLACK: Color = Color::indexed(0);
    pub const RED: Color = Color::indexed(1);
    pub const GREEN: Color = Color::indexed(2);
    pub const YELLOW: Color = Color::indexed(3);
    pub const BLUE: Color = Color::indexed(4);
    pub const MAGENTA: Color = Color::indexed(5);
    pub const CYAN: Color = Color::indexed(6);
    pub const GRAY: Color = Color::indexed(7);
    pub const DARK_GRAY: Color = Color::indexed(8);
    pub const LIGHT_RED: Color = Color::indexed(9);
    pub const LIGHT_GREEN: Color = Color::indexed(10);
    pub const LIGHT_YELLOW: Color = Color::indexed(11);
    pub const LIGHT_BLUE: Color = Color::indexed(12);
    pub const LIGHT_MAGENTA: Color = Color::indexed(13);
    pub const LIGHT_CYAN: Color = Color::indexed(14);
    pub const WHITE: Color = Color::indexed(15);

    const fn packed(bits: u32) -> Color {
        match NonZeroU32::new(bits) {
            Some(bits) => Color(bits),
            None => panic!("a color is never 0"),
        }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color::packed(2 << 24 | (r as u32) << 16 | (g as u32) << 8 | b as u32)
    }

    pub const fn indexed(i: u8) -> Color {
        Color::packed(3 << 24 | i as u32)
    }

    pub const fn kind(self) -> Kind {
        let [kind, r, g, b] = self.0.get().to_be_bytes();
        match kind {
            2 => Kind::Rgb(r, g, b),
            3 => Kind::Indexed(b),
            _ => Kind::Reset,
        }
    }

    /// The color as a cell holds it.
    pub(crate) const fn bits(self) -> u32 {
        self.0.get()
    }

    /// The color a cell holds, from [`Color::bits`].
    pub(crate) const fn from_bits(bits: u32) -> Color {
        match NonZeroU32::new(bits) {
            Some(bits) => Color(bits),
            None => Color::RESET,
        }
    }

    /// A style's color as patching packs it: 0 for none.
    pub(crate) const fn bits_of(c: Option<Color>) -> u32 {
        match c {
            Some(c) => c.0.get(),
            None => 0,
        }
    }
}

impl Default for Color {
    fn default() -> Color {
        Color::RESET
    }
}

impl fmt::Debug for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind().fmt(f)
    }
}

/// A color name that isn't one.
#[derive(Debug, PartialEq, Eq)]
pub struct ParseColorError;

impl fmt::Display for ParseColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a color")
    }
}

impl FromStr for Color {
    type Err = ParseColorError;

    /// A name in any case, spaces, dashes and underscores ignored, "bright" for "light", "grey"
    /// or "silver" for "gray"; an index (`"42"`); or `#rrggbb`. As ratatui parses them, so a
    /// theme reads as it did.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let name = s
            .to_lowercase()
            .replace([' ', '-', '_'], "")
            .replace("bright", "light")
            .replace("grey", "gray")
            .replace("silver", "gray")
            .replace("lightblack", "darkgray")
            .replace("lightwhite", "white")
            .replace("lightgray", "white");
        const NAMES: [&str; 16] = [
            "black",
            "red",
            "green",
            "yellow",
            "blue",
            "magenta",
            "cyan",
            "gray",
            "darkgray",
            "lightred",
            "lightgreen",
            "lightyellow",
            "lightblue",
            "lightmagenta",
            "lightcyan",
            "white",
        ];
        if name == "reset" {
            Ok(Color::RESET)
        } else if let Some(i) = NAMES.iter().position(|&n| n == name) {
            Ok(Color::indexed(i as u8))
        } else if let Ok(i) = s.parse::<u8>() {
            Ok(Color::indexed(i))
        } else {
            hex(s).ok_or(ParseColorError)
        }
    }
}

/// `#rrggbb`.
fn hex(s: &str) -> Option<Color> {
    let digits = s.strip_prefix('#').filter(|d| d.len() == 6)?;
    let at = |i: usize| u8::from_str_radix(digits.get(i..i + 2)?, 16).ok();
    Some(Color::rgb(at(0)?, at(2)?, at(4)?))
}

/// Text attributes as bits, numbered as ratatui numbers them.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Modifier(u16);

impl Modifier {
    pub const BOLD: Modifier = Modifier(1);
    pub const DIM: Modifier = Modifier(1 << 1);
    pub const ITALIC: Modifier = Modifier(1 << 2);
    pub const UNDERLINED: Modifier = Modifier(1 << 3);
    pub const SLOW_BLINK: Modifier = Modifier(1 << 4);
    pub const RAPID_BLINK: Modifier = Modifier(1 << 5);
    pub const REVERSED: Modifier = Modifier(1 << 6);
    pub const HIDDEN: Modifier = Modifier(1 << 7);
    pub const CROSSED_OUT: Modifier = Modifier(1 << 8);

    const NAMES: [(Modifier, &str); 9] = [
        (Modifier::BOLD, "BOLD"),
        (Modifier::DIM, "DIM"),
        (Modifier::ITALIC, "ITALIC"),
        (Modifier::UNDERLINED, "UNDERLINED"),
        (Modifier::SLOW_BLINK, "SLOW_BLINK"),
        (Modifier::RAPID_BLINK, "RAPID_BLINK"),
        (Modifier::REVERSED, "REVERSED"),
        (Modifier::HIDDEN, "HIDDEN"),
        (Modifier::CROSSED_OUT, "CROSSED_OUT"),
    ];

    pub const fn empty() -> Modifier {
        Modifier(0)
    }

    pub const fn all() -> Modifier {
        Modifier((1 << 9) - 1)
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    /// The modifiers among `bits`, the rest dropped.
    pub const fn from_bits_truncate(bits: u16) -> Modifier {
        Modifier(bits & Modifier::all().0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, other: Modifier) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Modifier) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn union(self, other: Modifier) -> Modifier {
        Modifier(self.0 | other.0)
    }

    pub const fn difference(self, other: Modifier) -> Modifier {
        Modifier(self.0 & !other.0)
    }

    pub fn insert(&mut self, other: Modifier) {
        self.0 |= other.0;
    }

    pub fn remove(&mut self, other: Modifier) {
        self.0 &= !other.0;
    }
}

impl fmt::Debug for Modifier {
    /// `NONE`, or the names joined by `|`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("NONE");
        }
        let mut names = Modifier::NAMES.iter().filter(|(m, _)| self.contains(*m));
        if let Some((_, first)) = names.next() {
            f.write_str(first)?;
        }
        names.try_for_each(|(_, name)| write!(f, " | {name}"))
    }
}

impl BitOr for Modifier {
    type Output = Modifier;
    fn bitor(self, other: Modifier) -> Modifier {
        self.union(other)
    }
}

impl BitOrAssign for Modifier {
    fn bitor_assign(&mut self, other: Modifier) {
        self.insert(other);
    }
}

impl BitAnd for Modifier {
    type Output = Modifier;
    fn bitand(self, other: Modifier) -> Modifier {
        Modifier(self.0 & other.0)
    }
}

impl Sub for Modifier {
    type Output = Modifier;
    /// The modifiers in `self` that aren't in `other`.
    fn sub(self, other: Modifier) -> Modifier {
        self.difference(other)
    }
}

/// What a style sets: a foreground, a background, an underline color, modifiers to add and
/// modifiers to take away. What it leaves `None` or empty stays as it was.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub underline_color: Option<Color>,
    pub add_modifier: Modifier,
    pub sub_modifier: Modifier,
}

/// `Style` methods that add or take away one modifier.
macro_rules! modifiers {
    ($($m:ident: $on:ident, $off:ident;)*) => {
        $(
            #[must_use]
            pub const fn $on(self) -> Style {
                self.add_modifier(Modifier::$m)
            }

            #[must_use]
            pub const fn $off(self) -> Style {
                self.remove_modifier(Modifier::$m)
            }
        )*
    };
}

impl Style {
    /// A style that sets nothing.
    pub const fn new() -> Style {
        Style {
            fg: None,
            bg: None,
            underline_color: None,
            add_modifier: Modifier::empty(),
            sub_modifier: Modifier::empty(),
        }
    }

    /// A style that sets everything back to the terminal's defaults.
    pub const fn reset() -> Style {
        Style {
            fg: Some(Color::RESET),
            bg: Some(Color::RESET),
            underline_color: Some(Color::RESET),
            add_modifier: Modifier::empty(),
            sub_modifier: Modifier::all(),
        }
    }

    #[must_use]
    pub const fn fg(mut self, color: Color) -> Style {
        self.fg = Some(color);
        self
    }

    #[must_use]
    pub const fn bg(mut self, color: Color) -> Style {
        self.bg = Some(color);
        self
    }

    #[must_use]
    pub const fn underline_color(mut self, color: Color) -> Style {
        self.underline_color = Some(color);
        self
    }

    #[must_use]
    pub const fn add_modifier(mut self, modifier: Modifier) -> Style {
        self.sub_modifier = self.sub_modifier.difference(modifier);
        self.add_modifier = self.add_modifier.union(modifier);
        self
    }

    #[must_use]
    pub const fn remove_modifier(mut self, modifier: Modifier) -> Style {
        self.add_modifier = self.add_modifier.difference(modifier);
        self.sub_modifier = self.sub_modifier.union(modifier);
        self
    }

    /// `other` over this style: what `other` sets wins, the rest stays.
    #[must_use]
    pub fn patch(mut self, other: Style) -> Style {
        self.fg = other.fg.or(self.fg);
        self.bg = other.bg.or(self.bg);
        self.underline_color = other.underline_color.or(self.underline_color);
        self.add_modifier.remove(other.sub_modifier);
        self.add_modifier.insert(other.add_modifier);
        self.sub_modifier.remove(other.add_modifier);
        self.sub_modifier.insert(other.sub_modifier);
        self
    }

    #[must_use]
    pub const fn red(self) -> Style {
        self.fg(Color::RED)
    }

    modifiers! {
        BOLD: bold, not_bold;
        DIM: dim, not_dim;
        ITALIC: italic, not_italic;
        UNDERLINED: underlined, not_underlined;
        REVERSED: reversed, not_reversed;
        CROSSED_OUT: crossed_out, not_crossed_out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_parse_as_ratatui_parses_them() {
        let cases = [
            ("Red", Ok(Color::RED)),
            ("light-blue", Ok(Color::LIGHT_BLUE)),
            ("bright white", Ok(Color::WHITE)),
            ("bright_black", Ok(Color::DARK_GRAY)),
            ("light gray", Ok(Color::WHITE)),
            ("grey", Ok(Color::GRAY)),
            ("silver", Ok(Color::GRAY)),
            ("dark_grey", Ok(Color::DARK_GRAY)),
            ("RESET", Ok(Color::RESET)),
            ("42", Ok(Color::indexed(42))),
            ("#0aFf10", Ok(Color::rgb(10, 255, 16))),
            ("256", Err(ParseColorError)),
            ("#abc", Err(ParseColorError)),
            ("#gg0000", Err(ParseColorError)),
            ("purple", Err(ParseColorError)),
        ];
        for (s, want) in cases {
            assert_eq!(s.parse::<Color>(), want, "{s:?}");
        }
    }

    #[test]
    fn colors_are_what_they_were_made_of() {
        let cases = [
            (Color::RESET, Kind::Reset),
            (Color::WHITE, Kind::Indexed(15)),
            (Color::indexed(200), Kind::Indexed(200)),
            (Color::rgb(9, 8, 7), Kind::Rgb(9, 8, 7)),
        ];
        for (c, kind) in cases {
            assert_eq!(c.kind(), kind);
            assert_eq!(Color::from_bits(c.bits()), c);
        }
        // no color is 0, so a style's unset one costs nothing
        assert_eq!(size_of::<Option<Color>>(), 4);
        assert_eq!(Color::bits_of(None), 0);
    }

    #[test]
    fn a_patch_keeps_what_it_doesnt_set() {
        let base = Style::new().fg(Color::RED).bg(Color::BLUE).bold().italic();
        let over = Style::new().bg(Color::GREEN).not_bold().underlined();
        let both = base.patch(over);
        assert_eq!((both.fg, both.bg), (Some(Color::RED), Some(Color::GREEN)));
        assert_eq!(both.add_modifier, Modifier::ITALIC | Modifier::UNDERLINED);
        assert_eq!(both.sub_modifier, Modifier::BOLD);
        // added after taken away, and the other way around
        assert_eq!(Style::new().not_dim().dim().sub_modifier, Modifier::empty());
        assert_eq!(Style::new().dim().not_dim().add_modifier, Modifier::empty());
        assert_eq!(Style::new().patch(Style::reset()), Style::reset());
    }

    #[test]
    fn modifiers_print_by_name() {
        assert_eq!(format!("{:?}", Modifier::empty()), "NONE");
        assert_eq!(
            format!("{:?}", Modifier::BOLD | Modifier::HIDDEN),
            "BOLD | HIDDEN"
        );
        assert_eq!(Modifier::from_bits_truncate(u16::MAX), Modifier::all());
    }
}
