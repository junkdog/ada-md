//! Kotlin's readings on [`super`]'s scanner: its line and block APIs directly, and through
//! [`crate::lex`] by [`Lang::Kotlin`].

use super::*;
use crate::Lang;
use crate::tests::marked;

/// The tokens of `lines` on [`super::line`], each with its scopes.
fn of(lines: &[&str]) -> Vec<(String, String)> {
    let mut state = State::default();
    let mut v = Vec::new();
    for l in lines {
        let mut spans = Vec::new();
        line(l.as_bytes(), &mut state, &mut Out::new(&mut spans));
        for s in &spans {
            let text = &l[s.start as usize..s.end as usize];
            v.push((text.to_owned(), scopes(s.kind).join(" ")));
        }
    }
    v
}

fn has(lines: &[&str], text: &str, scope: &str) {
    let toks = of(lines);
    assert!(
        toks.iter().any(|(t, s)| t == text && s == scope),
        "{text:?} as {scope:?} in {toks:#?}"
    );
}

/// `text` is never `scope` in `lines`.
fn never(lines: &[&str], text: &str, scope: &str) {
    let toks = of(lines);
    assert!(
        toks.iter().all(|(t, s)| t != text || s != scope),
        "{text:?} never {scope:?} in {toks:#?}"
    );
}

/// `text` is plain wherever it stands in `lines`.
fn plain(lines: &[&str], text: &str) {
    let toks = of(lines);
    assert!(
        toks.iter().all(|(t, _)| t != text),
        "{text:?} plain in {toks:#?}"
    );
}

#[test]
fn lex_takes_this_scanner() {
    let m = marked(Lang::Kotlin, &["a !in b"]);
    assert!(m.iter().any(|(t, _)| *t == "!in"), "{m:?}");
}

#[test]
fn declarations() {
    let l = [
        "package sift.core.api",
        "import kotlin.reflect.KClass as K",
        "data class Foo<T : Any>(val x: Int, y: List<String> = listOf()) : Bar(x), Baz {",
        "    fun <R> List<T>.map(f: (T) -> R): List<R> = TODO()",
        "    val name: String get() = \"x\"",
        "    private var count = 0",
        "        private set(value) { field = value }",
        "    constructor(a: Int) : this(a, \"\")",
        "    init { println(1) }",
        "}",
    ];
    has(&l, "package", "keyword.other.kotlin");
    has(&l, "sift.core.api", "entity.name.package.kotlin");
    has(&l, "import", "keyword.other.kotlin");
    has(&l, "as", "keyword.other.kotlin");
    has(&l, "data", "storage.modifier.kotlin");
    has(&l, "class", "storage.modifier.kotlin");
    has(&l, "Foo", "entity.name.type.class.kotlin");
    has(&l, "T", "storage.type.generic.kotlin");
    has(&l, "Any", "storage.type.buildin.kotlin");
    has(&l, "val", "storage.modifier.kotlin");
    has(&l, "x", "variable.parameter.function.kotlin");
    has(&l, "List", "storage.type.buildin.collection.kotlin");
    has(&l, "=", "keyword.operator.declaration.kotlin");
    plain(&l, "Bar");
    has(&l, "fun", "keyword.other.kotlin");
    has(&l, "R", "storage.type.generic.kotlin");
    has(&l, "map", "entity.name.function.kotlin");
    has(&l, "f", "variable.parameter.function.kotlin");
    has(&l, "->", "keyword.operator.declaration.kotlin");
    has(&l, "val", "keyword.other.kotlin");
    has(&l, "name", "entity.name.variable.kotlin");
    has(&l, "get", "entity.name.function.kotlin");
    has(&l, "count", "entity.name.variable.kotlin");
    has(&l, "set", "entity.name.function.kotlin");
    has(&l, "value", "variable.parameter.function.kotlin");
    has(&l, "constructor", "keyword.other.kotlin");
    has(&l, "a", "variable.parameter.function.kotlin");
    has(&l, "init", "keyword.other.kotlin");
}

