//! `cargo xtask dump [--check]`: ada's syntax sets, `assets/syntaxes-{onig,fancy}.bin`. Each is
//! two-face's set (bat's grammars, in the flavor of its regexes) with the grammars ada's own
//! lexers stand in for ([`ada_parse::Lang`]) emptied, as the size offset for those lexers
//! (ADR-40). An emptied grammar keeps its contexts, with no patterns in them, so every context
//! another grammar links to stays where it was; an embed of one inside a grammar ada keeps
//! (bash in Markdown's fences, JSON in an HTTP body) shows plain. It also keeps its name,
//! extensions and first-line match: syntect still answers which language a file is, and ada
//! takes such an answer to its own lexer, never to the empty grammar.
//!
//! `--check` builds them and compares them with the files instead, exit 1 when they differ:
//! after a two-face update, or a change to the languages ada-parse covers.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use ada_parse::Lang;
use syntect::dumps;
use syntect::parsing::{SyntaxDefinition, SyntaxSet, SyntaxSetBuilder};

/// The regex flavors two-face ships a set for, by the name of its file and of ada's.
const FLAVORS: [&str; 2] = ["onig", "fancy"];

pub fn run(root: &Path, check: bool) -> Result<ExitCode, String> {
    let two_face = two_face_dir(root)?;
    let mut stale = Vec::new();
    for flavor in FLAVORS {
        let source = source_path(&two_face, flavor);
        let full: SyntaxSet = dumps::from_uncompressed_data(&read(&source)?)
            .map_err(|e| format!("{}: {e}", source.display()))?;
        if flavor == FLAVORS[0] {
            report(&full);
        }
        let bytes = dump(&trim(full))?;
        let out = asset_path(root, flavor);
        let before = fs::metadata(&source).map_or(0, |m| m.len());
        println!(
            "{flavor}: {before} → {} bytes ({} fewer)",
            bytes.len(),
            before as i64 - bytes.len() as i64,
        );
        if check {
            if fs::read(&out).ok().as_deref() != Some(&bytes[..]) {
                stale.push(out.display().to_string());
            }
        } else {
            fs::write(&out, &bytes).map_err(|e| format!("{}: {e}", out.display()))?;
        }
    }
    if stale.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    eprintln!(
        "stale: {}; `cargo xtask dump` rebuilds them",
        stale.join(", ")
    );
    Ok(ExitCode::FAILURE)
}

/// `set` with the grammars [`emptied`] picks emptied: no patterns in their contexts, and no
/// variables.
pub fn trim(set: SyntaxSet) -> SyntaxSet {
    // `into_builder` keeps the linked contexts and their ids; `build` sorts each grammar's
    // contexts by name again, as they already are
    let full = set.into_builder();
    let empty = emptied(full.syntaxes());
    let mut builder = SyntaxSetBuilder::new();
    for (def, empty) in full.syntaxes().iter().zip(empty) {
        let mut def = def.clone();
        if empty {
            for context in def.contexts.values_mut() {
                context.patterns.clear();
            }
            def.variables.clear();
        }
        builder.add(def);
    }
    builder.build()
}

/// Covered grammars that a grammar ada keeps is built on, not just embeds a block of: emptied,
/// it would lose them. Objective-C++ links to C++'s contexts 45 times (its classes, templates,
/// expressions), and C++ (so kept) to 127 of C's, on which Objective-C is built too; every
/// other grammar's links to a covered one are 1–3, an embedded block.
const BUILT_ON: &[&str] = &["C++", "C"];

/// Covered grammars kept for the blocks of them other grammars embed, where those blocks are
/// common enough that going plain would be a loss: `JavaScript (Babel)` is what
/// `scope:source.js` resolves to in the onig set, so HTML's, Vue's and Svelte's `<script>`
/// (and PHP's, Ruby's, Markdown's …) run on it (ADR-40's phase 5; until phase 6 covers HTML);
/// `TypeScript` is Vue's and Svelte's `<script lang="ts">`. `TypeScriptReact` goes: only
/// Typst's fences embed it. `SQL`, for Ruby's and PHP's SQL strings and heredocs, Perl's,
/// CFML's and Crystal's, and `SQL (Rails)`, which is SQL whole: 2,250 bytes of the dump. `CSS`,
/// for HTML's `<style>` and `style=` (and PHP's, Perl's, Ruby's, Crystal's, Twig's, MediaWiki's),
/// until phase 6 covers HTML. `SCSS`, for Vue's `<style lang="scss">`, as `TypeScript` for its
/// `<script lang="ts">`: 5,831 bytes of the onig dump.
const EMBEDDED: &[&str] = &["JavaScript (Babel)", "TypeScript", "SQL", "CSS", "SCSS"];

