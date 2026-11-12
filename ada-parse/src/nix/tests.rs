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

/// bash's scope for a command's name.
const COMMAND: &str = "meta.function-call.shell support.function.command.shell";

#[test]
fn shell_in_strings() {
    let l = [
        "{",
        "  postPatch = ''",
        "    substituteInPlace Makefile --replace-fail gcc cc",
        "    for f in ${./a.patch} $out/bin/*; do",
        "      echo \"$f\" ''${HOME} '''q'''",
        "    done",
        "    cat > $out/x <<EOF",
        "    hello ${name}",
        "    EOF",
        "    make",
        "  '';",
        "  description = ''",
        "    echo words",
        "  '';",
        "  installPhase = ''",
        "    ${lib.optionalString stdenv.isDarwin ''",
        "      install_name_tool -id $out/lib/libfoo.dylib",
        "    ''}",
        "    cp -r . $out",
        "  '';",
        "  x = writeShellScript \"x\" ''exit 0'';",
        "  y = /* bash */ ''ls'';",
        "  z = pkgs.runCommand \"z\" {",
        "    buildInputs = [ ];",
        "  } ''",
        "    mkdir $out",
        "  '';",
        "  q = [ (writeShellScriptBin \"a\" ''rm'') ''chmod'' ];",
        "  systemd.services.a.script = ''sleep 1''; preBuild = ''touch'';",
        "  app = writeShellApplication { name = \"a\"; text = ''grep''; };",
        "  t = runCommand \"t\" { s = writeText \"s\" ''<?php echo''; } ''ln -s a $out'';",
        "  postFixup = let f = writeText \"f\" ''set(x'';",
        "    in ''chown a'';",
        "}",
    ];
    has(
        &l,
        "''",
        "string.quoted.other.nix punctuation.definition.string.other.start.nix",
    );
    has(&l, "substituteInPlace", COMMAND);
    has(&l, "for", "keyword.control.loop.for.shell");
    // an interpolation Nix's, its text a word to bash
    has(&l, "./a.patch", "string.unquoted.path.nix");
    has(&l, "out", "variable.other.readwrite.shell");
    // Nix's escapes Nix's, the text they stand for bash's
    has(
        &l,
        "''$",
        "string.quoted.other.nix constant.character.escape.nix",
    );
    has(&l, "HOME", "variable.other.readwrite.shell");
    has(
        &l,
        "'''",
        "string.quoted.other.nix constant.character.escape.nix",
    );
    // a here-document, its closer indented as the string is (Nix strips it)
    has(&l, "EOF", "keyword.control.heredoc-token.shell");
    has(&l, "hello ", "string.unquoted.heredoc.shell");
    has(&l, "make", COMMAND);
    // not a script
    has(&l, "    echo words", "string.quoted.other.nix");
    // a string spliced in from an interpolation, the script going on after it
    has(&l, "install_name_tool", COMMAND);
    has(&l, "cp", COMMAND);
    // a shell writer's string, a comment's, `runCommand`'s after its set
    has(
        &l,
        "exit",
        "meta.function-call.shell support.function.exit.shell",
    );
    has(&l, "ls", COMMAND);
    has(&l, "mkdir", COMMAND);
    // the writer's item ends with its parenthesis
    has(&l, "rm", COMMAND);
    has(&l, "chmod", "string.quoted.other.nix");
    // NixOS's `script` at a path's end; two scripts on a line
    has(&l, "sleep", COMMAND);
    has(&l, "touch", COMMAND);
    // `writeShellApplication`'s `text`
    has(&l, "grep", COMMAND);
    // a writer's script, not a string in its arguments' set
    has(&l, "<?php echo", "string.quoted.other.nix");
    has(&l, "ln", COMMAND);
    // a binding in a shell value's `let`: its string not the script
    has(&l, "set(x", "string.quoted.other.nix");
    has(&l, "chown", COMMAND);
}

#[test]
fn shell_state_across_lines() {
    let l = [
        "{",
        "  postInstall = ''",
        "    echo \"a",
        "    b\" done",
        "    make \\",
        "      FOO=1",
        "    ${lib.concatMapStrings (p: ''",
        "      cp ${p} $out",
        "    '') ps}",
        "  '' + lib.optionalString stdenv.isLinux ''",
        "    patchelf $out",
        "  '';",
        "  preFixup = ''",
        "    args+=(",
        "      \"a\"",
        "  ''",
        "  + ''",
        "    )",
        "    wrap $out",
        "  '';",
        "  meta.longDescription = ''x'';",
        "}",
    ];
    has(&l, "b", "string.quoted.double.shell");
    has(&l, "\"", "punctuation.definition.string.end.shell");
    has(&l, "\\", "punctuation.separator.continuation.line.shell");
    lacks(&l, "FOO", COMMAND);
    has(&l, "cp", COMMAND);
    has(&l, "patchelf", COMMAND);
    // strings joined by `+`, one script
    has(&l, ")", "punctuation.section.parens.end.shell");
    has(&l, "wrap", COMMAND);
    has(&l, "x", "string.quoted.other.nix");
}

