//! Shell completion scripts, generated from the CLI's flag table (`ada --completions SHELL`).
//! ada has no clap (ADR-10), so instead of clap_complete these few lines write the zsh, bash,
//! fish and PowerShell scripts from [`cli::FLAGS`](crate::cli::FLAGS): a new flag completes without touching this file. Themes
//! complete to the built-ins, the user's themes directory (read when completing) and `.toml`
//! paths; languages to the names the bundled syntaxes take, listed in the script; the file to any
//! file.

use std::fmt::Write as _;

use crate::cli::{FLAGS, Flag, Value};
use crate::filetype::language_names;
use crate::theme::BUILTIN;

pub fn script(shell: &str) -> Option<String> {
    match shell {
        "zsh" => Some(zsh()),
        "bash" => Some(bash()),
        "fish" => Some(fish()),
        "powershell" => Some(powershell()),
        _ => None,
    }
}

fn builtin_themes() -> String {
    BUILTIN
        .iter()
        .map(|(n, _)| *n)
        .collect::<Vec<_>>()
        .join(" ")
}

fn language_words() -> String {
    language_names()
        .into_iter()
        .map(|(n, _)| n)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The first line of a flag's description.
fn summary(f: &Flag) -> &'static str {
    f.help.split('\n').next().unwrap_or_default()
}

const THEMES_DIR: &str = "${XDG_CONFIG_HOME:-$HOME/.config}/ada/themes";