#[test]
fn heads_kotlin_reads() {
    // a destructuring's names, a receiver's type, a backticked name, `fun interface`
    let l = [
        "val (a, b) = pair",
        "fun String.shout() = uppercase()",
        "fun `a test name`() {}",
        "fun interface Action { fun run() }",
        "class Foo private constructor(val a: Int)",
        "object Single : Base()",
        "companion object Factory",
    ];
    has(&l, "a", "entity.name.variable.kotlin");
    has(&l, "b", "entity.name.variable.kotlin");
    has(&l, "String", "storage.type.buildin.kotlin");
    has(&l, "shout", "entity.name.function.kotlin");
    has(&l, "`a test name`", "entity.name.function.kotlin");
    has(&l, "interface", "storage.modifier.kotlin");
    has(&l, "Action", "entity.name.type.class.kotlin");
    has(&l, "run", "entity.name.function.kotlin");
    has(&l, "private", "storage.modifier.kotlin");
    has(&l, "constructor", "keyword.other.kotlin");
    has(&l, "Single", "entity.name.type.class.kotlin");
    has(&l, "companion object", "storage.modifier.kotlin");
    has(&l, "Factory", "entity.name.type.class.kotlin");
    // a parameter list across lines, its comment a comment, its annotation plain
    let l = [
        "fun f(",
        "    a: Int, // the first",
        "    @Named(\"b\") b: String = \"x\",",
        "): Int {",
    ];
    has(&l, "a", "variable.parameter.function.kotlin");
    has(&l, " the first", "comment.line.double-slash.kotlin");
    plain(&l, "Named");
    plain(&["class P(@field:Json(name = \"id\") val id: Int)"], "Json");
    has(&l, "b", "variable.parameter.function.kotlin");
    has(&l, "Int", "storage.type.buildin.kotlin");
}

#[test]
fn type_arguments_against_comparisons() {
    // X-123: a word before `<` opened a type's arguments (`i<n`)
    let l = [
        "if (i<n && a > b) listOf<String>()",
        "val m: Map<K, List<V>> = x",
    ];
    has(&l, "<", "keyword.operator.comparison.kotlin");
    has(&l, ">", "keyword.operator.comparison.kotlin");
    has(&l, "String", "storage.type.buildin.kotlin");
    has(&l, "Map", "storage.type.buildin.collection.kotlin");
    let toks = of(&l[1..]);
    assert!(
        toks.iter().all(|(_, s)| !s.contains("comparison")),
        "{toks:#?}"
    );
}

#[test]
fn operators_and_numbers() {
    let l = [
        "a === b !== c; x !in y; z !is W",
        "a?.b ?: c!!.d; 0..10; 0..<n; f::g",
        "1_000_000 0x1F_FF 0b1010 1.5e3f 2L 3u 4uL (.5) 1.toString()",
        "x as? Y",
    ];
    has(&l, "===", "keyword.operator.comparison.kotlin");
    has(&l, "!==", "keyword.operator.comparison.kotlin");
    has(&l, "!in", "keyword.operator.kotlin");
    has(&l, "!is", "keyword.operator.kotlin");
    has(&l, "?.", "keyword.operator.safenav.kotlin");
    has(&l, "?:", "keyword.operator.elvis.kotlin");
    has(&l, "!!", "keyword.operator.kotlin");
    has(&l, "..", "keyword.operator.range.kotlin");
    has(&l, "..<", "keyword.operator.range.kotlin");
    has(&l, "::", "keyword.operator.kotlin");
    has(&l, ";", "punctuation.terminator.kotlin");
    for n in [
        "1_000_000",
        "0x1F_FF",
        "0b1010",
        "1.5e3f",
        "2L",
        "3u",
        "4uL",
        ".5",
        "1",
    ] {
        has(&l, n, "constant.numeric.kotlin");
    }
    has(&l, "as?", "keyword.operator.kotlin");
    // `type` is no keyword (X-123: bat opened a typedef at it, even inside a word)
    plain(&["val t = type + contentType<b>()"], "type");
}

