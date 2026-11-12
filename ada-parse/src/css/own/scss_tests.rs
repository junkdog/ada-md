//! SCSS's readings on CSS's scanner (X-149).

use super::*;

/// The pieces of `lines`, as `(text, scopes)`, lexed a line at a time as SCSS, after checking
/// the spans' order.
fn of(lines: &[&str]) -> Vec<(String, String)> {
    let (mut spans, mut state, mut v) = (Vec::new(), State::default(), Vec::new());
    for line in lines {
        spans.clear();
        scss_line(line.as_bytes(), &mut state, &mut Out::new(&mut spans));
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
                scss_scopes(s.kind).join(" "),
            )
        }));
    }
    v
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

const PROP: &str = "support.type.property-name.css";
const CLASS: &str = "entity.other.attribute-name.class.css";
const VAR: &str = "variable.other.sass";
const DECL: &str = "variable.declaration.sass";
const PARAM: &str = "variable.parameter.sass";
const UNQ: &str = "string.unquoted.css";
const OP: &str = "keyword.operator.sass";
const ARITH: &str = "keyword.operator.arithmetic.sass";

#[test]
fn variables() {
    let l = [
        "$spacer: 1rem !default;",
        ".a { $local: $spacer * 2 !global; m: $local; }",
    ];
    has(&l, "spacer", DECL);
    has(
        &l,
        "$",
        "variable.declaration.sass punctuation.definition.variable.sass",
    );
    has(&l, "1", "constant.numeric.value.css");
    has(&l, "rem", "constant.numeric.suffix.css");
    has(&l, "!default", "keyword.other.sass");
    has(&l, "!global", "keyword.other.sass");
    has(&l, "local", DECL);
    has(&l, "spacer", VAR);
    has(&l, "*", ARITH);
    has(&l, "m", PROP);
    has(&l, "local", VAR);
    // a module's variable read and set
    let l = ["a { b: math.$pi; }", "cfg.$x: 1;"];
    has(&l, "math", "variable.other.sass entity.name.namespace.sass");
    has(
        &l,
        ".",
        "variable.other.sass punctuation.separator.namespace.sass",
    );
    has(&l, "pi", VAR);
    has(&l, "cfg", "variable.other.sass entity.name.namespace.sass");
    has(&l, "x", DECL);
}

#[test]
fn maps_and_lists() {
    let l = [
        "$grid: (",
        "  xs: 0,",
        "  'sm': 576px,",
        "  auto: (a: #fff, $v: red),",
        ") !default;",
        "$list: 1px 2px (3px, 4px);",
    ];
    has(&l, "xs", PARAM);
    has(&l, "auto", PARAM);
    has(&l, "a", PARAM);
    has(&l, "sm", "string.quoted.single.css");
    has(&l, "v", VAR);
    has(&l, "(", "punctuation.definition.group.begin.css");
    has(&l, ")", "punctuation.definition.group.end.css");
    has(&l, ":", "punctuation.separator.key-value.css");
    has(&l, "fff", "constant.other.color.rgb-value.css");
}

#[test]
fn interpolation() {
    let l = [
        ".btn-#{$color}-lg, #{$parent}--mod, %ph-#{$x} { margin-#{$side}: 0; }",
        "a { --#{$prefix}gap: #{$gap}; width: calc(100% - #{$a * 2}); content: \"a#{$b}c\"; }",
        "@media #{$mq} and (min-width: $bp) { a { b: c } }",
        "a { padding: var(--#{$p}x) #{$i}px; }",
    ];
    has(&l, "btn-", CLASS);
    has(&l, "-lg", CLASS);
    has(&l, "--mod", "entity.other.attribute-name.css");
    has(&l, "ph-", "entity.other.attribute-name.placeholder.sass");
    has(&l, "#", "punctuation.definition.variable.sass");
    has(&l, "{", "punctuation.definition.group.begin.sass");
    has(&l, "}", "punctuation.definition.group.end.sass");
    has(&l, "margin-", PROP);
    has(&l, "side", VAR);
    has(
        &l,
        "--",
        "support.type.custom-property.css punctuation.definition.custom-property.css",
    );
    has(
        &l,
        "gap",
        "support.type.custom-property.css support.type.custom-property.name.css",
    );
    has(&l, "a", "string.quoted.double.css");
    has(&l, "c", "string.quoted.double.css");
    has(&l, "b", VAR);
    has(&l, "mq", VAR);
    has(&l, "min-width", "support.type.property-name.media.css");
    has(&l, "bp", VAR);
    has(&l, "px", UNQ);
    has(
        &l,
        "x",
        "support.type.custom-property.css support.type.custom-property.name.css",
    );
    // a URL's text after an interpolation in it
    let urls = ["a { b: url(#{$p}/img.png) url($u) url(a#{$b}c.png); }"];
    has(&urls, "/img.png", UNQ);
    has(&urls, "u", VAR);
    has(&urls, "a", UNQ);
    has(&urls, "c.png", UNQ);
    // after the string's interpolation, its text and its end
    has(
        &l,
        "\"",
        "string.quoted.double.css punctuation.definition.string.end.css",
    );
}

