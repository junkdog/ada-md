//! ada-parse's lexers alone, in MB of source a second, on the census corpora (ADR-40): each
//! file's lines through `lex` from the default state, the corpus repeated to at least 4 MB a
//! round, best of 5 rounds. The lines are split before the timing, so only the lexer is timed:
//! `str::lines` inside the loop was 11–19% of a lexer's time and doubled its branch misses where
//! lines are short (X-051); `SPLIT=in` times the old loop, the split in it. `$ADA_PARSE_LISTS` is
//! the directory of lists (`<lang>.txt`, one path a line); `$ONLY` picks one language, for a
//! profile. Run pinned, in release:
//!
//! ```zsh
//! ADA_PARSE_LISTS=bench/b2/root/build/syntect-probe/lists taskset -c 2 \
//!   cargo test -p ada-parse --release --test lexer_speed -- --ignored --nocapture
//! ```

use std::time::{Duration, Instant};

use ada_parse::{Lang, State, lex};

const LANGS: &[(&str, Lang)] = &[
    ("json", Lang::Json),
    ("diff", Lang::Diff),
    // X-188's corpus: this machine's man pages as `man` shows them at 80 columns, `col -bx`'d
    ("man", Lang::Manpage),
    // token-cost's `log.txt`, log-corpus/'s snapshot
    ("log", Lang::Log),
    ("ini", Lang::Ini),
    ("toml", Lang::Toml),
    ("xml", Lang::Xml),
    ("yaml", Lang::Yaml),
    ("dockerfile", Lang::Dockerfile),
    ("properties", Lang::Properties),
    ("dotenv", Lang::DotEnv),
    ("gitignore", Lang::GitIgnore),
    ("gitattributes", Lang::GitAttributes),
    ("mailmap", Lang::GitMailmap),
    // X-177's corpus: this machine's repositories' `.git/config`, `.gitmodules` and gitconfigs
    ("gitconfig", Lang::GitConfig),
    ("fstab", Lang::Fstab),
    // X-146's corpus: this machine's installed `.pc` files
    ("pkgconfig", Lang::PkgConfig),
    ("known_hosts", Lang::KnownHosts),
    // X-060's corpus: GitHub's client and server configs, and this machine's
    ("ssh_config", Lang::SshConfig),
    ("sshd_config", Lang::SshdConfig),
    // X-059's corpus: GitHub's crontabs, and this machine's
    ("crontab", Lang::Crontab),
    // X-061's corpus: GitHub's nginx configs and Markdown's nginx fences, and this machine's
    ("nginx", Lang::Nginx),
    ("rust", Lang::Rust),
    ("kotlin", Lang::Kotlin),
    ("java", Lang::Java),
    ("cpp", Lang::Cpp),
    ("c", Lang::C),
    ("objc", Lang::ObjC),
    ("objcpp", Lang::ObjCpp),
    ("csharp", Lang::CSharp),
    ("go", Lang::Go),
    // X-171's corpus: cloned repositories' x86 (NASM, MASM, GAS) and ARM (ARM32, AArch64)
    ("x86asm", Lang::X86Asm),
    ("armasm", Lang::ArmAsm),
    // X-181's corpus: cloned repositories' RISC-V, MIPS (GAS, MARS, armips), Z80 (rgbds, sjasmplus,
    // z80asm, TASM) and 6502 (ca65, DASM, ACME)
    ("riscvasm", Lang::RiscvAsm),
    ("mipsasm", Lang::MipsAsm),
    ("z80asm", Lang::Z80Asm),
    ("asm6502", Lang::Mos6502Asm),
    // X-183's corpus: cloned repositories' 68k (vasm, Devpac, asm68k, AS, GAS's and MIT's)
    ("m68kasm", Lang::M68kAsm),
    ("python", Lang::Python),
    ("lua", Lang::Lua),
    // X-099's corpus: twenty cloned PHP repositories
    ("php", Lang::Php),
    // X-101's corpus: 30 Swift repositories' files
    ("swift", Lang::Swift),
    // X-104's corpus: 40 CMake-built repositories' CMake files
    ("cmake", Lang::CMake),
    // X-140's: the clones' 54 `.h.in` and `.h.cmake` header templates
    ("cmakecheader", Lang::CMakeCHeader),
    // X-102's corpus: 36 Terraform and HCL repositories' files
    ("terraform", Lang::Terraform),
    // X-103's corpus: 55 Nix repositories' files (nixpkgs by its subtrees)
    ("nix", Lang::Nix),
    // X-137's corpus: 34 Scala repositories' files
    ("scala", Lang::Scala),
    // X-135's corpus: 33 Groovy repositories' files (Gradle, Jenkins, Grails, Spock…)
    ("groovy", Lang::Groovy),
    // X-169's corpus: 42 Dart repositories' files (the SDK, Flutter, dart-lang's packages, apps)
    ("dart", Lang::Dart),
    // X-138's corpus: 49 Elixir repositories' files (Elixir, Phoenix, Ecto, Livebook, Plausible…)
    ("elixir", Lang::Elixir),
    // X-136's corpus: 42 Haskell repositories' files
    ("haskell", Lang::Haskell),
    // X-100's corpus: 37 Ruby repositories' files
    ("ruby", Lang::Ruby),
    ("sql", Lang::Sql),
    ("powershell", Lang::PowerShell),
    ("javascript", Lang::JavaScript),
    ("typescript", Lang::TypeScript),
    ("tsx", Lang::TypeScriptReact),
    ("css", Lang::Css),
    ("scss", Lang::Scss),
    ("html", Lang::Html),
    ("lisp", Lang::Lisp),
    ("elisp", Lang::EmacsLisp),
    ("scheme", Lang::Scheme),
    ("racket", Lang::Racket),
    ("clojure", Lang::Clojure),
    ("bash", Lang::Bash),
    // `make.txt` phase 0's Makefiles
    ("make", Lang::Makefile),
];

