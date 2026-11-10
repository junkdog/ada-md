//! C#'s scanner's tests: its readings (C# 9–12's among them), a block against its lines, long
//! lines in linear time.

use super::*;

/// `text` a line at a time on the line API: each line's spans and the state after it.
fn lines(text: &str) -> Vec<(Vec<crate::Span>, State)> {
    let mut state = State::default();
    let mut out = Vec::new();
    for l in text.lines() {
        let mut spans = Vec::new();
        line(l.as_bytes(), &mut state, &mut Out::new(&mut spans));
        out.push((spans, state));
    }
    out
}

/// The tokens of `text` a line at a time, each as its text and its scopes.
fn toks(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (l, (spans, _)) in text.lines().zip(lines(text)) {
        let mut at = 0;
        for s in spans {
            let (b, e) = (s.start as usize, s.end as usize);
            assert!(b >= at && e > b && e <= l.len(), "{l:?}: {s:?} after {at}");
            at = e;
            out.push((l[b..e].to_string(), scopes(s.kind).join(" ")));
        }
    }
    out
}

/// Whether a token of `text` reads `t` with scopes ending in `leaf`.
#[track_caller]
fn has(text: &str, t: &str, leaf: &str) {
    let all = toks(text);
    assert!(
        all.iter().any(|(x, s)| x == t && s.ends_with(leaf)),
        "{t:?} as {leaf:?} in {text:?}: {all:?}"
    );
}

/// Whether no token of `text` is `t` (it is plain).
#[track_caller]
fn plain(text: &str, t: &str) {
    let all = toks(text);
    assert!(
        !all.iter().any(|(x, _)| x == t),
        "{t:?} plain in {text:?}: {all:?}"
    );
}

#[test]
fn declarations_and_members() {
    let t = "namespace A.B;\n\
             public sealed class Foo<T> : Base<T>, IFoo where T : class, new()\n{\n\
             private readonly int _count = 0, _other;\n\
             public List<string> Names { get; private set; } = new();\n\
             public string this[int i] => _names[i];\n\
             public Foo(int count) : base(count) { }\n\
             public static implicit operator int(Foo f) => f._count;\n\
             protected virtual async Task<int> RunAsync<V>(V v, ref int x, params object[] rest) where V : struct { return 1; }\n\
             int IFoo.Explicit() => 1;\n\
             public delegate void Handler(object sender);\n}\n";
    has(t, "namespace", "storage.type.namespace.cs");
    has(t, "B", "entity.name.namespace.cs");
    has(t, "Foo", "entity.name.class.cs");
    has(t, "Base", "entity.other.inherited-class.cs");
    has(t, "_count", "variable.other.member.cs");
    has(t, "_other", "variable.other.member.cs");
    has(t, "Names", "variable.other.member.cs");
    has(t, "get", "storage.type.function.accessor.get.cs");
    has(t, "this", "variable.language.cs");
    has(t, "i", "variable.parameter.cs");
    has(t, "Foo", "entity.name.function.constructor.cs");
    has(t, "base", "variable.language.cs");
    has(t, "RunAsync", "entity.name.function.cs");
    has(t, "V", "support.type.cs");
    has(t, "rest", "variable.parameter.cs");
    has(t, "params", "storage.modifier.parameter.cs");
    has(t, "IFoo", "entity.other.inherited-class.cs");
    has(t, "Explicit", "entity.name.function.cs");
    has(t, "Handler", "variable.other.member.delegate.cs");
}