#[test]
fn comments() {
    let l = [
        "// a line comment { not: a block }",
        "a { b: c; // after a value",
        "  background: url(//cdn.x/y.png); }",
    ];
    has(
        &l,
        "//",
        "comment.line.double-slash.sass punctuation.definition.comment.css",
    );
    has(
        &l,
        " a line comment { not: a block }",
        "comment.line.double-slash.sass",
    );
    has(&l, "//cdn.x/y.png", UNQ);
    has(&l, "background", PROP);
    // the comment's `{` opened nothing: the next line's rule closes as it should
    let l = ["a { b: c } // {", "d { e: f }"];
    has(&l, "d", "entity.name.tag.css");
    has(&l, "e", PROP);
}

#[test]
fn at_rules() {
    let l = [
        "@use \"sass:math\" as m;",
        "@use 'a' as *; @forward \"b\" show f, $g hide h;",
        "@use 'c' with ($d: 1, $e: 2);",
        "@mixin bp($name, $map: $grid) { @content; }",
        "@function f($x) { @return math.div($x, 2) * 1%; }",
        "@include ns.mx(1px, $b: 2) { c: d; }",
        "@include bp(md);",
        "@each $k, $v in $map { .a-#{$k} { b: $v; } }",
        "@for $i from 1 through 3 { } @for $j from 1 to 3 { }",
        "@if $a == b and not $c { } @else if $a != c { } @else { }",
        "@while $i > 0 { $i: $i - 1; }",
        "@debug $x; @warn 'w'; @error \"e\";",
        ".a { @extend %ph !optional; @at-root .b { c: d } }",
    ];
    let at = |s: &str| format!("keyword.control.at-rule.{s}.sass");
    has(&l, "use", &at("module"));
    has(&l, "forward", &at("module"));
    has(
        &l,
        "@",
        "keyword.control.at-rule.module.sass punctuation.definition.keyword.sass",
    );
    has(&l, "as", "keyword.control.as.sass");
    has(&l, "*", "constant.language.import-all.sass");
    has(&l, "show", "keyword.control.show-hide.sass");
    has(&l, "hide", "keyword.control.show-hide.sass");
    has(&l, "with", "keyword.control.with.sass");
    has(&l, "d", VAR);
    has(&l, "mixin", "keyword.control.at-rule.sass");
    has(&l, "function", "keyword.control.at-rule.sass");
    has(&l, "bp", "entity.name.function.sass");
    has(&l, "f", "entity.name.function.sass");
    has(&l, "name", PARAM);
    has(&l, "map", PARAM);
    has(&l, "grid", VAR);
    has(&l, "content", &at("content"));
    has(&l, "return", &at("return"));
    has(
        &l,
        "math",
        "support.function.sass entity.name.namespace.sass",
    );
    has(&l, "div", "support.function.sass");
    has(
        &l,
        "include",
        "variable.function.sass keyword.control.at-rule.sass",
    );
    has(
        &l,
        "ns",
        "variable.function.sass entity.name.namespace.sass",
    );
    has(&l, "mx", "variable.function.sass");
    has(&l, "bp", "variable.function.sass");
    has(&l, "md", UNQ);
    has(&l, "c", PROP);
    has(&l, "each", "keyword.control.flow.sass");
    has(&l, "in", OP);
    has(&l, "from", OP);
    has(&l, "through", OP);
    has(&l, "to", OP);
    has(&l, "if", "keyword.control.flow.conditional.sass");
    has(&l, "else if", "keyword.control.flow.conditional.sass");
    has(&l, "else", "keyword.control.flow.conditional.sass");
    has(&l, "==", OP);
    has(&l, "!=", OP);
    has(&l, ">", OP);
    has(&l, "and", OP);
    has(&l, "not", OP);
    has(&l, "-", ARITH);
    has(&l, "debug", &at("debugger"));
    has(&l, "warn", &at("warn"));
    has(&l, "error", &at("warn"));
    has(&l, "extend", &at("extend"));
    has(&l, "ph", "entity.other.attribute-name.placeholder.sass");
    has(&l, "!optional", "keyword.other.sass");
    has(&l, "at-root", &at("at-root"));
    has(&l, "b", CLASS);
}

#[test]
fn selectors_and_nesting() {
    let l = [
        ".a {",
        "  &:hover:not(.x) { b: c; }",
        "  &__el, &-lg { d: e; }",
        "  > .icon { fill: currentColor; }",
        "  .x &, & + & { left: 0; }",
        "  inset-inline: 0;",
        "  font: { family: Foo; size: 1px; }",
        "}",
        "%clearfix { &::after { content: ''; } }",
    ];
    has(&l, "&", "keyword.operator.ampersand.css");
    has(&l, "hover", "entity.other.pseudo-class.css");
    has(&l, "b", PROP);
    has(&l, "__el", "entity.other.attribute-name.css");
    has(&l, "-lg", "entity.other.attribute-name.css");
    has(&l, "icon", CLASS);
    has(&l, "inset-inline", PROP);
    has(&l, "family", PROP);
    has(&l, "size", PROP);
    has(&l, "Foo", UNQ);
    has(
        &l,
        "clearfix",
        "entity.other.attribute-name.placeholder.sass",
    );
    has(
        &l,
        "%",
        "entity.other.attribute-name.placeholder.sass punctuation.definition.entity.sass",
    );
    has(&l, "after", "entity.other.pseudo-element.css");
}