#[test]
#[ignore]
fn lexer_speed() {
    let dir = std::env::var("ADA_PARSE_LISTS").expect("ADA_PARSE_LISTS");
    let only = std::env::var("ONLY").ok();
    let split_in = std::env::var("SPLIT").is_ok_and(|s| s == "in");
    let mut spans = Vec::new();
    for &(name, lang) in LANGS {
        if only.as_deref().is_some_and(|o| o != name) {
            continue;
        }
        let Ok(list) = std::fs::read_to_string(format!("{dir}/{name}.txt")) else {
            continue;
        };
        let files: Vec<String> = list
            .lines()
            .filter_map(|p| std::fs::read_to_string(p).ok())
            .collect();
        let bytes: usize = files.iter().map(String::len).sum();
        // each file's lines, split as `str::lines` splits them (a `\r` before a `\n` dropped)
        let split: Vec<Vec<&str>> = files.iter().map(|f| f.lines().collect()).collect();
        let lines: usize = split.iter().map(Vec::len).sum();
        if bytes == 0 {
            continue;
        }
        let repeat = (4_000_000 / bytes).max(1);
        let mut best = Duration::MAX;
        for _ in 0..5 {
            let t = Instant::now();
            if split_in {
                for _ in 0..repeat {
                    for text in &files {
                        let mut state = State::default();
                        for line in text.lines() {
                            lex(lang, line, &mut state, &mut spans);
                        }
                    }
                }
            } else {
                for _ in 0..repeat {
                    for file in &split {
                        let mut state = State::default();
                        for &line in file {
                            lex(lang, line, &mut state, &mut spans);
                        }
                    }
                }
            }
            best = best.min(t.elapsed());
        }
        let secs = best.as_secs_f64() / repeat as f64;
        println!(
            "{name:14} {:7.1} MB/s  {:6.0} ns/line  {bytes:>9} bytes {lines:>7} lines {:>4} files",
            bytes as f64 / secs / 1e6,
            secs * 1e9 / lines as f64,
            files.len()
        );
    }
}