#[test]
fn csharp_9_to_12() {
    let t = "public record Person(string First, int Age);\n\
             public record struct Point(int X, int Y);\n\
             public readonly record struct R(int A) : IR { }\n\
             class C { public required string Name { get; init; } }\n\
             file class Hidden { }\n\
             var p2 = p with { Age = 3 };\n\
             if (x is [1, 2, .. var rest]) { }\n\
             if (o is not null and > 3 or < -1) { }\n\
             if (o is { Length: > 0 } s) { }\n\
             Point pt = new(1, 2);\n\
             class P(int x) : B(x) { }\n\
             var u = \"abc\"u8;\n\
             nint n = 0;\n";
    has(t, "record", "storage.type.class.cs");
    has(t, "Person", "entity.name.class.cs");
    has(t, "First", "variable.parameter.cs");
    has(t, "struct", "storage.type.struct.cs");
    has(t, "Point", "entity.name.class.cs");
    has(t, "required", "storage.modifier.cs");
    has(t, "init", "storage.type.function.accessor.init.cs");
    has(t, "file", "storage.modifier.cs");
    has(t, "with", "keyword.other.cs");
    has(t, "Age", "variable.other.cs");
    has(t, "rest", "variable.other.cs");
    has(
        "if (x is [var a, .. var rest]) { }",
        "var",
        "storage.type.variable.cs",
    );
    has(
        "if (x is [var a, .. var rest]) { }",
        "rest",
        "variable.other.cs",
    );
    has(t, "not", "keyword.operator.logical.cs");
    has(t, "and", "keyword.operator.logical.cs");
    has(t, "or", "keyword.operator.logical.cs");
    has(t, "Length", "variable.other.cs");
    has(t, "new", "keyword.operator.new.cs");
    has(t, "x", "variable.parameter.cs");
    has(t, "u8", "storage.type.string.cs");
    has(t, "nint", "storage.type.cs");
}

#[test]
fn raw_and_interpolated_strings() {
    let t = "var a = \"\"\"single \"quoted\" raw\"\"\";\n\
             var b = \"\"\"\n    {not} \"\"x\"\"\n    \"\"\";\n\
             var c = $\"\"\"\n    {name} has {{braces}}\n    \"\"\";\n\
             var d = $$\"\"\"{ \"n\": {{n + 1}} }\"\"\";\n\
             var e = $\"{x,5:N2} {{ {y}\";\n\
             var f = $@\"both {x} \"\"q\"\"\n  {z}\";\n\
             var g = @\"verbatim \"\"q\"\" \\n\";\n";
    has(t, "single \"quoted\" raw", "string.quoted.double.raw.cs");
    has(t, "    {not} \"\"x\"\"", "string.quoted.double.raw.cs");
    has(t, "name", "variable.other.cs");
    has(
        t,
        "\"\"\"",
        "string.quoted.double.raw.cs punctuation.definition.string.end.cs",
    );
    has(t, "{{", "punctuation.section.interpolation.begin.cs");
    has(t, "n", "variable.other.cs");
    has(t, "5", "constant.numeric.integer.decimal.formatting.cs");
    has(t, "N2", "constant.other.format-spec.cs");
    has(t, "y", "variable.other.cs");
    has(t, "z", "variable.other.cs");
    has(
        t,
        "\"\"",
        "string.quoted.double.raw.cs constant.character.escape.cs",
    );
    // the lines after are code again
    let l = lines(t);
    assert_eq!(l.last().unwrap().1.arg, u16::from(M_CODE));
}

#[test]
fn statements_and_expressions() {
    let t = "class C { void M() {\n\
             List<int> list = new();\n\
             Foo.Bar baz = null;\n\
             int? n = null;\n\
             var (p, q) = t;\n\
             foreach (var item in list) { }\n\
             TryGet(out var v1, out Foo v2);\n\
             Foo(name: 1);\n\
             var r = x switch { 1 => \"a\", _ => \"c\" };\n\
             var w = list.Where(x => x > 1).Select((a, i) => a);\n\
             var i = (Foo)y + (int)z;\n\
             var j = obj.Method<string, int>(x) < a;\n\
             label: goto label;\n\
             switch (k) { case Color.Red: break; default: break; }\n\
             var q2 = from c in list where c > 1 select c;\n\
             } }\n";
    has(t, "List", "support.type.cs");
    has(t, "list", "variable.other.cs");
    has(t, "Bar", "support.type.cs");
    has(t, "?", "storage.type.nullable.cs");
    has(
        "if (x is [var a, .. var rest]) { }",
        "var",
        "storage.type.variable.cs",
    );
    has(
        "if (x is [var a, .. var rest]) { }",
        "rest",
        "variable.other.cs",
    );
    has(t, "in", "keyword.control.flow.cs");
    has(t, "out", "storage.modifier.argument.cs");
    has(t, "Foo", "support.type.cs");
    has(t, "v2", "variable.other.cs");
    has(t, "name", "variable.parameter.cs");
    has(t, "=>", "keyword.operator.cs");
    has(t, "x", "variable.parameter.cs");
    has(t, "a", "variable.parameter.cs");
    has(t, "=>", "storage.type.function.lambda.cs");
    has(t, "Method", "variable.function.cs");
    has(t, "string", "storage.type.cs");
    has(t, "label", "entity.name.label.cs");
    has(t, "label", "constant.other.label.cs");
    has(t, "Red", "constant.other.cs");
    has(t, "from", "keyword.other.sql.cs");
    has(t, "select", "keyword.other.sql.cs");
}

