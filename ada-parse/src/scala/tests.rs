//! Scala's readings on [`super`]'s scanner: its line and block APIs directly, and through
//! [`crate::lex`] by [`Lang::Scala`].

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

const PARAM: &str = "variable.parameter.scala";
const VAL: &str = "variable.other.constant.scala";
const TYPE: &str = "support.type.scala";

#[test]
fn lex_takes_this_scanner() {
    let m = marked(Lang::Scala, &["val x = 1"]);
    assert!(m.iter().any(|(t, _)| *t == "x"), "{m:?}");
}

#[test]
fn declarations() {
    let l = [
        "package a.b",
        "object Main extends App with b.Foo {",
        "  val x = 1",
        "  var y: Int = 0",
        "  def f[A <: B : Ord, +C](y: Int)(implicit ev: Ord[A]): Int = y",
        "  def +(that: T): T = ???",
        "  case class P(x: Int)",
        "  type Alias[A] = Map[String, A]",
        "  private[this] val z = new java.io.File(path)",
        "}",
    ];
    has(&l, "a.b", "entity.name.namespace.header.scala");
    has(&l, " Main", "entity.name.class.scala");
    has(&l, "x", VAL);
    has(&l, "y", "variable.other.readwrite.scala");
    has(&l, "f", "entity.name.function.scala");
    has(&l, "+", "entity.name.function.scala");
    has(&l, "y", PARAM);
    has(&l, "ev", PARAM);
    has(&l, "<:", "keyword.operator.bound.scala");
    has(&l, ":", "keyword.operator.bound.scala");
    has(&l, "+", "keyword.operator.bound.scala");
    has(&l, "case class", "storage.type.class.scala");
    has(&l, "Alias", "entity.name.type.scala");
    has(&l, "this", "variable.language.scala");
    has(&l, "java", TYPE);
    has(&l, "io", TYPE);
    has(&l, "???", "keyword.other.scala");
    // the parents plain (bat's inherited class), lowercase too
    plain(&l, "App");
    plain(&l, "b");
    plain(&l, "path");
}

#[test]
fn heads_across_lines() {
    // a def's parameters on the next line, a class's
    let l = [
        "def f(a: Int)",
        "  (b: Long): Unit = {}",
        "class C(",
        "  open: Boolean,",
        "  using: Int",
        ")",
    ];
    has(&l, "b", PARAM);
    has(&l, "open", PARAM);
    has(&l, "using", PARAM);
}

#[test]
fn patterns() {
    let l = [
        "x match {",
        "  case Foo(a, Bar(b)) if a > 0 => a",
        "  case s: String => s",
        "  case e @ (_: A | _: B) => e",
        "  case `y` => 1",
        "  case xs: _* => xs",
        "  case _ => 0",
        "}",
        "val (p, Some(q)) = t",
        "val List(u, v) = l",
    ];
    has(&l, "a", PARAM);
    has(&l, "b", PARAM);
    has(&l, "s", PARAM);
    has(&l, "e", PARAM);
    has(&l, "@", "keyword.operator.at.scala");
    has(&l, "=>", "storage.type.function.arrow.case.scala");
    has(&l, "if", "keyword.control.flow.scala");
    has(&l, "_*", "keyword.operator.varargs.scala");
    has(&l, "p", VAL);
    has(&l, "q", VAL);
    has(&l, "u", VAL);
    plain(&l, "Foo");
    plain(&l, "Some");
    // the guard is code: no lambda at `a > 0 =>`
    never(&l, "0", PARAM);
    // a type in a pattern's parentheses ends with them
    let l = ["xs.collect { case (a: A) :: (b: B) :: Nil => b }"];
    has(&l, "b", PARAM);
    never(&l, "b", TYPE);
}

#[test]
fn lambdas() {
    let l = [
        "xs.map(x => x + 1)",
        "xs.foldLeft(0)((acc, y) => acc + y)",
        "xs.map { case (k, v) => k }",
        "xs.map { z => z }",
        "val f: (Int, String) => Boolean = g",
        "xs.flatMap: w =>",
        "  w",
        "() => ()",
    ];
    for n in ["x", "acc", "y", "k", "v", "z", "w"] {
        has(&l, n, PARAM);
    }
    has(&l, "=>", "storage.type.function.arrow.lambda.scala");
    has(&l, "=>", "keyword.operator.arrow.scala");
    // Scala 3's fewer braces: the method stays a name (bat's parameter)
    plain(&l, "flatMap");
    // an untyped lambda's `,` plain, as bat's
    never(&l[1..2], ",", "punctuation.separator.scala");
}

