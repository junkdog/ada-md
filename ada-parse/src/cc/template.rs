//! CMake's header templates (X-140): bat's `CMake C Header` (`h.in`) and `CMake C++ Header`
//! (`hh.in`, `hpp.in`, `hxx.in`, `h++.in`), a C or C++ header that `configure_file` writes. The
//! header's text is C's or C++'s, on their scanner; what configuring replaces is read over it:
//! `#cmakedefine NAME …` and `#cmakedefine01 NAME`, which become a `#define` (or a comment), as a
//! `#define`; `@VAR@` and `${VAR}`, its placeholders, wherever they stand (code, a string, a
//! comment, a directive), as bat's prototype reads them.
//!
//! The scanner reads the template's line masked, byte for byte and ASCII for ASCII, so its
//! length, places and UTF-8 hold: `cmakedefine` as `     define` (`cmakedefine01` `       define`)
//! after a line's `#`, a placeholder as a name (`@VAR@` `_VAR_`, `${VAR}` `__VAR_`). It reads a
//! directive and a word where configuring leaves one, and its frames and state go on as the
//! configured header's. Then each placeholder's spans are laid over the scanner's, under what it
//! stands in ([`UNDER`]): a string's, a comment's, a directive's scope, as bat's stack has them.
//!
//! Departures from bat's grammar, by design: `#cmakedefine`'s name is a `#define`'s
//! (`entity.name.constant.preprocessor`; bat's `entity.name.variable.cmake`) and its rest a
//! macro's body (bat's code); `#cmakedefine01` is one (bat's `\bcmakedefine\b` misses it);
//! only an `@` before a name and an `@` is a placeholder (bat's `@` opens one anywhere and runs
//! to the next `@`, across lines: an address in a comment derails it).

use super::*;
use crate::Span;
use crate::emit::Rule;

/// A template's kinds from here: a placeholder's part ([`PARTS`]) under one of [`UNDER`]'s
/// kinds, `TPL + under * 3 + part`.
pub(super) const TPL: u16 = SQL_DQ + 128;
const _: () = assert!(sql::KINDS <= 128);

/// What a placeholder may stand in, as the scanner's kind there tells: code (or a name), a
/// directive's text, a string, a comment, a disabled branch, an include's path.
const UNDER: [u16; 9] = [
    PLAIN, PP_TEXT, DQ, SQ, BLOCK_TEXT, LINE_TEXT, IF_BRANCH, INC_DQ, INC_LT,
];

/// A placeholder's parts' scopes, bat's: `@` or `${`, the name, `@` or `}`.
const PARTS: [&[&str]; 3] = [
    &["variable.cmake", "keyword.other.block.start.cmake"],
    &["variable.cmake"],
    &["variable.cmake", "keyword.other.block.end.cmake"],
];

/// The scopes of a template's kind ([`TPL`] on), in `lang`'s row.
pub(super) fn scopes(kind: u16, lang: usize) -> Vec<&'static str> {
    let k = usize::from(kind - TPL);
    let (under, part) = (k / 3, k % 3);
    if under >= UNDER.len() {
        return Vec::new();
    }
    let mut out = scopes_in(UNDER[under], lang);
    out.extend(PARTS[part]);
    out
}

/// Which of [`UNDER`] the scanner's `kind` stands in: a string's, a comment's, an include's
/// own kinds and their marks; a regex or SQL in a string the string's; anything else code
/// ([`PLAIN`]), or a directive's text on a directive's line.
fn under(kind: u16, directive: bool) -> usize {
    let k = match kind {
        k if k >= RE_DQ => DQ,
        DQ..=DQ_PLACEHOLDER => DQ,
        SQ..=SQ_CONT => SQ,
        BLOCK_TEXT | COMMENT_MARK => BLOCK_TEXT,
        LINE_TEXT..=LINE_CONT => LINE_TEXT,
        IF_BRANCH => IF_BRANCH,
        INC_DQ..=INC_DQ_END => INC_DQ,
        INC_LT..=INC_LT_END => INC_LT,
        _ if directive => PP_TEXT,
        _ => PLAIN,
    };
    UNDER.iter().position(|&u| u == k).unwrap_or(0)
}

/// What a placeholder's name may hold: CMake's `configure_file` reads `@[A-Za-z0-9_/.+-]+@`
/// and a variable reference's name alike.
fn name_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'/' | b'.' | b'+' | b'-')
}

/// A placeholder at `p` (`@` or `$`): where its name starts, ends, and where it ends.
fn placeholder(b: &[u8], p: usize) -> Option<(usize, usize, usize)> {
    let s = match b[p] {
        b'@' => p + 1,
        b'$' if b.get(p + 1) == Some(&b'{') => p + 2,
        _ => return None,
    };
    let mut e = s;
    while e < b.len() && name_byte(b[e]) {
        e += 1;
    }
    let close = if b[p] == b'@' { b'@' } else { b'}' };
    (e > s && b.get(e) == Some(&close)).then_some((s, e, e + 1))
}

