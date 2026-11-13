//! ada's readings of each language, held as snapshots of its own (X-116): a test's text is lexed
//! a line at a time, each line from the state the one before left, and every span written out
//! with its text and its kind's scopes; the result must be the file of the test's name under
//! `tests/readings/`. A change to a reading shows as that file's diff, and is accepted by
//! rewriting the files and reviewing the diff:
//!
//! ```zsh
//! ADA_BLESS=1 cargo test -p ada-parse --test readings
//! ```
//!
//! No scanner is held to syntect (the user, 2026-11-09: "we don't want to rely on bat's quirks,
//! better to have something good ourselves, that we can rely on"): syntect's colors on bat's
//! grammars are a report, `examples/parity.rs`. Each scanner's own unit tests pin its readings
//! one by one, and the `tokens` fuzz target its invariants.

use ada_parse::{Lang, Span, State, lex};
use std::fmt::Write as _;

/// ada's readings of `text` in `lang`, against the snapshot `tests/readings/{name}.txt`.
fn readings(name: &str, lang: Lang, text: &str) {
    readings_of(name, lang, &[text]);
}

/// As [`readings`], for several blocks, each from a block's first state.
fn readings_of<S: AsRef<str>>(name: &str, lang: Lang, blocks: &[S]) {
    let mut out = format!("# {lang:?}: ada-parse/tests/readings.rs, {name}\n");
    for (b, block) in blocks.iter().enumerate() {
        if blocks.len() > 1 {
            let _ = writeln!(out, "\n## block {}", b + 1);
        }
        write_block(&mut out, lang, block.as_ref());
    }
    compare(name, &out);
}

/// Each line of `text` (a `\r` before its `\n` dropped), then each of its spans: the text it
/// covers and its kind's scopes. Bytes no span covers are plain and not written.
fn write_block(out: &mut String, lang: Lang, text: &str) {
    let mut state = State::default();
    let mut spans: Vec<Span> = Vec::new();
    for (n, line) in (1..).zip(text.split_inclusive('\n')) {
        let line = line.strip_suffix('\n').unwrap_or(line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        lex(lang, line, &mut state, &mut spans);
        let _ = writeln!(out, "{n:>4} | {}", visible(line));
        for s in &spans {
            let scopes = lang.scopes(s.kind);
            let _ = writeln!(
                out,
                "       {:?} {}",
                &line[s.start as usize..s.end as usize],
                if scopes.is_empty() {
                    "-".to_owned()
                } else {
                    scopes.join(" ")
                }
            );
        }
    }
}

/// A line as written: control characters and a byte-order mark escaped, the rest as is.
fn visible(line: &str) -> String {
    let mut s = String::with_capacity(line.len());
    for c in line.chars() {
        match c {
            '\t' => s.push_str("\\t"),
            c if c.is_control() || c == '\u{feff}' => {
                let _ = write!(s, "\\u{{{:x}}}", u32::from(c));
            }
            c => s.push(c),
        }
    }
    s
}

/// `got` against the snapshot `name`, or written to it under `ADA_BLESS`.
fn compare(name: &str, got: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/readings")
        .join(format!("{name}.txt"));
    if std::env::var_os("ADA_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "no snapshot {}: ADA_BLESS=1 cargo test -p ada-parse --test readings",
            path.display()
        )
    });
    if want == got {
        return;
    }
    let (w, g): (Vec<&str>, Vec<&str>) = (want.lines().collect(), got.lines().collect());
    let at = w
        .iter()
        .zip(&g)
        .position(|(a, b)| a != b)
        .unwrap_or(w.len().min(g.len()));
    let from = at.saturating_sub(4);
    let show = |lines: &[&str]| lines[from.min(lines.len())..(at + 6).min(lines.len())].join("\n");
    panic!(
        "{name}: ada's readings differ from {} at its line {}\n--- snapshot\n{}\n--- now\n{}\n\
         accept a change with ADA_BLESS=1 cargo test -p ada-parse --test readings, then review \
         the diff",
        path.display(),
        at + 1,
        show(&w),
        show(&g)
    );
}

/// JSON's readings (O-48): objects, arrays, every value, escapes good and bad, and the comments
/// JSONC allows.
#[test]
fn json() {
    let text = concat!(
        "{\n",
        "  \"name\": \"ada\", \"tab\":\t\"a\\tb \\\"q\\\" \\u00e9 \\x\",\n",
        "  \"n\" : [0, -12, 1.5, -0.25e+3, 1E9, true, false, null],\n",
        "  \"o\": {\"k\": [], \"e\": {}}, // note\n",
        "  /* a comment\n",
        "     over lines */ \"k\\\"ey\": \"//\" /* inline */\n",
        "}\n",
    );
    readings("json", Lang::Json, text);
}

/// Diff's readings (ADR-40): git's and unified headers, hunks, context, removed and added lines,
/// the context and normal formats, Perforce's header.
#[test]
fn diff() {
    let text = concat!(
        "diff --git a/src/x.rs b/src/x.rs\n",
        "Index: src/x.rs\n",
        "===================================================================\n",
        "--- a/src/x.rs\t2026-10-31\n",
        "+++ b/src/x.rs\n",
        "@@ -1,3 +1,4 @@ fn main() {\n",
        " context\n",
        "-removed\n",
        "+added\n",
        "@@  @@\n",
        "***************\n",
        "*** 1,5 ****\n",
        "! changed\n",
        "--- 1,5 ----\n",
        "12,15c12\n",
        "< old\n",
        "---\n",
        "> new\n",
        "==== //depot/a.c#3 - /home/a.c ====\n",
    );
    readings("diff", Lang::Diff, text);
}

/// INI's readings (ADR-40): comments, sections, keys and values, quoted keys and values, a comment
/// after a value's blank (X-112).
#[test]
fn ini() {
    let text = concat!(
        "; comment\n",
        "  # another\n",
        "[section] ; why\n",
        "[a]b]\n",
        "key = value ; note\n",
        "\"quoted key\" = 'quoted ; value' ; c\n",
        "  = x\n",
        "bare words\n",
        "\n",
    );
    readings("ini", Lang::Ini, text);
}

/// TOML's readings (ADR-40), TOML 1.1 as TOML reads it (X-020): tables, keys, every value and its
/// bad forms (`toml_scan.rs`'s tests pin the departures from bat's grammar: `truex` one bad value,
/// the blanks after a value no error).
#[test]
fn toml() {
    let text = concat!(
        "# comment\n",
        "[package]\n",
        "name = \"ada-md\" # why\n",
        "version.workspace = true\n",
        "\"quoted key\" = 'literal'\n",
        "[[bin]] # array of tables\n",
        "[ a . \"b\" . 'c' ]\n",
        "dates = [1979-05-27T07:32:00Z, 1979-05-27 07:32:00.999, 1979-05-27, 07:32:00]\n",
        "nums = [ +1, -0x_1, 0xdead_BEEF, 0o17, 0b101, 1_000, 3.14, -2e-3, 6.0E+2, inf, -nan, 01 ]\n",
        "odd = [ 0o0k, 0x1g, x0b1, 1a ]\n",
        "inline = { a = 1, b.c = \"x\\n\\q\", d = [ ] }\n",
        "multi = \"\"\"\n",
        "  a \\u00e9 b\n",
        "  \"\"\"\n",
        "lit = '''\n",
        "raw \\n'''\n",
        "nested = [\n",
        "  [1, 2], # c\n",
        "  { x = 1 },\n",
        "  \"s\",\n",
        "]\n",
        "bad = ]\n",
        "[]\n",
        "key\n",
        "= 1\n",
        "a = \"unterminated\n",
        "b = yes\n",
    );
    readings("toml", Lang::Toml, text);
}

/// XML's readings (ADR-40): the declaration, processing instructions, a doctype with its internal
/// subset, comments, CDATA, tags, attributes and references.
#[test]
fn xml() {
    let text = concat!(
        "<?xml version=\"1.0\" encoding='UTF-8'?>\n",
        "<?xml-stylesheet href=\"a.xsl\"?>\n",
        "<?php echo 1; ?>\n",
        "<!DOCTYPE note SYSTEM \"note.dtd\" [\n",
        "  <!ELEMENT note (to,from?,(a|b)*)>\n",
        "  <!ELEMENT br EMPTY>\n",
        "  <!ATTLIST note id ID #REQUIRED kind (x|y) \"x\" ref IDREFS #IMPLIED>\n",
        "  <!ENTITY % param \"value\">\n",
        "  <!ENTITY pic SYSTEM \"p.gif\" NDATA gif>\n",
        "  <!NOTATION gif PUBLIC \"image/gif\">\n",
        "  %param;\n",
        "  <!BOGUS x>\n",
        "]>\n",
        "<!doctype html>\n",
        "<ns:root xmlns:ns=\"urn:x\" a='1' b=bare c = \"&amp;&#65;&#x41;&bad\">\n",
        "  <!-- a comment -- with -- dashes -->\n",
        "  <![CDATA[ raw <text> ]]>\n",
        "  <item key=\"v\"/>text &lt; & more < x\n",
        "  <a:b:c d:e:f=\"1\" ?>\n",
        "  <bad$name 1x=\"2\" / >\n",
        "  <multi\n",
        "    attr=\"v\"\n",
        "  >\n",
        "</ns:root>\n",
        "<!-- open\n",
        "comment -->\n",
    );
    readings("xml", Lang::Xml, text);
}

/// YAML's readings (ADR-40): documents, keys, flow and block collections, anchors, aliases, tags,
/// scalars of every style, comments.
#[test]
fn yaml() {
    let text = concat!(
        "%YAML 1.2\n",
        "%TAG !e! tag:example.com,2000:app/\n",
        "%FOO bar baz # c\n",
        "---\n",
        "# a comment\n",
        "name: ada # trailing\n",
        "on: yes\n",
        "nums: [0b101, 0o17, 017, 019, -42, 0x_ff, 1:30, 1_000, .5, -1.5e+3, 1.2.3, .inf, -.Inf, .NaN, 1:30.5]\n",
        "misc: {a: ~, b: null, c: =, <<: *base, ? d : e}\n",
        "when: 2001-12-14t21:59:43.10-05:00\n",
        "day: 2002-12-14\n",
        "spaced: 2001-12-14 21:59:43.10 -5 # tz\n",
        "loose: 2001-1-2 3:04:05   \n",
        "anchor: &base !!map\n",
        "  key with spaces: value with: colon\n",
        "  \"quoted\": 'it''s'\n",
        "  - item\n",
        "  - \"esc \\n \\x41 \\q\"\n",
        "  -not a dash\n",
        "tagged: !<tag:x> v\n",
        "local: !e!thing v\n",
        "bad: &a[b *c]d !\n",
        "literal: |2-\n",
        "    indented\n",
        "\n",
        "   still\n",
        "  back\n",
        "folded: >\n",
        "  text\n",
        "  # not a comment\n",
        "next: x\n",
        "flow: [a, {b: c}, d: e, \"f\": g\n",
        "  , h]\n",
        "multi: \"line one\n",
        "  line two\\\n",
        "  end\"\n",
        "? complex\n",
        ": value\n",
        "...\n",
    );
    readings("yaml", Lang::Yaml, text);
}

/// Rust's readings (X-090's scanner, of ada's own): attributes, items and their generics, a
/// placeholder's width, `write!`'s first argument, `impl` names past `for`, strings of every kind,
/// closures, labels, turbofish, numbers, nested and doc comments, macros. Departures from bat's
/// grammar by design are pinned in ada-parse's `rust` tests (`types`, `tuple_fields`, `headers`).
#[test]
fn rust() {
    let text = concat!(
        "//! crate docs $x\n",
        "#![allow(dead_code)]\n",
        "#[derive(Debug, Clone)] #[cfg(not(feature = \"x\"))]\n",
        "pub(crate) struct S<'a, T: Sized> { pub a: &'a T, b: Vec<[u8; 4]> }\n",
        "struct Tuple(u32, Option<u8>);\n",
        "enum E { A(u8), B { x: i32 }, C = 3 }\n",
        "trait Tr { type Out; fn f(&self) -> Self::Out; }\n",
        "impl<'a, T> fmt::Display for S<'a, T> where T: Copy {\n",
        "    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {\n",
        "        write!(f.out(), \"{:>8} {:10} {0:.2} {name:?} {{}}\", self.a)?;\n",
        "        println!(\"{:#?} {x:08x}\", 1u8);\n",
        "        let r = r##\"raw \"# still\"##; let b = b\"\\x00\\q\"; let c = ['\\n', b'a', '\\u{1F600}'];\n",
        "        let f = |x: u32, (a, b): (u8, u8)| -> u32 { x + 1 };\n",
        "        'outer: loop { break 'outer; }\n",
        "        let v: Vec<_> = xs.iter().map(|x| x * 2).collect::<Vec<_>>();\n",
        "        match self { Self::A if a >= 0x1F_u8 => 1.5e-3f64, _ => 0o17 as f64 + 0b1010 as f64 };\n",
        "        unsafe { *ptr = MAX_LEN << 2; std::mem::drop(x); }\n",
        "        /* block /* nested */ still */ /** doc\n",
        "          * star\n",
        "          */ x..=y; a != b && !c || d\n",
        "    }\n",
        "}\n",
        "macro_rules! m { ($x:expr, $($y:ident),*) => { $x + 1 }; }\n",
        "type Alias = fn(u8) -> u8;\n",
        "mod tests { use super::*; }\n",
        "const C_T: c_int = 0; static mut size_t: usize = 1.;\n",
        "fn g<T>(\n",
        "    a: impl Iterator<Item = u8>,\n",
        ") -> Box<dyn Fn() -> u8 + Send> where T: Default {}\n",
    );
    readings("rust", Lang::Rust, text);
}

/// Kotlin's readings (X-130, a scanner of ada's own): a package and imports, classes, functions and
/// properties with their parameters, type parameters and types, accessors, statements, operators,
/// strings with templates and escapes, raw strings, comments. Kotlin's departures from bat's
/// grammar (`type` no keyword, nested braces and block comments, modifiers only where they modify,
/// accessors only where they can be, a receiver's type a type) are pinned in ada-parse's `kotlin`
/// tests.
#[test]
fn kotlin() {
    let text = concat!(
        "package sift.core.api // the api\n",
        "import kotlin.reflect.KClass as K\n",
        "import java.util.*\n",
        "/* block\n",
        "   comment */\n",
        "@Suppress(\"UNCHECKED_CAST\")\n",
        "data class Foo<T : Comparable<T>, out R>(val x: Int, var y: List<String> = listOf()) : Bar(x), Baz {\n",
        "    companion object { const val MAX = 0x1FL; private val e = 1.5e-3f }\n",
        "    override fun <R> map(f: (T) -> R, vararg n: Int): Map<String, R>? = TODO()\n",
        "    val name: String get() = \"x\"\n",
        "    var size: Int\n",
        "        get() { return field }\n",
        "        set(value) { field = value }\n",
        "    fun run() {\n",
        "        val s = \"a $x ${y.z} \\n\" + \"\"\"raw $w\n",
        "            still\"\"\".trimIndent() + 'c' + '\\t'\n",
        "        if (a?.b ?: 0 >= 1 && c is D || !e) return null else x++\n",
        "        while (i < n) y += i * 2 % 3 / 4 - 1\n",
        "        when (v) { is Int -> v; else -> throw IllegalStateException() }\n",
        "        try { a::class; Foo::bar } catch (e: Exception) { } finally { }\n",
        "        val lazyValue: String by lazy { \"v\" }; array[0] = 1\n",
        "    }\n",
        "    interface I { fun f(a: Array<Int>, b: IntArray): Unit }\n",
        "}\n",
        "enum class E { A, B; fun x() = A }\n",
        "sealed interface S\n",
        "object Single : Base() { suspend inline fun <reified T> t(): T? = null }\n",
        "val y = 1e10; val z = 100\n",
    );
    readings("kotlin", Lang::Kotlin, text);
}