#[test]
fn typed_lambdas_in_braces() {
    // Scala 2's `{ x: T =>`: a block's first name before `:` is its parameter, the type ends at
    // its first `=>`, the lambda's arrow; the body is code again
    let lambda = "storage.type.function.arrow.lambda.scala";
    for l in [
        &["xs.map { x: Int => x + 1 }"][..],
        &["xs.map{x: Option[Int] ⇒ x.get}"],
        &["xs.map {", "  x: Int => x", "}"],
        &["xs.map { // c", "", "  x: Int => x", "}"],
        &["xs.map { implicit x: Ctx =>", "  x", "}"],
        &["trait A { x: B with C =>"],
    ] {
        let arrow = if l.concat().contains('⇒') {
            "⇒"
        } else {
            "=>"
        };
        has(l, "x", PARAM);
        has(l, arrow, lambda);
        never(l, "x", TYPE);
        never(l, "get", TYPE);
    }
    has(&["trait A { this: B =>"], "this", PARAM);
    // Scala 3's indented body: a self type after `trait A:`
    let l = ["trait A extends B:", "  this: C & D =>", "  def f = 1"];
    has(&l, "this", PARAM);
    has(&l, "=>", lambda);
    // the type's names are a type's, not a parameter's
    for (l, t) in [
        ("xs.map { s: Ctx => s }", "Ctx"),
        ("trait A { self: B with C =>", "C"),
    ] {
        never(&[l], t, PARAM);
    }
    has(&["xs.map { _: Int => 1 }"], "=>", lambda);
    // a function type in parentheses, then the lambda's arrow
    let l = ["val k = { f: (Int => Int) => f(1) }"];
    has(&l, "=>", "keyword.operator.arrow.scala");
    has(&l, "=>", lambda);
    never(&l, "f", TYPE);
    // not a block's first token, a `:` ending the line, Scala 3's fewer braces: no parameter
    never(&["foo { a; b: Int => b }"], "b", PARAM);
    never(&["locally {", "  f:", "    1", "}"], "f", PARAM);
    plain(&["xs.foo { bar: x =>", "  x", "}"], "bar");
    // an ascription's type ends with its line
    let l = ["locally {", "  x: Int", "  y.z", "}"];
    never(&l, "z", TYPE);
    // a declared type's parenthesis across lines: its `=>` a function type's
    let l = ["val f: (", "  A,", "  B", ") => C[D] =", "  (a, b) => c"];
    has(&l, "=>", "keyword.operator.arrow.scala");
    has(&l, "=>", lambda);
    has(&l, "a", PARAM);
    // a bracket at a block's start: its names are its own
    let l = ["val f = if (a) {", "  (_: P) => None", "} else g"];
    has(&l, "=>", lambda);
    plain(&l, "None");
    // `(x: Int,` across lines: a lambda's parameters
    let l = ["xs.foreach { (x: Int,", "    y: String) =>", "  x", "}"];
    has(&l, "x", PARAM);
    has(&l, "y", PARAM);
    has(&l, "=>", lambda);
}

#[test]
fn for_comprehensions() {
    let l = [
        "for (i <- 1 to 10; j <- xs if j > i) yield i * j",
        "for {",
        "  a <- fa",
        "  b = a + 1",
        "  if b > 2",
        "  c <- fb",
        "    .map(identity)",
        "} yield c",
    ];
    for n in ["i", "j", "a", "b", "c"] {
        has(&l, n, PARAM);
    }
    has(&l, "<-", "keyword.operator.assignment.scala");
    // a continued line is code: bat's pattern only where the line has `<-` or `=`
    never(&l, "identity", PARAM);
}

#[test]
fn types() {
    let l = [
        "val a: scala.collection.Seq[Int] = x",
        "val b: x.type = x",
        "val c: A | B = x",
        "def m(x: => Int, ys: Int*): Map[?, _] = ???",
        "val i = 512: java.lang.Integer",
    ];
    has(&l, "scala", TYPE);
    has(&l, "collection", TYPE);
    has(&l, "lang", TYPE);
    has(&l, "type", "keyword.other.scala");
    has(&l, "|", TYPE);
    has(&l, "=>", "keyword.operator.arrow.scala");
    has(&l, "*", "keyword.operator.varargs.scala");
    has(&l, "?", "variable.language.hole.scala");
    has(&l, "Int", "storage.type.primitive.scala");
}

