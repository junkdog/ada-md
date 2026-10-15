//! Rows → styled pieces, shared by the viewport and `--print`. Code rows are where paint does
//! real work: syntax colours are overlaid on the code span, and in no-wrap mode the span is a
//! window into its line, shifted by the shared offset (ADR-20).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
}

impl Painter<'_> {
    /// Paint rows starting at `scroll` into `area`.
    pub fn rows(&self, buf: &mut Buffer, area: Rect, scroll: u32) {
        let rows = self.layout.rows.iter().skip(scroll as usize);
        for (y, row) in (area.y..area.bottom()).zip(rows) {
            self.row_into(buf, area.x, y, area.right(), row);
        }
    }

    /// Paint row `row` on line `y`, from `x` up to `right`.
    pub fn row_into(&self, buf: &mut Buffer, mut x: u16, y: u16, right: u16, row: &Row) {
        self.row(row, &mut |text, style| {
            if x < right {
                (x, _) = buf.set_stringn(x, y, text, usize::from(right - x), style);
            }
        });
    }

    /// A row's styled pieces, left to right. Their widths add up to the row's.
    pub fn row(&self, row: &Row, emit: &mut impl FnMut(&str, Style)) {
        let RowKind::Code { block, line, from } = row.kind else {
            for (text, span) in self.layout.spans(row) {
                emit(text, span_style(self.theme, span));
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

    /// A clip mark, padded to the `width` of the span it replaces.
    fn mark(&self, glyph: &str, width: u16, pad: Style, emit: &mut impl FnMut(&str, Style)) {
        emit(glyph, self.theme.style(StyleKey::CodeWrapMark));
        spaces(usize::from(width).saturating_sub(glyph.width()), pad, emit);
    }
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
    let mut run_start = 0;
    for run in runs {
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
