//! The settings popup (`F2`, `,`): the options in groups, labels in the popup's muted text, and
//! values in a column of their own on an input background, so every value reads as something to
//! change. On/off options are switches, with the word beside them for terminals and eyes where
//! shape and color aren't enough; one of a few is a row of every choice, the current one filled.
//! At the right, dimmed, the key that changes the option outside the popup, or `restart` for one
//! read at start. A row whose value isn't the config file's has a mark. Clicks resolve here too,
//! against the same geometry ([`hit`]).

use crate::geom::Rect;
use crate::grid::Grid;
use crate::style::{Modifier, Style};
use unicode_width::UnicodeWidthStr;

use super::{elide_end, popup_frame, put};
use crate::app::{Hit, MIN_WIDTH, Row, State, Value};
use crate::theme::{StyleKey, Theme};

/// The widest label, "Keep screen on quit".
const LABEL: u16 = 19;
/// The widest value: a field around every image protocol.
const VALUE: u16 = 44;
/// The key column: `restart` at most.
const KEYS: u16 = 7;
/// The prose width's field, without its padding.
const NUMBER: usize = 8;
const MARK: &str = "•";
const RESTART: &str = "restart";

/// The ways out, as the footer names them: the key, and what it does, long and short.
const FOOTER: [(&str, &str, &str, Hit); 3] = [
    ("a", "keep for this session", "keep", Hit::Keep),
    ("s", "save to config", "save", Hit::Save),
    ("esc", "undo", "undo", Hit::Cancel),
];

/// A line of the popup's body: a group's heading, or a row, by its index in [`Row::ALL`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Line {
    Group(&'static str),
    Row(usize),
}

fn lines() -> Vec<Line> {
    let mut lines = Vec::new();
    for (i, row) in Row::ALL.iter().enumerate() {
        if i == 0 || Row::ALL[i - 1].group() != row.group() {
            lines.push(Line::Group(row.group()));
        }
        lines.push(Line::Row(i));
    }
    lines
}

/// Where the popup goes, and how its columns divide its width.
struct Frame {
    rect: Rect,
    lines: Vec<Line>,
    /// The first line of the body shown, and how many fit.
    first: usize,
    shown: usize,
    /// The value column: from the inner left, and how wide.
    value_x: u16,
    value_w: u16,
    /// Whether the keys column fits.
    keys: bool,
}

impl Frame {
    fn inner(&self) -> u16 {
        self.rect.width - 2
    }

    /// The screen row of the body's line `i`, among those shown.
    fn y(&self, i: usize) -> Option<u16> {
        let at = i.checked_sub(self.first).filter(|&at| at < self.shown)?;
        Some(self.rect.y + 1 + at as u16)
    }

    fn footer_y(&self) -> u16 {
        self.rect.bottom() - 2
    }
}

fn frame(state: &State, area: Rect) -> Option<Frame> {
    let popup = state.settings.as_ref()?;
    let lines = lines();
    let natural = 3 + LABEL + 1 + VALUE + 2 + KEYS + 1;
    let width = (natural + 2).min(area.width.saturating_sub(2));
    // the body, a border above and below, the footer and the rule over it
    let height = (lines.len() as u16 + 4).min(area.height.saturating_sub(2));
    if width < 24 || height < 6 {
        return None;
    }
    let inner = width - 2;
    let label = LABEL.min(inner * 2 / 5);
    let value_x = 3 + label + 1;
    let keys = inner >= value_x + 24 + KEYS + 2;
    let value_w = inner - value_x - if keys { KEYS + 2 } else { 1 };
    // the focused row in view, its heading too when it can be
    let shown = usize::from(height - 4);
    let at = lines
        .iter()
        .position(|&l| l == Line::Row(popup.row))
        .unwrap_or(0);
    let first = at
        .saturating_sub(shown.saturating_sub(1) / 2)
        .min(lines.len() - shown.min(lines.len()));
    Some(Frame {
        rect: Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + 1,
            width,
            height,
        },
        lines,
        first,
        shown,
        value_x,
        value_w,
        keys,
    })
}

/// A row's value, in pieces: the text, its style, and which choice it is, if any.
type Pieces = Vec<(String, Style, Option<usize>)>;