fn zsh() -> String {
    let mut out = format!(
        "\
#compdef ada
# zsh completion for ada, from `ada --completions zsh`

_ada_themes() {{
  if [[ $PREFIX == (./|../|/)* ]]; then
    _alternative 'files:theme file:_path_files -g \"*.toml(-.)\"' 'directories:directory:_path_files -/'
    return
  fi
  local -a themes
  themes=({} {THEMES_DIR}/*.toml(N:t:r))
  _describe -t themes theme themes
}}

_ada_languages() {{
  local -a languages
  languages=(
{}  )
  _describe -t languages language languages
}}

_ada() {{
  _arguments -s -S \\
",
        builtin_themes(),
        language_names()
            .iter()
            .map(|(n, syntax)| {
                // `name:description`, single-quoted
                let syntax = syntax.replace(':', "\\:").replace('\'', "'\\''");
                format!("    '{n}:{syntax}'\n")
            })
            .collect::<String>()
    );
    for f in FLAGS {
        // `[` and `]` end the description, and `:` would start the value's
        let help = summary(f)
            .replace('[', "\\[")
            .replace(']', "\\]")
            .replace(':', "\\:")
            .replace('\'', "'\\''");
        let action = match f.value.1 {
            Value::None => String::new(),
            Value::Number => format!(":{}:", f.value.0.to_lowercase()),
            Value::Theme => ":theme:_ada_themes".to_owned(),
            Value::Language => ":language:_ada_languages".to_owned(),
            Value::File => ":file:_files".to_owned(),
            Value::Rec => ":recording:_files -g \"*.rec\"".to_owned(),
            Value::Choice(values) => {
                format!(":{}:({})", f.value.0.to_lowercase(), values.join(" "))
            }
        };
        let exclusive = matches!(f.long, "help" | "version" | "completions");
        let takes = if f.value.1 == Value::None { "" } else { "=" };
        let line = match (f.short, exclusive) {
            (_, true) => {
                let names = f.short.map_or_else(
                    || format!("--{}{takes}", f.long),
                    |c| format!("{{-{c},--{}}}", f.long),
                );
                format!("'(- *)'{names}'[{help}]{action}'")
            }
            (Some(c), false) => {
                format!(
                    "'(-{c} --{l})'{{-{c},--{l}{takes}}}'[{help}]{action}'",
                    l = f.long
                )
            }
            (None, false) => format!("'--{}{takes}[{help}]{action}'", f.long),
        };
        let _ = writeln!(out, "    {line} \\");
    }
    out.push_str(
        "    '1:file:_files'
}

# Autoloaded from $fpath, this file is the completion function, so complete. Sourced, as with
# `source <(ada --completions zsh)` after compinit, register the function instead.
if [[ $funcstack[1] == _ada ]]; then
  _ada \"$@\"
else
  compdef _ada ada
fi
",
    );
    out
}

fn bash() -> String {
    let words: Vec<String> = FLAGS
        .iter()
        .flat_map(|f| {
            f.short
                .map(|c| format!("-{c}"))
                .into_iter()
                .chain([format!("--{}", f.long)])
        })
        .collect();
    let names = |f: &Flag| {
        let mut names = format!("--{}", f.long);
        if let Some(c) = f.short {
            names = format!("-{c}|{names}");
        }
        names
    };
    let mut cases = String::new();
    for f in FLAGS {
        let reply = match f.value.1 {
            Value::None => continue,
            Value::Number => "return".to_owned(),
            Value::Theme => format!(
                "if [[ $cur == ./* || $cur == ../* || $cur == /* ]]; then\n        COMPREPLY=($(compgen -f -X '!*.toml' -- \"$cur\") $(compgen -d -- \"$cur\"))\n        return\n      fi\n      compopt +o filenames 2>/dev/null\n      local themes=\"{}\" f\n      for f in \"{THEMES_DIR}\"/*.toml; do\n        [[ -e $f ]] && f=${{f##*/}} && themes+=\" ${{f%.toml}}\"\n      done\n      COMPREPLY=($(compgen -W \"$themes\" -- \"$cur\"))\n      return",
                builtin_themes()
            ),
            Value::Language => format!(
                "COMPREPLY=($(compgen -W \"{}\" -- \"$cur\"))\n      return",
                language_words()
            ),
            Value::File => "COMPREPLY=($(compgen -f -- \"$cur\"))\n      return".to_owned(),
            Value::Rec => "COMPREPLY=($(compgen -f -X '!*.rec' -- \"$cur\") $(compgen -d -- \"$cur\"))\n      return".to_owned(),
            Value::Choice(values) => format!(
                "COMPREPLY=($(compgen -W \"{}\" -- \"$cur\"))\n      return",
                values.join(" ")
            ),
        };
        let _ = writeln!(cases, "    {})\n      {reply}\n      ;;", names(f));
    }
    format!(
        "\
# bash completion for ada, from `ada --completions bash`

_ada() {{
  local cur=${{COMP_WORDS[COMP_CWORD]}} prev=${{COMP_WORDS[COMP_CWORD-1]}}
  case $prev in
{cases}  esac
  if [[ $cur == -* ]]; then
    COMPREPLY=($(compgen -W \"{}\" -- \"$cur\"))
    return
  fi
  COMPREPLY=($(compgen -f -- \"$cur\"))
}}

complete -o filenames -F _ada ada
",
        words.join(" ")
    )
}

fn fish() -> String {
    let mut out = format!(
        "\
# fish completion for ada, from `ada --completions fish`

function __ada_themes
    set -l token (commandline -ct | string replace -r -- '^--theme=' '')
    if string match -qr -- '^[.]{{0,2}}/' $token
        for f in $token*
            if test -d $f
                echo $f/
            else if string match -q -- '*.toml' $f
                echo $f
            end
        end
        return
    end
    printf '%s\\n' {}
    set -l config $XDG_CONFIG_HOME
    test -n \"$config\"; or set config ~/.config
    for f in $config/ada/themes/*.toml
        basename $f .toml
    end
end

function __ada_languages
    printf '%s\\t%s\\n' \\
{}
end

",
        builtin_themes(),
        language_names()
            .iter()
            .map(|(n, syntax)| format!("        '{n}' '{}'", syntax.replace('\'', "\\'")))
            .collect::<Vec<_>>()
            .join(" \\\n")
    );
    for f in FLAGS {
        let mut line = String::from("complete -c ada");
        if let Some(c) = f.short {
            let _ = write!(line, " -s {c}");
        }
        let _ = write!(line, " -l {}", f.long);
        match f.value.1 {
            Value::None => {}
            Value::Number => line.push_str(" -x"),
            Value::Theme => line.push_str(" -x -a '(__ada_themes)'"),
            Value::Language => line.push_str(" -x -a '(__ada_languages)'"),
            Value::File => line.push_str(" -r -F"),
            Value::Rec => line.push_str(" -r -k -a '(__fish_complete_suffix .rec)'"),
            Value::Choice(values) => {
                let _ = write!(line, " -x -a '{}'", values.join(" "));
            }
        }
        let _ = writeln!(line, " -d '{}'", summary(f).replace('\'', "\\'"));
        out.push_str(&line);
    }
    out
}

/// PowerShell's, for Windows PowerShell 5.1 and pwsh alike (WIN-009). A native completer gets
/// the word and the line, and finds the flag before the word itself; a value it has nothing for
/// (a number, a file) completes nothing, and PowerShell falls back to paths. The themes
/// directory is `ada`'s: `%APPDATA%` before `$HOME/.config`, which only pwsh off Windows reaches.
fn powershell() -> String {
    // single-quoted, where only `'` is special, doubled
    let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let pair = |a: &str, b: &str| format!("        ,({}, {})\n", quote(a), quote(b));
    let mut out = String::from(
        "\
# PowerShell completion for ada, from `ada --completions powershell`

Register-ArgumentCompleter -Native -CommandName ada, ada.exe -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)

",
    );
    let themes: Vec<String> = BUILTIN.iter().map(|(n, _)| quote(n)).collect();
    let _ = writeln!(out, "    $themes = @({})", themes.join(", "));
    out.push_str("    $languages = @(\n");
    for (n, syntax) in language_names() {
        out.push_str(&pair(&n, syntax));
    }
    out.push_str("    )\n    $flags = @(\n");
    for f in FLAGS {
        if let Some(c) = f.short {
            out.push_str(&pair(&format!("-{c}"), summary(f)));
        }
        out.push_str(&pair(&format!("--{}", f.long), summary(f)));
    }
    out.push_str(
        "    )

    # `--theme=NAME` completes as `--theme NAME` does
    $word, $prefix = \"$wordToComplete\", ''
    if ($word -match '^(--[^=]+)=(.*)$') {
        $prev, $prefix, $word = $Matches[1], \"$($Matches[1])=\", $Matches[2]
    } else {
        $before = @($commandAst.CommandElements | Where-Object { $_.Extent.EndOffset -lt $cursorPosition })
        $prev = if ($before.Count -gt 1) { $before[-1].Extent.Text } else { '' }
    }
    $like = [System.Management.Automation.WildcardPattern]::Escape($word) + '*'

    # -l and -L are different flags
    $kind, $choices = '', @()
    switch -CaseSensitive ($prev) {
",
    );
    for f in FLAGS {
        let kind = match f.value.1 {
            Value::None => continue,
            Value::Number => "$kind = 'number'".to_owned(),
            Value::Theme => "$kind = 'theme'".to_owned(),
            Value::Language => "$kind = 'language'".to_owned(),
            Value::File => "$kind = 'file'".to_owned(),
            Value::Rec => "$kind = 'rec'".to_owned(),
            Value::Choice(values) => {
                let values: Vec<String> = values.iter().map(|v| quote(v)).collect();
                format!("$kind = 'choice'; $choices = {}", values.join(", "))
            }
        };
        let long = quote(&format!("--{}", f.long));
        let names = f.short.map_or(long.clone(), |c| format!("'-{c}', {long}"));
        let _ = writeln!(out, "        {{ $_ -cin {names} }} {{ {kind} }}");
    }
    out.push_str(
        "    }

    $complete = {
        param($text, $tip, $type)
        $quoted = if ($text -match '\\s') { \"'$($text -replace \"'\", \"''\")'\" } else { $text }
        [System.Management.Automation.CompletionResult]::new(\"$prefix$quoted\", $text, $type, $tip)
    }
    # the entries under the word's directory: directories, and files ending in $ext
    $paths = {
        param($ext)
        $dir = $word.Substring(0, $word.LastIndexOfAny([char[]]'\\/') + 1)
        $sep = if ($word.Contains('\\')) { '\\' } else { '/' }
        foreach ($item in @(Get-ChildItem -Path \"$word*\" -ErrorAction Ignore)) {
            if ($item.PSIsContainer) {
                & $complete \"$dir$($item.Name)$sep\" $item.Name 'ProviderContainer'
            } elseif ($item.Name.EndsWith($ext)) {
                & $complete \"$dir$($item.Name)\" $item.Name 'ProviderItem'
            }
        }
    }

    switch ($kind) {
        'theme' {
            if ($word -match '^(\\.{0,2}[\\\\/]|[A-Za-z]:)') {
                & $paths '.toml'
                break
            }
            $config = if ($env:XDG_CONFIG_HOME) { $env:XDG_CONFIG_HOME }
                elseif ($env:APPDATA) { $env:APPDATA }
                else { Join-Path $HOME '.config' }
            $user = @(Get-ChildItem -Path (Join-Path $config 'ada/themes') -Filter '*.toml' -ErrorAction Ignore |
                ForEach-Object BaseName)
            @($themes) + $user | Where-Object { $_ -like $like } | ForEach-Object { & $complete $_ $_ 'ParameterValue' }
        }
        'language' {
            $languages | Where-Object { $_[0] -like $like } | ForEach-Object { & $complete $_[0] $_[1] 'ParameterValue' }
        }
        'choice' {
            $choices | Where-Object { $_ -like $like } | ForEach-Object { & $complete $_ $_ 'ParameterValue' }
        }
        'rec' { & $paths '.rec' }
        '' {
            if ($word.StartsWith('-')) {
                $flags | Where-Object { $_[0] -clike $like } | ForEach-Object { & $complete $_[0] $_[1] 'ParameterName' }
            }
        }
    }
}
",
    );
    out
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    #[test]
    fn every_flag_is_in_every_script() {
        for shell in crate::cli::SHELLS {
            let text = script(shell).unwrap();
            for f in FLAGS {
                assert!(text.contains(f.long), "{shell}: --{}", f.long);
            }
            assert!(
                text.contains("gruvbox-light"),
                "{shell}: the built-in themes"
            );
            assert!(text.contains("yml"), "{shell}: the languages");
        }
        assert_eq!(script("tcsh"), None);
        // Windows PowerShell reads `ada --completions powershell | Invoke-Expression` in the
        // console's code page
        assert!(script("powershell").unwrap().is_ascii());
    }

    // zsh, bash and fish run on unix only: on Windows, `bash` is System32's WSL launcher before
    // Git Bash, as `Command` looks there before `PATH`

    /// Both ways of installing the zsh script register it without running it: sourced after
    /// compinit, and autoloaded from `$fpath`.
    #[test]
    #[cfg(unix)]
    fn zsh_sources_and_autoloads() {
        let dir = std::env::temp_dir().join(format!("ada-zsh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("_ada"), script("zsh").unwrap()).unwrap();
        let d = dir.display();
        let sourced = format!(
            "autoload -U compinit; compinit -u -d {d}/dump; source {d}/_ada; print -r -- $_comps[ada]"
        );
        let autoloaded = format!(
            "fpath=({d} $fpath); autoload -U compinit; compinit -u -d {d}/dump2; print -r -- $_comps[ada]"
        );
        for (how, cmd) in [("sourced", sourced), ("autoloaded", autoloaded)] {
            let Ok(out) = Command::new("zsh").args(["-f", "-c", &cmd]).output() else {
                break;
            };
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(out.status.success() && stderr.is_empty(), "{how}: {stderr}");
            assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "_ada", "{how}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A theme completes to the theme names alone, not to the files and directories around it,
    /// and to `.toml` files and directories only after `./`, `../` or `/`; when bash and fish
    /// are installed.
    #[test]
    #[cfg(unix)]
    fn themes_complete_to_names_and_paths_to_files() {
        let dir = std::env::temp_dir().join(format!("ada-themes-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("x.toml"), "").unwrap();
        std::fs::write(dir.join("notes.md"), "").unwrap();
        let bash = |line: &str| {
            let cmd = format!(
                "{}\nCOMP_WORDS=({line}); COMP_CWORD=2; _ada; printf '%s\\n' \"${{COMPREPLY[@]}}\" | sort",
                script("bash").unwrap()
            );
            Command::new("bash")
                .args(["--norc", "-c", &cmd])
                .current_dir(&dir)
                .output()
        };
        let fish = |line: &str| {
            let cmd = format!(
                "{}\ncomplete -C '{line}' | cut -f1 | sort",
                script("fish").unwrap()
            );
            Command::new("fish")
                .args(["--no-config", "-c", &cmd])
                .current_dir(&dir)
                .output()
        };
        type Run<'a> = &'a dyn Fn(&str) -> std::io::Result<std::process::Output>;
        let shells: [(&str, Run, &str, &str); 2] = [
            ("bash", &bash, "ada -t ''", "ada -t ./"),
            ("fish", &fish, "ada -t ", "ada -t ./"),
        ];
        for (shell, run, names, paths) in shells {
            let Ok(out) = run(names) else {
                continue;
            };
            let names = String::from_utf8_lossy(&out.stdout).into_owned();
            assert!(names.contains("gruvbox-dark\n"), "{shell}: {names}");
            assert!(
                !names.contains("sub") && !names.contains(".md"),
                "{shell}: {names}"
            );
            let out = run(paths).unwrap();
            let paths = String::from_utf8_lossy(&out.stdout).into_owned();
            assert!(
                paths.starts_with("./sub") && paths.contains("./x.toml\n"),
                "{shell}: {paths}"
            );
            assert!(
                !paths.contains("notes") && !paths.contains("gruvbox"),
                "{shell}: {paths}"
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The scripts parse, when the shell is installed.
    #[test]
    #[cfg(unix)]
    fn the_shells_read_them() {
        for (shell, args) in [
            ("zsh", ["-n", "-c"]),
            ("bash", ["-n", "-c"]),
            ("fish", ["-n", "-c"]),
        ] {
            let Ok(out) = Command::new(shell)
                .args(args)
                .arg(script(shell).unwrap())
                .output()
            else {
                continue;
            };
            assert!(
                out.status.success(),
                "{shell}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    /// Flags, values and paths complete in PowerShell, when pwsh is installed (as on every CI
    /// runner): asked as TabExpansion2 asks, after `-l` and `-L` alike.
    #[test]
    fn powershell_completes() {
        let dir = std::env::temp_dir().join(format!("ada-pwsh-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for f in ["x.toml", "notes.md", "a.rec"] {
            std::fs::write(dir.join(f), "").unwrap();
        }
        std::fs::write(dir.join("ada.ps1"), script("powershell").unwrap()).unwrap();
        let lines = [
            "ada -",
            "ada -L",
            "ada -t ",
            "ada -t ./",
            "ada --theme=gruvbox-l",
            "ada -l yam",
            "ada -L 4",
            "ada -n ",
            "ada --replay ",
            "ada --completions p",
        ];
        let mut cmd = String::from("$ErrorActionPreference = 'Stop'; . ./ada.ps1\n");
        for line in lines {
            let _ = writeln!(
                cmd,
                "(TabExpansion2 -inputScript '{line}' -cursorColumn {}).CompletionMatches.CompletionText -join ' '",
                line.len()
            );
        }
        // one line per completion
        let Ok(out) = Command::new("pwsh")
            .args(["-NoProfile", "-NonInteractive", "-Command", &cmd])
            .current_dir(&dir)
            .output()
        else {
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        };
        std::fs::remove_dir_all(&dir).unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "{stdout}{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let got: Vec<&str> = stdout.lines().collect();
        assert_eq!(got.len(), lines.len(), "{stdout}");
        let words = |i: usize| got[i].split(' ').collect::<Vec<_>>();
        assert!(
            words(0).contains(&"--print") && words(0).contains(&"-l"),
            "{}",
            got[0]
        );
        assert_eq!(words(1), ["-L"], "-L, not -l");
        assert!(words(2).contains(&"gruvbox-dark"), "{}", got[2]);
        assert!(
            !got[2].contains("sub") && !got[2].contains(".md"),
            "{}",
            got[2]
        );
        assert_eq!(words(3), ["./sub/", "./x.toml"], "directories and themes");
        assert_eq!(words(4), ["--theme=gruvbox-light"]);
        assert!(words(5).contains(&"yaml"), "{}", got[5]);
        // a number has nothing to complete: PowerShell's own paths, not a language
        assert!(!words(6).contains(&"yaml"), "{}", got[6]);
        assert_eq!(words(7), ["off", "code", "all"]);
        assert_eq!(words(8), ["sub/", "a.rec"]);
        assert_eq!(words(9), ["powershell"]);
    }
}
