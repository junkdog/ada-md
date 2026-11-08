use super::*;
use crate::{Span, State};

/// Each line's tokens of C++ (or C) as (text, scopes), lexed from the default state, each
/// line's spans checked: in order, not empty, inside the line.
fn lexed(c: bool, lines: &[&str]) -> Vec<(String, String)> {
    let mut state = State::default();
    let mut got = Vec::new();
    for l in lines {
        let mut spans: Vec<Span> = Vec::new();
        let mut out = Out::new(&mut spans);
        if c {
            line_c(l.as_bytes(), &mut state, &mut out);
        } else {
            line(l.as_bytes(), &mut state, &mut out);
        }
        let mut end = 0;
        for s in &spans {
            assert!(
                s.start >= end && s.end > s.start && s.end as usize <= l.len(),
                "{spans:?} on {l:?}"
            );
            end = s.end;
            let scopes = if c { scopes_c(s.kind) } else { scopes(s.kind) };
            got.push((
                l[s.start as usize..s.end as usize].to_owned(),
                scopes.join(" "),
            ));
        }
    }
    got
}

fn has(c: bool, lines: &[&str], text: &str, scopes: &str) {
    let got = lexed(c, lines);
    assert!(
        got.iter().any(|(t, s)| t == text && s == scopes),
        "{text:?} as {scopes:?} in {got:#?}"
    );
}

fn plain(c: bool, lines: &[&str], text: &str) {
    let got = lexed(c, lines);
    assert!(
        !got.iter().any(|(t, _)| t == text),
        "{text:?} not plain in {got:#?}"
    );
}

#[test]
fn words_and_calls() {
    let l = ["int x = foo(1) + obj.bar(2) + p->q + printf(\"%d\\n\", n) + kMax + m_v;"];
    has(false, &l, "int", "storage.type.c++");
    has(false, &l, "foo", "variable.function.c++");
    has(false, &l, "bar", "variable.function.member.c++");
    has(false, &l, "q", "variable.other.readwrite.member.c++");
    has(false, &l, "printf", "support.function.C99.c++");
    has(
        false,
        &l,
        "%d",
        "string.quoted.double.c++ constant.other.placeholder.c++",
    );
    has(
        false,
        &l,
        "\\n",
        "string.quoted.double.c++ constant.character.escape.c++",
    );
    has(false, &l, "kMax", "constant.other.variable.mac-classic.c++");
    has(false, &l, "m_v", "variable.other.readwrite.member.c++");
    plain(false, &l, "x");
    plain(false, &l, "n");
    // C hasn't C++'s words nor its member shapes
    let l = ["class this; m_v = new_thing;"];
    plain(true, &l, "class");
    plain(true, &l, "this");
    plain(true, &l, "m_v");
    has(false, &l, "class", "storage.type.c++");
    has(false, &l, "this", "variable.language.c++");
}

#[test]
fn numbers() {
    let l = ["x = 0x1Fu + 1.5e3f + 0777 + 0b101 + 1'000ull + .5;"];
    has(
        false,
        &l,
        "0x",
        "constant.numeric.integer.hexadecimal.c++ punctuation.definition.numeric.base.c++",
    );
    has(false, &l, "1F", "constant.numeric.integer.hexadecimal.c++");
    has(
        false,
        &l,
        "u",
        "constant.numeric.integer.hexadecimal.c++ storage.type.numeric.c++",
    );
    has(false, &l, "5e3", "constant.numeric.float.decimal.c++");
    has(
        false,
        &l,
        "f",
        "constant.numeric.float.decimal.c++ storage.type.numeric.c++",
    );
    has(false, &l, "777", "constant.numeric.integer.octal.c++");
    has(false, &l, "101", "constant.numeric.integer.binary.c++");
    has(false, &l, "1'000", "constant.numeric.integer.decimal.c++");
    has(
        false,
        &l,
        "ull",
        "constant.numeric.integer.decimal.c++ storage.type.numeric.c++",
    );
}