/// `row`'s value within `width` columns.
fn value(state: &State, row: Row, width: u16, focused: bool) -> Pieces {
    let theme = &state.theme;
    let field = theme.style(StyleKey::SettingsField);
    let (on, off) = (
        theme.style(StyleKey::SettingsSwitchOn),
        theme.style(StyleKey::SettingsSwitchOff),
    );
    let enabled = state.setting_enabled(row);
    let width = usize::from(width);
    match state.setting(row) {
        Value::Switch(true) => vec![
            (" ".into(), field, None),
            ("━━●".into(), on, None),
            (" on  ".into(), on, None),
        ],
        Value::Switch(false) => vec![
            (" ".into(), field, None),
            ("●━━".into(), off, None),
            (" off ".into(), off, None),
        ],
        Value::Choice(names, current) => {
            let picked = if enabled {
                theme.style(StyleKey::SettingsChoice)
            } else {
                off.add_modifier(Modifier::REVERSED)
            };
            let (plain, sep) = (if enabled { field } else { off }, off);
            let all: usize = names.iter().map(|n| n.width() + 3).sum::<usize>() - 1;
            if all <= width {
                let mut pieces = Vec::new();
                for (i, name) in names.iter().enumerate() {
                    if i > 0 {
                        pieces.push(("│".into(), sep, None));
                    }
                    let style = if i == current { picked } else { plain };
                    pieces.push((format!(" {name} "), style, Some(i)));
                }
                pieces
            } else {
                // too narrow for every choice: the current one, between arrows to the others
                let n = names.len();
                let name = elide_end(names[current], width.saturating_sub(4));
                vec![
                    ("‹".into(), sep, Some((current + n - 1) % n)),
                    (format!(" {name} "), picked, Some(current)),
                    ("›".into(), sep, Some((current + 1) % n)),
                ]
            }
        }
        Value::Width(typed) => {
            let label = theme.style(StyleKey::SettingsLabel);
            let cursor = if focused { "▏" } else { " " };
            let (text, style) = match typed {
                "" => ("terminal", off),
                typed => (typed, field),
            };
            let mut pieces = vec![
                (" ".into(), field, None),
                (text.into(), style, None),
                (
                    format!(
                        "{cursor}{:pad$} ",
                        "",
                        pad = NUMBER.saturating_sub(text.width())
                    ),
                    field,
                    None,
                ),
            ];
            let short = typed.parse::<u16>().is_ok_and(|w| w < MIN_WIDTH);
            if short {
                let hint = format!(" at least {MIN_WIDTH}");
                pieces.push((hint, label, None));
            } else if !typed.is_empty() {
                pieces.push((" columns".into(), label, None));
            }
            pieces
        }
        Value::Theme(name) => {
            let name = elide_end(name, width.saturating_sub(4));
            vec![(format!(" {name} ▸ "), field, None)]
        }
    }
}

/// The footer's pieces within `inner` columns, each with what a click on it does.
fn footer(theme: &Theme, inner: u16) -> Vec<(String, Style, Option<Hit>)> {
    let (key, text) = (
        theme.style(StyleKey::PopupTitle),
        theme.style(StyleKey::SettingsLabel),
    );
    let long: usize = FOOTER
        .iter()
        .map(|(k, what, ..)| k.width() + what.width() + 4)
        .sum();
    let mut pieces = Vec::new();
    for (k, what, short, hit) in FOOTER {
        let what = if long <= usize::from(inner) {
            what
        } else {
            short
        };
        pieces.push((format!("  {k}"), key, Some(hit)));
        pieces.push((format!(" {what}"), text, Some(hit)));
    }
    pieces
}

