//! Snapshot and invariant tests over `tests/fixtures/` (architecture §13).

use std::fs;
use std::path::{Path, PathBuf};

use super::{Layout, LayoutParams, RowKind, layout};
use crate::doc::{BlockKind, parse};
use crate::theme::Theme;

fn fixtures() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .expect("tests/fixtures")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let name = p.file_stem().unwrap().to_string_lossy().into_owned();
            (name, fs::read_to_string(&p).expect("fixture"))
        })
        .collect()
}

fn params(width: u16) -> LayoutParams {
    LayoutParams {
        width,
        max_width: None,
        code_wrap: true,
        glyphs_gen: 0,
    }
}

fn render(layout: &Layout) -> String {
    let mut out = String::new();
    for (i, row) in layout.rows.iter().enumerate() {
        let src = if row.src.is_none() {
            "     ".to_owned()
        } else if row.src.start == row.src.end {
            format!("{:>5}", row.src.start)
        } else {
            format!("{:>2}-{:<2}", row.src.start, row.src.end)
        };
        out.push_str(&format!("{src}│{}\n", layout.row_text(i).trim_end()));
    }
    out
}

#[test]
fn fixture_snapshots() {
    let theme = Theme::builtin();
    for (name, text) in fixtures() {
        let doc = parse(&text);
        for width in [40, 80, 135] {
            let l = layout(&doc, &theme.glyphs, params(width));
            insta::assert_snapshot!(format!("{name}@{width}"), render(&l));
        }
    }
}

/// Every row that renders content maps to source, mapping is monotonic outside the footnote
/// section, and no row is wider than the terminal.
#[test]
fn mapping_and_width_invariants() {
    let theme = Theme::builtin();
    for (name, text) in fixtures() {
        let doc = parse(&text);
        let footnotes = doc
            .blocks
            .iter()
            .position(|b| matches!(b.kind, BlockKind::FootnoteDefs));
        for width in [12, 20, 40, 80, 135, 200] {
            let l = layout(&doc, &theme.glyphs, params(width));
            check_invariants(&l, footnotes, &format!("{name}@{width}"));
        }
    }
}

pub(crate) fn check_invariants(l: &Layout, footnote_block: Option<usize>, what: &str) {
    let end = footnote_block.map_or(l.rows.len(), |b| l.block_rows[b].start as usize);
    let mut last = 0;
    for (i, row) in l.rows.iter().enumerate() {
        let text = l.row_text(i);
        let width: u16 = l.spans(row).map(|(_, s)| s.width).sum();
        assert_eq!(
            usize::from(width),
            unicode_width::UnicodeWidthStr::width(text.as_str()),
            "{what} row {i}: span widths disagree with the text"
        );
        if l.params.width >= 20 {
            assert!(
                width <= l.params.width,
                "{what} row {i} is {width} wide: {text:?}"
            );
        }
        if matches!(row.kind, RowKind::Code { .. })
            || (row.kind == RowKind::Text && !text.trim().is_empty())
        {
            assert!(!row.src.is_none(), "{what} row {i} has no source: {text:?}");
        }
        if i < end && !row.src.is_none() {
            assert!(
                row.src.start >= last,
                "{what} row {i} goes back from line {last} to {}: {text:?}",
                row.src.start
            );
            last = row.src.start;
        }
    }
}

#[test]
fn max_width_limits_prose_but_not_tables() {
    let theme = Theme::builtin();
    let text = "word ".repeat(40)
        + "\n\n| a | b |\n|---|---|\n| "
        + &"x".repeat(60)
        + " | "
        + &"y".repeat(40)
        + " |\n";
    let doc = parse(&text);
    let l = layout(
        &doc,
        &theme.glyphs,
        LayoutParams {
            max_width: Some(60),
            ..params(140)
        },
    );
    let widest = |range: std::ops::Range<u32>| {
        range
            .map(|r| l.row_text(r as usize).trim_end().chars().count())
            .max()
            .unwrap_or(0)
    };
    assert!(widest(l.block_rows[0].clone()) <= 58);
    assert!(widest(l.block_rows[1].clone()) > 100);
}

#[test]
fn hits_cover_link_text() {
    let theme = Theme::builtin();
    let doc = parse("see [the docs](x.md) and [[wiki]]\n");
    let l = layout(&doc, &theme.glyphs, params(80));
    let texts: Vec<_> = l
        .hits
        .iter()
        .map(|h| {
            let row = l.row_text(h.row as usize);
            row.chars()
                .skip(h.cols.start as usize)
                .take((h.cols.end - h.cols.start) as usize)
                .collect::<String>()
        })
        .collect();
    assert_eq!(texts, ["the docs", "wiki"]);
}

#[test]
fn anchors_map_to_rows() {
    let theme = Theme::builtin();
    let doc = parse("# One\n\ntext\n\n## Two\n\nnote[^n]\n\n[^n]: the note\n");
    let l = layout(&doc, &theme.glyphs, params(80));
    let rows: Vec<String> = l
        .anchor_rows
        .iter()
        .map(|&r| l.row_text(r as usize).trim().to_owned())
        .collect();
    assert_eq!(rows, ["One", "## Two", "[1] the note"]);
}

/// The invariants over a real corpus: `PREEN_CORPUS=~/opt/birb-vault cargo test -- --ignored`.
#[test]
#[ignore = "needs PREEN_CORPUS"]
fn corpus_invariants() {
    let root = PathBuf::from(std::env::var_os("PREEN_CORPUS").expect("PREEN_CORPUS"));
    let mut stack = vec![root];
    let mut notes = 0;
    let theme = Theme::builtin();
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("readable corpus").flatten() {
            let path = entry.path();
            if path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
            {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "md")
                && let Ok(text) = fs::read_to_string(&path)
            {
                let doc = parse(&text);
                let footnotes = doc
                    .blocks
                    .iter()
                    .position(|b| matches!(b.kind, BlockKind::FootnoteDefs));
                for width in [40, 80, 135] {
                    let l = layout(&doc, &theme.glyphs, params(width));
                    check_invariants(&l, footnotes, &format!("{}@{width}", path.display()));
                }
                notes += 1;
            }
        }
    }
    assert!(notes > 0);
}