#[test]
fn comments_and_strings_across_lines() {
    let l = [
        "a /* one",
        "two */ b; // x \\",
        "still",
        "c = \"s\\",
        "t\";",
    ];
    has(
        false,
        &l,
        "/*",
        "comment.block.c++ punctuation.definition.comment.c++",
    );
    has(false, &l, "two ", "comment.block.c++");
    let m = ["/*", " * one", " */", "a->b = c.d;"];
    has(
        false,
        &m,
        "*",
        "comment.block.c++ punctuation.definition.comment.c++",
    );
    has(false, &m, "b", "variable.other.readwrite.member.c++");
    plain(true, &m, "b");
    has(true, &m, "d", "variable.other.readwrite.member.c");
    has(false, &l, "still", "comment.line.double-slash.c++");
    has(false, &l, "t", "string.quoted.double.c++");
    // a string without a `\` ends at its line's end
    let l = ["s = \"open", "int x;"];
    has(false, &l, "int", "storage.type.c++");
    let l = ["auto r = R\"xy(raw ) \"body", "more)xy\";"];
    has(
        false,
        &l,
        "R",
        "string.quoted.double.c++ storage.type.string.c++",
    );
    has(false, &l, "raw ) \"body", "string.quoted.double.c++");
    has(
        false,
        &l,
        ")xy\"",
        "string.quoted.double.c++ punctuation.definition.string.end.c++",
    );
}

#[test]
fn colons() {
    let l = [
        "class Foo : public Bar {",
        "public:",
        "  Foo() : a(1) {}",
        "};",
        "void g() { switch (x) { case 1: y = c ? 1 : 2; } label: ; }",
    ];
    has(false, &l, ":", "punctuation.section.class.c++");
    has(false, &l, ":", "punctuation.separator.initializer-list.c++");
    has(false, &l, ":", "punctuation.separator.c++");
    has(false, &l, ":", "keyword.operator.ternary.c++");
    has(false, &l, "label", "entity.name.label.c++");
}

#[test]
fn directives() {
    let l = [
        "#include <stdio.h> // c",
        "#define BAR(a, b) ((a) + (b)) \\",
        "  * 2",
        "#if defined(X) && X > 2",
        "#error \"no\" don't",
        "#if 0",
        "int dead;",
        "#if 1",
        "#endif",
        "#else",
        "int live;",
        "#endif",
    ];
    has(false, &l, "#include", "keyword.control.import.include.c++");
    has(
        false,
        &l,
        "stdio.h",
        "string.quoted.other.lt-gt.include.c++",
    );
    has(false, &l, " ", "meta.preprocessor.c++");
    has(false, &l, " c", "comment.line.double-slash.c++");
    has(false, &l, "BAR", "entity.name.function.preprocessor.c++");
    has(false, &l, "a", "variable.parameter.c++");
    has(false, &l, "  ", "meta.preprocessor.c++");
    has(false, &l, "2", "constant.numeric.integer.decimal.c++");
    has(false, &l, "defined", "keyword.control.c++");
    has(false, &l, "(X) && X > 2", "meta.preprocessor.c++");
    has(false, &l, "don't", "string.unquoted.c++");
    has(
        false,
        &l,
        "int dead;",
        "comment.block.preprocessor.if-branch.c++",
    );
    has(
        false,
        &l,
        "#if 1",
        "comment.block.preprocessor.if-branch.c++",
    );
    has(false, &l, "#else", "keyword.control.import.else.c++");
    has(false, &l, "int", "storage.type.c++");
}

#[test]
fn braces_and_state() {
    let mut state = State::default();
    let mut spans = Vec::new();
    for l in ["namespace a {", "class B {", "void f() {", "  { x; }"] {
        line(l.as_bytes(), &mut state, &mut Out::new(&mut spans));
    }
    assert_eq!(state.depth, 3);
    {
        let mut out = Out::new(&mut spans);
        let mut s = Scan::new(b"", &mut out, false, Spans::new());
        s.load(&state);
        // innermost first: the function's body, the class's, the namespace's
        assert_eq!(s.braces & 0x3f, u128::from(CODE | CLASS << 2 | NS << 4));
    }
    for l in ["}", "};", "}"] {
        line(l.as_bytes(), &mut state, &mut Out::new(&mut spans));
    }
    assert_eq!(state, State::default());
}

#[test]
fn a_block_lexes_as_its_lines() {
    let text = "#define X \\\n  1\nint a; /* b\n c */ s = \"x\\\ny\";\n#if 0\nz\n#endif\n";
    let mut state = State::default();
    let mut one = Vec::new();
    let mut spans = Vec::new();
    for l in text.lines() {
        spans.clear();
        line(l.as_bytes(), &mut state, &mut Out::new(&mut spans));
        one.push((spans.clone(), state));
    }
    let mut state = State::default();
    let mut all = Vec::new();
    let mut v = Vec::new();
    block(&mut state, &mut v, &mut |c| {
        for l in text.lines() {
            all.push(c.next(l.as_bytes()).to_vec());
        }
        None
    });
    assert_eq!(all, one.iter().map(|(s, _)| s.clone()).collect::<Vec<_>>());
    assert_eq!(state, one.last().unwrap().1);
}
