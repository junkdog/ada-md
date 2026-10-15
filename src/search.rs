//! Search (ADR-12, architecture §6). A regex runs over markup-free text built from the IR, so it
//! never has to account for `**`, backticks or decorations. Link destinations, aliased wiki
//! targets and image sources are searched too, as *hidden* units owned by a visible link.
//!
//! The text depends only on the Doc. Matches are projected onto rows and columns per layout, so
//! they survive resize and table-mode changes.

use std::ops::Range;

use regex::{Regex, RegexBuilder};
use unicode_width::UnicodeWidthStr;

use crate::doc::{Block, BlockKind, Doc, LinkId, LinkKind, UnitId, code_lines};
use crate::layout::{self, Layout, RowKind, expand_tabs, wrap};
use crate::theme::Glyphs;

/// Matches past this many are dropped: a pattern like `.` shouldn't stall the UI.
pub const MAX_MATCHES: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitKind {
    Visible,
    /// Text that isn't on screen, such as a link's destination: matches show on its owner.
    Hidden {
        owner: LinkId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unit {
    /// Bytes of [`SearchMap::text`].
    pub start: u32,
    pub end: u32,
    pub kind: UnitKind,
}

/// A Doc's searchable text: every unit, joined with `\n`. Visible units come first, indexed by
/// [`UnitId`], then the hidden ones.
#[derive(Debug, Default)]
pub struct SearchMap {
    pub text: String,
    pub units: Vec<Unit>,
}

impl SearchMap {
    pub fn build(doc: &Doc, glyphs: &Glyphs) -> Self {
        let mut texts = vec![String::new(); doc.unit_count as usize];
        let footnotes = doc.footnotes.iter().map(|f| &f.blocks[..]);
        for blocks in std::iter::once(&doc.blocks[..]).chain(footnotes) {
            collect(blocks, doc, glyphs, &mut texts);
        }
        let mut map = SearchMap::default();
        for text in &texts {
            map.push(text, UnitKind::Visible);
        }
        for (id, link) in doc.links.iter().enumerate() {
            let hidden = match link.kind {
                LinkKind::Inline | LinkKind::Reference | LinkKind::Image => true,
                LinkKind::Wiki { aliased } => aliased,
                LinkKind::Autolink | LinkKind::Email | LinkKind::Footnote(_) => false,
            };
            if hidden && !link.dest.is_empty() {
                let owner = id as LinkId;
                map.push(&wrap::clean(&link.dest), UnitKind::Hidden { owner });
            }
        }
        map
    }

    fn push(&mut self, text: &str, kind: UnitKind) {
        if !self.units.is_empty() {
            self.text.push('\n');
        }
        let start = self.text.len() as u32;
        self.text.push_str(text);
        self.units.push(Unit {
            start,
            end: self.text.len() as u32,
            kind,
        });
    }

    pub fn unit_text(&self, unit: UnitId) -> &str {
        self.units
            .get(unit as usize)
            .map_or("", |u| &self.text[u.start as usize..u.end as usize])
    }

    /// Every non-empty match of `re`, in text order, clipped to the unit it starts in.
    pub fn find(&self, re: &Regex) -> Vec<Match> {
        let mut out = Vec::new();
        for m in re.find_iter(&self.text) {
            if m.is_empty() {
                continue;
            }
            let (start, end) = (m.start() as u32, m.end() as u32);
            let i = self.units.partition_point(|u| u.start <= start);
            let Some(unit) = i.checked_sub(1).and_then(|i| self.units.get(i)) else {
                continue;
            };
            let end = end.min(unit.end);
            if start >= end {
                continue;
            }
            out.push(Match {
                unit: (i - 1) as UnitId,
                start: start - unit.start,
                end: end - unit.start,
            });
            if out.len() >= MAX_MATCHES {
                break;
            }
        }
        out
    }
}

fn collect(blocks: &[Block], doc: &Doc, glyphs: &Glyphs, texts: &mut [String]) {
    fn put(texts: &mut [String], unit: UnitId, text: String) {
        if let Some(slot) = texts.get_mut(unit as usize) {
            *slot = text;
        }
    }
    for block in blocks {
        match &block.kind {
            BlockKind::Heading { inlines, .. } | BlockKind::Paragraph(inlines) => {
                put(
                    texts,
                    inlines.unit,
                    wrap::unit_text(&inlines.runs, doc, glyphs),
                );
            }
            BlockKind::Table(t) => {
                for row in std::iter::once(&t.header).chain(&t.rows) {
                    for cell in &row.cells {
                        put(texts, cell.unit, wrap::unit_text(&cell.runs, doc, glyphs));
                    }
                }
            }
            BlockKind::Code(code) => {
                for (n, line) in code_lines(&code.text).enumerate() {
                    put(
                        texts,
                        code.first_unit + n as UnitId,
                        expand_tabs(line).into_owned(),
                    );
                }
            }
            BlockKind::Image { alt, unit, .. } => {
                put(texts, *unit, wrap::clean(layout::image_alt(alt)));
            }
            BlockKind::List(list) => {
                for item in &list.items {
                    collect(&item.blocks, doc, glyphs, texts);
                }
            }
            BlockKind::Quote { blocks, .. } => collect(blocks, doc, glyphs, texts),
            BlockKind::Rule | BlockKind::Html(_) | BlockKind::FootnoteDefs => {}
        }
    }
}

/// A match: bytes `start..end` of a unit's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub unit: UnitId,
    pub start: u32,
    pub end: u32,
}

/// Compile a user pattern. Smart-case: case-insensitive unless the pattern has an uppercase
/// letter; `\c` forces insensitive and `\C` sensitive.
pub fn compile(pattern: &str) -> Result<Regex, String> {
    let mut clean = String::with_capacity(pattern.len());
    let mut force = None;
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            clean.push(c);
            continue;
        }
        match chars.next() {
            Some('c') => force = Some(true),
            Some('C') => force = Some(false),
            Some(n) => {
                clean.push('\\');
                clean.push(n);
            }
            None => clean.push('\\'),
        }
    }
    let insensitive = force.unwrap_or_else(|| !has_uppercase(&clean));
    RegexBuilder::new(&clean)
        .case_insensitive(insensitive)
        .size_limit(1 << 23)
        .build()
        .map_err(|e| match e {
            regex::Error::Syntax(s) => s.lines().last().unwrap_or_default().trim().to_owned(),
            other => other.to_string(),
        })
}