/// Scala's readings (X-137, a scanner of ada's own): a package and imports, classes, objects,
/// traits with their parameters, type parameters and parents, defs, vals and vars, types,
/// patterns, lambdas (a block's typed parameter, `{ x: Int =>`, X-141), a self type, for
/// comprehensions, strings with interpolations, escapes and formats, comments with Scaladoc,
/// numbers, annotations with their meta scope, XML literals (a comment and an attribute's string
/// across lines, elements in a sequence), SQL and regexes in strings, and Scala 3's syntax.
/// Scala's departures from bat's grammar (`a<b` no XML, a `"` string ending with its line, Scala
/// 3's keywords, fewer braces' lambdas) are pinned in ada-parse's `scala` tests.
#[test]
fn scala() {
    let text = concat!(
        "package com.example.app // the app\n",
        "import scala.util.{Try, Success => S, _}\n",
        "import cats.syntax.all.*\n",
        "/** A thing.\n",
        "  * @param x the x, see {@link Other}\n",
        "  */\n",
        "@deprecated(\"use Bar\", since = \"1.0\")\n",
        "final case class Foo[F[_]: Monad, +A <: AnyRef](x: Int, y: String = \"\")(implicit ev: Ord[A])\n",
        "    extends Bar(x) with Baz {\n",
        "  private[this] lazy val (a, b) = (1, 2L)\n",
        "  var count: Long = 0x1FL\n",
        "  type Alias[T] = Map[String, T]\n",
        "  def +(that: Foo[F, A]): Foo[F, A] = ???\n",
        "  override def run(xs: List[Int], f: Int => Boolean): Unit = {\n",
        "    val s = s\"a $x ${xs.size + 1} \\n\" + f\"$y%.2f\" + raw\"\\d\" + \"\"\"raw $w\n",
        "      still\"\"\".stripMargin + 'c' + 'sym.name\n",
        "    for (i <- 1 to 10; j <- xs if j > i) yield i * j\n",
        "    xs.map { case (k, v) => k } .foldLeft(0)((acc, n) => acc + n)\n",
        "    x match {\n",
        "      case Some(v) if v > 0 => v\n",
        "      case e: Exception => throw e\n",
        "      case _ => 0\n",
        "    }\n",
        "    val q = sql\"SELECT * FROM t WHERE id = $id\"\n",
        "    val re = \"[a-z]+\\\\d\".r\n",
        "    val node = <p class=\"x\">{s} &amp; more</p>\n",
        "    val inc = xs.map { x: Int => x + 1 }\n",
        "    val seq = <a><!-- one\n",
        "      two --></a> <b c=\"d\n",
        "      e\"/>\n",
        "    if (a<b && c != d) return else ()\n",
        "  }\n",
        "}\n",
        "trait T { self: Foo[Int, String] =>\n",
        "}\n",
        "object Foo extends App:\n",
        "  given ord: Ordering[Foo[?, ?]] = Ordering.by(_.x)\n",
        "  extension (f: Foo[?, ?]) def twice: Int = f.x * 2\n",
        "  enum Color { case Red, Green }\n",
        "  xs.flatMap: x =>\n",
        "    if x > 0 then List(x) else Nil\n",
        "end Foo\n",
    );
    readings("scala", Lang::Scala, text);
}

/// Groovy's readings (X-135, a scanner of ada's own): a hashbang, `package`, `import` and `import
/// static` paths, annotations with keys, a class's head with `extends` and `implements`, an
/// interface's, an enum's and a trait's, fields, constructors and methods with modifiers, `<T>`,
/// result types, parameters with defaults and `throws`, a head carried to the next line's `{`,
/// GStrings with `$name.a` and `${…}` (its plain text the string's), triple-quoted, slashy and
/// dollar-slashy strings, a slashy string against division, closures, map keys and labels,
/// ternaries, `case` and `assert` with their `:`, calls' punctuation, qualified class names and
/// `env.X`'s constant, Groovy's numbers, Groovydoc, SQL in groovy.sql.Sql's calls (a GString's
/// `${…}` a parameter), regexes after `~`, `=~` and in `Pattern.compile(`. The departures from
/// bat's grammar are pinned in ada-parse's `groovy` tests.
#[test]
fn groovy() {
    let text = concat!(
        "#!/usr/bin/env groovy\n",
        "package com.example.app\n",
        "import static org.junit.Assert.assertTrue\n",
        "import groovy.sql.Sql\n",
        "@Grab(group = 'org.foo', module = 'bar', version = '1.0')\n",
        "/**\n",
        " * A greeter, {@code new Greeter()}; see {@link Foo#bar}.\n",
        " * @param name the <b>name</b>\n",
        " */\n",
        "@CompileStatic\n",
        "class Greeter extends Base implements Named, Serializable {\n",
        "    private static final int MAX_SIZE = 1_000\n",
        "    String name\n",
        "    Greeter(String name) { this.name = name }\n",
        "    public <T> List<T> pick(Map opts = [:], T... items)\n",
        "        throws IOException\n",
        "    {\n",
        "        def x = 10 / 2 / 5\n",
        "        def r = items.findAll { it =~ /^a+\\d$/ }\n",
        "        println \"Hi ${name.toUpperCase()}, $name.size of ${items.size() + 1}\"\n",
        "        def t = x > 1 ? 'big' : \"small\"\n",
        "        assert t : 'message'\n",
        "        switch (x) {\n",
        "            case ~/[0-9]+/: break\n",
        "            default: return null\n",
        "        }\n",
        "        return r as List<T>\n",
        "    }\n",
        "}\n",
        "interface Named { String getName() }\n",
        "enum Color { RED, GREEN }\n",
        "trait Loud { def shout() { name.toUpperCase() } }\n",
        "def sql = Sql.newInstance(url, user, pass)\n",
        "sql.eachRow(\"SELECT id, name FROM users WHERE age > ${age}\") { row -> println row.name }\n",
        "sql.execute '''\n",
        "    DELETE FROM users -- all of them\n",
        "'''\n",
        "def p = java.util.regex.Pattern.compile(\"(\\\\w+)@(\\\\w+)\")\n",
        "def s = $/dollar ${p} slashy $$ /$\n",
        "def v = 0x1F + 1.5e3 + 10L + 1.0G - 0b101\n",
        "node('linux') {\n",
        "    stage('Build') { sh \"echo ${env.BUILD_ID}\" }\n",
        "    given: def m = [a: 1, (k): 2]\n",
        "}\n",
    );
    readings("groovy", Lang::Groovy, text);
}

/// Elixir's readings (X-138, a scanner of ada's own): a hashbang, a module's head, `@moduledoc`
/// and `@doc` (a heredoc, a `~S` one, a string, `false`), attributes, `use`/`import`/`alias`,
/// function heads with guards, defaults and `do:`, a head across lines, a protocol's head without
/// a body, `defimpl`, keys (quoted too), atoms (quoted and operators'), captures, pins,
/// characters, numbers, strings with escapes and `#{…}`, charlists, heredocs (one holding its
/// delimiter past a line's start), sigils (lowercase, uppercase with an escaped delimiter,
/// multi-letter, word lists with modifiers), `fn` with `when`, binaries with types, comments and
/// sections, regexes in sigils and `Regex.compile`, SQL in Ecto's `fragment(`, a repository's
/// `query!(` and a migration's `execute`, a HEEx template with islands. The departures from
/// bat's grammar are pinned in ada-parse's `elixir` tests.
#[test]
fn elixir() {
    let text = concat!(
        "#!/usr/bin/env elixir\n",
        "defmodule MyApp.Accounts do\n",
        "  @moduledoc \"\"\"\n",
        "  Accounts, `#{__MODULE__}`.\n",
        "      iex> ~s\"\"\"\n",
        "      ...> \"\"\"\n",
        "  \"\"\"\n",
        "  use Ecto.Schema\n",
        "  import Ecto.Query, only: [from: 2]\n",
        "  alias MyApp.{Repo, User}\n",
        "  @max 1_000\n",
        "  @doc ~S\"raw\"\n",
        "  @doc false\n",
        "  def get(id, opts \\\\ []) when is_integer(id) and id > 0, do: Repo.get(User, id)\n",
        "  defp head(a,\n",
        "            b)\n",
        "       when is_binary(a) do\n",
        "    %{\"key\": 1, name: a, ok: :ok, op: :<>, q: :\"x #{b}\"}\n",
        "    |> Enum.map(&String.upcase/1)\n",
        "    |> Enum.filter(&valid?/1)\n",
        "    x = [?a, ?\\n, 0x1F, 1.5e-3, ^pin, 'chars', \"s \\t #{a <> b}\"]\n",
        "    f = fn y when y > 0 -> y; _ -> nil end\n",
        "    <<len::size(8), rest::binary-size(len), \"ab\", 1::utf8>> = x\n",
        "    ~w(a b c)a ++ ~S(p \\) q) ++ ~HTML\"<p>\"\n",
        "    r = ~r/^(?<y>\\d{4})-[a-z\\/]+$/iu\n",
        "    Regex.compile!(\"^\\\\d+$\")\n",
        "    from(u in User, where: fragment(\"lower(?) = ?\", u.email, ^a))\n",
        "    Repo.query!(\"SELECT * FROM users WHERE id = $1\", [b]) # done\n",
        "  end\n",
        "  ## Section\n",
        "  def render(assigns) do\n",
        "    ~H\"\"\"\n",
        "    <div class={@class} phx-click=\"go\">\n",
        "      <.link navigate={~p\"/a\"}>{@name}</.link>\n",
        "      <%= if @x do %><b>&amp;</b><% end %>\n",
        "    </div>\n",
        "    \"\"\"\n",
        "  end\n",
        "end\n",
        "defprotocol Size do\n",
        "  def size(data)\n",
        "end\n",
        "defimpl Size, for: BitString do\n",
        "  def size(s), do: byte_size(s)\n",
        "end\n",
        "defmodule MyApp.Repo.Migrations.Index do\n",
        "  def up, do: execute \"CREATE INDEX i ON users (lower(email))\"\n",
        "end\n",
    );
    readings("elixir", Lang::Elixir, text);
}

/// Haskell's readings (X-136, a scanner of ada's own): pragmas, a module's head with its export
/// list, imports, CPP's lines, data declarations with records and deriving clauses, classes and
/// instances, type signatures across lines, operators read whole and in parentheses, backticks,
/// comments (`-->` an operator), numbers, characters, strings with escapes and a gap,
/// quasi-quotes (text, SQL, a regex), Template Haskell's quotes as code, SQL in a call's string,
/// regexes after `=~` and `mkRegex`; types wherever Haskell has one (X-143): a record's fields
/// across lines, a GADT's constructors, `a, b ::`, `::` starting a line, `let f ::`, a pattern
/// synonym's signature, deriving clauses with strategies and `via`, an instance's head across
/// lines, annotations. The departures from bat's grammar are pinned in ada-parse's `haskell`
/// tests.
#[test]
fn haskell() {
    let text = concat!(
        "{-# LANGUAGE OverloadedStrings,\n",
        "             QuasiQuotes #-}\n",
        "-- | The module.\n",
        "module Data.Foo\n",
        "  ( Foo (..), bar, (<+>)\n",
        "  , module X\n",
        "  ) where\n",
        "import qualified Data.Map.Strict as M\n",
        "import Data.List (sortBy, foldl')\n",
        "#if MIN_VERSION_base(4,8,0)\n",
        "import Data.Functor ((<&>))\n",
        "#endif\n",
        "data Foo a = Foo { fooX :: !Int, fooY :: a }\n",
        "  | Bar deriving (Show, Eq)\n",
        "newtype Wrap = Wrap Int deriving newtype Num\n",
        "class (Eq a) => Container f a where\n",
        "  empty :: f a\n",
        "instance Show Wrap where\n",
        "  show (Wrap x) = \"Wrap \" ++ show x\n",
        "infixl 6 <+>\n",
        "(<+>) :: Foo a -> Foo a -> Foo a\n",
        "a <+> _ = a\n",
        "bar :: forall a. (Ord a, Show a)\n",
        "    => [a] -> M.Map a Int -> IO ()\n",
        "bar xs m = do\n",
        "  let n = 0x1F + 0o17 + 0b101 + 1_000 + 1.5e-3 + 0x1.8p3\n",
        "      c = 'x' : '\\n' : '\\^C' : 'λ' : []\n",
        "  x <- pure $ M.lookup n m <*> y `div` 2 >>= f &&& g\n",
        "  case x of\n",
        "    Just v | v > 0 -> print v --> w\n",
        "    _ -> pure () -- done\n",
        "  where\n",
        "    s = \"a \\\"b\\\" \\1234 \\SOH gap \\\n",
        "        \\on\" ++ [r|raw|] {- a {- nested -} comment -}\n",
        "    q = [sql| SELECT a FROM t WHERE b = ? |]\n",
        "    r = [re|^(\\d+)-(\\w+)$|]\n",
        "    t = [d| f = $(g 'h ''T) |]\n",
        "    u = query conn \"SELECT id FROM users WHERE age > ?\" (Only n)\n",
        "    v = s =~ \"^[a-z]+(x|y)$\" || isJust (mkRegex \"a.b\")\n",
        "data R = R\n",
        "  { rA, rB :: {-# UNPACK #-} !Int -- ^ doc\n",
        "  , rC ::\n",
        "      Map Int String\n",
        "  }\n",
        "  deriving stock (Eq, Generic)\n",
        "  deriving anyclass ToJSON\n",
        "  deriving (Semigroup) via (Sum Int)\n",
        "data T a where\n",
        "  TInt, TOne :: Int -> T Int\n",
        "  TRec :: { tf :: Int } -> T Int\n",
        "pattern Pair :: a -> b -> (a, b)\n",
        "f, g\n",
        "  :: (Monad m)\n",
        "  => m ()\n",
        "f = let k :: Int\n",
        "        k = read \"1\" :: Int in pure (k :: Int, ())\n",
        "instance\n",
        "  (Show a) =>\n",
        "  Show (W a) where\n",
    );
    readings("haskell", Lang::Haskell, text);
}

/// Literate Haskell's readings (X-136): prose plain, Bird tracks, `\begin{code}` blocks and
/// markdown-unlit's fences around Haskell's code.
#[test]
fn literate_haskell() {
    let text = concat!(
        "\\section{Prose}\n",
        "Some prose with \"quotes\" -- and dashes.\n",
        "\n",
        "> main :: IO ()\n",
        "> main = putStrLn \"hi\" -- c\n",
        "\n",
        "\\begin{code}\n",
        "f :: Int -> Int\n",
        "f = (+ 1)\n",
        "\\end{code}\n",
        "``` haskell\n",
        "g = 'x'\n",
        "```\n",
        "h = \"prose\"\n",
    );
    readings("literate_haskell", Lang::LiterateHaskell, text);
}

/// Java's readings (ADR-40): an annotation's line end and the next line's indent, `default ->` a
/// lambda's parameter, Javadoc's HTML and block tags, generics, lambdas, text blocks, the SQL
/// marker before a string (phase 4).
#[test]
fn java() {
    let text = concat!(
        "package org.example.app; // the app\n",
        "import java.util.*;\n",
        "import static org.junit.Assert.assertEquals;\n",
        "/** Doc with <a href=\"x\" id=y class='z'>link</a>, <my-tag Bad>x</my-tag>, <!-- c -->\n",
        " * &amp; &#169; &bad <> a<b List<String> {@code a {b} c} {@inheritDoc} {@value Foo#BAR}\n",
        " * {@linkplain java.util.List#add(Object, int) the add}\n",
        " * @see Foo#bar(int, String)\n",
        " * @hidden unknown\n",
        " *   so this stays <b>plain</b>\n",
        " * @param <T> the type\n",
        "@param at the line's start\n",
        " * @return the {@code value\n",
        " *   across} lines */\n",
        "@SuppressWarnings({\"unchecked\", \"rawtypes\"})\n",
        "@org.example.Ann(value = 1, other = @Inner(x = \"y\"), arr = {1, 2})\n",
        "public abstract class Foo<T extends Comparable<? super T> & Serializable> extends com.x.Bar<T> implements Baz {\n",
        "    private static final long A = 0x1FL, B = 0b1010_1010L, C = 0_17, D = 09, E = 1.5e-3f, F = .5d, G = 0x1.8p3;\n",
        "    protected int[] arr = new int[] {1, 2}, more[] = {};\n",
        "    char c = '\\'', d = '\\u00e9'; String s = \"unterminated\n",
        "    static { init(); }\n",
        "    @Override\n",
        "    public <R> List<R> map(final java.util.function.Function<? super T, ? extends R> f, int... xs) throws IOException {\n",
        "        var list = new ArrayList<String>(10); Runnable r = () -> { System.out.println(\"x\"); };\n",
        "        list.forEach(e -> f(e)); list.stream().map(String::valueOf); Foo.<String>bar(); Foo.class; T.x;\n",
        "        int x = a < b ? -1 : 1, y = x >>> 2; x >>= 1; x %= 2; boolean q = !(x != y) && x == y || ~x > 0;\n",
        "        if (o instanceof String s) { return null; } else { }\n",
        "        switch (x) { case 1: break; case FOO -> g(); default -> {} default: continue; }\n",
        "        for (int i = 0; i < 10; i++) { } try (var in = open(f)) { } catch (IOException | RuntimeException e) { } finally { }\n",
        "        assert x > 0 : \"msg\"; synchronized (this) { this.x = super.y; }\n",
        "        Object o = new Object() { @Override public String toString() { return \"\"; } };\n",
        "        goto label; const int K = 1; 1abc(); $x = 1; foo$bar(); x = 1.foo;\n",
        "    }\n",
        "    public @interface Ann { String value() default \"x\"; Class<?> c() default Object.class; }\n",
        "    enum Color { RED, GREEN(\"g\") { void x() {} }, BLUE;\n",
        "        private final int v;\n",
        "    }\n",
        "    record P(int x, int y) implements I { }\n",
        "    x = y %> z;\n",
        "}\n",
        "/* block /* not nested */ /** doc */ /***/ /**/ // line /* with a block\n",
        "module com.example.mod { requires transitive java.base; exports com.example.api to other.mod; provides a.Spi with a.Impl; }\n",
    );
    readings("java", Lang::Java, text);
}

