use super::*;

/// `text`'s lines lexed one at a time: each line's spans and the state after it, the spans
/// checked in order.
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

/// `text`'s lines lexed one at a time: each span's text and scopes.
fn lex(text: &str) -> Vec<(String, Vec<&'static str>)> {
    let mut out = Vec::new();
    for (l, (spans, _)) in text.lines().zip(lines(text)) {
        for s in spans {
            out.push((
                l[s.start as usize..s.end as usize].to_string(),
                scopes(s.kind),
            ));
        }
    }
    out
}

#[track_caller]
fn has(t: &[(String, Vec<&str>)], text: &str, scope: &str) {
    assert!(
        t.iter().any(|(x, s)| x == text && s.join(" ") == scope),
        "{text:?} as {scope:?} in {t:?}"
    );
}

/// Whether `text` is written with a scope containing `part` anywhere.
fn is(t: &[(String, Vec<&str>)], text: &str, part: &str) -> bool {
    t.iter()
        .any(|(x, s)| x == text && s.iter().any(|s| s.contains(part)))
}

#[track_caller]
fn plain(t: &[(String, Vec<&str>)], text: &str) {
    assert!(
        !t.iter().any(|(x, s)| x == text && !s.is_empty()),
        "{text:?} plain in {t:?}"
    );
}

#[test]
fn a_function_and_its_parameters() {
    let t = lex("def f(a, *b, c: int = 1, /, **d) -> None:\n    return a\n");
    has(&t, "f", "entity.name.function.python");
    has(&t, "a", "variable.parameter.python");
    has(&t, "b", "variable.parameter.python");
    has(&t, "d", "variable.parameter.python");
    has(&t, "int", "support.type.python");
    has(&t, "/", "storage.modifier.positional-args-only.python");
    has(&t, "->", "punctuation.separator.annotation.return.python");
    has(&t, ":", "punctuation.section.function.begin.python");
    has(&t, "return", "keyword.control.flow.return.python");
    // parameters across lines, a default's call and its keyword argument
    let t = lex("def g(\n    self,\n    x=h(y, z=1),\n    w: Dict[str, int] = {},\n):\n    pass\n");
    has(&t, "self", "variable.parameter.python");
    has(&t, "x", "variable.parameter.python");
    has(&t, "w", "variable.parameter.python");
    has(&t, "z", "variable.parameter.python");
    plain(&t, "y");
    has(&t, ":", "punctuation.section.function.begin.python");
    // a def's name that spells a builtin is the def's, a magic one keeps its class
    let t = lex("def filter(self): pass\ndef __init__(self): pass\n");
    has(&t, "filter", "entity.name.function.python");
    has(
        &t,
        "__init__",
        "entity.name.function.python support.function.magic.python",
    );
}

#[test]
fn calls_keyword_arguments_and_lambdas() {
    let t = lex("x = f(a, key=lambda t, y=1: t + y, *rest, **kw)\n");
    has(&t, "f", "variable.function.python");
    has(&t, "key", "variable.parameter.python");
    has(&t, "t", "variable.parameter.python");
    has(&t, "y", "variable.parameter.python");
    has(&t, "*", "keyword.operator.unpacking.sequence.python");
    has(&t, "**", "keyword.operator.unpacking.mapping.python");
    has(&t, ":", "punctuation.section.function.begin.python");
    plain(&t, "a");
    // a builtin's call, a type's, a member's (a builtin's spelling plain there), a constant
    let t = lex("print(len(x), int(y), s.len(), MAX_SIZE, FOO.bar, x.MAX, _)\n");
    has(
        &t,
        "print",
        "variable.function.python support.function.builtin.python",
    );
    has(&t, "int", "variable.function.python support.type.python");
    has(&t, "len", "variable.function.python");
    has(&t, "MAX_SIZE", "variable.other.constant.python");
    has(&t, "FOO", "variable.other.constant.python");
    has(&t, "MAX", "variable.other.constant.python");
    has(&t, "_", "variable.language.python");
    // `==` isn't a keyword argument's
    let t = lex("f(a == b)\n");
    plain(&t, "a");
}