#[test]
fn comments_directives_attributes() {
    let t = "#region Stuff\n\
             #if DEBUG && !RELEASE\n\
             #pragma warning disable CS0168 // why\n\
             #nullable enable\n\
             /// <summary>Does <see cref=\"Foo\"/> things.</summary>\n\
             /* block\n   comment */ int x;\n\
             [Serializable, Obsolete(\"x\", DiagnosticId = \"A\")]\n\
             [assembly: System.Runtime.InteropServices.ComVisible(false)]\n\
             class C { void M([FromBody] Foo f) { var r = new Regex(@\"^\\d+$\"); } }\n";
    has(t, "region", "keyword.other.preprocessor.cs");
    has(t, "Stuff", "entity.name.section.cs");
    has(t, "DEBUG", "constant.other.flag.cs");
    has(t, "&&", "keyword.operator.logical.cs");
    has(t, "CS0168", "string.unquoted.warning.cs");
    has(t, "nullable", "keyword.other.preprocessor.cs");
    has(t, "enable", "keyword.other.preprocessor.cs");
    has(t, "summary", "entity.name.tag.begin.cs");
    has(t, "cref", "entity.other.attribute-name.cs");
    has(t, " block", "comment.block.cs");
    has(t, "Serializable", "variable.annotation.cs");
    has(
        t,
        "DiagnosticId",
        "meta.annotation.cs variable.parameter.cs",
    );
    has(t, "assembly", "storage.modifier.target.cs");
    has(t, "Runtime", "meta.annotation.cs variable.other.cs");
    has(t, "ComVisible", "variable.annotation.cs");
    has(t, "FromBody", "variable.annotation.cs");
    has(t, "^", "keyword.control.anchor.regexp");
    has(
        t,
        "\\d",
        "constant.other.character-class.escape.backslash.regexp",
    );
    plain(t, "x ");
}

#[test]
fn a_bom_is_nothing() {
    has(
        "\u{feff}using System;",
        "using",
        "keyword.control.import.cs",
    );
    has(
        "\u{feff}#nullable enable",
        "nullable",
        "keyword.other.preprocessor.cs",
    );
}

#[test]
fn a_block_lexes_as_its_lines() {
    // every field a line hands the next: frames past the levels, an open raw string's quotes and
    // `$`s, a verbatim string and a verbatim regex across lines, a block comment, a doc
    // comment's run, a ternary, an attribute across lines, a header and its `where` across lines
    // frames of every bracket's kind past the levels kept, popped back below them
    let deep = "( [ { ".repeat(14) + &"} ] ) ".repeat(13);
    let text = format!(
        "class C<T>\n  : B\n  where T : class\n{{\n  int x = a ? b\n    : c;\n  void M() {{ var d = {deep}\n}} ] ) x(y[z{{a}}]);\n\
         var r = $$\"\"\"\n  {{{{x}}}} \"\"\n  \"\"\";\n var v = @\"a\n b\"\" c\";\n\
         var re = new Regex(@\"(a\n|b)\");\n /* a\n b */\n /// <c>\n /// d\n\
         [A(1,\n 2)]\n int y; }}\n}}\n"
    );
    let one = lines(&text);
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
    assert_eq!(one.last().unwrap().1.arg, u16::from(M_CODE));
}

#[test]
fn long_lines_lex_in_linear_time() {
    for unit in [
        "a(",
        "{ ",
        "(a",
        "(a)=>",
        "a<",
        "a<b,",
        "a.b.",
        "Foo x, ",
        "\"{0}",
        "$\"{",
        "@\"",
        "\"\"\"",
        "$$\"\"\"{{",
        "/* ",
        "[A(",
        "case ",
        "x is ",
        "new ",
        "(Foo)",
        "((a, b) => ",
        "int ",
        "from a in ",
        "where T : ",
    ] {
        let l = unit.repeat(20_000);
        let t = std::time::Instant::now();
        lines(&l);
        assert!(t.elapsed().as_millis() < 300, "{unit:?}: {:?}", t.elapsed());
    }
}