/// C++'s readings (X-057's scanner, of ada's own): the preprocessor (a disabled branch a comment,
/// `#if 1`'s else branch, `#pragma mark`), an upper-case macro call on its own line, raw strings
/// across lines, class, method and function definitions (a friend class, a trailing return type,
/// `delete[]`); an enum's underlying type and a base list's type words types, a lambda's `-> int` a
/// type (ada-parse's `cc` tests).
#[test]
fn cpp() {
    let text = concat!(
        "#include <vector>\n",
        "#include \"local.h\" // why\n",
        "#define MAX(a, b) ((a) > (b) ? (a) : (b))\n",
        "#define NAME \"x\" \\\n",
        "  \"continued\"\n",
        "#if 0\n",
        "int disabled(); #if nested\n",
        "#endif\n",
        "#else\n",
        "int enabled;\n",
        "#endif\n",
        "#if 1\n",
        "int on;\n",
        "#else\n",
        "int off;\n",
        "#endif\n",
        "#ifdef __cplusplus\n",
        "extern \"C\" {\n",
        "#endif\n",
        "/* = banner = */\n",
        "namespace foo::bar {\n",
        "using namespace std;\n",
        "using Alias = std::map<int, std::string>;\n",
        "template <typename T, class... Rest> class Widget final : public Base<T>, private virtual Other {\n",
        " public:\n",
        "  Widget(int x) : x_(x), y_{2} {}\n",
        "  ~Widget() override;\n",
        "  explicit operator bool() const noexcept { return mFlag && m_ready; }\n",
        "  Widget& operator=(const Widget&) = delete;\n",
        "  static_assert(sizeof(T) > 0, \"size\");\n",
        "  friend class Other;\n",
        "  virtual void run() = 0;\n",
        "  auto get() const -> decltype(x_) { return this->x_; }\n",
        " private:\n",
        "  int x_ = 0b1010'1010, y_ = 0x1.8p3, z_ = 017, w_ = 1'000'000ull;\n",
        "  double d_ = 1.5e-3f, e_ = .5L;\n",
        "};\n",
        "struct Point { int x, y; };\n",
        "enum class Color { Red, Green = 2 };\n",
        "union U { int i; float f; };\n",
        "typedef struct node node_t;\n",
        "int Widget::count = 0;\n",
        "std::vector<int> make(size_t n, const char* s = nullptr) {\n",
        "  auto raw = R\"delim(one\n",
        ")\" still )delim\";\n",
        "  auto lam = [&](int a) mutable { return a * 2; };\n",
        "  if (n <= 0 || !s) { throw std::runtime_error(\"bad %d\\n\"); }\n",
        "  for (auto& v : list) v.push_back(static_cast<int>(n));\n",
        "  switch (n) { case 1: break; default: goto done; }\n",
        "done:\n",
        "  printf(\"%5.2f %s %%\", 1.0, u8\"x\");\n",
        "  ptr->member.call<int>(1)->field;\n",
        "  delete[] arr; new int[3];\n",
        "  TEST_MACRO(a, sizeof(b), \"c\")\n",
        "  return std::move(n);\n",
        "}\n",
        "}  // namespace foo::bar\n",
        "#ifdef __cplusplus\n",
        "}\n",
        "#endif\n",
        "#pragma mark Section\n",
        "#error \"stop\" here\n",
    );
    readings("cpp", Lang::Cpp, text);
}

/// C's readings (C++'s scanner, X-057's): C's top level (a type, a function's name before its
/// parameters, a call's group, a struct and an enum), the global `#if` rules and branches, a float
/// ending on its `.`, a hex number before a name, C's strings, escapes and placeholders, and a
/// continued `#define` whose body ends with its last line.
#[test]
fn c() {
    let text = concat!(
        "#include <stdio.h>\n",
        "#include \"local.h\" /* why */\n",
        "#define MAX(a, b) ((a) > (b) ? (a) : (b))\n",
        "#define HALF 1.\n",
        "#if 0\n",
        "int disabled(void); #if nested\n",
        "#endif\n",
        "#else\n",
        "int enabled;\n",
        "#endif\n",
        "#if 1\n",
        "static int on;\n",
        "#else\n",
        "int off;\n",
        "#endif\n",
        "#ifdef HAVE_X\n",
        "{ int braced; }\n",
        "#elif defined(Y)\n",
        "extern long y;\n",
        "#endif\n",
        "typedef struct node { struct node *next; int value; } node_t;\n",
        "enum color { RED, GREEN = 2 };\n",
        "union u { int i; float f; };\n",
        "static const char *names[] = { \"a\", \"b\\n\", L\"wide\" };\n",
        "GType qmi_get_type (void) G_GNUC_CONST;\n",
        "API_EXPORT int\n",
        "count_items(const struct list *l, size_t n)\n",
        "{\n",
        "  int total = 0x1fUL + 017 + 1e3 + .5f + 0x__bad;\n",
        "  for (size_t i = 0; i < n; i++) {\n",
        "    if (l->items[i].ok && !l->skip) total += sizeof(l->items[i]);\n",
        "    else continue;\n",
        "  }\n",
        "  switch (total) { case 1: break; default: goto done; }\n",
        "done:\n",
        "  printf(\"%5.2f %s %%\\n\", 1.0, u8\"x\");\n",
        "  memcpy(dst, src, n); FOO_BAR(x)\n",
        "  return total > 0 ? total : -1;\n",
        "}\n",
        "#pragma mark Section\n",
        "#error \"stop\" here\n",
        "#define NAME \\\n",
        "          OTHER_NAME\n",
    );
    readings("c", Lang::C, text);
}

/// Objective-C's readings (X-134, C's scanner with Objective-C's words and tokens): `@interface`'s
/// name, super class, category and protocols; `@property`'s attributes; method heads over lines and
/// a macro after one (bat leaves the `@interface` there); sends whose receiver is a send, a member
/// or a cast (bat reads them as brackets); blocks in a send's arguments; the `@` keywords and
/// literals, `%@`, a dictionary's `:`; lightweight generics and `id<…>`; Cocoa's types, functions
/// and classes; SQL in FMDB's and SQLite's strings, a pattern in `NSRegularExpression`'s.
#[test]
fn objc() {
    let text = concat!(
        "#import <Foundation/Foundation.h>\n",
        "@class Bar, Baz;\n",
        "NS_ASSUME_NONNULL_BEGIN\n",
        "@protocol Runner <NSObject>\n",
        "@required\n",
        "- (void)run;\n",
        "@optional\n",
        "+ (nullable instancetype)make:(in NSString *)s, ...;\n",
        "@end\n",
        "@interface Foo<T> : NSObject <Runner, NSCopying> {\n",
        "  @private\n",
        "  __weak id<Runner> _runner;\n",
        "}\n",
        "@property (nonatomic, copy, getter=isOn) NSArray<NSString *> *names;\n",
        "@property (class, readonly) void (^handler)(NSError * _Nullable error);\n",
        "- (void)runWithBlock:(void (^)(BOOL finished))completion NS_SWIFT_NAME(run(block:));\n",
        "- (instancetype)initWithName:(NSString *)name\n",
        "                       count:(NSInteger)count NS_DESIGNATED_INITIALIZER;\n",
        "@end\n",
        "@interface Foo (Private)\n",
        "@end\n",
        "static NSString * const kKey = @\"key\";\n",
        "@implementation Foo\n",
        "@synthesize names = _names;\n",
        "- (instancetype)initWithName:(NSString *)name count:(NSInteger)count\n",
        "{\n",
        "  self = [super init];\n",
        "  if (self) { _names = [name copy]; self.count = count; }\n",
        "  return self;\n",
        "}\n",
        "- (void)go {\n",
        "  id x = [[[Foo alloc] initWithName:@\"%@ %d\\n\" count:2] autorelease];\n",
        "  [[NSNotificationCenter defaultCenter] addObserver:self selector:@selector(go:with:) name:nil object:nil];\n",
        "  [self.view addConstraint:[NSLayoutConstraint constraintWithItem:x\n",
        "                                                        attribute:NSLayoutAttributeTop\n",
        "                                                        relatedBy:0]];\n",
        "  [UIView animateWithDuration:0.3 animations:^{\n",
        "    switch (x.tag) { case 1: break; default: break; }\n",
        "  } completion:^(BOOL finished) {\n",
        "    [self done];\n",
        "  }];\n",
        "  NSArray *a = @[@1, @\"two\", @(x.count), @YES];\n",
        "  NSDictionary *d = @{ @\"k\": @1, kKey: a[0] };\n",
        "  for (id obj in a) { [obj run]; }\n",
        "  @try { [self run]; } @catch (NSException *e) { @throw; } @finally { }\n",
        "  @synchronized (self) { @autoreleasepool { NSLog(@\"%@\", NSStringFromClass([self class])); } }\n",
        "  CGRect r = CGRectZero; SEL s = @selector(go); BOOL ok = YES; Class c = Nil;\n",
        "  FMResultSet *rs = [db executeQuery:@\"SELECT * FROM t WHERE a = ?\", @1];\n",
        "  sqlite3_prepare_v2(h, \"SELECT a \"\n",
        "                        \"FROM t\", -1, &stmt, NULL);\n",
        "  NSRegularExpression *re = [NSRegularExpression regularExpressionWithPattern:@\"^[a-z]+\\\\d$\" options:0 error:nil];\n",
        "}\n",
        "@end\n",
        "NS_ASSUME_NONNULL_END\n",
    );
    readings("objc", Lang::ObjC, text);
}

/// Objective-C++'s readings (X-134, C++'s scanner with Objective-C's words and tokens): C++'s
/// namespaces, classes, templates and lambdas beside an `@implementation`'s ivars, sends with C++
/// in their arguments, a raw SQL string over lines in a Qt call.
#[test]
fn objcpp() {
    let text = concat!(
        "#import \"Bridge.h\"\n",
        "#include <memory>\n",
        "namespace app {\n",
        "class Engine {\n",
        " public:\n",
        "  void run(std::vector<int> &v) { for (auto &x : v) { x++; } }\n",
        "};\n",
        "}  // namespace app\n",
        "@implementation Bridge {\n",
        "  std::unique_ptr<app::Engine> _engine;\n",
        "}\n",
        "- (void)start:(NSArray<NSString *> *)names {\n",
        "  auto f = [&](int x) { return x + 1; };\n",
        "  _engine->run(v);\n",
        "  [self.delegate bridge:self didStart:static_cast<int>(names.count)];\n",
        "  std::string s = [name UTF8String];\n",
        "  query.exec(R\"sql(\n",
        "    SELECT a -- the key\n",
        "    FROM t)sql\");\n",
        "  dispatch_async(dispatch_get_main_queue(), ^{\n",
        "    [self finish];\n",
        "  });\n",
        "}\n",
        "@end\n",
    );
    readings("objcpp", Lang::ObjCpp, text);
}

/// C#'s readings (X-122, a scanner of ada's own, C# 9–12): declarations, members, statements,
/// expressions, generics, casts, lambdas, strings with placeholders and interpolations, doc
/// comments, preprocessor lines, attributes. Departures from bat's grammar (records, `init`, raw
/// strings, patterns, a line comment ending with its line) are pinned in ada-parse's `cs` tests.
#[test]
fn csharp() {
    let text = concat!(
        "// a note // nested\n",
        "/* a block\n",
        "still the block */\n",
        "using System; using static System.Math; using Alias = Some.Name<int>;\n",
        "#region Things\n",
        "#if DEBUG && !RELEASE // why\n",
        "#pragma warning restore\n",
        "#endif\n",
        "namespace A.B.C\n",
        "{\n",
        "    /// <summary>A <see cref=\"Foo\"/> thing, TODO: https://x.y/z</summary>\n",
        "    /// <param name=\"x\">the x</param>\n",
        "    [Serializable, Obsolete(\"old\", error: true)]\n",
        "    [assembly: global::Xunit.Auto._47DEQ.Attr]\n",
        "    public sealed partial class Foo<T, in U> : Bar<T>, IBaz where T : class, new()\n",
        "    {\n",
        "        private readonly int _x = 0x1F_FF, _y = 0b1010;\n",
        "        public event EventHandler<int> Changed;\n",
        "        public string Name { get; private set; } = \"n\";\n",
        "        public int this[int i] => i * 2;\n",
        "        static Foo() : base() { }\n",
        "        ~Foo() { }\n",
        "        public static Foo operator +(Foo a, Foo b) => a;\n",
        "        internal async Task<List<int>> GetAsync(ref int a, out int b, params object[] c)\n",
        "        {\n",
        "            var s = $\"a {x,-10:N2} {{b}} {(y ? 1 : 2)}\" + @\"c:\\dir\"\" {0}\" + $@\"{z}\";\n",
        "            var f = string.Format(\"{0:D3} {1}\", 1.5e-3f, 'c') + '\\u0041' + 10UL + 0.5m;\n",
        "            Texture.Create(TextureType.Cube, 1_000);\n",
        "            if (a is int n && b as string != null) { return; } else if (x) y++; else z--;\n",
        "            foreach (var (k, v) in dict) { switch (k) { case 1: case Kind.A when v > 0: break; default: goto done; } }\n",
        "            for (int i = 0; i < 10; i++) while (true) do { } while (false);\n",
        "            try { throw new InvalidOperationException(nameof(x)); }\n",
        "            catch (Exception e) when (e.Message != null) { }\n",
        "            finally { lock (o) { } }\n",
        "            using (var r = new StreamReader(p)) { }\n",
        "            Func<int, int> sq = x => x * x; var t = (1, 2); (int q, string w) = (1, \"\");\n",
        "            var list = new List<int> { 1, 2 }; var anon = new { A = 1, B = \"b\" };\n",
        "            var arr = new int[] { 1 }; int? m = null; var c = (Foo)obj; checked(x + 1);\n",
        "            var q2 = from c in list where c > 1 select c;\n",
        "            yield return default(T); typeof(List<>); unsafe { int* p = &x; }\n",
        "        done:\n",
        "            return await Task.FromResult(new List<int>());\n",
        "        }\n",
        "    }\n",
        "    public enum Color : byte { Red = 1, Green, }\n",
        "    public delegate void Handler<T>(object sender, T args);\n",
        "    interface IBaz { void Do(); }\n",
        "    public readonly ref struct Span2 { }\n",
        "}\n",
    );
    readings("csharp", Lang::CSharp, text);
}

/// Go's readings (ADR-40): a `//go:` line's annotation, a template in a string, a raw string across
/// lines, a call in parentheses, `make`'s type argument, a method's receiver, embedded types and
/// tags, `iota`, a type switch, Unicode digits.
#[test]
fn go() {
    let text = concat!(
        "//go:build linux && !cgo\n",
        "// Package x does things.\n",
        "package x\n",
        "\n",
        "import (\n",
        "\t\"fmt\"\n",
        "\tstr \"strings\"\n",
        ")\n",
        "\n",
        "//go:generate stringer -type=Op\n",
        "type Op int\n",
        "\n",
        "const (\n",
        "\tA Op = iota // first\n",
        "\tB, C\n",
        "\t_ = 1 << 2\n",
        ")\n",
        "\n",
        "var (\n",
        "\tm   = map[string][]*Op{\"a\": nil}\n",
        "\tch  chan<- struct{}\n",
        "\tfn  func(int, ...string) (bool, error)\n",
        ")\n",
        "\n",
        "type (\n",
        "\tPair[K comparable, V any] struct {\n",
        "\t\tKey K `json:\"key\"`\n",
        "\t\tVal V /* the value */\n",
        "\t\tio.Reader\n",
        "\t\t*Embedded\n",
        "\t}\n",
        "\tAlias = Pair[string, int]\n",
        "\tI     interface {\n",
        "\t\tfmt.Stringer\n",
        "\t\tRead(p []byte) (n int, err error)\n",
        "\t\tClose() error\n",
        "\t}\n",
        ")\n",
        "\n",
        "func (p *Pair[K, V]) Swap(a, b int, rest ...float64) (r Pair[V, K]) {\n",
        "\tx, y := 0x1F, 0o17i + 1.5e-3 + .5 + 0x1p-2 + 089 + 1_000 + '\\n' + '\\x41' + 'é'\n",
        "\tx &^= y; x <<= 1; x >>= 2; ok := x != y && !(x <= y) || x >= y\n",
        "\ts := \"%5.2f %[1]q %% {{.Name | printf \\\"%v\\\" | html}} \\u00e9\\t\" + `raw {{ $v := .X }}\n",
        "still raw %d`\n",
        "\tv := make([]map[string]int, 3); w := new(Pair[int, int]); n := len(v) + cap(v)\n",
        "\tswitch t := any(v).(type) { case int, string: fallthrough; default: _ = t }\n",
        "\tfor i := range v { if i > 0 { continue } else { break } }\n",
        "\tgo func() { defer close(ch2) }(); select { case <-done: return }\n",
        "\tf := func(a int) int { return a * 2 }; _ = (f(1)); goto end\n",
        "end:\n",
        "\tfmt.Println(str.ToUpper(\"x\"), int64(x), []byte(s), true, false, nil, ٣, x.y.z)\n",
        "\tvar q struct{ a, b int }; var e error = nil; const k, l = 1, 2\n",
        "\tm := map[string]\n",
        "\treturn\n",
        "}\n",
        "\n",
        "// X-168: Go's readings where bat's grammar has none or derails\n",
        "func Map[T any, U comparable](xs []T, f func(T) U) (out []U) { return }\n",
        "type Num interface { ~int | ~float64 }\n",
        "func max(a, b int) int { x := y[0] * z; return min(a, b) }\n",
        "func (c *conn) close(done chan<- os.Signal) { clear(c.m); rune, n := 'x', 1 }\n",
        "func t(v any) { switch v.(type) { case *pkg.T, []byte, nil: } }\n",
        "var tmpl = \"{{ if eq .A \\\"x\\\" }}y{{ end }}\" + \"50% off %d %z \\q\"\n",
        "var after = []pkg.T{{A: 1}}\n",
        "var unclosed = \"no end\n",
        "var next = 2\n",
    );
    readings("go", Lang::Go, text);
}