#[test]
fn regexes_in_match_and_split() {
    let or = "string.quoted.double.nix keyword.operator.or.regexp";
    let l = [r#"m = builtins.match "([a-z]+)-(.*)\\.nix$" name;"#];
    has(
        &l,
        "(",
        "string.quoted.double.nix punctuation.definition.group.begin.regexp",
    );
    has(
        &l,
        "+",
        "string.quoted.double.nix keyword.operator.quantifier.regexp",
    );
    has(&l, "-", "string.quoted.double.nix");
    has(
        &l,
        r"\\.",
        "string.quoted.double.nix constant.character.escape.backslash.regexp",
    );
    has(
        &l,
        "$",
        "string.quoted.double.nix keyword.control.anchor.regexp",
    );
    has(&l, "name", "variable.parameter.name.nix");
    // a POSIX class, and the regex going on past an interpolation
    let l = [r#"s = lib.split "[[:space:]]+${sep}(a|b)" str;"#];
    has(
        &l,
        "[:space:]",
        "string.quoted.double.nix constant.other.character-class.set.regexp \
         constant.other.character-class.posix.regexp",
    );
    has(&l, "sep", "variable.parameter.name.nix");
    has(&l, "|", or);
    // across lines, and `match` under `with builtins;`
    has(&["v = with builtins; match \"(a", "|b)\" s;"], "|", or);
    // Nix's escape the string's
    has(
        &[r#"m = match "a\"b" s;"#],
        r#"\""#,
        "string.quoted.double.nix constant.character.escape.nix",
    );
    // no string after it, or another name
    lacks(&["u = match (f \"(a|b)\") y;"], "|", or);
    lacks(&["w = matches \"(a|b)\";"], "|", or);
}

#[test]
fn a_shell_block_lexes_as_its_lines() {
    // bash's levels past the 9 kept, a here-document, a spliced string across lines, a regex's
    // group across an interpolation and lines
    let text = "{ postPatch = ''\n  echo \"$(echo \"$(echo \"$(echo \"$(echo \"$(echo \"$(\n  \
                x)\")\")\")\")\")\"\n  cat <<A\n  ${f ''\n    y\n  ''}\n  A\n''; m = \
                builtins.match \"(a\n|${b}\n)\" c; }\n";
    let one = lines(text);
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
    // nothing kept past the strings
    assert_eq!(state, State::default());
}

#[test]
fn long_shell_lines_lex_in_linear_time() {
    for (head, unit) in [
        ("{ postPatch = ''", "${a} "),
        ("{ postPatch = ''", "${f ''x''} "),
        ("{ postPatch = ''", "''${a} "),
        ("{ postPatch = ''", "\"$("),
        ("{ postPatch = ''${f ''", "${g \""),
        ("m = builtins.match \"", "(a|${b})"),
        ("m = builtins.match \"", "\\\\"),
    ] {
        let l = String::from(head) + &unit.repeat(20_000);
        let t = std::time::Instant::now();
        lines(&l);
        assert!(t.elapsed().as_millis() < 300, "{unit:?}: {:?}", t.elapsed());
    }
}

#[test]
fn marks_past_the_depth_counter() {
    // a shell value's mark, a hold and a string past 255 open levels, where the depth saturates
    let deep = "{ ".repeat(300);
    for l in [
        format!("{deep}postPatch = {{ a = ''x''; }}; ''y''"),
        format!("{deep}x = writeShellScript \"a\" ''ls''; m = match \"(a\" b;"),
        format!("{deep}postPatch = {{ a =\n''x'';\n''y''\n}};"),
    ] {
        let one = lines(&l);
        let mut state = State::default();
        let mut all = Vec::new();
        let mut v = Vec::new();
        block(&mut state, &mut v, &mut |c| {
            for t in l.lines() {
                all.push(c.next(t.as_bytes()).to_vec());
            }
            None
        });
        assert_eq!(all, one.iter().map(|(s, _)| s.clone()).collect::<Vec<_>>());
        assert_eq!(state, one.last().unwrap().1);
    }
}