#[test]
fn scala_3() {
    let l = [
        "given intOrd: Ordering[Int] with {}",
        "given listOrd[T](using ord: Ordering[T]): Ordering[List[T]] = ???",
        "extension (s: String) def shout: String = s",
        "enum Color { case Red, Green }",
        "enum Planet(mass: Double) extends Enum[Planet]:",
        "  case Mercury extends Planet(3.3e23)",
        "end Planet",
        "if x then y else z",
        "opaque type Id = String",
        "inline def f(inline x: Int): Int = x",
        "case class P(x: Int) derives Codec",
        "export a.b",
        "import a.{given, *}",
        "val given = 1; val open = 2",
    ];
    has(&l, "given", "storage.modifier.other.scala");
    has(&l, "intOrd", VAL);
    has(&l, "listOrd", "entity.name.function.scala");
    has(&l, "using", "storage.modifier.other.scala");
    has(&l, "ord", PARAM);
    has(&l, "extension", "storage.type.function.scala");
    has(&l, "s", PARAM);
    has(&l, " Color", "entity.name.class.scala");
    plain(&l, "Red");
    has(&l, "end", "keyword.declaration.scala");
    has(&l, "then", "keyword.control.flow.scala");
    has(&l, "opaque", "storage.modifier.other.scala");
    has(&l, "inline", "storage.modifier.other.scala");
    has(&l, "derives", "keyword.declaration.scala");
    has(&l, "extends", "keyword.declaration.scala");
    has(&l, "export", "keyword.other.import.scala");
    // soft keywords are names elsewhere
    has(&l, "given", VAL);
    has(&l, "open", VAL);
}

#[test]
fn strings_and_interpolation() {
    let l = [
        r#"val s = s"a $name and ${x + 1} \n $$ end""#,
        r#"val f = f"$x%.2f %d""#,
        r#"val r = raw"a\nb $y""#,
        r#"val t = s"""multi"#,
        r#"line ${ q } """"#,
        r#"val c = '\n'; val sym = 'sym"#,
        r#"val u = "unclosed"#,
        r#"val v = 1"#,
    ];
    has(
        &l,
        "s",
        "string.quoted.interpolated.scala support.function.scala",
    );
    has(
        &l,
        "$",
        "string.quoted.interpolated.scala punctuation.definition.variable.scala variable.other.scala",
    );
    has(
        &l,
        "name",
        "string.quoted.interpolated.scala variable.other.scala",
    );
    has(
        &l,
        "${",
        "string.quoted.interpolated.scala punctuation.definition.expression.scala",
    );
    has(
        &l,
        "\\n",
        "string.quoted.interpolated.scala constant.character.escape.scala",
    );
    has(
        &l,
        "$$",
        "string.quoted.interpolated.scala constant.character.escape.scala",
    );
    has(
        &l,
        "%.2f",
        "string.quoted.interpolated.scala constant.other.formatting.scala",
    );
    has(&l, "a\\nb ", "string.quoted.raw.interpolated.scala");
    has(&l, "line ", "string.quoted.triple.interpolated.scala");
    has(
        &l,
        "${",
        "string.quoted.triple.interpolated.scala punctuation.definition.expression.scala",
    );
    has(&l, "'\\n'", "constant.character.literal.scala");
    has(&l, "'sym", "constant.other.symbol.scala");
    // a `"` string ends with its line (bat's runs on to the next quote)
    has(&l, "v", VAL);
}

#[test]
fn comments() {
    let l = [
        "/** Doc with @param x the x {@link Foo} /* nested */ more",
        "  * line two",
        "  */ val a = 1",
        "/* a /* b */ c */ val b = 2",
        "/**/ val c = 3",
        "x */*c*/ y",
    ];
    has(
        &l,
        "/**",
        "comment.block.documentation.scala punctuation.definition.comment.scala",
    );
    has(
        &l,
        "@param ",
        "comment.block.documentation.scala keyword.other.documentation.scaladoc.scala",
    );
    has(
        &l,
        "{@link Foo}",
        "comment.block.documentation.scala keyword.other.documentation.scaladoc.link.scala",
    );
    has(
        &l,
        "/* nested */",
        "comment.block.documentation.scala comment.block.scala",
    );
    has(
        &l,
        "*",
        "comment.block.documentation.scala punctuation.definition.comment.scala",
    );
    has(&l, "a", VAL);
    has(&l, "b", VAL);
    has(&l, "/**/", "comment.block.empty.scala");
    // an operator's run ends where a comment starts, as Scala's lexer has it
    has(&l, "/*c*/", "comment.block.scala");
}

