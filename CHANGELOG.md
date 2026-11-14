# Changelog

The important changes in each release of ada, newest first. A release includes the changes of the pre-releases
before it.

## [1.1.0](https://github.com/junkdog/ada-md/compare/v1.0.0..v1.1.0) (unreleased)

ada 1.1 runs on Windows, and highlights code with lexers of its own: 61 languages on ada-parse's scanners, for most
of them 100–400× faster than syntect on bat's grammars, which is out of the build. ada-parse is published on
crates.io as a library, in step with ada's version.

### Highlighting

- 61 languages on scanners of ada's own: the common languages (Rust, Python, Go, Java, Kotlin, Scala, C, C++, C#,
  Objective-C, Swift, JavaScript, TypeScript, PHP, Ruby, Elixir, Haskell, the Lisps and more) and the config
  formats (CMake, Terraform, Nix, nginx, OpenSSH's files, crontab and more).
- SQL and regexes inside other languages' strings, in Python, Java, Kotlin, Scala, Groovy, C#, C, C++, JavaScript,
  TypeScript, Ruby, Swift, Elixir and others; each regex in its engine's dialect.
- Code inside code in its own colors: HTML's styles and scripts, PHP's and HEEx's HTML, a Dockerfile's `RUN` and
  Nix's phases as shell, nginx's Lua blocks.
- Piped input is told apart by every language ada lexes.
- JSON keys are colored apart from string values.
- Fences take `csharp`, `pwsh` and `posh` as language names.

### Features

- **Windows:** an x86_64 binary, the first release with one. The config is read from `%APPDATA%\ada`, links open
  with the system's handler, `e` falls back to notepad, live reload watches the directory, and the image probe runs
  through the console. PowerShell completions.
- A gif shows its first frame: inline, full screen, in `--print` and piped in.
- `--print` marks the `--line` row, `--match`'s matches and the git gutter (`--gutter`).
- `live_reload = "notify"` marks a changed file until `r` instead of reloading it.
- The search prompt walks the session's searches with ↑ ↓, and every input field has a movable cursor.
- Tables keep words whole, breaking after `-` and `/`, and cut long words at `.`, `_`, `:` or camelCase; a long
  header wraps before the rows do when that saves body lines.
- `--version` names the commit, the build time and whether the hot code was ordered.

### Performance

- Big code files print a piece at a time: 6–7× less peak memory (4–5 MB files: 50–57 → 7.7–9.0 MiB).
- Markdown parses, lays out and prints faster: markup and line starts found by SIMD scans, words read eight bytes at
  a time, lines filled from SSE2 masks, styles written in fewer stores.
- Less memory for a document: its heap −4 to −67%, a table-heavy print's peak 94 → 65 MiB.
- The TUI sends kitty images through shared memory.

### Changes

- A language with no lexer of ada's own is still recognized but shown plain; 1.0 colored all of bat's languages
  through syntect.
- Code files print bare in `--print`: on the terminal's background, without the margin, the indent or a language
  label.
- Building from source needs Rust 1.95 (1.88 for 1.0).

### Fixes

- RUSTSEC-2026-0433: nested image wikilinks took quadratic time to parse (pulldown-cmark 0.13.6).
- A document of 4 GiB or more is refused instead of being misread.
- A JPEG too short for the size its header claims is refused.
- A file git ignores shows no git gutter.
- The prompt's cursor is a block over its character, not a bar that read as a space.

## [1.0.0](https://github.com/junkdog/ada-md/compare/v1.0.0-rc.1..v1.0.0) (2026-10-27)

### Features

- *(cli)* Add --code-wrap on|off, and let fzf previews cut code at the pane's edge
- *(diagram)* Draw mermaid class diagrams as text, fitted to the width and styled by the theme
- *(filetype)* Draw .mmd/.mermaid files and piped mermaid diagrams
- *(release)* Ship static musl binaries for x86_64 and aarch64 Linux
- *(ui)* Tint where a jump landed until the next key

### Fixes

- *(app)* List a link wrapped inside a word without a space
- *(nav)* Treat rows under a sticky table header as off screen when focusing links
- *(order)* Read the traced build's link map from its target directory
- *(order)* Read link maps as bytes and measure peak RSS through time
- *(order)* Skip AArch64 mapping symbols in lld maps
- *(release)* Compile aarch64 musl atomics inline for jemalloc
- *(ui)* Keep the outline popup's size from wrapping past 65,535 headings or columns
- Hold documents past 65,535 columns or entries, and nesting 10,000 deep.

### Performance

- *(grid)* Compare plain rows as bytes and give styles the cell's color encoding
- *(grid)* Erase blanks on the theme's background instead of writing spaces

## 1.0.0-rc.1 (2026-10-23)

The first release: its [release notes](https://github.com/junkdog/ada-md/releases/tag/v1.0.0-rc.1) list the features
it shipped with.
