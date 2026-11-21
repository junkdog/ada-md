//! Dart's scanner's tests (X-169): its readings, each departure from bat's grammar among them, the
//! state across lines, a block against its lines, linear time on long lines.

use super::*;
use crate::Lang;
use crate::tests::marked;

/// Each line of `text` lexed alone from the state the one before left: its spans (checked in
/// order, on the line) and the state after it.
fn lines(text: &str) -> Vec<(Vec<crate::Span>, State)> {
    let mut state = State::default();
    let mut out = Vec::new();
    for l in text.lines() {
        let mut spans = Vec::new();
        line(l.as_bytes(), &mut state, &mut Out::new(&mut spans));
        let mut at = 0;
        for s in &spans {
            assert!(
                s.start as usize >= at && s.end >= s.start && s.end as usize <= l.len(),
                "{l:?}: {spans:?}"
            );
            at = s.end as usize;
        }
        out.push((spans, state));
    }
    out
}

fn of(lines: &[&str]) -> Vec<(String, String)> {
    marked(Lang::Dart, lines)
        .into_iter()
        .map(|(t, s)| (t.to_owned(), s.to_owned()))
        .collect()
}

fn has(lines: &[&str], text: &str, scopes: &str) {
    let got = of(lines);
    assert!(
        got.iter().any(|(t, s)| t == text && s == scopes),
        "{text:?} as {scopes:?} in {got:#?}"
    );
}

/// `text` has no span of a scope holding `scope`.
fn lacks(lines: &[&str], text: &str, scope: &str) {
    let got = of(lines);
    assert!(
        !got.iter().any(|(t, s)| t == text && s.contains(scope)),
        "{text:?} not {scope:?} in {got:#?}"
    );
}

/// `text` is plain: no span covers it.
fn plain(lines: &[&str], text: &str) {
    let got = of(lines);
    assert!(
        !got.iter().any(|(t, _)| t == text),
        "{text:?} plain in {got:#?}"
    );
}

const SQ: &str = "string.interpolated.single.dart";
const DQ: &str = "string.interpolated.double.dart";

#[test]
fn keywords_and_names() {
    let l = [
        "abstract class A extends B with C implements D {",
        "  static const int k = 1;",
        "  late final String s;",
        "  factory A.named({required int a}) => A();",
        "  void f() async { await g(); }",
        "  Stream<int> h() async* { yield* s; }",
        "}",
        "try { x(); } on FormatException catch (e) { rethrow; } finally {}",
        "var b = x is! T || y as U != null;",
    ];
    has(&l, "abstract", "keyword.declaration.dart");
    has(&l, "static", "storage.modifier.dart");
    has(&l, "late", "storage.modifier.dart");
    has(&l, "required", "storage.modifier.dart");
    has(&l, "factory", "keyword.declaration.dart");
    has(&l, "named", "entity.name.function.dart");
    has(&l, "async", "keyword.control.dart");
    has(&l, "await", "keyword.control.dart");
    has(&l, "yield", "keyword.control.dart");
    has(&l, "on", "keyword.control.catch-exception.dart");
    has(&l, "is", "keyword.operator.dart");
    has(&l, "as", "keyword.cast.dart");
    has(&l, "||", "keyword.operator.logical.dart");
    has(&l, "A", "support.class.dart");
    has(&l, "int", "support.class.dart");
    has(&l, "void", "storage.type.primitive.dart");
    has(&l, "null", "constant.language.dart");
    has(&l, "f", "entity.name.function.dart");
}

/// Built-in identifiers are keywords only where Dart's grammar makes them one (bat's
/// everywhere): a member, a named argument, a variable, a call.
#[test]
fn built_in_identifiers_as_names() {
    let l = [
        "x.set(1); var set = 2; m.get(k);",
        "f(sync: true, type: t); final factory = f;",
        "stream.on(e); mixin.toString(); column.as?.name;",
        "final late = 1; required(x);",
    ];
    for w in [
        "set", "get", "sync", "type", "factory", "on", "mixin", "as", "late",
    ] {
        lacks(&l, w, "keyword");
        lacks(&l, w, "storage");
    }
    has(&l, "set", "entity.name.function.dart");
    has(&l, "get", "entity.name.function.dart");
}