/// Python's readings (X-162, a scanner of ada's own): docstrings (any quote), decorators with and
/// without a call and over lines, dicts, sets and tuples, f-strings with a conversion, a format
/// spec and a nested replacement, PEP 701's quotes and lines in a field, a t-string, regexes at
/// `re`'s calls (raw and cooked) and `match=`, a raw string read as a regex where `re` would take
/// it and not where it wouldn't (a path), SQL by its uppercase keyword and by an execute-like
/// call's first argument (a lone keyword not), printf, strftime and `str.format` placeholders,
/// py2 numbers, a line continuation, `print` as a statement and as a call, PEP 695's type
/// parameters and `type`, `match`/`case`, `except*`, keyword arguments, lambdas' parameters.
#[test]
fn python() {
    let text = concat!(
        "#!/usr/bin/env python3\n",
        "\"\"\"A module's docstring, \\N{BULLET} \\x41.\"\"\"\n",
        "from __future__ import annotations\n",
        "from .pkg import (a as b, c)  # names\n",
        "import os.path, re\n",
        "\n",
        "@dataclass\n",
        "@app.route(\"/x/<id>\", methods=['GET', \"POST\"])\n",
        "class Point(Base, metaclass=ABCMeta):\n",
        "    '''Raw: \\n stays.'''\n",
        "    x: int = 0\n",
        "    MAX_SIZE = 0x1F + 0o17 + 0b101 + 017 + 10L + 1_000j + .5e-3 + 1.\n",
        "\n",
        "    async def go(self, a, /, *args, b: str = \"x\", **kw) -> dict[str, int]:\n",
        "        global count; nonlocal_ = lambda x, *y: x + y\n",
        "        d = {k: v for k, v in kw.items() if k != 'z'}; s = {1, 2}; t = (1, 2)\n",
        "        e = {'a': 1, **d}; f = {*s}; g = [*t]; h = {}; i = ()\n",
        "        msg = f\"{a!r:>{width}} {b=} {{literal}} {x['k']}\" + rf'\\d{{2}}{a}\\w+'\n",
        "        rx = re.compile(r\"^(?P<name>[a-z_]\\w*)\\s*(?:=|:)\\s*(?#c)(.+?)$\", re.I)\n",
        "        sql = \"\"\"\n",
        "            SELECT a.id, COUNT(*) FROM t AS a -- the rows\n",
        "            WHERE a.name = '%s' AND b IS NOT NULL ORDER BY 1 DESC\n",
        "        \"\"\"\n",
        "        old = \"%(name)-10s %5.2f %%\" % vars() + \"{0:>{1}} {name!s}\".format(1, 2)\n",
        "        when = time.strftime('%Y-%m-%d %-H') + b'\\x00\\777' + u'\\u00e9'\n",
        "        print >> sys.stderr, \"x\"\n",
        "        print(x, end=\"\")\n",
        "        x = y if z else \\\n",
        "            \"\"\"not a docstring\"\"\"\n",
        "        with open(p) as fh, lock: pass\n",
        "        try:\n",
        "            yield from gen()\n",
        "        except (KeyError, ValueError) as err:\n",
        "            raise RuntimeError(\"x\") from err\n",
        "        finally:\n",
        "            return await x[1:2], ..., None, __name__, __init__\n",
        "    for i in range(10): del i\n",
        "    while not done: break\n",
        "def first[T: (int, str), *Ts](xs: list[T], /, *, key=lambda v, w=1: v) -> T:\n",
        "    'A docstring in single quotes.'\n",
        "    type Pair[K] = tuple[K, K]\n",
        "    match xs:\n",
        "        case [x, *rest] if x:\n",
        "            pass\n",
        "        case _:\n",
        "            pass\n",
        "    match = re.match(r\"(?P<n>\\w+)=(?P=n)\", s) or re.compile(\"\\\\d+\")\n",
        "    path, tex = r\"C:\\Users\\x\", r\"$\\mathrm{d}x$\"\n",
        "    cur.execute(\"select a from t where b = %(b)s\", {\"b\": 1})\n",
        "    methods = (\"DELETE\", \"GET\")\n",
        "    msg = f\"{x[\"k\"]!r:>{w}} {y = }\" + t\"{z}\"\n",
        "    with pytest.raises(ValueError, match=r\"bad \\d+\"):\n",
        "        pass\n",
        "    try:\n",
        "        pass\n",
        "    except* OSError as eg:\n",
        "        raise\n",
        "@pytest.mark.parametrize(\n",
        "    \"a\", [1, 2],\n",
        ")\n",
        "def test(a): assert a\n",
    );
    readings("python", Lang::Python, text);
}

/// Lua's readings (X-098, a scanner of ada's own): long brackets with levels across lines, every
/// escape and a `\z` escape's spaces across a line, hex floats, a function's dotted name and
/// method, metamethods as keys and properties, the builtin modules' members, labels, a keyword in a
/// function's parameters.
#[test]
fn lua() {
    let text = concat!(
        "#!/usr/bin/env lua\n",
        "--[==[ a long comment\n",
        "]] still ]==] local M = {}\n",
        "local mt = { __index = M, __call = function(self, ...) end, __mode = 'k' }\n",
        "function M.util:run(a, b, ...) -- the method\n",
        "  local s = 'a\\n\\x41\\65\\u{48}\\q' .. \"b\\z   \n",
        "     c\" .. [[x]] .. [=[y\n",
        "]]z]=]\n",
        "  for i = 1, #a do print(a[i], string.format('%d', math.pi), io.stdout) end\n",
        "  for k, v in pairs(t) do if k ~= nil and not v then goto skip elseif x then else end end\n",
        "  ::skip:: while x >= 0x1p4 + 0xA.8 + 1e-3 + .5 + 0xff do x = x // 2 >> 1 end\n",
        "  repeat local y <const> = (a, b) until y == false\n",
        "  self.x.__index = function() return self, _G, _VERSION, true end\n",
        "  t[1]:m{1, [2] = 3; x = f\"s\"}; f[[long]] f'q' (g)(h)\n",
        "  return setmetatable({}, mt), coroutine.wrap(f), utf8.charpattern, x.__metatable\n",
        "end end\n",
        "local f = function(a, end) return a end\n",
    );
    readings("lua", Lang::Lua, text);
}

/// Common Lisp's readings (X-006): every declaration and the frames it sets up, a class's parents
/// and a struct's options, `let`'s, `flet`'s and `macrolet`'s bindings, `declare`'s specifiers,
/// `loop`'s keywords and builtins, `(quote …)` and `'(…)`, namespaces before a head, `|…|` symbols,
/// annotations, every `#` macro, numbers of every base, format directives, nested block comments
/// across lines, a stray `)` and `|#`, parens by depth (X-030), backslashes in symbols as the Lisp
/// reader reads them (G-001).
#[test]
fn lisp() {
    let text = concat!(
        ";;; -*- lisp -*-\n",
        "(defun foo (x &optional (y 2) &rest r) \"Doc ~a~% ~2,'0d ~:@r ~10,2f ~5$\" (car x))\n",
        "(defmacro m (a &body b) `(progn ,a ,@b))\n",
        "(defclass c (pkg:a b) ((s :initarg :s)))\n",
        "(defstruct (pt (:conc-name pt-)) x y) (deftype ty (&optional n) 'integer)\n",
        "(defvar *v* nil) (defparameter *p* t) (defconstant +c+ 3.5d0)\n",
        "(let ((a 1) b) (declare (ignore b) (type fixnum a)) (flet ((f (z) z)) (f a)))\n",
        "(macrolet ((mm (q) q)) (labels ((g () 1)) (g)))\n",
        "(defmethod run :around ((o c) n) (call-next-method))\n",
        "(loop named outer for i from 0 below 10 with s = 0 collect (1+ i) into xs\n",
        "      finally (return xs))\n",
        "(dolist (e l) (dotimes (i 3) (do ((k 0 (1+ k))) ((> k 2)) (print k))))\n",
        "(cl:defun g () (quote (a (b c))) '(d e) 'f #'car #'(lambda (q) q) #'+)\n",
        "(make-pt :x 1) (set! a 1) (null? a) (foo-p a) (|f| x) (when (eq a b) (return-from g))\n",
        "#| block #| nested |# still\n",
        "   comment |# (if t nil) #\\Space #\\a #\\( #2\\x #+sbcl :a #-(or a b) b #. x #:g\n",
        "#1=(a . #1#) #(1 2) #3(a) #*1010 #b101 #B-1/10 #o17 #x1F #16rFF #3r12 #c(1 2)\n",
        "#p\"/tmp/x\" #s(pt :x 1) #2A((1 2)) #a #< 1.5 .5 1. 1/2 -3 +4 1e10 12.x 1.2.3\n",
        "+a+b+ *print-base* array string true false keyword:k :key ::k |a b| &x |#\n",
        "(*x* 1) (&rest) ('a) (t) (:k) (1 2) (\"s\") () (:a . b))\n",
    );
    readings("lisp", Lang::Lisp, text);
}

/// Clojure's readings (X-006): every head (`def…` as a prefix, `defprotocol`, `definterface`,
/// `deftype`, `defrecord`, `defmethod`, `reify`, `proxy`, `extend-*`, `fn`, `declare`) and the
/// tails they set up, the reader macros and dispatch (`#_`, `##Inf`, tagged literals, `#?`, `#:`),
/// metadata, characters, keywords, every number and its suffixes, strings across lines, comments
/// and commas, stray closers, delimiters by depth (X-030).
#[test]
fn clojure() {
    let text = concat!(
        "#!/usr/bin/env bb\n",
        "(in my.app (:require [clojure.string :as str])) ; c\n",
        "(defn- f \"doc\" [x & ys] {:pre [(valid x)]} (str/join \",\" ys))\n",
        "(def ^:private x 1) (def ^{:a 1} y 2N) (default z)\n",
        "(defprotocol P (m [this] \"doc\") (n [this a]))\n",
        "(definterface I (go [])) (deftype T [a] P (m [_] a)) (defrecord R [a b])\n",
        "(defmethod mm :k [x] x) (reify P (m [_] 1)) (proxy [Object] [] (toString [] \"s\"))\n",
        "(extend-protocol P String (m [s] s)) (extend-type T P (m [_] nil))\n",
        "(fn [x] x) (fn* [y] y) (fn name [z] z) (declare a b) (nil 1) (true) (f / 2)\n",
        "[1 -2 +3 012 1N 0x1F 0XffN 2r101 36rZZ 1/2 1.5 1.5e10 1e5 1M 1.5M 1. 1# x# x']\n",
        "{:k 'q `r ~s ~@t @u ::v :a/b \\a \\newline \\( #_ (skip) ##Inf ##-Inf #inst \"2020\"}\n",
        "#{1 2} #(g % 1) #?(:clj 1 :cljs 2) #?@(:clj [1]) #:a{:b 1} #'var , \"multi\n",
        "line\" \"esc\\\"aped\" ) ] } (h [a 1] a)\n",
    );
    readings("clojure", Lang::Clojure, text);
}

/// SQL's readings (ADR-40, X-124): `create`'s name, `comment on … is`, `alter`, the types' sizes
/// and time zones, `is not null`, the multi-word DML, quotes doubled and escaped, interpolation,
/// `%{}` and `%r{}`, a block comment's `*` lines, `#` and `--` comments. The dialects' words,
/// variables and dollar quotes are pinned in ada-parse's `sql` tests.
#[test]
fn sql() {
    let text = concat!(
        "-- schema\n",
        "CREATE OR REPLACE FUNCTION s.\"f\"(a int)\n",
        "CREATE UNIQUE INDEX\n",
        "  idx;\n",
        "create table `t` (id bigint primary key, n varchar(255) not null,\n",
        "  p numeric(10,2) default 0, d timestamp(3) with time zone, c character varying(10),\n",
        "  x double precision, y bit varying(3), z var char(2), f float(8),\n",
        "  FOREIGN KEY (a) REFERENCES u(id) ON DELETE CASCADE, CHECK (n <> ''));\n",
        "DROP VIEW x;\n",
        "ALTER TABLE t COMMENT ON COLUMN a.b IS 'x';\n",
        "  COMMENT ON TABLE t IS 'it''s \\'q\\'';\n",
        "/* block\n",
        " * star\n",
        " */ SELECT DISTINCT a.b, count(*), COUNT (x), counter(y), current_user, lower(n)\n",
        "FROM t NATURAL LEFT OUTER JOIN u INNER JOIN v USING (id) # mysql\n",
        "WHERE x IS NOT NULL AND y not null OR z IS null AND q <=> 1 AND r != 2 || 'a'\n",
        "GROUP BY a ORDER  BY b DESC LIMIT 10 UNION ALL SELECT TOP 5 * FROM w;\n",
        "INSERT IGNORE INTO t VALUES (1, \"a\"\"b #{x} c\", %{p #{q} r}, %r{s}, ());\n",
        "BEGIN WORK; START TRANSACTION; GRANT WITH GRANT OPTION; REVOKE; COMMIT;\n",
        // a fraction or an exponent (`1.5`, `1e3`) reads as one number, bat's as two or
        // none: ada-parse's `decimals_and_exponents` (X-008)
        "SELECT 1 AS one, true, FALSE, 12ab, ٣, x.y.z ASC;\n",
    );
    readings("sql", Lang::Sql, text);
}

/// PowerShell's readings (ADR-40): `#requires` with a hashtable, `using namespace`, a block
/// comment's docs, `function` with a scope, attributes and their parameters, types, commands with a
/// path before them, every variable form, operators after `-`, numbers with suffixes and units,
/// strings with escapes and interpolation, here-strings, `--%`, `class`, `%` and `?`, dot-sourcing,
/// a range, a type's name after a digit.
#[test]
fn powershell() {
    let text = concat!(
        "#requires -Version 5.1 -Modules @{ ModuleName = 'x'; RequiredVersion = '1.0' }\n",
        "using namespace System.IO.\n",
        "<# .SYNOPSIS\n",
        "  Does x.\n",
        ".PARAMETER Name\n",
        "  The name.  #> <# .NOTES\n",
        "#>\n",
        "function global:Get-Thing {\n",
        "    [CmdletBinding(SupportsShouldProcess = $true)]\n",
        "    param([Parameter(Mandatory, Position=0)][ValidateSet('a', 'b')][string[]] $Name, [int]$N = 0x1FUL)\n",
        "    begin { $ErrorActionPreference = 'Stop' } process {\n",
        "        Get-ChildItem C:\\x\\Get-Foo.ps1 -Path $env:TEMP 2>&1 | ForEach-Object { $_.FullName } | Where-Object Length -gt 10mb\n",
        "        $h = @{ 'k' = 1; Name = \"a $x `n $($y.z) `u{1F600} `u{ \"\"q\"\" ${env:path}.Length\" }; $a = @(1, 2..3, -.5, 1.5e3d, 1e1, _)\n",
        "        $s = @\"\n",
        "text $v `t $(Get-Date) \"\" ${global:x}\n",
        "\"@\n",
        "        $t = @'\n",
        "raw $v ''\n",
        "'@\n",
        "        if ($global:x -is [string] -and -not $script:y.Count -ceq 1 -or $z -bxor 2) { cmd.exe /c x --% raw %PATH% } elseif ($?) { return $null } else { throw }\n",
        "        $x.foreach({ $PSItem }) ; $y | % { $_ } ; x.% foo ; 'it''s' + \u{2018}smart\u{2019} -f $Host.UI.RawUI\n",
        "    }\n",
        "}\n",
        "class Foo : Bar { hidden static [string] $Name }  # done ## x `\n",
        "filter f { $input }; .\\x.ps1 ; . $p; [System.IO.File]::Exists($p) ; [2abc] ; a2>> b ; \"x `\n",
        "y\"\n",
    );
    readings("powershell", Lang::PowerShell, text);
}

/// JavaScript's readings (X-058's scanner, of ada's own): a shebang, labels and quoted keys with
/// functions and arrows, classes with properties, static and async methods, an object's accessor,
/// prototype assignments, a regex after `(` and `=` but a division after a name, a template with a
/// substitution, escapes and a line continuation, JSX across lines with attributes, entities and
/// expressions, Flow annotations, `-Infinity`, hex, octal and legacy octal numbers, `{{`, `switch`
/// and `for`, a constructor, node's and the console's objects, `<!--` and a line's `-->`.
#[test]
fn javascript() {
    let text = concat!(
        "#!/usr/bin/env node\n",
        "/** The module. */\n",
        "import React, { useState } from 'react'; // a comment\n",
        "export default class Foo extends Bar {\n",
        "  static async *load(id, ...rest) { await fetch(`/api/${id}?q=${rest[0]}`); }\n",
        "  static count = 0\n",
        "  state = { open: false, 'quoted': \"x\\ty\\u00e9\" }\n",
        "  @observable value: string = 'v'\n",
        "  render() {\n",
        "    const { a, b } = this.props, n = a ?? b ?? 1;\n",
        "    return (\n",
        "      <div className=\"box\" onClick={() => this.toggle(n)}>\n",
        "        &amp; {n > 1 && <Item key={n} label='one' />}\n",
        "      </div>\n",
        "    );\n",
        "  }\n",
        "}\n",
        "Foo.prototype.reset = function reset() { this.n = 0x1F + 0o17 + 07 + 1e-3 + -Infinity; };\n",
        "Foo.prototype.add = (a, b = 2) => a + b;\n",
        "const re = /abc/gi, half = total / 2, ok = re.test(s) && !(x instanceof Y);\n",
        "const obj = {\n",
        "  get size() { return this.items.length; },\n",
        "  method: function (x) { return typeof x === 'number'; },\n",
        "  arrow: async (y) => y * 2,\n",
        "  'key': value,\n",
        "  [computed]: null,\n",
        "};\n",
        "switch (kind) { case 'a': break; default: console.log(process.env.HOME); }\n",
        "for (let i = 0; i < 10; i++) { module.exports[i] = new Map(); }\n",
        "const s = \"line \\\n",
        "next\"; const t = `multi\n",
        "line ${ {{ }} }`;\n",
        "function* gen(a: number, b?: string) { yield* other(); }\n",
        "if (x) { throw new TypeError('bad'); } else { debugger; }\n",
        "<!-- legacy\n",
        "-->\n",
        "const Box = styled(Base).attrs({ role: 'x' })`\n",
        "  color: ${(p) => p.c}; &:hover { top: 0 }\n",
        "  ${({ on }) => on && css`margin: 0;`}\n",
        "`, fade = keyframes`from { opacity: 0 }`, q = gql`{ a }`;\n",
    );
    readings("javascript", Lang::JavaScript, text);
}