#[test]
fn decorators_and_classes() {
    let t = lex(
        "@app.route(\"/x\", methods=[\"GET\"])\n@property\nclass A(Base, metaclass=M):\n    pass\n",
    );
    has(
        &t,
        "@",
        "meta.annotation.python punctuation.definition.annotation.python",
    );
    has(&t, "app", "meta.annotation.python");
    has(
        &t,
        "route",
        "meta.annotation.python variable.annotation.function.python",
    );
    has(
        &t,
        "methods",
        "meta.annotation.python variable.parameter.python",
    );
    has(
        &t,
        "property",
        "meta.annotation.python variable.annotation.python support.function.builtin.python",
    );
    has(&t, "A", "entity.name.class.python");
    has(
        &t,
        "metaclass",
        "variable.parameter.class-inheritance.python",
    );
    has(&t, ":", "punctuation.section.class.begin.python");
    plain(&t, "Base");
    // a decorator's arguments over lines stay its, and end with its `)`
    let t = lex("@pytest.mark.parametrize(\n    \"a\", [1]\n)\ndef f(): pass\n");
    has(&t, "    ", "meta.annotation.python");
    has(
        &t,
        ")",
        "meta.annotation.python punctuation.section.arguments.end.python",
    );
    has(
        &t,
        "def",
        "storage.type.function.python keyword.declaration.function.python",
    );
}

#[test]
fn statements_and_their_heads() {
    let t = lex(concat!(
        "for i, (j, k) in enumerate(z):\n",
        "    if x: continue\n",
        "    elif y: pass\n",
        "    else: break\n",
        "with open(p) as f, lock:\n",
        "try:\n",
        "    pass\n",
        "except* (A, B) as eg:\n",
        "    raise E from eg\n",
        "finally:\n",
        "    x = [a for a, b in c if a]\n",
        "global g, h\n",
        "x: int = 1\n",
        "while (a and\n",
        "       b):\n",
        "    yield from g()\n",
    ));
    has(&t, ",", "punctuation.separator.target-list.python");
    has(&t, "in", "keyword.control.loop.for.in.python");
    has(&t, ":", "punctuation.section.block.loop.for.python");
    has(&t, ":", "punctuation.section.block.conditional.if.python");
    has(
        &t,
        ":",
        "punctuation.section.block.conditional.elseif.python",
    );
    has(&t, ":", "punctuation.section.block.conditional.else.python");
    has(&t, "as", "keyword.control.flow.with.as.python");
    has(&t, ",", "punctuation.separator.with-resources.python");
    has(&t, "except*", "keyword.control.exception.catch.python");
    has(&t, "as", "keyword.control.exception.catch.as.python");
    has(&t, "from", "keyword.control.flow.raise.from.python");
    has(&t, "for", "keyword.control.loop.for.generator.python");
    has(&t, ",", "punctuation.separator.storage-list.python");
    has(&t, ":", "punctuation.separator.annotation.variable.python");
    has(&t, ":", "punctuation.section.block.loop.while.python");
    has(&t, "from", "keyword.control.flow.yield-from.python");
    let t = lex("from ..pkg.mod import (a as b,\n    c)\nimport os.path, json\nfrom x import *\n");
    has(&t, "..", "keyword.control.import.relative.python");
    has(&t, "as", "keyword.control.import.as.python");
    has(&t, ",", "punctuation.separator.import-list.python");
    has(&t, "*", "constant.language.import-all.python");
    plain(&t, "json");
}