/// Uppercase letters outside escapes: `\S` or `\W` don't make a pattern case-sensitive.
fn has_uppercase(pattern: &str) -> bool {
    let mut escaped = false;
    for c in pattern.chars() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c.is_uppercase() {
            return true;
        }
    }
    false
}

/// Cells a match covers on one row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cells {
    pub row: u32,
    pub cols: Range<u16>,
    /// The match, an index into the match list.
    pub at: u32,
    /// Painted on the owner of a hidden match.
    pub hidden: bool,
    /// In no-wrap code: where the code window starts, for paint to shift by the offset.
    pub window: Option<u16>,
}

/// Matches projected onto one layout.
#[derive(Debug, Default)]
pub struct Projection {
    /// Sorted by row.
    pub cells: Vec<Cells>,
    /// Match indices in screen order; matches that don't render are left out.
    pub order: Vec<u32>,
    /// Match index → its position in `order`.
    rank: Vec<Option<u32>>,
}

impl Projection {
    pub fn new(map: &SearchMap, matches: &[Match], layout: &Layout) -> Self {
        let mut by_unit: Vec<u32> = (0..layout.search.len() as u32).collect();
        by_unit.sort_by_key(|&i| layout.search[i as usize].unit);
        let mut by_link: Vec<u32> = (0..layout.hits.len() as u32).collect();
        by_link.sort_by_key(|&i| layout.hits[i as usize].link);
        let nowrap = !layout.params.code_wrap;

        let mut cells = Vec::new();
        for (at, m) in matches.iter().enumerate() {
            let at = at as u32;
            match map.units.get(m.unit as usize).map(|u| u.kind) {
                Some(UnitKind::Visible) => {
                    let text = map.unit_text(m.unit);
                    let first =
                        by_unit.partition_point(|&i| layout.search[i as usize].unit < m.unit);
                    for &i in &by_unit[first..] {
                        let seg = layout.search[i as usize];
                        if seg.unit != m.unit {
                            break;
                        }
                        let (a, b) = (m.start.max(seg.start), m.end.min(seg.end));
                        if a >= b {
                            continue;
                        }
                        let width = |to: u32| {
                            text.get(seg.start as usize..to as usize)
                                .map_or(0, |t| t.width() as u16)
                        };
                        let code =
                            matches!(layout.rows[seg.row as usize].kind, RowKind::Code { .. });
                        cells.push(Cells {
                            row: seg.row,
                            cols: seg.col + width(a)..seg.col + width(b),
                            at,
                            hidden: false,
                            window: (nowrap && code).then_some(seg.col),
                        });
                    }
                }
                Some(UnitKind::Hidden { owner }) => {
                    let first = by_link.partition_point(|&i| layout.hits[i as usize].link < owner);
                    for &i in &by_link[first..] {
                        let hit = &layout.hits[i as usize];
                        if hit.link != owner {
                            break;
                        }
                        cells.push(Cells {
                            row: hit.row,
                            cols: hit.cols.clone(),
                            at,
                            hidden: true,
                            window: None,
                        });
                    }
                }
                None => {}
            }
        }
        cells.sort_by_key(|c| (c.row, c.cols.start));
        // One stretch per row, covering decoration inside the match, such as inline code padding.
        cells.dedup_by(|next, prev| {
            let same = (prev.at, prev.row, prev.hidden) == (next.at, next.row, next.hidden);
            if same {
                prev.cols.end = prev.cols.end.max(next.cols.end);
            }
            same
        });

        let mut order = Vec::new();
        let mut rank = vec![None; matches.len()];
        for c in &cells {
            if let Some(slot) = rank.get_mut(c.at as usize)
                && slot.is_none()
            {
                *slot = Some(order.len() as u32);
                order.push(c.at);
            }
        }
        Self { cells, order, rank }
    }

