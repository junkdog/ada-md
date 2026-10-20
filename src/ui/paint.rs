//! Rows → styled pieces, shared by the viewport and `--print`. Code rows are where paint does
//! real work: syntax colors are overlaid on the code span, and in no-wrap mode the span is a
//! window into its line, shifted by the shared offset (ADR-20).

use crate::geom::Rect;
use crate::grid::Grid;
use crate::style::Style;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::LinkState;
use crate::highlight::{Highlights, Run};
use crate::layout::{Layout, Row, RowKind, Span};
use crate::theme::{StyleKey, Theme};

/// A laid-out span's style.
pub fn span_style(theme: &Theme, span: &Span) -> Style {
    // Images and footnote references have their own link-like styles.
    let link = span.link.is_some() && !matches!(span.key, StyleKey::Image | StyleKey::FootnoteRef);
    theme.span_style(span.key, span.inline, link)
}

/// Everything paint reads besides the rows themselves.
pub struct Painter<'a> {
    pub theme: &'a Theme,
    pub layout: &'a Layout,
    pub highlights: &'a Highlights,
    /// The shared horizontal offset of code in no-wrap mode.
    pub code_x: u16,
    /// Where each link leads, so broken ones look it; empty when unknown.
    pub links: &'a [LinkState],
}

impl Painter<'_> {
    /// Paint rows starting at `scroll` into `area`.
    pub fn rows(&self, buf: &mut Grid, area: Rect, scroll: u32) {
        let rows = self.layout.rows.iter().skip(scroll as usize);
        for (y, row) in (area.y..area.bottom()).zip(rows) {
            self.row_into(buf, area.x, y, area.right(), row);
        }
    }

    /// Paint row `row` on line `y`, from `x` up to `right`.
    pub fn row_into(&self, buf: &mut Grid, mut x: u16, y: u16, right: u16, row: &Row) {
        self.row(row, &mut |text, style| {
            if x < right {
                (x, _) = buf.set_stringn(x, y, text, usize::from(right - x), style);
            }
        });
    }

    /// Row `row`'s line number, right-aligned in the [`Layout::numbers`] columns with a space
    /// after it, or blanks.
    pub fn number(&self, row: u32, emit: &mut impl FnMut(&str, Style)) {
        let width = usize::from(self.layout.numbers);
        match self.layout.line_number(row) {
            _ if width == 0 => {}
            Some(n) => {
                let style = self.theme.style(StyleKey::LineNumber);
                emit(&format!("{n:>w$} ", w = width - 1), style);
            }
            None => spaces(width, self.theme.style(StyleKey::Text), emit),
        }
    }

    /// A row's styled pieces, left to right. Their widths add up to the row's.
    pub fn row(&self, row: &Row, emit: &mut impl FnMut(&str, Style)) {
        let RowKind::Code { block, line, from } = row.kind else {
            let Some(stripe) = &row.stripe else {
                for (text, span) in self.layout.spans(row) {
                    emit(text, self.style(span));
                }
                return;
            };
            let bg = self.theme.styles[StyleKey::TableRowAlt as usize];
            let plain = self.theme.styles[StyleKey::Text as usize].bg;
            let mut col = 0;
            for (text, span) in self.layout.spans(row) {
                let style = self.style(span);
                let width = span.width;
                // Spans with a background of their own, like inline code, keep it.
                if col >= stripe.end || col + width <= stripe.start || style.bg != plain {
                    emit(text, style);
                } else {
                    stripe_piece(text, col, stripe, style, bg, emit);
                }
                col += width;
            }
            return;
        };
        let info = self.layout.code.get(block as usize).copied();
        let runs = info
            .and_then(|i| i.key)
            .and_then(|k| self.highlights.get(k))
            .map_or(&[][..], |s| s.line(line));
        let spans: Vec<_> = self.layout.spans(row).collect();
        let code = spans.iter().position(|(_, s)| s.key == StyleKey::CodeText);

        let wrap = self.layout.params.code_wrap;
        let mut x = 0;
        let mut clipped = [false; 2];
        if let (false, Some(i), Some(info)) = (wrap, code, info) {
            x = self.code_x.min(info.max_x());
            let width = spans[i].0.width();
            clipped = [
                x > 0 && width > 0,
                width > usize::from(x) + usize::from(info.window),
            ];
        }

        let g = &self.theme.glyphs;
        for (i, (text, span)) in spans.iter().enumerate() {
            let style = span_style(self.theme, span);
            if Some(i) == code {
                if wrap {
                    overlay(text, from, runs, style, self.theme, emit);
                } else {
                    let window = Window {
                        x,
                        width: span.width,
                    };
                    window.paint(text, runs, style, self.theme, emit);
                }
            } else if clipped[0] && Some(i + 1) == code {
                self.mark(&g.code_clip[0], span.width, style, emit);
            } else if clipped[1] && code.is_some_and(|c| c + 1 == i) {
                self.mark(&g.code_clip[1], span.width, style, emit);
            } else {
                emit(text, style);
            }
        }
    }

    /// A span's style, with broken links styled as such.
    fn style(&self, span: &Span) -> Style {
        let broken = span
            .link
            .and_then(|l| self.links.get(l as usize))
            .is_some_and(|s| matches!(s, LinkState::Broken(_)));
        if broken {
            let t = self.theme;
            t.span_style(span.key, span.inline, false)
                .patch(t.styles[StyleKey::LinkBroken as usize])
        } else {
            span_style(self.theme, span)
        }
    }

    /// A clip mark, padded to the `width` of the span it replaces.
    fn mark(&self, glyph: &str, width: u16, pad: Style, emit: &mut impl FnMut(&str, Style)) {
        emit(glyph, self.theme.style(StyleKey::CodeWrapMark));
        spaces(usize::from(width).saturating_sub(glyph.width()), pad, emit);
    }
}

