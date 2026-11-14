# ada-parse

The syntax highlighting lexers of [ada](https://github.com/junkdog/ada-md), the terminal
markdown viewer: one pass over a line's bytes, no regex engine, no grammar files to load, and no
dependencies.

Each lexer splits a line into spans of the language's kinds, and each kind carries the scopes
[bat](https://github.com/sharkdp/bat)'s grammars give the same text (`keyword.control.rust`,
`string.quoted.double.json`), so that themes written for bat, Sublime Text or TextMate apply
as they are.

```rust
use ada_parse::{Lang, State, lex};

let lang = Lang::from_token("rust").unwrap(); // or from_file_name, from_first_line
let mut state = State::default(); // what a line leaves the next: open strings, comments
let mut spans = Vec::new();
for line in "fn main() {\n    println!(\"hi\"); // greet\n}".lines() {
    lex(lang, line, &mut state, &mut spans);
    for span in &spans {
        let text = &line[span.start as usize..span.end as usize];
        // the scopes under the language's base scope, outermost first
        println!("{text:?}: {} {}", lang.base(), lang.scopes(span.kind).join(" "));
    }
}
```

Bytes no span covers are plain text. `State` is small and `Copy`: keep one per line and any
line can be lexed again from the state before it. For the languages whose
`Lang::line_by_line()` is true, every line starts from `State::default()`, in any order, on any
thread.

## Examples

Three small programs in
[`ada-parse/examples`](https://github.com/junkdog/ada-md/tree/main/ada-parse/examples), std only:

- **`spans`:** each line's spans, with their text and scopes.
- **`highlight`:** a `cat` with colors, its language from the file's name or first line and
  its colors from a theme of a dozen scope selectors, matched as TextMate themes match them.
- **`incremental`:** highlighting again after an edit, as an editor would: a `State` kept per
  line, and lexing that stops at the first line whose state didn't change.

```zsh
cargo run -p ada-parse --example spans -- json < package.json
cargo run -p ada-parse --example highlight -- ada-parse/src/lib.rs
cargo run -p ada-parse --example incremental
```

## Languages

- **Code:** bash, C, C++, C#, Clojure, CSS, SCSS, Elixir, Go, Groovy, Haskell (and literate),
  HTML, Java, JavaScript, Kotlin, Lisp (Common Lisp, Emacs Lisp, Scheme, Racket), Lua, Nix,
  Objective-C, Objective-C++, PHP, PowerShell, Python, Ruby, Rust, Scala, SQL, Swift,
  TypeScript, TSX.
- **Config and data:** CMake (and its cache, and `configure_file` templates), crontab, diff,
  Dockerfile, `.env`, fstab, Git's own files (ignore, attributes, mailmap, link), INI, Java
  properties, JSON, logs, Makefile, nginx, OpenSSH's `ssh_config`, `sshd_config` and
  `known_hosts`, pkg-config, Terraform, TOML, XML, YAML.

`Lang::ALL` lists them. Languages arrive in most releases.

Code inside code is lexed in its own language, its scopes inside the host's: SQL and regexes in
other languages' strings (each regex in its engine's dialect, `keyword.other.DML.sql` inside a
Python string's scopes), HTML's styles and scripts, PHP's HTML, a Dockerfile's `RUN` and Nix's
build phases as shell, nginx's Lua blocks.

## Performance

Each lexer is a hand-written scanner that makes one forward pass over a line's bytes: no regex
engine and no backtracking, its spans written into the caller's vector, reused from line to
line. `State` is 32 bytes and `Copy`, so keeping one per line to restart from costs next to
nothing.

Single-threaded, on an AMD Ryzen AI MAX+ PRO 395 (November 2026):

- **Code:** 370–790 MB/s, most languages between 450 and 730 (Scheme and Racket are the
  slowest, PHP the fastest).
- **Config and data:** 0.45–7.7 GB/s, most above 0.8 (JSON 1.4, TOML 1.2, YAML 0.9, diff
  7.7); Makefile, mostly shell, is the slowest.
- **Against syntect** on bat's grammars, which ada used before: 100–400× faster for most
  languages, from 19× (Racket) up.
- **Against other engines:** faster than Lexilla (Scintilla's lexers) and KSyntaxHighlighting
  (Kate's) on each language compared (JSON, Rust, Python, bash, Java), and within 16% of a
  token-only [logos](https://crates.io/crates/logos) lexer on JSON and Rust, which gives
  tokens where ada-parse gives scopes.

Code size is the trade: every language is compiled in, about 7 MB of a release binary for all
of them (x86_64, fat LTO, stripped).

## Stability

ada-parse is versioned with ada and released with it. Its API is `lex`, `Span`, `State` and
`Lang` (its lookups, `name`, `base`, `scopes` and `line_by_line`), and it follows semver: a
breaking change waits for ada 2.0. New `Lang` variants are not breaking (`#[non_exhaustive]`).

What the lexers give is behavior, not API, and changes in any release: a kind's number and
scopes, where spans start and end, and which names, extensions and first lines the lookups
know. Read a kind only through `Lang::scopes`, in the same version. Items hidden from the docs
are ada's, and change without notice.

MSRV 1.95. MIT licensed.
