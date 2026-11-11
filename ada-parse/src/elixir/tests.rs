//! Elixir's scanner's tests (X-138): the readings ada adds where bat's grammar has none (parity
//! can't see them), state across lines, a block against its lines, long lines.

use super::*;
use crate::{Lang, Span};

/// `text`'s spans a line at a time, each as its text and its kind's last scope.
fn toks(text: &str) -> Vec<Vec<(String, String)>> {
    let mut st = State::default();
    let mut v: Vec<Span> = Vec::new();
    let mut out = Vec::new();
    for l in text.lines() {
        crate::lex(Lang::Elixir, l, &mut st, &mut v);
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

/// The leaf `text`'s token `tok` has on line `line` (0-based), its first occurrence; `""` for
/// plain text.
fn leaf(t: &[Vec<(String, String)>], line: usize, tok: &str) -> String {
    t[line]
        .iter()
        .find(|(s, _)| s == tok)
        .map_or_else(String::new, |(_, k)| k.clone())
}

#[test]
fn heads_and_keys() {
    let t = toks("defmodule A.B do\n  def f(x) when is_atom(x), do: x\n  defp g, do: :ok\nend");
    assert_eq!(leaf(&t, 0, "A"), "entity.name.class.elixir");
    assert_eq!(leaf(&t, 1, "f"), "entity.name.function.public.elixir");
    assert_eq!(leaf(&t, 1, "is_atom"), "keyword.guard.elixir");
    assert_eq!(leaf(&t, 1, "do:"), "constant.other.keywords.elixir");
    assert_eq!(leaf(&t, 2, "g"), "entity.name.function.private.elixir");
    assert_eq!(leaf(&t, 2, "ok"), "constant.other.symbol.elixir");
}

#[test]
fn heredocs_end_at_a_line_starting_with_their_delimiter() {
    // bat's ends at `text """` and inverts the rest
    let t = toks("x = \"\"\"\n  a\n  text \"\"\"\n  \"\"\" <> y\nz = :ok");
    assert_eq!(
        leaf(&t, 2, "  text \"\"\""),
        "string.quoted.triple.double.elixir"
    );
    assert_eq!(
        leaf(&t, 3, "\"\"\""),
        "punctuation.definition.string.end.elixir"
    );
    assert_eq!(
        leaf(&t, 3, "<>"),
        "keyword.operator.binary-concatenation.elixir"
    );
    assert_eq!(leaf(&t, 4, "ok"), "constant.other.symbol.elixir");
}

#[test]
fn documentation_ends_where_elixir_ends_it() {
    // bat's doc ends at `"""` anywhere on a line
    let t = toks("@doc ~S\"\"\"\niex> ~s\"\"\"\n...> hi\n...> \"\"\"\n\"\"\"\ndef f, do: 1");
    assert_eq!(leaf(&t, 3, "...> \"\"\""), "comment.documentation.heredoc");
    assert_eq!(leaf(&t, 4, "\"\"\""), "comment.documentation.heredoc");
    assert_eq!(leaf(&t, 5, "f"), "entity.name.function.public.elixir");
    let t = toks("@moduledoc false\n@doc \"a #{b} \\n\"");
    assert_eq!(
        leaf(&t, 0, "@moduledoc false"),
        "comment.documentation.false"
    );
    assert_eq!(leaf(&t, 1, "\\n"), "constant.character.escape.char.elixir");
    assert_eq!(
        leaf(&t, 1, "#{"),
        "punctuation.section.interpolation.begin.elixir"
    );
}

#[test]
fn sigils() {
    // an uppercase sigil's `\` takes the delimiter after it (bat's ends there)
    let t = toks("a = ~S\"x \\\" y\" <> ~S(p \\) q)\nb = 1");
    assert_eq!(
        leaf(&t, 0, "<>"),
        "keyword.operator.binary-concatenation.elixir"
    );
    assert_eq!(leaf(&t, 1, "1"), "constant.numeric.elixir");
    // Elixir 1.15's multi-letter sigils
    let t = toks("~HTML\"\"\"\n<p>x</p>\n\"\"\"\n~w(a b)a");
    assert_eq!(leaf(&t, 0, "~HTML"), "storage.type.string.elixir");
    assert_eq!(leaf(&t, 3, ")"), "punctuation.definition.string.end.elixir");
    assert_eq!(leaf(&t, 3, "a"), "storage.type.string.elixir");
    // a sigil's closing `/` a delimiter, as its opening one
    let t = toks("~r/a+/i");
    assert_eq!(leaf(&t, 0, "+"), "keyword.operator.quantifier.regexp");
    assert_eq!(leaf(&t, 0, "i"), "storage.type.string.elixir");
    let n = t[0]
        .iter()
        .filter(|(_, k)| k == "punctuation.definition.string.end.elixir");
    assert_eq!(n.count(), 1);
    // an escaped delimiter is the regex's escape, not its end
    let t = toks("~r/[a\\/]+/");
    assert_eq!(leaf(&t, 0, "+"), "keyword.operator.quantifier.regexp");
    // a heredoc regex's comments
    let t = toks("~r\"\"\"\n  a+  # one or more\n\"\"\"x");
    assert_eq!(
        leaf(&t, 1, "# one or more"),
        "comment.line.number-sign.regexp.elixir"
    );
}

#[test]
fn function_heads() {
    // a head without a body ends with its line (bat's goes on to the next `do`)
    let t = toks("defprotocol P do\n  def size(data)\n  def f(x), do: length(x)\nend");
    assert_eq!(leaf(&t, 0, "P"), "entity.name.protocol.elixir");
    assert_eq!(leaf(&t, 2, "length"), "");
    // a head across lines, `when` on the next one
    let t = toks("def f(a,\n      b)\n    when is_atom(a) do\n  is_atom(b)\nend");
    assert_eq!(leaf(&t, 2, "is_atom"), "keyword.guard.elixir");
    assert_eq!(leaf(&t, 2, "do"), "keyword.control.module.elixir");
    assert_eq!(leaf(&t, 3, "is_atom"), "");
    // `def unquote(name)(…)`: `unquote` a keyword; a capture's name with its `?`
    let t = toks("def unquote(name)(x), do: x\nEnum.filter(l, &valid?/1)");
    assert_eq!(leaf(&t, 0, "unquote"), "keyword.control.elixir");
    assert_eq!(leaf(&t, 1, "valid?"), "variable.other.capture.elixir");
    // a `do:` keyword parameter inside the head's parentheses doesn't end it
    let t = toks("def f(a, do: b) when is_nil(a) do");
    assert_eq!(leaf(&t, 0, "is_nil"), "keyword.guard.elixir");
    // `fn`'s `when` before its `->` is a keyword
    let t = toks("f = fn x when x > 0 -> x end\ng = fn(x) when x -> x end");
    assert_eq!(leaf(&t, 0, "when"), "keyword.control.elixir");
    assert_eq!(leaf(&t, 1, "when"), "keyword.operator.elixir");
}

#[test]
fn keys_atoms_and_binaries() {
    let t = toks("%{\"a b\": 1, c: 2, do: :x, nil: 3} |> f(:\"q #{x}\", :<<>>, :+)");
    assert_eq!(
        leaf(&t, 0, "a b"),
        "constant.other.symbol.double-quoted.elixir"
    );
    assert_eq!(
        leaf(&t, 0, "\":"),
        "punctuation.definition.constant.end.regexp.elixir"
    );
    assert_eq!(leaf(&t, 0, "c"), "constant.other.keywords.elixir");
    assert_eq!(leaf(&t, 0, "nil"), "constant.language.elixir");
    assert_eq!(leaf(&t, 0, "<<>>"), "constant.other.symbol.elixir");
    let t = toks("<<a::binary-size(len), 1::8,\n  b::utf8>> = x");
    assert_eq!(leaf(&t, 0, "len"), "variable.numeric.elixir");
    assert_eq!(leaf(&t, 0, "8"), "constant.numeric.elixir");
    assert_eq!(leaf(&t, 1, "utf8"), "keyword.type.binary.elixir");
    assert_eq!(leaf(&t, 1, "="), "keyword.operator.assignment.elixir");
}

#[test]
fn sql_and_regexes_in_strings() {
    let t = toks(concat!(
        "fragment(\"lower(?) = ?\", u.email, ^e)\n",
        "Repo.query!(\"SELECT * FROM users WHERE id = $1\", [id])\n",
        "execute \"CREATE INDEX i ON t (a)\"\n",
        "repo().query!(\"\"\"\n",
        "  SELECT count(*) -- n\n",
        "  FROM posts WHERE id = #{id}\n",
        "\"\"\")\n",
        "Regex.compile!(\"^\\\\d+$\", \"i\")\n",
        "IO.puts(\"SELECT 1\")",
    ));
    assert_eq!(leaf(&t, 0, "lower"), "support.function.string.sql");
    assert_eq!(leaf(&t, 1, "SELECT"), "keyword.other.DML.sql");
    assert_eq!(leaf(&t, 2, "CREATE"), "keyword.other.create.sql");
    assert_eq!(leaf(&t, 4, "SELECT"), "keyword.other.DML.sql");
    assert_eq!(leaf(&t, 5, "FROM"), "keyword.other.DML.sql");
    assert_eq!(
        leaf(&t, 5, "#{"),
        "punctuation.section.interpolation.begin.elixir"
    );
    assert_eq!(leaf(&t, 7, "+"), "keyword.operator.quantifier.regexp");
    // the second string is the options, not a regex
    assert_eq!(leaf(&t, 7, "i"), "string.quoted.double.elixir");
    assert_eq!(leaf(&t, 8, "SELECT 1"), "string.quoted.double.elixir");
}

/// A text that drives every field the state keeps: frames past their levels, strings inside
/// `#{…}` past eight, a heredoc, a binary's type, a head across lines, SQL and a regex across
/// lines.
fn deep_text() -> String {
    let mut s = String::new();
    s.push_str(&"[{(".repeat(12));
    s.push('\n');
    s.push_str(&"\"#{".repeat(10));
    s.push_str("\n}\"}\"\n");
    s.push_str("x = \"\"\"\n  #{a <> \"b\n c\"}\n\"\"\"\n<<a::\n  binary, b>>\n");
    s.push_str("def f(a,\n  b) when\n  is_atom(a) do\nend\n");
    s.push_str(
        "execute \"\"\"\n SELECT 1 /* c\n */ FROM t\n\"\"\"\n~r\"\"\"\n (a [b\n c]) # x\n\"\"\"x\n",
    );
    s.push_str("~H\"\"\"\n<div\n  class={[\n \"a\",\n @b]} id=\"x\">\n<%= if @c do %>{@d}<% end %>\n<p style=\"color:\n red\">\n\"\"\"\n");
    s.push_str(&")}]".repeat(12));
    s
}

#[test]
fn heex_templates() {
    let t = toks(concat!(
        "~H\"\"\"\n",
        "<div class={@class} id=\"main\">\n",
        "  <.link navigate={~p\"/a/#{x}\"}>{x.name}</.link>\n",
        "  <%= for i <- @items do %><p style=\"color: {red}\">{i}</p><% end %>\n",
        "  <span class={[\n",
        "    \"a\", @b\n",
        "  ]}>&amp;</span>\n",
        "\"\"\"\n",
        "x = ~H\"<i>{@y}</i>\" <> z",
    ));
    assert_eq!(leaf(&t, 1, "div"), "entity.name.tag.block.any.html");
    assert_eq!(
        leaf(&t, 1, "class"),
        "entity.other.attribute-name.class.html"
    );
    assert_eq!(
        leaf(&t, 1, "{"),
        "punctuation.section.embedded.begin.elixir"
    );
    assert_eq!(
        leaf(&t, 1, "@"),
        "keyword.operator.definition.constant.elixir"
    );
    // the attribute after an island's value is an attribute's
    assert_eq!(leaf(&t, 1, "id"), "entity.other.attribute-name.id.html");
    assert_eq!(leaf(&t, 2, ".link"), "entity.name.tag.other.html");
    assert_eq!(leaf(&t, 2, "~p"), "storage.type.string.elixir");
    assert_eq!(
        leaf(&t, 3, "<%="),
        "punctuation.section.embedded.begin.elixir"
    );
    assert_eq!(leaf(&t, 3, "for"), "keyword.control.elixir");
    assert_eq!(leaf(&t, 3, "%>"), "punctuation.section.embedded.end.elixir");
    // a quoted attribute value's `{` is text
    assert_eq!(
        leaf(&t, 3, "red"),
        "support.constant.color.w3c-standard-color-name.css"
    );
    // an island across lines: code, then the template again
    assert_eq!(leaf(&t, 5, "a"), "string.quoted.double.elixir");
    assert_eq!(leaf(&t, 6, "amp"), "constant.character.entity.named.html");
    assert_eq!(
        leaf(&t, 7, "\"\"\""),
        "punctuation.definition.string.end.elixir"
    );
    assert_eq!(leaf(&t, 8, "i"), "entity.name.tag.inline.any.html");
    assert_eq!(
        leaf(&t, 8, "<>"),
        "keyword.operator.binary-concatenation.elixir"
    );
    // EEx's and HEEx's comments across lines
    let t = toks("~H\"\"\"\n<%!-- the org's\nlist --%><p>\n<%# a\n'b %><i>\n\"\"\"\nx = 1");
    assert_eq!(
        leaf(&t, 1, "<%!-- the org's"),
        "comment.line.number-sign.elixir"
    );
    assert_eq!(leaf(&t, 2, "p"), "entity.name.tag.block.any.html");
    assert_eq!(leaf(&t, 4, "i"), "entity.name.tag.inline.any.html");
    assert_eq!(leaf(&t, 6, "1"), "constant.numeric.elixir");
    // a `<style>`'s and a `<script>`'s text has no islands
    let t = toks(
        "~H\"\"\"\n<style :type={X}>\n.a { color: red; }\n</style>\n<script>if (a) { b() }</script>{@c}\n\"\"\"\nx = 1",
    );
    assert_eq!(leaf(&t, 1, "X"), "entity.name.class.elixir");
    assert_eq!(leaf(&t, 2, "color"), "support.type.property-name.css");
    assert_eq!(
        leaf(&t, 4, "@"),
        "keyword.operator.definition.constant.elixir"
    );
    assert_eq!(leaf(&t, 6, "1"), "constant.numeric.elixir");
}

#[test]
fn a_block_lexes_as_its_lines() {
    let text = deep_text();
    let lines: Vec<&str> = text.lines().collect();
    let mut st = State::default();
    let mut v = Vec::new();
    let mut want = Vec::new();
    for l in &lines {
        crate::lex(Lang::Elixir, l, &mut st, &mut v);
        want.push((v.clone(), st));
    }
    let mut st = State::default();
    let mut spans = Vec::new();
    let mut got = Vec::new();
    block(&mut st, &mut spans, &mut |c| {
        for l in &lines {
            let v = c.next(l.as_bytes()).to_vec();
            got.push(v);
        }
        None
    });
    for (i, (w, g)) in want.iter().zip(&got).enumerate() {
        assert_eq!(&w.0, g, "line {i}: {:?}", lines[i]);
    }
    assert_eq!(want.last().unwrap().1, st);
}

#[test]
fn long_lines_lex_in_linear_time() {
    for unit in [
        "def f(a) when is_atom(a), do: \"#{a}\" <> ~r/x+/ ",
        "<<a::size(8), \"b\", 1>> ",
        "fn x when x -> x end ",
        "\"a\": 1, :b, &c/1, ?d, @e ",
        "fragment(\"? = ?\", a, b) ",
        "{[(",
        "\"#{",
    ] {
        let line = unit.repeat(20_000);
        let t = std::time::Instant::now();
        let mut st = State::default();
        let mut v = Vec::new();
        crate::lex(Lang::Elixir, &line, &mut st, &mut v);
        assert!(
            t.elapsed().as_secs_f64() < 2.0,
            "{unit:?}: {:?}",
            t.elapsed()
        );
    }
}

/// The fuzzer's findings: an attribute's island left open to a line that ends in a character
/// past ASCII (the quoted value HTML's scanner was given ended inside it).
#[test]
fn fuzz_findings() {
    for text in [
        "~H\"\"\"\n<div a={b \u{fffd}\nc}>\n\"\"\"",
        "~H\"<p x={é\"",
        "~H\"\"\"\n<i y=<%= é",
    ] {
        let mut st = State::default();
        let mut v = Vec::new();
        for l in text.lines() {
            crate::lex(Lang::Elixir, l, &mut st, &mut v);
            for s in &v {
                let (a, b) = (s.start as usize, s.end as usize);
                assert!(
                    l.is_char_boundary(a) && l.is_char_boundary(b),
                    "{l:?}: {v:?}"
                );
            }
        }
    }
}