#[test]
fn soft_keywords_and_pep_695() {
    let t = lex(concat!(
        "match command.split():\n",
        "    case [action, *rest] if rest:  # c\n",
        "        pass\n",
        "match = 1\n",
        "match(x)\n",
        "type Alias[T] = list[T]\n",
        "type = 3\n",
        "def f[T: (int, str), *Ts, **P](x: T) -> T: ...\n",
        "class C[T](Base[T]): pass\n",
    ));
    has(&t, "match", "keyword.control.conditional.match.python");
    has(&t, "case", "keyword.control.conditional.case.python");
    has(
        &t,
        ":",
        "punctuation.section.block.conditional.match.python",
    );
    has(&t, ":", "punctuation.section.block.conditional.case.python");
    has(&t, "match", "variable.function.python");
    has(
        &t,
        "type",
        "storage.type.type.python keyword.declaration.type.python",
    );
    has(&t, "Alias", "entity.name.type.python");
    has(&t, "type", "support.function.builtin.python");
    has(&t, "T", "variable.parameter.type.python");
    has(&t, "Ts", "variable.parameter.type.python");
    has(&t, "P", "variable.parameter.type.python");
    has(&t, "x", "variable.parameter.python");
    has(&t, "[", "punctuation.section.type-parameters.begin.python");
    // `list[T]` after the alias's `=` is a subscript
    has(&t, "[", "punctuation.section.brackets.begin.python");
    // Python 2's statements, and `print` a builtin elsewhere
    let t = lex("print >>f, x\nprint x\nx = print\nprint(x)\n");
    has(&t, "print", "keyword.other.print.python");
    has(&t, "print", "support.function.builtin.python");
    has(
        &t,
        "print",
        "variable.function.python support.function.builtin.python",
    );
}

