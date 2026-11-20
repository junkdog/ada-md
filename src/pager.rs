//! ada as git's pager (FEAT-021): what git was asked for, which its output alone doesn't say.
//! git starts its pager as a child of its own, the command's arguments on its command line (an
//! alias already expanded: git runs itself again with the alias's command), and gives it
//! `GIT_EXEC_PATH`. `git blame`'s output doesn't name the file it blames, but for `-f`; its
//! command line does, and so the language its lines are in (FEAT-026).
//!
//! Read once, on Linux, from `/proc`: the parent's command line, or its parent's when a shell
//! between them didn't exec the pager. Elsewhere, nothing: the sniffer reads the language from the
//! text.

use std::sync::OnceLock;

/// The file `git blame` (or `git annotate`) is paging, as its command line names it: relative to
/// where git was run, not to ada's directory (git's toplevel). Only its name is read.
pub fn blamed_file() -> Option<&'static str> {
    static BLAMED: OnceLock<Option<String>> = OnceLock::new();
    BLAMED
        .get_or_init(|| {
            // git gives every command it runs its exec path: no git, no lookup
            std::env::var_os("GIT_EXEC_PATH")?;
            parents()
                .iter()
                .find_map(|args| blamed(&args.iter().map(String::as_str).collect::<Vec<_>>()))
                .map(str::to_owned)
        })
        .as_deref()
}

/// The arguments of ada's parent and its parent's.
#[cfg(target_os = "linux")]
fn parents() -> Vec<Vec<String>> {
    let mut found = Vec::new();
    let mut pid = std::os::unix::process::parent_id();
    for _ in 0..2 {
        let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
            break;
        };
        let args = cmdline
            .split(|&b| b == 0)
            .filter(|a| !a.is_empty())
            .map(|a| String::from_utf8_lossy(a).into_owned());
        found.push(args.collect());
        // `/proc/<pid>/stat`: `pid (comm) state ppid …`, the name in parentheses any text
        let Some(ppid) = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|s| {
                s.get(s.rfind(')')? + 1..)?
                    .split_whitespace()
                    .nth(1)?
                    .parse()
                    .ok()
            })
        else {
            break;
        };
        pid = ppid;
    }
    found
}

#[cfg(not(target_os = "linux"))]
fn parents() -> Vec<Vec<String>> {
    Vec::new()
}

/// The file a git command line `args` blames: `git [options] blame [options] [rev] [--] file`.
/// The file is the last argument, or the first after `--`; an option's value (`-L 10,20`) is
/// never last. `None` for any other command.
fn blamed<'a>(args: &[&'a str]) -> Option<&'a str> {
    let (program, args) = args.split_first()?;
    let name = program.rsplit('/').next()?;
    if name != "git" && name != "git.exe" {
        return None;
    }
    let at = args.iter().position(|&a| a == "blame" || a == "annotate")?;
    let args = &args[at + 1..];
    let file = match args.iter().position(|&a| a == "--") {
        Some(dashes) => args.get(dashes + 1)?,
        None => args.last()?,
    };
    (!file.starts_with('-')).then_some(*file)
}

#[cfg(test)]
mod tests {
    use super::blamed;

    #[test]
    fn the_blamed_file_from_gits_command_line() {
        fn file(line: &str) -> Option<&str> {
            blamed(&line.split(' ').collect::<Vec<_>>())
        }
        assert_eq!(file("git blame src/main.rs"), Some("src/main.rs"));
        assert_eq!(file("git blame -L 10,20 HEAD~3 -- a.py"), Some("a.py"));
        assert_eq!(
            file("/usr/lib/git-core/git blame -s -- README.md"),
            Some("README.md")
        );
        assert_eq!(file("git -c core.pager=ada annotate x.c"), Some("x.c"));
        assert_eq!(file("git log -p"), None);
        assert_eq!(file("git blame"), None);
        assert_eq!(file("git blame --"), None);
        assert_eq!(file("git blame -w"), None);
        assert_eq!(file("zsh"), None);
        assert_eq!(file("blame x.rs"), None);
    }
}