/// Dart 3's class modifiers, `extension type`, a record's type after a modifier, `when` as a
/// guard but not mockito's call, `operator`, a getter's and a setter's names.
#[test]
fn declarations() {
    let l = [
        "sealed class S {} base mixin M {} interface class I {} final class F {}",
        "extension type Id(int v) {}",
        "extension E on String {}",
        "static (int, String) pair() => (1, 'a');",
        "switch (x) { case int n when n > 0: break; }",
        "when(mock.foo()).thenReturn(1);",
        "bool operator ==(Object other) => true;",
        "int get length { return 1; }",
        "set value(int v) {}",
        "String get name => _n;",
    ];
    for w in ["sealed", "base", "interface", "extension", "type"] {
        has(&l, w, "keyword.declaration.dart");
    }
    has(&l, "on", "keyword.control.catch-exception.dart");
    has(&l, "static", "storage.modifier.dart");
    has(&l, "when", "keyword.control.dart");
    has(&l, "operator", "keyword.declaration.dart");
    for f in ["length", "value", "name", "pair", "thenReturn"] {
        has(&l, f, "entity.name.function.dart");
    }
    // mockito's `when(`: a call
    let m = of(&l[5..6]);
    assert!(
        m.iter()
            .any(|(t, s)| t == "when" && s == "entity.name.function.dart"),
        "{m:?}"
    );
}

/// A function's name before `(` or type arguments and `(`, not before a comparison (bat's
/// before any `<`: `i<n`) nor a switch arm's `=>`.
#[test]
fn functions() {
    let l = [
        "for (var i=0; i<n; i++) {}",
        "final c = x.cast<int>(); T first<T>(List<T> l) => l[0];",
        "var e = switch (v) { int n => n, _ => 0 };",
        "if (a<b && c>d) {}",
    ];
    plain(&l, "i");
    plain(&l, "a");
    has(&l, "cast", "entity.name.function.dart");
    has(&l, "first", "entity.name.function.dart");
    plain(&l, "n");
}

#[test]
fn directives() {
    let l = [
        "library;",
        "import 'package:a/a.dart' as a show B, C hide D;",
        "import 'x.dart' deferred as x;",
        "export 'src/y.dart'",
        "    show Y;",
        "part of 'main.dart';",
        "import 'stub.dart' if (dart.library.io) 'io.dart';",
        "void f() { var show = 1; }",
    ];
    for w in [
        "library", "import", "export", "part", "of", "show", "hide", "deferred",
    ] {
        has(&l, w, "keyword.other.import.dart");
    }
    // `as` in a directive is the import's keyword
    let d = of(&l[1..2]);
    assert!(
        d.iter()
            .any(|(t, s)| t == "as" && s == "keyword.other.import.dart"),
        "{d:?}"
    );
    let v = of(&l[7..8]);
    assert!(!v.iter().any(|(t, _)| t == "show"), "{v:?}");
}

/// Strings as Dart reads them: a `'…'` or `"…"` one ends with its line (bat runs it on), a
/// `${…}` is code to its own `}` (past braces and strings in it), escapes whole, raw strings
/// read nothing.
#[test]
fn strings() {
    const TSQ: &str = "string.interpolated.triple.single.dart";
    let l = [
        "var t = 'open string",
        "var after = 1;",
        "var s = '${list.map((e) { return e; })} after';",
        "var w = \"x ${ \"nested ${y}\" } z\";",
        "const k = 'a\\'b\\u{1F600}\\x41\\u0041\\q';",
        "var r = r'raw \\n $x';",
        "var p = 'hi $name, ${a.b} $this \\$ \\${x}';",
        "var m = '''multi ${a +",
        "  b} line $c",
        "''';",
        "var n = r\"\"\"raw",
        "${x}\"\"\";",
        "var cjk = '$count要';",
    ];
    has(&l, "'open string", SQ);
    plain(&l, "after");
    has(&l, "'${", SQ);
    has(&l, "list", &format!("{SQ} variable.parameter.dart"));
    has(&l, "map", "entity.name.function.dart");
    has(&l, "return", "keyword.control.dart");
    has(&l, "} after'", SQ);
    has(&l, "\"nested ${", DQ);
    has(&l, "y", &format!("{DQ} variable.parameter.dart"));
    has(&l, "} z\"", DQ);
    has(&l, "\\'", &format!("{SQ} constant.character.escape.dart"));
    has(
        &l,
        "\\u{1F600}\\x41\\u0041\\q",
        &format!("{SQ} constant.character.escape.dart"),
    );
    has(&l, "r'raw \\n $x'", "string.quoted.single.dart");
    has(&l, "name", &format!("{SQ} variable.parameter.dart"));
    has(&l, "this", &format!("{SQ} variable.parameter.dart"));
    has(&l, "\\$", &format!("{SQ} constant.character.escape.dart"));
    has(&l, "'''multi ${", TSQ);
    has(&l, "+", "keyword.operator.arithmetic.dart");
    has(&l, "b", &format!("{TSQ} variable.parameter.dart"));
    has(&l, "} line $", TSQ);
    has(&l, "c", &format!("{TSQ} variable.parameter.dart"));
    has(&l, "'''", TSQ);
    has(&l, "r\"\"\"raw", "string.quoted.triple.double.dart");
    has(&l, "${x}\"\"\"", "string.quoted.triple.double.dart");
    // Dart's names are ASCII's: `要` is the string's (Oniguruma's `\w` takes it)
    has(&l, "count", &format!("{SQ} variable.parameter.dart"));
    has(&l, "要'", SQ);
}