    /// The position of match `at` in screen order.
    pub fn rank(&self, at: u32) -> Option<u32> {
        self.rank.get(at as usize).copied().flatten()
    }

    /// The first row match `at` shows on.
    pub fn row_of(&self, at: u32) -> Option<u32> {
        self.cells.iter().find(|c| c.at == at).map(|c| c.row)
    }

    /// The cells on rows `rows`.
    pub fn on_rows(&self, rows: Range<u32>) -> &[Cells] {
        let a = self.cells.partition_point(|c| c.row < rows.start);
        let b = self.cells.partition_point(|c| c.row < rows.end);
        &self.cells[a..b]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::parse;
    use crate::layout::{LayoutParams, TableMode};
    use crate::theme::Theme;

    fn params(width: u16) -> LayoutParams {
        LayoutParams {
            width,
            max_width: None,
            table_mode: TableMode::Auto,
            code_wrap: true,
            glyphs_gen: 0,
            images: None,
        }
    }

    /// The text under each match's cells, in screen order.
    fn found(md: &str, pattern: &str, width: u16) -> Vec<String> {
        found_with(md, pattern, params(width))
    }

    fn found_with(md: &str, pattern: &str, p: LayoutParams) -> Vec<String> {
        let theme = Theme::builtin();
        let doc = parse(md);
        let map = SearchMap::build(&doc, &theme.glyphs);
        let matches = map.find(&compile(pattern).unwrap());
        let layout = crate::layout::layout(&doc, &theme.glyphs, p);
        let proj = Projection::new(&map, &matches, &layout);
        proj.order
            .iter()
            .map(|&at| {
                proj.cells
                    .iter()
                    .filter(|c| c.at == at)
                    .map(|c| {
                        let row = layout.row_text(c.row as usize);
                        let text: String = cells_text(&row, c.cols.clone());
                        if c.hidden { format!("[{text}]") } else { text }
                    })
                    .collect::<Vec<_>>()
                    .join("|")
            })
            .collect()
    }

    fn cells_text(row: &str, cols: Range<u16>) -> String {
        let mut col = 0;
        let mut out = String::new();
        for c in row.chars() {
            let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) as u16;
            if col >= cols.start && col < cols.end {
                out.push(c);
            }
            col += w;
        }
        out
    }

