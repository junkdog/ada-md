//! Go's scanner's tests: the compiled scanner's (X-168 kept them), then the readings of X-168.

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
    marked(Lang::Go, lines)
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

#[test]
fn regexes_in_strings() {
    const RE: &str = "string.quoted.double.go";
    const RAW: &str = "string.quoted.other.go";
    let l = [
        r#"var a = regexp.MustCompile("^(?P<y>\\d{4})\t[]a-z]%d{{x}}$")"#,
        "ok, _ := regexp.MatchString(`(?i:[[:alpha:]]+|\\p{^Greek})*?",
        "\\Q*\\E\\x{41}\\101`, s)",
        r#"fmt.Printf("%d{2}\\d", x); f := myregexp.MustCompile("a+")"#,
    ];
    has(&l, "^", &format!("{RE} keyword.control.anchor.regexp"));
    has(
        &l,
        "?P",
        &format!("{RE} punctuation.definition.group.begin.regexp"),
    );
    has(&l, "y", &format!("{RE} entity.name.other.group.regexp"));
    has(
        &l,
        r"\\d",
        &format!("{RE} constant.other.character-class.escape.backslash.regexp"),
    );
    // the host's escape between the regex's tokens
    has(&l, r"\t", &format!("{RE} constant.character.escape.go"));
    // a `]` first in a class is a character; `%d` and `{{` are the regex's text
    has(
        &l,
        "]",
        &format!("{RE} constant.other.character-class.set.regexp"),
    );
    let got = of(&l[..1]);
    assert!(
        !got.iter()
            .any(|(_, s)| s.contains("placeholder") || s.contains("interpolation")),
        "{got:?}"
    );
    // a raw regex across lines
    has(
        &l,
        "?i",
        &format!("{RAW} keyword.other.option-toggle.regexp"),
    );
    has(
        &l,
        "[:alpha:]",
        &format!(
            "{RAW} constant.other.character-class.set.regexp constant.other.character-class.posix.regexp"
        ),
    );
    has(&l, "^", &format!("{RAW} keyword.operator.negation.regexp"));
    has(
        &l,
        "Greek",
        &format!("{RAW} support.constant.unicode-property.regexp"),
    );
    has(
        &l,
        r"\Q",
        &format!("{RAW} constant.character.escape.backslash.regexp"),
    );
    has(
        &l,
        r"\x{41}",
        &format!("{RAW} constant.character.escape.backslash.regexp"),
    );
    has(
        &l,
        r"\101",
        &format!("{RAW} constant.character.escape.backslash.regexp"),
    );
    // not a regex: any other call
    has(&l, "%d", &format!("{RE} constant.other.placeholder.go"));
    let got = of(&l[3..]);
    assert!(!got.iter().any(|(_, s)| s.contains("regexp")), "{got:?}");
}

