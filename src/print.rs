//! `--print`, and output to a pipe (ADR-15): the same [`Layout`](crate::layout::Layout), written
//! as SGR-styled text, so preen can stand in for glow in pipes and fzf preview panes.

use std::fmt::Write as _;
use std::io::{self, Write};

use ratatui::style::{Color, Modifier, Style};

use crate::doc::Doc;
use crate::layout::{self, LayoutParams};
use crate::theme::Theme;
use crate::ui;

/// Render a document as ANSI text, one line per row.
pub fn render(doc: &Doc, theme: &Theme, params: LayoutParams) -> String {
    let layout = layout::layout(doc, &theme.glyphs, params);
    let mut out = String::with_capacity(layout.text.len() * 2);
    for row in &layout.rows {
        let mut current = None;
        for (text, span) in layout.spans(row) {
            let style = ui::span_style(theme, span);
            if current != Some(style) {
                sgr(style, &mut out);
                current = Some(style);
            }
            out.push_str(text);
        }
        if current.is_some() {
            out.push_str("\x1b[0m");
        }
        out.push('\n');
    }
    out
}

/// Print a document to stdout. A closed pipe (fzf moving on) is not an error.
pub fn run(doc: &Doc, theme: &Theme, params: LayoutParams) -> io::Result<()> {
    let out = render(doc, theme, params);
    let mut stdout = io::stdout().lock();
    match stdout
        .write_all(out.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result,
    }
}

fn sgr(style: Style, out: &mut String) {
    out.push_str("\x1b[0");
    let m = style.add_modifier;
    for (flag, code) in [
        (Modifier::BOLD, "1"),
        (Modifier::DIM, "2"),
        (Modifier::ITALIC, "3"),
        (Modifier::UNDERLINED, "4"),
        (Modifier::REVERSED, "7"),
        (Modifier::CROSSED_OUT, "9"),
    ] {
        if m.contains(flag) {
            out.push(';');
            out.push_str(code);
        }
    }
    if let Some(c) = style.fg {
        color(c, 30, out);
    }
    if let Some(c) = style.bg {
        color(c, 40, out);
    }
    if m.contains(Modifier::UNDERLINED)
        && let Some(Color::Rgb(r, g, b)) = style.underline_color
    {
        let _ = write!(out, ";58;2;{r};{g};{b}");
    }
    out.push('m');
}

fn color(c: Color, base: u8, out: &mut String) {
    let named = |n: u8| if n < 8 { base + n } else { base + 60 + n - 8 };
    let _ = match c {
        Color::Reset => Ok(()),
        Color::Rgb(r, g, b) => write!(out, ";{};2;{r};{g};{b}", base + 8),
        Color::Indexed(n) => write!(out, ";{};5;{n}", base + 8),
        Color::Black => write!(out, ";{}", named(0)),
        Color::Red => write!(out, ";{}", named(1)),
        Color::Green => write!(out, ";{}", named(2)),
        Color::Yellow => write!(out, ";{}", named(3)),
        Color::Blue => write!(out, ";{}", named(4)),
        Color::Magenta => write!(out, ";{}", named(5)),
        Color::Cyan => write!(out, ";{}", named(6)),
        Color::Gray => write!(out, ";{}", named(7)),
        Color::DarkGray => write!(out, ";{}", named(8)),
        Color::LightRed => write!(out, ";{}", named(9)),
        Color::LightGreen => write!(out, ";{}", named(10)),
        Color::LightYellow => write!(out, ";{}", named(11)),
        Color::LightBlue => write!(out, ";{}", named(12)),
        Color::LightMagenta => write!(out, ";{}", named(13)),
        Color::LightCyan => write!(out, ";{}", named(14)),
        Color::White => write!(out, ";{}", named(15)),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sgr_sequences() {
        let mut out = String::new();
        sgr(
            Style::new()
                .fg(Color::Rgb(1, 2, 3))
                .bg(Color::LightRed)
                .add_modifier(Modifier::BOLD | Modifier::ITALIC),
            &mut out,
        );
        assert_eq!(out, "\x1b[0;1;3;38;2;1;2;3;101m");
    }

    #[test]
    fn renders_one_line_per_row() {
        let doc = crate::doc::parse("# Title\n\nsome *text*\n");
        let params = LayoutParams {
            width: 40,
            max_width: None,
            table_mode: Default::default(),
            code_wrap: true,
            glyphs_gen: 0,
        };
        let out = render(&doc, &Theme::builtin(), params);
        assert_eq!(out.lines().count(), 3);
        assert!(out.contains("Title"));
    }
}
