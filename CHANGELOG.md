# Changelog

What changed in each release of ada, newest first. A release includes the changes of the pre-releases before it.

## Unreleased

### Features

- *(completions)* Add PowerShell completions
- *(windows)* Live reload through ReadDirectoryChangesW instead of the 1 s poll
- *(windows)* Probe the terminal for image support through the console input
- *(windows)* Read the config from %APPDATA%, open links with ShellExecuteW, fall back to notepad

### Performance

- *(highlight)* Highlight shell scripts with the port of bat's bash grammar instead of syntect
- *(highlight)* Highlight Makefiles with a port of bat's grammar instead of syntect

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