/// TypeScript's readings (X-091's scanner, of ada's own): declarations of every kind (interfaces,
/// type aliases with conditional, mapped and template literal types, enums, namespaces, abstract
/// classes with modifiers, decorators, overloads, accessors and index signatures), generics in
/// calls and arrows, the `<T>x` cast, `as const`, optional chaining, destructuring, a `const` at a
/// line's end, JSDoc with types, a regex with groups and classes, templates with a tag, scrambled
/// lines, and objects nested past the 28 levels a state keeps.
#[test]
fn typescript() {
    let mut text = String::from(concat!(
        "/// <reference path=\"./types.d.ts\" />\n",
        "import type { A, B as C } from './a';\n",
        "import * as fs from 'fs';\n",
        "export interface Point<T extends number = number> { readonly x: T; y?: T; [key: string]: unknown }\n",
        "type Maybe<T> = T extends null | undefined ? never : T;\n",
        "type Keys = { [K in keyof Point as `get${Capitalize<K&string>}`]: () => Point[K] };\n",
        "export enum Color { Red = 1, Green = 'g', ['Blue'] }\n",
        "declare namespace NS { export const v: number; }\n",
        "/**\n",
        " * Makes one.\n",
        " * @param {string} name - the name\n",
        " * @returns {Promise<Foo>} the one\n",
        " * @example\n",
        " * make('a');\n",
        " */\n",
        "@Component({ selector: 'app' })\n",
        "export abstract class Foo<T> extends Base<T> implements I {\n",
        "  private static readonly count: number = 0;\n",
        "  #secret = 1;\n",
        "  constructor(public name: string, protected opts?: Options) { super(); }\n",
        "  get size(): number { return this.items?.length ?? 0; }\n",
        "  abstract run(): void;\n",
        "  async *load<U>(id: U, ...rest: unknown[]): AsyncGenerator<U> { yield* other<U>(id); }\n",
        "  handler = (e: Event): void => { console.log(e.target as HTMLElement); };\n",
        "}\n",
        "function over(a: string): string;\n",
        "function over(a: any) { return <string>a; }\n",
        "const { a, b: [c, d] } = obj, n = a! + 0x1F + 1_000n;\n",
        "const pick = <T, K extends keyof T>(o: T, k: K): T[K] => o[k];\n",
        // a regex's body is ECMAScript's grammar's since phase 4 (ada-parse's `typescript`
        // tests), not bat's TypeScript's: these few tokens color alike in both
        "let re = /x+$/gi, half = total / 2;\n",
        "const tag = html`<p>${name}</p>`, plain = `a ${b ? 'c' : \"d\"}`;\n",
        "const conf = { mode: 'x', list: [1, 2] } as const;\n",
        "const\n",
        "  late = typeof window !== 'undefined' && window instanceof Object;\n",
        "for (const [k, v] of Object.entries(map)) { if (k in v) continue; }\n",
        "switch (kind) { case 'a': break; default: throw new Error(`bad ${kind}`); }\n",
        "export default async function main(): Promise<void> { await Promise.all([]); }\n",
        // corners met in scrambled input
        "interface I { a : () => void; b? : () => void; c: () => void }\n",
        "import type = x;\n",
    ));
    // objects nested past 28 frames: the bottom ones dropped between lines
    text.push_str("const deep = {\n");
    for _ in 0..30 {
        text.push_str("key: {\n");
    }
    for _ in 0..30 {
        text.push_str("},\n");
    }
    text.push_str("};\n");
    text.push_str(
        "const B = styled.button<{ $on: boolean }>`color: ${(p) => (p.$on ? 'red' : 'blue')};`;\n",
    );
    text.push_str("static styles = css`:host { display: block; }`;\n");
    readings("typescript", Lang::TypeScript, &text);
}

/// TSX's readings (TypeScript's scanner with JSX): elements with and without attributes, components
/// with namespaces and type arguments, entities in children and in attribute strings, expressions
/// as children and values, a generic arrow `<T,>` that is no function for a name, and `<T extends
/// …>` that is.
#[test]
fn tsx() {
    let text = concat!(
        "import React from 'react';\n",
        "export const List = <T,>({ items }: Props<T>) => (\n",
        "  <ul className=\"list\" data-n='1 &amp; 2'>\n",
        "    {items.map((item, i) => <Item<T> key={i} value={item} />)}\n",
        "    <svg:rect width={10} /> &copy; &#169; &#xA9; & more\n",
        "    <></>\n",
        "  </ul>\n",
        ");\n",
        "const pick = <T extends object>(o: T) => o;\n",
        "function App(): JSX.Element {\n",
        "  return <Layout.Main title={`a ${b}`} {...rest}>\n",
        "    <p>text</p>\n",
        "  </Layout.Main>;\n",
        "}\n",
        "const ns = <svg:>y</svg:>;\n",
        "const G = createGlobalStyle`body { margin: 0 }`;\n",
    );
    readings("tsx", Lang::TypeScriptReact, text);
}

/// CSS's readings (ADR-40's phase 6): every at-rule and its prelude, a comment across lines,
/// selectors with custom elements, classes, ids, pseudo elements and classes and their functions,
/// attribute selectors with a namespace and a flag, combinators bad and good; properties with
/// vendor prefixes, custom and deprecated ones, `font`'s unquoted names; the functions' groups,
/// numbers of each type with units, colors, constants, strings with escapes and an escaped newline,
/// unicode ranges, `!important` and `progid`.
#[test]
fn css() {
    let text = concat!(
        "@charset \"utf-8\"; x\n",
        "@import url(\"a.css\") screen and (orientation: landscape);\n",
        "@namespace svg url(http://www.w3.org/2000/svg);\n",
        "@custom-media --narrow (max-width: 30em);\n",
        "/* a comment\n",
        " * across lines */\n",
        "@media only screen and (-webkit-min-device-pixel-ratio: 2), print and (aspect-ratio: 16/9) {\n",
        "  my-element > .btn-primary:hover::before, a[href$=\".pdf\" i] ~ #main *, svg|rect >>> b {\n",
        "    -webkit-transition: opacity .3s ease-in-out, transform 0.5s cubic-bezier(.17, .67, .83, .67);\n",
        "    --main-color: #ffcc00aa; var-x y; color: var(--main-color, red) !important;\n",
        "    font: italic bold 12px/30px Georgia, \"Times New Roman\", serif;\n",
        "    background: url(img.png) no-repeat, linear-gradient(to right, rgba(0, 0, 0, .5) 0%, #fff 100%);\n",
        "    grid-template-columns: repeat(auto-fill, minmax(100px, 1fr)) [line-a] auto-flow;\n",
        "    transform: translateX(-10%) rotate(45deg) scale(1.5) translateZ(0.5px); width: calc(100% - (2 * 1.5em));\n",
        "    content: \"\\201C x \\\"\" attr(data-x px, 1); filter: blur(10) progid:DXImageTransform.Microsoft.gradient(startColorstr='#80000000');\n",
        "    list-style: decimal-leading-zero; counter-reset: c 1; content: counter(c, upper-roman) counters(c, \".\", lower-alpha);\n",
        "    margin: 0 auto -1.5e2px +2em; z-index: 10; line-height: 1.5; color: CurrentColor; outline: 1px solid aliceblue;\n",
        "    content: 'a\\\n",
        "b';\n",
        "  }\n",
        "  li:nth-child(2n + 1):not(.a, :first-child) { clip: rect(0, auto, 1px, 2px) }\n",
        "}\n",
        "@font-face { font-family: \"X\"; src: local(Arial), url(x.woff2) format(\"woff2\"); unicode-range: U+0025-00FF, u+4??; }\n",
        "@keyframes spin { from { transform: rotate(0deg) } 50% { opacity: .5 } to { transform: rotate(1turn) } }\n",
        "@-webkit-keyframes a, b { }\n",
        "@supports (display: grid) and (not (display: inline-grid)) { .x { display: grid } }\n",
        "@page :first { margin: 1in }\n",
        "@counter-style thumbs { system: cyclic; symbols: \"x\"; }\n",
        "@document url(http://x), url-prefix(y), domain(z), regexp(\"a\") { b { c: d } }\n",
        "div.a#b, h1 + p, clipPath, :root, ::-moz-selection, :lang(en), :drop(active) { -moz-box-sizing: border-box; }\n",
        "@tailwind base; .é-x { top: 5q; transition: all 200ms steps(4, jump-end); color: hsl(120deg, 50%, 50%) }\n",
        "@layer reset, base;\n",
        "@layer base {\n",
        "  .card { color: oklch(70% 0.1 200 / 50%); &:hover { color: color-mix(in srgb, red 50%, blue) }\n",
        "    > p { margin-inline: clamp(1rem, 2.5vw + 1rem, 3rem) } .x & { top: 0 }\n",
        "    a:focus{top:0}b{width:1px} @media (width >= 600px) { padding: 2px } --theme: { a: b };\n",
        "  }\n",
        "}\n",
        "@container sidebar (inline-size > 30em) and style(--on: true) { .c { container-type: inline-size } }\n",
        "@property --angle { syntax: '<angle>'; inherits: false; initial-value: 0deg; }\n",
        "@scope (.card) to (.content) { :scope > img { aspect-ratio: 16 / 9 } }\n",
        "@starting-style { DIV:has(> img), *|*, [lang|=en], [data-x=y s] { opacity: 0 } }\n",
    );
    readings("css", Lang::Css, text);
}

/// SCSS's readings (CSS's scanner with Sass's, X-149): the front matter's YAML, `//` comments,
/// variables (namespaced, declared, set in a module, with flags), maps with keys of both kinds,
/// interpolation in selectors (a name it breaks going on after it), property names, custom
/// properties, strings, URLs and values, every Sass at-rule with its prelude, mixins with
/// parameters and includes with arguments and `using`, `&` and its suffixes, placeholders, nested
/// properties, Sass's functions and custom ones, `if(`'s `;`, operators and a sign's two readings,
/// the CSS at-rules in SCSS's form, css-modules, a property bat's list lacks.
#[test]
fn scss() {
    let text = concat!(
        "---\n",
        "title: x\n",
        "---\n",
        "@use \"sass:math\" as m;\n",
        "@use \"a\" as *; @forward \"b\" show f, $g; @use \"c\" with ($d: 1);\n",
        "@import \"x\", url(y) screen; @import (reference) \"z\";\n",
        "// a line comment\n",
        "$map: (key: 1px, 'k2': (a: #fff, $v: red), b: math.$pi) !default;\n",
        "$x: 10px - 2 * 3 / 4 !global;\n",
        "@mixin m($a, $b: 10px) { width: $a + $b; @content; }\n",
        "@function f($x) { @return math.div($x, 2) * 1%; }\n",
        "@each $k, $v in $map { .icon-#{$k} { color: map-get($map, $k); } }\n",
        "@for $i from 1 through 3 { .m-#{$i} { margin: #{$i * 4}px -2px; } }\n",
        "@if $a == b and not $c { @warn \"w #{$d}\"; } @else if $a != c { @debug $x; } @else { @error 'e'; }\n",
        "@while $i > 0 { $i: $i - 1; }\n",
        "%ph { a: b; } .a { @extend %ph; @extend .b !optional; }\n",
        ".btn, #id > a:hover::before, my-el:not(.x, y), &__el, & + &, [data-x=\"y\" i] {\n",
        "  @include ns.mx(1px, $b: 2) { c: d; }\n",
        "  @include bp(md);\n",
        "  font: { family: \"X\", sans-serif; size: 12px; }\n",
        "  border: { top-width: 1px; }\n",
        "  margin-#{$side}: 0; --#{$v}: 1; --custom: #{$z};\n",
        "  -webkit-transition: opacity .3s, transform 0.5s cubic-bezier(.17, .67, .83, .67);\n",
        "  background: url(#{$p}/img.png) no-repeat, linear-gradient(to right, rgba($c, .5) 0%, darken($c, 10%) 100%);\n",
        "  width: calc(100% - #{$gutter}) clamp(1px, 2vw, 3px); color: Red; content: \"a#{$b}c\\\"\" + d;\n",
        "  transform: translate(-50%, 1px) rotate(45deg); grid: repeat(auto-fill, minmax(10px, 1fr)) / auto;\n",
        "  @at-root .x { y: z; }\n",
        "}\n",
        "@media screen and (min-width: $bp) and (orientation: landscape), print { .p { q: r } }\n",
        "@keyframes #{$n} { from { a: b } 50% { c: d } to { e: f } }\n",
        "@supports (display: grid) and (not (x: y)) { .s { t: u } }\n",
        "@font-face { src: url(x.woff2) format(\"woff2\"); } @charset \"utf-8\";\n",
        "@page :first { margin: 1in } @counter-style thumbs { system: cyclic; }\n",
        ":export { primary: $primary; } :global(.g) { h: i }\n",
        "a { b: if(true, null, false) nth($l, 1) e(\"x\") my-fn(1) ns.fn(2) 1px+2px; }\n",
        ".btn-#{$c}-lg, .#{$a}#{$b}x-#{$d}-y, #{$parent}--mod, %ph-#{$p} { a: b; }\n",
        ".c { --#{$p}gap: 1; gap: var(--#{$p}gap) var(#{$p}gap); bg: url(#{$u}/i.png) url($v); }\n",
        ".d { inset-inline: 0; top: -10px; m: 10px -5px (1)-2; @include x using ($e) { f: $e; } }\n",
        "@media #{$mq} { .e { w: if(sass($m): 2; else: 1); } }\n",
        "@use 'cfg' as c; c.$x: 1; .f { @include c.mx; g: color-mix(in srgb, red, $h...); }\n",
    );
    readings("scss", Lang::Scss, text);
}

/// HTML's readings, with CSS's and JavaScript's scanners for what HTML embeds (ADR-40's phase 6):
/// the doctype with its external id and internal subset, `<?xml`, comments good and bad, CDATA,
/// every tag list and its end, custom tags with bad characters, attributes of each kind with
/// quoted, unquoted and bad values, character references, `<style>` and `<script>` of each type
/// with their closing tags (`</style >` a tag to its `>`), `style="…"` and `on…='…'`; each embed
/// ended where a browser ends it (X-120): a script at its first `</script`, a style at its first
/// `</style`, an attribute's value at its quote, wherever they stand (a `//` or `/* */` comment, a
/// string, a template, a regex and its class, `\'`), not at `</scripts>`.
#[test]
fn html() {
    let text = concat!(
        "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0//EN\" 'x.dtd' [ <!-- s --> ]>\n",
        "<?xml version=\"1.0\" encoding='utf-8'?>\n",
        "<!doctype html><!-- a <!-- b --!> c --> <!--> x --> <![CDATA[ <b> ]]> <>\n",
        "<html lang=en><head><meta charset=\"utf-8\"><title>T &amp; &#169; &#x1F600; &x</title>\n",
        "<link rel=stylesheet href='a.css'/><base href=/x/></head>\n",
        "<body class=\"a b\" id='main' data-x=\"1\" hidden><div><p>x<br/><hr/><HR>\n",
        "<form action=\"/go\"><fieldset><input type=text value=\"a&quot;b\" disabled/><button>b</button>\n",
        "<a href=\"#x\" a\"b=1 c=d\"e`f<g>l</a><table><tr><td>1</td></tr></table></fieldset></form>\n",
        "<my-el foo=bar></my-el><x-É-Y.z\\.w></x-É-Y.z\\.w><foo:bar baz='1'/></foo:bar><svg-x/>\n",
        "<span style=\"color: red; font-family: Arial\" onclick=\"go(1); return false\">s</span>\n",
        "<style type=\"text/css\" media=print>\n",
        "  body > .a::before { color: #fc0; margin: 0 -1em; }\n",
        "  /* c */ @media (min-width: 40em) { a:hover { b: c } }\n",
        "</style><style type=text/less>@x: 1;</style><style>a { b: c }</style >x\n",
        "y</style><style/>\n",
        "<script>\n",
        "<!--\n",
        "  var x = 1 < 2 && \"a\" || 'b'; // a comment\n",
        "  function f(a) { return `t ${a}` / 2; }\n",
        "-->\n",
        "</script>\n",
        "<script type=\"module\" src=x.js></script><script type='text/html'><b>t</b></script>\n",
        "<script type=\"text/template\"><div>{{x}}</div></script><script>f()</SCRIPT >\n",
        "<script>r = /[a</SCRIPT>b]/; s = /x[^c</script>d]/g;</script>\n",
        "<b onclick=\"r = /[a\" c=\"]/\" onmouseover='x = /a[^b'>x</b>\n",
        "<script>a = 1; // x </script>y<script>s = \"a</script>b\"; t = `c</Script\t>d`</script>\n",
        "<script>/* c </script>*/ u = 1 </scripts> + 2;</script><style>a { content: \"x</style>\" }</style>\n",
        "</div></body></html>\n",
        // an escaped quote and a `//` comment end at the attribute's quote
        "<i style='content:\"\\'x\"' onmouseover='a // b'>i</i>\n",
    );
    readings("html", Lang::Html, text);
}

