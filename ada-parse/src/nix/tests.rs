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
fn shell_in_double_quoted_strings() {
    let l = [
        "{",
        "  postInstall = \"wrapProgram $out/bin/a --set X ${x}\";",
        "  preBuild = lib.optionalString (system == \"x86_64-linux\") \"export A=1\\nmake\";",
        "  b = runCommand \"b\" {} \"mkdir $out\";",
        "  c = writeShellScriptBin \"c\" \"rm \\\"$1\\\"\";",
        "  d = /* sh */ \"ls\";",
        "  postFixup = lib.concatMapStringsSep \"\\n\" (x: \"ln ${x}\") xs;",
        "  postPatch = \"",
        "    substituteInPlace a \\\\",
        "      --replace b c",
        "  \" + ''",
        "    chmod a",
        "  '';",
        "  description = \"echo words\";",
        "  script = writeShellScript \"named\" ''pwd'';",
        "  preHook = if system == \"x86\" then a else \"cd\";",
        "}",
    ];
    has(
        &l,
        "\"",
        "string.quoted.double.nix punctuation.definition.string.double.start.nix",
    );
    // a writer's name under a binding's mark; a comparison's operand
    has(&l, "named", "string.quoted.double.nix");
    has(&l, "x86", "string.quoted.double.nix");
    has(
        &l,
        "cd",
        "meta.function-call.shell support.function.cd.shell",
    );
    // a writer's mark ended with a formal's default
    has(
        &["{ runCommand ? pkgs.runCommand, tag ? \"latest\" }: x"],
        "latest",
        "string.quoted.double.nix",
    );
    has(&l, "wrapProgram", COMMAND);
    has(&l, "out", "variable.other.readwrite.shell");
    // an argument deeper than the value's level: a condition's string
    has(&l, "x86_64-linux", "string.quoted.double.nix");
    // Nix's escape Nix's, a line break to bash
    has(
        &l,
        "\\n",
        "string.quoted.double.nix constant.character.escape.nix",
    );
    has(&l, "make", COMMAND);
    // a writer's name, its first argument, a string; its script shell
    has(&l, "b", "string.quoted.double.nix");
    has(&l, "mkdir", COMMAND);
    has(&l, "c", "string.quoted.double.nix");
    has(&l, "rm", COMMAND);
    // `\"` Nix's escape, bash's quote
    has(&l, "1", "variable.other.readwrite.shell");
    has(&l, "ls", COMMAND);
    // a separator and a lambda's string in parentheses: arguments
    has(&l, "ln ", "string.quoted.double.nix");
    // across lines, a `\\` bash's continuation, joined to a `''…''` string
    has(&l, "substituteInPlace", COMMAND);
    lacks(&l, "--replace", COMMAND);
    has(&l, "chmod", COMMAND);
    has(&l, "echo words", "string.quoted.double.nix");
}