/// Emit `text`, which starts at column `col`, with `bg` patched over the part in `stripe`.
fn stripe_piece(
    text: &str,
    col: u16,
    stripe: &std::ops::Range<u16>,
    style: Style,
    bg: Style,
    emit: &mut impl FnMut(&str, Style),
) {
    let mut start = 0;
    let mut inside = col >= stripe.start && col < stripe.end;
    let mut c = col;
    for (i, ch) in text.char_indices() {
        let now = c >= stripe.start && c < stripe.end;
        if now != inside {
            let piece = &text[start..i];
            emit(piece, if inside { style.patch(bg) } else { style });
            start = i;
            inside = now;
        }
        c += ch.width().unwrap_or(0) as u16;
    }
    emit(&text[start..], if inside { style.patch(bg) } else { style });
}

fn spaces(n: usize, style: Style, emit: &mut impl FnMut(&str, Style)) {
    if n > 0 {
        emit(&" ".repeat(n), style);
    }
}

/// Emit `text`, which starts at byte `from` of its line, with the line's syntax runs on top.
fn overlay(
    text: &str,
    from: u32,
    runs: &[Run],
    base: Style,
    theme: &Theme,
    emit: &mut impl FnMut(&str, Style),
) {
    let from = from as usize;
    let end = from + text.len();
    let mut pos = from;
    // a row of a long line starts deep in its runs (O-48)
    let first = runs.partition_point(|r| r.end as usize <= from);
    let mut run_start = first.checked_sub(1).map_or(0, |i| runs[i].end as usize);
    for run in &runs[first..] {
        let (a, b) = (run_start.max(pos), (run.end as usize).min(end));
        run_start = run.end as usize;
        if a >= b {
            if run_start >= end {
                break;
            }
            continue;
        }
        let (Some(gap), Some(piece)) =
            (text.get(pos - from..a - from), text.get(a - from..b - from))
        else {
            break;
        };
        if !gap.is_empty() {
            emit(gap, base);
        }
        let style = run
            .rule
            .style(&theme.syntax)
            .map_or(base, |s| base.patch(s));
        emit(piece, style);
        pos = b;
    }
    if pos < end {
        emit(&text[pos - from..], base);
    }
}

/// The columns of a no-wrap code line that are on screen.
struct Window {
    x: u16,
    width: u16,
}

impl Window {
    /// Paint the visible part of `line`, padded to the window's width. A wide character cut by
    /// either edge shows as spaces.
    fn paint(
        &self,
        line: &str,
        runs: &[Run],
        base: Style,
        theme: &Theme,
        emit: &mut impl FnMut(&str, Style),
    ) {
        let (x, width) = (usize::from(self.x), usize::from(self.width));
        let mut col = 0;
        let mut start = None;
        let mut end = line.len();
        let mut cut = 0;
        for (i, c) in line.char_indices() {
            let cw = c.width().unwrap_or(0);
            if start.is_none() {
                if col < x {
                    cut = (col + cw).saturating_sub(x);
                    col += cw;
                    continue;
                }
                start = Some(i);
            }
            if col + cw > x + width {
                end = i;
                break;
            }
            col += cw;
        }
        let start = start.unwrap_or(line.len());
        let cut = cut.min(width);
        spaces(cut, base, emit);
        let shown = &line[start..end];
        overlay(shown, start as u32, runs, base, theme, emit);
        spaces(width.saturating_sub(cut + shown.width()), base, emit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::Rule;

    fn overlaid(text: &str, from: u32, runs: &[Run]) -> Vec<String> {
        let mut out = Vec::new();
        let theme = Theme::builtin();
        overlay(text, from, runs, Style::new(), &theme, &mut |t: &str, _| {
            out.push(t.to_owned());
        });
        out
    }

    #[test]
    fn stripes_split_pieces_at_their_edges() {
        let bg = Style::new().bg(crate::style::Color::Red);
        let mut out = Vec::new();
        stripe_piece(
            "ab日cd",
            3,
            &(4..7),
            Style::new(),
            bg,
            &mut |t: &str, s| {
                out.push((t.to_owned(), s == bg));
            },
        );
        // columns: a=3 b=4 日=5-6 c=7 d=8
        let expected = [("a", false), ("b日", true), ("cd", false)];
        assert_eq!(out, expected.map(|(t, b)| (t.to_owned(), b)));
    }

    #[test]
    fn overlay_splits_at_runs() {
        let runs = [
            Run {
                end: 3,
                rule: Rule(0),
            },
            Run {
                end: 6,
                rule: Rule(Rule::NONE),
            },
            Run {
                end: 9,
                rule: Rule(1),
            },
        ];
        assert_eq!(overlaid("cdefgh", 2, &runs), ["c", "def", "gh"]);
        assert_eq!(overlaid("abc", 0, &[]), ["abc"]);
    }

    fn window(line: &str, x: u16, width: u16) -> String {
        let mut out = String::new();
        let theme = Theme::builtin();
        let window = Window { x, width };
        window.paint(line, &[], Style::new(), &theme, &mut |t: &str, _| {
            out.push_str(t);
        });
        out
    }

    #[test]
    fn windows_slice_by_columns() {
        assert_eq!(window("abcdefgh", 0, 4), "abcd");
        assert_eq!(window("abcdefgh", 3, 4), "defg");
        assert_eq!(window("abcdefgh", 6, 4), "gh  ");
        assert_eq!(window("ab", 5, 3), "   ");
        // 日 spans columns 2-3, 本 4-5
        assert_eq!(window("ab日本c", 3, 3), " 本");
        assert_eq!(window("ab日本c", 0, 3), "ab ");
        assert_eq!(window("ab日本c", 1, 4), "b日 ");
    }
}