/// Dockerfile's readings (ADR-40): instructions, their flags and arguments, `ONBUILD`, JSON arrays,
/// continuations, comments and parser directives (here-documents on bash's scanner are pinned in
/// ada-parse's `dockerfile` tests, X-105).
#[test]
fn dockerfile() {
    let text = concat!(
        "# syntax=docker/dockerfile:1\n",
        "ARG BASE=alpine\n",
        "FROM rust:1.88-slim@sha256:abc AS build\n",
        "  # indented comment\n",
        "WORKDIR /src\n",
        "COPY --from=deps /cache /cache\n",
        "copy . .\n",
        "RUN apt-get update && \\\n",
        "    apt-get install -y \"curl\" 'jq' \\\n",
        "\n",
        "    && echo \"a \\\" b\"\n",
        "ENV PATH=\"/x:$PATH\" \\ \n",
        "ONBUILD RUN make\n",
        "MAINTAINER someone\n",
        "EXPOSE 8080\n",
        "RUN\n",
        "echo continues\n",
        "CMD [\"ada\", \"--help\"]\n",
        "ENTRYPOINT [\"ada\"]\n",
        "LABEL x=\"open\n",
        "lower case line\n",
        "HEALTHCHECK CMD true\n",
        "FROM scratch\n",
        "garbage here\n",
    );
    readings("dockerfile", Lang::Dockerfile, text);
}

/// Java properties' readings (ADR-40): comments, keys with escapes, the three separators, values
/// with escapes and continuations.
#[test]
fn properties() {
    let text = concat!(
        "# comment\n",
        "  ! another\n",
        "key = value\n",
        "key2:value2\n",
        "key3 value3\n",
        "a\\ b = c\\td \\u00e9 \\uZZ\n",
        "multi = one, \\\n",
        "  two, \\\n",
        "  three\n",
        "cont\\\n",
        "inued = x\n",
        "empty=\n",
        "   \n",
        "lone\n",
    );
    readings("properties", Lang::Properties, text);
}

/// CMakeCache's readings (X-104): comments, keys, types and values.
#[test]
fn cmake_cache() {
    let text = concat!(
        "# This is the CMakeCache file.\n",
        "//Path to a program.\n",
        "CMAKE_AR:FILEPATH=/usr/bin/ar\n",
        "K-2:STATIC=\n",
        "  K:BOOL=ON # x\n",
        "KEY\n",
        ":STRING=a\n",
        "K:T==x\n",
        "K:A B.c;v\n",
        "x // c\n",
    );
    readings("cmake_cache", Lang::CMakeCache, text);
}

/// fstab's readings (ADR-40): comments and the six fields.
#[test]
fn fstab() {
    let text = concat!(
        "# <file system> <dir> <type> <options> <dump> <pass>\n",
        "UUID=ab12\t/\text4\trw,relatime,x=1\t0 1\n",
        "/dev/sda2 none swap sw 0 0\n",
        "server:/export /nfs nfs4 _netdev 0 0 # c\n",
        "//srv/share /mnt cifs user=a 3 x y\n",
    );
    readings("fstab", Lang::Fstab, text);
}

/// pkg-config's readings (X-146, ada's own): variables, fields by their key, comments wherever a
/// `#` stands, escapes, a continued value, a template's placeholders.
#[test]
fn pkgconfig() {
    let text = concat!(
        "# zlib pkg-config file\n",
        "prefix=@CMAKE_INSTALL_PREFIX@\n",
        "exec_prefix = ${prefix}\n",
        "libdir=${exec_prefix}/lib # the libraries\n",
        "sharp=a\\#b$$c\n",
        "@pc_data@\n",
        "\n",
        "Name: zlib\n",
        "Description: zlib compression library, PKCS#11 cut here\n",
        "Version: @VERSION@\n",
        "URL: https://zlib.net/\n",
        "Requires: glib-2.0 >= 2.50, gio-2.0,libffi\n",
        "Requires.private: x<=1 y != 2\n",
        "Libs: -L${libdir} -lz@SUFFIX@ /usr/lib/libm.so\n",
        "Libs.Private: -pthread\n",
        "CFlags: -I${includedir} \\\n",
        "  -DZ=1 \\\n",
        "  -I/opt/x\n",
        "Unknown: some text\n",
        "not a key line\n",
    );
    readings("pkgconfig", Lang::PkgConfig, text);
}

/// `known_hosts`' readings (ADR-40): markers, host patterns, hashed hosts, key types and keys,
/// comments.
#[test]
fn known_hosts() {
    let text = concat!(
        "# c\n",
        "@revoked [a,b*]:22,1.2.3.4,::1 ssh-rsa AAAAB3NzaC1yc2E= a label\n",
        "|1|abcdefghijklmnopqrstuvwxyz0=|abcdefghijklmnopqrstuvwxyz0= ssh-dss x\n",
        "@cert-authority !*.example.com ecdsa-sha2-nistp256 AAAAE2VjZHNh\n",
    );
    readings("known_hosts", Lang::KnownHosts, text);
}

/// SSH Config's readings (X-060): keywords, `Host` and `Match` blocks, values by kind, a command's
/// value on bash's scanner (a command's name in bash's color for it, X-093).
#[test]
fn ssh_config() {
    let text = concat!(
        "# a comment\n",
        "Host github.com *.example.org !bad ?x\n",
        "  HostName 192.168.1.10\n",
        "  User git\n",
        "  Port 2222\n",
        "  IdentityFile ~/.ssh/id_ed25519\n",
        "  ForwardAgent yes\n",
        "  StrictHostKeyChecking ask\n",
        "  Ciphers +aes128-ctr,chacha20-poly1305@openssh.com,3des-cbc\n",
        "  HostKeyAlgorithms -ssh-dss,ssh-ed25519\n",
        "  ServerAliveInterval \"60s\"\n",
        "  LocalForward 8080 localhost:80\n",
        "  ProxyCommand ssh -W %h:%p bastion\n",
        "  ControlPath ~/.ssh/cm-%r@%h:%p\n",
        "  SendEnv LANG LC_*\n",
        "Match host *.corp user admin exec \"test -f /tmp/x\"\n",
        "  ForwardX11 no\n",
        "Match originalhost x canonical all\n",
        "  LocalCommand echo %n\n",
    );
    readings("ssh_config", Lang::SshConfig, text);
}

/// SSHD Config's readings (X-060): keywords, `Match` blocks, values by kind, `ForceCommand`'s value
/// on bash's scanner.
#[test]
fn sshd_config() {
    let text = concat!(
        "# $OpenBSD$\n",
        "Port 22\n",
        "ListenAddress 0.0.0.0\n",
        "PermitRootLogin no\n",
        "KexAlgorithms curve25519-sha256,diffie-hellman-group14-sha1\n",
        "MACs hmac-sha2-256,hmac-md5\n",
        "AuthorizedKeysFile .ssh/authorized_keys %h/.ssh/keys\n",
        "RekeyLimit 1G 1h\n",
        "Banner none\n",
        "Subsystem sftp /usr/lib/ssh/sftp-server\n",
        "Match User anoncvs,!root Address 10.0.0.0/8 LocalPort 2222\n",
        "  X11Forwarding no\n",
        "  ForceCommand cvs server\n",
        "Match all\n",
    );
    readings("sshd_config", Lang::SshdConfig, text);
}

/// Crontab's readings (X-059): variables, schedules, `@` nicknames, a command on bash's scanner to
/// its line's end.
#[test]
fn crontab() {
    let text = concat!(
        "# m h dom mon dow command TODO: x\n",
        "SHELL=/bin/bash\n",
        "MAILTO=  # nobody\n",
        "CRON_TZ=UTC\n",
        "*/5 * * * * /usr/bin/php /var/www/cron.php > /dev/null 2>&1\n",
        "15,45 1-5 1-31/2 Jan-Mar mon-fri echo $HOME; ls # done\n",
        "0 0 15W * 5L cmd\n",
        "0 0 ? x5 MON#2 cmd\n",
        "60 24 0 13 8 cmd\n",
        "5, ,5 ** */ Sunday cmd\n",
        "00 1-2-3 H H(0-5) * cmd\n",
        "  0 0 * * *\tcd /srv && ./run.sh \"$(date +%F)\"\n",
        "@reboot sleep 60 && /opt/start.sh\n",
        "@daily#c\n",
        "@weekly\n",
        "@foo bar # c\n",
        "garbage line # c\n",
        "  abc def\n",
        "cron x\n",
    );
    readings("crontab", Lang::Crontab, text);
}

/// nginx's readings (X-061): directives, blocks, variables, strings, regexes in `location` and
/// `if`, numbers with units, comments.
#[test]
fn nginx() {
    let text = concat!(
        "\u{feff}user  www-data;  # who\n",
        "worker_processes  auto;\n",
        "events {\n",
        "    worker_connections  1024;\n",
        "}\n",
        "http {\n",
        "    include       mime.types;\n",
        "    log_format  main  '$remote_addr - $remote_user [$time_local] \"$request\" '\n",
        "                      '$status $body_bytes_sent';\n",
        "    sendfile on; keepalive_timeout 65s;\n",
        "    map $http_upgrade $connection_upgrade {\n",
        "        default upgrade;\n",
        "        ''      close;  # none\n",
        "    }\n",
        "    upstream backend {\n",
        "        server 10.0.0.1:8080 weight=5;\n",
        "        ip_hash;\n",
        "    }\n",
        "    server {\n",
        "        listen 80 default_server;\n",
        "        listen [::]:443 ssl http2;\n",
        "        server_name example.com *.example.com;\n",
        "        allow 192.168.1.0/24; deny all;\n",
        "        location ~* \\.(jpg|png|gif)$ { expires 30d; }\n",
        "        location = /50x.html { root html; }\n",
        "        location ^~ /static/ { alias /var/www/static/; }\n",
        "        location /api {\n",
        "            proxy_set_header Host $host;\n",
        "            proxy_pass http://backend;\n",
        "            add_header Cache-Control \"public, max-age=300\" always;\n",
        "        }\n",
        "        if ($http_user_agent ~ MSIE) {\n",
        "            rewrite ^(.*)$ /msie/$1 break;\n",
        "        }\n",
        "        if ($request_uri !~* \"^/ok\") { return 403; }\n",
        "        rewrite \\.php$ /index.php last;\n",
        "        set $a \"x\\\"y $b\";\n",
        "        client_max_body_size 10m;\n",
        "        stub_status;\n",
        "        limit_req zone=one burst=5;\n",
        "        brotli on;\n",
        "        error_page 500 502 503 504 /50x.html;\n",
        "    }\n",
        "    types {\n",
        "        text/html  html htm;\n",
        "        application/vnd.ms-excel xls;\n",
        "    }\n",
        "}\n",
    );
    readings("nginx", Lang::Nginx, text);
}

/// PHP's readings (X-099, a scanner of ada's own): a file that opens with `<?php` and a template's
/// HTML, namespaces and uses, a doc comment, classes, functions and their parameters,
/// interpolation, heredocs and nowdocs, SQL in a string, casts, closures, `?>` and islands in HTML.
/// Departures from bat's grammar (attributes, HTML one stream across PHP's islands, an indented
/// heredoc closer) are pinned in ada-parse's `php` tests.
#[test]
fn php() {
    let text = concat!(
        "<?php\n",
        "namespace App\\Http;\n",
        "\n",
        "use Foo\\Bar as Baz, Qux;\n",
        "use function strlen;\n",
        "use const PHP_EOL;\n",
        "\n",
        "/**\n",
        " * A doc, {@inheritdoc}.\n",
        " * @param int $x the x\n",
        " * @return string\n",
        " */\n",
        "abstract class Foo extends Bar implements \\Countable, Baz\n",
        "{\n",
        "    use SomeTrait;\n",
        "    const ANSWER = 42;\n",
        "    public static ?int $count = 0;\n",
        "    protected array $items = [];\n",
        "\n",
        "    public function __construct(private Foo $foo, int $n = 3, ...$rest)\n",
        "    {\n",
        "        parent::__construct();\n",
        "        $this->name = strtolower($foo->name . 'x');\n",
        "        $y = self::ANSWER + static::$count++ ?? Foo::class;\n",
        "        if ($y instanceof \\DateTime && !empty($rest)) {\n",
        "            return array_map(fn($a) => $a * 2, $rest);\n",
        "        }\n",
        "        echo \"Hello $name, {$this->name}, $a->b, $c[0] and ${x} \\n\";\n",
        "        $q = MY_CONST . E_ALL . PHP_EOL . true . null;\n",
        "        $r = Foo\\Bar\\baz(1, 2.5, 0x1F, 0b101, 1_000);\n",
        "        $s = $obj?->prop['key'][0] <=> $t;\n",
        "        foreach ($list as $k => &$v) { continue; }\n",
        "        try { throw new \\RuntimeException(\"bad\"); } catch (\\Exception $e) { }\n",
        "        $sql = 'SELECT * FROM t WHERE id = 1';\n",
        "        $h = <<<EOT\n",
        "          Hello $name!\n",
        "          EOT;\n",
        "        $n = <<<'EOT'\n",
        "        raw $x\n",
        "        EOT;\n",
        "        // a comment\n",
        "        # another\n",
        "        /* block */\n",
        "        $b = (int) $x + (string)$y;\n",
        "        $f = function ($x) use (&$y): int { return 1; };\n",
        "    }\n",
        "}\n",
        "interface I extends J {}\n",
        "trait T {}\n",
        "label:\n",
        "goto label;\n",
        "?>\n",
        "<div class=\"a\"><?= $x ?></div>\n",
        "<?php if ($y): ?>\n",
        "  <p>yes</p>\n",
        "<?php endif; ?>\n",
    );
    readings("php", Lang::Php, text);
}

/// Ruby's readings (X-100, a scanner of ada's own): classes, modules, methods, blocks, symbols,
/// strings with interpolation, percent literals, here-documents (a SQL body), numbers,
/// `begin`/`rescue`, `case`, `__END__`, regex literals in Ruby's dialect (X-114), and `/`, `<<`
/// and `?` by Ruby's lexer state, a name assigned before a local variable (X-118). Ruby's
/// here-document readings are pinned in ada-parse's `ruby` tests.
#[test]
fn ruby() {
    let text = concat!(
        "#!/usr/bin/env ruby\n",
        "# frozen_string_literal: true\n",
        "require \"json\"\n",
        "require_relative 'lib/thing'\n",
        "\n",
        "module Shop::Billing\n",
        "  class Invoice < ApplicationRecord::Base\n",
        "    include Comparable\n",
        "    attr_accessor :total, :lines\n",
        "    TAX_RATE = 0.25\n",
        "    DEFAULTS, LIMITS = {}, []\n",
        "\n",
        "    def initialize(lines = [], total: 0, **opts, &blk)\n",
        "      @lines = lines\n",
        "      @@count ||= 0\n",
        "      $stdout.sync = true\n",
        "    end\n",
        "\n",
        "    def self.parse(text) = new(JSON.parse(text))\n",
        "\n",
        "    def <=>(other)\n",
        "      total <=> other.total\n",
        "    end\n",
        "\n",
        "    def to_s\n",
        "      \"Invoice #{id}: #{format('%.2f', total)} (#@lines)\\n\"\n",
        "    end\n",
        "\n",
        "    def each_line\n",
        "      lines.each do |line, i = 0|\n",
        "        next if line.nil?\n",
        "        yield line unless line.empty?\n",
        "      end\n",
        "      lines.map { |l| l.amount * 2 }.sum\n",
        "    end\n",
        "\n",
        "    def matches?(pattern)\n",
        "      name =~ /inv-/i ? $1.to_i : %r{a/b}x.match?(name)\n",
        "    end\n",
        "\n",
        "    private\n",
        "\n",
        "    def sql\n",
        "      <<~SQL\n",
        "        SELECT id, total FROM invoices\n",
        "        WHERE id = #{id} -- one\n",
        "      SQL\n",
        "    end\n",
        "\n",
        "    def note\n",
        "      <<-EOS.strip\n",
        "        A note: #{@lines.size} lines \\t done\n",
        "      EOS\n",
        "    end\n",
        "  end\n",
        "end\n",
        "\n",
        "x = [1, 0x1F, 0b101, 017, 1_000, 1.5, 1e-3, 2r, 3i]\n",
        "y = %w[a b c] + %i(d e) + %q{f {g}} + [%Q<h #{x}>]\n",
        "z = a ? b : c\n",
        "w = :sym, :\"quoted #{x}\", ?a\n",
        "begin\n",
        "  raise ArgumentError, \"bad\" if x.empty? && !y\n",
        "rescue => e\n",
        "  warn e.message\n",
        "ensure\n",
        "  puts \"done\"\n",
        "end\n",
        "case x\n",
        "when Integer then 1\n",
        "else 2\n",
        "end\n",
        "buf=[]; n = 4\n",
        "buf <<x if x!=1\n",
        "puts <<~TXT, n /2, buf.size / 2\n",
        "  #{buf}\n",
        "TXT\n",
        "expect(out).to match /done$/\n",
        "PATTERN = /\n",
        "  \\A\\d+\n",
        "/x\n",
        "t = c ?abc : ?d\n",
        "f(?\\C-a, 1<<2, module?(x))\n",
        "__END__\n",
        "data\n",
    );
    readings("ruby", Lang::Ruby, text);
}

