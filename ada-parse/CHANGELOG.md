# Changelog

The important changes in each release of ada-parse, newest first. ada-parse is versioned with
[ada](https://github.com/junkdog/ada-md) and released with it; ada's own changes are in its
[changelog](https://github.com/junkdog/ada-md/blob/main/CHANGELOG.md). Items under `#[doc(hidden)]` are ada's and may
change in any release.

## [1.2.0](https://github.com/junkdog/ada-md/compare/v1.1.0..v1.2.0) (unreleased)

73 languages, up from 61: Dart, Perl, assembly in seven families, Git Config, man pages and markdown.

### Languages

- Dart, with SQL and regexes in its strings.
- Perl, its regexes in Perl's dialect: recursion, branch resets, backtracking verbs, `\Q…\E`. A here-document in
  HTML, CSS, JavaScript, JSON or XML, and POD's `=begin html` regions, are Perl's text in `lex`'s spans.
- Assembly in seven families: x86 and x86-64 (NASM, YASM, MASM, GAS), ARM (ARM32 and AArch64, GAS and armasm),
  RISC-V, MIPS, Z80 (and the Game Boy's SM83), 6502 (and 65C02, 65816) and 68k (to the 68060, and ColdFire). bat's
  set has only the first two; the others carry scopes of ada-parse's own, under `source.asm.*`.
- Git Config (`.gitconfig`, `.gitmodules`, `.lfsconfig`), the commands git runs through the shell (a `!` alias,
  `core.pager`, a filter's `clean`, …) as bash.
- Manpage, a page as `man` shows it, the C in its synopses as C.
- Markdown as source: CommonMark, GitHub's tables, strikethrough and links. A fence's code is one raw span in `lex`'s
  spans, its state carried through the fence.

### API

- `Lang`'s new variants: `Dart`, `Perl`, `GitConfig`, `Manpage`, `Markdown`, `X86Asm`, `ArmAsm`, `RiscvAsm`,
  `MipsAsm`, `Z80Asm`, `Mos6502Asm` and `M68kAsm`, found by `from_token` and `from_file_name`.
- `Lang::assembly_by_text` says whether a file's name (`.s`, `.S`, `.asm`, `.inc`) leaves its architecture to its
  text, and gives the name's guess; `Lang::is_assembly` tells the seven families from other languages.
- `Lang::for_include` and `Lang::is_include`: a `.i` file is 68k's where its text says so, and plain otherwise
  (SWIG's interfaces, `gcc -E`'s output).

## [1.1.0](https://github.com/junkdog/ada-md/compare/v1.0.0..v1.1.0) (2026-11-18)

The first release on crates.io: ada's lexers as a library of their own, with no dependencies.

- 61 languages: the common languages (Rust, Python, Go, Java, Kotlin, Scala, C, C++, C#, Objective-C, Swift,
  JavaScript, TypeScript, PHP, Ruby, Elixir, Haskell, the Lisps and more) and the config formats (CMake, Terraform,
  Nix, nginx, OpenSSH's files, crontab and more), each kind carrying the scopes bat's grammars give the same text.
- SQL and regexes inside other languages' strings, each regex in its engine's dialect; HTML's styles and scripts,
  PHP's HTML, a Dockerfile's `RUN` and Nix's phases as shell, nginx's Lua blocks.
- `lex` for a line's spans, from a `State` that is small and `Copy`, so any line can be lexed again from the state
  before it; `Lang::from_token`, `from_file_name` and `from_first_line` to find a language.