/// SQL in sqflite's and drift's calls, across adjacent strings and lines, interpolations in
/// it; regexes in `RegExp(`'s first argument, raw and cooked.
#[test]
fn embeds() {
    const SQL_KW: &str = "keyword.other.DML.sql";
    let l = [
        "await db.rawQuery('SELECT * FROM t WHERE id = ?', [id]);",
        "await db.execute(",
        "  'CREATE TABLE $table ('",
        "  'id INTEGER PRIMARY KEY)');",
        "customSelect('SELECT ${t.name} FROM x', variables: []);",
        "await db.customStatement('''",
        "  DELETE FROM t -- all",
        "''');",
        "final re = RegExp(r'^\\d+(\\.\\d+)?$');",
        "final re2 = RegExp('^[a-z]+\\\\d\\$', caseSensitive: false);",
        "final re3 = RegExp(",
        "  r'(a|b)*');",
        "runner.execute('not sql'); execute('x');",
        "final s = 'SELECT 1';",
    ];
    has(&l, "SELECT", &format!("{SQ} {SQL_KW}"));
    has(
        &l,
        "CREATE",
        &format!("{SQ} meta.create.sql keyword.other.create.sql"),
    );
    has(&l, "table", &format!("{SQ} variable.parameter.dart"));
    has(&l, "PRIMARY KEY", &format!("{SQ} storage.modifier.sql"));
    has(&l, "t", &format!("{SQ} variable.parameter.dart"));
    has(&l, "FROM", &format!("{SQ} {SQL_KW}"));
    has(
        &l,
        "DELETE",
        &format!("string.interpolated.triple.single.dart {SQL_KW}"),
    );
    has(
        &l,
        "^",
        "string.quoted.single.dart keyword.control.anchor.regexp",
    );
    has(
        &l,
        "\\d",
        "string.quoted.single.dart constant.other.character-class.escape.backslash.regexp",
    );
    has(
        &l,
        "\\\\d",
        &format!("{SQ} constant.other.character-class.escape.backslash.regexp"),
    );
    has(&l, "\\$", &format!("{SQ} constant.character.escape.dart"));
    has(
        &l,
        "(",
        "string.quoted.single.dart punctuation.definition.group.begin.regexp",
    );
    // a receiver's `execute(` is sqflite's call shape too, a bare one isn't; a string after the
    // call or outside one isn't SQL
    has(&l[12..], "'", SQ);
    has(&l[12..], "'x'", SQ);
    has(&l[13..], "'SELECT 1'", SQ);
}

#[test]
fn comments_and_docs() {
    const DOC: &str = "comment.block.documentation.dart";
    let l = [
        "/* outer /* inner */ still */ int x;",
        "/// See [Foo.bar] and `code` here, [unclosed and `open",
        "/// ```dart",
        "/// final x = [a];",
        "/// ```",
        "/// after",
        "int y;",
        "/**",
        " * doc [ref]",
        " *     indented code",
        " * ```",
        " * fenced */ int z;",
        "/**/ // line",
        "/* a",
        "  /* b */ still",
        "*/ int w;",
    ];
    has(&l, "/* outer /* inner */ still */", "comment.block.dart");
    has(&l, "int", "support.class.dart");
    has(&l, "[Foo.bar]", &format!("{DOC} variable.name.source.dart"));
    has(&l, "`code`", &format!("{DOC} variable.other.source.dart"));
    has(&l, " here, [unclosed and `open", DOC);
    has(
        &l,
        " final x = [a];",
        &format!("{DOC} variable.other.source.dart"),
    );
    has(&l, "/// after", DOC);
    has(&l, "[ref]", &format!("{DOC} variable.name.source.dart"));
    has(
        &l,
        "    indented code",
        &format!("{DOC} variable.other.source.dart"),
    );
    // bat's fence runs past the comment's end; Dart's comment ends at its `*/`
    has(
        &l,
        " * fenced ",
        &format!("{DOC} variable.other.source.dart"),
    );
    plain(&l, "z");
    has(
        &l,
        "/**/",
        "comment.block.empty.dart punctuation.definition.comment.dart",
    );
    has(&l, "// line", "comment.line.double-slash.dart");
    has(&l, "  /* b */ still", "comment.block.dart");
    plain(&l, "w");
}