/// Swift's readings (X-101, a scanner of ada's own): declarations, attributes, functions and their
/// parameters, generics, closures, strings with interpolation, operators, comments.
#[test]
fn swift() {
    let text = concat!(
        "#!/usr/bin/env swift\n",
        "import Foundation\n",
        "@testable import Kit\n",
        "import struct Foundation.Date\n",
        "/// A thing.\n",
        "/** Docs. */\n",
        "@objc public final class Foo: NSObject, Bar {\n",
        "    private(set) var x: Int = 5\n",
        "    weak var delegate: Delegate?\n",
        "    lazy var names: [String: Any] = [:]\n",
        "    let s = \"a\\(b.c + 1)d\\n\\t\\\"\"\n",
        "    @available(iOS 13, *) func run<T: Equatable>(_ a: T, b: Array<Int>) throws -> Bool {\n",
        "        return a == a && !c || x != 1\n",
        "    }\n",
        "    static func + (lhs: Foo, rhs: Foo) -> Foo { lhs }\n",
        "    func compute() -> Dictionary<String, Int> { [:] }\n",
        "}\n",
        "let y = x-1 + -2 * 0x1F + 0b101 + 0o17 + 1e5 + 1_000 % 3\n",
        "z?.bar(a ?? b)!.baz\n",
        "guard let q = try? f() else { fatalError(\"no\") }\n",
        "arr.map { $0 + 1 }.filter { $0 > 2 }\n",
        "switch v { case .a: break; default: () }\n",
        "let t = x ? 1 : 2\n",
        "for i in 0..<10 where i > 2 { continue }\n",
        "while true { repeat { } while false }\n",
        "do { try g() } catch { throw e }\n",
        "if let v = w as? Int, w is String { defer { } }\n",
        "let n: Int8 = nil, u: UInt64 = 0, c: Character = \"c\"\n",
        "x += 1; x -= 1; x &+= 1; y = x << 2 >> 1 & 3 | 4 ^ 5\n",
        "let r = a ... b, h = a..<b\n",
        "var v: Int { get { 1 } set { } }\n",
        "extension Foo: P where T: Q { }\n",
        "enum E: String { case a = \"x\", b }\n",
        "typealias F = (Int) -> Void\n",
        "subscript(i: Int) -> Int { 0 }\n",
        "init?(raw: String) { self.init() ; super.foo() }\n",
        "#if DEBUG\n",
        "let d = __FILE__ + __LINE__\n",
        "#endif\n",
        "/* block */ let w = 2 // trailing\n",
    );
    readings("swift", Lang::Swift, text);
}

/// CMake's readings (X-104, a scanner of ada's own): commands, arguments quoted and unquoted,
/// variables and generator expressions, bracket arguments and comments, builtins' keywords, a
/// `MATCHES [[…]]` regex as CMake reads it (X-106).
#[test]
fn cmake() {
    let text = concat!(
        "cmake_minimum_required(VERSION 3.16)\n",
        "project(foo LANGUAGES CXX)\n",
        "#[==[ a bracket\n",
        "comment ]==] after\n",
        "set(FOO bar CACHE STRING \"doc\" FORCE)\n",
        "set(X 1 CACHE PATHS \"d\")\n",
        "set(${X}_y 1 PARENT_SCOPE)\n",
        "SET(\n",
        "  Y 2)\n",
        "if(NOT DEFINED FOO AND (A OR B) OR C VERSION_LESS_EQUAL 2)\n",
        "  message(STATUS \"x ${FOO} $ENV{HOME} $<CONFIG:Debug> \\n \\q ; a\\\n",
        "b\")\n",
        // a regex on a later line than its `MATCHES` departs from bat (X-108): ada-parse's tests
        "elseif(X MATCHES [[^a(b|[0-9]+.*)\\.c$]] OR Y\n",
        "  [=[raw]=])\n",
        "  break()\n",
        "else()\n",
        "endif()\n",
        "endif()\n",
        "else junk\n",
        "function(my_fn a b)\n",
        "  continue()\n",
        "  break()\n",
        "endfunction()\n",
        "macro(m)\n",
        "endmacro()\n",
        "foreach(x IN LISTS y ITEMS z)\n",
        "  if(x)\n",
        "    continue()\n",
        "  endif()\n",
        "endforeach()\n",
        "while(A)\n",
        "endwhile()\n",
        "my_cmd(FOO BAR_1 foo FOO=1 FOO-x (a b) \"s\" [[raw]] PROPERTIES FOO/${X}#c\n",
        "BAR)\n",
        "foo(a\\;b\\q c 'x y' -D${A}=\"$<$<CXX_COMPILER_ID:GNU>:-Wall ${W}>\")\n",
        "target_link_libraries(foo PRIVATE bar (x) y)\n",
        "add_executable(foo WIN32 src/a.c)\n",
        "add_custom_command(OUTPUT x USES_TERMINAL COMMAND_EXPAND_LISTS)\n",
        "install(TARGETS x INCLUDES DESTINATION y RUNTIME DESTINATION bin) # trailing\n",
        "include(foo OPTIONAL RESULT_VARIABLE res)\n",
        "if # c\n",
        "(x)\n",
        "endif()\n",
        "message (STATUS x) junk\n",
        "target_compile_options(x PRIVATE $<$<A:b>:c # d\n",
        ">)\n",
        "Set_Property(X ${a_${b}} ${A # e\n",
        "})\n",
        // X-119: a command as CMake parses one, `@ONLY` and `debug` keywords, a template's
        // placeholder; X-110: a generic command's bracket argument
        "configure_file(a.in a @ONLY)\n",
        "target_link_libraries(t debug a optimized b)\n",
        "add_custom_target(t COMMAND $(MAKE) -C (x))\n",
        "@PACKAGE_INIT@\n",
        "check_c_source_compiles([[int main(void) { return \"\"; }]] HAVE_X)\n",
        "endif() endif()\n",
    );
    readings("cmake", Lang::CMake, text);
}

/// CMake's header templates (X-140): C's and C++'s scanner, `#cmakedefine` a `#define`, `@VAR@`
/// and `${VAR}` placeholders in code, strings, comments and directives.
#[test]
fn cmake_headers() {
    let text = concat!(
        "/* config.h.in: @PROJECT_NAME@'s, see ${DOC_URL} or me@example.com */\n",
        "#ifndef CONFIG_H\n",
        "#define CONFIG_H\n",
        "#cmakedefine HAVE_UNISTD_H 1\n",
        "#cmakedefine01 ENABLE_X\n",
        "# cmakedefine HAVE_${NAME} @VALUE@\n",
        "#define VERSION \"@PROJECT_VERSION@\"\n",
        "#define PREFIX \"${CMAKE_INSTALL_PREFIX}\"\n",
        "#include \"@GEN_DIR@/gen.h\"\n",
        "static const int k = @K@ + ${J};\n",
        "int z_@SUFFIX@(void);\n",
        "#endif\n",
    );
    readings("cmake_headers", Lang::CMakeCHeader, text);
    readings("cmake_headers_cpp", Lang::CMakeCppHeader, text);
}

/// Terraform's readings (X-102, a scanner of ada's own): blocks and their labels, attributes, objects' keys
/// (`=`, `:`, a traversal), for-expressions, calls and builtins, templates with their directives and strip
/// markers, HCL's escapes, a string ended with its line, here-documents, comments.
#[test]
fn terraform() {
    let text = concat!(
        "# Terraform: blocks, attributes, objects\n",
        "terraform {\n",
        "  required_providers {\n",
        "    aws = { source = \"hashicorp/aws\", version = \"~> 5.0\" }\n",
        "    null = { source = \"hashicorp/null\" }\n",
        "  }\n",
        "}\n",
        "resource \"aws_instance\" \"web\" {\n",
        "  ami           = \"ami-${var.ami_id}\"\n",
        "  count         = var.enabled ? 1 : 0\n",
        "  nullable=false\n",
        "  tags = merge(local.tags, {\n",
        "    Name      = \"web-${count.index}\"\n",
        "    \"k-1\"     = 2\n",
        "    eventName : [\"INSERT\"]\n",
        "  })\n",
        "  dynamic \"ingress\" {\n",
        "    for_each = var.rules\n",
        "    content {\n",
        "      from_port = ingress.value.port\n",
        "    }\n",
        "  }\n",
        "  lifecycle { ignore_changes = [tags] }\n",
        "}\n",
        "module \"vpc.a/b\" {\n",
        "  source    = \"./modules/vpc\"\n",
        "  providers = { aws.west = aws.west }\n",
        "}\n",
        "moved {\n",
        "  from = aws_instance.old\n",
        "  to   = aws_instance.web\n",
        "}\n",
        "path \"secret/data/*\" { capabilities = [\"read\"] }\n",
        "block_type bare-label \"q\" {}\n",
        "locals {\n",
        "  ids   = [for s in var.subnets : s.id if s.public]\n",
        "  by_az = { for k, v in local.m : k => v... }\n",
        "  first = try(aws_instance.web[*].id, a.0, (b), my_fn(c), -1.5e3, d.*.e)\n",
        "  _hidden = can(regex(\"^a\", var.x)) && !false || null == true\n",
        "  t     = list(string)\n",
        "  esc   = \"a\\\"b\\n \\\\ \\u00e9 \\U0001F600 $${no} %%{no} \\.\"\n",
        "  tpl   = \"%{ if var.x ~}yes%{ else }no%{ endif ~} ${ {a = 1}.a }\"\n",
        "  open  = \"runs to the line's end\n",
        "  multi = \"${\n",
        "    var.x\n",
        "  } after\"\n",
        "  doc   = <<-EOT\n",
        "    hello ${var.name} $${literal}\n",
        "    %{ for x in var.l ~}${x}%{ endfor }\n",
        "    EOT\n",
        "  json = jsonencode(<<EOF\n",
        "{\"a\": 1}\n",
        "EOF\n",
        "  )\n",
        "}\n",
        "// a line comment\n",
        "x = 1 /* inline */ + 2\n",
        "/* a block\n",
        "   comment */ y = 3\n",
    );
    readings("terraform", Lang::Terraform, text);
}

/// Nix's readings (X-103, a scanner of ada's own): a function's formals (on one line and across lines, `...`, `@`),
/// `let` and `in`, bindings' attribute paths (a name's, a string's and an interpolation's parts), `inherit` and its
/// source, `with` and `assert`, lambdas, members (a builtin under `builtins` only), the global builtins, strings of
/// both kinds with their escapes and interpolations, `$${` as text, paths (relative, absolute, home, search paths,
/// through an interpolation), URIs, floats, operators, comments and their remarks. Shell in a phase's `''…''` strings
/// (X-147: interpolations Nix's, Nix's escapes, a here-document's closer under the string's indentation, a string
/// spliced from `lib.optionalString`, strings joined by `+` one script, a writer's script but not a string in its
/// arguments' set) and regexes in `builtins.match` and `lib.split` (X-148: POSIX's ERE, `\\` the regex's `\`).
#[test]
fn nix() {
    let text = concat!(
        "# Nix: a package, its formals and bindings\n",
        "{ lib, stdenv, fetchurl ? null, ... }@args:\n",
        "{ pkgs\n",
        ", enableFoo ? false\n",
        ", ...\n",
        "}:\n",
        "let\n",
        "  cfg = config.services.foo;\n",
        "  inherit (pkgs) hello;\n",
        "  inherit lib map;\n",
        "  f = { x, y ? { }, ... }: x * y;\n",
        "  g = x: y: x + y;\n",
        "in\n",
        "assert enableFoo -> stdenv.isLinux;\n",
        "rec {\n",
        "  pname = \"foo\";\n",
        "  version = \"1.0\";\n",
        "  src = fetchurl { url = \"https://x.org/${pname}-${version}.tar.gz\"; hash = \"sha256-abc\"; };\n",
        "  meta = with lib; { license = licenses.mit; platforms = platforms.linux; };\n",
        "  \"quoted.key\".a = 1;\n",
        "  ${dyn}.b = 2;\n",
        "  a.\"b\".${c}.d = 3;\n",
        "  passthru.tests = { inherit (pkgs) hello; };\n",
        "  buildInputs = [ pkgs.openssl ] ++ lib.optional stdenv.isLinux pkgs.systemd;\n",
        "  x = if a > 1 then \"yes\" else \"no\";\n",
        "  y = a.b or \"default\";\n",
        "  z = !true && false || null == (attrs ? key) // { };\n",
        "  n = [ 42 1.5 1.5e3 .5 (-1) ];\n",
        "  w = builtins.readFile ./x.txt + toString (import ./y.nix { }) + lib.map f [ ];\n",
        "  p = [ ./foo.nix ../bar/baz /etc/nixos ~/x <nixpkgs> <nixpkgs/lib> a/b ./foo/${name}.nix ];\n",
        "  u = [ https://example.org/a?b=c git+https://x.org/y ];\n",
        "  e = \"a \\n \\\" \\${no} $${no} $$${yes} \\\\\";\n",
        "  s = ''\n",
        "    text ${x} ''${no} ''' ''\\n $${no}\n",
        "    ${ \"in ${y}\" } ${placeholder \"out\"}/bin\n",
        "  '';\n",
        "  m = \"multi\n",
        "    line\";\n",
        "  postPatch = ''\n",
        "    substituteInPlace Makefile --replace-fail gcc \"${stdenv.cc}/bin/cc\"\n",
        "    for f in $out/bin/*; do wrapProgram \"$f\" --prefix PATH : ${lib.makeBinPath [ jq ]}; done\n",
        "    cat > $out/etc/x.conf <<EOF\n",
        "    home=''${HOME} # ${x}\n",
        "    EOF\n",
        "    ${lib.optionalString stdenv.isDarwin ''\n",
        "      install_name_tool -id $out/lib/libx.dylib $out/lib/libx.dylib\n",
        "    ''}\n",
        "  '' + ''\n",
        "    rm -r $out/share/doc\n",
        "  '';\n",
        "  t = runCommand \"t\" { f = writeText \"f\" ''not shell''; } ''ln -s ${f} $out'';\n",
        "  r = builtins.match \"([a-z]+)-([0-9.]+)\\\\.tar\\\\.gz$\" name;\n",
        "  q = lib.split \"[[:space:]]+${sep}(,|;)\" str;\n",
        "}\n",
        "# TODO: remarks as words, not in DEBUG\n",
        "/* FIXME a block\n",
        "   comment BUG !!! */\n",
    );
    readings("nix", Lang::Nix, text);
}

/// `.env`'s readings (ADR-40): comments, `export`, keys, values quoted and bare, interpolation,
/// escapes, numbers and constants.
#[test]
fn dotenv() {
    let text = concat!(
        "# comment\n",
        "export DATABASE_URL=\"postgres://${USER}:$PASS@host/db\\n\" # c\n",
        "DEBUG = true\n",
        "N=0x1F\n",
        "F=1.5e+3\n",
        "G=.5\n",
        "H=1. 2.x\n",
        "S='single $NOT'\n",
        "M=\"multi\n",
        "line {x} $$y\"\n",
        "EXPORTER=null\n",
        "a=b=c\n",
    );
    readings("dotenv", Lang::DotEnv, text);
}

/// Git's files' readings (ADR-40): ignore patterns, attributes, mailmap entries, a link file's
/// `gitdir:`.
#[test]
fn git() {
    let ignore = concat!(
        "# build output\n",
        "/target/\n",
        "!keep.me\n",
        "*.log ; not a comment\n",
        "**/node_modules\n",
        "docs/[!a-c]*.md\n",
        "a[]b]c\n",
        "a[x\n",
        "\\#literal \\ \n",
        "$HOME/../.cache/./x\n",
        "C:/drive ~/home\n",
        "   \n",
    );
    readings("git_ignore", Lang::GitIgnore, ignore);
    let attributes = concat!(
        "# attributes\n",
        "* text=auto eol=lf\n",
        "*.png binary -diff\n",
        "*.rs diff=rust merge=union filter=lfs encoding=UTF-16 whitespace=tab-in-indent,trailing-space\n",
        "\"quoted path\" -text !eol custom=a,b, x=\"q\\\"v\"\n",
        "[attr]binary -diff -merge -text\n",
        "-bad op\n",
        "path/[ab]*.c foo= bar\n",
    );
    readings("git_attributes", Lang::GitAttributes, attributes);
    let mailmap = concat!(
        "# mailmap\n",
        "Proper Name <proper@email.xx>\n",
        "<proper@email.xx> <commit@email.xx>\n",
        "Proper Name <proper@email.xx> Commit Name <commit@email.xx>\n",
        "Name Only\n",
        "<bad email> x\n",
    );
    readings("git_mailmap", Lang::GitMailmap, mailmap);
    let link = concat!(
        "gitdir: ../.git/modules/sub\n",
        "  gitdir :C:\\repo\\.git.\n",
        "gitdir: ~/x/*\n",
        "notgitdir: x\n",
        "# comment\n",
        "gitdir\n",
    );
    readings("git_link", Lang::GitLink, link);
}

