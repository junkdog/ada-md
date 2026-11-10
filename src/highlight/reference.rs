//! syntect on bat's whole syntax set, as ada highlighted until 1.1: what ada's scope selectors
//! are checked against (ADR-43), and what `examples/parity.rs` reports ada's lexers' colors
//! against (a report, not a check: X-116). syntect is a dev-dependency only, so this module
//! isn't in the library: its tests include it as `highlight::reference`, and the examples by
//! path, with `highlight` and `layout` at their crate's root (`use ada::{highlight, layout};`).

use std::str::FromStr;
use std::sync::OnceLock;

use syntect::highlighting::{
    Color, HighlightIterator, HighlightState, Highlighter, ScopeSelectors, StyleModifier, Theme,
    ThemeItem, ThemeSettings,
};
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxSet};

use crate::highlight::{CodeSpans, Rule, Run, SyntaxTheme};
use crate::layout::expand_tabs_onto;

/// Longer lines syntect left plain in ada, unparsed: backtracking regexes can be slow on them.
pub const MAX_LINE: usize = 4096;

/// bat's whole syntax set, as two-face ships it on Oniguruma.
pub fn syntaxes() -> &'static SyntaxSet {
    static FULL: OnceLock<SyntaxSet> = OnceLock::new();
    FULL.get_or_init(two_face::syntax::extra_newlines)
}

/// `rules` as a syntect theme, as ada built it: rule `i` sets the foreground to an encoding of
/// `i`; text no rule matches keeps the transparent default.
pub fn theme(rules: &SyntaxTheme) -> Theme {
    let scopes = rules
        .scopes()
        .enumerate()
        .filter_map(|(i, scope)| {
            let scope = ScopeSelectors::from_str(scope).ok()?;
            let foreground = Some(Color {
                r: (i >> 8) as u8,
                g: i as u8,
                b: 0,
                a: 0xff,
            });
            Some(ThemeItem {
                scope,
                style: StyleModifier {
                    foreground,
                    background: None,
                    font_style: None,
                },
            })
        })
        .collect();
    let settings = ThemeSettings {
        foreground: Some(Color {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        }),
        ..ThemeSettings::default()
    };
    Theme {
        settings,
        scopes,
        ..Theme::default()
    }
}

/// The rule a style's foreground stands for ([`theme`]'s encoding).
fn decode(c: Color) -> Rule {
    match c.a {
        0 => Rule(Rule::NONE),
        _ => Rule(u16::from(c.r) << 8 | u16::from(c.g)),
    }
}

/// The rule syntect gives a scope stack, its outermost scope first (`style_for_stack`).
pub fn rule(theme: &Theme, stack: &[&str]) -> Rule {
    let stack: Vec<Scope> = stack
        .iter()
        .map(|s| Scope::new(s).expect("scope"))
        .collect();
    decode(Highlighter::new(theme).style_for_stack(&stack).foreground)
}

/// A code block's text highlighted by syntect with `set`'s syntax `name`, line by line as ada's
/// `highlight` goes; `None` when the set has no such syntax or syntect fails on a line.
pub fn highlight(
    set: &'static SyntaxSet,
    name: &str,
    text: &str,
    rules: &SyntaxTheme,
) -> Option<CodeSpans> {
    let syntax = set.find_syntax_by_name(name)?;
    let theme = theme(rules);
    let highlighter = Highlighter::new(&theme);
    let mut parse = ParseState::new(syntax);
    let mut scopes = HighlightState::new(&highlighter, ScopeStack::new());
    let (mut buf, mut lines) = (String::new(), Vec::new());
    // the lines as ada's code blocks split them
    let text = text.strip_suffix('\n').unwrap_or(text);
    for line in text.split('\n').filter(|_| !text.is_empty()) {
        let line = line.strip_suffix('\r').unwrap_or(line);
        buf.clear();
        expand_tabs_onto(line, &mut buf);
        let len = buf.len();
        if len > MAX_LINE {
            lines.push(Vec::new());
            continue;
        }
        buf.push('\n');
        let ops = parse.parse_line(&buf, set).ok()?;
        let mut runs = Vec::new();
        let mut end = 0;
        for (style, piece) in HighlightIterator::new(&mut scopes, &ops, &buf, &highlighter) {
            let start = end;
            end = (end + piece.len()).min(len);
            if end > start {
                runs.push(Run {
                    end: end as u32,
                    rule: decode(style.foreground),
                });
            }
        }
        lines.push(runs);
    }
    Some(CodeSpans::from_lines(rules.id, lines))
}