#[test]
fn sql_in_strings() {
    const DQ: &str = "string.quoted.double.go";
    const RAW: &str = "string.quoted.other.go";
    let dml = |s: &str| format!("{s} keyword.other.DML.sql");
    let l = [
        "// language=SQL",
        r#"var q = "SELECT a FROM t WHERE b = '%d'\n""#,
        r#"rows, err := db.Query("DELETE FROM u WHERE id = $1", id)"#,
        "err = conn.QueryRow(ctx, `UPDATE u SET n = 2`).Scan(&n)",
        r#"_, err = db.Exec(query, "LIMIT")"#,
        r#"err = db.Get(&p, "INSERT INTO p VALUES (1)")"#,
        r#"v := params.Get("ORDER")"#,
        r#"_, err = conn.Prepare(ctx, "GROUP", "VALUES (2)")"#,
        "rows, _ := conn.Query(",
        "\tcontext.Background(),",
        r#"	"UNION ALL SELECT n","#,
        ")",
        "_, err = tx.ExecContext(c, `",
        "  ALTER TABLE foo -- a note",
        "  ADD id int`)",
        r#"db.Exec("HAVING " + "JOIN x", "WHERE")"#,
        r#"f("NULL"); db.Exec(fmt.Sprintf("BETWEEN %d", 1))"#,
        r#"stmt.Exec("CASE"); insertStmt.Get(&x, "ELSE")"#,
    ];
    // a marker's string; the host's escape between SQL's tokens, `%d` SQL's text
    has(&l, "SELECT", &dml(DQ));
    has(&l, r"\n", &format!("{DQ} constant.character.escape.go"));
    has(&l, "%d", &format!("{DQ} string.quoted.single.sql"));
    // the first argument, after a context, after a destination
    has(&l, "DELETE", &dml(DQ));
    // a parameter (X-129)
    has(&l, "$1", &format!("{DQ} constant.other.placeholder.sql"));
    has(&l, "UPDATE", &dml(RAW));
    has(&l, "INSERT INTO", &dml(DQ));
    // pgx's statement name, then its SQL
    has(&l, "VALUES", &format!("{DQ} keyword.other.DML.II.sql"));
    has(&l, "GROUP", DQ);
    // over lines, after a context
    has(&l[8..12], "UNION ALL", &dml(DQ));
    // a raw string over lines: SQL's line comment ends with its line
    has(
        &l,
        " a note",
        &format!("{RAW} comment.line.double-dash.sql"),
    );
    has(&l, "ADD", &format!("{RAW} keyword.other.sql"));
    // `+` continues the SQL; not the next argument
    has(&l, "JOIN", &dml(DQ));
    has(&l, "WHERE", DQ);
    // a variable first, a string first to `Get`, any other call, a call in the argument,
    // and a prepared statement's arguments
    for w in ["LIMIT", "ORDER", "NULL", "BETWEEN", "CASE", "ELSE"] {
        assert!(
            of(&l)
                .iter()
                .any(|(t, s)| t.contains(w) && !s.contains(".sql")),
            "{w}: {:#?}",
            of(&l)
        );
    }
}

#[test]
fn declarations() {
    let l = [
        "package main",
        "import \"fmt\"",
        "type Point struct {",
        "\tX, Y int `json:\"x\"`",
        "\tio.Reader",
        "}",
        "func (p *Point) Add(q Point, n ...int) (r Point, err error) {",
        "\tconst limit = iota",
        "\tvar m map[string][]byte",
        "\tx, _ := make([]int, 3), new(T)",
        "}",
    ];
    has(&l, "package", "keyword.other.package.go");
    has(&l, "Point", "entity.name.type.go");
    has(&l, "struct", "storage.type.keyword.struct.go");
    has(&l, "X", "variable.other.member.declaration.go");
    has(&l, "int", "storage.type.go support.type.builtin.go");
    has(&l, "Reader", "entity.other.inherited-class.go");
    has(&l, "func", "storage.type.keyword.function.go");
    has(&l, "p", "variable.parameter.go");
    has(&l, "Add", "entity.name.function.go");
    has(&l, "...", "keyword.operator.variadic.go");
    has(&l, "limit", "variable.other.constant.declaration.go");
    has(&l, "iota", "constant.numeric.integer.decimal.go");
    has(&l, "m", "variable.declaration.go");
    has(&l, "map", "storage.type.keyword.map.go");
    has(&l, "x", "variable.declaration.go");
    has(&l, "_", "variable.language.blank.go");
    has(
        &l,
        "make",
        "variable.function.go support.function.builtin.go",
    );
}

#[test]
fn literals_comments_and_templates() {
    let l = [
        "//go:generate stringer -type=Op",
        "a := 0x1F + 1.5e3 + 0o17i + 'x' + '\\n' + 089 // done",
        "s := \"%5.2f {{.Name | printf}} \\u00e9\" /* block",
        " * still */ return",
    ];
    has(
        &l,
        "generate",
        "comment.line.go meta.annotation.go meta.variable.function.go",
    );
    has(
        &l,
        "-type=Op",
        "comment.line.go meta.annotation.go meta.variable.parameter.go",
    );
    has(
        &l,
        "0x",
        "constant.numeric.integer.hexadecimal.go punctuation.definition.numeric.base.go",
    );
    has(&l, "1F", "constant.numeric.integer.hexadecimal.go");
    has(
        &l,
        ".",
        "constant.numeric.float.decimal.go punctuation.separator.decimal.go",
    );
    has(
        &l,
        "i",
        "constant.numeric.imaginary.octal.go storage.type.numeric.go",
    );
    has(&l, "'x'", "constant.character.go");
    has(&l, "089", "invalid.illegal.go");
    has(
        &l,
        "%5.2f",
        "string.quoted.double.go constant.other.placeholder.go",
    );
    has(&l, "Name", "meta.interpolation.go variable.other.member.go");
    has(
        &l,
        "printf",
        "meta.interpolation.go variable.function.go support.function.builtin.go",
    );
    has(
        &l,
        "*",
        "comment.block.go punctuation.definition.comment.go",
    );
    has(&l, "return", "keyword.control.go");
}