/// Makefile's readings (X-097, a scanner of ada's own): rules, targets and prerequisites, variables
/// and their assignments, functions, conditionals, `define`, directives; recipes on bash's scanner
/// (X-094), a make token a word in the shell's text and `$$` its `$`, as make hands the shell its
/// line.
#[test]
fn make() {
    let text = concat!(
        "# build \\\n",
        "  continued comment\n",
        "include config.mk\n",
        "-include $(wildcard *.d)\n",
        "vpath %.c src\n",
        "CC ?= gcc\n",
        "CFLAGS := -O2 -Wall # flags\n",
        "OBJS = $(patsubst %.c,%.o,$(SRCS)) ${EXTRA:.c=.o}\n",
        "override LDFLAGS += -lm\n",
        "export PATH := $(HOME)/bin:$(PATH)\n",
        "REV != git rev-parse --short HEAD\n",
        "RUN = sh -c 'echo \"$$1\" && exit 0'\n",
        "define HELP =\n",
        "  usage: make [target]\n",
        "endef\n",
        "ifeq ($(CC),gcc)\n",
        "  WARN = -Wextra\n",
        "else ifneq \"$(CC)\" 'clang'\n",
        "endif\n",
        "all: $(OBJS) | dirs ; @echo inline\n",
        "%.o: %.c config.h\n",
        "\t@gcc $(CFLAGS) -c $< -o $@\n",
        "\t-rm -f \"$@.tmp\" 2>/dev/null || true\n",
        "\tfor f in $^; do [ -e \"$<\" ] && echo `basename $@`; done\n",
        "\tf() { printf '%s' $<; }; f x\n",
        "test: CFLAGS += -g\n",
        "test:: all\n",
        "\t$(info $(call show,$(OBJS),\"quoted\"))\n",
        "\t$(Q)$(kecho) \"  SYNC    $@\"\n",
        "\tVAR=1 cd build && make -C sub $(shell uname -s)\n",
        "\tif [ -n \"$${HOME}\" ]; then echo ok; else exit 1; fi\n",
        "\tcase $< in a|b) echo $$((1 + 2));; esac\n",
        "endef\n",
        "a((m)    ((g)    (i)    : b\n",
    );
    readings("make", Lang::Makefile, text);
}

/// bash's readings (X-093's scanner): commands (a command's name as the builtins, apart from its
/// arguments, as zsh-syntax-highlighting colors it), options, strings, parameters and expansions,
/// arithmetic, tests, here-documents, `case`, functions, redirections, comments.
#[test]
fn bash() {
    let text = concat!(
        "#!/usr/bin/env bash\n",
        "set -euo pipefail # strict\n",
        "readonly DIR=\"$(cd \"$(dirname \"${BASH_SOURCE[0]}\")\" && pwd)\"\n",
        "declare -A seen=([a]=1 [b]=2)\n",
        "export PATH=\"$HOME/bin:$PATH\" LANG=C\n",
        "usage() {\n",
        "  printf 'usage: %s [-v] file...\\n' \"${0##*/}\" >&2\n",
        "  exit \"${1:-1}\"\n",
        "}\n",
        "function log { echo \"[$(date +%T)] $*\"; }\n",
        "while getopts 'vh' opt; do\n",
        "  case $opt in\n",
        "    v) verbose=1 ;;\n",
        "    h|*) usage 0 ;;\n",
        "  esac\n",
        "done\n",
        "shift $((OPTIND - 1))\n",
        "for f in \"$@\" *.txt; do\n",
        "  [[ -f $f && $f != *.bak ]] || continue\n",
        "  if (( ${#f} > 3 )); then echo \"${f%.txt}\" | tr a-z A-Z; fi\n",
        "done 2>/dev/null\n",
        "cat <<-EOF > \"$DIR/out\"\n",
        "\tname: ${USER}\n",
        "\tEOF\n",
        // the prototype's comment on the opening line keeps the here-document's string
        "cat <<'EOF'\t# literal\n",
        "$x\n",
        "EOF\n",
        "coproc worker { while read -r l; do echo \"$l\"; done; }\n",
        "x=`basename $PWD` y=$'tab\\there' z=~/src\n",
        "arr+=( \"${arr[@]/#/-I}\" ) && echo ${arr[*]:1:2} {a,b}{1..3}\n",
        "trap 'rm -f \"$tmp\"' EXIT; wait $!; exit $?\n",
    );
    readings("bash", Lang::Bash, text);
}

const GIT: [(&str, Lang); 4] = [
    ("git_ignore", Lang::GitIgnore),
    ("git_attributes", Lang::GitAttributes),
    ("git_mailmap", Lang::GitMailmap),
    ("git_link", Lang::GitLink),
];

/// XML's corners, the blocks its scanner was built against (from `tests/exact_lexers.rs`, X-116):
/// declarations, a doctype's internal subset and conditional sections, comments, CDATA, bad names
/// and attributes, references, and parens and sections nested deep.
#[test]
fn xml_examples() {
    let blocks = [
        "<?xml version=\"1.0\" encoding='UTF-8'?>\n<root a=\"1\">text &amp; more</root>\n",
        "<?xml-stylesheet href=\"a.xsl\"?>\n<?php echo 1; ?>\n<?XmL x?>\n<?123?>\n",
        "<!DOCTYPE note SYSTEM \"note.dtd\" [\n  <!ELEMENT note (to,from?,(a|b)*)>\n  <!ELEMENT br EMPTY>\n  <!ATTLIST note id ID #REQUIRED kind (x|y) \"x\" ref IDREFS #IMPLIED>\n  <!ENTITY % param \"value\">\n  <!ENTITY pic SYSTEM \"p.gif\" NDATA gif>\n  <!NOTATION gif PUBLIC \"image/gif\">\n  %param;\n  <!BOGUS x>\n]>\n",
        "<!doctype html>\n<!DOCTYPE>\n<!DOCTYPE a:b PUBLIC \"x\" 'y'>\n",
        "<ns:root xmlns:ns=\"urn:x\" a='1' b=bare c = \"&amp;&#65;&#x41;&bad\">\n",
        "  <!-- a comment -- with -- dashes --->\n  <![CDATA[ raw <text> ]]>\n",
        "  <item key=\"v\"/>text &lt; & more < x\n  <a:b:c d:e:f=\"1\" ?>\n  <bad$name 1x=\"2\" / >\n",
        "  <multi\n    attr=\"v\"\n  >\n</ns:root>\n<!-- open\ncomment -->\n",
        "<a b=\"x <c>\n<a b='open\nstill' c\n=\n'v'>\n</a >\n</ a>\n</a x>\n",
        "<a =\"x\" b/c=1 d=/>\n<a ?x=1 /y>\n<?xml a/b=1 >x?>\n<a b==\"c\">\n",
        "<!DOCTYPE x [\n<![ INCLUDE [\n<!ELEMENT a ((b|c)+, d)*>\n<![%p;[<!ATTLIST a b CDATA #FIXED \"1\">]]>\n]]>\n]>\n",
        "<!DOCTYPE x [ <!ENTITY a \"&#38;&amp;\"> <!-- c --> <?pi x?> <![CDATA[x]]> ]>\n",
        "<!DOCTYPE x [<!ELEMENT a (#PCDATA|b)* > <!ATTLIST a b (c|d|&e;|<) #IMPLIED>]>\n",
        "\u{e9}<\u{e9}l\u{e9} \u{e9}=\"\u{e9}\">&\u{e9};</\u{e9}l\u{e9}>\n",
    ];
    readings_of("xml_examples", Lang::Xml, &blocks);
    let deep = [
        format!("<!DOCTYPE x [<!ELEMENT a {}b", "(".repeat(200)),
        format!("<!DOCTYPE x [{}", "<![[".repeat(60)),
    ];
    readings_of("xml_deep", Lang::Xml, &deep);
}

/// `.env`'s corners (from `tests/exact_lexers.rs`, X-116): `export`, numbers and constants good and
/// bad, quotes across lines, interpolation and escapes, blank and empty blocks.
#[test]
fn dotenv_examples() {
    let blocks = [
        "# comment\nexport DATABASE_URL=\"postgres://${USER}:$PASS@host/db\\n\" # c\n",
        "DEBUG = true\nN=0x1F\nF=1.5e+3\nG=.5\nH=1. 2.x\nI=1.5x 0x1g 12e5 3E-2y a.5 1e\n",
        "S='single $NOT'\nM=\"multi\nline {x} $$y\"\nEXPORTER=null\na=b=c\n",
        "x =  NULL\ny= False\nz=TRUEx\n export\texport x\nmyexportexport=1 reexport\n",
        "A=\"${a $b ${c}} {$$d} \\q \\$ \\\"\"\nB=\"${unclosed\nstill}\" '\nC='a\nb' \"\n",
        "K=\"$\x7f\" ${é} $é1 $_x_ ${1}\"\n\u{e9}=\u{e9} caf\u{e9}=1 =1 ==2 a = b\n",
        "0x=1 0x 1.e5 .e5 1..2 x=1.2.3 x=-1 +2 x=1_000\n",
        "",
        "\n\n",
        "#\n\"\n'\n${\n",
        "  # lead\n \t export A=1\n\t\n   =true\n  ' x\n\t'\n  \"s\n",
    ];
    readings_of("dotenv_examples", Lang::DotEnv, &blocks);
}

/// Git's files' corners (from `tests/exact_lexers.rs`, X-116), every block read as each of the
/// four: patterns and classes, attributes and their values, mailmap names and addresses, `gitdir:`
/// lines.
#[test]
fn git_examples() {
    let blocks = [
        "# build output\n/target/\n!keep.me\n*.log ; not a comment\n**/node_modules\n",
        "docs/[!a-c]*.md\na[]b]c\na[x\n\\#literal \\ \n$HOME/../.cache/./x\nC:/drive ~/home\n   \n",
        "a[\\]b]c [^x] [*?] [ ] [é]\nx\\\ny\\$z\\*\\é \"q\" a:b ./x ../y .z/w.v\n[\n\\\n!\n!!x\n",
        "# attributes\n* text=auto eol=lf\n*.png binary -diff\n",
        "*.rs diff=rust merge=union filter=lfs encoding=UTF-16 whitespace=tab-in-indent,trailing-space\n",
        "\"quoted path\" -text !eol custom=a,b, x=\"q\\\"v\"\n[attr]binary -diff -merge -text\n-bad op\n",
        "path/[ab]*.c foo= bar\nx a=\"open\nx a=, b=,c ,d a=b,,c a==b a=b=c\n !x\n\"[a\" b\n\"a\nb\\\nc d\n",
        "x textual eolx diffx -diff- a-b- a--b _ é=é [ab] [-x] [!x] [a] b\n x\t\ty=\"\\t\\x\" z=\" \"\n",
        "# mailmap\nProper Name <proper@email.xx>\n<proper@email.xx> <commit@email.xx>\n",
        "Proper Name <proper@email.xx> Commit Name <commit@email.xx>\nName Only\n<bad email> x\n",
        "a <b@c.d.e> f ; g\n<<a>> <> <a <a@b@c.d> a\t <x>\nname ;comment <x>\n\r<a>\n",
        "gitdir: ../.git/modules/sub\n  gitdir :C:\\repo\\.git.\ngitdir: ~/x/*\nnotgitdir: x\n",
        "# comment\ngitdir\nx\ngitdir x : y\ngitdir:\ngitdir: a/..\\./b.c.\n gitdir: .\n",
        "a\"b c x\"y=\"z\" \"q\"r s\nab\"[c\" d\n",
        "",
        "\n\n",
    ];
    for (name, lang) in GIT {
        readings_of(&format!("{name}_examples"), lang, &blocks);
    }
}

/// Dockerfile's corners (from `tests/exact_lexers.rs`, X-116): parser directives, `FROM`'s forms,
/// continuations, a string open across lines, instructions bare, in lower and mixed case, `ONBUILD`
/// chains, near-instructions.
#[test]
fn dockerfile_examples() {
    let blocks = [
        "# syntax=docker/dockerfile:1\nARG BASE=rust\nFROM ${BASE}:1.95 AS build\n",
        "FROM rust:1.95 as build\nWORKDIR /src\nCOPY --from=deps /x /y\nRUN cargo build \\\n  --release && \\\n  echo \"done\\n\" 'x'\n# c\n  # c2\n",
        "FROM x\nRUN echo \"open\nnext\nlower\nUPPER\n",
        "FROM x\nonbuild run x\nONBUILD COPY a b\nCMD [\"a\", \"b\"]\nENTRYPOINT [\"x\"]\nMAINTAINER me\n",
        "FROM x\nRUN\nnext line\nCOPY\nx\n",
        "FROM x\nRUN a \\ \nb\n  RUN x\n\tlabel a=b\n",
        "FROM x@sha256:abc\nFROM scratch\nEXPOSE 80\nHEALTHCHECK CMD x\n",
        "from x\nFROM\nFROM \nARG\nRUN 'a\n\nb'\n",
        "FROM x\nONBUILD MAINTAINER x\nONBUILD ONBUILD RUN x\nRUN2 x\nCOPYX y\nonbuild\nONBUILD  \n",
        "FROM x\nONBUILDRUN x\ncopy --from=a b c\nHealthcheck NONE\nstopSignal x\n  Add a b\nOnBuild Cmd x\n",
        "FROM x\nShell [\"a\"]\nvolume /x\nUSER\nlabel\texpose 1\nENTRYPOINT\nMAINTAINERS x\nWORKDIR\tx\n",
        "",
        "\n\n",
    ];
    readings_of("dockerfile_examples", Lang::Dockerfile, &blocks);
}

/// fstab's corners (from `tests/exact_lexers.rs`, X-116): every field's forms, options with and
/// without values, comments after each field, short and long lines.
#[test]
fn fstab_examples() {
    let blocks = [
        "# <file system> <dir> <type> <options> <dump> <pass>\nUUID=ab12\t/\text4\trw,relatime\t0 1\n",
        "LABEL=x /boot vfat rw,fmask=0077,x=,=y,a=b=c 0 2 # c\n/dev/sda1 none swap sw 0 0\n",
        "//srv/share /mnt cifs user=a,pass=b 0 0\n\\\\srv\\x /m smb defaults\nserver:/export /nfs nfs4 _netdev 0 0\n",
        "  sda1  /x  ext4\nabc/def /m\n/a/b/ /c\n/ / / / 1 2\n/x/ y\n\\x/y z\n:a b\na: b\n=a b\nx=y\n",
        "tmpfs /tmp tmpfs defaults 10 20 extra\nproc /proc proc defaults 3 x y\na b c d,e,,f g h # c\n",
        "a #b\na b #c\na b c #d\na b c d #e\na b c d 0 #f\n  # c\n\t\n",
        "a b c d\ne f g\nh i\nj\n",
        "",
        "\n\n",
    ];
    readings_of("fstab_examples", Lang::Fstab, &blocks);
}

/// CMakeCache's corners (from `tests/exact_lexers.rs`, X-116): keys, types and values good and bad,
/// comments.
#[test]
fn cmake_cache_examples() {
    let blocks = [
        "# This is the CMakeCache file.\n//Path to a program.\nCMAKE_AR:FILEPATH=/usr/bin/ar\nK-2:STATIC=\n",
        "  K:BOOL=ON # x\nKEY\n:STRING=a\nK:T==x\nK:STRING;x\nK:A B.c=v\n",
        "x // c\n/x\nK=1\nK\n\n  \nK:\t=1\n# c\n",
        "\u{e9}:INTERNAL=1\nK:a:b/c\\d=\u{e9}\n\t\n",
        "",
        "\n\n",
    ];
    readings_of("cmake_cache_examples", Lang::CMakeCache, &blocks);
}

/// `known_hosts`' corners (from `tests/exact_lexers.rs`, X-116): markers, host patterns with ports,
/// hashed hosts, addresses, key types, keys and comments.
#[test]
fn known_hosts_examples() {
    let blocks = [
        "# c\n  # c\n@revoked [a,b*]:22,1.2.3.4 ssh-rsa AAAAB3Nza+/= my label \n",
        "|1|abcdefghijklmnopqrstuvwxyz0=|abcdefghijklmnopqrstuvwxyz0= ssh-dss AAAAC3x\n",
        "[foo\nbar]x\n[a\n,b\n]:123456,c\n",
        "github.com,140.82.121.4 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl\n",
        "[::1]:22,::ffff:1.2.3.4,fe80::7:8%eth0,1:2:3:4:5:6:7:8 ecdsa-sha2-nistp256 AAAAE2VjZHNh=\n",
        "[!1.2.3.4,::1]x !a*b?c ssh-rsa,ssh-dss x509v3-ssh-rs\"x AAAAB3N\n",
        "@cert-authority *.example.com ssh-rsa AAAAB3NzaC1yc2E== a\x0b\n@revokedX y\n  @revoked z\n",
        "a AAAAB3Nza==ssh-rsa b\n1.2.3.456 1.2.3.4a ::1abg x::1 AAAAB3Nz\x0b a  \n",
        "   \n\n[]\n[]:\n[]:x\n[1.2.3.4]:2\n[,]\n[x*,?]\n",
        "",
    ];
    readings_of("known_hosts_examples", Lang::KnownHosts, &blocks);
}

/// Java properties' corners (from `tests/exact_lexers.rs`, X-116): comments, escaped keys and
/// separators, continuations, empty keys and values.
#[test]
fn properties_examples() {
    let blocks = [
        "# c\n  ! c\na\\ b = x\\ty \\u00e9\\\n  more\nk:v\nk \\\n v\n",
        "key\nkey=\nkey =\nkey  value:x\n=v\n:v\nk\\u12x=y\nk\\\\=v\nk\\=x = y\\\\\n",
        "a = b\\\nc\\\nd\n x = \\x\n\t\tx\t:\ty\n",
        "",
        "\n\n",
    ];
    readings_of("properties_examples", Lang::Properties, &blocks);
}
