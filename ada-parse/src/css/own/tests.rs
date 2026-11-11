use super::*;
use crate::Lang;
use crate::tests::marked;

fn kinds(lines: &[&'static str]) -> Vec<(&'static str, &'static str)> {
    marked(Lang::Css, lines)
}

/// The marked pieces of `lines`, as `(text, scopes)`, lexed a line at a time on the new
/// scanner, after checking the spans' order.
fn of(lines: &[&str]) -> Vec<(String, String)> {
    let (mut spans, mut state, mut v) = (Vec::new(), State::default(), Vec::new());
    for line in lines {
        spans.clear();
        line_spans(line.as_bytes(), &mut state, &mut spans);
        let mut last = 0;
        for s in &spans {
            assert!(s.start >= last && s.end > s.start, "{line}: {spans:?}");
            assert!(
                line.is_char_boundary(s.start as usize) && line.is_char_boundary(s.end as usize)
            );
            last = s.end;
        }
        assert!(last as usize <= line.len());
        v.extend(spans.iter().map(|s| {
            (
                line[s.start as usize..s.end as usize].to_owned(),
                scopes(s.kind).join(" "),
            )
        }));
    }
    v
}

fn line_spans(b: &[u8], state: &mut State, spans: &mut Vec<crate::Span>) {
    let mut out = Out::new(spans);
    line(b, state, &mut out);
}

#[track_caller]
fn has(lines: &[&str], text: &str, scopes: &str) {
    let got = of(lines);
    assert!(
        got.iter().any(|(t, s)| t == text && s == scopes),
        "{text:?} as {scopes:?} in {got:#?}"
    );
}

#[track_caller]
fn lacks(lines: &[&str], text: &str) {
    let got = of(lines);
    assert!(!got.iter().any(|(t, _)| t == text), "{text:?} in {got:#?}");
}

#[track_caller]
fn plain(lines: &[&str], text: &str) {
    lacks(lines, text);
}

const PROP: &str = "support.type.property-name.css";
const TAG: &str = "entity.name.tag.css";
const CLASS: &str = "entity.other.attribute-name.class.css";
const PC: &str = "entity.other.pseudo-class.css";
const BRACE: &str = "punctuation.section.property-list.css";

#[test]
fn rules_and_declarations() {
    let l = [".card > p, DIV#main { color: red; margin: 0 auto !important }"];
    has(&l, "card", CLASS);
    has(&l, "p", TAG);
    has(&l, "DIV", TAG);
    has(&l, "main", "entity.other.attribute-name.id.css");
    has(&l, ">", "punctuation.separator.combinator.css");
    has(&l, ",", "punctuation.separator.sequence.css");
    has(&l, "color", PROP);
    has(&l, ":", "punctuation.separator.key-value.css");
    has(
        &l,
        "red",
        "support.constant.color.w3c-standard-color-name.css",
    );
    has(&l, ";", "punctuation.terminator.rule.css");
    has(&l, "0", "constant.numeric.integer.decimal.css");
    has(&l, "auto", "support.constant.property-value.css");
    has(&l, "!important", "keyword.other.important.css");
    has(&l, "{", BRACE);
    has(&l, "}", BRACE);
}

#[test]
fn nesting() {
    let l = [
        ".card {",
        "  color: red;",
        "  &:hover { color: blue; }",
        "  & > .title { font-weight: bold; }",
        "  a:hover { top: 0 }",
        "  div p { width: 1px }",
        "  @media (width > 600px) { padding: 2px; }",
        "  inline-size: 1px;",
        "}",
        ".after { color: red }",
    ];
    let got = of(&l);
    for name in [
        "color",
        "font-weight",
        "top",
        "width",
        "padding",
        "inline-size",
    ] {
        assert!(
            got.iter().any(|(t, s)| t == name && s == PROP),
            "{name} in {got:#?}"
        );
    }
    has(&l, "&", "keyword.operator.ampersand.css");
    has(&l, "hover", PC);
    has(&l, "a", TAG);
    has(&l, "div", TAG);
    has(&l, "after", CLASS);
    has(&l, "width", "support.type.property-name.media.css");
    has(&l, ">", "keyword.operator.comparison.css");
    // minified: `a:hover{` a rule, `color:red` a declaration
    let m = [".min{color:red;&:hover{color:blue}a:focus{top:0}}.x{top:0}"];
    has(&m, "focus", PC);
    has(&m, "a", TAG);
    has(&m, "top", PROP);
    has(&m, "x", CLASS);
}

#[test]
fn custom_properties_hold_anything() {
    let l = [
        "a {",
        "  --x: { a: b };",
        "  --json: [1, 2];",
        "  container-type: inline-size;",
        "  color: var(--x, red);",
        "}",
    ];
    has(
        &l,
        "x",
        "support.type.custom-property.css support.type.custom-property.name.css",
    );
    has(
        &l,
        "--",
        "support.type.custom-property.css punctuation.definition.custom-property.css",
    );
    has(&l, "container-type", PROP);
    has(&l, "var", "support.function.var.css");
    // the braces' `}` closed the braces, not the rule
    let got = of(&l);
    let n = got.iter().filter(|(t, s)| t == "}" && s == BRACE).count();
    assert_eq!(n, 2, "{got:#?}");
}

#[test]
fn functions() {
    let l = [
        "a { color: oklch(70% 0.1 200 / 50%); width: clamp(1rem, 2.5vw + 1rem, 3rem); b: url(x.png) }",
    ];
    has(&l, "oklch", "support.function.color.css");
    has(&l, "clamp", "support.function.calc.css");
    has(&l, "+", "keyword.operator.arithmetic.css");
    has(
        &l,
        "rem",
        "constant.numeric.integer.decimal.css keyword.other.unit.css",
    );
    has(
        &l,
        "vw",
        "constant.numeric.float.decimal.css keyword.other.unit.css",
    );
    has(&l, "x.png", "string.unquoted.css");
    has(&l, "(", "punctuation.definition.group.begin.css");
    has(&l, ")", "punctuation.definition.group.end.css");
    has(
        &["a { b: frobnicate(1) }"],
        "frobnicate",
        "support.function.css",
    );
}

#[test]
fn at_rules() {
    has(
        &["@layer reset, base;"],
        "layer",
        "keyword.control.at-rule.layer.css",
    );
    has(
        &["@layer reset, base;"],
        "@",
        "keyword.control.at-rule.layer.css punctuation.definition.keyword.css",
    );
    // a layer's block holds rules
    has(&["@layer base { html { color: red } }"], "html", TAG);
    has(
        &["@container sidebar (min-width: 400px) { .c { top: 0 } }"],
        "min-width",
        "support.type.property-name.media.css",
    );
    has(
        &["@media screen and (prefers-color-scheme: dark) {}"],
        "prefers-color-scheme",
        "support.type.property-name.media.css",
    );
    has(
        &["@media screen and (prefers-color-scheme: dark) {}"],
        "screen",
        "support.constant.media.css",
    );
    has(
        &["@media screen and (prefers-color-scheme: dark) {}"],
        "and",
        "keyword.operator.logic.media.css",
    );
    has(&["@supports not (display: grid) {}"], "display", PROP);
    has(
        &["@supports not (display: grid) {}"],
        "not",
        "keyword.operator.logic.css",
    );
    has(&["@supports selector(:has(a)) {}"], "has", PC);
    // a property's block holds declarations
    let p = [
        "@property --angle {",
        "  syntax: '<angle>';",
        "  inherits: false;",
        "}",
    ];
    has(&p, "syntax", PROP);
    has(&p, "inherits", PROP);
    has(
        &p,
        "angle",
        "support.type.custom-property.css support.type.custom-property.name.css",
    );
    has(
        &["@scope (.card) to (.content) { img { top: 0 } }"],
        "card",
        CLASS,
    );
    has(
        &["@scope (.card) to (.content) { img { top: 0 } }"],
        "img",
        TAG,
    );
    has(
        &["@keyframes spin { from { top: 0 } 50.5% { top: 1px } }"],
        "from",
        "keyword.keyframe-selector.css",
    );
    has(
        &["@keyframes spin { from { top: 0 } 50.5% { top: 1px } }"],
        "spin",
        "entity.other.animation-name.css",
    );
    has(
        &["@page :first { margin: 1in; @top-center { content: 'x' } }"],
        "top-center",
        "keyword.control.at-rule.page-margin.css",
    );
    has(
        &["@tailwind base;"],
        "tailwind",
        "keyword.control.at-rule.css",
    );
    has(
        &["@import url(\"a.css\") layer(x) supports(display: grid) screen;"],
        "display",
        PROP,
    );
}

#[test]
fn selectors() {
    let l = ["a:has(> img), li:nth-child(2n + 1 of .x), [data-x=y s], *|*, svg|circle {}"];
    has(&l, "has", PC);
    has(&l, "img", TAG);
    has(&l, "of", "keyword.other.pseudo-class.css");
    has(&l, "x", CLASS);
    has(&l, "data-x", "entity.other.attribute-name.css");
    has(&l, "y", "string.unquoted.css");
    has(&l, "s", "keyword.other.css");
    has(&l, "svg", "entity.other.namespace-prefix.css");
    has(&l, "|", "punctuation.separator.namespace.css");
    has(
        &["input::-webkit-input-placeholder {}"],
        "-webkit-",
        "entity.other.pseudo-element.css support.type.vendor-prefix.css",
    );
    has(
        &["a:before {}"],
        "before",
        "entity.other.pseudo-element.css",
    );
    has(&["my-el {}"], "my-el", "entity.name.tag.custom.css");
}

#[test]
fn strings_and_comments() {
    let l = ["a { content: \"\\201C\" 'x\\'y'; /* c", "still */ top: 0 }"];
    has(
        &l,
        "\\201C",
        "string.quoted.double.css constant.character.escape.css",
    );
    has(
        &l,
        "\\'",
        "string.quoted.single.css constant.character.escape.css",
    );
    has(&l, "still ", "comment.block.css");
    has(&l, "top", PROP);
    // an unclosed string ends with its line
    let u = ["a { color: \"open", "; top: 0 }"];
    has(&u, "top", PROP);
    // a `\` at a line's end continues it
    let c = ["a { content: \"a\\", "b\"; top: 0 }"];
    has(&c, "b", "string.quoted.double.css");
}

#[test]
fn values() {
    let l = [
        "a { color: #abc #aabbccdd; font-family: Roboto, sans-serif; unicode-range: U+0-7F; grid-template-columns: [full-start] 1fr }",
    ];
    has(&l, "abc", "constant.other.color.rgb-value.css");
    has(&l, "aabbccdd", "constant.other.color.rgba-value.css");
    has(&l, "Roboto", "string.unquoted.css");
    has(&l, "sans-serif", "support.constant.font-name.css");
    has(
        &l,
        "U+",
        "support.unicode-range.css support.constant.unicode-range.prefix.css",
    );
    has(&l, "full-start", "string.unquoted.line-name.css");
    plain(&["a { width: 1px }"], "a ");
}

/// bat's readings kept where they are CSS's (r1): a gradient's keywords, a vendor prefix on a
/// value's keyword, `attr(`'s attribute, a comment's gutter, `@-webkit-keyframes`' prefix, a
/// selector's plain arguments as strings.
#[test]
fn readings_kept_from_bat() {
    let l = [
        "a { b: linear-gradient(to right, red); c: radial-gradient(circle at top, red); d: -moz-max-content; e: attr(data-x) }",
    ];
    has(&l, "to", "keyword.other.css");
    has(&l, "circle", "keyword.other.css");
    has(&l, "at", "keyword.other.css");
    has(&l, "-moz-", "support.type.vendor-prefix.css");
    has(&l, "max-content", "support.constant.property-value.css");
    has(&l, "data-x", "entity.other.attribute-name.css");
    let c = ["/**", " * gutter", " */"];
    has(
        &c,
        "*",
        "comment.block.css punctuation.definition.comment.css",
    );
    has(&c, " gutter", "comment.block.css");
    has(
        &["@-webkit-keyframes x {}"],
        "-webkit-",
        "keyword.control.at-rule.keyframe.css support.type.property-vendor.css",
    );
    has(
        &["@-webkit-keyframes x {}"],
        "keyframes",
        "keyword.control.at-rule.keyframe.css",
    );
    has(&[":lang(en) {}"], "en", "string.unquoted.css");
}

/// The readings snapshot's catches (R4): an at-rule after an unfinished selector (not in a
/// value: GTK's colors), an+b's sign, a counter's name, `local(`'s and `url-prefix(`'s text.
#[test]
fn snapshot_catches() {
    let l = ["@charset \"utf-8\"; x", "@import url(\"a.css\") screen;"];
    has(&l, "import", "keyword.control.at-rule.import.css");
    has(&l, "screen", "support.constant.media.css");
    lacks(&["a { color: @theme_fg_color; }"], "theme_fg_color");
    lacks(&["a { --x: @media; }"], "media");
    has(
        &["li:nth-child(2n + 1) {}"],
        "+",
        "keyword.operator.arithmetic.css",
    );
    has(
        &["a { content: counter(c, upper-roman) }"],
        "c",
        "entity.other.counter-name.css string.unquoted.css",
    );
    has(
        &["@font-face { src: local(Arial Black) }"],
        "Arial Black",
        "string.unquoted.css",
    );
    has(
        &["@document url-prefix(http://x) {}"],
        "http://x",
        "string.unquoted.css",
    );
}

/// A block lexes as its lines do one at a time, the scan's fields carried.
#[test]
fn a_block_lexes_as_its_lines() {
    let text = ".a {\n  color: red;\n  &:hover {\n    --x: { a: b\n  };\n  }\n  /* open\n */\n}\n@media (min-width:\n 1px) { b { content: \"x\\\ny\" } }\n";
    let (mut want, mut state) = (Vec::new(), State::default());
    for l in text.split('\n') {
        let mut spans = Vec::new();
        line_spans(l.as_bytes(), &mut state, &mut spans);
        want.push((spans, state));
    }
    let mut state = State::default();
    let mut spans = Vec::new();
    let mut got = Vec::new();
    block(&mut state, &mut spans, &mut |c| {
        for l in text.split('\n') {
            got.push(c.next(l.as_bytes()).to_vec());
        }
        None
    });
    assert_eq!(got, want.iter().map(|(s, _)| s.clone()).collect::<Vec<_>>());
    assert_eq!(state, want.last().unwrap().1);
}

/// Each construct repeated on one long line lexes in linear time.
#[test]
fn long_lines_lex_in_linear_time() {
    for unit in [
        "a:b",
        "a:b,",
        "a{",
        "x:y;",
        "(",
        "[",
        "\"",
        "/*",
        "url(",
        "@media (",
        ":is(",
        "--x:{",
        "a:hover{b:c}",
        "{a:b",
        "a:b(",
        "#",
        ".",
        "!",
        "U+",
    ] {
        let line = unit.repeat(20_000);
        let start = std::time::Instant::now();
        let mut st = rule_list_body();
        let mut spans = Vec::new();
        line_spans(line.as_bytes(), &mut st, &mut spans);
        assert!(
            start.elapsed().as_secs_f64() < 2.0,
            "{unit:?}: {:?}",
            start.elapsed()
        );
    }
}

/// A `\` at a line's end, in code, in a string continued and in a selector, is passed over (the
/// fuzzer's first finding: the identifier reader read nothing there, and the loop stood still).
#[test]
fn a_backslash_at_the_end_moves_on() {
    for l in [
        &["a\\"][..],
        &["a { b: c\\"],
        &["  \"s\":  \"\\", "  \"\0\\\0s\\"],
        &["\\"],
    ] {
        of(l);
    }
}

// the old scanner's tests (`css/mod.rs` before X-125), read on this one

/// The old scanner's long lines (its memoized runs) stay linear on this one too.
#[test]
fn the_old_scanners_long_lines_stay_linear() {
    let lines = [
        format!("a {{ b: {}", "a-".repeat(30000)),
        format!("a {{ b: {}", "a".repeat(60000)),
        format!("a {{ b: blur({}", "1".repeat(60000)),
        format!("a {{ b: {}{}", "1".repeat(30000), "p".repeat(30000)),
        format!("a {{ b: rgb({}", " ".repeat(60000)),
        format!("a {{ font: {}", "x ".repeat(30000)),
        "a\u{b7}".repeat(20000),
        format!("@media ({}", "1".repeat(60000)),
        format!("@media {}", "a-".repeat(30000)),
        format!("/*{}", "*".repeat(60000)),
        format!("a[{}", "x ".repeat(30000)),
        format!(":nth-child({}", "1 ".repeat(30000)),
        "a-b ".repeat(15000),
        ">".repeat(60000),
    ];
    let t = std::time::Instant::now();
    let mut spans = Vec::new();
    for line in &lines {
        crate::lex(Lang::Css, line, &mut State::default(), &mut spans);
    }
    assert!(t.elapsed().as_secs() < 2, "{:?}", t.elapsed());
}

#[test]
fn a_rule_reads() {
    let m = kinds(&["a.b:hover { color: red; margin: 0 1.5em }"]);
    assert!(m.contains(&("a", "entity.name.tag.css")), "{m:?}");
    assert!(
        m.contains(&("color", "support.type.property-name.css")),
        "{m:?}"
    );
    assert!(
        m.contains(&("red", "support.constant.color.w3c-standard-color-name.css")),
        "{m:?}"
    );
    assert!(
        m.contains(&(
            "em",
            "constant.numeric.float.decimal.css keyword.other.unit.css"
        )),
        "{m:?}"
    );
    // a keyword whole, where bat's `\b(…|auto(hiding-scrollbar|-flow)?|…)\b` stopped at `auto`
    let m = kinds(&["a { b: auto-fill }"]);
    assert!(
        m.contains(&("auto-fill", "support.constant.property-value.css")),
        "{m:?}"
    );
}

/// A block comment and a string continued by an escaped newline carry their state between
/// lines; a string the line's end leaves open closes there.
#[test]
fn state_between_lines() {
    let m = kinds(&["/* a", "b */ x { c: 'd\\", "e' } y { z: \"w", "}"]);
    assert!(m.contains(&("b ", "comment.block.css")), "{m:?}");
    assert!(
        m.contains(&(
            "\\",
            "string.quoted.single.css constant.character.escape.newline.css"
        )),
        "{m:?}"
    );
    assert!(m.contains(&("e", "string.quoted.single.css")), "{m:?}");
    assert!(m.contains(&("w", "string.quoted.double.css")), "{m:?}");
    assert_eq!(
        m.last(),
        Some(&("}", "punctuation.section.property-list.css"))
    );
}

/// No limit of syntect's on nesting (X-133): past its 99 levels a group still opens at
/// every `(`, and a value in it still reads.
#[test]
fn nesting_past_syntects_depth() {
    let line = format!("a {{ width: calc({}1px + 2em) }}", "(".repeat(120));
    let got = marked(Lang::Css, &[&line]);
    // the groups' `(`s, joined into one span where they stand together
    let groups: usize = got
        .iter()
        .filter(|(_, s)| *s == "punctuation.definition.group.begin.css")
        .map(|(t, _)| t.len())
        .sum();
    assert_eq!(groups, 121, "{got:?}");
    assert!(
        got.contains(&(
            "px",
            "constant.numeric.integer.decimal.css keyword.other.unit.css"
        )),
        "{got:?}"
    );
}
