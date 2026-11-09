// `cargo xtask names` writes this file from two-face's syntax set: its first-line regexes,
// in its order, for the `regex` crate (ADR-43). Don't edit it by hand.
[
    ("AppleScript", r#"^#!.*(osascript)"#),
    ("C++", r#"-\*- C\+\+ -\*-"#),
    ("C", r#"-[*]-( Mode:)? C -[*]-"#),
    ("D", r#"^#!.*\bg?dmd\b."#),
    ("Diff", r#"(?x)^
    (===\ modified\ file
    |==== \s* // .+ \s - \s .+ \s+ ====
    |Index:[\x20]
    |---\ [^%]
    |\*\*\*.*\d{4}\s*$
    |\d+(,\d+)* (a|d|c) \d+(,\d+)* $
    |diff\ --git[\x20]
    )"#),
    ("Erlang", r#"(?x:
  ^ \#! .* \b(erlang|escript)\b |                # shebang
  ^ \s* \%+ \s* -\*- .*? \b[Ee]rlang\b .*? -\*-  # editorconfig
)"#),
    ("Git Config", r#"^\[core\]"#),
    ("Git Link", r#"^\s*gitdir\s*:"#),
    ("Git Log", r#"^commit\s+[0-9A-Fa-f]{7,}"#),
    ("Git Rebase Todo", r#"^(?:drop|edit|exec|fixup|pick|reword|squash|[defprsx]) [0-9A-Fa-f]{7,} "#),
    ("HTML", r#"(?i)<(!DOCTYPE\s*)?html"#),
    ("JavaScript", r#"^#!\s*/.*\b(node|bun|js)\b"#),
    ("LaTeX Log", r#"This is (pdf|pdfe)?TeXk?, Version "#),
    ("Lisp", r#"(?xi:
  ^ \s* ; .*? -\*- .*? \blisp\b .*? -\*-  # editorconfig
)"#),
    ("Makefile", r#"(?xi:
  ^\#! .* \bmake\b |                     # shebang
  ^\# \s* -\*- [^*]* makefile [^*]* -\*- # editorconfig
)"#),
    ("MultiMarkdown", r#"(?i)^format:\s*complete\s*$"#),
    ("PHP", r#"^(#!.*[^-]php[0-9]?|<\?php)\b"#),
    ("Perl", r#"(?xi:
  ^\#! .* \bperl\b |                     # shebang
  ^\# \s* -\*- [^*]* perl [^*]* -\*-     # editorconfig
)"#),
    ("Python", r#"^#!\s*/.*\bpython(\d(\.\d)?)?\b"#),
    ("Ruby", r#"(?xi:
  ^\#! .* \bj?ruby\b |                # shebang
  ^\# \s* -\*- [^*]* ruby [^*]* -\*-  # editorconfig
)"#),
    ("Bourne Again Shell (bash)", r#"(?x)
  ^\#! .* \b(bash|zsh|sh|tcsh|ash|dash|ksh)\b
| ^\# \s* -\*- [^*]* mode: \s* shell-script [^*]* -\*-
"#),
    ("Textile", r#"textile"#),
    ("XML", r#"(?x:
  ^(?:
      <\?xml\s
    | \s*<([\w-]+):Envelope\s+xmlns:\1\s*=\s*"http://schemas.xmlsoap.org/soap/envelope/"\s*>
    | \s*(?i:<!DOCTYPE\s+(?!html[\x20\t\n\f>]))
  )
)"#),
    ("YAML", r#"^%YAML( ?1.\d+)?"#),
    ("CMakeCache", r#"# This is the CMakeCache file."#),
    ("CoffeeScript", r#"^#!.*\bcoffee"#),
    ("Crystal", r#"^#!/.*\bcrystal"#),
    ("Dockerfile", r#"^\s*(?i:(from(?!\s+\S+\s+import)|arg))\s+"#),
    ("Elixir", r#"^#!/.*\b(?:elixirc?|iex)"#),
    ("Fish", r#"^#!.*\b(fish)\b"#),
    ("GLSL", r#"-[*]-( Mode:)? GLSL -[*]-"#),
    ("JavaScript (Babel)", r#"^#!\s*/.*\b(node|js)$\n?"#),
    ("HTML (Jinja2)", r#"^\{% extends ["'][^"']+["'] %}"#),
    ("Julia", r#"^#!.*\bjulia\s*$"#),
    ("LiveScript", r#"^#!.*\bls"#),
    ("Protocol Buffer", r#"^(syntax)\s*(=)\s*("proto\d")\s*(;)\s*$"#),
    ("Requirements.txt", r#"(?xi:
  ^ \#! .* \bpip                                        # shebang
| ^ \s* \# .*? -\*- .*? \bpip-requirements\b .*? -\*-   # editorconfig
| ^ \s* \# (vim?|ex): .*? \brequirements\b              # modeline
)"#),
    ("JQ", r#"(?xi:
  ^ \#! .* \b(jq|gojq|jaq|yq|xq)\b  # shebang
)"#),
    ("Svelte", r#"(?i)<(!DOCTYPE\s*)?html"#),
    ("Swift", r#"^#!/.*\bswift"#),
    ("HTTP Request and Response", r#"(?x:
  ^
  (?:
    (?:GET|HEAD|POST|PUT|DELETE|CONNECT|OPTIONS|TRACE|PATCH)\ .+\ HTTP/\d(?:\.\d)?$
    |
    (?:HTTP/\d(?:\.\d)?\ \d{3}\ )
  )
)"#),
    ("Private Key", r#"^-----BEGIN [\w\x20]+ PRIVATE KEY-----"#),
    ("Odin", r#"-[*]-( Mode:)? Odin -[*]-"#),
]