#[test]
fn literals_and_punctuation() {
    let l = [
        "var n = 1_000_000 + 0xFF_FF + 1.5e-3 + .5 + 1.toString();",
        "var sym = #foo.bar; var op = #+; var ix = #[]=;",
        "@override @JsonKey(name: 'x') @pragma('vm:prefer-inline')",
        "obj..a = 1..b = 2; x ??= y; z?.w; v ?? q; [...a, ...?b];",
        "var s = a >>> 2; s >>>= 1; d = a ~/ b; d ~/= 2;",
        "#!/usr/bin/env dart",
    ];
    for n in ["1_000_000", "0xFF_FF", "1.5e-3", ".5", "1"] {
        has(&l, n, "constant.numeric.dart");
    }
    has(&l, "#foo.bar", "constant.other.symbol.dart");
    has(&l, "#+", "constant.other.symbol.dart");
    has(&l, "#[]=", "constant.other.symbol.dart");
    has(&l, "@override", "storage.type.annotation.dart");
    has(&l, "@JsonKey", "storage.type.annotation.dart");
    has(&l, "..", "punctuation.dot.dart");
    has(&l, "...", "punctuation.dot.dart");
    has(&l, "??", "keyword.operator.ternary.dart");
    has(&l, ">>>", "keyword.operator.bitwise.dart");
    has(&l, ">>>=", "keyword.operator.assignment.bitwise.dart");
    has(&l, "~/", "keyword.operator.arithmetic.dart");
    has(&l, "~/=", "keyword.operator.assignment.arithmetic.dart");
    has(&l, "#!/usr/bin/env dart", "meta.preprocessor.script.dart");
}

/// A string's, a comment's or an escape's opener alone at a line's end, and the lines after it
/// (the guide's mistake 33).
#[test]
fn openers_at_a_lines_end() {
    for t in [
        "'",
        "\"",
        "'''",
        "r'",
        "r'''",
        "\\",
        "'\\",
        "'$",
        "'${",
        "/*",
        "/**",
        "///",
        "//",
        "@",
        "#",
        "'\\u{",
        "'\\x",
        "${",
        "}",
        ")",
        "]",
        "'''${'''",
        "/** [",
        "/** `",
        "/** ```",
        "/// ```",
        "RegExp('",
        "db.execute('",
        "db.execute('''${",
    ] {
        let text = format!("{t}\nx = 1;\n{t}");
        lines(&text);
    }
}

/// Interpolations nested past the levels the state keeps, every frame kind, then closed.
#[test]
fn deep_frames() {
    let mut text = String::new();
    let open = [
        "'${",
        "\"${",
        "'''${",
        "(",
        "[",
        "{",
        "db.execute(",
        "RegExp(",
    ];
    for i in 0..30 {
        text.push_str(open[i % open.len()]);
        text.push('\n');
    }
    for _ in 0..30 {
        text.push_str("}')]}\"'''\n");
    }
    text.push_str("var after = 'a $b';\n");
    lines(&text);
}

/// A block lexes as its lines do one at a time, past every field's cap: frames deeper than the
/// state keeps (interpolations among them), a nested comment, a triple-quoted string's SQL
/// and interpolation across lines, a fenced doc, a directive across lines.
#[test]
fn a_block_lexes_as_its_lines() {
    let mut text = String::from("import 'a.dart'\n  show B;\n");
    for i in 0..40 {
        text.push_str(["f({\n", "'${g(\n", "x = [\n", "\"\"\"${(\n"][i % 4]);
    }
    text.push_str("/* a /* b\n */ c */ int x;\n");
    text.push_str(
        "/// ```\n/// code [x]\n/// ```\nawait db.execute('''\n  SELECT ${t}\n  FROM x -- c\n''');\n",
    );
    text.push_str("final re = RegExp(r'''a+\n(b|c)''');\n/** doc\n * ```\n * code */ var y;\n");
    for _ in 0..40 {
        text.push_str(")}']\"\"\"\n");
    }
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
    assert!(
        one.iter().any(|(_, s)| s.depth > LEVELS),
        "past the levels its frames keep"
    );
    assert!(one.iter().any(|(_, s)| s.arg & 3 == u16::from(M_STR)));
    assert!(one.iter().any(|(_, s)| s.arg & 3 == u16::from(M_COMMENT)));
    assert!(one.iter().any(|(_, s)| s.arg & FENCE != 0));
    assert!(one.iter().any(|(_, s)| s.arg & DIRECTIVE != 0));
}

/// Each construct repeated on one long line lexes in linear time (the guide's mistake 17).
#[test]
fn long_lines_are_linear() {
    for unit in [
        "a, ",
        "f(a, ",
        "foo<",
        "i<n ",
        "x.y.z(",
        "'$a ${b} \\$ ",
        "get ",
        "/// [a `b ",
        "/** [a `b ",
        "db.execute('SELECT ${x}",
        "db.execute('''${'",
        "RegExp('a$b${",
        "'${'${",
        "/*",
        "#",
    ] {
        let line = unit.repeat(20_000);
        let t = std::time::Instant::now();
        let _ = of(&[&line]);
        assert!(t.elapsed().as_secs() < 2, "{unit:?}: {:?}", t.elapsed());
    }
}