#[test]
fn strings_comments_and_templates() {
    let l = [
        r#"val s = "a $b ${c.d { e }} \u0041 \n" + 'x' + '\''"#,
        r#"val r = """raw \n $x"#,
        r#"  still"""""#,
        "/* a /* nested */ still */ done",
        "/** doc",
        "*/ val k = `when`",
    ];
    has(
        &l,
        "$b",
        "string.quoted.double.kotlin variable.parameter.template.kotlin",
    );
    has(
        &l,
        "${c.d { e }}",
        "string.quoted.double.kotlin variable.parameter.template.kotlin",
    );
    has(
        &l,
        r"\u0041",
        "string.quoted.double.kotlin constant.character.escape.kotlin",
    );
    has(
        &l,
        r"\'",
        "string.quoted.single.kotlin constant.character.escape.kotlin",
    );
    has(&l, r"raw \n ", "string.quoted.third.kotlin");
    has(&l, "  still\"", "string.quoted.third.kotlin");
    has(
        &l,
        r#"""""#,
        "string.quoted.third.kotlin punctuation.definition.string.end.kotlin",
    );
    has(&l, " still ", "comment.block.kotlin");
    plain(&l, "done");
    has(&l, "val", "keyword.other.kotlin");
    plain(&l, "`when`");
    // a `"` string ends with its line, but for a template open across lines
    has(
        &["val u = \"open", "val v = 1"],
        "val",
        "keyword.other.kotlin",
    );
    let l = ["val n = \"a ${b.c {", "  d }} e $f\""];
    has(
        &l,
        "${",
        "string.quoted.double.kotlin variable.parameter.template.kotlin",
    );
    has(
        &l,
        "}",
        "string.quoted.double.kotlin variable.parameter.template.kotlin",
    );
    has(&l, " e ", "string.quoted.double.kotlin");
    has(
        &l,
        "$f",
        "string.quoted.double.kotlin variable.parameter.template.kotlin",
    );
}

#[test]
fn sql_and_regexes_in_strings() {
    const DQ: &str = "string.quoted.double.kotlin";
    const RAW: &str = "string.quoted.third.kotlin";
    let l = [
        "// language=SQL",
        r#"val q = "SELECT a FROM t WHERE b = $x""#,
        r#"@Language("sql")"#,
        r#"val r = """"#,
        r#"  DELETE FROM t -- d"#,
        r#"""""#,
        r#"@Query(value = "UPDATE u SET n = :n", name = "HAVING")"#,
        r#"db.execSQL("INSERT INTO t", "ORDER")"#,
        r#"val re = Regex("^\\d+$")"#,
        r#"val p = "a*".toRegex()"#,
    ];
    has(&l, "SELECT", &format!("{DQ} keyword.other.DML.sql"));
    has(
        &l,
        "$x",
        &format!("{DQ} variable.parameter.template.kotlin"),
    );
    has(&l, "DELETE", &format!("{RAW} keyword.other.DML.sql"));
    has(&l, " d", &format!("{RAW} comment.line.double-dash.sql"));
    has(&l, "UPDATE", &format!("{DQ} keyword.other.DML.sql"));
    has(&l, "HAVING", DQ);
    has(&l, "INSERT INTO", &format!("{DQ} keyword.other.DML.sql"));
    has(&l, "ORDER", DQ);
    has(&l, "^", &format!("{DQ} keyword.control.anchor.regexp"));
    has(&l, "*", &format!("{DQ} keyword.operator.quantifier.regexp"));
}

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