pub fn draw(buf: &mut Grid, area: Rect, state: &State) {
    let (Some(popup), Some(frame)) = (&state.settings, frame(state, area)) else {
        return;
    };
    let theme = &state.theme;
    let rect = frame.rect;
    let border = popup_frame(buf, rect, theme);
    let inner = frame.inner();
    let x = rect.x + 1;
    buf.set_stringn(
        rect.x + 2,
        rect.y,
        " settings ",
        usize::from(inner - 1),
        theme.style(StyleKey::PopupTitle),
    );

    let (label, focus) = (
        theme.style(StyleKey::SettingsLabel),
        theme.style(StyleKey::SettingsFocus),
    );
    for (i, &line) in frame.lines.iter().enumerate() {
        let Some(y) = frame.y(i) else {
            continue;
        };
        let row = match line {
            Line::Group(name) => {
                let title = theme.style(StyleKey::PopupTitle);
                buf.set_stringn(x + 1, y, name, usize::from(inner - 1), title);
                continue;
            }
            Line::Row(row) => row,
        };
        let setting = Row::ALL[row];
        let focused = row == popup.row;
        let base = if focused {
            buf.set_style(Rect::new(x, y, inner, 1), focus);
            focus
        } else {
            theme.style(StyleKey::Popup)
        };
        if state.setting_changed(setting) {
            let changed = base.patch(theme.styles[StyleKey::SettingsChanged as usize]);
            buf.set_stringn(x + 1, y, MARK, 1, changed);
        }
        let label_style = if focused { focus } else { label };
        let room = usize::from(frame.value_x.saturating_sub(4));
        buf.set_stringn(x + 3, y, setting.label(), room, label_style);
        let pieces: Vec<_> = value(state, setting, frame.value_w, focused)
            .into_iter()
            .map(|(text, style, _)| (text, style))
            .collect();
        put(
            buf,
            x + frame.value_x,
            y,
            usize::from(frame.value_w),
            &pieces,
        );
        if frame.keys {
            let key = if setting.at_start() {
                Some(RESTART.to_owned())
            } else {
                setting
                    .action()
                    .and_then(|a| state.config.keys.keys(a).first())
                    .map(ToString::to_string)
            };
            if let Some(key) = key {
                let key = elide_end(&key, usize::from(KEYS));
                let dim = base.patch(Style {
                    fg: label.fg,
                    ..Style::new()
                });
                let at = x + inner - 1 - key.width() as u16;
                buf.set_stringn(at, y, &key, usize::from(KEYS), dim);
            }
        }
    }

    // the rule over the footer, then the ways out, centered
    let y = frame.footer_y();
    let rule = format!("├{}┤", "─".repeat(usize::from(inner)));
    buf.set_string(rect.x, y - 1, rule, border);
    let pieces: super::Pieces = footer(theme, inner)
        .into_iter()
        .map(|(text, style, _)| (text, style))
        .collect();
    let used: usize = pieces.iter().map(|(t, _)| t.width()).sum();
    let pad = usize::from(inner).saturating_sub(used + 2) / 2;
    put(buf, x + pad as u16, y, usize::from(inner), &pieces);
}