#[test]
fn numbers() {
    let l = ["val n = 0xFFL + 1L + 1.5e3 + 2f + 1_000 + .5"];
    has(
        &l,
        "0x",
        "constant.numeric.integer.hexadecimal.scala punctuation.definition.numeric.base.scala",
    );
    has(&l, "FF", "constant.numeric.integer.hexadecimal.scala");
    has(
        &l,
        "L",
        "constant.numeric.integer.decimal.scala storage.type.numeric.scala",
    );
    has(&l, "1", "constant.numeric.float.decimal.scala");
    has(
        &l,
        ".",
        "constant.numeric.float.decimal.scala punctuation.separator.decimal.scala",
    );
    has(
        &l,
        "f",
        "constant.numeric.float.decimal.scala storage.type.numeric.scala",
    );
    has(&l, "1_000", "constant.numeric.integer.decimal.scala");
}

#[test]
fn annotations_carry_their_meta_scope() {
    let l = [
        "@deprecated(\"use g\", since = \"1.0\") def f = 1",
        "@annotation.tailrec def g = 2",
        "@ExpressionDescription(",
        "  usage = \"x\")",
        "val after = 3",
    ];
    has(
        &l,
        "@",
        "meta.annotation.scala punctuation.definition.annotation.scala",
    );
    has(
        &l,
        "deprecated",
        "meta.annotation.identifier.scala variable.annotation.scala",
    );
    has(&l, "annotation", "meta.annotation.identifier.scala");
    has(&l, " since ", "meta.annotation.parameters.scala");
    has(&l, "  usage ", "meta.annotation.parameters.scala");
    has(
        &l,
        "x",
        "meta.annotation.parameters.scala string.quoted.double.scala",
    );
    has(&l, "after", VAL);
    has(&l, "f", "entity.name.function.scala");
}

#[test]
fn xml_literals() {
    let l = [
        "val node = <div class=\"x\" id={id}>Don't {name} &amp; <b>bold</b></div>",
        "val two = <a>",
        "  <!-- c --> text",
        "</a>",
        "if (a<b) c",
        "val after = 1",
    ];
    has(&l, "div", "text.xml meta.tag.xml entity.name.tag.xml");
    has(
        &l,
        "class",
        "text.xml meta.tag.xml entity.other.attribute-name.localname.xml",
    );
    has(&l, "x", "text.xml meta.tag.xml string.quoted.double.xml");
    has(&l, "{", "punctuation.definition.inline.begin.xml");
    has(&l, "Don't ", "text.xml");
    has(&l, "&amp;", "text.xml constant.character.entity.xml");
    has(&l, " c ", "text.xml comment.block.xml");
    has(&l, "a", "text.xml meta.tag.xml entity.name.tag.xml");
    has(&l, "after", VAL);
    // `a<b` is a comparison, as Scala's scanner reads it (bat's opens a tag)
    let m = ["if (a<b) c", "val d = 1"];
    never(&m, "b", "text.xml meta.tag.xml entity.name.tag.xml");
    has(&m, "d", VAL);
    // a comment, CDATA and an attribute's strings go on past their line
    let l = [
        "val e = <a>",
        "  <!-- one",
        "  two --> t",
        "  <![CDATA[ x",
        "  y ]]>",
        "  <b c=\"d",
        "  e\" f='g",
        "  h'>i</b>",
        "</a>",
        "val after = 1",
    ];
    has(&l, "  two ", "text.xml comment.block.xml");
    has(
        &l,
        "-->",
        "text.xml comment.block.xml punctuation.definition.comment.end.xml",
    );
    has(&l, " t", "text.xml");
    has(&l, "  y ]]>", "text.xml string.unquoted.cdata.xml");
    has(&l, "  e", "text.xml meta.tag.xml string.quoted.double.xml");
    has(&l, "  h", "text.xml meta.tag.xml string.quoted.double.xml");
    has(&l, "i", "text.xml");
    has(&l, "after", VAL);
    // elements one after another are one literal, a sequence (Scala's parser), and no
    // block's parameter after one
    let l = ["val x = {", "  <x:a b=\"c\"/> <y:d e=\"f\"/>", "}"];
    has(&l, "y:d", "text.xml meta.tag.xml entity.name.tag.xml");
    never(&l, "y", PARAM);
}