/// The grammars to empty, by index: the ones ada-parse covers but [`BUILT_ON`]'s and
/// [`EMBEDDED`]'s, then every
/// hidden one no file is named for (no extensions, no first line) that only emptied ones link
/// to, such as bash's `commands-builtin-shell-bash`.
fn emptied(defs: &[SyntaxDefinition]) -> Vec<bool> {
    let covered = Lang::ALL.map(Lang::name);
    let mut empty: Vec<bool> = defs
        .iter()
        .map(|d| {
            let name = d.name.as_str();
            covered.contains(&name) && !BUILT_ON.contains(&name) && !EMBEDDED.contains(&name)
        })
        .collect();
    let links: Vec<BTreeMap<usize, usize>> = defs.iter().map(links).collect();
    let unnamed = |d: &SyntaxDefinition| {
        d.hidden && d.file_extensions.is_empty() && d.first_line_match.is_none()
    };
    loop {
        let behind = |i: usize| {
            let from = (0..defs.len()).filter(|&j| j != i && links[j].contains_key(&i));
            unnamed(&defs[i]) && from.clone().count() > 0 && from.into_iter().all(|j| empty[j])
        };
        let Some(i) = (0..defs.len()).find(|&i| !empty[i] && behind(i)) else {
            return empty;
        };
        empty[i] = true;
    }
}

/// The grammars a grammar's contexts link to, by index, and how often: its patterns' pushes,
/// sets and includes, and its contexts' prototypes.
fn links(def: &SyntaxDefinition) -> BTreeMap<usize, usize> {
    let mut links = BTreeMap::new();
    for context in def.contexts.values() {
        // a `ContextId`'s fields are the crate's own; its `Debug` names them
        let text = format!("{context:?}");
        for part in text.split("syntax_index: ").skip(1) {
            let n = part.split(|c: char| !c.is_ascii_digit()).next();
            if let Some(n) = n.and_then(|n| n.parse().ok()) {
                *links.entry(n).or_default() += 1;
            }
        }
    }
    links
}

/// The set as two-face dumps it (`dump_to_uncompressed_file`): bincode, each grammar's contexts
/// compressed apart.
pub fn dump(set: &SyntaxSet) -> Result<Vec<u8>, String> {
    bincode::serialize(set).map_err(|e| format!("dump: {e}"))
}

/// The grammars emptied, and which kept ones link into them, how often: those embeds go plain.
fn report(set: &SyntaxSet) {
    let builder = set.clone().into_builder();
    let defs = builder.syntaxes();
    let empty = emptied(defs);
    let names = |pick: bool| -> Vec<&str> {
        let picked = defs.iter().zip(&empty).filter(|&(_, &e)| e == pick);
        picked.map(|(d, _)| d.name.as_str()).collect()
    };
    println!("emptied: {}", names(true).join(", "));
    for (def, _) in defs.iter().zip(&empty).filter(|&(_, &e)| !e) {
        let into: BTreeMap<&str, usize> = links(def)
            .into_iter()
            .filter(|&(i, _)| empty[i])
            .map(|(i, n)| (defs[i].name.as_str(), n))
            .collect();
        if !into.is_empty() {
            println!("  {} links to {into:?}", def.name);
        }
    }
}

/// two-face's crate directory, as the lockfile resolves it.
pub fn two_face_dir(root: &Path) -> Result<PathBuf, String> {
    let out = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--manifest-path",
        ])
        .arg(root.join("Cargo.toml"))
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let json = String::from_utf8_lossy(&out.stdout);
    // a package's object starts with its name and version (a dependency's, with its name and
    // source); its manifest path follows
    let at = json
        .find(r#"{"name":"two-face","version":"#)
        .ok_or("cargo metadata: no two-face package")?;
    let key = r#""manifest_path":""#;
    let start = json[at..]
        .find(key)
        .ok_or("cargo metadata: no manifest path")?
        + at
        + key.len();
    let end = json[start..]
        .find('"')
        .ok_or("cargo metadata: unterminated path")?
        + start;
    let manifest = PathBuf::from(&json[start..end]);
    Ok(manifest.parent().unwrap().to_path_buf())
}

pub fn source_path(two_face: &Path, flavor: &str) -> PathBuf {
    two_face.join(format!("generated/syntaxes-{flavor}-newlines.bin"))
}

pub fn asset_path(root: &Path, flavor: &str) -> PathBuf {
    root.join(format!("assets/syntaxes-{flavor}.bin"))
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
    }

    /// Rebuilding two-face's set changes nothing but what [`trim`] takes out: `build` relinks
    /// nothing that was linked, and orders the contexts as they were.
    #[test]
    fn a_rebuilt_set_is_two_faces() {
        let source = source_path(&two_face_dir(root()).unwrap(), "onig");
        let bytes = read(&source).unwrap();
        let set: SyntaxSet = dumps::from_uncompressed_data(&bytes).unwrap();
        let mut builder = SyntaxSetBuilder::new();
        for def in set.into_builder().syntaxes() {
            builder.add(def.clone());
        }
        assert!(dump(&builder.build()).unwrap() == bytes);
    }

    /// The dumps in `assets/` are what `cargo xtask dump` builds now.
    #[test]
    fn the_assets_are_current() {
        let two_face = two_face_dir(root()).unwrap();
        for flavor in FLAVORS {
            let full =
                dumps::from_uncompressed_data(&read(&source_path(&two_face, flavor)).unwrap());
            let bytes = dump(&trim(full.unwrap())).unwrap();
            let asset = read(&asset_path(root(), flavor)).unwrap();
            assert!(asset == bytes, "{flavor}: run `cargo xtask dump`");
        }
    }
}