/// What the screen's cell `col`, `row` shows of the popup.
pub fn hit(state: &State, col: u16, row: u16) -> Option<Hit> {
    let popup = state.settings.as_ref()?;
    let area = Rect::new(0, 0, state.size.w, state.size.h.saturating_sub(1));
    let frame = frame(state, area)?;
    let (rect, inner) = (frame.rect, frame.inner());
    let x = rect.x + 1;
    if col < x || col >= x + inner {
        return None;
    }
    if row == frame.footer_y() {
        let pieces = footer(&state.theme, inner);
        let used: usize = pieces.iter().map(|(t, ..)| t.width()).sum();
        let mut at = x + (usize::from(inner).saturating_sub(used + 2) / 2) as u16;
        for (text, _, hit) in pieces {
            let end = at + text.width() as u16;
            // the leading blanks are no one's
            let start = at + (text.width() - text.trim_start().width()) as u16;
            if (start..end).contains(&col) {
                return hit;
            }
            at = end;
        }
        return None;
    }
    let i = (0..frame.lines.len()).find(|&i| frame.y(i) == Some(row))?;
    let Line::Row(i) = frame.lines[i] else {
        return None;
    };
    let setting = Row::ALL[i];
    let mut at = x + frame.value_x;
    let pieces = value(state, setting, frame.value_w, i == popup.row);
    let field_end = match state.setting(setting) {
        // the field's own pieces, not the words after it
        Value::Width(_) => 3,
        _ => pieces.len(),
    };
    for (n, (text, _, choice)) in pieces.into_iter().enumerate() {
        let end = at + text.width() as u16;
        if (at..end).contains(&col) && n < field_end {
            return Some(match (choice, state.setting(setting)) {
                (Some(choice), _) => Hit::Choice(i, choice),
                (None, Value::Choice(..)) => Hit::Row(i),
                (None, _) => Hit::Value(i),
            });
        }
        at = end;
    }
    Some(Hit::Row(i))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Computed, Config, Msg, Size, update};

    /// The popup on a 90×26 screen in `theme`, its theme background as `bg` says, with a mark,
    /// a switch of each kind and the focus on the tables.
    fn shown(theme: Theme, bg: bool) -> (State, Grid) {
        let config = Config {
            file: Some("c.toml".into()),
            theme_bg: bg,
            ..Config::default()
        };
        let file = format!("theme = \"{}\"\ncode_wrap = false\n", theme.name);
        let mut state = State::new(theme, config);
        for msg in [
            Msg::Init {
                size: Size { w: 90, h: 26 },
            },
            Msg::DocLoaded {
                path: "t.md".into(),
                mtime: None,
                text: "# Behind\n\nthe document\n".into(),
                doc: Computed(None),
            },
            Msg::OpenSettings,
            Msg::ConfigRead {
                path: "c.toml".into(),
                text: file.into(),
            },
            Msg::SettingsMove { by: 3 },
        ] {
            update(&mut state, msg);
        }
        let mut grid = Grid::empty(Rect::new(0, 0, 90, 26));
        super::super::view(&state, &mut grid);
        (state, grid)
    }

    /// Which slot styles each cell of the popup, a letter each: what every theme must agree on.
    fn slots(state: &State, grid: &Grid) -> String {
        let theme = &state.theme;
        let style = |k| theme.style(k);
        let changed = |base: Style| base.patch(theme.styles[StyleKey::SettingsChanged as usize]);
        let dim = |base: Style, fg| base.patch(Style { fg, ..Style::new() });
        let label_fg = style(StyleKey::SettingsLabel).fg;
        let popup = style(StyleKey::Popup);
        let focus = style(StyleKey::SettingsFocus);
        let known = [
            ('f', style(StyleKey::SettingsField)),
            ('O', style(StyleKey::SettingsSwitchOn)),
            ('o', style(StyleKey::SettingsSwitchOff)),
            ('C', style(StyleKey::SettingsChoice)),
            ('M', changed(popup)),
            ('m', changed(focus)),
            ('F', focus),
            ('k', dim(focus, label_fg)),
            ('L', style(StyleKey::SettingsLabel)),
            ('T', style(StyleKey::PopupTitle)),
            ('B', style(StyleKey::PopupBorder)),
            ('.', popup),
        ];
        let frame = frame(state, grid.area()).unwrap().rect;
        let mut out = String::new();
        for y in frame.top()..frame.bottom() {
            for x in frame.left()..frame.right() {
                let cell = &grid[(x, y)];
                let letter = known
                    .iter()
                    .find(|(_, s)| cell.fg() == s.fg.unwrap() && cell.bg() == s.bg.unwrap())
                    .map_or('?', |&(c, _)| c);
                out.push(letter);
            }
            out.push('\n');
        }
        out
    }

    /// The default theme's map, with and without its background, is a snapshot; every other
    /// built-in, dark and light, must draw the same.
    #[test]
    fn every_theme_styles_the_popup_alike() {
        for bg in [true, false] {
            let (state, grid) = shown(Theme::builtin(), bg);
            let want = slots(&state, &grid);
            insta::assert_snapshot!(format!("slots-theme-bg-{bg}"), want);
            assert!(!want.contains('?'), "{want}");
            // but for the theme row, as long as the theme's name
            let others = |map: &str| {
                let mut lines: Vec<&str> = map.lines().collect();
                lines.remove(2);
                lines.join("\n")
            };
            for theme in Theme::builtins() {
                let name = format!("{} (theme_bg = {bg})", theme.name);
                let (state, grid) = shown(theme, bg);
                assert_eq!(others(&slots(&state, &grid)), others(&want), "{name}");
            }
        }
    }

    #[test]
    fn a_short_screen_scrolls_the_rows_to_the_focus() {
        let (mut state, _) = shown(Theme::builtin(), true);
        update(
            &mut state,
            Msg::Resize {
                size: Size { w: 50, h: 12 },
            },
        );
        update(&mut state, Msg::SettingsMove { by: 99 });
        let mut grid = Grid::empty(Rect::new(0, 0, 50, 12));
        super::super::view(&state, &mut grid);
        let text = crate::record::frame_text(&grid);
        assert!(text.contains("Keep screen"), "{text}");
        assert!(!text.contains("Appearance"), "{text}");
        assert!(text.contains("esc"), "the footer stays: {text}");
        // and a choice that doesn't fit shows between arrows
        update(&mut state, Msg::SettingsMove { by: -3 });
        super::super::view(&state, &mut grid);
        let text = crate::record::frame_text(&grid);
        assert!(text.contains("‹ auto ›"), "{text}");
        // a click on an arrow takes the choice it points at
        let (y, line) = text
            .lines()
            .enumerate()
            .find(|(_, l)| l.contains("‹ auto ›"))
            .unwrap();
        let x = line[..line.find('›').unwrap()].chars().count() as u16;
        update(
            &mut state,
            Msg::SettingsClick {
                col: x,
                row: y as u16,
            },
        );
        assert_eq!(
            state.next_start.images,
            crate::images::ImageMode::Force(crate::images::Proto::Kitty)
        );
    }
}