#[test]
fn sql_and_regexes_in_strings() {
    let l = [
        r#"val q = sql"SELECT a FROM t WHERE id = $id""#,
        r#"val r = SQL("select * from users")"#,
        r#"val d = spark.sql("INSERT INTO t VALUES (1)")"#,
        r#"val re = "a+[0-9]".r"#,
        r#"val p = Pattern.compile("(\\d+)")"#,
        r#"val plain = "a+[0-9]""#,
    ];
    has(
        &l,
        "SELECT",
        "string.quoted.interpolated.scala keyword.other.DML.sql",
    );
    has(
        &l,
        "id",
        "string.quoted.interpolated.scala variable.other.scala",
    );
    has(
        &l,
        "select",
        "string.quoted.double.scala keyword.other.DML.sql",
    );
    has(
        &l,
        "INSERT INTO",
        "string.quoted.double.scala keyword.other.DML.sql",
    );
    has(
        &l,
        "+",
        "string.quoted.double.scala keyword.operator.quantifier.regexp",
    );
    has(
        &l,
        "\\\\d",
        "string.quoted.double.scala constant.other.character-class.escape.backslash.regexp",
    );
    has(&l, "a+[0-9]", "string.quoted.double.scala");
}

/// Each line of `text` lexed on [`line`] from the state the one before left: its spans and state.
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
    // depth and kind, a `"""` string's kind, SQL's context, a regex's state, the string kinds
    // under `${`, a carried head, an annotation's arguments, a marker
    let kinds = "def h(d = f(a: List[(b, c = { case (x, y) if (".repeat(5);
    let text = format!(
        "val a = {kinds}\n)]))))\n/** a /* b /* c\n * d */ */ e */ x\nval r = s\"\"\"a\n  b $c ${{ d(\n e) }} f\n\"\"\" y\n\
         // language=SQL\nval q = \"\"\"SELECT a /* x\n  */ FROM t -- c\n  WHERE\"\"\"\n\
         val z = sql\"\"\"SELECT ${{\n x }} FROM t\n\"\"\"\n\
         val re = Pattern.compile(\"\"\"(a\n  [b\"\"\")\ndef f(a: Int)\n  (b: Int): Int = {{\n}}\n\
         @Ann(\n  x = \"\"\"a\n b\"\"\")\nval t = s\"a ${{b(\n  c)}} d $e\"\nfor {{\n  a <- b\n  c\n}} yield a\n"
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
    assert!(one[0].1.depth > LEVELS, "past the levels its frames keep");
    assert_eq!(one.last().unwrap().1.arg & 7, u16::from(M_CODE));
}

#[test]
fn spans_stay_in_order_on_every_line() {
    // a line's spans in order and on characters, on corners that write out of the common path
    // (the fuzzer's: an annotation opened in a SQL string's `${…}`, unwound by its `}`)
    let text = "@A(x = s\"${ a(b) }\" /* c */ \"\"\"d\n e\"\"\" .r)\nval ⇒ = 'λ' ∙ \"∙\".r\n\
                case x @ `y` :: _* ← z ⇒ w\n@throws[E](\"m\") def f = 1\n\
                fr\"\"\"\n${\n\"\"\"\n\"\"\"\n@y(\"\n(}\ne.b\n";
    for (l, (spans, _)) in text.lines().zip(lines(text)) {
        let mut at = 0;
        for s in &spans {
            assert!(
                s.start as usize >= at && s.end >= s.start,
                "{l:?}: {spans:?}"
            );
            assert!(l.is_char_boundary(s.start as usize) && l.is_char_boundary(s.end as usize));
            at = s.end as usize;
        }
    }
}

#[test]
fn long_lines_lex_in_linear_time() {
    for unit in [
        "a(",
        "{ ",
        "s\"$",
        "s\"${",
        "s\"${ {",
        "/* ",
        "/** @a ",
        "def f(",
        "def f[",
        "(a, ",
        "((",
        "x => ",
        "case ",
        "for (",
        "for {",
        "@a(",
        "@a.b.",
        "SQL(\"",
        "Pattern.compile(\"[",
        "'\\",
        "`a",
        "val (",
        "class A[",
        "import a.",
        "new a.",
        ": x =>",
        "a: ",
        "xs.map: ",
        "{ x: ",
        "{ (x: ",
        "{ _: { x: ",
        "<a b=\"",
        "<a><!--",
        "<a><![CDATA[",
    ] {
        let l = unit.repeat(20_000);
        let t = std::time::Instant::now();
        lines(&l);
        assert!(t.elapsed().as_millis() < 200, "{unit:?}: {:?}", t.elapsed());
    }
}
