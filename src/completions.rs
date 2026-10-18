//! Shell completion scripts, generated from the CLI's flag table (`ada --completions SHELL`).
//! ada has no clap (ADR-10), so instead of clap_complete these few lines write the zsh, bash and
//! fish scripts from [`cli::FLAGS`](crate::cli::FLAGS): a new flag completes without touching this file. Themes
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
  local -a themes
  themes=({} {THEMES_DIR}/*.toml(N:t:r))
  _alternative 'themes:theme:compadd -a themes' 'files:theme file:_files -g \"*.toml\"'
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
                "local themes=\"{}\" f\n      for f in \"{THEMES_DIR}\"/*.toml; do\n        [[ -e $f ]] && f=${{f##*/}} && themes+=\" ${{f%.toml}}\"\n      done\n      COMPREPLY=($(compgen -W \"$themes\" -- \"$cur\") $(compgen -f -X '!*.toml' -- \"$cur\") $(compgen -d -- \"$cur\"))\n      return",
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
            Value::Theme => line.push_str(" -r -a '(__ada_themes)'"),
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
    }

    /// Both ways of installing the zsh script register it without running it: sourced after
    /// compinit, and autoloaded from `$fpath`.
    #[test]
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

    /// The scripts parse, when the shell is installed.
    #[test]
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
}
