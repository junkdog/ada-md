//! The lexers whose kinds carry bat's scopes exactly (ADR-40), against syntect on bat's
//! grammars, byte for byte: each kind's scopes must be syntect's whole stack, on hand-picked
//! blocks and on blocks of each language's tokens in random order. The `parse` fuzz target does
//! the same on what libFuzzer finds; this keeps the corners each scanner was built against,
//! which were held to the machine's ports (`machine.rs`, gone 2026-11-03) until then.
//!
//! `EXACT_SOUP=N` sets the soups' rounds (default 300 a language; syntect is the slow side).

#[path = "../fuzz/fuzz_targets/lexer_parity.rs"]
mod lexer_parity;

use ada_parse::Lang;
use lexer_parity::{Stacks, check};

/// syntect's search cache is a line's, and a line is cut here, as the fuzz target cuts it.
const LINE: usize = 256;

fn exact(lang: Lang, block: &str) {
    // XML's scanner keeps a nested group's or conditional section's meta scopes once
    let stacks = if lang == Lang::Xml {
        Stacks::Once
    } else {
        Stacks::Whole
    };
    check(lang, block.as_bytes(), LINE, stacks);
}

/// SplitMix64, so the test needs no crate: any seed, zero included.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn pick<'a>(&mut self, of: &[&'a str]) -> &'a str {
        of[(self.next() % of.len() as u64) as usize]
    }
}

/// Blocks of `tokens` in random order, up to `most` a block, in each of `langs`.
fn soup(langs: &[Lang], tokens: &[&str], most: u64) {
    let rounds: u64 = std::env::var("EXACT_SOUP")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(300);
    let mut rng = Rng(0);
    for _ in 0..rounds {
        let n = 1 + rng.next() % most;
        let block: String = (0..n).map(|_| rng.pick(tokens)).collect();
        for &lang in langs {
            let run = std::panic::catch_unwind(|| exact(lang, &block));
            assert!(run.is_ok(), "{lang:?}, block: {block:?}");
        }
    }
}

const GIT: [Lang; 4] = [
    Lang::GitIgnore,
    Lang::GitAttributes,
    Lang::GitMailmap,
    Lang::GitLink,
];

#[test]
fn xml_examples() {
    let blocks = [
        "<?xml version=\"1.0\" encoding='UTF-8'?>\n<root a=\"1\">text &amp; more</root>\n",
        "<?xml-stylesheet href=\"a.xsl\"?>\n<?php echo 1; ?>\n<?XmL x?>\n<?123?>\n",
        "<!DOCTYPE note SYSTEM \"note.dtd\" [\n  <!ELEMENT note (to,from?,(a|b)*)>\n  <!ELEMENT br EMPTY>\n  <!ATTLIST note id ID #REQUIRED kind (x|y) \"x\" ref IDREFS #IMPLIED>\n  <!ENTITY % param \"value\">\n  <!ENTITY pic SYSTEM \"p.gif\" NDATA gif>\n  <!NOTATION gif PUBLIC \"image/gif\">\n  %param;\n  <!BOGUS x>\n]>\n",
        "<!doctype html>\n<!DOCTYPE>\n<!DOCTYPE a:b PUBLIC \"x\" 'y'>\n",
        "<ns:root xmlns:ns=\"urn:x\" a='1' b=bare c = \"&amp;&#65;&#x41;&bad\">\n",
        "  <!-- a comment -- with -- dashes --->\n  <![CDATA[ raw <text> ]]>\n",
        "  <item key=\"v\"/>text &lt; & more < x\n  <a:b:c d:e:f=\"1\" ?>\n  <bad$name 1x=\"2\" / >\n",
        "  <multi\n    attr=\"v\"\n  >\n</ns:root>\n<!-- open\ncomment -->\n",
        "<a b=\"x <c>\n<a b='open\nstill' c\n=\n'v'>\n</a >\n</ a>\n</a x>\n",
        "<a =\"x\" b/c=1 d=/>\n<a ?x=1 /y>\n<?xml a/b=1 >x?>\n<a b==\"c\">\n",
        "<!DOCTYPE x [\n<![ INCLUDE [\n<!ELEMENT a ((b|c)+, d)*>\n<![%p;[<!ATTLIST a b CDATA #FIXED \"1\">]]>\n]]>\n]>\n",
        "<!DOCTYPE x [ <!ENTITY a \"&#38;&amp;\"> <!-- c --> <?pi x?> <![CDATA[x]]> ]>\n",
        "<!DOCTYPE x [<!ELEMENT a (#PCDATA|b)* > <!ATTLIST a b (c|d|&e;|<) #IMPLIED>]>\n",
        "\u{e9}<\u{e9}l\u{e9} \u{e9}=\"\u{e9}\">&\u{e9};</\u{e9}l\u{e9}>\n",
    ];
    for block in blocks {
        exact(Lang::Xml, block);
    }
    exact(
        Lang::Xml,
        &format!("<!DOCTYPE x [<!ELEMENT a {}b", "(".repeat(200)),
    );
    exact(Lang::Xml, &format!("<!DOCTYPE x [{}", "<![[".repeat(60)));
}

