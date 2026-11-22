//! Markdown's scanner (X-189): its readings, a fence's language on the run path and the state
//! it carries, the span and run paths leaving the same state, and linear time on long lines.

use super::*;
use crate::tests::marked;

fn md(lines: &[&'static str]) -> Vec<(&'static str, &'static str)> {
    marked(Lang::Markdown, lines)
}

/// The pieces of `lines` whose scopes end in `leaf`.
fn with(lines: &[&'static str], leaf: &str) -> Vec<&'static str> {
    md(lines)
        .into_iter()
        .filter(|(_, s)| s.rsplit(' ').next() == Some(leaf))
        .map(|(t, _)| t)
        .collect()
}

/// `lines` on the run path with a theme that keeps guests ([`crate::tests::guest_runs`]).
fn runs(lines: &[&str]) -> Vec<(String, Lang, String)> {
    crate::tests::guest_runs(Lang::Markdown, lines)
}

#[test]
fn headings() {
    assert_eq!(
        md(&["## Two *em* #"]),
        [
            (
                "##",
                "markup.heading.2.markdown punctuation.definition.heading.begin.markdown"
            ),
            (" ", "markup.heading.2.markdown"),
            (
                "Two ",
                "markup.heading.2.markdown entity.name.section.markdown"
            ),
            (
                "*",
                "markup.heading.2.markdown entity.name.section.markdown markup.italic.markdown \
                 punctuation.definition.italic.begin.markdown"
            ),
            (
                "em",
                "markup.heading.2.markdown entity.name.section.markdown markup.italic.markdown"
            ),
            (
                "*",
                "markup.heading.2.markdown entity.name.section.markdown markup.italic.markdown \
                 punctuation.definition.italic.end.markdown"
            ),
            (" ", "markup.heading.2.markdown"),
            (
                "#",
                "markup.heading.2.markdown punctuation.definition.heading.end.markdown"
            ),
        ]
    );
    assert_eq!(
        with(&["### Three"], "entity.name.section.markdown"),
        ["Three"]
    );
    // seven `#`s, or none of a blank after them, are text
    assert!(md(&["####### seven"]).is_empty());
    assert!(md(&["#hashtag"]).is_empty());
    // a setext underline after a paragraph's line; a thematic break after none
    assert_eq!(
        with(
            &["Title", "====="],
            "punctuation.definition.heading.setext.markdown"
        ),
        ["====="]
    );
    assert_eq!(
        with(
            &["", "---"],
            "punctuation.definition.thematic-break.markdown"
        ),
        ["---"]
    );
    assert_eq!(
        with(&["- - -"], "punctuation.definition.thematic-break.markdown"),
        ["-", "-", "-"]
    );
}

#[test]
fn lists_and_quotes() {
    let bullet = "punctuation.definition.list_item.markdown";
    assert_eq!(with(&["- a", "* b", "+ c"], bullet), ["-", "*", "+"]);
    assert_eq!(with(&["1. a", "2) b"], bullet), [".", ")"]);
    assert_eq!(
        with(&["10. ten"], "markup.list.numbered.bullet.markdown"),
        ["10"]
    );
    // not a list: no blank after the marker, or ten digits
    assert!(md(&["-x", "1234567890. x"]).is_empty());
    assert_eq!(
        with(
            &["> a", "> > b"],
            "punctuation.definition.blockquote.markdown"
        ),
        [">", ">", ">"]
    );
    // a list item's text is a block of its own
    assert_eq!(
        with(&["- # head"], "entity.name.section.markdown"),
        ["head"]
    );
}

#[test]
fn indented_code() {
    let raw = "markup.raw.block.markdown";
    assert_eq!(with(&["", "    code"], raw), ["    code"]);
    // not after a paragraph's line: its continuation
    assert!(with(&["text", "    more"], raw).is_empty());
    // in a list item, four past its content's column
    assert!(with(&["- item", "", "  more"], raw).is_empty());
    assert_eq!(with(&["- item", "", "      code"], raw), ["      code"]);
}

#[test]
fn inline() {
    assert_eq!(
        with(&["a `x` and ``y ` z``"], "markup.raw.inline.markdown"),
        ["x", "y ` z"]
    );
    assert_eq!(
        with(&["**b** __c__ *i* _j_"], "markup.bold.markdown"),
        ["b", "c"]
    );
    assert_eq!(
        with(&["**b** __c__ *i* _j_"], "markup.italic.markdown"),
        ["i", "j"]
    );
    // `_` inside a word is no emphasis; `*` is
    assert!(with(&["snake_case_name"], "markup.italic.markdown").is_empty());
    assert_eq!(with(&["2*3*4"], "markup.italic.markdown"), ["3"]);
    // `*` between blanks is no opener
    assert!(with(&["2 * 3 * 4"], "markup.italic.markdown").is_empty());
    assert_eq!(
        with(&["~~gone~~"], "markup.strikethrough.markdown-gfm"),
        ["gone"]
    );
    assert_eq!(
        with(&[r"\* not \a"], "constant.character.escape.markdown"),
        [r"\*"]
    );
    assert_eq!(
        with(
            &["&amp; &#123; &#x1F; &nope"],
            "constant.character.entity.named.html"
        ),
        ["amp"]
    );
    // bold and a code span across a paragraph's lines, ended by a blank line
    let open = ["**open", "still", "", "plain"];
    assert_eq!(with(&open, "markup.bold.markdown"), ["open", "still"]);
    let code = ["`open", "still`", "plain"];
    assert_eq!(with(&code, "markup.raw.inline.markdown"), ["open", "still"]);
}

#[test]
fn links() {
    let url = "markup.underline.link.markdown";
    assert_eq!(
        with(&[r#"[a [b] c](http://x.y/(z) "t") ![i](p.png)"#], url),
        ["http://x.y/(z)"]
    );
    assert_eq!(
        with(&["![i](p.png)"], "markup.underline.link.image.markdown"),
        ["p.png"]
    );
    assert_eq!(
        with(
            &[r#"[t](u "title")"#],
            "string.other.link.description.title.markdown"
        ),
        ["title"]
    );
    assert_eq!(
        with(
            &["[t][ref] [u][]"],
            "constant.other.reference.link.markdown"
        ),
        ["ref"]
    );
    // a badge: an image in a link's text
    assert_eq!(
        with(
            &["[![b](https://img)](https://site)"],
            "markup.underline.link.markdown"
        ),
        ["https://site"]
    );
    assert_eq!(
        with(&["<https://a.b> https://c.d/e. www.f.g x@y.com"], url),
        ["https://a.b", "https://c.d/e", "www.f.g", "x@y.com"]
    );
    // a definition: its label, its destination and title
    let def = md(&[r#"[ref]: https://x.y "T""#]);
    assert!(
        def.contains(&("ref", "entity.name.reference.link.markdown")),
        "{def:?}"
    );
    assert!(def.contains(&("https://x.y", url)), "{def:?}");
    // not a definition after a paragraph's line
    assert!(with(&["text", "[ref]: x"], "entity.name.reference.link.markdown").is_empty());
}

#[test]
fn tables() {
    let sep = "punctuation.separator.table-cell.markdown";
    // a header without its outer pipes is read from its delimiter row on
    assert_eq!(with(&["a | b", "--|--", "1 | 2"], sep), ["|", "|"]);
    assert_eq!(with(&["| a |"], sep), ["|", "|"]);
    // a pipe in prose
    assert!(with(&["", "a | b"], sep).is_empty());
    assert_eq!(
        with(
            &["|:-|"],
            "punctuation.definition.table-cell-alignment.markdown"
        ),
        [":"]
    );
}

#[test]
fn html_and_comments() {
    let tag = "entity.name.tag.block.any.html";
    assert_eq!(
        with(&["<div align=\"c\">", "x", "</div>"], tag),
        ["div", "div"]
    );
    assert_eq!(
        with(&["a <span>b</span>"], "entity.name.tag.inline.any.html"),
        ["span", "span"]
    );
    // an HTML block ends at a blank line, and markdown is read again
    assert_eq!(
        with(&["<details>", "", "*x*"], "markup.italic.markdown"),
        ["x"]
    );
    let c = "comment.block.html";
    assert_eq!(with(&["a <!-- b", "c --> *d*"], c), [" b", "c "]);
    assert_eq!(
        with(&["a <!-- b", "c --> *d*"], "markup.italic.markdown"),
        ["d"]
    );
}

#[test]
fn fences() {
    let r = runs(&["```rust", "let s = \"a", "b\";", "```", "*x*"]);
    // the string open across the fence's lines
    assert!(
        r.contains(&(
            "b".to_owned(),
            Lang::Rust,
            "string.quoted.double.rust".to_owned()
        )),
        "{r:?}"
    );
    assert!(
        r.contains(&(
            "x".to_owned(),
            Lang::Markdown,
            "markup.italic.markdown".to_owned()
        )),
        "{r:?}"
    );
    // a longer fence holds a shorter one; tildes, an indent, a quote, a list item
    let r = runs(&["````", "```", "````", "~~~ py", "x = 1", "~~~"]);
    assert!(r.iter().any(|(_, l, _)| *l == Lang::Python), "{r:?}");
    let r = runs(&["> ```js", "> var x", "> ```"]);
    assert!(
        r.iter()
            .any(|(t, l, _)| t == "var" && *l == Lang::JavaScript),
        "{r:?}"
    );
    let r = runs(&["1. a", "", "   ```sh", "   ls -l", "   ```"]);
    assert!(
        r.iter().any(|(t, l, _)| t == "ls" && *l == Lang::Bash),
        "{r:?}"
    );
    // a quote's fence ends with the quote
    let r = runs(&["> ```js", "var x"]);
    assert!(!r.iter().any(|(_, l, _)| *l == Lang::JavaScript), "{r:?}");
    // an info string in braces, R Markdown's
    let r = runs(&["```{python echo=FALSE}", "x = 1", "```"]);
    assert!(r.iter().any(|(_, l, _)| *l == Lang::Python), "{r:?}");
    // a fence without a language is one raw kind, on the span path too
    assert_eq!(
        with(&["```", "x", "```"], "markup.raw.code-fence.markdown-gfm"),
        ["x"]
    );
    assert_eq!(
        with(
            &["```rust", "fn", "```"],
            "markup.raw.code-fence.markdown-gfm"
        ),
        ["fn"]
    );
    // an unknown language's name is still its name
    assert_eq!(
        with(&["```klingon"], "constant.other.language-name.markdown"),
        ["klingon"]
    );
}

/// Markdown in a markdown fence: its own fence closes, and the outer one after it.
#[test]
fn markdown_in_markdown() {
    let r = runs(&[
        "````md", "# h", "```rust", "fn", "```", "*i*", "````", "**b**",
    ]);
    let md = |t: &str, leaf: &str| {
        r.iter()
            .any(|(x, l, s)| x == t && *l == Lang::Markdown && s == leaf)
    };
    assert!(md("h", "entity.name.section.markdown"), "{r:?}");
    assert!(md("i", "markup.italic.markdown"), "{r:?}");
    assert!(md("b", "markup.bold.markdown"), "{r:?}");
    assert!(
        r.iter().any(|(x, l, _)| x == "fn" && *l == Lang::Rust),
        "{r:?}"
    );
}

#[test]
fn front_matter() {
    let r = runs(&["---", "title: x", "---", "# h"]);
    assert!(
        r.iter().any(|(t, l, _)| t == "title" && *l == Lang::Yaml),
        "{r:?}"
    );
    assert!(
        r.iter()
            .any(|(t, _, s)| t == "h" && s == "entity.name.section.markdown"),
        "{r:?}"
    );
    let r = runs(&["+++", "a = 1", "+++"]);
    assert!(r.iter().any(|(_, l, _)| *l == Lang::Toml), "{r:?}");
    // only on the first line
    let r = runs(&["", "---", "title: x"]);
    assert!(!r.iter().any(|(_, l, _)| *l == Lang::Yaml), "{r:?}");
}

/// A fence's language that leaves one of markdown's bytes set loses its state there: its lines
/// are lexed from a fresh one each, and the fence still closes.
#[test]
fn a_lost_state_still_closes() {
    let deep = "(".repeat(64) + "{".repeat(64).as_str();
    let lines = ["```tsx", deep.as_str(), "x", "```", "*i*"];
    let r = runs(&lines);
    assert!(
        r.iter().any(|(t, l, _)| t == "i" && *l == Lang::Markdown),
        "{r:?}"
    );
    let mut st = State::default();
    let mut spans = Vec::new();
    for l in &lines[..2] {
        crate::lex(Lang::Markdown, l, &mut st, &mut spans);
    }
    let b = st.bytes();
    assert_eq!(b[B0] & MODE, M_FENCE);
}

/// Every opener at a line's end, alone and after text.
#[test]
fn openers_at_a_lines_end() {
    for unit in [
        "\\", "`", "``", "*", "_", "~~", "[", "![", "<", "</", "<!--", "<a", "<a b=\"", "&", "&#",
        "&#x", ":", "://", "http://", "@", ".", "www.", "|", "](", "[a](", "[a][", "#", "> ", ">",
        "- ", "1.", "```", "~~~", "<div", "[x]:",
    ] {
        for line in [unit.to_owned(), format!("a {unit}")] {
            let _ = runs(&[line.as_str(), line.as_str(), ""]);
        }
    }
}

/// Each construct repeated on one long line lexes in linear time (the guide's mistake 17).
#[test]
fn long_lines_are_linear() {
    for unit in [
        "[", "](", "[a](", "[a](b \"", "[a](b '", "![", "<a x=\"", "<a x='", "<a ", "</a", "`",
        "`` ", "*", "**a", "_a ", "~~", "&", "&#", "http://", "www.", "a@", "<!--", "|", "\\",
        "<http:", "[x][", ":", "a.",
    ] {
        let line = unit.repeat(20_000);
        let t = std::time::Instant::now();
        let _ = runs(&[&line, &line]);
        assert!(t.elapsed().as_secs() < 2, "{unit:?}: {:?}", t.elapsed());
    }
}