/// No limit of syntect's on nesting (X-133): past its 99 levels a string still opens, its
/// SQL and regexes still read, and a `//` comment still takes the line.
#[test]
fn nesting_past_syntects_depth() {
    let line = format!(
        "x := {}\"s\", regexp.MustCompile(\"a+\") // c",
        "(".repeat(120)
    );
    let got = of(&[&line]);
    let is = |text: &str, scope: &str| got.iter().any(|(t, s)| t == text && s.contains(scope));
    assert!(is("s", "string.quoted.double.go"), "{got:?}");
    assert!(is("+", "keyword.operator.quantifier.regexp"), "{got:?}");
    assert!(is(" c", "comment.line.go"), "{got:?}");
}

/// Whether `text` is never lexed with a scope holding `scope`.
fn lacks(lines: &[&str], text: &str, scope: &str) {
    let got = of(lines);
    assert!(
        !got.iter().any(|(t, s)| t == text && s.contains(scope)),
        "{text:?} with {scope:?} in {got:#?}"
    );
}

/// Generics (Go 1.18): type parameters, constraints, type arguments in a type, a receiver's.
#[test]
fn generics() {
    let l = [
        "func Map[T any, U comparable](xs []T, f func(T) U) []U { return nil }",
        "type List[T any] struct { next *List[T]; val T }",
        "func (l *List[T]) Push(v T) {}",
        "type Number interface { ~int | ~int64 | float64 }",
        "func Sum[S ~[]E, E Number](s S) (total E) { return }",
        "var p = Pair[string, int]{Key: \"a\"}",
    ];
    has(&l, "Map", "entity.name.function.go");
    has(&l, "T", "variable.parameter.go");
    has(&l, "any", "storage.type.go support.type.builtin.go");
    has(&l, "comparable", "storage.type.go support.type.builtin.go");
    has(&l, "xs", "variable.parameter.go");
    has(&l, "U", "storage.type.go");
    has(&l, "List", "entity.name.type.go");
    has(&l, "Push", "entity.name.function.go");
    has(&l, "~", "keyword.operator.go");
    has(&l, "int64", "storage.type.go support.type.builtin.go");
    has(&l, "Sum", "entity.name.function.go");
    has(&l, "S", "storage.type.go");
    has(&l, "total", "variable.parameter.go");
    has(&l, "string", "storage.type.go support.type.builtin.go");
    // `next *List[T]`'s `T` a type, not a field
    lacks(&l[1..2], "T", "member");
}

/// Go's reading where bat's derails: an index's `*` multiplies, a channel's type after `<-`,
/// a builtin's name declared, a type switch's cases.
#[test]
fn positions() {
    let l = [
        "x := y[0] * z",
        "func notify(c chan<- os.Signal) {}",
        "func max(a, b int) int { return a }",
        "func (c *conn) close() error { return nil }",
        "func RangeArgs(min int, max int) {}",
        "rune, size := utf8.DecodeRuneInString(s)",
        "switch v := x.(type) {",
        "case int, *Foo, []byte:",
        "case nil:",
        "}",
        "n := min(a, b) + max(1, 2); clear(m)",
    ];
    has(&l, "z", "variable.other.go");
    has(&l, "Signal", "storage.type.go");
    has(&l, "max", "entity.name.function.go");
    has(&l, "close", "entity.name.function.go");
    has(&l, "min", "variable.parameter.go");
    has(&l, "rune", "variable.declaration.go");
    has(&l, "size", "variable.declaration.go");
    has(&l, "type", "storage.type.keyword.type.go");
    has(&l, "Foo", "storage.type.go");
    has(&l, "byte", "storage.type.go support.type.builtin.go");
    has(&l, "nil", "constant.language.go");
    has(
        &l,
        "min",
        "variable.function.go support.function.builtin.go",
    );
    has(
        &l,
        "clear",
        "variable.function.go support.function.builtin.go",
    );
}