#[test]
fn xml_soup() {
    let tokens = [
        "<",
        ">",
        "</",
        "/>",
        "<?",
        "?>",
        "<?xml",
        "<?xml-s",
        "<?php",
        "<!",
        "<!--",
        "-->",
        "--",
        "-",
        "<![CDATA[",
        "]]>",
        "<!DOCTYPE",
        "<!doctype",
        "<!ELEMENT",
        "<!ATTLIST",
        "<!ENTITY",
        "<!NOTATION",
        "<![",
        "[",
        "]",
        "]>",
        "(",
        ")",
        "|",
        ",",
        "*",
        "?",
        "+",
        "#PCDATA",
        "#CDATA",
        "#REQUIRED",
        "#FIXED",
        "#",
        "%",
        "%p;",
        "&",
        "&amp;",
        "&#65;",
        "&#x41;",
        "&#x;",
        ";",
        "=",
        "\"",
        "'",
        " ",
        "  ",
        "\t",
        "\n",
        "\n",
        "\n",
        "a",
        "b:c",
        "ns:",
        ":",
        "x1",
        "1",
        "_",
        ".",
        "/",
        "PUBLIC",
        "SYSTEM",
        "NDATA",
        "EMPTY",
        "ANY",
        "CDATA",
        "ID",
        "IDREFS",
        "NMTOKEN",
        "é",
        "$",
    ];
    soup(&[Lang::Xml], &tokens, 40);
}

#[test]
fn dotenv_examples() {
    let blocks = [
        "# comment\nexport DATABASE_URL=\"postgres://${USER}:$PASS@host/db\\n\" # c\n",
        "DEBUG = true\nN=0x1F\nF=1.5e+3\nG=.5\nH=1. 2.x\nI=1.5x 0x1g 12e5 3E-2y a.5 1e\n",
        "S='single $NOT'\nM=\"multi\nline {x} $$y\"\nEXPORTER=null\na=b=c\n",
        "x =  NULL\ny= False\nz=TRUEx\n export\texport x\nmyexportexport=1 reexport\n",
        "A=\"${a $b ${c}} {$$d} \\q \\$ \\\"\"\nB=\"${unclosed\nstill}\" '\nC='a\nb' \"\n",
        "K=\"$\x7f\" ${é} $é1 $_x_ ${1}\"\n\u{e9}=\u{e9} caf\u{e9}=1 =1 ==2 a = b\n",
        "0x=1 0x 1.e5 .e5 1..2 x=1.2.3 x=-1 +2 x=1_000\n",
        "",
        "\n\n",
        "#\n\"\n'\n${\n",
        "  # lead\n \t export A=1\n\t\n   =true\n  ' x\n\t'\n  \"s\n",
    ];
    for block in blocks {
        exact(Lang::DotEnv, block);
    }
}

#[test]
fn dotenv_soup() {
    let tokens = [
        "#", "\"", "'", "=", " = ", " ", "\t", "\n", "\n", "$", "$$", "${", "{", "}", "\\", "\\n",
        "\\$", "\\\"", "\\q", "export", "EXPORT", "Export ", "true", "False", "NULL", "nul", "KEY",
        "a_b", "x", "e", "E", "+", "-", ".", "0", "1", "42", "0x", "0xFf", "1.5", "e5", "E-3", "_",
        "é", "\u{7f}", "€", ":", "/", "@",
    ];
    soup(&[Lang::DotEnv], &tokens, 40);
}

#[test]
fn git_examples() {
    let blocks = [
        "# build output\n/target/\n!keep.me\n*.log ; not a comment\n**/node_modules\n",
        "docs/[!a-c]*.md\na[]b]c\na[x\n\\#literal \\ \n$HOME/../.cache/./x\nC:/drive ~/home\n   \n",
        "a[\\]b]c [^x] [*?] [ ] [é]\nx\\\ny\\$z\\*\\é \"q\" a:b ./x ../y .z/w.v\n[\n\\\n!\n!!x\n",
        "# attributes\n* text=auto eol=lf\n*.png binary -diff\n",
        "*.rs diff=rust merge=union filter=lfs encoding=UTF-16 whitespace=tab-in-indent,trailing-space\n",
        "\"quoted path\" -text !eol custom=a,b, x=\"q\\\"v\"\n[attr]binary -diff -merge -text\n-bad op\n",
        "path/[ab]*.c foo= bar\nx a=\"open\nx a=, b=,c ,d a=b,,c a==b a=b=c\n !x\n\"[a\" b\n\"a\nb\\\nc d\n",
        "x textual eolx diffx -diff- a-b- a--b _ é=é [ab] [-x] [!x] [a] b\n x\t\ty=\"\\t\\x\" z=\" \"\n",
        "# mailmap\nProper Name <proper@email.xx>\n<proper@email.xx> <commit@email.xx>\n",
        "Proper Name <proper@email.xx> Commit Name <commit@email.xx>\nName Only\n<bad email> x\n",
        "a <b@c.d.e> f ; g\n<<a>> <> <a <a@b@c.d> a\t <x>\nname ;comment <x>\n\r<a>\n",
        "gitdir: ../.git/modules/sub\n  gitdir :C:\\repo\\.git.\ngitdir: ~/x/*\nnotgitdir: x\n",
        "# comment\ngitdir\nx\ngitdir x : y\ngitdir:\ngitdir: a/..\\./b.c.\n gitdir: .\n",
        "",
        "\n\n",
    ];
    for block in blocks {
        for lang in GIT {
            exact(lang, block);
        }
    }
}

