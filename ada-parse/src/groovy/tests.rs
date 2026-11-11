//! Groovy's scanner's tests (X-135): the readings ada adds where bat's grammar has none (parity
//! can't see them), state across lines, a block against its lines, long lines.

use super::*;
use crate::{Lang, Span};

/// `text`'s spans a line at a time, each as its text and its kind's last scope.
fn toks(text: &str) -> Vec<Vec<(String, String)>> {
    let mut st = State::default();
    let mut v: Vec<Span> = Vec::new();
    let mut out = Vec::new();
    for l in text.lines() {
        crate::lex(Lang::Groovy, l, &mut st, &mut v);
        out.push(
            v.iter()
                .map(|s| {
                    let sc = scopes(s.kind);
                    (
                        l[s.start as usize..s.end as usize].to_string(),
                        sc.last().copied().unwrap_or("").to_string(),
                    )
                })
                .collect(),
        );
    }
    out
}

/// The leaf `text`'s token `tok` has on line `line` (0-based), its first occurrence.
fn leaf(t: &[Vec<(String, String)>], line: usize, tok: &str) -> String {
    t[line]
        .iter()
        .find(|(s, _)| s == tok)
        .map_or_else(|| "<none>".to_string(), |(_, k)| k.clone())
}

#[test]
fn strings_and_templates() {
    let t = toks(r#"def x = "a ${b + 1} $c.d e""#);
    assert!(leaf(&t, 0, "${").starts_with("punctuation.section.embedded"));
    assert!(leaf(&t, 0, "d").starts_with("variable.other.interpolated"));
    // the plain code inside `${…}` takes its string's color
    assert_eq!(leaf(&t, 0, "b "), "string.quoted.double.groovy");
    assert_eq!(leaf(&t, 0, "+"), "keyword.operator.arithmetic.groovy");
}

#[test]
fn slashy_strings_where_an_operand_starts() {
    let t = toks("def x = 10 / 2 / 5\ndef r = ~/a+b/\nprintln(/s/)\ndef y = a /b/ c");
    assert_eq!(leaf(&t, 0, "/"), "keyword.operator.arithmetic.groovy");
    assert_eq!(
        leaf(&t, 1, "/"),
        "punctuation.definition.string.regexp.begin.groovy"
    );
    assert_eq!(leaf(&t, 1, "+"), "keyword.operator.quantifier.regexp");
    assert_eq!(leaf(&t, 2, "s"), "string.regexp.groovy");
    assert_eq!(leaf(&t, 3, "/"), "keyword.operator.arithmetic.groovy");
}

#[test]
fn strings_across_lines() {
    let t = toks("def a = \"\"\"x ${\n  y\n} z\"\"\"\ndef b = '''\nq'''\nc = 1");
    assert_eq!(leaf(&t, 1, "  y"), "string.quoted.double.block.groovy");
    assert_eq!(leaf(&t, 2, "}"), "punctuation.section.embedded.groovy");
    assert_eq!(leaf(&t, 2, " z"), "string.quoted.double.block.groovy");
    assert_eq!(leaf(&t, 4, "q"), "string.quoted.single.block.groovy");
    assert_eq!(leaf(&t, 5, "1"), "constant.numeric.groovy");
    // a `"…"` string's `${` ends with its line
    let t = toks("def a = \"x ${y\nb = 2");
    assert_eq!(leaf(&t, 1, "2"), "constant.numeric.groovy");
    // a slashy string goes on
    let t = toks("def r = /a\nb/\nc = 3");
    assert_eq!(leaf(&t, 1, "b"), "string.regexp.groovy");
    assert_eq!(leaf(&t, 2, "3"), "constant.numeric.groovy");
}

#[test]
fn groovy_numbers() {
    let t = toks("x = 1_000 + 1.0G + 0b101 + 10L + 0x1F");
    for n in ["1_000", "1.0G", "0b101", "10L", "0x1F"] {
        assert_eq!(leaf(&t, 0, n), "constant.numeric.groovy", "{n}");
    }
}

#[test]
fn type_heads() {
    let t = toks(
        "interface Shape {\n  double area()\n}\nenum Color { RED }\ntrait T {}\n@CompileStatic class A extends B implements C, D {\n}",
    );
    assert_eq!(leaf(&t, 0, "interface"), "storage.type.class.groovy");
    assert_eq!(leaf(&t, 0, "Shape"), "entity.name.type.class.groovy");
    // a body's braces plain, its methods' heads read
    assert_eq!(leaf(&t, 0, "{"), "<none>");
    assert_eq!(leaf(&t, 1, "area"), "entity.name.function.groovy");
    assert_eq!(
        leaf(&t, 1, "double"),
        "storage.type.return-type.primitive.groovy"
    );
    assert_eq!(leaf(&t, 3, "Color"), "entity.name.type.class.groovy");
    assert_eq!(leaf(&t, 3, "RED"), "constant.other.groovy");
    assert_eq!(leaf(&t, 4, "T"), "entity.name.type.class.groovy");
    assert_eq!(leaf(&t, 5, "A"), "entity.name.type.class.groovy");
    assert_eq!(leaf(&t, 5, "B"), "entity.other.inherited-class.groovy");
    assert_eq!(
        leaf(&t, 5, "D"),
        "entity.other.inherited-class.interface.groovy"
    );
    // `Foo.class` is no head
    let t = toks("def c = Foo.class\nx = [class: 'a']");
    assert_eq!(leaf(&t, 0, "class"), "<none>");
    assert_eq!(leaf(&t, 1, "class"), "constant.other.key.groovy");
}

#[test]
fn method_heads() {
    let t = toks(
        "class Ab {\n  @Test void test_a() throws Exception {\n  }\n  private static def make(Map args = [:], final String... names) {}\n  public <T> List<T> all(Class<T> c)\n  {\n  }\n  Ab(String s) {}\n}",
    );
    assert_eq!(leaf(&t, 1, "test_a"), "entity.name.function.groovy");
    assert_eq!(leaf(&t, 1, "void"), "storage.type.return-type.void.groovy");
    assert_eq!(leaf(&t, 1, "throws"), "storage.modifier.throws.groovy");
    assert_eq!(leaf(&t, 1, "Exception"), "storage.type.throwable.groovy");
    assert_eq!(leaf(&t, 1, "{"), "<none>");
    assert_eq!(leaf(&t, 3, "def"), "storage.type.return-type.def.groovy");
    assert_eq!(leaf(&t, 3, "make"), "entity.name.function.groovy");
    assert_eq!(leaf(&t, 3, "Map"), "storage.type.parameter.groovy");
    assert_eq!(leaf(&t, 3, "args"), "variable.parameter.groovy");
    // a default value is code
    assert_eq!(
        leaf(&t, 3, "["),
        "punctuation.definition.structure.begin.groovy"
    );
    assert_eq!(leaf(&t, 3, "final"), "storage.modifier.final.groovy");
    assert_eq!(leaf(&t, 3, "names"), "variable.parameter.groovy");
    assert_eq!(leaf(&t, 3, "..."), "<none>");
    assert_eq!(leaf(&t, 3, ".."), "<none>");
    assert_eq!(leaf(&t, 4, "all"), "entity.name.function.groovy");
    assert_eq!(
        leaf(&t, 4, "List<T>"),
        "storage.type.return-type.class.groovy"
    );
    // the head carried to the next line's `{`: a body's, plain
    assert_eq!(leaf(&t, 5, "{"), "<none>");
    assert_eq!(leaf(&t, 7, "Ab"), "entity.name.function.constructor.groovy");
    // a local's type is no head
    let t = toks("def f() {\n  String s = g(1)\n}");
    assert_eq!(leaf(&t, 1, "String"), "storage.type.class.groovy");
    assert_eq!(
        leaf(&t, 1, "("),
        "punctuation.definition.method-parameters.begin.groovy"
    );
}

#[test]
fn imports_and_qualified_names() {
    let t = toks(
        "import static org.junit.Assert.assertTrue\nimport a.b.C as D\ndef l = new java.util.ArrayList()\necho env.BUILD_ID",
    );
    assert_eq!(leaf(&t, 0, "static"), "storage.modifier.static.groovy");
    assert_eq!(
        leaf(&t, 0, "org.junit.Assert.assertTrue"),
        "storage.type.import.groovy"
    );
    assert_eq!(leaf(&t, 1, "as"), "keyword.operator.as.groovy");
    assert_eq!(
        leaf(&t, 2, "java.util.ArrayList"),
        "storage.type.class.groovy"
    );
    // a property's constant, not a class
    assert_eq!(leaf(&t, 3, "BUILD_ID"), "constant.other.groovy");
}

#[test]
fn keys_ternaries_cases_asserts() {
    let t = toks(
        "f(a: 1, b ? c : d)\nswitch (x) {\n  case y: break\n}\nassert x : 'm'\nlabel: z\n@Grab(group = 'g')",
    );
    assert_eq!(leaf(&t, 0, "a"), "constant.other.key.groovy");
    assert_eq!(leaf(&t, 0, "c"), "<none>");
    assert_eq!(
        leaf(&t, 0, ":"),
        "punctuation.definition.separator.key-value.groovy"
    );
    assert!(
        t[0].iter()
            .any(|(s, k)| s == ":" && k == "keyword.operator.ternary.expression-separator.groovy")
    );
    assert_eq!(
        leaf(&t, 2, ":"),
        "punctuation.definition.case-terminator.groovy"
    );
    assert_eq!(
        leaf(&t, 4, ":"),
        "keyword.operator.assert.expression-separator.groovy"
    );
    assert_eq!(leaf(&t, 5, "label"), "constant.other.key.groovy");
    assert_eq!(leaf(&t, 6, "group"), "constant.other.key.groovy");
}

#[test]
fn calls_before_keywords() {
    // bat's `(\w+)(\()` comes before an operator's, a modifier's and a type's word
    let t = toks("assert a.is(b)\nsynchronized(lock) {}\nif (x) {}");
    assert_eq!(leaf(&t, 0, "is"), "<none>");
    assert_eq!(
        leaf(&t, 0, "("),
        "punctuation.definition.method-parameters.begin.groovy"
    );
    assert_eq!(leaf(&t, 1, "synchronized"), "<none>");
    assert_eq!(leaf(&t, 2, "("), "<none>");
}

#[test]
fn sql_and_regexes() {
    let t = toks(
        "sql.eachRow(\"select a from t where id = ${id}\") {}\nsql.execute '''\n  delete from t\n'''\nx =~ /a+/\nPattern.compile(\"\\\\d+\")\n\"a\".matches(\"[a-c]\")",
    );
    assert_eq!(leaf(&t, 0, "select"), "keyword.other.DML.sql");
    assert_eq!(leaf(&t, 0, "id"), "string.quoted.double.groovy");
    assert_eq!(leaf(&t, 2, "delete"), "keyword.other.DML.sql");
    assert_eq!(leaf(&t, 4, "+"), "keyword.operator.quantifier.regexp");
    assert_eq!(
        leaf(&t, 5, "\\\\d"),
        "constant.other.character-class.escape.backslash.regexp"
    );
    assert_eq!(
        leaf(&t, 6, "["),
        "punctuation.definition.character-class.begin.regexp"
    );
    // a string that isn't a call's
    let t = toks("println \"select a from t\"");
    assert_eq!(
        leaf(&t, 0, "select a from t"),
        "string.quoted.double.groovy"
    );
}

#[test]
fn groovydoc() {
    let t = toks("/**\n * Does {@code x}.\n * @param name the <b>name</b>\n */");
    assert_eq!(leaf(&t, 1, "*"), "punctuation.definition.comment.javadoc");
    assert_eq!(leaf(&t, 1, "x"), "markup.raw.javadoc");
    assert_eq!(leaf(&t, 2, "param"), "keyword.other.documentation.javadoc");
    assert_eq!(leaf(&t, 2, "name"), "variable.parameter.javadoc");
    assert_eq!(leaf(&t, 2, "b"), "entity.name.tag.other.html");
    assert_eq!(
        leaf(&t, 3, "*/"),
        "punctuation.definition.comment.end.javadoc"
    );
}

/// A text that drives every saved field: frames past their levels, strings and `${…}`s nested
/// and across lines, SQL and regexes across lines, heads carried, parameters across lines.
const DEEP: &str = r#"class A extends B
{
  def m(int a,
        String b = "x ${ [1, [2, [3, [4, [5, [6, [7, [8, [9, [10, [11, [12, [13, [14, [15, [16, [17, [18, [19, [20, [21, [22, [23, [24, [25, [26]]]]]]]]]]]]]]]]]]]]]]]]]] }") {
    def s = """a ${ f("""b ${ g('''c''') }
      d""") } e
    """
    sql.execute """
      select x -- c
      from t where y = ${y}
    """
    def r = $/a${
      b }/$ =~ /c
    d/
    /** doc
     * @param z
     */
  }
}
"#;

#[test]
fn a_block_lexes_as_its_lines() {
    let lines: Vec<&str> = DEEP.lines().collect();
    let mut st = State::default();
    let mut v: Vec<Span> = Vec::new();
    let mut by_line = Vec::new();
    for l in &lines {
        crate::lex(Lang::Groovy, l, &mut st, &mut v);
        by_line.push((v.clone(), st));
    }
    let mut st = State::default();
    let mut spans = Vec::new();
    let mut i = 0;
    block(&mut st, &mut spans, &mut |c| {
        for l in &lines {
            let got = c.next(l.as_bytes()).to_vec();
            assert_eq!(got, by_line[i].0, "line {}: {l:?}", i + 1);
            i += 1;
        }
        None
    });
    assert_eq!(st, by_line.last().unwrap().1);
}

#[test]
fn long_lines_lex_in_linear_time() {
    for unit in [
        "a.b.c.",
        "x ? y : ",
        "\"${a}\" + ",
        "f(a: ",
        "env.",
        "/x/ + ",
        "@A(b = ",
    ] {
        let line = unit.repeat(20_000);
        let t = std::time::Instant::now();
        let mut st = State::default();
        let mut v: Vec<Span> = Vec::new();
        crate::lex(Lang::Groovy, &line, &mut st, &mut v);
        assert!(
            t.elapsed().as_secs_f64() < 1.0,
            "{unit:?}: {:?}",
            t.elapsed()
        );
    }
}

#[test]
fn a_fill_starts_where_the_scan_stands() {
    // a block comment open in a `${…}` inside another, closed on the next line before the inner
    // `}`: the outer `${…}`'s fill starts past the comment (the fuzzer's spans out of order)
    let text = "\"${\"${/*\n*/}\n\"${x ##*/}\"\nz = 1";
    let mut st = State::default();
    let mut v: Vec<Span> = Vec::new();
    for l in text.lines() {
        crate::lex(Lang::Groovy, l, &mut st, &mut v);
        let mut at = 0;
        for s in &v {
            assert!(at <= s.start && s.start < s.end, "{l:?}: {v:?}");
            at = s.end;
        }
    }
}

#[test]
fn a_bad_escape_and_a_string_end_read_alike() {
    // `\u` before no four hex digits takes four characters (bat's), the quote among them: the
    // SQL string's end is found past it too (the fuzzer's spans out of order)
    let t = toks("executeQuery(\"\\u{]\"R\nz = 1");
    assert_eq!(leaf(&t, 0, "\\u{]\"R"), "invalid.illegal.escape.groovy");
    assert_eq!(leaf(&t, 1, "1"), "constant.numeric.groovy");
}
