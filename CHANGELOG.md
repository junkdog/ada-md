# Changelog

The important changes in each release of ada, newest first. A release includes the changes of the pre-releases
before it.

## [1.2.0](https://github.com/junkdog/ada-md/compare/v1.1.0..v1.2.0) (unreleased)

ada 1.2 reads an Obsidian vault as Obsidian shows it, and pages git and man in place of delta and less: diffs and
`git blame` in their files' languages. 73 languages on ada-parse's lexers, up from 61.
<!-- 1.2 release, item 5: add "at 1.1's speed or better" to the lead only if the bar is measured as met. -->

### Obsidian vaults

- A note under a `.obsidian` folder reads in Obsidian's flavor: a single newline breaks the line, unless the vault
  turns on strict line breaks; `%%comments%%` are hidden, `#tags` are drawn as pills and a block's `^id` is hidden.
  `--flavor`, or `flavor` under `[markdown]`, reads every note one way, `commonmark` or `obsidian`, stdin too.
- Callouts: GitHub's five kinds and Obsidian's thirteen with their aliases, titles with markup, nesting; one that
  folds shows `▾` and starts open.
- Math in any note, `$…$` and `$$…$$`, typeset as Unicode text: `\mathbb{R}` as ℝ, scripts, fractions, display
  math centered with its rows aligned on `&`, matrices and cases drawn tall. Search finds it by what it shows and by
  its TeX. A price, $5 or $10, stays text.
- An embed alone in its paragraph, `![[note]]`, `![[note#Heading]]` or `![[note#^id]]`, draws that note, the
  heading's section or the block in place under its link, and reloads when that note changes.
- Front matter, YAML or TOML, shows as the note's properties: a dim row for each key, `tags` as tags, `[[links]]`
  to follow. `p` hides them, and `properties = "source"` shows the front matter as written. A `title` heads the
  outline of a note without an H1.
- Links find a note by the `aliases` in its front matter, a heading under another (`[[note#A#B]]`) and a block
  (`[[note#^id]]`); a markdown link's path is found from the vault's root too, as Obsidian's absolute link format
  writes it.
- `==highlights==` on the theme's highlight color, in any flavor; the task statuses of Obsidian's themes and the
  Tasks plugin (`[/]`, `[-]`, `[>]`, `[<]`, `[!]`, `[?]`, `[*]`) as task boxes of their own; `![[cat.png|200]]`
  caps the image at 200 pixels across; an Excalidraw drawing shows its text without the plugin's data.

### A pager for git and man

- `git config --global core.pager 'ada -F'`: diffs in their files' languages, context and combined diffs too,
  changed words picked out, `n` `N` between files and commits and `o` listing them. `git blame` in the file's
  language. `--color-only` is git's `interactive.diffFilter`.
- `-F` prints and exits when the text fits on the screen, as `less -F` does.
- `MANPAGER='ada -F'`: man pages highlighted from `man`'s own output, C in their synopses, and `o` lists their
  sections.
- Piped text in its own colors keeps them (`ls -l --color=always | ada`), but a diff, which ada highlights itself;
  `-l ansi` keeps any input's.

### Highlighting

- Dart, with SQL and regexes in its strings.
- Perl, its regexes in Perl's dialect and its here-documents' code in their languages.
- Assembly: x86 and x86-64, ARM, RISC-V, MIPS, Z80, 6502 and 68k.
- Git's config files, and markdown as source, each fence's code in its language: a note in a diff or `git blame`.

### Changes

- In an Obsidian vault, single line breaks, `%%comments%%`, tags and block ids follow Obsidian; `--flavor
  commonmark` reads a note as 1.1 did.
- Front matter shows as property rows instead of a code block; `properties = "source"` shows it as 1.1 did.
- A theme from before 1.2 gets highlights (reversed) and tags (its inline code's style) derived.
- A code file in the viewer drops the code block's label and closing rows, as `--print` did in 1.1.

### Fixes

- `e` in vim, neovim, gvim, mvim and `view` opens with ada's top line at the top of the window, not partway down.
- A cell both bold and dim shows bold, in every frame, on a terminal with one intensity (WezTerm).
<!-- 1.2 release, item 1: Dependabot's #9 (unicode-width 0.2.3, Unicode 18's widths), if users see it change. -->

<!-- 1.2 release, item 5: a Performance section from the bar's measurements against v1.1.0, if there's a win to state. -->

## [1.1.0](https://github.com/junkdog/ada-md/compare/v1.0.0..v1.1.0) (2026-11-18)

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
- On Linux, the TUI sends kitty images through shared memory when the terminal supports it.

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
- `\n` in a mermaid label breaks the line, as `<br>` does, instead of showing as written; a space in a gantt name.
- A wrapped mermaid label carries at least three characters of a word onto the next line, so `getFoo()` no longer
  leaves its `)` alone on a row.
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