#[test]
fn a_block_lexes_as_its_lines() {
    // every field a line hands the next: frames past the levels (of every kind), a comment's
    // depth, a raw string's, a SQL raw string's context, a regex raw string's state, a marker
    let kinds = "fun h(d = fun f(a: List<(b, c = [fun <T : (".repeat(6);
    let text = format!(
        "val a = {kinds}\n)]))))\n/* a /* b /* c\n*/ */ d */ x\nval r = \"\"\"a\n  b $c\n\"\"\" y\n\
         // language=SQL\nval q = \"\"\"SELECT a /* x\n  */ FROM t -- c\n  WHERE\"\"\"\n\
         val re = Regex(\"\"\"(a\n  [b\"\"\")\nfun f(\n  a: Int,\n  b: T = 1\n): Int {{\n}}\nval t = \"a ${{b(\n  c)}} d $e\"\n"
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
    assert!(one[0].1.depth > 32, "past the levels its frames keep");
    assert_eq!(one.last().unwrap().1.arg & 7, u16::from(M_CODE));
}

#[test]
fn long_lines_lex_in_linear_time() {
    for unit in [
        "a(",
        "{ ",
        "\"$",
        "\"${",
        "\"${ {",
        "/* ",
        "fun f(",
        "fun <",
        "a<b",
        "a<(",
        "List<",
        "x < ",
        "@a(",
        "exec(",
        "Regex(\"[",
        "'\\",
        "`a",
        "val (",
        "class A<",
        "import a.",
    ] {
        let l = unit.repeat(20_000);
        let t = std::time::Instant::now();
        lines(&l);
        assert!(t.elapsed().as_millis() < 200, "{unit:?}: {:?}", t.elapsed());
    }
}

#[test]
fn modifiers_where_they_modify() {
    // Kotlin's soft modifiers: a modifier before a declaration, a name elsewhere (bat colored
    // `data`, `open`, `in` everywhere)
    let l = [
        "data class D(val data: Int)",
        "val n = data.size + open()",
        "for (x in xs) if (x !in ys) open = true",
        "fun <in T, out R, reified V> f(vararg xs: T, crossinline g: suspend () -> R) {}",
        "value class V(private val x: Int)",
        "internal lateinit var y: String",
        "    private set",
        "expect fun e(); actual fun a() {}",
    ];
    has(&l, "data", "storage.modifier.kotlin");
    has(&l, "data", "variable.parameter.function.kotlin");
    let line1 = of(&l[1..2]);
    assert!(
        line1.iter().all(|(t, _)| t != "data" && t != "open"),
        "{line1:#?}"
    );
    has(&l, "in", "keyword.operator.kotlin");
    has(&l, "!in", "keyword.operator.kotlin");
    for m in [
        "in",
        "out",
        "reified",
        "vararg",
        "crossinline",
        "suspend",
        "value",
        "private",
        "internal",
        "lateinit",
        "expect",
        "actual",
    ] {
        has(&l, m, "storage.modifier.kotlin");
    }
    has(&l, "T", "storage.type.generic.kotlin");
    has(&l, "xs", "variable.parameter.function.kotlin");
    has(&l, "g", "variable.parameter.function.kotlin");
}

#[test]
fn types_in_parentheses() {
    // a function type's parameters are types, a receiver's function type too; a soft keyword
    // names a function
    let l = [
        "fun <P1, R> ((P1) -> R).curried(): (P1) -> R = TODO()",
        "fun f(g: (T1) -> R, h: Map<K, (URL) -> Unit>) {}",
        "infix fun <T> Property<T>.by(value: T) {}",
        "class C : Base(MAX, other)",
    ];
    for t in ["P1", "T1", "URL"] {
        never(&l, t, "constant.other.kotlin");
    }
    plain(&l[1..], "T1");
    has(&l, "curried", "entity.name.function.kotlin");
    has(&l, "by", "entity.name.function.kotlin");
    has(&l, "value", "variable.parameter.function.kotlin");
    has(&l, "MAX", "constant.other.kotlin");
}

#[test]
fn accessors_and_hard_keywords() {
    let l = [
        "val name: String get() = \"x\"",
        "val y = get()",
        "typealias Fn = (Int) -> Unit",
    ];
    has(&l[..1], "get", "entity.name.function.kotlin");
    plain(&l[1..2], "get");
    has(&l, "typealias", "storage.modifier.kotlin");
}

// From the compiled scanner's tests (before X-130), which the redesign keeps.

#[test]
fn regexes_in_strings() {
    const RE: &str = "string.quoted.double.kotlin";
    const RAW: &str = "string.quoted.third.kotlin";
    let l = [
        r#"val a = Regex("^(?<y>\\d{4})[a-z&&[^q]]$x\\$")"#,
        r#"val b = "\\w+\"\\".toRegex() + Pattern.compile("""(\d+)\Q*\E""")"#,
        r#"val c = "(\\d+".toPattern(); val d = Regex.fromLiteral("a+"); f("a+")"#,
        r#"val e = Regex(""""#,
        r#"  (a|b)+ ${x}$"""+"x*")"#,
        r#"val f = """q\"""".toRegex()"#,
    ];
    has(&l, "^", &format!("{RE} keyword.control.anchor.regexp"));
    has(&l, "y", &format!("{RE} entity.name.other.group.regexp"));
    has(
        &l,
        r"\\d",
        &format!("{RE} constant.other.character-class.escape.backslash.regexp"),
    );
    has(
        &l,
        "&&",
        &format!(
            "{RE} constant.other.character-class.set.regexp keyword.operator.intersection.regexp"
        ),
    );
    // a template, then an escaped `$`
    has(
        &l,
        "$x",
        &format!("{RE} variable.parameter.template.kotlin"),
    );
    has(
        &l,
        r"\\$",
        &format!("{RE} constant.character.escape.backslash.regexp"),
    );
    // the host's escape, then the regex's lone `\` before the string's end
    has(
        &l,
        r#"\""#,
        &format!("{RE} constant.character.escape.kotlin"),
    );
    has(
        &l,
        r"\\",
        &format!("{RE} constant.character.escape.backslash.regexp"),
    );
    has(&l, "+", &format!("{RE} keyword.operator.quantifier.regexp"));
    has(
        &l,
        r#"""""#,
        &format!("{RAW} punctuation.definition.string.end.kotlin"),
    );
    // raw: `\d` as it reads, `\Q…\E`
    has(
        &l,
        r"\d",
        &format!("{RAW} constant.other.character-class.escape.backslash.regexp"),
    );
    has(
        &l,
        r"\Q",
        &format!("{RAW} constant.character.escape.backslash.regexp"),
    );
    // `.toPattern()`
    has(
        &l,
        "(",
        &format!("{RE} punctuation.definition.group.begin.regexp"),
    );
    // a `"""` regex across lines, its template and anchor
    has(&l, "|", &format!("{RAW} keyword.operator.or.regexp"));
    has(
        &l,
        "${x}",
        &format!("{RAW} variable.parameter.template.kotlin"),
    );
    has(&l, "$", &format!("{RAW} keyword.control.anchor.regexp"));
    // not a regex: `Regex.fromLiteral`, any other call, a string after a regex
    let got = of(&[&l[2][36..]]);
    assert!(!got.iter().any(|(_, s)| s.contains("regexp")), "{got:?}");
    has(&l[3..], "x*", RE);
    // a run of quotes ends a raw regex at its last three, as Kotlin reads it
    has(
        &l[5..],
        r#"\""#,
        &format!("{RAW} constant.character.escape.backslash.regexp"),
    );
    has(
        &l[5..],
        r#"""""#,
        &format!("{RAW} punctuation.definition.string.end.kotlin"),
    );
}

#[test]
fn raw_strings_as_kotlin_reads_them() {
    const RAW: &str = "string.quoted.third.kotlin";
    // no escapes; a run of quotes ends at its last three, and nothing after it is a string
    let l = [r#"val j = """{"a":"C:\new\"""" + x"#];
    has(&l, r#"{"a":"C:\new\""#, RAW);
    has(
        &l,
        r#"""""#,
        &format!("{RAW} punctuation.definition.string.end.kotlin"),
    );
    has(&l, "+", "keyword.operator.arithmetic.kotlin");
    // across lines, with a template
    let l = [r#"val s = """a $x \"#, r#"""""""#];
    has(
        &l,
        "$x",
        &format!("{RAW} variable.parameter.template.kotlin"),
    );
    // five quotes: two of text, then the end
    has(&l, r#""""#, RAW);
}