/// Lex `b`, a line of a C header's template (`CMake C Header`), from `state`.
#[inline(never)]
pub fn line_cmake_c(b: &[u8], state: &mut State, out: &mut Out) {
    template_line(b, state, out, L_C);
}

/// Lex `b`, a line of a C++ header's template (`CMake C++ Header`), from `state`.
#[inline(never)]
pub fn line_cmake_cpp(b: &[u8], state: &mut State, out: &mut Out) {
    template_line(b, state, out, L_CPP);
}

/// Each kind its own rule ([`Kinds`]): the run sink's runs are then the kinds' spans, joined
/// where they meet, with plain gaps between.
static IDENTITY: [Rule; TPL as usize] = {
    let mut t = [Rule(0); TPL as usize];
    let mut i = 0;
    while i < TPL as usize {
        t[i] = Rule(i as u16);
        i += 1;
    }
    t
};

/// A theme whose rule for a kind is the kind ([`IDENTITY`]).
struct Kinds;

impl Rules for Kinds {
    fn table(&self) -> &[Rule] {
        &IDENTITY
    }

    fn resolve(&mut self, kind: u16) -> Rule {
        Rule(kind)
    }
}

/// A template's line on C's or C++'s scan (`lang`), its spans into `out`. The scan is the run
/// copy ada holds for C and C++ ([`super::line_runs_c`], [`super::line_runs`]) under [`Kinds`],
/// its runs turned back into spans: ada holds no span copy of the scan (C's and C++'s runs come
/// from the run copy), and one for the templates alone cost it ~100 KB.
#[inline(never)]
fn base(b: &[u8], state: &mut State, out: &mut Out, lang: usize) {
    let mut runs: Vec<Run> = Vec::new();
    let mut embedded: Vec<Span> = Vec::new();
    let mut o = Out::new(&mut embedded);
    if lang == L_CPP {
        super::line_runs(b, state, &mut o, &mut Kinds, &mut runs);
    } else {
        super::line_runs_c(b, state, &mut o, &mut Kinds, &mut runs);
    }
    let mut start = 0;
    for r in &runs {
        if r.rule.0 != Rule::NONE {
            out.0.push(Span {
                start,
                end: r.end,
                kind: r.rule.0,
            });
        }
        start = r.end;
    }
}

fn template_line(b: &[u8], state: &mut State, out: &mut Out, lang: usize) {
    let base = |b: &[u8], state: &mut State, out: &mut Out| base(b, state, out, lang);
    // a line without an `@`, a `$` or a `#` is the header's as it stands
    if !b.iter().any(|&c| matches!(c, b'@' | b'$' | b'#')) {
        base(b, state, out);
        return;
    }
    let mut masked: Option<Vec<u8>> = None;
    // `#cmakedefine` and `#cmakedefine01` as `#define`, the `#`'s blanks kept
    let first = b.iter().position(|c| !c.is_ascii_whitespace());
    let directive = first.is_some_and(|h| b[h] == b'#');
    if let Some(h) = first.filter(|_| directive) {
        let mut s = h + 1;
        while s < b.len() && matches!(b[s], b' ' | b'\t') {
            s += 1;
        }
        let rest = &b[s..];
        for (word, as_define) in [
            (&b"cmakedefine01"[..], &b"       define"[..]),
            (b"cmakedefine", b"     define"),
        ] {
            if rest.starts_with(word)
                && !rest
                    .get(word.len())
                    .is_some_and(|&c| c.is_ascii_alphanumeric() || c == b'_')
            {
                masked.get_or_insert_with(|| b.to_vec())[s..s + word.len()]
                    .copy_from_slice(as_define);
                break;
            }
        }
    }
    // the placeholders, each masked as a name
    let mut marks: Vec<(usize, usize, usize, usize)> = Vec::new();
    let mut p = 0;
    while p < b.len() {
        if let Some((s, e, z)) = placeholder(b, p) {
            let m = masked.get_or_insert_with(|| b.to_vec());
            m[p..s].fill(b'_');
            m[e..z].fill(b'_');
            marks.push((p, s, e, z));
            p = z;
        } else {
            p += 1;
        }
    }
    let Some(m) = masked else {
        base(b, state, out);
        return;
    };
    base(&m, state, out);
    if !marks.is_empty() {
        lay(&mut *out.0, &marks, directive);
    }
}