#[test]
fn spliced_strings_and_writers() {
    let l = [
        "{",
        "  postInstall = ''",
        "    ${lib.optionalString x ''",
        "      cat > a <<EOF",
        "        EOF",
        "      EOF",
        "      touch b",
        "    ''}",
        "    cat > c <<B",
        "      body",
        "    B",
        "    cp ${writeText \"a\" ''",
        "      not shell",
        "    ''} $out/a",
        "    ln -s ${pkgs.writeText \"b\" ''echo x''} $out/b",
        "    install ${builtins.toFile \"c\" ''rm y''} $out",
        "    ${lib.optionalString c ''chown z''}",
        "    ${lib.optionalString writeHieFiles ''chgrp h''}",
        "  '';",
        "  preConfig = ''local x = 1'';",
        "  preCheckConfig = ''mkdir d'';",
        "  postFailureMessage = ''see the log'';",
        "  mkSetupHook = rc: mkDerivation { setupHook = writeText \"h.sh\" ''source ${rc}''; };",
        "  preBuild = let script = toFile \"a.py\" ''import os''; in ''python ${script}'';",
        "  langs = [ /* bash */ ''ls'' /* ruby */ ''puts 1'' ];",
        "}",
    ];
    // a flag, not a writer: no name after it
    has(&l, "chgrp", COMMAND);
    // a writer's file named as a shell script's, a script's binding below a mark that isn't
    // one; and another file in its language (X-153), the value's script going on after its
    // binding
    has(&l, "import", "source.python keyword.control.import.python");
    has(&l, "python", COMMAND);
    has(
        &l,
        "source",
        "meta.function-call.shell support.function.source.shell",
    );
    // a comment naming another language
    has(&l, "puts 1", "string.quoted.other.nix");
    // a spliced string's own indentation stripped: a closer deeper than it is the body's
    has(&l, "  EOF", "string.unquoted.heredoc.shell");
    has(&l, "touch", COMMAND);
    // the script's indentation learned again after it
    has(&l, "  body", "string.unquoted.heredoc.shell");
    has(&l, "B", "keyword.control.heredoc-token.shell");
    // a writer's string that isn't shell, in the script's interpolation
    has(&l, "      not shell", "string.quoted.other.nix");
    has(&l, "cp", COMMAND);
    has(&l, "echo x", "string.quoted.other.nix");
    has(&l, "ln", COMMAND);
    has(&l, "rm y", "string.quoted.other.nix");
    has(&l, "install", COMMAND);
    // the hold ended with its interpolation
    has(&l, "chown", COMMAND);
    // `pre*` and `post*` that aren't scripts
    has(&l, "local x = 1", "string.quoted.other.nix");
    has(&l, "mkdir", COMMAND);
    has(&l, "see the log", "string.quoted.other.nix");
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
    // a `''…''` string: its text the regex's as it is, Nix's escapes Nix's, across lines
    let other = "string.quoted.other.nix keyword.operator.or.regexp";
    let l = [r"m = builtins.match ''([a-z]+)\.nix(a''$|${b}'' name;"];
    has(
        &l,
        "(",
        "string.quoted.other.nix punctuation.definition.group.begin.regexp",
    );
    has(
        &l,
        r"\.",
        "string.quoted.other.nix constant.character.escape.backslash.regexp",
    );
    has(
        &l,
        "''$",
        "string.quoted.other.nix constant.character.escape.nix",
    );
    has(&l, "|", other);
    has(&l, "b", "variable.parameter.name.nix");
    has(
        &l,
        "''",
        "string.quoted.other.nix punctuation.definition.string.other.end.nix",
    );
    has(&l, "name", "variable.parameter.name.nix");
    has(&["s = lib.split ''(a", "|b)'' t;"], "|", other);
}

