use crate::Lang;
use crate::tests::marked;

fn man(lines: &[&'static str]) -> Vec<(&'static str, &'static str)> {
    marked(Lang::Manpage, lines)
}

/// The pieces of `lines` whose leaf is `leaf`.
fn with(lines: &[&'static str], leaf: &str) -> Vec<&'static str> {
    man(lines)
        .into_iter()
        .filter(|(_, s)| s.rsplit(' ').next() == Some(leaf))
        .map(|(t, _)| t)
        .collect()
}

const PAGE: &[&str] = &[
    "LS(1)                            User Commands                            LS(1)",
    "",
    "NAME",
    "     ls - list directory contents",
    "",
    "SYNOPSIS",
    "     ls [OPTION]... [FILE]...",
    "",
    "DESCRIPTION",
    "     List information about the FILEs (the current directory by default).  Sort",
    "     entries alphabetically if none of -cftuvSUX nor --sort is specified.",
    "",
    "     -a, --all",
    "            do not ignore entries starting with .",
    "",
    "     --color[=WHEN]",
    "            color the output WHEN; more info below",
    "",
    "     -I, --ignore=PATTERN",
    "            do not list implied entries matching shell PATTERN",
    "",
    "   Exit status:",
    "     0      if OK,",
    "",
    "SEE ALSO",
    "     dircolors(1), LS_COLORS in $HOME/.config, systemd.unit(5), Archive::Tar(3perl)",
    "",
    "GNU coreutils 9.12                October 2026                            LS(1)",
];

/// The title's name and section as bat reads them, its middle the title's heading; the footer's
/// `NAME(SECTION)` the same, its source and date plain (bat reads the footer as a heading).
#[test]
fn title_and_footer() {
    let m = man(&[PAGE[0]]);
    assert_eq!(
        m,
        [
            ("LS", "meta.preprocessor.man"),
            ("(", "keyword.operator.man"),
            ("1", "string.quoted.other.man"),
            (")", "keyword.operator.man"),
            ("User Commands", "markup.heading.title.man"),
            ("LS", "meta.preprocessor.man"),
            ("(", "keyword.operator.man"),
            ("1", "string.quoted.other.man"),
            (")", "keyword.operator.man"),
        ]
    );
    let m = man(PAGE);
    let last = &m[m.len() - 4..];
    assert_eq!(last[0], ("LS", "meta.preprocessor.man"));
    assert!(!m.iter().any(|(t, _)| t.contains("coreutils")), "{m:?}");
    // a title truncated by groff, with one blank around its middle
    let t = "SYSTEMD-...GENERATOR(8) systemd-environment-d-generator SYSTEMD-...GENERATOR(8)";
    assert_eq!(
        with(&[t], "meta.preprocessor.man"),
        ["SYSTEMD-...GENERATOR"; 2]
    );
    // a name with blanks in it
    let t = "411toppm User Manual(0)                                 411toppm User Manual(0)";
    assert_eq!(
        with(&[t], "meta.preprocessor.man"),
        ["411toppm User Manual"; 2]
    );
}

/// Headings at the margin, by section; a subsection's three columns in (ada's: bat's are body).
#[test]
fn headings() {
    assert_eq!(
        with(PAGE, "markup.heading.other.man"),
        ["NAME", "DESCRIPTION", "SEE ALSO"]
    );
    assert_eq!(with(PAGE, "markup.heading.synopsis.man"), ["SYNOPSIS"]);
    assert_eq!(
        with(PAGE, "markup.heading.subsection.man"),
        ["Exit status:"]
    );
    let m = with(&["ENVIRONMENT", "     X"], "markup.heading.env.man");
    assert_eq!(m, ["ENVIRONMENT"]);
    // a line at the margin that isn't the first is no title, even of its shape
    let m = man(&["NAME", "     x", "LS(1)  x  LS(1)"]);
    assert!(m.iter().all(|(_, s)| !s.contains("title")), "{m:?}");
}

/// Options in prose where bat finds them (not after a word's byte), a value after `=`.
#[test]
fn prose_options() {
    let o = with(PAGE, "entity.name.command-line-option.man");
    assert!(o.contains(&"-cftuvSUX") && o.contains(&"--sort"), "{o:?}");
    let l = [
        "DESCRIPTION",
        "     use --color=auto, not non-blocking or 2024-03-31 or -- alone",
    ];
    assert_eq!(with(&l, "entity.name.command-line-option.man"), ["--color"]);
    assert_eq!(with(&l, "variable.parameter.man"), ["auto"]);
}

/// An option list's tags: options, separators, values in their brackets.
#[test]
fn tags() {
    let o = with(PAGE, "entity.name.command-line-option.man");
    for want in ["-a", "--all", "--color", "-I", "--ignore"] {
        assert!(o.contains(&want), "{want} in {o:?}");
    }
    let p = with(PAGE, "variable.parameter.man");
    assert!(p.contains(&"WHEN") && p.contains(&"PATTERN"), "{p:?}");
    let m = man(&["OPTIONS", "", "     -o file", "            write to file."]);
    assert!(m.contains(&("file", "variable.parameter.man")), "{m:?}");
    // a description after one blank isn't a parameter: `--help` is six columns, its text at the
    // body's indent plus seven
    let m = man(&["OPTIONS", "", "     --help display this help and exit"]);
    assert!(!m.iter().any(|(_, s)| s.contains("parameter")), "{m:?}");
    // compact lists: a tag right after another's description
    let l = [
        "OPTIONS",
        "",
        "     -a     all",
        "     -b, --bytes=N",
        "            bytes",
        "     -c     count",
    ];
    assert_eq!(
        with(&l, "entity.name.command-line-option.man"),
        ["-a", "-b", "--bytes", "-c"]
    );
    // a prose line starting with an option, not after a blank: an option, but no tag
    let l = ["DESCRIPTION", "     words and", "     --sort is specified."];
    assert_eq!(with(&l, "entity.name.command-line-option.man"), ["--sort"]);
    assert!(with(&l, "variable.parameter.man").is_empty());
}

/// A synopsis's command, brackets across lines, bars, placeholders, options and their values.
#[test]
fn synopsis() {
    let l = [
        "SYNOPSIS",
        "     git log [<options>] [<revision-range>] [[--] <path>...]",
        "     tar -c [-f ARCHIVE] [OPTIONS] [FILE...]",
        "     ssh [-46] [-B bind_interface] [-b bind_address | -c cipher_spec",
        "         [-D port]] destination [command [argument ...]]",
    ];
    let m = man(&l);
    assert_eq!(with(&l, "entity.name.function.man"), ["git", "tar", "ssh"]);
    let p = with(&l, "variable.parameter.man");
    for want in [
        "options",
        "path",
        "ARCHIVE",
        "OPTIONS",
        "FILE",
        "bind_interface",
        "port",
    ] {
        assert!(p.contains(&want), "{want} in {p:?}");
    }
    assert!(m.contains(&("|", "keyword.operator.logical.man")), "{m:?}");
    // the bracket opened on the line before closes on this one
    let begins = with(&l, "punctuation.section.brackets.begin.man").len();
    let ends = with(&l, "punctuation.section.brackets.end.man").len();
    assert_eq!(begins, ends);
    // literal words plain
    assert!(
        !m.iter().any(|(t, _)| *t == "destination" || *t == "log"),
        "{m:?}"
    );
}

/// References and functions; plurals in parentheses aren't; a name stops at the last token.
#[test]
fn references() {
    let f = with(PAGE, "entity.name.function.man");
    for want in ["dircolors", "systemd.unit", "Archive::Tar"] {
        assert!(f.contains(&want), "{want} in {f:?}");
    }
    assert_eq!(with(PAGE, "constant.numeric.man"), ["1", "5", "3perl"]);
    let l = [
        "DESCRIPTION",
        "     SSL_new() and malloc(size) on file(s), g++(1)",
    ];
    assert_eq!(
        with(&l, "entity.name.function.man"),
        ["SSL_new", "malloc", "g++"]
    );
    // arguments stay plain (bat reads them as a section's number)
    assert_eq!(with(&l, "constant.numeric.man"), ["1"]);
    // a call with another inside
    let l = [
        "EXAMPLES",
        "     if (EVP_KDF_derive(kctx, sizeof(out)) <= 0)",
    ];
    assert_eq!(
        with(&l, "entity.name.function.man"),
        ["EVP_KDF_derive", "sizeof"]
    );
    // `$VAR`
    assert_eq!(with(PAGE, "constant.other.man"), ["HOME"]);
}

/// An ENVIRONMENT section's variables, tags only; a COMMANDS section's commands.
#[test]
fn environment_and_commands() {
    let l = [
        "ENVIRONMENT",
        "     LS_COLORS",
        "            the colors; A sentence.",
        "",
        "     TZ     the zone",
        "",
        "     A sentence at a tag's place.",
    ];
    assert_eq!(
        with(&l, "support.constant.environment-variable.man"),
        ["LS_COLORS", "TZ"]
    );
    let l = [
        "COMMANDS",
        "     add [-f] <file>",
        "            adds a file",
        "",
        "     remove-all",
        "            removes",
    ];
    assert_eq!(with(&l, "entity.name.command.man"), ["add", "remove-all"]);
    assert_eq!(with(&l, "variable.parameter.man"), ["file"]);
}

/// C from an `#include` to the next sentence, on C's scanner; a comment open across lines.
#[test]
fn c() {
    let l = [
        "SYNOPSIS",
        "     #include <stdio.h>",
        "",
        "     int printf(const char *restrict format, ...);",
        "     /* a comment",
        "        across lines */",
        "",
        "   Feature Test Macro Requirements for glibc (see feature_test_macros(7)):",
        "",
        "DESCRIPTION",
        "     #define X 1",
        "     The end.",
    ];
    let m = man(&l);
    assert!(
        m.iter()
            .any(|(t, s)| *t == "int" && s.starts_with("source.c storage.type")),
        "{m:?}"
    );
    assert!(
        m.iter()
            .any(|(t, s)| *t == "printf" && s.starts_with("source.c ")),
        "{m:?}"
    );
    assert!(
        m.iter()
            .any(|(t, s)| t.contains("across") && s.contains("comment.block")),
        "{m:?}"
    );
    // the subsection's heading ends C; a `#define` starts it again, a sentence ends it
    assert_eq!(
        with(&l, "markup.heading.subsection.man"),
        ["Feature Test Macro Requirements for glibc (see feature_test_macros(7)):"]
    );
    assert!(
        m.contains(&("#define", "source.c keyword.control.import.define.c")),
        "{m:?}"
    );
    assert!(
        !m.iter()
            .any(|(t, s)| t.contains("end") && s.starts_with("source.c")),
        "{m:?}"
    );
}

/// Long lines lex in linear time: a line of `(`s or `-`s each looking back or ahead.
#[test]
fn long_lines_stay_linear() {
    use std::time::Instant;
    let time = |unit: &str, n: usize| {
        let line = format!("     {}", unit.repeat(n));
        let leaked: &'static str = Box::leak(line.into_boxed_str());
        let l = ["DESCRIPTION", leaked];
        let start = Instant::now();
        for _ in 0..3 {
            let _ = man(&l);
        }
        start.elapsed().as_secs_f64()
    };
    for unit in ["a(", "a-", "a(b", "-a ", "$a", "x(1) ", "[-a "] {
        let small = time(unit, 2000).max(20e-6);
        let large = time(unit, 8000);
        assert!(large < 10.0 * small, "{unit:?}: {small} s, then {large} s");
    }
}

/// The outline's headings: sections and a subsection, not the title or the footer; a page is
/// told by its title.
#[test]
fn headings_as_the_outline_reads_them() {
    let text = PAGE.join("\n");
    let got: Vec<(u32, u8, &str)> = super::sections(&text)
        .iter()
        .map(|s| (s.line, s.level, s.text))
        .collect();
    assert_eq!(
        got,
        [
            (2, 1, "NAME"),
            (5, 1, "SYNOPSIS"),
            (8, 1, "DESCRIPTION"),
            (21, 2, "Exit status:"),
            (24, 1, "SEE ALSO"),
        ]
    );
    assert!(super::is_page(&text));
    assert!(!super::is_page("NAME\n  x\n"));
    assert!(!super::is_page("f(x) = g(x)\n"));
}

/// A library's synopsis (section 2 or 3, not Perl's) is C without an `#include` before it
/// (doxygen's member lists), but for its sentences; a command's page's isn't.
#[test]
fn library_synopsis() {
    let l = [
        "VSTRING(3)                Library Functions Manual                VSTRING(3)",
        "",
        "SYNOPSIS",
        "   Public Types",
        "     typedef _Alloc allocator_type",
        "     struct rb_node *RB_MAX(struct name *head);",
        "     Link with -lfoo.",
    ];
    let m = man(&l);
    assert!(m.contains(&("typedef", "source.c storage.type.c")), "{m:?}");
    assert!(
        m.contains(&("Public Types", "markup.heading.subsection.man")),
        "{m:?}"
    );
    assert!(
        !m.iter()
            .any(|(t, s)| *t == "Link" || s.starts_with("source.c") && t.contains("lfoo")),
        "{m:?}"
    );
    assert!(
        m.contains(&("-lfoo", "entity.name.command-line-option.man")),
        "{m:?}"
    );
    let perl = [
        "Foo::Bar(3perl)  Perl Programmers Reference Guide  Foo::Bar(3perl)",
        "",
        "SYNOPSIS",
        "     use Foo::Bar;",
    ];
    assert!(!man(&perl).iter().any(|(_, s)| s.starts_with("source.c")));
}

/// A tag list right under its paragraph's line, deeper than it (ImageMagick's), and a
/// subsection that is an option's (go-md2man's, podman's): its tag read, still a subsection.
#[test]
fn tags_under_text_and_option_subsections() {
    let l = [
        "DESCRIPTION",
        "     Image Settings:",
        "       -affine matrix       affine transform matrix",
        "       -alpha option        on, activate, off",
        "",
        "OPTIONS",
        "   --limit=limit",
        "     Limit the number of results.",
    ];
    assert_eq!(
        with(&l, "variable.parameter.man"),
        ["matrix", "option", "limit"]
    );
    assert_eq!(
        with(&l, "entity.name.command-line-option.man"),
        ["-affine", "-alpha", "--limit"]
    );
    let text = l.join("\n");
    let s: Vec<_> = super::sections(&text).iter().map(|s| s.text).collect();
    assert_eq!(s, ["DESCRIPTION", "OPTIONS", "--limit=limit"]);
}

/// Values in brackets stay in their region and in order: `[=` nested, unclosed, or past a
/// synopsis bracket's end.
#[test]
fn bracketed_values_in_order() {
    for line in [
        "     --a[=--a[=--a[=x]]]",
        "     --a[=",
        "     --a[=x",
        "     [--a[=x]] [--b=[y]",
        "     [-a[=--b=<c]>]",
        "     -o <x",
        "     -o=",
        "     $tar->write('files.tgz', COMPRESS_GZIP);    # gzip",
        "     f(a, [b] c)  d(e  f)",
        "     add f(a  b) [x(c  d]",
    ] {
        for head in ["SYNOPSIS", "OPTIONS", "COMMANDS"] {
            let _ = man(&[head, "", Box::leak(line.to_owned().into_boxed_str())]);
        }
    }
}

/// bat's C openers: `struct …{` with or without a name; a word starting `struct` isn't one.
#[test]
fn c_openers() {
    use super::c_starts;
    for yes in [
        "struct stat {",
        "struct {",
        "struct  s  {",
        "#include <a.h>",
        "#define A",
        "/* x */",
    ] {
        assert!(c_starts(yes.as_bytes()), "{yes}");
    }
    for no in ["structure of", "struct stat;", "struct", "# include", "/ x"] {
        assert!(!c_starts(no.as_bytes()), "{no}");
    }
}
