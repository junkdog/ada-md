# Changelog

What changed in each release of ada, newest first. A release includes the changes of the pre-releases before it.

## Unreleased

### Features

- *(ada-parse)* Highlight Lisp, Emacs Lisp, Scheme, Racket and Clojure with scanners of ada's own (X-006)
- *(ada-parse)* Highlight SQL in Java's and Kotlin's strings by call site and marker, and Java's text blocks (ADR-40)
- *(ada-parse)* Highlight regexes in JavaScript's and TypeScript's RegExp strings, and TypeScript's literals on the regex scanner (ADR-40)
- *(ada-parse)* Highlight regexes in C#'s and PowerShell's strings, in .NET's dialect (ADR-40)
- *(ada-parse)* Highlight JavaScript's regex literals as regexes, and a class's characters in Kotlin's and Java's (ADR-40)
- *(ada-parse)* Highlight regexes in Kotlin's and Java's strings, as IntelliJ does (ADR-40)
- *(ada-parse)* Add c language lexer and scanning rules
- *(cli)* Name the commit, build time and symbol order in --version
- *(completions)* Add PowerShell completions
- *(images)* Show a gif's first frame, inline, full screen, in --print and piped in
- *(print)* Mark the --line row, --match's matches and the git gutter (--gutter)
- *(reload)* Mark a changed file until r with live_reload = "notify"
- *(search)* Walk the session's searches with ↑ ↓ in the prompt, and move a cursor in every input field
- *(tables)* Keep words whole before breaking after - and /, and cut long words at . _ : or camelCase
- *(windows)* Live reload through ReadDirectoryChangesW instead of the 1 s poll
- *(windows)* Probe the terminal for image support through the console input
- *(windows)* Read the config from %APPDATA%, open links with ShellExecuteW, fall back to notepad
- Highlight TypeScript and TSX with ada's own scanner, exact on all but compiler tests (ADR-40)

### Fixes

- *(ada-parse)* Highlight a long line of Java's `a.b.c` chains in linear time, not quadratic (X-003)
- *(ci)* Name the gnu target for cargo fuzz build, as the prebuilt cargo-fuzz defaults to musl
- *(images)* Refuse a JPEG too short for the size its header claims
- Refuse documents of 4 GiB or more instead of wrapping their u32 byte offsets (PAR-001)
- End HTML's script at </script or the attribute's quote inside a regex character class (X-004)
- Keep SCSS's spans in order when a namespaced call meets the nesting limit
- Require Rust 1.95, and build the examples on macOS and Windows
- Highlight TypeScript as bat's grammar does where scrambled input showed it differing (ADR-40)

### Performance

- *(ada-parse)* Highlight Java 20% faster in ada by classifying tokens once in front of the grammar's patterns (X-005)
- *(highlight)* Highlight shell scripts with the port of bat's bash grammar instead of syntect
- *(highlight)* Highlight Makefiles with a port of bat's grammar instead of syntect
- *(parse)* Scan for markup 16 bytes at a time on x86_64 (PAR-005)
- *(parse)* Find line starts with AVX2, count code lines with memchr, hash with foldhash (PAR-003)
- *(parse)* Find each event's lines with a cursor, not two binary searches (PAR-002)
- *(print)* Write the SGR's first parameter without its separator, instead of removing it
- Keep each document's runs and each table's cells in one Vec, the Doc's heap −4 to −67% and tables' print peak 94 → 65 MiB (PAR-008)
- Print big files in ada-parse's languages a piece at a time, 6–7× less peak memory (PRN-003)
- Highlight .env and Git's ignore, attributes and mailmap files 2.6–4.1× faster with scanners of ada's own
- Highlight ada's own languages 30–124% faster in ada by trimming the per-line path around the lexers (HL-004)
- Highlight HTML with a scanner of ada's own, its CSS and JavaScript on theirs, 31× faster than syntect in ada (ADR-40)
- Highlight SCSS 3.4× faster alone and twice as fast in ada, with the same output (ADR-40)
- Highlight SCSS on CSS's scanner, exact against bat's grammar in all 14 themes and about 60× faster than syntect (ADR-40)
- Highlight CSS with a scanner of ada's own, exact against bat's grammar in all 14 themes and 37× faster in ada (ADR-40)
- Highlight JavaScript about 40% faster, with the same output (ADR-40)
- Highlight TypeScript about 30% faster and TSX about 25% faster, with the same output (ADR-40)
- Highlight TypeScript and TSX about 75% faster, with the same output (ADR-40)
- Highlight PowerShell with a scanner of ada's own, exact against bat's grammar in all 14 themes and 52× faster in ada (ADR-40)
- Highlight SQL with a scanner of ada's own, exact against bat's grammar in all 14 themes and 66× faster in ada (ADR-40)
- Highlight Makefiles on bash's scanner, exact against bat's grammar in all 14 themes and 4× faster than the port (ADR-40)
- Highlight bash with a scanner of ada's own, exact against bat's grammar in all 14 themes and 10× faster than the port (ADR-40)
- Highlight JavaScript about 30% faster, with the same output (ADR-40)
- Highlight JavaScript with a scanner compiled from Babel's grammar for every JavaScript token, exact but for regex bodies (ADR-40)
- Highlight Lua with a scanner compiled from bat's grammar, exact in all 14 themes (ADR-40)
- Highlight Python with a scanner compiled from bat's grammar, the SQL and regexes in its strings included, exact in all 14 themes (ADR-40)
- Highlight Go with a scanner compiled from bat's grammar, exact in all 14 themes (ADR-40)
- Highlight C# with a scanner compiled from bat's grammar, exact in all 14 themes (ADR-40)
- Highlight C++ about half again as fast, with the same output (ADR-40)
- Highlight C++ with a scanner compiled from bat's grammar, the C it reaches included, exact in all 14 themes (ADR-40)
- Highlight Java with a scanner compiled from bat's grammar, Javadoc and its HTML included, exact in all 14 themes (ADR-40)
- Highlight Kotlin with a scanner compiled from bat's grammar, exact in all 14 themes (ADR-40)
- Highlight Rust with a scanner compiled from bat's grammar, exact in all 14 themes (ADR-40)
- Highlight XML with a scanner compiled from bat's grammar, exact against its port (ADR-40)
- Highlight YAML and TOML with ada-parse's scanners, JSON and logs moved into the crate (ADR-40)
- Highlight TOML, XML, YAML, Dockerfile, properties, .env and Git files on ada-parse's port of syntect's machine (ADR-40)
- Highlight diffs and INI files with ada-parse's own lexers (ADR-40)
- Name JSON, log, Makefile and bash without loading syntect's set (ADR-40)
- Flatten a table's cells once and draw its borders unmeasured (LAY-003, LAY-004)
- Hold the runs' text as ranges of one buffer a document, not a box a run (PAR-004)
- Hold a code block's highlight runs end to end, not boxed a line each (HL-002)
- Hold a document's layout in less memory, 2.5–2.9 MiB less on a 2 MB note (MEM-010)

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

---

*Generated from the commit messages by [git-cliff](https://git-cliff.org) ❤️*