/// Strings as Go reads them: a template's `\"` doesn't run on past its string, fmt's verbs only,
/// an escape Go lacks invalid, a `"…"` string ends with its line.
#[test]
fn strings_as_go_reads_them() {
    let l = [
        r#"var s = "{{ if eq .A \"x\" }}yes{{ end }}""#,
        "var t = 1",
        r#"var w = "50% off %d %-5.2f %[1]q %z %%""#,
        r#"var q = "a\q\x41é""#,
        "var open = \"no end",
        "var after = 2",
        r#"var u = `${{"}}"}}`"#,
        r#"var v = "{{printf `%d` 23}}""#,
    ];
    has(&l, "var", "storage.type.keyword.var.go");
    has(&l, "1", "constant.numeric.integer.decimal.go");
    has(&l, "x", "string.quoted.double.go");
    has(&l, "end", "meta.interpolation.go keyword.control.go");
    has(
        &l,
        "%d",
        "string.quoted.double.go constant.other.placeholder.go",
    );
    has(
        &l,
        "%-5.2f",
        "string.quoted.double.go constant.other.placeholder.go",
    );
    has(
        &l,
        "%[1]q",
        "string.quoted.double.go constant.other.placeholder.go",
    );
    has(
        &l,
        "%%",
        "string.quoted.double.go constant.character.escape.go",
    );
    lacks(&l, "% o", "placeholder");
    lacks(&l, "%z", "placeholder");
    has(
        &l,
        r"\q",
        "string.quoted.double.go invalid.illegal.unknown-escape.go",
    );
    has(
        &l,
        r"\x41",
        "string.quoted.double.go constant.character.escape.go",
    );
    has(&l, "2", "constant.numeric.integer.decimal.go");
    // a template's `}}` past its own strings; fmt's verbs in them
    has(&l, "}}", "string.quoted.double.go");
    has(
        &l,
        "printf",
        "meta.interpolation.go variable.function.go support.function.builtin.go",
    );
    has(
        &l,
        "%d",
        "string.quoted.double.go constant.other.placeholder.go",
    );
    // a template's string's `\` before a character past ASCII takes it whole (the smoke's finding)
    let got = of(&["s := \"{{\\\"\\é\\\"}}\""]);
    assert!(got.iter().any(|(t, _)| *t == "\\é"), "{got:?}");
}

/// A block lexes as its lines do one at a time, past every field's cap: frames deeper than the
/// state keeps, a raw string's SQL and a regex across lines, a SQL call's argument on a later
/// line, a parameter list opened at a line's end.
#[test]
fn a_block_lexes_as_its_lines() {
    let mut text = String::new();
    for i in 0..40 {
        text.push_str(&format!("x{i} := f(struct {{\n"));
    }
    text.push_str("func f(\n\tctx context.Context,\n) (\n\tint,\n\terror,\n) {\n");
    text.push_str("rows, err := db.QueryContext(\n\tctx,\n\t`SELECT a\n FROM t -- c\n WHERE b`)\n");
    text.push_str("re := regexp.MustCompile(`a+\n(b|c)`)\n/* a\n * b */ var x = 1\n");
    for _ in 0..40 {
        text.push_str("})}\n");
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
    assert!(one.iter().any(|(_, s)| s.arg & 7 == u16::from(M_RAW_SQL)));
    assert!(one.iter().any(|(_, s)| s.arg & 7 == u16::from(M_RAW_RE)));
}

/// Each construct repeated on one long line lexes in linear time (the guide's mistake 17).
#[test]
fn long_lines_are_linear() {
    for unit in [
        "a, ",
        "f(a, ",
        "func(",
        "[]map[string]chan<- ",
        "x.y.z(",
        "\"%d {{.A}} \\\" \" + ",
        "case int, ",
        "{{",
    ] {
        let line = unit.repeat(20_000);
        let t = std::time::Instant::now();
        let _ = of(&[&line]);
        assert!(t.elapsed().as_secs() < 2, "{unit:?}: {:?}", t.elapsed());
    }
}