#[test]
fn f_strings_as_pep_701_reads_them() {
    let t = lex(r#"x = f"a {b!r:>{w}} {c=} {d["k"]} {{lit}} {e:%Y-%m-%d} {'\n'.join(l)}""#);
    has(&t, "f", "storage.type.string.python");
    has(&t, "{", "punctuation.section.interpolation.begin.python");
    has(&t, "!r", "storage.modifier.conversion.python");
    has(&t, ":>", "constant.other.format-spec.python");
    has(&t, "=", "storage.modifier.debug.python");
    has(&t, "k", "string.quoted.double.python");
    has(
        &t,
        "{{",
        "string.quoted.double.python constant.character.escape.python",
    );
    has(&t, ":%Y-%m-%d", "constant.other.format-spec.python");
    has(
        &t,
        r"\n",
        "string.quoted.single.python constant.character.escape.python",
    );
    has(&t, "join", "variable.function.python");
    // a field over lines in a triple-quoted f-string, a comment in it; then the string goes on
    let t = lex("x = f\"\"\"a {\n    b  # c\n} d\"\"\"\ny = 1\n");
    has(&t, " c", "comment.line.number-sign.python");
    has(&t, " d", "string.quoted.double.block.python");
    has(
        &t,
        "\"\"\"",
        "string.quoted.double.block.python punctuation.definition.string.end.python",
    );
    has(&t, "1", "constant.numeric.integer.decimal.python");
    // a t-string
    let t = lex("x = t\"{a}\"\n");
    has(&t, "t", "storage.type.string.python");
    has(&t, "{", "punctuation.section.interpolation.begin.python");
}

#[test]
fn docstrings_and_strings() {
    let t = lex(concat!(
        "def f():\n",
        "    'a docstring'\n",
        "    \"\"\"Another, \\N{BULLET}\n",
        "    over lines.\"\"\"\n",
        "    x = (\n",
        "        \"\"\"not one\"\"\"\n",
        "    )\n",
        "    y = 1 + \\\n",
        "        \"\"\"nor this\"\"\"\n",
        "    s = 'it\\'s' \"line \\\n",
        "continued\"\n",
    ));
    has(&t, "a docstring", "comment.block.documentation.python");
    has(
        &t,
        r"\N{BULLET}",
        "comment.block.documentation.python constant.character.escape.unicode.name.python",
    );
    has(&t, "    over lines.", "comment.block.documentation.python");
    has(&t, "not one", "string.quoted.double.block.python");
    has(&t, "nor this", "string.quoted.double.block.python");
    has(
        &t,
        r"\'",
        "string.quoted.single.python constant.character.escape.python",
    );
    has(
        &t,
        "\\",
        "string.quoted.double.python punctuation.separator.continuation.line.python",
    );
    has(&t, "continued", "string.quoted.double.python");
    // escapes: hex, octal, 16 and 32 bits, a bad one; bytes have no `\u`
    let t = lex(r#"s = "\x41\101\u00e9\U0001F600\q" + b"\u00e9""#);
    has(
        &t,
        r"\x41",
        "string.quoted.double.python constant.character.escape.hex.python",
    );
    has(
        &t,
        r"\101",
        "string.quoted.double.python constant.character.escape.octal.python",
    );
    has(
        &t,
        r"\u00e9",
        "string.quoted.double.python constant.character.escape.unicode.16-bit-hex.python",
    );
    has(
        &t,
        r"\U0001F600",
        "string.quoted.double.python constant.character.escape.unicode.32-bit-hex.python",
    );
    has(
        &t,
        r"\q",
        "string.quoted.double.python invalid.deprecated.character.escape.python",
    );
    has(
        &t,
        r"\u",
        "string.quoted.double.python invalid.deprecated.character.escape.python",
    );
    // `\N{` without a name's `}` before the string's end is no escape (the fuzzer's: its search ran past the
    // quote, in a SQL string)
    let t = lex(r#"cur.execute("ALTER TABLE t ADD \N{a::b" + f(x)) + "{41}""#);
    has(
        &t,
        r"\N",
        "string.quoted.double.python invalid.deprecated.character.escape.python",
    );
}

#[test]
fn placeholders() {
    let t = lex(r#"s = "%(name)-10s %5.2f %Y {} {0:>{w}} {a.b[0]!r:^10} {user:([0-9]+)}" % d"#);
    has(
        &t,
        "%(",
        "string.quoted.double.python constant.other.placeholder.python",
    );
    has(
        &t,
        "name",
        "string.quoted.double.python constant.other.placeholder.python variable.other.placeholder.python",
    );
    has(
        &t,
        "%5.2f",
        "string.quoted.double.python constant.other.placeholder.python",
    );
    has(
        &t,
        "%Y",
        "string.quoted.double.python constant.other.placeholder.python",
    );
    has(
        &t,
        "{",
        "string.quoted.double.python constant.other.placeholder.python punctuation.definition.placeholder.begin.python",
    );
    has(
        &t,
        "a.b[0]",
        "string.quoted.double.python constant.other.placeholder.python",
    );
    has(
        &t,
        "!r",
        "string.quoted.double.python constant.other.placeholder.python storage.modifier.conversion.python",
    );
    // aiohttp's route `{name:regex}` is no format spec
    assert!(!is(&t, "user", "placeholder"), "{t:?}");
}

#[test]
fn sql_by_its_keyword_and_its_call() {
    let dq = "string.quoted.double.python";
    let t = lex(r#"q = "SELECT a FROM t WHERE id = %s""#);
    has(&t, "SELECT", &format!("{dq} keyword.other.DML.sql"));
    has(&t, "%s", &format!("{dq} constant.other.placeholder.python"));
    // a lone keyword is no SQL (an HTTP method)
    let t = lex(r#"methods = ("DELETE", "GET")"#);
    has(&t, "DELETE", dq);
    // an execute-like call's first argument, any case; a triple-quoted one decided by its first text
    let t = lex(
        "cur.execute(\"select * from t\")\ncur.execute(\"\"\"\n    insert into t values (1)\n\"\"\")\n",
    );
    has(&t, "select", &format!("{dq} keyword.other.DML.sql"));
    has(
        &t,
        "insert into",
        "string.quoted.double.block.python keyword.other.DML.sql",
    );
    // a message isn't SQL at a call that isn't one
    let t = lex("log.info(\"select a value\")\n");
    has(&t, "select a value", dq);
    // SQL in an f-string: a field between, and SQL's string context kept across it
    let t = lex(r#"q = f"SELECT a FROM t WHERE b = '{v}' AND c = 1""#);
    has(&t, "{", "punctuation.section.interpolation.begin.python");
    has(&t, "AND", &format!("{dq} keyword.operator.logical.sql"));
}

#[test]
fn regexes_where_python_takes_one() {
    // a regex call's pattern, raw or cooked
    let t = lex(r#"p = re.compile(r"(?P<n>\w+)\s*(?P=n)") + re.match("\\d+", s)"#);
    has(
        &t,
        "n",
        "string.quoted.double.python entity.name.other.group.regexp",
    );
    has(
        &t,
        r"\w",
        "string.quoted.double.python constant.character.character-class.regexp",
    );
    has(
        &t,
        "+",
        "string.quoted.double.python keyword.operator.quantifier.regexp",
    );
    assert!(is(&t, r"\\d", "character-class"), "{t:?}");
    // `match=`
    let t = lex(r#"with pytest.raises(E, match=r"bad \d+ value"):"#);
    assert!(is(&t, r"\d", "character-class"), "{t:?}");
    // a raw string elsewhere: a regex where `re` would take it
    let t = lex(r#"P = r"^(\w+)@(\w+)\.com$""#);
    has(
        &t,
        "^",
        "string.quoted.double.python keyword.control.anchor.regexp",
    );
    // a Windows path, LaTeX `re` rejects, a lone `\`: no regex
    let t = lex(r#"a = r"C:\Users\name" + r"$\mathrm{x}$" + r"(a""#);
    plain_string(&t, r"C:\Users\name");
    plain_string(&t, r"$\mathrm{x}$");
    plain_string(&t, "(a");
    // a verbose pattern's comments over lines
    let t = lex("r = re.compile(r\"\"\"\n    \\d+  # digits\n\"\"\", re.X)\n");
    has(
        &t,
        "#",
        "string.quoted.double.block.python comment.line.number-sign.regexp punctuation.definition.comment.regexp",
    );
    // a docstring is never one
    let t = lex("def f():\n    r\"\"\"^\\d$\"\"\"\n");
    has(&t, r"^\d$", "comment.block.documentation.python");
}

/// `text` is written as a string's text, no regex in it.
#[track_caller]
fn plain_string(t: &[(String, Vec<&str>)], text: &str) {
    assert!(
        t.iter()
            .any(|(x, s)| x == text && s.iter().all(|s| s.starts_with("string."))),
        "{text:?} a plain string's in {t:?}"
    );
}

#[test]
fn re_valid_reads_as_pythons_re() {
    for ok in [
        r"^(\w+)@(\w+)\.com$",
        r"[]a]",
        r"a{2,3}?",
        r"a*+",
        r"\N{BULLET}",
        r"(?P<n>a)|b",
        "plain",
    ] {
        assert!(text::re_valid(ok.as_bytes(), false), "{ok}");
    }
    for bad in [
        r"C:\Users\x",
        r"\l",
        r"(a",
        r"a)",
        r"[a",
        r"*a",
        r"a**",
        r"\x4",
        r"\u12",
        "\\",
    ] {
        assert!(!text::re_valid(bad.as_bytes(), false), "{bad}");
    }
    assert!(text::re_valid(br"\d{{2}}{x}(?:{y})", true));
}

#[test]
fn numbers() {
    let t = lex("x = 0x1F + 0o17 + 0b101 + 1_000 + 1.5e-3 + .5 + 3j + 1e10 + 0777 + 10L\n");
    has(
        &t,
        "0x",
        "constant.numeric.integer.hexadecimal.python punctuation.definition.numeric.base.python",
    );
    has(&t, "1F", "constant.numeric.integer.hexadecimal.python");
    has(&t, "17", "constant.numeric.integer.octal.python");
    has(&t, "101", "constant.numeric.integer.binary.python");
    has(&t, "1_000", "constant.numeric.integer.decimal.python");
    has(
        &t,
        ".",
        "constant.numeric.float.decimal.python punctuation.separator.decimal.python",
    );
    has(&t, "5e-3", "constant.numeric.float.decimal.python");
    has(
        &t,
        "j",
        "constant.numeric.imaginary.decimal.python storage.type.numeric.python",
    );
    has(&t, "1e10", "constant.numeric.float.decimal.python");
    has(&t, "777", "constant.numeric.integer.octal.python");
    has(
        &t,
        "L",
        "constant.numeric.integer.decimal.python storage.type.numeric.python",
    );
}

#[test]
fn a_block_lexes_as_its_lines() {
    // every field a line hands the next: frames past the levels (of every kind), a string open
    // (each quote and content: a docstring, SQL, a regex, undecided), an f-string's field, a
    // decorator's arguments, a head under brackets, a `\`
    let kinds = "f(a=def_[b, {c: (lambda: [".repeat(6);
    let close = "])}])".repeat(6);
    let text = format!(
        "x = {kinds}\n{close}\n\"\"\"doc\n  more\"\"\"\nq = '''\nSELECT a -- c\n  FROM t'''\n\
         r = re.compile(r\"\"\"(a\n  [b # c\n\"\"\")\ns = f\"\"\"x {{\n  y}} {{z\n}} w\"\"\"\n\
         @dec(\n  1,\n)\ndef f(\n  a: int,\n  b=2,\n) -> T:\n  pass\nx = 1 + \\\n  2\nclass C[\n  T\n]: pass\n\
         t = f'a {{b\n}} c'\nu = 'a \\\nb'\n\
         q = f\"\"\"SELECT '{{re.match(\"(a\", s) +\n  x}}' z\"\"\"\n"
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
    assert!(
        usize::from(one[0].1.depth) > LEVELS,
        "past the levels its frames keep"
    );
    assert_eq!(one.last().unwrap().1, State::default());
}

#[test]
fn long_lines_lex_in_linear_time() {
    for unit in [
        "a(",
        "{ ",
        "[",
        "f(a=",
        "def f(",
        "lambda a, ",
        "f\"{",
        "f\"{x:",
        "f'{",
        "\"{",
        "\"{:",
        "\"%(",
        "r\"",
        "r'\\",
        "\"\"\"",
        "re.compile(\"",
        "re.compile(\"(",
        "execute(\"select ",
        "f\"SELECT {a} ",
        "@a(",
        "x.",
        "match ",
        "type x",
        "print ",
        "# ",
        "\\",
        "'\\",
        "0x",
        "1.",
        "from .",
        "import a.",
        "except* ",
    ] {
        let l = unit.repeat(20_000);
        let t = std::time::Instant::now();
        lines(&l);
        assert!(t.elapsed().as_millis() < 300, "{unit:?}: {:?}", t.elapsed());
    }
}

// The compiled scanner's tests (phase 3, X-133), on this one.

/// The SQL in a string runs on SQL's scanner, embedded: the string's own patterns come first at
/// each place they can start, and SQL's take the rest.
#[test]
fn sql_in_strings_takes_turns_with_the_string() {
    let dq = "string.quoted.double.python";
    // an escape ending in a word's byte leaves no `\b` before `FROM`
    let t = lex(r#"q = "SELECT a\nFROM b WHERE c = %s""#);
    has(&t, "SELECT", &format!("{dq} keyword.other.DML.sql"));
    has(&t, r"\n", &format!("{dq} constant.character.escape.python"));
    has(&t, "WHERE", &format!("{dq} keyword.other.DML.sql"));
    has(&t, "%s", &format!("{dq} constant.other.placeholder.python"));
    // the string's end inside a SQL string and a SQL comment
    let t = lex(r#"q = "SELECT 'a" + "INSERT INTO t -- c" + x"#);
    has(
        &t,
        "'",
        &format!("{dq} string.quoted.single.sql punctuation.definition.string.begin.sql"),
    );
    has(&t, " c", &format!("{dq} comment.line.double-dash.sql"));
    has(&t, "+", "keyword.operator.arithmetic.python");
    // `create … view\s*` takes the line's `\n`; the next line's name is plain
    let t = lex("q = \"\"\"CREATE VIEW \nv AS SELECT 1\"\"\"");
    assert!(!is(&t, "v", "entity"), "{t:?}");
    has(
        &t,
        "AS",
        "string.quoted.double.block.python keyword.operator.assignment.alias.sql",
    );
}

#[test]
fn constant_names() {
    assert!(is_constant_name(b"ABC"));
    assert!(is_constant_name(b"_AB1"));
    assert!(is_constant_name(b"MAX_SIZE"));
    assert!(!is_constant_name(b"A_B"));
    assert!(!is_constant_name(b"Abc"));
    assert!(!is_constant_name(b"A"));
    assert!(is_constant_name("ÄÖ_1".as_bytes()));
}

/// No limit of syntect's on nesting (X-133): past its 99 levels a string still opens and a
/// call's name still reads.
#[test]
fn nesting_past_syntects_depth() {
    let t = lex(&format!("x = {}'s', len(y)", "(".repeat(120)));
    assert!(is(&t, "s", "string.quoted.single.python"), "{t:?}");
    assert!(is(&t, "len", "support.function.builtin.python"), "{t:?}");
}
