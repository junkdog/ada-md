use super::*;
use crate::Lang;
use crate::tests::marked;

fn of(lines: &[&str]) -> Vec<(String, String)> {
    marked(Lang::Nix, lines)
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

fn lacks(lines: &[&str], text: &str, scopes: &str) {
    let got = of(lines);
    assert!(
        !got.iter().any(|(t, s)| t == text && s == scopes),
        "{text:?} as {scopes:?} in {got:#?}"
    );
}

/// Every span of `lines`, as `text scopes` a line.
fn all(lines: &[&str]) -> String {
    of(lines)
        .into_iter()
        .map(|(t, s)| format!("{t:?} {s}\n"))
        .collect()
}

#[test]
fn bindings_and_formals() {
    let l = [
        "{ lib, stdenv, fetchurl ? null, ... }@args:",
        "{ pkgs",
        ", lib",
        "}:",
        "let",
        "  cfg = config.services.foo;",
        "  inherit (pkgs) hello;",
        "  inherit lib map;",
        "in rec {",
        "  a.b.c = 1;",
        "  \"quoted\".d = 2;",
        "  e.\"q\".f = 3;",
        "  ${dyn}.g = 4;",
        "  h = x: y: x;",
        "  i = { j }: j;",
        "  k = with lib; mkIf cfg.enable { };",
        "  l = a == b;",
        "  m = f",
        "    n;",
        "  o = if p then",
        "    null",
        "  else q;",
        "  r = { s, ...",
        "  }: s;",
        "}",
    ];
    has(&l, "lib", "variable.parameter.function.1.nix");
    has(&l, "fetchurl", "variable.parameter.function.1.nix");
    has(&l, "... ", "keyword.operator.nix");
    has(&l, "args", "variable.parameter.function.4.nix");
    has(&l, "pkgs", "variable.parameter.function.1.nix");
    has(&l, "}", "punctuation.definition.entity.function.nix");
    has(&l, ":", "punctuation.definition.function.nix");
    has(&l, "let", "keyword.other.nix");
    has(&l, "cfg", "entity.other.attribute-name.multipart.nix");
    has(&l, "=", "keyword.operator.bind.nix");
    has(&l, "config", "variable.parameter.name.nix");
    has(&l, ".", "keyword.operator.nix");
    has(&l, ";", "punctuation.terminator.bind.nix");
    has(&l, "inherit", "keyword.other.inherit.nix");
    has(&l, "(", "punctuation.section.function.arguments.nix");
    has(&l, "hello", "entity.other.attribute-name.single.nix");
    // a builtin's name inherited is an attribute's
    has(&l, "map", "entity.other.attribute-name.single.nix");
    has(&l, ";", "punctuation.terminator.inherit.nix");
    has(&l, "in", "keyword.other.nix");
    has(&l, "rec", "keyword.other.nix");
    has(&l, "c", "entity.other.attribute-name.multipart.nix");
    // an attribute path's dots plain, its parts after a string or an interpolation attributes'
    lacks(&l, ".", "");
    has(&l, "d", "entity.other.attribute-name.multipart.nix");
    has(&l, "f", "entity.other.attribute-name.multipart.nix");
    has(&l, "g", "entity.other.attribute-name.multipart.nix");
    has(&l, "dyn", "variable.parameter.name.nix");
    has(&l, "quoted", "string.quoted.double.nix");
    has(&l, "x", "variable.parameter.function.4.nix");
    has(&l, "j", "variable.parameter.function.1.nix");
    has(&l, "with", "keyword.other.nix");
    has(&l, "mkIf", "variable.parameter.name.nix");
    // `==` isn't a binding's `=`; a value's line isn't an item's start
    has(&l, "a", "variable.parameter.name.nix");
    has(&l, "n", "variable.parameter.name.nix");
    has(&l, "null", "constant.language.nix");
    // bat's operator scope over the blanks after `...`, to the `}` on the next line
    has(&l, "...", "keyword.operator.nix");
    has(&l, "  ", "keyword.operator.nix");
    has(&l, "{", "punctuation.definition.attrset-or-function.nix");
    has(&l, "}", "punctuation.definition.attrset.nix");
    let s = all(&l);
    // `with`'s `;` plain, as bat's
    assert!(s.contains("\"mkIf\""), "{s}");
}

#[test]
fn strings_and_interpolation() {
    let l = [
        "{",
        "  a = \"x ${y} \\n \\${no} $${no} $$${yes} end\";",
        "  b = ''",
        "    text ${z} ''${no} ''' ''\\n $${no}",
        "    ${ \"in ${w}\" }",
        "  '';",
        "  c = \"multi",
        "    line\";",
        "  d = \"${",
        "    e",
        "  } f\";",
        "}",
    ];
    has(&l, "${", "punctuation.section.embedded.begin.nix");
    has(&l, "}", "punctuation.section.embedded.end.nix");
    has(&l, "y", "variable.parameter.name.nix");
    has(
        &l,
        "\\n",
        "string.quoted.double.nix constant.character.escape.nix",
    );
    has(
        &l,
        "\\$",
        "string.quoted.double.nix constant.character.escape.nix",
    );
    // `$${` is text, `$$${` text and an interpolation
    has(&l, "{no} $${no} $$", "string.quoted.double.nix");
    has(&l, "yes", "variable.parameter.name.nix");
    has(
        &l,
        "''",
        "string.quoted.other.nix punctuation.definition.string.other.start.nix",
    );
    has(
        &l,
        "''$",
        "string.quoted.other.nix constant.character.escape.nix",
    );
    has(
        &l,
        "'''",
        "string.quoted.other.nix constant.character.escape.nix",
    );
    has(
        &l,
        "''\\n",
        "string.quoted.other.nix constant.character.escape.nix",
    );
    has(&l, "w", "variable.parameter.name.nix");
    has(
        &l,
        "''",
        "string.quoted.other.nix punctuation.definition.string.other.end.nix",
    );
    has(&l, "    line", "string.quoted.double.nix");
    has(&l, "e", "variable.parameter.name.nix");
    has(&l, " f", "string.quoted.double.nix");
    // no italics over an interpolation
    lacks(&l, " ", "markup.italic");
}

#[test]
fn paths_uris_numbers_operators() {
    let l = [
        "[",
        "  ./foo.nix ../bar/baz /etc/nixos ~/x/y <nixpkgs> <nixpkgs/lib> a/b",
        "  ./foo/${name}.nix ./${x}",
        "  https://example.org/a?b=c x:x git+https://x.org/y",
        "  42 1.5 1.5e3 .5 2.",
        "  (a // b) (c ++ d) (!e && f || g -> h) (i ? j) (k.l or m) (n < o) (-p)",
        "  (q / r)",
        "  (builtins.toString true) (import ./x.nix) (lib.map f) (fetchTarball u)",
        "  (builtins.readFile ./f) (cfg.placeholder) (import-cargo.x) (a.builtins.y)",
        "]",
    ];
    has(&l, "./foo.nix", "string.unquoted.path.nix");
    has(&l, "../bar/baz", "string.unquoted.path.nix");
    has(&l, "/etc/nixos", "string.unquoted.path.nix");
    has(&l, "~/x/y", "string.unquoted.path.nix");
    has(&l, "<nixpkgs>", "string.unquoted.spath.nix");
    has(&l, "<nixpkgs/lib>", "string.unquoted.spath.nix");
    has(&l, "a/b", "string.unquoted.path.nix");
    // a path runs on through an interpolation
    has(&l, "./foo/", "string.unquoted.path.nix");
    has(&l, ".nix", "string.unquoted.path.nix");
    has(&l, "./", "string.unquoted.path.nix");
    has(&l, "https://example.org/a?b=c", "string.unquoted.url.nix");
    has(&l, "x:x", "string.unquoted.url.nix");
    has(&l, "git+https://x.org/y", "string.unquoted.url.nix");
    has(&l, "42", "constant.numeric.nix");
    has(&l, "1.5", "constant.numeric.nix");
    has(&l, "1.5e3", "constant.numeric.nix");
    has(&l, ".5", "constant.numeric.nix");
    has(&l, "2.", "constant.numeric.nix");
    for op in ["//", "++", "!", "&&", "||", "->", "?", "or", "<", "-", "/"] {
        has(&l, op, "keyword.operator.nix");
    }
    has(&l, "builtins", "constant.language.nix");
    has(&l, "toString", "support.function.nix");
    has(&l, "true", "constant.language.nix");
    has(&l, "import", "support.function.nix");
    // a member is a name, a builtin under `builtins`
    has(&l, "map", "variable.parameter.name.nix");
    lacks(&l, "map", "support.function.nix");
    has(&l, "readFile", "support.function.nix");
    has(&l, "placeholder", "variable.parameter.name.nix");
    has(&l, "import-cargo", "variable.parameter.name.nix");
    has(&l, "y", "variable.parameter.name.nix");
    has(&l, "fetchTarball", "support.function.nix");
    has(&l, "[", "punctuation.definition.list.nix");
    has(&l, "(", "punctuation.definition.expression.nix");
}

#[test]
fn comments_and_remarks() {
    let l = ["# TODO: x DEBUG", "/* FIXME a", "   BUG b !!! */ x = 1;"];
    has(&l, "# ", "comment.line.number-sign.nix");
    has(
        &l,
        "TODO",
        "comment.line.number-sign.nix markup.bold.comment.nix",
    );
    has(&l, ": x DEBUG", "comment.line.number-sign.nix");
    has(&l, "/* ", "comment.block.nix");
    has(&l, "FIXME", "comment.block.nix markup.bold.comment.nix");
    has(&l, "BUG", "comment.block.nix markup.bold.comment.nix");
    has(&l, "!!!", "comment.block.nix markup.bold.comment.nix");
    has(&l, " */", "comment.block.nix");
    has(&l, "1", "constant.numeric.nix");
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
    // every field a line hands the next: frames past the 32 kept levels, of every kind, a
    // string of each kind and a comment across lines, a path under its interpolation
    let deep = "{ a = [ ( let b = {\n".repeat(6)
        + "c = ''\n${ \"${ ./p/${\n"
        + &"with d; inherit (e\n".repeat(4)
        + &") f;\n".repeat(4)
        + "q }.nix\" }\n'';\n";
    let text = format!(
        "{deep}{}\n/* g\nh */ i = \"j\nk\";\n",
        "}; in b ) ] ; }\n".repeat(6)
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
}

#[test]
fn long_lines_lex_in_linear_time() {
    for unit in [
        "a ",
        "a.b ",
        "a = ",
        "{ a.",
        "{ a.b.",
        "\"${",
        "''${",
        "./a/${",
        "x:",
        "/*",
        "1.",
        "a/",
        "<a/",
        "{ a, ",
        "inherit (",
        "# TODO ",
        "a.\"",
        "a.${",
    ] {
        let l = unit.repeat(20_000);
        let t = std::time::Instant::now();
        lines(&l);
        assert!(t.elapsed().as_millis() < 200, "{unit:?}: {:?}", t.elapsed());
    }
}