#[test]
fn values() {
    let l = [
        "a { margin: -$x (-$y) 0 - 2px; z: $a + 1; w: 10px -5px; v: $a % 2;",
        "  c: if($min != 0, $min, null) true false; d: f($args...); e: 1px/2; }",
    ];
    has(&l, "-", ARITH);
    has(&l, "+", ARITH);
    has(&l, "%", ARITH);
    has(&l, "/", ARITH);
    // a sign before a number is the number's (bat's `invalid.illegal.operator`, not followed)
    has(&l, "-5", "constant.numeric.value.css");
    // right after a value, a sign is an operator, as Sass reads it
    let signs = ["a { b: 1px+2px (1)-2 $a -1; }"];
    has(&signs, "+", ARITH);
    has(&signs, "-", ARITH);
    has(&signs, "-1", "constant.numeric.value.css");
    // CSS's `if()` is a function of CSS's list
    has(&l, "if", "support.function.var.css");
    has(&l, "f", "support.function.sass");
    has(&l, "null", "constant.language.sass");
    has(&l, "true", "constant.language.sass");
    has(&l, "...", OP);
    lacks(&l, "invalid.illegal.operator.css");
}

#[test]
fn front_matter() {
    let l = ["---", "title: x", "---", "a { b: c }"];
    has(
        &l,
        "---",
        "frontmatter.jekyll punctuation.section.frontmatter.begin.jekyll",
    );
    has(
        &l,
        "---",
        "frontmatter.jekyll punctuation.section.frontmatter.end.jekyll",
    );
    has(
        &l,
        "title",
        "frontmatter.jekyll source.yaml string.unquoted.plain.out.yaml entity.name.tag.yaml",
    );
    has(&l, "b", PROP);
}

#[test]
fn a_block_lexes_as_its_lines() {
    let text = [
        "---",
        "x: [1, 2]",
        "---",
        "@use 'a' with (",
        "  $b: 1,",
        ");",
        ".a-#{$b} {",
        "  /* open",
        "   */ $c: \"x#{",
        "    $d}y\";",
        "  @include m(",
        "    $e: 2",
        "  ) { f: g; }",
        "  font: {",
        "    family: h;",
        "  }",
        "}",
        // an interpolation closed on a later line: the name it broke isn't carried
        ".p-#{$a",
        "}-q, .r-#{$b}-s { t: u }",
    ];
    let mut lines = Vec::new();
    let mut state = State::default();
    for t in text {
        let mut spans = Vec::new();
        scss_line(t.as_bytes(), &mut state, &mut Out::new(&mut spans));
        lines.push((spans, state));
    }
    let mut spans = Vec::new();
    let mut i = 0;
    let mut st = State::default();
    scss_block(&mut st, &mut spans, &mut |c| {
        for t in text {
            assert_eq!(c.next(t.as_bytes()), &lines[i].0[..], "line {i}: {t}");
            i += 1;
        }
        None
    });
    assert_eq!(st, lines.last().unwrap().1);
}

#[test]
fn long_lines_lex_in_linear_time() {
    for unit in [
        "a-",
        "$a ",
        "#{",
        "(",
        "a:",
        "1px+",
        "#{a}-",
        "@include a(",
        "\"#{",
        "a:b ",
        "%a",
    ] {
        let small: String = unit.repeat(1000);
        let big: String = unit.repeat(8000);
        let t = std::time::Instant::now();
        let _ = of(&[small.as_str()]);
        let one = t.elapsed();
        let t = std::time::Instant::now();
        let _ = of(&[big.as_str()]);
        let eight = t.elapsed();
        assert!(
            eight < one * 40 + std::time::Duration::from_millis(20),
            "{unit:?}: {one:?} then {eight:?}"
        );
    }
}

#[test]
fn deep_nesting_keeps_its_spans_in_order() {
    // past the 25 levels SCSS's frames keep: the outer ones lost, the spans still in order
    let line = format!("$x: f({} res.text() }}", "(".repeat(200));
    let _ = of(&[line.as_str()]);
    let line = format!("a {{ b: {} }}", "#{".repeat(60));
    let _ = of(&[line.as_str()]);
    let deep: Vec<String> = (0..40).map(|i| format!(".a{i} {{ $v: \"#{{")).collect();
    let deep: Vec<&str> = deep.iter().map(String::as_str).collect();
    let _ = of(&deep);
}

#[test]
fn a_loose_end_moves_on() {
    for line in [
        "\\", "a\\", "$", "#", "#{", "%", "@", "@include", "@mixin", "\"#{", "/", "&", "!", "...",
    ] {
        let _ = of(&[line, line]);
    }
}