    #[test]
    fn markup_is_invisible_to_search() {
        let md = "**foo** _bar_\n\n[foo bar](x.md)\n\n`foo` bar\n\n## foo  bar\n";
        assert_eq!(
            found(md, "foo bar", 80),
            ["foo bar", "foo bar", "foo  bar"],
            "the heading has two spaces"
        );
        assert_eq!(found(md, "foo +bar", 80).len(), 4);
    }

    #[test]
    fn matches_cross_soft_wraps() {
        let md = "alpha beta gamma delta\n";
        assert_eq!(found(md, "gamma delta", 20), ["gamma|delta"]);
    }

    #[test]
    fn smart_case() {
        let md = "Foo foo FOO\n";
        assert_eq!(found(md, "foo", 80).len(), 3);
        assert_eq!(found(md, "Foo", 80), ["Foo"]);
        assert_eq!(found(md, "Foo\\c", 80).len(), 3);
        assert_eq!(found(md, "\\Cfoo", 80), ["foo"]);
        assert_eq!(found(md, "\\w+", 80).len(), 3, "escapes are not uppercase");
        assert_eq!(found(md, "\\Sfoo", 80).len(), 0);
    }

    #[test]
    fn invalid_patterns_report_an_error() {
        let err = compile("foo(").unwrap_err();
        assert!(err.contains("unclosed"), "{err}");
    }

    #[test]
    fn hidden_text_matches_on_its_owner() {
        let md = "see [the docs](../resources/README.md) and [[note#Part|alias]] and [[plain]]\n\n![cat](img/cat.png)\n";
        assert_eq!(found(md, "readme", 80), ["[the docs]"]);
        assert_eq!(found(md, "note", 80), ["[alias]"]);
        assert_eq!(found(md, "plain", 80), ["plain"], "no hidden twin");
        // the image block shows its source, but as a hidden unit it lands on the whole block
        assert_eq!(found(md, "cat", 80), ["[🖼 cat (img/cat.png)]", "cat"]);
    }

    #[test]
    fn units_do_not_join() {
        let md = "| a | b |\n|---|---|\n| foo | bar |\n\nfoo\n\nbar\n";
        assert!(found(md, "foo bar", 80).is_empty());
        assert!(found(md, "foo.bar", 80).is_empty());
    }

    #[test]
    fn code_lines_and_cells() {
        let md =
            "```rust\nlet\tx = 1;\nlet y = 2;\n```\n\n| k | v |\n|---|---|\n| x | **y** = 2 |\n";
        assert_eq!(found(md, "y = 2", 80), ["y = 2", "y = 2"]);
        assert_eq!(found(md, "let +x", 80), ["let x"], "tabs are expanded");
    }

    #[test]
    fn record_keys_match_on_every_card() {
        let md = "| key | v |\n|---|---|\n| 1 | a |\n| 2 | b |\n";
        let p = LayoutParams {
            table_mode: TableMode::Records,
            ..params(80)
        };
        assert_eq!(found_with(md, "key", p), ["key|key"]);
    }

    #[test]
    fn inline_images_and_footnotes() {
        let md = "a ![small icon](i.png) b[^1]\n\n[^1]: the note\n";
        assert_eq!(found(md, "icon", 80), ["icon"]);
        assert!(
            found(md, "\\[1\\]", 80).is_empty(),
            "footnote numbers are decoration"
        );
        assert_eq!(found(md, "note", 80), ["note"]);
    }

    #[test]
    fn nowrap_code_cells_know_their_window() {
        let theme = Theme::builtin();
        let doc = parse("```\nabc def\n```\n");
        let map = SearchMap::build(&doc, &theme.glyphs);
        let matches = map.find(&compile("def").unwrap());
        let p = LayoutParams {
            code_wrap: false,
            ..params(40)
        };
        let layout = crate::layout::layout(&doc, &theme.glyphs, p);
        let proj = Projection::new(&map, &matches, &layout);
        let c = &proj.cells[0];
        let w = c.window.expect("a window");
        assert_eq!(c.cols.start - w, 4);
    }
}