#[test]
fn a_shell_block_lexes_as_its_lines() {
    // bash's levels past the 9 kept, a here-document, a spliced string across lines, a regex's
    // group across an interpolation and lines
    let text = "{ postPatch = ''\n  echo \"$(echo \"$(echo \"$(echo \"$(echo \"$(echo \"$(\n  \
                x)\")\")\")\")\")\"\n  cat <<A\n  ${f ''\n    y\n  ''}\n  A\n''; m = \
                builtins.match \"(a\n|${b}\n)\" c; postFixup = \"cat <<C\n  ${g ''\n    z\n  \
                ''}\nC\n\"; n = split ''(d\n|${e}\n)'' f; preBuild = ''${writeText \"h\" ''\n  \
                i\n''}''; }\n";
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
        ("{ postPatch = \"", "${a} "),
        ("{ postPatch = \"", "\\n\\\""),
        ("{ postPatch = ''", "${writeText \"a\" ''x''} "),
        ("{ postPatch = ''", "${f ''\n"),
        ("m = match ''", "(a|${b})"),
        ("m = match ''", "''$"),
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
        format!("{deep}postPatch = \"x\"; s = match ''(a'' b;"),
        format!("{deep}postPatch = ''${{writeText \"a\" ''b''}}\n${{f ''\nc''}}'';"),
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

/// Python's `import`.
const PY_IMPORT: &str = "source.python keyword.control.import.python";

#[test]
fn writers_files_in_their_language() {
    let l = [
        "{",
        "  a = writeText \"a.json\" ''",
        "    { \"k\": ${toJSON v}, \"n\": [1, 2] }",
        "  '';",
        "  b = pkgs.writers.writePython3Bin \"b\" {",
        "    libraries = [ ];",
        "  } ''",
        "    import os",
        "  '';",
        "  c = builtins.toFile \"c.toml\" \"[t]\\nk = 1\";",
        "  d = writeText \"d.sql\" ''SELECT 1 FROM t'';",
        "  e = writeScript \"e.sh\" ''",
        "    ls -l",
        "  '';",
        "  f = writeText \"f.conf\" ''x = 1'';",
        "  g = writeText name ''plain'';",
        "  h = pkgs.writeTextDir \"share/h.yaml\" ''",
        "    top:",
        "      key: 1",
        "  '';",
        "  i = writeText \"i.json\"; j = ''also plain'';",
        "  k = writeLua \"k\" { } ''local m = 1'';",
        "}",
    ];
    has(
        &l,
        "k",
        "source.json meta.mapping.key.json string.quoted.double.json",
    );
    has(&l, "toJSON", "variable.parameter.name.nix");
    has(&l, "2", "source.json constant.numeric.integer.decimal.json");
    // the writer's name and its set no file's
    has(&l, "b", "string.quoted.double.nix");
    has(&l, "libraries", "entity.other.attribute-name.multipart.nix");
    has(&l, "import", PY_IMPORT);
    // a `"…"` string's `\n` a line break to its language
    has(
        &l,
        "k",
        "source.toml meta.tag.key.toml entity.name.tag.toml",
    );
    has(&l, "SELECT", "source.sql keyword.other.DML.sql");
    has(&l, "ls", COMMAND);
    // `.conf` names none of a writer's (bat's nginx), a name without an extension none
    has(&l, "x = 1", "string.quoted.other.nix");
    has(&l, "plain", "string.quoted.other.nix");
    // indentation stripped as Nix strips it
    has(
        &l,
        "top",
        "source.yaml string.unquoted.plain.out.yaml entity.name.tag.yaml",
    );
    has(
        &l,
        "key",
        "source.yaml string.unquoted.plain.out.yaml entity.name.tag.yaml",
    );
    // a mark ended with its item
    has(&l, "also plain", "string.quoted.other.nix");
    has(&l, "local", "source.lua storage.modifier.lua");
}

#[test]
fn files_and_scripts_apart() {
    let l = [
        "{",
        // a file in a script's interpolation: a string (the script's state kept)
        "  postInstall = ''",
        "    cp ${writeText \"a.json\" ''{ \"x\": 1 }''} $out",
        "  '';",
        // a script named as one, at a shell value's level: the value's (X-152)
        "  script = writeScript \"s.sh\" ''touch s'';",
        // in a file's interpolation, no script and no other file
        "  b = writeText \"b.py\" ''",
        "    import ${writeShellScript \"t\" ''rm t''} ${writeText \"c.json\" ''[1]''}",
        "  '';",
        // a file in a binding below a shell value's mark, the value's script after it
        "  preBuild = let f = toFile \"f.py\" ''import g''; in ''python ${f}'';",
        "}",
    ];
    has(&l, "{ \"x\": 1 }", "string.quoted.other.nix");
    has(&l, "cp", COMMAND);
    has(&l, "touch", COMMAND);
    has(&l, "rm t", "string.quoted.other.nix");
    has(&l, "[1]", "string.quoted.other.nix");
    has(&l, "import", PY_IMPORT);
    has(&l, "python", COMMAND);
    lacks(&l, "rm", COMMAND);
}

#[test]
fn a_file_whose_state_does_not_fit() {
    // Python sixteen brackets deep: its depth past the nibble kept, the string's text a string's
    // from the next line on
    let l = [
        "a = writeText \"a.py\" ''",
        "  x = ((((((((((((((((",
        "  1",
        "  ))))))))))))))))",
        "'';",
        "b = writeText \"b.py\" ''import c'';",
    ];
    has(
        &l,
        "((((((((((((((((",
        "source.python punctuation.section.group.begin.python",
    );
    has(&l, "  1", "string.quoted.other.nix");
    has(&l, "import", PY_IMPORT);
}

#[test]
fn a_file_block_lexes_as_its_lines() {
    let text = "{ a = writeText \"a.py\" ''\n  def f(\n    x,\n  ):\n    return \"${y}\"\n''; \
                b = toFile \"b.json\" \"[1,\\n2]\"; c = writeText \"c.py\" ''\n  ((((((((((((((((\n  \
                1\n''; d = writePython3 \"d\" {\n  e = 1;\n} ''\n  \"\"\"doc\n  ${f}\n  \"\"\"\n''; g = \
                writeText \"g.sql\"\n  ''SELECT\n  1''; }\n";
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
fn long_file_lines_lex_in_linear_time() {
    for (head, unit) in [
        ("a = writeText \"a.json\" ''", "[1, ${a}, "),
        ("a = writeText \"a.py\" ''", "f(\"${a}\") "),
        ("a = toFile \"a.toml\" \"", "k = 1\\n"),
        ("a = toFile \"a.php\" \"<?php ", "$a${b}; "),
    ] {
        let l = String::from(head) + &unit.repeat(20_000);
        let t = std::time::Instant::now();
        lines(&l);
        assert!(t.elapsed().as_millis() < 300, "{unit:?}: {:?}", t.elapsed());
    }
}

#[test]
fn file_kinds_fold_and_unfold() {
    for (i, f) in FILES.iter().enumerate() {
        let slot = i as u8 + 1;
        for kind in 0..u16::MAX {
            let Some(k) = fold(slot, kind) else {
                continue;
            };
            if f.lang == Lang::Bash {
                assert_eq!(k, SH_BASE + kind);
                continue;
            }
            assert!(
                k >= FILE_BASES[i] && k < FILE_BASES[i] + f.size,
                "{:?} {kind}",
                f.lang
            );
            let (lang, back) = unfold(k).unwrap();
            assert_eq!(lang, f.lang);
            // Python's and PHP's folds keep a kind's leaf and its wrap's or embed's own
            assert_eq!(fold(slot, back), Some(k), "{:?} {kind} {back}", f.lang);
            if !matches!(f.lang, Lang::Python | Lang::Php) {
                assert_eq!(back, kind);
            }
        }
    }
}

#[test]
fn files_named_by_their_shebang() {
    let l = [
        "{",
        "  a = writeScript \"update\" ''",
        "    #!${runtimeShell}",
        "    touch a",
        "  '';",
        "  b = writeScriptBin name ''",
        "    #!/usr/bin/env nix-shell",
        "    #!nix-shell -i python3 -p python3",
        "    import sys",
        "  '';",
        "  c = writeText \"notes\" ''",
        "    just text",
        "  '';",
        "  d = writeScript \"d\" ''#!${pkgs.python3}/bin/python3",
        "    import os",
        "  '';",
        "  e = toFile \"e\" \"#!/bin/sh\\nrm e\";",
        "}",
    ];
    has(&l, "touch", COMMAND);
    has(&l, "import", PY_IMPORT);
    has(&l, "    just text", "string.quoted.other.nix");
    lacks(&l, "import", "string.quoted.other.nix");
    has(&l, "rm", COMMAND);
}

#[test]
fn file_slots_name_their_languages() {
    for (slot, lang) in [
        (BASH_SLOT, Lang::Bash),
        (JSON_SLOT, Lang::Json),
        (TOML_SLOT, Lang::Toml),
        (INI_SLOT, Lang::Ini),
        (PROPS_SLOT, Lang::Properties),
        (DIFF_SLOT, Lang::Diff),
        (YAML_SLOT, Lang::Yaml),
        (SQL_SLOT, Lang::Sql),
        (LUA_SLOT, Lang::Lua),
        (CSS_SLOT, Lang::Css),
        (EL_SLOT, Lang::EmacsLisp),
        (SCM_SLOT, Lang::Scheme),
        (CLJ_SLOT, Lang::Clojure),
        (C_SLOT, Lang::C),
        (CPP_SLOT, Lang::Cpp),
        (HS_SLOT, Lang::Haskell),
        (RS_SLOT, Lang::Rust),
        (JS_SLOT, Lang::JavaScript),
        (XML_SLOT, Lang::Xml),
        (PHP_SLOT, Lang::Php),
        (PY_SLOT, Lang::Python),
    ] {
        assert_eq!(FILES[usize::from(slot) - 1].lang, lang);
    }
    assert!(FILES.len() < usize::from(SHEBANG));
}

#[test]
fn ext_slots_name_as_ada_does() {
    for ext in [
        "sh",
        "bash",
        "zsh",
        "ash",
        "json",
        "ipynb",
        "toml",
        "tml",
        "ini",
        "inf",
        "reg",
        "lng",
        "cfg",
        "desktop",
        "url",
        "properties",
        "diff",
        "patch",
        "yaml",
        "yml",
        "sql",
        "ddl",
        "dml",
        "lua",
        "css",
        "el",
        "scm",
        "ss",
        "sld",
        "clj",
        "cljc",
        "cljs",
        "edn",
        "c",
        "cpp",
        "cc",
        "cp",
        "cxx",
        "c++",
        "hh",
        "hpp",
        "hxx",
        "h++",
        "inl",
        "ipp",
        "hs",
        "rs",
        "js",
        "mjs",
        "jsx",
        "babel",
        "es6",
        "cjs",
        "xml",
        "xsd",
        "xslt",
        "tld",
        "dtml",
        "rng",
        "rss",
        "opml",
        "svg",
        "xaml",
        "php",
        "php3",
        "php4",
        "php5",
        "php7",
        "phps",
        "phpt",
        "phtml",
        "py",
        "py3",
        "pyw",
        "pyi",
        "pyx",
        "pxd",
        "pxi",
        "rpy",
        "cpy",
        "gyp",
        "gypi",
        "vpy",
        "bzl",
        "bazel",
        "PY",
    ] {
        let slot = ext_slot(ext.as_bytes());
        assert_ne!(slot, 0, "{ext}");
        assert_eq!(
            Lang::from_file_name(&format!("x.{ext}")),
            Some(FILES[usize::from(slot) - 1].lang),
            "{ext}"
        );
    }
    // a language ada names a file in that no writer's file is read in (nginx's `.conf`), or none
    for ext in ["conf", "txt", "nix", "rb", "h", "service"] {
        assert_eq!(ext_slot(ext.as_bytes()), 0, "{ext}");
    }
}

#[test]
fn a_deep_file_block_lexes_as_its_lines() {
    // a writer's file's mark deep in the frames, its interpolation past the 32 levels they keep
    let open = "{ ".repeat(30);
    let text = format!(
        "{open}a = writeText \"a.json\" ''\n  [1, ${{ {{ b = {{ c = {{\n  }}; }}; }} }}, 2]\n''; }}\n\
         {open}{{ d = writeText \"d.json\" ''\n  [3]\n''; }}\n"
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
fn file_states_pack_and_unpack() {
    for f in &FILES {
        // every kept byte set, and the depth where it is kept
        let mut bytes = [0u8; SH_HELD];
        for (i, (b, &p)) in bytes.iter_mut().zip(&f.keep).enumerate() {
            if p != NO {
                *b = i as u8 + 1;
            }
        }
        let depth = if f.deep { 15 } else { 0 };
        let s = unpack(f, &bytes, depth);
        assert_eq!(pack(f, &s), Some((bytes, depth)), "{:?}", f.lang);
        // a byte it doesn't keep, or a depth past it: none
        let mut t = s;
        t.withs[if f.keep.contains(&(W_ + 13)) { 0 } else { 13 }] = 1;
        assert_eq!(pack(f, &t), None, "{:?}", f.lang);
        let mut t = s;
        t.depth = if f.deep { 16 } else { 1 };
        assert_eq!(pack(f, &t), None, "{:?}", f.lang);
    }
}