#[test]
fn git_soup() {
    let tokens = [
        "#",
        ";",
        "!",
        "-",
        "[",
        "]",
        "^",
        "/",
        "\\",
        ".",
        "..",
        "*",
        "?",
        "$",
        "$HOME",
        ":",
        "C:",
        "~",
        "\"",
        "=",
        ",",
        "<",
        ">",
        "@",
        " ",
        "  ",
        "\t",
        "\r",
        "\n",
        "\n",
        "\n",
        "a",
        "x_y",
        "é",
        "diff",
        "text",
        "eol",
        "encoding",
        "filter",
        "merge",
        "whitespace",
        "binary",
        "gitdir",
        "gitdir:",
        "\\n",
        "\\\"",
        "a@b.c",
        "name",
        "1",
    ];
    soup(&GIT, &tokens, 40);
}

#[test]
fn dockerfile_examples() {
    let blocks = [
        "# syntax=docker/dockerfile:1\nARG BASE=rust\nFROM ${BASE}:1.95 AS build\n",
        "FROM rust:1.95 as build\nWORKDIR /src\nCOPY --from=deps /x /y\nRUN cargo build \\\n  --release && \\\n  echo \"done\\n\" 'x'\n# c\n  # c2\n",
        "FROM x\nRUN echo \"open\nnext\nlower\nUPPER\n",
        "FROM x\nonbuild run x\nONBUILD COPY a b\nCMD [\"a\", \"b\"]\nENTRYPOINT [\"x\"]\nMAINTAINER me\n",
        "FROM x\nRUN\nnext line\nCOPY\nx\n",
        "FROM x\nRUN a \\ \nb\n  RUN x\n\tlabel a=b\n",
        "FROM x@sha256:abc\nFROM scratch\nEXPOSE 80\nHEALTHCHECK CMD x\n",
        "from x\nFROM\nFROM \nARG\nRUN 'a\n\nb'\n",
        "",
        "\n\n",
    ];
    for block in blocks {
        exact(Lang::Dockerfile, block);
    }
}

#[test]
fn dockerfile_soup() {
    let tokens = [
        "FROM ",
        "from ",
        "x",
        "rust:1.95",
        "@sha",
        " AS ",
        " as ",
        "stage",
        "RUN ",
        "run ",
        "RUN",
        "COPY ",
        "COPY",
        "--from=a ",
        "--from=",
        "ONBUILD ",
        "onbuild ",
        "CMD ",
        "ENV ",
        "ARG ",
        "ARG",
        "MAINTAINER ",
        "MAINTAINER",
        "ENTRYPOINT ",
        "\n",
        "\n",
        "\n",
        " ",
        "\t",
        "#",
        "# c",
        "\\",
        "\\\n",
        "\\ ",
        "\"",
        "'",
        "a",
        "A",
        "=",
        "[",
        "]",
        ",",
        ":",
    ];
    soup(&[Lang::Dockerfile], &tokens, 60);
}

#[test]
fn properties_examples() {
    let blocks = [
        "# c\n  ! c\na\\ b = x\\ty \\u00e9\\\n  more\nk:v\nk \\\n v\n",
        "key\nkey=\nkey =\nkey  value:x\n=v\n:v\nk\\u12x=y\nk\\\\=v\nk\\=x = y\\\\\n",
        "a = b\\\nc\\\nd\n x = \\x\n\t\tx\t:\ty\n",
        "",
        "\n\n",
    ];
    for block in blocks {
        exact(Lang::Properties, block);
    }
}

#[test]
fn properties_soup() {
    let tokens = [
        "#", "!", "key", "k", "=", ":", " ", "\t", "\n", "\n", "\\", "\\\n", "\\u", "\\u00e9",
        "00e9", "\\t", "\\ ", "\\:", "v", "x y", "\u{c}",
    ];
    soup(&[Lang::Properties], &tokens, 60);
}