/// The placeholders `marks` (`@`/`${` from `.0`, the name from `.1` to `.2`, the close to `.3`)
/// laid over the scanner's `spans`, each under what the scanner had at its start.
fn lay(spans: &mut Vec<Span>, marks: &[(usize, usize, usize, usize)], directive: bool) {
    let at = |spans: &[Span], p: usize| {
        spans
            .iter()
            .find(|s| s.start as usize <= p && p < s.end as usize)
            .map_or(PLAIN, |s| s.kind)
    };
    let kinds: Vec<u16> = marks
        .iter()
        .map(|&(a, ..)| TPL + 3 * under(at(spans, a), directive) as u16)
        .collect();
    // the scanner's spans with the placeholders cut out of them
    let mut out: Vec<Span> = Vec::with_capacity(spans.len() + 3 * marks.len());
    for s in spans.iter() {
        let mut start = s.start as usize;
        let end = s.end as usize;
        for &(a, .., z) in marks {
            if z <= start || a >= end {
                continue;
            }
            if a > start {
                out.push(Span {
                    start: start as u32,
                    end: a as u32,
                    kind: s.kind,
                });
            }
            start = start.max(z);
        }
        if start < end {
            out.push(Span {
                start: start as u32,
                end: end as u32,
                kind: s.kind,
            });
        }
    }
    for (&(a, s, e, z), &k) in marks.iter().zip(&kinds) {
        for (from, to, part) in [(a, s, 0), (s, e, 1), (e, z, 2)] {
            out.push(Span {
                start: from as u32,
                end: to as u32,
                kind: k + part,
            });
        }
    }
    out.sort_by_key(|s| s.start);
    *spans = out;
}

#[cfg(test)]
mod tests {
    use crate::Lang;
    use crate::tests::marked;

    fn has(lang: Lang, lines: &[&str], text: &str, scopes: &str) {
        let got = marked(lang, lines);
        assert!(
            got.iter().any(|&(t, s)| t == text && s == scopes),
            "{text:?} as {scopes:?} in {got:#?}"
        );
    }

    #[test]
    fn cmakedefines_and_placeholders() {
        let l = [
            "#ifndef CONFIG_H",
            "#cmakedefine HAVE_UNISTD_H 1",
            "#  cmakedefine01 ENABLE_X",
            "#define VERSION \"@PROJECT_VERSION@\"",
            "#define PREFIX ${CMAKE_INSTALL_PREFIX}",
            "/* see @DOC_URL@, mail me@example.com */",
            "#include \"@GEN_DIR@/x.h\"",
            "static const int k = @K@;",
            "#endif",
        ];
        let c = Lang::CMakeCHeader;
        has(c, &l, "#cmakedefine", "keyword.control.import.define.c");
        has(c, &l, "#  cmakedefine01", "keyword.control.import.define.c");
        has(
            c,
            &l,
            "HAVE_UNISTD_H",
            "meta.preprocessor.c entity.name.constant.preprocessor.c",
        );
        has(
            c,
            &l,
            "ENABLE_X",
            "meta.preprocessor.c entity.name.constant.preprocessor.c",
        );
        has(
            c,
            &l,
            "@",
            "string.quoted.double.c variable.cmake keyword.other.block.start.cmake",
        );
        has(
            c,
            &l,
            "PROJECT_VERSION",
            "string.quoted.double.c variable.cmake",
        );
        has(
            c,
            &l,
            "${",
            "meta.preprocessor.c variable.cmake keyword.other.block.start.cmake",
        );
        has(
            c,
            &l,
            "}",
            "meta.preprocessor.c variable.cmake keyword.other.block.end.cmake",
        );
        has(c, &l, "DOC_URL", "comment.block.c variable.cmake");
        // an address is no placeholder: one `@`
        has(c, &l, ", mail me@example.com ", "comment.block.c");
        has(
            c,
            &l,
            "GEN_DIR",
            "string.quoted.double.include.c variable.cmake",
        );
        has(c, &l, "K", "variable.cmake");
        has(c, &l, "int", "storage.type.c");
        let cpp = Lang::CMakeCppHeader;
        has(cpp, &l, "#cmakedefine", "keyword.control.import.define.c++");
        has(cpp, &l, "K", "variable.cmake");
    }

    #[test]
    fn template_names() {
        for (name, lang) in [
            ("config.h.in", Some(Lang::CMakeCHeader)),
            ("llvm-config.h.cmake", Some(Lang::CMakeCHeader)),
            ("version.hpp.in", Some(Lang::CMakeCppHeader)),
            ("x.HXX.CMake", Some(Lang::CMakeCppHeader)),
            ("h.in", Some(Lang::CMakeCHeader)),
            ("hpp.in", Some(Lang::CMakeCppHeader)),
            (".h.in", None),
            // a C or C++ source's template too, and `.cmake` names (X-144)
            ("x.c.in", Some(Lang::CMakeCHeader)),
            ("isl_srcdir.c.cmake", Some(Lang::CMakeCHeader)),
            ("x.cpp.in", Some(Lang::CMakeCppHeader)),
            ("x.CC.cmake", Some(Lang::CMakeCppHeader)),
            ("x.h", Some(Lang::ObjC)),
            // by the rest, as bat (X-145)
            ("Config.cmake.in", Some(Lang::CMake)),
            ("x.h.orig", Some(Lang::ObjC)),
        ] {
            assert_eq!(Lang::from_file_name(name), lang, "{name}");
        }
        let cpp = b"#cmakedefine X\nnamespace a {}\n";
        assert_eq!(Lang::for_header_template(cpp), Lang::CMakeCppHeader);
        let c = b"#cmakedefine X\n#define Y @Y@\n";
        assert_eq!(Lang::for_header_template(c), Lang::CMakeCHeader);
    }
}
