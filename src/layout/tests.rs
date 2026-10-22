//! Snapshot and invariant tests over `tests/fixtures/` (architecture §13).

use std::fs;
use std::path::{Path, PathBuf};

use crate::geom::Size;

use super::{
    ImageFit, Layout, LayoutParams, LineNumbers, Measure, RowKind, Sort, TableMode, layout,
    layout_measured, layout_print, relayout,
};
use crate::doc::{BlockKind, parse};
use crate::theme::{Glyphs, TableBorder, Theme};

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

const MODES: [TableMode; 3] = [TableMode::Auto, TableMode::Grid, TableMode::Records];

fn params(width: u16) -> LayoutParams {
    LayoutParams {
        width,
        max_width: None,
        table_mode: TableMode::Auto,
        code_wrap: true,
        line_numbers: Default::default(),
        glyphs_gen: 0,
        images: None,
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

/// A layout built in another one's buffers is the layout built from scratch.
#[test]
fn relayout_in_spare_buffers_matches_a_fresh_layout() {
    let theme = Theme::builtin();
    let fixtures = fixtures();
    // Each layout reuses the buffers of a different document's, at another width.
    let mut spare = layout(&parse("# spare\n\n[a](b) `c`\n"), &theme.glyphs, params(30));
    for (name, md) in &fixtures {
        let doc = parse(md);
        let measure = Measure::new(&doc, &theme.glyphs);
        for width in [24, 80] {
            let fresh = layout_measured(&doc, &measure, &theme.glyphs, params(width));
            let reused = relayout(&doc, &measure, &theme.glyphs, params(width), spare);
            assert_eq!(
                format!("{fresh:?}"),
                format!("{reused:?}"),
                "{name} at {width}"
            );
            spare = reused;
        }
    }
}

/// Print's layout is the screen's without the search segments and link hits (MEM-05).
#[test]
fn print_layout_is_the_screen_layout_without_search_and_hits() {
    let theme = Theme::builtin();
    for (name, md) in &fixtures() {
        let doc = parse(md);
        let measure = Measure::new(&doc, &theme.glyphs);
        for width in [24, 80] {
            let mut screen = layout_measured(&doc, &measure, &theme.glyphs, params(width));
            let print = layout_print(&doc, &measure, &theme.glyphs, params(width));
            assert!(print.search.is_empty() && print.hits.is_empty(), "{name}");
            screen.search.clear();
            screen.hits.clear();
            assert_eq!(
                format!("{screen:?}"),
                format!("{print:?}"),
                "{name} at {width}"
            );
        }
    }
}

/// The forced table modes, at the widths where they differ from `auto`.
#[test]
fn table_mode_snapshots() {
    let theme = Theme::builtin();
    let text =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tables.md"))
            .expect("fixture");
    let doc = parse(&text);
    for (mode, width) in [(TableMode::Grid, 40), (TableMode::Records, 80)] {
        let l = layout(
            &doc,
            &theme.glyphs,
            LayoutParams {
                table_mode: mode,
                ..params(width)
            },
        );
        insta::assert_snapshot!(format!("tables-{}@{width}", mode.name()), render(&l));
    }
}

/// Code in no-wrap mode: one row per line, in a fixed window that paint slices.
#[test]
fn code_nowrap_snapshot() {
    let theme = Theme::builtin();
    let text =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/code.md"))
            .expect("fixture");
    let doc = parse(&text);
    let l = layout(
        &doc,
        &theme.glyphs,
        LayoutParams {
            code_wrap: false,
            ..params(40)
        },
    );
    insta::assert_snapshot!("code-nowrap@40", render(&l));
    let long = l
        .code
        .iter()
        .find(|c| c.width > c.window)
        .expect("the rust block has a long line");
    assert_eq!(long.window, 40 - 2 * 2 - 2);
    assert!(long.max_x() > 0);
    assert!(l.code.iter().all(|c| c.window == long.window));
}

#[test]
fn code_rows_know_their_byte_offsets() {
    let theme = Theme::builtin();
    let doc = parse("```rust\nlet x = \"abcdefghijklmnopqrstuvwxyz\";\n```\n");
    let l = layout(&doc, &theme.glyphs, params(20));
    let rows: Vec<_> = l
        .rows
        .iter()
        .enumerate()
        .filter_map(|(i, r)| match r.kind {
            RowKind::Code { line, from, .. } => Some((line, from, i)),
            _ => None,
        })
        .collect();
    let mut joined = String::new();
    for &(line, from, i) in &rows {
        assert_eq!(line, 0);
        assert_eq!(from as usize, joined.len());
        let code: String = l
            .spans(&l.rows[i])
            .filter(|(_, s)| s.key == crate::theme::StyleKey::CodeText)
            .map(|(t, _)| t)
            .collect();
        joined.push_str(&code);
    }
    assert!(rows.len() > 1);
    assert_eq!(joined, "let x = \"abcdefghijklmnopqrstuvwxyz\";");
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
        let map = crate::search::SearchMap::build(&doc, &theme.glyphs);
        for width in [12, 20, 40, 80, 135, 200] {
            for table_mode in MODES {
                for code_wrap in [true, false] {
                    let p = LayoutParams {
                        table_mode,
                        code_wrap,
                        ..params(width)
                    };
                    let l = layout(&doc, &theme.glyphs, p);
                    let what = format!("{name}@{width} {table_mode:?} wrap={code_wrap}");
                    check_invariants(&l, footnotes, &what);
                    check_search_segs(&l, &map, &what);
                }
            }
        }
    }
}

/// Every search segment shows exactly its stretch of the unit's text, where it says it does.
pub(crate) fn check_search_segs(l: &Layout, map: &crate::search::SearchMap, what: &str) {
    for seg in &l.search {
        let row = &l.rows[seg.row as usize];
        if !l.params.code_wrap && matches!(row.kind, RowKind::Code { .. }) {
            continue;
        }
        let unit = map.unit_text(seg.unit);
        let Some(expected) = unit.get(seg.start as usize..seg.end as usize) else {
            panic!("{what}: {seg:?} is outside its unit {unit:?}");
        };
        let expected = expected.replace(super::wrap::NBSP, " ");
        let width = unicode_width::UnicodeWidthStr::width(expected.as_str());
        // Columns as layout counts them: string widths, which treat emoji sequences as one.
        let text = l.row_text(seg.row as usize);
        let col = |i: usize| unicode_width::UnicodeWidthStr::width(&text[..i]);
        let mut bounds = (0..=text.len()).filter(|&i| text.is_char_boundary(i));
        let start = usize::from(seg.col);
        let a = bounds.clone().find(|&i| col(i) >= start);
        // zero-width characters such as joiners belong to what precedes them
        let b = bounds.rfind(|&i| col(i) <= start + width);
        let (Some(a), Some(b)) = (a, b) else {
            panic!("{what}: {seg:?} runs past row {text:?}");
        };
        assert_eq!(&text[a..b], expected, "{what}: {seg:?} in row {text:?}");
    }
}

pub(crate) fn check_invariants(l: &Layout, footnote_block: Option<usize>, what: &str) {
    let end = footnote_block.map_or(l.rows.len(), |b| l.block_rows[b].start as usize);
    let mut last = 0;
    for (i, row) in l.rows.iter().enumerate() {
        let text = l.row_text(i);
        let width: u16 = l.spans(row).map(|(_, s)| s.width).sum();
        // A no-wrap code span is a window onto its whole line, so only its width is fixed.
        let window = !l.params.code_wrap && matches!(row.kind, RowKind::Code { .. });
        if !window {
            assert_eq!(
                usize::from(width),
                unicode_width::UnicodeWidthStr::width(text.as_str()),
                "{what} row {i}: span widths disagree with the text"
            );
        }
        // A diagram row holds its whole line; paint shows the block's window of it.
        if let RowKind::Diagram { block, x } = row.kind {
            let info = l.code[block as usize];
            assert!(
                l.params.width < 20 || x + info.window <= l.params.width,
                "{what} row {i}: a {}-column window from column {x}",
                info.window
            );
        } else if l.params.width >= 20 {
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
fn grid_rows_cover_header_and_body() {
    let theme = Theme::builtin();
    let doc = parse("text\n\n| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n\n| only |\n|---|\n");
    let l = layout(&doc, &theme.glyphs, params(80));
    assert_eq!(
        l.grids.len(),
        1,
        "a header-only table has nothing to scroll"
    );
    let g = &l.grids[0];
    let header: Vec<_> = g.header.clone().map(|r| l.row_text(r as usize)).collect();
    assert_eq!(header.len(), 2);
    assert!(header[0].contains("│ a │ b │"));
    assert!(header[1].contains("├───┼───┤"));
    assert!(l.row_text(g.end as usize - 1).contains("│ 3 │ 4 │"));
    assert!(l.row_text(g.end as usize).contains("╰───┴───╯"));
}

/// A bordered grid draws the sort mark on the rule under its header; without a border, and on
/// cards, it sits in the padding after the header. Either way a one-cell mark leaves the table as
/// wide as it was.
#[test]
fn sort_marks_sit_on_the_rule_or_in_the_header_padding() {
    let rows = |md: &str, glyphs: &Glyphs, sort: Option<Sort>, table_mode| {
        let doc = parse(md);
        let mut measure = Measure::new(&doc, glyphs);
        measure.set_sort(doc.tables()[0], sort);
        let p = LayoutParams {
            table_mode,
            ..params(40)
        };
        let l = layout_measured(&doc, &measure, glyphs, p);
        let row = |r| l.row_text(r).trim().to_owned();
        (0..l.rows.len()).map(row).collect::<Vec<_>>()
    };
    let md = "| Name | Size |\n|---|--:|\n| beta | 10 |\n| alpha | 9 |\n";
    let by_size = Some(Sort { col: 1, desc: true });
    let by_name = Some(Sort {
        col: 0,
        desc: false,
    });
    let mut glyphs = Theme::builtin().glyphs;
    let unsorted = rows(md, &glyphs, None, TableMode::Grid);
    assert_eq!(unsorted[1..3], ["│ Name  │ Size │", "├───────┼──────┤"]);
    // at the right end of its column's stretch of the rule, the header row untouched
    assert_eq!(
        rows(md, &glyphs, by_size, TableMode::Grid)[1..3],
        ["│ Name  │ Size │", "├───────┼────▼─┤"]
    );
    assert_eq!(
        rows(md, &glyphs, by_name, TableMode::Grid)[1..3],
        ["│ Name  │ Size │", "├─────▲─┼──────┤"]
    );
    // a header alone has it on the bottom edge
    let alone = "| Name | Size |\n|---|--:|\n";
    assert_eq!(
        rows(alone, &glyphs, by_size, TableMode::Grid)[1..3],
        ["│ Name │ Size │", "╰──────┴────▼─╯"]
    );
    // cards have no rule under the header
    let cards = rows(md, &glyphs, by_name, TableMode::Records);
    assert!(cards.contains(&"│ Name▲ alpha".to_owned()), "{cards:#?}");
    assert!(cards.contains(&"│ Size  9".to_owned()), "{cards:#?}");

    glyphs.table_border = TableBorder::Ascii;
    assert_eq!(
        rows(md, &glyphs, by_size, TableMode::Grid)[1..3],
        ["| Name  | Size |", "+-------+----▼-+"]
    );

    // snug when the header fills its column, a space off it when there's room
    glyphs.table_border = TableBorder::None;
    assert_eq!(rows(md, &glyphs, None, TableMode::Grid)[0], "Name   Size");
    assert_eq!(
        rows(md, &glyphs, by_size, TableMode::Grid)[0],
        "Name   Size▼"
    );
    assert_eq!(
        rows(md, &glyphs, by_name, TableMode::Grid)[0],
        "Name ▲ Size"
    );

    // a wider mark comes out of the slack when the column has some, and widens it otherwise
    glyphs.sort_mark = ["↑↑".into(), "↓↓".into()];
    assert_eq!(
        rows(md, &glyphs, by_size, TableMode::Grid)[0],
        "Name   Size↓↓"
    );
    assert_eq!(
        rows(md, &glyphs, by_name, TableMode::Grid)[0],
        "Name↑↑ Size"
    );
    let cards = rows(md, &glyphs, by_name, TableMode::Records);
    assert!(cards.contains(&"1.".to_owned()), "{cards:#?}");
    assert!(cards.contains(&"Name↑↑ alpha".to_owned()), "{cards:#?}");
    assert!(cards.contains(&"Size   9".to_owned()), "{cards:#?}");

    // on the rule it gives up the cell clear of the junction before it widens anything
    glyphs.table_border = TableBorder::Rounded;
    assert_eq!(
        rows(md, &glyphs, by_size, TableMode::Grid)[1..3],
        ["│ Name  │ Size │", "├───────┼───↓↓─┤"]
    );
    glyphs.sort_mark = ["↑↑↑↑↑↑".into(), "↓↓↓↓↓↓".into()];
    assert_eq!(
        rows(md, &glyphs, by_size, TableMode::Grid)[1..3],
        ["│ Name  │ Size │", "├───────┼↓↓↓↓↓↓┤"]
    );
    // and goes without it when it's wider than the stretch
    glyphs.sort_mark = ["↑↑↑↑↑↑↑".into(), "↓↓↓↓↓↓↓".into()];
    assert_eq!(
        rows(md, &glyphs, by_size, TableMode::Grid)[1..3],
        unsorted[1..3]
    );
}

/// Every other body row is striped, on every terminal row it takes, inside the outer borders.
#[test]
fn alternate_rows_are_striped() {
    let theme = Theme::builtin();
    let md = "| k | v |\n|---|---|\n| a | one |\n| b | two words that wrap |\n| c | three |\n";
    let doc = parse(md);
    let striped = |l: &Layout| -> Vec<String> {
        l.rows
            .iter()
            .enumerate()
            .filter_map(|(i, r)| {
                let cols = r.stripe.clone()?;
                let text: String = l
                    .row_text(i)
                    .chars()
                    .skip(cols.start as usize)
                    .take((cols.end - cols.start) as usize)
                    .collect();
                Some(text)
            })
            .collect()
    };
    let grid = layout(&doc, &theme.glyphs, params(20));
    assert_eq!(striped(&grid), [" b │ two words    ", "   │ that wrap    "]);
    let records = layout(
        &doc,
        &theme.glyphs,
        LayoutParams {
            table_mode: TableMode::Records,
            ..params(30)
        },
    );
    let rows = striped(&records);
    assert_eq!(rows.len(), 2, "the second card's fields: {rows:?}");
    assert!(rows[0].starts_with(" k  b"), "{rows:?}");
}

#[test]
fn cells_keep_links_and_styles_when_wrapped() {
    let theme = Theme::builtin();
    let doc = parse(
        "| k | v |\n|---|---|\n| x | see [the long link text](a.md) and **bold words here** |\n",
    );
    let l = layout(
        &doc,
        &theme.glyphs,
        LayoutParams {
            table_mode: TableMode::Grid,
            ..params(30)
        },
    );
    let linked: String = l
        .hits
        .iter()
        .map(|h| {
            let row = l.row_text(h.row as usize);
            row.chars()
                .skip(h.cols.start as usize)
                .take((h.cols.end - h.cols.start) as usize)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(linked, "the long link text");
    let bold: Vec<_> = l
        .rows
        .iter()
        .flat_map(|r| l.spans(r))
        .filter(|(_, s)| s.inline.contains(crate::doc::InlineStyle::STRONG))
        .map(|(t, _)| t.trim())
        .collect();
    assert_eq!(bold.join(" "), "bold words here");
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

/// The invariants over a real corpus: `ADA_CORPUS=~/opt/birb-vault cargo test -- --ignored`.
#[test]
#[ignore = "needs ADA_CORPUS"]
fn corpus_invariants() {
    let root = PathBuf::from(std::env::var_os("ADA_CORPUS").expect("ADA_CORPUS"));
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
                let map = crate::search::SearchMap::build(&doc, &theme.glyphs);
                for width in [40, 80, 135] {
                    for (table_mode, code_wrap) in
                        MODES.iter().flat_map(|&m| [(m, true), (m, false)])
                    {
                        let p = LayoutParams {
                            table_mode,
                            code_wrap,
                            ..params(width)
                        };
                        let l = layout(&doc, &theme.glyphs, p);
                        let what =
                            format!("{}@{width} {table_mode:?} wrap={code_wrap}", path.display());
                        check_invariants(&l, footnotes, &what);
                        check_search_segs(&l, &map, &what);
                    }
                }
                notes += 1;
            }
        }
    }
    assert!(notes > 0);
}

#[test]
fn block_images_reserve_rows_once_their_size_is_known() {
    let theme = Theme::builtin();
    let doc = parse("intro\n\n![a cat](cat.png)\n\n> ![](deep/dog.jpg)\n\n![gone](x.png)\n");
    let mut measure = Measure::new(&doc, &theme.glyphs);
    measure.set_image(0, Some((400, 300)));
    measure.set_image(1, Some((4000, 1000)));
    let p = LayoutParams {
        images: Some(ImageFit {
            cell: Size::new(10, 20),
            max_rows: 10,
            screen_rows: 16,
        }),
        ..params(60)
    };
    let l = layout_measured(&doc, &measure, &theme.glyphs, p);
    check_invariants(&l, None, "images");
    insta::assert_snapshot!(render(&l));

    // 40×15 cells, cut to 10 rows; the quoted one is cut to the 54 columns inside the bar
    let slots: Vec<_> = l.images.iter().map(|s| (s.link, s.x, s.size)).collect();
    assert_eq!(slots, [(0, 2, Size::new(27, 10)), (1, 4, Size::new(54, 7))]);
    let cat = l.images[0].row as usize;
    assert!(
        l.rows[cat..cat + 10]
            .iter()
            .all(|r| r.kind == RowKind::Image { slot: 0 })
    );
    assert_eq!(l.row_text(cat + 10).trim(), "🖼 a cat");
    assert_eq!(
        l.hits.iter().filter(|h| h.link == 0).count(),
        1,
        "the caption is the link"
    );

    // without images, or before their headers are read, they are lines of text
    let text = layout(&doc, &theme.glyphs, params(60));
    assert!(text.images.is_empty());
    assert!(text.row_text(2).contains("🖼 a cat (cat.png)"));
}

/// Line numbers take columns of their own, left of the margin, and number each line on its
/// first row only.
#[test]
fn line_numbers_take_columns_and_number_first_rows() {
    let theme = Theme::builtin();
    let text: String = (1..=12)
        .map(|i| format!("let x{i} = {}0;\n", "1 + ".repeat(i * 3)))
        .collect();
    let doc = crate::doc::file::code(&text, Some("Rust"));
    let numbered = |width| LayoutParams {
        line_numbers: LineNumbers::Code,
        ..params(width)
    };
    let l = layout(&doc, &theme.glyphs, numbered(40));
    let plain = layout(&doc, &theme.glyphs, params(40));
    assert_eq!((l.numbers, plain.numbers), (3, 0), "two digits and a space");
    assert_eq!(l.params.width, 40, "relayouts compare the width asked for");
    assert!(l.rows.len() > plain.rows.len(), "the rows have less room");
    for (i, row) in l.rows.iter().enumerate() {
        let width: u16 = l.spans(row).map(|(_, s)| s.width).sum();
        assert!(width <= 40 - 3, "row {i} is {width} wide");
    }
    let shown: Vec<u32> = (0..l.row_count())
        .filter_map(|r| l.line_number(r))
        .collect();
    assert_eq!(shown, (1..=12).collect::<Vec<_>>());

    // the document keeps the room the margin needs
    assert_eq!(layout(&doc, &theme.glyphs, numbered(26)).numbers, 0);
    assert_eq!(layout(&doc, &theme.glyphs, numbered(27)).numbers, 3);
}
